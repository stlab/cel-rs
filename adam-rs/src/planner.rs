//! Planning pass: selects one method per relationship and returns them in dependency order.
//!
//! Implements the Adam algorithm: cells are visited in descending strength
//! (write-recency) order. The first time a cell is visited it becomes a *source* — its
//! current value is taken as given. A relationship's methods are *candidates* for
//! selection; whenever a cell becomes determined (as a source, or as the output of some
//! other relationship's already-selected method), every relationship adjacent to that
//! cell eliminates any candidate whose `outputs` set contains it. The instant a
//! relationship's candidates narrow to exactly one, that method is selected and each of
//! its output cells becomes determined too, cascading the same elimination outward. A
//! relationship whose candidates narrow to zero cannot be assigned — see
//! [`Error::Conflict`].
//!
//! Because every method's `outputs` set is unique within its relationship (enforced by
//! [`crate::sheet::Sheet::add_relationship`]), this single mechanism handles
//! self-referencing methods (a cell in both a method's `inputs` and `outputs`) without
//! special-casing: whichever of two candidate cells resolves first eliminates exactly
//! the method that would have produced it, leaving the other as sole survivor.
//!
//! **Structurally forced cells**: some cells are guaranteed to be produced by a method
//! regardless of cell strength — e.g. the sole output of a single-method relationship.
//! [`forced_output_cells`] computes this set as a fixpoint over all active
//! relationships, independent of any specific run's cell strengths (needed because
//! [`crate::sheet::Sheet::is_forced`] must answer "can this cell ever meaningfully be
//! written?" regardless of what a caller might write). Relationships already narrowed to
//! one candidate by this fixpoint are selected immediately, before any cell is chosen as
//! a fresh source; every other structurally forced cell is excluded from source
//! candidacy in the strength-ordered pass.
//!
//! **Execution order**: because a relationship can be selected before its inputs are
//! actually resolved (a structurally forced single-method relationship is selected
//! immediately, regardless of when its input arrives), the order relationships are
//! *selected* in is not necessarily a valid execution order. [`topological_order`]
//! computes one separately from the final selection, and reports [`Error::Cycle`] if the
//! selected methods have no valid order (e.g. two single-method relationships that each
//! require the other's output).

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet, VecDeque};

use slotmap::SlotMap;

use crate::{
    cell::{CellData, CellId},
    error::Error,
    relationship::{Method, RelationshipData, RelationshipId},
};

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
/// invisible to the elimination process. The conflict check counts against
/// `active.len()`.
///
/// A method may have cells in both `inputs` and `outputs` (self-referencing); see the
/// module documentation for why this needs no special handling here. Such a cell is
/// read at its pre-execution value and overwritten with the result.
///
/// # Errors
///
/// - `Error::Conflict` — some active relationship's candidates narrowed to zero, or not
///   every active relationship could be assigned a method.
/// - `Error::Cycle` — the selected methods have no valid execution order.
///
/// - Complexity: O(D · R · M · K²) for [`forced_output_cells`] (D = methods eliminated
///   across its fixpoint), plus O(C log C) to sort cells by strength, plus O(R·M·K²) for
///   the elimination pass (each cell triggers one elimination scan per adjacent
///   relationship), plus O(R·K) for the final topological sort. C = cells, R = active
///   relationships, M = methods per relationship, K = cells per method.
pub(crate) fn plan(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> Result<Plan, Error> {
    let (forced_outputs, structural_alive) = forced_output_cells(relationships, active);
    let forced_relationships: HashSet<RelationshipId> = structural_alive
        .iter()
        .filter(|(_, methods)| methods.iter().filter(|&&is_alive| is_alive).count() == 1)
        .map(|(&rel_id, _)| rel_id)
        .collect();

    let mut candidates = structural_alive;
    let mut determined: HashSet<CellId> = HashSet::new();
    let mut selected: HashMap<RelationshipId, usize> = HashMap::new();
    let mut queue: VecDeque<CellId> = VecDeque::new();

    // Bootstrap: relationships already down to one candidate — either genuinely
    // single-method, or narrowed by the forced-output fixpoint above — are selected
    // immediately, before any cell is chosen as a fresh source.
    for &rel_id in active {
        select_if_sole_candidate(
            rel_id,
            relationships,
            &mut candidates,
            &mut determined,
            &mut selected,
            &mut queue,
        )?;
        drain(
            &mut queue,
            cells,
            relationships,
            active,
            &mut candidates,
            &mut determined,
            &mut selected,
        )?;
    }

    // Strength-ordered seeding: the highest-strength cell not already determined or
    // structurally forced becomes a fresh source.
    let mut cells_sorted: Vec<CellId> = cells.keys().collect();
    cells_sorted.sort_by_key(|&id| Reverse(cells[id].strength));
    for &cell in &cells_sorted {
        if determined.contains(&cell) || forced_outputs.contains(&cell) {
            continue;
        }
        determined.insert(cell);
        queue.push_back(cell);
        drain(
            &mut queue,
            cells,
            relationships,
            active,
            &mut candidates,
            &mut determined,
            &mut selected,
        )?;
    }

    if selected.len() != active.len() {
        return Err(Error::Conflict);
    }

    let execution_order = topological_order(relationships, &selected)?;

    Ok(Plan {
        execution_order,
        forced_outputs,
        forced_relationships,
    })
}

/// Selects `rel_id`'s sole surviving candidate method, if exactly one remains.
///
/// Does nothing if `rel_id` is already selected or if more than one candidate remains.
/// Used both to bootstrap relationships that start with (or are narrowed by
/// [`forced_output_cells`] to) a single candidate, and after each candidate
/// elimination in [`drain`].
///
/// - Postcondition: if selected, each output cell not already in `determined` is
///   inserted into `determined` and pushed onto `queue` for cascading elimination.
///
/// # Errors
///
/// - `Error::Conflict` — `rel_id` has zero surviving candidates.
///
/// - Complexity: O(M), where M is the number of methods in the relationship.
fn select_if_sole_candidate(
    rel_id: RelationshipId,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    candidates: &mut HashMap<RelationshipId, Vec<bool>>,
    determined: &mut HashSet<CellId>,
    selected: &mut HashMap<RelationshipId, usize>,
    queue: &mut VecDeque<CellId>,
) -> Result<(), Error> {
    if selected.contains_key(&rel_id) {
        return Ok(());
    }
    let alive = &candidates[&rel_id];
    let mut survivors = alive.iter().enumerate().filter(|&(_, &is_alive)| is_alive);
    match (survivors.next(), survivors.next()) {
        (None, _) => Err(Error::Conflict),
        (Some(_), Some(_)) => Ok(()),
        (Some((idx, _)), None) => {
            selected.insert(rel_id, idx);
            for &output in &relationships[rel_id].methods[idx].outputs {
                if determined.insert(output) {
                    queue.push_back(output);
                }
            }
            Ok(())
        }
    }
}

/// Processes `queue`, eliminating candidates in every relationship adjacent to each
/// dequeued cell and selecting any relationship whose candidates narrow to one.
///
/// - Precondition: every cell in `queue` is already present in `determined`.
///
/// # Errors
///
/// - `Error::Conflict` — some relationship's candidates narrow to zero.
///
/// - Complexity: O(Q·R·M·K), where Q is the queue size, R is the number of
///   relationships adjacent to each cell, M is the number of methods per
///   relationship, and K is the number of cells per method.
fn drain(
    queue: &mut VecDeque<CellId>,
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
    candidates: &mut HashMap<RelationshipId, Vec<bool>>,
    determined: &mut HashSet<CellId>,
    selected: &mut HashMap<RelationshipId, usize>,
) -> Result<(), Error> {
    while let Some(cell) = queue.pop_front() {
        for &rel_id in &cells[cell].adj {
            if !active.contains(&rel_id) || selected.contains_key(&rel_id) {
                continue;
            }
            {
                let alive = candidates
                    .get_mut(&rel_id)
                    .expect("seeded for every active id");
                for (idx, method) in relationships[rel_id].methods.iter().enumerate() {
                    if alive[idx] && method.outputs.contains(&cell) {
                        alive[idx] = false;
                    }
                }
            }
            select_if_sole_candidate(
                rel_id,
                relationships,
                candidates,
                determined,
                selected,
                queue,
            )?;
        }
    }
    Ok(())
}

/// Orders `selected` so each method appears after the methods producing its pure
/// inputs (inputs not also present in its own outputs).
///
/// - Precondition: every cell produced by an entry of `selected` is produced by at
///   most one entry — guaranteed by the elimination in [`plan`], since a cell is
///   inserted into `determined` (and thus becomes an output of exactly one selected
///   method) at most once.
///
/// # Errors
///
/// - `Error::Cycle` — the dependency graph over `selected` contains a cycle.
///
/// - Complexity: O(R·K) where R = `selected.len()` and K = max cells per method.
fn topological_order(
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    selected: &HashMap<RelationshipId, usize>,
) -> Result<Vec<(RelationshipId, usize)>, Error> {
    let producer: HashMap<CellId, RelationshipId> = selected
        .iter()
        .flat_map(|(&rel_id, &idx)| {
            relationships[rel_id].methods[idx]
                .outputs
                .iter()
                .map(move |&output| (output, rel_id))
        })
        .collect();

    let mut dependents: HashMap<RelationshipId, Vec<RelationshipId>> = selected
        .keys()
        .map(|&rel_id| (rel_id, Vec::new()))
        .collect();
    let mut in_degree: HashMap<RelationshipId, usize> =
        selected.keys().map(|&rel_id| (rel_id, 0)).collect();

    for (&rel_id, &idx) in selected {
        let method = &relationships[rel_id].methods[idx];
        for input in method.inputs.iter().filter(|i| !method.outputs.contains(i)) {
            if let Some(&producer_rel) = producer.get(input) {
                dependents
                    .get_mut(&producer_rel)
                    .expect("seeded for every relationship in `selected`")
                    .push(rel_id);
                *in_degree
                    .get_mut(&rel_id)
                    .expect("seeded for every relationship in `selected`") += 1;
            }
        }
    }

    let mut ready: VecDeque<RelationshipId> = in_degree
        .iter()
        .filter(|&(_, &degree)| degree == 0)
        .map(|(&rel_id, _)| rel_id)
        .collect();
    let mut order = Vec::with_capacity(selected.len());
    while let Some(rel_id) = ready.pop_front() {
        order.push((rel_id, selected[&rel_id]));
        for &dependent in &dependents[&rel_id] {
            let degree = in_degree.get_mut(&dependent).expect("seeded above");
            *degree -= 1;
            if *degree == 0 {
                ready.push_back(dependent);
            }
        }
    }

    if order.len() != selected.len() {
        return Err(Error::Cycle);
    }
    Ok(order)
}

/// Returns the cells `method` writes but does not read.
///
/// Self-referencing cells (present in both `inputs` and `outputs`) are excluded: they
/// are read at their pre-execution value, so they retain their ordinary role as
/// potential sources.
///
/// - Complexity: O(K²) where K = cells per method (`inputs.contains` scans linearly).
fn pure_outputs(method: &Method) -> HashSet<CellId> {
    method
        .outputs
        .iter()
        .filter(|o| !method.inputs.contains(o))
        .copied()
        .collect()
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
/// alive flag (`false` for eliminated methods); the caller must exclude dead methods
/// from selection entirely, not just their cells from source candidacy — a dead method's
/// pure output can still be undetermined at the moment the flood-fill considers it, so
/// the ordinary "output not yet determined" eligibility check alone cannot rule it out.
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
    fn relationship_selects_exactly_one_method_when_multiple_are_eligible() {
        // R1 has two self-referencing methods over {a, b} (min→a, max→b). Once both
        // a and b are written, both methods become simultaneously eligible — the
        // selection logic must still choose exactly one, so R1 contributes exactly
        // one entry to the plan and R2 (which depends on the chosen output) is still
        // assigned correctly.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(10_i32);
        let b = sheet.add_cell(5_i32);
        let c = sheet.add_cell(0_i32);

        // R1: two self-referencing methods — min and max.
        // Both methods reference {a, b} and output to one of {a, b}.
        // When both a and b are sources, both methods are eligible (all self-ref inputs in source_cells,
        // no pure outputs in determined).
        sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| Ok(*x.min(y))),
                Method::from_fn_2_1([a, b], b, |x: &i32, y: &i32| Ok(*x.max(y))),
            ])
            .unwrap();
        // R2: depends on a. Verifies that exactly one method of R1 is selected.
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, c, |x: &i32| Ok(*x))])
            .unwrap();

        assert!(sheet.propagate().is_ok());
        // Verify one method was selected: a is now either 5 or 10 (one of the two eligible methods).
        let a_val = *sheet.read::<i32>(a).unwrap();
        assert!(a_val == 5 || a_val == 10);
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
    fn execution_order_respects_producer_consumer_dependency() {
        // r_bc (b -> c) is added to the sheet *before* r_ab (a -> b). Both are
        // single-method (structurally forced) relationships, so both are selected
        // during the bootstrap loop — over a `HashSet`, whose iteration order need
        // not match insertion order. If `topological_order` were broken and just
        // returned selection order unchanged, this insertion order (consumer before
        // producer) is exactly the arrangement that would surface the bug: c's
        // producer (r_bc) reads b, which is only produced by r_ab.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(1_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);

        let r_bc = sheet
            .add_relationship(vec![Method::from_fn_1_1(b, c, |x: &i32| Ok(*x + 1))])
            .unwrap();
        let r_ab = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();

        let mut active = HashSet::new();
        active.insert(r_bc);
        active.insert(r_ab);

        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();

        let position_of = |rel_id| {
            plan.execution_order
                .iter()
                .position(|&(id, _)| id == rel_id)
                .expect("relationship must appear in execution_order")
        };

        // r_ab produces b; r_bc consumes b to produce c. The producer of b must
        // come before the producer of c, regardless of selection/insertion order.
        assert!(position_of(r_ab) < position_of(r_bc));
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
}
