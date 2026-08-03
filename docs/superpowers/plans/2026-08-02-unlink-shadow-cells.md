# Automatic Shadow Values for Self-Reference and Conditional Forcing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `adam-rs` cells a second, automatically-managed "derived" value slot so that self-referencing methods and conditionally-forced relationships stop permanently overwriting a cell's original written value — matching
[`docs/superpowers/specs/2026-08-02-unlink-shadow-cells-design.md`](../specs/2026-08-02-unlink-shadow-cells-design.md).

**Architecture:** `CellData` splits its single value into `source` (written by `write`/`add_cell`, and also by `execute_plan` for any output that isn't shadowed) and `derived: Option<Box<dyn Any>>` (written only by `execute_plan`, only for shadowed outputs, reset to `None` for every cell at the start of every `propagate()` before planning). `Sheet::read()` returns `derived.unwrap_or(source)`; a new `Sheet::source()` exposes the raw source. `execute_plan` decides, per output cell of a firing method, whether to write into `derived` (self-referencing output, or pure output of a conditionally-registered relationship) or straight into `source` (everything else, unchanged from today). No `Clone` bound is added anywhere — each slot has exactly one writer for any given cell in any given round, and values are moved, never copied into both.

**Tech Stack:** Rust 2024 edition, `adam-rs` crate only (`cell.rs`, `sheet.rs`, `tests/integration.rs`). No changes to `adam-lang`, `begin`, or any other crate.

## Global Constraints

- `cargo fmt --all` before every commit (pre-commit hook enforced).
- `cargo build --workspace` and `cargo test --workspace` must produce **zero compiler warnings** (not just pass — read the output).
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` must pass.
- `cargo clippy -p begin --no-default-features --all-targets -- -D warnings` must pass.
- `cargo clippy -p begin --all-targets -- -D warnings` must pass.
- Every function/field needs a `///` contract-style doc comment (Summary; `Preconditions`/`# Errors` as bulleted `- Precondition:`/error list; `Postconditions` only when non-obvious; `- Complexity:` when not O(1)). Public APIs need `# Examples`.
- Unit/integration tests are written against the public contract only, never against implementation details.
- No new crate dependencies; no new `adam-lang` syntax; no changes to any public API signature other than the new `Sheet::source` method (existing `add_cell`, `write`, `read` signatures are unchanged).

---

## File Map

| File | Responsibility |
|---|---|
| `adam-rs/src/cell.rs` | `CellData` struct: `source`/`derived` fields, `effective()` helper. |
| `adam-rs/src/sheet.rs` | `read`, `write`, new `source`, `build_active_set`, `conditional_active_branch`, `execute_plan`, `propagate`. |
| `adam-rs/tests/integration.rs` | New end-to-end scenarios: self-ref pressure persistence, conditional forcing/reversion, staleness regression, branch-overlap. |

---

### Task 1: Split `CellData` into `source` + `derived`, no behavior change

**Files:**
- Modify: `adam-rs/src/cell.rs:16-58`
- Modify: `adam-rs/src/sheet.rs:84-95` (`add_cell`), `:306-345` (`write`, `read`), `:457-487` (`build_active_set`), `:583-625` (`execute_plan`), `:774-784` (`conditional_active_branch`)

**Interfaces:**
- Produces: `CellData { source: Box<dyn Any>, derived: Option<Box<dyn Any>>, .. }` and `CellData::effective(&self) -> &dyn Any`, both `pub(crate)`, used by every later task.

This task is a pure refactor: after it, `derived` is always `None` everywhere (nothing populates it yet), so every existing test must continue to pass unchanged.

- [ ] **Step 1: Update the existing `CellData` unit test to the new field names (it will fail to compile)**

In `adam-rs/src/cell.rs`, replace the `cell_data_initial_state` test body:

```rust
    #[test]
    fn cell_data_initial_state() {
        let data = CellData {
            source: Box::new(42_i32),
            derived: None,
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
        assert!(data.derived.is_none());
        assert_eq!(*data.source.downcast_ref::<i32>().unwrap(), 42);
        assert_eq!(*data.effective().downcast_ref::<i32>().unwrap(), 42);
        let x: i32 = 42;
        let y: i32 = 99;
        assert!((data.eq_fn)(&x, &x));
        assert!(!(data.eq_fn)(&x, &y));
    }
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p adam-rs cell_data_initial_state`
Expected: compile error — `CellData` has no field `source`/`derived`, and no method `effective`.

- [ ] **Step 3: Update the `CellData` struct and add `effective()`**

In `adam-rs/src/cell.rs`, replace the struct definition:

```rust
/// Internal storage for a single value cell.
pub(crate) struct CellData {
    /// The value from the most recent `write()`/`add_cell`. Never written by
    /// `Sheet::propagate`; self-referencing methods and conditionally forced
    /// relationships read/write around this field via `derived` instead.
    pub(crate) source: Box<dyn Any>,
    /// The value most recently produced by a method this round, if this cell was
    /// shadowed (a self-referencing output, or a pure output of a conditionally
    /// registered relationship). Reset to `None` for every cell at the start of
    /// every `Sheet::propagate` call, before planning begins.
    pub(crate) derived: Option<Box<dyn Any>>,
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

impl CellData {
    /// Returns the effective current value: `derived` if present, else `source`.
    pub(crate) fn effective(&self) -> &dyn Any {
        self.derived.as_deref().unwrap_or(self.source.as_ref())
    }
}
```

- [ ] **Step 4: Update every construction/use site in `adam-rs/src/sheet.rs`**

`add_cell` (around line 84):

```rust
    pub fn add_cell<T: Any + PartialEq + 'static>(&mut self, value: T) -> CellId {
        self.next_strength += 1;
        let strength = self.next_strength | (1u64 << 63);
        self.cells.insert(CellData {
            source: Box::new(value),
            derived: None,
            type_id: TypeId::of::<T>(),
            strength,
            changed: false,
            adj: Vec::new(),
            eq_fn: |a, b| a.downcast_ref::<T>() == b.downcast_ref::<T>(),
        })
    }
```

`write` (around line 306):

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
        cell.source = Box::new(value);
        cell.derived = None;
        Ok(())
    }
```

Add a doc line above `write` noting the new postcondition: `/// - Postcondition: any pending derived override is cleared, so the written value is immediately visible via \`read()\`.`

`read` (around line 330), updated summary + body:

```rust
    /// Returns a shared reference to the cell's effective current value: its derived
    /// override if one exists, otherwise its source (last written) value.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `id` is not a cell in this sheet.
    /// - `Error::TypeMismatch` — `T` does not match the cell's registered `TypeId`.
    pub fn read<T: Any + 'static>(&self, id: CellId) -> Result<&T, Error> {
        let cell = self.cells.get(id).ok_or(Error::InvalidId)?;
        if cell.type_id != TypeId::of::<T>() {
            return Err(Error::TypeMismatch {
                expected: cell.type_id,
                found: TypeId::of::<T>(),
            });
        }
        Ok(cell.effective().downcast_ref::<T>().expect("type checked above"))
    }
```

`build_active_set` (around line 467), only the two lines reading the match cell's value:

```rust
            let cell = &self.cells[cond.cell];
            let eq_fn = cell.eq_fn;
            let value = cell.effective();
```

`conditional_active_branch` (around line 776):

```rust
        let cond = self.conditionals.get(id)?;
        let cell = &self.cells[cond.cell];
        let eq_fn = cell.eq_fn;
        let value = cell.effective();
```

`execute_plan` (around line 583) — input gathering and output writing, unchanged in *behavior* this task (no self-ref/conditional distinction yet — that's Tasks 3 and 4), just renamed:

```rust
    fn execute_plan(&mut self, execution_order: &[(RelationshipId, usize)]) -> Result<(), Error> {
        for &(rel_id, method_idx) in execution_order {
            let (outputs, output_ids) = {
                let method = &self.relationships[rel_id].methods[method_idx];
                let inputs: Vec<&dyn Any> = method
                    .inputs
                    .iter()
                    .map(|&id| self.cells[id].effective())
                    .collect();
                let outputs = (method.function)(&inputs).map_err(Error::MethodFailed)?;
                let output_ids = method.outputs.clone();
                (outputs, output_ids)
            };

            if outputs.len() != output_ids.len() {
                return Err(Error::MethodFailed(anyhow::anyhow!(
                    "method produced {} outputs but relationship expects {}",
                    outputs.len(),
                    output_ids.len()
                )));
            }

            for (cell_id, new_value) in output_ids.into_iter().zip(outputs) {
                let cell = &mut self.cells[cell_id];
                let found = new_value.as_ref().type_id();
                if found != cell.type_id {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found,
                    });
                }
                cell.source = new_value;
                if !cell.changed {
                    cell.changed = true;
                    self.changed_cells.push(cell_id);
                }
            }
        }
        Ok(())
    }
```

- [ ] **Step 5: Run the full test suite to verify nothing changed behaviorally**

Run: `cargo test --workspace`
Expected: PASS — every existing test (including `cell_data_initial_state`) passes unchanged, because `derived` is always `None` at this point, so `effective()` is always equivalent to the old `value`.

- [ ] **Step 6: Commit**

```bash
git add adam-rs/src/cell.rs adam-rs/src/sheet.rs
git commit -m "refactor(adam-rs): split CellData value into source + derived slots

Pure refactor, no behavior change: derived is always None until later
tasks populate it for self-referencing and conditionally forced cells."
```

---

### Task 2: `Sheet::source()` public accessor

**Files:**
- Modify: `adam-rs/src/sheet.rs` (add method near `read`, after line ~345)
- Test: `adam-rs/src/sheet.rs` test module (insert after `read_wrong_type_returns_type_mismatch`, currently ending around line 995)

**Interfaces:**
- Consumes: `CellData.source: Box<dyn Any>` (Task 1).
- Produces: `Sheet::source::<T: Any + 'static>(&self, id: CellId) -> Result<&T, Error>`, used by Tasks 3, 4, 5, 6's tests.

- [ ] **Step 1: Write the failing tests**

In `adam-rs/src/sheet.rs`'s `#[cfg(test)] mod tests` block, insert after `read_wrong_type_returns_type_mismatch`:

```rust
    #[test]
    fn source_matches_read_for_a_plain_unshadowed_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(3_i32);
        assert_eq!(*sheet.source::<i32>(a).unwrap(), 3);
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 3);

        sheet.write(a, 8_i32).unwrap();
        assert_eq!(*sheet.source::<i32>(a).unwrap(), 8);
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 8);
    }

    #[test]
    fn source_returns_invalid_id_for_unknown_cell() {
        let sheet = Sheet::new();
        assert!(matches!(
            sheet.source::<i32>(CellId::default()),
            Err(Error::InvalidId)
        ));
    }

    #[test]
    fn source_wrong_type_returns_type_mismatch() {
        let mut sheet = Sheet::new();
        let id = sheet.add_cell(0_i32);
        assert!(matches!(
            sheet.source::<f64>(id),
            Err(Error::TypeMismatch { .. })
        ));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-rs source_`
Expected: compile error — no method `source` on `Sheet`.

- [ ] **Step 3: Implement `Sheet::source`**

In `adam-rs/src/sheet.rs`, add immediately after `read`:

```rust
    /// Returns the last explicitly written (source) value, ignoring any derived
    /// override produced by a self-referencing method or a conditionally forced
    /// relationship.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `id` is not a cell in this sheet.
    /// - `Error::TypeMismatch` — `T` does not match the cell's registered `TypeId`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use adam_rs::Sheet;
    ///
    /// let mut sheet = Sheet::new();
    /// let a = sheet.add_cell(3_i32);
    /// sheet.write(a, 8_i32).unwrap();
    /// assert_eq!(*sheet.source::<i32>(a).unwrap(), 8);
    /// ```
    pub fn source<T: Any + 'static>(&self, id: CellId) -> Result<&T, Error> {
        let cell = self.cells.get(id).ok_or(Error::InvalidId)?;
        if cell.type_id != TypeId::of::<T>() {
            return Err(Error::TypeMismatch {
                expected: cell.type_id,
                found: TypeId::of::<T>(),
            });
        }
        Ok(cell.source.downcast_ref::<T>().expect("type checked above"))
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p adam-rs source_`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): add Sheet::source accessor for a cell's raw written value"
```

---

### Task 3: Self-referencing outputs shadow into `derived`; self-ref inputs always read `source`

**Files:**
- Modify: `adam-rs/src/sheet.rs` (`execute_plan`, around line 583 post-Task-1)
- Test: `adam-rs/tests/integration.rs` (insert near `self_ref_direct_clamp`/`self_ref_le_chain`)

**Interfaces:**
- Consumes: `CellData.source`, `CellData.derived`, `CellData::effective()` (Task 1); `Sheet::source()` (Task 2).
- Produces: no new public signatures — `execute_plan`'s internal shadowing behavior, relied on by Task 4 (which extends the same `shadow` condition) and Task 5.

- [ ] **Step 1: Write the failing integration test**

In `adam-rs/tests/integration.rs`, insert near `self_ref_direct_clamp`:

```rust
#[test]
fn self_ref_pressure_persists_without_rewriting_anchor() {
    // a = min(a, b): b applies downward pressure on a, but a's original written
    // value must survive across rounds where only b is rewritten.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| {
            Ok((*x).min(*y))
        })])
        .unwrap();

    sheet.write(a, 10_i32).unwrap();
    sheet.write(b, 3_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 3);
    assert_eq!(*sheet.source::<i32>(a).unwrap(), 10);

    // Only b changes; a's original 10 (not the previous derived 3) is used.
    sheet.write(b, 20_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 10);
    assert_eq!(*sheet.source::<i32>(a).unwrap(), 10);

    sheet.write(b, 5_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    assert_eq!(*sheet.source::<i32>(a).unwrap(), 10);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p adam-rs self_ref_pressure_persists_without_rewriting_anchor`
Expected: FAIL at the second assertion block — `a` reads back `3` (the previous round's overwrite of `source`), not `10`, because `execute_plan` still writes every output straight into `source`.

- [ ] **Step 3: Make self-referencing outputs shadow into `derived`, and self-ref inputs read `source`**

In `adam-rs/src/sheet.rs`, replace `execute_plan`'s body:

```rust
    fn execute_plan(&mut self, execution_order: &[(RelationshipId, usize)]) -> Result<(), Error> {
        for &(rel_id, method_idx) in execution_order {
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
                    .map(|o| method.inputs.contains(o))
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

Note: `shadow` here is exactly "is this output also one of this method's own inputs" (self-referencing). Task 4 extends this same boolean with the conditional-forcing case.

- [ ] **Step 4: Run the test to verify it passes, then run the full suite**

Run: `cargo test -p adam-rs self_ref_pressure_persists_without_rewriting_anchor`
Expected: PASS.

Run: `cargo test --workspace`
Expected: PASS — `self_ref_direct_clamp` and `self_ref_le_chain` still pass because they rewrite every cell every round, which never observes the difference between `source` and `derived`.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/sheet.rs adam-rs/tests/integration.rs
git commit -m "feat(adam-rs): shadow self-referencing method outputs into derived

Self-referencing inputs now always read the pre-propagation source value,
so a self-referencing constraint applies pressure against the cell's
original written value every round instead of an ever-drifting accumulator."
```

---

### Task 4: Conditionally-forced pure outputs also shadow into `derived`

**Files:**
- Modify: `adam-rs/src/sheet.rs` (`execute_plan`, the `shadow_outputs` computation from Task 3)
- Test: `adam-rs/tests/integration.rs` (insert near `conditional_activates_matching_branch`)

**Interfaces:**
- Consumes: `self.conditional_relationships: HashSet<RelationshipId>` (existing field, populated by `add_conditional`); the `shadow_outputs` mechanism from Task 3.
- Produces: no new public signatures.

- [ ] **Step 1: Write the failing integration tests**

In `adam-rs/tests/integration.rs`, insert near `conditional_activates_matching_branch`:

```rust
#[test]
fn conditional_forced_cell_shadows_original_value() {
    let mut sheet = Sheet::new();
    let p = sheet.add_cell(0_i32);
    let a = sheet.add_cell(7_i32);
    let b = sheet.add_cell(0_i32);

    let rel_force = sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_conditional(p, vec![(vec![1_i32], vec![rel_force])], vec![])
        .unwrap();

    sheet.write(p, 1_i32).unwrap();
    sheet.write(b, 42_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 42);
    assert_eq!(*sheet.source::<i32>(a).unwrap(), 7);
}

#[test]
fn explicit_write_to_forced_cell_takes_immediate_effect() {
    let mut sheet = Sheet::new();
    let p = sheet.add_cell(1_i32);
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);

    let rel_force = sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_conditional(p, vec![(vec![1_i32], vec![rel_force])], vec![])
        .unwrap();

    sheet.write(p, 1_i32).unwrap();
    sheet.write(b, 42_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 42);

    // Direct write takes effect immediately, before the next propagate() re-forces it.
    sheet.write(a, 99_i32).unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 99);
}
```

- [ ] **Step 2: Run the tests to verify the first one fails**

Run: `cargo test -p adam-rs conditional_forced_cell_shadows_original_value`
Expected: FAIL — `sheet.source::<i32>(a)` returns `42`, not `7`, because `a` is a pure output (not self-referencing), so Task 3's `shadow` rule doesn't cover it yet; `execute_plan` still writes it straight into `source`.

(`explicit_write_to_forced_cell_takes_immediate_effect` already passes — `write()` has always taken immediate effect — it's included here as a regression guard for Task 5's changes, not as new failing behavior.)

- [ ] **Step 3: Extend the shadow condition to cover conditionally forced pure outputs**

In `adam-rs/src/sheet.rs`, inside `execute_plan`, change the `shadow_outputs` computation (the rest of the function is unchanged from Task 3):

```rust
            let is_conditional = self.conditional_relationships.contains(&rel_id);
            let (outputs, output_ids, shadow_outputs) = {
                let method = &self.relationships[rel_id].methods[method_idx];
                let inputs: Vec<&dyn Any> = method
                    .inputs
                    .iter()
                    .map(|&id| {
                        if method.outputs.contains(&id) {
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
```

(`is_conditional` is computed before the scoped block since it only needs `&self.conditional_relationships`, a different field than the one borrowed inside the block.)

- [ ] **Step 4: Run the tests to verify they pass, then run the full suite**

Run: `cargo test -p adam-rs conditional_forced_cell_shadows_original_value explicit_write_to_forced_cell_takes_immediate_effect`
Expected: PASS.

Run: `cargo test --workspace`
Expected: PASS — existing conditional tests (`conditional_activates_matching_branch`, `conditional_no_match_and_no_default_succeeds_silently`, etc.) are unaffected because none of them inspect `source()`, only `read()`, whose value is unchanged by this task.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/sheet.rs adam-rs/tests/integration.rs
git commit -m "feat(adam-rs): shadow conditionally forced pure outputs into derived

A cell forced by an active conditional branch keeps its pre-force source
value intact underneath the forced derived value."
```

---

### Task 5: Reset `derived` at the start of every `propagate()`; restore-on-deactivation + change tracking

**Files:**
- Modify: `adam-rs/src/sheet.rs` (`propagate`, around line 543 post-Task-1)
- Test: `adam-rs/tests/integration.rs` (insert near the conditional tests added in Task 4)

**Interfaces:**
- Consumes: `CellData.derived` (Task 1), the shadow-writing behavior from Tasks 3–4.
- Produces: no new public signatures — fixes the correctness/change-tracking gap described in the design doc's "why the round-start reset makes this safe" section.

- [ ] **Step 1: Write the failing integration tests**

In `adam-rs/tests/integration.rs`:

```rust
#[test]
fn conditional_forced_cell_reverts_to_source_when_deactivated() {
    let mut sheet = Sheet::new();
    let p = sheet.add_cell(0_i32);
    let a = sheet.add_cell(7_i32);
    let b = sheet.add_cell(0_i32);

    let rel_force = sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_conditional(p, vec![(vec![1_i32], vec![rel_force])], vec![])
        .unwrap();

    sheet.write(p, 1_i32).unwrap();
    sheet.write(b, 42_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 42);

    sheet.write(p, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(
        *sheet.read::<i32>(a).unwrap(),
        7,
        "a must revert to its original value, not stay at the stale forced 42"
    );
    assert_eq!(*sheet.source::<i32>(a).unwrap(), 7);
}

#[test]
fn changed_reports_cell_reverted_by_conditional_deactivation() {
    let mut sheet = Sheet::new();
    let p = sheet.add_cell(0_i32);
    let a = sheet.add_cell(7_i32);
    let b = sheet.add_cell(0_i32);

    let rel_force = sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_conditional(p, vec![(vec![1_i32], vec![rel_force])], vec![])
        .unwrap();

    sheet.write(p, 1_i32).unwrap();
    sheet.write(b, 42_i32).unwrap();
    sheet.propagate().unwrap();

    sheet.write(p, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(
        sheet.changed().any(|id| id == a),
        "a's effective value changed (42 -> 7) even though no method wrote to it this round"
    );
}

#[test]
fn pure_input_never_observes_stale_derived_after_conditional_deactivates() {
    // a is forced to b's value only when p == 1. c always reads a directly
    // (c = a * 10), regardless of the conditional. When p flips back to 0, a
    // must revert to its own source value (7) before c is recomputed — c must
    // never see a's stale forced value from the previous round.
    let mut sheet = Sheet::new();
    let p = sheet.add_cell(0_i32);
    let a = sheet.add_cell(7_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);

    let rel_force = sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, c, |x: &i32| Ok(*x * 10))])
        .unwrap();
    sheet
        .add_conditional(p, vec![(vec![1_i32], vec![rel_force])], vec![])
        .unwrap();

    sheet.write(p, 1_i32).unwrap();
    sheet.write(b, 42_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 42);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 420);

    sheet.write(p, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 7);
    assert_eq!(
        *sheet.read::<i32>(c).unwrap(),
        70,
        "c must be derived from a's reverted source value, not the stale forced 42"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-rs conditional_forced_cell_reverts_to_source_when_deactivated changed_reports_cell_reverted_by_conditional_deactivation pure_input_never_observes_stale_derived_after_conditional_deactivates`
Expected: FAIL on all three — `a`'s `derived` from the `p == 1` round is never cleared once `rel_force` stops firing, so `read(a)` stays at `42`, `changed()` never reports `a` on the second round, and `c` recomputes to `420` again instead of `70`.

- [ ] **Step 3: Add the round-start reset and end-of-round change bookkeeping**

In `adam-rs/src/sheet.rs`, update `propagate`:

```rust
    /// Runs the planning pass and executes the selected methods.
    ///
    /// Clears the changed-cell set from the previous `propagate()` call before planning.
    /// After propagation, call [`Sheet::changed`] to inspect which cells were updated,
    /// and [`Sheet::clear_changed`] when done.
    ///
    /// **Phase 0 — Derived reset:** every cell's derived override is cleared before
    /// planning begins, so no pure-input read this round can observe a derived value
    /// left over from a previous round.
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
    /// **Phase 5 — Reversion change-tracking:** a cell whose derived override existed
    /// before this round but wasn't reclaimed by any method this round has effectively
    /// reverted to its source value (e.g. its forcing conditional went inactive); it is
    /// marked changed even though no method wrote to it this round.
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

        // Phase 0: snapshot cells with a live derived override (for Phase 5 only),
        // then reset every cell's derived override before planning begins.
        let previously_derived: Vec<CellId> = self
            .cells
            .iter()
            .filter(|(_, cell)| cell.derived.is_some())
            .map(|(id, _)| id)
            .collect();
        for (_, cell) in self.cells.iter_mut() {
            cell.derived = None;
        }

        // Phase 1: pre-plan for derived match cells.
        if !self.conditionals.is_empty() {
            let match_cells: Vec<CellId> = self.conditionals.values().map(|c| c.cell).collect();
            let pre_active = self.match_cell_subgraph(&match_cells);
            if !pre_active.is_empty() {
                let pre_plan = crate::planner::plan(&self.cells, &self.relationships, &pre_active)?;
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

        // Phase 5: cells that reverted (had a derived override, didn't get a fresh one
        // this round) need explicit change-tracking.
        for id in previously_derived {
            if let Some(cell) = self.cells.get_mut(id)
                && cell.derived.is_none()
                && !cell.changed
            {
                cell.changed = true;
                self.changed_cells.push(id);
            }
        }

        self.last_forced = Some(plan.forced_outputs);
        self.last_forced_relationships = Some(plan.forced_relationships);
        self.last_plan = Some(plan.execution_order);
        Ok(())
    }
```

- [ ] **Step 4: Run the tests to verify they pass, then run the full suite**

Run: `cargo test -p adam-rs conditional_forced_cell_reverts_to_source_when_deactivated changed_reports_cell_reverted_by_conditional_deactivation pure_input_never_observes_stale_derived_after_conditional_deactivates`
Expected: PASS.

Run: `cargo test --workspace`
Expected: PASS — all tests from Tasks 1–4 and every pre-existing test still pass; the Phase 0 reset has no effect on cells that never shadow anything, and self-referencing cells never consult `derived` for their own self-ref input regardless of the reset.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/sheet.rs adam-rs/tests/integration.rs
git commit -m "fix(adam-rs): reset derived overrides at the start of propagate, not after

Resetting after execution let a pure-input read mid-round observe a stale
derived value from the previous round (e.g. a cell whose forcing
conditional just went inactive). Resetting before planning begins means
there is nothing stale left to observe — a genuine source this round was
already cleared and never reclaimed."
```

---

### Task 6: Regression test — a cell shadowed as self-ref in one conditional branch and as a forced pure output in another

**Files:**
- Test only: `adam-rs/tests/integration.rs` (insert near the conditional tests)

**Interfaces:**
- Consumes: everything from Tasks 1–5. No new production code — this validates that the per-output, per-firing shadow decision (Task 3 + Task 4's `shadow = method.inputs.contains(&output) || is_conditional`) handles a cell that's classified differently depending on which relationship happens to fire, with no static per-cell classification required.

- [ ] **Step 1: Write the test**

In `adam-rs/tests/integration.rs`:

```rust
#[test]
fn cell_shadowed_as_self_ref_in_one_branch_and_forced_output_in_another() {
    // p == 0: a <= b enforced by a two-way self-referencing relationship.
    // p != 0 (default): a and b are forced from each other directly, whichever
    // is the stronger (more recently written) cell wins.
    let mut sheet = Sheet::new();
    let p = sheet.add_cell(0_i32);
    let a = sheet.add_cell(4_i32);
    let b = sheet.add_cell(9_i32);

    let rel_self_ref = sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| Ok((*x).min(*y))),
            Method::from_fn_2_1([a, b], b, |x: &i32, y: &i32| Ok((*x).max(*y))),
        ])
        .unwrap();
    let rel_force = sheet
        .add_relationship(vec![
            Method::from_fn_1_1(b, a, |y: &i32| Ok(*y)),
            Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
        ])
        .unwrap();
    sheet
        .add_conditional(p, vec![(vec![0_i32], vec![rel_self_ref])], vec![rel_force])
        .unwrap();

    // p == 0: self-referencing branch. a=4, b=9 already satisfy a <= b: unchanged.
    sheet.write(p, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 4);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 9);
    assert_eq!(*sheet.source::<i32>(a).unwrap(), 4);
    assert_eq!(*sheet.source::<i32>(b).unwrap(), 9);

    // p == 1: default (forcing) branch. b is the more recently written cell,
    // so a <- b.
    sheet.write(a, 4_i32).unwrap();
    sheet.write(b, 20_i32).unwrap();
    sheet.write(p, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 20);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 20);
    // Sources are untouched by the forcing branch.
    assert_eq!(*sheet.source::<i32>(a).unwrap(), 4);
    assert_eq!(*sheet.source::<i32>(b).unwrap(), 20);

    // Back to p == 0: self-ref recomputed fresh from each cell's own source
    // (4 and 20), not from the stale forced value.
    sheet.write(p, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 4);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 20);
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p adam-rs cell_shadowed_as_self_ref_in_one_branch_and_forced_output_in_another`
Expected: PASS immediately (Tasks 1–5 already implement everything this test needs). If it fails, that indicates a gap in Tasks 3–5, not a new task — stop and re-examine those tasks' implementations against the failure before proceeding.

- [ ] **Step 3: Run the full workspace suite**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add adam-rs/tests/integration.rs
git commit -m "test(adam-rs): cover a cell shadowed as self-ref in one branch, forced output in another"
```

---

### Task 7: Full verification pass

**Files:** none (verification only).

- [ ] **Step 1: Format**

Run: `cargo fmt --all`

- [ ] **Step 2: Build the whole workspace and confirm zero warnings**

Run: `cargo build --workspace`
Expected: builds clean; read the full output — zero warnings, not just success.

- [ ] **Step 3: Run every test, including doc tests, and confirm zero warnings**

Run: `cargo test --workspace`
Run: `cargo test --doc --workspace`
Expected: all pass; read the full output — zero warnings.

- [ ] **Step 4: Run all three required clippy invocations**

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Run: `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
Run: `cargo clippy -p begin --all-targets -- -D warnings`
Expected: all three pass with no warnings.

- [ ] **Step 5: Build docs**

Run: `cargo doc --lib --no-deps --workspace`
Expected: builds clean, no warnings (missing-docs, broken intra-doc links, etc.).

- [ ] **Step 6: Commit any formatting fixes**

```bash
git add -A
git status
```

If `cargo fmt --all` changed anything, commit it:

```bash
git commit -m "chore(adam-rs): cargo fmt"
```

If nothing changed, skip this commit.
