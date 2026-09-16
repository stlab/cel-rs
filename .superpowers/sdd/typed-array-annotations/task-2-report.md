# Task 2 Report: recursive type expressions and generic CEL type resolution

## Scope
Implemented only Task 2 from `docs/superpowers/plans/2026-09-16-typed-array-annotations.md`: recursive CEL `TypeExpr` parsing/representation, a generic CEL-facing leaf type resolver/descriptor boundary, parser entry points for type expressions, and a parser-context annotated-array hook. This deliberately does **not** add array annotation grammar or Adam integration yet.

## Changed files
- `cel-parser/src/type_expr.rs`
  - Added public recursive `TypeExpr`, `ResolvedLeafType`, `ResolvedType`, `ResolvedArrayType`, and `TypeResolver`.
  - Added built-in CEL scalar resolver support and recursive resolution helpers.
  - Added focused tests for scalar, recursive array, tuple syntax reuse, unknown names, and custom registry-style resolution.
- `cel-parser/src/lib.rs`
  - Exported the new type-expression/resolution types.
  - Added parser-held configurable type resolver with built-in defaults.
  - Added `parse_type_expr`, `parse_type_expr_tokens`, `parse_type_expr_str`, and `resolve_type_expr`.
  - Added the recursive internal `parse_type_expression` grammar.
- `cel-parser/src/parser_context.rs`
  - Added `ParserContext::make_annotated_array` with a default unannotated fallback.
  - Implemented the typed-descriptor path for `DynSegmentContext`.
  - Added focused tests covering preserved inference and typed-empty collection via the new context hook.
- `.superpowers/sdd/typed-array-annotations/task-2-report.md`
  - Recorded Task 2 implementation details and verification evidence.

## TDD sequence
1. Added failing tests that referenced the missing Task 2 surface:
   - `TypeExpr` parsing and resolution entry points.
   - Custom resolver configuration.
   - `ParserContext::make_annotated_array`.
2. Ran focused tests to confirm expected compile failures.
3. Implemented the minimal recursive type-expression module, parser entry points, default/custom resolver support, and parser-context annotated-array hook.
4. Re-ran focused tests until green.
5. Ran `cargo fmt --all` and re-ran the focused tests on the formatted code.

## Test commands and results
### RED verification
- `cargo test -p cel-parser type_expr`
  - Failed at compile time as expected with unresolved `type_expr` exports and missing `parse_type_expr_str`, `resolve_type_expr`, and `with_type_resolver` APIs.

### GREEN verification
- `cargo test -p cel-parser type_expr`
  - Passed: 6 tests.
- `cargo test -p cel-parser parser_context`
  - Passed: 16 tests.
- `cargo fmt --all`
  - Succeeded.
- `cargo test -p cel-parser type_expr`
  - Passed again after formatting: 6 tests.
- `cargo test -p cel-parser parser_context`
  - Passed again after formatting: 16 tests.

## Design decisions
- Kept `TypeResolver` leaf-only: it resolves named scalar leaves, while recursive array/tuple composition stays in `TypeExpr::resolve`. This is the smallest boundary both direct CEL execution and future deferred checking can share.
- Preserved `CELParser::new(OpLookup::new())` behavior by defaulting the parser to built-in CEL scalar names and adding `with_type_resolver`/`set_type_resolver` only as opt-in extensions.
- Introduced `ResolvedArrayType` as the parser-context handoff object so future array annotations can validate that the complete annotation is an array before reaching `make_annotated_array`.
- Kept tuple type-expression syntax reusable in `TypeExpr` without changing runtime tuple-array support; `ResolvedArrayType::from_resolved_type` still rejects tuple-valued array elements with the existing issue `#213` diagnostic.
- Left closure parameter type parsing untouched because its current one-tuple behavior intentionally differs from general `type_expr` grammar.

## Concerns
- Built-in leaf resolution currently duplicates the built-in scalar-name match once for cast/closure metadata and once for array element descriptors. A later cleanup may want to consolidate those tables without broadening Task 2 scope.
- Task 2 intentionally stops short of threading unresolved type annotations into `AstContext` or parsing `: type_expr` after arrays; Task 3 must connect the new reusable grammar to array expressions and AST storage.
