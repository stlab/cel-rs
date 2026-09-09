# Locating adam-rs Errors in adam-lang Source

**Date:** 2026-09-08
**Branch:** worktree-adam-lang/improve-error-reporting
**Status:** Approved (design), not yet implemented

## Summary

`adam-lang` reports every `adam_rs::Error` it encounters while building a `Sheet` at the
sheet's opening span, regardless of which `relationship`, method, or output actually
caused it — e.g. a `relationship { a := b; b := 42; }` block with mismatched method cells
is reported on the sheet's first line instead of at the `relationship` block. The
underlying cause splits into two independent problems with two independent fixes:

- **Build-time errors** (`add_relationship`, `add_conditional`, `add_filter`, `add_out`,
  `add_requirement`) fail synchronously while `adam-lang`'s parser still has the relevant
  span in scope; several call sites throw it away in favor of
  `proc_macro2::Span::call_site()`.
- **Runtime errors** (`Error::MethodFailed`/`Error::TypeMismatch` raised by
  `Sheet::propagate()`'s `execute_plan`) surface long after parsing — typically from a UI
  event handler — when no span is in scope at all. `adam-rs` knows which relationship and
  method raised the error at the point it happens, but currently discards that too.

This design fixes the build-time case directly (thread the already-available span through
instead of `call_site()`) and, for the runtime case, has `adam-rs` tag the error with a new
`ErrorLocation` component id; `adam-lang` retains an auxiliary `(RelationshipId, method
index) -> SourceSpan` table built during parsing to translate that id back to a span when
formatting the error.

`Error::Cycle`/`Conflict`/`FilterCycle` (which implicate a *set* of relationships rather
than one method) are explicitly out of scope — tracked as
[stlab/cel-rs#188](https://github.com/stlab/cel-rs/issues/188).

---

## 1. Two categories of error, two fixes

| Call | When it fails | Span available at the raise site? | Fix |
|---|---|---|---|
| `add_relationship` (structural checks) | Synchronously, during parsing, before a `RelationshipId` exists | No `RelationshipId` yet — only a position in the `Vec<Method>` being validated | New `ErrorLocation::MethodIndex(usize)`, resolved immediately by the caller |
| `add_conditional`, `add_filter`, `add_out`, `add_requirement` | Synchronously, during parsing | Yes — `adam-lang` already holds the right span in `ParseContext` | Use the span already in scope; no `adam-rs` change |
| `execute_plan`'s `MethodFailed`/`TypeMismatch` | During `Sheet::propagate()`, called by application code well after parsing | No — parsing is long over | New `ErrorLocation::Method(RelationshipId, usize)`, resolved later via `adam-lang`'s aux table |

## 2. `adam_rs::ErrorLocation`

A new public, `#[non_exhaustive]` enum in `adam-rs`, carrying only structural ids — no
span or text, keeping `adam-rs` unaware of source locations entirely:

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorLocation {
    /// Index into the `methods` `Vec` passed to `add_relationship`, before any
    /// `RelationshipId` exists for it.
    MethodIndex(usize),
    /// A method within an already-registered relationship.
    Method(RelationshipId, usize),
}
```

`location: Option<ErrorLocation>` is added to the five `Error` variants `add_relationship`
raises (`TypeMismatch`, `InvalidMethod`, `MismatchedMethodCells`, `DuplicateMethodOutputs`,
`InvalidCellKind`) and to `MethodFailed`/`TypeMismatch` where `execute_plan` raises them.
`MethodFailed`'s current constructor changes from a tuple variant to a struct variant
(`MethodFailed { error: anyhow::Error, location: Option<ErrorLocation> }`) to carry the new
field; a `pub fn location(&self) -> Option<ErrorLocation>` accessor is added to `Error` so
callers don't need to match on every variant.

Set to `None` where no method index applies (e.g. `add_relationship(vec![])`'s empty-list
check, or `MethodFailed` raised from requirement/filter evaluation in `propagate()`, which
have no method index at all) — callers fall back to the best whole-construct span they
already have, so this is never worse than today's behavior, only sometimes not improved.

Within `add_relationship`, each structural check sets `MethodIndex` to the method actually
responsible:

- Per-cell checks (`InvalidId`/`TypeMismatch`/`InvalidCellKind` in the input/output loops) —
  the method currently being validated.
- `MismatchedMethodCells` — the first method whose cell set diverges from method 0's.
- `DuplicateMethodOutputs` — the method whose output set is empty-with-duplicates or a
  repeat of an earlier method's.
- `InvalidMethod` (empty `outputs`) — the offending method; `None` for the "methods list
  itself is empty" case.

Within `execute_plan`, `MethodFailed` and `TypeMismatch` are raised inside the `PlanStep::
Method(rel_id, method_idx)` arm, which already binds both values — `Some(ErrorLocation::
Method(rel_id, method_idx))` is passed straight through.

## 3. `adam-lang`: resolving `MethodIndex` immediately (build-time)

`AdamParser::parse_relationship_decl` (`adam-lang/src/parser.rs:789`) already loops over
each binding to build the `methods: Vec<Method>` it passes to `add_relationship`. It
additionally collects each binding's `SourceSpan` (via
`SourceSpan::from_proc_macro2_range`, start of the binding target to end of the `;`) into a
parallel `Vec<SourceSpan>`, plus the whole block's span (`relationship` keyword to closing
`}`).

On `add_relationship`'s `Err`, `location` is resolved against these parallel spans:
`MethodIndex(i)` → `spans[i]`; `None` → the whole block's span. The resulting
`ParseError` carries that span instead of `Span::call_site()`.

`add_conditional`/`add_filter`/`add_out`/`add_requirement`'s `map_err` closures are updated
the same way, but simpler: each already has a specific, correct span sitting in `ctx` at
the call site (e.g. `match_span` for `add_conditional`, `name_span` for `add_out` — matching
the pattern `add_filter`'s error conversion already uses at
[parser.rs:1326](../../../adam-lang/src/parser.rs#L1326)); they just stop discarding it in
favor of `Span::call_site()`.

## 4. `adam-lang`: the auxiliary span table (runtime)

`ParsedSheet` (`adam-lang/src/parser.rs:38`) gains:

```rust
pub method_spans: HashMap<(RelationshipId, usize), cel_parser::SourceSpan>,
```

populated in `parse_relationship_decl` whenever `add_relationship` *succeeds*: for each
`(idx, span)` collected in Section 3, insert `(rel_id, idx) -> span`. `cel_parser::
SourceSpan` (not `proc_macro2::Span`) is used to match the existing convention — it's
`Send + Sync` and is already how `SpanContext` stores locations for CEL-internal runtime
errors, so the table stays usable from application code that outlives the original
`TokenStream`.

## 5. Consuming `ErrorLocation` at runtime: `format_adam_error`

`adam-web-ui/src/labels.rs`'s `format_adam_error` (the only place in the workspace that
renders a post-parse `adam_rs::Error` for a human, used by `begin`) gains a
`method_spans: &HashMap<(RelationshipId, usize), SourceSpan>` parameter:

```rust
pub fn format_adam_error(
    e: &Error,
    method_spans: &HashMap<(RelationshipId, usize), SourceSpan>,
    source: &str,
    file_name: &str,
    renderer: &Renderer,
) -> String
```

For `MethodFailed`, the existing inner-`SpanContext` check (CEL-internal errors like
division-by-zero, attached automatically by `cel-parser`'s `span-diagnostics` feature) runs
first, unchanged — it's strictly finer-grained (points at the exact failing
sub-expression). Only when that's absent does it fall back to `e.location()` →
`method_spans` lookup, rendering the whole failing binding's span. `TypeMismatch` has no
inner CEL error to check, so it goes straight to the `location()` fallback. Every other
variant keeps rendering via `Display`, exactly as today.

`adam-lsp` needs no changes: it never calls `Sheet::propagate()`, so it never observes a
runtime `adam_rs::Error` — only the build-time `ParseError`s Section 3 already fixes.

## 6. Testing

- `adam-rs`: existing `add_relationship_*_returns_error` tests
  (`adam-rs/src/sheet.rs:2183` onward) gain assertions on `.location()` for each variant;
  a new `execute_plan`-driven test asserts `MethodFailed`/`TypeMismatch`'s `location()`
  matches the failing `(RelationshipId, method_idx)`.
- `adam-lang`: a new test parses the exact example from this design's motivation
  (mismatched method cells) and asserts the returned `ParseError`'s span covers the
  `relationship { ... }` block, not line 1; a companion test asserts a *successful* parse
  populates `method_spans` with one entry per binding.
- `adam-web-ui`: `format_adam_error`'s existing tests
  (`adam-web-ui/src/labels.rs:314` onward) gain a case where `MethodFailed` has no
  `SpanContext` but does have an `ErrorLocation`, asserting the rendered diagnostic still
  underlines the right source line via the `method_spans` fallback.

## 7. Out of scope

`Error::Cycle`, `Error::Conflict`, `Error::FilterCycle` — raised by the planner
(`adam-rs/src/planner.rs`, `adam-rs/src/planner/release.rs`) during `propagate()`, these
implicate a *set* of relationships, not one `(RelationshipId, method_idx)`. `ErrorLocation`
as designed here doesn't fit that shape and needs its own design pass. Tracked as
[stlab/cel-rs#188](https://github.com/stlab/cel-rs/issues/188).
