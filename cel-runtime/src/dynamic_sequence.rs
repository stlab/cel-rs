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
#[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

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
