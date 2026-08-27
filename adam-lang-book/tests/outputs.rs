//! Examples backing `book-src/outputs.md` (Chapter 7). See `tests/tutorial.rs` for how these
//! are wired into the book.

#[test]
fn basic_output() {
    // ANCHOR: basic_output
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet area_demo {
                cell width: i32 = 10;
                cell height: i32 = 20;

                out area := width * height;
            }
            "#,
        )
        .unwrap();
    parsed.propagate().unwrap();
    let area = parsed.cell_names["area"].0;
    assert_eq!(*parsed.read::<i32>(area).unwrap(), 200);
    // ANCHOR_END: basic_output
}

#[test]
fn output_cell_is_terminal() {
    // ANCHOR: output_cell_is_terminal
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str("sheet s { cell width: i32 = 10; out area := width * 2; }")
        .unwrap();
    let output = parsed.output_names["area"];
    let area_cell = parsed.output_cell(output).unwrap();
    let err = parsed.write(area_cell, 999_i32).unwrap_err();
    assert!(matches!(err, adam_rs::Error::TerminalCell));
    // ANCHOR_END: output_cell_is_terminal
}

#[test]
fn requirement_diagnostic() {
    // ANCHOR: requirement_diagnostic
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
    assert!(parsed.output_valid(output));

    let width = parsed.cell_names["width"].0;
    parsed.write(width, 50_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(!parsed.output_valid(output));
    // ANCHOR_END: requirement_diagnostic
}

#[test]
fn multiple_requirements() {
    // ANCHOR: multiple_requirements
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet bounds_demo {
                cell x: i32 = 50;

                out clamped: i32 := x require {
                    not_negative: clamped >= 0;
                    not_too_big: clamped <= 100;
                };
            }
            "#,
        )
        .unwrap();
    let output = parsed.output_names["clamped"];
    let x = parsed.cell_names["x"].0;

    parsed.write(x, -10_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(parsed.violated_requirements(output).count(), 1);
    assert!(!parsed.output_valid(output));
    // ANCHOR_END: multiple_requirements
}
