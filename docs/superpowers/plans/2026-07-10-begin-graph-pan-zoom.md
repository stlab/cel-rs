# begin Graph View: Fixed Sizing + Pan/Zoom Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the graph view so it sizes correctly to its container on load and never rescales on window/container resize, then add pan and zoom with a fit-scale floor and content-bounds pan clamp, plus on-screen zoom controls.

**Architecture:** All logic lives in `begin/assets/graph.js` (a plain, unbundled script — no build step, no test runner). `init()` is split so the container is measured via a one-shot `ResizeObserver` before the `<svg>` is built with fixed pixel dimensions. All existing D3 layers are wrapped in a new `g.zoom-layer` that a `d3.zoom()` behavior transforms; `scaleExtent`/`translateExtent` are recomputed from the live node bounding box every time the graph structure changes. Three small button handlers are exposed on `window.beginGraph` and wired to new RSX buttons in `begin/src/graph_view.rs`.

**Tech Stack:** Dioxus (Rust), D3.js v7 (vendored at `begin/assets/d3.v7.min.js`), plain JS/CSS assets served via `asset!`.

**Full context:** see the design spec at `docs/superpowers/specs/2026-07-10-begin-graph-pan-zoom-design.md`.

## Global Constraints

- `cargo fmt --all` must be run before every commit that touches `.rs` files (enforced by the pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`, `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`, and `cargo clippy -p begin --all-targets -- -D warnings` must all pass before opening a PR.
- Never commit directly to `main`.
- `graph.js` has no test runner in this repo — verification for JS-only tasks is manual, via `dx serve --platform desktop` run from `begin/`.

---

## Task 1: Fixed initial sizing via one-shot ResizeObserver

**Files:**
- Modify: `begin/assets/graph.js:33-34` (add `resizeObserver` var), `begin/assets/graph.js:79-135` (split `init()` into `init()` + `buildGraph()`)

**Interfaces:**
- Consumes: nothing new — same `init(containerId, data)` entry point already called from `begin/src/graph_view.rs`.
- Produces: `buildGraph(container, data)` — internal helper, takes the already-measured `container` DOM element and the initial `data` snapshot, builds the `<svg>` and simulation. Later tasks (2, 3) add code inside this function. Module-level `width`/`height` vars are set by `init()` before `buildGraph()` runs, same as today.

- [ ] **Step 1: Add the `resizeObserver` module variable**

In `begin/assets/graph.js`, in the variable block that starts at line 33 (`var width = 800;` / `var height = 600;`), add one line right after:

```javascript
    var width = 800;
    var height = 600;
    var resizeObserver = null;
```

- [ ] **Step 2: Split `init()` into a measuring `init()` and a building `buildGraph()`**

Replace the entire `init(containerId, data)` function (currently `begin/assets/graph.js:79-135`, from `function init(containerId, data) {` through its closing `}` right before `function update(data) {`) with:

```javascript
    function init(containerId, data) {
        // Tear down any previous init (component remount / hot-reload).
        if (resizeObserver) { resizeObserver.disconnect(); resizeObserver = null; }
        if (simulation) { simulation.stop(); simulation = null; }
        if (svg) { svg.remove(); svg = null; }
        nodes = [];
        links = [];

        var container = document.getElementById(containerId);

        // Measure once, after layout has settled, then never resize the graph
        // again — a plain clientWidth/clientHeight read here can race layout
        // and return a stale (often zero) size, which is what made the graph
        // appear cut off on first load.
        resizeObserver = new ResizeObserver(function () {
            resizeObserver.disconnect();
            resizeObserver = null;
            width = container.clientWidth || width;
            height = container.clientHeight || height;
            buildGraph(container, data);
        });
        resizeObserver.observe(container);
    }

    function buildGraph(container, data) {
        svg = d3.select(container)
            .append('svg')
            .attr('width', width)
            .attr('height', height)
            .attr('viewBox', [0, 0, width, height]);

        var defs = svg.append('defs');

        // Arrowhead: refX=10 places the tip (at local x=10) at the line endpoint.
        // Lines are drawn edge-to-edge so the tip lands exactly at the node boundary.
        defs.append('marker')
            .attr('id', 'arrowhead')
            .attr('viewBox', '0 -5 10 10')
            .attr('refX', 10)
            .attr('refY', 0)
            .attr('markerWidth', 8)
            .attr('markerHeight', 8)
            .attr('markerUnits', 'userSpaceOnUse')
            .attr('orient', 'auto')
            .append('path').attr('d', 'M0,-5L10,0L0,5').attr('fill', '#999');

        // Layer z-order: bg → control links → constraint links → cells → rels → conditionals → labels → values
        svg.append('g').attr('class', 'bg-layer');
        controlLinkLayer = svg.append('g').attr('class', 'control-link-layer'); // NEW
        linkLayer = svg.append('g').attr('class', 'link-layer');
        cellLayer = svg.append('g').attr('class', 'cell-layer');
        relLayer = svg.append('g').attr('class', 'rel-layer');
        condLayer = svg.append('g').attr('class', 'cond-layer');               // NEW
        labelLayer = svg.append('g').attr('class', 'label-layer');
        valueLayer = svg.append('g').attr('class', 'value-layer');

        simulation = d3.forceSimulation()
            .force('link', d3.forceLink().id(function (d) { return d.id; }).distance(LINK_DISTANCE))
            .force('charge', d3.forceManyBody().strength(CHARGE_STRENGTH))
            .force('center', d3.forceCenter(width / 2, height / 2))
            // CHANGED: collision radius handles Conditional nodes.
            .force('collide', d3.forceCollide().radius(function (d) {
                if (d.kind === 'Cell') return CELL_COLLIDE_R;
                if (d.kind === 'Conditional') return COND_COLLIDE_R;
                return REL_COLLIDE_R;
            }));

        simulation.on('tick', ticked);

        update(data);
    }
```

Note: this step only changes the SVG creation (`width`/`height` are now the measured pixel values, not `'100%'`) and moves the body after the `container`/measurement lines into `buildGraph`. The layer/simulation/`update(data)` code is otherwise unchanged from what's already in the file.

- [ ] **Step 3: Manual verification via `dx serve`**

Run (from `begin/`): `dx serve --platform desktop`

1. Wait for the app to load with `demo.pm`'s graph (8 cells, 2 relationships, 2 conditionals).
2. Confirm the entire graph renders inside the graph pane with no part of it cut off at the right or bottom edge.
3. Resize the OS window (drag an edge to make it noticeably wider or narrower).
4. Confirm the graph does **not** rescale — node/text sizes stay constant. The graph pane may now show extra blank canvas margin or clip part of the graph at the new edge; both are expected, since the point is that the rendered graph itself no longer stretches.

This step has no automated equivalent — it's the acceptance check for this task's deliverable, per the `verify` skill.

- [ ] **Step 4: Commit**

```bash
git add begin/assets/graph.js
git commit -m "$(cat <<'EOF'
fix(begin): size the graph view once via ResizeObserver, stop rescaling on resize

The SVG's width/height were `100%` while its viewBox stayed fixed to a
one-time clientWidth/clientHeight read, so any later container resize
rescaled the rendered graph via preserveAspectRatio. Measure the
container with a one-shot ResizeObserver (robust against layout not
being settled yet at mount) and build the SVG with fixed pixel
dimensions so later resizes no longer affect it.
EOF
)"
```

---

## Task 2: Pan/zoom with fit-scale floor and content-bounds pan clamp

**Files:**
- Modify: `begin/assets/graph.js` (variable block, `buildGraph()`'s layer section, new helper functions, `settleSimulation()`)

**Interfaces:**
- Consumes: `buildGraph(container, data)`, module vars `svg`, `nodes`, `width`, `height` (from Task 1); `settleSimulation()`, `CELL_W`, `CELL_H`, `REL_R`, `COND_COLLIDE_R` (pre-existing).
- Produces: module vars `zoom` (the `d3.zoom()` behavior) and `zoomLayer` (the `<g>` selection all layers are nested under); functions `computeBBox()` → `{minX, minY, maxX, maxY}`, `fitTransformFor(bbox)` → `{fitScale, transform}`, `updateZoomConstraints()` (no return value — mutates `zoom`'s extents and, on the first call after a fresh `init()`, sets the initial view). Task 3 calls `zoom`, `fitTransformFor`, and `computeBBox` directly.

- [ ] **Step 1: Add `zoom`, `zoomLayer`, `hasInitialFit`, and `MAX_ZOOM` module variables**

In the same variable block touched in Task 1, add:

```javascript
    var width = 800;
    var height = 600;
    var resizeObserver = null;
    var zoom = null;
    var zoomLayer = null;
    var hasInitialFit = false;
    var MAX_ZOOM = 8;
```

- [ ] **Step 2: Reset the new state in `init()`'s teardown**

In `init()` (added in Task 1), change:

```javascript
        if (resizeObserver) { resizeObserver.disconnect(); resizeObserver = null; }
        if (simulation) { simulation.stop(); simulation = null; }
        if (svg) { svg.remove(); svg = null; }
        nodes = [];
        links = [];
```

to:

```javascript
        if (resizeObserver) { resizeObserver.disconnect(); resizeObserver = null; }
        if (simulation) { simulation.stop(); simulation = null; }
        if (svg) { svg.remove(); svg = null; }
        zoom = null;
        zoomLayer = null;
        hasInitialFit = false;
        nodes = [];
        links = [];
```

- [ ] **Step 3: Wrap the layers in `g.zoom-layer` and attach `d3.zoom()`**

In `buildGraph()`, change:

```javascript
        // Layer z-order: bg → control links → constraint links → cells → rels → conditionals → labels → values
        svg.append('g').attr('class', 'bg-layer');
        controlLinkLayer = svg.append('g').attr('class', 'control-link-layer'); // NEW
        linkLayer = svg.append('g').attr('class', 'link-layer');
        cellLayer = svg.append('g').attr('class', 'cell-layer');
        relLayer = svg.append('g').attr('class', 'rel-layer');
        condLayer = svg.append('g').attr('class', 'cond-layer');               // NEW
        labelLayer = svg.append('g').attr('class', 'label-layer');
        valueLayer = svg.append('g').attr('class', 'value-layer');
```

to:

```javascript
        // Layer z-order: bg → control links → constraint links → cells → rels → conditionals → labels → values
        zoomLayer = svg.append('g').attr('class', 'zoom-layer');
        zoomLayer.append('g').attr('class', 'bg-layer');
        controlLinkLayer = zoomLayer.append('g').attr('class', 'control-link-layer'); // NEW
        linkLayer = zoomLayer.append('g').attr('class', 'link-layer');
        cellLayer = zoomLayer.append('g').attr('class', 'cell-layer');
        relLayer = zoomLayer.append('g').attr('class', 'rel-layer');
        condLayer = zoomLayer.append('g').attr('class', 'cond-layer');               // NEW
        labelLayer = zoomLayer.append('g').attr('class', 'label-layer');
        valueLayer = zoomLayer.append('g').attr('class', 'value-layer');

        // Pan/zoom: the transform is applied to zoomLayer; scale/pan bounds
        // are set by updateZoomConstraints() once node positions are known.
        zoom = d3.zoom().on('zoom', function (event) {
            zoomLayer.attr('transform', event.transform);
        });
        svg.call(zoom);
```

(`defs` stays a direct child of `svg`, unaffected by the zoom transform — no change there.)

- [ ] **Step 4: Add `computeBBox`, `fitTransformFor`, and `updateZoomConstraints`**

In `begin/assets/graph.js`, directly after the `linkEndpoints` function and before `settleSimulation`, add:

```javascript
    // Returns the axis-aligned bounding box of all node visuals, in graph
    // (pre-zoom-transform) coordinates. Falls back to the viewport when there
    // are no nodes yet.
    function computeBBox() {
        if (nodes.length === 0) {
            return { minX: 0, minY: 0, maxX: width, maxY: height };
        }
        var minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
        nodes.forEach(function (n) {
            var hw, hh;
            if (n.kind === 'Cell') { hw = CELL_W / 2; hh = CELL_H / 2; }
            else if (n.kind === 'Conditional') { hw = COND_COLLIDE_R; hh = COND_COLLIDE_R; }
            else { hw = REL_R; hh = REL_R; }
            minX = Math.min(minX, n.x - hw);
            minY = Math.min(minY, n.y - hh);
            maxX = Math.max(maxX, n.x + hw);
            maxY = Math.max(maxY, n.y + hh);
        });
        return { minX: minX, minY: minY, maxX: maxX, maxY: maxY };
    }

    // Returns the scale that fits `bbox` entirely inside the current
    // viewport, and the centered zoom transform at that scale.
    function fitTransformFor(bbox) {
        var cx = (bbox.minX + bbox.maxX) / 2;
        var cy = (bbox.minY + bbox.maxY) / 2;
        var contentW = Math.max(bbox.maxX - bbox.minX, 1);
        var contentH = Math.max(bbox.maxY - bbox.minY, 1);
        var fitScale = Math.min(width / contentW, height / contentH);
        return {
            fitScale: fitScale,
            transform: d3.zoomIdentity.translate(width / 2, height / 2).scale(fitScale).translate(-cx, -cy)
        };
    }

    // Recomputes zoom scale/pan bounds from the current node layout. On the
    // first call after init(), snaps the view to fit; afterward, preserves
    // the user's current pan/zoom, only re-clamping it if it now falls
    // outside the new bounds.
    function updateZoomConstraints() {
        var bbox = computeBBox();
        var fit = fitTransformFor(bbox);
        var maxScale = Math.max(fit.fitScale, MAX_ZOOM);
        zoom.scaleExtent([fit.fitScale, maxScale])
            .translateExtent([[bbox.minX, bbox.minY], [bbox.maxX, bbox.maxY]])
            .extent([[0, 0], [width, height]]);
        if (!hasInitialFit) {
            svg.call(zoom.transform, fit.transform);
            hasInitialFit = true;
        } else {
            svg.call(zoom.transform, d3.zoomTransform(svg.node()));
        }
    }
```

- [ ] **Step 5: Call `updateZoomConstraints()` from `settleSimulation()`**

Change:

```javascript
    // Runs the simulation synchronously until settled, then updates the display.
    function settleSimulation() {
        var n = Math.ceil(Math.log(simulation.alphaMin()) / Math.log(1 - simulation.alphaDecay()));
        simulation.stop().alpha(1).tick(n);
        ticked();
    }
```

to:

```javascript
    // Runs the simulation synchronously until settled, then updates the display.
    function settleSimulation() {
        var n = Math.ceil(Math.log(simulation.alphaMin()) / Math.log(1 - simulation.alphaDecay()));
        simulation.stop().alpha(1).tick(n);
        ticked();
        updateZoomConstraints();
    }
```

(`settleSimulation` already only runs when `update()` detects a structural change, so this satisfies "recompute bounds on structural change" without any extra call sites.)

- [ ] **Step 6: Manual verification via `dx serve`**

Run (from `begin/`): `dx serve --platform desktop`

1. Load the graph. Confirm the entire graph is visible at load, centered, filling the pane (this is the fit-scale initial view).
2. Click-drag to pan in every direction (up, down, left, right). Confirm dragging stops exactly at the content's edges — you can't pan past where the graph's outermost node would leave the view.
3. Scroll to zoom out (mouse wheel down / trackpad pinch-out). Confirm it stops at the same fit scale as the initial view (the whole graph stays visible, no further zoom-out).
4. Scroll to zoom in. Confirm it stops at 8x scale.
5. Pan/zoom away from the fit view, then edit `begin/assets/demo.pm` to add a new cell (e.g. add `cell h: f64 = 1.0;` and reference it in a relationship) and save. Confirm the hot-reloaded graph preserves your current pan/zoom position (does not snap back to fit) unless the new layout now falls outside the old bounds, in which case it clamps back in. Revert the `demo.pm` edit afterward.

This step has no automated equivalent — it's the acceptance check for this task's deliverable, per the `verify` skill.

- [ ] **Step 7: Commit**

```bash
git add begin/assets/graph.js
git commit -m "$(cat <<'EOF'
feat(begin): add pan/zoom to the graph view, clamped to content bounds

Wraps the existing D3 layers in a zoom-layer group and attaches
d3.zoom(). scaleExtent/translateExtent are recomputed from the live
node bounding box on every structural change, so zooming out never
goes smaller than fit-to-view and panning never leaves the content's
own bounds. The user's current pan/zoom is preserved across structural
changes and only re-clamped if it now falls outside the new bounds.
EOF
)"
```

---

## Task 3: Zoom control buttons

**Files:**
- Modify: `begin/assets/graph.js` (add `zoomIn`, `zoomOut`, `resetZoom`, update the `window.beginGraph` export)
- Modify: `begin/src/graph_view.rs` (add button markup)
- Modify: `begin/assets/graph.css` (button styles)

**Interfaces:**
- Consumes: `zoom`, `svg`, `fitTransformFor`, `computeBBox` (from Task 2).
- Produces: `window.beginGraph.zoomIn()`, `window.beginGraph.zoomOut()`, `window.beginGraph.resetZoom()` — no arguments, no return value, all guarded to no-op if called before `svg`/`zoom` exist (mirrors the existing `update()` guard for the same pre-init race).

- [ ] **Step 1: Add the button handler functions and update the export**

In `begin/assets/graph.js`, change the final line:

```javascript
    window.beginGraph = { init: init, update: update };
```

to:

```javascript
    // Called by the on-screen zoom controls in graph_view.rs.
    function zoomIn() {
        if (!svg || !zoom) return;
        svg.transition().duration(200).call(zoom.scaleBy, 1.3);
    }

    function zoomOut() {
        if (!svg || !zoom) return;
        svg.transition().duration(200).call(zoom.scaleBy, 1 / 1.3);
    }

    function resetZoom() {
        if (!svg || !zoom) return;
        var fit = fitTransformFor(computeBBox());
        svg.transition().duration(300).call(zoom.transform, fit.transform);
    }

    window.beginGraph = { init: init, update: update, zoomIn: zoomIn, zoomOut: zoomOut, resetZoom: resetZoom };
```

- [ ] **Step 2: Add button styles**

Append to `begin/assets/graph.css`:

```css
.graph-zoom-controls {
    position: absolute;
    top: 8px;
    right: 8px;
    display: flex;
    gap: 4px;
    z-index: 10;
}

.graph-zoom-controls button {
    width: 28px;
    height: 28px;
    border: 1px solid #999;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.85);
    font-family: monospace;
    font-size: 14px;
    cursor: pointer;
}

.graph-zoom-controls button:hover {
    background: #fff;
}
```

- [ ] **Step 3: Add the button markup to `GraphView`**

In `begin/src/graph_view.rs`, change:

```rust
    rsx! {
        div {
            id: "graph-container",
            style: "flex: 1; height: 100%; overflow: hidden;",
            onmounted: move |_evt| async move {
```

to:

```rust
    rsx! {
        div {
            id: "graph-container",
            style: "flex: 1; height: 100%; overflow: hidden; position: relative;",
            onmounted: move |_evt| async move {
```

Then, right after the `onmounted` handler's closing `}` and before the outer `div`'s closing `}` (i.e., as the next child of `#graph-container`), add:

```rust
            div {
                class: "graph-zoom-controls",
                button {
                    onclick: move |_| {
                        spawn(async move {
                            let _ = document::eval("window.beginGraph.zoomIn();").await;
                        });
                    },
                    "+"
                }
                button {
                    onclick: move |_| {
                        spawn(async move {
                            let _ = document::eval("window.beginGraph.zoomOut();").await;
                        });
                    },
                    "-"
                }
                button {
                    onclick: move |_| {
                        spawn(async move {
                            let _ = document::eval("window.beginGraph.resetZoom();").await;
                        });
                    },
                    "Fit"
                }
            }
```

The full `rsx!` block should now read:

```rust
    rsx! {
        div {
            id: "graph-container",
            style: "flex: 1; height: 100%; overflow: hidden; position: relative;",
            onmounted: move |_evt| async move {
                let json = serde_json::to_string(&data.peek().clone()).unwrap_or_default();
                // Seed __beginGraphData with the current snapshot; use_effect may
                // update it if the sheet changes before D3 finishes loading.
                // document::Script injects <script> tags asynchronously.
                let script = format!(
                    r#"if (!window.__beginGraphData) window.__beginGraphData = {json};
                       (function tryInit(n) {{
                           if (typeof d3 !== 'undefined' && typeof window.beginGraph !== 'undefined') {{
                               window.beginGraph.init('graph-container', window.__beginGraphData);
                           }} else if (n > 0) {{
                               setTimeout(function() {{ tryInit(n - 1); }}, 50);
                           }}
                       }})(60);"#
                );
                let _ = document::eval(&script).await;
            }
            div {
                class: "graph-zoom-controls",
                button {
                    onclick: move |_| {
                        spawn(async move {
                            let _ = document::eval("window.beginGraph.zoomIn();").await;
                        });
                    },
                    "+"
                }
                button {
                    onclick: move |_| {
                        spawn(async move {
                            let _ = document::eval("window.beginGraph.zoomOut();").await;
                        });
                    },
                    "-"
                }
                button {
                    onclick: move |_| {
                        spawn(async move {
                            let _ = document::eval("window.beginGraph.resetZoom();").await;
                        });
                    },
                    "Fit"
                }
            }
        }
    }
```

- [ ] **Step 4: Run the full check suite**

```bash
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

Expected: all commands exit successfully, with zero warnings from `cargo build`/`cargo test` and zero clippy findings.

- [ ] **Step 5: Manual verification via `dx serve`**

Run (from `begin/`): `dx serve --platform desktop`

1. Confirm a small `+` / `-` / `Fit` button cluster appears in the top-right corner of the graph pane.
2. Click `+` a few times; confirm the view zooms in smoothly, stopping at 8x.
3. Click `-` repeatedly; confirm the view zooms out smoothly, stopping at the fit scale (matches wheel-zoom-out's floor from Task 2).
4. Pan and zoom in with the mouse, then click `Fit`; confirm the view animates back to the centered, whole-graph-visible state.

This step has no automated equivalent — it's the acceptance check for this task's deliverable, per the `verify` skill.

- [ ] **Step 6: Commit**

```bash
git add begin/assets/graph.js begin/assets/graph.css begin/src/graph_view.rs
git commit -m "$(cat <<'EOF'
feat(begin): add zoom in/out/fit buttons to the graph view

Exposes zoomIn/zoomOut/resetZoom on window.beginGraph (mirroring the
existing wheel/drag zoom behavior and its fit-scale floor) and wires
them to a small button cluster in the graph pane's top-right corner.
EOF
)"
```
