# Conditionals for property-model — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add conditional match-cell branching to the `property-model` crate, with strength-partitioned stability guarantees, so relationships can be selectively activated based on runtime cell values.

**Architecture:** Five sequential tasks build the feature bottom-up. Each compiles cleanly and passes all tests before the next begins. Task 1 adds equality support and strength partitioning (foundational, touching existing APIs). Tasks 2–4 add new types and `add_conditional`. Task 5 rewrites `propagate()` as a four-phase operation.

**Tech Stack:** Rust stable (edition 2024), `slotmap 1.0`, `anyhow 1.0`

## Global Constraints

- All source under `property-model/src/`; integration tests under `property-model/tests/`
- Format before every commit: `cargo fmt --all`
- Lint (warnings are errors): `cargo clippy -p property-model -- -D warnings`
- Run all tests: `cargo test -p property-model`
- Run single test: `cargo test -p property-model <test_name>`
- `strength`, `eq_fn`, `conditionals`, `conditional_relationships`, `cells`, `relationships` fields are `pub(crate)`; unit tests inside module blocks (`#[cfg(test)] mod tests { ... }` in the same file) can access them
- Every public function must have a `///` doc comment in contract style per CLAUDE.md
- Checked arithmetic: signed integer ops use `checked_*`; unsigned counters use `saturating_sub` for the decrement

---

## File Map

| File | Status | Responsibility |
| --- | --- | --- |
| `src/cell.rs` | Modify | Add `eq_fn` field; add `PartialEq` bound to `add_cell` |
| `src/sheet.rs` | Modify | High-bit strength; `post_process_strengths`; new slotmap fields; `add_conditional`; rewrite `propagate` |
| `src/planner.rs` | Modify | Accept `active: &HashSet<RelationshipId>` filter; plan only active relationships |
| `src/conditional.rs` | Create | `ConditionalId`, `Branch`, `ConditionalData` |
| `src/error.rs` | Modify | Add `InvalidConditional` variant |
| `src/lib.rs` | Modify | `pub mod conditional`; re-export `ConditionalId` |
| `tests/integration.rs` | Modify | New conditional tests |

---

## Task 1: Equality Support and Strength Partitioning

**Files:**
- Modify: `property-model/src/cell.rs`
- Modify: `property-model/src/sheet.rs`

**Interfaces:**
- Produces: `CellData.eq_fn: fn(&dyn Any, &dyn Any) -> bool`
- Produces: `Sheet::add_cell<T: Any + PartialEq + 'static>` (tightened bound)
- Produces: `Sheet::post_process_strengths(&mut self, execution_order: &[(RelationshipId, usize)])` (private)
- Produces: strength invariant: `(written/added strength) & (1 << 63) != 0` always; `(derived strength) & (1 << 63) == 0` always

- [ ] **Step 1: Write failing strength tests in `src/sheet.rs`**

Add these two tests to the `mod tests` block in `sheet.rs`. They compile against the current code (the `strength` field already exists) but fail because the high-order bit is not yet set and no post-processing runs.

```rust
#[test]
fn add_cell_and_write_set_high_order_bit_on_strength() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    assert!(
        sheet.cells[a].strength & (1u64 << 63) != 0,
        "add_cell must set high-order bit"
    );
    sheet.write(a, 1_i32).unwrap();
    assert!(
        sheet.cells[a].strength & (1u64 << 63) != 0,
        "write must set high-order bit"
    );
}

#[test]
fn propagate_assigns_low_order_strength_to_derived_cells() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    sheet.write(a, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(
        sheet.cells[a].strength & (1u64 << 63) != 0,
        "source cell must keep high-order strength"
    );
    assert!(
        sheet.cells[b].strength & (1u64 << 63) == 0,
        "derived cell must have low-order strength"
    );
    assert!(sheet.cells[a].strength > sheet.cells[b].strength);
}
```

- [ ] **Step 2: Verify the tests fail**

```
cargo test -p property-model add_cell_and_write_set_high_order_bit_on_strength
cargo test -p property-model propagate_assigns_low_order_strength_to_derived_cells
```

Expected: both FAIL (assertion panics about high-order bit).

- [ ] **Step 3: Update `src/cell.rs` — add `eq_fn` to `CellData`**

Replace the entire `CellData` struct and its test:

```rust
use std::any::{Any, TypeId};

use slotmap::new_key_type;

use crate::relationship::RelationshipId;

new_key_type! {
    /// A stable handle to a cell in a [`crate::sheet::Sheet`].
    pub struct CellId;
}

/// Internal storage for a single value cell.
pub(crate) struct CellData {
    /// The type-erased current value.
    pub(crate) value: Box<dyn Any>,
    /// The `TypeId` of the value, fixed at cell creation.
    pub(crate) type_id: TypeId,
    /// Write-recency strength. High-order bit (bit 63) is set for cells that have been
    /// written or created via `add_cell`. Derived cells (outputs of selected methods)
    /// receive strengths with bit 63 clear, assigned during the post-processing pass.
    pub(crate) strength: u64,
    /// Set during `Sheet::propagate`; cleared by `Sheet::clear_changed`.
    pub(crate) changed: bool,
    /// Relationships that include this cell.
    pub(crate) adj: Vec<RelationshipId>,
    /// Type-erased equality: returns `true` iff both arguments hold equal values of the
    /// cell's registered type. Captured at `add_cell` time from the concrete `T: PartialEq`.
    pub(crate) eq_fn: fn(&dyn Any, &dyn Any) -> bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_data_initial_state() {
        let data = CellData {
            value: Box::new(42_i32),
            type_id: TypeId::of::<i32>(),
            strength: 0,
            changed: false,
            adj: vec![],
            eq_fn: |a, b| a.downcast_ref::<i32>() == b.downcast_ref::<i32>(),
        };
        assert_eq!(data.type_id, TypeId::of::<i32>());
        assert_eq!(data.strength, 0);
        assert!(!data.changed);
        assert!(data.adj.is_empty());
        assert_eq!(*data.value.downcast_ref::<i32>().unwrap(), 42);
        let x: i32 = 42;
        let y: i32 = 99;
        assert!((data.eq_fn)(&x, &x));
        assert!(!(data.eq_fn)(&x, &y));
    }

    #[test]
    fn cell_id_is_copy() {
        fn takes_copy<T: Copy>(_: T) {}
        takes_copy(CellId::default());
    }
}
```

- [ ] **Step 4: Update `src/sheet.rs` — high-bit strength, `add_cell` bound, `post_process_strengths`**

**4a.** Change `add_cell` signature and body. Replace the existing `add_cell` method:

```rust
/// Registers a cell with an initial value and returns a stable handle.
///
/// The cell's `TypeId` is fixed at creation time; subsequent `write` and
/// `read` calls that use a different type will return `Error::TypeMismatch`.
///
/// Each call increments the sheet's internal strength counter and sets bit 63
/// of the result. This partitions the strength space: written/added cells always
/// have higher strength than derived cells, ensuring stability across conditional
/// branch switches.
pub fn add_cell<T: Any + PartialEq + 'static>(&mut self, value: T) -> CellId {
    self.next_strength += 1;
    let strength = self.next_strength | (1u64 << 63);
    self.cells.insert(CellData {
        value: Box::new(value),
        type_id: TypeId::of::<T>(),
        strength,
        changed: false,
        adj: Vec::new(),
        eq_fn: |a, b| a.downcast_ref::<T>() == b.downcast_ref::<T>(),
    })
}
```

**4b.** Change `write` to use the high-order bit. Replace the body (signature unchanged):

```rust
pub fn write<T: Any + 'static>(&mut self, id: CellId, value: T) -> Result<(), Error> {
    let cell = self.cells.get_mut(id).ok_or(Error::InvalidId)?;
    if cell.type_id != TypeId::of::<T>() {
        return Err(Error::TypeMismatch {
            expected: cell.type_id,
            found: TypeId::of::<T>(),
        });
    }
    self.next_strength += 1;
    cell.strength = self.next_strength | (1u64 << 63);
    cell.value = Box::new(value);
    Ok(())
}
```

**4c.** Add `post_process_strengths` as a private method on `Sheet`, before `propagate`:

```rust
/// Assigns derived-cell strengths after a planning pass.
///
/// Walks `execution_order` and assigns a decrementing counter (starting at
/// `0x7FFF_FFFF_FFFF_FFFF`) to each output cell of each selected method, in
/// execution order. Cells evaluated first receive the highest derived strength.
/// Source cells (not the output of any selected method) are not modified.
///
/// - Complexity: O(R·K) where R is the number of entries and K is the maximum
///   outputs per method.
fn post_process_strengths(&mut self, execution_order: &[(RelationshipId, usize)]) {
    let mut derived_strength = u64::MAX >> 1; // 0x7FFF_FFFF_FFFF_FFFF
    let mut seen: std::collections::HashSet<CellId> = std::collections::HashSet::new();
    for &(rel_id, method_idx) in execution_order {
        if let Some(rel) = self.relationships.get(rel_id) {
            if let Some(method) = rel.methods.get(method_idx) {
                for &output in &method.outputs {
                    if seen.insert(output) {
                        if let Some(cell) = self.cells.get_mut(output) {
                            cell.strength = derived_strength;
                            derived_strength = derived_strength.saturating_sub(1);
                        }
                    }
                }
            }
        }
    }
}
```

**4d.** Update `propagate` to call `post_process_strengths`. Replace the existing `propagate` body:

```rust
pub fn propagate(&mut self) -> Result<(), Error> {
    self.clear_changed();
    let plan = crate::planner::plan(&self.cells, &self.relationships)?;
    self.execute_plan(&plan.execution_order)?;
    self.post_process_strengths(&plan.execution_order);
    self.last_plan = Some(plan.execution_order);
    Ok(())
}
```

**4e.** Update `propagate_without_replan` to call `post_process_strengths`:

```rust
/// Re-executes the cached plan without invoking the planner.
///
/// - Precondition: Every cell written since the last successful `propagate()` or
///   `propagate_without_replan()` call satisfies `is_source(id)`. Violation produces
///   incorrect output values but no panic.
/// - Precondition: If the sheet has conditionals, no match-cell value has changed
///   since the last `propagate()`. Violation produces incorrect branch activation.
///
/// # Errors
///
/// - `Error::Conflict` — `propagate()` has not yet been called; no plan is cached.
/// - `Error::MethodFailed` — a method's function returned an error.
/// - `Error::TypeMismatch` — a method output's runtime type does not match the cell's
///   registered type.
///
/// - Complexity: O(R·K) where R is the number of relationships in the cached plan and K is the maximum cells per method, plus per-method execution cost.
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

- [ ] **Step 5: Run all tests and verify they pass**

```
cargo test -p property-model
```

Expected: all tests pass including the two new ones. Note: `strength_drives_method_selection` in `tests/integration.rs` has comments like "strength=1, strength=2" — these comments are now stale (actual values have the high-order bit set) but the test behavior is preserved because relative ordering is unchanged.

- [ ] **Step 6: Lint and commit**

```
cargo clippy -p property-model -- -D warnings
cargo fmt --all
git add property-model/src/cell.rs property-model/src/sheet.rs
git commit -m "feat(property-model): add equality support and strength partitioning"
```

---

## Task 2: Conditional Data Structures and Error Variant

**Files:**
- Create: `property-model/src/conditional.rs`
- Modify: `property-model/src/error.rs`
- Modify: `property-model/src/lib.rs`

**Interfaces:**
- Consumes: `CellId` (cell.rs), `RelationshipId` (relationship.rs)
- Produces: `pub struct ConditionalId` (slotmap key, Copy + Clone)
- Produces: `pub(crate) struct Branch { keys: Vec<Box<dyn Any>>, relationships: Vec<RelationshipId> }`
- Produces: `pub(crate) struct ConditionalData { cell: CellId, branches: Vec<Branch>, default: Vec<RelationshipId> }`
- Produces: `Error::InvalidConditional` variant

- [ ] **Step 1: Write failing test for `ConditionalId`**

Add a test to the bottom of `src/conditional.rs` (file doesn't exist yet, so also write the whole file). This compiles only after the file exists — write it now as the first content:

```rust
// property-model/src/conditional.rs
//! Conditionals in the property model: match-cell branching.
//!
//! Each conditional binds to one cell (the *match cell*) and holds a list of
//! branches. During propagation the branch whose keys contain the current match
//! cell value is activated; its relationships participate in the general planning
//! pass.

use std::any::Any;

use slotmap::new_key_type;

use crate::{cell::CellId, relationship::RelationshipId};

new_key_type! {
    /// A stable handle to a conditional in a [`crate::sheet::Sheet`].
    pub struct ConditionalId;
}

/// One arm of a [`ConditionalData`]: a set of key values and the relationships
/// to activate when the match cell equals any key.
pub(crate) struct Branch {
    /// Type-erased key values; each `TypeId` matches the match cell's registered type.
    pub(crate) keys: Vec<Box<dyn Any>>,
    /// Relationships activated when any key matches.
    pub(crate) relationships: Vec<RelationshipId>,
}

/// Internal storage for a conditional.
pub(crate) struct ConditionalData {
    /// The cell whose value is tested.
    pub(crate) cell: CellId,
    /// Branches evaluated in definition order; first match wins.
    pub(crate) branches: Vec<Branch>,
    /// Relationships activated when no branch matches. Empty means no default.
    pub(crate) default: Vec<RelationshipId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditional_id_is_copy() {
        fn takes_copy<T: Copy>(_: T) {}
        takes_copy(ConditionalId::default());
    }
}
```

- [ ] **Step 2: Add `InvalidConditional` to `src/error.rs`**

Add the variant after `InvalidMethod`:

```rust
/// A conditional is structurally invalid: the cell was not found, a referenced
/// relationship was not found or has more than one method, a relationship appears
/// in more than one conditional branch, a branch key's type does not match the
/// cell's registered type, or a branch has no keys.
InvalidConditional,
```

Also add its display arm in the `Display` impl:

```rust
Error::InvalidConditional => write!(f, "conditional is structurally invalid"),
```

And add a `source()` arm (already covered by the catch-all `None` for non-`MethodFailed` variants — no change needed to `source()`).

Also add a test to the `mod tests` block in `error.rs`:

```rust
#[test]
fn invalid_conditional_display_contains_conditional() {
    assert!(Error::InvalidConditional.to_string().contains("conditional"));
}

#[test]
fn invalid_conditional_has_no_source() {
    assert!(std::error::Error::source(&Error::InvalidConditional).is_none());
}
```

- [ ] **Step 3: Wire into `src/lib.rs`**

Add the new module and re-export `ConditionalId`:

```rust
pub mod conditional;

pub use conditional::ConditionalId;
```

The `pub mod conditional` line goes after `pub mod relationship;`. The re-export goes with the other `pub use` lines.

- [ ] **Step 4: Run all tests**

```
cargo test -p property-model
```

Expected: all existing tests pass; new tests (`conditional_id_is_copy`, `invalid_conditional_display_contains_conditional`, `invalid_conditional_has_no_source`) pass.

- [ ] **Step 5: Lint and commit**

```
cargo clippy -p property-model -- -D warnings
cargo fmt --all
git add property-model/src/conditional.rs property-model/src/error.rs property-model/src/lib.rs
git commit -m "feat(property-model): add ConditionalId, Branch, ConditionalData, Error::InvalidConditional"
```

---

## Task 3: Planner Active-Set Filter

**Files:**
- Modify: `property-model/src/planner.rs`
- Modify: `property-model/src/sheet.rs` (update call site)

**Interfaces:**
- Consumes (from Task 1): `CellData`, `RelationshipData`, `RelationshipId`
- Produces: `plan(cells, relationships, active: &HashSet<RelationshipId>) -> Result<Plan, Error>` (changed signature)
- The `active` set replaces the implicit "all relationships" assumption. Only relationships in `active` are planned; the conflict check compares against `active.len()`.

- [ ] **Step 1: Write a failing test for subset planning in `src/planner.rs`**

Add to the `mod tests` block in `planner.rs`:

```rust
#[test]
fn plan_with_active_subset_ignores_inactive_relationship() {
    use std::collections::HashSet;
    // Two independent relationships: R1 (a→b) and R2 (c→d).
    // Plan with only R1 active; R2 must be ignored (not required in output).
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);
    let d = sheet.add_cell(0_i32);

    let r1 = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    let _r2 = sheet
        .add_relationship(vec![Method::from_fn_1_1(c, d, |x: &i32| Ok(*x))])
        .unwrap();

    let mut active = HashSet::new();
    active.insert(r1);

    let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();
    assert_eq!(plan.execution_order.len(), 1);
    assert_eq!(plan.execution_order[0].0, r1);
}
```

This test fails to compile until the signature is updated — note that to the implementer. Run `cargo build -p property-model` to observe the compile error.

- [ ] **Step 2: Update `plan()` signature and internals in `src/planner.rs`**

Replace the function signature and add the active filter. The full updated function:

```rust
/// Assigns one method per active relationship and returns them in dependency order.
///
/// Only relationships in `active` are planned; relationships outside `active` are
/// invisible to the flood-fill. The conflict check counts against `active.len()`.
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
/// - `Error::Conflict` — not every active relationship could be assigned a method.
///
/// - Complexity: O(C log C + R·M·K²) where C = cells, R = active relationships,
///   M = methods per relationship, K = cells per method.
pub(crate) fn plan(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> Result<Plan, Error> {
    let mut determined: HashSet<CellId> = HashSet::new();
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
                if !active.contains(&rel_id) {
                    continue;
                }
                if selected_set.contains(&rel_id) {
                    continue;
                }
                let rel = &relationships[rel_id];

                let is_eligible = |m: &Method| {
                    m.inputs
                        .iter()
                        .filter(|i| !m.outputs.contains(i))
                        .all(|i| determined.contains(i))
                        && m.inputs
                            .iter()
                            .filter(|i| m.outputs.contains(i))
                            .all(|i| source_cells.contains(i))
                        && m.outputs
                            .iter()
                            .filter(|o| !m.inputs.contains(o))
                            .all(|o| !determined.contains(o))
                };

                let chosen = rel
                    .methods
                    .iter()
                    .enumerate()
                    .find(|(_, m)| {
                        is_eligible(m) && m.outputs.contains(&cell) && m.inputs.contains(&cell)
                    })
                    .or_else(|| rel.methods.iter().enumerate().find(|(_, m)| is_eligible(m)));

                if let Some((method_idx, method)) = chosen {
                    for &output in &method.outputs {
                        let newly_determined = determined.insert(output);
                        pre_claimed.remove(&output);
                        source_cells.remove(&output);
                        if newly_determined {
                            queue.push_back(output);
                        }
                    }
                    selected_set.insert(rel_id);
                    selected.push((rel_id, method_idx));
                } else {
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

    if selected.len() != active.len() {
        return Err(Error::Conflict);
    }

    Ok(Plan {
        execution_order: selected,
    })
}
```

Also add `HashSet` to the imports at the top of `planner.rs`:

```rust
use std::collections::{HashSet, VecDeque};
```

(Replace the existing `use std::collections::VecDeque;` line, or add `HashSet` to it.)

- [ ] **Step 3: Update the call site in `src/sheet.rs`**

In `Sheet::propagate`, replace the `plan(...)` call:

```rust
pub fn propagate(&mut self) -> Result<(), Error> {
    self.clear_changed();
    let active: std::collections::HashSet<RelationshipId> =
        self.relationships.keys().collect();
    let plan = crate::planner::plan(&self.cells, &self.relationships, &active)?;
    self.execute_plan(&plan.execution_order)?;
    self.post_process_strengths(&plan.execution_order);
    self.last_plan = Some(plan.execution_order);
    Ok(())
}
```

- [ ] **Step 4: Run all tests**

```
cargo test -p property-model
```

Expected: all existing tests pass; `plan_with_active_subset_ignores_inactive_relationship` passes.

- [ ] **Step 5: Lint and commit**

```
cargo clippy -p property-model -- -D warnings
cargo fmt --all
git add property-model/src/planner.rs property-model/src/sheet.rs
git commit -m "feat(property-model): add active-set filter to planner"
```

---

## Task 4: `add_conditional` on Sheet

**Files:**
- Modify: `property-model/src/sheet.rs`

**Interfaces:**
- Consumes (Task 2): `ConditionalId`, `ConditionalData`, `Branch`, `Error::InvalidConditional`
- Produces: `Sheet::add_conditional<T: Any + PartialEq + 'static>(cell, branches, default) -> Result<ConditionalId, Error>`
- Produces: `Sheet.conditionals: SlotMap<ConditionalId, ConditionalData>`
- Produces: `Sheet.conditional_relationships: HashSet<RelationshipId>`

- [ ] **Step 1: Write failing tests in `src/sheet.rs`**

Add to the `mod tests` block. These fail because `add_conditional` doesn't exist yet.

```rust
#[test]
fn add_conditional_returns_error_for_invalid_cell() {
    let mut sheet = Sheet::new();
    let result = sheet.add_conditional(
        CellId::default(),
        vec![(vec![0_i32], vec![])],
        vec![],
    );
    assert!(matches!(result, Err(Error::InvalidId)));
}

#[test]
fn add_conditional_returns_invalid_conditional_for_type_mismatch() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    // Branch keys are f64 but cell holds i32.
    let result = sheet.add_conditional(
        a,
        vec![(vec![0.0_f64], vec![])],
        vec![],
    );
    assert!(matches!(result, Err(Error::InvalidConditional)));
}

#[test]
fn add_conditional_returns_invalid_conditional_for_missing_relationship() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let result = sheet.add_conditional(
        a,
        vec![(vec![0_i32], vec![RelationshipId::default()])],
        vec![],
    );
    assert!(matches!(result, Err(Error::InvalidConditional)));
}

#[test]
fn add_conditional_returns_invalid_conditional_for_multi_method_relationship() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    // Relationship has two methods — not allowed in a conditional branch.
    let rel = sheet
        .add_relationship(vec![
            Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
            Method::from_fn_1_1(b, a, |x: &i32| Ok(*x)),
        ])
        .unwrap();
    let result = sheet.add_conditional(
        a,
        vec![(vec![0_i32], vec![rel])],
        vec![],
    );
    assert!(matches!(result, Err(Error::InvalidConditional)));
}

#[test]
fn add_conditional_returns_invalid_conditional_for_empty_branch_keys() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    // Empty key list is invalid.
    let result = sheet.add_conditional(
        a,
        vec![(vec![], vec![rel])],
        vec![],
    );
    assert!(matches!(result, Err(Error::InvalidConditional)));
}

#[test]
fn add_conditional_returns_invalid_conditional_for_duplicate_relationship_across_branches() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    // Add rel to the first conditional.
    sheet
        .add_conditional(a, vec![(vec![0_i32], vec![rel])], vec![])
        .unwrap();
    // Try to add the same rel to a second conditional.
    let result = sheet.add_conditional(
        a,
        vec![(vec![1_i32], vec![rel])],
        vec![],
    );
    assert!(matches!(result, Err(Error::InvalidConditional)));
}

#[test]
fn add_conditional_returns_id_for_valid_input() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    let cid = sheet
        .add_conditional(a, vec![(vec![0_i32], vec![rel])], vec![])
        .unwrap();
    // ConditionalId must be a live key.
    let _ = cid; // just check it compiles and succeeds
}
```

- [ ] **Step 2: Verify the tests fail to compile**

```
cargo build -p property-model
```

Expected: compile errors — `add_conditional` does not exist, `conditional_relationships` does not exist.

- [ ] **Step 3: Add new fields and imports to `Sheet`**

In `src/sheet.rs`, add to the imports at the top:

```rust
use std::any::TypeId;
use std::collections::HashSet;

use crate::conditional::{Branch, ConditionalData, ConditionalId};
```

(The `use std::any::{Any, TypeId}` line may already import `TypeId`; adjust as needed.)

Add two fields to `Sheet`:

```rust
pub struct Sheet {
    pub(crate) cells: SlotMap<CellId, CellData>,
    pub(crate) relationships: SlotMap<RelationshipId, RelationshipData>,
    pub(crate) changed_cells: Vec<CellId>,
    next_strength: u64,
    last_plan: Option<Vec<(RelationshipId, usize)>>,
    /// All conditionals registered on this sheet.
    pub(crate) conditionals: SlotMap<ConditionalId, ConditionalData>,
    /// Union of all RelationshipIds assigned to any conditional branch or default.
    /// Used to exclude them from the unconditional active set.
    pub(crate) conditional_relationships: HashSet<RelationshipId>,
}
```

Update `Sheet::new()` to initialise the new fields:

```rust
pub fn new() -> Self {
    Sheet {
        cells: SlotMap::with_key(),
        relationships: SlotMap::with_key(),
        changed_cells: Vec::new(),
        next_strength: 0,
        last_plan: None,
        conditionals: SlotMap::with_key(),
        conditional_relationships: HashSet::new(),
    }
}
```

- [ ] **Step 4: Implement `add_conditional`**

Add this method to the `Sheet` impl block, after `add_relationship`:

```rust
/// Registers a conditional that activates relationships based on the value of `cell`.
///
/// Each element of `branches` is `(keys, relationships)`: when the match cell's value
/// equals any key in `keys`, the branch's `relationships` are added to the active set
/// for `propagate`. Branches are evaluated in definition order; first match wins.
/// `default` holds relationships activated when no branch matches; pass an empty `Vec`
/// for no default.
///
/// The match cell's value is compared using the equality function captured at
/// `add_cell` time.
///
/// # Errors
///
/// - `Error::InvalidId` — `cell` is not in this sheet.
/// - `Error::InvalidConditional` — a branch key's `TypeId` does not match `cell`'s;
///   a referenced relationship does not exist; a relationship has more than one method;
///   a relationship already appears in another conditional branch; or a branch has no keys.
///
/// - Complexity: O(B·(K + R)) where B = branches, K = keys per branch, R = relationships per branch.
pub fn add_conditional<T: Any + PartialEq + 'static>(
    &mut self,
    cell: CellId,
    branches: Vec<(Vec<T>, Vec<RelationshipId>)>,
    default: Vec<RelationshipId>,
) -> Result<ConditionalId, Error> {
    let cell_data = self.cells.get(cell).ok_or(Error::InvalidId)?;
    if cell_data.type_id != TypeId::of::<T>() {
        return Err(Error::InvalidConditional);
    }

    // Collect and validate all relationship IDs (branches + default).
    let all_rels: Vec<RelationshipId> = branches
        .iter()
        .flat_map(|(_, rels)| rels.iter().copied())
        .chain(default.iter().copied())
        .collect();

    for &rel_id in &all_rels {
        let rel = self
            .relationships
            .get(rel_id)
            .ok_or(Error::InvalidConditional)?;
        if rel.methods.len() != 1 {
            return Err(Error::InvalidConditional);
        }
        if self.conditional_relationships.contains(&rel_id) {
            return Err(Error::InvalidConditional);
        }
    }

    // Validate branch keys are non-empty.
    for (keys, _) in &branches {
        if keys.is_empty() {
            return Err(Error::InvalidConditional);
        }
    }

    // Check for duplicate relationship IDs within this call.
    let mut seen: HashSet<RelationshipId> = HashSet::new();
    for &rel_id in &all_rels {
        if !seen.insert(rel_id) {
            return Err(Error::InvalidConditional);
        }
    }

    // Type-erase branch keys.
    let typed_branches: Vec<Branch> = branches
        .into_iter()
        .map(|(keys, relationships)| Branch {
            keys: keys.into_iter().map(|k| Box::new(k) as Box<dyn Any>).collect(),
            relationships,
        })
        .collect();

    // Record all relationships as conditional so they are excluded from the
    // unconditional active set in propagate().
    for &rel_id in &all_rels {
        self.conditional_relationships.insert(rel_id);
    }

    Ok(self.conditionals.insert(ConditionalData {
        cell,
        branches: typed_branches,
        default,
    }))
}
```

- [ ] **Step 5: Run all tests**

```
cargo test -p property-model
```

Expected: all tests pass including the seven new `add_conditional_*` tests.

- [ ] **Step 6: Lint and commit**

```
cargo clippy -p property-model -- -D warnings
cargo fmt --all
git add property-model/src/sheet.rs
git commit -m "feat(property-model): add Sheet::add_conditional with full validation"
```

---

## Task 5: Four-Phase Propagation

**Files:**
- Modify: `property-model/src/sheet.rs`
- Modify: `property-model/tests/integration.rs`

**Interfaces:**
- Consumes (Task 3): `plan(cells, relationships, active)`
- Consumes (Task 4): `Sheet.conditionals`, `Sheet.conditional_relationships`, `CellData.eq_fn`
- Produces: `Sheet::propagate` — four-phase: pre-plan, conditional eval, general plan, strength post-processing
- Produces: `Sheet::build_active_set` (private helper)
- Produces: `Sheet::match_cell_subgraph` (private helper)

- [ ] **Step 1: Write failing integration tests in `tests/integration.rs`**

Add all of these to `tests/integration.rs`:

```rust
#[test]
fn conditional_activates_matching_branch() {
    // mode=1 activates rel_on which doubles `a` into `b`.
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

    sheet.write(mode, 1_i32).unwrap();
    sheet.write(a, 3_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 6);
}

#[test]
fn conditional_no_match_and_no_default_succeeds_silently() {
    // No branch matches, no default — propagate succeeds, b keeps its value.
    let mut sheet = Sheet::new();
    let mode = sheet.add_cell(0_i32);
    let a = sheet.add_cell(3_i32);
    let b = sheet.add_cell(99_i32);

    let rel_on = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
        .unwrap();

    sheet
        .add_conditional(mode, vec![(vec![1_i32], vec![rel_on])], vec![])
        .unwrap();

    // mode=0, no match, rel_on inactive.
    sheet.write(mode, 0_i32).unwrap();
    sheet.propagate().unwrap();
    // b unchanged: no method wrote to it.
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 99);
}

#[test]
fn conditional_default_branch_activates_when_no_key_matches() {
    let mut sheet = Sheet::new();
    let mode = sheet.add_cell(0_i32);
    let a = sheet.add_cell(3_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);

    let rel_double = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
        .unwrap();
    let rel_triple = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, c, |x: &i32| Ok(*x * 3))])
        .unwrap();

    sheet
        .add_conditional(
            mode,
            vec![(vec![1_i32], vec![rel_double])],
            vec![rel_triple], // default
        )
        .unwrap();

    // mode=1: double branch.
    sheet.write(mode, 1_i32).unwrap();
    sheet.write(a, 4_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 8);

    // mode=99: default branch.
    sheet.write(mode, 99_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 12);
}

#[test]
fn conditional_multi_key_branch_matches_any_key() {
    // Branch is active for mode=0 OR mode=2.
    let mut sheet = Sheet::new();
    let mode = sheet.add_cell(0_i32);
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(0_i32);

    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();

    sheet
        .add_conditional(mode, vec![(vec![0_i32, 2_i32], vec![rel])], vec![])
        .unwrap();

    sheet.write(a, 7_i32).unwrap();
    sheet.write(mode, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 7);

    sheet.write(mode, 2_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 7);

    // mode=1 does not match; b stays at its last derived value.
    sheet.write(mode, 1_i32).unwrap();
    sheet.propagate().unwrap();
    // b is no longer derived; it keeps the last value (7).
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 7);
}

#[test]
fn conditional_branch_switch_stability() {
    // When branch switches, previously derived cells should not block the new plan.
    // Setup: mode controls which of two independent relationships is active.
    // Branch 0: a→out (out = a * 2)
    // Branch 1: b→out (out = b * 3)
    let mut sheet = Sheet::new();
    let mode = sheet.add_cell(0_i32);
    let a = sheet.add_cell(4_i32);
    let b = sheet.add_cell(5_i32);
    let out = sheet.add_cell(0_i32);

    let rel_a = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, out, |x: &i32| Ok(*x * 2))])
        .unwrap();
    let rel_b = sheet
        .add_relationship(vec![Method::from_fn_1_1(b, out, |x: &i32| Ok(*x * 3))])
        .unwrap();

    sheet
        .add_conditional(
            mode,
            vec![
                (vec![0_i32], vec![rel_a]),
                (vec![1_i32], vec![rel_b]),
            ],
            vec![],
        )
        .unwrap();

    // mode=0: out derived from a.
    sheet.write(mode, 0_i32).unwrap();
    sheet.write(a, 4_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(out).unwrap(), 8);

    // mode=1: out derived from b. Must not conflict even though out has a stale derived strength.
    sheet.write(mode, 1_i32).unwrap();
    sheet.write(b, 5_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(out).unwrap(), 15);
}

#[test]
fn conditional_match_cell_is_derived_from_unconditional_relationship() {
    // The match cell (flag) is computed by an unconditional single-method relationship.
    let mut sheet = Sheet::new();
    let x = sheet.add_cell(5_i32);
    let flag = sheet.add_cell(false);
    let a = sheet.add_cell(3_i32);
    let b = sheet.add_cell(0_i32);

    // Unconditional: x → flag  (flag = x > 0)
    sheet
        .add_relationship(vec![Method::from_fn_1_1(x, flag, |x: &i32| Ok(*x > 0))])
        .unwrap();

    let rel_true = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
        .unwrap();

    sheet
        .add_conditional(flag, vec![(vec![true], vec![rel_true])], vec![])
        .unwrap();

    // x=5 > 0 → flag=true → rel_true active.
    sheet.write(x, 5_i32).unwrap();
    sheet.write(a, 3_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<bool>(flag).unwrap(), true);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 6);

    // x=-1 ≤ 0 → flag=false → no match, rel_true inactive.
    sheet.write(x, -1_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<bool>(flag).unwrap(), false);
    // b has no active relationship; it keeps its previous value.
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 6);
}
```

- [ ] **Step 2: Run to verify the tests fail**

```
cargo test -p property-model conditional_
```

Expected: all six new tests FAIL (panics or assertion errors, since `propagate` currently doesn't evaluate conditionals).

- [ ] **Step 3: Implement `match_cell_subgraph` helper in `src/sheet.rs`**

Add as a private method on `Sheet`:

```rust
/// Returns the set of unconditional relationships transitively needed to derive
/// the given `match_cells`.
///
/// Walks upstream (from each match cell, through relationships whose outputs include
/// the cell) collecting only relationships not in `self.conditional_relationships`.
/// Relationships that only take a match cell as *input* (not output) are skipped.
///
/// - Complexity: O(C·R) in the worst case where C = cells and R = relationships.
fn match_cell_subgraph(&self, match_cells: &[CellId]) -> HashSet<RelationshipId> {
    let mut result: HashSet<RelationshipId> = HashSet::new();
    let mut visited: HashSet<CellId> = HashSet::new();
    let mut queue: std::collections::VecDeque<CellId> = match_cells.iter().copied().collect();

    for &cell in match_cells {
        visited.insert(cell);
    }

    while let Some(cell) = queue.pop_front() {
        for &rel_id in &self.cells[cell].adj {
            if self.conditional_relationships.contains(&rel_id) {
                continue;
            }
            if result.contains(&rel_id) {
                continue;
            }
            let rel = &self.relationships[rel_id];
            // Only include relationships that output this cell.
            let outputs_cell = rel
                .methods
                .iter()
                .any(|m| m.outputs.contains(&cell));
            if !outputs_cell {
                continue;
            }
            result.insert(rel_id);
            // Enqueue all inputs of this relationship for upstream BFS.
            for method in &rel.methods {
                for &input in &method.inputs {
                    if visited.insert(input) {
                        queue.push_back(input);
                    }
                }
            }
        }
    }

    result
}
```

- [ ] **Step 4: Implement `build_active_set` helper in `src/sheet.rs`**

Add as a private method on `Sheet`:

```rust
/// Builds the active relationship set for the general planning pass.
///
/// Starts with all unconditional relationships (those not in
/// `self.conditional_relationships`), then evaluates each conditional: the first
/// branch whose keys contain the match cell's current value is selected, and its
/// relationships are added. If no branch matches, the default relationships are added.
///
/// - Complexity: O(R + C·B·K) where R = total relationships, C = conditionals,
///   B = branches per conditional, K = keys per branch.
fn build_active_set(&self) -> HashSet<RelationshipId> {
    let mut active: HashSet<RelationshipId> = self
        .relationships
        .keys()
        .filter(|id| !self.conditional_relationships.contains(id))
        .collect();

    for (_, cond) in &self.conditionals {
        let cell = &self.cells[cond.cell];
        let eq_fn = cell.eq_fn;
        let value = cell.value.as_ref();

        let mut matched = false;
        for branch in &cond.branches {
            if branch.keys.iter().any(|key| eq_fn(value, key.as_ref())) {
                for &rel_id in &branch.relationships {
                    active.insert(rel_id);
                }
                matched = true;
                break;
            }
        }
        if !matched {
            for &rel_id in &cond.default {
                active.insert(rel_id);
            }
        }
    }

    active
}
```

- [ ] **Step 5: Rewrite `Sheet::propagate` as a four-phase operation**

Replace the entire `propagate` method:

```rust
/// Runs the planning pass and executes the selected methods.
///
/// Clears the changed-cell set from the previous `propagate()` call before planning.
/// After propagation, call [`Sheet::changed`] to inspect which cells were updated,
/// and [`Sheet::clear_changed`] when done.
///
/// **Phase 1 — Pre-plan:** if any conditional match cells are derived (have an
/// in-edge in the unconditional relationship graph), the minimal unconditional
/// subgraph needed to compute them is planned and executed so their values are
/// current before branch evaluation.
///
/// **Phase 2 — Conditional evaluation:** each conditional's match cell value is
/// read and compared against branch keys; the active relationship set is built.
///
/// **Phase 3 — General plan:** the Adam algorithm runs on the active set.
///
/// **Phase 4 — Strength post-processing:** derived cells receive low-order strengths
/// in evaluation order, enforcing the stability invariant.
///
/// # Errors
///
/// - `Error::Conflict` — no valid method assignment exists.
/// - `Error::MethodFailed` — a method's function returned an error, or a method
///   produced the wrong number of outputs.
/// - `Error::TypeMismatch` — a method output's runtime type does not match the
///   cell's registered type.
pub fn propagate(&mut self) -> Result<(), Error> {
    self.clear_changed();

    // Phase 1: pre-plan for derived match cells.
    if !self.conditionals.is_empty() {
        let match_cells: Vec<CellId> =
            self.conditionals.values().map(|c| c.cell).collect();
        let pre_active = self.match_cell_subgraph(&match_cells);
        if !pre_active.is_empty() {
            let pre_plan =
                crate::planner::plan(&self.cells, &self.relationships, &pre_active)?;
            self.execute_plan(&pre_plan.execution_order)?;
        }
    }

    // Phase 2: evaluate conditionals and build the active relationship set.
    let active = self.build_active_set();

    // Phase 3: general plan on the active set.
    let plan = crate::planner::plan(&self.cells, &self.relationships, &active)?;
    self.execute_plan(&plan.execution_order)?;

    // Phase 4: assign derived-cell strengths in evaluation order.
    self.post_process_strengths(&plan.execution_order);

    self.last_plan = Some(plan.execution_order);
    Ok(())
}
```

- [ ] **Step 6: Run all tests**

```
cargo test -p property-model
```

Expected: all tests pass — existing tests unchanged, all six new integration tests pass.

- [ ] **Step 7: Lint and commit**

```
cargo clippy -p property-model -- -D warnings
cargo fmt --all
git add property-model/src/sheet.rs property-model/tests/integration.rs
git commit -m "feat(property-model): implement four-phase propagation with conditional branching"
```

---

## Self-Review

**Spec coverage:**

| Spec section | Covered by |
| --- | --- |
| §1 Equality support (`eq_fn`, `PartialEq` bound) | Task 1 |
| §2 Strength partitioning (high-bit write, post-processing) | Task 1 |
| §3 `ConditionalId`, `Branch`, `ConditionalData` | Task 2 |
| §3 `Error::InvalidConditional` | Task 2 |
| §4 `add_conditional` validation (all 6 rules) | Task 4 |
| §4 Branch semantics (first match, multi-key, empty default) | Task 5 tests |
| §5 Phase 1 pre-plan | Task 5 |
| §5 Phase 2 conditional evaluation | Task 5 |
| §5 Phase 3 general plan on active set | Task 5 |
| §5 Phase 4 strength post-processing | Task 1 (helper), Task 5 (wired in) |
| §5 `last_plan` = Phase 3 order only | Task 5 Step 5 |
| §5 `propagate_without_replan` precondition note | Task 1 Step 4e |
| §6 All validation rules | Task 4 tests cover all 5 rows |
| §7 All files changed | Tasks 1–5 |

**Placeholder scan:** No TBD, TODO, or vague steps found.

**Type consistency:**
- `ConditionalId` — defined Task 2, used in Task 4 return type ✓
- `Branch.keys: Vec<Box<dyn Any>>` — defined Task 2, populated in Task 4 ✓
- `ConditionalData.default: Vec<RelationshipId>` — defined Task 2, used in Task 4 and Task 5 ✓
- `plan(cells, relationships, active: &HashSet<RelationshipId>)` — changed Task 3, called in Task 5 ✓
- `post_process_strengths(&mut self, execution_order: &[(RelationshipId, usize)])` — defined Task 1, called in Task 5 ✓
- `match_cell_subgraph(&self, match_cells: &[CellId]) -> HashSet<RelationshipId>` — defined Task 5 ✓
- `build_active_set(&self) -> HashSet<RelationshipId>` — defined Task 5 ✓
