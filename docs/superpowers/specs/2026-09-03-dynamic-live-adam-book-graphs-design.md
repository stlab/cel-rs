# Dynamic live constraint graphs in adam-lang-book

## Context

`docs/superpowers/specs/2026-09-03-live-adam-book-graphs-design.md` added live `<graph
sheet="name">` mounts to the book: an author drops the tag in a chapter's prose and the
preprocessor rewrites it into a `.adam-live-graph` div that the bootstrap mounts a `GraphView`
into. That pass shipped the graph as a read-only snapshot and explicitly listed as a non-goal:
"Editing a sheet's cells from within a book page's live graph." The graph renders once and never
changes.

The book pairs each example's `{{#include}}` with an auto-inserted `.adam-live` inspector mount
(the sidebar of editable per-cell widgets from `SheetInspector`), so a reader can already write
values into a sheet's cells. In the tutorial's §1.1, `first_sheet` gets both: an inspector from
its include and a `<graph sheet="first_sheet">` later in the prose. But the two are separate
Dioxus `VirtualDom` mounts, each with its own `build_sheet` of the same source, so they hold two
unrelated `Sheet`s. Editing the inspector's widgets re-propagates the inspector's sheet; the
graph's frozen sheet is untouched, and the graph never moves.

This spec makes the graph dynamic: a widget edit updates the graph's values and its derived
relationship state (which method is selected, which branch is active, which cells and
relationships are forced). It deliberately does not add structural editing (adding or removing
cells and relationships), which would require re-parsing the source; the sheet's shape is fixed
at parse time, and only cell values and the plan derived from them change.

`begin` (the desktop/web app these book widgets were extracted from) already does exactly this.
`App` owns a single `Signal<Sheet>` that both `SheetInspector` and `GraphView` read, so a widget
edit re-propagates that one sheet and the graph updates automatically. The book split that shared
pair into two independent mounts. This spec restores `begin`'s single-sheet model across the
book's DOM gap.

## The gap that forces the design

In `begin`, the inspector and graph are siblings under one component that owns the sheet, so they
share a `Signal<Sheet>` by construction. In the book they cannot be siblings: the inspector
widgets render where the `{{#include}}` sits, and the graph renders wherever the author placed
`<graph>`, potentially several paragraphs apart. One Dioxus subtree renders into one contiguous
DOM root, so a single component cannot own both positions.

The graph, however, is not really drawn by Dioxus. `GraphView` renders an empty container div and
then drives D3 imperatively through `window.beginGraph.init/update(id, data)` via `document::eval`
(passing the serialized `GraphData` through a `window.__beginGraphData[id]` global). Because that
drive is keyed only by a container id, any code on the page can drive any graph container,
regardless of which `VirtualDom` (if any) created it. That is the seam this design uses: the
mount that owns the sheet reaches the remote graph container by id and drives it, rather than
rendering it as a sibling.

## Decisions

- **The inspector mount owns the sheet; it drives the graph, and the graph's own sheet is
  eliminated.** Today `mount_graph` builds a second `Sheet` purely to render a frozen graph. That
  second sheet is deleted. The inspector mount for a given example becomes the single owner of
  that sheet, and pushes the graph's `GraphData` to the associated graph container(s) itself. One
  source of truth, matching `begin`.

- **The graph is display-only; edits come only through the inspector widgets.** A reader changes
  values by writing the inspector's number fields, sliders, and checkboxes, exactly as the prose
  already instructs ("try writing a value outside `[0, 100]` into `level` above"). Clicking a
  graph node does not edit anything; the graph keeps its existing drag and zoom behavior. No
  JS-to-wasm write-back seam is introduced.

- **`<graph sheet="name">` is bound to the inspector for the same resolved example on the same
  page.** A chapter renders as one HTML page, and the design of the prior pass already resolves a
  bare `name` against the current chapter (`first_sheet` in `tutorial.md` resolves to the example
  path `tutorial/first_sheet`). Both the inspector div's and the graph div's `data-example` end
  up as that same `tutorial/first_sheet`, which is the association key. There is no standalone
  graph: every `<graph>` binds to an inspector already present on the page for the same example.

- **The preprocessor fails the build when a `<graph>` has no paired include.** Matching the
  repo's "nothing here can silently drift" rule (already applied to the include/mount-div pairing
  and to the graph tag's own file-existence check), the `<graph sheet="name">` pass additionally
  verifies the same chapter `{{#include}}`s that example's `.adm2` file. If it does not, `run()`
  returns an `Err` and `mdbook build` fails, rather than shipping a graph that can never become
  dynamic because nothing owns its sheet.

- **The graph-drive eval seam becomes a single pure helper, shared by `GraphView` and the book.**
  The exact JavaScript string that seeds `window.__beginGraphData[id]` and calls
  `window.beginGraph.init` or `update` is extracted from `GraphView`'s effect into a pure,
  contract-tested function in `adam-web-ui::graph`. `GraphView` calls it (no behavior change for
  `begin`); the book's inspector effect calls it too, so the JS seam lives in exactly one place.

- **The book graph div is the D3 container directly, with no `-container` child.** `mount_graph`
  previously gave `GraphView`'s inner div a derived `{element_id}-container` id to avoid a
  duplicate-id clash with the Dioxus root it mounted into. With no `GraphView` component mounted
  into the graph div, that clash cannot arise: the `.adam-live-graph` div is itself the container
  graph.js attaches to, and the inspector mount drives it by that id.

- **`GraphView` and `to_graph_data` are otherwise unchanged.** `GraphView` stays in `adam-web-ui`
  for `begin`'s use. `to_graph_data` already introspects the live sheet for every field the graph
  needs (`selected_method`, `conditional_active_branch`, `forced_cells`, `forced_relationships`,
  `changed`), so making the graph dynamic is entirely a matter of re-running it on each propagate
  against the shared sheet; none of its logic changes.

## Components

### `adam-web-ui::graph` (extract a pure helper)

A new pure function, e.g.:

```rust
/// Builds the JavaScript that stores `json` as this container's graph data and invokes
/// `window.beginGraph.<call>(id, data)` if the driver is loaded.
///
/// - Precondition: `call` is `"init"` or `"update"`.
fn graph_drive_script(container_id: &str, json: &str, call: &str) -> String
```

produces the string `GraphView`'s effect and `onmounted` handler build inline today. `GraphView`
is refactored to call it in place of the inline `format!`, a no-behavior-change refactor verified
by `begin`'s existing rendering. Contract-derived unit tests cover the `init` and `update` calls
and confirm `container_id`/`json` are embedded.

### `adam-lang-book-live` (own the sheet, drive the graph, drop `mount_graph`)

- `mount_graph` is removed.
- `mount(element_id: &str, source: &str, name: &str, graph_ids: Vec<String>)` gains the last
  parameter: the ids of the graph containers this sheet drives (empty when the example has no
  graph on the page). `wasm-bindgen` accepts `Vec<String>`.
- The inspector's `Root` component gains a `use_effect` mirroring `begin`'s `App`: when `sheet`
  changes it recomputes `to_graph_data(&sheet.read(), &labels.read())`, serializes it, and for
  each id in `graph_ids` drives the container via `graph_drive_script`. A single
  `use_signal<bool>` guards first-vs-subsequent: the first effect run drives every id with `init`
  and flips the flag, and every later run drives every id with `update` (all ids share one
  initial `init` because they are all driven together in that first run).
  `GraphRootProps`/`GraphRoot`/the graph-only mount path are deleted.

The parse-failure path is unchanged: a source that fails to parse still renders the diagnostic
`<pre>` from the inspector mount, and simply drives no graph.

### `adam-lang-book-preprocessor` (validate the pairing)

The existing `<graph sheet="name">` pass already resolves the chapter and checks
`book-src/examples/<chapter>/<name>.adm2` exists. It gains one check: the same chapter's raw
content also contains an `{{#include ...<chapter>/<name>.adm2}}` (the mount that will own the
sheet). On failure it returns `Err` from `run()`, failing the build with a message naming the
unpaired graph, exactly as a bad file reference already does. Unit tests cover the failing
(no include) and passing (include present) cases, alongside the existing tag-rewrite tests.

### `adam-lang-book/book-src/theme/adam-live-bootstrap.js` (wire ids, drop the graph pass)

The mount loop is reordered:

1. Assign each `.adam-live-graph` div its container id and build a map from `data-example` to the
   list of graph container ids for that example.
2. Mount each `.adam-live` inspector as today, additionally passing the graph container ids for
   its `data-example` (from the map; empty when none).
3. The separate `mountGraph` pass is removed; the graph divs are now plain containers the
   inspector mount drives.

d3.js and graph.js still load only when at least one `.adam-live-graph` div is present, unchanged.

### `begin`

No functional change. `GraphView`'s effect now calls `graph_drive_script` instead of an inline
`format!`. Re-verified with the `verifying-begin-ui` skill: the graph still renders, zoom/fit/
show-inactive still work, and switching examples still resets cleanly.

## Data flow

1. **Authoring:** unchanged. A chapter `{{#include}}`s an example (auto-paired inspector) and
   places `<graph sheet="name">` in its prose.
2. **Book build:** the preprocessor rewrites the tag into a `.adam-live-graph` div and fails the
   build if the chapter has no matching include; the wasm build produces `mount` (with its new
   parameter) and no longer `mount_graph`.
3. **Page load:** the bootstrap assigns graph container ids, builds the `data-example → [id…]`
   map, and mounts each inspector with its graph ids. Each inspector builds its sheet, renders its
   widgets, and its effect fires once, calling `init` to draw each associated graph from the
   initial state.
4. **Reader interaction:** the reader edits a widget; the sheet re-propagates; the effect
   recomputes `to_graph_data` and calls `update`, pushing new values, arrow directions, active
   branches, and forced state to each associated graph. Two graphs on one page are driven
   independently by their own inspector mounts and never share state.

## Testing

- `graph_drive_script`: contract unit tests for the `init` and `update` calls and for embedding
  the container id and JSON.
- `adam-lang-book-preprocessor`: unit tests for the new paired-include validation (build fails
  when a `<graph>` has no include; passes when it does), plus the existing tag-rewrite tests.
- `to_graph_data`: unchanged existing tests.
- `graph.js`: no JS test harness exists; verified by rendering.
- `begin`: `verifying-begin-ui` confirms no regression from the `graph_drive_script` extraction.
- End-to-end: serve the built book, edit `first_sheet`'s inspector widgets, and confirm the graph
  updates its values and relationships live; confirm two simultaneously-live graphs on one page
  update independently. `cargo build`/`mdbook build` succeeding is not sufficient per the repo's
  UI verification rule.

## Removed

- `adam-lang-book-live`'s `mount_graph`, `GraphRoot`, and `GraphRootProps`.
- The bootstrap's separate `.adam-live-graph` mount pass (replaced by id-wiring into the inspector
  mount).
- The inline graph-drive `format!` strings in `GraphView` (replaced by `graph_drive_script`).

## Non-goals (this pass)

- Editing a sheet's cells by clicking graph nodes: values change only through the inspector
  widgets.
- Structural edits (adding or removing cells/relationships) from a live page: that requires a
  re-parse and is out of scope; only values and the derived plan change.
- Cross-page or cross-chapter graph/inspector pairing: a `<graph>` binds to an inspector on the
  same chapter page, consistent with the prior pass's bare-name-resolved-to-current-chapter model.

## Phases

1. Extract `graph_drive_script` in `adam-web-ui::graph`; refactor `GraphView` to use it; verify
   `begin` unaffected (`verifying-begin-ui`).
2. Add the paired-include validation to `adam-lang-book-preprocessor` with unit tests.
3. Rework `adam-lang-book-live`: drop `mount_graph`/`GraphRoot`, extend `mount` with `graph_ids`,
   add the sheet-driven graph effect.
4. Rewire `adam-live-bootstrap.js`: assign graph ids, build the map, pass ids into `mount`, drop
   the graph-mount pass.
5. End-to-end verification: build and serve the book; confirm `first_sheet`'s graph updates live
   from its widgets and that two graphs on one page stay independent.
