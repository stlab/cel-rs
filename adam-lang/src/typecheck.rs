//! A best-effort static type checker over [`crate::ast::Sheet`] trees, built on
//! [`cel_parser::ty::check_expr`]. Checks each `cell`'s literal initializer against its `:
//! type_name` annotation, each `relationship`/`conditional` method's body against its declared
//! outputs (arity: does the body actually produce as many values as declared; and per-output
//! type), and each `out`'s writer body against its optional `: type_name` annotation, with each
//! `condition` body checked to produce `bool` type. An absent annotation, an annotation naming a
//! type [`crate::TypeRegistry`] doesn't recognize, or an operator [`cel_parser::op_table::builtin_operand_types`]
//! doesn't recognize all resolve to [`cel_parser::Ty::Any`] and are never flagged — matching
//! adam-lang/CEL's extensible type system. Not a complete type system; see the design doc's
//! "Type checking (v1)" section.

use cel_parser::{Expr, Literal, ParseError, Ty, ty::check_expr};

use crate::TypeRegistry;
use crate::ast::{CellDecl, MethodDecl, OutDecl, Sheet, SheetItem};
use crate::type_registry::TypeShape;

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
/// use adam_lang::{AdamAstParser, TypeRegistry, check_sheet};
///
/// let sheet = AdamAstParser::new()
///     .parse_str("sheet s { cell x: i32 = 1.0; }")
///     .unwrap();
/// let diagnostics = check_sheet(&sheet, &TypeRegistry::new());
/// assert_eq!(diagnostics.len(), 1, "1.0 defaults to f64, mismatching the i32 annotation");
/// ```
pub fn check_sheet(sheet: &Sheet, registry: &TypeRegistry) -> Vec<ParseError> {
    let mut diagnostics = Vec::new();
    let (cell_types, shapes) = declared_cell_types(sheet, registry);
    let resolve = |name: &str| -> Ty { cell_types.get(name).copied().unwrap_or(Ty::Any) };
    for item in &sheet.items {
        match item {
            SheetItem::Cell(cell) => check_cell_initializer(cell, registry, &mut diagnostics),
            SheetItem::Relationship(rel) => {
                for method in &rel.methods {
                    check_method(method, registry, &shapes, &resolve, &mut diagnostics);
                }
            }
            SheetItem::Conditional(cond) => {
                for branch in &cond.branches {
                    for rel in &branch.relationships {
                        for method in &rel.methods {
                            check_method(method, registry, &shapes, &resolve, &mut diagnostics);
                        }
                    }
                }
                if let Some(default_rels) = &cond.default {
                    for rel in default_rels {
                        for method in &rel.methods {
                            check_method(method, registry, &shapes, &resolve, &mut diagnostics);
                        }
                    }
                }
            }
            SheetItem::Out(out_decl) => {
                check_out(out_decl, registry, &shapes, &resolve, &mut diagnostics)
            }
            SheetItem::Error { .. } => {} // already reported as a syntax error; nothing to type-check
        }
    }
    diagnostics
}

/// Maps every declared cell name — from a `cell` or an `out` — to both its scalar `Ty` (unaware
/// of tuple structure, for use as the identifier resolver method/condition bodies are checked
/// against) and its full recursive `TypeShape` (for `expr_matches_shape`'s tuple-aware checks). A
/// `cell`/`out` with no annotation, or one naming a type `registry` doesn't resolve, is absent
/// from the `TypeShape` map and maps to `Ty::Any` in the `Ty` map; a tuple-typed annotation also
/// maps to `Ty::Any` in the `Ty` map (`Ty` has no tuple variant), but *is* present in the
/// `TypeShape` map. An `out` with an annotation resolves the same way as a `cell`; one without is
/// inferred from its writer body's checked type, using only `cell`-declared types as context (not
/// other `out`s' inferred types — see this function's own note above), and is never present in
/// the `TypeShape` map (only annotated cells/outs are).
fn declared_cell_types(
    sheet: &Sheet,
    registry: &TypeRegistry,
) -> (
    std::collections::HashMap<String, Ty>,
    std::collections::HashMap<String, TypeShape>,
) {
    /// Resolves `type_expr` against `registry`, if present and recognized; `None` covers both an
    /// absent annotation and one naming a type `registry` doesn't recognize — the two cases
    /// `declared_cell_types`'s callers already treat identically (fall back to `Ty::Any`, or, for
    /// an `out`, infer from its writer body).
    fn resolve_annotation_shape(
        type_expr: Option<&crate::ast::TypeExpr>,
        registry: &TypeRegistry,
    ) -> Option<TypeShape> {
        type_expr.and_then(|type_expr| registry.resolve(type_expr).ok())
    }

    /// Converts a resolved `TypeShape` to its scalar `Ty` approximation: `Ty` has no tuple
    /// variant, so a `TypeShape::Tuple` always maps to `Ty::Any`.
    fn shape_to_ty(shape: &TypeShape) -> Ty {
        match shape {
            TypeShape::Named(type_id) => Ty::from_type_id(*type_id),
            TypeShape::Tuple(_) => Ty::Any,
        }
    }

    let mut map = std::collections::HashMap::new();
    let mut shapes = std::collections::HashMap::new();
    for item in &sheet.items {
        if let SheetItem::Cell(cell) = item {
            let shape = resolve_annotation_shape(cell.type_name.as_ref(), registry);
            let ty = shape.as_ref().map(shape_to_ty).unwrap_or(Ty::Any);
            if let Some(shape) = shape {
                shapes.insert(cell.name.clone(), shape);
            }
            map.insert(cell.name.clone(), ty);
        }
    }
    let resolve_cells = |name: &str| -> Ty { map.get(name).copied().unwrap_or(Ty::Any) };
    let mut out_types = std::collections::HashMap::new();
    for item in &sheet.items {
        if let SheetItem::Out(out_decl) = item {
            let shape = resolve_annotation_shape(out_decl.type_name.as_ref(), registry);
            let ty = shape
                .as_ref()
                .map(shape_to_ty)
                .unwrap_or_else(|| check_expr(&out_decl.writer.body, &resolve_cells).0);
            if let Some(shape) = shape {
                shapes.insert(out_decl.name.clone(), shape);
            }
            out_types.insert(out_decl.name.clone(), ty);
        }
    }
    map.extend(out_types);
    (map, shapes)
}

/// Checks whether `lit` is compatible with `declared`, mirroring `adam_lang::parser`'s
/// `parse_literal_as` — the function adam-lang's real `cell_decl` grammar actually uses once a cell
/// has a `: type_name` annotation. `parse_literal_as` parses the literal's digits/value directly
/// against the declared type, ignoring any suffix on the literal itself (unlike
/// `infer_and_parse_literal`, used only when no annotation is present, which defaults an
/// unsuffixed integer to `i32` and an unsuffixed float to `f64`) — so any integer-typed literal
/// (`lit`'s own suffix, or lack of one already resolved to `i32` by `AstContext`, doesn't
/// matter — every integer-width variant is treated as one undifferentiated "integer literal"
/// category, exactly as `parse_literal_as`'s suffix-ignoring behavior implies) is valid for *any*
/// declared numeric type (`parse_literal_as` accepts it via `parse_int_literal`, which covers
/// every integer width and both float types), and a float-typed literal is valid only for
/// `f32`/`f64`. `declared == Ty::Any` (an unregistered custom type) always matches — not
/// statically checked.
fn literal_matches_declared_ty(lit: &Literal, declared: Ty) -> bool {
    if declared == Ty::Any {
        return true;
    }
    match lit {
        Literal::I8(_)
        | Literal::I16(_)
        | Literal::I32(_)
        | Literal::I64(_)
        | Literal::I128(_)
        | Literal::Isize(_)
        | Literal::U8(_)
        | Literal::U16(_)
        | Literal::U32(_)
        | Literal::U64(_)
        | Literal::U128(_)
        | Literal::Usize(_) => matches!(
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
        Literal::F32(_) | Literal::F64(_) => matches!(declared, Ty::F32 | Ty::F64),
        Literal::Bool(_) => declared == Ty::Bool,
        Literal::Str(_) => declared == Ty::String,
        // char/byte-string/C-string/unit: parse_literal_as has no arm for these against any
        // registered type, so adam-lang's runtime rejects them unconditionally.
        _ => false,
    }
}

/// Checks whether `expr` structurally matches `shape`, recursively: a `TypeShape::Named` leaf
/// must be a non-tuple `Expr` whose checked `Ty` unifies with that leaf (mirroring
/// `literal_matches_declared_ty`'s spirit, generalized past bare literals now that initializers
/// are full `or_expression`s); a `TypeShape::Tuple` must be an `Expr::Tuple` of matching arity,
/// checked element-wise. `TypeShape::Named(TypeId)` with no registered entry (an unrecognized
/// custom type) always matches — not statically checked, mirroring `Ty::Any`'s existing
/// leniency.
///
/// - Complexity: O(n) in the number of (nested) tuple elements.
fn expr_matches_shape(
    expr: &Expr,
    shape: &TypeShape,
    registry: &TypeRegistry,
    resolve: &impl Fn(&str) -> Ty,
    diagnostics: &mut Vec<ParseError>,
) {
    match (expr, shape) {
        (Expr::Tuple { elements, .. }, TypeShape::Tuple(expected)) => {
            if elements.len() != expected.len() {
                diagnostics.push(ParseError::new_range(
                    format!(
                        "expected a {}-element tuple `{}`, got {}",
                        expected.len(),
                        registry.display_name(shape),
                        elements.len()
                    ),
                    expr.span().start,
                    expr.span().end,
                ));
                return;
            }
            for (element, element_shape) in elements.iter().zip(expected) {
                expr_matches_shape(element, element_shape, registry, resolve, diagnostics);
            }
        }
        (_, TypeShape::Tuple(_)) => {
            diagnostics.push(ParseError::new_range(
                format!("expected tuple `{}`", registry.display_name(shape)),
                expr.span().start,
                expr.span().end,
            ));
        }
        (Expr::Tuple { .. }, TypeShape::Named(_)) => {
            diagnostics.push(ParseError::new_range(
                format!("expected `{}`, got a tuple", registry.display_name(shape)),
                expr.span().start,
                expr.span().end,
            ));
        }
        (_, TypeShape::Named(type_id)) => {
            let Some(entry) = registry.entry_by_type_id(*type_id) else {
                return; // unrecognized custom type: never statically checked, matches Ty::Any
            };
            let declared = Ty::from_type_id(entry.type_id);
            let (actual, body_diags) = check_expr(expr, resolve);
            diagnostics.extend(body_diags);
            if !declared.unifies_with(&actual) {
                diagnostics.push(ParseError::new_range(
                    format!(
                        "expression produces `{}`, but `{}` was expected",
                        actual.name(),
                        declared.name()
                    ),
                    expr.span().start,
                    expr.span().end,
                ));
            }
        }
    }
}

/// Checks one `cell`'s initializer against its `: type_expr` annotation. A no-op if either half
/// is absent, or if the annotation names a type `registry` doesn't recognize. Dispatches to
/// [`expr_matches_shape`] for a tuple-shaped annotation (recursively, element-wise); otherwise
/// falls back to the original literal/scalar check, since a non-tuple initializer that isn't a
/// bare literal fails to constant-fold in the real parser anyway.
fn check_cell_initializer(
    cell: &CellDecl,
    registry: &TypeRegistry,
    diagnostics: &mut Vec<ParseError>,
) {
    let (Some(type_expr), Some(expr)) = (&cell.type_name, &cell.initializer) else {
        return;
    };
    let Ok(shape) = registry.resolve(type_expr) else {
        return; // unknown type name: already reported by the real parser's own error path
    };
    if let TypeShape::Tuple(_) = shape {
        let resolve = |_: &str| Ty::Any; // initializers reference no cells
        expr_matches_shape(expr, &shape, registry, &resolve, diagnostics);
        return;
    }
    // Scalar case: unchanged from before, still literal-shaped in practice (an initializer that
    // isn't a bare literal fails to constant-fold in the real parser; this checker only needs to
    // flag a literal/type mismatch, exactly as it always has).
    let Expr::Literal {
        value: literal,
        span: lit_span,
    } = expr
    else {
        return;
    };
    let declared = Ty::from_type_id(
        match registry.entry_by_type_id(match shape {
            TypeShape::Named(tid) => tid,
            TypeShape::Tuple(_) => unreachable!("handled above"),
        }) {
            Some(entry) => entry.type_id,
            None => return,
        },
    );
    if !literal_matches_declared_ty(literal, declared) {
        diagnostics.push(ParseError::new_range(
            format!("literal cannot be used as type `{}`", declared.name()),
            lit_span.start,
            lit_span.end,
        ));
    }
}

/// Checks one `method`'s body against its declared outputs: for a single output, the body's
/// inferred type must unify with that output cell's declared type (or, when that output's
/// declared type is itself a tuple — per `shapes` — the body is checked recursively via
/// [`expr_matches_shape`] instead); for `n > 1` outputs, the body must be an `n`-element tuple,
/// checked element-wise against each output cell. Operator-level diagnostics from inside the body
/// (via [`check_expr`]) are always included exactly once, regardless of which branch below runs.
fn check_method(
    method: &MethodDecl,
    registry: &TypeRegistry,
    shapes: &std::collections::HashMap<String, TypeShape>,
    resolve: &impl Fn(&str) -> Ty,
    diagnostics: &mut Vec<ParseError>,
) {
    match method.outputs.as_slice() {
        [] => {
            let (_, body_diags) = check_expr(&method.body, resolve);
            diagnostics.extend(body_diags);
        }
        [(name, _)] => {
            if let Some(shape @ TypeShape::Tuple(_)) = shapes.get(name) {
                expr_matches_shape(&method.body, shape, registry, resolve, diagnostics);
                return;
            }
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

/// Checks one `out`'s writer body against its optional `: type_expr` annotation — mirroring
/// `check_method`'s single-output branch, since an out's writer is structurally a `method`
/// with one implicit output (the out cell itself), including the same tuple-shaped dispatch to
/// [`expr_matches_shape`] via `shapes` — and each of its conditions' bodies against `Ty::Bool`.
/// Operator-level diagnostics from inside any body (via `check_expr`) are always included,
/// regardless of whether a mismatch diagnostic is also added.
fn check_out(
    out_decl: &OutDecl,
    registry: &TypeRegistry,
    shapes: &std::collections::HashMap<String, TypeShape>,
    resolve: &impl Fn(&str) -> Ty,
    diagnostics: &mut Vec<ParseError>,
) {
    if let Some(shape @ TypeShape::Tuple(_)) = shapes.get(&out_decl.name) {
        expr_matches_shape(&out_decl.writer.body, shape, registry, resolve, diagnostics);
    } else {
        let (body_ty, body_diags) = check_expr(&out_decl.writer.body, resolve);
        diagnostics.extend(body_diags);
        if out_decl.type_name.is_some() {
            let declared = resolve(&out_decl.name);
            if !declared.unifies_with(&body_ty) {
                diagnostics.push(ParseError::new_range(
                    format!(
                        "out `{}` body produces `{}`, but is declared `{}`",
                        out_decl.name,
                        body_ty.name(),
                        declared.name()
                    ),
                    out_decl.writer.body.span().start,
                    out_decl.writer.body.span().end,
                ));
            }
        }
    }
    for condition in &out_decl.conditions {
        let (cond_ty, cond_diags) = check_expr(&condition.body, resolve);
        diagnostics.extend(cond_diags);
        if !cond_ty.unifies_with(&Ty::Bool) {
            diagnostics.push(ParseError::new_range(
                format!(
                    "condition `{}` produces `{}`, but conditions must be `bool`",
                    condition.name,
                    cond_ty.name()
                ),
                condition.body.span().start,
                condition.body.span().end,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AdamAstParser;

    fn parse(source: &str) -> Sheet {
        AdamAstParser::new().parse_str(source).unwrap()
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
        // adam_lang::parser's real cell_decl grammar parses an annotated initializer via
        // parse_literal_as(entry, lit, span) — it parses the literal's digits directly as the
        // declared type, ignoring the literal's own (absent) suffix. `cell x: u32 = 1;` is valid,
        // accepted adam-lang; the checker must not falsely flag it.
        let sheet = parse("sheet s { cell x: u32 = 1; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn cell_initializer_char_literal_against_any_registered_type_is_a_diagnostic() {
        // parse_literal_as has no arm for a char literal against any registered type — adam-lang's
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
                 0i32 => { relationship { method [a] -> [out] { a } } }, \
                 _ => { relationship { method [b] -> [out] { b } } }, \
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

    #[test]
    fn out_body_matching_its_annotation_has_no_diagnostic() {
        let sheet = parse(
            "sheet s { cell width: f64; cell height: f64; \
             out area: f64 { method [width, height] { width * height } } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn out_body_mismatched_with_its_annotation_is_a_diagnostic() {
        let sheet = parse(
            "sheet s { cell width: f64; cell height: f64; \
             out area: i32 { method [width, height] { width * height } } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn out_with_no_annotation_infers_its_type_and_has_no_diagnostic() {
        // No `: type_name` to cross-check against — nothing to flag, and a later reference to
        // `area`'s name (were one added) would resolve through the inferred f64, not Ty::Any.
        let sheet = parse(
            "sheet s { cell width: f64; cell height: f64; \
             out area { method [width, height] { width * height } } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn condition_with_bool_body_has_no_diagnostic() {
        let sheet = parse(
            "sheet s { cell width: f64; cell max_width: f64; \
             out area: f64 { \
                 method [width] { width } \
                 condition max_width [width, max_width] { width <= max_width } \
             } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn condition_with_non_bool_body_is_a_diagnostic() {
        let sheet = parse(
            "sheet s { cell width: f64; \
             out area: f64 { \
                 method [width] { width } \
                 condition bogus [width] { width } \
             } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn cell_tuple_initializer_matching_its_annotation_has_no_diagnostic() {
        let sheet = parse("sheet s { cell a: (i32, f64) = (1, 2.5); }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn cell_tuple_initializer_arity_mismatch_is_a_diagnostic() {
        let sheet = parse("sheet s { cell a: (i32, f64, i32) = (1, 2.5); }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn cell_tuple_initializer_element_type_mismatch_is_a_diagnostic() {
        let sheet = parse("sheet s { cell a: (i32, i32) = (1, 2.5); }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn cell_nested_tuple_initializer_matching_its_annotation_has_no_diagnostic() {
        let sheet = parse("sheet s { cell a: (i32, (f64, String)) = (1, (2.5, \"x\")); }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn method_single_tuple_typed_output_matching_body_has_no_diagnostic() {
        let sheet = parse(
            "sheet s { cell a: i32; cell b: i32; cell pair: (i32, i32); \
             relationship { method [a, b] -> [pair] { (a, b) } } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn method_single_tuple_typed_output_element_type_mismatch_is_a_diagnostic() {
        let sheet = parse(
            "sheet s { cell a: i32; cell b: f64; cell pair: (i32, i32); \
             relationship { method [a, b] -> [pair] { (a, b) } } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }
}
