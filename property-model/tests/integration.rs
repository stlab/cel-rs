//! End-to-end integration tests for the property-model crate.

use std::any::TypeId;

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
    // b added first (lower strength) so it is the output; a added second
    // (higher strength) so it is the source. The planner selects [a]→b,
    // which runs the method and surfaces the error.
    let b = sheet.add_cell(0.0_f64);
    let a = sheet.add_cell(0.0_f64);
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
fn mutually_dependent_relationships_return_conflict() {
    // a→b and b→a: Adam marks a as a source, flows to b via the first
    // relationship, then the second relationship's only method (b→a) cannot
    // fire because a is already determined. The second relationship is left
    // unassigned, which is reported as a Conflict.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();

    assert!(matches!(sheet.propagate(), Err(Error::Conflict)));
}

#[test]
fn arity_3_2_1() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell("a".to_string());
    let c = sheet.add_cell("ab".to_string());
    let b = sheet.add_cell("b".to_string());
    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, b], c, |x: &String, y: &String| Ok(x.clone() + y)),
            Method::new(
                vec![c],
                vec![a, b],
                vec![TypeId::of::<String>()],
                vec![TypeId::of::<String>(), TypeId::of::<String>()],
                |args| {
                    let z = args[0]
                        .downcast_ref::<String>()
                        .expect("type checked at add_relationship");
                    let mut chars = z.chars();
                    let first = chars.next().unwrap_or_default().to_string();
                    let rest = chars.collect::<String>();
                    Ok(vec![Box::new(first), Box::new(rest)])
                },
            ),
        ])
        .unwrap();

    sheet.propagate().unwrap();

    assert_eq!(sheet.read::<String>(a).unwrap(), "a");
    assert_eq!(sheet.read::<String>(b).unwrap(), "b");
    assert_eq!(sheet.read::<String>(c).unwrap(), "ab");
}

#[test]
fn self_ref_direct_clamp() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, a, |x: &i32| Ok((*x).min(0)))])
        .unwrap();

    // Value above 0: clamped to 0.
    sheet.write(a, 5_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 0);

    // Value at 0: unchanged.
    sheet.write(a, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 0);

    // Value below 0: idempotent, unchanged.
    sheet.write(a, -3_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), -3);
}

#[test]
fn self_ref_le_chain() {
    // a <= b <= c enforced by two self-referencing constraints.
    //
    // R1 — a <= b:
    //   M0: a = min(a, b)  fires when b is the stronger source
    //   M1: b = max(a, b)  fires when a is the stronger source
    //
    // R2 — b <= c:
    //   M2: b = min(b, c)  fires when c is the stronger source
    //   M3: c = max(b, c)  fires when b is the stronger source
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);

    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| Ok((*x).min(*y))),
            Method::from_fn_2_1([a, b], b, |x: &i32, y: &i32| Ok((*x).max(*y))),
        ])
        .unwrap();

    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([b, c], b, |x: &i32, y: &i32| Ok((*x).min(*y))),
            Method::from_fn_2_1([b, c], c, |x: &i32, y: &i32| Ok((*x).max(*y))),
        ])
        .unwrap();

    // Case 1: already satisfied — no adjustment.
    // Write order c, b, a → a is strongest.
    sheet.write(c, 5_i32).unwrap();
    sheet.write(b, 3_i32).unwrap();
    sheet.write(a, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 1);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 3);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 5);

    // Case 2: a > b and a > c, a is strongest → b and c raised to a.
    sheet.write(c, 1_i32).unwrap();
    sheet.write(b, 3_i32).unwrap();
    sheet.write(a, 5_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 5);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 5);

    // Case 3: b > c, c is strongest → b lowered to c; a already <= b.
    sheet.write(a, 1_i32).unwrap();
    sheet.write(b, 5_i32).unwrap();
    sheet.write(c, 3_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 1);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 3);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 3);

    // Case 4: b is strongest, a above and c below → a clamped to b, c raised to b.
    sheet.write(c, 1_i32).unwrap();
    sheet.write(a, 5_i32).unwrap();
    sheet.write(b, 3_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 3);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 3);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 3);
}
