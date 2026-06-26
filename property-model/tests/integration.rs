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

#[test]
fn conditional_activates_matching_branch() {
    // mode=1 activates rel_on which doubles `a` into `b`.
    let mut sheet = Sheet::new();
    let mode = sheet.add_cell(0_i32);
    let a = sheet.add_cell(3_i32);
    let b = sheet.add_cell(0_i32);

    let rel_on = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
        .unwrap();

    sheet
        .add_conditional(mode, vec![(vec![1_i32], vec![rel_on])], vec![])
        .unwrap();

    sheet.write(mode, 1_i32).unwrap();
    sheet.write(a, 3_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 6);
}

#[test]
fn conditional_no_match_and_no_default_succeeds_silently() {
    // No branch matches, no default — propagate succeeds, b keeps its value.
    let mut sheet = Sheet::new();
    let mode = sheet.add_cell(0_i32);
    let a = sheet.add_cell(3_i32);
    let b = sheet.add_cell(99_i32);

    let rel_on = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
        .unwrap();

    sheet
        .add_conditional(mode, vec![(vec![1_i32], vec![rel_on])], vec![])
        .unwrap();

    // mode=0, no match, rel_on inactive.
    sheet.write(mode, 0_i32).unwrap();
    sheet.propagate().unwrap();
    // b unchanged: no method wrote to it.
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 99);
}

#[test]
fn conditional_default_branch_activates_when_no_key_matches() {
    let mut sheet = Sheet::new();
    let mode = sheet.add_cell(0_i32);
    let a = sheet.add_cell(3_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);

    let rel_double = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
        .unwrap();
    let rel_triple = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, c, |x: &i32| Ok(*x * 3))])
        .unwrap();

    sheet
        .add_conditional(
            mode,
            vec![(vec![1_i32], vec![rel_double])],
            vec![rel_triple], // default
        )
        .unwrap();

    // mode=1: double branch.
    sheet.write(mode, 1_i32).unwrap();
    sheet.write(a, 4_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 8);

    // mode=99: default branch.
    sheet.write(mode, 99_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 12);
}

#[test]
fn conditional_multi_key_branch_matches_any_key() {
    // Branch is active for mode=0 OR mode=2.
    let mut sheet = Sheet::new();
    let mode = sheet.add_cell(0_i32);
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(0_i32);

    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();

    sheet
        .add_conditional(mode, vec![(vec![0_i32, 2_i32], vec![rel])], vec![])
        .unwrap();

    sheet.write(a, 7_i32).unwrap();
    sheet.write(mode, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 7);

    sheet.write(mode, 2_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 7);

    // mode=1 does not match; b stays at its last derived value.
    sheet.write(mode, 1_i32).unwrap();
    sheet.propagate().unwrap();
    // b is no longer derived; it keeps the last value (7).
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 7);
}

#[test]
fn conditional_branch_switch_stability() {
    // When branch switches, previously derived cells should not block the new plan.
    // Setup: mode controls which of two independent relationships is active.
    // Branch 0: a→out (out = a * 2)
    // Branch 1: b→out (out = b * 3)
    let mut sheet = Sheet::new();
    let mode = sheet.add_cell(0_i32);
    let a = sheet.add_cell(4_i32);
    let b = sheet.add_cell(5_i32);
    let out = sheet.add_cell(0_i32);

    let rel_a = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, out, |x: &i32| Ok(*x * 2))])
        .unwrap();
    let rel_b = sheet
        .add_relationship(vec![Method::from_fn_1_1(b, out, |x: &i32| Ok(*x * 3))])
        .unwrap();

    sheet
        .add_conditional(
            mode,
            vec![(vec![0_i32], vec![rel_a]), (vec![1_i32], vec![rel_b])],
            vec![],
        )
        .unwrap();

    // mode=0: out derived from a.
    sheet.write(mode, 0_i32).unwrap();
    sheet.write(a, 4_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(out).unwrap(), 8);

    // mode=1: out derived from b. Must not conflict even though out has a stale derived strength.
    sheet.write(mode, 1_i32).unwrap();
    sheet.write(b, 5_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(out).unwrap(), 15);
}

#[test]
fn conditional_match_cell_derived_from_multi_method_unconditional_relationship() {
    // A multi-method unconditional relationship can produce the match cell.
    // Setup: `flag` is produced by a two-method relationship between `x` and `y`.
    //   M0: [x, y] → flag  (flag = x > y)
    //   M1: [flag, x] → y  (y = x - 1 if flag else x + 1)
    // x is written with the highest strength, so the planner picks M0.
    // When flag=true (x > y), rel_active fires and doubles a into b.
    let mut sheet = Sheet::new();
    let x = sheet.add_cell(0_i32);
    let y = sheet.add_cell(0_i32);
    let flag = sheet.add_cell(false);
    let a = sheet.add_cell(3_i32);
    let b = sheet.add_cell(0_i32);

    // Multi-method unconditional relationship: x, y ↔ flag.
    // M0: [x, y] → flag  (true iff x > y)
    // M1: [flag, x] → y
    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([x, y], flag, |x: &i32, y: &i32| Ok(*x > *y)),
            Method::new(
                vec![flag, x],
                vec![y],
                vec![TypeId::of::<bool>(), TypeId::of::<i32>()],
                vec![TypeId::of::<i32>()],
                |args| {
                    let f = args[0].downcast_ref::<bool>().unwrap();
                    let xv = args[1].downcast_ref::<i32>().unwrap();
                    Ok(vec![Box::new(if *f { *xv - 1 } else { *xv + 1 })])
                },
            ),
        ])
        .unwrap();

    let rel_active = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
        .unwrap();
    sheet
        .add_conditional(flag, vec![(vec![true], vec![rel_active])], vec![])
        .unwrap();

    // Write x with the highest strength so M0 (x,y→flag) is selected.
    sheet.write(y, 0_i32).unwrap();
    sheet.write(x, 10_i32).unwrap(); // x > y → flag = true
    sheet.write(a, 3_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<bool>(flag).unwrap(), true);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 6);

    // Flip: x=0 ≤ y → flag = false → rel_active inactive.
    sheet.write(y, 5_i32).unwrap();
    sheet.write(x, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<bool>(flag).unwrap(), false);
    // b keeps its last derived value (6) since rel_active is no longer active.
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 6);
}

#[test]
fn conditional_match_cell_is_derived_from_unconditional_relationship() {
    // The match cell (flag) is computed by an unconditional single-method relationship.
    let mut sheet = Sheet::new();
    let x = sheet.add_cell(5_i32);
    let flag = sheet.add_cell(false);
    let a = sheet.add_cell(3_i32);
    let b = sheet.add_cell(0_i32);

    // Unconditional: x → flag  (flag = x > 0)
    sheet
        .add_relationship(vec![Method::from_fn_1_1(x, flag, |x: &i32| Ok(*x > 0))])
        .unwrap();

    let rel_true = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
        .unwrap();

    sheet
        .add_conditional(flag, vec![(vec![true], vec![rel_true])], vec![])
        .unwrap();

    // x=5 > 0 → flag=true → rel_true active.
    sheet.write(x, 5_i32).unwrap();
    sheet.write(a, 3_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<bool>(flag).unwrap(), true);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 6);

    // x=-1 ≤ 0 → flag=false → no match, rel_true inactive.
    sheet.write(x, -1_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<bool>(flag).unwrap(), false);
    // b has no active relationship; it keeps its previous value.
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 6);
}
