# Relationship Method Constraints and Elimination-Based Plan Algorithm Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add two structural validity constraints to `adam-rs` relationships (matching cell sets across methods, unique output sets) and replace the planner's eligibility/pre-claiming machinery with a single elimination-based selection algorithm plus an explicit topological sort for execution order.

**Architecture:** Two new `Error` variants back new validation in `Sheet::add_relationship`. `planner.rs`'s `plan()` function is rewritten around one primitive — narrow a relationship's candidate methods by output-cell-set membership until one remains — bootstrapped for structurally-forced relationships and driven by strength order for everything else, followed by a topological sort of the final assignment to produce `execution_order`.

**Tech Stack:** Rust, `slotmap`, existing `adam-rs` crate conventions (contract-style doc comments, `.op1r`-style fallible ops not applicable here).

## Global Constraints

- `cargo fmt --all` must be run before every commit (enforced by pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`, `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`, and `cargo clippy -p begin --all-targets -- -D warnings` must all pass before opening a PR.
- Every public/private function needs a contract-style `///` doc comment (Summary, Preconditions as `debug_assert!`-backed bullets, `# Errors`, Postconditions, Complexity when not O(1)) per `CLAUDE.md`.
- Never commit directly to `main`; this work happens on the existing `worktree-improve-plan-algorithm` worktree/branch.
- Design spec: `docs/superpowers/specs/2026-08-06-relationship-method-constraints-and-plan-algorithm-design.md` — follow it for the *why*; this plan is the *how*.

---

### Task 1: Add `Error::MismatchedMethodCells` and `Error::DuplicateMethodOutputs`

**Files:**
- Modify: `adam-rs/src/error.rs`

**Interfaces:**
- Produces: two new `adam_rs::Error` variants, used by Task 2's validation and returned from `Sheet::add_relationship`.

- [ ] **Step 1: Add the two variants to the `Error` enum**

In `adam-rs/src/error.rs`, add these variants after `InvalidMethod` (keep `#[non_exhaustive]` on the enum as-is):

```rust
    /// Two methods in the same relationship have `inputs ∪ outputs` sets that don't
    /// match. Every method in a relationship must reference exactly the same set of
    /// cells.
    MismatchedMethodCells,

    /// A method's own `outputs` list names a cell more than once, or two methods in
    /// the same relationship have identical `outputs` sets.
    DuplicateMethodOutputs,
```

- [ ] **Step 2: Add `Display` arms**

In the same file's `impl std::fmt::Display for Error`, add arms alongside the existing ones:

```rust
            Error::MismatchedMethodCells => write!(
                f,
                "methods in a relationship must reference the same set of cells"
            ),
            Error::DuplicateMethodOutputs => write!(
                f,
                "a method's outputs must be duplicate-free, and no two methods in a \
                 relationship may share an outputs set"
            ),
```

- [ ] **Step 3: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `adam-rs/src/error.rs`:

```rust
    #[test]
    fn mismatched_method_cells_display_contains_cells() {
        assert!(Error::MismatchedMethodCells.to_string().contains("cells"));
    }

    #[test]
    fn duplicate_method_outputs_display_contains_outputs() {
        assert!(
            Error::DuplicateMethodOutputs
                .to_string()
                .contains("outputs")
        );
    }
```

Also extend the existing `non_method_failed_variants_have_no_source` test with two more assertions, so every non-`MethodFailed` variant is explicitly covered (matching this test's existing style):

```rust
        assert!(std::error::Error::source(&Error::MismatchedMethodCells).is_none());
        assert!(std::error::Error::source(&Error::DuplicateMethodOutputs).is_none());
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p adam-rs --lib error::tests`
Expected: all pass, including the two new tests.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/error.rs
git commit -m "feat(adam-rs): add MismatchedMethodCells and DuplicateMethodOutputs errors"
```

---

### Task 2: Validate matching cell sets and unique outputs in `add_relationship`

**Files:**
- Modify: `adam-rs/src/sheet.rs:117-180` (`add_relationship`), and its test module (starting ~line 919)
- Modify: `adam-rs/src/relationship.rs:113-118` (`RelationshipData::adj` doc comment)
- Modify: `adam-rs/src/planner.rs` (remove one now-invalid test, in its `#[cfg(test)] mod tests` block)

**Interfaces:**
- Consumes: `Error::MismatchedMethodCells`, `Error::DuplicateMethodOutputs` from Task 1.
- Produces: `Sheet::add_relationship` now rejects relationships violating either constraint — Task 3's planner rewrite relies on both invariants holding for every relationship it plans.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `adam-rs/src/sheet.rs` (near the other `add_relationship_*` tests, e.g. after `add_relationship_empty_outputs_returns_invalid_method`):

```rust
    #[test]
    fn add_relationship_mismatched_cells_returns_error() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let d = sheet.add_cell(0_i32);
        // Method 0 spans {a, b}; Method 1 spans {c, d} — mismatched cell sets.
        let result = sheet.add_relationship(vec![
            Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
            Method::from_fn_1_1(c, d, |x: &i32| Ok(*x)),
        ]);
        assert!(matches!(result, Err(Error::MismatchedMethodCells)));
    }

    #[test]
    fn add_relationship_duplicate_output_sets_across_methods_returns_error() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        // Both methods span {a, b, c} and both output {c} — identical output sets.
        let result = sheet.add_relationship(vec![
            Method::from_fn_2_1([a, b], c, |x: &i32, y: &i32| Ok(*x + *y)),
            Method::from_fn_2_1([a, b], c, |x: &i32, y: &i32| Ok(*x - *y)),
        ]);
        assert!(matches!(result, Err(Error::DuplicateMethodOutputs)));
    }

    #[test]
    fn add_relationship_duplicate_cell_within_own_outputs_returns_error() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        // The method's own outputs list names `b` twice.
        let method = Method::new(
            vec![a],
            vec![b, b],
            vec![TypeId::of::<i32>()],
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
            |args| {
                let x = args[0].downcast_ref::<i32>().unwrap();
                Ok(vec![Box::new(*x), Box::new(*x)])
            },
        );
        let result = sheet.add_relationship(vec![method]);
        assert!(matches!(result, Err(Error::DuplicateMethodOutputs)));
    }
```

`TypeId` is already imported at the top of this test module (`use std::any::TypeId;`).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-rs --lib sheet::tests::add_relationship_mismatched_cells_returns_error sheet::tests::add_relationship_duplicate_output_sets_across_methods_returns_error sheet::tests::add_relationship_duplicate_cell_within_own_outputs_returns_error`
Expected: all three FAIL (currently `add_relationship` returns `Ok`, so `matches!(Ok(_), Err(...))` is false).

- [ ] **Step 3: Implement the validation**

In `adam-rs/src/sheet.rs`, inside `add_relationship`, insert this block after the existing per-method validation loop (after the closing `}` of the `for method in &methods` loop that checks empty inputs/outputs and type mismatches, i.e. right before the `// Collect the union of all adjacent cells...` comment):

```rust
        // Every method in a relationship must reference the same combined set of cells.
        let cell_sets: Vec<HashSet<CellId>> = methods
            .iter()
            .map(|m| m.inputs.iter().chain(m.outputs.iter()).copied().collect())
            .collect();
        if cell_sets[1..].iter().any(|set| set != &cell_sets[0]) {
            return Err(Error::MismatchedMethodCells);
        }

        // A method's own outputs must be duplicate-free, and no two methods may share
        // an outputs set.
        let mut seen_output_sets: Vec<HashSet<CellId>> = Vec::with_capacity(methods.len());
        for method in &methods {
            let output_set: HashSet<CellId> = method.outputs.iter().copied().collect();
            if output_set.len() != method.outputs.len() || seen_output_sets.contains(&output_set)
            {
                return Err(Error::DuplicateMethodOutputs);
            }
            seen_output_sets.push(output_set);
        }
```

`HashSet` and `CellId` are already imported in this file.

- [ ] **Step 4: Update `add_relationship`'s doc comment**

In `adam-rs/src/sheet.rs`, `add_relationship`'s doc comment currently lists errors as:

```rust
    /// - `Error::InvalidMethod` — `methods` is empty, a method has no inputs,
    ///   or a method has no outputs.
    /// - `Error::InvalidId` — a `CellId` in any method is not found in this sheet.
    /// - `Error::TypeMismatch` — a method's declared `TypeId` does not match the
    ///   cell's registered `TypeId`.
```

Add two more lines after the `InvalidMethod` line:

```rust
    /// - `Error::MismatchedMethodCells` — some method's `inputs ∪ outputs` differs
    ///   from another method's in the same relationship.
    /// - `Error::DuplicateMethodOutputs` — a method's own `outputs` list names a cell
    ///   more than once, or two methods in the same relationship have identical
    ///   `outputs` sets.
```

- [ ] **Step 5: Update `RelationshipData::adj`'s doc comment**

In `adam-rs/src/relationship.rs`, change:

```rust
    /// Union of all cell IDs referenced by any method in this relationship (union across all methods).
    pub(crate) adj: Vec<CellId>,
```

to:

```rust
    /// The set of cells referenced by every method in this relationship — every method
    /// references the same set (enforced by `Sheet::add_relationship`).
    pub(crate) adj: Vec<CellId>,
```

- [ ] **Step 6: Run the new tests to verify they pass**

Run: `cargo test -p adam-rs --lib sheet::tests::add_relationship_mismatched_cells_returns_error sheet::tests::add_relationship_duplicate_output_sets_across_methods_returns_error sheet::tests::add_relationship_duplicate_cell_within_own_outputs_returns_error`
Expected: all three PASS.

- [ ] **Step 7: Remove the now-invalid `dead_method_not_selected_before_owning_relationship` test**

This test (in `adam-rs/src/planner.rs`'s `#[cfg(test)] mod tests`) builds a relationship with three methods spanning three disjoint cell sets (`{x,b}`, `{y,c}`, `{b,c,d}`), which the new matching-cell-set constraint makes impossible to construct — its `add_relationship(...).unwrap()` call will now panic since `add_relationship` returns `Err(Error::MismatchedMethodCells)`. Delete the entire test function (including its doc comment), from:

```rust
    #[test]
    fn dead_method_not_selected_before_owning_relationship() {
```

through its closing `}`, i.e. the whole test including the preceding comment block starting at "R_A: p -> b (single method, forces b)." Do not replace it with anything — the scenario it exercised (excluding a dead method from selection) is superseded by Task 3's elimination-based algorithm, which handles cross-relationship dead-method exclusion structurally rather than via the flood-fill's explicit `alive` check this test was targeting.

- [ ] **Step 8: Run the full adam-rs test suite**

Run: `cargo test -p adam-rs`
Expected: all pass (the deleted test no longer runs; no other test references it).

- [ ] **Step 9: Commit**

```bash
git add adam-rs/src/error.rs adam-rs/src/sheet.rs adam-rs/src/relationship.rs adam-rs/src/planner.rs
git commit -m "feat(adam-rs): validate matching cell sets and unique outputs in add_relationship"
```

---

### Task 3: Rewrite the planner's method-selection algorithm

**Files:**
- Modify: `adam-rs/src/planner.rs` (module doc comment, `plan()`, remove `is_eligible`/pre-claiming logic, add `select_if_sole_candidate`, `drain`, `topological_order`; keep `Plan`, `pure_outputs`, `forced_output_cells` as-is)
- Modify: `adam-rs/tests/integration.rs:405-422` (`mutually_dependent_relationships_return_conflict`)

**Interfaces:**
- Consumes: `RelationshipData`/`Method` from `crate::relationship` (unchanged), `CellData`/`CellId` from `crate::cell` (unchanged), `Error::Conflict` and `Error::Cycle` (both already exist in `error.rs`; `Cycle` was previously defined but unused).
- Produces: `pub(crate) fn plan(...) -> Result<Plan, Error>` keeps its existing signature and the `Plan` struct's existing fields (`execution_order: Vec<(RelationshipId, usize)>`, `forced_outputs: HashSet<CellId>`, `forced_relationships: HashSet<RelationshipId>`) — no caller in `sheet.rs` needs to change.

This task replaces `is_eligible`, `is_feasible`, `pre_claimed`, and `source_cells` (the flood-fill machinery inside the old `plan()`) with three new functions plus a rewritten `plan()` body. `pure_outputs` and `forced_output_cells` are unchanged — they still compute the strength-independent forced-cell/forced-relationship information that `Sheet::is_forced`/`is_relationship_forced` depend on.

- [ ] **Step 1: Update the module doc comment**

Replace `adam-rs/src/planner.rs`'s module doc comment (the `//!` block at the top of the file, lines 1–39) with:

```rust
//! Planning pass: selects one method per relationship and returns them in dependency order.
//!
//! Implements the Adam algorithm: cells are visited in descending strength
//! (write-recency) order. The first time a cell is visited it becomes a *source* — its
//! current value is taken as given. A relationship's methods are *candidates* for
//! selection; whenever a cell becomes determined (as a source, or as the output of some
//! other relationship's already-selected method), every relationship adjacent to that
//! cell eliminates any candidate whose `outputs` set contains it. The instant a
//! relationship's candidates narrow to exactly one, that method is selected and each of
//! its output cells becomes determined too, cascading the same elimination outward. A
//! relationship whose candidates narrow to zero cannot be assigned — see
//! [`Error::Conflict`].
//!
//! Because every method's `outputs` set is unique within its relationship (enforced by
//! [`crate::sheet::Sheet::add_relationship`]), this single mechanism handles
//! self-referencing methods (a cell in both a method's `inputs` and `outputs`) without
//! special-casing: whichever of two candidate cells resolves first eliminates exactly
//! the method that would have produced it, leaving the other as sole survivor.
//!
//! **Structurally forced cells**: some cells are guaranteed to be produced by a method
//! regardless of cell strength — e.g. the sole output of a single-method relationship.
//! [`forced_output_cells`] computes this set as a fixpoint over all active
//! relationships, independent of any specific run's cell strengths (needed because
//! [`crate::sheet::Sheet::is_forced`] must answer "can this cell ever meaningfully be
//! written?" regardless of what a caller might write). Relationships already narrowed to
//! one candidate by this fixpoint are selected immediately, before any cell is chosen as
//! a fresh source; every other structurally forced cell is excluded from source
//! candidacy in the strength-ordered pass.
//!
//! **Execution order**: because a relationship can be selected before its inputs are
//! actually resolved (a structurally forced single-method relationship is selected
//! immediately, regardless of when its input arrives), the order relationships are
//! *selected* in is not necessarily a valid execution order. [`topological_order`]
//! computes one separately from the final selection, and reports [`Error::Cycle`] if the
//! selected methods have no valid order (e.g. two single-method relationships that each
//! require the other's output).
```

- [ ] **Step 2: Replace the `plan()` function and its helper closures**

Leave the `use` statements just below the module doc comment untouched (`std::cmp::Reverse`, `std::collections::{HashMap, HashSet, VecDeque}`, `slotmap::SlotMap`, and the `crate::{...}` import — all of them are still needed and nothing new needs importing). Starting from the blank line after those `use` statements, replace everything through the end of the old `plan()` function's closing `}` (i.e. the `Plan` struct definition and the old `plan()` function together, but stopping before `fn pure_outputs`) with:

```rust
/// The output of the planning pass.
pub(crate) struct Plan {
    /// Selected `(RelationshipId, method_index)` pairs in execution order.
    pub(crate) execution_order: Vec<(RelationshipId, usize)>,
    /// Cells that can never be a source under the relationships this plan considered.
    /// See [`forced_output_cells`].
    pub(crate) forced_outputs: HashSet<CellId>,
    /// Active relationships with exactly one alive method after the forced-output
    /// fixpoint (see [`forced_output_cells`]) — the planner has no alternative method
    /// to choose for these, regardless of cell strength.
    pub(crate) forced_relationships: HashSet<RelationshipId>,
}

/// Assigns one method per active relationship and returns them in dependency order.
///
/// Only relationships in `active` are planned; relationships outside `active` are
/// invisible to the elimination process. The conflict check counts against
/// `active.len()`.
///
/// A method may have cells in both `inputs` and `outputs` (self-referencing); see the
/// module documentation for why this needs no special handling here. Such a cell is
/// read at its pre-execution value and overwritten with the result.
///
/// # Errors
///
/// - `Error::Conflict` — some active relationship's candidates narrowed to zero, or not
///   every active relationship could be assigned a method.
/// - `Error::Cycle` — the selected methods have no valid execution order.
///
/// - Complexity: O(D · R · M · K²) for [`forced_output_cells`] (D = methods eliminated
///   across its fixpoint), plus O(C log C) to sort cells by strength, plus O(R·M·K) for
///   the elimination pass (each cell triggers one elimination scan per adjacent
///   relationship), plus O(R·K) for the final topological sort. C = cells, R = active
///   relationships, M = methods per relationship, K = cells per method.
pub(crate) fn plan(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> Result<Plan, Error> {
    let (forced_outputs, structural_alive) = forced_output_cells(relationships, active);
    let forced_relationships: HashSet<RelationshipId> = structural_alive
        .iter()
        .filter(|(_, methods)| methods.iter().filter(|&&is_alive| is_alive).count() == 1)
        .map(|(&rel_id, _)| rel_id)
        .collect();

    let mut candidates = structural_alive;
    let mut determined: HashSet<CellId> = HashSet::new();
    let mut selected: HashMap<RelationshipId, usize> = HashMap::new();
    let mut queue: VecDeque<CellId> = VecDeque::new();

    // Bootstrap: relationships already down to one candidate — either genuinely
    // single-method, or narrowed by the forced-output fixpoint above — are selected
    // immediately, before any cell is chosen as a fresh source.
    for &rel_id in active {
        select_if_sole_candidate(
            rel_id,
            relationships,
            &mut candidates,
            &mut determined,
            &mut selected,
            &mut queue,
        )?;
        drain(
            &mut queue,
            cells,
            relationships,
            active,
            &mut candidates,
            &mut determined,
            &mut selected,
        )?;
    }

    // Strength-ordered seeding: the highest-strength cell not already determined or
    // structurally forced becomes a fresh source.
    let mut cells_sorted: Vec<CellId> = cells.keys().collect();
    cells_sorted.sort_by_key(|&id| Reverse(cells[id].strength));
    for &cell in &cells_sorted {
        if determined.contains(&cell) || forced_outputs.contains(&cell) {
            continue;
        }
        determined.insert(cell);
        queue.push_back(cell);
        drain(
            &mut queue,
            cells,
            relationships,
            active,
            &mut candidates,
            &mut determined,
            &mut selected,
        )?;
    }

    if selected.len() != active.len() {
        return Err(Error::Conflict);
    }

    let execution_order = topological_order(relationships, &selected)?;

    Ok(Plan {
        execution_order,
        forced_outputs,
        forced_relationships,
    })
}

/// Selects `rel_id`'s sole surviving candidate method, if exactly one remains.
///
/// Does nothing if `rel_id` is already selected or if more than one candidate remains.
/// Used both to bootstrap relationships that start with (or are narrowed by
/// [`forced_output_cells`] to) a single candidate, and after each candidate
/// elimination in [`drain`].
///
/// - Postcondition: if selected, each output cell not already in `determined` is
///   inserted into `determined` and pushed onto `queue` for cascading elimination.
///
/// # Errors
///
/// - `Error::Conflict` — `rel_id` has zero surviving candidates.
fn select_if_sole_candidate(
    rel_id: RelationshipId,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    candidates: &mut HashMap<RelationshipId, Vec<bool>>,
    determined: &mut HashSet<CellId>,
    selected: &mut HashMap<RelationshipId, usize>,
    queue: &mut VecDeque<CellId>,
) -> Result<(), Error> {
    if selected.contains_key(&rel_id) {
        return Ok(());
    }
    let alive = &candidates[&rel_id];
    let mut survivors = alive.iter().enumerate().filter(|&(_, &is_alive)| is_alive);
    match (survivors.next(), survivors.next()) {
        (None, _) => Err(Error::Conflict),
        (Some(_), Some(_)) => Ok(()),
        (Some((idx, _)), None) => {
            selected.insert(rel_id, idx);
            for &output in &relationships[rel_id].methods[idx].outputs {
                if determined.insert(output) {
                    queue.push_back(output);
                }
            }
            Ok(())
        }
    }
}

/// Processes `queue`, eliminating candidates in every relationship adjacent to each
/// dequeued cell and selecting any relationship whose candidates narrow to one.
///
/// - Precondition: every cell in `queue` is already present in `determined`.
///
/// # Errors
///
/// - `Error::Conflict` — some relationship's candidates narrow to zero.
fn drain(
    queue: &mut VecDeque<CellId>,
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
    candidates: &mut HashMap<RelationshipId, Vec<bool>>,
    determined: &mut HashSet<CellId>,
    selected: &mut HashMap<RelationshipId, usize>,
) -> Result<(), Error> {
    while let Some(cell) = queue.pop_front() {
        for &rel_id in &cells[cell].adj {
            if !active.contains(&rel_id) || selected.contains_key(&rel_id) {
                continue;
            }
            {
                let alive = candidates
                    .get_mut(&rel_id)
                    .expect("seeded for every active id");
                for (idx, method) in relationships[rel_id].methods.iter().enumerate() {
                    if alive[idx] && method.outputs.contains(&cell) {
                        alive[idx] = false;
                    }
                }
            }
            select_if_sole_candidate(rel_id, relationships, candidates, determined, selected, queue)?;
        }
    }
    Ok(())
}

/// Orders `selected` so each method appears after the methods producing its pure
/// inputs (inputs not also present in its own outputs).
///
/// - Precondition: every cell produced by an entry of `selected` is produced by at
///   most one entry — guaranteed by the elimination in [`plan`], since a cell is
///   inserted into `determined` (and thus becomes an output of exactly one selected
///   method) at most once.
///
/// # Errors
///
/// - `Error::Cycle` — the dependency graph over `selected` contains a cycle.
///
/// - Complexity: O(R·K) where R = `selected.len()` and K = max cells per method.
fn topological_order(
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    selected: &HashMap<RelationshipId, usize>,
) -> Result<Vec<(RelationshipId, usize)>, Error> {
    let producer: HashMap<CellId, RelationshipId> = selected
        .iter()
        .flat_map(|(&rel_id, &idx)| {
            relationships[rel_id].methods[idx]
                .outputs
                .iter()
                .map(move |&output| (output, rel_id))
        })
        .collect();

    let mut dependents: HashMap<RelationshipId, Vec<RelationshipId>> =
        selected.keys().map(|&rel_id| (rel_id, Vec::new())).collect();
    let mut in_degree: HashMap<RelationshipId, usize> =
        selected.keys().map(|&rel_id| (rel_id, 0)).collect();

    for (&rel_id, &idx) in selected {
        let method = &relationships[rel_id].methods[idx];
        for input in method.inputs.iter().filter(|i| !method.outputs.contains(i)) {
            if let Some(&producer_rel) = producer.get(input) {
                dependents
                    .get_mut(&producer_rel)
                    .expect("seeded for every relationship in `selected`")
                    .push(rel_id);
                *in_degree
                    .get_mut(&rel_id)
                    .expect("seeded for every relationship in `selected`") += 1;
            }
        }
    }

    let mut ready: VecDeque<RelationshipId> = in_degree
        .iter()
        .filter(|&(_, &degree)| degree == 0)
        .map(|(&rel_id, _)| rel_id)
        .collect();
    let mut order = Vec::with_capacity(selected.len());
    while let Some(rel_id) = ready.pop_front() {
        order.push((rel_id, selected[&rel_id]));
        for &dependent in &dependents[&rel_id] {
            let degree = in_degree.get_mut(&dependent).expect("seeded above");
            *degree -= 1;
            if *degree == 0 {
                ready.push_back(dependent);
            }
        }
    }

    if order.len() != selected.len() {
        return Err(Error::Cycle);
    }
    Ok(order)
}
```

Leave `pure_outputs` and `forced_output_cells` (the functions immediately following, currently at old lines 258–347) completely unchanged — do not edit them.

- [ ] **Step 3: Update `mutually_dependent_relationships_return_conflict` to expect `Error::Cycle`**

In `adam-rs/tests/integration.rs`, replace:

```rust
#[test]
fn mutually_dependent_relationships_return_conflict() {
    // a→b and b→a: Adam marks a as a source, flows to b via the first
    // relationship, then the second relationship's only method (b→a) cannot
    // fire because a is already determined. The second relationship is left
    // unassigned, which is reported as a Conflict.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();

    assert!(matches!(sheet.propagate(), Err(Error::Conflict)));
}
```

with:

```rust
#[test]
fn mutually_dependent_relationships_return_cycle() {
    // a→b and b→a: both are single-method relationships, so each is trivially
    // selected (one candidate each) regardless of cell strength. Neither method's
    // input is ever produced by anything outside this pair, so there is no valid
    // execution order between them — a genuine cycle, not an under-constrained plan.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();

    assert!(matches!(sheet.propagate(), Err(Error::Cycle)));
}
```

- [ ] **Step 4: Run the full adam-rs test and doc-test suite**

Run: `cargo test -p adam-rs && cargo test --doc -p adam-rs`
Expected: all pass. If any test other than the one updated in Step 3 fails, re-read the module doc comment's stated invariants against that specific test's setup before changing algorithm code — the design spec (`docs/superpowers/specs/2026-08-06-relationship-method-constraints-and-plan-algorithm-design.md`) verified every other existing test already satisfies the new constraints and should pass unchanged.

- [ ] **Step 5: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: all pass — `adam-lang` and `begin` consume `adam-rs::Sheet`/`Method`/`Error` but don't match on `Error` exhaustively (`begin/src/bridge.rs`'s only match has a wildcard arm), so no other crate needs code changes.

- [ ] **Step 6: Commit**

```bash
git add adam-rs/src/planner.rs adam-rs/tests/integration.rs
git commit -m "feat(adam-rs): rewrite planner selection as output-set elimination with topological execution order"
```

---

### Task 4: Full workspace verification before PR

**Files:** none (verification only)

**Interfaces:** none

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Expected: no diff (or stages any formatting fixes).

- [ ] **Step 2: Build with zero warnings**

Run: `cargo build --workspace`
Expected: succeeds with no compiler warnings. Fix any that appear (e.g. unused imports left over from removing `is_eligible`/`is_feasible`/`pre_claimed`/`source_cells`).

- [ ] **Step 3: Test with zero warnings**

Run: `cargo test --workspace && cargo test --doc --workspace`
Expected: all pass, no compiler warnings in test builds.

- [ ] **Step 4: Lint the main workspace**

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Expected: no warnings. Fix any lints raised by the new `planner.rs` code (e.g. needless collect, redundant clones).

- [ ] **Step 5: Lint `begin` without default features**

Run: `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Lint `begin` with default features**

Run: `cargo clippy -p begin --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit any fixes**

If any of the above steps required code changes:

```bash
git add -A
git commit -m "fix(adam-rs): address warnings from full workspace check"
```

If no changes were needed, skip this step — do not create an empty commit.
