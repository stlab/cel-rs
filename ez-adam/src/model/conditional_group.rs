//! Conditional groups: alternative sets of relationship-group activations
//! selected by a condition (mirrors `.adm2`'s
//! `conditional <expr> { <literal> => {...} }`).

use serde::{Deserialize, Serialize};
use slotmap::new_key_type;

use crate::model::cell::CellId;
use crate::model::geometry::Point;
use crate::model::relationship_group::RelationshipGroupId;

new_key_type! {
    /// A stable handle to a [`ConditionalGroup`] in a
    /// [`crate::model::document::Document`].
    pub struct ConditionalGroupId;
}

/// A literal value matched against a conditional group's branch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CellValueLiteral {
    /// A boolean literal value.
    Bool(bool),
    /// A 64-bit signed integer literal value.
    I64(i64),
    /// A text literal value.
    Text(String),
}

/// The expression a [`ConditionalGroup`] branches on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConditionExpr {
    /// Implicit tuple of the dragged-in cells' own values. Auto-enumerable
    /// into a full branch table only when every cell is `Bool`.
    Cells(Vec<CellId>),
    /// A user-authored CEL expression referencing `referenced_cells` (e.g.
    /// `x > 100`). Branches are added manually.
    Formula {
        /// The cells referenced in the expression.
        referenced_cells: Vec<CellId>,
        /// The CEL expression string.
        expr: String,
    },
}

/// One row of a conditional group's enable-table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionalBranch {
    /// One literal per [`ConditionExpr`] cell, aligned by index.
    pub values: Vec<CellValueLiteral>,
    /// The relationship groups active (checked) on this branch.
    pub enabled_groups: Vec<RelationshipGroupId>,
}

/// A set of alternative relationship-group activations selected by
/// [`ConditionExpr`]'s current value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionalGroup {
    /// The display name of this conditional group.
    pub display_name: String,
    /// The canvas position of this conditional group.
    pub position: Point,
    /// The condition expression that drives branch selection.
    pub condition: ConditionExpr,
    /// The table of branches: each branch maps literal values to an enabled set
    /// of relationship groups.
    pub branches: Vec<ConditionalBranch>,
    /// The relationship groups active when no branch matches. Always
    /// present — `adam-rs`'s `Sheet::add_conditional` requires a default
    /// non-optionally.
    pub default: Vec<RelationshipGroupId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    #[test]
    fn cell_value_literals_of_different_variants_are_unequal() {
        assert_ne!(CellValueLiteral::Bool(true), CellValueLiteral::Bool(false));
        assert_ne!(CellValueLiteral::I64(1), CellValueLiteral::I64(2));
    }

    #[test]
    fn condition_expr_cells_stores_the_given_cell_ids() {
        let mut cells: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = cells.insert(());
        let b = cells.insert(());
        let condition = ConditionExpr::Cells(vec![a, b]);
        assert_eq!(condition, ConditionExpr::Cells(vec![a, b]));
    }

    #[test]
    fn conditional_branch_stores_values_and_enabled_groups() {
        let mut groups: SlotMap<RelationshipGroupId, ()> = SlotMap::with_key();
        let group = groups.insert(());
        let branch = ConditionalBranch {
            values: vec![CellValueLiteral::Bool(true)],
            enabled_groups: vec![group],
        };
        assert_eq!(branch.values, vec![CellValueLiteral::Bool(true)]);
        assert_eq!(branch.enabled_groups, vec![group]);
    }
}
