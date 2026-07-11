# begin Graph View Live-Resize Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the graph view's visible canvas grow and shrink with its container continuously, instead of being frozen at its size from mount, without disturbing the user's current pan/zoom or reintroducing content distortion.

**Architecture:** `begin/assets/graph.js`'s `init()` currently uses a one-shot `ResizeObserver` that measures the container once, builds the SVG at that fixed pixel size, and disconnects. This plan makes the observer continuous: its first firing still builds the graph exactly as today; every later firing calls a new `resizeCanvas()` helper that updates the existing SVG's `width`/`height`/`viewBox` together (keeping them pixel-equal, which is what prevents the old `preserveAspectRatio` distortion bug), nudges the simulation's center force to match, and calls the existing `updateZoomConstraints()` to recompute pan/zoom bounds — which already preserves the current transform, only re-clamping if it now falls outside the new bounds.

**Tech Stack:** Plain JS (`begin/assets/graph.js`, no build step, no test runner), D3.js v7.

**Full context:** see the design spec at `docs/superpowers/specs/2026-07-10-begin-graph-view-live-resize-design.md`.

## Global Constraints

- `graph.js` has no test runner in this repo — verification for this task is manual, via `dx serve --platform desktop` run from `begin/`.
- No debouncing: each `ResizeObserver` firing is cheap (attribute updates plus the existing constraint recomputation) and the force simulation is not restarted, so resize events don't need to be coalesced.
- The view area must track the container symmetrically in both directions (grow and shrink) — no minimum/high-water-mark size.
- A resize must never change the current zoom scale or pan position on its own; it only changes how much canvas is available.

---

## Task 1: Continuous `ResizeObserver` with in-place canvas resizing

**Files:**
- Modify: `begin/assets/graph.js:153-178` (the `init()` function)

**Interfaces:**
- Consumes: `buildGraph(container, data)`, `updateZoomConstraints()`, module vars `svg`, `simulation`, `width`, `height` (all pre-existing, from the pan/zoom work already in this file).
- Produces: `resizeCanvas()` — new internal helper, no parameters, no return value; reads and uses the current module-level `width`/`height`. Nothing outside this file calls it; it exists purely so `init()`'s observer callback can branch to it.

- [ ] **Step 1: Replace `init()`**

The current `init()` (`begin/assets/graph.js:153-178`) reads:

```javascript
    function init(containerId, data) {
        // Tear down any previous init (component remount / hot-reload).
        if (resizeObserver) { resizeObserver.disconnect(); resizeObserver = null; }
        if (simulation) { simulation.stop(); simulation = null; }
        if (svg) { svg.remove(); svg = null; }
        zoom = null;
        zoomLayer = null;
        hasInitialFit = false;
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
```

Replace it with:

```javascript
    function init(containerId, data) {
        // Tear down any previous init (component remount / hot-reload).
        if (resizeObserver) { resizeObserver.disconnect(); resizeObserver = null; }
        if (simulation) { simulation.stop(); simulation = null; }
        if (svg) { svg.remove(); svg = null; }
        zoom = null;
        zoomLayer = null;
        hasInitialFit = false;
        nodes = [];
        links = [];

        var container = document.getElementById(containerId);

        // Keep observing for the life of this mount (torn down above on the
        // next init() call) so the view area tracks the container's size
        // continuously, not just once at mount. The first firing measures
        // after layout has settled — a plain clientWidth/clientHeight read
        // here can race layout and return a stale (often zero) size, which
        // is what made the graph appear cut off on first load — and builds
        // the graph; every later firing just resizes the existing canvas.
        resizeObserver = new ResizeObserver(function () {
            width = container.clientWidth || width;
            height = container.clientHeight || height;
            if (!svg) {
                buildGraph(container, data);
            } else {
                resizeCanvas();
            }
        });
        resizeObserver.observe(container);
    }

    // Resizes the existing SVG to the current width/height without touching
    // node positions or restarting the simulation. Keeps viewBox equal to the
    // pixel size (not just fixed) so the browser never stretches existing
    // content to fill the new size — that mismatch is what caused the graph
    // to visually distort on resize before pan/zoom existed. Recomputing the
    // zoom constraints preserves the user's current pan/zoom, only
    // re-clamping it if it now falls outside the new bounds.
    function resizeCanvas() {
        svg.attr('width', width)
            .attr('height', height)
            .attr('viewBox', [0, 0, width, height]);
        simulation.force('center').x(width / 2).y(height / 2);
        updateZoomConstraints();
    }
```

(Only `init()` changes and the new `resizeCanvas()` function is added immediately after it; `buildGraph()`, `updateZoomConstraints()`, and everything else in the file are unchanged.)

- [ ] **Step 2: Manual verification via `dx serve`**

Run (from `begin/`): `dx serve --platform desktop`

1. Wait for the app to load with `demo.pm`'s graph. Confirm it fits its initial container with no part cut off, same as before this change.
2. Resize the OS window larger (drag an edge to make it noticeably wider and taller).
3. Confirm the graph view's visible canvas grows to fill the new space, and existing content is **not** stretched or distorted — node/text sizes stay the same as before the resize.
4. Use the `+` zoom button (or scroll-wheel) to zoom in, then resize the window larger again.
5. Confirm the zoom level and pan position are unchanged by the resize, and more of the graph becomes visible within the larger canvas (rather than the view snapping back to a fit scale).
6. Shrink the window back down, including below the size where the current pan/zoom would fall outside the graph's bounds.
7. Confirm the view re-clamps back within bounds (pan and/or scale adjusts back in) rather than showing content pulled outside the visible area, matching the existing re-clamping behavior already used for structural changes.
8. Click the "Fit" button after resizing; confirm it still fits the whole graph into the current (possibly resized) canvas.

This step has no automated equivalent — it's the acceptance check for this task's deliverable, per the `verify` skill.

- [ ] **Step 3: Commit**

```bash
git add begin/assets/graph.js
git commit -m "$(cat <<'EOF'
feat(begin): live-resize the graph view area, independent of zoom scale

The ResizeObserver in init() disconnected after its first firing, freezing
the SVG's pixel size and viewBox at whatever the container measured at
mount. Keep observing continuously instead: every later firing now resizes
the existing SVG in place (keeping viewBox pixel-equal to avoid the old
preserveAspectRatio distortion bug) and recomputes zoom bounds via the
existing updateZoomConstraints(), which already preserves the user's current
pan/zoom rather than resetting it.
EOF
)"
```
