//! Examples backing `book-src/conditionals.md` (Chapter 5). See `tests/tutorial.rs` for how
//! these are wired into the book.

#[test]
fn multi_cell_match_subject() {
    // ANCHOR: multi_cell_match_subject
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet resample_demo {
                cell resample: bool = true;
                cell constrain: bool = true;
                cell locked: bool = false;

                conditional resample && constrain {
                    true => {
                        relationship { locked := true; }
                    }
                    false => {
                        relationship { locked := false; }
                    }
                }
            }
            "#,
        )
        .unwrap();
    parsed.propagate().unwrap();
    let locked = parsed.cell_names["locked"].0;
    assert!(*parsed.read::<bool>(locked).unwrap());

    let resample = parsed.cell_names["resample"].0;
    parsed.write(resample, false).unwrap();
    parsed.propagate().unwrap();
    assert!(!*parsed.read::<bool>(locked).unwrap());
    // ANCHOR_END: multi_cell_match_subject
}

#[test]
fn default_branch_and_spring_back() {
    // ANCHOR: default_branch_and_spring_back
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet no_default {
                cell mode: i32 = 0;
                cell x: f64 = 1.0;

                conditional mode {
                    0i32 => {
                        relationship { x := 100.0; }
                    }
                }
            }
            "#,
        )
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
    // ANCHOR_END: default_branch_and_spring_back
}
