# Task 5 Report: Typed array ascription docs and cross-crate coverage

## Scope

Implemented Task 5 of `docs/superpowers/plans/2026-09-16-typed-array-annotations.md` for issue
#212.

## Changes

- Updated `cel-parser` module/API documentation to describe:
  - colon array type ascriptions such as `[0, 1]: [i32]` and `[]: [i32]`;
  - recursive array type expressions such as `[]: [[i32]]`;
  - custom registry-backed leaf names via `CELParser::with_type_resolver`;
  - typed empty arrays; and
  - the explicit unsupported tuple-valued array diagnostic for both tuple literals and tuple type
    annotations.
- Updated `adam-lang` module documentation to describe embedded CEL array ascriptions, registry
  name resolution through `TypeRegistry`, typed empties, and the continued issue #213 tuple-array
  limitation.
- Added the remaining cross-crate contract tests that were still missing in `adam-lang`:
  - direct `AdamParser` rejection of `[]: [(i32, f64)]`; and
  - deferred `check_sheet` rejection of the same tuple-valued array annotation.
- Added doctest coverage for:
  - direct `cel-parser` custom resolver usage with `[]: [Custom]`; and
  - `adam-lang` custom `TypeRegistry` usage with typed array annotations inside embedded CEL.

## Commands and results

- `cargo test -p adam-lang parse_cell_initializer_rejects_tuple_typed_array_annotations`
  - PASS
- `cargo test -p adam-lang tuple_typed_array_annotations_report_the_existing_issue_213_diagnostic`
  - PASS
- `cargo test --doc -p cel-parser -p adam-lang -p cel-runtime`
  - PASS
- `cargo fmt --all`
  - PASS
- `git diff --check -- cel-parser/src/lib.rs cel-parser/src/ast.rs cel-parser/src/fmt.rs adam-lang/src/lib.rs adam-lang/src/parser.rs adam-lang/src/typecheck.rs .superpowers/sdd/typed-array-annotations/task-5-report.md`
  - PASS

## Concerns

- `docs/superpowers/*` still contains historical design/plan documents from before issue #212 was
  implemented. I left those unchanged because they record prior accepted scope rather than current
  public module behavior.

## Review fix pass

- Updated `docs/superpowers/specs/2026-09-15-cel-homogeneous-arrays-design.md` to keep its
  original historical framing while explicitly marking #212 as implemented, clarifying that only
  bare `[]` remains unsupported, and keeping the tuple-array limitation tied to #213.
- Expanded public rustdoc coverage in `cel-parser` and `adam-lang` so the published contracts now
  show compile-checked scalar, nested-array, custom-registry, and empty-array annotation examples.
- Added exact rustdoc-backed mismatch and unknown-type examples for both direct CEL parsing and
  adam-lang's deferred `check_sheet` path.

### Additional verification

- `cargo test --doc -p cel-parser -p adam-lang`
  - PASS
