/// Aligns an index to the specified alignment boundary.
/// Returns the next aligned position that satisfies the alignment requirement.
#[must_use]
pub const fn align_index(align: usize, index: usize) -> usize {
    (index + align - 1) & !(align - 1)
}
