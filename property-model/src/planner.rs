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
//! Because a method is only selected once all its inputs are determined, any method that
//! writes an input to a later method necessarily appears earlier in the selection order.
//! The selection order is therefore already a valid topological execution order.

use std::cmp::Reverse;
use std::collections::{HashSet, VecDeque};

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
}

/// Assigns one method per relationship and returns them in dependency order.
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
/// - `Error::Conflict` — not every relationship could be assigned a method.
///
/// - Complexity: O(C log C + R·M·K²) where C = cells, R = relationships,
///   M = methods per relationship, K = cells per method.
pub(crate) fn plan(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
) -> Result<Plan, Error> {
    let mut determined: HashSet<CellId> = HashSet::new();
    // Subset of `determined`: cells whose value came from write(), not from a selected method.
    // Only source cells may serve as self-referencing inputs.
    let mut source_cells: HashSet<CellId> = HashSet::new();
    let mut pre_claimed: HashSet<CellId> = HashSet::new();
    let mut selected: Vec<(RelationshipId, usize)> = Vec::new();
    let mut selected_set: HashSet<RelationshipId> = HashSet::new();

    let mut cells_sorted: Vec<CellId> = cells.keys().collect();
    cells_sorted.sort_by_key(|&id| Reverse(cells[id].strength));

    for &source in &cells_sorted {
        if determined.contains(&source) || pre_claimed.contains(&source) {
            continue;
        }
        determined.insert(source);
        source_cells.insert(source);

        let mut queue: VecDeque<CellId> = VecDeque::new();
        queue.push_back(source);

        while let Some(cell) = queue.pop_front() {
            for &rel_id in &cells[cell].adj {
                if selected_set.contains(&rel_id) {
                    continue;
                }
                let rel = &relationships[rel_id];

                // A method is eligible when:
                //   pure inputs  (inputs ∖ outputs): all in `determined`
                //   self-ref     (inputs ∩ outputs): all in `source_cells`
                //   pure outputs (outputs ∖ inputs): none in `determined`
                let is_eligible = |m: &Method| {
                    m.inputs
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
                    .find(|(_, m)| {
                        is_eligible(m) && m.outputs.contains(&cell) && m.inputs.contains(&cell)
                    })
                    .or_else(|| rel.methods.iter().enumerate().find(|(_, m)| is_eligible(m)));

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
                    // feasible — meaning no other method can run without overwriting a cell
                    // that is already determined or pre-claimed — and the current cell is
                    // one of its inputs, pre-claim its pure outputs so the flood-fill
                    // propagates further.
                    //
                    // A self-referencing output that is a source is feasible (the method
                    // may overwrite its own source cell). A self-referencing output that
                    // was derived by another method (in `determined` but not `source_cells`)
                    // is infeasible. Pure outputs must not be determined or pre-claimed.
                    //
                    // Only pure outputs are pre-claimed; self-referencing outputs are
                    // already committed as sources and do not need pre-claiming.
                    let is_feasible = |m: &Method| {
                        m.outputs.iter().all(|o| {
                            if m.inputs.contains(o) {
                                !pre_claimed.contains(o)
                                    && (!determined.contains(o) || source_cells.contains(o))
                            } else {
                                !determined.contains(o) && !pre_claimed.contains(o)
                            }
                        })
                    };

                    let mut feasible = rel.methods.iter().filter(|m| is_feasible(m));
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

    if selected.len() != relationships.len() {
        return Err(Error::Conflict);
    }

    Ok(Plan {
        execution_order: selected,
    })
}

#[cfg(test)]
mod tests {
    use crate::{Error, Method, Sheet};

    // Propagation-behavior tests live in the integration tests.

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
}
