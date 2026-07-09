# Planner Forced-Output Cells Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the property-model planner so a relationship's forced direction (a cell that some active relationship's method structure guarantees will always be produced) is never overridden by unrelated cell strength, and expose that information as a public `Sheet` API for UI consumers.

**Architecture:** Add a fixpoint pre-pass (`forced_output_cells`) to `property-model/src/planner.rs::plan()` that computes, per `plan()` call, the set of cells no active relationship can ever treat as a source; exclude that set from the existing strength-sorted outer loop's source candidacy (the flood-fill itself is unchanged). Thread the result out through a new `Plan.forced_outputs` field into `Sheet::propagate()`, cached as `Sheet.last_forced`, and expose it via `Sheet::is_forced` / `Sheet::forced_cells`.

**Tech Stack:** Rust, `slotmap`, `std::collections::{HashMap, HashSet}`. No new dependencies.

## Global Constraints

- Every function gets a `///` contract-style doc comment (Summary, Preconditions as `debug_assert!`-backed bullets, Postconditions, Complexity when not O(1)) — see root `CLAUDE.md` "Documentation comments".
- Tests are derived from the contract/public interface only — do not test implementation details, do not test precondition violations.
- No heap allocations beyond what's already idiomatic here (`HashSet`/`HashMap`/`Vec` are fine; avoid `Box<dyn Trait>` / unnecessary `String`/`Vec` clones).
- `cargo fmt --all` must be run before every commit (enforced by the pre-commit hook).
- `cargo clippy --workspace --exclude begin -- -D warnings` and `cargo clippy -p begin --no-default-features -- -D warnings` must both be clean before the branch is considered done.
- Full spec: `docs/superpowers/specs/2026-07-09-planner-forced-outputs-design.md`.

---

### Task 1: Forced-output fixpoint in the planner

**Files:**
- Modify: `property-model/src/planner.rs` (module doc comment, imports, `Plan` struct, `plan()` body, new helper functions, `mod tests`)
- Modify: `property-model/tests/integration.rs` (new cascading regression test)

**Interfaces:**
- Consumes: existing `crate::planner::plan(cells, relationships, active) -> Result<Plan, Error>` signature (unchanged), `Method { inputs, outputs, .. }` (unchanged).
- Produces: `pub(crate) struct Plan { execution_order: Vec<(RelationshipId, usize)>, forced_outputs: HashSet<CellId> }` — Task 2 reads `plan.forced_outputs` from this.

- [ ] **Step 1: Write the failing planner.rs unit tests**

Open `property-model/src/planner.rs`. In the existing `#[cfg(test)] mod tests` block (starts at line 201), add these two tests after `conflict_returns_error`:

```rust
    #[test]
    fn single_method_output_is_forced_and_not_selected_as_source() {
        // b outranks a in strength (added second), but the relationship has only one
        // method (a -> b), so b must never be treated as a source.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 3))])
            .unwrap();

        let active: HashSet<_> = sheet.relationships().collect();
        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();

        assert!(plan.forced_outputs.contains(&b));
        assert!(!plan.forced_outputs.contains(&a));
        assert_eq!(plan.execution_order.len(), 1);
    }

    #[test]
    fn forced_outputs_cascade_through_adjacent_relationship() {
        // R1: a -> b (single method) forces b.
        // R2: b -> c or c -> b (two methods) — once b is forced by R1, R2's c -> b
        // method would double-write b, so it is eliminated, forcing c too.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(2_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 10))])
            .unwrap();
        sheet
            .add_relationship(vec![
                Method::from_fn_1_1(b, c, |x: &i32| Ok(*x + 1)),
                Method::from_fn_1_1(c, b, |x: &i32| Ok(*x + 1)),
            ])
            .unwrap();

        let active: HashSet<_> = sheet.relationships().collect();
        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();

        assert!(plan.forced_outputs.contains(&b));
        assert!(plan.forced_outputs.contains(&c));
        assert!(!plan.forced_outputs.contains(&a));
        assert_eq!(plan.execution_order.len(), 2);
    }
```

- [ ] **Step 2: Write the failing integration.rs test**

Open `property-model/tests/integration.rs`. Add this test immediately after `single_method_forced_direction` (line 34):

```rust
#[test]
fn forced_direction_cascades_through_adjacent_relationship() {
    // R1: a -> b (single method) forces b, regardless of strength.
    // R2: b -> c or c -> b (two methods) — b is already forced by R1, so R2's
    // c -> b method can never fire without double-writing b; c is forced too.
    // b and c are added after a and never written, so if strength alone decided
    // source selection, either could wrongly become a source instead of a.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(2_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);

    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 10))])
        .unwrap();
    sheet
        .add_relationship(vec![
            Method::from_fn_1_1(b, c, |x: &i32| Ok(*x + 1)),
            Method::from_fn_1_1(c, b, |x: &i32| Ok(*x + 1)),
        ])
        .unwrap();

    sheet.propagate().unwrap();

    assert_eq!(*sheet.read::<i32>(b).unwrap(), 20);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 21);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p property-model`
Expected: compile error in the `planner` unit tests (`no field \`forced_outputs\` on type \`Plan\``), which fails the whole `cargo test` invocation. This is the RED signal for both new tests (the pre-existing `single_method_forced_direction` integration test is also currently failing at runtime, for the same underlying reason — it will be fixed by this task too).

- [ ] **Step 4: Update the module doc comment**

In `property-model/src/planner.rs`, after the pre-claiming paragraph (ends at line 22, `... one of its inputs.`) and before the paragraph starting `//! Because a method is only selected...` (line 24), insert:

```rust
//!
//! **Forced outputs**: a cell that is a pure output — present in a method's `outputs`
//! but not its `inputs` — in every currently-viable method of some relationship can
//! never be a source, regardless of strength: that relationship has no alternative but
//! to produce it. [`forced_output_cells`] computes this as a fixpoint over all active
//! relationships (eliminating a method whose pure output is guaranteed to be produced by
//! a *different* relationship can force further cells), and the result is excluded from
//! source candidacy before the strength-ordered pass below begins.
```

- [ ] **Step 5: Add `HashMap` to the imports**

In `property-model/src/planner.rs`, change line 29 from:

```rust
use std::collections::{HashSet, VecDeque};
```

to:

```rust
use std::collections::{HashMap, HashSet, VecDeque};
```

- [ ] **Step 6: Add `forced_outputs` to `Plan`**

In `property-model/src/planner.rs`, change the `Plan` struct (lines 40-43) from:

```rust
pub(crate) struct Plan {
    /// Selected `(RelationshipId, method_index)` pairs in execution order.
    pub(crate) execution_order: Vec<(RelationshipId, usize)>,
}
```

to:

```rust
pub(crate) struct Plan {
    /// Selected `(RelationshipId, method_index)` pairs in execution order.
    pub(crate) execution_order: Vec<(RelationshipId, usize)>,
    /// Cells that can never be a source under the relationships this plan considered.
    /// See [`forced_output_cells`].
    pub(crate) forced_outputs: HashSet<CellId>,
}
```

- [ ] **Step 7: Compute and apply `forced_outputs` in `plan()`**

In `property-model/src/planner.rs`, change the start of the `plan()` body from:

```rust
    let mut determined: HashSet<CellId> = HashSet::new();
    // Subset of `determined`: cells whose value came from write(), not from a selected method.
    // Only source cells may serve as self-referencing inputs.
    let mut source_cells: HashSet<CellId> = HashSet::new();
    let mut pre_claimed: HashSet<CellId> = HashSet::new();
    let mut selected: Vec<(RelationshipId, usize)> = Vec::new();
    let mut selected_set: HashSet<RelationshipId> = HashSet::new();

    let mut cells_sorted: Vec<CellId> = cells.keys().collect();
    cells_sorted.sort_by_key(|&id| Reverse(cells[id].strength));

    for &source in &cells_sorted {
        if determined.contains(&source) || pre_claimed.contains(&source) {
            continue;
        }
```

to:

```rust
    let mut determined: HashSet<CellId> = HashSet::new();
    // Subset of `determined`: cells whose value came from write(), not from a selected method.
    // Only source cells may serve as self-referencing inputs.
    let mut source_cells: HashSet<CellId> = HashSet::new();
    let mut pre_claimed: HashSet<CellId> = HashSet::new();
    let mut selected: Vec<(RelationshipId, usize)> = Vec::new();
    let mut selected_set: HashSet<RelationshipId> = HashSet::new();

    // Cells that some active relationship's method structure guarantees will always
    // be produced by a method, regardless of strength. These can never be a source.
    let forced_outputs = forced_output_cells(relationships, active);

    let mut cells_sorted: Vec<CellId> = cells.keys().collect();
    cells_sorted.sort_by_key(|&id| Reverse(cells[id].strength));

    for &source in &cells_sorted {
        if determined.contains(&source)
            || pre_claimed.contains(&source)
            || forced_outputs.contains(&source)
        {
            continue;
        }
```

Then change the end of `plan()` from:

```rust
    if selected.len() != active.len() {
        return Err(Error::Conflict);
    }

    Ok(Plan {
        execution_order: selected,
    })
}
```

to:

```rust
    if selected.len() != active.len() {
        return Err(Error::Conflict);
    }

    Ok(Plan {
        execution_order: selected,
        forced_outputs,
    })
}
```

- [ ] **Step 8: Add the `pure_outputs` and `forced_output_cells` helper functions**

In `property-model/src/planner.rs`, insert these two functions immediately after the closing `}` of `plan()` and before `#[cfg(test)]`:

```rust
/// Returns the cells `method` writes but does not read.
///
/// Self-referencing cells (present in both `inputs` and `outputs`) are excluded: they
/// are read at their pre-execution value, so they retain their ordinary role as
/// potential sources.
fn pure_outputs(method: &Method) -> HashSet<CellId> {
    method
        .outputs
        .iter()
        .filter(|o| !method.inputs.contains(o))
        .copied()
        .collect()
}

/// Computes the cells that can never be a source under `active`: cells that some
/// relationship in `active` guarantees will always be produced by a method, regardless
/// of strength.
///
/// A cell is forced by a relationship when it is a [`pure_outputs`] member of every one
/// of that relationship's currently-alive methods. Starting with all methods alive, this
/// runs to a fixpoint: any method whose pure outputs include a cell forced by a
/// *different* relationship is eliminated (selecting it would always double-write that
/// cell), which can force more cells for the relationships that lost a method. The loop
/// stops once no relationship loses another method.
///
/// - Precondition: every `RelationshipId` in `active` is present in `relationships`.
///
/// - Complexity: O(D · R · M · K) where D = total methods eliminated across all
///   iterations (bounded by the total method count), R = active relationships,
///   M = methods per relationship, K = cells per method.
fn forced_output_cells(
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> HashSet<CellId> {
    let mut alive: HashMap<RelationshipId, Vec<bool>> = active
        .iter()
        .map(|&rel_id| (rel_id, vec![true; relationships[rel_id].methods.len()]))
        .collect();

    loop {
        let mut forced_per_rel: HashMap<RelationshipId, HashSet<CellId>> = HashMap::new();
        for &rel_id in active {
            let rel = &relationships[rel_id];
            let alive_methods = &alive[&rel_id];
            let mut forced: Option<HashSet<CellId>> = None;
            for (idx, method) in rel.methods.iter().enumerate() {
                if !alive_methods[idx] {
                    continue;
                }
                let po = pure_outputs(method);
                forced = Some(match forced {
                    None => po,
                    Some(prev) => prev.intersection(&po).copied().collect(),
                });
            }
            forced_per_rel.insert(rel_id, forced.unwrap_or_default());
        }

        let global_forced: HashSet<CellId> = forced_per_rel.values().flatten().copied().collect();

        let mut changed = false;
        for &rel_id in active {
            let others_forced: HashSet<CellId> = global_forced
                .difference(&forced_per_rel[&rel_id])
                .copied()
                .collect();
            if others_forced.is_empty() {
                continue;
            }
            let rel = &relationships[rel_id];
            let alive_methods = alive.get_mut(&rel_id).expect("seeded for every active id");
            for (idx, method) in rel.methods.iter().enumerate() {
                if alive_methods[idx]
                    && pure_outputs(method).iter().any(|c| others_forced.contains(c))
                {
                    alive_methods[idx] = false;
                    changed = true;
                }
            }
        }

        if !changed {
            return global_forced;
        }
    }
}
```

- [ ] **Step 9: Run tests to verify they pass**

Run: `cargo test -p property-model`
Expected: PASS for all tests, including `single_method_forced_direction`, `single_method_output_is_forced_and_not_selected_as_source`, `forced_outputs_cascade_through_adjacent_relationship`, and `forced_direction_cascades_through_adjacent_relationship`.

- [ ] **Step 10: Format and lint**

Run: `cargo fmt --all`
Run: `cargo clippy -p property-model -- -D warnings`
Expected: no warnings.

- [ ] **Step 11: Commit**

```bash
git add property-model/src/planner.rs property-model/tests/integration.rs
git commit -m "$(cat <<'EOF'
fix(property-model): forced-output cells can never be planner sources

A relationship's method structure can guarantee a cell is always
produced by a method (e.g. the sole output of a single-method
relationship), independent of cell strength. The planner previously
let strength alone decide source candidacy, so such a cell could be
wrongly promoted to a source, orphaning its real input or reporting a
spurious Conflict. forced_output_cells computes this set as a
fixpoint over active relationships (eliminating a method whose output
collides with another relationship's forced cell can force further
cells) and excludes it from the strength-ordered source loop.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `Sheet::is_forced` / `Sheet::forced_cells` public API

**Files:**
- Modify: `property-model/src/sheet.rs` (struct field, `new()`, `propagate()`, new public methods, `propagate_without_replan()` doc comment)
- Modify: `property-model/tests/integration.rs` (new tests)

**Interfaces:**
- Consumes: `Plan.forced_outputs: HashSet<CellId>` (produced by Task 1).
- Produces: `pub fn Sheet::is_forced(&self, id: CellId) -> bool`, `pub fn Sheet::forced_cells(&self) -> impl Iterator<Item = CellId> + '_`.

- [ ] **Step 1: Write the failing integration.rs tests**

Open `property-model/tests/integration.rs`. Add these tests after `forced_direction_cascades_through_adjacent_relationship` (added in Task 1):

```rust
#[test]
fn is_forced_false_before_propagate() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    assert!(!sheet.is_forced(b));
}

#[test]
fn is_forced_true_for_single_method_output() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 3))])
        .unwrap();

    sheet.propagate().unwrap();

    assert!(sheet.is_forced(b));
    assert!(!sheet.is_forced(a));
}

#[test]
fn is_forced_false_for_multi_method_relationship() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0.0_f64);
    let b = sheet.add_cell(0.0_f64);
    let c = sheet.add_cell(0.0_f64);
    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok((*x) * (*y))),
            Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok((*y) / (*x))),
            Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok((*y) / (*x))),
        ])
        .unwrap();
    sheet.write(a, 2.0_f64).unwrap();
    sheet.write(b, 3.0_f64).unwrap();

    sheet.propagate().unwrap();

    assert!(!sheet.is_forced(a));
    assert!(!sheet.is_forced(b));
    assert!(!sheet.is_forced(c));
}

#[test]
fn forced_cells_iterates_all_forced_cells() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(2_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 10))])
        .unwrap();
    sheet
        .add_relationship(vec![
            Method::from_fn_1_1(b, c, |x: &i32| Ok(*x + 1)),
            Method::from_fn_1_1(c, b, |x: &i32| Ok(*x + 1)),
        ])
        .unwrap();

    sheet.propagate().unwrap();

    let forced: std::collections::HashSet<_> = sheet.forced_cells().collect();
    assert_eq!(forced, std::collections::HashSet::from([b, c]));
}

#[test]
fn is_forced_respects_conditional_branch_activation() {
    let mut sheet = Sheet::new();
    let mode = sheet.add_cell(0_i32);
    let a = sheet.add_cell(3_i32);
    let b = sheet.add_cell(0_i32);

    let rel_on = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
        .unwrap();
    sheet
        .add_conditional(mode, vec![(vec![1_i32], vec![rel_on])], vec![])
        .unwrap();

    // mode=0: rel_on inactive, b is not forced.
    sheet.write(mode, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(!sheet.is_forced(b));

    // mode=1: rel_on active, b is forced.
    sheet.write(mode, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(sheet.is_forced(b));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p property-model`
Expected: compile error — `no method named \`is_forced\` found for struct \`Sheet\`` (and similarly for `forced_cells`).

- [ ] **Step 3: Add the `last_forced` field**

In `property-model/src/sheet.rs`, change the `Sheet` struct's `last_plan` field (line 45) from:

```rust
    last_plan: Option<Vec<(RelationshipId, usize)>>,
```

to:

```rust
    last_plan: Option<Vec<(RelationshipId, usize)>>,
    /// Cells reported forced (see [`Sheet::is_forced`]) by the last full `propagate()`
    /// call. Not recomputed by `propagate_without_replan`.
    last_forced: Option<HashSet<CellId>>,
```

- [ ] **Step 4: Initialize it in `Sheet::new()`**

In `property-model/src/sheet.rs`, change the `Sheet::new()` body (around line 56-64) from:

```rust
        Sheet {
            cells: SlotMap::with_key(),
            relationships: SlotMap::with_key(),
            changed_cells: Vec::new(),
            next_strength: 0,
            last_plan: None,
            conditionals: SlotMap::with_key(),
            conditional_relationships: HashSet::new(),
        }
```

to:

```rust
        Sheet {
            cells: SlotMap::with_key(),
            relationships: SlotMap::with_key(),
            changed_cells: Vec::new(),
            next_strength: 0,
            last_plan: None,
            last_forced: None,
            conditionals: SlotMap::with_key(),
            conditional_relationships: HashSet::new(),
        }
```

- [ ] **Step 5: Cache `forced_outputs` in `propagate()`**

In `property-model/src/sheet.rs`, change the end of `propagate()` (around lines 551-559) from:

```rust
        // Phase 3: general plan on the active set.
        let plan = crate::planner::plan(&self.cells, &self.relationships, &active)?;
        self.execute_plan(&plan.execution_order)?;

        // Phase 4: assign derived-cell strengths in evaluation order.
        self.post_process_strengths(&plan.execution_order);

        self.last_plan = Some(plan.execution_order);
        Ok(())
```

to:

```rust
        // Phase 3: general plan on the active set.
        let plan = crate::planner::plan(&self.cells, &self.relationships, &active)?;
        self.execute_plan(&plan.execution_order)?;

        // Phase 4: assign derived-cell strengths in evaluation order.
        self.post_process_strengths(&plan.execution_order);

        self.last_forced = Some(plan.forced_outputs);
        self.last_plan = Some(plan.execution_order);
        Ok(())
```

- [ ] **Step 6: Add `is_forced` and `forced_cells`**

In `property-model/src/sheet.rs`, insert these two methods immediately after `is_source` (after its closing `}`, around line 667):

```rust
    /// Returns `true` if `id` can never be a source under the currently active
    /// relationships.
    ///
    /// Some active relationship's method structure guarantees the cell is always
    /// produced by a method, regardless of strength — writing to it has no lasting
    /// effect once `propagate()` runs again. Useful for disabling input fields in a UI.
    ///
    /// Returns `false` if no propagation has run yet.
    pub fn is_forced(&self, id: CellId) -> bool {
        self.last_forced
            .as_ref()
            .is_some_and(|forced| forced.contains(&id))
    }

    /// Iterates cells that are forced (see [`Sheet::is_forced`]) as of the last
    /// `propagate()` call.
    ///
    /// - Complexity: O(n) where n is the number of forced cells.
    pub fn forced_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.last_forced.iter().flatten().copied()
    }
```

- [ ] **Step 7: Note staleness in `propagate_without_replan`'s doc comment**

In `property-model/src/sheet.rs`, change the doc comment for `propagate_without_replan` from:

```rust
    /// - Precondition: If the sheet has conditionals, no match-cell value has changed
    ///   since the last `propagate()` call. Violation produces incorrect branch activation.
    ///
    /// # Errors
```

to:

```rust
    /// - Precondition: If the sheet has conditionals, no match-cell value has changed
    ///   since the last `propagate()` call. Violation produces incorrect branch activation.
    ///
    /// `is_forced` and `forced_cells` continue to reflect the last full `propagate()`
    /// call; this method does not recompute them.
    ///
    /// # Errors
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test -p property-model`
Expected: PASS for all tests, including the five new `is_forced`/`forced_cells` tests.

- [ ] **Step 9: Format and lint**

Run: `cargo fmt --all`
Run: `cargo clippy -p property-model -- -D warnings`
Expected: no warnings.

- [ ] **Step 10: Commit**

```bash
git add property-model/src/sheet.rs property-model/tests/integration.rs
git commit -m "$(cat <<'EOF'
feat(property-model): expose forced cells via Sheet::is_forced

UI code binding a form to a Sheet needs to know which fields can never
accept user input (they are always overwritten by an active
relationship, regardless of priority) so it can disable them.
is_forced/forced_cells expose the forced-output set computed by the
planner, cached from the last full propagate() the same way
is_source/last_plan already are.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Full workspace verification

**Files:** none (verification only)

**Interfaces:**
- Consumes: everything from Tasks 1-2.
- Produces: nothing new; confirms the branch is ready to hand off per root `CLAUDE.md`'s "Before creating a PR" checklist.

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Expected: no changes (already formatted per-task), or if it does reformat something, stage and include it in the commit below.

- [ ] **Step 2: Build the whole workspace**

Run: `cargo build --workspace`
Expected: builds cleanly.

- [ ] **Step 3: Run the full test suite**

Run: `cargo test --workspace`
Run: `cargo test --doc --workspace`
Expected: all pass, no regressions in `pm-lang`, `begin`, `cel-runtime`, `cel-parser`, or `cel-rs`.

- [ ] **Step 4: Lint the whole workspace**

Run: `cargo clippy --workspace --exclude begin -- -D warnings`
Run: `cargo clippy -p begin --no-default-features -- -D warnings`
Expected: no warnings from either invocation.

- [ ] **Step 5: Commit any formatting fixes (only if Step 1 produced changes)**

```bash
git add -u
git commit -m "$(cat <<'EOF'
chore: cargo fmt

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

If Step 1 produced no changes, skip this step — there is nothing to commit.
