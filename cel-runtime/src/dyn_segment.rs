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

use anyhow::{Result, anyhow, ensure};

use crate::canonical_operation::{CleanupPlan, OperationPlan};
use crate::container_execution::{ContainerPlan, ResultPlan, StackFramePlan, stack_position};
use crate::host_value_bridge::HostValuePlan;
use crate::owned_value::OwnedValue;
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
    /// Shares each stack boundary's immutable destruction prefix.
    cleanup_prefixes: Vec<CleanupPlan>,
}

impl DynSegment {
    /// Creates a segment with an optional preloaded opaque-leaf stack.
    ///
    /// # Panics
    /// Rejects preloaded erased whole-value owner carriers; import those at a checked external boundary.
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
        let mut cleanup_prefixes = Vec::with_capacity(stack_ids.len() + 1);
        cleanup_prefixes.push(CleanupPlan::empty(0));
        for info in &stack_ids {
            cleanup_prefixes.push(
                cleanup_prefixes
                    .last()
                    .unwrap()
                    .push(info.value_type.clone())
                    .expect("validated preloaded stack"),
            );
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
            cleanup_prefixes,
        }
    }

    /// Creates a conditional fragment above the ambient stack below the condition.
    ///
    /// - Precondition: at least one operand is present.
    ///
    /// # Examples
    /// See [`Self::join2`].
    pub fn new_fragment(&self) -> Self {
        debug_assert!(!self.stack_ids.is_empty());
        let mut result = Self::new::<()>();
        result.base_stack_index = self.stack_offset_before_top(1).expect("condition present");
        result.cleanup_prefixes[0] = CleanupPlan::empty(result.base_stack_index);
        result
    }

    /// Returns the current canonical stack byte length.
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
    fn offset_after(&self, count: usize) -> usize {
        self.cleanup_prefixes[count].end
    }

    /// Binds ambient cleanup and source-ordered operands without mutating metadata.
    ///
    /// # Errors
    /// Rejects insufficient operands or an unrepresentable frame.
    /// - Complexity: O(n) time and space for operand/result descriptor nodes.
    fn operation(&self, arity: usize, result: ValueType) -> Result<OperationPlan> {
        let first = self
            .stack_ids
            .len()
            .checked_sub(arity)
            .ok_or_else(|| anyhow!("not enough operands"))?;
        let operands: Vec<_> = self.stack_ids[first..]
            .iter()
            .map(|info| info.value_type.clone())
            .collect();
        OperationPlan::with_cleanup(self.cleanup_prefixes[first].clone(), &operands, result)
    }

    /// Replaces emitted operands with their validated canonical result slot.
    ///
    /// - Complexity: O(a + d) for a removed operands and d result descriptor nodes.
    fn replace(&mut self, arity: usize, result: ValueType) {
        self.stack_ids.truncate(self.stack_ids.len() - arity);
        self.cleanup_prefixes.truncate(self.stack_ids.len() + 1);
        let before = self.current_stack_offset();
        let prefix = self
            .cleanup_prefixes
            .last()
            .unwrap()
            .push(result.clone())
            .expect("validated result");
        self.segment.update_base_alignment(result.align());
        self.stack_ids.push(StackInfo {
            padding: stack_position(before, result.layout())
                .expect("validated result")
                .0
                != before,
            value_type: result,
        });
        self.cleanup_prefixes.push(prefix);
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
    /// Rejects erased whole-value owner results or unrepresentable frames. Use `op0_repr`
    /// for fallible registration and `just_external` for checked owner imports.
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
    /// Rejects erased whole-value owner results or unrepresentable frames during registration.
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
    /// Rejects erased whole-value owner constants; use [`Self::just_external`] to import them.
    ///
    /// # Examples
    /// ```
    /// let mut b = cel_runtime::DynSegment::new::<()>(); b.just(7i32);
    /// ```
    pub fn just<T: Clone + 'static>(&mut self, value: T) {
        self.op0(move || value.clone());
        self.replace(1, ValueType::cloneable_leaf::<T>());
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
    /// use cel_runtime::{DynSegment, Array, OwnedValue, ValueType, Nil, ArrayOf, Leaf, IntoCStackList};
    /// let mut body = DynSegment::new::<()>();
    /// body.just_external(ValueType::array(ValueType::cloneable_leaf::<i32>()),
    ///     Box::new(OwnedValue::new::<ArrayOf<Leaf<i32>>>(Array::from_vec(vec![7i32])))).unwrap();
    /// let mut body = body.prepare::<Nil, ArrayOf<Leaf<i32>>>().unwrap();
    /// assert_eq!(body.call(&().into_c_stack_list()).unwrap().as_slice(), [7]);
    /// ```
    pub fn just_external(&mut self, value_type: ValueType, value: Box<dyn Any>) -> Result<()> {
        value_type.validate_canonical()?;
        value_type.validate_clone()?;
        let owner = HostValuePlan::new(value_type.clone())?.import_box(&mut Some(value))?;
        let result = ResultPlan::new(self.current_stack_offset(), value_type.clone())?;
        let cleanup = self.cleanup_prefixes[self.stack_ids.len()].clone();
        self.segment.update_base_alignment(value_type.align());
        self.segment.raw0_(move |stack| {
            let mut guard = cleanup.guard(stack);
            // SAFETY: Import validated the exact complete representation and
            // clone capability before this instruction was registered.
            unsafe {
                result.clone_from(guard.stack, owner.as_ptr());
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

    /// Registers a canonical borrowed argument read with its frozen clone plan.
    ///
    /// # Errors
    /// Rejects missing clone capability or unrepresentable frame positions.
    /// Preparation validates its complete identity against the argument witness.
    /// - Complexity: O(d) registration for d descriptor nodes; execution has the
    ///   value's clone cost without temporary whole-value allocation.
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
        let cleanup = self.cleanup_prefixes[self.stack_ids.len()].clone();
        let result = ResultPlan::new(self.current_stack_offset(), value_type.clone())?;
        self.segment.update_base_alignment(value_type.align());
        self.segment.raw0_(move |stack| {
            let mut guard = cleanup.guard(stack);
            // SAFETY: The external boundary or preparation validates the complete
            // pointer frame; this read's frozen descriptor supplies cloning.
            unsafe {
                let source = ARGUMENTS.with(|slot| *slot.get().add(index));
                result.clone_from(guard.stack, source);
            }
            guard.armed = false;
            Ok(())
        });
        self.argument_reads.push((index, value_type.clone()));
        self.replace(0, value_type);
        Ok(())
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
        let cleanup = self.cleanup_prefixes[self.stack_ids.len() - 1].clone();
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
    /// Moving the validated index into its concrete type does not allocate.
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
        let cleanup = self.cleanup_prefixes[first].clone();
        self.segment.update_base_alignment(element.align());
        self.segment.raw0_(move |stack| {
            let mut guard = cleanup.guard(stack);
            // SAFETY: Construction validates the exact opaque I descriptor.
            let native = unsafe { index.take_typed::<I>(guard.stack) };
            let array_owner = unsafe { array.take(guard.stack) };
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
        let cleanup = self.cleanup_prefixes[first].clone();
        self.segment.raw0_(move |stack| {
            let mut guard = cleanup.guard(stack);
            let owner = unsafe { input.take(guard.stack) };
            drop(owner);
            guard.armed = false;
            Ok(())
        });
        self.stack_ids.truncate(first);
        self.cleanup_prefixes.truncate(first + 1);
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
    /// Structural results return canonical whole-value owners; callable results return
    /// their concrete carrier. Interpreter operations never use this conversion.
    /// Scalar inputs are borrowed after complete input preflight and cloned only
    /// by executed reads; compound inputs borrow checked canonical owners.
    ///
    /// # Errors
    /// Rejects dependent bodies, non-single results, unsupported external shapes,
    /// unavailable argument capabilities, or mismatched arguments/results; propagates body errors.
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
                ensure!(
                    TypeId::of::<R>() == TypeId::of::<OwnedValue>(),
                    "canonical result owner required"
                );
                let owner: Box<dyn Any> = Box::new(self.call_dyn_value(inputs)?);
                return Ok(*owner
                    .downcast::<R>()
                    .map_err(|_| anyhow!("canonical result owner required"))?);
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

    /// Validates all external inputs, then borrows canonical storage.
    ///
    /// Scalar inputs remain borrowed for this call, including nested calls.
    /// Only executed scalar reads clone them. Recursive validation completes
    /// before any clone or body operation runs.
    ///
    /// # Errors
    /// Rejects dependent bodies or invalid external reads; propagates body errors.
    /// - Complexity: O(r + i) preflight for r read descriptor nodes and i inputs,
    ///   plus host-value validation and body execution.
    fn call_external(&mut self, inputs: &[&dyn Any]) -> Result<RawStack> {
        self.validate_independent_body()?;
        let mut first_reads: Vec<Option<&ValueType>> = vec![None; inputs.len()];
        for (index, ty) in &self.argument_reads {
            ensure!(*index < inputs.len(), "missing argument {index}");
            if let Some(first) = first_reads[*index] {
                ensure!(
                    ty.same_shape(first),
                    "inconsistent argument {index} representation"
                );
            } else {
                first_reads[*index] = Some(ty);
            }
        }
        let mut pointers = vec![std::ptr::null(); inputs.len()];
        for (index, ty) in first_reads.into_iter().enumerate() {
            if let Some(ty) = ty {
                pointers[index] = HostValuePlan::new(ty.clone())?.validate_any(inputs[index])?;
            }
        }
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
        let cleanup = self.cleanup_prefixes[self.stack_ids.len()].clone();
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
    /// Moving the bound callee into its concrete carrier does not allocate.
    /// Argument and result transfers retain their erased ownership.
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
        let cleanup = self.cleanup_prefixes[first].clone();
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
        let destination = ResultPlan::new(ambient, result_type.clone())?;
        self.segment.update_base_alignment(result_type.align());
        let output_type = result_type.clone();
        self.segment.raw0_(move |stack| {
            // Move every operand to independent guarded owners before user code.
            let mut guard = cleanup.guard(stack);
            let arguments = unsafe { frame.take(guard.stack) };
            // SAFETY: Every validated callable descriptor uses the DynClosure carrier.
            let closure = unsafe { callee.take_typed::<crate::DynClosure>(guard.stack) };
            // Compiler-produced carriers retain a freeze lease; typed native
            // and prepared payloads have no mutable compiler-facing escape.
            let allocation = crate::owned_value::Allocation::new(output_type.layout());
            let evaluated = unsafe {
                closure.call_frame_trusted(arguments.as_ptr(), allocation.pointer.as_ptr())
            };
            match evaluated {
                Ok(()) => {
                    let mut result = unsafe {
                        crate::OwnedValue::from_allocation(allocation, output_type.clone())
                    };
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
        let cleanup = self.cleanup_prefixes[self.stack_ids.len() - 1].clone();
        let condition = ResultPlan::new(origin, ValueType::leaf::<bool>())?;
        self.segment.update_base_alignment(
            yes.segment
                .base_alignment()
                .max(no.segment.base_alignment()),
        );
        self.argument_reads.extend(yes.argument_reads);
        self.argument_reads.extend(no.argument_reads);
        self.segment.raw0_(move |stack| {
            // SAFETY: Construction validates bool; the bound plan restores the
            // exact ambient origin before the cleanup guard is armed.
            let value = unsafe { condition.take_typed::<bool>(stack) };
            let mut guard = cleanup.guard(stack);
            let branch = if value { &yes.segment } else { &no.segment };
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

#[cfg(test)]
mod performance_tests {
    use super::*;
    use crate::{Array, ArrayOf, Cons, IntoCStackList, Nil};
    use std::cell::RefCell;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::rc::Rc;

    /// Records only live runtime owners, not borrowed sources or registration values.
    #[repr(align(128))]
    struct CloneDrop {
        name: &'static str,
        events: Rc<RefCell<Vec<&'static str>>>,
        fail: Rc<Cell<bool>>,
        owned: bool,
    }

    impl Clone for CloneDrop {
        /// Creates an aligned owner unless this source requests a clone panic.
        fn clone(&self) -> Self {
            assert!(!self.fail.get(), "deliberate clone panic");
            Self {
                name: self.name,
                events: self.events.clone(),
                fail: self.fail.clone(),
                owned: true,
            }
        }
    }

    impl Drop for CloneDrop {
        /// Records each initialized runtime owner exactly once.
        fn drop(&mut self) {
            if self.owned {
                self.events.borrow_mut().push(self.name);
            }
        }
    }

    /// Checks recursive clone rollback, branch origins, reverse cleanup, and replay.
    #[test]
    fn direct_clone_panic_restores_branch_and_ambient_ownership() {
        type Pair = Cons<ArrayOf<Leaf<CloneDrop>>, Cons<Leaf<CloneDrop>, Nil>>;
        for external_constant in [false, true] {
            let events = Rc::new(RefCell::new(Vec::new()));
            let fail = Rc::new(Cell::new(true));
            let leaf = |name, fail: Rc<Cell<bool>>, owned| CloneDrop {
                name,
                events: events.clone(),
                fail,
                owned,
            };
            let descriptor = ValueType::tuple(vec![
                AssociatedType {
                    offset: 0,
                    value_type: ValueType::array(ValueType::cloneable_leaf::<CloneDrop>()),
                },
                AssociatedType {
                    offset: 0,
                    value_type: ValueType::cloneable_leaf::<CloneDrop>(),
                },
            ])
            .unwrap();
            let mut source = Some(
                OwnedValue::new::<Pair>(
                    (
                        Array::from_vec(vec![leaf("child", Rc::new(Cell::new(false)), false)]),
                        leaf("panic", fail.clone(), false),
                    )
                        .into_c_stack_list(),
                )
                .with_value_type(descriptor.clone())
                .unwrap(),
            );
            let mut body = DynSegment::new::<()>();
            for name in ["outer-a", "outer-b"] {
                let log = events.clone();
                body.op0(move || CloneDrop {
                    name,
                    events: log.clone(),
                    fail: Rc::new(Cell::new(false)),
                    owned: true,
                });
            }
            body.just(true);
            let mut yes = body.new_fragment();
            let log = events.clone();
            yes.op0(move || CloneDrop {
                name: "branch",
                events: log.clone(),
                fail: Rc::new(Cell::new(false)),
                owned: true,
            });
            if external_constant {
                yes.just_external(descriptor, Box::new(source.take().unwrap()))
                    .unwrap();
            } else {
                yes.push_argument(0, descriptor).unwrap();
            }
            yes.drop_value().unwrap();
            yes.drop_value().unwrap();
            yes.just(9u32);
            let mut no = body.new_fragment();
            no.just(3u32);
            body.join2(yes, no).unwrap();
            body.op3(|_: CloneDrop, _: CloneDrop, result: u32| result)
                .unwrap();

            let pointers: Vec<_> = source.iter().map(OwnedValue::as_ptr).collect();
            for _ in 0..2 {
                let mut stack = RawStack::with_base_alignment(body.segment.base_alignment());
                let _arguments = ArgumentGuard::install(&pointers);
                // SAFETY: The source and body share the complete bound descriptor.
                assert!(
                    catch_unwind(AssertUnwindSafe(|| unsafe {
                        body.segment.call0_stack(&mut stack)
                    }))
                    .is_err()
                );
                assert_eq!(stack.len(), 0);
                assert_eq!(
                    &*events.borrow(),
                    &["child", "branch", "outer-b", "outer-a"]
                );
                events.borrow_mut().clear();
            }
            fail.set(false);
            let inputs: Vec<&dyn Any> = source.iter().map(|owner| owner as &dyn Any).collect();
            assert_eq!(body.call_dyn::<u32>(&inputs).unwrap(), 9);
            assert_eq!(
                events
                    .borrow()
                    .iter()
                    .filter(|name| **name == "child")
                    .count(),
                1
            );
            assert_eq!(
                events
                    .borrow()
                    .iter()
                    .filter(|name| **name == "panic")
                    .count(),
                1
            );
        }
    }

    /// Rejects inconsistent descriptors before executing any instruction on every call.
    #[test]
    fn inconsistent_reads_precede_body_effects() {
        let effects = Rc::new(Cell::new(0));
        let counter = effects.clone();
        let mut body = DynSegment::new::<()>();
        body.op0(move || counter.set(counter.get() + 1));
        body.drop_value().unwrap();
        body.push_arg::<i32>(0);
        body.drop_value().unwrap();
        body.push_arg::<String>(0);
        body.drop_value().unwrap();
        body.just(7u32);
        for _ in 0..2 {
            let error = body.call_dyn::<u32>(&[&1i32]).unwrap_err();
            assert!(error.to_string().contains("inconsistent argument 0"));
            assert_eq!(effects.get(), 0);
        }
    }

    /// Checks linear retained cleanup metadata for flat literal arrays.
    #[test]
    fn literal_cleanup_metadata_is_linear() {
        for length in [1_000, 2_000, 4_000] {
            let before = crate::canonical_operation::LIVE_CLEANUP_SLOTS.with(Cell::get);
            let mut body = DynSegment::new::<()>();
            for _ in 0..length {
                body.just(1i32);
            }
            body.make_array(length, 0).unwrap();
            let retained = crate::canonical_operation::LIVE_CLEANUP_SLOTS.with(Cell::get) - before;
            eprintln!("{length} literals: {retained} retained cleanup slots");
            assert!(retained <= 4 * length + 16);
            let output = body.call_dyn_value(&[]).unwrap();
            assert_eq!(
                output
                    .try_into_typed::<crate::ArrayOf<Leaf<i32>>>()
                    .unwrap()
                    .len(),
                length
            );
            drop(body);
            crate::canonical_operation::LIVE_CLEANUP_SLOTS
                .with(|count| assert_eq!(count.get(), before));
        }
    }

    /// Checks that borrowed reads and imported constants share their cleanup prefixes.
    #[test]
    fn clone_cleanup_metadata_is_linear() {
        for length in [1_000, 2_000, 4_000] {
            for external_constant in [false, true] {
                let before = crate::canonical_operation::LIVE_CLEANUP_SLOTS.with(Cell::get);
                let mut body = DynSegment::new::<()>();
                for _ in 0..length {
                    if external_constant {
                        body.just_external(ValueType::cloneable_leaf::<i32>(), Box::new(1i32))
                            .unwrap();
                    } else {
                        body.push_arg::<i32>(0);
                    }
                }
                body.make_array(length, 0).unwrap();
                let retained =
                    crate::canonical_operation::LIVE_CLEANUP_SLOTS.with(Cell::get) - before;
                eprintln!(
                    "{length} clone reads, constant={external_constant}: {retained} cleanup slots"
                );
                assert!(retained <= 4 * length + 16);
                let result = body.call_dyn_value(&[&1i32]).unwrap();
                assert_eq!(
                    result.try_into_typed::<ArrayOf<Leaf<i32>>>().unwrap().len(),
                    length
                );
                drop(body);
                crate::canonical_operation::LIVE_CLEANUP_SLOTS
                    .with(|count| assert_eq!(count.get(), before));
            }
        }
    }

    /// Checks that prepared calls allocate only their erased argument and result frames.
    #[test]
    fn prepared_callees_avoid_temporary_operand_allocations() {
        use crate::{Callable, FnOf};

        type Args = Cons<Leaf<u64>, Nil>;
        type Function = FnOf<Args, Leaf<u64>>;
        /// Adds one through a native borrowed argument frame.
        fn increment(args: &<Args as ValueRepr>::Value) -> Result<u64> {
            args.1.checked_add(1).ok_or_else(|| anyhow!("overflow"))
        }
        let mut interpreted = DynSegment::new::<()>();
        interpreted.push_arg::<u64>(0);
        interpreted.just(1u64);
        interpreted
            .op2r(|a: u64, b: u64| a.checked_add(b).ok_or_else(|| anyhow!("overflow")))
            .unwrap();
        let interpreted =
            Callable::from_prepared(interpreted.prepare::<Args, Leaf<u64>>().unwrap());
        for function in [Callable::from_fn(increment), interpreted] {
            for calls in [1u64, 10, 100] {
                let mut body = DynSegment::new::<()>();
                body.just(17u64);
                for _ in 0..calls {
                    let function = function.clone();
                    body.op0_repr::<Function, _>(move || function.clone())
                        .unwrap();
                    body.push_arg::<u64>(0);
                    body.invoke_closure(1).unwrap();
                    body.op2r(|a: u64, b: u64| a.checked_add(b).ok_or_else(|| anyhow!("overflow")))
                        .unwrap();
                }
                let mut prepared = body.prepare::<Args, Leaf<u64>>().unwrap();
                let args = (7u64,).into_c_stack_list();
                let expected = 17 + calls * 8;
                assert_eq!(prepared.call(&args).unwrap(), expected);
                crate::owned_value::ALLOCATIONS.with(|count| count.set(0));
                for _ in 0..100 {
                    assert_eq!(prepared.call(&args).unwrap(), expected);
                }
                let allocations = crate::owned_value::ALLOCATIONS.with(Cell::get);
                assert_eq!(allocations, 200 * calls as usize, "{calls} calls");
            }
        }
    }

    /// Checks that prepared indexing allocates only the erased array and selected value.
    #[test]
    fn prepared_indices_avoid_temporary_operand_allocations() {
        for accesses in [1u64, 10, 100] {
            let mut body = DynSegment::new::<()>();
            body.just(17u64);
            for _ in 0..accesses {
                let array = Array::from_vec(vec![7u64]);
                body.op0_repr::<ArrayOf<Leaf<u64>>, _>(move || array.clone())
                    .unwrap();
                body.push_arg::<i32>(0);
                body.array_index::<i32>().unwrap();
                body.op2r(|a: u64, b: u64| a.checked_add(b).ok_or_else(|| anyhow!("overflow")))
                    .unwrap();
            }
            let mut prepared = body.prepare::<Cons<Leaf<i32>, Nil>, Leaf<u64>>().unwrap();
            let args = (0i32,).into_c_stack_list();
            let expected = 17 + accesses * 7;
            assert_eq!(prepared.call(&args).unwrap(), expected);
            crate::owned_value::ALLOCATIONS.with(|count| count.set(0));
            for _ in 0..100 {
                assert_eq!(prepared.call(&args).unwrap(), expected);
            }
            let allocations = crate::owned_value::ALLOCATIONS.with(Cell::get);
            assert_eq!(allocations, 200 * accesses as usize, "{accesses} accesses");
        }
    }

    /// Checks callee, argument, and aligned ambient ownership across retries and unwinding.
    #[test]
    fn prepared_callee_moves_preserve_error_and_unwind_cleanup() {
        use crate::{Callable, FnOf};

        type Args = Cons<Leaf<CloneDrop>, Nil>;
        type Function = FnOf<Args, Leaf<u64>>;
        let events = Rc::new(RefCell::new(Vec::new()));
        let mode = Rc::new(Cell::new(0u8));
        let leaf = |name| CloneDrop {
            name,
            events: events.clone(),
            fail: Rc::new(Cell::new(false)),
            owned: true,
        };
        let mut body = DynSegment::new::<()>();
        let ambient = leaf("ambient");
        body.op0(move || ambient.clone());
        body.just(9u8);
        let callee_events = events.clone();
        let callee_mode = mode.clone();
        body.op0_repr::<Function, _>(move || {
            let sentinel = CloneDrop {
                name: "callee",
                events: callee_events.clone(),
                fail: Rc::new(Cell::new(false)),
                owned: true,
            };
            let mode = callee_mode.clone();
            let mut inner = DynSegment::new::<()>();
            inner.push_arg::<CloneDrop>(0);
            inner
                .op1r(move |argument: CloneDrop| {
                    let _sentinel = &sentinel;
                    drop(argument);
                    match mode.get() {
                        0 => Ok(7u64),
                        1 => Err(anyhow!("callee failure")),
                        _ => panic!("callee panic"),
                    }
                })
                .unwrap();
            Callable::from_prepared(inner.prepare::<Args, Leaf<u64>>().unwrap())
        })
        .unwrap();
        let argument = leaf("argument");
        body.op0(move || argument.clone());
        body.invoke_closure(1).unwrap();
        body.op3(|ambient: CloneDrop, marker: u8, value: u64| {
            assert_eq!(marker, 9);
            drop(ambient);
            value
        })
        .unwrap();
        let mut prepared = body.prepare::<Nil, Leaf<u64>>().unwrap();
        let args = ().into_c_stack_list();
        for _ in 0..2 {
            for next in [0, 1, 0, 2, 0] {
                mode.set(next);
                let result = catch_unwind(AssertUnwindSafe(|| prepared.call(&args)));
                match next {
                    0 => assert_eq!(result.unwrap().unwrap(), 7),
                    1 => assert_eq!(result.unwrap().unwrap_err().to_string(), "callee failure"),
                    _ => assert!(result.is_err()),
                }
                let expected = if next == 2 {
                    ["argument", "callee", "argument", "ambient"]
                } else {
                    ["argument", "argument", "callee", "ambient"]
                };
                assert_eq!(&*events.borrow(), &expected);
                events.borrow_mut().clear();
            }
        }
    }

    /// Owns an over-aligned index whose conversion can succeed, fail, or unwind.
    #[repr(align(128))]
    struct TrackedIndex {
        mode: u8,
        events: Rc<RefCell<Vec<&'static str>>>,
    }

    impl TryFrom<TrackedIndex> for usize {
        type Error = ();

        /// Converts valid indices, reports conversion failure, or triggers a test unwind.
        fn try_from(index: TrackedIndex) -> std::result::Result<Self, Self::Error> {
            match index.mode {
                0 => Ok(1),
                1 => Err(()),
                2 => panic!("index conversion panic"),
                _ => Ok(2),
            }
        }
    }

    impl Drop for TrackedIndex {
        /// Records destruction of the uniquely owned index on every exit.
        fn drop(&mut self) {
            self.events.borrow_mut().push("index");
        }
    }

    /// Checks index, array, and misaligned ambient ownership across all conversion exits.
    #[test]
    fn prepared_index_moves_preserve_error_and_unwind_cleanup() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mode = Rc::new(Cell::new(0u8));
        let mut body = DynSegment::new::<()>();
        let log = events.clone();
        body.op0(move || CloneDrop {
            name: "ambient",
            events: log.clone(),
            fail: Rc::new(Cell::new(false)),
            owned: true,
        });
        body.just(9u8);
        let origin = body.current_stack_offset();
        for name in ["first", "last"] {
            let log = events.clone();
            body.op0(move || CloneDrop {
                name,
                events: log.clone(),
                fail: Rc::new(Cell::new(false)),
                owned: true,
            });
        }
        body.make_array(2, origin).unwrap();
        let index_events = events.clone();
        let index_mode = mode.clone();
        body.op0(move || TrackedIndex {
            mode: index_mode.get(),
            events: index_events.clone(),
        });
        body.array_index::<TrackedIndex>().unwrap();
        body.op3(|ambient: CloneDrop, marker: u8, value: CloneDrop| {
            assert_eq!(marker, 9);
            drop(ambient);
            value
        })
        .unwrap();
        let mut prepared = body.prepare::<Nil, Leaf<CloneDrop>>().unwrap();
        let args = ().into_c_stack_list();
        for _ in 0..2 {
            for next in [0, 1, 0, 2, 0, 3, 0] {
                mode.set(next);
                let result = catch_unwind(AssertUnwindSafe(|| prepared.call(&args)));
                match next {
                    0 => drop(result.unwrap().unwrap()),
                    1 => assert_eq!(
                        result.unwrap().err().unwrap().to_string(),
                        "array index is negative or unrepresentable"
                    ),
                    2 => assert!(result.is_err()),
                    _ => assert!(
                        result
                            .unwrap()
                            .err()
                            .unwrap()
                            .to_string()
                            .contains("out of bounds")
                    ),
                }
                let expected = if next == 0 {
                    ["index", "first", "ambient", "last"]
                } else {
                    ["index", "last", "first", "ambient"]
                };
                assert_eq!(&*events.borrow(), &expected);
                events.borrow_mut().clear();
            }
        }
    }

    /// Checks that prepared conditionals and short-circuit branches allocate no owners.
    #[test]
    fn prepared_branches_avoid_temporary_allocations() {
        type Inputs = Cons<Leaf<bool>, Cons<Leaf<bool>, Nil>>;
        for kind in ["if", "&&", "||"] {
            for conditions in [1u64, 10, 100] {
                let mut body = DynSegment::new::<()>();
                body.just(17u64);
                for _ in 0..conditions {
                    body.push_arg::<bool>(0);
                    let mut yes = body.new_fragment();
                    let mut no = body.new_fragment();
                    match kind {
                        "if" => {
                            yes.push_arg::<bool>(1);
                            no.push_arg::<bool>(1);
                            no.op1(|value: bool| !value).unwrap();
                        }
                        "&&" => {
                            yes.push_arg::<bool>(1);
                            no.just(false);
                        }
                        "||" => {
                            yes.just(true);
                            no.push_arg::<bool>(1);
                        }
                        _ => unreachable!(),
                    }
                    body.join2(yes, no).unwrap();
                    body.op2r(|total: u64, value: bool| {
                        total
                            .checked_add(u64::from(value))
                            .ok_or_else(|| anyhow!("overflow"))
                    })
                    .unwrap();
                }
                let mut prepared = body.prepare::<Inputs, Leaf<u64>>().unwrap();
                for condition in [false, true] {
                    for rhs in [false, true] {
                        let value = match kind {
                            "if" => {
                                if condition {
                                    rhs
                                } else {
                                    !rhs
                                }
                            }
                            "&&" => condition && rhs,
                            "||" => condition || rhs,
                            _ => unreachable!(),
                        };
                        let expected = 17 + conditions * u64::from(value);
                        let inputs = (condition, rhs).into_c_stack_list();
                        assert_eq!(prepared.call(&inputs).unwrap(), expected);
                        crate::owned_value::ALLOCATIONS.with(|count| count.set(0));
                        for _ in 0..100 {
                            assert_eq!(prepared.call(&inputs).unwrap(), expected);
                        }
                        let allocations = crate::owned_value::ALLOCATIONS.with(Cell::get);
                        assert_eq!(allocations, 0, "{kind}, {conditions} conditions");
                    }
                }
            }
        }
    }

    /// Checks that scalar reads and imported constants need no temporary owners.
    #[test]
    fn scalar_cloning_avoids_temporary_allocations() {
        for reads in [1, 10, 100] {
            for external_constant in [false, true] {
                let mut body = DynSegment::new::<()>();
                for read in 0..reads {
                    if external_constant {
                        body.just_external(ValueType::cloneable_leaf::<i32>(), Box::new(1i32))
                            .unwrap();
                    } else {
                        body.push_arg::<i32>(0);
                    }
                    if read != 0 {
                        body.op2r(|a: i32, b: i32| {
                            a.checked_add(b).ok_or_else(|| anyhow!("overflow"))
                        })
                        .unwrap();
                    }
                }
                let mut prepared = body.prepare::<Cons<Leaf<i32>, Nil>, Leaf<i32>>().unwrap();
                let input = (1i32,).into_c_stack_list();
                assert_eq!(prepared.call(&input).unwrap(), reads);
                crate::owned_value::ALLOCATIONS.with(|count| count.set(0));
                for _ in 0..100 {
                    assert_eq!(prepared.call(&input).unwrap(), reads);
                }
                let allocations = crate::owned_value::ALLOCATIONS.with(Cell::get);
                eprintln!(
                    "{reads} reads, constant={external_constant}: {allocations} temporary allocations"
                );
                assert_eq!(allocations, 0);
            }
        }
    }

    /// Checks linear descriptor work on every checked call and none after preparation.
    #[test]
    fn repeated_read_preflight_is_linear() {
        for reads in [1_000, 2_000, 4_000] {
            let mut body = DynSegment::new::<()>();
            body.push_arg::<i32>(0);
            for _ in 1..reads {
                body.push_arg::<i32>(0);
                body.op2r(|a: i32, b: i32| a.checked_add(b).ok_or_else(|| anyhow!("overflow")))
                    .unwrap();
            }
            for _ in 0..2 {
                crate::value_type::SHAPE_COMPARISONS.with(|count| count.set(0));
                assert_eq!(body.call_dyn::<i32>(&[&1i32]).unwrap(), reads);
                let comparisons = crate::value_type::SHAPE_COMPARISONS.with(Cell::get);
                eprintln!("{reads} checked reads: {comparisons} descriptor comparisons");
                assert!(comparisons <= 4 * reads as usize + 16);
                assert!(body.call_dyn::<i32>(&[]).is_err());
                assert!(body.call_dyn::<i32>(&[&1u32]).is_err());
            }
            let mut prepared = body.prepare::<Cons<Leaf<i32>, Nil>, Leaf<i32>>().unwrap();
            crate::value_type::SHAPE_COMPARISONS.with(|count| count.set(0));
            assert_eq!(prepared.call(&(1i32,).into_c_stack_list()).unwrap(), reads);
            crate::value_type::SHAPE_COMPARISONS.with(|count| assert_eq!(count.get(), 0));
        }
    }
}
