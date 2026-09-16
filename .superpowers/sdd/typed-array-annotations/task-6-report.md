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
