//! Builds the planner's dependency digraph from a chosen [`Assignment`], and checks
//! whether it is acyclic.

use std::collections::HashMap;

use slotmap::SlotMap;

use crate::relationship::{RelationshipData, RelationshipId};

use super::matching::{pure_outputs, Assignment};
use super::scc::tarjan_scc;

/// A node in the planner's dependency digraph: either a cell or a relationship.
///
/// Modeling relationships as their own nodes (rather than only cells) allows a
/// relationship with no pure outputs to still appear in this graph if it has at least
/// one plain (non-self-referencing) input. A relationship with neither plain inputs
/// nor pure outputs (fully self-referencing) contributes no edges and does not appear
/// in this graph; callers that require every active relationship represented must
/// account for that case separately.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Node {
    Cell(crate::cell::CellId),
    Relationship(RelationshipId),
}

/// Builds the dependency digraph induced by `assignment`: an edge from each of a
/// relationship's plain (non-self-referencing) input cells to the relationship, and
/// from the relationship to each of its pure-output cells.
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
            adj.entry(Node::Cell(input)).or_default().push(Node::Relationship(rel_id));
        }
        for output in pure_outputs(method) {
            adj.entry(Node::Relationship(rel_id)).or_default().push(Node::Cell(output));
        }
    }
    adj
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
    tarjan_scc(&adj).iter().all(|component| component.len() == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Method, Sheet};
    use std::collections::HashSet;

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
            .add_relationship(vec![Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok(x * y))])
            .unwrap();
        let r2 = sheet
            .add_relationship(vec![Method::from_fn_2_1([c, d], b, |x: &f64, y: &f64| Ok(y / x))])
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
        // Zero pure outputs (a is excluded, self-referencing) and zero plain inputs
        // (a is the only input, also self-referencing): the relationship contributes
        // no edges at all, so it must not appear as a key in `adj`.
        assert!(!adj.contains_key(&Node::Relationship(rel)));
        assert!(is_acyclic(&assignment, &sheet.relationships));
    }
}
