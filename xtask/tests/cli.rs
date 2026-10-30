//! Checks task dispatch without running downloads or rebuilding generated assets.

use std::process::Command;

/// Reports absent and unknown tasks as usage errors without writing success output.
#[test]
fn absent_and_unknown_tasks_fail_with_usage() {
    for arguments in [vec![], vec!["unknown-task"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("Usage: cargo xtask")
        );
    }
}
