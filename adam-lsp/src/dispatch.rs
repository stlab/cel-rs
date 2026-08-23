//! Wires `lsp-server`'s stdio transport to [`crate::diagnostics::diagnostics_for_source`].

use std::collections::HashMap;

use adam_lang::{AdamAstParser, attach_trivia, format_sheet};
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

use crate::diagnostics::diagnostics_for_source;

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

/// Performs the LSP initialize handshake on `connection`, then serves `textDocument/didOpen`
/// and `textDocument/didChange` notifications as `textDocument/publishDiagnostics` until the
/// client shuts the server down.
///
/// Exposed separately from [`run`] so tests can drive an in-memory [`Connection::memory`] pair
/// instead of real stdio.
///
/// # Errors
///
/// Returns `Err` under the same conditions as [`run`].
pub fn serve(connection: &Connection) -> anyhow::Result<()> {
    let capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        document_formatting_provider: Some(OneOf::Left(true)),
        ..Default::default()
    })?;
    connection.initialize(capabilities)?;
    main_loop(connection)
}

/// Dispatches every message on `connection` until a `shutdown`/`exit` sequence ends the server.
///
/// # Errors
///
/// Returns `Err` if a message can't be read from or sent to `connection` (a broken transport).
// `lsp_types::Uri`'s `Hash`/`Eq` are keyed on `as_str()` only (see its `impl Hash for Uri`); the
// interior-mutable field clippy's flagging here never participates in either, so it's safe as a
// map key despite the lint.
#[allow(clippy::mutable_key_type)]
fn main_loop(connection: &Connection) -> anyhow::Result<()> {
    let mut documents: HashMap<Uri, String> = HashMap::new();
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                handle_request(connection, &documents, req)?;
            }
            Message::Notification(not) => handle_notification(connection, &mut documents, not)?,
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
// See the matching `#[allow]` on `main_loop` for why `HashMap<Uri, _>` is fine despite the lint.
#[allow(clippy::mutable_key_type)]
fn handle_request(
    connection: &Connection,
    documents: &HashMap<Uri, String>,
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
                        .get(&params.text_document.uri)
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

/// Handles one client notification, publishing fresh diagnostics on `didOpen`/`didChange` and
/// recording the document's current text in `documents` so a later `textDocument/formatting`
/// request can look it up by URI (that request's params carry only a URI, not the text).
///
/// # Errors
///
/// Returns `Err` only if sending the resulting `publishDiagnostics` notification fails (a broken
/// transport). A `didOpen`/`didChange` notification whose params fail to deserialize is logged to
/// stderr and skipped rather than propagated, so one malformed client message can't take down the
/// server.
// See the matching `#[allow]` on `main_loop` for why `HashMap<Uri, _>` is fine despite the lint.
#[allow(clippy::mutable_key_type)]
fn handle_notification(
    connection: &Connection,
    documents: &mut HashMap<Uri, String>,
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
                params.text_document.uri.clone(),
                params.text_document.text.clone(),
            );
            publish(
                connection,
                &params.text_document.uri,
                &params.text_document.text,
            )?;
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
                documents.insert(params.text_document.uri.clone(), change.text.clone());
                publish(connection, &params.text_document.uri, &change.text)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Computes diagnostics for `source` and sends them as a `textDocument/publishDiagnostics`
/// notification for `uri`.
///
/// # Errors
///
/// Returns `Err` if sending the notification on `connection` fails (a broken transport).
fn publish(connection: &Connection, uri: &Uri, source: &str) -> anyhow::Result<()> {
    let params = PublishDiagnosticsParams {
        uri: uri.clone(),
        diagnostics: diagnostics_for_source(source),
        version: None,
    };
    let notification = ServerNotification::new(PublishDiagnostics::METHOD.to_string(), params);
    connection
        .sender
        .send(Message::Notification(notification))?;
    Ok(())
}

/// Computes the `textDocument/formatting` edit for adam-lang `source`.
///
/// - Postcondition: returns an empty `Vec` if `source` doesn't parse (`AdamAstParser::parse_str`
///   returns `Err`) or parses with any recovered syntax error (`Sheet.errors` non-empty) —
///   refusing to format code it can't fully understand, matching `rustfmt`. Otherwise returns
///   exactly one [`TextEdit`] replacing the whole document with [`format_sheet`]'s output.
/// - Complexity: O(n) in the length of `source` — parses, attaches trivia to, and formats the
///   whole sheet once, with no caching across calls.
fn format_edits(source: &str) -> Vec<TextEdit> {
    let mut parser = AdamAstParser::new();
    let mut sheet = match parser.parse_str(source) {
        Ok(sheet) if sheet.errors.is_empty() => sheet,
        _ => return Vec::new(),
    };
    attach_trivia(source, &mut sheet);
    vec![TextEdit {
        range: whole_document_range(),
        new_text: format_sheet(&sheet),
    }]
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
        let edits = format_edits("sheet s { cell a:i32=1 filter |x:i32| x; }");
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].new_text,
            "sheet s {\n    cell a: i32 = 1 filter |x: i32| x;\n}\n"
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
