//! Wires `lsp-server`'s stdio transport to [`crate::diagnostics::diagnostics_for_source`].

use lsp_server::{Connection, Message, Notification as ServerNotification, Response};
use lsp_types::{
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, PublishDiagnosticsParams,
    ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
    notification::{
        DidChangeTextDocument, DidOpenTextDocument, Notification as _, PublishDiagnostics,
    },
};

use crate::diagnostics::diagnostics_for_source;

/// The JSON-RPC "Method not found" error code, reused by LSP for unhandled request methods.
const METHOD_NOT_FOUND: i32 = -32601;

/// Runs the pm-lang language server on stdin/stdout until the client sends `exit`.
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
fn main_loop(connection: &Connection) -> anyhow::Result<()> {
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                let response = Response::new_err(
                    req.id.clone(),
                    METHOD_NOT_FOUND,
                    format!("unhandled method: {}", req.method),
                );
                connection.sender.send(Message::Response(response))?;
            }
            Message::Notification(not) => handle_notification(connection, not)?,
            Message::Response(_) => {}
        }
    }
    Ok(())
}

/// Handles one client notification, publishing fresh diagnostics on `didOpen`/`didChange`.
///
/// # Errors
///
/// Returns `Err` only if sending the resulting `publishDiagnostics` notification fails (a broken
/// transport). A `didOpen`/`didChange` notification whose params fail to deserialize is logged to
/// stderr and skipped rather than propagated, so one malformed client message can't take down the
/// server.
fn handle_notification(connection: &Connection, not: ServerNotification) -> anyhow::Result<()> {
    match not.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let params: DidOpenTextDocumentParams = match not.extract(DidOpenTextDocument::METHOD) {
                Ok(params) => params,
                Err(error) => {
                    eprintln!(
                        "pm-lsp: ignoring malformed {}: {error}",
                        DidOpenTextDocument::METHOD
                    );
                    return Ok(());
                }
            };
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
                            "pm-lsp: ignoring malformed {}: {error}",
                            DidChangeTextDocument::METHOD
                        );
                        return Ok(());
                    }
                };
            if let Some(change) = params.content_changes.into_iter().last() {
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

#[cfg(test)]
mod tests {
    use lsp_server::{Connection, Message, Notification as ServerNotification, Request, RequestId};
    use lsp_types::{
        DidChangeTextDocumentParams, DidOpenTextDocumentParams, PublishDiagnosticsParams,
        TextDocumentContentChangeEvent, TextDocumentItem, VersionedTextDocumentIdentifier,
        notification::{
            DidChangeTextDocument, DidOpenTextDocument, Notification as _, PublishDiagnostics,
        },
    };

    use super::serve;

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
                        language_id: "pm-lang".to_string(),
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
                        language_id: "pm-lang".to_string(),
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
                        language_id: "pm-lang".to_string(),
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
