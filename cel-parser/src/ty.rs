//! A minimal static type model for the built-in primitives `pm_lang::TypeRegistry::new()`
//! registers by default, plus [`Ty::Any`] for everything else (custom host-registered types,
//! unannotated cells, unresolved identifiers). Used by [`check_expr`] to type-check
//! [`crate::Expr`] trees built by [`crate::AstContext`]. Not a complete type system — see the
//! design doc's "Type checking (v1)" section for what's deliberately out of scope.

use std::any::TypeId;

use crate::op_table::builtin_operand_types;
use crate::{Expr, ExprSpan, Literal, ParseError};

/// A static type: one of the built-in primitives, or [`Ty::Any`] for anything pm-lang/CEL's
/// extensible type system doesn't statically know about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    /// `i8`.
    I8,
    /// `i16`.
    I16,
    /// `i32`.
    I32,
    /// `i64`.
    I64,
    /// `i128`.
    I128,
    /// `isize`.
    Isize,
    /// `u8`.
    U8,
    /// `u16`.
    U16,
    /// `u32`.
    U32,
    /// `u64`.
    U64,
    /// `u128`.
    U128,
    /// `usize`.
    Usize,
    /// `f32`.
    F32,
    /// `f64`.
    F64,
    /// `bool`.
    Bool,
    /// `String`.
    String,
    /// Anything not statically known: a custom host-registered type, an unannotated cell, an
    /// unresolved identifier, or a node kind [`check_expr`] doesn't check directly (e.g. a tuple
    /// or call result). Unifies silently with every other `Ty`, in both directions.
    Any,
}

impl Ty {
    /// Maps a resolved [`Literal`] to its [`Ty`]. `Char`/`ByteStr`/`CStr`/`Unit` have no `Ty`
    /// variant and map to [`Ty::Any`] — not an error, matching this model's "unresolved falls
    /// back to `Any`" convention.
    pub fn from_literal(lit: &Literal) -> Ty {
        match lit {
            Literal::I8(_) => Ty::I8,
            Literal::I16(_) => Ty::I16,
            Literal::I32(_) => Ty::I32,
            Literal::I64(_) => Ty::I64,
            Literal::I128(_) => Ty::I128,
            Literal::Isize(_) => Ty::Isize,
            Literal::U8(_) => Ty::U8,
            Literal::U16(_) => Ty::U16,
            Literal::U32(_) => Ty::U32,
            Literal::U64(_) => Ty::U64,
            Literal::U128(_) => Ty::U128,
            Literal::Usize(_) => Ty::Usize,
            Literal::F32(_) => Ty::F32,
            Literal::F64(_) => Ty::F64,
            Literal::Bool(_) => Ty::Bool,
            Literal::Str(_) => Ty::String,
            Literal::Char(_) | Literal::ByteStr(_) | Literal::CStr(_) | Literal::Unit => Ty::Any,
        }
    }

    /// Maps a `TypeId` (e.g. from `pm_lang::TypeRegistry::TypeEntry::type_id`) to its [`Ty`].
    /// An unrecognized `TypeId` maps to [`Ty::Any`] — not an error, matching pm-lang/CEL's
    /// extensible type system (a host binary's custom registered types are invisible here).
    pub fn from_type_id(id: TypeId) -> Ty {
        if id == TypeId::of::<i8>() {
            Ty::I8
        } else if id == TypeId::of::<i16>() {
            Ty::I16
        } else if id == TypeId::of::<i32>() {
            Ty::I32
        } else if id == TypeId::of::<i64>() {
            Ty::I64
        } else if id == TypeId::of::<i128>() {
            Ty::I128
        } else if id == TypeId::of::<isize>() {
            Ty::Isize
        } else if id == TypeId::of::<u8>() {
            Ty::U8
        } else if id == TypeId::of::<u16>() {
            Ty::U16
        } else if id == TypeId::of::<u32>() {
            Ty::U32
        } else if id == TypeId::of::<u64>() {
            Ty::U64
        } else if id == TypeId::of::<u128>() {
            Ty::U128
        } else if id == TypeId::of::<usize>() {
            Ty::Usize
        } else if id == TypeId::of::<f32>() {
            Ty::F32
        } else if id == TypeId::of::<f64>() {
            Ty::F64
        } else if id == TypeId::of::<bool>() {
            Ty::Bool
        } else if id == TypeId::of::<String>() {
            Ty::String
        } else {
            Ty::Any
        }
    }

    /// Returns this type's `TypeId`, or `None` for [`Ty::Any`] (which has no single concrete
    /// Rust type).
    pub fn type_id(&self) -> Option<TypeId> {
        Some(match self {
            Ty::I8 => TypeId::of::<i8>(),
            Ty::I16 => TypeId::of::<i16>(),
            Ty::I32 => TypeId::of::<i32>(),
            Ty::I64 => TypeId::of::<i64>(),
            Ty::I128 => TypeId::of::<i128>(),
            Ty::Isize => TypeId::of::<isize>(),
            Ty::U8 => TypeId::of::<u8>(),
            Ty::U16 => TypeId::of::<u16>(),
            Ty::U32 => TypeId::of::<u32>(),
            Ty::U64 => TypeId::of::<u64>(),
            Ty::U128 => TypeId::of::<u128>(),
            Ty::Usize => TypeId::of::<usize>(),
            Ty::F32 => TypeId::of::<f32>(),
            Ty::F64 => TypeId::of::<f64>(),
            Ty::Bool => TypeId::of::<bool>(),
            Ty::String => TypeId::of::<String>(),
            Ty::Any => return None,
        })
    }

    /// A human-readable name for diagnostics (e.g. `"i32"`, `"<any>"`).
    pub fn name(&self) -> &'static str {
        match self {
            Ty::I8 => "i8",
            Ty::I16 => "i16",
            Ty::I32 => "i32",
            Ty::I64 => "i64",
            Ty::I128 => "i128",
            Ty::Isize => "isize",
            Ty::U8 => "u8",
            Ty::U16 => "u16",
            Ty::U32 => "u32",
            Ty::U64 => "u64",
            Ty::U128 => "u128",
            Ty::Usize => "usize",
            Ty::F32 => "f32",
            Ty::F64 => "f64",
            Ty::Bool => "bool",
            Ty::String => "String",
            Ty::Any => "<any>",
        }
    }

    /// Returns `true` if `self` and `other` are compatible: either is [`Ty::Any`], or they're
    /// equal. `Ty::Any` unifying silently with everything (in both directions) is the load-bearing
    /// property that lets unannotated cells and custom host types produce zero false-positive
    /// diagnostics.
    pub fn unifies_with(&self, other: &Ty) -> bool {
        matches!(self, Ty::Any) || matches!(other, Ty::Any) || self == other
    }
}

/// Infers `expr`'s type against `resolve_ident` (looks up a free identifier's declared type, or
/// `Ty::Any` if unknown — e.g. a bare CEL builtin name, or a pm-lang cell with no `: type`
/// annotation), returning the expression's inferred type plus every type diagnostic found.
///
/// Only [`Expr::Op`] (via [`builtin_operand_types`]) and [`Expr::Logical`] (CEL's fixed `&&`/`||`
/// semantics: both operands must unify with `bool`) are checked directly. [`Expr::Apply`],
/// [`Expr::Tuple`], [`Expr::TupleIndex`], and [`Expr::If`] are recursed into — so an `Op` nested
/// inside one is still checked — but the node itself always infers as [`Ty::Any`]: checking call
/// return types, tuple shapes, and if/else branch agreement is deferred to a later phase (see the
/// design doc's "Type checking (v1)" section).
///
/// - Complexity: O(n) in the number of nodes in `expr`.
pub fn check_expr(expr: &Expr, resolve_ident: &impl Fn(&str) -> Ty) -> (Ty, Vec<ParseError>) {
    match expr {
        Expr::Literal { value, .. } => (Ty::from_literal(value), Vec::new()),
        Expr::Ident { name, .. } => (resolve_ident(name), Vec::new()),
        Expr::Op {
            name,
            operands,
            span,
        } => check_op(name, operands, *span, resolve_ident),
        Expr::Logical { lhs, rhs, span, .. } => check_logical(lhs, rhs, *span, resolve_ident),
        Expr::Apply { callee, args, .. } => {
            let mut diagnostics = check_expr(callee, resolve_ident).1;
            for arg in args {
                diagnostics.extend(check_expr(arg, resolve_ident).1);
            }
            (Ty::Any, diagnostics)
        }
        Expr::Tuple { elements, .. } => {
            let mut diagnostics = Vec::new();
            for element in elements {
                diagnostics.extend(check_expr(element, resolve_ident).1);
            }
            (Ty::Any, diagnostics)
        }
        Expr::TupleIndex { base, .. } => (Ty::Any, check_expr(base, resolve_ident).1),
        Expr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            let mut diagnostics = check_expr(cond, resolve_ident).1;
            diagnostics.extend(check_expr(then_branch, resolve_ident).1);
            diagnostics.extend(check_expr(else_branch, resolve_ident).1);
            (Ty::Any, diagnostics)
        }
    }
}

/// Checks an [`Expr::Op`] node: infers each operand, then (only if every operand resolved to a
/// concrete type) matches them against [`builtin_operand_types`]. An operator
/// `builtin_operand_types` doesn't recognize at all (e.g. a tuple-shaped custom op registered
/// only at runtime) can't be checked here and infers as `Ty::Any` — not an error.
fn check_op(
    name: &str,
    operands: &[Expr],
    span: ExprSpan,
    resolve_ident: &impl Fn(&str) -> Ty,
) -> (Ty, Vec<ParseError>) {
    let mut diagnostics = Vec::new();
    let operand_tys: Vec<Ty> = operands
        .iter()
        .map(|operand| {
            let (ty, operand_diags) = check_expr(operand, resolve_ident);
            diagnostics.extend(operand_diags);
            ty
        })
        .collect();
    if operand_tys.contains(&Ty::Any) {
        return (Ty::Any, diagnostics);
    }
    let signatures = builtin_operand_types(name);
    if signatures.is_empty() {
        return (Ty::Any, diagnostics); // unregistered/custom operator: nothing to check
    }
    let matched = signatures.iter().find(|sig| {
        sig.arity as usize == operand_tys.len()
            && Some(sig.lhs) == operand_tys[0].type_id()
            && (operand_tys.len() < 2 || Some(sig.rhs) == operand_tys[1].type_id())
    });
    match matched {
        Some(_) => (result_ty_for_op(name, operand_tys[0]), diagnostics),
        None => {
            let described = operand_tys
                .iter()
                .map(Ty::name)
                .collect::<Vec<_>>()
                .join(", ");
            diagnostics.push(ParseError::new_range(
                format!("`{name}` is not defined for operand type(s) `{described}`"),
                span.start,
                span.end,
            ));
            (Ty::Any, diagnostics)
        }
    }
}

/// Returns the result type of a matched built-in operator application: `Ty::Bool` for the
/// comparison operators, otherwise the (matched, homogeneous) operand type — every built-in
/// signature is either a comparison (returning `bool`) or same-type-in-same-type-out (arithmetic,
/// bitwise, shifts, unary negation, logical not).
fn result_ty_for_op(name: &str, operand_ty: Ty) -> Ty {
    match name {
        "==" | "!=" | "<" | "<=" | ">" | ">=" => Ty::Bool,
        _ => operand_ty,
    }
}

/// Checks an [`Expr::Logical`] (`&&`/`||`) node: both operands should unify with `Ty::Bool` (CEL's
/// fixed short-circuit semantics, not table-driven like [`Expr::Op`]); the node's own type is
/// always `Ty::Bool` regardless of whether a diagnostic was recorded.
fn check_logical(
    lhs: &Expr,
    rhs: &Expr,
    span: ExprSpan,
    resolve_ident: &impl Fn(&str) -> Ty,
) -> (Ty, Vec<ParseError>) {
    let (lhs_ty, mut diagnostics) = check_expr(lhs, resolve_ident);
    let (rhs_ty, rhs_diags) = check_expr(rhs, resolve_ident);
    diagnostics.extend(rhs_diags);
    for (side, ty) in [("left", lhs_ty), ("right", rhs_ty)] {
        if !ty.unifies_with(&Ty::Bool) {
            diagnostics.push(ParseError::new_range(
                format!(
                    "`&&`/`||` requires `bool`, found `{}` on the {side}",
                    ty.name()
                ),
                span.start,
                span.end,
            ));
        }
    }
    (Ty::Bool, diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LogicalOp;

    #[test]
    fn from_literal_maps_every_concrete_variant() {
        assert_eq!(Ty::from_literal(&Literal::I8(1)), Ty::I8);
        assert_eq!(Ty::from_literal(&Literal::I16(1)), Ty::I16);
        assert_eq!(Ty::from_literal(&Literal::I32(1)), Ty::I32);
        assert_eq!(Ty::from_literal(&Literal::I64(1)), Ty::I64);
        assert_eq!(Ty::from_literal(&Literal::I128(1)), Ty::I128);
        assert_eq!(Ty::from_literal(&Literal::Isize(1)), Ty::Isize);
        assert_eq!(Ty::from_literal(&Literal::U8(1)), Ty::U8);
        assert_eq!(Ty::from_literal(&Literal::U16(1)), Ty::U16);
        assert_eq!(Ty::from_literal(&Literal::U32(1)), Ty::U32);
        assert_eq!(Ty::from_literal(&Literal::U64(1)), Ty::U64);
        assert_eq!(Ty::from_literal(&Literal::U128(1)), Ty::U128);
        assert_eq!(Ty::from_literal(&Literal::Usize(1)), Ty::Usize);
        assert_eq!(Ty::from_literal(&Literal::F32(1.0)), Ty::F32);
        assert_eq!(Ty::from_literal(&Literal::F64(1.0)), Ty::F64);
        assert_eq!(Ty::from_literal(&Literal::Bool(true)), Ty::Bool);
        assert_eq!(Ty::from_literal(&Literal::Str("s".to_string())), Ty::String);
    }

    #[test]
    fn from_literal_maps_unsupported_kinds_to_any() {
        assert_eq!(Ty::from_literal(&Literal::Char('a')), Ty::Any);
        assert_eq!(Ty::from_literal(&Literal::ByteStr(vec![1])), Ty::Any);
        assert_eq!(Ty::from_literal(&Literal::Unit), Ty::Any);
    }

    #[test]
    fn type_id_round_trips_through_from_type_id_for_every_concrete_variant() {
        for ty in [
            Ty::I8,
            Ty::I16,
            Ty::I32,
            Ty::I64,
            Ty::I128,
            Ty::Isize,
            Ty::U8,
            Ty::U16,
            Ty::U32,
            Ty::U64,
            Ty::U128,
            Ty::Usize,
            Ty::F32,
            Ty::F64,
            Ty::Bool,
            Ty::String,
        ] {
            let id = ty.type_id().expect("concrete Ty has a TypeId");
            assert_eq!(Ty::from_type_id(id), ty);
        }
    }

    #[test]
    fn any_has_no_type_id() {
        assert_eq!(Ty::Any.type_id(), None);
    }

    #[test]
    fn from_type_id_maps_an_unregistered_type_id_to_any() {
        assert_eq!(Ty::from_type_id(TypeId::of::<Vec<u8>>()), Ty::Any);
    }

    #[test]
    fn any_unifies_with_every_concrete_type_in_both_directions() {
        assert!(Ty::Any.unifies_with(&Ty::I32));
        assert!(Ty::I32.unifies_with(&Ty::Any));
        assert!(Ty::Any.unifies_with(&Ty::Any));
    }

    #[test]
    fn identical_concrete_types_unify() {
        assert!(Ty::F64.unifies_with(&Ty::F64));
    }

    #[test]
    fn distinct_concrete_types_do_not_unify() {
        assert!(!Ty::I32.unifies_with(&Ty::F64));
        assert!(!Ty::I32.unifies_with(&Ty::Bool));
    }

    #[test]
    fn name_is_distinct_per_type() {
        let names: Vec<&str> = [Ty::I32, Ty::F64, Ty::Bool, Ty::String, Ty::Any]
            .iter()
            .map(Ty::name)
            .collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            names.len(),
            unique.len(),
            "every listed Ty has a distinct name"
        );
    }

    fn any_resolver(_name: &str) -> Ty {
        Ty::Any
    }

    fn point(span: proc_macro2::Span) -> ExprSpan {
        ExprSpan {
            start: span,
            end: span,
        }
    }

    fn lit_i32(v: i32) -> Expr {
        Expr::Literal {
            value: Literal::I32(v),
            span: point(proc_macro2::Span::call_site()),
        }
    }

    fn lit_bool(v: bool) -> Expr {
        Expr::Literal {
            value: Literal::Bool(v),
            span: point(proc_macro2::Span::call_site()),
        }
    }

    fn lit_str(v: &str) -> Expr {
        Expr::Literal {
            value: Literal::Str(v.to_string()),
            span: point(proc_macro2::Span::call_site()),
        }
    }

    fn op(name: &str, operands: Vec<Expr>) -> Expr {
        Expr::Op {
            name: name.to_string(),
            operands,
            span: point(proc_macro2::Span::call_site()),
        }
    }

    #[test]
    fn literal_infers_its_own_type() {
        let (ty, diags) = check_expr(&lit_i32(1), &any_resolver);
        assert_eq!(ty, Ty::I32);
        assert!(diags.is_empty());
    }

    #[test]
    fn ident_resolves_via_the_supplied_resolver() {
        let expr = Expr::Ident {
            name: "width".to_string(),
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&expr, &|name| {
            if name == "width" { Ty::F64 } else { Ty::Any }
        });
        assert_eq!(ty, Ty::F64);
        assert!(diags.is_empty());
    }

    #[test]
    fn unknown_ident_is_any_and_not_a_diagnostic() {
        let expr = Expr::Ident {
            name: "mystery".to_string(),
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Any);
        assert!(diags.is_empty());
    }

    #[test]
    fn op_with_matching_signature_infers_the_operand_type() {
        let expr = op("+", vec![lit_i32(1), lit_i32(2)]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::I32);
        assert!(diags.is_empty());
    }

    #[test]
    fn comparison_op_always_infers_bool() {
        let expr = op("==", vec![lit_i32(1), lit_i32(2)]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Bool);
        assert!(diags.is_empty());
    }

    #[test]
    fn unary_negation_preserves_the_operand_type() {
        let expr = op("-", vec![lit_i32(1)]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::I32);
        assert!(diags.is_empty());
    }

    #[test]
    fn op_with_mismatched_operand_types_produces_one_diagnostic_and_infers_any() {
        let expr = op("+", vec![lit_i32(1), lit_str("s")]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Any);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn op_with_an_any_operand_produces_no_diagnostic() {
        let expr = op(
            "+",
            vec![
                Expr::Ident {
                    name: "mystery".to_string(),
                    span: point(proc_macro2::Span::call_site()),
                },
                lit_i32(1),
            ],
        );
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Any);
        assert!(diags.is_empty());
    }

    #[test]
    fn unregistered_operator_name_is_any_and_not_a_diagnostic() {
        // "greet" is only ever registered at runtime via OpLookup::register_tuple_op; the static
        // checker can't see it and must not guess.
        let expr = op("greet", vec![lit_i32(1)]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Any);
        assert!(diags.is_empty());
    }

    #[test]
    fn logical_and_with_bool_operands_infers_bool_with_no_diagnostic() {
        let expr = Expr::Logical {
            op: LogicalOp::And,
            lhs: Box::new(lit_bool(true)),
            rhs: Box::new(lit_bool(false)),
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Bool);
        assert!(diags.is_empty());
    }

    #[test]
    fn logical_or_with_a_non_bool_operand_produces_a_diagnostic_but_still_infers_bool() {
        let expr = Expr::Logical {
            op: LogicalOp::Or,
            lhs: Box::new(lit_i32(1)),
            rhs: Box::new(lit_bool(true)),
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Bool);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn a_broken_op_nested_inside_a_tuple_still_surfaces_a_diagnostic() {
        let expr = Expr::Tuple {
            elements: vec![op("+", vec![lit_i32(1), lit_str("s")]), lit_i32(2)],
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Any, "Tuple itself is not type-checked in v1");
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn a_broken_op_nested_inside_an_if_condition_still_surfaces_a_diagnostic() {
        let expr = Expr::If {
            cond: Box::new(op("+", vec![lit_i32(1), lit_str("s")])),
            then_branch: Box::new(lit_i32(1)),
            else_branch: Box::new(lit_i32(2)),
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Any, "If itself is not type-checked in v1");
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn a_broken_op_nested_inside_a_call_argument_still_surfaces_a_diagnostic() {
        let expr = Expr::Apply {
            callee: Box::new(Expr::Ident {
                name: "f".to_string(),
                span: point(proc_macro2::Span::call_site()),
            }),
            args: vec![op("+", vec![lit_i32(1), lit_str("s")])],
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Any, "Apply itself is not type-checked in v1");
        assert_eq!(diags.len(), 1);
    }
}
