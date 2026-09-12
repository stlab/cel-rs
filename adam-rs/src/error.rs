//! The `Error` type returned by all fallible operations in this crate.

use crate::cell::CellId;
use crate::relationship::RelationshipId;
use std::any::TypeId;

/// Identifies a single sheet component an error implicates, for translating back to a
/// source location. An `Error`'s `sites()` is an ordered list of these: the primary
/// culprit first, followed by related context (e.g. the other method(s) and cell(s)
/// involved in a structural conflict, or the remaining members of a cycle).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorSite {
    /// Index into the `methods` `Vec` passed to `add_relationship`, before any
    /// `RelationshipId` exists for it.
    MethodIndex(usize),
    /// A method within an already-registered relationship.
    Method(RelationshipId, usize),
    /// A relationship as a whole (no single method within it is implicated).
    Relationship(RelationshipId),
    /// A single cell.
    Cell(CellId),
}

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
        /// The method this mismatch was detected in, if known. Empty when unknown.
        sites: Vec<ErrorSite>,
    },

    /// A `CellId` or `RelationshipId` was not found in the sheet.
    InvalidId,

    /// No valid method assignment exists (overconstrained).
    Conflict {
        /// A subset-minimal group of relationships that together admit no valid
        /// method assignment: removing any member of the group makes the remainder
        /// feasible. Every entry is `ErrorSite::Relationship`.
        sites: Vec<ErrorSite>,
    },

    /// The selected methods form a cycle.
    Cycle {
        /// The cycle's members in loop-traversal order, alternating `Relationship` and
        /// `Cell` entries for the length of the loop. Either kind may be `sites[0]`:
        /// the loop is reported starting at the first node revisited during recovery,
        /// which is not necessarily a `Relationship`.
        sites: Vec<ErrorSite>,
    },

    /// A method's function returned an error during execution.
    MethodFailed {
        /// The underlying error the method's function (or a requirement's/conditional's
        /// expression function) returned.
        error: anyhow::Error,
        /// The method this failure originated from, if known. Empty when unknown.
        sites: Vec<ErrorSite>,
    },

    /// A method is structurally invalid (e.g. the outputs list is empty, a
    /// relationship's methods reference different sets of cells, or two methods in
    /// a relationship share an identical output set). A method with no inputs is
    /// not an error: it defines a fixed point (a constant) rather than a derivation.
    InvalidMethod {
        /// The method (by index within the `Vec` passed to `add_relationship`) that's
        /// invalid, if known. Empty when `methods` itself is empty.
        sites: Vec<ErrorSite>,
    },

    /// Two methods in the same relationship have `inputs ∪ outputs` sets that don't
    /// match. Every method in a relationship must reference exactly the same set of
    /// cells.
    MismatchedMethodCells {
        /// `sites[0]` is the first method (by index within the `Vec` passed to
        /// `add_relationship`) whose cell set diverges from method 0's; `sites[1]` is
        /// always `MethodIndex(0)`, the baseline every other method's cell set is
        /// compared against; further entries are the cells in the symmetric
        /// difference of the two methods' cell sets.
        sites: Vec<ErrorSite>,
    },

    /// A method's own `outputs` list names a cell more than once, or two methods in
    /// the same relationship have identical `outputs` sets.
    DuplicateMethodOutputs {
        /// `sites[0]` is the method (by index within the `Vec` passed to
        /// `add_relationship`) whose output set collided. For a method's own outputs
        /// repeating a cell, further entries are the repeated cell(s). For two methods
        /// sharing an output set, `sites[1]` is the earlier method's index and further
        /// entries are the shared output cell(s).
        sites: Vec<ErrorSite>,
    },

    /// A conditional is structurally invalid: the cell was not found, a referenced
    /// relationship was not found, a branch relationship that shares a cell with the match
    /// cell or any of its unconditional upstream contributors has more than one method, a
    /// relationship appears in more than one conditional branch, a branch key's type does
    /// not match the cell's registered type, or a branch has no keys.
    InvalidConditional {
        /// Empty for the expression-output type-mismatch case; the match-cell
        /// type-mismatch case names the cell (`sites = [Cell(match_cell)]`). The
        /// duplicate-relationship and multi-method-branch cases name the offending
        /// relationship(s) (and, for the multi-method case, the contributing cell).
        /// Empty for the missing-relationship and empty-branch-keys cases.
        sites: Vec<ErrorSite>,
    },

    /// An `add_out` call is structurally invalid: the writer method does not have
    /// exactly one output cell.
    InvalidOutput,

    /// A relationship or conditional attempted to claim a `Source`-kind cell as a
    /// method's output, `write()` targeted an `Out`-kind cell, or `add_out` targeted a
    /// cell that is already `Source`/`Out` kind or already claimed as another method's
    /// output.
    InvalidCellKind {
        /// `sites[0]` is the method (by index within the `Vec` passed to
        /// `add_relationship`) whose output cell has the wrong kind, if the error
        /// originated there; `sites[1]` is that offending cell. Empty when the error
        /// did not originate from `add_relationship`.
        sites: Vec<ErrorSite>,
    },

    /// An `add_filter` call is structurally invalid: the cell already has a filter,
    /// the filter's own value type does not match the cell's registered type, or the
    /// filter's own argument list names `cell` itself. (An unknown cell or an
    /// argument-cell type mismatch use the shared `InvalidId`/`TypeMismatch` variants
    /// instead — `add_filter` has no cell-kind restriction, so it never returns
    /// `InvalidCellKind`.)
    InvalidFilter,

    /// An `add_requirement` call is structurally invalid: `name` is `Some` and `cell`
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
    FilterCycle {
        /// The cycle's members in loop-traversal order, alternating `Relationship` and
        /// `Cell` entries (either kind may be `sites[0]`, for the same reason as
        /// `Cycle::sites`), including the filtered `Cell` and the `Cell` → `Cell` edge
        /// from its filter argument.
        sites: Vec<ErrorSite>,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TypeMismatch {
                expected, found, ..
            } => {
                write!(f, "type mismatch: expected {expected:?}, found {found:?}")
            }
            Error::InvalidId => write!(f, "invalid cell or relationship id"),
            Error::Conflict { .. } => write!(f, "no valid method assignment (overconstrained)"),
            Error::Cycle { .. } => write!(f, "selected methods form a cycle"),
            Error::MethodFailed { error, .. } => write!(f, "method execution failed: {error}"),
            Error::InvalidMethod { .. } => write!(f, "method is structurally invalid"),
            Error::MismatchedMethodCells { .. } => write!(
                f,
                "methods in a relationship must reference the same set of cells"
            ),
            Error::DuplicateMethodOutputs { .. } => write!(
                f,
                "a method's outputs must be duplicate-free, and no two methods in a \
                 relationship may share an outputs set"
            ),
            Error::InvalidConditional { .. } => write!(f, "conditional is structurally invalid"),
            Error::InvalidOutput => write!(f, "output is structurally invalid"),
            Error::InvalidCellKind { .. } => {
                write!(f, "cell's kind does not permit this operation")
            }
            Error::InvalidFilter => write!(f, "filter is structurally invalid"),
            Error::InvalidRequirement => write!(f, "requirement is structurally invalid"),
            Error::FilterCycle { .. } => write!(
                f,
                "a filter's argument dependency closes a cycle with the selected methods"
            ),
        }
    }
}

impl std::error::Error for Error {
    /// Returns the underlying `anyhow::Error` source for `MethodFailed`.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let Error::MethodFailed { error, .. } = self {
            Some(error.as_ref())
        } else {
            None
        }
    }
}

impl Error {
    /// The source components this error implicates, primary first.
    ///
    /// `sites[0]` is the primary culprit; further entries are ordered related context
    /// (for a cycle, the remaining members in loop order). Empty for variants that never
    /// track a site, or when none applies to this occurrence.
    pub fn sites(&self) -> &[ErrorSite] {
        match self {
            Error::TypeMismatch { sites, .. }
            | Error::MethodFailed { sites, .. }
            | Error::InvalidMethod { sites }
            | Error::MismatchedMethodCells { sites }
            | Error::DuplicateMethodOutputs { sites }
            | Error::InvalidCellKind { sites }
            | Error::Conflict { sites }
            | Error::Cycle { sites }
            | Error::FilterCycle { sites }
            | Error::InvalidConditional { sites } => sites,
            _ => &[],
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
        let e = Error::TypeMismatch {
            expected,
            found,
            sites: vec![],
        };
        match e {
            Error::TypeMismatch {
                expected: e,
                found: f,
                ..
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
            sites: vec![],
        };
        assert!(err.to_string().contains("type mismatch"));
    }

    #[test]
    fn invalid_id_display_contains_invalid() {
        assert!(Error::InvalidId.to_string().contains("invalid"));
    }

    #[test]
    fn conflict_display_contains_overconstrained() {
        assert!(
            Error::Conflict { sites: vec![] }
                .to_string()
                .contains("overconstrained")
        );
    }

    #[test]
    fn cycle_display_contains_cycle() {
        assert!(Error::Cycle { sites: vec![] }.to_string().contains("cycle"));
    }

    #[test]
    fn method_failed_display_contains_source_message() {
        let err = Error::MethodFailed {
            error: anyhow::anyhow!("division by zero"),
            sites: vec![],
        };
        assert!(err.to_string().contains("division by zero"));
    }

    #[test]
    fn invalid_method_display_contains_invalid() {
        assert!(
            Error::InvalidMethod { sites: vec![] }
                .to_string()
                .contains("invalid")
        );
    }

    #[test]
    fn error_implements_std_error() {
        fn takes_error(_: &dyn std::error::Error) {}
        takes_error(&Error::InvalidId);
        takes_error(&Error::Conflict { sites: vec![] });
    }

    #[test]
    fn method_failed_source_returns_some() {
        let err = Error::MethodFailed {
            error: anyhow::anyhow!("inner"),
            sites: vec![],
        };
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn non_method_failed_variants_have_no_source() {
        assert!(std::error::Error::source(&Error::InvalidId).is_none());
        assert!(std::error::Error::source(&Error::Conflict { sites: vec![] }).is_none());
        assert!(std::error::Error::source(&Error::Cycle { sites: vec![] }).is_none());
        assert!(std::error::Error::source(&Error::InvalidMethod { sites: vec![] }).is_none());
        assert!(
            std::error::Error::source(&Error::TypeMismatch {
                expected: std::any::TypeId::of::<i32>(),
                found: std::any::TypeId::of::<f64>(),
                sites: vec![],
            })
            .is_none()
        );
        assert!(
            std::error::Error::source(&Error::MismatchedMethodCells { sites: vec![] }).is_none()
        );
        assert!(
            std::error::Error::source(&Error::DuplicateMethodOutputs { sites: vec![] }).is_none()
        );
    }

    #[test]
    fn invalid_conditional_display_contains_conditional() {
        assert!(
            Error::InvalidConditional { sites: vec![] }
                .to_string()
                .contains("conditional")
        );
    }

    #[test]
    fn invalid_conditional_has_no_source() {
        assert!(std::error::Error::source(&Error::InvalidConditional { sites: vec![] }).is_none());
    }

    #[test]
    fn mismatched_method_cells_display_contains_cells() {
        assert!(
            Error::MismatchedMethodCells { sites: vec![] }
                .to_string()
                .contains("cells")
        );
    }

    #[test]
    fn duplicate_method_outputs_display_contains_outputs() {
        assert!(
            Error::DuplicateMethodOutputs { sites: vec![] }
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
        assert!(
            Error::InvalidCellKind { sites: vec![] }
                .to_string()
                .contains("kind")
        );
    }

    // Regression guard for https://github.com/stlab/cel-rs/issues/166: the message used to
    // claim the cell "belongs to a terminal output", which stopped being true once `out` cells
    // became usable as inputs, and never covered the `Source`-kind case at all.
    #[test]
    fn invalid_cell_kind_display_does_not_mention_terminal() {
        assert!(
            !Error::InvalidCellKind { sites: vec![] }
                .to_string()
                .contains("terminal")
        );
    }

    #[test]
    fn invalid_cell_kind_has_no_source() {
        assert!(std::error::Error::source(&Error::InvalidCellKind { sites: vec![] }).is_none());
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
        assert!(
            Error::FilterCycle { sites: vec![] }
                .to_string()
                .contains("cycle")
        );
    }

    #[test]
    fn filter_cycle_has_no_source() {
        assert!(std::error::Error::source(&Error::FilterCycle { sites: vec![] }).is_none());
    }

    #[test]
    fn error_site_variants_are_distinct() {
        let a = ErrorSite::MethodIndex(0);
        let b = ErrorSite::Method(RelationshipId::default(), 0);
        let c = ErrorSite::Relationship(RelationshipId::default());
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_eq!(a, ErrorSite::MethodIndex(0));
    }

    #[test]
    fn sites_is_empty_for_a_siteless_variant() {
        assert!(Error::InvalidId.sites().is_empty());
    }

    #[test]
    fn sites_returns_the_recorded_site() {
        let rid = RelationshipId::default();
        let e = Error::MethodFailed {
            error: anyhow::anyhow!("x"),
            sites: vec![ErrorSite::Method(rid, 2)],
        };
        assert_eq!(e.sites(), &[ErrorSite::Method(rid, 2)]);
    }
}
