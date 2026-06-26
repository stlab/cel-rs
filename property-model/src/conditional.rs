//! Conditionals in the property model: match-cell branching.
//!
//! Each conditional binds to one cell (the *match cell*) and holds a list of
//! branches. During propagation the branch whose keys contain the current match
//! cell value is activated; its relationships participate in the general planning
//! pass.

use std::any::Any;

use slotmap::new_key_type;

use crate::{cell::CellId, relationship::RelationshipId};

new_key_type! {
    /// A stable handle to a conditional in a [`crate::sheet::Sheet`].
    pub struct ConditionalId;
}

/// One arm of a [`ConditionalData`]: a set of key values and the relationships
/// to activate when the match cell equals any key.
#[allow(dead_code)]
pub(crate) struct Branch {
    /// Type-erased key values; each `TypeId` matches the match cell's registered type.
    pub(crate) keys: Vec<Box<dyn Any>>,
    /// Relationships activated when any key matches.
    pub(crate) relationships: Vec<RelationshipId>,
}

/// Internal storage for a conditional.
#[allow(dead_code)]
pub(crate) struct ConditionalData {
    /// The cell whose value is tested.
    pub(crate) cell: CellId,
    /// Branches evaluated in definition order; first match wins.
    pub(crate) branches: Vec<Branch>,
    /// Relationships activated when no branch matches. Empty means no default.
    pub(crate) default: Vec<RelationshipId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditional_id_is_copy() {
        fn takes_copy<T: Copy>(_: T) {}
        takes_copy(ConditionalId::default());
    }
}
