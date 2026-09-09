# Value-Aware Self-Referencing Planning Implementation Plan

**Superseded:** the architecture below was implemented, then replaced by *seedfill* — a
value-blind planner plus `planner::build_seeds`, which reconstructs each self-referencing
input's value from `source` at execution time instead of partitioning into components and
scoring candidate assignments. Kept here as the record of the approach that was tried and
why it fell short; see the `refactor(adam-rs): replace value-aware self-ref planning with
seedfill` commit message and `adam-rs/src/planner/seed.rs` for the mechanism actually
shipped.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix issue #182 (planner silently overwrites a consistent edit in a self-referencing inequality chain) by making source selection value-aware for self-referencing relationships, and document `adam-rs`'s sheet invariants.

**Architecture:** Partition each `propagate()`'s active relationship set into connected components. A component with no self-referencing method keeps today's exact strength-lexicographic greedy release (`release::resolve_plain`, renamed from today's `resolve`). A component containing at least one self-referencing method is instead resolved by a new `planner::stay` module: enumerate every structurally valid acyclic assignment for that component, concretely execute each against current cell values, and pick the one whose violated self-referencing stays are lexicographically weakest.

**Tech Stack:** Rust, `slotmap`, existing `adam-rs` planner internals (`planner::{matching, digraph, scc}`).

**Spec:** `docs/superpowers/specs/2026-09-07-adam-rs-value-aware-self-ref-planning-design.md`

## Global Constraints

- No change to `Method`, `Filter`, `Plan`, or any public `Sheet` API.
- No change to how functional (non-self-referencing) relationships are planned — all existing `planner.rs`/`matching.rs`/`digraph.rs`/`release.rs` unit tests must keep passing unmodified.
- `cargo fmt --all` must be run before every commit (enforced by pre-commit hook).
- Every new `pub(crate)`/`pub` item needs a contract-style doc comment (Summary / Preconditions / `# Errors` / Postconditions / Complexity, per project convention) — see `CLAUDE.md`'s Documentation comments section.
- Fallible arithmetic uses `checked_*`, not wrapping — not applicable to this plan's own new code (no new arithmetic), but do not introduce any.
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings.

---

## Task 1: `Assignment::solve_acyclic_all` in `planner/matching.rs`

**Files:**
- Modify: `adam-rs/src/planner/matching.rs`

**Interfaces:**
- Produces: `Assignment::solve_acyclic_all(relationships: &SlotMap<RelationshipId, RelationshipData>, active: &HashSet<RelationshipId>, forbidden: &HashSet<CellId>) -> Vec<Assignment>` — used by Task 3's `stay::resolve_component`.

- [ ] **Step 1: Write the failing tests**

Add to `adam-rs/src/planner/matching.rs`'s existing `mod tests` block:

```rust
    #[test]
    fn solve_acyclic_all_returns_every_structurally_valid_acyclic_assignment() {
        // The issue #182 shape: a<=b via rel1 (self-ref a or b), b<=c via rel2
        // (self-ref b or c). Exactly 3 combinations are simultaneously valid (no
        // double claim on b) and acyclic: source={a}, source={b}, source={c}.
        // (rel1 claims b AND rel2 claims b is the only combination excluded, since
        // both would claim the same cell.)
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let rel1 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([a, b], b, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        let rel2 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([b, c], b, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([b, c], c, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        let active: HashSet<_> = [rel1, rel2].into_iter().collect();

        let results = Assignment::solve_acyclic_all(&sheet.relationships, &active, &HashSet::new());

        assert_eq!(results.len(), 3);
        let source_sets: HashSet<CellId> = results
            .iter()
            .map(|assignment| {
                *[a, b, c]
                    .iter()
                    .find(|c| !assignment.claimed.contains_key(c))
                    .expect("exactly one cell is unclaimed per valid assignment")
            })
            .collect();
        assert_eq!(source_sets, [a, b, c].into_iter().collect());
    }

    #[test]
    fn solve_acyclic_all_returns_a_single_assignment_when_only_one_choice_exists() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let active: HashSet<_> = [rel].into_iter().collect();

        let results = Assignment::solve_acyclic_all(&sheet.relationships, &active, &HashSet::new());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].claimed[&b], rel);
    }

    #[test]
    fn solve_acyclic_all_returns_empty_when_no_assignment_exists() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let out = sheet.add_cell(0_i32);
        let r1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, out, |x: &i32| Ok(*x))])
            .unwrap();
        let r2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(b, out, |x: &i32| Ok(*x))])
            .unwrap();
        let active: HashSet<_> = [r1, r2].into_iter().collect();

        let results = Assignment::solve_acyclic_all(&sheet.relationships, &active, &HashSet::new());

        assert!(results.is_empty());
    }

    #[test]
    fn solve_acyclic_all_excludes_a_structurally_valid_but_cyclic_combination() {
        // R1's only method claims c via [a,b]->c; R2's only method claims b via
        // [c,d]->b: the only structurally valid combination is cyclic (b depends on
        // c, c depends on b), so solve_acyclic_all must return nothing even though
        // Assignment::solve (which has no acyclicity notion) accepts it.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0.0_f64);
        let b = sheet.add_cell(0.0_f64);
        let c = sheet.add_cell(0.0_f64);
        let d = sheet.add_cell(0.0_f64);
        let r1 = sheet
            .add_relationship(vec![Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| {
                Ok(x * y)
            })])
            .unwrap();
        let r2 = sheet
            .add_relationship(vec![Method::from_fn_2_1([c, d], b, |x: &f64, y: &f64| {
                Ok(y / x)
            })])
            .unwrap();
        let active: HashSet<_> = [r1, r2].into_iter().collect();

        assert!(Assignment::solve(&sheet.relationships, &active, &HashSet::new()).is_some());
        let results = Assignment::solve_acyclic_all(&sheet.relationships, &active, &HashSet::new());
        assert!(results.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs --lib planner::matching -- solve_acyclic_all`
Expected: FAIL to compile — `solve_acyclic_all` is not a member of `Assignment`.

- [ ] **Step 3: Implement `solve_acyclic_all`**

Add this method inside the existing `impl Assignment { ... }` block in `adam-rs/src/planner/matching.rs`, directly after `solve_acyclic`:

```rust
    /// Finds every assignment of one method per relationship in `active`, forbidding any
    /// cell in `forbidden` from being claimed as an output by anyone, such that no two
    /// relationships claim the same cell *and* the resulting dependency digraph
    /// ([`super::digraph::is_acyclic`]) has no cycle.
    ///
    /// Unlike [`Assignment::solve_acyclic`], which returns only the first such assignment
    /// it finds, this collects every one of them — used by
    /// [`super::stay::resolve_component`] to compare candidates by the concrete values
    /// they would produce, not just by structural feasibility.
    ///
    /// - Complexity: O(M^R · R·K) worst case (M = max methods per relationship, R =
    ///   active relationships, K = cells per method) — the same exhaustive search as
    ///   [`Assignment::solve_acyclic`], continued after each success instead of stopping
    ///   at the first.
    pub(crate) fn solve_acyclic_all(
        relationships: &SlotMap<RelationshipId, RelationshipData>,
        active: &HashSet<RelationshipId>,
        forbidden: &HashSet<CellId>,
    ) -> Vec<Self> {
        let order: Vec<RelationshipId> = relationships
            .keys()
            .filter(|r| active.contains(r))
            .collect();
        let mut assignment = Assignment {
            chosen: HashMap::new(),
            claimed: HashMap::new(),
        };
        let mut results = Vec::new();
        search_acyclic_all(&order, 0, relationships, forbidden, &mut assignment, &mut results);
        results
    }
```

Add this free function directly after the existing `search_acyclic` function (below the `impl Assignment` block):

```rust
/// Recursive backtracking search over every combination of method choices for
/// `order[idx..]`, used by [`Assignment::solve_acyclic_all`]. Identical to
/// [`search_acyclic`] except it records every acyclic combination it finds in `results`
/// instead of returning at the first one.
///
/// - Complexity: O(M^(len(order) - idx)) branches explored in the worst case.
fn search_acyclic_all(
    order: &[RelationshipId],
    idx: usize,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    forbidden: &HashSet<CellId>,
    assignment: &mut Assignment,
    results: &mut Vec<Assignment>,
) {
    let Some(&rel_id) = order.get(idx) else {
        if super::digraph::is_acyclic(assignment, relationships) {
            results.push(Assignment {
                chosen: assignment.chosen.clone(),
                claimed: assignment.claimed.clone(),
            });
        }
        return;
    };

    for (method_idx, method) in relationships[rel_id].methods.iter().enumerate() {
        let outputs: HashSet<CellId> = method.outputs.iter().copied().collect();
        if !outputs.is_disjoint(forbidden)
            || outputs.iter().any(|c| assignment.claimed.contains_key(c))
        {
            continue;
        }

        for &c in &outputs {
            assignment.claimed.insert(c, rel_id);
        }
        assignment.chosen.insert(rel_id, method_idx);

        search_acyclic_all(order, idx + 1, relationships, forbidden, assignment, results);

        assignment.chosen.remove(&rel_id);
        for &c in &outputs {
            assignment.claimed.remove(&c);
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-rs --lib planner::matching`
Expected: PASS (all matching.rs tests, including the 4 new ones).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-rs/src/planner/matching.rs
git commit -m "feat(adam-rs): add Assignment::solve_acyclic_all to enumerate every candidate"
```

---

## Task 2: `planner/stay.rs` skeleton — `has_self_reference` and `partition_components`

**Files:**
- Create: `adam-rs/src/planner/stay.rs`
- Modify: `adam-rs/src/planner.rs:38-41` (add `mod stay;`)

**Interfaces:**
- Consumes: none beyond `crate::cell::{CellData, CellId}`, `crate::relationship::{RelationshipData, RelationshipId}`.
- Produces: `has_self_reference(rel: &RelationshipData) -> bool`, `partition_components(cells, relationships, active) -> Vec<HashSet<RelationshipId>>` — both used by Task 4's `release::resolve`.

- [ ] **Step 1: Register the module**

In `adam-rs/src/planner.rs`, change:

```rust
mod digraph;
mod matching;
mod release;
mod scc;
```

to:

```rust
mod digraph;
mod matching;
mod release;
mod scc;
mod stay;
```

- [ ] **Step 2: Write the failing tests**

Create `adam-rs/src/planner/stay.rs` with just the module doc comment, imports, and a `mod tests` block (implementation comes in Step 4):

```rust
//! Value-aware source selection for self-referencing blocks: for a connected component of
//! relationships containing at least one self-referencing method, chooses which cell(s)
//! stay literal sources by concretely executing every structurally valid candidate and
//! comparing violated stays, instead of by strength alone. A component with no
//! self-referencing method is untouched, left to [`super::release`]'s existing
//! strength-lexicographic algorithm. See
//! `docs/superpowers/specs/2026-09-07-adam-rs-value-aware-self-ref-planning-design.md` for
//! the full design rationale and literature grounding.

use std::any::Any;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet, VecDeque};

use slotmap::SlotMap;

use crate::{
    cell::{CellData, CellId},
    relationship::{RelationshipData, RelationshipId},
};

use super::digraph::{self, Node};
use super::matching::{self, Assignment};
use super::scc;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Method, Sheet};

    #[test]
    fn has_self_reference_true_for_a_method_with_overlapping_inputs_and_outputs() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| {
                Ok((*x).min(*y))
            })])
            .unwrap();

        assert!(has_self_reference(&sheet.relationships[rel]));
    }

    #[test]
    fn has_self_reference_false_for_a_purely_functional_relationship() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();

        assert!(!has_self_reference(&sheet.relationships[rel]));
    }

    #[test]
    fn partition_components_merges_relationships_sharing_a_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let rel1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let rel2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(b, c, |x: &i32| Ok(*x))])
            .unwrap();
        let active: HashSet<_> = [rel1, rel2].into_iter().collect();

        let components = partition_components(&sheet.cells, &sheet.relationships, &active);

        assert_eq!(components.len(), 1);
        assert_eq!(components[0], active);
    }

    #[test]
    fn partition_components_keeps_disjoint_relationships_in_separate_components() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let d = sheet.add_cell(0_i32);
        let rel1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let rel2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(c, d, |x: &i32| Ok(*x))])
            .unwrap();
        let active: HashSet<_> = [rel1, rel2].into_iter().collect();

        let components = partition_components(&sheet.cells, &sheet.relationships, &active);

        assert_eq!(components.len(), 2);
        let comp1: HashSet<_> = [rel1].into_iter().collect();
        let comp2: HashSet<_> = [rel2].into_iter().collect();
        assert!(
            (components[0] == comp1 && components[1] == comp2)
                || (components[0] == comp2 && components[1] == comp1)
        );
    }
}
```

(Note: `HashSet<T>` does not implement `Hash`, so a `HashSet<HashSet<RelationshipId>>` does
not compile — the assertion above checks both valid orderings directly instead.)

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p adam-rs --lib planner::stay`
Expected: FAIL to compile — `has_self_reference` and `partition_components` are not defined.

- [ ] **Step 4: Implement `has_self_reference` and `partition_components`**

Add above the `#[cfg(test)]` block in `adam-rs/src/planner/stay.rs`:

```rust
/// Returns `true` if any of `rel`'s methods is self-referencing (some cell appears in
/// both its `inputs` and `outputs`).
pub(crate) fn has_self_reference(rel: &RelationshipData) -> bool {
    rel.methods
        .iter()
        .any(|m| m.inputs.iter().any(|i| m.outputs.contains(i)))
}

/// Partitions `active` into its connected components under cell-sharing: two
/// relationships are in the same component iff they share a cell, transitively. A
/// self-referencing chain overlapping a functional diamond via a shared cell always
/// lands in one component — see the design doc's rationale for why no separate cascade
/// step is needed on top of this.
///
/// - Complexity: O(R + sum of adjacency sizes) via breadth-first search.
pub(crate) fn partition_components(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> Vec<HashSet<RelationshipId>> {
    let mut visited: HashSet<RelationshipId> = HashSet::new();
    let mut components = Vec::new();

    for &start in active {
        if visited.contains(&start) {
            continue;
        }
        let mut component = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(start);
        visited.insert(start);

        while let Some(rel_id) = queue.pop_front() {
            component.insert(rel_id);
            for &cell_id in &relationships[rel_id].adj {
                for &neighbor in &cells[cell_id].adj {
                    if active.contains(&neighbor) && visited.insert(neighbor) {
                        queue.push_back(neighbor);
                    }
                }
            }
        }
        components.push(component);
    }
    components
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p adam-rs --lib planner::stay`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add adam-rs/src/planner.rs adam-rs/src/planner/stay.rs
git commit -m "feat(adam-rs): add stay module skeleton (has_self_reference, partition_components)"
```

---

## Task 3: `stay::resolve_component` — score and select

**Files:**
- Modify: `adam-rs/src/planner/stay.rs`

**Interfaces:**
- Consumes: `matching::Assignment::solve_acyclic_all` (Task 1), `digraph::{Node, build_digraph}`, `scc::tarjan_scc` (existing `pub(crate)` items).
- Produces: `resolve_component(cells: &SlotMap<CellId, CellData>, relationships: &SlotMap<RelationshipId, RelationshipData>, component: &HashSet<RelationshipId>) -> Option<Assignment>` — used by Task 4's `release::resolve`.

- [ ] **Step 1: Write the failing tests**

Add to `adam-rs/src/planner/stay.rs`'s `mod tests` block:

```rust
    #[test]
    fn resolve_component_prefers_the_consistent_edit_over_the_higher_strength_cell() {
        // Issue #182: a<=b<=c. Writing a=25 then c=40 is jointly consistent (any
        // b in [25,40] satisfies a<=b<=c), so a's edit must survive even though c's
        // strength (written last) is higher than a's.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(10_i32);
        let b = sheet.add_cell(20_i32);
        let c = sheet.add_cell(30_i32);
        let rel1 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([a, b], b, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        let rel2 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([b, c], b, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([b, c], c, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        sheet.write(a, 25_i32).unwrap();
        sheet.write(c, 40_i32).unwrap();

        let component: HashSet<_> = [rel1, rel2].into_iter().collect();
        let assignment = resolve_component(&sheet.cells, &sheet.relationships, &component)
            .expect("a valid acyclic assignment exists");

        assert!(
            !assignment.claimed.contains_key(&a),
            "a must remain the literal source"
        );
        assert_eq!(assignment.claimed[&b], rel1);
        assert_eq!(assignment.claimed[&c], rel2);
    }

    #[test]
    fn resolve_component_matches_todays_choice_when_all_candidates_honor_their_stays() {
        // The discriminating case from issue #182: a=11, b=20 (declared), c=100 already
        // satisfy a<=b<=c, so every candidate source violates nothing -- the tie-break
        // must still pick c (highest strength), matching today's behavior exactly.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(10_i32);
        let b = sheet.add_cell(20_i32);
        let c = sheet.add_cell(30_i32);
        let rel1 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([a, b], b, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        let rel2 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([b, c], b, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([b, c], c, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        sheet.write(a, 11_i32).unwrap();
        sheet.write(c, 0_i32).unwrap();
        sheet.write(c, 100_i32).unwrap();

        let component: HashSet<_> = [rel1, rel2].into_iter().collect();
        let assignment = resolve_component(&sheet.cells, &sheet.relationships, &component)
            .expect("a valid acyclic assignment exists");

        assert!(
            !assignment.claimed.contains_key(&c),
            "c must remain the literal source, matching today's behavior"
        );
        assert_eq!(assignment.claimed[&a], rel1);
        assert_eq!(assignment.claimed[&b], rel2);
    }

    #[test]
    fn resolve_component_treats_a_candidate_execution_error_as_worst_case() {
        // rel's method claiming q always errors; its only alternative (claiming p)
        // always succeeds and honors p's stay. Even though q has higher strength than p
        // (so today's plain algorithm would prefer releasing q, choosing the method that
        // ALWAYS ERRORS), resolve_component must still pick the method claiming p.
        let mut sheet = Sheet::new();
        let p = sheet.add_cell(5_i32);
        let q = sheet.add_cell(50_i32);
        sheet.write(q, 50_i32).unwrap();
        let rel = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([p, q], p, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::new(
                    vec![p, q],
                    vec![q],
                    vec![std::any::TypeId::of::<i32>(), std::any::TypeId::of::<i32>()],
                    vec![std::any::TypeId::of::<i32>()],
                    |_args| Err(anyhow::anyhow!("boom")),
                ),
            ])
            .unwrap();

        let component: HashSet<_> = [rel].into_iter().collect();
        let assignment = resolve_component(&sheet.cells, &sheet.relationships, &component)
            .expect("a valid acyclic assignment exists");

        assert_eq!(assignment.claimed[&p], rel);
        assert!(!assignment.claimed.contains_key(&q));
    }

    #[test]
    fn resolve_component_generalizes_to_a_four_cell_chain() {
        // a<=b<=c<=d via 3 relationships. Writing a=25 (b,c,d untouched since creation)
        // then d=999 makes d the highest strength; today's plain algorithm would
        // (wrongly) keep d as the sole source, overwriting a's edit via the chain
        // (a would end up min'd down to b=20). The value-aware choice must instead keep
        // a as the source (sacrificing only b's untouched, lowest-strength stay).
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(10_i32);
        let b = sheet.add_cell(20_i32);
        let c = sheet.add_cell(30_i32);
        let d = sheet.add_cell(40_i32);
        let rel1 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([a, b], b, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        let rel2 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([b, c], b, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([b, c], c, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        let rel3 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([c, d], c, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([c, d], d, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        sheet.write(a, 25_i32).unwrap();
        sheet.write(d, 999_i32).unwrap();

        let component: HashSet<_> = [rel1, rel2, rel3].into_iter().collect();
        let assignment = resolve_component(&sheet.cells, &sheet.relationships, &component)
            .expect("a valid acyclic assignment exists");

        assert!(
            !assignment.claimed.contains_key(&a),
            "a must remain the literal source, sacrificing only b's untouched stay"
        );
    }

    #[test]
    fn resolve_component_never_scores_a_functional_output_as_a_violated_stay() {
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(5_i32);
        let p = sheet.add_cell(3_i32);
        let y = sheet.add_cell(10_i32);
        sheet.write(y, 10_i32).unwrap(); // bump y's strength above x and p; value unchanged

        let rel1 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([x, y], x, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([x, y], y, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        // Purely functional (non-self-referencing) pair sharing y with rel1.
        let rel2 = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(y, p, |y: &i32| Ok(*y + 1)),
                Method::from_fn_1_1(p, y, |p: &i32| Ok(*p - 1)),
            ])
            .unwrap();

        let component: HashSet<_> = [rel1, rel2].into_iter().collect();
        let assignment = resolve_component(&sheet.cells, &sheet.relationships, &component)
            .expect("a valid acyclic assignment exists");

        // x=5 <= y=10 already holds, so rel1 carries zero self-referencing violations
        // either way, and rel2 is never self-referencing, so every candidate has zero
        // true violations; y's strength (highest) must decide the tie-break. If a
        // functional output were wrongly scored, rel2 claiming p (derived value 11 vs
        // its stored 3) would be spuriously penalized and this would fail.
        assert!(
            !assignment.claimed.contains_key(&y),
            "y must remain the literal source"
        );
        assert_eq!(assignment.claimed[&x], rel1);
        assert_eq!(assignment.claimed[&p], rel2);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs --lib planner::stay`
Expected: FAIL to compile — `resolve_component` is not defined.

- [ ] **Step 3: Implement `score_candidate` and `resolve_component`**

Add above the `#[cfg(test)]` block in `adam-rs/src/planner/stay.rs`:

```rust
/// Executes `assignment`'s chosen methods against `cells`' current values, without
/// mutating `cells`. Returns the sorted-descending strengths of every self-referencing
/// output whose tentative value differs from that cell's own `source` (a violated
/// stay), or `None` if any method returns `Err`.
///
/// Mirrors `Sheet::execute_plan`'s input rule exactly: a self-referencing input always
/// reads the cell's real `source`, never a tentative value from this same scoring pass;
/// every other input reads a prior method's tentative output if this pass already
/// produced one, else the cell's real current `effective()` value. A purely functional
/// (non-self-referencing) output is executed, since a downstream method may depend on
/// it, but never scored — only a self-referencing output has a stay to violate.
///
/// - Precondition: `assignment`'s induced digraph (`digraph::build_digraph`) is acyclic.
///
/// - Complexity: O(R · K) where R = relationships in `assignment.chosen`, K = cells per
///   method.
fn score_candidate(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    assignment: &Assignment,
) -> Option<Vec<u64>> {
    let adj = digraph::build_digraph(assignment, relationships);
    let mut components = scc::tarjan_scc(&adj);
    components.reverse();

    let mut overlay: HashMap<CellId, Box<dyn Any>> = HashMap::new();
    let mut violated: Vec<u64> = Vec::new();

    for component in components {
        debug_assert_eq!(component.len(), 1, "assignment must already be acyclic");
        let Node::Relationship(rel_id) = component[0] else {
            continue;
        };
        let method_idx = assignment.chosen[&rel_id];
        let method = &relationships[rel_id].methods[method_idx];

        let inputs: Vec<&dyn Any> = method
            .inputs
            .iter()
            .map(|&id| {
                if method.outputs.contains(&id) {
                    cells[id].source.as_ref()
                } else {
                    overlay
                        .get(&id)
                        .map(|v| v.as_ref())
                        .unwrap_or_else(|| cells[id].effective())
                }
            })
            .collect();

        let outputs = (method.function)(&inputs).ok()?;
        if outputs.len() != method.outputs.len() {
            return None;
        }

        for (&output_id, value) in method.outputs.iter().zip(outputs) {
            if method.inputs.contains(&output_id) {
                let cell = &cells[output_id];
                if !(cell.eq_fn)(value.as_ref(), cell.source.as_ref()) {
                    violated.push(cell.strength);
                }
            }
            overlay.insert(output_id, value);
        }
    }

    violated.sort_unstable_by(|a, b| b.cmp(a));
    Some(violated)
}

/// Chooses the value-aware optimal assignment for one connected component containing at
/// least one self-referencing method: enumerates every structurally valid acyclic
/// assignment for `component` ([`matching::Assignment::solve_acyclic_all`]), scores each
/// by concretely executing it against `cells`' current values ([`score_candidate`]), and
/// returns the one whose violated self-referencing stays, sorted descending, are
/// lexicographically smallest — breaking ties by today's rule of lexicographically
/// largest sorted source-set strengths. A candidate whose execution errors is scored as
/// worst-case (`vec![u64::MAX]`) rather than excluded outright, so it can still win the
/// tie-break fallback if every candidate errors.
///
/// - Precondition: every relationship in `component` is present in `relationships`.
///
/// - Postcondition: returns `None` iff no structurally valid acyclic assignment exists
///   for `component` at all, mirroring [`super::release::ReleaseFailure`] (the caller
///   distinguishes `NoAssignment` from `NoAcyclicAssignment` the same way
///   `release::resolve` already does for the non-self-referencing path).
///
/// - Complexity: O(N · R·K) to score N candidates found by
///   [`matching::Assignment::solve_acyclic_all`] (R = relationships in `component`, K =
///   cells per method), on top of that function's own exponential-in-R worst-case
///   search.
pub(crate) fn resolve_component(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    component: &HashSet<RelationshipId>,
) -> Option<Assignment> {
    let candidates = matching::Assignment::solve_acyclic_all(relationships, component, &HashSet::new());

    let source_cells: HashSet<CellId> = component
        .iter()
        .flat_map(|&rel_id| relationships[rel_id].adj.iter().copied())
        .collect();

    candidates
        .into_iter()
        .map(|assignment| {
            let violated =
                score_candidate(cells, relationships, &assignment).unwrap_or(vec![u64::MAX]);
            let mut source_strengths: Vec<u64> = source_cells
                .iter()
                .filter(|c| !assignment.claimed.contains_key(c))
                .map(|&c| cells[c].strength)
                .collect();
            source_strengths.sort_unstable_by(|a, b| b.cmp(a));
            (violated, Reverse(source_strengths), assignment)
        })
        .min_by(|(v1, s1, _), (v2, s2, _)| v1.cmp(v2).then_with(|| s1.cmp(s2)))
        .map(|(_, _, assignment)| assignment)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-rs --lib planner::stay`
Expected: PASS (9 tests total in this module).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-rs/src/planner/stay.rs
git commit -m "feat(adam-rs): add stay::resolve_component (value-aware source selection)"
```

---

## Task 4: Wire `release::resolve` to dispatch by component

**Files:**
- Modify: `adam-rs/src/planner/release.rs` (full rewrite of the module doc comment and the `resolve` function; `resolve_plain` is `resolve`'s old body, renamed)
- Modify: `adam-rs/src/planner.rs:1-26` (one sentence added to the module doc comment)

**Interfaces:**
- Consumes: `stay::{has_self_reference, partition_components, resolve_component}` (Tasks 2–3).
- Produces: `release::resolve` keeps its existing public signature and `ReleaseFailure` contract — no change visible to `planner::plan` (its only caller).

- [ ] **Step 1: Confirm the existing regression baseline passes before changing anything**

Run: `cargo test -p adam-rs --lib planner::release`
Expected: PASS (the 4 existing tests: `no_assignment_returns_no_assignment_failure`, `genuinely_unsolvable_cycle_returns_no_acyclic_assignment_failure`, `strength_prefers_the_higher_strength_cell_as_source`, `diamond_collision_pattern_resolves_instead_of_failing`).

- [ ] **Step 2: Write the failing tests**

Add to `adam-rs/src/planner/release.rs`'s existing `mod tests` block:

```rust
    #[test]
    fn resolve_prefers_the_consistent_edit_over_the_higher_strength_cell() {
        // The issue #182 shape, exercised through the public resolve() dispatch (not
        // stay::resolve_component directly) to confirm the partition/merge wiring
        // itself routes a self-referencing component to the value-aware path.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(10_i32);
        let b = sheet.add_cell(20_i32);
        let c = sheet.add_cell(30_i32);
        let rel1 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([a, b], b, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        let rel2 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([b, c], b, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([b, c], c, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        sheet.write(a, 25_i32).unwrap();
        sheet.write(c, 40_i32).unwrap();

        let active: HashSet<_> = [rel1, rel2].into_iter().collect();
        let assignment = resolve(&sheet.cells, &sheet.relationships, &active).unwrap();

        assert!(
            !assignment.claimed.contains_key(&a),
            "a must remain the literal source"
        );
    }

    #[test]
    fn resolve_merges_independent_plain_and_value_aware_components() {
        // A self-referencing pair (x,y) fully disjoint from a functional diamond
        // (p,q,r): each must be resolved by its own algorithm and merged without
        // interference.
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(5_i32);
        let y = sheet.add_cell(10_i32);
        let rel1 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([x, y], x, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([x, y], y, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();

        let p = sheet.add_cell(0.0_f64);
        let q = sheet.add_cell(0.0_f64);
        let r = sheet.add_cell(0.0_f64);
        let rel2 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([p, q], r, |a: &f64, b: &f64| Ok(a * b)),
                Method::from_fn_2_1([p, r], q, |a: &f64, b: &f64| Ok(b / a)),
                Method::from_fn_2_1([q, r], p, |a: &f64, b: &f64| Ok(b / a)),
            ])
            .unwrap();
        sheet.write(p, 2.0).unwrap();
        sheet.write(q, 3.0).unwrap();

        let active: HashSet<_> = [rel1, rel2].into_iter().collect();
        let assignment = resolve(&sheet.cells, &sheet.relationships, &active).unwrap();

        assert_eq!(assignment.chosen.len(), 2);
        assert_eq!(
            assignment.claimed[&r], rel2,
            "the functional diamond must still pick r as the derived cell"
        );
    }

    #[test]
    fn resolve_reports_no_assignment_when_any_component_lacks_one_even_if_another_only_fails_acyclically() {
        // A plain component that's a genuine algebraic loop (x=f(y); y=g(x), single
        // method each, no self-reference) -- NoAcyclicAssignment on its own, mirroring
        // genuinely_unsolvable_cycle_returns_no_acyclic_assignment_failure -- alongside a
        // disjoint self-referencing component whose two relationships both insist on
        // claiming the same cell with no alternative method -- NoAssignment on its own.
        // The aggregate failure must be NoAssignment, matching the pre-partition
        // monolithic algorithm's semantics (a full assignment exists iff every disjoint
        // component has one), not NoAcyclicAssignment just because resolve_plain --
        // checked first -- only sees its own component's cyclic-only failure.
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(0_i32);
        let y = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(y, x, |v: &i32| Ok(*v + 1))])
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(x, y, |v: &i32| Ok(*v + 1))])
            .unwrap();

        let a = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, a, |x: &i32| Ok((*x).min(0)))])
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, a, |x: &i32| Ok((*x).max(0)))])
            .unwrap();

        let active: HashSet<_> = sheet.relationships().collect();
        let result = resolve(&sheet.cells, &sheet.relationships, &active);

        assert!(matches!(result, Err(ReleaseFailure::NoAssignment)));
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p adam-rs --lib planner::release`
Expected: FAIL — `resolve_prefers_the_consistent_edit_over_the_higher_strength_cell` fails its assertion (reproduces the bug through the current, unmodified `resolve`); `resolve_merges_independent_plain_and_value_aware_components` may pass already (it doesn't yet exercise any self-referencing dispatch); `resolve_reports_no_assignment_when_any_component_lacks_one_even_if_another_only_fails_acyclically` should fail (the current, unmodified `resolve` has no self-referencing dispatch at all, so this scenario instead exercises the single monolithic path and needs re-verification once the dispatch/aggregation logic in Step 4 exists). All three must be re-checked once Step 4 lands.

- [ ] **Step 4: Replace `release.rs`'s doc comment, `resolve`, and rename the old body to `resolve_plain`**

Replace the module doc comment at the top of `adam-rs/src/planner/release.rs` (everything from the first `//!` line down to the last one before `use std::cmp::Reverse;`) with:

```rust
//! Chooses which cells are sources.
//!
//! [`resolve`] partitions the active relationship set into connected components
//! ([`super::stay::partition_components`]). A component with no self-referencing method
//! is resolved by [`resolve_plain`]: greedily releasing cells in descending strength
//! order, checking at each step whether a matching + acyclic assignment still exists
//! with that cell (and every previously released cell) forbidden from being claimed. A
//! component containing at least one self-referencing method is instead resolved by
//! [`super::stay::resolve_component`], which compares candidates by the concrete values
//! they would produce rather than by strength alone — plain strength-lexicographic
//! release is not value-safe once a method can read one of its own outputs; see
//! `docs/superpowers/specs/2026-09-07-adam-rs-value-aware-self-ref-planning-design.md`.
//!
//! This module has no visibility into a filter's dynamic-argument dependencies —
//! `digraph::add_filter_edges` adds those edges to the digraph only *after* `resolve` has
//! already finished searching (see
//! `docs/superpowers/specs/2026-08-25-adam-rs-filter-revalidation-design.md` §3).
//! `resolve`'s acyclicity guarantee therefore holds only for the relationship-only
//! subgraph; `plan()` re-checks acyclicity once more after filter edges are added,
//! returning `Error::FilterCycle` (distinct from this module's own `Error::Cycle`)
//! if that combined graph turns out cyclic. Generalizing `resolve` itself to search
//! around filter edges is tracked as issue #153.
```

Immediately below that doc comment, change the existing import:

```rust
use super::matching::Assignment;
```

to:

```rust
use super::matching::Assignment;
use super::stay;
```

Then replace the existing `resolve` function (everything from its doc comment through its closing `}`, i.e. from `/// Finds the strength-optimal acyclic assignment...` through the line before `#[cfg(test)]`) with:

```rust
/// Finds the optimal acyclic assignment for `active`, dispatching each connected
/// component ([`stay::partition_components`]) to whichever algorithm applies: a
/// component with no self-referencing method is resolved by [`resolve_plain`]
/// (unchanged strength-lexicographic release); a component containing at least one is
/// resolved by [`stay::resolve_component`] (value-aware release). The two never
/// interact, since components are disjoint by construction, so their results merge
/// directly.
///
/// Failure precedence is computed across *every* component before returning, matching
/// the single monolithic pre-partition algorithm's semantics: since components are
/// cell-disjoint, a full assignment over `active` exists iff one exists for every
/// individual component, so [`ReleaseFailure::NoAssignment`] takes precedence over
/// [`ReleaseFailure::NoAcyclicAssignment`] even when a *different* component is the one
/// that failed acyclicity — checking only the first failing component (in partition
/// order) would report a less severe failure than the sheet as a whole actually has.
///
/// # Errors
///
/// - [`ReleaseFailure::NoAssignment`] — no method assignment exists at all for some
///   component, cyclic or not.
/// - [`ReleaseFailure::NoAcyclicAssignment`] — a method assignment exists for every
///   component, but at least one component admits no acyclic assignment.
///
/// - Complexity: see [`resolve_plain`] and [`stay::resolve_component`]; components are
///   independent, so their costs add rather than multiply.
pub(crate) fn resolve(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> Result<Assignment, ReleaseFailure> {
    let mut plain: HashSet<RelationshipId> = HashSet::new();
    let mut value_aware: Vec<HashSet<RelationshipId>> = Vec::new();
    for component in stay::partition_components(cells, relationships, active) {
        if component
            .iter()
            .any(|&rel_id| stay::has_self_reference(&relationships[rel_id]))
        {
            value_aware.push(component);
        } else {
            plain.extend(component);
        }
    }

    let plain_result = resolve_plain(cells, relationships, &plain);
    let component_results: Vec<Result<Assignment, ReleaseFailure>> = value_aware
        .iter()
        .map(|component| {
            stay::resolve_component(cells, relationships, component).ok_or_else(|| {
                if Assignment::solve(relationships, component, &HashSet::new()).is_some() {
                    ReleaseFailure::NoAcyclicAssignment
                } else {
                    ReleaseFailure::NoAssignment
                }
            })
        })
        .collect();

    let any_no_assignment = matches!(plain_result, Err(ReleaseFailure::NoAssignment))
        || component_results
            .iter()
            .any(|r| matches!(r, Err(ReleaseFailure::NoAssignment)));
    if any_no_assignment {
        return Err(ReleaseFailure::NoAssignment);
    }

    let mut assignment = plain_result?;
    for result in component_results {
        let component_assignment = result?;
        assignment.chosen.extend(component_assignment.chosen);
        assignment.claimed.extend(component_assignment.claimed);
    }

    Ok(assignment)
}

/// Finds the strength-optimal acyclic assignment for a relationship set containing no
/// self-referencing method: an [`Assignment`] where the set of cells left unclaimed
/// (sources) is lexicographically maximal in descending strength order among all
/// assignments whose induced digraph is acyclic.
///
/// Processes every cell in descending strength order, tentatively adding it to the
/// forbidden set and searching for an assignment that is both valid (no double claims)
/// and acyclic with that cell -- and every previously accepted release -- forbidden from
/// being claimed ([`Assignment::solve_acyclic`]). The release is kept only when such an
/// assignment exists. This single mechanism handles both ordinary strength-based method
/// selection (an uncontested relationship's choice of which cell to leave exogenous)
/// and cyclic ("diamond") resolution uniformly -- both are just instances of "does
/// releasing this cell still admit a valid acyclic assignment".
///
/// Every cell is re-checked this way, even one that happens not to be claimed by the
/// current best assignment: a cell being currently unclaimed is an artifact of
/// `solve_acyclic`'s deterministic method-choice order, not proof that leaving it a
/// source is compatible with releasing every higher-strength cell still to come, so it
/// cannot be adopted as released without the same check every other cell gets.
///
/// # Errors
///
/// - [`ReleaseFailure::NoAssignment`] -- no method assignment exists at all, cyclic or
///   not.
/// - [`ReleaseFailure::NoAcyclicAssignment`] -- a method assignment exists, but none of
///   them is acyclic.
///
/// - Complexity: O(C · `solve_acyclic`) where C = cells -- each cell triggers one
///   `solve_acyclic` attempt, itself exponential in the number of active relationships
///   in the worst case (see its own doc comment).
fn resolve_plain(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> Result<Assignment, ReleaseFailure> {
    let mut released: HashSet<CellId> = HashSet::new();
    let Some(mut current) = Assignment::solve_acyclic(relationships, active, &released) else {
        return Err(
            if Assignment::solve(relationships, active, &released).is_some() {
                ReleaseFailure::NoAcyclicAssignment
            } else {
                ReleaseFailure::NoAssignment
            },
        );
    };

    let mut cells_sorted: Vec<CellId> = cells.keys().collect();
    cells_sorted.sort_by_key(|&id| Reverse(cells[id].strength));

    for cell in cells_sorted {
        let mut candidate_released = released.clone();
        candidate_released.insert(cell);

        if let Some(candidate) =
            Assignment::solve_acyclic(relationships, active, &candidate_released)
        {
            released = candidate_released;
            current = candidate;
        }
    }

    Ok(current)
}
```

- [ ] **Step 5: Update `planner.rs`'s module doc for accuracy**

In `adam-rs/src/planner.rs`, in the module doc comment, change this sentence:

```rust
//! The planner finds the strength-optimal acyclic assignment of methods to
//! relationships: [`release::resolve`] greedily tries, in descending cell-strength
//! order, to leave each cell unclaimed (a source), keeping the change only when a
//! valid method assignment still exists ([`matching::Assignment::solve`]) *and* its
//! induced dependency digraph is acyclic ([`digraph::is_acyclic`]).
```

to:

```rust
//! The planner finds the strength-optimal acyclic assignment of methods to
//! relationships: for a connected component with no self-referencing method,
//! [`release::resolve`] greedily tries, in descending cell-strength order, to leave
//! each cell unclaimed (a source), keeping the change only when a valid method
//! assignment still exists ([`matching::Assignment::solve`]) *and* its induced
//! dependency digraph is acyclic ([`digraph::is_acyclic`]). A component containing a
//! self-referencing method is instead resolved by `stay::resolve_component`'s
//! value-aware comparison — see
//! `docs/superpowers/specs/2026-09-07-adam-rs-value-aware-self-ref-planning-design.md`.
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p adam-rs --lib planner`
Expected: PASS — every `planner.rs`/`matching.rs`/`digraph.rs`/`scc.rs`/`stay.rs`/`release.rs` test, including the 3 new `release.rs` tests and the 4 pre-existing `release.rs` tests (unchanged behavior for the plain path).

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add adam-rs/src/planner.rs adam-rs/src/planner/release.rs
git commit -m "fix(adam-rs): dispatch self-referencing components to value-aware release (fixes #182)"
```

---

## Task 5: End-to-end `Sheet`-level regression tests

**Files:**
- Modify: `adam-rs/tests/integration.rs` (replace the two ad hoc verification tests already appended during investigation with final, polished versions)

- [ ] **Step 1: Locate and remove the ad hoc verification tests**

Open `adam-rs/tests/integration.rs` and find the two tests named `issue_182_repro_inequality_strength_bug` and `issue_182_repro_inequality_discriminating_case` (appended at the end of the file during initial investigation, including `eprintln!` debug lines). Delete both in full.

- [ ] **Step 2: Write the final regression tests**

Add to the end of `adam-rs/tests/integration.rs`:

```rust
#[test]
fn issue_182_inequality_chain_preserves_a_consistent_edit() {
    // a<=b<=c via two self-referencing relationships (the inequality.adm2 tutorial
    // shape). Writing a=25 then c=40 is jointly consistent (any b in [25,40] satisfies
    // a<=b<=c), so both edits must survive across two separate propagate() rounds.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(10_i32);
    let b = sheet.add_cell(20_i32);
    let c = sheet.add_cell(30_i32);
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

    sheet.propagate().unwrap();
    sheet.write(a, 25_i32).unwrap();
    sheet.propagate().unwrap();
    sheet.write(c, 40_i32).unwrap();
    sheet.propagate().unwrap();

    assert_eq!(*sheet.read::<i32>(a).unwrap(), 25);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 25);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 40);
}

#[test]
fn issue_182_inequality_chain_discriminating_case_unchanged() {
    // The case that must stay correct: stays are already consistent (11 <= 20 <= 100),
    // so nothing needs to move.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(10_i32);
    let b = sheet.add_cell(20_i32);
    let c = sheet.add_cell(30_i32);
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

    sheet.propagate().unwrap();
    sheet.write(a, 11_i32).unwrap();
    sheet.propagate().unwrap();
    sheet.write(c, 0_i32).unwrap();
    sheet.propagate().unwrap();
    sheet.write(c, 100_i32).unwrap();
    sheet.propagate().unwrap();

    assert_eq!(*sheet.read::<i32>(a).unwrap(), 11);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 20);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 100);
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p adam-rs --test integration issue_182`
Expected: PASS (2 tests).

- [ ] **Step 4: Run the full `adam-rs` test suite**

Run: `cargo test -p adam-rs`
Expected: PASS — every unit and integration test in the crate.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-rs/tests/integration.rs
git commit -m "test(adam-rs): add end-to-end regression tests for issue #182"
```

---

## Task 6: Document sheet invariants in `adam-rs/src/lib.rs`

**Files:**
- Modify: `adam-rs/src/lib.rs`

- [ ] **Step 1: Add the `## Invariants` section**

In `adam-rs/src/lib.rs`, insert the following new section into the crate-level doc comment, directly after the existing `# Filters` section's closing code block (i.e., after the line `//! assert!(sheet.filter_violated_cells().any(|id| id == b));` and its closing ` ```/// ` fence, and before the `pub mod cell;` line):

```rust
//!
//! # Invariants
//!
//! Enforced by code:
//!
//! - Every relationship's methods share the same `inputs ∪ outputs` cell set
//!   ([`Sheet::add_relationship`] validation; [`Error::MismatchedMethodCells`]).
//! - No two relationships may claim the same cell as a pure output in one round
//!   ([`Error::Conflict`] when infeasible).
//! - The selected methods' induced dependency digraph is acyclic before execution
//!   ([`Error::Cycle`]/[`Error::FilterCycle`] when not).
//! - A self-referencing input always reads the pre-round `source` value, never a
//!   same-round derived value.
//! - `source` is written only by [`Sheet::write`]/[`Sheet::add_cell`], never by method
//!   or filter execution.
//! - An `Out`-kind cell can never be [`Sheet::write`]-ed or claimed as another method's
//!   output ([`Error::InvalidCellKind`]).
//!
//! Enforced only by convention (caller contract, not checked by the runtime):
//!
//! - A self-referencing method must be idempotent: applying it twice to the same inputs
//!   must produce the same result as applying it once.
//! - A filter must be a pure, conforming function of its cell's value and its argument
//!   cells' values.
//! - Iteration order used to break ties among equal-strength cells is not stable API
//!   and must not be relied on by callers.
//!
//! The source-capture property, amended:
//!
//! Capturing every cell's `source` value and reapplying the highest-strength sources
//! reconstructs the sheet's *current* state, but not what a subsequent edit will do,
//! since no method-selection state is captured, only values. This property does not
//! hold as stated for a self-referencing cell: because which method a relationship
//! selects is chosen by strength, but a strictly weaker self-referenced cell can still
//! contribute to that method's output, the *source* value alone is not sufficient to
//! reconstruct even the current state for such a cell.
//!
//! Amended: for a self-referencing cell, the value that must be captured to reconstruct
//! current state is its *derived* value ([`Sheet::read`]'s effective value), not its raw
//! `source`. Because a self-referencing method is required to be idempotent, replaying
//! that derived value through the same method again is guaranteed to reproduce it: the
//! capture is stable under replay even though it discards the cell's original written
//! value.
```

- [ ] **Step 2: Verify doc examples still build**

Run: `cargo test --doc -p adam-rs`
Expected: PASS — the new section adds no new code blocks, so no new doctests; existing doctests in `# Example`, `# Out cells and requirements`, and `# Filters` are unaffected since nothing above them changed.

Run: `RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps -p adam-rs`
Expected: exit 0 — no broken intra-doc links (`[Sheet::add_relationship]` etc. must resolve; if any link fails, fix the path, e.g. `[Sheet::write]` requires `use crate::sheet::Sheet;` style resolution already established by the rest of this doc comment's existing links).

- [ ] **Step 3: Commit**

```bash
cargo fmt --all
git add adam-rs/src/lib.rs
git commit -m "docs(adam-rs): document sheet invariants, amend source-capture property for issue #182"
```

---

## Task 7: Full verification pass

**Files:** none (verification only; fix any files that produce warnings)

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Expected: no diff (already formatted after each task's commit).

- [ ] **Step 2: Build the whole workspace with zero warnings**

Run: `cargo build --workspace`
Expected: exit 0, no warnings in the output. If any warning appears, fix it before continuing (do not suppress with `#[allow(...)]` unless the warning is a documented false positive).

- [ ] **Step 3: Run the full workspace test suite with zero warnings**

Run: `cargo test --workspace`
Expected: exit 0, no warnings, all tests pass.

Run: `cargo test --doc --workspace`
Expected: exit 0, all doc tests pass.

- [ ] **Step 4: Lint**

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Expected: exit 0.

Run: `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
Expected: exit 0 (this plan touches no `begin` code, but CLAUDE.md requires this check before every PR).

Run: `cargo clippy -p begin --all-targets -- -D warnings`
Expected: exit 0.

- [ ] **Step 5: Docs, as CI checks them**

Run: `RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace`
Expected: exit 0.

- [ ] **Step 6: Fix any warnings found**

If Steps 2–5 surfaced any warning, fix it now (in the file it originated from) and re-run the specific failing command to confirm. Re-run Step 1 (`cargo fmt --all`) after any fix.

- [ ] **Step 7: Final commit (only if Step 6 made changes)**

```bash
git add -A
git commit -m "chore(adam-rs): fix warnings surfaced by full verification pass"
```

If Step 6 made no changes, skip this commit — there is nothing to commit.
