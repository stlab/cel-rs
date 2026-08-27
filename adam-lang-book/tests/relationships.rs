//! Examples backing `book-src/relationships.md` (Chapter 4). See `tests/tutorial.rs` for how
//! these are wired into the book.

#[test]
fn shared_cell_example() {
    // ANCHOR: shared_cell_example
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet diamond {
                cell a = 0.0;
                cell b = 0.0;
                cell c = 2.0;
                cell d = 3.0;

                relationship {
                    c := a * b;
                    b := c / a;
                    a := c / b;
                }

                relationship {
                    d := b * c;
                    c := d / b;
                    b := d / c;
                }
            }
            "#,
        )
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
    // ANCHOR_END: shared_cell_example
}

#[test]
fn conflict_error() {
    // ANCHOR: conflict_error
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(
            r#"
            sheet conflict {
                cell x = 1.0;

                relationship { x := 1.0; }
                relationship { x := 2.0; }
            }
            "#,
        )
        .unwrap()
        .propagate()
        .unwrap_err();
    assert!(matches!(err, adam_rs::Error::Conflict));
    // ANCHOR_END: conflict_error
}

#[test]
fn cycle_error() {
    // ANCHOR: cycle_error
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(
            r#"
            sheet cycle {
                cell x = 1.0;
                cell y = 1.0;
                cell z = 1.0;

                relationship { y := x; }
                relationship { z := y; }
                relationship { x := z; }
            }
            "#,
        )
        .unwrap()
        .propagate()
        .unwrap_err();
    assert!(matches!(err, adam_rs::Error::Cycle));
    // ANCHOR_END: cycle_error
}

#[test]
fn destructuring_binding() {
    // ANCHOR: destructuring_binding
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet swap_demo {
                cell a: i32 = 1;
                cell b: i32 = 2;

                relationship {
                    (a, b) := (b, a);
                }
            }
            "#,
        )
        .unwrap();
    parsed.propagate().unwrap();
    let (a, b) = (parsed.cell_names["a"].0, parsed.cell_names["b"].0);
    assert_eq!(*parsed.read::<i32>(a).unwrap(), 2);
    assert_eq!(*parsed.read::<i32>(b).unwrap(), 1);
    // ANCHOR_END: destructuring_binding
}
