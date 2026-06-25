# Self-Referencing Relationships Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the property model planner to support methods where a cell appears in both `inputs` and `outputs`, enabling idempotent self-referencing constraints such as `a <== min(a, 0)` and two-way inequality constraints such as `relate { a <== min(a, b); b <== max(a, b); }`.

**Architecture:** Remove the `inputs ∩ outputs = ∅` validation in `add_relationship`, then extend the planner with a `source_cells` set and a revised eligibility rule that classifies each method's cells into three groups (pure inputs, self-referencing, pure outputs) and applies distinct predicates to each group. When multiple methods in one relationship are simultaneously eligible — possible when all participating cells are sources — a preference rule selects the method whose self-referencing output matches the cell currently being processed.

**Tech Stack:** Rust, `slotmap`, `std::collections::HashSet`, `std::collections::VecDeque`.

## Global Constraints

- Run `cargo fmt --all` before every commit; the pre-commit hook enforces it.
- Lint: `cargo clippy --workspace --exclude begin -- -D warnings` must pass clean.
- Every modified public or `pub(crate)` function must keep its `///` doc comment in contract style (see CLAUDE.md).
- No new crate dependencies.
- Branch: `worktree-self-reference` (already active).

---

## File Map

| File | Change |
| --- | --- |
| `property-model/tests/integration.rs` | Add two new tests (written first, to fail) |
| `property-model/src/relationship.rs` | Remove `inputs ∩ outputs = ∅` check; update doc comment |
| `property-model/src/planner.rs` | Add `source_cells`; revise eligibility, feasibility, disambiguation, re-queuing; remove stale `debug_assert`; update doc comment |

---

## Task 1: Write Failing Tests

**Files:**

- Modify: `property-model/tests/integration.rs`

**Interfaces:**

- Consumes: `Sheet`, `Method` (existing public API)
- Produces: two new `#[test]` functions that fail until Tasks 2–3 are complete

- [ ] **Step 1: Add the two failing tests to the bottom of `integration.rs`**

Append both tests. `from_fn_1_1(a, a, …)` and `from_fn_2_1([a, b], a, …)` produce self-referencing methods (same cell in inputs and outputs); both currently hit `Error::InvalidMethod`.

```rust
#[test]
fn self_ref_direct_clamp() {
    // Single self-referencing method: a = min(a, 0).
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, a, |x: &i32| Ok((*x).min(0)))])
        .unwrap();

    // Value above 0: clamped to 0.
    sheet.write(a, 5_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 0);

    // Value at 0: unchanged.
    sheet.write(a, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 0);

    // Value below 0: unchanged (idempotent).
    sheet.write(a, -3_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), -3);
}

#[test]
fn self_ref_le_chain() {
    // a <= b <= c enforced by two self-referencing constraints.
    //
    // R1 — a <= b:
    //   M0: a = min(a, b)  fires when b is the stronger source
    //   M1: b = max(a, b)  fires when a is the stronger source
    //
    // R2 — b <= c:
    //   M2: b = min(b, c)  fires when c is the stronger source
    //   M3: c = max(b, c)  fires when b is the stronger source
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);

    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| Ok((*x).min(*y))),
            Method::from_fn_2_1([a, b], b, |x: &i32, y: &i32| Ok((*x).max(*y))),
        ])
        .unwrap();

    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([b, c], b, |x: &i32, y: &i32| Ok((*x).min(*y))),
            Method::from_fn_2_1([b, c], c, |x: &i32, y: &i32| Ok((*x).max(*y))),
        ])
        .unwrap();

    // Case 1: already satisfied — no adjustment.
    // Write order c, b, a → a is strongest.
    sheet.write(c, 5_i32).unwrap();
    sheet.write(b, 3_i32).unwrap();
    sheet.write(a, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 1);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 3);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 5);

    // Case 2: a > b and a > c, a is strongest — b and c raised to a.
    // Execution order: M1 (b = max(a,b)) then M3 (c = max(b,c)).
    // M3 reads the post-M1 value of b, so c is raised via the updated b.
    sheet.write(c, 1_i32).unwrap();
    sheet.write(b, 3_i32).unwrap();
    sheet.write(a, 5_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 5);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 5);

    // Case 3: b > c, c is strongest — b lowered to c; a already <= b.
    // Execution order: M2 (b = min(b,c)) then M0 (a = min(a,b)).
    // M0 reads the post-M2 value of b.
    sheet.write(a, 1_i32).unwrap();
    sheet.write(b, 5_i32).unwrap();
    sheet.write(c, 3_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 1);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 3);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 3);

    // Case 4: b is strongest, a above and c below — a clamped down, c raised up.
    sheet.write(c, 1_i32).unwrap();
    sheet.write(a, 5_i32).unwrap();
    sheet.write(b, 3_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 3);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 3);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 3);
}
```

- [ ] **Step 2: Run the tests and confirm they fail with `InvalidMethod`**

```bash
cargo test --workspace self_ref_direct_clamp self_ref_le_chain
```

Expected: both tests fail. The `unwrap()` on `add_relationship` panics because the current validation rejects overlapping inputs and outputs with `Error::InvalidMethod`.

- [ ] **Step 3: Commit the failing tests**

```bash
git add property-model/tests/integration.rs
git commit -m "test(property-model): add failing self-referencing constraint tests"
```

---

## Task 2: Allow Self-Referencing Methods in `add_relationship`

**Files:**

- Modify: `property-model/src/relationship.rs` (no line numbers — search for the text shown)
- Modify: `property-model/src/sheet.rs` (doc comment only)

**Interfaces:**

- Consumes: nothing new
- Produces: `add_relationship` now accepts methods where `inputs ∩ outputs ≠ ∅`

- [ ] **Step 1: Remove the disjoint check from `relationship.rs`**

In `property-model/src/relationship.rs` inside `Sheet::add_relationship`, remove this block entirely (it is the only place the word "overlap" appears in the codebase):

```rust
            // inputs ∩ outputs must be empty
            for output in &method.outputs {
                if method.inputs.contains(output) {
                    return Err(Error::InvalidMethod);
                }
            }
```

After removal the surrounding code should flow directly from the `inputs.is_empty() || outputs.is_empty()` check to the type-count length check. Nothing else changes in this file.

- [ ] **Step 2: Update the `add_relationship` doc comment in `sheet.rs`**

Find the `Error::InvalidMethod` error-condition line in the `add_relationship` doc block in `property-model/src/sheet.rs`. It currently reads:

```rust
    /// - `Error::InvalidMethod` — `methods` is empty, a method has no inputs,
    ///   a method has no outputs, or a method's inputs and outputs overlap.
```

Replace with:

```rust
    /// - `Error::InvalidMethod` — `methods` is empty, a method has no inputs,
    ///   or a method has no outputs. A cell that appears in both a method's inputs
    ///   and its outputs is a self-referencing cell and is explicitly allowed.
```

- [ ] **Step 3: Run the tests and confirm the failure mode changes**

```bash
cargo test --workspace self_ref_direct_clamp self_ref_le_chain
```

Expected: both tests now panic at `sheet.propagate().unwrap()` (not at `add_relationship`). The error is `Error::Conflict` because the planner cannot yet handle self-referencing methods.

- [ ] **Step 4: Run the full test suite to confirm no regressions**

```bash
cargo test --workspace
```

Expected: all previously passing tests still pass; the two new tests still fail.

- [ ] **Step 5: Commit**

```bash
git add property-model/src/relationship.rs property-model/src/sheet.rs
git commit -m "feat(property-model): allow self-referencing methods in add_relationship"
```

---

## Task 3: Implement Self-Referencing Support in the Planner

**Files:**

- Modify: `property-model/src/planner.rs`

**Interfaces:**

- Consumes: `source_cells` distinction (internal to this task)
- Produces: `plan()` correctly selects and orders self-referencing methods; both new tests pass

The changes touch only the module-level doc comment, one import line, and the `plan` function. The function signature (`pub(crate) fn plan(…) -> Result<Plan, Error>`) does not change.

- [ ] **Step 1: Add `Method` to the import and update the module-level doc comment in `planner.rs`**

The `plan` function closure parameters use `&Method` explicitly; the type must be in scope.

Replace the `use crate` block (lines 30–34):

```rust
use crate::{
    cell::{CellData, CellId},
    error::Error,
    relationship::{RelationshipData, RelationshipId},
};
```

with:

```rust
use crate::{
    cell::{CellData, CellId},
    error::Error,
    relationship::{Method, RelationshipData, RelationshipId},
};
```

Also update the module-level doc comment. The sentence that currently reads:

```text
//! In a properly formed multi-way constraint,
//! at most one method per relationship can be eligible at any given point (the inputs of
//! each method are the outputs of the other methods), so selection is deterministic.
```

Replace it with:

```text
//! In a standard (non-self-referencing) multi-way constraint, at most one method per
//! relationship can be eligible at any given point (the inputs of each method are the
//! outputs of the other methods). Self-referencing methods — where a cell appears in both
//! inputs and outputs — can have multiple eligible methods simultaneously when all
//! participating cells are sources; disambiguation is described in [`plan`].
```

- [ ] **Step 2: Replace the `plan` function in `planner.rs`**

Replace the full `plan` function (from `pub(crate) fn plan` through its closing `}`) with:

```rust
/// Assigns one method per relationship and returns them in dependency order.
///
/// A method may have cells in both `inputs` and `outputs` (self-referencing). Such a cell is
/// read at its pre-execution value and overwritten with the result. A self-referencing method
/// is only selected when every self-referencing cell is a *source* — determined by the outer
/// flood-fill pass, not as the output of another method.
///
/// When two methods in one relationship are simultaneously eligible (possible when all
/// participating cells are sources), the method whose self-referencing output matches the cell
/// currently being processed from the queue is preferred.
///
/// # Errors
///
/// - `Error::Conflict` — not every relationship could be assigned a method.
///
/// - Complexity: O(C log C + R·M·K²) where C = cells, R = relationships,
///   M = methods per relationship, K = cells per method.
pub(crate) fn plan(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
) -> Result<Plan, Error> {
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
        determined.insert(source);
        source_cells.insert(source);

        let mut queue: VecDeque<CellId> = VecDeque::new();
        queue.push_back(source);

        while let Some(cell) = queue.pop_front() {
            for &rel_id in &cells[cell].adj {
                if selected_set.contains(&rel_id) {
                    continue;
                }
                let rel = &relationships[rel_id];

                // A method is eligible when:
                //   pure inputs  (inputs ∖ outputs): all in `determined`
                //   self-ref     (inputs ∩ outputs): all in `source_cells` or `pre_claimed`
                //   pure outputs (outputs ∖ inputs): none in `determined`
                let is_eligible = |m: &Method| {
                    m.inputs
                        .iter()
                        .filter(|i| !m.outputs.contains(i))
                        .all(|i| determined.contains(i))
                        && m.inputs
                            .iter()
                            .filter(|i| m.outputs.contains(i))
                            .all(|i| source_cells.contains(i) || pre_claimed.contains(i))
                        && m.outputs
                            .iter()
                            .filter(|o| !m.inputs.contains(o))
                            .all(|o| !determined.contains(o))
                };

                // Prefer the eligible method whose self-referencing output is `cell`
                // (the cell currently being processed). This resolves ties when two
                // methods are simultaneously eligible: `cell` is the weakest source
                // processed so far, so the method that adjusts it should be chosen.
                // Fall back to the first eligible method for non-self-referencing cases.
                let chosen = rel
                    .methods
                    .iter()
                    .enumerate()
                    .find(|(_, m)| {
                        is_eligible(m) && m.outputs.contains(&cell) && m.inputs.contains(&cell)
                    })
                    .or_else(|| {
                        rel.methods
                            .iter()
                            .enumerate()
                            .find(|(_, m)| is_eligible(m))
                    });

                if let Some((method_idx, method)) = chosen {
                    for &output in &method.outputs {
                        // Guard: self-referencing outputs are already in `determined`
                        // (as sources); only re-queue cells that are newly determined.
                        let newly_determined = determined.insert(output);
                        pre_claimed.remove(&output);
                        if newly_determined {
                            queue.push_back(output);
                        }
                    }
                    selected_set.insert(rel_id);
                    selected.push((rel_id, method_idx));
                } else {
                    // No method is immediately selectable. If exactly one method remains
                    // feasible — meaning no other method can run without overwriting a cell
                    // that is already determined or pre-claimed — and the current cell is
                    // one of its inputs, pre-claim its pure outputs so the flood-fill
                    // propagates further.
                    //
                    // A self-referencing output that is a source is feasible (the method
                    // may overwrite its own source cell). A self-referencing output that
                    // was derived by another method (in `determined` but not `source_cells`)
                    // is infeasible. Pure outputs must not be determined or pre-claimed.
                    //
                    // Only pure outputs are pre-claimed; self-referencing outputs are
                    // already committed as sources and do not need pre-claiming.
                    let is_feasible = |m: &Method| {
                        m.outputs.iter().all(|o| {
                            if m.inputs.contains(o) {
                                !pre_claimed.contains(o)
                                    && (!determined.contains(o) || source_cells.contains(o))
                            } else {
                                !determined.contains(o) && !pre_claimed.contains(o)
                            }
                        })
                    };

                    let mut feasible = rel.methods.iter().filter(|m| is_feasible(m));
                    let first = feasible.next();
                    let second = feasible.next();
                    if let (Some(sole), None) = (first, second)
                        && sole.inputs.contains(&cell)
                    {
                        for &output in &sole.outputs {
                            if !sole.inputs.contains(&output) && pre_claimed.insert(output) {
                                queue.push_back(output);
                            }
                        }
                    }
                }
            }
        }
    }

    if selected.len() != relationships.len() {
        return Err(Error::Conflict);
    }

    Ok(Plan {
        execution_order: selected,
    })
}
```

- [ ] **Step 3: Run the two new tests**

```bash
cargo test --workspace self_ref_direct_clamp self_ref_le_chain
```

Expected: both tests pass.

- [ ] **Step 4: Run the full test suite**

```bash
cargo test --workspace
cargo test --doc --workspace
```

Expected: all tests pass with no regressions.

- [ ] **Step 5: Run clippy**

```bash
cargo clippy --workspace --exclude begin -- -D warnings
cargo clippy -p begin --no-default-features -- -D warnings
```

Expected: no warnings.

- [ ] **Step 6: Format and commit**

```bash
cargo fmt --all
git add property-model/src/planner.rs
git commit -m "feat(property-model): support self-referencing methods in planner"
```
