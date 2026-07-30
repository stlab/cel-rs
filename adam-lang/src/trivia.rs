//! Recovers comments and blank-line-before flags discarded/erased by `proc_macro2`'s tokenizer,
//! re-slicing the gap between two consecutive AST nodes' spans (the same technique `rustfmt` uses
//! for the identical problem — see `cel-parser/src/lex_lexer.rs`'s `test_span_preservation`), and
//! attaches each to the nearest following node. Applied recursively to every sibling list in the
//! tree — `Sheet.items`, a `RelationshipDecl`'s `methods`, a `ConditionalDecl`'s `branches` and
//! `default`, and each `ConditionalBranch`'s `relationships` — not just the top level.
//!
//! A comment is attached only if nothing but whitespace-on-the-same-line separates it from the
//! following item — a blank line between an earlier comment and the item breaks the attachment,
//! matching the common convention that a blank line ends a comment's association with what
//! follows. `blank_line_before` is set independently: it reflects whether *any* blank line
//! remained in the gap once the (possibly absent) attached trailing comment run was accounted
//! for, so `cell a;\n\n// c\ncell b;`'s blank line (before the comment, not after it) still marks
//! `b.blank_line_before` true even though the comment still attaches to `b`.
//!
//! A comment or blank line in the gap between a block's *last* item and that block's closing `}`
//! (nothing follows it) is not attached to anything and is dropped — see
//! <https://github.com/stlab/cel-rs/issues/52>.

use proc_macro2::LineColumn;

use crate::ast::{
    ConditionalBranch, ConditionalDecl, ExprSpan, MethodDecl, RelationshipDecl, Sheet,
};

/// An AST node that can carry recovered leading trivia, attached by [`attach_gaps`].
trait TriviaTarget {
    fn span(&self) -> ExprSpan;
    fn set_leading_comment(&mut self, comment: String);
    fn set_blank_line_before(&mut self, value: bool);
}

impl TriviaTarget for crate::ast::SheetItem {
    fn span(&self) -> ExprSpan {
        crate::ast::SheetItem::span(self)
    }
    fn set_leading_comment(&mut self, comment: String) {
        crate::ast::SheetItem::set_leading_comment(self, comment)
    }
    fn set_blank_line_before(&mut self, value: bool) {
        crate::ast::SheetItem::set_blank_line_before(self, value)
    }
}

impl TriviaTarget for MethodDecl {
    fn span(&self) -> ExprSpan {
        self.span
    }
    fn set_leading_comment(&mut self, comment: String) {
        self.leading_comment = Some(comment);
    }
    fn set_blank_line_before(&mut self, value: bool) {
        self.blank_line_before = value;
    }
}

impl TriviaTarget for RelationshipDecl {
    fn span(&self) -> ExprSpan {
        self.span
    }
    fn set_leading_comment(&mut self, comment: String) {
        self.leading_comment = Some(comment);
    }
    fn set_blank_line_before(&mut self, value: bool) {
        self.blank_line_before = value;
    }
}

impl TriviaTarget for ConditionalBranch {
    fn span(&self) -> ExprSpan {
        self.span
    }
    fn set_leading_comment(&mut self, comment: String) {
        self.leading_comment = Some(comment);
    }
    fn set_blank_line_before(&mut self, value: bool) {
        self.blank_line_before = value;
    }
}

/// Recovers comments/blank-lines from every gap in `sheet` — its own top-level items, and every
/// nested `relationship`/`conditional` body — attaching each to the nearest following node.
///
/// - Precondition: `sheet` was parsed from exactly `source` (unmodified), so its items' spans'
///   line/column positions resolve correctly against it.
///
/// - Complexity: O(n) in the length of `source` plus the number of nested lists — every gap's
///   `LineColumn -> byte offset` conversion reuses the shared `line_starts` table computed once
///   up front (see [`line_start_byte_offsets`]), rather than rescanning `source` per gap.
pub fn attach_trivia(source: &str, sheet: &mut Sheet) {
    let line_starts = line_start_byte_offsets(source);
    attach_gaps(source, &line_starts, &mut sheet.items);
    for item in &mut sheet.items {
        match item {
            crate::ast::SheetItem::Relationship(rel) => {
                attach_relationship(source, &line_starts, rel)
            }
            crate::ast::SheetItem::Conditional(cond) => {
                attach_conditional(source, &line_starts, cond)
            }
            crate::ast::SheetItem::Cell(_) | crate::ast::SheetItem::Error { .. } => {}
        }
    }
}

fn attach_relationship(source: &str, line_starts: &[usize], rel: &mut RelationshipDecl) {
    attach_gaps(source, line_starts, &mut rel.methods);
}

fn attach_conditional(source: &str, line_starts: &[usize], cond: &mut ConditionalDecl) {
    attach_gaps(source, line_starts, &mut cond.branches);
    for branch in &mut cond.branches {
        attach_gaps(source, line_starts, &mut branch.relationships);
        for rel in &mut branch.relationships {
            attach_relationship(source, line_starts, rel);
        }
    }
    if let Some(default) = &mut cond.default {
        attach_gaps(source, line_starts, default);
        for rel in default.iter_mut() {
            attach_relationship(source, line_starts, rel);
        }
    }
}

/// Recovers comments/blank-lines from the gaps between consecutive `items`, attaching each to the
/// nearest following item. The first item in `items` never gets a blank-line-before or comment —
/// nothing in this list precedes it (a blank line or comment between it and this list's own
/// enclosing `{` is a separate, untracked case; see the module doc's linked issue).
fn attach_gaps<T: TriviaTarget>(source: &str, line_starts: &[usize], items: &mut [T]) {
    if items.len() < 2 {
        return;
    }
    for i in 1..items.len() {
        let start = line_column_to_byte(source, line_starts, items[i - 1].span().end.end());
        let end = line_column_to_byte(source, line_starts, items[i].span().start.start());
        let gap_text = &source[start..end];
        let (comment, blank_line_before) = analyze_gap(gap_text);
        items[i].set_blank_line_before(blank_line_before);
        if let Some(comment) = comment {
            items[i].set_leading_comment(comment);
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

/// Analyzes one gap between two consecutive items: the maximal trailing run of `//` line
/// comments (or a single `/* ... */` block comment) immediately preceding the next item, if any,
/// and whether a blank line remains anywhere in what's left of the gap once that trailing run is
/// accounted for (see the module doc for why the scan order matters).
fn analyze_gap(gap: &str) -> (Option<String>, bool) {
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
    let comment = if collected.is_empty() {
        None
    } else {
        collected.reverse();
        Some(collected.join("\n"))
    };
    // `lines[0]` is always the trailing remainder of the *previous* item's own source line (the
    // fragment before the gap's first `\n`), which is empty whenever that item's last token is
    // already at end-of-line — the common case. It must be excluded here: only a line strictly
    // after it sits between two `\n`s in the original gap and can be a genuine blank line.
    let blank_line_before = lines.len() > 1 && lines[1..].iter().any(|l| l.trim().is_empty());
    (comment, blank_line_before)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AdamAstParser;

    #[test]
    fn attaches_a_line_comment_immediately_before_a_cell_decl() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // the total\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment.as_deref(), Some("the total"));
    }

    #[test]
    fn attaches_a_multi_line_comment_block() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // line one\n    // line two\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
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
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
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
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment, None);
    }

    #[test]
    fn no_comment_in_the_gap_leaves_leading_comment_none() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment, None);
    }

    #[test]
    fn attaches_comments_correctly_across_more_than_one_gap() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // first\n    cell b: i32 = 2;\n    // second\n    cell c: i32 = 3;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
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
        let source = "sheet s {\n    cell a: i32 = 1;\n    // fix me\n    cell bad unknown_syntax\n    cell c: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Error {
            leading_comment, ..
        } = &sheet.items[1]
        else {
            panic!("expected Error");
        };
        assert_eq!(leading_comment.as_deref(), Some("fix me"));
    }

    #[test]
    fn recovery_span_that_abuts_the_next_keyword_does_not_invert_the_gap() {
        let source = "sheet s { cell bad relationship { method [x] -> [y] { x } } }";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet); // must not panic
        assert!(matches!(
            sheet.items[0],
            crate::ast::SheetItem::Error { .. }
        ));
        assert!(matches!(
            sheet.items[1],
            crate::ast::SheetItem::Relationship(_)
        ));
    }

    #[test]
    fn sets_blank_line_before_true_when_a_blank_line_separates_two_items() {
        let source = "sheet s {\n    cell a: i32 = 1;\n\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert!(b.blank_line_before);
    }

    #[test]
    fn sets_blank_line_before_false_when_items_are_packed_tight() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert!(!b.blank_line_before);
    }

    #[test]
    fn a_run_of_several_blank_lines_still_just_sets_the_flag_true() {
        let source = "sheet s {\n    cell a: i32 = 1;\n\n\n\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert!(b.blank_line_before);
    }

    #[test]
    fn blank_line_before_an_attached_comment_still_sets_the_flag_true() {
        // The blank line precedes the comment, not the item — the comment still attaches to b
        // (nothing separates the comment itself from b), but the blank line further back in the
        // gap still counts as separating a from (comment + b) as a group.
        let source = "sheet s {\n    cell a: i32 = 1;\n\n    // c\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment.as_deref(), Some("c"));
        assert!(b.blank_line_before);
    }

    #[test]
    fn attaches_a_comment_and_blank_line_to_a_method_inside_a_relationship() {
        let source = "sheet s {\n    relationship {\n        method [a] -> [b] { a }\n\n        // second\n        method [b] -> [a] { b }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert_eq!(rel.methods[1].leading_comment.as_deref(), Some("second"));
        assert!(rel.methods[1].blank_line_before);
    }

    #[test]
    fn attaches_a_comment_to_a_conditional_branch() {
        let source = "sheet s {\n    conditional m {\n        0i32 => { relationship { method [a] -> [b] { a } } }\n        // one\n        1i32 => { relationship { method [a] -> [b] { a } } }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(cond.branches[1].leading_comment.as_deref(), Some("one"));
    }

    #[test]
    fn attaches_a_comment_to_a_relationship_nested_inside_a_conditional_branch() {
        let source = "sheet s {\n    conditional m {\n        0i32 => {\n            relationship { method [a] -> [b] { a } }\n            // second\n            relationship { method [b] -> [a] { b } }\n        }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(
            cond.branches[0].relationships[1].leading_comment.as_deref(),
            Some("second")
        );
    }

    #[test]
    fn attaches_a_comment_to_a_relationship_nested_inside_the_default_branch() {
        let source = "sheet s {\n    conditional m {\n        _ => {\n            relationship { method [a] -> [b] { a } }\n            // second\n            relationship { method [b] -> [a] { b } }\n        }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        let default = cond.default.as_ref().expect("default branch present");
        assert_eq!(default[1].leading_comment.as_deref(), Some("second"));
    }
}
