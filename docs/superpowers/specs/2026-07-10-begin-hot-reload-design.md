# begin: retire in-app source/error panel, hot-load `.pm` edits (desktop)

## Context

`docs/VISION.md` already calls this out as the planned pivot for `begin`: "further
investment shifts away from deepening the in-app editor toward interop with VSCode:
edit `.pm` sources in VSCode, hot-load them into a running Dioxus app... with errors
and diagnostics reported back through the terminal/console where `dx serve` was
invoked. **The in-app `SourcePanel` is scaffolding, not a permanent feature**."

This spec implements that pivot: the `SourcePanel` (textarea, Apply button, and
in-app diagnostic panel) is removed. The demo pm-lang source moves out of a Rust
string constant into a standalone `.pm` file that's watched via Dioxus's own
hot-reload devserver connection, and all diagnostics — parse errors, startup
errors, and runtime propagate/write errors from Inspector edits — are printed to
stderr as ANSI-colored rustc-style diagnostics instead of being held in app state.

Scope is desktop-only. The `web` feature keeps building (falls back to a
compiled-in snapshot, no live reload); closing that gap is explicitly deferred
(see "Deferred web-target issues" below).

## Components

### `begin/assets/demo.pm` (new)

The pm-lang source currently embedded as `DEMO_SOURCE: &str` in `app.rs`, moved
verbatim into its own file. This is the file a developer edits in VSCode.

### `begin/src/demo_source.rs` (replaces `begin/src/source_panel.rs`)

Owns everything related to loading and rebuilding the demo sheet:

- `const DEMO_ASSET: Asset = asset!("/assets/demo.pm")` — referencing the file
  individually (not its containing folder) is required for `dx serve`'s file
  watcher to track it and report it in hot-reload messages; folder-level
  `asset!()` references are documented as not hot-reloading.
- `pub struct BuildOutcome { sheet_labels: Option<(Sheet, Labels)>, error: Option<String> }`
  and `pub fn build_sheet(source: &str) -> BuildOutcome` — moved from
  `source_panel.rs` with one change: its diagnostic formatting switches from
  `Renderer::plain()` to `Renderer::styled()` since output now always goes to a
  terminal, not an HTML `<pre>` block.
- `pub fn load_demo_source() -> String`:
  - `#[cfg(feature = "desktop")]`: resolve `DEMO_ASSET` to a filesystem path via
    `dioxus::asset_resolver::asset_path` and read it with `std::fs`.
  - `#[cfg(not(feature = "desktop"))]` (i.e. `web`): `include_str!("../assets/demo.pm")`,
    a compile-time snapshot with no live reload.
- `pub fn spawn_hot_reload(...)` *(desktop only, gated on `debug_assertions` since
  it only does anything under `dx serve`)*:
  - Calls `dioxus::devtools::connect(callback)` on a background thread (this is
    the same public devserver websocket connection the CLI itself uses — confirmed
    via `docs.rs/dioxus-devtools` and the vendored source, not guessed).
  - The callback inspects `DevserverMsg::HotReload(msg)` → `msg.assets: Vec<PathBuf>`
    for an entry matching our resolved demo-file path, and forwards a reload
    notification through a channel.
  - An async task spawned from `App` (so it runs on the Dioxus runtime and can
    touch signals) receives that notification, calls `load_demo_source()` +
    `build_sheet()`, and applies the result exactly like the Apply button does
    today (see Data flow, step 2).

### `begin/src/source_panel.rs` — deleted

### `begin/src/app.rs`

- Drops the `editor_source`, `source_panel_open`, `error`, and `error_is_parse`
  signals and the bottom docked `SourcePanel` from the layout.
- Keeps `sheet`, `labels`, and a renamed `active_source: Signal<String>` (was
  `applied_source`) — the source text of the currently *applied* sheet, needed
  by the Inspector to format runtime propagate-error spans correctly.
- Startup: `load_demo_source()` → `build_sheet()`, panicking on parse failure —
  same invariant as today's `.expect("DEMO_SOURCE must parse successfully")`.
- Wires up `demo_source::spawn_hot_reload` under `#[cfg(all(feature = "desktop", debug_assertions))]`.

### `begin/src/inspector.rs`

- Drops the `error: Signal<Option<String>>` and `error_is_parse: Signal<bool>`
  props entirely — with no persistent error panel, there's nothing to clear or
  distinguish "parse vs. runtime" for.
- Keeps the existing per-field local `has_error: Signal<bool>` (red/invalid
  highlight on `SpTextfield`) unchanged.
- On a write/propagate failure, formats the diagnostic via
  `bridge::format_property_model_error` and does `eprintln!` immediately instead
  of storing it in a shared signal.

### `begin/src/bridge.rs`

- `format_property_model_error` switches its internal `Renderer::plain()` to
  `Renderer::styled()`.

## Data flow

1. **Startup:** `App` calls `demo_source::load_demo_source()`, then `build_sheet()`.
   A parse failure here panics (startup invariant); this mirrors today's behavior
   since `DEMO_SOURCE`/`demo.pm` is expected to always be valid at rest.
2. **Edit in VSCode → hot reload (desktop, debug only):** `dx serve` detects the
   `.pm` file change and pushes a `HotReloadMsg` over its devserver websocket. Our
   listener matches the asset path, reads the new file, calls `build_sheet()`.
   - On parse success (with or without a subsequent propagate failure): replace
     `sheet`, `labels`, and `active_source` — matching the current Apply-button
     semantics exactly (a runtime/propagate failure still replaces the sheet so
     the new source's structure is visible; only a parse failure leaves the
     previous good sheet in place).
   - On parse failure: `sheet`/`labels`/`active_source` are left unchanged; only
     the diagnostic is printed.
3. **Errors:** every failure path other than startup now ends in
   `eprintln!("{msg}")` using the styled renderer — no error text is retained in
   app state anywhere.
4. **Inspector cell edits:** mechanically unchanged (write → propagate); failures
   print to stderr instead of setting a shared signal, and the edited field's own
   local `has_error` still drives its red/invalid highlight.

## Removed

- The `SourcePanel` component: textarea, Apply button, collapse toggle, and the
  in-app diagnostic `<pre>` block.
- The `error_is_parse` distinction, entirely — it existed only to decide whether
  the shared error panel should be cleared by a successful Inspector propagate;
  with no panel, there's nothing to clear.

## Deferred web-target issues (tracked here, not solved now)

1. **No public hot-reload hook for custom assets on web.** `dioxus_devtools::connect`
   is `#[cfg(not(target_family = "wasm"))]`; `dioxus-web`'s equivalent websocket
   listener is a private (`pub(crate)`) implementation detail with no exposed
   callback for arbitrary (non-CSS/JS) asset changes. Closing this gap needs
   either an upstream Dioxus API addition or a hand-written `web_sys` websocket
   client against the CLI's internal `/_dioxus` devserver endpoint.
2. **No stderr-equivalent surface on web.** Browser code has no stderr; the
   equivalent would be routing through a wasm panic/log hook to `console.error`,
   which is a different mechanism than desktop's `eprintln!` and is not designed
   here.

These may become upstream Dioxus issues or get solved via a custom dev server;
that decision is out of scope for this spec.

## Testing

- `demo_source::build_sheet` keeps its existing unit tests, moved as-is from
  `source_panel.rs`.
- Replace `app.rs`'s `DEMO_SOURCE`-const-based tests with equivalents that read
  `begin/assets/demo.pm` (via `include_str!` or `load_demo_source()`) and assert
  the same forced-cell behavior.
- The hot-reload websocket wiring itself is not unit-testable in the usual sense;
  it will be verified manually via `dx serve --platform desktop` (edit
  `demo.pm`, confirm the graph updates without touching the app window) before
  this is considered done, per the `verify` skill.
