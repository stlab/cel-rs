//! Relationships and methods in the property model bipartite graph.
//!
//! Each relationship holds a list of [`Method`]s; the planner selects one
//! method per relationship at propagation time based on cell strength.

use std::any::{Any, TypeId};

use slotmap::new_key_type;

use crate::cell::CellId;

new_key_type! {
    /// A stable handle to a relationship in a [`crate::sheet::Sheet`].
    pub struct RelationshipId;
}

/// A single method within a relationship.
///
/// A method declares a disjoint partition of some cells into inputs and
/// outputs, plus a type-erased function that computes the outputs from the
/// inputs. TypeIds for inputs and outputs are stored alongside the function
/// and validated at [`crate::sheet::Sheet::add_relationship`] time.
#[allow(dead_code, clippy::type_complexity)]
pub struct Method {
    pub(crate) inputs: Vec<CellId>,
    pub(crate) outputs: Vec<CellId>,
    pub(crate) input_types: Vec<TypeId>,
    pub(crate) output_types: Vec<TypeId>,
    pub(crate) function: Box<dyn Fn(&[&dyn Any]) -> Result<Vec<Box<dyn Any>>, anyhow::Error>>,
}

/// Internal storage for a relationship.
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

    #[test]
    fn cell_id_is_copy() {
        fn takes_copy<T: Copy>(_: T) {}
        takes_copy(CellId::default());
    }
}
