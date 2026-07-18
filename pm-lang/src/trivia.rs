//! Recovers comments discarded by `proc_macro2`'s tokenizer and attaches them to the nearest
//! following [`crate::ast::SheetItem`], the same re-slicing-the-gap technique `rustfmt` uses for
//! the identical problem (see `cel-parser/src/lex_lexer.rs`'s `test_span_preservation`).

use cel_parser::SourceSpan;

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
/// - Complexity: O(n) in the length of `source`.
pub fn attach_trivia(source: &str, sheet: &mut Sheet) {
    for i in 1..sheet.items.len() {
        let gap = SourceSpan {
            start: sheet.items[i - 1].span().end.end(),
            end: sheet.items[i].span().start.start(),
        };
        let byte_range = gap.to_byte_range(source);
        let gap_text = &source[byte_range];
        if let Some(comment) = trailing_comment_block(gap_text) {
            sheet.items[i].set_leading_comment(comment);
        }
    }
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
}
