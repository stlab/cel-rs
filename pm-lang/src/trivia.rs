//! Recovers comments discarded by `proc_macro2`'s tokenizer and attaches them to the nearest
//! following [`crate::ast::SheetItem`], the same re-slicing-the-gap technique `rustfmt` uses for
//! the identical problem (see `cel-parser/src/lex_lexer.rs`'s `test_span_preservation`).

use proc_macro2::LineColumn;

use crate::ast::Sheet;

/// Recovers comments from the gaps between consecutive [`crate::ast::SheetItem`]s in `sheet` and
/// attaches the trailing contiguous comment block immediately preceding each item as that item's
/// `leading_comment`.
///
/// A comment is attached only if nothing but whitespace-on-the-same-line separates it from the
/// following item — a blank line between an earlier comment and the item breaks the attachment,
/// matching the common convention that a blank line ends a comment's association with what
/// follows.
///
/// - Precondition: `sheet` was parsed from exactly `source` (unmodified), so its items' spans'
///   line/column positions resolve correctly against it.
///
/// - Complexity: O(n) in the length of `source`. Every item's gap needs a `LineColumn → byte
///   offset` conversion; rather than calling [`cel_parser::SourceSpan::to_byte_range`] once per
///   gap (each call rescanning `source` from byte 0, making the whole function O(items ×
///   `source` length)), this precomputes each line's starting byte offset in `source` once via
///   [`line_start_byte_offsets`] and reuses that table for every gap — each gap then costs only
///   O(its own line's length), not O(`source` length).
pub fn attach_trivia(source: &str, sheet: &mut Sheet) {
    if sheet.items.len() < 2 {
        return;
    }
    let line_starts = line_start_byte_offsets(source);
    for i in 1..sheet.items.len() {
        let start = line_column_to_byte(source, &line_starts, sheet.items[i - 1].span().end.end());
        let end = line_column_to_byte(source, &line_starts, sheet.items[i].span().start.start());
        let gap_text = &source[start..end];
        if let Some(comment) = trailing_comment_block(gap_text) {
            sheet.items[i].set_leading_comment(comment);
        }
    }
}

/// Returns the byte offset of the start of each line in `source`: `result[line - 1]` is the
/// start of 1-based line `line` (matching [`proc_macro2::LineColumn::line`]'s convention).
///
/// - Complexity: O(n) in the length of `source`.
fn line_start_byte_offsets(source: &str) -> Vec<usize> {
    let mut offsets = vec![0usize];
    let mut byte = 0usize;
    for line in source.split_inclusive('\n') {
        byte += line.len();
        offsets.push(byte);
    }
    offsets
}

/// Converts a [`LineColumn`] (1-based line, 0-based character column) to a byte offset in
/// `source`, using `line_starts` (from [`line_start_byte_offsets`]) instead of rescanning
/// `source` from byte 0.
///
/// - Precondition: `line_starts` was built from exactly `source`, and `pos` was recorded
///   against `source` (so `pos.line - 1` is in range).
///
/// - Complexity: O(k), where k is `pos.column` — bounded by that one line's length, not the
///   whole of `source`.
fn line_column_to_byte(source: &str, line_starts: &[usize], pos: LineColumn) -> usize {
    let line_start = line_starts[pos.line - 1];
    line_start
        + source[line_start..]
            .chars()
            .take(pos.column)
            .map(char::len_utf8)
            .sum::<usize>()
}

/// Returns the maximal trailing run of `//` line comments (or a single `/* ... */` block
/// comment) in `gap`, joined with `\n`, or `None` if `gap`'s last non-blank line isn't a
/// comment. A blank line breaks the run.
fn trailing_comment_block(gap: &str) -> Option<String> {
    let mut lines: Vec<&str> = gap.lines().collect();
    // `gap` ends exactly where the following item's first token begins. When that token isn't
    // at column 0, `lines()`'s final entry is only the leading whitespace before it on its own
    // line, not a blank source line — drop that fragment before scanning for a trailing comment
    // run so a real blank line (a genuine empty entry from `lines()`) still breaks the run.
    if !gap.ends_with('\n') {
        lines.pop();
    }
    let mut collected = Vec::new();
    while let Some(line) = lines.last() {
        let trimmed = line.trim();
        if let Some(text) = trimmed.strip_prefix("//") {
            collected.push(text.trim().to_string());
            lines.pop();
        } else if let Some(text) = trimmed
            .strip_prefix("/*")
            .and_then(|s| s.strip_suffix("*/"))
        {
            collected.push(text.trim().to_string());
            lines.pop();
            break; // a block comment is one unit; don't merge with an earlier `//` run
        } else {
            break;
        }
    }
    if collected.is_empty() {
        return None;
    }
    collected.reverse();
    Some(collected.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PmAstParser;

    #[test]
    fn attaches_a_line_comment_immediately_before_a_cell_decl() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // the total\n    cell b: i32 = 2;\n}";
        let mut sheet = PmAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment.as_deref(), Some("the total"));
    }

    #[test]
    fn attaches_a_multi_line_comment_block() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // line one\n    // line two\n    cell b: i32 = 2;\n}";
        let mut sheet = PmAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment.as_deref(), Some("line one\nline two"));
    }

    #[test]
    fn attaches_a_single_line_block_comment() {
        let source =
            "sheet s {\n    cell a: i32 = 1;\n    /* the total */\n    cell b: i32 = 2;\n}";
        let mut sheet = PmAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment.as_deref(), Some("the total"));
    }

    #[test]
    fn does_not_attach_a_comment_separated_by_a_blank_line() {
        let source =
            "sheet s {\n    cell a: i32 = 1;\n    // stale comment\n\n    cell b: i32 = 2;\n}";
        let mut sheet = PmAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment, None);
    }

    #[test]
    fn no_comment_in_the_gap_leaves_leading_comment_none() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    cell b: i32 = 2;\n}";
        let mut sheet = PmAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment, None);
    }

    #[test]
    fn attaches_comments_correctly_across_more_than_one_gap() {
        // Exercises the shared line-start-offset table across multiple gaps in one
        // `attach_trivia` call, confirming later gaps resolve correctly relative to earlier
        // ones rather than only the first.
        let source = "sheet s {\n    cell a: i32 = 1;\n    // first\n    cell b: i32 = 2;\n    // second\n    cell c: i32 = 3;\n}";
        let mut sheet = PmAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment.as_deref(), Some("first"));
        let crate::ast::SheetItem::Cell(c) = &sheet.items[2] else {
            panic!("expected Cell");
        };
        assert_eq!(c.leading_comment.as_deref(), Some("second"));
    }

    #[test]
    fn attaches_a_comment_preceding_a_recovered_error_item() {
        // A comment immediately before a malformed declaration must still be recovered onto
        // the SheetItem::Error placeholder, not silently dropped.
        let source = "sheet s {\n    cell a: i32 = 1;\n    // fix me\n    cell bad unknown_syntax\n    cell c: i32 = 2;\n}";
        let mut sheet = PmAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Error {
            leading_comment, ..
        } = &sheet.items[1]
        else {
            panic!("expected Error");
        };
        assert_eq!(leading_comment.as_deref(), Some("fix me"));
    }
}
