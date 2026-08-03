# begin: open an arbitrary `.adm2` file (desktop + web)

## Context

`begin` currently only loads `.adm2` sources bundled at compile time under
`begin/assets/`, discovered by `build.rs` and switched between via the
`DemoPicker` row in `app.rs` (see `docs/superpowers/specs/2026-07-10-begin-hot-reload-design.md`
for how bundled-demo hot reload already works on desktop). There is no way to
point `begin` at a `.adm2` file that isn't checked into the crate.

This spec adds an "Open…" action that lets the user pick any `.adm2` file —
via the OS-native dialog on desktop, the browser's File System Access picker
on web — and loads it alongside the existing demo picker, without requiring
the file to live under `begin/assets/`. Diagnostics for an opened file are
reported against its real path/name, not a synthetic `begin/assets/...` header.

Live reload semantics differ deliberately by platform: desktop gets automatic,
push-based reload (matching the existing bundled-demo experience) via a
filesystem watcher; the web build has no filesystem-watch API available to
browser content, so it gets an explicit "Refresh" button instead of polling.

## Data model

`begin/src/demo_source.rs`'s `ActiveSource` gains an origin tag so the app can
tell a bundled demo apart from a user-opened file:

```rust
pub struct ActiveSource {
    pub name: String,        // display label: demo name, or opened file's name
    pub text: String,
    pub origin: SourceOrigin,
}

pub enum SourceOrigin {
    /// One of `available_demos()`, loaded from `begin/assets/`.
    Demo,
    /// A file opened via the "Open…" action.
    /// - Desktop: the real absolute filesystem path.
    /// - Web: the picked file's name only — browsers never expose a real path.
    Opened(String),
}
```

`ActiveSource::file_name()` dispatches on `origin`:
`Demo` keeps today's `"begin/assets/{name}.adm2"` format; `Opened(path)` returns
`path` directly, so parse/propagate diagnostics show the real location.

## Core reload flow (shared)

`app.rs`'s existing reload loop (the `while rx.next().await` block driven by
`spawn_hot_reload`) generalizes from "reload the active demo by name" to
"reload the active source by its origin": the same channel, the same
`build_sheet()` call and error-reporting path, just dispatching on
`SourceOrigin` for *how* to refetch the text (`load_demo_source(name)` vs. a
direct read of the opened file/handle). Only one reload source is ever wired
up at a time — opening a file tears down the demo hot-reload watch, and
picking a demo afterward tears down the opened-file watch/handle. There is no
dual-tracking to reconcile.

## Desktop implementation

- **New dependencies** (both `desktop`-feature-gated in `begin/Cargo.toml`):
  `rfd` (native file dialogs) and `notify` (filesystem watching).
- **Dialog**: an "Open…" `SpActionButton` next to `DemoPicker` spawns
  `rfd::FileDialog::new().add_filter("adm2", &["adm2"]).pick_file()` via
  `spawn_blocking` (rfd's call blocks the calling thread; running it inline
  would freeze the WebView2 event loop while the OS dialog is open). `None`
  (dialog cancelled) is a no-op — current sheet/labels/active_source untouched.
- **Load**: on `Some(path)`, read with `std::fs::read_to_string`, call
  `build_sheet(&source, &path.display().to_string())`, set
  `ActiveSource { origin: SourceOrigin::Opened(path), .. }`.
- **Watch**: a `notify` watcher is (re-)created on that one path, replacing any
  previous demo-hot-reload or opened-file watcher. Change events feed the same
  reload channel `spawn_hot_reload` already uses, so the existing consumer loop
  in `app.rs` handles both origins uniformly.

## Web implementation

- **Dialog**: the "Open…" button calls `document::eval` with a JS snippet using
  the File System Access API:
  `window.showOpenFilePicker({ types: [{ accept: { "application/octet-stream": [".adm2"] } }] })`.
  This API is **Chromium-only** (Chrome/Edge/Opera; not Firefox/Safari).
- **Load**: JS stores the resulting `FileSystemFileHandle` in a JS-side map
  keyed by an id, reads initial text via `handle.getFile().then(f => f.text())`,
  and sends `{id, name, text}` back to Rust with `dioxus.send()`, read via
  `eval.recv().await` — the same round-trip pattern already used in
  `inspector.rs`'s `oninput` handler and `graph_view.rs`. `ActiveSource.origin`
  becomes `Opened(name)`.
- **Reload — explicit "Refresh" button, not polling**: whenever the active
  source is `Opened` on the web build, a "Refresh" button renders next to the
  file name. Clicking it calls a JS snippet that re-reads the same stored
  handle (`handle.getFile().then(f => f.text())`) and sends the text back
  through the same `dioxus.send()`/`eval.recv()` round trip, then re-runs
  `build_sheet()` — the same code path as any other reload, just user-triggered
  rather than watcher-triggered. No background timer, no polling.
- **Fallback (Firefox/Safari)**: browsers without `showOpenFilePicker` use a
  plain `<input type="file" accept=".adm2">` — one-shot load, no handle
  survives to refresh from. On this path the "Refresh" button is disabled with
  a tooltip explaining the file must be re-opened instead.
- The "Refresh" button only renders on the web build. Desktop already gets
  automatic push-based reload via `notify`, so an extra manual control there
  would be redundant.

## UI

- An "Open…" `SpActionButton` sits alongside the existing `DemoPicker` row in
  `app.rs`.
- When the active source is `Opened`, none of the demo buttons show selected,
  and the opened file's name/path renders next to the "Open…" button (plus the
  "Refresh" button, web-only, per above).
- Picking a demo afterward switches `origin` back to `Demo` and tears down any
  opened-file watcher/handle, exactly mirroring the reverse direction.

## Error handling

Unchanged mechanism: `build_sheet()`'s existing `format_rustc_style`/
`format_adam_error` diagnostics print to stderr on parse/propagate failure,
exactly as demo loads do today — just keyed off the real path/name via
`file_name()` instead of the synthetic `begin/assets/...` header. A read
failure (permission denied, file deleted before a refresh, handle permission
revoked) leaves sheet/labels at their last-good state and prints to stderr,
matching current demo-hot-reload-failure behavior.

## Testing

- Unit tests for `SourceOrigin`/`ActiveSource::file_name()` covering both
  variants (`Demo` and `Opened`), following the existing style in
  `demo_source.rs`'s test module.
- Unit tests for the platform-agnostic reload/build dispatch: given some source
  text and a `SourceOrigin`, `build_sheet` is called with the right
  `file_name()` string.
- Not unit-testable (same limitation the existing `spawn_hot_reload` already
  has): the native `rfd` dialog call, the `notify` watcher, and the JS/File
  System Access interop. These are verified manually — desktop via
  `dx serve --platform desktop` (open a file outside `assets/`, edit it
  externally, confirm live reload; then switch back to a demo and confirm the
  watcher tears down), web via the `verifying-begin-ui` skill plus manual
  Chromium testing for the picker/refresh round trip and Firefox/Safari for the
  `<input type="file">` fallback path.
