//! Owned, homogeneous arrays that erase a `Vec<T>` allocation behind runtime type metadata.
//!
//! A `DynamicArray` preserves the allocation owned by a concrete `Vec<T>` while storing the
//! element type as metadata. Callers can recover the original vector only after requesting the
//! same concrete element type:
//!
//! ```rust
//! use cel_runtime::DynamicArray;
//!
//! let mut values = Vec::with_capacity(4);
//! values.extend([1i32, 2]);
//! let capacity = values.capacity();
//!
//! let array = DynamicArray::try_from_vec(values).unwrap();
//! assert_eq!(array.len(), 2);
//!
//! let values = array.try_into_vec::<i32>().unwrap();
//! assert_eq!(values, vec![1, 2]);
//! assert_eq!(values.capacity(), capacity);
//! ```
//!
//! Borrowed access performs the same runtime type check:
//!
//! ```rust
//! use cel_runtime::DynamicArray;
//!
//! let array = DynamicArray::try_from_vec(vec!["a", "b"]).unwrap();
//! assert_eq!(array.try_as_slice::<&'static str>().unwrap(), &["a", "b"]);
//! ```
//!
//! Mismatched element requests return a type error and never cast the allocation:
//!
//! ```rust
//! use cel_runtime::DynamicArray;
//!
//! let array = DynamicArray::try_from_vec(vec![1i32]).unwrap();
//! assert!(array.try_into_vec::<u32>().is_err());
//! ```
//!
//! An array whose elements are themselves arrays is a recursively typed rank-one array — its
//! concrete element type is `DynamicArray`, and each inner array independently owns a `Vec<T>`
//! allocation. Every element must share the same complete recursive type, so the outer
//! descriptor stays accurate:
//!
//! ```rust
//! use cel_runtime::DynamicArray;
//!
//! let rows = vec![
//!     DynamicArray::try_from_vec(vec![0i32]).unwrap(),
//!     DynamicArray::try_from_vec(vec![1i32]).unwrap(),
//! ];
//! let nested = DynamicArray::try_from_vec(rows).unwrap();
//! let rows = nested.try_into_vec::<DynamicArray>().unwrap();
//! assert_eq!(rows[1].try_as_slice::<i32>().unwrap(), &[1]);
//!
//! // Inner element types must agree.
//! let mixed = vec![
//!     DynamicArray::try_from_vec(vec![0i32]).unwrap(),
//!     DynamicArray::try_from_vec(vec![1.0f64]).unwrap(),
//! ];
//! assert!(DynamicArray::try_from_vec(mixed).is_err());
//! ```
//!
//! `cel-parser` array literals (`[0, 1, 2]`, `[[0], [1]]`) compile to operations that collect
//! their evaluated elements into exactly these values.

use crate::dyn_segment::{RawDropper, raw_dropper_for};
use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::fmt;
use std::mem::{ManuallyDrop, MaybeUninit, align_of, size_of};
use std::ptr::NonNull;
use std::slice;

/// Describes the concrete element type and recursive array metadata for a [`DynamicArray`].
#[derive(Clone)]
pub struct ArrayElementType {
    type_id: TypeId,
    type_name: Cow<'static, str>,
    size: usize,
    align: usize,
    drop: RawDropper,
    nested: Option<Box<ArrayElementType>>,
}

impl ArrayElementType {
    /// Returns the descriptor for a non-array leaf element type.
    ///
    /// # Errors
    /// Returns [`ArrayBuildErrorKind::MissingNestedElementType`] for `DynamicArray`, because an
    /// array element descriptor must include its nested element descriptor.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::ArrayElementType;
    /// use std::any::TypeId;
    ///
    /// let element = ArrayElementType::leaf::<i32>().unwrap();
    /// assert_eq!(element.type_id(), TypeId::of::<i32>());
    /// ```
    pub fn leaf<T: 'static>() -> Result<Self, ArrayBuildErrorKind> {
        if TypeId::of::<T>() == TypeId::of::<DynamicArray>() {
            return Err(ArrayBuildErrorKind::MissingNestedElementType);
        }
        Ok(Self {
            type_id: TypeId::of::<T>(),
            type_name: Cow::Borrowed(std::any::type_name::<T>()),
            size: size_of::<T>(),
            align: align_of::<T>(),
            drop: raw_dropper_for::<T>(),
            nested: None,
        })
    }

    /// Returns the descriptor for a leaf element described by already-erased metadata, for a
    /// caller (such as [`DynSegment::make_array`](crate::DynSegment::make_array)) that knows an
    /// element's layout only at runtime.
    ///
    /// - Precondition: `size`, `align`, and `drop` are those of the single Rust type identified
    ///   by `type_id`.
    ///
    /// # Panics
    ///
    /// Panics, in every build profile, if `type_id` is [`DynamicArray`]'s — an array element
    /// descriptor is built by [`array_of`](Self::array_of) so its nested descriptor travels with
    /// it, and a leaf descriptor claiming the array marker would describe elements as
    /// `DynamicArray` values with no element type of their own (the same malformed state
    /// [`leaf`](Self::leaf) refuses to build). This check is unconditional, matching
    /// [`ValueType::leaf_from_parts`](crate::ValueType::leaf_from_parts), because the resulting
    /// descriptor governs how element bytes are interpreted.
    pub(crate) fn leaf_from_parts(
        type_id: TypeId,
        type_name: Cow<'static, str>,
        size: usize,
        align: usize,
        drop: RawDropper,
    ) -> Self {
        debug_assert!(align.is_power_of_two(), "align must be a power of two");
        debug_assert!(
            size.is_multiple_of(align),
            "size must be a multiple of align"
        );
        assert!(
            type_id != TypeId::of::<DynamicArray>(),
            "a leaf element descriptor must not claim the array marker TypeId"
        );
        Self {
            type_id,
            type_name,
            size,
            align,
            drop,
            nested: None,
        }
    }

    /// Returns the descriptor for a nested `DynamicArray` element.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{ArrayElementType, DynamicArray};
    /// use std::any::TypeId;
    ///
    /// let element = ArrayElementType::array_of(ArrayElementType::leaf::<i32>().unwrap());
    /// assert_eq!(element.type_id(), TypeId::of::<DynamicArray>());
    /// assert_eq!(element.nested().unwrap().type_id(), TypeId::of::<i32>());
    /// ```
    pub fn array_of(element: ArrayElementType) -> Self {
        Self {
            type_id: TypeId::of::<DynamicArray>(),
            type_name: Cow::Borrowed(std::any::type_name::<DynamicArray>()),
            size: size_of::<DynamicArray>(),
            align: align_of::<DynamicArray>(),
            drop: raw_dropper_for::<DynamicArray>(),
            nested: Some(Box::new(element)),
        }
    }

    /// Returns the concrete Rust [`TypeId`] stored by this descriptor.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::ArrayElementType;
    /// use std::any::TypeId;
    ///
    /// let element = ArrayElementType::leaf::<i32>().unwrap();
    /// assert_eq!(element.type_id(), TypeId::of::<i32>());
    /// ```
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Returns the human-readable concrete type name.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::ArrayElementType;
    ///
    /// let element = ArrayElementType::leaf::<i32>().unwrap();
    /// assert_eq!(element.type_name(), "i32");
    /// ```
    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    /// Returns the element size in bytes.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::ArrayElementType;
    ///
    /// let element = ArrayElementType::leaf::<i32>().unwrap();
    /// assert_eq!(element.size(), std::mem::size_of::<i32>());
    /// ```
    pub fn size(&self) -> usize {
        self.size
    }

    /// Returns the element alignment in bytes.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::ArrayElementType;
    ///
    /// let element = ArrayElementType::leaf::<i32>().unwrap();
    /// assert_eq!(element.align(), std::mem::align_of::<i32>());
    /// ```
    pub fn align(&self) -> usize {
        self.align
    }

    /// Returns the nested element descriptor for array elements.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::ArrayElementType;
    ///
    /// let element = ArrayElementType::array_of(ArrayElementType::leaf::<i32>().unwrap());
    /// assert_eq!(element.nested().unwrap().type_name(), "i32");
    /// ```
    pub fn nested(&self) -> Option<&ArrayElementType> {
        self.nested.as_deref()
    }

    /// Returns the recursive name used in type mismatch diagnostics.
    ///
    /// - Complexity: O(depth).
    pub(crate) fn display_name(&self) -> Cow<'static, str> {
        match &self.nested {
            Some(nested) => Cow::Owned(format!("[{}]", nested.display_name())),
            None => self.type_name.clone(),
        }
    }

    /// Returns whether `self` and `other` describe the same recursive element shape and layout.
    ///
    /// This is stricter than [`PartialEq`]: it compares the stored size and alignment in addition
    /// to the recursive type marker, so a runtime-resolved descriptor must agree with the bytes a
    /// collected value actually occupies before the two can be treated as interchangeable.
    ///
    /// - Complexity: O(depth).
    pub(crate) fn same_shape_and_layout(&self, other: &Self) -> bool {
        self.type_id == other.type_id
            && self.size == other.size
            && self.align == other.align
            && match (&self.nested, &other.nested) {
                (Some(left), Some(right)) => left.same_shape_and_layout(right),
                (None, None) => true,
                _ => false,
            }
    }
}

impl PartialEq for ArrayElementType {
    fn eq(&self, other: &Self) -> bool {
        self.type_id == other.type_id && self.nested == other.nested
    }
}

impl Eq for ArrayElementType {}

impl fmt::Debug for ArrayElementType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArrayElementType")
            .field("type_name", &self.type_name)
            .field("size", &self.size)
            .field("align", &self.align)
            .field("nested", &self.nested)
            .finish_non_exhaustive()
    }
}

/// Describes why a concrete vector could not become a [`DynamicArray`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArrayBuildErrorKind {
    /// A `Vec<DynamicArray>` construction did not provide a nested element descriptor.
    MissingNestedElementType,
    /// A nested array element's descriptor differs from the expected descriptor.
    HeterogeneousNestedElement {
        /// Index of the element with a mismatched descriptor.
        index: usize,
        /// Expected recursive element name.
        expected: Cow<'static, str>,
        /// Found recursive element name.
        found: Cow<'static, str>,
    },
    /// The supplied descriptor's concrete type differs from the vector element type.
    DescriptorTypeMismatch {
        /// Expected descriptor type name.
        expected: Cow<'static, str>,
        /// Found vector element type name.
        found: Cow<'static, str>,
    },
}

impl fmt::Display for ArrayBuildErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingNestedElementType => {
                f.write_str("missing nested element type for DynamicArray elements")
            }
            Self::HeterogeneousNestedElement {
                index,
                expected,
                found,
            } => write!(
                f,
                "nested array element {index} has type {found}, expected {expected}"
            ),
            Self::DescriptorTypeMismatch { expected, found } => {
                write!(
                    f,
                    "descriptor type {expected} does not match vector element type {found}"
                )
            }
        }
    }
}

/// Preserves a vector that could not become a [`DynamicArray`].
pub struct ArrayBuildError<T> {
    kind: ArrayBuildErrorKind,
    values: Vec<T>,
}

impl<T> ArrayBuildError<T> {
    /// Returns the construction failure reason.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{ArrayBuildErrorKind, DynamicArray};
    ///
    /// let error = DynamicArray::try_from_vec(Vec::<DynamicArray>::new()).unwrap_err();
    /// assert_eq!(error.kind(), &ArrayBuildErrorKind::MissingNestedElementType);
    /// ```
    pub fn kind(&self) -> &ArrayBuildErrorKind {
        &self.kind
    }

    /// Returns ownership of the original vector.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::DynamicArray;
    ///
    /// let values = Vec::<DynamicArray>::new();
    /// let error = DynamicArray::try_from_vec(values).unwrap_err();
    /// assert!(error.into_vec().is_empty());
    /// ```
    pub fn into_vec(self) -> Vec<T> {
        self.values
    }

    /// Returns a construction error that preserves `values`.
    fn new(kind: ArrayBuildErrorKind, values: Vec<T>) -> Self {
        Self { kind, values }
    }
}

impl<T> fmt::Debug for ArrayBuildError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArrayBuildError")
            .field("kind", &self.kind)
            .field("len", &self.values.len())
            .field("capacity", &self.values.capacity())
            .finish()
    }
}

impl<T> fmt::Display for ArrayBuildError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to build DynamicArray: {}", self.kind)
    }
}

impl<T: 'static> std::error::Error for ArrayBuildError<T> {}

/// Reports a typed access that a [`DynamicArray`] descriptor rejects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArrayTypeError {
    /// The requested element type differs from the stored descriptor.
    TypeMismatch {
        /// Requested type name.
        expected: Cow<'static, str>,
        /// Stored element type name.
        found: Cow<'static, str>,
    },
    /// Mutable typed access requested an array whose elements are themselves arrays.
    NestedMutableAccess {
        /// Stored recursive array element name.
        element: Cow<'static, str>,
    },
}

impl ArrayTypeError {
    /// Returns the requested type name for a type mismatch.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::DynamicArray;
    ///
    /// let error = DynamicArray::try_from_vec(vec![1i32])
    ///     .unwrap()
    ///     .try_into_vec::<u32>()
    ///     .unwrap_err();
    /// assert_eq!(error.expected(), Some("u32"));
    /// ```
    pub fn expected(&self) -> Option<&str> {
        match self {
            Self::TypeMismatch { expected, .. } => Some(expected),
            Self::NestedMutableAccess { .. } => None,
        }
    }

    /// Returns the stored element type name for a type mismatch.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::DynamicArray;
    ///
    /// let error = DynamicArray::try_from_vec(vec![1i32])
    ///     .unwrap()
    ///     .try_into_vec::<u32>()
    ///     .unwrap_err();
    /// assert_eq!(error.found(), Some("i32"));
    /// ```
    pub fn found(&self) -> Option<&str> {
        match self {
            Self::TypeMismatch { found, .. } => Some(found),
            Self::NestedMutableAccess { .. } => None,
        }
    }

    /// Returns the stored recursive element name for a nested mutable-access rejection.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::DynamicArray;
    ///
    /// let error = DynamicArray::try_from_vec(vec![1i32])
    ///     .unwrap()
    ///     .try_into_vec::<u32>()
    ///     .unwrap_err();
    /// assert_eq!(error.nested_mutable_element(), None);
    /// ```
    pub fn nested_mutable_element(&self) -> Option<&str> {
        match self {
            Self::TypeMismatch { .. } => None,
            Self::NestedMutableAccess { element } => Some(element),
        }
    }

    /// Returns a mismatch between requested `T` and `found`.
    fn for_requested<T: 'static>(found: Cow<'static, str>) -> Self {
        Self::TypeMismatch {
            expected: Cow::Borrowed(std::any::type_name::<T>()),
            found,
        }
    }
}

impl fmt::Display for ArrayTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TypeMismatch { expected, found } => write!(
                f,
                "array element type mismatch: expected {expected}, found {found}"
            ),
            Self::NestedMutableAccess { element } => write!(
                f,
                "nested array element {element} does not allow mutable slice access"
            ),
        }
    }
}

impl std::error::Error for ArrayTypeError {}

/// Reports array storage whose byte size cannot be represented as a valid allocation [`Layout`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ArrayLayoutError {
    /// Recursive element name, for diagnostics.
    element: Cow<'static, str>,
    /// Requested element count.
    capacity: usize,
}

impl fmt::Display for ArrayLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "array storage for {} elements of type {} cannot be represented safely",
            self.capacity, self.element
        )
    }
}

impl std::error::Error for ArrayLayoutError {}

/// Returns the allocation layout for `capacity` elements, or `None` when the storage needs no
/// allocation (a zero-sized element type, or zero capacity).
///
/// # Errors
/// Returns [`ArrayLayoutError`] when `size * capacity` overflows `usize` or exceeds the largest
/// object a [`Layout`] can describe.
///
/// - Complexity: O(1).
fn checked_array_layout(
    element: &ArrayElementType,
    capacity: usize,
) -> Result<Option<Layout>, ArrayLayoutError> {
    if element.size == 0 || capacity == 0 {
        return Ok(None);
    }
    let overflow = || ArrayLayoutError {
        element: element.display_name(),
        capacity,
    };
    let bytes = element.size.checked_mul(capacity).ok_or_else(overflow)?;
    Layout::from_size_align(bytes, element.align)
        .map(Some)
        .map_err(|_| overflow())
}

/// Checks that storage for `capacity` elements can be allocated, without allocating it.
///
/// This lets a caller that will build the array later (an array literal's parse-time validation,
/// for instance) reject an unrepresentable size before it commits to the construction.
///
/// # Errors
/// Returns [`ArrayLayoutError`] under exactly the conditions
/// [`DynamicArrayBuilder::with_capacity`] would.
///
/// - Complexity: O(1).
pub(crate) fn check_array_capacity(
    element: &ArrayElementType,
    capacity: usize,
) -> Result<(), ArrayLayoutError> {
    checked_array_layout(element, capacity).map(|_| ())
}

/// Returns the allocation layout for non-empty storage of non-zero-sized elements.
///
/// - Precondition: `capacity` elements of `element` were allocated successfully, so their layout
///   is known to be representable.
///
/// - Complexity: O(1).
fn allocated_layout(element: &ArrayElementType, capacity: usize) -> Option<Layout> {
    checked_array_layout(element, capacity)
        .expect("a live array's own capacity has a representable layout")
}

/// Returns the non-null, suitably aligned pointer used for storage that owns no allocation.
///
/// - Complexity: O(1).
fn dangling_for(element: &ArrayElementType) -> NonNull<u8> {
    debug_assert!(element.align.is_power_of_two());
    NonNull::new(std::ptr::without_provenance_mut(element.align))
        .expect("a non-zero alignment is a non-null address")
}

/// Owns one exact-capacity array allocation while its elements are moved into it one at a time.
///
/// The builder is the guard the collection of an array literal runs under: it tracks the
/// initialized prefix, so dropping it partway through a transfer drops exactly the elements
/// already moved in — never uninitialized storage — and always frees the allocation.
///
/// # Examples
///
/// ```ignore
/// let mut builder = DynamicArrayBuilder::with_capacity(element, 2)?;
/// unsafe { builder.push_bytes(first) };
/// unsafe { builder.push_bytes(second) };
/// let array = builder.finish();
/// ```
pub(crate) struct DynamicArrayBuilder {
    ptr: NonNull<u8>,
    /// Number of element slots reserved by the allocation.
    capacity: usize,
    /// Number of leading slots already initialized.
    len: usize,
    element: ArrayElementType,
}

impl DynamicArrayBuilder {
    /// Returns a builder owning uninitialized storage for exactly `capacity` elements.
    ///
    /// # Errors
    /// Returns [`ArrayLayoutError`] when storage for `capacity` elements cannot be described by a
    /// valid [`Layout`]. Ordinary allocation failure follows Rust's global allocation-error
    /// behavior.
    ///
    /// - Postcondition: the returned builder holds no initialized element.
    ///
    /// - Complexity: O(1).
    pub(crate) fn with_capacity(
        element: ArrayElementType,
        capacity: usize,
    ) -> Result<Self, ArrayLayoutError> {
        let ptr = match checked_array_layout(&element, capacity)? {
            // Safety: `layout` has non-zero size, so `alloc` is called correctly; a null result
            // is the documented allocation failure, reported through `handle_alloc_error`.
            Some(layout) => {
                NonNull::new(unsafe { alloc(layout) }).unwrap_or_else(|| handle_alloc_error(layout))
            }
            None => dangling_for(&element),
        };
        Ok(Self {
            ptr,
            capacity,
            len: 0,
            element,
        })
    }

    /// Moves one element's bytes out of `src` into the next uninitialized slot, taking ownership
    /// of that element.
    ///
    /// A zero-sized element copies nothing: only the count of owned elements grows, which is what
    /// later runs its drop glue exactly once per element.
    ///
    /// - Postcondition: the builder owns one more element than before.
    ///
    /// - Complexity: O(1) in the element's size.
    ///
    /// # Safety
    /// Fewer than `capacity` elements must have been pushed; pushing beyond that writes past the
    /// end of the builder's allocation. `src` must be valid for reads of the element type's size
    /// and must hold a live value of that type. Ownership of that value transfers to the builder:
    /// the caller must not drop it (or let anything else drop it) afterward.
    pub(crate) unsafe fn push_bytes(&mut self, src: *const MaybeUninit<u8>) {
        debug_assert!(self.len < self.capacity, "builder capacity exceeded");
        if self.element.size != 0 {
            let offset = self.len * self.element.size;
            // Safety: `offset + size` stays within the allocation because `len < capacity` and
            // the allocation holds `capacity * size` bytes; `src` is valid for `size` reads per
            // this function's contract, and a fresh allocation cannot overlap it.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    src,
                    self.ptr.as_ptr().add(offset).cast::<MaybeUninit<u8>>(),
                    self.element.size,
                );
            }
        }
        self.len += 1;
    }

    /// Returns the [`Vec`]-compatible capacity for the storage this builder owns.
    ///
    /// - Complexity: O(1).
    fn vec_capacity(&self) -> usize {
        if self.element.size == 0 {
            usize::MAX
        } else {
            self.capacity
        }
    }

    /// Returns the array owning every element transferred into this builder so far.
    ///
    /// - Postcondition: the result's length is the number of pushed elements and its capacity is
    ///   the requested capacity, so it converts to a `Vec<T>` without reallocating.
    ///
    /// - Complexity: O(1).
    pub(crate) fn finish(self) -> DynamicArray {
        let this = ManuallyDrop::new(self);
        // Safety: `this` is never dropped, so moving `element` out of it by value leaves no
        // second owner of the descriptor's heap data.
        let element = unsafe { std::ptr::read(&this.element) };
        // Safety: the allocation holds `capacity` slots of `element`'s layout, its first `len`
        // slots were initialized by `push_bytes`, and ownership of both transfers here.
        unsafe {
            DynamicArray::try_from_raw_parts(this.ptr, this.len, this.vec_capacity(), element)
        }
        .expect("builder storage satisfies DynamicArray's invariants")
    }
}

impl fmt::Debug for DynamicArrayBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DynamicArrayBuilder")
            .field("len", &self.len)
            .field("capacity", &self.capacity)
            .field("element", &self.element)
            .finish()
    }
}

impl Drop for DynamicArrayBuilder {
    fn drop(&mut self) {
        // Handing the initialized prefix to a `DynamicArray` reuses its destructor: exactly the
        // `len` live elements are dropped, in reverse order, and the allocation is freed with
        // the layout its capacity implies.
        let prefix = unsafe {
            DynamicArray::try_from_raw_parts(
                self.ptr,
                self.len,
                self.vec_capacity(),
                self.element.clone(),
            )
        };
        debug_assert!(
            prefix.is_some(),
            "builder storage satisfies DynamicArray's invariants"
        );
        drop(prefix);
    }
}

/// Owns a homogeneous, type-erased, `Vec`-compatible allocation.
pub struct DynamicArray {
    ptr: NonNull<u8>,
    len: usize,
    capacity: usize,
    element: ArrayElementType,
}

impl DynamicArray {
    /// Takes ownership of a concrete vector without moving or reallocating its elements.
    ///
    /// # Errors
    /// Returns [`ArrayBuildErrorKind::MissingNestedElementType`] for an empty
    /// `Vec<DynamicArray>`. Returns [`ArrayBuildErrorKind::HeterogeneousNestedElement`] when
    /// `Vec<DynamicArray>` contains arrays with different element descriptors.
    ///
    /// - Complexity: O(n) for `Vec<DynamicArray>`; O(1) otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::DynamicArray;
    ///
    /// let values = vec![1i32, 2];
    /// let array = DynamicArray::try_from_vec(values).unwrap();
    /// assert_eq!(array.len(), 2);
    /// ```
    pub fn try_from_vec<T: 'static>(values: Vec<T>) -> Result<Self, ArrayBuildError<T>> {
        let element = match Self::inferred_element_type(&values) {
            Ok(element) => element,
            Err(kind) => return Err(ArrayBuildError::new(kind, values)),
        };
        Ok(Self::from_validated_vec(values, element))
    }

    /// Returns an empty array carrying `element` as its runtime descriptor.
    ///
    /// The returned array owns no element allocation. It exists for contexts such as typed empty
    /// CEL array literals, where the element descriptor is known but no element value exists from
    /// which to infer one.
    ///
    /// - Postcondition: the result has length and capacity zero.
    /// - Complexity: O(1).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{ArrayElementType, DynamicArray};
    ///
    /// let element = ArrayElementType::leaf::<i32>().unwrap();
    /// let array = DynamicArray::empty_with_element_type(element.clone());
    ///
    /// assert!(array.is_empty());
    /// assert_eq!(array.element_type(), &element);
    /// ```
    #[must_use]
    pub fn empty_with_element_type(element: ArrayElementType) -> Self {
        let ptr = dangling_for(&element);
        unsafe { Self::try_from_raw_parts(ptr, 0, 0, element) }
            .expect("an empty typed array satisfies DynamicArray's invariants")
    }

    /// Takes ownership of a vector after checking an explicit element descriptor.
    ///
    /// # Errors
    /// Returns [`ArrayBuildErrorKind::DescriptorTypeMismatch`] when `element` describes a concrete
    /// type other than `T`. Returns [`ArrayBuildErrorKind::MissingNestedElementType`] when a
    /// `Vec<DynamicArray>` descriptor omits its nested element descriptor. Returns
    /// [`ArrayBuildErrorKind::HeterogeneousNestedElement`] when `Vec<DynamicArray>` contains an
    /// array whose descriptor differs from the supplied nested descriptor.
    ///
    /// - Complexity: O(n) for `Vec<DynamicArray>`; O(1) otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{ArrayElementType, DynamicArray};
    ///
    /// let element = ArrayElementType::leaf::<i32>().unwrap();
    /// let array = DynamicArray::try_from_vec_with_element_type(vec![1i32], element).unwrap();
    /// assert_eq!(array.try_as_slice::<i32>().unwrap(), &[1]);
    /// ```
    pub fn try_from_vec_with_element_type<T: 'static>(
        values: Vec<T>,
        element: ArrayElementType,
    ) -> Result<Self, ArrayBuildError<T>> {
        if TypeId::of::<T>() != element.type_id {
            return Err(ArrayBuildError::new(
                ArrayBuildErrorKind::DescriptorTypeMismatch {
                    expected: element.display_name(),
                    found: Cow::Borrowed(std::any::type_name::<T>()),
                },
                values,
            ));
        }
        if TypeId::of::<T>() == TypeId::of::<DynamicArray>() {
            let Some(expected) = element.nested() else {
                return Err(ArrayBuildError::new(
                    ArrayBuildErrorKind::MissingNestedElementType,
                    values,
                ));
            };
            if let Err(kind) = Self::validate_nested_elements(&values, expected) {
                return Err(ArrayBuildError::new(kind, values));
            }
        }

        Ok(Self::from_validated_vec(values, element))
    }

    /// Returns the inferred element descriptor for `values`.
    ///
    /// # Errors
    /// Returns [`ArrayBuildErrorKind::MissingNestedElementType`] when `values` is an empty
    /// `Vec<DynamicArray>`. Returns [`ArrayBuildErrorKind::HeterogeneousNestedElement`] when a
    /// nested element descriptor differs from the first descriptor.
    ///
    /// - Complexity: O(n) for `Vec<DynamicArray>`; O(1) otherwise.
    fn inferred_element_type<T: 'static>(
        values: &[T],
    ) -> Result<ArrayElementType, ArrayBuildErrorKind> {
        if TypeId::of::<T>() != TypeId::of::<DynamicArray>() {
            return ArrayElementType::leaf::<T>();
        }

        let Some(first) = values.first() else {
            return Err(ArrayBuildErrorKind::MissingNestedElementType);
        };
        let expected = Self::nested_element(first)
            .expect("TypeId equality guarantees DynamicArray downcast")
            .element_type()
            .clone();
        Self::validate_nested_elements(values, &expected)?;
        Ok(ArrayElementType::array_of(expected))
    }

    /// Checks that every nested array element has `expected` as its descriptor.
    ///
    /// # Errors
    /// Returns [`ArrayBuildErrorKind::HeterogeneousNestedElement`] at the first index whose
    /// descriptor differs from `expected`.
    ///
    /// - Precondition: `T` is exactly [`DynamicArray`].
    /// - Complexity: O(n).
    fn validate_nested_elements<T: 'static>(
        values: &[T],
        expected: &ArrayElementType,
    ) -> Result<(), ArrayBuildErrorKind> {
        for (index, value) in values.iter().enumerate() {
            let found = Self::nested_element(value)
                .expect("TypeId equality guarantees DynamicArray downcast")
                .element_type();
            if found != expected {
                return Err(ArrayBuildErrorKind::HeterogeneousNestedElement {
                    index,
                    expected: expected.display_name(),
                    found: found.display_name(),
                });
            }
        }
        Ok(())
    }

    /// Returns `value` as a nested dynamic-array element when `T` is exactly [`DynamicArray`].
    ///
    /// - Complexity: O(1).
    fn nested_element<T: 'static>(value: &T) -> Option<&DynamicArray> {
        if TypeId::of::<T>() != TypeId::of::<DynamicArray>() {
            return None;
        }
        let value = value as &dyn Any;
        value.downcast_ref::<DynamicArray>()
    }

    /// Takes ownership of a vector that has already been checked against `element`.
    ///
    /// - Precondition: `element` matches `T` and every nested descriptor in `values`.
    ///
    /// - Complexity: O(1).
    fn from_validated_vec<T: 'static>(values: Vec<T>, element: ArrayElementType) -> Self {
        let mut values = ManuallyDrop::new(values);
        let ptr = NonNull::new(values.as_mut_ptr().cast::<u8>())
            .expect("Vec::as_mut_ptr returns a non-null pointer");
        let len = values.len();
        let capacity = values.capacity();
        unsafe { Self::try_from_raw_parts(ptr, len, capacity, element) }
            .expect("Vec raw parts satisfy DynamicArray invariants")
    }

    /// Takes ownership of raw vector-compatible parts after checking layout invariants.
    ///
    /// Returns `None` when `len > capacity`, `ptr` is not aligned for `element`, or the allocation
    /// layout for `capacity` elements overflows.
    ///
    /// - Complexity: O(1).
    ///
    /// # Safety
    /// `ptr` must be the allocation pointer for `capacity` elements with `element`'s size and
    /// alignment, and the first `len` elements must be live values that can be dropped by
    /// `element`'s dropper with an empty associated-type slice. Ownership of those elements and
    /// the allocation transfers to the returned `DynamicArray`.
    pub(crate) unsafe fn try_from_raw_parts(
        ptr: NonNull<u8>,
        len: usize,
        capacity: usize,
        element: ArrayElementType,
    ) -> Option<Self> {
        if element.align == 0
            || !element.align.is_power_of_two()
            || len > capacity
            || !(ptr.as_ptr() as usize).is_multiple_of(element.align)
        {
            return None;
        }
        if checked_array_layout(&element, capacity).is_err() {
            return None;
        }
        Some(Self {
            ptr,
            len,
            capacity,
            element,
        })
    }

    /// Converts this array back into a concrete vector after checking the element type.
    ///
    /// # Errors
    /// Returns [`ArrayTypeError`] when `T` differs from the stored concrete element type. The array
    /// is dropped on error without casting the allocation.
    ///
    /// - Complexity: O(1).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::DynamicArray;
    ///
    /// let array = DynamicArray::try_from_vec(vec![1i32]).unwrap();
    /// assert_eq!(array.try_into_vec::<i32>().unwrap(), vec![1]);
    /// ```
    pub fn try_into_vec<T: 'static>(self) -> Result<Vec<T>, ArrayTypeError> {
        if TypeId::of::<T>() != self.element.type_id {
            return Err(ArrayTypeError::for_requested::<T>(
                self.element.display_name(),
            ));
        }

        let mut this = ManuallyDrop::new(self);
        let len = this.len;
        let capacity = this.capacity;
        let ptr = this.ptr.as_ptr().cast::<T>();
        // Suppressing this array's `Drop` keeps its elements and their allocation alive for the
        // vector, but the element descriptor is metadata, not part of that allocation: a nested
        // array's descriptor owns a boxed inner descriptor that nothing else would ever free.
        // Safety: `this` is never dropped, so this is the descriptor's only destruction.
        unsafe { std::ptr::drop_in_place(&raw mut this.element) };
        Ok(unsafe { Vec::from_raw_parts(ptr, len, capacity) })
    }

    /// Returns a typed shared slice after checking the concrete element type.
    ///
    /// # Errors
    /// Returns [`ArrayTypeError`] when `T` differs from the stored concrete element type.
    ///
    /// - Complexity: O(1).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::DynamicArray;
    ///
    /// let array = DynamicArray::try_from_vec(vec![1i32, 2]).unwrap();
    /// assert_eq!(array.try_as_slice::<i32>().unwrap(), &[1, 2]);
    /// ```
    pub fn try_as_slice<T: 'static>(&self) -> Result<&[T], ArrayTypeError> {
        if TypeId::of::<T>() != self.element.type_id {
            return Err(ArrayTypeError::for_requested::<T>(
                self.element.display_name(),
            ));
        }
        Ok(unsafe { slice::from_raw_parts(self.ptr.as_ptr().cast::<T>(), self.len) })
    }

    /// Returns a typed mutable slice for leaf element arrays.
    ///
    /// # Errors
    /// Returns [`ArrayTypeError`] when `T` differs from the stored concrete element type or when
    /// the stored element type is a nested array whose recursive descriptor must be protected.
    ///
    /// - Complexity: O(1).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::DynamicArray;
    ///
    /// let mut array = DynamicArray::try_from_vec(vec![1i32, 2]).unwrap();
    /// array.try_as_mut_slice::<i32>().unwrap()[0] = 3;
    /// assert_eq!(array.try_into_vec::<i32>().unwrap(), vec![3, 2]);
    /// ```
    pub fn try_as_mut_slice<T: 'static>(&mut self) -> Result<&mut [T], ArrayTypeError> {
        if TypeId::of::<T>() != self.element.type_id {
            return Err(ArrayTypeError::for_requested::<T>(
                self.element.display_name(),
            ));
        }
        if self.element.nested.is_some() {
            return Err(ArrayTypeError::NestedMutableAccess {
                element: self.element.display_name(),
            });
        }
        Ok(unsafe { slice::from_raw_parts_mut(self.ptr.as_ptr().cast::<T>(), self.len) })
    }

    /// Returns the number of live elements.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::DynamicArray;
    ///
    /// let array = DynamicArray::try_from_vec(vec![1i32, 2]).unwrap();
    /// assert_eq!(array.len(), 2);
    /// ```
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns the stored vector capacity.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::DynamicArray;
    ///
    /// let values = Vec::<i32>::with_capacity(8);
    /// let array = DynamicArray::try_from_vec(values).unwrap();
    /// assert_eq!(array.capacity(), 8);
    /// ```
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns whether the array contains no elements.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::DynamicArray;
    ///
    /// let array = DynamicArray::try_from_vec(Vec::<i32>::new()).unwrap();
    /// assert!(array.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the stored element descriptor.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::DynamicArray;
    /// use std::any::TypeId;
    ///
    /// let array = DynamicArray::try_from_vec(vec![1i32]).unwrap();
    /// assert_eq!(array.element_type().type_id(), TypeId::of::<i32>());
    /// ```
    pub fn element_type(&self) -> &ArrayElementType {
        &self.element
    }
}

impl fmt::Debug for DynamicArray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DynamicArray")
            .field("len", &self.len)
            .field("capacity", &self.capacity)
            .field("element", &self.element)
            .finish()
    }
}

impl Drop for DynamicArray {
    fn drop(&mut self) {
        let base = self.ptr.as_ptr();
        for index in (0..self.len).rev() {
            let ptr = if self.element.size == 0 {
                base
            } else {
                let offset = index
                    .checked_mul(self.element.size)
                    .expect("DynamicArray element offset fits in usize");
                unsafe { base.add(offset) }
            };
            unsafe { (self.element.drop)(ptr, &[]) };
        }

        if let Some(layout) = allocated_layout(&self.element, self.capacity) {
            unsafe { dealloc(base, layout) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::TypeId;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct NoTraits(u32);

    struct CountedDrop(Arc<AtomicUsize>);

    impl Drop for CountedDrop {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    static ZST_DROPS: AtomicUsize = AtomicUsize::new(0);

    struct DroppingZst;

    impl Drop for DroppingZst {
        fn drop(&mut self) {
            ZST_DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[repr(align(64))]
    struct Aligned(u8);

    #[test]
    fn vec_round_trip_preserves_allocation_and_capacity() {
        let mut values = Vec::with_capacity(8);
        values.extend([NoTraits(1), NoTraits(2)]);
        let ptr = values.as_ptr();
        let capacity = values.capacity();

        let array = DynamicArray::try_from_vec(values).unwrap();
        assert_eq!(array.len(), 2);
        assert_eq!(array.capacity(), capacity);

        let values = array.try_into_vec::<NoTraits>().unwrap();
        assert_eq!(values.as_ptr(), ptr);
        assert_eq!(values.capacity(), capacity);
        assert_eq!(values[0].0, 1);
        assert_eq!(values[1].0, 2);
    }

    #[test]
    fn typed_slice_access_checks_the_element_type() {
        let array = DynamicArray::try_from_vec(vec![1i32, 2]).unwrap();
        assert_eq!(array.try_as_slice::<i32>().unwrap(), &[1, 2]);
        assert!(array.try_as_slice::<u32>().is_err());
    }

    #[test]
    fn mutable_leaf_slice_updates_the_owned_values() {
        let mut array = DynamicArray::try_from_vec(vec![1i32, 2]).unwrap();
        array.try_as_mut_slice::<i32>().unwrap()[1] = 9;
        assert_eq!(array.try_into_vec::<i32>().unwrap(), vec![1, 9]);
    }

    #[test]
    fn mutable_nested_slice_rejection_reports_dedicated_error_kind() {
        let inner = DynamicArray::try_from_vec(vec![1i32]).unwrap();
        let mut array = DynamicArray::try_from_vec(vec![inner]).unwrap();

        let error = array.try_as_mut_slice::<DynamicArray>().unwrap_err();

        assert!(matches!(
            error,
            ArrayTypeError::NestedMutableAccess { ref element } if element == "[i32]"
        ));
        assert_eq!(error.expected(), None);
        assert_eq!(error.found(), None);
    }

    #[test]
    fn nested_vec_round_trip_preserves_outer_and_inner_allocations() {
        let left = DynamicArray::try_from_vec(vec![0i32]).unwrap();
        let right = DynamicArray::try_from_vec(vec![1i32]).unwrap();
        let outer_values = vec![left, right];
        let outer_ptr = outer_values.as_ptr();

        let outer = DynamicArray::try_from_vec(outer_values).unwrap();
        let inner = outer.element_type().nested().unwrap();
        assert_eq!(inner.type_id(), TypeId::of::<i32>());

        let values = outer.try_into_vec::<DynamicArray>().unwrap();
        assert_eq!(values.as_ptr(), outer_ptr);
        assert_eq!(values[0].try_as_slice::<i32>().unwrap(), &[0]);
        assert_eq!(values[1].try_as_slice::<i32>().unwrap(), &[1]);
    }

    #[test]
    fn heterogeneous_nested_vec_is_returned_on_error() {
        let values = vec![
            DynamicArray::try_from_vec(vec![0i32]).unwrap(),
            DynamicArray::try_from_vec(vec![1f64]).unwrap(),
        ];

        let error = DynamicArray::try_from_vec(values).unwrap_err();

        assert!(matches!(
            error.kind(),
            ArrayBuildErrorKind::HeterogeneousNestedElement {
                index: 1,
                expected,
                found,
            } if expected == "i32" && found == "f64"
        ));
        assert_eq!(error.into_vec().len(), 2);
    }

    #[test]
    fn empty_nested_vec_uses_an_explicit_inner_type() {
        let element = ArrayElementType::array_of(ArrayElementType::leaf::<i32>().unwrap());

        let array =
            DynamicArray::try_from_vec_with_element_type(Vec::<DynamicArray>::new(), element)
                .unwrap();

        assert!(array.try_into_vec::<DynamicArray>().unwrap().is_empty());
    }

    #[test]
    fn empty_with_element_type_preserves_leaf_descriptor() {
        let element = ArrayElementType::leaf::<i32>().unwrap();

        let array = DynamicArray::empty_with_element_type(element.clone());

        assert!(array.is_empty());
        assert_eq!(array.capacity(), 0);
        assert_eq!(array.element_type(), &element);
        assert!(array.try_into_vec::<i32>().unwrap().is_empty());
    }

    #[test]
    fn empty_with_element_type_preserves_nested_descriptor() {
        let element = ArrayElementType::array_of(ArrayElementType::leaf::<i32>().unwrap());

        let array = DynamicArray::empty_with_element_type(element.clone());

        assert!(array.is_empty());
        assert_eq!(array.capacity(), 0);
        assert_eq!(array.element_type(), &element);
        assert!(array.try_into_vec::<DynamicArray>().unwrap().is_empty());
    }

    #[test]
    fn empty_with_element_type_preserves_zero_sized_leaf_descriptor() {
        let element = ArrayElementType::leaf::<DroppingZst>().unwrap();

        let array = DynamicArray::empty_with_element_type(element.clone());

        assert!(array.is_empty());
        assert_eq!(array.capacity(), 0);
        assert_eq!(array.element_type(), &element);
        assert!(array.try_into_vec::<DroppingZst>().unwrap().is_empty());
    }

    #[test]
    fn aligned_element_slice_preserves_vec_alignment() {
        let array = DynamicArray::try_from_vec(vec![Aligned(7)]).unwrap();
        let values = array.try_as_slice::<Aligned>().unwrap();
        assert_eq!(values.as_ptr() as usize % 64, 0);
        assert_eq!(values[0].0, 7);
    }

    #[test]
    fn drop_destroys_each_element_once() {
        let drops = Arc::new(AtomicUsize::new(0));
        {
            let _array = DynamicArray::try_from_vec(vec![
                CountedDrop(drops.clone()),
                CountedDrop(drops.clone()),
            ])
            .unwrap();
            assert_eq!(drops.load(Ordering::SeqCst), 0);
        }
        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn zst_drop_runs_for_each_live_element() {
        ZST_DROPS.store(0, Ordering::SeqCst);
        {
            let array =
                DynamicArray::try_from_vec(vec![DroppingZst, DroppingZst, DroppingZst]).unwrap();
            assert_eq!(array.len(), 3);
            assert_eq!(array.capacity(), usize::MAX);
        }
        assert_eq!(ZST_DROPS.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn mismatched_vec_extraction_drops_without_casting() {
        let drops = Arc::new(AtomicUsize::new(0));
        let array = DynamicArray::try_from_vec(vec![CountedDrop(drops.clone())]).unwrap();
        let error = array.try_into_vec::<u32>().unwrap_err();

        assert_eq!(error.expected(), Some("u32"));
        assert!(
            error
                .found()
                .is_some_and(|found| found.ends_with("CountedDrop"))
        );
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn empty_vec_round_trip_preserves_length_and_capacity() {
        let values = Vec::<i32>::new();
        let ptr = values.as_ptr();
        let array = DynamicArray::try_from_vec(values).unwrap();

        assert!(array.is_empty());
        assert_eq!(array.capacity(), 0);

        let values = array.try_into_vec::<i32>().unwrap();
        assert_eq!(values.as_ptr(), ptr);
        assert!(values.is_empty());
        assert_eq!(values.capacity(), 0);
    }

    #[test]
    fn leaf_dynamic_array_descriptor_requires_nested_element_type() {
        assert_eq!(
            ArrayElementType::leaf::<DynamicArray>(),
            Err(ArrayBuildErrorKind::MissingNestedElementType)
        );
    }

    /// The erased constructor rejects the same marker id [`ArrayElementType::leaf`] refuses, in
    /// every build profile: a leaf descriptor claiming it would hand out element views over
    /// values of another type.
    #[test]
    #[should_panic(expected = "a leaf element descriptor must not claim the array marker TypeId")]
    fn leaf_from_parts_rejects_the_array_marker_type() {
        let _ = ArrayElementType::leaf_from_parts(
            TypeId::of::<DynamicArray>(),
            Cow::Borrowed("DynamicArray"),
            size_of::<DynamicArray>(),
            align_of::<DynamicArray>(),
            raw_dropper_for::<DynamicArray>(),
        );
    }

    #[test]
    fn explicit_descriptor_must_match_the_vector_element_type() {
        let error = DynamicArray::try_from_vec_with_element_type(
            vec![1i32],
            ArrayElementType::leaf::<u32>().unwrap(),
        )
        .unwrap_err();

        assert!(matches!(
            error.kind(),
            ArrayBuildErrorKind::DescriptorTypeMismatch {
                expected,
                found,
            } if expected == "u32" && found == "i32"
        ));
        assert_eq!(error.into_vec(), vec![1]);
    }

    /// Returns a pointer to `value`'s bytes, for a builder transfer that must not also drop it.
    fn moved_bytes<T>(value: &ManuallyDrop<T>) -> *const MaybeUninit<u8> {
        (&raw const **value).cast::<MaybeUninit<u8>>()
    }

    #[test]
    fn builder_finish_owns_every_transferred_element() {
        let mut builder =
            DynamicArrayBuilder::with_capacity(ArrayElementType::leaf::<i32>().unwrap(), 3)
                .unwrap();
        for value in 0i32..3 {
            let value = ManuallyDrop::new(value);
            unsafe { builder.push_bytes(moved_bytes(&value)) };
        }

        let array = builder.finish();

        assert_eq!(array.capacity(), 3);
        assert_eq!(array.try_into_vec::<i32>().unwrap(), vec![0, 1, 2]);
    }

    #[test]
    fn builder_drops_only_its_initialized_prefix() {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut builder =
            DynamicArrayBuilder::with_capacity(ArrayElementType::leaf::<CountedDrop>().unwrap(), 4)
                .unwrap();
        for _ in 0..2 {
            let value = ManuallyDrop::new(CountedDrop(drops.clone()));
            unsafe { builder.push_bytes(moved_bytes(&value)) };
        }

        drop(builder);

        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn builder_finishes_zero_sized_elements_without_allocating() {
        ZST_DROPS.store(0, Ordering::SeqCst);
        let mut builder =
            DynamicArrayBuilder::with_capacity(ArrayElementType::leaf::<DroppingZst>().unwrap(), 3)
                .unwrap();
        for _ in 0..3 {
            let value = ManuallyDrop::new(DroppingZst);
            unsafe { builder.push_bytes(moved_bytes(&value)) };
        }

        let array = builder.finish();
        assert_eq!(array.len(), 3);
        assert_eq!(array.capacity(), usize::MAX);
        assert_eq!(ZST_DROPS.load(Ordering::SeqCst), 0);

        drop(array);
        assert_eq!(ZST_DROPS.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn builder_rejects_a_capacity_whose_layout_overflows() {
        let error = DynamicArrayBuilder::with_capacity(
            ArrayElementType::leaf::<[u8; 1024]>().unwrap(),
            usize::MAX / 512,
        )
        .unwrap_err();

        assert!(error.to_string().contains("[u8; 1024]"), "{error}");
    }

    #[test]
    fn checked_array_capacity_accepts_a_representable_layout() {
        assert!(check_array_capacity(&ArrayElementType::leaf::<i32>().unwrap(), 4).is_ok());
        assert!(
            check_array_capacity(
                &ArrayElementType::leaf::<DroppingZst>().unwrap(),
                usize::MAX
            )
            .is_ok(),
            "zero-sized elements never allocate, so no capacity overflows"
        );
    }
}
