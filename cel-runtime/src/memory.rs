/// Aligns an index to the specified alignment boundary.
/// Returns the next aligned position that satisfies the alignment requirement.
///
/// # Panics
///
/// Panics (in debug configuration) if the alignment is not a power of two.
///
#[must_use]
pub const fn align_index(align: usize, index: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (index + align - 1) & !(align - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_align_index() {
        assert_eq!(align_index(16, 0), 0);
        assert_eq!(align_index(16, 1), 16);
        assert_eq!(align_index(16, 15), 16);
        assert_eq!(align_index(16, 16), 16);
        assert_eq!(align_index(16, 17), 32);
    }
}
