//! `DynamicSequence`: an owned, type-erased CEL tuple value that persists beyond any single
//! `DynSegment` evaluation, and can be stored directly as an `adam_rs::Sheet` cell's value.
//!
//! Converts type-safely to and from concrete, nestable Rust tuples via the [`TupleSequence`]
//! trait, implemented for arities 1 through 12. All the actual byte-layout work is implemented
//! exactly once, generically, by [`SequenceList`], over the cons-list `tuple_list.rs`'s
//! `IntoTupleList` already produces.

use crate::memory::align_index;
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
        drop: |ptr| unsafe { std::ptr::drop_in_place(ptr.cast::<T>()) },
        clone: |src, dst| unsafe { std::ptr::write(dst.cast::<T>(), (*src.cast::<T>()).clone()) },
        eq: |a, b| unsafe { *a.cast::<T>() == *b.cast::<T>() },
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
}
