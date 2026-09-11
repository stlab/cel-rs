//! Cycle and infeasible-set reconstruction for planner error reporting. Runs only on
//! the cold error path, after a plan has already failed.

use std::collections::{HashMap, HashSet};

use slotmap::SlotMap;

use crate::relationship::{RelationshipData, RelationshipId};

use super::digraph::Node;
use super::matching::Assignment;

/// Returns one simple cycle within `component`, as an ordered node list whose consecutive
/// entries (wrapping from last to first) are edges of `adj`.
///
/// - Precondition: `component`'s nodes contain at least one cycle reachable within the set
///   (true for any strongly connected component of size > 1).
/// - Complexity: O(V + E) over the nodes and edges induced by `component`.
pub(crate) fn recover_cycle(adj: &HashMap<Node, Vec<Node>>, component: &[Node]) -> Vec<Node> {
    let members: HashSet<Node> = component.iter().copied().collect();
    let start = match component.first() {
        Some(&n) => n,
        None => return Vec::new(),
    };
    let mut path: Vec<Node> = Vec::new();
    let mut on_path: HashSet<Node> = HashSet::new();
    let mut stack: Vec<(Node, usize)> = vec![(start, 0)];
    while let Some(&mut (node, ref mut next)) = stack.last_mut() {
        if *next == 0 {
            if on_path.contains(&node) {
                // Found the loop: slice from the earlier occurrence.
                let at = path.iter().position(|&n| n == node).unwrap();
                return path[at..].to_vec();
            }
            path.push(node);
            on_path.insert(node);
        }
        let successors = adj.get(&node).map(|v| v.as_slice()).unwrap_or(&[]);
        let mut advanced = false;
        while *next < successors.len() {
            let w = successors[*next];
            *next += 1;
            if !members.contains(&w) {
                continue;
            }
            if on_path.contains(&w) {
                let at = path.iter().position(|&n| n == w).unwrap();
                return path[at..].to_vec();
            }
            stack.push((w, 0));
            advanced = true;
            break;
        }
        if !advanced {
            on_path.remove(&node);
            path.pop();
            stack.pop();
        }
    }
    Vec::new()
}

/// Returns a subset-minimal group of `active` relationships that has no valid method
/// assignment (`Assignment::solve` returns `None`): removing any member of the result makes
/// the remainder feasible.
///
/// - Precondition: `active` itself is infeasible under `Assignment::solve` with no cells
///   forbidden.
/// - Complexity: O(R) calls to `Assignment::solve`, each O(R²·M·K); cold error path only.
pub(crate) fn minimal_infeasible_set(
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> HashSet<RelationshipId> {
    let mut candidate = active.clone();
    // Deletion filtering: drop each relationship whose removal keeps the set infeasible.
    let members: Vec<RelationshipId> = active.iter().copied().collect();
    for r in members {
        let mut without = candidate.clone();
        without.remove(&r);
        if Assignment::solve(relationships, &without, &HashSet::new()).is_none() {
            candidate = without;
        }
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellId;
    use crate::planner::digraph::Node;
    use crate::relationship::RelationshipId;
    use slotmap::SlotMap;
    use std::collections::HashMap;

    #[test]
    fn recover_cycle_returns_a_simple_loop() {
        let mut rmap: SlotMap<RelationshipId, ()> = SlotMap::with_key();
        let r1 = rmap.insert(());
        let r2 = rmap.insert(());
        let mut cmap: SlotMap<CellId, ()> = SlotMap::with_key();
        let x = cmap.insert(());
        let y = cmap.insert(());
        // r1 -> x -> r2 -> y -> r1
        let mut adj: HashMap<Node, Vec<Node>> = HashMap::new();
        adj.insert(Node::Relationship(r1), vec![Node::Cell(x)]);
        adj.insert(Node::Cell(x), vec![Node::Relationship(r2)]);
        adj.insert(Node::Relationship(r2), vec![Node::Cell(y)]);
        adj.insert(Node::Cell(y), vec![Node::Relationship(r1)]);
        let component = vec![
            Node::Relationship(r1),
            Node::Cell(x),
            Node::Relationship(r2),
            Node::Cell(y),
        ];
        let cycle = recover_cycle(&adj, &component);
        assert_eq!(cycle.len(), 4);
        // consecutive-with-wraparound edges all exist in adj
        for i in 0..cycle.len() {
            let from = cycle[i];
            let to = cycle[(i + 1) % cycle.len()];
            assert!(adj[&from].contains(&to), "missing edge {from:?}->{to:?}");
        }
    }

    #[test]
    fn minimal_infeasible_set_isolates_the_conflicting_pair() {
        use crate::{Method, Sheet};
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let out = sheet.add_cell(0_i32);
        let free = sheet.add_cell(0_i32);
        // r1 and r2 both must claim `out` (infeasible together); r3 is unrelated (claims free).
        let r1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, out, |x: &i32| Ok(*x))])
            .unwrap();
        let r2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(b, out, |x: &i32| Ok(*x))])
            .unwrap();
        let r3 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, free, |x: &i32| Ok(*x))])
            .unwrap();
        let active: std::collections::HashSet<_> = [r1, r2, r3].into_iter().collect();
        let set = minimal_infeasible_set(&sheet.relationships, &active);
        assert_eq!(set, [r1, r2].into_iter().collect());
    }
}
