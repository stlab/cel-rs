# Begin: Spectrum 2 Migration + Zoom Control Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate `begin`'s Spectrum Web Components theme to Spectrum 2, and replace the graph view's plain-HTML zoom button cluster with a proper `sp-action-group`/`sp-action-button` segmented control using real Spectrum workflow icons.

**Architecture:** Bump the vendored SWC bundle version in `versions.toml` and add two new vendored icon assets, fetched via the existing `cargo xtask fetch-assets` mechanism. Add a `system` prop to the existing `SpTheme` wrapper and set it to `"spectrum-two"`. Add `SpActionGroup`, `SpActionButton`, `SpIconZoomIn`, `SpIconZoomOut` wrappers to `spectrum.rs` (the codebase's single file that knows raw SWC element names), then swap `graph_view.rs`'s three plain `<button>` elements for a `compact` `sp-action-group` of three `sp-action-button`s, removing the now-dead CSS.

**Tech Stack:** Rust, Dioxus 0.7 (RSX/`#[component]`), Spectrum Web Components 1.12.2 (esm.sh-vendored), `xtask` asset fetcher (`ureq`).

## Global Constraints

- Spectrum packages must be `>=1.0.0` for Spectrum 2 (`system="spectrum-two"`) support — pin to `1.12.2` everywhere.
- Every vendored asset must be a self-contained single-file ES module (no external runtime network dependency) — use the exact `es2022/*.bundle.mjs` esm.sh URLs verified during brainstorming, not the bare (non-bundled) entry points.
- `versions.toml` is the single source of truth for vendored asset versions/URLs; assets are updated via `cargo xtask fetch-assets`, never hand-edited or hand-downloaded.
- `begin/src/spectrum.rs` remains the only module in the codebase that references raw SWC element/tag names (e.g. `sp-action-group`); all other modules import `SpXxx` wrapper components from it.
- This codebase has no automated UI/DOM test harness for Dioxus RSX wrapper components or for `graph.js`/the graph view (confirmed: neither `spectrum.rs` nor `graph_view.rs` has a `#[cfg(test)]` module, and the pan-zoom design spec explicitly notes `graph.js` has none either). Verification for UI-only tasks in this plan is compile-time (`cargo build` / `cargo clippy`) plus a manual smoke test via the `run` skill — there is no gap here relative to existing project precedent.
- Before opening a PR: `cargo fmt --all`, a zero-warning `cargo build --workspace` and `cargo test --workspace`, and all three clippy invocations (`cargo clippy --workspace --exclude begin --all-targets -- -D warnings`, `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`, `cargo clippy -p begin --all-targets -- -D warnings`) must pass, per `CLAUDE.md`.

---

### Task 1: Vendor the Spectrum 2 bundle and zoom icon assets

**Files:**
- Modify: `begin/assets/versions.toml`
- Generated (via `cargo xtask fetch-assets`, commit the output): `begin/assets/swc.js`, `begin/assets/swc-icon-zoom-in.js`, `begin/assets/swc-icon-zoom-out.js`

**Interfaces:**
- Produces: `begin/assets/swc.js` (updated to SWC 1.12.2, loaded via the existing `document::Script { r#type: "module", src: asset!("/assets/swc.js") }` in `app.rs`), `begin/assets/swc-icon-zoom-in.js` and `begin/assets/swc-icon-zoom-out.js` (new files that Task 3 wires into `app.rs` as additional script tags).

- [ ] **Step 1: Edit `begin/assets/versions.toml`**

Replace the existing `[spectrum-web-components]` section and append two new sections, so the file reads:

```toml
[d3]
version = "7.9.0"
url = "https://cdn.jsdelivr.net/npm/d3@7.9.0/dist/d3.min.js"
file = "d3.v7.min.js"

[spectrum-web-components]
# 1.12.2 is the latest release at time of writing and is the first line to support
# Spectrum 2 (system="spectrum-two") — Spectrum 2 requires >=1.0.0.
# URL uses esm.sh's self-bundled endpoint for a self-contained single-file ESM bundle
# (verified to contain no external relative imports).
version = "1.12.2"
url = "https://esm.sh/@spectrum-web-components/bundle@1.12.2/es2022/elements.bundle.mjs"
file = "swc.js"

[spectrum-icon-zoom-in]
# icons-workflow is not part of the main bundle package (verified: bundle's elements.js
# entry point has no "icons-workflow" import), so each icon needed is vendored separately.
version = "1.12.2"
url = "https://esm.sh/@spectrum-web-components/icons-workflow@1.12.2/es2022/icons/sp-icon-zoom-in.bundle.mjs"
file = "swc-icon-zoom-in.js"

[spectrum-icon-zoom-out]
version = "1.12.2"
url = "https://esm.sh/@spectrum-web-components/icons-workflow@1.12.2/es2022/icons/sp-icon-zoom-out.bundle.mjs"
file = "swc-icon-zoom-out.js"
```

- [ ] **Step 2: Fetch the assets**

Run: `cargo xtask fetch-assets`

Expected output (byte counts will vary slightly):

```
Fetching d3 v7.9.0 ...
  -> .../begin/assets/d3.v7.min.js (...)
Fetching spectrum-web-components v1.12.2 ...
  -> .../begin/assets/swc.js (...)
Fetching spectrum-icon-zoom-in v1.12.2 ...
  -> .../begin/assets/swc-icon-zoom-in.js (...)
Fetching spectrum-icon-zoom-out v1.12.2 ...
  -> .../begin/assets/swc-icon-zoom-out.js (...)
```

All four lines must appear with no `error:` line. If a URL 404s, re-verify it directly (e.g. `curl -sI <url>`) before proceeding — do not silently substitute a different path.

- [ ] **Step 3: Verify the fetched files register the expected custom elements**

Run (from the repo root):

```bash
grep -c "sp-action-group" begin/assets/swc.js
grep -c "sp-action-button" begin/assets/swc.js
grep -c "sp-icon-zoom-in" begin/assets/swc-icon-zoom-in.js
grep -c "sp-icon-zoom-out" begin/assets/swc-icon-zoom-out.js
```

Expected: each command prints a number `>= 1` (the minified bundles contain the
`customElements.define("sp-action-group", ...)`-style registration string as a literal
substring). If any command prints `0`, the fetch pulled the wrong file — stop and
re-check the URL in `versions.toml`.

- [ ] **Step 4: Commit**

```bash
git add begin/assets/versions.toml begin/assets/swc.js begin/assets/swc-icon-zoom-in.js begin/assets/swc-icon-zoom-out.js
git commit -m "chore(begin): vendor Spectrum 2 bundle and zoom workflow icons"
```

---

### Task 2: Migrate the theme to Spectrum 2 (`system` prop)

**Files:**
- Modify: `begin/src/spectrum.rs:21-30` (`SpTheme`)
- Modify: `begin/src/app.rs:87-89` (`SpTheme` call site)

**Interfaces:**
- Consumes: nothing new from Task 1 beyond the already-vendored `swc.js` (this task doesn't touch the icon assets).
- Produces: `SpTheme(color: String, scale: String, system: String, children: Element) -> Element` — Task 3 and Task 4 do not call `SpTheme` directly, so no other task depends on this signature.

- [ ] **Step 1: Add the `system` prop to `SpTheme`**

In `begin/src/spectrum.rs`, replace:

```rust
/// Provides Spectrum token context for all descendant SWC components.
///
/// Must be the root ancestor of any `SpXxx` component. Maps to `<sp-theme>`.
#[component]
pub fn SpTheme(color: String, scale: String, children: Element) -> Element {
    rsx! {
        sp-theme {
            "color": "{color}",
            "scale": "{scale}",
            {children}
        }
    }
}
```

with:

```rust
/// Provides Spectrum token context for all descendant SWC components.
///
/// Must be the root ancestor of any `SpXxx` component. Maps to `<sp-theme>`.
#[component]
pub fn SpTheme(color: String, scale: String, system: String, children: Element) -> Element {
    rsx! {
        sp-theme {
            "color": "{color}",
            "scale": "{scale}",
            "system": "{system}",
            {children}
        }
    }
}
```

- [ ] **Step 2: Pass `system: "spectrum-two"` at the call site**

In `begin/src/app.rs`, replace:

```rust
        SpTheme {
            color: "light".to_string(),
            scale: "medium".to_string(),
            div {
```

with:

```rust
        SpTheme {
            color: "light".to_string(),
            scale: "medium".to_string(),
            system: "spectrum-two".to_string(),
            div {
```

- [ ] **Step 3: Build to confirm the new prop wires through**

Run: `cargo build -p begin --no-default-features`
Expected: builds with no errors or warnings.

- [ ] **Step 4: Lint**

Run: `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add begin/src/spectrum.rs begin/src/app.rs
git commit -m "feat(begin): migrate SpTheme to Spectrum 2 (system=spectrum-two)"
```

---

### Task 3: Add `SpActionGroup`, `SpActionButton`, and zoom icon wrappers

**Files:**
- Modify: `begin/src/spectrum.rs` (append new components after `SpHeading`)
- Modify: `begin/src/app.rs:83-85` (add two new script tags)

**Interfaces:**
- Consumes: `begin/assets/swc-icon-zoom-in.js` and `begin/assets/swc-icon-zoom-out.js` from Task 1.
- Produces: `SpActionGroup(compact: bool, children: Element) -> Element`, `SpActionButton(quiet: bool, onclick: EventHandler<MouseEvent>, children: Element) -> Element`, `SpIconZoomIn() -> Element`, `SpIconZoomOut() -> Element` — all consumed by Task 4.

- [ ] **Step 1: Append the new components to `begin/src/spectrum.rs`**

After the existing `SpHeading` component (end of file), add:

```rust

/// Groups a row of `SpActionButton`s into a single visual cluster.
///
/// Maps to `<sp-action-group>`. Setting `compact` to `true` removes the gaps between
/// buttons and rounds only the group's outermost corners — interior buttons (including
/// a lone middle button) render square on both sides.
#[component]
pub fn SpActionGroup(compact: bool, children: Element) -> Element {
    rsx! {
        sp-action-group {
            "compact": if compact { "true" },
            {children}
        }
    }
}

/// A single button within an `SpActionGroup` (or standalone).
///
/// Maps to `<sp-action-button>`. Setting `quiet` to `true` renders the SWC diminished
/// visual-prominence state.
#[component]
pub fn SpActionButton(quiet: bool, onclick: EventHandler<MouseEvent>, children: Element) -> Element {
    rsx! {
        sp-action-button {
            "quiet": if quiet { "true" },
            onclick: move |e| onclick.call(e),
            {children}
        }
    }
}

/// Zoom-in glyph, used as `SpActionButton` icon content.
///
/// Maps to `<sp-icon-zoom-in>`.
#[component]
pub fn SpIconZoomIn() -> Element {
    rsx! {
        sp-icon-zoom-in {}
    }
}

/// Zoom-out glyph, used as `SpActionButton` icon content.
///
/// Maps to `<sp-icon-zoom-out>`.
#[component]
pub fn SpIconZoomOut() -> Element {
    rsx! {
        sp-icon-zoom-out {}
    }
}
```

- [ ] **Step 2: Load the two new icon scripts in `begin/src/app.rs`**

Replace:

```rust
        document::Script { r#type: "module", src: asset!("/assets/swc.js") }
```

with:

```rust
        document::Script { r#type: "module", src: asset!("/assets/swc.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc-icon-zoom-in.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc-icon-zoom-out.js") }
```

- [ ] **Step 3: Build**

Run: `cargo build -p begin --no-default-features`
Expected: builds with no errors or warnings. (The new components are unused until Task
4 wires them in — if the compiler warns `unused`, that's expected and will be resolved
by Task 4; do not suppress it here.)

- [ ] **Step 4: Commit**

```bash
git add begin/src/spectrum.rs begin/src/app.rs
git commit -m "feat(begin): add SpActionGroup/SpActionButton/zoom icon component wrappers"
```

---

### Task 4: Replace the zoom button cluster and clean up dead CSS

**Files:**
- Modify: `begin/src/graph_view.rs:54-80`
- Modify: `begin/assets/graph.css:75-97`

**Interfaces:**
- Consumes: `SpActionGroup`, `SpActionButton`, `SpIconZoomIn`, `SpIconZoomOut` from Task 3 (import via `use crate::spectrum::*;`).

- [ ] **Step 1: Replace the button cluster in `begin/src/graph_view.rs`**

Add the import near the top of the file (after the existing `use` lines):

```rust
use crate::spectrum::{SpActionButton, SpActionGroup, SpIconZoomIn, SpIconZoomOut};
```

Replace:

```rust
            div {
                class: "graph-zoom-controls",
                button {
                    onclick: move |_| {
                        spawn(async move {
                            let _ = document::eval("window.beginGraph.zoomIn();").await;
                        });
                    },
                    "+"
                }
                button {
                    onclick: move |_| {
                        spawn(async move {
                            let _ = document::eval("window.beginGraph.zoomOut();").await;
                        });
                    },
                    "-"
                }
                button {
                    onclick: move |_| {
                        spawn(async move {
                            let _ = document::eval("window.beginGraph.resetZoom();").await;
                        });
                    },
                    "Fit"
                }
            }
```

with:

```rust
            div {
                class: "graph-zoom-controls",
                SpActionGroup {
                    compact: true,
                    SpActionButton {
                        quiet: false,
                        onclick: move |_| {
                            spawn(async move {
                                let _ = document::eval("window.beginGraph.zoomOut();").await;
                            });
                        },
                        SpIconZoomOut {}
                    }
                    SpActionButton {
                        quiet: false,
                        onclick: move |_| {
                            spawn(async move {
                                let _ = document::eval("window.beginGraph.resetZoom();").await;
                            });
                        },
                        "Fit"
                    }
                    SpActionButton {
                        quiet: false,
                        onclick: move |_| {
                            spawn(async move {
                                let _ = document::eval("window.beginGraph.zoomIn();").await;
                            });
                        },
                        SpIconZoomIn {}
                    }
                }
            }
```

Note the left-to-right order: zoom-out first (renders leftmost), "Fit" in the middle,
zoom-in last (renders rightmost) — this is what gives the correct left/right placement,
with no positional prop needed.

- [ ] **Step 2: Remove the now-dead button CSS in `begin/assets/graph.css`**

Replace:

```css
.graph-zoom-controls {
    position: absolute;
    top: 8px;
    right: 8px;
    display: flex;
    gap: 4px;
    z-index: 10;
}

.graph-zoom-controls button {
    width: 28px;
    height: 28px;
    border: 1px solid #999;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.85);
    font-family: monospace;
    font-size: 14px;
    cursor: pointer;
}

.graph-zoom-controls button:hover {
    background: #fff;
}
```

with:

```css
.graph-zoom-controls {
    position: absolute;
    top: 8px;
    right: 8px;
    z-index: 10;
}
```

(`sp-action-group[compact]` owns its own internal spacing and border-radius; the
container only needs to anchor the group to the corner of `#graph-container`.)

- [ ] **Step 3: Build**

Run: `cargo build -p begin --no-default-features`
Expected: builds with no errors or warnings (the `unused` warning from Task 3 is now
resolved since all four new components are used).

- [ ] **Step 4: Lint (both begin variants)**

Run:
```bash
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: no warnings from either invocation.

- [ ] **Step 5: Manual smoke test via the `run` skill**

Use the `run` skill to launch the `begin` dev server, then in the running app:

1. Confirm the app renders without a blank/broken page (Spectrum 2 theme tokens
   resolved correctly — if `system` were misspelled or the bundle failed to load, SWC
   components would render unstyled).
2. Locate the zoom control in the top-right corner of the graph view. Confirm it renders
   as a single joined pill: rounded left end, square middle, rounded right end — not
   three separate floating buttons.
3. Click the left (zoom-out) button — confirm the graph zooms out.
4. Click the middle ("Fit") button — confirm the graph resets to fit-scale.
5. Click the right (zoom-in) button — confirm the graph zooms in.
6. Confirm the zoom-out/zoom-in buttons show icon glyphs, not text `+`/`-`.

- [ ] **Step 6: Commit**

```bash
git add begin/src/graph_view.rs begin/assets/graph.css
git commit -m "feat(begin): redesign zoom controls as a compact sp-action-group"
```

---

### Task 5: Full workspace verification before PR

**Files:** none (verification only).

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Expected: no diff (or only whitespace fixes — if any, `git add -u` and amend into the
last commit's follow-up, don't leave unformatted code).

- [ ] **Step 2: Full workspace build**

Run: `cargo build --workspace`
Expected: zero warnings, zero errors.

- [ ] **Step 3: Full workspace tests**

Run:
```bash
cargo test --workspace
cargo test --doc --workspace
```
Expected: all pass, zero warnings in the build output.

- [ ] **Step 4: All three clippy invocations**

Run:
```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: no warnings from any of the three.

- [ ] **Step 5: Commit any formatting fixes (if Step 1 produced a diff)**

```bash
git add -A
git commit -m "chore(begin): cargo fmt"
```

(Skip this step entirely if Step 1 produced no diff.)

## Self-Review Notes

- **Spec coverage:** Part 1 (theme migration) → Task 2. Part 2 (versions.toml + new
  assets) → Task 1; (spectrum.rs components + script tags) → Task 3; (graph_view.rs +
  graph.css) → Task 4. Repo-wide PR checklist → Task 5. No spec section is uncovered.
- **Placeholder scan:** no TBD/TODO markers; every step has complete, literal code or
  exact commands with expected output.
- **Type consistency:** `SpActionGroup(compact: bool, children: Element)`,
  `SpActionButton(quiet: bool, onclick: EventHandler<MouseEvent>, children: Element)`,
  `SpIconZoomIn()`/`SpIconZoomOut()` are defined once in Task 3 and used with matching
  names/arity in Task 4; `SpTheme`'s new `system: String` param (Task 2) matches its one
  call site (also Task 2, same task).
