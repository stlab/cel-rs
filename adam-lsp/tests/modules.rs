//! Verifies caller-owned module checking and its editor transport.

use adam_lang::{ModulePath, ModuleSet};
use adam_lsp::diagnostics::diagnostics_for_module;
use lsp_server::{Connection, Message, Notification, Request};
use serde_json::json;
use std::time::Duration;

/// Checks explicit modules with the editor's installed operation libraries.
#[test]
fn module_diagnostics_check_bindings_branches_imports_and_libraries() {
    let path: ModulePath = "app".parse().unwrap();
    for (definition, source, valid) in [
        (
            "sheet ordered(a: f64, b: f64) {}",
            "use constraints::ordered as ord; sheet app { cell x: f64; cell y: f64; relationship ord(b: y, a: x); }",
            true,
        ),
        (
            "sheet ordered(a: f64, b: f64) {}",
            "use constraints::ordered as ord; sheet app { cell x: i32; cell y: f64; relationship ord(a: x, b: y); }",
            false,
        ),
        (
            "sheet ordered(a: f64, b: f64) { cell private: i32; }",
            "use constraints::ordered; sheet app { cell x: f64; cell y: f64; conditional false { true => { relationship ordered(a: x, b: y); } } }",
            false,
        ),
        (
            "sheet ordered(a: f64, b: f64) {}",
            "use missing::ordered; sheet app {}",
            false,
        ),
        (
            "sheet ordered(a: f64, b: f64) {}",
            "sheet app { out p := Point2D { x: 1.0, y: 2.0 }; out n := min(3.0, 4.0); }",
            true,
        ),
        (
            "sheet ordered(a: f64, b: f64) {}",
            "sheet app { cell x: i32 = 1 / 0; conditional (1 / 0) { _ => {} } }",
            true,
        ),
    ] {
        let mut modules = ModuleSet::new();
        modules
            .insert_source(
                "constraints".parse().unwrap(),
                "file:///constraints.adm2",
                definition,
            )
            .unwrap();
        modules
            .insert_source(path.clone(), "file:///app.adm2", source)
            .unwrap();
        let errors = diagnostics_for_module(&path, &modules);
        assert_eq!(errors.is_empty(), valid, "{errors:?}");
    }
    assert!(!diagnostics_for_module(&path, &ModuleSet::new()).is_empty());
}

/// Receives one bounded transport message, failing rather than hanging on regressions.
fn receive(client: &Connection) -> Message {
    client
        .receiver
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
}

/// Reads one diagnostic publication.
///
/// - Complexity: O(n) time and space for n serialized publication bytes.
fn published(client: &Connection) -> lsp_types::PublishDiagnosticsParams {
    let Message::Notification(notification) = receive(client) else {
        panic!("expected publication")
    };
    assert_eq!(notification.method, "textDocument/publishDiagnostics");
    serde_json::from_value(notification.params).unwrap()
}

/// Sends an arbitrary notification through the actual LSP transport.
///
/// - Complexity: O(n) time and space for n method and parameter bytes.
fn notify(client: &Connection, method: &str, params: serde_json::Value) {
    client
        .sender
        .send(Message::Notification(Notification::new(
            method.into(),
            params,
        )))
        .unwrap();
}

/// Collects complete publications preceding an ordered request barrier by source URI.
///
/// - Complexity: O(b + p log(p + 1) * u) time and O(b + p) space for b publication
///   bytes, p publications and maximum URI length u.
fn publications(
    client: &Connection,
) -> std::collections::BTreeMap<String, lsp_types::PublishDiagnosticsParams> {
    client
        .sender
        .send(Message::Request(Request::new(
            99.into(),
            "test/barrier".into(),
            json!(null),
        )))
        .unwrap();
    let mut output = std::collections::BTreeMap::new();
    loop {
        match receive(client) {
            Message::Notification(notification) => {
                assert_eq!(notification.method, "textDocument/publishDiagnostics");
                let params: lsp_types::PublishDiagnosticsParams =
                    serde_json::from_value(notification.params).unwrap();
                output.insert(params.uri.as_str().to_owned(), params);
            }
            Message::Response(response) => {
                assert_eq!(response.id, 99.into());
                assert_eq!(response.response_result.unwrap_err().code, -32601);
                return output;
            }
            other => panic!("unexpected server request: {other:?}"),
        }
    }
}

/// Starts the real transport with an explicit caller-owned source collection.
///
/// - Complexity: initialization analysis plus O(n) wire storage for n source bytes.
fn start_modules(
    records: serde_json::Value,
) -> (Connection, std::thread::JoinHandle<anyhow::Result<()>>) {
    let (server, client) = Connection::memory();
    let thread = std::thread::spawn(move || adam_lsp::serve(&server));
    client
        .sender
        .send(Message::Request(Request::new(
            1.into(),
            "initialize".into(),
            json!({"initializationOptions":{"moduleSources":records}}),
        )))
        .unwrap();
    successful_response(&client, 1);
    notify(&client, "initialized", json!({}));
    (client, thread)
}

/// Completes the in-memory shutdown handshake and joins the owned server task.
///
/// - Complexity: waits for pending server dispatch and shutdown completion.
fn stop_modules(client: &Connection, thread: std::thread::JoinHandle<anyhow::Result<()>>) {
    client
        .sender
        .send(Message::Request(Request::new(
            2.into(),
            "shutdown".into(),
            json!(null),
        )))
        .unwrap();
    successful_response(client, 2);
    notify(client, "exit", json!(null));
    thread.join().unwrap().unwrap();
}

/// Requires the next transport response to succeed with the requested ID.
///
/// - Complexity: O(n) space for the received response payload.
fn successful_response(client: &Connection, id: i32) {
    let Message::Response(response) = receive(client) else {
        panic!("expected successful response")
    };
    assert_eq!(response.id, id.into());
    assert!(response.response_result.is_ok(), "{response:?}");
}

/// Requires exactly the expected publication owners, including clean owners.
///
/// - Complexity: O(p log(p + 1) * u) time and O(p) space for p owners of maximum u bytes.
fn expect_owners(
    output: &std::collections::BTreeMap<String, lsp_types::PublishDiagnosticsParams>,
    expected: &[&str],
) {
    let owners: std::collections::BTreeSet<_> = output.keys().map(String::as_str).collect();
    assert_eq!(owners, expected.iter().copied().collect());
}

/// Preserves parsed Unicode token locations through owning diagnostics and UTF-16 transport.
#[test]
fn transport_unicode_binding_locations_match_original_tokens() {
    let definition = "// retained first line\n/* 😀 é */ sheet copy(entrée: f64, b: f64) {}";
    let caller = "// independent first line\n/* 🦀 é */ use library::copy; sheet app { cell café: i32; cell y: f64; relationship copy(entrée: café, b: y); }";
    let mut modules = ModuleSet::new();
    modules
        .insert_source(
            "library".parse().unwrap(),
            "file:///unicode-library.adm2",
            definition,
        )
        .unwrap();
    modules
        .insert_source("app".parse().unwrap(), "file:///unicode-app.adm2", caller)
        .unwrap();
    let errors = diagnostics_for_module(&"app".parse().unwrap(), &modules);
    drop(modules);
    let error = errors
        .iter()
        .find(|error| error.message == "component argument type mismatch")
        .unwrap();
    for (uri, text, token) in [
        ("file:///unicode-library.adm2", definition, "entrée"),
        ("file:///unicode-app.adm2", caller, "café"),
    ] {
        let location = error
            .labels
            .iter()
            .filter_map(|label| label.location.as_ref())
            .find(|location| location.source.file_name() == uri)
            .unwrap();
        assert_eq!(location.source.text(), text);
        assert_eq!(&text[location.span.to_byte_range(text)], token);
        let offset = text.rfind(token).unwrap();
        let line = text[..offset].rsplit('\n').next().unwrap();
        assert_eq!(location.span.start.line, 2);
        assert_eq!(location.span.start.column, line.chars().count());
        assert_eq!(
            location.span.end.column,
            line.chars().count() + token.chars().count()
        );
    }

    let (client, thread) = start_modules(json!([
        {"path":"library","uri":"file:///unicode-library.adm2","text":definition},
        {"path":"app","uri":"file:///unicode-app.adm2","text":caller}
    ]));
    notify(
        &client,
        "textDocument/didOpen",
        json!({"textDocument":{"uri":"file:///unicode-app.adm2","languageId":"adam-lang","version":1,"text":caller}}),
    );
    let output = publications(&client);
    stop_modules(&client, thread);
    for (uri, text, token) in [
        ("file:///unicode-library.adm2", definition, "entrée"),
        ("file:///unicode-app.adm2", caller, "café"),
    ] {
        let diagnostic = output[uri]
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == error.message)
            .unwrap();
        let offset = text.rfind(token).unwrap();
        let column = text[..offset]
            .rsplit('\n')
            .next()
            .unwrap()
            .encode_utf16()
            .count() as u32;
        let expected = lsp_types::Range::new(
            lsp_types::Position::new(1, column),
            lsp_types::Position::new(1, column + token.encode_utf16().count() as u32),
        );
        assert_eq!(diagnostic.range, expected, "{uri}");
        let related = diagnostic.related_information.as_ref().unwrap();
        assert_eq!(related.len(), 2);
        assert!(
            related
                .iter()
                .any(|related| related.location.uri.as_str() == uri
                    && related.location.range == expected),
            "{related:?}"
        );
    }
}

/// Publishes exact UTF-16 error ranges for documents outside the explicit module snapshot.
#[test]
fn transport_unregistered_unicode_source_uses_utf16() {
    let source = "// first line\n/* 😀 é */ sheet app { cell café: i32 = 1.0; }";
    let (client, thread) = start_modules(json!([]));
    notify(
        &client,
        "textDocument/didOpen",
        json!({"textDocument":{"uri":"file:///standalone.adm2","languageId":"adam-lang","version":1,"text":source}}),
    );
    let output = publications(&client);
    stop_modules(&client, thread);
    let diagnostics = &output["file:///standalone.adm2"].diagnostics;
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    let offset = source.find("1.0").unwrap();
    let column = source[..offset]
        .rsplit('\n')
        .next()
        .unwrap()
        .encode_utf16()
        .count() as u32;
    assert_eq!(
        diagnostics[0].range,
        lsp_types::Range::new(
            lsp_types::Position::new(1, column),
            lsp_types::Position::new(1, column + 3),
        )
    );
}

/// A malformed definition cannot clear independent registered errors or hide incomplete checking.
#[test]
fn transport_malformed_definition_preserves_independent_errors_and_recovers() {
    let records = json!([
        {"path":"lib","uri":"file:///lib.adm2","text":"sheet pair(a: f64) {}"},
        {"path":"app","uri":"file:///app.adm2","text":"use lib::pair; sheet app { cell x: i32; relationship pair(a: x); }"},
        {"path":"unrelated","uri":"file:///unrelated.adm2","text":"sheet unrelated { cell x: i32 = true; }"}
    ]);
    let (client, thread) = start_modules(records.clone());
    notify(
        &client,
        "workspace/didChangeConfiguration",
        json!({"settings":{"moduleSources":records}}),
    );
    let baseline = publications(&client);
    let independent_messages: Vec<_> = baseline["file:///unrelated.adm2"]
        .diagnostics
        .iter()
        .filter(|d| !d.message.contains("incomplete"))
        .map(|d| d.message.clone())
        .collect();
    assert!(!independent_messages.is_empty(), "{baseline:?}");
    assert!(
        !baseline["file:///app.adm2"].diagnostics.is_empty(),
        "{baseline:?}"
    );
    notify(
        &client,
        "textDocument/didOpen",
        json!({"textDocument":{"uri":"file:///lib.adm2","languageId":"adam-lang","version":1,"text":"use"}}),
    );
    let broken = publications(&client);
    assert!(
        independent_messages
            .iter()
            .any(|message| broken["file:///unrelated.adm2"]
                .diagnostics
                .iter()
                .any(|d| &d.message == message)),
        "{broken:?}"
    );
    assert!(
        broken["file:///app.adm2"]
            .diagnostics
            .iter()
            .any(|d| d.message.contains("incomplete")),
        "{broken:?}"
    );
    assert!(
        broken["file:///lib.adm2"]
            .diagnostics
            .iter()
            .any(|diagnostic| {
                diagnostic
                    .related_information
                    .as_ref()
                    .is_some_and(|related| {
                        related
                            .iter()
                            .any(|label| label.location.uri.as_str() == "file:///lib.adm2")
                    })
            }),
        "{broken:?}"
    );
    notify(
        &client,
        "textDocument/didChange",
        json!({"textDocument":{"uri":"file:///lib.adm2","version":2},"contentChanges":[{"text":"sheet pair(a: f64) {}"}]}),
    );
    let restored = publications(&client);
    assert!(independent_messages.iter().any(|message| {
        restored["file:///unrelated.adm2"]
            .diagnostics
            .iter()
            .any(|d| &d.message == message)
    }));
    assert!(restored["file:///app.adm2"].diagnostics.iter().any(|d| {
        d.related_information.as_ref().is_some_and(|related| {
            related
                .iter()
                .any(|r| r.location.uri.as_str() == "file:///lib.adm2")
        })
    }));
    notify(
        &client,
        "textDocument/didChange",
        json!({"textDocument":{"uri":"file:///lib.adm2","version":3},"contentChanges":[{"text":"sheet pair(a: i32) {}"}]}),
    );
    let fixed = publications(&client);
    assert!(
        fixed["file:///app.adm2"].diagnostics.is_empty(),
        "{fixed:?}"
    );
    assert!(
        fixed["file:///lib.adm2"].diagnostics.is_empty(),
        "{fixed:?}"
    );
    assert!(!fixed["file:///unrelated.adm2"].diagnostics.is_empty());
    stop_modules(&client, thread);
}

/// Only an explicit array replaces/removes ownership after successful registration.
#[test]
fn transport_incomplete_refresh_retains_owners_until_explicit_removal() {
    let records = json!([
        {"path":"lib","uri":"file:///lib.adm2","text":"sheet pair(a: f64) {}"},
        {"path":"app","uri":"file:///app.adm2","text":"use lib::pair; sheet app { cell x: i32; relationship pair(a: x); }"}
    ]);
    let (client, thread) = start_modules(records.clone());
    notify(
        &client,
        "textDocument/didOpen",
        json!({"textDocument":{"uri":"file:///app.adm2","languageId":"adam-lang","version":1,"text":"use lib::pair; sheet app { cell x: i32; relationship pair(a: x); }"}}),
    );
    assert!(
        !publications(&client)["file:///app.adm2"]
            .diagnostics
            .is_empty()
    );
    for settings in [
        json!({}),
        json!(null),
        json!({"editor":{"tabSize":4}}),
        json!({"moduleSource":records}),
        json!({"moduleSources":null}),
    ] {
        notify(
            &client,
            "workspace/didChangeConfiguration",
            json!({"settings":settings}),
        );
        let failed = publications(&client);
        assert!(
            !failed["file:///lib.adm2"].diagnostics.is_empty(),
            "{settings}: {failed:?}"
        );
        assert!(
            failed["file:///app.adm2"]
                .diagnostics
                .iter()
                .all(|d| d.source.as_deref() == Some("adam-lang configuration")),
            "{settings}: {failed:?}"
        );
        assert!(
            !failed["file:///app.adm2"].diagnostics.is_empty(),
            "{settings}: {failed:?}"
        );
        notify(
            &client,
            "workspace/didChangeConfiguration",
            json!({"settings":{"moduleSources":records}}),
        );
        let recovered = publications(&client);
        assert!(recovered["file:///app.adm2"].diagnostics.iter().any(|d| {
            d.related_information.as_ref().is_some_and(|related| {
                related
                    .iter()
                    .any(|r| r.location.uri.as_str() == "file:///lib.adm2")
            })
        }));
    }
    notify(
        &client,
        "workspace/didChangeConfiguration",
        json!({"settings":{"moduleSources":[]}}),
    );
    let removed = publications(&client);
    assert!(removed["file:///lib.adm2"].diagnostics.is_empty());
    assert!(
        removed["file:///app.adm2"]
            .diagnostics
            .iter()
            .all(|d| d.source.as_deref() != Some("adam-lang configuration"))
    );
    notify(
        &client,
        "workspace/didChangeConfiguration",
        json!({"settings":{"moduleSources":records}}),
    );
    let restored = publications(&client);
    assert!(
        restored["file:///app.adm2"]
            .diagnostics
            .iter()
            .any(|diagnostic| {
                diagnostic
                    .related_information
                    .as_ref()
                    .is_some_and(|related| {
                        related
                            .iter()
                            .any(|label| label.location.uri.as_str() == "file:///lib.adm2")
                    })
            }),
        "{restored:?}"
    );
    stop_modules(&client, thread);
}

/// Close, source/configuration failure and removal all replace stale transport diagnostics.
#[test]
fn transport_close_failures_recovery_and_removal_are_consistent() {
    let (server, client) = Connection::memory();
    let thread = std::thread::spawn(move || adam_lsp::serve(&server));
    let records = json!([
        {"path":"lib","uri":"file:///lib.adm2","text":"sheet pair(a: f64) {}"},
        {"path":"app","uri":"file:///app.adm2","text":"use lib::pair; sheet app { cell x: f64; relationship pair(a: x); }"}
    ]);
    client
        .sender
        .send(Message::Request(Request::new(
            1.into(),
            "initialize".into(),
            json!({"initializationOptions":{"moduleSources":records}}),
        )))
        .unwrap();
    successful_response(&client, 1);
    notify(&client, "initialized", json!({}));
    notify(
        &client,
        "textDocument/didOpen",
        json!({"textDocument":{"uri":"file:///lib.adm2","languageId":"adam-lang","version":1,"text":"sheet pair(a: i32) {}"}}),
    );
    assert!(
        !publications(&client)["file:///app.adm2"]
            .diagnostics
            .is_empty()
    );
    notify(
        &client,
        "textDocument/didClose",
        json!({"textDocument":{"uri":"file:///lib.adm2"}}),
    );
    let closed = publications(&client);
    expect_owners(&closed, &["file:///app.adm2", "file:///lib.adm2"]);
    assert!(closed.values().all(|p| p.diagnostics.is_empty()));
    notify(
        &client,
        "workspace/didChangeConfiguration",
        json!({"settings":{"moduleSourcesError":"read denied"}}),
    );
    let failed = publications(&client);
    expect_owners(&failed, &["file:///app.adm2", "file:///lib.adm2"]);
    assert!(
        failed
            .values()
            .all(|p| p.diagnostics[0].message.contains("read denied"))
    );
    notify(
        &client,
        "workspace/didChangeConfiguration",
        json!({"settings":{"moduleSources":records}}),
    );
    let recovered = publications(&client);
    expect_owners(&recovered, &["file:///app.adm2", "file:///lib.adm2"]);
    assert!(recovered.values().all(|p| p.diagnostics.is_empty()));
    notify(
        &client,
        "workspace/didChangeConfiguration",
        json!({"settings":{"moduleSources":[]}}),
    );
    let cleared = publications(&client);
    expect_owners(&cleared, &["file:///app.adm2", "file:///lib.adm2"]);
    assert!(cleared.values().all(|p| p.diagnostics.is_empty()));
    notify(
        &client,
        "textDocument/didOpen",
        json!({"textDocument":{"uri":"file:///standalone.adm2","languageId":"adam-lang","version":1,"text":"sheet good {}"}}),
    );
    assert!(
        publications(&client)["file:///standalone.adm2"]
            .diagnostics
            .is_empty()
    );
    notify(
        &client,
        "workspace/didChangeConfiguration",
        json!({"settings":{"moduleSources":42}}),
    );
    assert!(
        publications(&client)["file:///standalone.adm2"].diagnostics[0]
            .message
            .contains("array")
    );
    notify(
        &client,
        "workspace/didChangeConfiguration",
        json!({"settings":{"moduleSources":[]}}),
    );
    assert!(
        publications(&client)["file:///standalone.adm2"]
            .diagnostics
            .is_empty()
    );
    notify(
        &client,
        "textDocument/didClose",
        json!({"textDocument":{"uri":"file:///standalone.adm2"}}),
    );
    assert!(
        publications(&client)["file:///standalone.adm2"]
            .diagnostics
            .is_empty()
    );
    client
        .sender
        .send(Message::Request(Request::new(
            2.into(),
            "shutdown".into(),
            json!(null),
        )))
        .unwrap();
    successful_response(&client, 2);
    notify(&client, "exit", json!(null));
    thread.join().unwrap().unwrap();
}

/// Registers owned sources, overlays edits by URI and refreshes dependent locations.
#[test]
fn transport_republishes_dependents_on_definition_change_and_configuration_refresh() {
    let (server, client) = Connection::memory();
    let thread = std::thread::spawn(move || adam_lsp::serve(&server));
    let records = json!([
        {"path":"constraints","uri":"file:///definition.adm2","text":"sheet ordered(a: f64, b: f64) {}"},
        {"path":"app","uri":"file:///app.adm2","text":"use constraints::ordered as ord; sheet app { cell x: f64; cell y: f64; relationship ord(a: x, b: y); }"}
    ]);
    client
        .sender
        .send(Message::Request(Request::new(
            1.into(),
            "initialize".into(),
            json!({"initializationOptions":{"moduleSources": records}}),
        )))
        .unwrap();
    successful_response(&client, 1);
    notify(&client, "initialized", json!({}));
    notify(
        &client,
        "textDocument/didOpen",
        json!({"textDocument":{"uri":"file:///definition.adm2","languageId":"adam-lang","version":1,"text":"sheet ordered(a: i32, b: f64) {}"}}),
    );
    let publications = [published(&client), published(&client)];
    let app = publications
        .iter()
        .find(|p| p.uri.as_str() == "file:///app.adm2")
        .unwrap();
    assert!(!app.diagnostics.is_empty(), "{publications:?}");
    assert!(
        app.diagnostics
            .iter()
            .flat_map(|d| d.related_information.iter().flatten())
            .any(|r| r.location.uri.as_str() == "file:///definition.adm2")
    );
    // A snapshot refresh cannot overwrite the open definition's actual source.
    notify(
        &client,
        "workspace/didChangeConfiguration",
        json!({"settings":{"moduleSources":records}}),
    );
    let publications = [published(&client), published(&client)];
    assert!(
        publications
            .iter()
            .any(|p| p.uri.as_str() == "file:///app.adm2" && !p.diagnostics.is_empty())
    );
    notify(
        &client,
        "textDocument/didChange",
        json!({"textDocument":{"uri":"file:///definition.adm2","version":2},"contentChanges":[{"text":"sheet ordered(a: f64, b: f64) {}"}]}),
    );
    let fixed = [published(&client), published(&client)];
    let owners: std::collections::BTreeSet<_> = fixed.iter().map(|p| p.uri.as_str()).collect();
    assert_eq!(
        owners,
        ["file:///app.adm2", "file:///definition.adm2"]
            .into_iter()
            .collect()
    );
    assert!(fixed.iter().all(|p| p.diagnostics.is_empty()));
    client
        .sender
        .send(Message::Request(Request::new(
            2.into(),
            "shutdown".into(),
            json!(null),
        )))
        .unwrap();
    successful_response(&client, 2);
    notify(&client, "exit", json!(null));
    thread.join().unwrap().unwrap();
}
