//! Binds canonical host operations to the existing raw interpreter.
//!
//! This private emission layer validates complete representations before
//! registering monomorphized operations. It does not execute segments itself.
//! Build an `OperationPlan` from canonical ambient/operand metadata, then use
//! `emit0` through `emit3`, `emit_vec`, or `emit_slice` to register the host
//! operation on the existing `RawSegment`. Its unsafe execution must receive
//! the exact live frame described by the plan.

use crate::container_execution::stack_position;
use crate::owned_value::drop_raw;
use crate::value_type::{ArrayOf, ValueRepr, ValueType};
use crate::{RawSegment, RawStack};

/// Binds one canonical value to an absolute stack position and its ambient length.
struct Slot {
    before: usize,
    offset: usize,
    value_type: ValueType,
}

impl Slot {
    /// Checks the aligned offset and returns the following stack length.
    ///
    /// # Errors
    /// Rejects an aligned position or end outside allocation bounds.
    fn new(before: usize, value_type: ValueType) -> anyhow::Result<(Self, usize)> {
        let (offset, end) = stack_position(before, value_type.layout())?;
        Ok((
            Self {
                before,
                offset,
                value_type,
            },
            end,
        ))
    }

    /// Moves the last operand into its already validated concrete representation.
    ///
    /// # Safety
    /// The stack must end with this slot's live value; `T` must have been validated
    /// against its complete descriptor. The caller assumes its sole ownership.
    /// Reading a mismatched representation or retaining another owner is undefined
    /// behavior.
    /// - Complexity: O(s) representation copying for s bytes.
    unsafe fn take<T>(&self, stack: &mut RawStack) -> T {
        // SAFETY: Only validated, monomorphized emission calls this private helper,
        // in reverse operand order, before executing any user callback.
        unsafe {
            let value = stack.read_at(self.offset, |pointer| pointer.cast::<T>().read());
            stack.truncate_to(self.before, false);
            value
        }
    }
}

/// Binds complete signatures and ambient destruction before emitting raw operations.
///
/// Operands are source ordered. The opaque byte prefix preceding `origin` belongs
/// to the caller and is never destroyed. On success ambient values survive; on
/// host error or unwind they are destroyed in reverse stack order. Host operands
/// are transferred to concrete Rust owners before executing user code.
pub(crate) struct OperationPlan {
    origin: usize,
    ambient: Vec<Slot>,
    operands: Vec<Slot>,
    result: ValueType,
}

impl OperationPlan {
    /// Checks all stack positions and the result's ambient-relative layout.
    ///
    /// # Errors
    /// Rejects alignment or size overflow, including the result position.
    /// - Complexity: O(n) time and space for n ambient values and operands.
    pub(crate) fn new(
        origin: usize,
        ambient: &[ValueType],
        operands: &[ValueType],
        result: ValueType,
    ) -> anyhow::Result<Self> {
        let mut end = origin;
        let mut slots = |values: &[ValueType]| -> anyhow::Result<Vec<Slot>> {
            values
                .iter()
                .map(|value_type| {
                    let (slot, next) = Slot::new(end, value_type.clone())?;
                    end = next;
                    Ok(slot)
                })
                .collect()
        };
        let ambient = slots(ambient)?;
        let ambient_end = end;
        let operands = operands
            .iter()
            .map(|value_type| {
                let (slot, next) = Slot::new(end, value_type.clone())?;
                end = next;
                Ok(slot)
            })
            .collect::<anyhow::Result<_>>()?;
        stack_position(ambient_end, result.layout())?;
        Ok(Self {
            origin,
            ambient,
            operands,
            result,
        })
    }

    /// Validates arity and every complete operand/result witness before registration.
    ///
    /// # Errors
    /// Rejects any differing recursive semantic identity, layout, or operand count.
    /// - Complexity: O(n) comparisons for n descriptor nodes.
    fn validate<R: ValueRepr>(&self, operands: &[ValueType]) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.operands.len() == operands.len(),
            "canonical operation operand count mismatch"
        );
        for (index, (slot, expected)) in self.operands.iter().zip(operands).enumerate() {
            anyhow::ensure!(
                slot.value_type.same_shape(expected),
                "canonical operation operand {index} representation mismatch"
            );
        }
        anyhow::ensure!(
            self.result.same_shape(&R::value_type()),
            "canonical operation result representation mismatch"
        );
        Ok(())
    }

    /// Registers one monomorphized host boundary with frozen ambient cleanup.
    ///
    /// # Safety
    /// The signature must already have been validated against `R` and all concrete
    /// types used by `invoke`. The callback must first take every operand in reverse
    /// order without intervening user code, then invoke the host function. The raw
    /// interpreter caller must supply exactly this plan's live input frame.
    ///
    /// - Complexity: O(n) registration alignment checks for n frame values;
    ///   execution is the host cost plus O(a) ambient cleanup on failure.
    unsafe fn emit<R: ValueRepr, F>(self, segment: &mut RawSegment, invoke: F)
    where
        F: Fn(&Self, &mut RawStack) -> anyhow::Result<R::Value> + 'static,
    {
        for slot in self.ambient.iter().chain(&self.operands) {
            segment.update_base_alignment(slot.value_type.layout().align());
        }
        segment.update_base_alignment(self.result.layout().align());
        segment.raw0_(move |stack| {
            let mut cleanup = AmbientCleanup {
                stack,
                slots: &self.ambient,
                remaining: self.ambient.len(),
                origin: self.origin,
                armed: true,
            };
            match invoke(&self, cleanup.stack) {
                Ok(result) => {
                    cleanup.stack.push(result);
                    cleanup.armed = false;
                    Ok(())
                }
                Err(error) => {
                    // Run outside Drop so an ambient destructor panic resumes
                    // cleanup through the still-live guard.
                    cleanup.destroy();
                    Err(error)
                }
            }
        });
    }

    /// Validates and emits a fallible nullary canonical result.
    ///
    /// # Errors
    /// Rejects a nonempty operand list or a mismatched result representation.
    /// - Complexity: O(n) validation for n result/ambient descriptor nodes.
    pub(crate) fn emit0<R: ValueRepr, F>(
        self,
        segment: &mut RawSegment,
        function: F,
    ) -> anyhow::Result<()>
    where
        F: Fn() -> anyhow::Result<R::Value> + 'static,
    {
        self.validate::<R>(&[])?;
        // SAFETY: The full signature matches, with no operands to consume.
        unsafe { self.emit::<R, _>(segment, move |_, _| function()) };
        Ok(())
    }

    /// Validates and emits a fallible unary canonical host operation.
    ///
    /// # Errors
    /// Rejects a different arity or complete operand/result representation.
    /// - Complexity: O(n) validation for n signature/ambient descriptor nodes.
    pub(crate) fn emit1<A: ValueRepr, R: ValueRepr, F>(
        self,
        segment: &mut RawSegment,
        function: F,
    ) -> anyhow::Result<()>
    where
        F: Fn(A::Value) -> anyhow::Result<R::Value> + 'static,
    {
        self.validate::<R>(&[A::value_type()])?;
        // SAFETY: The signature matches; take precedes every user callback.
        unsafe {
            self.emit::<R, _>(segment, move |plan, stack| {
                let a = plan.operands[0].take::<A::Value>(stack);
                function(a)
            });
        }
        Ok(())
    }

    /// Validates and emits a fallible binary operation in source operand order.
    ///
    /// # Errors
    /// Rejects a different arity or complete operand/result representation.
    /// - Complexity: O(n) validation for n signature/ambient descriptor nodes.
    pub(crate) fn emit2<A: ValueRepr, B: ValueRepr, R: ValueRepr, F>(
        self,
        segment: &mut RawSegment,
        function: F,
    ) -> anyhow::Result<()>
    where
        F: Fn(A::Value, B::Value) -> anyhow::Result<R::Value> + 'static,
    {
        self.validate::<R>(&[A::value_type(), B::value_type()])?;
        // SAFETY: All witnesses match, and operands are taken top-first.
        unsafe {
            self.emit::<R, _>(segment, move |plan, stack| {
                let b = plan.operands[1].take::<B::Value>(stack);
                let a = plan.operands[0].take::<A::Value>(stack);
                function(a, b)
            });
        }
        Ok(())
    }

    /// Validates and emits a fallible ternary operation in source operand order.
    ///
    /// # Errors
    /// Rejects a different arity or complete operand/result representation.
    /// - Complexity: O(n) validation for n signature/ambient descriptor nodes.
    pub(crate) fn emit3<A: ValueRepr, B: ValueRepr, C: ValueRepr, R: ValueRepr, F>(
        self,
        segment: &mut RawSegment,
        function: F,
    ) -> anyhow::Result<()>
    where
        F: Fn(A::Value, B::Value, C::Value) -> anyhow::Result<R::Value> + 'static,
    {
        self.validate::<R>(&[A::value_type(), B::value_type(), C::value_type()])?;
        // SAFETY: All witnesses match, and operands are taken top-first.
        unsafe {
            self.emit::<R, _>(segment, move |plan, stack| {
                let c = plan.operands[2].take::<C::Value>(stack);
                let b = plan.operands[1].take::<B::Value>(stack);
                let a = plan.operands[0].take::<A::Value>(stack);
                function(a, b, c)
            });
        }
        Ok(())
    }

    /// Transfers a canonical array's allocation to a vector-taking host operation.
    ///
    /// No element allocation, cloning, checked conversion, or descriptor work occurs
    /// at execution. The host receives the original pointer, length, and capacity.
    ///
    /// # Errors
    /// Rejects a different arity or complete array-element/result representation.
    /// - Complexity: O(n) validation for n signature/ambient descriptor nodes.
    pub(crate) fn emit_vec<E: ValueRepr, R: ValueRepr, F>(
        self,
        segment: &mut RawSegment,
        function: F,
    ) -> anyhow::Result<()>
    where
        F: Fn(Vec<E::Value>) -> anyhow::Result<R::Value> + 'static,
    {
        self.emit1::<ArrayOf<E>, R, _>(segment, move |array| function(array.into_vec()))
    }

    /// Borrows a consumed canonical array's original elements during a host call.
    ///
    /// The array stays owned locally and is destroyed after the host returns or
    /// unwinds. An owned `'static` result cannot borrow from this frame.
    ///
    /// # Errors
    /// Rejects a different arity or complete array-element/result representation.
    /// - Complexity: O(n) validation for n signature/ambient descriptor nodes;
    ///   execution adds O(e) destruction for e elements, excluding the host cost.
    pub(crate) fn emit_slice<E: ValueRepr, R: ValueRepr, F>(
        self,
        segment: &mut RawSegment,
        function: F,
    ) -> anyhow::Result<()>
    where
        F: Fn(&[E::Value]) -> anyhow::Result<R::Value> + 'static,
    {
        self.emit1::<ArrayOf<E>, R, _>(segment, move |array| function(array.as_slice()))
    }
}

/// Destroys only the still-owned ambient prefix when a host operation fails.
///
/// The current slot is marked consumed before its destructor runs, allowing
/// ordinary-error cleanup to resume through Drop if a destructor unwinds.
struct AmbientCleanup<'a> {
    stack: &'a mut RawStack,
    slots: &'a [Slot],
    remaining: usize,
    origin: usize,
    armed: bool,
}

impl AmbientCleanup<'_> {
    /// Destroys the remaining ambient owners in reverse order and restores origin.
    ///
    /// - Complexity: O(n) destruction for n remaining values, excluding nested costs.
    fn destroy(&mut self) {
        if !self.armed {
            return;
        }
        while self.remaining != 0 {
            self.remaining -= 1;
            let slot = &self.slots[self.remaining];
            // SAFETY: Operands have been moved into typed owners; each slot is live
            // until removed from this guard. The precomputed offset matches its plan.
            unsafe {
                self.stack
                    .drop_at(slot.offset, |pointer| drop_raw(&slot.value_type, pointer));
                self.stack.truncate_to(slot.before, false);
            }
        }
        // SAFETY: Every owned value above origin has been moved or destroyed.
        unsafe { self.stack.truncate_to(self.origin, false) };
        self.armed = false;
    }
}

impl Drop for AmbientCleanup<'_> {
    /// Cleans ambient values on host panic, error, or a failed result push.
    fn drop(&mut self) {
        self.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value_type::{ArrayOf, Cons, Leaf, Nil, ValueRepr, ValueType};
    use crate::{Array, IntoCStackList, IntoRustTuple, RawSegment, RawStack};
    use std::cell::{Cell, RefCell};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::rc::Rc;

    /// Records observable construction and destruction in logical element order.
    struct Tracked {
        name: &'static str,
        events: Rc<RefCell<Vec<String>>>,
    }

    impl Tracked {
        /// Constructs one owner and records its name.
        ///
        /// - Complexity: O(n) for n bytes in the name.
        fn new(name: &'static str, events: &Rc<RefCell<Vec<String>>>) -> Self {
            events.borrow_mut().push(format!("new {name}"));
            Self {
                name,
                events: events.clone(),
            }
        }
    }

    impl Drop for Tracked {
        /// Records the unique destruction of this owner.
        ///
        /// - Complexity: O(n) for n bytes in the name.
        fn drop(&mut self) {
            self.events.borrow_mut().push(format!("drop {}", self.name));
        }
    }

    /// Binds a full descriptor signature with a caller-owned prefix.
    ///
    /// - Complexity: O(n) for n signature and ambient descriptor nodes.
    fn plan<A: ValueRepr, R: ValueRepr>(origin: usize, ambient: &[ValueType]) -> OperationPlan {
        OperationPlan::new(origin, ambient, &[A::value_type()], R::value_type()).unwrap()
    }

    /// Constructs a nested array whose destruction order is visible.
    fn nested(events: &Rc<RefCell<Vec<String>>>) -> Array<Array<Tracked>> {
        Array::from_vec(vec![
            Array::from_vec(vec![Tracked::new("a", events), Tracked::new("b", events)]),
            Array::from_vec(vec![Tracked::new("c", events)]),
        ])
    }

    /// Rejects equal-layout but different recursive operand and result identities.
    #[test]
    fn validates_complete_signatures_before_emission() {
        type Input = ArrayOf<Leaf<i32>>;
        type Wrong = ArrayOf<Leaf<u32>>;
        let mut segment = RawSegment::new();
        assert!(
            plan::<Wrong, Leaf<usize>>(0, &[])
                .emit1::<Input, Leaf<usize>, _>(&mut segment, |a| Ok(a.len()))
                .is_err()
        );
        assert!(
            plan::<Input, Leaf<u32>>(0, &[])
                .emit1::<Input, Leaf<i32>, _>(&mut segment, |_| Ok(3))
                .is_err()
        );
        assert!(
            plan::<Input, Wrong>(0, &[])
                .emit1::<Input, Input, _>(&mut segment, Ok)
                .is_err()
        );
        assert!(
            plan::<Nil, Nil>(0, &[])
                .emit1::<Leaf<()>, Nil, _>(&mut segment, |()| Ok(().into_c_stack_list()))
                .is_err()
        );
        let mismatched_arity = OperationPlan::new(0, &[], &[], ValueType::leaf::<usize>()).unwrap();
        assert!(
            mismatched_arity
                .emit1::<Input, Leaf<usize>, _>(&mut segment, |a| Ok(a.len()))
                .is_err()
        );
        // Rejection must not emit an operation that reads absent operands.
        let mut stack = RawStack::with_base_alignment(8);
        unsafe { segment.call0_stack(&mut stack).unwrap() };
        assert_eq!(stack.len(), 0);
    }

    /// Supports unit and tuple elements without legacy wrappers or leaf-only casts.
    #[test]
    fn transfers_canonical_tuple_arrays_to_concrete_vectors() {
        type Pair = Cons<Leaf<i32>, Cons<Leaf<f64>, Nil>>;
        type Pairs = ArrayOf<Pair>;
        let mut segment = RawSegment::new();
        plan::<Pairs, Leaf<Vec<(i32, f64)>>>(0, &[])
            .emit_vec::<Pair, Leaf<Vec<(i32, f64)>>, _>(&mut segment, |rows| {
                Ok(rows.into_iter().map(|row| row.into_rust_tuple()).collect())
            })
            .unwrap();
        let mut stack = RawStack::with_base_alignment(segment.base_alignment());
        stack.push(Array::from_vec(vec![
            (1i32, 2.0f64).into_c_stack_list(),
            (3i32, 4.0f64).into_c_stack_list(),
        ]));
        unsafe { segment.call0_stack(&mut stack).unwrap() };
        let result: Vec<(i32, f64)> = unsafe { stack.pop(false) };
        assert_eq!(result, [(1, 2.0), (3, 4.0)]);
        assert_eq!(stack.len(), 0);
    }

    /// Preserves allocation identity while borrowing and destroys the consumed owner afterward.
    #[test]
    fn slice_borrows_original_pointer_then_drops_owner() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let values = Array::from_vec(vec![Tracked::new("a", &events), Tracked::new("b", &events)]);
        let pointer = values.as_slice().as_ptr();
        let observed = events.clone();
        let mut segment = RawSegment::new();
        plan::<ArrayOf<Leaf<Tracked>>, Leaf<usize>>(0, &[])
            .emit_slice::<Leaf<Tracked>, Leaf<usize>, _>(&mut segment, move |slice| {
                assert_eq!(slice.as_ptr(), pointer);
                assert_eq!(observed.borrow().len(), 2);
                observed.borrow_mut().push("host".into());
                Ok(slice.len())
            })
            .unwrap();
        let mut stack = RawStack::with_base_alignment(segment.base_alignment());
        stack.push(values);
        unsafe { segment.call0_stack(&mut stack).unwrap() };
        assert_eq!(unsafe { stack.pop::<usize>(false) }, 2);
        assert_eq!(
            &*events.borrow(),
            &["new a", "new b", "host", "drop b", "drop a"]
        );
    }

    /// Cleans nested operands and ambient owners exactly once on errors and panics.
    #[test]
    fn nested_failure_cleans_ambient_but_preserves_origin() {
        for panic in [false, true] {
            let events = Rc::new(RefCell::new(Vec::new()));
            let mut segment = RawSegment::new();
            plan::<ArrayOf<ArrayOf<Leaf<Tracked>>>, Nil>(1, &[ValueType::leaf::<Tracked>()])
                .emit1::<ArrayOf<ArrayOf<Leaf<Tracked>>>, Nil, _>(&mut segment, move |_rows| {
                    if panic {
                        panic!("nested host panic");
                    }
                    Err(anyhow::anyhow!("nested host error").context("outer context"))
                })
                .unwrap();
            let mut stack = RawStack::with_base_alignment(segment.base_alignment());
            stack.push(19u8);
            stack.push(Tracked::new("ambient", &events));
            stack.push(nested(&events));
            let result = catch_unwind(AssertUnwindSafe(|| unsafe {
                segment.call0_stack(&mut stack)
            }));
            if panic {
                assert!(result.is_err());
            } else {
                let error = result.unwrap().unwrap_err();
                assert_eq!(format!("{error:#}"), "outer context: nested host error");
            }
            assert_eq!(stack.len(), 1);
            assert_eq!(unsafe { stack.pop::<u8>(false) }, 19);
            assert_eq!(
                &*events.borrow(),
                &[
                    "new ambient",
                    "new a",
                    "new b",
                    "new c",
                    "drop c",
                    "drop b",
                    "drop a",
                    "drop ambient",
                ]
            );
        }
    }

    /// Leaves successful ambient values live and replaces operands at the right alignment.
    #[test]
    fn success_preserves_ambient_and_source_operand_order() {
        let mut segment = RawSegment::new();
        OperationPlan::new(
            0,
            &[ValueType::leaf::<u8>()],
            &[ValueType::leaf::<i32>(), ValueType::leaf::<f64>()],
            ValueType::leaf::<String>(),
        )
        .unwrap()
        .emit2::<Leaf<i32>, Leaf<f64>, Leaf<String>, _>(&mut segment, |a, b| Ok(format!("{a}/{b}")))
        .unwrap();
        let mut stack = RawStack::with_base_alignment(segment.base_alignment());
        stack.push(7u8);
        stack.push(3i32);
        stack.push(4.5f64);
        unsafe { segment.call0_stack(&mut stack).unwrap() };
        assert_eq!(unsafe { stack.pop::<String>(true) }, "3/4.5");
        assert_eq!(unsafe { stack.pop::<u8>(false) }, 7);
    }

    /// Supports nullary and ternary operations including zero-sized source unit.
    #[test]
    fn nullary_ternary_and_unit_signatures() {
        type Unit = <Nil as ValueRepr>::Value;
        let mut segment = RawSegment::new();
        OperationPlan::new(0, &[], &[], Nil::value_type())
            .unwrap()
            .emit0::<Nil, _>(&mut segment, || Ok(().into_c_stack_list()))
            .unwrap();
        let mut stack = RawStack::with_base_alignment(segment.base_alignment());
        unsafe { segment.call0_stack(&mut stack).unwrap() };
        let _: Unit = unsafe { stack.pop(false) };
        assert_eq!(stack.len(), 0);
        let mut segment = RawSegment::new();
        OperationPlan::new(
            0,
            &[],
            &[
                Nil::value_type(),
                ValueType::leaf::<i32>(),
                Nil::value_type(),
            ],
            ValueType::leaf::<i32>(),
        )
        .unwrap()
        .emit3::<Nil, Leaf<i32>, Nil, Leaf<i32>, _>(&mut segment, |_: Unit, value, _: Unit| {
            Ok(value)
        })
        .unwrap();
        let mut stack = RawStack::with_base_alignment(segment.base_alignment());
        stack.push(().into_c_stack_list());
        stack.push(8i32);
        stack.push(().into_c_stack_list());
        unsafe { segment.call0_stack(&mut stack).unwrap() };
        assert_eq!(unsafe { stack.pop::<i32>(false) }, 8);
    }

    /// Allows repeated independent owned results without cloning the old result.
    #[test]
    fn repeated_results_are_independent() {
        let next = Rc::new(Cell::new(0usize));
        let counter = next.clone();
        let mut segment = RawSegment::new();
        OperationPlan::new(0, &[], &[], ArrayOf::<Leaf<usize>>::value_type())
            .unwrap()
            .emit0::<ArrayOf<Leaf<usize>>, _>(&mut segment, move || {
                let value = counter.get();
                counter.set(value + 1);
                Ok(Array::from_vec(vec![value]))
            })
            .unwrap();
        let mut stack = RawStack::with_base_alignment(segment.base_alignment());
        unsafe { segment.call0_stack(&mut stack).unwrap() };
        let first: Array<usize> = unsafe { stack.pop(false) };
        unsafe { segment.call0_stack(&mut stack).unwrap() };
        let second: Array<usize> = unsafe { stack.pop(false) };
        assert_eq!(first.as_slice(), &[0]);
        assert_eq!(second.as_slice(), &[1]);
        assert_ne!(first.as_slice().as_ptr(), second.as_slice().as_ptr());
    }

    /// Performs no repeated descriptor comparisons or concrete clone operations.
    #[test]
    fn repeated_calls_use_only_frozen_moves() {
        type Rows = ArrayOf<ArrayOf<Leaf<i32>>>;
        let mut segment = RawSegment::new();
        let comparisons = crate::value_type::SHAPE_COMPARISONS.with(Cell::get);
        plan::<Rows, Rows>(0, &[])
            .emit1::<Rows, Rows, _>(&mut segment, Ok)
            .unwrap();
        assert!(crate::value_type::SHAPE_COMPARISONS.with(Cell::get) > comparisons);
        let comparisons = crate::value_type::SHAPE_COMPARISONS.with(Cell::get);
        for _ in 0..3 {
            let input = Array::from_vec(vec![Array::from_vec(vec![11i32, 12])]);
            let pointer = input.as_slice().as_ptr();
            let nested_pointer = input.as_slice()[0].as_slice().as_ptr();
            let mut stack = RawStack::with_base_alignment(segment.base_alignment());
            stack.push(input);
            unsafe { segment.call0_stack(&mut stack).unwrap() };
            let output: <Rows as ValueRepr>::Value = unsafe { stack.pop(false) };
            assert_eq!(output.as_slice().as_ptr(), pointer);
            assert_eq!(output.as_slice()[0].as_slice().as_ptr(), nested_pointer);
            assert_eq!(output.as_slice()[0].as_slice(), &[11, 12]);
        }
        assert_eq!(
            crate::value_type::SHAPE_COMPARISONS.with(Cell::get),
            comparisons
        );
    }

    /// Resumes reverse ambient cleanup when an ordinary-error destructor panics.
    #[test]
    fn ambient_destructor_unwind_resumes_cleanup() {
        /// Panics after recording destruction, leaving its field to Rust drop glue.
        struct PanicDrop(Tracked);
        impl Drop for PanicDrop {
            /// Records the panic before the owned tracked field is destroyed.
            fn drop(&mut self) {
                self.0.events.borrow_mut().push("panic".into());
                panic!("ambient destructor panic");
            }
        }
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut segment = RawSegment::new();
        OperationPlan::new(
            1,
            &[
                ValueType::leaf::<Tracked>(),
                ArrayOf::<ArrayOf<Leaf<Tracked>>>::value_type(),
                ValueType::leaf::<PanicDrop>(),
            ],
            &[],
            Nil::value_type(),
        )
        .unwrap()
        .emit0::<Nil, _>(&mut segment, || Err(anyhow::anyhow!("host error")))
        .unwrap();
        let mut stack = RawStack::with_base_alignment(segment.base_alignment());
        stack.push(37u8);
        stack.push(Tracked::new("ambient", &events));
        stack.push(nested(&events));
        stack.push(PanicDrop(Tracked::new("panicky", &events)));
        assert!(
            catch_unwind(AssertUnwindSafe(|| unsafe {
                segment.call0_stack(&mut stack)
            }))
            .is_err()
        );
        assert_eq!(stack.len(), 1);
        assert_eq!(unsafe { stack.pop::<u8>(false) }, 37);
        assert_eq!(
            &*events.borrow(),
            &[
                "new ambient",
                "new a",
                "new b",
                "new c",
                "new panicky",
                "panic",
                "drop panicky",
                "drop c",
                "drop b",
                "drop a",
                "drop ambient",
            ]
        );
    }

    /// Checks allocation-bound failures before a segment receives any operation.
    #[test]
    fn rejects_unrepresentable_frame_offsets() {
        assert!(OperationPlan::new(usize::MAX, &[], &[], Nil::value_type()).is_err());
        assert!(
            OperationPlan::new(
                isize::MAX as usize,
                &[ValueType::leaf::<u64>()],
                &[],
                Nil::value_type()
            )
            .is_err()
        );
        assert!(
            OperationPlan::new(
                isize::MAX as usize,
                &[],
                &[ValueType::leaf::<u64>()],
                Nil::value_type()
            )
            .is_err()
        );
        assert!(
            OperationPlan::new(isize::MAX as usize, &[], &[], ValueType::leaf::<u64>()).is_err()
        );
    }

    /// Keeps a borrowed slice's owner alive and cleans it on either failure mode.
    #[test]
    fn slice_error_and_panic_drop_consumed_owner() {
        for panic in [false, true] {
            let events = Rc::new(RefCell::new(Vec::new()));
            let values = Array::from_vec(vec![Tracked::new("slice", &events)]);
            let pointer = values.as_slice().as_ptr();
            let observed = events.clone();
            let mut segment = RawSegment::new();
            plan::<ArrayOf<Leaf<Tracked>>, Nil>(0, &[])
                .emit_slice::<Leaf<Tracked>, Nil, _>(&mut segment, move |values| {
                    assert_eq!(values.as_ptr(), pointer);
                    assert_eq!(&*observed.borrow(), &["new slice"]);
                    if panic {
                        panic!("slice host panic");
                    }
                    Err(anyhow::anyhow!("slice host error"))
                })
                .unwrap();
            let mut stack = RawStack::with_base_alignment(segment.base_alignment());
            stack.push(values);
            let result = catch_unwind(AssertUnwindSafe(|| unsafe {
                segment.call0_stack(&mut stack)
            }));
            if panic {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap().unwrap_err().to_string(), "slice host error");
            }
            assert_eq!(stack.len(), 0);
            assert_eq!(&*events.borrow(), &["new slice", "drop slice"]);
        }
    }

    /// Emits canonical nested unit/array values, including typed empty arrays.
    #[test]
    fn nested_unit_and_empty_array_representations() {
        type Item = Cons<Nil, Cons<ArrayOf<Leaf<i32>>, Nil>>;
        type Items = ArrayOf<Item>;
        let mut segment = RawSegment::new();
        plan::<Items, Leaf<Vec<Vec<i32>>>>(0, &[])
            .emit_vec::<Item, Leaf<Vec<Vec<i32>>>, _>(&mut segment, |items| {
                Ok(items
                    .into_iter()
                    .map(|item| {
                        let (_, array) = item.into_rust_tuple();
                        array.into_vec()
                    })
                    .collect())
            })
            .unwrap();
        let mut stack = RawStack::with_base_alignment(segment.base_alignment());
        stack.push(Array::from_vec(vec![
            (().into_c_stack_list(), Array::from_vec(vec![1i32])).into_c_stack_list(),
            (().into_c_stack_list(), Array::from_vec(Vec::<i32>::new())).into_c_stack_list(),
        ]));
        unsafe { segment.call0_stack(&mut stack).unwrap() };
        let result: Vec<Vec<i32>> = unsafe { stack.pop(false) };
        assert_eq!(result, [vec![1], vec![]]);
        stack.push(Array::<<Item as ValueRepr>::Value>::from_vec(Vec::new()));
        unsafe { segment.call0_stack(&mut stack).unwrap() };
        let result: Vec<Vec<i32>> = unsafe { stack.pop(false) };
        assert!(result.is_empty());
    }

    /// Passes three distinct operands in source order without suppressing any operand.
    #[test]
    fn ternary_preserves_source_operand_order() {
        let mut segment = RawSegment::new();
        OperationPlan::new(
            0,
            &[],
            &[
                ValueType::leaf::<i32>(),
                ValueType::leaf::<f64>(),
                ValueType::leaf::<String>(),
            ],
            ValueType::leaf::<String>(),
        )
        .unwrap()
        .emit3::<Leaf<i32>, Leaf<f64>, Leaf<String>, Leaf<String>, _>(&mut segment, |a, b, c| {
            Ok(format!("{a}/{b}/{c}"))
        })
        .unwrap();
        let mut stack = RawStack::with_base_alignment(segment.base_alignment());
        stack.push(1i32);
        stack.push(2.5f64);
        stack.push(String::from("three"));
        unsafe { segment.call0_stack(&mut stack).unwrap() };
        assert_eq!(unsafe { stack.pop::<String>(false) }, "1/2.5/three");
        assert_eq!(stack.len(), 0);
    }

    /// Cleans completed nested elements when evaluating a later element fails.
    #[test]
    fn later_element_failure_cleans_completed_nested_values() {
        for panic in [false, true] {
            let events = Rc::new(RefCell::new(Vec::new()));
            let initialize = events.clone();
            let fail = events.clone();
            let mut segment = RawSegment::new();
            segment.push_op0(move || nested(&initialize));
            OperationPlan::new(
                1,
                &[ArrayOf::<ArrayOf<Leaf<Tracked>>>::value_type()],
                &[],
                ValueType::leaf::<Tracked>(),
            )
            .unwrap()
            .emit0::<Leaf<Tracked>, _>(&mut segment, move || {
                let _partial = Tracked::new("later", &fail);
                if panic {
                    panic!("later element panic");
                }
                Err(anyhow::anyhow!("later element error").context("array construction"))
            })
            .unwrap();
            let mut stack = RawStack::with_base_alignment(segment.base_alignment());
            stack.push(41u8);
            let result = catch_unwind(AssertUnwindSafe(|| unsafe {
                segment.call0_stack(&mut stack)
            }));
            if panic {
                assert!(result.is_err());
            } else {
                assert_eq!(
                    format!("{:#}", result.unwrap().unwrap_err()),
                    "array construction: later element error"
                );
            }
            assert_eq!(stack.len(), 1);
            assert_eq!(unsafe { stack.pop::<u8>(false) }, 41);
            assert_eq!(
                &*events.borrow(),
                &[
                    "new a",
                    "new b",
                    "new c",
                    "new later",
                    "drop later",
                    "drop c",
                    "drop b",
                    "drop a"
                ]
            );
        }
    }
}
