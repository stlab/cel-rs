//! Shared trivia recovery: turns the raw source text of a grammatically-constrained gap
//! (whitespace, comments, and a known fixed sequence of literal tokens) into an ordered sequence
//! of [`GapPiece`]s, and converts `proc_macro2` span positions to byte offsets. Used by
//! `cel-parser`'s own expression formatter and by `adam-lang`'s declaration formatter, so both
//! recover comments the same way. See <https://github.com/stlab/cel-rs/issues/201>.

use proc_macro2::LineColumn;

/// A recovered `//`/`/* */` comment, remembering which delimiter style the source used so a
/// formatter can reproduce it instead of normalizing every comment to one style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Comment {
    /// A line comment's text — its `//` and surrounding whitespace stripped. A producer that
    /// merges a run of consecutive `//` lines (e.g. adam-lang's trivia recovery) stores them
    /// joined with `\n`; cel-parser's `scan_gap` emits one `Line` per `//` line.
    Line(String),
    /// A `/* text */` block comment (single- or multi-line), inner text `\n`-joined with the
    /// `/*`/`*/` delimiters and per-line indentation stripped.
    Block(String),
}

/// One element of a scanned gap: either a recovered comment or one of the literal tokens the
/// caller declared it expected to find there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GapPiece {
    /// A comment found in the gap.
    Comment(Comment),
    /// One expected literal token (an operator symbol, delimiter, comma, or keyword).
    Punct(&'static str),
}

/// Scans the raw text of a grammatically-constrained gap — whitespace, `//`/`/* */` comments, and
/// exactly the literal tokens in `expected`, in that order — returning every comment found plus
/// every `expected` token, interleaved in source order.
///
/// - Postcondition: the result contains each element of `expected` exactly once, as a
///   `GapPiece::Punct`, in `expected`'s order, regardless of how much of `gap` actually matched
///   (an empty or synthetic `gap` still yields all expected tokens, so a hand-built `Expr` with no
///   real source re-synthesizes its separators).
///
/// - Complexity: O(n) in `gap.len()`.
pub fn scan_gap(gap: &str, expected: &[&'static str]) -> Vec<GapPiece> {
    let mut pieces = Vec::new();
    let mut next = 0usize;
    let mut rest = gap;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if let Some(after) = rest.strip_prefix("//") {
            let end = after.find('\n').unwrap_or(after.len());
            pieces.push(GapPiece::Comment(Comment::Line(
                after[..end].trim().to_string(),
            )));
            rest = &after[end..];
        } else if let Some(after) = rest.strip_prefix("/*") {
            let close = after.find("*/").unwrap_or(after.len());
            pieces.push(GapPiece::Comment(Comment::Block(normalize_block(
                &after[..close],
            ))));
            rest = after.get(close + 2..).unwrap_or("");
        } else if next < expected.len() && rest.starts_with(expected[next]) {
            pieces.push(GapPiece::Punct(expected[next]));
            rest = &rest[expected[next].len()..];
            next += 1;
        } else {
            // The gap didn't match the next expected token (synthetic/empty source, or an
            // unforeseen shape). Stop scanning; remaining expected tokens are synthesized below.
            break;
        }
    }
    while next < expected.len() {
        pieces.push(GapPiece::Punct(expected[next]));
        next += 1;
    }
    pieces
}

/// Normalizes a block comment's inner text (between `/*` and `*/`): trims each line and joins the
/// non-empty ones with `\n`, matching `adam-lang`'s existing `Comment::Block` convention.
///
/// - Complexity: O(n) in `inner.len()`.
fn normalize_block(inner: &str) -> String {
    inner
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Returns the byte offset of the start of each line in `source`: `result[line - 1]` is the start
/// of 1-based line `line` (matching [`proc_macro2::LineColumn::line`]'s convention).
///
/// - Complexity: O(n) in `source.len()`.
pub fn line_start_byte_offsets(source: &str) -> Vec<usize> {
    let mut offsets = vec![0usize];
    let mut byte = 0usize;
    for line in source.split_inclusive('\n') {
        byte += line.len();
        offsets.push(byte);
    }
    offsets
}

/// Converts a [`LineColumn`] (1-based line, 0-based character column) to a byte offset in
/// `source`, using `line_starts` (from [`line_start_byte_offsets`]).
///
/// - Precondition: `line_starts` was built from exactly `source`, and `pos.line - 1` is in range.
///
/// - Complexity: O(k) in `pos.column`.
pub fn line_column_to_byte(source: &str, line_starts: &[usize], pos: LineColumn) -> usize {
    debug_assert!(
        pos.line >= 1 && pos.line - 1 < line_starts.len(),
        "pos.line out of range for line_starts"
    );
    let line_start = line_starts[pos.line - 1];
    line_start
        + source[line_start..]
            .chars()
            .take(pos.column)
            .map(char::len_utf8)
            .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comments(pieces: Vec<GapPiece>) -> Vec<Comment> {
        pieces
            .into_iter()
            .filter_map(|p| match p {
                GapPiece::Comment(c) => Some(c),
                GapPiece::Punct(_) => None,
            })
            .collect()
    }

    fn puncts(pieces: &[GapPiece]) -> Vec<&'static str> {
        pieces
            .iter()
            .filter_map(|p| match p {
                GapPiece::Punct(s) => Some(*s),
                GapPiece::Comment(_) => None,
            })
            .collect()
    }

    #[test]
    fn empty_gap_yields_just_the_expected_tokens() {
        let pieces = scan_gap(" ", &["+"]);
        assert_eq!(puncts(&pieces), vec!["+"]);
        assert!(comments(pieces).is_empty());
    }

    #[test]
    fn a_block_comment_before_the_token_is_recovered_in_order() {
        let pieces = scan_gap(" /* a */ + ", &["+"]);
        assert!(matches!(pieces[0], GapPiece::Comment(Comment::Block(ref t)) if t == "a"));
        assert!(matches!(pieces[1], GapPiece::Punct("+")));
    }

    #[test]
    fn a_block_comment_after_the_token_is_recovered_in_order() {
        let pieces = scan_gap(" + /* b */ ", &["+"]);
        assert!(matches!(pieces[0], GapPiece::Punct("+")));
        assert!(matches!(pieces[1], GapPiece::Comment(Comment::Block(ref t)) if t == "b"));
    }

    #[test]
    fn comments_on_both_sides_of_the_token_keep_source_order() {
        let pieces = scan_gap(" /* a */ + /* b */ ", &["+"]);
        assert!(matches!(pieces[0], GapPiece::Comment(Comment::Block(ref t)) if t == "a"));
        assert!(matches!(pieces[1], GapPiece::Punct("+")));
        assert!(matches!(pieces[2], GapPiece::Comment(Comment::Block(ref t)) if t == "b"));
    }

    #[test]
    fn two_consecutive_block_comments_are_two_pieces() {
        let pieces = scan_gap(" /* a */ /* b */ + ", &["+"]);
        assert_eq!(comments(pieces.clone()).len(), 2);
        assert!(matches!(pieces[2], GapPiece::Punct("+")));
    }

    #[test]
    fn two_consecutive_line_comments_are_two_pieces() {
        let pieces = scan_gap(" // a\n // b\n + ", &["+"]);
        assert_eq!(comments(pieces.clone()).len(), 2);
        assert!(matches!(pieces.last(), Some(GapPiece::Punct("+"))));
    }

    #[test]
    fn a_line_comment_is_recovered_as_comment_line() {
        let pieces = scan_gap(" + // hi\n", &["+"]);
        assert!(matches!(pieces[0], GapPiece::Punct("+")));
        assert!(matches!(pieces[1], GapPiece::Comment(Comment::Line(ref t)) if t == "hi"));
    }

    #[test]
    fn a_multi_line_block_comment_joins_its_inner_lines() {
        let pieces = scan_gap(" /*\n  one\n  two\n*/ + ", &["+"]);
        assert!(matches!(pieces[0], GapPiece::Comment(Comment::Block(ref t)) if t == "one\ntwo"));
    }

    #[test]
    fn multiple_expected_tokens_are_returned_in_order() {
        // Between a callee and its first arg with a trailing comma case:
        let pieces = scan_gap(" ( /* c */ ", &["("]);
        assert!(matches!(pieces[0], GapPiece::Punct("(")));
        assert!(matches!(pieces[1], GapPiece::Comment(Comment::Block(ref t)) if t == "c"));
    }

    #[test]
    fn missing_expected_token_is_still_synthesized() {
        // A synthetic/empty gap (hand-built Expr) must still yield every expected token.
        let pieces = scan_gap("", &["(", ")"]);
        assert_eq!(puncts(&pieces), vec!["(", ")"]);
    }

    #[test]
    fn line_column_to_byte_maps_a_position_on_the_second_line() {
        let source = "ab\ncd";
        let starts = line_start_byte_offsets(source);
        // line 2 (1-based), column 1 (0-based) => the 'd', byte offset 4.
        let pos = LineColumn { line: 2, column: 1 };
        assert_eq!(line_column_to_byte(source, &starts, pos), 4);
    }
}
