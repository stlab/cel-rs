# begin: Open an Arbitrary `.adm2` File Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user in `begin` open any `.adm2` file — not just the bundled demos under `begin/assets/` — via a native OS dialog on desktop or the browser's File System Access API on web, with diagnostics reported against the file's real path/name and live reload (automatic on desktop, manual "Refresh" on web).

**Architecture:** `ActiveSource` gains a `SourceOrigin` tag (`Demo` vs `Opened(path)`). A new `open_file` module owns platform-specific file acquisition: `rfd::AsyncFileDialog` + `notify` filesystem watching on desktop, a `window.beginOpenFile` JS helper (File System Access API, with an `<input type="file">` fallback for non-Chromium browsers) on web. Both platforms funnel into the same `build_sheet()`/error-reporting path already used for demos, and share one reload-notification channel so "reload the active source" dispatches on `origin` regardless of what triggered it.

**Tech Stack:** Rust, Dioxus 0.7 (desktop via `wry`/WebView2, web via wasm), `rfd` (native file dialogs, desktop-only), `notify` (filesystem watching, desktop-only), vanilla JS + the File System Access API (web-only), `document::eval`/`dioxus.send()` for the Rust↔JS bridge (existing pattern, see `inspector.rs`/`graph_view.rs`).

## Global Constraints

- `rfd` and `notify` are optional dependencies, pulled in only by the `desktop` feature — never compiled into the `web` build.
- Diagnostics for an opened file (parse/propagate errors) must show its real path (desktop) or name (web) in the header, via `ActiveSource::file_name()` — never a synthetic `begin/assets/...` string.
- Only one reload watch is ever active: opening a file replaces any prior opened-file watcher; picking a demo again leaves the file-watcher signal empty (no accumulation of stale watchers/threads over an app session).
- Web has **no polling**. Live reload there is a manual "Refresh" button, rendered only when a re-readable browser file handle exists.
- Browsers without `window.showOpenFilePicker` (Firefox, Safari) fall back to a one-shot `<input type="file" accept=".adm2">`; no "Refresh" button renders on that path.
- Cancelling the OS/browser dialog is a no-op: current `sheet`/`labels`/`active_source` are left untouched.
- A read/parse/propagate failure on an opened file never crashes the app: it prints to stderr and leaves the last-good sheet/labels in place, matching today's demo-hot-reload-failure behavior.

---

## File Structure

- **`begin/Cargo.toml`** — add `rfd` and `notify` as optional dependencies gated into the `desktop` feature.
- **`begin/src/demo_source.rs`** (modified) — gains `SourceOrigin` and the `origin` field on `ActiveSource`; `file_name()` dispatches on it. No change to demo-loading behavior itself.
- **`begin/src/open_file.rs`** (new) — OS-integration primitives only, no UI: desktop file-pick/read/watch (`rfd`/`notify`), and the web-side JSON payload type + `document::eval` script constants. This is the only file that touches `rfd`/`notify`/the JS bridge directly.
- **`begin/assets/open_file.js`** (new) — `window.beginOpenFile`: File System Access API open/refresh, with the `<input type="file">` fallback. Loaded unconditionally via `document::Script` (harmless dead code on desktop, which never calls it).
- **`begin/src/app.rs`** (modified) — `load_opened` (desktop) alongside the existing `load_demo`; the hot-reload consumer loop generalizes to dispatch on `SourceOrigin`; a new `OpenFileControls` component (alongside `DemoPicker`) renders the "Open…" button (both platforms) and "Refresh" button (web only, when a handle exists).

---

### Task 1: `SourceOrigin` and `ActiveSource.origin`

**Files:**
- Modify: `begin/src/demo_source.rs`

**Interfaces:**
- Produces: `pub enum SourceOrigin { Demo, Opened(String) }` (derives `Clone, PartialEq, Eq, Debug, Default`, `#[default] Demo`), `ActiveSource.origin: SourceOrigin`, `ActiveSource::file_name(&self) -> String` (updated to dispatch on `origin`).

- [ ] **Step 1: Write the failing tests**

Add to `demo_source.rs`'s existing `#[cfg(test)] mod tests`, replacing the current `active_source_file_name_matches_convention` test:

```rust
#[test]
fn active_source_file_name_demo_matches_convention() {
    let active = ActiveSource {
        name: "toy_example".to_string(),
        text: String::new(),
        origin: SourceOrigin::Demo,
    };
    assert_eq!(active.file_name(), "begin/assets/toy_example.adm2");
}

#[test]
fn active_source_file_name_opened_returns_path_directly() {
    let active = ActiveSource {
        name: "my_model".to_string(),
        text: String::new(),
        origin: SourceOrigin::Opened("/home/user/models/my_model.adm2".to_string()),
    };
    assert_eq!(active.file_name(), "/home/user/models/my_model.adm2");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p begin active_source_file_name`
Expected: FAIL to compile — `ActiveSource` has no field `origin` and `SourceOrigin` doesn't exist yet.

- [ ] **Step 3: Implement `SourceOrigin` and update `ActiveSource`**

In `demo_source.rs`, above `ActiveSource`:

```rust
/// Identifies where an [`ActiveSource`]'s content came from: a bundled demo
/// discovered under `assets/`, or a file the user opened via the "Open…" action.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum SourceOrigin {
    /// One of [`available_demos`], loaded from `begin/assets/`.
    #[default]
    Demo,
    /// A file opened via the "Open…" action.
    ///
    /// Desktop: the real absolute filesystem path (`Path::display()`
    /// formatting). Web: the picked file's name only — browsers never expose
    /// a real filesystem path.
    Opened(String),
}
```

Update `ActiveSource` and its `file_name`:

```rust
/// The currently active source: its display name, full text, and where it
/// came from (a bundled demo or a user-opened file) — see [`SourceOrigin`].
#[derive(Clone, Default)]
pub struct ActiveSource {
    /// Display label: the demo's name (its filename stem) or the opened
    /// file's name.
    pub name: String,
    /// The source's full adam-lang source text.
    pub text: String,
    /// Where this source came from.
    pub origin: SourceOrigin,
}

impl ActiveSource {
    /// The path shown in diagnostic headers: `begin/assets/<name>.adm2` for a
    /// bundled demo, or the opened file's real path/name directly.
    pub fn file_name(&self) -> String {
        match &self.origin {
            SourceOrigin::Demo => format!("begin/assets/{}.adm2", self.name),
            SourceOrigin::Opened(path) => path.clone(),
        }
    }
}
```

- [ ] **Step 4: Fix existing `ActiveSource` construction sites to compile**

In `begin/src/app.rs`'s `load_demo` function, both `ActiveSource { name: ..., text: ... }` literals need `origin: SourceOrigin::Demo` added (import `crate::demo_source::SourceOrigin` alongside the existing `demo_source` imports at the top of `app.rs`).

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p begin active_source_file_name`
Expected: PASS (2 tests).

Run: `cargo test -p begin --workspace` (or at least `-p begin`) to confirm nothing else broke.
Expected: PASS, zero warnings.

- [ ] **Step 6: Commit**

```bash
git add begin/src/demo_source.rs begin/src/app.rs
git commit -m "feat(begin): add SourceOrigin to track bundled-demo vs opened-file sources"
```

---

### Task 2: Desktop file read (`open_file::read_opened_file`)

**Files:**
- Create: `begin/src/open_file.rs`
- Modify: `begin/src/main.rs` (add `mod open_file;`)
- Modify: `begin/Cargo.toml`

**Interfaces:**
- Consumes: nothing from prior tasks (pure I/O).
- Produces: `#[cfg(feature = "desktop")] pub fn read_opened_file(path: &std::path::Path) -> Result<String, String>` — used by Task 5's `load_opened`.

- [ ] **Step 1: Add `rfd` and `notify` as desktop-only optional dependencies**

Run:
```bash
cargo add rfd --optional -p begin
cargo add notify --optional -p begin
```

Then edit `begin/Cargo.toml`'s `[features]` section so both ride in with the `desktop` feature:

```toml
[features]
default = ["desktop"]
desktop = ["dioxus/desktop", "dep:rfd", "dep:notify"]
web = ["dioxus/web"]
playground = []
```

> Note for the implementer: `rfd`'s async dialog (`AsyncFileDialog`, used in Task 3) manages its own background thread on desktop backends and does not need an explicit async-runtime cargo feature selected on Windows/macOS. If the Linux build fails to compile with a missing-runtime error, check `rfd`'s docs.rs page for the resolved version — its GTK/XDG-portal backend sometimes requires picking a `tokio`/`async-std` feature to match the host async runtime (Dioxus desktop uses `tokio`).

- [ ] **Step 2: Add the new module**

In `begin/src/main.rs`, add `mod open_file;` alongside the other `mod` declarations.

- [ ] **Step 3: Write the failing test**

Create `begin/src/open_file.rs`:

```rust
//! Desktop file-open primitives: OS-native dialog, direct filesystem read,
//! and a filesystem watcher for the currently opened file. See
//! `begin/assets/open_file.js` and this module's web-only items for the web
//! build's equivalent (File System Access API instead of a real filesystem).

#[cfg(feature = "desktop")]
use std::path::Path;

/// Reads `path`'s full text.
///
/// # Errors
///
/// Returns `Err` with a human-readable message if `path` cannot be read
/// (missing, permission denied, not valid UTF-8, etc).
#[cfg(feature = "desktop")]
pub fn read_opened_file(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("failed to read {}: {e}", path.display()))
}

#[cfg(all(test, feature = "desktop"))]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn read_opened_file_returns_file_contents() {
        let path = temp_path("begin_open_file_test_contents.adm2");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"sheet s { cell a: i32 = 1; }")
            .unwrap();

        let result = read_opened_file(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.unwrap(), "sheet s { cell a: i32 = 1; }");
    }

    #[test]
    fn read_opened_file_missing_file_returns_err() {
        let path = temp_path("begin_open_file_test_does_not_exist.adm2");
        let _ = std::fs::remove_file(&path);

        let result = read_opened_file(&path);

        assert!(result.is_err());
    }
}
```

- [ ] **Step 4: Run the tests to verify they fail, then pass**

Run: `cargo test -p begin open_file::`
Expected first: FAIL (module didn't exist / didn't compile before Step 3's file existed — if the file is created directly with the implementation already in place, instead run the tests once to confirm both pass and skip the "verify it fails" sub-step, noting this in the task).
Expected after Step 3 is saved: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add begin/src/open_file.rs begin/src/main.rs begin/Cargo.toml
git commit -m "feat(begin): add desktop file-read primitive for opened files"
```

---

### Task 3: Desktop file dialog (`open_file::pick_file`)

**Files:**
- Modify: `begin/src/open_file.rs`

**Interfaces:**
- Consumes: `rfd::AsyncFileDialog` (added in Task 2).
- Produces: `#[cfg(feature = "desktop")] pub async fn pick_file() -> Option<std::path::PathBuf>` — used by Task 5's `load_opened` trigger.

- [ ] **Step 1: Implement `pick_file`**

There is no meaningful automated test for this function — it drives a real OS dialog and blocks on user interaction, the same category of limitation as the existing `spawn_hot_reload` (also untested). It will be verified manually in Task 5.

Add to `begin/src/open_file.rs`:

```rust
/// Opens the native "Open File" dialog restricted to `.adm2` files and
/// returns the picked path, or `None` if the user cancelled.
///
/// - Complexity: awaits user interaction; no upper bound on wall-clock time.
#[cfg(feature = "desktop")]
pub async fn pick_file() -> Option<std::path::PathBuf> {
    let handle = rfd::AsyncFileDialog::new()
        .add_filter("adm2", &["adm2"])
        .pick_file()
        .await?;
    Some(handle.path().to_path_buf())
}
```

- [ ] **Step 2: Confirm the crate still builds**

Run: `cargo build -p begin --features desktop`
Expected: builds cleanly, zero warnings.

- [ ] **Step 3: Commit**

```bash
git add begin/src/open_file.rs
git commit -m "feat(begin): add native file-open dialog for desktop"
```

---

### Task 4: Desktop file watcher (`open_file::spawn_watch`)

**Files:**
- Modify: `begin/src/open_file.rs`

**Interfaces:**
- Consumes: `notify::recommended_watcher` (added in Task 2).
- Produces: `#[cfg(feature = "desktop")] pub fn spawn_watch(path: std::path::PathBuf, on_change: impl FnMut() + Send + 'static) -> notify::Result<notify::RecommendedWatcher>` — used by Task 5, whose caller must keep the returned watcher alive (e.g. in a `Signal`) for as long as it should keep watching; dropping it stops the watch.

- [ ] **Step 1: Implement `spawn_watch`**

Not unit-tested, for the same reason as `pick_file`: it wires up real OS filesystem-event delivery, which is flaky/slow to assert on in a unit test and untested precedent already exists for this category (`spawn_hot_reload`). Verified manually in Task 5 (edit an opened file externally, confirm the sheet reloads).

Add to `begin/src/open_file.rs`:

```rust
/// Watches `path` for changes, calling `on_change` on every filesystem event.
/// The returned watcher must be kept alive for as long as the watch should
/// remain active — dropping it stops watching.
///
/// # Errors
///
/// Returns `Err` if the underlying OS watch could not be established (e.g.
/// `path`'s parent directory doesn't exist).
#[cfg(feature = "desktop")]
pub fn spawn_watch(
    path: std::path::PathBuf,
    mut on_change: impl FnMut() + Send + 'static,
) -> notify::Result<notify::RecommendedWatcher> {
    use notify::Watcher;

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            on_change();
        }
    })?;
    watcher.watch(&path, notify::RecursiveMode::NonRecursive)?;
    Ok(watcher)
}
```

- [ ] **Step 2: Confirm the crate still builds**

Run: `cargo build -p begin --features desktop`
Expected: builds cleanly, zero warnings.

- [ ] **Step 3: Commit**

```bash
git add begin/src/open_file.rs
git commit -m "feat(begin): add filesystem watcher for opened files on desktop"
```

---

### Task 5: Wire desktop "Open…" into `app.rs`

**Files:**
- Modify: `begin/src/app.rs`

**Interfaces:**
- Consumes: `open_file::pick_file()`, `open_file::read_opened_file()`, `open_file::spawn_watch()` (Tasks 2–4); `SourceOrigin`, `ActiveSource` (Task 1).
- Produces: `load_opened(path: std::path::PathBuf) -> (Sheet, Labels, ActiveSource)` (desktop-only, alongside the existing `load_demo`); a generalized reload dispatch inside the existing hot-reload consumer loop; an `OpenFileControls` component rendered next to `DemoPicker`.

- [ ] **Step 1: Add `load_opened`, mirroring `load_demo`**

In `app.rs`, alongside the existing `load_demo` function:

```rust
/// Reads `path`, builds its sheet, and returns it alongside the
/// [`ActiveSource`] describing what just loaded. Mirrors [`load_demo`]'s
/// failure handling: a read or parse failure prints to stderr and falls back
/// to an empty sheet rather than failing.
#[cfg(feature = "desktop")]
fn load_opened(path: std::path::PathBuf) -> (Sheet, Labels, ActiveSource) {
    let file_name = path.display().to_string();
    match crate::open_file::read_opened_file(&path) {
        Ok(source) => {
            let outcome = build_sheet(&source, &file_name);
            if let Some(err) = &outcome.error {
                eprintln!("{err}");
            }
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| file_name.clone());
            let active_source = ActiveSource {
                name,
                text: source,
                origin: crate::demo_source::SourceOrigin::Opened(file_name),
            };
            match outcome.sheet_labels {
                Some((sheet, labels)) => (sheet, labels, active_source),
                None => (Sheet::new(), Labels::new(), active_source),
            }
        }
        Err(err) => {
            eprintln!("{err}");
            (
                Sheet::new(),
                Labels::new(),
                ActiveSource {
                    name: file_name.clone(),
                    text: String::new(),
                    origin: crate::demo_source::SourceOrigin::Opened(file_name),
                },
            )
        }
    }
}
```

- [ ] **Step 2: Generalize the hot-reload consumer loop's dispatch**

Replace the body of the `while rx.next().await.is_some() { ... }` loop inside `App`'s `#[cfg(feature = "desktop")]` `use_hook` (currently always reloading via `load_demo_source(&name)`) with origin-aware dispatch:

```rust
while rx.next().await.is_some() {
    let current = active_source.read().clone();
    let loaded = match &current.origin {
        crate::demo_source::SourceOrigin::Demo => {
            eprintln!("loading begin/assets/{}.adm2", current.name);
            crate::demo_source::load_demo_source(&current.name)
        }
        crate::demo_source::SourceOrigin::Opened(path) => {
            eprintln!("loading {path}");
            crate::open_file::read_opened_file(std::path::Path::new(path))
        }
    };
    let source = match loaded {
        Ok(source) => source,
        Err(err) => {
            eprintln!("{err}");
            continue;
        }
    };
    let outcome = build_sheet(&source, &current.file_name());
    if let Some((new_sheet, new_labels)) = outcome.sheet_labels {
        sheet.set(new_sheet);
        labels.set(new_labels);
        active_source.set(ActiveSource {
            text: source,
            ..current
        });
    }
    if let Some(msg) = outcome.error {
        eprintln!("{msg}");
    }
}
```

(`ActiveSource` needs `#[derive(Clone)]` already present — confirm `..current` struct-update syntax works given `name`/`origin` are cloned from `current` and only `text` changes.)

- [ ] **Step 3: Expose the reload sender outside `use_hook` and add the watcher slot**

The `tx` created inside the existing `use_hook` closure must be reachable by the new Open-button handler so an external file's watcher can feed the same channel. Change the `use_hook` call to return `tx`:

```rust
#[cfg(feature = "desktop")]
let reload_tx = {
    let mut sheet = sheet;
    let mut labels = labels;
    let mut active_source = active_source;
    use_hook(move || {
        let (tx, mut rx) = futures_channel::mpsc::unbounded::<()>();
        crate::demo_source::spawn_hot_reload({
            let tx = tx.clone();
            move || {
                let _ = tx.unbounded_send(());
            }
        });
        spawn(async move {
            use futures_util::StreamExt;
            while rx.next().await.is_some() {
                // ... generalized dispatch from Step 2 ...
            }
        });
        tx
    })
};

#[cfg(feature = "desktop")]
let watcher_slot: Signal<Option<notify::RecommendedWatcher>> = use_signal(|| None);
```

- [ ] **Step 4: Add `OpenFileControls`, rendered next to `DemoPicker`**

```rust
/// "Open…" button: on desktop, opens the native file dialog, loads the
/// picked file, and (re)installs a filesystem watcher on it so external
/// edits reload automatically — replacing any previous opened-file watcher.
#[cfg(feature = "desktop")]
#[component]
fn OpenFileControls(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<ActiveSource>,
    reload_tx: futures_channel::mpsc::UnboundedSender<()>,
    mut watcher_slot: Signal<Option<notify::RecommendedWatcher>>,
) -> Element {
    rsx! {
        SpActionButton {
            onclick: move |_| {
                let mut sheet = sheet;
                let mut labels = labels;
                let mut active_source = active_source;
                let reload_tx = reload_tx.clone();
                spawn(async move {
                    let Some(path) = crate::open_file::pick_file().await else {
                        return;
                    };
                    let (new_sheet, new_labels, new_active) = load_opened(path.clone());
                    sheet.set(new_sheet);
                    labels.set(new_labels);
                    active_source.set(new_active);
                    match crate::open_file::spawn_watch(path, move || {
                        let _ = reload_tx.unbounded_send(());
                    }) {
                        Ok(watcher) => watcher_slot.set(Some(watcher)),
                        Err(err) => eprintln!("failed to watch opened file: {err}"),
                    }
                });
            },
            "Open…"
        }
    }
}
```

Wire it into `App`'s `rsx!` next to `DemoPicker`:

```rust
DemoPicker { sheet, labels, active_source }
#[cfg(feature = "desktop")]
OpenFileControls { sheet, labels, active_source, reload_tx: reload_tx.clone(), watcher_slot }
```

- [ ] **Step 5: Build and run the existing test suite**

Run: `cargo build -p begin --features desktop`
Expected: builds cleanly, zero warnings.

Run: `cargo test -p begin --features desktop`
Expected: all existing tests still pass (this task adds no new automated tests — it wires together Tasks 1–4's already-tested pieces plus the untestable OS dialog/watcher).

- [ ] **Step 6: Manual verification**

Run `dx serve --platform desktop` from `begin/`. Click "Open…", pick a `.adm2` file *outside* `begin/assets/` (e.g. copy `begin/assets/toy_example.adm2` to a scratch location first). Confirm:
- The sheet loads and the picker shows no demo selected.
- Editing the opened file externally reloads it live in the running app.
- Diagnostics (e.g. temporarily break the file's syntax) show the real path in the header, not `begin/assets/...`.
- Clicking a demo afterward switches back cleanly (no crash, no stale reload from the old opened file).

- [ ] **Step 7: Commit**

```bash
git add begin/src/app.rs
git commit -m "feat(begin): wire native Open File dialog with live reload (desktop)"
```

---

### Task 6: Web JS bridge (`open_file.js` + `OpenedFilePayload`)

**Files:**
- Create: `begin/assets/open_file.js`
- Modify: `begin/src/open_file.rs`

**Interfaces:**
- Produces: `#[derive(serde::Deserialize)] pub struct OpenedFilePayload { pub id: Option<u32>, pub name: String, pub text: String }` (platform-agnostic type, used by Task 7 on both platforms' Rust side since `document::eval` round-trips go through it regardless of `cfg`), plus two script-builder helpers: `pub const OPEN_SCRIPT: &str` and `pub fn refresh_script(id: u32) -> String`.
- `id: None` means "no re-readable handle" (the `<input type="file">` fallback path) — Task 7 hides the "Refresh" button in that case.

- [ ] **Step 1: Write `begin/assets/open_file.js`**

```javascript
// window.beginOpenFile: bridges Rust (via document::eval + dioxus.send) to
// the browser's File System Access API, with a plain <input type="file">
// fallback for browsers that don't support it (Firefox, Safari).
//
// open() resolves to `{ id, name, text }` or `null` if the user cancelled.
// `id` is a number (a re-readable handle exists; pass it to refresh()) or
// `null` (the input-fallback path: one-shot only, nothing to refresh).
window.beginOpenFile = {
  handles: {},
  nextId: 0,

  open() {
    if (window.showOpenFilePicker) {
      return (async () => {
        let handle;
        try {
          [handle] = await window.showOpenFilePicker({
            types: [
              {
                description: "adam property model",
                accept: { "application/octet-stream": [".adm2"] },
              },
            ],
          });
        } catch (e) {
          return null; // user cancelled the picker
        }
        const id = this.nextId++;
        this.handles[id] = handle;
        const file = await handle.getFile();
        const text = await file.text();
        return { id, name: handle.name, text };
      })();
    }

    // Fallback: one-shot <input type="file">, no handle survives to refresh from.
    return new Promise((resolve) => {
      const input = document.createElement("input");
      input.type = "file";
      input.accept = ".adm2";
      input.addEventListener("change", async () => {
        const file = input.files[0];
        if (!file) {
          resolve(null);
          return;
        }
        const text = await file.text();
        resolve({ id: null, name: file.name, text });
      });
      // A freshly-created <input type="file"> does not fire "change" on
      // cancel in any major browser — without this listener, cancelling
      // here hangs the promise forever instead of resolving null.
      input.addEventListener("cancel", () => resolve(null));
      input.click();
    });
  },

  refresh(id) {
    const handle = this.handles[id];
    if (!handle) return Promise.resolve(null);
    return (async () => {
      const file = await handle.getFile();
      const text = await file.text();
      return { id, name: handle.name, text };
    })();
  },
};
```

- [ ] **Step 2: Write the failing test for `OpenedFilePayload`**

Add to `begin/src/open_file.rs`, in a `#[cfg(test)] mod web_tests` block:

```rust
#[cfg(test)]
mod web_tests {
    use super::*;

    #[test]
    fn opened_file_payload_deserializes_with_handle_id() {
        let json = r#"{"id": 3, "name": "my_model.adm2", "text": "sheet s {}"}"#;
        let payload: OpenedFilePayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.id, Some(3));
        assert_eq!(payload.name, "my_model.adm2");
        assert_eq!(payload.text, "sheet s {}");
    }

    #[test]
    fn opened_file_payload_deserializes_without_handle_id() {
        let json = r#"{"id": null, "name": "my_model.adm2", "text": "sheet s {}"}"#;
        let payload: OpenedFilePayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.id, None);
    }

    #[test]
    fn refresh_script_embeds_the_given_id() {
        let script = refresh_script(3);
        assert!(script.contains("beginOpenFile.refresh(3)"), "{script}");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p begin web_tests`
Expected: FAIL to compile — `OpenedFilePayload`, `refresh_script` don't exist yet.

- [ ] **Step 4: Implement the payload type and script constants**

Add to `begin/src/open_file.rs` (platform-agnostic — no `#[cfg]` needed, since both platforms' Rust side deserialize the same shape; only the desktop UI never happens to call it):

```rust
/// The result of a web-side `open()`/`refresh()` call, sent from JS via
/// `dioxus.send()` and read back with `eval.recv::<Option<OpenedFilePayload>>()`.
///
/// `id` is `Some(handle_id)` when a re-readable `FileSystemFileHandle` backs
/// this result (the File System Access path) — pass it to [`refresh_script`]
/// to reload later. `None` means the `<input type="file">` fallback was used:
/// the load is one-shot, with nothing to refresh from.
#[derive(serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct OpenedFilePayload {
    pub id: Option<u32>,
    pub name: String,
    pub text: String,
}

/// `document::eval` script that opens the file picker (or its `<input
/// type="file">` fallback) and sends the result back via `dioxus.send()`.
/// Resolves to `null` on JS's side (received as `None`) if the user
/// cancelled.
pub const OPEN_SCRIPT: &str =
    "(async () => { dioxus.send(await window.beginOpenFile.open()); })();";

/// `document::eval` script that re-reads the file behind handle `id` and
/// sends the refreshed `{id, name, text}` back via `dioxus.send()`.
pub fn refresh_script(id: u32) -> String {
    format!("(async () => {{ dioxus.send(await window.beginOpenFile.refresh({id})); }})();")
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p begin web_tests`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add begin/assets/open_file.js begin/src/open_file.rs
git commit -m "feat(begin): add web File System Access bridge for opening files"
```

---

### Task 7: Wire web "Open…"/"Refresh" into `app.rs`

**Files:**
- Modify: `begin/src/app.rs`

**Interfaces:**
- Consumes: `open_file::OPEN_SCRIPT`, `open_file::refresh_script`, `open_file::OpenedFilePayload` (Task 6); `SourceOrigin`, `ActiveSource` (Task 1).
- Produces: the web half of `OpenFileControls` (same component name as Task 5's desktop half, `#[cfg(not(feature = "desktop"))]`), a `refresh_handle: Signal<Option<u32>>` tracking whether a "Refresh" button should render.

- [ ] **Step 1: Load `open_file.js` unconditionally in `App`'s `rsx!`**

Alongside the existing `document::Script` tags near the top of `App`'s `rsx!` block:

```rust
document::Script { src: asset!("/assets/open_file.js") }
```

- [ ] **Step 2: Add a shared helper to turn a payload into an `ActiveSource` + built sheet**

```rust
/// Builds a sheet from a web-side `OpenedFilePayload` and returns it
/// alongside the [`ActiveSource`] describing what just loaded. Mirrors
/// [`load_demo`]/[`load_opened`]'s failure handling.
#[cfg(not(feature = "desktop"))]
fn load_from_payload(payload: crate::open_file::OpenedFilePayload) -> (Sheet, Labels, ActiveSource) {
    let outcome = build_sheet(&payload.text, &payload.name);
    if let Some(err) = &outcome.error {
        eprintln!("{err}");
    }
    let active_source = ActiveSource {
        name: payload.name.clone(),
        text: payload.text,
        origin: crate::demo_source::SourceOrigin::Opened(payload.name),
    };
    match outcome.sheet_labels {
        Some((sheet, labels)) => (sheet, labels, active_source),
        None => (Sheet::new(), Labels::new(), active_source),
    }
}
```

- [ ] **Step 3: Add the web half of `OpenFileControls`**

```rust
/// "Open…"/"Refresh" controls for the web build: "Open…" always calls
/// `window.beginOpenFile.open()`; "Refresh" (rendered only once a
/// re-readable handle exists) re-reads that same handle. Neither watches for
/// changes automatically — browsers have no filesystem-watch API, so reload
/// here is always user-triggered.
#[cfg(not(feature = "desktop"))]
#[component]
fn OpenFileControls(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<ActiveSource>,
) -> Element {
    let mut refresh_handle = use_signal(|| None::<u32>);

    rsx! {
        SpActionButton {
            onclick: move |_| {
                let mut sheet = sheet;
                let mut labels = labels;
                let mut active_source = active_source;
                let mut refresh_handle = refresh_handle;
                spawn(async move {
                    let mut eval = document::eval(crate::open_file::OPEN_SCRIPT);
                    let Ok(payload) = eval.recv::<Option<crate::open_file::OpenedFilePayload>>().await else {
                        return;
                    };
                    let Some(payload) = payload else { return };
                    refresh_handle.set(payload.id);
                    let (new_sheet, new_labels, new_active) = load_from_payload(payload);
                    sheet.set(new_sheet);
                    labels.set(new_labels);
                    active_source.set(new_active);
                });
            },
            "Open…"
        }
        if let Some(id) = *refresh_handle.read() {
            SpActionButton {
                onclick: move |_| {
                    let mut sheet = sheet;
                    let mut labels = labels;
                    let mut active_source = active_source;
                    spawn(async move {
                        let script = crate::open_file::refresh_script(id);
                        let mut eval = document::eval(&script);
                        let Ok(payload) = eval.recv::<Option<crate::open_file::OpenedFilePayload>>().await else {
                            return;
                        };
                        let Some(payload) = payload else { return };
                        let (new_sheet, new_labels, new_active) = load_from_payload(payload);
                        sheet.set(new_sheet);
                        labels.set(new_labels);
                        active_source.set(new_active);
                    });
                },
                "Refresh"
            }
        }
    }
}
```

- [ ] **Step 4: Wire it into `App`'s `rsx!` (same call site as Task 5's desktop version)**

```rust
DemoPicker { sheet, labels, active_source }
#[cfg(feature = "desktop")]
OpenFileControls { sheet, labels, active_source, reload_tx: reload_tx.clone(), watcher_slot }
#[cfg(not(feature = "desktop"))]
OpenFileControls { sheet, labels, active_source }
```

- [ ] **Step 5: Build and run the existing test suite for the web feature**

Run: `cargo build -p begin --no-default-features --features web`
Expected: builds cleanly, zero warnings.

Run: `cargo test -p begin --no-default-features --features web`
Expected: all existing tests still pass; this task adds no new automated tests (the JS bridge itself isn't unit-testable in `cargo test`, same limitation noted in Task 6).

- [ ] **Step 6: Manual verification**

Use the `verifying-begin-ui` skill (serves `begin` as a web app, drives headless Edge) to confirm the "Open…" button renders. Then, manually in a Chromium browser (Edge/Chrome): click "Open…", pick a `.adm2` file, confirm it loads and a "Refresh" button appears; edit the file externally, click "Refresh", confirm the sheet updates. Then manually in Firefox (or Safari, if available): confirm "Open…" still works via the `<input type="file">` fallback and that no "Refresh" button appears.

- [ ] **Step 7: Commit**

```bash
git add begin/src/app.rs
git commit -m "feat(begin): wire browser file picker with manual Refresh (web)"
```

---

### Task 8: Full workspace verification

**Files:** none (verification only).

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Expected: no diff (or apply and re-check).

- [ ] **Step 2: Build and test the whole workspace with zero warnings**

Run: `cargo build --workspace`
Run: `cargo test --workspace`
Run: `cargo test --doc --workspace`
Expected: all pass, zero compiler warnings (per this repo's `CLAUDE.md`, a plain build/test warning — e.g. an unused `mut` left over from a refactor step above — must be fixed even though clippy wouldn't necessarily flag it).

- [ ] **Step 3: Lint all three `begin` configurations**

Run:
```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: all pass with zero warnings.

- [ ] **Step 4: Commit any fixes from Steps 1–3**

```bash
git add -A
git commit -m "chore(begin): fmt/lint fixes for open-file feature"
```

(Skip this step if nothing needed fixing.)

---

## Self-Review

**Spec coverage:**
- Data model (`SourceOrigin`/`ActiveSource`) → Task 1.
- Desktop dialog/read/watch/UI → Tasks 2–5.
- Web JS bridge, payload type, manual Refresh, non-Chromium fallback → Tasks 6–7.
- Error handling via `file_name()`/`build_sheet` → Task 1 (data model) + Tasks 5/7 (call sites use it).
- Testing plan (unit-testable pieces vs. manually-verified OS/browser integration) → called out per-task; final full-suite run → Task 8.

**Placeholder scan:** no TBD/TODO; every step has literal code or an exact command; no "similar to Task N" references — Task 5 and Task 7's `OpenFileControls` are written out in full separately (they're two different `#[cfg]`-gated definitions of the same component name, not a repeat-by-reference).

**Type consistency:** `SourceOrigin`, `ActiveSource` (Task 1) are used identically in Tasks 5 and 7. `OpenedFilePayload`, `OPEN_SCRIPT`, `refresh_script` (Task 6) are consumed with matching names/signatures in Task 7. `read_opened_file`, `pick_file`, `spawn_watch` (Tasks 2–4) are consumed with matching signatures in Task 5.
