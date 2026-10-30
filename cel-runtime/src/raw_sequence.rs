//! Stores heterogeneous values whose caller tracks types and destruction.
//!
//! Push values in order, then traverse or destroy them using matching types.
//! Dropping the sequence releases its byte buffer, not the stored values.

use crate::memory::align_index;
use crate::raw_vec::RawVec;
use std::mem;

/// A sequence that stores heterogeneous values with proper alignment.
///
/// The `RawSequence` provides a memory-efficient way to store heterogeneous values
/// while maintaining proper alignment requirements for each type. It uses an
/// internal buffer that raises its base alignment to satisfy each stored type.
///
/// The caller owns destruction of stored values, including before discarding
/// the sequence. Zero-sized values occupy no logical bytes.
///
/// # Examples
/// ```
/// let mut values = cel_runtime::RawSequence::new();
/// values.push(String::from("hello"));
/// assert_eq!(unsafe { values.next::<String>(0) }.0, "hello");
/// unsafe { values.drop_in_place::<String>(0) };
/// ```
pub struct RawSequence {
    /// Owns aligned value bytes; callers retain responsibility for typed destruction.
    buffer: RawVec,
}

impl Default for RawSequence {
    /// Creates an empty sequence.
    fn default() -> Self {
        Self::new()
    }
}

impl RawSequence {
    /// Creates a new empty `RawSequence`.
    /// The sequence initially requests 4096-byte base alignment and raises it
    /// when a stored type requires a wider alignment.
    ///
    /// # Examples
    /// ```
    /// let values = cel_runtime::RawSequence::new();
    /// assert!(values.is_empty());
    /// ```
    #[must_use]
    pub fn new() -> Self {
        RawSequence {
            buffer: RawVec::with_base_alignment(4096),
        }
    }

    /// Pushes a value onto the sequence.
    ///
    /// The value is stored at an address that satisfies its alignment requirements.
    /// Automatically grows the internal buffer if needed.
    ///
    /// - Complexity: Amortized O(size of `T` + alignment of `T`) time and space
    ///   at an existing base alignment. Raising the base alignment also moves
    ///   O(n) stored bytes and replaces the buffer allocation.
    ///
    /// # Examples
    /// ```
    /// let mut values = cel_runtime::RawSequence::new();
    /// values.push(42u32);
    /// assert_eq!(*unsafe { values.next::<u32>(0) }.0, 42);
    /// ```
    pub fn push<T>(&mut self, value: T) {
        let len = self.buffer.len();
        if mem::align_of::<T>() > self.buffer.base_alignment() {
            let mut buffer = RawVec::with_base_alignment_and_capacity(
                mem::align_of::<T>(),
                self.buffer.capacity(),
            );
            // SAFETY: Both allocations contain at least len bytes, and copying
            // their representations transfers the caller-managed stored owners.
            unsafe {
                std::ptr::copy_nonoverlapping(self.buffer.as_ptr(), buffer.as_mut_ptr(), len);
                buffer.set_len(len);
            }
            self.buffer = buffer;
        }
        let aligned: usize = align_index(mem::align_of::<T>(), len);
        let new_len = aligned
            .checked_add(mem::size_of::<T>())
            .expect("sequence size overflow");

        self.buffer.reserve((new_len - len).max(1));
        unsafe {
            self.buffer.set_len(new_len);
            std::ptr::write(self.buffer.as_mut_ptr().add(aligned).cast::<T>(), value);
        }
    }

    /// Destroys the next value after `p` and returns the position after it.
    ///
    /// # Safety
    /// `p` must precede a live stored `T`, with only its alignment padding
    /// between them. The value must not be used or destroyed again.
    ///
    /// # Examples
    /// ```
    /// let mut values = cel_runtime::RawSequence::new();
    /// values.push(String::from("owned"));
    /// unsafe { values.drop_in_place::<String>(0) };
    /// ```
    pub unsafe fn drop_in_place<T>(&mut self, p: usize) -> usize {
        let mut next = p;
        unsafe { self.drop_next::<T>(&mut next) };
        next
    }

    /// Advances `p` past the next value before destroying it.
    ///
    /// - Postcondition: `p` advances even when the value's destructor panics.
    ///
    /// # Safety
    /// `p` must precede a live stored `T`, with only its alignment padding
    /// between them. The value must not be used or destroyed again.
    pub(crate) unsafe fn drop_next<T>(&mut self, p: &mut usize) {
        let aligned = align_index(mem::align_of::<T>(), *p);
        *p = aligned + mem::size_of::<T>();
        unsafe { std::ptr::drop_in_place(self.buffer.as_mut_ptr().add(aligned).cast::<T>()) };
    }

    /// Retrieves a reference to the next value at the specified position.
    ///
    /// # Safety
    /// - The position must point to a valid value of type T
    /// - The caller must ensure that the value is actually of type T
    ///
    /// Returns a tuple containing:
    /// - A reference to the value
    /// - The position immediately after the value
    ///
    /// # Examples
    /// ```
    /// let mut values = cel_runtime::RawSequence::new();
    /// values.push(42u32);
    /// let (value, next) = unsafe { values.next::<u32>(0) };
    /// assert_eq!((*value, next), (42, 4));
    /// ```
    #[must_use]
    pub unsafe fn next<T>(&self, p: usize) -> (&T, usize) {
        let aligned: usize = align_index(mem::align_of::<T>(), p);
        let ptr = unsafe { self.buffer.as_ptr().add(aligned).cast::<T>() };
        unsafe { (&*ptr, aligned + mem::size_of::<T>()) }
    }

    /// Returns the number of bytes in the RawSequence, also referred to as its ‘length’.
    ///
    /// # Examples
    /// ```
    /// let mut values = cel_runtime::RawSequence::new();
    /// values.push(42u32);
    /// assert_eq!(values.len(), 4);
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Returns true if no logical bytes are stored, including only zero-sized values.
    ///
    /// # Examples
    /// ```
    /// let mut values = cel_runtime::RawSequence::new();
    /// values.push(());
    /// assert!(values.is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

#[cfg(test)]
mod tests {
    /// Test module for RawSequence.
    ///
    /// Contains tests that verify:
    /// - Pushing different types of values
    /// - Retrieving values in correct order
    /// - Proper memory cleanup using drop_in_place
    use super::*;

    /// Traverses heterogeneous values and explicitly destroys each value.
    #[test]
    fn sequence_operations() {
        let mut seq = RawSequence::new();

        seq.push(100u32);
        seq.push(200u32);
        seq.push(42.0f64);
        seq.push("Hello, world!");

        let (value, p) = unsafe { seq.next::<u32>(0) };
        assert_eq!(*value, 100);
        let (value, p) = unsafe { seq.next::<u32>(p) };
        assert_eq!(*value, 200);
        let (value, p) = unsafe { seq.next::<f64>(p) };
        assert_eq!(*value, 42.0);
        let (value, _) = unsafe { seq.next::<&str>(p) };
        assert_eq!(*value, "Hello, world!");

        let p = unsafe { seq.drop_in_place::<u32>(0) };
        let p = unsafe { seq.drop_in_place::<u32>(p) };
        let p = unsafe { seq.drop_in_place::<f64>(p) };
        let _ = unsafe { seq.drop_in_place::<&str>(p) };
    }

    /// Provides a correctly aligned address even for the first zero-sized value.
    #[test]
    fn aligned_zero_sized_value() {
        /// Requires alignment without occupying logical bytes.
        #[repr(align(4096))]
        struct Aligned;

        let mut seq = RawSequence::default();
        seq.push(Aligned);
        let (value, next) = unsafe { seq.next::<Aligned>(0) };
        assert_eq!(value as *const Aligned as usize % 4096, 0);
        assert_eq!(next, 0);
        assert!(seq.is_empty());
        unsafe { seq.drop_in_place::<Aligned>(0) };
    }
}
