//! Builds checked instructions for the canonical raw interpreter.
//!
//! Ordinary operations name opaque Rust leaves. Structural operations name a
//! sealed representation witness; `prepare` checks the whole borrowed frame once.
//! ```
//! use cel_runtime::{DynSegment, Nil, Leaf, IntoCStackList};
//! let mut body = DynSegment::new::<()>();
//! body.just(42i32);
//! let mut body = body.prepare::<Nil, Leaf<i32>>().unwrap();
//! assert_eq!(body.call(&().into_c_stack_list()).unwrap(), 42);
//! ```

use std::any::{Any, TypeId};
use std::cell::Cell;
use std::mem::MaybeUninit;

use anyhow::{Result, anyhow, ensure};

use crate::canonical_operation::OperationPlan;
use crate::container_execution::{ContainerPlan, ResultPlan, StackFramePlan, stack_position};
use crate::host_value_bridge::HostValuePlan;
use crate::owned_value::{OwnedValue, clone_raw, drop_raw};
use crate::{
    AssociatedType, CNil, CStackList, CStackListHeadLimit, CStackListHeadPadded, IntoCStackList,
    Leaf, List, RawSegment, RawStack, ReverseList, ValueKind, ValueRepr, ValueType,
};

thread_local! {
    /// Borrows the innermost call's already validated positional argument pointers.
    static ARGUMENTS: Cell<*const *const u8> = const { Cell::new(std::ptr::null()) };
}

/// Restores an enclosing raw argument context on return, error, or unwind.
struct ArgumentGuard(*const *const u8);

impl ArgumentGuard {
    /// Installs pointers that remain live until this guard is destroyed.
    fn install(arguments: &[*const u8]) -> Self {
        Self(ARGUMENTS.with(|slot| slot.replace(arguments.as_ptr())))
    }
}

impl Drop for ArgumentGuard {
    /// Restores the preceding call's borrowed argument frame.
    fn drop(&mut self) {
        ARGUMENTS.with(|slot| slot.set(self.0));
    }
}

/// Destroys an external wrapper leaf in place.
///
/// # Safety
/// The pointer must uniquely own the concrete type associated with this hook.
pub type RawDropper = unsafe fn(*mut u8, &[AssociatedType]);

/// Clones a concrete external leaf into an `Any` box.
///
/// # Safety
/// The pointer must borrow the concrete type associated with this hook. The
/// hook must clone rather than move; the caller retains ownership of the source.
pub type BoxExtractor = unsafe fn(*const u8) -> Box<dyn Any>;

/// Supplies concrete capability hooks at an explicitly checked external tuple boundary.
type TupleHooks = (
    crate::ElementDropper,
    crate::ElementCloner,
    crate::ElementEq,
    crate::ElementDebug,
);

/// Selects external extraction of one source-ordered tuple field.
pub enum DynExtractor {
    /// Clones an opaque concrete leaf through the caller's trusted extractor.
    Scalar(TypeId, BoxExtractor),
    /// Moves a canonical nested tuple into an external `DynamicSequence`.
    Tuple(Box<dyn Fn(TypeId) -> Option<TupleHooks>>),
}

/// Prepares external tuple metadata without changing interpreter storage.
///
/// # Safety
/// Every returned hook must describe the concrete leaf named by the lookup key.
/// # Errors
/// Rejects absent hooks or unsupported array/callable external tuple fields.
/// - Complexity: O(n) descriptor preparation for n tuple/leaf nodes.
unsafe fn external_tuple_type(
    value: &ValueType,
    lookup: &(impl Fn(TypeId) -> Option<TupleHooks> + ?Sized),
) -> Result<ValueType> {
    match value.kind() {
        ValueKind::Leaf(id) => {
            let (_, clone, eq, debug) =
                lookup(*id).ok_or_else(|| anyhow!("external tuple leaf is unregistered"))?;
            Ok(unsafe { value.clone().with_external_hooks(clone, eq, debug) })
        }
        ValueKind::Tuple(fields) => ValueType::tuple(
            fields
                .iter()
                .map(|field| {
                    Ok(AssociatedType {
                        offset: 0,
                        value_type: unsafe { external_tuple_type(&field.value_type, lookup)? },
                    })
                })
                .collect::<Result<_>>()?,
        ),
        _ => anyhow::bail!("external tuples do not support structural array or callable fields"),
    }
}

/// Returns an external leaf destructor while its concrete type is known.
///
/// # Examples
/// ```
/// let _ = cel_runtime::raw_dropper_for::<String>();
/// ```
pub fn raw_dropper_for<T: 'static>() -> RawDropper {
    |pointer, _| unsafe { pointer.cast::<T>().drop_in_place() }
}

/// Marks a canonical tuple for diagnostic-only legacy `TypeId` dispatch.
///
/// This marker never authorizes a raw cast; structural boundaries need witnesses.
#[derive(Debug)]
pub struct DynTuple;

/// Describes a live canonical stack slot.
#[derive(Clone, Debug)]
pub struct StackInfo {
    /// Whether the slot follows alignment padding.
    pub padding: bool,
    /// Complete recursive identity, layout, and ownership metadata.
    pub value_type: ValueType,
}

/// Formats recursive source-order type names for construction errors.
///
/// - Complexity: O(d * s) for depth d and rendered name length s.
fn descriptor_name(ty: &ValueType) -> String {
    match ty.kind() {
        ValueKind::Array(element) => format!("[{}]", descriptor_name(element)),
        ValueKind::Tuple(fields) => format!(
            "({})",
            fields
                .iter()
                .map(|field| descriptor_name(&field.value_type))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => ty.type_name().to_owned(),
    }
}

/// Enumerates opaque leaf types in a concrete preloaded stack list.
pub trait ToTypeIdList: List {
    /// Returns source-ordered leaf slots.
    ///
    /// - Complexity: O(n) slots for n list elements.
    fn to_stack_info_list() -> Vec<StackInfo>;
}

impl ToTypeIdList for CNil<()> {
    /// Returns an empty preloaded stack.
    fn to_stack_info_list() -> Vec<StackInfo> {
        Vec::new()
    }
}

impl<H: 'static, T: ToTypeIdList + 'static + CStackListHeadLimit> ToTypeIdList
    for CStackList<H, T>
{
    /// Enumerates tail storage followed by the opaque head.
    ///
    /// - Complexity: O(n) slots for n list elements.
    fn to_stack_info_list() -> Vec<StackInfo> {
        let mut result = T::to_stack_info_list();
        result.push(StackInfo {
            padding: Self::HEAD_PADDED,
            value_type: ValueType::leaf::<H>(),
        });
        result
    }
}

/// Owns validated raw instructions and complete canonical stack metadata.
///
/// `new::<()>()` starts an independent body with borrowed positional reads.
/// Other `new` argument lists retain the direct, preloaded opaque-leaf ABI.
pub struct DynSegment {
    pub(crate) segment: RawSegment,
    pub(crate) argument_ids: Vec<TypeId>,
    pub(crate) stack_ids: Vec<StackInfo>,
    pub(crate) base_stack_index: usize,
    pub(crate) argument_reads: Vec<(usize, ValueType)>,
}

/// Binds live raw slots to reverse-order destruction and exact truncation points.
///
/// The opaque prefix preceding `origin` belongs to an enclosing interpreter.
struct CleanupPlan {
    origin: usize,
    slots: Vec<(usize, usize, ValueType)>,
}

impl CleanupPlan {
    /// Computes immutable destruction positions before registering an instruction.
    ///
    /// # Errors
    /// Rejects unrepresentable frame positions.
    /// - Complexity: O(n) time and space for n slots.
    fn new(origin: usize, values: &[ValueType]) -> Result<Self> {
        let mut end = origin;
        let slots = values
            .iter()
            .map(|value| {
                let before = end;
                let (offset, next) = stack_position(before, value.layout())?;
                end = next;
                Ok((before, offset, value.clone()))
            })
            .collect::<Result<_>>()?;
        Ok(Self { origin, slots })
    }

    /// Arms cleanup before any operation that can unwind or return an error.
    fn guard<'a>(&'a self, stack: &'a mut RawStack) -> CleanupGuard<'a> {
        CleanupGuard {
            stack,
            plan: self,
            remaining: self.slots.len(),
            armed: true,
        }
    }
}

/// Destroys the remaining initialized suffix if a callback fails or unwinds.
struct CleanupGuard<'a> {
    stack: &'a mut RawStack,
    plan: &'a CleanupPlan,
    remaining: usize,
    armed: bool,
}

impl CleanupGuard<'_> {
    /// Destroys slots in reverse order, advancing before each destructor.
    ///
    /// - Complexity: O(n) destruction calls for n remaining slots.
    fn destroy(&mut self) {
        while self.remaining != 0 {
            self.remaining -= 1;
            let (before, offset, ty) = &self.plan.slots[self.remaining];
            // Remove ownership before user destruction, so unwinding resumes
            // with the preceding slot rather than destroying this slot twice.
            unsafe {
                self.stack.truncate_to(*before, false);
                self.stack.drop_at(*offset, |pointer| drop_raw(ty, pointer));
            }
        }
        debug_assert_eq!(self.stack.len(), self.plan.origin);
        self.armed = false;
    }
}

impl Drop for CleanupGuard<'_> {
    /// Continues reverse cleanup during errors and host unwinding.
    fn drop(&mut self) {
        if self.armed {
            self.destroy();
        }
    }
}

impl DynSegment {
    /// Creates a segment with an optional preloaded opaque-leaf stack.
    ///
    /// # Panics
    /// Rejects preloaded legacy wrapper carriers; import those at a checked external boundary.
    ///
    /// # Examples
    /// ```
    /// let _ = cel_runtime::DynSegment::new::<()>();
    /// ```
    pub fn new<Args: IntoCStackList>() -> Self
    where
        ReverseList<Args::Output>: ToTypeIdList,
    {
        let stack_ids = ReverseList::<Args::Output>::to_stack_info_list();
        let mut segment = RawSegment::new();
        segment.update_base_alignment(1);
        for info in &stack_ids {
            info.value_type
                .validate_canonical()
                .expect("preloaded dynamic arguments must be canonical or ordinary opaque leaves");
            segment.update_base_alignment(info.value_type.align());
        }
        Self {
            segment,
            argument_ids: stack_ids
                .iter()
                .map(|info| info.value_type.type_id())
                .collect(),
            stack_ids,
            base_stack_index: 0,
            argument_reads: Vec::new(),
        }
    }

    /// Creates a conditional fragment above the ambient stack below the condition.
    ///
    /// - Precondition: at least one operand is present.
    /// - Complexity: O(n) layout work for n ambient slots.
    ///
    /// # Examples
    /// See [`Self::join2`].
    pub fn new_fragment(&self) -> Self {
        debug_assert!(!self.stack_ids.is_empty());
        let mut result = Self::new::<()>();
        result.base_stack_index = self.stack_offset_before_top(1).expect("condition present");
        result
    }

    /// Returns the current canonical stack byte length.
    ///
    /// - Complexity: O(n) layout work for n slots.
    ///
    /// # Examples
    /// ```
    /// assert_eq!(cel_runtime::DynSegment::new::<()>().current_stack_offset(), 0);
    /// ```
    pub fn current_stack_offset(&self) -> usize {
        self.offset_after(self.stack_ids.len())
    }

    /// Returns the byte length preceding the top `n` operands.
    ///
    /// # Errors
    /// Rejects insufficient operands.
    /// - Complexity: O(k) layout work for k preceding slots.
    ///
    /// # Examples
    /// ```
    /// assert!(cel_runtime::DynSegment::new::<()>().stack_offset_before_top(1).is_err());
    /// ```
    pub fn stack_offset_before_top(&self, n: usize) -> Result<usize> {
        let count = self
            .stack_ids
            .len()
            .checked_sub(n)
            .ok_or_else(|| anyhow!("not enough operands"))?;
        Ok(self.offset_after(count))
    }

    /// Computes the canonical position following a known slot prefix.
    ///
    /// - Complexity: O(count) layout work.
    fn offset_after(&self, count: usize) -> usize {
        self.stack_ids[..count]
            .iter()
            .fold(self.base_stack_index, |end, info| {
                stack_position(end, info.value_type.layout())
                    .expect("validated frame position")
                    .1
            })
    }

    /// Copies the complete metadata of a known stack prefix.
    ///
    /// - Complexity: O(count) descriptor references.
    fn types(&self, count: usize) -> Vec<ValueType> {
        self.stack_ids[..count]
            .iter()
            .map(|info| info.value_type.clone())
            .collect()
    }

    /// Binds ambient cleanup and source-ordered operands without mutating metadata.
    ///
    /// # Errors
    /// Rejects insufficient operands or an unrepresentable frame.
    /// - Complexity: O(n) time and space for n stack slots.
    fn operation(&self, arity: usize, result: ValueType) -> Result<OperationPlan> {
        let first = self
            .stack_ids
            .len()
            .checked_sub(arity)
            .ok_or_else(|| anyhow!("not enough operands"))?;
        let types = self.types(self.stack_ids.len());
        OperationPlan::new(
            self.base_stack_index,
            &types[..first],
            &types[first..],
            result,
        )
    }

    /// Replaces emitted operands with their validated canonical result slot.
    ///
    /// - Complexity: O(n) layout work for n surviving slots.
    fn replace(&mut self, arity: usize, result: ValueType) {
        self.stack_ids.truncate(self.stack_ids.len() - arity);
        let before = self.current_stack_offset();
        self.segment.update_base_alignment(result.align());
        self.stack_ids.push(StackInfo {
            padding: stack_position(before, result.layout())
                .expect("validated result")
                .0
                != before,
            value_type: result,
        });
    }

    /// Borrows up to the top `n` complete descriptors.
    ///
    /// # Examples
    /// ```
    /// let body = cel_runtime::DynSegment::new::<()>();
    /// assert!(body.peek_stack_infos(0).is_empty());
    /// ```
    pub fn peek_stack_infos(&self, n: usize) -> &[StackInfo] {
        &self.stack_ids[self.stack_ids.len().saturating_sub(n)..]
    }

    /// Returns a diagnostic dispatch identity, never a structural cast authority.
    ///
    /// # Examples
    /// ```
    /// assert!(cel_runtime::DynSegment::new::<()>().peek_output_type_id().is_none());
    /// ```
    pub fn peek_output_type_id(&self) -> Option<TypeId> {
        Some(self.stack_ids.last()?.value_type.type_id())
    }

    /// Returns the source tuple arity, including zero for canonical unit.
    ///
    /// # Examples
    /// ```
    /// assert!(cel_runtime::DynSegment::new::<()>().peek_tuple_arity().is_none());
    /// ```
    pub fn peek_tuple_arity(&self) -> Option<usize> {
        Some(self.stack_ids.last()?.value_type.tuple_elements()?.len())
    }

    /// Registers a nullary opaque-leaf host operation.
    ///
    /// # Panics
    /// Rejects legacy wrapper results or unrepresentable frames. Use `op0_repr`
    /// for fallible registration and `just_external` for checked wrapper imports.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.op0(|| 7i32);
    /// assert_eq!(b.call0::<i32>().unwrap(), 7);
    /// ```
    pub fn op0<R: 'static, F: Fn() -> R + 'static>(&mut self, f: F) {
        self.op0_repr::<Leaf<R>, _>(f)
            .expect("opaque result fits raw frame");
    }

    /// Registers a fallible nullary opaque-leaf operation with ambient cleanup.
    ///
    /// # Panics
    /// Rejects legacy wrapper results or unrepresentable frames during registration.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.op0r(|| Ok(7i32));
    /// ```
    pub fn op0r<R: 'static, F: Fn() -> Result<R> + 'static>(&mut self, f: F) {
        self.op0r_repr::<Leaf<R>, _>(f)
            .expect("opaque result fits raw frame");
    }

    /// Registers a witnessed nullary operation.
    ///
    /// # Errors
    /// Rejects unrepresentable stack positions.
    /// - Complexity: O(n) registration work for n ambient slots.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.op0_repr::<cel_runtime::Nil, _>(|| cel_runtime::IntoCStackList::into_c_stack_list(())).unwrap();
    /// ```
    pub fn op0_repr<R: ValueRepr, F: Fn() -> R::Value + 'static>(&mut self, f: F) -> Result<()> {
        self.op0r_repr::<R, _>(move || Ok(f()))
    }

    /// Registers a fallible witnessed nullary operation.
    ///
    /// # Errors
    /// Rejects unrepresentable stack positions; host errors propagate on execution.
    /// - Complexity: O(n) registration work for n ambient slots.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.op0r_repr::<cel_runtime::Leaf<i32>, _>(|| Ok(7)).unwrap();
    /// ```
    pub fn op0r_repr<R: ValueRepr, F: Fn() -> Result<R::Value> + 'static>(
        &mut self,
        f: F,
    ) -> Result<()> {
        let result = R::value_type();
        self.operation(0, result.clone())?
            .emit0::<R, _>(&mut self.segment, f)?;
        self.replace(0, result);
        Ok(())
    }

    /// Registers a unary opaque-leaf operation.
    ///
    /// # Errors
    /// Rejects insufficient or nonmatching opaque operands.
    /// - Complexity: O(n) registration work for n stack slots.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.just(1i32); b.op1(|x: i32| x.to_string()).unwrap();
    /// ```
    pub fn op1<A: 'static, R: 'static, F: Fn(A) -> R + 'static>(&mut self, f: F) -> Result<()> {
        self.op1_repr::<Leaf<A>, Leaf<R>, _>(f)
    }

    /// Registers a fallible unary opaque-leaf operation.
    ///
    /// # Errors
    /// Rejects insufficient or nonmatching operands; execution propagates host errors.
    /// - Complexity: O(n) registration work for n stack slots.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.just(1i32); b.op1r(|x: i32| Ok(x)).unwrap();
    /// ```
    pub fn op1r<A: 'static, R: 'static, F: Fn(A) -> Result<R> + 'static>(
        &mut self,
        f: F,
    ) -> Result<()> {
        self.op1r_repr::<Leaf<A>, Leaf<R>, _>(f)
    }

    /// Registers a unary operation after checking its complete operand witness.
    ///
    /// # Errors
    /// Rejects insufficient operands or recursive representation mismatch.
    /// - Complexity: O(n) registration work for n frame/descriptor nodes.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.just(1i32);
    /// b.op1_repr::<cel_runtime::Leaf<i32>, cel_runtime::Leaf<i32>, _>(|x| x).unwrap();
    /// ```
    pub fn op1_repr<A: ValueRepr, R: ValueRepr, F: Fn(A::Value) -> R::Value + 'static>(
        &mut self,
        f: F,
    ) -> Result<()> {
        self.op1r_repr::<A, R, _>(move |a| Ok(f(a)))
    }

    /// Registers a fallible unary witnessed operation with error/unwind cleanup.
    ///
    /// # Errors
    /// Rejects insufficient operands or recursive mismatch; propagates host errors on execution.
    /// - Complexity: O(n) registration work for n frame/descriptor nodes.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.just(1i32);
    /// b.op1r_repr::<cel_runtime::Leaf<i32>, cel_runtime::Leaf<i32>, _>(Ok).unwrap();
    /// ```
    pub fn op1r_repr<A: ValueRepr, R: ValueRepr, F: Fn(A::Value) -> Result<R::Value> + 'static>(
        &mut self,
        f: F,
    ) -> Result<()> {
        let result = R::value_type();
        self.operation(1, result.clone())?
            .emit1::<A, R, _>(&mut self.segment, f)?;
        self.replace(1, result);
        Ok(())
    }

    /// Registers a binary opaque-leaf operation in source order.
    ///
    /// # Errors
    /// Rejects insufficient or mismatched operands.
    /// - Complexity: O(n) registration work for n frame/descriptor nodes.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.just(1i32); b.just(2i32); b.op2(|a: i32, b: i32| a + b).unwrap();
    /// ```
    pub fn op2<A: 'static, B: 'static, R: 'static, F: Fn(A, B) -> R + 'static>(
        &mut self,
        f: F,
    ) -> Result<()> {
        self.op2_repr::<Leaf<A>, Leaf<B>, Leaf<R>, _>(f)
    }

    /// Registers a fallible binary opaque-leaf operation.
    ///
    /// # Errors
    /// Rejects insufficient/mismatched operands; propagates host errors on execution.
    /// - Complexity: O(n) registration work for n frame/descriptor nodes.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.just(1i32); b.just(2i32); b.op2r(|a: i32, b: i32| Ok(a + b)).unwrap();
    /// ```
    pub fn op2r<A: 'static, B: 'static, R: 'static, F: Fn(A, B) -> Result<R> + 'static>(
        &mut self,
        f: F,
    ) -> Result<()> {
        self.op2r_repr::<Leaf<A>, Leaf<B>, Leaf<R>, _>(f)
    }

    /// Registers a binary witnessed operation in source order.
    ///
    /// # Errors
    /// Rejects insufficient operands or complete witness mismatch.
    /// - Complexity: O(n) registration work for n frame/descriptor nodes.
    ///
    /// # Examples
    /// See [`Self::op2`]; use one witness per operand and result.
    pub fn op2_repr<
        A: ValueRepr,
        B: ValueRepr,
        R: ValueRepr,
        F: Fn(A::Value, B::Value) -> R::Value + 'static,
    >(
        &mut self,
        f: F,
    ) -> Result<()> {
        self.op2r_repr::<A, B, R, _>(move |a, b| Ok(f(a, b)))
    }

    /// Registers a fallible binary witnessed operation.
    ///
    /// # Errors
    /// Rejects insufficient operands or complete mismatch; execution propagates host errors.
    /// - Complexity: O(n) registration work for n frame/descriptor nodes.
    ///
    /// # Examples
    /// See [`Self::op2r`]; use one witness per operand and result.
    pub fn op2r_repr<
        A: ValueRepr,
        B: ValueRepr,
        R: ValueRepr,
        F: Fn(A::Value, B::Value) -> Result<R::Value> + 'static,
    >(
        &mut self,
        f: F,
    ) -> Result<()> {
        let result = R::value_type();
        self.operation(2, result.clone())?
            .emit2::<A, B, R, _>(&mut self.segment, f)?;
        self.replace(2, result);
        Ok(())
    }

    /// Registers a ternary opaque-leaf operation in source order.
    ///
    /// # Errors
    /// Rejects insufficient or mismatched operands.
    /// - Complexity: O(n) registration work for n frame/descriptor nodes.
    ///
    /// # Examples
    /// See [`Self::op2`]; push one additional operand.
    pub fn op3<A: 'static, B: 'static, C: 'static, R: 'static, F: Fn(A, B, C) -> R + 'static>(
        &mut self,
        f: F,
    ) -> Result<()> {
        self.op3_repr::<Leaf<A>, Leaf<B>, Leaf<C>, Leaf<R>, _>(f)
    }

    /// Registers a fallible ternary opaque-leaf operation.
    ///
    /// # Errors
    /// Rejects insufficient/mismatched operands; execution propagates host errors.
    /// - Complexity: O(n) registration work for n frame/descriptor nodes.
    ///
    /// # Examples
    /// See [`Self::op2r`]; push one additional operand.
    pub fn op3r<
        A: 'static,
        B: 'static,
        C: 'static,
        R: 'static,
        F: Fn(A, B, C) -> Result<R> + 'static,
    >(
        &mut self,
        f: F,
    ) -> Result<()> {
        self.op3r_repr::<Leaf<A>, Leaf<B>, Leaf<C>, Leaf<R>, _>(f)
    }

    /// Registers a ternary witnessed operation in source order.
    ///
    /// # Errors
    /// Rejects insufficient operands or complete witness mismatch.
    /// - Complexity: O(n) registration work for n frame/descriptor nodes.
    ///
    /// # Examples
    /// See [`Self::op2_repr`]; supply one additional witness.
    pub fn op3_repr<
        A: ValueRepr,
        B: ValueRepr,
        C: ValueRepr,
        R: ValueRepr,
        F: Fn(A::Value, B::Value, C::Value) -> R::Value + 'static,
    >(
        &mut self,
        f: F,
    ) -> Result<()> {
        self.op3r_repr::<A, B, C, R, _>(move |a, b, c| Ok(f(a, b, c)))
    }

    /// Registers a fallible ternary witnessed operation.
    ///
    /// # Errors
    /// Rejects insufficient operands or complete mismatch; execution propagates host errors.
    /// - Complexity: O(n) registration work for n frame/descriptor nodes.
    ///
    /// # Examples
    /// See [`Self::op2r_repr`]; supply one additional witness.
    pub fn op3r_repr<
        A: ValueRepr,
        B: ValueRepr,
        C: ValueRepr,
        R: ValueRepr,
        F: Fn(A::Value, B::Value, C::Value) -> Result<R::Value> + 'static,
    >(
        &mut self,
        f: F,
    ) -> Result<()> {
        let result = R::value_type();
        self.operation(3, result.clone())?
            .emit3::<A, B, C, R, _>(&mut self.segment, f)?;
        self.replace(3, result);
        Ok(())
    }

    /// Transfers a canonical array allocation into a concrete host vector.
    ///
    /// # Errors
    /// Rejects insufficient operands or a differing complete element witness.
    /// - Complexity: O(n) registration work; execution preserves the allocation.
    ///
    /// # Examples
    /// See the `prepared_containers` integration tests.
    pub fn op1_vec<E: ValueRepr, R: ValueRepr, F: Fn(Vec<E::Value>) -> R::Value + 'static>(
        &mut self,
        f: F,
    ) -> Result<()> {
        self.op1r_vec::<E, R, _>(move |a| Ok(f(a)))
    }

    /// Registers a fallible zero-copy canonical-array-to-vector adapter.
    ///
    /// # Errors
    /// Rejects witness mismatch; execution propagates host errors with ambient cleanup.
    /// - Complexity: O(n) registration work; O(1) ownership transfer.
    ///
    /// # Examples
    /// See [`Self::op1_vec`].
    pub fn op1r_vec<
        E: ValueRepr,
        R: ValueRepr,
        F: Fn(Vec<E::Value>) -> Result<R::Value> + 'static,
    >(
        &mut self,
        f: F,
    ) -> Result<()> {
        let result = R::value_type();
        self.operation(1, result.clone())?
            .emit_vec::<E, R, _>(&mut self.segment, f)?;
        self.replace(1, result);
        Ok(())
    }

    /// Borrows the original canonical array payload during a host callback.
    ///
    /// The consumed array is destroyed after the callback; the result is owned.
    /// # Errors
    /// Rejects insufficient operands or complete element witness mismatch.
    /// - Complexity: O(n) registration work; O(1) borrowing plus destruction.
    ///
    /// # Examples
    /// See [`Self::op1_vec`].
    pub fn op1_slice<E: ValueRepr, R: ValueRepr, F: Fn(&[E::Value]) -> R::Value + 'static>(
        &mut self,
        f: F,
    ) -> Result<()> {
        self.op1r_slice::<E, R, _>(move |a| Ok(f(a)))
    }

    /// Registers a fallible borrowed-slice adapter with owned result and cleanup.
    ///
    /// # Errors
    /// Rejects complete witness mismatch; execution propagates host errors.
    /// - Complexity: O(n) registration work; O(1) borrowing plus destruction.
    ///
    /// # Examples
    /// See [`Self::op1_slice`].
    pub fn op1r_slice<
        E: ValueRepr,
        R: ValueRepr,
        F: Fn(&[E::Value]) -> Result<R::Value> + 'static,
    >(
        &mut self,
        f: F,
    ) -> Result<()> {
        let result = R::value_type();
        self.operation(1, result.clone())?
            .emit_slice::<E, R, _>(&mut self.segment, f)?;
        self.replace(1, result);
        Ok(())
    }

    /// Pushes a cloned opaque constant on each execution.
    ///
    /// # Panics
    /// Rejects legacy wrapper constants; use [`Self::just_external`] to import them.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>(); b.just(7i32);
    /// ```
    pub fn just<T: Clone + 'static>(&mut self, value: T) {
        self.op0(move || value.clone());
        self.stack_ids.last_mut().unwrap().value_type = ValueType::cloneable_leaf::<T>();
    }

    /// Imports an external constant once and emits only trusted canonical cloning.
    ///
    /// The descriptor must include every borrowed clone capability. Wrapper headers
    /// are converted before registration, never during interpreter execution.
    /// # Errors
    /// Rejects noncanonical descriptors, unavailable cloning, invalid external
    /// identity/shape, unsupported external conversions, or frame overflow.
    /// - Complexity: Boundary validation/import at setup; value cloning on each execution.
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::{DynSegment, DynamicArray, ValueType, Nil, ArrayOf, Leaf, IntoCStackList};
    /// let mut body = DynSegment::new::<()>();
    /// body.just_external(ValueType::array(ValueType::cloneable_leaf::<i32>()),
    ///     Box::new(DynamicArray::try_from_vec(vec![7i32]).unwrap())).unwrap();
    /// let mut body = body.prepare::<Nil, ArrayOf<Leaf<i32>>>().unwrap();
    /// assert_eq!(body.call(&().into_c_stack_list()).unwrap().as_slice(), [7]);
    /// ```
    pub fn just_external(&mut self, value_type: ValueType, value: Box<dyn Any>) -> Result<()> {
        value_type.validate_canonical()?;
        value_type.validate_clone()?;
        let owner = HostValuePlan::new(value_type.clone())?.import_box(&mut Some(value))?;
        let result = ResultPlan::new(self.current_stack_offset(), value_type.clone())?;
        let cleanup = CleanupPlan::new(self.base_stack_index, &self.types(self.stack_ids.len()))?;
        let constant_type = value_type.clone();
        self.segment.update_base_alignment(value_type.align());
        self.segment.raw0_(move |stack| {
            let mut guard = cleanup.guard(stack);
            let allocation = crate::owned_value::Allocation::new(constant_type.layout());
            // SAFETY: Import validated the exact complete representation and
            // clone capability before this instruction was registered.
            unsafe {
                clone_raw(&constant_type, owner.as_ptr(), allocation.pointer.as_ptr());
                let mut value = OwnedValue::from_allocation(allocation, constant_type.clone());
                result.push(guard.stack, &mut value);
            }
            guard.armed = false;
            Ok(())
        });
        self.replace(0, value_type);
        Ok(())
    }

    /// Registers a borrowed opaque-leaf argument read.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.push_arg::<i32>(0);
    /// assert_eq!(b.call_dyn::<i32>(&[&7i32]).unwrap(), 7);
    /// ```
    pub fn push_arg<T: Clone + 'static>(&mut self, index: usize) {
        self.push_argument(index, ValueType::cloneable_leaf::<T>())
            .expect("cloneable leaf");
    }

    /// Registers a legacy monomorphized argument factory using complete metadata.
    pub(crate) fn push_typed_arg<T: Clone + 'static>(
        &mut self,
        index: usize,
        value_type: ValueType,
    ) {
        let _ = std::marker::PhantomData::<T>;
        self.push_argument(index, value_type)
            .expect("registered clone capability");
    }

    /// Registers a canonical borrowed argument read with its frozen clone plan.
    ///
    /// # Errors
    /// Rejects missing clone capability or unrepresentable frame positions.
    /// Preparation validates its complete identity against the argument witness.
    /// - Complexity: O(n) registration work; execution has the value's clone cost.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.push_argument(0, cel_runtime::ValueType::cloneable_leaf::<i32>()).unwrap();
    /// ```
    pub fn push_argument(&mut self, index: usize, value_type: ValueType) -> Result<()> {
        value_type.validate_canonical()?;
        ensure!(
            value_type.can_clone(),
            "argument clone capability unavailable"
        );
        let cleanup = CleanupPlan::new(self.base_stack_index, &self.types(self.stack_ids.len()))?;
        let result = ResultPlan::new(self.current_stack_offset(), value_type.clone())?;
        let read_type = value_type.clone();
        self.segment.update_base_alignment(value_type.align());
        self.segment.raw0_(move |stack| {
            let mut guard = cleanup.guard(stack);
            // SAFETY: The external boundary or preparation validates the complete
            // pointer frame; this read's frozen descriptor supplies cloning.
            let allocation = crate::owned_value::Allocation::new(read_type.layout());
            unsafe {
                let source = ARGUMENTS.with(|slot| *slot.get().add(index));
                clone_raw(&read_type, source, allocation.pointer.as_ptr());
                let mut owner = OwnedValue::from_allocation(allocation, read_type.clone());
                result.push(guard.stack, &mut owner);
            }
            guard.armed = false;
            Ok(())
        });
        self.argument_reads.push((index, value_type.clone()));
        self.replace(0, value_type);
        Ok(())
    }

    /// Registers an external tuple argument as a canonical borrowed tuple read.
    ///
    /// Conversion occurs only at `call_dyn`'s checked external boundary.
    /// Prepared calls instead borrow the exact canonical witness directly.
    /// # Errors
    /// Rejects invalid tuple layout or missing recursive clone capabilities.
    /// - Complexity: O(n) registration for n frame/descriptor nodes.
    ///
    /// # Examples
    /// ```
    /// let mut body = cel_runtime::DynSegment::new::<()>();
    /// body.push_arg_as_dynamic_sequence_tuple(0, vec![]).unwrap();
    /// ```
    pub fn push_arg_as_dynamic_sequence_tuple(
        &mut self,
        index: usize,
        associated: Vec<AssociatedType>,
    ) -> Result<()> {
        self.push_argument(index, ValueType::tuple(associated)?)
    }

    /// Packs source-ordered operands into their canonical concrete list layout.
    ///
    /// # Errors
    /// Rejects insufficient operands, a wrong ambient snapshot, or layout overflow.
    /// - Complexity: O(n + s) registration and raw moves for n fields and s bytes.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.just(1i32); b.just(2f64); b.make_tuple(2, 0).unwrap();
    /// ```
    pub fn make_tuple(&mut self, n: usize, ambient_start: usize) -> Result<()> {
        ensure!(
            self.stack_offset_before_top(n)? == ambient_start,
            "tuple ambient origin mismatch"
        );
        let fields = self
            .peek_stack_infos(n)
            .iter()
            .map(|info| AssociatedType {
                offset: 0,
                value_type: info.value_type.clone(),
            })
            .collect();
        let value_type = ValueType::tuple(fields)?;
        let frame = StackFramePlan::new(ambient_start, value_type.clone())?;
        let result = ResultPlan::new(ambient_start, value_type.clone())?;
        self.segment.update_base_alignment(value_type.align());
        self.segment.raw0_(move |stack| {
            // SAFETY: The frozen source frame matches every emitted operand.
            let mut owner = unsafe { frame.take(stack) };
            unsafe { result.push(stack, &mut owner) };
            Ok(())
        });
        self.replace(n, value_type);
        Ok(())
    }

    /// Packs homogeneous operands into a canonical descriptor-free array header.
    ///
    /// # Errors
    /// Rejects empty inference, differing complete types, wrong ambient origin,
    /// insufficient operands, or allocation-layout overflow.
    /// - Complexity: O(n + s) checks and moves for n elements and s bytes.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.just(1i32); b.make_array(1, 0).unwrap();
    /// ```
    pub fn make_array(&mut self, n: usize, ambient_start: usize) -> Result<()> {
        ensure!(
            n != 0,
            "empty array requires an explicit complete element type"
        );
        self.stack_offset_before_top(n)?;
        let element = self.peek_stack_infos(n)[0].value_type.clone();
        self.make_typed_array(n, ambient_start, element)
    }

    /// Packs operands using an explicit complete element descriptor.
    ///
    /// # Errors
    /// Rejects wrong ambient origin, insufficient/differently shaped operands,
    /// or allocation-layout overflow. Empty arrays need no element capabilities.
    /// - Complexity: O(n + s) checks and moves for n elements and s bytes.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.make_typed_array(0, 0, cel_runtime::ValueType::leaf::<i32>()).unwrap();
    /// ```
    pub fn make_typed_array(
        &mut self,
        n: usize,
        ambient_start: usize,
        element: ValueType,
    ) -> Result<()> {
        element.validate_canonical()?;
        ensure!(
            self.stack_offset_before_top(n)? == ambient_start,
            "array ambient origin mismatch"
        );
        for (index, info) in self.peek_stack_infos(n).iter().enumerate() {
            ensure!(
                info.value_type.same_shape(&element),
                "array element {index} has type {}, expected {}",
                descriptor_name(&info.value_type),
                descriptor_name(&element)
            );
        }
        let container = ContainerPlan::array(element.clone(), n)?;
        let result_type = ValueType::array(element.clone());
        let result = ResultPlan::new(ambient_start, result_type.clone())?;
        let mut end = ambient_start;
        let offsets = (0..n)
            .map(|_| {
                let (offset, next) = stack_position(end, element.layout())?;
                end = next;
                Ok(offset)
            })
            .collect::<Result<Vec<_>>>()?;
        self.segment.update_base_alignment(result_type.align());
        self.segment.raw0_(move |stack| {
            // No user code or fallible operation follows the first child move.
            let mut owner = unsafe {
                container.construct(|index, destination| {
                    stack.read_at(offsets[index], |source| {
                        std::ptr::copy_nonoverlapping(source, destination, element.size())
                    });
                    Ok(())
                })?
            };
            unsafe {
                stack.truncate_to(ambient_start, false);
                result.push(stack, &mut owner);
            }
            Ok(())
        });
        self.replace(n, result_type);
        Ok(())
    }

    /// Moves one source-ordered tuple field and destroys every other field.
    ///
    /// # Errors
    /// Rejects a missing tuple or an out-of-range index before emitting.
    /// - Complexity: O(n) registration/destruction for n fields, plus representation moves.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.just(1i32); b.just(2f64); b.make_tuple(2, 0).unwrap();
    /// b.tuple_index(0).unwrap(); assert_eq!(b.call0::<i32>().unwrap(), 1);
    /// ```
    pub fn tuple_index(&mut self, index: usize) -> Result<()> {
        let ty = self
            .stack_ids
            .last()
            .ok_or_else(|| anyhow!("missing tuple"))?
            .value_type
            .clone();
        let fields = ty
            .tuple_elements()
            .ok_or_else(|| anyhow!("tuple required"))?;
        let field = fields
            .get(index)
            .ok_or_else(|| anyhow!("tuple index out of range"))?
            .value_type
            .clone();
        let ambient = self.stack_offset_before_top(1)?;
        let input = ResultPlan::new(ambient, ty)?;
        let output = ResultPlan::new(ambient, field.clone())?;
        let cleanup =
            CleanupPlan::new(self.base_stack_index, &self.types(self.stack_ids.len() - 1))?;
        self.segment.raw0_(move |stack| {
            let mut guard = cleanup.guard(stack);
            let owner = unsafe { input.take(guard.stack) };
            let mut selected = unsafe { owner.take_field(index) };
            unsafe { output.push(guard.stack, &mut selected) };
            guard.armed = false;
            Ok(())
        });
        self.replace(1, field);
        Ok(())
    }

    /// Moves an indexed canonical array element and destroys the consumed array's siblings.
    ///
    /// The stack holds the array followed by an opaque index of type `I`.
    /// No clone capability is required, including for nested tuple/array elements.
    /// # Errors
    /// Rejects absent/nonmatching operands during registration. Execution rejects
    /// negative/unrepresentable or out-of-bounds indices and cleans every operand.
    /// - Complexity: O(n) setup/destruction for n stack slots or array elements.
    ///
    /// # Examples
    /// ```
    /// let mut body = cel_runtime::DynSegment::new::<()>();
    /// body.just(7i32); body.make_array(1, 0).unwrap();
    /// body.just(0i32); body.array_index::<i32>().unwrap();
    /// assert_eq!(body.call0::<i32>().unwrap(), 7);
    /// ```
    pub fn array_index<I: TryInto<usize> + 'static>(&mut self) -> Result<()> {
        let first = self
            .stack_ids
            .len()
            .checked_sub(2)
            .ok_or_else(|| anyhow!("array index requires two operands"))?;
        self.stack_ids[first + 1]
            .value_type
            .validate_opaque_leaf(TypeId::of::<I>())?;
        let array_type = self.stack_ids[first].value_type.clone();
        let element = array_type
            .array_element()
            .ok_or_else(|| anyhow!("canonical array required"))?
            .clone();
        let ambient = self.offset_after(first);
        let index = ResultPlan::new(self.offset_after(first + 1), ValueType::leaf::<I>())?;
        let array = ResultPlan::new(ambient, array_type)?;
        let output = ResultPlan::new(ambient, element.clone())?;
        let cleanup = CleanupPlan::new(self.base_stack_index, &self.types(first))?;
        self.segment.update_base_alignment(element.align());
        self.segment.raw0_(move |stack| {
            let mut guard = cleanup.guard(stack);
            let mut index_owner = unsafe { index.take(guard.stack) };
            let array_owner = unsafe { array.take(guard.stack) };
            let mut native = MaybeUninit::<I>::uninit();
            unsafe { index_owner.move_into(native.as_mut_ptr().cast()) };
            let native = unsafe { native.assume_init() };
            let selected = native
                .try_into()
                .map_err(|_| anyhow!("array index is negative or unrepresentable"))
                .and_then(|index| unsafe { array_owner.take_array_element(index) });
            match selected {
                Ok(mut value) => {
                    unsafe { output.push(guard.stack, &mut value) };
                    guard.armed = false;
                    Ok(())
                }
                Err(error) => {
                    guard.destroy();
                    Err(error)
                }
            }
        });
        self.replace(2, element);
        Ok(())
    }

    /// Destroys the top canonical value without synthesizing a result.
    ///
    /// Ambient values survive success and are cleaned if a destructor unwinds.
    /// # Errors
    /// Rejects an empty stack or unrepresentable frame before registration.
    /// - Complexity: O(n) setup for n stack slots; value-dependent destruction.
    ///
    /// # Examples
    /// ```
    /// let mut body = cel_runtime::DynSegment::new::<()>();
    /// body.just(1i32); body.make_array(1, 0).unwrap();
    /// body.drop_value().unwrap();
    /// body.just(7i32); assert_eq!(body.call0::<i32>().unwrap(), 7);
    /// ```
    pub fn drop_value(&mut self) -> Result<()> {
        let first = self
            .stack_ids
            .len()
            .checked_sub(1)
            .ok_or_else(|| anyhow!("missing value"))?;
        let input = ResultPlan::new(
            self.offset_after(first),
            self.stack_ids[first].value_type.clone(),
        )?;
        let cleanup = CleanupPlan::new(self.base_stack_index, &self.types(first))?;
        self.segment.raw0_(move |stack| {
            let mut guard = cleanup.guard(stack);
            let owner = unsafe { input.take(guard.stack) };
            drop(owner);
            guard.armed = false;
            Ok(())
        });
        self.stack_ids.truncate(first);
        Ok(())
    }

    /// Validates a complete value using an explicit matching structural witness.
    ///
    /// # Errors
    /// Rejects an absent or mismatched value; this never authorizes opaque casts.
    /// - Complexity: O(n) complete descriptor comparison for n nodes.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>();
    /// b.make_tuple(0, 0).unwrap(); b.relabel_repr::<cel_runtime::Nil>().unwrap();
    /// ```
    pub fn relabel_repr<R: ValueRepr>(&mut self) -> Result<()> {
        self.stack_ids
            .last()
            .ok_or_else(|| anyhow!("missing value"))?
            .value_type
            .validate::<R>()
    }

    /// Installs complete registered capabilities without changing the top value's identity.
    ///
    /// A host registry can retain clone/equality/debug adapters on operation
    /// results for checked external export. No instruction or raw cast is emitted.
    /// # Errors
    /// Rejects a missing value or any recursive identity/layout mismatch.
    /// - Complexity: O(n) complete comparison for n descriptor nodes.
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::{DynSegment, Leaf, ValueType};
    /// let mut body = DynSegment::new::<()>(); body.just(7i32);
    /// body.annotate_top(ValueType::cloneable_leaf::<i32>()
    ///     .with_equality::<Leaf<i32>>().unwrap()
    ///     .with_debug::<Leaf<i32>>().unwrap()).unwrap();
    /// ```
    pub fn annotate_top(&mut self, value_type: ValueType) -> Result<()> {
        let info = self
            .stack_ids
            .last_mut()
            .ok_or_else(|| anyhow!("missing value"))?;
        ensure!(
            info.value_type.same_shape(&value_type),
            "top value annotation representation mismatch"
        );
        info.value_type = value_type;
        Ok(())
    }

    /// Relabels a canonical tuple as its concrete opaque list using an explicit witness.
    ///
    /// No bytes move. The witness authorizes the concrete Rust destructor and
    /// permits an ordinary typed operation or direct `Segment` conversion.
    /// # Errors
    /// Rejects a missing/nontuple value or any complete witness mismatch.
    /// - Complexity: O(n) complete comparison for n descriptor nodes.
    ///
    /// # Examples
    /// ```
    /// let mut body = cel_runtime::DynSegment::new::<()>();
    /// body.make_tuple(0, 0).unwrap();
    /// body.pop_tuple_as::<cel_runtime::Nil>().unwrap();
    /// ```
    pub fn pop_tuple_as<R: ValueRepr>(&mut self) -> Result<()> {
        let info = self
            .stack_ids
            .last_mut()
            .ok_or_else(|| anyhow!("missing tuple"))?;
        ensure!(
            info.value_type.tuple_elements().is_some(),
            "canonical tuple required"
        );
        info.value_type.validate::<R>()?;
        info.value_type = ValueType::leaf::<R::Value>();
        Ok(())
    }

    /// Relabels a concrete opaque canonical list as a structural tuple using its witness.
    ///
    /// # Errors
    /// Rejects missing values, nontuple witnesses, or a different opaque concrete type.
    /// - Complexity: O(n) witness construction for n tuple descriptor nodes.
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::{DynSegment, Nil, IntoCStackList};
    /// let mut body = DynSegment::new::<()>();
    /// body.just(().into_c_stack_list());
    /// body.push_tuple::<Nil>().unwrap();
    /// ```
    pub fn push_tuple<R: ValueRepr>(&mut self) -> Result<()> {
        let value_type = R::value_type();
        ensure!(
            value_type.tuple_elements().is_some(),
            "canonical tuple witness required"
        );
        let info = self
            .stack_ids
            .last_mut()
            .ok_or_else(|| anyhow!("missing concrete list"))?;
        info.value_type
            .validate_opaque_leaf(TypeId::of::<R::Value>())?;
        debug_assert_eq!(info.value_type.layout(), value_type.layout());
        info.value_type = value_type;
        Ok(())
    }

    /// Checks that this body starts on an independent empty raw execution stack.
    ///
    /// # Errors
    /// Rejects preloaded arguments or a nonzero fragment origin.
    pub(crate) fn validate_independent_body(&self) -> Result<()> {
        ensure!(
            self.argument_ids.is_empty(),
            "independent body must not require preloaded arguments"
        );
        ensure!(
            self.base_stack_index == 0,
            "independent body must not require a nonzero ambient stack origin"
        );
        Ok(())
    }

    /// Executes the existing raw instruction sequence with trusted argument pointers.
    ///
    /// # Safety
    /// The independent body and every read's complete identity/count/clone plan
    /// must have been validated; arguments must stay live until return.
    /// # Errors
    /// Propagates body errors without changing their context.
    /// - Complexity: Body-dependent; no descriptor comparisons or `Any` downcasts.
    pub(crate) unsafe fn execute_raw(&mut self, arguments: &[*const u8]) -> Result<RawStack> {
        let _guard = ArgumentGuard::install(arguments);
        let mut output = RawStack::with_base_alignment(self.segment.base_alignment());
        unsafe { self.segment.call0_stack(&mut output)? };
        Ok(output)
    }

    /// Evaluates a result at a checked external host boundary.
    ///
    /// Structural results export legacy host wrappers; callable results return
    /// their concrete carrier. Interpreter operations never use this conversion.
    ///
    /// # Errors
    /// Rejects dependent bodies, non-single results, unsupported external shapes,
    /// unavailable host capabilities, or mismatched arguments/results; propagates body errors.
    /// - Complexity: Boundary validation and argument clones plus evaluation.
    ///
    /// # Examples
    /// See [`Self::push_arg`].
    pub fn call_dyn<R: 'static>(&mut self, inputs: &[&dyn Any]) -> Result<R> {
        let result = self.result_type()?;
        match result.kind() {
            ValueKind::Tuple(fields)
                if fields.is_empty() && TypeId::of::<R>() == TypeId::of::<()>() =>
            {
                drop(self.call_dyn_value(inputs)?);
                let unit: Box<dyn Any> = Box::new(());
                return Ok(*unit.downcast::<R>().expect("checked host unit identity"));
            }
            ValueKind::Array(_) | ValueKind::Tuple(_) => {
                let external = match result.kind() {
                    ValueKind::Array(_) => TypeId::of::<crate::DynamicArray>(),
                    _ => TypeId::of::<crate::DynamicSequence>(),
                };
                ensure!(
                    TypeId::of::<R>() == external,
                    "external result type mismatch"
                );
                return self
                    .call_dyn_external(inputs)?
                    .downcast::<R>()
                    .map(|value| *value)
                    .map_err(|_| anyhow!("external result type mismatch"));
            }
            ValueKind::Callable { .. } => {
                ensure!(
                    TypeId::of::<R>() == TypeId::of::<crate::DynClosure>(),
                    "external callable result type mismatch"
                );
            }
            ValueKind::Leaf(_) => result.validate_opaque_leaf(TypeId::of::<R>())?,
        }
        let plan = ResultPlan::new(0, self.result_type()?.clone())?;
        let mut output = self.call_external(inputs)?;
        Ok(unsafe { plan.take_typed::<R>(&mut output) })
    }

    /// Evaluates an owned canonical value through a checked external argument boundary.
    ///
    /// # Errors
    /// Rejects dependent bodies, non-single results, or invalid external arguments.
    /// Propagates operation errors without erasing their context.
    /// - Complexity: Boundary validation/cloning plus body evaluation and one raw result move.
    ///
    /// # Examples
    /// ```
    /// let mut body = cel_runtime::DynSegment::new::<()>();
    /// body.just(7i32);
    /// assert_eq!(body.call_dyn_value(&[]).unwrap().try_into_typed::<cel_runtime::Leaf<i32>>().unwrap(), 7);
    /// ```
    pub fn call_dyn_value(&mut self, inputs: &[&dyn Any]) -> Result<OwnedValue> {
        let result = ResultPlan::new(0, self.result_type()?.clone())?;
        let mut output = self.call_external(inputs)?;
        Ok(unsafe { result.take(&mut output) })
    }

    /// Exports a result as an explicitly checked external host owner.
    ///
    /// Structural tuples become `DynamicSequence`; arrays become `DynamicArray`.
    /// This boundary is not used by prepared execution or emitted instructions.
    /// # Errors
    /// Rejects unsupported external shapes/capabilities before evaluation, invalid
    /// external arguments, data-dependent export overflow, or operation errors.
    /// - Complexity: Boundary checks/conversions plus body evaluation.
    ///
    /// # Examples
    /// ```
    /// let mut body = cel_runtime::DynSegment::new::<()>(); body.just(7i32);
    /// assert_eq!(*body.call_dyn_external(&[]).unwrap().downcast::<i32>().unwrap(), 7);
    /// ```
    pub fn call_dyn_external(&mut self, inputs: &[&dyn Any]) -> Result<Box<dyn Any>> {
        let plan = HostValuePlan::new(self.result_type()?.clone())?;
        plan.validate_export_capabilities()?;
        let mut owner = Some(self.call_dyn_value(inputs)?);
        plan.export_box(&mut owner)
    }

    /// Exports a canonical tuple using capabilities installed in its complete leaves.
    ///
    /// # Errors
    /// Rejects a nontuple result, unsupported tuple fields, missing external leaf
    /// capabilities, invalid external inputs, or operation errors.
    /// - Complexity: Boundary checks/conversions plus body evaluation.
    ///
    /// # Examples
    /// ```
    /// let mut body = cel_runtime::DynSegment::new::<()>();
    /// body.make_tuple(0, 0).unwrap();
    /// assert_eq!(body.call_dyn_as_dynamic_sequence(&[]).unwrap().arity(), 0);
    /// ```
    pub fn call_dyn_as_dynamic_sequence(
        &mut self,
        inputs: &[&dyn Any],
    ) -> Result<crate::DynamicSequence> {
        ensure!(
            self.result_type()?.tuple_elements().is_some(),
            "tuple result required"
        );
        self.call_dyn_external(inputs)?
            .downcast::<crate::DynamicSequence>()
            .map(|value| *value)
            .map_err(|_| anyhow!("external tuple representation mismatch"))
    }

    /// Reconstructs a native Rust tuple whose fields are opaque canonical leaves.
    ///
    /// Structural nested tuples need an explicit representation witness instead.
    /// # Errors
    /// Rejects non-single/nontuple results, arity or concrete field mismatches,
    /// invalid external inputs, or operation errors before transferring ownership.
    /// - Complexity: Boundary checks and raw moves plus body evaluation.
    ///
    /// # Examples
    /// ```
    /// let mut body = cel_runtime::DynSegment::new::<()>();
    /// body.just(7i32); body.just(2f64); body.make_tuple(2, 0).unwrap();
    /// assert_eq!(body.call_dyn_as_tuple::<(i32, f64)>(&[]).unwrap(), (7, 2.0));
    /// ```
    pub fn call_dyn_as_tuple<T: crate::TupleSequence>(&mut self, inputs: &[&dyn Any]) -> Result<T>
    where
        T::Output: crate::SequenceList,
    {
        let fields = self
            .result_type()?
            .tuple_elements()
            .ok_or_else(|| anyhow!("tuple result required"))?;
        let mut shape = Vec::new();
        <T::Output as crate::SequenceList>::append_shape(&mut shape, 0, &mut 1);
        ensure!(fields.len() == shape.len(), "native tuple arity mismatch");
        for (field, expected) in fields.iter().zip(&shape) {
            field.value_type.validate_opaque_leaf(expected.type_id)?;
            ensure!(
                field.value_type.size() == expected.size
                    && field.value_type.align() == expected.align,
                "native tuple field layout mismatch"
            );
        }
        // SAFETY: TupleSequence's monomorphized shape supplies each concrete
        // leaf's hooks, after exact field identity and layout validation.
        let value_type = unsafe {
            external_tuple_type(self.result_type()?, &|id| {
                shape
                    .iter()
                    .find(|field| field.type_id == id)
                    .map(|field| (field.drop, field.clone, field.eq, field.debug))
            })?
        };
        let plan = HostValuePlan::new(value_type)?;
        plan.validate_export_capabilities()?;
        let mut owner = Some(self.call_dyn_value(inputs)?);
        let sequence = plan
            .export_box(&mut owner)?
            .downcast::<crate::DynamicSequence>()
            .map_err(|_| anyhow!("external tuple representation mismatch"))?;
        sequence.try_into_tuple::<T>()
    }

    /// Splits one canonical tuple into scalar boxes and checked external nested tuples.
    ///
    /// # Safety
    /// Scalar extractors must clone their named concrete leaves. Tuple lookups must
    /// return hooks for exactly the leaf type named by each lookup key.
    /// # Errors
    /// Rejects arity/leaf mismatches, unsupported or unregistered tuple fields,
    /// invalid external arguments, or operation errors. Static checks precede evaluation.
    /// - Complexity: Boundary checks/conversions and source field moves plus evaluation.
    ///
    /// # Examples
    /// Use [`Self::call_dyn_tuple`] for all-scalar results or
    /// [`Self::call_dyn_value`] to retain canonical ownership.
    pub unsafe fn call_dyn_tuple_mixed(
        &mut self,
        inputs: &[&dyn Any],
        extractors: &[DynExtractor],
    ) -> Result<Vec<Box<dyn Any>>> {
        let fields = self
            .result_type()?
            .tuple_elements()
            .ok_or_else(|| anyhow!("tuple result required"))?;
        ensure!(
            fields.len() == extractors.len(),
            "tuple extractor arity mismatch"
        );
        let plans: Vec<_> = fields
            .iter()
            .zip(extractors)
            .map(|(field, extractor)| match extractor {
                DynExtractor::Scalar(id, _) => {
                    field.value_type.validate_opaque_leaf(*id)?;
                    Ok(None)
                }
                DynExtractor::Tuple(lookup) => {
                    ensure!(
                        field.value_type.tuple_elements().is_some(),
                        "nested tuple field required"
                    );
                    let plan = HostValuePlan::new(unsafe {
                        external_tuple_type(&field.value_type, lookup.as_ref())?
                    })?;
                    plan.validate_export_capabilities()?;
                    Ok(Some(plan))
                }
            })
            .collect::<Result<_>>()?;
        let owners = self.call_dyn_value(inputs)?.into_fields()?;
        owners
            .into_iter()
            .zip(extractors)
            .zip(plans)
            .map(|((owner, extractor), plan)| match extractor {
                DynExtractor::Scalar(_, extract) => Ok(unsafe { extract(owner.as_ptr()) }),
                DynExtractor::Tuple(_) => plan.unwrap().export_box(&mut Some(owner)),
            })
            .collect()
    }

    /// Clones checked opaque fields from a canonical tuple into external boxes.
    ///
    /// # Safety
    /// Every extractor must clone the concrete leaf named by its `TypeId`.
    /// Structural fields require the complete owned/prepared boundary instead.
    /// # Errors
    /// Rejects nontuple results, arity/leaf mismatches, or invalid external arguments.
    /// Propagates operation errors and destroys the full result if extraction unwinds.
    /// - Complexity: Body evaluation plus O(n) external clones for n fields.
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::DynSegment;
    /// use std::any::{Any, TypeId};
    /// unsafe fn clone_i32(p: *const u8) -> Box<dyn Any> {
    ///     Box::new(unsafe { *p.cast::<i32>() })
    /// }
    /// let mut body = DynSegment::new::<()>(); body.just(7i32);
    /// body.make_tuple(1, 0).unwrap();
    /// let fields = unsafe { body.call_dyn_tuple(&[], &[(TypeId::of::<i32>(), clone_i32)]) }.unwrap();
    /// assert_eq!(*fields[0].downcast_ref::<i32>().unwrap(), 7);
    /// ```
    pub unsafe fn call_dyn_tuple(
        &mut self,
        inputs: &[&dyn Any],
        extractors: &[(TypeId, BoxExtractor)],
    ) -> Result<Vec<Box<dyn Any>>> {
        let fields = self
            .result_type()?
            .tuple_elements()
            .ok_or_else(|| anyhow!("tuple result required"))?
            .to_vec();
        ensure!(
            fields.len() == extractors.len(),
            "tuple extractor arity mismatch"
        );
        for (field, (id, _)) in fields.iter().zip(extractors) {
            field.value_type.validate_opaque_leaf(*id)?;
        }
        let owner = self.call_dyn_value(inputs)?;
        fields
            .iter()
            .zip(extractors)
            .map(|(field, (_, extract))| Ok(unsafe { extract(owner.as_ptr().add(field.offset)) }))
            .collect()
    }

    /// Validates and imports external owners before entering the raw interpreter.
    ///
    /// # Errors
    /// Rejects dependent bodies or invalid external reads; propagates body errors.
    /// - Complexity: Boundary checking/cloning plus body execution.
    fn call_external(&mut self, inputs: &[&dyn Any]) -> Result<RawStack> {
        self.validate_independent_body()?;
        for (position, (index, ty)) in self.argument_reads.iter().enumerate() {
            ensure!(*index < inputs.len(), "missing argument {index}");
            for (other_index, other) in &self.argument_reads[..position] {
                ensure!(
                    *index != *other_index || ty.same_shape(other),
                    "inconsistent argument {index} representation"
                );
            }
        }
        let mut owners: Vec<Option<OwnedValue>> = (0..inputs.len()).map(|_| None).collect();
        for (index, ty) in &self.argument_reads {
            if owners[*index].is_some() {
                continue;
            }
            let input = inputs
                .get(*index)
                .ok_or_else(|| anyhow!("missing argument {index}"))?;
            let owner = HostValuePlan::new(ty.clone())?.clone_any(*input)?;
            owners[*index] = Some(owner);
        }
        let pointers: Vec<_> = owners
            .iter()
            .map(|owner| owner.as_ref().map_or(std::ptr::null(), OwnedValue::as_ptr))
            .collect();
        unsafe { self.execute_raw(&pointers) }
    }

    /// Borrows the sole canonical result metadata.
    ///
    /// # Errors
    /// Rejects zero or multiple results.
    pub(crate) fn result_type(&self) -> Result<&ValueType> {
        ensure!(
            self.stack_ids.len() == 1,
            "body must produce exactly one value"
        );
        Ok(&self.stack_ids[0].value_type)
    }

    /// Executes a checked no-argument host call without consuming type metadata.
    ///
    /// # Errors
    /// Rejects a result mismatch, extra stack values, or required preloaded arguments.
    /// - Complexity: Body-dependent evaluation plus boundary checks.
    ///
    /// # Examples
    /// See [`Self::op0`].
    pub fn call0<R: 'static>(&mut self) -> Result<R> {
        self.call_dyn(&[])
    }

    /// Executes a preloaded single opaque-leaf argument and result.
    ///
    /// # Errors
    /// Rejects nonmatching argument/result identities, positional reads, or extra results.
    /// - Complexity: Body-dependent.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<(i32,)>();
    /// b.op1(|x: i32| x.to_string()).unwrap();
    /// assert_eq!(b.call1::<i32, String>(7).unwrap(), "7");
    /// ```
    pub fn call1<A: 'static, R: 'static>(&mut self, arg: A) -> Result<R> {
        ensure!(
            self.argument_ids == [TypeId::of::<A>()],
            "argument type mismatch"
        );
        ensure!(
            self.argument_reads.is_empty(),
            "preloaded call cannot use positional argument reads"
        );
        self.result_type()?
            .validate_opaque_leaf(TypeId::of::<R>())?;
        unsafe { self.segment.call1(arg) }
    }

    /// Prepares a complete borrowed argument list and owned result once.
    ///
    /// # Errors
    /// Rejects dependent bodies, non-list arguments, mismatched reads/results,
    /// missing borrowed clone capabilities, or inconsistent reads of one argument.
    /// - Complexity: O(n) complete comparisons for n frame/descriptor nodes.
    ///
    /// # Examples
    /// See the module example.
    pub fn prepare<A: ValueRepr, R: ValueRepr>(self) -> Result<crate::PreparedSegment<A, R>> {
        crate::PreparedSegment::new(self)
    }

    /// Pushes a known callable with complete metadata and shared body ownership.
    ///
    /// # Errors
    /// Rejects an untyped/invalid closure or unavailable trusted binding.
    /// - Complexity: O(n) signature/body validation for n descriptor nodes.
    ///
    /// # Examples
    /// See [`Self::invoke_closure`].
    pub fn push_closure(&mut self, closure: crate::DynClosure) -> Result<()> {
        let signature = closure
            .signature()
            .ok_or_else(|| anyhow!("closure signature metadata is unavailable"))?;
        closure.validate_body_signature()?;
        let ty = ValueType::callable(signature.parameters.clone(), signature.result.clone());
        // The lease is retained by the emitted constant, across every alias.
        let closure = closure.bind_canonical()?;
        let cleanup = CleanupPlan::new(self.base_stack_index, &self.types(self.stack_ids.len()))?;
        self.segment.update_base_alignment(ty.align());
        self.segment.raw0_(move |stack| {
            let mut guard = cleanup.guard(stack);
            guard.stack.push(closure.clone());
            guard.armed = false;
            Ok(())
        });
        self.replace(0, ty);
        Ok(())
    }

    /// Invokes a canonical callable below its source-ordered evaluated arguments.
    ///
    /// # Errors
    /// Rejects unavailable/mismatched callable metadata or insufficient operands.
    /// Execution propagates body errors, retaining their original context.
    /// - Complexity: O(n) setup; body execution plus argument/result raw moves.
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::{DynClosure, DynSegment};
    /// let mut body = DynSegment::new::<()>(); body.just(7i32);
    /// let closure = DynClosure::typed(vec![], body).unwrap();
    /// let mut b = DynSegment::new::<()>();
    /// b.push_closure(closure).unwrap(); b.invoke_closure(0).unwrap();
    /// assert_eq!(b.call0::<i32>().unwrap(), 7);
    /// ```
    pub fn invoke_closure(&mut self, arity: usize) -> Result<()> {
        ensure!(
            self.argument_ids.is_empty(),
            "closure invocation requires a positional body"
        );
        let first = self
            .stack_ids
            .len()
            .checked_sub(arity + 1)
            .ok_or_else(|| anyhow!("missing callee"))?;
        let callee_type = self.stack_ids[first].value_type.clone();
        let ValueKind::Callable { parameters, result } = callee_type.kind() else {
            anyhow::bail!("callable signature required")
        };
        ensure!(parameters.len() == arity, "closure arity mismatch");
        ensure!(
            parameters
                .iter()
                .zip(&self.stack_ids[first + 1..])
                .all(|(expected, actual)| expected.same_shape(&actual.value_type)),
            "closure argument type mismatch"
        );
        let result_type = result.as_ref().clone();
        let ambient = self.offset_after(first);
        let cleanup = CleanupPlan::new(self.base_stack_index, &self.types(first))?;
        let argument_type = ValueType::tuple(
            parameters
                .iter()
                .map(|value_type| AssociatedType {
                    offset: 0,
                    value_type: value_type.clone(),
                })
                .collect(),
        )?;
        let arguments_start = self.offset_after(first + 1);
        let frame = StackFramePlan::new(arguments_start, argument_type.clone())?;
        let callee = ResultPlan::new(ambient, callee_type.clone())?;
        let output = ResultPlan::new(0, result_type.clone())?;
        let destination = ResultPlan::new(ambient, result_type.clone())?;
        let fields = argument_type.tuple_elements().unwrap().to_vec();
        self.segment.update_base_alignment(result_type.align());
        self.segment.raw0_(move |stack| {
            // Move every operand to independent guarded owners before user code.
            let mut guard = cleanup.guard(stack);
            let arguments = unsafe { frame.take(guard.stack) };
            let mut callee_owner = unsafe { callee.take(guard.stack) };
            let mut closure = MaybeUninit::<crate::DynClosure>::uninit();
            unsafe { callee_owner.move_into(closure.as_mut_ptr().cast()) };
            let closure = unsafe { closure.assume_init() };
            // Every producer of structural callables freezes its carrier. An
            // imported opaque DynClosure cannot acquire structural metadata.
            let pointers: Vec<_> = fields
                .iter()
                .map(|field| unsafe { arguments.as_ptr().add(field.offset) })
                .collect();
            let evaluated = unsafe { closure.call_raw_trusted(&pointers) };
            match evaluated {
                Ok(mut stack) => {
                    let mut result = unsafe { output.take(&mut stack) };
                    drop(arguments);
                    drop(closure);
                    unsafe { destination.push(guard.stack, &mut result) };
                    guard.armed = false;
                    Ok(())
                }
                Err(error) => {
                    drop(arguments);
                    drop(closure);
                    guard.destroy();
                    Err(error)
                }
            }
        });
        self.replace(arity + 1, result_type);
        Ok(())
    }

    /// Joins conditional fragments with identical complete results and origins.
    ///
    /// # Errors
    /// Rejects a nonboolean condition, wrong fragment origins, or different results.
    /// - Complexity: O(n) setup for n frame/descriptor nodes; executes one branch.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>(); b.just(true);
    /// let mut yes = b.new_fragment(); yes.just(1i32);
    /// let mut no = b.new_fragment(); no.just(2i32);
    /// b.join2(yes, no).unwrap(); assert_eq!(b.call0::<i32>().unwrap(), 1);
    /// ```
    pub fn join2(&mut self, yes: Self, no: Self) -> Result<()> {
        let origin = self.stack_offset_before_top(1)?;
        self.stack_ids
            .last()
            .unwrap()
            .value_type
            .validate_opaque_leaf(TypeId::of::<bool>())?;
        ensure!(
            yes.base_stack_index == origin && no.base_stack_index == origin,
            "fragment origins must match"
        );
        ensure!(
            yes.argument_ids.is_empty() && no.argument_ids.is_empty(),
            "fragments cannot take preloaded arguments"
        );
        let result = yes.result_type()?.clone();
        ensure!(
            result.same_shape(no.result_type()?),
            "fragment result types must match"
        );
        let cleanup =
            CleanupPlan::new(self.base_stack_index, &self.types(self.stack_ids.len() - 1))?;
        let condition = ResultPlan::new(origin, ValueType::leaf::<bool>())?;
        self.segment.update_base_alignment(
            yes.segment
                .base_alignment()
                .max(no.segment.base_alignment()),
        );
        self.argument_reads.extend(yes.argument_reads);
        self.argument_reads.extend(no.argument_reads);
        self.segment.raw0_(move |stack| {
            let mut owner = unsafe { condition.take(stack) };
            let mut value = MaybeUninit::<bool>::uninit();
            unsafe { owner.move_into(value.as_mut_ptr().cast()) };
            let mut guard = cleanup.guard(stack);
            let branch = if unsafe { value.assume_init() } {
                &yes.segment
            } else {
                &no.segment
            };
            match unsafe { branch.call0_stack(guard.stack) } {
                Ok(()) => {
                    guard.armed = false;
                    Ok(())
                }
                Err(error) => {
                    guard.destroy();
                    Err(error)
                }
            }
        });
        self.replace(1, result);
        Ok(())
    }
}
