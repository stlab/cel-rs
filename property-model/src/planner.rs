//! Planning pass: selects one method per relationship and returns them in dependency order.
//!
//! Implements the Adam algorithm: cells are visited in descending strength (write-recency)
//! order. The first time a cell is visited it becomes a *source* — its current value is
//! taken as given. After each cell is determined (either as a source or as the output of
//! a selected method), the planner flood-fills through adjacent relationships: any
//! relationship whose method has all inputs determined and all outputs still undetermined
//! is selected and its outputs are enqueued. In a standard (non-self-referencing)
//! multi-way constraint, at most one method per relationship can be eligible at any given
//! point (the inputs of each method are the outputs of the other methods). Self-referencing
//! methods — where a cell appears in both inputs and outputs — can have multiple eligible
//! methods simultaneously when all participating cells are sources; disambiguation is
//! described in [`plan`].
//!
//! **Pre-claiming**: when a determined cell eliminates all but one feasible method for a
//! relationship (a method is infeasible if it would overwrite a determined or pre-claimed
//! cell), that sole method's outputs are *pre-claimed*: they can never become sources, even
//! before all of the method's inputs are determined. Excluding pre-claimed outputs from
//! feasibility prevents a method whose output is pre-claimed by the current flood-fill pass
//! from being counted as a second viable option. This allows the planner to correctly handle
//! constraints where the highest-strength cell is one of several inputs to the selected
//! method.
//!
//! **Forced outputs**: a cell that is a pure output — present in a method's `outputs`
//! but not its `inputs` — in every currently-viable method of some relationship can
//! never be a source, regardless of strength: that relationship has no alternative but
//! to produce it. [`forced_output_cells`] computes this as a fixpoint over all active
//! relationships (eliminating a method whose pure output is guaranteed to be produced by
//! a *different* relationship can force further cells), and the result is excluded from
//! source candidacy before the strength-ordered pass below begins. The methods eliminated
//! along the way are dead: the flood-fill below must also refuse to select them, even
//! when their own pure output happens to still be undetermined at the time they're
//! considered — otherwise a higher-strength cell reachable only through a dead method can
//! be flood-filled first, permanently claiming a cell the *other* relationship was meant
//! to produce and turning an otherwise solvable sheet into a spurious conflict.
//!
//! Because a method is only selected once all its inputs are determined, any method that
//! writes an input to a later method necessarily appears earlier in the selection order.
//! The selection order is therefore already a valid topological execution order.

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
}

/// Assigns one method per active relationship and returns them in dependency order.
///
/// Only relationships in `active` are planned; relationships outside `active` are
/// invisible to the flood-fill. The conflict check counts against `active.len()`.
///
/// A method may have cells in both `inputs` and `outputs` (self-referencing). Such a cell is
/// read at its pre-execution value and overwritten with the result. A self-referencing method
/// is only selected when every self-referencing cell is a *source* — determined by the outer
/// flood-fill pass, not as the output of another method.
///
/// When two methods in one relationship are simultaneously eligible (possible when all
/// participating cells are sources), the method whose self-referencing output matches the cell
/// currently being processed from the queue is preferred.
///
/// # Errors
///
/// - `Error::Conflict` — not every active relationship could be assigned a method.
///
/// - Complexity: O(C log C + R·M·K² + D·R·M·K²) where C = cells, R = active
///   relationships, M = methods per relationship, K = cells per method, and
///   D = methods eliminated while computing [`forced_output_cells`] (bounded by the
///   total method count).
pub(crate) fn plan(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> Result<Plan, Error> {
    let mut determined: HashSet<CellId> = HashSet::new();
    // Subset of `determined`: cells whose value came from write(), not from a selected method.
    // Only source cells may serve as self-referencing inputs.
    let mut source_cells: HashSet<CellId> = HashSet::new();
    let mut pre_claimed: HashSet<CellId> = HashSet::new();
    let mut selected: Vec<(RelationshipId, usize)> = Vec::new();
    let mut selected_set: HashSet<RelationshipId> = HashSet::new();

    // Cells that some active relationship's method structure guarantees will always
    // be produced by a method, regardless of strength. These can never be a source.
    // `alive` marks, per relationship, which methods survive the fixpoint: a method is
    // dead when it would double-write a cell a *different* relationship forces: such a
    // method must never be selected by the flood-fill below, regardless of timing.
    let (forced_outputs, alive) = forced_output_cells(relationships, active);

    let mut cells_sorted: Vec<CellId> = cells.keys().collect();
    cells_sorted.sort_by_key(|&id| Reverse(cells[id].strength));

    for &source in &cells_sorted {
        if determined.contains(&source)
            || pre_claimed.contains(&source)
            || forced_outputs.contains(&source)
        {
            continue;
        }
        determined.insert(source);
        source_cells.insert(source);

        let mut queue: VecDeque<CellId> = VecDeque::new();
        queue.push_back(source);

        while let Some(cell) = queue.pop_front() {
            for &rel_id in &cells[cell].adj {
                if !active.contains(&rel_id) {
                    continue;
                }
                if selected_set.contains(&rel_id) {
                    continue;
                }
                let rel = &relationships[rel_id];
                let rel_alive = &alive[&rel_id];

                // A method is eligible when:
                //   alive        : it survived the forced-output fixpoint (see `alive`)
                //   pure inputs  (inputs ∖ outputs): all in `determined`
                //   self-ref     (inputs ∩ outputs): all in `source_cells`
                //   pure outputs (outputs ∖ inputs): none in `determined`
                let is_eligible = |idx: usize, m: &Method| {
                    rel_alive[idx]
                        && m.inputs
                            .iter()
                            .filter(|i| !m.outputs.contains(i))
                            .all(|i| determined.contains(i))
                        && m.inputs
                            .iter()
                            .filter(|i| m.outputs.contains(i))
                            .all(|i| source_cells.contains(i))
                        && m.outputs
                            .iter()
                            .filter(|o| !m.inputs.contains(o))
                            .all(|o| !determined.contains(o))
                };

                // Prefer the eligible method whose self-referencing output is `cell`
                // (the cell currently being processed). This resolves ties when two
                // methods are simultaneously eligible: `cell` is the weakest source
                // processed so far, so the method that adjusts it should be chosen.
                // Fall back to the first eligible method for non-self-referencing cases.
                let chosen = rel
                    .methods
                    .iter()
                    .enumerate()
                    .find(|(idx, m)| {
                        is_eligible(*idx, m)
                            && m.outputs.contains(&cell)
                            && m.inputs.contains(&cell)
                    })
                    .or_else(|| {
                        rel.methods
                            .iter()
                            .enumerate()
                            .find(|(idx, m)| is_eligible(*idx, m))
                    });

                if let Some((method_idx, method)) = chosen {
                    for &output in &method.outputs {
                        // Guard: self-referencing outputs are already in `determined`
                        // (as sources); only re-queue cells that are newly determined.
                        let newly_determined = determined.insert(output);
                        pre_claimed.remove(&output);
                        // A method's output is no longer a source: remove it so that
                        // subsequent self-referencing eligibility checks cannot treat
                        // a method-derived value as a source value.
                        source_cells.remove(&output);
                        if newly_determined {
                            queue.push_back(output);
                        }
                    }
                    selected_set.insert(rel_id);
                    selected.push((rel_id, method_idx));
                } else {
                    // No method is immediately selectable. If exactly one method remains
                    // feasible — meaning no other alive method can run without overwriting a
                    // cell that is already determined or pre-claimed — and the current cell
                    // is one of its inputs, pre-claim its pure outputs so the flood-fill
                    // propagates further. Dead methods are never feasible: they must not be
                    // pre-claimed for, nor counted as competing alternatives to, a live method.
                    //
                    // A self-referencing output that is a source is feasible (the method
                    // may overwrite its own source cell). A self-referencing output that
                    // was derived by another method (in `determined` but not `source_cells`)
                    // is infeasible. Pure outputs must not be determined or pre-claimed.
                    //
                    // Only pure outputs are pre-claimed; self-referencing outputs are
                    // already committed as sources and do not need pre-claiming.
                    let is_feasible = |idx: usize, m: &Method| {
                        rel_alive[idx]
                            && m.outputs.iter().all(|o| {
                                if m.inputs.contains(o) {
                                    !pre_claimed.contains(o)
                                        && (!determined.contains(o) || source_cells.contains(o))
                                } else {
                                    !determined.contains(o) && !pre_claimed.contains(o)
                                }
                            })
                    };

                    let mut feasible = rel
                        .methods
                        .iter()
                        .enumerate()
                        .filter(|(idx, m)| is_feasible(*idx, m))
                        .map(|(_, m)| m);
                    let first = feasible.next();
                    let second = feasible.next();
                    if let (Some(sole), None) = (first, second)
                        && sole.inputs.contains(&cell)
                    {
                        for &output in &sole.outputs {
                            if !sole.inputs.contains(&output) && pre_claimed.insert(output) {
                                queue.push_back(output);
                            }
                        }
                    }
                }
            }
        }
    }

    if selected.len() != active.len() {
        return Err(Error::Conflict);
    }

    Ok(Plan {
        execution_order: selected,
        forced_outputs,
    })
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
    fn relationship_selected_at_most_once() {
        // x is inserted first so it sorts before a (equal strength, stable sort).
        // Without selected_set, flood-fill selects R1 twice (Method 0 then Method 1),
        // x becomes a source before R2 can run, and propagate() falsely returns Ok.
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(0_i32);
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);

        // R1: two chained methods — Method 0: a→b, Method 1: b→c
        sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, b, |v: &i32| Ok(*v)),
                Method::from_fn_1_1(b, c, |v: &i32| Ok(*v)),
            ])
            .unwrap();
        // R2: single method c→x
        sheet
            .add_relationship(vec![Method::from_fn_1_1(c, x, |v: &i32| Ok(*v))])
            .unwrap();

        // Both relationships must be assigned exactly one method; if R1 were selected
        // twice the count check would pass and R2 would silently be skipped.
        assert!(sheet.propagate().is_err());
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
    fn dead_method_not_selected_before_owning_relationship() {
        // R_A: p -> b (single method, forces b).
        // R_B: q -> c (single method, forces c).
        // R2: three methods — M0 (x -> b) and M1 (y -> c) are dead, since b and c are
        // each forced by a *different* relationship; M2 ([b, c] -> d) is the sole
        // survivor. x's strength is bumped above every other cell's, so if the
        // flood-fill doesn't know M0 is dead, it selects M0 (using x) before R_A ever
        // runs, permanently determining b via the wrong relationship and leaving R_A's
        // real method ineligible — a spurious conflict on an otherwise solvable sheet.
        let mut sheet = Sheet::new();
        let p = sheet.add_cell(2_i32);
        let x = sheet.add_cell(0_i32);
        let q = sheet.add_cell(3_i32);
        let y = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let d = sheet.add_cell(0_i32);

        sheet
            .add_relationship(vec![Method::from_fn_1_1(p, b, |v: &i32| Ok(*v))])
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(q, c, |v: &i32| Ok(*v))])
            .unwrap();
        sheet
            .add_relationship(vec![
                Method::from_fn_1_1(x, b, |v: &i32| Ok(*v)),
                Method::from_fn_1_1(y, c, |v: &i32| Ok(*v)),
                Method::from_fn_2_1([b, c], d, |bb: &i32, cc: &i32| Ok(*bb + *cc)),
            ])
            .unwrap();

        // Bump x's strength above every other cell so it is chosen as a source first.
        sheet.write(x, 10_i32).unwrap();

        assert!(sheet.propagate().is_ok());
        assert_eq!(*sheet.read::<i32>(d).unwrap(), 5); // p(2) + q(3)
    }
}
