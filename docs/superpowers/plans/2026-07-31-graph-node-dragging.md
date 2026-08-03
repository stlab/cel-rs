# Graph Node Dragging Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user drag Cell, Relationship, and Conditional nodes in the `begin` graph view to manually untangle crossing edges, while keeping the D3 force simulation live so dragging one node pulls on the rest of the graph. Dropped nodes stay pinned; double-clicking a pinned node releases it back into the simulation.

**Architecture:** All logic lives in `begin/assets/graph.js` (a plain, unbundled script — no build step, no test runner) plus a small cursor-affordance addition in `begin/assets/graph.css`. A new `dragBehavior(simulation)` factory returns a configured `d3.drag()` that pins a node's `fx`/`fy` on the first actual `drag` movement (not `drag-start`, so a plain click with no movement stays a no-op), tracks the pointer during drag, and — critically — leaves `fx`/`fy` set on drag-end instead of clearing them. It's applied via `.call(...)` to the existing Cell/Relationship/Conditional join selections in `update()`. A separate `dblclick` handler clears `fx`/`fy` to unpin. No Rust changes are needed; `fx`/`fy` persistence across data updates falls out of the existing node-identity-preserving merge in `update()` with no code changes.

**Tech Stack:** Dioxus (Rust), D3.js v7 (vendored at `begin/assets/d3.v7.min.js`), plain JS/CSS assets served via `asset!`.

**Full context:** see the design spec at `docs/superpowers/specs/2026-07-31-graph-node-dragging-design.md`.

## Global Constraints

- `cargo fmt --all` must be run before every commit that touches `.rs` files (enforced by the pre-commit hook) — not applicable here since no `.rs` files change, but re-run it if that changes.
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`, `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`, and `cargo clippy -p begin --all-targets -- -D warnings` must all pass before opening a PR.
- Never commit directly to `main`.
- `graph.js` has no test runner in this repo — verification for JS-only tasks is manual, via `dx serve --platform desktop` run from `begin/`. Adding that test runner is tracked separately as [stlab/cel-rs#65](https://github.com/stlab/cel-rs/issues/65) and is explicitly out of scope for this plan.
- Branch junction nodes (`NodeKind::Branch`) have no visible shape and must stay excluded from drag — do not add a drag call to any Branch-node selection (none exists today).

---

## Task 1: Drag-to-pin, double-click-to-unpin, and cursor affordance

**Files:**
- Modify: `begin/assets/graph.js:307-344` (insert `dragBehavior`/`unpinNode` between `buildGraph` and `update`)
- Modify: `begin/assets/graph.js:461-469` (cell rect join — add `.call()`/`.on('dblclick', ...)`)
- Modify: `begin/assets/graph.js:472-478` (relationship circle join — same)
- Modify: `begin/assets/graph.js:538-545` (conditional diamond join — same)
- Modify: `begin/assets/graph.css` (cursor styling for `.node-cell`, `.node-relationship`, `.node-conditional`, plus a new `.dragging` modifier)

**Interfaces:**
- Consumes: module-level `simulation` (the live `d3.forceSimulation()` instance, already set up in `buildGraph()`); the existing `cellLayer`/`relLayer`/`condLayer` join selections inside `update()`.
- Produces: `dragBehavior(sim)` — a function taking the simulation instance, returning a `d3.drag()` behavior object suitable for `.call()` on a node selection. `unpinNode(event, d)` — a `dblclick` handler (signature matches any d3 `.on()` listener: `(event, datum)`, with `this` bound to the DOM element) that clears `d.fx`/`d.fy` and reheats the simulation. Neither is exported on `window.beginGraph` — both are internal to `graph.js`, called only from `update()`.

- [ ] **Step 1: Add the `dragBehavior` and `unpinNode` functions**

In `begin/assets/graph.js`, find the end of `buildGraph()` and the start of `update()`:

```javascript
        simulation.on('tick', function () {
            ticked();
            updateZoomConstraints();
        });

        update(data);
    }

    function update(data) {
```

Replace it with (inserting the two new functions between `buildGraph`'s closing `}` and `update`):

```javascript
        simulation.on('tick', function () {
            ticked();
            updateZoomConstraints();
        });

        update(data);
    }

    // Returns a d3.drag() behavior that pins a node's position while it's
    // being dragged and reheats `sim` so the rest of the graph reacts live.
    // Deliberately does NOT clear fx/fy on drag-end — the node stays exactly
    // where it was dropped; see unpinNode() for how a node is released.
    function dragBehavior(sim) {
        return d3.drag()
            .on('start', function (event, d) {
                // Both d3.zoom (on the <svg>) and this drag (on the node
                // shape) listen for pointer-down; without this the same
                // gesture would also pan the canvas.
                event.sourceEvent.stopPropagation();
            })
            .on('drag', function (event, d) {
                // 'start'/'end' fire on every pointerdown/pointerup, even a
                // plain click with no movement, but 'drag' only fires once
                // actual movement occurs — so gate the reheat and cursor
                // state on it (via the 'dragging' class, set at most once
                // per gesture here) to keep a no-movement click a true no-op.
                if (!d3.select(this).classed('dragging')) {
                    sim.alphaTarget(0.3).restart();
                    d3.select(this).classed('dragging', true);
                }
                d.fx = event.x;
                d.fy = event.y;
            })
            .on('end', function (event, d) {
                if (!event.active) sim.alphaTarget(0);
                d3.select(this).classed('dragging', false);
            });
    }

    // Releases a pinned node back into the free simulation.
    function unpinNode(event, d) {
        event.stopPropagation();
        d.fx = null;
        d.fy = null;
        simulation.alpha(Math.max(simulation.alpha(), 0.3)).restart();
    }

    function update(data) {
```

- [ ] **Step 2: Wire the cell rect join to drag + unpin**

In `begin/assets/graph.js`, change:

```javascript
        // Join cell rects
        cellLayer.selectAll('rect')
            .data(cellNodes, function (d) { return d.id; })
            .join('rect')
            .attr('class', 'node-cell')
            .attr('width', CELL_W)
            .attr('height', CELL_H)
            .attr('rx', CELL_RX);
```

to:

```javascript
        // Join cell rects
        cellLayer.selectAll('rect')
            .data(cellNodes, function (d) { return d.id; })
            .join('rect')
            .attr('class', 'node-cell')
            .attr('width', CELL_W)
            .attr('height', CELL_H)
            .attr('rx', CELL_RX)
            .call(dragBehavior(simulation))
            .on('dblclick', unpinNode);
```

- [ ] **Step 3: Wire the relationship circle join to drag + unpin**

In `begin/assets/graph.js`, change:

```javascript
        // Join relationship circles
        relLayer.selectAll('circle')
            .data(relNodes, function (d) { return d.id; })
            .join('circle')
            .attr('class', 'node-relationship')
            .attr('r', REL_R);
```

to:

```javascript
        // Join relationship circles
        relLayer.selectAll('circle')
            .data(relNodes, function (d) { return d.id; })
            .join('circle')
            .attr('class', 'node-relationship')
            .attr('r', REL_R)
            .call(dragBehavior(simulation))
            .on('dblclick', unpinNode);
```

- [ ] **Step 4: Wire the conditional diamond join to drag + unpin**

In `begin/assets/graph.js`, change:

```javascript
        // NEW: Conditional diamond nodes (rotated rect)
        condLayer.selectAll('rect')
            .data(condNodes, function (d) { return d.id; })
            .join('rect')
            .attr('class', 'node-conditional')
            .attr('width', COND_SIZE * 2)
            .attr('height', COND_SIZE * 2);
```

to:

```javascript
        // NEW: Conditional diamond nodes (rotated rect)
        condLayer.selectAll('rect')
            .data(condNodes, function (d) { return d.id; })
            .join('rect')
            .attr('class', 'node-conditional')
            .attr('width', COND_SIZE * 2)
            .attr('height', COND_SIZE * 2)
            .call(dragBehavior(simulation))
            .on('dblclick', unpinNode);
```

Note: Branch junction nodes have no join/selection anywhere in `graph.js` (they render as nothing), so there is no fourth call site to add — they're excluded from drag by simply never having a drag behavior attached, per the Global Constraints above.

- [ ] **Step 5: Add cursor styling**

In `begin/assets/graph.css`, change each of the three `cursor: default;` lines to `cursor: grab;`:

```css
.node-cell {
    fill: #fff;
    stroke: #444;
    stroke-width: 1.5;
    cursor: grab;
}

.node-relationship {
    fill: #fff;
    stroke: #444;
    stroke-width: 1.5;
    cursor: grab;
}
```

and

```css
.node-conditional {
    fill: #fff;
    stroke: #444;
    stroke-width: 1.5;
    cursor: grab;
}
```

Then append a new rule at the end of `begin/assets/graph.css`:

```css
.node-cell.dragging,
.node-relationship.dragging,
.node-conditional.dragging {
    cursor: grabbing;
}
```

- [ ] **Step 6: Run the full check suite**

```bash
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

Expected: all commands exit successfully, with zero warnings from `cargo build`/`cargo test` and zero clippy findings. (No Rust source changed in this task, so this is a regression check, not expected to surface anything new.)

- [ ] **Step 7: Manual verification via `dx serve`**

Run (from `begin/`): `dx serve --platform desktop`

1. Load the graph. Hover over a Cell, a Relationship, and a Conditional node; confirm the cursor changes to a "grab" hand over each (not the default arrow).
2. Click-drag a Cell node to a new position. Confirm: the cursor becomes a "grabbing" fist while dragging; connected nodes visibly shift/react during the drag (the simulation is reheated, not frozen); releasing the mouse leaves the node exactly where it was dropped (it does not spring back).
3. Repeat step 2 for a Relationship node and a Conditional node.
4. Drag a second node near the first. Confirm the first, already-pinned node does **not** move as a result of the second drag's simulation reheat — it stays exactly where it was pinned.
5. Double-click the first pinned node. Confirm it releases and visibly resettles into the simulation (it may drift briefly as forces re-equilibrate).
6. With one node still pinned, trigger a data update — e.g. edit `begin/assets/demo.pm` to change a cell's value/expression and save (hot reload) — and confirm the pinned node does not move when the update lands.
7. Click-drag on empty canvas background (not on a node). Confirm this still pans the view (drag-to-pan is unaffected) and does not accidentally pin/move any node.
8. If the loaded demo graph includes a multi-relationship branch (producing a `Branch` junction point — see `begin/CLAUDE.md`/the conditional-branch-junction-nodes design), confirm there is nothing to grab at that point (no cursor change, no drag) — this is expected since Branch nodes are invisible and excluded.

This step has no automated equivalent — it's the acceptance check for this task's deliverable, per the `verify` skill.

- [ ] **Step 8: Commit**

```bash
git add begin/assets/graph.js begin/assets/graph.css
git commit -m "$(cat <<'EOF'
feat(begin): add node dragging to the graph view

Adds a d3.drag() behavior to Cell/Relationship/Conditional nodes that
pins fx/fy on drag and reheats the simulation so the rest of the graph
reacts live, letting the user manually untangle crossing edges.
Dropped nodes stay pinned rather than springing back; double-clicking
a pinned node releases it. Branch junction nodes are excluded (no
visible shape to grab). Pinned position survives data updates for
free, since update() already preserves node identity across merges.
EOF
)"
```

---

## Self-Review Notes

- **Spec coverage:** `dragBehavior` factory (start/drag/end, pin-on-release) — Step 1. Applied to cell/rel/cond join selections — Steps 2-4. `dblclick` unpin — Steps 1-4. CSS cursor grab/grabbing — Step 5. No pan/drag conflict — `stopPropagation()` in Step 1, verified in Step 7.7. Branch-node exclusion — noted in Step 4 and verified in Step 7.8. Persistence across updates — no code needed (existing `update()` behavior), verified in Step 7.6. JS test harness gap — explicitly called out as out-of-scope/tracked in Global Constraints, matching the spec.
- **Placeholder scan:** no TBD/TODO; all steps contain literal code or literal verification actions.
- **Type consistency:** `dragBehavior(sim)` parameter name is local to that function (shadows nothing); call sites always pass the module-level `simulation`. `unpinNode(event, d)` matches d3's standard `.on()` listener signature, consistent with other handlers already in the file (e.g. `zoom.on('zoom', function (event) {...})`).
