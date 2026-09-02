//! Pure mutation functions over a [`Document`]'s relationship groups.

use crate::model::cell_node::{CellNode, CellNodeId};
use crate::model::document::Document;
use crate::model::geometry::Point;
use crate::model::relationship_group::{RelationshipGroup, RelationshipGroupId};

/// Creates a new relationship group binding `a` and `b` as members with
/// empty formula text, auto-named `"r<n>"` from `doc`'s current
/// relationship-group count.
///
/// - Precondition: `a` and `b` are valid keys in `doc.cell_nodes`.
/// - Postcondition: the returned group's `members` is `[(a, ""), (b, "")]`.
#[must_use]
pub fn create_relationship(
    doc: &mut Document,
    a: CellNodeId,
    b: CellNodeId,
    position: Point,
) -> RelationshipGroupId {
    debug_assert!(doc.cell_nodes.contains_key(a), "a is not a valid key");
    debug_assert!(doc.cell_nodes.contains_key(b), "b is not a valid key");
    let display_name = format!("r{}", doc.relationship_group_order.len() + 1);
    let mut group = RelationshipGroup::new(display_name, position);
    group.members.push((a, String::new()));
    group.members.push((b, String::new()));
    let id = doc.relationship_groups.insert(group);
    doc.relationship_group_order.push(id);
    id
}

/// Adds `node` as a new member of `group` with empty formula text.
///
/// - Precondition: `group` is a valid key in `doc.relationship_groups`.
/// - Precondition: `node` is a valid key in `doc.cell_nodes`.
/// - Precondition: `node` is not already a member of `group`.
pub fn add_member(doc: &mut Document, group: RelationshipGroupId, node: CellNodeId) {
    debug_assert!(
        doc.relationship_groups.contains_key(group),
        "group is not a valid key"
    );
    debug_assert!(doc.cell_nodes.contains_key(node), "node is not a valid key");
    let g = &mut doc.relationship_groups[group];
    debug_assert!(
        !g.members.iter().any(|(n, _)| *n == node),
        "node is already a member"
    );
    g.members.push((node, String::new()));
}

/// Sets `node`'s RHS formula text within `group`.
///
/// - Precondition: `group` is a valid key in `doc.relationship_groups`.
/// - Precondition: `node` is a member of `group`.
pub fn set_member_formula(
    doc: &mut Document,
    group: RelationshipGroupId,
    node: CellNodeId,
    formula: impl Into<String>,
) {
    debug_assert!(
        doc.relationship_groups.contains_key(group),
        "group is not a valid key"
    );
    let g = &mut doc.relationship_groups[group];
    let entry = g.members.iter_mut().find(|(n, _)| *n == node);
    debug_assert!(entry.is_some(), "node is not a member of group");
    entry.unwrap().1 = formula.into();
}

/// Creates a copy of `group`'s formula "shape": a new relationship group
/// bound to new [`CellNode`]s over the *same* underlying cells as `group`'s
/// members (offset by `offset`), with formula text cleared.
///
/// This is "two instances of the same value in the graph" — the duplicated
/// nodes reference the same [`CellId`](crate::model::cell::CellId)s, not
/// copies of the cells themselves.
///
/// - Precondition: `group` is a valid key in `doc.relationship_groups`.
/// - Postcondition: the returned group has the same number of members as
///   `group`, each bound to a fresh node over the same cell, with empty
///   formula text.
///
/// - Complexity: O(n) in `group`'s member count.
#[must_use]
pub fn duplicate_relationship_group(
    doc: &mut Document,
    group: RelationshipGroupId,
    offset: Point,
) -> RelationshipGroupId {
    let source_members = doc.relationship_groups[group].members.clone();
    let source_position = doc.relationship_groups[group].position;

    let mut new_members = Vec::with_capacity(source_members.len());
    for (node, _formula) in &source_members {
        let CellNode { cell, position } = doc.cell_nodes[*node];
        let new_position = Point::new(position.x + offset.x, position.y + offset.y);
        let new_node = doc.cell_nodes.insert(CellNode::new(cell, new_position));
        new_members.push((new_node, String::new()));
    }

    let display_name = format!("r{}", doc.relationship_group_order.len() + 1);
    let new_position = Point::new(source_position.x + offset.x, source_position.y + offset.y);
    let mut new_group = RelationshipGroup::new(display_name, new_position);
    new_group.members = new_members;
    let id = doc.relationship_groups.insert(new_group);
    doc.relationship_group_order.push(id);
    id
}

#[cfg(test)]
mod duplicate_tests {
    use super::*;
    use crate::model::cell::CellType;
    use crate::ops::cells::{add_cell, add_cell_node};

    #[test]
    fn duplicate_binds_new_nodes_to_the_same_cells() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group, a_node, "height_pixels * 2");

        let dup = duplicate_relationship_group(&mut doc, group, Point::new(0.0, 100.0));

        let dup_cells: Vec<_> = doc.relationship_groups[dup]
            .members
            .iter()
            .map(|(n, _)| doc.cell_nodes[*n].cell)
            .collect();
        assert_eq!(dup_cells, vec![a, b]);
    }

    #[test]
    fn duplicate_clears_formula_text() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group, a_node, "height_pixels * 2");

        let dup = duplicate_relationship_group(&mut doc, group, Point::new(0.0, 100.0));

        for (_, formula) in &doc.relationship_groups[dup].members {
            assert_eq!(formula, "");
        }
    }

    #[test]
    fn duplicate_creates_distinct_node_instances() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let dup = duplicate_relationship_group(&mut doc, group, Point::new(0.0, 100.0));

        let dup_nodes: Vec<_> = doc.relationship_groups[dup]
            .members
            .iter()
            .map(|(n, _)| *n)
            .collect();
        assert!(!dup_nodes.contains(&a_node));
        assert!(!dup_nodes.contains(&b_node));
    }

    #[test]
    fn duplicate_auto_names_sequentially() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let dup = duplicate_relationship_group(&mut doc, group, Point::new(0.0, 100.0));

        assert_eq!(doc.relationship_groups[group].display_name, "r1");
        assert_eq!(doc.relationship_groups[dup].display_name, "r2");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::cell::CellType;
    use crate::ops::cells::{add_cell, add_cell_node};

    fn two_nodes(doc: &mut Document) -> (CellNodeId, CellNodeId) {
        let a = add_cell(doc, "width_pixels", CellType::i64());
        let b = add_cell(doc, "height_pixels", CellType::i64());
        (
            add_cell_node(doc, a, Point::new(0.0, 0.0)),
            add_cell_node(doc, b, Point::new(10.0, 0.0)),
        )
    }

    #[test]
    fn create_relationship_binds_both_nodes_with_empty_formulas() {
        let mut doc = Document::new("demo");
        let (a, b) = two_nodes(&mut doc);
        let group = create_relationship(&mut doc, a, b, Point::new(5.0, 5.0));
        assert_eq!(
            doc.relationship_groups[group].members,
            vec![(a, String::new()), (b, String::new())]
        );
    }

    #[test]
    fn create_relationship_auto_names_sequentially() {
        let mut doc = Document::new("demo");
        let (a, b) = two_nodes(&mut doc);
        let g1 = create_relationship(&mut doc, a, b, Point::new(0.0, 0.0));
        let g2 = create_relationship(&mut doc, a, b, Point::new(1.0, 1.0));
        assert_eq!(doc.relationship_groups[g1].display_name, "r1");
        assert_eq!(doc.relationship_groups[g2].display_name, "r2");
    }

    #[test]
    fn add_member_appends_a_new_member_with_empty_formula() {
        let mut doc = Document::new("demo");
        let (a, b) = two_nodes(&mut doc);
        let group = create_relationship(&mut doc, a, b, Point::new(0.0, 0.0));
        let c = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let c_node = add_cell_node(&mut doc, c, Point::new(20.0, 0.0));
        add_member(&mut doc, group, c_node);
        assert_eq!(doc.relationship_groups[group].members.len(), 3);
        assert_eq!(
            doc.relationship_groups[group].members[2],
            (c_node, String::new())
        );
    }

    #[test]
    fn set_member_formula_updates_the_matching_members_formula() {
        let mut doc = Document::new("demo");
        let (a, b) = two_nodes(&mut doc);
        let group = create_relationship(&mut doc, a, b, Point::new(0.0, 0.0));
        set_member_formula(&mut doc, group, a, "height_pixels * 2");
        assert_eq!(
            doc.relationship_groups[group].members[0].1,
            "height_pixels * 2"
        );
        assert_eq!(doc.relationship_groups[group].members[1].1, "");
    }
}
