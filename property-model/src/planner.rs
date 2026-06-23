//! Planning pass: selects one method per relationship and returns them in dependency order.
//!
//! Implements the Adam algorithm: cells are visited in descending strength (write-recency)
//! order. The first time a cell is visited it becomes a *source* — its current value is
//! taken as given. After each cell is determined (either as a source or as the output of
//! a selected method), the planner flood-fills through adjacent relationships: any
//! relationship whose method has all inputs determined and all outputs still undetermined
//! is selected and its outputs are enqueued. In a properly formed multi-way constraint,
//! at most one method per relationship can be eligible at any given point (the inputs of
//! each method are the outputs of the other methods), so selection is deterministic.
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
    relationship::{RelationshipData, RelationshipId},
};

/// The output of the planning pass.
pub(crate) struct Plan {
    /// Selected `(RelationshipId, method_index)` pairs in execution order.
    pub(crate) execution_order: Vec<(RelationshipId, usize)>,
}

/// Assigns one method per relationship and returns them in dependency order.
///
/// # Errors
///
/// - `Error::Conflict` — not every relationship could be assigned a method.
///
/// - Complexity: O(C log C + R·M·K) where C = cells, R = relationships,
///   M = methods per relationship, K = cells per method.
pub(crate) fn plan(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
) -> Result<Plan, Error> {
    let mut determined: HashSet<CellId> = HashSet::new();
    let mut selected: Vec<(RelationshipId, usize)> = Vec::new();
    let mut selected_set: HashSet<RelationshipId> = HashSet::new();

    let mut cells_sorted: Vec<CellId> = cells.keys().collect();
    cells_sorted.sort_by_key(|&id| Reverse(cells[id].strength));

    for &source in &cells_sorted {
        if determined.contains(&source) {
            continue;
        }
        determined.insert(source);

        let mut queue: VecDeque<CellId> = VecDeque::new();
        queue.push_back(source);

        while let Some(cell) = queue.pop_front() {
            for &rel_id in &cells[cell].adj {
                if selected_set.contains(&rel_id) {
                    continue;
                }
                let rel = &relationships[rel_id];
                if let Some((method_idx, method)) = rel.methods.iter().enumerate().find(|(_, m)| {
                    m.inputs.iter().all(|i| determined.contains(i))
                        && m.outputs.iter().all(|o| !determined.contains(o))
                }) {
                    debug_assert!(
                        !rel.methods[method_idx + 1..].iter().any(|m| {
                            m.inputs.iter().all(|i| determined.contains(i))
                                && m.outputs.iter().all(|o| !determined.contains(o))
                        }),
                        "invariant violated: multiple eligible methods for one relationship"
                    );
                    for &output in &method.outputs {
                        determined.insert(output);
                        queue.push_back(output);
                    }
                    selected_set.insert(rel_id);
                    selected.push((rel_id, method_idx));
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
