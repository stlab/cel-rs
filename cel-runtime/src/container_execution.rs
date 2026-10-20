//! Prepares canonical ownership moves without changing the public interpreter.
//!
//! Construction binds complete descriptors before touching live storage. Trusted
//! execution uses those frozen layouts; it never infers ownership from markers.
//! Prepare a `ContainerPlan` before evaluating its children, or a `StackFramePlan`
//! before moving an initialized operand region. Use `ArgumentPlan` for borrowed
//! clone adapters and `ResultPlan` to transfer a whole canonical result. Only
//! `TrustedInvocation` bridges to the existing interpreter, after checking that
//! its raw frame uses matching opaque leaf representations.

use std::alloc::Layout;
use std::ptr;

use crate::array::RawArray;
use crate::owned_value::{Allocation, Children, OwnedValue, Prefix, clone_raw};
use crate::value_type::{ValueKind, ValueType};
use crate::{DynClosure, RawStack};

/// Computes a stack value's aligned start and end using checked arithmetic.
///
/// # Errors
/// Rejects alignment/size overflow and byte lengths exceeding allocation bounds.
fn stack_position(ambient: usize, layout: Layout) -> anyhow::Result<(usize, usize)> {
    let padding = ambient.wrapping_neg() & (layout.align() - 1);
    let offset = ambient
        .checked_add(padding)
        .ok_or_else(|| anyhow::anyhow!("frame alignment overflow"))?;
    let end = offset
        .checked_add(layout.size())
        .ok_or_else(|| anyhow::anyhow!("frame size overflow"))?;
    anyhow::ensure!(
        end <= isize::MAX as usize,
        "frame exceeds allocation bounds"
    );
    Ok((offset, end))
}

/// Freezes one canonical tuple or a fixed-length array's payload layout.
///
/// Fields/elements are initialized in source order, independently of their
/// physical offsets. Array payload capacity equals length, except for ZSTs.
pub(crate) struct ContainerPlan {
    value_type: ValueType,
    payload_layout: Layout,
    length: usize,
}

impl ContainerPlan {
    /// Binds a complete canonical tuple's validated offsets and frozen ownership.
    ///
    /// # Errors
    /// Returns an error if `value_type` is not a canonical tuple descriptor.
    pub(crate) fn tuple(value_type: ValueType) -> anyhow::Result<Self> {
        let ValueKind::Tuple(fields) = value_type.kind() else {
            anyhow::bail!("canonical tuple descriptor required");
        };
        Ok(Self {
            payload_layout: value_type.layout(),
            length: fields.len(),
            value_type,
        })
    }

    /// Checks a canonical array's allocation size before evaluating elements.
    ///
    /// # Errors
    /// Returns an error if length times element stride exceeds `Layout` bounds.
    pub(crate) fn array(element: ValueType, length: usize) -> anyhow::Result<Self> {
        let size = element
            .layout()
            .size()
            .checked_mul(length)
            .ok_or_else(|| anyhow::anyhow!("array allocation size overflow"))?;
        let payload_layout = Layout::from_size_align(size, element.layout().align())?;
        Ok(Self {
            value_type: ValueType::array(element),
            payload_layout,
            length,
        })
    }

    /// Returns the source-indexed field plan or homogeneous element plan.
    fn children(&self) -> Children<'_> {
        match self.value_type.kind() {
            ValueKind::Tuple(fields) => Children::Fields(fields),
            ValueKind::Array(element) => Children::Elements(element),
            _ => unreachable!("container plan kind"),
        }
    }

    /// Returns the canonical child offset for one logical source index.
    ///
    /// - Precondition: `index < self.length`.
    fn offset(&self, index: usize) -> usize {
        debug_assert!(index < self.length);
        match self.value_type.kind() {
            ValueKind::Tuple(fields) => fields[index].offset,
            ValueKind::Array(element) => index * element.layout().size(),
            _ => unreachable!("container plan kind"),
        }
    }

    /// Initializes children in source order and adopts their frozen drop plan.
    ///
    /// Completed children receive reverse-order cleanup on error or unwind.
    /// Allocation guards release storage even if child destruction unwinds.
    ///
    /// # Safety
    ///
    /// Each successful callback must initialize exactly the indexed child of
    /// this plan in its supplied aligned, disjoint destination. On error or panic
    /// it must leave that child uninitialized and clean up any partial child.
    /// It must transfer sole destruction responsibility on success, and must
    /// not retain the destination pointer beyond the callback.
    ///
    /// # Errors
    /// Propagates an initializer error after destroying its completed prefix.
    /// - Complexity: O(n) callbacks and payload storage for n children.
    pub(crate) unsafe fn construct(
        &self,
        mut initialize: impl FnMut(usize, *mut u8) -> anyhow::Result<()>,
    ) -> anyhow::Result<OwnedValue> {
        // Allocate every enclosing region before transferring any child owner.
        let header = matches!(self.value_type.kind(), ValueKind::Array(_))
            .then(|| Allocation::new(self.value_type.layout()));
        let allocation = Allocation::new(self.payload_layout);
        let mut prefix = Prefix {
            base: allocation.pointer.as_ptr(),
            children: self.children(),
            initialized: 0,
        };
        for index in 0..self.length {
            if let Err(error) =
                unsafe { initialize(index, allocation.pointer.as_ptr().add(self.offset(index))) }
            {
                // Start cleanup outside Drop so Prefix::drop can resume if a
                // destructor unwinds while processing this ordinary error.
                unsafe { prefix.destroy() };
                return Err(error);
            }
            prefix.initialized += 1;
        }
        if let Some(header) = header {
            let capacity = if self.payload_layout.size() == 0 {
                // Empty non-ZST arrays retain capacity zero.
                match self.value_type.kind() {
                    ValueKind::Array(element) if element.layout().size() == 0 => usize::MAX,
                    _ => 0,
                }
            } else {
                self.length
            };
            unsafe {
                header.pointer.as_ptr().cast::<RawArray>().write(RawArray {
                    ptr: allocation.pointer.cast(),
                    len: self.length,
                    capacity,
                });
            }
            prefix.initialized = 0;
            drop(prefix);
            std::mem::forget(allocation);
            // SAFETY: All children and the descriptor-free header are initialized.
            Ok(unsafe { OwnedValue::from_allocation(header, self.value_type.clone()) })
        } else {
            prefix.initialized = 0;
            drop(prefix);
            // SAFETY: All canonical fields are initialized and uniquely owned.
            Ok(unsafe { OwnedValue::from_allocation(allocation, self.value_type.clone()) })
        }
    }
}

/// Binds source-order raw stack positions to a canonical tuple destination.
///
/// The source can include ambient storage and per-value alignment padding.
/// A disjoint destination avoids in-place permutation clobbering of tail-first
/// fields, including values whose source and destination orders are opposite.
pub(crate) struct StackFramePlan {
    container: ContainerPlan,
    ambient: usize,
    offsets: Vec<usize>,
    end: usize,
}

impl StackFramePlan {
    /// Checks every aligned source offset and the complete frame length.
    ///
    /// # Errors
    /// Returns an error for a non-tuple descriptor or unrepresentable byte offsets.
    /// - Complexity: O(n) time and offset storage for n frame values.
    pub(crate) fn new(ambient: usize, value_type: ValueType) -> anyhow::Result<Self> {
        let container = ContainerPlan::tuple(value_type)?;
        let ValueKind::Tuple(fields) = container.value_type.kind() else {
            unreachable!()
        };
        let mut end = ambient;
        let mut offsets = Vec::with_capacity(fields.len());
        for field in fields {
            let (offset, next) = stack_position(end, field.value_type.layout())?;
            offsets.push(offset);
            end = next;
        }
        anyhow::ensure!(
            end <= isize::MAX as usize,
            "frame exceeds allocation bounds"
        );
        Ok(Self {
            container,
            ambient,
            offsets,
            end,
        })
    }

    /// Moves an initialized source frame and restores its exact ambient length.
    ///
    /// # Safety
    ///
    /// The stack must have exactly this plan's length, offsets, types, and sufficient
    /// base alignment. The caller must surrender destruction of the removed frame.
    /// The ambient prefix remains live and must not contain pointers into the frame.
    ///
    /// - Complexity: O(n + s) time and O(s) destination storage for n fields and s bytes.
    pub(crate) unsafe fn take(&self, stack: &mut RawStack) -> OwnedValue {
        debug_assert_eq!(stack.len(), self.end);
        let ValueKind::Tuple(fields) = self.container.value_type.kind() else {
            unreachable!()
        };
        // No fallible/panicking operation follows the first ownership transfer.
        let result = unsafe {
            self.container.construct(|index, destination| {
                stack.read_at(self.offsets[index], |source| {
                    ptr::copy_nonoverlapping(
                        source,
                        destination,
                        fields[index].value_type.layout().size(),
                    );
                });
                Ok(())
            })
        }
        .expect("raw moves cannot return an error");
        unsafe { stack.truncate_to(self.ambient, false) };
        result
    }
}

/// Freezes cloneable canonical argument types and their physical frame offsets.
pub(crate) struct ArgumentPlan {
    container: ContainerPlan,
}

impl ArgumentPlan {
    /// Borrows source-ordered field pointers from an already validated canonical frame.
    ///
    /// # Safety
    /// The frame must match this plan completely and remain live throughout
    /// `execute`, which must not retain any pointers beyond that call.
    /// - Complexity: O(n) time and pointer storage for n arguments.
    pub(crate) unsafe fn with_frame_arguments<R>(
        &self,
        frame: &OwnedValue,
        execute: impl FnOnce(&[*const u8]) -> R,
    ) -> R {
        let ValueKind::Tuple(fields) = self.container.value_type.kind() else {
            unreachable!()
        };
        let arguments: Vec<_> = fields
            .iter()
            .map(|field| unsafe { frame.as_ptr().add(field.offset) })
            .collect();
        execute(&arguments)
    }

    /// Validates every borrowed argument's clone capability before execution.
    ///
    /// # Errors
    /// Returns an error for non-tuple metadata or any unavailable clone capability.
    pub(crate) fn new(value_type: ValueType) -> anyhow::Result<Self> {
        anyhow::ensure!(
            value_type.can_clone(),
            "argument clone capability unavailable"
        );
        Ok(Self {
            container: ContainerPlan::tuple(value_type)?,
        })
    }

    /// Clones borrowed raw arguments in source order into an independent frame.
    ///
    /// Completed cloned arguments are destroyed if a later clone panics. Inputs
    /// remain borrowed and unchanged, including nested allocations.
    ///
    /// # Safety
    /// Arguments must match this plan's complete types and count, stay live for
    /// the call, and not alias the fresh destination allocation.
    ///
    /// # Errors
    /// Reserved for propagation of construction errors; frozen clones cannot return errors.
    /// - Complexity: O(v) cloning work and storage for v contained values, excluding leaf costs.
    pub(crate) unsafe fn clone_frame(&self, arguments: &[*const u8]) -> anyhow::Result<OwnedValue> {
        debug_assert_eq!(arguments.len(), self.container.length);
        let ValueKind::Tuple(fields) = self.container.value_type.kind() else {
            unreachable!()
        };
        unsafe {
            self.container.construct(|index, destination| {
                clone_raw(&fields[index].value_type, arguments[index], destination);
                Ok(())
            })
        }
    }
}

/// Freezes a single result's stack position relative to ambient storage.
pub(crate) struct ResultPlan {
    value_type: ValueType,
    ambient: usize,
    offset: usize,
    end: usize,
}

impl ResultPlan {
    /// Checks result layout and ambient-relative offset arithmetic.
    ///
    /// # Errors
    /// Rejects unrepresentable stack offsets before execution touches storage.
    pub(crate) fn new(ambient: usize, value_type: ValueType) -> anyhow::Result<Self> {
        let (offset, end) = stack_position(ambient, value_type.layout())?;
        Ok(Self {
            value_type,
            ambient,
            offset,
            end,
        })
    }

    /// Moves a matching owner onto an ambient stack.
    ///
    /// # Safety
    /// The stack must contain exactly `ambient` live bytes and have sufficient
    /// base alignment. The owner must match this plan's complete descriptor.
    /// The caller assumes destruction responsibility for the new stack value.
    ///
    /// - Complexity: O(s) representation copying and amortized stack growth for s bytes.
    pub(crate) unsafe fn push(&self, stack: &mut RawStack, value: &mut OwnedValue) {
        debug_assert_eq!(stack.len(), self.ambient);
        unsafe {
            stack.reserve_and_write(
                self.value_type.layout().align(),
                self.value_type.layout().size(),
                |destination| value.move_into(destination),
            );
        }
    }

    /// Removes a matching live result, leaving ambient storage unchanged.
    ///
    /// # Safety
    /// The stack must contain exactly this plan's frame and a live result of
    /// this complete descriptor at its bound offset, with sufficient alignment.
    /// The caller surrenders destruction responsibility for that stack result.
    ///
    /// - Complexity: O(s) representation copying and owned storage for s bytes.
    pub(crate) unsafe fn take(&self, stack: &mut RawStack) -> OwnedValue {
        debug_assert_eq!(stack.len(), self.end);
        let allocation = Allocation::new(self.value_type.layout());
        unsafe {
            stack.read_at(self.offset, |source| {
                ptr::copy_nonoverlapping(
                    source,
                    allocation.pointer.as_ptr(),
                    self.value_type.layout().size(),
                );
            });
            stack.truncate_to(self.ambient, false);
            OwnedValue::from_allocation(allocation, self.value_type.clone())
        }
    }
}

/// Binds a complete signature to one validated existing closure and raw frame.
///
/// The legacy body uses different structural array/tuple representations.
/// Preparation therefore accepts only opaque leaves at this boundary. Canonical
/// aggregates remain fully supported by construction and frame plans, but are
/// never cast into the legacy aggregate ABI.
pub(crate) struct TrustedInvocation {
    closure: DynClosure,
    signature: ValueType,
    result: ResultPlan,
}

impl TrustedInvocation {
    /// Validates semantic identity and the matching legacy physical frame once.
    ///
    /// # Errors
    /// Rejects missing/mismatched signatures and incompatible structural frames.
    /// - Complexity: O(n) comparisons for n parameters.
    pub(crate) fn new(closure: DynClosure, signature: ValueType) -> anyhow::Result<Self> {
        let ValueKind::Callable { parameters, result } = signature.kind() else {
            anyhow::bail!("complete callable descriptor required");
        };
        let legacy = closure
            .signature()
            .ok_or_else(|| anyhow::anyhow!("closure signature metadata is unavailable"))?;
        anyhow::ensure!(
            parameters.len() == legacy.parameters.len(),
            "closure arity mismatch"
        );
        for (complete, old) in parameters
            .iter()
            .zip(&legacy.parameters)
            .chain(std::iter::once((result.as_ref(), &legacy.result)))
        {
            Self::validate_legacy_leaf(complete, old)?;
        }
        let result = ResultPlan::new(0, result.as_ref().clone())?;
        Ok(Self {
            closure,
            signature,
            result,
        })
    }

    /// Checks that a complete opaque leaf exactly matches legacy raw storage.
    ///
    /// # Errors
    /// Rejects structural metadata even if sizes happen to match.
    fn validate_legacy_leaf(complete: &ValueType, legacy: &crate::ValueType) -> anyhow::Result<()> {
        let ValueKind::Leaf(id) = complete.kind() else {
            anyhow::bail!("canonical structural frame is incompatible with legacy closure body");
        };
        anyhow::ensure!(
            legacy.tuple_elements().is_none()
                && legacy.array_element().is_none()
                && legacy.callable.is_none(),
            "legacy structural frame requires an explicit representation adapter"
        );
        anyhow::ensure!(
            *id == legacy.type_id()
                && complete.layout().size() == legacy.size()
                && complete.layout().align() == legacy.align(),
            "closure leaf representation mismatch"
        );
        Ok(())
    }

    /// Runs the bound body without external signature/conversion checks and moves its result.
    ///
    /// No argument is consumed; each successful call produces an independent owner.
    /// The existing body's raw execution guard restores the enclosing argument
    /// context on success, error, and unwind.
    ///
    /// # Safety
    /// Arguments must be live borrowed pointers matching this plan's full
    /// parameter descriptors, in source order, for the duration of execution.
    /// The bound compiled body must remain unchanged after preparation, including
    /// through any legacy `CallDynFn` dispatcher given mutable body access.
    ///
    /// # Errors
    /// Propagates reentrant borrow and body execution errors without producing a result.
    /// - Complexity: Body-dependent evaluation plus one result representation move.
    pub(crate) unsafe fn invoke(&self, arguments: &[*const u8]) -> anyhow::Result<OwnedValue> {
        let ValueKind::Callable { parameters, .. } = self.signature.kind() else {
            unreachable!()
        };
        debug_assert_eq!(parameters.len(), arguments.len());
        let mut output = unsafe { self.closure.call_raw_trusted(arguments)? };
        Ok(unsafe { self.result.take(&mut output) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value_type::{ArrayOf, AssociatedType, Cons, Leaf, Nil, ValueRepr};
    use crate::{Array, DynClosure, DynSegment, IntoCStackList, IntoRustTuple, RawStack};
    use std::cell::{Cell, RefCell};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::rc::Rc;

    /// Builds canonical tuple metadata in logical source order.
    fn tuple(fields: Vec<ValueType>) -> ValueType {
        ValueType::tuple(
            fields
                .into_iter()
                .map(|value_type| AssociatedType {
                    offset: 0,
                    value_type,
                })
                .collect(),
        )
        .unwrap()
    }

    /// Tracks exactly-once ownership destruction without imposing clone support.
    struct Tracked(Rc<RefCell<Vec<usize>>>, usize);

    impl Drop for Tracked {
        /// Records destruction of this particular owner.
        fn drop(&mut self) {
            self.0.borrow_mut().push(self.1);
        }
    }

    /// Moves nested owners in source order despite reversed physical tuple offsets.
    #[test]
    fn nested_array_tuple_construction_preserves_order_and_allocations() {
        type Row = Cons<Leaf<String>, Cons<ArrayOf<Leaf<u8>>, Nil>>;
        let row_type = Row::value_type();
        let row_plan = ContainerPlan::tuple(row_type.clone()).unwrap();
        let mut fields = [
            OwnedValue::new::<Leaf<String>>("first".into()),
            OwnedValue::new::<ArrayOf<Leaf<u8>>>(Array::from_vec(vec![3, 4])),
        ];
        let pointer = unsafe { &*fields[1].as_ptr().cast::<Array<u8>>() }
            .as_slice()
            .as_ptr();
        let mut order = vec![];
        let row = unsafe {
            row_plan.construct(|index, destination| {
                order.push(index);
                fields[index].move_into(destination);
                Ok(())
            })
        }
        .unwrap();
        assert_eq!(order, [0, 1]);
        let array_plan = ContainerPlan::array(row_type, 1).unwrap();
        let mut row = row;
        let rows = unsafe {
            array_plan.construct(|_, destination| {
                row.move_into(destination);
                Ok(())
            })
        }
        .unwrap()
        .try_into_typed::<ArrayOf<Row>>()
        .unwrap();
        let (text, bytes) = rows.into_vec().pop().unwrap().into_rust_tuple();
        assert_eq!(text, "first");
        assert_eq!(bytes.as_slice(), [3, 4]);
        assert_eq!(bytes.as_slice().as_ptr(), pointer);
    }

    /// Restores an ambient raw stack after moving a source-ordered frame.
    #[test]
    fn raw_frame_moves_preserve_ambient_and_padding() {
        type Fields = Cons<Leaf<u8>, Cons<Leaf<u64>, Cons<Leaf<String>, Nil>>>;
        let mut stack = RawStack::with_base_alignment(64);
        let ambient = stack.push(17_u8);
        let plan = StackFramePlan::new(stack.len(), Fields::value_type()).unwrap();
        stack.push(2_u8);
        stack.push(9_u64);
        stack.push(String::from("owned"));
        let frame = unsafe { plan.take(&mut stack) };
        assert_eq!(stack.len(), 1);
        assert_eq!(unsafe { stack.pop::<u8>(ambient) }, 17);
        let values: (u8, u64, String) = frame.try_into_typed::<Fields>().unwrap().into_rust_tuple();
        assert_eq!(values, (2, 9, "owned".into()));
    }

    /// Keeps aligned ZST owners live even when all source positions coincide.
    #[test]
    fn overaligned_zst_array_and_empty_containers_drop_exact_counts() {
        thread_local! { static DROPS: Cell<usize> = const { Cell::new(0) }; }
        /// Requires alignment independently of its zero byte size.
        #[repr(align(128))]
        struct Zst;
        impl Drop for Zst {
            /// Counts each logical zero-sized owner.
            fn drop(&mut self) {
                DROPS.with(|drops| drops.set(drops.get() + 1));
            }
        }
        let plan = ContainerPlan::array(ValueType::leaf::<Zst>(), 4).unwrap();
        let array = unsafe {
            plan.construct(|_, destination| {
                assert_eq!(destination.addr() % 128, 0);
                destination.cast::<Zst>().write(Zst);
                Ok(())
            })
        }
        .unwrap();
        assert_eq!(DROPS.with(Cell::get), 0);
        drop(array);
        assert_eq!(DROPS.with(Cell::get), 4);
        let empty = ContainerPlan::array(ValueType::leaf::<Zst>(), 0).unwrap();
        drop(unsafe { empty.construct(|_, _| unreachable!()) }.unwrap());
        let unit = ContainerPlan::tuple(Nil::value_type()).unwrap();
        drop(unsafe { unit.construct(|_, _| unreachable!()) }.unwrap());
        assert_eq!(DROPS.with(Cell::get), 4);
    }

    /// Destroys only the completed prefix on both error and unwind.
    #[test]
    fn construction_error_and_panic_destroy_initialized_prefix_backwards() {
        for is_tuple in [false, true] {
            for panic in [false, true] {
                let drops = Rc::new(RefCell::new(vec![]));
                let element = ValueType::leaf::<Tracked>();
                let plan = if is_tuple {
                    ContainerPlan::tuple(tuple(vec![element; 4])).unwrap()
                } else {
                    ContainerPlan::array(element, 4).unwrap()
                };
                let result = catch_unwind(AssertUnwindSafe(|| unsafe {
                    plan.construct(|index, destination| {
                        if index == 2 {
                            if panic {
                                panic!("construction panic");
                            }
                            anyhow::bail!("construction error");
                        }
                        destination
                            .cast::<Tracked>()
                            .write(Tracked(drops.clone(), index));
                        Ok(())
                    })
                }));
                if panic {
                    assert!(result.is_err());
                } else {
                    assert!(result.unwrap().is_err());
                }
                assert_eq!(*drops.borrow(), [1, 0]);
            }
        }
    }

    /// Rejects allocation arithmetic overflow before evaluating any element.
    #[test]
    fn plans_reject_wrong_kinds_and_overflow() {
        assert!(ContainerPlan::tuple(ValueType::leaf::<u8>()).is_err());
        assert!(ContainerPlan::array(ValueType::leaf::<u64>(), usize::MAX).is_err());
        assert!(StackFramePlan::new(usize::MAX, tuple(vec![ValueType::leaf::<u64>()])).is_err());
    }

    /// Clones borrowed arguments into distinct canonical owners on every execution.
    #[test]
    fn borrowed_frame_clones_are_independent_and_source_ordered() {
        type Fields = Cons<ArrayOf<Leaf<String>>, Cons<Leaf<u8>, Nil>>;
        let array = Array::from_vec(vec![String::from("source")]);
        let number = 7_u8;
        let plan = ArgumentPlan::new(tuple(vec![
            ValueType::array(ValueType::cloneable_leaf::<String>()),
            ValueType::cloneable_leaf::<u8>(),
        ]))
        .unwrap();
        let arguments = [(&raw const array).cast(), (&raw const number).cast()];
        let a = unsafe { plan.clone_frame(&arguments) }.unwrap();
        let b = unsafe { plan.clone_frame(&arguments) }.unwrap();
        let (a, n): (Array<String>, u8) = a.try_into_typed::<Fields>().unwrap().into_rust_tuple();
        let (b, _): (Array<String>, u8) = b.try_into_typed::<Fields>().unwrap().into_rust_tuple();
        assert_eq!(n, 7);
        assert_eq!(a.as_slice(), ["source"]);
        assert_ne!(a.as_slice().as_ptr(), b.as_slice().as_ptr());
        assert_ne!(a.as_slice().as_ptr(), array.as_slice().as_ptr());
        assert!(ArgumentPlan::new(tuple(vec![ValueType::leaf::<String>()])).is_err());
    }

    /// Binds a trusted call to its validated carrier and moves repeated owned results.
    #[test]
    fn trusted_closure_calls_return_independent_results() {
        let mut body = DynSegment::new::<()>();
        body.push_arg::<String>(0);
        body.op1(|s: String| format!("{s}!")).unwrap();
        let closure = DynClosure::typed(vec![crate::ValueType::leaf::<String>()], body).unwrap();
        let signature = ValueType::callable(
            vec![ValueType::cloneable_leaf::<String>()],
            ValueType::leaf::<String>(),
        );
        let plan = TrustedInvocation::new(closure, signature).unwrap();
        let input = String::from("input");
        let args = [(&raw const input).cast()];
        let a = unsafe { plan.invoke(&args) }
            .unwrap()
            .try_into_typed::<Leaf<String>>()
            .unwrap();
        let b = unsafe { plan.invoke(&args) }
            .unwrap()
            .try_into_typed::<Leaf<String>>()
            .unwrap();
        assert_eq!(a, "input!");
        assert_eq!(b, a);
        assert_ne!(a.as_ptr(), b.as_ptr());
        assert_eq!(input, "input");
    }

    /// Rejects canonical aggregates before a legacy body can reinterpret their storage.
    #[test]
    fn trusted_preparation_rejects_mismatch_and_legacy_aggregate_frames() {
        let mut body = DynSegment::new::<()>();
        body.push_arg::<String>(0);
        let closure = DynClosure::typed(vec![crate::ValueType::leaf::<String>()], body).unwrap();
        assert!(
            TrustedInvocation::new(
                closure.clone(),
                ValueType::callable(vec![ValueType::leaf::<u64>()], ValueType::leaf::<String>(),)
            )
            .is_err()
        );
        assert!(TrustedInvocation::new(closure, ValueType::leaf::<DynClosure>()).is_err());
        let mut body = DynSegment::new::<()>();
        body.just(1_i32);
        let closure = DynClosure::typed(vec![crate::ValueType::tuple(vec![])], body).unwrap();
        assert!(
            TrustedInvocation::new(
                closure,
                ValueType::callable(vec![Nil::value_type()], ValueType::leaf::<i32>(),)
            )
            .is_err()
        );
        let mut body = DynSegment::new::<()>();
        body.just(1_i32);
        let closure = DynClosure::typed(vec![], body).unwrap();
        assert!(
            TrustedInvocation::new(
                closure.clone(),
                ValueType::callable(vec![ValueType::leaf::<i32>()], ValueType::leaf::<i32>()),
            )
            .is_err()
        );
        assert!(
            TrustedInvocation::new(
                closure.clone(),
                ValueType::callable(vec![], ValueType::array(ValueType::leaf::<i32>())),
            )
            .is_err()
        );
        assert!(
            TrustedInvocation::new(
                closure,
                ValueType::callable(vec![], ValueType::leaf::<u32>()),
            )
            .is_err()
        );
        let mut body = DynSegment::new::<()>();
        body.just(1_i32);
        let untyped = DynClosure::new(vec![], std::any::TypeId::of::<i32>(), body);
        assert!(
            TrustedInvocation::new(
                untyped,
                ValueType::callable(vec![], ValueType::leaf::<i32>()),
            )
            .is_err()
        );
    }

    /// Transfers a canonical nested result through a padded ambient stack without cloning.
    #[test]
    fn result_roundtrip_preserves_nested_owners_and_ambient() {
        type Result = Cons<ArrayOf<Leaf<String>>, Cons<Leaf<u64>, Nil>>;
        let array = Array::from_vec(vec![String::from("retained")]);
        let pointer = array.as_slice().as_ptr();
        let mut result = OwnedValue::new::<Result>((array, 32_u64).into_c_stack_list());
        let mut stack = RawStack::with_base_alignment(64);
        let padding = stack.push(123_u8);
        let plan = ResultPlan::new(stack.len(), Result::value_type()).unwrap();
        unsafe { plan.push(&mut stack, &mut result) };
        drop(result);
        let returned = unsafe { plan.take(&mut stack) };
        assert_eq!(stack.len(), 1);
        let (array, n): (Array<String>, u64) = returned
            .try_into_typed::<Result>()
            .unwrap()
            .into_rust_tuple();
        assert_eq!(array.as_slice().as_ptr(), pointer);
        assert_eq!(n, 32);
        assert_eq!(unsafe { stack.pop::<u8>(padding) }, 123);
        assert!(ResultPlan::new(usize::MAX, ValueType::leaf::<u64>()).is_err());
    }

    /// Clones a canonical borrowed frame and passes its source-ordered pointers to a bound body.
    #[test]
    fn canonical_argument_frame_invokes_compatible_legacy_body() {
        let complete = tuple(vec![
            ValueType::cloneable_leaf::<String>(),
            ValueType::cloneable_leaf::<u8>(),
        ]);
        let arguments = ArgumentPlan::new(complete).unwrap();
        let mut body = DynSegment::new::<()>();
        body.push_arg::<String>(0);
        body.push_arg::<u8>(1);
        body.op2(|s: String, n: u8| format!("{s}{n}")).unwrap();
        let closure = DynClosure::typed(
            vec![
                crate::ValueType::leaf::<String>(),
                crate::ValueType::leaf::<u8>(),
            ],
            body,
        )
        .unwrap();
        let invocation = TrustedInvocation::new(
            closure,
            ValueType::callable(
                vec![ValueType::leaf::<String>(), ValueType::leaf::<u8>()],
                ValueType::leaf::<String>(),
            ),
        )
        .unwrap();
        let s = String::from("argument");
        let n = 5_u8;
        let frame =
            unsafe { arguments.clone_frame(&[(&raw const s).cast(), (&raw const n).cast()]) }
                .unwrap();
        let result =
            unsafe { arguments.with_frame_arguments(&frame, |raw| invocation.invoke(raw)) }
                .unwrap()
                .try_into_typed::<Leaf<String>>()
                .unwrap();
        assert_eq!(result, "argument5");
        assert_eq!(s, "argument");
    }

    /// Cleans nested cloned prefixes and completed sibling arguments when a later leaf panics.
    #[test]
    fn borrowed_clone_panic_cleans_nested_prefixes_and_keeps_originals() {
        /// Counts clones and destructions, panicking before constructing the selected clone.
        struct Cloned {
            id: usize,
            events: Rc<RefCell<Vec<String>>>,
            panic_at: usize,
        }
        impl Clone for Cloned {
            /// Records source-order cloning or panics before returning an owner.
            fn clone(&self) -> Self {
                self.events.borrow_mut().push(format!("clone{}", self.id));
                assert_ne!(self.id, self.panic_at, "clone panic");
                Self {
                    id: self.id,
                    events: self.events.clone(),
                    panic_at: self.panic_at,
                }
            }
        }
        impl Drop for Cloned {
            /// Records ownership destruction.
            fn drop(&mut self) {
                self.events.borrow_mut().push(format!("drop{}", self.id));
            }
        }
        let events = Rc::new(RefCell::new(vec![]));
        let first = Cloned {
            id: 0,
            events: events.clone(),
            panic_at: 2,
        };
        let nested = Array::from_vec(vec![
            Cloned {
                id: 1,
                events: events.clone(),
                panic_at: 2,
            },
            Cloned {
                id: 2,
                events: events.clone(),
                panic_at: 2,
            },
        ]);
        let plan = ArgumentPlan::new(tuple(vec![
            ValueType::cloneable_leaf::<Cloned>(),
            ValueType::array(ValueType::cloneable_leaf::<Cloned>()),
        ]))
        .unwrap();
        assert!(
            catch_unwind(AssertUnwindSafe(|| unsafe {
                plan.clone_frame(&[(&raw const first).cast(), (&raw const nested).cast()])
            }))
            .is_err()
        );
        assert_eq!(
            *events.borrow(),
            ["clone0", "clone1", "clone2", "drop1", "drop0"]
        );
        drop(nested);
        drop(first);
        assert_eq!(
            *events.borrow(),
            [
                "clone0", "clone1", "clone2", "drop1", "drop0", "drop2", "drop1", "drop0",
            ]
        );
    }

    /// Restores an enclosing raw argument frame after inner success, error, and panic.
    #[test]
    fn trusted_nested_calls_restore_frames_on_every_exit() {
        for exit in 0..3 {
            let drops = Rc::new(Cell::new(0));
            /// Counts temporary argument clones destroyed by the body.
            struct Argument(Rc<Cell<usize>>);
            impl Clone for Argument {
                /// Shares the destruction counter without consuming the borrowed input.
                fn clone(&self) -> Self {
                    Self(self.0.clone())
                }
            }
            impl Drop for Argument {
                /// Records each owned instance's destruction.
                fn drop(&mut self) {
                    self.0.set(self.0.get() + 1);
                }
            }
            let mut inner = DynSegment::new::<()>();
            inner.push_arg::<Argument>(0);
            inner
                .op1r(move |arg: Argument| -> anyhow::Result<u8> {
                    drop(arg);
                    match exit {
                        0 => Ok(8),
                        1 => anyhow::bail!("body error"),
                        _ => panic!("body panic"),
                    }
                })
                .unwrap();
            let inner = TrustedInvocation::new(
                DynClosure::typed(vec![crate::ValueType::leaf::<Argument>()], inner).unwrap(),
                ValueType::callable(vec![ValueType::leaf::<Argument>()], ValueType::leaf::<u8>()),
            )
            .unwrap();
            let nested_drops = drops.clone();
            let mut outer = DynSegment::new::<()>();
            outer.op0(move || {
                let argument = Argument(nested_drops.clone());
                let result = catch_unwind(AssertUnwindSafe(|| unsafe {
                    inner.invoke(&[(&raw const argument).cast()])
                }));
                match exit {
                    0 => assert_eq!(
                        result
                            .unwrap()
                            .unwrap()
                            .try_into_typed::<Leaf<u8>>()
                            .unwrap(),
                        8
                    ),
                    1 => assert!(result.unwrap().is_err()),
                    _ => assert!(result.is_err()),
                }
                1_u8
            });
            // Reading after the nested exit must use the outer raw frame again.
            outer.push_arg::<String>(0);
            outer.op2(|_: u8, value: String| value).unwrap();
            let outer = TrustedInvocation::new(
                DynClosure::typed(vec![crate::ValueType::leaf::<String>()], outer).unwrap(),
                ValueType::callable(
                    vec![ValueType::leaf::<String>()],
                    ValueType::leaf::<String>(),
                ),
            )
            .unwrap();
            let value = String::from("outer");
            for _ in 0..2 {
                let result = unsafe { outer.invoke(&[(&raw const value).cast()]) }.unwrap();
                assert_eq!(result.try_into_typed::<Leaf<String>>().unwrap(), "outer");
            }
            assert_eq!(drops.get(), 4);
        }
    }

    /// Resumes prefix cleanup if one completed child's destructor itself panics.
    #[test]
    fn construction_cleanup_resumes_after_destructor_panic() {
        /// Panics only for the designated live child after recording its destruction.
        struct PanickingDrop(Rc<RefCell<Vec<usize>>>, usize);
        impl Drop for PanickingDrop {
            /// Records destruction before the single intentional unwind.
            fn drop(&mut self) {
                self.0.borrow_mut().push(self.1);
                assert_ne!(self.1, 1, "destructor panic");
            }
        }
        for array in [false, true] {
            let events = Rc::new(RefCell::new(vec![]));
            let leaf = ValueType::leaf::<PanickingDrop>();
            let plan = if array {
                ContainerPlan::array(leaf, 3).unwrap()
            } else {
                ContainerPlan::tuple(tuple(vec![leaf; 3])).unwrap()
            };
            assert!(
                catch_unwind(AssertUnwindSafe(|| unsafe {
                    plan.construct(|index, destination| {
                        if index == 2 {
                            anyhow::bail!("stop construction");
                        }
                        destination
                            .cast::<PanickingDrop>()
                            .write(PanickingDrop(events.clone(), index));
                        Ok(())
                    })
                }))
                .is_err()
            );
            assert_eq!(*events.borrow(), [1, 0]);
        }
    }

    /// Retains source evaluation order for homogeneous owners and reverse destruction.
    #[test]
    fn array_evaluation_and_destruction_orders_are_logical() {
        let events = Rc::new(RefCell::new(vec![]));
        let evaluated = RefCell::new(vec![]);
        let plan = ContainerPlan::array(ValueType::leaf::<Tracked>(), 3).unwrap();
        let array = unsafe {
            plan.construct(|index, destination| {
                evaluated.borrow_mut().push(index);
                destination
                    .cast::<Tracked>()
                    .write(Tracked(events.clone(), index));
                Ok(())
            })
        }
        .unwrap();
        assert_eq!(*evaluated.borrow(), [0, 1, 2]);
        assert!(events.borrow().is_empty());
        drop(array);
        assert_eq!(*events.borrow(), [2, 1, 0]);
    }

    /// Moves overlapping-address ZST fields without losing logical ownership or alignment.
    #[test]
    fn aligned_zst_tuple_frame_and_result_restore_ambient() {
        thread_local! { static DROPS: Cell<usize> = const { Cell::new(0) }; }
        /// Has distinct logical ownership but no payload bytes.
        #[repr(align(128))]
        struct Zst;
        impl Drop for Zst {
            /// Counts logical destruction even at coincident field addresses.
            fn drop(&mut self) {
                DROPS.with(|n| n.set(n.get() + 1));
            }
        }
        type Fields = Cons<Leaf<Zst>, Cons<Leaf<u8>, Cons<Leaf<Zst>, Nil>>>;
        let mut stack = RawStack::with_base_alignment(128);
        let ambient = stack.push(6_u8);
        let frame_plan = StackFramePlan::new(stack.len(), Fields::value_type()).unwrap();
        stack.push(Zst);
        stack.push(7_u8);
        stack.push(Zst);
        let frame = unsafe { frame_plan.take(&mut stack) };
        assert_eq!(stack.len(), 1);
        assert_eq!(frame.as_ptr().addr() % 128, 0);
        drop(frame);
        assert_eq!(DROPS.with(Cell::get), 2);
        let result_plan = ResultPlan::new(stack.len(), ValueType::leaf::<Zst>()).unwrap();
        let mut result = OwnedValue::new::<Leaf<Zst>>(Zst);
        unsafe { result_plan.push(&mut stack, &mut result) };
        drop(result);
        assert_eq!(DROPS.with(Cell::get), 2);
        drop(unsafe { result_plan.take(&mut stack) });
        assert_eq!(DROPS.with(Cell::get), 3);
        assert_eq!(stack.len(), 1);
        assert_eq!(unsafe { stack.pop::<u8>(ambient) }, 6);
    }

    /// Returns a borrow error for recursion without poisoning subsequent independent calls.
    #[test]
    fn trusted_reentrancy_reports_error_and_releases_body_borrow() {
        let slot = Rc::new(RefCell::new(None::<TrustedInvocation>));
        let nested = Rc::downgrade(&slot);
        let mut body = DynSegment::new::<()>();
        body.op0(move || {
            let slot = nested.upgrade().unwrap();
            let plan = slot.borrow();
            assert!(unsafe { plan.as_ref().unwrap().invoke(&[]) }.is_err());
            42_u8
        });
        let closure = DynClosure::typed(vec![], body).unwrap();
        *slot.borrow_mut() = Some(
            TrustedInvocation::new(
                closure,
                ValueType::callable(vec![], ValueType::leaf::<u8>()),
            )
            .unwrap(),
        );
        for _ in 0..2 {
            let plan = slot.borrow();
            let result = unsafe { plan.as_ref().unwrap().invoke(&[]) }.unwrap();
            assert_eq!(result.try_into_typed::<Leaf<u8>>().unwrap(), 42);
        }
    }

    /// Restores checked host arguments after a nested trusted raw call returns.
    #[test]
    fn trusted_call_preserves_enclosing_checked_argument_context() {
        let mut body = DynSegment::new::<()>();
        body.push_arg::<String>(0);
        let plan = TrustedInvocation::new(
            DynClosure::typed(vec![crate::ValueType::leaf::<String>()], body).unwrap(),
            ValueType::callable(
                vec![ValueType::leaf::<String>()],
                ValueType::leaf::<String>(),
            ),
        )
        .unwrap();
        let mut enclosing = DynSegment::new::<()>();
        enclosing.op0(move || {
            let inner = String::from("inner");
            let result = unsafe { plan.invoke(&[(&raw const inner).cast()]) }.unwrap();
            assert_eq!(result.try_into_typed::<Leaf<String>>().unwrap(), "inner");
            1_u8
        });
        enclosing.push_arg::<String>(0);
        enclosing.op2(|_: u8, outer: String| outer).unwrap();
        for _ in 0..2 {
            let outer = String::from("outer");
            assert_eq!(enclosing.call_dyn::<String>(&[&outer]).unwrap(), outer);
        }
    }
}
