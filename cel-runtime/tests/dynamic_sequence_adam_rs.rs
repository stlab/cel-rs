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

    let f = DynamicSequence::adapt_fn_1(|_: &(i32, i32)| Ok(99i32));
    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(match_cell, output, f)])
        .unwrap();

    sheet
        .add_conditional::<DynamicSequence>(
            match_cell,
            vec![(vec![DynamicSequence::from_tuple((1i32, 2i32))], vec![rel])],
            vec![],
        )
        .unwrap();

    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(output).unwrap(), 99);
}
