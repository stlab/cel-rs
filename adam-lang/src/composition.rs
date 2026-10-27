//! Builds complete-shape templates without parsing source text.
//!
//! Emitters return fresh segments; they never execute expressions during definition
//! checking. Each live factory creates fresh mutable execution frames. Immutable
//! definitions and intentionally shared host inputs may be captured by emitters.
//!
//! ```
//! use adam_lang::{CompositionBuilder, ParsedSheet, TypeRegistry};
//! use adam_lang::type_registry::TypeShape;
//! use std::any::TypeId;
//! let types = TypeRegistry::new();
//! let shape = TypeShape::Named(TypeId::of::<i32>());
//! let mut builder = CompositionBuilder::new(types.clone());
//! builder.parameter("x", shape.clone())?;
//! let template = builder.finish()?;
//! let mut destination = ParsedSheet::new();
//! let x = destination.add_cell(3i32);
//! destination.register_cell("x", x, shape, &types)?;
//! template.instantiate(&mut destination, &[("x", x)])?;
//! # Ok::<(), adam_lang::ModuleDiagnostic>(())
//! ```

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use adam_rs::{
    CellId, Instance, MatchExpr, Method, OccurrenceId, RelationshipSlot, SheetTemplate,
    TemplateBuilder, TemplateId, TemplateInclusion,
};
use cel_runtime::DynSegment;

use crate::cell_value::{CellValue, Frame};
use crate::type_registry::TypeShape;
use crate::{ModuleDiagnostic, ParsedSheet, SourceLabel, TypeRegistry};

/// Retains a native local handle and its complete declared shape.
///
/// See the [module example](self).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellSlot {
    native: adam_rs::CellSlot,
    shape: TypeShape,
}

impl CellSlot {
    /// Borrows the owner-checked native handle for occurrence inspection.
    ///
    /// # Examples
    /// ```
    /// let mut builder = adam_lang::CompositionBuilder::new(adam_lang::TypeRegistry::new());
    /// let slot = builder.parameter("x", adam_lang::type_registry::TypeShape::Tuple(vec![]))?;
    /// assert_eq!(slot.native(), slot.native());
    /// # Ok::<(), adam_lang::ModuleDiagnostic>(())
    /// ```
    pub fn native(&self) -> &adam_rs::CellSlot {
        &self.native
    }

    /// Borrows the complete shape, independently of native storage identity.
    ///
    /// # Examples
    /// See [`Self::native`].
    pub fn shape(&self) -> &TypeShape {
        &self.shape
    }
}

/// Owns a repeatable native method with full input and output shapes.
///
/// One output receives the whole result; multiple outputs destructure a tuple.
/// See [`CompositionBuilder::method`].
pub struct MethodRecipe {
    native: adam_rs::MethodRecipe,
    inputs: Vec<TypeShape>,
    outputs: Vec<TypeShape>,
}

impl MethodRecipe {
    /// Borrows the positional input shapes.
    ///
    /// # Examples
    /// See [`CompositionBuilder::method`].
    pub fn input_shapes(&self) -> &[TypeShape] {
        &self.inputs
    }

    /// Borrows the positional output shapes.
    ///
    /// # Examples
    /// See [`CompositionBuilder::method`].
    pub fn output_shapes(&self) -> &[TypeShape] {
        &self.outputs
    }
}

/// Retains one occurrence using parent links, never copied full invocation paths.
///
/// Identities retain no recipe edges. See [`TypedTemplate::instantiate`].
#[derive(Debug)]
pub struct OccurrenceOrigin {
    /// Identifies this live occurrence.
    pub id: OccurrenceId,
    /// Identifies its immediate parent, or `None` for a root invocation.
    pub parent: Option<OccurrenceId>,
    /// Identifies its immutable definition.
    pub template: TemplateId,
    /// Identifies the local inclusion group in the parent.
    pub inclusion_site: Option<RelationshipSlot>,
    /// Retains a real definition location, or a span-free native label.
    pub label: SourceLabel,
}

/// Retains one direct invocation's flat occurrence records after template drop.
///
/// See [`TypedTemplate::instantiate`].
#[derive(Debug)]
pub struct InstanceOrigin {
    /// Preserves depth-first native occurrence order.
    pub occurrences: Vec<OccurrenceOrigin>,
}

/// Maps one local declaration or bound formal to its live occurrence.
///
/// A live boundary cell can retain origins from several occurrences.
/// See [`TypedTemplate::instantiate`].
#[derive(Debug)]
pub struct CellOrigin {
    /// Identifies the occurrence containing this declaration or formal.
    pub occurrence: OccurrenceId,
    /// Identifies the local cell within that occurrence's definition.
    pub slot: adam_rs::CellSlot,
}

/// Constructs fresh type-erased initial values for monomorphized native adapters.
type Initializer = Box<dyn Fn() -> anyhow::Result<Box<dyn Any>>>;

/// Selects native constructors for one registered concrete storage type.
#[derive(Clone, Copy)]
pub(crate) struct LeafAdapters {
    parameter: fn(&mut TemplateBuilder, &str) -> Result<adam_rs::CellSlot, adam_rs::TemplateError>,
    cell: fn(
        &mut TemplateBuilder,
        &str,
        Initializer,
        bool,
    ) -> Result<adam_rs::CellSlot, adam_rs::TemplateError>,
}

impl LeafAdapters {
    /// Selects monomorphized constructors without a fixed list of host types.
    pub(crate) fn of<T: Any + PartialEq>() -> Self {
        Self {
            parameter: TemplateBuilder::parameter::<T>,
            cell: add_private::<T>,
        }
    }
}

/// Adds a private concrete cell or source using the canonical frame's output.
///
/// # Errors
/// Rejects duplicate names or native recipe errors; initializer errors occur later.
/// - Complexity: Amortized O(l) name bytes, excluding later initializer costs.
fn add_private<T: Any + PartialEq>(
    builder: &mut TemplateBuilder,
    name: &str,
    initial: Initializer,
    source: bool,
) -> Result<adam_rs::CellSlot, adam_rs::TemplateError> {
    let make = move || {
        initial()?
            .downcast::<T>()
            .map(|value| *value)
            .map_err(|_| anyhow::anyhow!("initializer storage mismatch"))
    };
    if source {
        builder.source(name, make)
    } else {
        builder.cell(name, make)
    }
}

/// Retains only a definition's local shapes and compact child metadata edges.
struct Metadata {
    id: TemplateId,
    cells: HashMap<adam_rs::CellSlot, TypeShape>,
    formals: HashMap<String, CellSlot>,
    children: Vec<Rc<Metadata>>,
    label: SourceLabel,
}

impl Drop for Metadata {
    /// Releases last-owned child metadata iteratively, including deep include chains.
    ///
    /// - Complexity: O(n) released metadata and O(w) pending child edges.
    fn drop(&mut self) {
        let mut pending = std::mem::take(&mut self.children);
        while let Some(child) = pending.pop() {
            if let Ok(mut child) = Rc::try_unwrap(child) {
                pending.append(&mut child.children);
            }
        }
    }
}

/// Builds shape-checked recipes by delegating every graph operation to Adam.
///
/// Fallible boundaries reject unregistered shapes before constructing adapters.
/// Factories must return fresh segments and fresh segment-local mutable owners.
/// Emitters run during checking and instance construction, but their expressions
/// run only during initialization or ordinary native execution.
/// See the [module example](self).
pub struct CompositionBuilder {
    native: TemplateBuilder,
    types: Rc<TypeRegistry>,
    cells: HashMap<adam_rs::CellSlot, TypeShape>,
    formals: HashMap<String, CellSlot>,
    children: Vec<Rc<Metadata>>,
    label: SourceLabel,
}

impl CompositionBuilder {
    /// Owns a registry and starts an empty source-free recipe.
    ///
    /// # Examples
    /// See the [module example](self).
    pub fn new(types: TypeRegistry) -> Self {
        Self {
            native: TemplateBuilder::new(),
            types: Rc::new(types),
            cells: HashMap::new(),
            formals: HashMap::new(),
            children: Vec::new(),
            label: SourceLabel {
                location: None,
                label: "native template".into(),
            },
        }
    }

    /// Retains a definition label without synthesizing source locations.
    ///
    /// # Examples
    /// ```
    /// let mut builder = adam_lang::CompositionBuilder::new(adam_lang::TypeRegistry::new());
    /// builder.set_label(adam_lang::SourceLabel { location: None, label: "host".into() });
    /// ```
    pub fn set_label(&mut self, label: SourceLabel) {
        self.label = label;
    }

    /// Declares a named parameter with its complete concrete shape.
    ///
    /// # Errors
    /// Rejects unregistered shapes and native declaration errors without mutation.
    /// - Complexity: Expected amortized O(n + l) shape nodes and name bytes.
    ///
    /// # Examples
    /// See the [module example](self).
    pub fn parameter(
        &mut self,
        name: &str,
        shape: TypeShape,
    ) -> Result<CellSlot, ModuleDiagnostic> {
        let adapters = self.adapters(&shape)?;
        let native = (adapters.parameter)(&mut self.native, name).map_err(diagnostic)?;
        let slot = self.record(native, shape);
        self.formals.insert(name.into(), slot.clone());
        Ok(slot)
    }

    /// Declares a fresh private ordinary cell initialized at instantiation.
    ///
    /// # Errors
    /// Rejects unknown shapes, incompatible emitted signatures, missing capabilities,
    /// or native declaration errors. Initializer execution errors occur at instantiation.
    /// - Complexity: O(n + e + l) shape nodes, emitted reads and name bytes,
    ///   excluding emitter costs.
    ///
    /// # Examples
    /// ```
    /// use adam_lang::{CompositionBuilder, TypeRegistry};
    /// use adam_lang::type_registry::TypeShape;
    /// let mut builder = CompositionBuilder::new(TypeRegistry::new());
    /// builder.cell("x", TypeShape::Named(std::any::TypeId::of::<i32>()), || {
    ///     let mut body = cel_runtime::DynSegment::new::<()>();
    ///     body.op0(|| 1i32);
    ///     Ok(body)
    /// })?;
    /// # Ok::<(), adam_lang::ModuleDiagnostic>(())
    /// ```
    pub fn cell<F>(
        &mut self,
        name: &str,
        shape: TypeShape,
        emit: F,
    ) -> Result<CellSlot, ModuleDiagnostic>
    where
        F: Fn() -> anyhow::Result<DynSegment> + 'static,
    {
        self.private(name, shape, emit, false)
    }

    /// Declares a fresh private source initialized at instantiation.
    ///
    /// # Errors
    /// Returns the same definition errors as [`Self::cell`].
    /// - Complexity: The same as [`Self::cell`].
    ///
    /// # Examples
    /// See [`Self::cell`]; use `source` instead of `cell`.
    pub fn source<F>(
        &mut self,
        name: &str,
        shape: TypeShape,
        emit: F,
    ) -> Result<CellSlot, ModuleDiagnostic>
    where
        F: Fn() -> anyhow::Result<DynSegment> + 'static,
    {
        self.private(name, shape, emit, true)
    }

    /// Declares a private cell using the registry's canonical default emitter.
    ///
    /// No default value is constructed until instantiation.
    /// # Errors
    /// Rejects unknown shapes, missing defaults/capabilities, or native declaration errors.
    /// - Complexity: O(n + l) shape nodes and name bytes.
    ///
    /// # Examples
    /// ```
    /// let mut b = adam_lang::CompositionBuilder::new(adam_lang::TypeRegistry::new());
    /// b.default_cell("empty", adam_lang::type_registry::TypeShape::Tuple(vec![]))?;
    /// # Ok::<(), adam_lang::ModuleDiagnostic>(())
    /// ```
    pub fn default_cell(
        &mut self,
        name: &str,
        shape: TypeShape,
    ) -> Result<CellSlot, ModuleDiagnostic> {
        let adapters = self.adapters(&shape)?;
        self.types.validate_default(&shape).map_err(diagnostic)?;
        let types = self.types.clone();
        let default_shape = shape.clone();
        // Preparation must not call host Default implementations.
        let ty = types.value_type(&shape);
        CellValue::validate(&ty).map_err(diagnostic)?;
        let native = (adapters.cell)(
            &mut self.native,
            name,
            Box::new(move || {
                let mut body = DynSegment::new::<()>();
                types.emit_default(&default_shape, &mut body)?;
                let mut frame = Frame::new(
                    body,
                    &[],
                    std::slice::from_ref(&default_shape),
                    false,
                    &types,
                )?;
                Ok(frame.call(&[])?.remove(0))
            }),
            false,
        )
        .map_err(diagnostic)?;
        Ok(self.record(native, shape))
    }

    /// Checks an initializer without executing it, then retains its repeatable factory.
    ///
    /// # Errors
    /// Rejects unknown shapes, signatures, capabilities, or native declarations.
    /// - Complexity: O(n + e + l) descriptor nodes, emitted reads and name bytes.
    fn private<F>(
        &mut self,
        name: &str,
        shape: TypeShape,
        emit: F,
        source: bool,
    ) -> Result<CellSlot, ModuleDiagnostic>
    where
        F: Fn() -> anyhow::Result<DynSegment> + 'static,
    {
        let adapters = self.adapters(&shape)?;
        CellValue::validate(&self.types.value_type(&shape)).map_err(diagnostic)?;
        Frame::new(
            emit().map_err(diagnostic)?,
            &[],
            std::slice::from_ref(&shape),
            false,
            &self.types,
        )
        .map_err(diagnostic)?;
        let types = self.types.clone();
        let initial_shape = shape.clone();
        let native = (adapters.cell)(
            &mut self.native,
            name,
            Box::new(move || {
                let mut frame = Frame::new(
                    emit()?,
                    &[],
                    std::slice::from_ref(&initial_shape),
                    false,
                    &types,
                )?;
                Ok(frame.call(&[])?.remove(0))
            }),
            source,
        )
        .map_err(diagnostic)?;
        Ok(self.record(native, shape))
    }

    /// Constructs a repeatable, complete-shape method without executing its expression.
    ///
    /// Inputs use positional `push_arg` indices. A single output receives the whole
    /// result; two or more outputs destructure the result's source-ordered tuple.
    /// Callbacks obey native method purity and self-reference idempotence contracts.
    /// # Errors
    /// Rejects foreign handles, malformed edges, incompatible bodies, or unknown shapes.
    /// - Complexity: O(n + e + o²) descriptor nodes n, emitted reads e and outputs o.
    ///
    /// # Examples
    /// ```
    /// use adam_lang::{CompositionBuilder, TypeRegistry};
    /// use adam_lang::type_registry::TypeShape;
    /// let mut b = CompositionBuilder::new(TypeRegistry::new());
    /// let x = b.parameter("x", TypeShape::Named(std::any::TypeId::of::<i32>()))?;
    /// let recipe = b.method(std::slice::from_ref(&x), std::slice::from_ref(&x), || {
    ///     let mut body = cel_runtime::DynSegment::new::<()>();
    ///     body.push_arg::<i32>(0);
    ///     Ok(body)
    /// })?;
    /// assert_eq!(recipe.input_shapes(), recipe.output_shapes());
    /// b.relationship(vec![recipe])?;
    /// # Ok::<(), adam_lang::ModuleDiagnostic>(())
    /// ```
    pub fn method<F>(
        &self,
        inputs: &[CellSlot],
        outputs: &[CellSlot],
        emit: F,
    ) -> Result<MethodRecipe, ModuleDiagnostic>
    where
        F: Fn() -> anyhow::Result<DynSegment> + 'static,
    {
        let (input_slots, input_shapes) = self.slots(inputs)?;
        let (output_slots, output_shapes) = self.slots(outputs)?;
        if outputs.is_empty() {
            return Err(diagnostic("a method requires an output"));
        }
        Frame::new(
            emit().map_err(diagnostic)?,
            &input_shapes,
            &output_shapes,
            outputs.len() > 1,
            &self.types,
        )
        .map_err(diagnostic)?;
        let types = self.types.clone();
        let i_shapes = input_shapes.clone();
        let o_shapes = output_shapes.clone();
        let native = adam_rs::MethodRecipe::new(&input_slots, &output_slots, move |i, o| {
            frame_method(emit()?, i, o, &i_shapes, &o_shapes, &types)
        })
        .map_err(diagnostic)?;
        Ok(MethodRecipe {
            native,
            inputs: input_shapes,
            outputs: output_shapes,
        })
    }

    /// Appends a native relationship, preserving its alternative method ordering.
    ///
    /// # Errors
    /// Returns native ownership and relationship validation errors before mutation.
    /// - Complexity: O(m² × e) for m methods of at most e edges.
    ///
    /// # Examples
    /// See [`Self::method`].
    pub fn relationship(
        &mut self,
        methods: Vec<MethodRecipe>,
    ) -> Result<RelationshipSlot, ModuleDiagnostic> {
        self.native
            .relationship(methods.into_iter().map(|m| m.native).collect())
            .map_err(diagnostic)
    }

    /// Promotes a private ordinary cell through its single sole-writer method.
    ///
    /// # Errors
    /// Rejects formal, source, multi-output, foreign, or previously written targets.
    /// - Complexity: O(i + r) inputs and prior output summaries.
    ///
    /// # Examples
    /// See [`Self::method`]; pass a private single-output recipe to `out`.
    pub fn out(&mut self, writer: MethodRecipe) -> Result<RelationshipSlot, ModuleDiagnostic> {
        self.native.out(writer.native).map_err(diagnostic)
    }

    /// Attaches a value-returning or numeric-range filter with a fresh frame per instance.
    ///
    /// Argument zero is the candidate; subsequent arguments are the declared inputs.
    /// The callback obeys the native filter's purity contract.
    /// # Errors
    /// Rejects foreign/formal targets, invalid dependencies, unknown shapes or mismatched bodies.
    /// - Complexity: O(n + e + i) shape nodes, emitted reads and inputs.
    ///
    /// # Examples
    /// See [`Self::cell`]; a filter emitter may use `push_arg::<i32>(0)` for its candidate.
    pub fn filter<F>(
        &mut self,
        cell: &CellSlot,
        inputs: &[CellSlot],
        emit: F,
    ) -> Result<(), ModuleDiagnostic>
    where
        F: Fn() -> anyhow::Result<DynSegment> + 'static,
    {
        self.check_slot(cell)?;
        let (slots, shapes) = self.slots(inputs)?;
        let types = self.types.clone();
        let shape = cell.shape.clone();
        let candidate_shapes: Vec<_> = std::iter::once(shape.clone())
            .chain(shapes.iter().cloned())
            .collect();
        let body = emit().map_err(diagnostic)?;
        let range = body
            .peek_output_type_id()
            .and_then(|id| types.range_entry(id));
        if let Some(range) = range {
            if range.element_type_id != crate::parser::cell_type_id(&shape) {
                return Err(diagnostic("filter range element mismatch"));
            }
            let result = body.peek_stack_infos(1)[0].value_type.clone();
            crate::cell_value::RawFrame::new(body, &candidate_shapes, &result, &types)
                .map_err(diagnostic)?;
        } else {
            Frame::new(
                body,
                &candidate_shapes,
                std::slice::from_ref(&shape),
                false,
                &types,
            )
            .map_err(diagnostic)?;
        }
        self.native
            .filter(&cell.native, &slots, move |_, ids| {
                let body = emit()?;
                if body
                    .peek_output_type_id()
                    .and_then(|id| types.range_entry(id))
                    .is_some()
                {
                    crate::parser::build_range_filter(body, ids.to_vec(), &shapes, &shape, &types)
                } else {
                    crate::parser::build_value_filter(body, ids.to_vec(), &shapes, &shape, &types)
                }
            })
            .map_err(diagnostic)
    }

    /// Attaches a bool-returning requirement with a fresh frame per instance.
    ///
    /// The callback obeys the native requirement's purity contract.
    /// # Errors
    /// Rejects foreign/formal targets, unknown shapes and incompatible emitted signatures.
    /// Ordinary/source requirement execution errors occur during native attachment.
    /// - Complexity: O(n + e + i) descriptor nodes, reads and inputs.
    ///
    /// # Examples
    /// See [`Self::cell`]; a nullary requirement may emit `body.op0(|| true)`.
    pub fn require<F>(
        &mut self,
        cell: &CellSlot,
        inputs: &[CellSlot],
        emit: F,
    ) -> Result<(), ModuleDiagnostic>
    where
        F: Fn() -> anyhow::Result<DynSegment> + 'static,
    {
        self.types
            .validate_shape(&TypeShape::Named(TypeId::of::<bool>()))
            .map_err(diagnostic)?;
        self.check_slot(cell)?;
        let (slots, shapes) = self.slots(inputs)?;
        let types = self.types.clone();
        Frame::new(
            emit().map_err(diagnostic)?,
            &shapes,
            &[TypeShape::Named(TypeId::of::<bool>())],
            false,
            &types,
        )
        .map_err(diagnostic)?;
        self.native
            .require(&cell.native, &slots, move |ids| {
                crate::parser::build_requirement(emit()?, ids.to_vec(), &shapes, &types)
            })
            .map_err(diagnostic)
    }

    /// Registers conditional groups using a registered host leaf as the match key.
    ///
    /// Each included group must be transitively relationship-only. Keys and emitted
    /// results have exactly type `T`; callbacks obey native match-expression purity.
    /// # Errors
    /// Rejects unknown key/input shapes, incompatible bodies or native group errors.
    /// - Complexity: O(n + e + i + g + k) descriptors, reads, inputs, groups and keys.
    ///
    /// # Examples
    /// ```
    /// let mut b = adam_lang::CompositionBuilder::new(adam_lang::TypeRegistry::new());
    /// b.conditional(&[], || {
    ///     let mut body = cel_runtime::DynSegment::new::<()>();
    ///     body.op0(|| true);
    ///     Ok(body)
    /// }, vec![(vec![true], vec![])], vec![])?;
    /// # Ok::<(), adam_lang::ModuleDiagnostic>(())
    /// ```
    pub fn conditional<T, F>(
        &mut self,
        inputs: &[CellSlot],
        emit: F,
        branches: Vec<(Vec<T>, Vec<RelationshipSlot>)>,
        default: Vec<RelationshipSlot>,
    ) -> Result<(), ModuleDiagnostic>
    where
        T: Any + Clone + PartialEq,
        F: Fn() -> anyhow::Result<DynSegment> + 'static,
    {
        let shape = TypeShape::Named(TypeId::of::<T>());
        self.types.validate_shape(&shape).map_err(diagnostic)?;
        self.conditional_frame(inputs, emit, shape, branches, default)
    }

    /// Registers conditional groups using complete canonical compound keys.
    ///
    /// The witness supplies both key and result identity. Compound equality and cloning
    /// must be available; callable keys without equality are rejected.
    /// # Errors
    /// Rejects unknown shapes, non-compound witnesses, missing capabilities,
    /// incompatible emitted results, or native group errors before recipe mutation.
    /// - Complexity: O(n + e + i + g + k) descriptors, reads, inputs, groups and key values.
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::{Array, ArrayOf, Leaf, DynSegment};
    /// let mut b = adam_lang::CompositionBuilder::new(adam_lang::TypeRegistry::new());
    /// b.conditional_compound::<ArrayOf<Leaf<bool>>, _>(&[], || {
    ///     let mut body = DynSegment::new::<()>();
    ///     body.op0_repr::<ArrayOf<Leaf<bool>>, _>(|| Array::from_vec(vec![true]))?;
    ///     Ok(body)
    /// }, vec![(vec![Array::from_vec(vec![true])], vec![])], vec![])?;
    /// # Ok::<(), adam_lang::ModuleDiagnostic>(())
    /// ```
    pub fn conditional_compound<R, F>(
        &mut self,
        inputs: &[CellSlot],
        emit: F,
        branches: Vec<(Vec<R::Value>, Vec<RelationshipSlot>)>,
        default: Vec<RelationshipSlot>,
    ) -> Result<(), ModuleDiagnostic>
    where
        R: cel_runtime::ValueRepr,
        F: Fn() -> anyhow::Result<DynSegment> + 'static,
    {
        let shape = self.types.shape_of(&R::value_type()).map_err(diagnostic)?;
        if matches!(shape, TypeShape::Named(_)) {
            return Err(diagnostic("expected a compound conditional witness"));
        }
        let descriptor = self.types.checked_value_type(&shape).map_err(diagnostic)?;
        CellValue::validate(&descriptor).map_err(diagnostic)?;
        let branches = branches
            .into_iter()
            .map(|(keys, groups)| {
                let keys = keys
                    .into_iter()
                    .map(|key| {
                        cel_runtime::OwnedValue::new::<R>(key)
                            .with_value_type(descriptor.clone())
                            .map(CellValue)
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                Ok((keys, groups))
            })
            .collect::<anyhow::Result<_>>()
            .map_err(diagnostic)?;
        self.conditional_frame(inputs, emit, shape, branches, default)
    }

    /// Prepares a conditional frame with a checked native storage result.
    ///
    /// - Precondition: `shape` is registered and its storage is exactly `T`.
    /// # Errors
    /// Rejects invalid inputs, emitted signatures or native group structure.
    /// - Complexity: O(n + e + i + g + k) descriptors, reads, inputs, groups and keys.
    fn conditional_frame<T, F>(
        &mut self,
        inputs: &[CellSlot],
        emit: F,
        shape: TypeShape,
        branches: Vec<(Vec<T>, Vec<RelationshipSlot>)>,
        default: Vec<RelationshipSlot>,
    ) -> Result<(), ModuleDiagnostic>
    where
        T: Any + Clone + PartialEq,
        F: Fn() -> anyhow::Result<DynSegment> + 'static,
    {
        let (slots, shapes) = self.slots(inputs)?;
        let types = self.types.clone();
        Frame::new(
            emit().map_err(diagnostic)?,
            &shapes,
            std::slice::from_ref(&shape),
            false,
            &types,
        )
        .map_err(diagnostic)?;
        self.native
            .conditional(
                &slots,
                move |ids| {
                    let frame = RefCell::new(Frame::new(
                        emit()?,
                        &shapes,
                        std::slice::from_ref(&shape),
                        false,
                        &types,
                    )?);
                    Ok(MatchExpr::new(
                        ids.to_vec(),
                        storage_ids(&shapes),
                        TypeId::of::<T>(),
                        |a, b| a.downcast_ref::<T>() == b.downcast_ref::<T>(),
                        move |args| Ok(frame.borrow_mut().call(args)?.remove(0)),
                    ))
                },
                branches,
                default,
            )
            .map_err(diagnostic)
    }

    /// Includes a typed child using complete named bindings before native mutation.
    ///
    /// # Errors
    /// Rejects foreign handles, unknown/missing/duplicate names, aliases, full-shape
    /// mismatches and native kind/structural errors. Rejection leaves the recipe unchanged.
    /// - Complexity: O(b + n) bindings and compared shape nodes, plus native local preflight.
    ///
    /// # Examples
    /// ```
    /// let types = adam_lang::TypeRegistry::new();
    /// let child = adam_lang::CompositionBuilder::new(types.clone()).finish()?;
    /// let mut parent = adam_lang::CompositionBuilder::new(types);
    /// parent.include(&child, &[])?;
    /// # Ok::<(), adam_lang::ModuleDiagnostic>(())
    /// ```
    pub fn include(
        &mut self,
        child: &TypedTemplate,
        bindings: &[(&str, CellSlot)],
    ) -> Result<TemplateInclusion, ModuleDiagnostic> {
        let mut names = HashSet::new();
        let mut actuals = HashSet::new();
        for (name, slot) in bindings {
            self.check_slot(slot)?;
            let formal = child
                .metadata
                .formals
                .get(*name)
                .ok_or_else(|| diagnostic("unknown parameter"))?;
            if !names.insert(*name) || !actuals.insert(&slot.native) {
                return Err(diagnostic("duplicate parameter or aliased argument"));
            }
            if formal.shape != slot.shape {
                return Err(diagnostic("complete shape mismatch"));
            }
        }
        if names.len() != child.metadata.formals.len() {
            return Err(diagnostic("missing parameter"));
        }
        let native: Vec<_> = bindings
            .iter()
            .map(|(name, slot)| (*name, slot.native.clone()))
            .collect();
        let inclusion = self
            .native
            .include(child.native.clone(), &native)
            .map_err(diagnostic)?;
        self.children.push(child.metadata.clone());
        Ok(inclusion)
    }

    /// Finalizes an immutable native recipe and its complete local metadata.
    ///
    /// # Errors
    /// Rejects included parameters that became private outputs after inclusion.
    /// - Complexity: O(n + e) local declarations and edges, without DAG expansion.
    ///
    /// # Examples
    /// See the [module example](self).
    pub fn finish(self) -> Result<TypedTemplate, ModuleDiagnostic> {
        let native = Rc::new(self.native.finish().map_err(diagnostic)?);
        let metadata = Rc::new(Metadata {
            id: native.id(),
            cells: self.cells,
            formals: self.formals,
            children: self.children,
            label: self.label,
        });
        Ok(TypedTemplate { native, metadata })
    }

    /// Validates a complete shape before selecting its storage constructor.
    ///
    /// # Errors
    /// Rejects every unregistered leaf, including nested callable signatures.
    /// - Complexity: O(n) shape nodes.
    fn adapters(&self, shape: &TypeShape) -> Result<LeafAdapters, ModuleDiagnostic> {
        self.types.validate_shape(shape).map_err(diagnostic)?;
        Ok(match shape {
            TypeShape::Named(id) => {
                self.types
                    .entry_by_type_id(*id)
                    .expect("validated leaf")
                    .template
            }
            _ => LeafAdapters::of::<CellValue>(),
        })
    }

    /// Records a successfully declared local shape.
    ///
    /// - Complexity: O(n) cloned shape nodes and expected O(1) map insertion.
    fn record(&mut self, native: adam_rs::CellSlot, shape: TypeShape) -> CellSlot {
        self.cells.insert(native.clone(), shape.clone());
        CellSlot { native, shape }
    }

    /// Validates one local owner and complete shape without allocating edge vectors.
    ///
    /// # Errors
    /// Rejects foreign or unregistered slots before emitter invocation.
    /// - Complexity: O(n) shape nodes.
    fn check_slot(&self, slot: &CellSlot) -> Result<(), ModuleDiagnostic> {
        if self.cells.get(&slot.native) != Some(&slot.shape) {
            return Err(diagnostic("foreign cell slot"));
        }
        self.types.validate_shape(&slot.shape).map_err(diagnostic)
    }

    /// Checks ownership and resolves positional slots without relying on carrier equality.
    ///
    /// # Errors
    /// Rejects foreign or unregistered slots before emitter invocation.
    /// - Complexity: O(i + n) slots and shape nodes.
    fn slots(
        &self,
        slots: &[CellSlot],
    ) -> Result<(Vec<adam_rs::CellSlot>, Vec<TypeShape>), ModuleDiagnostic> {
        for slot in slots {
            self.check_slot(slot)?;
        }
        Ok((
            slots.iter().map(|s| s.native.clone()).collect(),
            slots.iter().map(|s| s.shape.clone()).collect(),
        ))
    }
}

/// Owns a reusable native definition, complete formal/local shapes and source metadata.
///
/// No live runtime is cloned. Metadata retains compact child edges and releases them
/// iteratively on last-owner drop. See the [module example](self).
pub struct TypedTemplate {
    native: Rc<SheetTemplate>,
    metadata: Rc<Metadata>,
}

impl TypedTemplate {
    /// Instantiates into an existing source-free or parsed destination.
    ///
    /// Named bindings must be complete, distinct, registered in the destination and
    /// exactly shape-compatible. Native checks enforce cell kinds and specialization.
    /// Binding/structural rejection precedes all mutation. Later initializer/body
    /// factory errors leave the destination safely discardable, not rolled back.
    /// Private names never enter root lookup maps; full private shapes and flat
    /// occurrence/method origins survive definition and returned instance drop.
    /// # Errors
    /// Reports binding, native structural/capacity, initializer or body-factory failures.
    /// Native diagnostics retain span-free labels unless a real label was supplied.
    /// - Complexity: Expected O(b + n + d + o + c + r + m + s) bindings, shape nodes,
    ///   unique metadata DAG size, occurrences, local cell mappings, relationships and
    ///   surviving methods and previously retained sources s, plus native expansion
    ///   and factory costs.
    ///
    /// # Examples
    /// See the [module example](self).
    pub fn instantiate(
        &self,
        destination: &mut ParsedSheet,
        bindings: &[(&str, CellId)],
    ) -> Result<Instance, ModuleDiagnostic> {
        let mut names = HashSet::new();
        let mut actuals = HashSet::new();
        for (name, id) in bindings {
            let formal = self
                .metadata
                .formals
                .get(*name)
                .ok_or_else(|| self.error("unknown parameter"))?;
            if !names.insert(*name) || !actuals.insert(*id) {
                return Err(self.error("duplicate parameter or aliased argument"));
            }
            if destination.cell_shape(*id) != Some(&formal.shape) {
                return Err(self.error("complete shape mismatch or unregistered actual"));
            }
            destination
                .validate_cell_shape(*id, &formal.shape)
                .map_err(|e| self.error(e))?;
        }
        if names.len() != self.metadata.formals.len() {
            return Err(self.error("missing parameter"));
        }
        let instance = self
            .native
            .instantiate(&mut destination.sheet, bindings)
            .map_err(|e| self.error(e))?;
        let definitions = self.definitions();
        let mut occurrences = Vec::with_capacity(instance.occurrences().len());
        let mut source_ids: HashSet<_> = destination.sources.iter().cloned().collect();
        for occurrence in instance.occurrences() {
            let definition = definitions[occurrence.template()];
            for (slot, shape) in &definition.cells {
                let id = instance
                    .cell(occurrence.id(), slot)
                    .expect("native owner-checked mapping");
                destination.cell_shapes.insert(id, shape.clone());
                destination
                    .cell_origins
                    .entry(id)
                    .or_default()
                    .push(CellOrigin {
                        occurrence: occurrence.id().clone(),
                        slot: slot.clone(),
                    });
            }
            if let Some(location) = &definition.label.location
                && source_ids.insert(location.source.clone())
            {
                destination.sources.push(location.source.clone());
            }
            occurrences.push(OccurrenceOrigin {
                id: occurrence.id().clone(),
                parent: occurrence.parent().cloned(),
                template: occurrence.template().clone(),
                inclusion_site: occurrence.inclusion_site().cloned(),
                label: definition.label.clone(),
            });
        }
        destination.method_origins.extend(
            instance
                .method_origins()
                .map(|(r, m, origin)| ((r, m), origin)),
        );
        destination
            .instance_origins
            .push(InstanceOrigin { occurrences });
        Ok(instance)
    }

    /// Returns native relationship-only eligibility before any source specialization.
    ///
    /// # Examples
    /// ```
    /// let template = adam_lang::CompositionBuilder::new(adam_lang::TypeRegistry::new()).finish()?;
    /// assert!(template.is_relationship_only());
    /// # Ok::<(), adam_lang::ModuleDiagnostic>(())
    /// ```
    pub fn is_relationship_only(&self) -> bool {
        self.native.is_relationship_only()
    }

    /// Indexes unique immutable metadata nodes without expanding a second graph.
    ///
    /// - Complexity: O(d) time and space in unique metadata nodes and inclusion edges.
    fn definitions(&self) -> HashMap<&TemplateId, &Metadata> {
        let mut result = HashMap::new();
        let mut pending = vec![self.metadata.as_ref()];
        while let Some(node) = pending.pop() {
            if result.insert(&node.id, node).is_none() {
                pending.extend(node.children.iter().map(Rc::as_ref));
            }
        }
        result
    }

    /// Labels an instantiation failure without inventing a source range.
    ///
    /// - Complexity: O(m) formatted message bytes.
    fn error(&self, error: impl std::fmt::Display) -> ModuleDiagnostic {
        ModuleDiagnostic {
            message: error.to_string(),
            labels: vec![self.metadata.label.clone()],
        }
    }
}

/// Produces a source-free diagnostic with no invented location.
///
/// - Complexity: O(m) formatted message bytes.
pub(crate) fn diagnostic(error: impl std::fmt::Display) -> ModuleDiagnostic {
    let message = error.to_string();
    ModuleDiagnostic {
        labels: vec![SourceLabel {
            location: None,
            label: message.clone(),
        }],
        message,
    }
}

/// Converts complete shapes to the storage identities used only at the native boundary.
///
/// - Complexity: O(i) shapes.
pub(crate) fn storage_ids(shapes: &[TypeShape]) -> Vec<TypeId> {
    shapes.iter().map(crate::parser::cell_type_id).collect()
}

/// Prepares a fresh canonical method frame and wraps the native callback boundary.
///
/// - Precondition: All named shapes are registered.
/// # Errors
/// Rejects incompatible emitted signatures or unavailable cell capabilities.
/// - Complexity: O(n + e + i + o) signature nodes, reads and edges.
fn frame_method(
    body: DynSegment,
    inputs: &[CellId],
    outputs: &[CellId],
    input_shapes: &[TypeShape],
    output_shapes: &[TypeShape],
    types: &TypeRegistry,
) -> anyhow::Result<Method> {
    let frame = RefCell::new(Frame::new(
        body,
        input_shapes,
        output_shapes,
        outputs.len() > 1,
        types,
    )?);
    Ok(Method::new(
        inputs.to_vec(),
        outputs.to_vec(),
        storage_ids(input_shapes),
        storage_ids(output_shapes),
        move |args| frame.borrow_mut().call(args),
    ))
}
