//! Examples backing `book-src/source.md` (Chapter 3). See `src/lib.rs` for how these `.adm2`
//! files are wired into the book.

#[test]
fn basic_source() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/source/basic_source.adm2"
        ))
        .unwrap();
    let width = parsed.cell_names["width"].0;
    assert_eq!(parsed.cell_kind(width), Some(adam_rs::CellKind::Source));

    parsed.propagate().unwrap();
    assert!(parsed.is_source(width)); // always a source, never claimed

    let area = parsed.output_names["area"];
    assert_eq!(*parsed.read::<i32>(area).unwrap(), 1920 * 1080);

    // A source cell is written directly, exactly like a plain cell.
    parsed.write(width, 3840_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(parsed.is_source(width));
    assert_eq!(*parsed.read::<i32>(area).unwrap(), 3840 * 1080);
}

#[test]
fn source_cannot_be_derived() {
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/source/source_cannot_be_derived.adm2"
        ))
        .err()
        .unwrap();
    // `Error::InvalidCellKind`'s Display text is stale for this case (still says "terminal
    // output", which predates `out` becoming non-terminal and never mentions `source` at all —
    // see https://github.com/stlab/cel-rs/issues/166), so this test only asserts that the sheet
    // is rejected at parse time, not the message text.
    let _ = err;
}

#[test]
fn source_with_a_requirement() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/source/source_with_a_requirement.adm2"
        ))
        .unwrap();
    let width = parsed.cell_names["width"].0;

    parsed.propagate().unwrap();
    assert!(parsed.cell_requirements_valid(width));

    parsed.write(width, -1_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(!parsed.cell_requirements_valid(width));
}
