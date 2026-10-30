//! Computes byte offsets for aligned storage.
//!
//! Use [`align_index`] with a power-of-two alignment and a representable
//! padded index before addressing an aligned buffer.

/// Returns the least aligned index not smaller than `index`.
///
/// - Precondition: `align` is a power of two and `index + align - 1` is representable.
///
/// # Examples
/// ```
/// assert_eq!(cel_runtime::memory::align_index(8, 9), 16);
/// ```
///
#[must_use]
pub const fn align_index(align: usize, index: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    debug_assert!(index <= usize::MAX - (align - 1));
    (index + align - 1) & !(align - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rounds up unaligned indexes and preserves aligned indexes.
    #[test]
    fn test_align_index() {
        assert_eq!(align_index(16, 0), 0);
        assert_eq!(align_index(16, 1), 16);
        assert_eq!(align_index(16, 15), 16);
        assert_eq!(align_index(16, 16), 16);
        assert_eq!(align_index(16, 17), 32);
    }
}
