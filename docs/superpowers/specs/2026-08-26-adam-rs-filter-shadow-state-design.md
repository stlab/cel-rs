# Filters on Source Cells as Live Self-Referential Corrections (adam-rs)

**Date:** 2026-08-26
**Branch:** worktree-filter-using-shadow-state
**Status:** Approved (design), not yet implemented

## Summary

A filter attached to a *source* cell (never produced by a relationship this round) is,
structurally, the same thing as a self-referencing method: it reads the cell's own
current value plus its argument cells' current values, and produces a new current
value for that same cell. `adam-rs` already has a mechanism purpose-built for exactly
this shape — the `source`/`derived` shadow-state split from
[2026-08-02](2026-08-02-unlink-shadow-cells-design.md) — but the filter machinery added
afterward ([2026-08-21](2026-08-21-adam-rs-input-filters-design.md),
[2026-08-25](2026-08-25-adam-rs-filter-revalidation-design.md)) never used it: `write()`,
`add_filter`, and the planner's `PlanStep::FilterReclamp` step all bake the filter's
*output* directly into `source`, permanently discarding the raw value the caller
actually wrote.

This design makes a source cell's filter behave exactly like a self-referencing method:
`source` holds the raw last-written value, untouched by the filter, forever; the
filter's live output goes into `derived`, recomputed fresh every `propagate()` call,
exactly the way `[a, b] -> [a] { min(a, b) }` already works. This closes a
shrinking-accumulator bug identical in shape to the one 2026-08-02 fixed for
relationships, and — as a direct consequence — makes filters fully consistent with
`Condition`/`Requirement`'s existing "diagnose, never gate" philosophy: `write()` no
longer synchronously rejects input a filter can't conform.

---

## 1. The bug this fixes

```rust
let a = sheet.add_cell(50_i32);
let bound = sheet.add_cell(100_i32);
sheet.add_filter(a, Filter::from_fn_1(bound, |v: &i32, b: &i32| Ok((*v).min(*b)))).unwrap();

sheet.write(bound, 10_i32).unwrap();
sheet.propagate().unwrap();
assert_eq!(*sheet.read::<i32>(a).unwrap(), 10);   // correct so far

sheet.write(bound, 100_i32).unwrap();
sheet.propagate().unwrap();
assert_eq!(*sheet.read::<i32>(a).unwrap(), 10);   // wrong: a's original 50 is gone
                                                    // for good; it can never spring back
```

`a`'s original `50` was overwritten into `source` the moment the bound first tightened
(`adam-rs/src/sheet.rs`'s `PlanStep::FilterReclamp` arm, `cell.source = v;`). Once that
happens, there is no way to recover it — the exact same failure mode
[2026-08-02](2026-08-02-unlink-shadow-cells-design.md) diagnosed for
`[a, b] -> [a] { min(a, b) }` before the `source`/`derived` split existed. The same
destruction happens one step earlier, in `write()` and `add_filter`'s own inline
transforms, which conform-and-store in one step — so even a single direct write of an
out-of-range value to a filtered cell already loses the raw input, with no bound change
required to trigger it.

## 2. Changes

### 2.1 `write()` — delete all filter special-casing

`write()` no longer inspects `self.cells[id].filter` at all. It stores the raw value
into `source` and clears `derived`, exactly as it already does for a cell with no
filter:

```rust
pub fn write<T: Any + 'static>(&mut self, id: CellId, value: T) -> Result<(), Error> {
    if self.terminal_cells.contains(&id) {
        return Err(Error::TerminalCell);
    }
    let cell_type = self.cells.get(id).ok_or(Error::InvalidId)?.type_id;
    if cell_type != TypeId::of::<T>() {
        return Err(Error::TypeMismatch { expected: cell_type, found: TypeId::of::<T>() });
    }

    self.next_strength += 1;
    let cell = &mut self.cells[id];
    cell.strength = self.next_strength | (1u64 << 63);
    cell.source = Box::new(value);
    cell.derived = None;
    Ok(())
}
```

This deletes §3.1 of the 2026-08-21 design outright: there is no more synchronous
conform-or-reject step, and `write()` can no longer return `Error::MethodFailed` for a
filter reason. `read(id)` immediately after `write(id, ...)` now shows the *raw* value
until the next `propagate()` call — the same rule that already governs every other
cell in the sheet (`read()` reflects state as of the last `propagate()`, not a
per-write side effect).

### 2.2 `add_filter` — keep structural validation, delete the retroactive conform

Every structural check stays exactly as it is (missing cell, terminal cell,
already-filtered cell, value-type mismatch, self-referencing arg, missing/mismatched
arg cell) — those are about API misuse, not value conformance, and none of them
evaluate the filter's function. What's deleted is the block that does:

```rust
// deleted
let args: Vec<&dyn Any> = filter.0.args.iter().map(|&a| self.cells[a].effective()).collect();
let conformed = (filter.0.function)(self.cells[cell].source.as_ref(), &args)
    .map_err(Error::MethodFailed)?;
if conformed.as_ref().type_id() != cell_type {
    return Err(Error::TypeMismatch { expected: cell_type, found: conformed.as_ref().type_id() });
}
```

`add_filter`'s tail becomes:

```rust
for &arg in &filter.0.args {
    self.filter_dependents.entry(arg).or_default().push(cell);
}
self.cells[cell].filter = Some(filter.0);
Ok(())
```

`add_filter` can no longer return `Error::MethodFailed` or a value-derived
`Error::TypeMismatch` — only the structural variants above. It never touches `source`
or `derived`: attaching a filter is not a fresh input and must not change the cell's
current effective value, which is already this section's existing stated principle
(2026-08-21 §3.2, "attaching a filter is not a fresh external input") — just now applied
consistently, with no exception for the value itself.

This also deletes §3.2's whole "cold start" special case and its documented boundary
note: since the very next `propagate()` (see §2.3) reclamps *any* filtered source cell
unconditionally, there is no longer a gap for an always-source, never-written-again cell
whose initial value doesn't conform — it's simply the ordinary first-round case of the
mechanism below, not a special one.

### 2.3 `execute_plan`'s `PlanStep::FilterReclamp` — the one real mechanism

The planner integration from [2026-08-25 §2](2026-08-25-adam-rs-filter-revalidation-design.md#2-source-cell-reapply-folded-into-the-planner)
— `digraph::add_filter_edges`, `PlanStep::FilterReclamp`, `Error::FilterCycle`, and the
topological placement they produce — is **unchanged**. That machinery already answers
"when in the round must this run," which has nothing to do with which slot the result
lands in. Only the step's body changes, to match a self-referencing method exactly:

```rust
PlanStep::FilterReclamp(id) => {
    let filter = self.cells[id]
        .filter
        .as_ref()
        .expect("plan() only emits FilterReclamp for a filtered cell");
    let args: Vec<&dyn Any> = filter.args.iter().map(|&a| self.cells[a].effective()).collect();
    // Self-referencing input: always source, never a possibly-shadowed `derived` —
    // same rule as any other self-referencing method (2026-08-02 design).
    let current = self.cells[id].source.as_ref();
    match (filter.function)(current, &args) {
        Ok(v) => {
            let cell_type = self.cells[id].type_id;
            if v.as_ref().type_id() != cell_type {
                filter_violations.push((id, FilterViolation::Failed(anyhow::anyhow!(
                    "filter returned a value of a different type than the cell"
                ))));
            } else {
                // Unconditional write, no equality check — matches every other
                // shadowed output's "no equality check" convention (2026-08-02 design).
                let cell = &mut self.cells[id];
                cell.derived = Some(v);
                if !cell.changed {
                    cell.changed = true;
                    self.changed_cells.push(id);
                }
            }
        }
        Err(e) => filter_violations.push((id, FilterViolation::Failed(e))),
    }
}
```

Two changes from today's implementation:

- **Reads `source`, not `effective()`, as its own "self" input.** Correct per the
  self-referencing-input rule: a self-referencing input must never observe a value
  *this same round's* execution of itself already produced (there is no such thing —
  a cell reclamps at most once per round — but reading `source` is what keeps this
  step's *own* semantics identical to every other self-referencing method, and is what
  makes `source` provably untouched by the filter across any number of rounds).
- **Writes to `derived`, unconditionally, with no `eq_fn` comparison.** There is no
  longer an `Ok(Some(v)) / Ok(None)` split — the filter's `Ok` output is authoritative
  the same way any method's output is, so it always lands in `derived`. This deletes
  the `eq_fn` read entirely from this path (it's still used by §4's read-only
  diagnostic, which does need a value comparison — see §2.5).
- **`Err` (or a wrong-type `Ok`) leaves `derived` unset.** `read()` falls back to
  `source` — the cell keeps showing its last raw input, not a stale prior correction
  and not a half-applied one. Still fully non-gating: `propagate()` does not abort.

### 2.4 `digraph::add_filter_edges` — a zero-argument filter must still get a node

Since §2.1–2.2 make the `FilterReclamp` step the *only* place a source cell's filter is
ever evaluated, it must fire for **every** filtered source cell, including one with no
dynamic arguments at all (`Filter::from_fn_0`). Today, `add_filter_edges` only inserts
an adjacency entry when iterating a filter's `args`:

```rust
for &arg in &filter.args {
    adj.entry(Node::Cell(arg)).or_default().push(Node::Cell(cell_id));
}
```

A zero-arg filter's `args` is empty, so this loop never runs, and `cell_id` is never
inserted into `adj` as a key *or* a value — unless the cell happens to also be part of
some relationship (which already puts it in the graph via `build_digraph`). A
standalone cell with a `from_fn_0` filter and no relationship membership therefore never
appears in any `tarjan_scc` component, and `plan()`'s `Node::Cell(id) if
filtered_source_cells.contains(&id)` arm is never reached for it — no `FilterReclamp`
step is ever emitted. This was harmless before this design (a zero-arg filter has
nothing that can go stale, so write()-time conforming alone was sufficient); it becomes
a silent, total loss of conformance once write()-time conforming is deleted.

Fix: `add_filter_edges` unconditionally ensures the filtered cell itself is a node,
before iterating its args:

```rust
pub(crate) fn add_filter_edges(
    adj: &mut HashMap<Node, Vec<Node>>,
    cells: &SlotMap<CellId, CellData>,
    assignment: &Assignment,
) {
    for (cell_id, cell) in cells.iter() {
        let Some(filter) = cell.filter.as_ref() else {
            continue;
        };
        if assignment.claimed.contains_key(&cell_id) {
            continue;
        }
        adj.entry(Node::Cell(cell_id)).or_default();
        for &arg in &filter.args {
            adj.entry(Node::Cell(arg))
                .or_default()
                .push(Node::Cell(cell_id));
        }
    }
}
```

`adj.entry(Node::Cell(cell_id)).or_default()` guarantees `cell_id` is a key with at
least an empty successor list — matching `tarjan_scc`'s own documented behavior for a
no-edge node (`single_node_no_edges_is_trivial_component`) — so it always lands in its
own trivial component and always gets a `PlanStep::FilterReclamp`, independent of arg
count or relationship membership.

### 2.5 `FilterViolation::NotConformed` becomes derived-cell-only

For a source cell, the filter's `Ok` output is now authoritative by construction —
there is nothing left for it to "not conform" to, the same way a plain method's output
is never checked against the value it's replacing. `NotConformed` keeps its exact
existing meaning for §4's read-only derived-cell diagnostic
(`Sheet::propagate`'s existing post-execution phase, entirely unchanged by this design):
a relationship produced a value for a filtered cell, and the filter — which has no
authority to correct a derived cell — observes that its own output differs from what's
there. Doc comment update on the enum:

```rust
pub enum FilterViolation {
    /// The filter succeeded but its output differs from the cell's current value.
    /// Only ever recorded for a *derived* cell (`Sheet::propagate`'s post-execution
    /// diagnostic phase) — a filtered source cell's filter output is unconditionally
    /// authoritative once computed (see `PlanStep::FilterReclamp`), so this variant
    /// never applies there.
    NotConformed,
    /// The filter's function itself returned an error, or returned a value of a
    /// different type than the cell. Can occur on either side: a derived cell's
    /// read-only diagnostic check, or a source cell's live reclamp (in which case the
    /// cell's stored value is left unchanged, falling back to whatever `source` last
    /// held).
    Failed(anyhow::Error),
}
```

## 3. What's unchanged

- **§4's derived-cell diagnostic** (`Sheet::propagate`, read-only, non-gating) —
  untouched, `eq_fn` comparison included. This is the mechanism that handles a filtered
  cell *losing* a strength competition to a relationship, e.g.:

  ```text
  cell a = 0 filter 0..=10;
  cell b = 11;                    // b is stronger (written more recently)
  relationship { a := b; b := a; } // planner selects a := b
  ```

  `a` is claimed by the selected method this round, so it is **derived**, not a source
  — `PlanStep::FilterReclamp` (§2.3) is never emitted for it; §2.3's `eq_fn` removal
  therefore never applies here. `a` instead goes through §4's unchanged diagnostic:
  `clamp(11, 0..=10) == 11` is false, so `FilterViolation::NotConformed` is recorded and
  `a` is left at `11` — the relationship's constraint stands, the filter only observes.
  §2.3 and §4 are mutually exclusive per cell per round (a cell is either a source or
  derived, never both, per 2026-08-25's existing disjoint-keys reasoning); which one
  runs is decided entirely by the planner's source/derived classification, not by
  anything this design changes.
- **The planner integration** (`digraph::add_filter_edges`, `PlanStep`,
  `Error::FilterCycle`, the `release::resolve` filter-blind boundary) — untouched. This
  design only changes what a `FilterReclamp` step *does*, never when it runs.
- **`Sheet::filter_dependents`, `filter_args`, `filter_kind`, `filter_range`** — all
  read-only queries, unaffected.
- **`propagate_without_replan()`** — still replays a cached `FilterReclamp` step using
  current argument values every call, per §2.3 of 2026-08-25; only what that replay
  writes changes (`derived`, not `source`).

## 4. Net simplification

Three overlapping filter-application code paths (`write()`'s inline transform,
`add_filter`'s retroactive conform, `execute_plan`'s reclamp) collapse into one: the
planner-driven live step is now the *only* place a source cell's filter output is ever
computed. `write()` and `add_filter` return to being exactly as simple for a filtered
cell as for an unfiltered one. Filters lose their status as the only gating diagnostic
construct in `adam-rs` — every diagnostic (`Condition`, `Requirement`, and now `Filter`)
follows the same "observe or self-correct, never abort a caller's operation" rule.

## 5. Non-goals

- No change to `Filter`'s public constructors (`from_fn_0`/`from_fn_1`/`from_fn_2`/`new`/`range`)
  or their contracts — a filter is still "conforms or errors," only *when* it's
  evaluated changes.
- No change to `begin`'s wiring (`cell_needs_full_propagate`, `write_and_propagate`):
  confirmed by reading `begin/src/inspector.rs`'s `write_and_propagate`, which already
  calls `propagate()`/`propagate_without_replan()` immediately after every `write()` in
  the same function, before checking `clamped_away` or `filter_violated_cells` — so
  deferring conformance from `write()`-time to the following `propagate()` is invisible
  to the UI.
- No revisiting of the `release::resolve` filter-blind boundary (2026-08-25 §3) — out of
  scope here exactly as it was there.

## 6. Testing

Contract-derived, per this repo's convention. Existing tests in `adam-rs/src/sheet.rs`
that assert the *old* write()/add_filter conforming behavior are testing a mechanism
this design deletes; they're replaced, not merely adjusted, because their premise no
longer holds.

**Delete** (assert `write()`/`add_filter` synchronously conform or reject a value —
no longer true):

- `add_filter_conforms_the_cells_current_value_immediately`
- `add_filter_leaves_a_conforming_value_unchanged`
- `add_filter_returns_method_failed_when_current_value_cannot_conform`
- `add_filter_resolves_a_dynamic_argument_cells_current_value`
- `add_filter_returns_type_mismatch_when_the_filters_function_returns_the_wrong_type`
- `write_returns_type_mismatch_when_the_filters_function_returns_the_wrong_type`
- `write_conforms_a_value_through_the_cells_filter`
- `write_rejects_a_value_the_filter_cannot_conform`
- `write_through_a_filter_still_bumps_strength` (its premise — "conformed away from what
  was passed in" — no longer applies; ordinary strength behavior for a filtered cell is
  now just ordinary strength behavior, already covered elsewhere)

**Rewrite:**

- `from_fn_2_conforms_values_through_sheet_using_both_dynamic_arguments` — drop the
  `write()`-then-`read()`-without-`propagate()` assertions; keep it as a `propagate()`-driven
  test of both dynamic arguments feeding one filter.

**Add** (all in `adam-rs/src/sheet.rs`'s `mod tests`, following existing naming style):

- `add_filter_does_not_change_the_cells_current_value` — attach a filter whose function
  would reject or transform the cell's current value; assert `add_filter` still returns
  `Ok(())` and `read()` is unchanged immediately after.
- `write_leaves_the_raw_value_in_source_until_propagate_conforms_it` — write an
  out-of-range value to a filtered cell; assert `read()` shows the raw value
  *before* `propagate()`, and the conformed value *after*.
- `filtered_source_cell_springs_back_to_its_original_value_when_a_bound_loosens` — this
  design's motivating repro (§1): tighten a bound, propagate, assert the clamped value;
  loosen the bound back, propagate, assert the cell reads its *original* raw value, not
  the intermediate clamp.
- `propagate_after_add_filter_conforms_the_initial_value` — `add_cell` with an
  out-of-range value, `add_filter`, no `write()` at all; assert `read()` still shows the
  raw value pre-`propagate()` and the conformed value post-`propagate()` (closes the
  former §3.2 "cold start" special case as an ordinary case of this mechanism).
- `filter_reclamp_records_failed_violation_when_the_filters_function_returns_the_wrong_type`
  — the source-side analogue of the deleted `write_returns_type_mismatch...` test,
  exercised via `propagate()`: asserts `FilterViolation::Failed`, and that the cell's
  stored value is unchanged.
- `propagate_conforms_a_standalone_zero_argument_filtered_cell` — regression test for
  §2.4: `add_cell` an out-of-range value, attach a `Filter::from_fn_0` (no dynamic
  args), no relationship at all; assert `read()` shows the conformed value after
  `propagate()`.

In `adam-rs/src/planner/digraph.rs`'s `mod tests`:

- `add_filter_edges_adds_a_node_for_a_zero_argument_filter_with_no_relationship_membership`
  — regression test for §2.4: build an `Assignment` for a sheet with one unclaimed cell
  carrying a `Filter::from_fn_0` and no relationship at all; assert `adj` contains an
  entry for `Node::Cell(that_cell)` (even though it's an empty successor list) after
  `add_filter_edges` runs.

**Comment fix (no behavior change):**

- `propagate_reports_failed_when_the_filter_errors_on_a_derived_value`'s setup comment
  ("`add_filter` re-checks the cell's *current* value immediately (see §3.2 of the
  design)...") references the retroactive-conform behavior §2.2 deletes. The test's
  body and assertions are unaffected — update the comment to stop citing §3.2.
- `filter_reclamp_failure_is_recorded_without_aborting_propagate_or_changing_the_cell`'s
  setup comment ("Accept anything up to `bound` so add_filter's own immediate re-check
  ... succeeds") has the same issue — update it for the same reason.

**Unchanged and still meaningful as-is:**

- `write_without_a_filter_behaves_exactly_as_before`
- `propagate_reclamps_a_filtered_source_cell_when_its_argument_changes` (2026-08-25) —
  its assertions are on `read()` *after* `propagate()`, which still holds; only the
  internal `source`-vs-`derived` mechanics behind it change.
- `propagate_reclamps_before_a_relationship_consumes_the_reclamped_value` (2026-08-25) —
  same reasoning.
- `filter_reclamp_failure_is_recorded_without_aborting_propagate_or_changing_the_cell`
  (2026-08-25) — same reasoning; "changing the cell" now precisely means `source` is
  untouched and `derived` stays unset.
- `propagate_without_replan_reapplies_a_cached_filter_reclamp_but_does_not_touch_last_filter_violations`
  (2026-08-25) — same reasoning.
- All of §4's existing derived-cell diagnostic tests
  (`propagate_reports_no_violation_when_a_derived_value_conforms`,
  `propagate_reports_not_conformed_when_a_derived_value_violates_its_filter`,
  `propagate_reports_failed_when_the_filter_errors_on_a_derived_value`,
  `propagate_reports_failed_when_the_filter_returns_the_wrong_type_on_a_derived_value`,
  `propagate_never_flags_a_filtered_cell_that_stayed_a_plain_source`) — entirely
  untouched by this design.
- All `filter_args`/`filter_dependents`/`filter_kind`/`filter_range`/`filter_violation*`
  query tests — entirely untouched.
