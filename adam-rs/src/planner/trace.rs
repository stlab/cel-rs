//! Cycle and infeasible-set reconstruction for planner error reporting. Runs only on
//! the cold error path, after a plan has already failed.

use std::collections::{HashMap, HashSet};

use super::digraph::Node;

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
}
