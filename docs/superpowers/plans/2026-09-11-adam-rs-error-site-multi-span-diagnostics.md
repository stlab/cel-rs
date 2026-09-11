# Error-Site Sets and Multi-Span Diagnostics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `adam_rs::Error` an ordered set of structural sites (relationships, methods, cells), reconstruct the cycle/overconstrained sets in the planner, and render multi-caret diagnostics — a cycle backtrace at propagation time and both offending methods at parse time.

**Architecture:** `adam-rs` replaces the two-variant `Copy` `ErrorLocation` with an ordered `Vec<ErrorSite>` on each error; the planner reconstructs cycles (DFS) and minimal overconstrained sets (deletion filtering) on the cold error path. `adam-lang` gains relationship- and cell-declaration span tables plus a `locate_error` resolver; `cel-parser` gains a multi-span renderer and secondary spans on `ParseError`; `adam-web-ui` renders through them and `begin` consolidates its parse state onto a single `ParsedSheet`.

**Tech Stack:** Rust, `slotmap`, `annotate-snippets` 0.12, `proc_macro2`, `indexmap`, Dioxus (begin).

**Spec:** `docs/superpowers/specs/2026-09-11-adam-rs-error-site-multi-span-diagnostics-design.md`

## Global Constraints

- Every function gets a contract-style `///` doc comment (Summary; Preconditions as `- Precondition:`; `# Errors`; Postconditions; `- Complexity:` whenever not O(1)). Public APIs get `# Examples`.
- `cargo fmt --all` before every commit (enforced by the pre-commit hook).
- Warnings are errors. Before any PR run all four lint invocations: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`; `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`; `cargo clippy -p begin --all-targets -- -D warnings`. `cargo build --workspace` and `cargo test --workspace` must be warning-free too.
- Derive tests from the contract and public interface only, not the implementation. Do not test precondition violations.
- `Error` and `ErrorSite` are `#[non_exhaustive]`; downstream `match` arms keep a wildcard.
- Signed-integer arithmetic uses `checked_*`; fallible ops use `.op1r`/`.op2r`. (Not expected to arise here, but holds.)
- Never commit to `main`; work stays on `worktree-adam-lang/issue-188`.

---

## File Structure

- `adam-rs/src/error.rs` — replace `ErrorLocation` with `ErrorSite`; `sites` fields; `sites()` accessor. (Modify)
- `adam-rs/src/sheet.rs` — structural + conditional raise sites build `sites` vectors. (Modify)
- `adam-rs/src/planner.rs` — populate `Cycle`/`FilterCycle`/`Conflict` sites. (Modify)
- `adam-rs/src/planner/release.rs` — `ReleaseFailure::NoAcyclicAssignment(Assignment)`. (Modify)
- `adam-rs/src/planner/trace.rs` — `recover_cycle`, `minimal_infeasible_set`. (Create)
- `cel-parser/src/error.rs` — `SpanLabel`, `format_multi_span`, `ParseError` secondaries. (Modify)
- `adam-lang/src/parser.rs` — `relationship_spans`/`cell_spans` tables; `site_label`; `locate_error`; build-time site resolution. (Modify)
- `adam-web-ui/src/labels.rs` — `format_adam_error` takes `&ParsedSheet`, multi-span control flow. (Modify)
- `adam-web-ui/src/build.rs` — `BuildOutcome` carries `ParsedSheet`; drop `MethodSpans`. (Modify)
- `adam-web-ui/src/inspector.rs` — render call passes `&ParsedSheet`. (Modify)
- `begin/src/app.rs` (+ `example_source.rs`) — consolidate onto `Signal<ParsedSheet>`. (Modify)

---

## Phase 1 — adam-rs: `ErrorSite` core

### Task 1.1: Replace `ErrorLocation` with `ErrorSite`; migrate every variant and raise site

**Files:**
- Modify: `adam-rs/src/error.rs`
- Modify: `adam-rs/src/sheet.rs` (raise sites at 209, 225, 209-230, 244, 257, 192, 201, 220, 335, 500, 514, 612, 951, 976, 1012, 1142, 1370, 1501, 1515, 1531; and `add_conditional` 324, 330, 395, 397, 400, 407, 415)
- Modify: `adam-rs/src/planner.rs` (116-117, 135, 153; execute-plan sites live in `sheet.rs`)
- Test: `adam-rs/src/error.rs` (tests module)

**Interfaces:**
- Produces:
  - `pub enum ErrorSite { MethodIndex(usize), Method(RelationshipId, usize), Relationship(RelationshipId), Cell(CellId) }` (`#[non_exhaustive]`, `Debug, Clone, Copy, PartialEq, Eq, Hash`).
  - `Error` struct variants each gain `sites: Vec<ErrorSite>`: `TypeMismatch`, `MethodFailed`, `InvalidMethod`, `MismatchedMethodCells`, `DuplicateMethodOutputs`, `InvalidCellKind` (rename from `location: Option<ErrorLocation>`); and newly `Conflict { sites }`, `Cycle { sites }`, `FilterCycle { sites }`, `InvalidConditional { sites }` (were unit variants).
  - `pub fn Error::sites(&self) -> &[ErrorSite]`.
- Consumes: nothing (first task).

- [ ] **Step 1: Write the failing test** in `adam-rs/src/error.rs` tests module (replaces `error_location_variants_are_distinct` and `location_is_none_for_a_locationless_variant`):

```rust
#[test]
fn error_site_variants_are_distinct() {
    let a = ErrorSite::MethodIndex(0);
    let b = ErrorSite::Method(RelationshipId::default(), 0);
    let c = ErrorSite::Relationship(RelationshipId::default());
    assert_ne!(a, b);
    assert_ne!(b, c);
    assert_eq!(a, ErrorSite::MethodIndex(0));
}

#[test]
fn sites_is_empty_for_a_siteless_variant() {
    assert!(Error::InvalidId.sites().is_empty());
}

#[test]
fn sites_returns_the_recorded_site() {
    let rid = RelationshipId::default();
    let e = Error::MethodFailed { error: anyhow::anyhow!("x"), sites: vec![ErrorSite::Method(rid, 2)] };
    assert_eq!(e.sites(), &[ErrorSite::Method(rid, 2)]);
}
```

- [ ] **Step 2: Replace the type and accessor** in `adam-rs/src/error.rs`: delete `ErrorLocation`; add `ErrorSite`; on the six existing variants rename `location: Option<ErrorLocation>` → `sites: Vec<ErrorSite>`; convert `Conflict`/`Cycle`/`FilterCycle` from unit to `{ sites: Vec<ErrorSite> }` and `InvalidConditional` likewise; update each variant's doc comment to describe its `sites` (per spec §3). Replace `location()` with:

```rust
/// The source components this error implicates, primary first.
///
/// `sites[0]` is the primary culprit; further entries are ordered related context
/// (for a cycle, the remaining members in loop order). Empty for variants that never
/// track a site, or when none applies to this occurrence.
pub fn sites(&self) -> &[ErrorSite] {
    match self {
        Error::TypeMismatch { sites, .. }
        | Error::MethodFailed { sites, .. }
        | Error::InvalidMethod { sites }
        | Error::MismatchedMethodCells { sites }
        | Error::DuplicateMethodOutputs { sites }
        | Error::InvalidCellKind { sites }
        | Error::Conflict { sites }
        | Error::Cycle { sites }
        | Error::FilterCycle { sites }
        | Error::InvalidConditional { sites } => sites,
        _ => &[],
    }
}
```

Leave every `Display` arm's message text unchanged (match the new struct variants with `{ .. }`).

- [ ] **Step 3: Update in-crate raise sites.** In `sheet.rs` and `planner.rs`, replace `location: Some(ErrorLocation::MethodIndex(i))` → `sites: vec![ErrorSite::MethodIndex(i)]`, `location: Some(ErrorLocation::Method(r, i))` → `sites: vec![ErrorSite::Method(r, i)]`, and `location: None` → `sites: vec![]`. For the newly struct-form planner errors, temporarily emit `sites: vec![]` (populated in Phase 2): `planner.rs:116` → `Error::Conflict { sites: vec![] }`, `117` → `Error::Cycle { sites: vec![] }`, `135` → `Error::FilterCycle { sites: vec![] }`, `153` → `Error::Conflict { sites: vec![] }`. For every `Error::InvalidConditional` in `add_conditional`, temporarily `Error::InvalidConditional { sites: vec![] }` (populated in Task 1.4). Update the `import`: `use crate::error::{Error, ErrorSite}` where needed.

- [ ] **Step 4: Update in-crate test match arms.** Change `Err(Error::Cycle)` → `Err(Error::Cycle { .. })`, same for `Conflict`, `FilterCycle`, `InvalidConditional`, and `InvalidMethod { location: None }` → `InvalidMethod { sites: _ }` (or `{ .. }`), across `sheet.rs`, `planner.rs`, `planner/release.rs`, `error.rs`, and `tests/integration.rs`. In `error.rs`'s `non_method_failed_variants_have_no_source`, update the constructors to `sites: vec![]`.

- [ ] **Step 5: Run the crate tests**

Run: `cargo test -p adam-rs`
Expected: PASS (behavior unchanged; only field names/shapes changed).

- [ ] **Step 6: Commit**

```bash
git add adam-rs/src/error.rs adam-rs/src/sheet.rs adam-rs/src/planner.rs adam-rs/src/planner/release.rs adam-rs/tests/integration.rs
git commit -m "refactor(adam-rs): replace ErrorLocation with ordered ErrorSite list (#188)"
```

### Task 1.2: Enrich `MismatchedMethodCells` and `DuplicateMethodOutputs` sites

**Files:**
- Modify: `adam-rs/src/sheet.rs:234-262`
- Test: `adam-rs/src/sheet.rs` (tests module)

**Interfaces:**
- Consumes: `ErrorSite` (Task 1.1).
- Produces: `MismatchedMethodCells.sites = [MethodIndex(diverging), MethodIndex(0), Cell(each differing cell)…]`; `DuplicateMethodOutputs.sites` = self-dup `[MethodIndex(i), Cell(dup)]`, cross-dup `[MethodIndex(later), MethodIndex(earlier), Cell(shared)…]`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn mismatched_method_cells_names_both_methods_and_the_differing_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);
    // method 0 references {a,b}; method 1 references {a,c}: c (and b) diverge.
    let err = sheet.add_relationship(vec![
        Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
        Method::from_fn_1_1(a, c, |x: &i32| Ok(*x)),
    ]).unwrap_err();
    let sites = err.sites();
    assert_eq!(sites[0], ErrorSite::MethodIndex(1));
    assert_eq!(sites[1], ErrorSite::MethodIndex(0));
    assert!(sites[2..].contains(&ErrorSite::Cell(c)));
}

#[test]
fn duplicate_output_set_across_methods_names_both_methods_and_the_shared_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    // both methods reference {a,b} and both output b.
    let err = sheet.add_relationship(vec![
        Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
        Method::from_fn_1_1(a, b, |x: &i32| Ok(*x + 1)),
    ]).unwrap_err();
    let sites = err.sites();
    assert_eq!(sites[0], ErrorSite::MethodIndex(1));
    assert_eq!(sites[1], ErrorSite::MethodIndex(0));
    assert!(sites[2..].contains(&ErrorSite::Cell(b)));
}

#[test]
fn duplicate_cell_within_own_outputs_names_the_method_and_the_repeated_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let i32_ty = std::any::TypeId::of::<i32>();
    // one method whose outputs name b twice.
    let err = sheet.add_relationship(vec![Method::new(
        vec![a], vec![b, b], vec![i32_ty], vec![i32_ty, i32_ty],
        |args| { let v = *args[0].downcast_ref::<i32>().unwrap(); Ok(vec![Box::new(v), Box::new(v)]) },
    )]).unwrap_err();
    let sites = err.sites();
    assert_eq!(sites[0], ErrorSite::MethodIndex(0));
    assert!(sites[1..].contains(&ErrorSite::Cell(b)));
}
```

- [ ] **Step 2: Run to verify failure** — Run: `cargo test -p adam-rs mismatched_method_cells_names_both`; Expected: FAIL (sites has only one entry today).

- [ ] **Step 3: Implement** in `add_relationship`. Replace the mismatch block (around 243):

```rust
if let Some(rel_idx) = cell_sets[1..].iter().position(|set| set != &cell_sets[0]) {
    let diverging = rel_idx + 1;
    let mut sites = vec![ErrorSite::MethodIndex(diverging), ErrorSite::MethodIndex(0)];
    // Symmetric difference of the two cell sets, in a stable order.
    for &c in cell_sets[diverging].symmetric_difference(&cell_sets[0]) {
        sites.push(ErrorSite::Cell(c));
    }
    return Err(Error::MismatchedMethodCells { sites });
}
```

Replace the duplicate-outputs loop (around 254) so it records the responsible cell(s) and the earlier method:

```rust
let mut seen_output_sets: Vec<(usize, HashSet<CellId>)> = Vec::with_capacity(methods.len());
for (idx, method) in methods.iter().enumerate() {
    let output_set: HashSet<CellId> = method.outputs.iter().copied().collect();
    if output_set.len() != method.outputs.len() {
        // A cell repeated within this method's own outputs.
        let mut sites = vec![ErrorSite::MethodIndex(idx)];
        let mut seen = HashSet::new();
        for &o in &method.outputs {
            if !seen.insert(o) { sites.push(ErrorSite::Cell(o)); }
        }
        return Err(Error::DuplicateMethodOutputs { sites });
    }
    if let Some((earlier, _)) = seen_output_sets.iter().find(|(_, s)| *s == output_set) {
        let mut sites = vec![ErrorSite::MethodIndex(idx), ErrorSite::MethodIndex(*earlier)];
        for &o in &method.outputs { sites.push(ErrorSite::Cell(o)); }
        return Err(Error::DuplicateMethodOutputs { sites });
    }
    seen_output_sets.push((idx, output_set));
}
```

(`symmetric_difference` needs both as `HashSet`; `cell_sets` already are.)

- [ ] **Step 4: Run to verify pass** — Run: `cargo test -p adam-rs`; Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): name both methods and cells in structural relationship errors (#188)"
```

### Task 1.3: Add the output-cell site to `InvalidCellKind`

**Files:**
- Modify: `adam-rs/src/sheet.rs:217-223`
- Test: `adam-rs/src/sheet.rs`

**Interfaces:**
- Produces: `InvalidCellKind.sites = [MethodIndex(idx), Cell(bad_output)]`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn invalid_cell_kind_names_the_method_and_the_source_output_cell() {
    let mut sheet = Sheet::new();
    let s = sheet.add_source_cell(0_i32); // Source-kind cell
    let err = sheet.add_relationship(vec![Method::from_fn_1_1(s, s, |x: &i32| Ok(*x))]);
    // A Source-kind output is rejected.
    let err = err.unwrap_err();
    let sites = err.sites();
    assert_eq!(sites[0], ErrorSite::MethodIndex(0));
    assert!(sites[1..].contains(&ErrorSite::Cell(s)));
}
```

(Confirm the Source-cell constructor name in `sheet.rs` — use whatever `add_source_cell`/`add_source` is called; adjust the test accordingly.)

- [ ] **Step 2: Run to verify failure** — `cargo test -p adam-rs invalid_cell_kind_names_the_method`; Expected: FAIL.

- [ ] **Step 3: Implement** — at the `CellKind::Source` check:

```rust
if cell.kind == CellKind::Source {
    return Err(Error::InvalidCellKind {
        sites: vec![ErrorSite::MethodIndex(idx), ErrorSite::Cell(cell_id)],
    });
}
```

- [ ] **Step 4: Run** — `cargo test -p adam-rs`; Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): name the offending output cell in InvalidCellKind (#188)"
```

### Task 1.4: Populate `InvalidConditional` sites

**Files:**
- Modify: `adam-rs/src/sheet.rs` `add_conditional` (324, 397, 400, 415; leave 330/395/407 empty per spec §3 rationale)
- Test: `adam-rs/src/sheet.rs`

**Interfaces:**
- Produces: `InvalidConditional.sites` — 324 (match cell type): `[Cell(match_cell)]`; 397 (branch rel with extra method touching a contributing cell): `[Relationship(rel_id), Cell(the contributing adj cell)]`; 400 (already conditional): `[Relationship(rel_id)]`; 415 (duplicate in this call): `[Relationship(rel_id)]`. 330/395/407: `sites: vec![]`.

- [ ] **Step 1: Write the failing tests** (extend the existing `add_conditional_*` tests):

```rust
#[test]
fn invalid_conditional_duplicate_relationship_names_that_relationship() {
    // Reuse the body of add_conditional_returns_invalid_conditional_for_duplicate_relationship_across_branches,
    // capturing the offending RelationshipId, then:
    let err = result.unwrap_err();
    assert!(matches!(err, Error::InvalidConditional { .. }));
    assert!(err.sites().contains(&ErrorSite::Relationship(dup_rel)));
}

#[test]
fn invalid_conditional_multi_method_branch_names_the_relationship() {
    // Reuse add_conditional_returns_invalid_conditional_for_multi_method_relationship_involving_match_cell,
    // capturing the branch RelationshipId:
    let err = result.unwrap_err();
    assert!(err.sites().contains(&ErrorSite::Relationship(branch_rel)));
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p adam-rs invalid_conditional_duplicate_relationship_names`; Expected: FAIL.

- [ ] **Step 3: Implement** — set `sites` at each raise point:
  - 324: `return Err(Error::InvalidConditional { sites: vec![ErrorSite::Cell(*cell)] });`
  - 397: `return Err(Error::InvalidConditional { sites: { let mut s = vec![ErrorSite::Relationship(rel_id)]; if let Some(&c) = rel.adj.iter().find(|c| contributing_cells.contains(c)) { s.push(ErrorSite::Cell(c)); } s } });`
  - 400: `return Err(Error::InvalidConditional { sites: vec![ErrorSite::Relationship(rel_id)] });`
  - 415: `return Err(Error::InvalidConditional { sites: vec![ErrorSite::Relationship(rel_id)] });`
  - 330, 395, 407: `sites: vec![]`.

- [ ] **Step 4: Run** — `cargo test -p adam-rs`; Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): name implicated relationships/cells in InvalidConditional (#188)"
```

---

## Phase 2 — adam-rs: planner set reconstruction

### Task 2.1: `recover_cycle` in `planner/trace.rs`

**Files:**
- Create: `adam-rs/src/planner/trace.rs`
- Modify: `adam-rs/src/planner.rs` (add `mod trace;`)
- Test: `adam-rs/src/planner/trace.rs`

**Interfaces:**
- Consumes: `super::digraph::Node`.
- Produces: `pub(crate) fn recover_cycle(adj: &HashMap<Node, Vec<Node>>, component: &[Node]) -> Vec<Node>` — returns one simple cycle as an ordered node list; the last node's successors include the first (loop closes). Empty if `component` has no internal cycle (should not happen for a real SCC of size > 1).

- [ ] **Step 1: Write the failing test** in `trace.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::digraph::Node;
    use crate::relationship::RelationshipId;
    use crate::cell::CellId;
    use slotmap::SlotMap;
    use std::collections::HashMap;

    #[test]
    fn recover_cycle_returns_a_simple_loop() {
        let mut rmap: SlotMap<RelationshipId, ()> = SlotMap::with_key();
        let r1 = rmap.insert(()); let r2 = rmap.insert(());
        let mut cmap: SlotMap<CellId, ()> = SlotMap::with_key();
        let x = cmap.insert(()); let y = cmap.insert(());
        // r1 -> x -> r2 -> y -> r1
        let mut adj: HashMap<Node, Vec<Node>> = HashMap::new();
        adj.insert(Node::Relationship(r1), vec![Node::Cell(x)]);
        adj.insert(Node::Cell(x), vec![Node::Relationship(r2)]);
        adj.insert(Node::Relationship(r2), vec![Node::Cell(y)]);
        adj.insert(Node::Cell(y), vec![Node::Relationship(r1)]);
        let component = vec![Node::Relationship(r1), Node::Cell(x), Node::Relationship(r2), Node::Cell(y)];
        let cycle = recover_cycle(&adj, &component);
        assert_eq!(cycle.len(), 4);
        // consecutive-with-wraparound edges all exist in adj
        for i in 0..cycle.len() {
            let from = cycle[i];
            let to = cycle[(i + 1) % cycle.len()];
            assert!(adj[&from].contains(&to), "missing edge {from:?}->{to:?}");
        }
    }
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p adam-rs recover_cycle_returns_a_simple_loop`; Expected: FAIL (module/function missing).

- [ ] **Step 3: Implement** `recover_cycle`:

```rust
//! Cycle and infeasible-set reconstruction for planner error reporting. Runs only on
//! the cold error path, after a plan has already failed.

use std::collections::{HashMap, HashSet};

use slotmap::SlotMap;

use crate::cell::{CellData, CellId};
use crate::relationship::{RelationshipData, RelationshipId};

use super::digraph::Node;
use super::matching::Assignment;

/// Returns one simple cycle within `component`, as an ordered node list whose consecutive
/// entries (wrapping from last to first) are edges of `adj`.
///
/// - Precondition: `component`'s nodes contain at least one cycle reachable within the set
///   (true for any strongly connected component of size > 1).
/// - Complexity: O(V + E) over the nodes and edges induced by `component`.
pub(crate) fn recover_cycle(adj: &HashMap<Node, Vec<Node>>, component: &[Node]) -> Vec<Node> {
    let members: HashSet<Node> = component.iter().copied().collect();
    let start = match component.first() {
        Some(&n) => n,
        None => return Vec::new(),
    };
    let mut path: Vec<Node> = Vec::new();
    let mut on_path: HashSet<Node> = HashSet::new();
    let mut stack: Vec<(Node, usize)> = vec![(start, 0)];
    while let Some(&mut (node, ref mut next)) = stack.last_mut() {
        if *next == 0 {
            if on_path.contains(&node) {
                // Found the loop: slice from the earlier occurrence.
                let at = path.iter().position(|&n| n == node).unwrap();
                return path[at..].to_vec();
            }
            path.push(node);
            on_path.insert(node);
        }
        let successors = adj.get(&node).map(|v| v.as_slice()).unwrap_or(&[]);
        let mut advanced = false;
        while *next < successors.len() {
            let w = successors[*next];
            *next += 1;
            if !members.contains(&w) {
                continue;
            }
            if on_path.contains(&w) {
                let at = path.iter().position(|&n| n == w).unwrap();
                return path[at..].to_vec();
            }
            stack.push((w, 0));
            advanced = true;
            break;
        }
        if !advanced {
            on_path.remove(&node);
            path.pop();
            stack.pop();
        }
    }
    Vec::new()
}
```

Add `mod trace;` to `planner.rs`.

- [ ] **Step 4: Run** — `cargo test -p adam-rs recover_cycle`; Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/planner/trace.rs adam-rs/src/planner.rs
git commit -m "feat(adam-rs): recover an ordered cycle from a planner SCC (#188)"
```

### Task 2.2: `minimal_infeasible_set` in `planner/trace.rs`

**Files:**
- Modify: `adam-rs/src/planner/trace.rs`
- Test: `adam-rs/src/planner/trace.rs`

**Interfaces:**
- Consumes: `super::matching::Assignment`, `RelationshipData`, `CellData`.
- Produces: `pub(crate) fn minimal_infeasible_set(relationships: &SlotMap<RelationshipId, RelationshipData>, active: &HashSet<RelationshipId>) -> HashSet<RelationshipId>` — a subset-minimal group with no valid `Assignment::solve` (no cells forbidden). `_cells` param is not needed (solve ignores strength).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn minimal_infeasible_set_isolates_the_conflicting_pair() {
    use crate::{Method, Sheet};
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let out = sheet.add_cell(0_i32);
    let free = sheet.add_cell(0_i32);
    // r1 and r2 both must claim `out` (infeasible together); r3 is unrelated (claims free).
    let r1 = sheet.add_relationship(vec![Method::from_fn_1_1(a, out, |x: &i32| Ok(*x))]).unwrap();
    let r2 = sheet.add_relationship(vec![Method::from_fn_1_1(b, out, |x: &i32| Ok(*x))]).unwrap();
    let r3 = sheet.add_relationship(vec![Method::from_fn_1_1(a, free, |x: &i32| Ok(*x))]).unwrap();
    let active: std::collections::HashSet<_> = [r1, r2, r3].into_iter().collect();
    let set = minimal_infeasible_set(&sheet.relationships, &active);
    assert_eq!(set, [r1, r2].into_iter().collect());
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p adam-rs minimal_infeasible_set_isolates`; Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
/// Returns a subset-minimal group of `active` relationships that has no valid method
/// assignment (`Assignment::solve` returns `None`): removing any member of the result makes
/// the remainder feasible.
///
/// - Precondition: `active` itself is infeasible under `Assignment::solve` with no cells
///   forbidden.
/// - Complexity: O(R) calls to `Assignment::solve`, each O(R²·M·K); cold error path only.
pub(crate) fn minimal_infeasible_set(
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> HashSet<RelationshipId> {
    let mut candidate = active.clone();
    // Deletion filtering: drop each relationship whose removal keeps the set infeasible.
    let members: Vec<RelationshipId> = active.iter().copied().collect();
    for r in members {
        let mut without = candidate.clone();
        without.remove(&r);
        if Assignment::solve(relationships, &without, &HashSet::new()).is_none() {
            candidate = without;
        }
    }
    candidate
}
```

- [ ] **Step 4: Run** — `cargo test -p adam-rs minimal_infeasible_set`; Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/planner/trace.rs
git commit -m "feat(adam-rs): compute a minimal infeasible relationship set (#188)"
```

### Task 2.3: Carry the cyclic assignment on `NoAcyclicAssignment` and populate `Cycle` sites

**Files:**
- Modify: `adam-rs/src/planner/release.rs` (enum + `resolve`)
- Modify: `adam-rs/src/planner.rs` (map at 115-118; build the cycle)
- Test: `adam-rs/src/planner.rs`

**Interfaces:**
- Consumes: `trace::recover_cycle`, `build_digraph`, `scc::tarjan_scc`, `digraph::Node`.
- Produces: `ReleaseFailure::NoAcyclicAssignment(Assignment)`; `Error::Cycle { sites }` where sites alternate `Relationship`/`Cell` in loop order.

- [ ] **Step 1: Write the failing test** in `planner.rs` tests:

```rust
#[test]
fn cycle_error_names_the_relationships_in_loop_order() {
    use crate::{Method, Sheet, error::ErrorSite};
    let mut sheet = Sheet::new();
    let x = sheet.add_cell(0_i32);
    let y = sheet.add_cell(0_i32);
    let r1 = sheet.add_relationship(vec![Method::from_fn_1_1(y, x, |v: &i32| Ok(*v + 1))]).unwrap();
    let r2 = sheet.add_relationship(vec![Method::from_fn_1_1(x, y, |v: &i32| Ok(*v + 1))]).unwrap();
    let err = sheet.propagate().unwrap_err();
    let sites = match &err { Error::Cycle { sites } => sites.clone(), other => panic!("{other:?}") };
    let rels: std::collections::HashSet<_> = sites.iter().filter_map(|s| match s {
        ErrorSite::Relationship(r) => Some(*r), _ => None }).collect();
    assert_eq!(rels, [r1, r2].into_iter().collect());
    assert!(sites.iter().any(|s| matches!(s, ErrorSite::Cell(_))));
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p adam-rs cycle_error_names_the_relationships`; Expected: FAIL (sites empty).

- [ ] **Step 3: Implement.** In `release.rs`, change the enum and `resolve`:

```rust
pub(crate) enum ReleaseFailure {
    NoAssignment,
    NoAcyclicAssignment(Assignment),
}
```

```rust
let Some(mut current) = Assignment::solve_acyclic(relationships, active, &released) else {
    return Err(match Assignment::solve(relationships, active, &released) {
        Some(cyclic) => ReleaseFailure::NoAcyclicAssignment(cyclic),
        None => ReleaseFailure::NoAssignment,
    });
};
```

In `planner.rs`, replace the `map_err` (115-118) with an explicit match so the cyclic assignment can be traced:

```rust
let assignment = match release::resolve(cells, relationships, active) {
    Ok(a) => a,
    Err(ReleaseFailure::NoAssignment) => {
        return Err(Error::Conflict {
            sites: conflict_sites(relationships, active),
        });
    }
    Err(ReleaseFailure::NoAcyclicAssignment(cyclic)) => {
        return Err(Error::Cycle { sites: cycle_sites(&cyclic, relationships) });
    }
};
```

Add two private helpers in `planner.rs`:

```rust
/// Maps a cyclic assignment to `Relationship`/`Cell` sites in loop order.
fn cycle_sites(
    assignment: &matching::Assignment,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
) -> Vec<ErrorSite> {
    let adj = build_digraph(assignment, relationships);
    let component = scc::tarjan_scc(&adj)
        .into_iter()
        .find(|c| c.len() > 1)
        .unwrap_or_default();
    trace::recover_cycle(&adj, &component)
        .into_iter()
        .map(node_to_site)
        .collect()
}

/// Wraps `minimal_infeasible_set` as `Relationship` sites.
fn conflict_sites(
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> Vec<ErrorSite> {
    let mut sites: Vec<ErrorSite> = trace::minimal_infeasible_set(relationships, active)
        .into_iter()
        .map(ErrorSite::Relationship)
        .collect();
    sites.sort_by_key(|s| match s { ErrorSite::Relationship(r) => *r, _ => Default::default() });
    sites
}

/// Maps a digraph `Node` to its `ErrorSite`.
fn node_to_site(node: Node) -> ErrorSite {
    match node {
        Node::Relationship(r) => ErrorSite::Relationship(r),
        Node::Cell(c) => ErrorSite::Cell(c),
    }
}
```

(Import `ErrorSite`, `trace`, `scc`, `Node`, `build_digraph` as needed. `RelationshipId` is `Ord`? slotmap keys are `Copy + Ord` via `KeyData`; if `sort_by_key` on the key doesn't compile, sort by `slotmap::Key::data().as_ffi()` instead, or drop the sort — order among conflict members is not contractual.)

- [ ] **Step 4: Run** — `cargo test -p adam-rs`; Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/planner.rs adam-rs/src/planner/release.rs
git commit -m "feat(adam-rs): attach the ordered cycle to Error::Cycle (#188)"
```

### Task 2.4: Populate `FilterCycle` sites

**Files:**
- Modify: `adam-rs/src/planner.rs:129-136`
- Test: `adam-rs/src/planner.rs`

**Interfaces:**
- Consumes: `cycle_sites`-style logic over the already-built `adj` and the offending `component`.
- Produces: `Error::FilterCycle { sites }` in loop order, including the filtered `Cell` and its filter-arg edge.

- [ ] **Step 1: Write the failing test.** Reuse the structure from `release.rs`/`planner.rs`'s existing `Err(Error::FilterCycle { .. })` test at 620; capture the involved relationship/cell and assert `err.sites()` is non-empty and contains a `Relationship` and a `Cell`:

```rust
#[test]
fn filter_cycle_error_names_its_members() {
    // Build the same sheet the existing FilterCycle test uses (a filter whose argument
    // closes a loop with the selected method), then:
    let err = sheet.propagate().unwrap_err();
    let sites = match &err { Error::FilterCycle { sites } => sites.clone(), o => panic!("{o:?}") };
    assert!(sites.iter().any(|s| matches!(s, ErrorSite::Cell(_))));
    assert!(!sites.is_empty());
}
```

(Locate the existing FilterCycle test body — `planner.rs:620` area — and mirror its sheet.)

- [ ] **Step 2: Run to verify failure** — Expected: FAIL (sites empty).

- [ ] **Step 3: Implement** — at the `component.len() != 1` branch:

```rust
if component.len() != 1 {
    let sites = trace::recover_cycle(&adj, &component)
        .into_iter()
        .map(node_to_site)
        .collect();
    return Err(Error::FilterCycle { sites });
}
```

(`adj` and `component` are already in scope in the loop; note `components` was reversed and consumed by the `for component in components` loop — capture the offending `component` slice directly. If `component` is a moved `Vec<Node>`, pass `&component`.)

- [ ] **Step 4: Run** — `cargo test -p adam-rs`; Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/planner.rs
git commit -m "feat(adam-rs): attach the filter-induced cycle to Error::FilterCycle (#188)"
```

### Task 2.5: Populate the post-plan `Conflict` site path

**Files:**
- Modify: `adam-rs/src/planner.rs:152-154`
- Test: `adam-rs/src/planner.rs` (existing `Conflict` test at 322 already asserts the variant; add a sites assertion)

**Interfaces:**
- Produces: the `method_count != active.len()` path also uses `conflict_sites(relationships, active)`.

- [ ] **Step 1: Write the failing test** — extend the existing test at `planner.rs:322`:

```rust
#[test]
fn conflict_error_names_the_minimal_infeasible_set() {
    // Reuse the two-relationships-one-output infeasible structure; capture r1, r2:
    let err = sheet.propagate().unwrap_err();
    let sites = match &err { Error::Conflict { sites } => sites.clone(), o => panic!("{o:?}") };
    let rels: std::collections::HashSet<_> = sites.iter().filter_map(|s| match s {
        ErrorSite::Relationship(r) => Some(*r), _ => None }).collect();
    assert_eq!(rels, [r1, r2].into_iter().collect());
}
```

- [ ] **Step 2: Run to verify failure** — Expected: FAIL if this structure reaches the 153 path, else already covered by 2.3 (the `resolve` path). Keep the test regardless — it pins the observable contract.

- [ ] **Step 3: Implement** — at line 153:

```rust
if method_count != active.len() {
    return Err(Error::Conflict { sites: conflict_sites(relationships, active) });
}
```

- [ ] **Step 4: Run** — `cargo test -p adam-rs`; Expected: PASS. Then `cargo test -p adam-rs --doc` and `cargo clippy -p adam-rs --all-targets -- -D warnings`.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/planner.rs
git commit -m "feat(adam-rs): attach the minimal infeasible set to the post-plan Conflict (#188)"
```

---

## Phase 3 — cel-parser: multi-span rendering

### Task 3.1: `SpanLabel` and `format_multi_span`

**Files:**
- Modify: `cel-parser/src/error.rs`
- Modify: `cel-parser/src/lib.rs` (re-export `SpanLabel`, `format_multi_span`)
- Test: `cel-parser/src/error.rs`

**Interfaces:**
- Produces: `pub struct SpanLabel { pub span: SourceSpan, pub label: String }`; `pub fn format_multi_span(title: &str, labels: &[SpanLabel], source: &str, filename: &str, start_line: u32, renderer: &Renderer) -> String`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn format_multi_span_underlines_every_span_and_prints_labels() {
    let source = "aaa bbb ccc";
    let labels = vec![
        SpanLabel { span: SourceSpan::new(1, 0, 1, 3), label: "first".into() },
        SpanLabel { span: SourceSpan::new(1, 8, 1, 11), label: "third".into() },
    ];
    let out = format_multi_span("mismatch", &labels, source, "t.adm2", 1, &Renderer::plain());
    assert!(out.contains("mismatch"), "{out}");
    assert!(out.contains("first"), "{out}");
    assert!(out.contains("third"), "{out}");
    assert!(!out.contains('\u{1b}'), "plain renderer has no ANSI: {out}");
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p cel-parser format_multi_span_underlines`; Expected: FAIL.

- [ ] **Step 3: Implement** in `error.rs`:

```rust
/// One labelled span in a multi-span diagnostic.
pub struct SpanLabel {
    /// The source region to underline.
    pub span: SourceSpan,
    /// The label printed beside the caret.
    pub label: String,
}

/// Renders `title` with several labelled annotations over `source`, the first as the
/// primary caret and the rest as secondary context, in rustc style.
///
/// - Precondition: `labels` is non-empty.
/// - Complexity: O(n) in `source` length plus the number of labels.
///
/// # Examples
///
/// ```rust
/// use annotate_snippets::Renderer;
/// use cel_parser::{SourceSpan, SpanLabel, format_multi_span};
/// let labels = vec![SpanLabel { span: SourceSpan::new(1, 0, 1, 1), label: "here".into() }];
/// let out = format_multi_span("oops", &labels, "x", "f.adm2", 1, &Renderer::plain());
/// assert!(out.contains("oops"));
/// ```
pub fn format_multi_span(
    title: &str,
    labels: &[SpanLabel],
    source_code: &str,
    filename: &str,
    start_line: u32,
    renderer: &Renderer,
) -> String {
    debug_assert!(!labels.is_empty(), "`labels` must be non-empty");
    let mut snippet = Snippet::source(source_code)
        .path(filename)
        .line_start(start_line as usize);
    for (i, l) in labels.iter().enumerate() {
        let range = span_to_byte_range(source_code, l.span);
        let kind = if i == 0 { AnnotationKind::Primary } else { AnnotationKind::Context };
        snippet = snippet.annotation(kind.span(range).label(l.label.as_str()));
    }
    let report = [Group::with_title(Level::ERROR.primary_title(title)).element(snippet)];
    renderer.render(&report)
}
```

Re-export from `lib.rs`: add `SpanLabel, format_multi_span` to the `pub use error::{...}` list.

- [ ] **Step 4: Run** — `cargo test -p cel-parser`; Expected: PASS. (If `.label()` or `AnnotationKind::Context` names differ in annotate-snippets 0.12, adjust to the crate's actual API — check `cargo doc -p annotate-snippets --open` or the existing `AnnotationKind::Primary` usage.)

- [ ] **Step 5: Commit**

```bash
git add cel-parser/src/error.rs cel-parser/src/lib.rs
git commit -m "feat(cel-parser): add multi-span diagnostic renderer (#188)"
```

### Task 3.2: `ParseError` secondary spans

**Files:**
- Modify: `cel-parser/src/error.rs` (`ParseError`)
- Test: `cel-parser/src/error.rs`

**Interfaces:**
- Produces: `ParseError` gains `secondary: Vec<SpanLabel>` (default empty) with `pub fn with_secondary(self, secondary: Vec<SpanLabel>) -> Self`; `ParseError::format_rustc_style` routes to `format_multi_span` when `secondary` is non-empty.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn parse_error_with_secondary_renders_all_spans() {
    let source = "aaa bbb";
    let e = ParseError::new_range("bad", Span::call_site(), Span::call_site())
        .with_secondary(vec![SpanLabel { span: SourceSpan::new(1, 4, 1, 7), label: "also here".into() }]);
    // primary span is call_site (line 1 col 0..0); secondary underlines "bbb".
    let out = e.format_rustc_style(source, "t.cel", 1, &Renderer::plain());
    assert!(out.contains("bad"), "{out}");
    assert!(out.contains("also here"), "{out}");
}

#[test]
fn parse_error_without_secondary_renders_single_span_as_before() {
    let e = ParseError::new("bad", Span::call_site());
    let out = e.format_rustc_style("10 + 20 30", "t.cel", 1, &Renderer::plain());
    assert!(out.contains("error: bad"), "{out}");
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p cel-parser parse_error_with_secondary`; Expected: FAIL.

- [ ] **Step 3: Implement** — add `secondary: Vec<SpanLabel>` to `ParseError`; initialize `secondary: Vec::new()` in `new`, `new_range`, `from_lex_error`; add:

```rust
/// Attaches secondary labelled spans, rendered as extra carets alongside the primary.
pub fn with_secondary(mut self, secondary: Vec<SpanLabel>) -> Self {
    self.secondary = secondary;
    self
}
```

In `format_rustc_style`, before building the single-span report:

```rust
if !self.secondary.is_empty() {
    let primary = SourceSpan { start: self.span.start(), end: self.end_span.unwrap_or(self.span).end() };
    let mut labels = vec![SpanLabel { span: primary, label: String::new() }];
    labels.extend(self.secondary.iter().map(|l| SpanLabel { span: l.span, label: l.label.clone() }));
    return format_multi_span(&self.message, &labels, source_code, filename, start_line, renderer);
}
```

(Do not derive `Copy` implications: `ParseError` is already `Clone`; `Vec<SpanLabel>` keeps it `Clone` as long as `SpanLabel: Clone` — derive `Clone` on `SpanLabel`.)

- [ ] **Step 4: Run** — `cargo test -p cel-parser`; Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add cel-parser/src/error.rs
git commit -m "feat(cel-parser): render ParseError secondary spans as extra carets (#188)"
```

---

## Phase 4 — adam-lang: span tables, labels, resolution

### Task 4.1: `relationship_spans` and `cell_spans` tables

**Files:**
- Modify: `adam-lang/src/parser.rs` (`ParsedSheet`, `ParseContext`, `parse_str`, `parse_relationship_decl`, `parse_cell_decl`, `parse_source_decl`, `parse_out_decl` cell creation)
- Test: `adam-lang/src/parser.rs`

**Interfaces:**
- Produces: `pub relationship_spans: HashMap<RelationshipId, SourceSpan>` and `pub cell_spans: HashMap<CellId, SourceSpan>` on `ParsedSheet`, populated on success.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn parse_populates_relationship_and_cell_spans() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    let parsed = parser.parse_str(
        "sheet s { cell a: i32 = 0; cell b: i32; relationship { b := a; } }"
    ).unwrap();
    assert_eq!(parsed.cell_spans.len(), 2);
    assert_eq!(parsed.relationship_spans.len(), 1);
    // every declared cell id has a span
    for (_, (id, _)) in &parsed.cell_names {
        assert!(parsed.cell_spans.contains_key(id));
    }
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p adam-lang parse_populates_relationship_and_cell_spans`; Expected: FAIL.

- [ ] **Step 3: Implement.** Add both fields to `ParsedSheet` and `ParseContext` (init empty). In `parse_str`, move them into the returned `ParsedSheet`. In `parse_relationship_decl`'s `Ok(rel_id)` arm, insert `ctx.relationship_spans.insert(rel_id, SourceSpan::from_proc_macro2_range(block_start, close_span));`. Wherever a cell id is created for a declaration (`build_default_cell`/`build_default_source_cell` return sites in `parse_cell_decl` ~280, `parse_source_decl` ~373, `parse_out_decl` ~1324), insert `ctx.cell_spans.insert(cell_id, SourceSpan::from_proc_macro2(name_span));`. Update `ParsedSheet`'s `Debug` impl to include the two new fields.

- [ ] **Step 4: Run** — `cargo test -p adam-lang`; Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add adam-lang/src/parser.rs
git commit -m "feat(adam-lang): record relationship-block and cell-declaration spans (#188)"
```

### Task 4.2: `site_label` free function

**Files:**
- Modify: `adam-lang/src/parser.rs` (or a new small `adam-lang/src/error_labels.rs` module — prefer a new module for isolation)
- Test: same module

**Interfaces:**
- Produces: `pub(crate) fn site_label(e: &adam_rs::Error, index: usize, cell_name: &impl Fn(CellId) -> Option<String>) -> String`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn site_label_describes_cycle_relationship_steps() {
    let e = adam_rs::Error::Cycle { sites: vec![
        adam_rs::ErrorSite::Relationship(adam_rs::RelationshipId::default()),
        adam_rs::ErrorSite::Cell(adam_rs::CellId::default()),
    ]};
    let name = |_id: adam_rs::CellId| Some("x".to_string());
    let l0 = site_label(&e, 0, &name);
    let l1 = site_label(&e, 1, &name);
    assert!(!l0.is_empty());
    assert!(l1.contains("x")); // the cell step names the cell
}

#[test]
fn site_label_describes_mismatched_method_cells() {
    let e = adam_rs::Error::MismatchedMethodCells { sites: vec![
        adam_rs::ErrorSite::MethodIndex(1),
        adam_rs::ErrorSite::MethodIndex(0),
        adam_rs::ErrorSite::Cell(adam_rs::CellId::default()),
    ]};
    let name = |_id| Some("c".to_string());
    assert!(site_label(&e, 0, &name).to_lowercase().contains("differ"));
    assert!(site_label(&e, 1, &name).to_lowercase().contains("baseline"));
    assert!(site_label(&e, 2, &name).contains("c"));
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p adam-lang site_label_describes`; Expected: FAIL.

- [ ] **Step 3: Implement** a `match` on the `Error` variant and `index`, using `e.sites()[index]` and `cell_name` for `Cell` sites. Cover: `Cycle`/`FilterCycle` (step-oriented wording, cell steps name the cell), `Conflict` ("this relationship is part of an overconstrained group"), `MismatchedMethodCells` (index 0 "cell set differs from the baseline", 1 "the baseline method", cells "cell `c` appears in only one method"), `DuplicateMethodOutputs` (0 "this method's output set", 1 "collides with this earlier method", cells "shared output cell `c`"), `InvalidCellKind`, `InvalidConditional`, and a generic fallback for other variants/sites. Keep phrasing plain (writing-style rules: no AI-marker vocabulary).

- [ ] **Step 4: Run** — `cargo test -p adam-lang`; Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add adam-lang/src/parser.rs adam-lang/src/error_labels.rs adam-lang/src/lib.rs
git commit -m "feat(adam-lang): synthesize human labels for error sites (#188)"
```

### Task 4.3: `ParsedSheet::locate_error`

**Files:**
- Modify: `adam-lang/src/parser.rs`
- Test: `adam-lang/src/parser.rs`

**Interfaces:**
- Consumes: `site_label`, the three span tables, `cell_names`.
- Produces: `pub fn locate_error(&self, e: &adam_rs::Error) -> Vec<(SourceSpan, String)>`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn locate_error_resolves_a_cycle_to_multiple_ordered_spans() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    let mut parsed = parser.parse_str(
        "sheet s { cell x: i32 = 0; cell y: i32 = 0; relationship { x := y + 1i32; } relationship { y := x + 1i32; } }"
    ).unwrap();
    let err = parsed.propagate().unwrap_err();
    assert!(matches!(err, adam_rs::Error::Cycle { .. }));
    let located = parsed.locate_error(&err);
    // both relationship blocks resolve to spans
    let rel_spans = located.len();
    assert!(rel_spans >= 2, "expected >=2 spans, got {rel_spans}: {located:?}");
}
```

- [ ] **Step 2: Run to verify failure** — Expected: FAIL (method missing).

- [ ] **Step 3: Implement**

```rust
/// Resolves each of `e`'s `ErrorSite`s to a source span and a human label, primary first.
///
/// Sites whose span is not recorded are skipped, so the result may be shorter than
/// `e.sites()`; empty when none resolves (the caller then falls back to `Display`).
///
/// - Complexity: O(s) in the number of sites.
pub fn locate_error(&self, e: &adam_rs::Error) -> Vec<(SourceSpan, String)> {
    let by_id: HashMap<CellId, String> =
        self.cell_names.iter().map(|(n, (id, _))| (*id, n.clone())).collect();
    let name = |id: CellId| by_id.get(&id).cloned();
    let mut out = Vec::new();
    for (i, site) in e.sites().iter().enumerate() {
        let span = match site {
            ErrorSite::Method(r, idx) => self.method_spans.get(&(*r, *idx)).copied(),
            ErrorSite::Relationship(r) => self.relationship_spans.get(r).copied(),
            ErrorSite::Cell(c) => self.cell_spans.get(c).copied(),
            ErrorSite::MethodIndex(_) => None,
            _ => None,
        };
        if let Some(span) = span {
            out.push((span, crate::error_labels::site_label(e, i, &name)));
        }
    }
    out
}
```

(Import `ErrorSite`; add to the `use adam_rs::{...}` list.)

- [ ] **Step 4: Run** — `cargo test -p adam-lang`; Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add adam-lang/src/parser.rs
git commit -m "feat(adam-lang): resolve error sites to labelled source spans (#188)"
```

### Task 4.4: Build-time multi-span resolution in `parse_relationship_decl`

**Files:**
- Modify: `adam-lang/src/parser.rs:838-846`
- Test: `adam-lang/src/parser.rs`

**Interfaces:**
- Consumes: `SpanLabel`, `ParseError::with_secondary`, `site_label`, the parallel `spans` vec, `cell_spans`.
- Produces: a `ParseError` whose primary + secondary spans cover all resolvable sites of the failed `add_relationship`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn duplicate_output_set_error_underlines_both_bindings() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    let err = parser.parse_str(
        "sheet s { cell a: i32 = 0; cell b: i32; relationship { b := a; b := a + 1i32; } }"
    ).unwrap_err();
    let out = err.format_rustc_style(
        "sheet s { cell a: i32 = 0; cell b: i32; relationship { b := a; b := a + 1i32; } }",
        "t.adm2", 1, &cel_parser::Renderer::plain());
    // both `b := ...` bindings underlined: expect two carets / the later + earlier binding text
    assert!(out.matches("b :=").count() >= 1, "{out}");
    assert!(out.contains("outputs"), "{out}"); // message text
}
```

(Refine assertions to the actual rendered layout once implemented; the key contract is that both binding spans appear.)

- [ ] **Step 2: Run to verify failure** — Expected: FAIL (single span today).

- [ ] **Step 3: Implement.** Replace the `Err(e)` arm of `parse_relationship_decl`'s `match`:

```rust
Err(e) => {
    let by_id: HashMap<CellId, String> =
        ctx.cell_names.iter().map(|(n, (id, _))| (*id, n.clone())).collect();
    let name = |id: CellId| by_id.get(&id).cloned();
    // Resolve each site against the parallel binding spans and the cells recorded so far.
    let resolved: Vec<(SourceSpan, String)> = e.sites().iter().enumerate().filter_map(|(i, s)| {
        let span = match s {
            ErrorSite::MethodIndex(idx) => spans.get(*idx)
                .map(|(st, en)| SourceSpan::from_proc_macro2_range(*st, *en)),
            ErrorSite::Cell(c) => ctx.cell_spans.get(c).copied(),
            _ => None,
        };
        span.map(|sp| (sp, crate::error_labels::site_label(&e, i, &name)))
    }).collect();

    match resolved.split_first() {
        Some(((primary_span, _), rest)) => {
            let (start, end) = (
                proc_macro_start(*primary_span), // helper: build a ParseError from a SourceSpan
                proc_macro_end(*primary_span),
            );
            // Simpler: keep the primary as the binding's own proc_macro2 spans when available.
            let secondary: Vec<cel_parser::SpanLabel> = rest.iter()
                .map(|(sp, l)| cel_parser::SpanLabel { span: *sp, label: l.clone() })
                .collect();
            Err(build_range_error(&e.to_string(), *primary_span, secondary))
        }
        None => {
            // No site resolved: fall back to the whole block.
            Err(ParseError::new_range(e.to_string(), block_start, close_span))
        }
    }
}
```

Because `ParseError::new_range` needs `proc_macro2::Span`s but our resolved primary is a `SourceSpan`, prefer resolving the **primary** site to its `proc_macro2` spans directly from `spans` (for a `MethodIndex`) and only use `SourceSpan` for secondaries. Concretely: find the first site that is a `MethodIndex(i)` with a `spans[i]` entry, use `spans[i]`'s `(start, end)` as the `new_range` primary, and put every *other* resolved site (including cells) into `secondary`. Implement a small local helper:

```rust
// Within parse_relationship_decl:
let primary_pm = e.sites().iter().find_map(|s| match s {
    ErrorSite::MethodIndex(i) => spans.get(*i).copied(),
    _ => None,
});
let (start, end) = primary_pm.unwrap_or((block_start, close_span));
let secondary: Vec<cel_parser::SpanLabel> = e.sites().iter().enumerate().filter_map(|(i, s)| {
    // skip the primary method binding itself
    let sp = match s {
        ErrorSite::MethodIndex(idx) if Some(*idx) != primary_idx => spans.get(*idx)
            .map(|(st, en)| SourceSpan::from_proc_macro2_range(*st, *en)),
        ErrorSite::Cell(c) => ctx.cell_spans.get(c).copied(),
        _ => None,
    }?;
    Some(cel_parser::SpanLabel { span: sp, label: crate::error_labels::site_label(&e, i, &name) })
}).collect();
Err(ParseError::new_range(e.to_string(), start, end).with_secondary(secondary))
```

(`primary_idx` = the `MethodIndex` chosen as primary. Keep it simple and correct; drop the earlier sketch's `proc_macro_start`/`build_range_error` placeholders.)

- [ ] **Step 4: Run** — `cargo test -p adam-lang`; Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add adam-lang/src/parser.rs
git commit -m "feat(adam-lang): render build-time structural errors with multi-span carets (#188)"
```

### Task 4.5: Build-time resolution in `parse_conditional_decl`

**Files:**
- Modify: `adam-lang/src/parser.rs` `parse_conditional_decl` error arm (~1160+)
- Test: `adam-lang/src/parser.rs`

**Interfaces:**
- Produces: `InvalidConditional` `ParseError`s resolve `Relationship`/`Cell` sites to `relationship_spans`/`cell_spans` secondaries.

- [ ] **Step 1: Read** `parse_conditional_decl` to find how it currently maps `add_conditional`'s `Err` to a `ParseError` (grep for the `add_conditional(` call and its `map_err`/`match`). Write a failing test parsing a conditional that lists one relationship in two branches, asserting the rendered error underlines that relationship's block:

```rust
#[test]
fn conditional_duplicate_relationship_underlines_the_relationship() {
    // Construct a minimal sheet with a conditional reusing a relationship across branches.
    // Assert parse_str(...).unwrap_err().format_rustc_style(...) mentions the relationship span.
}
```

(Fill the source from an existing `add_conditional_returns_invalid_conditional_for_duplicate_relationship_across_branches`-style adm2 snippet; if adam-lang's grammar cannot yet express two branches referencing the same relationship name, assert on the single available span and record a follow-up. Confirm feasibility while reading the grammar in Step 1.)

- [ ] **Step 2: Run to verify failure** — Expected: FAIL.

- [ ] **Step 3: Implement** the same resolve-all-sites pattern as Task 4.4, but map `Relationship(r)` → `ctx.relationship_spans.get(r)` and `Cell(c)` → `ctx.cell_spans.get(c)`; choose the primary from the conditional's own in-scope span (e.g. `match_span`) and attach the rest as secondaries.

- [ ] **Step 4: Run** — `cargo test -p adam-lang`; Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add adam-lang/src/parser.rs
git commit -m "feat(adam-lang): render InvalidConditional with multi-span carets (#188)"
```

---

## Phase 5 — adam-web-ui: `format_adam_error`

### Task 5.1: `format_adam_error` takes `&ParsedSheet` and renders multi-span

**Files:**
- Modify: `adam-web-ui/src/labels.rs` (`format_adam_error`, delete `location_span`; update `write_str` closures at 87, 123)
- Modify: `adam-web-ui/src/build.rs` (`BuildOutcome`, `build_sheet`)
- Modify: `adam-web-ui/src/inspector.rs:321` (call site)
- Test: `adam-web-ui/src/labels.rs`

**Interfaces:**
- Consumes: `ParsedSheet::locate_error`, `cel_parser::format_multi_span`, `SpanContext::format_rustc_style`.
- Produces: `pub fn format_adam_error(e: &Error, parsed: &adam_lang::ParsedSheet, source: &str, file_name: &str, renderer: &Renderer) -> String`; `BuildOutcome { sheet_labels: Option<(ParsedSheet, Labels)>, error: Option<String> }` (no `method_spans` field; `MethodSpans` type alias deleted).

- [ ] **Step 1: Write the failing test** in `labels.rs`:

```rust
#[test]
fn format_adam_error_cycle_renders_a_multi_span_backtrace() {
    use adam_lang::{AdamParser, TypeRegistry};
    use cel_parser::OpLookup;
    let src = "sheet s { cell x: i32 = 0; cell y: i32 = 0; relationship { x := y + 1i32; } relationship { y := x + 1i32; } }";
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    let mut parsed = parser.parse_str(src).unwrap();
    let err = parsed.propagate().unwrap_err();
    let msg = format_adam_error(&err, &parsed, src, "t.adm2", &Renderer::plain());
    assert!(msg.contains("cycle"), "{msg}");
    // both relationship bindings referenced
    assert!(msg.contains("x :=") && msg.contains("y :="), "{msg}");
}
```

- [ ] **Step 2: Run to verify failure** — Expected: FAIL (signature + behavior).

- [ ] **Step 3: Implement.** Rewrite `format_adam_error`:

```rust
pub fn format_adam_error(
    e: &Error,
    parsed: &adam_lang::ParsedSheet,
    source: &str,
    file_name: &str,
    renderer: &Renderer,
) -> String {
    // CEL-internal sub-expression span wins when present.
    if let Error::MethodFailed { error, .. } = e {
        if error.downcast_ref::<SpanContext>().is_some() {
            return error.format_rustc_style(source, file_name, 1, renderer);
        }
    }
    let located = parsed.locate_error(e);
    match located.as_slice() {
        [] => e.to_string(),
        [(span, _label)] => SpanContext::new(*span)
            .format_rustc_style(&e.to_string(), source, file_name, 1, renderer),
        many => {
            let labels: Vec<cel_parser::SpanLabel> = many.iter()
                .map(|(span, label)| cel_parser::SpanLabel { span: *span, label: label.clone() })
                .collect();
            cel_parser::format_multi_span(&e.to_string(), &labels, source, file_name, 1, renderer)
        }
    }
}
```

Delete `location_span`. Update the two `write_str` closures to `Error::MethodFailed { error: ..., sites: vec![] }`. Update `use` imports (`SourceSpan`/`RelationshipId` may now be unused — remove them; add `cel_parser::SpanLabel`). Update `labels.rs` tests that constructed `Error::MethodFailed { .. , location: None }` and called `format_adam_error(&err, &HashMap::new(), ...)`: rebuild them against a real `ParsedSheet` (parse a small source) or delete the ones that only exercised the removed `method_spans` map, keeping coverage of the SpanContext fast path and the Display fallback.

In `build.rs`: change `BuildOutcome` to `sheet_labels: Option<(ParsedSheet, Labels)>` (import `adam_lang::ParsedSheet`), delete the `method_spans` field and the `MethodSpans` type alias, and in `build_sheet` return `Some((parsed, labels))` (build `labels` from `&parsed.sheet`/`&parsed.cell_names` before moving `parsed`), calling `format_adam_error(&e, &parsed, source, file_name, renderer)` on the propagate-error path. Update `build.rs` tests that read `outcome.method_spans`.

In `inspector.rs:321`: pass the `&ParsedSheet` the inspector now holds (see Phase 6) instead of `method_spans`.

- [ ] **Step 4: Run** — `cargo test -p adam-web-ui`; Expected: PASS. Then `cargo clippy -p adam-web-ui --all-targets -- -D warnings`.

- [ ] **Step 5: Commit**

```bash
git add adam-web-ui/src/labels.rs adam-web-ui/src/build.rs adam-web-ui/src/inspector.rs
git commit -m "feat(adam-web-ui): render adam-rs errors via ParsedSheet with multi-span backtraces (#188)"
```

---

## Phase 6 — begin: consolidate onto `Signal<ParsedSheet>`

### Task 6.1: Replace the `Sheet` + `MethodSpans` signals with one `ParsedSheet`

**Files:**
- Modify: `begin/src/app.rs` (all `sheet`/`method_spans` signal declarations and prop threads: 49-53, 84, 125-126, 241-252, 318, 327, 367, 430, 463-486, 540, 555-593, 605-631, 657-696)
- Modify: `begin/src/example_source.rs` (269, 289)
- Test: existing `begin` tests + build/clippy

**Interfaces:**
- Consumes: `adam_web_ui::build::BuildOutcome { sheet_labels: Option<(ParsedSheet, Labels)>, error }`.
- Produces: a single `Signal<ParsedSheet>` (deref-mut to `Sheet` for writes/propagate); `Labels` stays a derived signal; the `MethodSpans` signal and all its threads are removed.

- [ ] **Step 1: Read** `begin/src/app.rs` end-to-end to map every use of the `sheet`, `labels`, and `method_spans` signals, and how `BuildOutcome` is destructured (the `outcome.method_spans.unwrap_or_default()` sites at 367, 430, 540). Note each component that currently receives `method_spans: Signal<MethodSpans>`.

- [ ] **Step 2: Change the state shape.** Replace `sheet: Signal<Sheet>` with `parsed: Signal<ParsedSheet>` and delete `method_spans: Signal<MethodSpans>`. Where the code destructures `BuildOutcome`, bind `(parsed_value, labels_value)` from `sheet_labels`. Cell writes that did `sheet.write(...)` / `sheet.propagate()` now go through `parsed` (deref-mut). The inspector render path calls `format_adam_error(&e, &parsed.read(), ...)`.

- [ ] **Step 3: Update every component signature and call** that passed `sheet: Signal<Sheet>` and/or `method_spans` to take `parsed: Signal<ParsedSheet>` instead (SheetInspector, ExamplesPicker, and the reparse/hot-reload closures). Remove `MethodSpans` imports.

- [ ] **Step 4: Build and lint** (begin has no headless UI test harness — verify by compiling and the existing tests, then a manual run):

Run:
```bash
cargo build -p begin
cargo test -p begin
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: all PASS, zero warnings.

- [ ] **Step 5: Manually verify** a cycle diagnostic renders in-app. Use the `superpowers:verifying-begin-ui` skill (WebView2 app; no Playwright): load an example whose relationships form a cycle, confirm the error pane shows a multi-line backtrace naming each relationship. If no such example exists, add a minimal `begin/examples/cycle.adm2` and confirm.

- [ ] **Step 6: Commit**

```bash
git add begin/src/app.rs begin/src/example_source.rs begin/examples
git commit -m "refactor(begin): hold the whole ParsedSheet in one signal for error rendering (#188)"
```

---

## Phase 7 — Whole-branch verification

### Task 7.1: Full check suite

- [ ] **Step 1:** `cargo fmt --all`
- [ ] **Step 2:** `cargo build --workspace` — zero warnings (read the output).
- [ ] **Step 3:** `cargo test --workspace` and `cargo test --doc --workspace` — all pass, zero warnings.
- [ ] **Step 4:** all four clippy invocations from Global Constraints — zero warnings.
- [ ] **Step 5:** `RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace` — clean.
- [ ] **Step 6:** Update the handoff doc under `docs/superpowers/` summarizing what shipped, then open the PR with `Fixes #188` via the `pr-open` skill.

---

## Self-Review

**Spec coverage:**
- §1 `ErrorSite` + `sites()` → Task 1.1. §2 planner reconstruction → Tasks 2.1-2.5. §3 audit (each variant) → 1.1 (single-method + empty planner), 1.2 (mismatch/duplicate), 1.3 (cell kind), 1.4 (conditional), 2.3/2.4/2.5 (cycle/filtercycle/conflict). §4 span tables + `site_label` + `locate_error` + §4.1 build-time → 4.1-4.5. §5 `format_multi_span` + `ParseError` secondaries → 3.1, 3.2. §6 `format_adam_error` + begin consolidation → 5.1, 6.1. §7 testing → distributed per task. §8 migration → 1.1, 4.4, 5.1, 6.1. All covered.

**Placeholder scan:** Task 4.4's Step 3 initially sketched `proc_macro_start`/`build_range_error` then replaces them with the concrete `primary_pm`/`primary_idx`/`with_secondary` code — the concrete version is authoritative; the sketch is labelled as superseded. No `TBD`/`TODO` remain.

**Type consistency:** `sites: Vec<ErrorSite>` and `Error::sites() -> &[ErrorSite]` are used identically across all phases; `SpanLabel { span, label }` and `format_multi_span(title, &[SpanLabel], source, filename, start_line, renderer)` match between 3.1, 3.2, 4.4, 5.1; `BuildOutcome.sheet_labels: Option<(ParsedSheet, Labels)>` matches between 5.1 and 6.1; `locate_error -> Vec<(SourceSpan, String)>` matches between 4.3 and 5.1.

**Open items an executor must resolve while reading code (each flagged in-task):** the exact Source-cell constructor name (1.3); whether `slotmap` keys sort (2.3 — fallback provided); the exact `add_conditional` error mapping in `parse_conditional_decl` and whether the grammar can express a duplicate-branch relationship (4.5 — fallback provided); the precise rendered-output assertions for build-time carets (4.4, 4.5); the annotate-snippets 0.12 `.label()`/`AnnotationKind::Context` spelling (3.1).
