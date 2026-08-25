//! Pure mutation functions over a [`Document`]'s conditional groups.

use crate::model::cell::{CellId, CellType};
use crate::model::conditional_group::{
    CellValueLiteral, ConditionExpr, ConditionalBranch, ConditionalGroup, ConditionalGroupId,
};
use crate::model::document::Document;
use crate::model::geometry::Point;
use crate::model::relationship_group::RelationshipGroupId;

/// Wraps `group` in a new conditional group whose condition is the tuple of
/// `cells`' own boolean values, auto-enumerated into every combination of
/// `true`/`false` (`2.pow(cells.len())` branches). `group` is enabled on
/// the branch where every cell is `true`; every other branch (and the
/// default) starts with no enabled groups.
///
/// - Precondition: `cells` is non-empty.
/// - Precondition: every cell in `cells` has [`CellType::Bool`].
/// - Precondition: `group` is a valid key in `doc.relationship_groups`.
/// - Postcondition: the returned group has exactly `2.pow(cells.len())`
///   branches and an empty `default`.
///
/// - Complexity: O(2^n) in `cells.len()`.
#[must_use]
pub fn add_conditional_from_bool_cells(
    doc: &mut Document,
    cells: Vec<CellId>,
    group: RelationshipGroupId,
    position: Point,
) -> ConditionalGroupId {
    debug_assert!(!cells.is_empty(), "cells must be non-empty");
    debug_assert!(
        cells
            .iter()
            .all(|c| matches!(doc.cells[*c].ty, CellType::Bool)),
        "every condition cell must be Bool"
    );
    debug_assert!(
        doc.relationship_groups.contains_key(group),
        "group is not a valid key"
    );

    let branch_count = 1usize << cells.len();
    let mut branches = Vec::with_capacity(branch_count);
    for combo in 0..branch_count {
        let values: Vec<CellValueLiteral> = (0..cells.len())
            .map(|i| CellValueLiteral::Bool((combo >> i) & 1 == 1))
            .collect();
        let all_true = values
            .iter()
            .all(|v| matches!(v, CellValueLiteral::Bool(true)));
        let enabled_groups = if all_true { vec![group] } else { Vec::new() };
        branches.push(ConditionalBranch {
            values,
            enabled_groups,
        });
    }

    let display_name = format!("c{}", doc.conditional_group_order.len() + 1);
    let id = doc.conditional_groups.insert(ConditionalGroup {
        display_name,
        position,
        condition: ConditionExpr::Cells(cells),
        branches,
        default: Vec::new(),
    });
    doc.conditional_group_order.push(id);
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::geometry::Point;
    use crate::ops::cells::{add_cell, add_cell_node};
    use crate::ops::relationships::create_relationship;

    fn setup_group_over_two_bool_cells(doc: &mut Document) -> (CellId, RelationshipGroupId) {
        let condition_cell = add_cell(doc, "constrain_proportions", CellType::Bool);
        let a = add_cell(doc, "width_pixels", CellType::i64());
        let b = add_cell(doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(doc, a_node, b_node, Point::new(5.0, 5.0));
        (condition_cell, group)
    }

    #[test]
    fn one_bool_cell_creates_two_branches() {
        let mut doc = Document::new("demo");
        let (condition_cell, group) = setup_group_over_two_bool_cells(&mut doc);
        let cond = add_conditional_from_bool_cells(
            &mut doc,
            vec![condition_cell],
            group,
            Point::new(0.0, 0.0),
        );
        assert_eq!(doc.conditional_groups[cond].branches.len(), 2);
    }

    #[test]
    fn two_bool_cells_creates_four_branches() {
        let mut doc = Document::new("demo");
        let (condition_cell, group) = setup_group_over_two_bool_cells(&mut doc);
        let second_cell = add_cell(&mut doc, "lock_aspect", CellType::Bool);
        let cond = add_conditional_from_bool_cells(
            &mut doc,
            vec![condition_cell, second_cell],
            group,
            Point::new(0.0, 0.0),
        );
        assert_eq!(doc.conditional_groups[cond].branches.len(), 4);
    }

    #[test]
    fn group_is_enabled_only_on_the_all_true_branch() {
        let mut doc = Document::new("demo");
        let (condition_cell, group) = setup_group_over_two_bool_cells(&mut doc);
        let cond = add_conditional_from_bool_cells(
            &mut doc,
            vec![condition_cell],
            group,
            Point::new(0.0, 0.0),
        );
        let all_true_branch = doc.conditional_groups[cond]
            .branches
            .iter()
            .find(|b| b.values == vec![CellValueLiteral::Bool(true)])
            .unwrap();
        assert_eq!(all_true_branch.enabled_groups, vec![group]);

        let false_branch = doc.conditional_groups[cond]
            .branches
            .iter()
            .find(|b| b.values == vec![CellValueLiteral::Bool(false)])
            .unwrap();
        assert!(false_branch.enabled_groups.is_empty());
    }

    #[test]
    fn with_two_cells_only_the_all_true_branch_is_enabled() {
        let mut doc = Document::new("demo");
        let (condition_cell, group) = setup_group_over_two_bool_cells(&mut doc);
        let second_cell = add_cell(&mut doc, "lock_aspect", CellType::Bool);
        let cond = add_conditional_from_bool_cells(
            &mut doc,
            vec![condition_cell, second_cell],
            group,
            Point::new(0.0, 0.0),
        );
        for branch in &doc.conditional_groups[cond].branches {
            let all_true =
                branch.values == vec![CellValueLiteral::Bool(true), CellValueLiteral::Bool(true)];
            if all_true {
                assert_eq!(branch.enabled_groups, vec![group]);
            } else {
                assert!(branch.enabled_groups.is_empty());
            }
        }
    }

    #[test]
    fn default_starts_empty() {
        let mut doc = Document::new("demo");
        let (condition_cell, group) = setup_group_over_two_bool_cells(&mut doc);
        let cond = add_conditional_from_bool_cells(
            &mut doc,
            vec![condition_cell],
            group,
            Point::new(0.0, 0.0),
        );
        assert!(doc.conditional_groups[cond].default.is_empty());
    }
}
