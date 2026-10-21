//! Executable compound-cell contracts included by the cells chapter.

// ANCHOR: compound-cells
/// Reads, writes, and propagates canonical arrays while retaining source aspirations.
#[test]
fn compound_cell_interoperability() {
    use adam_lang::{AdamParser, TypeRegistry};
    use cel_parser::OpLookup;
    use cel_runtime::{Array, ArrayOf, Cons, IntoCStackList, IntoRustTuple, Leaf, Nil};

    type Row = Cons<Leaf<i32>, Cons<ArrayOf<Leaf<f64>>, Nil>>;
    type Rows = ArrayOf<Row>;
    let mut sheet = AdamParser::new(TypeRegistry::new(), OpLookup::new())
        .parse_str(
            "sheet data {
                source rows: [(i32, [f64])];
                cell defaults: (i32, [f64]);
                cell unit: ();
                out copy := rows;
            }",
        )
        .unwrap();
    let rows = sheet.cell_names["rows"].0;
    let defaults = sheet
        .read_compound::<Row>(sheet.cell_names["defaults"].0)
        .unwrap()
        .into_rust_tuple();
    assert_eq!(defaults.0, 0);
    assert!(defaults.1.is_empty());
    assert!(sheet.read_compound::<Rows>(rows).unwrap().is_empty());
    sheet
        .read_compound::<Nil>(sheet.cell_names["unit"].0)
        .unwrap();
    sheet
        .write_compound::<Rows>(
            rows,
            Array::from_vec(vec![(7, Array::from_vec(vec![1.5])).into_c_stack_list()]),
        )
        .unwrap();
    sheet.propagate().unwrap();
    let copied = sheet
        .read_compound::<Rows>(sheet.output_names["copy"])
        .unwrap()
        .into_vec()
        .pop()
        .unwrap()
        .into_rust_tuple();
    assert_eq!(copied.0, 7);
    assert_eq!(copied.1.as_slice(), [1.5]);
    assert_eq!(
        sheet.source_compound::<Rows>(rows).unwrap().as_slice()[0].1,
        7
    );
    assert!(
        sheet
            .write_compound::<ArrayOf<Leaf<u32>>>(rows, Array::from_vec(vec![99]))
            .is_err()
    );
    assert_eq!(sheet.read_compound::<Rows>(rows).unwrap().len(), 1);
    assert_eq!(
        sheet.source_compound::<Rows>(rows).unwrap().as_slice()[0].1,
        7
    );
}
// ANCHOR_END: compound-cells

// ANCHOR: callable-cell-capability
/// Rejects persistent callable cells without inventing function equality.
#[test]
fn callable_cells_require_equality() {
    let error =
        adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(), cel_parser::OpLookup::new())
            .parse_str("sheet data { cell functions = []: [Fn((i32,), [i32])]; }")
            .err()
            .unwrap();
    assert!(
        error
            .to_string()
            .contains("equality capability unavailable")
    );
}
// ANCHOR_END: callable-cell-capability
