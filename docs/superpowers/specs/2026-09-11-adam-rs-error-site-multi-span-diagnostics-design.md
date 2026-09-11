# Attaching Component Sets to adam-rs Errors and Rendering Multi-Span Diagnostics

**Date:** 2026-09-11
**Branch:** worktree-adam-lang/issue-188
**Status:** Approved (design), not yet implemented
**Issue:** [stlab/cel-rs#188](https://github.com/stlab/cel-rs/issues/188)

## Summary

The prior error-location pass (`2026-09-08-adam-lang-error-location-design.md`) gave
relationship- and method-level `adam_rs::Error`s a single `ErrorLocation`, which
`adam-lang` resolves back to one source span. That shape fits an error implicating one
method. It does not fit the planner errors `Error::Cycle`, `Error::Conflict`, and
`Error::FilterCycle`, each of which implicates a *set* of relationships (a cycle's
members, or an overconstrained group), nor the structural errors that name two methods at
once (`MismatchedMethodCells`, `DuplicateMethodOutputs`).

This design replaces the two-variant `Copy` `ErrorLocation` with an ordered list of
`ErrorSite`s. Each `Error` variant that can point somewhere carries `sites: Vec<ErrorSite>`,
where `sites[0]` is the primary culprit and the remaining entries are ordered related
context — for a cycle, the full loop in traversal order. `adam-rs` reconstructs the cycle
order and the minimal overconstrained set on the error path only. `adam-lang` gains
relationship- and cell-declaration span tables so it can resolve any `ErrorSite` to a
source span, and `cel-parser` gains a renderer that underlines several spans at once, so a
cycle surfaces as a multi-caret backtrace rather than a single line.

`adam-rs` stays unaware of source locations: `ErrorSite` carries only structural ids
(`RelationshipId`, `CellId`, method index). All naming and span resolution live in
`adam-lang`, all rendering in `cel-parser`.

---

## 1. `adam_rs::ErrorSite`

`ErrorLocation` (with its `MethodIndex`/`Method` variants) is removed. In its place:

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorSite {
    /// A method by position in the `Vec` passed to `add_relationship`, before any
    /// `RelationshipId` exists for it.
    MethodIndex(usize),
    /// A method within a registered relationship.
    Method(RelationshipId, usize),
    /// A whole relationship.
    Relationship(RelationshipId),
    /// A single cell.
    Cell(CellId),
}
```

`ErrorSite` stays `Copy`. The multi-object shape comes from holding several of them in a
`Vec`, not from any one variant carrying a collection.

Each `Error` variant that can name a source component carries `sites: Vec<ErrorSite>`,
replacing the previous `location: Option<ErrorLocation>` field. A variant that never names
one (see the audit in §3) carries no such field. A single accessor replaces `location()`:

```rust
/// The source components this error implicates, primary first.
///
/// `sites[0]` is the primary culprit; any further entries are ordered related context
/// (for a cycle, the remaining members in loop-traversal order). Empty for variants that
/// never track a site, or when a site legitimately does not apply to this occurrence
/// (e.g. `add_relationship(vec![])`'s empty-list `InvalidMethod`).
pub fn sites(&self) -> &[ErrorSite]
```

### Why `sites[0]`-primary convention rather than a per-site role

`adam-rs` does not tag each site with a `Primary`/`Related` role. The ordering convention
carries that distinction (element 0 is primary), and the human-facing wording that a role
would drive — "the baseline method", "cell missing from method 2" — depends on the cell
and relationship *names*, which live only in `adam-lang`. `adam-lang` therefore
synthesizes each site's label by matching on the `Error` variant and walking `sites` in
that variant's documented order (§3), and `adam-web-ui` renders `sites[0]` with a primary
caret and the rest as secondary annotations. Keeping the role out of `adam-rs` avoids a
role enum that would duplicate what the `Error` variant already states.

### `Vec` on the error path

`sites` is a heap `Vec`, against the crate's general "avoid heap allocations" guidance.
This is deliberate and confined to the cold error path: an `Error` is constructed only when
an operation fails, never in steady-state propagation, so the allocation is not on any hot
path. A single-method error's `sites` is a one-element `Vec`.

## 2. Reconstructing the sets in the planner

All three planner errors are raised from `planner::plan` (`adam-rs/src/planner.rs`) or its
`release` submodule. The reconstruction below runs only when a plan has already failed, so
its cost never affects a successful `propagate`.

A new submodule `adam-rs/src/planner/trace.rs` holds two helpers:

- `fn recover_cycle(adj: &HashMap<Node, Vec<Node>>, component: &[Node]) -> Vec<Node>` —
  given a strongly connected component (or a digraph known to contain a cycle), returns one
  simple cycle as an ordered node list (a depth-first search that stops when it revisits a
  node on the current path, then slices out the loop). The result alternates
  `Node::Relationship` and `Node::Cell`.
- `fn minimal_infeasible_set(relationships, active, cells) -> HashSet<RelationshipId>` —
  deletion filtering: starting from `active`, remove each relationship in turn and keep the
  removal whenever the remaining set is still infeasible under `Assignment::solve` (with no
  cells forbidden). The result is a subset-minimal group that cannot be simultaneously
  satisfied. O(R) solves in the worst case.

### `Error::Cycle` (`ReleaseFailure::NoAcyclicAssignment`)

`release::resolve` already computes a valid-but-cyclic `Assignment` (via `Assignment::solve`)
to tell `NoAcyclicAssignment` apart from `NoAssignment`. That assignment is carried on the
failure so `plan` can trace it:

```rust
pub(crate) enum ReleaseFailure {
    NoAssignment,
    NoAcyclicAssignment(Assignment),
}
```

On `NoAcyclicAssignment(assignment)`, `plan` builds the assignment's digraph
(`build_digraph`), finds a non-trivial `tarjan_scc` component, calls `recover_cycle`, and
maps the resulting node order to `sites`: each `Node::Relationship(r)` becomes
`ErrorSite::Relationship(r)` and each `Node::Cell(c)` becomes `ErrorSite::Cell(c)`, in loop
order. `sites` therefore reads relationship, cell, relationship, cell, … around the loop.

### `Error::FilterCycle`

`plan` already detects this case: after `add_filter_edges`, a `tarjan_scc` component with
`len() != 1` is the filter-induced cycle. That component (a mix of `Node::Relationship`,
`Node::Cell`, including the filtered source cell and its filter-argument edge) is passed to
`recover_cycle`, and the ordered node list is mapped to `sites` the same way as `Cycle`.

### `Error::Conflict` (`ReleaseFailure::NoAssignment`, and the post-plan method-count check)

`NoAssignment` means no method assignment exists at all — an overconstrained group, with no
cycle to trace. `plan` calls `minimal_infeasible_set` and emits one
`ErrorSite::Relationship` per member. Order among them is not significant, so no particular
ordering is promised.

`plan` also returns `Error::Conflict` from its post-plan `method_count != active.len()`
check. That path likewise runs `minimal_infeasible_set` over `active` to populate `sites`.

## 3. The `Error` enum audit

Every variant was reviewed for whether it implicates more than one source component. The
site list each site-bearing variant carries, in order:

| Variant | `sites` (ordered, primary first) |
|---|---|
| `Cycle` | the cycle in loop order: `Relationship`, `Cell`, `Relationship`, `Cell`, … |
| `FilterCycle` | the filter-induced cycle in loop order, including the filtered `Cell` and its filter-argument edge |
| `Conflict` | the minimal infeasible `Relationship` set (§2) |
| `MismatchedMethodCells` | `Method(diverging)`, `Method(baseline = index 0)`, then one `Cell` per cell that is in exactly one of the two cell sets |
| `DuplicateMethodOutputs` | self-duplicate case: `Method`, `Cell(the repeated output)`. Cross-method case: `Method(later)`, `Method(earlier)`, then one `Cell` per shared output |
| `InvalidCellKind` | `Method`, `Cell(the wrong-kind output)` |
| `TypeMismatch` | `Method` (or `MethodIndex` at build time) |
| `InvalidMethod` | `Method`/`MethodIndex`, or empty when the methods list itself is empty |
| `MethodFailed` | `Method`, or empty for a failure raised from requirement/filter evaluation (no method index applies) |
| `InvalidConditional` | the cross-referencing cases: the `Relationship`(s) named in more than one branch, or a branch `Relationship` with an extra method, plus the match `Cell` |

Variants with no `sites` field, and why each was left single-object:

- `InvalidId` — the id was not found, so there is no live component to point at.
- `InvalidOutput`, `InvalidFilter`, `InvalidRequirement` — raised synchronously by
  `add_out`/`add_filter`/`add_requirement` while `adam-lang`'s parser still holds the exact
  span of the single construct being built (the `out`/`filter`/`require` declaration). The
  parser attaches that in-scope span directly, so these need no structural site to resolve
  later. Each implicates exactly one construct; there is no second object to name.

`InvalidConditional` is the one build-time error that does gain sites, because its
cross-referencing failure modes (a relationship listed in two conditional branches, a
branch relationship carrying a disallowed extra method) implicate a component declared
*elsewhere* in the source than the `conditional` block being parsed. Naming it needs a
structural site the parser can resolve back to that other declaration's span, which the
in-scope block span cannot reach.

## 4. `adam-lang`: span tables and site resolution

`ParsedSheet` (`adam-lang/src/parser.rs`) gains two tables alongside `method_spans`:

```rust
pub relationship_spans: HashMap<RelationshipId, SourceSpan>,
pub cell_spans: HashMap<CellId, SourceSpan>,
```

`relationship_spans` maps each successfully-added relationship to its whole-block span
(`relationship` keyword through the closing `}`), which `parse_relationship_decl` already
computes as `block_start`..`close_span`. `cell_spans` maps each declared cell to its
declaration span, available as `name_span` at the point `build_default_cell` /
`build_default_source_cell` returns the `CellId`. Both are populated in `ParseContext`
during parsing and moved into `ParsedSheet`, matching how `method_spans` is already built.

A `CellId → name` reverse map is derived from `cell_names` for labelling. `RelationshipId`
has no source name; a relationship is labelled by its position or its block ("the
relationship at line N"), derived from `relationship_spans`.

`ParsedSheet` gains:

```rust
/// Resolves each of `e`'s `ErrorSite`s to a source span and a human label, primary first.
///
/// Sites whose span is not recorded (e.g. an `out`-writer relationship absent from the
/// span tables) are skipped, so the result may be shorter than `e.sites()`; it is empty
/// when no site resolves, and the caller falls back to `Display`.
///
/// - Complexity: O(s) in the number of sites.
pub fn locate_error(&self, e: &adam_rs::Error) -> Vec<(SourceSpan, String)>
```

`locate_error` walks `e.sites()`, resolving each `ErrorSite` against the three span tables,
and synthesizes a label from the `Error` variant and the site's position (per §3's ordering
convention): for `Cycle`, "this relationship writes cell `x`" / "which feeds …"; for
`MismatchedMethodCells`, "this method's cell set differs from the baseline" and "cell `c`
appears in only one method"; and so on. `MethodIndex` never appears in a post-parse error
(the parser resolves it immediately), so `locate_error` treats it as unresolvable.

Label synthesis is the same phrasing whether the error surfaces at parse time or at
propagation time, so it lives in one free function keyed on the `Error` variant and the
site index:

```rust
/// The human label for site `index` of `e`, given a `CellId → name` lookup.
///
/// - Precondition: `index < e.sites().len()`.
fn site_label(e: &adam_rs::Error, index: usize, cell_name: &impl Fn(CellId) -> Option<String>) -> String
```

`locate_error` calls it for the runtime path; §4.1's build-time resolver calls it for the
parse path. Only span resolution differs between the two, never the wording.

### 4.1 Build-time site resolution

`MismatchedMethodCells` and `DuplicateMethodOutputs` (and `InvalidConditional`) are raised
by `add_relationship`/`add_conditional` *during parsing*, so they never reach `locate_error`
— they surface as `ParseError`s (§5). Their sites still resolve, but against what the parser
holds mid-parse rather than the finished tables: a `MethodIndex(i)` maps to the parallel
per-binding span vector `parse_relationship_decl` already builds (§ the existing
`spans: Vec<(Span, Span)>`), and a `Cell(c)` maps to the `cell_spans` accumulated so far
(every cell a relationship references is declared before the relationship, so its span is
already recorded). `Relationship(r)` maps to `relationship_spans` for the
cross-referencing `InvalidConditional` cases. The parser resolves *every* site this way,
producing a primary span plus secondary labelled spans, and attaches them to the
`ParseError` (§5) so the build-time diagnostic shows the same multi-caret backtrace the
runtime path does.

## 5. `cel-parser`: multi-span rendering

`cel-parser`'s `error.rs` renders single-span diagnostics today
(`CELError::format_rustc_style`, `SpanContext::format_rustc_style`). It gains one public
entry point for the multi-span case:

```rust
/// One labelled span in a multi-span diagnostic.
pub struct SpanLabel {
    pub span: SourceSpan,
    pub label: String,
}

/// Renders `title` with several labelled annotations over `source`, the first as the
/// primary caret and the rest as secondary context, in rustc style.
///
/// - Precondition: `labels` is non-empty.
/// - Complexity: O(n) in the total source length plus the number of labels.
pub fn format_multi_span(
    title: &str,
    labels: &[SpanLabel],
    source: &str,
    filename: &str,
    start_line: u32,
    renderer: &Renderer,
) -> String
```

It builds one `annotate_snippets::Snippet` over `source` and chains one `.annotation(...)`
per `SpanLabel` — `AnnotationKind::Primary` for `labels[0]`, `AnnotationKind::Context` for
the rest — each carrying its label text. `annotate-snippets` lays the carets out in source
order and prints each label, producing the cycle backtrace. The existing single-span
methods are untouched.

`ParseError` (the build-time error type) gains an optional list of secondary labelled
spans, defaulting to empty so every existing construction site is unaffected:

```rust
/// Attaches secondary labelled spans, rendered as extra carets alongside the primary.
pub fn with_secondary(self, secondary: Vec<SpanLabel>) -> Self
```

`ParseError::format_rustc_style` renders through `format_multi_span` when `secondary` is
non-empty (primary span as `labels[0]`, the secondaries after), and through the existing
single-span path otherwise. This is what lets a parse-time `DuplicateMethodOutputs` or
`MismatchedMethodCells` underline both methods and the differing cells. `adam-lsp` ignores
the new field and keeps rendering the primary span only; surfacing secondaries as LSP
related-information is a possible later enhancement, out of scope here.

## 6. `adam-web-ui`: `format_adam_error`

`format_adam_error` (`adam-web-ui/src/labels.rs`) is the workspace's only renderer of a
post-parse `adam_rs::Error`. Its `method_spans` parameter is replaced by the `&ParsedSheet`
(the source of all three span tables and of `locate_error`):

```rust
pub fn format_adam_error(
    e: &Error,
    parsed: &adam_lang::ParsedSheet,
    source: &str,
    file_name: &str,
    renderer: &Renderer,
) -> String
```

The control flow:

1. `Error::MethodFailed` whose inner `anyhow::Error` carries a `SpanContext` renders through
   that context exactly as today (the finest-grained, sub-expression span). This path is
   unchanged.
2. Otherwise, `parsed.locate_error(e)` produces the labelled spans. Zero → fall back to
   `e.to_string()` (never worse than today). One → the existing single-caret
   `SpanContext::format_rustc_style`. Two or more → `cel_parser::format_multi_span`, the
   backtrace.

`begin` today shreds each parse result into separate `Signal<Sheet>`, `Signal<Labels>`,
and `Signal<MethodSpans>` values threaded independently through its components. Since
`format_adam_error` now needs all three span tables together, `begin` is changed to hold
the whole `ParsedSheet` in one place (a `Signal<ParsedSheet>`, which derefs mutably to the
live `Sheet` for writes, carrying the immutable span tables along unchanged), with `Labels`
still derived from it. This keeps the parse's auxiliary information in one location instead
of re-threading a growing list of side tables, and removes the standalone `MethodSpans`
signal. The reparse/hot-reload paths set the new `ParsedSheet` in one assignment.

`adam-lsp` needs no change: it never calls `propagate`, so it never observes these runtime
errors — only the build-time `ParseError`s, which already carry their own spans.

## 7. Testing

- `adam-rs`: for each site-bearing variant, a test asserting `sites()` names the expected
  components in the expected order. A genuine two-relationship algebraic loop asserts
  `Cycle`'s `sites` are the two `Relationship`s and the connecting `Cell`s in loop order; a
  two-relationships-one-output structure asserts `Conflict`'s `sites` are exactly the
  minimal infeasible pair; a filter-argument loop asserts `FilterCycle`'s ordered members.
  `MismatchedMethodCells` and `DuplicateMethodOutputs` assert both methods and the differing
  or shared cells appear.
- `planner/trace.rs`: `recover_cycle` returns a genuine simple cycle for a known cyclic
  digraph; `minimal_infeasible_set` returns a subset-minimal group (removing any member
  makes the remainder feasible).
- `adam-lang`: `locate_error` on a parsed cycle returns several ordered spans covering the
  loop's relationships; on mismatched-method-cells returns the two method spans plus the
  differing cells. A successful parse populates `relationship_spans` and `cell_spans` with
  one entry per relationship and per cell.
- `cel-parser`: `format_multi_span` output contains every label and underlines each span;
  `Renderer::plain` output carries no ANSI escapes. A `ParseError::with_secondary` error's
  `format_rustc_style` output underlines the primary and every secondary span; a plain
  `ParseError` (no secondaries) renders exactly as before.
- `adam-lang`: parsing a relationship with two methods sharing an output set returns a
  `ParseError` whose rendered output underlines both bindings; parsing mismatched method
  cells underlines the diverging binding, the baseline binding, and the differing cell
  declarations.
- `adam-web-ui`: a `Cycle` error renders a multi-line backtrace naming each relationship in
  the loop; an error whose sites do not resolve falls back to `Display`.

## 8. Migration

Removing `ErrorLocation` and the `location` fields touches every construction site and the
former `location()` callers:

- `adam-rs/src/sheet.rs` — `add_relationship`'s structural checks build `sites` vectors
  instead of `Some(ErrorLocation::MethodIndex(i))` / `None`; the conditional checks in
  `add_conditional` gain sites per §3.
- `adam-rs/src/planner.rs` — `execute_plan`'s `MethodFailed`/`TypeMismatch` pass
  `vec![ErrorSite::Method(rel_id, method_idx)]`; the three planner errors are populated per
  §2.
- `adam-lang/src/parser.rs` — `parse_relationship_decl`'s (and `parse_conditional_decl`'s)
  `Err` arm resolves *all* of `e.sites()` (§4.1) into a primary span plus
  `SpanLabel` secondaries and calls `ParseError::with_secondary`, replacing the old
  `e.location()` single-span match; the two new span tables are populated and moved into
  `ParsedSheet`.
- `adam-web-ui/src/labels.rs` — `format_adam_error` and its helper are rewritten per §6; the
  `write_str` closures constructing `Error::MethodFailed { .. }` drop `location` for
  `sites: vec![]`.
- `begin` and any other `format_adam_error` caller pass `&ParsedSheet` instead of the
  `method_spans` map.

`Error` and `ErrorSite` are both `#[non_exhaustive]`, so downstream match arms keep their
wildcard fallbacks.
