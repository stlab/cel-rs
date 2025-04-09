use crate::raw_segment::RawSegment;
use crate::raw_stack::RawStack;
use crate::type_list::{IntoList, List, TypeHandler};
use anyhow::Result;
use anyhow::ensure;
use std::any::TypeId;

pub trait ToTypeIDList: List {
    fn to_type_id_list() -> Vec<TypeId>;
    fn to_drop_list() -> Vec<Dropper>;
}

impl ToTypeIDList for () {
    fn to_type_id_list() -> Vec<TypeId> {
        Vec::new()
    }
    fn to_drop_list() -> Vec<Dropper> {
        Vec::new()
    }
}

impl<T: 'static, U: ToTypeIDList + 'static> ToTypeIDList for (T, U) {
    fn to_type_id_list() -> Vec<TypeId> {
        let mut list = U::to_type_id_list();
        list.push(TypeId::of::<T>());
        list
    }
    fn to_drop_list() -> Vec<Dropper> {
        let mut list = U::to_drop_list();
        list.push(|stack| unsafe { stack.drop::<T>() });
        list
    }
}

// Pulled from segment.rs - generalize this and have one copy
struct EqListTypeIDListHandler<'a>(&'a [TypeId], &'a mut usize, &'a mut bool);

impl TypeHandler for EqListTypeIDListHandler<'_> {
    fn invoke<T: List + 'static>(&mut self) {
        *self.2 = *self.2 && TypeId::of::<T::Head>() == self.0[*self.1];
        *self.1 += 1;
    }
}

trait EqListTypeIDList {
    fn equal(ids: &[TypeId]) -> bool;
}

impl<T: List> EqListTypeIDList for T {
    fn equal(ids: &[TypeId]) -> bool {
        let mut index: usize = 0;
        let mut result: bool = true;
        let mut handler = EqListTypeIDListHandler(ids, &mut index, &mut result);
        T::for_each_type(&mut handler);
        result
    }
}

/**
A type-checked wrapper around RawSegment that maintains a stack of type information
to ensure type safety during operation execution.

DynSegment tracks the types of values on the stack at compile time and verifies
that operations receive arguments of the correct type. This prevents type mismatches
that could occur when using RawSegment directly.
*/
type Dropper = fn(&mut RawStack);
pub struct DynSegment {
    pub(crate) segment: RawSegment,
    pub(crate) argument_ids: Vec<TypeId>,
    // Invariant: stack_ids.len() == stack_unwind.len(), consider making this a vector of tuples
    pub(crate) stack_ids: Vec<TypeId>,
    stack_unwind: Vec<Dropper>,
}
pub trait IntoReverseTypeIDList {
    fn to_type_id_list() -> Vec<TypeId>;
    fn to_drop_list() -> Vec<Dropper>;
}

impl<Args: IntoList> IntoReverseTypeIDList for Args
where
    Args::Result: List,
    <Args::Result as List>::Reverse: ToTypeIDList,
{
    fn to_type_id_list() -> Vec<TypeId> {
        <<Args::Result as List>::Reverse as ToTypeIDList>::to_type_id_list()
    }
    fn to_drop_list() -> Vec<Dropper> {
        <<Args::Result as List>::Reverse as ToTypeIDList>::to_drop_list()
    }
}

impl DynSegment {
    /*
       I'm not certain if new should be a generic function or if the argument
       types should be passed in as a slice. I'm going to try the generic for now.
    */

    /** Creates a new empty segment with no operations. */
    pub fn new<Args: IntoReverseTypeIDList>() -> Self {
        DynSegment {
            segment: RawSegment::new(),
            argument_ids: Args::to_type_id_list(),
            stack_ids: Args::to_type_id_list(),
            stack_unwind: Args::to_drop_list(),
        }
    }

    /*
    Removes the expected type from the type and unwind stacks.

    # Preconditions

    The type and unwind stacks must be the same length and at least one type must be present.
    */
    fn pop_types(&mut self, n: usize) {
        self.stack_ids.truncate(self.stack_ids.len() - n);
        self.stack_unwind.truncate(self.stack_unwind.len() - n);
    }

    /**
    Verifies that the argument types match the expected types on the type stack.

    Returns an error if the argument types don't match the expected types or if
    there are too many arguments.

    To avoid reversing the arguments and reversing the slice, this operation
    is done in argument order, not stack order.
    */
    fn type_check_stack<L: List>(&self) -> Result<()> {
        ensure!(
            L::LENGTH <= self.stack_ids.len(),
            "Too many arguments: expected {}, got {}",
            L::LENGTH,
            self.stack_ids.len()
        );
        let start = self.stack_ids.len() - L::LENGTH;
        ensure!(L::equal(&self.stack_ids[start..]), "Type mismatch");
        Ok(())
    }

    // Push type to stack and register dropper.
    fn push_type<T>(&mut self)
    where
        T: 'static,
    {
        self.stack_ids.push(TypeId::of::<T>());
        self.stack_unwind.push(|stack| {
            unsafe { stack.drop::<T>() };
        });
    }

    /**
    Pushes a nullary operation that takes no arguments and returns a value of type R.

    The return type is tracked in the type stack for subsequent operations.
    */
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
        let unwind = self.stack_unwind.clone();
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

    /**
    Pushes a unary operation that takes one argument of type T and returns a value of type R.

    Verifies that the top of the type stack matches the expected input type T
    before adding the operation.
    */
    pub fn op1<T, R, F>(&mut self, op: F) -> Result<()>
    where
        F: Fn(T) -> R + 'static,
        T: 'static,
        R: 'static,
    {
        self.type_check_stack::<(T, ())>()?;
        self.pop_types(1);
        self.segment.push_op1(op);
        self.push_type::<R>();
        Ok(())
    }

    /**
    Pushes a binary operation that takes two arguments of types T and U and returns a value of type R.

    Verifies that the top two types on the type stack match the expected input types U and T
    (in that order) before adding the operation.
    */
    pub fn op2<T, U, R, F>(&mut self, op: F) -> Result<()>
    where
        F: Fn(T, U) -> R + 'static,
        T: 'static,
        U: 'static,
        R: 'static,
    {
        self.type_check_stack::<(T, (U, ()))>()?;
        self.pop_types(2);
        self.segment.push_op2(op);
        self.push_type::<R>();
        Ok(())
    }

    /**
    Pushes a ternary operation that takes three arguments of types T, U, and V and returns a value of type R.

    Verifies that the top three types on the type stack match the expected input types V, U, and T
    (in that order) before adding the operation.
    */
    pub fn op3<T, U, V, R, F>(&mut self, op: F) -> Result<()>
    where
        F: Fn(T, U, V) -> R + 'static,
        T: 'static,
        U: 'static,
        V: 'static,
        R: 'static,
    {
        self.type_check_stack::<(T, (U, (V, ())))>()?;
        self.pop_types(3);
        self.segment.push_op3(op);
        self.push_type::<R>();
        Ok(())
    }

    /**
    Executes all operations in the segment and returns the final result.

    # Returns
    - `Ok(R)` if execution succeeds and the final value is of type R
    - `Err` if:
      - There are unexpected arguments (expected none)
      - The final type doesn't match R
      - There are remaining values on the stack after getting the result
    */
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
        self.type_check_stack::<(R, ())>()?;
        self.pop_types(1);
        if !self.stack_ids.is_empty() {
            return Err(anyhow::anyhow!(
                "{} value(s) left on execution stack",
                self.stack_ids.len()
            ));
        }
        unsafe { self.segment.call0() }
    }

    /**
    Executes all operations in the segment with one argument and returns the final result.

    # Returns
    - `Ok(R)` if execution succeeds and the final value is of type R
    - `Err` if:
      - The number of arguments doesn't match (expected one)
      - The argument type doesn't match the expected type
      - The final type doesn't match R
      - There are remaining values on the stack after getting the result
    */
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
        self.type_check_stack::<(R, ())>()?;
        self.pop_types(1);
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
