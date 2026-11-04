//! Exercises graph ownership, filter provenance and publication contracts.

use adam_rs::{CellId, Error, Filter, MatchExpr, Method, Sheet};
use std::any::TypeId;
use std::collections::HashSet;

/// Carries an owned scalar without imposing value-copy semantics.
#[derive(Debug, PartialEq)]
struct NonClone(i32);

/// Reports no capture sources until a propagation has succeeded.
#[test]
fn source_cells_are_empty_before_successful_propagation() {
    let mut sheet = Sheet::new();
    assert_eq!(sheet.source_cells().count(), 0);
    sheet.add_cell(1_i32);
    assert_eq!(sheet.source_cells().count(), 0);
    sheet.propagate().unwrap();
    assert_eq!(sheet.source_cells().count(), 1);
}

/// Includes unclaimed and self-referencing inputs once, excluding pure outputs.
#[test]
fn source_cells_include_shadowed_inputs_without_requiring_clone() {
    let mut sheet = Sheet::new();
    let host = sheet.add_source(NonClone(7));
    let input = sheet.add_cell(10_i32);
    let shadowed = sheet.add_cell(20_i32);
    let output = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_2_1(
            [input, shadowed],
            shadowed,
            |a: &i32, b: &i32| Ok((*a).max(*b)),
        )])
        .unwrap();
    sheet
        .add_out(
            Method::from_fn_1_1(shadowed, output, |v: &i32| Ok(*v)),
            vec![],
        )
        .unwrap();
    sheet.write(input, 100_i32).unwrap();
    sheet.propagate().unwrap();

    let sources: Vec<_> = sheet.source_cells().collect();
    assert_eq!(sources.len(), 3);
    assert_eq!(
        sources.into_iter().collect::<HashSet<_>>(),
        HashSet::from([host, input, shadowed])
    );
    assert!(!sheet.is_source(shadowed));
    assert_eq!(sheet.read::<NonClone>(host).unwrap().0, 7);
    assert_eq!(*sheet.source::<i32>(shadowed).unwrap(), 20);
    assert_eq!(*sheet.read::<i32>(shadowed).unwrap(), 100);
}

/// Adds an inequality chain with distinct initial aspirations.
fn add_snapshot_chain(sheet: &mut Sheet) -> [CellId; 3] {
    let a = sheet.add_cell(10_i32);
    let b = sheet.add_cell(20_i32);
    let c = sheet.add_cell(30_i32);
    // Constrain each neighboring pair without discarding either aspiration.
    for [left, right] in [[a, b], [b, c]] {
        sheet
            .add_relationship(vec![
                Method::from_fn_2_1([left, right], left, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([left, right], right, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
    }
    [a, b, c]
}

/// Replays effective shadow values in every order using host identities, not live IDs.
#[test]
fn source_cells_snapshot_replays_inequality_chain_in_any_write_order() {
    let mut original = Sheet::new();
    let cells = add_snapshot_chain(&mut original);
    original.propagate().unwrap();
    original.write(cells[0], 100_i32).unwrap();
    original.propagate().unwrap();
    let captured: Vec<_> = original
        .source_cells()
        .map(|id| {
            (
                cells.iter().position(|&cell| cell == id).unwrap(),
                *original.read::<i32>(id).unwrap(),
            )
        })
        .collect();
    assert_eq!(captured.len(), 3);
    assert!(captured.iter().all(|&(_, value)| value == 100));

    // Replay the complete batch before propagating, independent of dictionary order.
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut replay = Sheet::new();
        replay.add_source(());
        let targets = add_snapshot_chain(&mut replay);
        replay.propagate().unwrap();
        for index in order {
            let (identity, value) = captured[index];
            replay.write(targets[identity], value).unwrap();
        }
        replay.propagate().unwrap();
        assert_eq!(
            targets.map(|id| *replay.read::<i32>(id).unwrap()),
            [100, 100, 100]
        );

        replay.write(targets[0], 0_i32).unwrap();
        replay.propagate().unwrap();
        assert_eq!(
            targets.map(|id| *replay.read::<i32>(id).unwrap()),
            [0, 100, 100]
        );
    }
    original.write(cells[0], 0_i32).unwrap();
    original.propagate().unwrap();
    assert_eq!(
        cells.map(|id| *original.read::<i32>(id).unwrap()),
        [0, 20, 30]
    );
}

/// Captures only the winning input after a priority-only write changes direction.
#[test]
fn source_cells_snapshot_tracks_multiway_direction_after_same_value_writes() {
    // Choose each direction by rewriting its input without changing its value.
    for winner in 0..2 {
        let mut original = Sheet::new();
        let cells = [original.add_cell(1_i32), original.add_cell(2_i32)];
        original
            .add_relationship(vec![
                Method::from_fn_1_1(cells[0], cells[1], |v: &i32| Ok(*v)),
                Method::from_fn_1_1(cells[1], cells[0], |v: &i32| Ok(*v)),
            ])
            .unwrap();
        original.write(cells[0], 9_i32).unwrap();
        original.write(cells[1], 9_i32).unwrap();
        original.write(cells[winner], 9_i32).unwrap();
        original.propagate().unwrap();
        let captured: Vec<_> = original.source_cells().collect();
        assert_eq!(captured, [cells[winner]]);
        let value = *original.read::<i32>(captured[0]).unwrap();

        let mut replay = Sheet::new();
        replay.add_source(());
        let targets = [replay.add_cell(0_i32), replay.add_cell(17_i32)];
        replay
            .add_relationship(vec![
                Method::from_fn_1_1(targets[0], targets[1], |v: &i32| Ok(*v)),
                Method::from_fn_1_1(targets[1], targets[0], |v: &i32| Ok(*v)),
            ])
            .unwrap();
        replay.propagate().unwrap();
        replay.write(targets[winner], value).unwrap();
        replay.propagate().unwrap();
        assert_eq!(targets.map(|id| *replay.read::<i32>(id).unwrap()), [9, 9]);
        assert!(replay.is_source(targets[winner]));
    }
}

/// Adds a dynamically filtered input and a conditional pure output.
fn add_filtered_snapshot(sheet: &mut Sheet) -> [CellId; 4] {
    let enabled = sheet.add_source(false);
    let limit = sheet.add_source(50_i32);
    let input = sheet.add_cell(40_i32);
    let output = sheet.add_cell(7_i32);
    sheet
        .add_filter(
            input,
            Filter::range(
                TypeId::of::<i32>(),
                vec![limit],
                vec![TypeId::of::<i32>()],
                |value, args| {
                    Ok(Box::new(
                        (*value.downcast_ref::<i32>().unwrap())
                            .clamp(0, *args[0].downcast_ref::<i32>().unwrap()),
                    ))
                },
                |args| Some((Box::new(0_i32), Box::new(*args[0].downcast_ref::<i32>()?))),
            ),
        )
        .unwrap();
    let relationship = sheet
        .add_relationship(vec![Method::from_fn_1_1(input, output, |v: &i32| Ok(*v))])
        .unwrap();
    sheet
        .add_conditional(
            MatchExpr::cell(enabled),
            vec![(vec![true], vec![relationship])],
            vec![],
        )
        .unwrap();
    [enabled, limit, input, output]
}

/// Replays filtered values and a conditional selector without capturing the forced output.
#[test]
fn source_cells_snapshot_replays_filters_and_conditionals_in_any_write_order() {
    let mut original = Sheet::new();
    let [enabled, limit, input, output] = add_filtered_snapshot(&mut original);
    original.propagate().unwrap();
    original.write(enabled, true).unwrap();
    original.write(limit, 20_i32).unwrap();
    original.propagate().unwrap();
    let sources: Vec<_> = original.source_cells().collect();
    assert_eq!(sources.len(), 3);
    assert_eq!(
        sources.into_iter().collect::<HashSet<_>>(),
        HashSet::from([enabled, limit, input])
    );
    assert_eq!(*original.source::<i32>(input).unwrap(), 40);
    assert_eq!(*original.read::<i32>(input).unwrap(), 20);
    assert_eq!(*original.read::<i32>(output).unwrap(), 20);
    let captured_enabled = *original.read::<bool>(enabled).unwrap();
    let captured_limit = *original.read::<i32>(limit).unwrap();
    let captured_input = *original.read::<i32>(input).unwrap();

    // Host schema positions identify typed fields across independently built sheets.
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut replay = Sheet::new();
        replay.add_source(());
        let targets = add_filtered_snapshot(&mut replay);
        replay.propagate().unwrap();
        for field in order {
            match field {
                0 => replay.write(targets[0], captured_enabled).unwrap(),
                1 => replay.write(targets[1], captured_limit).unwrap(),
                2 => replay.write(targets[2], captured_input).unwrap(),
                _ => unreachable!(),
            }
        }
        replay.propagate().unwrap();
        assert!(*replay.read::<bool>(targets[0]).unwrap());
        assert_eq!(*replay.read::<i32>(targets[1]).unwrap(), 20);
        assert_eq!(*replay.read::<i32>(targets[2]).unwrap(), 20);
        assert_eq!(*replay.read::<i32>(targets[3]).unwrap(), 20);
    }
}

/// Returns freshly evaluated range bounds without copying their concrete carrier.
#[test]
fn range_bounds_need_not_be_clone() {
    let mut sheet = Sheet::new();
    let cell = sheet.add_cell(NonClone(1));
    sheet
        .add_filter(
            cell,
            Filter::range(
                TypeId::of::<NonClone>(),
                vec![],
                vec![],
                |v, _| {
                    Ok(Box::new(NonClone(
                        v.downcast_ref::<NonClone>().unwrap().0.clamp(0, 10),
                    )))
                },
                |_| Some((Box::new(NonClone(0)), Box::new(NonClone(10)))),
            ),
        )
        .unwrap();
    assert_eq!(
        sheet.filter_range::<NonClone>(cell),
        Some((NonClone(0), NonClone(10)))
    );
}

/// Builds fresh conditional keys without requiring them to support copying.
#[test]
fn conditional_factory_keys_need_not_be_clone() {
    let mut builder = adam_rs::TemplateBuilder::new();
    let flag = builder.parameter::<NonClone>("flag").unwrap();
    builder
        .conditional_factory::<NonClone, _, _>(
            &[flag],
            |ids| Ok(MatchExpr::cell(ids[0])),
            || Ok(vec![vec![NonClone(1)]]),
            vec![vec![]],
            vec![],
        )
        .unwrap();
    let template = builder.finish().unwrap();
    let mut sheet = Sheet::new();
    let flag = sheet.add_cell(NonClone(1));
    let instance = template.instantiate(&mut sheet, &[("flag", flag)]).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(instance.occurrences()[0].conditionals().len(), 1);
}

/// Rejects every ordinary writer of an already fixed Out cell without changing its producer.
#[test]
fn ordinary_relationship_cannot_replace_an_out_writer() {
    let mut sheet = Sheet::new();
    let source = sheet.add_cell(7_i32);
    let output = sheet.add_cell(0_i32);
    sheet
        .add_out(
            Method::from_fn_1_1(source, output, |v: &i32| Ok(*v)),
            vec![],
        )
        .unwrap();
    sheet.propagate().unwrap();
    let before: Vec<_> = sheet.relationships().collect();
    assert!(matches!(
        sheet.add_relationship(vec![Method::from_fn_1_1(source, output, |v: &i32| Ok(*v))]),
        Err(Error::InvalidCellKind { .. })
    ));
    assert_eq!(sheet.relationships().collect::<Vec<_>>(), before);
    sheet.write(source, 9_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(output).unwrap(), 9);
}

/// Keeps a fixed Out writer unconditional in both branch and default registrations.
#[test]
fn conditional_cannot_hide_an_out_writer() {
    for in_default in [false, true] {
        let mut sheet = Sheet::new();
        let source = sheet.add_cell(7_i32);
        let output = sheet.add_cell(0_i32);
        sheet
            .add_out(
                Method::from_fn_1_1(source, output, |v: &i32| Ok(*v)),
                vec![],
            )
            .unwrap();
        let writer = sheet.relationships().next().unwrap();
        let (branches, default) = if in_default {
            (vec![(vec![7_i32], vec![])], vec![writer])
        } else {
            (vec![(vec![0_i32], vec![writer])], vec![])
        };
        assert!(matches!(
            sheet.add_conditional(MatchExpr::cell(source), branches, default),
            Err(Error::InvalidCellKind { .. })
        ));
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(output).unwrap(), 7);
    }
}

/// Clears obsolete filter diagnostics immediately after a valid external write.
#[test]
fn external_write_clears_its_filter_diagnostic() {
    let mut sheet = Sheet::new();
    let source = sheet.add_cell(120_i32);
    let output = sheet.add_cell(0_i32);
    sheet
        .add_filter(output, Filter::from_fn_0(|v: &i32| Ok((*v).clamp(0, 100))))
        .unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(source, output, |v: &i32| Ok(*v))])
        .unwrap();
    sheet.propagate().unwrap();
    assert!(sheet.filter_violation(output).is_some());
    sheet.write(output, 50_i32).unwrap();
    assert_eq!(*sheet.read::<i32>(output).unwrap(), 50);
    assert!(sheet.filter_violation(output).is_none());
    assert_eq!(sheet.filter_violated_cells().count(), 0);
    assert!(sheet.filter_violation_cells().is_empty());
}

/// Includes transitive authoritative filter arguments but not diagnostic-only arguments.
#[test]
fn contributors_follow_live_source_filters() {
    let mut sheet = Sheet::new();
    let limit_source = sheet.add_cell(10_i32);
    let limit = sheet.add_cell(0_i32);
    let source = sheet.add_cell(90_i32);
    let output = sheet.add_cell(0_i32);
    let diagnostic_limit = sheet.add_cell(100_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(limit_source, limit, |v: &i32| {
            Ok(*v)
        })])
        .unwrap();
    sheet
        .add_filter(
            source,
            Filter::from_fn_1(limit, |v: &i32, hi: &i32| Ok((*v).min(*hi))),
        )
        .unwrap();
    sheet
        .add_filter(
            output,
            Filter::from_fn_1(diagnostic_limit, |v: &i32, hi: &i32| Ok((*v).min(*hi))),
        )
        .unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(source, output, |v: &i32| Ok(*v))])
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(output).unwrap(), 10);
    assert_eq!(
        sheet.contributing_cells(output),
        HashSet::from([source, limit_source])
    );
    sheet.write(limit_source, 20_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(output).unwrap(), 20);
}

/// Excludes a self-referencing writer's diagnostic filter from value provenance.
#[test]
fn self_referencing_filter_is_not_a_value_dependency() {
    let mut sheet = Sheet::new();
    let limit = sheet.add_cell(10_i32);
    let cell = sheet.add_cell(5_i32);
    sheet
        .add_filter(
            cell,
            Filter::from_fn_1(limit, |v: &i32, hi: &i32| Ok((*v).min(*hi))),
        )
        .unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(cell, cell, |v: &i32| {
            Ok((*v).max(0))
        })])
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(sheet.contributing_cells(cell), HashSet::from([cell]));
}

/// Includes a filter argument when another strength order can leave its cell unclaimed.
#[test]
fn contributors_include_filters_of_alternative_sources() {
    let mut sheet = Sheet::new();
    let limit = sheet.add_cell(10_i32);
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(5_i32);
    sheet
        .add_filter(
            a,
            Filter::from_fn_1(limit, |v: &i32, hi: &i32| Ok((*v).min(*hi))),
        )
        .unwrap();
    sheet
        .add_relationship(vec![
            Method::from_fn_1_1(a, b, |v: &i32| Ok(*v)),
            Method::from_fn_1_1(b, a, |v: &i32| Ok(*v)),
        ])
        .unwrap();
    sheet.propagate().unwrap();
    assert!(!sheet.is_source(a));
    assert_eq!(sheet.contributing_cells(a), HashSet::from([limit, a, b]));
}

/// Returns only the queried cell until a completed propagation establishes contributors.
#[test]
fn contributors_before_propagation_ignore_unselected_branches() {
    let mut sheet = Sheet::new();
    let mode = sheet.add_cell(true);
    let input = sheet.add_cell(5_i32);
    let output = sheet.add_cell(0_i32);
    let relation = sheet
        .add_relationship(vec![Method::from_fn_1_1(input, output, |v: &i32| Ok(*v))])
        .unwrap();
    sheet
        .add_conditional(
            MatchExpr::cell(mode),
            vec![(vec![true], vec![relation])],
            vec![],
        )
        .unwrap();
    assert_eq!(sheet.contributing_cells(output), HashSet::from([output]));
}

/// Checks filtered contributor roots through an Out alongside independent reversible pairs.
#[test]
fn disconnected_relationships_preserve_filtered_contributors() {
    for pair_count in [0, 4, 8, 12] {
        let mut sheet = Sheet::new();
        let target = sheet.add_cell(5_i32);
        let output = sheet.add_cell(0_i32);
        let bound = sheet.add_cell(10_i32);
        sheet
            .add_filter(
                target,
                Filter::from_fn_1(bound, |v: &i32, hi: &i32| Ok((*v).min(*hi))),
            )
            .unwrap();
        sheet
            .add_out(
                Method::from_fn_1_1(target, output, |v: &i32| Ok(*v)),
                vec![],
            )
            .unwrap();
        let pairs: Vec<_> = (0..pair_count)
            .map(|_| (sheet.add_cell(1_i32), sheet.add_cell(1_i32)))
            .collect();
        for &(a, b) in pairs.iter().rev() {
            sheet
                .add_relationship(vec![
                    Method::from_fn_1_1(a, b, |v: &i32| Ok(*v)),
                    Method::from_fn_1_1(b, a, |v: &i32| Ok(*v)),
                ])
                .unwrap();
        }
        sheet
            .add_relationship(vec![Method::from_fn_1_1(target, target, |v: &i32| Ok(*v))])
            .unwrap();
        let started = std::time::Instant::now();
        sheet.propagate().unwrap();
        let propagation = started.elapsed();
        assert!(!sheet.is_forced(target));
        assert!(!sheet.is_source(target));
        let started = std::time::Instant::now();
        for _ in 0..3 {
            assert_eq!(sheet.contributing_cells(output), HashSet::from([target]));
        }
        eprintln!(
            "independent_pairs={pair_count}; propagation={propagation:?}; three_queries={:?}",
            started.elapsed()
        );
    }
}

/// Preserves completed active-set contributors through dirty structure and both failure phases.
#[test]
fn filtered_contributors_keep_completed_active_graph_until_success() {
    let mut sheet = Sheet::new();
    let mode = sheet.add_cell(false);
    let target = sheet.add_cell(5_i32);
    let peer = sheet.add_cell(5_i32);
    let bound = sheet.add_cell(10_i32);
    let output = sheet.add_cell(0_i32);
    let fail_method = sheet.add_cell(false);
    let fail_requirement = sheet.add_cell(false);
    let failure_output = sheet.add_cell(false);
    sheet
        .add_filter(
            target,
            Filter::from_fn_1(bound, |v: &i32, hi: &i32| Ok((*v).min(*hi))),
        )
        .unwrap();
    sheet
        .add_out(
            Method::from_fn_1_1(target, output, |v: &i32| Ok(*v)),
            vec![adam_rs::Requirement::from_fn_1(
                fail_requirement,
                |fail: &bool| {
                    if *fail {
                        anyhow::bail!("requirement failure");
                    }
                    Ok(true)
                },
            )],
        )
        .unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(
            fail_method,
            failure_output,
            |fail: &bool| {
                if *fail {
                    anyhow::bail!("method failure");
                }
                Ok(false)
            },
        )])
        .unwrap();
    let reversible = sheet
        .add_relationship(vec![
            Method::from_fn_1_1(target, peer, |v: &i32| Ok(*v)),
            Method::from_fn_1_1(peer, target, |v: &i32| Ok(*v)),
        ])
        .unwrap();
    let self_writer = sheet
        .add_relationship(vec![Method::from_fn_1_1(target, target, |v: &i32| Ok(*v))])
        .unwrap();
    sheet
        .add_conditional(
            MatchExpr::cell(mode),
            vec![(vec![true], vec![self_writer])],
            vec![reversible],
        )
        .unwrap();
    sheet.propagate().unwrap();
    let previous_roots = HashSet::from([mode, target, peer, bound]);
    assert!(!sheet.is_source(target));
    assert_eq!(sheet.contributing_cells(output), previous_roots);

    sheet.write(mode, true).unwrap();
    sheet.write(target, 7).unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(peer, peer, |v: &i32| Ok(*v))])
        .unwrap();
    assert_eq!(sheet.contributing_cells(output), previous_roots);
    sheet.write(fail_method, true).unwrap();
    assert!(matches!(sheet.propagate(), Err(Error::MethodFailed { .. })));
    assert_eq!(sheet.read::<i32>(output).unwrap(), &5);
    assert_eq!(sheet.contributing_cells(output), previous_roots);

    sheet.write(fail_method, false).unwrap();
    sheet.write(fail_requirement, true).unwrap();
    assert!(matches!(sheet.propagate(), Err(Error::MethodFailed { .. })));
    assert_eq!(sheet.read::<i32>(output).unwrap(), &7);
    assert_eq!(sheet.contributing_cells(output), previous_roots);

    sheet.write(fail_requirement, false).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(
        sheet.contributing_cells(output),
        HashSet::from([mode, target])
    );
    sheet.write(mode, false).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(
        sheet.contributing_cells(output),
        HashSet::from([mode, target, peer])
    );
}
