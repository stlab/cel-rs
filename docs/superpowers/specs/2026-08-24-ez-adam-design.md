# ez-adam: A Standalone Visual Editor for adam-rs Property Models

**Date:** 2026-08-24
**Branch:** worktree-ez-adam
**Status:** Approved (design), not yet implemented

## Summary

`ez-adam` is a new, standalone Dioxus desktop crate: a diagram-based visual
editor for building `adam-rs` property models by direct manipulation —
placing cell nodes, drawing relationships and conditional groups between
them, editing per-cell formulas, and setting per-cell output/clamp/restrict
properties. It implements the interaction model sketched in
`2026-08-24 Property Model User Interface.pdf` (repo root).

This is a deliberate reopening of in-app visual editing as its own
exploration, **not** a reversal of the `begin`/VSCode-interop pivot recorded
in `docs/VISION.md`'s `begin` section, and **not** an extension of `begin`
itself: `ez-adam` shares no code with `begin` (no dependency on
`begin`'s `graph_view.rs`/`inspector.rs`/`bridge.rs`), though it uses the
same underlying technologies (Dioxus, `adam-rs`, `cel-parser`) and
`begin`'s graph view is explicit inspiration for the interaction feel.

`ez-adam` persists to its own native JSON document format (which records
node positions and other editor-only state that `.adm2` has no room for),
and can export — one-way, never import — that document to `.adm2` source
text for consumption by `adam-lang`/`adam-rs`/`begin`/the VS Code extension
elsewhere. It performs no live evaluation of the property model; this is a
structural editor only.

---

## 1. Motivation

`docs/superpowers/specs/` and `docs/VISION.md` document a rich, growing
`adam-rs`/`adam-lang` constraint model, but authoring one today means
hand-writing `.adm2` text — including relationship groups, conditional
branches, and (once implemented) filter-based clamp/restrict constraints —
with no direct-manipulation tool for exploring the shape of a property
model interactively. The attached sketches, from a 2026-08-24 internal
presentation ("Property Model Visualization", D. Sankel / S. Parent),
propose a specific graphical interaction model: drop cells, connect them
with relationship/conditional tools, edit formulas in a side panel, and
duplicate/branch alternative solving methods. This design turns those
sketches into a concrete crate.

---

## 2. Design decisions (settled during brainstorming)

- **Standalone crate, no code sharing with `begin`.** `begin`'s existing
  D3-based graph view is inspiration for feel, not a dependency — `ez-adam`
  does not import from or modify `begin`. This is an explicit reopening of
  the in-app-editing question as its own independent effort, not a
  continuation or reversal of the `begin`/VSCode pivot in `docs/VISION.md`.
- **Diagrammatic, not live.** Widget nodes on the canvas are schematic
  representations of cells (a labeled shape), not real running Dioxus
  widgets. No live value propagation/evaluation via `adam-rs` in this
  design — purely structural editing of the model's shape.
- **Dioxus-native canvas rendering, no D3/JS bridge.** The sketches show
  manual, user-controlled node placement ("drag these formulas and puts
  height next to width"), which a force-directed D3 layout actively fights.
  The canvas is plain SVG/HTML rendered by Dioxus from Rust-owned node
  positions; dragging/selection/connection are ordinary Dioxus event
  handlers over explicit state. This also avoids `begin`'s WebView
  JS-bridge (`bridge.rs`) entirely.
- **Own persistent JSON document format; one-way `.adm2` export only.**
  The editor's document is the source of truth (it records node positions
  and other editor-only state `.adm2` cannot express); `.adm2` is a
  generated build artifact, never parsed back in. No `.adm2` import in this
  design.
- **One `adam-lang` `sheet` per document.** Matches `adam-lang`'s current
  single-sheet-per-file model; multi-sheet composition (if ever needed)
  means opening multiple documents.
- **Desktop only for v1**, via Dioxus's `desktop` feature and `rfd` for
  native file dialogs — mirrors `begin`'s "desktop is furthest along"
  posture.
- **Reduced cell-type set: `F64`, `I64`, `Bool`, `Text` only** (not
  `adam-lang`'s full `i8..u128`/`f32`/`String` registry). Narrower integer
  ranges are expected to be expressed via input filters (clamp/restrict),
  not via choice of storage width.
- **Clamp bounds are stored in the cell's own concrete type**, not a shared
  `f64`, so e.g. a `-100.3` minimum on an integer cell is a compile-time
  type error, not a runtime check — consistent with this workspace's
  general preference for compile-time type safety over validation.
- **Clamp and restrict are independent, composable properties** of a cell
  (not a mutually-exclusive choice) — a cell may have a clamp range *and* a
  restrict expression simultaneously, matching the sketch's three
  independent checkboxes.
- **A cell's *data* (name/type/output/restrict) is distinct from its
  *node(s)* (canvas placements).** Duplicating a relationship group creates
  new visual node instances referencing the *same* underlying cell — "two
  instances of the same value in the graph" — not new cells. Editing a
  cell's properties through any one of its node instances updates the
  single shared `Cell`.
- **Conditional group conditions are a single expression, not raw
  multi-cell matching**, mirroring `adam-lang`'s actual
  `conditional <expr> { <literal> => {...} }` grammar. Dragging in
  `Bool`-only cells defaults to an implicit tuple-of-cells condition,
  auto-enumerable into a full truth table (2 cells → 4 rows). Dragging in a
  cell that isn't sensibly enumerable this way (e.g. `F64`, for a
  threshold condition like `x > 100`) switches the group to a
  user-authored CEL formula, with branch rows added manually.

---

## 3. Document model

```rust
// --- Data (shared by all visual instances) ---

struct Document {
    sheet_name: String,
    format_version: u32,
    cells: SlotMap<CellId, Cell>,
    cell_nodes: SlotMap<CellNodeId, CellNode>,
    relationship_groups: SlotMap<RelationshipGroupId, RelationshipGroup>,
    conditional_groups: SlotMap<ConditionalGroupId, ConditionalGroup>,
}

struct Cell {
    name: String,
    ty: CellType,
    output: bool,
    /// Raw CEL boolean expression text; `_` refers to this cell's own value.
    /// Compiles to a `filter` clause alongside any numeric clamp.
    restrict: Option<String>,
}

struct ClampRange<T> {
    min: Option<T>,
    max: Option<T>,
}

enum CellType {
    F64 { clamp: ClampRange<f64> },
    I64 { clamp: ClampRange<i64> },
    Bool,
    Text,
}

// --- Canvas placement (view) ---

struct CellNode {
    cell: CellId,
    position: Point,
}

struct RelationshipGroup {
    /// UI bookkeeping only ("r1", "r2"); never emitted to `.adm2`, which
    /// has no relationship-naming syntax.
    display_name: String,
    position: Point,
    /// One entry per bound cell; the `CellNodeId` gives the edge's
    /// canvas endpoint, `CellNode::cell` gives the actual bound `Cell`.
    members: Vec<(CellNodeId, String)>, // (member node, RHS formula text)
}

struct ConditionalGroup {
    display_name: String,
    position: Point,
    condition: ConditionExpr,
    branches: Vec<ConditionalBranch>,
    /// Always present — `adam-rs`'s `Sheet::add_conditional` requires a
    /// default non-optionally. Empty if genuinely unused.
    default: Vec<RelationshipGroupId>,
}

enum ConditionExpr {
    /// Implicit tuple of the dragged-in cells' own values. Auto-enumerable
    /// into a full branch table only when every cell is `Bool`.
    Cells(Vec<CellId>),
    /// User-authored CEL expression referencing the dragged-in cells
    /// (e.g. `x > 100`). Branches are added manually.
    Formula { referenced_cells: Vec<CellId>, expr: String },
}

struct ConditionalBranch {
    /// One literal per `Cells`/`referenced_cells` entry, aligned by index.
    values: Vec<CellValueLiteral>,
    /// Checked columns in the enable-table row.
    enabled_groups: Vec<RelationshipGroupId>,
}

enum CellValueLiteral {
    Bool(bool),
    I64(i64),
    Text(String),
}
```

---

## 4. UI architecture and interactions

**State:** a single `Document` signal is the source of truth, plus
ephemeral UI-only state: `selection: HashSet<NodeId>`, `active_tool: Tool`
(`Select | AddRelationship | AddConditional | Duplicate`), and transient
drag/rubber-band-select state. Every mutation goes through small pure
functions (`add_relationship_member(doc, group, cell_node)`,
`duplicate_relationship_group(doc, group)`, etc.), each with its own
doc-comment contract and unit tests derived from that contract — Dioxus
event handlers stay thin passthroughs to these functions, per this
workspace's rule against untested branching logic in framework glue.

**Canvas:** plain SVG rendered by Dioxus from `Document` positions — cells
as rounded rects, relationship groups as filled circles, conditional
groups as diamonds, edges as lines between bound node positions.
Tool-dependent click/drag behavior:

- **Select** (default): click selects (drives the side panel); drag moves
  a node's position; drag-on-empty-canvas rubber-bands a multiselect.
- **Add Relationship**: click cell A, then cell B → new
  `RelationshipGroup` with both as members (empty formulas). Clicking an
  existing relationship-group node, then a cell, adds that cell as a new
  member (matches the sketch connecting `height_pixels` into an existing
  group).
- **Add Conditional**: drag from a relationship-group node to a cell →
  wraps that group in a new `ConditionalGroup` (`Cells` mode if the
  target is `Bool`; otherwise starts in `Formula` mode awaiting user
  input).
- **Duplicate**: given a multiselected relationship group and its member
  nodes, creates new `CellNode`s pointing at the *same* underlying
  `CellId`s, plus a new `RelationshipGroup` bound to those new nodes with
  cleared formulas — a second, alternative way to solve the same
  variables, matching `adam-rs`'s multi-way-constraint model and the
  sketch's `r1`/`r2` split under a conditional.

This also means "place another instance of an existing cell elsewhere on
the canvas" falls out as a general capability of the model, independent of
Duplicate — not exposed as its own toolbar action in this design, but
available later (e.g. a palette to drop an existing cell again without
redrawing edges across a cluttered canvas) without further model changes.

**Side panel**, context-sensitive on selection:

- A **cell**: name, type, output checkbox, clamp min/max (numeric types
  only), restrict expression text.
- A **relationship group**: its editable `name := expr` formula list
  (drag-reorderable), each formula's CEL syntax validated live via
  `cel-parser` with rustc-style diagnostics rendered the same way
  `begin`'s `SourcePanel` does via `annotate-snippets`.
- A **conditional group**: the enable-table (rows = branches, columns =
  relationship groups, checkboxes = `enabled_groups`) plus the condition
  editor (cell list, or formula text, depending on `ConditionExpr` mode).

---

## 5. Code generation (`Document` → `.adm2`)

A pure function `generate_adm2(doc: &Document) -> String`, one-way only:

- `Document::sheet_name` → `sheet <name> { ... }`.
- Each `Cell` → `cell <name>: <Type>;`, with type mapping
  `F64→f64`, `I64→i64`, `Bool→bool`, `Text→String`.
- `clamp`/`restrict` compile to a `filter` clause on the cell declaration,
  with the closure parameter always named `_` so restrict-expression text
  (which already uses `_` as the self-placeholder) needs no substitution,
  e.g. `filter |_: i64| clamp(_, 0, 100)`. A cell with both clamp and
  restrict needs both folded into one filter clause; the exact composition
  (chained boolean vs. a single combined expression) depends on
  `cel-std`'s actual available functions and is left to be resolved during
  implementation, not fully specified here.
- `output: true` → a separate `out <name> := <name>;` declaration, emitted
  once per unique `CellId` regardless of how many `CellNode` instances
  exist for it.
- Top-level `RelationshipGroup`s (owned by no `ConditionalGroup`) →
  `relationship { <cell> := <formula>; ... }` blocks directly in the sheet
  body.
- `ConditionalGroup` → `conditional <condition-expr> { <branch-literal> =>
  { relationship {...} relationship {...} } ... _ => { <default groups> }
  }`. The `_` branch is always emitted (`adam-rs`'s `add_conditional`
  requires `default` non-optionally), empty if unused. `condition-expr` is
  the cell name (single-cell `Cells`), a tuple literal of cell names
  (multi-cell `Cells`), or the raw `Formula` expression text.
  `branch-literal` is the single value, or a tuple literal, matching
  `condition-expr`'s arity.

---

## 6. Persistence

`Document` is `serde`-derived to JSON with a `format_version: u32` field
for future migrations, saved/loaded via `rfd` native file dialogs
(matching `begin`'s existing pattern), desktop-only. "Export to `.adm2`"
is a separate save-dialog action that runs `generate_adm2` and writes the
result as plain text — never read back in by `ez-adam` itself.

---

## 7. Testing notes

Derived from each component's contract and public interface only:

- Every document-mutation function (`add_relationship_member`,
  `duplicate_relationship_group`, tool-click dispatch, etc.) has its own
  doc-comment contract; tests cover each documented postcondition and the
  edge cases implied by its summary (e.g. duplicating a group with a
  single member, adding a relationship between two already-connected
  cells).
- `generate_adm2` is tested by asserting exact output text for
  representative documents (a plain relationship, a conditional group in
  both `Cells` and `Formula` mode, a cell with clamp only / restrict only
  / both, an output cell) *and* by round-tripping generated text through
  `cel-parser`/`adam-lang`'s own parser to confirm it is syntactically
  valid `.adm2` — parse-only, no evaluation, consistent with this design's
  "structural editing only" scope.
- Formula and restrict-expression text fields are validated as CEL syntax
  via `cel-parser`; tests assert the `Err`/diagnostic path for malformed
  expressions and the accept path for valid ones.
- Any branching/combining logic inside a Dioxus event handler (tool
  dispatch based on `active_tool` + clicked node kind, multiselect
  rubber-band hit-testing, etc.) is extracted into its own pure function
  with a contract and unit tests, per this workspace's rule that framework
  glue with its own decision to make is not exempt from testing.
- Document (de)serialization round-trips through `serde_json` for a
  representative document covering every variant of `CellType`,
  `ConditionExpr`, and a multi-branch `ConditionalGroup`.

---

## 8. Deferred / explicitly out of scope

- Live evaluation/preview of the property model via a running `adam-rs`
  graph (test values, propagation, an Inspector-like panel) — this design
  is structural editing only.
- `.adm2` import / round-trip — export is one-way; opening an existing
  `.adm2` file and inferring a layout for it is a separate future design.
- Multiple sheets/screens (tabs) within a single document — one `sheet`
  per document only.
- Parameterized/composable `sheet`s — not implemented in `adam-lang`
  itself yet (`docs/VISION.md`'s `adam-lang` section), so nothing to
  target regardless.
- Web/mobile platforms — desktop only for v1.
- `adam-lang`'s full numeric type registry (`i8`..`u128`, `f32`) — only
  `F64`/`I64`/`Bool`/`Text` are supported cell types; narrower integer
  ranges are expected to come from clamp/restrict filters instead of
  storage-width choice.
- Sharing code with `begin` (its `graph_view.rs`, `inspector.rs`,
  `bridge.rs`) — `ez-adam` is a fully standalone crate; `begin` is
  inspiration only.
