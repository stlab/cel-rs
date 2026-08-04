//! Planning pass: selects one method per relationship and returns them in dependency
//! order.
//!
//! The planner finds the strength-optimal acyclic assignment of methods to
//! relationships: [`release::resolve`] greedily tries, in descending cell-strength
//! order, to leave each cell unclaimed (a source), keeping the change only when a
//! valid method assignment still exists ([`matching::Assignment::solve`]) *and* its
//! induced dependency digraph is acyclic ([`digraph::is_acyclic`]). This single
//! mechanism handles both ordinary strength-based method selection (an uncontested
//! relationship's choice of which cell to leave exogenous) and overlapping cyclic
//! ("diamond") structures uniformly -- both are instances of "does releasing this cell
//! still admit a valid acyclic assignment". See
//! `docs/superpowers/specs/2026-08-04-cyclic-constraint-planner-design.md` for the
//! full design rationale and literature grounding.
//!
//! Once [`release::resolve`] succeeds, its result's induced digraph is guaranteed
//! acyclic, so a plain topological sort (reusing [`scc::tarjan_scc`], which produces
//! components in reverse topological order on an acyclic graph -- each component is
//! then a single node) yields `execution_order` directly.
//!
//! A separate fixpoint, [`forced_output_cells`], computes cells that can never be a
//! source (a relationship's method structure guarantees the cell is always produced),
//! purely for the informational [`Plan::forced_outputs`] / [`Plan::forced_relationships`]
//! fields exposed to callers (e.g. disabling form fields in `begin`'s Inspector) -- it
//! does not influence method selection above, which discovers the same infeasibility
//! structurally via failed augmenting-path displacement in [`matching::Assignment::solve`].

use std::collections::{HashMap, HashSet};

use slotmap::SlotMap;

use crate::{
    cell::{CellData, CellId},
    error::Error,
    relationship::{RelationshipData, RelationshipId},
};

mod digraph;
mod matching;
mod release;
mod scc;

use digraph::{Node, build_digraph};
use matching::pure_outputs;

/// The output of the planning pass.
pub(crate) struct Plan {
    /// Selected `(RelationshipId, method_index)` pairs in execution order.
    pub(crate) execution_order: Vec<(RelationshipId, usize)>,
    /// Cells that can never be a source under the relationships this plan considered.
    /// See [`forced_output_cells`].
    pub(crate) forced_outputs: HashSet<CellId>,
    /// Active relationships with exactly one alive method after the forced-output
    /// fixpoint (see [`forced_output_cells`]) — the planner has no alternative method
    /// to choose for these, regardless of cell strength.
    pub(crate) forced_relationships: HashSet<RelationshipId>,
}

/// Assigns one method per active relationship and returns them in dependency order.
///
/// Only relationships in `active` are planned; relationships outside `active` are
/// invisible to method selection.
///
/// # Errors
///
/// - `Error::Conflict` — no valid, acyclic method assignment exists for `active`.
///
/// - Complexity: O(C · R² · M · K) where C = cells, R = active relationships, M =
///   methods per relationship, K = cells per method — [`release::resolve`] attempts up
///   to C full re-solves, each up to O(R² · M · K) in the worst case.
pub(crate) fn plan(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> Result<Plan, Error> {
    let (forced_outputs, alive) = forced_output_cells(relationships, active);

    let assignment = release::resolve(cells, relationships, active).ok_or(Error::Conflict)?;

    let mut adj = build_digraph(&assignment, relationships);
    // Ensure every active relationship appears as a node even if its chosen method has
    // zero pure outputs and zero plain inputs (fully self-referencing, e.g. a -> a):
    // such a relationship contributes no edges via build_digraph and would otherwise be
    // silently missing from the topological order below.
    for &rel_id in active {
        adj.entry(Node::Relationship(rel_id)).or_default();
    }

    let mut components = scc::tarjan_scc(&adj);
    components.reverse();

    let mut execution_order: Vec<(RelationshipId, usize)> = Vec::new();
    for component in components {
        debug_assert_eq!(
            component.len(),
            1,
            "release::resolve guarantees an acyclic digraph"
        );
        if let Node::Relationship(rel_id) = component[0] {
            execution_order.push((rel_id, assignment.chosen[&rel_id]));
        }
    }

    if execution_order.len() != active.len() {
        return Err(Error::Conflict);
    }

    let forced_relationships: HashSet<RelationshipId> = alive
        .iter()
        .filter(|(_, methods)| methods.iter().filter(|&&is_alive| is_alive).count() == 1)
        .map(|(&rel_id, _)| rel_id)
        .collect();

    Ok(Plan {
        execution_order,
        forced_outputs,
        forced_relationships,
    })
}

/// Computes the cells that can never be a source under `active`, and which methods
/// survive that determination.
///
/// A cell is forced by a relationship when it is a [`pure_outputs`] member of every one
/// of that relationship's currently-alive methods. Starting with all methods alive, this
/// runs to a fixpoint: any method whose pure outputs include a cell forced by a
/// *different* relationship is eliminated (selecting it would always double-write that
/// cell), which can force more cells for the relationships that lost a method. The loop
/// stops once no relationship loses another method.
///
/// The returned `HashMap` gives, for each relationship in `active`, a per-method-index
/// alive flag (`false` for eliminated methods); used only to populate
/// [`Plan::forced_relationships`] and the `forced` half of [`Plan::forced_outputs`] --
/// it does not gate method selection in [`plan`] above, which discovers the same
/// infeasibility structurally.
///
/// - Precondition: every `RelationshipId` in `active` is present in `relationships`.
///
/// - Complexity: O(D · R · M · K²) where D = total methods eliminated across all
///   iterations (bounded by the total method count), R = active relationships,
///   M = methods per relationship, K = cells per method (squared because
///   [`pure_outputs`] scans `inputs` once per output).
fn forced_output_cells(
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> (HashSet<CellId>, HashMap<RelationshipId, Vec<bool>>) {
    let mut alive: HashMap<RelationshipId, Vec<bool>> = active
        .iter()
        .map(|&rel_id| (rel_id, vec![true; relationships[rel_id].methods.len()]))
        .collect();

    loop {
        let mut forced_per_rel: HashMap<RelationshipId, HashSet<CellId>> = HashMap::new();
        for &rel_id in active {
            let rel = &relationships[rel_id];
            let alive_methods = &alive[&rel_id];
            let mut forced: Option<HashSet<CellId>> = None;
            for (idx, method) in rel.methods.iter().enumerate() {
                if !alive_methods[idx] {
                    continue;
                }
                let po = pure_outputs(method);
                forced = Some(match forced {
                    None => po,
                    Some(prev) => prev.intersection(&po).copied().collect(),
                });
            }
            forced_per_rel.insert(rel_id, forced.unwrap_or_default());
        }

        let global_forced: HashSet<CellId> = forced_per_rel.values().flatten().copied().collect();

        let mut changed = false;
        for &rel_id in active {
            let own_forced = &forced_per_rel[&rel_id];
            let rel = &relationships[rel_id];
            let alive_methods = alive.get_mut(&rel_id).expect("seeded for every active id");
            for (idx, method) in rel.methods.iter().enumerate() {
                if alive_methods[idx]
                    && pure_outputs(method)
                        .iter()
                        .any(|c| global_forced.contains(c) && !own_forced.contains(c))
                {
                    alive_methods[idx] = false;
                    changed = true;
                }
            }
        }

        if !changed {
            return (global_forced, alive);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Error, Method, Sheet};
    use std::collections::HashSet;

    // Propagation-behavior tests live in the integration tests.

    #[test]
    fn plan_with_active_subset_ignores_inactive_relationship() {
        // Two independent relationships: R1 (a→b) and R2 (c→d).
        // Plan with only R1 active; R2 must be ignored (not required in output).
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let d = sheet.add_cell(0_i32);

        let r1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let _r2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(c, d, |x: &i32| Ok(*x))])
            .unwrap();

        sheet.write(a, 1_i32).unwrap();

        let mut active = HashSet::new();
        active.insert(r1);

        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();
        assert_eq!(plan.execution_order.len(), 1);
        assert_eq!(plan.execution_order[0].0, r1);
    }

    #[test]
    fn relationship_selected_at_most_once() {
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(0_i32);
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);

        // R1: two methods, both referencing {a, b, c} -- method 0 ignores c, method 1
        // ignores a.
        let r1 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, c], b, |a: &i32, _c: &i32| Ok(*a)),
                Method::from_fn_2_1([a, b], c, |_a: &i32, b: &i32| Ok(*b)),
            ])
            .unwrap();
        // R2: single method c→x
        let r2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(c, x, |v: &i32| Ok(*v))])
            .unwrap();

        // Both relationships must be assigned exactly one method each.
        assert!(sheet.propagate().is_ok());
        assert!(sheet.selected_method(r1).is_some());
        assert!(sheet.selected_method(r2).is_some());
    }

    #[test]
    fn conflict_returns_error() {
        // Two relationships both want to overwrite the same cell; only one method
        // each, and both output the same cell.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let out = sheet.add_cell(0_i32);

        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, out, |x: &i32| Ok(*x))])
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(b, out, |x: &i32| Ok(*x))])
            .unwrap();

        assert!(matches!(sheet.propagate(), Err(Error::Conflict)));
    }

    #[test]
    fn single_method_output_is_forced_and_not_selected_as_source() {
        // b outranks a in strength (added second), but the relationship has only one
        // method (a -> b), so b must never be treated as a source.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 3))])
            .unwrap();

        let active: HashSet<_> = sheet.relationships().collect();
        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();

        assert!(plan.forced_outputs.contains(&b));
        assert!(!plan.forced_outputs.contains(&a));
        assert_eq!(plan.execution_order.len(), 1);
    }

    #[test]
    fn forced_outputs_cascade_through_adjacent_relationship() {
        // R1: a -> b (single method) forces b.
        // R2: b -> c or c -> b (two methods) — once b is forced by R1, R2's c -> b
        // method would double-write b, so it is eliminated, forcing c too.
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

        let active: HashSet<_> = sheet.relationships().collect();
        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();

        assert!(plan.forced_outputs.contains(&b));
        assert!(plan.forced_outputs.contains(&c));
        assert!(!plan.forced_outputs.contains(&a));
        assert_eq!(plan.execution_order.len(), 2);
    }

    #[test]
    fn forced_relationships_true_for_single_method_relationship() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 3))])
            .unwrap();

        let active: HashSet<_> = sheet.relationships().collect();
        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();

        assert!(plan.forced_relationships.contains(&rel));
    }

    #[test]
    fn forced_relationships_excludes_multi_method_relationship() {
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

        let active: HashSet<_> = sheet.relationships().collect();
        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();

        assert!(!plan.forced_relationships.contains(&rel));
    }

    #[test]
    fn forced_relationships_cascade_through_adjacent_relationship() {
        // R1: a -> b (single method) is trivially forced.
        // R2: b -> c or c -> b — c -> b dies once b is forced by R1 (it would
        // double-write b), leaving b -> c as R2's sole alive method, so R2 becomes
        // forced too even though it started with two methods.
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

        let active: HashSet<_> = sheet.relationships().collect();
        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();

        assert!(plan.forced_relationships.contains(&r1));
        assert!(plan.forced_relationships.contains(&r2));
    }

    #[test]
    fn dead_method_not_selected_before_owning_relationship() {
        // R_A: p -> b (single method, forces b).
        // R_B: q -> c (single method, forces c).
        // R2: three methods, all referencing {x, y, b, c, d} -- M0 (produces b from x,
        // ignoring y/c/d) and M1 (produces c from y, ignoring x/b/d) are dead, since b
        // and c are each forced by a *different* relationship; M2 (produces d from
        // b + c, ignoring x/y) is the sole survivor. x's strength is bumped above every
        // other cell's, so if the flood-fill doesn't know M0 is dead, it selects M0
        // (using x) before R_A ever runs, permanently determining b via the wrong
        // relationship and leaving R_A's real method ineligible — a spurious conflict on
        // an otherwise solvable sheet.
        let mut sheet = Sheet::new();
        let p = sheet.add_cell(2_i32);
        let x = sheet.add_cell(0_i32);
        let q = sheet.add_cell(3_i32);
        let y = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let d = sheet.add_cell(0_i32);
        let i32_type = std::any::TypeId::of::<i32>();

        sheet
            .add_relationship(vec![Method::from_fn_1_1(p, b, |v: &i32| Ok(*v))])
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(q, c, |v: &i32| Ok(*v))])
            .unwrap();
        sheet
            .add_relationship(vec![
                Method::new(
                    vec![x, y, c, d],
                    vec![b],
                    vec![i32_type, i32_type, i32_type, i32_type],
                    vec![i32_type],
                    |args| Ok(vec![Box::new(*args[0].downcast_ref::<i32>().unwrap())]),
                ),
                Method::new(
                    vec![y, x, b, d],
                    vec![c],
                    vec![i32_type, i32_type, i32_type, i32_type],
                    vec![i32_type],
                    |args| Ok(vec![Box::new(*args[0].downcast_ref::<i32>().unwrap())]),
                ),
                Method::new(
                    vec![b, c, x, y],
                    vec![d],
                    vec![i32_type, i32_type, i32_type, i32_type],
                    vec![i32_type],
                    |args| {
                        let bb = args[0].downcast_ref::<i32>().unwrap();
                        let cc = args[1].downcast_ref::<i32>().unwrap();
                        Ok(vec![Box::new(bb + cc)])
                    },
                ),
            ])
            .unwrap();

        // Bump x's strength above every other cell so it is chosen as a source first.
        sheet.write(x, 10_i32).unwrap();

        assert!(sheet.propagate().is_ok());
        assert_eq!(*sheet.read::<i32>(d).unwrap(), 5); // p(2) + q(3)
    }
}
