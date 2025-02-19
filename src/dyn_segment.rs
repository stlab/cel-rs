use crate::raw_segment::RawSegment;
use std::any::TypeId;

/**
A type-checked wrapper around RawSegment that maintains a stack of type information
to ensure type safety during operation execution.

DynSegment tracks the types of values on the stack at compile time and verifies
that operations receive arguments of the correct type. This prevents type mismatches
that could occur when using RawSegment directly.
*/
pub struct DynSegment {
    segment: RawSegment,
    type_ids: Vec<TypeId>,
}

impl DynSegment {
    /** Creates a new empty segment with no operations. */
    pub fn new() -> Self {
        DynSegment {
            segment: RawSegment::new(),
            type_ids: Vec::new(),
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
        match self.type_ids.pop() {
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
        self.type_ids.push(TypeId::of::<R>());
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
        self.type_ids.push(TypeId::of::<R>());
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
        self.type_ids.push(TypeId::of::<R>());
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
        self.type_ids.push(TypeId::of::<R>());
    }

    /**
    Executes all operations in the segment and returns the final result.

    Verifies that the final type on the stack matches the expected return type T
    and that no other values remain on the stack.

    Panics if the type stack is empty, the final type doesn't match T, or if
    there are remaining values on the stack after getting the result.
    */
    pub fn call<T>(&mut self) -> T
    where
        T: 'static,
    {
        self.pop_type::<T>();
        if self.type_ids.len() != 0 {
            panic!("Value(s) left on execution stack");
        }
        unsafe { self.segment.call0() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_segment_operations() {
        let mut operations = DynSegment::new();

        operations.push_op0(|| -> u32 { 30 });
        operations.push_op0(|| -> u32 { 12 });
        operations.push_op2(|x: u32, y: u32| -> u32 { x + y });
        operations.push_op0(|| -> u32 { 100 });
        operations.push_op0(|| -> u32 { 10 });
        operations.push_op3(|x: u32, y: u32, z: u32| -> u32 { x + y - z });
        operations.push_op1(|x: u32| -> String { format!("result: {}", x.to_string()) });

        let final_result: String = operations.call();
        assert_eq!(final_result, "result: 132");
    }
}
