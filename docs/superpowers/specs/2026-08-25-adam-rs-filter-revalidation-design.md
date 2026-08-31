# Filter Revalidation on Bound-Argument Change (adam-rs)

**Date:** 2026-08-25
**Branch:** worktree-adam-rs-filter-revalidation
**Status:** Approved (design), not yet implemented

## Summary

A [`Filter`](../../../adam-rs/src/filter.rs)'s dynamic argument cells (e.g. a shared `max`
bound another cell's range clamp reads) can change independently of the filtered cell
itself. Today nothing re-validates the filtered cell when that happens: `write()` only
conforms the cell actually written, and `propagate()`'s existing derived-value diagnostic
(§4 of
[the original input-filters design](2026-08-21-adam-rs-input-filters-design.md)) only
re-checks cells a method derived *this round* — a filter argument is never a relationship
input, so changing it never triggers that check either. This is tracked as
[issue #132](https://github.com/stlab/cel-rs/issues/132).

This design closes that gap generally, for both halves of a filtered cell's two possible
roles:

- **Source cells** (never produced by a method under the current plan): reapply the
  filter and correct the stored value in place, exactly as `write()`/`add_filter` already
  do for a direct write — folded into the planner's own dependency graph so it happens in
  the correct order relative to every relationship and every other filter, in a single
  pass, for any shape of dependency between filter arguments and relationships.
- **Derived cells** (produced by a method this round): unchanged — §4's existing
  diagnostic already handles this correctly once a full `propagate()` actually runs; the
  gap on this side is entirely in `begin` not knowing to *call* a full `propagate()` for a
  filter-argument write (§5).

Per this workspace's [library-first design principle](../../../CLAUDE.md#library-first-design),
this must hold for any sheet shape a caller can construct, not just `begin`'s example
sheets. §3 below states the one case this phase does not fully solve, and how it's
diagnosed rather than silently mishandled.

---

## 1. Motivation

[Issue #132](https://github.com/stlab/cel-rs/issues/132)'s repro:

```rust
let a = sheet.add_cell(50_i32);
let bound = sheet.add_cell(100_i32);
sheet.add_filter(a, Filter::from_fn_1(bound, |v: &i32, b: &i32| Ok((*v).min(*b)))).unwrap();
sheet.write(bound, 10_i32).unwrap();
// `a` is still 50 in storage: exceeds its own filter's bound (10), and nothing
// re-checks it, because `a` is never produced by a method.
```

This surfaced through a concrete `begin` UI bug: `begin/examples/inequality.adm2` has
`cell max_v = 100 filter 0..=200; cell a = 0 filter 0..=max_v; ...` with `a`/`b`/`c`
linked by `min`/`max` relationships. Shrinking `max_v` left whichever of `a`/`b`/`c` was
currently a plain source silently out of range, and the `begin` Inspector's number field
showed a value that matched neither the slider nor the graph. `inequality.adm2` is a
minimal example that *reveals* the gap; it is not the scope of the fix — see §3 and the
CLAUDE.md addition referenced above.

---

## 2. Source-cell reapply, folded into the planner

### 2.1 Why the planner, not `write()` or a post-`propagate()` pass

Two simpler alternatives were considered and rejected:

- **Reapply from `write()`'s own bookkeeping** (a reverse-adjacency lookup, "when cell X
  is written, immediately reapply any filter that lists X as an argument"). Rejected: it
  would give `write()` cross-cell side effects, breaking the "batch several writes, then
  call `propagate()` once" usage pattern this crate's API is built around. `write()` stays
  exactly as it is today.
- **A new phase in `propagate()` after `execute_plan()` runs** (mirroring §4's existing
  derived-value diagnostic, which *is* correctly a post-execution, read-only phase).
  Rejected for a *mutating* reclamp: if a source cell's filter reclamps it after
  `execute_plan()` already ran, any relationship that consumed that cell's pre-reclamp
  value earlier in the same round is left stale until some later `propagate()` call
  happens to run again — for `inequality.adm2`'s own `a := min(a, b); b := max(a, b)`
  relationship, this is not a hypothetical edge case, it is the central case.

The planner already knows, for the current round, exactly which cells are sources and
what order relationships must execute in to respect their dependencies
(`adam-rs/src/planner/digraph.rs`, `adam-rs/src/planner.rs`). A filter's dependency on its
argument cells is the same kind of edge as a relationship's dependency on its input
cells: fold it into the same dependency graph, and the same topological sort places the
reclamp at the one point in the round where it's simultaneously *after* everything the
filter depends on and *before* everything that depends on the filtered cell — for any
combination of relationships and filters, not just the ones `begin`'s examples happen to
construct.

### 2.2 Digraph and `Plan` changes

**`build_digraph`/`is_acyclic` themselves are untouched.** `is_acyclic` is called deep
inside `release::resolve`'s own candidate search (`matching.rs`'s recursive `solve`, one
call per candidate release), which knows nothing about filters and must not start
knowing about them here — that's exactly §3's stated boundary. Threading filter
awareness into that shared, hot-path function would silently make `release::resolve`
partially filter-aware in a way nobody decided on, rather than the deliberate,
documented boundary §3 describes.

Instead, `digraph::Node`'s existing `Cell(CellId)` variant (alongside
`Relationship(RelationshipId)`) is reused by a new, separate function,
`digraph::add_filter_edges(adj: &mut HashMap<Node, Vec<Node>>, cells: &SlotMap<CellId,
CellData>, assignment: &Assignment)`, called by `plan()` **once**, after
`release::resolve` has already finished searching and `build_digraph(&assignment,
relationships)` has already produced the base relationship-only graph. `add_filter_edges`
mutates that graph in place, adding one edge `Cell(arg) → Cell(filtered)` for every
argument `arg` of every filtered cell `filtered` that is a **source** under `assignment`
(not an output of any chosen method — the same classification `Sheet::is_source`
exposes, computed here directly from `assignment.chosen` since `Sheet::last_plan` isn't
updated until `plan()` returns). A filtered cell that is *not* a source under this
round's assignment (i.e. is derived) contributes no filter edges — §4's existing
diagnostic already covers it, purely as a read-only check after execution, with no
ordering constraint to satisfy.

`plan()`'s existing flow becomes: `release::resolve` → `build_digraph` (unchanged) →
`add_filter_edges` (new, mutates the same map) → `tarjan_scc` (unchanged) → walk
components. Only this last, already-computed-assignment stage ever sees filter edges.

`Plan::execution_order`'s element type changes from `(RelationshipId, usize)` to a new
`pub(crate) enum PlanStep { Method(RelationshipId, usize), FilterReclamp(CellId) }`. The
component-walking loop that currently does:

```rust
if let Node::Relationship(rel_id) = component[0] {
    execution_order.push((rel_id, assignment.chosen[&rel_id]));
}
```

now also matches `Node::Cell(id)` where `id` names a filtered source cell (computed once,
before the loop, as a `HashSet<CellId>`) and pushes `PlanStep::FilterReclamp(id)`; every
other `Node::Cell` component (a cell with no filter, or a filtered cell that's derived) is
skipped exactly as all `Node::Cell` components are today.

`Sheet::last_plan`'s type follows `Plan::execution_order`'s
(`Option<Vec<PlanStep>>`). `Sheet::is_source` and `Sheet::selected_method` — the two
existing readers of `last_plan` — adjust their filters to match on `PlanStep::Method`
instead of a bare tuple; their public signatures are unchanged.

### 2.3 `execute_plan` changes

`execute_plan`'s existing per-step loop gains a case for `PlanStep::FilterReclamp(id)`:
evaluate the cell's filter against its own current `effective()` value and its
arguments' current `effective()` values (both already correctly settled, by construction
of §2.2's ordering) and:

- `Ok(v)` where `v` doesn't equal the cell's current value (`cell.eq_fn`) — update
  `source` to `v`; mark `changed` exactly as any other mutating step does.
- `Ok(v)` that already equals the current value, or a wrong-type `v` — see below.
- `Err(e)` — the filter couldn't repair the current value against its new arguments.

The last two cases don't abort `propagate()` — filters are already established as
non-gating diagnostics (§4 of the original design), and a `FilterReclamp` step failing to
repair a cell is the source-cell analogue of §4's derived-cell "doesn't conform" outcome.
`execute_plan` takes a new `&mut Vec<(CellId, FilterViolation)>` out-parameter and pushes
into it for the `Err`/wrong-type cases (never for the two `Ok`-and-conforming cases, which
are silent, matching `write()`/`add_filter`'s existing silent-conform philosophy).

`propagate()` seeds its existing `last_filter_violations` map (§4's) with this vector's
entries before running §4's own derived-cell loop into the same map — the two populate
disjoint keys (a cell is either a source or derived this round, never both), so there's no
merge conflict. `propagate_without_replan()` passes its own scratch `Vec::new()` and
discards it: `last_filter_violations` stays pinned to the last full `propagate()`'s
result, exactly as today's docs already state, and as `is_forced`/`last_violated` already
do. It still *replays* `FilterReclamp` steps baked into the cached plan, though — the
mutation applies unconditionally on every call; only the *diagnostic map* stays pinned.
This means `propagate_without_replan()` gets correct source-cell reclamping for free
whenever the filtered cells' source/derived classification hasn't changed since the last
full `propagate()` — consistent with its existing precondition ("every cell written since
the last successful propagate()/propagate_without_replan() call satisfies `is_source(id)`").

### 2.4 Cold-start correctness

Because §2.2's edges are derived fresh from `self.cells[..].filter` on every `plan()`
call — not from any persisted history — issue #132's repro is fixed even on the *first*
`propagate()` call ever made on a sheet, including the case where the bound was written
before `propagate()` was ever called. There is no bootstrap gap.

---

## 3. The boundary: `release::resolve` doesn't know about filter edges

`release::resolve` (`adam-rs/src/planner/release.rs`) chooses *which* cells become
sources in the first place, searching only for a relationship-cycle-free assignment. It
has no visibility into the filter edges §2.2 adds. Consequently:

> `plan()` can report a cycle purely because of a filter dependency, even in a case where
> a *different*, equally-valid relationship assignment would have avoided it.

This is **sound but incomplete**: it never produces a wrong value, silently or otherwise.
When the combined graph (relationship edges + filter edges from §2.2) has a non-trivial
strongly-connected component, `plan()` returns a new error variant,
`Error::FilterCycle`, distinct from the existing `Error::Cycle` (which stays exactly what
it means today: a cyclic relationship assignment, independent of any filter). The existing
`debug_assert_eq!(component.len(), 1, ...)` in `plan()`'s component-walking loop — which
today encodes "`release::resolve` guarantees an acyclic digraph" — becomes a real runtime
check for both variants: `release::resolve`'s own guarantee still holds for the
relationship-only subgraph, but is no longer sufficient once filter edges are added, so
the check can no longer be `debug_assert`-only.

**Generalizing this** — making `release::resolve` itself aware of filter edges so it
searches for an assignment that's acyclic *including* filters, not just relationships — is
explicitly out of scope for this phase: it touches the hardest, most sensitive part of the
solver (`release.rs`/`matching.rs`) for a completeness gain, not a soundness one. Track it
as a new GitHub issue once this phase lands, referenced from `Error::FilterCycle`'s doc
comment and from `release::resolve`'s module doc.

---

## 4. Testing

Contract-derived, following this repo's existing convention:

**Planner (`adam-rs/src/planner.rs`, `adam-rs/src/planner/digraph.rs`):**
- A filtered source cell with a plain-source argument gets a `FilterReclamp` step
  positioned after nothing (the argument has no producer) and before any relationship
  step that consumes the filtered cell as input.
- A filtered source cell whose argument is itself produced by a relationship this round
  gets its `FilterReclamp` step positioned after that relationship's step — the
  generalization issue #132's repro didn't require, proven here explicitly so this isn't
  quietly narrowed back to "argument is a plain source."
- A filtered cell that is *derived* this round contributes no `FilterReclamp` step.
- A filter-argument dependency that closes a cycle with the relationship assignment's own
  edges returns `Error::FilterCycle`, distinct from a plain `Error::Cycle`.
- A sheet with no filters at all produces byte-identical `execution_order` `Method` steps
  to today (no `FilterReclamp` steps, no behavior change).

**Sheet (`adam-rs/src/sheet.rs`):**
- Issue #132's exact repro, via `propagate()`: after writing the bound, the filtered
  source cell reads back conforming.
- The `inequality.adm2`-shaped case: a two-method mutual relationship (`a := min(a, b);
  b := max(a, b)`) where the currently-source cell of the pair is also filtered against a
  bound that just shrank — the *other* (derived) cell's value reflects the corrected
  source, not the pre-reclamp one, within a single `propagate()` call.
- A `FilterReclamp` failure (`Err`, or wrong output type) is recorded in
  `last_filter_violations` without aborting `propagate()`, and leaves the cell's stored
  value unchanged.
- `propagate_without_replan()` re-applies a cached `FilterReclamp` step's correction using
  current argument values, but does not add/update entries in `last_filter_violations`.
- `Sheet::filter_dependents` (§5): presence/absence and multi-dependent aggregation,
  mirroring `filter_args`'s existing tests.

**begin:**
- `cell_needs_full_propagate` returns `true` for a cell with a non-empty
  `filter_dependents`.
- Manual/UI verification (`verifying-begin-ui`): adjusting `max_v` on `inequality.adm2`
  leaves the Inspector's number field, slider, and graph agreeing, and marks a
  now-out-of-range *derived* cell invalid (§4's existing diagnostic, now actually
  reachable because `begin` forces a full `propagate()`).

---

## 5. `begin` wiring

`Sheet` gains one new public, read-only query, independent of §2's mechanism:

```rust
/// Returns the live cells whose filter references `id` as one of its dynamic
/// arguments — the reverse of a filter's own argument list (`Sheet::filter_args`).
/// Empty if no live cell's filter references `id`.
pub fn filter_dependents(&self, id: CellId) -> &[CellId]
```

Backed by a `Sheet::filter_dependents: HashMap<CellId, Vec<CellId>>` reverse index built
in `add_filter` (cells and filters are never removed once added, so this index needs no
invalidation/cleanup logic — consistent with `terminal_cells` and every other per-cell
set `Sheet` already maintains for its own lifetime).

This is *not* needed for §2's source-cell reclamp — that now works correctly under either
`propagate()` or `propagate_without_replan()`, per §2.3. It exists purely so `begin`'s
`write_and_propagate` (`begin/src/inspector.rs`) can decide when it must call the full
`propagate()` instead of the cheaper `propagate_without_replan()`, because a full
`propagate()` is the only thing that refreshes §4's *derived*-cell diagnostic
(`last_filter_violations` for derived cells stays pinned otherwise, per §2.3). Extend
`cell_needs_full_propagate` with `!sheet.filter_dependents(id).is_empty()` alongside its
existing match-cell/requirement-input checks.

Once that's wired, ask #2 (mark a now-out-of-range derived cell invalid in the Inspector)
requires no further `begin` UI change: `cell_flags`'s existing
`status.filter_violated.contains(&id)` check (already exercising §4's `filter_violated_cells`)
picks it up as soon as `filter_violated_cells()` is actually current.

---

## 6. Non-goals for this phase

- Generalizing `release::resolve` to search around filter-induced cycles (§3) — tracked
  as a follow-up issue, not blocking this phase.
- [Issue #152](https://github.com/stlab/cel-rs/issues/152) (folding
  `propagate_without_replan`'s optimization into `propagate` itself) — a related but
  independent simplification, filed separately during this design's discussion.
- Any change to `begin`'s number-field display logic (`number_field_bounds` in
  `begin/src/inspector.rs`) — that fix (already pushed to `worktree-continue-filter-work`,
  this branch's parent, not yet merged to `main`) handles a Spectrum-widget display quirk
  and is unaffected by this design; this phase's payoff for `begin` is that the underlying
  values it displays become correct, and derived-cell violations become visible.
