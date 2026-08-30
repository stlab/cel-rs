//! Value cells in the property model bipartite graph.
//!
//! Cells are accessed exclusively through [`crate::sheet::Sheet`].

use std::any::{Any, TypeId};

use slotmap::new_key_type;

use crate::filter::FilterData;
use crate::relationship::RelationshipId;
use crate::requirement::RequirementId;

new_key_type! {
    /// A stable handle to a cell in a [`crate::sheet::Sheet`].
    pub struct CellId;
}

/// A cell's fixed role in the planner's per-round source/derived assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    /// May be a source or derived, chosen per round by the planner. Default kind.
    Cell,
    /// Always a source: never claimable as any method's output.
    Source,
    /// Always derived by exactly one fixed writer method; never `write()`-able.
    Out,
}

/// Internal storage for a single value cell.
pub(crate) struct CellData {
    /// The value from the most recent `write()`/`add_cell`. Also written directly by
    /// `Sheet::propagate` for outputs that aren't shadowed (the common case: an ordinary
    /// derived cell behaves exactly as it did before `derived` existed). Left untouched by
    /// `propagate()` only for outputs that *are* shadowed — self-referencing methods and
    /// conditionally forced relationships write those into `derived` instead, which is what
    /// keeps this field holding the original value for exactly those cells.
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
    /// This cell's filter, if one is attached via `Sheet::add_filter`. At most one per
    /// cell.
    pub(crate) filter: Option<FilterData>,
    /// This cell's fixed role in the planner's per-round source/derived assignment.
    pub(crate) kind: CellKind,
    /// This cell's requirements, in attachment order. Empty for most cells.
    #[allow(dead_code)]
    pub(crate) requirements: Vec<RequirementId>,
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
            filter: None,
            kind: CellKind::Cell,
            requirements: Vec::new(),
        };
        assert_eq!(data.type_id, TypeId::of::<i32>());
        assert_eq!(data.strength, 0);
        assert!(!data.changed);
        assert!(data.adj.is_empty());
        assert!(data.derived.is_none());
        assert_eq!(*data.source.downcast_ref::<i32>().unwrap(), 42);
        assert_eq!(*data.effective().downcast_ref::<i32>().unwrap(), 42);
        assert_eq!(data.kind, CellKind::Cell);
        assert!(data.requirements.is_empty());
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
