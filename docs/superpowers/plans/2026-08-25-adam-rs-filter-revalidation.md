# Filter Revalidation on Bound-Argument Change (adam-rs) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close [issue #132](https://github.com/stlab/cel-rs/issues/132): when a filtered
source cell's dynamic argument cell changes, fold reapplying that filter into the
planner's own dependency graph so the filtered cell is reclamped in the correct order
relative to every relationship and every other filter, in a single `propagate()` pass.

**Architecture:** `digraph::add_filter_edges` adds `Cell(arg) → Cell(filtered)` edges to
the planner's existing dependency digraph for every filtered cell that is a *source*
this round (a filtered *derived* cell is left to the existing derived-value diagnostic).
`Plan::execution_order`'s element type becomes `PlanStep::{Method, FilterReclamp}` so the
same topological sort produces reclamp steps in the correct position. `execute_plan`
executes `FilterReclamp` steps by re-running the cell's filter and mutating `source` in
place when the result differs, recording a non-gating `FilterViolation` otherwise.
`release::resolve` itself stays filter-unaware (documented boundary — see Task 1); a
combined-graph cycle purely from a filter edge becomes a new `Error::FilterCycle`,
distinct from `Error::Cycle`. Separately, `Sheet::filter_dependents` (a reverse index of
`filter_args`) lets `begin` decide when a write requires a full `propagate()` instead of
`propagate_without_replan()`, so `begin`'s Inspector actually re-runs the *derived*-cell
filter diagnostic (§4 of the original filters design) that's already implemented but
today never triggered by a plain source write.

**Tech Stack:** Rust, `adam-rs` crate (planner internals) and `begin` (one `inspector.rs`
predicate) — no new dependencies.

**Spec:** [docs/superpowers/specs/2026-08-25-adam-rs-filter-revalidation-design.md](../specs/2026-08-25-adam-rs-filter-revalidation-design.md)

## Global Constraints

- `cargo fmt --all` must be clean before any commit (enforced by the pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce **zero** compiler
  warnings (not just clippy-clean).
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`,
  `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`, and
  `cargo clippy -p begin --all-targets -- -D warnings` must all pass.
- Every `pub`/`pub(crate)` function needs a contract-style `///` doc comment: summary
  sentence, `- Precondition:`/`- Postcondition:`/`# Errors`/`- Complexity:` bullets only
  where non-obvious or non-O(1); `debug_assert!` for precondition checks, never runtime
  errors for them.
- Unit tests are derived from the contract/public interface only — never from reading
  the implementation (planner-internals tests below are the one deliberate exception:
  `Plan`/`PlanStep`/`digraph` are `pub(crate)`, not part of the crate's public API, so
  their tests necessarily exercise `pub(crate)` internals directly, matching this
  module's own existing test style in `digraph.rs`/`planner.rs`).
- Arithmetic on signed integers uses `checked_*`, not wrapping — not applicable here (no
  new arithmetic), noted for completeness.
- `release::resolve`/`matching.rs`/`is_acyclic` are **not** touched by this plan — see
  Task 1's boundary note (design §3). Generalizing them to search around filter-induced
  cycles is out of scope, tracked as a separate follow-up issue.

---

### Task 1: `Error::FilterCycle` and the `release::resolve` boundary note

**Files:**
- Modify: `adam-rs/src/error.rs`
- Modify: `adam-rs/src/planner/release.rs`

**Interfaces:**
- Produces: `Error::FilterCycle` (unit variant), usable everywhere `Error` is already
  matched (the enum is `#[non_exhaustive]`).

- [ ] **Step 1: Open the follow-up GitHub issue this variant's doc comment references**

Design §3 explicitly asks that "generalizing `release::resolve` to search around
filter-induced cycles" be tracked as its own issue once this phase lands, referenced
from both `Error::FilterCycle`'s doc comment and `release::resolve`'s module doc. Open
it now so the number is available for Steps 4 and 6 below:

```bash
gh issue create \
  --title "Generalize release::resolve to search around filter-induced cycles" \
  --body "$(cat <<'EOF'
adam-rs/src/planner/release.rs's resolve() chooses which cells become sources by
searching only for a relationship-cycle-free assignment; it has no visibility into
the filter-argument edges digraph::add_filter_edges adds (see
docs/superpowers/specs/2026-08-25-adam-rs-filter-revalidation-design.md §3).

Consequently plan() can report Error::FilterCycle purely because of a filter
dependency, even in a case where a different, equally-valid relationship assignment
would have avoided it. This is sound but incomplete: it never produces a wrong value,
silently or otherwise.

Generalizing resolve() (and matching.rs's Assignment::solve/solve_acyclic) to search
for an assignment that's acyclic *including* filter edges is a completeness
improvement, not a soundness fix — tracked here rather than blocking the
filter-revalidation phase that introduced Error::FilterCycle.
EOF
)"
```

Note the returned issue number (referred to as `#N` below — substitute the real number
in Steps 4 and 6).

- [ ] **Step 2: Write the failing tests**

Add to the `tests` module at the bottom of `adam-rs/src/error.rs`, after
`invalid_filter_has_no_source`:

```rust
    #[test]
    fn filter_cycle_display_contains_cycle() {
        assert!(Error::FilterCycle.to_string().contains("cycle"));
    }

    #[test]
    fn filter_cycle_has_no_source() {
        assert!(std::error::Error::source(&Error::FilterCycle).is_none());
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p adam-rs filter_cycle`
Expected: FAIL to compile — `no variant named FilterCycle found for enum Error`.

- [ ] **Step 4: Add the variant and its `Display` arm**

In `adam-rs/src/error.rs`, add the variant right after `InvalidFilter` inside `pub enum
Error` (substitute the real issue number for `#N`):

```rust
    /// The combined dependency digraph — relationship edges plus a filtered source
    /// cell's argument edges (see `Sheet::propagate`'s planning pass) — has a
    /// non-trivial strongly connected component that is not purely a relationship
    /// cycle (that case is `Error::Cycle`). `release::resolve` guarantees the
    /// relationship-only subgraph is acyclic but has no visibility into filter edges,
    /// so this is sound but incomplete: a different, equally-valid relationship
    /// assignment might have avoided the cycle. See issue #N.
    FilterCycle,
```

And add the matching arm inside `impl std::fmt::Display for Error`, right after the
`InvalidFilter` arm:

```rust
            Error::FilterCycle => write!(
                f,
                "a filter's argument dependency closes a cycle with the selected methods"
            ),
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p adam-rs filter_cycle`
Expected: PASS (2 tests).

- [ ] **Step 6: Add the boundary note to `release::resolve`'s module doc**

In `adam-rs/src/planner/release.rs`, append this paragraph to the file's top `//!`
module doc comment (substitute the real issue number for `#N`):

```rust
//!
//! This module has no visibility into a filter's dynamic-argument dependencies —
//! `digraph::add_filter_edges` adds those edges to the digraph only *after*
//! `resolve` has already finished searching (see
//! `docs/superpowers/specs/2026-08-25-adam-rs-filter-revalidation-design.md` §3).
//! `resolve`'s acyclicity guarantee therefore holds only for the relationship-only
//! subgraph; `plan()` re-checks acyclicity once more after filter edges are added,
//! returning `Error::FilterCycle` (distinct from this module's own `Error::Cycle`)
//! if that combined graph turns out cyclic. Generalizing `resolve` itself to search
//! around filter edges is tracked as issue #N.
```

- [ ] **Step 7: Run the full test suite**

Run: `cargo test -p adam-rs`
Expected: PASS, no regressions.

- [ ] **Step 8: Commit**

```bash
git add adam-rs/src/error.rs adam-rs/src/planner/release.rs
git commit -m "feat(adam-rs): add Error::FilterCycle, document release::resolve's filter-blind boundary"
```

---

### Task 2: `digraph::add_filter_edges`

**Files:**
- Modify: `adam-rs/src/planner/digraph.rs`

**Interfaces:**
- Consumes: `crate::cell::CellData` (existing, `filter: Option<FilterData>` field),
  `super::matching::Assignment` (existing, `claimed: HashMap<CellId, RelationshipId>`
  field).
- Produces: `pub(crate) fn add_filter_edges(adj: &mut HashMap<Node, Vec<Node>>, cells:
  &SlotMap<CellId, CellData>, assignment: &Assignment)`, consumed by Task 3's `plan()`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module at the bottom of `adam-rs/src/planner/digraph.rs`. Update that
module's existing imports first — change:

```rust
    use super::*;
    use crate::{Method, Sheet};
    use std::collections::HashSet;
```

to:

```rust
    use super::*;
    use crate::{Filter, Method, Sheet};
    use std::collections::{HashMap, HashSet};
```

Then add these two tests after `purely_self_referencing_relationship_still_appears_as_a_node`:

```rust
    #[test]
    fn add_filter_edges_adds_edge_from_argument_to_filtered_source_cell() {
        let mut sheet = Sheet::new();
        let bound = sheet.add_cell(10_i32);
        let a = sheet.add_cell(5_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |x: &i32, b: &i32| Ok((*x).min(*b))),
            )
            .unwrap();

        let assignment =
            Assignment::solve(&sheet.relationships, &HashSet::new(), &HashSet::new()).unwrap();
        let mut adj: HashMap<Node, Vec<Node>> = HashMap::new();
        add_filter_edges(&mut adj, &sheet.cells, &assignment);

        assert_eq!(adj.get(&Node::Cell(bound)), Some(&vec![Node::Cell(a)]));
    }

    #[test]
    fn add_filter_edges_skips_a_filtered_cell_claimed_by_the_assignment() {
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(5_i32);
        let bound = sheet.add_cell(10_i32);
        let y = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(x, y, |v: &i32| Ok(*v))])
            .unwrap();
        sheet
            .add_filter(
                y,
                Filter::from_fn_1(bound, |v: &i32, b: &i32| Ok((*v).min(*b))),
            )
            .unwrap();

        let active: HashSet<_> = [rel].into_iter().collect();
        let assignment =
            Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        let mut adj: HashMap<Node, Vec<Node>> = HashMap::new();
        add_filter_edges(&mut adj, &sheet.cells, &assignment);

        assert!(adj.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs add_filter_edges`
Expected: FAIL to compile — `cannot find function add_filter_edges in this scope`.

- [ ] **Step 3: Write the implementation**

In `adam-rs/src/planner/digraph.rs`, change the top import line:

```rust
use crate::cell::CellId;
```

to:

```rust
use crate::cell::{CellData, CellId};
```

Then add this function after `build_digraph` (before `is_acyclic`):

```rust
/// Adds one edge `Cell(arg) → Cell(filtered)` for every dynamic argument `arg` of every
/// filtered cell `filtered` that is a **source** under `assignment` — not claimed as an
/// output by any of `assignment`'s chosen methods. A filtered cell that *is* claimed
/// (derived this round) contributes no edges: `Sheet::propagate`'s existing derived-value
/// diagnostic already covers it, as a read-only check with no ordering requirement.
///
/// Mutates `adj` in place. Called by [`super::plan`] once, after [`build_digraph`] has
/// already produced the base relationship-only graph, so the same topological sort places
/// each reclamp after everything its filter depends on and before everything that depends
/// on the filtered cell.
///
/// - Complexity: O(C · a) where C = cells with a filter, a = arguments per filter.
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
        for &arg in &filter.args {
            adj.entry(Node::Cell(arg))
                .or_default()
                .push(Node::Cell(cell_id));
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-rs add_filter_edges`
Expected: PASS (2 tests).

- [ ] **Step 5: Run the full test suite**

Run: `cargo test -p adam-rs`
Expected: PASS, no regressions.

- [ ] **Step 6: Commit**

```bash
git add adam-rs/src/planner/digraph.rs
git commit -m "feat(adam-rs): add digraph::add_filter_edges"
```

---

### Task 3: `PlanStep`, `Plan::execution_order`, and `plan()` integration

**Files:**
- Modify: `adam-rs/src/planner.rs`

**Interfaces:**
- Consumes: `digraph::add_filter_edges` (Task 2), `Error::FilterCycle` (Task 1).
- Produces:
  - `pub(crate) enum PlanStep { Method(RelationshipId, usize), FilterReclamp(CellId) }`
    (`Clone, Copy, PartialEq, Eq, Debug`), consumed by Task 4's `sheet.rs` changes.
  - `Plan::execution_order: Vec<PlanStep>` (was `Vec<(RelationshipId, usize)>`).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module at the bottom of `adam-rs/src/planner.rs`. First fix the one
existing test that reads the old tuple shape — replace:

```rust
        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();
        assert_eq!(plan.execution_order.len(), 1);
        assert_eq!(plan.execution_order[0].0, r1);
    }
```

(inside `plan_with_active_subset_ignores_inactive_relationship`) with:

```rust
        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();
        assert_eq!(plan.execution_order.len(), 1);
        assert!(matches!(plan.execution_order[0], PlanStep::Method(r, _) if r == r1));
    }
```

Then add `PlanStep` to that test module's imports — change:

```rust
    use crate::{Error, Method, Sheet};
    use std::collections::HashSet;
```

to:

```rust
    use crate::planner::PlanStep;
    use crate::{Error, Filter, Method, Sheet};
    use std::collections::HashSet;
```

Then add these five tests after `dead_method_not_selected_before_owning_relationship`:

```rust
    #[test]
    fn plan_with_no_filters_produces_only_method_steps_matching_relationship_selection() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();

        let active: HashSet<_> = sheet.relationships().collect();
        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();

        assert_eq!(plan.execution_order, vec![PlanStep::Method(rel, 0)]);
    }

    #[test]
    fn no_filter_reclamp_step_for_a_filtered_cell_that_is_derived_this_round() {
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(5_i32);
        let y = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(x, y, |v: &i32| Ok(*v))])
            .unwrap();
        sheet
            .add_filter(y, Filter::from_fn_0(|v: &i32| Ok((*v).clamp(0, 100))))
            .unwrap();

        let active: HashSet<_> = sheet.relationships().collect();
        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();

        assert!(
            !plan
                .execution_order
                .iter()
                .any(|s| matches!(s, PlanStep::FilterReclamp(id) if *id == y))
        );
    }

    #[test]
    fn filter_reclamp_positioned_before_consuming_relationship_when_argument_is_a_plain_source() {
        let mut sheet = Sheet::new();
        let bound = sheet.add_cell(10_i32);
        let a = sheet.add_cell(500_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |x: &i32, b: &i32| Ok((*x).min(*b))),
            )
            .unwrap();
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();

        let active: HashSet<_> = sheet.relationships().collect();
        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();

        let reclamp_pos = plan
            .execution_order
            .iter()
            .position(|s| matches!(s, PlanStep::FilterReclamp(id) if *id == a))
            .expect("a is a filtered source cell, so it must get a FilterReclamp step");
        let method_pos = plan
            .execution_order
            .iter()
            .position(|s| matches!(s, PlanStep::Method(r, _) if *r == rel))
            .expect("rel must be in the execution order");
        assert!(reclamp_pos < method_pos);
    }

    #[test]
    fn filter_reclamp_positioned_after_relationship_that_produces_its_argument() {
        let mut sheet = Sheet::new();
        let q = sheet.add_cell(10_i32);
        let bound = sheet.add_cell(0_i32);
        let bound_rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(q, bound, |x: &i32| Ok(*x))])
            .unwrap();
        let a = sheet.add_cell(500_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |x: &i32, b: &i32| Ok((*x).min(*b))),
            )
            .unwrap();

        let active: HashSet<_> = sheet.relationships().collect();
        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();

        let reclamp_pos = plan
            .execution_order
            .iter()
            .position(|s| matches!(s, PlanStep::FilterReclamp(id) if *id == a))
            .expect("a is a filtered source cell, so it must get a FilterReclamp step");
        let method_pos = plan
            .execution_order
            .iter()
            .position(|s| matches!(s, PlanStep::Method(r, _) if *r == bound_rel))
            .expect("bound_rel must be in the execution order");
        assert!(method_pos < reclamp_pos);
    }

    #[test]
    fn a_filter_argument_cycle_returns_filter_cycle_error() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(b, |x: &i32, bound: &i32| Ok((*x).min(*bound))),
            )
            .unwrap();

        let active: HashSet<_> = sheet.relationships().collect();
        let result = crate::planner::plan(&sheet.cells, &sheet.relationships, &active);
        assert!(matches!(result, Err(Error::FilterCycle)));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs --lib planner::`
Expected: FAIL to compile — `cannot find type PlanStep in this scope` (and, once that's
fixed by Step 3, several of the new tests fail their assertions since `plan()` doesn't
add filter edges yet).

- [ ] **Step 3: Add `PlanStep` and change `Plan::execution_order`'s type**

In `adam-rs/src/planner.rs`, change the `use digraph::...` import line:

```rust
use digraph::{Node, build_digraph};
```

to:

```rust
use digraph::{Node, add_filter_edges, build_digraph};
```

Then replace the `Plan` struct's `execution_order` field and add `PlanStep` right above
it:

```rust
/// One step of a [`Plan`]'s `execution_order`: either a selected method, or reapplying a
/// source cell's filter against its (now-settled) current argument values.
///
/// See `docs/superpowers/specs/2026-08-25-adam-rs-filter-revalidation-design.md` §2.2.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PlanStep {
    /// Execute method `usize` of relationship `RelationshipId`.
    Method(RelationshipId, usize),
    /// Reapply this cell's filter against its current value and its filter arguments'
    /// current values, in place — the source-cell analogue of a method execution.
    FilterReclamp(CellId),
}

/// The output of the planning pass.
pub(crate) struct Plan {
    /// Selected steps (methods and filter reclamps) in execution order.
    pub(crate) execution_order: Vec<PlanStep>,
    /// Cells that can never be a source under the relationships this plan considered.
    /// See [`forced_output_cells`].
    pub(crate) forced_outputs: HashSet<CellId>,
    /// Active relationships with exactly one alive method after the forced-output
    /// fixpoint (see [`forced_output_cells`]) — the planner has no alternative method
    /// to choose for these, regardless of cell strength.
    pub(crate) forced_relationships: HashSet<RelationshipId>,
}
```

- [ ] **Step 4: Rewrite `plan()`'s component-walking loop**

Still in `adam-rs/src/planner.rs`, replace the body of `plan()` from `let adj =
build_digraph(...)` through the `Ok(Plan { ... })` at the end with:

```rust
    let mut adj = build_digraph(&assignment, relationships);
    add_filter_edges(&mut adj, cells, &assignment);

    let filtered_source_cells: HashSet<CellId> = cells
        .iter()
        .filter(|&(id, cell)| cell.filter.is_some() && !assignment.claimed.contains_key(&id))
        .map(|(id, _)| id)
        .collect();

    let mut components = scc::tarjan_scc(&adj);
    components.reverse();

    let mut execution_order: Vec<PlanStep> = Vec::new();
    for component in components {
        if component.len() != 1 {
            return Err(Error::FilterCycle);
        }
        match component[0] {
            Node::Relationship(rel_id) => {
                execution_order.push(PlanStep::Method(rel_id, assignment.chosen[&rel_id]));
            }
            Node::Cell(id) if filtered_source_cells.contains(&id) => {
                execution_order.push(PlanStep::FilterReclamp(id));
            }
            Node::Cell(_) => {}
        }
    }

    let method_count = execution_order
        .iter()
        .filter(|step| matches!(step, PlanStep::Method(..)))
        .count();
    if method_count != active.len() {
        return Err(Error::Conflict);
    }

    let forced_relationships: HashSet<RelationshipId> = alive
        .iter()
        .filter(|(_, methods)| methods.iter().filter(|&&is_alive| is_alive).count() == 1)
        .map(|(&rel_id, _)| rel_id)
        .collect();

    Ok(Plan {
        execution_order,
        forced_outputs,
        forced_relationships,
    })
}
```

This removes the old `debug_assert_eq!(component.len(), 1, ...)` entirely — per design
§3, `release::resolve`'s acyclicity guarantee no longer covers the filter-augmented
graph, so the check must be a real, non-debug `Error::FilterCycle` return.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p adam-rs --lib planner::`
Expected: PASS.

- [ ] **Step 6: Run the full test suite**

Run: `cargo test -p adam-rs`
Expected: FAIL — `sheet.rs` won't compile yet (`execute_plan`, `is_source`,
`selected_method`, `post_process_strengths`, `propagate`, `propagate_without_replan`,
and `last_plan`'s field type all still assume the old `(RelationshipId, usize)` tuple).
This is expected; Task 4 fixes it. Confirm the *only* new failures are compile errors
in `adam-rs/src/sheet.rs`, not in `planner.rs`/`digraph.rs`.

- [ ] **Step 7: Commit**

```bash
git add adam-rs/src/planner.rs
git commit -m "feat(adam-rs): add PlanStep, fold filter edges into plan()'s digraph"
```

(This commit is deliberately non-compiling at the crate level — `sheet.rs` catches up in
Task 4. If your workflow requires every commit to build, squash Tasks 3 and 4 into one
commit instead; the two are split here only for reviewability.)

---

### Task 4: `sheet.rs` — `PlanStep`-aware execution, `FilterReclamp` semantics

**Files:**
- Modify: `adam-rs/src/sheet.rs`

**Interfaces:**
- Consumes: `PlanStep` (Task 3).
- Produces: no new public signatures — `execute_plan`'s private signature changes
  (adds a `&mut Vec<(CellId, FilterViolation)>` out-parameter), `last_plan`'s field type
  changes to `Option<Vec<PlanStep>>`. `Sheet::is_source`/`Sheet::selected_method`'s
  public signatures and documented behavior are unchanged.

- [ ] **Step 1: Write the failing tests**

Add to `sheet.rs`'s `mod tests` (after `propagate_never_flags_a_filtered_cell_that_stayed_a_plain_source`,
before `propagate_without_replan_does_not_recompute_filter_violations` — keeping the
new source-cell tests grouped with the existing filter-violation tests):

```rust
    #[test]
    fn propagate_reclamps_a_filtered_source_cell_when_its_argument_changes() {
        // Issue #132's exact repro.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(50_i32);
        let bound = sheet.add_cell(100_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |v: &i32, b: &i32| Ok((*v).min(*b))),
            )
            .unwrap();
        sheet.write(bound, 10_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 10);
    }

    #[test]
    fn propagate_reclamps_before_a_relationship_consumes_the_reclamped_value() {
        // The inequality.adm2-shaped case: a and b are linked by a two-method mutual
        // relationship (a := min(a, b); b := max(a, b)); a is the currently-source cell
        // of the pair and is filtered against a bound that just shrank. b (derived) must
        // reflect the corrected a, not the pre-reclamp one, within a single propagate().
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(50_i32);
        let b = sheet.add_cell(20_i32);
        let bound = sheet.add_cell(100_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |v: &i32, bnd: &i32| Ok((*v).min(*bnd))),
            )
            .unwrap();
        sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], b, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        sheet.propagate().unwrap();

        sheet.write(bound, 5_i32).unwrap();
        sheet.propagate().unwrap();

        // a reclamps to min(50, 5) = 5; b's method (a.min(b)) then reads the reclamped
        // a, not the stale 50.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 5);
    }

    #[test]
    fn filter_reclamp_failure_is_recorded_without_aborting_propagate_or_changing_the_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let bound = sheet.add_cell(100_i32);
        // Accept anything up to `bound` so add_filter's own immediate re-check (against
        // a's current value, 5, and bound's current value, 100) succeeds; the write to
        // `bound` below is what trips the filter.
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |v: &i32, b: &i32| {
                    if *v <= *b {
                        Ok(*v)
                    } else {
                        Err(anyhow::anyhow!("cannot conform"))
                    }
                }),
            )
            .unwrap();
        sheet.write(bound, 0_i32).unwrap();

        sheet.propagate().unwrap();

        assert!(matches!(
            sheet.filter_violation(a),
            Some(FilterViolation::Failed(_))
        ));
        // Rejected reclamp: the cell's stored value is left completely unchanged.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    }

    #[test]
    fn propagate_without_replan_reapplies_a_cached_filter_reclamp_but_does_not_touch_last_filter_violations() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(50_i32);
        let bound = sheet.add_cell(100_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |v: &i32, b: &i32| Ok((*v).min(*b))),
            )
            .unwrap();
        sheet.propagate().unwrap();
        assert!(sheet.filter_violation(a).is_none());

        // bound is itself a plain source (is_source(bound) holds), so rewriting it and
        // re-running only the cached plan is exactly propagate_without_replan's
        // documented precondition.
        sheet.write(bound, 10_i32).unwrap();
        sheet.propagate_without_replan().unwrap();

        assert_eq!(*sheet.read::<i32>(a).unwrap(), 10);
        // last_filter_violations is not recomputed by propagate_without_replan.
        assert!(sheet.filter_violation(a).is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs --lib`
Expected: FAIL to compile (`execute_plan` etc. still typed against the old tuple; the
crate doesn't build). This continues from Task 3's deliberately-broken intermediate
state.

- [ ] **Step 3: Update `use` imports for `PlanStep`**

In `adam-rs/src/sheet.rs`, add to the top-of-file `use crate::{ ... };` block (in
alphabetical position, right before `error::Error,`):

```rust
    output::{OutputData, OutputId},
    planner::PlanStep,
    relationship::{Method, RelationshipData, RelationshipId},
```

(i.e. insert `planner::PlanStep,` between the existing `output::{...}` and
`relationship::{...}` lines.)

- [ ] **Step 4: Change `last_plan`'s field type**

Change the field declaration:

```rust
    last_plan: Option<Vec<(RelationshipId, usize)>>,
```

to:

```rust
    last_plan: Option<Vec<PlanStep>>,
```

(`Sheet::new()`'s `last_plan: None,` initializer is unaffected.)

- [ ] **Step 5: Rewrite `post_process_strengths`**

Replace the function signature and body:

```rust
    fn post_process_strengths(&mut self, execution_order: &[(RelationshipId, usize)]) {
        let mut derived_strength = u64::MAX >> 1; // 0x7FFF_FFFF_FFFF_FFFF
        let mut seen: std::collections::HashSet<CellId> = std::collections::HashSet::new();
        for &(rel_id, method_idx) in execution_order {
            if let Some(rel) = self.relationships.get(rel_id)
                && let Some(method) = rel.methods.get(method_idx)
            {
                for &output in &method.outputs {
                    if seen.insert(output)
                        && let Some(cell) = self.cells.get_mut(output)
                    {
                        cell.strength = derived_strength;
                        derived_strength = derived_strength.saturating_sub(1);
                    }
                }
            }
        }
    }
```

with:

```rust
    fn post_process_strengths(&mut self, execution_order: &[PlanStep]) {
        let mut derived_strength = u64::MAX >> 1; // 0x7FFF_FFFF_FFFF_FFFF
        let mut seen: std::collections::HashSet<CellId> = std::collections::HashSet::new();
        for step in execution_order {
            let PlanStep::Method(rel_id, method_idx) = step else {
                continue;
            };
            if let Some(rel) = self.relationships.get(*rel_id)
                && let Some(method) = rel.methods.get(*method_idx)
            {
                for &output in &method.outputs {
                    if seen.insert(output)
                        && let Some(cell) = self.cells.get_mut(output)
                    {
                        cell.strength = derived_strength;
                        derived_strength = derived_strength.saturating_sub(1);
                    }
                }
            }
        }
    }
```

- [ ] **Step 6: Rewrite `execute_plan` to take a `PlanStep` slice and a filter-violation out-parameter**

Replace the whole function (signature through closing `}`):

```rust
    fn execute_plan(&mut self, execution_order: &[(RelationshipId, usize)]) -> Result<(), Error> {
        for &(rel_id, method_idx) in execution_order {
            let is_conditional = self.conditional_relationships.contains(&rel_id);
            let (outputs, output_ids, shadow_outputs) = {
                let method = &self.relationships[rel_id].methods[method_idx];
                let inputs: Vec<&dyn Any> = method
                    .inputs
                    .iter()
                    .map(|&id| {
                        if method.outputs.contains(&id) {
                            // Self-referencing input: always the pre-execution source,
                            // never a derived override from a previous execution.
                            self.cells[id].source.as_ref()
                        } else {
                            self.cells[id].effective()
                        }
                    })
                    .collect();
                let outputs = (method.function)(&inputs).map_err(Error::MethodFailed)?;
                let output_ids = method.outputs.clone();
                let shadow_outputs: Vec<bool> = method
                    .outputs
                    .iter()
                    .map(|o| method.inputs.contains(o) || is_conditional)
                    .collect();
                (outputs, output_ids, shadow_outputs)
            };

            if outputs.len() != output_ids.len() {
                return Err(Error::MethodFailed(anyhow::anyhow!(
                    "method produced {} outputs but relationship expects {}",
                    outputs.len(),
                    output_ids.len()
                )));
            }

            for ((cell_id, new_value), shadow) in
                output_ids.into_iter().zip(outputs).zip(shadow_outputs)
            {
                let cell = &mut self.cells[cell_id];
                let found = new_value.as_ref().type_id();
                if found != cell.type_id {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found,
                    });
                }
                if shadow {
                    cell.derived = Some(new_value);
                } else {
                    cell.source = new_value;
                }
                if !cell.changed {
                    cell.changed = true;
                    self.changed_cells.push(cell_id);
                }
            }
        }
        Ok(())
    }
```

with:

```rust
    fn execute_plan(
        &mut self,
        execution_order: &[PlanStep],
        filter_violations: &mut Vec<(CellId, FilterViolation)>,
    ) -> Result<(), Error> {
        for step in execution_order {
            match *step {
                PlanStep::Method(rel_id, method_idx) => {
                    let is_conditional = self.conditional_relationships.contains(&rel_id);
                    let (outputs, output_ids, shadow_outputs) = {
                        let method = &self.relationships[rel_id].methods[method_idx];
                        let inputs: Vec<&dyn Any> = method
                            .inputs
                            .iter()
                            .map(|&id| {
                                if method.outputs.contains(&id) {
                                    // Self-referencing input: always the pre-execution
                                    // source, never a derived override from a previous
                                    // execution.
                                    self.cells[id].source.as_ref()
                                } else {
                                    self.cells[id].effective()
                                }
                            })
                            .collect();
                        let outputs = (method.function)(&inputs).map_err(Error::MethodFailed)?;
                        let output_ids = method.outputs.clone();
                        let shadow_outputs: Vec<bool> = method
                            .outputs
                            .iter()
                            .map(|o| method.inputs.contains(o) || is_conditional)
                            .collect();
                        (outputs, output_ids, shadow_outputs)
                    };

                    if outputs.len() != output_ids.len() {
                        return Err(Error::MethodFailed(anyhow::anyhow!(
                            "method produced {} outputs but relationship expects {}",
                            outputs.len(),
                            output_ids.len()
                        )));
                    }

                    for ((cell_id, new_value), shadow) in
                        output_ids.into_iter().zip(outputs).zip(shadow_outputs)
                    {
                        let cell = &mut self.cells[cell_id];
                        let found = new_value.as_ref().type_id();
                        if found != cell.type_id {
                            return Err(Error::TypeMismatch {
                                expected: cell.type_id,
                                found,
                            });
                        }
                        if shadow {
                            cell.derived = Some(new_value);
                        } else {
                            cell.source = new_value;
                        }
                        if !cell.changed {
                            cell.changed = true;
                            self.changed_cells.push(cell_id);
                        }
                    }
                }
                PlanStep::FilterReclamp(id) => {
                    let outcome = {
                        let filter = self.cells[id]
                            .filter
                            .as_ref()
                            .expect("plan() only emits FilterReclamp for a filtered cell");
                        let args: Vec<&dyn Any> = filter
                            .args
                            .iter()
                            .map(|&a| self.cells[a].effective())
                            .collect();
                        let current = self.cells[id].effective();
                        match (filter.function)(current, &args) {
                            Ok(v) => {
                                let cell_type = self.cells[id].type_id;
                                if v.as_ref().type_id() != cell_type {
                                    Err(FilterViolation::Failed(anyhow::anyhow!(
                                        "filter returned a value of a different type than \
                                         the cell"
                                    )))
                                } else if !(self.cells[id].eq_fn)(v.as_ref(), current) {
                                    Ok(Some(v))
                                } else {
                                    Ok(None)
                                }
                            }
                            Err(e) => Err(FilterViolation::Failed(e)),
                        }
                    };
                    match outcome {
                        Ok(Some(v)) => {
                            let cell = &mut self.cells[id];
                            cell.source = v;
                            cell.derived = None;
                            if !cell.changed {
                                cell.changed = true;
                                self.changed_cells.push(id);
                            }
                        }
                        Ok(None) => {}
                        Err(violation) => filter_violations.push((id, violation)),
                    }
                }
            }
        }
        Ok(())
    }
```

Immediately above the `fn execute_plan(...)` line you just replaced, there is an
existing doc comment block:

```rust
    /// Executes `execution_order` without invoking the planner.
    ///
    /// # Errors
    ///
    /// - `Error::MethodFailed` — the method's function returned an error, or the method
    ///   produced a different number of outputs than declared.
    /// - `Error::TypeMismatch` — a method output's runtime type does not match the cell's
    ///   registered type.
    ///
    /// - Complexity: O(R·K) where R is the number of entries and K is the max cells per method,
    ///   plus per-method execution cost.
```

Replace it with:

```rust
    /// Executes `execution_order` without invoking the planner.
    ///
    /// A `PlanStep::FilterReclamp(id)` step re-evaluates `id`'s filter against its own
    /// current effective value and its filter arguments' current effective values, and
    /// updates `id`'s `source` in place if the result differs. A `PlanStep::Method`
    /// step's outputs follow the existing shadow/non-shadow rule, unchanged. A reclamp
    /// whose filter returns `Err`, or a value of the wrong type, is pushed into
    /// `filter_violations` instead of aborting; the cell's stored value is left
    /// untouched in that case.
    ///
    /// # Errors
    ///
    /// - `Error::MethodFailed` — a `PlanStep::Method` step's function returned an error,
    ///   or the method produced a different number of outputs than declared.
    /// - `Error::TypeMismatch` — a `PlanStep::Method` step's output runtime type does
    ///   not match the cell's registered type.
    ///
    /// - Complexity: O(R·K) where R is the number of entries and K is the max cells per method,
    ///   plus per-method execution cost.
```

- [ ] **Step 7: Update `propagate()`'s call sites and Phase 6b**

In `adam-rs/src/sheet.rs`'s `propagate()`:

Replace the Phase 1 pre-plan call:

```rust
            if !pre_active.is_empty() {
                let pre_plan = crate::planner::plan(&self.cells, &self.relationships, &pre_active)?;
                self.execute_plan(&pre_plan.execution_order)?;
            }
```

with (a scratch, discarded out-param — this pre-plan pass exists only to settle match
cells before branch evaluation, exactly like `propagate_without_replan`'s treatment of
`last_filter_violations`, so any reclamp it happens to run is superseded by Phase 3's
real run on the full active set and must not pollute the round's real diagnostic):

```rust
            if !pre_active.is_empty() {
                let pre_plan = crate::planner::plan(&self.cells, &self.relationships, &pre_active)?;
                self.execute_plan(&pre_plan.execution_order, &mut Vec::new())?;
            }
```

Replace the Phase 3 call:

```rust
        let plan = crate::planner::plan(&self.cells, &self.relationships, &active)?;
        self.execute_plan(&plan.execution_order)?;
```

with:

```rust
        let plan = crate::planner::plan(&self.cells, &self.relationships, &active)?;
        let mut source_filter_violations: Vec<(CellId, FilterViolation)> = Vec::new();
        self.execute_plan(&plan.execution_order, &mut source_filter_violations)?;
```

Then replace Phase 6b's `derived_this_round` collection loop and its seeding of
`last_filter_violations`:

```rust
        let mut derived_this_round: HashSet<CellId> = HashSet::new();
        for &(rel_id, method_idx) in &plan.execution_order {
            if let Some(method) = self
                .relationships
                .get(rel_id)
                .and_then(|r| r.methods.get(method_idx))
            {
                derived_this_round.extend(method.outputs.iter().copied());
            }
        }
        let mut last_filter_violations: HashMap<CellId, FilterViolation> = HashMap::new();
```

with:

```rust
        let mut derived_this_round: HashSet<CellId> = HashSet::new();
        for step in &plan.execution_order {
            if let PlanStep::Method(rel_id, method_idx) = step
                && let Some(method) = self
                    .relationships
                    .get(*rel_id)
                    .and_then(|r| r.methods.get(*method_idx))
            {
                derived_this_round.extend(method.outputs.iter().copied());
            }
        }
        // Seeded from execute_plan's source-cell reclamp failures above; disjoint keys
        // from the derived-cell loop below (a cell is a source or derived this round,
        // never both), so there's no merge conflict.
        let mut last_filter_violations: HashMap<CellId, FilterViolation> =
            source_filter_violations.into_iter().collect();
```

(The remainder of Phase 6b — the `for &cell_id in &derived_this_round { ... }` loop and
the final `self.last_filter_violations = last_filter_violations;` — is unchanged.)

- [ ] **Step 8: Update `selected_method`, `is_source`, and `propagate_without_replan`**

Replace `selected_method`:

```rust
    pub fn selected_method(&self, rel: RelationshipId) -> Option<usize> {
        self.last_plan
            .as_ref()?
            .iter()
            .find(|&&(r, _)| r == rel)
            .map(|&(_, idx)| idx)
    }
```

with:

```rust
    pub fn selected_method(&self, rel: RelationshipId) -> Option<usize> {
        self.last_plan.as_ref()?.iter().find_map(|step| match step {
            PlanStep::Method(r, idx) if *r == rel => Some(*idx),
            _ => None,
        })
    }
```

Replace `is_source`:

```rust
    pub fn is_source(&self, id: CellId) -> bool {
        let Some(plan) = &self.last_plan else {
            return false;
        };
        !plan.iter().any(|&(rel_id, method_idx)| {
            self.relationships
                .get(rel_id)
                .and_then(|r| r.methods.get(method_idx))
                .map(|m| m.outputs.contains(&id))
                .unwrap_or(false)
        })
    }
```

with:

```rust
    pub fn is_source(&self, id: CellId) -> bool {
        let Some(plan) = &self.last_plan else {
            return false;
        };
        !plan.iter().any(|step| match step {
            PlanStep::Method(rel_id, method_idx) => self
                .relationships
                .get(*rel_id)
                .and_then(|r| r.methods.get(*method_idx))
                .map(|m| m.outputs.contains(&id))
                .unwrap_or(false),
            PlanStep::FilterReclamp(_) => false,
        })
    }
```

Replace `propagate_without_replan`'s body:

```rust
    pub fn propagate_without_replan(&mut self) -> Result<(), Error> {
        let Some(execution_order) = self.last_plan.take() else {
            return Err(Error::Conflict);
        };
        self.clear_changed();
        let result = self.execute_plan(&execution_order);
        if result.is_ok() {
            self.post_process_strengths(&execution_order);
        }
        self.last_plan = Some(execution_order);
        result
    }
```

with:

```rust
    pub fn propagate_without_replan(&mut self) -> Result<(), Error> {
        let Some(execution_order) = self.last_plan.take() else {
            return Err(Error::Conflict);
        };
        self.clear_changed();
        // Discarded: this replays any cached FilterReclamp step's mutation
        // unconditionally, but last_filter_violations stays pinned to the last full
        // propagate()'s result, per this method's documented contract.
        let result = self.execute_plan(&execution_order, &mut Vec::new());
        if result.is_ok() {
            self.post_process_strengths(&execution_order);
        }
        self.last_plan = Some(execution_order);
        result
    }
```

Also add one sentence to `propagate_without_replan`'s doc comment, right after the
paragraph beginning "`is_forced` and `forced_cells` continue to reflect...":

```rust
    /// A cached [`PlanStep::FilterReclamp`] step is still re-executed on every call,
    /// using each argument's *current* effective value — only the `last_filter_violations`
    /// diagnostic map stays pinned, not the reclamp's mutation itself.
```

- [ ] **Step 9: Run tests to verify they pass**

Run: `cargo test -p adam-rs --lib`
Expected: PASS, including all pre-existing tests and the four new ones from Step 1.

- [ ] **Step 10: Run the full test suite, including doctests**

Run: `cargo test -p adam-rs`
Run: `cargo test --doc -p adam-rs`
Expected: PASS, no regressions.

- [ ] **Step 11: Commit**

```bash
git add adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): execute FilterReclamp plan steps, revalidate filtered source cells"
```

---

### Task 5: `Sheet::filter_dependents`

**Files:**
- Modify: `adam-rs/src/sheet.rs`

**Interfaces:**
- Consumes: `CellData.filter`/`FilterData.args` (existing).
- Produces: `pub fn filter_dependents(&self, id: CellId) -> &[CellId]`, consumed by
  Task 6's `begin` wiring.

- [ ] **Step 1: Write the failing tests**

Add to `sheet.rs`'s `mod tests`, after `filter_violation_cells_includes_root_causes_of_a_failed_violation`
(or after the last `filter_`-prefixed test, whichever is later in the file):

```rust
    #[test]
    fn filter_dependents_returns_the_cells_whose_filter_references_this_one() {
        let mut sheet = Sheet::new();
        let bound = sheet.add_cell(10_i32);
        let a = sheet.add_cell(5_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |v: &i32, b: &i32| Ok((*v).min(*b))),
            )
            .unwrap();
        assert_eq!(sheet.filter_dependents(bound), &[a]);
    }

    #[test]
    fn filter_dependents_is_empty_for_a_cell_no_filter_references() {
        let mut sheet = Sheet::new();
        let bound = sheet.add_cell(10_i32);
        let a = sheet.add_cell(5_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|v: &i32| Ok((*v).clamp(0, 100))))
            .unwrap();
        assert!(sheet.filter_dependents(bound).is_empty());
    }

    #[test]
    fn filter_dependents_is_empty_for_an_invalid_cell() {
        let sheet = Sheet::new();
        assert!(sheet.filter_dependents(CellId::default()).is_empty());
    }

    #[test]
    fn filter_dependents_aggregates_multiple_dependents_of_the_same_argument() {
        let mut sheet = Sheet::new();
        let bound = sheet.add_cell(10_i32);
        let a = sheet.add_cell(5_i32);
        let b = sheet.add_cell(5_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |v: &i32, bd: &i32| Ok((*v).min(*bd))),
            )
            .unwrap();
        sheet
            .add_filter(
                b,
                Filter::from_fn_1(bound, |v: &i32, bd: &i32| Ok((*v).min(*bd))),
            )
            .unwrap();
        let dependents = sheet.filter_dependents(bound);
        assert_eq!(dependents.len(), 2);
        assert!(dependents.contains(&a));
        assert!(dependents.contains(&b));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs filter_dependents`
Expected: FAIL to compile — `no method named filter_dependents found for struct Sheet`.

- [ ] **Step 3: Add the reverse-index field**

Add the field to `struct Sheet`, right after `last_filter_violations`:

```rust
    /// Reverse index of `filter_args`: for each cell, the live cells whose filter
    /// references it as one of its dynamic arguments. Built incrementally in
    /// `add_filter`; cells and filters are never removed once added, so this needs no
    /// invalidation, matching `terminal_cells` and every other per-cell set `Sheet`
    /// already maintains for its own lifetime.
    filter_dependents: HashMap<CellId, Vec<CellId>>,
```

and initialize it in `Sheet::new()`, right after `last_filter_violations: HashMap::new(),`:

```rust
            filter_dependents: HashMap::new(),
```

- [ ] **Step 4: Populate the index in `add_filter`**

In `add_filter`, insert this loop right before the existing `let cell_data = &mut
self.cells[cell];` line (i.e. after the value has been successfully conformed and every
earlier `?`/`return Err(...)` has already passed, but before the field writes that
commit the filter):

```rust
        for &arg in &filter.0.args {
            self.filter_dependents.entry(arg).or_default().push(cell);
        }

```

- [ ] **Step 5: Add the query method**

Add right after `filter_args`:

```rust
    /// Returns the live cells whose filter references `id` as one of its dynamic
    /// arguments — the reverse of a filter's own argument list ([`Sheet::filter_args`]).
    ///
    /// - Postcondition: empty if no live cell's filter references `id`.
    pub fn filter_dependents(&self, id: CellId) -> &[CellId] {
        self.filter_dependents
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p adam-rs filter_dependents`
Expected: PASS (4 tests).

- [ ] **Step 7: Run the full test suite**

Run: `cargo test -p adam-rs`
Expected: PASS, no regressions.

- [ ] **Step 8: Commit**

```bash
git add adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): add Sheet::filter_dependents"
```

---

### Task 6: `begin` wiring — `cell_needs_full_propagate`

**Files:**
- Modify: `begin/src/inspector.rs`

**Interfaces:**
- Consumes: `Sheet::filter_dependents` (Task 5).
- Produces: no new signatures — extends `cell_needs_full_propagate`'s existing
  `bool`-returning logic with one more disjunct.

- [ ] **Step 1: Write the failing test**

Add to `begin/src/inspector.rs`'s `mod tests`, right after
`cell_needs_full_propagate_true_for_cell_feeding_an_output_requirement`:

```rust
    #[test]
    fn cell_needs_full_propagate_true_for_a_cell_referenced_as_a_filter_argument() {
        use adam_rs::Filter;

        let mut sheet = Sheet::new();
        let bound = sheet.add_cell(10_i32);
        let a = sheet.add_cell(5_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |v: &i32, b: &i32| Ok((*v).min(*b))),
            )
            .unwrap();

        assert!(cell_needs_full_propagate(&sheet, bound));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p begin cell_needs_full_propagate_true_for_a_cell_referenced_as_a_filter_argument`
Expected: FAIL — `cell_needs_full_propagate` returns `false` (today's implementation
doesn't know about `filter_dependents`).

- [ ] **Step 3: Extend `cell_needs_full_propagate`**

Replace:

```rust
fn cell_needs_full_propagate(sheet: &Sheet, id: CellId) -> bool {
    let is_match_cell = sheet.conditionals().any(|cid| {
        sheet
            .conditional_match_cells(cid)
            .is_some_and(|c| c.contains(&id))
    });
    let feeds_requirement = sheet.outputs().any(|oid| {
        sheet.output_requirements(oid).is_some_and(|requirements| {
            requirements.iter().any(|&rid| {
                sheet
                    .requirement_inputs(rid)
                    .is_some_and(|inputs| inputs.contains(&id))
            })
        })
    });
    is_match_cell || feeds_requirement
}
```

with:

```rust
fn cell_needs_full_propagate(sheet: &Sheet, id: CellId) -> bool {
    let is_match_cell = sheet.conditionals().any(|cid| {
        sheet
            .conditional_match_cells(cid)
            .is_some_and(|c| c.contains(&id))
    });
    let feeds_requirement = sheet.outputs().any(|oid| {
        sheet.output_requirements(oid).is_some_and(|requirements| {
            requirements.iter().any(|&rid| {
                sheet
                    .requirement_inputs(rid)
                    .is_some_and(|inputs| inputs.contains(&id))
            })
        })
    });
    let feeds_a_filter = !sheet.filter_dependents(id).is_empty();
    is_match_cell || feeds_requirement || feeds_a_filter
}
```

Also update the function's doc comment — replace its final line:

```rust
/// - Complexity: O(number of conditionals + number of output requirements in the sheet).
```

with:

```rust
/// This also holds for a cell referenced as another cell's filter argument
/// ([`adam_rs::Sheet::filter_dependents`]): a source-cell filter reclamp is folded into
/// the planner's own dependency graph (see the adam-rs planner) and is only revalidated
/// by a full `Sheet::propagate()`'s own diagnostic phase, not by
/// `propagate_without_replan`.
///
/// - Complexity: O(number of conditionals + number of output requirements + number of
///   filter dependents of `id`).
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p begin cell_needs_full_propagate`
Expected: PASS (4 tests, including the 3 pre-existing ones).

- [ ] **Step 5: Run `begin`'s full test suite**

Run: `cargo test -p begin`
Expected: PASS, no regressions.

- [ ] **Step 6: Manual UI verification**

Use the `verifying-begin-ui` skill to confirm the end-to-end fix on the example that
originally surfaced this bug: open `begin/examples/inequality.adm2`, reduce `max_v`
below one of `a`/`b`/`c`'s current values, and confirm:

- The Inspector's number field, the live-range slider, and the graph view all agree on
  the corrected value for whichever of `a`/`b`/`c` is currently a plain source (no
  three-way disagreement, which was the original symptom).
- The *derived* cell of the pair (whichever of `a`/`b`/`c` a relationship currently
  produces) shows as invalid in the Inspector if its own filter is now violated by the
  corrected upstream value — `cell_flags`'s existing `status.filter_violated` check,
  now actually reachable because `write_and_propagate` calls a full `propagate()` for
  this write.

- [ ] **Step 7: Commit**

```bash
git add begin/src/inspector.rs
git commit -m "fix(begin): force a full propagate() when writing a cell referenced as a filter argument"
```

---

### Task 7: Full verification sweep

**Files:** none (verification only).

**Interfaces:** none.

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Expected: no diff (or, if it reformats something, review the diff, then re-run to
confirm idempotence).

- [ ] **Step 2: Build the whole workspace and check for warnings**

Run: `cargo build --workspace`
Expected: builds clean, **zero warnings** in the output (not just no errors).

- [ ] **Step 3: Test the whole workspace, including doctests, and check for warnings**

Run: `cargo test --workspace`
Run: `cargo test --doc --workspace`
Expected: all pass, zero warnings in either run's output.

- [ ] **Step 4: Clippy — all three required invocations**

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Run: `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
Run: `cargo clippy -p begin --all-targets -- -D warnings`
Expected: all three pass with no warnings.

- [ ] **Step 5: Fix anything Steps 2–4 surfaced**

If any warning or clippy lint appears, fix it in the relevant task's file and re-run
that specific check before moving on. Do not proceed to Step 6 with any warning
outstanding.

- [ ] **Step 6: Final commit, if Step 5 made changes**

```bash
git add -A
git commit -m "chore(adam-rs,begin): fix warnings/lints found by the full verification sweep"
```

If Step 5 made no changes, skip this step — there's nothing to commit.

- [ ] **Step 7: Open the PR referencing issue #132**

When opening the PR for this branch, include `Closes #132` in the PR description (the
design doc's issue) so it closes automatically on merge — do not close it manually
ahead of the PR.
