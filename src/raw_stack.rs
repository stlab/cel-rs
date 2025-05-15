use crate::memory::align_index;
use crate::raw_vec::RawVec;
use std::mem::MaybeUninit;
use std::mem::size_of;

/// A simple raw stack that stores values as raw bytes. Each value is naturally aligned given the
/// base alignment of the stack, which is the maximum alignment of any value stored in the stack.
#[derive(Debug)]
pub struct RawStack {
    buffer: RawVec,
}

impl RawStack {
    /// Creates a new `RawStack` with base alignment.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_rs::raw_stack::RawStack;
    /// let stack = RawStack::with_base_alignment(align_of::<u32>());
    /// ```
    #[must_use]
    pub fn with_base_alignment(base_alignment: usize) -> Self {
        RawStack {
            buffer: RawVec::with_base_alignment(base_alignment),
        }
    }

    /// Pushes a value of type `T` onto the stack.
    ///
    /// The value is stored as raw bytes in the internal buffer. The pushed value must be
    /// later popped using the correct type.
    ///
    /// # Type Parameters
    ///
    /// * `T`: The type of the value to push.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_rs::raw_stack::RawStack;
    /// let mut stack = RawStack::with_base_alignment(align_of::<u32>());
    /// let _ = stack.push(42u32);
    /// ```
    ///
    /// # Complexity
    ///
    /// The function has an amortized O(1) time complexity.
    pub fn push<T>(&mut self, value: T) -> bool {
        let len = self.buffer.len();
        let aligned_index = align_index(align_of::<T>(), len);
        let new_len = aligned_index + size_of::<T>();

        self.buffer.reserve(new_len - len);
        unsafe {
            self.buffer.set_len(new_len);
            if aligned_index - len > 0 {
                // write a 1 in the first padding byte and 0 in the rest
                self.buffer[len].write(1);
                self.buffer[len + 1..aligned_index].fill(MaybeUninit::new(0));
            }

            std::ptr::write(
                self.buffer.as_mut_ptr().add(aligned_index).cast::<T>(),
                value,
            );
        }
        aligned_index - len > 0
    }

    /// Pops a value of type `T` from the stack. Does not change the stack capacity.
    ///
    /// # Safety
    ///
    /// The type `T` must be the same type as the value on the top of the stack.
    /// Incorrect usage can lead to undefined behavior.
    ///
    /// # Type Parameters
    ///
    /// * `T`: The type of the value to pop.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_rs::raw_stack::RawStack;
    /// let mut stack = RawStack::with_base_alignment(align_of::<u32>());
    /// let padding = stack.push(100u32);
    /// let value: u32 = unsafe { stack.pop(padding) };
    /// ```
    pub unsafe fn pop<T>(&mut self, padding: bool) -> T {
        let p: usize = self.buffer.len() - size_of::<T>();
        let result = unsafe { std::ptr::read(self.buffer.as_ptr().add(p).cast::<T>()) };
        // count the number of trailing 0s in the buffer before the result
        let padding_count = if padding {
            self.buffer[..p]
                .iter()
                .rev()
                .take_while(|&x| unsafe { x.assume_init() == 0 })
                .count()
                + 1
        } else {
            0
        };
        self.buffer.truncate(p - padding_count);
        result
    }

    /// Pops a value of type `T` from the stack and drops it.
    ///
    /// # Safety
    ///
    /// The type `T` must be the same type as the value on the top of the stack.
    /// Incorrect usage can lead to undefined behavior.
    ///
    /// # Note
    ///
    /// This cannot use `drop_in_place` because the type may not be aligned.
    pub unsafe fn drop<T>(&mut self, padding: bool) {
        unsafe { self.pop::<T>(padding) };
    }
}

/* Test module */
#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::max;

    #[test]
    fn push_pop_u32() {
        let mut stack = RawStack::with_base_alignment(align_of::<u32>());
        let padding = stack.push(10u32);
        let result: u32 = unsafe { stack.pop(padding) };
        assert_eq!(result, 10);
    }

    #[test]
    fn multiple_push_pop() {
        let mut stack = RawStack::with_base_alignment(align_of::<u32>());
        let padding1 = stack.push(1u32);
        let padding2 = stack.push(2u32);
        let padding3 = stack.push(3u32);
        let v3: u32 = unsafe { stack.pop(padding3) };
        let v2: u32 = unsafe { stack.pop(padding2) };
        let v1: u32 = unsafe { stack.pop(padding1) };
        assert_eq!(v1, 1);
        assert_eq!(v2, 2);
        assert_eq!(v3, 3);
    }

    #[test]
    fn push_pop_different_types() {
        let mut stack = RawStack::with_base_alignment(max(align_of::<u32>(), align_of::<f64>()));
        let padding1 = stack.push(42u32);
        let padding2 = stack.push(3.14f64);
        let value_f: f64 = unsafe { stack.pop(padding2) };
        let value_u: u32 = unsafe { stack.pop(padding1) };
        assert_eq!(value_f, 3.14);
        assert_eq!(value_u, 42);
    }
}
