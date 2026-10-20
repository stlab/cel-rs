//! Checks existing Any/tuple host boundaries before canonical execution.
//!
//! Prepare a complete descriptor, import external owners, then execute only
//! after import succeeds. Export reconstructs legacy headers explicitly; legacy
//! wrappers never become structural canonical fields or array elements.

use std::alloc::Layout;
use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::ptr;

use crate::array::RawArray;
use crate::container_execution::ContainerPlan;
use crate::owned_value::{Allocation, OwnedValue};
use crate::value_type::{ValueKind, ValueType};
use crate::{
    ArrayElementType, DynElementSpec, DynamicArray, DynamicSequence, ElementCloner, ElementDebug,
    ElementEq,
};

/// Retains monomorphized ownership adapters, independent of optional capabilities.
#[derive(Clone, Debug)]
pub(crate) struct LeafBridge {
    pub(crate) name: Cow<'static, str>,
    write: unsafe fn(Box<dyn Any>, *mut u8),
    read: unsafe fn(*const u8) -> Box<dyn Any>,
    array_drop: crate::dyn_segment::RawDropper,
    pub(crate) clone: Option<ElementCloner>,
    pub(crate) eq: Option<ElementEq>,
    pub(crate) debug: Option<ElementDebug>,
}

impl LeafBridge {
    /// Borrows the concrete leaf's name for diagnostic compatibility.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }
    /// Prepares native box moves without imposing clone, equality, or debug bounds.
    pub(crate) fn new<T: 'static>() -> Self {
        Self {
            name: Cow::Borrowed(std::any::type_name::<T>()),
            write: crate::element_writer_for::<T>(),
            read: |source| Box::new(unsafe { source.cast::<T>().read() }),
            array_drop: crate::dyn_segment::raw_dropper_for::<T>(),
            clone: None,
            eq: None,
            debug: None,
        }
    }
}

/// Binds complete types for checked external ownership transfers.
///
/// This is not an interpreter operation. Callers must finish every checked
/// input conversion before evaluating an expression. Optional capabilities
/// constrain only export into the preexisting capability-rich tuple wrapper.
pub(crate) struct HostValuePlan {
    value_type: ValueType,
    children: Vec<HostValuePlan>,
}

impl HostValuePlan {
    /// Clones a checked borrowed host value without consuming its original.
    ///
    /// Complete representation and recursive leaf capabilities are checked
    /// before invoking any clone. A clone panic destroys completed canonical
    /// prefixes while leaving the borrowed external owner intact.
    ///
    /// # Errors
    /// Rejects mismatched external shapes or missing leaf clone capabilities.
    /// - Complexity: O(v * d + s) time and O(s) clone storage, excluding leaf
    ///   clone costs, for v values, external array metadata depth d (at least 1),
    ///   and s representation bytes.
    pub(crate) fn clone_any(&self, input: &dyn Any) -> anyhow::Result<OwnedValue> {
        anyhow::ensure!(
            input.type_id() == self.external_type().0,
            "external box type mismatch"
        );
        let source = input as *const dyn Any as *const u8;
        unsafe { self.validate_raw(source)? };
        self.validate_clone()?;
        // SAFETY: The complete external shape and every leaf clone are checked.
        unsafe { self.clone_external(source) }
    }

    /// Validates recursive leaf clone capability independently of aggregate adapters.
    ///
    /// # Errors
    /// Rejects any leaf without a frozen clone plan, even if an enclosing
    /// canonical aggregate has a whole-representation adapter.
    /// - Complexity: O(n) time and O(d) call space for n nodes and depth d.
    fn validate_clone(&self) -> anyhow::Result<()> {
        if matches!(self.value_type.kind(), ValueKind::Leaf(_)) {
            anyhow::ensure!(
                self.value_type.can_clone(),
                "host leaf clone capability unavailable"
            );
        }
        for child in &self.children {
            child.validate_clone()?;
        }
        Ok(())
    }

    /// Clones checked external storage into guarded canonical storage.
    ///
    /// # Safety
    /// Source must remain a live external representation matching this plan;
    /// every recursive leaf must pass `validate_clone`.
    ///
    /// # Errors
    /// Propagates container allocation-layout errors, destroying completed clones.
    /// - Complexity: O(v + s) time and O(s) clone storage, excluding leaf costs.
    unsafe fn clone_external(&self, source: *const u8) -> anyhow::Result<OwnedValue> {
        match self.value_type.kind() {
            ValueKind::Leaf(_) => {
                let allocation = Allocation::new(self.value_type.layout());
                unsafe {
                    crate::owned_value::clone_raw(
                        &self.value_type,
                        source,
                        allocation.pointer.as_ptr(),
                    );
                    Ok(OwnedValue::from_allocation(
                        allocation,
                        self.value_type.clone(),
                    ))
                }
            }
            ValueKind::Tuple(_) => {
                let sequence = unsafe { &*source.cast::<DynamicSequence>() };
                let container = ContainerPlan::tuple(self.value_type.clone())?;
                unsafe {
                    container.construct(|index, destination| {
                        let mut owner = sequence
                            .read_element_at(sequence.shape()[index].offset, |source| {
                                self.children[index].clone_external(source)
                            })?;
                        owner.move_into(destination);
                        Ok(())
                    })
                }
            }
            ValueKind::Array(_) => {
                let array = unsafe { &*source.cast::<DynamicArray>() };
                let child = &self.children[0];
                let container = ContainerPlan::array(child.value_type.clone(), array.len())?;
                let (pointer, _, _) = array.allocation_parts();
                unsafe {
                    container.construct(|index, destination| {
                        let mut owner = child.clone_external(
                            pointer.as_ptr().add(index * child.external_type().1.size()),
                        )?;
                        owner.move_into(destination);
                        Ok(())
                    })
                }
            }
            ValueKind::Callable { .. } => unreachable!(),
        }
    }

    /// Prepares leaves, tuples, and supported legacy array conversions.
    ///
    /// # Errors
    /// Rejects legacy wrapper leaves, callables, and tuple-valued array elements.
    /// Legacy arrays can hold `DynamicSequence` elements, but this bridge does
    /// not prepare their recursive structural tuple metadata and conversion.
    /// Other explicitly opaque registered aggregates remain leaves.
    ///
    /// - Complexity: O(n) time and storage for n descriptor nodes.
    pub(crate) fn new(value_type: ValueType) -> anyhow::Result<Self> {
        let children = match value_type.kind() {
            ValueKind::Leaf(id) => {
                anyhow::ensure!(
                    *id != TypeId::of::<DynamicSequence>() && *id != TypeId::of::<DynamicArray>(),
                    "legacy wrappers require complete structural descriptors"
                );
                Vec::new()
            }
            ValueKind::Tuple(fields) => fields
                .iter()
                .map(|field| Self::new(field.value_type.clone()))
                .collect::<anyhow::Result<_>>()?,
            ValueKind::Array(element) => {
                anyhow::ensure!(
                    matches!(element.kind(), ValueKind::Leaf(_) | ValueKind::Array(_)),
                    "host bridge does not support tuple or callable array elements"
                );
                vec![Self::new(element.clone())?]
            }
            ValueKind::Callable { .. } => anyhow::bail!("callable host conversion is not prepared"),
        };
        Ok(Self {
            value_type,
            children,
        })
    }

    /// Returns the external wrapper's concrete identity and layout, not canonical layout.
    fn external_type(&self) -> (TypeId, Layout) {
        match self.value_type.kind() {
            ValueKind::Leaf(id) => (*id, self.value_type.layout()),
            ValueKind::Tuple(_) => (
                TypeId::of::<DynamicSequence>(),
                Layout::new::<DynamicSequence>(),
            ),
            ValueKind::Array(_) => (TypeId::of::<DynamicArray>(), Layout::new::<DynamicArray>()),
            ValueKind::Callable { .. } => unreachable!(),
        }
    }

    /// Validates a live external value recursively without moving or cloning it.
    ///
    /// # Safety
    /// `source` must contain the live external representation from `external_type`.
    ///
    /// # Errors
    /// Rejects any nested identity, layout, arity, or element descriptor mismatch.
    /// - Complexity: O(v * d) worst-case time and O(d) call space for v values
    ///   and nesting depth d (at least 1), including repeated external array metadata.
    unsafe fn validate_raw(&self, source: *const u8) -> anyhow::Result<()> {
        match self.value_type.kind() {
            ValueKind::Leaf(_) => Ok(()),
            ValueKind::Tuple(_) => {
                let sequence = unsafe { &*source.cast::<DynamicSequence>() };
                anyhow::ensure!(
                    sequence.arity() == self.children.len(),
                    "external tuple arity mismatch"
                );
                for (element, child) in sequence.shape().iter().zip(&self.children) {
                    let (id, layout) = child.external_type();
                    anyhow::ensure!(
                        element.type_id == id
                            && element.size == layout.size()
                            && element.align == layout.align(),
                        "external tuple field mismatch"
                    );
                    unsafe {
                        sequence
                            .read_element_at(element.offset, |source| child.validate_raw(source))?
                    };
                }
                Ok(())
            }
            ValueKind::Array(_) => {
                let array = unsafe { &*source.cast::<DynamicArray>() };
                let child = &self.children[0];
                child.validate_array_element(array.element_type())?;
                if !matches!(child.value_type.kind(), ValueKind::Leaf(_)) {
                    let (pointer, length, _) = array.allocation_parts();
                    let stride = child.external_type().1.size();
                    for index in 0..length {
                        unsafe { child.validate_raw(pointer.as_ptr().add(index * stride))? };
                    }
                }
                Ok(())
            }
            ValueKind::Callable { .. } => unreachable!(),
        }
    }

    /// Checks an existing array's complete recursive metadata against this element.
    ///
    /// # Errors
    /// Rejects mismatched native identities, layouts, or nested array descriptors.
    /// - Complexity: O(d) time and call space for array nesting depth d.
    fn validate_array_element(&self, element: &ArrayElementType) -> anyhow::Result<()> {
        let (id, layout) = self.external_type();
        anyhow::ensure!(
            element.type_id() == id
                && element.size() == layout.size()
                && element.align() == layout.align(),
            "external array element mismatch"
        );
        match self.value_type.kind() {
            ValueKind::Array(_) => self.children[0].validate_array_element(
                element
                    .nested()
                    .ok_or_else(|| anyhow::anyhow!("missing external nested array type"))?,
            ),
            ValueKind::Leaf(_) => {
                anyhow::ensure!(
                    element.nested().is_none(),
                    "unexpected external nested array type"
                );
                Ok(())
            }
            _ => unreachable!(),
        }
    }

    /// Validates an external box without moving or cloning its owner.
    ///
    /// # Errors
    /// Rejects missing input or any complete external representation mismatch.
    /// - Complexity: O(v * d) time and O(d) call space for v values and
    ///   external array metadata depth d (at least 1).
    fn validate_box(&self, input: &Option<Box<dyn Any>>) -> anyhow::Result<()> {
        let value = input
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing external owner"))?;
        anyhow::ensure!(
            (**value).type_id() == self.external_type().0,
            "external box type mismatch"
        );
        let source = &**value as *const dyn Any as *const u8;
        unsafe { self.validate_raw(source) }
    }

    /// Imports a box, retaining its entire original owner on validation failure.
    ///
    /// # Errors
    /// Rejects missing input or any complete external representation mismatch.
    /// No expression executes here, and no input is consumed on error.
    /// - Complexity: O(v * d + s) time and O(s) converted storage for v values,
    ///   external array metadata depth d (at least 1), and s representation bytes.
    pub(crate) fn import_box(
        &self,
        input: &mut Option<Box<dyn Any>>,
    ) -> anyhow::Result<OwnedValue> {
        self.validate_box(input)?;
        let boxed = input.take().unwrap();
        match self.value_type.kind() {
            ValueKind::Leaf(_) => {
                let allocation = Allocation::new(self.value_type.layout());
                // SAFETY: Any identity was checked against the concrete factory.
                unsafe {
                    (self.value_type.host_leaf().unwrap().write)(
                        boxed,
                        allocation.pointer.as_ptr(),
                    );
                    Ok(OwnedValue::from_allocation(
                        allocation,
                        self.value_type.clone(),
                    ))
                }
            }
            ValueKind::Tuple(_) => {
                Ok(self.import_sequence(*boxed.downcast::<DynamicSequence>().unwrap()))
            }
            ValueKind::Array(_) => {
                Ok(self.import_array(*boxed.downcast::<DynamicArray>().unwrap()))
            }
            ValueKind::Callable { .. } => unreachable!(),
        }
    }

    /// Validates all boxed tuple fields before transferring any owner.
    ///
    /// Validation failure leaves every slot and its original owner unchanged.
    /// After validation succeeds, each successful transfer empties its slot;
    /// guarded construction cleans completed fields if a later conversion fails.
    ///
    /// # Errors
    /// Rejects non-tuples, wrong arity, missing slots, or mismatching fields.
    /// - Complexity: O(v * d + s) time and O(s) converted storage for v values,
    ///   external array metadata depth d (at least 1), and s representation bytes.
    #[cfg(test)]
    pub(crate) fn import_elements(
        &self,
        inputs: &mut [Option<Box<dyn Any>>],
    ) -> anyhow::Result<OwnedValue> {
        let plan = ContainerPlan::tuple(self.value_type.clone())?;
        anyhow::ensure!(
            inputs.len() == self.children.len(),
            "external tuple arity mismatch"
        );
        for (child, input) in self.children.iter().zip(inputs.iter()) {
            child.validate_box(input)?;
        }
        // SAFETY: All owners are validated before moving; ContainerPlan owns only completed fields.
        unsafe {
            plan.construct(|index, destination| {
                let mut owner = self.children[index].import_box(&mut inputs[index])?;
                owner.move_into(destination);
                Ok(())
            })
        }
    }

    /// Moves validated external bytes into a canonical child, converting wrappers explicitly.
    ///
    /// # Safety
    /// Source must uniquely own a completely validated external value; the caller
    /// must suppress its old destruction after return. Destination must be aligned,
    /// disjoint canonical storage. No user callbacks or fallible checks execute.
    /// - Complexity: O(v * d + s) time and O(s) converted storage for v values,
    ///   external array metadata depth d (at least 1), and s representation bytes.
    unsafe fn import_raw(&self, source: *const u8, destination: *mut u8) {
        match self.value_type.kind() {
            ValueKind::Leaf(_) => unsafe {
                ptr::copy_nonoverlapping(source, destination, self.value_type.layout().size());
            },
            ValueKind::Tuple(_) => {
                let sequence = unsafe { source.cast::<DynamicSequence>().read() };
                let mut owner = self.import_sequence(sequence);
                unsafe { owner.move_into(destination) };
            }
            ValueKind::Array(_) => {
                let array = unsafe { source.cast::<DynamicArray>().read() };
                let mut owner = self.import_array(array);
                unsafe { owner.move_into(destination) };
            }
            ValueKind::Callable { .. } => unreachable!(),
        }
    }

    /// Converts a prevalidated sequence into a canonical source-indexed list.
    ///
    /// - Precondition: The complete sequence passes `validate_raw`.
    /// - Complexity: O(v + s) time and O(s) converted storage for v values and s bytes.
    fn import_sequence(&self, mut sequence: DynamicSequence) -> OwnedValue {
        debug_assert!(matches!(self.value_type.kind(), ValueKind::Tuple(_)));
        debug_assert_eq!(sequence.arity(), self.children.len());
        let plan = ContainerPlan::tuple(self.value_type.clone()).expect("prepared tuple kind");
        // SAFETY: All children were checked before transfer. Disarm each old owner
        // immediately after its infallible move, including nested wrapper conversion.
        unsafe {
            plan.construct(|index, destination| {
                let offset = sequence.shape()[index].offset;
                sequence.read_element_at(offset, |source| {
                    self.children[index].import_raw(source, destination)
                });
                sequence.disarm_element(index);
                Ok(())
            })
        }
        .expect("validated moves cannot fail")
    }

    /// Converts a prevalidated array, explicitly replacing every nested wrapper header.
    ///
    /// Leaf payload allocations retain their pointer and capacity. Nested wrapper
    /// payloads are rebuilt because their stride differs from canonical RawArray.
    ///
    /// - Precondition: The complete array passes `validate_raw`.
    /// - Complexity: O(v * d + s) time and O(s) converted storage, including
    ///   destruction of repeated legacy metadata; O(1) for leaf arrays.
    fn import_array(&self, array: DynamicArray) -> OwnedValue {
        debug_assert!(matches!(self.value_type.kind(), ValueKind::Array(_)));
        let child = &self.children[0];
        let (pointer, length, capacity, _) = array.into_allocation_parts();
        if matches!(child.value_type.kind(), ValueKind::Leaf(_)) {
            let allocation = Allocation::new(self.value_type.layout());
            unsafe {
                allocation
                    .pointer
                    .as_ptr()
                    .cast::<RawArray>()
                    .write(RawArray {
                        ptr: pointer.cast(),
                        len: length,
                        capacity,
                    });
                return OwnedValue::from_allocation(allocation, self.value_type.clone());
            }
        }
        let external_layout = child.external_type().1;
        let _storage = Allocation {
            pointer,
            layout: payload_layout(external_layout, capacity),
        };
        let plan = ContainerPlan::array(child.value_type.clone(), length)
            .expect("canonical array headers are smaller than legacy headers");
        unsafe {
            plan.construct(|index, destination| {
                child.import_raw(
                    pointer.as_ptr().add(index * external_layout.size()),
                    destination,
                );
                Ok(())
            })
        }
        .expect("validated moves cannot fail")
    }

    /// Prepares the existing tuple wrapper's capability-rich element metadata.
    ///
    /// # Errors
    /// Rejects leaves without any legacy tuple capability, and array fields whose
    /// existing wrapper supplies no equality/debug tuple adapters.
    fn sequence_element(&self) -> anyhow::Result<DynElementSpec> {
        match self.value_type.kind() {
            ValueKind::Leaf(id) => {
                let host = self.value_type.host_leaf().unwrap();
                Ok(DynElementSpec {
                    type_id: *id,
                    type_name: host.name.clone(),
                    size: self.value_type.layout().size(),
                    align: self.value_type.layout().align(),
                    drop: self.value_type.leaf_drop().unwrap(),
                    clone: host
                        .clone
                        .ok_or_else(|| anyhow::anyhow!("legacy tuple requires cloning"))?,
                    eq: host
                        .eq
                        .ok_or_else(|| anyhow::anyhow!("legacy tuple requires equality"))?,
                    debug: host
                        .debug
                        .ok_or_else(|| anyhow::anyhow!("legacy tuple requires formatting"))?,
                    write: host.write,
                })
            }
            ValueKind::Tuple(_) => Ok(DynElementSpec {
                type_id: TypeId::of::<DynamicSequence>(),
                type_name: Cow::Borrowed("DynamicSequence"),
                size: Layout::new::<DynamicSequence>().size(),
                align: Layout::new::<DynamicSequence>().align(),
                drop: crate::element_dropper_for::<DynamicSequence>(),
                clone: crate::element_cloner_for::<DynamicSequence>(),
                eq: crate::element_eq_for::<DynamicSequence>(),
                debug: crate::element_debug_for::<DynamicSequence>(),
                write: crate::element_writer_for::<DynamicSequence>(),
            }),
            _ => anyhow::bail!("existing tuple has no structural array/callable adapters"),
        }
    }

    /// Builds ownership-only legacy array metadata at the checked export boundary.
    ///
    /// - Precondition: This plan describes a supported external array element.
    /// - Complexity: O(d) time and storage for array nesting depth d.
    fn array_element(&self) -> ArrayElementType {
        match self.value_type.kind() {
            ValueKind::Leaf(id) => {
                let host = self.value_type.host_leaf().unwrap();
                // The legacy dropper has an associated-fields parameter.
                // Its concrete monomorphization is retained by the leaf factory.
                let mut element = ArrayElementType::leaf_from_parts(
                    *id,
                    host.name.clone(),
                    self.value_type.layout().size(),
                    self.value_type.layout().align(),
                    host.array_drop,
                );
                element.clone_value = host.clone;
                element.complete = Some(self.value_type.clone());
                element
            }
            ValueKind::Array(_) => ArrayElementType::array_of(self.children[0].array_element()),
            _ => unreachable!(),
        }
    }

    /// Validates external adapters and freezes recursive export structure before moves.
    ///
    /// # Errors
    /// Rejects unavailable legacy tuple adapters or unrepresentable external tuple layouts.
    /// - Complexity: O(n) time and metadata storage for n descriptor nodes.
    fn prepare_export(&self) -> anyhow::Result<ExportPlan> {
        match self.value_type.kind() {
            ValueKind::Leaf(_) => Ok(ExportPlan::Leaf),
            ValueKind::Tuple(_) => {
                let mut layout = Layout::from_size_align(0, 1)?;
                for child in &self.children {
                    layout = layout.extend(child.external_type().1)?.0;
                }

                let _ = layout.pad_to_align();
                Ok(ExportPlan::Tuple(
                    self.children
                        .iter()
                        .map(|child| {
                            child.sequence_element()?;
                            child.prepare_export()
                        })
                        .collect::<anyhow::Result<_>>()?,
                ))
            }
            ValueKind::Array(_) => Ok(ExportPlan::Array {
                element: self.children[0].array_element(),
            }),
            ValueKind::Callable { .. } => unreachable!(),
        }
    }

    /// Checks all statically knowable external export capabilities before execution.
    ///
    /// # Errors
    /// Rejects unavailable tuple adapters or unrepresentable external layouts.
    /// - Complexity: O(n) metadata preparation for n descriptor nodes.
    pub(crate) fn validate_export_capabilities(&self) -> anyhow::Result<()> {
        self.prepare_export().map(|_| ())
    }

    /// Exports a whole canonical owner, preserving it on every conversion error.
    ///
    /// # Errors
    /// Rejects missing owners, complete shape mismatch, unavailable external tuple
    /// capabilities, or an unrepresentable external nested-header allocation.
    /// All checks and metadata preparation precede the first ownership transfer.
    /// - Complexity: O(v * d + s) time and external storage for v values,
    ///   array metadata depth d (at least 1), and s representation bytes.
    pub(crate) fn export_box(
        &self,
        owner: &mut Option<OwnedValue>,
    ) -> anyhow::Result<Box<dyn Any>> {
        let value = owner
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing canonical owner"))?;
        anyhow::ensure!(
            value.value_type().same_shape(&self.value_type),
            "canonical host result mismatch"
        );
        let export = self.prepare_export()?;
        unsafe { self.validate_export(value.as_ptr())? };
        let allocation = Allocation::new(self.value_type.layout());
        let mut value = owner.take().unwrap();
        // SAFETY: Complete shape and external allocation bounds are validated.
        // Moving into staging disarms the original before recursively consuming it.
        unsafe {
            value.move_into(allocation.pointer.as_ptr());
            Ok(self.export_raw(allocation.pointer.as_ptr(), export))
        }
    }

    /// Checks data-dependent external payload bounds before any result transfer.
    ///
    /// # Safety
    /// Source must contain one live canonical value matching this plan.
    /// # Errors
    /// Rejects lengths whose larger legacy nested headers cannot fit Layout.
    /// - Complexity: O(v) time and O(d) call space for v values and depth d.
    unsafe fn validate_export(&self, source: *const u8) -> anyhow::Result<()> {
        match self.value_type.kind() {
            ValueKind::Tuple(fields) => {
                for (field, child) in fields.iter().zip(&self.children) {
                    unsafe { child.validate_export(source.add(field.offset))? };
                }
            }
            ValueKind::Array(_) => {
                let array = unsafe { &*source.cast::<RawArray>() };
                let child = &self.children[0];
                let size = child
                    .external_type()
                    .1
                    .size()
                    .checked_mul(array.len)
                    .ok_or_else(|| anyhow::anyhow!("external array payload overflow"))?;
                Layout::from_size_align(size, child.external_type().1.align())?;
                if !matches!(child.value_type.kind(), ValueKind::Leaf(_)) {
                    for index in 0..array.len {
                        unsafe {
                            child.validate_export(
                                array
                                    .ptr
                                    .as_ptr()
                                    .cast::<u8>()
                                    .add(index * child.value_type.layout().size()),
                            )?
                        };
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Consumes a checked canonical value into explicitly constructed external wrappers.
    ///
    /// # Safety
    /// Source uniquely owns a matching canonical value and passes `validate_export`.
    /// Export metadata must come from this exact plan. Source destruction is
    /// surrendered to this method, which runs no user code or recoverable checks.
    /// - Complexity: O(v * d + s) time and external storage for v values,
    ///   array metadata depth d (at least 1), and s representation bytes.
    unsafe fn export_raw(&self, source: *const u8, export: ExportPlan) -> Box<dyn Any> {
        match export {
            ExportPlan::Leaf => unsafe { (self.value_type.host_leaf().unwrap().read)(source) },
            ExportPlan::Tuple(exports) => {
                let ValueKind::Tuple(fields) = self.value_type.kind() else {
                    unreachable!()
                };
                let elements = exports
                    .into_iter()
                    .zip(fields)
                    .zip(&self.children)
                    .map(|((export, field), child)| {
                        let spec = child
                            .sequence_element()
                            .expect("immutable external adapters checked before transfer");
                        (spec, unsafe {
                            child.export_raw(source.add(field.offset), export)
                        })
                    })
                    .collect();
                Box::new(DynamicSequence::from_dyn_elements(elements))
            }
            ExportPlan::Array { element } => {
                let array = unsafe { source.cast::<RawArray>().read() };
                let child = &self.children[0];
                if matches!(child.value_type.kind(), ValueKind::Leaf(_)) {
                    return Box::new(
                        unsafe {
                            DynamicArray::try_from_raw_parts(
                                array.ptr.cast(),
                                array.len,
                                array.capacity,
                                element,
                            )
                        }
                        .expect("checked native payload"),
                    );
                }
                let _storage = Allocation {
                    pointer: array.ptr.cast(),
                    layout: payload_layout(child.value_type.layout(), array.capacity),
                };
                let nested = element
                    .nested()
                    .expect("prepared nested array type")
                    .clone();
                let mut builder =
                    crate::dynamic_array::DynamicArrayBuilder::with_capacity(element, array.len)
                        .expect("external header bounds checked before transfer");
                for index in 0..array.len {
                    let boxed = unsafe {
                        child.export_raw(
                            array
                                .ptr
                                .as_ptr()
                                .cast::<u8>()
                                .add(index * child.value_type.layout().size()),
                            ExportPlan::Array {
                                element: nested.clone(),
                            },
                        )
                    };
                    let value = *boxed.downcast::<DynamicArray>().unwrap();
                    let value = std::mem::ManuallyDrop::new(value);
                    unsafe { builder.push_bytes((&*value as *const DynamicArray).cast()) };
                }
                Box::new(builder.finish())
            }
        }
    }
}

/// Holds prevalidated external metadata independently of canonical value ownership.
enum ExportPlan {
    Leaf,
    Tuple(Vec<ExportPlan>),
    Array { element: ArrayElementType },
}

/// Returns the allocation layout of an already validated homogeneous payload.
///
/// - Precondition: `capacity` belongs to a valid allocation of `element` values.
fn payload_layout(element: Layout, capacity: usize) -> Layout {
    Layout::from_size_align(
        element.size().checked_mul(capacity).unwrap(),
        element.align(),
    )
    .expect("live payload layout")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owned_value::OwnedValue;
    use crate::value_type::{AssociatedType, Cons, Leaf, Nil, ValueKind, ValueType};
    use crate::{DynamicArray, DynamicSequence};
    use std::any::Any;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Describes source-ordered fields without trusting caller-supplied offsets.
    fn tuple(fields: Vec<ValueType>) -> ValueType {
        ValueType::tuple(
            fields
                .into_iter()
                .map(|value_type| AssociatedType {
                    offset: usize::MAX,
                    value_type,
                })
                .collect(),
        )
        .unwrap()
    }

    /// Builds the capabilities the existing legacy tuple interface requires.
    fn registered<T: Clone + PartialEq + std::fmt::Debug + 'static>() -> ValueType {
        ValueType::cloneable_leaf::<T>()
            .with_equality::<Leaf<T>>()
            .unwrap()
            .with_debug::<Leaf<T>>()
            .unwrap()
    }

    /// Round trips nested tuples through canonical lists without reordering fields.
    #[test]
    fn nested_tuple_roundtrip_uses_canonical_source_offsets() {
        let descriptor = tuple(vec![
            registered::<u8>(),
            tuple(vec![registered::<String>(), registered::<i64>()]),
            tuple(vec![]),
        ]);
        let plan = HostValuePlan::new(descriptor).unwrap();
        let text = String::from("identity");
        let pointer = text.as_ptr();
        let mut input: Option<Box<dyn Any>> = Some(Box::new(DynamicSequence::from_tuple((
            3u8,
            DynamicSequence::from_tuple((text, -7i64)),
            DynamicSequence::from_tuple(()),
        ))));
        let owner = plan.import_box(&mut input).unwrap();
        assert!(input.is_none());
        type Nested =
            Cons<Leaf<u8>, Cons<Cons<Leaf<String>, Cons<Leaf<i64>, Nil>>, Cons<Nil, Nil>>>;
        assert!(
            owner
                .value_type()
                .same_shape(&<Nested as crate::value_type::ValueRepr>::value_type())
        );
        let ValueKind::Tuple(fields) = owner.value_type().kind() else {
            panic!()
        };
        assert!(fields[0].offset > fields[1].offset);
        let mut owner = Some(owner);
        let output = plan.export_box(&mut owner).unwrap();
        assert!(owner.is_none());
        let (first, nested, empty): (u8, DynamicSequence, DynamicSequence) = output
            .downcast::<DynamicSequence>()
            .unwrap()
            .try_into_tuple()
            .unwrap();
        let (text, last): (String, i64) = nested.try_into_tuple().unwrap();
        assert_eq!(
            (first, text.as_str(), last, empty.arity()),
            (3, "identity", -7, 0)
        );
        assert_eq!(text.as_ptr(), pointer);
    }

    /// Transfers a foreign move-only leaf without requiring optional capabilities.
    #[test]
    fn foreign_move_only_leaf_preserves_identity_and_allocation() {
        let value = Box::new(std::sync::Mutex::new(String::from("foreign")));
        let pointer = &*value as *const _;
        let plan = HostValuePlan::new(ValueType::leaf::<Box<std::sync::Mutex<String>>>()).unwrap();
        let mut input: Option<Box<dyn Any>> = Some(Box::new(value));
        let owner = plan.import_box(&mut input).unwrap();
        assert!(owner.try_clone().is_err());
        let mut owner = Some(owner);
        let output = plan
            .export_box(&mut owner)
            .unwrap()
            .downcast::<Box<std::sync::Mutex<String>>>()
            .unwrap();
        assert_eq!(&**output as *const _, pointer);
        assert_eq!(*output.lock().unwrap(), "foreign");
    }

    /// Rejects nested same-layout mismatches before a dependent expression runs.
    #[test]
    fn mismatch_preserves_external_owner_and_prevents_execution() {
        let plan = HostValuePlan::new(tuple(vec![
            registered::<String>(),
            tuple(vec![registered::<u32>()]),
        ]))
        .unwrap();
        let mut input: Option<Box<dyn Any>> = Some(Box::new(DynamicSequence::from_tuple((
            String::from("retained"),
            DynamicSequence::from_tuple((1i32,)),
        ))));
        let executions = Rc::new(std::cell::Cell::new(0usize));
        let count = executions.clone();
        let mut expression = crate::DynSegment::new::<()>();
        expression.op0(move || {
            count.set(count.get() + 1);
            17u32
        });
        let result = plan
            .import_box(&mut input)
            .and_then(|_| expression.call_dyn::<u32>(&[]));
        assert!(result.is_err());
        assert_eq!(executions.get(), 0);
        let retained = input.unwrap().downcast::<DynamicSequence>().unwrap();
        let (text, nested): (String, DynamicSequence) = retained.try_into_tuple().unwrap();
        assert_eq!(text, "retained");
        assert_eq!(nested.try_into_tuple::<(i32,)>().unwrap(), (1,));
        let mut valid: Option<Box<dyn Any>> = Some(Box::new(DynamicSequence::from_tuple((
            String::from("accepted"),
            DynamicSequence::from_tuple((1u32,)),
        ))));
        assert_eq!(
            plan.import_box(&mut valid)
                .and_then(|_| expression.call_dyn::<u32>(&[]))
                .unwrap(),
            17
        );
        assert_eq!(executions.get(), 1);
    }

    /// Records destruction without providing cloning, equality, or formatting.
    struct Tracked(usize, Rc<RefCell<Vec<usize>>>);
    impl Drop for Tracked {
        /// Records one destruction of this uniquely owned leaf.
        fn drop(&mut self) {
            self.1.borrow_mut().push(self.0);
        }
    }

    /// Retains every original box when a later field has the wrong concrete identity.
    #[test]
    fn field_identity_mismatch_retains_all_original_owners() {
        let drops = Rc::new(RefCell::new(Vec::new()));
        let plan = HostValuePlan::new(tuple(vec![
            ValueType::leaf::<Tracked>(),
            ValueType::leaf::<u32>(),
            ValueType::leaf::<Tracked>(),
        ]))
        .unwrap();
        let mut inputs: Vec<Option<Box<dyn Any>>> = vec![
            Some(Box::new(Tracked(0, drops.clone()))),
            Some(Box::new(Tracked(1, drops.clone()))),
            Some(Box::new(Tracked(2, drops.clone()))),
        ];
        let pointers: Vec<_> = inputs
            .iter()
            .map(|input| &**input.as_ref().unwrap() as *const dyn Any as *const u8)
            .collect();
        assert!(plan.import_elements(&mut inputs).is_err());
        assert!(drops.borrow().is_empty());
        for (input, pointer) in inputs.iter().zip(pointers) {
            let retained = input.as_ref().unwrap().downcast_ref::<Tracked>().unwrap();
            assert_eq!(retained as *const Tracked as *const u8, pointer);
        }
        drop(inputs);
        assert_eq!(&*drops.borrow(), &[0, 1, 2]);
    }

    /// Retains an earlier native owner when a later required slot is absent.
    #[test]
    fn missing_later_field_retains_original_owner() {
        let drops = Rc::new(RefCell::new(Vec::new()));
        let plan = HostValuePlan::new(tuple(vec![
            ValueType::leaf::<Tracked>(),
            ValueType::leaf::<u32>(),
        ]))
        .unwrap();
        let original = Box::new(Tracked(0, drops.clone()));
        let pointer = &*original as *const Tracked;
        let mut inputs: Vec<Option<Box<dyn Any>>> = vec![Some(original), None];
        assert!(plan.import_elements(&mut inputs).is_err());
        assert!(drops.borrow().is_empty());
        let retained = inputs[0]
            .as_ref()
            .unwrap()
            .downcast_ref::<Tracked>()
            .unwrap();
        assert_eq!(retained as *const Tracked, pointer);
        assert!(inputs[1].is_none());
        drop(inputs);
        assert_eq!(&*drops.borrow(), &[0]);
    }

    /// Checks nested tuple and array identities before taking any native owner.
    #[test]
    fn nested_field_mismatch_retains_all_original_owners() {
        for (descriptor, original) in [
            (
                tuple(vec![registered::<u32>()]),
                Box::new(DynamicSequence::from_tuple((String::from("retained"),))) as Box<dyn Any>,
            ),
            (
                ValueType::array(ValueType::array(registered::<u32>())),
                Box::new(
                    DynamicArray::try_from_vec(vec![
                        DynamicArray::try_from_vec(vec![String::from("retained")]).unwrap(),
                    ])
                    .unwrap(),
                ) as Box<dyn Any>,
            ),
        ] {
            let drops = Rc::new(RefCell::new(Vec::new()));
            let plan = HostValuePlan::new(tuple(vec![
                ValueType::leaf::<Tracked>(),
                descriptor,
                ValueType::leaf::<Tracked>(),
            ]))
            .unwrap();
            let mut inputs: Vec<Option<Box<dyn Any>>> = vec![
                Some(Box::new(Tracked(0, drops.clone()))),
                Some(original),
                Some(Box::new(Tracked(2, drops.clone()))),
            ];
            let pointers: Vec<_> = inputs
                .iter()
                .map(|input| &**input.as_ref().unwrap() as *const dyn Any as *const u8)
                .collect();
            assert!(plan.import_elements(&mut inputs).is_err());
            assert!(drops.borrow().is_empty());
            for (input, pointer) in inputs.iter().zip(pointers) {
                assert_eq!(
                    &**input.as_ref().unwrap() as *const dyn Any as *const u8,
                    pointer
                );
            }
            let retained = inputs[1].take().unwrap();
            if retained.is::<DynamicSequence>() {
                let (text,): (String,) = retained
                    .downcast::<DynamicSequence>()
                    .unwrap()
                    .try_into_tuple()
                    .unwrap();
                assert_eq!(text, "retained");
            } else {
                let rows = retained
                    .downcast::<DynamicArray>()
                    .unwrap()
                    .try_into_vec::<DynamicArray>()
                    .unwrap();
                assert_eq!(rows[0].try_as_slice::<String>().unwrap(), &["retained"]);
            }
            drop(inputs);
            assert_eq!(&*drops.borrow(), &[0, 2]);
        }
    }

    /// Converts nested legacy array headers rather than copying wrapper bytes.
    #[test]
    fn nested_arrays_at_any_boundary_use_canonical_headers() {
        let descriptor = ValueType::array(ValueType::array(registered::<String>()));
        let plan = HostValuePlan::new(descriptor).unwrap();
        let text = String::from("array leaf");
        let pointer = text.as_ptr();
        let array =
            DynamicArray::try_from_vec(vec![DynamicArray::try_from_vec(vec![text]).unwrap()])
                .unwrap();
        let mut input: Option<Box<dyn Any>> = Some(Box::new(array));
        let owner = plan.import_box(&mut input).unwrap();
        type Shape = crate::value_type::ArrayOf<crate::value_type::ArrayOf<Leaf<String>>>;
        let array = owner.try_into_typed::<Shape>().unwrap();
        assert_eq!(array.as_slice()[0].as_slice()[0].as_ptr(), pointer);
        let mut owner = Some(OwnedValue::new::<Shape>(array));
        let array = *plan
            .export_box(&mut owner)
            .unwrap()
            .downcast::<DynamicArray>()
            .unwrap();
        let rows = array.try_into_vec::<DynamicArray>().unwrap();
        assert_eq!(
            rows[0].try_as_slice::<String>().unwrap()[0].as_ptr(),
            pointer
        );
    }

    /// Leaves a canonical owner intact when the external tuple lacks required capabilities.
    #[test]
    fn export_rejection_preserves_canonical_owner() {
        let drops = Rc::new(RefCell::new(Vec::new()));
        type Shape = Cons<Leaf<Tracked>, Nil>;
        let descriptor = tuple(vec![ValueType::leaf::<Tracked>()]);
        let plan = HostValuePlan::new(descriptor).unwrap();
        let mut owner = Some(OwnedValue::new::<Shape>(
            crate::IntoCStackList::into_c_stack_list((Tracked(9, drops.clone()),)),
        ));
        assert!(plan.export_box(&mut owner).is_err());
        assert!(drops.borrow().is_empty());
        assert!(owner.is_some());
        drop(owner);
        assert_eq!(&*drops.borrow(), &[9]);
    }

    /// Rejects legacy wrappers masquerading as canonical leaf storage.
    #[test]
    fn preparation_rejects_legacy_wrapper_leaves_and_unavailable_structural_boundaries() {
        for descriptor in [
            ValueType::leaf::<DynamicSequence>(),
            tuple(vec![ValueType::leaf::<DynamicArray>()]),
            ValueType::array(ValueType::leaf::<DynamicArray>()),
            ValueType::array(tuple(vec![ValueType::leaf::<i32>()])),
            ValueType::callable(vec![], ValueType::leaf::<i32>()),
        ] {
            assert!(HostValuePlan::new(descriptor).is_err());
        }
    }

    /// Rejects same-layout foreign identities without consuming native boxes.
    #[test]
    fn foreign_identity_is_not_inferred_from_equal_layout() {
        use std::num::{Saturating, Wrapping};
        let plan = HostValuePlan::new(ValueType::leaf::<Wrapping<u64>>()).unwrap();
        let mut input: Option<Box<dyn Any>> = Some(Box::new(Saturating(27u64)));
        assert!(plan.import_box(&mut input).is_err());
        assert_eq!(
            *input.unwrap().downcast::<Saturating<u64>>().unwrap(),
            Saturating(27)
        );
        let mut owner = Some(OwnedValue::new::<Leaf<Saturating<u64>>>(Saturating(28)));
        assert!(plan.export_box(&mut owner).is_err());
        assert_eq!(
            owner
                .unwrap()
                .try_into_typed::<Leaf<Saturating<u64>>>()
                .unwrap(),
            Saturating(28)
        );
    }

    /// Preserves custom registered cloning across nested canonical import and export.
    #[test]
    fn registered_foreign_leaf_capabilities_survive_nested_conversion() {
        type Foreign = std::ops::RangeInclusive<i32>;
        type Shape = Cons<Cons<Leaf<Foreign>, Nil>, Nil>;
        let plan = HostValuePlan::new(tuple(vec![tuple(vec![registered::<Foreign>()])])).unwrap();
        let mut input: Option<Box<dyn Any>> = Some(Box::new(DynamicSequence::from_tuple((
            DynamicSequence::from_tuple((4..=9,)),
        ))));
        let original = plan.import_box(&mut input).unwrap();
        let clone = original.try_clone().unwrap();
        assert!(original.try_eq(&clone).unwrap());
        assert_eq!(clone.try_debug().unwrap(), "((4..=9,),)");
        let (nested,): (crate::CStackList<Foreign, crate::CNil<()>>,) =
            crate::IntoRustTuple::into_rust_tuple(clone.try_into_typed::<Shape>().unwrap());
        assert_eq!(crate::IntoRustTuple::into_rust_tuple(nested), (4..=9,));
        let mut original = Some(original);
        let output = plan
            .export_box(&mut original)
            .unwrap()
            .downcast::<DynamicSequence>()
            .unwrap();
        let (nested,): (DynamicSequence,) = output.try_to_tuple().unwrap();
        assert_eq!(nested.try_to_tuple::<(Foreign,)>().unwrap(), (4..=9,));
    }

    /// Retains nested legacy owners when a later outer field fails validation.
    #[test]
    fn later_field_mismatch_retains_nested_legacy_owner() {
        let drops = Rc::new(RefCell::new(Vec::new()));
        let plan = HostValuePlan::new(tuple(vec![
            tuple(vec![
                ValueType::leaf::<Tracked>(),
                ValueType::leaf::<Tracked>(),
            ]),
            ValueType::leaf::<u32>(),
        ]))
        .unwrap();
        // Move-only fields cannot be exported into legacy tuples. Build the
        // existing tuple owner with its real ownership hook and unused adapters.
        let elements = [0, 1].map(|id| {
            (
                DynElementSpec {
                    type_id: std::any::TypeId::of::<Tracked>(),
                    type_name: Cow::Borrowed("Tracked"),
                    size: std::mem::size_of::<Tracked>(),
                    align: std::mem::align_of::<Tracked>(),
                    drop: crate::element_dropper_for::<Tracked>(),
                    clone: |_, _| panic!("cloning is not part of an ownership transfer"),
                    eq: |_, _| panic!("equality is not part of an ownership transfer"),
                    debug: |_, _| panic!("formatting is not part of an ownership transfer"),
                    write: crate::element_writer_for::<Tracked>(),
                },
                Box::new(Tracked(id, drops.clone())) as Box<dyn Any>,
            )
        });
        let nested = DynamicSequence::from_dyn_elements(elements.into());
        let mut inputs: Vec<Option<Box<dyn Any>>> =
            vec![Some(Box::new(nested)), Some(Box::new(-1i32))];
        let pointer = inputs[0]
            .as_ref()
            .unwrap()
            .downcast_ref::<DynamicSequence>()
            .unwrap() as *const DynamicSequence;
        assert!(plan.import_elements(&mut inputs).is_err());
        assert!(drops.borrow().is_empty());
        assert_eq!(
            inputs[0]
                .as_ref()
                .unwrap()
                .downcast_ref::<DynamicSequence>()
                .unwrap() as *const DynamicSequence,
            pointer
        );
        assert_eq!(
            *inputs[1].as_ref().unwrap().downcast_ref::<i32>().unwrap(),
            -1
        );
        inputs[1] = Some(Box::new(7u32));
        let owner = plan.import_elements(&mut inputs).unwrap();
        assert!(inputs.iter().all(Option::is_none));
        assert!(drops.borrow().is_empty());
        drop(inputs);
        assert!(drops.borrow().is_empty());
        drop(owner);
        assert_eq!(&*drops.borrow(), &[1, 0]);
    }

    /// Transfers nested arrays of move-only values without installing capabilities.
    #[test]
    fn nested_move_only_array_roundtrip_drops_exactly_once() {
        let drops = Rc::new(RefCell::new(Vec::new()));
        let plan = HostValuePlan::new(ValueType::array(ValueType::array(
            ValueType::leaf::<Tracked>(),
        )))
        .unwrap();
        let mut row = Vec::with_capacity(7);
        row.extend([Tracked(0, drops.clone()), Tracked(1, drops.clone())]);
        let pointer = row.as_ptr();
        let rows =
            DynamicArray::try_from_vec(vec![DynamicArray::try_from_vec(row).unwrap()]).unwrap();
        let mut input: Option<Box<dyn Any>> = Some(Box::new(rows));
        let owner = plan.import_box(&mut input).unwrap();
        assert!(owner.try_clone().is_err());
        let mut owner = Some(owner);
        let rows = *plan
            .export_box(&mut owner)
            .unwrap()
            .downcast::<DynamicArray>()
            .unwrap();
        let rows = rows.try_into_vec::<DynamicArray>().unwrap();
        let row = rows
            .into_iter()
            .next()
            .unwrap()
            .try_into_vec::<Tracked>()
            .unwrap();
        assert_eq!(row.as_ptr(), pointer);
        assert_eq!(row.capacity(), 7);
        assert!(drops.borrow().is_empty());
        drop(row);
        assert_eq!(&*drops.borrow(), &[0, 1]);
    }

    /// Keeps source unit distinct from opaque native unit and handles zero-sized arrays.
    #[test]
    fn source_unit_native_unit_and_zero_sized_arrays_roundtrip() {
        let unit = HostValuePlan::new(tuple(vec![])).unwrap();
        let mut native: Option<Box<dyn Any>> = Some(Box::new(()));
        assert!(unit.import_box(&mut native).is_err());
        assert!(native.is_some());
        let native_plan = HostValuePlan::new(ValueType::leaf::<()>()).unwrap();
        let mut native = Some(native_plan.import_box(&mut native).unwrap());
        assert!(native_plan.export_box(&mut native).unwrap().is::<()>());
        let mut empty: Option<Box<dyn Any>> = Some(Box::new(DynamicSequence::from_tuple(())));
        let mut canonical = Some(unit.import_box(&mut empty).unwrap());
        assert_eq!(
            unit.export_box(&mut canonical)
                .unwrap()
                .downcast::<DynamicSequence>()
                .unwrap()
                .arity(),
            0
        );
        for length in [0, 3] {
            let array_plan = HostValuePlan::new(ValueType::array(ValueType::leaf::<()>())).unwrap();
            let mut array: Option<Box<dyn Any>> = Some(Box::new(
                DynamicArray::try_from_vec(vec![(); length]).unwrap(),
            ));
            let mut canonical = Some(array_plan.import_box(&mut array).unwrap());
            let array = *array_plan
                .export_box(&mut canonical)
                .unwrap()
                .downcast::<DynamicArray>()
                .unwrap();
            assert_eq!(array.len(), length);
            assert_eq!(array.capacity(), usize::MAX);
            assert_eq!(array.try_into_vec::<()>().unwrap().len(), length);
        }
    }

    /// Preserves over-aligned native values across tail-first tuple permutations.
    #[test]
    fn over_aligned_nested_tuple_roundtrip() {
        /// Supplies a foreign alignment greater than the wrapper headers.
        #[derive(Clone, Debug, PartialEq)]
        #[repr(align(128))]
        struct Aligned(u8);
        let plan = HostValuePlan::new(tuple(vec![
            registered::<u8>(),
            tuple(vec![registered::<Aligned>(), registered::<u16>()]),
        ]))
        .unwrap();
        let mut input: Option<Box<dyn Any>> = Some(Box::new(DynamicSequence::from_tuple((
            5u8,
            DynamicSequence::from_tuple((Aligned(8), 65535u16)),
        ))));
        let mut owner = Some(plan.import_box(&mut input).unwrap());
        let output = plan
            .export_box(&mut owner)
            .unwrap()
            .downcast::<DynamicSequence>()
            .unwrap();
        let (first, nested): (u8, DynamicSequence) = output.try_into_tuple().unwrap();
        let (aligned, last): (Aligned, u16) = nested.try_into_tuple().unwrap();
        assert_eq!((first, aligned.0, last), (5, 8, 65535));
    }

    /// Rejects missing owners and wrong field counts without consuming available values.
    #[test]
    fn absent_inputs_and_wrong_arity_are_checked() {
        let plan = HostValuePlan::new(tuple(vec![registered::<u32>()])).unwrap();
        assert!(plan.import_box(&mut None).is_err());
        assert!(plan.export_box(&mut None).is_err());
        assert!(plan.import_elements(&mut []).is_err());
        assert!(plan.import_elements(&mut [None]).is_err());
        let mut input: Option<Box<dyn Any>> = Some(Box::new(DynamicSequence::from_tuple(())));
        assert!(plan.import_box(&mut input).is_err());
        assert!(input.is_some());
        let leaf = HostValuePlan::new(ValueType::leaf::<u32>()).unwrap();
        let mut inputs = [Some(Box::new(8u32) as Box<dyn Any>)];
        assert!(leaf.import_elements(&mut inputs).is_err());
        assert_eq!(*inputs[0].take().unwrap().downcast::<u32>().unwrap(), 8);
    }

    /// Clones borrowed nested foreign values through their registered frozen plans.
    #[test]
    fn borrowed_tuple_import_preserves_native_owners_and_clone_capabilities() {
        let plan = HostValuePlan::new(tuple(vec![
            registered::<String>(),
            tuple(vec![registered::<std::ops::RangeInclusive<i32>>()]),
        ]))
        .unwrap();
        let input = DynamicSequence::from_tuple((
            String::from("borrowed"),
            DynamicSequence::from_tuple((2..=6,)),
        ));
        let mut cloned = Some(plan.clone_any(&input).unwrap());
        let output = plan
            .export_box(&mut cloned)
            .unwrap()
            .downcast::<DynamicSequence>()
            .unwrap();
        assert_eq!(*output, input);
        let (original, _): (String, DynamicSequence) = input.try_into_tuple().unwrap();
        let (cloned, _): (String, DynamicSequence) = output.try_into_tuple().unwrap();
        assert_eq!(cloned, original);
        assert_ne!(cloned.as_ptr(), original.as_ptr());
    }

    /// Rejects borrowed cloning of ownership-only leaves before touching their value.
    #[test]
    fn borrowed_move_only_import_rejects_without_consuming() {
        let drops = Rc::new(RefCell::new(Vec::new()));
        let plan = HostValuePlan::new(ValueType::leaf::<Tracked>()).unwrap();
        let input = Tracked(3, drops.clone());
        assert!(plan.clone_any(&input).is_err());
        assert!(drops.borrow().is_empty());
        drop(input);
        assert_eq!(&*drops.borrow(), &[3]);
    }

    /// Cleans completed nested borrowed clones on panic without touching original owners.
    #[test]
    fn borrowed_clone_panic_cleans_nested_prefix_and_preserves_original() {
        /// Distinguishes clone cleanup from later original-value destruction.
        #[derive(Debug, PartialEq)]
        struct PanicClone {
            id: usize,
            cloned: bool,
            drops: Rc<RefCell<Vec<(bool, usize)>>>,
        }
        impl Clone for PanicClone {
            /// Clones values 0 and 1 and deliberately panics for value 2.
            fn clone(&self) -> Self {
                assert_ne!(self.id, 2, "deliberate leaf clone panic");
                Self {
                    id: self.id,
                    cloned: true,
                    drops: self.drops.clone(),
                }
            }
        }
        impl Drop for PanicClone {
            /// Records whether this is clone cleanup or original destruction.
            fn drop(&mut self) {
                self.drops.borrow_mut().push((self.cloned, self.id));
            }
        }
        let drops = Rc::new(RefCell::new(Vec::new()));
        let value = |id| PanicClone {
            id,
            cloned: false,
            drops: drops.clone(),
        };
        let input = DynamicSequence::from_tuple((
            DynamicSequence::from_tuple((value(0), value(1))),
            value(2),
        ));
        let wrong_plan = HostValuePlan::new(tuple(vec![
            tuple(vec![registered::<PanicClone>(), registered::<PanicClone>()]),
            registered::<u32>(),
        ]))
        .unwrap();
        assert!(wrong_plan.clone_any(&input).is_err());
        assert!(drops.borrow().is_empty());
        let plan = HostValuePlan::new(tuple(vec![
            tuple(vec![registered::<PanicClone>(), registered::<PanicClone>()]),
            registered::<PanicClone>(),
        ]))
        .unwrap();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| plan.clone_any(&input)))
                .is_err()
        );
        assert_eq!(&*drops.borrow(), &[(true, 1), (true, 0)]);
        let (nested, last): (DynamicSequence, PanicClone) = input.try_into_tuple().unwrap();
        let (first, second): (PanicClone, PanicClone) = nested.try_into_tuple().unwrap();
        assert_eq!((first.id, second.id, last.id), (0, 1, 2));
        drop((last, second, first));
        assert_eq!(
            &*drops.borrow(),
            &[(true, 1), (true, 0), (false, 2), (false, 1), (false, 0)]
        );
    }

    /// Clones nested borrowed arrays using registry capabilities without copying legacy headers.
    #[test]
    fn borrowed_nested_array_import_keeps_original_payloads() {
        let plan =
            HostValuePlan::new(ValueType::array(ValueType::array(registered::<String>()))).unwrap();
        let input = DynamicArray::try_from_vec(vec![
            DynamicArray::try_from_vec(vec![String::from("original")]).unwrap(),
        ])
        .unwrap();
        let original = input.try_as_slice::<DynamicArray>().unwrap()[0]
            .try_as_slice::<String>()
            .unwrap()[0]
            .as_ptr();
        let owner = plan.clone_any(&input).unwrap();
        type Shape = crate::value_type::ArrayOf<crate::value_type::ArrayOf<Leaf<String>>>;
        let cloned = owner.try_into_typed::<Shape>().unwrap();
        assert_eq!(cloned.as_slice()[0].as_slice()[0], "original");
        assert_ne!(cloned.as_slice()[0].as_slice()[0].as_ptr(), original);
        assert_eq!(
            input.try_as_slice::<DynamicArray>().unwrap()[0]
                .try_as_slice::<String>()
                .unwrap()[0]
                .as_ptr(),
            original
        );
    }
}
