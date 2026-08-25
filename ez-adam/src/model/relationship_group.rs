//! Relationship groups: an alternative method for deriving one or more
//! bound cells.

use serde::{Deserialize, Serialize};
use slotmap::new_key_type;

use crate::model::cell_node::CellNodeId;
use crate::model::geometry::Point;

new_key_type! {
    /// A stable handle to a [`RelationshipGroup`] in a
    /// [`crate::model::document::Document`].
    pub struct RelationshipGroupId;
}

/// A group of cell bindings representing one `.adm2` `relationship { ... }`
/// block (or a branch entry inside a `conditional`).
///
/// `display_name` (e.g. `"r1"`) is UI bookkeeping only — `.adm2`
/// relationship blocks are anonymous, so it is never emitted by
/// `generate_adm2`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipGroup {
    /// The display name for the relationship group (e.g., `"r1"`) — UI
    /// bookkeeping only, never emitted to `.adm2`.
    pub display_name: String,
    /// The position of the relationship group on the canvas.
    pub position: Point,
    /// One entry per bound cell: the node gives the edge's canvas
    /// endpoint, the `String` is that member's RHS formula text (CEL
    /// source, empty until the user fills it in).
    pub members: Vec<(CellNodeId, String)>,
}

impl RelationshipGroup {
    /// Creates an empty relationship group at `position` with no members.
    #[must_use]
    pub fn new(display_name: impl Into<String>, position: Point) -> Self {
        RelationshipGroup {
            display_name: display_name.into(),
            position,
            members: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_no_members() {
        let group = RelationshipGroup::new("r1", Point::new(0.0, 0.0));
        assert_eq!(group.display_name, "r1");
        assert!(group.members.is_empty());
    }
}
