# Spectrum Web Components Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Integrate Adobe Spectrum Web Components into the `begin` app, replacing all hand-rolled inline-CSS elements with SWC components under a Spectrum 1 light theme.

**Architecture:** Add an `xtask` workspace crate that reads `begin/assets/versions.toml` and downloads vendored JS assets (D3 and SWC bundle). Create `begin/src/spectrum.rs` with Dioxus element bindings for SWC custom elements and typed component wrappers. Update `app.rs` to load SWC and wrap the UI in `sp-theme`. Update `inspector.rs` to use SWC form components.

**Tech Stack:** Rust, Dioxus 0.7.9, Spectrum Web Components 0.45.4, ureq 3 (xtask only), toml 1 + serde 1 (xtask only).

## Global Constraints

- Dioxus version: 0.7.9
- SWC bundle version: 0.45.0 (`@spectrum-web-components/bundle`; 0.45.4 does not exist on npm)
- D3 version: 7.9.0
- Spectrum theme: `color="light"`, `scale="medium"` (Spectrum 1)
- No npm/Node.js tooling; all JS assets committed to `begin/assets/`
- `cargo xtask fetch-assets` is for **updating** versions, not initial setup
- `cargo clippy -p begin --no-default-features -- -D warnings` must pass
- Every public item in every modified file must have a `///` doc comment (workspace `missing_docs` lint)
- All arithmetic on signed integers must use `checked_*`; this plan adds no arithmetic

---

## File Structure

**New files:**

- `xtask/Cargo.toml` — binary crate, dev-only task runner
- `xtask/src/main.rs` — `fetch-assets` subcommand
- `begin/assets/versions.toml` — vendored asset version manifest
- `begin/assets/swc.js` — downloaded SWC bundle (committed)
- `begin/src/spectrum.rs` — Dioxus element bindings + component wrappers

**Modified files:**

- `Cargo.toml` — add `xtask` to `[workspace] members`
- `.vscode/tasks.json` — add `begin: fetch assets` task
- `begin/src/main.rs` — add `mod spectrum;`
- `begin/src/app.rs` — load `swc.js`, wrap UI in `SpTheme`
- `begin/src/inspector.rs` — replace inline-CSS elements with SWC components

**Unchanged:**

- `begin/assets/graph.js`, `begin/assets/graph.css` — authored JS/CSS
- `begin/src/bridge.rs`, `begin/src/graph_view.rs` — no UI changes

---

## Task 1: xtask crate, asset manifest, and VS Code task

**Files:**

- Create: `xtask/Cargo.toml`
- Create: `xtask/src/main.rs`
- Create: `begin/assets/versions.toml`
- Modify: `Cargo.toml`
- Modify: `.vscode/tasks.json`

**Interfaces:**

- Produces: `cargo xtask fetch-assets` command that downloads assets listed in `begin/assets/versions.toml`

- [ ] **Step 1: Create `begin/assets/versions.toml`**

```toml
[d3]
version = "7.9.0"
url = "https://cdn.jsdelivr.net/npm/d3@7.9.0/dist/d3.min.js"
file = "d3.v7.min.js"

[spectrum-web-components]
version = "0.45.0"
url = "https://esm.sh/@spectrum-web-components/bundle@0.45.0/es2022/elements.bundle.mjs"
file = "swc.js"
```

- [ ] **Step 2: Create `xtask/Cargo.toml`**

```toml
[package]
name = "xtask"
version = "0.1.0"
edition = "2024"
publish = false

[[bin]]
name = "xtask"
path = "src/main.rs"

[dependencies]
ureq = "3"
toml = "1"
serde = { version = "1", features = ["derive"] }
```

- [ ] **Step 3: Create `xtask/src/main.rs`**

```rust
//! Task runner for the cel-rs workspace.
//!
//! Run tasks with `cargo xtask <task>`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// A single vendored asset entry from `begin/assets/versions.toml`.
#[derive(Deserialize)]
struct Asset {
    /// Pinned version string (informational only).
    version: String,
    /// Full URL to download the asset from.
    url: String,
    /// Destination filename within `begin/assets/`.
    file: String,
}

/// Returns the workspace root (one directory above the `xtask` manifest).
fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(1)
        .unwrap()
        .to_path_buf()
}

/// Downloads every asset listed in `begin/assets/versions.toml` into `begin/assets/`.
fn fetch_assets() -> Result<(), Box<dyn std::error::Error>> {
    let root = project_root();
    let manifest_path = root.join("begin").join("assets").join("versions.toml");
    let manifest_str = std::fs::read_to_string(&manifest_path)?;
    let assets: HashMap<String, Asset> = toml::from_str(&manifest_str)?;

    let assets_dir = root.join("begin").join("assets");

    for (name, asset) in &assets {
        let dest = assets_dir.join(&asset.file);
        println!("Fetching {name} v{} ...", asset.version);
        let body = ureq::get(&asset.url).call()?.into_body().read_to_vec()?;
        std::fs::write(&dest, &body)?;
        println!("  -> {} ({} bytes)", dest.display(), body.len());
    }

    Ok(())
}

/// Entry point: dispatches to the named task.
fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("fetch-assets") => {
            if let Err(e) = fetch_assets() {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("Usage: cargo xtask fetch-assets");
            std::process::exit(1);
        }
    }
}
```

- [ ] **Step 4: Add `xtask` to the workspace in `Cargo.toml`**

In the root `Cargo.toml`, change:

```toml
[workspace]
members = [
    "cel-runtime",
    "cel-parser",
    "cel-rs-macros",
    "property-model",
    "begin",
]
```

to:

```toml
[workspace]
members = [
    "cel-runtime",
    "cel-parser",
    "cel-rs-macros",
    "property-model",
    "begin",
    "xtask",
]
```

- [ ] **Step 5: Add the VS Code task in `.vscode/tasks.json`**

Insert the following task into the `"tasks"` array, after the `"begin: serve (web)"` task and before the `"// ============ Build Tasks ============"` comment:

```json
{
    "label": "begin: fetch assets",
    "type": "shell",
    "command": "cargo",
    "args": ["xtask", "fetch-assets"],
    "options": {
        "cwd": "${workspaceFolder}"
    },
    "problemMatcher": ["$rustc"],
    "presentation": {
        "reveal": "always",
        "panel": "dedicated"
    }
},
```

- [ ] **Step 6: Build the xtask crate**

```sh
cargo build -p xtask
```

Expected: compiles without errors or warnings.

- [ ] **Step 7: Run `cargo xtask fetch-assets` and verify output**

```sh
cargo xtask fetch-assets
```

Expected output (sizes may vary):

```text
Fetching d3 v7.9.0 ...
  -> begin\assets\d3.v7.min.js (NNNNN bytes)
Fetching spectrum-web-components v0.45.4 ...
  -> begin\assets\swc.js (NNNNN bytes)
```

Verify both files exist in `begin/assets/` and are non-empty.

- [ ] **Step 8: Commit**

```sh
git add xtask/ begin/assets/versions.toml Cargo.toml .vscode/tasks.json
git commit -m "feat(xtask): add fetch-assets task runner and asset version manifest"
```

---

## Task 2: Download and commit the SWC bundle

**Files:**

- `begin/assets/swc.js` — downloaded by xtask, now committed

**Interfaces:**

- Consumes: `begin/assets/swc.js` from Task 1 (already downloaded)
- Produces: `swc.js` committed to the repository

- [ ] **Step 1: Verify `begin/assets/swc.js` is a valid ES module**

Open `begin/assets/swc.js` in a text editor and confirm the first line starts with something like:

```js
var t=Object.defineProperty;
```

or is otherwise a minified JavaScript bundle (not an HTML error page from the CDN).

If the file is an HTML error page, the CDN URL in `versions.toml` is wrong — check `https://unpkg.com/@spectrum-web-components/bundle@0.45.4/elements.js` in a browser to find the correct path.

- [ ] **Step 2: Commit `swc.js`**

```sh
git add begin/assets/swc.js
git commit -m "chore(assets): vendor Spectrum Web Components bundle v0.45.4"
```

---

## Task 3: Create `spectrum.rs` — element bindings and component wrappers

**Files:**

- Create: `begin/src/spectrum.rs`
- Modify: `begin/src/main.rs`

**Interfaces:**

- Produces (used by Tasks 4 and 5):
  - `SpTheme(color: String, scale: String, children: Element) -> Element`
  - `SpTextfield(value: String, invalid: bool, oninput: EventHandler<FormEvent>, onfocus: EventHandler<FocusEvent>, onblur: EventHandler<FocusEvent>) -> Element`
  - `SpFieldLabel(for_: String, children: Element) -> Element`
  - `SpDivider() -> Element`
  - `SpHeading(children: Element) -> Element`

### How Dioxus custom element bindings work

In Dioxus 0.7, the RSX macro resolves snake_case identifiers as HTML element modules. A module is treated as an element if it exposes:

```rust
pub const TAG_NAME: &'static str = "html-tag-name";
pub const NAME_SPACE: Option<&'static str> = None;
```

Crucially, the Rust module name (e.g. `sp_textfield_el`) can differ from the HTML tag (e.g. `"sp-textfield"`). This is how dioxus-html maps `$element [$name:literal, $namespace]` entries: the Rust identifier uses underscores, the actual DOM tag uses hyphens or any string.

Custom attributes are passed with string literal keys: `"value": "{val}"`. Event handlers (`oninput`, `onfocus`, etc.) are resolved generically by the RSX event system and do not need to be declared in the element module.

- [ ] **Step 1: Create `begin/src/spectrum.rs`**

```rust
//! Dioxus element bindings and component wrappers for Spectrum Web Components.
//!
//! Import with `use crate::spectrum::*;` to bring both element modules and
//! component wrappers into scope. Element modules are used internally by the
//! component wrappers; callers only need the `SpXxx` component functions.

#![allow(non_snake_case)]

use dioxus::prelude::*;

// ─── Element binding modules ────────────────────────────────────────────────
// Each module exposes TAG_NAME so the RSX macro emits the correct HTML tag.
// Global HTML attributes (style, class, id, ...) are re-exported so that
// callers can pass them to the wrapper components if needed.

/// Dioxus element binding for `<sp-theme>`.
pub mod sp_theme_el {
    /// HTML tag emitted by the RSX macro.
    pub const TAG_NAME: &'static str = "sp-theme";
    /// No XML namespace (standard HTML custom element).
    pub const NAME_SPACE: Option<&'static str> = None;
    pub use dioxus::html::global_attributes::*;
}

/// Dioxus element binding for `<sp-textfield>`.
pub mod sp_textfield_el {
    /// HTML tag emitted by the RSX macro.
    pub const TAG_NAME: &'static str = "sp-textfield";
    /// No XML namespace (standard HTML custom element).
    pub const NAME_SPACE: Option<&'static str> = None;
    pub use dioxus::html::global_attributes::*;
}

/// Dioxus element binding for `<sp-field-label>`.
pub mod sp_field_label_el {
    /// HTML tag emitted by the RSX macro.
    pub const TAG_NAME: &'static str = "sp-field-label";
    /// No XML namespace (standard HTML custom element).
    pub const NAME_SPACE: Option<&'static str> = None;
    pub use dioxus::html::global_attributes::*;
}

/// Dioxus element binding for `<sp-divider>`.
pub mod sp_divider_el {
    /// HTML tag emitted by the RSX macro.
    pub const TAG_NAME: &'static str = "sp-divider";
    /// No XML namespace (standard HTML custom element).
    pub const NAME_SPACE: Option<&'static str> = None;
    pub use dioxus::html::global_attributes::*;
}

/// Dioxus element binding for `<sp-heading>`.
pub mod sp_heading_el {
    /// HTML tag emitted by the RSX macro.
    pub const TAG_NAME: &'static str = "sp-heading";
    /// No XML namespace (standard HTML custom element).
    pub const NAME_SPACE: Option<&'static str> = None;
    pub use dioxus::html::global_attributes::*;
}

// ─── Component wrappers ─────────────────────────────────────────────────────
// PascalCase functions are Dioxus components; RSX resolves them via function
// call, not as element bindings. Each wraps one SWC custom element.

/// Provides Spectrum token context for all descendant SWC components.
///
/// Must be the root ancestor of any `SpXxx` component. Maps to `<sp-theme>`.
#[component]
pub fn SpTheme(color: String, scale: String, children: Element) -> Element {
    rsx! {
        sp_theme_el {
            "color": "{color}",
            "scale": "{scale}",
            {children}
        }
    }
}

/// Single-line text input.
///
/// Maps to `<sp-textfield>`. Fires standard DOM `input`, `focus`, and `blur`
/// events. Setting `invalid` to `true` renders the SWC error state (red ring
/// and `aria-invalid`).
#[component]
pub fn SpTextfield(
    value: String,
    invalid: bool,
    oninput: EventHandler<FormEvent>,
    onfocus: EventHandler<FocusEvent>,
    onblur: EventHandler<FocusEvent>,
) -> Element {
    rsx! {
        sp_textfield_el {
            "value": "{value}",
            // Boolean attribute: omit entirely when false; presence = invalid.
            "invalid": if invalid { "true" },
            oninput: move |e| oninput.call(e),
            onfocus: move |e| onfocus.call(e),
            onblur: move |e| onblur.call(e),
        }
    }
}

/// Label associated with a form control.
///
/// Maps to `<sp-field-label>`. The `for_` prop sets the `for` HTML attribute
/// linking the label to an input by id.
#[component]
pub fn SpFieldLabel(for_: String, children: Element) -> Element {
    rsx! {
        sp_field_label_el {
            "for": "{for_}",
            {children}
        }
    }
}

/// Horizontal visual separator.
///
/// Maps to `<sp-divider>` with `size="s"` (small).
#[component]
pub fn SpDivider() -> Element {
    rsx! {
        sp_divider_el {
            "size": "s",
        }
    }
}

/// Section heading.
///
/// Maps to `<sp-heading>`.
#[component]
pub fn SpHeading(children: Element) -> Element {
    rsx! {
        sp_heading_el {
            {children}
        }
    }
}
```

- [ ] **Step 2: Add `mod spectrum;` to `begin/src/main.rs`**

```rust
//! Entry point for the `begin` property model development environment.
mod app;
mod bridge;
mod graph_view;
mod inspector;
mod spectrum;

fn main() {
    dioxus::launch(app::App);
}
```

- [ ] **Step 3: Build to verify**

```sh
cargo build -p begin --no-default-features
```

Expected: compiles without errors. If the RSX macro cannot resolve `sp_theme_el` (or similar) as an element binding, the error will say something like "expected function, found module" or "no `TAG_NAME` in scope". In that case: check that the `sp_theme_el` module is in scope in the file that uses it (via `use crate::spectrum::*;`), and that the module name in RSX exactly matches the module identifier.

- [ ] **Step 4: Commit**

```sh
git add begin/src/spectrum.rs begin/src/main.rs
git commit -m "feat(begin): add Spectrum Web Components element bindings and wrappers"
```

---

## Task 4: Update `app.rs` — load SWC bundle, wrap UI in `SpTheme`

**Files:**

- Modify: `begin/src/app.rs`

**Interfaces:**

- Consumes: `SpTheme` from `crate::spectrum`
- Produces: app wrapped in `<sp-theme color="light" scale="medium">`

- [ ] **Step 1: Replace `begin/src/app.rs`**

```rust
//! Root [`App`] component and demo sheet factory.

use dioxus::prelude::*;
use property_model::{Method, Sheet};

use crate::bridge::{Labels, to_graph_data};
use crate::graph_view::GraphView;
use crate::inspector::Inspector;
use crate::spectrum::SpTheme;

/// Builds the `a × b = c` demo sheet with three bidirectional methods.
///
/// Cells are added in order `c, a, b` so that `a` and `b` have higher initial
/// strength than `c`. The planner therefore treats `a` and `b` as sources and
/// derives `c`, which is the intended default direction. `propagate()` is called
/// once to compute the initial value of `c`.
pub fn make_demo_sheet() -> (Sheet, Labels) {
    let mut sheet = Sheet::new();
    let mut labels = Labels::new();

    // c added first → lowest strength (output by default).
    let c = sheet.add_cell(0.0_f64);
    // a and b added later → higher strength (sources by default).
    let a = sheet.add_cell(2.0_f64);
    let b = sheet.add_cell(3.0_f64);

    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok(x * y)),
            Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok(y / x)),
            Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok(y / x)),
        ])
        .unwrap();

    let d = sheet.add_cell(4.0_f64);
    let e = sheet.add_cell(5.0_f64);

    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([d, e], c, |x: &f64, y: &f64| Ok(x * y)),
            Method::from_fn_2_1([c, e], d, |x: &f64, y: &f64| Ok(x / y)),
            Method::from_fn_2_1([c, d], e, |x: &f64, y: &f64| Ok(x / y)),
        ])
        .unwrap();

    // Compute c = a × b = 6 on startup; clear changed so c does not pulse immediately.
    sheet.propagate().unwrap();
    sheet.clear_changed();

    labels.add_cell::<f64>(a, "a");
    labels.add_cell::<f64>(b, "b");
    labels.add_cell::<f64>(c, "c");
    labels.add_cell::<f64>(d, "d");
    labels.add_cell::<f64>(e, "e");

    (sheet, labels)
}

/// Root component: Spectrum theme wrapper, two-panel layout with the D3 graph on the
/// left and the Inspector on the right.
#[component]
pub fn App() -> Element {
    let (initial_sheet, initial_labels) = make_demo_sheet();
    let sheet = use_signal(|| initial_sheet);
    let labels = use_signal(|| initial_labels);

    let graph_data = use_memo(move || to_graph_data(&sheet.read(), &labels.read()));

    rsx! {
        document::Link { rel: "stylesheet", href: asset!("/assets/graph.css") }
        document::Script { src: asset!("/assets/d3.v7.min.js") }
        document::Script { src: asset!("/assets/graph.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc.js") }

        SpTheme {
            color: "light".to_string(),
            scale: "medium".to_string(),
            div {
                style: "position: fixed; inset: 0; display: flex; overflow: hidden;",
                GraphView { data: graph_data }
                Inspector { sheet, labels }
            }
        }
    }
}
```

- [ ] **Step 2: Build to verify**

```sh
cargo build -p begin --no-default-features
```

Expected: compiles without errors.

- [ ] **Step 3: Run the app and verify the theme wrapper is present**

```sh
dx serve --platform desktop
```

Open the app. It should look unchanged visually at this point (SWC is loaded but the inspector still uses plain HTML). Open the browser DevTools (right-click → Inspect in the WebView, or use Ctrl+Shift+I if available) and confirm `<sp-theme color="light" scale="medium">` wraps the app root in the DOM.

- [ ] **Step 4: Commit**

```sh
git add begin/src/app.rs
git commit -m "feat(begin): load SWC bundle and wrap app in sp-theme"
```

---

## Task 5: Update `inspector.rs` — replace inline-CSS elements with SWC components

**Files:**

- Modify: `begin/src/inspector.rs`

**Interfaces:**

- Consumes: `SpTextfield`, `SpFieldLabel`, `SpDivider`, `SpHeading` from `crate::spectrum`
- Produces: Inspector sidebar using SWC form components

- [ ] **Step 1: Replace `begin/src/inspector.rs`**

```rust
//! [`Inspector`] — sidebar listing all cells with their current values and a write form.

use dioxus::prelude::*;
use property_model::{CellId, Sheet};

use crate::bridge::Labels;
use crate::spectrum::{SpDivider, SpFieldLabel, SpHeading, SpTextfield};

/// Sidebar panel showing all cells with labels, current values, and text inputs for writing.
///
/// Editing an input field immediately writes the parsed value to the sheet and propagates
/// constraints. If propagation fails (for example, division by zero), `SpTextfield` renders
/// in its invalid state until the user blurs. The input is not reset while the field is
/// focused; it syncs back to the computed value on blur, keeping non-edited cells up to date.
#[component]
pub fn Inspector(sheet: Signal<Sheet>, labels: Signal<Labels>) -> Element {
    let ids: Vec<CellId> = labels.read().cells.keys().copied().collect();

    rsx! {
        div {
            style: "width: 260px; min-width: 260px; height: 100%; overflow-y: auto; padding: 12px; box-sizing: border-box;",
            SpHeading { "Cells" }
            SpDivider {}
            for id in ids {
                CellRow { key: "{id:?}", id, sheet, labels }
            }
        }
    }
}

#[component]
fn CellRow(id: CellId, sheet: Signal<Sheet>, labels: Signal<Labels>) -> Element {
    let label = use_memo(move || {
        labels
            .read()
            .cells
            .get(&id)
            .map(|m| m.label.clone())
            .unwrap_or_default()
    });

    let value = use_memo(move || {
        let s = sheet.read();
        let l = labels.read();
        l.cells
            .get(&id)
            .map(|m| (m.display)(&s))
            .unwrap_or_default()
    });

    let mut input = use_signal(|| value.peek().clone());
    let mut is_focused = use_signal(|| false);
    let mut has_error = use_signal(|| false);

    // Sync input to the computed value whenever it changes, but not while the user
    // is actively editing — that would interrupt mid-value typing (e.g. "1." → "1").
    use_effect(move || {
        let v = value.read().clone();
        if !*is_focused.read() {
            input.set(v);
        }
    });

    let field_id = format!("cell-{id:?}");

    rsx! {
        div {
            style: "margin-bottom: 8px;",
            SpFieldLabel { for_: field_id.clone(), "{label}" }
            SpTextfield {
                value: input.read().clone(),
                invalid: *has_error.read(),
                oninput: move |e: FormEvent| {
                    let s = e.value();
                    input.set(s.clone());
                    let mut sheet_w = sheet.write();
                    let labels_r = labels.read();
                    if let Some(meta) = labels_r.cells.get(&id)
                        && (meta.write_str)(&mut sheet_w, &s).is_ok()
                    {
                        let result = if sheet_w.is_source(id) {
                            sheet_w.propagate_without_replan()
                        } else {
                            sheet_w.propagate()
                        };
                        has_error.set(result.is_err());
                    }
                },
                onfocus: move |_| is_focused.set(true),
                onblur: move |_| {
                    is_focused.set(false);
                    has_error.set(false);
                },
            }
        }
        SpDivider {}
    }
}
```

- [ ] **Step 2: Build to verify**

```sh
cargo build -p begin --no-default-features
```

Expected: compiles without errors or warnings. If clippy fails, run:

```sh
cargo clippy -p begin --no-default-features -- -D warnings
```

and fix any reported issues before continuing.

- [ ] **Step 3: Run and visually verify the inspector**

```sh
dx serve --platform desktop
```

Verify:
1. The Inspector sidebar displays `sp-field-label` elements for cell names.
2. Each cell has an `sp-textfield` input pre-filled with the current value.
3. Typing a new numeric value in any field updates the other connected cells immediately.
4. Typing a non-numeric value (e.g. `"abc"`) causes the `sp-textfield` to render its invalid (red) state.
5. Clicking away (blur) clears the error state and restores the last valid computed value.
6. The D3 graph continues to update correctly as values change.

- [ ] **Step 4: Run the full check suite**

```sh
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --exclude begin -- -D warnings
cargo clippy -p begin --no-default-features -- -D warnings
```

All commands must pass without errors.

- [ ] **Step 5: Commit**

```sh
git add begin/src/inspector.rs
git commit -m "feat(begin): replace inline-CSS inspector elements with Spectrum Web Components"
```
