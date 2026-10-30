//! Examples backing `book-src/filters.md` (Chapter 5). See `src/lib.rs` for how these `.adm2`
//! files are wired into the book.

/// Retains the raw write until propagation applies the filter.
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

/// Restores a filtered cell's original source aspiration when its bounds expand.
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

/// Exposes inclusive range-filter metadata and its bounds.
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

/// Reports a derived cell's filter violation without clamping its value.
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

/// Diagnoses an output filter after an edit moves the derived value out of range.
#[test]
fn filter_on_an_out_cell() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/filters/filter_on_an_out_cell.adm2"
        ))
        .unwrap();
    let area = parsed.output_names["area"];

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(area).unwrap(), 4); // within range, unclamped
    let width = parsed.cell_names["width"].0;
    parsed.write(width, 500i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(area).unwrap(), 500);
    assert!(parsed.filter_violated_cells().any(|id| id == area));
}
