//! Terminal output cells in the property model bipartite graph.
//!
//! An output is a cell written by exactly one method, together with zero or more named
//! [`crate::requirement::Requirement`]s checked after every `Sheet::propagate`. An output's
//! cell is terminal: it can never be used as an input to another relationship,
//! conditional, requirement, or output. See [`crate::sheet::Sheet::add_output`].

use slotmap::new_key_type;

use crate::cell::CellId;
use crate::requirement::RequirementId;

new_key_type! {
    /// A stable handle to an output in a [`crate::sheet::Sheet`].
    pub struct OutputId;
}

/// Internal storage for a single output.
pub(crate) struct OutputData {
    /// The terminal cell this output writes.
    pub(crate) cell: CellId,
    /// This output's requirements, in declaration order.
    pub(crate) requirements: Vec<RequirementId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_id_is_copy() {
        fn takes_copy<T: Copy>(_: T) {}
        takes_copy(OutputId::default());
    }
}
