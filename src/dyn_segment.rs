use crate::raw_segment::RawSegment;
use std::any::TypeId;

pub trait TupleToDynTypeList {
    fn to_type_list() -> Vec<TypeId>;
}

impl TupleToDynTypeList for () {
    fn to_type_list() -> Vec<TypeId> {
        Vec::new()
    }
}

impl<A: 'static> TupleToDynTypeList for (A,) {
    fn to_type_list() -> Vec<TypeId> {
        vec![TypeId::of::<A>()]
    }
}

impl<A: 'static, B: 'static> TupleToDynTypeList for (A, B) {
    fn to_type_list() -> Vec<TypeId> {
        vec![TypeId::of::<A>(), TypeId::of::<B>()]
    }
}

impl<A: 'static, B: 'static, C: 'static> TupleToDynTypeList for (A, B, C) {
    fn to_type_list() -> Vec<TypeId> {
        vec![TypeId::of::<A>(), TypeId::of::<B>(), TypeId::of::<C>()]
    }
}

/**
A type-checked wrapper around RawSegment that maintains a stack of type information
to ensure type safety during operation execution.

DynSegment tracks the types of values on the stack at compile time and verifies
that operations receive arguments of the correct type. This prevents type mismatches
that could occur when using RawSegment directly.
*/
pub struct DynSegment {
    segment: RawSegment,
    stack_ids: Vec<TypeId>,
    argument_ids: Vec<TypeId>,
}

impl DynSegment {
    /*
       I'm not certain if new should be a generic function or if the argument
       types should be passed in as a slice. I'm going to try the generic for now.
    */

    /** Creates a new empty segment with no operations. */
    pub fn new<Args: TupleToDynTypeList>() -> Self {
        DynSegment {
            segment: RawSegment::new(),
            stack_ids: <Args as TupleToDynTypeList>::to_type_list(),
            argument_ids: <Args as TupleToDynTypeList>::to_type_list(),
        }
    }

    /*
    Verifies and removes the expected type from the type stack.

    Panics if the type being popped doesn't match the expected type or if
    the type stack is empty.
    */
    fn pop_type<T>(&mut self)
    where
        T: 'static,
    {
        match self.stack_ids.pop() {
            Some(tid) if tid == TypeId::of::<T>() => {}
            _ => {
                panic!("Type mismatch: expected {}", std::any::type_name::<T>());
            }
        }
    }

    /**
    Pushes a nullary operation that takes no arguments and returns a value of type R.

    The return type is tracked in the type stack for subsequent operations.
    */
    pub fn push_op0<R, F>(&mut self, op: F)
    where
        F: Fn() -> R + 'static,
        R: 'static,
    {
        self.segment.push_op0(op);
        self.stack_ids.push(TypeId::of::<R>());
    }

    /**
    Pushes a unary operation that takes one argument of type T and returns a value of type R.

    Verifies that the top of the type stack matches the expected input type T
    before adding the operation.
    */
    pub fn push_op1<T, R, F>(&mut self, op: F)
    where
        F: Fn(T) -> R + 'static,
        T: 'static,
        R: 'static,
    {
        self.pop_type::<T>();
        self.segment.push_op1(op);
        self.stack_ids.push(TypeId::of::<R>());
    }

    /**
    Pushes a binary operation that takes two arguments of types T and U and returns a value of type R.

    Verifies that the top two types on the type stack match the expected input types U and T
    (in that order) before adding the operation.
    */
    pub fn push_op2<T, U, R, F>(&mut self, op: F)
    where
        F: Fn(T, U) -> R + 'static,
        T: 'static,
        U: 'static,
        R: 'static,
    {
        self.pop_type::<U>();
        self.pop_type::<T>();
        self.segment.push_op2(op);
        self.stack_ids.push(TypeId::of::<R>());
    }

    /**
    Pushes a ternary operation that takes three arguments of types T, U, and V and returns a value of type R.

    Verifies that the top three types on the type stack match the expected input types V, U, and T
    (in that order) before adding the operation.
    */
    pub fn push_op3<T, U, V, R, F>(&mut self, op: F)
    where
        F: Fn(T, U, V) -> R + 'static,
        T: 'static,
        U: 'static,
        V: 'static,
        R: 'static,
    {
        self.pop_type::<V>();
        self.pop_type::<U>();
        self.pop_type::<T>();
        self.segment.push_op3(op);
        self.stack_ids.push(TypeId::of::<R>());
    }

    /**
    Executes all operations in the segment and returns the final result.

    Verifies that the final type on the stack matches the expected return type T
    and that no other values remain on the stack.

    Panics if the type stack is empty, the final type doesn't match T, or if
    there are remaining values on the stack after getting the result.
    */
    pub fn call0<T>(&mut self) -> T
    where
        T: 'static,
    {
        assert!(
            self.argument_ids.len() == 0,
            "Expected {} argument(s)",
            self.argument_ids.len()
        );
        self.pop_type::<T>();
        assert!(
            self.stack_ids.len() == 0,
            "Value(s) left on execution stack"
        );
        unsafe { self.segment.call0() }
    }

    pub fn call1<A, R>(&mut self, arg: A) -> R
    where
        A: 'static,
        R: 'static,
    {
        assert!(
            self.argument_ids.len() == 1,
            "Expected {} argument(s)",
            self.argument_ids.len()
        );
        assert!(
            self.argument_ids[0] == TypeId::of::<A>(),
            "Argument type mismatch"
        );
        self.pop_type::<R>();
        if self.stack_ids.len() != 0 {
            panic!("Value(s) left on execution stack");
        }
        unsafe { self.segment.call1(arg) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_segment_operations() {
        let mut operations = DynSegment::new::<()>();

        operations.push_op0(|| -> u32 { 30 });
        operations.push_op0(|| -> u32 { 12 });
        operations.push_op2(|x: u32, y: u32| -> u32 { x + y });
        operations.push_op0(|| -> u32 { 100 });
        operations.push_op0(|| -> u32 { 10 });
        operations.push_op3(|x: u32, y: u32, z: u32| -> u32 { x + y - z });
        operations.push_op1(|x: u32| -> String { format!("result: {}", x.to_string()) });

        let final_result: String = operations.call0();
        assert_eq!(final_result, "result: 132");
    }
    #[test]
    fn test_segment_with_argument() {
        let mut operations = DynSegment::new::<(u32,)>();

        operations.push_op0(|| -> u32 { 12 });
        operations.push_op2(|x: u32, y: u32| -> u32 { x + y });
        operations.push_op0(|| -> u32 { 100 });
        operations.push_op0(|| -> u32 { 10 });
        operations.push_op3(|x: u32, y: u32, z: u32| -> u32 { x + y - z });
        operations.push_op1(|x: u32| -> String { format!("result: {}", x.to_string()) });

        let final_result: String = operations.call1(30u32);
        assert_eq!(final_result, "result: 132");
    }
}
