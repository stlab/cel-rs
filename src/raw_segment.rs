use crate::raw_sequence::RawSequence;
use crate::raw_stack::RawAlignedStack;
use anyhow::Result;
use std::cmp::max;

type Operation = fn(&RawSequence, usize, &mut RawAlignedStack) -> Result<usize>;

/**
A segment represents a sequence of operations that can be executed.

Each operation is stored along with its data in the segment's storage,
and can manipulate values on a stack during execution.
*/
pub struct RawSegmentAlignedStack {
    ops: Vec<Operation>,
    storage: RawSequence,
    dropper: Vec<fn(&mut RawSequence, usize) -> usize>,
    base_alignment: usize,
}

impl Default for RawSegmentAlignedStack {
    fn default() -> Self {
        Self::new()
    }
}

impl RawSegmentAlignedStack {
    /// Creates a new empty segment.
    pub fn new() -> Self {
        RawSegmentAlignedStack {
            ops: Vec::new(),
            storage: RawSequence::new(),
            dropper: Vec::new(),
            base_alignment: 0,
        }
    }

    /* Pushes a value into the segment's storage and registers its dropper. */
    fn push_storage<T>(&mut self, value: T)
    where
        T: 'static,
    {
        self.storage.push(value);
        self.dropper
            .push(|storage, p| unsafe { storage.drop_in_place::<T>(p) });
    }

    pub fn raw0<R, F>(&mut self, op: F)
    where
        F: Fn(&mut RawAlignedStack) -> Result<R> + 'static,
        R: 'static,
    {
        self.push_storage(op);
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let result = f(stack)?;
            stack.push(result);
            Ok(r)
        });
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    /// Pushes a nullary operation (taking no arguments) that returns a value of type R.
    pub fn push_op0<R, F>(&mut self, op: F)
    where
        F: Fn() -> R + 'static,
        R: 'static,
    {
        self.push_storage(op);
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            stack.push(f());
            Ok(r)
        });
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    fn _push_op1<const PADDING0: bool, T, R, F>(&mut self)
    where
        F: Fn(T) -> R + 'static,
        T: 'static,
        R: 'static,
    {
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let x: T = unsafe { stack.pop(PADDING0) };
            stack.push(f(x));
            Ok(r)
        });
    }

    /** Pushes a unary operation that takes one argument of type T and returns a value of type R. */
    pub fn push_op1<T, R, F>(&mut self, op: F, padding0: bool)
    where
        F: Fn(T) -> R + 'static,
        T: 'static,
        R: 'static,
    {
        self.push_storage(op);
        match padding0 {
            false => self._push_op1::<false, T, R, F>(),
            true => self._push_op1::<true, T, R, F>(),
        }
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    fn _drop1<const PADDING0: bool, T, F>(&mut self)
    where
        F: Fn(T) + 'static,
        T: 'static,
    {
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let x: T = unsafe { stack.pop(PADDING0) };
            f(x); // drop the result
            Ok(r)
        });
    }

    pub fn drop1<T, F>(&mut self, op: F, padding0: bool)
    where
        F: Fn(T) + 'static,
        T: 'static,
    {
        self.push_storage(op);
        match padding0 {
            false => self._drop1::<false, T, F>(),
            true => self._drop1::<true, T, F>(),
        }
    }

    fn _push_op2<const PADDING0: bool, const PADDING1: bool, T, U, R, F>(&mut self)
    where
        F: Fn(T, U) -> R + 'static,
        T: 'static,
        U: 'static,
        R: 'static,
    {
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let y: U = unsafe { stack.pop(PADDING1) };
            let x: T = unsafe { stack.pop(PADDING0) };
            stack.push(f(x, y));
            Ok(r)
        });
    }

    /// Pushes a binary operation that takes two arguments of types T and U and returns a value of type R.
    pub fn push_op2<T, U, R, F>(&mut self, op: F, padding0: bool, padding1: bool)
    where
        F: Fn(T, U) -> R + 'static,
        T: 'static,
        U: 'static,
        R: 'static,
    {
        self.push_storage(op);
        match (padding0, padding1) {
            (false, false) => self._push_op2::<false, false, T, U, R, F>(),
            (false, true) => self._push_op2::<false, true, T, U, R, F>(),
            (true, false) => self._push_op2::<true, false, T, U, R, F>(),
            (true, true) => self._push_op2::<true, true, T, U, R, F>(),
        }
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    /// Pushes a ternary operation that takes three arguments of types T, U, and V and returns a value of type R.
    fn _push_op3<const PADDING0: bool, const PADDING1: bool, const PADDING2: bool, T, U, V, R, F>(
        &mut self,
    ) where
        F: Fn(T, U, V) -> R + 'static,
        T: 'static,
        U: 'static,
        V: 'static,
        R: 'static,
    {
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let z: V = unsafe { stack.pop(PADDING2) };
            let y: U = unsafe { stack.pop(PADDING1) };
            let x: T = unsafe { stack.pop(PADDING0) };
            stack.push(f(x, y, z));
            Ok(r)
        });
    }

    /// Pushes a ternary operation that takes three arguments of types T, U, and V and returns a value of type R.
    pub fn push_op3<T, U, V, R, F>(&mut self, op: F, padding0: bool, padding1: bool, padding2: bool)
    where
        F: Fn(T, U, V) -> R + 'static,
        T: 'static,
        U: 'static,
        V: 'static,
        R: 'static,
    {
        self.push_storage(op);

        match (padding0, padding1, padding2) {
            (false, false, false) => self._push_op3::<false, false, false, T, U, V, R, F>(),
            (false, false, true) => self._push_op3::<false, false, true, T, U, V, R, F>(),
            (false, true, false) => self._push_op3::<false, true, false, T, U, V, R, F>(),
            (false, true, true) => self._push_op3::<false, true, true, T, U, V, R, F>(),
            (true, false, false) => self._push_op3::<true, false, false, T, U, V, R, F>(),
            (true, false, true) => self._push_op3::<true, false, true, T, U, V, R, F>(),
            (true, true, false) => self._push_op3::<true, true, false, T, U, V, R, F>(),
            (true, true, true) => self._push_op3::<true, true, true, T, U, V, R, F>(),
        }
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    /**
    Executes all operations in the segment and returns the final result.

    # Safety
    This function is unsafe if the result type does not match the type returned by the operations in the segment or if the operations expect any initial values on the stack.
    */
    pub unsafe fn call0<T>(&self) -> Result<T>
    where
        T: 'static,
    {
        let mut stack = RawAlignedStack::with_base_alignment(self.base_alignment);
        let mut p = 0;
        for op in self.ops.iter() {
            p = op(&self.storage, p, &mut stack)?;
        }
        Ok(unsafe { stack.pop(false) })
    }

    /**
    Executes all operations in the segment with one argument of type A and returns the final result.

    # Safety
    This function is unsafe if the argument and result types do not match the types expected or
    returned by the operations in the segment.
    */
    pub unsafe fn call1<A, T>(&self, arg: A) -> Result<T>
    where
        T: 'static,
    {
        // TODO: where does base alignment come from?
        let mut stack = RawAlignedStack::with_base_alignment(self.base_alignment);
        stack.push(arg);
        let mut p = 0;
        for op in self.ops.iter() {
            p = op(&self.storage, p, &mut stack)?;
        }
        Ok(unsafe { stack.pop(false) })
    }

    /**
    Executes all operations in the segment with two arguments of types A and B and returns the final result.

    # Safety
    This function is unsafe if the arguments and result types do not match the types expected or
    returned by the operations in the segment.
    */
    pub unsafe fn call2<A, B, T>(&self, arg: (A, B)) -> Result<T>
    where
        T: 'static,
    {
        // TODO: where does base alignment come from?
        let mut stack = RawAlignedStack::with_base_alignment(self.base_alignment);
        stack.push(arg.0);
        stack.push(arg.1);
        let mut p = 0;
        for op in self.ops.iter() {
            p = op(&self.storage, p, &mut stack)?;
        }
        Ok(unsafe { stack.pop(false) })
    }
}

impl Drop for RawSegmentAlignedStack {
    fn drop(&mut self) {
        let mut p = 0;
        for e in self.dropper.iter() {
            p = e(&mut self.storage, p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nullary_operation() {
        let mut segment = RawSegmentAlignedStack::new();
        segment.push_op0(|| 42);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 42);
        }
    }

    #[test]
    fn test_unary_operation() {
        let mut segment = RawSegmentAlignedStack::new();
        segment.push_op0(|| 42);
        segment.push_op1(|x: i32| x * 2, false);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 84);
        }
    }

    #[test]
    fn test_binary_operation() {
        let mut segment = RawSegmentAlignedStack::new();
        segment.push_op0(|| 10);
        segment.push_op0(|| 5);
        segment.push_op2(|x: i32, y: i32| x + y, false, false);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 15);
        }
    }

    #[test]
    fn test_ternary_operation() {
        let mut segment = RawSegmentAlignedStack::new();
        segment.push_op0(|| 2);
        segment.push_op0(|| 3);
        segment.push_op0(|| 4);
        segment.push_op3(|x: i32, y: i32, z: i32| x + y + z, false, false, false);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 9);
        }
    }

    #[test]
    fn test_complex_chain() {
        let mut segment = RawSegmentAlignedStack::new();
        segment.push_op0(|| 10);
        segment.push_op1(|x: i32| x * 2, false);
        segment.push_op0(|| 5);
        segment.push_op2(|x: i32, y: i32| x + y, false, false);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 25);
        }
    }
}
