//! Examples backing `book-src/expressions.md` (Chapter 4). See `src/lib.rs` for how these
//! `.adm2` files are wired into the book. `no_standard_library` is kept as a regression test
//! only — it needs a parser built without `cel-std`, so it is never `{{#include}}`d into the
//! chapter itself; see `NO_LIVE_MOUNT` in `adam-lang-book-live-config`.

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
