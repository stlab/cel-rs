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
pub(crate) struct CellData {
    /// The value from the most recent `write()`/`add_cell`. Never written by
    /// `Sheet::propagate`; self-referencing methods and conditionally forced
    /// relationships read/write around this field via `derived` instead.
    pub(crate) source: Box<dyn Any>,
    /// The value most recently produced by a method this round, if this cell was
    /// shadowed (a self-referencing output, or a pure output of a conditionally
    /// registered relationship). Reset to `None` for every cell at the start of
    /// every `Sheet::propagate` call, before planning begins.
    pub(crate) derived: Option<Box<dyn Any>>,
    /// The `TypeId` of the value, fixed at cell creation.
    pub(crate) type_id: TypeId,
    /// Write-recency strength. High-order bit (bit 63) is set for cells that have been
    /// written or created via `add_cell`. Derived cells (outputs of selected methods)
    /// receive strengths with bit 63 clear, assigned during the post-processing pass.
    pub(crate) strength: u64,
    /// Set during `Sheet::propagate`; cleared by `Sheet::clear_changed`.
    pub(crate) changed: bool,
    /// Relationships that include this cell.
    pub(crate) adj: Vec<RelationshipId>,
    /// Type-erased equality: returns `true` iff both arguments hold equal values of the
    /// cell's registered type. Captured at `add_cell` time from the concrete `T: PartialEq`.
    pub(crate) eq_fn: fn(&dyn Any, &dyn Any) -> bool,
}

impl CellData {
    /// Returns the effective current value: `derived` if present, else `source`.
    pub(crate) fn effective(&self) -> &dyn Any {
        self.derived.as_deref().unwrap_or(self.source.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_data_initial_state() {
        let data = CellData {
            source: Box::new(42_i32),
            derived: None,
            type_id: TypeId::of::<i32>(),
            strength: 0,
            changed: false,
            adj: vec![],
            eq_fn: |a, b| a.downcast_ref::<i32>() == b.downcast_ref::<i32>(),
        };
        assert_eq!(data.type_id, TypeId::of::<i32>());
        assert_eq!(data.strength, 0);
        assert!(!data.changed);
        assert!(data.adj.is_empty());
        assert!(data.derived.is_none());
        assert_eq!(*data.source.downcast_ref::<i32>().unwrap(), 42);
        assert_eq!(*data.effective().downcast_ref::<i32>().unwrap(), 42);
        let x: i32 = 42;
        let y: i32 = 99;
        assert!((data.eq_fn)(&x, &x));
        assert!(!(data.eq_fn)(&x, &y));
    }

    #[test]
    fn cell_id_is_copy() {
        fn takes_copy<T: Copy>(_: T) {}
        takes_copy(CellId::default());
    }
}
