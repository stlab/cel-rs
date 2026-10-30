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
//! Gap comments remain in source order even when blank lines detach them from an item.
//! A single comment retains its style; multiple detached or mixed runs normalize to line-comment
//! text because each ordinary AST attachment stores one comment. `blank_line_before` records separation
//! independently: it reflects whether *any* blank line
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
//!
//! Attachment costs below use `v` for visited AST nodes, `g` for the sum of scanned
//! gap bytes, `c` for the sum of character-column prefixes traversed by coordinate
//! conversions, and `h` for attachment call depth. Repeated scans count repeatedly.
//! A shared line-start table avoids rebuilding the index, but conversions still
//! traverse each line's character prefix; many items on one long line can make
//! `c` quadratic in that line's length. Recovered comment storage is included in
//! the space bounds.
//!
//! A `//`/`/* */` comment ending or beginning on the same source line as an adjacent node's own
//! first/last token — rather than on a fresh line of its own — attaches as that adjacent node's
//! own `trailing_line_comment` instead of the following node's `leading_comment`; see
//! [`analyze_gap`]'s `same_line` result and <https://github.com/stlab/cel-rs/issues/59>. This
//! reuses the same node-to-node gaps described above, so it covers every sibling list and
//! closing-brace gap this module already tracks, but goes no further: a comment written *inside*
//! a CEL expression body (between two sub-expression tokens, with no adam-lang-tracked node
//! boundary between them) is invisible to this whole gap-scanning mechanism and is silently
//! dropped on reformat — see <https://github.com/stlab/cel-rs/issues/201>.

use cel_parser::trivia::{line_column_to_byte, line_start_byte_offsets};
use proc_macro2::LineColumn;

use crate::ast::{
    BindingDecl, CellDecl, ConditionalBranch, ConditionalDecl, ExprSpan, OutDecl, RelationshipDecl,
    RequireBlock, RequirementDecl, Sheet, SourceDecl,
};

/// An AST node that can carry recovered leading trivia, attached by [`attach_gaps`].
trait TriviaTarget {
    /// Returns storage for complete opening-gap comments when this node supports it.
    fn opening_comments_mut(&mut self) -> Option<&mut Vec<crate::ast::Comment>> {
        None
    }

    /// Returns this node's source range.
    fn span(&self) -> ExprSpan;
    /// Replaces this node's leading comment.
    fn set_leading_comment(&mut self, comment: crate::ast::Comment);
    /// Sets a `//`/`/* */` comment recovered on the same source line as this node's own last
    /// token, immediately before the next sibling (or the enclosing container's own closing
    /// `}`). See <https://github.com/stlab/cel-rs/issues/59>.
    fn set_trailing_line_comment(&mut self, comment: crate::ast::Comment);
    /// Records whether a blank line precedes this node.
    fn set_blank_line_before(&mut self, value: bool);
}

impl TriviaTarget for crate::ast::SheetItem {
    /// Returns the complete opening-comment sequence for a named relationship use.
    fn opening_comments_mut(&mut self) -> Option<&mut Vec<crate::ast::Comment>> {
        match self {
            Self::RelationshipUse(call) => Some(&mut call.opening_comments),
            _ => None,
        }
    }

    /// Returns the selected declaration's source range.
    fn span(&self) -> ExprSpan {
        crate::ast::SheetItem::span(self)
    }
    /// Replaces the selected declaration's leading comment.
    fn set_leading_comment(&mut self, comment: crate::ast::Comment) {
        crate::ast::SheetItem::set_leading_comment(self, comment)
    }
    /// Replaces the selected declaration's same-line trailing comment.
    fn set_trailing_line_comment(&mut self, comment: crate::ast::Comment) {
        crate::ast::SheetItem::set_trailing_line_comment(self, comment)
    }
    /// Records whether a blank line precedes the selected declaration.
    fn set_blank_line_before(&mut self, value: bool) {
        crate::ast::SheetItem::set_blank_line_before(self, value)
    }
}

impl TriviaTarget for BindingDecl {
    /// Returns the binding's source range.
    fn span(&self) -> ExprSpan {
        self.span
    }
    /// Replaces the binding's leading comment.
    fn set_leading_comment(&mut self, comment: crate::ast::Comment) {
        self.leading_comment = Some(comment);
    }
    /// Replaces the binding's same-line trailing comment.
    fn set_trailing_line_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_line_comment = Some(comment);
    }
    /// Records whether a blank line precedes the binding.
    fn set_blank_line_before(&mut self, value: bool) {
        self.blank_line_before = value;
    }
}

impl TriviaTarget for RelationshipDecl {
    /// Returns the relationship's source range.
    fn span(&self) -> ExprSpan {
        self.span
    }
    /// Replaces the relationship's leading comment.
    fn set_leading_comment(&mut self, comment: crate::ast::Comment) {
        self.leading_comment = Some(comment);
    }
    /// Replaces the relationship's same-line trailing comment.
    fn set_trailing_line_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_line_comment = Some(comment);
    }
    /// Records whether a blank line precedes the relationship.
    fn set_blank_line_before(&mut self, value: bool) {
        self.blank_line_before = value;
    }
}

impl TriviaTarget for crate::ast::BranchItem {
    /// Returns the complete opening-comment sequence for a named relationship use.
    fn opening_comments_mut(&mut self) -> Option<&mut Vec<crate::ast::Comment>> {
        match self {
            Self::RelationshipUse(call) => Some(&mut call.opening_comments),
            _ => None,
        }
    }

    /// Returns the inline relationship or component use span.
    fn span(&self) -> ExprSpan {
        match self {
            Self::Relationship(rel) => rel.span,
            Self::RelationshipUse(call) => call.span,
        }
    }

    /// Attaches a leading comment to this branch item.
    fn set_leading_comment(&mut self, comment: crate::ast::Comment) {
        match self {
            Self::Relationship(rel) => rel.leading_comment = Some(comment),
            Self::RelationshipUse(call) => call.leading_comment = Some(comment),
        }
    }

    /// Attaches a same-line comment to this branch item.
    fn set_trailing_line_comment(&mut self, comment: crate::ast::Comment) {
        match self {
            Self::Relationship(rel) => rel.trailing_line_comment = Some(comment),
            Self::RelationshipUse(call) => call.trailing_line_comment = Some(comment),
        }
    }

    /// Records a preceding blank line.
    fn set_blank_line_before(&mut self, value: bool) {
        match self {
            Self::Relationship(rel) => rel.blank_line_before = value,
            Self::RelationshipUse(call) => call.blank_line_before = value,
        }
    }
}

impl TriviaTarget for ConditionalBranch {
    /// Returns the named branch's source range.
    fn span(&self) -> ExprSpan {
        self.span
    }
    /// Replaces the named branch's leading comment.
    fn set_leading_comment(&mut self, comment: crate::ast::Comment) {
        self.leading_comment = Some(comment);
    }
    /// Replaces the named branch's same-line trailing comment.
    fn set_trailing_line_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_line_comment = Some(comment);
    }
    /// Records whether a blank line precedes the named branch.
    fn set_blank_line_before(&mut self, value: bool) {
        self.blank_line_before = value;
    }
}

impl TriviaTarget for RequirementDecl {
    /// Returns the requirement's source range.
    fn span(&self) -> ExprSpan {
        self.span
    }
    /// Replaces the requirement's leading comment.
    fn set_leading_comment(&mut self, comment: crate::ast::Comment) {
        self.leading_comment = Some(comment);
    }
    /// Replaces the requirement's same-line trailing comment.
    fn set_trailing_line_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_line_comment = Some(comment);
    }
    /// Records whether a blank line precedes the requirement.
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
    /// Replaces the comment immediately before the closing brace.
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment);
    /// Records whether a blank line precedes the closing brace.
    fn set_blank_line_before_close(&mut self, value: bool);
}

impl TrailingTriviaTarget for Sheet {
    /// Returns the sheet's opening brace span.
    fn open_brace_span(&self) -> proc_macro2::Span {
        self.open_brace_span.end
    }
    /// Returns the sheet's closing brace span.
    fn close_span(&self) -> proc_macro2::Span {
        self.span.end
    }
    /// Replaces the sheet's trailing interior comment.
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_comment = Some(comment);
    }
    /// Records whether a blank line precedes the sheet's closing brace.
    fn set_blank_line_before_close(&mut self, value: bool) {
        self.blank_line_before_close = value;
    }
}

impl TrailingTriviaTarget for RelationshipDecl {
    /// Returns the relationship's opening brace span.
    fn open_brace_span(&self) -> proc_macro2::Span {
        self.open_brace_span.end
    }
    /// Returns the relationship's closing brace span.
    fn close_span(&self) -> proc_macro2::Span {
        self.span.end
    }
    /// Replaces the relationship's trailing interior comment.
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_comment = Some(comment);
    }
    /// Records whether a blank line precedes the relationship's closing brace.
    fn set_blank_line_before_close(&mut self, value: bool) {
        self.blank_line_before_close = value;
    }
}

impl TrailingTriviaTarget for RequireBlock {
    /// Returns the requirement block's opening brace span.
    fn open_brace_span(&self) -> proc_macro2::Span {
        self.open_brace_span.end
    }
    /// Returns the requirement block's closing brace span.
    fn close_span(&self) -> proc_macro2::Span {
        self.span.end
    }
    /// Replaces the requirement block's trailing interior comment.
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_comment = Some(comment);
    }
    /// Records whether a blank line precedes the requirement block's closing brace.
    fn set_blank_line_before_close(&mut self, value: bool) {
        self.blank_line_before_close = value;
    }
}

impl TrailingTriviaTarget for ConditionalBranch {
    /// Returns the named branch's opening brace span.
    fn open_brace_span(&self) -> proc_macro2::Span {
        self.open_brace_span.end
    }
    /// Returns the named branch's closing brace span.
    fn close_span(&self) -> proc_macro2::Span {
        self.span.end
    }
    /// Replaces the named branch's trailing interior comment.
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_comment = Some(comment);
    }
    /// Records whether a blank line precedes the named branch's closing brace.
    fn set_blank_line_before_close(&mut self, value: bool) {
        self.blank_line_before_close = value;
    }
}

impl TrailingTriviaTarget for crate::ast::DefaultBranch {
    /// Returns the default arm's opening brace span.
    fn open_brace_span(&self) -> proc_macro2::Span {
        self.open_brace_span.end
    }
    /// Returns the default arm's closing brace span.
    fn close_span(&self) -> proc_macro2::Span {
        self.span.end
    }
    /// Replaces the default arm's trailing interior comment.
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_comment = Some(comment);
    }
    /// Records whether a blank line precedes the default arm's closing brace.
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
///
/// - Complexity: O(c) time for the opening position's character column; O(1) space.
fn after_open_brace(source: &str, line_starts: &[usize], open_brace: proc_macro2::Span) -> usize {
    line_column_to_byte(source, line_starts, open_brace.start()) + 1
}

/// Byte offset immediately before a container's own closing `}` — the mirror of
/// [`after_open_brace`], exploiting the same whole-group-span quirk from the other end:
/// `close_brace.end()` is the position just past the `}`; stepping back one byte lands just
/// before it.
///
/// - Complexity: O(c) time for the closing position's character column; O(1) space.
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
///
/// When the recovered comment's first line begins on the same source line as the last child's
/// own last token, it belongs to that child's `trailing_line_comment` instead of `container`'s
/// own `trailing_comment` (see <https://github.com/stlab/cel-rs/issues/59>) — but this function
/// has no access to the child list itself (only its precomputed end position), so it returns the
/// comment to the caller in that case instead of attaching it, leaving the caller (which does
/// have the list) to set it on `items.last_mut()`. Returns `None` when there is no such same-line
/// comment to route (either no comment was found, or it was a fresh leading-style comment and has
/// already been attached directly to `container`).
///
/// - Complexity: O(g + c) time and O(g) space for scanned gap bytes and converted prefixes.
fn attach_trailing<C: TrailingTriviaTarget>(
    source: &str,
    line_starts: &[usize],
    last_child_end: Option<LineColumn>,
    container: &mut C,
) -> Option<crate::ast::Comment> {
    let start = match last_child_end {
        Some(pos) => line_column_to_byte(source, line_starts, pos),
        None => after_open_brace(source, line_starts, container.open_brace_span()),
    };
    let end = before_close_brace(source, line_starts, container.close_span());
    if start >= end {
        return None;
    }
    let gap_text = &source[start..end];
    let (comment, blank_line_before_close, same_line) = analyze_gap(gap_text);
    container.set_blank_line_before_close(blank_line_before_close);
    match comment {
        Some(comment) if same_line && last_child_end.is_some() => Some(comment),
        Some(comment) => {
            container.set_trailing_comment(comment);
            None
        }
        None => None,
    }
}

/// Recovers `ConditionalDecl`'s own trailing trivia — the gap before its outer closing `}`,
/// after its default arm if present, else its last branch, else (an empty conditional) its own
/// opening `{`. Handled specially because a `ConditionalDecl`'s "last child" isn't a single
/// homogeneous list — it's whichever of `branches`/`default` came last in declaration order.
///
/// Same-line routing mirrors [`attach_trailing`]: returns the comment instead of attaching it to
/// `cond` when it belongs on the same line as the default arm's (or last branch's) own last
/// token, leaving the caller to attach it there. See <https://github.com/stlab/cel-rs/issues/59>.
///
/// - Complexity: O(g + c) time and O(g) space for scanned gap bytes and converted prefixes.
fn attach_conditional_trailing(
    source: &str,
    line_starts: &[usize],
    cond: &mut ConditionalDecl,
) -> Option<crate::ast::Comment> {
    let has_last_child = cond.default.is_some() || !cond.branches.is_empty();
    let start = if let Some(default) = &cond.default {
        line_column_to_byte(source, line_starts, default.span.end.end())
    } else if let Some(last_branch) = cond.branches.last() {
        line_column_to_byte(source, line_starts, last_branch.span.end.end())
    } else {
        after_open_brace(source, line_starts, cond.open_brace_span.end)
    };
    let end = before_close_brace(source, line_starts, cond.span.end);
    if start >= end {
        return None;
    }
    let gap_text = &source[start..end];
    let (comment, blank_line_before_close, same_line) = analyze_gap(gap_text);
    cond.blank_line_before_close = blank_line_before_close;
    match comment {
        Some(comment) if same_line && has_last_child => Some(comment),
        Some(comment) => {
            cond.trailing_comment = Some(comment);
            None
        }
        None => None,
    }
}

/// Recovers comments/blank-lines from every gap in `sheet` — a leading comment before the
/// `sheet` keyword itself, its own top-level items, and every nested `relationship`/`conditional`
/// body — attaching each to the nearest following node.
///
/// - Precondition: `sheet` was parsed from exactly `source` (unmodified), so its items' spans'
///   line/column positions resolve correctly against it.
/// - Postcondition: preserves comment payload order. Multiple mixed or detached runs
///   sharing one ordinary AST attachment normalize to line-comment text; a single
///   comment retains its style.
///
/// - Complexity: O(n + v + g + c) time for source bytes n, visited AST nodes v,
///   summed scanned gap bytes g, and summed converted character-column prefixes c.
///   O(l + g + h) space for source lines l, recovered/scanned gaps, and call depth h.
///   Sharing line starts does not eliminate repeated prefix scans; c can be quadratic
///   for many declarations on one long line.
///
/// # Examples
///
/// See [`crate::format_sheet`] for recovering trivia before formatting a parsed sheet.
pub fn attach_trivia(source: &str, sheet: &mut Sheet) {
    let line_starts = line_start_byte_offsets(source);
    let sheet_start = line_column_to_byte(source, &line_starts, sheet.span.start.start());
    let (leading_comment, _, _) = analyze_gap(&source[..sheet_start]);
    sheet.leading_comment = leading_comment;
    attach_sheet_body(source, &line_starts, sheet);
}

/// Attaches nested sheet trivia using the caller's shared source-offset table.
///
/// - Precondition: spans and offsets refer to `source`.
/// - Complexity: O(v + g + c) time and O(g + h) space for the sheet and its nested
///   lists, using the module's attachment cost dimensions.
fn attach_sheet_body(source: &str, line_starts: &[usize], sheet: &mut Sheet) {
    attach_opening(source, line_starts, sheet.open_brace_span, &mut sheet.items);
    attach_gaps(source, line_starts, &mut sheet.items);
    let last_child_end = sheet.items.last().map(|item| item.span().end.end());
    if let Some(comment) = attach_trailing(source, line_starts, last_child_end, sheet) {
        sheet
            .items
            .last_mut()
            .expect("same-line routing only returns Some when a last child exists")
            .set_trailing_line_comment(comment);
    }
    for item in &mut sheet.items {
        match item {
            crate::ast::SheetItem::Relationship(rel) => {
                attach_relationship(source, line_starts, rel)
            }
            crate::ast::SheetItem::Conditional(cond) => {
                attach_conditional(source, line_starts, cond)
            }
            crate::ast::SheetItem::Out(out_decl) => attach_out(source, line_starts, out_decl),
            crate::ast::SheetItem::Cell(cell) => attach_cell(source, line_starts, cell),
            crate::ast::SheetItem::Source(source_decl) => {
                attach_source(source, line_starts, source_decl)
            }
            crate::ast::SheetItem::RelationshipUse(_) | crate::ast::SheetItem::Error { .. } => {}
        }
    }
}

/// Attaches every file-boundary comment and each sheet's existing nested trivia.
///
/// - Precondition: `module` was parsed from exactly `source`.
/// - Postcondition: each file-boundary comment belongs to exactly one gap.
/// - Complexity: O(n + v + g + c) time for source bytes n, visited AST nodes v,
///   summed scanned gap bytes g, and summed converted character-column prefixes c,
///   including every nested attachment. O(l + g + h) space for source lines l,
///   recovered/scanned gaps, and call depth h. Repeated prefix scans can be quadratic
///   for many declarations on one long line.
///
/// # Examples
///
/// ```
/// let source = "// header\nsheet root {}\n// end\n";
/// let mut module = adam_lang::AdamAstParser::new().parse_module_str(source).unwrap();
/// adam_lang::attach_module_trivia(source, &mut module);
/// assert_eq!(module.comments.len(), 2);
/// ```
pub fn attach_module_trivia(source: &str, module: &mut crate::ast::Module) {
    use cel_parser::trivia::{GapPiece, scan_gap};
    let line_starts = line_start_byte_offsets(source);
    module.comments.clear();
    let mut previous_end = 0;
    for item in &mut module.items {
        let span = item.span();
        let start = line_column_to_byte(source, &line_starts, span.start.start());
        module.comments.push(
            scan_gap(source.get(previous_end..start).unwrap_or(""), &[])
                .into_iter()
                .filter_map(|piece| match piece {
                    GapPiece::Comment(comment) => Some(comment),
                    GapPiece::Punct(_) => None,
                })
                .collect(),
        );
        previous_end = line_column_to_byte(source, &line_starts, span.end.end());
        if let crate::ast::ModuleItem::Sheet(sheet) = item {
            attach_sheet_body(source, &line_starts, sheet);
            sheet.leading_comment = None;
        }
    }
    module.comments.push(
        scan_gap(&source[previous_end..], &[])
            .into_iter()
            .filter_map(|piece| match piece {
                GapPiece::Comment(comment) => Some(comment),
                GapPiece::Punct(_) => None,
            })
            .collect(),
    );
}

/// Recovers trivia for a relationship block's bindings.
///
/// - Complexity: O(v + g + c) time and O(g) space for its bindings, scanned gaps,
///   and converted prefixes.
fn attach_relationship(source: &str, line_starts: &[usize], rel: &mut RelationshipDecl) {
    attach_opening(source, line_starts, rel.open_brace_span, &mut rel.bindings);
    attach_gaps(source, line_starts, &mut rel.bindings);
    let last_child_end = rel.bindings.last().map(|b| b.span().end.end());
    if let Some(comment) = attach_trailing(source, line_starts, last_child_end, rel) {
        rel.bindings
            .last_mut()
            .expect("same-line routing only returns Some when a last child exists")
            .set_trailing_line_comment(comment);
    }
}

/// Recovers trivia for a conditional's branches, its default, and their nested relationships.
///
/// - Complexity: O(v + g + c) time and O(g + h) space for every visited descendant,
///   using the module's attachment cost dimensions.
fn attach_conditional(source: &str, line_starts: &[usize], cond: &mut ConditionalDecl) {
    attach_opening(
        source,
        line_starts,
        cond.open_brace_span,
        &mut cond.branches,
    );
    attach_gaps(source, line_starts, &mut cond.branches);
    for branch in &mut cond.branches {
        attach_opening(
            source,
            line_starts,
            branch.open_brace_span,
            &mut branch.relationships,
        );
        attach_gaps(source, line_starts, &mut branch.relationships);
        let last_child_end = branch.relationships.last().map(|r| r.span().end.end());
        if let Some(comment) = attach_trailing(source, line_starts, last_child_end, branch) {
            branch
                .relationships
                .last_mut()
                .expect("same-line routing only returns Some when a last child exists")
                .set_trailing_line_comment(comment);
        }
        for rel in &mut branch.relationships {
            if let crate::ast::BranchItem::Relationship(rel) = rel {
                attach_relationship(source, line_starts, rel);
            }
        }
    }
    if let Some(default) = &mut cond.default {
        let start = cond.branches.last().map_or_else(
            || after_open_brace(source, line_starts, cond.open_brace_span.end),
            |branch| line_column_to_byte(source, line_starts, branch.span.end.end()),
        );
        let end = line_column_to_byte(source, line_starts, default.span.start.start());
        let (comment, blank, same_line) = analyze_gap(source.get(start..end).unwrap_or(""));
        default.blank_line_before = blank;
        if same_line && let Some(previous) = cond.branches.last_mut() {
            previous.trailing_line_comment = comment;
        } else {
            default.leading_comment = comment;
        }
        attach_opening(
            source,
            line_starts,
            default.open_brace_span,
            &mut default.relationships,
        );
        attach_gaps(source, line_starts, &mut default.relationships);
        let last_child_end = default.relationships.last().map(|r| r.span().end.end());
        if let Some(comment) = attach_trailing(source, line_starts, last_child_end, default) {
            default
                .relationships
                .last_mut()
                .expect("same-line routing only returns Some when a last child exists")
                .set_trailing_line_comment(comment);
        }
        for rel in default.relationships.iter_mut() {
            if let crate::ast::BranchItem::Relationship(rel) = rel {
                attach_relationship(source, line_starts, rel);
            }
        }
    }
    if let Some(comment) = attach_conditional_trailing(source, line_starts, cond) {
        if let Some(default) = &mut cond.default {
            default.trailing_line_comment = Some(comment);
        } else if let Some(last_branch) = cond.branches.last_mut() {
            last_branch.set_trailing_line_comment(comment);
        }
    }
}

/// Recovers trivia for a `require { ... }` block, if present — the gap before its own closing
/// `}`, and gaps between its requirements. A `None` require block (a `cell`/`source`/`out` with
/// no `require { ... }` clause) has nothing further to recover here: its `initializer`/`filter`
/// expressions carry no trivia of their own. Shared by [`attach_cell`], [`attach_source`], and
/// [`attach_out`], since a `require` block has the same trivia-recovery rules regardless of which
/// cell kind it's attached to.
///
/// - Complexity: O(v + g + c) time and O(g) space for requirements, scanned gaps,
///   and converted prefixes; O(1) when no block exists.
fn attach_require_block(source: &str, line_starts: &[usize], require: Option<&mut RequireBlock>) {
    let Some(require) = require else {
        return;
    };
    attach_opening(
        source,
        line_starts,
        require.open_brace_span,
        &mut require.requirements,
    );
    attach_gaps(source, line_starts, &mut require.requirements);
    let last_child_end = require.requirements.last().map(|r| r.span().end.end());
    if let Some(comment) = attach_trailing(source, line_starts, last_child_end, require) {
        require
            .requirements
            .last_mut()
            .expect("same-line routing only returns Some when a last child exists")
            .set_trailing_line_comment(comment);
    }
}

/// Recovers trivia for an `out` declaration's `require` block, if present, via
/// [`attach_require_block`].
///
/// - Complexity: the time and space of [`attach_require_block`].
fn attach_out(source: &str, line_starts: &[usize], out_decl: &mut OutDecl) {
    attach_require_block(source, line_starts, out_decl.require.as_mut());
}

/// Recovers trivia for a `cell` declaration's `require` block, if present, via
/// [`attach_require_block`].
///
/// - Complexity: the time and space of [`attach_require_block`].
fn attach_cell(source: &str, line_starts: &[usize], cell: &mut CellDecl) {
    attach_require_block(source, line_starts, cell.require.as_mut());
}

/// Recovers trivia for a `source` declaration's `require` block, if present, via
/// [`attach_require_block`].
///
/// - Complexity: the time and space of [`attach_require_block`].
fn attach_source(source: &str, line_starts: &[usize], source_decl: &mut SourceDecl) {
    attach_require_block(source, line_starts, source_decl.require.as_mut());
}

/// Attaches trivia between a container's opening brace and its first child.
///
/// - Precondition: spans and offsets refer to `source`.
/// - Complexity: O(g + c) time and O(g) space for opening-gap bytes and converted prefixes.
fn attach_opening<T: TriviaTarget>(
    source: &str,
    line_starts: &[usize],
    open: ExprSpan,
    items: &mut [T],
) {
    if let Some(first) = items.first_mut() {
        let start = after_open_brace(source, line_starts, open.end);
        let end = line_column_to_byte(source, line_starts, first.span().start.start());
        let gap = source.get(start..end).unwrap_or("");
        if let Some(comments) = first.opening_comments_mut() {
            use cel_parser::trivia::{GapPiece, scan_gap};
            *comments = scan_gap(gap, &[])
                .into_iter()
                .filter_map(|piece| match piece {
                    GapPiece::Comment(comment) => Some(comment),
                    GapPiece::Punct(_) => None,
                })
                .collect();
            return;
        }
        let (comment, blank, _) = analyze_gap(gap);
        if let Some(comment) = comment {
            first.set_leading_comment(comment);
        }
        first.set_blank_line_before(blank);
    }
}

/// Recovers comments/blank-lines from the gaps between consecutive `items`, attaching each to the
/// nearest following item — unless the comment's first line begins on the same source line as
/// the *previous* item's own last token, in which case it attaches to that previous item's
/// `trailing_line_comment` instead (see <https://github.com/stlab/cel-rs/issues/59>). The first
/// item's opening-brace gap belongs to [`attach_opening`], not this sibling-gap pass.
///
/// - Complexity: O(v + g + c) time for sibling count v, summed scanned gap bytes g,
///   and summed converted character-column prefixes c; O(g) space for recovered
///   comments and scanning. The line index does not cache character-prefix conversions.
fn attach_gaps<T: TriviaTarget>(source: &str, line_starts: &[usize], items: &mut [T]) {
    if items.len() < 2 {
        return;
    }
    for i in 1..items.len() {
        let start = line_column_to_byte(source, line_starts, items[i - 1].span().end.end());
        let end = line_column_to_byte(source, line_starts, items[i].span().start.start());
        let gap_text = &source[start..end];
        let (comment, blank_line_before, same_line) = analyze_gap(gap_text);
        items[i].set_blank_line_before(blank_line_before);
        if let Some(comment) = comment {
            if same_line {
                items[i - 1].set_trailing_line_comment(comment);
            } else {
                items[i].set_leading_comment(comment);
            }
        }
    }
}

/// Detects a complete blank line between a region's initial and final partial lines.
///
/// - Complexity: O(n) time for n region bytes; O(1) additional space.
fn has_blank_line(region: &str) -> bool {
    region
        .split_inclusive('\n')
        .skip(1)
        .any(|line| line.ends_with('\n') && line.trim().is_empty())
}

/// Recovers every comment payload, blank-line spacing, and same-line attachment from one gap.
///
/// A single comment retains its style. Multiple runs normalize into one line-comment
/// attachment in source order. The first comment determines same-line routing.
///
/// - Complexity: O(n) time and space in gap bytes, including normalized comment text.
fn analyze_gap(gap: &str) -> (Option<crate::ast::Comment>, bool, bool) {
    use crate::ast::Comment;
    use cel_parser::trivia::{GapPiece, scan_gap};
    let mut comments = scan_gap(gap, &[])
        .into_iter()
        .filter_map(|piece| match piece {
            GapPiece::Comment(comment) => Some(comment),
            GapPiece::Punct(_) => None,
        });
    let first = comments.next();
    let prefix = &gap[..gap.len() - gap.trim_start().len()];
    let same_line = first.is_some() && !prefix.contains('\n');
    if let Some(second) = comments.next() {
        // A single AST attachment stores mixed/detached runs as safe line-comment text.
        let text = first
            .into_iter()
            .chain(std::iter::once(second))
            .chain(comments)
            .enumerate()
            .fold(String::new(), |mut joined, (index, comment)| {
                if index != 0 {
                    joined.push('\n');
                }
                match comment {
                    Comment::Line(text) | Comment::Block(text) => joined.push_str(&text),
                }
                joined
            });
        return (Some(Comment::Line(text)), has_blank_line(gap), same_line);
    }
    let blank_line_before = if first.is_some() {
        let suffix = &gap[gap.trim_end().len()..];
        has_blank_line(prefix) || has_blank_line(suffix)
    } else {
        has_blank_line(gap)
    };
    (first, blank_line_before, same_line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AdamAstParser;

    /// Retains an empty first comment when normalizing a mixed-style run.
    #[test]
    fn mixed_comment_runs_preserve_empty_first_payload() {
        let (comment, _, _) = analyze_gap("\n//\n/* second */\n");
        assert_eq!(comment, Some(crate::ast::Comment::Line("\nsecond".into())));
    }

    /// Recognizes whitespace-only blank lines between mixed runs under LF and CRLF.
    #[test]
    fn mixed_comment_blank_lines_allow_whitespace_and_crlf() {
        for separator in ["\n \t\n", "\r\n \t\r\n"] {
            let gap = format!("\n// first{separator}/* second */\n    ");
            let (comment, blank, same_line) = analyze_gap(&gap);
            assert!(blank, "{gap:?}");
            assert!(!same_line);
            assert_eq!(
                comment,
                Some(crate::ast::Comment::Line("first\nsecond".into()))
            );
        }
        let (_, blank, _) = analyze_gap("\n// first\n/* second */\n    ");
        assert!(!blank, "next-item indentation is not a blank line");
    }

    /// Joins consecutive leading line comments in source order.
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

    /// Preserves a single-line block comment's style and text.
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

    /// Preserves a multiline block comment's style and text.
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

    /// Retains the license-header regression fixture as a block comment.
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

    /// Retains a detached comment rather than dropping its text.
    #[test]
    fn preserves_a_comment_separated_by_a_blank_line() {
        let source =
            "sheet s {\n    cell a: i32 = 1;\n    // stale comment\n\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(
            b.leading_comment,
            Some(crate::ast::Comment::Line("stale comment".into()))
        );
    }

    /// Leaves the leading comment absent when the gap contains no comment.
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

    /// Routes comments independently across successive declaration gaps.
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

    /// Attaches leading trivia to a syntax-recovery placeholder.
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

    /// Keeps recovery-adjacent gaps valid when the error ends at the next keyword.
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

    /// Marks a blank line separating declarations.
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

    /// Leaves the blank-line flag clear for adjacent declarations.
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

    /// Collapses multiple blank lines into one spacing flag.
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

    /// Retains blank-line spacing before an attached comment.
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

    /// Attaches leading comments and spacing to nested bindings.
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

    /// Attaches leading comments to named conditional branches.
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

    /// Attaches comments to relationships within a named branch.
    #[test]
    fn attaches_a_comment_to_a_relationship_nested_inside_a_conditional_branch() {
        let source = "sheet s {\n    conditional m {\n        0i32 => {\n            relationship { b := a; }\n            // second\n            relationship { a := b; }\n        }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(
            match &cond.branches[0].relationships[1] {
                crate::ast::BranchItem::Relationship(rel) => &rel.leading_comment,
                crate::ast::BranchItem::RelationshipUse(call) => &call.leading_comment,
            },
            &Some(crate::ast::Comment::Line("second".to_string()))
        );
    }

    /// Attaches comments to relationships within the default arm.
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
            match &default.relationships[1] {
                crate::ast::BranchItem::Relationship(rel) => &rel.leading_comment,
                crate::ast::BranchItem::RelationshipUse(call) => &call.leading_comment,
            },
            &Some(crate::ast::Comment::Line("second".to_string()))
        );
    }

    /// Attaches a leading file comment to the sheet.
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

    /// Joins multiple leading file-comment lines on the sheet.
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

    /// Leaves sheet header trivia absent when no leading comment exists.
    #[test]
    fn no_leading_comment_before_the_sheet_leaves_it_none() {
        let source = "sheet s {\n    cell a: i32 = 1;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        assert_eq!(sheet.leading_comment, None);
    }

    /// Attaches comments to requirements within an output.
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
        let source = "sheet s {\n    out area: f64 := width * height require {\n        @max_area width * height <= max_area;\n        // second\n        @c width <= 10.0;\n    };\n}";
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

    /// Retains a sheet's trailing interior comment.
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

    /// Retains a trailing comment in an empty relationship.
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

    /// Retains a relationship's trailing interior comment.
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

    /// Retains a named branch's trailing interior comment.
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

    /// Retains the default arm's trailing interior comment.
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

    /// Retains a conditional's trailing interior comment.
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

    /// Retains an output requirement block's trailing comment.
    #[test]
    fn recovers_a_trailing_comment_before_a_requires_closing_brace() {
        let source = "sheet s {\n    out area: f64 := w require {\n        @c w <= 10.0;\n        // trailing\n    };\n}";
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

    /// Attaches comments to requirements within an ordinary cell.
    #[test]
    fn attaches_a_comment_to_a_requirement_inside_a_cell_declaration() {
        // Mirrors `attaches_a_comment_to_a_requirement_inside_an_out_declaration`, but for a
        // `cell`'s own `require` block — `CellDecl` gained `require` alongside `out`'s, and its
        // trivia recovery must not be dropped on the floor the way it was before this fix.
        let source = "sheet s {\n    cell a: i32 = 1 require {\n        @r1 a > 0;\n        // second\n        @r2 a < 10;\n    };\n}";
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

    /// Retains an ordinary cell requirement block's trailing comment.
    #[test]
    fn recovers_a_trailing_comment_before_a_cells_requires_closing_brace() {
        // Mirrors `recovers_a_trailing_comment_before_a_requires_closing_brace`, but for a
        // `cell`'s own `require` block.
        let source = "sheet s {\n    cell a: i32 = 1 require {\n        @r a > 0;\n        // trailing\n    };\n}";
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

    /// Attaches comments to requirements within a source cell.
    #[test]
    fn attaches_a_comment_to_a_requirement_inside_a_source_declaration() {
        // Mirrors `attaches_a_comment_to_a_requirement_inside_an_out_declaration`, but for a
        // `source`'s own `require` block.
        let source = "sheet s {\n    source a: i32 = 1 require {\n        @r1 a > 0;\n        // second\n        @r2 a < 10;\n    };\n}";
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

    /// Retains a source requirement block's trailing comment.
    #[test]
    fn recovers_a_trailing_comment_before_a_sources_requires_closing_brace() {
        // Mirrors `recovers_a_trailing_comment_before_a_requires_closing_brace`, but for a
        // `source`'s own `require` block.
        let source = "sheet s {\n    source a: i32 = 1 require {\n        @r a > 0;\n        // trailing\n    };\n}";
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

    /// Marks blank-line spacing before a container closes.
    #[test]
    fn sets_blank_line_before_close_when_a_blank_line_precedes_the_closing_brace() {
        let source = "sheet s {\n    cell a: i32 = 1;\n\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        assert!(sheet.blank_line_before_close);
    }

    /// Leaves trailing trivia absent when the closing gap contains no comment.
    #[test]
    fn no_trailing_comment_leaves_trailing_comment_none() {
        let source = "sheet s {\n    cell a: i32 = 1;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        assert_eq!(sheet.trailing_comment, None);
    }

    // Issue #59: a `//` comment at the end of a declaration's own line must attach to that
    // declaration's own `trailing_line_comment`, not the following item's `leading_comment`.

    /// Routes a same-line line comment to the preceding cell only.
    #[test]
    fn a_same_line_trailing_line_comment_attaches_to_the_previous_cell_not_the_next() {
        let source =
            "sheet s {\n    cell a: f64 = 2.0; // end of line comment\n    cell b: f64 = 3.0;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(a) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert_eq!(
            a.trailing_line_comment,
            Some(crate::ast::Comment::Line("end of line comment".to_string()))
        );
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment, None);
        assert!(!b.blank_line_before);
    }

    /// Routes a same-line block comment to the preceding cell only.
    #[test]
    fn a_same_line_trailing_block_comment_attaches_to_the_previous_cell_not_the_next() {
        let source = "sheet s {\n    cell a: f64 = 2.0; /* note */\n    cell b: f64 = 3.0;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(a) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert_eq!(
            a.trailing_line_comment,
            Some(crate::ast::Comment::Block("note".to_string()))
        );
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment, None);
    }

    /// Routes an own-line comment to the following declaration only.
    #[test]
    fn a_comment_on_its_own_line_still_attaches_as_a_leading_comment() {
        // Regression guard: a comment that starts on a fresh line (not same-line as the previous
        // item) must keep attaching as the next item's leading_comment, exactly as before #59.
        let source = "sheet s {\n    cell a: i32 = 1;\n    // the total\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(a) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert_eq!(a.trailing_line_comment, None);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(
            b.leading_comment,
            Some(crate::ast::Comment::Line("the total".to_string()))
        );
    }

    /// Routes a same-line trailing comment to its binding.
    #[test]
    fn a_same_line_trailing_comment_attaches_to_a_binding_inside_a_relationship() {
        let source =
            "sheet s {\n    relationship {\n        b := a; // note\n        a := b;\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert_eq!(
            rel.bindings[0].trailing_line_comment,
            Some(crate::ast::Comment::Line("note".to_string()))
        );
        assert_eq!(rel.bindings[1].leading_comment, None);
    }

    /// Routes a final same-line comment to the last sheet item.
    #[test]
    fn a_same_line_trailing_comment_before_a_sheets_closing_brace_attaches_to_the_last_item() {
        let source = "sheet s {\n    cell a: i32 = 1; // note\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(a) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert_eq!(
            a.trailing_line_comment,
            Some(crate::ast::Comment::Line("note".to_string()))
        );
        assert_eq!(sheet.trailing_comment, None);
    }

    /// Recovers a block comment from a gap without any newline.
    #[test]
    fn same_line_gap_with_no_newline_still_recovers_a_block_comment() {
        // PR #202 review: when both items and the comment share one physical line with no
        // newline anywhere in the gap, the old unconditional "drop the final fragment" logic
        // popped the whole gap (comment included) before the comment scanner ever ran.
        let source = "sheet s {\n    cell a: i32 = 1; /* note */ cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(a) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert_eq!(
            a.trailing_line_comment,
            Some(crate::ast::Comment::Block("note".to_string()))
        );
    }

    /// Routes a final same-line comment to the last relationship binding.
    #[test]
    fn a_same_line_trailing_comment_before_a_relationships_closing_brace_attaches_to_the_last_binding()
     {
        let source = "sheet s {\n    relationship {\n        b := a; // note\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert_eq!(
            rel.bindings[0].trailing_line_comment,
            Some(crate::ast::Comment::Line("note".to_string()))
        );
        assert_eq!(rel.trailing_comment, None);
    }

    /// Routes a final same-line comment to the last relationship of a named branch.
    #[test]
    fn a_same_line_trailing_comment_before_a_conditional_branchs_own_closing_brace_attaches_to_the_last_relationship()
     {
        let source = "sheet s {\n    conditional m {\n        0i32 => {\n            relationship { b := a; } // note\n        }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(
            match &cond.branches[0].relationships[0] {
                crate::ast::BranchItem::Relationship(rel) => &rel.trailing_line_comment,
                crate::ast::BranchItem::RelationshipUse(call) => &call.trailing_line_comment,
            },
            &Some(crate::ast::Comment::Line("note".to_string()))
        );
        assert_eq!(cond.branches[0].trailing_comment, None);
    }

    /// Routes a final same-line comment to the last relationship of the default arm.
    #[test]
    fn a_same_line_trailing_comment_before_a_default_arms_own_closing_brace_attaches_to_the_last_relationship()
     {
        let source = "sheet s {\n    conditional m {\n        _ => {\n            relationship { b := a; } // note\n        }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        let default = cond.default.as_ref().expect("default branch present");
        assert_eq!(
            match &default.relationships[0] {
                crate::ast::BranchItem::Relationship(rel) => &rel.trailing_line_comment,
                crate::ast::BranchItem::RelationshipUse(call) => &call.trailing_line_comment,
            },
            &Some(crate::ast::Comment::Line("note".to_string()))
        );
        assert_eq!(default.trailing_comment, None);
    }

    /// Routes a final same-line comment to the last named branch.
    #[test]
    fn a_same_line_trailing_comment_before_a_conditionals_own_closing_brace_with_only_branches_attaches_to_the_last_branch()
     {
        let source = "sheet s {\n    conditional m {\n        0i32 => { relationship { b := a; } } // note\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(
            cond.branches[0].trailing_line_comment,
            Some(crate::ast::Comment::Line("note".to_string()))
        );
        assert_eq!(cond.trailing_comment, None);
    }

    /// Routes a final same-line comment to the default arm.
    #[test]
    fn a_same_line_trailing_comment_before_a_conditionals_own_closing_brace_with_a_default_attaches_to_the_default()
     {
        let source = "sheet s {\n    conditional m {\n        _ => { relationship { b := a; } } // note\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        let default = cond.default.as_ref().expect("default branch present");
        assert_eq!(
            default.trailing_line_comment,
            Some(crate::ast::Comment::Line("note".to_string()))
        );
        assert_eq!(cond.trailing_comment, None);
    }

    /// Routes a final same-line comment to the last requirement.
    #[test]
    fn a_same_line_trailing_comment_before_a_requires_closing_brace_attaches_to_the_last_requirement()
     {
        let source =
            "sheet s {\n    out area: f64 := w require {\n        @c w <= 10.0; // note\n    };\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Out(out) = &sheet.items[0] else {
            panic!("expected Out");
        };
        let require = out.require.as_ref().expect("require block present");
        assert_eq!(
            require.requirements[0].trailing_line_comment,
            Some(crate::ast::Comment::Line("note".to_string()))
        );
        assert_eq!(require.trailing_comment, None);
    }
}
