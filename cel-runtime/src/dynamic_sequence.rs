//! `DynamicSequence`: an owned, type-erased CEL tuple value that persists beyond any single
//! `DynSegment` evaluation.
//!
//! Converts type-safely to and from concrete, nestable Rust tuples via the [`TupleSequence`]
//! trait, implemented for arities 1 through 12. All the actual byte-layout work is implemented
//! exactly once, generically, by [`SequenceList`], over the cons-list `tuple_list.rs`'s
//! `IntoTupleList` already produces.
//!
//! # Examples
//!
//! Build a sequence from a concrete tuple, then read it back — both by cloning
//! ([`try_to_tuple`](DynamicSequence::try_to_tuple), leaving the sequence usable afterward) and
//! by consuming ([`try_into_tuple`](DynamicSequence::try_into_tuple)):
//!
//! ```rust
//! use cel_runtime::DynamicSequence;
//!
//! let seq = DynamicSequence::from_tuple((3i32, 4.5f64));
//! assert_eq!(seq.arity(), 2);
//!
//! let cloned: (i32, f64) = seq.try_to_tuple().unwrap();
//! assert_eq!(cloned, (3, 4.5));
//!
//! let moved: (i32, f64) = seq.try_into_tuple().unwrap();
//! assert_eq!(moved, (3, 4.5));
//! ```
//!
//! [`adapt_fn_1`](DynamicSequence::adapt_fn_1) adapts a closure written against a concrete tuple
//! into a closure over `&DynamicSequence`, so ordinary tuple-shaped Rust code can be called
//! directly with a type-erased sequence:
//!
//! ```rust
//! use cel_runtime::DynamicSequence;
//!
//! let seq = DynamicSequence::from_tuple((3i32, 4.5f64));
//! let wrapped = DynamicSequence::adapt_fn_1(|t: &(i32, f64)| Ok(t.0 as f64 + t.1));
//! assert_eq!(wrapped(&seq).unwrap(), 7.5);
//! ```

use crate::memory::align_index;
use crate::tuple_list::IntoTupleList;
use std::any::TypeId;
use std::borrow::Cow;

/// Drops a value in place, given a pointer to its bytes.
///
/// # Safety
/// `ptr` must point to a valid, live, properly aligned value of the type this dropper was
/// generated for.
pub type ElementDropper = unsafe fn(*mut u8);

/// Clones a value in place: reads a live value at `src`, writes a fresh clone of it at `dst`.
///
/// # Safety
/// `src` must point to a valid, live, properly aligned value of the type this cloner was
/// generated for; `dst` must be valid for writes of that same type's size and alignment.
pub type ElementCloner = unsafe fn(*const u8, *mut u8);

/// Compares two values of the same type for equality.
///
/// # Safety
/// `a` and `b` must each point to a valid, live, properly aligned value of the type this
/// comparator was generated for.
pub type ElementEq = unsafe fn(*const u8, *const u8) -> bool;

/// Describes one element of a [`DynamicSequence`]: its type identity, byte layout, and
/// in-place drop/clone/equality functions.
#[derive(Clone)]
pub struct SequenceElement {
    /// Runtime type id for this element.
    pub type_id: TypeId,
    /// Human-readable name for error reporting.
    pub type_name: Cow<'static, str>,
    /// Byte offset from the start of the enclosing sequence.
    pub offset: usize,
    /// Size in bytes of this element's value.
    pub size: usize,
    /// Required alignment in bytes of this element's value.
    pub align: usize,
    /// In-place dropper for this element, callable at its own start address.
    pub drop: ElementDropper,
    /// In-place cloner for this element.
    pub clone: ElementCloner,
    /// Equality comparator for this element.
    pub eq: ElementEq,
}

/// Returns an [`ElementDropper`] that drops a value of type `T` in place.
pub fn element_dropper_for<T: 'static>() -> ElementDropper {
    |ptr| unsafe { std::ptr::drop_in_place(ptr.cast::<T>()) }
}

/// Returns an [`ElementCloner`] that clones a value of type `T` in place.
pub fn element_cloner_for<T: 'static + Clone>() -> ElementCloner {
    |src, dst| unsafe { std::ptr::write(dst.cast::<T>(), (*src.cast::<T>()).clone()) }
}

/// Returns an [`ElementEq`] that compares two values of type `T` for equality.
pub fn element_eq_for<T: 'static + PartialEq>() -> ElementEq {
    |a, b| unsafe { *a.cast::<T>() == *b.cast::<T>() }
}

/// Returns a function that moves a boxed `T`'s bytes to `dst`, consuming the box without
/// running `T`'s destructor (ownership transfers to `dst`).
///
/// # Safety
/// The returned function's `dst` must be valid for writes of `size_of::<T>()` bytes at
/// `align_of::<T>()`; its `Box<dyn Any>` argument's runtime type must be `T`.
pub fn element_writer_for<T: 'static>() -> unsafe fn(Box<dyn std::any::Any>, *mut u8) {
    |boxed, dst| unsafe {
        let value = *boxed
            .downcast::<T>()
            .expect("element_writer_for: type mismatch");
        std::ptr::write(dst.cast::<T>(), value);
    }
}

/// Computes the next aligned offset for a `'static + Clone + PartialEq` field of type `T`,
/// appends its [`SequenceElement`] to `out`, folds `T`'s alignment into `*max_align`, and returns
/// the byte position immediately after this element.
///
/// - Complexity: O(1).
fn push_element<T: 'static + Clone + PartialEq>(
    out: &mut Vec<SequenceElement>,
    offset: usize,
    max_align: &mut usize,
) -> usize {
    use std::mem::{align_of, size_of};

    let align = align_of::<T>();
    let aligned_offset = align_index(align, offset);
    *max_align = (*max_align).max(align);
    out.push(SequenceElement {
        type_id: TypeId::of::<T>(),
        type_name: Cow::Borrowed(std::any::type_name::<T>()),
        offset: aligned_offset,
        size: size_of::<T>(),
        align,
        drop: element_dropper_for::<T>(),
        clone: element_cloner_for::<T>(),
        eq: element_eq_for::<T>(),
    });
    aligned_offset + size_of::<T>()
}

/// Cons-list (`()` or `(H, T)`, matching `tuple_list.rs`'s `IntoTupleList` output) that knows how
/// to lay itself out as a [`DynamicSequence`] shape and move/clone itself to and from raw bytes
/// at that layout.
///
/// Implemented exactly twice — for `()` and `(H, T)` — covering every tuple arity generically; no
/// per-arity unsafe code is needed here or anywhere downstream.
pub trait SequenceList: Sized {
    /// Appends this list's own elements (head-first order) to `out`, computing each one's offset
    /// from `offset` (the byte position immediately after the previous element) and folding each
    /// element's alignment into `*max_align`. Returns the byte position immediately after the
    /// last element appended.
    ///
    /// - Complexity: O(length).
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize;

    /// Writes this list's fields into `dst`, at the positions in `offsets` (which must be
    /// exactly this list's own element offsets, head-first, as produced by
    /// [`append_shape`](Self::append_shape)), consuming `self`.
    ///
    /// - Complexity: O(length).
    ///
    /// # Safety
    /// `dst` must be valid for writes covering every offset + size of this list's elements.
    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]);

    /// Reads this list back out of `src` by moving each field's bytes, at the positions in
    /// `offsets`.
    ///
    /// - Complexity: O(length).
    ///
    /// # Safety
    /// `src` must point to a live value whose layout matches `offsets`; the caller must not
    /// separately drop those bytes afterward.
    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self;

    /// Reads this list back out of `src` by cloning each field's bytes, at the positions in
    /// `offsets`, leaving `src` untouched.
    ///
    /// - Complexity: O(length).
    ///
    /// # Safety
    /// `src` must point to a live value whose layout matches `offsets`.
    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self;
}

impl SequenceList for () {
    fn append_shape(
        _out: &mut Vec<SequenceElement>,
        offset: usize,
        _max_align: &mut usize,
    ) -> usize {
        offset
    }

    unsafe fn write_into(self, _dst: *mut u8, _offsets: &[usize]) {}

    unsafe fn read_from(_src: *const u8, _offsets: &[usize]) -> Self {}

    unsafe fn clone_from(_src: *const u8, _offsets: &[usize]) -> Self {}
}

impl<H: 'static + Clone + PartialEq, T: SequenceList> SequenceList for (H, T) {
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize {
        let offset = push_element::<H>(out, offset, max_align);
        T::append_shape(out, offset, max_align)
    }

    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]) {
        unsafe {
            std::ptr::write(dst.add(offsets[0]).cast::<H>(), self.0);
            T::write_into(self.1, dst, &offsets[1..]);
        }
    }

    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            let h = std::ptr::read(src.add(offsets[0]).cast::<H>());
            let t = T::read_from(src, &offsets[1..]);
            (h, t)
        }
    }

    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            let h = (*src.add(offsets[0]).cast::<H>()).clone();
            let t = T::clone_from(src, &offsets[1..]);
            (h, t)
        }
    }
}

/// Bridges a concrete Rust tuple `T` to and from its [`IntoTupleList`] cons-list representation,
/// so [`DynamicSequence`] can convert to/from `T` while all the actual byte-layout work is
/// handled generically by [`SequenceList`].
///
/// Implemented for tuples of arity 1 through 12 — the same range `cel-runtime`'s `IntoList`
/// supports — via a single-line reversal of [`IntoTupleList::into_tuple_list`]; no unsafe code
/// appears in any of these impls. A nested tuple field needs no special handling: it's simply an
/// ordinary `'static + Clone + PartialEq` element type to its enclosing tuple's own impl.
pub trait TupleSequence: IntoTupleList + Sized
where
    Self::Output: SequenceList,
{
    /// Reconstructs `Self` from its cons-list representation — the reverse of
    /// [`IntoTupleList::into_tuple_list`].
    fn from_list(list: Self::Output) -> Self;
}

impl<A: 'static + Clone + PartialEq> TupleSequence for (A,) {
    fn from_list(list: Self::Output) -> Self {
        let (a, ()) = list;
        (a,)
    }
}

impl<A: 'static + Clone + PartialEq, B: 'static + Clone + PartialEq> TupleSequence for (A, B) {
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, ())) = list;
        (a, b)
    }
}

impl<A: 'static + Clone + PartialEq, B: 'static + Clone + PartialEq, C: 'static + Clone + PartialEq>
    TupleSequence for (A, B, C)
{
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, ()))) = list;
        (a, b, c)
    }
}

impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D)
{
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, ())))) = list;
        (a, b, c, d)
    }
}

impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
    E: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D, E)
{
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, (e, ()))))) = list;
        (a, b, c, d, e)
    }
}

impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
    E: 'static + Clone + PartialEq,
    F: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D, E, F)
{
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, (e, (f, ())))))) = list;
        (a, b, c, d, e, f)
    }
}

impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
    E: 'static + Clone + PartialEq,
    F: 'static + Clone + PartialEq,
    G: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D, E, F, G)
{
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, (e, (f, (g, ()))))))) = list;
        (a, b, c, d, e, f, g)
    }
}

impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
    E: 'static + Clone + PartialEq,
    F: 'static + Clone + PartialEq,
    G: 'static + Clone + PartialEq,
    H: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D, E, F, G, H)
{
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, (e, (f, (g, (h, ())))))))) = list;
        (a, b, c, d, e, f, g, h)
    }
}

impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
    E: 'static + Clone + PartialEq,
    F: 'static + Clone + PartialEq,
    G: 'static + Clone + PartialEq,
    H: 'static + Clone + PartialEq,
    I: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D, E, F, G, H, I)
{
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, (e, (f, (g, (h, (i, ()))))))))) = list;
        (a, b, c, d, e, f, g, h, i)
    }
}

impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
    E: 'static + Clone + PartialEq,
    F: 'static + Clone + PartialEq,
    G: 'static + Clone + PartialEq,
    H: 'static + Clone + PartialEq,
    I: 'static + Clone + PartialEq,
    J: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D, E, F, G, H, I, J)
{
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, (e, (f, (g, (h, (i, (j, ())))))))))) = list;
        (a, b, c, d, e, f, g, h, i, j)
    }
}

impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
    E: 'static + Clone + PartialEq,
    F: 'static + Clone + PartialEq,
    G: 'static + Clone + PartialEq,
    H: 'static + Clone + PartialEq,
    I: 'static + Clone + PartialEq,
    J: 'static + Clone + PartialEq,
    K: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D, E, F, G, H, I, J, K)
{
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, (e, (f, (g, (h, (i, (j, (k, ()))))))))))) = list;
        (a, b, c, d, e, f, g, h, i, j, k)
    }
}

impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
    E: 'static + Clone + PartialEq,
    F: 'static + Clone + PartialEq,
    G: 'static + Clone + PartialEq,
    H: 'static + Clone + PartialEq,
    I: 'static + Clone + PartialEq,
    J: 'static + Clone + PartialEq,
    K: 'static + Clone + PartialEq,
    L: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D, E, F, G, H, I, J, K, L)
{
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, (e, (f, (g, (h, (i, (j, (k, (l, ())))))))))))) = list;
        (a, b, c, d, e, f, g, h, i, j, k, l)
    }
}

/// An owned, type-erased CEL tuple value that persists beyond any single `DynSegment`
/// evaluation.
///
/// Converts type-safely to and from concrete Rust tuples via [`TupleSequence`] — see
/// [`from_tuple`](Self::from_tuple), [`try_into_tuple`](Self::try_into_tuple), and
/// [`try_to_tuple`](Self::try_to_tuple).
pub struct DynamicSequence {
    buffer: crate::raw_stack::RawStack,
    shape: Vec<SequenceElement>,
    max_align: usize,
}

impl DynamicSequence {
    /// Builds a `DynamicSequence` from a concrete Rust tuple, consuming it.
    ///
    /// - Complexity: O(arity) time; exactly one heap allocation.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::DynamicSequence;
    ///
    /// let seq = DynamicSequence::from_tuple((1i32, "hello".to_string()));
    /// assert_eq!(seq.arity(), 2);
    /// ```
    #[must_use]
    pub fn from_tuple<T: TupleSequence>(value: T) -> Self
    where
        T::Output: SequenceList,
    {
        let list = value.into_tuple_list();
        let mut shape = Vec::new();
        let mut max_align = 1usize;
        let end = T::Output::append_shape(&mut shape, 0, &mut max_align);
        let total_size = align_index(max_align, end);
        let offsets: Vec<usize> = shape.iter().map(|e| e.offset).collect();

        let mut buffer = crate::raw_stack::RawStack::with_base_alignment(max_align);
        unsafe {
            buffer.reserve_and_write(max_align, total_size, |dst| {
                list.write_into(dst, &offsets);
            });
        }
        DynamicSequence {
            buffer,
            shape,
            max_align,
        }
    }

    /// Returns the number of elements this sequence holds.
    #[must_use]
    pub fn arity(&self) -> usize {
        self.shape.len()
    }

    /// Returns whether `T`'s element `TypeId` sequence matches this sequence's actual elements
    /// exactly (same arity, same type at each position, in order).
    fn shape_matches<T: TupleSequence>(&self) -> bool
    where
        T::Output: SequenceList,
    {
        let mut expected = Vec::new();
        let mut max_align = 1usize;
        T::Output::append_shape(&mut expected, 0, &mut max_align);
        self.shape.len() == expected.len()
            && self
                .shape
                .iter()
                .zip(&expected)
                .all(|(a, b)| a.type_id == b.type_id)
    }

    /// Consumes this sequence and reconstructs it as the concrete tuple `T`.
    ///
    /// - Complexity: O(arity).
    ///
    /// # Errors
    /// Returns `Err` if `T`'s element `TypeId` sequence doesn't match this sequence's actual
    /// elements (different arity, or a different type at some position).
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::DynamicSequence;
    ///
    /// let seq = DynamicSequence::from_tuple((3i32, 4.5f64));
    /// let result: (i32, f64) = seq.try_into_tuple().unwrap();
    /// assert_eq!(result, (3, 4.5));
    /// ```
    pub fn try_into_tuple<T: TupleSequence>(mut self) -> anyhow::Result<T>
    where
        T::Output: SequenceList,
    {
        anyhow::ensure!(
            self.shape_matches::<T>(),
            "DynamicSequence::try_into_tuple: shape mismatch"
        );
        let offsets: Vec<usize> = self.shape.iter().map(|e| e.offset).collect();
        let list = unsafe {
            self.buffer
                .read_at(0, |base| T::Output::read_from(base, &offsets))
        };
        // Fields were just moved out of `self.buffer`'s bytes above; clearing `shape` makes
        // `Drop`'s element loop a no-op so those fields aren't dropped a second time. `buffer`'s
        // own backing allocation is still freed normally when `self` goes out of scope below.
        self.shape.clear();
        Ok(T::from_list(list))
    }

    /// Reconstructs the concrete tuple `T` by cloning this sequence's elements, leaving `self`
    /// untouched.
    ///
    /// - Complexity: O(arity).
    ///
    /// # Errors
    /// Returns `Err` if `T`'s element `TypeId` sequence doesn't match this sequence's actual
    /// elements (different arity, or a different type at some position).
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::DynamicSequence;
    ///
    /// let seq = DynamicSequence::from_tuple((1i32, "hello".to_string()));
    /// let a: (i32, String) = seq.try_to_tuple().unwrap();
    /// let b: (i32, String) = seq.try_to_tuple().unwrap(); // `seq` is still usable afterward.
    /// assert_eq!(a, (1, "hello".to_string()));
    /// assert_eq!(a, b);
    /// ```
    pub fn try_to_tuple<T: TupleSequence>(&self) -> anyhow::Result<T>
    where
        T::Output: SequenceList,
    {
        anyhow::ensure!(
            self.shape_matches::<T>(),
            "DynamicSequence::try_to_tuple: shape mismatch"
        );
        let offsets: Vec<usize> = self.shape.iter().map(|e| e.offset).collect();
        let list = unsafe {
            self.buffer
                .read_at(0, |base| T::Output::clone_from(base, &offsets))
        };
        Ok(T::from_list(list))
    }

    /// Adapts a closure over a concrete tuple `A` into a closure over `&DynamicSequence`.
    ///
    /// Every call clones `A`'s fields out of the `&DynamicSequence` (via
    /// [`try_to_tuple`](Self::try_to_tuple)) into a temporary `A`, calls `f` with a reference to
    /// it, then drops the temporary.
    ///
    /// # Errors
    /// The returned closure returns `Err` if `A`'s element `TypeId` sequence doesn't match the
    /// `DynamicSequence`'s actual elements.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::DynamicSequence;
    ///
    /// let seq = DynamicSequence::from_tuple((3i32, 4.5f64));
    /// let wrapped = DynamicSequence::adapt_fn_1(|t: &(i32, f64)| Ok(t.0 as f64 + t.1));
    /// assert_eq!(wrapped(&seq).unwrap(), 7.5);
    /// ```
    pub fn adapt_fn_1<A, R, F>(f: F) -> impl Fn(&DynamicSequence) -> anyhow::Result<R>
    where
        A: TupleSequence,
        A::Output: SequenceList,
        F: Fn(&A) -> anyhow::Result<R>,
    {
        move |seq: &DynamicSequence| {
            let a: A = seq.try_to_tuple()?;
            f(&a)
        }
    }

    /// Assembles a `DynamicSequence` directly from an already-populated buffer and shape.
    ///
    /// # Safety
    /// `buffer` must contain exactly the bytes described by `shape`, laid out at each element's
    /// own `offset`; `max_align` must be at least as large as every element's `align`.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_raw_parts(
        buffer: crate::raw_stack::RawStack,
        shape: Vec<SequenceElement>,
        max_align: usize,
    ) -> Self {
        DynamicSequence {
            buffer,
            shape,
            max_align,
        }
    }

    /// Returns this sequence's own element shape, for use by `dyn_segment`'s tuple-expansion
    /// machinery.
    #[allow(dead_code)]
    pub(crate) fn shape(&self) -> &[SequenceElement] {
        &self.shape
    }

    /// Reads this sequence's element at `offset` via `read`, given a pointer to its start.
    ///
    /// - Precondition: `offset` is one of `self.shape()`'s own recorded element offsets.
    #[allow(dead_code)]
    pub(crate) fn read_element_at<R>(&self, offset: usize, read: impl FnOnce(*const u8) -> R) -> R {
        unsafe { self.buffer.read_at(offset, read) }
    }
}

impl Drop for DynamicSequence {
    fn drop(&mut self) {
        for elem in self.shape.iter().rev() {
            unsafe { self.buffer.drop_at(elem.offset, |ptr| (elem.drop)(ptr)) };
        }
    }
}

impl Clone for DynamicSequence {
    fn clone(&self) -> Self {
        let total_size = self.buffer.len();
        let mut buffer = crate::raw_stack::RawStack::with_base_alignment(self.max_align);
        unsafe {
            buffer.reserve_and_write(self.max_align, total_size, |dst| {
                for elem in &self.shape {
                    self.buffer.read_at(elem.offset, |src| {
                        (elem.clone)(src, dst.add(elem.offset));
                    });
                }
            });
        }
        DynamicSequence {
            buffer,
            shape: self.shape.clone(),
            max_align: self.max_align,
        }
    }
}

impl PartialEq for DynamicSequence {
    fn eq(&self, other: &Self) -> bool {
        self.shape.len() == other.shape.len()
            && self
                .shape
                .iter()
                .zip(&other.shape)
                .all(|(a, b)| a.type_id == b.type_id)
            && self.shape.iter().all(|elem| unsafe {
                self.buffer.read_at(elem.offset, |a| {
                    other.buffer.read_at(elem.offset, |b| (elem.eq)(a, b))
                })
            })
    }
}

impl std::fmt::Debug for DynamicSequence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynamicSequence")
            .field("arity", &self.shape.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

    #[test]
    fn sequence_list_base_case_is_a_no_op() {
        let mut shape = Vec::new();
        let mut max_align = 1usize;
        let end = <()>::append_shape(&mut shape, 0, &mut max_align);
        assert_eq!(end, 0);
        assert!(shape.is_empty());
    }

    #[test]
    fn sequence_list_cons_case_round_trips_two_elements() {
        let mut shape = Vec::new();
        let mut max_align = 1usize;
        <(i32, (f64, ()))>::append_shape(&mut shape, 0, &mut max_align);
        assert_eq!(shape[0].offset, 0);
        assert_eq!(shape[1].offset, 8); // i32 at [0,4); f64 aligned up to 8
        let offsets: Vec<usize> = shape.iter().map(|e| e.offset).collect();

        let mut buf = [0u8; 16];
        let list = (7i32, (2.5f64, ()));
        unsafe { list.write_into(buf.as_mut_ptr(), &offsets) };
        let cloned =
            unsafe { <(i32, (f64, ())) as SequenceList>::clone_from(buf.as_ptr(), &offsets) };
        assert_eq!(cloned, (7, (2.5, ())));
        let read = unsafe { <(i32, (f64, ()))>::read_from(buf.as_ptr(), &offsets) };
        assert_eq!(read, (7, (2.5, ())));
    }

    #[test]
    fn sequence_list_supports_nested_tuple_elements() {
        // A nested tuple field needs no special handling: (i32, i32) is just an
        // ordinary 'static + Clone + PartialEq element type here.
        let mut shape = Vec::new();
        let mut max_align = 1usize;
        <(i32, ((i32, i32), (bool, ())))>::append_shape(&mut shape, 0, &mut max_align);
        let offsets: Vec<usize> = shape.iter().map(|e| e.offset).collect();

        let mut buf = [0u8; 16];
        let list = (1i32, ((2i32, 3i32), (true, ())));
        unsafe { list.write_into(buf.as_mut_ptr(), &offsets) };
        let read = unsafe {
            <(i32, ((i32, i32), (bool, ()))) as SequenceList>::read_from(buf.as_ptr(), &offsets)
        };
        assert_eq!(read, (1, ((2, 3), (true, ()))));
    }

    #[test]
    fn push_element_records_layout_and_generates_working_fn_pointers() {
        let mut out = Vec::new();
        let mut max_align = 1usize;
        let next = push_element::<i32>(&mut out, 0, &mut max_align);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].type_id, TypeId::of::<i32>());
        assert_eq!(out[0].offset, 0);
        assert_eq!(out[0].size, size_of::<i32>());
        assert_eq!(out[0].align, align_of::<i32>());
        assert_eq!(next, size_of::<i32>());
        assert_eq!(max_align, align_of::<i32>());

        let mut value = 7i32;
        let mut cloned = 0i32;
        unsafe {
            (out[0].clone)(
                (&raw mut value).cast::<u8>(),
                (&raw mut cloned).cast::<u8>(),
            );
        }
        assert_eq!(cloned, 7);
        assert!(unsafe {
            (out[0].eq)(
                (&raw const value).cast::<u8>(),
                (&raw const cloned).cast::<u8>(),
            )
        });
        unsafe { (out[0].drop)((&raw mut value).cast::<u8>()) };
    }

    #[test]
    fn push_element_computes_alignment_padding_between_calls() {
        let mut out = Vec::new();
        let mut max_align = 1usize;
        let after_u8 = push_element::<u8>(&mut out, 0, &mut max_align);
        let after_u32 = push_element::<u32>(&mut out, after_u8, &mut max_align);

        assert_eq!(out[0].offset, 0);
        assert_eq!(out[1].offset, 4); // u8 at [0,1); u32 aligned up to 4
        assert_eq!(after_u32, 8);
        assert_eq!(max_align, 4);
    }

    #[test]
    fn tuple_sequence_from_list_reverses_into_tuple_list_for_several_arities() {
        assert_eq!(<(i32,)>::from_list((1i32, ())), (1,));
        assert_eq!(<(i32, f64)>::from_list((1i32, (2.5f64, ()))), (1, 2.5));
        assert_eq!(
            <(i32, f64, bool)>::from_list((1i32, (2.5f64, (true, ())))),
            (1, 2.5, true)
        );
        let full_arity_12 = (
            1i32, 2i32, 3i32, 4i32, 5i32, 6i32, 7i32, 8i32, 9i32, 10i32, 11i32, 12i32,
        );
        assert_eq!(
            <(i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32)>::from_list(
                full_arity_12.into_tuple_list()
            ),
            full_arity_12
        );
    }

    #[test]
    fn from_tuple_records_correct_arity() {
        let seq = DynamicSequence::from_tuple((1i32, 2.5f64, true));
        assert_eq!(seq.arity(), 3);
    }

    #[test]
    fn from_tuple_and_drop_drops_every_element_exactly_once_in_reverse_order() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct DropCounter(Arc<AtomicUsize>, Arc<std::sync::Mutex<Vec<u8>>>, u8);
        impl Clone for DropCounter {
            fn clone(&self) -> Self {
                DropCounter(self.0.clone(), self.1.clone(), self.2)
            }
        }
        impl PartialEq for DropCounter {
            fn eq(&self, other: &Self) -> bool {
                self.2 == other.2
                    && Arc::ptr_eq(&self.0, &other.0)
                    && Arc::ptr_eq(&self.1, &other.1)
            }
        }
        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
                self.1.lock().unwrap().push(self.2);
            }
        }

        let count = Arc::new(AtomicUsize::new(0));
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let a = DropCounter(count.clone(), order.clone(), 1);
        let b = DropCounter(count.clone(), order.clone(), 2);

        let seq = DynamicSequence::from_tuple((a, b));
        drop(seq);

        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(*order.lock().unwrap(), vec![2, 1]); // reverse of declaration order
    }

    #[test]
    fn clone_produces_an_independently_droppable_equal_copy() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Clone)]
        struct DropCounter(Arc<AtomicUsize>);
        impl PartialEq for DropCounter {
            fn eq(&self, other: &Self) -> bool {
                Arc::ptr_eq(&self.0, &other.0)
            }
        }
        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let count = Arc::new(AtomicUsize::new(0));
        let original = DynamicSequence::from_tuple((DropCounter(count.clone()), 7i32));
        let cloned = original.clone();

        assert_eq!(original, cloned);

        drop(original);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        drop(cloned);
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn partial_eq_is_false_for_different_arity_or_element_type() {
        let a = DynamicSequence::from_tuple((1i32, 2i32));
        let b = DynamicSequence::from_tuple((1i32,));
        let c = DynamicSequence::from_tuple((1i32, 2.0f64));
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn partial_eq_is_false_for_different_values_of_the_same_shape() {
        let a = DynamicSequence::from_tuple((1i32, 2i32));
        let b = DynamicSequence::from_tuple((1i32, 3i32));
        assert_ne!(a, b);
        let c = DynamicSequence::from_tuple((1i32, 2i32));
        assert_eq!(a, c);
    }

    #[test]
    fn try_into_tuple_round_trips_for_a_matching_shape() -> anyhow::Result<()> {
        let seq = DynamicSequence::from_tuple((3i32, 4.5f64));
        let result: (i32, f64) = seq.try_into_tuple()?;
        assert_eq!(result, (3, 4.5));
        Ok(())
    }

    #[test]
    fn try_into_tuple_errs_on_shape_mismatch() {
        let seq = DynamicSequence::from_tuple((3i32, 4i32));
        let result = seq.try_into_tuple::<(i32, f64)>();
        assert!(result.is_err());
    }

    #[test]
    fn try_to_tuple_clones_without_consuming_the_sequence() -> anyhow::Result<()> {
        let seq = DynamicSequence::from_tuple((1i32, "hello".to_string()));
        let a: (i32, String) = seq.try_to_tuple()?;
        let b: (i32, String) = seq.try_to_tuple()?;
        assert_eq!(a, (1, "hello".to_string()));
        assert_eq!(b, (1, "hello".to_string()));
        Ok(())
    }

    #[test]
    fn try_to_tuple_errs_on_shape_mismatch() {
        let seq = DynamicSequence::from_tuple((1i32,));
        let result = seq.try_to_tuple::<(i32, i32)>();
        assert!(result.is_err());
    }

    #[test]
    fn try_into_tuple_moves_fields_without_double_dropping_or_leaking() -> anyhow::Result<()> {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Clone)]
        struct DropCounter(Arc<AtomicUsize>);
        impl PartialEq for DropCounter {
            fn eq(&self, other: &Self) -> bool {
                Arc::ptr_eq(&self.0, &other.0)
            }
        }
        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let count = Arc::new(AtomicUsize::new(0));
        let seq = DynamicSequence::from_tuple((DropCounter(count.clone()), 7i32));
        let (extracted, n): (DropCounter, i32) = seq.try_into_tuple()?;
        assert_eq!(n, 7);
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "moving out must not drop the element"
        );
        drop(extracted);
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "the moved-out value must still drop exactly once, on its own schedule"
        );
        Ok(())
    }

    #[test]
    fn adapt_fn_1_calls_the_wrapped_closure_with_a_concrete_tuple() -> anyhow::Result<()> {
        let seq = DynamicSequence::from_tuple((3i32, 4.5f64));
        let wrapped = DynamicSequence::adapt_fn_1(|t: &(i32, f64)| Ok(t.0 as f64 + t.1));
        assert_eq!(wrapped(&seq)?, 7.5);
        Ok(())
    }

    #[test]
    fn adapt_fn_1_returns_err_on_shape_mismatch() {
        let seq = DynamicSequence::from_tuple((3i32, 4i32));
        let wrapped = DynamicSequence::adapt_fn_1(|t: &(i32, f64)| Ok(t.0 as f64 + t.1));
        assert!(wrapped(&seq).is_err());
    }

    #[test]
    fn element_dropper_for_drops_the_correct_type_exactly_once() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct DropCounter(Arc<AtomicUsize>);
        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let count = Arc::new(AtomicUsize::new(0));
        let mut value = DropCounter(count.clone());
        let dropper = element_dropper_for::<DropCounter>();
        unsafe { dropper((&raw mut value).cast::<u8>()) };
        std::mem::forget(value);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn element_cloner_for_clones_the_correct_type() {
        let cloner = element_cloner_for::<i32>();
        let src = 7i32;
        let mut dst = 0i32;
        unsafe { cloner((&raw const src).cast::<u8>(), (&raw mut dst).cast::<u8>()) };
        assert_eq!(dst, 7);
    }

    #[test]
    fn element_eq_for_compares_the_correct_type() {
        let eq = element_eq_for::<i32>();
        let (a, b, c) = (5i32, 5i32, 6i32);
        assert!(unsafe { eq((&raw const a).cast::<u8>(), (&raw const b).cast::<u8>()) });
        assert!(!unsafe { eq((&raw const a).cast::<u8>(), (&raw const c).cast::<u8>()) });
    }

    #[test]
    fn element_writer_for_moves_a_boxed_value_without_dropping_it() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Clone)]
        struct DropCounter(Arc<AtomicUsize>);
        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let count = Arc::new(AtomicUsize::new(0));
        let boxed: Box<dyn std::any::Any> = Box::new(DropCounter(count.clone()));
        let writer = element_writer_for::<DropCounter>();
        let mut dst = std::mem::MaybeUninit::<DropCounter>::uninit();
        unsafe { writer(boxed, dst.as_mut_ptr().cast::<u8>()) };
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "the box's move must not run Drop"
        );
        unsafe { dst.assume_init_drop() };
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
