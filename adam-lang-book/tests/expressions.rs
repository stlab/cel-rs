//! Examples backing `book-src/expressions.md` (Chapter 4). See `src/lib.rs` for how these
//! `.adm2` files are wired into the book.

#[test]
fn no_standard_library() {
    let mut parser =
        adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(), cel_parser::OpLookup::new());
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/expressions/no_standard_library.adm2"
        ))
        .err()
        .unwrap();
    assert!(format!("{err}").to_lowercase().contains("min"));
}

#[test]
fn initializer_sees_no_cells() {
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/expressions/initializer_sees_no_cells.adm2"
        ))
        .err()
        .unwrap();
    assert!(format!("{err}").to_lowercase().contains("x"));
}
