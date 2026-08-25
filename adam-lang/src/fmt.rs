//! Pretty-prints an [`crate::ast::Sheet`] back to adam-lang source text: 4-space indentation,
//! opening braces on the same line, `leading_comment`/`blank_line_before` reproduced exactly as
//! [`crate::trivia::attach_trivia`] recovered them (including a file-header-style comment
//! preceding the `sheet` keyword itself, `Sheet.leading_comment`), and binding bodies/cell
//! initializers/requirement bodies delegated to [`cel_parser::format_expr`] (a normalization
//! improvement over span-based re-emit, which was only ever a stopgap for when there was no
//! parsed `Expr` to format). Type annotations are re-emitted via `Span::source_text()` directly
//! via `TypeExpr::span()`, and branch-match literals via `ConditionalBranch::literal_span` — see
//! the design doc for why no `Literal` value is needed. Conditional branches omit the grammar's
//! optional trailing `,`, matching `begin/assets/demo.adm2`'s existing style.
//!
//! Never called on a sheet with any recorded syntax errors — see `adam-lsp`'s
//! `textDocument/formatting` handler, which refuses to format in that case.

use crate::ast;

/// 4 spaces per nesting level.
fn indent(depth: usize) -> String {
    "    ".repeat(depth)
}

/// Writes one recovered `Comment` at `depth`'s indentation: a `Comment::Line` as one `// ` line
/// per stored line; a `Comment::Block` as `/* text */` on one line when its text has no internal
/// `\n`, or a multi-line `/*`/`*/`-delimited block (one line per stored line, indented one level
/// past `depth`) when it does.
fn write_comment(out: &mut String, comment: &ast::Comment, depth: usize) {
    match comment {
        ast::Comment::Line(text) => {
            for line in text.split('\n') {
                out.push_str(&indent(depth));
                out.push_str("// ");
                out.push_str(line);
                out.push('\n');
            }
        }
        ast::Comment::Block(text) => {
            out.push_str(&indent(depth));
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

/// Re-emits a literal's exact original text via its span, falling back to an empty string when
/// none is recoverable — mirrors `cel_parser::fmt`'s identical fallback (see the module doc for
/// why no `Literal` value is needed here).
fn source_text_or_empty(span: ast::ExprSpan) -> String {
    span.start.source_text().unwrap_or_default()
}

/// Writes one `a := ...;` / `(a, b) := ...;` binding, delegating its body to
/// [`cel_parser::format_expr`].
fn write_binding(out: &mut String, binding: &ast::BindingDecl, depth: usize) {
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
    out.push_str(" := ");
    out.push_str(&cel_parser::format_expr(&binding.body));
    out.push_str(";\n");
}

/// Writes one `relationship { ... }` declaration and its bindings, in declaration order.
fn write_relationship(out: &mut String, rel: &ast::RelationshipDecl, depth: usize) {
    write_trivia(
        out,
        rel.blank_line_before,
        rel.leading_comment.as_ref(),
        depth,
    );
    write_doc_comment(out, "///", rel.doc_comment.as_deref(), depth);
    out.push_str(&indent(depth));
    out.push_str("relationship {\n");
    for binding in &rel.bindings {
        write_binding(out, binding, depth + 1);
    }
    write_trailing_trivia(
        out,
        rel.blank_line_before_close,
        rel.trailing_comment.as_ref(),
        depth + 1,
    );
    out.push_str(&indent(depth));
    out.push_str("}\n");
}

/// Writes a `{ ... }` block of relationships, shared by both a named conditional branch and the
/// default (`_ =>`) arm.
fn write_branch_relationships(
    out: &mut String,
    relationships: &[ast::RelationshipDecl],
    trailing_comment: Option<&ast::Comment>,
    blank_line_before_close: bool,
    depth: usize,
) {
    out.push_str("{\n");
    for rel in relationships {
        write_relationship(out, rel, depth + 1);
    }
    write_trailing_trivia(out, blank_line_before_close, trailing_comment, depth + 1);
    out.push_str(&indent(depth));
    out.push_str("}\n");
}

/// Writes one `literal => { ... }` conditional branch, re-emitting the match literal via its
/// span rather than the (unused) `Literal` value.
fn write_branch(out: &mut String, branch: &ast::ConditionalBranch, depth: usize) {
    write_trivia(
        out,
        branch.blank_line_before,
        branch.leading_comment.as_ref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str(&source_text_or_empty(branch.literal_span));
    out.push_str(" => ");
    write_branch_relationships(
        out,
        &branch.relationships,
        branch.trailing_comment.as_ref(),
        branch.blank_line_before_close,
        depth,
    );
}

/// Writes one `conditional <expr> { ... }` declaration: its branches in declaration
/// order (dispatching on the match-subject expression), followed by its optional `_ => { ... }` default arm.
fn write_conditional(out: &mut String, cond: &ast::ConditionalDecl, depth: usize) {
    write_trivia(
        out,
        cond.blank_line_before,
        cond.leading_comment.as_ref(),
        depth,
    );
    write_doc_comment(out, "///", cond.doc_comment.as_deref(), depth);
    out.push_str(&indent(depth));
    out.push_str("conditional ");
    out.push_str(&cel_parser::format_expr(&cond.match_expr));
    out.push_str(" {\n");
    for branch in &cond.branches {
        write_branch(out, branch, depth + 1);
    }
    if let Some(default) = &cond.default {
        out.push_str(&indent(depth + 1));
        out.push_str("_ => ");
        write_branch_relationships(
            out,
            &default.relationships,
            default.trailing_comment.as_ref(),
            default.blank_line_before_close,
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
    out.push_str("}\n");
}

/// Writes one `cell name[: type][ = initializer][ filter body];` declaration, delegating its
/// type annotation to [`source_text_or_empty`] via `TypeExpr::span()` and its initializer/filter
/// body to [`cel_parser::format_expr`].
fn write_cell(out: &mut String, cell: &ast::CellDecl, depth: usize) {
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
    if let Some(expr) = &cell.initializer {
        out.push_str(" = ");
        out.push_str(&cel_parser::format_expr(expr));
    }
    if let Some(filter) = &cell.filter {
        out.push_str(" filter ");
        out.push_str(&cel_parser::format_expr(&filter.body));
    }
    out.push_str(";\n");
}

/// Writes one `name: ...;` requirement.
fn write_requirement(out: &mut String, req: &ast::RequirementDecl, depth: usize) {
    write_trivia(
        out,
        req.blank_line_before,
        req.leading_comment.as_ref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str(&req.name);
    out.push_str(": ");
    out.push_str(&cel_parser::format_expr(&req.body));
    out.push_str(";\n");
}

/// Writes one `out name[: type] := ...[ require { ... } ];` declaration.
fn write_out(out: &mut String, decl: &ast::OutDecl, depth: usize) {
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
    out.push_str(" := ");
    out.push_str(&cel_parser::format_expr(&decl.initializer));
    if let Some(require) = &decl.require {
        out.push_str(" require {\n");
        for req in &require.requirements {
            write_requirement(out, req, depth + 1);
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
    out.push_str(";\n");
}

/// Dispatches to the writer for one top-level sheet item.
///
/// - Precondition: `item` is not `SheetItem::Error` — [`format_sheet`]'s own precondition
///   (`sheet.errors.is_empty()`) guarantees no `Error` item ever reaches this function.
fn write_sheet_item(out: &mut String, item: &ast::SheetItem, depth: usize) {
    match item {
        ast::SheetItem::Cell(cell) => write_cell(out, cell, depth),
        ast::SheetItem::Relationship(rel) => write_relationship(out, rel, depth),
        ast::SheetItem::Conditional(cond) => write_conditional(out, cond, depth),
        ast::SheetItem::Out(out_decl) => write_out(out, out_decl, depth),
        ast::SheetItem::Error { .. } => {
            unreachable!("format_sheet is only called on a sheet with no recorded syntax errors")
        }
    }
}

/// Pretty-prints `sheet` back to adam-lang source text — see the module doc for the printing
/// rules.
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
/// assert_eq!(format_sheet(&sheet), "sheet s {\n    cell x: i32 = 1;\n}\n");
/// ```
pub fn format_sheet(sheet: &ast::Sheet) -> String {
    debug_assert!(
        sheet.errors.is_empty(),
        "format_sheet's precondition: no recorded syntax errors"
    );
    let mut out = String::new();
    write_trivia(&mut out, false, sheet.leading_comment.as_ref(), 0);
    write_doc_comment(&mut out, "//!", sheet.doc_comment.as_deref(), 0);
    out.push_str(&format!("sheet {} {{\n", sheet.name));
    for item in &sheet.items {
        write_sheet_item(&mut out, item, 1);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AdamAstParser;

    fn format(source: &str) -> String {
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        crate::attach_trivia(source, &mut sheet);
        format_sheet(&sheet)
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
        let source = "sheet s {\n    out area: f64 := width * height require {\n        max_area: width * height <= max_area;\n    };\n}";
        let expected = "sheet s {\n    out area: f64 := width * height require {\n        max_area: width * height <= max_area;\n    };\n}\n";
        assert_eq!(format(source), expected);
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
        // The comment also needs its own source line: when a comment and its neighboring items
        // all share one physical line, `analyze_gap`'s same-line-fragment handling (pre-existing,
        // also unrelated to this change) can't distinguish trailing whitespace from a same-line
        // comment. What this test is actually checking is that a `/* */` comment round-trips as
        // `/* */`, not `//`.
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
        let source = "sheet s {\n    out area: f64 := w require {\n        c: w <= 10.0;\n        // trailing\n    };\n}";
        let expected = "sheet s {\n    out area: f64 := w require {\n        c: w <= 10.0;\n        // trailing\n    };\n}\n";
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
}
