//! Value cells in the property model bipartite graph.
//!
//! Cells are accessed exclusively through [`crate::sheet::Sheet`].

use std::any::{Any, TypeId};

use slotmap::new_key_type;

use crate::relationship::RelationshipId;

new_key_type! {
    /// A stable handle to a cell in a [`crate::sheet::Sheet`].
    pub struct CellId;
}

/// Internal storage for a single value cell.
// used in sheet.rs (Task 4)
#[allow(dead_code)]
pub(crate) struct CellData {
    /// The type-erased current value.
    pub(crate) value: Box<dyn Any>,
    /// The `TypeId` of the value, fixed at cell creation.
    pub(crate) type_id: TypeId,
    /// Monotonically increasing write-recency clock; incremented by `Sheet::write`.
    pub(crate) strength: u64,
    /// Set during `Sheet::propagate`; cleared by `Sheet::clear_changed`.
    pub(crate) changed: bool,
    /// Relationships that include this cell.
    pub(crate) adj: Vec<RelationshipId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_data_initial_state() {
        let data = CellData {
            value: Box::new(42_i32),
            type_id: TypeId::of::<i32>(),
            strength: 0,
            changed: false,
            adj: vec![],
        };
        assert_eq!(data.type_id, TypeId::of::<i32>());
        assert_eq!(data.strength, 0);
        assert!(!data.changed);
        assert!(data.adj.is_empty());
        assert_eq!(*data.value.downcast_ref::<i32>().unwrap(), 42);
    }

    #[test]
    fn cell_id_is_copy() {
        fn takes_copy<T: Copy>(_: T) {}
        takes_copy(CellId::default());
    }
}
