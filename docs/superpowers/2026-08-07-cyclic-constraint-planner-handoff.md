# Cyclic Constraint Planner — Handoff

Status snapshot as of 2026-08-07, written for whoever picks up any further planner work in a new
conversation/context. Read
`docs/superpowers/specs/2026-08-04-cyclic-constraint-planner-design.md` first for the original
design and literature grounding (Dulmage–Mendelsohn decomposition, Tarjan SCC, transversal-matroid
greedy tearing) — this doc summarizes what's landed since, what was reconciled against `main`, and
what's left.

## What's done

**Matching + SCC + greedy-strength-release pipeline — landed prior to this session.**
`adam-rs/src/planner.rs` was already split into `planner/{matching,digraph,release,scc}.rs` per
the design doc: `Assignment::solve` (bipartite/hypergraph matching via augmenting-path search),
`digraph::is_acyclic` (builds the induced dependency graph and runs `scc::tarjan_scc`), and
`release::resolve` (releases cells as sources in descending strength order, keeping a release only
if a valid + acyclic assignment still exists). This replaced the original greedy flood-fill and
was the mechanism intended to resolve `begin/examples/diamond.adm2`'s collision pattern (R1{a,b,c}
and R2{b,c,d} sharing b,c).

**Reconciled against `main`'s independent PR #74 (this session).**
While this worktree was in progress, `main` merged `worktree-improve-plan-algorithm` — a
validation/simplification refactor (`docs/superpowers/specs/2026-08-06-relationship-method-constraints-and-plan-algorithm-design.md`)
that was **not** aimed at the diamond problem and had no knowledge of this worktree's design. It
rewrote `planner.rs` with a different (flood-fill/elimination) algorithm, and — as a side effect of
adding stricter double-write detection — added a regression test asserting that the exact diamond
collision pattern returns `Error::Conflict`. That assertion is provably wrong: a valid, acyclic,
strength-optimal plan exists for that scenario (verified by hand-tracing the bipartite structure);
main's simpler algorithm just can't find it, the same class of order-dependent incompleteness the
original flood-fill had, manifesting as a false conflict instead of a wrong answer.

Merged `main` into this branch (commit `c590d69`) and reconciled file-by-file:

- **Kept** this worktree's matching+SCC `planner.rs` (`git checkout --ours` on the one real
  conflict) rather than adopting main's flood-fill rewrite — only this design can, in principle,
  resolve the diamond case correctly.
- **Adopted from main:** the `Error::MismatchedMethodCells` / `Error::DuplicateMethodOutputs`
  variants (replacing this branch's less-specific `Error::InvalidMethod` for the same checks in
  `Sheet::add_relationship`), plus main's extra check this branch was missing — a single method's
  own `outputs` list may not name the same cell twice. Renamed/retargeted this branch's two
  pre-existing tests for these checks (`adam-rs/src/sheet.rs`) to assert the new dedicated variants
  instead of `InvalidMethod`.
- **Adopted from main:** distinguishing `Error::Cycle` (a method assignment exists but every one is
  cyclic — a genuine algebraic loop with no external input) from `Error::Conflict` (no method
  assignment exists at all, cyclic or not). Previously this branch's planner returned `Conflict`
  for both.
- Dependency bumps (`slotmap` 1.0→1.1, `dioxus`/`dioxus-devtools` 0.7.9→0.7.10) merged cleanly.

**Found and fixed a real bug in the matching engine (this session).**
`Assignment::solve`'s augmenting-path search stops at the *first* method combination satisfying
the disjoint-claims constraint — it has no notion of acyclicity. `release::resolve` only checked
whether *that one* candidate was acyclic; if not, it gave up on releasing the cell entirely, even
when a *different* valid combination (never tried) would have been acyclic. Confirmed empirically
by building the literal `begin/examples/diamond.adm2` file through the real `adam-lang` parser
pipeline: writing `a` then `d` (making `d` strictly the higher-strength cell) still resulted in `a`
kept as a source and `d` silently overwritten back to its default — backwards from the "keep the
higher-strength cell" contract `release::resolve`'s own doc comment promises, and reproducible
regardless of which of `a`/`d` was written last.

Fix, in `adam-rs/src/planner/matching.rs`:

- Added `Assignment::solve_acyclic` — a recursive backtracking search over every combination of
  method choices (not just the first found), accepting a combination only if its induced digraph
  is acyclic (via `super::digraph::is_acyclic`). Exponential in the number of active relationships
  in the worst case; acceptable at `adam-rs`'s target scale (UI property models: tens of cells,
  small relationship counts) per the design doc's own stated scope.

`adam-rs/src/planner/release.rs` changes:

- `resolve` now calls `solve_acyclic` for **every** cell in strength order, including cells that
  happen to already be unclaimed by the current best assignment. The old code took a shortcut —
  `if !current.claimed.contains_key(&cell) { released.insert(cell); continue; }` — that silently
  trusted whichever cells an arbitrary first-found matching left unclaimed, without ever verifying
  that was compatible with releasing every higher-strength cell still to come. That shortcut was
  the actual root cause: it let a lower-strength cell get "grandfathered in" as a source ahead of
  a higher-strength one purely because of method-declaration-order luck.
- `resolve`'s return type changed from `Option<Assignment>` to
  `Result<Assignment, ReleaseFailure>` (`NoAssignment` / `NoAcyclicAssignment`), mapped in
  `planner::plan` to `Error::Conflict` / `Error::Cycle` respectively — this is what let the
  Cycle/Conflict distinction adopted from main actually plug into this branch's algorithm.

**Verified, not just non-crashing.** Tightened `diamond_relationships_resolve_when_outer_cells_outrank_shared_cells`
(`adam-rs/tests/integration.rs`) and `diamond_collision_pattern_resolves_instead_of_failing`
(`adam-rs/src/planner/release.rs`) to assert the *specific* correct source set — `d` and `c` stay
sources, `a` and `b` are derived — rather than only cardinality/no-double-write, which is what let
the bug pass review previously. Watched both fail against the pre-fix code (RED) before applying
the fix (GREEN). Separately confirmed against the literal `begin/examples/diamond.adm2` file
through `adam_lang::AdamParser` (not a hand-written equivalent) via a throwaway test, deleted after
confirming.

Full check suite: `cargo build --workspace`, `cargo test --workspace`, `cargo test --doc
--workspace`, and all three required clippy invocations (`--workspace --exclude begin`, `-p begin
--no-default-features`, `-p begin`) all pass with zero warnings.

## What's left

1. **Manual UI sanity check in the running `begin` app** — the original design doc's Task 7 Step 5
   (open `diamond.adm2` in the Inspector, confirm no conflict surfaces) was marked optional and has
   not been done via the `verifying-begin-ui` skill. Not required — the fix is verified against the
   real `.adm2` file through the same parser pipeline `begin` uses — but worth doing if a UI
   regression is ever suspected in this area.
2. **`solve_acyclic`'s exponential worst case** is unaddressed, matching the original design doc's
   own "Future Work" note (incremental re-matching across `propagate()` calls) — not pursued since
   `adam-rs` targets UI-scale property models. Revisit only if profiling on a large sheet ever shows
   this mattering.
3. **`begin/examples/diamond.adm2` is not wired into `begin`'s bundled demo picker** (only
   `begin/assets/*.adm2` are). It's a standalone regression-reproduction fixture referenced by
   tests, not a demo end users see. No action needed unless someone wants it promoted to a demo.

## Key files

- `adam-rs/src/planner.rs` — `plan()` entry point; now maps `release::ReleaseFailure` to
  `Error::Conflict`/`Error::Cycle`.
- `adam-rs/src/planner/matching.rs` — `Assignment::solve` (original, acyclicity-unaware) and the
  new `Assignment::solve_acyclic` + `search_acyclic` (this session's fix).
- `adam-rs/src/planner/release.rs` — `release::resolve` (rewritten to drop the buggy shortcut) and
  `ReleaseFailure`.
- `adam-rs/src/planner/digraph.rs`, `scc.rs` — unchanged this session.
- `adam-rs/src/error.rs` — `Error::MismatchedMethodCells`/`DuplicateMethodOutputs` (adopted from
  main), `Error::Cycle` (now actually reachable from the matching-based planner).
- `adam-rs/tests/integration.rs` — `diamond_relationships_resolve_when_outer_cells_outrank_shared_cells`,
  `overlapping_diamond_chain_resolves_via_cascade`,
  `mutually_dependent_relationships_with_no_external_input_remain_cycle` (renamed from
  `..._remain_conflict`), `mutually_dependent_relationships_return_cycle` (from main).
- `begin/examples/diamond.adm2` — the regression fixture this whole effort targets.
