//! Examples backing `book-src/relationships-continued.md` (Chapter 8). See `src/lib.rs` for how
//! these `.adm2` files are wired into the book.

#[test]
fn destructuring_binding() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/relationships-continued/destructuring_binding.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();
    let (area, perimeter) = (
        parsed.cell_names["area"].0,
        parsed.cell_names["perimeter"].0,
    );
    assert_eq!(*parsed.read::<f64>(area).unwrap(), 40.0); // 10.0 * 4.0
    assert_eq!(*parsed.read::<f64>(perimeter).unwrap(), 28.0); // 2.0 * (10.0 + 4.0)
}

#[test]
fn self_referencing_method() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/relationships-continued/self_referencing_method.adm2"
        ))
        .unwrap();
    let level = parsed.cell_names["level"].0;

    parsed.write(level, 5_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 0);

    parsed.write(level, 0_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 0); // already conformed: idempotent

    parsed.write(level, -3_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), -3); // already <= 0: unchanged
}
