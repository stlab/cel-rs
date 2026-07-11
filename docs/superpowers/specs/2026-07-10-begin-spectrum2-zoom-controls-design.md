# Begin: Spectrum 2 Migration + Zoom Control Redesign

**Date:** 2026-07-10
**Branch:** worktree-begin-improvements
**Status:** Approved — ready for implementation planning

## Overview

Two related changes to the `begin` app's Spectrum Web Components (SWC) integration:

1. Migrate the app's theme system from Spectrum 1 to Spectrum 2 (`system="spectrum-two"`),
   as anticipated (and deferred) in the original
   [Spectrum integration design](2026-06-25-spectrum-integration-design.md).
2. Replace the graph view's plain-HTML zoom control cluster (`+` / `−` / `Fit` buttons in
   [begin/src/graph_view.rs](../../../begin/src/graph_view.rs)) with a proper Spectrum
   segmented-toolbar control using `sp-action-group` and `sp-action-button`, with real
   Spectrum workflow icons for zoom in/out.

Both changes touch the same files (`versions.toml`, `spectrum.rs`, `app.rs`), so they're
planned together, but they are independently testable/revertable.

## Part 1: Spectrum 2 theme migration

### Bundle version bump

`begin/assets/versions.toml`'s `[spectrum-web-components]` entry moves from `0.45.0`
(Spectrum 1 only) to `1.12.2` (latest at time of writing; Spectrum 2 support requires
`@spectrum-web-components` packages at `>=1.0.0`). The URL pattern is unchanged — esm.sh
serves a self-contained bundle at the same `es2022/elements.bundle.mjs` path for the new
version:

```toml
[spectrum-web-components]
version = "1.12.2"
url = "https://esm.sh/@spectrum-web-components/bundle@1.12.2/es2022/elements.bundle.mjs"
file = "swc.js"
```

Verified (via direct fetch): this URL resolves to a ~185KB self-contained ESM module with
no external relative imports — same "single vendored file" property as the current 0.45.0
asset. It does **not** include `infield-button` or `icons-workflow` (confirmed by
inspecting the unbundled `elements.js` entry point's import list) — those remain separate
concerns, relevant to Part 2.

### `SpTheme` gains a `system` prop

`SpTheme` in [begin/src/spectrum.rs](../../../begin/src/spectrum.rs#L21-L30) currently
takes only `color` and `scale`. Add a `system: String` prop, rendered as the `system`
attribute on `<sp-theme>`.

### App root

In [begin/src/app.rs](../../../begin/src/app.rs#L87-L89), the `SpTheme` call gains
`system: "spectrum-two".to_string()` alongside the existing `color`/`scale` values.
No other component in the app changes — component names and props are stable across
Spectrum 1/2 for everything already in use (`sp-textfield`, `sp-field-label`, `sp-divider`,
`sp-heading`).

Workflow icons (see Part 2) render different SVGs depending on the ambient `system`
automatically (confirmed by inspecting the icon bundle source), so no extra work is needed
there once the theme's `system` attribute is set.

## Part 2: Zoom control redesign

### Component choice: `sp-action-group` (compact), not `sp-infield-button`

The initial direction considered `sp-infield-button` (a component that visually attaches
a button to one side of an input field by rounding only its outer corner). That component
is the wrong fit here: our control isn't attached to a field, and mixing an infield-button
with a plain button in the middle would need manual CSS (zeroed gap) to fake a flush seam,
with no guarantee the center button's corners come out square.

`sp-action-group` with the `compact` attribute is Spectrum's purpose-built segmented-toolbar
pattern: a horizontal row of `sp-action-button` children where `compact` removes inter-button
gaps and rounds *only* the outermost corners of the group — every interior button, including
a lone middle button, renders square on both sides automatically, driven by DOM position.
Left-to-right DOM order gives correct left/right placement (zoom-out on the left, zoom-in on
the right) without any `inline`/positional prop bookkeeping.

### New vendored asset: workflow icons

`sp-icon-zoom-in` / `sp-icon-zoom-out` come from `@spectrum-web-components/icons-workflow`,
not from the main bundle. Two new `versions.toml` entries, each a verified self-contained
esm.sh bundle (~18-24KB, no external imports):

```toml
[spectrum-icon-zoom-in]
version = "1.12.2"
url = "https://esm.sh/@spectrum-web-components/icons-workflow@1.12.2/es2022/icons/sp-icon-zoom-in.bundle.mjs"
file = "swc-icon-zoom-in.js"

[spectrum-icon-zoom-out]
version = "1.12.2"
url = "https://esm.sh/@spectrum-web-components/icons-workflow@1.12.2/es2022/icons/sp-icon-zoom-out.bundle.mjs"
file = "swc-icon-zoom-out.js"
```

Both are loaded in `app.rs` as additional `document::Script { r#type: "module", ... }` tags,
alongside the existing `swc.js` script tag.

### `spectrum.rs` additions

| Dioxus component | SWC element | Key props |
| --- | --- | --- |
| `SpActionGroup` | `sp-action-group` | `compact: bool`, `children` |
| `SpActionButton` | `sp-action-button` | `quiet: bool`, `onclick: EventHandler<MouseEvent>`, `children` |
| `SpIconZoomIn` | `sp-icon-zoom-in` | none |
| `SpIconZoomOut` | `sp-icon-zoom-out` | none |

`SpActionButton`'s `children` slot carries either an icon component (zoom in/out) or plain
text (the "Fit" label) — both are valid `sp-action-button` content.

### `graph_view.rs` changes

The `.graph-zoom-controls` `div`'s three plain `<button>` elements
([begin/src/graph_view.rs:54-80](../../../begin/src/graph_view.rs#L54-L80)) are replaced by:

```rust
SpActionGroup {
    compact: true,
    SpActionButton { onclick: /* zoomOut */, SpIconZoomOut {} }
    SpActionButton { onclick: /* resetZoom */, "Fit" }
    SpActionButton { onclick: /* zoomIn */,  SpIconZoomIn {} }
}
```

Click handlers are unchanged — same `document::eval` calls into `window.beginGraph.zoomOut
/ zoomIn / resetZoom` as today.

### `graph.css` changes

Remove the now-dead `.graph-zoom-controls button` and `.graph-zoom-controls button:hover`
rules (lines 84-97) — Spectrum styles its own shadow DOM and `compact` handles spacing
internally, so these rules no longer apply to anything. `.graph-zoom-controls`'s
positioning rules (`position: absolute; top: 8px; right: 8px; z-index: 10`) are kept; the
`display: flex; gap: 4px` on the container becomes unnecessary once `sp-action-group` owns
layout, but the outer positioning div itself is still needed to anchor the control to the
top-right corner of `#graph-container`.

## Out of scope

- Dark mode (still deferred from the original integration spec).
- Migrating any other component beyond what's touched here (Inspector's `sp-textfield` /
  `sp-field-label` / `sp-divider` / `sp-heading` usage is unaffected — those component
  names and props are stable across Spectrum 1/2).
- A live zoom-percentage readout in the control (considered and declined during
  brainstorming — "Fit" stays a plain text label, not a field).
- Tree-shaking / npm bundling of SWC assets (full per-component esm.sh bundles remain
  acceptable for a desktop dev tool, per the original integration spec).
