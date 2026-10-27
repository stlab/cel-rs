//! Construction and binding failures without language source dependencies.

use std::any::TypeId;
use std::fmt;

use super::{CellSlot, RelationshipSlot, TemplateId};
use crate::{CellId, Error};

/// Identifies a malformed recipe, binding, factory, or native graph construction.
///
/// Binding and declared-structure errors precede destination mutation. Factory-result
/// validation, allocation and native construction failures may leave partial structure;
/// the sheet remains safe to discard. See [`super::SheetTemplate::instantiate`].
#[derive(Debug)]
pub enum TemplateError {
    /// A binding names no formal parameter.
    UnknownParameter {
        /// The unrecognized binding name.
        name: String,
    },
    /// A formal parameter has no binding.
    MissingParameter {
        /// The unsupplied formal name.
        name: String,
    },
    /// A formal name is declared or bound more than once.
    DuplicateParameter {
        /// The repeated declaration or binding name.
        name: String,
    },
    /// Two formals bind the same actual cell.
    AliasedArgument {
        /// The first formal bound to the actual.
        first: String,
        /// The second formal bound to the same actual.
        second: String,
    },
    /// A local handle belongs to another recipe.
    ForeignHandle {
        /// The local handle from a different owner.
        slot: CellSlot,
    },
    /// An actual handle is absent from the destination.
    InvalidActual {
        /// The formal receiving the invalid actual.
        name: String,
        /// The absent live handle.
        actual: CellId,
    },
    /// A binding's concrete type differs from its formal declaration.
    TypeMismatch {
        /// The formal receiving the wrongly typed argument.
        name: String,
        /// The formal's declared concrete type.
        expected: TypeId,
        /// The actual's registered concrete type.
        found: TypeId,
    },
    /// An actual is a caller-owned out cell, unsupported even as an input.
    OutArgument {
        /// The formal receiving the out cell.
        name: String,
        /// The unsupported caller-owned out cell.
        actual: CellId,
    },
    /// A relationship has no methods.
    EmptyRelationship,
    /// A component containing private structure is used in a relationship-only site.
    ConditionalComponent,
    /// A declaration or factory result violates the recipe's graph contract.
    InvalidDefinition {
        /// The violated recipe invariant.
        detail: &'static str,
        /// The implicated local output, when a declaration has one.
        slot: Option<CellSlot>,
    },
    /// The expanded graph exceeds representable or reservable capacity.
    Capacity,
    /// A repeatable initializer or method factory failed.
    Factory {
        /// The failed initializer or method factory.
        source: anyhow::Error,
        /// The private cell or first declared method output.
        slot: CellSlot,
    },
    /// A conditional's repeatable match-expression or key factory failed.
    ConditionalFactory {
        /// The underlying factory failure.
        source: anyhow::Error,
    },
    /// Retains a failed conditional's definition, local ordinal and active invocation sites.
    ///
    /// Context owns identities, not recipes or live state. The original error and
    /// its typed cause remain available through [`std::error::Error::source`].
    /// See [`super::SheetTemplate::instantiate`].
    ConditionalConstruction {
        /// Identifies the definition containing the failed conditional.
        template: TemplateId,
        /// Identifies the zero-based conditional registration ordinal in that definition.
        conditional: usize,
        /// Retains only active inclusion sites, ordered from the root to the failed definition.
        inclusions: Vec<RelationshipSlot>,
        /// Retains the original concrete construction failure and its cause.
        source: Box<TemplateError>,
    },
    /// An existing native graph mutator rejected construction.
    Runtime(Error),
}

impl fmt::Display for TemplateError {
    /// Describes the error with its local name, handle, or underlying failure.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownParameter { name } => write!(f, "unknown parameter {name:?}"),
            Self::MissingParameter { name } => write!(f, "missing parameter {name:?}"),
            Self::DuplicateParameter { name } => write!(f, "duplicate parameter {name:?}"),
            Self::AliasedArgument { first, second } => {
                write!(f, "parameters {first:?} and {second:?} alias one actual")
            }
            Self::ForeignHandle { slot } => write!(f, "foreign local cell {slot:?}"),
            Self::InvalidActual { name, actual } => {
                write!(f, "invalid actual {actual:?} for {name:?}")
            }
            Self::TypeMismatch {
                name,
                expected,
                found,
            } => {
                write!(
                    f,
                    "type mismatch for {name:?}: expected {expected:?}, found {found:?}"
                )
            }
            Self::OutArgument { name, actual } => write!(f, "out argument {actual:?} for {name:?}"),
            Self::EmptyRelationship => f.write_str("relationship requires a method"),
            Self::ConditionalComponent => f.write_str("component is not relationship-only"),
            Self::InvalidDefinition { detail, slot } => write!(f, "{detail} at {slot:?}"),
            Self::Capacity => f.write_str("template expansion exceeds capacity"),
            Self::Factory { source, slot } => {
                write!(f, "template factory failed at {slot:?}: {source}")
            }
            Self::ConditionalFactory { source } => {
                write!(f, "template conditional factory failed: {source}")
            }
            Self::ConditionalConstruction {
                conditional,
                source,
                ..
            } => write!(
                f,
                "template conditional {conditional} construction failed: {source}"
            ),
            Self::Runtime(source) => write!(f, "native template construction failed: {source}"),
        }
    }
}

impl std::error::Error for TemplateError {
    /// Returns the underlying factory or native construction failure.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Factory { source, .. } => Some(source.as_ref()),
            Self::ConditionalFactory { source } => Some(source.as_ref()),
            Self::ConditionalConstruction { source, .. } => Some(source.as_ref()),
            Self::Runtime(source) => Some(source),
            _ => None,
        }
    }
}
