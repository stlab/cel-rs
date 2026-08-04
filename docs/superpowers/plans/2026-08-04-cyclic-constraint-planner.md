# Cyclic Constraint Planner Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `adam-rs`'s greedy strength-ordered flood-fill planner with a
bipartite-matching + cycle-checking algorithm that correctly resolves overlapping
cyclic relationship structures (the "diamond" pattern in `begin/examples/diamond.adm2`),
which the current planner spuriously reports as `Error::Conflict`.

**Architecture:** A new `Assignment::solve` (bipartite/hypergraph matching via a
recursive augmenting-path search) finds *some* valid method-per-relationship
assignment, optionally forbidding specific cells from being claimed as outputs. A new
`release::resolve` greedily tries, in descending cell-strength order, to add each
currently-claimed cell to the forbidden set — keeping the change only if a matching
still exists *and* its induced dependency digraph (checked via a new generic Tarjan SCC
in `scc.rs`) is acyclic. This single mechanism handles both today's ordinary
strength-based method selection and diamond-style cyclic conflicts uniformly, since
both reduce to "does releasing this cell still admit a valid acyclic assignment?" The
existing `forced_output_cells` fixpoint is kept unchanged (only its private
`pure_outputs` helper is deduplicated into the new `matching` module) since it is
already correct and populates a separate, orthogonal part of the public API
(`Plan.forced_outputs` / `Plan.forced_relationships`).

**Tech Stack:** Rust, `slotmap` (already a dependency), `std::collections`. No new
crate dependencies.

## Global Constraints

- `cargo fmt --all` must be run before every commit (enforced by the pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce **zero compiler
  warnings**.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` must pass
  (plus the two `begin`-specific clippy invocations, unaffected by this work but part
  of the required full check suite before any PR).
- Every function needs a contract-style `///` doc comment (Summary, Preconditions via
  `debug_assert!`, Postconditions, Complexity when not O(1)) per the project's
  `CLAUDE.md`.
- Arithmetic on signed integers must use `checked_*`; not applicable to this work (no
  new arithmetic is introduced), noted for completeness.
- All existing `adam-rs` tests (unit tests in `planner.rs`, all tests in
  `adam-rs/tests/integration.rs`) must continue to pass unchanged — they assert
  observable contracts, not implementation details, and double as regression tests for
  this rewrite.
- Design reference: `docs/superpowers/specs/2026-08-04-cyclic-constraint-planner-design.md`.

---

## File Structure

- **Create** `adam-rs/src/planner/scc.rs` — generic Tarjan strongly-connected-components
  algorithm, reusable for any node type.
- **Create** `adam-rs/src/planner/matching.rs` — `pure_outputs` helper (moved out of
  `planner.rs`) and `Assignment`, the bipartite/hypergraph matcher.
- **Create** `adam-rs/src/planner/digraph.rs` — builds the planner's cell/relationship
  dependency digraph from an `Assignment` and checks acyclicity.
- **Create** `adam-rs/src/planner/release.rs` — the greedy strength-ordered release loop
  that turns a matching into the strength-optimal *acyclic* assignment.
- **Modify** `adam-rs/src/planner.rs` — replace the flood-fill body of `plan()` with the
  new pipeline; keep `Plan`, keep `forced_output_cells` (importing `pure_outputs` from
  `matching` instead of a private copy); keep the existing `#[cfg(test)] mod tests`
  unchanged; declare the four new submodules.
- **Modify** `adam-rs/tests/integration.rs` — add the diamond-shaped regression tests
  from the design's Testing section.

---

### Task 1: Generic Tarjan SCC (`planner/scc.rs`)

**Files:**
- Create: `adam-rs/src/planner/scc.rs`
- Test: same file, `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `pub(crate) fn tarjan_scc<N: Copy + Eq + std::hash::Hash>(adj: &HashMap<N, Vec<N>>) -> Vec<Vec<N>>`
  — components in **reverse** topological order (successors' components appear before
  predecessors'; callers wanting forward order must `.reverse()` the result).

- [ ] **Step 1: Write the module with its test suite**

Create `adam-rs/src/planner/scc.rs`:

```rust
//! Generic strongly-connected-components decomposition (Tarjan's algorithm), used by
//! the planner to detect cyclic dependency structures in its induced digraph.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// Computes the strongly connected components of the directed graph described by `adj`
/// (an adjacency map from node to its successors).
///
/// Nodes that only appear as a successor (a value in some `adj` entry) but never as a
/// key are still included as trivial (size-1) components.
///
/// - Postcondition: every node appearing as a key or value in `adj` appears in exactly
///   one returned component.
/// - Postcondition: for any edge `u -> v` where `u` and `v` land in different
///   components, `v`'s component appears **before** `u`'s component in the returned
///   `Vec` (Tarjan's classic reverse-topological output order). Callers that want
///   forward topological order must reverse the result.
///
/// - Complexity: O(V + E) where V = nodes, E = edges.
pub(crate) fn tarjan_scc<N>(adj: &HashMap<N, Vec<N>>) -> Vec<Vec<N>>
where
    N: Copy + Eq + Hash,
{
    struct State<N> {
        index: HashMap<N, usize>,
        lowlink: HashMap<N, usize>,
        on_stack: HashSet<N>,
        stack: Vec<N>,
        next_index: usize,
        components: Vec<Vec<N>>,
    }

    fn strongconnect<N>(v: N, adj: &HashMap<N, Vec<N>>, s: &mut State<N>)
    where
        N: Copy + Eq + Hash,
    {
        s.index.insert(v, s.next_index);
        s.lowlink.insert(v, s.next_index);
        s.next_index += 1;
        s.stack.push(v);
        s.on_stack.insert(v);

        if let Some(successors) = adj.get(&v) {
            for &w in successors {
                if !s.index.contains_key(&w) {
                    strongconnect(w, adj, s);
                    let w_low = s.lowlink[&w];
                    let v_low = s.lowlink[&v];
                    s.lowlink.insert(v, v_low.min(w_low));
                } else if s.on_stack.contains(&w) {
                    let w_idx = s.index[&w];
                    let v_low = s.lowlink[&v];
                    s.lowlink.insert(v, v_low.min(w_idx));
                }
            }
        }

        if s.lowlink[&v] == s.index[&v] {
            let mut component = Vec::new();
            loop {
                let w = s.stack.pop().expect("v's own SCC root is still on stack");
                s.on_stack.remove(&w);
                component.push(w);
                if w == v {
                    break;
                }
            }
            s.components.push(component);
        }
    }

    let mut nodes: Vec<N> = Vec::new();
    let mut seen: HashSet<N> = HashSet::new();
    for (&k, vs) in adj {
        if seen.insert(k) {
            nodes.push(k);
        }
        for &v in vs {
            if seen.insert(v) {
                nodes.push(v);
            }
        }
    }

    let mut state = State {
        index: HashMap::new(),
        lowlink: HashMap::new(),
        on_stack: HashSet::new(),
        stack: Vec::new(),
        next_index: 0,
        components: Vec::new(),
    };

    for node in nodes {
        if !state.index.contains_key(&node) {
            strongconnect(node, adj, &mut state);
        }
    }

    state.components
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellId;
    use slotmap::SlotMap;

    fn cells(n: usize) -> Vec<CellId> {
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        (0..n).map(|_| map.insert(())).collect()
    }

    #[test]
    fn empty_graph_has_no_components() {
        let adj: HashMap<CellId, Vec<CellId>> = HashMap::new();
        assert!(tarjan_scc(&adj).is_empty());
    }

    #[test]
    fn single_node_no_edges_is_trivial_component() {
        let ids = cells(1);
        let mut adj = HashMap::new();
        adj.insert(ids[0], vec![]);
        let components = tarjan_scc(&adj);
        assert_eq!(components, vec![vec![ids[0]]]);
    }

    #[test]
    fn two_cycle_is_one_component() {
        let ids = cells(2);
        let mut adj = HashMap::new();
        adj.insert(ids[0], vec![ids[1]]);
        adj.insert(ids[1], vec![ids[0]]);
        let components = tarjan_scc(&adj);
        assert_eq!(components.len(), 1);
        let comp: HashSet<CellId> = components[0].iter().copied().collect();
        let expected: HashSet<CellId> = ids.iter().copied().collect();
        assert_eq!(comp, expected);
    }

    #[test]
    fn diamond_shape_isolates_shared_cycle() {
        // a -> c, b -> c (R1: a,b -> c); c -> b, d -> b (R2: c,d -> b): b<->c cycle,
        // a and d are trivial (source-only) components.
        let ids = cells(4);
        let (a, b, c, d) = (ids[0], ids[1], ids[2], ids[3]);
        let mut adj = HashMap::new();
        adj.insert(a, vec![c]);
        adj.insert(b, vec![c]);
        adj.insert(c, vec![b]);
        adj.insert(d, vec![b]);
        let components = tarjan_scc(&adj);
        let non_trivial: Vec<&Vec<CellId>> = components.iter().filter(|c| c.len() > 1).collect();
        assert_eq!(non_trivial.len(), 1);
        let cyclic: HashSet<CellId> = non_trivial[0].iter().copied().collect();
        let expected: HashSet<CellId> = [b, c].into_iter().collect();
        assert_eq!(cyclic, expected);
    }

    #[test]
    fn chain_reversed_gives_topological_order() {
        // a -> b -> c (DAG, no cycle): reversed component order should be [a, b, c].
        let ids = cells(3);
        let (a, b, c) = (ids[0], ids[1], ids[2]);
        let mut adj = HashMap::new();
        adj.insert(a, vec![b]);
        adj.insert(b, vec![c]);
        let mut components = tarjan_scc(&adj);
        components.reverse();
        let order: Vec<CellId> = components.into_iter().flatten().collect();
        assert_eq!(order, vec![a, b, c]);
    }
}
```

This file is not yet wired into `lib.rs`/`planner.rs`, so it cannot be compiled/tested
standalone yet — that happens in Task 5 when `mod scc;` is declared. Proceed directly
to Task 2 and 3 before attempting to compile; Task 5's Step 2 is the first point all
four new modules are wired in and can be run.

- [ ] **Step 2: Commit**

```bash
git add adam-rs/src/planner/scc.rs
git commit -m "$(cat <<'EOF'
feat(adam-rs): add generic Tarjan SCC for planner cycle detection

Not yet wired into the module tree -- planner.rs still declares no
submodules until the rewrite lands.
EOF
)"
```

---

### Task 2: Bipartite/hypergraph matching (`planner/matching.rs`)

**Files:**
- Create: `adam-rs/src/planner/matching.rs`
- Test: same file, `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces:
  - `pub(crate) fn pure_outputs(method: &Method) -> HashSet<CellId>`
  - `pub(crate) struct Assignment { pub(crate) chosen: HashMap<RelationshipId, usize>, pub(crate) claimed: HashMap<CellId, RelationshipId> }`
  - `impl Assignment { pub(crate) fn solve(relationships: &SlotMap<RelationshipId, RelationshipData>, active: &HashSet<RelationshipId>, forbidden: &HashSet<CellId>) -> Option<Self> }`

- [ ] **Step 1: Write the module with its test suite**

Create `adam-rs/src/planner/matching.rs`:

```rust
//! Bipartite/hypergraph matching: assigns each active relationship one of its methods
//! such that no two relationships claim the same cell as a pure (non-self-referencing)
//! output, optionally forbidding specific cells from being claimed by anyone at all.

use std::collections::{HashMap, HashSet};

use slotmap::SlotMap;

use crate::{
    cell::CellId,
    relationship::{Method, RelationshipData, RelationshipId},
};

/// Returns the cells `method` writes but does not read: the cells that must not
/// already be determined for the method to be eligible, and that become claimed by
/// whichever relationship selects it.
///
/// Self-referencing cells (present in both `inputs` and `outputs`) are excluded: they
/// are read at their pre-execution value, so a self-referencing method places no
/// exclusive claim on them.
///
/// - Complexity: O(K²) where K = cells per method (`inputs.contains` scans linearly).
pub(crate) fn pure_outputs(method: &Method) -> HashSet<CellId> {
    method
        .outputs
        .iter()
        .filter(|o| !method.inputs.contains(o))
        .copied()
        .collect()
}

enum Change {
    Assigned(RelationshipId, Option<usize>),
    Claimed(CellId, Option<RelationshipId>),
}

/// One method chosen per active relationship, and which relationship currently claims
/// each pure-output cell.
pub(crate) struct Assignment {
    pub(crate) chosen: HashMap<RelationshipId, usize>,
    pub(crate) claimed: HashMap<CellId, RelationshipId>,
}

impl Assignment {
    /// Finds an assignment of one method per relationship in `active` such that no cell
    /// in `forbidden` is claimed as a pure output by anyone, and no two relationships
    /// claim the same cell.
    ///
    /// Relationships are considered in `relationships`' natural (insertion-stable)
    /// order restricted to `active`, so the result is deterministic across calls with
    /// the same inputs.
    ///
    /// Returns `None` if no such assignment exists for any combination of method
    /// choices.
    ///
    /// - Complexity: O(R² · M · K) worst case (R = active relationships, M = methods
    ///   per relationship, K = cells per method): each relationship's assignment search
    ///   may recursively displace up to R-1 others, each doing an O(M·K) scan.
    pub(crate) fn solve(
        relationships: &SlotMap<RelationshipId, RelationshipData>,
        active: &HashSet<RelationshipId>,
        forbidden: &HashSet<CellId>,
    ) -> Option<Self> {
        let mut this = Assignment {
            chosen: HashMap::new(),
            claimed: HashMap::new(),
        };
        let order: Vec<RelationshipId> = relationships.keys().filter(|r| active.contains(r)).collect();
        for rel_id in order {
            if this.chosen.contains_key(&rel_id) {
                continue; // already assigned as a side effect of an earlier displacement
            }
            let mut visited = HashSet::new();
            let mut trail = Vec::new();
            if !this.try_assign(rel_id, relationships, &mut visited, &mut trail, forbidden) {
                return None;
            }
        }
        Some(this)
    }

    /// Attempts to find (and commit) a method for `rel_id` whose pure outputs avoid
    /// `forbidden`, recursively displacing other relationships' claims via augmenting
    /// search when a candidate method's outputs are already claimed. `visited` prevents
    /// re-entering a relationship already being displaced earlier in this same search.
    ///
    /// - Complexity: O(M · (R + K)) per call, recursively bounded by the number of
    ///   distinct relationships in `visited` (at most R).
    fn try_assign(
        &mut self,
        rel_id: RelationshipId,
        relationships: &SlotMap<RelationshipId, RelationshipData>,
        visited: &mut HashSet<RelationshipId>,
        trail: &mut Vec<Change>,
        forbidden: &HashSet<CellId>,
    ) -> bool {
        if !visited.insert(rel_id) {
            return false;
        }

        let rel = &relationships[rel_id];
        for (method_idx, method) in rel.methods.iter().enumerate() {
            let outputs = pure_outputs(method);
            if !outputs.is_disjoint(forbidden) {
                continue;
            }

            let mark = trail.len();
            // While resolving blockers below, nobody (including a displaced blocker)
            // may reclaim one of THIS method's own target outputs -- they're reserved
            // for `rel_id` for the duration of this attempt.
            let mut inner_forbidden = forbidden.clone();
            inner_forbidden.extend(outputs.iter().copied());

            let blockers: HashSet<RelationshipId> = outputs
                .iter()
                .filter_map(|c| self.claimed.get(c).copied())
                .filter(|&r| r != rel_id)
                .collect();

            let mut ok = true;
            for blocker in blockers {
                if visited.contains(&blocker) {
                    // Already resolved (or currently being resolved further up the call
                    // stack) as a side effect of displacing a different blocker in this
                    // same attempt.
                    continue;
                }
                if let Some(&old_idx) = self.chosen.get(&blocker) {
                    let old_outputs = pure_outputs(&relationships[blocker].methods[old_idx]);
                    self.clear_assignment(blocker, trail);
                    for c in old_outputs {
                        if self.claimed.get(&c) == Some(&blocker) {
                            self.clear_claim(c, trail);
                        }
                    }
                }
                if !self.try_assign(blocker, relationships, visited, trail, &inner_forbidden) {
                    ok = false;
                    break;
                }
            }

            if ok {
                for &c in &outputs {
                    self.set_claim(c, rel_id, trail);
                }
                self.set_assignment(rel_id, method_idx, trail);
                return true;
            }

            self.undo(trail, mark);
        }
        false
    }

    fn set_assignment(&mut self, rel: RelationshipId, idx: usize, trail: &mut Vec<Change>) {
        trail.push(Change::Assigned(rel, self.chosen.insert(rel, idx)));
    }

    fn clear_assignment(&mut self, rel: RelationshipId, trail: &mut Vec<Change>) {
        trail.push(Change::Assigned(rel, self.chosen.remove(&rel)));
    }

    fn set_claim(&mut self, cell: CellId, rel: RelationshipId, trail: &mut Vec<Change>) {
        trail.push(Change::Claimed(cell, self.claimed.insert(cell, rel)));
    }

    fn clear_claim(&mut self, cell: CellId, trail: &mut Vec<Change>) {
        trail.push(Change::Claimed(cell, self.claimed.remove(&cell)));
    }

    fn undo(&mut self, trail: &mut Vec<Change>, mark: usize) {
        while trail.len() > mark {
            match trail.pop().expect("loop condition checked len > mark") {
                Change::Assigned(rel, Some(idx)) => {
                    self.chosen.insert(rel, idx);
                }
                Change::Assigned(rel, None) => {
                    self.chosen.remove(&rel);
                }
                Change::Claimed(cell, Some(r)) => {
                    self.claimed.insert(cell, r);
                }
                Change::Claimed(cell, None) => {
                    self.claimed.remove(&cell);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Method, Sheet};

    #[test]
    fn single_relationship_single_method_is_assigned() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let active: HashSet<_> = [rel].into_iter().collect();
        let assignment = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        assert_eq!(assignment.chosen[&rel], 0);
        assert_eq!(assignment.claimed[&b], rel);
        assert!(!assignment.claimed.contains_key(&a));
    }

    #[test]
    fn two_relationships_wanting_the_same_only_output_is_infeasible() {
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
        assert!(Assignment::solve(&sheet.relationships, &active, &HashSet::new()).is_none());
    }

    #[test]
    fn diamond_relationships_admit_a_feasible_assignment() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0.0_f64);
        let b = sheet.add_cell(0.0_f64);
        let c = sheet.add_cell(0.0_f64);
        let d = sheet.add_cell(0.0_f64);
        let r1 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok(x * y)),
                Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok(y / x)),
                Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok(y / x)),
            ])
            .unwrap();
        let r2 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([b, c], d, |x: &f64, y: &f64| Ok(x * y)),
                Method::from_fn_2_1([b, d], c, |x: &f64, y: &f64| Ok(y / x)),
                Method::from_fn_2_1([c, d], b, |x: &f64, y: &f64| Ok(y / x)),
            ])
            .unwrap();
        let active: HashSet<_> = [r1, r2].into_iter().collect();
        let assignment = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        let unique: HashSet<_> = assignment.claimed.values().collect();
        assert_eq!(unique.len(), assignment.claimed.len(), "no two relationships may claim the same cell");
    }

    #[test]
    fn self_referencing_output_does_not_conflict_with_a_different_relationship() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let r1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, a, |x: &i32| Ok((*x).min(0)))])
            .unwrap();
        let r2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        let active: HashSet<_> = [r1, r2].into_iter().collect();
        let assignment = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        assert_eq!(assignment.chosen.len(), 2);
        assert!(!assignment.claimed.contains_key(&a));
        assert_eq!(assignment.claimed[&b], r2);
    }

    #[test]
    fn multi_output_method_claims_all_its_outputs_when_forced() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell("a".to_string());
        let b = sheet.add_cell("b".to_string());
        let c = sheet.add_cell("ab".to_string());
        let rel = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], c, |x: &String, y: &String| Ok(x.clone() + y)),
                Method::new(
                    vec![c],
                    vec![a, b],
                    vec![std::any::TypeId::of::<String>()],
                    vec![std::any::TypeId::of::<String>(), std::any::TypeId::of::<String>()],
                    |args| {
                        let z = args[0].downcast_ref::<String>().unwrap();
                        let mut chars = z.chars();
                        let first = chars.next().unwrap_or_default().to_string();
                        let rest = chars.collect::<String>();
                        Ok(vec![Box::new(first), Box::new(rest)])
                    },
                ),
            ])
            .unwrap();
        let active: HashSet<_> = [rel].into_iter().collect();

        let unconstrained = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        assert_eq!(unconstrained.chosen[&rel], 0);
        assert_eq!(unconstrained.claimed[&c], rel);

        let mut forbidden = HashSet::new();
        forbidden.insert(c);
        let constrained = Assignment::solve(&sheet.relationships, &active, &forbidden).unwrap();
        assert_eq!(constrained.chosen[&rel], 1);
        assert_eq!(constrained.claimed[&a], rel);
        assert_eq!(constrained.claimed[&b], rel);
        assert!(!constrained.claimed.contains_key(&c));
    }

    #[test]
    fn blocker_displacement_falls_back_when_the_cascade_cannot_complete() {
        // R1's only method wants `x`, currently claimed by R2's default method; R2's
        // only alternative wants `y`, currently claimed by R3's default method; R3's
        // only alternative claims `q`, which is free. Exercises multi-level blocker
        // resolution without corrupting state on the way to the final assignment.
        let mut sheet = Sheet::new();
        let p = sheet.add_cell(0_i32);
        let x = sheet.add_cell(0_i32);
        let q = sheet.add_cell(0_i32);
        let y = sheet.add_cell(0_i32);
        let s = sheet.add_cell(0_i32);
        let r1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(p, x, |v: &i32| Ok(*v))])
            .unwrap();
        let r2 = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(q, x, |v: &i32| Ok(*v)),
                Method::from_fn_1_1(q, y, |v: &i32| Ok(*v)),
            ])
            .unwrap();
        let r3 = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(s, y, |v: &i32| Ok(*v)),
                Method::from_fn_1_1(s, q, |v: &i32| Ok(*v)),
            ])
            .unwrap();
        let active: HashSet<_> = [r1, r2, r3].into_iter().collect();
        let assignment = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        assert_eq!(assignment.chosen.len(), 3);
        let unique: HashSet<_> = assignment.claimed.values().collect();
        assert_eq!(unique.len(), assignment.claimed.len());
    }
}
```

This file references `crate::planner::matching` implicitly via `super::*` in its own
tests — it is not yet wired into `lib.rs`/`planner.rs`, so it cannot be compiled/tested
standalone. Proceed to Task 3.

- [ ] **Step 2: Commit**

```bash
git add adam-rs/src/planner/matching.rs
git commit -m "$(cat <<'EOF'
feat(adam-rs): add bipartite/hypergraph matching for planner method selection

Not yet wired into the module tree.
EOF
)"
```

---

### Task 3: Dependency digraph + acyclicity check (`planner/digraph.rs`)

**Files:**
- Create: `adam-rs/src/planner/digraph.rs`
- Test: same file, `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `matching::{pure_outputs, Assignment}` (Task 2), `scc::tarjan_scc` (Task 1).
- Produces:
  - `pub(crate) enum Node { Cell(CellId), Relationship(RelationshipId) }`
  - `pub(crate) fn build_digraph(assignment: &Assignment, relationships: &SlotMap<RelationshipId, RelationshipData>) -> HashMap<Node, Vec<Node>>`
  - `pub(crate) fn is_acyclic(assignment: &Assignment, relationships: &SlotMap<RelationshipId, RelationshipData>) -> bool`

- [ ] **Step 1: Write the module with its test suite**

Create `adam-rs/src/planner/digraph.rs`:

```rust
//! Builds the planner's dependency digraph from a chosen [`Assignment`], and checks
//! whether it is acyclic.

use std::collections::HashMap;

use slotmap::SlotMap;

use crate::relationship::{RelationshipData, RelationshipId};

use super::matching::{pure_outputs, Assignment};
use super::scc::tarjan_scc;

/// A node in the planner's dependency digraph: either a cell or a relationship.
///
/// Modeling relationships as their own nodes (rather than only cells) ensures a
/// relationship whose chosen method has zero pure outputs (a purely self-referencing
/// method) still appears exactly once in a topological ordering of this graph.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Node {
    Cell(crate::cell::CellId),
    Relationship(RelationshipId),
}

/// Builds the dependency digraph induced by `assignment`: an edge from each of a
/// relationship's plain (non-self-referencing) input cells to the relationship, and
/// from the relationship to each of its pure-output cells.
///
/// - Complexity: O(R · K) where R = assigned relationships, K = cells per chosen method.
pub(crate) fn build_digraph(
    assignment: &Assignment,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
) -> HashMap<Node, Vec<Node>> {
    let mut adj: HashMap<Node, Vec<Node>> = HashMap::new();
    for (&rel_id, &method_idx) in &assignment.chosen {
        let method = &relationships[rel_id].methods[method_idx];
        for &input in &method.inputs {
            if method.outputs.contains(&input) {
                continue; // self-referencing input: pre-round value, no dependency edge
            }
            adj.entry(Node::Cell(input)).or_default().push(Node::Relationship(rel_id));
        }
        for output in pure_outputs(method) {
            adj.entry(Node::Relationship(rel_id)).or_default().push(Node::Cell(output));
        }
    }
    adj
}

/// Returns `true` if `assignment`'s induced digraph has no non-trivial strongly
/// connected component (every relationship can be executed in some valid order).
///
/// - Complexity: O(R · K) (dominated by [`build_digraph`]; SCC is O(V + E) on the
///   resulting graph).
pub(crate) fn is_acyclic(
    assignment: &Assignment,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
) -> bool {
    let adj = build_digraph(assignment, relationships);
    tarjan_scc(&adj).iter().all(|component| component.len() == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Method, Sheet};
    use std::collections::HashSet;

    #[test]
    fn acyclic_assignment_reports_acyclic() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let active: HashSet<_> = [rel].into_iter().collect();
        let assignment = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        assert!(is_acyclic(&assignment, &sheet.relationships));
    }

    #[test]
    fn cyclic_assignment_reports_not_acyclic() {
        // Force the diamond's colliding pairing directly: R1's only method claims c via
        // [a,b]->c, R2's only method claims b via [c,d]->b -- b depends on c (R1) and
        // c depends on b (R2).
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0.0_f64);
        let b = sheet.add_cell(0.0_f64);
        let c = sheet.add_cell(0.0_f64);
        let d = sheet.add_cell(0.0_f64);
        let r1 = sheet
            .add_relationship(vec![Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok(x * y))])
            .unwrap();
        let r2 = sheet
            .add_relationship(vec![Method::from_fn_2_1([c, d], b, |x: &f64, y: &f64| Ok(y / x))])
            .unwrap();
        let active: HashSet<_> = [r1, r2].into_iter().collect();
        let assignment = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        assert!(!is_acyclic(&assignment, &sheet.relationships));
    }

    #[test]
    fn purely_self_referencing_relationship_still_appears_as_a_node() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, a, |x: &i32| Ok((*x).min(0)))])
            .unwrap();
        let active: HashSet<_> = [rel].into_iter().collect();
        let assignment = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        let adj = build_digraph(&assignment, &sheet.relationships);
        // Zero pure outputs (a is excluded, self-referencing) and zero plain inputs
        // (a is the only input, also self-referencing): the relationship contributes
        // no edges at all, so it must not appear as a key in `adj`.
        assert!(!adj.contains_key(&Node::Relationship(rel)));
        assert!(is_acyclic(&assignment, &sheet.relationships));
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add adam-rs/src/planner/digraph.rs
git commit -m "$(cat <<'EOF'
feat(adam-rs): add planner dependency digraph + acyclicity check

Not yet wired into the module tree.
EOF
)"
```

---

### Task 4: Greedy strength-ordered release (`planner/release.rs`)

**Files:**
- Create: `adam-rs/src/planner/release.rs`
- Test: same file, `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `matching::Assignment` (Task 2), `digraph::is_acyclic` (Task 3).
- Produces: `pub(crate) fn resolve(cells: &SlotMap<CellId, CellData>, relationships: &SlotMap<RelationshipId, RelationshipData>, active: &HashSet<RelationshipId>) -> Option<Assignment>`

- [ ] **Step 1: Write the module with its test suite**

Create `adam-rs/src/planner/release.rs`:

```rust
//! Chooses which cells are sources by greedily releasing cells in descending strength
//! order, checking at each step whether a matching + acyclic assignment still exists
//! with that cell (and every previously released cell) forbidden from being claimed.

use std::cmp::Reverse;
use std::collections::HashSet;

use slotmap::SlotMap;

use crate::{
    cell::{CellData, CellId},
    relationship::{RelationshipData, RelationshipId},
};

use super::digraph::is_acyclic;
use super::matching::Assignment;

/// Finds the strength-optimal acyclic assignment: an [`Assignment`] where the set of
/// cells left unclaimed (sources) is lexicographically maximal in descending strength
/// order among all assignments whose induced digraph is acyclic.
///
/// Processes cells in descending strength order; for each currently-claimed cell,
/// tentatively adds it to the forbidden set and re-solves. If a matching still exists
/// and its induced digraph is acyclic, the release is kept; otherwise the cell remains
/// claimed. This single mechanism handles both ordinary strength-based method
/// selection (an uncontested relationship's choice of which cell to leave exogenous)
/// and cyclic ("diamond") resolution uniformly -- both are just instances of "does
/// releasing this cell still admit a valid acyclic assignment".
///
/// Returns `None` if no acyclic assignment exists at all (a genuine algebraic loop, or
/// no assignment exists whatsoever).
///
/// - Complexity: O(C · solve) where C = cells and `solve` is [`Assignment::solve`]'s
///   cost -- each cell triggers at most one full re-solve attempt.
pub(crate) fn resolve(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> Option<Assignment> {
    let mut released: HashSet<CellId> = HashSet::new();
    let mut current = Assignment::solve(relationships, active, &released)?;

    let mut cells_sorted: Vec<CellId> = cells.keys().collect();
    cells_sorted.sort_by_key(|&id| Reverse(cells[id].strength));

    for cell in cells_sorted {
        if !current.claimed.contains_key(&cell) {
            released.insert(cell);
            continue;
        }

        let mut candidate_released = released.clone();
        candidate_released.insert(cell);

        if let Some(candidate) = Assignment::solve(relationships, active, &candidate_released)
            && is_acyclic(&candidate, relationships)
        {
            released = candidate_released;
            current = candidate;
        }
    }

    is_acyclic(&current, relationships).then_some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Method, Sheet};

    #[test]
    fn no_assignment_returns_none() {
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
        assert!(resolve(&sheet.cells, &sheet.relationships, &active).is_none());
    }

    #[test]
    fn genuinely_unsolvable_cycle_returns_none() {
        // x = f(y); y = g(x), each with only one method and no other cell involved:
        // no acyclic assignment exists no matter which cell is released.
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(0_i32);
        let y = sheet.add_cell(0_i32);
        let r1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(y, x, |v: &i32| Ok(*v + 1))])
            .unwrap();
        let r2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(x, y, |v: &i32| Ok(*v + 1))])
            .unwrap();
        let active: HashSet<_> = [r1, r2].into_iter().collect();
        assert!(resolve(&sheet.cells, &sheet.relationships, &active).is_none());
    }

    #[test]
    fn strength_prefers_the_higher_strength_cell_as_source() {
        // Triangle a,b,c: a and b are written (higher strength) after c is added, so
        // a and b must remain sources and c must be derived, regardless of method
        // iteration order.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0.0_f64);
        let b = sheet.add_cell(0.0_f64);
        let c = sheet.add_cell(0.0_f64);
        let rel = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok(x * y)),
                Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok(y / x)),
                Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok(y / x)),
            ])
            .unwrap();
        sheet.write(a, 2.0).unwrap();
        sheet.write(b, 3.0).unwrap();
        let active: HashSet<_> = [rel].into_iter().collect();
        let assignment = resolve(&sheet.cells, &sheet.relationships, &active).unwrap();
        assert_eq!(assignment.claimed[&c], rel);
        assert!(!assignment.claimed.contains_key(&a));
        assert!(!assignment.claimed.contains_key(&b));
    }

    #[test]
    fn diamond_collision_pattern_resolves_instead_of_failing() {
        // R1{a,b,c}, R2{b,c,d}: a and d outrank b and c (the collision pattern from
        // begin/examples/diamond.adm2). resolve() must still find a valid, acyclic
        // assignment -- not return None.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0.0_f64);
        let b = sheet.add_cell(0.0_f64);
        let c = sheet.add_cell(0.0_f64);
        let d = sheet.add_cell(0.0_f64);
        let r1 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok(x * y)),
                Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok(y / x)),
                Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok(y / x)),
            ])
            .unwrap();
        let r2 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([b, c], d, |x: &f64, y: &f64| Ok(x * y)),
                Method::from_fn_2_1([b, d], c, |x: &f64, y: &f64| Ok(y / x)),
                Method::from_fn_2_1([c, d], b, |x: &f64, y: &f64| Ok(y / x)),
            ])
            .unwrap();
        sheet.write(a, 3.0).unwrap();
        sheet.write(d, 24.0).unwrap();
        let active: HashSet<_> = [r1, r2].into_iter().collect();
        let assignment = resolve(&sheet.cells, &sheet.relationships, &active)
            .expect("a valid acyclic assignment exists for this structure");
        assert_eq!(assignment.chosen.len(), 2);
        let unique: HashSet<_> = assignment.claimed.values().collect();
        assert_eq!(unique.len(), assignment.claimed.len());
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add adam-rs/src/planner/release.rs
git commit -m "$(cat <<'EOF'
feat(adam-rs): add greedy strength-ordered release for the planner

Not yet wired into the module tree.
EOF
)"
```

---

### Task 5: Wire the new pipeline into `planner.rs`

**Files:**
- Modify: `adam-rs/src/planner.rs` (entire file)

**Interfaces:**
- Consumes: `matching::pure_outputs` (Task 2), `digraph::{build_digraph, Node}` (Task 3),
  `release::resolve` (Task 4), `scc::tarjan_scc` (Task 1).
- Produces: `pub(crate) fn plan(...) -> Result<Plan, Error>` — **signature unchanged**
  from today; `pub(crate) struct Plan` — **shape unchanged**.

- [ ] **Step 1: Replace `planner.rs`'s content**

Read the current file's `#[cfg(test)] mod tests` block first (`adam-rs/src/planner.rs`,
everything from `#[cfg(test)]` to the end of the file) and keep it **verbatim** — only
the code above it changes. Replace everything from the top of the file down to (but not
including) `#[cfg(test)]` with:

```rust
//! Planning pass: selects one method per relationship and returns them in dependency
//! order.
//!
//! The planner finds the strength-optimal acyclic assignment of methods to
//! relationships: [`release::resolve`] greedily tries, in descending cell-strength
//! order, to leave each cell unclaimed (a source), keeping the change only when a
//! valid method assignment still exists ([`matching::Assignment::solve`]) *and* its
//! induced dependency digraph is acyclic ([`digraph::is_acyclic`]). This single
//! mechanism handles both ordinary strength-based method selection (an uncontested
//! relationship's choice of which cell to leave exogenous) and overlapping cyclic
//! ("diamond") structures uniformly -- both are instances of "does releasing this cell
//! still admit a valid acyclic assignment". See
//! `docs/superpowers/specs/2026-08-04-cyclic-constraint-planner-design.md` for the
//! full design rationale and literature grounding.
//!
//! Once [`release::resolve`] succeeds, its result's induced digraph is guaranteed
//! acyclic, so a plain topological sort (reusing [`scc::tarjan_scc`], which produces
//! components in reverse topological order on an acyclic graph -- each component is
//! then a single node) yields `execution_order` directly.
//!
//! A separate fixpoint, [`forced_output_cells`], computes cells that can never be a
//! source (a relationship's method structure guarantees the cell is always produced),
//! purely for the informational [`Plan::forced_outputs`] / [`Plan::forced_relationships`]
//! fields exposed to callers (e.g. disabling form fields in `begin`'s Inspector) -- it
//! does not influence method selection above, which discovers the same infeasibility
//! structurally via failed augmenting-path displacement in [`matching::Assignment::solve`].

use std::collections::{HashMap, HashSet};

use slotmap::SlotMap;

use crate::{
    cell::{CellData, CellId},
    error::Error,
    relationship::{RelationshipData, RelationshipId},
};

mod digraph;
mod matching;
mod release;
mod scc;

use digraph::{build_digraph, Node};
use matching::pure_outputs;

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
/// invisible to method selection.
///
/// # Errors
///
/// - `Error::Conflict` — no valid, acyclic method assignment exists for `active`.
///
/// - Complexity: O(C · R² · M · K) where C = cells, R = active relationships, M =
///   methods per relationship, K = cells per method — [`release::resolve`] attempts up
///   to C full re-solves, each up to O(R² · M · K) in the worst case.
pub(crate) fn plan(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> Result<Plan, Error> {
    let (forced_outputs, alive) = forced_output_cells(relationships, active);

    let assignment = release::resolve(cells, relationships, active).ok_or(Error::Conflict)?;

    let mut adj = build_digraph(&assignment, relationships);
    // Ensure every active relationship appears as a node even if its chosen method has
    // zero pure outputs and zero plain inputs (fully self-referencing, e.g. a -> a):
    // such a relationship contributes no edges via build_digraph and would otherwise be
    // silently missing from the topological order below.
    for &rel_id in active {
        adj.entry(Node::Relationship(rel_id)).or_default();
    }

    let mut components = scc::tarjan_scc(&adj);
    components.reverse();

    let mut execution_order: Vec<(RelationshipId, usize)> = Vec::new();
    for component in components {
        debug_assert_eq!(component.len(), 1, "release::resolve guarantees an acyclic digraph");
        if let Node::Relationship(rel_id) = component[0] {
            execution_order.push((rel_id, assignment.chosen[&rel_id]));
        }
    }

    if execution_order.len() != active.len() {
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

/// Computes the cells that can never be a source under `active`, and which methods
/// survive that determination.
///
/// A cell is forced by a relationship when it is a [`pure_outputs`] member of every one
/// of that relationship's currently-alive methods. Starting with all methods alive, this
/// runs to a fixpoint: any method whose pure outputs include a cell forced by a
/// *different* relationship is eliminated (selecting it would always double-write that
/// cell), which can force more cells for the relationships that lost a method. The loop
/// stops once no relationship loses another method.
///
/// The returned `HashMap` gives, for each relationship in `active`, a per-method-index
/// alive flag (`false` for eliminated methods); used only to populate
/// [`Plan::forced_relationships`] and the `forced` half of [`Plan::forced_outputs`] --
/// it does not gate method selection in [`plan`] above, which discovers the same
/// infeasibility structurally.
///
/// - Precondition: every `RelationshipId` in `active` is present in `relationships`.
///
/// - Complexity: O(D · R · M · K²) where D = total methods eliminated across all
///   iterations (bounded by the total method count), R = active relationships,
///   M = methods per relationship, K = cells per method (squared because
///   [`pure_outputs`] scans `inputs` once per output).
fn forced_output_cells(
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> (HashSet<CellId>, HashMap<RelationshipId, Vec<bool>>) {
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
            let own_forced = &forced_per_rel[&rel_id];
            let rel = &relationships[rel_id];
            let alive_methods = alive.get_mut(&rel_id).expect("seeded for every active id");
            for (idx, method) in rel.methods.iter().enumerate() {
                if alive_methods[idx]
                    && pure_outputs(method)
                        .iter()
                        .any(|c| global_forced.contains(c) && !own_forced.contains(c))
                {
                    alive_methods[idx] = false;
                    changed = true;
                }
            }
        }

        if !changed {
            return (global_forced, alive);
        }
    }
}
```

- [ ] **Step 2: Run the full `adam-rs` test suite**

Run: `cargo test -p adam-rs`

Expected: every test passes — the four new modules' own tests (Tasks 1–4), the existing
`planner.rs` unit tests (kept verbatim), and all of `adam-rs/tests/integration.rs`. If
any existing test fails, stop and debug before proceeding — do not touch Task 6 while
Task 5 is red.

- [ ] **Step 3: Run clippy on adam-rs**

Run: `cargo clippy -p adam-rs --all-targets -- -D warnings`

Expected: no warnings. Fix any and re-run before proceeding.

- [ ] **Step 4: Commit**

```bash
git add adam-rs/src/planner.rs
git commit -m "$(cat <<'EOF'
refactor(adam-rs): replace planner's greedy flood-fill with matching + SCC pipeline

Wires up the new matching/digraph/release/scc modules. plan()'s public
signature and Plan's shape are unchanged; forced_output_cells is kept
verbatim (now importing pure_outputs from the matching module instead of
a private copy).
EOF
)"
```

---

### Task 6: Diamond regression tests (`adam-rs/tests/integration.rs`)

**Files:**
- Modify: `adam-rs/tests/integration.rs` (append at end of file, after line 1019)

**Interfaces:**
- Consumes: `adam_rs::{Error, Method, Sheet}` (already imported at the top of the file).

- [ ] **Step 1: Append the new tests**

Append to the end of `adam-rs/tests/integration.rs`:

```rust

#[test]
fn diamond_relationships_resolve_when_outer_cells_outrank_shared_cells() {
    // Reproduces begin/examples/diamond.adm2: R1{a,b,c} and R2{b,c,d} are triangle
    // relationships sharing b and c. Before this change, writing a and d (making them
    // outrank the never-written b and c) made propagate() return Error::Conflict --
    // see docs/superpowers/specs/2026-08-04-cyclic-constraint-planner-design.md.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0.0_f64);
    let b = sheet.add_cell(0.0_f64);
    let c = sheet.add_cell(0.0_f64);
    let d = sheet.add_cell(0.0_f64);
    let r1 = sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok(x * y)),
            Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok(y / x)),
            Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok(y / x)),
        ])
        .unwrap();
    let r2 = sheet
        .add_relationship(vec![
            Method::from_fn_2_1([b, c], d, |x: &f64, y: &f64| Ok(x * y)),
            Method::from_fn_2_1([b, d], c, |x: &f64, y: &f64| Ok(y / x)),
            Method::from_fn_2_1([c, d], b, |x: &f64, y: &f64| Ok(y / x)),
        ])
        .unwrap();

    sheet.write(a, 3.0).unwrap();
    sheet.write(d, 24.0).unwrap();

    sheet.propagate().unwrap();

    let r1_idx = sheet.selected_method(r1).expect("r1 is planned");
    let r2_idx = sheet.selected_method(r2).expect("r2 is planned");
    let r1_out = sheet.method_outputs(r1, r1_idx).unwrap()[0];
    let r2_out = sheet.method_outputs(r2, r2_idx).unwrap()[0];

    assert_ne!(r1_out, r2_out, "no double-write between the two relationships");
    let sources = [a, b, c, d].into_iter().filter(|&x| sheet.is_source(x)).count();
    assert_eq!(sources, 2, "exactly one input cell per relationship remains a source");
}

#[test]
fn overlapping_diamond_chain_resolves_via_cascade() {
    // R1{a,b,c}, R2{b,c,d}, R3{c,d,e}: two overlapping diamonds (R1/R2 share b,c;
    // R2/R3 share c,d). a and e -- the two outer tips -- outrank the three shared
    // cells, exercising the cascade: resolving the first diamond must be able to
    // shrink the second rather than each being resolved in isolation.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0.0_f64);
    let b = sheet.add_cell(0.0_f64);
    let c = sheet.add_cell(0.0_f64);
    let d = sheet.add_cell(0.0_f64);
    let e = sheet.add_cell(0.0_f64);
    let r1 = sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok(x * y)),
            Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok(y / x)),
            Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok(y / x)),
        ])
        .unwrap();
    let r2 = sheet
        .add_relationship(vec![
            Method::from_fn_2_1([b, c], d, |x: &f64, y: &f64| Ok(x * y)),
            Method::from_fn_2_1([b, d], c, |x: &f64, y: &f64| Ok(y / x)),
            Method::from_fn_2_1([c, d], b, |x: &f64, y: &f64| Ok(y / x)),
        ])
        .unwrap();
    let r3 = sheet
        .add_relationship(vec![
            Method::from_fn_2_1([c, d], e, |x: &f64, y: &f64| Ok(x * y)),
            Method::from_fn_2_1([c, e], d, |x: &f64, y: &f64| Ok(y / x)),
            Method::from_fn_2_1([d, e], c, |x: &f64, y: &f64| Ok(y / x)),
        ])
        .unwrap();

    sheet.write(a, 3.0).unwrap();
    sheet.write(e, 24.0).unwrap();

    sheet.propagate().unwrap();

    let outputs: std::collections::HashSet<_> = [r1, r2, r3]
        .into_iter()
        .map(|r| {
            let idx = sheet.selected_method(r).expect("every relationship is planned");
            sheet.method_outputs(r, idx).unwrap()[0]
        })
        .collect();
    assert_eq!(outputs.len(), 3, "no two relationships may claim the same cell");
    let sources = [a, b, c, d, e].into_iter().filter(|&x| sheet.is_source(x)).count();
    assert_eq!(sources, 2);
}

#[test]
fn mutually_dependent_relationships_with_no_external_input_remain_conflict() {
    // x = f(y); y = g(x), each with only one method and no other cell involved: a
    // genuine algebraic loop with no valid acyclic execution order, regardless of
    // strength. Must still return Error::Conflict.
    let mut sheet = Sheet::new();
    let x = sheet.add_cell(0_i32);
    let y = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(y, x, |v: &i32| Ok(*v + 1))])
        .unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(x, y, |v: &i32| Ok(*v + 1))])
        .unwrap();
    assert!(matches!(sheet.propagate(), Err(Error::Conflict)));
}
```

- [ ] **Step 2: Run the full test suite**

Run: `cargo test -p adam-rs`

Expected: all tests pass, including the three new ones above.

- [ ] **Step 3: Commit**

```bash
git add adam-rs/tests/integration.rs
git commit -m "$(cat <<'EOF'
test(adam-rs): add diamond, cascade, and unsolvable-cycle regression tests

Covers the collision pattern from begin/examples/diamond.adm2 (previously
Error::Conflict, now resolves), a chain of two overlapping diamonds
(exercising the cascade), and confirms a genuine algebraic loop with no
external input still correctly reports Error::Conflict.
EOF
)"
```

---

### Task 7: Full workspace verification

**Files:** none (verification only).

- [ ] **Step 1: Format**

Run: `cargo fmt --all`

- [ ] **Step 2: Build the whole workspace with zero warnings**

Run: `cargo build --workspace`

Expected: clean build, no warnings. If any appear, fix them (this is a hard requirement
per `CLAUDE.md` — plain `cargo build` catches things `clippy -D warnings` doesn't, e.g.
an unused `mut`).

- [ ] **Step 3: Test the whole workspace with zero warnings**

Run: `cargo test --workspace` and `cargo test --doc --workspace`

Expected: all tests pass, no warnings.

- [ ] **Step 4: Clippy, all three required invocations**

Run:

```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

Expected: no warnings from any of the three. `begin` is unaffected by this change (it
only calls `adam-rs`'s public `Sheet` API, which is unchanged), but the project's
Git Workflow rule requires all three before any PR.

- [ ] **Step 5: Update the diamond example's viability note (optional sanity check)**

If `begin` can be run interactively (see `verifying-begin-ui` skill), open
`begin/examples/diamond.adm2` and confirm writing `a` and `d` to high values no longer
surfaces a conflict in the Inspector. This is a manual sanity check, not required for
the automated suite to pass.

- [ ] **Step 6: Commit (only if Step 1 produced formatting changes)**

```bash
git add -u
git commit -m "$(cat <<'EOF'
chore(adam-rs): cargo fmt after cyclic constraint planner rewrite
EOF
)"
```
