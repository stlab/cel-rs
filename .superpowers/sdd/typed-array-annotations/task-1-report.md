# Task 1 Report: typed-empty runtime array construction

## Scope
Implemented only Task 1 from `docs/superpowers/plans/2026-09-16-typed-array-annotations.md`: typed-empty runtime array construction in `cel-runtime`, plus the explicit `DynSegment` typed-array operation needed for annotated empty arrays.

## Changed files
- `cel-runtime/src/dynamic_array.rs`
  - Added `DynamicArray::empty_with_element_type`.
  - Added `ArrayElementType::same_shape_and_layout` for explicit descriptor validation.
  - Added tests for typed empty leaf, nested-array, and zero-sized descriptors.
- `cel-runtime/src/dyn_segment.rs`
  - Added `DynSegment::make_typed_array`.
  - Refactored array construction through a shared internal helper so inferred `make_array` behavior stays unchanged.
  - Added tests for typed empty leaf/nested arrays and explicit descriptor mismatch rejection.
- `.superpowers/sdd/typed-array-annotations/task-1-report.md`
  - Recorded implementation details, test evidence, and concerns.

## TDD sequence
1. Added failing tests that referenced the missing `DynamicArray::empty_with_element_type` and `DynSegment::make_typed_array` APIs.
2. Ran focused tests to confirm the expected compile failures.
3. Implemented the minimal constructor and typed-array collection path.
4. Re-ran focused runtime tests and the existing `make_array`-focused tests.
5. Ran `cargo fmt --all`.

## Test commands and results
### RED verification
- `cargo test -p cel-runtime empty_with_element_type`
  - Failed at compile time with `no associated function or constant named 'empty_with_element_type' found for struct 'dynamic_array::DynamicArray'`.
- `cargo test -p cel-runtime make_typed_array`
  - Failed at compile time with `no method named 'make_typed_array' found for struct 'dyn_segment::DynSegment'`.

### GREEN verification
- `cargo test -p cel-runtime empty_with_element_type`
  - Passed: 3 tests.
- `cargo test -p cel-runtime make_typed_array`
  - Passed: 3 tests.
- `cargo test -p cel-runtime make_array`
  - Passed: 17 tests, confirming the existing inferred array path still works.
- `cargo fmt --all`
  - Succeeded.
- `git --no-pager diff --check`
  - Succeeded with no diff-format errors.

## Design decisions
- Chose a dedicated `DynamicArray::empty_with_element_type` constructor instead of forcing callers through a synthetic `Vec<T>` path, because typed empty arrays have no source vector to infer from.
- Kept typed-empty arrays allocation-free by constructing them from a descriptor plus the existing aligned dangling pointer helper.
- Stored length and capacity as zero for the typed-empty constructor, matching the "no element allocation" requirement while preserving valid `try_into_vec` round-trips in tests.
- Preserved the existing inferred `make_array` contract by routing it through a shared helper but keeping its empty-array rejection and same-shape inference logic unchanged.
- Made explicit descriptor validation stricter than semantic equality by checking recursive shape plus layout (`size` and `align`) through `ArrayElementType::same_shape_and_layout`, so future registry-backed erased descriptors must match the bytes actually on the stack.

## Concerns
- `git diff --check` reported Git's existing line-ending warning for `cel-runtime/src/dyn_segment.rs` (`LF will be replaced by CRLF the next time Git touches it`), but the diff check still exited successfully and no whitespace errors were present.
- Task 1 intentionally does not add parser-facing type resolution yet; later tasks must ensure any registry-provided erased descriptors satisfy the documented layout preconditions before reaching `make_typed_array`.

## Review fix: explicit descriptor drop-hook compatibility
### High issue addressed
`DynSegment::make_typed_array` previously validated an explicit `ArrayElementType` against stack values by recursive type marker plus layout, then stored that explicit descriptor on the resulting `DynamicArray`. A forged descriptor with the right `TypeId`/size/alignment but the wrong drop hook could therefore pass validation and later destroy the collected values with incompatible drop glue.

### Fix
- Tightened explicit descriptor compatibility from “same shape and layout” to “same shape, layout, and drop ownership” by comparing the stored drop hook recursively via `ArrayElementType::same_shape_layout_and_ownership`.
- Updated `DynSegment::make_typed_array` to use that stronger compatibility check before accepting a non-empty annotated array.
- Preserved typed-empty behavior: `make_typed_array(0, ..., element)` still stores the explicit descriptor because no stack-derived values exist yet.
- Left the existing inferred `make_array` path unchanged.

### Regression test
Added `make_typed_array_rejects_a_descriptor_whose_drop_hook_owns_a_different_type`, which forges a descriptor using the correct `TypeId`/layout but the wrong drop hook and verifies `make_typed_array` now rejects it.

### Review-fix verification
#### RED
- `cargo test -p cel-runtime make_typed_array_rejects_a_descriptor_whose_drop_hook_owns_a_different_type`
  - Failed before the fix because `make_typed_array` accepted the forged descriptor (`called Result::unwrap_err() on an Ok value`).

#### GREEN
- `cargo test -p cel-runtime make_typed_array`
  - Passed: 4 tests, including the new drop-hook mismatch regression.
- `cargo test -p cel-runtime`
  - Passed: full `cel-runtime` unit/doctest suite.
- `cargo fmt --all`
  - Succeeded.
- `git --no-pager diff --check`
  - Succeeded; Git again printed only the existing LF→CRLF warning for `cel-runtime/src/dyn_segment.rs`.
