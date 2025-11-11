use crate::memory::align_index;
use std::cmp::max;
use std::mem::MaybeUninit;
use std::ops::{Index, IndexMut};
use std::slice::SliceIndex;

/// A vector of bytes aligned to a given value. The alignment can be increase by calling `align`.
#[derive(Debug)]
pub struct RawVec {
    buffer: Vec<MaybeUninit<u8>>,
    base_alignment: usize,
    start_offset: usize,
}

impl<I> Index<I> for RawVec
where
    I: SliceIndex<[MaybeUninit<u8>]>,
{
    type Output = I::Output;

    fn index(&self, index: I) -> &Self::Output {
        let slice = &self.buffer[self.start_offset..];
        &slice[index]
    }
}

impl<I> IndexMut<I> for RawVec
where
    I: SliceIndex<[MaybeUninit<u8>]>,
{
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        let slice = &mut self.buffer[self.start_offset..];
        &mut slice[index]
    }
}

impl RawVec {
    /// Creates a new `RawVec` with base alignment.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::RawVec;
    /// use std::mem::align_of;
    /// let vec = RawVec::with_base_alignment(align_of::<u32>());
    /// ```
    #[must_use]
    pub fn with_base_alignment(base_alignment: usize) -> Self {
        RawVec {
            base_alignment,
            start_offset: 0,
            buffer: Vec::new(),
        }
    }

    /// Creates a new `RawVec` with base alignment and initial capacity.
    #[must_use]
    pub fn with_base_alignment_and_capacity(base_alignment: usize, capacity: usize) -> Self {
        let mut buffer = Vec::with_capacity(capacity + base_alignment - 1);
        let ptr_as_index = buffer.as_ptr() as usize;
        let start_offset = align_index(base_alignment, ptr_as_index) - ptr_as_index;
        unsafe { buffer.set_len(start_offset) };
        RawVec {
            buffer,
            base_alignment,
            start_offset,
        }
    }

    /// Returns the capacity of the vector in bytes.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.buffer.capacity() - self.start_offset
    }

    /// Returns the current length of the vector in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffer.len() - self.start_offset
    }

    /// Returns true if the vector contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reserves capacity for at least additional more elements. The collection may reserve more
    /// space to guarantee amortized constant time growth. After calling reserve, capacity will be
    /// greater than or equal to `self.len() + additional`. Does nothing if capacity is already
    /// sufficient.
    ///
    /// # Panics
    ///
    /// Panics if the new capacity exceeds `isize::MAX bytes`.
    pub fn reserve(&mut self, addition: usize) {
        let capacity = self.len() + addition;
        if capacity > self.capacity() {
            let mut new_buffer = Self::with_base_alignment_and_capacity(
                self.base_alignment,
                max(capacity, self.capacity() * 2),
            );
            unsafe {
                std::ptr::copy_nonoverlapping(self.as_ptr(), new_buffer.as_mut_ptr(), self.len());
            }
            *self = new_buffer;
        }
    }

    /// Sets the length of the vector.
    ///
    /// # Panics
    ///
    /// - The length must be less than or equal to the capacity.
    ///
    /// # Safety
    ///
    /// - The elements at `old_len..new_len` must be initialized.
    pub unsafe fn set_len(&mut self, len: usize) {
        assert!(len <= self.capacity());
        unsafe { self.buffer.set_len(self.start_offset + len) };
    }

    /// Returns a raw mutable pointer to the vector's buffer, or a dangling raw pointer valid for
    /// zero sized reads if the vector didn't allocate.
    ///
    /// This method guarantees that for the purpose of the aliasing model, this method does not
    /// materialize a reference to the underlying slice, and thus the returned pointer will remain
    /// valid when mixed with other calls to [`Self::as_ptr`], [`Self::as_mut_ptr`]. Note
    /// that calling other methods that materialize references to the slice, or references to
    /// specific elements you are planning on accessing through this pointer, may still invalidate
    /// this pointer.
    ///
    /// # Safety
    ///
    /// The pointer is valid until the vector buffer is reallocated or the vector's lifetime ends.
    pub unsafe fn as_mut_ptr(&mut self) -> *mut MaybeUninit<u8> {
        unsafe { self.buffer.as_mut_ptr().add(self.start_offset) }
    }

    /// Returns a raw pointer to the vector's buffer, or a dangling raw pointer valid for zero sized
    /// reads if the vector didn't allocate.
    ///
    /// The caller must also ensure that the memory the pointer (non-transitively) points to is
    /// never written to (except inside an `UnsafeCell`) using this pointer or any pointer derived
    /// from it. If you need to mutate the contents of the slice, use [`Self::as_mut_ptr`].
    ///
    /// This method guarantees that for the purpose of the aliasing model, this method does not
    /// materialize a reference to the underlying slice, and thus the returned pointer will remain
    /// valid when mixed with other calls to [`Self::as_ptr`], [`Self::as_mut_ptr`]. Note
    /// that calling other methods that materialize mutable references to the slice, or mutable
    /// references to specific elements you are planning on accessing through this pointer, as well
    /// as writing to those elements, may still invalidate this pointer.
    ///
    /// # Safety
    ///
    /// The pointer is valid until the vector buffer is reallocated or the vector's lifetime ends.
    #[must_use]
    pub unsafe fn as_ptr(&self) -> *const MaybeUninit<u8> {
        unsafe { self.buffer.as_ptr().add(self.start_offset) }
    }

    /// Shortens the vector, keeping the first `len` elements and dropping
    /// the rest.
    ///
    /// If `len` is greater or equal to the vector's current length, this has
    /// no effect.
    pub fn truncate(&mut self, len: usize) {
        self.buffer.truncate(self.start_offset + len);
    }
}

/* Test module */
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_base_alignment() {
        let vec = RawVec::with_base_alignment(align_of::<u32>());
        assert_eq!(vec.capacity(), 0);
        assert_eq!(vec.len(), 0);
    }

    #[test]
    fn with_base_alignment_and_capacity() {
        let vec = RawVec::with_base_alignment_and_capacity(align_of::<u32>(), 10);
        assert!(vec.capacity() >= 10);
        assert_eq!(vec.len(), 0);
        assert_eq!(unsafe { vec.as_ptr() as usize } % align_of::<u32>(), 0);
    }

    #[test]
    fn reserve() {
        let mut vec = RawVec::with_base_alignment(align_of::<u32>());
        vec.reserve(10);
        assert!(vec.capacity() >= 10);
        assert_eq!(vec.len(), 0);
        assert_eq!(unsafe { vec.as_ptr() as usize } % align_of::<u32>(), 0);
    }

    #[test]
    fn set_len() {
        let mut vec = RawVec::with_base_alignment_and_capacity(align_of::<u32>(), 10);
        unsafe { vec.set_len(10) };
        assert_eq!(vec.len(), 10);
        assert_eq!(unsafe { vec.as_ptr() as usize } % align_of::<u32>(), 0);
    }

    #[test]
    fn index() {
        let mut vec = RawVec::with_base_alignment_and_capacity(align_of::<u32>(), 10);
        unsafe { vec.set_len(1) };
        vec[0].write(42);
        assert_eq!(unsafe { vec[0].assume_init() }, 42);
    }
}
