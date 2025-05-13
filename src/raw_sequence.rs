use crate::memory::align_index;
use crate::raw_vec::RawVec;
// use aligned_vec::{AVec, ConstAlign};
use std::mem;

/**
A sequence that stores heterogeneous values with proper alignment.

The RawSequence provides a memory-efficient way to store heterogeneous values
while maintaining proper alignment requirements for each type. It uses an
internal buffer that aligns values according to their type's requirements,
up to a maximum alignment of 4096 bytes.
*/
pub struct RawSequence {
    buffer: RawVec,
}

impl Default for RawSequence {
    fn default() -> Self {
        Self::new()
    }
}

impl RawSequence {
    /**
    Creates a new empty RawSequence.
    The sequence is initialized with a 4096-byte aligned buffer.
    */
    pub fn new() -> Self {
        RawSequence {
            buffer: RawVec::with_base_alignment(4096),
        }
    }

    /**
    Pushes a value onto the sequence.

    The value is stored at an address that satisfies its alignment requirements.
    Automatically grows the internal buffer if needed.

    # Panics
    Panics if the type's alignment requirement exceeds 4096 bytes.
    */
    pub fn push<T>(&mut self, value: T) {
        assert!(mem::align_of::<T>() <= 4096);
        let len = self.buffer.len();
        let aligned: usize = align_index(mem::align_of::<T>(), len);
        let new_len = aligned + mem::size_of::<T>();

        self.buffer.reserve(new_len - len);
        unsafe {
            self.buffer.set_len(new_len);
            std::ptr::write(self.buffer.as_mut_ptr().add(aligned) as *mut T, value);
        }
    }

    /**
    Drops a value in-place at the specified position.

    # Safety
    - The position must point to a valid value of type T
    - The caller must ensure that the value is actually of type T

    Returns the position immediately after the dropped value.
    */
    pub unsafe fn drop_in_place<T>(&mut self, p: usize) -> usize {
        let aligned: usize = align_index(mem::align_of::<T>(), p);
        unsafe { std::ptr::drop_in_place(self.buffer.as_ptr().add(aligned) as *mut T) };
        aligned + mem::size_of::<T>()
    }

    /**
    Retrieves a reference to the next value at the specified position.

    # Safety
    - The position must point to a valid value of type T
    - The caller must ensure that the value is actually of type T

    Returns a tuple containing:
    - A reference to the value
    - The position immediately after the value
    */
    pub unsafe fn next<T>(&self, p: usize) -> (&T, usize) {
        let aligned: usize = align_index(mem::align_of::<T>(), p);
        let ptr = unsafe { self.buffer.as_ptr().add(aligned) as *const T };
        unsafe { (&*ptr, aligned + mem::size_of::<T>()) }
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

#[cfg(test)]
mod tests {
    /*!
    Test module for RawSequence.

    Contains tests that verify:
    - Pushing different types of values
    - Retrieving values in correct order
    - Proper memory cleanup using drop_in_place
    */
    use super::*;

    #[test]
    fn test_sequence_operations() {
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
}
