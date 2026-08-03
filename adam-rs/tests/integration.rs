//! End-to-end integration tests for the adam-rs crate.

use std::any::TypeId;

use adam_rs::{Error, Method, Sheet};

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
fn single_method_forced_direction() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(0_i32); // b has higher priority
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 3))])
        .unwrap();

    sheet.propagate().unwrap();

    assert_eq!(*sheet.read::<i32>(b).unwrap(), 15);
}

#[test]
fn forced_direction_cascades_through_adjacent_relationship() {
    // R1: a -> b (single method) forces b, regardless of strength.
    // R2: b -> c or c -> b (two methods) — b is already forced by R1, so R2's
    // c -> b method can never fire without double-writing b; c is forced too.
    // b and c are added after a and never written, so if strength alone decided
    // source selection, either could wrongly become a source instead of a.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(2_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);

    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 10))])
        .unwrap();
    sheet
        .add_relationship(vec![
            Method::from_fn_1_1(b, c, |x: &i32| Ok(*x + 1)),
            Method::from_fn_1_1(c, b, |x: &i32| Ok(*x + 1)),
        ])
        .unwrap();

    sheet.propagate().unwrap();

    assert_eq!(*sheet.read::<i32>(b).unwrap(), 20);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 21);
}

#[test]
fn is_forced_false_before_propagate() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    assert!(!sheet.is_forced(b));
}

#[test]
fn is_forced_true_for_single_method_output() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 3))])
        .unwrap();

    sheet.propagate().unwrap();

    assert!(sheet.is_forced(b));
    assert!(!sheet.is_forced(a));
}

#[test]
fn is_forced_false_for_multi_method_relationship() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0.0_f64);
    let b = sheet.add_cell(0.0_f64);
    let c = sheet.add_cell(0.0_f64);
    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok((*x) * (*y))),
            Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok((*y) / (*x))),
            Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok((*y) / (*x))),
        ])
        .unwrap();
    sheet.write(a, 2.0_f64).unwrap();
    sheet.write(b, 3.0_f64).unwrap();

    sheet.propagate().unwrap();

    assert!(!sheet.is_forced(a));
    assert!(!sheet.is_forced(b));
    assert!(!sheet.is_forced(c));
}

#[test]
fn forced_cells_iterates_all_forced_cells() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(2_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 10))])
        .unwrap();
    sheet
        .add_relationship(vec![
            Method::from_fn_1_1(b, c, |x: &i32| Ok(*x + 1)),
            Method::from_fn_1_1(c, b, |x: &i32| Ok(*x + 1)),
        ])
        .unwrap();

    sheet.propagate().unwrap();

    let forced: std::collections::HashSet<_> = sheet.forced_cells().collect();
    assert_eq!(forced, std::collections::HashSet::from([b, c]));
}

#[test]
fn is_forced_respects_conditional_branch_activation() {
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

    // mode=0: rel_on inactive, b is not forced.
    sheet.write(mode, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(!sheet.is_forced(b));

    // mode=1: rel_on active, b is forced.
    sheet.write(mode, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(sheet.is_forced(b));
}

#[test]
fn is_relationship_forced_false_before_propagate() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    assert!(!sheet.is_relationship_forced(rel));
}

#[test]
fn is_relationship_forced_true_for_single_method_relationship() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(0_i32);
    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 3))])
        .unwrap();

    sheet.propagate().unwrap();

    assert!(sheet.is_relationship_forced(rel));
}

#[test]
fn is_relationship_forced_false_for_multi_method_relationship() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0.0_f64);
    let b = sheet.add_cell(0.0_f64);
    let c = sheet.add_cell(0.0_f64);
    let rel = sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok((*x) * (*y))),
            Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok((*y) / (*x))),
            Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok((*y) / (*x))),
        ])
        .unwrap();
    sheet.write(a, 2.0_f64).unwrap();
    sheet.write(b, 3.0_f64).unwrap();

    sheet.propagate().unwrap();

    assert!(!sheet.is_relationship_forced(rel));
}

#[test]
fn forced_relationships_cascade_through_adjacent_relationship() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(2_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);

    let r1 = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 10))])
        .unwrap();
    let r2 = sheet
        .add_relationship(vec![
            Method::from_fn_1_1(b, c, |x: &i32| Ok(*x + 1)),
            Method::from_fn_1_1(c, b, |x: &i32| Ok(*x + 1)),
        ])
        .unwrap();

    sheet.propagate().unwrap();

    assert!(sheet.is_relationship_forced(r1));
    assert!(sheet.is_relationship_forced(r2));
}

#[test]
fn forced_relationships_iterates_all_forced_relationships() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(2_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);

    let r1 = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 10))])
        .unwrap();
    let r2 = sheet
        .add_relationship(vec![
            Method::from_fn_1_1(b, c, |x: &i32| Ok(*x + 1)),
            Method::from_fn_1_1(c, b, |x: &i32| Ok(*x + 1)),
        ])
        .unwrap();

    sheet.propagate().unwrap();

    let forced: std::collections::HashSet<_> = sheet.forced_relationships().collect();
    assert_eq!(forced, std::collections::HashSet::from([r1, r2]));
}

#[test]
fn is_relationship_forced_respects_conditional_branch_activation() {
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

    // mode=0: rel_on inactive (not part of the planned active set at all).
    sheet.write(mode, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(!sheet.is_relationship_forced(rel_on));

    // mode=1: rel_on active, single method, forced.
    sheet.write(mode, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(sheet.is_relationship_forced(rel_on));
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
fn self_ref_pressure_persists_without_rewriting_anchor() {
    // a = min(a, b): b applies downward pressure on a, but a's original written
    // value must survive across rounds where only b is rewritten.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| {
            Ok((*x).min(*y))
        })])
        .unwrap();

    sheet.write(a, 10_i32).unwrap();
    sheet.write(b, 3_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 3);
    assert_eq!(*sheet.source::<i32>(a).unwrap(), 10);

    // Only b changes; a's original 10 (not the previous derived 3) is used.
    sheet.write(b, 20_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 10);
    assert_eq!(*sheet.source::<i32>(a).unwrap(), 10);

    sheet.write(b, 5_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    assert_eq!(*sheet.source::<i32>(a).unwrap(), 10);
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
fn conditional_forced_cell_shadows_original_value() {
    let mut sheet = Sheet::new();
    let p = sheet.add_cell(0_i32);
    let a = sheet.add_cell(7_i32);
    let b = sheet.add_cell(0_i32);

    let rel_force = sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_conditional(p, vec![(vec![1_i32], vec![rel_force])], vec![])
        .unwrap();

    sheet.write(p, 1_i32).unwrap();
    sheet.write(b, 42_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 42);
    assert_eq!(*sheet.source::<i32>(a).unwrap(), 7);
}

#[test]
fn conditional_forced_cell_reverts_to_source_when_deactivated() {
    let mut sheet = Sheet::new();
    let p = sheet.add_cell(0_i32);
    let a = sheet.add_cell(7_i32);
    let b = sheet.add_cell(0_i32);

    let rel_force = sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_conditional(p, vec![(vec![1_i32], vec![rel_force])], vec![])
        .unwrap();

    sheet.write(p, 1_i32).unwrap();
    sheet.write(b, 42_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 42);

    sheet.write(p, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(
        *sheet.read::<i32>(a).unwrap(),
        7,
        "a must revert to its original value, not stay at the stale forced 42"
    );
    assert_eq!(*sheet.source::<i32>(a).unwrap(), 7);
}

#[test]
fn changed_reports_cell_reverted_by_conditional_deactivation() {
    let mut sheet = Sheet::new();
    let p = sheet.add_cell(0_i32);
    let a = sheet.add_cell(7_i32);
    let b = sheet.add_cell(0_i32);

    let rel_force = sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_conditional(p, vec![(vec![1_i32], vec![rel_force])], vec![])
        .unwrap();

    sheet.write(p, 1_i32).unwrap();
    sheet.write(b, 42_i32).unwrap();
    sheet.propagate().unwrap();

    sheet.write(p, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(
        sheet.changed().any(|id| id == a),
        "a's effective value changed (42 -> 7) even though no method wrote to it this round"
    );
}

#[test]
fn pure_input_never_observes_stale_derived_after_conditional_deactivates() {
    // a is forced to b's value only when p == 1. c always reads a directly
    // (c = a * 10), regardless of the conditional. When p flips back to 0, a
    // must revert to its own source value (7) before c is recomputed — c must
    // never see a's stale forced value from the previous round.
    let mut sheet = Sheet::new();
    let p = sheet.add_cell(0_i32);
    let a = sheet.add_cell(7_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);

    let rel_force = sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, c, |x: &i32| Ok(*x * 10))])
        .unwrap();
    sheet
        .add_conditional(p, vec![(vec![1_i32], vec![rel_force])], vec![])
        .unwrap();

    sheet.write(p, 1_i32).unwrap();
    sheet.write(b, 42_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 42);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 420);

    sheet.write(p, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 7);
    assert_eq!(
        *sheet.read::<i32>(c).unwrap(),
        70,
        "c must be derived from a's reverted source value, not the stale forced 42"
    );
}

#[test]
fn explicit_write_to_forced_cell_takes_immediate_effect() {
    let mut sheet = Sheet::new();
    let p = sheet.add_cell(1_i32);
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);

    let rel_force = sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_conditional(p, vec![(vec![1_i32], vec![rel_force])], vec![])
        .unwrap();

    sheet.write(p, 1_i32).unwrap();
    sheet.write(b, 42_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 42);

    // Direct write takes effect immediately, before the next propagate() re-forces it.
    sheet.write(a, 99_i32).unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 99);
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

    // mode=1 does not match; b reverts to its original source value.
    sheet.write(mode, 1_i32).unwrap();
    sheet.propagate().unwrap();
    // b is no longer derived; it reverts to its source value (0), not the stale forced 7.
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 0);
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
    assert!(*sheet.read::<bool>(flag).unwrap());
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 6);

    // Flip: x=0 ≤ y → flag = false → rel_active inactive.
    sheet.write(y, 5_i32).unwrap();
    sheet.write(x, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(!*sheet.read::<bool>(flag).unwrap());
    // b reverts to its source value (0) since rel_active is no longer active.
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 0);
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
    assert!(*sheet.read::<bool>(flag).unwrap());
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 6);

    // x=-1 ≤ 0 → flag=false → no match, rel_true inactive.
    sheet.write(x, -1_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(!*sheet.read::<bool>(flag).unwrap());
    // b has no active relationship; it reverts to its source value (0).
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 0);
}

#[test]
fn cell_shadowed_as_self_ref_in_one_branch_and_forced_output_in_another() {
    // p == 0: a <= b enforced by a two-way self-referencing relationship.
    // p != 0 (default): a and b are forced from each other directly, whichever
    // is the stronger (more recently written) cell wins.
    let mut sheet = Sheet::new();
    let p = sheet.add_cell(0_i32);
    let a = sheet.add_cell(4_i32);
    let b = sheet.add_cell(9_i32);

    let rel_self_ref = sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| Ok((*x).min(*y))),
            Method::from_fn_2_1([a, b], b, |x: &i32, y: &i32| Ok((*x).max(*y))),
        ])
        .unwrap();
    let rel_force = sheet
        .add_relationship(vec![
            Method::from_fn_1_1(b, a, |y: &i32| Ok(*y)),
            Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
        ])
        .unwrap();
    sheet
        .add_conditional(p, vec![(vec![0_i32], vec![rel_self_ref])], vec![rel_force])
        .unwrap();

    // p == 0: self-referencing branch. a=4, b=9 already satisfy a <= b: unchanged.
    sheet.write(p, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 4);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 9);
    assert_eq!(*sheet.source::<i32>(a).unwrap(), 4);
    assert_eq!(*sheet.source::<i32>(b).unwrap(), 9);

    // p == 1: default (forcing) branch. b is the more recently written cell,
    // so a <- b.
    sheet.write(a, 4_i32).unwrap();
    sheet.write(b, 20_i32).unwrap();
    sheet.write(p, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 20);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 20);
    // Sources are untouched by the forcing branch.
    assert_eq!(*sheet.source::<i32>(a).unwrap(), 4);
    assert_eq!(*sheet.source::<i32>(b).unwrap(), 20);

    // Back to p == 0: self-ref recomputed fresh from each cell's own source
    // (4 and 20), not from the stale forced value.
    sheet.write(p, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 4);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 20);
}
