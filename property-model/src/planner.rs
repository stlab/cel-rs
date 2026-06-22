//! Planning pass: selects one method per relationship and topologically orders execution.
//!
//! Phase 1 greedily assigns a method to each relationship by minimising the minimum
//! strength (write-recency clock value) of the method's output cells — preferring to
//! derive cells that were written least recently. Phase 2 runs Kahn's algorithm to
//! produce a dependency-ordered execution sequence.
//!
//! The greedy Phase 1 selection is per-relationship with no global optimality guarantee;
//! it minimises the minimum output-cell strength locally but does not guarantee that the
//! globally strongest cell is preserved across the whole sheet. A future model checker
//! will verify solvability.

use std::collections::{HashMap, HashSet, VecDeque};

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

/// Runs Phase 1 (greedy method selection) and Phase 2 (topological sort).
///
/// Phase 1 iterates `relationship_order` and, for each relationship, selects
/// the method whose output cells have the minimum write-strength — preferring
/// to derive cells that were written least recently. A method is invalid if
/// any of its output cells were already claimed by an earlier relationship.
///
/// Phase 2 runs Kahn's algorithm over the selected methods, where an edge
/// A→B means method A writes a cell that method B reads as input.
///
/// # Errors
///
/// - `Error::Conflict` — no valid method exists for some relationship.
/// - `Error::Cycle` — the selected methods form a dependency cycle.
///
/// - Complexity: O(R·M + N) where R = relationships, M = methods per
///   relationship, N = total cells across all selected methods.
pub(crate) fn plan(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    relationship_order: &[RelationshipId],
) -> Result<Plan, Error> {
    // ── Phase 1: greedy method selection ────────────────────────────────────
    let mut claimed: HashSet<CellId> = HashSet::new();
    let mut selected: Vec<(RelationshipId, usize)> = Vec::new();

    for &rel_id in relationship_order {
        let rel = &relationships[rel_id];

        let best = rel
            .methods
            .iter()
            .enumerate()
            .filter(|(_, m)| m.outputs.iter().all(|o| !claimed.contains(o)))
            .min_by_key(|(_, m)| {
                m.outputs
                    .iter()
                    .map(|&id| cells[id].strength)
                    .min()
                    .unwrap_or(0)
            });

        let (method_idx, method) = best.ok_or(Error::Conflict)?;

        for &output in &method.outputs {
            claimed.insert(output);
        }
        selected.push((rel_id, method_idx));
    }

    // ── Phase 2: Kahn's topological sort ────────────────────────────────────
    let n = selected.len();

    // Map each output cell to the index (in `selected`) of the method that produces it.
    let mut producer: HashMap<CellId, usize> = HashMap::new();
    for (i, (rel_id, method_idx)) in selected.iter().enumerate() {
        let method = &relationships[*rel_id].methods[*method_idx];
        for &output in &method.outputs {
            producer.insert(output, i);
        }
    }

    // Adjacency list and in-degree for the execution DAG.
    let mut adj: Vec<Vec<usize>> = vec![vec![]; n];
    let mut in_degree: Vec<usize> = vec![0; n];

    for (i, (rel_id, method_idx)) in selected.iter().enumerate() {
        let method = &relationships[*rel_id].methods[*method_idx];
        for &input in &method.inputs {
            if let Some(&p) = producer.get(&input)
                && p != i
            {
                adj[p].push(i);
                in_degree[i] += 1;
            }
        }
    }

    let mut queue: VecDeque<usize> = in_degree
        .iter()
        .enumerate()
        .filter(|&(_, d)| *d == 0)
        .map(|(i, _)| i)
        .collect();

    let mut order: Vec<usize> = Vec::with_capacity(n);
    while let Some(node) = queue.pop_front() {
        order.push(node);
        for &next in &adj[node] {
            in_degree[next] -= 1;
            if in_degree[next] == 0 {
                queue.push_back(next);
            }
        }
    }

    if order.len() != n {
        return Err(Error::Cycle);
    }

    Ok(Plan {
        execution_order: order.iter().map(|&i| selected[i]).collect(),
    })
}

#[cfg(test)]
mod tests {
    use crate::{Error, Method, Sheet};

    // Propagation-behavior tests (single_method, strength_drives_selection, chained) live in
    // Task 6's integration tests, where the full propagate() implementation is wired.

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
