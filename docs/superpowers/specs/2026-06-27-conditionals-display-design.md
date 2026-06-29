# Conditionals Display in Begin

**Date:** 2026-06-27
**Branch:** `worktree-conditionals-display`

## Goal

Render property-model conditionals in the `begin` D3 force graph and update the demo sheet to include a representative conditional.

## Approach

A new **diamond-shaped Conditional node** represents each conditional. The match cell connects to it with a normal constraint edge. From the diamond, dashed **control lines** run to each relationship belonging to each branch, color-coded by branch index. Inactive branch relationships and their control lines are dimmed to `INACTIVE_OPACITY`. The full conditional structure is always visible — nothing is hidden when a branch is inactive.

## Section 1 — Demo Sheet (`begin/src/app.rs`)

Replace the current two-relationship demo with:

- `a`, `b`, `c` — bidirectional constraint `a × b = c` (3 methods, unconditional)
- `d`, `e`, `f` — bidirectional constraint `d × e = f` (3 methods, unconditional)
- `p` — i32 match cell controlling the conditional
- Branch `p = 0`: one two-method relationship `{f → c: c = f, c → f: f = c}`
- Branch `p = 1`: one two-method relationship `{f → c: c = f × 2, c → f: f = c / 2}`
- Default (any other `p`): no relationships — the two systems are independent

Cell insertion order: `c` and `f` first (lowest strength, natural outputs), then `a`, `b`, `d`, `e`, `p` (higher strength, natural sources). `propagate()` called once at startup; `clear_changed()` called after so no cells pulse on first render.

## Section 2 — Property Model Public API (`property-model/src/sheet.rs`)

Six new methods on `Sheet`, all `pub`, all O(1) unless noted:

```rust
/// Iterates all live conditional IDs.
/// - Complexity: O(n)
pub fn conditionals(&self) -> impl Iterator<Item = ConditionalId> + '_

/// Returns the match cell for `id`.
pub fn conditional_match_cell(&self, id: ConditionalId) -> Option<CellId>

/// Returns the number of named branches in `id`.
pub fn conditional_branch_count(&self, id: ConditionalId) -> Option<usize>

/// Returns the relationship IDs for branch `branch` of conditional `id`.
pub fn conditional_branch_relationships(&self, id: ConditionalId, branch: usize) -> Option<&[RelationshipId]>

/// Returns the default relationship IDs for `id` (active when no branch matches).
pub fn conditional_default_relationships(&self, id: ConditionalId) -> Option<&[RelationshipId]>

/// Returns the index of the currently matching branch for `id`.
/// Returns `None` when no branch key matches the match cell's current value
/// (i.e., the default branch is active).
/// - Complexity: O(B·K) where B = branches, K = keys per branch.
pub fn conditional_active_branch(&self, id: ConditionalId) -> Option<usize>
```

Each method returns `None` for an invalid `ConditionalId`. `conditional_active_branch` re-runs the key-matching logic from `build_active_set` as a pure read (no mutation).

## Section 3 — Bridge (`begin/src/bridge.rs`)

### NodeKind

```rust
pub enum NodeKind { Cell, Relationship, Conditional }
```

Conditional node IDs: `"cond{ffi}"`.

### LinkData

```rust
pub enum LinkKind { Constraint, Control }

pub struct LinkData {
    pub source: String,
    pub target: String,
    pub kind: LinkKind,
    pub branch_index: Option<usize>,  // Some(_) for named branch Control links
    pub branch_active: Option<bool>,  // Some(_) for Control links
}
```

All existing links are `Constraint` with `None` optional fields. Backward-compatible — JS checks `kind`.

### GraphData

Remove the `groups: Vec<GroupData>` field and `GroupData` struct (were always empty, now superseded).

### `to_graph_data` additions

For each conditional from `sheet.conditionals()`:
1. Emit a `Conditional` node.
2. Emit a `Constraint` link: match cell → conditional node.
3. Call `sheet.conditional_active_branch(id)` to get the active branch index.
4. For each named branch `i` and each relationship in it: emit a `Control` link (conditional → relationship) with `branch_index: Some(i)`, `branch_active: Some(active_branch == Some(i))`.
5. For each default relationship: emit a `Control` link with `branch_index: None`, `branch_active: Some(active_branch.is_none())`.

## Section 4 — JavaScript (`begin/assets/graph.js`)

### New constants

```javascript
var COND_SIZE = 20;          // half-width/height of diamond
var BRANCH_COLORS = ['#4a90d9', '#e67e22'];  // blue, orange by branch index
var DEFAULT_BRANCH_COLOR = '#888';
var INACTIVE_OPACITY = 0.25;
```

### Layer order (z-order, back to front)

`bg-layer` → `controlLinkLayer` (new) → `linkLayer` → `cellLayer` → `relLayer` → `condLayer` (new) → `labelLayer` → `valueLayer`

Control links render below constraint links so they don't obscure them. Conditional diamonds render above relationship circles.

### Conditional node rendering

Diamond = SVG `rect` rotated 45° around its center:

```javascript
condLayer.selectAll('rect')
    .data(condNodes, d => d.id)
    .join('rect')
    .attr('class', 'node-conditional')
    .attr('width', COND_SIZE * 2).attr('height', COND_SIZE * 2);
// positioned in ticked(): translate(x, y) rotate(45)
```

### Control link rendering

Separate D3 join on `controlLinkLayer`. Each line:
- `stroke-dasharray: 5 3`
- `stroke`: `BRANCH_COLORS[branch_index]` if named branch, `DEFAULT_BRANCH_COLOR` for default
- `stroke-opacity`: 1.0 if `branch_active`, `INACTIVE_OPACITY` otherwise
- No arrowhead

### Inactive relationship dimming

After the control link join, compute the set of relationship node IDs that have at least one incoming control link but no active one. Apply `opacity: INACTIVE_OPACITY` to those relationship circles.

### Collision radius

Conditional nodes use `COND_SIZE * Math.sqrt(2)` (diagonal half-length) for the `forceCollide` radius.

## Section 5 — CSS (`begin/assets/graph.css`)

```css
.node-conditional {
    fill: #fffff0;
    stroke: #555;
    stroke-width: 1.5;
    cursor: default;
}
.link-control {
    stroke-width: 1.5;
    fill: none;
}
```

## Files to Change

| File | Change |
|------|--------|
| `property-model/src/sheet.rs` | Add 6 public accessor methods |
| `begin/src/app.rs` | Replace demo sheet with conditional example |
| `begin/src/bridge.rs` | `NodeKind`, `LinkKind`, `LinkData`, `GraphData`, `to_graph_data` |
| `begin/assets/graph.js` | New layers, conditional rendering, control links, dimming |
| `begin/assets/graph.css` | `.node-conditional`, `.link-control` |

## Out of Scope

- Inspector support for conditional nodes (no click-to-edit `p` via the graph)
- Animated branch transitions
- More than two branch colors
- Conditional nodes for the default branch (default is represented by gray control lines, not a separate node)
