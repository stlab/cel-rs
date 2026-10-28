//! Construction and binding failures without language source dependencies.

use std::any::TypeId;
use std::fmt;

use super::{CellSlot, RelationshipSlot, TemplateId};
use crate::{CellId, Error};

/// Identifies an authored construction boundary before any live relationship exists.
///
/// Method indices precede source specialization; policy indices count registrations
/// on the same local cell. Handles retain definition identity without retaining recipes.
///
/// # Examples
/// ```
/// use adam_rs::{ConstructionSite, Sheet, TemplateBuilder, TemplateError};
/// let mut builder = TemplateBuilder::new();
/// let slot = builder.cell::<i32, _>("value",
///     || Err(std::io::Error::other("initialization failed").into()))?;
/// let template = builder.finish()?;
/// let error = template.instantiate(&mut Sheet::new(), &[]).unwrap_err();
/// assert!(matches!(error, TemplateError::Construction {
///     site: ConstructionSite::Cell(ref failed), ..
/// } if failed == &slot));
/// # Ok::<(), TemplateError>(())
/// ```
#[derive(Debug)]
pub enum ConstructionSite {
    /// Constructs one private cell.
    Cell(CellSlot),
    /// Attaches one cell's filter or requirement, in registration order.
    Policy {
        /// Identifies the governed cell.
        cell: CellSlot,
        /// Identifies the cell-local policy registration.
        index: usize,
    },
    /// Constructs one authored method before specialization.
    Method {
        /// Identifies the local relationship.
        relationship: RelationshipSlot,
        /// Identifies the original authored method.
        index: usize,
    },
    /// Registers the combined relationship after its factories succeed.
    Relationship(RelationshipSlot),
}

/// Identifies a malformed recipe, binding, factory, or native graph construction.
///
/// Binding and declared-structure errors precede destination mutation. Factory-result
/// validation, allocation and native construction failures may leave partial structure;
/// the sheet remains safe to discard. See [`super::SheetTemplate::instantiate`].
///
/// # Examples
/// See [`ConstructionSite`]'s failed-initializer example.
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
    /// Retains a construction boundary and only its actual root-to-child inclusion chain.
    ///
    /// The boxed original failure preserves concrete error downcasting.
    /// See [`super::SheetTemplate::instantiate`].
    Construction {
        /// Identifies the failed definition.
        template: TemplateId,
        /// Identifies the declaration or registration that failed.
        site: ConstructionSite,
        /// Retains active inclusion sites, ordered root to child.
        inclusions: Vec<RelationshipSlot>,
        /// Retains the original failure without requiring Send or Sync.
        source: Box<TemplateError>,
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
    ///
    /// - Complexity: O(L + D) for L formatted characters and D nested construction
    ///   wrappers, plus underlying factory/native error formatting.
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
            Self::Construction { source, .. } => source.fmt(f),
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
            Self::Construction { source, .. } => Some(source.as_ref()),
            Self::Runtime(source) => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    /// Describes every leaf error and exposes causes only for wrapped failures.
    #[test]
    fn leaf_errors_preserve_display_and_sources() {
        let mut builder = super::super::TemplateBuilder::new();
        let slot = builder.parameter::<i32>("value").unwrap();
        let cases = [
            (
                TemplateError::UnknownParameter {
                    name: "value".into(),
                },
                "unknown parameter",
            ),
            (
                TemplateError::MissingParameter {
                    name: "value".into(),
                },
                "missing parameter",
            ),
            (
                TemplateError::DuplicateParameter {
                    name: "value".into(),
                },
                "duplicate parameter",
            ),
            (
                TemplateError::AliasedArgument {
                    first: "a".into(),
                    second: "b".into(),
                },
                "alias",
            ),
            (
                TemplateError::ForeignHandle { slot: slot.clone() },
                "foreign local cell",
            ),
            (
                TemplateError::InvalidActual {
                    name: "value".into(),
                    actual: CellId::default(),
                },
                "invalid actual",
            ),
            (
                TemplateError::TypeMismatch {
                    name: "value".into(),
                    expected: TypeId::of::<i32>(),
                    found: TypeId::of::<bool>(),
                },
                "type mismatch",
            ),
            (
                TemplateError::OutArgument {
                    name: "value".into(),
                    actual: CellId::default(),
                },
                "out argument",
            ),
            (TemplateError::EmptyRelationship, "requires a method"),
            (TemplateError::ConditionalComponent, "not relationship-only"),
            (
                TemplateError::InvalidDefinition {
                    detail: "invalid edges",
                    slot: Some(slot.clone()),
                },
                "invalid edges",
            ),
            (TemplateError::Capacity, "capacity"),
        ];
        for (error, expected) in cases {
            assert!(error.to_string().contains(expected), "{error}");
            assert!(error.source().is_none());
        }
        let factory = TemplateError::Factory {
            source: std::io::Error::other("typed cause").into(),
            slot,
        };
        assert!(factory.to_string().contains("typed cause"));
        assert!(
            factory
                .source()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .is_some()
        );
        let conditional = TemplateError::ConditionalFactory {
            source: std::io::Error::other("key cause").into(),
        };
        assert!(conditional.to_string().contains("key cause"));
        assert!(
            conditional
                .source()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .is_some()
        );
        let runtime = TemplateError::Runtime(Error::InvalidId);
        assert!(runtime.to_string().contains(&Error::InvalidId.to_string()));
        assert!(runtime.source().unwrap().downcast_ref::<Error>().is_some());
    }

    /// Retains construction context and concrete nested causes without losing display details.
    #[test]
    fn construction_wrappers_preserve_context_and_typed_cause() {
        let mut builder = super::super::TemplateBuilder::new();
        let slot = builder.parameter::<i32>("value").unwrap();
        let template = builder.finish().unwrap();
        let error = TemplateError::Construction {
            template: template.id(),
            site: ConstructionSite::Cell(slot.clone()),
            inclusions: vec![],
            source: Box::new(TemplateError::Factory {
                slot: slot.clone(),
                source: std::io::Error::other("root cause").into(),
            }),
        };
        assert!(error.to_string().contains("root cause"));
        assert!(
            error
                .source()
                .unwrap()
                .source()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .is_some()
        );
        assert!(matches!(&error, TemplateError::Construction {
                    template: owner, site: ConstructionSite::Cell(cell), inclusions, ..
                } if *owner == template.id() && *cell == slot && inclusions.is_empty()));
        let error = TemplateError::ConditionalConstruction {
            template: template.id(),
            conditional: 2,
            inclusions: vec![],
            source: Box::new(error),
        };
        assert!(error.to_string().contains("conditional 2"));
        assert!(error.to_string().contains("root cause"));
        assert!(
            error
                .source()
                .unwrap()
                .downcast_ref::<TemplateError>()
                .is_some()
        );
    }
}
