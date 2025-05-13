use crate::c_stack_list::*;
use crate::memory::align_index;
use crate::raw_segment::RawSegment;
use crate::raw_stack::RawStack;
use crate::type_list::*;
use anyhow::Result;
use anyhow::ensure;
use std::any::TypeId;
pub struct StackInfo {
    pub(crate) stack_id: TypeId,
    stack_unwind: Dropper,
    padded: bool,
}

pub trait ToTypeIDList: List {
    fn to_stack_info_list() -> Vec<StackInfo>;
}

impl ToTypeIDList for CNil<()> {
    fn to_stack_info_list() -> Vec<StackInfo> {
        Vec::new()
    }
}

impl<T: 'static, U: ToTypeIDList + 'static> ToTypeIDList for CStackList<T, U> {
    fn to_stack_info_list() -> Vec<StackInfo> {
        let mut list = U::to_stack_info_list();
        list.push(StackInfo {
            stack_id: TypeId::of::<T>(),
            stack_unwind: |stack| unsafe { stack.drop::<T>(Self::HEAD_PADDING != 0) },
            padded: Self::HEAD_PADDING != 0,
        });
        list
    }
}

/// A type-checked wrapper around RawSegmentAlignedStack that maintains a stack of type information
/// to ensure type safety during operation execution.
///
/// DynSegment tracks the types of values on the stack at compile time and verifies
/// that operations receive arguments of the correct type. This prevents type mismatches
/// that could occur when using RawSegment directly.
type Dropper = fn(&mut RawStack);
pub struct DynSegment {
    pub(crate) segment: RawSegment,
    pub(crate) argument_ids: Vec<TypeId>,
    pub(crate) stack_ids: Vec<StackInfo>,
    stack_index: usize,
}

impl DynSegment {
    /// Creates a new empty segment with no operations.
    pub fn new<Args: IntoCStackList>() -> Self
    where
        <Args::Output as List>::Reverse: ToTypeIDList,
    {
        let stack_ids = <Args::Output as List>::Reverse::to_stack_info_list();
        DynSegment {
            segment: RawSegment::new(),
            argument_ids: stack_ids.iter().map(|s| s.stack_id).collect(),
            stack_ids,
            stack_index: size_of::<<Args::Output as List>::Reverse>(),
        }
    }

    /// Verifies that the argument types match the expected types on the type stack.
    ///
    /// Returns an error if the argument types don't match the expected types or if
    /// there are too many arguments.
    ///
    /// To avoid reversing the arguments and reversing the slice, this operation
    /// is done in argument order, not stack order.
    fn pop_types<L: ListTypeIteratorAdvance<TypeId> + 'static>(&mut self) -> Result<()> {
        ensure!(
            L::LENGTH <= self.stack_ids.len(),
            "Too many arguments: expected {}, got {}",
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
            stack_unwind: match padded {
                true => |stack| unsafe { stack.drop::<T>(true) },
                false => |stack| unsafe { stack.drop::<T>(false) },
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

    /// Executes all operations in the segment and returns the final result.
    ///
    /// # Returns
    /// - `Ok(R)` if execution succeeds and the final value is of type R
    /// - `Err` if:
    ///   - There are unexpected arguments (expected none)
    ///   - The final type doesn't match R
    ///   - There are remaining values on the stack after getting the result
    pub fn call0<R>(&mut self) -> Result<R>
    where
        R: 'static,
    {
        if !self.argument_ids.is_empty() {
            return Err(anyhow::anyhow!(
                "Expected no arguments, but segment requires {} argument(s)",
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
    /// # Returns
    /// - `Ok(R)` if execution succeeds and the final value is of type R
    /// - `Err` if:
    ///   - The number of arguments doesn't match (expected one)
    ///   - The argument type doesn't match the expected type
    ///   - The final type doesn't match R
    ///   - There are remaining values on the stack after getting the result
    pub fn call1<A, R>(&mut self, arg: A) -> Result<R>
    where
        A: 'static,
        R: 'static,
    {
        if self.argument_ids.len() != 1 {
            return Err(anyhow::anyhow!(
                "Expected 1 argument, but segment requires {} argument(s)",
                self.argument_ids.len()
            ));
        }
        if self.argument_ids[0] != TypeId::of::<A>() {
            return Err(anyhow::anyhow!(
                "Argument type mismatch: expected {}, got {}",
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
    fn test_drop_on_error() -> Result<(), anyhow::Error> {
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
    fn test_segment_operations() -> Result<(), anyhow::Error> {
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
    fn test_segment_with_argument() -> Result<(), anyhow::Error> {
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
}
