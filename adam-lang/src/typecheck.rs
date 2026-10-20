//! A best-effort static type checker over [`crate::ast::Sheet`] trees, built on
//! [`cel_parser::ty::check_expr_with_type_resolver`]. Checks each `cell`'s literal or
//! tuple-shaped initializer against its `: type_name` annotation (a `source`'s initializer is
//! checked identically — a `source` shares
//! `cell`'s exact shape, including its optional `filter` clause), each `relationship`/`conditional`
//! binding's body against its declared outputs (arity: does the body actually produce as many
//! values as declared; and per-output type), and each `out`'s initializer body against its optional
//! `: type_name` annotation. Any `cell`, `source`, or `out`'s optional `require { ... }` block has
//! each of its `requirement` bodies checked to produce `bool` type. Adam cell annotations that
//! name a type [`crate::TypeRegistry`] doesn't recognize still resolve to [`cel_parser::Ty::Any`]
//! and are never flagged, while CEL array annotations inside expression bodies resolve named
//! leaves through the same registry-backed CEL resolver [`check_sheet`] builds per pass. An absent
//! annotation or a call without metadata resolves to
//! [`cel_parser::Ty::Any`] and are never flagged — matching adam-lang/CEL's extensible type
//! system. Not a complete type system; see the design doc's "Type checking (v1)" section.

use cel_parser::{Expr, ExprSpan, Literal, ParseError, Ty, ty::check_expr_with_type_resolver};

use crate::TypeRegistry;
use crate::ast::{BindingDecl, CellFilter, OutDecl, RequireBlock, Sheet, SheetItem, TypeExpr};
use crate::type_registry::TypeShape;

/// Checks `sheet` against registered types and operation/record metadata, returning diagnostics.
/// Never fails — an absent Adam annotation, an unresolved identifier, an Adam `cell`/`source`/`out`
/// annotation `registry` doesn't resolve, or a custom call without metadata falls back to
/// [`cel_parser::Ty::Any`] and are silently skipped, not reported. CEL array annotations inside
/// expressions use the same registry-backed CEL resolver, so unknown leaf names such as
/// `[]: [Missing]` are still reported. Known custom annotations retain their complete leaf
/// descriptors. Constructors and members use `lookup`'s schemas/signatures, not value names;
/// unknown constructors and incompatible known fields/operands produce diagnostics.
///
/// - Complexity: proportional to checking every expression with
///   [`check_expr_with_type_resolver`], plus O(n) sheet traversal and registry snapshot costs.
///
/// # Examples
///
/// ```rust
/// use adam_lang::{AdamAstParser, TypeRegistry, check_sheet};
///
/// let sheet = AdamAstParser::new()
///     .parse_str("sheet s { cell x: i32 = 1.0; }")
///     .unwrap();
/// let diagnostics = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
/// assert_eq!(diagnostics.len(), 1, "1.0 defaults to f64, mismatching the i32 annotation");
/// ```
///
/// ```rust
/// use adam_lang::{AdamAstParser, TypeRegistry, check_sheet};
///
/// let sheet = AdamAstParser::new()
///     .parse_str("sheet s { out values := [0, 1]: [f64]; }")
///     .unwrap();
/// let diagnostics = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
/// assert_eq!(diagnostics.len(), 1);
/// assert_eq!(
///     diagnostics[0].message(),
///     "array elements must match the annotation exactly: expected `f64`, found `i32`"
/// );
/// ```
///
/// ```rust
/// use adam_lang::{AdamAstParser, TypeRegistry, check_sheet};
///
/// let sheet = AdamAstParser::new()
///     .parse_str("sheet s { out values := []: [Missing]; }")
///     .unwrap();
/// let diagnostics = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
/// assert_eq!(diagnostics.len(), 1);
/// assert_eq!(diagnostics[0].message(), "unknown type `Missing`");
/// ```
pub fn check_sheet(
    sheet: &Sheet,
    registry: &TypeRegistry,
    lookup: &cel_parser::OpLookup,
) -> Vec<ParseError> {
    let mut diagnostics = Vec::new();
    let declared = declared_cell_types(sheet, registry, lookup);
    let shapes = &declared.shapes;
    let resolve = |name: &str| -> Ty { declared.values.get(name).cloned().unwrap_or(Ty::Any) };
    let type_resolver = registry.cel_type_resolver();
    for item in &sheet.items {
        match item {
            SheetItem::Cell(cell) => {
                check_cell_initializer(
                    cell.type_name.as_ref(),
                    cell.initializer.as_ref(),
                    registry,
                    &type_resolver,
                    lookup,
                    &mut diagnostics,
                );
                check_filter(
                    &cell.name,
                    cell.filter.as_ref(),
                    &declared,
                    &resolve,
                    &type_resolver,
                    lookup,
                    &mut diagnostics,
                );
                check_requirements(
                    cell.require.as_ref(),
                    &resolve,
                    &type_resolver,
                    lookup,
                    &mut diagnostics,
                );
            }
            SheetItem::Source(source) => {
                check_cell_initializer(
                    source.type_name.as_ref(),
                    source.initializer.as_ref(),
                    registry,
                    &type_resolver,
                    lookup,
                    &mut diagnostics,
                );
                check_filter(
                    &source.name,
                    source.filter.as_ref(),
                    &declared,
                    &resolve,
                    &type_resolver,
                    lookup,
                    &mut diagnostics,
                );
                check_requirements(
                    source.require.as_ref(),
                    &resolve,
                    &type_resolver,
                    lookup,
                    &mut diagnostics,
                );
            }
            SheetItem::Relationship(rel) => {
                for binding in &rel.bindings {
                    check_binding(
                        binding,
                        registry,
                        shapes,
                        &resolve,
                        &type_resolver,
                        lookup,
                        &mut diagnostics,
                    );
                }
            }
            SheetItem::Conditional(cond) => {
                diagnostics.extend(
                    check_expr_with_type_resolver(
                        &cond.match_expr,
                        &resolve,
                        &type_resolver,
                        lookup,
                    )
                    .1,
                );
                for branch in &cond.branches {
                    for rel in &branch.relationships {
                        for binding in &rel.bindings {
                            check_binding(
                                binding,
                                registry,
                                shapes,
                                &resolve,
                                &type_resolver,
                                lookup,
                                &mut diagnostics,
                            );
                        }
                    }
                }
                if let Some(default) = &cond.default {
                    for rel in &default.relationships {
                        for binding in &rel.bindings {
                            check_binding(
                                binding,
                                registry,
                                shapes,
                                &resolve,
                                &type_resolver,
                                lookup,
                                &mut diagnostics,
                            );
                        }
                    }
                }
            }
            SheetItem::Out(out_decl) => check_out(
                out_decl,
                registry,
                &declared,
                &resolve,
                &type_resolver,
                lookup,
                &mut diagnostics,
            ),
            SheetItem::Error { .. } => {} // already reported as a syntax error; nothing to type-check
        }
    }
    diagnostics
}

/// Converts a recursive [`TypeShape`] to the CEL expression checker's complete static type.
///
/// - Complexity: O(n + l) for n recursive nodes and l total registered leaf-name bytes.
fn shape_to_ty(shape: &TypeShape, registry: &TypeRegistry) -> Ty {
    match shape {
        TypeShape::Named(type_id) => registry
            .resolved_leaf_type(*type_id)
            .map(|leaf| Ty::from_leaf(&leaf))
            .unwrap_or(Ty::Any),
        TypeShape::Tuple(elements) => Ty::Tuple(
            elements
                .iter()
                .map(|element| shape_to_ty(element, registry))
                .collect(),
        ),
    }
}

/// Returns whether `lit` matches `declared` exactly, preserving adam-lang's no-coercion literal
/// initializer rules.
fn literal_matches_declared_ty(lit: &Literal, declared: &Ty) -> bool {
    Ty::from_literal(lit).unifies_with(declared)
}

/// Keeps scalar approximations and exact annotation shapes for the same sheet declarations.
///
/// `values` includes all cells, sources, and outputs; `shapes` includes only resolved annotations.
struct DeclaredTypes {
    /// Inferred or annotated value types, using `Any` where unresolved.
    values: std::collections::HashMap<String, Ty>,
    /// Resolved recursive annotation shapes, including tuple structure.
    shapes: std::collections::HashMap<String, TypeShape>,
}

/// Maps every declared cell name — from a `cell`, a `source`, or an `out` — to both its recursive
/// `Ty` (for use as the identifier resolver method/condition bodies
/// are checked against) and its full recursive `TypeShape` (for `expr_matches_shape`'s tuple-aware
/// checks). A `source` is resolved exactly like a `cell` (same shape) so its name is referenceable
/// by other cells'/sources'/outs' expressions exactly like a `cell`'s.
/// A `cell`/`source`/`out` with no annotation, or one naming a type `registry` doesn't resolve, is
/// absent from the `TypeShape` map and maps to `Ty::Any` in the `Ty` map. An `out` with an
/// annotation resolves the same way as a `cell`; one without is
/// inferred from its initializer body's checked type, using only `cell`-declared types as context (not
/// other `out`s' inferred types — see this function's own note above), and is never present in
/// the `TypeShape` map (only annotated cells/outs are).
///
/// - Complexity: O(n) sheet traversal plus annotation resolution and output-body checking.
fn declared_cell_types(
    sheet: &Sheet,
    registry: &TypeRegistry,
    lookup: &cel_parser::OpLookup,
) -> DeclaredTypes {
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

    let mut map = std::collections::HashMap::new();
    let mut shapes = std::collections::HashMap::new();
    for item in &sheet.items {
        match item {
            SheetItem::Cell(cell) => {
                let shape = resolve_annotation_shape(cell.type_name.as_ref(), registry);
                let ty = shape
                    .as_ref()
                    .map(|shape| shape_to_ty(shape, registry))
                    .unwrap_or(Ty::Any);
                if let Some(shape) = shape {
                    shapes.insert(cell.name.clone(), shape);
                }
                map.insert(cell.name.clone(), ty);
            }
            SheetItem::Source(source) => {
                let shape = resolve_annotation_shape(source.type_name.as_ref(), registry);
                let ty = shape
                    .as_ref()
                    .map(|shape| shape_to_ty(shape, registry))
                    .unwrap_or(Ty::Any);
                if let Some(shape) = shape {
                    shapes.insert(source.name.clone(), shape);
                }
                map.insert(source.name.clone(), ty);
            }
            _ => {}
        }
    }
    let resolve_cells = |name: &str| -> Ty { map.get(name).cloned().unwrap_or(Ty::Any) };
    let type_resolver = registry.cel_type_resolver();
    let mut out_types = std::collections::HashMap::new();
    for item in &sheet.items {
        if let SheetItem::Out(out_decl) = item {
            let shape = resolve_annotation_shape(out_decl.type_name.as_ref(), registry);
            let ty = shape
                .as_ref()
                .map(|shape| shape_to_ty(shape, registry))
                .unwrap_or_else(|| {
                    check_expr_with_type_resolver(
                        &out_decl.initializer,
                        &resolve_cells,
                        &type_resolver,
                        lookup,
                    )
                    .0
                });
            if let Some(shape) = shape {
                shapes.insert(out_decl.name.clone(), shape);
            }
            out_types.insert(out_decl.name.clone(), ty);
        }
    }
    map.extend(out_types);
    DeclaredTypes {
        values: map,
        shapes,
    }
}

/// Checks whether `expr` structurally matches `shape`, recursively: a `TypeShape::Named` leaf
/// must be a non-tuple `Expr` whose checked `Ty` unifies with that leaf (mirroring
/// adam-lang's exact-type initializer matching, generalized past bare literals now that
/// initializers are full `expression`s); a `TypeShape::Tuple` must be an `Expr::Tuple` of matching arity,
/// checked element-wise, or an `Expr::If` whose `then_branch` (and `else_branch`, if present —
/// itself possibly another `Expr::If`, covering `else if` chains) each recursively match the same
/// `shape`, since every branch that can be taken must produce a value of that shape. An `if` with
/// no `else` is checked only against `then_branch`, matching `check_expr`'s existing leniency
/// toward a missing else. `TypeShape::Named(TypeId)` with no registered entry (an unrecognized
/// custom type) always matches — not statically checked, mirroring `Ty::Any`'s existing
/// leniency.
///
/// - Complexity: O(n) in the number of (nested) tuple elements and `if`/`else if` branches.
fn expr_matches_shape(
    expr: &Expr,
    shape: &TypeShape,
    registry: &TypeRegistry,
    resolve: &impl Fn(&str) -> Ty,
    resolve_type: &impl cel_parser::TypeResolver,
    lookup: &cel_parser::OpLookup,
    diagnostics: &mut Vec<ParseError>,
) {
    match (expr, shape) {
        (Expr::Tuple { elements, .. }, TypeShape::Tuple(expected)) => {
            if elements.len() != expected.len() {
                diagnostics
                    .extend(check_expr_with_type_resolver(expr, resolve, resolve_type, lookup).1);
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
                expr_matches_shape(
                    element,
                    element_shape,
                    registry,
                    resolve,
                    resolve_type,
                    lookup,
                    diagnostics,
                );
            }
        }
        (
            Expr::If {
                cond,
                then_branch,
                else_branch,
                ..
            },
            TypeShape::Tuple(_),
        ) => {
            let (_, cond_diags) =
                check_expr_with_type_resolver(cond, resolve, resolve_type, lookup);
            diagnostics.extend(cond_diags);
            expr_matches_shape(
                then_branch,
                shape,
                registry,
                resolve,
                resolve_type,
                lookup,
                diagnostics,
            );
            if let Some(else_branch) = else_branch {
                expr_matches_shape(
                    else_branch,
                    shape,
                    registry,
                    resolve,
                    resolve_type,
                    lookup,
                    diagnostics,
                );
            }
        }
        (_, TypeShape::Tuple(_)) => {
            let (actual, nested) =
                check_expr_with_type_resolver(expr, resolve, resolve_type, lookup);
            if !nested.is_empty() {
                diagnostics.extend(nested);
                return;
            }
            if actual != Ty::Any && shape_to_ty(shape, registry).unifies_with(&actual) {
                return;
            }
            diagnostics.push(ParseError::new_range(
                format!("expected tuple `{}`", registry.display_name(shape)),
                expr.span().start,
                expr.span().end,
            ));
        }
        (Expr::Tuple { .. }, TypeShape::Named(_)) => {
            diagnostics
                .extend(check_expr_with_type_resolver(expr, resolve, resolve_type, lookup).1);
            diagnostics.push(ParseError::new_range(
                format!("expected `{}`, got a tuple", registry.display_name(shape)),
                expr.span().start,
                expr.span().end,
            ));
        }
        (_, TypeShape::Named(type_id)) => {
            let Some(entry) = registry.entry_by_type_id(*type_id) else {
                diagnostics
                    .extend(check_expr_with_type_resolver(expr, resolve, resolve_type, lookup).1);
                return; // unrecognized custom type: never statically checked, matches Ty::Any
            };
            let declared = shape_to_ty(&TypeShape::Named(entry.type_id), registry);
            let (actual, body_diags) =
                check_expr_with_type_resolver(expr, resolve, resolve_type, lookup);
            let valid = body_diags.is_empty();
            diagnostics.extend(body_diags);
            if valid && !declared.unifies_with(&actual) {
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

/// Checks one `cell`'s or `source`'s initializer against its `: type_expr` annotation. A no-op if
/// the initializer is absent. Every initializer is recursively checked, including when its
/// annotation is absent or unresolved. Known annotations check the initializer's inferred
/// type exactly; tuple annotations check recursively element-wise. An invalid body produces
/// its own diagnostics without an additional annotation mismatch.
///
/// Takes `type_name`/`initializer` directly (rather than a whole `&CellDecl`) so both
/// `SheetItem::Cell` and `SheetItem::Source` — which share this same shape but aren't the same
/// Rust type — can share one check.
///
/// - Complexity: the cost of recursively checking the initializer and resolving its annotation.
fn check_cell_initializer(
    type_name: Option<&TypeExpr>,
    initializer: Option<&Expr>,
    registry: &TypeRegistry,
    resolve_type: &impl cel_parser::TypeResolver,
    lookup: &cel_parser::OpLookup,
    diagnostics: &mut Vec<ParseError>,
) {
    let Some(expr) = initializer else {
        return;
    };
    let resolve = |_: &str| Ty::Any;
    let Some(shape) = type_name.and_then(|type_expr| registry.resolve(type_expr).ok()) else {
        diagnostics.extend(check_expr_with_type_resolver(expr, &resolve, resolve_type, lookup).1);
        return;
    };
    if let TypeShape::Tuple(_) = shape {
        expr_matches_shape(
            expr,
            &shape,
            registry,
            &resolve,
            resolve_type,
            lookup,
            diagnostics,
        );
        return;
    }
    if let Expr::Literal {
        value: literal,
        span: lit_span,
    } = expr
    {
        let declared = shape_to_ty(&shape, registry);
        if !literal_matches_declared_ty(literal, &declared) {
            diagnostics.push(ParseError::new_range(
                format!("literal cannot be used as type `{}`", declared.name()),
                lit_span.start,
                lit_span.end,
            ));
        }
        return;
    }
    expr_matches_shape(
        expr,
        &shape,
        registry,
        &resolve,
        resolve_type,
        lookup,
        diagnostics,
    );
}

/// The expected `TypeShape` for a filtered cell's own declared/inferred shape (`_`'s type inside
/// its filter body) — `Some` only when a concrete shape is known: a tuple-typed annotation
/// (from `declared.shapes`), or a scalar type either annotated or already resolved to a concrete `Ty`
/// (from `declared.values`, converted via `Ty::type_id`). `None` when nothing is known (an unannotated
/// cell, or a name neither map has an entry for), mirroring `Ty::Any`'s existing "never flagged"
/// leniency elsewhere in this file.
///
/// - Complexity: O(1) amortized lookup plus an O(k) clone of a possibly-nested `TypeShape`
///   (k = the shape's own element count) when found in `declared.shapes`.
fn expected_shape(name: &str, declared: &DeclaredTypes) -> Option<TypeShape> {
    if let Some(shape) = declared.shapes.get(name) {
        return Some(shape.clone());
    }
    declared
        .values
        .get(name)
        .and_then(Ty::type_id)
        .map(TypeShape::Named)
}

/// Returns whether `expr` contains a reference to the identifier `name` anywhere in its tree.
/// Used by `check_filter` to check whether a filter's body references `_` — deliberately a plain
/// structural walk, not built on `check_expr`'s identifier resolution, so checking for `_`'s
/// presence never runs type-checking a second time over any part of `expr`.
///
/// - Complexity: O(n) in the number of sub-expressions in `expr`.
fn expr_references_ident(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Literal { .. } => false,
        Expr::Ident { name: ident, .. } => ident == name,
        Expr::Struct { fields, .. } => fields
            .iter()
            .any(|field| expr_references_ident(&field.value, name)),
        Expr::Op { operands, .. } => operands.iter().any(|e| expr_references_ident(e, name)),
        Expr::Apply { callee, args, .. } => {
            expr_references_ident(callee, name)
                || args.iter().any(|e| expr_references_ident(e, name))
        }
        Expr::Member { receiver, args, .. } => {
            expr_references_ident(receiver, name)
                || args
                    .iter()
                    .flatten()
                    .any(|e| expr_references_ident(e, name))
        }
        Expr::Tuple { elements, .. } => elements.iter().any(|e| expr_references_ident(e, name)),
        Expr::Array { elements, .. } => elements.iter().any(|e| expr_references_ident(e, name)),
        Expr::TupleIndex { base, .. } => expr_references_ident(base, name),
        Expr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            expr_references_ident(cond, name)
                || expr_references_ident(then_branch, name)
                || else_branch
                    .as_deref()
                    .is_some_and(|e| expr_references_ident(e, name))
        }
        Expr::Logical { lhs, rhs, .. } => {
            expr_references_ident(lhs, name) || expr_references_ident(rhs, name)
        }
        Expr::Cast { expr, .. } => expr_references_ident(expr, name),
        Expr::Closure { params, body, .. } => {
            !params.iter().any(|param| param.name == name) && expr_references_ident(body, name)
        }
    }
}

/// Returns `true` if `expr` is itself a top-level `..=` range expression (`Expr::Op { name:
/// "range_inclusive", .. }`) — the one structural shape `check_filter` exempts from its "must
/// reference `_`" requirement, mirroring `adam_lang::parser::AdamParser::parse_cell_filter`'s
/// matching exemption for the same reason: a `lo..=hi` filter body's bounds never depend on the
/// candidate value, only on its own two endpoints. Deliberately checks only the whole body, not
/// any nested sub-expression — matches the runtime layer's own recognition, which is keyed on the
/// *entire compiled expression's* inferred type, not a nested occurrence.
fn is_range_inclusive_body(expr: &Expr) -> bool {
    matches!(expr, Expr::Op { name, .. } if name == "range_inclusive")
}

/// Checks one filtered cell's `filter` clause, if present: a tuple-typed filtered cell is
/// rejected outright (mirroring the runtime parser's own rejection — not yet supported by either
/// layer); otherwise, the body's inferred type must unify with this cell's own declared/inferred
/// shape (`_`'s type, via `body_resolve`'s special case below), and the body must reference `_`
/// — the value being filtered — at least once. Every other identifier is resolved exactly as any
/// other deduced expression in this file (a `relationship` binding, an `out` initializer): via
/// `resolve`, which leaves an unrecognized name as `Ty::Any` rather than raising a diagnostic —
/// the runtime `Sheet`-building parser (`adam_lang::parser::AdamParser::parse_cell_filter`) is
/// what raises "undeclared cell" for a name that isn't actually a declared cell, mirroring how it
/// (not this file) is the one that raises that error for bindings' deduced expressions too.
///
/// Takes `name`/`filter` directly (rather than a whole `&CellDecl`) so a `cell`'s
/// [`SheetItem::Cell`] arm, a `source`'s [`SheetItem::Source`] arm, and an `out`'s [`check_out`]
/// can all share this one check — `filter` is `Option<&CellFilter>` (not an owned/whole-decl
/// reference) since `OutDecl`, `CellDecl`, and `SourceDecl` each declare their own `filter:
/// Option<CellFilter>` field independently.
///
/// - Complexity: body type checking plus O(n) reference traversal over the filter expression.
fn check_filter(
    name: &str,
    filter: Option<&CellFilter>,
    declared: &DeclaredTypes,
    resolve: &impl Fn(&str) -> Ty,
    resolve_type: &impl cel_parser::TypeResolver,
    lookup: &cel_parser::OpLookup,
    diagnostics: &mut Vec<ParseError>,
) {
    let Some(filter) = filter else {
        return;
    };

    let shape = expected_shape(name, declared);
    if matches!(shape, Some(TypeShape::Tuple(_))) {
        // Mirrors `adam_lang::parser::AdamParser::parse_cell_filter`'s runtime rejection of a
        // tuple-typed filtered cell — not yet supported by either layer, so both must agree
        // rather than the CST checker accepting a construct the runtime cannot build.
        diagnostics.push(ParseError::new_range(
            format!("cell `{name}`: filter on a tuple-typed cell is not yet supported"),
            filter.span.start,
            filter.span.end,
        ));
        return;
    }

    let own_ty = resolve(name);
    let body_resolve = |ident: &str| -> Ty {
        if ident == "_" {
            own_ty.clone()
        } else {
            resolve(ident)
        }
    };

    match shape {
        Some(TypeShape::Tuple(_)) => unreachable!("handled above"),
        Some(TypeShape::Named(_)) => {
            let (body_ty, body_diags) =
                check_expr_with_type_resolver(&filter.body, &body_resolve, resolve_type, lookup);
            diagnostics.extend(body_diags);
            let declared = own_ty.clone();
            if !declared.unifies_with(&body_ty) {
                diagnostics.push(ParseError::new_range(
                    format!("cell `{name}`: filter must produce `{}`", declared.name()),
                    filter.body.span().start,
                    filter.body.span().end,
                ));
            }
        }
        None => {
            let (_, body_diags) =
                check_expr_with_type_resolver(&filter.body, &body_resolve, resolve_type, lookup);
            diagnostics.extend(body_diags);
        }
    }

    if !is_range_inclusive_body(&filter.body) && !expr_references_ident(&filter.body, "_") {
        diagnostics.push(ParseError::new_range(
            "filter must reference `_` (the value being filtered)".to_string(),
            filter.span.start,
            filter.span.end,
        ));
    }
}

/// Checks `body`'s multi-output shape against `outputs`, recursively: an `Expr::Tuple` of matching
/// arity is checked element-wise, each element against its corresponding output cell's declared
/// type; an `Expr::If` recurses into `then_branch` (and `else_branch`, if present — itself possibly
/// another `Expr::If`, covering `else if` chains) against the same `outputs`, since every branch
/// that can be taken must produce a value of that shape (mirroring [`expr_matches_shape`]'s
/// `Expr::If` handling); a known scalar or wrong-size literal tuple is a diagnostic.
/// An `if` with no `else` is checked
/// only against `then_branch`, matching `check_expr`'s existing leniency toward a missing else.
/// Unresolved nonliteral bodies retain child diagnostics but defer tuple arity to runtime.
///
/// - Complexity: O(n) in the number of declared outputs, times the number of (nested) `if`/`else
///   if` branches.
fn check_tuple_output_body(
    body: &Expr,
    outputs: &[(String, ExprSpan)],
    resolve: &impl Fn(&str) -> Ty,
    resolve_type: &impl cel_parser::TypeResolver,
    lookup: &cel_parser::OpLookup,
    diagnostics: &mut Vec<ParseError>,
) {
    match body {
        Expr::Tuple { elements, .. } if elements.len() == outputs.len() => {
            for (element, (name, _)) in elements.iter().zip(outputs) {
                let (element_ty, element_diags) =
                    check_expr_with_type_resolver(element, resolve, resolve_type, lookup);
                diagnostics.extend(element_diags);
                let declared = resolve(name);
                if !declared.unifies_with(&element_ty) {
                    diagnostics.push(ParseError::new_range(
                        format!(
                            "binding output `{name}` produces `{}`, but is declared `{}`",
                            element_ty.name(),
                            declared.name()
                        ),
                        element.span().start,
                        element.span().end,
                    ));
                }
            }
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            let (_, cond_diags) =
                check_expr_with_type_resolver(cond, resolve, resolve_type, lookup);
            diagnostics.extend(cond_diags);
            check_tuple_output_body(
                then_branch,
                outputs,
                resolve,
                resolve_type,
                lookup,
                diagnostics,
            );
            if let Some(else_branch) = else_branch {
                check_tuple_output_body(
                    else_branch,
                    outputs,
                    resolve,
                    resolve_type,
                    lookup,
                    diagnostics,
                );
            }
        }
        other => {
            let (body_ty, body_diags) =
                check_expr_with_type_resolver(other, resolve, resolve_type, lookup);
            let valid = body_diags.is_empty();
            diagnostics.extend(body_diags);
            if !valid {
                return;
            }
            if let Ty::Tuple(elements) = &body_ty
                && elements.len() == outputs.len()
            {
                for (element_ty, (name, _)) in elements.iter().zip(outputs) {
                    let declared = resolve(name);
                    if !declared.unifies_with(element_ty) {
                        diagnostics.push(ParseError::new_range(
                            format!(
                                "binding output `{name}` produces `{}`, but is declared `{}`",
                                element_ty.name(),
                                declared.name()
                            ),
                            other.span().start,
                            other.span().end,
                        ));
                    }
                }
                return;
            }
            if body_ty == Ty::Any && !matches!(other, Expr::Tuple { .. }) {
                return;
            }
            let n = outputs.len();
            diagnostics.push(ParseError::new_range(
                format!("binding declares {n} outputs but its body is not a {n}-tuple"),
                other.span().start,
                other.span().end,
            ));
        }
    }
}

/// Checks one `binding`'s body against its declared outputs, dispatching on
/// [`BindingDecl::destructure`]: a destructuring binding (`(a, b) := ...` or the single-element
/// `(a,) := ...`) is checked recursively via [`check_tuple_output_body`] against each output
/// cell; a direct-bind single output's inferred type must instead unify with that output cell's
/// declared type (or, when that output's declared type is itself a tuple — per `shapes` — the
/// body is checked recursively via [`expr_matches_shape`] instead). Operator-level diagnostics
/// from inside the body (via [`check_expr`]) are always included exactly once, regardless of
/// which branch below runs.
///
/// - Complexity: body type checking plus recursive output-shape validation.
fn check_binding(
    binding: &BindingDecl,
    registry: &TypeRegistry,
    shapes: &std::collections::HashMap<String, TypeShape>,
    resolve: &impl Fn(&str) -> Ty,
    resolve_type: &impl cel_parser::TypeResolver,
    lookup: &cel_parser::OpLookup,
    diagnostics: &mut Vec<ParseError>,
) {
    if binding.destructure {
        check_tuple_output_body(
            &binding.body,
            &binding.outputs,
            resolve,
            resolve_type,
            lookup,
            diagnostics,
        );
        return;
    }
    let Some((name, _)) = binding.outputs.first() else {
        let (_, body_diags) =
            check_expr_with_type_resolver(&binding.body, resolve, resolve_type, lookup);
        diagnostics.extend(body_diags);
        return;
    };
    if let Some(shape @ TypeShape::Tuple(_)) = shapes.get(name) {
        expr_matches_shape(
            &binding.body,
            shape,
            registry,
            resolve,
            resolve_type,
            lookup,
            diagnostics,
        );
        return;
    }
    let (body_ty, body_diags) =
        check_expr_with_type_resolver(&binding.body, resolve, resolve_type, lookup);
    diagnostics.extend(body_diags);
    if let Expr::Tuple { elements, .. } = &binding.body {
        let n = elements.len();
        diagnostics.push(ParseError::new_range(
            format!("binding declares 1 output but its body is a {n}-tuple"),
            binding.body.span().start,
            binding.body.span().end,
        ));
        return;
    }
    let declared = resolve(name);
    if !declared.unifies_with(&body_ty) {
        diagnostics.push(ParseError::new_range(
            format!(
                "binding body produces `{}`, but `{name}` is declared `{}`",
                body_ty.name(),
                declared.name()
            ),
            binding.body.span().start,
            binding.body.span().end,
        ));
    }
}

/// Checks one `out`'s initializer body against its optional `: type_expr` annotation — mirroring
/// `check_binding`'s single-output branch, since an out's initializer is structurally a binding
/// with one implicit output (the out cell itself), including the same tuple-shaped dispatch to
/// [`expr_matches_shape`] via `shapes` — its optional `filter` clause, via the same [`check_filter`]
/// shared with a `cell`'s [`SheetItem::Cell`] arm and a `source`'s [`SheetItem::Source`] arm — and,
/// if a `require { ... }` block is present, each of its requirements' bodies against `Ty::Bool`.
/// Operator-level diagnostics from inside any body (via `check_expr`) are always included,
/// regardless of whether a mismatch diagnostic is also added.
///
/// - Complexity: body, filter, and requirement checking plus annotation-shape validation.
fn check_out(
    out_decl: &OutDecl,
    registry: &TypeRegistry,
    declared: &DeclaredTypes,
    resolve: &impl Fn(&str) -> Ty,
    resolve_type: &impl cel_parser::TypeResolver,
    lookup: &cel_parser::OpLookup,
    diagnostics: &mut Vec<ParseError>,
) {
    if let Some(shape @ TypeShape::Tuple(_)) = declared.shapes.get(&out_decl.name) {
        expr_matches_shape(
            &out_decl.initializer,
            shape,
            registry,
            resolve,
            resolve_type,
            lookup,
            diagnostics,
        );
    } else {
        let (body_ty, body_diags) =
            check_expr_with_type_resolver(&out_decl.initializer, resolve, resolve_type, lookup);
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
                    out_decl.initializer.span().start,
                    out_decl.initializer.span().end,
                ));
            }
        }
    }
    check_filter(
        &out_decl.name,
        out_decl.filter.as_ref(),
        declared,
        resolve,
        resolve_type,
        lookup,
        diagnostics,
    );
    check_requirements(
        out_decl.require.as_ref(),
        resolve,
        resolve_type,
        lookup,
        diagnostics,
    );
}

/// Checks every requirement in `require`'s body against `resolve`, appending a diagnostic for
/// each one that doesn't type-check as `bool`. A no-op if `require` is absent (a plain `cell`,
/// `source`, or `out` with no `require { ... }` block). Shared by [`check_out`] and by the
/// `SheetItem::Cell`/`SheetItem::Source` arms of [`check_sheet`]'s match, since `require` has the
/// same shape and rules regardless of which cell kind it's attached to.
///
/// - Complexity: the sum of type-checking costs for every requirement body.
fn check_requirements(
    require: Option<&RequireBlock>,
    resolve: &impl Fn(&str) -> Ty,
    resolve_type: &impl cel_parser::TypeResolver,
    lookup: &cel_parser::OpLookup,
    diagnostics: &mut Vec<ParseError>,
) {
    let Some(require) = require else {
        return;
    };
    for requirement in &require.requirements {
        let (req_ty, req_diags) =
            check_expr_with_type_resolver(&requirement.body, resolve, resolve_type, lookup);
        diagnostics.extend(req_diags);
        if !req_ty.unifies_with(&Ty::Bool) {
            diagnostics.push(ParseError::new_range(
                match &requirement.name {
                    Some(name) => format!(
                        "requirement `{name}` produces `{}`, but requirements must be `bool`",
                        req_ty.name()
                    ),
                    None => format!(
                        "requirement produces `{}`, but requirements must be `bool`",
                        req_ty.name()
                    ),
                },
                requirement.body.span().start,
                requirement.body.span().end,
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

    #[derive(Clone, Debug, PartialEq)]
    struct Custom(i32);

    #[test]
    fn cell_initializer_matching_its_annotation_has_no_diagnostic() {
        let sheet = parse("sheet s { cell x: i32 = 1; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn cell_initializer_mismatched_with_its_annotation_is_a_diagnostic() {
        // Unsuffixed float literal defaults to f64, not i32.
        let sheet = parse("sheet s { cell x: i32 = 1.0; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn source_initializer_mismatched_with_its_annotation_is_a_diagnostic() {
        // Unsuffixed float literal defaults to f64, not i32.
        let sheet = parse("sheet s { source x: i32 = 1.0; }");
        let diagnostics = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn custom_array_annotations_use_the_registry_type_resolver() {
        let sheet = parse(
            "sheet s { \
                out values := []: [Custom]; \
                out nested := [[]: [Custom]]: [[Custom]]; \
            }",
        );
        let mut registry = TypeRegistry::new();
        registry.register_no_default::<Custom>("Custom");

        let diagnostics = check_sheet(&sheet, &registry, &cel_parser::OpLookup::new());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn custom_array_annotations_reject_exact_scalar_type_mismatches() {
        let sheet = parse("sheet s { out values := [1]: [Custom]; }");
        let mut registry = TypeRegistry::new();
        registry.register_no_default::<Custom>("Custom");

        let diagnostics = check_sheet(&sheet, &registry, &cel_parser::OpLookup::new());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(
            diagnostics[0]
                .message()
                .contains("expected `Custom`, found `i32`"),
            "got: {}",
            diagnostics[0].message()
        );
    }

    #[test]
    fn unknown_custom_array_annotation_type_is_reported() {
        let sheet = parse("sheet s { out values := []: [Missing]; }");
        let diagnostics = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].message().contains("unknown type `Missing`"),
            "got: {}",
            diagnostics[0].message()
        );
    }

    #[test]
    fn tuple_typed_array_annotations_are_accepted_by_cel_checking() {
        let sheet = parse("sheet s { out values := []: [(i32, f64)]; }");
        let diagnostics = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn cell_requirement_non_bool_body_is_a_diagnostic() {
        let sheet = parse("sheet s { cell x: i32 = 5 require { @positive x; }; }");
        let diagnostics = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn cell_initializer_non_literal_expression_is_checked() {
        let sheet = parse("sheet s { cell x: f64 = 1 + 2; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        let messages: Vec<&str> = diags.iter().map(cel_parser::ParseError::message).collect();
        assert_eq!(
            messages,
            ["expression produces `i32`, but `f64` was expected"]
        );
    }

    #[test]
    fn cell_initializer_array_annotation_diagnostics_still_surface() {
        // The initializer's own body diagnostics (here, a CEL typed-array annotation mismatch)
        // must still be reported even though the initializer isn't a bare literal.
        let sheet = parse("sheet s { cell x: f64 = [0, 1]: [f64]; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        let messages: Vec<&str> = diags.iter().map(cel_parser::ParseError::message).collect();
        assert_eq!(
            messages,
            vec!["array elements must match the annotation exactly: expected `f64`, found `i32`"]
        );
    }

    #[test]
    fn cell_with_only_an_annotation_has_nothing_to_cross_check() {
        let sheet = parse("sheet s { cell x: i32; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn cell_initializer_unsuffixed_int_literal_mismatched_with_a_declared_unsigned_type_is_a_diagnostic()
     {
        // adam_lang::parser's real cell_decl grammar requires the initializer's inferred type to
        // equal the declared type exactly, no int-width coercion. An unsuffixed integer literal
        // always infers as `i32`, so it mismatches a `u32` annotation; `cell x: u32 = 1;` is
        // rejected by the real parser, and the checker must flag it too.
        let sheet = parse("sheet s { cell x: u32 = 1; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn cell_initializer_unsuffixed_int_literal_mismatched_with_a_declared_float_type_is_a_diagnostic()
     {
        // Same exact-type rule as above, but across the int/float boundary: an unsuffixed integer
        // literal infers as `i32`, never as `f64`, so no int-to-float coercion is allowed either.
        let sheet = parse("sheet s { cell x: f64 = 1; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn cell_initializer_char_literal_against_any_registered_type_is_a_diagnostic() {
        // adam-lang's real parser has no rule accepting a char literal against any registered
        // type, so the checker must flag it too.
        let sheet = parse("sheet s { cell x: i32 = 'a'; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn cell_annotated_with_an_unregistered_type_name_is_never_flagged() {
        let sheet = parse("sheet s { cell x: WidgetHandle = 1; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn binding_single_output_matching_declared_type_has_no_diagnostic() {
        let sheet = parse(
            "sheet s { cell width: f64; cell height: f64; cell area: f64; \
             relationship { area := width * height; } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn binding_single_output_mismatched_with_declared_type_is_a_diagnostic() {
        let sheet = parse(
            "sheet s { cell width: f64; cell height: f64; cell area: i32; \
             relationship { area := width * height; } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn binding_multi_output_matching_tuple_has_no_diagnostic() {
        let sheet = parse(
            "sheet s { cell a: i32; cell b: i32; cell sum: i32; cell diff: i32; \
             relationship { (sum, diff) := (a + b, a - b); } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    /// Complete call-result shapes check arity and leaves without suppressing body errors.
    #[test]
    fn binding_multi_output_iife_defers_unknown_shape_but_checks_children() {
        for (body, expected) in [
            ("(|x: i32| (x, x))(1)", 0),
            ("(|x: i32| (x, true + x))(1)", 1),
            ("(|x: i32| (x, true))(1)", 1),
            ("(|x: i32| (x,))(1)", 1),
            ("1", 1),
            ("(1, 2, 3)", 1),
        ] {
            let sheet = parse(&format!(
                "sheet s {{ cell a: i32; cell b: i32; relationship {{ (a,b) := {body}; }} }}"
            ));
            let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
            assert_eq!(diags.len(), expected, "{body}: {diags:?}");
        }
    }

    /// Inferred tuple call results satisfy matching annotations and reject mismatched leaves.
    #[test]
    fn tuple_call_results_match_declared_shapes() {
        for (annotation, expected) in [("(i32, i32)", 0), ("(i32, bool)", 1)] {
            let sheet = parse(&format!(
                "sheet s {{ cell pair: {annotation} = (|x: i32| (x,x))(1); }}"
            ));
            let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
            assert_eq!(diags.len(), expected, "{annotation}: {diags:?}");
        }
    }

    #[test]
    fn binding_multi_output_arity_mismatch_is_a_diagnostic() {
        let sheet = parse(
            "sheet s { cell a: i32; cell b: i32; cell sum: i32; cell diff: i32; \
             relationship { (sum, diff) := a + b; } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn binding_single_output_with_a_tuple_shaped_body_is_a_diagnostic() {
        // Body is a 2-tuple but only 1 output is declared: `check_expr` would otherwise infer
        // `Ty::Any` for the tuple and let this slip through with no diagnostic at all.
        let sheet = parse(
            "sheet s { cell a: i32; cell b: i32; cell out: i32; \
             relationship { out := (a, b); } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn binding_multi_output_per_element_type_mismatch_is_a_diagnostic() {
        let sheet = parse(
            "sheet s { cell a: i32; cell b: i32; cell sum: i32; cell diff: f64; \
             relationship { (sum, diff) := (a + b, a - b); } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn an_operator_error_inside_a_binding_body_surfaces() {
        let sheet = parse(
            "sheet s { cell name: String; cell count: i32; cell out: i32; \
             relationship { out := name + count; } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn conditional_branch_and_default_bindings_are_both_checked() {
        let sheet = parse(
            "sheet s { cell mode: i32; cell a: i32; cell b: i32; cell out: i32; \
             conditional mode { \
                 0i32 => { relationship { out := a; } }, \
                 _ => { relationship { out := b; } }, \
             } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn a_cell_with_no_type_annotation_unifies_with_anything_used_in_a_binding() {
        // `cell a = 1;` has an initializer but no `: type_name` — declared_cell_types maps it to
        // Ty::Any, which must unify silently with `out`'s declared `i32`.
        let sheet = parse(
            "sheet s { cell a = 1; cell out: i32; \
             relationship { out := a; } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
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
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn out_body_matching_its_annotation_has_no_diagnostic() {
        let sheet = parse(
            "sheet s { cell width: f64; cell height: f64; \
             out area: f64 := width * height; }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn out_body_mismatched_with_its_annotation_is_a_diagnostic() {
        let sheet = parse(
            "sheet s { cell width: f64; cell height: f64; \
             out area: i32 := width * height; }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn out_with_no_annotation_infers_its_type_and_has_no_diagnostic() {
        // No `: type_name` to cross-check against — nothing to flag, and a later reference to
        // `area`'s name (were one added) would resolve through the inferred f64, not Ty::Any.
        let sheet = parse(
            "sheet s { cell width: f64; cell height: f64; \
             out area := width * height; }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn requirement_with_bool_body_has_no_diagnostic() {
        let sheet = parse(
            "sheet s { cell width: f64; cell max_width: f64; \
             out area: f64 := width require { \
                 @max_width width <= max_width; \
             }; }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn requirement_with_non_bool_body_is_a_diagnostic() {
        let sheet = parse(
            "sheet s { cell width: f64; \
             out area: f64 := width require { \
                 @bogus width; \
             }; }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn unlabeled_requirement_with_non_bool_body_is_a_diagnostic() {
        let sheet = parse(
            "sheet s { cell width: f64; \
             out area: f64 := width require { \
                 width; \
             }; }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn cell_tuple_initializer_matching_its_annotation_has_no_diagnostic() {
        let sheet = parse("sheet s { cell a: (i32, f64) = (1, 2.5); }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn cell_tuple_initializer_arity_mismatch_is_a_diagnostic() {
        let sheet = parse("sheet s { cell a: (i32, f64, i32) = (1, 2.5); }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn cell_tuple_initializer_element_type_mismatch_is_a_diagnostic() {
        let sheet = parse("sheet s { cell a: (i32, i32) = (1, 2.5); }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn cell_nested_tuple_initializer_matching_its_annotation_has_no_diagnostic() {
        let sheet = parse("sheet s { cell a: (i32, (f64, String)) = (1, (2.5, \"x\")); }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn filter_with_matching_types_has_no_diagnostic() {
        let sheet = parse("sheet s { cell a: i32 = 1 filter _; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn filter_with_matching_types_on_a_source_has_no_diagnostic() {
        let sheet = parse("sheet s { source a: i32 = 1 filter _; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn filter_referencing_a_cell_has_no_diagnostic() {
        let sheet = parse(
            "sheet s { cell hi: i32 = 100; cell a: i32 = 1 filter if _ > hi { hi } else { _ }; }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn filter_body_type_mismatch_is_a_diagnostic() {
        // Body is `bool`-typed (a comparison), but `a` is declared `i32`.
        let sheet = parse("sheet s { cell a: i32 = 1 filter _ > 0; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn filter_without_underscore_is_a_diagnostic() {
        let sheet = parse("sheet s { cell a: i32 = 1 filter 1; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    /// A closure parameter hides value references, not labels or type names.
    #[test]
    fn closure_parameter_shadows_only_value_references() {
        for (source, references) in [
            ("|_: i32| _", false),
            ("|x: i32| _", true),
            ("|_: i32| value._", false),
            ("|_: i32| _ { x: value }", false),
            ("|x: i32| Record { x: _ }.x", true),
        ] {
            let expr =
                cel_parser::Parser::<cel_parser::AstContext>::new(cel_parser::OpLookup::new())
                    .parse_str_ast(source)
                    .unwrap();
            assert_eq!(expr_references_ident(&expr, "_"), references, "{source}");
        }
    }

    /// Verifies record values contribute dependencies, while constructor and field labels do not.
    #[test]
    fn record_filter_dependencies_only_visit_field_values() {
        let sheet = parse("sheet s { cell a: i32 = 1 filter Record { value: _ }; }");
        let diagnostics = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message().contains("unknown record type"));
        for source in [
            "sheet s { cell a: i32 = 1 filter Record { _: 1 }; }",
            "sheet s { cell a: i32 = 1 filter _ { value: 1 }; }",
        ] {
            let sheet = parse(source);
            let diagnostics =
                check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
            assert_eq!(diagnostics.len(), 2);
            assert!(
                diagnostics
                    .iter()
                    .any(|d| d.message().contains("must reference `_`"))
            );
        }
    }

    #[test]
    fn filter_body_type_mismatch_on_an_out_is_a_diagnostic() {
        // Mirrors `filter_body_type_mismatch_is_a_diagnostic`, but for an `out`'s filter clause:
        // body is `bool`-typed (a comparison), but `area` is declared `i32`.
        let sheet = parse("sheet s { cell width: i32 = 1; out area: i32 := width filter _ > 0; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn filter_without_underscore_on_an_out_is_a_diagnostic() {
        // Mirrors `filter_without_underscore_is_a_diagnostic`, but for an `out`'s filter clause.
        let sheet = parse("sheet s { cell width: i32 = 1; out area: i32 := width filter 1; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn filter_body_type_mismatch_on_a_source_is_a_diagnostic() {
        // Mirrors `filter_body_type_mismatch_is_a_diagnostic`, but for a `source`'s filter
        // clause: body is `bool`-typed (a comparison), but `a` is declared `i32`.
        let sheet = parse("sheet s { source a: i32 = 1 filter _ > 0; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn filter_without_underscore_on_a_source_is_a_diagnostic() {
        // Mirrors `filter_without_underscore_is_a_diagnostic`, but for a `source`'s filter
        // clause.
        let sheet = parse("sheet s { source a: i32 = 1 filter 1; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn filter_on_a_tuple_typed_cell_is_a_diagnostic() {
        // Mirrors the runtime parser's own rejection (`adam_lang::parser::AdamParser::
        // parse_cell_filter`) — a tuple-typed filtered cell isn't yet supported by either layer.
        let sheet = parse("sheet s { cell a: (i32, f64) = (1, 2.5) filter (_.0, _.1); }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn filter_references_underscore_nested_inside_a_call_has_no_missing_underscore_diagnostic() {
        // `_` appears only inside an `if`'s then-branch, not as the whole body or a bare
        // operand — exercises `expr_references_ident`'s `Expr::If` arm specifically.
        let sheet = parse("sheet s { cell a: i32 = 1 filter if true { _ } else { 1 }; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    /// Verifies filter identifier walks visit receivers and arguments, but not member labels.
    #[test]
    fn member_filter_references_are_operands_not_labels() {
        for body in ["_.accessor", "value.accessor(_)", "_.accessor()"] {
            let sheet = parse(&format!("sheet s {{ cell value: i32 = 1 filter {body}; }}"));
            assert!(
                check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new()).is_empty(),
                "{body}"
            );
        }
        let sheet = parse("sheet s { cell value: i32 = 1 filter value._; }");
        assert_eq!(
            check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new()).len(),
            1
        );
    }

    #[test]
    fn binding_single_tuple_typed_output_matching_body_has_no_diagnostic() {
        let sheet = parse(
            "sheet s { cell a: i32; cell b: i32; cell pair: (i32, i32); \
             relationship { pair := (a, b); } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn binding_single_tuple_typed_output_element_type_mismatch_is_a_diagnostic() {
        let sheet = parse(
            "sheet s { cell a: i32; cell b: f64; cell pair: (i32, i32); \
             relationship { pair := (a, b); } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn binding_single_tuple_typed_output_if_else_body_matching_both_branches_has_no_diagnostic() {
        let sheet = parse(
            "sheet s { cell cond: bool; cell a: i32; cell b: i32; cell pair: (i32, i32); \
             relationship { pair := \
                 if cond { (a, b) } else { (b, a) }; \
             } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn binding_single_tuple_typed_output_if_without_else_matching_then_branch_has_no_diagnostic() {
        let sheet = parse(
            "sheet s { cell cond: bool; cell a: i32; cell b: i32; cell pair: (i32, i32); \
             relationship { pair := \
                 if cond { (a, b) }; \
             } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn binding_single_tuple_typed_output_if_else_if_chain_matching_all_branches_has_no_diagnostic()
    {
        let sheet = parse(
            "sheet s { cell mode: i32; cell a: i32; cell b: i32; cell pair: (i32, i32); \
             relationship { pair := \
                 if mode == 0 { (a, b) } else if mode == 1 { (b, a) } else { (0, 0) }; \
             } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn binding_single_tuple_typed_output_if_else_body_element_mismatch_in_each_branch_is_two_diagnostics()
     {
        // Both branches use the same mismatched `c`, so a correctly recursing checker reports one
        // diagnostic per branch (2 total). The old catch-all — which never recursed into an
        // `Expr::If` at all — could only ever report exactly 1 diagnostic for the whole node
        // regardless of how many branches were wrong, so `diags.len() == 2` distinguishes the fix
        // from the bug (unlike asserting `== 1`, which both the buggy and fixed behavior satisfy
        // when only one branch mismatches).
        let sheet = parse(
            "sheet s { cell cond: bool; cell a: i32; cell c: f64; cell pair: (i32, i32); \
             relationship { pair := \
                 if cond { (a, c) } else { (a, c) }; \
             } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 2);
    }

    #[test]
    fn binding_multi_output_if_else_body_matching_both_branches_has_no_diagnostic() {
        let sheet = parse(
            "sheet s { cell cond: bool; cell a: i32; cell b: i32; cell sum: i32; cell diff: i32; \
             relationship { (sum, diff) := \
                 if cond { (a + b, a - b) } else { (a - b, a + b) }; \
             } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn binding_multi_output_if_else_body_element_mismatch_in_a_branch_is_a_diagnostic() {
        let sheet = parse(
            "sheet s { cell cond: bool; cell a: i32; cell b: i32; cell c: f64; \
             cell sum: i32; cell diff: i32; \
             relationship { (sum, diff) := \
                 if cond { (a, b) } else { (a, c) }; \
             } }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn binding_single_element_tuple_destructure_has_no_diagnostic() {
        let sheet =
            parse("sheet s { cell a: i32; cell out: i32; relationship { (out,) := (a,); } }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn binding_single_element_tuple_destructure_type_mismatch_is_a_diagnostic() {
        let sheet =
            parse("sheet s { cell a: f64; cell out: i32; relationship { (out,) := (a,); } }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn binding_single_element_tuple_destructure_arity_mismatch_is_a_diagnostic() {
        let sheet = parse("sheet s { cell a: i32; cell out: i32; relationship { (out,) := a; } }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn filter_range_inclusive_body_does_not_require_underscore() {
        let sheet = parse("sheet s { cell a: i32 filter 0..=100; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
        assert!(diags.is_empty());
    }
}
