# Planner: Forced-Output Cells Design

**Date:** 2026-07-09
**Author:** Sean Parent (with Claude)
**Status:** Approved

## Problem

`property_model::planner::plan` selects, for each active relationship, one method to
execute, using cell `strength` (write-recency) to decide which cells are treated as
sources. The outer loop visits cells in descending-strength order and marks the *first*
undetermined cell it sees a source, unconditionally:

```rust
for &source in &cells_sorted {
    if determined.contains(&source) || pre_claimed.contains(&source) {
        continue;
    }
    determined.insert(source);
    source_cells.insert(source);
    // flood-fill from `source`...
}
```

This is correct when a relationship offers a genuine choice of methods (e.g. `a*b=c`
with a method per direction) — strength should decide which cell is exogenous. It is
**wrong** when a relationship has only one method: the method's output cell has no
alternative role and can never legitimately be a source, no matter its strength. The
planner doesn't know this, so a single-method relationship whose output cell happens to
outrank its input in strength either produces `Error::Conflict` or — worse, as seen in
`single_method_forced_direction` — silently strands the input cell as an orphan while the
output cell is treated as the source, so the relationship never re-derives its output at
all.

This generalizes beyond single-method relationships. Given:

```
R1: [a] -> [b]                       // single method
R2: [b] -> [c]  or  [c] -> [b]       // two methods
```

`b` is forced by `R1` alone. But once `b` is forced, `R2`'s `c → b` method is *also*
dead — it would double-write `b` — leaving `b → c` as `R2`'s only viable method. So `c`
is transitively forced too, even though neither of `R2`'s methods alone makes it so
structurally. Forced-ness must be computed to a fixpoint across relationships, not in a
single pass.

## Design

### Forced-output fixpoint

Add a pre-pass, run once per `plan()` call (both the Phase 1 pre-plan and the Phase 3
general plan already call `plan()` with their own `active` set — the pre-pass is scoped
to whatever `active` set that call receives, so conditional relationships that aren't
currently active contribute nothing).

Definitions:

- `pure_outputs(method) = method.outputs \ method.inputs` — cells the method writes but
  does not read. Self-referencing cells (in both `inputs` and `outputs`) are excluded;
  they remain eligible as sources, consistent with existing self-reference support.
- Each active relationship `R` starts with all of its methods "alive."
- `forced(R)` = the intersection of `pure_outputs(m)` over all of `R`'s currently-alive
  methods `m`. (If `R` has zero alive methods, `forced(R) = ∅`; this signals a genuine
  structural conflict that the existing end-of-plan `selected.len() != active.len()`
  check will catch once the flood-fill runs.)

Fixpoint loop:

```
loop:
    for each active R: recompute forced(R) from R's alive methods
    global_forced = union of forced(R) over all active R
    changed = false
    for each active R:
        others_forced = global_forced \ forced(R)
        for each alive method m of R:
            if pure_outputs(m) intersects others_forced:
                mark m dead
                changed = true
    if not changed: return global_forced
```

A method dies when one of its pure-output cells is guaranteed to be produced by a
*different* relationship — writing it too would always be a double-write conflict, so
that method can never be validly selected regardless of execution order.

This terminates: each iteration either kills at least one method or stops, and the total
number of methods is finite and only shrinks. Complexity is bounded by
`O(total_methods · R · M · K)` in the worst case (R = active relationships, M = methods
per relationship, K = cells per method) — one full re-scan per method killed. In
practice constraint graphs are small and convergence is fast (the two worked examples
above converge in 1–2 iterations).

The final `global_forced` set is used exactly like the current ad-hoc forced check: cells
in it are skipped by the outer strength-sorted loop in `plan()`, so they can never be
chosen as an initial source. The existing flood-fill (`is_eligible`, `is_feasible`,
reactive pre-claiming) is **unchanged** — once a forced cell like `b` is excluded from
source candidacy, the existing dynamic eligibility checks correctly resolve the rest
(e.g. `R2`'s `c → b` becomes ineligible once `b` is determined by `R1`, leaving `b → c`
as the only feasible method) without any special-casing.

### Relationship to the existing flood-fill

The fixpoint pre-pass and the flood-fill's reactive pre-claim mechanism
(`planner.rs:150-187`) are the same *shape* — both narrow down which method a
relationship will use as information becomes available — but operate on different
domains:

- The fixpoint pre-pass is **structural**: it only inspects method input/output shapes,
  never cell values or strengths, and must run to completion before any strength-based
  decision is made.
- The flood-fill's pre-claiming is **dynamic**: it depends on which specific cell was
  determined in which order during this particular traversal, which is driven by
  strength.

They share one piece of logic — computing `pure_outputs(method)` — which will be
extracted into a single helper function used by both. A deeper unification (e.g.
generalizing the flood-fill into a single worklist that also handles the structural case)
is not pursued here: it would entangle an order-independent static analysis with an
order-dependent traversal for no correctness benefit, and is noted below as a possible
future refactor rather than undertaken now.

### Public API: exposing forced cells

Forced cells are useful outside the planner: a UI binding a form to a `Sheet` can disable
fields that can never accept user input (writing to one has no effect once
`propagate()` runs again, regardless of priority). Mirroring the existing `is_source`
accessor:

```rust
impl Sheet {
    /// Returns `true` if `id` can never be a source under the currently active
    /// relationships — some active relationship's method structure guarantees the
    /// cell is always produced by a method, regardless of strength.
    ///
    /// Returns `false` if no propagation has run yet.
    pub fn is_forced(&self, id: CellId) -> bool;

    /// Iterates cells that are forced (see `is_forced`) as of the last `propagate()`
    /// call.
    pub fn forced_cells(&self) -> impl Iterator<Item = CellId> + '_;
}
```

`plan()`'s return type (`pub(crate) struct Plan`) gains a `forced_outputs: HashSet<CellId>`
field, populated by the fixpoint pre-pass. `Sheet` gains a `last_forced:
Option<HashSet<CellId>>` field, set from the Phase 3 (general) plan's `forced_outputs` in
`propagate()` — Phase 3's active set is already a superset of Phase 1's pre-plan set
(match-cell-producing relationships are unconditional, so they're part of Phase 3's base
active set too), so one cached set covers both phases. `propagate_without_replan()`
does not recompute it, consistent with how `last_plan` is already handled (documented via
the same staleness precondition already on that method).

## Error Handling

No changes to `Error`. Genuine conflicts (e.g. two single-method relationships that both
force the same cell, as in `mutually_dependent_relationships_return_conflict`) still
surface as `Error::Conflict` via the existing `selected.len() != active.len()` check —
the fixpoint doesn't need its own error path.

## Testing

- Existing `single_method_forced_direction` integration test (currently failing) should
  pass once this lands.
- New integration test for the transitive case: `R1: a→b` (single method), `R2: {b→c,
  c→b}` (two methods), with cell strengths arranged so `c` and/or `b` outrank `a`.
  Verifies `propagate()` succeeds and produces `b = f(a)`, `c = g(b)` regardless of
  relative strength among `b` and `c`.
- New unit/integration coverage for `is_forced` / `forced_cells`: forced cells reported
  correctly for a single-method relationship's output, not reported for cells in a
  genuine multi-method (choice) relationship, and correctly scoped to the active
  conditional branch (a cell forced only when a particular branch is active is not
  forced when that branch is inactive).
- Hand-traced (not re-run as new tests, since behavior is unchanged) against all existing
  planner and integration tests: `plan_with_active_subset_ignores_inactive_relationship`,
  `relationship_selected_at_most_once`, `conflict_returns_error`,
  `strength_drives_method_selection`, `mutually_dependent_relationships_return_conflict`,
  `self_ref_direct_clamp`, `self_ref_le_chain`, and the conditional-branch integration
  tests — none change outcome.

## Future Work

- A deeper unification of the structural fixpoint and the dynamic flood-fill into a
  single worklist algorithm, if a concrete need arises (e.g. performance on much larger
  constraint graphs). Not pursued now — see "Relationship to the existing flood-fill"
  above.
- The model-checker static analysis mentioned in the original property-model design doc
  (verifying a constraint graph is always solvable while preserving the strongest cell)
  could subsume this fixpoint as a byproduct, but is out of scope here.
