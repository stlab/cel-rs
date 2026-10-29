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
/// - Precondition: `cells.len() < usize::BITS`, so `1 << cells.len()` (the
///   branch count) neither overflows nor wraps. In practice `cells.len()`
///   must be far smaller, since the branch count grows as `2^cells.len()`.
/// - Precondition: every cell in `cells` is a valid key in `doc.cells`.
/// - Precondition: every cell in `cells` has [`CellType::Bool`].
/// - Precondition: `group` is a valid key in `doc.relationship_groups`.
/// - Postcondition: the returned group has exactly `2.pow(cells.len())`
///   branches and an empty `default`.
///
/// - Complexity: O(n·2^n) time and space in `cells.len()`.
/// # Examples
///
/// `add_conditional_from_bool_cells(&mut document, vec![flag], group, position)`
/// creates false/true branches and enables `group` only on the true branch.
#[must_use]
pub fn add_conditional_from_bool_cells(
    doc: &mut Document,
    cells: Vec<CellId>,
    group: RelationshipGroupId,
    position: Point,
) -> ConditionalGroupId {
    debug_assert!(!cells.is_empty(), "cells must be non-empty");
    debug_assert!(
        cells.len() < usize::BITS as usize,
        "cells.len() must be shift-safe for 1 << cells.len()"
    );
    debug_assert!(
        cells.iter().all(|c| doc.cells.contains_key(*c)),
        "every condition cell must be a valid key"
    );
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

/// Creates a new conditional group whose condition is a user-authored CEL
/// formula over `referenced_cells`, with no branches yet (added via
/// [`add_branch`]) and an empty default.
///
/// - Precondition: every cell in `referenced_cells` is a valid key in
///   `doc.cells`.
/// - Complexity: O(expression bytes) when converting borrowed expression text.
///
/// # Examples
///
/// `add_conditional_with_formula(&mut document, vec![], "true", position)`
/// creates a scalar constant condition without dependencies.
#[must_use]
pub fn add_conditional_with_formula(
    doc: &mut Document,
    referenced_cells: Vec<CellId>,
    expr: impl Into<String>,
    position: Point,
) -> ConditionalGroupId {
    debug_assert!(
        referenced_cells.iter().all(|c| doc.cells.contains_key(*c)),
        "every referenced cell must be a valid key"
    );
    let display_name = format!("c{}", doc.conditional_group_order.len() + 1);
    let id = doc.conditional_groups.insert(ConditionalGroup {
        display_name,
        position,
        condition: ConditionExpr::Formula {
            referenced_cells,
            expr: expr.into(),
        },
        branches: Vec::new(),
        default: Vec::new(),
    });
    doc.conditional_group_order.push(id);
    id
}

/// Adds a new branch to `conditional` matching `values`, with no
/// relationship groups enabled yet.
///
/// - Precondition: `conditional` is a valid key in `doc.conditional_groups`.
/// - Precondition: `values.len()` matches `conditional`'s condition arity
///   (the number of cells in [`ConditionExpr::Cells`], or one scalar
///   result for [`ConditionExpr::Formula`], regardless of dependency count).
/// - Postcondition: returns the new branch's index within
///   `conditional.branches`.
/// # Examples
///
/// `add_branch(&mut document, scalar_condition, vec![CellValueLiteral::Bool(true)])`
/// adds a scalar key independently of the expression's dependency count.
pub fn add_branch(
    doc: &mut Document,
    conditional: ConditionalGroupId,
    values: Vec<CellValueLiteral>,
) -> usize {
    debug_assert!(
        doc.conditional_groups.contains_key(conditional),
        "conditional is not a valid key"
    );
    let group = &mut doc.conditional_groups[conditional];
    let arity = match &group.condition {
        ConditionExpr::Cells(cells) => cells.len(),
        ConditionExpr::Formula { .. } => 1,
    };
    debug_assert_eq!(
        values.len(),
        arity,
        "values.len() must match condition arity"
    );
    group.branches.push(ConditionalBranch {
        values,
        enabled_groups: Vec::new(),
    });
    group.branches.len() - 1
}

/// Sets `conditional`'s `Formula`-mode condition expression text.
///
/// - Precondition: `conditional` is a valid key in `doc.conditional_groups`.
/// - Precondition: `conditional`'s condition is `ConditionExpr::Formula` (not
///   `Cells`).
/// - Complexity: O(expression bytes) when converting borrowed expression text.
///
/// # Examples
///
/// `set_condition_formula(&mut document, formula_condition, "width > height")`
/// updates the draft while preserving its dependency metadata.
pub fn set_condition_formula(
    doc: &mut Document,
    conditional: ConditionalGroupId,
    expr: impl Into<String>,
) {
    debug_assert!(
        doc.conditional_groups.contains_key(conditional),
        "conditional is not a valid key"
    );
    let group = &mut doc.conditional_groups[conditional];
    let ConditionExpr::Formula {
        expr: current_expr, ..
    } = &mut group.condition
    else {
        debug_assert!(
            false,
            "set_condition_formula requires a Formula-mode condition"
        );
        return;
    };
    *current_expr = expr.into();
}

/// Sets `conditional`'s display name — UI bookkeeping only, never emitted
/// to `.adm2`.
///
/// - Precondition: `conditional` is a valid key in `doc.conditional_groups`.
/// - Complexity: O(name bytes) when converting borrowed text.
///
/// # Examples
///
/// `set_display_name(&mut document, conditional, "Landscape")` updates its canvas label.
pub fn set_display_name(
    doc: &mut Document,
    conditional: ConditionalGroupId,
    name: impl Into<String>,
) {
    debug_assert!(
        doc.conditional_groups.contains_key(conditional),
        "conditional is not a valid key"
    );
    doc.conditional_groups[conditional].display_name = name.into();
}

/// Toggles whether `group` is active on `conditional`'s branch at
/// `branch_index` — enables it if absent, disables it if present.
///
/// - Precondition: `conditional` is a valid key in `doc.conditional_groups`.
/// - Precondition: `branch_index < conditional.branches.len()`.
/// - Precondition: `group` is a valid key in `doc.relationship_groups`.
///
/// - Complexity: O(n) in the branch's `enabled_groups` length (it scans for
///   `group` to toggle it).
/// # Examples
///
/// `toggle_enabled_group(&mut document, conditional, 0, group)` toggles the first row.
pub fn toggle_enabled_group(
    doc: &mut Document,
    conditional: ConditionalGroupId,
    branch_index: usize,
    group: RelationshipGroupId,
) {
    debug_assert!(
        doc.conditional_groups.contains_key(conditional),
        "conditional is not a valid key"
    );
    debug_assert!(
        branch_index < doc.conditional_groups[conditional].branches.len(),
        "branch_index is out of bounds"
    );
    debug_assert!(
        doc.relationship_groups.contains_key(group),
        "group is not a valid key"
    );
    let branch = &mut doc.conditional_groups[conditional].branches[branch_index];
    if let Some(pos) = branch.enabled_groups.iter().position(|g| *g == group) {
        branch.enabled_groups.remove(pos);
    } else {
        branch.enabled_groups.push(group);
    }
}

/// Removes `conditional` from `doc`. Nothing else in a [`Document`] ever
/// references a [`ConditionalGroupId`], so no further cleanup is needed.
///
/// - Precondition: `conditional` is a valid key in `doc.conditional_groups`.
///
/// - Complexity: O(order entries + deleted group contents) time and O(1) auxiliary space.
///
/// # Examples
///
/// `delete_conditional_group(&mut document, conditional)` removes the group and its order entry.
pub fn delete_conditional_group(doc: &mut Document, conditional: ConditionalGroupId) {
    debug_assert!(
        doc.conditional_groups.contains_key(conditional),
        "conditional is not a valid key"
    );
    doc.conditional_groups.remove(conditional);
    doc.conditional_group_order.retain(|c| *c != conditional);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::geometry::Point;
    use crate::ops::cells::{add_cell, add_cell_node};
    use crate::ops::relationships::create_relationship;

    /// Adds a Boolean selector and a two-member integer relationship to `doc`.
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
    /// One Boolean dependency produces false and true branches.
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
    /// Two Boolean dependencies produce all four combinations.
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
    /// A one-cell truth table enables its relationship only for true.
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
    /// A two-cell truth table enables its relationship only when both values are true.
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
    /// A newly enumerated conditional starts without default relationships.
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

#[cfg(test)]
mod formula_tests {
    use super::*;
    use crate::model::geometry::Point;
    use crate::ops::cells::{add_cell, add_cell_node};
    use crate::ops::relationships::create_relationship;

    #[test]
    /// Formula conditions retain their expression and start without branches.
    fn add_conditional_with_formula_starts_with_no_branches() {
        let mut doc = Document::new("demo");
        let x = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let cond = add_conditional_with_formula(
            &mut doc,
            vec![x],
            "aspect_ratio > 2.0",
            Point::new(0.0, 0.0),
        );
        assert!(doc.conditional_groups[cond].branches.is_empty());
        assert!(doc.conditional_groups[cond].default.is_empty());
    }

    #[test]
    /// Branch insertion preserves its key and starts without enabled groups.
    fn add_branch_appends_a_branch_with_no_enabled_groups() {
        let mut doc = Document::new("demo");
        let x = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let cond = add_conditional_with_formula(
            &mut doc,
            vec![x],
            "aspect_ratio > 2.0",
            Point::new(0.0, 0.0),
        );
        add_branch(&mut doc, cond, vec![CellValueLiteral::Bool(true)]);
        assert_eq!(doc.conditional_groups[cond].branches.len(), 1);
        assert!(
            doc.conditional_groups[cond].branches[0]
                .enabled_groups
                .is_empty()
        );
    }

    #[test]
    /// Branch insertion returns the appended row's index.
    fn add_branch_returns_the_new_branchs_index() {
        let mut doc = Document::new("demo");
        let x = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let cond = add_conditional_with_formula(
            &mut doc,
            vec![x],
            "aspect_ratio > 2.0",
            Point::new(0.0, 0.0),
        );
        let i0 = add_branch(&mut doc, cond, vec![CellValueLiteral::Bool(true)]);
        let i1 = add_branch(&mut doc, cond, vec![CellValueLiteral::Bool(false)]);
        assert_eq!(i0, 0);
        assert_eq!(i1, 1);
    }

    #[test]
    /// Formula edits replace the stored expression draft.
    fn set_condition_formula_updates_the_formula_expr() {
        let mut doc = Document::new("demo");
        let x = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let cond = add_conditional_with_formula(&mut doc, vec![x], "", Point::new(0.0, 0.0));

        set_condition_formula(&mut doc, cond, "aspect_ratio > 2.0");

        let ConditionExpr::Formula { expr, .. } = &doc.conditional_groups[cond].condition else {
            panic!("expected a Formula-mode condition");
        };
        assert_eq!(expr, "aspect_ratio > 2.0");
    }

    #[test]
    /// Display-name edits replace the conditional's canvas label.
    fn set_display_name_updates_the_conditionals_display_name() {
        let mut doc = Document::new("demo");
        let x = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let cond = add_conditional_with_formula(&mut doc, vec![x], "", Point::new(0.0, 0.0));

        set_display_name(&mut doc, cond, "aspect gate");

        assert_eq!(doc.conditional_groups[cond].display_name, "aspect gate");
    }

    #[test]
    /// Toggling a relationship twice restores the branch's original enabled set.
    fn toggle_enabled_group_enables_then_disables() {
        let mut doc = Document::new("demo");
        let x = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let cond = add_conditional_with_formula(
            &mut doc,
            vec![x],
            "aspect_ratio > 2.0",
            Point::new(0.0, 0.0),
        );
        add_branch(&mut doc, cond, vec![CellValueLiteral::Bool(true)]);

        // A real relationship group in `doc`, so `group` is a valid key (the
        // op's precondition), rather than a dangling id from an unrelated map.
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        toggle_enabled_group(&mut doc, cond, 0, group);
        assert_eq!(
            doc.conditional_groups[cond].branches[0].enabled_groups,
            vec![group]
        );

        toggle_enabled_group(&mut doc, cond, 0, group);
        assert!(
            doc.conditional_groups[cond].branches[0]
                .enabled_groups
                .is_empty()
        );
    }

    #[test]
    /// Conditional deletion removes its value and declaration-order entry.
    fn delete_conditional_group_removes_it_from_the_document() {
        let mut doc = Document::new("demo");
        let x = add_cell(&mut doc, "x", CellType::f64());
        let cond = add_conditional_with_formula(&mut doc, vec![x], "x > 1.0", Point::new(0.0, 0.0));

        delete_conditional_group(&mut doc, cond);

        assert!(doc.conditional_groups_in_order().next().is_none());
        assert!(!doc.conditional_group_order.contains(&cond));
    }
}
