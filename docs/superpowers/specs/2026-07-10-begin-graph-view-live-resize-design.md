# begin Graph View: live-resizing view area (scale unchanged)

## Context

The graph view's `<svg>` is currently sized once, at mount, via a one-shot
`ResizeObserver` (see `docs/superpowers/specs/2026-07-10-begin-graph-pan-zoom-design.md`,
Part 1): the observer measures the container after its first real layout,
sets the SVG's pixel `width`/`height` and `viewBox` to that size, and then
disconnects — the SVG never resizes again for the life of that mount.

That one-shot behavior was a deliberate fix for a *different* bug: before it,
the SVG had `width: 100%; height: 100%` with a fixed `viewBox`, so the
browser's default `preserveAspectRatio` handling visually stretched/distorted
the graph's content on every window resize. Freezing both the pixel size and
the `viewBox` together (instead of just the `viewBox`) stopped the distortion,
at the cost of the view area no longer tracking the container's size at all —
a later window/pane resize just clips or blank-pads the frozen canvas.

Now that pan/zoom exists (same spec) with `updateZoomConstraints()` already
preserving the user's current pan/zoom transform across bounds recalculation
(only re-clamping if the current view falls outside new bounds, never
resetting to fit), the same "keep `viewBox` and pixel size equal" trick can
be applied continuously instead of once: the view area can track the
container's size on every resize without reintroducing the distortion bug,
because `viewBox` is updated in lockstep with the pixel size rather than
staying fixed while the size changes.

## Goal

The graph view's visible canvas grows and shrinks with its container
(window resize, sidebar toggle, etc.) so the graph is never needlessly
clipped. A resize only changes the amount of canvas around/within reach of
the current view — it never *directly* changes zoom level or pan position.
If the user is zoomed in past fit scale, a larger container reveals more of
the graph at the same zoom level, rather than zooming out to fit.

This is deliberately *not* the same as "zoom is always pixel-identical
before and after a resize": fit scale is itself defined relative to the
container size (the scale at which the whole graph exactly fills it), so it
rises as the container grows. If the user hasn't zoomed past fit — i.e.
their current scale is sitting at that floor — enlarging the window carries
the floor up with it, and the existing re-clamp in `updateZoomConstraints()`
(which exists to pull an out-of-bounds transform back in, not specifically
for resizes) brings the scale up to match, so the graph keeps filling the
larger pane rather than leaving blank canvas around it. This is intentional:
"scale changes" here is a *consequence* of re-clamping to newly-recomputed
bounds using the same mechanism already used for structural changes, not a
resize handler independently deciding to rescale anything. Only a user who
has zoomed in past fit is guaranteed an unchanged scale across a resize.

## Design

All changes are confined to `begin/assets/graph.js`; no Rust/Dioxus changes
are needed (`begin/src/graph_view.rs`'s `onmounted` → `init()` call is
unaffected).

### Continuous `ResizeObserver`

`init()`'s `ResizeObserver` stops disconnecting after its first callback and
keeps observing the container for the life of the mount (still torn down and
recreated on the next `init()` call, same as today — component remount /
hot-reload behavior is unchanged). Every time it fires:

1. Update the module-level `width`/`height` from `container.clientWidth`/`clientHeight`.
2. **First firing** (`svg` not yet built): unchanged from today — call
   `buildGraph(container, data)` to construct the SVG, layers, simulation,
   and zoom behavior at the measured size.
3. **Every later firing** (`svg` already exists): update the existing
   `<svg>`'s `width`, `height`, and `viewBox` attributes together, so pixel
   size and `viewBox` stay equal (this is what prevents the old distortion
   bug from reappearing — a `viewBox` that lags the pixel size is exactly
   what caused it before). Update the simulation's `forceCenter` target to
   the new `width/2, height/2` (has no visible effect until a future
   structural-change resettle, but keeps it from being stale then). Call the
   existing `updateZoomConstraints()` to recompute `scaleExtent` /
   `translateExtent` / `extent` for the new size — its existing "not the
   first fit" branch already preserves the current pan/zoom transform,
   re-clamping only if it now falls outside the new bounds, so this step
   never resets or refits the view on its own.

No debouncing: each firing is cheap (attribute updates plus the existing
constraint recomputation), and the force simulation is not restarted, so
there's no need to coalesce rapid resize events.

### Shrinking

The same mechanism runs symmetrically in both directions — a container
shrinking is just another `ResizeObserver` firing with smaller dimensions,
updated exactly the same way. There is no minimum/high-water-mark size; the
view area always matches the container's current size.

## Testing

`graph.js` has no test runner in this repo (per the pan/zoom spec); this is
verified manually via `dx serve --platform desktop`:

- Load the graph, confirm it fits its initial container as today.
- Resize the window (or otherwise resize the container, e.g. a future
  sidebar toggle): confirm the visible canvas grows/shrinks with it, with no
  stretching/distortion of existing content.
- Without touching zoom (still at fit scale), enlarge the window: confirm
  the graph zooms in just enough to keep filling the new, larger pane (the
  expected fit-scale-tracks-the-floor behavior — see Goal), not left at its
  old pixel scale with blank canvas around it.
- Zoom in *past* fit scale, then resize the window larger: confirm the zoom
  level and pan position are unchanged, and more of the graph becomes
  visible/reachable within the larger canvas.
- Shrink the window below the current pan/zoom's bounds: confirm the view
  re-clamps back within bounds (same re-clamping behavior already verified
  for structural changes in the pan/zoom spec) rather than showing an
  invalid/out-of-bounds transform.
