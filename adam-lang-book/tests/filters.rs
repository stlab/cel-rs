//! Examples backing `book-src/filters.md` (Chapter 6). See `src/lib.rs` for how these `.adm2`
//! files are wired into the book.

#[test]
fn write_never_filters() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/filters/write_never_filters.adm2"
        ))
        .unwrap();
    let level = parsed.cell_names["level"].0;

    parsed.write(level, 500_i32).unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 500); // the raw value, unfiltered

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 100); // now conformed
}

#[test]
fn raw_value_never_lost() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/filters/raw_value_never_lost.adm2"
        ))
        .unwrap();
    let (max, level) = (parsed.cell_names["max"].0, parsed.cell_names["level"].0);

    parsed.write(max, 10_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 10); // clamped down

    parsed.write(max, 100_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 50); // back to the original 50, not 10
}

#[test]
fn range_filter_kind() {
    let mut parser = adam_lang_book::support::parser();
    let parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/filters/range_filter_kind.adm2"
        ))
        .unwrap();
    let level = parsed.cell_names["level"].0;
    assert!(matches!(
        parsed.filter_kind(level),
        Some(adam_rs::FilterKind::Range { .. })
    ));
    assert_eq!(parsed.filter_range::<i32>(level), Some((0, 100)));
}

#[test]
fn derived_cell_diagnosed_not_corrected() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/filters/derived_cell_diagnosed_not_corrected.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();

    let bound = parsed.cell_names["bound"].0;
    assert_eq!(*parsed.read::<i32>(bound).unwrap(), 500); // not clamped
    assert!(parsed.filter_violated_cells().any(|id| id == bound));
}

#[test]
fn must_reference_underscore() {
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/filters/must_reference_underscore.adm2"
        )) // never mentions `_`
        .err()
        .unwrap();
    assert!(format!("{err}").contains("must reference `_`"));
}

#[test]
fn tuple_filter_not_supported() {
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/filters/tuple_filter_not_supported.adm2"
        )) // tuple-typed cell
        .err()
        .unwrap();
    assert!(format!("{err}").contains("tuple"));
}
