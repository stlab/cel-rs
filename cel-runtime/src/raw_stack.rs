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
    /// use cel_runtime::raw_stack::RawStack;
    /// let stack = RawStack::with_base_alignment(align_of::<u32>());
    /// ```
    #[must_use]
    pub fn with_base_alignment(base_alignment: usize) -> Self {
        RawStack {
            buffer: RawVec::with_base_alignment(base_alignment),
        }
    }

    /// Returns the number of bytes currently on the stack.
    #[must_use]
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.buffer.len()
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
    /// use cel_runtime::raw_stack::RawStack;
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

    /// Pushes `size` raw bytes from `src`, aligned to `align`, using the same
    /// padding/marker-byte bookkeeping as [`push`](Self::push).
    ///
    /// - Precondition: `align` is a power of two.
    ///
    /// # Safety
    /// `src` must be valid for reads of `size` bytes.
    pub unsafe fn push_raw(&mut self, align: usize, size: usize, src: *const u8) -> bool {
        debug_assert!(align.is_power_of_two());
        let len = self.buffer.len();
        let aligned_index = align_index(align, len);
        let new_len = aligned_index + size;

        self.buffer.reserve(new_len - len);
        unsafe {
            self.buffer.set_len(new_len);
            if aligned_index - len > 0 {
                self.buffer[len].write(1);
                self.buffer[len + 1..aligned_index].fill(MaybeUninit::new(0));
            }
            std::ptr::copy_nonoverlapping(
                src,
                self.buffer.as_mut_ptr().add(aligned_index).cast::<u8>(),
                size,
            );
        }
        aligned_index - len > 0
    }

    /// Copies `size` bytes starting at absolute buffer offset `offset` into `dst`.
    ///
    /// # Safety
    /// `offset..offset + size` must be within the currently-initialized buffer;
    /// `dst` must be valid for writes of `size` bytes.
    pub unsafe fn copy_from(&self, offset: usize, size: usize, dst: *mut u8) {
        unsafe {
            std::ptr::copy_nonoverlapping(self.buffer.as_ptr().add(offset).cast::<u8>(), dst, size);
        }
    }

    /// Drops a value in place at absolute buffer offset `offset`, without
    /// altering the stack's tracked length.
    ///
    /// # Safety
    /// `offset` must point to a live, valid value; `run_drop` must correctly
    /// run that value's destructor given a pointer to its start.
    pub unsafe fn drop_at(&mut self, offset: usize, run_drop: impl FnOnce(*mut u8)) {
        unsafe { run_drop(self.buffer.as_mut_ptr().add(offset).cast::<u8>()) };
    }

    /// Truncates the stack back to `new_len`, additionally stripping `padding`
    /// bytes that preceded the removed region (scanned the same way
    /// [`pop`](Self::pop) does).
    ///
    /// # Safety
    /// No live (undropped) value may exist at or above `new_len`.
    pub unsafe fn truncate_to(&mut self, new_len: usize, padding: bool) {
        let padding_count = if padding {
            self.buffer[..new_len]
                .iter()
                .rev()
                .take_while(|&x| unsafe { x.assume_init() == 0 })
                .count()
                + 1
        } else {
            0
        };
        self.buffer.truncate(new_len - padding_count);
    }

    /// Drops a value of `size` bytes at the top of the stack in place, then
    /// removes it (and any padding that preceded it).
    ///
    /// # Safety
    /// The top `size` bytes (plus padding if `padding` is true) must be a
    /// live, valid value; `run_drop` must correctly run its destructor given a
    /// pointer to its start.
    pub unsafe fn drop_sized(
        &mut self,
        size: usize,
        padding: bool,
        run_drop: impl FnOnce(*mut u8),
    ) {
        let p = self.buffer.len() - size;
        unsafe {
            self.drop_at(p, run_drop);
            self.truncate_to(p, padding);
        }
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
    /// use cel_runtime::raw_stack::RawStack;
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

    #[test]
    fn len_reflects_pushed_bytes() {
        let mut stack = RawStack::with_base_alignment(align_of::<u32>());
        assert_eq!(stack.len(), 0);
        let _ = stack.push(7u32);
        assert_eq!(stack.len(), size_of::<u32>());
    }

    #[test]
    fn push_raw_round_trips_like_push() {
        let mut stack = RawStack::with_base_alignment(align_of::<f64>());
        let padding1 = stack.push(1u8);
        let value = 3.14f64;
        let padding2 = unsafe {
            stack.push_raw(
                align_of::<f64>(),
                size_of::<f64>(),
                (&value as *const f64).cast::<u8>(),
            )
        };
        let popped: f64 = unsafe { stack.pop(padding2) };
        assert_eq!(popped, 3.14);
        let popped_u8: u8 = unsafe { stack.pop(padding1) };
        assert_eq!(popped_u8, 1);
    }

    #[test]
    fn push_raw_padding_matches_typed_push() {
        let mut stack_a = RawStack::with_base_alignment(align_of::<f64>());
        let _ = stack_a.push(1u8);
        let padding_typed = stack_a.push(2.5f64);

        let mut stack_b = RawStack::with_base_alignment(align_of::<f64>());
        let _ = stack_b.push(1u8);
        let value = 2.5f64;
        let padding_raw = unsafe {
            stack_b.push_raw(
                align_of::<f64>(),
                size_of::<f64>(),
                (&value as *const f64).cast::<u8>(),
            )
        };
        assert_eq!(padding_typed, padding_raw);
    }

    #[test]
    fn copy_from_reads_bytes_at_offset() {
        let mut stack = RawStack::with_base_alignment(align_of::<u32>());
        let _ = stack.push(10u32);
        let _ = stack.push(20u32);
        let mut buf = [0u8; 4];
        unsafe { stack.copy_from(0, 4, buf.as_mut_ptr()) };
        assert_eq!(u32::from_ne_bytes(buf), 10);
    }

    #[test]
    fn drop_at_runs_destructor_without_changing_length() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct DropCounter(Arc<AtomicUsize>);
        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let count = Arc::new(AtomicUsize::new(0));
        let mut stack = RawStack::with_base_alignment(align_of::<DropCounter>());
        let _ = stack.push(DropCounter(count.clone()));
        let len_before = stack.len();
        unsafe {
            stack.drop_at(0, |ptr| unsafe {
                std::ptr::drop_in_place(ptr.cast::<DropCounter>())
            });
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(stack.len(), len_before);
    }

    #[test]
    fn truncate_to_strips_recorded_padding() {
        let mut stack = RawStack::with_base_alignment(align_of::<f64>());
        let _ = stack.push(1u8);
        let padding = stack.push(2.5f64); // padding == true: 7 bytes inserted before the f64
        let len_with_value = stack.len();
        unsafe { stack.truncate_to(len_with_value - size_of::<f64>(), padding) };
        assert_eq!(stack.len(), 1); // back to just the u8
    }

    #[test]
    fn drop_sized_combines_drop_at_and_truncate_to() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct DropCounter(Arc<AtomicUsize>);
        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let count = Arc::new(AtomicUsize::new(0));
        let mut stack = RawStack::with_base_alignment(align_of::<DropCounter>());
        let _ = stack.push(1u8);
        let padding = stack.push(DropCounter(count.clone()));
        unsafe {
            stack.drop_sized(size_of::<DropCounter>(), padding, |ptr| unsafe {
                std::ptr::drop_in_place(ptr.cast::<DropCounter>())
            });
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(stack.len(), 1);
    }
}
