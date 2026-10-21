//! Computes layouts for source-ordered, physically tail-first canonical lists.
//!
//! Native tuple bridges preserve logical order independently of physical layout:
//!
//! ```
//! use cel_runtime::{IntoCStackList, IntoRustTuple};
//!
//! let list = (String::from("left"), 7_u8).into_c_stack_list();
//! let value: (String, u8) = list.into_rust_tuple();
//! assert_eq!(value, (String::from("left"), 7));
//! ```

use std::alloc::Layout;

use crate::CNil;

/// Stores a canonical list's aggregate layout and logical source-field offsets.
///
/// Each offset addresses a head inside its recursive, tail-first `repr(C)` node.
/// Every node includes its own trailing padding; native tuple layout is irrelevant.
pub(crate) struct TupleLayout {
    /// The layout of the complete canonical list, including trailing padding.
    pub(crate) layout: Layout,
    /// Head offsets indexed by logical source order, relative to the list base.
    pub(crate) offsets: Vec<usize>,
}

impl TupleLayout {
    /// Computes a canonical list layout from logical source-ordered field layouts.
    ///
    /// Starts with `CNil<()>` and wraps each padded tail in a new tail-first
    /// `repr(C)` node. Empty input yields the canonical empty sentinel's layout.
    ///
    /// - Complexity: O(n) time and O(n) space for n fields.
    ///
    /// # Errors
    ///
    /// Returns an error if any intermediate node, including its alignment padding,
    /// exceeds the size representable by [`Layout`].
    pub(crate) fn from_fields(fields: &[Layout]) -> anyhow::Result<Self> {
        let mut layout = Layout::new::<CNil<()>>();
        let mut offsets = vec![0; fields.len()];
        for (index, field) in fields.iter().enumerate().rev() {
            let (node, offset) = layout.extend(*field)?;
            offsets[index] = offset;
            layout = node.pad_to_align();
        }
        Ok(Self { layout, offsets })
    }
}

#[cfg(test)]
mod tests {
    use std::alloc::Layout;
    use std::ptr;

    use crate::{CNil, CStackList, IntoCStackList};

    use super::TupleLayout;

    /// Represents an over-aligned zero-sized field.
    #[repr(align(64))]
    struct AlignedZst;

    /// Compares calculated source offsets and layout with actual recursive list fields.
    fn check_three<A: 'static, B: 'static, C: 'static>(values: (A, B, C)) {
        let list = values.into_c_stack_list();
        let calculated =
            TupleLayout::from_fields(&[Layout::new::<A>(), Layout::new::<B>(), Layout::new::<C>()])
                .unwrap();
        let base = ptr::from_ref(&list).addr();
        assert_eq!(
            calculated.offsets,
            [
                ptr::from_ref(&list.1).addr() - base,
                ptr::from_ref(&list.0.1).addr() - base,
                ptr::from_ref(&list.0.0.1).addr() - base,
            ],
        );
        assert_eq!(
            calculated.layout,
            Layout::new::<CStackList<A, CStackList<B, CStackList<C, CNil<()>>>>>(),
        );
    }

    /// Verifies mixed-alignment offsets follow the actual list rather than native tuples.
    #[test]
    fn layout_matches_tail_first_list() {
        check_three((1_u8, 2_f64, 3_u16));
    }

    /// Verifies each recursive tail retains its trailing padding before the next head.
    #[test]
    fn layout_preserves_internal_node_padding() {
        check_three((1_u8, 2_u8, 3_u64));
    }

    /// Verifies zero-sized fields in every position, including alignment-only fields.
    #[test]
    fn layout_zero_sized_fields() {
        check_three(((), 1_f64, 2_u16));
        check_three((1_u8, (), 2_u16));
        check_three((1_u8, 2_f64, ()));
        check_three((AlignedZst, 1_u8, 2_u16));
        check_three((1_u8, AlignedZst, 2_u16));
        check_three((1_u8, 2_f64, AlignedZst));
        check_three((AlignedZst, (), AlignedZst));
    }

    /// Verifies nested list fields contribute their full recursive layout.
    #[test]
    fn layout_nested_list_field() {
        check_three((1_u8, (2_u8, 3_u64).into_c_stack_list(), 4_u16));
    }

    /// Verifies the empty sentinel and singleton have their actual list layouts.
    #[test]
    fn layout_empty_and_singleton() {
        let empty = TupleLayout::from_fields(&[]).unwrap();
        assert!(empty.offsets.is_empty());
        assert_eq!(empty.layout, Layout::for_value(&().into_c_stack_list()));
        let singleton = TupleLayout::from_fields(&[Layout::new::<u16>()]).unwrap();
        assert_eq!(singleton.offsets, [0]);
        assert_eq!(
            singleton.layout,
            Layout::for_value(&(1_u16,).into_c_stack_list()),
        );
    }

    /// Verifies layout calculation has no native tuple arity limit.
    #[test]
    fn layout_arbitrary_field_count() {
        let calculated = TupleLayout::from_fields(&[Layout::new::<u8>(); 32]).unwrap();
        assert_eq!(calculated.offsets, (0..32).rev().collect::<Vec<_>>());
        assert_eq!(calculated.layout, Layout::from_size_align(32, 1).unwrap());
    }

    /// Verifies aggregate size and alignment-padding overflow return errors.
    #[test]
    fn layout_rejects_overflow() {
        let huge = Layout::from_size_align(isize::MAX as usize, 1).unwrap();
        assert!(TupleLayout::from_fields(&[Layout::new::<u8>(), huge]).is_err());
        assert!(TupleLayout::from_fields(&[Layout::new::<AlignedZst>(), huge]).is_err());
        let boundary = TupleLayout::from_fields(&[huge]).unwrap();
        assert_eq!(boundary.offsets, [0]);
        assert_eq!(boundary.layout, huge);
    }
}
