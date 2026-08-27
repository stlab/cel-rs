//! Examples backing `book-src/outputs.md` (Chapter 7). See `tests/tutorial.rs` for how these
//! are wired into the book.

#[test]
fn basic_output() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/outputs/basic_output.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();
    let area = parsed.cell_names["area"].0;
    assert_eq!(*parsed.read::<i32>(area).unwrap(), 200);
}

#[test]
fn output_cell_is_terminal() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/outputs/output_cell_is_terminal.adm2"
        ))
        .unwrap();
    let output = parsed.output_names["area"];
    let area_cell = parsed.output_cell(output).unwrap();
    let err = parsed.write(area_cell, 999_i32).unwrap_err();
    assert!(matches!(err, adam_rs::Error::TerminalCell));
}

#[test]
fn requirement_diagnostic() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/outputs/requirement_diagnostic.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();
    let output = parsed.output_names["area"];
    assert!(parsed.output_valid(output));

    let width = parsed.cell_names["width"].0;
    parsed.write(width, 50_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(!parsed.output_valid(output));
}

#[test]
fn multiple_requirements() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/outputs/multiple_requirements.adm2"
        ))
        .unwrap();
    let output = parsed.output_names["clamped"];
    let x = parsed.cell_names["x"].0;

    parsed.write(x, -10_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(parsed.violated_requirements(output).count(), 1);
    assert!(!parsed.output_valid(output));
}
