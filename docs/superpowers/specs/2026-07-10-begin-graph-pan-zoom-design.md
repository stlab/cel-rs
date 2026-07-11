# begin graph view: fixed initial sizing + pan/zoom

## Problem

`GraphView` ([begin/src/graph_view.rs](../../../begin/src/graph_view.rs)) mounts a D3
force graph into a `<div>` via `graph.js`. Two problems:

1. **Sizing bug.** `init()` creates the `<svg>` with `width: 100%`, `height: 100%`, and a
   `viewBox` fixed to `container.clientWidth/clientHeight` captured once at mount. Because
   the SVG element keeps resizing to 100% of its container while the `viewBox` stays fixed,
   the browser's default `preserveAspectRatio` rescales the graph's *content* to match on
   every container/window resize. This also means if the container hasn't finished layout
   at mount time, the initial capture can be wrong, making the graph appear cut off.
2. **No pan/zoom.** There is currently no way to reposition or rescale the view; the force
   simulation centers nodes on `width/2, height/2` with no interaction beyond that.

## Goals

- Graph sizes correctly to the container on first load, and never rescales due to a
  window/container resize afterward.
- Users can pan and zoom the graph.
- Panning cannot move the content past its own bounds.
- Zooming out cannot go smaller than the scale that fits the whole graph in the view.
- Zoom in/out/reset controls are available as on-screen buttons, in addition to
  wheel-to-zoom and drag-to-pan.

## Part 1 — Fixed initial sizing

Replace the one-shot `clientWidth`/`clientHeight` read with a **one-shot `ResizeObserver`**
on the container element: the first callback captures the container's real, fully-laid-out
size, and the observer disconnects immediately after. This is more robust than a raw
`clientWidth` read or a `setTimeout`-based retry, since it fires exactly when layout has
produced a size, regardless of timing.

The `<svg>` is then created with **fixed pixel `width`, `height`, and `viewBox`** (all equal
to the captured size) instead of percentage width/height. Because the dimensions are fixed
pixels, later container or window resizes no longer trigger any `viewBox`-driven rescaling.
The container's existing `overflow: hidden` clips any overflow (or leaves blank space) if the
container's actual size differs from the captured size after a later resize — this is
expected and acceptable, since the point is that the graph itself doesn't rescale.

This sizing capture happens once per `init()` call (i.e., once per component mount /
hot-reload), same as today.

## Part 2 — Pan & zoom

### Structure

Wrap all existing D3 layers in one new group, and attach `d3.zoom()` to the `<svg>`:

```
svg
  g.zoom-layer          <- NEW: zoom/pan transform applied here
    g.bg-layer
    g.control-link-layer
    g.link-layer
    g.cell-layer
    g.rel-layer
    g.cond-layer
    g.label-layer
    g.value-layer
```

`zoom.on('zoom', event => zoomLayer.attr('transform', event.transform))`.

### Bounding box & constraints

After every `settleSimulation()` call (i.e., whenever `update()` detects a structural
change — nodes or links added/removed), recompute the content bounding box from node
positions plus their per-kind visual extents, reusing existing size constants:

- `Cell`: half-extents `CELL_W/2`, `CELL_H/2`
- `Relationship`: radius `REL_R`
- `Conditional`: circumradius `COND_COLLIDE_R`

From the bbox (`minX, minY, maxX, maxY`, `contentWidth = maxX - minX`,
`contentHeight = maxY - minY`):

- `fitScale = min(width / contentWidth, height / contentHeight)`
- `scaleExtent = [fitScale, max(fitScale, MAX_ZOOM)]` where `MAX_ZOOM = 8`. The lower bound
  enforces "zoom out no smaller than fit"; the `max(...)` guards the edge case where a very
  small graph's `fitScale` already exceeds the nominal 8x cap.
- `translateExtent = [[minX, minY], [maxX, maxY]]` — the standard d3-zoom pattern: combined
  with `extent` (the viewport, `[[0,0],[width,height]]`), this stops panning at the content's
  own edges, and combined with the `scaleExtent` floor, panning becomes a no-op exactly at
  fit scale (the full content exactly fills the view, so there's nowhere to pan to).

Apply the new `scaleExtent`/`translateExtent` to the zoom behavior via
`svg.call(zoom.scaleExtent(...).translateExtent(...))`.

### Initial view vs. later structural changes

- **First `init()` call:** set the view directly to the centered fit transform
  (`d3.zoomIdentity` translated/scaled so the bbox center maps to the viewport center at
  `fitScale`).
- **Later structural changes** (`update()` with `structureChanged === true`): do **not**
  reset the user's current pan/zoom. Instead, re-apply the (possibly narrower)
  `scaleExtent`/`translateExtent`, then re-clamp the *existing* transform through
  `svg.call(zoom.transform, d3.zoomTransform(svg.node()))` — d3-zoom's constrain function
  runs on this call, so the view only snaps back if it now falls outside the new bounds.

### Controls

A small button cluster (`+`, `−`, "fit") is added to `GraphView`'s RSX in
[begin/src/graph_view.rs](../../../begin/src/graph_view.rs), absolutely positioned in the
top-right corner of `#graph-container` (which becomes `position: relative`). Buttons call
three new functions exposed on `window.beginGraph`:

- `zoomIn()` / `zoomOut()`: `zoom.scaleBy(svg.transition().duration(200), 1.3)` /
  `(1 / 1.3)`.
- `resetZoom()`: transitions to the current fit transform (recomputed from the live bbox),
  same as the initial-load transform.

## Non-goals

- No changes to the force simulation itself (link distance, charge, collision radii).
- No persistence of pan/zoom state across page reloads or hot-reloads — each `init()` starts
  at fit scale.
- No touch/pinch-specific handling beyond what `d3.zoom()` provides by default.

## Testing

`graph.js` has no existing test harness (it's a plain script, not built/bundled). Verify
manually via the `run` skill / dev server:

- Load the graph; confirm it's fully visible with no cut-off content, at fit scale.
- Resize the browser window; confirm the graph does not rescale (may clip or show blank
  space at container edges, which is expected).
- Drag to pan; confirm it stops at the content edges in every direction.
- Wheel-zoom out; confirm it stops at fit scale (matches "no cut-off, no scroll" state).
- Wheel-zoom in; confirm it stops at 8x.
- Use the `+`/`−`/fit buttons; confirm equivalent behavior to wheel/drag.
- Trigger a structural change (edit `demo.pm` to add/remove a cell or relationship) while
  panned/zoomed away from fit; confirm the view is preserved unless it now falls outside the
  new bounds, in which case it clamps back in.
