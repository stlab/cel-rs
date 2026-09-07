# Live constraint graphs in adam-lang-book

## Context

`adam-lang-book`'s tutorial chapter has a placeholder image (`![alt text](image.png)`,
`book-src/tutorial.md:29`) standing in for a rendering of the constraint graph that the
`first_sheet` example describes. Separately, `begin` (the Dioxus desktop/web app) already
renders exactly this: [`GraphView`](../../../begin/src/graph_view.rs) draws the property
model's bipartite graph (cells as rectangles, relationships as circles, conditionals as
diamonds) as a D3 force layout, fed by [`to_graph_data`](../../../begin/src/bridge.rs)'s
serialization of an `adam_rs::Sheet`.

This mirrors the situation `docs/superpowers/specs/2026-08-27-live-adam-book-examples-design.md`
solved for the book's cell inspector: that spec extracted `begin`'s `Inspector` into a
reusable `adam-web-ui` crate, built a `wasm-bindgen` mount crate (`adam-lang-book-live`), and
wired an mdBook preprocessor (`adam-lang-book-preprocessor`) to auto-insert live-mount `<div>`s
after every example's `{{#include}}`. It explicitly deferred the graph: *"Graph view stays
`begin`-only... not touched or extracted in this pass."* This spec is that deferred pass —
extracting `GraphView`/`to_graph_data` the same way, and adding a second live-mount kind for
graphs.

One difference from the inspector case: a live graph doesn't pair 1:1 with an `{{#include}}`
the way the inspector's mount div does — a chapter shows the sheet source first, then several
paragraphs of prose, then wants the graph. So instead of an auto-inserted mount point, the
author places one explicitly: `<graph sheet="first_sheet">` wherever the graph belongs in the
chapter's prose.

A second, more consequential difference: `graph.js` (`begin/assets/graph.js`) is written as a
single global D3 instance — module-level `svg`/`simulation`/`nodes`/`links` state and one
`window.beginGraph` object, torn down and rebuilt on every `init()`. `SheetInspector` was
already designed for many independent per-page mounts (Dioxus gives each its own `VirtualDom`);
`graph.js` was not, and a book page can reasonably show more than one live graph. Making the
book work means making `graph.js` instance-scoped, not just relocating it.

## Decisions

- **`GraphView`/`to_graph_data` move into `adam-web-ui`.** A new `adam-web-ui/src/graph.rs`
  gets `GraphView`, `GraphLegend`, and `GraphData`/`NodeData`/`NodeKind`/`LinkData`/`LinkKind`/
  `to_graph_data`, moved verbatim from `begin/src/graph_view.rs` and `begin/src/bridge.rs`
  (both files are deleted from `begin` once nothing begin-specific remains in them), re-exported
  from `adam-web-ui`'s `lib.rs` alongside `SheetInspector`. This is a pure move: none of this
  code is `Sheet`-mutating or desktop-specific, matching why `SheetInspector` was extractable in
  the same way.

- **`GraphView` takes a `graph_id` prop instead of a hardcoded container id.** Today it always
  mounts to `"graph-container"`; multiple simultaneous mounts on one book page require each to
  own a distinct id, supplied by the caller (`begin`'s `App` keeps passing `"graph-container"`;
  the book's wasm mount crate passes its `element_id`).

- **`graph.js` becomes an instance registry, and cross-source state merging is deleted, not
  reproduced per-instance.** `window.beginGraph` changes from a singleton to
  `{ init(id, data), update(id, data), destroy(id), zoomIn(id), zoomOut(id), resetZoom(id),
  setShowInactive(id, bool) }`, backed by a `Map<id, instance>` where each instance closes over
  its own `svg`/`simulation`/`nodes`/`links`/`zoom`/`resizeObserver`/`hasInitialFit`/
  `showInactive` — none of that state is module-level anymore.

  Today's `sourceChanged`/`relabeledIds` logic inside `update()` exists to guard against a
  reused container silently inheriting a stale node's layout position when an unrelated sheet
  with a colliding id (cell/relationship ids are only unique *within* one `Sheet`) becomes
  active. That guard is deleted outright rather than moved inside each instance's closure: the
  decision of "is this the same sheet as last time, or a different one" moves to the Rust caller
  (`GraphView`, the one place that already computes `source_id`), which calls `destroy(id)` then
  `init(id, data)` for a new sheet (fresh, empty instance state — nothing to accidentally
  inherit) and `update(id, data)` only when the sheet is unchanged (safe to merge, since ids
  within one still-live instance never collide by construction). `update()` itself no longer
  takes or reasons about a `sourceId` at all.

- **`GraphView` tracks the last-initialized `source_id` itself.** A `use_signal<Option<String>>`
  inside `GraphView` remembers which `source_id` its mounted instance currently reflects; its
  effect compares the incoming `source_id` against it to choose `destroy`+`init` vs. `update`,
  then updates the stored value. This is a real (small) behavior change to `begin`'s existing
  effect logic, not just a relocation.

- **`graph.css`'s app-shell reset is separated from its graph styling.** `html, body { margin:
  0; padding: 0; overflow: hidden; }` is `begin`'s full-viewport app-shell reset, not part of
  the graph's own appearance, and would break scrolling on a book page. It moves out of
  `graph.css` into a small `begin`-only stylesheet (or inline in `app.rs`); `graph.css` itself
  (node/link/legend rules) ships to the book unmodified via the existing asset-copy pattern.

- **Graph assets stay physically in `begin/assets/`.** `graph.js`, `graph.css`, and
  `d3.v7.min.js` are not moved into `adam-web-ui` — `begin` still references them via
  Dioxus's `asset!()` macro, which resolves relative to the crate using it, exactly the
  existing precedent for `swc.js`/`inspector.css` (owned conceptually by `adam-web-ui`'s
  `SheetInspector`, physically copied out of `begin/assets/`). `xtask::live_book_assets::
  prepare_live_book_assets` adds these three files to its existing copy list.

- **A new wasm entry point, not a new crate.** `adam-lang-book-live` gains
  `mount_graph(element_id: &str, source: &str, name: &str)` alongside the existing `mount`,
  reusing the same `build_sheet` path and the same parse-failure-renders-diagnostic fallback.
  `graph_id` is just `element_id` — one mount point, one graph, no extra id plumbing.

- **New markdown syntax: `<graph sheet="name">`, resolved against the current chapter.**
  Unlike the inspector's auto-inserted mount div, a live graph is hand-placed wherever the
  author wants it (potentially paragraphs after the example's `{{#include}}`). `name` is a bare
  example name (e.g. `first_sheet`), resolved against the chapter the tag appears in — chapter
  directories already match markdown filenames 1:1 (`tutorial.md` ↔ `examples/tutorial/`), so
  this needs no explicit `chapter/name` from the author and matches how a graph only ever makes
  sense next to an example already shown earlier in the same chapter.

- **The preprocessor validates the reference and fails the build if it's wrong.** Matching this
  repo's "nothing here can silently drift" rule (already applied to the include/mount-div
  pairing), `mdbook-live-examples` checks that
  `book-src/examples/<current-chapter>/<name>.adm2` exists on disk before rewriting the tag, and
  errors out the `mdbook build` (not a runtime console warning) if it doesn't — the same
  failure-visibility level a typo'd `{{#include}}` already gets from mdBook's own "links"
  preprocessor.

## Components

### `adam-web-ui::graph` (new module)

Moved from `begin/src/graph_view.rs` + `begin/src/bridge.rs`:

- `GraphView(graph_id: ReadSignal<String>, data: ReadSignal<GraphData>, source_id:
  ReadSignal<String>) -> Element` — the `graph_id` prop is the only signature change from
  today's `begin`-only version. Internally replaces the `use_effect`'s unconditional `update`
  call with the destroy-vs-update branch described above.
- `GraphLegend` — unchanged, still a static key with no props.
- `GraphData`/`NodeData`/`NodeKind`/`LinkData`/`LinkKind`/`to_graph_data` — unchanged logic,
  moved verbatim along with their existing unit tests.

### `begin` (refactor, no behavior change)

- `app.rs`/`graph_view.rs`'s imports switch to `adam_web_ui::graph::*`; `bridge.rs` is deleted
  (nothing begin-specific remains once `to_graph_data` moves).
- `App` passes `graph_id: "graph-container"` to `GraphView` (previously hardcoded inside
  `GraphView` itself).
- The app-shell CSS reset moves out of `graph.css` into `begin`'s own stylesheet/inline style.
- Verified via the `verifying-begin-ui` skill: screenshot + DOM dump confirming the graph still
  renders, zoom/fit/show-inactive controls still work, and switching examples still resets the
  layout cleanly (now via destroy+init rather than the old merge guard).

### `begin/assets/graph.js` (instance registry rewrite)

`window.beginGraph = { init(id, data), update(id, data), destroy(id), zoomIn(id), zoomOut(id),
resetZoom(id), setShowInactive(id, bool) }`. A module-level `Map<string, Instance>` holds one
entry per active container id; each `Instance` owns exactly the state (`svg`, `simulation`,
`nodes`, `links`, `zoom`, `zoomLayer`, `resizeObserver`, `hasInitialFit`, `showInactive`,
`hiddenNodeIds`) that today's module-level variables hold, unchanged in behavior otherwise
(force layout constants, drag/pin behavior, dimming/forcing/pulsing logic, zoom/fit math are all
copied as-is per instance). `sourceChanged`/`relabeledIds`/`currentSourceId` are deleted from
`update()`; a fresh `init(id, ...)` simply discards any prior entry for `id` and starts empty,
so there's nothing left to guard against.

### `adam-lang-book-live` (extended)

`mount_graph(element_id: &str, source: &str, name: &str)`: parses `source` via `build_sheet`
(same as `mount`); on success, mounts a `Root`-equivalent component rendering `GraphView` bound
to `to_graph_data(&sheet, &labels)` with `graph_id`/`source_id` both set to `element_id`/`name`;
on parse failure, renders the same `format_adam_error` diagnostic `<pre>` that `mount` already
falls back to. No `SpTheme` wrapper (unlike `mount`'s `Root`) — `GraphView` renders plain
SVG/D3, no Spectrum Web Components.

### `adam-lang-book-preprocessor` (extended)

A second pass, after the existing `.adm2`-include pass, matching
`<graph\s+sheet="([A-Za-z0-9_]+)"\s*/?>(\s*</graph>)?` in each chapter's raw content. For each
match:

1. Resolve the chapter name from the chapter's `source_path` (its filename stem — e.g.
   `tutorial.md` → `tutorial`).
2. Check `book-src/examples/<chapter>/<name>.adm2` exists (relative to `PreprocessorContext`'s
   root). If missing, return an `Err` from `run()` (mdBook surfaces preprocessor errors as a
   failed build, exactly like a broken `{{#include}}`).
3. Otherwise replace the tag with `<div class="adam-live-graph" data-example="<chapter>/<name>">
   </div>`.

Unlike the include-pass's `inject_mount_points`, this doesn't need fence-avoidance logic — the
tag is written directly in prose, never inside a fenced code block, so it's replaced in place.

### `adam-lang-book/book-src/theme/adam-live-bootstrap.js` (extended)

A second `document.querySelectorAll(".adam-live-graph")` pass alongside the existing
`.adam-live` one, reusing the same already-fetched manifest and wasm module (both mount kinds
share one `import()`/`fetch()` pair — no duplicate network requests). Looks up each div's
`data-example` in the manifest and calls `mount_graph(id, source, name)`. The bootstrap script
also loads `d3.v7.min.js` (in parallel with the existing `swc.js` load) since `GraphView`
depends on it and `SheetInspector` doesn't.

### `xtask::live_book_assets`

`prepare_live_book_assets`'s existing per-file copy loop (`swc.js`, `inspector.css`) gains
`graph.js`, `graph.css`, `d3.v7.min.js`.

### `book.toml`

`additional-css` gains `book-src/theme/graph.css`.

## Data flow

1. **Authoring:** a chapter's markdown places `<graph sheet="first_sheet">` after the prose
   discussing that example — no fixed positional relationship to the `{{#include}}` required.
2. **Book build:** `mdbook-live-examples` validates and rewrites the tag into a
   `div.adam-live-graph` mount point (failing the build on a bad reference); the wasm build step
   produces `mount`/`mount_graph` in the same bundle; `xtask` copies the manifest, Spectrum
   bundle, and now the D3/graph assets into the book's theme directory.
3. **Page load:** the bootstrap script mounts a `SheetInspector` for every `.adam-live` div and
   a `GraphView` for every `.adam-live-graph` div — each an independent Dioxus `VirtualDom`
   rooted at its own element, each `GraphView` backed by its own `graph.js` instance keyed by
   that element's id.
4. **Reader interaction:** dragging/zooming one page's graph never touches another's — each
   instance's D3 simulation and layout state is private to its own closure.

## Testing

- `adam-web-ui::graph`: `to_graph_data`'s existing unit tests move over unchanged (pure
  function, no behavior change from the move itself).
- `adam-lang-book-preprocessor`: new unit tests for the `<graph sheet="...">` regex/replacement
  (self-closing and paired-tag forms), chapter-name resolution from `source_path`, and the
  build-failing error path when the referenced `.adm2` file doesn't exist.
- `xtask::live_book_assets`: extend `build_manifest`/copy tests to cover the three newly-copied
  asset files.
- `graph.js`: no JS test harness exists in this repo today (confirmed — nothing currently tests
  `graph.js`), so the instance-registry rewrite is verified by rendering, not by a JS unit
  suite: `verifying-begin-ui` (screenshot + DOM dump) confirms `begin`'s single graph is
  unaffected; a real-browser check of the built book with at least two chapters' graphs
  simultaneously live on one page confirms they don't share or clobber state (dragging a node in
  one doesn't move anything in the other; switching neither graph's layout leaks into the
  other).
- End-to-end: serve the built book locally and confirm the `first_sheet` example in the tutorial
  renders a live, interactive graph in place of today's placeholder image — `cargo build`/
  `mdbook build` succeeding is not sufficient evidence per this repo's UI verification rule.

## Removed

- `book-src/tutorial.md`'s placeholder `![alt text](image.png)` and `book-src/image.png` itself
  (confirmed unreferenced anywhere else in the book).
- `begin/src/graph_view.rs`, `begin/src/bridge.rs` (contents moved into `adam-web-ui::graph`;
  files deleted once empty).
- `graph.js`'s `sourceChanged`/`relabeledIds`/module-level singleton state.
- `graph.css`'s `html, body` app-shell reset (relocated, not deleted — see Decisions).

## Non-goals (this pass)

- Editing a sheet's cells from within a book page's live graph — a `<graph>` mount has no
  paired `SheetInspector` unless the author also places a separate `<div class="adam-live"
  ...>`/`{{#include}}` widget nearby; this spec doesn't change that pairing, just adds the graph
  as its own independently-placeable mount kind.
- Cross-chapter `<graph sheet="chapter/name">` references — the bare-name-resolved-to-current-
  chapter design assumes a graph only ever references an example already shown earlier in its
  own chapter. Revisit if a future chapter needs to reference another chapter's example.
- Any new CI workflow steps — the existing `wasm-pack build` and `xtask -- prepare-live-book-
  assets` steps already cover the new `mount_graph` export and the extra copied assets with no
  changes to `ci.yml`/`docs.yml` themselves.

## Phases

1. Extract `adam-web-ui::graph` from `begin`; add the `graph_id` prop; verify `begin` unaffected
   (`verifying-begin-ui`).
2. Rewrite `graph.js` as an instance registry; update `GraphView`'s effect logic to
   destroy+init vs. update based on `source_id`; re-verify `begin` (`verifying-begin-ui`),
   including an example-switch check that layout doesn't bleed between sheets.
3. Split `graph.css`'s app-shell reset out; extend `xtask::live_book_assets` and `book.toml`
   for the newly-shared assets.
4. Add `mount_graph` to `adam-lang-book-live`.
5. Extend `adam-lang-book-preprocessor` for `<graph sheet="...">`, with validation and unit
   tests.
6. Extend `adam-live-bootstrap.js` for the `.adam-live-graph` mount kind and the `d3.v7.min.js`
   load.
7. Replace `tutorial.md`'s placeholder image with `<graph sheet="first_sheet">`; remove
   `image.png` if unused elsewhere.
8. End-to-end verification: build the book locally, confirm the tutorial's live graph renders
   and behaves correctly, and confirm two simultaneously-live graphs on one page don't interfere.
