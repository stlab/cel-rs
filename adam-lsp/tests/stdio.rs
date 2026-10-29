//! Exercises the shipped stdio entry point, including transport-thread shutdown.

use std::io::BufReader;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use lsp_server::{Message, Notification, Request};
use serde_json::json;

/// Owns the fixture process and reaps it even when a protocol assertion fails.
struct ServerProcess(Child);

impl Drop for ServerProcess {
    /// Terminates any still-running fixture and waits for its process resources.
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Exits the binary after successful initialize, shutdown and exit messages.
#[test]
fn stdio_server_joins_transport_threads_after_exit() {
    let mut process = ServerProcess(
        Command::new(env!("CARGO_BIN_EXE_adam-lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let child = &mut process.0;
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    Message::Request(Request::new(1.into(), "initialize".into(), json!({})))
        .write(&mut input)
        .unwrap();
    Message::Notification(Notification::new("initialized".into(), json!({})))
        .write(&mut input)
        .unwrap();
    Message::Request(Request::new(2.into(), "shutdown".into(), json!(null)))
        .write(&mut input)
        .unwrap();
    Message::Notification(Notification::new("exit".into(), json!(null)))
        .write(&mut input)
        .unwrap();
    drop(input);
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("server did not release its transport threads after exit");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success(), "{status}");
    let Message::Response(response) = Message::read(&mut output).unwrap().unwrap() else {
        panic!("expected initialization response")
    };
    assert_eq!(response.id, 1.into());
    let capabilities = response.response_result.unwrap();
    assert_eq!(capabilities["capabilities"]["textDocumentSync"], 1);
    let Message::Response(response) = Message::read(&mut output).unwrap().unwrap() else {
        panic!("expected shutdown response")
    };
    assert_eq!(response.id, 2.into());
    assert!(response.response_result.is_ok(), "{response:?}");
    assert!(Message::read(&mut output).unwrap().is_none());
}
