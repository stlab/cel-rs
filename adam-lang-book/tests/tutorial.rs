//! Examples backing `book-src/tutorial.md` (Chapter 1). See `src/lib.rs` for how these `.adm2`
//! files are wired into the book.

#[test]
fn first_sheet() {
    let mut parser = adam_lang_book::support::parser();
    let parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/tutorial/first_sheet.adm2"
        ))
        .unwrap();

    let width = parsed.cell_names["width"].0;
    assert_eq!(*parsed.read::<i32>(width).unwrap(), 1920);
}

#[test]
fn clamp_demo() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/tutorial/clamp_demo.adm2"
        ))
        .unwrap();
    let level = parsed.cell_names["level"].0;

    parsed.write(level, 500_i32).unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 500); // still raw

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 100); // now clamped
}

#[test]
fn area_with_requirement() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/tutorial/area_with_requirement.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();

    let output = parsed.output_names["area"];
    assert!(parsed.cell_requirements_valid(output)); // 10 * 20 == 200 <= 300

    let width = parsed.cell_names["width"].0;
    parsed.write(width, 50_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(!parsed.cell_requirements_valid(output)); // 50 * 20 == 1000 > 300
}

#[test]
fn multiplication_triangle() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/tutorial/multiplication_triangle.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();

    let (a, b, c) = (
        parsed.cell_names["a"].0,
        parsed.cell_names["b"].0,
        parsed.cell_names["c"].0,
    );
    assert_eq!(*parsed.read::<f64>(c).unwrap(), 6.0); // 2.0 * 3.0, derived

    parsed.write(b, 5.0).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(a).unwrap(), 2.0); // untouched
    assert_eq!(*parsed.read::<f64>(b).unwrap(), 5.0); // just written
    assert_eq!(*parsed.read::<f64>(c).unwrap(), 10.0); // 2.0 * 5.0, re-derived
}

#[test]
fn destructuring_demo() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/tutorial/destructuring_demo.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();

    let (area, perimeter) = (
        parsed.cell_names["area"].0,
        parsed.cell_names["perimeter"].0,
    );
    assert_eq!(*parsed.read::<f64>(area).unwrap(), 40.0); // 10.0 * 4.0
    assert_eq!(*parsed.read::<f64>(perimeter).unwrap(), 28.0); // 2.0 * (10.0 + 4.0)

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(area).unwrap(), 40.0);
    assert_eq!(*parsed.read::<f64>(perimeter).unwrap(), 28.0);
}

#[test]
fn mode_demo() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!("../book-src/examples/tutorial/mode_demo.adm2"))
        .unwrap();

    let p = parsed.cell_names["p"].0;
    let x = parsed.cell_names["x"].0;

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 2.0); // p == 0: x := y

    parsed.write(p, 2_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 0.0); // p matches no named branch: default
}

#[test]
fn forced_and_self_ref_shadow() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/tutorial/forced_and_self_ref_shadow.adm2"
        ))
        .unwrap();
    let (mode, low, high) = (
        parsed.cell_names["mode"].0,
        parsed.cell_names["low"].0,
        parsed.cell_names["high"].0,
    );

    // mode == 0 (declared default): self-referencing branch. 4 <= 9 already: unchanged.
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(low).unwrap(), 4);
    assert_eq!(*parsed.read::<i32>(high).unwrap(), 9);
    assert_eq!(*parsed.source::<i32>(low).unwrap(), 4);
    assert_eq!(*parsed.source::<i32>(high).unwrap(), 9);

    // mode == 1: `low` is forced from `high` (single-method relationship).
    parsed.write(high, 42_i32).unwrap();
    parsed.write(mode, 1_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(parsed.is_forced(low));
    assert_eq!(*parsed.read::<i32>(low).unwrap(), 42);
    assert_eq!(*parsed.source::<i32>(low).unwrap(), 4); // low's own source, untouched

    // Back to mode == 0: both cells recomputed fresh from their own sources (4, 42),
    // not from the stale forced 42.
    parsed.write(mode, 0_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(low).unwrap(), 4);
    assert_eq!(*parsed.read::<i32>(high).unwrap(), 42);
}
