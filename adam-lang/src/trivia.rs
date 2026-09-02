//! Recovers comments and blank-line-before flags discarded/erased by `proc_macro2`'s tokenizer,
//! re-slicing the gap between two consecutive AST nodes' spans (the same technique `rustfmt` uses
//! for the identical problem — see `cel-parser/src/lex_lexer.rs`'s `test_span_preservation`), and
//! attaches each to the nearest following node. Applied recursively to every sibling list in the
//! tree — `Sheet.items`, a `RelationshipDecl`'s `bindings`, a `ConditionalDecl`'s `branches` and
//! `default`, each `ConditionalBranch`'s `relationships`, and a `CellDecl`/`SourceDecl`/`OutDecl`'s
//! `require` block's `requirements` — not just the top level. Also
//! recovers a comment preceding the `sheet` keyword itself (e.g. a file header) into
//! `Sheet.leading_comment` — the one gap with no enclosing sibling list to attach via, so it's
//! handled directly against the start of `source` rather than through [`attach_gaps`].
//!
//! A comment is attached only if nothing but whitespace-on-the-same-line separates it from the
//! following item — a blank line between an earlier comment and the item breaks the attachment,
//! matching the common convention that a blank line ends a comment's association with what
//! follows. `blank_line_before` is set independently: it reflects whether *any* blank line
//! remained in the gap once the (possibly absent) attached trailing comment run was accounted
//! for, so `cell a;\n\n// c\ncell b;`'s blank line (before the comment, not after it) still marks
//! `b.blank_line_before` true even though the comment still attaches to `b`.
//!
//! A comment or blank line in the gap between a block's *last* item and that block's own closing
//! `}` (or, for a child-empty block, between its opening `{` and closing `}`) has nothing
//! following it for the above machinery to attach to, so it is recovered separately, into each
//! container's own `trailing_comment`/`blank_line_before_close` fields, by [`attach_trailing`]
//! and its special-cased sibling [`attach_conditional_trailing`] (a `CellDecl`/`SourceDecl`/
//! `OutDecl`'s `require` block, when present, uses the standard [`attach_trailing`] path
//! directly, like any other container).
//! See <https://github.com/stlab/cel-rs/issues/52>.

use proc_macro2::LineColumn;

use crate::ast::{
    BindingDecl, CellDecl, ConditionalBranch, ConditionalDecl, ExprSpan, OutDecl, RelationshipDecl,
    RequireBlock, RequirementDecl, Sheet, SourceDecl,
};

/// An AST node that can carry recovered leading trivia, attached by [`attach_gaps`].
trait TriviaTarget {
    fn span(&self) -> ExprSpan;
    fn set_leading_comment(&mut self, comment: crate::ast::Comment);
    fn set_blank_line_before(&mut self, value: bool);
}

impl TriviaTarget for crate::ast::SheetItem {
    fn span(&self) -> ExprSpan {
        crate::ast::SheetItem::span(self)
    }
    fn set_leading_comment(&mut self, comment: crate::ast::Comment) {
        crate::ast::SheetItem::set_leading_comment(self, comment)
    }
    fn set_blank_line_before(&mut self, value: bool) {
        crate::ast::SheetItem::set_blank_line_before(self, value)
    }
}

impl TriviaTarget for BindingDecl {
    fn span(&self) -> ExprSpan {
        self.span
    }
    fn set_leading_comment(&mut self, comment: crate::ast::Comment) {
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
    fn set_leading_comment(&mut self, comment: crate::ast::Comment) {
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
    fn set_leading_comment(&mut self, comment: crate::ast::Comment) {
        self.leading_comment = Some(comment);
    }
    fn set_blank_line_before(&mut self, value: bool) {
        self.blank_line_before = value;
    }
}

impl TriviaTarget for RequirementDecl {
    fn span(&self) -> ExprSpan {
        self.span
    }
    fn set_leading_comment(&mut self, comment: crate::ast::Comment) {
        self.leading_comment = Some(comment);
    }
    fn set_blank_line_before(&mut self, value: bool) {
        self.blank_line_before = value;
    }
}

/// A container whose own `{ ... }` block may carry trailing trivia — a comment or blank line
/// between its last child and its own closing `}` — recovered by [`attach_trailing`]. See
/// <https://github.com/stlab/cel-rs/issues/52>.
trait TrailingTriviaTarget {
    /// The span of this container's own opening `{`, used as the trailing gap's start when its
    /// child list is empty.
    fn open_brace_span(&self) -> proc_macro2::Span;
    /// The span of this container's own closing `}`.
    fn close_span(&self) -> proc_macro2::Span;
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment);
    fn set_blank_line_before_close(&mut self, value: bool);
}

impl TrailingTriviaTarget for Sheet {
    fn open_brace_span(&self) -> proc_macro2::Span {
        self.open_brace_span.end
    }
    fn close_span(&self) -> proc_macro2::Span {
        self.span.end
    }
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_comment = Some(comment);
    }
    fn set_blank_line_before_close(&mut self, value: bool) {
        self.blank_line_before_close = value;
    }
}

impl TrailingTriviaTarget for RelationshipDecl {
    fn open_brace_span(&self) -> proc_macro2::Span {
        self.open_brace_span.end
    }
    fn close_span(&self) -> proc_macro2::Span {
        self.span.end
    }
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_comment = Some(comment);
    }
    fn set_blank_line_before_close(&mut self, value: bool) {
        self.blank_line_before_close = value;
    }
}

impl TrailingTriviaTarget for RequireBlock {
    fn open_brace_span(&self) -> proc_macro2::Span {
        self.open_brace_span.end
    }
    fn close_span(&self) -> proc_macro2::Span {
        self.span.end
    }
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_comment = Some(comment);
    }
    fn set_blank_line_before_close(&mut self, value: bool) {
        self.blank_line_before_close = value;
    }
}

impl TrailingTriviaTarget for ConditionalBranch {
    fn open_brace_span(&self) -> proc_macro2::Span {
        self.open_brace_span.end
    }
    fn close_span(&self) -> proc_macro2::Span {
        self.span.end
    }
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_comment = Some(comment);
    }
    fn set_blank_line_before_close(&mut self, value: bool) {
        self.blank_line_before_close = value;
    }
}

impl TrailingTriviaTarget for crate::ast::DefaultBranch {
    fn open_brace_span(&self) -> proc_macro2::Span {
        self.open_brace_span.end
    }
    fn close_span(&self) -> proc_macro2::Span {
        self.span.end
    }
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_comment = Some(comment);
    }
    fn set_blank_line_before_close(&mut self, value: bool) {
        self.blank_line_before_close = value;
    }
}

/// Byte offset immediately after a container's own opening `{`.
///
/// `open_brace` is the flattened `OpenDelim` token's span, which `cel_parser::lex_lexer::LexLexer`
/// sets to the *whole* delimited group's span (`proc_macro2::Group::span()`'s documented
/// behavior) rather than to just the one-character `{` token — the same span value the matching
/// `CloseDelim` token carries. So `open_brace.start()` is the position of the `{` character
/// itself; advancing one byte (a brace is always exactly one ASCII byte) lands just past it.
fn after_open_brace(source: &str, line_starts: &[usize], open_brace: proc_macro2::Span) -> usize {
    line_column_to_byte(source, line_starts, open_brace.start()) + 1
}

/// Byte offset immediately before a container's own closing `}` — the mirror of
/// [`after_open_brace`], exploiting the same whole-group-span quirk from the other end:
/// `close_brace.end()` is the position just past the `}`; stepping back one byte lands just
/// before it.
fn before_close_brace(
    source: &str,
    line_starts: &[usize],
    close_brace: proc_macro2::Span,
) -> usize {
    line_column_to_byte(source, line_starts, close_brace.end()) - 1
}

/// Recovers trailing trivia (a comment/blank line between the last child's end position —
/// `last_child_end`, precomputed by the caller from its own child list before taking a mutable
/// borrow of `container`, since `container` may be that same list's owner — and `container`'s
/// own closing `}`, or between its opening `{` and closing `}` when `last_child_end` is `None`)
/// and attaches it to `container`. See <https://github.com/stlab/cel-rs/issues/52>.
fn attach_trailing<C: TrailingTriviaTarget>(
    source: &str,
    line_starts: &[usize],
    last_child_end: Option<LineColumn>,
    container: &mut C,
) {
    let start = match last_child_end {
        Some(pos) => line_column_to_byte(source, line_starts, pos),
        None => after_open_brace(source, line_starts, container.open_brace_span()),
    };
    let end = before_close_brace(source, line_starts, container.close_span());
    if start < end {
        let gap_text = &source[start..end];
        let (comment, blank_line_before_close) = analyze_gap(gap_text);
        container.set_blank_line_before_close(blank_line_before_close);
        if let Some(comment) = comment {
            container.set_trailing_comment(comment);
        }
    }
}

/// Recovers `ConditionalDecl`'s own trailing trivia — the gap before its outer closing `}`,
/// after its default arm if present, else its last branch, else (an empty conditional) its own
/// opening `{`. Handled specially because a `ConditionalDecl`'s "last child" isn't a single
/// homogeneous list — it's whichever of `branches`/`default` came last in declaration order.
fn attach_conditional_trailing(source: &str, line_starts: &[usize], cond: &mut ConditionalDecl) {
    let start = if let Some(default) = &cond.default {
        line_column_to_byte(source, line_starts, default.span.end.end())
    } else if let Some(last_branch) = cond.branches.last() {
        line_column_to_byte(source, line_starts, last_branch.span.end.end())
    } else {
        after_open_brace(source, line_starts, cond.open_brace_span.end)
    };
    let end = before_close_brace(source, line_starts, cond.span.end);
    if start < end {
        let gap_text = &source[start..end];
        let (comment, blank_line_before_close) = analyze_gap(gap_text);
        cond.blank_line_before_close = blank_line_before_close;
        if let Some(comment) = comment {
            cond.trailing_comment = Some(comment);
        }
    }
}

/// Recovers comments/blank-lines from every gap in `sheet` — a leading comment before the
/// `sheet` keyword itself, its own top-level items, and every nested `relationship`/`conditional`
/// body — attaching each to the nearest following node.
///
/// - Precondition: `sheet` was parsed from exactly `source` (unmodified), so its items' spans'
///   line/column positions resolve correctly against it.
///
/// - Complexity: O(n) in the length of `source` plus the number of nested lists — every gap's
///   `LineColumn -> byte offset` conversion reuses the shared `line_starts` table computed once
///   up front (see `line_start_byte_offsets`), rather than rescanning `source` per gap.
pub fn attach_trivia(source: &str, sheet: &mut Sheet) {
    let line_starts = line_start_byte_offsets(source);
    let sheet_start = line_column_to_byte(source, &line_starts, sheet.span.start.start());
    let (leading_comment, _) = analyze_gap(&source[..sheet_start]);
    sheet.leading_comment = leading_comment;
    attach_gaps(source, &line_starts, &mut sheet.items);
    let last_child_end = sheet.items.last().map(|item| item.span().end.end());
    attach_trailing(source, &line_starts, last_child_end, sheet);
    for item in &mut sheet.items {
        match item {
            crate::ast::SheetItem::Relationship(rel) => {
                attach_relationship(source, &line_starts, rel)
            }
            crate::ast::SheetItem::Conditional(cond) => {
                attach_conditional(source, &line_starts, cond)
            }
            crate::ast::SheetItem::Out(out_decl) => attach_out(source, &line_starts, out_decl),
            crate::ast::SheetItem::Cell(cell) => attach_cell(source, &line_starts, cell),
            crate::ast::SheetItem::Source(source_decl) => {
                attach_source(source, &line_starts, source_decl)
            }
            crate::ast::SheetItem::Error { .. } => {}
        }
    }
}

/// Recovers trivia for a relationship block's bindings.
fn attach_relationship(source: &str, line_starts: &[usize], rel: &mut RelationshipDecl) {
    attach_gaps(source, line_starts, &mut rel.bindings);
    let last_child_end = rel.bindings.last().map(|b| b.span().end.end());
    attach_trailing(source, line_starts, last_child_end, rel);
}

/// Recovers trivia for a conditional's branches, its default, and their nested relationships.
fn attach_conditional(source: &str, line_starts: &[usize], cond: &mut ConditionalDecl) {
    attach_gaps(source, line_starts, &mut cond.branches);
    for branch in &mut cond.branches {
        attach_gaps(source, line_starts, &mut branch.relationships);
        let last_child_end = branch.relationships.last().map(|r| r.span().end.end());
        attach_trailing(source, line_starts, last_child_end, branch);
        for rel in &mut branch.relationships {
            attach_relationship(source, line_starts, rel);
        }
    }
    if let Some(default) = &mut cond.default {
        attach_gaps(source, line_starts, &mut default.relationships);
        let last_child_end = default.relationships.last().map(|r| r.span().end.end());
        attach_trailing(source, line_starts, last_child_end, default);
        for rel in default.relationships.iter_mut() {
            attach_relationship(source, line_starts, rel);
        }
    }
    attach_conditional_trailing(source, line_starts, cond);
}

/// Recovers trivia for a `require { ... }` block, if present — the gap before its own closing
/// `}`, and gaps between its requirements. A `None` require block (a `cell`/`source`/`out` with
/// no `require { ... }` clause) has nothing further to recover here: its `initializer`/`filter`
/// expressions carry no trivia of their own. Shared by [`attach_cell`], [`attach_source`], and
/// [`attach_out`], since a `require` block has the same trivia-recovery rules regardless of which
/// cell kind it's attached to.
fn attach_require_block(source: &str, line_starts: &[usize], require: Option<&mut RequireBlock>) {
    let Some(require) = require else {
        return;
    };
    attach_gaps(source, line_starts, &mut require.requirements);
    let last_child_end = require.requirements.last().map(|r| r.span().end.end());
    attach_trailing(source, line_starts, last_child_end, require);
}

/// Recovers trivia for an `out` declaration's `require` block, if present, via
/// [`attach_require_block`].
fn attach_out(source: &str, line_starts: &[usize], out_decl: &mut OutDecl) {
    attach_require_block(source, line_starts, out_decl.require.as_mut());
}

/// Recovers trivia for a `cell` declaration's `require` block, if present, via
/// [`attach_require_block`].
fn attach_cell(source: &str, line_starts: &[usize], cell: &mut CellDecl) {
    attach_require_block(source, line_starts, cell.require.as_mut());
}

/// Recovers trivia for a `source` declaration's `require` block, if present, via
/// [`attach_require_block`].
fn attach_source(source: &str, line_starts: &[usize], source_decl: &mut SourceDecl) {
    attach_require_block(source, line_starts, source_decl.require.as_mut());
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
/// comments, or a single `/* ... */` block comment (possibly spanning multiple lines),
/// immediately preceding the next item, if any, and whether a blank line remains anywhere in
/// what's left of the gap once that trailing run is accounted for (see the module doc for why
/// the scan order matters).
///
/// - Complexity: O(n) in the length of `gap`.
fn analyze_gap(gap: &str) -> (Option<crate::ast::Comment>, bool) {
    use crate::ast::Comment;
    let mut lines: Vec<&str> = gap.lines().collect();
    // `gap` ends exactly where the following item's first token begins. When that token isn't
    // at column 0, `lines()`'s final entry is only the leading whitespace before it on its own
    // line, not a blank source line — drop that fragment before scanning for a trailing comment
    // run so a real blank line (a genuine empty entry from `lines()`) still breaks the run.
    if !gap.ends_with('\n') {
        lines.pop();
    }
    let mut comment = None;
    if let Some(last) = lines.last() {
        let trimmed = last.trim();
        if let Some(text) = trimmed.strip_prefix("//") {
            let mut collected = vec![text.trim().to_string()];
            lines.pop();
            while let Some(line) = lines.last() {
                let trimmed = line.trim();
                if let Some(text) = trimmed.strip_prefix("//") {
                    collected.push(text.trim().to_string());
                    lines.pop();
                } else {
                    break;
                }
            }
            collected.reverse();
            comment = Some(Comment::Line(collected.join("\n")));
        } else if let Some(inner) = trimmed
            .strip_prefix("/*")
            .and_then(|s| s.strip_suffix("*/"))
        {
            // A single-line block comment.
            comment = Some(Comment::Block(inner.trim().to_string()));
            lines.pop();
        } else if trimmed.ends_with("*/") {
            // The close of a block comment that opened on an earlier line — collect backwards
            // until the line that opens it (see #105). A block comment is one unit; don't merge
            // with an earlier `//` run.
            let mut collected = Vec::new();
            let closing_prefix = trimmed
                .strip_suffix("*/")
                .expect("checked ends_with(\"*/\") above")
                .trim();
            if !closing_prefix.is_empty() {
                collected.push(closing_prefix.to_string());
            }
            lines.pop();
            let mut found_open = false;
            while let Some(line) = lines.last() {
                let trimmed = line.trim();
                lines.pop();
                if let Some(text) = trimmed.strip_prefix("/*") {
                    let opening_suffix = text.trim();
                    if !opening_suffix.is_empty() {
                        collected.push(opening_suffix.to_string());
                    }
                    found_open = true;
                    break;
                }
                collected.push(trimmed.to_string());
            }
            // If the gap is exhausted without finding a matching `/*` (not expected for
            // well-formed input), `found_open` stays false and no comment is fabricated.
            if found_open {
                collected.reverse();
                comment = Some(Comment::Block(collected.join("\n")));
            }
        }
    }
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
        assert_eq!(
            b.leading_comment,
            Some(crate::ast::Comment::Line("the total".to_string()))
        );
    }

    #[test]
    fn attaches_a_multi_line_comment_block() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // line one\n    // line two\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(
            b.leading_comment,
            Some(crate::ast::Comment::Line("line one\nline two".to_string()))
        );
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
        assert_eq!(
            b.leading_comment,
            Some(crate::ast::Comment::Block("the total".to_string()))
        );
    }

    #[test]
    fn attaches_a_multi_line_block_comment() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    /*\n        line one\n        line two\n    */\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(
            b.leading_comment,
            Some(crate::ast::Comment::Block("line one\nline two".to_string()))
        );
    }

    #[test]
    fn attaches_the_issue_105_license_header_repro_as_a_block_comment() {
        let source =
            "/*\n    Copyright 2013 Adobe\n    ...\n*/\nsheet s {\n    cell a: i32 = 1;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        assert_eq!(
            sheet.leading_comment,
            Some(crate::ast::Comment::Block(
                "Copyright 2013 Adobe\n...".to_string()
            ))
        );
    }

    #[test]
    fn a_line_comment_is_recovered_as_comment_line() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // the total\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(
            b.leading_comment,
            Some(crate::ast::Comment::Line("the total".to_string()))
        );
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
        assert_eq!(
            b.leading_comment,
            Some(crate::ast::Comment::Line("first".to_string()))
        );
        let crate::ast::SheetItem::Cell(c) = &sheet.items[2] else {
            panic!("expected Cell");
        };
        assert_eq!(
            c.leading_comment,
            Some(crate::ast::Comment::Line("second".to_string()))
        );
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
        assert_eq!(
            leading_comment,
            &Some(crate::ast::Comment::Line("fix me".to_string()))
        );
    }

    #[test]
    fn recovery_span_that_abuts_the_next_keyword_does_not_invert_the_gap() {
        let source = "sheet s { cell bad relationship { y := x; } }";
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
        assert_eq!(
            b.leading_comment,
            Some(crate::ast::Comment::Line("c".to_string()))
        );
        assert!(b.blank_line_before);
    }

    #[test]
    fn attaches_a_comment_and_blank_line_to_a_binding_inside_a_relationship() {
        let source = "sheet s {\n    relationship {\n        b := a;\n\n        // second\n        a := b;\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert_eq!(
            rel.bindings[1].leading_comment,
            Some(crate::ast::Comment::Line("second".to_string()))
        );
        assert!(rel.bindings[1].blank_line_before);
    }

    #[test]
    fn attaches_a_comment_to_a_conditional_branch() {
        let source = "sheet s {\n    conditional m {\n        0i32 => { relationship { b := a; } }\n        // one\n        1i32 => { relationship { b := a; } }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(
            cond.branches[1].leading_comment,
            Some(crate::ast::Comment::Line("one".to_string()))
        );
    }

    #[test]
    fn attaches_a_comment_to_a_relationship_nested_inside_a_conditional_branch() {
        let source = "sheet s {\n    conditional m {\n        0i32 => {\n            relationship { b := a; }\n            // second\n            relationship { a := b; }\n        }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(
            cond.branches[0].relationships[1].leading_comment,
            Some(crate::ast::Comment::Line("second".to_string()))
        );
    }

    #[test]
    fn attaches_a_comment_to_a_relationship_nested_inside_the_default_branch() {
        let source = "sheet s {\n    conditional m {\n        _ => {\n            relationship { b := a; }\n            // second\n            relationship { a := b; }\n        }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        let default = cond.default.as_ref().expect("default branch present");
        assert_eq!(
            default.relationships[1].leading_comment,
            Some(crate::ast::Comment::Line("second".to_string()))
        );
    }

    #[test]
    fn attaches_a_leading_comment_before_the_sheet_itself() {
        let source = "// file header\nsheet s {\n    cell a: i32 = 1;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        assert_eq!(
            sheet.leading_comment,
            Some(crate::ast::Comment::Line("file header".to_string()))
        );
    }

    #[test]
    fn attaches_a_multi_line_leading_comment_before_the_sheet_itself() {
        let source = "// line one\n// line two\nsheet s {\n    cell a: i32 = 1;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        assert_eq!(
            sheet.leading_comment,
            Some(crate::ast::Comment::Line("line one\nline two".to_string()))
        );
    }

    #[test]
    fn no_leading_comment_before_the_sheet_leaves_it_none() {
        let source = "sheet s {\n    cell a: i32 = 1;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        assert_eq!(sheet.leading_comment, None);
    }

    #[test]
    fn attaches_a_comment_to_a_requirement_inside_an_out_declaration() {
        // NOTE: deviates from a literal per-requirement translation of the old fixture, which
        // put the commented requirement first. The old grammar could attach a leading comment to
        // a require block's *first* requirement because `out_decl`'s writer method was a real,
        // separately-spanned sibling `attach_out` computed a manual gap from; the new grammar
        // flattens `out`'s writer into a bare `initializer` expression with no span-tracked
        // sibling of its own, so `require.requirements[0]` is now subject to the same
        // never-first-item limitation as every other sibling list (see the module doc's #52
        // link). A second requirement here lands the comment in a tracked gap between two
        // siblings instead, exercising the same attach-to-a-nested-requirement behavior.
        let source = "sheet s {\n    out area: f64 := width * height require {\n        max_area: width * height <= max_area;\n        // second\n        c: width <= 10.0;\n    };\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Out(out) = &sheet.items[0] else {
            panic!("expected Out");
        };
        let require = out.require.as_ref().expect("require block present");
        assert_eq!(
            require.requirements[1].leading_comment,
            Some(crate::ast::Comment::Line("second".to_string()))
        );
    }

    #[test]
    fn recovers_a_trailing_comment_before_a_sheets_closing_brace() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // trailing\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        assert_eq!(
            sheet.trailing_comment,
            Some(crate::ast::Comment::Line("trailing".to_string()))
        );
    }

    #[test]
    fn recovers_a_trailing_comment_in_an_empty_relationship_block() {
        let source = "sheet s {\n    relationship {\n        // only this\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert_eq!(
            rel.trailing_comment,
            Some(crate::ast::Comment::Line("only this".to_string()))
        );
    }

    #[test]
    fn recovers_a_trailing_comment_before_a_relationships_closing_brace() {
        let source =
            "sheet s {\n    relationship {\n        b := a;\n        // trailing\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert_eq!(
            rel.trailing_comment,
            Some(crate::ast::Comment::Line("trailing".to_string()))
        );
    }

    #[test]
    fn recovers_a_trailing_comment_before_a_conditional_branchs_closing_brace() {
        let source = "sheet s {\n    conditional m {\n        0i32 => {\n            relationship { b := a; }\n            // trailing\n        }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(
            cond.branches[0].trailing_comment,
            Some(crate::ast::Comment::Line("trailing".to_string()))
        );
    }

    #[test]
    fn recovers_a_trailing_comment_in_a_default_arm() {
        let source = "sheet s {\n    conditional m {\n        _ => {\n            relationship { b := a; }\n            // trailing\n        }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        let default = cond.default.as_ref().expect("default branch present");
        assert_eq!(
            default.trailing_comment,
            Some(crate::ast::Comment::Line("trailing".to_string()))
        );
    }

    #[test]
    fn recovers_a_trailing_comment_before_a_conditionals_own_closing_brace() {
        let source = "sheet s {\n    conditional m {\n        0i32 => { relationship { b := a; } }\n        // trailing\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(
            cond.trailing_comment,
            Some(crate::ast::Comment::Line("trailing".to_string()))
        );
    }

    // NOTE: `recovers_a_trailing_comment_before_an_outs_closing_brace_with_no_conditions` (the old
    // no-`require`-block case) is intentionally not carried forward: `OutDecl` no longer has a
    // closing brace or a `trailing_comment` field of its own (the new grammar is flat and
    // `;`-terminated), so there is nothing left to attach such a comment to when no `require`
    // block is present — see the task brief's guidance to drop this case rather than invent one.

    #[test]
    fn recovers_a_trailing_comment_before_a_requires_closing_brace() {
        let source = "sheet s {\n    out area: f64 := w require {\n        c: w <= 10.0;\n        // trailing\n    };\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Out(out) = &sheet.items[0] else {
            panic!("expected Out");
        };
        let require = out.require.as_ref().expect("require block present");
        assert_eq!(
            require.trailing_comment,
            Some(crate::ast::Comment::Line("trailing".to_string()))
        );
    }

    #[test]
    fn attaches_a_comment_to_a_requirement_inside_a_cell_declaration() {
        // Mirrors `attaches_a_comment_to_a_requirement_inside_an_out_declaration`, but for a
        // `cell`'s own `require` block — `CellDecl` gained `require` alongside `out`'s, and its
        // trivia recovery must not be dropped on the floor the way it was before this fix.
        let source = "sheet s {\n    cell a: i32 = 1 require {\n        r1: a > 0;\n        // second\n        r2: a < 10;\n    };\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        let require = cell.require.as_ref().expect("require block present");
        assert_eq!(
            require.requirements[1].leading_comment,
            Some(crate::ast::Comment::Line("second".to_string()))
        );
    }

    #[test]
    fn recovers_a_trailing_comment_before_a_cells_requires_closing_brace() {
        // Mirrors `recovers_a_trailing_comment_before_a_requires_closing_brace`, but for a
        // `cell`'s own `require` block.
        let source = "sheet s {\n    cell a: i32 = 1 require {\n        r: a > 0;\n        // trailing\n    };\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        let require = cell.require.as_ref().expect("require block present");
        assert_eq!(
            require.trailing_comment,
            Some(crate::ast::Comment::Line("trailing".to_string()))
        );
    }

    #[test]
    fn attaches_a_comment_to_a_requirement_inside_a_source_declaration() {
        // Mirrors `attaches_a_comment_to_a_requirement_inside_an_out_declaration`, but for a
        // `source`'s own `require` block.
        let source = "sheet s {\n    source a: i32 = 1 require {\n        r1: a > 0;\n        // second\n        r2: a < 10;\n    };\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Source(source_decl) = &sheet.items[0] else {
            panic!("expected Source");
        };
        let require = source_decl.require.as_ref().expect("require block present");
        assert_eq!(
            require.requirements[1].leading_comment,
            Some(crate::ast::Comment::Line("second".to_string()))
        );
    }

    #[test]
    fn recovers_a_trailing_comment_before_a_sources_requires_closing_brace() {
        // Mirrors `recovers_a_trailing_comment_before_a_requires_closing_brace`, but for a
        // `source`'s own `require` block.
        let source = "sheet s {\n    source a: i32 = 1 require {\n        r: a > 0;\n        // trailing\n    };\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Source(source_decl) = &sheet.items[0] else {
            panic!("expected Source");
        };
        let require = source_decl.require.as_ref().expect("require block present");
        assert_eq!(
            require.trailing_comment,
            Some(crate::ast::Comment::Line("trailing".to_string()))
        );
    }

    #[test]
    fn sets_blank_line_before_close_when_a_blank_line_precedes_the_closing_brace() {
        let source = "sheet s {\n    cell a: i32 = 1;\n\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        assert!(sheet.blank_line_before_close);
    }

    #[test]
    fn no_trailing_comment_leaves_trailing_comment_none() {
        let source = "sheet s {\n    cell a: i32 = 1;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        assert_eq!(sheet.trailing_comment, None);
    }
}
