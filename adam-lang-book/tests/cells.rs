//! Examples backing `book-src/cells.md` (Chapter 2). See `src/lib.rs` for how these `.adm2`
//! files are wired into the book.

#[test]
fn tuple_typed_cell() {
    let mut parser = adam_lang_book::support::parser();
    let parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/cells/tuple_typed_cell.adm2"
        ))
        .unwrap();
    let point = parsed.cell_names["point"].0;
    type Point = cel_runtime::Cons<
        cel_runtime::Leaf<f64>,
        cel_runtime::Cons<cel_runtime::Leaf<f64>, cel_runtime::Nil>,
    >;
    let value = parsed.read_compound::<Point>(point).unwrap();
    assert_eq!((value.1, value.0.1), (0.0, 0.0));
}
