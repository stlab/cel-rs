//! Canvas placements of cells (see [`crate::model::cell::Cell`]).

use serde::{Deserialize, Serialize};
use slotmap::new_key_type;

use crate::model::cell::CellId;
use crate::model::geometry::Point;

new_key_type! {
    /// A stable handle to a [`CellNode`] in a
    /// [`crate::model::document::Document`].
    pub struct CellNodeId;
}

/// A visual placement of a [`Cell`](crate::model::cell::Cell) on the
/// canvas.
///
/// Multiple `CellNode`s may reference the same [`CellId`] — "two instances
/// of the same value in the graph" — each with its own [`Point`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CellNode {
    /// The cell being placed on the canvas.
    pub cell: CellId,
    /// The position of this placement on the canvas.
    pub position: Point,
}

impl CellNode {
    /// Creates a node placing `cell` at `position`.
    #[must_use]
    pub fn new(cell: CellId, position: Point) -> Self {
        CellNode { cell, position }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    #[test]
    fn new_sets_cell_and_position() {
        let mut cells: SlotMap<CellId, ()> = SlotMap::with_key();
        let cell = cells.insert(());
        let node = CellNode::new(cell, Point::new(1.0, 2.0));
        assert_eq!(node.cell, cell);
        assert_eq!(node.position, Point::new(1.0, 2.0));
    }

    #[test]
    fn two_nodes_of_the_same_cell_at_different_positions_are_distinct() {
        let mut cells: SlotMap<CellId, ()> = SlotMap::with_key();
        let cell = cells.insert(());
        let a = CellNode::new(cell, Point::new(0.0, 0.0));
        let b = CellNode::new(cell, Point::new(10.0, 0.0));
        assert_eq!(a.cell, b.cell);
        assert_ne!(a, b);
    }
}
