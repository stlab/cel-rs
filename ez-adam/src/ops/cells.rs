//! Pure mutation functions over a [`Document`]'s cells.

use crate::model::cell::{Cell, CellId, CellType};
use crate::model::cell_node::{CellNode, CellNodeId};
use crate::model::document::Document;
use crate::model::geometry::Point;

/// Adds a new, non-output cell named `name` with no restriction.
///
/// - Postcondition: the returned id resolves to a [`Cell`] with `output ==
///   false` and `restrict == None`.
#[must_use]
pub fn add_cell(doc: &mut Document, name: impl Into<String>, ty: CellType) -> CellId {
    let id = doc.cells.insert(Cell::new(name, ty));
    doc.cell_order.push(id);
    id
}

/// Places a new visual instance of `cell` at `position`.
///
/// - Precondition: `cell` is a valid key in `doc.cells`.
#[must_use]
pub fn add_cell_node(doc: &mut Document, cell: CellId, position: Point) -> CellNodeId {
    debug_assert!(doc.cells.contains_key(cell), "cell is not a valid key");
    doc.cell_nodes.insert(CellNode::new(cell, position))
}

/// Sets whether `cell` is an output cell. Not currently reflected by
/// `.adm2` codegen — see <https://github.com/stlab/cel-rs/issues/147>.
///
/// - Precondition: `cell` is a valid key in `doc.cells`.
pub fn set_output(doc: &mut Document, cell: CellId, output: bool) {
    debug_assert!(doc.cells.contains_key(cell), "cell is not a valid key");
    doc.cells[cell].output = output;
}

/// Sets `cell`'s restrict-expression text (or clears it with `None`).
///
/// - Precondition: `cell` is a valid key in `doc.cells`.
pub fn set_restrict(doc: &mut Document, cell: CellId, restrict: Option<String>) {
    debug_assert!(doc.cells.contains_key(cell), "cell is not a valid key");
    doc.cells[cell].restrict = restrict;
}

/// Sets `cell`'s name.
///
/// - Precondition: `cell` is a valid key in `doc.cells`.
/// - Postcondition: every [`crate::model::cell_node::CellNode`] placing
///   `cell` reflects the new name, since a `Cell`'s data is shared by all
///   of its placements (see [`Cell`]'s own doc comment) — this does not,
///   however, rewrite any formula text that referenced the old name.
pub fn set_name(doc: &mut Document, cell: CellId, name: impl Into<String>) {
    debug_assert!(doc.cells.contains_key(cell), "cell is not a valid key");
    doc.cells[cell].name = name.into();
}

/// Parses clamp-bound input text into the new bound value to apply.
///
/// - Postcondition: returns `Some(None)` ("clear the bound") for an empty
///   `text`, `Some(Some(v))` ("set the bound to `v`") for text that parses
///   as `T`, or `None` ("leave the existing bound alone") for anything
///   else — an in-progress or malformed number (e.g. a lone `"-"` while
///   typing a negative value) shouldn't silently erase an already-valid
///   bound.
fn parse_clamp_bound<T: std::str::FromStr>(text: &str) -> Option<Option<T>> {
    if text.is_empty() {
        return Some(None);
    }
    text.parse::<T>().ok().map(Some)
}

/// Sets `cell`'s clamp minimum from `text`: empty text clears the bound,
/// text that parses as `cell`'s numeric type sets it, and anything else
/// leaves the existing bound unchanged (see `parse_clamp_bound`).
///
/// - Precondition: `cell` is a valid key in `doc.cells`.
/// - Precondition: `cell`'s type is `F64` or `I64` (has a clamp to set).
pub fn set_clamp_min(doc: &mut Document, cell: CellId, text: &str) {
    debug_assert!(doc.cells.contains_key(cell), "cell is not a valid key");
    match &mut doc.cells[cell].ty {
        CellType::F64 { clamp } => {
            if let Some(new_min) = parse_clamp_bound::<f64>(text) {
                clamp.min = new_min;
            }
        }
        CellType::I64 { clamp } => {
            if let Some(new_min) = parse_clamp_bound::<i64>(text) {
                clamp.min = new_min;
            }
        }
        CellType::Bool | CellType::Text => {
            debug_assert!(false, "set_clamp_min requires a numeric cell type");
        }
    }
}

/// Sets `cell`'s clamp maximum from `text`: empty text clears the bound,
/// text that parses as `cell`'s numeric type sets it, and anything else
/// leaves the existing bound unchanged (see `parse_clamp_bound`).
///
/// - Precondition: `cell` is a valid key in `doc.cells`.
/// - Precondition: `cell`'s type is `F64` or `I64` (has a clamp to set).
pub fn set_clamp_max(doc: &mut Document, cell: CellId, text: &str) {
    debug_assert!(doc.cells.contains_key(cell), "cell is not a valid key");
    match &mut doc.cells[cell].ty {
        CellType::F64 { clamp } => {
            if let Some(new_max) = parse_clamp_bound::<f64>(text) {
                clamp.max = new_max;
            }
        }
        CellType::I64 { clamp } => {
            if let Some(new_max) = parse_clamp_bound::<i64>(text) {
                clamp.max = new_max;
            }
        }
        CellType::Bool | CellType::Text => {
            debug_assert!(false, "set_clamp_max requires a numeric cell type");
        }
    }
}

/// Removes `node` (a canvas placement) from `doc`, cascading to keep the
/// document consistent:
/// - `node` is removed from every relationship group's `members`; any
///   relationship group left with zero members is itself deleted (via
///   [`crate::ops::relationships::delete_relationship_group`], which also
///   detaches it from any conditional group that referenced it).
/// - If this was the last [`crate::model::cell_node::CellNode`]
///   referencing its underlying [`Cell`], the `Cell` itself is removed too
///   — an orphaned cell with no placement anywhere would be invisible and
///   unrecoverable through the UI — and any conditional group whose
///   condition depends on that cell is removed as well (via
///   [`crate::ops::conditionals::delete_conditional_group`]), since a
///   conditional with no way to evaluate its own condition can't be kept
///   meaningfully.
///
/// - Precondition: `node` is a valid key in `doc.cell_nodes`.
///
/// - Complexity: O(n) in the total size of `doc` (relationship-group
///   members, conditional-group branches, and cell nodes).
pub fn delete_cell_node(doc: &mut Document, node: CellNodeId) {
    debug_assert!(doc.cell_nodes.contains_key(node), "node is not a valid key");
    let cell = doc.cell_nodes[node].cell;
    doc.cell_nodes.remove(node);

    let mut now_empty_groups = Vec::new();
    for (id, group) in &mut doc.relationship_groups {
        group.members.retain(|(n, _)| *n != node);
        if group.members.is_empty() {
            now_empty_groups.push(id);
        }
    }
    for group in now_empty_groups {
        crate::ops::relationships::delete_relationship_group(doc, group);
    }

    let cell_still_placed = doc.cell_nodes.values().any(|n| n.cell == cell);
    if !cell_still_placed {
        doc.cells.remove(cell);
        doc.cell_order.retain(|c| *c != cell);

        let dependent_conditionals: Vec<_> = doc
            .conditional_groups
            .iter()
            .filter(|(_, cond)| cond.condition.referenced_cells().contains(&cell))
            .map(|(id, _)| id)
            .collect();
        for conditional in dependent_conditionals {
            crate::ops::conditionals::delete_conditional_group(doc, conditional);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_cell_inserts_a_non_output_cell_with_no_restrict() {
        let mut doc = Document::new("demo");
        let id = add_cell(&mut doc, "width_pixels", CellType::i64());
        assert_eq!(doc.cells[id].name, "width_pixels");
        assert!(!doc.cells[id].output);
        assert!(doc.cells[id].restrict.is_none());
        assert_eq!(doc.cell_order, vec![id]);
    }

    #[test]
    fn add_cell_node_places_the_cell_at_the_position() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        let node = add_cell_node(&mut doc, cell, Point::new(10.0, 20.0));
        assert_eq!(doc.cell_nodes[node].cell, cell);
        assert_eq!(doc.cell_nodes[node].position, Point::new(10.0, 20.0));
    }

    #[test]
    fn set_output_updates_the_cells_output_flag() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        set_output(&mut doc, cell, true);
        assert!(doc.cells[cell].output);
    }

    #[test]
    fn set_restrict_updates_the_cells_restrict_text() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        set_restrict(&mut doc, cell, Some("_ > 0".to_string()));
        assert_eq!(doc.cells[cell].restrict.as_deref(), Some("_ > 0"));
    }

    #[test]
    fn set_name_updates_the_cells_name() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        set_name(&mut doc, cell, "w");
        assert_eq!(doc.cells[cell].name, "w");
    }

    #[test]
    fn set_name_is_visible_through_every_node_placing_the_shared_cell() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        let node_a = add_cell_node(&mut doc, cell, Point::new(0.0, 0.0));
        let node_b = add_cell_node(&mut doc, cell, Point::new(10.0, 0.0));

        set_name(&mut doc, cell, "w");

        assert_eq!(doc.cells[doc.cell_nodes[node_a].cell].name, "w");
        assert_eq!(doc.cells[doc.cell_nodes[node_b].cell].name, "w");
    }

    #[test]
    fn set_clamp_min_parses_and_sets_an_f64_bound() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        set_clamp_min(&mut doc, cell, "0.5");
        let CellType::F64 { clamp } = &doc.cells[cell].ty else {
            panic!("expected F64");
        };
        assert_eq!(clamp.min, Some(0.5));
    }

    #[test]
    fn set_clamp_min_parses_and_sets_an_i64_bound() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        set_clamp_min(&mut doc, cell, "10");
        let CellType::I64 { clamp } = &doc.cells[cell].ty else {
            panic!("expected I64");
        };
        assert_eq!(clamp.min, Some(10));
    }

    #[test]
    fn set_clamp_min_with_empty_text_clears_the_bound() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        set_clamp_min(&mut doc, cell, "10");
        set_clamp_min(&mut doc, cell, "");
        let CellType::I64 { clamp } = &doc.cells[cell].ty else {
            panic!("expected I64");
        };
        assert_eq!(clamp.min, None);
    }

    #[test]
    fn set_clamp_min_with_unparseable_text_leaves_the_bound_unchanged() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        set_clamp_min(&mut doc, cell, "10");
        set_clamp_min(&mut doc, cell, "-");
        let CellType::I64 { clamp } = &doc.cells[cell].ty else {
            panic!("expected I64");
        };
        assert_eq!(clamp.min, Some(10));
    }

    #[test]
    fn set_clamp_max_parses_and_sets_an_f64_bound() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        set_clamp_max(&mut doc, cell, "2.5");
        let CellType::F64 { clamp } = &doc.cells[cell].ty else {
            panic!("expected F64");
        };
        assert_eq!(clamp.max, Some(2.5));
    }

    #[test]
    fn set_clamp_max_with_empty_text_clears_the_bound() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        set_clamp_max(&mut doc, cell, "100");
        set_clamp_max(&mut doc, cell, "");
        let CellType::I64 { clamp } = &doc.cells[cell].ty else {
            panic!("expected I64");
        };
        assert_eq!(clamp.max, None);
    }

    #[test]
    fn set_clamp_max_with_unparseable_text_leaves_the_bound_unchanged() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        set_clamp_max(&mut doc, cell, "100");
        set_clamp_max(&mut doc, cell, "abc");
        let CellType::I64 { clamp } = &doc.cells[cell].ty else {
            panic!("expected I64");
        };
        assert_eq!(clamp.max, Some(100));
    }

    #[test]
    fn delete_cell_node_removes_just_the_node_when_another_node_shares_the_cell() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        let node_a = add_cell_node(&mut doc, cell, Point::new(0.0, 0.0));
        let node_b = add_cell_node(&mut doc, cell, Point::new(10.0, 0.0));

        delete_cell_node(&mut doc, node_a);

        assert!(!doc.cell_nodes.contains_key(node_a));
        assert!(doc.cell_nodes.contains_key(node_b));
        assert!(doc.cells.contains_key(cell), "cell should survive");
    }

    #[test]
    fn delete_cell_node_removes_the_cell_too_when_it_was_the_last_node() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        let node = add_cell_node(&mut doc, cell, Point::new(0.0, 0.0));

        delete_cell_node(&mut doc, node);

        assert!(!doc.cells.contains_key(cell));
        assert!(!doc.cell_order.contains(&cell));
    }

    #[test]
    fn delete_cell_node_removes_it_from_a_relationship_groups_members() {
        use crate::ops::relationships::create_relationship;

        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let c = add_cell(&mut doc, "c", CellType::i64());
        let c_node = add_cell_node(&mut doc, c, Point::new(20.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        crate::ops::relationships::add_member(&mut doc, group, c_node);

        delete_cell_node(&mut doc, c_node);

        assert_eq!(doc.relationship_groups[group].members.len(), 2);
    }

    #[test]
    fn delete_cell_node_cascades_to_delete_a_relationship_group_left_with_no_members() {
        use crate::ops::relationships::create_relationship;

        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        delete_cell_node(&mut doc, a_node);
        delete_cell_node(&mut doc, b_node);

        assert!(!doc.relationship_groups.contains_key(group));
        assert!(!doc.relationship_group_order.contains(&group));
    }

    #[test]
    fn delete_cell_node_cascades_to_delete_a_dependent_conditional_group() {
        use crate::model::conditional_group::ConditionExpr;
        use crate::ops::conditionals::add_conditional_with_formula;

        let mut doc = Document::new("demo");
        let flag = add_cell(&mut doc, "flag", CellType::Bool);
        let flag_node = add_cell_node(&mut doc, flag, Point::new(0.0, 0.0));
        let cond =
            add_conditional_with_formula(&mut doc, vec![flag], "flag", Point::new(0.0, 20.0));
        assert!(matches!(
            doc.conditional_groups[cond].condition,
            ConditionExpr::Formula { .. }
        ));

        delete_cell_node(&mut doc, flag_node);

        assert!(!doc.conditional_groups.contains_key(cond));
        assert!(!doc.conditional_group_order.contains(&cond));
    }
}
