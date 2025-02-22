use crate::raw_sequence::RawSequence;
use crate::raw_stack::RawStack;
use anyhow::Result;
type Operation = fn(&RawSequence, usize, &mut RawStack) -> Result<usize>;

/**
A segment represents a sequence of operations that can be executed.

Each operation is stored along with its data in the segment's storage,
and can manipulate values on a stack during execution.
*/
pub struct RawSegment {
    ops: Vec<Operation>,
    storage: RawSequence,
    dropper: Vec<fn(&mut RawSequence, usize) -> usize>,
}

impl RawSegment {
    /** Creates a new empty segment. */
    pub fn new() -> Self {
        RawSegment {
            ops: Vec::new(),
            storage: RawSequence::new(),
            dropper: Vec::new(),
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
    }

    /** Pushes a nullary operation (taking no arguments) that returns a value of type R. */
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
    }

    /** Pushes a unary operation that takes one argument of type T and returns a value of type R. */
    pub fn push_op1<T, R, F>(&mut self, op: F)
    where
        F: Fn(T) -> R + 'static,
        T: 'static,
        R: 'static,
    {
        self.push_storage(op);
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let x: T = unsafe { stack.pop() };
            stack.push(f(x));
            Ok(r)
        });
    }

    pub fn drop1<T, F>(&mut self, op: F)
    where
        F: Fn(T) + 'static,
        T: 'static,
    {
        self.push_storage(op);
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let x: T = unsafe { stack.pop() };
            f(x); // drop the result
            Ok(r)
        });
    }

    /** Pushes a binary operation that takes two arguments of types T and U and returns a value of type R. */
    pub fn push_op2<T, U, R, F>(&mut self, op: F)
    where
        F: Fn(T, U) -> R + 'static,
        T: 'static,
        U: 'static,
        R: 'static,
    {
        self.push_storage(op);
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let y: U = unsafe { stack.pop() };
            let x: T = unsafe { stack.pop() };
            stack.push(f(x, y));
            Ok(r)
        });
    }

    /** Pushes a ternary operation that takes three arguments of types T, U, and V and returns a value of type R. */
    pub fn push_op3<T, U, V, R, F>(&mut self, op: F)
    where
        F: Fn(T, U, V) -> R + 'static,
        T: 'static,
        U: 'static,
        V: 'static,
        R: 'static,
    {
        self.push_storage(op);
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let z: V = unsafe { stack.pop() };
            let y: U = unsafe { stack.pop() };
            let x: T = unsafe { stack.pop() };
            stack.push(f(x, y, z));
            Ok(r)
        });
    }

    /**
    Executes all operations in the segment and returns the final result.

    # Safety
    This function is unsafe because it performs unchecked type conversions when
    popping values from the stack. The caller must ensure that the operations
    were pushed in the correct order and with matching types.
    */
    pub unsafe fn call0<T>(&self) -> Result<T>
    where
        T: 'static,
    {
        let mut stack = RawStack::new();
        let mut p = 0;
        for op in self.ops.iter() {
            p = op(&self.storage, p, &mut stack)?;
        }
        Ok(stack.pop())
    }

    pub unsafe fn call1<A, T>(&self, arg: A) -> Result<T>
    where
        T: 'static,
    {
        let mut stack = RawStack::new();
        stack.push(arg);
        let mut p = 0;
        for op in self.ops.iter() {
            p = op(&self.storage, p, &mut stack)?;
        }
        Ok(stack.pop())
    }

    pub unsafe fn call2<A, B, T>(&self, arg: (A, B)) -> Result<T>
    where
        T: 'static,
    {
        let mut stack = RawStack::new();
        stack.push(arg.0);
        stack.push(arg.1);
        let mut p = 0;
        for op in self.ops.iter() {
            p = op(&self.storage, p, &mut stack)?;
        }
        Ok(stack.pop())
    }
}

impl Drop for RawSegment {
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
        segment.push_op1(|x: i32| x * 2);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 84);
        }
    }

    #[test]
    fn test_binary_operation() {
        let mut segment = RawSegment::new();
        segment.push_op0(|| 10);
        segment.push_op0(|| 5);
        segment.push_op2(|x: i32, y: i32| x + y);
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
        segment.push_op3(|x: i32, y: i32, z: i32| x + y + z);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 9);
        }
    }

    #[test]
    fn test_complex_chain() {
        let mut segment = RawSegment::new();
        segment.push_op0(|| 10);
        segment.push_op1(|x: i32| x * 2);
        segment.push_op0(|| 5);
        segment.push_op2(|x: i32, y: i32| x + y);
        unsafe {
            assert_eq!(segment.call0::<i32>().unwrap(), 25);
        }
    }
}
