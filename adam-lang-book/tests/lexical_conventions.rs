//! Examples backing `book-src/lexical-conventions.md` (Chapter 10). See `src/lib.rs` for how
//! these `.adm2` files are wired into the book.

#[test]
fn doc_comments_are_preserved_by_the_formatter() {
    let mut ast_parser = adam_lang::AdamAstParser::new();
    let sheet = ast_parser
        .parse_str(include_str!(
            "../book-src/examples/lexical-conventions/doc_comments.adm2"
        ))
        .unwrap();
    assert!(sheet.errors.is_empty());

    let formatted = adam_lang::format_sheet(&sheet);
    assert!(formatted.contains("//! A sheet describing a simple resize dialog."));
    assert!(formatted.contains("/// The image's width in pixels, before any resampling."));
}
