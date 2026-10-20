//! Concrete, contiguous ownership for homogeneous CEL arrays.
//!
//! Transfers to and from vectors preserve their allocations. Nested arrays use
//! `Array<Array<T>>`, so each outer element is itself a typed array owner.
//! Its outer allocation transfers to `Vec<Array<T>>`, not `Vec<Vec<T>>`;
//! explicitly converting each inner owner builds a new outer vector. Borrowing
//! exposes `&[Array<T>]` without consuming the outer or inner owners.
//!
//! ```
//! use cel_runtime::Array;
//!
//! let rows = Array::from_vec(vec![Array::from_vec(vec![1_i32, 2])]);
//! assert_eq!(rows.as_slice()[0].as_slice(), &[1, 2]);
//! ```

use std::fmt;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ptr::NonNull;

/// Stores an array's pointer, element count, and element capacity, in that order.
///
/// The `repr(C)` header contains no descriptor and has no destructor. The pointer
/// is non-null and aligned for the concrete element type, including empty and
/// zero-sized arrays. Length and capacity count elements, not bytes; length does
/// not exceed capacity. The first `len` elements are initialized. For non-zero-sized
/// elements with nonzero capacity, the pointer uniquely owns a global-allocator
/// allocation compatible with `Vec<T>` at exactly this capacity and element type.
///
/// A typed owner or a future erased descriptor must supply the concrete element
/// type and destroy the elements and allocation exactly once. This layout is
/// defined independently of Rust's unspecified `Vec<T>` header layout.
#[repr(C)]
pub(crate) struct RawArray {
    pub(crate) ptr: NonNull<()>,
    pub(crate) len: usize,
    pub(crate) capacity: usize,
}

/// Owns homogeneous, contiguous elements without bounds on the element type.
///
/// The transparent representation is a pointer/length/capacity `RawArray`
/// header, with no descriptor. Its elements have the concrete layout of `T`.
/// Nested arrays use `Array<Array<T>>`, not nested vectors.
///
/// Dropping an array destroys its elements exactly once in reverse index order,
/// then releases its allocation. Remaining elements also receive reverse-order
/// cleanup if a destructor unwinds; a second destructor panic during unwinding
/// aborts as usual. After [`Self::into_vec`], the vector instead follows Rust's
/// vector destruction behavior.
///
/// - Complexity: Destruction takes O(n) time and O(1) auxiliary space, excluding
///   element destructor costs.
///
/// # Examples
///
/// ```
/// use cel_runtime::Array;
///
/// let array = Array::from_vec(vec![String::from("leaf")]);
/// assert_eq!(array.as_slice(), &["leaf"]);
/// ```
///
/// Elements that are not `Send` cannot become transferable through an array:
///
/// ```compile_fail
/// use cel_runtime::Array;
/// use std::rc::Rc;
///
/// fn require_send<T: Send>() {}
/// require_send::<Array<Rc<()>>>();
/// ```
///
/// Elements that are not `Sync` cannot become shareable through an array:
///
/// ```compile_fail
/// use cel_runtime::Array;
/// use std::cell::Cell;
///
/// fn require_sync<T: Sync>() {}
/// require_sync::<Array<Cell<i32>>>();
/// ```
#[repr(transparent)]
pub struct Array<T> {
    pub(crate) raw: RawArray,
    marker: PhantomData<T>,
}

// SAFETY: The header uniquely owns T elements and never exposes untyped access
// publicly. Moving ownership between threads is valid exactly when T is Send.
unsafe impl<T: Send> Send for Array<T> {}
// SAFETY: Shared access exposes only &[T], which is shareable exactly when T is Sync.
unsafe impl<T: Sync> Sync for Array<T> {}

impl<T> Array<T> {
    /// Takes ownership of a vector without moving elements or allocating.
    ///
    /// - Postcondition: Contents, allocation pointer, length, and capacity are
    ///   unchanged. Empty and zero-sized vectors retain their raw parts.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::Array;
    /// let values = vec![3_i32, 5];
    /// let pointer = values.as_ptr();
    /// let array = Array::from_vec(values);
    /// assert_eq!(array.as_slice().as_ptr(), pointer);
    /// ```
    #[must_use]
    pub fn from_vec(values: Vec<T>) -> Self {
        let (ptr, len, capacity) = values.into_raw_parts();
        Self {
            raw: RawArray {
                // SAFETY: Vec raw parts always provide a non-null aligned pointer.
                ptr: unsafe { NonNull::new_unchecked(ptr.cast()) },
                len,
                capacity,
            },
            marker: PhantomData,
        }
    }

    /// Transfers ownership into a vector without moving elements or allocating.
    ///
    /// - Postcondition: Contents, allocation pointer, length, and capacity are
    ///   unchanged; this array runs no element destructors.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::Array;
    /// let array = Array::from_vec(vec![3_i32, 5]);
    /// let pointer = array.as_slice().as_ptr();
    /// let values = array.into_vec();
    /// assert_eq!(values.as_ptr(), pointer);
    /// assert_eq!(values, [3, 5]);
    /// ```
    #[must_use]
    pub fn into_vec(self) -> Vec<T> {
        let owner = ManuallyDrop::new(self);
        // SAFETY: The header preserves Vec<T>'s raw parts, and ManuallyDrop
        // prevents the old owner from destroying the transferred allocation.
        unsafe {
            Vec::from_raw_parts(
                owner.raw.ptr.cast::<T>().as_ptr(),
                owner.raw.len,
                owner.raw.capacity,
            )
        }
    }

    /// Borrows the contiguous elements without copying or allocating.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::Array;
    /// let array = Array::from_vec(vec![3_i32, 5]);
    /// let values: &[i32] = array.as_slice();
    /// assert_eq!(values, &[3, 5]);
    /// ```
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        // SAFETY: The typed owner guarantees an aligned non-null pointer and
        // len initialized T elements, alive for the lifetime of this borrow.
        unsafe { std::slice::from_raw_parts(self.raw.ptr.cast::<T>().as_ptr(), self.raw.len) }
    }

    /// Returns the number of initialized elements.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::Array;
    /// assert_eq!(Array::from_vec(vec![3_i32, 5]).len(), 2);
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.raw.len
    }

    /// Returns the allocation capacity in elements, as reported by the source vector.
    ///
    /// Zero-sized elements retain the vector's capacity of `usize::MAX`.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::Array;
    /// let values = Vec::<i32>::with_capacity(8);
    /// let capacity = values.capacity();
    /// assert_eq!(Array::from_vec(values).capacity(), capacity);
    /// ```
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.raw.capacity
    }

    /// Returns whether the array contains no elements.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::Array;
    /// assert!(Array::from_vec(Vec::<i32>::new()).is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.raw.len == 0
    }
}

impl<T> Drop for Array<T> {
    /// Destroys elements in reverse order and frees storage in O(n) time.
    fn drop(&mut self) {
        // SAFETY: This owner uniquely holds the original Vec<T> raw parts and
        // no longer accesses them after the reconstructed vector is destroyed.
        let mut guard = ReverseDrop(unsafe {
            Vec::from_raw_parts(
                self.raw.ptr.cast::<T>().as_ptr(),
                self.raw.len,
                self.raw.capacity,
            )
        });
        // The guard resumes reverse cleanup if an element destructor unwinds.
        while let Some(value) = guard.0.pop() {
            drop(value);
        }
    }
}

/// Owns remaining elements during cleanup and frees storage even during unwinding.
struct ReverseDrop<T>(Vec<T>);

impl<T> Drop for ReverseDrop<T> {
    /// Drops remaining elements in reverse order in O(n) time and O(1) space.
    fn drop(&mut self) {
        while let Some(value) = self.0.pop() {
            drop(value);
        }
    }
}

impl<T: Clone> Clone for Array<T> {
    /// Clones elements into a new array without sharing owned storage.
    ///
    /// - Complexity: O(n) time and space, excluding element clone costs.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::Array;
    /// let array = Array::from_vec(vec![String::from("leaf")]);
    /// assert_eq!(array.clone().as_slice(), array.as_slice());
    /// ```
    fn clone(&self) -> Self {
        Self::from_vec(self.as_slice().to_vec())
    }
}

impl<T: PartialEq> PartialEq for Array<T> {
    /// Compares lengths and corresponding elements using element equality.
    ///
    /// - Complexity: O(n) time in the worst case, excluding element comparison
    ///   costs, and O(1) auxiliary space.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::Array;
    /// assert_eq!(Array::from_vec(vec![1_i32]), Array::from_vec(vec![1_i32]));
    /// ```
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<T: fmt::Debug> fmt::Debug for Array<T> {
    /// Formats the array as `Array([elements])` using element debug formatting.
    ///
    /// - Complexity: O(n) element formatting calls, excluding element formatting
    ///   costs and output storage.
    ///
    /// # Errors
    ///
    /// Returns an error if the formatter fails to write the representation.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::Array;
    /// assert_eq!(format!("{:?}", Array::from_vec(vec![1_i32])), "Array([1])");
    /// ```
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Array")
            .field(&self.as_slice())
            .finish()
    }
}
