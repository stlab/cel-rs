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
/// # Errors
///
/// Returns `Err` if the initialize handshake fails, a message can't be read from or written to
/// stdio, or the background reader/writer threads panic.
pub fn run() -> anyhow::Result<()> {
    let (connection, io_threads) = Connection::stdio();
    serve(&connection)?;
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
/// Returns `Err` under the same conditions as [`run`].
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
/// - Complexity: O(d) for the serialized diagnostic size.
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
/// - Complexity: O(d) for the serialized diagnostic size.
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
/// - Complexity: O(n) in the length of `source` — parses, attaches trivia to, and formats the
///   whole sheet once, with no caching across calls.
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
            "sheet good {} sheet bad { cell : i32; }",
        ] {
            assert!(format_edits(malformed).is_empty(), "{malformed}");
        }
    }

    #[test]
    fn format_edits_is_empty_for_a_syntax_error() {
        assert!(format_edits("not a sheet at all").is_empty());
    }

    #[test]
    fn format_edits_is_empty_for_a_recovered_syntax_error() {
        assert!(format_edits("sheet s { cell x unknown_syntax }").is_empty());
    }

    #[test]
    fn format_edits_returns_one_edit_replacing_the_whole_document() {
        let edits = format_edits("sheet   s{cell x:i32=1;}");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "sheet s {\n    cell x: i32 = 1;\n}\n");
    }

    #[test]
    fn format_edits_formats_a_cell_with_a_filter() {
        let edits = format_edits("sheet s { cell a:i32=1 filter _; }");
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].new_text,
            "sheet s {\n    cell a: i32 = 1 filter _;\n}\n"
        );
    }

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
        let edits: Vec<TextEdit> =
            serde_json::from_value(response.response_result.unwrap()).unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "sheet s {\n    cell x: i32 = 1;\n}\n");

        shut_down(&client);
        server_thread.join().unwrap().unwrap();
    }

    /// Sends the `initialize` -> `initialized` handshake on `client`, discarding the response.
    fn initialize(client: &Connection) {
        client
            .sender
            .send(Message::Request(Request::new(
                RequestId::from(1),
                "initialize".to_string(),
                serde_json::json!({}),
            )))
            .unwrap();
        client.receiver.recv().unwrap();
        client
            .sender
            .send(Message::Notification(ServerNotification::new(
                "initialized".to_string(),
                serde_json::json!({}),
            )))
            .unwrap();
    }

    /// Sends the `shutdown` -> `exit` sequence on `client`, discarding the response.
    fn shut_down(client: &Connection) {
        client
            .sender
            .send(Message::Request(Request::new(
                RequestId::from(2),
                "shutdown".to_string(),
                serde_json::json!(null),
            )))
            .unwrap();
        client.receiver.recv().unwrap();
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
    fn expect_published(client: &Connection) -> PublishDiagnosticsParams {
        let published = match client.receiver.recv().unwrap() {
            Message::Notification(n) => n,
            other => panic!("expected a notification, got {other:?}"),
        };
        assert_eq!(published.method, PublishDiagnostics::METHOD);
        serde_json::from_value(published.params).unwrap()
    }

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
        assert!(response.response_result.is_err());

        shut_down(&client);
        server_thread.join().unwrap().unwrap();
    }
}
