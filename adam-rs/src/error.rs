//! The `Error` type returned by all fallible operations in this crate.

use std::any::TypeId;

/// Errors returned by `Sheet` operations and propagation.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A value's TypeId did not match the cell's registered TypeId.
    ///
    /// - `expected`: the TypeId registered when the cell was created.
    /// - `found`: the TypeId of the value or declaration supplied by the caller.
    TypeMismatch {
        /// The TypeId registered when the cell was created.
        expected: TypeId,
        /// The TypeId of the value or declaration supplied by the caller.
        found: TypeId,
    },

    /// A `CellId` or `RelationshipId` was not found in the sheet.
    InvalidId,

    /// No valid method assignment exists (overconstrained).
    Conflict,

    /// The selected methods form a cycle.
    Cycle,

    /// A method's function returned an error during execution.
    MethodFailed(anyhow::Error),

    /// A method is structurally invalid (e.g. the outputs list is empty, a
    /// relationship's methods reference different sets of cells, or two methods in
    /// a relationship share an identical output set). A method with no inputs is
    /// not an error: it defines a fixed point (a constant) rather than a derivation.
    InvalidMethod,

    /// Two methods in the same relationship have `inputs ∪ outputs` sets that don't
    /// match. Every method in a relationship must reference exactly the same set of
    /// cells.
    MismatchedMethodCells,

    /// A method's own `outputs` list names a cell more than once, or two methods in
    /// the same relationship have identical `outputs` sets.
    DuplicateMethodOutputs,

    /// A conditional is structurally invalid: the cell was not found, a referenced
    /// relationship was not found, a branch relationship that shares a cell with the match
    /// cell or any of its unconditional upstream contributors has more than one method, a
    /// relationship appears in more than one conditional branch, a branch key's type does
    /// not match the cell's registered type, or a branch has no keys.
    InvalidConditional,

    /// An `add_out` call is structurally invalid: the writer method does not have
    /// exactly one output cell.
    InvalidOutput,

    /// A relationship or conditional attempted to claim a `Source`-kind cell as a
    /// method's output, `write()` targeted an `Out`-kind cell, or `add_out` targeted a
    /// cell that is already `Source`/`Out` kind or already claimed as another method's
    /// output.
    InvalidCellKind,

    /// An `add_filter` call is structurally invalid: the cell already has a filter, the
    /// filter's own value type does not match the cell's registered type, or the
    /// filter's own argument list names `cell` itself. (An unknown cell, a terminal
    /// cell, or an argument-cell type mismatch use the shared
    /// `InvalidId`/`InvalidCellKind`/`TypeMismatch` variants instead, matching
    /// `add_relationship`/`add_conditional`'s existing convention.)
    InvalidFilter,

    /// An `add_requirement` call is structurally invalid: the name is empty, `cell`
    /// already has a same-named requirement, or (on a `Cell`/`Source` kind cell)
    /// evaluating the requirement against current values returns `Ok(false)`.
    InvalidRequirement,

    /// The combined dependency digraph — relationship edges plus a filtered source
    /// cell's argument edges (see `Sheet::propagate`'s planning pass) — has a
    /// non-trivial strongly connected component that is not purely a relationship
    /// cycle (that case is `Error::Cycle`). `release::resolve` guarantees the
    /// relationship-only subgraph is acyclic but has no visibility into filter edges,
    /// so this is sound but incomplete: a different, equally-valid relationship
    /// assignment might have avoided the cycle. See issue #153.
    FilterCycle,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TypeMismatch { expected, found } => {
                write!(f, "type mismatch: expected {expected:?}, found {found:?}")
            }
            Error::InvalidId => write!(f, "invalid cell or relationship id"),
            Error::Conflict => write!(f, "no valid method assignment (overconstrained)"),
            Error::Cycle => write!(f, "selected methods form a cycle"),
            Error::MethodFailed(e) => write!(f, "method execution failed: {e}"),
            Error::InvalidMethod => write!(f, "method is structurally invalid"),
            Error::MismatchedMethodCells => write!(
                f,
                "methods in a relationship must reference the same set of cells"
            ),
            Error::DuplicateMethodOutputs => write!(
                f,
                "a method's outputs must be duplicate-free, and no two methods in a \
                 relationship may share an outputs set"
            ),
            Error::InvalidConditional => write!(f, "conditional is structurally invalid"),
            Error::InvalidOutput => write!(f, "output is structurally invalid"),
            Error::InvalidCellKind => write!(f, "cell's kind does not permit this operation"),
            Error::InvalidFilter => write!(f, "filter is structurally invalid"),
            Error::InvalidRequirement => write!(f, "requirement is structurally invalid"),
            Error::FilterCycle => write!(
                f,
                "a filter's argument dependency closes a cycle with the selected methods"
            ),
        }
    }
}

impl std::error::Error for Error {
    /// Returns the underlying `anyhow::Error` source for `MethodFailed`.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let Error::MethodFailed(e) = self {
            Some(e.as_ref())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_mismatch_fields_convention() {
        use std::any::TypeId;
        let expected = TypeId::of::<i32>();
        let found = TypeId::of::<f64>();
        let e = Error::TypeMismatch { expected, found };
        match e {
            Error::TypeMismatch {
                expected: e,
                found: f,
            } => {
                assert_eq!(e, TypeId::of::<i32>());
                assert_eq!(f, TypeId::of::<f64>());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn type_mismatch_display_contains_type_mismatch() {
        let err = Error::TypeMismatch {
            expected: TypeId::of::<i32>(),
            found: TypeId::of::<f64>(),
        };
        assert!(err.to_string().contains("type mismatch"));
    }

    #[test]
    fn invalid_id_display_contains_invalid() {
        assert!(Error::InvalidId.to_string().contains("invalid"));
    }

    #[test]
    fn conflict_display_contains_overconstrained() {
        assert!(Error::Conflict.to_string().contains("overconstrained"));
    }

    #[test]
    fn cycle_display_contains_cycle() {
        assert!(Error::Cycle.to_string().contains("cycle"));
    }

    #[test]
    fn method_failed_display_contains_source_message() {
        let err = Error::MethodFailed(anyhow::anyhow!("division by zero"));
        assert!(err.to_string().contains("division by zero"));
    }

    #[test]
    fn invalid_method_display_contains_invalid() {
        assert!(Error::InvalidMethod.to_string().contains("invalid"));
    }

    #[test]
    fn error_implements_std_error() {
        fn takes_error(_: &dyn std::error::Error) {}
        takes_error(&Error::InvalidId);
        takes_error(&Error::Conflict);
    }

    #[test]
    fn method_failed_source_returns_some() {
        let err = Error::MethodFailed(anyhow::anyhow!("inner"));
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn non_method_failed_variants_have_no_source() {
        assert!(std::error::Error::source(&Error::InvalidId).is_none());
        assert!(std::error::Error::source(&Error::Conflict).is_none());
        assert!(std::error::Error::source(&Error::Cycle).is_none());
        assert!(std::error::Error::source(&Error::InvalidMethod).is_none());
        assert!(
            std::error::Error::source(&Error::TypeMismatch {
                expected: std::any::TypeId::of::<i32>(),
                found: std::any::TypeId::of::<f64>(),
            })
            .is_none()
        );
        assert!(std::error::Error::source(&Error::MismatchedMethodCells).is_none());
        assert!(std::error::Error::source(&Error::DuplicateMethodOutputs).is_none());
    }

    #[test]
    fn invalid_conditional_display_contains_conditional() {
        assert!(
            Error::InvalidConditional
                .to_string()
                .contains("conditional")
        );
    }

    #[test]
    fn invalid_conditional_has_no_source() {
        assert!(std::error::Error::source(&Error::InvalidConditional).is_none());
    }

    #[test]
    fn mismatched_method_cells_display_contains_cells() {
        assert!(Error::MismatchedMethodCells.to_string().contains("cells"));
    }

    #[test]
    fn duplicate_method_outputs_display_contains_outputs() {
        assert!(
            Error::DuplicateMethodOutputs
                .to_string()
                .contains("outputs")
        );
    }

    #[test]
    fn invalid_output_display_contains_invalid() {
        assert!(Error::InvalidOutput.to_string().contains("invalid"));
    }

    #[test]
    fn invalid_output_has_no_source() {
        assert!(std::error::Error::source(&Error::InvalidOutput).is_none());
    }

    #[test]
    fn invalid_cell_kind_display_contains_kind() {
        assert!(Error::InvalidCellKind.to_string().contains("kind"));
    }

    // Regression guard for https://github.com/stlab/cel-rs/issues/166: the message used to
    // claim the cell "belongs to a terminal output", which stopped being true once `out` cells
    // became usable as inputs, and never covered the `Source`-kind case at all.
    #[test]
    fn invalid_cell_kind_display_does_not_mention_terminal() {
        assert!(!Error::InvalidCellKind.to_string().contains("terminal"));
    }

    #[test]
    fn invalid_cell_kind_has_no_source() {
        assert!(std::error::Error::source(&Error::InvalidCellKind).is_none());
    }

    #[test]
    fn invalid_filter_display_contains_filter() {
        assert!(Error::InvalidFilter.to_string().contains("filter"));
    }

    #[test]
    fn invalid_filter_has_no_source() {
        assert!(std::error::Error::source(&Error::InvalidFilter).is_none());
    }

    #[test]
    fn invalid_requirement_display_contains_requirement() {
        assert!(
            Error::InvalidRequirement
                .to_string()
                .contains("requirement")
        );
    }

    #[test]
    fn invalid_requirement_has_no_source() {
        assert!(std::error::Error::source(&Error::InvalidRequirement).is_none());
    }

    #[test]
    fn filter_cycle_display_contains_cycle() {
        assert!(Error::FilterCycle.to_string().contains("cycle"));
    }

    #[test]
    fn filter_cycle_has_no_source() {
        assert!(std::error::Error::source(&Error::FilterCycle).is_none());
    }
}
