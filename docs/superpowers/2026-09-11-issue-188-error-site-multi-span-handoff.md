# Handoff: Error-Site Sets and Multi-Span Diagnostics (issue #188)

**Date:** 2026-09-11
**Branch:** worktree-adam-lang/issue-188
**Spec:** `docs/superpowers/specs/2026-09-11-adam-rs-error-site-multi-span-diagnostics-design.md`
**Plan:** `docs/superpowers/plans/2026-09-11-adam-rs-error-site-multi-span-diagnostics.md`
**Issue:** [stlab/cel-rs#188](https://github.com/stlab/cel-rs/issues/188)

## What shipped

Issue #188 asked adam-rs errors to carry the ids of the components they implicate, and
adam-lang to map those to source spans for useful diagnostics, with cycle reports showing a
backtrace, and a review of other errors that could name more than one object. The branch
delivers all of that:

- **adam-rs — `ErrorSite` (replaces `ErrorLocation`).** Each `Error` variant that can point
  somewhere carries `sites: Vec<ErrorSite>` (ordered, `sites[0]` primary); `Error::sites()`
  returns the slice. `ErrorSite` is a `Copy` enum of `MethodIndex`, `Method(RelationshipId,
  usize)`, `Relationship(RelationshipId)`, `Cell(CellId)`. The old `location()`/`ErrorLocation`
  are gone.
- **adam-rs planner — set reconstruction (cold path only).** `planner/trace.rs` adds
  `recover_cycle` (one simple ordered cycle out of an SCC) and `minimal_infeasible_set`
  (deletion-filtering to a subset-minimal overconstrained group). `Error::Cycle` and
  `Error::FilterCycle` carry the loop in `Relationship`/`Cell` order; `Error::Conflict`
  carries the minimal `Relationship` set. `ReleaseFailure::NoAcyclicAssignment` now carries
  the cyclic `Assignment` so the loop can be traced.
- **adam-rs — full `Error` audit.** `MismatchedMethodCells` names both methods plus the
  differing cells; `DuplicateMethodOutputs` names both methods plus the shared/repeated
  cells (self-dup and cross-method shapes); `InvalidCellKind` names the bad output cell;
  `InvalidConditional` names the implicated relationship(s)/cell for its cross-referencing
  cases. Genuinely single-object build-time errors (`InvalidId`, `InvalidOutput`,
  `InvalidFilter`, `InvalidRequirement`) were reviewed and left as-is (documented in the
  spec's §3 rationale).
- **cel-parser — multi-span rendering.** `SpanLabel` + `format_multi_span` render a title
  with several labelled carets (the backtrace); `ParseError` gains optional `secondary`
  spans (`with_secondary`) so build-time structural errors render multi-caret too. The
  single-span paths are unchanged.
- **adam-lang — span tables + resolution.** `ParsedSheet` gains `relationship_spans` and
  `cell_spans` (alongside `method_spans`); `error_labels::site_label` synthesizes per-variant
  human labels; `ParsedSheet::locate_error` resolves an error's sites to labelled spans; the
  `parse_relationship_decl`/`parse_conditional_decl` error arms resolve all sites into a
  primary + secondary `ParseError`.
- **adam-web-ui + begin.** `format_adam_error` takes `&ParsedSheet` and renders zero/one/many
  spans (Display / single caret / multi-span backtrace), keeping the CEL `SpanContext` fast
  path. `BuildOutcome` carries the whole `ParsedSheet`; `begin` holds one
  `Signal<ParsedSheet>` (the `MethodSpans` signal is gone). A `pick_default_example_name`
  helper keeps begin launching on a working example. `begin/examples/cycle.adm2` demonstrates
  the multi-span cycle backtrace.

## Verification

At branch head: `cargo fmt --all --check`, `cargo build --workspace` (zero warnings), all
three clippy invocations with `-D warnings`, and `RUSTDOCFLAGS=-D warnings cargo doc --lib
--no-deps --workspace` are clean. `cargo test --workspace` and `--doc --workspace` pass
except `adam-lang-book::tutorial::area_with_requirement`, which fails identically at the
branch base (untouched by this branch) — pre-existing, tracked as
[#199](https://github.com/stlab/cel-rs/issues/199).

## Deliberately deferred / out of scope

- **#199** — `adam-lang-book::tutorial::area_with_requirement` is a pre-existing tutorial
  failure unrelated to this work; not addressed here.
- The `Assignment::solve_acyclic` generalization around filter edges (issue #153) is
  unchanged; `FilterCycle` remains sound-but-incomplete as before.
- adam-lsp still renders only the primary span for build-time `ParseError`s; surfacing
  secondary spans as LSP related-information is a possible later enhancement (spec §5).
- Build-time structural diagnostics label the secondary carets (baseline method, differing
  cells) but not the primary caret itself — the primary binding shows the error title with a
  bare caret. This is cosmetic (the diagnostic is still clear) and giving the primary its own
  per-caret label would mean threading a primary label through cel-parser's `ParseError`
  rendering; deferred as UI polish, not blocking.

## Remaining

Open the PR (`Fixes #188`). Nothing else outstanding.
