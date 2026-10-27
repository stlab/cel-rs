//! Private canonical cell storage and construction-time frozen Adam/CEL adapters.
//!
//! Primitive cells remain concrete Rust values. Compound access passes through
//! `ParsedSheet`'s checked representation-witness boundary.
use std::any::{Any, TypeId};

use cel_runtime::{DynSegment, OwnedValue, ValueType, prepared_segment::PreparedRawSegment};

use crate::{TypeRegistry, type_registry::TypeShape};

/// Formats a canonical compound cell at the checked host boundary.
///
/// This also supports hosts that retain the underlying `adam_rs::Sheet`.
/// # Errors
/// Rejects an unknown cell, concrete leaf storage, or absent debug capability.
/// - Complexity: O(v + s) for v contained values and s output bytes.
///
/// # Examples
/// ```
/// let parsed = adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(), cel_parser::OpLookup::new())
///     .parse_str("sheet s { cell point: (i32, i32) = (3, 4); }").unwrap();
/// assert_eq!(adam_lang::debug_compound_cell(&parsed.sheet, parsed.cell_names["point"].0).unwrap(), "(3, 4)");
/// ```
pub fn debug_compound_cell(sheet: &adam_rs::Sheet, id: adam_rs::CellId) -> anyhow::Result<String> {
    sheet.read::<CellValue>(id)?.0.try_debug()
}

/// Prepares canonical compound-cell formatting from the cell owner's debug capability.
///
/// The returned closure reads the current value, checks its complete shape, and
/// formats through the retained capability. It does not consume or clone values.
///
/// # Errors
/// Rejects an unknown cell, noncompound storage, or unavailable debug capability.
/// The closure also rejects a different shape or missing cell in its input sheet.
/// - Complexity: Preparation O(1); each call O(n + v + s) for n type nodes,
///   v contained values and s output bytes, excluding leaf formatter costs.
///
/// # Examples
/// ```
/// let parsed = adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(), cel_parser::OpLookup::new())
///     .parse_str("sheet s { cell xs: [i32]; }").unwrap();
/// let format = adam_lang::prepare_compound_cell_formatter(&parsed, parsed.cell_names["xs"].0).unwrap();
/// assert_eq!(format(&parsed).unwrap(), "[]");
/// ```
pub fn prepare_compound_cell_formatter(
    sheet: &adam_rs::Sheet,
    id: adam_rs::CellId,
) -> anyhow::Result<impl Fn(&adam_rs::Sheet) -> anyhow::Result<String> + use<>> {
    let formatter = sheet.read::<CellValue>(id)?.0.prepare_debug()?;
    Ok(move |sheet: &adam_rs::Sheet| formatter.format(&sheet.read::<CellValue>(id)?.0))
}

/// Owns a compound value whose cloning and equality capabilities are bound.
///
/// Only checked host writes and frozen output adapters construct this private type.
pub(crate) struct CellValue(pub(crate) OwnedValue);

impl CellValue {
    /// Rejects missing cell capabilities before sheet insertion.
    ///
    /// # Errors
    /// Returns a diagnostic for missing cloning or equality.
    pub(crate) fn validate(ty: &ValueType) -> anyhow::Result<()> {
        anyhow::ensure!(
            ty.can_clone_for_argument(),
            "Adam cell clone capability unavailable for `{}`",
            ty.type_name()
        );
        anyhow::ensure!(
            ty.can_compare(),
            "Adam cell equality capability unavailable for `{}`",
            ty.type_name()
        );
        Ok(())
    }
}

impl Clone for CellValue {
    /// Clones through the previously validated frozen ownership plan.
    ///
    /// - Complexity: O(v) for v contained values, excluding leaf clone costs.
    fn clone(&self) -> Self {
        Self(self.0.try_clone().expect("validated cell clone capability"))
    }
}

impl PartialEq for CellValue {
    /// Compares values at a shape-validated cell boundary without repeating type checks.
    ///
    /// - Complexity: O(v) for v contained values, excluding leaf equality costs.
    fn eq(&self, other: &Self) -> bool {
        // SAFETY: This type is private; host writes and all prepared outputs bind
        // the complete cell shape before entering Adam's type-erased storage.
        unsafe { self.0.eq_bound(&other.0) }
    }
}

impl std::fmt::Debug for CellValue {
    /// Formats a value through its optional canonical debug capability.
    ///
    /// - Complexity: O(v + s) for v contained values and s formatted bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.try_debug() {
            Ok(value) => f.write_str(&value),
            Err(_) => f.write_str("<compound value>"),
        }
    }
}

/// Describes a construction-time validated move from canonical result to Adam storage.
enum Output {
    /// A registered primitive or foreign concrete Rust leaf.
    Leaf(unsafe fn(OwnedValue) -> Box<dyn Any>),
    /// Private whole-value ownership of a recursive compound.
    Compound,
}

impl Output {
    /// Moves one already-bound canonical result into its Adam storage carrier.
    fn store(&self, value: OwnedValue) -> Box<dyn Any> {
        match self {
            // SAFETY: The result identity was bound during construction.
            Self::Leaf(box_value) => unsafe { box_value(value) },
            Self::Compound => Box::new(CellValue(value)),
        }
    }
}

/// Executes a frozen canonical raw frame behind Adam's existing host callback boundary.
pub(crate) struct Frame {
    body: RawFrame,
    outputs: Vec<Output>,
    split: bool,
}

/// Binds Adam storage addresses to the canonical runtime's one raw execution engine.
pub(crate) struct RawFrame {
    body: PreparedRawSegment,
    inputs: Vec<Input>,
    pointers: Vec<*const u8>,
}

/// Retains the complete storage identity required at an erased host boundary.
enum Input {
    /// A concrete registered Rust leaf.
    Leaf(TypeId),
    /// A private carrier with an exact recursive canonical shape.
    Compound(ValueType),
}

impl Input {
    /// Checks storage identity and complete shape without binding or reading raw pointers.
    ///
    /// # Errors
    /// Rejects a different concrete type, compound carrier, or recursive shape.
    /// - Complexity: O(n) for n compound descriptor nodes; O(1) for leaves.
    fn validate(&self, input: &dyn Any) -> anyhow::Result<()> {
        let matches = match self {
            Self::Leaf(id) => input.type_id() == *id,
            Self::Compound(expected) => input
                .downcast_ref::<CellValue>()
                .is_some_and(|value| value.0.value_type().same_shape(expected)),
        };
        anyhow::ensure!(matches, "Adam argument storage or shape mismatch");
        Ok(())
    }
}

impl Frame {
    /// Validates complete reads, outputs, and cell capabilities before execution.
    ///
    /// # Errors
    /// Rejects mismatched recursive representations or missing required capabilities.
    /// - Complexity: O(n) in signature nodes and emitted reads.
    pub(crate) fn new(
        body: DynSegment,
        inputs: &[TypeShape],
        outputs: &[TypeShape],
        split: bool,
        types: &TypeRegistry,
    ) -> anyhow::Result<Self> {
        let result = if split {
            types.value_type(&TypeShape::Tuple(outputs.to_vec()))
        } else {
            types.value_type(&outputs[0])
        };
        let mut adapters = Vec::with_capacity(outputs.len());
        for shape in outputs {
            match shape {
                TypeShape::Named(id) => adapters.push(Output::Leaf(
                    types
                        .entry_by_type_id(*id)
                        .expect("resolved leaf")
                        .box_owned,
                )),
                _ => {
                    CellValue::validate(&types.value_type(shape))?;
                    adapters.push(Output::Compound);
                }
            }
        }
        Ok(Self {
            body: RawFrame::new(body, inputs, &result, types)?,
            outputs: adapters,
            split,
        })
    }

    /// Executes raw borrowed inputs already type-checked by Adam's callback boundary.
    ///
    /// - Precondition: Input storage matches the exact shapes supplied to construction.
    /// # Errors
    /// Propagates CEL operation failures after canonical cleanup.
    /// - Complexity: O(a) pointer binding plus body costs and output moves.
    pub(crate) fn call(&mut self, inputs: &[&dyn Any]) -> anyhow::Result<Vec<Box<dyn Any>>> {
        let result = self.body.call(inputs)?;
        self.store(result)
    }

    /// Executes a filter frame without allocating a merged host argument list.
    ///
    /// - Precondition: The candidate and dependencies match the prepared storage types.
    /// # Errors
    /// Propagates operation failures.
    /// - Complexity: O(a) pointer binding plus body costs and output moves.
    pub(crate) fn call_with_prefix(
        &mut self,
        value: &dyn Any,
        inputs: &[&dyn Any],
    ) -> anyhow::Result<Vec<Box<dyn Any>>> {
        let result = self.body.call_with_prefix(value, inputs)?;
        self.store(result)
    }

    /// Moves a whole result or its bound fields into the requested output cells.
    ///
    /// # Errors
    /// Returns an error if a destructuring result is not a tuple.
    /// - Complexity: O(n + s) for n output fields and s representation bytes.
    fn store(&self, result: OwnedValue) -> anyhow::Result<Vec<Box<dyn Any>>> {
        if self.split {
            Ok(result
                .into_fields()?
                .into_iter()
                .zip(&self.outputs)
                .map(|(value, output)| output.store(value))
                .collect())
        } else {
            Ok(vec![self.outputs[0].store(result)])
        }
    }
}

impl RawFrame {
    /// Validates every positional read and binds the result's capabilities before execution.
    ///
    /// # Errors
    /// Rejects a mismatched signature or missing borrowed cloning.
    /// - Complexity: O(n) in descriptor nodes and emitted reads.
    pub(crate) fn new(
        mut body: DynSegment,
        inputs: &[TypeShape],
        result: &ValueType,
        types: &TypeRegistry,
    ) -> anyhow::Result<Self> {
        let input_types: Vec<_> = inputs.iter().map(|shape| types.value_type(shape)).collect();
        body.annotate_top(result.clone())?;
        Ok(Self {
            body: PreparedRawSegment::new(body, &input_types, result)?,
            inputs: inputs
                .iter()
                .zip(input_types)
                .map(|(shape, value_type)| match shape {
                    TypeShape::Named(id) => Input::Leaf(*id),
                    _ => Input::Compound(value_type),
                })
                .collect(),
            pointers: vec![std::ptr::null(); inputs.len()],
        })
    }

    /// Executes an Adam-validated host frame without recursive checks or conversions.
    ///
    /// - Precondition: Every input matches its preparation-time storage adapter.
    /// # Errors
    /// Propagates operation failures after cleanup.
    /// - Complexity: O(a) pointer binding plus body costs.
    pub(crate) fn call(&mut self, inputs: &[&dyn Any]) -> anyhow::Result<OwnedValue> {
        self.call_arguments(inputs.iter().copied(), inputs.len())
    }

    /// Prepends a filter candidate without allocating a merged host argument list.
    ///
    /// - Precondition: The candidate and dependencies match their prepared storage types.
    /// # Errors
    /// Propagates operation failures.
    /// - Complexity: O(a) pointer binding plus body costs.
    pub(crate) fn call_with_prefix(
        &mut self,
        value: &dyn Any,
        inputs: &[&dyn Any],
    ) -> anyhow::Result<OwnedValue> {
        self.call_arguments(
            std::iter::once(value).chain(inputs.iter().copied()),
            inputs.len() + 1,
        )
    }

    /// Checks every erased input before binding any address or executing the prefixed frame.
    ///
    /// # Errors
    /// Rejects missing or extra inputs, wrong scalar identities, compound carriers, or
    /// recursive shapes on every invocation; propagates operation failures after cleanup.
    /// Rejection leaves previously bound pointers unused and permits later valid calls.
    /// - Complexity: O(a + n) validation for a inputs and n compound descriptor nodes,
    ///   plus pointer binding and body costs.
    pub(crate) fn call_checked_with_prefix(
        &mut self,
        value: &dyn Any,
        inputs: &[&dyn Any],
    ) -> anyhow::Result<OwnedValue> {
        anyhow::ensure!(
            inputs.len().checked_add(1) == Some(self.inputs.len()),
            "Adam argument count mismatch"
        );
        for (input, expected) in std::iter::once(value)
            .chain(inputs.iter().copied())
            .zip(&self.inputs)
        {
            expected.validate(input)?;
        }
        self.call_with_prefix(value, inputs)
    }

    /// Binds borrowed host addresses into the reusable canonical pointer slots.
    ///
    /// - Precondition: Input count and types match preparation.
    /// # Errors
    /// Propagates operation failures after raw frame cleanup.
    /// - Complexity: O(a) pointer binding plus body costs.
    fn call_arguments<'a>(
        &mut self,
        inputs: impl Iterator<Item = &'a dyn Any>,
        arity: usize,
    ) -> anyhow::Result<OwnedValue> {
        debug_assert_eq!(arity, self.pointers.len());
        for ((input, adapter), pointer) in inputs.zip(&self.inputs).zip(&mut self.pointers) {
            let data = std::ptr::from_ref(input).cast::<()>();
            *pointer = match adapter {
                Input::Compound(_) => {
                    // SAFETY: Adam's internal boundary or the checked public
                    // adapter verifies the private carrier and complete shape.
                    unsafe { &*data.cast::<CellValue>() }.0.as_ptr()
                }
                Input::Leaf(_) => data.cast(),
            };
        }
        // SAFETY: Construction validates every read. The internal callback boundary
        // or checked entry point guarantees storage types and complete compound shapes.
        unsafe { self.body.call(&self.pointers) }
    }
}
