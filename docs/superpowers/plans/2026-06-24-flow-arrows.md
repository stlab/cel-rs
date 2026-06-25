# Flow Arrows and Source Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show directed arrowheads on all D3 graph links (based on the planner's selected method) and skip re-planning when the edited cell is already a source.

**Architecture:** Cache `Plan::execution_order` in `Sheet` after each successful `propagate()`; expose it via accessor methods. The `begin` bridge uses these accessors to emit directed `LinkData` (source→target encodes direction; arrow at target end). The inspector calls `is_source()` before propagating and uses `propagate_without_replan()` when safe. The D3 JS replaces its placeholder arrowhead marker with two tuned markers and applies `marker-end` to every link.

**Tech Stack:** Rust (property-model, begin/src), D3.js v7, SVG marker API.

## Global Constraints

- Rust edition 2024; `cargo fmt --all` required before every commit (enforced by pre-commit hook).
- `cargo clippy --workspace -- -D warnings` must produce zero warnings.
- `cargo test --workspace` must pass after every task.
- Every new public function needs a `///` doc comment in contract style (see CLAUDE.md: summary, Preconditions, Errors, Complexity).
- Work on branch `sean-parent/flow-arrows`; never commit to `main`.

---

## Files

| File | Action | Responsibility |
|------|--------|---------------|
| `property-model/src/sheet.rs` | Modify | `last_plan` field; `execute_plan` private helper; `selected_method`, `method_inputs`, `method_outputs`, `is_source`, `propagate_without_replan` public methods |
| `begin/src/bridge.rs` | Modify | `arrows: bool` on `GraphData`; directed `LinkData` from plan |
| `begin/src/inspector.rs` | Modify | `oninput` uses `is_source` + `propagate_without_replan` |
| `begin/assets/graph.js` | Modify | Two tuned arrowhead markers; `marker-end` on all links |

---

## Task 1: Cache plan and expose method-level accessors in Sheet

**Files:**
- Modify: `property-model/src/sheet.rs`

**Interfaces produced (used by Tasks 2 and 3):**
- `Sheet::selected_method(rel: RelationshipId) -> Option<usize>`
- `Sheet::method_inputs(rel: RelationshipId, idx: usize) -> Option<&[CellId]>`
- `Sheet::method_outputs(rel: RelationshipId, idx: usize) -> Option<&[CellId]>`
- `Sheet::execute_plan(&mut self, execution_order: &[(RelationshipId, usize)]) -> Result<(), Error>` (private)

- [ ] **Step 1.1: Create feature branch**

```bash
git checkout -b sean-parent/flow-arrows
```

- [ ] **Step 1.2: Write failing tests**

Add inside the existing `mod tests` block in `property-model/src/sheet.rs`:

```rust
#[test]
fn selected_method_returns_none_before_propagate() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    assert!(sheet.selected_method(rel).is_none());
}

#[test]
fn selected_method_returns_index_after_propagate() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(sheet.selected_method(rel), Some(0));
}

#[test]
fn method_inputs_returns_inputs_for_valid_method() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    assert_eq!(sheet.method_inputs(rel, 0), Some([a].as_slice()));
}

#[test]
fn method_outputs_returns_outputs_for_valid_method() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    assert_eq!(sheet.method_outputs(rel, 0), Some([b].as_slice()));
}

#[test]
fn method_inputs_returns_none_for_out_of_bounds_idx() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    assert!(sheet.method_inputs(rel, 99).is_none());
}

#[test]
fn method_outputs_returns_none_for_out_of_bounds_idx() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    assert!(sheet.method_outputs(rel, 99).is_none());
}
```

- [ ] **Step 1.3: Run — expect compile error**

```
cargo test --workspace selected_method
```

Expected: compile error — `no method named 'selected_method' found for struct 'Sheet'`

- [ ] **Step 1.4: Add `last_plan` field to `Sheet` and `Sheet::new`**

In the struct definition add the field (after `next_strength`):

```rust
pub struct Sheet {
    pub(crate) cells: SlotMap<CellId, CellData>,
    pub(crate) relationships: SlotMap<RelationshipId, RelationshipData>,
    pub(crate) changed_cells: Vec<CellId>,
    next_strength: u64,
    last_plan: Option<Vec<(RelationshipId, usize)>>,
}
```

In `Sheet::new`, initialise the new field:

```rust
pub fn new() -> Self {
    Sheet {
        cells: SlotMap::with_key(),
        relationships: SlotMap::with_key(),
        changed_cells: Vec::new(),
        next_strength: 0,
        last_plan: None,
    }
}
```

- [ ] **Step 1.5: Extract `execute_plan` from `propagate` and cache the plan**

Replace the entire `propagate` method with the following two methods. The loop body is identical to the existing `propagate` body; it is just moved into `execute_plan`:

```rust
/// Runs the planning pass and executes the selected methods.
///
/// Clears the changed-cell set from the previous `propagate()` call before planning.
/// After propagation, call [`Sheet::changed`] to inspect which cells were updated,
/// and [`Sheet::clear_changed`] when done.
///
/// # Errors
///
/// - `Error::Conflict` — no valid method assignment exists.
/// - `Error::MethodFailed` — a method's function returned an error.
pub fn propagate(&mut self) -> Result<(), Error> {
    self.clear_changed();
    let plan = crate::planner::plan(&self.cells, &self.relationships)?;
    self.execute_plan(&plan.execution_order)?;
    self.last_plan = Some(plan.execution_order);
    Ok(())
}

/// Executes `execution_order` without invoking the planner.
///
/// - Complexity: O(R·K) where R is the number of entries and K is the max cells per method.
fn execute_plan(&mut self, execution_order: &[(RelationshipId, usize)]) -> Result<(), Error> {
    for &(rel_id, method_idx) in execution_order {
        let (outputs, output_ids) = {
            let method = &self.relationships[rel_id].methods[method_idx];
            let inputs: Vec<&dyn Any> = method
                .inputs
                .iter()
                .map(|&id| self.cells[id].value.as_ref())
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
            cell.value = new_value;
            if !cell.changed {
                cell.changed = true;
                self.changed_cells.push(cell_id);
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 1.6: Add the three public accessor methods**

Add after `execute_plan` (before `impl Default for Sheet`):

```rust
/// Returns the index of the method selected for `rel` in the last propagation.
///
/// Returns `None` if no propagation has run yet or `rel` is not in the cached plan.
pub fn selected_method(&self, rel: RelationshipId) -> Option<usize> {
    self.last_plan
        .as_ref()?
        .iter()
        .find(|&&(r, _)| r == rel)
        .map(|&(_, idx)| idx)
}

/// Returns the input cells of method `idx` in relationship `rel`.
///
/// Returns `None` if `rel` is not a live relationship or `idx` is out of bounds.
pub fn method_inputs(&self, rel: RelationshipId, idx: usize) -> Option<&[CellId]> {
    self.relationships
        .get(rel)?
        .methods
        .get(idx)
        .map(|m| m.inputs.as_slice())
}

/// Returns the output cells of method `idx` in relationship `rel`.
///
/// Returns `None` if `rel` is not a live relationship or `idx` is out of bounds.
pub fn method_outputs(&self, rel: RelationshipId, idx: usize) -> Option<&[CellId]> {
    self.relationships
        .get(rel)?
        .methods
        .get(idx)
        .map(|m| m.outputs.as_slice())
}
```

- [ ] **Step 1.7: Run tests — expect pass**

```
cargo test --workspace
```

Expected: all tests pass, zero warnings.

- [ ] **Step 1.8: Format and lint**

```
cargo fmt --all
cargo clippy --workspace -- -D warnings
```

Expected: exit 0, no output.

- [ ] **Step 1.9: Commit**

```bash
git add property-model/src/sheet.rs
git commit -m "feat(property-model): cache plan; expose selected_method, method_inputs, method_outputs"
```

---

## Task 2: Add `is_source` and `propagate_without_replan`

**Files:**
- Modify: `property-model/src/sheet.rs`

**Interfaces:**
- Consumes: `last_plan` and `execute_plan` from Task 1
- Produces:
  - `Sheet::is_source(id: CellId) -> bool`
  - `Sheet::propagate_without_replan(&mut self) -> Result<(), Error>`

- [ ] **Step 2.1: Write failing tests**

Add inside `mod tests` in `property-model/src/sheet.rs`:

```rust
#[test]
fn is_source_returns_false_before_propagate() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    assert!(!sheet.is_source(a));
}

#[test]
fn is_source_returns_true_for_input_cell_after_propagate() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    sheet.propagate().unwrap();
    assert!(sheet.is_source(a));
}

#[test]
fn is_source_returns_false_for_output_cell_after_propagate() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    sheet.propagate().unwrap();
    assert!(!sheet.is_source(b));
}

#[test]
fn propagate_without_replan_returns_conflict_before_propagate() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    assert!(matches!(
        sheet.propagate_without_replan(),
        Err(Error::Conflict)
    ));
}

#[test]
fn propagate_without_replan_executes_cached_plan() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
        .unwrap();
    sheet.propagate().unwrap();
    sheet.write(a, 5_i32).unwrap();
    sheet.propagate_without_replan().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 10);
}
```

- [ ] **Step 2.2: Run — expect compile error**

```
cargo test --workspace is_source
```

Expected: compile error — `no method named 'is_source' found for struct 'Sheet'`

- [ ] **Step 2.3: Implement `is_source` and `propagate_without_replan`**

Add after the Task 1 accessor methods (before `impl Default for Sheet`):

```rust
/// Returns `true` if `id` was not written by any selected method in the last propagation.
///
/// Returns `false` if no propagation has run yet (conservatively forces a full re-plan).
pub fn is_source(&self, id: CellId) -> bool {
    let Some(plan) = &self.last_plan else {
        return false;
    };
    !plan.iter().any(|&(rel_id, method_idx)| {
        self.relationships
            .get(rel_id)
            .and_then(|r| r.methods.get(method_idx))
            .map(|m| m.outputs.contains(&id))
            .unwrap_or(false)
    })
}

/// Re-executes the cached plan without invoking the planner.
///
/// - Precondition: Every cell written since the last `propagate()` satisfies
///   `is_source(id)`. Violation produces incorrect output values but no panic.
///
/// # Errors
///
/// - `Error::Conflict` — `propagate()` has not yet been called; no plan is cached.
/// - `Error::MethodFailed` — a method's function returned an error.
pub fn propagate_without_replan(&mut self) -> Result<(), Error> {
    let Some(execution_order) = self.last_plan.take() else {
        return Err(Error::Conflict);
    };
    self.clear_changed();
    let result = self.execute_plan(&execution_order);
    self.last_plan = Some(execution_order);
    result
}
```

Note: `take()` temporarily moves the plan out of `self` so `execute_plan` can mutably borrow `self`; it is restored unconditionally before returning.

- [ ] **Step 2.4: Run tests — expect pass**

```
cargo test --workspace
```

Expected: all tests pass, zero warnings.

- [ ] **Step 2.5: Format and lint**

```
cargo fmt --all
cargo clippy --workspace -- -D warnings
```

Expected: exit 0.

- [ ] **Step 2.6: Commit**

```bash
git add property-model/src/sheet.rs
git commit -m "feat(property-model): add is_source and propagate_without_replan"
```

---

## Task 3: Directed links in `bridge.rs`

**Files:**
- Modify: `begin/src/bridge.rs`

**Interfaces:**
- Consumes: `Sheet::selected_method`, `Sheet::method_inputs`, `Sheet::method_outputs` (Task 1)
- Produces: `GraphData::arrows: bool`; directed `LinkData` when plan is cached

**Background:** The bridge's existing `demo_sheet()` test helper adds `a` first (strength 1), `b` second (2), `c` third (3). With the only method being `[a,b]→c`, calling `propagate()` on that sheet returns `Err(Error::Conflict)` because `c` — highest strength — becomes a source and can't be overwritten. New propagation tests require a separate helper that adds `c` first so `a` and `b` have higher strength.

- [ ] **Step 3.1: Write failing tests**

Add a second test helper and four new tests inside `mod tests` in `begin/src/bridge.rs`:

```rust
// Separate helper that adds the output cell first so propagation succeeds.
fn demo_sheet_with_plan() -> (Sheet, Labels) {
    let mut sheet = Sheet::new();
    let mut labels = Labels::new();

    // c added first → lowest strength (output by default).
    let c = sheet.add_cell(0.0_f64);
    labels.add_cell::<f64>(c, "c");
    let a = sheet.add_cell(2.0_f64);
    labels.add_cell::<f64>(a, "a");
    let b = sheet.add_cell(3.0_f64);
    labels.add_cell::<f64>(b, "b");

    let rel = sheet
        .add_relationship(vec![Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| {
            Ok(x * y)
        })])
        .unwrap();
    labels.add_relationship(rel, "×");

    (sheet, labels)
}

#[test]
fn to_graph_data_arrows_false_before_propagate() {
    let (sheet, labels) = demo_sheet_with_plan();
    let data = to_graph_data(&sheet, &labels);
    assert!(!data.arrows);
}

#[test]
fn to_graph_data_arrows_true_after_propagate() {
    let (mut sheet, labels) = demo_sheet_with_plan();
    sheet.propagate().unwrap();
    let data = to_graph_data(&sheet, &labels);
    assert!(data.arrows);
}

#[test]
fn to_graph_data_directed_input_links_target_relationship() {
    // Method [a, b] → c; after propagate, a and b are inputs → 2 edges into rel.
    let (mut sheet, labels) = demo_sheet_with_plan();
    sheet.propagate().unwrap();
    let data = to_graph_data(&sheet, &labels);

    let rel_id = data
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Relationship)
        .map(|n| n.id.clone())
        .unwrap();

    let to_rel: Vec<_> = data.links.iter().filter(|l| l.target == rel_id).collect();
    assert_eq!(to_rel.len(), 2);
}

#[test]
fn to_graph_data_directed_output_links_source_relationship() {
    // Method [a, b] → c; after propagate, c is the output → 1 edge out of rel.
    let (mut sheet, labels) = demo_sheet_with_plan();
    sheet.propagate().unwrap();
    let data = to_graph_data(&sheet, &labels);

    let rel_id = data
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Relationship)
        .map(|n| n.id.clone())
        .unwrap();

    let from_rel: Vec<_> = data.links.iter().filter(|l| l.source == rel_id).collect();
    assert_eq!(from_rel.len(), 1);
}
```

- [ ] **Step 3.2: Run — expect compile error**

```
cargo test -p begin
```

Expected: compile error — `no field 'arrows' on type 'GraphData'`

- [ ] **Step 3.3: Add `arrows: bool` to `GraphData`**

```rust
#[derive(Serialize, Clone, PartialEq)]
pub struct GraphData {
    pub nodes: Vec<NodeData>,
    pub links: Vec<LinkData>,
    pub changed: Vec<String>,
    pub groups: Vec<GroupData>,
    pub arrows: bool,
}
```

- [ ] **Step 3.4: Update `to_graph_data` to produce directed links**

Replace the entire `to_graph_data` function body:

```rust
pub fn to_graph_data(sheet: &Sheet, labels: &Labels) -> GraphData {
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    let mut arrows = false;

    for id in sheet.cells() {
        let (label, value) = labels
            .cells
            .get(&id)
            .map(|m| (m.label.clone(), (m.display)(sheet)))
            .unwrap_or_default();
        nodes.push(NodeData {
            id: cell_node_id(id),
            kind: NodeKind::Cell,
            label,
            value,
        });
    }

    for id in sheet.relationships() {
        nodes.push(NodeData {
            id: rel_node_id(id),
            kind: NodeKind::Relationship,
            label: String::new(),
            value: String::new(),
        });

        if let Some(method_idx) = sheet.selected_method(id) {
            arrows = true;
            if let Some(inputs) = sheet.method_inputs(id, method_idx) {
                for &cell_id in inputs {
                    links.push(LinkData {
                        source: cell_node_id(cell_id),
                        target: rel_node_id(id),
                    });
                }
            }
            if let Some(outputs) = sheet.method_outputs(id, method_idx) {
                for &cell_id in outputs {
                    links.push(LinkData {
                        source: rel_node_id(id),
                        target: cell_node_id(cell_id),
                    });
                }
            }
        } else {
            if let Some(adj) = sheet.relationship_adj(id) {
                for &cell_id in adj {
                    links.push(LinkData {
                        source: cell_node_id(cell_id),
                        target: rel_node_id(id),
                    });
                }
            }
        }
    }

    let changed = sheet.changed().map(cell_node_id).collect();

    GraphData {
        nodes,
        links,
        changed,
        groups: vec![],
        arrows,
    }
}
```

- [ ] **Step 3.5: Run tests — expect pass**

```
cargo test --workspace
```

Expected: all tests pass. The existing `to_graph_data_produces_correct_link_count` assertion (`== 3`) still holds: without a plan it uses the undirected adjacency fallback, which yields 3 links (a, b, c all adjacent to the relationship).

- [ ] **Step 3.6: Format and lint**

```
cargo fmt --all
cargo clippy --workspace -- -D warnings
```

Expected: exit 0.

- [ ] **Step 3.7: Commit**

```bash
git add begin/src/bridge.rs
git commit -m "feat(begin): directed links and arrows flag in GraphData"
```

---

## Task 4: Source optimization in inspector

**Files:**
- Modify: `begin/src/inspector.rs`

**Interfaces:**
- Consumes: `Sheet::is_source(CellId) -> bool` (Task 2), `Sheet::propagate_without_replan() -> Result<(), Error>` (Task 2)

- [ ] **Step 4.1: Update `oninput` in `CellRow`**

In `begin/src/inspector.rs`, find the `oninput` handler and replace:

```rust
oninput: move |e| {
    let s = e.value();
    input.set(s.clone());
    let mut sheet_w = sheet.write();
    let labels_r = labels.read();
    if let Some(meta) = labels_r.cells.get(&id)
        && (meta.write_str)(&mut sheet_w, &s).is_ok()
    {
        has_error.set(sheet_w.propagate().is_err());
    }
},
```

with:

```rust
oninput: move |e| {
    let s = e.value();
    input.set(s.clone());
    let mut sheet_w = sheet.write();
    let labels_r = labels.read();
    if let Some(meta) = labels_r.cells.get(&id)
        && (meta.write_str)(&mut sheet_w, &s).is_ok()
    {
        let result = if sheet_w.is_source(id) {
            sheet_w.propagate_without_replan()
        } else {
            sheet_w.propagate()
        };
        has_error.set(result.is_err());
    }
},
```

- [ ] **Step 4.2: Run tests — expect pass**

```
cargo test --workspace
```

Expected: all tests pass.

- [ ] **Step 4.3: Format and lint**

```
cargo fmt --all
cargo clippy --workspace -- -D warnings
```

Expected: exit 0.

- [ ] **Step 4.4: Commit**

```bash
git add begin/src/inspector.rs
git commit -m "feat(begin): skip replanning when edited cell is a source"
```

---

## Task 5: Directed arrowheads in the D3 graph

**Files:**
- Modify: `begin/assets/graph.js`

**Interfaces:**
- Consumes: `data.arrows: bool` (Task 3), node `kind` field already present on every node object

**How the `refX` formula works:** The arrowhead path `M0,-5L10,0L0,5` places its tip at `(10, 0)` in marker coordinates. With `markerUnits="userSpaceOnUse"`, `refX` is in SVG pixels. Setting `refX = 10 + offset` means the reference point is `offset` pixels past the tip, so the tip lands `offset` pixels before the line endpoint (the node centre). Using `offset = REL_R` clips the tip to the circle edge; using `offset = CELL_W / 2` approximates the rect edge for most approach angles.

- [ ] **Step 5.1: Replace the placeholder `#arrowhead` marker with two tuned markers**

In `init()`, find and remove:

```javascript
// Arrowhead marker reserved for method-direction arrows
var defs = svg.append('defs');
defs.append('marker')
    .attr('id', 'arrowhead')
    .attr('viewBox', '0 -5 10 10')
    .attr('refX', 20).attr('refY', 0)
    .attr('markerWidth', 6).attr('markerHeight', 6)
    .attr('orient', 'auto')
    .append('path').attr('d', 'M0,-5L10,0L0,5').attr('fill', '#999');
```

Replace with:

```javascript
var defs = svg.append('defs');

// Arrow tip at relationship circle edge: refX = tip(10) + REL_R
defs.append('marker')
    .attr('id', 'arrowhead-to-rel')
    .attr('viewBox', '0 -5 10 10')
    .attr('refX', 10 + REL_R).attr('refY', 0)
    .attr('markerWidth', 8).attr('markerHeight', 8)
    .attr('markerUnits', 'userSpaceOnUse')
    .attr('orient', 'auto')
    .append('path').attr('d', 'M0,-5L10,0L0,5').attr('fill', '#999');

// Arrow tip at cell rect edge (approx): refX = tip(10) + CELL_W / 2
defs.append('marker')
    .attr('id', 'arrowhead-to-cell')
    .attr('viewBox', '0 -5 10 10')
    .attr('refX', 10 + CELL_W / 2).attr('refY', 0)
    .attr('markerWidth', 8).attr('markerHeight', 8)
    .attr('markerUnits', 'userSpaceOnUse')
    .attr('orient', 'auto')
    .append('path').attr('d', 'M0,-5L10,0L0,5').attr('fill', '#999');
```

- [ ] **Step 5.2: Rebuild `nodeMap` from updated nodes and apply `marker-end` to links**

In `update()`, the current first line is:
```javascript
var nodeMap = new Map(nodes.map(function (n) { return [n.id, n]; }));
```

This map is used only to merge positions from old nodes into new data. Rename it `oldNodeMap` and build a fresh `nodeMap` from the fully-updated `nodes` array so that `marker-end` lookup reflects current node kinds:

```javascript
function update(data) {
    if (!svg) return;

    var oldNodeMap = new Map(nodes.map(function (n) { return [n.id, n]; }));
    nodes = data.nodes.map(function (n) {
        var existing = oldNodeMap.get(n.id);
        if (existing) {
            existing.kind = n.kind;
            existing.label = n.label;
            existing.value = n.value;
            return existing;
        }
        return Object.assign({}, n);
    });
    var nodeMap = new Map(nodes.map(function (n) { return [n.id, n]; }));
    links = data.links.map(function (l) { return Object.assign({}, l); });

    var changedSet = new Set(data.changed || []);
    var cellNodes = nodes.filter(function (n) { return n.kind === 'Cell'; });
    var relNodes = nodes.filter(function (n) { return n.kind === 'Relationship'; });

    linkLayer.selectAll('line')
        .data(links, function (d) {
            var src = typeof d.source === 'object' ? d.source.id : d.source;
            var tgt = typeof d.target === 'object' ? d.target.id : d.target;
            return src + '-' + tgt;
        })
        .join('line')
        .attr('class', 'link')
        .attr('marker-end', function (d) {
            if (!data.arrows) return null;
            var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
            var tgtNode = nodeMap.get(tgtId);
            if (!tgtNode) return null;
            return tgtNode.kind === 'Cell'
                ? 'url(#arrowhead-to-cell)'
                : 'url(#arrowhead-to-rel)';
        });

    // Everything below this line is unchanged from the original update() body.
    cellLayer.selectAll('rect')
        .data(cellNodes, function (d) { return d.id; })
        .join('rect')
        .attr('class', 'node-cell')
        .attr('width', CELL_W)
        .attr('height', CELL_H)
        .attr('rx', CELL_RX);

    relLayer.selectAll('circle')
        .data(relNodes, function (d) { return d.id; })
        .join('circle')
        .attr('class', 'node-relationship')
        .attr('r', REL_R);

    labelLayer.selectAll('text')
        .data(cellNodes, function (d) { return d.id; })
        .join('text')
        .attr('class', 'node-label')
        .text(function (d) { return d.label; });

    valueLayer.selectAll('text')
        .data(cellNodes, function (d) { return d.id; })
        .join('text')
        .attr('class', 'node-value')
        .text(function (d) { return d.value || ''; });

    if (changedSet.size > 0) {
        cellLayer.selectAll('rect')
            .filter(function (d) { return changedSet.has(d.id); })
            .transition().duration(PULSE_ON_MS)
            .style('fill', PULSE_COLOR)
            .transition().duration(PULSE_OFF_MS)
            .style('fill', null);
    }

    simulation.nodes(nodes);
    simulation.force('link').links(links);
    simulation.alpha(0.3).restart();
}
```

- [ ] **Step 5.3: Run all tests**

```
cargo test --workspace
```

Expected: all tests pass.

- [ ] **Step 5.4: Build and visually verify**

```
cargo build --workspace
```

Then from the `begin/` directory:

```
dx serve --platform desktop
```

Open the app. Verify:
- All three links have arrowheads.
- Links from `a` and `b` to the `×` relationship node have arrowheads pointing into the circle.
- The link from `×` to `c` has an arrowhead pointing into the `c` rect.
- Editing `a` or `b` updates `c` and all arrowheads remain correct (arrows still point same direction — source optimization fires).
- Editing `c` (changing the output to a source) re-plans and reverses the arrows to point toward `a` or `b`.

- [ ] **Step 5.5: Commit**

```bash
git add begin/assets/graph.js
git commit -m "feat(begin): directed arrowheads on all graph links"
```
