//! Examples backing `book-src/filters.md` (Chapter 6). See `tests/tutorial.rs` for how these
//! are wired into the book.

#[test]
fn write_never_filters() {
    // ANCHOR: write_never_filters
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str("sheet s { cell level: i32 = 50 filter 0..=100; }")
        .unwrap();
    let level = parsed.cell_names["level"].0;

    parsed.write(level, 500_i32).unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 500); // the raw value, unfiltered

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 100); // now conformed
    // ANCHOR_END: write_never_filters
}

#[test]
fn raw_value_never_lost() {
    // ANCHOR: raw_value_never_lost
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet spring_back {
                cell max: i32 = 100 filter 0..=200;
                cell level: i32 = 50 filter 0..=max;
            }
            "#,
        )
        .unwrap();
    let (max, level) = (parsed.cell_names["max"].0, parsed.cell_names["level"].0);

    parsed.write(max, 10_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 10); // clamped down

    parsed.write(max, 100_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 50); // back to the original 50, not 10
    // ANCHOR_END: raw_value_never_lost
}

#[test]
fn range_filter_kind() {
    // ANCHOR: range_filter_kind
    let mut parser = adam_lang_book::support::parser();
    let parsed = parser
        .parse_str("sheet s { cell level: i32 = 50 filter 0..=100; }")
        .unwrap();
    let level = parsed.cell_names["level"].0;
    assert!(matches!(
        parsed.filter_kind(level),
        Some(adam_rs::FilterKind::Range { .. })
    ));
    assert_eq!(parsed.filter_range::<i32>(level), Some((0, 100)));
    // ANCHOR_END: range_filter_kind
}

#[test]
fn derived_cell_diagnosed_not_corrected() {
    // ANCHOR: derived_cell_diagnosed_not_corrected
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet diagnose_only {
                cell bound: i32 = 100 filter 0..=100;
                cell driver: i32 = 500;

                relationship {
                    bound := driver;
                }
            }
            "#,
        )
        .unwrap();
    parsed.propagate().unwrap();

    let bound = parsed.cell_names["bound"].0;
    assert_eq!(*parsed.read::<i32>(bound).unwrap(), 500); // not clamped
    assert!(parsed.filter_violated_cells().any(|id| id == bound));
    // ANCHOR_END: derived_cell_diagnosed_not_corrected
}

#[test]
fn must_reference_underscore() {
    // ANCHOR: must_reference_underscore
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str("sheet s { cell x: i32 = 0 filter 5; }") // never mentions `_`
        .err()
        .unwrap();
    assert!(format!("{err}").contains("must reference `_`"));
    // ANCHOR_END: must_reference_underscore
}

#[test]
fn tuple_filter_not_supported() {
    // ANCHOR: tuple_filter_not_supported
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str("sheet s { cell x: (i32, i32) = (0, 0) filter _; }") // tuple-typed cell
        .err()
        .unwrap();
    assert!(format!("{err}").contains("tuple"));
    // ANCHOR_END: tuple_filter_not_supported
}
