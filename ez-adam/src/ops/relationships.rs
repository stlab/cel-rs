//! Pure mutation functions over a [`Document`]'s relationship groups.

use crate::model::cell_node::CellNodeId;
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
    let g = &mut doc.relationship_groups[group];
    let entry = g.members.iter_mut().find(|(n, _)| *n == node);
    debug_assert!(entry.is_some(), "node is not a member of group");
    entry.unwrap().1 = formula.into();
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
