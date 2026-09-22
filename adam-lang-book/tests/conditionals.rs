//! Examples backing `book-src/conditionals.md` (Chapter 9). See `src/lib.rs` for how these
//! `.adm2` files are wired into the book.

#[test]
fn multi_cell_match_subject() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/conditionals/multi_cell_match_subject.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();
    let locked = parsed.cell_names["locked"].0;
    assert!(*parsed.read::<bool>(locked).unwrap());

    let resample = parsed.cell_names["resample"].0;
    parsed.write(resample, false).unwrap();
    parsed.propagate().unwrap();
    assert!(!*parsed.read::<bool>(locked).unwrap());
}

#[test]
fn default_branch_and_spring_back() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/conditionals/default_branch_and_spring_back.adm2"
        ))
        .unwrap();
    let mode = parsed.cell_names["mode"].0;
    let x = parsed.cell_names["x"].0;

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 100.0); // mode == 0: branch active
    assert!(!parsed.is_source(x));

    parsed.write(mode, 7_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 1.0); // no branch matches; x reverts to its
    // own declared default, not 100.0
    assert!(parsed.is_source(x));
}

#[test]
fn forced_and_self_ref_shadow() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/conditionals/forced_and_self_ref_shadow.adm2"
        ))
        .unwrap();
    let (mode, low, high) = (
        parsed.cell_names["mode"].0,
        parsed.cell_names["low"].0,
        parsed.cell_names["high"].0,
    );

    // `low`, declared first, is stalest, so the solver selects `low := min(low, high)`;
    // `high` stays an ordinary, unclaimed source throughout this branch.
    parsed.propagate().unwrap();
    assert!(!parsed.is_source(low));
    assert!(parsed.is_source(high));
    assert_eq!(*parsed.read::<i32>(low).unwrap(), 4);
    assert_eq!(*parsed.read::<i32>(high).unwrap(), 9);
    assert_eq!(*parsed.source::<i32>(low).unwrap(), 4);
    assert_eq!(*parsed.source::<i32>(high).unwrap(), 9);

    parsed.write(high, 42_i32).unwrap();
    parsed.write(mode, 1_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(parsed.is_forced(low));
    assert_eq!(*parsed.read::<i32>(low).unwrap(), 42);
    assert_eq!(*parsed.source::<i32>(low).unwrap(), 4);

    parsed.write(mode, 0_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(!parsed.is_source(low));
    assert!(parsed.is_source(high));
    assert_eq!(*parsed.read::<i32>(low).unwrap(), 4);
    assert_eq!(*parsed.read::<i32>(high).unwrap(), 42);

    // Writing `low` promotes it to freshest, flipping which binding the solver selects: now
    // `high := max(low, high)` fires instead, pulling `high` up to match `low`.
    parsed.write(low, 100_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(parsed.is_source(low));
    assert!(!parsed.is_source(high));
    assert_eq!(*parsed.read::<i32>(low).unwrap(), 100);
    assert_eq!(*parsed.read::<i32>(high).unwrap(), 100);
}
