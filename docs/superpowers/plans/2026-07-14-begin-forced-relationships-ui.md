# `begin`: Highlight Forced Relationships and Their Edges Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose which relationships the planner's method-elimination fixpoint leaves
with exactly one viable method (`property_model::Sheet::is_relationship_forced`/
`forced_relationships()`), thread that through `begin`'s `GraphData` bridge, and
highlight forced relationship nodes plus all of their constraint edges in the D3 graph
the same way forced cells are already highlighted.

**Architecture:** `planner::plan` already computes an `alive: HashMap<RelationshipId,
Vec<bool>>` fixpoint internally (a method dies when its pure output is guaranteed to be
produced by a different relationship); a relationship is forced when its `alive` vector
has exactly one `true` entry. `Plan` gains a `forced_relationships` field computed from
that same map. `Sheet` gains `last_forced_relationships` (set in `propagate()`, mirrors
`last_forced`) plus `is_relationship_forced`/`forced_relationships()` accessors mirroring
the existing cell-level API. `GraphData` gains a `forced_relationships: Vec<String>`
field populated the same way as `forced`. `graph.js`'s existing forced-highlighting IIFE
extends to toggle `.forced` on relationship circles and widen the `forced-edge`
predicate to include edges touching a forced relationship.

**Tech Stack:** Rust, `property-model` crate (planner/`Sheet`), `begin` crate
(`bridge.rs`, D3.js v7 `graph.js`, plain CSS `graph.css`).

## Global Constraints

- `cargo fmt --all` must be run before every commit (enforced by pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`,
  `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`, and
  `cargo clippy -p begin --all-targets -- -D warnings` must all be clean before the
  branch is considered done.
- Every function needs a `///` contract-style doc comment (summary, preconditions/
  postconditions only where non-obvious, `Complexity` bullet whenever not O(1)).
- Unit tests are derived from the contract/public interface only, never from
  implementation details.
- Never commit directly to `main`; this work happens on the
  `worktree-forced-relationships-ui` branch.

---

### Task 1: Planner computes `forced_relationships`

**Files:**
- Modify: `property-model/src/planner.rs:52-59` (`Plan` struct), `property-model/src/planner.rs:237-245` (`plan()` tail)
- Test: `property-model/src/planner.rs` (`#[cfg(test)] mod tests`, same file)

**Interfaces:**
- Consumes: the `alive: HashMap<RelationshipId, Vec<bool>>` map already computed by
  `forced_output_cells` and bound in `plan()` at `property-model/src/planner.rs:101`
  (`let (forced_outputs, alive) = forced_output_cells(relationships, active);`).
- Produces: `Plan::forced_relationships: HashSet<RelationshipId>` — active relationships
  with exactly one alive method. Task 2 consumes this field by name.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block at the bottom of
`property-model/src/planner.rs` (after `forced_outputs_cascade_through_adjacent_relationship`,
before `dead_method_not_selected_before_owning_relationship`):

```rust
    #[test]
    fn forced_relationships_true_for_single_method_relationship() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 3))])
            .unwrap();

        let active: HashSet<_> = sheet.relationships().collect();
        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();

        assert!(plan.forced_relationships.contains(&rel));
    }

    #[test]
    fn forced_relationships_excludes_multi_method_relationship() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0.0_f64);
        let b = sheet.add_cell(0.0_f64);
        let c = sheet.add_cell(0.0_f64);
        let rel = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok((*x) * (*y))),
                Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok((*y) / (*x))),
                Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok((*y) / (*x))),
            ])
            .unwrap();
        sheet.write(a, 2.0_f64).unwrap();
        sheet.write(b, 3.0_f64).unwrap();

        let active: HashSet<_> = sheet.relationships().collect();
        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();

        assert!(!plan.forced_relationships.contains(&rel));
    }

    #[test]
    fn forced_relationships_cascade_through_adjacent_relationship() {
        // R1: a -> b (single method) is trivially forced.
        // R2: b -> c or c -> b — c -> b dies once b is forced by R1 (it would
        // double-write b), leaving b -> c as R2's sole alive method, so R2 becomes
        // forced too even though it started with two methods.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(2_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let r1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 10))])
            .unwrap();
        let r2 = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(b, c, |x: &i32| Ok(*x + 1)),
                Method::from_fn_1_1(c, b, |x: &i32| Ok(*x + 1)),
            ])
            .unwrap();

        let active: HashSet<_> = sheet.relationships().collect();
        let plan = crate::planner::plan(&sheet.cells, &sheet.relationships, &active).unwrap();

        assert!(plan.forced_relationships.contains(&r1));
        assert!(plan.forced_relationships.contains(&r2));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p property-model forced_relationships`
Expected: compile error — `no field \`forced_relationships\` on type \`Plan\`` (the
struct literal at the end of `plan()` doesn't build one yet, and the tests read
`plan.forced_relationships`).

- [ ] **Step 3: Add the `forced_relationships` field and populate it**

In `property-model/src/planner.rs`, add a field to `Plan` (`planner.rs:52-59`):

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
```

At the tail of `plan()` (`planner.rs:237-245`), compute it from the `alive` map already
in scope and add it to the returned `Plan`:

```rust
    if selected.len() != active.len() {
        return Err(Error::Conflict);
    }

    let forced_relationships: HashSet<RelationshipId> = alive
        .iter()
        .filter(|(_, methods)| methods.iter().filter(|&&is_alive| is_alive).count() == 1)
        .map(|(&rel_id, _)| rel_id)
        .collect();

    Ok(Plan {
        execution_order: selected,
        forced_outputs,
        forced_relationships,
    })
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p property-model forced_relationships`
Expected: PASS (3 passed)

Run: `cargo test -p property-model`
Expected: all existing tests still pass (no other code reads `Plan`'s field set
directly).

- [ ] **Step 5: Commit**

```bash
git add property-model/src/planner.rs
git commit -m "$(cat <<'EOF'
feat(property-model): compute forced relationships in the planner

Plan::forced_relationships surfaces which active relationships the
forced-output fixpoint left with exactly one alive method, reusing the
`alive` map forced_output_cells already computes. Sheet-level exposure
follows in the next commit.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `Sheet` exposes `is_relationship_forced`/`forced_relationships`

**Files:**
- Modify: `property-model/src/sheet.rs:44-54` (`Sheet` struct fields), `property-model/src/sheet.rs:58-69` (`Sheet::new`), `property-model/src/sheet.rs:539-565` (`propagate`), `property-model/src/sheet.rs:682-694` (add new methods after `forced_cells`)
- Test: `property-model/tests/integration.rs`

**Interfaces:**
- Consumes: `Plan::forced_relationships` (Task 1).
- Produces: `Sheet::is_relationship_forced(&self, id: RelationshipId) -> bool` and
  `Sheet::forced_relationships(&self) -> impl Iterator<Item = RelationshipId> + '_`.
  Task 3 consumes `forced_relationships()` by name.

- [ ] **Step 1: Write the failing tests**

Add to `property-model/tests/integration.rs` (after `is_forced_respects_conditional_branch_activation`,
before `chained_relationships_execute_in_order`):

```rust
#[test]
fn is_relationship_forced_false_before_propagate() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    assert!(!sheet.is_relationship_forced(rel));
}

#[test]
fn is_relationship_forced_true_for_single_method_relationship() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(0_i32);
    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 3))])
        .unwrap();

    sheet.propagate().unwrap();

    assert!(sheet.is_relationship_forced(rel));
}

#[test]
fn is_relationship_forced_false_for_multi_method_relationship() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0.0_f64);
    let b = sheet.add_cell(0.0_f64);
    let c = sheet.add_cell(0.0_f64);
    let rel = sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok((*x) * (*y))),
            Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok((*y) / (*x))),
            Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok((*y) / (*x))),
        ])
        .unwrap();
    sheet.write(a, 2.0_f64).unwrap();
    sheet.write(b, 3.0_f64).unwrap();

    sheet.propagate().unwrap();

    assert!(!sheet.is_relationship_forced(rel));
}

#[test]
fn forced_relationships_cascade_through_adjacent_relationship() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(2_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);

    let r1 = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 10))])
        .unwrap();
    let r2 = sheet
        .add_relationship(vec![
            Method::from_fn_1_1(b, c, |x: &i32| Ok(*x + 1)),
            Method::from_fn_1_1(c, b, |x: &i32| Ok(*x + 1)),
        ])
        .unwrap();

    sheet.propagate().unwrap();

    assert!(sheet.is_relationship_forced(r1));
    assert!(sheet.is_relationship_forced(r2));
}

#[test]
fn forced_relationships_iterates_all_forced_relationships() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(2_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);

    let r1 = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 10))])
        .unwrap();
    let r2 = sheet
        .add_relationship(vec![
            Method::from_fn_1_1(b, c, |x: &i32| Ok(*x + 1)),
            Method::from_fn_1_1(c, b, |x: &i32| Ok(*x + 1)),
        ])
        .unwrap();

    sheet.propagate().unwrap();

    let forced: std::collections::HashSet<_> = sheet.forced_relationships().collect();
    assert_eq!(forced, std::collections::HashSet::from([r1, r2]));
}

#[test]
fn is_relationship_forced_respects_conditional_branch_activation() {
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

    // mode=0: rel_on inactive (not part of the planned active set at all).
    sheet.write(mode, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(!sheet.is_relationship_forced(rel_on));

    // mode=1: rel_on active, single method, forced.
    sheet.write(mode, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(sheet.is_relationship_forced(rel_on));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p property-model --test integration is_relationship_forced`
Expected: compile error — `no method named \`is_relationship_forced\` found for struct
\`Sheet\`` (and similarly for `forced_relationships`).

- [ ] **Step 3: Add the `Sheet` field and methods**

In `property-model/src/sheet.rs`, add a field next to `last_forced` (`sheet.rs:44-54`):

```rust
    last_plan: Option<Vec<(RelationshipId, usize)>>,
    /// Cells reported forced (see [`Sheet::is_forced`]) by the last full `propagate()`
    /// call. Not recomputed by `propagate_without_replan`.
    last_forced: Option<HashSet<CellId>>,
    /// Relationships reported forced (see [`Sheet::is_relationship_forced`]) by the
    /// last full `propagate()` call. Not recomputed by `propagate_without_replan`.
    last_forced_relationships: Option<HashSet<RelationshipId>>,
```

Initialize it in `Sheet::new()` (`sheet.rs:58-69`):

```rust
            last_plan: None,
            last_forced: None,
            last_forced_relationships: None,
```

Set it in `propagate()` (`sheet.rs:562-563`), alongside the existing `last_forced`
assignment:

```rust
        self.last_forced = Some(plan.forced_outputs);
        self.last_forced_relationships = Some(plan.forced_relationships);
        self.last_plan = Some(plan.execution_order);
```

Add two methods after `forced_cells` (`sheet.rs:688-694`):

```rust
    /// Returns `true` if `id` had exactly one viable method as of the last successful
    /// `propagate()` call — the planner has no alternative method to choose for this
    /// relationship, regardless of cell strength.
    ///
    /// Returns `false` if no propagation has run yet.
    pub fn is_relationship_forced(&self, id: RelationshipId) -> bool {
        self.last_forced_relationships
            .as_ref()
            .is_some_and(|forced| forced.contains(&id))
    }

    /// Iterates relationships that are forced (see [`Sheet::is_relationship_forced`])
    /// as of the last `propagate()` call.
    ///
    /// - Complexity: O(n) where n is the number of forced relationships.
    pub fn forced_relationships(&self) -> impl Iterator<Item = RelationshipId> + '_ {
        self.last_forced_relationships.iter().flatten().copied()
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p property-model --test integration is_relationship_forced`
Run: `cargo test -p property-model --test integration forced_relationships`
Expected: PASS (6 passed total across both filters)

Run: `cargo test -p property-model`
Expected: all tests in the crate pass, no regressions.

- [ ] **Step 5: Commit**

```bash
git add property-model/src/sheet.rs property-model/tests/integration.rs
git commit -m "$(cat <<'EOF'
feat(property-model): expose Sheet::is_relationship_forced/forced_relationships

Mirrors the existing is_forced/forced_cells cell-level API at the
relationship level, backed by Plan::forced_relationships from the
previous commit.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `GraphData` reports forced relationships

**Files:**
- Modify: `begin/src/bridge.rs:199-206` (`GraphData` struct), `begin/src/bridge.rs:353-363` (`to_graph_data` tail)
- Test: `begin/src/bridge.rs` (`#[cfg(test)] mod tests`, same file)

**Interfaces:**
- Consumes: `property_model::Sheet::forced_relationships(&self) -> impl Iterator<Item = RelationshipId> + '_`
  (Task 2); the existing private `rel_node_id(id: RelationshipId) -> String` helper in
  `bridge.rs:212-214`.
- Produces: `GraphData::forced_relationships: Vec<String>` — stable relationship-node
  IDs (`"r{ffi}"`) of relationships forced as of the last `propagate()`. Task 4 consumes
  this field by name (as `data.forced_relationships` in JS).

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `begin/src/bridge.rs` (after
`to_graph_data_forced_field_excludes_cell_when_branch_inactive`, at the end of the
block):

```rust
    #[test]
    fn to_graph_data_forced_relationships_field_contains_forced_relationship() {
        let (mut sheet, labels) = sheet_with_forced_conditional();
        let rel_id = sheet.relationships().next().unwrap();
        sheet.propagate().unwrap();

        let data = to_graph_data(&sheet, &labels);
        assert!(data.forced_relationships.contains(&rel_node_id(rel_id)));
    }

    #[test]
    fn to_graph_data_forced_relationships_field_excludes_relationship_when_branch_inactive() {
        let (mut sheet, labels) = sheet_with_forced_conditional();
        let rel_id = sheet.relationships().next().unwrap();
        let p_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("p"))
            .unwrap();
        sheet.write(p_id, 1_i32).unwrap();
        sheet.propagate().unwrap();

        let data = to_graph_data(&sheet, &labels);
        assert!(!data.forced_relationships.contains(&rel_node_id(rel_id)));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p begin --no-default-features to_graph_data_forced_relationships`
Expected: compile error — `no field \`forced_relationships\` on type \`GraphData\`` (the
struct literal at the end of `to_graph_data` doesn't build one yet, and the tests read
`data.forced_relationships`).

- [ ] **Step 3: Add the `forced_relationships` field and populate it**

In `begin/src/bridge.rs`, add a field to `GraphData` (after the existing `forced`
field, `bridge.rs:199-202`):

```rust
    /// Stable IDs of cells forced by an active relationship (see
    /// [`property_model::Sheet::is_forced`]); consumers should disable input for these
    /// cells and may render them distinctly.
    pub forced: Vec<String>,
    /// Stable IDs of relationships forced by the planner (see
    /// [`property_model::Sheet::is_relationship_forced`]); consumers may render them
    /// distinctly, along with their constraint edges.
    pub forced_relationships: Vec<String>,
```

At the tail of `to_graph_data` (`bridge.rs:353-363`), populate it alongside `forced`:

```rust
    let changed = sheet.changed().map(cell_node_id).collect();
    let forced = sheet.forced_cells().map(cell_node_id).collect();
    let forced_relationships = sheet.forced_relationships().map(rel_node_id).collect();

    GraphData {
        nodes,
        links,
        changed,
        forced,
        forced_relationships,
        arrows,
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p begin --no-default-features to_graph_data_forced_relationships`
Expected: PASS (2 passed)

Run: `cargo test -p begin --no-default-features`
Expected: all existing `bridge.rs` tests still pass (the new field doesn't change any
existing assertion, since none of them check `GraphData`'s field set directly except
`to_graph_data_no_groups_field`, which only asserts the JSON doesn't contain
`"groups"` — unaffected by adding `forced_relationships`).

- [ ] **Step 5: Commit**

```bash
git add begin/src/bridge.rs
git commit -m "$(cat <<'EOF'
feat(begin): report forced relationships in GraphData

GraphData::forced_relationships mirrors the existing `forced` field,
populated from Sheet::forced_relationships(), so graph_view/graph.js
can highlight relationship nodes (and their constraint edges) that the
planner has no alternative method for, regardless of cell strength.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Graph highlights forced relationships and their edges

**Files:**
- Modify: `begin/assets/graph.js:393-409` (the forced-highlighting IIFE inside `update()`)
- Modify: `begin/assets/graph.css:65-73` (after the existing `.node-cell.forced`/`.link.forced-edge` rules)

**Interfaces:**
- Consumes: `GraphData.forced_relationships: Vec<String>` (Task 3) as
  `data.forced_relationships` in JS.
- Produces: `.forced` class also applied to relationship `<circle>`s — purely visual,
  no other task depends on this.

- [ ] **Step 1: Extend the forced-highlighting IIFE**

In `begin/assets/graph.js`, replace the existing forced-highlighting IIFE
(`graph.js:393-409`):

```javascript
        // Highlight forced cells (see property_model::Sheet::is_forced) and every
        // constraint edge touching one: the incoming edge that produces it, and any
        // outgoing edges carrying its (also guaranteed) value onward to other
        // relationships. Forced cells always belong to a currently active
        // relationship, so this never overlaps with the inactive-relationship
        // dimming above.
        (function () {
            var forcedSet = new Set(data.forced || []);
            cellLayer.selectAll('rect')
                .classed('forced', function (d) { return forcedSet.has(d.id); });
            linkLayer.selectAll('line')
                .classed('forced-edge', function (d) {
                    var srcId = typeof d.source === 'object' ? d.source.id : d.source;
                    var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                    return forcedSet.has(srcId) || forcedSet.has(tgtId);
                });
        }());
```

with:

```javascript
        // Highlight forced cells (see property_model::Sheet::is_forced) and forced
        // relationships (see property_model::Sheet::is_relationship_forced) — those
        // with only one viable method, regardless of cell strength — plus every
        // constraint edge touching either: the incoming edge into a forced
        // relationship, its outgoing edge(s), and any further edges carrying a
        // forced cell's guaranteed value onward. Both always belong to a currently
        // active relationship, so this never overlaps with the inactive-relationship
        // dimming above.
        (function () {
            var forcedSet = new Set(data.forced || []);
            var forcedRelSet = new Set(data.forced_relationships || []);
            cellLayer.selectAll('rect')
                .classed('forced', function (d) { return forcedSet.has(d.id); });
            relLayer.selectAll('circle')
                .classed('forced', function (d) { return forcedRelSet.has(d.id); });
            linkLayer.selectAll('line')
                .classed('forced-edge', function (d) {
                    var srcId = typeof d.source === 'object' ? d.source.id : d.source;
                    var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                    return forcedSet.has(srcId) || forcedSet.has(tgtId)
                        || forcedRelSet.has(srcId) || forcedRelSet.has(tgtId);
                });
        }());
```

- [ ] **Step 2: Add the CSS rule**

In `begin/assets/graph.css`, after the existing `.node-cell.forced`/`.link.forced-edge`
rules (end of file, `graph.css:65-73`), add:

```css
.node-relationship.forced {
    stroke: #8e44ad;
    stroke-width: 3;
}
```

- [ ] **Step 3: Manually verify in the running app**

Use the `verifying-begin-ui` skill (`.claude/skills/verifying-begin-ui/SKILL.md`) or run
`dx serve --platform desktop` from `begin/` directly. Steps:
1. Load the default demo graph (`begin/assets/demo.pm`), `p` starts at `0`.
2. Set `p` to `1`.
3. Confirm: `g`'s cell rect is purple/thick-stroked (pre-existing behavior), the
   `[c] -> g` relationship's circle is now also purple/thick-stroked, and the
   constraint edge from `c` into that relationship is now also purple/thick — none of
   these three were highlighted together before this task.
4. Set `p` back to `0` and confirm all three revert to their normal (non-forced)
   styling.

- [ ] **Step 4: Commit**

```bash
git add begin/assets/graph.js begin/assets/graph.css
git commit -m "$(cat <<'EOF'
feat(begin): highlight forced relationships and their edges in the graph

Relationships reported by GraphData::forced_relationships (see prior
commit) get the same purple outline as forced cells, and every
constraint edge touching one — including edges from inputs that aren't
themselves forced cells — is highlighted too, showing the section of
the graph the planner has already determined independent of cell
strength.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Full workspace verification

**Files:** none (verification only)

**Interfaces:**
- Consumes: everything from Tasks 1-4.
- Produces: nothing new; confirms the branch is ready to hand off per root
  `CLAUDE.md`'s "Before creating a PR" checklist.

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Expected: no changes (already formatted per-task), or if it does reformat something,
stage and include it in the commit below.

- [ ] **Step 2: Build the whole workspace**

Run: `cargo build --workspace`
Expected: builds cleanly, zero warnings.

- [ ] **Step 3: Run the full test suite**

Run: `cargo test --workspace`
Run: `cargo test --doc --workspace`
Expected: all pass, no regressions anywhere in the workspace.

- [ ] **Step 4: Lint the whole workspace**

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Run: `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
Run: `cargo clippy -p begin --all-targets -- -D warnings`
Expected: no warnings from any of the three invocations.

- [ ] **Step 5: Commit any formatting fixes (only if Step 1 produced changes)**

```bash
git add -A
git commit -m "$(cat <<'EOF'
style: cargo fmt

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```
