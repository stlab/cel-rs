# Task 3 Report — Typed Array Annotations

## Scope
Implemented Task 3 of `docs/superpowers/plans/2026-09-16-typed-array-annotations.md` for issue #212 in `cel-parser`.

## Changes
- Extended `array_expression` parsing from `[...]` to `[...] [: type_expr]`.
- Allowed typed empty arrays such as `[]: [i32]` and nested annotations such as `[]: [[i32]]`.
- Preserved unannotated non-empty array inference (`[0, 1]` still infers `i32`).
- Routed array annotations through `ParserContext::make_annotated_array` using a lazy resolver callback so:
  - `DynSegmentContext` resolves runtime descriptors and constructs typed arrays immediately.
  - `AstContext` preserves unresolved `TypeExpr` syntax and annotation spans for deferred checking/formatting.
- Extended `Expr::Array` with `type_annotation` and `annotation_span`.
- Updated formatter output for annotated arrays to emit `: Type` and support nested/tuple type syntax.
- Added static type-checking support for array annotations, including:
  - complete-array-type validation (`[0]: i32` is rejected),
  - exact element mismatch diagnostics,
  - nested array annotation handling,
  - tuple-type parsing reuse with the existing tuple-array-element unsupported diagnostic.
- Added `ty::check_expr_with_type_resolver` while preserving existing `check_expr` behavior via the built-in resolver.
- Updated parser and API docs for annotated arrays.

## TDD / RED evidence
Added failing tests first in `cel-parser/src/lib.rs`, `cel-parser/src/ast.rs`, `cel-parser/src/fmt.rs`, and `cel-parser/src/ty.rs`.

Initial RED command:
- `cargo test -p cel-parser typed_array_annotation`
  - Result: FAILED (10 failing tests, 1 passing test)
  - Failures showed the expected current gaps: `unexpected token` for postfix annotations and the dedicated unannotated-empty-array diagnostic for `[]`.

## Tests added/updated
- Direct execution parser tests for:
  - `[0, 1]: [i32]`
  - `[]: [i32]`
  - `[]: [[i32]]`
  - exact mismatch rejection
  - unknown type rejection
  - complete-array-type requirement
  - tuple type syntax parsing with tuple array element rejection
  - unsuffixed non-empty inference regression coverage
- AST tests for preserving typed array annotations and typed empty arrays.
- Formatter tests for annotated arrays.
- Type-checking tests for declared array types, nested annotations, exact mismatches, and tuple element rejection.
- Doctest coverage for `check_expr_with_type_resolver`.

## Verification commands and results
- `cargo test -p cel-parser typed_array_annotation`
  - RED verified before production changes.
- `cargo fmt --all`
  - PASS.
- `cargo test -p cel-parser --quiet`
  - PASS (`501` unit/integration tests, `48` doctests).
- `cargo test -p cel-parser typed_array --quiet`
  - PASS (`14` focused typed-array tests).
- `git --no-pager diff --check`
  - PASS (no diff errors; Git reported existing LF/CRLF checkout warnings only).

## Key decisions
- Kept AST parsing unresolved by passing a lazy array-type resolver callback into `ParserContext::make_annotated_array` instead of forcing parse-time resolution for all contexts.
- Returned the declared annotated array type from static checking even when element mismatches are reported, so downstream diagnostics keep the user-declared intent.
- Reused Task 2 `TypeExpr`/resolver APIs and kept tuple type syntax reusable, while preserving the explicit issue #213 diagnostic for tuple-valued array elements.

## Concerns
- No functional concerns for Task 3.
- `git diff --check` reports line-ending warnings from the current checkout (`LF` -> `CRLF` on future Git writes), but there are no whitespace or patch-application problems.
