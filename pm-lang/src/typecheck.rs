//! A best-effort static type checker over [`crate::ast::Sheet`] trees, built on
//! [`cel_parser::ty::check_expr`]. Checks each `cell`'s literal initializer against its `:
//! type_name` annotation, and each `relationship`/`conditional` method's body against its declared
//! outputs (arity: does the body actually produce as many values as declared; and per-output
//! type). An absent annotation, an annotation naming a type [`crate::TypeRegistry`] doesn't
//! recognize, or an operator [`cel_parser::op_table::builtin_operand_types`] doesn't recognize all
//! resolve to [`cel_parser::Ty::Any`] and are never flagged — matching pm-lang/CEL's extensible
//! type system. Not a complete type system; see the design doc's "Type checking (v1)" section.

use cel_parser::lex_lexer::Literal as LexLiteral;
use cel_parser::{Expr, ParseError, Ty, ty::check_expr};

use crate::TypeRegistry;
use crate::ast::{CellDecl, MethodDecl, Sheet, SheetItem};

/// Checks `sheet` against `registry`'s registered types, returning every type diagnostic found.
/// Never fails — an unrecognized annotation, an unresolved identifier, or a custom operator
/// [`cel_parser::op_table::builtin_operand_types`] doesn't know about all resolve to
/// [`cel_parser::Ty::Any`] and are silently skipped, not reported.
///
/// - Complexity: O(n) in the number of nodes across every item in `sheet`.
///
/// # Examples
///
/// ```rust
/// use pm_lang::{PmAstParser, TypeRegistry, check_sheet};
///
/// let sheet = PmAstParser::new()
///     .parse_str("sheet s { cell x: i32 = 1.0; }")
///     .unwrap();
/// let diagnostics = check_sheet(&sheet, &TypeRegistry::new());
/// assert_eq!(diagnostics.len(), 1, "1.0 defaults to f64, mismatching the i32 annotation");
/// ```
pub fn check_sheet(sheet: &Sheet, registry: &TypeRegistry) -> Vec<ParseError> {
    let mut diagnostics = Vec::new();
    let cell_types = declared_cell_types(sheet, registry);
    let resolve = |name: &str| -> Ty { cell_types.get(name).copied().unwrap_or(Ty::Any) };
    for item in &sheet.items {
        match item {
            SheetItem::Cell(cell) => check_cell_initializer(cell, registry, &mut diagnostics),
            SheetItem::Relationship(rel) => {
                for method in &rel.methods {
                    check_method(method, &resolve, &mut diagnostics);
                }
            }
            SheetItem::Conditional(cond) => {
                for branch in &cond.branches {
                    for method in &branch.methods {
                        check_method(method, &resolve, &mut diagnostics);
                    }
                }
                if let Some(default_methods) = &cond.default {
                    for method in default_methods {
                        check_method(method, &resolve, &mut diagnostics);
                    }
                }
            }
            SheetItem::Error { .. } => {} // already reported as a syntax error; nothing to type-check
        }
    }
    diagnostics
}

/// Maps every declared cell name to its `Ty` (from its `: type_name` annotation, resolved through
/// `registry`), for use as the identifier resolver method bodies are checked against. A cell with
/// no annotation, or one naming a type `registry` doesn't recognize, maps to `Ty::Any`.
fn declared_cell_types(
    sheet: &Sheet,
    registry: &TypeRegistry,
) -> std::collections::HashMap<String, Ty> {
    let mut map = std::collections::HashMap::new();
    for item in &sheet.items {
        if let SheetItem::Cell(cell) = item {
            let ty = cell
                .type_name
                .as_ref()
                .and_then(|(name, _)| registry.get(name))
                .map(|entry| Ty::from_type_id(entry.type_id))
                .unwrap_or(Ty::Any);
            map.insert(cell.name.clone(), ty);
        }
    }
    map
}

/// Checks whether `lit` is compatible with `declared`, mirroring `pm_lang::parser`'s
/// `parse_literal_as` — the function pm-lang's real `cell_decl` grammar actually uses once a cell
/// has a `: type_name` annotation. `parse_literal_as` parses the literal's digits/value directly
/// against the declared type, ignoring any suffix on the literal itself (unlike
/// `infer_and_parse_literal`, used only when no annotation is present, which defaults an
/// unsuffixed integer to `i32` and an unsuffixed float to `f64`) — so an unsuffixed integer
/// literal is valid for *any* declared numeric type (`parse_literal_as` accepts it via
/// `parse_int_literal`, which covers every integer width and both float types), and an unsuffixed
/// float literal is valid only for `f32`/`f64`. `declared == Ty::Any` (an unregistered custom
/// type) always matches — not statically checked.
fn literal_matches_declared_ty(lit: &LexLiteral, declared: Ty) -> bool {
    use syn::Lit;
    if declared == Ty::Any {
        return true;
    }
    match lit {
        Lit::Int(_) => matches!(
            declared,
            Ty::I8
                | Ty::I16
                | Ty::I32
                | Ty::I64
                | Ty::I128
                | Ty::Isize
                | Ty::U8
                | Ty::U16
                | Ty::U32
                | Ty::U64
                | Ty::U128
                | Ty::Usize
                | Ty::F32
                | Ty::F64
        ),
        Lit::Float(_) => matches!(declared, Ty::F32 | Ty::F64),
        Lit::Bool(_) => declared == Ty::Bool,
        Lit::Str(_) => declared == Ty::String,
        // char/byte-string/C-string: parse_literal_as has no arm for these against any
        // registered type, so pm-lang's runtime rejects them unconditionally.
        _ => false,
    }
}

/// Checks one `cell`'s literal initializer against its `: type_name` annotation. A no-op if either
/// half is absent, or if the annotation names a type `registry` doesn't recognize.
fn check_cell_initializer(
    cell: &CellDecl,
    registry: &TypeRegistry,
    diagnostics: &mut Vec<ParseError>,
) {
    let (Some((type_name, _)), Some((literal, lit_span))) = (&cell.type_name, &cell.initializer)
    else {
        return;
    };
    let Some(entry) = registry.get(type_name) else {
        return;
    };
    let declared = Ty::from_type_id(entry.type_id);
    if !literal_matches_declared_ty(literal, declared) {
        diagnostics.push(ParseError::new_range(
            format!("literal cannot be used as type `{}`", declared.name()),
            lit_span.start,
            lit_span.end,
        ));
    }
}

/// Checks one `method`'s body against its declared outputs: for a single output, the body's
/// inferred type must unify with that output cell's declared type; for `n > 1` outputs, the body
/// must be an `n`-element tuple, checked element-wise against each output cell. Operator-level
/// diagnostics from inside the body (via [`check_expr`]) are always included exactly once,
/// regardless of which branch below runs.
fn check_method(
    method: &MethodDecl,
    resolve: &impl Fn(&str) -> Ty,
    diagnostics: &mut Vec<ParseError>,
) {
    match method.outputs.as_slice() {
        [] => {
            let (_, body_diags) = check_expr(&method.body, resolve);
            diagnostics.extend(body_diags);
        }
        [(name, _)] => {
            let (body_ty, body_diags) = check_expr(&method.body, resolve);
            diagnostics.extend(body_diags);
            if let Expr::Tuple { elements, .. } = &method.body {
                let n = elements.len();
                diagnostics.push(ParseError::new_range(
                    format!("method declares 1 output but its body is a {n}-tuple"),
                    method.body.span().start,
                    method.body.span().end,
                ));
                return;
            }
            let declared = resolve(name);
            if !declared.unifies_with(&body_ty) {
                diagnostics.push(ParseError::new_range(
                    format!(
                        "method body produces `{}`, but `{name}` is declared `{}`",
                        body_ty.name(),
                        declared.name()
                    ),
                    method.body.span().start,
                    method.body.span().end,
                ));
            }
        }
        outputs => {
            let n = outputs.len();
            match &method.body {
                Expr::Tuple { elements, .. } if elements.len() == n => {
                    for (element, (name, _)) in elements.iter().zip(outputs) {
                        let (element_ty, element_diags) = check_expr(element, resolve);
                        diagnostics.extend(element_diags);
                        let declared = resolve(name);
                        if !declared.unifies_with(&element_ty) {
                            diagnostics.push(ParseError::new_range(
                                format!(
                                    "method output `{name}` produces `{}`, but is declared `{}`",
                                    element_ty.name(),
                                    declared.name()
                                ),
                                element.span().start,
                                element.span().end,
                            ));
                        }
                    }
                }
                other => {
                    let (_, body_diags) = check_expr(other, resolve);
                    diagnostics.extend(body_diags);
                    diagnostics.push(ParseError::new_range(
                        format!("method declares {n} outputs but its body is not a {n}-tuple"),
                        other.span().start,
                        other.span().end,
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PmAstParser;

    fn parse(source: &str) -> Sheet {
        PmAstParser::new().parse_str(source).unwrap()
    }

    #[test]
    fn cell_initializer_matching_its_annotation_has_no_diagnostic() {
        let sheet = parse("sheet s { cell x: i32 = 1; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn cell_initializer_mismatched_with_its_annotation_is_a_diagnostic() {
        // Unsuffixed float literal defaults to f64, not i32.
        let sheet = parse("sheet s { cell x: i32 = 1.0; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn cell_with_only_an_annotation_has_nothing_to_cross_check() {
        let sheet = parse("sheet s { cell x: i32; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn cell_initializer_unsuffixed_int_literal_matches_a_declared_unsigned_type() {
        // pm_lang::parser's real cell_decl grammar parses an annotated initializer via
        // parse_literal_as(entry, lit, span) — it parses the literal's digits directly as the
        // declared type, ignoring the literal's own (absent) suffix. `cell x: u32 = 1;` is valid,
        // accepted pm-lang; the checker must not falsely flag it.
        let sheet = parse("sheet s { cell x: u32 = 1; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn cell_initializer_char_literal_against_any_registered_type_is_a_diagnostic() {
        // parse_literal_as has no arm for a char literal against any registered type — pm-lang's
        // runtime rejects `cell x: i32 = 'a';` unconditionally, so the checker must too (same root
        // cause as the unsuffixed-int case above: the check must consult the declared type, not
        // infer the literal's type independently).
        let sheet = parse("sheet s { cell x: i32 = 'a'; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn cell_annotated_with_an_unregistered_type_name_is_never_flagged() {
        let sheet = parse("sheet s { cell x: WidgetHandle = 1; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn method_single_output_matching_declared_type_has_no_diagnostic() {
        let sheet = parse(
            "sheet s { cell width: f64; cell height: f64; cell area: f64; \
             relationship { method [width, height] -> [area] { width * height } } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn method_single_output_mismatched_with_declared_type_is_a_diagnostic() {
        let sheet = parse(
            "sheet s { cell width: f64; cell height: f64; cell area: i32; \
             relationship { method [width, height] -> [area] { width * height } } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn method_multi_output_matching_tuple_has_no_diagnostic() {
        let sheet = parse(
            "sheet s { cell a: i32; cell b: i32; cell sum: i32; cell diff: i32; \
             relationship { method [a, b] -> [sum, diff] { (a + b, a - b) } } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn method_multi_output_arity_mismatch_is_a_diagnostic() {
        let sheet = parse(
            "sheet s { cell a: i32; cell b: i32; cell sum: i32; cell diff: i32; \
             relationship { method [a, b] -> [sum, diff] { a + b } } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn method_single_output_with_a_tuple_shaped_body_is_a_diagnostic() {
        // Body is a 2-tuple but only 1 output is declared: `check_expr` would otherwise infer
        // `Ty::Any` for the tuple and let this slip through with no diagnostic at all.
        let sheet = parse(
            "sheet s { cell a: i32; cell b: i32; cell out: i32; \
             relationship { method [a, b] -> [out] { (a, b) } } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn method_multi_output_per_element_type_mismatch_is_a_diagnostic() {
        let sheet = parse(
            "sheet s { cell a: i32; cell b: i32; cell sum: i32; cell diff: f64; \
             relationship { method [a, b] -> [sum, diff] { (a + b, a - b) } } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn an_operator_error_inside_a_method_body_surfaces() {
        let sheet = parse(
            "sheet s { cell name: String; cell count: i32; cell out: i32; \
             relationship { method [name, count] -> [out] { name + count } } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn conditional_branch_and_default_methods_are_both_checked() {
        let sheet = parse(
            "sheet s { cell mode: i32; cell a: i32; cell b: i32; cell out: i32; \
             conditional mode { \
                 0i32 => { method [a] -> [out] { a } }, \
                 _ => { method [b] -> [out] { b } }, \
             } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn a_cell_with_no_type_annotation_unifies_with_anything_used_in_a_method() {
        // `cell a = 1;` has an initializer but no `: type_name` — declared_cell_types maps it to
        // Ty::Any, which must unify silently with `out`'s declared `i32`.
        let sheet = parse(
            "sheet s { cell a = 1; cell out: i32; \
             relationship { method [a] -> [out] { a } } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn recovered_error_items_are_skipped_without_panicking() {
        let sheet =
            parse("sheet s { cell good: i32 = 1; cell bad unknown_syntax cell after: i32 = 2; }");
        assert!(
            !sheet.errors.is_empty(),
            "fixture must actually recover an error item"
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }
}
