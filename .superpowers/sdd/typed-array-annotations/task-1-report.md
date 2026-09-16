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
