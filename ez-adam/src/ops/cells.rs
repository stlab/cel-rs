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
}
