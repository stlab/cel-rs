//! Wires `lsp-server` to the explicit module and source-only diagnostic hosts.

use std::collections::HashMap;

use adam_lang::format_source;
use lsp_server::{Connection, Message, Notification as ServerNotification, Request, Response};
use lsp_types::{
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, DocumentFormattingParams, OneOf,
    Position, PublishDiagnosticsParams, Range, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextEdit, Uri,
    notification::{
        DidChangeTextDocument, DidOpenTextDocument, Notification as _, PublishDiagnostics,
    },
    request::{Formatting, Request as _},
};

use crate::modules::ModuleSources;

/// The JSON-RPC "Method not found" error code, reused by LSP for unhandled request methods.
const METHOD_NOT_FOUND: i32 = -32601;
/// The JSON-RPC "Invalid params" error code, used when `textDocument/formatting`'s params fail
/// to deserialize.
const INVALID_PARAMS: i32 = -32602;

/// Runs the adam-lang language server on stdin/stdout until the client sends `exit`.
///
/// - Complexity: cumulative message dispatch, source analysis and serialization
///   time; space for the current source snapshot and its analysis.
///
/// # Errors
///
/// Returns `Err` if the initialize handshake fails, a message can't be read from or written to
/// stdio, or the background reader/writer threads panic.
///
/// # Examples
///
/// ```no_run
/// adam_lsp::run()?;
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn run() -> anyhow::Result<()> {
    let (connection, io_threads) = Connection::stdio();
    serve(&connection)?;
    drop(connection);
    io_threads.join()?;
    Ok(())
}

/// Performs initialization, then publishes source/module diagnostics until shutdown.
///
/// Reads the caller-owned `moduleSources` wire snapshot described in the
/// [crate documentation](crate), and accepts complete configuration replacements.
///
/// Exposed separately from [`run`] so tests can drive an in-memory [`Connection::memory`] pair
/// instead of real stdio.
///
/// # Errors
///
/// Returns an error if initialization, shutdown or transport communication fails.
///
/// - Complexity: cumulative message dispatch, source analysis and serialization
///   time; space for the current source snapshot and its analysis.
///
/// # Examples
///
/// ```no_run
/// let (server, _client) = lsp_server::Connection::memory();
/// adam_lsp::serve(&server)?;
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn serve(connection: &Connection) -> anyhow::Result<()> {
    let capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        document_formatting_provider: Some(OneOf::Left(true)),
        ..Default::default()
    })?;
    let options = connection.initialize(capabilities)?;
    let modules = ModuleSources::from_initialization(
        options
            .get("initializationOptions")
            .unwrap_or(&serde_json::Value::Null),
    );
    main_loop(connection, modules)
}

/// Dispatches every message on `connection` until a `shutdown`/`exit` sequence ends the server.
///
/// - Complexity: cumulative dispatch and analysis time; space for the current
///   source snapshot and its analysis.
///
/// # Errors
///
/// Returns `Err` if a message can't be read from or sent to `connection` (a broken transport).
fn main_loop(connection: &Connection, mut modules: ModuleSources) -> anyhow::Result<()> {
    let mut documents: HashMap<String, String> = HashMap::new();
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                handle_request(connection, &documents, req)?;
            }
            Message::Notification(not) => {
                handle_notification(connection, &mut documents, &mut modules, not)?
            }
            Message::Response(_) => {}
        }
    }
    Ok(())
}

/// Handles one client request. Only `textDocument/formatting` is implemented; every other method
/// gets a JSON-RPC "Method not found" response (`shutdown` is intercepted earlier, in
/// `main_loop`, and never reaches here).
///
/// - Complexity: formatting time/space for an open source, plus response
///   serialization and expected O(u) URI lookup time for u URI bytes.
///
/// # Errors
///
/// Returns `Err` only if sending the response fails (a broken transport).
fn handle_request(
    connection: &Connection,
    documents: &HashMap<String, String>,
    req: Request,
) -> anyhow::Result<()> {
    match req.method.as_str() {
        Formatting::METHOD => {
            let id = req.id.clone();
            match req.extract::<DocumentFormattingParams>(Formatting::METHOD) {
                Ok((id, params)) => {
                    // A URI not present in `documents` (never seen via didOpen/didChange) silently
                    // gets no edits, the same "nothing to do" response as a syntax error — not an error
                    // response.
                    let edits = documents
                        .get(params.text_document.uri.as_str())
                        .map(|source| format_edits(source))
                        .unwrap_or_default();
                    connection
                        .sender
                        .send(Message::Response(Response::new_ok(id, edits)))?;
                }
                Err(error) => {
                    connection.sender.send(Message::Response(Response::new_err(
                        id,
                        INVALID_PARAMS,
                        error.to_string(),
                    )))?;
                }
            }
        }
        _ => {
            let response = Response::new_err(
                req.id.clone(),
                METHOD_NOT_FOUND,
                format!("unhandled method: {}", req.method),
            );
            connection.sender.send(Message::Response(response))?;
        }
    }
    Ok(())
}

/// Applies document/configuration changes and republishes all affected source owners.
///
/// Open documents overlay registered revisions by exact URI; closing restores the
/// supplied revision. Malformed notifications leave documents unchanged.
/// FULL-sync changes must omit both range fields; any range-bearing entry
/// rejects the complete batch. Complete replacements apply in array order.
/// - Complexity: diagnostic analysis and serialization plus O(n) full-text storage.
///
/// # Errors
///
/// Returns `Err` only if sending the resulting `publishDiagnostics` notification fails (a broken
/// transport). A `didOpen`/`didChange` notification whose params fail to deserialize is logged to
/// stderr and skipped rather than propagated, so one malformed client message can't take down the
/// server.
fn handle_notification(
    connection: &Connection,
    documents: &mut HashMap<String, String>,
    modules: &mut ModuleSources,
    not: ServerNotification,
) -> anyhow::Result<()> {
    match not.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let params: DidOpenTextDocumentParams = match not.extract(DidOpenTextDocument::METHOD) {
                Ok(params) => params,
                Err(error) => {
                    eprintln!(
                        "adam-lsp: ignoring malformed {}: {error}",
                        DidOpenTextDocument::METHOD
                    );
                    return Ok(());
                }
            };
            documents.insert(
                params.text_document.uri.as_str().to_owned(),
                params.text_document.text,
            );
            publish_all(connection, documents, modules)?;
        }
        DidChangeTextDocument::METHOD => {
            let params: DidChangeTextDocumentParams =
                match not.extract(DidChangeTextDocument::METHOD) {
                    Ok(params) => params,
                    Err(error) => {
                        eprintln!(
                            "adam-lsp: ignoring malformed {}: {error}",
                            DidChangeTextDocument::METHOD
                        );
                        return Ok(());
                    }
                };
            if params
                .content_changes
                .iter()
                .any(|change| change.range.is_some() || change.range_length.is_some())
            {
                eprintln!("adam-lsp: ignoring incremental didChange with FULL synchronization");
                return Ok(());
            }
            if let Some(change) = params.content_changes.into_iter().last() {
                documents.insert(params.text_document.uri.as_str().to_owned(), change.text);
                publish_all(connection, documents, modules)?;
            }
        }
        "textDocument/didClose" => {
            let params: lsp_types::DidCloseTextDocumentParams =
                match serde_json::from_value(not.params) {
                    Ok(params) => params,
                    Err(error) => {
                        eprintln!("adam-lsp: ignoring malformed didClose: {error}");
                        return Ok(());
                    }
                };
            documents.remove(params.text_document.uri.as_str());
            // Clear stale source-only diagnostics; module publications below replace this.
            publish(connection, params.text_document.uri, Vec::new())?;
            publish_all(connection, documents, modules)?;
        }
        "workspace/didChangeConfiguration" => {
            let params: lsp_types::DidChangeConfigurationParams =
                match serde_json::from_value(not.params) {
                    Ok(params) => params,
                    Err(error) => {
                        eprintln!("adam-lsp: ignoring malformed didChangeConfiguration: {error}");
                        return Ok(());
                    }
                };
            let old = modules.diagnostics(documents);
            modules.configure(&params.settings);
            let fresh = modules.diagnostics(documents);
            for uri in old.keys().filter(|uri| !fresh.contains_key(*uri)) {
                publish(connection, uri.parse()?, Vec::new())?;
            }
            publish_diagnostics(connection, fresh)?;
        }
        _ => {}
    }
    Ok(())
}

/// Sends one complete diagnostic replacement for `uri`.
///
/// - Complexity: O(u + d) time and space for u URI and d serialized diagnostic bytes.
///
/// # Errors
///
/// Returns `Err` if sending the notification on `connection` fails (a broken transport).
fn publish(
    connection: &Connection,
    uri: Uri,
    diagnostics: Vec<lsp_types::Diagnostic>,
) -> anyhow::Result<()> {
    let params = PublishDiagnosticsParams {
        uri,
        diagnostics,
        version: None,
    };
    let notification = ServerNotification::new(PublishDiagnostics::METHOD.to_string(), params);
    connection
        .sender
        .send(Message::Notification(notification))?;
    Ok(())
}

/// Rechecks all supplied and open sources and publishes clean/error replacements.
///
/// - Complexity: the source snapshot's analysis cost plus diagnostic serialization.
/// # Errors
/// Returns an error if the transport fails.
fn publish_all(
    connection: &Connection,
    documents: &HashMap<String, String>,
    modules: &ModuleSources,
) -> anyhow::Result<()> {
    publish_diagnostics(connection, modules.diagnostics(documents))
}

/// Sends a complete diagnostic replacement for each URI in the supplied map.
///
/// - Complexity: O(f + u + d) time and O(u + d) serialization space for f
///   owners, u URI bytes and d diagnostic bytes.
/// # Errors
/// Returns an error if the transport fails or an internal URI cannot be parsed.
fn publish_diagnostics(
    connection: &Connection,
    diagnostics: std::collections::BTreeMap<String, Vec<lsp_types::Diagnostic>>,
) -> anyhow::Result<()> {
    for (uri, diagnostics) in diagnostics {
        publish(connection, uri.parse()?, diagnostics)?;
    }
    Ok(())
}

/// Computes the `textDocument/formatting` edit for adam-lang `source`.
///
/// - Postcondition: returns an empty `Vec` if [`format_source`] refuses `source` (it doesn't
///   parse, or parses with any recovered syntax error) — refusing to format code it can't fully
///   understand, matching `rustfmt`. Otherwise returns exactly one [`TextEdit`] replacing the
///   whole document with `format_source`'s output.
/// - Complexity: [`format_source`]'s parsing/trivia/formatting costs and O(o)
///   output space for o formatted bytes, with no caching across calls.
fn format_edits(source: &str) -> Vec<TextEdit> {
    match format_source(source) {
        Ok(formatted) => vec![TextEdit {
            range: whole_document_range(),
            new_text: formatted,
        }],
        Err(_) => Vec::new(),
    }
}

/// A `Range` guaranteed to cover an entire document regardless of its actual length — LSP
/// clients clamp an out-of-bounds end position to the document's real end, so this avoids
/// needing to compute the exact last line/column of `source` (and getting it wrong for the
/// common trailing-newline edge case).
fn whole_document_range() -> Range {
    Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: u32::MAX,
            character: u32::MAX,
        },
    }
}

/// Verifies formatting, lifecycle handshakes and safe notification dispatch.
#[cfg(test)]
mod tests {
    use lsp_server::{Connection, Message, Notification as ServerNotification, Request, RequestId};
    use lsp_types::{
        DidChangeTextDocumentParams, DidOpenTextDocumentParams, DocumentFormattingParams,
        PublishDiagnosticsParams, TextDocumentContentChangeEvent, TextDocumentIdentifier,
        TextDocumentItem, TextEdit, VersionedTextDocumentIdentifier,
        notification::{
            DidChangeTextDocument, DidOpenTextDocument, Notification as _, PublishDiagnostics,
        },
        request::{Formatting, Request as _},
    };

    use super::{format_edits, serve};

    /// Rejects malformed formatting parameters and returns no edits for unknown documents.
    #[test]
    fn formatting_request_validates_params_and_unknown_document() {
        let (server, client) = Connection::memory();
        let documents = std::collections::HashMap::new();
        for (params, valid) in [
            (serde_json::json!({"missing":"params"}), false),
            (
                serde_json::json!({
                    "textDocument":{"uri":"file:///missing.adm2"},
                    "options":{"tabSize":4,"insertSpaces":true}
                }),
                true,
            ),
        ] {
            super::handle_request(
                &server,
                &documents,
                Request::new(4.into(), Formatting::METHOD.into(), params),
            )
            .unwrap();
            let Message::Response(response) = client.receiver.recv().unwrap() else {
                panic!("expected formatting response")
            };
            assert_eq!(response.id, 4.into());
            if valid {
                assert_eq!(response.response_result.unwrap(), serde_json::json!([]));
            } else {
                assert_eq!(
                    response.response_result.unwrap_err().code,
                    super::INVALID_PARAMS
                );
            }
        }
    }

    /// Rejects incremental FULL-sync messages without losing the last complete document.
    #[test]
    fn full_sync_rejects_range_changes_and_recovers() {
        let (server, client) = Connection::memory();
        let uri = "file:///full.adm2";
        let original = "sheet old {}";
        let mut documents = std::collections::HashMap::from([(uri.into(), original.into())]);
        let mut modules = crate::modules::ModuleSources::default();
        for changes in [
            serde_json::json!([{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"text":"x"}]),
            serde_json::json!([{"rangeLength":1,"text":"x"}]),
            serde_json::json!([{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"text":"x"},{"text":"sheet new {}"}]),
            serde_json::json!([]),
        ] {
            super::handle_notification(
                &server,
                &mut documents,
                &mut modules,
                ServerNotification::new(
                    DidChangeTextDocument::METHOD.into(),
                    serde_json::json!({"textDocument":{"uri":uri,"version":2},"contentChanges":changes}),
                ),
            ).unwrap();
            assert_eq!(documents[uri], original);
            assert!(client.receiver.try_recv().is_err());
        }
        super::handle_notification(
            &server,
            &mut documents,
            &mut modules,
            ServerNotification::new(
                DidChangeTextDocument::METHOD.into(),
                serde_json::json!({"textDocument":{"uri":uri,"version":3},"contentChanges":[{"text":"sheet interim {}"},{"text":"sheet new {}"}]}),
            ),
        ).unwrap();
        assert_eq!(documents[uri], "sheet new {}");
        let publication = expect_published(&client);
        assert_eq!(publication.uri.as_str(), uri);
        assert!(publication.diagnostics.is_empty());
    }

    /// Whole-document edits preserve comments around list closers without trailing commas.
    #[test]
    fn format_edits_preserve_closing_list_comments() {
        let source = "sheet helper(a: i32 /* HEADER_BEFORE */) // HEADER_AFTER\n\
                 {} sheet app { relationship helper(a: x // CALL_BEFORE\n\
                 ) /* CALL_AFTER */; }";
        let edits = format_edits(source);
        assert_eq!(edits.len(), 1);
        let formatted = &edits[0].new_text;
        for marker in ["HEADER_BEFORE", "HEADER_AFTER", "CALL_BEFORE", "CALL_AFTER"] {
            assert_eq!(formatted.matches(marker).count(), 1, "{formatted}");
        }
        assert_eq!(format_edits(formatted)[0].new_text, *formatted);
    }

    /// Module formatting preserves imports, aliases, parameter types and named bindings.
    #[test]
    fn format_edits_preserve_complete_modules_and_refuse_recovered_items() {
        let source = "use lib::pair as p; sheet helper(a:f64){} sheet app { cell x:f64; relationship p(a:x); }";
        let edits = format_edits(source);
        assert_eq!(edits.len(), 1);
        let formatted = &edits[0].new_text;
        assert!(formatted.contains("use lib::pair as p;"), "{formatted}");
        assert!(formatted.contains("helper(a: f64)"), "{formatted}");
        assert!(formatted.contains("p(a: x)"), "{formatted}");
        assert_eq!(format_edits(formatted)[0].new_text, *formatted);
        for malformed in [
            "use lib::; sheet good {}",
            "sheet helper(a: ) {} sheet app {}",
            "sheet helper(a: i32,) {} sheet app {}",
            "sheet app { relationship helper(a: x,); }",
            "sheet good {} sheet bad { cell : i32; }",
        ] {
            assert!(format_edits(malformed).is_empty(), "{malformed}");
        }
    }

    /// Refuses formatting a source without a sheet.
    #[test]
    fn format_edits_is_empty_for_a_syntax_error() {
        assert!(format_edits("not a sheet at all").is_empty());
    }

    /// Refuses formatting a sheet with a recovered malformed declaration.
    #[test]
    fn format_edits_is_empty_for_a_recovered_syntax_error() {
        assert!(format_edits("sheet s { cell x unknown_syntax }").is_empty());
    }

    /// Produces one canonical full-document replacement for a clean sheet.
    #[test]
    fn format_edits_returns_one_edit_replacing_the_whole_document() {
        let edits = format_edits("sheet   s{cell x:i32=1;}");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start, lsp_types::Position::new(0, 0));
        assert!(edits[0].range.end.line >= 1);
        assert_eq!(edits[0].new_text, "sheet s {\n    cell x: i32 = 1;\n}\n");
    }

    /// Preserves a cell filter in canonical formatting.
    #[test]
    fn format_edits_formats_a_cell_with_a_filter() {
        let edits = format_edits("sheet s { cell a:i32=1 filter _; }");
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].new_text,
            "sheet s {\n    cell a: i32 = 1 filter _;\n}\n"
        );
    }

    /// Formats inclusive range filters without failing the request.
    #[test]
    fn format_edits_formats_a_cell_with_a_range_inclusive_filter() {
        // Regression test: formatting a range-literal filter (as in
        // begin/examples/inequality.adm2's `filter 0..=200`) used to panic inside
        // cel_parser::fmt::binary_op_level ("unknown operator `range_inclusive`"), crashing
        // adam-lsp's single-threaded server mid-request — which VS Code's format-on-save then
        // waited on forever, indistinguishable from a hang.
        let edits = format_edits("sheet s { cell a:i32=0 filter 0..=200; }");
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].new_text,
            "sheet s {\n    cell a: i32 = 0 filter 0..=200;\n}\n"
        );
    }

    /// Formats the stored source of an opened document over the real transport.
    #[test]
    fn formatting_request_returns_the_edit_for_a_previously_opened_document() {
        let (server, client) = Connection::memory();
        let server_thread = std::thread::spawn(move || serve(&server));
        initialize(&client);

        let uri: lsp_types::Uri = "file:///test.adm2".parse().unwrap();
        client
            .sender
            .send(Message::Notification(ServerNotification::new(
                DidOpenTextDocument::METHOD.to_string(),
                DidOpenTextDocumentParams {
                    text_document: TextDocumentItem {
                        uri: uri.clone(),
                        language_id: "adam-lang".to_string(),
                        version: 1,
                        text: "sheet s { cell x: i32 = 1; }".to_string(),
                    },
                },
            )))
            .unwrap();
        expect_published(&client); // the didOpen's diagnostics notification

        client
            .sender
            .send(Message::Request(Request::new(
                RequestId::from(3),
                Formatting::METHOD.to_string(),
                DocumentFormattingParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                    options: lsp_types::FormattingOptions {
                        tab_size: 4,
                        insert_spaces: true,
                        ..Default::default()
                    },
                    work_done_progress_params: Default::default(),
                },
            )))
            .unwrap();
        let response = match client.receiver.recv().unwrap() {
            Message::Response(r) => r,
            other => panic!("expected a response, got {other:?}"),
        };
        assert_eq!(response.id, RequestId::from(3));
        let edits: Vec<TextEdit> =
            serde_json::from_value(response.response_result.unwrap()).unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "sheet s {\n    cell x: i32 = 1;\n}\n");

        shut_down(&client);
        server_thread.join().unwrap().unwrap();
    }

    /// Completes initialization after verifying its successful response ID and FULL sync.
    ///
    /// - Complexity: O(n) space for the initialization response's n payload bytes.
    fn initialize(client: &Connection) {
        client
            .sender
            .send(Message::Request(Request::new(
                RequestId::from(1),
                "initialize".to_string(),
                serde_json::json!({}),
            )))
            .unwrap();
        let Message::Response(response) = client.receiver.recv().unwrap() else {
            panic!("expected initialization response")
        };
        assert_eq!(response.id, RequestId::from(1));
        let result = response.response_result.unwrap();
        assert_eq!(result["capabilities"]["textDocumentSync"], 1);
        client
            .sender
            .send(Message::Notification(ServerNotification::new(
                "initialized".to_string(),
                serde_json::json!({}),
            )))
            .unwrap();
    }

    /// Sends exit after verifying the successful shutdown response ID.
    fn shut_down(client: &Connection) {
        client
            .sender
            .send(Message::Request(Request::new(
                RequestId::from(2),
                "shutdown".to_string(),
                serde_json::json!(null),
            )))
            .unwrap();
        let Message::Response(response) = client.receiver.recv().unwrap() else {
            panic!("expected shutdown response")
        };
        assert_eq!(response.id, RequestId::from(2));
        assert!(response.response_result.is_ok(), "{response:?}");
        client
            .sender
            .send(Message::Notification(ServerNotification::new(
                "exit".to_string(),
                serde_json::json!(null),
            )))
            .unwrap();
    }

    /// Receives the next message from `client` and asserts it's a `publishDiagnostics`
    /// notification, returning its deserialized params.
    ///
    /// - Complexity: O(n) time and space for n serialized diagnostic bytes.
    fn expect_published(client: &Connection) -> PublishDiagnosticsParams {
        let published = match client.receiver.recv().unwrap() {
            Message::Notification(n) => n,
            other => panic!("expected a notification, got {other:?}"),
        };
        assert_eq!(published.method, PublishDiagnostics::METHOD);
        serde_json::from_value(published.params).unwrap()
    }

    /// Publishes the opened owner's type errors.
    #[test]
    fn open_notification_triggers_a_publish_diagnostics_notification() {
        let (server, client) = Connection::memory();
        let server_thread = std::thread::spawn(move || serve(&server));
        initialize(&client);

        let uri: lsp_types::Uri = "file:///test.pm".parse().unwrap();
        client
            .sender
            .send(Message::Notification(ServerNotification::new(
                DidOpenTextDocument::METHOD.to_string(),
                DidOpenTextDocumentParams {
                    text_document: TextDocumentItem {
                        uri: uri.clone(),
                        language_id: "adam-lang".to_string(),
                        version: 1,
                        text: "sheet s { cell x: i32 = 1.0; }".to_string(),
                    },
                },
            )))
            .unwrap();

        let params = expect_published(&client);
        assert_eq!(params.uri, uri);
        assert_eq!(params.diagnostics.len(), 1);

        shut_down(&client);
        server_thread.join().unwrap().unwrap();
    }

    /// Continues serving after a malformed open notification.
    #[test]
    fn malformed_open_notification_is_skipped_not_crashed() {
        let (server, client) = Connection::memory();
        let server_thread = std::thread::spawn(move || serve(&server));
        initialize(&client);

        // Params that don't deserialize as `DidOpenTextDocumentParams` at all.
        client
            .sender
            .send(Message::Notification(ServerNotification::new(
                DidOpenTextDocument::METHOD.to_string(),
                serde_json::json!({"not": "valid"}),
            )))
            .unwrap();

        // A subsequent well-formed `didOpen` still gets a normal `publishDiagnostics` response,
        // proving the server is still alive and serving requests after the malformed message.
        let uri: lsp_types::Uri = "file:///test.pm".parse().unwrap();
        client
            .sender
            .send(Message::Notification(ServerNotification::new(
                DidOpenTextDocument::METHOD.to_string(),
                DidOpenTextDocumentParams {
                    text_document: TextDocumentItem {
                        uri: uri.clone(),
                        language_id: "adam-lang".to_string(),
                        version: 1,
                        text: "sheet s { cell x: i32 = 1; }".to_string(),
                    },
                },
            )))
            .unwrap();

        let params = expect_published(&client);
        assert_eq!(params.uri, uri);
        assert!(params.diagnostics.is_empty());

        shut_down(&client);
        server_thread.join().unwrap().unwrap();
    }

    /// Replaces clean diagnostics after a complete text update introduces a type error.
    #[test]
    fn change_notification_after_open_triggers_a_second_publish_diagnostics_notification() {
        let (server, client) = Connection::memory();
        let server_thread = std::thread::spawn(move || serve(&server));
        initialize(&client);

        let uri: lsp_types::Uri = "file:///test.pm".parse().unwrap();
        client
            .sender
            .send(Message::Notification(ServerNotification::new(
                DidOpenTextDocument::METHOD.to_string(),
                DidOpenTextDocumentParams {
                    text_document: TextDocumentItem {
                        uri: uri.clone(),
                        language_id: "adam-lang".to_string(),
                        version: 1,
                        text: "sheet s { cell x: i32 = 1; }".to_string(),
                    },
                },
            )))
            .unwrap();
        let first = expect_published(&client);
        assert!(first.diagnostics.is_empty());

        client
            .sender
            .send(Message::Notification(ServerNotification::new(
                DidChangeTextDocument::METHOD.to_string(),
                DidChangeTextDocumentParams {
                    text_document: VersionedTextDocumentIdentifier {
                        uri: uri.clone(),
                        version: 2,
                    },
                    content_changes: vec![TextDocumentContentChangeEvent {
                        range: None,
                        range_length: None,
                        text: "sheet s { cell x: i32 = 1.0; }".to_string(),
                    }],
                },
            )))
            .unwrap();
        let second = expect_published(&client);
        assert_eq!(second.uri, uri);
        assert_eq!(second.diagnostics.len(), 1);

        shut_down(&client);
        server_thread.join().unwrap().unwrap();
    }

    /// Returns method-not-found with the original ID for unsupported requests.
    #[test]
    fn unrecognized_request_method_gets_a_method_not_found_response() {
        let (server, client) = Connection::memory();
        let server_thread = std::thread::spawn(move || serve(&server));
        initialize(&client);

        client
            .sender
            .send(Message::Request(Request::new(
                RequestId::from(3),
                "textDocument/hover".to_string(),
                serde_json::json!({}),
            )))
            .unwrap();

        let response = match client.receiver.recv().unwrap() {
            Message::Response(r) => r,
            other => panic!("expected a response, got {other:?}"),
        };
        assert_eq!(response.id, RequestId::from(3));
        assert_eq!(
            response.response_result.unwrap_err().code,
            super::METHOD_NOT_FOUND
        );

        shut_down(&client);
        server_thread.join().unwrap().unwrap();
    }
}
