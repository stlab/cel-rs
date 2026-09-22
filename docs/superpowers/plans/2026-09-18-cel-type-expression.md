# Unified CEL `type_expression` Grammar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace three parallel, inconsistent type-expression implementations
(`cel_parser::type_expr::TypeExpr`, `cel_parser::ast::ClosureParamTypeExpr`,
`adam_lang::ast::TypeExpr`) with one shared `cel_parser::TypeExpr` grammar/AST — extended with
parenthesized generic type arguments (`RangeInclusive(f64)`) — used uniformly for array
annotations, closure parameters, and adam-lang cell/out/source declarations.

**Architecture:** `cel_parser::type_expr::TypeExpr` gains an `args: Vec<TypeExpr>` field and
parses `Name(arg, ...)`. `TypeResolver::resolve_named_type` gains an `args: &[ResolvedType]`
parameter. A new `op_table.rs` table (`builtin_generic_type`/`builtin_generic_type_0`) resolves
`Range`/`RangeInclusive`/`RangeFrom`/`RangeTo`/`RangeToInclusive`/`RangeFull` the same way
`builtin_scalar_type` resolves plain scalars — with full, non-optional `ArrayElementType`
support. Closures stop building their own parallel `ClosureParamTypeExpr` AST and instead parse
the shared `TypeExpr`, converting it to the existing runtime-facing `ClosureParamType` via a new
free function that consults `op_table` directly (closures never see host-registered custom
types). `adam-lang` deletes its own hand-rolled `TypeExpr` and both duplicated `parse_type_expr`
functions, delegating to `cel_parser`'s shared parser via the same take/set-tokens handoff it
already uses for expressions; `TypeRegistry::resolve` gains an explicit, issue-referenced
rejection for array- and generic-typed cell declarations (a larger feature tracked separately),
and `RegistryTypeResolver` falls back to `cel-parser`'s own builtin resolver so embedded CEL
expressions inside adam-lang source (array annotations, casts, closures) see the new generic
types immediately.

**Tech Stack:** Rust 2024, `cel-parser` (recursive-descent parser over `proc_macro2` tokens),
`cel-runtime` (`ArrayElementType`, `DynSegment`), `adam-lang` (its own token-cursor-based
parser(s) delegating to an embedded `cel_parser::Parser`).

**Spec:** `docs/superpowers/specs/2026-09-18-cel-type-expression-design.md`

## Global Constraints

- Parenthesis syntax for generic type arguments: `Name(T, U)`, not `Name<T, U>` or `Name[T]`.
- `TypeResolver::resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType>` —
  every existing bare-name registration keeps working unchanged; `args` is empty for a plain name.
- Built-in generic types (the `Range` family) get a full, non-optional `ArrayElementType` —
  exactly the same capability level as scalars. `ResolvedLeafType` itself is not changed to make
  this field optional.
- `ClosureParamTypeExpr` (cel-parser) and `adam_lang::ast::TypeExpr` are deleted outright — no
  compatibility shims, per repository convention (pre-release, prefer clean redesigns).
- Array-typed closure parameters remain unsupported: reject at parse time citing
  `https://github.com/stlab/cel-rs/issues/228`.
- Array- and generic-typed adam-lang cell/out/source declarations remain unsupported: reject at
  resolve time citing `https://github.com/stlab/cel-rs/issues/227`. CEL *expression* bodies
  embedded in adam-lang source (array annotations, casts, closures) are unaffected by this
  restriction.
- `cargo fmt --all` before every commit. `cargo build --workspace` and
  `cargo test --workspace` must produce zero compiler warnings. All three clippy invocations
  (`--workspace --exclude begin`, `-p begin --no-default-features`, `-p begin`) must pass with
  `-D warnings`. Run `cargo test --doc --workspace` and, if library doc comments changed,
  `RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace`.
- Every function needs a contract-style `///` doc comment (summary, non-obvious
  preconditions/postconditions, `# Errors`, complexity if non-O(1)) per repository conventions.

---

## Task 1: Add `args` to `TypeExpr::Named` and parse `Name(args)`

**Files:**
- Modify: `cel-parser/src/type_expr.rs:18-42` (the `TypeExpr` enum and its doc comment)
- Modify: `cel-parser/src/lib.rs:1877-1975` (`parse_type_expression`)
- Modify: `cel-parser/src/lib.rs:40-56` (the `type_expr`/grammar doc comments referenced by the
  original request at `lib.rs:40-41`)
- Test: `cel-parser/src/type_expr.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `TypeExpr::Named { name: String, args: Vec<TypeExpr>, span: ExprSpan }` (was `{ name,
  span }`, no `args`). `args` is empty for a bare name; non-empty for `Name(a, b)`.
- Consumes: nothing new; `ExprSpan`, `Token`, `Delimiter` already imported in both files.

This task only changes the AST and its parser; resolution (`TypeExpr::resolve`,
`TypeResolver`) is Task 2 — until Task 2 lands, `args` is parsed but not yet passed to a
resolver (existing `resolve` calls still compile against the *old* one-argument
`resolve_named_type`, so this task alone leaves `resolve_named_type` un-fed by `args`; that's
fixed in Task 2). Do not run `cargo build` expecting resolution to use `args` until Task 2 is
done — this task's own tests exercise parsing only.

- [ ] **Step 1: Update the `TypeExpr::Named` variant and its module doc comment**

In `cel-parser/src/type_expr.rs`, change:

```rust
/// `type_expr = identifier | "[" type_expr "]" | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".`
///
/// `()` is the empty tuple type (0 elements); `(T)` is grouping (same as bare `T`); `(T,)` is a
/// 1-element tuple; `(T, U, ...)` is n-element, no trailing comma. `[T]` names a rank-one array
/// whose elements have type `T`; nested brackets compose recursively.
#[derive(Clone, Debug)]
pub enum TypeExpr {
    /// A single type name, resolved later through a [`TypeResolver`].
    Named {
        /// The unresolved type name, exactly as written.
        name: String,
        /// The source span of the full name token.
        span: ExprSpan,
    },
```

to:

```rust
/// `type_expression = identifier [ "(" [ type_expression { "," type_expression } ] ")" ]
///                   | "[" type_expression "]"
///                   | "(" [ type_expression ["," [ type_expression { "," type_expression } ]] ] ")".`
///
/// `()` is the empty tuple type (0 elements); `(T)` is grouping (same as bare `T`); `(T,)` is a
/// 1-element tuple; `(T, U, ...)` is n-element, no trailing comma. `[T]` names a rank-one array
/// whose elements have type `T`; nested brackets compose recursively. `Name(A, B)` names a
/// parameterized (generic) type applying zero or more type arguments (`RangeInclusive(f64)`,
/// `RangeFull()`); an identifier immediately followed by `(` has no other meaning in type
/// position, so this is unambiguous with adjacent tuple-grouping syntax.
#[derive(Clone, Debug)]
pub enum TypeExpr {
    /// A single type name, optionally applying type arguments, resolved later through a
    /// [`TypeResolver`].
    Named {
        /// The unresolved type name, exactly as written.
        name: String,
        /// Type arguments, e.g. `f64` in `RangeInclusive(f64)`. Empty for a plain name.
        args: Vec<TypeExpr>,
        /// The source span of the full name token, including any parenthesized argument list.
        span: ExprSpan,
    },
```

- [ ] **Step 2: Fix every remaining construction/match of `TypeExpr::Named` in this file**

In the same file's `impl TypeExpr` block and `#[cfg(test)] mod tests`, every
`TypeExpr::Named { span, .. }` pattern (used in `span()`'s match arm) already uses `..` so it
needs no change. Every `TypeExpr::Named { name, span }` *construction* or exhaustive
*destructuring* (`TypeExpr::Named { name, span } => ...`) needs `args` added. Run:

```bash
cargo build -p cel-parser --lib 2>&1 | head -80
```

and fix each reported "missing field `args`" or "missing struct field" error by adding
`args: Vec::new()` at each test-only construction site inside `type_expr.rs`'s
`#[cfg(test)] mod tests` (the tests block only ever builds and matches plain names in this
file today, so every fix here is `args: Vec::new()` — there are no generic-typed literals to
preserve).

- [ ] **Step 3: Update `TypeExpr::resolve`'s `Named` arm to compile (still ignoring `args` for now)**

`resolve`'s `Named` arm destructures `TypeExpr::Named { name, span }`; change it to
`TypeExpr::Named { name, span, .. }` for this task only (the `..` intentionally discards `args`
here — Task 2 replaces this whole arm to actually resolve and pass `args` through). This keeps
the crate compiling standalone after this task without prematurely doing Task 2's work.

- [ ] **Step 4: Write a failing test for parsing `Name(arg)`**

Add to `cel-parser/src/type_expr.rs`'s `mod tests`:

```rust
#[test]
fn parse_named_type_expr_with_one_type_argument() {
    let mut parser = CELParser::new(OpLookup::new());
    let expr = parser.parse_type_expr_str("RangeInclusive(f64)").unwrap();

    match expr {
        TypeExpr::Named { name, args, span } => {
            assert_eq!(name, "RangeInclusive");
            assert_eq!(span.start.source_text().as_deref(), Some("RangeInclusive"));
            assert_eq!(span.end.source_text().as_deref(), Some(")"));
            assert_eq!(args.len(), 1);
            match &args[0] {
                TypeExpr::Named { name, args, .. } => {
                    assert_eq!(name, "f64");
                    assert!(args.is_empty());
                }
                other => panic!("expected a named type argument, got {other:?}"),
            }
        }
        other => panic!("expected a named type expression, got {other:?}"),
    }
}

#[test]
fn parse_named_type_expr_with_zero_type_arguments() {
    let mut parser = CELParser::new(OpLookup::new());
    let expr = parser.parse_type_expr_str("RangeFull()").unwrap();

    match expr {
        TypeExpr::Named { name, args, span } => {
            assert_eq!(name, "RangeFull");
            assert_eq!(span.end.source_text().as_deref(), Some(")"));
            assert!(args.is_empty());
        }
        other => panic!("expected a named type expression, got {other:?}"),
    }
}

#[test]
fn parse_named_type_expr_with_multiple_and_nested_type_arguments() {
    let mut parser = CELParser::new(OpLookup::new());
    let expr = parser
        .parse_type_expr_str("Pair(RangeInclusive(f64), i32)")
        .unwrap();

    match expr {
        TypeExpr::Named { name, args, .. } => {
            assert_eq!(name, "Pair");
            assert_eq!(args.len(), 2);
            match &args[0] {
                TypeExpr::Named { name, args, .. } => {
                    assert_eq!(name, "RangeInclusive");
                    assert_eq!(args.len(), 1);
                }
                other => panic!("expected the first argument to be named, got {other:?}"),
            }
            match &args[1] {
                TypeExpr::Named { name, args, .. } => {
                    assert_eq!(name, "i32");
                    assert!(args.is_empty());
                }
                other => panic!("expected the second argument to be named, got {other:?}"),
            }
        }
        other => panic!("expected a named type expression, got {other:?}"),
    }
}

#[test]
fn parse_named_type_expr_with_no_parens_still_parses_a_bare_name() {
    let mut parser = CELParser::new(OpLookup::new());
    let expr = parser.parse_type_expr_str("i32").unwrap();

    match expr {
        TypeExpr::Named { name, args, .. } => {
            assert_eq!(name, "i32");
            assert!(args.is_empty());
        }
        other => panic!("expected a named type expression, got {other:?}"),
    }
}
```

- [ ] **Step 5: Run the new tests and confirm they fail**

```bash
cargo test -p cel-parser --lib type_expr::tests::parse_named_type_expr_with_one_type_argument -- --nocapture
```

Expected: FAIL (the parser doesn't yet consume a `(` after an identifier).

- [ ] **Step 6: Implement `Name(args)` parsing in `parse_type_expression`**

In `cel-parser/src/lib.rs`, replace the identifier branch of `parse_type_expression`:

```rust
if let Some(Token::Identifier(_)) = self.peek_token() {
    let name = self.expect_identifier("expected a type name")?;
    return Ok(TypeExpr::Named { name, span });
}
```

with:

```rust
if let Some(Token::Identifier(_)) = self.peek_token() {
    let name = self.expect_identifier("expected a type name")?;
    let name_span = self.last_span;
    let mut args = Vec::new();
    let mut end_span = name_span;
    if self.is_open_paren() {
        if !self.is_close_paren() {
            args.push(self.parse_type_expression()?);
            while self.is_punctuation(",") {
                args.push(self.parse_type_expression()?);
            }
            if !self.is_close_paren() {
                return Err(self.error_at("expected ',' or closing ')' in type argument list"));
            }
        }
        end_span = self.last_span;
    }
    return Ok(TypeExpr::Named {
        name,
        args,
        span: ExprSpan {
            start: name_span,
            end: end_span,
        },
    });
}
```

(Delete the old two-line body this replaces, including its `let span = ExprSpan { start:
self.last_span, end: self.last_span };` line — the new version computes `span` from
`name_span`/`end_span` instead.)

- [ ] **Step 7: Fix the grouping-collapse arm in the same function to preserve `args`**

Further down in `parse_type_expression`, the `(T)` grouping-collapse arm currently reads:

```rust
return Ok(match first {
    TypeExpr::Named { name, .. } => TypeExpr::Named { name, span },
    TypeExpr::Array { element, .. } => TypeExpr::Array { element, span },
    TypeExpr::Tuple { elements, .. } => TypeExpr::Tuple { elements, span },
});
```

Change the first arm to:

```rust
TypeExpr::Named { name, args, .. } => TypeExpr::Named { name, args, span },
```

- [ ] **Step 8: Run the new tests and confirm they pass**

```bash
cargo test -p cel-parser --lib type_expr::tests:: -- --nocapture
```

Expected: PASS for all `type_expr::tests::*` tests, including the four new ones.

- [ ] **Step 9: Update the grammar doc comments cel-parser/src/lib.rs:40-41 references**

View `cel-parser/src/lib.rs` lines 1-60 and replace whatever placeholder/未定义 `type_expr`
reference exists there (the one flagged by the original request) with:

```text
/// type_expression = identifier [ "(" [ type_expression { "," type_expression } ] ")" ]
///                  | "[" type_expression "]"
///                  | "(" [ type_expression ["," [ type_expression { "," type_expression } ]] ] ")".
```

placed alongside the other top-of-file grammar productions, matching their existing formatting
(this task only fixes the doc comment; do not change grammar semantics here beyond quoting the
production now implemented).

- [ ] **Step 10: Run the full cel-parser test suite and commit**

```bash
cargo fmt --all
cargo test -p cel-parser --lib
cargo test --doc -p cel-parser
```

Expected: PASS, no warnings.

```bash
git add cel-parser/src/type_expr.rs cel-parser/src/lib.rs
git commit -m "feat(cel-parser): parse parenthesized type arguments in type_expression"
```

---

## Task 2: Make `TypeResolver::resolve_named_type` argument-aware

**Files:**
- Modify: `cel-parser/src/type_expr.rs:90-124` (`TypeResolver` trait, its two blanket/array
  impls, `BuiltinTypeResolver`, and `TypeExpr::resolve`'s `Named` arm)
- Modify: `adam-lang/src/type_registry.rs:735-739` (`RegistryTypeResolver`'s impl — signature
  only in this task; the builtin-fallback behavior is Task 8)
- Test: `cel-parser/src/type_expr.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `TypeExpr::Named { name, args, span }` from Task 1.
- Produces: `pub trait TypeResolver { fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType>; }`
  — every later task (3, 8) implements against this exact signature.

- [ ] **Step 1: Update the trait and its two generic impls**

In `cel-parser/src/type_expr.rs`:

```rust
pub trait TypeResolver: Send + Sync {
    /// Returns the registered leaf type named `name` applying `args` (already resolved), or
    /// `None` if `name`/`args` is unrecognized. `args` is empty for a plain (non-generic) name.
    fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType>;
}

impl<F> TypeResolver for F
where
    F: Fn(&str, &[ResolvedType]) -> Option<ResolvedLeafType> + Send + Sync,
{
    fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType> {
        self(name, args)
    }
}

impl<const N: usize> TypeResolver for [(&'static str, ResolvedLeafType); N] {
    fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType> {
        if !args.is_empty() {
            return None;
        }
        self.iter()
            .find(|(registered_name, _)| *registered_name == name)
            .map(|(_, leaf)| leaf.clone())
    }
}
```

(The `[(&str, ResolvedLeafType); N]` impl is a fixed bare-name table with no generic entries of
its own, so any non-empty `args` is simply unrecognized — matching how an unknown name behaves
today.)

- [ ] **Step 2: Update `BuiltinTypeResolver` (signature only — Task 3 adds real generic lookup)**

```rust
impl TypeResolver for BuiltinTypeResolver {
    fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType> {
        if !args.is_empty() {
            return None;
        }
        let builtin = op_table::builtin_scalar_type(name)?;
        Some(ResolvedLeafType::new(
            builtin.type_name,
            (builtin.element_type)(),
        ))
    }
}
```

- [ ] **Step 3: Update `RegistryTypeResolver` in adam-lang for the new signature only**

In `adam-lang/src/type_registry.rs`:

```rust
impl TypeResolver for RegistryTypeResolver {
    fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType> {
        if !args.is_empty() {
            return None;
        }
        self.by_name.get(name).cloned()
    }
}
```

(This is a placeholder-free intermediate state — it compiles and preserves today's behavior
exactly. Task 8 replaces the body to add the builtin-generic fallback; do not add it here, to
keep this task's diff scoped to the signature change alone.)

- [ ] **Step 4: Update `TypeExpr::resolve`'s `Named` arm to resolve and pass `args` through**

Replace the `Named` arm (currently `TypeExpr::Named { name, span, .. } => ...` from Task 1
Step 3) with:

```rust
TypeExpr::Named { name, args, span } => {
    let resolved_args = args
        .iter()
        .map(|arg| arg.resolve(resolver))
        .collect::<Result<Vec<_>>>()?;
    resolver
        .resolve_named_type(name, &resolved_args)
        .map_or_else(
            || {
                Err(ParseError::new_range(
                    format!("unknown type `{name}`"),
                    span.start,
                    span.end,
                ))
            },
            |leaf| Ok(ResolvedType::Scalar(leaf)),
        )
}
```

- [ ] **Step 5: Write a failing test asserting a custom generic-aware resolver receives `args`**

Add to `cel-parser/src/type_expr.rs`'s `mod tests`:

```rust
#[test]
fn resolve_type_expr_passes_resolved_args_to_the_resolver() {
    struct GenericResolver;
    impl TypeResolver for GenericResolver {
        fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType> {
            match (name, args) {
                ("Wrapper", [ResolvedType::Scalar(inner)]) => Some(ResolvedLeafType::new(
                    format!("Wrapper({})", inner.type_name()),
                    ArrayElementType::leaf::<i32>().unwrap(),
                )),
                ("i32", []) => Some(ResolvedLeafType::new(
                    "i32",
                    ArrayElementType::leaf::<i32>().unwrap(),
                )),
                _ => None,
            }
        }
    }

    let mut parser = CELParser::with_type_resolver(OpLookup::new(), GenericResolver);
    let expr = parser.parse_type_expr_str("Wrapper(i32)").unwrap();
    let resolved = parser.resolve_type_expr(&expr).unwrap();

    match resolved {
        ResolvedType::Scalar(leaf) => assert_eq!(leaf.type_name(), "Wrapper(i32)"),
        other => panic!("expected a resolved scalar, got {other:?}"),
    }
}

#[test]
fn resolve_type_expr_reports_unknown_type_when_args_are_unrecognized() {
    let mut parser = CELParser::new(OpLookup::new());
    let expr = parser.parse_type_expr_str("RangeInclusive(f64)").unwrap();
    let err = parser
        .resolve_type_expr(&expr)
        .expect_err("no resolver registers RangeInclusive yet in this test");

    assert!(
        err.message().contains("unknown type `RangeInclusive`"),
        "got: {}",
        err.message()
    );
}
```

- [ ] **Step 6: Run the new tests and confirm they fail, then fix compile errors from the
  signature change**

```bash
cargo build -p cel-parser --lib 2>&1 | head -100
```

Fix any remaining "wrong number of arguments" call sites this reveals (there should be none
beyond what Steps 1-4 already changed, since `resolve_named_type` is only called from
`TypeExpr::resolve`).

```bash
cargo test -p cel-parser --lib type_expr::tests:: -- --nocapture
```

Expected: PASS for all tests, including the two new ones from Step 5.

- [ ] **Step 7: Fix adam-lang's compile break from the trait signature change**

```bash
cargo build -p adam-lang --lib 2>&1 | head -100
```

This should report no errors beyond what Step 3 already fixed (adam-lang's only
`TypeResolver` impl is `RegistryTypeResolver`). If `cargo build -p adam-lang` fails for an
unrelated reason at this point (e.g. it also constructs `TypeExpr::Named` literals elsewhere),
that's Task 7's scope — do not fix `adam-lang::ast::TypeExpr`/parser call sites here; only
confirm the `TypeResolver` trait change itself compiles.

- [ ] **Step 8: Run cel-parser's full test suite and commit**

```bash
cargo fmt --all
cargo test -p cel-parser --lib
cargo test --doc -p cel-parser
```

Expected: PASS, no warnings.

```bash
git add cel-parser/src/type_expr.rs adam-lang/src/type_registry.rs
git commit -m "feat(cel-parser): thread resolved type arguments through TypeResolver"
```

---

## Task 3: Built-in generic types (`Range`/`RangeInclusive`/`RangeFrom`/`RangeTo`/`RangeToInclusive`/`RangeFull`)

**Files:**
- Modify: `cel-parser/src/op_table.rs` (add near the existing `builtin_scalars!` macro and
  `BuiltinScalarType` struct, ~line 1413-1470)
- Modify: `cel-parser/src/type_expr.rs` (`BuiltinTypeResolver::resolve_named_type`)
- Test: `cel-parser/src/op_table.rs` (`#[cfg(test)] mod tests`), `cel-parser/src/type_expr.rs`
  (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `BuiltinScalarType { type_id, type_name, size, align, dropper, push_arg,
  element_type }` (existing struct, unchanged shape) and `TypeResolver`/`ResolvedLeafType`
  (Task 2).
- Produces: `pub(crate) fn builtin_generic_type(generic_name: &str, arg_type_name: &str) ->
  Option<BuiltinScalarType>` and `pub(crate) fn builtin_generic_type_0(name: &str) ->
  Option<BuiltinScalarType>` — Task 6 (closures) calls these directly too.

- [ ] **Step 1: Write failing tests for the new lookup functions**

Add to `cel-parser/src/op_table.rs`'s `mod tests` (near
`builtin_scalar_type_resolves_every_documented_name`):

```rust
#[test]
fn builtin_generic_type_resolves_every_range_family_member_for_every_numeric_type() {
    for generic in ["Range", "RangeInclusive", "RangeFrom", "RangeTo", "RangeToInclusive"] {
        for arg in [
            "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128",
            "isize", "f32", "f64",
        ] {
            let resolved = builtin_generic_type(generic, arg)
                .unwrap_or_else(|| panic!("expected `{generic}({arg})` to resolve"));
            assert_eq!(resolved.type_name, format!("{generic}({arg})"));
        }
    }
}

#[test]
fn builtin_generic_type_range_inclusive_f64_matches_std_any_type_id() {
    let resolved = builtin_generic_type("RangeInclusive", "f64").unwrap();
    assert_eq!(
        resolved.type_id,
        TypeId::of::<std::ops::RangeInclusive<f64>>()
    );
    assert_eq!(
        resolved.size,
        std::mem::size_of::<std::ops::RangeInclusive<f64>>()
    );
}

#[test]
fn builtin_generic_type_is_none_for_unknown_generic_or_argument() {
    assert!(builtin_generic_type("NotAGeneric", "f64").is_none());
    assert!(builtin_generic_type("Range", "not_a_type").is_none());
    assert!(builtin_generic_type("Range", "bool").is_none());
    assert!(builtin_generic_type("Range", "String").is_none());
}

#[test]
fn builtin_generic_type_0_resolves_range_full() {
    let resolved = builtin_generic_type_0("RangeFull").unwrap();
    assert_eq!(resolved.type_name, "RangeFull");
    assert_eq!(resolved.type_id, TypeId::of::<std::ops::RangeFull>());
    assert!(builtin_generic_type_0("NotAGeneric").is_none());
}

#[test]
fn builtin_generic_type_push_arg_declares_a_readable_argument() {
    let resolved = builtin_generic_type("RangeInclusive", "i32").unwrap();
    let mut segment = DynSegment::new::<()>();
    (resolved.push_arg)(&mut segment, 0);
    let value = 1i32..=5i32;
    let result: std::ops::RangeInclusive<i32> = segment.call_dyn(&[&value]).unwrap();
    assert_eq!(result, 1..=5);
}
```

- [ ] **Step 2: Run the tests and confirm they fail to compile (functions don't exist yet)**

```bash
cargo test -p cel-parser --lib op_table::tests::builtin_generic -- --nocapture
```

Expected: FAIL to compile with "cannot find function `builtin_generic_type`" (or similar).

- [ ] **Step 3: Implement the `builtin_generic_types!` macro and table**

Add immediately after the existing `builtin_scalars! { ... }` invocation in
`cel-parser/src/op_table.rs`:

```rust
/// Declares the built-in *generic* (parameterized) type table: one entry per
/// `(generic constructor name, argument type name)` pair, in the same
/// `BuiltinScalarType` shape [`builtin_scalar_type`] returns for plain names. Covers exactly
/// the numeric type set each range operator (`..`, `..=`, etc. — see `RANGE_SIGNATURES` and
/// its siblings above) already supports.
macro_rules! builtin_generic_types {
    ($($generic:literal => { $($arg_name:literal => $ty:ty),* $(,)? }),* $(,)?) => {
        /// Resolves a one-argument built-in generic type (e.g. `Range(u8)`) to its full
        /// descriptor, given the generic constructor's name and its already-resolved argument
        /// type's name (e.g. `"u8"`). Returns `None` if `generic_name`/`arg_type_name` names no
        /// recognized combination.
        ///
        /// - Complexity: O(1).
        pub(crate) fn builtin_generic_type(
            generic_name: &str,
            arg_type_name: &str,
        ) -> Option<BuiltinScalarType> {
            match generic_name {
                $($generic => match arg_type_name {
                    $($arg_name => Some(BuiltinScalarType {
                        type_id: TypeId::of::<$ty>(),
                        type_name: concat!($generic, "(", $arg_name, ")"),
                        size: std::mem::size_of::<$ty>(),
                        align: std::mem::align_of::<$ty>(),
                        dropper: cel_runtime::raw_dropper_for::<$ty>(),
                        push_arg: |seg, idx| seg.push_arg::<$ty>(idx),
                        element_type: || {
                            cel_runtime::ArrayElementType::leaf::<$ty>()
                                .expect("a built-in generic type is never DynamicArray")
                                .with_type_name(concat!($generic, "(", $arg_name, ")"))
                        },
                    },)*
                    _ => None,
                },)*
                _ => None,
            }
        }
    };
}

builtin_generic_types! {
    "Range" => {
        "u8" => std::ops::Range<u8>, "u16" => std::ops::Range<u16>,
        "u32" => std::ops::Range<u32>, "u64" => std::ops::Range<u64>,
        "u128" => std::ops::Range<u128>, "usize" => std::ops::Range<usize>,
        "i8" => std::ops::Range<i8>, "i16" => std::ops::Range<i16>,
        "i32" => std::ops::Range<i32>, "i64" => std::ops::Range<i64>,
        "i128" => std::ops::Range<i128>, "isize" => std::ops::Range<isize>,
        "f32" => std::ops::Range<f32>, "f64" => std::ops::Range<f64>,
    },
    "RangeInclusive" => {
        "u8" => std::ops::RangeInclusive<u8>, "u16" => std::ops::RangeInclusive<u16>,
        "u32" => std::ops::RangeInclusive<u32>, "u64" => std::ops::RangeInclusive<u64>,
        "u128" => std::ops::RangeInclusive<u128>, "usize" => std::ops::RangeInclusive<usize>,
        "i8" => std::ops::RangeInclusive<i8>, "i16" => std::ops::RangeInclusive<i16>,
        "i32" => std::ops::RangeInclusive<i32>, "i64" => std::ops::RangeInclusive<i64>,
        "i128" => std::ops::RangeInclusive<i128>, "isize" => std::ops::RangeInclusive<isize>,
        "f32" => std::ops::RangeInclusive<f32>, "f64" => std::ops::RangeInclusive<f64>,
    },
    "RangeFrom" => {
        "u8" => std::ops::RangeFrom<u8>, "u16" => std::ops::RangeFrom<u16>,
        "u32" => std::ops::RangeFrom<u32>, "u64" => std::ops::RangeFrom<u64>,
        "u128" => std::ops::RangeFrom<u128>, "usize" => std::ops::RangeFrom<usize>,
        "i8" => std::ops::RangeFrom<i8>, "i16" => std::ops::RangeFrom<i16>,
        "i32" => std::ops::RangeFrom<i32>, "i64" => std::ops::RangeFrom<i64>,
        "i128" => std::ops::RangeFrom<i128>, "isize" => std::ops::RangeFrom<isize>,
        "f32" => std::ops::RangeFrom<f32>, "f64" => std::ops::RangeFrom<f64>,
    },
    "RangeTo" => {
        "u8" => std::ops::RangeTo<u8>, "u16" => std::ops::RangeTo<u16>,
        "u32" => std::ops::RangeTo<u32>, "u64" => std::ops::RangeTo<u64>,
        "u128" => std::ops::RangeTo<u128>, "usize" => std::ops::RangeTo<usize>,
        "i8" => std::ops::RangeTo<i8>, "i16" => std::ops::RangeTo<i16>,
        "i32" => std::ops::RangeTo<i32>, "i64" => std::ops::RangeTo<i64>,
        "i128" => std::ops::RangeTo<i128>, "isize" => std::ops::RangeTo<isize>,
        "f32" => std::ops::RangeTo<f32>, "f64" => std::ops::RangeTo<f64>,
    },
    "RangeToInclusive" => {
        "u8" => std::ops::RangeToInclusive<u8>, "u16" => std::ops::RangeToInclusive<u16>,
        "u32" => std::ops::RangeToInclusive<u32>, "u64" => std::ops::RangeToInclusive<u64>,
        "u128" => std::ops::RangeToInclusive<u128>, "usize" => std::ops::RangeToInclusive<usize>,
        "i8" => std::ops::RangeToInclusive<i8>, "i16" => std::ops::RangeToInclusive<i16>,
        "i32" => std::ops::RangeToInclusive<i32>, "i64" => std::ops::RangeToInclusive<i64>,
        "i128" => std::ops::RangeToInclusive<i128>, "isize" => std::ops::RangeToInclusive<isize>,
        "f32" => std::ops::RangeToInclusive<f32>, "f64" => std::ops::RangeToInclusive<f64>,
    },
}

/// Resolves a zero-argument built-in generic type (currently only `RangeFull`) to its full
/// descriptor.
///
/// - Complexity: O(1).
pub(crate) fn builtin_generic_type_0(name: &str) -> Option<BuiltinScalarType> {
    match name {
        "RangeFull" => Some(BuiltinScalarType {
            type_id: TypeId::of::<std::ops::RangeFull>(),
            type_name: "RangeFull",
            size: std::mem::size_of::<std::ops::RangeFull>(),
            align: std::mem::align_of::<std::ops::RangeFull>(),
            dropper: cel_runtime::raw_dropper_for::<std::ops::RangeFull>(),
            push_arg: |seg, idx| seg.push_arg::<std::ops::RangeFull>(idx),
            element_type: || {
                cel_runtime::ArrayElementType::leaf::<std::ops::RangeFull>()
                    .expect("RangeFull is never DynamicArray")
                    .with_type_name("RangeFull")
            },
        }),
        _ => None,
    }
}
```

- [ ] **Step 4: Run the new op_table tests and confirm they pass**

```bash
cargo test -p cel-parser --lib op_table::tests::builtin_generic -- --nocapture
```

Expected: PASS for all 5 new tests.

- [ ] **Step 5: Wire the new lookups into `BuiltinTypeResolver`**

In `cel-parser/src/type_expr.rs`, replace `BuiltinTypeResolver::resolve_named_type` (from
Task 2 Step 2) with:

```rust
impl TypeResolver for BuiltinTypeResolver {
    fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType> {
        let builtin = match args {
            [] => op_table::builtin_scalar_type(name)
                .or_else(|| op_table::builtin_generic_type_0(name)),
            [ResolvedType::Scalar(arg_leaf)] => {
                op_table::builtin_generic_type(name, arg_leaf.type_name())
            }
            _ => None,
        }?;
        Some(ResolvedLeafType::new(
            builtin.type_name,
            (builtin.element_type)(),
        ))
    }
}
```

- [ ] **Step 6: Write a failing end-to-end test resolving a built-in generic type through `TypeExpr`**

Add to `cel-parser/src/type_expr.rs`'s `mod tests`:

```rust
#[test]
fn resolve_builtin_generic_type_expr_end_to_end() {
    let mut parser = CELParser::new(OpLookup::new());
    let expr = parser.parse_type_expr_str("RangeInclusive(f64)").unwrap();
    let resolved = parser.resolve_type_expr(&expr).unwrap();

    match resolved {
        ResolvedType::Scalar(leaf) => {
            assert_eq!(leaf.type_name(), "RangeInclusive(f64)");
            assert_eq!(
                leaf.type_id(),
                TypeId::of::<std::ops::RangeInclusive<f64>>()
            );
        }
        other => panic!("expected a resolved scalar, got {other:?}"),
    }
}

#[test]
fn resolve_builtin_generic_type_as_array_element() {
    let mut parser = CELParser::new(OpLookup::new());
    let expr = parser.parse_type_expr_str("[RangeInclusive(f64)]").unwrap();
    let resolved = parser.resolve_type_expr(&expr).unwrap();
    let array = crate::ResolvedArrayType::from_resolved_type(resolved, expr.span()).unwrap();

    assert_eq!(
        array.element_type().type_id(),
        TypeId::of::<std::ops::RangeInclusive<f64>>()
    );
}

#[test]
fn resolve_range_full_generic_type_expr() {
    let mut parser = CELParser::new(OpLookup::new());
    let expr = parser.parse_type_expr_str("RangeFull()").unwrap();
    let resolved = parser.resolve_type_expr(&expr).unwrap();

    match resolved {
        ResolvedType::Scalar(leaf) => assert_eq!(leaf.type_name(), "RangeFull"),
        other => panic!("expected a resolved scalar, got {other:?}"),
    }
}
```

- [ ] **Step 7: Run the tests and confirm they pass**

```bash
cargo test -p cel-parser --lib type_expr::tests::resolve_builtin_generic -- --nocapture
cargo test -p cel-parser --lib type_expr::tests::resolve_range_full_generic_type_expr -- --nocapture
```

Expected: PASS.

- [ ] **Step 8: Run the full cel-parser suite and commit**

```bash
cargo fmt --all
cargo test -p cel-parser --lib
cargo test --doc -p cel-parser
cargo clippy -p cel-parser --all-targets -- -D warnings
```

Expected: PASS, no warnings.

```bash
git add cel-parser/src/op_table.rs cel-parser/src/type_expr.rs
git commit -m "feat(cel-parser): resolve built-in Range family generic types"
```

---

## Task 4: Expose a public builtin-resolver accessor from cel-parser

**Files:**
- Modify: `cel-parser/src/type_expr.rs:126-128` (`default_type_resolver`)
- Modify: `cel-parser/src/lib.rs:572` (its one call site)
- Modify: `cel-parser/src/lib.rs:224` (the `pub use type_expr::{...}` re-export list)
- Test: `cel-parser/src/type_expr.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `pub fn cel_parser::builtin_type_resolver() -> Arc<dyn TypeResolver>` — Task 8
  (adam-lang's `RegistryTypeResolver`) calls this directly.

This exists so a host (adam-lang) that builds its *own* `TypeResolver` (one that knows its own
custom registered types) can still fall back to `cel-parser`'s built-in scalar/generic types for
any name it doesn't itself recognize — without which built-in generics like
`RangeInclusive(f64)` would be invisible inside CEL expressions embedded in that host's source,
even though they resolve fine in a bare `cel-parser` parser. This closes a gap identified during
planning: the design spec states embedded CEL expressions "get full generic/array-of-generic
support immediately... since those go through `cel-parser`'s own resolution, not
`TypeRegistry::resolve`" — that's only true once this accessor exists and `RegistryTypeResolver`
uses it (Task 8).

- [ ] **Step 1: Rename `default_type_resolver` to a public `builtin_type_resolver`**

In `cel-parser/src/type_expr.rs`, replace:

```rust
pub(crate) fn default_type_resolver() -> Arc<dyn TypeResolver> {
    Arc::new(BuiltinTypeResolver)
}
```

with:

```rust
/// Returns `cel-parser`'s own built-in type resolver — every scalar name
/// [`crate::op_table::builtin_scalar_type`] recognizes, plus every built-in generic type
/// [`crate::op_table::builtin_generic_type`]/[`crate::op_table::builtin_generic_type_0`]
/// recognizes (the `Range` family).
///
/// A host embedding `cel-parser` with its own [`TypeResolver`] (one that also knows
/// host-specific custom types) can delegate any name it doesn't itself recognize to this
/// resolver, so built-in types stay visible inside host-embedded CEL expressions.
#[must_use]
pub fn builtin_type_resolver() -> Arc<dyn TypeResolver> {
    Arc::new(BuiltinTypeResolver)
}
```

- [ ] **Step 2: Fix the one internal call site**

In `cel-parser/src/lib.rs`, change:

```rust
Self::with_shared_type_resolver(op_lookup, type_expr::default_type_resolver())
```

to:

```rust
Self::with_shared_type_resolver(op_lookup, type_expr::builtin_type_resolver())
```

- [ ] **Step 3: Add it to the crate's public re-export list**

In `cel-parser/src/lib.rs`, change:

```rust
pub use type_expr::{ResolvedArrayType, ResolvedLeafType, ResolvedType, TypeExpr, TypeResolver};
```

to:

```rust
pub use type_expr::{
    ResolvedArrayType, ResolvedLeafType, ResolvedType, TypeExpr, TypeResolver,
    builtin_type_resolver,
};
```

- [ ] **Step 4: Write a test confirming the public accessor resolves both scalars and generics**

Add to `cel-parser/src/type_expr.rs`'s `mod tests`:

```rust
#[test]
fn builtin_type_resolver_resolves_scalars_and_generics() {
    let resolver = crate::builtin_type_resolver();

    let scalar = resolver.resolve_named_type("i32", &[]).unwrap();
    assert_eq!(scalar.type_name(), "i32");

    let arg = ResolvedType::Scalar(resolver.resolve_named_type("f64", &[]).unwrap());
    let generic = resolver
        .resolve_named_type("RangeInclusive", std::slice::from_ref(&arg))
        .unwrap();
    assert_eq!(generic.type_name(), "RangeInclusive(f64)");

    assert!(resolver.resolve_named_type("not_a_type", &[]).is_none());
}
```

- [ ] **Step 5: Run the test and confirm it passes**

```bash
cargo test -p cel-parser --lib type_expr::tests::builtin_type_resolver_resolves_scalars_and_generics -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Run the full cel-parser suite and commit**

```bash
cargo fmt --all
cargo build -p cel-parser --lib
cargo test -p cel-parser --lib
cargo test --doc -p cel-parser
```

Expected: PASS, no warnings.

```bash
git add cel-parser/src/type_expr.rs cel-parser/src/lib.rs
git commit -m "feat(cel-parser): expose a public builtin_type_resolver accessor"
```

---

## Task 5: Render `Name(args)` in `fmt.rs`'s `render_type_expr`

**Files:**
- Modify: `cel-parser/src/fmt.rs:358-373` (`render_type_expr`)
- Test: `cel-parser/src/fmt.rs` (`#[cfg(test)] mod tests`, or wherever existing `render_type_expr`
  / array-annotation formatting round-trip tests live — search for `fn format_expr` tests using
  `[i32]`/array annotations to find the right test module)

**Interfaces:**
- Consumes: `TypeExpr::Named { name, args, span }` (Task 1).
- Produces: no new public interface; `render_type_expr`'s output for a non-empty `args` list is
  now `"Name(arg, arg, ...)"` instead of a compile error (Task 1 already made the field
  mandatory, so this task only fixes rendering, which would otherwise ignore `args` via `..`).

- [ ] **Step 1: Write a failing round-trip formatting test**

Find the existing test(s) that assert array-annotation formatting round-trips (search
`cel-parser/src/fmt.rs` for a test asserting something like `format_expr` on an
`Expr::Array { .. }` with a `[i32]` annotation). Add a sibling test in the same module:

```rust
#[test]
fn format_array_annotation_with_a_generic_builtin_element_type() {
    let mut parser = CELParser::new(OpLookup::new());
    let source = "[]: [RangeInclusive(f64)]";
    let expr = parser.parse_expression_str(source).unwrap();
    let formatted = format_expr(&expr, source, 0);
    assert_eq!(formatted, source);
}
```

(If `parse_expression_str`/`format_expr`'s exact call shape differs from other array-annotation
round-trip tests already in this file, copy that shape exactly instead — the point of this test
is exercising a `[RangeInclusive(f64)]` annotation through the real parse → format round trip,
not a specific helper API.)

- [ ] **Step 2: Run it and confirm it fails**

```bash
cargo test -p cel-parser --lib fmt::tests::format_array_annotation_with_a_generic_builtin_element_type -- --nocapture
```

Expected: FAIL — `render_type_expr`'s `Named` arm currently discards `args` (`TypeExpr::Named {
name, .. } => name.clone()`), so the annotation renders as `[RangeInclusive]`, dropping `(f64)`.

- [ ] **Step 3: Implement `Name(args)` rendering**

In `cel-parser/src/fmt.rs`, change:

```rust
fn render_type_expr(type_expr: &crate::TypeExpr) -> String {
    match type_expr {
        crate::TypeExpr::Named { name, .. } => name.clone(),
        crate::TypeExpr::Array { element, .. } => format!("[{}]", render_type_expr(element)),
        crate::TypeExpr::Tuple { elements, .. } => {
```

to:

```rust
fn render_type_expr(type_expr: &crate::TypeExpr) -> String {
    match type_expr {
        crate::TypeExpr::Named { name, args, .. } => {
            if args.is_empty() {
                name.clone()
            } else {
                let inner = args
                    .iter()
                    .map(render_type_expr)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}({inner})")
            }
        }
        crate::TypeExpr::Array { element, .. } => format!("[{}]", render_type_expr(element)),
        crate::TypeExpr::Tuple { elements, .. } => {
```

- [ ] **Step 4: Run the test and confirm it passes**

```bash
cargo test -p cel-parser --lib fmt::tests::format_array_annotation_with_a_generic_builtin_element_type -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Run the full cel-parser suite and commit**

```bash
cargo fmt --all
cargo test -p cel-parser --lib
cargo test --doc -p cel-parser
```

Expected: PASS, no warnings, no regressions in other formatting tests.

```bash
git add cel-parser/src/fmt.rs
git commit -m "feat(cel-parser): render parenthesized type arguments in type annotations"
```

---

## Task 6: Migrate closures onto the shared `TypeExpr`

**Files:**
- Modify: `cel-parser/src/ast.rs:290-315` (`ClosureParam`, delete `ClosureParamTypeExpr`)
- Modify: `cel-parser/src/lib.rs:441-467` (`ClosureParamType` enum, `elements_to_associated`)
- Modify: `cel-parser/src/lib.rs:2008-2165` (`is_closure_expression`,
  `parse_closure_type_expression` — delete the latter, add a new free function)
- Modify: `cel-parser/src/ty.rs:807-815` (`closure_param_ty`)
- Modify: `cel-parser/src/fmt.rs:337-349,380-386,730-739` (`render_closure_param_type`,
  `closure_param_type_end`, and their call site)
- Test: `cel-parser/src/lib.rs`, `cel-parser/src/ty.rs`, `cel-parser/src/fmt.rs` (existing closure
  test modules)

**Interfaces:**
- Consumes: `TypeExpr` (Task 1), `crate::op_table::{builtin_scalar_type, builtin_generic_type,
  builtin_generic_type_0}` (Task 3).
- Produces: `ClosureParam.type_expr: TypeExpr` (was `ClosureParamTypeExpr`);
  `ClosureParamType::Named(BuiltinScalarType)` (renamed from `::Scalar`); a new
  `closure_param_type_from_type_expr(type_expr: &TypeExpr) -> Result<ClosureParamType>` free
  function in `lib.rs` — no other task calls it, but it is the single place a future task would
  extend if closures ever gain array-parameter support (issue #228).

- [ ] **Step 1: Delete `ClosureParamTypeExpr` and change `ClosureParam.type_expr`'s type**

In `cel-parser/src/ast.rs`, delete the `ClosureParamTypeExpr` enum and its doc comment
entirely (the block starting `/// \`closure_type_expression = ...\`` through
`Tuple(Vec<ClosureParamTypeExpr>, ExprSpan), }`). Change `ClosureParam`:

```rust
#[derive(Clone, Debug)]
pub struct ClosureParam {
    /// The parameter's declared name.
    pub name: String,
    /// The name token's span.
    pub name_span: ExprSpan,
    /// The parameter's declared, unresolved type.
    pub type_expr: crate::TypeExpr,
}
```

- [ ] **Step 2: Rename `ClosureParamType::Scalar` to `::Named` and update its uses**

In `cel-parser/src/lib.rs`, change the enum:

```rust
enum ClosureParamType {
    /// A single built-in scalar or generic type (e.g. `i32`, `RangeInclusive(f64)`).
    Named(crate::op_table::BuiltinScalarType),
    /// A (possibly nested) tuple of closure parameter types.
    Tuple(Vec<ClosureParamType>),
}
```

and its `type_id` method:

```rust
fn type_id(&self) -> TypeId {
    match self {
        ClosureParamType::Named(s) => s.type_id,
        ClosureParamType::Tuple(_) => TypeId::of::<cel_runtime::DynamicSequence>(),
    }
}
```

and `elements_to_associated`'s match arm:

```rust
value_type: match ty {
    ClosureParamType::Named(s) => cel_runtime::ValueType::leaf_from_parts(
        s.type_id,
        std::borrow::Cow::Borrowed(s.type_name),
        s.size,
        s.align,
        s.dropper,
    ),
    ClosureParamType::Tuple(nested) => {
        cel_runtime::ValueType::tuple(elements_to_associated(nested))
    }
},
```

- [ ] **Step 3: Delete `parse_closure_type_expression`; add `closure_param_type_from_type_expr`**

In `cel-parser/src/lib.rs`, delete the entire `parse_closure_type_expression` method (its
doc comment through its closing `}`). Add this free function near `elements_to_associated`
(outside the `impl<C: ParserContext> Parser<C>` block, since it needs no parser state):

```rust
/// Builds a closure parameter's runtime-dispatch [`ClosureParamType`] from its already-parsed
/// [`TypeExpr`]. Closures only ever support built-in scalar/generic/tuple types — never
/// host-registered custom types — so this resolves directly against `op_table`'s built-in
/// tables rather than through a [`TypeResolver`].
///
/// # Errors
///
/// Returns an error if a named leaf (or its single type argument) names no recognized built-in
/// type, or if `type_expr` contains an [`TypeExpr::Array`] (closure parameters cannot yet be
/// array-typed; see <https://github.com/stlab/cel-rs/issues/228>).
fn closure_param_type_from_type_expr(type_expr: &TypeExpr) -> Result<ClosureParamType> {
    match type_expr {
        TypeExpr::Named { name, args, span } => {
            let scalar = match args.as_slice() {
                [] => crate::op_table::builtin_scalar_type(name)
                    .or_else(|| crate::op_table::builtin_generic_type_0(name)),
                [TypeExpr::Named {
                    name: arg_name,
                    args: arg_args,
                    ..
                }] if arg_args.is_empty() => crate::op_table::builtin_generic_type(name, arg_name),
                _ => None,
            };
            scalar.map(ClosureParamType::Named).ok_or_else(|| {
                ParseError::new_range(format!("unknown type `{name}`"), span.start, span.end)
            })
        }
        TypeExpr::Tuple { elements, .. } => Ok(ClosureParamType::Tuple(
            elements
                .iter()
                .map(closure_param_type_from_type_expr)
                .collect::<Result<Vec<_>>>()?,
        )),
        TypeExpr::Array { span, .. } => Err(ParseError::new_range(
            "array-typed closure parameters are not yet supported; \
             see https://github.com/stlab/cel-rs/issues/228"
                .to_string(),
            span.start,
            span.end,
        )),
    }
}
```

- [ ] **Step 4: Update `is_closure_expression` to parse the shared grammar and convert**

In `cel-parser/src/lib.rs`'s `is_closure_expression`, change:

```rust
let mut params: Vec<(String, Span, ClosureParamType, ClosureParamTypeExpr)> = Vec::new();
if !params_already_closed {
    loop {
        let name = self.expect_identifier("expected closure parameter name")?;
        let name_span = self.last_span;
        if !self.is_punctuation(":") {
            return Err(self.error_at("expected ':' after closure parameter name"));
        }
        let (ty, ty_ast) = self.parse_closure_type_expression()?;
        params.push((name, name_span, ty, ty_ast));
        if self.is_punctuation(",") {
            continue;
        }
        break;
    }
    if !self.is_punctuation("|") {
        return Err(self.error_at("expected ',' or closing '|'"));
    }
}
```

to:

```rust
let mut params: Vec<(String, Span, ClosureParamType, TypeExpr)> = Vec::new();
if !params_already_closed {
    loop {
        let name = self.expect_identifier("expected closure parameter name")?;
        let name_span = self.last_span;
        if !self.is_punctuation(":") {
            return Err(self.error_at("expected ':' after closure parameter name"));
        }
        let ty_ast = self.parse_type_expression()?;
        let ty = closure_param_type_from_type_expr(&ty_ast)?;
        params.push((name, name_span, ty, ty_ast));
        if self.is_punctuation(",") {
            continue;
        }
        break;
    }
    if !self.is_punctuation("|") {
        return Err(self.error_at("expected ',' or closing '|'"));
    }
}
```

Further down in the same method, change:

```rust
match ty {
    ClosureParamType::Scalar(scalar) => (scalar.push_arg)(segment, *idx),
    ClosureParamType::Tuple(elements) => segment
        .push_arg_as_dynamic_sequence_tuple(*idx, elements_to_associated(elements)),
}
```

to:

```rust
match ty {
    ClosureParamType::Named(scalar) => (scalar.push_arg)(segment, *idx),
    ClosureParamType::Tuple(elements) => segment
        .push_arg_as_dynamic_sequence_tuple(*idx, elements_to_associated(elements)),
}
```

- [ ] **Step 5: Update `ty.rs`'s `closure_param_ty`**

In `cel-parser/src/ty.rs`, change:

```rust
fn closure_param_ty(type_expr: &crate::ClosureParamTypeExpr) -> Ty {
    match type_expr {
        crate::ClosureParamTypeExpr::Named(name, _) => Ty::from_name(name).unwrap_or(Ty::Any),
        crate::ClosureParamTypeExpr::Tuple(..) => Ty::Any,
    }
}
```

to:

```rust
/// Maps a closure parameter's declared type to a static [`Ty`] for type checking. A bare
/// built-in scalar name maps to its concrete `Ty`; a generic type (non-empty `args`), an array
/// type, or a tuple maps to [`Ty::Any`] — this pass does not yet model those shapes, consistent
/// with the existing "unresolved falls to `Any`" rule this function already applied to tuples.
fn closure_param_ty(type_expr: &crate::TypeExpr) -> Ty {
    match type_expr {
        crate::TypeExpr::Named { name, args, .. } if args.is_empty() => {
            Ty::from_name(name).unwrap_or(Ty::Any)
        }
        crate::TypeExpr::Named { .. }
        | crate::TypeExpr::Array { .. }
        | crate::TypeExpr::Tuple { .. } => Ty::Any,
    }
}
```

- [ ] **Step 6: Update `fmt.rs`'s closure rendering**

In `cel-parser/src/fmt.rs`, delete `render_closure_param_type` and `closure_param_type_end`
entirely (both small functions shown in the file today). Update their call site:

```rust
let params_s = params
    .iter()
    .map(|p| format!("{}: {}", p.name, render_closure_param_type(&p.type_expr)))
    .collect::<Vec<_>>()
    .join(", ");
```

becomes:

```rust
let params_s = params
    .iter()
    .map(|p| format!("{}: {}", p.name, render_type_expr(&p.type_expr)))
    .collect::<Vec<_>>()
    .join(", ");
```

and:

```rust
let (tail_start, expected): (proc_macro2::Span, &[&'static str]) = match params.last() {
    Some(last) => (closure_param_type_end(&last.type_expr), &["|"]),
    None => (span.start, &[]),
};
```

becomes:

```rust
let (tail_start, expected): (proc_macro2::Span, &[&'static str]) = match params.last() {
    Some(last) => (last.type_expr.span().end, &["|"]),
    None => (span.start, &[]),
};
```

- [ ] **Step 7: Build and fix any remaining compile errors from the rename/type change**

```bash
cargo build -p cel-parser --lib 2>&1 | head -150
```

Fix every reported error (expected: none beyond what Steps 1-6 already cover — this crate has
no other `ClosureParamTypeExpr`/`ClosureParamType::Scalar` references).

- [ ] **Step 8: Write a failing test for a generic-typed closure parameter parsing and running**

Add to `cel-parser/src/lib.rs`'s closure test module (search for existing `is_closure_expression`
/ closure literal tests, e.g. `fn closure_literal_with_scalar_param...`, and add alongside):

```rust
#[test]
fn closure_param_accepts_a_builtin_generic_type() {
    let mut parser = CELParser::new(OpLookup::new());
    let source = "|r: RangeInclusive(f64)| r";
    let segment = parser.parse_expression_str(source).unwrap();
    let range = 1.0f64..=2.0f64;
    let result: std::ops::RangeInclusive<f64> = segment.call_dyn(&[&range]).unwrap();
    assert_eq!(result, 1.0..=2.0);
}

#[test]
fn closure_param_rejects_an_array_type_citing_issue_228() {
    let mut parser = CELParser::new(OpLookup::new());
    let err = parser
        .parse_expression_str("|xs: [i32]| xs")
        .expect_err("array-typed closure parameters are not yet supported");
    assert!(
        err.message().contains("issues/228"),
        "got: {}",
        err.message()
    );
}
```

(If the existing closure tests use a different helper than `parse_expression_str` /
`call_dyn(&[...])` to build and invoke a `DynSegment` closure, copy that helper's exact call
shape instead — match the file's existing convention.)

- [ ] **Step 9: Run the new tests and confirm they pass; run the full closure/ty/fmt test suites**

```bash
cargo test -p cel-parser --lib closure_param_accepts_a_builtin_generic_type -- --nocapture
cargo test -p cel-parser --lib closure_param_rejects_an_array_type_citing_issue_228 -- --nocapture
cargo test -p cel-parser --lib
cargo test --doc -p cel-parser
```

Expected: PASS, no warnings, no regressions in existing closure/`ty.rs`/`fmt.rs` tests.

- [ ] **Step 10: Run clippy and commit**

```bash
cargo fmt --all
cargo clippy -p cel-parser --all-targets -- -D warnings
```

Expected: PASS, no warnings.

```bash
git add cel-parser/src/ast.rs cel-parser/src/lib.rs cel-parser/src/ty.rs cel-parser/src/fmt.rs
git commit -m "feat(cel-parser): migrate closure parameters onto the shared TypeExpr"
```

---

## Task 7: Migrate adam-lang's AST and parsers onto the shared `TypeExpr`

**Files:**
- Modify: `cel-parser/src/lib.rs` (`fn parse_type_expression` → `pub fn parse_type_expression`,
  no body change)
- Modify: `adam-lang/src/ast.rs:177-195` (delete local `TypeExpr` enum + impl; re-export
  `cel_parser::TypeExpr`), `adam-lang/src/ast.rs` test module (~739-777)
- Modify: `adam-lang/src/ast_parser.rs:303-360` (delete hand-rolled `parse_type_expr`; delegate),
  test module (~680, ~1305, ~1394-1545 — struct-variant match syntax)
- Modify: `adam-lang/src/parser.rs:864-920` (delete hand-rolled `parse_type_expr`; delegate)
- Modify: `adam-lang/src/typecheck.rs:221` (signature reference only — no body change needed;
  see rationale below)
- Test: `adam-lang/src/ast.rs`, `adam-lang/src/ast_parser.rs` (existing test modules)

**Note on `adam-lang/src/fmt.rs` and `adam-lang/src/type_registry.rs`:** neither needs changes
in this task. `adam-lang::ast::ExprSpan` is already `pub use cel_parser::ExprSpan;` (confirmed
in the current source), so once `ast::TypeExpr` becomes `pub use cel_parser::TypeExpr;`, every
existing `ast::TypeExpr::span` call site in `fmt.rs` keeps compiling unchanged — no `From`
conversion is required. `type_registry.rs`'s `TypeRegistry::resolve` and `RegistryTypeResolver`
match on `TypeExpr`'s two current *tuple*-variant constructors (`Named(name, span)` /
`Tuple(elements, _)`); once `TypeExpr` is the shared *struct*-variant type, that file fails to
compile until it's updated to the new variant syntax — that rewrite (plus the array/generic
rejection citing issue #227) is Task 8, not this one. Do not touch `type_registry.rs` in this
task; leave it red between Task 7 and Task 8.

**Interfaces:**
- Consumes: `cel_parser::TypeExpr` (Task 1), the now-`pub fn parse_type_expression` parser
  method (this task exposes it).
- Produces: `adam_lang::ast::TypeExpr` becomes an alias for `cel_parser::TypeExpr` — every
  downstream consumer (`fmt.rs`, `type_registry.rs`, `typecheck.rs`, and any future adam-lang
  code) sees exactly one `TypeExpr` type across both crates.

- [ ] **Step 1: Expose `parse_type_expression` publicly in cel-parser**

In `cel-parser/src/lib.rs`, change the method's visibility only (no body change, no signature
change beyond `pub`):

```rust
pub fn parse_type_expression(&mut self) -> Result<TypeExpr> {
```

This method already touches no `self.context`/`C`-specific state (confirmed by reading its
body — it only calls token-stream helpers and constructs `TypeExpr` directly), so making it
`pub` requires no other change, mirroring the existing `pub fn parse_expression_ctx`/
`parse_literal_pattern_ctx` methods this same `impl<C: ParserContext> Parser<C>` block already
exposes for adam-lang's identical token-handoff delegation pattern.

- [ ] **Step 2: Write a failing test that adam-lang's `ast::TypeExpr` is `cel_parser::TypeExpr`**

Add to `adam-lang/src/ast.rs`'s test module (alongside the existing `type_expr_named_span_is_its_own_span`/
`type_expr_tuple_span_is_the_whole_parenthesized_span` tests, which this step's Step 4 will
rewrite in place rather than duplicate):

```rust
#[test]
fn ast_type_expr_is_the_shared_cel_parser_type_expr() {
    fn assert_same_type<T>(_: &T) {}
    let span = cel_parser::ExprSpan {
        start: Span::call_site(),
        end: Span::call_site(),
    };
    let expr: TypeExpr = cel_parser::TypeExpr::Named {
        name: "i32".to_string(),
        args: Vec::new(),
        span,
    };
    assert_same_type::<cel_parser::TypeExpr>(&expr);
}
```

- [ ] **Step 3: Run it and confirm it fails**

```bash
cargo test -p adam-lang --lib ast::tests::ast_type_expr_is_the_shared_cel_parser_type_expr -- --nocapture
```

Expected: FAIL to compile — `adam_lang::ast::TypeExpr` is currently its own local enum with
tuple variants (`Named(String, ExprSpan)`), not `cel_parser::TypeExpr`'s struct variants, so
`TypeExpr::Named { name, args, span }` doesn't type-check against it yet.

- [ ] **Step 4: Replace `ast::TypeExpr` with a re-export**

In `adam-lang/src/ast.rs`, delete the whole `TypeExpr` enum and its `impl TypeExpr` block
(the doc comment starting `` /// `type_expr = identifier | ...` `` through the closing `}` of
`impl TypeExpr { pub fn span(&self) -> ExprSpan { ... } }`). Replace with:

```rust
/// `type_expr = identifier | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".`
///
/// `()` is the empty tuple type (0 elements); `(T)` is grouping (same as bare `T` — types have
/// no precedence to disambiguate, but staying symmetric with `cel_parser`'s expression grammar
/// costs nothing); `(T,)` is a 1-element tuple; `(T, U, ...)` is n-element, no trailing comma.
/// Shared verbatim with `cel_parser` (which additionally supports `[T]` array types and
/// `Name(args)` parameterized/generic types) so both crates resolve, format, and error on
/// exactly one type-expression grammar.
pub use cel_parser::TypeExpr;
```

- [ ] **Step 5: Run the new test and confirm it passes**

```bash
cargo test -p adam-lang --lib ast::tests::ast_type_expr_is_the_shared_cel_parser_type_expr -- --nocapture
```

Expected: still fails to compile at this point — `ast_parser.rs`/`parser.rs` still construct the
old tuple-variant `TypeExpr::Named(name, span)`/`TypeExpr::Tuple(elements, span)` forms. Continue
to Steps 6-8 before re-running.

- [ ] **Step 6: Delegate `ast_parser.rs`'s `parse_type_expr` to the shared grammar**

In `adam-lang/src/ast_parser.rs`, delete the entire hand-rolled `parse_type_expr` method (the
doc comment `` /// `type_expr = identifier | ...` `` through its closing `}`) and replace it
with a token-handoff delegation, matching this file's existing `parse_cel_expression`:

```rust
/// Delegates one `type_expr` to `cel_parser::Parser<AstContext>`, sharing the token stream
/// (the same take/set-tokens handoff `parse_cel_expression` uses).
fn parse_type_expr(&mut self, cursor: &mut TokenCursor) -> Result<ast::TypeExpr> {
    let tokens = cursor.take_tokens().expect("tokens present");
    self.cel.set_lex_tokens(tokens);
    let result = self.cel.parse_type_expression();
    cursor.set_tokens(self.cel.take_lex_tokens().expect("tokens set"));
    cursor.absorb_unbalanced_delimiters(self.cel.unbalanced_delimiter_count());
    result
}
```

- [ ] **Step 7: Delegate `parser.rs`'s `parse_type_expr` to the shared grammar**

In `adam-lang/src/parser.rs`, delete the entire hand-rolled `parse_type_expr` method and
replace it with a token-handoff delegation, matching this file's existing
`parse_cel_expression` (which, unlike `ast_parser.rs`'s version, does not call
`absorb_unbalanced_delimiters` — match this file's own convention, not `ast_parser.rs`'s):

```rust
/// Delegates one `type_expr` to CELParser, sharing the token stream.
fn parse_type_expr(&mut self, ctx: &mut ParseContext) -> Result<crate::ast::TypeExpr> {
    let tokens = ctx.cursor.take_tokens().expect("tokens present");
    self.cel.set_lex_tokens(tokens);
    let result = self.cel.parse_type_expression();
    ctx.cursor
        .set_tokens(self.cel.take_lex_tokens().expect("tokens set"));
    result
}
```

- [ ] **Step 8: Fix remaining construction/match sites across both parser files' test modules**

Every remaining compile error is a mechanical conversion from the old tuple-variant syntax to
the shared struct-variant syntax. Convert construction sites:

```rust
// before
ast::TypeExpr::Named(name, point(span))
ast::TypeExpr::Tuple(Vec::new(), ast::ExprSpan { start: open_span, end: close_span })

// after
ast::TypeExpr::Named { name, args: Vec::new(), span: point(span) }
ast::TypeExpr::Tuple { elements: Vec::new(), span: ast::ExprSpan { start: open_span, end: close_span } }
```

and match sites:

```rust
// before
ast::TypeExpr::Named(n, _) if n == "f64"
ast::TypeExpr::Tuple(elements, _) => elements.len()

// after
ast::TypeExpr::Named { name: n, .. } if n == "f64"
ast::TypeExpr::Tuple { elements, .. } => elements.len()
```

Apply this conversion to every remaining call/test site reported by:

```bash
cargo build -p adam-lang --lib --tests 2>&1 | head -200
```

Fix every reported error using the two conversions above (expected sites: `ast_parser.rs`'s
test module around the existing `ast_type_expr_named_span_is_its_own_span`-style tests and the
`CellDecl`/`OutDecl` fixture-building tests; `ast.rs`'s own test module's
`type_expr_named_span_is_its_own_span`, `type_expr_tuple_span_is_the_whole_parenthesized_span`,
and `cell_decl_type_name_holds_a_nested_tuple_type_expr` tests). Re-run the build until it's
clean, then confirm:

```bash
cargo test -p adam-lang --lib ast::tests::ast_type_expr_is_the_shared_cel_parser_type_expr -- --nocapture
```

Expected: PASS.

- [ ] **Step 9: Run the full adam-lang test suite**

```bash
cargo test -p adam-lang --lib
cargo test --doc -p adam-lang
```

Expected: all pre-existing `ast.rs`/`ast_parser.rs`/`parser.rs` tests still PASS — this task is
a pure representation change (tuple-variant → struct-variant syntax, same grammar, same
`ExprSpan` type), not a behavior change. `type_registry.rs` is expected to still fail to build at
this point (Task 8 fixes it) — restrict these test runs to `--lib` only if `cargo test -p
adam-lang --lib` itself fails to link due to `type_registry.rs`; if so, defer running adam-lang's
test suite to the end of Task 8 and note that here, but still complete Steps 1-8's `ast.rs`/
`ast_parser.rs`/`parser.rs` changes now.

- [ ] **Step 10: Format and commit**

```bash
cargo fmt --all
```

```bash
git add cel-parser/src/lib.rs adam-lang/src/ast.rs adam-lang/src/ast_parser.rs adam-lang/src/parser.rs
git commit -m "feat(adam-lang): migrate the AST parsers onto the shared cel_parser::TypeExpr"
```

---

## Task 8: Rewrite `TypeRegistry::resolve` and wire the builtin-generic fallback

**Files:**
- Modify: `adam-lang/src/type_registry.rs:494-511` (`TypeRegistry::resolve`)
- Modify: `adam-lang/src/type_registry.rs:735-739` (`RegistryTypeResolver::resolve_named_type`
  body — Task 2 left this signature-only)
- Test: `adam-lang/src/type_registry.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `cel_parser::TypeExpr` (Task 7), `cel_parser::builtin_type_resolver()` (Task 4),
  `TypeResolver::resolve_named_type(&self, name, args)` (Task 2).
- Produces: `TypeRegistry::resolve` now rejects array types and any generic (non-empty `args`)
  cell-declared type with a message citing issue #227, per the approved spec's explicit
  boundary; `RegistryTypeResolver` now recognizes every built-in generic/array-element type
  `cel_parser::builtin_type_resolver()` recognizes, in addition to the registry's own
  host-registered names — so a plain CEL expression body embedded in adam-lang source (which
  resolves independently through `cel-parser`'s own resolver, not through this one) and a
  `cell`'s array-typed initializer's element-type annotation both see the same generic types.

- [ ] **Step 1: Write a failing test for `resolve`'s array/generic rejection**

Add to `adam-lang/src/type_registry.rs`'s test module (alongside the existing
`resolve_named_type_expr_returns_the_matching_type_shape` test):

```rust
#[test]
fn resolve_rejects_an_array_cell_type_citing_issue_227() {
    let registry = TypeRegistry::new();
    let span = point(Span::call_site());
    let expr = cel_parser::TypeExpr::Array {
        element: Box::new(cel_parser::TypeExpr::Named {
            name: "i32".to_string(),
            args: Vec::new(),
            span,
        }),
        span,
    };
    let err = registry
        .resolve(&expr)
        .expect_err("array cell types are not yet supported");
    assert!(err.0.contains("issues/227"), "got: {}", err.0);
}

#[test]
fn resolve_rejects_a_generic_cell_type_citing_issue_227() {
    let registry = TypeRegistry::new();
    let span = point(Span::call_site());
    let expr = cel_parser::TypeExpr::Named {
        name: "RangeInclusive".to_string(),
        args: vec![cel_parser::TypeExpr::Named {
            name: "f64".to_string(),
            args: Vec::new(),
            span,
        }],
        span,
    };
    let err = registry
        .resolve(&expr)
        .expect_err("generic cell types are not yet supported");
    assert!(err.0.contains("issues/227"), "got: {}", err.0);
}
```

- [ ] **Step 2: Run them and confirm they fail (to compile)**

```bash
cargo test -p adam-lang --lib type_registry::tests::resolve_rejects_an_array_cell_type_citing_issue_227 -- --nocapture
```

Expected: FAIL to compile — `TypeRegistry::resolve` still matches on `crate::ast::TypeExpr`'s
old tuple-variant constructors (`Named(name, span)`/`Tuple(elements, _)`), which no longer exist
after Task 7, and has no `Array` arm at all.

- [ ] **Step 3: Rewrite `resolve` against the shared struct-variant `TypeExpr`**

In `adam-lang/src/type_registry.rs`, replace:

```rust
pub fn resolve(
    &self,
    expr: &crate::ast::TypeExpr,
) -> std::result::Result<TypeShape, (String, proc_macro2::Span)> {
    match expr {
        crate::ast::TypeExpr::Named(name, span) => {
            let entry = self
                .get(name)
                .ok_or_else(|| (format!("unknown type `{name}`"), span.start))?;
            Ok(TypeShape::Named(entry.type_id))
        }
        crate::ast::TypeExpr::Tuple(elements, _) => {
            let shapes = elements
                .iter()
                .map(|e| self.resolve(e))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(TypeShape::Tuple(shapes))
        }
    }
}
```

with:

```rust
pub fn resolve(
    &self,
    expr: &cel_parser::TypeExpr,
) -> std::result::Result<TypeShape, (String, proc_macro2::Span)> {
    match expr {
        cel_parser::TypeExpr::Named { name, args, span } => {
            if !args.is_empty() {
                return Err((
                    format!(
                        "cell type `{name}` cannot be a parameterized/generic type yet; \
                         see https://github.com/stlab/cel-rs/issues/227"
                    ),
                    span.start,
                ));
            }
            let entry = self
                .get(name)
                .ok_or_else(|| (format!("unknown type `{name}`"), span.start))?;
            Ok(TypeShape::Named(entry.type_id))
        }
        cel_parser::TypeExpr::Tuple { elements, .. } => {
            let shapes = elements
                .iter()
                .map(|e| self.resolve(e))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(TypeShape::Tuple(shapes))
        }
        cel_parser::TypeExpr::Array { span, .. } => Err((
            "cell types cannot be arrays yet; see https://github.com/stlab/cel-rs/issues/227"
                .to_string(),
            span.start,
        )),
    }
}
```

- [ ] **Step 4: Run the new tests and confirm they pass; fix remaining call sites**

```bash
cargo build -p adam-lang --lib --tests 2>&1 | head -200
```

Fix any remaining reported errors using the same tuple-variant → struct-variant conversion from
Task 7 Step 8 (expected: the existing `resolve_named_type_expr_returns_the_matching_type_shape`
test and any other direct `crate::ast::TypeExpr`/`cel_parser::TypeExpr` construction sites in
this file's test module). Then:

```bash
cargo test -p adam-lang --lib type_registry::tests::resolve_rejects_an_array_cell_type_citing_issue_227 -- --nocapture
cargo test -p adam-lang --lib type_registry::tests::resolve_rejects_a_generic_cell_type_citing_issue_227 -- --nocapture
cargo test -p adam-lang --lib type_registry::tests::resolve_named_type_expr_returns_the_matching_type_shape -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Write a failing test for the builtin-generic fallback**

```rust
#[test]
fn cel_type_resolver_falls_back_to_builtin_generics() {
    let registry = TypeRegistry::new();
    let resolver = registry.cel_type_resolver();
    let f64_leaf = ResolvedLeafType::new("f64".to_string(), (f64::default_element_type)());
    let resolved = resolver
        .resolve_named_type(
            "RangeInclusive",
            &[cel_parser::ResolvedType::Scalar(f64_leaf)],
        )
        .expect("RangeInclusive(f64) should resolve via the builtin fallback");
    assert_eq!(resolved.type_name(), "RangeInclusive<f64>");
}
```

(If `ResolvedLeafType::new`/`f64::default_element_type`/`ResolvedLeafType::type_name` differ
from this sketch, match whatever exact constructor/accessor Task 3's own
`builtin_scalar_type_resolves_every_documented_name`-style tests already use in
`cel-parser/src/op_table.rs` — copy that file's real helper calls verbatim instead of this
sketch.)

- [ ] **Step 6: Run it and confirm it fails**

```bash
cargo test -p adam-lang --lib type_registry::tests::cel_type_resolver_falls_back_to_builtin_generics -- --nocapture
```

Expected: FAIL — `RegistryTypeResolver::resolve_named_type` currently returns `None` for any
non-empty `args` (Task 2's signature-only placeholder body).

- [ ] **Step 7: Wire the builtin-generic fallback**

In `adam-lang/src/type_registry.rs`, replace:

```rust
impl TypeResolver for RegistryTypeResolver {
    fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType> {
        if !args.is_empty() {
            return None;
        }
        self.by_name.get(name).cloned()
    }
}
```

with:

```rust
impl TypeResolver for RegistryTypeResolver {
    fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType> {
        if args.is_empty() {
            if let Some(leaf) = self.by_name.get(name).cloned() {
                return Some(leaf);
            }
        }
        cel_parser::builtin_type_resolver().resolve_named_type(name, args)
    }
}
```

(Host-registered names are checked first — and only for a plain, non-generic name, since a
host-registered leaf can never itself be generic — falling back to `cel-parser`'s own builtin
resolver for both plain built-in scalars, e.g. `f64` if somehow not host-registered, and every
generic type, e.g. `RangeInclusive(f64)`. `cel_parser::builtin_type_resolver()` is a plain
function returning a fresh `Arc<dyn TypeResolver>` each call — no allocation concern beyond one
small `Arc` per lookup, consistent with how the rest of this method already allocates.)

- [ ] **Step 8: Run the new test and confirm it passes; run the full adam-lang and cel-parser suites**

```bash
cargo test -p adam-lang --lib type_registry::tests::cel_type_resolver_falls_back_to_builtin_generics -- --nocapture
cargo test -p adam-lang --lib
cargo test --doc -p adam-lang
cargo test -p cel-parser --lib
cargo test --doc -p cel-parser
```

Expected: PASS, no regressions — this restores the full adam-lang suite (deferred from Task 7
Step 9 if `type_registry.rs` had blocked it).

- [ ] **Step 9: Run clippy and commit**

```bash
cargo fmt --all
cargo clippy -p adam-lang --all-targets -- -D warnings
cargo clippy -p cel-parser --all-targets -- -D warnings
```

Expected: PASS, no warnings.

```bash
git add adam-lang/src/type_registry.rs
git commit -m "feat(adam-lang): reject array/generic cell types (#227) and fall back to builtin generics"
```

---

## Task 9: Update grammar doc comments to define the shared `type_expression` production

**Files:**
- Modify: `cel-parser/src/lib.rs:1-60` (crate-level grammar doc comment block)
- Modify: `cel-parser/src/lib.rs` (doc comment on `parse_type_expression`, already updated by
  Task 1 — verify wording stays consistent with this task's crate-level grammar)
- Modify: `adam-lang/src/ast.rs` (module-level doc comment referencing `type_expr`, if any —
  check during this task)

**Interfaces:**
- Consumes: nothing new — this task is documentation-only, closing out the original gap this
  whole plan addresses (`cel-parser/src/lib.rs:40-41` referenced an undefined `type_expr`
  production).
- Produces: no code interface; the crate-level grammar comment now defines `type_expression`
  as a first-class production and removes the obsolete `closure_type_expression` production.

- [ ] **Step 1: Rewrite the crate-level grammar block**

In `cel-parser/src/lib.rs`, replace:

```text
//! array_expression = "[" [ expression { "," expression } ] "]" [ ":" type_expr ].
//! if_expression = "if" expression "{" expression "}" [ "else" ( "{" expression "}" | if_expression ) ].
//! closure_expression = ("||" | "|" [ closure_param { "," closure_param } ] "|") expression.
//! closure_param = identifier ":" closure_type_expression.
//! closure_type_expression = identifier | "(" [ closure_type_expression { "," closure_type_expression } ] ")".
//! parameter_list = expression { "," expression }.
//!
//! literal_pattern = ["-"] literal.
//! ```
//!
//! `literal_pattern` is a separate entry point, not reachable from `expression` — it exists for
//! grammars that embed CEL literals in pattern position (e.g. adam-lang's `conditional_branch`)
//! and need Rust's own `LiteralPattern` rule: a bare literal, or one directly negated by a
//! leading `-` (no `!`, no chained `--`, no arbitrary unary/postfix operand) — see
//! <https://doc.rust-lang.org/reference/patterns.html#literal-patterns>.
//! `type_expr` is the reusable recursive type grammar used by array type ascriptions:
//! bare names resolve through the configured [`TypeResolver`], bracketed forms compose nested
//! array types (`[i32]`, `[[i32]]`), and tuple syntax is preserved for future typed CEL surfaces
//! even though tuple-valued array elements remain explicitly unsupported today.
```

with:

```text
//! array_expression = "[" [ expression { "," expression } ] "]" [ ":" type_expression ].
//! if_expression = "if" expression "{" expression "}" [ "else" ( "{" expression "}" | if_expression ) ].
//! closure_expression = ("||" | "|" [ closure_param { "," closure_param } ] "|") expression.
//! closure_param = identifier ":" type_expression.
//! parameter_list = expression { "," expression }.
//!
//! type_expression = identifier [ "(" [ type_expression { "," type_expression } ] ")" ]
//!                  | "[" type_expression "]"
//!                  | "(" [ type_expression ["," [ type_expression { "," type_expression } ]] ] ")".
//!
//! literal_pattern = ["-"] literal.
//! ```
//!
//! `literal_pattern` is a separate entry point, not reachable from `expression` — it exists for
//! grammars that embed CEL literals in pattern position (e.g. adam-lang's `conditional_branch`)
//! and need Rust's own `LiteralPattern` rule: a bare literal, or one directly negated by a
//! leading `-` (no `!`, no chained `--`, no arbitrary unary/postfix operand) — see
//! <https://doc.rust-lang.org/reference/patterns.html#literal-patterns>.
//!
//! `type_expression` is the single, reusable recursive type grammar shared by array type
//! ascriptions (`[i32]: [...]`), closure parameters (`|x: RangeInclusive(f64)| ...`), and
//! adam-lang cell/source/out type annotations (`cell x: (i32, f64);`). A bare identifier
//! resolves through the configured [`TypeResolver`] (array/closure contexts) or a host
//! `TypeRegistry` (adam-lang cell contexts); `Name(args)` names a parameterized/generic type
//! (e.g. `RangeInclusive(f64)`, the type produced by CEL's own `..=` range operator) resolved
//! against the same [`TypeResolver`]; `[T]` composes nested array types (`[i32]`, `[[i32]]`);
//! and parenthesized tuple syntax names a (possibly nested) tuple type. Every context that
//! accepts `type_expression` documents its own narrower support boundary where one exists (for
//! example, closure parameters do not yet support `[T]` — see
//! <https://github.com/stlab/cel-rs/issues/228> — and adam-lang cell types do not yet support
//! `[T]` or `Name(args)` — see <https://github.com/stlab/cel-rs/issues/227>).
```

- [ ] **Step 2: Check adam-lang's `ast.rs` module doc comment for a stale `type_expr` reference**

```bash
grep -n "type_expr" adam-lang/src/ast.rs
```

If the module-level (`//!`) doc comment at the top of `adam-lang/src/ast.rs` references the old
per-crate grammar independently (rather than deferring to `cel_parser`'s shared production),
update it to point at `cel_parser::TypeExpr`'s grammar instead of restating a duplicate
production — e.g. change any `` /// `type_expr = ...` `` line still present after Task 7's
edits to a short cross-reference: `` /// Uses [`cel_parser::TypeExpr`]'s shared `type_expression` grammar. ``
(Task 7 Step 4 already replaces the enum's own doc comment with the shared-grammar wording
above; this step only catches any *other*, module-level reference Task 7 didn't already touch.)

- [ ] **Step 3: Build docs and confirm no broken intra-doc links**

```bash
RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps -p cel-parser -p adam-lang
```

Expected: PASS, no warnings (broken links, bare URLs, etc. would fail this).

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add cel-parser/src/lib.rs adam-lang/src/ast.rs
git commit -m "docs(cel-parser): define the shared type_expression grammar production"
```

---

## Task 10: Full-workspace verification

**Files:** none (verification only).

- [ ] **Step 1: Format**

```bash
cargo fmt --all
git diff --exit-code
```

Expected: no diff (everything was already formatted incrementally per task; this is a final
safety net).

- [ ] **Step 2: Build the whole workspace with zero warnings**

```bash
cargo build --workspace 2>&1 | tee /tmp/build.log
grep -i warning /tmp/build.log
```

Expected: build succeeds; the `grep` finds no matches (per repo convention, a plain build must
emit zero compiler warnings, which `-D warnings` clippy runs do not fully cover — e.g. an
unused `mut`).

- [ ] **Step 3: Run the whole test suite, including doc tests**

```bash
cargo test --workspace 2>&1 | tee /tmp/test.log
grep -i warning /tmp/test.log
cargo test --doc --workspace
```

Expected: all tests PASS; no warnings in the test build output.

- [ ] **Step 4: Run all three required clippy invocations**

```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

Expected: PASS, no warnings, for all three (this plan touches no `begin` code directly, but
`begin` depends on `adam-lang`/`adam-rs`, so a signature change here can still surface a
downstream warning/error in `begin` — e.g. if `begin` pattern-matches on
`adam_lang::ast::TypeExpr`'s old tuple-variant shape anywhere).

- [ ] **Step 5: Check for any remaining `begin`/`adam-rs`/`adam-lsp` call sites this plan missed**

```bash
grep -rn "ast::TypeExpr::\(Named\|Tuple\)(" --include=*.rs -- . || true
grep -rn "ClosureParamTypeExpr\|ClosureParamType::Scalar\|default_type_resolver" --include=*.rs -- . || true
```

Expected: no matches — confirms no leftover reference to the deleted
tuple-variant`TypeExpr` construction/deconstruction syntax, the deleted `ClosureParamTypeExpr`
type, the renamed `ClosureParamType::Scalar` variant, or the renamed
`default_type_resolver` function anywhere in the workspace (including `adam-rs`, `adam-lsp`,
`begin`, `editors/vscode-adam-lang`'s Rust code if any, and `xtask`). If any match is found,
fix it using the same conversions from Task 7 Step 8 before proceeding.

- [ ] **Step 6: Rebuild docs with warnings promoted to errors**

```bash
RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace
```

Expected: PASS.

- [ ] **Step 7: Final commit (if Steps 1-6 produced any fixups)**

If any of the above steps required a fix, commit it:

```bash
git add -A
git commit -m "fix: address workspace-wide verification findings from the type_expression migration"
```

If no fixes were needed, skip this step — the plan's implementation is complete.
