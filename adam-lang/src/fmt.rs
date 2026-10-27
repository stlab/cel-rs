//! Pretty-prints an [`crate::ast::Sheet`] back to adam-lang source text: 4-space indentation,
//! opening braces on the same line, `leading_comment`/`blank_line_before` reproduced exactly as
//! [`crate::trivia::attach_trivia`] recovered them (including a file-header-style comment
//! preceding the `sheet` keyword itself, `Sheet.leading_comment`), and binding bodies/cell
//! initializers/requirement bodies delegated to [`cel_parser::format_expr_with_layout`].
//! Expression layout targets 100 columns including declaration prefixes and punctuation. Nested
//! lists, records and closure bodies use four spaces; binary continuations lead with operators.
//! Type annotations are re-emitted via `Span::source_text()` directly
//! via `TypeExpr::span()`; parameter annotations borrow their complete source range so applied
//! named types retain arguments and comments. Branch-match literals use `ConditionalBranch::literal_span` — see
//! the design doc for why no `Literal` value is needed. Conditional branches omit the grammar's
//! optional trailing `,`, matching `begin/assets/demo.adm2`'s existing style.
//!
//! Never called on a sheet with any recorded syntax errors — see `adam-lsp`'s
//! `textDocument/formatting` handler, which refuses to format in that case.

use crate::ast;

/// Returns the current output column in Unicode scalar values.
///
/// - Complexity: O(k) in the final line's length.
fn output_column(out: &str) -> usize {
    out.rsplit('\n').next().unwrap_or_default().chars().count()
}

/// Appends one expression using the complete emitted prefix and following punctuation budget.
///
/// - Complexity: O(n) document space plus layout, final-line measurement and source-gap scans.
fn write_expression(
    out: &mut String,
    source: &str,
    expr: &cel_parser::Expr,
    depth: usize,
    suffix_columns: usize,
) {
    let text = cel_parser::format_expr_with_layout(
        expr,
        source,
        depth,
        output_column(out),
        100,
        suffix_columns,
    );
    append_expression_layout(out, &text);
}

/// Appends expression layout, removing prefix padding when layout starts on a new line.
///
/// - Complexity: O(n + k) in the expression text and the prefix's trailing padding.
fn append_expression_layout(out: &mut String, text: &str) {
    if text.starts_with('\n') {
        out.truncate(out.trim_end_matches(' ').len());
    }
    out.push_str(text);
}

/// Reserves punctuation before the next clause, or the declaration's semicolon.
fn cell_clause_suffix(
    filter: Option<&ast::CellFilter>,
    require: Option<&ast::RequireBlock>,
) -> usize {
    if filter.is_some() {
        8
    } else if require.is_some() {
        10
    } else {
        1
    }
}

/// 4 spaces per nesting level.
fn indent(depth: usize) -> String {
    "    ".repeat(depth)
}

/// Writes `comment`'s text starting at the current write position — its first line gets no
/// leading indent or prefix of its own, so the caller positions the cursor first (either after
/// its own `indent(depth)`, for a comment on its own line, or inline after a declaration's own
/// terminator) — continuing any further lines at `depth`'s indentation, and always ending with a
/// newline: a `Comment::Line` as one `// ` line per stored line; a `Comment::Block` as
/// `/* text */` on one line when its text has no internal `\n`, or a multi-line `/*`/`*/`-
/// delimited block (one line per stored line, indented one level past `depth`) when it does.
/// Shared by [`write_comment`] and [`write_line_end`].
fn write_comment_inline(out: &mut String, comment: &ast::Comment, depth: usize) {
    match comment {
        ast::Comment::Line(text) => {
            let mut lines = text.split('\n');
            if let Some(first) = lines.next() {
                out.push_str("// ");
                out.push_str(first);
                out.push('\n');
            }
            for line in lines {
                out.push_str(&indent(depth));
                out.push_str("// ");
                out.push_str(line);
                out.push('\n');
            }
        }
        ast::Comment::Block(text) => {
            if text.contains('\n') {
                out.push_str("/*\n");
                for line in text.split('\n') {
                    out.push_str(&indent(depth + 1));
                    out.push_str(line);
                    out.push('\n');
                }
                out.push_str(&indent(depth));
                out.push_str("*/\n");
            } else {
                out.push_str("/* ");
                out.push_str(text);
                out.push_str(" */\n");
            }
        }
    }
}

/// Writes one recovered `Comment` on its own line at `depth`'s indentation, via
/// [`write_comment_inline`].
fn write_comment(out: &mut String, comment: &ast::Comment, depth: usize) {
    out.push_str(&indent(depth));
    write_comment_inline(out, comment, depth);
}

/// Writes `doc_comment`'s lines (if present) as one `marker` line per stored line, at `depth`'s
/// indentation. `marker` is `"///"` for a declaration's own doc comment or `"//!"` for the
/// sheet's own (each stored line already includes the space rustdoc puts after the marker, e.g.
/// `" the total"` for `/// the total`, so no extra space is inserted here).
fn write_doc_comment(out: &mut String, marker: &str, doc_comment: Option<&str>, depth: usize) {
    if let Some(doc) = doc_comment {
        for line in doc.split('\n') {
            out.push_str(&indent(depth));
            out.push_str(marker);
            out.push_str(line);
            out.push('\n');
        }
    }
}

/// Emits `blank_line_before`/`leading_comment` ahead of an item, if either is present.
fn write_trivia(
    out: &mut String,
    blank_line_before: bool,
    leading_comment: Option<&ast::Comment>,
    depth: usize,
) {
    if blank_line_before {
        out.push('\n');
    }
    if let Some(comment) = leading_comment {
        write_comment(out, comment, depth);
    }
}

/// Emits a container's trailing comment (honoring `blank_line_before_close`) immediately before
/// its closing `}` is written, reusing [`write_comment`] so block-vs-line style is preserved
/// exactly as for leading comments. See <https://github.com/stlab/cel-rs/issues/52>.
fn write_trailing_trivia(
    out: &mut String,
    blank_line_before_close: bool,
    trailing_comment: Option<&ast::Comment>,
    depth: usize,
) {
    if blank_line_before_close {
        out.push('\n');
    }
    if let Some(comment) = trailing_comment {
        write_comment(out, comment, depth);
    }
}

/// Terminates a declaration's own output line with `terminator` (`;` or `}`), appending its
/// same-line `trailing_line_comment` inline first when present — a `//` comment always ends the
/// line it's on, and a `/* */` comment stays inline with what precedes it, so both are written
/// before the newline rather than after it. See <https://github.com/stlab/cel-rs/issues/59>.
fn write_line_end(
    out: &mut String,
    terminator: &str,
    trailing_line_comment: Option<&ast::Comment>,
    depth: usize,
) {
    out.push_str(terminator);
    match trailing_line_comment {
        Some(comment) => {
            out.push(' ');
            write_comment_inline(out, comment, depth);
        }
        None => out.push('\n'),
    }
}

/// Re-emits a literal's exact original text via its span, falling back to an empty string when
/// none is recoverable — mirrors `cel_parser::fmt`'s identical fallback (see the module doc for
/// why no `Literal` value is needed here).
fn source_text_or_empty(span: ast::ExprSpan) -> String {
    span.start.source_text().unwrap_or_default()
}

/// Borrows a source region, returning `None` for empty source or an unresolvable range.
///
/// - Precondition: non-synthetic positions refer to `source`.
/// - Complexity: O(n) time and space in source length for the line-offset table.
fn source_region(
    source: &str,
    start: proc_macro2::LineColumn,
    end: proc_macro2::LineColumn,
) -> Option<&str> {
    use cel_parser::trivia::{line_column_to_byte, line_start_byte_offsets};
    if source.is_empty() {
        return None;
    }
    let lines = line_start_byte_offsets(source);
    let start = line_column_to_byte(source, &lines, start);
    let end = line_column_to_byte(source, &lines, end);
    source.get(start..end)
}

/// Emits the complete accepted parameter type and its comments, not only its first token.
///
/// Synthetic type spans use their stored source text.
///
/// - Precondition: spans refer to `source`, or `source` is empty.
/// - Complexity: O(n) time and space in source length plus type text length.
fn write_parameter_type(out: &mut String, source: &str, ty: &ast::TypeExpr) {
    let span = ty.span();
    if let Some(text) = source_region(source, span.start.start(), span.end.end()) {
        out.push_str(text);
    } else {
        out.push_str(&source_text_or_empty(span));
    }
}

/// Emits the complete documented header prefix, retaining interspersed comments in source order.
///
/// Synthetic declarations use their stored documentation instead.
///
/// - Precondition: spans refer to `source`, or `source` is empty.
/// - Complexity: O(n) time and space in source length plus emitted prefix length.
fn write_header_documentation(
    out: &mut String,
    source: &str,
    start: proc_macro2::Span,
    keyword: ast::ExprSpan,
    prefix: &str,
    documentation: Option<&str>,
    depth: usize,
) {
    if let Some(documentation) = documentation {
        if let Some(region) = source_region(source, start.start(), keyword.start.start())
            .filter(|region| !region.is_empty())
        {
            out.push_str(&indent(depth));
            out.push_str(region.trim_end_matches([' ', '\t']));
        } else {
            write_doc_comment(out, prefix, Some(documentation), depth);
        }
    }
}

/// A single-point `ExprSpan` (start and end coincide), for a gap endpoint with no spanned AST
/// node of its own — e.g. the terminating `;` following a `CellDecl`'s initializer, which has
/// only `CellDecl.span.end` (the `;` token's own span) available, not a full node.
fn point_span(span: proc_macro2::Span) -> ast::ExprSpan {
    ast::ExprSpan {
        start: span,
        end: span,
    }
}

/// Renders the source gap between two adam-lang declaration positions — any comments it holds,
/// interleaved *in source order* with `expected`, the fixed literal tokens (e.g. `"="`, `":="`)
/// the grammar guarantees appear there. Passing the operator through [`cel_parser::trivia::scan_gap`]
/// (rather than having the caller append it afterward) is what preserves ordering: a comment that
/// follows the `=` in `cell a: i32 = /* p */ 1;` stays after it, and one on each side of it
/// (`cell a: i32 /* before */ = /* after */ 1;`) keeps its position relative to the operator.
///
/// Spacing mirrors `cel_parser::fmt::emit_gap`'s `Around` convention: an `expected` token is
/// rendered space-separated, a `Comment::Block` stays inline, and a `Comment::Line` (which runs to
/// end of line) wraps — ending the gap with a newline plus `indent(depth + 1)` so whatever the
/// caller appends next starts on a fresh line rather than being swallowed into the comment.
///
/// - Postcondition: an *operator* gap (`expected` non-empty) always emits every `expected` token,
///   even when `source` is empty or the positions don't resolve (a hand-built declaration with
///   synthetic spans still gets its `=`/`:=`), and ends with a separating space so the caller's
///   next token (the initializer) is not glued on. A *comment-only* gap (`expected` empty) returns
///   `""` when it holds no comments, and never adds a trailing space, so the caller appends its own
///   terminator (`;`) or keyword directly.
///   Comment-free module paths and parameter/argument lists use compact punctuation and omit
///   optional trailing commas.
///
/// `depth` is the caller's own indent depth (the declaration's, not the gap's).
fn emit_decl_gap(
    source: &str,
    prev_end: ast::ExprSpan,
    next_start: ast::ExprSpan,
    expected: &[&'static str],
    depth: usize,
) -> String {
    use cel_parser::trivia::{GapPiece, line_column_to_byte, line_start_byte_offsets, scan_gap};
    // `scan_gap` re-synthesizes every `expected` token even on an empty or non-resolving gap, so a
    // hand-built declaration (empty `source`) still renders its operator.
    let pieces = if source.is_empty() {
        scan_gap("", expected)
    } else {
        let line_starts = line_start_byte_offsets(source);
        let start = line_column_to_byte(source, &line_starts, prev_end.end.end());
        let end = line_column_to_byte(source, &line_starts, next_start.start.start());
        match source.get(start..end) {
            Some(gap) => scan_gap(gap, expected),
            None => scan_gap("", expected),
        }
    };
    if pieces
        .iter()
        .all(|piece| matches!(piece, GapPiece::Punct(_)))
    {
        let compact = match expected {
            ["("] => Some("("),
            [","] => Some(", "),
            [":"] => Some(": "),
            ["::"] => Some("::"),
            [",", ")"] => Some(")"),
            ["(", ")"] => Some("()"),
            _ => None,
        };
        if let Some(compact) = compact {
            return compact.to_owned();
        }
    }
    let mut out = String::new();
    for piece in pieces {
        match piece {
            GapPiece::Punct(tok) => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(tok);
                out.push(' ');
            }
            GapPiece::Comment(cel_parser::Comment::Block(text)) => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str("/* ");
                out.push_str(&text);
                out.push_str(" */");
            }
            GapPiece::Comment(cel_parser::Comment::Line(text)) => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str("// ");
                out.push_str(&text);
                out.push('\n');
                out.push_str(&indent(depth + 1));
            }
        }
    }
    // An operator gap must end separated from the initializer the caller appends next; a
    // comment-only gap must not (its terminator/keyword is appended with its own leading space).
    if !expected.is_empty() && !out.is_empty() && !out.ends_with(' ') {
        out.push(' ');
    }
    out
}

/// Writes one `a := ...;` / `(a, b) := ...;` binding, delegating its body to
/// [`cel_parser::format_expr`].
///
/// - Complexity: O(n) output/document space plus layout and source-gap scans.
fn write_binding(out: &mut String, source: &str, binding: &ast::BindingDecl, depth: usize) {
    write_trivia(
        out,
        binding.blank_line_before,
        binding.leading_comment.as_ref(),
        depth,
    );
    out.push_str(&indent(depth));
    if binding.destructure {
        out.push('(');
        for (i, (name, _)) in binding.outputs.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(name);
        }
        if binding.outputs.len() == 1 {
            out.push(',');
        }
        out.push(')');
    } else {
        debug_assert_eq!(
            binding.outputs.len(),
            1,
            "a non-destructuring binding names exactly one output"
        );
        out.push_str(&binding.outputs[0].0);
    }
    // `:=` gap, rendered in-position so a comment on either side of it keeps its order.
    let op_start = binding
        .outputs
        .last()
        .map(|(_, span)| *span)
        .unwrap_or(binding.span);
    out.push_str(&emit_decl_gap(
        source,
        op_start,
        binding.body.span(),
        &[":="],
        depth,
    ));
    write_expression(out, source, &binding.body, depth, 1);
    // Comment between the binding's body and its terminating `;`.
    out.push_str(&emit_decl_gap(
        source,
        binding.body.span(),
        point_span(binding.span.end),
        &[],
        depth,
    ));
    write_line_end(out, ";", binding.trailing_line_comment.as_ref(), depth);
}

/// Writes one `relationship { ... }` declaration and its bindings, in declaration order.
fn write_relationship(out: &mut String, source: &str, rel: &ast::RelationshipDecl, depth: usize) {
    write_trivia(
        out,
        rel.blank_line_before,
        rel.leading_comment.as_ref(),
        depth,
    );
    write_doc_comment(out, "///", rel.doc_comment.as_deref(), depth);
    out.push_str(&indent(depth));
    out.push_str("relationship");
    // Comment between the `relationship` keyword and its `{` (the keyword's own span starts at
    // `rel.span.start`; scan from just past it to the block's `{`).
    out.push_str(&emit_decl_gap(
        source,
        point_span(rel.span.start),
        rel.open_brace_span,
        &[],
        depth,
    ));
    out.push_str(" {\n");
    for binding in &rel.bindings {
        write_binding(out, source, binding, depth + 1);
    }
    write_trailing_trivia(
        out,
        rel.blank_line_before_close,
        rel.trailing_comment.as_ref(),
        depth + 1,
    );
    out.push_str(&indent(depth));
    write_line_end(out, "}", rel.trailing_line_comment.as_ref(), depth);
}

/// Writes a `{ ... }` block of relationships, shared by both a named conditional branch and the
/// default (`_ =>`) arm.
fn write_branch_relationships(
    out: &mut String,
    source: &str,
    relationships: &[ast::BranchItem],
    trailing_comment: Option<&ast::Comment>,
    blank_line_before_close: bool,
    line_trailing_comment: Option<&ast::Comment>,
    depth: usize,
) {
    out.push_str("{\n");
    for rel in relationships {
        match rel {
            ast::BranchItem::Relationship(rel) => write_relationship(out, source, rel, depth + 1),
            ast::BranchItem::RelationshipUse(call) => {
                write_relationship_use(out, source, call, depth + 1)
            }
        }
    }
    write_trailing_trivia(out, blank_line_before_close, trailing_comment, depth + 1);
    out.push_str(&indent(depth));
    write_line_end(out, "}", line_trailing_comment, depth);
}

/// Writes one `literal_pattern => { ... }` conditional branch, re-emitting the match literal via
/// its span rather than the (unused) `Literal` value, with a leading `-` when
/// [`ast::ConditionalBranch::negated`].
fn write_branch(out: &mut String, source: &str, branch: &ast::ConditionalBranch, depth: usize) {
    write_trivia(
        out,
        branch.blank_line_before,
        branch.leading_comment.as_ref(),
        depth,
    );
    out.push_str(&indent(depth));
    if branch.negated {
        out.push('-');
    }
    out.push_str(&source_text_or_empty(branch.literal_span));
    // One gap from the literal through the `=>` to the branch body's `{`, recovering comments on
    // either side of `=>` in source order.
    out.push_str(&emit_decl_gap(
        source,
        branch.literal_span,
        branch.open_brace_span,
        &["=>"],
        depth,
    ));
    write_branch_relationships(
        out,
        source,
        &branch.relationships,
        branch.trailing_comment.as_ref(),
        branch.blank_line_before_close,
        branch.trailing_line_comment.as_ref(),
        depth,
    );
}

/// Writes one `conditional <expr> { ... }` declaration: its branches in declaration
/// order (dispatching on the match-subject expression), followed by its optional `_ => { ... }` default arm.
///
/// Preserves header-boundary comments across omitted groups while reserving the body brace.
///
/// - Complexity: O(n) output/document space plus layout and source-gap scans.
fn write_conditional(out: &mut String, source: &str, cond: &ast::ConditionalDecl, depth: usize) {
    write_trivia(
        out,
        cond.blank_line_before,
        cond.leading_comment.as_ref(),
        depth,
    );
    write_doc_comment(out, "///", cond.doc_comment.as_deref(), depth);
    out.push_str(&indent(depth));
    out.push_str("conditional ");
    let header = cel_parser::format_expr_header_with_layout(
        &cond.match_expr,
        source,
        depth,
        (cond.span.start.end(), cond.open_brace_span.start.start()),
        output_column(out),
        100,
        2,
    );
    append_expression_layout(out, &header);
    out.push_str(" {\n");
    for branch in &cond.branches {
        write_branch(out, source, branch, depth + 1);
    }
    if let Some(default) = &cond.default {
        out.push_str(&indent(depth + 1));
        out.push('_');
        // One gap from `_` through `=>` to the default body's `{`, recovering comments around `=>`.
        out.push_str(&emit_decl_gap(
            source,
            point_span(default.span.start),
            default.open_brace_span,
            &["=>"],
            depth + 1,
        ));
        write_branch_relationships(
            out,
            source,
            &default.relationships,
            default.trailing_comment.as_ref(),
            default.blank_line_before_close,
            default.trailing_line_comment.as_ref(),
            depth + 1,
        );
    }
    write_trailing_trivia(
        out,
        cond.blank_line_before_close,
        cond.trailing_comment.as_ref(),
        depth + 1,
    );
    out.push_str(&indent(depth));
    write_line_end(out, "}", cond.trailing_line_comment.as_ref(), depth);
}

/// Writes one `cell name[: type][ = initializer][ filter body][ require { ... }];` declaration,
/// delegating its type annotation to [`source_text_or_empty`] via `TypeExpr::span()` and its
/// initializer/filter body to [`cel_parser::format_expr`].
///
/// - Complexity: O(n) output/document space plus layout and source-gap scans.
fn write_cell(out: &mut String, source: &str, cell: &ast::CellDecl, depth: usize) {
    write_trivia(
        out,
        cell.blank_line_before,
        cell.leading_comment.as_ref(),
        depth,
    );
    write_doc_comment(out, "///", cell.doc_comment.as_deref(), depth);
    out.push_str(&indent(depth));
    out.push_str("cell ");
    out.push_str(&cell.name);
    if let Some(type_expr) = &cell.type_name {
        out.push_str(": ");
        out.push_str(&source_text_or_empty(type_expr.span()));
    }
    // `=` gap, rendered in-position so comments keep their order relative to the operator.
    if let Some(expr) = &cell.initializer {
        let op_start = cell
            .type_name
            .as_ref()
            .map(ast::TypeExpr::span)
            .unwrap_or(cell.name_span);
        out.push_str(&emit_decl_gap(source, op_start, expr.span(), &["="], depth));
        write_expression(
            out,
            source,
            expr,
            depth,
            cell_clause_suffix(cell.filter.as_ref(), cell.require.as_ref()),
        );
    }
    write_cell_clauses(
        out,
        source,
        depth,
        cell.initializer.as_ref().map(cel_parser::Expr::span),
        cell.type_name.as_ref().map(ast::TypeExpr::span),
        cell.name_span,
        cell.filter.as_ref(),
        cell.require.as_ref(),
        cell.span.end,
    );
    write_line_end(out, ";", cell.trailing_line_comment.as_ref(), depth);
}

/// Writes a declaration's optional `filter`/`require` clauses and recovers any comment in each gap
/// between adjacent segments (initializer → `filter` → `require` → `;`), shared by [`write_cell`],
/// [`write_source`], and [`write_out`]. `prev_*` give the end span of whichever of
/// initializer/type/name precedes the clauses; `terminator` is the declaration's own `;` span.
///
/// - Complexity: O(n) output/document space plus layout and source-gap scans.
#[allow(clippy::too_many_arguments)]
fn write_cell_clauses(
    out: &mut String,
    source: &str,
    depth: usize,
    initializer_span: Option<ast::ExprSpan>,
    type_span: Option<ast::ExprSpan>,
    name_span: ast::ExprSpan,
    filter: Option<&ast::CellFilter>,
    require: Option<&ast::RequireBlock>,
    terminator: proc_macro2::Span,
) {
    let mut prev = initializer_span.or(type_span).unwrap_or(name_span);
    if let Some(filter) = filter {
        // One gap from the previous segment through the `filter` keyword to the filter body,
        // recovering comments on either side of `filter` in source order.
        out.push_str(&emit_decl_gap(
            source,
            prev,
            filter.body.span(),
            &["filter"],
            depth,
        ));
        write_expression(
            out,
            source,
            &filter.body,
            depth,
            if require.is_some() { 10 } else { 1 },
        );
        prev = filter.body.span();
    }
    if let Some(require) = require {
        // One gap from the previous segment through the `require` keyword to the block's `{`,
        // recovering comments on either side of `require` (the keyword itself has no stored span,
        // but it lies inside this gap).
        out.push_str(&emit_decl_gap(
            source,
            prev,
            require.open_brace_span,
            &["require"],
            depth,
        ));
        write_require_block(out, source, require, depth);
        prev = require.span;
    }
    // Comment between the last segment and the terminating `;`.
    out.push_str(&emit_decl_gap(
        source,
        prev,
        point_span(terminator),
        &[],
        depth,
    ));
}

/// Writes one `[ "@" identifier " " ] ...;` requirement.
///
/// - Complexity: O(n) output/document space plus layout and source-gap scans.
fn write_requirement(out: &mut String, source: &str, req: &ast::RequirementDecl, depth: usize) {
    write_trivia(
        out,
        req.blank_line_before,
        req.leading_comment.as_ref(),
        depth,
    );
    out.push_str(&indent(depth));
    if let Some(name) = &req.name {
        out.push('@');
        out.push_str(name);
        // Recover any comment between the `@label` and the body (e.g. `@c /* keep */ w <= 10`).
        if let Some(name_span) = req.name_span {
            out.push_str(&emit_decl_gap(
                source,
                name_span,
                req.body.span(),
                &[],
                depth,
            ));
        }
        out.push(' ');
    }
    write_expression(out, source, &req.body, depth, 1);
    write_line_end(out, ";", req.trailing_line_comment.as_ref(), depth);
}

/// Writes one `require` block's `{ ... }` body (no `require` keyword, no trailing `;`) — the
/// caller emits the `require` keyword (and any comments around it) via [`emit_decl_gap`] so a
/// comment between `require` and its `{` is preserved. Shared by [`write_cell`], [`write_source`],
/// and [`write_out`].
fn write_require_block(out: &mut String, source: &str, require: &ast::RequireBlock, depth: usize) {
    out.push_str("{\n");
    for req in &require.requirements {
        write_requirement(out, source, req, depth + 1);
    }
    write_trailing_trivia(
        out,
        require.blank_line_before_close,
        require.trailing_comment.as_ref(),
        depth + 1,
    );
    out.push_str(&indent(depth));
    out.push('}');
}

/// Writes one `out name[: type] := ...[ filter body][ require { ... } ];` declaration.
///
/// - Complexity: O(n) output/document space plus layout and source-gap scans.
fn write_out(out: &mut String, source: &str, decl: &ast::OutDecl, depth: usize) {
    write_trivia(
        out,
        decl.blank_line_before,
        decl.leading_comment.as_ref(),
        depth,
    );
    write_doc_comment(out, "///", decl.doc_comment.as_deref(), depth);
    out.push_str(&indent(depth));
    out.push_str("out ");
    out.push_str(&decl.name);
    if let Some(type_expr) = &decl.type_name {
        out.push_str(": ");
        out.push_str(&source_text_or_empty(type_expr.span()));
    }
    // `:=` gap, rendered in-position so comments keep their order relative to the operator.
    let op_start = decl
        .type_name
        .as_ref()
        .map(ast::TypeExpr::span)
        .unwrap_or(decl.name_span);
    out.push_str(&emit_decl_gap(
        source,
        op_start,
        decl.initializer.span(),
        &[":="],
        depth,
    ));
    write_expression(
        out,
        source,
        &decl.initializer,
        depth,
        cell_clause_suffix(decl.filter.as_ref(), decl.require.as_ref()),
    );
    write_cell_clauses(
        out,
        source,
        depth,
        Some(decl.initializer.span()),
        decl.type_name.as_ref().map(ast::TypeExpr::span),
        decl.name_span,
        decl.filter.as_ref(),
        decl.require.as_ref(),
        decl.span.end,
    );
    write_line_end(out, ";", decl.trailing_line_comment.as_ref(), depth);
}

/// Writes one `source name[: type][ = initializer][ filter body][ require { ... }];` declaration.
/// Mirrors [`write_cell`] exactly.
///
/// - Complexity: O(n) output/document space plus layout and source-gap scans.
fn write_source(out: &mut String, source: &str, decl: &ast::SourceDecl, depth: usize) {
    write_trivia(
        out,
        decl.blank_line_before,
        decl.leading_comment.as_ref(),
        depth,
    );
    write_doc_comment(out, "///", decl.doc_comment.as_deref(), depth);
    out.push_str(&indent(depth));
    out.push_str("source ");
    out.push_str(&decl.name);
    if let Some(type_expr) = &decl.type_name {
        out.push_str(": ");
        out.push_str(&source_text_or_empty(type_expr.span()));
    }
    // `=` gap, rendered in-position so comments keep their order relative to the operator.
    if let Some(expr) = &decl.initializer {
        let op_start = decl
            .type_name
            .as_ref()
            .map(ast::TypeExpr::span)
            .unwrap_or(decl.name_span);
        out.push_str(&emit_decl_gap(source, op_start, expr.span(), &["="], depth));
        write_expression(
            out,
            source,
            expr,
            depth,
            cell_clause_suffix(decl.filter.as_ref(), decl.require.as_ref()),
        );
    }
    write_cell_clauses(
        out,
        source,
        depth,
        decl.initializer.as_ref().map(cel_parser::Expr::span),
        decl.type_name.as_ref().map(ast::TypeExpr::span),
        decl.name_span,
        decl.filter.as_ref(),
        decl.require.as_ref(),
        decl.span.end,
    );
    write_line_end(out, ";", decl.trailing_line_comment.as_ref(), depth);
}

/// Dispatches to the writer for one top-level sheet item.
///
/// - Precondition: `item` is not `SheetItem::Error` — [`format_sheet`]'s own precondition
///   (`sheet.errors.is_empty()`) guarantees no `Error` item ever reaches this function.
fn write_sheet_item(out: &mut String, source: &str, item: &ast::SheetItem, depth: usize) {
    match item {
        ast::SheetItem::Cell(cell) => write_cell(out, source, cell, depth),
        ast::SheetItem::Relationship(rel) => write_relationship(out, source, rel, depth),
        ast::SheetItem::RelationshipUse(call) => write_relationship_use(out, source, call, depth),
        ast::SheetItem::Conditional(cond) => write_conditional(out, source, cond, depth),
        ast::SheetItem::Out(out_decl) => write_out(out, source, out_decl, depth),
        ast::SheetItem::Source(decl) => write_source(out, source, decl, depth),
        ast::SheetItem::Error { .. } => {
            unreachable!("format_sheet is only called on a sheet with no recorded syntax errors")
        }
    }
}

/// Pretty-prints `sheet` back to adam-lang source text — see the module doc for the printing
/// rules.
///
/// `source` is the text `sheet` was parsed from (or `""` for a programmatically built `Sheet`);
/// it's forwarded to [`cel_parser::format_expr`] for every embedded expression.
///
/// - Precondition: `sheet` has no recorded syntax errors (`sheet.errors.is_empty()`) — a sheet
///   with a `SheetItem::Error` placeholder cannot be printed back to valid source.
///
/// # Examples
///
/// ```
/// use adam_lang::{AdamAstParser, attach_trivia, format_sheet};
///
/// let source = "sheet s { cell x: i32 = 1; }";
/// let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
/// attach_trivia(source, &mut sheet);
/// assert_eq!(format_sheet(&sheet, source), "sheet s {\n    cell x: i32 = 1;\n}\n");
/// ```
pub fn format_sheet(sheet: &ast::Sheet, source: &str) -> String {
    debug_assert!(
        sheet.errors.is_empty(),
        "format_sheet's precondition: no recorded syntax errors"
    );
    let mut out = String::new();
    write_trivia(&mut out, false, sheet.leading_comment.as_ref(), 0);
    write_header_documentation(
        &mut out,
        source,
        sheet.span.start,
        sheet.keyword_span,
        "//!",
        sheet.doc_comment.as_deref(),
        0,
    );
    out.push_str("sheet");
    out.push_str(&emit_decl_gap(
        source,
        sheet.keyword_span,
        sheet.name_span,
        &[],
        0,
    ));
    out.push(' ');
    out.push_str(&sheet.name);
    let mut previous = sheet.name_span;
    if let Some(parameters) = &sheet.parameters {
        for (index, parameter) in parameters.iter().enumerate() {
            out.push_str(&emit_decl_gap(
                source,
                previous,
                parameter.name_span,
                if index == 0 { &["("] } else { &[","] },
                0,
            ));
            out.push_str(&parameter.name);
            out.push_str(&emit_decl_gap(
                source,
                parameter.name_span,
                parameter.type_name.span(),
                &[":"],
                0,
            ));
            write_parameter_type(&mut out, source, &parameter.type_name);
            previous = parameter.type_name.span();
        }
        out.push_str(&emit_decl_gap(
            source,
            previous,
            sheet.open_brace_span,
            if parameters.is_empty() {
                &["(", ")"]
            } else {
                &[",", ")"]
            },
            0,
        ));
    } else {
        out.push_str(&emit_decl_gap(
            source,
            previous,
            sheet.open_brace_span,
            &[],
            0,
        ));
    }
    if !out.ends_with(' ') {
        out.push(' ');
    }
    out.push_str("{\n");
    for item in &sheet.items {
        write_sheet_item(&mut out, source, item, 1);
    }
    write_trailing_trivia(
        &mut out,
        sheet.blank_line_before_close,
        sheet.trailing_comment.as_ref(),
        1,
    );
    out.push_str("}\n");
    out
}

/// Error returned by [`format_source`] when `source` can't be safely formatted.
#[derive(Debug)]
pub enum FormatSourceError {
    /// `source` failed to parse at all.
    Parse(cel_parser::ParseError),
    /// `source` parsed but recovered one or more syntax errors inside individual sheet items.
    Recovered(Vec<cel_parser::ParseError>),
}

impl std::fmt::Display for FormatSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatSourceError::Parse(error) => write!(f, "{error}"),
            FormatSourceError::Recovered(errors) => {
                for (i, error) in errors.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{error}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for FormatSourceError {}

/// Parses and formats `source` as a complete adam-lang module.
///
/// # Errors
///
/// Returns [`FormatSourceError::Recovered`] for recovered syntax errors. Tokenization failures,
/// empty input, and input with no recognizable module declaration return
/// [`FormatSourceError::Parse`]. Refuses to format code it cannot fully understand.
///
/// # Examples
///
/// ```
/// use adam_lang::format_source;
///
/// assert_eq!(
///     format_source("sheet   s{cell x:i32=1;}").unwrap(),
///     "sheet s {\n    cell x: i32 = 1;\n}\n"
/// );
/// assert!(format_source("not a sheet at all").is_err());
/// ```
pub fn format_source(source: &str) -> Result<String, FormatSourceError> {
    let mut module = crate::AdamAstParser::new()
        .parse_module_str(source)
        .map_err(FormatSourceError::Parse)?;
    if module.items.is_empty() {
        return Err(FormatSourceError::Parse(cel_parser::ParseError::new(
            "expected `use` or `sheet`",
            proc_macro2::Span::call_site(),
        )));
    }
    if module
        .items
        .iter()
        .all(|item| matches!(item, ast::ModuleItem::Error(_)))
    {
        return Err(FormatSourceError::Parse(module.errors.remove(0)));
    }
    crate::attach_module_trivia(source, &mut module);
    format_module(&module, source)
}

/// Writes an unresolved path, preserving comments between its segments.
///
/// - Complexity: O(n) output space plus source-gap scans.
fn write_path(out: &mut String, source: &str, path: &[(String, ast::ExprSpan)], depth: usize) {
    for (index, (name, span)) in path.iter().enumerate() {
        if index > 0 {
            out.push_str(&emit_decl_gap(
                source,
                path[index - 1].1,
                *span,
                &["::"],
                depth,
            ));
        }
        out.push_str(name);
    }
}

/// Writes a named component use through the existing declaration-gap and trivia writers.
///
/// - Complexity: O(n) output space plus source-gap scans.
fn write_relationship_use(
    out: &mut String,
    source: &str,
    call: &ast::RelationshipUse,
    depth: usize,
) {
    for comment in &call.opening_comments {
        write_trivia(out, false, Some(comment), depth);
    }
    write_trivia(
        out,
        call.blank_line_before,
        call.leading_comment.as_ref(),
        depth,
    );
    write_header_documentation(
        out,
        source,
        call.span.start,
        call.keyword_span,
        "///",
        call.doc_comment.as_deref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str("relationship");
    out.push_str(&emit_decl_gap(
        source,
        call.keyword_span,
        call.path[0].1,
        &[],
        depth,
    ));
    out.push(' ');
    write_path(out, source, &call.path, depth);
    let mut previous = call.path.last().expect("parsed paths are nonempty").1;
    for (index, argument) in call.arguments.iter().enumerate() {
        out.push_str(&emit_decl_gap(
            source,
            previous,
            argument.name_span,
            if index == 0 { &["("] } else { &[","] },
            depth,
        ));
        out.push_str(&argument.name);
        out.push_str(&emit_decl_gap(
            source,
            argument.name_span,
            argument.cell_span,
            &[":"],
            depth,
        ));
        out.push_str(&argument.cell);
        previous = argument.cell_span;
    }
    out.push_str(&emit_decl_gap(
        source,
        previous,
        point_span(call.span.end),
        if call.arguments.is_empty() {
            &["(", ")"]
        } else {
            &[",", ")"]
        },
        depth,
    ));
    write_line_end(out, ";", call.trailing_line_comment.as_ref(), depth);
}

/// Formats every module item in order, preserving attached file-boundary comments.
///
/// - Precondition: spans and attached trivia refer to `source`, or use synthetic spans with `""`.
/// - Complexity: O(n) output space plus nested formatters' source-gap scans.
///
/// # Errors
///
/// Returns `Recovered` for any recorded syntax error or recovery placeholder.
///
/// # Examples
///
/// ```
/// let source = "sheet helper() {} sheet root {}";
/// let mut module = adam_lang::AdamAstParser::new().parse_module_str(source).unwrap();
/// adam_lang::attach_module_trivia(source, &mut module);
/// let formatted = adam_lang::format_module(&module, source).unwrap();
/// assert_eq!(adam_lang::format_source(&formatted).unwrap(), formatted);
/// ```
pub fn format_module(module: &ast::Module, source: &str) -> Result<String, FormatSourceError> {
    if !module.errors.is_empty() {
        return Err(FormatSourceError::Recovered(module.errors.clone()));
    }
    let mut out = String::new();
    for (index, item) in module.items.iter().enumerate() {
        if let Some(comments) = module.comments.get(index) {
            for comment in comments {
                write_trivia(&mut out, false, Some(comment), 0);
            }
        }
        match item {
            ast::ModuleItem::Sheet(sheet)
                if sheet
                    .items
                    .iter()
                    .any(|item| matches!(item, ast::SheetItem::Error { .. })) =>
            {
                return Err(FormatSourceError::Recovered(vec![
                    cel_parser::ParseError::new("recovered sheet item", sheet.span.start),
                ]));
            }
            ast::ModuleItem::Sheet(sheet) if sheet.errors.is_empty() => {
                out.push_str(&format_sheet(sheet, source))
            }
            ast::ModuleItem::Use(import) => {
                write_header_documentation(
                    &mut out,
                    source,
                    import.span.start,
                    import.keyword_span,
                    "//!",
                    import.doc_comment.as_deref(),
                    0,
                );
                out.push_str("use");
                out.push_str(&emit_decl_gap(
                    source,
                    import.keyword_span,
                    import.path[0].1,
                    &[],
                    0,
                ));
                out.push(' ');
                write_path(&mut out, source, &import.path, 0);
                let mut previous = import.path.last().expect("parsed paths are nonempty").1;
                if let Some((alias, span)) = &import.alias {
                    out.push_str(&emit_decl_gap(source, previous, *span, &["as"], 0));
                    out.push_str(alias);
                    previous = *span;
                }
                out.push_str(&emit_decl_gap(
                    source,
                    previous,
                    point_span(import.span.end),
                    &[],
                    0,
                ));
                out.push_str(";\n");
            }
            ast::ModuleItem::Sheet(sheet) => {
                return Err(FormatSourceError::Recovered(sheet.errors.clone()));
            }
            ast::ModuleItem::Error(span) => {
                return Err(FormatSourceError::Recovered(vec![
                    cel_parser::ParseError::new("recovered module item", span.start),
                ]));
            }
        }
    }
    if let Some(comments) = module.comments.get(module.items.len()) {
        for comment in comments {
            write_trivia(&mut out, false, Some(comment), 0);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    /// Measures the active line, counting Unicode scalars rather than UTF-8 bytes.
    #[test]
    fn output_column_tracks_the_active_line() {
        for (text, expected) in [
            ("", 0),
            ("cell é = ", 9),
            ("old prefix\n    := ", 7),
            ("line\n", 0),
        ] {
            assert_eq!(super::output_column(text), expected);
        }
    }

    /// Verifies grouped restricted subjects retain boundary comments exactly once and in order.
    #[test]
    fn grouped_conditional_headers_preserve_comments() {
        for header in [
            "(Sample { value: 1.0 }) /* first */",
            "/* first */ (Sample { value: 1.0 })",
            "(/* first */ Sample { value: 1.0 } /* second */) /* third */",
            "(/* first */ (/* second */ Sample { value: 1.0 }) /* third */) /* fourth */",
            "(Sample { value: 1.0 }) /* first /* nested ) */ end */",
            "(/* first /* nested ( */ end */ Sample { value: 1.0 }) /* second */",
            "((Sample { value: 1.0 }) /* first */) /* second */",
            "(// first\n Sample { value: 1.0 } // second\n) // third\n",
            "// first\n (/* second */ Sample { value: 1.0 }) // third\n",
            "(/* first */ p) /* second */",
            "(/* first */ (Sample { value: 1.0 }).value) /* second */",
        ] {
            let source = format!("sheet s {{ conditional {header} {{ _ => {{}} }} }}");
            let formatted = super::format_source(&source).unwrap();
            let mut previous = 0;
            for label in ["first", "second", "third", "fourth"] {
                if source.contains(label) {
                    assert_eq!(
                        formatted.matches(label).count(),
                        1,
                        "{source} -> {formatted}"
                    );
                    let position = formatted.find(label).unwrap();
                    assert!(position >= previous, "{formatted}");
                    previous = position;
                }
            }
            assert_eq!(super::format_source(&formatted).unwrap(), formatted);
        }
    }

    use super::*;
    use crate::AdamAstParser;

    fn format(source: &str) -> String {
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        crate::attach_trivia(source, &mut sheet);
        format_sheet(&sheet, source)
    }

    #[test]
    fn formats_an_empty_sheet() {
        assert_eq!(format("sheet s {}"), "sheet s {\n}\n");
    }

    #[test]
    fn preserves_a_leading_comment_before_the_sheet_itself() {
        assert_eq!(
            format("// file header\nsheet s { cell x: i32 = 1; }"),
            "// file header\nsheet s {\n    cell x: i32 = 1;\n}\n"
        );
    }

    #[test]
    fn no_leading_comment_before_the_sheet_emits_nothing_extra() {
        assert_eq!(
            format("sheet s { cell x: i32 = 1; }"),
            "sheet s {\n    cell x: i32 = 1;\n}\n"
        );
    }

    #[test]
    fn formats_a_cell_with_type_and_initializer() {
        assert_eq!(
            format("sheet s { cell width: f64 = 1920.0; }"),
            "sheet s {\n    cell width: f64 = 1920.0;\n}\n"
        );
    }

    #[test]
    fn formats_a_source_decl() {
        assert_eq!(
            format("sheet s { source width: i32 = 4; }"),
            "sheet s {\n    source width: i32 = 4;\n}\n"
        );
    }

    #[test]
    fn formats_a_cell_with_only_a_type_annotation() {
        assert_eq!(
            format("sheet s { cell area: f64; }"),
            "sheet s {\n    cell area: f64;\n}\n"
        );
    }

    #[test]
    fn formats_a_cell_with_only_an_initializer() {
        assert_eq!(
            format("sheet s { cell mode = 0i32; }"),
            "sheet s {\n    cell mode = 0i32;\n}\n"
        );
    }

    #[test]
    fn packed_cells_stay_packed_and_a_blank_line_before_a_relationship_is_preserved() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    cell b: i32 = 2;\n\n    relationship { b := a; }\n}";
        let expected = "sheet s {\n    cell a: i32 = 1;\n    cell b: i32 = 2;\n\n    relationship {\n        b := a;\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn a_run_of_blank_lines_collapses_to_one() {
        let source = "sheet s {\n    cell a: i32 = 1;\n\n\n\n    cell b: i32 = 2;\n}";
        let expected = "sheet s {\n    cell a: i32 = 1;\n\n    cell b: i32 = 2;\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_relationship_with_multiple_bindings() {
        let source = "sheet s {\n    relationship {\n        area := width * height;\n        width := area / height;\n    }\n}";
        let expected = "sheet s {\n    relationship {\n        area := width * height;\n        width := area / height;\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_multi_output_destructuring_binding_with_parens() {
        let source =
            "sheet s {\n    relationship {\n        (sum, diff) := (a + b, a - b);\n    }\n}";
        let expected =
            "sheet s {\n    relationship {\n        (sum, diff) := (a + b, a - b);\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_single_element_tuple_destructuring_binding_with_a_trailing_comma() {
        let source = "sheet s {\n    relationship {\n        (x,) := (a,);\n    }\n}";
        let expected = "sheet s {\n    relationship {\n        (x,) := (a,);\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn drops_redundant_grouping_parens_from_a_non_destructuring_binding() {
        let source = "sheet s {\n    relationship {\n        (x) := a;\n    }\n}";
        let expected = "sheet s {\n    relationship {\n        x := a;\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn preserves_a_comment_on_a_nested_binding() {
        let source = "sheet s {\n    relationship {\n        b := a;\n\n        // second\n        a := b;\n    }\n}";
        let expected = "sheet s {\n    relationship {\n        b := a;\n\n        // second\n        a := b;\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_conditional_with_branches_and_a_default_and_no_trailing_commas() {
        let source = "sheet s {\n    conditional p {\n        0i32 => { relationship { b := a; } },\n        _ => { relationship { a := b; } },\n    }\n}";
        let expected = "sheet s {\n    conditional p {\n        0i32 => {\n            relationship {\n                b := a;\n            }\n        }\n        _ => {\n            relationship {\n                a := b;\n            }\n        }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_conditional_branch_with_a_negated_literal_key() {
        let source = "sheet s {\n    conditional p {\n        -1i32 => { relationship { b := a; } },\n        _ => { relationship { a := b; } },\n    }\n}";
        let expected = "sheet s {\n    conditional p {\n        -1i32 => {\n            relationship {\n                b := a;\n            }\n        }\n        _ => {\n            relationship {\n                a := b;\n            }\n        }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_conditional_with_an_expression_match_subject() {
        let source = "sheet s {\n    conditional a && b {\n        _ => { relationship { d := c; } },\n    }\n}";
        let expected = "sheet s {\n    conditional a && b {\n        _ => {\n            relationship {\n                d := c;\n            }\n        }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn preserves_a_comment_on_a_conditional_branch() {
        let source = "sheet s {\n    conditional m {\n        0i32 => { relationship { b := a; } }\n        // one\n        1i32 => { relationship { b := a; } }\n    }\n}";
        let expected = "sheet s {\n    conditional m {\n        0i32 => {\n            relationship {\n                b := a;\n            }\n        }\n        // one\n        1i32 => {\n            relationship {\n                b := a;\n            }\n        }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn binding_body_delegates_precedence_aware_parenthesization_to_cel_parser() {
        let source = "sheet s { relationship { c := (a + b) * 2i32; } }";
        let expected = "sheet s {\n    relationship {\n        c := (a + b) * 2i32;\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn format_is_idempotent_through_a_reparse() {
        let source = "sheet demo {\n    cell a: f64 = 2.0;\n    cell b: f64 = 3.0;\n\n    relationship {\n        c := a * b;\n    }\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn format_is_idempotent_through_a_reparse_with_a_conditional() {
        let source = "sheet demo {\n    cell p: i32 = 0;\n    cell c: f64;\n\n    conditional p {\n        0i32 => {\n            relationship {\n                c := c;\n            }\n        }\n        _ => {\n            relationship {\n                c := c;\n            }\n        }\n    }\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn formats_an_out_with_explicit_type_and_no_requirements() {
        let source = "sheet s {\n    out area: f64 := width * height;\n}";
        let expected = "sheet s {\n    out area: f64 := width * height;\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_an_out_with_no_type_annotation() {
        let source = "sheet s {\n    out area := width;\n}";
        let expected = "sheet s {\n    out area := width;\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_an_out_with_requirements_in_declaration_order() {
        let source = "sheet s {\n    out area: f64 := width * height require {\n        @max_area width * height <= max_area;\n    };\n}";
        let expected = "sheet s {\n    out area: f64 := width * height require {\n        @max_area width * height <= max_area;\n    };\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_cell_with_a_require_block() {
        let source =
            "sheet s {\n    cell x: i32 = 5 require {\n        @positive x > 0;\n    };\n}";
        assert_eq!(format(source), format!("{source}\n"));
    }

    #[test]
    fn formats_a_source_with_a_require_block() {
        let source =
            "sheet s {\n    source x: i32 = 5 require {\n        @positive x > 0;\n    };\n}";
        assert_eq!(format(source), format!("{source}\n"));
    }

    #[test]
    fn formats_a_source_with_a_filter_clause() {
        let source = "sheet s {\n    source x: i32 = 5 filter 0..=10;\n}";
        assert_eq!(format(source), format!("{source}\n"));
    }

    #[test]
    fn formats_an_out_with_a_filter_clause() {
        let source =
            "sheet s {\n    cell width: i32 = 4;\n    out area := width filter 0..=100;\n}";
        assert_eq!(format(source), format!("{source}\n"));
    }

    #[test]
    fn formats_a_cell_with_an_explicit_tuple_type() {
        assert_eq!(
            format("sheet s { cell a: (i32, f64) = (1, 2.5); }"),
            "sheet s {\n    cell a: (i32, f64) = (1, 2.5);\n}\n"
        );
    }

    #[test]
    fn formats_a_cell_with_a_nested_tuple_type() {
        assert_eq!(
            format("sheet s { cell a: (i32, (f64, String)); }"),
            "sheet s {\n    cell a: (i32, (f64, String));\n}\n"
        );
    }

    #[test]
    fn formats_an_out_with_an_explicit_tuple_type() {
        assert_eq!(
            format("sheet s { out a: (i32, i32) := (x, x); }"),
            "sheet s {\n    out a: (i32, i32) := (x, x);\n}\n"
        );
    }

    #[test]
    fn format_is_idempotent_through_a_reparse_with_a_tuple_cell() {
        let source = "sheet s {\n    cell a: (i32, f64) = (1, 2.5);\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn formats_a_single_line_block_comment_preserving_its_style() {
        // A comment immediately after the sheet's opening `{` (before its first item) falls into
        // the untracked gap from issue #52 (see trivia.rs's module doc) — unrelated to this
        // change — so a preceding cell is included here to land the comment in a tracked gap.
        // The comment is on its own source line so it attaches as `x`'s leading comment, not `a`'s
        // same-line trailing comment (see issue #59) — what this test is actually checking is that
        // a `/* */` leading comment round-trips as `/* */`, not `//`.
        let source =
            "sheet s {\n    cell a: i32 = 1;\n    /* the total */\n    cell x: i32 = 1;\n}";
        let expected =
            "sheet s {\n    cell a: i32 = 1;\n    /* the total */\n    cell x: i32 = 1;\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_multi_line_block_comment_preserving_its_style() {
        // See the comment on `formats_a_single_line_block_comment_preserving_its_style` for why
        // a preceding cell is included.
        let source = "sheet s {\n    cell a: i32 = 1;\n    /*\n        line one\n        line two\n    */\n    cell x: i32 = 1;\n}";
        let expected = "sheet s {\n    cell a: i32 = 1;\n    /*\n        line one\n        line two\n    */\n    cell x: i32 = 1;\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_the_issue_105_license_header_repro_without_dropping_it() {
        let source =
            "/*\n    Copyright 2013 Adobe\n    ...\n*/\nsheet s {\n    cell a: i32 = 1;\n}";
        let expected =
            "/*\n    Copyright 2013 Adobe\n    ...\n*/\nsheet s {\n    cell a: i32 = 1;\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn block_comment_formatting_is_idempotent_through_a_reparse() {
        // See the comment on `formats_a_single_line_block_comment_preserving_its_style` for why
        // a preceding cell is included: without it, the comment lands in issue #52's untracked
        // gap and is dropped on the *first* format, making `once == twice` trivially true
        // regardless of whether the comment round-trips.
        let source = "sheet s {\n    cell a: i32 = 1;\n    /*\n        line one\n        line two\n    */\n    cell x: i32 = 1;\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn formats_a_cell_with_a_doc_comment() {
        assert_eq!(
            format("sheet s { /// the total\n cell x: i32 = 1; }"),
            "sheet s {\n    /// the total\n    cell x: i32 = 1;\n}\n"
        );
    }

    #[test]
    fn formats_a_sheet_level_doc_comment() {
        assert_eq!(
            format("//! module docs\nsheet s { cell x: i32 = 1; }"),
            "//! module docs\nsheet s {\n    cell x: i32 = 1;\n}\n"
        );
    }

    #[test]
    fn formats_a_plain_comment_and_doc_comment_together_in_source_order() {
        // Two items, not one: `trivia::attach_gaps` never attaches a leading plain comment to a
        // list's first element (a pre-existing, out-of-scope #52-adjacent limitation unrelated to
        // doc comments — see Tasks 2/3's identical fixture fix), so `x` needs a preceding sibling
        // for its `// TODO` to actually attach via the normal gap-scanning path.
        let source =
            "sheet s {\n    cell w: i32 = 0;\n    // TODO\n    /// docs\n    cell x: i32 = 1;\n}";
        let expected =
            "sheet s {\n    cell w: i32 = 0;\n    // TODO\n    /// docs\n    cell x: i32 = 1;\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_doc_comments_on_a_relationship_conditional_and_out() {
        let source = "sheet s {\n    /// r\n    relationship { b := a; }\n\n    /// o\n    out area: f64 := w;\n}";
        let expected = "sheet s {\n    /// r\n    relationship {\n        b := a;\n    }\n\n    /// o\n    out area: f64 := w;\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn doc_comment_formatting_is_idempotent_through_a_reparse() {
        let source = "sheet s {\n    /// the total\n    cell x: i32 = 1;\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn formats_a_trailing_comment_before_the_sheets_closing_brace() {
        assert_eq!(
            format("sheet s {\n    cell x: i32 = 1;\n    // trailing\n}"),
            "sheet s {\n    cell x: i32 = 1;\n    // trailing\n}\n"
        );
    }

    #[test]
    fn formats_a_trailing_comment_in_an_empty_relationship() {
        assert_eq!(
            format("sheet s {\n    relationship {\n        // only this\n    }\n}"),
            "sheet s {\n    relationship {\n        // only this\n    }\n}\n"
        );
    }

    #[test]
    fn formats_a_trailing_comment_before_a_relationships_closing_brace() {
        let source =
            "sheet s {\n    relationship {\n        b := a;\n        // trailing\n    }\n}";
        let expected =
            "sheet s {\n    relationship {\n        b := a;\n        // trailing\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_trailing_comment_before_a_conditionals_own_closing_brace() {
        let source = "sheet s {\n    conditional m {\n        0i32 => { relationship { b := a; } }\n        // trailing\n    }\n}";
        let expected = "sheet s {\n    conditional m {\n        0i32 => {\n            relationship {\n                b := a;\n            }\n        }\n        // trailing\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_trailing_comment_in_a_default_arm() {
        let source = "sheet s {\n    conditional m {\n        _ => {\n            relationship { b := a; }\n            // trailing\n        }\n    }\n}";
        let expected = "sheet s {\n    conditional m {\n        _ => {\n            relationship {\n                b := a;\n            }\n            // trailing\n        }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_trailing_comment_before_a_requires_closing_brace() {
        let source = "sheet s {\n    out area: f64 := w require {\n        @c w <= 10.0;\n        // trailing\n    };\n}";
        let expected = "sheet s {\n    out area: f64 := w require {\n        @c w <= 10.0;\n        // trailing\n    };\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_trailing_comment_before_a_cells_requires_closing_brace() {
        // Mirrors `formats_a_trailing_comment_before_a_requires_closing_brace`, but for a
        // `cell`'s own `require` block — regression test for the comment being silently dropped
        // (never attached, so never written back out) because `attach_trivia` only recovered
        // require-block trivia for `SheetItem::Out`, not `SheetItem::Cell`/`SheetItem::Source`.
        let source = "sheet s {\n    cell a: i32 = 1 require {\n        @r a > 0;\n        // trailing\n    };\n}";
        let expected = "sheet s {\n    cell a: i32 = 1 require {\n        @r a > 0;\n        // trailing\n    };\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_trailing_comment_before_a_sources_requires_closing_brace() {
        // Mirrors `formats_a_trailing_comment_before_a_requires_closing_brace`, but for a
        // `source`'s own `require` block.
        let source = "sheet s {\n    source a: i32 = 1 require {\n        @r a > 0;\n        // trailing\n    };\n}";
        let expected = "sheet s {\n    source a: i32 = 1 require {\n        @r a > 0;\n        // trailing\n    };\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn trailing_trivia_formatting_is_idempotent_through_a_reparse() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // trailing\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn formats_a_cell_with_a_filter() {
        assert_eq!(
            format("sheet s { cell a: i32 = 1 filter _; }"),
            "sheet s {\n    cell a: i32 = 1 filter _;\n}\n"
        );
    }

    #[test]
    fn formats_a_cell_with_a_filter_referencing_a_cell() {
        assert_eq!(
            format("sheet s { cell hi: i32 = 100; cell a: i32 = 1 filter min(_, hi); }"),
            "sheet s {\n    cell hi: i32 = 100;\n    cell a: i32 = 1 filter min(_, hi);\n}\n"
        );
    }

    #[test]
    fn format_is_idempotent_through_a_reparse_with_a_filter() {
        let source = "sheet s {\n    cell a: i32 = 1 filter _;\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn formats_a_filter() {
        let source = "sheet s { cell x: i32 = 0 filter 0..=10; }";
        let sheet = AdamAstParser::new().parse_str(source).unwrap();
        assert_eq!(
            format_sheet(&sheet, source),
            "sheet s {\n    cell x: i32 = 0 filter 0..=10;\n}\n"
        );
    }

    #[test]
    fn formats_a_requirement_with_no_label() {
        let source = "sheet s {\n    cell x: i32 = 5 require {\n        x > 0;\n    };\n}";
        assert_eq!(format(source), format!("{source}\n"));
    }

    #[test]
    fn format_source_formats_valid_source() {
        assert_eq!(
            format_source("sheet   s{cell x:i32=1;}").unwrap(),
            "sheet s {\n    cell x: i32 = 1;\n}\n"
        );
    }

    #[test]
    fn format_source_errors_on_source_that_fails_to_parse() {
        assert!(matches!(
            format_source("not a sheet at all"),
            Err(FormatSourceError::Parse(_))
        ));
    }

    #[test]
    fn format_source_errors_on_a_recovered_syntax_error() {
        assert!(matches!(
            format_source("sheet s { cell x unknown_syntax }"),
            Err(FormatSourceError::Recovered(_))
        ));
    }

    // Issue #59: a same-line trailing `//`/`/* */` comment stays on the declaration it trails,
    // instead of migrating onto its own line before the next declaration.

    #[test]
    fn formats_a_same_line_trailing_line_comment_on_a_cell() {
        let source =
            "sheet s {\n    cell a: f64 = 2.0; // end of line comment\n    cell b: f64 = 3.0;\n}";
        let expected =
            "sheet s {\n    cell a: f64 = 2.0; // end of line comment\n    cell b: f64 = 3.0;\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_same_line_trailing_block_comment_on_a_cell() {
        let source = "sheet s {\n    cell a: f64 = 2.0; /* note */\n    cell b: f64 = 3.0;\n}";
        let expected = "sheet s {\n    cell a: f64 = 2.0; /* note */\n    cell b: f64 = 3.0;\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_same_line_trailing_comment_on_the_last_item_before_the_sheets_closing_brace() {
        let source = "sheet s {\n    cell a: i32 = 1; // note\n}";
        let expected = "sheet s {\n    cell a: i32 = 1; // note\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_same_line_trailing_comment_on_a_binding() {
        let source =
            "sheet s {\n    relationship {\n        b := a; // note\n        a := b;\n    }\n}";
        let expected =
            "sheet s {\n    relationship {\n        b := a; // note\n        a := b;\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_same_line_block_comment_when_the_gap_has_no_newline_at_all() {
        // PR #202 review: previously the trailing `/* note */` was silently dropped, since
        // items+comment sharing one physical line with no `\n` anywhere in the gap fell into
        // the "drop the final fragment" path before the comment scanner ever ran.
        let source = "sheet s {\n    cell a: i32 = 1; /* note */ cell b: i32 = 2;\n}";
        let expected = "sheet s {\n    cell a: i32 = 1; /* note */\n    cell b: i32 = 2;\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_same_line_trailing_comment_before_a_conditional_branchs_own_closing_brace() {
        let source = "sheet s {\n    conditional m {\n        0i32 => {\n            relationship { b := a; } // note\n        }\n    }\n}";
        let expected = "sheet s {\n    conditional m {\n        0i32 => {\n            relationship {\n                b := a;\n            } // note\n        }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_same_line_trailing_comment_before_a_default_arms_own_closing_brace() {
        let source = "sheet s {\n    conditional m {\n        _ => {\n            relationship { b := a; } // note\n        }\n    }\n}";
        let expected = "sheet s {\n    conditional m {\n        _ => {\n            relationship {\n                b := a;\n            } // note\n        }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_same_line_trailing_comment_before_a_conditionals_own_closing_brace_with_only_branches()
     {
        let source = "sheet s {\n    conditional m {\n        0i32 => { relationship { b := a; } } // note\n    }\n}";
        let expected = "sheet s {\n    conditional m {\n        0i32 => {\n            relationship {\n                b := a;\n            }\n        } // note\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_same_line_trailing_comment_before_a_conditionals_own_closing_brace_with_a_default()
    {
        let source = "sheet s {\n    conditional m {\n        _ => { relationship { b := a; } } // note\n    }\n}";
        let expected = "sheet s {\n    conditional m {\n        _ => {\n            relationship {\n                b := a;\n            }\n        } // note\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_same_line_trailing_comment_before_a_requires_closing_brace() {
        let source =
            "sheet s {\n    out area: f64 := w require {\n        @c w <= 10.0; // note\n    };\n}";
        let expected = "sheet s {\n    out area: f64 := w require {\n        @c w <= 10.0; // note\n    };\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn same_line_trailing_comment_formatting_is_idempotent_through_a_reparse() {
        let source =
            "sheet s {\n    cell a: f64 = 2.0; // end of line comment\n    cell b: f64 = 3.0;\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }

    // Issue #201: comments in gaps *within* one declaration's own tokens (as opposed to gaps
    // between sibling declarations, or before a container's closing brace — both already handled
    // by `trivia.rs`).

    #[test]
    fn preserves_a_block_comment_between_a_cell_initializer_and_its_semicolon() {
        let source = "sheet s {\n    cell a: i32 = 1 /* trailing */;\n}";
        assert_eq!(
            format(source),
            "sheet s {\n    cell a: i32 = 1 /* trailing */;\n}\n"
        );
    }

    #[test]
    fn preserves_a_block_comment_between_a_type_and_the_equals() {
        let source = "sheet s {\n    cell a: i32 /* t */ = 1;\n}";
        assert_eq!(
            format(source),
            "sheet s {\n    cell a: i32 /* t */ = 1;\n}\n"
        );
    }

    #[test]
    fn a_declaration_with_no_intra_token_comments_is_unchanged() {
        assert_eq!(
            format("sheet s { cell a: i32 = 1; }"),
            "sheet s {\n    cell a: i32 = 1;\n}\n"
        );
    }

    #[test]
    fn a_cell_initializer_with_leading_inline_and_trailing_comments_round_trips_losslessly() {
        let source = "sheet s {\n    cell a: i32 = /* p */ 1 /* q */ + 2 /* r */;\n}";
        let once = format_source(source).unwrap();
        // Every comment survives (none silently dropped).
        for needle in ["/* p */", "/* q */", "/* r */"] {
            assert!(once.contains(needle), "comment {needle} lost: {once:?}");
        }
        // And formatting is idempotent.
        let twice = format_source(&once).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn preserves_a_line_comment_between_a_cell_initializer_and_its_semicolon() {
        // A `//` comment forces a line wrap; the terminating `;` must land on the continuation
        // line, not get glued onto the comment's own line, which would comment the `;` out and
        // make the output fail to reparse.
        let source = "sheet s {\n    cell a: i32 = 1 // c\n    ;\n}";
        let once = format_source(source).unwrap();
        assert!(once.contains("// c"), "comment must survive: {once:?}");
        assert!(
            once.trim_end().ends_with('}'),
            "the sheet's own closing brace must not have been commented out: {once:?}"
        );
        // Must still reparse (a stray `;` swallowed into the comment would fail this).
        let twice = format_source(&once).unwrap();
        assert_eq!(once, twice, "formatting must be idempotent");
    }

    #[test]
    fn preserves_a_line_comment_between_a_type_and_the_equals() {
        let source = "sheet s {\n    cell a: i32 // c\n    = 1;\n}";
        let once = format_source(source).unwrap();
        assert!(once.contains("// c"), "comment must survive: {once:?}");
        let twice = format_source(&once).unwrap();
        assert_eq!(once, twice, "formatting must be idempotent");
    }

    #[test]
    fn preserves_comment_order_after_the_equals() {
        let source = "sheet s {\n    cell a: i32 = /* after */ 1;\n}";
        let once = format_source(source).unwrap();
        let eq = once.find('=').expect("has =");
        let c = once.find("/* after */").expect("comment survives");
        assert!(c > eq, "comment must stay AFTER the =, got: {once:?}");
    }

    #[test]
    fn preserves_comment_order_around_the_equals() {
        let source = "sheet s {\n    cell a: i32 /* before */ = /* after */ 1;\n}";
        let once = format_source(source).unwrap();
        let before = once.find("/* before */").expect("before survives");
        let eq = once.find('=').expect("has =");
        let after = once.find("/* after */").expect("after survives");
        assert!(before < eq && eq < after, "order lost: {once:?}");
    }

    #[test]
    fn preserves_a_comment_before_the_filter_keyword() {
        let source = "sheet s {\n    cell a: i32 = 1 /* keep */ filter 0..=10;\n}";
        let once = format_source(source).unwrap();
        assert!(once.contains("/* keep */"), "comment dropped: {once:?}");
    }

    #[test]
    fn preserves_a_comment_after_the_filter_body() {
        let source = "sheet s {\n    cell a: i32 = 1 filter 0..=10 /* keep */;\n}";
        let once = format_source(source).unwrap();
        assert!(once.contains("/* keep */"), "comment dropped: {once:?}");
    }

    #[test]
    fn preserves_a_comment_before_the_require_keyword() {
        let source = "sheet s {\n    out area: f64 := w /* keep */ require {\n        @c w <= 10.0;\n    };\n}";
        let once = format_source(source).unwrap();
        assert!(once.contains("/* keep */"), "comment dropped: {once:?}");
    }

    /// Every keyword-to-body gap must preserve a comment and stay idempotent — one case per
    /// hardcoded token the declaration formatter reprints (`filter`, `require`, `=>`, the `@label`,
    /// the conditional subject's `{`, and the `relationship` keyword's `{`). See PR #207 review.
    fn assert_comment_survives_and_is_idempotent(source: &str) {
        let once = format_source(source).unwrap();
        assert!(once.contains("/* keep */"), "comment dropped: {once:?}");
        let twice = format_source(&once).unwrap();
        assert_eq!(once, twice, "formatting must be idempotent: {once:?}");
    }

    #[test]
    fn preserves_a_comment_between_the_filter_keyword_and_body() {
        assert_comment_survives_and_is_idempotent(
            "sheet s {\n    cell a: i32 = 1 filter /* keep */ 0..=10;\n}",
        );
    }

    #[test]
    fn preserves_a_comment_between_the_require_keyword_and_brace() {
        assert_comment_survives_and_is_idempotent(
            "sheet s {\n    out area: f64 := w require /* keep */ {\n        @c w <= 10.0;\n    };\n}",
        );
    }

    #[test]
    fn preserves_a_comment_between_a_branch_literal_and_the_fat_arrow() {
        assert_comment_survives_and_is_idempotent(
            "sheet s {\n    conditional p {\n        0i32 /* keep */ => { relationship { b := a; } }\n    }\n}",
        );
    }

    #[test]
    fn preserves_a_comment_between_the_fat_arrow_and_a_branch_body() {
        assert_comment_survives_and_is_idempotent(
            "sheet s {\n    conditional p {\n        0i32 => /* keep */ { relationship { b := a; } }\n    }\n}",
        );
    }

    #[test]
    fn preserves_a_comment_between_a_requirement_label_and_body() {
        assert_comment_survives_and_is_idempotent(
            "sheet s {\n    out area: f64 := w require {\n        @c /* keep */ w <= 10.0;\n    };\n}",
        );
    }

    #[test]
    fn preserves_a_comment_between_a_conditional_subject_and_its_brace() {
        assert_comment_survives_and_is_idempotent(
            "sheet s {\n    conditional p /* keep */ {\n        0i32 => { relationship { b := a; } }\n    }\n}",
        );
    }

    #[test]
    fn preserves_a_comment_between_the_relationship_keyword_and_its_brace() {
        assert_comment_survives_and_is_idempotent(
            "sheet s {\n    relationship /* keep */ {\n        b := a;\n    }\n}",
        );
    }
}
