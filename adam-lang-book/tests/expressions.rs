//! Examples backing `book-src/expressions.md` (Chapter 3). See `tests/tutorial.rs` for how
//! these are wired into the book.

#[test]
fn no_standard_library() {
    // ANCHOR: no_standard_library
    let mut parser =
        adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(), cel_parser::OpLookup::new());
    let err = parser
        .parse_str("sheet s { cell x: i32 = min(1, 2); }")
        .err()
        .unwrap();
    assert!(format!("{err}").to_lowercase().contains("min"));
    // ANCHOR_END: no_standard_library
}

#[test]
fn initializer_sees_no_cells() {
    // ANCHOR: initializer_sees_no_cells
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str("sheet s { cell x = 1; cell y = x + 1; }")
        .err()
        .unwrap();
    assert!(format!("{err}").to_lowercase().contains("x"));
    // ANCHOR_END: initializer_sees_no_cells
}
