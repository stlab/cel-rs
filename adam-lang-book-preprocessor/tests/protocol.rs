//! Exercises the preprocessor's renderer-selection and JSON stdio boundaries.

use std::io::Write;
use std::process::{Command, Stdio};

/// Accepts HTML support queries and rejects unsupported or absent renderers.
#[test]
fn renderer_queries_report_process_status() {
    for (arguments, supported) in [
        (vec!["supports", "html"], true),
        (vec!["supports", "epub"], false),
        (vec!["supports"], false),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_mdbook-live-examples"))
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(output.status.success(), supported);
        assert!(output.stdout.is_empty());
    }
}

/// Serializes a transformed book and reports malformed JSON without a success payload.
#[test]
fn stdio_transforms_books_and_reports_invalid_input() {
    for (input, valid) in [
        (
            serde_json::json!([
                {
                    "root": env!("CARGO_MANIFEST_DIR"),
                    "config":{"book":{"src":"book-src"}},
                    "renderer":"html",
                    "mdbook_version":"0.5.0"
                },
                {"items":[]}
            ])
            .to_string(),
            true,
        ),
        ("not JSON".into(), false),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mdbook-live-examples"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.success(), valid);
        if valid {
            let book: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(book, serde_json::json!({"items":[]}));
        } else {
            assert!(output.stdout.is_empty());
            assert!(!output.stderr.is_empty());
        }
    }
}
