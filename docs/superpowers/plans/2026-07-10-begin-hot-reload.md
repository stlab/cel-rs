# Begin Hot Reload Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Retire `begin`'s in-app source/error panel; move the demo pm-lang source to a standalone file that hot-reloads into the running desktop app via Dioxus's devserver connection, and print all diagnostics to stderr as ANSI-colored rustc-style output instead of holding them in app state.

**Architecture:** `begin/src/source_panel.rs` (UI component + `build_sheet`) is replaced by `begin/src/demo_source.rs` (no UI — just source loading, sheet building, and hot-reload wiring). The demo source moves from a Rust string constant to `begin/assets/demo.pm`. `App` drops its bottom docked panel and shared error signals; `Inspector` drops its shared error props and prints failures directly. Hot reload connects to the same `dx serve` devserver websocket the CLI already uses (`dioxus_devtools::connect`), filtering for our specific asset path — desktop only.

**Tech Stack:** Dioxus 0.7.9 (`asset!`/`Asset`, `dioxus::asset_resolver::asset_path`, `dioxus_devtools::connect`), `annotate-snippets` 0.12 (`Renderer::styled()`), `futures-channel`/`futures-util` 0.3 (async channel bridging a background OS thread into a Dioxus-spawned task).

## Global Constraints

- `cargo fmt --all` before every commit (enforced by pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` and `cargo clippy -p begin --no-default-features --all-targets -- -D warnings` must both pass. The `--no-default-features` invocation means any new desktop-only code must be `#[cfg(feature = "desktop")]`-gated so it compiles out cleanly (verified during planning — see Task 2).
- Never commit directly to `main`; this work happens on the current worktree branch.
- Doc comments follow the project's contract style (Summary / Preconditions / Postconditions / Complexity) — see `CLAUDE.md`.

## Verified assumptions (done during planning, not to be re-verified by the implementer)

The following were checked against the real vendored Dioxus 0.7.9 source and confirmed by compiling scratch code in this repo (not just inferred from docs):

- `dioxus::asset_resolver::asset_path(&Asset) -> Result<PathBuf, AssetPathError>` and `dioxus_devtools::connect(callback)` / `dioxus_devtools::{DevserverMsg, HotReloadMsg}` are all reachable via `use dioxus::prelude::*;` alone — **no new Cargo.toml dependency is needed for either the `asset` or `devtools` Dioxus features**, since both are part of `dioxus`'s own default feature set (`begin`'s `dioxus = { version = "0.7.9", features = [] }` does not disable Dioxus's defaults).
- Referencing an individual file via `asset!("/assets/demo.pm")` (not a folder) is required for `dx serve`'s watcher to report it in `HotReloadMsg.assets`.
- Swapping `Renderer::plain()` → `Renderer::styled()` does not break the existing substring-matching assertions in `bridge.rs`'s and `source_panel.rs`'s tests (ran them with the swap applied — both passed).
- `#[cfg(feature = "desktop")]` on `spawn_hot_reload` (and the `use_hook` block that calls it) compiles cleanly under both `cargo check -p begin` (default/desktop features) and `cargo check -p begin --no-default-features` — confirmed with a scratch module, not left to guesswork. `dioxus_devtools::connect` itself already no-ops when not running under `dx serve` (checked in its source: it returns immediately if `dioxus_cli_config::devserver_ws_endpoint()` is `None`), so **no additional `debug_assertions` gate is needed** — this refines the design doc's phrasing ("gated on debug_assertions") with a simpler, equally-safe mechanism already provided by `connect` itself.
- The channel-bridging pattern (`futures_channel::mpsc::unbounded` sender used from the background thread spawned inside `connect`, receiver consumed via `futures_util::StreamExt::next()` inside a `spawn`ed Dioxus task) type-checks against real `Signal<Sheet>`/`Signal<Labels>` mutation from within that task.
- `begin` has no library target — it's a binary crate (`src/main.rs` + modules); tests run via `cargo test -p begin` (not `--lib`).

---

### Task 1: Move demo source to a file; delete the SourcePanel; route all errors to stderr

**Files:**
- Create: `begin/assets/demo.pm`
- Create: `begin/src/demo_source.rs`
- Delete: `begin/src/source_panel.rs`
- Modify: `begin/src/main.rs`
- Modify: `begin/src/app.rs`
- Modify: `begin/src/inspector.rs`
- Modify: `begin/src/bridge.rs`

**Interfaces:**
- Produces (used by Task 2): `demo_source::DEMO_ASSET: Asset` (private to the module — Task 2 lives in the same file so no visibility change needed), `demo_source::load_demo_source() -> String`, `demo_source::build_sheet(source: &str) -> BuildOutcome`, `demo_source::DEMO_SOURCE_TEXT: &str` (`pub(crate)`).
- Consumes: `bridge::format_property_model_error(e: &property_model::Error, source: &str) -> String` (existing, only its internal renderer changes).

- [ ] **Step 1: Create `begin/assets/demo.pm`**

```
sheet demo {
    cell a: f64 = 2.0;
    cell b: f64 = 3.0;
    cell c: f64;
    cell d: f64 = 4.0;
    cell e: f64 = 5.0;
    cell f: f64;
    cell g: f64;
    cell p: i32 = 0;

    relationship {
        method [a, b] -> [c] { a * b }
        method [b, c] -> [a] { c / b }
        method [a, c] -> [b] { c / a }
    }

    relationship {
        method [d, e] -> [f] { d * e }
        method [e, f] -> [d] { f / e }
        method [d, f] -> [e] { f / d }
    }

    conditional p {
        0i32 => {
            method [f] -> [c] { f }
            method [c] -> [f] { c }
        }
        1i32 => {
            method [f] -> [c] { f * 2.0 }
            method [c] -> [f] { c / 2.0 }
        }
    }

    conditional p {
        1i32 => {
            method [c] -> [g] { c * 10.0 }
        }
    }
}
```

This is byte-for-byte the sheet body from `DEMO_SOURCE` in the current `app.rs` (just without the enclosing Rust raw-string literal).

- [ ] **Step 2: Create `begin/src/demo_source.rs`**

```rust
//! Loads the demo pm-lang source from `begin/assets/demo.pm` and builds a
//! [`Sheet`]/[`Labels`] pair from it.
//!
//! Two independent bidirectional constraint systems (`a × b = c` and `d × e = f`)
//! are linked by two conditionals on `p`:
//!
//! - `p = 0`: the relationship `c = f` (bidirectional) becomes active.
//! - `p = 1`: the relationship `c = f × 2` (bidirectional) becomes active, and a
//!   single-method relationship `g = c × 10` also becomes active — `g` is *forced*
//!   while this branch is active (see [`property_model::Sheet::is_forced`]), so its
//!   Inspector field is disabled and it is highlighted in the graph.
//! - Any other `p`: the two systems are independent and `g` is not forced.
//!
//! `g`'s relationship is declared in its own `conditional p { .. }` block rather than
//! folded into the first: pm-lang groups every method in one branch into a single
//! relationship, and a relationship's forced outputs are the *intersection* of its
//! methods' pure outputs — mixing `[c] -> [g]` in with the `c`/`f` methods would make
//! that intersection empty, forcing nothing. Two conditionals sharing the same match
//! cell compose independently, so this is a distinct relationship gated on the same
//! `p == 1` condition. This also means the graph renders two diamond nodes for `p`.

use annotate_snippets::Renderer;
use dioxus::prelude::*;
use pm_lang::{PmParser, TypeRegistry};
use property_model::Sheet;

use crate::bridge::{Labels, format_property_model_error, labels_from_cell_names};

/// The demo pm-lang source file, referenced individually (not via a folder) so
/// `dx serve`'s file watcher reports changes to it in hot-reload messages.
static DEMO_ASSET: Asset = asset!("/assets/demo.pm");

/// Compile-time snapshot of the demo source.
///
/// Used as the non-desktop [`load_demo_source`] fallback and as the fixture for
/// unit tests, both of which need a value that doesn't depend on desktop asset
/// bundling being available.
pub(crate) const DEMO_SOURCE_TEXT: &str = include_str!("../assets/demo.pm");

/// The result of parsing and building a sheet from pm-lang source.
///
/// `sheet_labels` is `None` only on parse failure. A successful parse that
/// then fails to propagate still returns the built sheet and labels alongside
/// the formatted error, matching how the Inspector already tolerates
/// propagate failures during cell edits.
pub struct BuildOutcome {
    /// The built sheet and its UI labels, if parsing succeeded.
    pub sheet_labels: Option<(Sheet, Labels)>,
    /// A formatted rustc-style diagnostic, if parsing or propagation failed.
    pub error: Option<String>,
}

/// Parses `source` as pm-lang, builds a `Sheet` and `Labels`, and propagates
/// once so initial derived values are populated.
///
/// - Complexity: O(n) in the length of `source` plus the cost of one `propagate()`.
pub fn build_sheet(source: &str) -> BuildOutcome {
    let mut parser = PmParser::new(TypeRegistry::new(), cel_parser::OpLookup::new());
    let mut parsed = match parser.parse_str(source) {
        Ok(p) => p,
        Err(e) => {
            let msg = e.format_rustc_style(source, "<pm-lang source>", 1, &Renderer::styled());
            return BuildOutcome {
                sheet_labels: None,
                error: Some(msg),
            };
        }
    };
    let labels = labels_from_cell_names(&parsed.cell_names);
    match parsed.propagate() {
        Ok(()) => {
            parsed.clear_changed();
            BuildOutcome {
                sheet_labels: Some((parsed.sheet, labels)),
                error: None,
            }
        }
        Err(e) => {
            let msg = format_property_model_error(&e, source);
            BuildOutcome {
                sheet_labels: Some((parsed.sheet, labels)),
                error: Some(msg),
            }
        }
    }
}

/// Reads the demo source, resolving [`DEMO_ASSET`] to a filesystem path on desktop.
///
/// - Precondition: the app is launched via `dx serve`/`dx build` (true for every
///   documented way of running `begin`), so `DEMO_ASSET` resolves to a real path.
#[cfg(feature = "desktop")]
pub fn load_demo_source() -> String {
    let path = dioxus::asset_resolver::asset_path(&DEMO_ASSET)
        .expect("demo.pm must resolve to a filesystem path on desktop");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

/// Non-desktop fallback: the compile-time snapshot, with no live reload.
#[cfg(not(feature = "desktop"))]
pub fn load_demo_source() -> String {
    DEMO_SOURCE_TEXT.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SOURCE: &str = r#"
        sheet s {
            cell a: f64 = 2.0;
            cell b: f64 = 3.0;
            cell c: f64;
            relationship {
                method [a, b] -> [c] { a * b }
                method [b, c] -> [a] { c / b }
                method [a, c] -> [b] { c / a }
            }
        }
    "#;

    #[test]
    fn build_sheet_valid_source_succeeds_with_no_error() {
        let outcome = build_sheet(VALID_SOURCE);
        assert!(outcome.sheet_labels.is_some());
        assert!(outcome.error.is_none());
    }

    #[test]
    fn build_sheet_parse_error_has_no_sheet_and_formatted_message() {
        let outcome = build_sheet("sheet s { cell x }");
        assert!(outcome.sheet_labels.is_none());
        let msg = outcome.error.expect("expected a parse error message");
        assert!(msg.contains("error"), "{msg}");
    }

    #[test]
    fn build_sheet_runtime_error_still_returns_sheet_and_message() {
        let source = "sheet s { cell x: i32 = 0; cell y: i32; relationship { method [x] -> [y] { 10i32 / x } } }";
        let outcome = build_sheet(source);
        assert!(
            outcome.sheet_labels.is_some(),
            "sheet should still be built after a propagate error"
        );
        assert!(outcome.error.is_some());
    }

    #[test]
    fn demo_source_text_parses_successfully() {
        let outcome = build_sheet(DEMO_SOURCE_TEXT);
        assert!(outcome.sheet_labels.is_some());
    }
}
```

- [ ] **Step 3: Delete `begin/src/source_panel.rs`**

```bash
git rm begin/src/source_panel.rs
```

- [ ] **Step 4: Update `begin/src/main.rs`'s module list**

Change:
```rust
mod source_panel;
```
to:
```rust
mod demo_source;
```
(keep alphabetical position consistent with the other `mod` lines — insert where `source_panel` was, i.e. before `mod spectrum;`, since `demo_source` alphabetically comes right after `bridge`; place it there instead: the full list should read `mod app; mod bridge; mod demo_source; mod graph_view; mod inspector; mod spectrum;`).

- [ ] **Step 5: Rewrite `begin/src/app.rs`**

```rust
//! Root [`App`] component.

use dioxus::prelude::*;

use crate::bridge::to_graph_data;
use crate::demo_source::{build_sheet, load_demo_source};
use crate::graph_view::GraphView;
use crate::inspector::Inspector;
use crate::spectrum::SpTheme;

/// Root component: Spectrum theme wrapper with the graph and Inspector filling the
/// viewport. The demo pm-lang source lives in `begin/assets/demo.pm` — edit it and
/// (on desktop, under `dx serve`) it hot-reloads into this running app.
#[component]
pub fn App() -> Element {
    let initial_source = load_demo_source();
    let initial = build_sheet(&initial_source);
    let (initial_sheet, initial_labels) = initial
        .sheet_labels
        .expect("demo.pm must parse successfully");
    let sheet = use_signal(|| initial_sheet);
    let labels = use_signal(|| initial_labels);
    let active_source = use_signal(|| initial_source);

    let graph_data = use_memo(move || to_graph_data(&sheet.read(), &labels.read()));

    rsx! {
        document::Link { rel: "icon", r#type: "image/x-icon", href: asset!("/assets/favicon.ico") }
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
                Inspector { sheet, labels, active_source }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo_source::DEMO_SOURCE_TEXT;

    #[test]
    fn demo_source_g_not_forced_when_p_is_zero() {
        let outcome = build_sheet(DEMO_SOURCE_TEXT);
        let (sheet, labels) = outcome.sheet_labels.expect("demo.pm must build");
        let g_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("g"))
            .unwrap();
        assert!(!sheet.is_forced(g_id), "g should not be forced when p == 0");
    }

    #[test]
    fn demo_source_g_forced_when_p_is_one() {
        let outcome = build_sheet(DEMO_SOURCE_TEXT);
        let (mut sheet, labels) = outcome.sheet_labels.expect("demo.pm must build");
        let p_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("p"))
            .unwrap();
        let g_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("g"))
            .unwrap();

        sheet.write(p_id, 1_i32).unwrap();
        sheet.propagate().unwrap();

        assert!(sheet.is_forced(g_id), "g should be forced when p == 1");
    }

    #[test]
    fn demo_source_g_unforced_again_after_p_returns_to_zero() {
        let outcome = build_sheet(DEMO_SOURCE_TEXT);
        let (mut sheet, labels) = outcome.sheet_labels.expect("demo.pm must build");
        let p_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("p"))
            .unwrap();
        let g_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("g"))
            .unwrap();

        sheet.write(p_id, 1_i32).unwrap();
        sheet.propagate().unwrap();
        sheet.write(p_id, 0_i32).unwrap();
        sheet.propagate().unwrap();

        assert!(
            !sheet.is_forced(g_id),
            "g should not be forced once p == 0 again"
        );
    }
}
```

- [ ] **Step 6: Rewrite `begin/src/inspector.rs`**

```rust
//! [`Inspector`] — sidebar listing all cells with their current values and a write form.

use dioxus::prelude::*;
use property_model::{CellId, Sheet};

use crate::bridge::{Labels, format_property_model_error};
use crate::spectrum::{SpDivider, SpFieldLabel, SpHeading, SpTextfield};

/// Sidebar panel showing all cells with labels, current values, and text inputs for writing.
///
/// Editing an input field immediately writes the parsed value to the sheet and propagates
/// constraints. If parsing or propagation fails (for example, non-numeric input or division
/// by zero), `SpTextfield` renders in its invalid state until the user blurs, and the
/// formatted diagnostic is printed to stderr. The input is not reset while the field is
/// focused; it syncs back to the computed value on blur, keeping non-edited cells up to date.
#[component]
pub fn Inspector(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<String>,
) -> Element {
    let ids: Vec<CellId> = labels.read().cells.keys().copied().collect();

    rsx! {
        div {
            style: "width: 260px; min-width: 260px; height: 100%; overflow-y: auto; padding: 12px; box-sizing: border-box;",
            SpHeading { "Cells" }
            SpDivider {}
            for id in ids {
                CellRow { key: "{id:?}", id, sheet, labels, active_source }
            }
        }
    }
}

#[component]
fn CellRow(
    id: CellId,
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<String>,
) -> Element {
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

    let forced = use_memo(move || sheet.read().is_forced(id));

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
                id: field_id,
                value: input.read().clone(),
                invalid: *has_error.read(),
                disabled: *forced.read(),
                // Dioxus's event serializer only reads event.target.value for
                // HTMLInputElement — custom elements (sp-textfield) always give "".
                // Use dioxus.send() in JS and eval.recv() to read the live value.
                oninput: move |_: FormEvent| {
                    spawn(async move {
                        let mut eval = document::eval(&format!(
                            r#"dioxus.send(document.getElementById("cell-{id:?}").value)"#
                        ));
                        let Ok(val) = eval.recv::<String>().await else { return; };
                        // Discard the result if the user blurred while the round-trip was
                        // in flight; blur already cleared the error and use_effect will
                        // restore the last valid computed value.
                        if !*is_focused.read() {
                            return;
                        }
                        input.set(val.clone());
                        let mut sheet_w = sheet.write();
                        let labels_r = labels.read();
                        let Some(meta) = labels_r.cells.get(&id) else { return; };
                        let write_result = (meta.write_str)(&mut sheet_w, &val);
                        drop(labels_r);
                        let propagate_result = match write_result {
                            Ok(()) => {
                                // A conditional match cell changes the active constraint set
                                // when written, which invalidates the plan even if the cell
                                // is a source — so we must always replan for match cells.
                                let is_match_cell = sheet_w
                                    .conditionals()
                                    .any(|cid| sheet_w.conditional_match_cell(cid) == Some(id));
                                if sheet_w.is_source(id) && !is_match_cell {
                                    sheet_w.propagate_without_replan()
                                } else {
                                    sheet_w.propagate()
                                }
                            }
                            Err(e) => Err(e),
                        };
                        match propagate_result {
                            Ok(()) => {
                                has_error.set(false);
                            }
                            Err(e) => {
                                has_error.set(true);
                                let source = active_source.read().clone();
                                eprintln!("{}", format_property_model_error(&e, &source));
                            }
                        }
                    });
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

- [ ] **Step 7: Update `begin/src/bridge.rs`**

Change the one `Renderer::plain()` call inside `format_property_model_error` to `Renderer::styled()`:

```rust
/// Formats an [`Error`] as a rustc-style diagnostic when possible.
///
/// `Error::MethodFailed` wraps an `anyhow::Error` raised by a compiled method
/// body; when that error carries a `SpanContext` (attached automatically by
/// cel-parser's `span-diagnostics` feature for built-in arithmetic ops) this
/// renders a full caret diagnostic against `source`, ANSI-colored for a
/// terminal. All other variants have no source span and fall back to their
/// `Display` message.
pub fn format_property_model_error(e: &Error, source: &str) -> String {
    match e {
        Error::MethodFailed(inner) => {
            inner.format_rustc_style(source, "<pm-lang source>", 1, &Renderer::styled())
        }
        other => other.to_string(),
    }
}
```

(Only the doc comment and the `Renderer::plain()` → `Renderer::styled()` call change; the rest of `bridge.rs` — `Labels`, `to_graph_data`, `GraphData`, etc. — is untouched.)

- [ ] **Step 8: Run the full test suite**

Run: `cargo test -p begin`
Expected: all tests pass, including the moved/renamed ones (`demo_source::tests::*`, `app::tests::demo_source_g_*`, `bridge::tests::format_property_model_error_*`).

- [ ] **Step 9: Run build and lint checks**

Run:
```bash
cargo build --workspace
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
```
Expected: zero warnings from both. (The workspace-wide `clippy --exclude begin` command is unaffected by this task since it excludes `begin`; still worth a sanity build.)

- [ ] **Step 10: Format and commit**

```bash
cargo fmt --all
git add begin/assets/demo.pm begin/src/demo_source.rs begin/src/main.rs begin/src/app.rs begin/src/inspector.rs begin/src/bridge.rs
git commit -m "$(cat <<'EOF'
feat(begin): retire SourcePanel, move demo source to a file, print errors to stderr

The in-app source/error panel was always scaffolding (see VISION.md); pm-lang
edits now happen in demo.pm directly and diagnostics go to the console instead
of a shared UI signal.
EOF
)"
```

---

### Task 2: Hot-reload `demo.pm` on desktop via the `dx serve` devserver connection

**Files:**
- Modify: `begin/Cargo.toml`
- Modify: `begin/src/demo_source.rs`
- Modify: `begin/src/app.rs`

**Interfaces:**
- Consumes: `demo_source::{DEMO_ASSET, load_demo_source, build_sheet}` (from Task 1, `DEMO_ASSET` stays private to the module since `spawn_hot_reload` is added to the same file).
- Produces: `demo_source::spawn_hot_reload(on_change: impl FnMut() + Send + 'static)` (desktop-only), used by `App`.

- [ ] **Step 1: Add channel dependencies to `begin/Cargo.toml`**

Add these two lines directly after the `dioxus = { ... }` line under `[dependencies]`:

```toml
futures-channel = "0.3"
futures-util = "0.3"
```

Run: `cargo check -p begin`
Expected: succeeds, `Cargo.lock` picks up the already-transitively-resolved `0.3.32` versions (no new major-version duplication).

- [ ] **Step 2: Write the failing test for the asset-matching predicate**

Append to `begin/src/demo_source.rs` (below the existing `#[cfg(test)] mod tests` block, as a new module):

```rust
#[cfg(test)]
mod hot_reload_tests {
    use super::*;
    use dioxus_devtools::HotReloadMsg;
    use std::path::PathBuf;

    #[test]
    fn hot_reload_targets_demo_true_when_assets_contains_path() {
        let demo_path = PathBuf::from("/x/demo.pm");
        let msg = HotReloadMsg {
            assets: vec![demo_path.clone()],
            ..Default::default()
        };
        assert!(hot_reload_targets_demo(&msg, &demo_path));
    }

    #[test]
    fn hot_reload_targets_demo_false_when_assets_empty() {
        let demo_path = PathBuf::from("/x/demo.pm");
        let msg = HotReloadMsg::default();
        assert!(!hot_reload_targets_demo(&msg, &demo_path));
    }

    #[test]
    fn hot_reload_targets_demo_false_for_unrelated_asset() {
        let demo_path = PathBuf::from("/x/demo.pm");
        let msg = HotReloadMsg {
            assets: vec![PathBuf::from("/x/graph.css")],
            ..Default::default()
        };
        assert!(!hot_reload_targets_demo(&msg, &demo_path));
    }
}
```

- [ ] **Step 3: Run the test to verify it fails to compile (function doesn't exist yet)**

Run: `cargo test -p begin hot_reload_targets_demo`
Expected: compile error, `cannot find function 'hot_reload_targets_demo' in this scope`.

- [ ] **Step 4: Implement `hot_reload_targets_demo` and `spawn_hot_reload`**

Add to `begin/src/demo_source.rs`, above the `#[cfg(test)]` blocks:

```rust
use dioxus_devtools::HotReloadMsg;

/// True if `msg` reports a change to the file at `demo_path`.
fn hot_reload_targets_demo(msg: &HotReloadMsg, demo_path: &std::path::Path) -> bool {
    msg.assets.iter().any(|p| p == demo_path)
}

/// Connects to the `dx serve` devserver and calls `on_change` whenever `demo.pm`
/// changes on disk. A no-op if not running under `dx serve` (`dioxus_devtools::connect`
/// itself returns immediately in that case) or if `DEMO_ASSET` can't be resolved to a
/// filesystem path.
///
/// - Complexity: spawns one background OS thread for the life of the process.
#[cfg(feature = "desktop")]
pub fn spawn_hot_reload(mut on_change: impl FnMut() + Send + 'static) {
    let Ok(demo_path) = dioxus::asset_resolver::asset_path(&DEMO_ASSET) else {
        return;
    };
    dioxus_devtools::connect(move |msg| {
        if let dioxus_devtools::DevserverMsg::HotReload(hot_reload) = msg {
            if hot_reload_targets_demo(&hot_reload, &demo_path) {
                on_change();
            }
        }
    });
}
```

(`use dioxus_devtools::HotReloadMsg;` at module scope — the type is used unconditionally by `hot_reload_targets_demo` and its tests, so it must not be behind `#[cfg(feature = "desktop")]`, otherwise `cargo clippy -p begin --no-default-features` would warn on the unused import. `DevserverMsg` is only used inside the `desktop`-gated function, so it's referenced by its full path (`dioxus_devtools::DevserverMsg::HotReload`) there instead of a top-level `use`, for the same reason.)

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p begin hot_reload_targets_demo`
Expected: 3 tests pass (`hot_reload_targets_demo_true_when_assets_contains_path`, `hot_reload_targets_demo_false_when_assets_empty`, `hot_reload_targets_demo_false_for_unrelated_asset`).

- [ ] **Step 6: Wire `spawn_hot_reload` into `App`**

In `begin/src/app.rs`, add this block inside `App`'s body, after the `let active_source = use_signal(...)` line and before `let graph_data = use_memo(...)`:

```rust
    #[cfg(feature = "desktop")]
    {
        let mut sheet = sheet;
        let mut labels = labels;
        let mut active_source = active_source;
        use_hook(move || {
            let (tx, mut rx) = futures_channel::mpsc::unbounded::<()>();
            crate::demo_source::spawn_hot_reload(move || {
                let _ = tx.unbounded_send(());
            });
            spawn(async move {
                use futures_util::StreamExt;
                while rx.next().await.is_some() {
                    let source = crate::demo_source::load_demo_source();
                    let outcome = crate::demo_source::build_sheet(&source);
                    if let Some((new_sheet, new_labels)) = outcome.sheet_labels {
                        sheet.set(new_sheet);
                        labels.set(new_labels);
                        active_source.set(source);
                    }
                    if let Some(msg) = outcome.error {
                        eprintln!("{msg}");
                    }
                }
            });
        });
    }
```

- [ ] **Step 7: Update `App`'s doc comment**

Change the `#[component]` doc comment on `App` from "edit it and (on desktop, under `dx serve`) it hot-reloads into this running app" to a more specific description now that the mechanism exists:

```rust
/// Root component: Spectrum theme wrapper with the graph and Inspector filling the
/// viewport. The demo pm-lang source lives in `begin/assets/demo.pm` — on desktop,
/// editing it while running under `dx serve` hot-reloads the sheet into this running
/// app via [`crate::demo_source::spawn_hot_reload`], exactly as if the old Apply
/// button had been pressed.
```

- [ ] **Step 8: Run the full test suite and lint checks**

Run:
```bash
cargo test -p begin
cargo build --workspace
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
```
Expected: all tests pass; zero warnings from both build and clippy.

- [ ] **Step 9: Manual verification via `dx serve`**

Run (from `begin/`): `dx serve --platform desktop`

With the app running:
1. Confirm the app starts and shows the demo graph (same as before).
2. Edit `begin/assets/demo.pm` (e.g. change `cell a: f64 = 2.0;` to `cell a: f64 = 20.0;`) and save.
3. Confirm the running app's graph updates to reflect the new value **without restarting the app or touching the window** — this is the hot-reload path working end to end.
4. Introduce a deliberate parse error in `demo.pm` (e.g. delete a closing brace), save, and confirm: the terminal running `dx serve` prints a colored rustc-style diagnostic to stderr, and the app keeps showing the last good graph (no crash, no stale in-app panel).
5. Fix the error, save again, and confirm the graph updates again.
6. In the Inspector sidebar, trigger a runtime error (e.g. type a value that causes a divide-by-zero) and confirm the diagnostic prints to the same terminal, with the field showing its red/invalid state.

This step has no automated equivalent — it's the acceptance check for this task's core deliverable, per the `verify` skill.

- [ ] **Step 10: Format and commit**

```bash
cargo fmt --all
git add begin/Cargo.toml begin/Cargo.lock begin/src/demo_source.rs begin/src/app.rs Cargo.lock
git commit -m "$(cat <<'EOF'
feat(begin): hot-reload demo.pm on desktop via the dx serve devserver connection

Connects to the same devserver websocket dx serve already uses
(dioxus_devtools::connect) and filters HotReloadMsg.assets for demo.pm,
so editing it in VSCode rebuilds the running sheet without an Apply button.
EOF
)"
```
