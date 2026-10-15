//! Persistent operation and record metadata shared by independent host libraries.
//!
//! Register a function marker once, then add disjoint operand signatures from any
//! installer. Ordinary calls and direct member lookup share the same emitters:
//!
//! ```
//! use cel_parser::{OpLookup, OperationSignature, ResolvedLeafType};
//! use cel_runtime::ArrayElementType;
//! use std::sync::Arc;
//! #[derive(Clone)]
//! struct Identity;
//! let mut lookup = OpLookup::new();
//! lookup.register_function("identity", Identity).unwrap();
//! let ty = ResolvedLeafType::new("u32", ArrayElementType::leaf::<u32>().unwrap());
//! lookup.register_operation(OperationSignature {
//!     name: "identity", operands: vec![ty.clone()], result: ty,
//!     emit: Arc::new(|segment, _| segment.op1(|value: u32| value)),
//! }).unwrap();
//! assert_eq!(lookup.operation_signatures("identity").count(), 1);
//! ```

use std::{any::TypeId, sync::Arc};

use anyhow::{Result, ensure};
use cel_runtime::{DynSegment, ValueKind, ValueType};

use crate::{OpLookup, ResolvedLeafType, SourceSpan};

/// Emits an operation after the complete operand signature has matched.
///
/// - Precondition: the stack ends with the declared operand descriptors.
/// - Postcondition: success replaces those operands with the declared result.
///
/// Emitters validate fallible setup before changing the segment.
pub type OperationEmitter = Arc<dyn Fn(&mut DynSegment, SourceSpan) -> Result<()> + Send + Sync>;

/// Consumes source-order fields using source-position to schema-position indices,
/// the ambient stack byte offset, and the expression span.
pub type RecordEmitter =
    Arc<dyn Fn(&mut DynSegment, &[usize], usize, SourceSpan) -> Result<()> + Send + Sync>;

/// Describes one named operation with complete concrete operand and result identities.
///
/// The emitter consumes exactly `operands` and produces exactly `result`.
pub struct OperationSignature {
    /// The shared operation name.
    pub name: &'static str,
    /// Operands in source order, including the receiver for member lookup.
    pub operands: Vec<ResolvedLeafType>,
    /// The single result's complete type.
    pub result: ResolvedLeafType,
    /// The implementation used by both direct lookup and ordinary calls.
    pub emit: OperationEmitter,
}

/// Describes one required record field.
pub struct RecordField {
    /// The source field name.
    pub name: &'static str,
    /// The field's complete concrete type.
    pub ty: ResolvedLeafType,
}

/// Describes a named record's required fields and construction implementation.
pub struct RecordSchema {
    /// The constructed type, whose source name keys this schema.
    pub ty: ResolvedLeafType,
    /// Required fields in canonical constructor order.
    pub fields: Vec<RecordField>,
    /// The constructor, invoked only after field validation.
    pub emit: RecordEmitter,
}

/// Stores the first marker selected for a function name.
pub(crate) struct RegisteredFunction {
    pub(crate) name: &'static str,
    pub(crate) marker_type: TypeId,
    pub(crate) emit: OperationEmitter,
}

/// Holds permanent registrations independently of transient parser scopes.
#[derive(Default)]
pub(crate) struct RegisteredOps {
    operations: Vec<OperationSignature>,
    functions: Vec<RegisteredFunction>,
    marker_names: Vec<(TypeId, &'static str)>,
    records: Vec<RecordSchema>,
}

/// Compares concrete recursive identities without treating diagnostic names as identity.
///
/// - Complexity: O(d) in descriptor nesting depth.
fn same_type(left: &ResolvedLeafType, right: &ResolvedLeafType) -> bool {
    left.element_type() == right.element_type()
}

/// Matches the complete runtime descriptor, never just an erased array marker.
///
/// - Complexity: O(d) in descriptor nesting depth.
fn matches_type(expected: &ResolvedLeafType, actual: &ValueType) -> bool {
    expected.type_id() == actual.type_id()
        && match actual.kind() {
            ValueKind::Leaf => expected.element_type().nested().is_none(),
            ValueKind::Array(element) => expected.element_type().nested() == Some(element),
            ValueKind::Tuple(_) => false,
        }
}

impl OpLookup {
    /// Registers one persistent named operation without replacing existing overloads.
    ///
    /// # Errors
    /// Returns an error for duplicate operand identities, including return-type-only overloads.
    ///
    /// - Complexity: O(s * a * d) for s signatures, a operands and descriptor depth d.
    ///
    /// # Examples
    /// See the [registration tutorial](crate::registered_ops).
    pub fn register_operation(&mut self, signature: OperationSignature) -> Result<()> {
        ensure!(
            !self.operation_signatures(signature.name).any(|existing| {
                existing.operands.len() == signature.operands.len()
                    && existing
                        .operands
                        .iter()
                        .zip(&signature.operands)
                        .all(|(a, b)| same_type(a, b))
            }),
            "duplicate operation signature for `{}`",
            signature.name
        );
        self.registered.operations.push(signature);
        Ok(())
    }

    /// Binds a persistent callable name, retaining its first marker across installers.
    ///
    /// Markers require `Send + Sync` so lookup retains its thread-safety guarantees.
    ///
    /// # Errors
    /// Returns an error if this marker type has already named a distinct function.
    ///
    /// - Complexity: O(f) for f registered marker types.
    ///
    /// # Examples
    /// See the [registration tutorial](crate::registered_ops).
    pub fn register_function<M: Clone + Send + Sync + 'static>(
        &mut self,
        name: &'static str,
        marker: M,
    ) -> Result<()> {
        let marker_type = TypeId::of::<M>();
        ensure!(
            self.registered
                .marker_names
                .iter()
                .all(|(ty, bound_name)| *ty != marker_type || *bound_name == name),
            "function marker already names another function"
        );
        if !self.registered.marker_names.contains(&(marker_type, name)) {
            self.registered.marker_names.push((marker_type, name));
        }
        if !self
            .registered
            .functions
            .iter()
            .any(|function| function.name == name)
        {
            self.registered.functions.push(RegisteredFunction {
                name,
                marker_type,
                emit: Arc::new(move |segment, _| {
                    segment.just(marker.clone());
                    Ok(())
                }),
            });
        }
        Ok(())
    }

    /// Registers a persistent record schema keyed by its source type name.
    ///
    /// # Errors
    /// Returns an error for a duplicate schema name or duplicate field names.
    ///
    /// - Complexity: O(r + f²) for r schemas and f fields.
    ///
    /// # Examples
    /// ```
    /// use cel_parser::{OpLookup, RecordSchema, ResolvedLeafType};
    /// use cel_runtime::ArrayElementType;
    /// use std::sync::Arc;
    /// let mut lookup = OpLookup::new();
    /// lookup.register_record(RecordSchema {
    ///     ty: ResolvedLeafType::new("Unit", ArrayElementType::leaf::<()>().unwrap()),
    ///     fields: vec![],
    ///     emit: Arc::new(|segment, _, _, _| { segment.just(()); Ok(()) }),
    /// }).unwrap();
    /// assert!(lookup.record_schema("Unit").is_some());
    /// ```
    pub fn register_record(&mut self, schema: RecordSchema) -> Result<()> {
        ensure!(
            self.record_schema(schema.ty.type_name()).is_none(),
            "duplicate record schema `{}`",
            schema.ty.type_name()
        );
        for (index, field) in schema.fields.iter().enumerate() {
            ensure!(
                !schema.fields[..index]
                    .iter()
                    .any(|other| other.name == field.name),
                "duplicate record field `{}`",
                field.name
            );
        }
        self.registered.records.push(schema);
        Ok(())
    }

    /// Iterates persistent signatures named `name` in registration order.
    ///
    /// - Complexity: O(s) to exhaust s registered signatures.
    ///
    /// # Examples
    /// See the [registration tutorial](crate::registered_ops).
    pub fn operation_signatures(&self, name: &str) -> impl Iterator<Item = &OperationSignature> {
        self.registered
            .operations
            .iter()
            .filter(move |signature| signature.name == name)
    }

    /// Returns the persistent schema with the given source type name.
    ///
    /// - Complexity: O(r) for r registered schemas.
    ///
    /// # Examples
    /// See [`Self::register_record`].
    pub fn record_schema(&self, name: &str) -> Option<&RecordSchema> {
        self.registered
            .records
            .iter()
            .find(|schema| schema.ty.type_name() == name)
    }

    /// Dispatches a registered signature only after every operand descriptor matches.
    ///
    /// Declining a signature leaves operand descriptors and emitted operations unchanged.
    ///
    /// # Errors
    /// Returns the selected emitter's setup error.
    ///
    /// - Complexity: O(s * a * d) for s signatures, a operands and descriptor depth d.
    pub(crate) fn lookup_registered_operation(
        &self,
        name: &str,
        segment: &mut DynSegment,
        arity: usize,
        span: SourceSpan,
    ) -> Result<bool> {
        let operands = segment.peek_stack_infos(arity);
        if operands.len() != arity {
            return Ok(false);
        }
        for signature in self.operation_signatures(name) {
            if signature.operands.len() == arity
                && signature
                    .operands
                    .iter()
                    .zip(operands)
                    .all(|(expected, actual)| matches_type(expected, &actual.value_type))
            {
                (signature.emit)(segment, span)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Resolves a bare registered name or a marker-qualified ordinary call.
    ///
    /// # Errors
    /// Returns a selected scope/emitter error or a checked stack-offset error.
    ///
    /// - Complexity: O(f + k + s * a * d + n) for f functions, k scopes,
    ///   s signatures, a operands, descriptor depth d and n ambient stack entries.
    pub(crate) fn lookup_registered(
        &self,
        name: &str,
        segment: &mut DynSegment,
        arity: usize,
        span: SourceSpan,
    ) -> Result<bool> {
        if arity == 0
            && let Some(function) = self.registered.functions.iter().find(|f| f.name == name)
        {
            (function.emit)(segment, span)?;
            return Ok(true);
        }
        if name == "()" && arity > 0 {
            let operands = segment.peek_stack_infos(arity);
            if operands.len() != arity {
                return Ok(false);
            }
            let marker_type = operands[0].value_type.type_id();
            if let Some(function) = self
                .registered
                .functions
                .iter()
                .find(|f| f.marker_type == marker_type)
            {
                let ambient_start = segment.stack_offset_before_top(arity)?;
                if self.lookup_direct_operation(function.name, segment, arity - 1, span)? {
                    segment.make_tuple(2, ambient_start);
                    segment.tuple_index(1);
                    return Ok(true);
                }
                return Ok(false);
            }
        }
        self.lookup_registered_operation(name, segment, arity, span)
    }
}
