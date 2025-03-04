use crate::raw_segment::RawSegment;
use crate::raw_stack::RawStack;
use anyhow::Result;
use std::any::TypeId;

pub trait TupleToDynTypeStack {
    fn to_type_list() -> Vec<TypeId>;
}

impl TupleToDynTypeStack for () {
    fn to_type_list() -> Vec<TypeId> {
        Vec::new()
    }
}

impl<A: 'static> TupleToDynTypeStack for (A,) {
    fn to_type_list() -> Vec<TypeId> {
        vec![TypeId::of::<A>()]
    }
}

impl<A: 'static, B: 'static> TupleToDynTypeStack for (A, B) {
    fn to_type_list() -> Vec<TypeId> {
        vec![TypeId::of::<A>(), TypeId::of::<B>()]
    }
}

impl<A: 'static, B: 'static, C: 'static> TupleToDynTypeStack for (A, B, C) {
    fn to_type_list() -> Vec<TypeId> {
        vec![TypeId::of::<A>(), TypeId::of::<B>(), TypeId::of::<C>()]
    }
}
impl<A: 'static, B: 'static, C: 'static, D: 'static> TupleToDynTypeStack for (A, B, C, D) {
    fn to_type_list() -> Vec<TypeId> {
        vec![
            TypeId::of::<A>(),
            TypeId::of::<B>(),
            TypeId::of::<C>(),
            TypeId::of::<D>(),
        ]
    }
}

impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static> TupleToDynTypeStack
    for (A, B, C, D, E)
{
    fn to_type_list() -> Vec<TypeId> {
        vec![
            TypeId::of::<A>(),
            TypeId::of::<B>(),
            TypeId::of::<C>(),
            TypeId::of::<D>(),
            TypeId::of::<E>(),
        ]
    }
}

impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static, F: 'static> TupleToDynTypeStack
    for (A, B, C, D, E, F)
{
    fn to_type_list() -> Vec<TypeId> {
        vec![
            TypeId::of::<A>(),
            TypeId::of::<B>(),
            TypeId::of::<C>(),
            TypeId::of::<D>(),
            TypeId::of::<E>(),
            TypeId::of::<F>(),
        ]
    }
}

impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static, F: 'static, G: 'static>
    TupleToDynTypeStack for (A, B, C, D, E, F, G)
{
    fn to_type_list() -> Vec<TypeId> {
        vec![
            TypeId::of::<A>(),
            TypeId::of::<B>(),
            TypeId::of::<C>(),
            TypeId::of::<D>(),
            TypeId::of::<E>(),
            TypeId::of::<F>(),
            TypeId::of::<G>(),
        ]
    }
}

impl<
        A: 'static,
        B: 'static,
        C: 'static,
        D: 'static,
        E: 'static,
        F: 'static,
        G: 'static,
        H: 'static,
    > TupleToDynTypeStack for (A, B, C, D, E, F, G, H)
{
    fn to_type_list() -> Vec<TypeId> {
        vec![
            TypeId::of::<A>(),
            TypeId::of::<B>(),
            TypeId::of::<C>(),
            TypeId::of::<D>(),
            TypeId::of::<E>(),
            TypeId::of::<F>(),
            TypeId::of::<G>(),
            TypeId::of::<H>(),
        ]
    }
}

impl<
        A: 'static,
        B: 'static,
        C: 'static,
        D: 'static,
        E: 'static,
        F: 'static,
        G: 'static,
        H: 'static,
        I: 'static,
    > TupleToDynTypeStack for (A, B, C, D, E, F, G, H, I)
{
    fn to_type_list() -> Vec<TypeId> {
        vec![
            TypeId::of::<A>(),
            TypeId::of::<B>(),
            TypeId::of::<C>(),
            TypeId::of::<D>(),
            TypeId::of::<E>(),
            TypeId::of::<F>(),
            TypeId::of::<G>(),
            TypeId::of::<H>(),
            TypeId::of::<I>(),
        ]
    }
}

impl<
        A: 'static,
        B: 'static,
        C: 'static,
        D: 'static,
        E: 'static,
        F: 'static,
        G: 'static,
        H: 'static,
        I: 'static,
        J: 'static,
    > TupleToDynTypeStack for (A, B, C, D, E, F, G, H, I, J)
{
    fn to_type_list() -> Vec<TypeId> {
        vec![
            TypeId::of::<A>(),
            TypeId::of::<B>(),
            TypeId::of::<C>(),
            TypeId::of::<D>(),
            TypeId::of::<E>(),
            TypeId::of::<F>(),
            TypeId::of::<G>(),
            TypeId::of::<H>(),
            TypeId::of::<I>(),
            TypeId::of::<J>(),
        ]
    }
}

impl<
        A: 'static,
        B: 'static,
        C: 'static,
        D: 'static,
        E: 'static,
        F: 'static,
        G: 'static,
        H: 'static,
        I: 'static,
        J: 'static,
        K: 'static,
    > TupleToDynTypeStack for (A, B, C, D, E, F, G, H, I, J, K)
{
    fn to_type_list() -> Vec<TypeId> {
        vec![
            TypeId::of::<A>(),
            TypeId::of::<B>(),
            TypeId::of::<C>(),
            TypeId::of::<D>(),
            TypeId::of::<E>(),
            TypeId::of::<F>(),
            TypeId::of::<G>(),
            TypeId::of::<H>(),
            TypeId::of::<I>(),
            TypeId::of::<J>(),
            TypeId::of::<K>(),
        ]
    }
}

impl<
        A: 'static,
        B: 'static,
        C: 'static,
        D: 'static,
        E: 'static,
        F: 'static,
        G: 'static,
        H: 'static,
        I: 'static,
        J: 'static,
        K: 'static,
        L: 'static,
    > TupleToDynTypeStack for (A, B, C, D, E, F, G, H, I, J, K, L)
{
    fn to_type_list() -> Vec<TypeId> {
        vec![
            TypeId::of::<A>(),
            TypeId::of::<B>(),
            TypeId::of::<C>(),
            TypeId::of::<D>(),
            TypeId::of::<E>(),
            TypeId::of::<F>(),
            TypeId::of::<G>(),
            TypeId::of::<H>(),
            TypeId::of::<I>(),
            TypeId::of::<J>(),
            TypeId::of::<K>(),
            TypeId::of::<L>(),
        ]
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
    segment: RawSegment,
    argument_ids: Vec<TypeId>,
    // Invariant: stack_ids.len() == stack_unwind.len(), consider making this a vector of tuples
    stack_ids: Vec<TypeId>,
    stack_unwind: Vec<Dropper>,
}

impl DynSegment {
    /*
       I'm not certain if new should be a generic function or if the argument
       types should be passed in as a slice. I'm going to try the generic for now.
    */

    /** Creates a new empty segment with no operations. */
    pub fn new<Args: TupleToDynTypeStack>() -> Self {
        DynSegment {
            segment: RawSegment::new(),
            argument_ids: <Args as TupleToDynTypeStack>::to_type_list(),
            stack_ids: <Args as TupleToDynTypeStack>::to_type_list(),
            stack_unwind: Vec::new(),
        }
    }

    /*
    Verifies and removes the expected type from the type stack.

    Returns an error if the type being popped doesn't match the expected type or if
    the type stack is empty.
    */
    fn pop_type<T>(&mut self) -> Result<()>
    where
        T: 'static,
    {
        if self.stack_ids.pop() != Some(TypeId::of::<T>()) {
            return Err(anyhow::anyhow!(
                "Type mismatch: expected {}",
                std::any::type_name::<T>()
            ));
        }
        self.stack_unwind.pop();
        Ok(())
    }

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
        self.pop_type::<T>()?;
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
        self.pop_type::<U>()?;
        self.pop_type::<T>()?;
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
        self.pop_type::<V>()?;
        self.pop_type::<U>()?;
        self.pop_type::<T>()?;
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
        if self.argument_ids.len() != 0 {
            return Err(anyhow::anyhow!(
                "Expected no arguments, but segment requires {} argument(s)",
                self.argument_ids.len()
            ));
        }
        self.pop_type::<R>()?;
        if self.stack_ids.len() != 0 {
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
        self.pop_type::<R>()?;
        if self.stack_ids.len() != 0 {
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

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
        operations.op1(|x: u32| -> String { format!("result: {}", x.to_string()) })?;

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
        operations.op1(|x: u32| -> String { format!("result: {}", x.to_string()) })?;

        let final_result: String = operations.call1(30u32)?;
        assert_eq!(final_result, "result: 132");

        Ok(())
    }
}
