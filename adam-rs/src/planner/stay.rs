//! Value-aware source selection for self-referencing blocks: for a connected component of
//! relationships containing at least one self-referencing method, chooses which cell(s)
//! stay literal sources by concretely executing every structurally valid candidate and
//! comparing violated stays, instead of by strength alone. A component with no
//! self-referencing method is untouched, left to [`super::release`]'s existing
//! strength-lexicographic algorithm. See
//! `docs/superpowers/specs/2026-09-07-adam-rs-value-aware-self-ref-planning-design.md` for
//! the full design rationale and literature grounding.

use std::collections::{HashSet, VecDeque};

use slotmap::SlotMap;

use crate::{
    cell::{CellData, CellId},
    relationship::{RelationshipData, RelationshipId},
};

/// Returns `true` if any of `rel`'s methods is self-referencing (some cell appears in
/// both its `inputs` and `outputs`).
pub(crate) fn has_self_reference(rel: &RelationshipData) -> bool {
    rel.methods
        .iter()
        .any(|m| m.inputs.iter().any(|i| m.outputs.contains(i)))
}

/// Partitions `active` into its connected components under cell-sharing: two
/// relationships are in the same component iff they share a cell, transitively. A
/// self-referencing chain overlapping a functional diamond via a shared cell always
/// lands in one component — see the design doc's rationale for why no separate cascade
/// step is needed on top of this.
///
/// - Complexity: O(R + sum of adjacency sizes) via breadth-first search.
pub(crate) fn partition_components(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> Vec<HashSet<RelationshipId>> {
    let mut visited: HashSet<RelationshipId> = HashSet::new();
    let mut components = Vec::new();

    for &start in active {
        if visited.contains(&start) {
            continue;
        }
        let mut component = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(start);
        visited.insert(start);

        while let Some(rel_id) = queue.pop_front() {
            component.insert(rel_id);
            for &cell_id in &relationships[rel_id].adj {
                for &neighbor in &cells[cell_id].adj {
                    if active.contains(&neighbor) && visited.insert(neighbor) {
                        queue.push_back(neighbor);
                    }
                }
            }
        }
        components.push(component);
    }
    components
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Method, Sheet};

    #[test]
    fn has_self_reference_true_for_a_method_with_overlapping_inputs_and_outputs() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| {
                Ok((*x).min(*y))
            })])
            .unwrap();

        assert!(has_self_reference(&sheet.relationships[rel]));
    }

    #[test]
    fn has_self_reference_false_for_a_purely_functional_relationship() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();

        assert!(!has_self_reference(&sheet.relationships[rel]));
    }

    #[test]
    fn partition_components_merges_relationships_sharing_a_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let rel1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let rel2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(b, c, |x: &i32| Ok(*x))])
            .unwrap();
        let active: HashSet<_> = [rel1, rel2].into_iter().collect();

        let components = partition_components(&sheet.cells, &sheet.relationships, &active);

        assert_eq!(components.len(), 1);
        assert_eq!(components[0], active);
    }

    #[test]
    fn partition_components_keeps_disjoint_relationships_in_separate_components() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let d = sheet.add_cell(0_i32);
        let rel1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let rel2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(c, d, |x: &i32| Ok(*x))])
            .unwrap();
        let active: HashSet<_> = [rel1, rel2].into_iter().collect();

        let components = partition_components(&sheet.cells, &sheet.relationships, &active);

        assert_eq!(components.len(), 2);
        let comp1: HashSet<_> = [rel1].into_iter().collect();
        let comp2: HashSet<_> = [rel2].into_iter().collect();
        assert!(
            (components[0] == comp1 && components[1] == comp2)
                || (components[0] == comp2 && components[1] == comp1)
        );
    }
}
