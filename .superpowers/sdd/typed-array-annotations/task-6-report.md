# Task 6 Report: Parser-context clippy blocker

## Scope

Fixed the Task 6 clippy blocker on `ParserContext::make_annotated_array` without changing parser
behavior or the public trait signature.

## Changes

- Added a private `AnnotatedArray` helper in `cel-parser/src/parser_context.rs` to bundle the
  type-ascription inputs used by annotated array construction.
- Updated `DynSegmentContext::make_annotated_array` to resolve the declared element descriptor
  through that helper instead of threading the optional annotation pieces manually.
- Updated `AstContext::make_annotated_array` to reuse the same helper when forwarding the parsed
  annotation into `Expr::Array`.
- Marked the public `ParserContext::make_annotated_array` compatibility shim with a scoped
  `#[expect(clippy::too_many_arguments)]`, since keeping the existing public trait method shape is
  the constraint that prevents reducing the signature itself.

## Focused reproduction

- `cargo clippy -p cel-parser --lib -- -D warnings`
  - FAIL before the change with `clippy::too_many_arguments` at
    `cel-parser/src/parser_context.rs:124`
  - PASS after the change

## Additional verification

- `cargo test -p cel-parser make_annotated_array --lib`
  - PASS
- `cargo test -p cel-parser typed_array --lib`
  - PASS
- `cargo fmt --all`
  - PASS
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
  - PASS
- `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
  - PASS
- `cargo clippy -p begin --all-targets -- -D warnings`
  - PASS

## Concerns

- None. The change is intentionally narrow and preserves the existing public parser-context API.

---

# Task 6 Addendum: final-review findings (2026-09-16)

Addresses all five findings from `.superpowers/sdd/typed-array-annotations/final-review.md`.
Behavioral changes were driven test-first (RED verified before implementation).

## Finding 1 — unscoped cell-initializer mismatch diagnostic (adam-lang/src/typecheck.rs)

**Problem.** The branch widened `check_cell_initializer`'s scalar path from "bare literal only" to
"any expression", adding a brand-new `expression produces ... but ... was expected` diagnostic for
every previously unchecked non-literal scalar initializer.

**Fix (minimal, scope-preserving).** The non-literal scalar path now runs
`check_expr_with_type_resolver` solely to *propagate the body's own* diagnostics (so a typed array
annotation inside the initializer is still reported) and no longer cross-checks the inferred type
against the annotation — the real parser constant-folds the initializer and reports its own
`cell ...: type mismatch`. Updated the function contract and the module doc summary accordingly.

**Tests (RED then GREEN), `adam-lang/src/typecheck.rs`:**
- `cell_initializer_non_literal_expression_is_not_cross_checked`
  - RED: `unexpected diagnostics: ["expression produces \`i32\`, but \`f64\` was expected"]`
- `cell_initializer_array_annotation_diagnostics_still_surface`
  - RED: got 2 diagnostics (array mismatch **plus** the new unscoped one); GREEN: exactly the
    array-annotation diagnostic.

## Finding 2 — Rust type paths in direct runtime array mismatch diagnostics

**Problem.** `ArrayElementType::leaf::<T>()` records `std::any::type_name::<T>()`, so a typed array
annotation resolved from a host registry produced `expected cel_parser::tests::Celsius` (and
`alloc::string::String` for `String`) instead of the source/registered name.

**Fix (minimal API impact).**
- `cel-runtime`: new `ArrayElementType::with_type_name(self, impl Into<Cow<'static, str>>)`
  (consuming builder; identity, layout, and drop hook unchanged, documented with an example).
- `cel-parser`: `ResolvedLeafType::new` now threads its user-facing name into the descriptor it
  stores, so every resolved annotation — built-in or host-registered — carries the source-level
  name into `DynSegment::make_typed_array`'s mismatch text. No signature changed; the new
  postcondition is documented.

**Tests:**
- `cel-parser` `typed_array_annotation_mismatch_names_the_registered_type_not_its_rust_path`
  - RED (verified by temporarily reverting the one-line thread-through):
    `array element 0 has type i32, expected cel_parser::tests::Celsius`
  - GREEN: `expected Celsius`, with no `::` anywhere in the message.
- `cel-runtime` `with_type_name_renames_a_leaf_without_changing_its_identity` and
  `with_type_name_on_a_leaf_is_visible_through_a_nested_descriptor` (RED: method did not exist).

## Finding 3 — Adam output mismatch diagnostics mixed naming schemes

**Problem.** `compile_outputs` rendered the expected side with `TypeRegistry::display_name`
(registered names) and the actual side from `TypeEntry::type_name` / `ValueType::type_name`
(Rust paths), producing `expected \`i32\`, got \`alloc::string::String\``.

**Fix.**
- `TypeRegistry::registered_name(TypeId) -> Option<&str>` (new public accessor for the DSL name;
  `display_name` now reads through it, so there is one lookup path).
- `AdamParser::registered_type_name` and `AdamParser::associated_display_name` render the actual
  side, recursing into nested tuples via the existing `shape_of_associated`, and falling back to
  the value's Rust path only for a genuinely unregistered leaf (no registered name exists).
- Both single-output and destructured-output mismatch messages now use them.

**Tests (RED then GREEN), `adam-lang/src/parser.rs`:**
- `parse_method_output_type_mismatch_names_the_registered_type`
  - RED: `expected \`i32\`, got \`alloc::string::String\``; GREEN: ``got `String` ``.
- `parse_method_destructured_output_type_mismatch_names_the_registered_type`
  - RED: `output 0 \`x\`: ... got \`alloc::string::String\``; GREEN: ``got `String` ``.

## Finding 4 — parser coverage for recursive descriptor mismatch

Added `cel-parser` `typed_array_annotation_rejects_a_recursive_element_type_mismatch`:
`[[0, 1]]: [[f64]]` (nested `i32` values against a nested `f64` annotation) is rejected at parse
time with `array element 0 has type [i32], expected [f64]`. This is coverage of already-correct
behavior, so it passed on first run — no production change was needed for it.

## Finding 5 — drift between the built-in scalar tables

**Problem.** `cel-parser` carried two independent 16-entry built-in tables
(`op_table::builtin_scalar_type` and `type_expr::builtin_array_element_type`) plus a third
hand-written name list in a test, all of which could drift from `Ty`.

**Fix (shared source, then locked down).**
- `op_table` now declares the table once through a `builtin_scalars!` macro that emits both
  `builtin_scalar_type` (with a new `element_type: fn() -> ArrayElementType` member, named with
  the source-level name) and a test-only `BUILTIN_SCALAR_NAMES`.
- `type_expr::builtin_array_element_type` is deleted; `BuiltinTypeResolver` reads the shared table.
- `op_table::tests::builtin_scalar_type_resolves_every_documented_name` iterates
  `BUILTIN_SCALAR_NAMES` instead of its own literal list.

**Tests (RED: both failed to compile against the missing shared const), `cel-parser/src/ty.rs`:**
- `every_builtin_scalar_table_agrees_on_the_same_names_and_types` — for every concrete `Ty`:
  name is in `BUILTIN_SCALAR_NAMES`, `Ty::from_name` round-trips, and `builtin_scalar_type` and
  `BuiltinTypeResolver` agree on `TypeId`, name, descriptor name, size, and align.
- `every_builtin_scalar_name_has_a_concrete_ty` — the reverse direction.

## Verification

- `cargo test -p cel-parser --lib` — 510 passed
- `cargo test -p adam-lang --lib` — 438 passed
- `cargo test -p cel-runtime --lib` — 210 passed
- `cargo test --workspace` — PASS (no failures)
- `cargo test --doc --workspace` — PASS
- `cargo build --workspace` — PASS, no warnings
- `cargo fmt --all` — PASS
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` — PASS
- `cargo clippy -p begin --no-default-features --all-targets -- -D warnings` — PASS
- `cargo clippy -p begin --all-targets -- -D warnings` — PASS
- `RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace` — PASS

## Concerns

- `BUILTIN_SCALAR_NAMES` is `#[cfg(test)]` because only the consistency tests read it; a
  non-test consumer can drop the gate.
- The cross-table consistency tests still enumerate the concrete `Ty` variants by hand
  (`concrete_tys()`); adding a new `Ty` variant forces updates to `Ty::name`/`Ty::type_id` but not
  to that list, so a new variant must still be added there deliberately.
- `associated_display_name` falls back to the runtime Rust type path for an unregistered leaf,
  which is the only name such a value has; the registered-name scheme is used everywhere a
  registered name exists.
- Other adam-lang diagnostics outside the output-mismatch path (for example
  `requirement ...: expected \`bool\`, got ...` and `type ... has no default`) still read
  `TypeEntry::type_name` and therefore still print Rust paths. That is pre-existing behavior and
  outside this review's findings, but it is now a one-call-site change via
  `TypeRegistry::registered_name` if it should be fixed.
