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

/// Requires binding diagnostics to retain definition and invocation source owners.
#[test]
fn module_diagnostics_keep_definition_and_use_locations() {
    let mut modules = ModuleSet::new();
    modules
        .insert_source(
            "constraints".parse().unwrap(),
            "file:///definition.adm2",
            "sheet ordered(a: f64, b: f64) {}",
        )
        .unwrap();
    let path = "app".parse().unwrap();
    modules.insert_source(path, "file:///app.adm2", "use constraints::ordered; sheet app { cell x: i32; cell y: f64; relationship ordered(a: x, b: y); }").unwrap();
    let errors = diagnostics_for_module(&"app".parse().unwrap(), &modules);
    let files: Vec<_> = errors
        .iter()
        .flat_map(|d| &d.labels)
        .filter_map(|l| l.location.as_ref())
        .map(|l| l.source.file_name())
        .collect();
    assert!(files.contains(&"file:///definition.adm2"), "{errors:?}");
    assert!(files.contains(&"file:///app.adm2"), "{errors:?}");
}

/// Receives one bounded transport message, failing rather than hanging on regressions.
fn receive(client: &Connection) -> Message {
    client
        .receiver
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
}

/// Reads one diagnostic publication.
fn published(client: &Connection) -> lsp_types::PublishDiagnosticsParams {
    let Message::Notification(notification) = receive(client) else {
        panic!("expected publication")
    };
    assert_eq!(notification.method, "textDocument/publishDiagnostics");
    serde_json::from_value(notification.params).unwrap()
}

/// Sends an arbitrary notification through the actual LSP transport.
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
                return output;
            }
            other => panic!("unexpected server request: {other:?}"),
        }
    }
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
    assert!(matches!(receive(&client), Message::Response(_)));
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
    assert!(
        publications(&client)
            .values()
            .all(|p| p.diagnostics.is_empty())
    );
    notify(
        &client,
        "workspace/didChangeConfiguration",
        json!({"settings":{"moduleSourcesError":"read denied"}}),
    );
    assert!(
        publications(&client)
            .values()
            .all(|p| p.diagnostics[0].message.contains("read denied"))
    );
    notify(
        &client,
        "workspace/didChangeConfiguration",
        json!({"settings":{"moduleSources":records}}),
    );
    assert!(
        publications(&client)
            .values()
            .all(|p| p.diagnostics.is_empty())
    );
    notify(
        &client,
        "workspace/didChangeConfiguration",
        json!({"settings":{"moduleSources":[]}}),
    );
    let cleared = publications(&client);
    assert_eq!(cleared.len(), 2);
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
    assert!(matches!(receive(&client), Message::Response(_)));
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
    assert!(matches!(receive(&client), Message::Response(_)));
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
    assert!(published(&client).diagnostics.is_empty());
    assert!(published(&client).diagnostics.is_empty());
    client
        .sender
        .send(Message::Request(Request::new(
            2.into(),
            "shutdown".into(),
            json!(null),
        )))
        .unwrap();
    assert!(matches!(receive(&client), Message::Response(_)));
    notify(&client, "exit", json!(null));
    thread.join().unwrap().unwrap();
}
