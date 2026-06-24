# `begin` Crate Design

**Date:** 2026-06-23
**Author:** Sean Parent
**Status:** Approved

## Overview

A new workspace crate `begin` — a cross-platform desktop-first app for developing and visualizing
property models. It renders the bipartite graph from `property-model::Sheet` using Dioxus 0.7 as
the UI framework and D3 7.9 for force-directed graph layout and propagation animation.

Relationships are represented as circles; cells as squares. Cell labels appear on the graph.
The Inspector sidebar always shows cell labels, current values, and write inputs. A hover panel
for relationship details and richer cell context is deferred to the DSL phase.

## Versions

- **Dioxus:** 0.7.9 (stable; 0.8.0-alpha exists but is not production-ready)
- **D3:** 7.9.0

## Technology Rationale

**UI framework — Dioxus:** The only Rust UI framework with a credible single-codebase story across
native desktop, web (WASM), and mobile (iOS/iPadOS/Android). Its `dx` CLI provides hot-reload for
RSX templates during development, philosophically aligned with the planned hot-reload story in the
property-model DSL.

**Graph layer — D3 force simulation:** The force-directed layout naturally clusters related nodes
and minimizes crossings without requiring a hand-crafted layout algorithm. This matches the
reference visualization style (see image in session notes), where related nodes sit close together
with near-fixed edge lengths. D3's transition API handles propagation pulse animation cleanly.

Cytoscape.js was considered: its built-in `bipartite` layout places nodes in two fixed rows, which
produces crossing lines for graphs with cells connected to multiple relationships. D3's force
simulation is the correct tool for this topology.

## Crate Layout

```text
begin/                           ← new workspace member
├── Cargo.toml
├── assets/
│   ├── d3.v7.min.js             ← pre-downloaded D3 bundle, checked in
│   ├── graph.js                 ← self-contained graph renderer
│   └── graph.css                ← node/edge visual styles
└── src/
    ├── main.rs                  ← Dioxus entry point (desktop + WASM)
    ├── app.rs                   ← Root component; two-panel layout
    ├── graph_view.rs            ← GraphView component; owns the D3 <div>
    ├── inspector.rs             ← Sidebar: cell list, write form
    └── bridge.rs                ← Labels, CellMeta, GraphData, serialization
```

`begin` is added to the root `Cargo.toml` `members` list and depends on `property-model` via path.
No other workspace crates are affected.

## Additions to `property-model::Sheet`

Four iteration methods are added to expose the graph topology. No new data is stored; these expose
fields that already exist.

```rust
/// Iterates all live cell IDs.
pub fn cells(&self) -> impl Iterator<Item = CellId> + '_

/// Iterates all live relationship IDs.
pub fn relationships(&self) -> impl Iterator<Item = RelationshipId> + '_

/// Returns the relationships adjacent to a cell.
/// Returns `None` if `id` is not a live cell in this sheet.
pub fn cell_adj(&self, id: CellId) -> Option<&[RelationshipId]>

/// Returns the cells adjacent to a relationship (union across all methods).
/// Returns `None` if `id` is not a live relationship in this sheet.
pub fn relationship_adj(&self, id: RelationshipId) -> Option<&[CellId]>
```

`RelationshipData.adj` already stores the union of cells across all methods; its
`#[expect(dead_code)]` annotation is removed. `CellData.adj` already stores adjacent
relationships. These four methods expose what is already there.

## Data Bridge (`begin/src/bridge.rs`)

### Label and metadata storage

Labels and type-erased display/write closures are stored alongside the `Sheet` in `begin`. The
`Sheet` remains the single structural source of truth; `begin` associates display metadata with
stable `CellId` / `RelationshipId` keys.

`begin` depends on the `indexmap` crate for insertion-ordered cell iteration in the sidebar.

```rust
pub struct CellMeta {
    pub label: String,
    /// Returns the current cell value as a display string.
    pub display: Box<dyn Fn(&Sheet) -> String>,
    /// Parses a string and writes it to the cell.
    pub write_str: Box<dyn Fn(&mut Sheet, &str) -> Result<(), Error>>,
}

pub struct Labels {
    pub cells: IndexMap<CellId, CellMeta>,       // insertion-ordered for stable sidebar list
    pub relationships: HashMap<RelationshipId, String>,
}
```

Type knowledge is captured at `add_cell` call sites and erased into closures — consistent with how
`Method` already handles type erasure.

### Graph serialization

```rust
pub struct GraphData {
    pub nodes: Vec<NodeData>,
    pub links: Vec<LinkData>,
    pub changed: Vec<String>,    // stable node IDs of cells changed in last propagate()
    pub groups: Vec<GroupData>,  // always empty; reserved for when/otherwise
}

pub struct NodeData {
    pub id: String,     // "c{ffi}" for cells, "r{ffi}" for relationships
    pub kind: NodeKind, // Cell | Relationship
    pub label: String,  // cell label; empty string for relationships
}

pub struct LinkData {
    pub source: String,
    pub target: String,
}

pub struct GroupData {
    pub id: String,
    pub member_ids: Vec<String>,
    pub condition_id: String,
}
```

Node IDs are derived from `slotmap`'s `key.data().as_ffi()` — stable for the lifetime of a
`Sheet` instance and unique within it. Prefix `"c"` / `"r"` disambiguates the two key spaces.

`to_graph_data(sheet: &Sheet, labels: &Labels) -> GraphData` is the single serialization entry
point. It iterates `sheet.cells()` and `sheet.relationships()` for nodes, then iterates
`sheet.relationships()` and calls `sheet.relationship_adj(id)` for links.

## Component Architecture

```text
App  (holds Signal<Sheet>, Signal<Labels>)
├── GraphView   (~70% width — renders <div id="graph-container">, mounts D3)
└── Inspector   (~30% width — cell list, write form)
```

**`GraphView`**: On mount (`use_effect`), calls
`eval("beginGraph.init('graph-container', data)")` with the initial serialized graph. Reacts to
the sheet signal; on change, calls `eval("beginGraph.update(data)")`. No other Rust logic.

**`Inspector`**: Lists all cells by label. Each row shows the label and a text input. Submitting
a value calls `sheet.write(id, parsed_value)` then `sheet.propagate()`, which updates the sheet
signal and triggers `GraphView` to push the new `GraphData` to D3.

## D3 `graph.js`

Exposes `window.beginGraph = { init(containerId, data), update(data) }`.

### `init(containerId, data)`

1. Creates an `<svg>` filling the container
2. Adds `<defs>` with arrowhead `<marker>` elements (reserved for method-direction arrows)
3. Creates layered `<g>` groups: background (future group boxes) → links → nodes → labels
4. Calls `update(data)`

### `update(data)`

1. D3 `join` on links: `<line>` elements keyed by `"${source}-${target}"`
2. D3 `join` on nodes keyed by `id`:
   - Cells → `<rect>` 40×28px, rx=4, + `<text>` label below
   - Relationships → `<circle>` r=16px, no label
3. Nodes in `data.changed` receive a pulse: 200ms highlight fill → 400ms fade back
4. Force simulation restarted with updated node/link arrays

### Force simulation

```javascript
d3.forceSimulation(nodes)
  .force("link",    d3.forceLink(links).id(d => d.id).distance(80))
  .force("charge",  d3.forceManyBody().strength(-300))
  .force("center",  d3.forceCenter(width / 2, height / 2))
  .force("collide", d3.forceCollide().radius(d => d.kind === "Cell" ? 30 : 22))
```

`distance(80)` keeps related nodes at near-fixed spacing. Both distance and charge strength are
named constants at the top of the file.

## Future: Conditional Relationship Groups

When `when`/`otherwise` is added to `property-model`, the `groups` field in `GraphData` will be
populated. On each simulation tick, D3 will compute the bounding box of each group's member nodes
and draw a dashed rounded `<rect>` behind them in the background layer. A group-cohesion force
will keep members from drifting apart. The condition cell will connect to the group rect border
rather than to individual relationships.

## Build Targets

```bash
dx serve --platform desktop   # dev with hot-reload (primary)
dx build --platform desktop   # release binary

dx serve --platform web       # WASM dev server
dx build --platform web       # outputs to dist/
```

`begin/Cargo.toml` features:

```toml
[features]
default = ["desktop"]
desktop = ["dioxus/desktop"]
web     = ["dioxus/web"]
```

D3 assets (`d3.v7.min.js`, `graph.js`, `graph.css`) live in `begin/assets/` and are bundled by
Dioxus's `asset!()` macro. No CDN dependency at runtime; no npm build step.

## Initial Demo

`main.rs` creates a hardcoded `Sheet` encoding `a × b = c` (the three-method example from the
`property-model` docs) with named cells, so there is a meaningful graph to look at immediately.
