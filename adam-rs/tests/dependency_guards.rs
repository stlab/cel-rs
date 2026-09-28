//! Contract tests for the static guard-independence invariant: a filter's arguments and a
//! conditional's match subject must not depend on a cell they govern.

use adam_rs::{Error, ErrorSite, Filter, MatchExpr, Method, Sheet};

fn min_filter(bound: adam_rs::CellId) -> Filter {
    Filter::from_fn_1(bound, |v: &i32, b: &i32| Ok((*v).min(*b)))
}

#[test]
fn add_filter_rejects_an_argument_derived_from_the_filtered_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(0_i32);
    let r = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    let err = sheet.add_filter(a, min_filter(b)).unwrap_err();
    match err {
        Error::DependencyCycle { sites } => assert_eq!(
            sites,
            vec![
                ErrorSite::Cell(a),
                ErrorSite::Relationship(r),
                ErrorSite::Cell(b)
            ]
        ),
        other => panic!("expected DependencyCycle, got {other:?}"),
    }
    assert_eq!(
        sheet.filter_args(a),
        None,
        "the sheet must be left unchanged"
    );
    assert!(sheet.filter_dependents(b).is_empty());
    sheet.propagate().unwrap();
}

#[test]
fn add_filter_rejects_the_issue_153_sum_example() {
    // z = x + y with methods z <- (x, y) and x <- (z, y); y filtered by z. y is never an
    // output, so the edge y -> z of z <- (x, y) always exists.
    let mut sheet = Sheet::new();
    let x = sheet.add_cell(1_i32);
    let y = sheet.add_cell(2_i32);
    let z = sheet.add_cell(3_i32);
    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([x, y], z, |a: &i32, b: &i32| Ok(a + b)),
            Method::from_fn_2_1([z, y], x, |c: &i32, b: &i32| Ok(c - b)),
        ])
        .unwrap();
    assert!(matches!(
        sheet.add_filter(y, min_filter(z)),
        Err(Error::DependencyCycle { .. })
    ));
}

#[test]
fn add_filter_rejects_the_issue_153_two_relationship_example() {
    // R1 over {x, y, z} with all three single-output methods, R2 over {w, y}; y filtered
    // by z. R1's z <- (x, y) makes z depend on y regardless of which plan runs.
    let mut sheet = Sheet::new();
    let w = sheet.add_cell(1_i32);
    let z = sheet.add_cell(3_i32);
    let x = sheet.add_cell(1_i32);
    let y = sheet.add_cell(2_i32);
    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([x, y], z, |a: &i32, b: &i32| Ok(a + b)),
            Method::from_fn_2_1([y, z], x, |b: &i32, c: &i32| Ok(c - b)),
            Method::from_fn_2_1([x, z], y, |a: &i32, c: &i32| Ok(c - a)),
        ])
        .unwrap();
    sheet
        .add_relationship(vec![
            Method::from_fn_1_1(w, y, |v: &i32| Ok(*v)),
            Method::from_fn_1_1(y, w, |v: &i32| Ok(*v)),
        ])
        .unwrap();
    assert!(matches!(
        sheet.add_filter(y, min_filter(z)),
        Err(Error::DependencyCycle { .. })
    ));
}

#[test]
fn add_filter_accepts_an_upstream_argument_and_conforms_on_propagate() {
    let mut sheet = Sheet::new();
    let hi = sheet.add_cell(3_i32);
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    sheet.add_filter(a, min_filter(hi)).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 3);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 3);
}

#[test]
fn add_filter_accepts_an_argument_computed_from_a_self_referencing_input() {
    // (x, y) <- (x, q) reads x at its pre-round value, so y does not depend on x and
    // filtering x by y closes no cycle.
    use std::any::{Any, TypeId};
    let mut sheet = Sheet::new();
    let x = sheet.add_cell(5_i32);
    let q = sheet.add_cell(1_i32);
    let y = sheet.add_cell(0_i32);
    let ids = vec![TypeId::of::<i32>(); 2];
    sheet
        .add_relationship(vec![Method::new(
            vec![x, q],
            vec![x, y],
            ids.clone(),
            ids,
            |args: &[&dyn Any]| {
                let x = *args[0].downcast_ref::<i32>().unwrap();
                let q = *args[1].downcast_ref::<i32>().unwrap();
                Ok(vec![Box::new(x) as Box<dyn Any>, Box::new(x + q)])
            },
        )])
        .unwrap();
    sheet.add_filter(x, min_filter(y)).unwrap();
    assert_eq!(sheet.filter_args(x), Some(&[y][..]));
    sheet.propagate().unwrap();
}

#[test]
fn add_filter_rejects_a_filter_closing_an_existing_conditional_guard() {
    // Conditional on m governs k -> o; filtering m by o closes m -gate-> o -filter-> m.
    let mut sheet = Sheet::new();
    let m = sheet.add_cell(0_i32);
    let k = sheet.add_cell(0_i32);
    let o = sheet.add_cell(0_i32);
    let r = sheet
        .add_relationship(vec![Method::from_fn_1_1(k, o, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_conditional(MatchExpr::cell(m), vec![(vec![0_i32], vec![r])], vec![])
        .unwrap();
    assert!(matches!(
        sheet.add_filter(m, min_filter(o)),
        Err(Error::DependencyCycle { .. })
    ));
    assert_eq!(sheet.filter_args(m), None);
}

#[test]
fn add_relationship_rejects_a_relationship_that_makes_a_filter_argument_dependent() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(0_i32);
    sheet.add_filter(a, min_filter(b)).unwrap();
    let err = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap_err();
    match err {
        Error::DependencyCycle { sites } => assert_eq!(
            sites,
            vec![
                ErrorSite::Cell(a),
                ErrorSite::MethodIndex(0),
                ErrorSite::Cell(b)
            ]
        ),
        other => panic!("expected DependencyCycle, got {other:?}"),
    }
    assert_eq!(
        sheet.relationships().count(),
        0,
        "the sheet must be left unchanged"
    );
    sheet.propagate().unwrap();
    assert_eq!(
        *sheet.read::<i32>(a).unwrap(),
        0,
        "filter still clamps a to b"
    );
}

#[test]
fn add_relationship_rejects_a_relationship_that_feeds_a_match_cell_from_its_branch() {
    // Conditional on p governs k -> o; a later unconditional o -> p closes the cycle.
    let mut sheet = Sheet::new();
    let p = sheet.add_cell(0_i32);
    let k = sheet.add_cell(0_i32);
    let o = sheet.add_cell(0_i32);
    let r = sheet
        .add_relationship(vec![Method::from_fn_1_1(k, o, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_conditional(MatchExpr::cell(p), vec![(vec![0_i32], vec![r])], vec![])
        .unwrap();
    assert!(matches!(
        sheet.add_relationship(vec![Method::from_fn_1_1(o, p, |x: &i32| Ok(*x))]),
        Err(Error::DependencyCycle { .. })
    ));
    assert_eq!(sheet.relationships().count(), 1);
}

#[test]
fn add_conditional_rejects_a_single_method_branch_that_writes_its_own_match_cell() {
    let mut sheet = Sheet::new();
    let mode = sheet.add_cell(0_i32);
    let other = sheet.add_cell(0_i32);
    let r = sheet
        .add_relationship(vec![Method::from_fn_1_1(other, mode, |x: &i32| Ok(*x))])
        .unwrap();
    let err = sheet
        .add_conditional(MatchExpr::cell(mode), vec![(vec![0_i32], vec![r])], vec![])
        .unwrap_err();
    match err {
        Error::DependencyCycle { sites } => assert_eq!(sites, vec![ErrorSite::Cell(mode)]),
        other => panic!("expected DependencyCycle, got {other:?}"),
    }
}

#[test]
fn add_conditional_rejects_a_branch_feeding_the_match_cell_through_another_relationship() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let p = sheet.add_cell(0_i32);
    let upstream = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, p, |x: &i32| Ok(*x))])
        .unwrap();
    let branch = sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();
    let err = sheet
        .add_conditional(
            MatchExpr::cell(p),
            vec![(vec![0_i32], vec![branch])],
            vec![],
        )
        .unwrap_err();
    match err {
        Error::DependencyCycle { sites } => assert_eq!(
            sites,
            vec![
                ErrorSite::Cell(a),
                ErrorSite::Relationship(upstream),
                ErrorSite::Cell(p)
            ]
        ),
        other => panic!("expected DependencyCycle, got {other:?}"),
    }
}

#[test]
fn add_conditional_accepts_a_multi_method_branch_that_only_reads_match_contributors() {
    // a feeds p; the branch relationship over {a, b, c} always reads a and writes b or c,
    // so nothing it governs reaches p.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(1_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);
    let p = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, p, |x: &i32| Ok(*x))])
        .unwrap();
    let branch = sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, c], b, |x: &i32, y: &i32| Ok(x + y)),
            Method::from_fn_2_1([a, b], c, |x: &i32, y: &i32| Ok(y - x)),
        ])
        .unwrap();
    let cid = sheet
        .add_conditional(
            MatchExpr::cell(p),
            vec![(vec![1_i32], vec![branch])],
            vec![],
        )
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(sheet.conditional_active_branch(cid).unwrap(), Some(0));
}

#[test]
fn add_conditional_rejects_a_conditional_closing_an_existing_filter_guard() {
    // y filtered by z; a conditional on y governing k -> z closes y -gate-> z -filter-> y.
    let mut sheet = Sheet::new();
    let y = sheet.add_cell(0_i32);
    let z = sheet.add_cell(0_i32);
    let k = sheet.add_cell(7_i32);
    sheet.add_filter(y, min_filter(z)).unwrap();
    let r = sheet
        .add_relationship(vec![Method::from_fn_1_1(k, z, |x: &i32| Ok(*x))])
        .unwrap();
    assert!(matches!(
        sheet.add_conditional(MatchExpr::cell(y), vec![(vec![0_i32], vec![r])], vec![]),
        Err(Error::DependencyCycle { .. })
    ));
    // Rolled back: r is still unconditional, so it runs.
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(z).unwrap(), 7);
}
