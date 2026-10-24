//! Characterizes the formatter's exit status, diagnostics, and in-place file updates.

use std::process::Command;

/// Rejects an empty argument list with usage on stderr and no stdout.
#[test]
fn no_arguments_reports_usage_and_failure() {
    let output = Command::new(env!("CARGO_BIN_EXE_adam-fmt"))
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"usage: adam-fmt <file.adm2>...\n");
}

/// Reports changed files, leaves formatted files silent, and succeeds without errors.
#[test]
fn successful_files_report_only_changes() {
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let changed = dir.path().join("changed.adm2");
    let unchanged = dir.path().join("unchanged.adm2");
    let formatted = "sheet s {\n    cell x: i32 = 1;\n}\n";
    std::fs::write(&changed, "sheet s{cell x:i32=1;}").unwrap();
    std::fs::write(&unchanged, formatted).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_adam-fmt"))
        .args([&changed, &unchanged])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("formatted {}\n", changed.display())
    );
    assert_eq!(std::fs::read_to_string(&changed).unwrap(), formatted);
    assert_eq!(std::fs::read_to_string(&unchanged).unwrap(), formatted);
}

/// Continues after read and format errors, preserves invalid input, and reports failure.
#[test]
fn mixed_files_continue_after_each_error() {
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let missing = dir.path().join("missing.adm2");
    let first = dir.path().join("first.adm2");
    let invalid = dir.path().join("invalid.adm2");
    let last = dir.path().join("last.adm2");
    let source = "sheet s{cell x:i32=1;}";
    let formatted = "sheet s {\n    cell x: i32 = 1;\n}\n";
    std::fs::write(&first, source).unwrap();
    std::fs::write(&invalid, "not a sheet at all").unwrap();
    std::fs::write(&last, source).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_adam-fmt"))
        .args([&missing, &first, &invalid, &last])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "formatted {}\nformatted {}\n",
            first.display(),
            last.display()
        )
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains(&format!("adam-fmt: {}:", missing.display())));
    assert!(stderr.contains(invalid.to_str().unwrap()));
    assert!(!missing.exists());
    assert_eq!(std::fs::read_to_string(&first).unwrap(), formatted);
    assert_eq!(std::fs::read_to_string(&last).unwrap(), formatted);
    assert_eq!(
        std::fs::read_to_string(&invalid).unwrap(),
        "not a sheet at all"
    );
}
