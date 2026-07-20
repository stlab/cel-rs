# pm-lang Type Checking (v1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give pm-lang a minimal, best-effort static type checker over `AstContext`-built trees — the design doc's "Type checking (v1)" section — so Phase 3's `pm-lsp` (a separate, follow-on plan) has type diagnostics to surface alongside `PmAstParser`'s existing syntax-error recovery.

**Architecture:** This is sub-plan 1 of 3 for the design doc's Phase 3 (`docs/superpowers/specs/2026-07-17-pm-lang-language-server-design.md`), split out per the writing-plans "Scope Check" guidance rather than one giant plan covering the type checker, the new `pm-lsp` crate, and the VS Code extension together — each is independently valuable and independently testable. This plan touches only `cel-parser` and `pm-lang`; no new crate, no LSP wiring.

A minimal `Ty` enum (the built-in primitives `TypeRegistry::new()` registers by default, plus `Ty::Any`) lives in `cel-parser` alongside the `Expr` tree it types. The existing runtime operator-dispatch tables in `cel_parser::op_table` (`OpSignature`/`BUILTINS`/`LEFT_SHIFT_SIGNATURES`/`RIGHT_SHIFT_SIGNATURES`) already encode exactly the operand-type combinations each built-in operator accepts; rather than hand-duplicating that knowledge into a second table the checker reads, this plan adds one new pub function, `op_table::builtin_operand_types(name) -> Vec<OperandTypes>`, that projects the *same* static arrays `BuiltinScope::lookup` dispatches against into a checker-usable `(arity, lhs TypeId, rhs TypeId)` shape (never exposing `op_fn`, which is execution-only). The result type for a matched overload doesn't need its own stored field: every built-in operator is either a comparison (`==`,`!=`,`<`,`<=`,`>`,`>=`, always returning `bool`) or same-type-in-same-type-out (arithmetic, bitwise, shifts, unary negation, logical not) — a fixed, two-way classification by operator name, not per-entry metadata that could drift from the dispatch table.

`cel_parser::ty::check_expr` walks an `Expr` tree bottom-up, given a caller-supplied `resolve_ident: impl Fn(&str) -> Ty` (pm-lang plugs in cell-name → declared-type lookups; a bare CEL consumer could plug in `|_| Ty::Any` to type-check nothing but still get the API). Only `Expr::Op` (via `builtin_operand_types`) and `Expr::Logical` (fixed `&&`/`||` semantics: both operands must unify with `bool`) are checked directly; `Expr::Apply`, `Expr::Tuple`, `Expr::TupleIndex`, and `Expr::If` are recursed into (so a broken `Op` nested inside one is still caught) but the node itself always infers as `Ty::Any` — checking call return types, tuple shapes, and if/else branch agreement is explicitly deferred, matching the design doc's "not a complete type system" framing.

`pm_lang::typecheck::check_sheet` is the pm-lang-specific layer on top: it resolves each `cell`'s declared type (via `TypeRegistry`), builds the `resolve_ident` closure from that map, and for every `relationship`/`conditional` method calls `cel_parser::ty::check_expr` on the body, then separately checks the method's declared output count against the body's actual shape (a bare expression for one output, an `n`-element `Expr::Tuple` for `n` outputs) and each output's declared type against the corresponding inferred type. It also checks each `cell`'s literal initializer (an unresolved `syn::Lit`/`cel_parser::lex_lexer::Literal`, needing pm-lang's own suffix-defaulting convention — unsuffixed integer → `i32`, unsuffixed float → `f64`, mirroring `pm_lang::parser`'s existing private `infer_and_parse_literal`) against its declared type annotation. `Ty::Any` (an absent annotation, an annotation naming a type `TypeRegistry` doesn't recognize, or an unregistered/custom operator `builtin_operand_types` doesn't recognize) unifies silently in both directions — never a diagnostic — matching pm-lang/CEL's extensible type system.

Diagnostics reuse `cel_parser::ParseError` (message + span), the same type `pm_lang::ast::Sheet::errors` already uses for syntax errors — no new diagnostic type, and `pm-lsp` (the next sub-plan) can render both kinds identically.

**Tech Stack:** Rust, existing `cel-parser`/`pm-lang` crates. No new dependencies (pm-lang already depends on `syn` directly for literal parsing).

## Global Constraints

- `cargo fmt --all` before every commit (enforced by pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` must pass (this plan
  never touches `begin`; its two `begin`-specific clippy invocations aren't relevant here but cost
  nothing to run).
- Never commit directly to `main`; this work happens on the current worktree branch.
- Doc comments follow the project's contract style (Summary / Preconditions / Postconditions /
  Complexity) — see `CLAUDE.md`.
- Tests are derived from the contract and public interface only — never from the implementation.
- This plan is purely additive: no existing `cel-parser` or `pm-lang` test's behavior changes.
- `Ty::Any` must unify silently (no diagnostic) with every other `Ty` in both directions, and with
  itself — this is a hard correctness property, not a heuristic; get it wrong and every
  unannotated cell/unresolved identifier starts producing false-positive diagnostics.

---

### Task 1: `cel_parser::Ty` — the minimal type model

**Files:**
- Create: `cel-parser/src/ty.rs`
- Modify: `cel-parser/src/lib.rs` (add `pub mod ty;` and re-export `Ty`)

**Interfaces:**
- Consumes: `cel_parser::Literal` (the resolved `Expr::Literal` payload enum, already shipped —
  **not** `cel_parser::lex_lexer::Literal`/`syn::Lit`, the unresolved raw literal type pm-lang's
  `CellDecl`/`ConditionalBranch` use; that one is handled separately in Task 4, inside `pm-lang`,
  since only pm-lang's AST uses it).
- Produces (used by Task 3, Task 4): `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Ty {
  I8, I16, I32, I64, I128, Isize, U8, U16, U32, U64, U128, Usize, F32, F64, Bool, String, Any }`
  with `pub fn from_literal(lit: &Literal) -> Ty`, `pub fn from_type_id(id: TypeId) -> Ty`, `pub fn
  type_id(&self) -> Option<TypeId>` (`None` only for `Any`), `pub fn name(&self) -> &'static str`,
  `pub fn unifies_with(&self, other: &Ty) -> bool`.

- [ ] **Step 1: Write the failing tests**

Create `cel-parser/src/ty.rs` with only this content (the types don't exist yet — this is the
intended failing state):

```rust
//! A minimal static type model for the built-in primitives `pm_lang::TypeRegistry::new()`
//! registers by default, plus [`Ty::Any`] for everything else (custom host-registered types,
//! unannotated cells, unresolved identifiers). Used by [`check_expr`] to type-check
//! [`crate::Expr`] trees built by [`crate::AstContext`]. Not a complete type system — see the
//! design doc's "Type checking (v1)" section for what's deliberately out of scope.

use std::any::TypeId;

use crate::Literal;

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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(names.len(), unique.len(), "every listed Ty has a distinct name");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test -p cel-parser ty::`
Expected: compile errors — `no function or associated item named \`from_literal\`` (and similarly
for the other missing items) — since `Ty` has no methods yet.

- [ ] **Step 3: Implement `Ty`'s methods**

Add this content **above** the `#[cfg(test)] mod tests { ... }` block already in
`cel-parser/src/ty.rs` (the module doc comment, imports, and enum from Step 1 stay where they are):

```rust
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser ty::`
Expected: all 9 tests pass.

- [ ] **Step 5: Wire the new module into `cel-parser/src/lib.rs`**

Find:

```rust
pub mod ast;
mod error;
pub mod lex_lexer;
pub mod op_table;
pub mod parser_context;

pub use ast::{AstContext, Expr, ExprSpan, Literal, LogicalOp};
pub use error::{CELError, FormatRustcStyle, ParseError, SourceSpan, SpanContext};
pub use op_table::OpLookup;
pub use parser_context::{DynSegmentContext, ParserContext};
pub use proc_macro2::LineColumn;
```

Replace with:

```rust
pub mod ast;
mod error;
pub mod lex_lexer;
pub mod op_table;
pub mod parser_context;
pub mod ty;

pub use ast::{AstContext, Expr, ExprSpan, Literal, LogicalOp};
pub use error::{CELError, FormatRustcStyle, ParseError, SourceSpan, SpanContext};
pub use op_table::{OpLookup, OperandTypes, builtin_operand_types};
pub use parser_context::{DynSegmentContext, ParserContext};
pub use proc_macro2::LineColumn;
pub use ty::Ty;
```

(`OperandTypes`/`builtin_operand_types` don't exist yet — that's Task 2, next. This edit is written
now so Task 2 doesn't need to touch this block again.)

- [ ] **Step 6: Confirm it doesn't build yet (expected — Task 2 isn't done)**

Run: `cargo build -p cel-parser 2>&1 | grep OperandTypes`
Expected: an error naming `OperandTypes`/`builtin_operand_types` as unresolved — confirms Step 5's
edit is exactly the shape Task 2 needs to complete. Do not attempt to fix this in this task.

- [ ] **Step 7: Commit**

```bash
git add cel-parser/src/ty.rs cel-parser/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(cel-parser): add Ty, a minimal static type model

I8..String plus Any, with Literal/TypeId conversions and a unifies_with
predicate where Any unifies silently with everything in both
directions. First piece of the design doc's Type checking (v1); Task 2
adds the op_table lookup this depends on transitively via lib.rs's
export list (this commit alone does not build cel-parser standalone —
see the plan for why that's expected here).
EOF
)"
```

---

### Task 2: `cel_parser::op_table::{OperandTypes, builtin_operand_types}`

**Files:**
- Modify: `cel-parser/src/op_table.rs`

**Interfaces:**
- Consumes: the existing private `OpSignature`/`BUILTINS`/`LEFT_SHIFT_SIGNATURES`/
  `RIGHT_SHIFT_SIGNATURES` (already shipped, unchanged).
- Produces (used by Task 3): `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct
  OperandTypes { pub arity: u8, pub lhs: TypeId, pub rhs: TypeId }` and `pub fn
  builtin_operand_types(name: &str) -> Vec<OperandTypes>`.

- [ ] **Step 1: Write the failing tests**

Find the end of `cel-parser/src/op_table.rs` (the last test in the file):

```rust
        assert!(
            result.is_err(),
            "empty-shape tuple signature must not match a non-tuple operand"
        );
    }
}
```

Replace with (appending new tests before the closing brace; the existing test above is unchanged):

```rust
        assert!(
            result.is_err(),
            "empty-shape tuple signature must not match a non-tuple operand"
        );
    }

    #[test]
    fn builtin_operand_types_reports_a_binary_arithmetic_overload() {
        let sigs = builtin_operand_types("+");
        assert!(
            sigs.iter()
                .any(|s| s.arity == 2 && s.lhs == TypeId::of::<i32>() && s.rhs == TypeId::of::<i32>())
        );
    }

    #[test]
    fn builtin_operand_types_reports_a_unary_overload() {
        let sigs = builtin_operand_types("!");
        assert!(
            sigs.iter()
                .any(|s| s.arity == 1 && s.lhs == TypeId::of::<bool>())
        );
    }

    #[test]
    fn builtin_operand_types_includes_unary_negation_but_only_for_signed_and_float_types() {
        let sigs = builtin_operand_types("-");
        assert!(sigs.iter().any(|s| s.arity == 1 && s.lhs == TypeId::of::<i32>()));
        assert!(sigs.iter().any(|s| s.arity == 2 && s.lhs == TypeId::of::<i32>()));
        assert!(
            !sigs
                .iter()
                .any(|s| s.arity == 1 && s.lhs == TypeId::of::<u32>()),
            "unsigned types have no unary negation overload"
        );
    }

    #[test]
    fn builtin_operand_types_covers_heterogeneous_shift_signatures() {
        let sigs = builtin_operand_types("<<");
        assert!(
            sigs.iter()
                .any(|s| s.arity == 2 && s.lhs == TypeId::of::<u64>() && s.rhs == TypeId::of::<u32>())
        );
    }

    #[test]
    fn builtin_operand_types_is_empty_for_an_unregistered_name() {
        assert!(builtin_operand_types("not_an_operator").is_empty());
    }

    #[test]
    fn builtin_operand_types_is_empty_for_a_runtime_only_tuple_op() {
        // Tuple-shaped ops are registered on an OpLookup instance at runtime
        // (OpLookup::register_tuple_op), never in the static BUILTINS table this function reads —
        // confirming they're invisible here, not an oversight.
        assert!(builtin_operand_types("greet").is_empty());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test -p cel-parser op_table::`
Expected: `cannot find function \`builtin_operand_types\`` — it doesn't exist yet.

- [ ] **Step 3: Implement `OperandTypes` and `builtin_operand_types`**

Find:

```rust
/// Compile-time perfect hash map for built-in operations.
///
/// Maps operator symbols to their signature arrays for O(1) lookup.
static BUILTINS: phf::Map<&'static str, &'static [OpSignature]> = phf_map! {
    "+" => ADD_SIGNATURES,
    "-" => SUB_SIGNATURES,
    "*" => MUL_SIGNATURES,
    "/" => DIV_SIGNATURES,
    "%" => MOD_SIGNATURES,
    "&" => BITWISE_AND_SIGNATURES,
    "|" => BITWISE_OR_SIGNATURES,
    "^" => BITWISE_XOR_SIGNATURES,
    "!" => LOGICAL_NOT_SIGNATURES,
    "==" => EQUAL_SIGNATURES,
    "!=" => NOT_EQUAL_SIGNATURES,
    "<" => LESS_THAN_SIGNATURES,
    "<=" => LESS_THAN_OR_EQUAL_SIGNATURES,
    ">" => GREATER_THAN_SIGNATURES,
    ">=" => GREATER_THAN_OR_EQUAL_SIGNATURES,
};

/// Built-in operation scope.
///
/// Provides lookup for standard operations using a compile-time hash table.
struct BuiltinScope;
```

Replace with:

```rust
/// Compile-time perfect hash map for built-in operations.
///
/// Maps operator symbols to their signature arrays for O(1) lookup.
static BUILTINS: phf::Map<&'static str, &'static [OpSignature]> = phf_map! {
    "+" => ADD_SIGNATURES,
    "-" => SUB_SIGNATURES,
    "*" => MUL_SIGNATURES,
    "/" => DIV_SIGNATURES,
    "%" => MOD_SIGNATURES,
    "&" => BITWISE_AND_SIGNATURES,
    "|" => BITWISE_OR_SIGNATURES,
    "^" => BITWISE_XOR_SIGNATURES,
    "!" => LOGICAL_NOT_SIGNATURES,
    "==" => EQUAL_SIGNATURES,
    "!=" => NOT_EQUAL_SIGNATURES,
    "<" => LESS_THAN_SIGNATURES,
    "<=" => LESS_THAN_OR_EQUAL_SIGNATURES,
    ">" => GREATER_THAN_SIGNATURES,
    ">=" => GREATER_THAN_OR_EQUAL_SIGNATURES,
};

/// A single built-in overload's declared operand types, exposed for the static type checker
/// (`cel_parser::ty::check_expr`) — never `op_fn`, which is execution-only and stays private.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperandTypes {
    /// Number of operands this overload accepts (1 or 2).
    pub arity: u8,
    /// The LHS (or sole, for arity 1) operand's `TypeId`.
    pub lhs: TypeId,
    /// The RHS operand's `TypeId`; equal to `lhs` for a homogeneous or arity-1 overload.
    pub rhs: TypeId,
}

/// Returns every built-in overload's declared operand types for `name`, in registration order.
///
/// Reads the exact same static signature tables [`BuiltinScope::lookup`] dispatches against, so
/// the static type checker and the runtime dispatcher share one source of truth and can't drift
/// apart on which operand-type combinations a built-in operator accepts.
///
/// - Postcondition: returns an empty `Vec` if `name` names no built-in operator. A custom scope
///   registered via [`OpLookup::push_scope`] or a tuple-shaped op registered via
///   [`OpLookup::register_tuple_op`] is runtime-only (attached to one `OpLookup` instance, not
///   this module's static tables) and is not visible here — the type checker treats such an
///   operator as unchecked, not an error.
///
/// - Complexity: O(s) where s is the number of overloads registered for `name`.
///
/// # Examples
///
/// ```rust
/// use cel_parser::op_table::builtin_operand_types;
///
/// assert!(builtin_operand_types("+").iter().any(|sig| sig.arity == 2));
/// assert!(builtin_operand_types("not_an_operator").is_empty());
/// ```
pub fn builtin_operand_types(name: &str) -> Vec<OperandTypes> {
    let signatures: &[OpSignature] = match name {
        "<<" => &LEFT_SHIFT_SIGNATURES,
        ">>" => &RIGHT_SHIFT_SIGNATURES,
        _ => match BUILTINS.get(name) {
            Some(s) => s,
            None => return Vec::new(),
        },
    };
    signatures
        .iter()
        .map(|sig| OperandTypes {
            arity: sig.arity,
            lhs: sig.lhs_type_id(),
            rhs: sig.rhs_type_id(),
        })
        .collect()
}

/// Built-in operation scope.
///
/// Provides lookup for standard operations using a compile-time hash table.
struct BuiltinScope;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser op_table::`
Expected: all new tests pass, plus every pre-existing `op_table` test still passes unchanged.

- [ ] **Step 5: Build the whole crate to confirm Task 1's export list now resolves**

Run: `cargo build -p cel-parser`
Expected: succeeds — `cel-parser/src/lib.rs`'s `pub use op_table::{OpLookup, OperandTypes,
builtin_operand_types};` (added in Task 1, Step 5) now compiles.

- [ ] **Step 6: Run the full existing `cel-parser` test suite to confirm zero regressions**

Run: `cargo test -p cel-parser`
Expected: every pre-existing test passes unchanged, plus Task 1's and this task's new tests.

- [ ] **Step 7: Format, lint, and doc-test**

Run:
```bash
cargo fmt --all
cargo test --doc -p cel-parser
cargo clippy -p cel-parser --all-targets -- -D warnings
```
Expected: `cargo fmt` makes no further changes beyond what it applies; doc tests (including
Task 1's `builtin_operand_types` example) pass; zero clippy warnings.

- [ ] **Step 8: Commit**

```bash
git add cel-parser/src/op_table.rs
git commit -m "$(cat <<'EOF'
feat(cel-parser): expose builtin_operand_types for the static type checker

Projects the existing runtime operator-dispatch tables (BUILTINS,
LEFT_SHIFT_SIGNATURES, RIGHT_SHIFT_SIGNATURES) into a checker-usable
(arity, lhs TypeId, rhs TypeId) shape, never exposing the execution-only
op_fn. The static checker (next commit) and the runtime dispatcher now
read one shared source of truth instead of a second, hand-duplicated
table that could drift from it.
EOF
)"
```

---

### Task 3: `cel_parser::ty::check_expr` — the `Expr` type checker

**Files:**
- Modify: `cel-parser/src/ty.rs` (append; Task 1's content is unchanged)

**Interfaces:**
- Consumes: `crate::{Expr, ExprSpan, LogicalOp, ParseError}` (already shipped), `crate::op_table::
  builtin_operand_types` (Task 2), `Ty` (Task 1).
- Produces (used by Task 4): `pub fn check_expr(expr: &Expr, resolve_ident: &impl Fn(&str) -> Ty)
  -> (Ty, Vec<ParseError>)`.

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests { ... }` block already in `cel-parser/src/ty.rs` (append
before the final closing `}`; every existing test from Task 1 stays unchanged):

Find:

```rust
    #[test]
    fn name_is_distinct_per_type() {
        let names: Vec<&str> = [Ty::I32, Ty::F64, Ty::Bool, Ty::String, Ty::Any]
            .iter()
            .map(Ty::name)
            .collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(names.len(), unique.len(), "every listed Ty has a distinct name");
    }
}
```

Replace with:

```rust
    #[test]
    fn name_is_distinct_per_type() {
        let names: Vec<&str> = [Ty::I32, Ty::F64, Ty::Bool, Ty::String, Ty::Any]
            .iter()
            .map(Ty::name)
            .collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(names.len(), unique.len(), "every listed Ty has a distinct name");
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
        let (ty, diags) = check_expr(&expr, &|name| if name == "width" { Ty::F64 } else { Ty::Any });
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
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test -p cel-parser ty::`
Expected: `cannot find function \`check_expr\`` — it doesn't exist yet.

- [ ] **Step 3: Implement `check_expr`**

Add this content **above** the `#[cfg(test)] mod tests { ... }` block in `cel-parser/src/ty.rs`
(after Task 1's `impl Ty { ... }` block; the module doc comment, imports, enum, and impl from
Task 1 stay where they are). First, update the `use` line:

Find:

```rust
use std::any::TypeId;

use crate::Literal;
```

Replace with:

```rust
use std::any::TypeId;

use crate::op_table::builtin_operand_types;
use crate::{Expr, ExprSpan, Literal, LogicalOp, ParseError};
```

Then, after the closing `}` of `impl Ty { ... }`, add:

```rust
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
    if operand_tys.iter().any(|ty| *ty == Ty::Any) {
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
                format!("`&&`/`||` requires `bool`, found `{}` on the {side}", ty.name()),
                span.start,
                span.end,
            ));
        }
    }
    (Ty::Bool, diagnostics)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser ty::`
Expected: all tests pass (Task 1's 9 plus this task's 15).

- [ ] **Step 5: Run the full existing `cel-parser` test suite to confirm zero regressions**

Run: `cargo test -p cel-parser`
Expected: every pre-existing test passes unchanged.

- [ ] **Step 6: Format and lint**

Run:
```bash
cargo fmt --all
cargo clippy -p cel-parser --all-targets -- -D warnings
```
Expected: `cargo fmt` makes no further changes beyond what it applies; zero clippy warnings.

- [ ] **Step 7: Commit**

```bash
git add cel-parser/src/ty.rs
git commit -m "$(cat <<'EOF'
feat(cel-parser): add check_expr, the static Expr type checker

Checks Expr::Op (via builtin_operand_types) and Expr::Logical (fixed
&&/|| bool semantics) directly; recurses into Apply/Tuple/TupleIndex/If
so a nested Op error still surfaces, but infers those nodes themselves
as Ty::Any — checking call/tuple/if shapes is deferred. Ty::Any silently
unifies with everything, so unresolved identifiers and unregistered
custom operators never produce false-positive diagnostics.
EOF
)"
```

---

### Task 4: `pm_lang::typecheck::check_sheet`

**Files:**
- Create: `pm-lang/src/typecheck.rs`
- Modify: `pm-lang/src/lib.rs` (add `mod typecheck;` and `pub use typecheck::check_sheet;`)

**Interfaces:**
- Consumes: `cel_parser::{ty::check_expr, Ty, ParseError, Expr}` (Tasks 1–3), `crate::ast::{Sheet,
  SheetItem, CellDecl, MethodDecl}` (already shipped), `crate::TypeRegistry` (already shipped,
  `TypeEntry::type_id`/`TypeRegistry::get`).
- Produces (this plan's final deliverable, consumed by the follow-on `pm-lsp` plan): `pub fn
  check_sheet(sheet: &ast::Sheet, registry: &TypeRegistry) -> Vec<ParseError>`.

- [ ] **Step 1: Write the failing tests**

Create `pm-lang/src/typecheck.rs` with only this content (the function doesn't exist yet — this is
the intended failing state):

```rust
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
        let sheet = parse("sheet s { cell good: i32 = 1; cell bad unknown_syntax cell after: i32 = 2; }");
        assert!(!sheet.errors.is_empty(), "fixture must actually recover an error item");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test -p pm-lang typecheck::`
Expected: `cannot find function \`check_sheet\`` — it doesn't exist yet.

- [ ] **Step 3: Implement `check_sheet`**

Add this content **above** the `#[cfg(test)] mod tests { ... }` block already in
`pm-lang/src/typecheck.rs` (the module doc comment and imports from Step 1 stay where they are):

```rust
/// Checks `sheet` against `registry`'s registered types, returning every type diagnostic found.
/// Never fails — an unrecognized annotation, an unresolved identifier, or a custom operator
/// [`cel_parser::op_table::builtin_operand_types`] doesn't know about all resolve to
/// [`cel_parser::Ty::Any`] and are silently skipped, not reported.
///
/// - Complexity: O(n) in the number of nodes across every item in `sheet`.
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
fn declared_cell_types(sheet: &Sheet, registry: &TypeRegistry) -> std::collections::HashMap<String, Ty> {
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

/// Infers a pm-lang literal's type via `registry`'s suffix-defaulting convention (unsuffixed
/// integer → `i32`, unsuffixed float → `f64`, mirroring `pm_lang::parser`'s existing
/// `infer_and_parse_literal`), falling back to `Ty::Any` for an unregistered suffix or an
/// unsupported literal kind (char/byte string/C string) — not an error.
fn ty_of_lex_literal(lit: &LexLiteral, registry: &TypeRegistry) -> Ty {
    use syn::Lit;
    let type_name = match lit {
        Lit::Int(i) if i.suffix().is_empty() => "i32",
        Lit::Int(i) => i.suffix(),
        Lit::Float(f) if f.suffix().is_empty() => "f64",
        Lit::Float(f) => f.suffix(),
        Lit::Bool(_) => "bool",
        Lit::Str(_) => "String",
        _ => return Ty::Any,
    };
    registry
        .get(type_name)
        .map(|entry| Ty::from_type_id(entry.type_id))
        .unwrap_or(Ty::Any)
}

/// Checks one `cell`'s literal initializer against its `: type_name` annotation. A no-op if either
/// half is absent, or if the annotation names a type `registry` doesn't recognize.
fn check_cell_initializer(cell: &CellDecl, registry: &TypeRegistry, diagnostics: &mut Vec<ParseError>) {
    let (Some((type_name, _)), Some((literal, lit_span))) = (&cell.type_name, &cell.initializer)
    else {
        return;
    };
    let Some(entry) = registry.get(type_name) else {
        return;
    };
    let declared = Ty::from_type_id(entry.type_id);
    let actual = ty_of_lex_literal(literal, registry);
    if !declared.unifies_with(&actual) {
        diagnostics.push(ParseError::new(
            format!("expected `{}`, found `{}`", declared.name(), actual.name()),
            lit_span.start,
        ));
    }
}

/// Checks one `method`'s body against its declared outputs: for a single output, the body's
/// inferred type must unify with that output cell's declared type; for `n > 1` outputs, the body
/// must be an `n`-element tuple, checked element-wise against each output cell. Operator-level
/// diagnostics from inside the body (via [`check_expr`]) are always included exactly once,
/// regardless of which branch below runs.
fn check_method(method: &MethodDecl, resolve: &impl Fn(&str) -> Ty, diagnostics: &mut Vec<ParseError>) {
    match method.outputs.as_slice() {
        [] => {
            let (_, body_diags) = check_expr(&method.body, resolve);
            diagnostics.extend(body_diags);
        }
        [(name, _)] => {
            let (body_ty, body_diags) = check_expr(&method.body, resolve);
            diagnostics.extend(body_diags);
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pm-lang typecheck::`
Expected: all 13 tests pass.

- [ ] **Step 5: Wire the new module into `pm-lang/src/lib.rs`**

Find:

```rust
pub mod ast;
mod ast_parser;
mod parser;
mod token_cursor;
mod trivia;
pub mod type_registry;

// pm-lang reuses cel_parser::ParseError directly; no new error type is introduced.
// All parse errors carry a proc_macro2::Span for source-location diagnostics.
pub use ast_parser::PmAstParser;
pub use cel_parser::ParseError;
pub use parser::{ParsedSheet, PmParser};
pub use trivia::attach_trivia;
pub use type_registry::TypeRegistry;
```

Replace with:

```rust
pub mod ast;
mod ast_parser;
mod parser;
mod token_cursor;
mod trivia;
mod typecheck;
pub mod type_registry;

// pm-lang reuses cel_parser::ParseError directly; no new error type is introduced.
// All parse errors carry a proc_macro2::Span for source-location diagnostics.
pub use ast_parser::PmAstParser;
pub use cel_parser::ParseError;
pub use parser::{ParsedSheet, PmParser};
pub use trivia::attach_trivia;
pub use typecheck::check_sheet;
pub use type_registry::TypeRegistry;
```

- [ ] **Step 6: Run the full existing `pm-lang` test suite to confirm zero regressions**

Run: `cargo test -p pm-lang`
Expected: every pre-existing test passes unchanged, plus this task's 13 new tests.

- [ ] **Step 7: Run the full workspace build and test suite**

Run:
```bash
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
```
Expected: zero compiler warnings; every test across the workspace passes (this plan never touches
`begin`, `cel-runtime`, or `property-model`, so their suites are an unaffected regression check).

- [ ] **Step 8: Format and lint the whole workspace**

Run:
```bash
cargo fmt --all
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: `cargo fmt` makes no further changes beyond what it applies; zero warnings from all three
clippy invocations (the two `begin`-specific ones are unaffected by this plan but are part of this
repo's required pre-PR check suite).

- [ ] **Step 9: Commit**

```bash
git add pm-lang/src/typecheck.rs pm-lang/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(pm-lang): add check_sheet, pm-lang's type checking (v1) entry point

Checks each cell's literal initializer against its : type_name
annotation, and each relationship/conditional method's body against
its declared outputs (arity: single value vs n-tuple; and per-output
type), built on cel_parser::ty::check_expr for the CEL expression
bodies. Completes the design doc's Type checking (v1) section; the
follow-on pm-lsp plan wires this and PmAstParser's existing syntax-error
recovery into textDocument/publishDiagnostics.
EOF
)"
```

---

## What this plan deliberately doesn't do

- Doesn't check `ConditionalBranch.literal` (the `0i32` in `0i32 => { ... }`) against its
  `conditional`'s match cell's declared type. Left for a later pass if wanted — omitted here to
  keep this plan's scope matching the design doc's explicitly listed checks (cell initializers,
  method arity/output types, operator type errors).
- Doesn't check `Expr::If`/`Expr::Tuple`/`Expr::TupleIndex`/`Expr::Apply` nodes themselves (only
  recurses into their children) — see Task 3's rationale.
- Doesn't wire any of this into an LSP or editor — that's the next sub-plan (`pm-lsp` +
  `textDocument/publishDiagnostics`), followed by the VS Code extension sub-plan, both from the
  same Phase 3 design doc section.
