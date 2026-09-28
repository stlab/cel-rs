//! Builds the planner's dependency digraph from a chosen [`Assignment`], and checks
//! whether it is acyclic.

use std::collections::{HashMap, VecDeque};

use slotmap::SlotMap;

use crate::cell::{CellData, CellId};
use crate::relationship::{RelationshipData, RelationshipId};

use super::matching::Assignment;
/// A node in the planner's dependency digraph: either a cell or a relationship.
///
/// Modeling relationships as their own nodes (rather than only cells) allows a
/// relationship with no plain inputs to still appear in this graph via its output
/// edges. Every relationship with a valid method (at least one output — enforced by
/// `Sheet::add_relationship`) contributes at least one edge and therefore always
/// appears in this graph: a method with no inputs at all (a fixed point) or a fully
/// self-referencing method (every cell is both an input and an output) contributes no
/// input edges (there are none, or they're all self-referencing and thus skipped), but
/// still contributes an output edge for each of its outputs, since `build_digraph`
/// draws output edges from *all* of a method's outputs, not just its pure ones — it is
/// exactly this output edge that guarantees the relationship is never absent from the
/// graph.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Node {
    Cell(CellId),
    Relationship(RelationshipId),
}

/// Builds the dependency digraph induced by `assignment`: an edge from each of a
/// relationship's plain (non-self-referencing) input cells to the relationship, and
/// from the relationship to each of its output cells (including self-referencing
/// ones: a self-referencing output still overwrites the cell with a freshly computed
/// value that same round, so any other relationship reading it as a plain input
/// genuinely depends on this one executing first).
///
/// - Complexity: O(R · K) where R = assigned relationships, K = cells per chosen method.
pub(crate) fn build_digraph(
    assignment: &Assignment,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
) -> HashMap<Node, Vec<Node>> {
    let mut adj: HashMap<Node, Vec<Node>> = HashMap::new();
    for (&rel_id, &method_idx) in &assignment.chosen {
        let method = &relationships[rel_id].methods[method_idx];
        for &input in &method.inputs {
            if method.outputs.contains(&input) {
                continue; // self-referencing input: pre-round value, no dependency edge
            }
            adj.entry(Node::Cell(input))
                .or_default()
                .push(Node::Relationship(rel_id));
        }
        for &output in &method.outputs {
            adj.entry(Node::Relationship(rel_id))
                .or_default()
                .push(Node::Cell(output));
        }
    }
    adj
}

/// Adds one edge `Cell(arg) → Cell(filtered)` for every dynamic argument `arg` of every
/// filtered cell `filtered` that is a **source** under `assignment` — not claimed as an
/// output by any of `assignment`'s chosen methods. A filtered cell that *is* claimed
/// (derived this round) contributes no edges: `Sheet::propagate`'s existing derived-value
/// diagnostic already covers it, as a read-only check with no ordering requirement.
///
/// Mutates `adj` in place. Called by [`super::plan`] once, after [`build_digraph`] has
/// already produced the base relationship-only graph, so the same topological sort places
/// each reclamp after everything its filter depends on and before everything that depends
/// on the filtered cell.
///
/// - Postcondition: every filtered, unclaimed cell is a key in `adj` after this call,
///   even one with zero filter args or no relationship membership.
///
/// - Complexity: O(C · a) where C = cells with a filter, a = arguments per filter.
pub(crate) fn add_filter_edges(
    adj: &mut HashMap<Node, Vec<Node>>,
    cells: &SlotMap<CellId, CellData>,
    assignment: &Assignment,
) {
    for (cell_id, cell) in cells.iter() {
        let Some(filter) = cell.filter.as_ref() else {
            continue;
        };
        if assignment.claimed.contains_key(&cell_id) {
            continue;
        }
        // Ensures the filtered cell is a node even when it has no args and belongs to
        // no relationship (a zero-argument filter contributes no edges below), so it
        // always gets a PlanStep::FilterReclamp.
        adj.entry(Node::Cell(cell_id)).or_default();
        for &arg in &filter.args {
            adj.entry(Node::Cell(arg))
                .or_default()
                .push(Node::Cell(cell_id));
        }
    }
}

/// Returns `true` if `assignment`'s induced digraph has no non-trivial strongly
/// connected component (every relationship can be executed in some valid order).
///
/// - Complexity: O(R · K) (dominated by [`build_digraph`]; SCC is O(V + E) on the
///   resulting graph).
pub(crate) fn is_acyclic(
    assignment: &Assignment,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
) -> bool {
    let adj = build_digraph(assignment, relationships);
    topological_order(&adj).is_some()
}

/// Returns every graph node in dependency order, or `None` when the graph contains a cycle.
///
/// Nodes that only appear as successors are included in the returned order.
///
/// - Complexity: O(V + E), where V is the number of nodes and E is the number of edges.
pub(crate) fn topological_order<N>(adj: &HashMap<N, Vec<N>>) -> Option<Vec<N>>
where
    N: Copy + Eq + std::hash::Hash,
{
    let mut indegree = HashMap::new();
    for (&node, successors) in adj {
        indegree.entry(node).or_insert(0);
        for &successor in successors {
            *indegree.entry(successor).or_insert(0) += 1;
        }
    }

    let mut ready: VecDeque<N> = indegree
        .iter()
        .filter_map(|(&node, &degree)| (degree == 0).then_some(node))
        .collect();
    let mut order = Vec::with_capacity(indegree.len());

    while let Some(node) = ready.pop_front() {
        order.push(node);
        for &successor in adj.get(&node).into_iter().flatten() {
            let degree = indegree
                .get_mut(&successor)
                .expect("every successor has an indegree entry");
            *degree -= 1;
            if *degree == 0 {
                ready.push_back(successor);
            }
        }
    }

    (order.len() == indegree.len()).then_some(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Filter, Method, Sheet};
    use std::collections::{HashMap, HashSet};

    #[test]
    fn acyclic_assignment_reports_acyclic() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let active: HashSet<_> = [rel].into_iter().collect();
        let assignment = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        assert!(is_acyclic(&assignment, &sheet.relationships));
    }

    #[test]
    fn cyclic_assignment_reports_not_acyclic() {
        // Force the diamond's colliding pairing directly: R1's only method claims c via
        // [a,b]->c, R2's only method claims b via [c,d]->b -- b depends on c (R1) and
        // c depends on b (R2).
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0.0_f64);
        let b = sheet.add_cell(0.0_f64);
        let c = sheet.add_cell(0.0_f64);
        let d = sheet.add_cell(0.0_f64);
        let r1 = sheet
            .add_relationship(vec![Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| {
                Ok(x * y)
            })])
            .unwrap();
        let r2 = sheet
            .add_relationship(vec![Method::from_fn_2_1([c, d], b, |x: &f64, y: &f64| {
                Ok(y / x)
            })])
            .unwrap();
        let active: HashSet<_> = [r1, r2].into_iter().collect();
        let assignment = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        assert!(!is_acyclic(&assignment, &sheet.relationships));
    }

    #[test]
    fn purely_self_referencing_relationship_still_appears_as_a_node() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, a, |x: &i32| Ok((*x).min(0)))])
            .unwrap();
        let active: HashSet<_> = [rel].into_iter().collect();
        let assignment = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        let adj = build_digraph(&assignment, &sheet.relationships);
        // Zero plain inputs (a is the only input, and it's self-referencing, so the
        // input-edge loop skips it) but a's self-referencing output still contributes
        // an output edge: the relationship must appear as a key in `adj`, with exactly
        // one edge to Cell(a).
        assert!(adj.contains_key(&Node::Relationship(rel)));
        assert_eq!(adj[&Node::Relationship(rel)], vec![Node::Cell(a)]);
        assert!(is_acyclic(&assignment, &sheet.relationships));
    }

    #[test]
    fn add_filter_edges_adds_edge_from_argument_to_filtered_source_cell() {
        let mut sheet = Sheet::new();
        let bound = sheet.add_cell(10_i32);
        let a = sheet.add_cell(5_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |x: &i32, b: &i32| Ok((*x).min(*b))),
            )
            .unwrap();

        let assignment =
            Assignment::solve(&sheet.relationships, &HashSet::new(), &HashSet::new()).unwrap();
        let mut adj: HashMap<Node, Vec<Node>> = HashMap::new();
        add_filter_edges(&mut adj, &sheet.cells, &assignment);

        assert_eq!(adj.get(&Node::Cell(bound)), Some(&vec![Node::Cell(a)]));
    }

    #[test]
    fn add_filter_edges_skips_a_filtered_cell_claimed_by_the_assignment() {
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(5_i32);
        let bound = sheet.add_cell(10_i32);
        let y = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(x, y, |v: &i32| Ok(*v))])
            .unwrap();
        sheet
            .add_filter(
                y,
                Filter::from_fn_1(bound, |v: &i32, b: &i32| Ok((*v).min(*b))),
            )
            .unwrap();

        let active: HashSet<_> = [rel].into_iter().collect();
        let assignment = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        let mut adj: HashMap<Node, Vec<Node>> = HashMap::new();
        add_filter_edges(&mut adj, &sheet.cells, &assignment);

        assert!(adj.is_empty());
    }

    #[test]
    fn add_filter_edges_adds_a_node_for_a_zero_argument_filter_with_no_relationship_membership() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(500_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();

        let assignment =
            Assignment::solve(&sheet.relationships, &HashSet::new(), &HashSet::new()).unwrap();
        let mut adj: HashMap<Node, Vec<Node>> = HashMap::new();
        add_filter_edges(&mut adj, &sheet.cells, &assignment);

        // `a` has no filter args and belongs to no relationship, so nothing would
        // otherwise ever insert it into the digraph — without this, plan() would
        // never emit a FilterReclamp step for it.
        assert!(adj.contains_key(&Node::Cell(a)));
    }

    #[test]
    fn topological_order_returns_forward_order_for_a_dag() {
        let ids = {
            let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
            (0..3).map(|_| map.insert(())).collect::<Vec<_>>()
        };
        let (a, b, c) = (ids[0], ids[1], ids[2]);
        let mut adj = HashMap::new();
        adj.insert(Node::Cell(a), vec![Node::Cell(b)]);
        adj.insert(Node::Cell(b), vec![Node::Cell(c)]);

        assert_eq!(
            topological_order(&adj),
            Some(vec![Node::Cell(a), Node::Cell(b), Node::Cell(c)])
        );
    }

    #[test]
    fn topological_order_returns_none_for_a_cycle() {
        let ids = {
            let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
            (0..2).map(|_| map.insert(())).collect::<Vec<_>>()
        };
        let (a, b) = (ids[0], ids[1]);
        let mut adj = HashMap::new();
        adj.insert(Node::Cell(a), vec![Node::Cell(b)]);
        adj.insert(Node::Cell(b), vec![Node::Cell(a)]);

        assert_eq!(topological_order(&adj), None);
    }
}
