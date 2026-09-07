# Live constraint graphs in adam-lang-book Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `adam-lang-book/book-src/tutorial.md`'s placeholder image with a live,
interactive D3 constraint graph, by extracting `begin`'s `GraphView`/`to_graph_data` into
`adam-web-ui`, making `graph.js` support many independent simultaneous mounts, and adding a
`<graph sheet="name">` markdown tag the mdBook preprocessor turns into a live mount point.

**Architecture:** `GraphView`/`GraphData`/`to_graph_data` move from `begin` into a new
`adam-web-ui::graph` module (mirroring how `SheetInspector` was already extracted).
`begin/assets/graph.js` changes from a single global D3 instance to a registry of independent
per-container instances, so a book page can host more than one live graph without them sharing
state. `adam-lang-book-live` gains a `mount_graph` wasm entry point; `adam-lang-book-preprocessor`
gains a second pass recognizing `<graph sheet="name">`; `adam-live-bootstrap.js` gains a second
mount pass; `xtask`/`book.toml` gain the newly-shared assets.

**Tech Stack:** Rust 2024, Dioxus 0.7.10, D3 7.9.0, `wasm-bindgen`, mdBook + `mdbook-preprocessor`
0.5, `regex`, `anyhow` (via `mdbook_preprocessor::errors::Error`).

**Spec:** `docs/superpowers/specs/2026-09-03-live-adam-book-graphs-design.md`

## Global Constraints

- `cargo fmt --all` before every commit (pre-commit hook enforced).
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`,
  `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`, and
  `cargo clippy -p begin --all-targets -- -D warnings` must all pass with zero warnings.
- `cargo build --workspace` and `cargo test --workspace` (including `cargo test --doc
  --workspace`) must produce zero compiler warnings, not just pass clippy.
- Every `pub` function needs a contract-style `///` doc comment (Summary /
  Preconditions/`# Errors`/Postconditions/Complexity, as applicable) per this repo's
  CLAUDE.md; parser-shaped functions aren't relevant here.
- Arithmetic on signed integers uses `checked_*`, not wrapping — not applicable to this
  plan's code (no signed-integer arithmetic is added).
- Fallible operations return `Result`, not panic — the preprocessor's build-failure path
  (Task 6) follows this.
- Never commit directly to `main`; this work happens on the existing
  `worktree-adam-lang-book/add-graphs` branch.
- UI changes to `begin` must be verified by actually rendering them (`verifying-begin-ui`
  skill), not just by `cargo build`/`clippy` passing.

---

## Task 1: Move graph data/serialization into `adam-web-ui`

**Files:**
- Create: `adam-web-ui/src/graph/mod.rs`
- Create: `adam-web-ui/src/graph/data.rs` (moved from `begin/src/bridge.rs`)
- Modify: `adam-web-ui/src/lib.rs`
- Modify: `adam-web-ui/Cargo.toml`
- Modify: `begin/src/main.rs` (remove `mod bridge;`)
- Modify: `begin/src/app.rs:6` (import)
- Modify: `begin/Cargo.toml` (remove now-unused `slotmap` dependency)
- Delete: `begin/src/bridge.rs`

**Interfaces:**
- Produces: `adam_web_ui::{GraphData, NodeData, NodeKind, LinkData, LinkKind, to_graph_data}`
  (re-exported at the crate root, matching how `SheetInspector`/`build_sheet` are already
  exposed).

This task is a pure move — no behavior changes. `begin/src/bridge.rs` already depends on
`adam_web_ui::Labels` (not a `begin`-local type), so nothing about its logic needs to change,
only its location and two import lines.

- [ ] **Step 1: Run the baseline test suite before moving anything**

Run: `cargo test -p begin`
Expected: PASS (establishes a baseline — `to_graph_data`'s existing unit tests currently live
in `begin/src/bridge.rs` and must still pass after the move).

- [ ] **Step 2: Add the new dependencies `adam-web-ui` needs for the moved code**

Modify `adam-web-ui/Cargo.toml`'s `[dependencies]` section, adding these three lines (matching
the exact version strings `begin/Cargo.toml` already uses for the same crates):

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
slotmap = "1.1"
```

- [ ] **Step 3: Create `adam-web-ui/src/graph/data.rs` from `begin/src/bridge.rs`**

Copy the entire contents of `begin/src/bridge.rs` (all ~845 lines, including its `#[cfg(test)]
mod tests` block — every type, function, and test is moved unchanged) into a new file at
`adam-web-ui/src/graph/data.rs`, then apply exactly these two edits to the copied content:

Replace the module doc comment:
```rust
//! Serialization bridge from [`adam_rs::Sheet`] to D3-ready JSON, for [`crate::graph_view`].
```
with:
```rust
//! Serialization bridge from [`adam_rs::Sheet`] to D3-ready JSON, for `GraphView`.
```

Replace the import line:
```rust
use adam_web_ui::Labels;
```
with:
```rust
use crate::labels::Labels;
```

Every other line (the `NodeKind`/`NodeData`/`LinkKind`/`LinkData`/`GraphData` types,
`cell_node_id`/`rel_node_id`/`cond_node_id`/`branch_node_id`/`push_branch_links`,
`to_graph_data`, and the entire test module) is copied verbatim — none of it references
anything `begin`-specific.

- [ ] **Step 4: Create `adam-web-ui/src/graph/mod.rs`**

```rust
//! The property-model constraint graph: D3-ready serialization ([`data`]) and the Dioxus
//! component that renders it.

mod data;

pub use data::{GraphData, LinkData, LinkKind, NodeData, NodeKind, to_graph_data};
```

- [ ] **Step 5: Delete `begin/src/bridge.rs`**

- [ ] **Step 6: Remove the `mod bridge;` declaration from `begin/src/main.rs`**

`begin/src/main.rs` currently reads:
```rust
mod app;
mod bridge;
mod example_source;
mod graph_view;
mod open_file;
```
Remove the `mod bridge;` line (leave `mod graph_view;` — that's removed in Task 2).

- [ ] **Step 7: Update `begin/src/app.rs`'s import**

Replace:
```rust
use crate::bridge::to_graph_data;
```
with:
```rust
use adam_web_ui::to_graph_data;
```

- [ ] **Step 8: Remove the now-unused `slotmap` dependency from `begin/Cargo.toml`**

`slotmap` was only ever used by `begin/src/bridge.rs` (confirmed: no other file under
`begin/src/` references it). Remove this line from `begin/Cargo.toml`'s `[dependencies]`:
```toml
slotmap = "1.1"
```

- [ ] **Step 9: Add the new module to `adam-web-ui/src/lib.rs`**

Current file:
```rust
pub mod build;
pub mod diagnostics;
mod inspector;
pub mod labels;
pub mod spectrum;

pub use build::{BuildOutcome, build_sheet};
pub use inspector::SheetInspector;
pub use labels::{
    CellMeta, Labels, Renderer, WriteStrFn, format_adam_error, format_rounded,
    labels_from_cell_names,
};
```
New file:
```rust
pub mod build;
pub mod diagnostics;
pub mod graph;
mod inspector;
pub mod labels;
pub mod spectrum;

pub use build::{BuildOutcome, build_sheet};
pub use graph::{GraphData, LinkData, LinkKind, NodeData, NodeKind, to_graph_data};
pub use inspector::SheetInspector;
pub use labels::{
    CellMeta, Labels, Renderer, WriteStrFn, format_adam_error, format_rounded,
    labels_from_cell_names,
};
```

- [ ] **Step 10: Run the full test suite and confirm the move introduced no regressions**

Run: `cargo test -p adam-web-ui -p begin`
Expected: PASS — every test that used to run as part of `begin/src/bridge.rs`'s `mod tests`
now runs under `adam-web-ui`, with identical assertions.

- [ ] **Step 11: Run clippy on both affected crates**

Run: `cargo clippy -p adam-web-ui -p begin --all-targets -- -D warnings`
Expected: PASS with zero warnings.

- [ ] **Step 12: Commit**

```bash
git add adam-web-ui/src/graph adam-web-ui/src/lib.rs adam-web-ui/Cargo.toml \
  begin/src/main.rs begin/src/app.rs begin/Cargo.toml
git rm begin/src/bridge.rs
git commit -m "refactor(adam-web-ui): move graph data/serialization out of begin"
```

---

## Task 2: Move `GraphView`/`GraphLegend` into `adam-web-ui`, parameterize the container id

**Files:**
- Create: `adam-web-ui/src/graph/view.rs` (moved from `begin/src/graph_view.rs`)
- Modify: `adam-web-ui/src/graph/mod.rs`
- Modify: `adam-web-ui/src/lib.rs`
- Modify: `begin/src/main.rs` (remove `mod graph_view;`)
- Modify: `begin/src/app.rs`
- Delete: `begin/src/graph_view.rs`

**Interfaces:**
- Consumes: `adam_web_ui::graph::data::GraphData` (from Task 1).
- Produces: `adam_web_ui::GraphView` — `GraphView(graph_id: ReadSignal<String>, data:
  ReadSignal<GraphData>, source_id: ReadSignal<String>) -> Element`. The new `graph_id` prop is
  the only signature change from today's `begin`-only version (which hardcoded the container id
  to the literal `"graph-container"`). `begin`'s own behavior must be unchanged after this task
  — it just supplies `"graph-container"` explicitly now instead of implicitly.

This task does **not** touch `graph.js` or its single-global-instance behavior — that's Task 3.
Here, the only change is that the container id becomes a prop instead of a hardcoded literal,
threaded through to the one JS call site (`init`) whose signature already accepted a container
id parameter.

- [ ] **Step 1: Run the baseline test suite / UI check before moving anything**

Run: `cargo test -p begin` (expect PASS) and note that `begin`'s graph currently renders and
responds to zoom/pan/drag/example-switching — this task must not change that.

- [ ] **Step 2: Create `adam-web-ui/src/graph/view.rs`**

```rust
//! [`GraphView`] — renders the D3 force graph inside a `<div>`.
//!
//! Mounts D3 once via the element's `onmounted` event; pushes JSON updates
//! via `document::eval` whenever the `data` signal changes. Each update also
//! writes to `window.__beginGraphData` so that `onmounted`'s polling loop
//! always calls `init` with the latest snapshot rather than the one captured
//! at mount time.
//!
//! `graph_id` names the `<div>` this instance mounts into, and is passed to every
//! `window.beginGraph.*` call — see `begin/assets/graph.js` — so multiple independent
//! `GraphView`s (e.g. several live examples on one book page) never share D3/container state.
//!
//! `source_id` (see `App`'s doc comment for how it's derived) is passed
//! alongside every `init`/`update` call so `graph.js` can tell "the same
//! source got a new snapshot" (e.g. a hot-reloaded edit — keep the live
//! layout) apart from "a different demo/file just became active" (wipe the
//! layout cache instead of risking a stale position/width bleeding in from
//! an unrelated node that happens to reuse the same id — cell/relationship
//! node ids are only unique within one `Sheet`, not across different ones).

use dioxus::prelude::*;

use super::data::GraphData;

/// Renders the property model bipartite graph using D3.
///
/// On mount, polls until D3 is ready, then calls `window.beginGraph.init`
/// using `window.__beginGraphData`, which always holds the latest snapshot.
/// On every change to `data`, writes the latest snapshot to
/// `window.__beginGraphData` and calls `window.beginGraph.update`. The JS
/// guard in `graph.js` makes any `update` call before `init` a no-op.
///
/// The zoom controls and the "Show inactive" toggle live in `App`'s top bar
/// (not here) — they only ever call `window.beginGraph.*`/set a signal `App`
/// owns, so they don't need to be inside this component to work.
#[component]
pub fn GraphView(
    graph_id: ReadSignal<String>,
    data: ReadSignal<GraphData>,
    source_id: ReadSignal<String>,
) -> Element {
    let container_id = graph_id.read().clone();

    use_effect(move || {
        let json = serde_json::to_string(&*data.read()).unwrap_or_default();
        let source_id_json = serde_json::to_string(&*source_id.read()).unwrap_or_default();
        spawn(async move {
            let _ = document::eval(&format!(
                "window.__beginGraphData = {}; if (typeof window.beginGraph !== 'undefined') window.beginGraph.update(window.__beginGraphData, {});",
                json, source_id_json
            ))
            .await;
        });
    });

    rsx! {
        div {
            id: "{container_id}",
            style: "flex: 1; height: 100%; overflow: hidden; position: relative;",
            onmounted: move |_evt| async move {
                let id = graph_id.peek().clone();
                let json = serde_json::to_string(&data.peek().clone()).unwrap_or_default();
                let source_id_json = serde_json::to_string(&source_id.peek().clone()).unwrap_or_default();
                // Seed __beginGraphData with the current snapshot; use_effect may
                // update it if the sheet changes before D3 finishes loading.
                // document::Script injects <script> tags asynchronously.
                let script = format!(
                    r#"if (!window.__beginGraphData) window.__beginGraphData = {json};
                       (function tryInit(n) {{
                           if (typeof d3 !== 'undefined' && typeof window.beginGraph !== 'undefined') {{
                               window.beginGraph.init('{id}', window.__beginGraphData, {source_id_json});
                           }} else if (n > 0) {{
                               setTimeout(function() {{ tryInit(n - 1); }}, 50);
                           }}
                       }})(60);"#
                );
                let _ = document::eval(&script).await;
            },
            GraphLegend {}
        }
    }
}

/// Explains the graph's shapes, line styles, and outline colors. A static
/// key, not tied to `data` - every symbol it documents is fixed by
/// `graph.css`/`graph.js`, not by which sheet happens to be loaded.
#[component]
fn GraphLegend() -> Element {
    rsx! {
        div {
            class: "graph-legend",
            // Every row's icon sits in a fixed-width `graph-legend-icon`
            // column, whatever shape is actually inside it (a 14px square,
            // a 10px diamond, a 22px-wide edge glyph) - so the text after
            // it always starts at the same x position instead of drifting
            // with each icon's own width.
            div {
                class: "graph-legend-row",
                div { class: "graph-legend-icon",
                    div { class: "graph-legend-shape cell" }
                }
                "Cell (value)"
            }
            div {
                class: "graph-legend-row",
                div { class: "graph-legend-icon",
                    div { class: "graph-legend-shape relationship" }
                }
                "Relationship (constraint)"
            }
            div {
                class: "graph-legend-row",
                div { class: "graph-legend-icon",
                    div { class: "graph-legend-shape conditional" }
                }
                "Conditional (branch)"
            }
            div {
                class: "graph-legend-row",
                // Mirrors graph.js's #arrowhead marker: a solid triangle,
                // tip in the direction of travel.
                div {
                    class: "graph-legend-icon",
                    svg {
                        view_box: "0 0 22 14",
                        width: "22",
                        height: "14",
                        line { x1: "2", y1: "7", x2: "16", y2: "7", stroke: "#444", stroke_width: "1.5" }
                        path { d: "M16,3.5 L21,7 L16,10.5 Z", fill: "#444" }
                    }
                }
                "Depends on"
            }
            div {
                class: "graph-legend-row",
                // Mirrors graph.js's #dot marker: a solid circle on a
                // dashed line, matching link-control edges.
                div {
                    class: "graph-legend-icon",
                    svg {
                        view_box: "0 0 22 14",
                        width: "22",
                        height: "14",
                        line {
                            x1: "2", y1: "7", x2: "17", y2: "7",
                            stroke: "#444", stroke_width: "1.5", stroke_dasharray: "4 3",
                        }
                        circle { cx: "19", cy: "7", r: "3", fill: "#444" }
                    }
                }
                "Activates when matched"
            }
            div {
                class: "graph-legend-row",
                div { class: "graph-legend-icon",
                    div { class: "graph-legend-shape forced" }
                }
                "Forced (not directly editable)"
            }
            div {
                class: "graph-legend-row",
                div { class: "graph-legend-icon",
                    div { class: "graph-legend-shape inactive" }
                }
                "Inactive (branch not selected)"
            }
        }
    }
}
```

- [ ] **Step 3: Delete `begin/src/graph_view.rs`**

- [ ] **Step 4: Remove the `mod graph_view;` declaration from `begin/src/main.rs`**

- [ ] **Step 5: Update `adam-web-ui/src/graph/mod.rs`**

```rust
//! The property-model constraint graph: D3-ready serialization ([`data`]) and the Dioxus
//! component that renders it.

mod data;
mod view;

pub use data::{GraphData, LinkData, LinkKind, NodeData, NodeKind, to_graph_data};
pub use view::GraphView;
```

- [ ] **Step 6: Update `adam-web-ui/src/lib.rs`'s re-export line**

Replace:
```rust
pub use graph::{GraphData, LinkData, LinkKind, NodeData, NodeKind, to_graph_data};
```
with:
```rust
pub use graph::{GraphData, GraphView, LinkData, LinkKind, NodeData, NodeKind, to_graph_data};
```

- [ ] **Step 7: Update `begin/src/app.rs`**

Replace the import:
```rust
use crate::graph_view::GraphView;
```
with:
```rust
use adam_web_ui::GraphView;
```

Insert a new `graph_id` signal right after the existing `source_name` memo (so it's declared
before its first use further down):
```rust
    let source_id = use_memo(move || active_source.read().file_name());
    let source_text = use_memo(move || active_source.read().text.clone());
    let source_name = use_memo(move || active_source.read().file_name());
    let graph_id = use_signal(|| "graph-container".to_string());
```
(only the new `graph_id` line is added; the three `use_memo` lines above it are unchanged.)

Update the `GraphView` usage near the bottom of `App`'s `rsx!`:
```rust
                    GraphView { data: graph_data, source_id }
```
becomes:
```rust
                    GraphView { graph_id, data: graph_data, source_id }
```

- [ ] **Step 8: Run the test suite**

Run: `cargo test -p adam-web-ui -p begin`
Expected: PASS.

- [ ] **Step 9: Run clippy**

Run: `cargo clippy -p adam-web-ui -p begin --all-targets -- -D warnings`
Expected: PASS with zero warnings.

- [ ] **Step 10: Verify `begin`'s UI is unaffected**

Use the `verifying-begin-ui` skill to render `begin` as a web app and confirm: the graph still
renders in the same place, zoom in/out/fit still work, "Show inactive" still works, and
switching examples in the sidebar still resets the graph's layout cleanly. This step only
confirms no regression — the multi-instance behavior itself isn't testable yet (that's Task 3).

- [ ] **Step 11: Commit**

```bash
git add adam-web-ui/src/graph begin/src/main.rs begin/src/app.rs
git rm begin/src/graph_view.rs
git commit -m "refactor(adam-web-ui): move GraphView into adam-web-ui, parameterize container id"
```

---

## Task 3: Rewrite `graph.js` as an instance registry; update `GraphView`'s effect logic

**Files:**
- Modify: `begin/assets/graph.js` (full rewrite)
- Modify: `adam-web-ui/src/graph/view.rs`
- Modify: `begin/src/app.rs`

**Interfaces:**
- Produces: `window.beginGraph = { init(id, data), update(id, data), destroy(id), zoomIn(id),
  zoomOut(id), resetZoom(id), setShowInactive(id, bool) }` — every function now takes a
  container id as its first argument; `init`/`update` no longer take a `sourceId` (that
  decision moves to the Rust caller, per below).

This is the real architectural change described in the design spec: `graph.js` moves from one
set of module-level globals (`svg`/`simulation`/`nodes`/`links`/...) to a `Map` of per-id
`GraphInstance` objects, each owning its own D3 state. The old `sourceChanged`/`relabeledIds`
guards against cross-sheet id collisions are deleted outright: a call to `init(id, data)` always
tears down and replaces any existing instance for `id`, so a brand-new instance's `nodes`/`links`
start empty and can never inherit a stale, unrelated node's position or width. The "is this the
same sheet as before, or a different one" decision moves to `GraphView` (Rust), which already
computes `source_id` — it now decides whether to call `init` (different sheet) or `update` (same
sheet, new data) instead of `graph.js` inferring it from a passed-in id string.

- [ ] **Step 1: Replace the entire contents of `begin/assets/graph.js`**

```javascript
(function () {
    // Tunable layout constants (shared across every instance).
    var LINK_DISTANCE = 80;
    var CHARGE_STRENGTH = -300;
    var CELL_W = 60;
    var CELL_H = 36;
    var CELL_RX = 4;
    var CELL_LABEL_PADDING = 16;
    var REL_R = 16;
    var COND_SIZE = 20;
    var CELL_COLLIDE_R = 38;
    var REL_COLLIDE_R = 22;
    var COND_COLLIDE_R = COND_SIZE * Math.SQRT2;
    var NODE_STROKE_WIDTH = 1.5;
    var CONTROL_DOT_RADIUS = 2.4;
    var FIT_MARGIN = 16;
    var PULSE_COLOR = '#f90';
    var PULSE_ON_MS = 200;
    var PULSE_OFF_MS = 400;
    var INACTIVE_STROKE = '#ccc';
    var MAX_ZOOM = 8;

    // One GraphInstance per mounted container id -- begin's single view, or one of a
    // book page's several simultaneously-live examples. Each owns its D3
    // simulation/SVG/layout state entirely independently; nothing here is shared
    // across instances, so switching or dragging one can never affect another.
    var instances = new Map();

    // ---- Pure helpers (no instance state) ----

    function setsEqual(a, b) {
        if (a.size !== b.size) return false;
        for (var v of a) {
            if (!b.has(v)) return false;
        }
        return true;
    }

    function cellEdgePoint(sx, sy, tx, ty, hw, hh) {
        if (hw === undefined) hw = CELL_W / 2;
        if (hh === undefined) hh = CELL_H / 2;
        var dx = tx - sx, dy = ty - sy;
        var dist = Math.sqrt(dx * dx + dy * dy);
        if (dist < 1) return { x: tx, y: ty };
        var nx = dx / dist, ny = dy / dist;
        var td = Math.abs(nx) > 1e-9 ? hw / Math.abs(nx) : Infinity;
        var ld = Math.abs(ny) > 1e-9 ? hh / Math.abs(ny) : Infinity;
        var d = Math.min(td, ld);
        return { x: tx - nx * d, y: ty - ny * d };
    }

    function cellWidth(d) {
        return d.w || CELL_W;
    }

    function circleEdgePoint(sx, sy, cx, cy, r) {
        var dx = cx - sx, dy = cy - sy;
        var dist = Math.sqrt(dx * dx + dy * dy);
        if (dist < 1) return { x: cx, y: cy };
        return { x: cx - dx / dist * r, y: cy - dy / dist * r };
    }

    function linkEndpoints(d) {
        var s = d.source, t = d.target;
        function edgePt(node, ox, oy) {
            if (node.kind === 'Cell') return cellEdgePoint(ox, oy, node.x, node.y, cellWidth(node) / 2, CELL_H / 2);
            if (node.kind === 'Branch') return { x: node.x, y: node.y };
            var r = node.kind === 'Conditional' ? COND_COLLIDE_R : REL_R;
            return circleEdgePoint(ox, oy, node.x, node.y, r);
        }
        var srcPt = edgePt(s, t.x, t.y);
        var tgtPt = edgePt(t, s.x, s.y);
        return { x1: srcPt.x, y1: srcPt.y, x2: tgtPt.x, y2: tgtPt.y };
    }

    function dragBehavior(sim) {
        return d3.drag()
            .on('start', function (event) {
                // Both d3.zoom (on the <svg>) and this drag (on the node
                // shape) listen for pointer-down; without this the same
                // gesture would also pan the canvas.
                event.sourceEvent.stopPropagation();
            })
            .on('drag', function (event, d) {
                if (!d3.select(this).classed('dragging')) {
                    sim.alphaTarget(0.3).restart();
                    d3.select(this).classed('dragging', true);
                }
                d.fx = event.x;
                d.fy = event.y;
            })
            .on('end', function (event) {
                if (!event.active) sim.alphaTarget(0);
                d3.select(this).classed('dragging', false);
            });
    }

    // ---- GraphInstance: one independent D3 force layout mounted into one container ----

    function GraphInstance(containerId) {
        this.containerId = containerId;
        this.svg = null;
        this.simulation = null;
        this.controlLinkLayer = null;
        this.linkLayer = null;
        this.cellLayer = null;
        this.relLayer = null;
        this.condLayer = null;
        this.labelLayer = null;
        this.valueLayer = null;
        this.nodes = [];
        this.links = [];
        this.width = 800;
        this.height = 600;
        this.resizeObserver = null;
        this.zoom = null;
        this.zoomLayer = null;
        this.hasInitialFit = false;
        this.latestData = null;
        this.showInactive = true;
        this.hiddenNodeIds = new Set();
    }

    GraphInstance.prototype.computeBBox = function () {
        var self = this;
        var minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
        this.nodes.forEach(function (n) {
            if (self.hiddenNodeIds.has(n.id)) return;
            var hw, hh;
            if (n.kind === 'Cell') { hw = cellWidth(n) / 2; hh = CELL_H / 2; }
            else if (n.kind === 'Conditional') { hw = COND_COLLIDE_R; hh = COND_COLLIDE_R; }
            else if (n.kind === 'Branch') { hw = 0; hh = 0; }
            else { hw = REL_R; hh = REL_R; }
            minX = Math.min(minX, n.x - hw);
            minY = Math.min(minY, n.y - hh);
            maxX = Math.max(maxX, n.x + hw);
            maxY = Math.max(maxY, n.y + hh);
        });
        if (!isFinite(minX)) {
            return { minX: 0, minY: 0, maxX: this.width, maxY: this.height };
        }
        return {
            minX: minX - FIT_MARGIN, minY: minY - FIT_MARGIN,
            maxX: maxX + FIT_MARGIN, maxY: maxY + FIT_MARGIN
        };
    };

    GraphInstance.prototype.fitTransformFor = function (bbox) {
        var cx = (bbox.minX + bbox.maxX) / 2;
        var cy = (bbox.minY + bbox.maxY) / 2;
        var contentW = Math.max(bbox.maxX - bbox.minX, 1);
        var contentH = Math.max(bbox.maxY - bbox.minY, 1);
        var fitScale = Math.min(this.width / contentW, this.height / contentH);
        return {
            fitScale: fitScale,
            transform: d3.zoomIdentity.translate(this.width / 2, this.height / 2).scale(fitScale).translate(-cx, -cy)
        };
    };

    GraphInstance.prototype.updateZoomConstraints = function (forceFit) {
        var bbox = this.computeBBox();
        var fit = this.fitTransformFor(bbox);
        var maxScale = Math.max(fit.fitScale, MAX_ZOOM);
        var extent = [[0, 0], [this.width, this.height]];
        var translateExtent = [[bbox.minX, bbox.minY], [bbox.maxX, bbox.maxY]];
        this.zoom.scaleExtent([fit.fitScale, maxScale])
            .translateExtent(translateExtent)
            .extent(extent);
        if (!this.hasInitialFit || forceFit) {
            this.svg.call(this.zoom.transform, fit.transform);
            this.hasInitialFit = true;
        } else {
            var current = d3.zoomTransform(this.svg.node());
            var clampedK = Math.max(fit.fitScale, Math.min(maxScale, current.k));
            var rescaled = current.scale(clampedK / current.k);
            var clamped = this.zoom.constrain()(rescaled, extent, translateExtent);
            this.svg.call(this.zoom.transform, clamped);
        }
    };

    GraphInstance.prototype.settleSimulation = function (forceFit) {
        var n = Math.ceil(Math.log(this.simulation.alphaMin()) / Math.log(1 - this.simulation.alphaDecay()));
        this.simulation.stop().alpha(1).tick(n);
        this.ticked();
        this.updateZoomConstraints(forceFit);
    };

    // Starts this (freshly constructed) instance observing its container's size and
    // building the graph once a real size is known. Never called twice on the same
    // instance -- the public `init(id, data)` below always constructs a new
    // `GraphInstance` rather than reusing one, so there is nothing here to tear down.
    GraphInstance.prototype.start = function (data) {
        this.latestData = data;
        var self = this;
        var container = document.getElementById(this.containerId);

        // Keep observing for the life of this instance so the view area tracks the
        // container's size continuously, not just once at mount. The first firing
        // measures after layout has settled -- a plain clientWidth/clientHeight read
        // here can race layout and return a stale (often zero) size -- and builds
        // the graph; every later firing just resizes the existing canvas.
        this.resizeObserver = new ResizeObserver(function () {
            self.width = container.clientWidth || self.width;
            self.height = container.clientHeight || self.height;
            if (!self.svg) {
                self.buildGraph(container, self.latestData);
            } else {
                self.resizeCanvas();
            }
        });
        this.resizeObserver.observe(container);
    };

    // Resizes the existing SVG to the current width/height without touching
    // node positions or restarting the simulation.
    GraphInstance.prototype.resizeCanvas = function () {
        this.svg.attr('width', this.width)
            .attr('height', this.height)
            .attr('viewBox', [0, 0, this.width, this.height]);
        this.simulation.force('center').x(this.width / 2).y(this.height / 2);
        this.updateZoomConstraints();
    };

    GraphInstance.prototype.buildGraph = function (container, data) {
        var self = this;
        this.svg = d3.select(container)
            .append('svg')
            .attr('width', this.width)
            .attr('height', this.height)
            .attr('viewBox', [0, 0, this.width, this.height]);

        var defs = this.svg.append('defs');

        // Arrowhead: refX=10 places the tip (at local x=10) at the line endpoint.
        defs.append('marker')
            .attr('id', 'arrowhead')
            .attr('viewBox', '0 -5 10 10')
            .attr('refX', 10)
            .attr('refY', 0)
            .attr('markerWidth', 8)
            .attr('markerHeight', 8)
            .attr('markerUnits', 'userSpaceOnUse')
            .attr('orient', 'auto')
            .append('path').attr('d', 'M0,-5L10,0L0,5').attr('fill', 'context-stroke');

        // Dot marker: caps control links where they meet the relationship they target.
        defs.append('marker')
            .attr('id', 'dot')
            .attr('viewBox', '0 0 10 10')
            .attr('refX', 5)
            .attr('refY', 5)
            .attr('markerWidth', 6)
            .attr('markerHeight', 6)
            .attr('markerUnits', 'userSpaceOnUse')
            .attr('orient', 'auto')
            .append('circle').attr('cx', 5).attr('cy', 5).attr('r', 4).attr('fill', 'context-stroke');

        // Layer z-order: bg → control links → constraint links → cells → rels → conditionals → labels → values
        this.zoomLayer = this.svg.append('g').attr('class', 'zoom-layer');
        this.zoomLayer.append('g').attr('class', 'bg-layer');
        this.controlLinkLayer = this.zoomLayer.append('g').attr('class', 'control-link-layer');
        this.linkLayer = this.zoomLayer.append('g').attr('class', 'link-layer');
        this.cellLayer = this.zoomLayer.append('g').attr('class', 'cell-layer');
        this.relLayer = this.zoomLayer.append('g').attr('class', 'rel-layer');
        this.condLayer = this.zoomLayer.append('g').attr('class', 'cond-layer');
        this.labelLayer = this.zoomLayer.append('g').attr('class', 'label-layer');
        this.valueLayer = this.zoomLayer.append('g').attr('class', 'value-layer');

        this.zoom = d3.zoom().on('zoom', function (event) {
            self.zoomLayer.attr('transform', event.transform);
        });
        this.svg.call(this.zoom);

        this.simulation = d3.forceSimulation()
            .force('link', d3.forceLink().id(function (d) { return d.id; }).distance(function (d) {
                var sKind = typeof d.source === 'object' ? d.source.kind : null;
                var tKind = typeof d.target === 'object' ? d.target.kind : null;
                return (sKind === 'Branch' || tKind === 'Branch') ? LINK_DISTANCE / 2 : LINK_DISTANCE;
            }))
            .force('charge', d3.forceManyBody().strength(function (d) {
                return d.kind === 'Branch' ? 0 : CHARGE_STRENGTH;
            }))
            .force('center', d3.forceCenter(this.width / 2, this.height / 2))
            .force('collide', d3.forceCollide().radius(function (d) {
                if (d.kind === 'Cell') return Math.max(CELL_COLLIDE_R, cellWidth(d) / 2 + 4);
                if (d.kind === 'Conditional') return COND_COLLIDE_R;
                if (d.kind === 'Branch') return 0;
                return REL_COLLIDE_R;
            }));

        this.simulation.on('tick', function () {
            self.ticked();
            self.updateZoomConstraints();
        });

        this.update(data);
    };

    // Releases a pinned node back into the free simulation.
    GraphInstance.prototype.unpinNode = function (event, d) {
        event.stopPropagation();
        d.fx = null;
        d.fy = null;
        this.simulation.alpha(Math.max(this.simulation.alpha(), 0.3)).restart();
    };

    // Merges `data` into this instance's live nodes/links, preserving existing node
    // positions by id -- ids are only unique within the one Sheet this instance was
    // created for (see `to_graph_data` in `adam-web-ui/src/graph/data.rs`), which is
    // safe here specifically because a *different* Sheet always gets a brand new
    // `GraphInstance` (via the public `init` below) rather than reusing this one.
    GraphInstance.prototype.update = function (data) {
        var self = this;
        this.latestData = data;
        if (!this.svg) return;

        // True only for the very first call after this instance was built (its node
        // list is still empty) -- this instance-local fact replaces the old
        // cross-source "sourceChanged" string comparison, since a fresh instance
        // never carries over another sheet's nodes to begin with.
        var isFirstPopulation = this.nodes.length === 0 && data.nodes.length > 0;

        function linkKey(a, b) { return a < b ? a + '|' + b : b + '|' + a; }
        var oldNodeIds = new Set(this.nodes.map(function (n) { return n.id; }));
        var oldLinkSet = new Set(this.links.map(function (l) {
            var src = typeof l.source === 'object' ? l.source.id : l.source;
            var tgt = typeof l.target === 'object' ? l.target.id : l.target;
            return linkKey(src, tgt);
        }));
        var structureChanged = this.nodes.length !== data.nodes.length
            || this.links.length !== data.links.length
            || data.nodes.some(function (n) { return !oldNodeIds.has(n.id); })
            || data.links.some(function (l) { return !oldLinkSet.has(linkKey(l.source, l.target)); });

        var oldNodeMap = new Map(this.nodes.map(function (n) { return [n.id, n]; }));
        this.nodes = data.nodes.map(function (n) {
            var existing = oldNodeMap.get(n.id);
            if (existing) {
                existing.kind = n.kind;
                existing.label = n.label;
                existing.value = n.value;
                return existing;
            }
            return Object.assign({}, n);
        });
        var nodeMap = new Map(this.nodes.map(function (n) { return [n.id, n]; }));
        this.links = data.links.map(function (l) { return Object.assign({}, l); });

        var changedSet = new Set(data.changed || []);

        var controlledIds = new Set();
        var activeIds = new Set();
        this.links.forEach(function (l) {
            if (l.kind !== 'Control') return;
            var tgtId = typeof l.target === 'object' ? l.target.id : l.target;
            controlledIds.add(tgtId);
            if (l.branch_active) activeIds.add(tgtId);
        });
        function isInactive(id) {
            return controlledIds.has(id) && !activeIds.has(id);
        }

        var newHiddenIds = this.showInactive ? new Set() : new Set(
            this.nodes.filter(function (n) { return isInactive(n.id); }).map(function (n) { return n.id; })
        );
        var hiddenSetChanged = !setsEqual(newHiddenIds, this.hiddenNodeIds);
        this.hiddenNodeIds = newHiddenIds;
        structureChanged = structureChanged || hiddenSetChanged;

        function touchesHidden(l) {
            var srcId = typeof l.source === 'object' ? l.source.id : l.source;
            var tgtId = typeof l.target === 'object' ? l.target.id : l.target;
            return self.hiddenNodeIds.has(srcId) || self.hiddenNodeIds.has(tgtId);
        }
        var visibleNodes = this.nodes.filter(function (n) { return !self.hiddenNodeIds.has(n.id); });
        var visibleLinks = this.links.filter(function (l) { return !touchesHidden(l); });

        var cellNodes = visibleNodes.filter(function (n) { return n.kind === 'Cell'; });
        var relNodes = visibleNodes.filter(function (n) { return n.kind === 'Relationship'; });
        var condNodes = visibleNodes.filter(function (n) { return n.kind === 'Conditional'; });
        var constraintLinks = visibleLinks.filter(function (l) { return l.kind === 'Constraint'; });
        var controlLinks = visibleLinks.filter(function (l) { return l.kind === 'Control'; });

        this.linkLayer.selectAll('line')
            .data(constraintLinks, function (d) {
                var src = typeof d.source === 'object' ? d.source.id : d.source;
                var tgt = typeof d.target === 'object' ? d.target.id : d.target;
                return src + '-' + tgt;
            })
            .join('line')
            .attr('class', 'link');

        this.controlLinkLayer.selectAll('line')
            .data(controlLinks, function (d) {
                var src = typeof d.source === 'object' ? d.source.id : d.source;
                var tgt = typeof d.target === 'object' ? d.target.id : d.target;
                return src + '-' + tgt;
            })
            .join('line')
            .attr('class', 'link-control')
            .attr('stroke-dasharray', '5 3')
            .attr('marker-end', function (d) {
                var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                var tgtNode = nodeMap.get(tgtId);
                return (tgtNode && tgtNode.kind === 'Branch') ? null : 'url(#dot)';
            })
            .style('stroke', function (d) { return d.branch_active ? null : INACTIVE_STROKE; });

        var labelSel = this.labelLayer.selectAll('text')
            .data(cellNodes, function (d) { return d.id; })
            .join('text')
            .attr('class', 'node-label')
            .text(function (d) { return d.label; });

        var valueSel = this.valueLayer.selectAll('text')
            .data(cellNodes, function (d) { return d.id; })
            .join('text')
            .attr('class', 'node-value')
            .text(function (d) { return d.value || ''; });

        labelSel.each(function (d) {
            if (oldNodeMap.has(d.id)) return;
            d.w = Math.max(CELL_W, this.getBBox().width + CELL_LABEL_PADDING);
        });
        valueSel.each(function (d) {
            if (oldNodeMap.has(d.id) && !changedSet.has(d.id)) return;
            d.w = Math.max(d.w, this.getBBox().width + CELL_LABEL_PADDING);
        });

        this.cellLayer.selectAll('rect')
            .data(cellNodes, function (d) { return d.id; })
            .join('rect')
            .attr('class', 'node-cell')
            .attr('width', cellWidth)
            .attr('height', CELL_H)
            .attr('rx', CELL_RX)
            .call(dragBehavior(this.simulation))
            .on('dblclick', function (event, d) { self.unpinNode(event, d); });

        this.relLayer.selectAll('circle')
            .data(relNodes, function (d) { return d.id; })
            .join('circle')
            .attr('class', 'node-relationship')
            .attr('r', REL_R)
            .call(dragBehavior(this.simulation))
            .on('dblclick', function (event, d) { self.unpinNode(event, d); });

        (function () {
            self.relLayer.selectAll('circle').style('stroke', function (d) {
                return isInactive(d.id) ? INACTIVE_STROKE : null;
            });
            self.linkLayer.selectAll('line')
                .style('stroke', function (d) {
                    var srcId = typeof d.source === 'object' ? d.source.id : d.source;
                    var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                    return (isInactive(srcId) || isInactive(tgtId)) ? INACTIVE_STROKE : null;
                })
                .attr('marker-end', function (d) {
                    if (!data.arrows) return null;
                    var srcId = typeof d.source === 'object' ? d.source.id : d.source;
                    var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                    if (isInactive(srcId) || isInactive(tgtId)) return null;
                    var tgtNode = nodeMap.get(tgtId);
                    return tgtNode ? 'url(#arrowhead)' : null;
                });
        }());

        (function () {
            var forcedSet = new Set(data.forced || []);
            var forcedRelSet = new Set(data.forced_relationships || []);
            self.cellLayer.selectAll('rect')
                .classed('forced', function (d) { return forcedSet.has(d.id); });
            self.relLayer.selectAll('circle')
                .classed('forced', function (d) { return forcedRelSet.has(d.id); });
            self.linkLayer.selectAll('line')
                .classed('forced-edge', function (d) {
                    var srcId = typeof d.source === 'object' ? d.source.id : d.source;
                    var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                    return forcedSet.has(srcId) || forcedSet.has(tgtId)
                        || forcedRelSet.has(srcId) || forcedRelSet.has(tgtId);
                });
        }());

        this.condLayer.selectAll('rect')
            .data(condNodes, function (d) { return d.id; })
            .join('rect')
            .attr('class', 'node-conditional')
            .attr('width', COND_SIZE * 2)
            .attr('height', COND_SIZE * 2)
            .call(dragBehavior(this.simulation))
            .on('dblclick', function (event, d) { self.unpinNode(event, d); });

        if (changedSet.size > 0) {
            this.cellLayer.selectAll('rect')
                .filter(function (d) { return changedSet.has(d.id); })
                .transition().duration(PULSE_ON_MS)
                .style('fill', PULSE_COLOR)
                .transition().duration(PULSE_OFF_MS)
                .style('fill', null);
        }

        this.simulation.nodes(visibleNodes);
        this.simulation.force('link').links(visibleLinks);

        if (isFirstPopulation) {
            // Nothing to animate from -- settle synchronously and snap the view to fit.
            this.settleSimulation(true);
        } else if (structureChanged) {
            this.ticked();
            this.simulation.alpha(1).restart();
        } else {
            this.ticked();
        }
    };

    GraphInstance.prototype.ticked = function () {
        this.linkLayer.selectAll('line').each(function (d) {
            var ep = linkEndpoints(d);
            d3.select(this)
                .attr('x1', ep.x1).attr('y1', ep.y1)
                .attr('x2', ep.x2).attr('y2', ep.y2);
        });

        this.controlLinkLayer.selectAll('line').each(function (d) {
            var ep = linkEndpoints(d);
            var t = d.target;
            var tgtR = t.kind === 'Branch'
                ? 0
                : (t.kind === 'Conditional' ? COND_COLLIDE_R : REL_R) + NODE_STROKE_WIDTH / 2 + CONTROL_DOT_RADIUS;
            var tgtPt = circleEdgePoint(d.source.x, d.source.y, t.x, t.y, tgtR);
            d3.select(this)
                .attr('x1', ep.x1).attr('y1', ep.y1)
                .attr('x2', tgtPt.x).attr('y2', tgtPt.y);
        });

        this.cellLayer.selectAll('rect')
            .attr('x', function (d) { return d.x - cellWidth(d) / 2; })
            .attr('y', function (d) { return d.y - CELL_H / 2; });

        this.relLayer.selectAll('circle')
            .attr('cx', function (d) { return d.x; })
            .attr('cy', function (d) { return d.y; });

        this.condLayer.selectAll('rect')
            .attr('transform', function (d) {
                return 'translate(' + d.x + ',' + d.y + ') rotate(45) translate(' + (-COND_SIZE) + ',' + (-COND_SIZE) + ')';
            });

        this.labelLayer.selectAll('text')
            .attr('x', function (d) { return d.x; })
            .attr('y', function (d) { return d.y - 4; });

        this.valueLayer.selectAll('text')
            .attr('x', function (d) { return d.x; })
            .attr('y', function (d) { return d.y + 10; });
    };

    GraphInstance.prototype.zoomIn = function () {
        if (!this.svg || !this.zoom) return;
        this.svg.transition().duration(200).call(this.zoom.scaleBy, 1.3);
    };

    GraphInstance.prototype.zoomOut = function () {
        if (!this.svg || !this.zoom) return;
        this.svg.transition().duration(200).call(this.zoom.scaleBy, 1 / 1.3);
    };

    GraphInstance.prototype.resetZoom = function () {
        if (!this.svg || !this.zoom) return;
        var fit = this.fitTransformFor(this.computeBBox());
        this.svg.transition().duration(300).call(this.zoom.transform, fit.transform);
    };

    GraphInstance.prototype.setShowInactive = function (value) {
        this.showInactive = value;
        if (this.svg) this.update(this.latestData);
    };

    GraphInstance.prototype.destroy = function () {
        if (this.resizeObserver) { this.resizeObserver.disconnect(); this.resizeObserver = null; }
        if (this.simulation) { this.simulation.stop(); this.simulation = null; }
        if (this.svg) { this.svg.remove(); this.svg = null; }
    };

    // ---- Public registry: window.beginGraph, keyed by container id ----

    function init(id, data) {
        var existing = instances.get(id);
        if (existing) existing.destroy();
        var inst = new GraphInstance(id);
        instances.set(id, inst);
        inst.start(data);
    }

    function update(id, data) {
        var inst = instances.get(id);
        if (inst) inst.update(data);
    }

    function destroy(id) {
        var inst = instances.get(id);
        if (inst) {
            inst.destroy();
            instances.delete(id);
        }
    }

    function zoomIn(id) { var inst = instances.get(id); if (inst) inst.zoomIn(); }
    function zoomOut(id) { var inst = instances.get(id); if (inst) inst.zoomOut(); }
    function resetZoom(id) { var inst = instances.get(id); if (inst) inst.resetZoom(); }
    function setShowInactive(id, value) { var inst = instances.get(id); if (inst) inst.setShowInactive(value); }

    window.beginGraph = {
        init: init, update: update, destroy: destroy,
        zoomIn: zoomIn, zoomOut: zoomOut, resetZoom: resetZoom,
        setShowInactive: setShowInactive,
    };
}());
```

- [ ] **Step 2: Update `GraphView`'s effect logic in `adam-web-ui/src/graph/view.rs`**

The "same sheet, new data" vs. "different sheet" decision is real branching logic embedded in
framework-coupled code, not a trivial passthrough — per this repo's CLAUDE.md, extract it into
its own pure, contract-documented, unit-tested function rather than leaving it inline in the
`use_effect` closure.

Add this function above `GraphView` (after the `use super::data::GraphData;` import line):

```rust
/// Returns `true` when `current_source` differs from `initialized_source` — i.e. [`GraphView`]
/// should call `window.beginGraph.init` (a different sheet, needing a fresh D3 instance with no
/// carried-over layout) rather than `update` (the same sheet, new data — preserve layout).
fn source_changed(current_source: &str, initialized_source: &str) -> bool {
    current_source != initialized_source
}
```

Replace the entire `GraphView` function body with:

```rust
#[component]
pub fn GraphView(
    graph_id: ReadSignal<String>,
    data: ReadSignal<GraphData>,
    source_id: ReadSignal<String>,
) -> Element {
    let container_id = graph_id.read().clone();
    let mut initialized_source = use_signal(|| source_id.peek().clone());

    use_effect(move || {
        let id = graph_id.read().clone();
        let json = serde_json::to_string(&*data.read()).unwrap_or_default();
        let current_source = source_id.read().clone();
        let is_new_source = source_changed(&current_source, &initialized_source.peek());
        if is_new_source {
            initialized_source.set(current_source);
        }
        spawn(async move {
            let call = if is_new_source { "init" } else { "update" };
            let _ = document::eval(&format!(
                "window.__beginGraphData = {json}; if (typeof window.beginGraph !== 'undefined') window.beginGraph.{call}('{id}', window.__beginGraphData);"
            ))
            .await;
        });
    });

    rsx! {
        div {
            id: "{container_id}",
            style: "flex: 1; height: 100%; overflow: hidden; position: relative;",
            onmounted: move |_evt| async move {
                let id = graph_id.peek().clone();
                let json = serde_json::to_string(&data.peek().clone()).unwrap_or_default();
                let script = format!(
                    r#"if (!window.__beginGraphData) window.__beginGraphData = {json};
                       (function tryInit(n) {{
                           if (typeof d3 !== 'undefined' && typeof window.beginGraph !== 'undefined') {{
                               window.beginGraph.init('{id}', window.__beginGraphData);
                           }} else if (n > 0) {{
                               setTimeout(function() {{ tryInit(n - 1); }}, 50);
                           }}
                       }})(60);"#
                );
                let _ = document::eval(&script).await;
            },
            GraphLegend {}
        }
    }
}
```

Add unit tests for the new pure function. This file doesn't have a `#[cfg(test)] mod tests`
block yet — add one at the end of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_changed_is_false_for_identical_sources() {
        assert!(!source_changed("tutorial/first_sheet", "tutorial/first_sheet"));
    }

    #[test]
    fn source_changed_is_true_for_different_sources() {
        assert!(source_changed(
            "tutorial/first_sheet",
            "tutorial/area_with_requirement"
        ));
    }
}
```

Update the module doc comment's `source_id` paragraph (the "wipe the layout cache" sentence is
now literally true rather than aspirational) — replace:
```rust
//! `source_id` (see `App`'s doc comment for how it's derived) is passed
//! alongside every `init`/`update` call so `graph.js` can tell "the same
//! source got a new snapshot" (e.g. a hot-reloaded edit — keep the live
//! layout) apart from "a different demo/file just became active" (wipe the
//! layout cache instead of risking a stale position/width bleeding in from
//! an unrelated node that happens to reuse the same id — cell/relationship
//! node ids are only unique within one `Sheet`, not across different ones).
```
with:
```rust
//! `source_id` (see `App`'s doc comment for how it's derived) is compared against the last
//! source this component initialized for: unchanged means "the same source got a new
//! snapshot" (e.g. a hot-reloaded edit — call `update`, keeping the live layout); changed means
//! "a different demo/file just became active" — call `init`, which replaces any existing
//! `graph.js` instance for this id with a brand new one, so a stale position/width can never
//! bleed in from an unrelated node that happens to reuse the same id (cell/relationship node
//! ids are only unique within one `Sheet`, not across different ones).
```

- [ ] **Step 3: Update `begin/src/app.rs`'s zoom/show-inactive handlers to pass the id**

Replace the "Show inactive" effect:
```rust
    let mut show_inactive = use_signal(|| true);
    use_effect(move || {
        let show = *show_inactive.read();
        spawn(async move {
            let _ = document::eval(&format!(
                "if (typeof window.beginGraph !== 'undefined') window.beginGraph.setShowInactive({});",
                show
            ))
            .await;
        });
    });
```
with:
```rust
    let mut show_inactive = use_signal(|| true);
    use_effect(move || {
        let show = *show_inactive.read();
        let id = graph_id.peek().clone();
        spawn(async move {
            let _ = document::eval(&format!(
                "if (typeof window.beginGraph !== 'undefined') window.beginGraph.setShowInactive('{id}', {show});"
            ))
            .await;
        });
    });
```

Replace the three zoom button handlers:
```rust
                    SpActionGroup {
                        compact: true,
                        SpActionButton {
                            onclick: move |_| {
                                spawn(async move {
                                    let _ = document::eval("window.beginGraph.zoomOut();").await;
                                });
                            },
                            SpIconZoomOut {}
                        }
                        SpActionButton {
                            onclick: move |_| {
                                spawn(async move {
                                    let _ = document::eval("window.beginGraph.resetZoom();").await;
                                });
                            },
                            "Fit"
                        }
                        SpActionButton {
                            onclick: move |_| {
                                spawn(async move {
                                    let _ = document::eval("window.beginGraph.zoomIn();").await;
                                });
                            },
                            SpIconZoomIn {}
                        }
                    }
```
with:
```rust
                    SpActionGroup {
                        compact: true,
                        SpActionButton {
                            onclick: move |_| {
                                let id = graph_id.peek().clone();
                                spawn(async move {
                                    let _ = document::eval(&format!("window.beginGraph.zoomOut('{id}');")).await;
                                });
                            },
                            SpIconZoomOut {}
                        }
                        SpActionButton {
                            onclick: move |_| {
                                let id = graph_id.peek().clone();
                                spawn(async move {
                                    let _ = document::eval(&format!("window.beginGraph.resetZoom('{id}');")).await;
                                });
                            },
                            "Fit"
                        }
                        SpActionButton {
                            onclick: move |_| {
                                let id = graph_id.peek().clone();
                                spawn(async move {
                                    let _ = document::eval(&format!("window.beginGraph.zoomIn('{id}');")).await;
                                });
                            },
                            SpIconZoomIn {}
                        }
                    }
```

- [ ] **Step 4: Run the test suite**

Run: `cargo test -p adam-web-ui -p begin`
Expected: PASS, including the two new `source_changed` tests. (`graph.js` itself has no
Rust-side unit tests; this step also re-confirms the Rust changes compile and existing
non-graph tests pass.)

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -p adam-web-ui -p begin --all-targets -- -D warnings`
Expected: PASS with zero warnings.

- [ ] **Step 6: Verify `begin`'s UI via `verifying-begin-ui`**

Confirm: the graph still renders, zoom in/out/fit still work, "Show inactive" still works, and
—critically— switching between two different examples in the sidebar produces a graph that
snaps directly to a settled, fitted layout (matching the pre-refactor "new source" behavior)
rather than animating from an empty/collapsed state, and no console errors appear regarding
`window.beginGraph`.

- [ ] **Step 7: Commit**

```bash
git add begin/assets/graph.js adam-web-ui/src/graph/view.rs begin/src/app.rs
git commit -m "refactor(graph.js): rewrite as a per-container instance registry"
```

---

## Task 4: Split `graph.css`'s app-shell reset; wire the shared assets into the book

**Files:**
- Modify: `begin/assets/graph.css`
- Create: `begin/assets/app-shell.css`
- Modify: `begin/src/app.rs`
- Modify: `adam-web-ui/src/graph/view.rs`
- Modify: `xtask/src/live_book_assets.rs`
- Modify: `adam-lang-book/book.toml`
- Modify: `adam-lang-book/book-src/theme/adam-live.css`

**Interfaces:**
- Consumes: `adam_web_ui::GraphView` (from Task 3).
- Produces: a `.graph-view` CSS class (replacing `GraphView`'s inline style) that `begin` and
  the book both rely on for sizing; an `.adam-live-graph` sizing rule the book's mount wrapper
  needs since — unlike `begin`'s flex-row layout — a book page has no ambient flex parent to
  stretch a `height: 100%` element into.

- [ ] **Step 1: Move the app-shell reset out of `graph.css`**

Current `begin/assets/graph.css` starts with:
```css
html, body {
    margin: 0;
    padding: 0;
    overflow: hidden;
}
```
Remove that block from `graph.css` entirely, and create `begin/assets/app-shell.css` containing
exactly:
```css
html, body {
    margin: 0;
    padding: 0;
    overflow: hidden;
}
```

- [ ] **Step 2: Add a `.graph-view` class to `graph.css`, replacing `GraphView`'s inline style**

Add this rule to `begin/assets/graph.css` (anywhere — e.g. right after the removed `html, body`
block's former location):
```css
.graph-view {
    flex: 1;
    height: 100%;
    overflow: hidden;
    position: relative;
}
```

In `adam-web-ui/src/graph/view.rs`, replace:
```rust
        div {
            id: "{container_id}",
            style: "flex: 1; height: 100%; overflow: hidden; position: relative;",
```
with:
```rust
        div {
            id: "{container_id}",
            class: "graph-view",
```
(`flex: 1` only takes effect inside a flex container — `begin`'s case — and is a harmless no-op
in the book's plain block-flow layout; `height: 100%` needs an ancestor with a definite height
to resolve against, which the book's `.adam-live-graph` wrapper provides — see Step 4.)

- [ ] **Step 3: Register the new stylesheet in `begin/src/app.rs`**

`app.rs`'s `rsx!` currently opens with:
```rust
        document::Link { rel: "icon", r#type: "image/x-icon", href: "/favicon.ico" }
        document::Link { rel: "stylesheet", href: asset!("/assets/graph.css") }
        document::Link { rel: "stylesheet", href: asset!("/assets/inspector.css") }
```
Add the new stylesheet link:
```rust
        document::Link { rel: "icon", r#type: "image/x-icon", href: "/favicon.ico" }
        document::Link { rel: "stylesheet", href: asset!("/assets/app-shell.css") }
        document::Link { rel: "stylesheet", href: asset!("/assets/graph.css") }
        document::Link { rel: "stylesheet", href: asset!("/assets/inspector.css") }
```

- [ ] **Step 4: Give the book's live-graph mount point an explicit height**

Add this rule to `adam-lang-book/book-src/theme/adam-live.css` (which already exists and is
already registered in `book.toml`'s `additional-css`):
```css
.adam-live-graph {
  display: block;
  height: 480px;
  margin: 0.5em 0 1.5em 0;
}
```

- [ ] **Step 5: Register `graph.css` as an additional book stylesheet**

In `adam-lang-book/book.toml`, change:
```toml
additional-css = ["book-src/theme/adam-live.css", "book-src/theme/inspector.css"]
```
to:
```toml
additional-css = ["book-src/theme/adam-live.css", "book-src/theme/inspector.css", "book-src/theme/graph.css"]
```

- [ ] **Step 6: Extend `xtask`'s asset copy list**

In `xtask/src/live_book_assets.rs`, change:
```rust
    let begin_assets = root.join("begin").join("assets");
    for name in ["swc.js", "inspector.css"] {
```
to:
```rust
    let begin_assets = root.join("begin").join("assets");
    for name in ["swc.js", "inspector.css", "graph.js", "graph.css", "d3.v7.min.js"] {
```

Also update the module doc comment's one-line asset summary — change:
```rust
//! Components (SWC) bundle, and the compiled `adam-lang-book-live` wasm/js bundle — into
```
to:
```rust
//! Components (SWC) bundle, the D3/graph JS+CSS assets, and the compiled
//! `adam-lang-book-live` wasm/js bundle — into
```

- [ ] **Step 7: Run the workspace build and existing xtask tests**

Run: `cargo build --workspace && cargo test -p xtask`
Expected: PASS. (No new unit test is added for the copy-list loop itself — it's exercised only
by `prepare_live_book_assets()` against real filesystem paths, the same way `swc.js`/
`inspector.css` already are; this is covered end-to-end in Task 9, matching the existing test
coverage boundary in this file.)

- [ ] **Step 8: Verify `begin`'s UI is unaffected by the CSS split**

Use `verifying-begin-ui` to confirm the graph still fills its pane exactly as before (the page
still scrolls-hidden/full-viewport via the new `app-shell.css`, and the graph itself still fills
its flex slot via `.graph-view`).

- [ ] **Step 9: Commit**

```bash
git add begin/assets/graph.css begin/assets/app-shell.css begin/src/app.rs \
  adam-web-ui/src/graph/view.rs xtask/src/live_book_assets.rs \
  adam-lang-book/book.toml adam-lang-book/book-src/theme/adam-live.css
git commit -m "refactor(begin): split graph.css's app-shell reset; wire shared assets into the book"
```

---

## Task 5: Add `mount_graph` to `adam-lang-book-live`

**Files:**
- Modify: `adam-lang-book-live/src/lib.rs`

**Interfaces:**
- Consumes: `adam_web_ui::{GraphView, Renderer, build_sheet, to_graph_data}` (from Tasks 1-4).
- Produces: `#[wasm_bindgen] pub fn mount_graph(element_id: &str, source: &str, name: &str)`.

The mdBook preprocessor (Task 6) creates one `<div>` per `<graph sheet="...">` tag with a given
`element_id`; `mount_graph`'s `GraphView` must render into a *different*, derived id — giving it
`element_id` itself would create a duplicate DOM id once Dioxus mounts `GraphRoot`'s rendered
`<div>` as a child of the pre-existing `#element_id` wrapper (`document.getElementById` would
then resolve to the outer, unstyled wrapper instead of `GraphView`'s own styled, positioned
`<div>`, and `graph.js` would attach D3 to the wrong element).

- [ ] **Step 1: Add the `GraphRootProps`/`GraphRoot`/`mount_graph` to `adam-lang-book-live/src/lib.rs`**

Add these imports alongside the existing ones at the top of the file:
```rust
use adam_web_ui::{GraphView, to_graph_data};
```
(the file already has `use adam_web_ui::spectrum::SpTheme;` and
`use adam_web_ui::{Renderer, SheetInspector, build_sheet};` — leave those as-is.)

Append the following to the end of the file (after the existing `mount` function):

```rust
#[derive(Clone, PartialEq, Props)]
struct GraphRootProps {
    source: String,
    name: String,
    graph_id: String,
}

/// Parses `props.source`, then renders either a live [`GraphView`] (on success) or the
/// formatted diagnostic (on parse failure) — mirroring [`Root`]'s same two-outcome shape.
/// `graph_id`/`source_id` are both set to `props.graph_id`: within one independent mount the
/// source never changes, so there's nothing for `GraphView`'s destroy-vs-update logic to
/// distinguish (see [`mount_graph`]'s doc comment for why `graph_id` isn't just `element_id`).
#[component]
fn GraphRoot(props: GraphRootProps) -> Element {
    let outcome = build_sheet(&props.source, &props.name, &Renderer::plain());
    let graph_id = use_memo({
        let id = props.graph_id.clone();
        move || id.clone()
    });
    let source_id = use_memo({
        let id = props.graph_id.clone();
        move || id.clone()
    });

    match outcome.sheet_labels {
        Some((sheet, labels)) => {
            let sheet = use_signal(|| sheet);
            let labels = use_signal(|| labels);
            let data = use_memo(move || to_graph_data(&sheet.read(), &labels.read()));
            let error = outcome.error.clone();
            rsx! {
                GraphView { graph_id, data, source_id }
                if let Some(err) = error {
                    pre { class: "adam-live-error", "{err}" }
                }
            }
        }
        None => {
            let error = outcome.error.unwrap_or_default();
            rsx! {
                pre { class: "adam-live-error", "{error}" }
            }
        }
    }
}

/// Mounts a live [`GraphView`] for `source` into the DOM element with id `element_id`, using
/// `name` (the example's `data-example` attribute, e.g. `"tutorial/first_sheet"`) as the
/// diagnostic file name shown in any parse/propagate error.
///
/// `GraphView`'s own rendered `<div>` (what `graph.js` actually attaches D3 to) gets a derived
/// id, `"{element_id}-container"`, distinct from `element_id` itself: `element_id` names the
/// wrapper `<div>` the mdBook preprocessor already created (see `adam-lang-book-preprocessor`)
/// and that Dioxus mounts *into* as a child, so giving `GraphView`'s own `<div>` the same id
/// would create a duplicate-id DOM, and `document.getElementById` would resolve to the outer
/// wrapper instead of the div `graph.js` needs.
///
/// - Precondition: an element with id `element_id` already exists in the document.
#[wasm_bindgen]
pub fn mount_graph(element_id: &str, source: &str, name: &str) {
    let props = GraphRootProps {
        source: source.to_string(),
        name: format!("{name}.adm2"),
        graph_id: format!("{element_id}-container"),
    };
    let vdom = VirtualDom::new_with_props(GraphRoot, props);
    let config = dioxus::web::Config::new().rootname(element_id);
    dioxus::web::launch::launch_virtual_dom(vdom, config);
}
```

- [ ] **Step 2: Build the crate for the wasm target**

Run: `cd adam-lang-book-live && cargo check --target wasm32-unknown-unknown` (add the target
first if missing: `rustup target add wasm32-unknown-unknown`)
Expected: PASS with no errors. (`adam-lang-book-live` is a `cdylib`/wasm-only crate — a plain
host-target `cargo check` won't exercise `wasm-bindgen`'s codegen correctly, so the wasm target
is required here.)

- [ ] **Step 3: Run clippy for the crate on its native target (catches ordinary Rust issues)**

Run: `cargo clippy -p adam-lang-book-live --all-targets -- -D warnings`
Expected: PASS with zero warnings.

- [ ] **Step 4: Commit**

```bash
git add adam-lang-book-live/src/lib.rs
git commit -m "feat(adam-lang-book-live): add mount_graph wasm entry point"
```

---

## Task 6: Extend the mdBook preprocessor for `<graph sheet="name">`

**Files:**
- Modify: `adam-lang-book-preprocessor/src/main.rs`

**Interfaces:**
- Produces: a second content-rewriting pass that turns `<graph sheet="name">` into
  `<div class="adam-live-graph" data-example="chapter/name"></div>`, failing the `mdbook build`
  if `chapter/name.adm2` doesn't exist.

`mdbook-preprocessor`'s `Chapter` struct (confirmed by reading its source at
`mdbook-core-0.5.4/src/book.rs`) has a `source_path: Option<PathBuf>` field — the chapter's file
path relative to `book-src/`, e.g. `Some("tutorial.md")` — whose file stem gives the chapter
directory name (`examples/tutorial/`).

- [ ] **Step 1: Write the failing unit tests**

Add this to the bottom of `adam-lang-book-preprocessor/src/main.rs`'s existing `#[cfg(test)] mod
tests` block (i.e. inside the existing `mod tests { use super::*; ... }`, after the last
existing test function):

```rust
    fn write_example(dir: &std::path::Path, chapter: &str, name: &str) {
        let chapter_dir = dir.join(chapter);
        std::fs::create_dir_all(&chapter_dir).unwrap();
        std::fs::write(chapter_dir.join(format!("{name}.adm2")), "cell x: Int = 1;").unwrap();
    }

    #[test]
    fn inject_graph_mount_points_replaces_a_known_sheet_reference() {
        let tmp = std::env::temp_dir()
            .join(format!("adam-lang-book-preprocessor-test-{}-a", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        write_example(&tmp, "tutorial", "first_sheet");

        let re = graph_tag_regex();
        let content = "prose\n\n<graph sheet=\"first_sheet\">\n\nmore prose";
        let result = inject_graph_mount_points(content, &re, "tutorial", &tmp).unwrap();

        assert!(result.contains(
            "<div class=\"adam-live-graph\" data-example=\"tutorial/first_sheet\"></div>"
        ));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn inject_graph_mount_points_accepts_a_paired_closing_tag() {
        let tmp = std::env::temp_dir()
            .join(format!("adam-lang-book-preprocessor-test-{}-b", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        write_example(&tmp, "tutorial", "first_sheet");

        let re = graph_tag_regex();
        let content = "<graph sheet=\"first_sheet\"></graph>";
        let result = inject_graph_mount_points(content, &re, "tutorial", &tmp).unwrap();

        assert!(result.contains(
            "<div class=\"adam-live-graph\" data-example=\"tutorial/first_sheet\"></div>"
        ));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn inject_graph_mount_points_errors_when_the_example_does_not_exist() {
        let tmp = std::env::temp_dir()
            .join(format!("adam-lang-book-preprocessor-test-{}-c", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let re = graph_tag_regex();
        let content = "<graph sheet=\"does_not_exist\">";
        let result = inject_graph_mount_points(content, &re, "tutorial", &tmp);

        assert!(result.is_err());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn chapter_dir_name_uses_the_source_file_stem() {
        let chapter = Chapter::new("Chapter 1", String::new(), "tutorial.md", vec![]);
        assert_eq!(chapter_dir_name(&chapter), "tutorial");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-lang-book-preprocessor`
Expected: FAIL to compile — `graph_tag_regex`, `inject_graph_mount_points`, `chapter_dir_name`,
and `Chapter` are not yet defined/imported.

- [ ] **Step 3: Add the imports**

Change:
```rust
use adam_lang_book_live_config::NO_LIVE_MOUNT;
use mdbook_preprocessor::book::{Book, BookItem};
use mdbook_preprocessor::errors::{Error, Result};
use mdbook_preprocessor::{Preprocessor, PreprocessorContext, parse_input};
use regex::Regex;
use std::io;
```
to:
```rust
use adam_lang_book_live_config::NO_LIVE_MOUNT;
use mdbook_preprocessor::book::{Book, BookItem, Chapter};
use mdbook_preprocessor::errors::{Error, Result};
use mdbook_preprocessor::{Preprocessor, PreprocessorContext, parse_input};
use regex::Regex;
use std::io;
use std::path::Path;
```

- [ ] **Step 4: Implement `graph_tag_regex`, `chapter_dir_name`, and `inject_graph_mount_points`**

Add these functions after the existing `inject_mount_points` function (before the `struct
LiveExamples;` line):

```rust
/// Matches a `<graph sheet="name">` tag, self-closing or with a matching `</graph>`, capturing
/// the bare example name to resolve against the current chapter.
fn graph_tag_regex() -> Regex {
    Regex::new(r#"<graph\s+sheet="([A-Za-z0-9_]+)"\s*/?>(\s*</graph>)?"#).unwrap()
}

/// Returns the chapter directory name (matching `book-src/examples/<chapter>/`) for `chapter`,
/// derived from its source file's stem — e.g. `tutorial.md` -> `tutorial`.
///
/// - Postcondition: returns an empty string for a draft chapter (`source_path` is `None`); no
///   real chapter in this book is a draft, so this never occurs for a chapter [`run`] actually
///   scans a `<graph>` tag in.
fn chapter_dir_name(chapter: &Chapter) -> String {
    chapter
        .source_path
        .as_ref()
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Replaces every `<graph sheet="name">` tag in `content` with a live-mount `<div>`, resolving
/// `name` against `chapter_dir` (e.g. `tutorial`).
///
/// # Errors
/// Returns `Err` if any referenced `<examples_dir>/<chapter_dir>/<name>.adm2` file doesn't
/// exist, naming the missing path — matching how a broken `{{#include}}` already fails the
/// `mdbook build` via mdBook's own "links" preprocessor.
fn inject_graph_mount_points(
    content: &str,
    re: &Regex,
    chapter_dir: &str,
    examples_dir: &Path,
) -> Result<String> {
    let mut error = None;
    let replaced = re.replace_all(content, |caps: &regex::Captures| {
        let name = &caps[1];
        let adm2_path = examples_dir.join(chapter_dir).join(format!("{name}.adm2"));
        if !adm2_path.is_file() {
            error = Some(format!(
                "<graph sheet=\"{name}\"> in chapter \"{chapter_dir}\" references {}, which does not exist",
                adm2_path.display()
            ));
            return String::new();
        }
        format!("<div class=\"adam-live-graph\" data-example=\"{chapter_dir}/{name}\"></div>")
    });
    match error {
        Some(msg) => Err(Error::msg(msg)),
        None => Ok(replaced.into_owned()),
    }
}
```

- [ ] **Step 5: Run the tests again to verify they pass**

Run: `cargo test -p adam-lang-book-preprocessor`
Expected: PASS for the four new tests (the pre-existing `inject_mount_points_*` tests continue
to pass unchanged).

- [ ] **Step 6: Wire the new pass into `Preprocessor::run`**

Replace:
```rust
impl Preprocessor for LiveExamples {
    fn name(&self) -> &str {
        "live-examples"
    }

    fn run(&self, _ctx: &PreprocessorContext, mut book: Book) -> Result<Book> {
        let re = adm2_include_regex();
        book.for_each_mut(|item| {
            if let BookItem::Chapter(chapter) = item {
                chapter.content = inject_mount_points(&chapter.content, &re);
            }
        });
        Ok(book)
    }

    fn supports_renderer(&self, renderer: &str) -> Result<bool> {
        Ok(renderer == "html")
    }
}
```
with:
```rust
impl Preprocessor for LiveExamples {
    fn name(&self) -> &str {
        "live-examples"
    }

    fn run(&self, ctx: &PreprocessorContext, mut book: Book) -> Result<Book> {
        let include_re = adm2_include_regex();
        let graph_re = graph_tag_regex();
        let examples_dir = ctx.root.join(&ctx.config.book.src).join("examples");
        let mut error = None;
        book.for_each_mut(|item| {
            if error.is_some() {
                return;
            }
            if let BookItem::Chapter(chapter) = item {
                chapter.content = inject_mount_points(&chapter.content, &include_re);
                let chapter_dir = chapter_dir_name(chapter);
                match inject_graph_mount_points(&chapter.content, &graph_re, &chapter_dir, &examples_dir) {
                    Ok(content) => chapter.content = content,
                    Err(e) => error = Some(e),
                }
            }
        });
        if let Some(e) = error {
            return Err(e);
        }
        Ok(book)
    }

    fn supports_renderer(&self, renderer: &str) -> Result<bool> {
        Ok(renderer == "html")
    }
}
```

- [ ] **Step 7: Run the full test suite and clippy**

Run: `cargo test -p adam-lang-book-preprocessor && cargo clippy -p adam-lang-book-preprocessor --all-targets -- -D warnings`
Expected: PASS with zero warnings.

- [ ] **Step 8: Commit**

```bash
git add adam-lang-book-preprocessor/src/main.rs
git commit -m "feat(adam-lang-book-preprocessor): recognize <graph sheet=\"name\"> tags"
```

---

## Task 7: Extend `adam-live-bootstrap.js` for the `.adam-live-graph` mount kind

**Files:**
- Modify: `adam-lang-book/book-src/theme/adam-live-bootstrap.js`

**Interfaces:**
- Consumes: `mount_graph(id, source, name)` from the compiled wasm bundle (Task 5); the
  `.adam-live-graph` divs the preprocessor inserts (Task 6); `d3.v7.min.js` copied into
  `book-src/theme/` (Task 4).

- [ ] **Step 1: Replace the entire contents of `adam-lang-book/book-src/theme/adam-live-bootstrap.js`**

```javascript
// Mounts a live SheetInspector into every `.adam-live` div, and a live GraphView into every
// `.adam-live-graph` div, that the live-examples preprocessor inserted. Each div's
// `data-example` (e.g. "cells/tuple_typed_cell") names one of the `adam-live-examples.json`
// manifest's entries; the manifest and the compiled adam-lang-book-live wasm/js bundle are both
// generated into `book-src/theme/` by the book build (see the CI workflow changes), and
// mdBook's built-in theme-directory mechanism copies everything under `book-src/theme/` into
// `book-dist/theme/` verbatim (at the site root), regardless of whether it's also named in
// `book.toml`'s `additional-js`/`additional-css` (which this script itself is, so it is served
// from a different path — alongside a copy of `book-src/` preserved verbatim — than its sibling
// wasm/js/manifest files land at).
//
// A plain relative specifier can't paper over that split: `fetch()` resolves a relative URL
// against the *document's* URL, but a dynamic `import()` in a classic (non-module) script
// resolves its specifier against the *script's own* URL instead — two different base URLs in
// the same function, confirmed by a real-browser check (see this task's report). Building one
// absolute URL from `document.baseURI` and handing that same string to both calls sidesteps
// the ambiguity entirely: `import()` accepts a fully-qualified absolute URL unconditionally,
// with no dependency on which "referencing" URL it would otherwise use.
const themeBase = new URL("theme/", document.baseURI);
const moduleUrl = new URL("adam_lang_book_live.js", themeBase).href;
const manifestUrl = new URL("adam-live-examples.json", themeBase).href;
const swcUrl = new URL("swc.js", themeBase).href;
const d3Url = new URL("d3.v7.min.js", themeBase).href;

// `SheetInspector` renders `sp-*` elements (see `adam-web-ui/src/spectrum.rs`), but each
// mounted `VirtualDom` is rooted at its own `.adam-live` div — none of them ever renders a
// `<script>` tag of their own the way `begin/src/app.rs`'s top-level `App` component does for
// its single, page-wide desktop/web window. Left unloaded, every `sp-*` tag on the page stays
// an undefined custom element: no shadow DOM, so `SheetInspector`'s own `shadowRoot.querySelector`
// reads come back null and the number-field/slider write paths never fire, and no visible input
// box at all (an undefined custom element renders only its — here, absent — light-DOM children).
// Load `swc.js` once at the page level, in parallel with the wasm/manifest fetches below, so it
// defines every `sp-*` element exactly once regardless of how many examples the page mounts.
function loadSwc() {
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.type = "module";
    script.src = swcUrl;
    script.onload = () => resolve();
    script.onerror = () => reject(new Error(`adam-live: failed to load ${swcUrl}`));
    document.head.appendChild(script);
  });
}

// Loaded once at the page level for the same reason `swc.js` is: `GraphView` (mounted by
// `mount_graph`) drives D3 through `window.beginGraph` (see `begin/assets/graph.js`), which
// expects a global `d3`, regardless of how many `.adam-live-graph` divs the page mounts.
function loadD3() {
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.src = d3Url;
    script.onload = () => resolve();
    script.onerror = () => reject(new Error(`adam-live: failed to load ${d3Url}`));
    document.head.appendChild(script);
  });
}

(async () => {
  const inspectorMounts = document.querySelectorAll(".adam-live");
  const graphMounts = document.querySelectorAll(".adam-live-graph");
  if (inspectorMounts.length === 0 && graphMounts.length === 0) {
    return;
  }

  const loaders = [import(moduleUrl), fetch(manifestUrl).then((r) => r.json()), loadSwc()];
  if (graphMounts.length > 0) {
    loaders.push(loadD3());
  }
  const [{ default: init, mount, mount_graph: mountGraph }, manifest] = await Promise.all(loaders);
  await init();

  inspectorMounts.forEach((div, index) => {
    const name = div.dataset.example;
    const source = manifest[name];
    if (source === undefined) {
      console.error(`adam-live: no embedded source for "${name}"`);
      return;
    }
    const id = `adam-live-${index}`;
    div.id = id;
    mount(id, source, name);
  });

  graphMounts.forEach((div, index) => {
    const name = div.dataset.example;
    const source = manifest[name];
    if (source === undefined) {
      console.error(`adam-live: no embedded source for "${name}"`);
      return;
    }
    const id = `adam-live-graph-${index}`;
    div.id = id;
    mountGraph(id, source, name);
  });
})();
```

- [ ] **Step 2: Commit**

```bash
git add adam-lang-book/book-src/theme/adam-live-bootstrap.js
git commit -m "feat(adam-lang-book): mount live GraphViews in the bootstrap script"
```

(This file has no automated test harness — it's verified end-to-end in Task 9 by building and
serving the book in a real browser.)

---

## Task 8: Replace the tutorial's placeholder image with a live graph

**Files:**
- Modify: `adam-lang-book/book-src/tutorial.md`
- Delete: `adam-lang-book/book-src/image.png`

**Interfaces:**
- Consumes: everything from Tasks 1-7 (the `<graph sheet="...">` tag only does anything once the
  preprocessor, wasm bundle, and bootstrap script are all in place).

- [ ] **Step 1: Confirm `image.png` is unreferenced elsewhere**

Run (from the repo root): `grep -rn "image.png" adam-lang-book/` (or equivalent)
Expected: only `adam-lang-book/book-src/tutorial.md` matches — confirmed already during this
plan's research; re-confirm before deleting in case something changed.

- [ ] **Step 2: Edit `adam-lang-book/book-src/tutorial.md`**

Replace:
```markdown
A _cell_ is a named, typed storage location: the basic unit of state in a property model. `width`
and `height` are `i32`-typed cells, each given an initial value (the types are deduced from the
initial value). A `source` cell is like a spreadsheet's value cell: it holds a value written into it
and is never derived.

![alt text](image.png)
```
with:
```markdown
A _cell_ is a named, typed storage location: the basic unit of state in a property model. `width`
and `height` are `i32`-typed cells, each given an initial value (the types are deduced from the
initial value). A `source` cell is like a spreadsheet's value cell: it holds a value written into it
and is never derived.

<graph sheet="first_sheet">
```

- [ ] **Step 3: Delete `adam-lang-book/book-src/image.png`**

- [ ] **Step 4: Commit**

```bash
git add adam-lang-book/book-src/tutorial.md
git rm adam-lang-book/book-src/image.png
git commit -m "docs(adam-lang-book): replace tutorial's placeholder image with a live graph"
```

---

## Task 9: End-to-end verification

**Files:** none (verification only)

- [ ] **Step 1: Run the full check suite from `CLAUDE.md`**

```bash
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace
```
Expected: every command passes with zero warnings/errors. Re-read `cargo build`/`cargo test`
output specifically for warnings — `-D warnings` on clippy doesn't catch everything a plain
build/test compile can warn about, per this repo's CLAUDE.md.

- [ ] **Step 2: Build the live book locally**

```bash
rustup target add wasm32-unknown-unknown
cargo install --path adam-lang-book-preprocessor --force
cargo install wasm-pack --locked
(cd adam-lang-book-live && wasm-pack build --target web --release)
cargo run -p xtask -- prepare-live-book-assets
mdbook build adam-lang-book
```
Expected: every step succeeds; `mdbook build` in particular must succeed cleanly, which
confirms the preprocessor's `<graph sheet="first_sheet">` reference resolved correctly (a typo
here would fail this step with the error message from Task 6's `inject_graph_mount_points`).

- [ ] **Step 3: Serve the built book and verify the tutorial's live graph in a real browser**

Serve `adam-lang-book/book-dist/` with any static file server (e.g. `python3 -m http.server
8000 --directory adam-lang-book/book-dist`) and open the tutorial chapter. Confirm:
- The live graph renders where the placeholder image used to be, showing `first_sheet`'s cells.
- Zoom/pan/drag work the same as in `begin`.
- No console errors mentioning `beginGraph`, `d3`, or `mount_graph`.

- [ ] **Step 4: Verify two simultaneously-live graphs don't interfere**

Temporarily add a second tag, `<graph sheet="area_with_requirement">`, further down
`tutorial.md` (this example already exists at
`adam-lang-book/book-src/examples/tutorial/area_with_requirement.adm2`), rebuild the book
(`mdbook build adam-lang-book`), reload the page, and confirm: both graphs render independently,
dragging a node in one never moves anything in the other, and each has its own working
zoom/fit/legend. Once confirmed, revert this temporary addition — it isn't part of this plan's
scope (only the `first_sheet` placeholder replacement from Task 8 is) — with:
```bash
git checkout -- adam-lang-book/book-src/tutorial.md
mdbook build adam-lang-book
```

- [ ] **Step 5: Re-verify `begin` one final time via `verifying-begin-ui`**

Confirm `begin`'s own graph view still renders and behaves identically to before this entire
plan — screenshot + DOM dump, checking the graph, legend, zoom controls, and example switching.

- [ ] **Step 6: Confirm the working tree is clean and everything is committed**

Run: `git status`
Expected: nothing to commit (the temporary second `<graph>` tag from Step 4 was already
reverted). If anything unexpected is present, investigate before proceeding.

This plan's work is now complete. Opening a pull request is a separate, explicit step — use the
`pr-open` skill when ready.
