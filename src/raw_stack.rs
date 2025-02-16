use std::mem::size_of;

/**
A simple raw stack that stores values as raw bytes.

`RawStack` allows pushing values of any type into a byte buffer and retrieving
them manually. The retrieval (`pop`) operation is unsafe and requires the caller
to ensure that the type parameter matches the value at the top of the stack.
*/
#[derive(Debug)]
pub struct RawStack {
    buffer: Vec<u8>,
}

impl RawStack {
    /**
    Creates a new `RawStack` with an initial capacity.

    # Examples

    ```
    use cel_rs::RawStack;
    let stack = RawStack::new();
    ```
    */
    pub fn new() -> Self {
        RawStack { buffer: Vec::new() }
    }

    /**
    Pushes a value of type `T` onto the stack.

    The value is stored as raw bytes in the internal buffer. The pushed value must be
    later popped using the correct type.

    # Type Parameters

    * `T`: The type of the value to push.

    # Examples

    ```
    use cel_rs::RawStack;
    let mut stack = RawStack::new();
    stack.push(42u32);
    ```

    # Complexity

    The function has an amortized O(1) time complexity.
    */
    pub fn push<T>(&mut self, value: T) {
        let len = self.buffer.len();
        self.buffer.reserve(size_of::<T>());
        unsafe {
            self.buffer.set_len(len + size_of::<T>());
            std::ptr::write_unaligned(self.buffer.as_mut_ptr().add(len) as *mut T, value);
        }
    }

    /**
    Pops a value of type `T` from the stack. Does not change the stack capacity.

    # Safety

    The type `T` must be the same type as the value on the top of the stack.
    Incorrect usage can lead to undefined behavior.

    # Type Parameters

    * `T`: The type of the value to pop.

    # Examples

    ```
    use cel_rs::RawStack;
    let mut stack = RawStack::new();
    stack.push(100u32);
    let value: u32 = unsafe { stack.pop() };
    ```
    */
    pub unsafe fn pop<T>(&mut self) -> T {
        let p: usize = self.buffer.len() - size_of::<T>();
        let result = std::ptr::read(self.buffer.as_ptr().add(p) as *const T);
        self.buffer.truncate(p);
        result
    }
}

/* Test module */
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_pop_u32() {
        let mut stack = RawStack::new();
        stack.push(10u32);
        let result: u32 = unsafe { stack.pop() };
        assert_eq!(result, 10);
    }

    #[test]
    fn test_multiple_push_pop() {
        let mut stack = RawStack::new();
        stack.push(1u32);
        stack.push(2u32);
        stack.push(3u32);
        let v3: u32 = unsafe { stack.pop() };
        let v2: u32 = unsafe { stack.pop() };
        let v1: u32 = unsafe { stack.pop() };
        assert_eq!(v1, 1);
        assert_eq!(v2, 2);
        assert_eq!(v3, 3);
    }

    #[test]
    fn test_push_pop_different_types() {
        let mut stack = RawStack::new();
        stack.push(42u32);
        stack.push(3.14f64);
        let value_f: f64 = unsafe { stack.pop() };
        let value_u: u32 = unsafe { stack.pop() };
        assert_eq!(value_u, 42);
        assert!((value_f - 3.14).abs() < 1e-6);
    }
}
