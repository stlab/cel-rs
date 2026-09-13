//! Examples backing `book-src/style.md` (Chapter 9). See `src/lib.rs` for how these `.adm2`
//! files are wired into the book.

#[test]
fn canonical_formatting() {
    let source = include_str!("../book-src/examples/style/canonical_formatting.adm2");
    let mut ast_parser = adam_lang::AdamAstParser::new();
    let sheet = ast_parser.parse_str(source).unwrap();
    assert!(sheet.errors.is_empty());

    let formatted = adam_lang::format_sheet(&sheet, source);
    assert_eq!(
        formatted,
        "sheet s {\n    cell x: i32 = 1;\n    cell y: i32 = 2;\n}\n"
    );
}
