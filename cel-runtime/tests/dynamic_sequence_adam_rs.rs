//! Acceptance tests demonstrating DynamicSequence integration with adam-rs Sheet and Method.
use adam_rs::{Method, Sheet};
use cel_runtime::DynamicSequence;

#[test]
fn dynamic_sequence_cell_works_with_unmodified_method_from_fn_1_1() {
    let mut sheet = Sheet::new();
    let input = sheet.add_cell(DynamicSequence::from_tuple((3i32, 4.5f64)));
    let output = sheet.add_cell(0.0f64);

    let f = DynamicSequence::adapt_fn_1(|t: &(i32, f64)| Ok(t.0 as f64 + t.1));
    sheet
        .add_relationship(vec![Method::from_fn_1_1(input, output, f)])
        .unwrap();

    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<f64>(output).unwrap(), 7.5);
}

#[test]
fn dynamic_sequence_cell_selects_conditional_branch_via_partial_eq() {
    let mut sheet = Sheet::new();
    let match_cell = sheet.add_cell(DynamicSequence::from_tuple((1i32, 2i32)));
    let output = sheet.add_cell(0i32);
    let other_output = sheet.add_cell(0i32);

    let f = DynamicSequence::adapt_fn_1(|_: &(i32, i32)| Ok(99i32));
    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(match_cell, output, f)])
        .unwrap();

    // A second branch, keyed on a value that does not equal the match cell's actual value
    // ((1, 2)), so its relationship must not fire. Without this branch, a `PartialEq` impl
    // that always returned `true` would make this test pass just as well as the real one.
    let g = DynamicSequence::adapt_fn_1(|_: &(i32, i32)| Ok(7i32));
    let other_rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(match_cell, other_output, g)])
        .unwrap();

    sheet
        .add_conditional::<DynamicSequence>(
            match_cell,
            vec![
                (vec![DynamicSequence::from_tuple((1i32, 2i32))], vec![rel]),
                (
                    vec![DynamicSequence::from_tuple((9i32, 9i32))],
                    vec![other_rel],
                ),
            ],
            vec![],
        )
        .unwrap();

    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(output).unwrap(), 99);
    assert_eq!(
        *sheet.read::<i32>(other_output).unwrap(),
        0,
        "the non-matching branch must not fire"
    );
}
