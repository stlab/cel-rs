//! Generic strongly-connected-components decomposition (Tarjan's algorithm), used by
//! the planner to detect cyclic dependency structures in its induced digraph.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// Computes the strongly connected components of the directed graph described by `adj`
/// (an adjacency map from node to its successors).
///
/// Nodes that only appear as a successor (a value in some `adj` entry) but never as a
/// key are still included as trivial (size-1) components.
///
/// - Postcondition: every node appearing as a key or value in `adj` appears in exactly
///   one returned component.
/// - Postcondition: for any edge `u -> v` where `u` and `v` land in different
///   components, `v`'s component appears **before** `u`'s component in the returned
///   `Vec` (Tarjan's classic reverse-topological output order). Callers that want
///   forward topological order must reverse the result.
///
/// - Complexity: O(V + E) where V = nodes, E = edges.
pub(crate) fn tarjan_scc<N>(adj: &HashMap<N, Vec<N>>) -> Vec<Vec<N>>
where
    N: Copy + Eq + Hash,
{
    struct State<N> {
        index: HashMap<N, usize>,
        lowlink: HashMap<N, usize>,
        on_stack: HashSet<N>,
        stack: Vec<N>,
        next_index: usize,
        components: Vec<Vec<N>>,
    }

    /// Visits node `v`, assigns index/lowlink values, recurses into unvisited successors, and pops completed SCC when `v` is a root.
    ///
    /// - Complexity: O(V + E) across all recursive calls in the full Tarjan run; each node/edge visited once.
    fn strongconnect<N>(v: N, adj: &HashMap<N, Vec<N>>, s: &mut State<N>)
    where
        N: Copy + Eq + Hash,
    {
        s.index.insert(v, s.next_index);
        s.lowlink.insert(v, s.next_index);
        s.next_index += 1;
        s.stack.push(v);
        s.on_stack.insert(v);

        if let Some(successors) = adj.get(&v) {
            for &w in successors {
                if !s.index.contains_key(&w) {
                    strongconnect(w, adj, s);
                    let w_low = s.lowlink[&w];
                    let v_low = s.lowlink[&v];
                    s.lowlink.insert(v, v_low.min(w_low));
                } else if s.on_stack.contains(&w) {
                    let w_idx = s.index[&w];
                    let v_low = s.lowlink[&v];
                    s.lowlink.insert(v, v_low.min(w_idx));
                }
            }
        }

        if s.lowlink[&v] == s.index[&v] {
            let mut component = Vec::new();
            loop {
                let w = s.stack.pop().expect("v's own SCC root is still on stack");
                s.on_stack.remove(&w);
                component.push(w);
                if w == v {
                    break;
                }
            }
            s.components.push(component);
        }
    }

    let mut nodes: Vec<N> = Vec::new();
    let mut seen: HashSet<N> = HashSet::new();
    for (&k, vs) in adj {
        if seen.insert(k) {
            nodes.push(k);
        }
        for &v in vs {
            if seen.insert(v) {
                nodes.push(v);
            }
        }
    }

    let mut state = State {
        index: HashMap::new(),
        lowlink: HashMap::new(),
        on_stack: HashSet::new(),
        stack: Vec::new(),
        next_index: 0,
        components: Vec::new(),
    };

    for node in nodes {
        if !state.index.contains_key(&node) {
            strongconnect(node, adj, &mut state);
        }
    }

    state.components
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellId;
    use slotmap::SlotMap;

    fn cells(n: usize) -> Vec<CellId> {
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        (0..n).map(|_| map.insert(())).collect()
    }

    #[test]
    fn empty_graph_has_no_components() {
        let adj: HashMap<CellId, Vec<CellId>> = HashMap::new();
        assert!(tarjan_scc(&adj).is_empty());
    }

    #[test]
    fn single_node_no_edges_is_trivial_component() {
        let ids = cells(1);
        let mut adj = HashMap::new();
        adj.insert(ids[0], vec![]);
        let components = tarjan_scc(&adj);
        assert_eq!(components, vec![vec![ids[0]]]);
    }

    #[test]
    fn two_cycle_is_one_component() {
        let ids = cells(2);
        let mut adj = HashMap::new();
        adj.insert(ids[0], vec![ids[1]]);
        adj.insert(ids[1], vec![ids[0]]);
        let components = tarjan_scc(&adj);
        assert_eq!(components.len(), 1);
        let comp: HashSet<CellId> = components[0].iter().copied().collect();
        let expected: HashSet<CellId> = ids.iter().copied().collect();
        assert_eq!(comp, expected);
    }

    #[test]
    fn diamond_shape_isolates_shared_cycle() {
        // a -> c, b -> c (R1: a,b -> c); c -> b, d -> b (R2: c,d -> b): b<->c cycle,
        // a and d are trivial (source-only) components.
        let ids = cells(4);
        let (a, b, c, d) = (ids[0], ids[1], ids[2], ids[3]);
        let mut adj = HashMap::new();
        adj.insert(a, vec![c]);
        adj.insert(b, vec![c]);
        adj.insert(c, vec![b]);
        adj.insert(d, vec![b]);
        let components = tarjan_scc(&adj);
        let non_trivial: Vec<&Vec<CellId>> = components.iter().filter(|c| c.len() > 1).collect();
        assert_eq!(non_trivial.len(), 1);
        let cyclic: HashSet<CellId> = non_trivial[0].iter().copied().collect();
        let expected: HashSet<CellId> = [b, c].into_iter().collect();
        assert_eq!(cyclic, expected);
    }

    #[test]
    fn chain_reversed_gives_topological_order() {
        // a -> b -> c (DAG, no cycle): reversed component order should be [a, b, c].
        let ids = cells(3);
        let (a, b, c) = (ids[0], ids[1], ids[2]);
        let mut adj = HashMap::new();
        adj.insert(a, vec![b]);
        adj.insert(b, vec![c]);
        let mut components = tarjan_scc(&adj);
        components.reverse();
        let order: Vec<CellId> = components.into_iter().flatten().collect();
        assert_eq!(order, vec![a, b, c]);
    }
}
