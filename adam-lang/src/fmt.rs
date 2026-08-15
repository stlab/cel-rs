//! Pretty-prints an [`crate::ast::Sheet`] back to adam-lang source text: 4-space indentation,
//! opening braces on the same line, `leading_comment`/`blank_line_before` reproduced exactly as
//! [`crate::trivia::attach_trivia`] recovered them (including a file-header-style comment
//! preceding the `sheet` keyword itself, `Sheet.leading_comment`), and method bodies/cell
//! initializers/condition bodies delegated to [`cel_parser::format_expr`] (a normalization
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

/// Emits `blank_line_before`/`leading_comment` ahead of an item, if either is present.
fn write_trivia(
    out: &mut String,
    blank_line_before: bool,
    leading_comment: Option<&str>,
    depth: usize,
) {
    if blank_line_before {
        out.push('\n');
    }
    if let Some(comment) = leading_comment {
        for line in comment.split('\n') {
            out.push_str(&indent(depth));
            out.push_str("// ");
            out.push_str(line);
            out.push('\n');
        }
    }
}

/// Re-emits a literal's exact original text via its span, falling back to an empty string when
/// none is recoverable — mirrors `cel_parser::fmt`'s identical fallback (see the module doc for
/// why no `Literal` value is needed here).
fn source_text_or_empty(span: ast::ExprSpan) -> String {
    span.start.source_text().unwrap_or_default()
}

/// Writes a bracketed, comma-separated cell-name list (e.g. `[a, b]`).
fn write_cell_list(out: &mut String, cells: &[(String, ast::ExprSpan)]) {
    out.push('[');
    for (i, (name, _)) in cells.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(name);
    }
    out.push(']');
}

/// Writes one `method [...] -> [...] { ... }` declaration, delegating its body to
/// [`cel_parser::format_expr`].
fn write_method(out: &mut String, method: &ast::MethodDecl, depth: usize) {
    write_trivia(
        out,
        method.blank_line_before,
        method.leading_comment.as_deref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str("method ");
    write_cell_list(out, &method.inputs);
    out.push_str(" -> ");
    write_cell_list(out, &method.outputs);
    out.push_str(" { ");
    out.push_str(&cel_parser::format_expr(&method.body));
    out.push_str(" }\n");
}

/// Writes one `relationship [name] { ... }` declaration and its methods, in declaration order.
fn write_relationship(out: &mut String, rel: &ast::RelationshipDecl, depth: usize) {
    write_trivia(
        out,
        rel.blank_line_before,
        rel.leading_comment.as_deref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str("relationship ");
    if let Some((name, _)) = &rel.name {
        out.push_str(name);
        out.push(' ');
    }
    out.push_str("{\n");
    for method in &rel.methods {
        write_method(out, method, depth + 1);
    }
    out.push_str(&indent(depth));
    out.push_str("}\n");
}

/// Writes a `{ ... }` block of relationships, shared by both a named conditional branch and the
/// default (`_ =>`) arm.
fn write_branch_relationships(
    out: &mut String,
    relationships: &[ast::RelationshipDecl],
    depth: usize,
) {
    out.push_str("{\n");
    for rel in relationships {
        write_relationship(out, rel, depth + 1);
    }
    out.push_str(&indent(depth));
    out.push_str("}\n");
}

/// Writes one `literal => { ... }` conditional branch, re-emitting the match literal via its
/// span rather than the (unused) `Literal` value.
fn write_branch(out: &mut String, branch: &ast::ConditionalBranch, depth: usize) {
    write_trivia(
        out,
        branch.blank_line_before,
        branch.leading_comment.as_deref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str(&source_text_or_empty(branch.literal_span));
    out.push_str(" => ");
    write_branch_relationships(out, &branch.relationships, depth);
}

/// Writes one `conditional <expr> { ... }` declaration: its branches in declaration
/// order (dispatching on the match-subject expression), followed by its optional `_ => { ... }` default arm.
fn write_conditional(out: &mut String, cond: &ast::ConditionalDecl, depth: usize) {
    write_trivia(
        out,
        cond.blank_line_before,
        cond.leading_comment.as_deref(),
        depth,
    );
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
        write_branch_relationships(out, default, depth + 1);
    }
    out.push_str(&indent(depth));
    out.push_str("}\n");
}

/// Writes one `cell name[: type][ = initializer];` declaration, delegating its type annotation
/// to [`source_text_or_empty`] via `TypeExpr::span()` and its initializer to
/// [`cel_parser::format_expr`].
fn write_cell(out: &mut String, cell: &ast::CellDecl, depth: usize) {
    write_trivia(
        out,
        cell.blank_line_before,
        cell.leading_comment.as_deref(),
        depth,
    );
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
    out.push_str(";\n");
}

/// Writes one `method [...] { ... }` writer declaration inside an `out` block — like
/// `write_method`, but with no `-> [...]` half: an out cell's writer always writes exactly the
/// enclosing declaration's cell, so naming it again would be redundant.
fn write_out_method(out: &mut String, method: &ast::OutMethodDecl, depth: usize) {
    write_trivia(
        out,
        method.blank_line_before,
        method.leading_comment.as_deref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str("method ");
    write_cell_list(out, &method.inputs);
    out.push_str(" { ");
    out.push_str(&cel_parser::format_expr(&method.body));
    out.push_str(" }\n");
}

/// Writes one `condition name [...] { ... }` declaration.
fn write_condition(out: &mut String, cond: &ast::ConditionDecl, depth: usize) {
    write_trivia(
        out,
        cond.blank_line_before,
        cond.leading_comment.as_deref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str("condition ");
    out.push_str(&cond.name);
    out.push(' ');
    write_cell_list(out, &cond.inputs);
    out.push_str(" { ");
    out.push_str(&cel_parser::format_expr(&cond.body));
    out.push_str(" }\n");
}

/// Writes one `out name[: type] { ... }` declaration: its writer method followed by its
/// conditions, in declaration order.
fn write_out(out: &mut String, decl: &ast::OutDecl, depth: usize) {
    write_trivia(
        out,
        decl.blank_line_before,
        decl.leading_comment.as_deref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str("out ");
    out.push_str(&decl.name);
    if let Some(type_expr) = &decl.type_name {
        out.push_str(": ");
        out.push_str(&source_text_or_empty(type_expr.span()));
    }
    out.push_str(" {\n");
    write_out_method(out, &decl.writer, depth + 1);
    for cond in &decl.conditions {
        write_condition(out, cond, depth + 1);
    }
    out.push_str(&indent(depth));
    out.push_str("}\n");
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
    write_trivia(&mut out, false, sheet.leading_comment.as_deref(), 0);
    out.push_str(&format!("sheet {} {{\n", sheet.name));
    for item in &sheet.items {
        write_sheet_item(&mut out, item, 1);
    }
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
        let source = "sheet s {\n    cell a: i32 = 1;\n    cell b: i32 = 2;\n\n    relationship { method [a] -> [b] { a } }\n}";
        let expected = "sheet s {\n    cell a: i32 = 1;\n    cell b: i32 = 2;\n\n    relationship {\n        method [a] -> [b] { a }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn a_run_of_blank_lines_collapses_to_one() {
        let source = "sheet s {\n    cell a: i32 = 1;\n\n\n\n    cell b: i32 = 2;\n}";
        let expected = "sheet s {\n    cell a: i32 = 1;\n\n    cell b: i32 = 2;\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_named_relationship_with_multiple_methods() {
        let source = "sheet s {\n    relationship r {\n        method [width, height] -> [area] { width * height }\n        method [area, height] -> [width] { area / height }\n    }\n}";
        let expected = "sheet s {\n    relationship r {\n        method [width, height] -> [area] { width * height }\n        method [area, height] -> [width] { area / height }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn preserves_a_comment_on_a_nested_method() {
        let source = "sheet s {\n    relationship {\n        method [a] -> [b] { a }\n\n        // second\n        method [b] -> [a] { b }\n    }\n}";
        let expected = "sheet s {\n    relationship {\n        method [a] -> [b] { a }\n\n        // second\n        method [b] -> [a] { b }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_conditional_with_branches_and_a_default_and_no_trailing_commas() {
        let source = "sheet s {\n    conditional p {\n        0i32 => { relationship { method [a] -> [b] { a } } },\n        _ => { relationship { method [b] -> [a] { b } } },\n    }\n}";
        let expected = "sheet s {\n    conditional p {\n        0i32 => {\n            relationship {\n                method [a] -> [b] { a }\n            }\n        }\n        _ => {\n            relationship {\n                method [b] -> [a] { b }\n            }\n        }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_conditional_with_an_expression_match_subject() {
        let source = "sheet s {\n    conditional a && b {\n        _ => { relationship { method [c] -> [d] { c } } },\n    }\n}";
        let expected = "sheet s {\n    conditional a && b {\n        _ => {\n            relationship {\n                method [c] -> [d] { c }\n            }\n        }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn preserves_a_comment_on_a_conditional_branch() {
        let source = "sheet s {\n    conditional m {\n        0i32 => { relationship { method [a] -> [b] { a } } }\n        // one\n        1i32 => { relationship { method [a] -> [b] { a } } }\n    }\n}";
        let expected = "sheet s {\n    conditional m {\n        0i32 => {\n            relationship {\n                method [a] -> [b] { a }\n            }\n        }\n        // one\n        1i32 => {\n            relationship {\n                method [a] -> [b] { a }\n            }\n        }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn method_body_delegates_precedence_aware_parenthesization_to_cel_parser() {
        let source = "sheet s { relationship { method [a, b] -> [c] { (a + b) * 2i32 } } }";
        let expected = "sheet s {\n    relationship {\n        method [a, b] -> [c] { (a + b) * 2i32 }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn format_is_idempotent_through_a_reparse() {
        let source = "sheet demo {\n    cell a: f64 = 2.0;\n    cell b: f64 = 3.0;\n\n    relationship {\n        method [a, b] -> [c] { a * b }\n    }\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn format_is_idempotent_through_a_reparse_with_a_conditional() {
        let source = "sheet demo {\n    cell p: i32 = 0;\n    cell c: f64;\n\n    conditional p {\n        0i32 => {\n            relationship {\n                method [c] -> [c] { c }\n            }\n        }\n        _ => {\n            relationship {\n                method [c] -> [c] { c }\n            }\n        }\n    }\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn formats_an_out_with_explicit_type_and_no_conditions() {
        let source = "sheet s {\n    out area: f64 {\n        method [width, height] { width * height }\n    }\n}";
        let expected = "sheet s {\n    out area: f64 {\n        method [width, height] { width * height }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_an_out_with_no_type_annotation() {
        let source = "sheet s {\n    out area {\n        method [width] { width }\n    }\n}";
        let expected = "sheet s {\n    out area {\n        method [width] { width }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_an_out_with_conditions_in_declaration_order() {
        let source = "sheet s {\n    out area: f64 {\n        method [width, height] { width * height }\n        condition max_area [width, height, max_area] { width * height <= max_area }\n    }\n}";
        let expected = "sheet s {\n    out area: f64 {\n        method [width, height] { width * height }\n        condition max_area [width, height, max_area] { width * height <= max_area }\n    }\n}\n";
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
            format("sheet s { out a: (i32, i32) { method [x] { (x, x) } } }"),
            "sheet s {\n    out a: (i32, i32) {\n        method [x] { (x, x) }\n    }\n}\n"
        );
    }

    #[test]
    fn format_is_idempotent_through_a_reparse_with_a_tuple_cell() {
        let source = "sheet s {\n    cell a: (i32, f64) = (1, 2.5);\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }
}
