//! Rewrites bare `<n>rem` CSS lengths into host-scalable `calc()` expressions.
//!
//! `build.rs` includes this module with `#[path = "build/scale_rem.rs"]` to rewrite each
//! vendored stylesheet before parsing. The library's test tree includes the same file with
//! `#[cfg(test)] #[path = "../build/scale_rem.rs"]` so its contract tests run under
//! `cargo test -p adam-web-ui` without duplicating the implementation.
use std::fmt::Write;

/// Returns whether `byte` can continue a CSS identifier (a letter, digit, `_`, or `-`),
/// distinguishing identifier continuation from a leading numeric sign.
pub(crate) fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

/// Rewrites every bare `<n>rem` length in `source` into `calc(<n> * var(--adam-rem, 1rem))`,
/// letting a host scale every embedded control length by setting `--adam-rem` on or above
/// `.adam-theme`.
///
/// A single forward scan copies through non-matching bytes and replaces each numeric literal
/// immediately followed by `rem` and a non-identifier character (so `rem` is a whole unit, not
/// a prefix of a longer identifier). A digit or `.` starts a literal unless it continues an
/// identifier, i.e. it is preceded by a letter, digit, `_`, or a `-`/`+` that is itself
/// continuing an identifier rather than acting as a numeric sign. A leading `-` or `+`
/// immediately before the digit/`.` is a numeric sign, and is included in the rewritten
/// literal, whenever the byte before that sign is absent or not an identifier byte; a sign
/// that continues an identifier (e.g. the second `-` in `--adam-rem`, or the `-` in a class
/// name like `.a-1rem`) still blocks the match. This means a declaration of `--adam-rem`
/// itself inside a processed stylesheet is NOT left alone: its `1rem` fallback value still
/// starts right after `: `, which is not an identifier byte, so it is rewritten into a
/// self-referential `calc(1 * var(--adam-rem, 1rem))` -- exactly the hazard the theme
/// stylesheet's own comment warns against, which is why that stylesheet must not declare
/// `--adam-rem`. The scan does not special-case existing `calc(...)` wrappers, so a length
/// already inside one (e.g. `calc(1.15rem - 2px)`) nests into
/// `calc(calc(1.15 * var(--adam-rem, 1rem)) - 2px)`; Lightning CSS accepts and minifies nested
/// `calc()`, so this remains valid CSS.
///
/// A numeric literal is a run of ASCII digits and `.` containing at least one digit, so a
/// class selector such as `.rem` is never rewritten.
///
/// - Precondition: `source` contains no string literals or comments whose text resembles a
///   `<n>rem` length; the scan is not a CSS tokenizer and would rewrite such text.
///
/// - Complexity: O(b) time and space for source byte length `b`.
pub(crate) fn scale_rem(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = String::with_capacity(source.len());
    let mut copied_to = 0;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_digit() || byte == b'.' {
            let preceding = bytes.get(index.wrapping_sub(1)).copied();
            let number_start = match preceding {
                Some(b'-') | Some(b'+')
                    if !bytes
                        .get(index.wrapping_sub(2))
                        .is_some_and(|b| is_identifier_byte(*b)) =>
                {
                    Some(index - 1)
                }
                Some(b) if is_identifier_byte(b) => None,
                _ => Some(index),
            };
            if let Some(number_start) = number_start {
                let mut cursor = index;
                while bytes
                    .get(cursor)
                    .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
                {
                    cursor += 1;
                }
                let number_end = cursor;
                let has_digit = bytes[index..number_end].iter().any(u8::is_ascii_digit);
                let unit_end = cursor + 3;
                let followed_by_rem = bytes.get(cursor..unit_end) == Some(b"rem");
                let unit_is_whole_word = has_digit
                    && followed_by_rem
                    && !bytes.get(unit_end).is_some_and(|b| is_identifier_byte(*b));
                if unit_is_whole_word {
                    output.push_str(&source[copied_to..number_start]);
                    let literal = &source[number_start..number_end];
                    write!(output, "calc({literal} * var(--adam-rem, 1rem))").unwrap();
                    index = unit_end;
                    copied_to = unit_end;
                    continue;
                }
            }
        }
        index += 1;
    }
    output.push_str(&source[copied_to..]);
    output
}

#[cfg(test)]
mod tests {
    use super::scale_rem;

    /// A plain positive length scales with the host `--adam-rem` custom property.
    #[test]
    fn positive_length_scales_with_adam_rem() {
        assert_eq!(
            scale_rem("margin: 0.5rem;"),
            "margin: calc(0.5 * var(--adam-rem, 1rem));"
        );
    }

    /// A negative length keeps its sign inside the rewritten `calc()` literal.
    #[test]
    fn negative_length_keeps_its_sign() {
        assert_eq!(
            scale_rem("margin: -0.5rem;"),
            "margin: calc(-0.5 * var(--adam-rem, 1rem));"
        );
    }

    /// An explicit `+` sign is likewise treated as part of the numeric literal.
    #[test]
    fn explicit_plus_sign_is_kept() {
        assert_eq!(
            scale_rem("width: +0.5rem;"),
            "width: calc(+0.5 * var(--adam-rem, 1rem));"
        );
    }

    /// A digit that continues an identifier (a class name, not a length) is left untouched,
    /// including when the identifier is itself hyphenated.
    #[test]
    fn identifier_adjacent_digits_are_left_alone() {
        assert_eq!(
            scale_rem(".a-1rem { width: 1rem; }"),
            ".a-1rem { width: calc(1 * var(--adam-rem, 1rem)); }"
        );
    }

    /// A length already inside an existing `calc(...)` nests into a valid nested `calc()`.
    #[test]
    fn nested_calc_lengths_scale_too() {
        assert_eq!(
            scale_rem("transform: translateX(calc(2rem - 1px - (1.15rem - 2px)));"),
            "transform: translateX(calc(calc(2 * var(--adam-rem, 1rem)) - 1px - (calc(1.15 * var(--adam-rem, 1rem)) - 2px)));"
        );
    }

    /// A lone `.` before `rem` (a class selector) is not a number and is left untouched.
    #[test]
    fn digitless_dot_before_rem_is_left_alone() {
        assert_eq!(
            scale_rem(".rem { width: 1rem; }"),
            ".rem { width: calc(1 * var(--adam-rem, 1rem)); }"
        );
        assert_eq!(scale_rem("a .rem{}"), "a .rem{}");
    }

    /// A length in a unit other than `rem` is never rewritten.
    #[test]
    fn non_rem_lengths_are_untouched() {
        assert_eq!(scale_rem("width: 10px;"), "width: 10px;");
    }
}
