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

    /// A method is structurally invalid (e.g. inputs ∩ outputs is non-empty,
    /// or the outputs list is empty).
    InvalidMethod,
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
    }
}
