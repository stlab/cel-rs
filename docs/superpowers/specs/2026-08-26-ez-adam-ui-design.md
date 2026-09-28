# ez-adam Phase 2: Dioxus Desktop UI

**Date:** 2026-08-26
**Branch:** worktree-ez-adam-ui
**Status:** Approved (design), not yet implemented

## Summary

Adds the Dioxus desktop UI on top of `ez-adam`'s Phase 1 core (document model,
`ops`, `codegen`, `persistence` — see
`docs/superpowers/specs/2026-08-24-ez-adam-design.md`, §4 of which this
design supersedes with the concrete decisions below). This turns `ez-adam`
from a headless library into a runnable desktop app: a canvas for placing
and connecting cells/relationship-groups/conditional-groups, a toolbar
selecting the active interaction mode, and a side panel for editing
formulas and per-cell properties.

This branch (`worktree-ez-adam-ui`) is stacked on `worktree-ez-adam`
(Phase 1, PR #150, not yet merged) rather than `main`, since Phase 2 needs
Phase 1's crate structure to exist.

---

## 1. Design decisions (settled during brainstorming)

- **`ez-adam` becomes both a library and a binary.** Phase 1's `src/lib.rs`
  is untouched; Phase 2 adds `src/main.rs` plus a new `src/ui/` module
  tree, gated behind a `desktop` Cargo feature — mirroring `begin`'s
  `Cargo.toml` exactly (`default = ["desktop"]`,
  `desktop = ["dioxus/desktop", "dep:rfd"]`). Phase 1's lib tests remain
  unaffected by pulling in desktop UI dependencies.
- **A `web` feature is added purely for automated UI verification** —
  mirroring `begin`'s `web = ["dioxus/web"]`, enabling a
  `verifying-begin-ui`-style headless-screenshot workflow for `ez-adam`,
  since desktop WebView2 can't be driven by standard tooling either.
- **No code sharing with `begin`**, consistent with Phase 1's design —
  `begin`'s patterns (state-per-`use_signal`, `rfd` via async `spawn()`,
  pure-function extraction for testability) are followed as *precedent*,
  not imported as a dependency.
- **A new `NodeId` enum unifies selection across the three canvas-node
  kinds.** `begin` has no precedent for this (confirmed: no
  multi-kind-selection concept exists anywhere in `begin`). It lives in
  `ui::canvas`, not the Phase 1 core model, since it's a UI-only concept:

  ```rust
  enum NodeId {
      CellNode(CellNodeId),
      RelationshipGroup(RelationshipGroupId),
      ConditionalGroup(ConditionalGroupId),
  }
  ```

- **Pan/zoom is in scope from the start**, via a plain UI-layer
  `ViewTransform { x: f64, y: f64, k: f64 }` (pan offset + uniform scale)
  — not `kurbo`-based, since `docs/VISION.md` confirms that
  `cel-runtime`/`kurbo` integration doesn't exist yet. This is
  deliberately *not* `Document`-visible: `Document`'s stored positions
  (`CellNode.position`, `RelationshipGroup.position`,
  `ConditionalGroup.position`) remain in canvas/world space regardless of
  the current pan/zoom; only rendering and hit-testing consult
  `ViewTransform`.
- **Pan/zoom conflicts with the Phase 1 doc's "drag-on-empty-canvas
  rubber-bands a multiselect"; resolved by adopting D3's own convention**
  (which `begin`'s D3 graph view already follows) and common canvas-app
  practice (Figma, tldraw) — see §4 below for the full interaction table.
- **File I/O follows `begin/src/open_file.rs`'s exact pattern**: async via
  `rfd::AsyncFileDialog` driven through Dioxus's `spawn()`, not blocking.
- **Testing follows `adam-web-ui/src/inspector.rs`'s exact pattern**
  (this code originated as `begin/src/inspector.rs`, since extracted into
  the separate `adam-web-ui` crate by unrelated upstream work — the
  pattern itself is unchanged, only its file moved): non-trivial logic
  inside components (branching, combining, or suppressing multiple
  conditions) is extracted into a pure function with its own contract doc
  comment and unit tests; components stay thin `rsx!` wrappers that call
  those functions. This is not a new convention — it is already this
  workspace's documented rule (CLAUDE.md), unrelated to whether `ez-adam`
  depends on `adam-web-ui` itself (it doesn't — see above).

---

## 2. Crate structure

```
ez-adam/
  Cargo.toml           # + dioxus, rfd (desktop feature), web feature
  src/
    lib.rs             # Phase 1, unchanged
    main.rs            # bootstrap only, mirrors begin/src/main.rs
    model/ ops/ codegen/ persistence/ validation.rs   # Phase 1, unchanged
    ui/
      mod.rs
      app.rs           # top-level component: owns all use_signal state,
                        # composes Toolbar + Canvas + SidePanel
      canvas.rs         # NodeId, ViewTransform, canvas rendering + hit-testing
      toolbar.rs        # Tool enum, toolbar component
      side_panel.rs     # context-sensitive property/formula editing panel
      file_io.rs        # open/save/export via rfd (mirrors open_file.rs)
```

```toml
[features]
default = ["desktop"]
desktop = ["dioxus/desktop", "dep:rfd"]
web = ["dioxus/web"]

[dependencies]
dioxus = { version = "0.7.10", features = [] }
rfd = { version = "0.17.2", optional = true }
```

(Exact `dioxus`/`rfd` versions to match whatever `begin`'s `Cargo.toml`
currently pins, at implementation time.)

`CLAUDE.md`'s Commands section gains two more clippy invocations for
`ez-adam`, matching the existing `begin` pattern:

```bash
cargo clippy -p ez-adam --no-default-features --all-targets -- -D warnings
cargo clippy -p ez-adam --all-targets -- -D warnings
```

---

## 3. State model

One `use_signal` per piece of state, declared at the top of `App()`,
matching `begin/src/app.rs`'s pattern exactly (no combined state struct, no
context provider):

- `document: Signal<Document>` — the loaded/edited document (Phase 1's
  `Document`).
- `document_path: Signal<Option<PathBuf>>` — the native-format file
  currently open, if any (for "Save" vs. "Save As").
- `selection: Signal<HashSet<NodeId>>` — currently selected canvas nodes.
- `active_tool: Signal<Tool>` — `Select | AddRelationship | AddConditional
  | Duplicate`.
- `view_transform: Signal<ViewTransform>` — current pan/zoom.
- Transient drag/rubber-band state (drag origin, in-progress rubber-band
  rect, in-progress relationship-tool click sequence) — held in a small
  number of additional signals rather than one combined "interaction
  state" enum, matching `begin`'s flat-signals style.

---

## 4. Canvas: rendering, coordinates, and interactions

**Rendering:** plain SVG rendered by Dioxus from `Document` positions
transformed by `view_transform` — cells as rounded rects, relationship
groups as filled circles, conditional groups as diamonds, edges as lines
between bound positions. (`begin/src/graph_view.rs`'s `GraphLegend`
confirms plain `svg`/`line`/`path`/`circle` elements work as first-class
`rsx!` nodes in this Dioxus version — useful as syntax reference; the
D3/JS-bridge machinery around it is not reused.)

**Coordinate spaces:** `Document` positions are canvas/world space,
constant regardless of pan/zoom. Mouse events arrive in screen space and
are converted through `ViewTransform`'s inverse before any hit-testing
against `Document` positions — a pure function
(`screen_to_canvas(transform, screen_point) -> Point`) with its own
contract and unit tests, per the testing pattern in §1.

**Interaction table** (resolves the pan/zoom-vs-rubber-band conflict from
§1):

| Gesture | Select tool | Add Relationship / Add Conditional tools |
| --- | --- | --- |
| Click a node | Select it (drives side panel) | Tool-specific (pick relationship endpoints / conditional target) |
| Drag a node | Move it (updates `Document` position) | (tools don't drag nodes) |
| Drag empty canvas | **Pan the view** (`ViewTransform.x/y`) | Pan the view (same) |
| Shift+drag empty canvas | Rubber-band multiselect | — |
| Scroll wheel | **Zoom**, centered on cursor (`ViewTransform.k`) | Zoom (same) |
| Duplicate tool, with a selection | Duplicates the selected relationship group + its member nodes (Phase 1's `duplicate_relationship_group`) | — |

Zoom-centered-on-cursor and pan-clamping-to-content (if any) are each their
own pure function with a contract and unit tests, not implemented inline
in an event handler.

---

## 5. Side panel

Unchanged from Phase 1 design §4: context-sensitive on `selection`.

- A **cell**: name, type, output checkbox, clamp min/max (numeric types
  only), restrict expression text. (Per Phase 1's design, `output` and
  `restrict` are captured in the model but not yet reflected in
  `.adm2` codegen — see issues #146/#147 — so editing them here is
  legitimate UI work now, ahead of upstream `adam-lang` support landing.)
- A **relationship group**: its editable `name := expr` formula list
  (drag-reorderable), each formula's CEL syntax validated live via
  `validation::validate_cel_expression` with rustc-style diagnostics
  rendered the same way `adam-web-ui/src/labels.rs` does via
  `annotate-snippets` (`begin`'s own `SourcePanel`, cited in an earlier
  draft of this design, has since been retired — see
  `docs/VISION.md`'s note that it was scaffolding pending VSCode interop;
  `adam-web-ui` is the current precedent for this exact rendering
  technique, independent of whether `ez-adam` depends on that crate).
- A **conditional group**: the enable-table (rows = branches, columns =
  relationship groups, checkboxes = `enabled_groups`) plus the condition
  editor (cell list for `ConditionExpr::Cells`, formula text for
  `ConditionExpr::Formula`).

---

## 6. File I/O

Three actions, each following `begin/src/open_file.rs`'s async
`rfd::AsyncFileDialog` + Dioxus `spawn()` pattern:

- **Open**: pick a `.json` (or whatever extension the native format uses)
  file, `persistence::from_json` it into `document`/`document_path`.
- **Save / Save As**: `persistence::to_json`, write via `std::fs` (sync,
  matching `begin`'s file-read pattern), to `document_path` or a
  newly-picked path.
- **Export `.adm2`**: `codegen::generate_adm2`, write to a
  separately-picked `.adm2` path. One-way — this path is never read back
  in, per Phase 1's design.

---

## 7. Testing and verification

- Every non-trivial piece of logic inside a component (tool dispatch,
  hit-testing, `screen_to_canvas`, rubber-band containment, zoom-centering
  math, drag-delta application) is a pure function with a contract-style
  doc comment and unit tests, per `adam-web-ui/src/inspector.rs`'s
  established pattern (see §1's note on this file's provenance) — never
  left inline in an `rsx!` event handler closure.
- Visual/rendering verification uses a `web`-feature headless build,
  screenshotted the same way `begin`'s `verifying-begin-ui` skill does,
  since desktop WebView2 isn't drivable by standard tooling.
- `Document` mutations triggered by UI actions still go through Phase 1's
  `ops::*` functions exclusively — the UI layer never mutates `Document`
  fields directly.

---

## 8. Deferred / explicitly out of scope

- Everything Phase 1's design already deferred (live evaluation preview,
  `.adm2` import, multiple sheets per document, `adam-lang`'s full numeric
  type registry) — unchanged.
- Undo/redo — not mentioned in the sketches or raised during brainstorming;
  a later addition if needed.
- Keyboard shortcuts beyond whatever Shift+drag requires — no menu bar or
  shortcut system is being added now, matching `begin/src/main.rs`'s
  minimal-bootstrap precedent (no menu bar there either).
- Collaborative/multi-window editing.
- Web/mobile as real shipping targets for `ez-adam` itself (the new `web`
  feature exists solely for headless UI verification, not as a product
  target — consistent with Phase 1's "desktop only for v1" decision).
