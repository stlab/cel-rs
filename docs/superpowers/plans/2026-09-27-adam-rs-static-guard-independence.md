# adam-rs Static Guard Independence (Phase A) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reject, at construction time, any filter or conditional whose inputs depend on a cell it governs, and remove the now-unreachable plan-time `Error::FilterCycle`.

**Architecture:** A new private module `adam-rs/src/sheet/dependency.rs` adds `impl Sheet` helpers that BFS the *static dependency graph* (method `input → output` edges, filter guard `arg → filtered` edges, conditional guard `match cell → governed output` edges) and find a guard edge `g → t` for which `t` reaches `g`. `add_filter`, `add_conditional`, and `add_relationship` each tentatively apply their mutation, call `guard_violation()`, and roll back and return the new `Error::DependencyCycle { sites }` on violation. The planner's post-filter-edge cycle branch becomes an invariant `expect`.

**Tech Stack:** Rust 2024, `slotmap`, workspace crates `adam-rs`, `adam-lang`, `begin`.

> **Superseded after execution (PR #238 review):** the per-mutator check made sheet construction
> superlinear, so it was replaced by a lazy, linear check. Mutators now only mark the structure
> unvalidated; `Sheet::validate` (run first by `propagate`, and once by adam-lang after parsing)
> finds guard cycles with a single SCC pass in O(V + E). Nothing is rolled back, and every method hop
> is reported as `Relationship`. The spec §2 and the Phase A handoff describe the final design; the
> tasks below record the original execution.

**Spec:** `docs/superpowers/specs/2026-09-27-adam-rs-planner-generalization-design.md` (§2 and "Validity rules"). Phases B (§1 seedfill, #186) and C (§3 plan reuse, #152) get their own plans after this phase merges.

## Global Constraints

- Work in the existing worktree `.claude/worktrees/adam-rs/issue-18` on branch `worktree-adam-rs/issue-18`; never commit to `main`.
- `cargo fmt --all` before every commit.
- Every new/changed function has a contract-style `///` doc (present-tense summary ending in a period; `- Precondition:` / `- Postcondition:` / `# Errors` / `- Complexity:` as applicable). Modules use `//!`.
- Tests derive from public contracts only; each `# Errors` condition and postcondition gets a test.
- Avoid avoidable heap allocation; pass slices/`&HashSet`, borrow rather than clone.
- Before the PR, all of these must pass with **zero warnings** in build/test output:
  - `cargo build --workspace`
  - `cargo test --workspace`
  - `cargo test --doc --workspace`
  - `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
  - `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
  - `cargo clippy -p begin --all-targets -- -D warnings`
  - `$env:RUSTDOCFLAGS="-D warnings"; cargo doc --lib --no-deps --workspace`
  - `cargo test -p begin --no-default-features` (includes `every_bundled_example_parses_successfully`)
- Commit messages end with `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`.

## File Structure

- Create `adam-rs/src/sheet/dependency.rs` — static dependency graph traversal (`DependencyPath`, `Sheet::dependency_path`, `Sheet::guard_violation`). One responsibility: answering "does any guard close a cycle?".
- Modify `adam-rs/src/sheet.rs` — declare `mod dependency;`; call the check from `add_filter`, `add_conditional`, `add_relationship` with rollback; delete `add_conditional`'s contributing-cells rule; update docs and existing tests.
- Modify `adam-rs/src/error.rs` — replace `FilterCycle` with `DependencyCycle`; update `InvalidConditional` doc.
- Modify `adam-rs/src/planner.rs`, `adam-rs/src/planner/release.rs`, `adam-rs/src/lib.rs` — remove `FilterCycle` path/docs and #153 references.
- Create `adam-rs/tests/dependency_guards.rs` — contract tests via the public API.
- Modify `adam-lang/src/error_labels.rs`, `adam-lang/src/parser.rs` — labels for `DependencyCycle`; secondary labels for `add_filter` errors; test comment updates.
- Create `docs/superpowers/2026-09-27-planner-generalization-phase-a-handoff.md`.

---

### Task 1: `Error::DependencyCycle` replaces `Error::FilterCycle`

**Files:**
- Modify: `adam-rs/src/error.rs` (variant ~line 146-160, `Display` ~192, `sites()` ~227, tests ~441-449)
- Modify: `adam-rs/src/planner.rs:130-146` and tests `a_filter_argument_cycle_returns_filter_cycle_error`, `filter_cycle_error_names_its_members` (~line 694-745)
- Modify: `adam-rs/src/planner/release.rs:15-22` (module doc), `adam-rs/src/lib.rs:123`
- Modify: `adam-lang/src/error_labels.rs:27`

**Interfaces:**
- Produces: `adam_rs::Error::DependencyCycle { sites: Vec<ErrorSite> }` (listed in `Error::sites()`).

- [x] **Step 1: Update the error unit tests to the new variant (failing)**

In `adam-rs/src/error.rs` tests, rename `filter_cycle_display_contains_cycle` / `filter_cycle_has_no_source` to `dependency_cycle_display_contains_cycle` / `dependency_cycle_has_no_source` and replace `Error::FilterCycle { sites: vec![] }` with `Error::DependencyCycle { sites: vec![] }` in both. Add:

```rust
    #[test]
    fn dependency_cycle_exposes_its_sites() {
        let site = ErrorSite::Cell(CellId::default());
        let e = Error::DependencyCycle { sites: vec![site] };
        assert_eq!(e.sites(), &[site]);
    }
```

- [x] **Step 2: Run to verify failure**

Run: `cargo test -p adam-rs --lib error::tests`
Expected: compile error `no variant named DependencyCycle`.

- [x] **Step 3: Replace the variant**

Replace the `FilterCycle` variant and its doc with:

```rust
    /// A filter's argument, or a conditional's match subject, depends on a cell that
    /// filter or conditional governs, in the sheet's static dependency graph: every
    /// method's `input → output` edges (a self-referencing input adds none), each
    /// filter's `argument → filtered` guard edges, and each conditional's
    /// `match cell → output` guard edges for every method output of its branch and
    /// default relationships. Returned by `Sheet::add_filter`, `Sheet::add_conditional`,
    /// and `Sheet::add_relationship`, which leave the sheet unchanged when they return it.
    DependencyCycle {
        /// The cycle in dependency order, starting at the governed cell `t` and ending at
        /// the guard cell `g` whose guard edge `g → t` closes it. `Cell` entries are
        /// separated by the `Relationship` each method hop runs through — or, for a
        /// relationship still being added by `add_relationship`, that method's
        /// `MethodIndex`. Two consecutive `Cell` entries are a guard hop.
        sites: Vec<ErrorSite>,
    },
```

`Display` arm:

```rust
            Error::DependencyCycle { .. } => write!(
                f,
                "a filter or conditional depends on a cell it governs (dependency cycle)"
            ),
```

In `sites()`, replace `Error::FilterCycle { sites }` with `Error::DependencyCycle { sites }`.

- [x] **Step 4: Make the planner's filter-edge sort an invariant**

In `adam-rs/src/planner.rs` `plan()`, replace the `match topological_order(&adj) { ... None => { ... FilterCycle ... } }` block with:

```rust
    let order = topological_order(&adj).expect(
        "release::resolve returns an acyclic relationship assignment, and \
         Sheet's static guard check rules out any cycle through a filter edge",
    );
```

Update the comment above the `method_count` check to say the order contains every active relationship because the sort cannot fail. Remove imports that become unused (e.g. `trace::recover_cycle`/`node_to_site` only if no longer referenced — let clippy tell you). Delete the two planner tests `a_filter_argument_cycle_returns_filter_cycle_error` and `filter_cycle_error_names_its_members` (their sheets are now rejected at `add_filter`; Task 3 covers that contract). Update the `plan()` doc if it mentions `FilterCycle`.

In `adam-rs/src/planner/release.rs` module doc, replace the sentences stating that plan() may return `Error::FilterCycle` and that filter-aware search is tracked as #153 with:

```rust
//! Filter edges never need to be considered here: `Sheet` rejects, at construction,
//! any filter whose arguments depend on the filtered cell (`Error::DependencyCycle`),
//! so adding a filtered source's argument edges to an acyclic relationship assignment
//! can never close a cycle.
```

In `adam-rs/src/lib.rs:123`, change ``([`Error::Cycle`]/[`Error::FilterCycle`] when not)`` to ``([`Error::Cycle`] when not)``.

In `adam-lang/src/error_labels.rs:27`, change `Error::Cycle { .. } | Error::FilterCycle { .. }` to `Error::Cycle { .. } | Error::DependencyCycle { .. }`.

- [x] **Step 5: Run tests**

Run: `cargo test -p adam-rs --lib; cargo test -p adam-lang --lib`
Expected: PASS. (No production path returns `DependencyCycle` yet; that is Tasks 2–5.)

- [x] **Step 6: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "refactor(adam-rs): replace plan-time FilterCycle with DependencyCycle variant"
```

---

### Task 2: Static dependency graph traversal

**Files:**
- Create: `adam-rs/src/sheet/dependency.rs`
- Modify: `adam-rs/src/sheet.rs` (add `mod dependency;` near the top-level `use` block)

**Interfaces:**
- Consumes: `Sheet` fields `cells`, `relationships`, `conditionals`, `filter_dependents` (`HashMap<CellId, Vec<CellId>>`: arg → filtered cells); `ConditionalData::match_cells()`, `ConditionalData::{branches, default}`, `Branch::relationships`.
- Produces (all `pub(crate)` or private to `sheet`):
  - `pub(super) struct DependencyPath { cells: Vec<CellId>, hops: Vec<Hop> }` with `cells.len() == hops.len() + 1`.
  - `pub(super) enum Hop { Method { relationship: RelationshipId, method: usize }, Guard }`.
  - `impl DependencyPath { pub(super) fn into_sites(self, pending: Option<RelationshipId>) -> Vec<ErrorSite> }`.
  - `impl Sheet { pub(super) fn guard_violation(&self) -> Option<DependencyPath> }`.

- [x] **Step 1: Write the module with unit tests first (tests fail to compile until implemented)**

Create `adam-rs/src/sheet/dependency.rs`:

```rust
//! Static dependency-graph checks backing `Sheet`'s guard-independence invariant.
//!
//! The static dependency graph has one node per cell and three kinds of edge:
//! every method's `input → output` edges (a self-referencing input adds none), each
//! filter's `argument → filtered` *guard* edges, and each conditional's
//! `match cell → output` *guard* edges for every method output of its branch and
//! default relationships. The invariant is that no guard edge `g → t` lies on a cycle,
//! i.e. `t` never reaches `g`. Cycles made only of method edges are ordinary multi-way
//! relationships and are allowed.
//!
//! `Sheet::add_filter`, `Sheet::add_conditional` and `Sheet::add_relationship` apply
//! their mutation, call [`Sheet::guard_violation`], and roll back on `Some`.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::cell::CellId;
use crate::error::ErrorSite;
use crate::relationship::RelationshipId;

use super::Sheet;

/// One edge of a [`DependencyPath`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hop {
    /// A method edge through `relationship`'s method at index `method`.
    Method {
        relationship: RelationshipId,
        method: usize,
    },
    /// A filter or conditional guard edge.
    Guard,
}

/// A path in the static dependency graph.
///
/// - Invariant: `cells.len() == hops.len() + 1`; `hops[i]` leads from `cells[i]` to
///   `cells[i + 1]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DependencyPath {
    pub(super) cells: Vec<CellId>,
    pub(super) hops: Vec<Hop>,
}

impl DependencyPath {
    /// Returns the path as `Error::DependencyCycle` sites: each cell as `Cell`, each
    /// method hop as `Relationship`, or as `MethodIndex` when its relationship is
    /// `pending` (still being added), and each guard hop as nothing.
    ///
    /// - Complexity: O(n) in the path length.
    pub(super) fn into_sites(self, pending: Option<RelationshipId>) -> Vec<ErrorSite> {
        let mut sites = Vec::with_capacity(self.cells.len() + self.hops.len());
        sites.push(ErrorSite::Cell(self.cells[0]));
        for (hop, &cell) in self.hops.iter().zip(&self.cells[1..]) {
            if let Hop::Method {
                relationship,
                method,
            } = *hop
            {
                sites.push(if Some(relationship) == pending {
                    ErrorSite::MethodIndex(method)
                } else {
                    ErrorSite::Relationship(relationship)
                });
            }
            sites.push(ErrorSite::Cell(cell));
        }
        sites
    }
}

impl Sheet {
    /// Returns a violated guard's cycle, as a path from the governed cell `t` to the guard
    /// cell `g` whose guard edge `g → t` closes it, or `None` if every filter and
    /// conditional is independent of the cells it governs.
    ///
    /// Filters are checked in cell order, then conditionals in insertion order; the first
    /// violation found is returned.
    ///
    /// - Complexity: O(G · (V + E)) where G = filters + conditionals, V = cells, E =
    ///   method, filter and conditional edges.
    pub(super) fn guard_violation(&self) -> Option<DependencyPath> {
        let gates = self.conditional_gates();
        for (cell, data) in self.cells.iter() {
            if let Some(filter) = &data.filter {
                let args: HashSet<CellId> = filter.args.iter().copied().collect();
                if let Some(path) = self.dependency_path(&[cell], &args, &gates) {
                    return Some(path);
                }
            }
        }
        for conditional in self.conditionals.values() {
            let governed: Vec<CellId> = self.governed_outputs(conditional).collect();
            let matches: HashSet<CellId> = conditional.match_cells().iter().copied().collect();
            if let Some(path) = self.dependency_path(&governed, &matches, &gates) {
                return Some(path);
            }
        }
        None
    }

    /// Returns every method output of `conditional`'s branch and default relationships,
    /// possibly with repeats.
    ///
    /// - Complexity: O(R · M · K) over the governed relationships' methods and outputs.
    fn governed_outputs<'a>(
        &'a self,
        conditional: &'a crate::conditional::ConditionalData,
    ) -> impl Iterator<Item = CellId> + 'a {
        conditional
            .branches
            .iter()
            .flat_map(|branch| branch.relationships.iter())
            .chain(conditional.default.iter())
            .flat_map(move |&rel| self.relationships[rel].methods.iter())
            .flat_map(|method| method.outputs.iter().copied())
    }

    /// Returns each match cell's conditional guard targets: `gates[m]` lists every method
    /// output governed by a conditional that has `m` as a match cell.
    ///
    /// - Complexity: O(total governed outputs · match cells per conditional).
    fn conditional_gates(&self) -> HashMap<CellId, Vec<CellId>> {
        let mut gates: HashMap<CellId, Vec<CellId>> = HashMap::new();
        for conditional in self.conditionals.values() {
            for &m in conditional.match_cells() {
                gates
                    .entry(m)
                    .or_default()
                    .extend(self.governed_outputs(conditional));
            }
        }
        gates
    }

    /// Returns a shortest path in the static dependency graph from any cell in `from` to
    /// any cell in `to`, or `None` if no cell of `to` is reachable. A cell in both sets
    /// yields a single-cell path.
    ///
    /// `gates` is [`Sheet::conditional_gates`]'s result for the current sheet.
    ///
    /// - Complexity: O(V + E).
    fn dependency_path(
        &self,
        from: &[CellId],
        to: &HashSet<CellId>,
        gates: &HashMap<CellId, Vec<CellId>>,
    ) -> Option<DependencyPath> {
        let mut parent: HashMap<CellId, Option<(CellId, Hop)>> = HashMap::new();
        let mut queue: VecDeque<CellId> = VecDeque::new();
        for &cell in from {
            if parent.insert(cell, None).is_none() {
                queue.push_back(cell);
            }
        }
        while let Some(cell) = queue.pop_front() {
            if to.contains(&cell) {
                return Some(Self::rebuild_path(cell, &parent));
            }
            let mut visit = |next: CellId, hop: Hop| {
                if let std::collections::hash_map::Entry::Vacant(e) = parent.entry(next) {
                    e.insert(Some((cell, hop)));
                    queue.push_back(next);
                }
            };
            for &relationship in &self.cells[cell].adj {
                for (method, data) in self.relationships[relationship].methods.iter().enumerate() {
                    if data.inputs.contains(&cell) {
                        for &output in data.outputs.iter().filter(|&&o| o != cell) {
                            visit(output, Hop::Method { relationship, method });
                        }
                    }
                }
            }
            for &filtered in self.filter_dependents.get(&cell).into_iter().flatten() {
                visit(filtered, Hop::Guard);
            }
            for &governed in gates.get(&cell).into_iter().flatten() {
                visit(governed, Hop::Guard);
            }
        }
        None
    }

    /// Returns the path ending at `end`, following `parent` links back to a root.
    ///
    /// - Precondition: `parent` links from `end` terminate at a `None` root.
    /// - Complexity: O(n) in the path length.
    fn rebuild_path(end: CellId, parent: &HashMap<CellId, Option<(CellId, Hop)>>) -> DependencyPath {
        let mut cells = vec![end];
        let mut hops = Vec::new();
        let mut cursor = end;
        while let Some(&Some((prev, hop))) = parent.get(&cursor) {
            cells.push(prev);
            hops.push(hop);
            cursor = prev;
        }
        cells.reverse();
        hops.reverse();
        DependencyPath { cells, hops }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Filter, MatchExpr, Method};

    #[test]
    fn guard_violation_is_none_for_a_sheet_without_guards() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
                Method::from_fn_1_1(b, a, |x: &i32| Ok(*x)),
            ])
            .unwrap();
        assert_eq!(sheet.guard_violation(), None);
    }

    #[test]
    fn guard_violation_is_none_for_an_upstream_filter_argument() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        sheet
            .add_filter(b, Filter::from_fn_1(a, |v: &i32, lo: &i32| Ok((*v).max(*lo))))
            .unwrap();
        assert_eq!(sheet.guard_violation(), None);
    }

    #[test]
    fn into_sites_renders_method_hops_as_relationships_or_pending_method_indices() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let r = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let path = DependencyPath {
            cells: vec![a, b, c],
            hops: vec![Hop::Method { relationship: r, method: 0 }, Hop::Guard],
        };
        assert_eq!(
            path.clone().into_sites(None),
            vec![
                ErrorSite::Cell(a),
                ErrorSite::Relationship(r),
                ErrorSite::Cell(b),
                ErrorSite::Cell(c),
            ]
        );
        assert_eq!(
            path.into_sites(Some(r)),
            vec![
                ErrorSite::Cell(a),
                ErrorSite::MethodIndex(0),
                ErrorSite::Cell(b),
                ErrorSite::Cell(c),
            ]
        );
    }

    #[test]
    fn guard_violation_is_none_for_a_conditional_whose_branch_only_reads_its_match_cell() {
        let mut sheet = Sheet::new();
        let mode = sheet.add_cell(0_i32);
        let out = sheet.add_cell(0_i32);
        let r = sheet
            .add_relationship(vec![Method::from_fn_1_1(mode, out, |x: &i32| Ok(*x))])
            .unwrap();
        sheet
            .add_conditional(MatchExpr::cell(mode), vec![(vec![0_i32], vec![r])], vec![])
            .unwrap();
        assert_eq!(sheet.guard_violation(), None);
    }
}
```

In `adam-rs/src/sheet.rs`, directly after the file's `use` statements add:

```rust
mod dependency;
```

Violation-returning cases are covered through the public mutators in Tasks 3–5 (they cannot be constructed here once those tasks land, because the mutators reject them).

- [x] **Step 2: Run tests**

Run: `cargo test -p adam-rs --lib sheet::dependency`
Expected: 4 PASS. If `guard_violation`/`Hop` trip `dead_code` warnings, that is expected until Task 3 wires them in; do not add `#[allow]` — proceed directly to Task 3 before running clippy.

- [x] **Step 3: Commit**

```bash
cargo fmt --all
git add adam-rs/src/sheet/dependency.rs adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): static dependency graph traversal for guard independence"
```

---

### Task 3: `add_filter` rejects dependent arguments

**Files:**
- Modify: `adam-rs/src/sheet.rs` `add_filter` (~line 615-660)
- Create: `adam-rs/tests/dependency_guards.rs`

**Interfaces:**
- Consumes: `Sheet::guard_violation`, `DependencyPath::into_sites` (Task 2); `Error::DependencyCycle` (Task 1).

- [x] **Step 1: Write failing integration tests**

Create `adam-rs/tests/dependency_guards.rs`:

```rust
//! Contract tests for the static guard-independence invariant: a filter's arguments and a
//! conditional's match subject must not depend on a cell they govern.

use adam_rs::{Error, ErrorSite, Filter, MatchExpr, Method, Sheet};

fn min_filter(bound: adam_rs::CellId) -> Filter {
    Filter::from_fn_1(bound, |v: &i32, b: &i32| Ok((*v).min(*b)))
}

#[test]
fn add_filter_rejects_an_argument_derived_from_the_filtered_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(0_i32);
    let r = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    let err = sheet.add_filter(a, min_filter(b)).unwrap_err();
    match err {
        Error::DependencyCycle { sites } => assert_eq!(
            sites,
            vec![ErrorSite::Cell(a), ErrorSite::Relationship(r), ErrorSite::Cell(b)]
        ),
        other => panic!("expected DependencyCycle, got {other:?}"),
    }
    assert_eq!(sheet.filter_args(a), None, "the sheet must be left unchanged");
    assert!(sheet.filter_dependents(b).is_empty());
    sheet.propagate().unwrap();
}

#[test]
fn add_filter_rejects_the_issue_153_sum_example() {
    // z = x + y with methods z <- (x, y) and x <- (z, y); y filtered by z. y is never an
    // output, so the edge y -> z of z <- (x, y) always exists.
    let mut sheet = Sheet::new();
    let x = sheet.add_cell(1_i32);
    let y = sheet.add_cell(2_i32);
    let z = sheet.add_cell(3_i32);
    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([x, y], z, |a: &i32, b: &i32| Ok(a + b)),
            Method::from_fn_2_1([z, y], x, |c: &i32, b: &i32| Ok(c - b)),
        ])
        .unwrap();
    assert!(matches!(
        sheet.add_filter(y, min_filter(z)),
        Err(Error::DependencyCycle { .. })
    ));
}

#[test]
fn add_filter_rejects_the_issue_153_two_relationship_example() {
    // R1 over {x, y, z} with all three single-output methods, R2 over {w, y}; y filtered
    // by z. R1's z <- (x, y) makes z depend on y regardless of which plan runs.
    let mut sheet = Sheet::new();
    let w = sheet.add_cell(1_i32);
    let z = sheet.add_cell(3_i32);
    let x = sheet.add_cell(1_i32);
    let y = sheet.add_cell(2_i32);
    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([x, y], z, |a: &i32, b: &i32| Ok(a + b)),
            Method::from_fn_2_1([y, z], x, |b: &i32, c: &i32| Ok(c - b)),
            Method::from_fn_2_1([x, z], y, |a: &i32, c: &i32| Ok(c - a)),
        ])
        .unwrap();
    sheet
        .add_relationship(vec![
            Method::from_fn_1_1(w, y, |v: &i32| Ok(*v)),
            Method::from_fn_1_1(y, w, |v: &i32| Ok(*v)),
        ])
        .unwrap();
    assert!(matches!(
        sheet.add_filter(y, min_filter(z)),
        Err(Error::DependencyCycle { .. })
    ));
}

#[test]
fn add_filter_accepts_an_upstream_argument_and_conforms_on_propagate() {
    let mut sheet = Sheet::new();
    let hi = sheet.add_cell(3_i32);
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    sheet.add_filter(a, min_filter(hi)).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 3);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 3);
}

#[test]
fn add_filter_rejects_a_filter_closing_an_existing_conditional_guard() {
    // Conditional on m governs k -> o; filtering m by o closes m -gate-> o -filter-> m.
    let mut sheet = Sheet::new();
    let m = sheet.add_cell(0_i32);
    let k = sheet.add_cell(0_i32);
    let o = sheet.add_cell(0_i32);
    let r = sheet
        .add_relationship(vec![Method::from_fn_1_1(k, o, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_conditional(MatchExpr::cell(m), vec![(vec![0_i32], vec![r])], vec![])
        .unwrap();
    assert!(matches!(
        sheet.add_filter(m, min_filter(o)),
        Err(Error::DependencyCycle { .. })
    ));
    assert_eq!(sheet.filter_args(m), None);
}
```

- [x] **Step 2: Run to verify failure**

Run: `cargo test -p adam-rs --test dependency_guards`
Expected: the four `rejects` tests FAIL (`add_filter` returns `Ok`); `accepts_an_upstream_argument` PASSES.

- [x] **Step 3: Implement the check with rollback**

In `add_filter`, after the existing validation loop, replace the tail (`for &arg ... push(cell); self.cells[cell].filter = Some(filter.0); Ok(())`) with:

```rust
        for &arg in &filter.0.args {
            self.filter_dependents.entry(arg).or_default().push(cell);
        }
        self.cells[cell].filter = Some(filter.0);

        if let Some(path) = self.guard_violation() {
            let filter = self.cells[cell].filter.take().expect("attached above");
            for arg in &filter.args {
                if let Some(dependents) = self.filter_dependents.get_mut(arg) {
                    dependents.pop();
                    if dependents.is_empty() {
                        self.filter_dependents.remove(arg);
                    }
                }
            }
            return Err(Error::DependencyCycle {
                sites: path.into_sites(None),
            });
        }
        Ok(())
```

(`pop` is correct because each arg's `cell` entry was pushed last, just above.)

Update the `add_filter` doc: add to `# Errors`:

```rust
    /// - `Error::DependencyCycle` — with `filter` attached, some filter's or conditional's
    ///   inputs would depend on a cell it governs (see `Error::DependencyCycle`); e.g.
    ///   one of `filter`'s arguments is reachable from `cell` through method edges. The
    ///   sheet is left unchanged.
```

and change the complexity line to `/// - Complexity: O(G · (V + E)) for the dependency check; see Error::DependencyCycle.` spelled out as `G = filters + conditionals, V = cells, E = dependency edges`.

- [x] **Step 4: Run tests**

Run: `cargo test -p adam-rs`
Expected: `dependency_guards` all PASS. Any *other* failing test must be triaged: if its sheet has a filter whose argument depends on the filtered cell (an invalid sheet under spec rule 5), rewrite it to assert `Error::DependencyCycle` at `add_filter` or restructure it so the argument is independent while keeping its original intent; record each such change in the commit message body. Do not weaken the check.

- [x] **Step 5: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "feat(adam-rs): add_filter rejects filters that depend on the filtered cell"
```

---

### Task 4: `add_relationship` rejects relationships that close a guard cycle

**Files:**
- Modify: `adam-rs/src/sheet.rs` `add_relationship` (tail, ~line 285-311)
- Modify: `adam-rs/tests/dependency_guards.rs`

**Interfaces:**
- Consumes: `Sheet::guard_violation`, `DependencyPath::into_sites(Some(rel_id))`.

- [x] **Step 1: Write failing tests** (append to `adam-rs/tests/dependency_guards.rs`)

```rust
#[test]
fn add_relationship_rejects_a_relationship_that_makes_a_filter_argument_dependent() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(0_i32);
    sheet.add_filter(a, min_filter(b)).unwrap();
    let err = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap_err();
    match err {
        Error::DependencyCycle { sites } => assert_eq!(
            sites,
            vec![ErrorSite::Cell(a), ErrorSite::MethodIndex(0), ErrorSite::Cell(b)]
        ),
        other => panic!("expected DependencyCycle, got {other:?}"),
    }
    assert_eq!(sheet.relationships().count(), 0, "the sheet must be left unchanged");
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 0, "filter still clamps a to b");
}

#[test]
fn add_relationship_rejects_a_relationship_that_feeds_a_match_cell_from_its_branch() {
    // Conditional on p governs k -> o; a later unconditional o -> p closes the cycle.
    let mut sheet = Sheet::new();
    let p = sheet.add_cell(0_i32);
    let k = sheet.add_cell(0_i32);
    let o = sheet.add_cell(0_i32);
    let r = sheet
        .add_relationship(vec![Method::from_fn_1_1(k, o, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_conditional(MatchExpr::cell(p), vec![(vec![0_i32], vec![r])], vec![])
        .unwrap();
    assert!(matches!(
        sheet.add_relationship(vec![Method::from_fn_1_1(o, p, |x: &i32| Ok(*x))]),
        Err(Error::DependencyCycle { .. })
    ));
    assert_eq!(sheet.relationships().count(), 1);
}
```

- [x] **Step 2: Run to verify failure**

Run: `cargo test -p adam-rs --test dependency_guards add_relationship`
Expected: both FAIL (relationship accepted).

- [x] **Step 3: Implement**

At the end of `add_relationship`, replace the final `Ok(rel_id)` (after the `cell.adj.push(rel_id)` loop) with:

```rust
        if let Some(path) = self.guard_violation() {
            let rel = self.relationships.remove(rel_id).expect("inserted above");
            for cell_id in rel.adj {
                if let Some(cell) = self.cells.get_mut(cell_id)
                    && cell.adj.last() == Some(&rel_id)
                {
                    cell.adj.pop();
                }
            }
            return Err(Error::DependencyCycle {
                sites: path.into_sites(Some(rel_id)),
            });
        }

        Ok(rel_id)
```

Add to its `# Errors`:

```rust
    /// - `Error::DependencyCycle` — the relationship's methods would make some filter's
    ///   argument or conditional's match subject depend on a cell it governs; method hops
    ///   through this relationship are reported as `MethodIndex`. The sheet is left
    ///   unchanged.
```

and extend the complexity line with `+ O(G · (V + E))` for the dependency check.

- [x] **Step 4: Run tests**

Run: `cargo test -p adam-rs; cargo test -p adam-lang`
Expected: PASS. In adam-lang, `add_relationship` errors whose `sites()[0]` is not a `MethodIndex` fall back to the whole relationship block as the primary span (see the comment at `adam-lang/src/parser.rs:~915`) — that is the intended behavior for `DependencyCycle`; update that comment to name `DependencyCycle` as the variant that leads with a `Cell`. Triage other failures as in Task 3 Step 4.

- [x] **Step 5: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "feat(adam-rs): add_relationship rejects relationships that close a guard cycle"
```

---

### Task 5: `add_conditional` uses the guard check instead of the contributing-cells rule

**Files:**
- Modify: `adam-rs/src/sheet.rs` `add_conditional` (~line 312-478) and tests ~line 1950-2120
- Modify: `adam-rs/src/error.rs` `InvalidConditional` doc (~line 103-116)
- Modify: `adam-rs/tests/dependency_guards.rs`
- Modify: `adam-lang/src/error_labels.rs` (`InvalidConditional` `Cell` arm, ~line 84-93), `adam-lang/src/parser.rs` test `conditional_structural_error_spans_the_conditional_not_the_sheet` (~line 3737) comment

**Interfaces:**
- Consumes: `Sheet::guard_violation`, `DependencyPath::into_sites(None)`.

- [x] **Step 1: Write failing tests** (append to `adam-rs/tests/dependency_guards.rs`)

```rust
#[test]
fn add_conditional_rejects_a_single_method_branch_that_writes_its_own_match_cell() {
    let mut sheet = Sheet::new();
    let mode = sheet.add_cell(0_i32);
    let other = sheet.add_cell(0_i32);
    let r = sheet
        .add_relationship(vec![Method::from_fn_1_1(other, mode, |x: &i32| Ok(*x))])
        .unwrap();
    let err = sheet
        .add_conditional(MatchExpr::cell(mode), vec![(vec![0_i32], vec![r])], vec![])
        .unwrap_err();
    match err {
        Error::DependencyCycle { sites } => assert_eq!(sites, vec![ErrorSite::Cell(mode)]),
        other => panic!("expected DependencyCycle, got {other:?}"),
    }
}

#[test]
fn add_conditional_rejects_a_branch_feeding_the_match_cell_through_another_relationship() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let p = sheet.add_cell(0_i32);
    let upstream = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, p, |x: &i32| Ok(*x))])
        .unwrap();
    let branch = sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();
    let err = sheet
        .add_conditional(MatchExpr::cell(p), vec![(vec![0_i32], vec![branch])], vec![])
        .unwrap_err();
    match err {
        Error::DependencyCycle { sites } => assert_eq!(
            sites,
            vec![ErrorSite::Cell(a), ErrorSite::Relationship(upstream), ErrorSite::Cell(p)]
        ),
        other => panic!("expected DependencyCycle, got {other:?}"),
    }
}

#[test]
fn add_conditional_accepts_a_multi_method_branch_that_only_reads_match_contributors() {
    // a feeds p; the branch relationship over {a, b, c} always reads a and writes b or c,
    // so nothing it governs reaches p.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(1_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);
    let p = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, p, |x: &i32| Ok(*x))])
        .unwrap();
    let branch = sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, c], b, |x: &i32, y: &i32| Ok(x + y)),
            Method::from_fn_2_1([a, b], c, |x: &i32, y: &i32| Ok(y - x)),
        ])
        .unwrap();
    let cid = sheet
        .add_conditional(MatchExpr::cell(p), vec![(vec![1_i32], vec![branch])], vec![])
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(sheet.conditional_active_branch(cid).unwrap(), Some(0));
}

#[test]
fn add_conditional_rejects_a_conditional_closing_an_existing_filter_guard() {
    // y filtered by z; a conditional on y governing k -> z closes y -gate-> z -filter-> y.
    let mut sheet = Sheet::new();
    let y = sheet.add_cell(0_i32);
    let z = sheet.add_cell(0_i32);
    let k = sheet.add_cell(7_i32);
    sheet.add_filter(y, min_filter(z)).unwrap();
    let r = sheet
        .add_relationship(vec![Method::from_fn_1_1(k, z, |x: &i32| Ok(*x))])
        .unwrap();
    assert!(matches!(
        sheet.add_conditional(MatchExpr::cell(y), vec![(vec![0_i32], vec![r])], vec![]),
        Err(Error::DependencyCycle { .. })
    ));
    // Rolled back: r is still unconditional, so it runs.
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(z).unwrap(), 7);
}
```

- [x] **Step 2: Run to verify failure**

Run: `cargo test -p adam-rs --test dependency_guards add_conditional`
Expected: `single_method_branch` FAILS (accepted today); `through_another_relationship` FAILS (accepted today, single-method branch); `multi_method_branch_that_only_reads` FAILS with `InvalidConditional`; `closing_an_existing_filter_guard` FAILS (accepted).

- [x] **Step 3: Implement**

In `add_conditional`:
1. Delete the `contributing_cells` computation block and the `if rel.adj.iter().any(|c| contributing_cells.contains(c)) && rel.methods.len() != 1 { ... }` check inside the `for &rel_id in &all_rels` loop (keep the loop's existence check and the already-conditional check).
2. Replace the final insertion with insertion + check + rollback:

```rust
        for &rel_id in &all_rels {
            self.conditional_relationships.insert(rel_id);
        }
        let id = self.conditionals.insert(ConditionalData {
            source: source.0,
            branches: typed_branches,
            default,
        });

        if let Some(path) = self.guard_violation() {
            self.conditionals.remove(id);
            for rel_id in &all_rels {
                self.conditional_relationships.remove(rel_id);
            }
            return Err(Error::DependencyCycle {
                sites: path.into_sites(None),
            });
        }
        Ok(id)
```

3. Update the doc `# Errors`: remove the "a branch relationship shares a cell with the match subject or any of its unconditional upstream contributors and has more than one method" clause from `InvalidConditional`, and add:

```rust
    /// - `Error::DependencyCycle` — a method output of a branch or default relationship
    ///   reaches a match-subject cell in the static dependency graph, or the new
    ///   conditional closes a cycle through an existing filter or conditional. The sheet
    ///   is left unchanged.
```

Update complexity to add `+ O(G · (V + E))`.

In `adam-rs/src/error.rs`, remove the same clause from `InvalidConditional`'s doc and its `sites` doc (the multi-method case no longer exists; the remaining `Cell` site is the match-cell type-mismatch case).

4. Update existing `sheet.rs` tests:
   - The test at ~line 1955 (branch `{a→b, b→a}` with match `a`) and `add_conditional_returns_error_when_branch_rel_involves_cell_upstream_of_match_cell` and `..._of_either_expr_input`: change `Error::InvalidConditional { .. }` to `Error::DependencyCycle { .. }`, and rename `involves_cell_upstream` → `writes_a_cell_upstream` to match the new contract.
   - `invalid_conditional_multi_method_branch_names_the_relationship`: rename to `dependency_cycle_from_a_conditional_names_the_match_cell` and assert `err.sites().contains(&ErrorSite::Cell(a))`.
   - `add_conditional_allows_multi_method_rel_not_involving_match_cell`: unchanged; update its comment to "Branch relationships whose outputs do not reach the match cell may have any number of methods."

5. adam-lang:
   - `error_labels.rs` `InvalidConditional` `Cell` arm: change the text to ``format!("cell `{name}` is the match subject")`` / `"this cell is the match subject"`. Add a `DependencyCycle` label test next to the existing ones:

```rust
    #[test]
    fn dependency_cycle_cell_sites_are_labelled_as_part_of_the_cycle() {
        let cell = CellId::default();
        let e = adam_rs::Error::DependencyCycle { sites: vec![ErrorSite::Cell(cell)] };
        let label = site_label(&e, 0, &|_| Some("mode".to_string()));
        assert_eq!(label, "cell `mode` is part of the cycle");
    }
```

   - `parser.rs` `conditional_structural_error_spans_the_conditional_not_the_sheet`: update the comment to say the branch writes the match cell `mode`, so `add_conditional` returns `DependencyCycle`; the line-5 assertion is unchanged.

- [x] **Step 4: Run tests**

Run: `cargo test -p adam-rs; cargo test -p adam-lang`
Expected: PASS. Triage other failures as in Task 3 Step 4.

- [x] **Step 5: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "feat(adam-rs): conditionals use the static guard-independence check"
```

---

### Task 6: adam-lang secondary labels for filter errors

**Files:**
- Modify: `adam-lang/src/parser.rs` — the three `add_filter` call sites (~lines 387, 480, 1531) and `conditional_error` (~line 1363)

**Interfaces:**
- Produces: `fn sheet_error(ctx: &ParseContext, primary: Span, e: adam_rs::Error) -> ParseError` (renamed from `conditional_error`, same body).

- [x] **Step 1: Write the failing test** (in `parser.rs` tests)

```rust
    #[test]
    fn dependent_filter_is_reported_with_the_cycle_cells_as_secondary_labels() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let source = "sheet s {\n    cell b: i32 = 0;\n    cell a: i32 = 5;\n    relationship { b := a; }\n    cell c: i32 = 0 filter _ + b - b;\n}";
        // Control: a filter on an unrelated cell parses.
        assert!(parser.parse_str(source).is_ok());

        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let source = "sheet s {\n    cell b: i32 = 0;\n    relationship { b := a; }\n    cell a: i32 = 5 filter _ + b - b;\n}";
        let err = parser.parse_str(source).unwrap_err();
        assert_eq!(err.message(), "a filter or conditional depends on a cell it governs (dependency cycle)");
        assert!(!err.secondary().is_empty());
    }
```

Before writing it, check the exact grammar in existing `cell_filter_*` tests (~line 2176-2370) and adjust the filter expression and relationship syntax to forms those tests use (a filter body referencing `_` and another cell; a relationship referencing a cell declared later may be disallowed — if so, declare `a` first without a filter and use a `source`/`out` form that attaches a filter after the relationship, or restructure so the relationship follows the filtered cell and the violation is raised by `add_relationship` instead; in that case assert the message only). Check `ParseError`'s accessor name for secondary labels (`secondary()` or similar) in `cel-parser`.

- [x] **Step 2: Run to verify failure**

Run: `cargo test -p adam-lang --lib dependent_filter`
Expected: FAIL on the secondary-label assertion (plain `ParseError::new` has none).

- [x] **Step 3: Implement**

Rename `conditional_error` to `sheet_error` (update its doc: "Turns a `Sheet` construction failure into a `ParseError` with `primary` as the primary span ..."), update its two call sites, and change each `add_filter` call's `.map_err(|e| ParseError::new(e.to_string(), name_span))` to `.map_err(|e| Self::sheet_error(ctx, name_span, e))`. (If a borrow conflict arises because `ctx.sheet` is mutably borrowed, bind the result first: `let result = ctx.sheet.add_filter(..); result.map_err(|e| Self::sheet_error(ctx, name_span, e))?;`.)

- [x] **Step 4: Run tests**

Run: `cargo test -p adam-lang`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "feat(adam-lang): label dependency-cycle sites on filter errors"
```

---

### Task 7: Full verification, handoff, PR

**Files:**
- Create: `docs/superpowers/2026-09-27-planner-generalization-phase-a-handoff.md`
- Modify: `docs/superpowers/specs/2026-09-27-adam-rs-planner-generalization-design.md` (Status line)

- [x] **Step 1: Run the full check suite** (each must pass; build/test output must contain no `warning:` lines)

```powershell
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
$env:RUSTDOCFLAGS="-D warnings"; cargo doc --lib --no-deps --workspace
cargo test -p begin --no-default-features
```

If `every_bundled_example_parses_successfully` fails, the named example is an invalid sheet under rule 5: fix the example (keep its demonstrated behavior with an independent filter argument/match subject) and note it in the handoff.

- [x] **Step 2: Write the handoff doc**

`docs/superpowers/2026-09-27-planner-generalization-phase-a-handoff.md` with sections: **Done** (static guard check in `add_filter`/`add_relationship`/`add_conditional`, `DependencyCycle` replacing `FilterCycle`, contributing-cells rule removed, adam-lang labels; list any tests/examples rewritten as invalid sheets), **Deferred** (analyzer: rule 3 method reachability; 2D containment stress test), **Remaining** (Phase B: spec §1 seedfill — antichain output sets, elimination-based seed method selection, `Error::SeedCycle`, ascending-strength sibling fold, #186; Phase C: spec §3 automatic plan reuse, #152). Set the spec's Status to `Phase A implemented; Phases B–C pending`.

- [x] **Step 3: Commit and open the PR**

```bash
git add -A
git commit -m "docs(adam-rs): planner generalization phase A handoff"
```

Then use the `pr-open` skill. PR body: summary of rule 5 and the check, "Refs #186, #152" (not "Fixes" — those close in Phases B/C), and a note that #153 was closed as not applicable in favor of this check.
