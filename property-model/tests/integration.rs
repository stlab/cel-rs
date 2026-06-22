//! End-to-end integration tests for the property-model crate.

use property_model::{Error, Method, Sheet};

#[test]
fn single_method_executes_correctly() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 3))])
        .unwrap();

    sheet.write(a, 7_i32).unwrap();
    sheet.propagate().unwrap();

    assert_eq!(*sheet.read::<i32>(b).unwrap(), 21);
}

#[test]
fn chained_relationships_execute_in_order() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x + 1))])
        .unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(b, c, |x: &i32| Ok(*x + 1))])
        .unwrap();

    sheet.write(a, 10_i32).unwrap();
    sheet.propagate().unwrap();

    let changed: Vec<_> = sheet.changed().collect();
    assert_eq!(changed.len(), 2);
    assert!(changed.contains(&b));
    assert!(changed.contains(&c));

    // Verify methods executed in topological order: a → b → c
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 11);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 12);
}

#[test]
fn changed_cells_tracked() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();

    sheet.write(a, 1_i32).unwrap();
    sheet.propagate().unwrap();

    let changed: Vec<_> = sheet.changed().collect();
    assert_eq!(changed.len(), 1);
    assert!(changed.contains(&b));
}

#[test]
fn clear_changed_empties_the_changed_set() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();

    sheet.write(a, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(sheet.changed().count(), 1);

    sheet.clear_changed();
    assert_eq!(sheet.changed().count(), 0);
}

#[test]
fn propagate_clears_previous_changed_set() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();

    sheet.write(a, 1_i32).unwrap();
    sheet.propagate().unwrap();
    // b changed in first propagation

    sheet.write(a, 2_i32).unwrap();
    sheet.propagate().unwrap();
    // b changed again; changed set should have only cells from this propagation
    let changed: Vec<_> = sheet.changed().collect();
    assert_eq!(changed.len(), 1);
    assert!(changed.contains(&b));
}

#[test]
fn failed_propagation_preserves_previous_changed_set() {
    // A successful propagation records `b` as changed. A subsequent propagation
    // that fails during planning (Conflict) must leave the previous change set
    // intact so `changed()` still reflects the last successful run.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();

    sheet.write(a, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(sheet.changed().any(|id| id == b));

    // Introduce a conflict: two single-method relationships both write `out`,
    // so planning cannot satisfy both and returns `Error::Conflict`.
    let p = sheet.add_cell(0_i32);
    let q = sheet.add_cell(0_i32);
    let out = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(p, out, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(q, out, |x: &i32| Ok(*x))])
        .unwrap();

    assert!(matches!(sheet.propagate(), Err(Error::Conflict)));

    // The change set from the last successful propagation is still observable.
    let changed: Vec<_> = sheet.changed().collect();
    assert_eq!(changed.len(), 1);
    assert!(changed.contains(&b));
}

#[test]
fn strength_drives_method_selection() {
    // a * b = c — three methods, one per direction.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0.0_f64);
    let b = sheet.add_cell(0.0_f64);
    let c = sheet.add_cell(0.0_f64);

    let methods = vec![
        Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok((*x) * (*y))),
        Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok((*y) / (*x))),
        Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok((*y) / (*x))),
    ];
    sheet.add_relationship(methods).unwrap();

    // Write a=2 (strength=1), b=3 (strength=2). c.strength=0 is weakest → derive c.
    sheet.write(a, 2.0_f64).unwrap();
    sheet.write(b, 3.0_f64).unwrap();
    sheet.propagate().unwrap();
    assert!((sheet.read::<f64>(c).unwrap() - 6.0).abs() < 1e-10);

    // Write c=12 (strength=3). a.strength=1 is now weakest → derive a.
    sheet.write(c, 12.0_f64).unwrap();
    sheet.propagate().unwrap();
    assert!((sheet.read::<f64>(a).unwrap() - 4.0).abs() < 1e-10);
}

#[test]
fn method_returning_error_propagates_as_method_failed() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0.0_f64);
    let b = sheet.add_cell(0.0_f64);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(
            a,
            b,
            |_: &f64| -> Result<f64, _> { Err(anyhow::anyhow!("intentional error")) },
        )])
        .unwrap();

    let result = sheet.propagate();
    assert!(matches!(result, Err(Error::MethodFailed(_))));
}

#[test]
fn cycle_in_selected_methods_returns_cycle_error() {
    // a→b and b→a: each relationship has only one method, forming a cycle.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    // Relationship 1: inputs=[a], outputs=[b] — but a is input, so the only method writes b.
    // Relationship 2: inputs=[b], outputs=[a] — writes a.
    // Planner selects both (no conflict on outputs), but they form a cycle.
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();

    assert!(matches!(sheet.propagate(), Err(Error::Cycle)));
}
