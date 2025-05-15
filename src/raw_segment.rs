use crate::raw_sequence::RawSequence;
use crate::raw_stack::RawStack;
use anyhow::Result;
use std::cmp::max;

type Operation = fn(&RawSequence, usize, &mut RawStack) -> Result<usize>;

/// A segment represents a sequence of operations that can be executed.
///
/// Each operation is stored along with its data in the segment's storage,
/// and can manipulate values on a stack during execution.
pub struct RawSegment {
    ops: Vec<Operation>,
    storage: RawSequence,
    dropper: Vec<fn(&mut RawSequence, usize) -> usize>,
    base_alignment: usize,
}

impl Default for RawSegment {
    fn default() -> Self {
        Self::new()
    }
}

impl RawSegment {
    /// Creates a new empty segment.
    #[must_use]
    pub fn new() -> Self {
        RawSegment {
            ops: Vec::new(),
            storage: RawSequence::new(),
            dropper: Vec::new(),
            base_alignment: 0,
        }
    }

    /// Pushes a value into the segment's storage and registers its dropper.
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
        F: Fn(&mut RawStack) -> Result<R> + 'static,
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

    /// Pushes a unary operation that takes one argument of type T and returns a value of type R.
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

    fn push_op1_<const PADDING0: bool, T, R, F>(&mut self)
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

    fn push_op1r_<const PADDING0: bool, T, R, F>(&mut self)
    where
        F: Fn(&mut RawStack, T) -> Result<R> + 'static,
        T: 'static,
        R: 'static,
    {
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let x: T = unsafe { stack.pop(PADDING0) };
            let result = f(stack, x)?;
            stack.push(result);
            Ok(r)
        });
    }

    /// Pushes a unary operation that takes one argument of type T and returns a value of type R.
    pub fn push_op1<T, R, F>(&mut self, op: F, padding0: bool)
    where
        F: Fn(T) -> R + 'static,
        T: 'static,
        R: 'static,
    {
        self.push_storage(op);
        if padding0 {
            self.push_op1_::<true, T, R, F>();
        } else {
            self.push_op1_::<false, T, R, F>();
        }
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    pub fn raw1<T, R, F>(&mut self, op: F, padding0: bool)
    where
        F: Fn(&mut RawStack, T) -> Result<R> + 'static,
        T: 'static,
        R: 'static,
    {
        self.push_storage(op);
        if padding0 {
            self.push_op1r_::<true, T, R, F>();
        } else {
            self.push_op1r_::<false, T, R, F>();
        }
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    fn drop1_<const PADDING0: bool, T, F>(&mut self)
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
        if padding0 {
            self.drop1_::<true, T, F>();
        } else {
            self.drop1_::<false, T, F>();
        }
    }

    fn push_op2_<const PADDING0: bool, const PADDING1: bool, T, U, R, F>(&mut self)
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

    /// Pushes a binary operation that takes two arguments of types T and U and returns a value of
    /// type R.
    pub fn push_op2<T, U, R, F>(&mut self, op: F, padding0: bool, padding1: bool)
    where
        F: Fn(T, U) -> R + 'static,
        T: 'static,
        U: 'static,
        R: 'static,
    {
        self.push_storage(op);
        match (padding0, padding1) {
            (false, false) => self.push_op2_::<false, false, T, U, R, F>(),
            (false, true) => self.push_op2_::<false, true, T, U, R, F>(),
            (true, false) => self.push_op2_::<true, false, T, U, R, F>(),
            (true, true) => self.push_op2_::<true, true, T, U, R, F>(),
        }
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    /// Pushes a ternary operation that takes three arguments of types T, U, and V and returns a
    /// value of type R.
    #[expect(clippy::many_single_char_names, reason = "patterned code")]
    fn push_op3_<const PADDING0: bool, const PADDING1: bool, const PADDING2: bool, T, U, V, R, F>(
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
            (false, false, false) => self.push_op3_::<false, false, false, T, U, V, R, F>(),
            (false, false, true) => self.push_op3_::<false, false, true, T, U, V, R, F>(),
            (false, true, false) => self.push_op3_::<false, true, false, T, U, V, R, F>(),
            (false, true, true) => self.push_op3_::<false, true, true, T, U, V, R, F>(),
            (true, false, false) => self.push_op3_::<true, false, false, T, U, V, R, F>(),
            (true, false, true) => self.push_op3_::<true, false, true, T, U, V, R, F>(),
            (true, true, false) => self.push_op3_::<true, true, false, T, U, V, R, F>(),
            (true, true, true) => self.push_op3_::<true, true, true, T, U, V, R, F>(),
        }
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }

    /// Executes all operations in the segment and returns the final result.
    ///
    /// # Errors
    /// Halts execution and returns an error if any operation returns an error.
    ///
    /// # Safety
    /// This function is unsafe if the result type does not match the type returned by the
    /// operations in the segment or if the operations expect any initial values on the stack.
    pub unsafe fn call0<T>(&self) -> Result<T>
    where
        T: 'static,
    {
        let mut stack = RawStack::with_base_alignment(self.base_alignment);
        let mut p = 0;
        for op in &self.ops {
            p = op(&self.storage, p, &mut stack)?;
        }
        Ok(unsafe { stack.pop(false) })
    }

    /// Executes all operations in the segment with one argument of type A and returns the final
    /// result.
    ///
    /// # Errors
    /// Halts execution and returns an error if any operation returns an error.
    ///
    /// # Safety
    /// This function is unsafe if the argument and result types do not match the types expected or
    /// returned by the operations in the segment.
    pub unsafe fn call1<A, T>(&self, arg: A) -> Result<T>
    where
        T: 'static,
    {
        // TODO: where does base alignment come from?
        let mut stack = RawStack::with_base_alignment(self.base_alignment);
        stack.push(arg);
        let mut p = 0;
        for op in &self.ops {
            p = op(&self.storage, p, &mut stack)?;
        }
        Ok(unsafe { stack.pop(false) })
    }

    /// Executes all operations in the segment with two arguments of types A and B and returns the
    /// final result.
    ///
    /// # Errors
    /// Halts execution and returns an error if any operation returns an error.
    ///
    /// # Safety
    /// This function is unsafe if the arguments and result types do not match the types expected or
    /// returned by the operations in the segment.
    pub unsafe fn call2<A, B, T>(&self, arg: (A, B)) -> Result<T>
    where
        T: 'static,
    {
        // TODO: where does base alignment come from?
        let mut stack = RawStack::with_base_alignment(self.base_alignment);
        stack.push(arg.0);
        stack.push(arg.1);
        let mut p = 0;
        for op in &self.ops {
            p = op(&self.storage, p, &mut stack)?;
        }
        Ok(unsafe { stack.pop(false) })
    }
}

impl Drop for RawSegment {
    fn drop(&mut self) {
        let mut p = 0;
        for e in &self.dropper {
            p = e(&mut self.storage, p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nullary_operation() {
        let mut segment = RawSegment::new();
        segment.push_op0(|| 42);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 42);
        }
    }

    #[test]
    fn test_unary_operation() {
        let mut segment = RawSegment::new();
        segment.push_op0(|| 42);
        segment.push_op1(|x: i32| x * 2, false);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 84);
        }
    }

    #[test]
    fn test_binary_operation() {
        let mut segment = RawSegment::new();
        segment.push_op0(|| 10);
        segment.push_op0(|| 5);
        segment.push_op2(|x: i32, y: i32| x + y, false, false);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 15);
        }
    }

    #[test]
    fn test_ternary_operation() {
        let mut segment = RawSegment::new();
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
        let mut segment = RawSegment::new();
        segment.push_op0(|| 10);
        segment.push_op1(|x: i32| x * 2, false);
        segment.push_op0(|| 5);
        segment.push_op2(|x: i32, y: i32| x + y, false, false);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 25);
        }
    }
}
