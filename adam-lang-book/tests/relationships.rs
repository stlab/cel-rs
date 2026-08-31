//! Examples backing `book-src/relationships.md` (Chapter 4). See `src/lib.rs` for how these
//! `.adm2` files are wired into the book.

#[test]
fn shared_cell_example() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/relationships/shared_cell_example.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();

    let (a, b, c, d) = (
        parsed.cell_names["a"].0,
        parsed.cell_names["b"].0,
        parsed.cell_names["c"].0,
        parsed.cell_names["d"].0,
    );
    assert!(parsed.is_source(c) && parsed.is_source(d));
    assert!(!parsed.is_source(a) && !parsed.is_source(b));
    assert_eq!(*parsed.read::<f64>(c).unwrap(), 2.0); // untouched
    assert_eq!(*parsed.read::<f64>(d).unwrap(), 3.0); // untouched
    assert_eq!(*parsed.read::<f64>(b).unwrap(), 1.5); // d / c
    assert_eq!(*parsed.read::<f64>(a).unwrap(), 4.0 / 3.0); // c / b
}

#[test]
fn conflict_error() {
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/relationships/conflict_error.adm2"
        ))
        .unwrap()
        .propagate()
        .unwrap_err();
    assert!(matches!(err, adam_rs::Error::Conflict));
}

#[test]
fn cycle_error() {
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/relationships/cycle_error.adm2"
        ))
        .unwrap()
        .propagate()
        .unwrap_err();
    assert!(matches!(err, adam_rs::Error::Cycle));
}

#[test]
fn destructuring_binding() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/relationships/destructuring_binding.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();
    let (a, b) = (parsed.cell_names["a"].0, parsed.cell_names["b"].0);
    assert_eq!(*parsed.read::<i32>(a).unwrap(), 2);
    assert_eq!(*parsed.read::<i32>(b).unwrap(), 1);
}
