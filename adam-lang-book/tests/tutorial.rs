//! Examples backing `book-src/tutorial.md` (Chapter 1). Each function is pulled into the book
//! verbatim via mdBook's `{{#include tests/tutorial.rs:name}}`; see `src/lib.rs`.

#[test]
fn first_sheet() {
    // ANCHOR: first_sheet
    let mut parser = adam_lang_book::support::parser();
    let parsed = parser
        .parse_str(
            r#"
            sheet hello {
                cell width: i32 = 1920;
                cell height: i32 = 1080;
            }
            "#,
        )
        .unwrap();

    let width = parsed.cell_names["width"].0;
    assert_eq!(*parsed.read::<i32>(width).unwrap(), 1920);
    // ANCHOR_END: first_sheet
}

#[test]
fn multiplication_triangle() {
    // ANCHOR: multiplication_triangle
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet triangle {
                cell c = 0.0;
                cell a = 2.0;
                cell b = 3.0;

                relationship {
                    c := a * b;
                    a := c / b;
                    b := c / a;
                }
            }
            "#,
        )
        .unwrap();
    parsed.propagate().unwrap();

    let (a, b, c) = (
        parsed.cell_names["a"].0,
        parsed.cell_names["b"].0,
        parsed.cell_names["c"].0,
    );
    assert_eq!(*parsed.read::<f64>(c).unwrap(), 6.0); // 2.0 * 3.0, derived

    // Write b: it becomes the freshest cell, so the solver keeps both a and b as
    // sources and re-derives c from them.
    parsed.write(b, 5.0).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(a).unwrap(), 2.0); // untouched
    assert_eq!(*parsed.read::<f64>(b).unwrap(), 5.0); // just written
    assert_eq!(*parsed.read::<f64>(c).unwrap(), 10.0); // 2.0 * 5.0, re-derived
    // ANCHOR_END: multiplication_triangle
}

#[test]
fn mode_demo() {
    // ANCHOR: mode_demo
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet mode_demo {
                cell p: i32 = 0;
                cell x: f64 = 1.0;
                cell y: f64 = 2.0;

                conditional p {
                    0i32 => {
                        relationship {
                            x := y;
                        }
                    }
                    1i32 => {
                        relationship {
                            y := x;
                        }
                    }
                    _ => {
                        relationship {
                            x := 0.0;
                        }
                    }
                }
            }
            "#,
        )
        .unwrap();

    let p = parsed.cell_names["p"].0;
    let x = parsed.cell_names["x"].0;

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 2.0); // p == 0: x := y

    parsed.write(p, 2_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 0.0); // p matches no named branch: default
    // ANCHOR_END: mode_demo
}

#[test]
fn clamp_demo() {
    // ANCHOR: clamp_demo
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str("sheet volume { cell level: i32 = 50 filter 0..=100; }")
        .unwrap();
    let level = parsed.cell_names["level"].0;

    parsed.write(level, 500_i32).unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 500); // still raw

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 100); // now clamped
    // ANCHOR_END: clamp_demo
}

#[test]
fn area_with_requirement() {
    // ANCHOR: area_with_requirement
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet area_demo {
                cell width: i32 = 10;
                cell height: i32 = 20;

                out area: i32 := width * height require {
                    not_too_big: area <= 300;
                };
            }
            "#,
        )
        .unwrap();
    parsed.propagate().unwrap();

    let output = parsed.output_names["area"];
    assert!(parsed.output_valid(output)); // 10 * 20 == 200 <= 300

    let width = parsed.cell_names["width"].0;
    parsed.write(width, 50_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(!parsed.output_valid(output)); // 50 * 20 == 1000 > 300
    // ANCHOR_END: area_with_requirement
}
