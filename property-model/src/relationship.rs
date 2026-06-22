//! Relationships and methods in the property model bipartite graph.
//!
//! Each relationship holds a list of [`Method`]s; the planner selects one
//! method per relationship at propagation time based on cell strength.

use slotmap::new_key_type;

use crate::cell::CellId;

new_key_type! {
    /// A stable handle to a relationship in a [`crate::sheet::Sheet`].
    pub struct RelationshipId;
}

/// A single method within a relationship.
///
/// Constructors (`new`, `from_fn_1_1`, `from_fn_2_1`) are added in the next task.
pub struct Method {}

/// Internal storage for a relationship.
// used in sheet.rs (Task 4)
#[allow(dead_code)]
pub(crate) struct RelationshipData {
    pub(crate) methods: Vec<Method>,
    /// Union of all cell IDs referenced by any method in this relationship.
    pub(crate) adj: Vec<CellId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relationship_id_is_copy() {
        fn takes_copy<T: Copy>(_: T) {}
        // RelationshipId must be Copy so it can be stored in adjacency Vecs cheaply.
        // This test fails to compile if RelationshipId is not Copy.
        takes_copy(RelationshipId::default());
    }
}
