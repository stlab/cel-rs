# Graph Node Dragging

**Date:** 2026-07-31
**Branch:** `worktree-begin-link-condtional-branches`

## Goal

The `begin` graph view is a pure D3 force simulation with no manual layout control — as graphs
grow, edges cross and pass under nodes with no way for the user to untangle them. True automatic
crossing-minimization is out of scope (this graph mixes bipartite and hierarchical structure, and
a layered layout would fight the existing organic force feel). Instead, let the user manually drag
nodes to positions of their choosing, while keeping the force simulation live so dragging one node
pulls on the rest of the graph.

## Approach

Add standard D3 drag behavior (`d3.drag()`) to Cell, Relationship, and Conditional node shapes.
Dragging pins a node via `fx`/`fy` and reheats the simulation (`alphaTarget`) so the rest of the
graph reacts live; on release the node **stays pinned** at its dropped position rather than
springing back. Double-clicking a pinned node clears `fx`/`fy`, releasing it back into the free
simulation — the classic D3 "sticky force layout" idiom. Branch junction nodes (invisible,
zero-radius routing points introduced for multi-relationship branches) are excluded from drag —
they have no visible hit target.

Pinned state persists automatically: `update()` already preserves node identity across data
refreshes by reusing the existing node object when merging incoming data
([graph.js:311-322](../../../begin/assets/graph.js#L311-L322)), so `fx`/`fy` set by a drag survive
label/value updates and `settleSimulation()` re-ticks without extra code. A node that's
structurally removed and later re-added starts fresh (unpinned) — this is expected.

### Approaches considered

1. **Standard `d3.drag()` with pin-on-release (chosen).** Well-understood D3 idiom, no new UI
   chrome beyond a cursor change, and it directly addresses "let me untangle this by hand."
2. **Algorithmic crossing-minimization layout** (e.g. Sugiyama-style layering respecting control-
   link direction). Rejected: this graph isn't purely hierarchical (cells/relationships form an
   undirected constraint mesh; only control links are directed), so a layered layout would need to
   coexist with or replace the force simulation, a much larger and riskier change for uncertain
   payoff. Left as a possible future direction if manual dragging proves insufficient.
3. **Drag with spring-back on release** (clear `fx`/`fy` in `dragend`). Rejected per user
   preference — any manual untangling would be undone on the very next simulation tick.

## Section 1 — Rendering (`begin/assets/graph.js`)

Add a `dragBehavior(simulation)` factory returning a configured `d3.drag()`:

- **`start`**: `event.sourceEvent.stopPropagation()` (prevents the SVG's own pan/zoom drag from
  also firing on the same gesture — `zoom` and this drag are bound to different elements but both
  listen for pointer-down); `if (!event.active) simulation.alphaTarget(0.3).restart();`; pin
  `d.fx = d.x; d.fy = d.y;`.
- **`drag`**: `d.fx = event.x; d.fy = event.y;`.
- **`end`**: `if (!event.active) simulation.alphaTarget(0);` — deliberately does **not** clear
  `fx`/`fy`, so the node stays where dropped.

Apply `.call(dragBehavior(simulation))` to the `cellLayer`, `relLayer`, and `condLayer` join
selections in `update()` (the same merged selections already used for attribute joins) — not to
`controlLinkLayer`/`linkLayer` (edges aren't draggable) and not to any Branch-node selection (none
exists; Branch nodes have no shape today).

Add a separate `dblclick` handler on the same three selections: clears `d.fx = null; d.fy = null;`
and gives the simulation a small alpha bump (`simulation.alpha(Math.max(simulation.alpha(), 0.3)).restart();`)
so the released node visibly settles back into the layout instead of sitting inert until the next
unrelated tick.

## Section 2 — Styling (`begin/assets/graph.css`)

Change `cursor: default` to `cursor: grab` on `.node-cell`, `.node-relationship`, and
`.node-conditional`. Add a `.dragging` modifier (toggled on the dragged element's class list in
`start`/`end`) setting `cursor: grabbing`, so the cursor reflects active-drag state.

## Section 3 — Testing & verification

No existing JS test harness covers `graph.js` (tracked separately as
[stlab/cel-rs#65](https://github.com/stlab/cel-rs/issues/65) — deferred, not a blocker here).
Verification is manual via the `verifying-begin-ui` skill:

- Drag a Cell, a Relationship, and a Conditional node; confirm each stays exactly where dropped
  after release, and that other connected nodes visibly react (simulation reheats) during the
  drag.
- Confirm dragging a node does not also pan the canvas.
- Double-click a pinned node and confirm it releases back into the simulation and resettles.
- Trigger a data update (e.g. edit a cell value) while a node is pinned and confirm the pinned
  node does not move.
- Confirm Branch junction nodes (present only in graphs with a multi-relationship branch) are
  unaffected — no cursor change, no drag.
- `cargo build --workspace`, `cargo clippy -p begin --all-targets -- -D warnings` (JS is unaffected
  by Rust toolchecks, but this confirms no incidental Rust-side changes were introduced).

## Files to change

| File | Change |
|------|--------|
| `begin/assets/graph.js` | `dragBehavior` factory, `.call()` on node join selections, `dblclick` unpin handler |
| `begin/assets/graph.css` | `cursor: grab`/`grabbing` on draggable node classes |

## Out of scope

- Automatic crossing-minimization or alternative layout algorithms (see approaches considered).
- Dragging Branch junction nodes or edges.
- A UI affordance for "unpin all" or visually indicating which nodes are currently pinned beyond
  the cursor change.
- Adding a JS unit test harness (tracked as [stlab/cel-rs#65](https://github.com/stlab/cel-rs/issues/65)).
