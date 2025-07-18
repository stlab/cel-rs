use crate::c_stack_list::{CNil, CStackList, IntoCStackList};
use crate::list_traits::{List, ListTypeIteratorAdvance, TypeIdIterator};
use crate::memory::align_index;
use crate::raw_segment::RawSegment;
use crate::raw_stack::RawStack;
use crate::{CStackListHeadLimit, CStackListHeadPadded, ReverseList};
use anyhow::Result;
use anyhow::ensure;
use std::any::TypeId;
use std::cmp::max;
pub struct StackInfo {
    pub(crate) stack_id: TypeId,
    stack_unwind: Dropper,
    padded: bool,
}

pub trait ToTypeIdList: List {
    fn to_stack_info_list() -> Vec<StackInfo>;
}

impl ToTypeIdList for CNil<()> {
    fn to_stack_info_list() -> Vec<StackInfo> {
        Vec::new()
    }
}

impl<H: 'static, T: ToTypeIdList + 'static + CStackListHeadLimit> ToTypeIdList
    for CStackList<H, T>
{
    fn to_stack_info_list() -> Vec<StackInfo> {
        let mut list = T::to_stack_info_list();
        list.push(StackInfo {
            stack_id: TypeId::of::<H>(),
            stack_unwind: |stack| unsafe { stack.drop::<H>(Self::HEAD_PADDED) },
            padded: Self::HEAD_PADDED,
        });
        list
    }
}

/// A type-checked wrapper around [`RawSegment`] that maintains a stack of type information
/// to ensure type safety during operation execution.
///
/// [`DynSegment`] tracks the types of values on the stack at compile time and verifies
/// that operations receive arguments of the correct type. This prevents type mismatches
/// that could occur when using [`RawSegment`] directly.
type Dropper = fn(&mut RawStack);
pub struct DynSegment {
    pub(crate) segment: RawSegment,
    pub(crate) argument_ids: Vec<TypeId>,
    pub(crate) stack_ids: Vec<StackInfo>,
    stack_index: usize,
}

impl DynSegment {
    /// Creates a new empty segment with no operations.
    #[must_use]
    pub fn new<Args: IntoCStackList>() -> Self
    where
        ReverseList<Args::Output>: ToTypeIdList,
    {
        let stack_ids = ReverseList::<Args::Output>::to_stack_info_list();
        DynSegment {
            segment: RawSegment::new(),
            argument_ids: stack_ids.iter().map(|s| s.stack_id).collect(),
            stack_ids,
            stack_index: size_of::<ReverseList<Args::Output>>(),
        }
    }

    /// Create a DynSegment that is a fragment of a larger segment, it may
    /// be used to implement conditional execution.
    #[must_use]
    pub fn new_fragment(&self) -> Self {
        DynSegment {
            segment: RawSegment::new(),
            argument_ids: Vec::<TypeId>::new(), // should be optional?
            stack_ids: Vec::<StackInfo>::new(),
            stack_index: self.stack_index,
        }
    }

    /// Verifies that the argument types match the expected types on the type stack.
    ///
    /// Returns an error if the argument types don't match the expected types or if
    /// there are too many arguments.
    ///
    /// To avoid reversing the arguments and reversing the slice, this operation
    /// is done in argument order, not stack order.
    // REVISIT: pop_types should just return the last n padding values
    fn pop_types<L: ListTypeIteratorAdvance<TypeId> + 'static>(&mut self) -> Result<()> {
        ensure!(
            L::LENGTH <= self.stack_ids.len(),
            "too many arguments: expected {}, got {}",
            L::LENGTH,
            self.stack_ids.len()
        );
        let start = self.stack_ids.len() - L::LENGTH;
        ensure!(
            TypeIdIterator::<L>::new().eq(self.stack_ids[start..].iter().map(|info| info.stack_id)),
            "stack type ids do not match"
        );
        self.stack_ids.truncate(start);
        Ok(())
    }

    /// Push type to stack and register dropper.
    fn push_type<T>(&mut self)
    where
        T: 'static,
    {
        let aligned_index = align_index(align_of::<T>(), self.stack_index);
        let padded = aligned_index != self.stack_index;

        self.stack_ids.push(StackInfo {
            stack_id: TypeId::of::<T>(),
            stack_unwind: if padded {
                |stack| unsafe { stack.drop::<T>(true) }
            } else {
                |stack| unsafe { stack.drop::<T>(false) }
            },
            padded,
        });
        self.stack_index = aligned_index + size_of::<T>();
    }

    fn get_last_n_padded<const N: usize>(&self) -> [bool; N] {
        let mut result = [false; N];
        let start = self.stack_ids.len().saturating_sub(N);
        for (i, info) in self.stack_ids[start..].iter().enumerate() {
            result[i] = info.padded;
        }
        result
    }

    /// Pushes a nullary operation that takes no arguments and returns a value of type R.
    ///
    /// The return type is tracked in the type stack for subsequent operations.
    pub fn op0<R, F>(&mut self, op: F)
    where
        F: Fn() -> R + 'static,
        R: 'static,
    {
        self.segment.push_op0(op);
        self.push_type::<R>();
    }

    pub fn op0r<R, F>(&mut self, op: F)
    where
        F: Fn() -> anyhow::Result<R> + 'static,
        R: 'static,
    {
        let unwind: Vec<_> = self
            .stack_ids
            .iter()
            .map(|info| info.stack_unwind)
            .collect();
        self.segment.raw0(move |stack| match op() {
            Ok(r) => Ok(r),
            Err(e) => {
                for dropper in unwind.iter().rev() {
                    dropper(stack);
                }
                Err(e)
            }
        });
        self.push_type::<R>();
    }

    /// Pushes a unary operation that takes one argument of type T and returns a value of type R.
    ///
    /// Verifies that the top of the type stack matches the expected input type T
    /// before adding the operation.
    ///
    /// # Errors
    ///
    /// Returns an error if the argument type doesn't match the expected type.
    pub fn op1<T, R, F>(&mut self, op: F) -> Result<()>
    where
        F: Fn(T) -> R + 'static,
        T: 'static,
        R: 'static,
    {
        let [p0] = self.get_last_n_padded::<1>();
        self.pop_types::<(T, ())>()?;
        self.segment.push_op1(op, p0);
        self.push_type::<R>();
        Ok(())
    }

    /// Pushes a binary operation that takes two arguments of types T and U and returns a value of type R.
    ///
    /// Verifies that the top two types on the type stack match the expected input types U and T
    /// (in that order) before adding the operation.
    ///
    /// # Errors
    ///
    /// Returns an error if the argument types do not match the expected types.
    pub fn op2<T, U, R, F>(&mut self, op: F) -> Result<()>
    where
        F: Fn(T, U) -> R + 'static,
        T: 'static,
        U: 'static,
        R: 'static,
    {
        let [p0, p1] = self.get_last_n_padded::<2>();
        self.pop_types::<(T, (U, ()))>()?;
        self.segment.push_op2(op, p0, p1);
        self.push_type::<R>();
        Ok(())
    }

    /// Pushes a ternary operation that takes three arguments of types T, U, and V and returns a value of type R.
    ///
    /// Verifies that the top three types on the type stack match the expected input types V, U, and T
    /// (in that order) before adding the operation.
    ///
    /// # Errors
    ///
    /// Returns an error if the argument types do not match the expected types.
    pub fn op3<T, U, V, R, F>(&mut self, op: F) -> Result<()>
    where
        F: Fn(T, U, V) -> R + 'static,
        T: 'static,
        U: 'static,
        V: 'static,
        R: 'static,
    {
        let [p0, p1, p2] = self.get_last_n_padded::<3>();
        self.pop_types::<(T, (U, (V, ())))>()?;
        self.segment.push_op3(op, p0, p1, p2);
        self.push_type::<R>();
        Ok(())
    }

    pub fn join2(&mut self, mut fragment_0: DynSegment, fragment_1: DynSegment) -> Result<()> {
        let [p0] = self.get_last_n_padded::<1>();
        self.pop_types::<(bool, ())>()?;

        // fragment results must match and cannot take arguments.
        ensure!(
            fragment_0.argument_ids.is_empty(),
            "fragment 0 cannot take arguments, but has {} argument(s)",
            fragment_0.argument_ids.len()
        );
        ensure!(
            fragment_1.argument_ids.is_empty(),
            "fragment 1 cannot take arguments, but has {} argument(s)",
            fragment_1.argument_ids.len()
        );
        ensure!(
            fragment_0.stack_ids.len() == 1,
            "fragment 0 must have exactly 1 result, but has {}",
            fragment_0.stack_ids.len()
        );
        ensure!(
            fragment_1.stack_ids.len() == 1,
            "fragment 1 must have exactly 1 result, but has {}",
            fragment_1.stack_ids.len()
        );
        ensure!(
            fragment_0.stack_ids[0].stack_id == fragment_1.stack_ids[0].stack_id,
            "fragment result types must match"
        );

        self.stack_ids.push(fragment_0.stack_ids.pop().unwrap());
        self.segment.update_base_alignment(max(
            fragment_0.segment.base_alignment(),
            fragment_1.segment.base_alignment(),
        ));

        let raw_segment_0 = fragment_0.segment;
        let raw_segment_1 = fragment_1.segment;

        /*
           - pass the stack to call0
        */
        self.segment.raw0_(move |stack| {
            let conditional = unsafe { stack.pop(p0) };
            if conditional {
                unsafe {
                    raw_segment_0.call0_stack(stack)?;
                }
            } else {
                unsafe {
                    raw_segment_1.call0_stack(stack)?;
                }
            }
            Ok(())
        });
        Ok(())
    }

    /// Executes all operations in the segment and returns the final result.
    ///
    /// # Returns
    /// - `Ok(R)` if execution succeeds and the final value is of type R
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///   - There are unexpected arguments (expected none)
    ///   - The final type doesn't match R
    ///   - There are remaining values on the stack after getting the result
    ///
    pub fn call0<R>(&mut self) -> Result<R>
    where
        R: 'static,
    {
        if !self.argument_ids.is_empty() {
            return Err(anyhow::anyhow!(
                "expected no arguments, but segment requires {} argument(s)",
                self.argument_ids.len()
            ));
        }
        self.pop_types::<(R, ())>()?;
        if !self.stack_ids.is_empty() {
            return Err(anyhow::anyhow!(
                "{} value(s) left on execution stack",
                self.stack_ids.len()
            ));
        }
        unsafe { self.segment.call0() }
    }

    /// Executes all operations in the segment with one argument and returns the final result.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///   - The number of arguments doesn't match (expected one)
    ///   - The argument type doesn't match the expected type
    ///   - The final type doesn't match R
    ///   - There are remaining values on the stack after getting the result
    ///
    pub fn call1<A, R>(&mut self, arg: A) -> Result<R>
    where
        A: 'static,
        R: 'static,
    {
        if self.argument_ids.len() != 1 {
            return Err(anyhow::anyhow!(
                "expected 1 argument, but segment requires {} argument(s)",
                self.argument_ids.len()
            ));
        }
        if self.argument_ids[0] != TypeId::of::<A>() {
            return Err(anyhow::anyhow!(
                "argument type mismatch: expected {}, got {}",
                std::any::type_name::<A>(),
                std::any::type_name::<A>() // TODO: Need to store type names along with TypeId
            ));
        }
        self.pop_types::<(R, ())>()?;
        if !self.stack_ids.is_empty() {
            return Err(anyhow::anyhow!(
                "{} value(s) left on execution stack",
                self.stack_ids.len()
            ));
        }
        unsafe { self.segment.call1(arg) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct DropCounter(Arc<AtomicUsize>);

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl Clone for DropCounter {
        fn clone(&self) -> Self {
            DropCounter(self.0.clone())
        }
    }

    #[test]
    fn drop_on_error() -> Result<(), anyhow::Error> {
        let mut segment = DynSegment::new::<()>();

        let drop_count = Arc::new(AtomicUsize::new(0));
        let tracker = DropCounter(drop_count.clone());

        segment.op0(move || tracker.clone());
        segment.op0r(|| -> Result<u32> { Err(anyhow::anyhow!("error")) });
        segment.op2(|_: DropCounter, _: u32| 42u32)?;

        assert_eq!(drop_count.load(Ordering::SeqCst), 0); // Nothing dropped yet
        let result = segment.call0::<u32>();
        assert!(matches!(result, Err(e) if e.to_string() == "error"));
        assert_eq!(drop_count.load(Ordering::SeqCst), 1); // The DropCounter from op0 was dropped

        Ok(())
    }

    #[test]
    fn segment_operations() -> Result<(), anyhow::Error> {
        let mut operations = DynSegment::new::<()>();

        operations.op0(|| -> u32 { 30 });
        operations.op0(|| -> u32 { 12 });
        operations.op2(|x: u32, y: u32| -> u32 { x + y })?;
        operations.op0(|| -> u32 { 100 });
        operations.op0(|| -> u32 { 10 });
        operations.op3(|x: u32, y: u32, z: u32| -> u32 { x + y - z })?;
        operations.op1(|x: u32| -> String { format!("result: {}", x) })?;

        let final_result: String = operations.call0()?;
        assert_eq!(final_result, "result: 132");

        Ok(())
    }

    #[test]
    fn segment_with_argument() -> Result<(), anyhow::Error> {
        let mut operations = DynSegment::new::<(u32,)>();

        operations.op0(|| -> u32 { 12 });
        operations.op2(|x: u32, y: u32| -> u32 { x + y })?;
        operations.op0(|| -> u32 { 100 });
        operations.op0(|| -> u32 { 10 });
        operations.op3(|x: u32, y: u32, z: u32| -> u32 { x + y - z })?;
        operations.op1(|x: u32| -> String { format!("result: {}", x) })?;

        let final_result: String = operations.call1(30u32)?;
        assert_eq!(final_result, "result: 132");

        Ok(())
    }

    #[test]
    fn example_conditional_expression() -> Result<(), anyhow::Error> {
        let mut root_segment = DynSegment::new::<()>();
        root_segment.op0(|| true);
        root_segment.op0(|| false);
        root_segment.op2(|x: bool, y: bool| x && y)?;

        let mut segment_1 = root_segment.new_fragment();
        segment_1.op0(|| 42u32);

        let mut segment_2 = root_segment.new_fragment();
        segment_2.op0(|| 2u32);

        root_segment.join2(segment_1, segment_2)?;

        let result = root_segment.call0::<u32>()?;
        println!("Result: {}", result);

        Ok(())
    }
}
