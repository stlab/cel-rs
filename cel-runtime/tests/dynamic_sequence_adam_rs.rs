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
