# Filters on Source Cells as Live Self-Referential Corrections Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a source cell's filter behave exactly like a self-referencing method —
`source` holds the raw last-written value forever, and the filter's live output goes
into `derived`, recomputed fresh every `propagate()` — closing a shrinking-accumulator
bug where a filter's conformed output permanently overwrites the raw value it was
computed from.

**Architecture:** `write()` and `add_filter` stop evaluating a cell's filter at all —
they become exactly as simple for a filtered cell as for an unfiltered one.
`execute_plan`'s `PlanStep::FilterReclamp` step becomes the *only* place a source
cell's filter output is ever computed: it reads the cell's own `source` (never
`effective()`) plus its argument cells' `effective()` values, and writes the result
into `derived` unconditionally (no equality check), exactly matching how a
self-referencing method's shadowed output already works. A related latent gap is fixed
first: a zero-argument filter (`Filter::from_fn_0`) on a cell with no relationship
membership never became a node in the planner's dependency graph, so it would silently
stop being applied at all once `write()`/`add_filter` no longer compensate for it.

**Tech Stack:** Rust, `adam-rs` crate only (`sheet.rs`, `filter.rs`,
`planner/digraph.rs`) — no new dependencies, no `begin` changes.

**Spec:** [docs/superpowers/specs/2026-08-26-adam-rs-filter-shadow-state-design.md](../specs/2026-08-26-adam-rs-filter-shadow-state-design.md)

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
  the implementation (Task 1's `digraph.rs` tests are the one deliberate exception:
  `add_filter_edges` is `pub(crate)`, not part of the crate's public API, matching that
  module's own existing test style).
- Arithmetic on signed integers uses `checked_*`, not wrapping — not applicable here (no
  new arithmetic), noted for completeness.

---

### Task 1: `digraph::add_filter_edges` — zero-argument filter still gets a node

**Files:**
- Modify: `adam-rs/src/planner/digraph.rs`

**Interfaces:**
- Modifies: `pub(crate) fn add_filter_edges(adj: &mut HashMap<Node, Vec<Node>>, cells:
  &SlotMap<CellId, CellData>, assignment: &Assignment)` (existing, unchanged signature).

- [ ] **Step 1: Write the failing test**

Add to the `tests` module at the bottom of `adam-rs/src/planner/digraph.rs`, right after
`add_filter_edges_skips_a_filtered_cell_claimed_by_the_assignment`:

```rust
    #[test]
    fn add_filter_edges_adds_a_node_for_a_zero_argument_filter_with_no_relationship_membership()
     {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(500_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();

        let assignment =
            Assignment::solve(&sheet.relationships, &HashSet::new(), &HashSet::new()).unwrap();
        let mut adj: HashMap<Node, Vec<Node>> = HashMap::new();
        add_filter_edges(&mut adj, &sheet.cells, &assignment);

        // `a` has no filter args and belongs to no relationship, so nothing would
        // otherwise ever insert it into the digraph — without this, plan() would
        // never emit a FilterReclamp step for it.
        assert!(adj.contains_key(&Node::Cell(a)));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p adam-rs add_filter_edges_adds_a_node_for_a_zero_argument_filter`
Expected: FAIL — `assertion failed: adj.contains_key(&Node::Cell(a))` (today,
`add_filter_edges` never inserts an entry for a filter with an empty `args` list).

- [ ] **Step 3: Fix the implementation**

In `adam-rs/src/planner/digraph.rs`, replace:

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
        for &arg in &filter.args {
            adj.entry(Node::Cell(arg))
                .or_default()
                .push(Node::Cell(cell_id));
        }
    }
}
```

with:

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
        // Ensures the filtered cell is a node even when it has no args and belongs to
        // no relationship (a zero-argument filter contributes no edges below), so it
        // always lands in its own trivial tarjan_scc component and always gets a
        // PlanStep::FilterReclamp.
        adj.entry(Node::Cell(cell_id)).or_default();
        for &arg in &filter.args {
            adj.entry(Node::Cell(arg))
                .or_default()
                .push(Node::Cell(cell_id));
        }
    }
}
```

Also update the function's doc comment — replace its final line:

```rust
/// - Complexity: O(C · a) where C = cells with a filter, a = arguments per filter.
```

with:

```rust
/// - Postcondition: every filtered, unclaimed cell is a key in `adj` after this call,
///   even one with zero filter args or no relationship membership.
///
/// - Complexity: O(C · a) where C = cells with a filter, a = arguments per filter.
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-rs --lib planner::`
Expected: PASS, including the new test and all pre-existing `digraph.rs`/`planner.rs`
tests (in particular `add_filter_edges_adds_edge_from_argument_to_filtered_source_cell`
and `add_filter_edges_skips_a_filtered_cell_claimed_by_the_assignment`, unaffected by
this change).

- [ ] **Step 5: Run the full test suite**

Run: `cargo test -p adam-rs`
Expected: PASS, no regressions.

- [ ] **Step 6: Commit**

```bash
git add adam-rs/src/planner/digraph.rs
git commit -m "fix(adam-rs): add_filter_edges gives a zero-argument filter its own graph node"
```

---

### Task 2: `execute_plan`'s `PlanStep::FilterReclamp` — read `source`, write `derived` unconditionally

**Files:**
- Modify: `adam-rs/src/sheet.rs`
- Modify: `adam-rs/src/filter.rs`

**Interfaces:**
- Modifies: `Sheet::execute_plan`'s private `PlanStep::FilterReclamp` match arm (no
  signature change).
- Modifies: `pub enum FilterViolation` doc comments only (no variant/field change).

- [ ] **Step 1: Write the failing tests**

Add to `adam-rs/src/sheet.rs`'s `mod tests`, right after
`propagate_reclamps_before_a_relationship_consumes_the_reclamped_value`:

```rust
    #[test]
    fn filtered_source_cell_springs_back_to_its_original_value_when_a_bound_loosens() {
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

        sheet.write(bound, 100_i32).unwrap();
        sheet.propagate().unwrap();
        // a's original 50 must survive in `source` across the whole round-trip: it
        // springs back once the bound loosens again, rather than staying stuck at the
        // intermediate clamp.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 50);
    }

    #[test]
    fn filter_reclamp_records_failed_violation_when_the_filters_function_returns_the_wrong_type()
     {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let filter = Filter::new(TypeId::of::<i32>(), vec![], vec![], |_value, _args| {
            Ok(Box::new(1.5_f64) as Box<dyn Any>)
        });
        sheet.add_filter(a, filter).unwrap();

        sheet.propagate().unwrap();

        assert!(matches!(
            sheet.filter_violation(a),
            Some(FilterViolation::Failed(_))
        ));
        // The wrong-type result is discarded: the cell's stored value is unchanged.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    }
```

- [ ] **Step 2: Run the tests to verify the first one fails**

Run: `cargo test -p adam-rs filtered_source_cell_springs_back`
Expected: FAIL — `assertion 'left == right' failed` (`left: 10, right: 50`); today's
`PlanStep::FilterReclamp` overwrites `source` in place, so `a`'s original `50` is
already gone by the second `propagate()` call.

Run: `cargo test -p adam-rs filter_reclamp_records_failed_violation_when_the_filters_function_returns_the_wrong_type`
Expected: PASS already — this test's behavior is unchanged by this task (today's
wrong-type check already produces `FilterViolation::Failed` the same way); it's added
here to close a pre-existing coverage gap (no test exercised this exact path before),
not because it's failing.

- [ ] **Step 3: Rewrite the `PlanStep::FilterReclamp` arm**

In `adam-rs/src/sheet.rs`, inside `execute_plan`, replace:

```rust
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
```

with:

```rust
                PlanStep::FilterReclamp(id) => {
                    let filter = self.cells[id]
                        .filter
                        .as_ref()
                        .expect("plan() only emits FilterReclamp for a filtered cell");
                    let args: Vec<&dyn Any> = filter
                        .args
                        .iter()
                        .map(|&a| self.cells[a].effective())
                        .collect();
                    // Self-referencing input: always `source`, never a possibly-shadowed
                    // `derived` — same rule as any other self-referencing method (see
                    // the `PlanStep::Method` arm above, and the 2026-08-02 shadow-state
                    // design). This is what keeps `source` provably untouched by the
                    // filter across any number of rounds.
                    let current = self.cells[id].source.as_ref();
                    match (filter.function)(current, &args) {
                        Ok(v) => {
                            let cell_type = self.cells[id].type_id;
                            if v.as_ref().type_id() != cell_type {
                                filter_violations.push((
                                    id,
                                    FilterViolation::Failed(anyhow::anyhow!(
                                        "filter returned a value of a different type than \
                                         the cell"
                                    )),
                                ));
                            } else {
                                // Unconditional write, no equality check — matches every
                                // other shadowed output's "no equality check" convention
                                // (2026-08-02 design). The filter's Ok output is
                                // authoritative the same way any method's output is.
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

Then update `execute_plan`'s own doc comment — replace:

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
```

with:

```rust
    /// Executes `execution_order` without invoking the planner.
    ///
    /// A `PlanStep::FilterReclamp(id)` step re-evaluates `id`'s filter against `id`'s own
    /// current `source` value (never a possibly-shadowed `derived` — the same
    /// self-referencing-input rule a `PlanStep::Method` step's self-referencing inputs
    /// follow) and its filter arguments' current effective values, writing the result
    /// into `id`'s `derived` unconditionally — `source` is never touched by this step,
    /// exactly as it's never touched by any other self-referencing method's output. A
    /// `PlanStep::Method` step's outputs follow the existing shadow/non-shadow rule,
    /// unchanged. A reclamp whose filter returns `Err`, or a value of the wrong type, is
    /// pushed into `filter_violations` instead of aborting; the cell's stored value is
    /// left untouched in that case (its `derived` stays unset, so `read()` falls back to
    /// `source`).
```

- [ ] **Step 4: Update `FilterViolation`'s doc comments**

In `adam-rs/src/filter.rs`, replace:

```rust
pub enum FilterViolation {
    /// The filter succeeded but its output differs from the cell's current value.
    NotConformed,
    /// The filter's function itself returned an error, or returned a value of a
    /// different type than the cell — both treated as an equally soft diagnostic (see
    /// the design spec §4 for why a filter's own `Err` is not a propagation-aborting
    /// failure the way a `Requirement`'s is).
    Failed(anyhow::Error),
}
```

with:

```rust
pub enum FilterViolation {
    /// The filter succeeded but its output differs from the cell's current value. Only
    /// ever recorded for a *derived* cell (`Sheet::propagate`'s post-execution
    /// diagnostic phase) — a filtered source cell's filter output is unconditionally
    /// authoritative once computed (see `PlanStep::FilterReclamp`), so this variant
    /// never applies there.
    NotConformed,
    /// The filter's function itself returned an error, or returned a value of a
    /// different type than the cell — both treated as an equally soft diagnostic (see
    /// the design spec §4 for why a filter's own `Err` is not a propagation-aborting
    /// failure the way a `Requirement`'s is). Can occur on either side: a derived
    /// cell's read-only diagnostic check, or a source cell's live reclamp, in which
    /// case the cell's stored value is left unchanged.
    Failed(anyhow::Error),
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p adam-rs --lib sheet::`
Expected: PASS, including both new tests and every pre-existing filter test in
`sheet.rs` (in particular
`propagate_reclamps_a_filtered_source_cell_when_its_argument_changes`,
`propagate_reclamps_before_a_relationship_consumes_the_reclamped_value`,
`filter_reclamp_failure_is_recorded_without_aborting_propagate_or_changing_the_cell`,
and `propagate_without_replan_reapplies_a_cached_filter_reclamp_but_does_not_touch_last_filter_violations`
— their assertions are all on `read()`/`filter_violation()` after `propagate()`, which
this change preserves).

- [ ] **Step 6: Run the full test suite, including doctests**

Run: `cargo test -p adam-rs`
Run: `cargo test --doc -p adam-rs`
Expected: PASS, no regressions.

- [ ] **Step 7: Commit**

```bash
git add adam-rs/src/sheet.rs adam-rs/src/filter.rs
git commit -m "fix(adam-rs): FilterReclamp writes to derived, not source, fixing lost raw values"
```

---

### Task 3: `write()` — delete filter special-casing

**Files:**
- Modify: `adam-rs/src/sheet.rs`

**Interfaces:**
- Modifies: `pub fn write<T: Any + 'static>(&mut self, id: CellId, value: T) ->
  Result<(), Error>` (signature unchanged; can no longer return
  `Error::MethodFailed`/a filter-derived `Error::TypeMismatch`).

- [ ] **Step 1: Delete the tests whose premise this task removes**

In `adam-rs/src/sheet.rs`'s `mod tests`, delete these four tests in their entirety
(exact bodies as they exist today, so search-and-delete each block from its `#[test]`
line through its closing `}`):

```rust
    #[test]
    fn write_returns_type_mismatch_when_the_filters_function_returns_the_wrong_type() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        // Conforms correctly for the attach-time value (5), so `add_filter` succeeds,
        // but returns a `f64` for any other input, tripping `write`'s defensive check.
        let filter = Filter::new(TypeId::of::<i32>(), vec![], vec![], |value, _args| {
            let v = *value.downcast_ref::<i32>().unwrap();
            if v == 5 {
                Ok(Box::new(v) as Box<dyn Any>)
            } else {
                Ok(Box::new(1.5_f64) as Box<dyn Any>)
            }
        });
        sheet.add_filter(a, filter).unwrap();
        let result = sheet.write(a, 99_i32);
        assert!(matches!(result, Err(Error::TypeMismatch { .. })));
        // Rejected write: cell fully untouched.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    }

    #[test]
    fn write_conforms_a_value_through_the_cells_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet.write(a, 500_i32).unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 100);
    }

    #[test]
    fn write_rejects_a_value_the_filter_cannot_conform() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_0(|x: &i32| {
                    if *x > 100 {
                        Err(anyhow::anyhow!("value exceeds maximum"))
                    } else {
                        Ok(*x)
                    }
                }),
            )
            .unwrap();
        let result = sheet.write(a, 500_i32);
        assert!(matches!(result, Err(Error::MethodFailed(_))));
        // Rejected write: cell fully untouched.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    }
```

and, a few tests later:

```rust
    #[test]
    fn write_through_a_filter_still_bumps_strength() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet.write(b, 1_i32).unwrap();
        sheet.write(a, 500_i32).unwrap();
        // `a` was written after `b`, so its strength must be higher even though its
        // stored value was conformed away from what was passed in.
        assert!(sheet.cells[a].strength > sheet.cells[b].strength);
    }
```

(Leave `write_without_a_filter_behaves_exactly_as_before`, immediately before it,
untouched.)

- [ ] **Step 2: Rewrite `from_fn_2_conforms_values_through_sheet_using_both_dynamic_arguments`**

Its current body asserts the attach-time value already conforms and that `write()`
conforms immediately, neither of which holds anymore. Replace:

```rust
    #[test]
    fn from_fn_2_conforms_values_through_sheet_using_both_dynamic_arguments() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(50_i32);
        let lo = sheet.add_cell(0_i32);
        let hi = sheet.add_cell(100_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_2([lo, hi], |x: &i32, lo: &i32, hi: &i32| {
                    Ok((*x).clamp(*lo, *hi))
                }),
            )
            .unwrap();
        // Attach-time value (50) already conforms.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 50);
        // A later write is conformed against both dynamic argument cells.
        sheet.write(a, 500_i32).unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 100);
        sheet.write(a, -10_i32).unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 0);
    }
```

with:

```rust
    #[test]
    fn from_fn_2_conforms_values_through_sheet_using_both_dynamic_arguments() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(500_i32);
        let lo = sheet.add_cell(0_i32);
        let hi = sheet.add_cell(100_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_2([lo, hi], |x: &i32, lo: &i32, hi: &i32| {
                    Ok((*x).clamp(*lo, *hi))
                }),
            )
            .unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 100);

        sheet.write(a, -10_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 0);
    }
```

- [ ] **Step 3: Add the new test for deferred conformance**

Add right after `write_without_a_filter_behaves_exactly_as_before`:

```rust
    #[test]
    fn write_leaves_the_raw_value_in_source_until_propagate_conforms_it() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet.write(a, 500_i32).unwrap();
        // write() no longer runs the filter: the raw value stands until propagate().
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 500);
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 100);
    }
```

- [ ] **Step 4: Run tests to verify the new/updated ones behave as expected**

Run: `cargo test -p adam-rs write_leaves_the_raw_value_in_source_until_propagate_conforms_it`
Expected: FAIL — today's `write()` conforms inline, so `read()` already shows `100`
immediately after `write()`, before `propagate()`; the first assertion (`500`) fails.

Run: `cargo test -p adam-rs from_fn_2_conforms_values_through_sheet_using_both_dynamic_arguments`
Expected: FAIL to compile at this point if Step 1's deletions already ran (unrelated
tests are gone) — otherwise PASS already, since `add_filter`'s retroactive conform
(untouched until Task 4) still applies the filter to the initial `500` at attach time
and every `write()` before this task's Step 5 still conforms inline too. Either
outcome is fine; Step 5 below is what this test is really validating going forward.

- [ ] **Step 5: Implement the `write()` change**

In `adam-rs/src/sheet.rs`, replace:

```rust
    pub fn write<T: Any + 'static>(&mut self, id: CellId, value: T) -> Result<(), Error> {
        if self.terminal_cells.contains(&id) {
            return Err(Error::TerminalCell);
        }
        let cell_type = self.cells.get(id).ok_or(Error::InvalidId)?.type_id;
        if cell_type != TypeId::of::<T>() {
            return Err(Error::TypeMismatch {
                expected: cell_type,
                found: TypeId::of::<T>(),
            });
        }

        let boxed: Box<dyn Any> = if let Some(filter) = self.cells[id].filter.as_ref() {
            let args: Vec<&dyn Any> = filter
                .args
                .iter()
                .map(|&a| self.cells[a].effective())
                .collect();
            let conformed = (filter.function)(&value, &args).map_err(Error::MethodFailed)?;
            if conformed.as_ref().type_id() != cell_type {
                return Err(Error::TypeMismatch {
                    expected: cell_type,
                    found: conformed.as_ref().type_id(),
                });
            }
            conformed
        } else {
            Box::new(value)
        };

        self.next_strength += 1;
        let cell = &mut self.cells[id];
        cell.strength = self.next_strength | (1u64 << 63);
        cell.source = boxed;
        cell.derived = None;
        Ok(())
    }
```

with:

```rust
    pub fn write<T: Any + 'static>(&mut self, id: CellId, value: T) -> Result<(), Error> {
        if self.terminal_cells.contains(&id) {
            return Err(Error::TerminalCell);
        }
        let cell_type = self.cells.get(id).ok_or(Error::InvalidId)?.type_id;
        if cell_type != TypeId::of::<T>() {
            return Err(Error::TypeMismatch {
                expected: cell_type,
                found: TypeId::of::<T>(),
            });
        }

        self.next_strength += 1;
        let cell = &mut self.cells[id];
        cell.strength = self.next_strength | (1u64 << 63);
        cell.source = Box::new(value);
        cell.derived = None;
        Ok(())
    }
```

Then update `write`'s doc comment — delete this bullet from its `# Errors` list:

```rust
    /// - `Error::MethodFailed` — the cell has a filter and it rejected `value`; the
    ///   cell is left completely unchanged (no strength bump, no `source` change).
```

(leaving `Error::InvalidId`, `Error::TypeMismatch`, and `Error::TerminalCell` as the
only three `# Errors` bullets).

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p adam-rs --lib sheet::`
Expected: PASS.

- [ ] **Step 7: Run the full test suite, including doctests**

Run: `cargo test -p adam-rs`
Run: `cargo test --doc -p adam-rs`
Expected: PASS, no regressions.

- [ ] **Step 8: Commit**

```bash
git add adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): write() no longer evaluates a cell's filter"
```

---

### Task 4: `add_filter` — delete the retroactive conform

**Files:**
- Modify: `adam-rs/src/sheet.rs`

**Interfaces:**
- Modifies: `pub fn add_filter(&mut self, cell: CellId, filter: Filter) -> Result<(),
  Error>` (signature unchanged; can no longer return `Error::MethodFailed` or a
  value-derived `Error::TypeMismatch`).

- [ ] **Step 1: Delete the tests whose premise this task removes**

In `adam-rs/src/sheet.rs`'s `mod tests`, delete these five tests in their entirety:

```rust
    #[test]
    fn add_filter_conforms_the_cells_current_value_immediately() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(500_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 100);
    }

    #[test]
    fn add_filter_leaves_a_conforming_value_unchanged() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    }

    #[test]
    fn add_filter_returns_method_failed_when_current_value_cannot_conform() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let result = sheet.add_filter(
            a,
            Filter::from_fn_0(|_x: &i32| Err(anyhow::anyhow!("cannot conform"))),
        );
        assert!(matches!(result, Err(Error::MethodFailed(_))));
        // Rejected: the cell's original value must survive untouched.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    }
```

and, a few tests later:

```rust
    #[test]
    fn add_filter_resolves_a_dynamic_argument_cells_current_value() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(500_i32);
        let bound = sheet.add_cell(10_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |x: &i32, bound: &i32| Ok((*x).min(*bound))),
            )
            .unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 10);
    }
```

and, right after the (already-rewritten, by Task 3) `from_fn_2_conforms_values_...` test:

```rust
    #[test]
    fn add_filter_returns_type_mismatch_when_the_filters_function_returns_the_wrong_type() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        // `value_type` matches `a`'s registered type (so add_filter's own value-type
        // check passes), but the function itself always returns a `f64`, tripping
        // add_filter's defensive check on the conformed result.
        let filter = Filter::new(TypeId::of::<i32>(), vec![], vec![], |_value, _args| {
            Ok(Box::new(1.5_f64) as Box<dyn Any>)
        });
        let result = sheet.add_filter(a, filter);
        assert!(matches!(result, Err(Error::TypeMismatch { .. })));
    }
```

- [ ] **Step 2: Add the replacement tests**

Add right after `add_filter_returns_invalid_id_for_missing_cell`:

```rust
    #[test]
    fn add_filter_does_not_change_the_cells_current_value() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(500_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        // add_filter never evaluates the function against the current value: the raw
        // out-of-range value survives until the next propagate().
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 500);
    }

    #[test]
    fn propagate_after_add_filter_conforms_the_initial_value() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(500_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 500);
        sheet.propagate().unwrap();
        // `a` has no filter args and belongs to no relationship — this is the ordinary
        // first-round case of Task 1's fix, not a special "cold start" path.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 100);
    }
```

- [ ] **Step 3: Fix the two stale test comments**

In `propagate_reports_failed_when_the_filter_errors_on_a_derived_value`, replace:

```rust
        // `add_filter` re-checks the cell's *current* value immediately (see §3.2 of
        // the design), so a filter that unconditionally errors would reject at
        // attach time (b's initial value is 0) before propagate() ever runs. Accept
        // exactly 0 so attach succeeds, and let the relationship's derived value (1,
        // copied from `a`) be the one that trips the filter.
```

with:

```rust
        // Accept exactly 0 (b's initial value) so this filter's shape is exercised
        // only by the relationship's derived value (1, copied from `a`), not by
        // anything add_filter itself does — add_filter no longer evaluates a filter's
        // function at all.
```

In `filter_reclamp_failure_is_recorded_without_aborting_propagate_or_changing_the_cell`,
replace:

```rust
        // Accept anything up to `bound` so add_filter's own immediate re-check (against
        // a's current value, 5, and bound's current value, 100) succeeds; the write to
        // `bound` below is what trips the filter.
```

with:

```rust
        // Accept anything up to `bound` (a's initial 5 is within bound's initial 100);
        // the write to `bound` below is what trips the filter's next live reclamp.
```

- [ ] **Step 4: Simplify `add_filter`'s implementation**

In `adam-rs/src/sheet.rs`, replace:

```rust
    pub fn add_filter(&mut self, cell: CellId, filter: Filter) -> Result<(), Error> {
        let cell_type = self.cells.get(cell).ok_or(Error::InvalidId)?.type_id;
        if self.terminal_cells.contains(&cell) {
            return Err(Error::TerminalCell);
        }
        if self.cells[cell].filter.is_some() {
            return Err(Error::InvalidFilter);
        }
        if filter.0.value_type != cell_type {
            return Err(Error::InvalidFilter);
        }
        if filter.0.args.contains(&cell) {
            return Err(Error::InvalidFilter);
        }
        for (&arg_id, &declared) in filter.0.args.iter().zip(filter.0.arg_types.iter()) {
            let arg_cell = self.cells.get(arg_id).ok_or(Error::InvalidId)?;
            if arg_cell.type_id != declared {
                return Err(Error::TypeMismatch {
                    expected: arg_cell.type_id,
                    found: declared,
                });
            }
        }

        let args: Vec<&dyn Any> = filter
            .0
            .args
            .iter()
            .map(|&a| self.cells[a].effective())
            .collect();
        let conformed = (filter.0.function)(self.cells[cell].source.as_ref(), &args)
            .map_err(Error::MethodFailed)?;
        if conformed.as_ref().type_id() != cell_type {
            return Err(Error::TypeMismatch {
                expected: cell_type,
                found: conformed.as_ref().type_id(),
            });
        }

        for &arg in &filter.0.args {
            self.filter_dependents.entry(arg).or_default().push(cell);
        }

        let cell_data = &mut self.cells[cell];
        cell_data.source = conformed;
        cell_data.derived = None;
        cell_data.filter = Some(filter.0);
        Ok(())
    }
```

with:

```rust
    pub fn add_filter(&mut self, cell: CellId, filter: Filter) -> Result<(), Error> {
        let cell_type = self.cells.get(cell).ok_or(Error::InvalidId)?.type_id;
        if self.terminal_cells.contains(&cell) {
            return Err(Error::TerminalCell);
        }
        if self.cells[cell].filter.is_some() {
            return Err(Error::InvalidFilter);
        }
        if filter.0.value_type != cell_type {
            return Err(Error::InvalidFilter);
        }
        if filter.0.args.contains(&cell) {
            return Err(Error::InvalidFilter);
        }
        for (&arg_id, &declared) in filter.0.args.iter().zip(filter.0.arg_types.iter()) {
            let arg_cell = self.cells.get(arg_id).ok_or(Error::InvalidId)?;
            if arg_cell.type_id != declared {
                return Err(Error::TypeMismatch {
                    expected: arg_cell.type_id,
                    found: declared,
                });
            }
        }

        for &arg in &filter.0.args {
            self.filter_dependents.entry(arg).or_default().push(cell);
        }
        self.cells[cell].filter = Some(filter.0);
        Ok(())
    }
```

Then update `add_filter`'s doc comment — replace:

```rust
    /// Attaches `filter` to `cell`.
    ///
    /// Immediately applies `filter` to `cell`'s current `source` value, exactly as
    /// [`Sheet::write`] would, so a filtered cell's value is guaranteed to conform from
    /// this call onward — not just from the next external write.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `cell`, or one of `filter`'s argument cells, is not a
    ///   live cell in this sheet.
    /// - `Error::TerminalCell` — `cell` already belongs to an existing output.
    /// - `Error::InvalidFilter` — `cell` already has a filter, `filter`'s own value
    ///   type does not match `cell`'s registered type, or `filter`'s argument list
    ///   names `cell` itself.
    /// - `Error::TypeMismatch` — an argument cell's registered type does not match the
    ///   type `filter` declared for it, or (defensively) `filter`'s function returned
    ///   a value of a different type than `cell`'s registered type.
    /// - `Error::MethodFailed` — `filter` rejected `cell`'s current value.
    ///
    /// - Complexity: O(a) where a is the number of `filter`'s argument cells.
```

with:

```rust
    /// Attaches `filter` to `cell`.
    ///
    /// Never evaluates `filter`'s function — attaching a filter is not a fresh
    /// external input, so it never changes `cell`'s current effective value. The next
    /// full [`Sheet::propagate`] call conforms `cell` via the planner's
    /// `PlanStep::FilterReclamp` step; until then, `read()` reflects whatever `cell`
    /// held before this call.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `cell`, or one of `filter`'s argument cells, is not a
    ///   live cell in this sheet.
    /// - `Error::TerminalCell` — `cell` already belongs to an existing output.
    /// - `Error::InvalidFilter` — `cell` already has a filter, `filter`'s own value
    ///   type does not match `cell`'s registered type, or `filter`'s argument list
    ///   names `cell` itself.
    /// - `Error::TypeMismatch` — an argument cell's registered type does not match the
    ///   type `filter` declared for it.
    ///
    /// - Complexity: O(a) where a is the number of `filter`'s argument cells.
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p adam-rs --lib sheet::`
Expected: PASS, including the new tests, the two comment-only fixes' unaffected
assertions, and every remaining filter test in the suite (all of §4's derived-cell
diagnostic tests, all `filter_args`/`filter_dependents`/`filter_kind`/`filter_range`
query tests, and Task 2's/Task 3's new tests).

- [ ] **Step 6: Run the full test suite, including doctests**

Run: `cargo test -p adam-rs`
Run: `cargo test --doc -p adam-rs`
Expected: PASS, no regressions.

- [ ] **Step 7: Commit**

```bash
git add adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): add_filter no longer evaluates a cell's filter"
```

---

### Task 5: Full verification sweep

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
git commit -m "chore(adam-rs): fix warnings/lints found by the full verification sweep"
```

If Step 5 made no changes, skip this step — there's nothing to commit.

- [ ] **Step 7: Manual sanity check — `begin`'s filter examples still behave**

Per the design's non-goal (`begin` is unaffected because `write_and_propagate` already
calls `propagate()`/`propagate_without_replan()` immediately after every `write()`),
this is a confirmation step, not expected to surface anything. Use the
`verifying-begin-ui` skill: open `begin/examples/inequality.adm2`, edit a filtered
cell's value directly and edit `max_v` (its filter's dynamic bound) independently, and
confirm the Inspector's number field, slider, and graph still agree in both cases —
matching the behavior already verified when the 2026-08-25 filter-revalidation plan
landed.
