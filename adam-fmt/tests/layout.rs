//! Verifies CLI/editor agreement on a real nested expression sheet.

use std::process::Command;

/// The CLI preserves the shared formatter's filter, empty-closure and range boundary decisions.
#[test]
fn boundary_layout_agrees_with_the_editor() {
    let source = format!(
        "sheet s {{ cell {} = {} + {} filter predicate; cell {} = || predicate; cell range = {}..={}; }}",
        "n".repeat(17),
        "a".repeat(30),
        "a".repeat(30),
        "m".repeat(88),
        "a".repeat(45),
        "b".repeat(45)
    );
    let expected = adam_lang::format_source(&source).unwrap();
    assert!(expected.lines().all(|line| line.len() <= 100), "{expected}");
    let directory = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let path = directory.path().join("boundaries.adm2");
    std::fs::write(&path, source).unwrap();
    for _ in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_adam-fmt"))
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
    }
}

/// Formats a project-local copy without changing the reference and agrees with the editor API.
#[test]
fn crop_cli_and_editor_formatting_agree_and_are_idempotent() {
    let source = include_str!("../../cel-euclid/examples/crop_straighten.adm2");
    let expected = adam_lang::format_source(source).unwrap();
    let directory = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let path = directory.path().join("crop.adm2");
    std::fs::write(&path, source).unwrap();
    for pass in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_adam-fmt"))
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
        if pass == 1 {
            assert!(output.stdout.is_empty());
        }
    }
    for comment in source
        .lines()
        .filter_map(|line| line.trim().strip_prefix("//"))
    {
        assert!(expected.contains(comment.trim()), "{comment}");
    }
    assert_eq!(adam_lang::format_source(&expected).unwrap(), expected);
}
