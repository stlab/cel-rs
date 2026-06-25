# Flow Arrows and Source Optimization — Design Spec

**Date:** 2026-06-24  
**Status:** Approved

## Summary

Add directed arrowheads to the D3 force graph in the `begin` demo app so the viewer can see which cells are inputs and which are outputs for each relationship's selected method. Simultaneously implement the planner source-cell optimization: when the user edits a cell that the planner already designated as a source, skip re-planning and re-execute the last cached plan directly.

---

## Context

The `begin` crate renders a D3 bipartite graph of cells and relationships. Links are currently undirected `<line>` elements. The D3 `graph.js` file already defines an `#arrowhead` SVG marker (placeholder, never applied). The planner (`property-model/src/planner.rs`) selects one method per relationship at propagation time, but the result is never stored — `Sheet::propagate()` always re-plans from scratch, even when re-planning cannot change the outcome.

---

## Design

### Layer 1 — `property-model/src/sheet.rs`: expose plan

Add a `last_plan` field to `Sheet` that caches the most recent successful execution order:

```rust
last_plan: Option<Vec<(RelationshipId, usize)>>,
```

Populate it at the end of every successful `propagate()` call.

Add the following public methods:

| Method | Description |
|--------|-------------|
| `selected_method(rel: RelationshipId) -> Option<usize>` | Index of the method selected for `rel` in the last plan. `None` if no plan has run yet. |
| `method_inputs(rel: RelationshipId, idx: usize) -> Option<&[CellId]>` | Inputs of method `idx` in `rel`. `None` if `rel` or `idx` is invalid. |
| `method_outputs(rel: RelationshipId, idx: usize) -> Option<&[CellId]>` | Outputs of method `idx` in `rel`. `None` if `rel` or `idx` is invalid. |
| `is_source(id: CellId) -> bool` | Returns `true` if `id` was not written by any selected method in the last plan. `false` if no plan has run yet (conservatively forces re-plan). |
| `propagate_without_replan() -> Result<(), Error>` | Re-executes `last_plan` without invoking the planner. Returns `Error::Conflict` if no plan is cached. |

**Contract for `propagate_without_replan`:** The caller is responsible for ensuring the cached plan remains valid (i.e., the modified cell is a source). When this pre-condition holds, the execution order is identical to what a fresh plan would produce, so the result is correct.

### Layer 2 — `begin/src/bridge.rs`: directed links

Add `arrows: bool` to `GraphData`:

```rust
pub struct GraphData {
    pub nodes: Vec<NodeData>,
    pub links: Vec<LinkData>,
    pub changed: Vec<String>,
    pub groups: Vec<GroupData>,
    pub arrows: bool,   // ← new
}
```

Change `to_graph_data` link generation. When the sheet has a cached plan (`selected_method` returns `Some`), emit directed `LinkData`:

- **Input edges** (`cell → relationship`): `source = cell_node_id(cell)`, `target = rel_node_id(rel)` — arrow tip lands on the relationship node.
- **Output edges** (`relationship → cell`): `source = rel_node_id(rel)`, `target = cell_node_id(cell)` — arrow tip lands on the derived cell.

Set `arrows: true`.

When no plan is cached, fall back to the existing undirected adjacency-based links and set `arrows: false`.

### Layer 3 — `begin/src/inspector.rs`: source optimization

In `CellRow::oninput`, replace the unconditional `propagate()` call:

```rust
// before
has_error.set(sheet_w.propagate().is_err());

// after
let result = if sheet_w.is_source(id) {
    sheet_w.propagate_without_replan()
} else {
    sheet_w.propagate()
};
has_error.set(result.is_err());
```

The optimization is safe because `write_str` raises the cell's strength, and a source cell's relative rank cannot decrease (it was already not overwritten by any method; increasing its strength cannot make it an output).

### Layer 4 — `begin/assets/graph.js` and `graph.css`: render arrowheads

#### Markers

Replace the existing placeholder `#arrowhead` marker with two tuned markers, both using `markerUnits="userSpaceOnUse"`:

| Marker ID | Target node type | `refX` formula | Rationale |
|-----------|-----------------|----------------|-----------|
| `arrowhead-to-rel` | Relationship circle (`r = REL_R = 16`) | `10 + REL_R` = 26 | Places tip at circle edge |
| `arrowhead-to-cell` | Cell rect (half-width `CELL_W/2 = 30`) | `10 + CELL_W/2` = 40 | Places tip approximately at rect edge |

Both markers use the same arrowhead path `M0,-5L10,0L0,5` (tip at x=10). With `markerUnits="userSpaceOnUse"` and the refX formula, the tip lands at the node boundary for links arriving horizontally; off-axis links clip slightly inside the node, which is acceptable for this first pass.

#### Applying markers

In `update()`, after joining links, apply `marker-end` conditionally:

```javascript
.attr('marker-end', function(d) {
    if (!data.arrows) return null;
    var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
    var tgtNode = nodeMap.get(tgtId);
    if (!tgtNode) return null;
    return tgtNode.kind === 'Cell'
        ? 'url(#arrowhead-to-cell)'
        : 'url(#arrowhead-to-rel)';
})
```

The `nodeMap` is already built at the start of `update()`.

The `ticked` function is unchanged; line endpoints still go to node centers and `refX` handles the visual offset.

#### CSS

Add a `.link` rule for `marker-end` color consistency (if needed — the marker path already sets `fill: #999`).

---

## What is not changing

- The `planner.rs` module itself is not modified.
- `Plan` remains `pub(crate)`; `Sheet` wraps it.
- The D3 force simulation and layout logic are unchanged.
- `GraphData.groups` remains a reserved empty vec.

---

## Testing

### `property-model`
- `selected_method` returns `None` before any `propagate()`.
- After `propagate()`, `selected_method` returns the correct index for each relationship.
- `is_source` returns `false` for output cells, `true` for source cells.
- `is_source` returns `false` before any propagation.
- `propagate_without_replan` returns `Error::Conflict` if called before any `propagate()`.
- `propagate_without_replan` produces the same output as `propagate` when called on a source cell.
- `method_inputs` and `method_outputs` return `None` for invalid IDs.

### `begin/src/bridge.rs`
- When plan is available, `to_graph_data` produces directed links with correct source/target order.
- `links.len()` equals the total input + output count across all selected methods (may differ from the current adjacency-based count in multi-method relationships).
- `arrows` is `true` when plan is cached, `false` otherwise.

---

## Open questions

None. The "approximate clipping" for off-axis cell arrows is an accepted limitation of this first pass.
