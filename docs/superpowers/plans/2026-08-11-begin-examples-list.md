# begin Examples List Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `begin`'s two-item demo tab row with an extensible, live-updating examples list backed by `begin/examples/`, rendered as a scrollable sidebar.

**Architecture:** Rename every `demo`-flavored identifier to `example` and merge all five `.adm2` files into `begin/examples/` (Task 1), while simultaneously swapping the build-time file list + `dx-serve`-dependent hot reload for a runtime directory scan + `notify`-based filesystem watcher on desktop (also Task 1, since the two changes are inseparable). Then replace the tab-row widget with a scrollable `SpSideNav`-based sidebar panel (Task 2). Finish with the full project check suite and manual UI verification (Task 3).

**Tech Stack:** Rust, Dioxus 0.7 (desktop/web), Spectrum Web Components (via existing `spectrum.rs` wrappers), `notify` crate (already a desktop dependency).

## Global Constraints

- `cargo fmt --all` must be run before any commit (enforced by the repo's pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings.
- Before considering this done, run all three clippy invocations from `CLAUDE.md`: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`, `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`, `cargo clippy -p begin --all-targets -- -D warnings`.
- Never commit directly to `main` (this work happens on the existing `worktree-begin-example-list` branch).
- Every function needs a contract-style `///` doc comment per `CLAUDE.md`'s Documentation Comments rules (summary, preconditions/`# Errors`, postconditions, complexity where non-O(1)).
- No backwards-compatibility shims: rename fully, don't alias old names.
- Per `begin/CLAUDE.md`: a UI change isn't done until it's actually rendered and looked at via the `verifying-begin-ui` skill — passing `cargo build`/`cargo clippy` proves nothing about what renders.
- `notify = "8.2.0"` is already an optional, desktop-gated dependency in `begin/Cargo.toml` — no new dependencies are needed. `dioxus-devtools = "0.7.10"` (unconditional dependency) is being removed.

---

## Task 1: Rename `demo` → `example`, merge into `begin/examples/`, and replace the build-time list + `dx-serve` hot reload with a live directory scan + filesystem watcher

**Files:**
- Move: `begin/assets/toy_example.adm2` → `begin/examples/toy_example.adm2`
- Move: `begin/assets/image_resize.adm2` → `begin/examples/image_resize.adm2`
- Modify: `begin/build.rs` (full rewrite)
- Modify: `begin/src/demo_source.rs` → renamed to `begin/src/example_source.rs` (full rewrite)
- Modify: `begin/src/main.rs:4`
- Modify: `begin/src/inspector.rs:20,41`
- Modify: `begin/src/app.rs` (imports, `App`, `load_demo`→`load_example`, `DemoPicker`→`ExamplesPicker` — rename only, widget unchanged in this task, tests)
- Modify: `begin/Cargo.toml` (remove `dioxus-devtools` dependency)

**Interfaces:**
- Produces (used by Task 2): `crate::example_source::available_examples() -> Vec<String>`, `crate::example_source::ActiveSource`, `crate::example_source::SourceOrigin::Example`, `crate::example_source::load_example_source(name: &str) -> Result<String, String>`, `crate::example_source::spawn_examples_watch(on_change: impl FnMut() + Send + 'static) -> notify::Result<notify::RecommendedWatcher>` (desktop only), `crate::example_source::EXAMPLES_WITH_SOURCE: &[(&str, &str)]` (non-desktop/test only), `app::load_example(name: &str) -> (Sheet, Labels, ActiveSource)` (private, still used by Task 2's `ExamplesPicker`), `app::ExamplesPicker` component with props `sheet: Signal<Sheet>, labels: Signal<Labels>, active_source: Signal<ActiveSource>, example_names: Signal<Vec<String>>, on_select: Callback<()>` (Task 2 rewrites its body, not its props).

This task keeps the picker rendered as the existing `SpActionGroup`/`SpActionButton` row (same position, same widget) so the app stays fully working end to end — Task 2 only swaps that widget and its position in the layout.

- [ ] **Step 1: Move the two example files with git**

```bash
git mv begin/assets/toy_example.adm2 begin/examples/toy_example.adm2
git mv begin/assets/image_resize.adm2 begin/examples/image_resize.adm2
```

- [ ] **Step 2: Rewrite `begin/build.rs`**

```rust
//! Generates a compile-time manifest of `examples/*.adm2` files, embedded as
//! source text for platforms/builds with no live filesystem to read from at
//! runtime.
//!
//! Desktop reads examples directly off disk at runtime instead (see
//! `available_examples`/`load_example_source` in `src/example_source.rs`), so
//! it needs nothing from this script beyond compiling. The web build has no
//! filesystem to read at runtime, and tests want a fixture that doesn't
//! depend on desktop asset bundling being available - both instead get every
//! example's content embedded at compile time via `include_str!`, generated
//! here into `$OUT_DIR/example_manifest.rs`, which `example_source.rs`
//! splices in via `include!`.
//!
//! Deliberately watches only `build.rs` itself (`cargo:rerun-if-changed=
//! build.rs` below), not the `examples/` directory: watching the directory
//! would make Cargo treat every `.adm2` file as a build-script input, so
//! editing an *existing* example's content would force a full rebuild on
//! every edit for the platforms that read this generated manifest. Adding or
//! removing an example file still requires a rebuild for those platforms
//! (unavoidable - they have no live filesystem to notice the change), but
//! that trade-off doesn't apply to desktop, which never reads this manifest
//! at all.
use std::env;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    let examples_dir = Path::new(&manifest_dir).join("examples");

    let mut names: Vec<String> = fs::read_dir(&examples_dir)
        .expect("examples/ directory must exist")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("adm2") {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();
    names.sort();

    let mut out = String::new();
    out.push_str("/// `(name, embedded source)` pairs for every `examples/*.adm2` file,\n");
    out.push_str("/// sorted by name. Used on platforms/builds with no live filesystem to\n");
    out.push_str("/// read from at runtime: the web build, and tests (a fixture that\n");
    out.push_str("/// doesn't depend on desktop asset bundling being available). Desktop\n");
    out.push_str("/// reads both the list and each file's content live from disk instead -\n");
    out.push_str("/// see `available_examples`/`load_example_source`.\n");
    out.push_str("#[cfg(any(not(feature = \"desktop\"), test))]\n");
    out.push_str("pub static EXAMPLES_WITH_SOURCE: &[(&str, &str)] = &[\n");
    for name in &names {
        let abs_path = examples_dir.join(format!("{name}.adm2"));
        let abs_path_str = abs_path.display().to_string();
        out.push_str(&format!(
            "    ({name:?}, include_str!({abs_path_str:?})),\n"
        ));
    }
    out.push_str("];\n");

    let out_dir = env::var("OUT_DIR").expect("set by cargo");
    fs::write(Path::new(&out_dir).join("example_manifest.rs"), out).expect("write example_manifest.rs");
}
```

- [ ] **Step 3: Delete `begin/src/demo_source.rs` and create `begin/src/example_source.rs`**

```rust
//! Loads adam-lang example sources and builds a [`Sheet`]/[`Labels`] pair from
//! them. `begin` ships with several example property models (see
//! `examples/*.adm2`); [`available_examples`] lists them and
//! [`load_example_source`] loads any one of them by name.
//!
//! `toy_example.adm2` demonstrates two independent bidirectional constraint
//! systems (`a × b = c` and `d × e = f`) linked by one conditional on `p`:
//!
//! - `p = 0`: the relationship `c = f` (bidirectional) becomes active.
//! - `p = 1`: the relationship `c = f × 2` (bidirectional) becomes active, alongside a
//!   second, independent relationship `g = c × 10` in the same branch — `g` is *forced*
//!   while this branch is active (see [`adam_rs::Sheet::is_forced`]), so its
//!   Inspector field is disabled and it is highlighted in the graph.
//! - Any other `p`: the two systems are independent and `g` is not forced.
//!
//! `g`'s relationship is its own `relationship { .. }` block within the `1i32` branch,
//! not folded into the `c`/`f` relationship's methods: a relationship's forced outputs
//! are the *intersection* of its methods' pure outputs, so mixing `[c] -> [g]` in with
//! the `c`/`f` methods would make that intersection empty, forcing nothing. A single
//! `conditional` branch can hold any number of `relationship` blocks, each contributing
//! its own independent forced-output set while that branch is active.

use adam_lang::{AdamParser, TypeRegistry};
use adam_rs::Sheet;
use annotate_snippets::Renderer;
use dioxus::prelude::*;

use crate::bridge::{Labels, format_adam_error, labels_from_cell_names};

// Generated by build.rs from `examples/*.adm2`: an `EXAMPLES_WITH_SOURCE: &[(&str,
// &str)]` array of (name, embedded source) pairs (non-desktop and test builds only -
// desktop reads both the list and each file's content live from disk instead, see
// `available_examples`/`load_example_source` below). See build.rs's own doc comment
// for why the embedded source is gated out of ordinary desktop builds.
include!(concat!(env!("OUT_DIR"), "/example_manifest.rs"));

/// Identifies where an [`ActiveSource`]'s content came from: a bundled example
/// discovered under `examples/`, or a file the user opened via the "Open…" action.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum SourceOrigin {
    /// One of [`available_examples`], loaded from `begin/examples/`.
    #[default]
    Example,
    /// A file opened via the "Open…" action.
    ///
    /// Desktop: the real absolute filesystem path, stored losslessly — not a
    /// `Path::display()`-formatted `String`, which replaces invalid UTF-8
    /// with U+FFFD on Unix and so can't be round-tripped back into the exact
    /// same `Path`, breaking reload/re-read for non-UTF-8 paths. Web: the
    /// picked file's name only — browsers never expose a real filesystem
    /// path, and a browser-supplied name is always valid UTF-8 anyway.
    Opened(std::ffi::OsString),
}

/// The currently active source: its display name, full text, and where it
/// came from (a bundled example or a user-opened file) — see [`SourceOrigin`].
#[derive(Clone, Default)]
pub struct ActiveSource {
    /// Display label: the example's name (its filename stem) or the opened
    /// file's name.
    pub name: String,
    /// The source's full adam-lang source text.
    pub text: String,
    /// Where this source came from.
    pub origin: SourceOrigin,
}

impl ActiveSource {
    /// The path shown in diagnostic headers: `begin/examples/<name>.adm2` for a
    /// bundled example, or the opened file's real path/name directly (lossily
    /// converted to UTF-8 for this display string only — the stored
    /// `OsString` itself stays lossless for reload; see [`SourceOrigin::Opened`]).
    pub fn file_name(&self) -> String {
        match &self.origin {
            SourceOrigin::Example => format!("begin/examples/{}.adm2", self.name),
            SourceOrigin::Opened(path) => path.to_string_lossy().into_owned(),
        }
    }
}

/// This crate's bundled `examples/` directory, resolved relative to
/// `CARGO_MANIFEST_DIR` so it's found regardless of the process's current
/// working directory.
///
/// - Complexity: O(1).
#[cfg(feature = "desktop")]
fn examples_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples")
}

/// Lists every `.adm2` file directly under `dir`, by name (filename stem),
/// sorted. A missing or unreadable `dir` yields an empty list rather than an
/// error — [`available_examples`] has nothing sensible to fall back to.
///
/// - Complexity: O(n log n) in the number of entries under `dir`.
#[cfg(feature = "desktop")]
fn scan_examples_dir(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("adm2") {
                        path.file_stem()
                            .and_then(|s| s.to_str())
                            .map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Lists every example bundled with `begin`, by name, sorted. A picker UI
/// builds its options from this; see [`load_example_source`] for how each
/// name's content actually gets loaded.
///
/// - Complexity: on desktop, O(n log n) in the number of files under
///   `examples/` — a fresh directory scan on every call, so the result
///   always reflects the current directory contents (not cached). Otherwise
///   O(n log n) in [`EXAMPLES_WITH_SOURCE`]'s length.
pub fn available_examples() -> Vec<String> {
    #[cfg(feature = "desktop")]
    {
        scan_examples_dir(&examples_dir())
    }
    #[cfg(not(feature = "desktop"))]
    {
        let mut names: Vec<String> = EXAMPLES_WITH_SOURCE
            .iter()
            .map(|&(name, _)| name.to_string())
            .collect();
        names.sort();
        names
    }
}

/// The result of parsing and building a sheet from adam-lang source.
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

/// Parses `source` as adam-lang, builds a `Sheet` and `Labels`, and propagates
/// once so initial derived values are populated. `file_name` is used only to
/// build diagnostic headers (e.g. `--> begin/examples/toy_example.adm2:8:11`),
/// not to locate `source` itself.
///
/// - Complexity: O(n) in the length of `source` plus the cost of one `propagate()`.
pub fn build_sheet(source: &str, file_name: &str) -> BuildOutcome {
    let mut parser = AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new());
    let mut parsed = match parser.parse_str(source) {
        Ok(p) => p,
        Err(e) => {
            let msg = e.format_rustc_style(source, file_name, 1, &Renderer::styled());
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
            let msg = format_adam_error(&e, source, file_name);
            BuildOutcome {
                sheet_labels: Some((parsed.sheet, labels)),
                error: Some(msg),
            }
        }
    }
}

/// Reads example `name`'s source directly from its location in the crate source tree.
///
/// # Errors
///
/// Returns `Err` if `name` contains a path separator or `..` (rejected before
/// touching the filesystem, so it can't resolve outside `examples/`), or if
/// the file cannot be read (e.g. a transient race with an editor's save).
#[cfg(feature = "desktop")]
pub fn load_example_source(name: &str) -> Result<String, String> {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!("invalid example name {name:?}"));
    }
    let path = examples_dir().join(format!("{name}.adm2"));
    std::fs::read_to_string(&path).map_err(|e| format!("failed to read {}: {e}", path.display()))
}

/// Non-desktop fallback: looks `name` up in the compile-time embedded
/// [`EXAMPLES_WITH_SOURCE`], with no live reload.
///
/// # Errors
///
/// Returns `Err` if `name` doesn't match any embedded example.
#[cfg(not(feature = "desktop"))]
pub fn load_example_source(name: &str) -> Result<String, String> {
    EXAMPLES_WITH_SOURCE
        .iter()
        .find(|&&(n, _)| n == name)
        .map(|&(_, source)| source.to_string())
        .ok_or_else(|| format!("no embedded example named {name:?}"))
}

/// Watches `dir` for changes, calling `on_change` on every filesystem event —
/// a file created, removed, or modified directly under `dir`. The returned
/// watcher must be kept alive for as long as the watch should remain active —
/// dropping it stops watching.
///
/// # Errors
///
/// Returns `Err` if the underlying OS watch could not be established (e.g.
/// `dir` doesn't exist).
#[cfg(feature = "desktop")]
fn spawn_watch_on_dir(
    dir: std::path::PathBuf,
    mut on_change: impl FnMut() + Send + 'static,
) -> notify::Result<notify::RecommendedWatcher> {
    use notify::Watcher;

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            on_change();
        }
    })?;
    watcher.watch(&dir, notify::RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

/// Watches `begin/examples/` for changes — a file added, removed, or edited —
/// calling `on_change` on every event. The returned watcher must be kept
/// alive for as long as the watch should remain active.
///
/// Replaces this crate's previous `dx-serve`-devserver-dependent hot reload:
/// this watches the real filesystem directly, so it reacts to *any* change
/// (including a file appearing or disappearing) and works whether or not
/// `dx serve` happens to be running.
///
/// # Errors
///
/// Returns `Err` if the underlying OS watch could not be established.
#[cfg(feature = "desktop")]
pub fn spawn_examples_watch(
    on_change: impl FnMut() + Send + 'static,
) -> notify::Result<notify::RecommendedWatcher> {
    spawn_watch_on_dir(examples_dir(), on_change)
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
        let outcome = build_sheet(VALID_SOURCE, "test.adm2");
        assert!(outcome.sheet_labels.is_some());
        assert!(outcome.error.is_none());
    }

    #[test]
    fn build_sheet_parse_error_has_no_sheet_and_formatted_message() {
        let outcome = build_sheet("sheet s { cell x }", "test.adm2");
        assert!(outcome.sheet_labels.is_none());
        let msg = outcome.error.expect("expected a parse error message");
        assert!(msg.contains("error"), "{msg}");
    }

    #[test]
    fn build_sheet_runtime_error_still_returns_sheet_and_message() {
        let source = "sheet s { cell x: i32 = 0; cell y: i32; relationship { method [x] -> [y] { 10i32 / x } } }";
        let outcome = build_sheet(source, "test.adm2");
        assert!(
            outcome.sheet_labels.is_some(),
            "sheet should still be built after a propagate error"
        );
        assert!(outcome.error.is_some());
    }

    #[test]
    fn every_bundled_example_parses_successfully() {
        for &(name, source) in EXAMPLES_WITH_SOURCE {
            let outcome = build_sheet(source, &format!("{name}.adm2"));
            assert!(
                outcome.sheet_labels.is_some(),
                "{name} failed to build: {:?}",
                outcome.error
            );
        }
    }

    #[test]
    #[cfg(feature = "desktop")]
    fn load_example_source_rejects_name_containing_parent_dir_reference() {
        // Resolves (via `..`) to the same real file as "toy_example", but must
        // still be rejected rather than trusting whatever `fs::read_to_string`
        // happens to resolve.
        let sneaky = "toy_example/../toy_example";
        assert!(load_example_source(sneaky).is_err());
    }

    #[test]
    #[cfg(feature = "desktop")]
    fn load_example_source_rejects_name_containing_path_separator() {
        assert!(load_example_source("sub/toy_example").is_err());
        assert!(load_example_source("sub\\toy_example").is_err());
    }

    #[test]
    fn available_examples_is_sorted_and_nonempty() {
        let examples = available_examples();
        assert!(!examples.is_empty());
        let mut sorted = examples.clone();
        sorted.sort();
        assert_eq!(examples, sorted);
    }

    #[test]
    #[cfg(feature = "desktop")]
    fn scan_examples_dir_reflects_current_directory_contents() {
        let dir = std::env::temp_dir().join("begin_example_source_test_scan_examples_dir");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("zeta.adm2"), "sheet s {}").unwrap();
        std::fs::write(dir.join("alpha.adm2"), "sheet s {}").unwrap();
        std::fs::write(dir.join("ignored.txt"), "not an example").unwrap();

        let names = scan_examples_dir(&dir);

        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(names, vec!["alpha".to_string(), "zeta".to_string()]);
    }

    #[test]
    fn active_source_file_name_example_matches_convention() {
        let active = ActiveSource {
            name: "toy_example".to_string(),
            text: String::new(),
            origin: SourceOrigin::Example,
        };
        assert_eq!(active.file_name(), "begin/examples/toy_example.adm2");
    }

    #[test]
    fn active_source_file_name_opened_returns_path_directly() {
        let active = ActiveSource {
            name: "my_model".to_string(),
            text: String::new(),
            origin: SourceOrigin::Opened("/home/user/models/my_model.adm2".into()),
        };
        assert_eq!(active.file_name(), "/home/user/models/my_model.adm2");
    }

    #[test]
    fn active_source_file_name_opened_is_lossy_for_non_utf8_path() {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            // 0xFF is not valid UTF-8 on its own; to_string_lossy() must
            // replace it with U+FFFD rather than panicking or truncating.
            let non_utf8 = std::ffi::OsString::from_vec(vec![0xFF, b'.', b'a', b'd', b'm', b'2']);
            let active = ActiveSource {
                name: "weird".to_string(),
                text: String::new(),
                origin: SourceOrigin::Opened(non_utf8),
            };
            assert_eq!(active.file_name(), "\u{FFFD}.adm2");
        }
    }
}

#[cfg(all(test, feature = "desktop"))]
mod watch_tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn spawn_watch_on_dir_fires_on_new_file() {
        let dir = std::env::temp_dir().join("begin_example_source_test_watch_new_file");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();

        let (tx, rx) = mpsc::channel::<()>();
        let _watcher = spawn_watch_on_dir(dir.clone(), move || {
            let _ = tx.send(());
        })
        .unwrap();

        std::fs::write(dir.join("new_example.adm2"), "sheet s {}").unwrap();

        let fired = rx.recv_timeout(Duration::from_secs(5)).is_ok();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            fired,
            "watcher should fire when a file is created in the watched directory"
        );
    }

    #[test]
    fn spawn_watch_on_dir_fires_on_removed_file() {
        let dir = std::env::temp_dir().join("begin_example_source_test_watch_removed_file");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("goes_away.adm2");
        std::fs::write(&file, "sheet s {}").unwrap();

        let (tx, rx) = mpsc::channel::<()>();
        let _watcher = spawn_watch_on_dir(dir.clone(), move || {
            let _ = tx.send(());
        })
        .unwrap();

        std::fs::remove_file(&file).unwrap();

        let fired = rx.recv_timeout(Duration::from_secs(5)).is_ok();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            fired,
            "watcher should fire when a file is removed from the watched directory"
        );
    }
}
```

- [ ] **Step 4: Update `begin/src/main.rs:4`**

Change:
```rust
mod demo_source;
```
to:
```rust
mod example_source;
```

- [ ] **Step 5: Update `begin/src/inspector.rs:20` and `:41`**

Change both occurrences of:
```rust
    active_source: Signal<crate::demo_source::ActiveSource>,
```
to:
```rust
    active_source: Signal<crate::example_source::ActiveSource>,
```

- [ ] **Step 6: Update `begin/src/app.rs`**

Change the top-level `use` block (currently):
```rust
use crate::demo_source::{
    ActiveSource, SourceOrigin, available_demos, build_sheet, load_demo_source,
};
```
to:
```rust
use crate::example_source::{
    ActiveSource, SourceOrigin, available_examples, build_sheet, load_example_source,
};
```

Update `App`'s doc comment (currently lines 14–26) to:
```rust
/// Root component: Spectrum theme wrapper with an examples picker, the graph, and
/// the Inspector filling the viewport. `begin` ships with several example
/// property models (`begin/examples/*.adm2` — see
/// [`crate::example_source::available_examples`]); [`ExamplesPicker`] switches
/// which one is loaded. On desktop, editing the *currently selected* example's
/// file, or adding/removing a file under `begin/examples/`, live-updates this
/// running app via [`crate::example_source::spawn_examples_watch`], exactly as
/// if the old Apply button had been pressed.
///
/// A read or parse failure loading an example does not prevent the app from
/// launching or switching: it prints the diagnostic to stderr and falls back
/// to an empty sheet instead, so a syntax error can be fixed and
/// hot-reloaded in without restarting.
```

Inside `App`, change:
```rust
    let initial_demo_name = available_demos().first().copied().unwrap_or_default();
    let (initial_sheet, initial_labels, initial_active_source) = load_demo(initial_demo_name);
    let sheet = use_signal(|| initial_sheet);
    let labels = use_signal(|| initial_labels);
    let active_source = use_signal(|| initial_active_source);
```
to:
```rust
    let initial_example_name = available_examples().first().cloned().unwrap_or_default();
    let (initial_sheet, initial_labels, initial_active_source) = load_example(&initial_example_name);
    let sheet = use_signal(|| initial_sheet);
    let labels = use_signal(|| initial_labels);
    let active_source = use_signal(|| initial_active_source);
    let example_names = use_signal(|| available_examples());
```

Change the `#[cfg(feature = "desktop")] let reload_tx = { ... }` block from:
```rust
    #[cfg(feature = "desktop")]
    let reload_tx: Signal<futures_channel::mpsc::UnboundedSender<()>> = {
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
                    let current = active_source.read().clone();
                    let loaded = match &current.origin {
                        SourceOrigin::Demo => {
                            eprintln!("loading begin/assets/{}.adm2", current.name);
                            load_demo_source(&current.name)
                        }
                        SourceOrigin::Opened(path) => {
                            eprintln!("loading {}", path.to_string_lossy());
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
            });
            Signal::new(tx)
        })
    };
```
to:
```rust
    #[cfg(feature = "desktop")]
    let reload_tx: Signal<futures_channel::mpsc::UnboundedSender<()>> = {
        let mut sheet = sheet;
        let mut labels = labels;
        let mut active_source = active_source;
        let mut example_names = example_names;
        use_hook(move || {
            let (tx, mut rx) = futures_channel::mpsc::unbounded::<()>();
            if let Err(err) = crate::example_source::spawn_examples_watch({
                let tx = tx.clone();
                move || {
                    let _ = tx.unbounded_send(());
                }
            }) {
                eprintln!("failed to watch begin/examples/: {err}");
            }
            spawn(async move {
                use futures_util::StreamExt;
                while rx.next().await.is_some() {
                    example_names.set(crate::example_source::available_examples());
                    let current = active_source.read().clone();
                    let loaded = match &current.origin {
                        SourceOrigin::Example => {
                            eprintln!("loading begin/examples/{}.adm2", current.name);
                            load_example_source(&current.name)
                        }
                        SourceOrigin::Opened(path) => {
                            eprintln!("loading {}", path.to_string_lossy());
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
            });
            Signal::new(tx)
        })
    };
```

Rename (comments and identifiers, logic unchanged) both `on_demo_selected` bindings to `on_example_selected` — one in the `#[cfg(feature = "desktop")]` block, one in the `#[cfg(not(feature = "desktop"))]` block. Also update the three comments above/around `watcher_slot` that mention `DemoPicker`/`on_demo_selected` to say `ExamplesPicker`/`on_example_selected`.

Change the final `rsx!` block's:
```rust
                DemoPicker { sheet, labels, active_source, on_select: on_demo_selected }
```
to:
```rust
                ExamplesPicker { sheet, labels, active_source, example_names, on_select: on_example_selected }
```

Rename `load_demo` to `load_example` and update its body:
```rust
/// Loads example `name`, builds its sheet, and returns it alongside the
/// [`ActiveSource`] describing what just loaded.
///
/// A read or parse failure prints the diagnostic to stderr and returns an
/// empty sheet instead of failing — see [`App`]'s doc comment for why. The
/// returned [`ActiveSource`] still carries `name` (and, if the read
/// succeeded, the source text that failed to parse) even on failure, so the
/// desktop hot-reload loop keeps reloading the right file and can recover
/// once the on-disk error is fixed, instead of losing track of which
/// example was selected.
///
/// - Complexity: O(n) in the length of the example's source, plus the cost
///   of one `build_sheet` parse/propagate.
fn load_example(name: &str) -> (Sheet, Labels, ActiveSource) {
    match load_example_source(name) {
        Ok(source) => {
            let outcome = build_sheet(&source, &format!("begin/examples/{name}.adm2"));
            if let Some(err) = &outcome.error {
                eprintln!("{err}");
            }
            let active_source = ActiveSource {
                name: name.to_string(),
                text: source,
                origin: SourceOrigin::Example,
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
                    name: name.to_string(),
                    text: String::new(),
                    origin: SourceOrigin::Example,
                },
            )
        }
    }
}
```

Rename `DemoPicker` to `ExamplesPicker` — same widget for now (Task 2 replaces the body):
```rust
/// Picker row listing every example from `example_names`; clicking one
/// loads it into `sheet`/`labels`/`active_source`, highlighting whichever
/// name matches `active_source`'s current value, then calls `on_select` —
/// on desktop, `App` uses this to clear any watcher left over from a
/// previously opened file (see `App`'s `on_example_selected`).
#[component]
fn ExamplesPicker(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<ActiveSource>,
    example_names: Signal<Vec<String>>,
    on_select: Callback<()>,
) -> Element {
    let is_example_active = matches!(active_source.read().origin, SourceOrigin::Example);
    let current = active_source.read().name.clone();

    rsx! {
        div {
            style: "padding: 8px 12px; border-bottom: 1px solid #ccc; flex: none;",
            SpActionGroup {
                compact: true,
                for name in example_names.read().iter().cloned() {
                    SpActionButton {
                        key: "{name}",
                        selected: is_example_active && name == current,
                        onclick: {
                            let mut sheet = sheet;
                            let mut labels = labels;
                            let mut active_source = active_source;
                            let name = name.clone();
                            move |_| {
                                let (new_sheet, new_labels, new_active_source) = load_example(&name);
                                sheet.set(new_sheet);
                                labels.set(new_labels);
                                active_source.set(new_active_source);
                                on_select.call(());
                            }
                        },
                        "{name}"
                    }
                }
            }
        }
    }
}
```

In the `#[cfg(test)] mod tests` block, update `toy_example_source()`:
```rust
    fn toy_example_source() -> &'static str {
        crate::example_source::EXAMPLES_WITH_SOURCE
            .iter()
            .find(|&&(name, _)| name == "toy_example")
            .map(|&(_, source)| source)
            .expect("toy_example.adm2 must be bundled")
    }
```

Rename the three tests `demo_source_g_not_forced_when_p_is_zero`, `demo_source_g_forced_when_p_is_one`, and `demo_source_g_unforced_again_after_p_returns_to_zero` to `toy_example_g_not_forced_when_p_is_zero`, `toy_example_g_forced_when_p_is_one`, and `toy_example_g_unforced_again_after_p_returns_to_zero` respectively (bodies unchanged).

Rename `load_demo_unknown_name_falls_back_to_empty_sheet` to `load_example_unknown_name_falls_back_to_empty_sheet` and change its body's call from `load_demo("does_not_exist")` to `load_example("does_not_exist")`.

- [ ] **Step 7: Remove the `dioxus-devtools` dependency from `begin/Cargo.toml`**

Change:
```toml
[dependencies]
dioxus = { version = "0.7.10", features = [] }
dioxus-devtools = "0.7.10"
futures-channel = "0.3"
```
to:
```toml
[dependencies]
dioxus = { version = "0.7.10", features = [] }
futures-channel = "0.3"
```

- [ ] **Step 8: Run the test suite and verify everything passes**

Run: `cargo test -p begin`
Expected: all tests pass, including the new `example_source::tests::every_bundled_example_parses_successfully` (now covering all 5 examples), `scan_examples_dir_reflects_current_directory_contents`, and `watch_tests::*`. If `every_bundled_example_parses_successfully` fails for `diamond.adm2`, `diamond-wing.adm2`, or `out-cell.adm2`, read the printed diagnostic and fix that `.adm2` file's syntax to match the current adam-lang grammar — these files predate this task and haven't been exercised by `begin`'s parser before now.

- [ ] **Step 9: Sanity-check the non-desktop build path**

Run: `cargo check -p begin --no-default-features`
Expected: compiles cleanly (exercises the `#[cfg(not(feature = "desktop"))]` branches of `example_source.rs` and `app.rs`).

- [ ] **Step 10: Format and commit**

```bash
cargo fmt --all
git add begin/examples begin/assets begin/build.rs begin/src/example_source.rs begin/src/main.rs begin/src/inspector.rs begin/src/app.rs begin/Cargo.toml
git status
```
Confirm `begin/src/demo_source.rs` shows as deleted and `begin/src/example_source.rs` as added (git usually detects this as a rename), then:
```bash
git commit -m "$(cat <<'EOF'
refactor(begin): rename demo->example, merge into begin/examples/, live directory scan + watcher

Renames every demo-flavored identifier to example, moves the two
assets/*.adm2 files into begin/examples/ alongside the three curated
examples already there, and replaces the build-time file list plus
dx-serve-dependent hot reload with a runtime directory scan and a
notify-based filesystem watcher on desktop. The picker UI itself is
unchanged in this commit (still the SpActionGroup tab row) - only
what feeds it changed.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Replace the tab row with a scrollable sidebar list

**Files:**
- Modify: `begin/src/spectrum.rs` (add `SpSideNav`, `SpSideNavItem`)
- Modify: `begin/src/app.rs` (imports, `ExamplesPicker` body, layout)

**Interfaces:**
- Consumes: `crate::example_source::{ActiveSource, SourceOrigin, available_examples}` and `app::load_example` (from Task 1) — unchanged signatures.
- Produces: `crate::spectrum::SpSideNav { children: Element }`, `crate::spectrum::SpSideNavItem { label: String, onclick: EventHandler<MouseEvent>, selected: bool (default false) }`.

- [ ] **Step 1: Add `SpSideNav`/`SpSideNavItem` to `begin/src/spectrum.rs`**

Insert after the `SpActionButton` component (before `SpIconZoomIn`):

```rust
/// A scrollable, selectable vertical list of items — used as the examples
/// picker sidebar.
///
/// Maps to `<sp-sidenav>`.
#[component]
pub fn SpSideNav(children: Element) -> Element {
    rsx! {
        sp-sidenav {
            {children}
        }
    }
}

/// A single item within an `SpSideNav`.
///
/// Maps to `<sp-sidenav-item>`. `label` sets the item's visible text (via the
/// element's `label` attribute, not slotted content); `selected` renders it
/// in its highlighted/active state (e.g. to mark the current choice in a
/// list used as a picker).
#[component]
pub fn SpSideNavItem(
    label: String,
    onclick: EventHandler<MouseEvent>,
    #[props(default)] selected: bool,
) -> Element {
    rsx! {
        sp-sidenav-item {
            "label": "{label}",
            onclick: move |e| onclick.call(e),
            // Boolean attribute: omit entirely when false; presence = selected.
            "selected": if selected { "true" },
        }
    }
}
```

- [ ] **Step 2: Update `begin/src/app.rs`'s spectrum import**

Change:
```rust
use crate::spectrum::{SpActionButton, SpActionGroup, SpTheme};
```
to:
```rust
use crate::spectrum::{
    SpActionButton, SpActionGroup, SpDivider, SpHeading, SpSideNav, SpSideNavItem, SpTheme,
};
```

- [ ] **Step 3: Rewrite `ExamplesPicker`'s body in `begin/src/app.rs`**

Replace the whole `ExamplesPicker` function (added in Task 1) with:

```rust
/// Sidebar panel listing every example from `example_names`; clicking one
/// loads it into `sheet`/`labels`/`active_source`, highlighting whichever
/// name matches `active_source`'s current value, then calls `on_select` —
/// on desktop, `App` uses this to clear any watcher left over from a
/// previously opened file (see `App`'s `on_example_selected`). Scrolls
/// internally once the list outgrows the panel's height, so the list can
/// grow arbitrarily without crowding the rest of the window.
#[component]
fn ExamplesPicker(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<ActiveSource>,
    example_names: Signal<Vec<String>>,
    on_select: Callback<()>,
) -> Element {
    let is_example_active = matches!(active_source.read().origin, SourceOrigin::Example);
    let current = active_source.read().name.clone();

    rsx! {
        div {
            style: "width: 260px; min-width: 260px; height: 100%; overflow-y: auto; padding: 12px; box-sizing: border-box; border-right: 1px solid #ccc;",
            SpHeading { "Examples" }
            SpDivider {}
            SpSideNav {
                for name in example_names.read().iter().cloned() {
                    SpSideNavItem {
                        key: "{name}",
                        label: name.clone(),
                        selected: is_example_active && name == current,
                        onclick: {
                            let mut sheet = sheet;
                            let mut labels = labels;
                            let mut active_source = active_source;
                            let name = name.clone();
                            move |_| {
                                let (new_sheet, new_labels, new_active_source) = load_example(&name);
                                sheet.set(new_sheet);
                                labels.set(new_labels);
                                active_source.set(new_active_source);
                                on_select.call(());
                            }
                        },
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 4: Move `ExamplesPicker` into the main flex row in `App`'s `rsx!` block**

Change:
```rust
            div {
                style: "position: fixed; inset: 0; display: flex; flex-direction: column; overflow: hidden;",
                ExamplesPicker { sheet, labels, active_source, example_names, on_select: on_example_selected }
                {open_file_controls}
                div {
                    style: "flex: 1; display: flex; overflow: hidden; min-height: 0;",
                    GraphView { data: graph_data, source_id }
                    Inspector { sheet, labels, active_source }
                }
            }
```
to:
```rust
            div {
                style: "position: fixed; inset: 0; display: flex; flex-direction: column; overflow: hidden;",
                {open_file_controls}
                div {
                    style: "flex: 1; display: flex; overflow: hidden; min-height: 0;",
                    ExamplesPicker { sheet, labels, active_source, example_names, on_select: on_example_selected }
                    GraphView { data: graph_data, source_id }
                    Inspector { sheet, labels, active_source }
                }
            }
```

- [ ] **Step 5: Build and run the test suite**

Run: `cargo test -p begin`
Expected: all tests still pass (this task touches only rendering, not `example_source.rs`'s logic).

Run: `cargo clippy -p begin --all-targets -- -D warnings` and `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Manually verify the rendered UI**

Use the `verifying-begin-ui` skill to serve `begin` and screenshot/inspect the DOM. Confirm:
- The old horizontal tab row is gone.
- A left sidebar labeled "Examples" is present, listing all 5 examples (`diamond`, `diamond-wing`, `image_resize`, `out-cell`, `toy_example`, sorted).
- The sidebar has its own scrollbar / `overflow-y: auto` in computed styles (shrink the viewport height in the check if needed to confirm it scrolls rather than pushing other panels around).
- Clicking a different example switches the graph and highlights that item as selected.
- `GraphView` and `Inspector` still render correctly alongside the new sidebar.

If anything doesn't render as expected (e.g. `sp-sidenav-item`'s `label` attribute doesn't render text the way assumed), fix `SpSideNavItem` in `spectrum.rs` based on what the DOM inspection actually shows, then re-verify.

- [ ] **Step 7: Format and commit**

```bash
cargo fmt --all
git add begin/src/spectrum.rs begin/src/app.rs
git commit -m "$(cat <<'EOF'
feat(begin): replace examples tab row with a scrollable sidebar list

Adds SpSideNav/SpSideNavItem wrappers and moves the examples picker
into a persistent left sidebar (mirroring Inspector's panel on the
right) so the list can grow past a handful of examples without
running out of horizontal space.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Full verification and live-update check

**Files:** none (verification only).

- [ ] **Step 1: Run the full formatting/build/test/clippy suite from `CLAUDE.md`**

```bash
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: zero warnings and zero test failures across all of the above. Fix anything that surfaces before moving on.

- [ ] **Step 2: Manually verify live add/remove on desktop**

Run `begin` as a desktop app (`dx serve --platform desktop` or `cargo run -p begin`, per however this repo normally launches it for manual checks). With it running:
- Copy an existing example to a new name inside `begin/examples/` (e.g. `cp begin/examples/diamond.adm2 begin/examples/diamond-copy.adm2`) and confirm `diamond-copy` appears in the sidebar without restarting the app.
- Delete `begin/examples/diamond-copy.adm2` and confirm it disappears from the sidebar.
- While `diamond.adm2` is the active selection, edit its content on disk and confirm the graph updates live (same as today's hot reload).
- Clean up: `rm -f begin/examples/diamond-copy.adm2` if it's still present.

- [ ] **Step 3: Update the spec's status (optional, if the spec doc tracks status)**

No status field exists in `docs/superpowers/specs/2026-08-11-begin-examples-list-design.md`; nothing to update there. If any follow-up or deferred item was discovered during Steps 1–2, add a short note to that spec file's end under a new `## Follow-ups` heading describing it, then commit:

```bash
git add docs/superpowers/specs/2026-08-11-begin-examples-list-design.md
git commit -m "$(cat <<'EOF'
docs: note follow-ups discovered during begin examples list verification

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```
(Skip this step entirely if nothing was discovered.)
