//! Examples backing `book-src/cells.md` (Chapter 2). See `tests/tutorial.rs` for how these are
//! wired into the book.

#[test]
fn type_mismatch_is_a_parse_error() {
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/cells/type_mismatch_is_a_parse_error.adm2"
        ))
        .err()
        .unwrap();
    assert!(format!("{err}").contains("type mismatch"));
}

#[test]
fn tuple_typed_cell() {
    let mut parser = adam_lang_book::support::parser();
    let parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/cells/tuple_typed_cell.adm2"
        ))
        .unwrap();
    let point = parsed.cell_names["point"].0;
    let value = parsed
        .read::<cel_runtime::DynamicSequence>(point)
        .unwrap()
        .clone();
    assert_eq!(value.try_to_tuple::<(f64, f64)>().unwrap(), (0.0, 0.0));
}

#[test]
fn no_forward_references() {
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/cells/no_forward_references.adm2"
        ))
        .err()
        .unwrap();
    assert!(format!("{err}").contains("undeclared cell"));
}
