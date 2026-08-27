//! Examples backing `book-src/cells.md` (Chapter 2). See `tests/tutorial.rs` for how these are
//! wired into the book.

#[test]
fn type_mismatch_is_a_parse_error() {
    // ANCHOR: type_mismatch_is_a_parse_error
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str("sheet s { cell x: i32 = 1.0; }")
        .err()
        .unwrap();
    assert!(format!("{err}").contains("type mismatch"));
    // ANCHOR_END: type_mismatch_is_a_parse_error
}

#[test]
fn tuple_typed_cell() {
    // ANCHOR: tuple_typed_cell
    let mut parser = adam_lang_book::support::parser();
    let parsed = parser
        .parse_str("sheet s { cell point: (f64, f64) = (0.0, 0.0); }")
        .unwrap();
    let point = parsed.cell_names["point"].0;
    let value = parsed
        .read::<cel_runtime::DynamicSequence>(point)
        .unwrap()
        .clone();
    assert_eq!(value.try_to_tuple::<(f64, f64)>().unwrap(), (0.0, 0.0));
    // ANCHOR_END: tuple_typed_cell
}

#[test]
fn no_forward_references() {
    // ANCHOR: no_forward_references
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str("sheet s { relationship { y := x; } cell x = 0; cell y = 0; } ")
        .err()
        .unwrap();
    assert!(format!("{err}").contains("undeclared cell"));
    // ANCHOR_END: no_forward_references
}
