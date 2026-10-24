//! Exercises expression layout through the public sheet formatter.

use adam_lang::format_source;

/// Declaration prefixes and the terminating semicolon participate in exact-width decisions.
#[test]
fn declaration_prefix_and_semicolon_determine_exact_fit() {
    let operand = "a".repeat(30);
    for (length, wraps) in [(24, false), (25, true)] {
        let source = format!(
            "sheet s {{ cell {} = {operand} + {operand}; }}",
            "n".repeat(length)
        );
        let text = format_source(&source).unwrap();
        assert_eq!(text.contains("\n        + "), wraps, "{text}");
        assert!(text.lines().all(|line| line.len() <= 100), "{text}");
        assert_eq!(format_source(&text).unwrap(), text);
    }
}

/// Filter bodies and empty closures can start an indented continuation when their first token
/// cannot fit after an otherwise short initializer or declaration prefix.
#[test]
fn filter_and_empty_closure_boundaries_respect_the_width() {
    for declaration in [
        format!(
            "cell {} = {} + {} filter predicate;",
            "n".repeat(17),
            "a".repeat(30),
            "a".repeat(30)
        ),
        format!(
            "source {} = {} + {} filter predicate;",
            "n".repeat(17),
            "a".repeat(30),
            "a".repeat(30)
        ),
        format!(
            "out {} := {} + {} filter predicate;",
            "n".repeat(17),
            "a".repeat(30),
            "a".repeat(30)
        ),
        format!(
            "cell {} = {} + {} filter || predicate;",
            "n".repeat(17),
            "a".repeat(30),
            "a".repeat(30)
        ),
        format!("cell {} = || predicate;", "n".repeat(88)),
        format!(
            "cell {} = {} + {} /* before */ filter /* after */ || predicate;",
            "n".repeat(17),
            "a".repeat(17),
            "a".repeat(17)
        ),
    ] {
        let source = format!("sheet s {{ {declaration} }}");
        let text = format_source(&source).unwrap();
        assert!(
            text.lines().all(|line| line.chars().count() <= 100),
            "{text}"
        );
        assert!(text.lines().all(|line| line.trim_end() == line), "{text}");
        for label in ["before", "after"] {
            if source.contains(label) {
                assert_eq!(text.matches(label).count(), 1, "{text}");
            }
        }
        assert_eq!(format_source(&text).unwrap(), text);
    }
}

/// Range endpoints break before their compact operator in declarations and nested arguments.
#[test]
fn ranges_respect_declaration_and_argument_widths() {
    for expression in [
        format!("{}..={}", "a".repeat(45), "b".repeat(45)),
        format!("call({}..={})", "a".repeat(45), "b".repeat(45)),
    ] {
        let text = format_source(&format!("sheet s {{ cell x = {expression}; }}")).unwrap();
        assert!(
            text.contains("\n        ..=") || text.contains("\n            ..="),
            "{text}"
        );
        assert!(text.lines().all(|line| line.len() <= 100), "{text}");
        assert_eq!(format_source(&text).unwrap(), text);
    }
}

/// Every expression-bearing declaration uses its actual emitted prefix.
#[test]
fn expression_sites_respect_the_sheet_width() {
    let operand = "a".repeat(35);
    let expr = format!("{operand} + {operand} + {operand}");
    for declaration in [
        format!("cell value = {expr};"),
        format!("source value = {expr};"),
        format!("out value := {expr};"),
        format!("cell value = 0 filter |v: i32| {expr};"),
        format!("cell value = 0 require {{ @named {expr}; }};"),
        format!("relationship {{ value := {expr}; }}"),
        format!("conditional {expr} {{ _ => {{}} }}"),
    ] {
        let source = format!("sheet s {{ {declaration} }}");
        let text = format_source(&source).unwrap();
        assert!(text.lines().all(|line| line.len() <= 100), "{text}");
        assert_eq!(format_source(&text).unwrap(), text);
    }
}

/// The nested closure reference reparses, retains its comments, and reaches a fixed point.
#[test]
fn crop_reference_reparses_without_losing_comments() {
    let source = include_str!("../../cel-euclid/examples/crop_straighten.adm2");
    let text = format_source(source).unwrap();
    for line in text.lines() {
        let code = line.split("//").next().unwrap().trim_end();
        assert!(
            code.chars().count() <= 100,
            "{} columns: {line}",
            code.chars().count()
        );
    }
    for line in source
        .lines()
        .filter_map(|line| line.trim().strip_prefix("//"))
    {
        assert!(text.contains(line.trim()), "missing comment: {line}");
    }
    assert_eq!(format_source(&text).unwrap(), text);
}
