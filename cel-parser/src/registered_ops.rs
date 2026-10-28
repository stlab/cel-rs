//! Persistent operation and record metadata shared by independent host libraries.
//!
//! Register a function marker once, then add disjoint operand signatures from any
//! installer. Ordinary calls and direct member lookup share the same emitters:
//!
//! ```
//! use cel_parser::{OpLookup, OperationSignature, ResolvedLeafType};
//! use cel_runtime::ValueType;
//! #[derive(Clone)]
//! struct Identity;
//! let mut lookup = OpLookup::new();
//! lookup.register_function("identity", Identity).unwrap();
//! let ty = ResolvedLeafType::new("u32", ValueType::leaf::<u32>());
//! lookup.register_operation(OperationSignature {
//!     name: "identity", operands: vec![ty.clone().into()], result: ty.into(),
//!     emit: Box::new(|segment, _| segment.op1(|value: u32| value)),
//! }).unwrap();
//! assert_eq!(lookup.operation_signatures("identity").count(), 1);
//! ```

use std::any::TypeId;

use anyhow::{Result, ensure};
use cel_runtime::{DynSegment, ValueType};

use crate::{ExprSpan, OpLookup, ParseError, ResolvedLeafType, ResolvedType, SourceSpan};

/// Emits an operation after the complete operand signature has matched.
///
/// - Precondition: the stack ends with the declared operand descriptors.
/// - Postcondition: success replaces those operands with the declared result.
///
/// Emitters validate fallible setup before changing the segment. The lookup uniquely owns
/// each emitter and invokes it only while compiling; segments own the emitted operations.
/// Boxing a zero-sized, captureless closure does not allocate.
pub type OperationEmitter = Box<dyn Fn(&mut DynSegment, SourceSpan) -> Result<()> + Send + Sync>;

/// Consumes source-order fields using source-position to schema-position indices,
/// the ambient stack byte offset, and the expression span.
///
/// The lookup uniquely owns this compile-time callback. Boxing a zero-sized,
/// captureless closure does not allocate.
pub type RecordEmitter =
    Box<dyn Fn(&mut DynSegment, &[usize], usize, SourceSpan) -> Result<()> + Send + Sync>;

/// Describes one named operation with complete concrete operand and result identities.
///
/// The emitter consumes exactly `operands` and produces exactly `result`.
pub struct OperationSignature {
    /// The shared operation name.
    pub name: &'static str,
    /// Operands in source order, including the receiver for member lookup.
    pub operands: Vec<ResolvedType>,
    /// The single result's complete type.
    pub result: ResolvedType,
    /// The implementation used by both direct lookup and ordinary calls.
    pub emit: OperationEmitter,
}

/// Describes one required record field.
pub struct RecordField {
    /// The source field name.
    pub name: &'static str,
    /// The field's complete concrete type.
    pub ty: ResolvedType,
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
    pub(crate) marker: ResolvedLeafType,
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
/// - Complexity: O(n) in recursive descriptor nodes.
fn same_type(left: &ResolvedType, right: &ResolvedType) -> bool {
    match (left.complete_value_type(), right.complete_value_type()) {
        (Ok(left), Ok(right)) => left.same_shape(&right),
        _ => false,
    }
}

/// Matches the complete runtime descriptor, never just an erased array marker.
///
/// - Complexity: O(n) in recursive descriptor nodes.
fn matches_type(expected: &ResolvedType, actual: &ValueType) -> bool {
    expected
        .complete_value_type()
        .is_ok_and(|expected| expected.same_shape(actual))
}

impl OpLookup {
    /// Returns the concrete marker emitted by a bare registered function name.
    ///
    /// - Complexity: O(f) for f registered functions.
    pub(crate) fn registered_function_type(&self, name: &str) -> Option<&ResolvedLeafType> {
        self.registered
            .functions
            .iter()
            .find(|function| function.name == name)
            .map(|function| &function.marker)
    }

    /// Returns the callable name selected by executable dispatch for a concrete marker.
    ///
    /// - Complexity: O(f) for f registered functions.
    pub(crate) fn registered_function_name(&self, marker: TypeId) -> Option<&'static str> {
        self.registered
            .functions
            .iter()
            .find(|function| function.marker.type_id() == marker)
            .map(|function| function.name)
    }

    /// Validates required named fields and emits one record from their original stack order.
    ///
    /// Field descriptors must match exactly, including recursive array identities. Validation
    /// never changes the segment; only a fully validated constructor invokes its emitter.
    ///
    /// - Precondition: the stack ends with one value per field, starting at `ambient_start`.
    /// - Complexity: O(r + f² + f * d) for r schemas, f fields and descriptor depth d.
    ///
    /// # Errors
    /// Returns an error for unknown records, unknown/duplicate/missing fields, mismatched
    /// field types, or a constructor emitter's setup failure.
    ///
    /// # Examples
    /// ```
    /// use cel_parser::{CELParser, OpLookup, RecordSchema, ResolvedLeafType};
    /// use cel_runtime::ValueType;
    /// let mut lookup = OpLookup::new();
    /// lookup.register_record(RecordSchema {
    ///     ty: ResolvedLeafType::new("Empty", ValueType::leaf::<()>()),
    ///     fields: vec![],
    ///     emit: Box::new(|s, _, _, _| { s.just(()); Ok(()) }),
    /// }).unwrap();
    /// CELParser::new(lookup).parse_str("Empty {}").unwrap().call0::<()>().unwrap();
    /// ```
    pub fn construct_record(
        &self,
        name: &str,
        fields: &[(String, ExprSpan)],
        ambient_start: usize,
        segment: &mut DynSegment,
        start: proc_macro2::Span,
        end: proc_macro2::Span,
    ) -> crate::Result<()> {
        let error = |message| ParseError::new_range(message, start, end);
        let schema = self
            .record_schema(name)
            .ok_or_else(|| error(format!("unknown record `{name}`")))?;
        let mut mapping = Vec::with_capacity(fields.len());
        for (index, (field, span)) in fields.iter().enumerate() {
            let schema_index = schema
                .fields
                .iter()
                .position(|candidate| candidate.name == field)
                .ok_or_else(|| {
                    ParseError::new_range(
                        format!("unknown field `{field}` in record `{name}`"),
                        span.start,
                        span.end,
                    )
                })?;
            if fields[..index]
                .iter()
                .any(|(previous, _)| previous == field)
            {
                return Err(ParseError::new_range(
                    format!("duplicate field `{field}` in record `{name}`"),
                    span.start,
                    span.end,
                ));
            }
            mapping.push(schema_index);
        }
        if let Some(missing) = schema
            .fields
            .iter()
            .enumerate()
            .find(|(index, _)| !mapping.contains(index))
        {
            return Err(error(format!(
                "missing field `{}` in record `{name}`",
                missing.1.name
            )));
        }
        let operands = segment.peek_stack_infos(fields.len());
        debug_assert_eq!(operands.len(), fields.len());
        debug_assert_eq!(
            segment.stack_offset_before_top(fields.len()).ok(),
            Some(ambient_start)
        );
        for ((actual, &schema_index), (field, span)) in operands.iter().zip(&mapping).zip(fields) {
            let expected = &schema.fields[schema_index].ty;
            if !matches_type(expected, &actual.value_type) {
                return Err(ParseError::new_range(
                    format!(
                        "field `{field}`: type mismatch: expected `{}`",
                        expected.display_name()
                    ),
                    span.start,
                    span.end,
                ));
            }
        }
        (schema.emit)(
            segment,
            &mapping,
            ambient_start,
            SourceSpan::from_proc_macro2_range(start, end),
        )
        .map_err(|err| error(err.to_string()))?;
        segment
            .annotate_top(
                schema
                    .ty
                    .complete_value_type()
                    .map_err(|err| error(err.to_string()))?
                    .clone(),
            )
            .map_err(|err| error(err.to_string()))
    }

    /// Registers one persistent named operation without replacing existing overloads.
    ///
    /// # Errors
    /// Returns an error for unavailable complete factories or unrepresentable layouts, and
    /// duplicate operand identities, including return-type-only overloads.
    ///
    /// - Complexity: O(s * a * d) for s signatures, a operands and descriptor depth d.
    ///
    /// # Examples
    /// See the [registration tutorial](crate::registered_ops).
    pub fn register_operation(&mut self, signature: OperationSignature) -> Result<()> {
        for operand in &signature.operands {
            operand.complete_value_type()?;
        }
        signature.result.complete_value_type()?;
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
                marker: ResolvedLeafType::new(name, ValueType::cloneable_leaf::<M>()),
                emit: Box::new(move |segment, _| {
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
    /// Returns an error for unavailable complete factories or unrepresentable layouts, a
    /// duplicate schema name, or duplicate field names.
    ///
    /// - Complexity: O(r + f²) for r schemas and f fields.
    ///
    /// # Examples
    /// ```
    /// use cel_parser::{OpLookup, RecordSchema, ResolvedLeafType};
    /// use cel_runtime::ValueType;
    /// let mut lookup = OpLookup::new();
    /// lookup.register_record(RecordSchema {
    ///     ty: ResolvedLeafType::new("Unit", ValueType::leaf::<()>()),
    ///     fields: vec![],
    ///     emit: Box::new(|segment, _, _, _| { segment.just(()); Ok(()) }),
    /// }).unwrap();
    /// assert!(lookup.record_schema("Unit").is_some());
    /// ```
    pub fn register_record(&mut self, schema: RecordSchema) -> Result<()> {
        schema.ty.complete_value_type()?;
        for field in &schema.fields {
            field.ty.complete_value_type()?;
        }
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
                segment.annotate_top(signature.result.complete_value_type()?.clone())?;
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
        library_only: bool,
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
                .find(|f| f.marker.type_id() == marker_type)
            {
                let ambient_start = segment.stack_offset_before_top(arity)?;
                if self.lookup_direct_operation_scoped(
                    function.name,
                    segment,
                    arity - 1,
                    span,
                    library_only,
                )? {
                    segment.make_tuple(2, ambient_start)?;
                    segment.tuple_index(1)?;
                    return Ok(true);
                }
                return Ok(false);
            }
        }
        self.lookup_registered_operation(name, segment, arity, span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Supplies a complete scalar descriptor for registration contracts.
    fn scalar<T: 'static>(name: &'static str) -> ResolvedType {
        ResolvedLeafType::new(name, ValueType::leaf::<T>()).into()
    }

    /// Schema validation rejects duplicate fields and successful construction retains its descriptor.
    #[test]
    fn record_schema_validation_precedes_construction() {
        let mut lookup = OpLookup::new();
        let invalid = RecordSchema {
            ty: ResolvedLeafType::new("Invalid", ValueType::leaf::<u8>()),
            fields: vec![
                RecordField {
                    name: "x",
                    ty: scalar::<i32>("i32"),
                },
                RecordField {
                    name: "x",
                    ty: scalar::<i32>("i32"),
                },
            ],
            emit: Box::new(|_, _, _, _| panic!("invalid schema must not be installed")),
        };
        assert!(lookup.register_record(invalid).is_err());
        assert!(lookup.record_schema("Invalid").is_none());
        lookup
            .register_record(RecordSchema {
                ty: ResolvedLeafType::new("Empty", ValueType::leaf::<u8>()),
                fields: Vec::new(),
                emit: Box::new(|segment, mapping, ambient, _| {
                    assert!(mapping.is_empty());
                    assert_eq!(ambient, 0);
                    segment.just(9u8);
                    Ok(())
                }),
            })
            .unwrap();
        let mut segment = DynSegment::new::<()>();
        lookup
            .construct_record(
                "Empty",
                &[],
                0,
                &mut segment,
                proc_macro2::Span::call_site(),
                proc_macro2::Span::call_site(),
            )
            .unwrap();
        assert_eq!(
            segment.peek_stack_infos(1)[0].value_type.type_name(),
            "Empty"
        );
        assert_eq!(segment.call0::<u8>().unwrap(), 9);
    }

    /// Builds one signature whose execution adds an observable offset.
    fn increment() -> OperationSignature {
        OperationSignature {
            name: "increment",
            operands: vec![scalar::<i32>("i32")],
            result: scalar::<i32>("i32"),
            emit: Box::new(|segment, _| {
                segment.op1r(|value: i32| {
                    value
                        .checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("overflow"))
                })
            }),
        }
    }

    /// Aliases compare by complete identity, while nested element mismatches remain distinct.
    #[test]
    fn complete_identity_does_not_depend_on_diagnostic_names() {
        assert!(same_type(&scalar::<i32>("i32"), &scalar::<i32>("Alias")));
        assert!(!same_type(&scalar::<i32>("i32"), &scalar::<u32>("i32")));
        let array = ResolvedType::Array {
            element: Box::new(scalar::<i32>("i32")),
        };
        assert!(matches_type(
            &array,
            &ValueType::array(ValueType::leaf::<i32>())
        ));
        assert!(!matches_type(
            &array,
            &ValueType::array(ValueType::leaf::<u32>())
        ));
    }

    /// Duplicate rejection preserves the first signature and its working emitter.
    #[test]
    fn registration_and_dispatch_preserve_the_first_overload() {
        let mut lookup = OpLookup::new();
        lookup.register_operation(increment()).unwrap();
        let mut duplicate = increment();
        duplicate.result = scalar::<bool>("bool");
        assert!(lookup.register_operation(duplicate).is_err());
        assert_eq!(lookup.operation_signatures("increment").count(), 1);
        assert_eq!(lookup.operation_signatures("missing").count(), 0);
        let mut segment = DynSegment::new::<()>();
        segment.just(41i32);
        assert!(
            !lookup
                .lookup_registered_operation(
                    "missing",
                    &mut segment,
                    1,
                    SourceSpan::new(1, 0, 1, 1)
                )
                .unwrap()
        );
        assert!(
            lookup
                .lookup_registered_operation(
                    "increment",
                    &mut segment,
                    1,
                    SourceSpan::new(1, 0, 1, 1)
                )
                .unwrap()
        );
        assert_eq!(segment.call0::<i32>().unwrap(), 42);
    }

    /// A marker cannot identify two functions and metadata agrees with executable dispatch.
    #[test]
    fn function_markers_have_one_name_and_emit_the_registered_value() {
        let mut lookup = OpLookup::new();
        lookup.register_function("marker", 17u8).unwrap();
        assert!(lookup.register_function("other", 18u8).is_err());
        assert_eq!(
            lookup.registered_function_name(TypeId::of::<u8>()),
            Some("marker")
        );
        assert_eq!(
            lookup.registered_function_type("marker").unwrap().type_id(),
            TypeId::of::<u8>()
        );
        let mut segment = DynSegment::new::<()>();
        assert!(
            lookup
                .lookup_registered(
                    "marker",
                    &mut segment,
                    0,
                    SourceSpan::new(1, 0, 1, 1),
                    false
                )
                .unwrap()
        );
        assert_eq!(segment.call0::<u8>().unwrap(), 17);
    }
}
