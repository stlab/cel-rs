//! Aligned byte storage without ownership of values placed in its buffer.
//!
//! Reserve storage before writing bytes, then expose them with [`RawVec::set_len`].
//! The caller remains responsible for destroying any values stored in the bytes.

use crate::memory::align_index;
use std::cmp::max;
use std::mem::MaybeUninit;
use std::ops::{Index, IndexMut};
use std::slice::SliceIndex;

/// Stores bytes at a fixed power-of-two alignment after allocation.
///
/// The logical length excludes leading alignment padding. Moving or reserving
/// storage preserves the logical bytes but invalidates pointers into the buffer.
///
/// # Examples
/// ```
/// use cel_runtime::RawVec;
/// let mut bytes = RawVec::with_base_alignment(64);
/// bytes.reserve(8);
/// assert!(bytes.capacity() >= 8);
/// assert!(bytes.is_empty());
/// ```
#[derive(Debug)]
pub struct RawVec {
    /// Owns raw storage including bytes preceding its aligned logical origin.
    buffer: Vec<MaybeUninit<u8>>,
    /// Retains the nonzero power-of-two alignment required at the logical origin.
    base_alignment: usize,
    /// Locates the aligned logical origin within the allocation.
    start_offset: usize,
}

impl<I> Index<I> for RawVec
where
    I: SliceIndex<[MaybeUninit<u8>]>,
{
    type Output = I::Output;

    /// Borrows bytes at an index within the logical length.
    fn index(&self, index: I) -> &Self::Output {
        let slice = &self.buffer[self.start_offset..];
        &slice[index]
    }
}

impl<I> IndexMut<I> for RawVec
where
    I: SliceIndex<[MaybeUninit<u8>]>,
{
    /// Mutably borrows bytes at an index within the logical length.
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        let slice = &mut self.buffer[self.start_offset..];
        &mut slice[index]
    }
}

impl RawVec {
    /// Creates a new `RawVec` with base alignment.
    ///
    /// - Precondition: `base_alignment` is a power of two.
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
        assert!(base_alignment.is_power_of_two());
        RawVec {
            base_alignment,
            start_offset: 0,
            buffer: Vec::new(),
        }
    }

    /// Creates a new `RawVec` with base alignment and initial capacity.
    ///
    /// - Precondition: `base_alignment` is a power of two.
    /// - Complexity: Allocates O(`capacity + base_alignment`) bytes.
    ///
    /// # Examples
    /// ```
    /// let bytes = cel_runtime::RawVec::with_base_alignment_and_capacity(64, 8);
    /// assert!(bytes.capacity() >= 8);
    /// assert!(bytes.is_empty());
    /// ```
    #[must_use]
    pub fn with_base_alignment_and_capacity(base_alignment: usize, capacity: usize) -> Self {
        assert!(base_alignment.is_power_of_two());
        let allocation_size = capacity
            .checked_add(base_alignment.checked_sub(1).expect("invalid alignment"))
            .expect("capacity overflow");
        let mut buffer = Vec::with_capacity(allocation_size);
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
    ///
    /// # Examples
    /// ```
    /// let bytes = cel_runtime::RawVec::with_base_alignment_and_capacity(8, 16);
    /// assert!(bytes.capacity() >= 16);
    /// ```
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.buffer.capacity() - self.start_offset
    }

    /// Returns the alignment guaranteed after allocating storage.
    pub(crate) fn base_alignment(&self) -> usize {
        self.base_alignment
    }

    /// Returns the current length of the vector in bytes.
    ///
    /// # Examples
    /// ```
    /// let bytes = cel_runtime::RawVec::with_base_alignment(8);
    /// assert_eq!(bytes.len(), 0);
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffer.len() - self.start_offset
    }

    /// Returns true if the vector contains no bytes.
    ///
    /// # Examples
    /// ```
    /// assert!(cel_runtime::RawVec::with_base_alignment(8).is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reserves capacity for at least additional more elements. The collection may reserve more
    /// space to guarantee amortized constant time growth. After calling reserve, capacity will be
    /// greater than or equal to `self.len() + additional`. Does nothing if capacity is already
    /// sufficient.
    ///
    /// - Postcondition: The logical length and existing bytes remain unchanged.
    /// - Complexity: O(`self.len()`) time when growing; allocates O(new capacity
    ///   + base alignment) bytes.
    ///
    /// # Panics
    ///
    /// Panics if the new capacity exceeds `isize::MAX` bytes.
    ///
    /// # Examples
    /// ```
    /// let mut bytes = cel_runtime::RawVec::with_base_alignment(8);
    /// bytes.reserve(16);
    /// assert!(bytes.capacity() >= 16);
    /// assert!(bytes.is_empty());
    /// ```
    pub fn reserve(&mut self, addition: usize) {
        let capacity = self.len().checked_add(addition).expect("capacity overflow");
        if capacity > self.capacity() {
            let mut new_buffer = Self::with_base_alignment_and_capacity(
                self.base_alignment,
                max(capacity, self.capacity().saturating_mul(2)),
            );
            unsafe {
                std::ptr::copy_nonoverlapping(self.as_ptr(), new_buffer.as_mut_ptr(), self.len());
                new_buffer.set_len(self.len());
            }
            *self = new_buffer;
        }
    }

    /// Sets the length of the vector.
    ///
    /// # Safety
    ///
    /// `len` must not exceed the capacity. The elements are `MaybeUninit<u8>`;
    /// their payload bytes need not be initialized.
    ///
    /// # Examples
    /// ```
    /// let mut bytes = cel_runtime::RawVec::with_base_alignment_and_capacity(8, 1);
    /// unsafe { bytes.set_len(1) };
    /// bytes[0].write(42);
    /// assert_eq!(unsafe { bytes[0].assume_init() }, 42);
    /// ```
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
    ///
    /// # Examples
    /// ```
    /// let mut bytes = cel_runtime::RawVec::with_base_alignment_and_capacity(64, 1);
    /// assert_eq!(unsafe { bytes.as_mut_ptr() } as usize % 64, 0);
    /// ```
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
    ///
    /// # Examples
    /// ```
    /// let bytes = cel_runtime::RawVec::with_base_alignment_and_capacity(64, 1);
    /// assert_eq!(unsafe { bytes.as_ptr() } as usize % 64, 0);
    /// ```
    #[must_use]
    pub unsafe fn as_ptr(&self) -> *const MaybeUninit<u8> {
        unsafe { self.buffer.as_ptr().add(self.start_offset) }
    }

    /// Shortens the vector, keeping the first `len` elements and dropping
    /// the rest.
    ///
    /// If `len` is greater or equal to the vector's current length, this has
    /// no effect.
    ///
    /// # Examples
    /// ```
    /// let mut bytes = cel_runtime::RawVec::with_base_alignment_and_capacity(8, 2);
    /// unsafe { bytes.set_len(2) };
    /// bytes.truncate(1);
    /// assert_eq!(bytes.len(), 1);
    /// ```
    pub fn truncate(&mut self, len: usize) {
        if len < self.len() {
            self.buffer.truncate(self.start_offset + len);
        }
    }
}

/* Test module */
#[cfg(test)]
mod tests {
    use super::*;

    /// Preserves the logical length and initialized bytes when capacity grows.
    #[test]
    fn reserve_preserves_contents() {
        let mut vec = RawVec::with_base_alignment_and_capacity(64, 3);
        unsafe { vec.set_len(3) };
        for (byte, value) in vec[..].iter_mut().zip([11, 22, 33]) {
            byte.write(value);
        }
        let addition = vec.capacity() + 1;
        vec.reserve(addition);
        assert_eq!(vec.len(), 3);
        assert!(vec.capacity() >= 3 + addition);
        assert_eq!(unsafe { vec.as_ptr() as usize } % 64, 0);
        assert_eq!(
            vec[..]
                .iter()
                .map(|byte| unsafe { byte.assume_init() })
                .collect::<Vec<_>>(),
            [11, 22, 33]
        );
    }

    /// Leaves the logical bytes unchanged when the requested length is larger.
    #[test]
    fn truncate_large_length_is_noop() {
        let mut vec = RawVec::with_base_alignment_and_capacity(4096, 1);
        unsafe { vec.set_len(1) };
        vec[0].write(42);
        vec.truncate(usize::MAX);
        assert_eq!(vec.len(), 1);
        assert_eq!(unsafe { vec[0].assume_init() }, 42);
    }

    /// Creates empty, unallocated storage.
    #[test]
    fn with_base_alignment() {
        let vec = RawVec::with_base_alignment(align_of::<u32>());
        assert_eq!(vec.base_alignment(), align_of::<u32>());
        assert_eq!(vec.capacity(), 0);
        assert_eq!(vec.len(), 0);
    }

    /// Reserves aligned storage without exposing logical bytes.
    #[test]
    fn with_base_alignment_and_capacity() {
        let vec = RawVec::with_base_alignment_and_capacity(align_of::<u32>(), 10);
        assert!(vec.capacity() >= 10);
        assert_eq!(vec.len(), 0);
        assert_eq!(unsafe { vec.as_ptr() as usize } % align_of::<u32>(), 0);
    }

    /// Grows empty storage while preserving its alignment.
    #[test]
    fn reserve() {
        let mut vec = RawVec::with_base_alignment(align_of::<u32>());
        vec.reserve(10);
        assert!(vec.capacity() >= 10);
        assert_eq!(vec.len(), 0);
        assert_eq!(unsafe { vec.as_ptr() as usize } % align_of::<u32>(), 0);
    }

    /// Exposes the requested number of bytes within capacity.
    #[test]
    fn set_len() {
        let mut vec = RawVec::with_base_alignment_and_capacity(align_of::<u32>(), 10);
        unsafe { vec.set_len(10) };
        assert_eq!(vec.len(), 10);
        assert_eq!(unsafe { vec.as_ptr() as usize } % align_of::<u32>(), 0);
    }

    /// Reads and writes logical bytes through indexing.
    #[test]
    fn index() {
        let mut vec = RawVec::with_base_alignment_and_capacity(align_of::<u32>(), 10);
        unsafe { vec.set_len(1) };
        vec[0].write(42);
        assert_eq!(unsafe { vec[0].assume_init() }, 42);
    }
}
