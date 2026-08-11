# begin: extensible, live-updating examples list (replaces the demo tab row)

## Context

`begin` currently ships two `.adm2` demo files under `begin/assets/` (`toy_example.adm2`,
`image_resize.adm2`), picked between via a horizontal row of `SpActionGroup`/`SpActionButton`
"tabs" at the top of the window (`DemoPicker` in `app.rs`). The list is generated at
**build time**: `build.rs` scans `assets/*.adm2` once and bakes the names into a
`DEMO_NAMES` constant; per its own doc comment, adding or removing a demo file requires a
manual nudge (e.g. `touch build.rs`) before it shows up — editing an *existing* file's
content hot-reloads live via `dx serve`'s devserver channel, but the file list itself does
not.

Separately, `begin/examples/` already holds three more `.adm2` files (`diamond.adm2`,
`diamond-wing.adm2`, `out-cell.adm2`) that are referenced only in `adam-rs` test comments —
`begin` doesn't load them at all today.

This spec merges both sets into one extensible, **genuinely live-updating** list of
examples, replacing the tab row with a scrollable sidebar list, so `begin` can grow past a
handful of examples without either a rebuild step or running out of horizontal space.

## Decisions

- All five `.adm2` files move into `begin/examples/` (a directory that already exists with
  three of them). `assets/` keeps only genuine static assets (JS/CSS/icons).
- Every `demo`-flavored identifier renames to `example` (module, types, functions, tests) —
  full rename, not a partial patch, so the vocabulary matches the feature's new framing
  everywhere it appears.
- The tab row (`SpActionGroup`/`SpActionButton`) is removed outright, not resized or paged.
  It's replaced by a persistent, scrollable left sidebar list, mirroring `Inspector`'s panel
  on the right.
- Desktop gets a real live-update loop: a filesystem watcher on `begin/examples/` that
  reacts to files being added, removed, or edited — no rebuild needed for any of the three.
  This also replaces today's `dx-serve`-devserver-dependent hot reload (`spawn_hot_reload`),
  letting the `dioxus-devtools` dependency be dropped entirely (it isn't used anywhere else
  in `begin`).
- Web has no live filesystem; it keeps a build-time-embedded snapshot, matching today's
  existing (and accepted) limitation for demos.

## Components

### `begin/examples/` (five files, merged)

`toy_example.adm2` and `image_resize.adm2` move here from `begin/assets/`, joining
`diamond.adm2`, `diamond-wing.adm2`, and `out-cell.adm2`.

### `begin/build.rs`

Trimmed to scan `begin/examples/` instead of `begin/assets/`, and to emit only one array —
`EXAMPLES_WITH_SOURCE: &[(&str, &str)]` (name, embedded source via `include_str!`), gated
`#[cfg(any(not(feature = "desktop"), test))]` exactly as `DEMOS_WITH_SOURCE` is today. The
`DEMO_NAMES` array and the `DEMO_ASSETS`/`asset!()` registrations are dropped: `DEMO_NAMES`
is superseded by desktop's runtime directory scan (below), and `DEMO_ASSETS` existed solely
to nudge `dx`'s bundler into tracking files for the hot-reload mechanism this spec removes.

### `begin/src/example_source.rs` (renamed from `demo_source.rs`)

- `SourceOrigin::Demo` → `SourceOrigin::Example`; `ActiveSource::file_name()`'s demo branch
  formats `begin/examples/{name}.adm2` instead of `begin/assets/{name}.adm2`.
- `pub fn available_examples() -> Vec<String>` (renamed from `available_demos`, return type
  changed from `&'static [&'static str]`):
  - `#[cfg(feature = "desktop")]`: `std::fs::read_dir` over `examples_dir`
    (`CARGO_MANIFEST_DIR/examples`), filtered to `.adm2`, file-stemmed, sorted, collected —
    a fresh scan on every call, not cached, so it always reflects the current directory
    contents.
  - `#[cfg(not(feature = "desktop"))]`: names derived from `EXAMPLES_WITH_SOURCE`'s keys,
    sorted — no separate compile-time names array needed.
- `pub fn load_example_source(name: &str) -> Result<String, String>` (renamed from
  `load_demo_source`):
  - `#[cfg(feature = "desktop")]`: rejects `name` containing `/`, `\`, or `..` (replacing the
    old whitelist-against-`DEMO_NAMES` check, which no longer exists as a static array), then
    reads `examples_dir.join(format!("{name}.adm2"))` directly — same path-traversal
    protection, expressed as a direct check instead of a list membership test.
  - `#[cfg(not(feature = "desktop"))]`: unchanged shape, looked up in `EXAMPLES_WITH_SOURCE`.
- `spawn_hot_reload`/`hot_reload_targets_demo` (desktop-only, `dioxus_devtools`-based):
  deleted.
- New `#[cfg(feature = "desktop")] pub fn spawn_examples_watch(on_change: impl FnMut() + Send + 'static) -> notify::Result<notify::RecommendedWatcher>`:
  a `notify::recommended_watcher` on `examples_dir` (`NonRecursive`), calling `on_change` on
  every filesystem event — add, remove, or edit — mirroring `open_file.rs::spawn_watch`'s
  existing shape but pointed at a directory instead of a single file.

### `begin/src/spectrum.rs`

New wrapper components, following the existing `SpActionGroup`/`SpActionButton` pattern:

- `SpSideNav { children: Element }` → `<sp-sidenav>`.
- `SpSideNavItem { label: String, selected: bool, onclick: EventHandler<MouseEvent> }` →
  `<sp-sidenav-item>`. Both map to Spectrum Web Components already registered via the
  existing `@spectrum-web-components/bundle/elements.js` import in `js/spectrum-entry.js` —
  no new JS bundling changes needed.

### `begin/src/app.rs`

- `DemoPicker` → `ExamplesPicker`, rewritten to render a scrollable `SpSideNav` of
  `SpSideNavItem`s (one per name in a new `example_names: Signal<Vec<String>>` prop) instead
  of the `SpActionGroup` tab row. Styled like `Inspector`'s panel: `width: 260px; min-width:
  260px; height: 100%; overflow-y: auto; padding: 12px; box-sizing: border-box;`.
- Layout: `ExamplesPicker` moves out of the top bar and into the main flex row, as the first
  child alongside `GraphView` and `Inspector`:

  ```
  div { flex: 1; display: flex; ...
      ExamplesPicker { sheet, labels, active_source, example_names, on_select: on_example_selected }
      GraphView { data: graph_data, source_id }
      Inspector { sheet, labels, active_source }
  }
  ```

  `OpenFileControls` (opening an arbitrary file outside the examples list) stays in the top
  bar, unaffected.
- `App` gains an `example_names: Signal<Vec<String>>` initialized from `available_examples()`
  at startup.
- The existing single-consumer reload loop (today fed by `spawn_hot_reload` and the
  opened-file watcher) gains a third producer: `spawn_examples_watch`. On wake, in addition
  to today's reload-the-active-source logic, it also does `example_names.set(available_examples())`
  to pick up any add/remove. If the *active* example was the one removed, the reload attempt
  fails; matching the existing tolerance pattern elsewhere, the failure is printed to stderr
  and the last-good sheet is left in place rather than cleared.
- `load_demo`/`load_example` (private helper) and its embedded `format!("begin/examples/{name}.adm2")`
  path updated to match.

## Data flow

1. **Startup:** `available_examples()` seeds `example_names`; the first name loads via
   `load_example_source` + `build_sheet`, same as today.
2. **Selecting an item:** click on an `SpSideNavItem` → `load_example` → replaces `sheet`,
   `labels`, `active_source`; `on_select` clears any leftover opened-file watcher, same as
   today's `DemoPicker`/`on_demo_selected` wiring.
3. **Desktop: file added/removed/edited in `begin/examples/`:** `spawn_examples_watch` fires
   → `example_names` refreshes; if the active source is an `Example` whose file still exists,
   its content is re-read and rebuilt (same as an in-place edit today); if it no longer
   exists, the read fails and is reported to stderr without disturbing the current view.
4. **Web:** `example_names` and content are fixed at build time; no live update (unchanged
   limitation from today).

## Removed

- `DemoPicker`'s `SpActionGroup`/`SpActionButton` tab row.
- `spawn_hot_reload`, `hot_reload_targets_demo`, and the `dioxus-devtools` dependency
  (`Cargo.toml` and `js`/bundling untouched — this dependency is Rust-only).
- `build.rs`'s `DEMO_NAMES` and `DEMO_ASSETS` arrays.

## Testing

- `example_source.rs` tests renamed/updated in place: `toy_example_source()` helper,
  `every_bundled_demo_parses_successfully` (now exercises all five files),
  `active_source_file_name_*` tests updated to the `begin/examples/{name}.adm2` convention,
  `available_demos_is_sorted_and_nonempty` → `available_examples_is_sorted_and_nonempty`.
- New: a desktop test creating a temp `.adm2` file in a scratch directory and asserting
  `available_examples()`-equivalent scanning logic picks it up (sorted, nonempty), and a
  `load_example_source` test asserting names containing `../` are rejected — replacing
  `load_demo_source_rejects_name_not_exactly_in_manifest`'s intent now that there's no static
  list to check membership against.
- New: a `spawn_examples_watch` test mirroring `open_file.rs`'s existing `spawn_watch` tests
  (create/modify a temp file in a watched directory, assert the callback fires).
- Manual verification via the `verifying-begin-ui` skill: confirm the sidebar renders,
  scrolls with more items than fit, selecting an item switches the graph, and (desktop)
  adding a new `.adm2` file to `begin/examples/` while the app is running makes it appear in
  the list without a restart.
