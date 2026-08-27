# Live Adam Examples in adam-lang-book Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every code example in `adam-lang-book` becomes a standalone `.adm2` file, executed
by the test suite directly (no more Rust-embedded sheet source), and rendered in the built
book as a live, editable `SheetInspector` widget compiled to WASM — extracted from `begin`
into a new reusable `adam-web-ui` crate.

**Architecture:** Extract `begin`'s Sheet/Spectrum UI (`spectrum.rs`, the cell-list rendering
from `inspector.rs`, the `Labels`/diagnostic-formatting/`build_sheet` plumbing from
`bridge.rs`/`example_source.rs`) into a new `adam-web-ui` library crate that `begin` then
depends on. Convert the book's 27 examples from Rust string literals to `.adm2` files loaded
via `include_str!`. Build a thin `#[wasm_bindgen]` crate (`adam-lang-book-live`) on top of
`adam-web-ui` that mounts one independent `SheetInspector` per example. A new mdBook
preprocessor auto-inserts the mount point after each example's `{{#include}}`. CI builds the
wasm bundle before `mdbook build` in both the PR-check and Pages-deploy workflows.

**Tech Stack:** Rust 2024, Dioxus 0.7.10, `wasm-bindgen`, mdBook + a custom
`mdbook::preprocess::Preprocessor`, GitHub Actions.

**Spec:** [docs/superpowers/specs/2026-08-27-live-adam-book-examples-design.md](../specs/2026-08-27-live-adam-book-examples-design.md)

## Global Constraints

- `cargo fmt --all` must pass before every commit (enforced by the pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce **zero compiler
  warnings** (not just zero clippy warnings) — read build/test output, don't just check the
  exit code.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`,
  `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`, and
  `cargo clippy -p begin --all-targets -- -D warnings` must all pass before opening a PR.
- Every public function needs a contract-style `///` doc comment (Summary /
  Preconditions / Postconditions / Complexity as applicable) per this repo's `CLAUDE.md`.
  Unit tests are derived from the contract and public interface only.
- Never commit directly to `main`; this work happens in the `live-book` worktree/branch.
- The graph view (`begin/src/graph_view.rs` and its D3/`graph.js` assets) is explicitly out
  of scope — it stays `begin`-only.
- `expressions/no_standard_library.adm2` is excluded from live mounting: it exists
  specifically to demonstrate behavior *without* `cel-std` installed, which the shared live
  parser (`adam_web_ui::build_sheet`) always installs — mounting it live would silently show
  it succeeding instead of erroring, contradicting the surrounding prose. This exclusion is
  implemented explicitly (a documented list in the preprocessor), not silently.

---

## Phase 1 — Extract `adam-web-ui` from `begin`

### Task 1: Scaffold the `adam-web-ui` crate

**Files:**
- Create: `adam-web-ui/Cargo.toml`
- Create: `adam-web-ui/src/lib.rs`
- Modify: `Cargo.toml:22-34` (workspace `members`)

**Interfaces:**
- Produces: an empty, compiling workspace member named `adam-web-ui` that later tasks add
  modules to.

- [ ] **Step 1: Create `adam-web-ui/Cargo.toml`**

```toml
[package]
name = "adam-web-ui"
version = "0.1.0"
edition = "2024"
description = "Reusable Dioxus + Spectrum Web Components UI for browsing and editing a live adam-rs Sheet"

[features]
desktop = []

[dependencies]
dioxus = { version = "0.7.10", features = [] }
adam-rs = { path = "../adam-rs" }
adam-lang = { path = "../adam-lang" }
cel-parser = { path = "../cel-parser" }
cel-runtime = { path = "../cel-runtime" }
cel-std = { path = "../cel-std" }
annotate-snippets = "0.12"
anyhow = "1"
indexmap = "2"
wasm-bindgen = "0.2"
web-sys = { version = "0.3", features = ["console"] }

[lints]
workspace = true
```

- [ ] **Step 2: Create `adam-web-ui/src/lib.rs`**

```rust
//! Reusable Dioxus UI for browsing and editing a live `adam_rs::Sheet`, built on Spectrum Web
//! Components. Named for the web rendering stack it targets — Dioxus + Spectrum Web
//! Components render as DOM whether hosted in a real browser or an embedded webview — not
//! tied to any one Dioxus renderer feature, so it's usable from a desktop app (`begin`), a
//! `dioxus/web` app, or a plain `wasm-bindgen` embed with no full app shell around it.
```

- [ ] **Step 3: Add `adam-web-ui` to the workspace**

In `Cargo.toml`, change:

```toml
[workspace]
members = [
    "cel-runtime",
    "cel-parser",
    "cel-rs-macros",
    "cel-std",
    "adam-rs",
    "adam-lang",
    "adam-lang-book",
    "adam-lsp",
    "begin",
    "xtask",
]
```

to:

```toml
[workspace]
members = [
    "cel-runtime",
    "cel-parser",
    "cel-rs-macros",
    "cel-std",
    "adam-rs",
    "adam-lang",
    "adam-lang-book",
    "adam-lsp",
    "adam-web-ui",
    "begin",
    "xtask",
]
```

- [ ] **Step 4: Verify it builds**

Run: `cargo build -p adam-web-ui` and `cargo doc -p adam-web-ui --no-deps`
Expected: both succeed with zero warnings.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml adam-web-ui/Cargo.toml adam-web-ui/src/lib.rs
git commit -m "Scaffold the adam-web-ui crate"
```

---

### Task 2: Move `spectrum.rs` into `adam-web-ui`

**Files:**
- Move: `begin/src/spectrum.rs` → `adam-web-ui/src/spectrum.rs` (verbatim, no content changes)
- Modify: `adam-web-ui/src/lib.rs`
- Modify: `begin/src/main.rs:2-9`
- Modify: `begin/src/app.rs:12-15`
- Modify: `begin/src/inspector.rs:7-9` (still lives in `begin` until Task 6)

**Interfaces:**
- Produces: `adam_web_ui::spectrum::{SpTheme, SpTextfield, SpNumberfield, SpCheckbox,
  SpSlider, SpFieldLabel, SpHeading, SpDivider, SpActionGroup, SpActionButton, SpSideNav,
  SpSideNavItem, SpSwitch, SpIconZoomIn, SpIconZoomOut}` (the full existing set, unchanged).

- [ ] **Step 1: Move the file**

```bash
git mv begin/src/spectrum.rs adam-web-ui/src/spectrum.rs
```

- [ ] **Step 2: Declare the module in `adam-web-ui/src/lib.rs`**

Add:

```rust
pub mod spectrum;
```

- [ ] **Step 3: Update `begin/src/main.rs`**

Remove the line `mod spectrum;` from the `mod` list at the top of the file (it now lives in
`adam-web-ui`, not `begin`).

- [ ] **Step 4: Update `begin/src/app.rs`'s import**

Change:

```rust
use crate::spectrum::{
    SpActionButton, SpActionGroup, SpDivider, SpHeading, SpIconZoomIn, SpIconZoomOut, SpSideNav,
    SpSideNavItem, SpSwitch, SpTheme,
};
```

to:

```rust
use adam_web_ui::spectrum::{
    SpActionButton, SpActionGroup, SpDivider, SpHeading, SpIconZoomIn, SpIconZoomOut, SpSideNav,
    SpSideNavItem, SpSwitch, SpTheme,
};
```

- [ ] **Step 5: Update `begin/src/inspector.rs`'s import (temporary — this file itself moves in Task 6)**

Change:

```rust
use crate::spectrum::{
    SpCheckbox, SpDivider, SpFieldLabel, SpHeading, SpNumberfield, SpSlider, SpTextfield,
};
```

to:

```rust
use adam_web_ui::spectrum::{
    SpCheckbox, SpDivider, SpFieldLabel, SpHeading, SpNumberfield, SpSlider, SpTextfield,
};
```

- [ ] **Step 6: Add the `adam-web-ui` dependency to `begin/Cargo.toml`**

In `begin/Cargo.toml`, add under `[dependencies]` (alphabetically near `adam-rs`):

```toml
adam-web-ui = { path = "../adam-web-ui" }
```

- [ ] **Step 7: Verify**

Run: `cargo build -p begin --no-default-features` and `cargo build -p adam-web-ui`
Expected: both succeed with zero warnings. (`begin -p begin` without `--no-default-features`
will also work but pulls in the heavier desktop deps; the renderer-agnostic check is enough
here.)

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "Move begin's Spectrum component wrappers into adam-web-ui"
```

---

### Task 3: Move `diagnostics.rs` into `adam-web-ui`

**Files:**
- Move: `begin/src/diagnostics.rs` → `adam-web-ui/src/diagnostics.rs`
- Modify: `adam-web-ui/src/lib.rs`
- Modify: `begin/src/main.rs` (remove `mod diagnostics;`)
- Modify: `begin/src/app.rs` (7 call sites)
- Modify: `begin/Cargo.toml` (feature forwarding, drop now-unused direct deps)

**Interfaces:**
- Produces: `adam_web_ui::diagnostics::report_error(msg: &str)`, gated by `adam-web-ui`'s own
  `desktop` Cargo feature (mirroring `begin`'s existing feature name/semantics, but as a
  separate feature flag owned by this crate).

- [ ] **Step 1: Move the file, updating its module doc**

```bash
git mv begin/src/diagnostics.rs adam-web-ui/src/diagnostics.rs
```

Replace its module doc comment (the first paragraph, which currently says "in `begin`") with:

```rust
//! Cross-platform diagnostic reporting.
//!
//! `eprintln!` reaches a visible stderr on desktop, but `wasm32-unknown-unknown`'s stdio is
//! a no-op sink — nothing written to it is ever observable, in a devtools console or
//! otherwise. [`report_error`] is the single point every diagnostic raised through this
//! crate's UI should go through instead, so a failure is visible on every platform a
//! consumer of `adam-web-ui` ships on.
//!
//! Gated on `target_arch = "wasm32"` rather than just `feature = "desktop"`: the web-only
//! code paths this feeds are also built and unit-tested on the native host (`cargo test -p
//! adam-web-ui`), and calling the real `web_sys`/`wasm_bindgen` JS FFI outside an actual
//! wasm32 host crashes the process rather than erroring — `eprintln!` is the correct, safe
//! fallback for that native-host case too. A consumer embedding this crate's components into
//! its own desktop (webview) build should enable this crate's `desktop` feature so its
//! diagnostics land on that process's stderr instead of being silently dropped.
```

The two `#[cfg(...)]`-gated `pub fn report_error` bodies below the doc comment are unchanged.

- [ ] **Step 2: Declare the module in `adam-web-ui/src/lib.rs`**

Add:

```rust
pub mod diagnostics;
```

- [ ] **Step 3: Update `begin/src/main.rs`**

Remove the line `mod diagnostics;`.

- [ ] **Step 4: Update `begin/Cargo.toml`**

Change the `desktop` feature to forward to `adam-web-ui`:

```toml
desktop = ["dioxus/desktop", "dep:rfd", "dep:notify", "adam-web-ui/desktop"]
```

Remove the now-directly-unused `wasm-bindgen` and `web-sys` dependency lines (nothing left in
`begin/src/*.rs` calls them directly once `diagnostics.rs` has moved — verify with
`grep -rn "web_sys\|wasm_bindgen" begin/src` before removing; it should return no results).

- [ ] **Step 5: Update all 7 call sites in `begin/src/app.rs`**

Replace every occurrence of `crate::diagnostics::report_error` with
`adam_web_ui::diagnostics::report_error` (lines 321, 334, 476, 512, 521, 549, 558 as of this
plan's writing — confirm exact line numbers with
`grep -n "crate::diagnostics::report_error" begin/src/app.rs` before editing, since earlier
tasks may have shifted them slightly).

- [ ] **Step 6: Verify**

Run: `cargo build -p begin --no-default-features`, `cargo build -p begin --features web
--no-default-features`, `cargo build -p adam-web-ui`
Expected: all succeed with zero warnings.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "Move begin's diagnostic reporting into adam-web-ui"
```

---

### Task 4: Extract `Labels`/`format_adam_error`/`format_rounded` into `adam-web-ui/src/labels.rs`

**Files:**
- Create: `adam-web-ui/src/labels.rs`
- Modify: `begin/src/bridge.rs` (delete lines 1–278 and the moved tests; adjust the `use`
  block that remains)
- Modify: `adam-web-ui/src/lib.rs`
- Modify: `begin/src/example_source.rs:28` (import)

**Interfaces:**
- Consumes: `adam_rs::{CellId, Error, Sheet}`, `adam_lang::type_registry::TypeShape`,
  `cel_parser::FormatRustcStyle`, `annotate_snippets::Renderer`, `indexmap::IndexMap`.
- Produces: `adam_web_ui::labels::{WriteStrFn, CellMeta, Labels, format_rounded,
  labels_from_cell_names, format_adam_error}` — identical public signatures to today's
  `begin::bridge` versions, just relocated.

- [ ] **Step 1: Create `adam-web-ui/src/labels.rs`**

```rust
//! Cell display/write metadata and diagnostic formatting for a live [`adam_rs::Sheet`].
//!
//! [`Labels`] associates display metadata (names, type-erased display and write closures)
//! with stable [`CellId`] keys, driving [`crate::SheetInspector`]'s rendering.
//! [`format_adam_error`] formats an [`adam_rs::Error`] as a rustc-style diagnostic when
//! possible.

use adam_lang::type_registry::TypeShape;
use adam_rs::{CellId, Error, Sheet};
use annotate_snippets::Renderer;
use cel_parser::FormatRustcStyle;
use indexmap::IndexMap;
use std::any::TypeId;

/// Type-erased write closure: parses a string and writes it to a cell.
pub type WriteStrFn = Box<dyn Fn(&mut Sheet, &str) -> Result<(), Error>>;

/// Display and write metadata for a single cell.
pub struct CellMeta {
    /// Human-readable cell name shown in the inspector.
    pub label: String,
    /// `true` if the cell holds a `bool`, so [`crate::SheetInspector`] can render it as a
    /// checkbox instead of a text field.
    pub is_bool: bool,
    /// `true` if the cell holds one of the 14 numeric primitive types, so
    /// [`crate::SheetInspector`] can render it with [`crate::spectrum::SpNumberfield`]
    /// instead of a plain text field.
    pub is_numeric: bool,
    /// Returns the current cell value as a display string.
    pub display: Box<dyn Fn(&Sheet) -> String>,
    /// Parses `s` and writes the result to the cell; returns `Err` on parse failure or type
    /// mismatch. May also always return `Err` for a cell type with no write support yet (e.g.
    /// tuples — see [`Labels::add_tuple_cell`]).
    pub write_str: WriteStrFn,
    /// Live slider bounds, present only for a numeric cell whose filter is a
    /// [`adam_rs::FilterKind::Range`] — recomputed from the filter's current argument values on
    /// every call, so a range driven by other cells or relationships stays live. Cast to `f64`
    /// for display, matching [`format_rounded`]'s existing all-numeric-types-as-`f64` convention.
    #[allow(clippy::type_complexity)]
    pub range: Option<Box<dyn Fn(&Sheet) -> (f64, f64)>>,
}

/// Associates human-readable labels and type-erased closures with stable sheet IDs.
pub struct Labels {
    /// Cells in insertion order (preserves display ordering).
    pub cells: IndexMap<CellId, CellMeta>,
}

impl Labels {
    /// Creates an empty label set.
    pub fn new() -> Self {
        Self {
            cells: IndexMap::new(),
        }
    }

    /// Registers display metadata for a cell of type `T`.
    ///
    /// - Precondition: `id` is a live cell in the sheet this `Labels` will be used with.
    /// - Precondition: `T` matches the type registered with `Sheet::add_cell` for `id`.
    pub fn add_cell<T>(&mut self, id: CellId, label: &str)
    where
        T: std::any::Any + std::fmt::Display + std::str::FromStr + 'static,
        T::Err: std::fmt::Display,
    {
        self.cells.insert(
            id,
            CellMeta {
                label: label.to_owned(),
                is_bool: TypeId::of::<T>() == TypeId::of::<bool>(),
                is_numeric: false,
                display: Box::new(move |sheet| {
                    sheet
                        .read::<T>(id)
                        .map(|v| format!("{}", v))
                        .unwrap_or_else(|_| "?".to_owned())
                }),
                write_str: Box::new(move |sheet, s| {
                    let value = s
                        .parse::<T>()
                        .map_err(|e| Error::MethodFailed(anyhow::anyhow!("parse error: {}", e)))?;
                    sheet.write(id, value)
                }),
                range: None,
            },
        );
    }

    /// Registers display-only metadata for a tuple-typed cell of any shape.
    ///
    /// `write_str` always returns `Err` — no tuple-literal parser exists yet (tracked as a
    /// follow-up: see the "Support editing tuple-typed cells in `begin`" GitHub issue). The
    /// field still participates fully in [`crate::SheetInspector`]'s existing
    /// invalid/warning/disabled machinery, since that's entirely keyed on `CellId`, not on any
    /// per-type behavior.
    ///
    /// - Precondition: `id` is a live cell in the sheet this `Labels` will be used with, holding
    ///   a `cel_runtime::DynamicSequence`.
    pub fn add_tuple_cell(&mut self, id: CellId, label: &str) {
        self.cells.insert(
            id,
            CellMeta {
                label: label.to_owned(),
                is_bool: false,
                is_numeric: false,
                display: Box::new(move |sheet| {
                    sheet
                        .read::<cel_runtime::DynamicSequence>(id)
                        .map(|v| format!("{v:?}"))
                        .unwrap_or_else(|_| "?".to_owned())
                }),
                write_str: Box::new(|_sheet, _s| {
                    Err(Error::MethodFailed(anyhow::anyhow!(
                        "editing tuple-typed cells is not yet supported"
                    )))
                }),
                range: None,
            },
        );
    }
}

impl Default for Labels {
    /// Returns `Labels::new()`.
    fn default() -> Self {
        Self::new()
    }
}

/// Formats a floating-point value for display, rounded to 2 decimal places with
/// trailing zeros (and a bare trailing decimal point) trimmed.
///
/// Used in place of plain `Display` for `f32`/`f64` cells so inspector fields show `86.67`
/// and `300` rather than `86.666666666667` and `300.0`. Not applied to other cell types:
/// precision has no meaningful effect on integers, and it would truncate `String` values
/// outright.
///
/// # Examples
///
/// ```text
/// format_rounded(86.666666666667) == "86.67"
/// format_rounded(300.0)            == "300"
/// format_rounded(-0.001)           == "0"
/// ```
pub fn format_rounded(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0');
    let s = s.trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// Converts a filter-recognized numeric primitive to `f64` for display — the same "every numeric
/// type displays as `f64`" convention [`format_rounded`] already documents. Implemented for
/// exactly the 14 primitives `TypeRegistry::range_entry` recognizes range support for; `i64`,
/// `u64`, `i128`, `u128`, `usize`, and `isize` lose precision beyond 2^53, identical to
/// `labels_from_cell_names`'s existing `try_float_ty!`-driven display path for those types.
trait ToF64Display {
    fn to_f64_display(&self) -> f64;
}

macro_rules! impl_to_f64_display {
    ($($T:ty),*) => {
        $(impl ToF64Display for $T {
            fn to_f64_display(&self) -> f64 {
                *self as f64
            }
        })*
    };
}
impl_to_f64_display!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64
);

/// Builds a [`Labels`] from an adam-lang-style declaration-ordered cell name map.
///
/// Matches each scalar cell's `TypeId` against the built-in primitive types
/// `adam_lang::TypeRegistry::new()` registers. A tuple-typed cell
/// (`TypeShape::Tuple`) appears with a Debug-formatted, display-only entry via
/// [`Labels::add_tuple_cell`]. Cells whose `TypeId` is none of the built-in
/// primitives are silently skipped, so they simply won't appear in the sidebar.
///
/// - Complexity: O(n) in the number of cells.
pub fn labels_from_cell_names(
    sheet: &Sheet,
    cell_names: &IndexMap<String, (CellId, TypeShape)>,
) -> Labels {
    let mut labels = Labels::new();
    for (name, (id, shape)) in cell_names {
        let id = *id;
        let type_id = match shape {
            TypeShape::Named(type_id) => *type_id,
            TypeShape::Tuple(_) => {
                labels.add_tuple_cell(id, name);
                continue;
            }
        };
        macro_rules! try_numeric_ty {
            ($T:ty) => {
                if type_id == TypeId::of::<$T>() {
                    labels.add_cell::<$T>(id, name);
                    mark_numeric::<$T>(&mut labels, sheet, id);
                    continue;
                }
            };
        }
        try_numeric_ty!(i8);
        try_numeric_ty!(i16);
        try_numeric_ty!(i32);
        try_numeric_ty!(i64);
        try_numeric_ty!(i128);
        try_numeric_ty!(isize);
        try_numeric_ty!(u8);
        try_numeric_ty!(u16);
        try_numeric_ty!(u32);
        try_numeric_ty!(u64);
        try_numeric_ty!(u128);
        try_numeric_ty!(usize);

        macro_rules! try_numeric_float_ty {
            ($T:ty) => {
                if type_id == TypeId::of::<$T>() {
                    labels.add_cell::<$T>(id, name);
                    if let Some(meta) = labels.cells.get_mut(&id) {
                        meta.display = Box::new(move |sheet| {
                            sheet
                                .read::<$T>(id)
                                .map(|v| format_rounded(*v as f64))
                                .unwrap_or_else(|_| "?".to_owned())
                        });
                    }
                    mark_numeric::<$T>(&mut labels, sheet, id);
                    continue;
                }
            };
        }
        try_numeric_float_ty!(f32);
        try_numeric_float_ty!(f64);

        macro_rules! try_ty {
            ($T:ty) => {
                if type_id == TypeId::of::<$T>() {
                    labels.add_cell::<$T>(id, name);
                    continue;
                }
            };
        }
        try_ty!(bool);
        try_ty!(String);
    }
    labels
}

/// Marks `id`'s `CellMeta` as numeric and, if `sheet.filter_kind(id)` is a range clamp,
/// populates its live-range closure.
fn mark_numeric<T: std::any::Any + Clone + ToF64Display>(
    labels: &mut Labels,
    sheet: &Sheet,
    id: CellId,
) {
    let Some(meta) = labels.cells.get_mut(&id) else {
        return;
    };
    meta.is_numeric = true;
    if matches!(
        sheet.filter_kind(id),
        Some(adam_rs::FilterKind::Range { .. })
    ) {
        meta.range = Some(Box::new(move |sheet: &Sheet| {
            sheet
                .filter_range::<T>(id)
                .map(|(lo, hi)| (lo.to_f64_display(), hi.to_f64_display()))
                .unwrap_or((0.0, 0.0))
        }));
    }
}

/// Formats an [`Error`] as a rustc-style diagnostic when possible.
///
/// `Error::MethodFailed` wraps an `anyhow::Error` raised by a compiled method
/// body; when that error carries a `SpanContext` (attached automatically by
/// cel-parser's `span-diagnostics` feature for built-in arithmetic ops) this
/// renders a full caret diagnostic against `source`, ANSI-colored for a
/// terminal, with `file_name` (e.g. `"begin/examples/toy_example.adm2"`) shown
/// in the diagnostic header. All other variants have no source span and fall
/// back to their `Display` message, ignoring `file_name`.
pub fn format_adam_error(e: &Error, source: &str, file_name: &str) -> String {
    match e {
        Error::MethodFailed(inner) => {
            inner.format_rustc_style(source, file_name, 1, &Renderer::styled())
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adam_rs::Sheet as AdamSheet;

    #[test]
    fn format_adam_error_invalid_id_falls_back_to_display() {
        let msg = format_adam_error(&Error::InvalidId, "source text", "test.adm2");
        assert_eq!(msg, "invalid cell or relationship id");
    }

    #[test]
    fn format_adam_error_method_failed_renders_caret_diagnostic() {
        use cel_parser::{SourceSpan, SpanContext};

        let source = "1i32 / 0i32";
        let span = SourceSpan::new(1, 0, 1, 11);
        let inner = anyhow::anyhow!("division by zero").context(SpanContext::new(span));
        let err = Error::MethodFailed(inner);

        let msg = format_adam_error(&err, source, "test.adm2");

        assert!(msg.contains("division by zero"), "{msg}");
        assert!(msg.contains(source), "{msg}");
    }

    #[test]
    fn format_rounded_trims_trailing_zeros_and_point() {
        assert_eq!(format_rounded(86.666666666667), "86.67");
        assert_eq!(format_rounded(300.0), "300");
        assert_eq!(format_rounded(2.5), "2.5");
        assert_eq!(format_rounded(0.0), "0");
    }

    #[test]
    fn format_rounded_negative_zero_has_no_minus_sign() {
        assert_eq!(format_rounded(-0.0), "0");
        assert_eq!(format_rounded(-0.001), "0");
    }

    #[test]
    fn labels_from_cell_names_rounds_float_display_to_two_decimals() {
        let mut sheet = AdamSheet::new();
        let a = sheet.add_cell(86.666666666667_f64);

        let mut cell_names = IndexMap::new();
        cell_names.insert("a".to_string(), (a, TypeShape::Named(TypeId::of::<f64>())));

        let labels = labels_from_cell_names(&sheet, &cell_names);

        assert_eq!((labels.cells[&a].display)(&sheet), "86.67");
    }

    #[test]
    fn labels_from_cell_names_builds_entries_for_supported_types() {
        let mut sheet = AdamSheet::new();
        let a = sheet.add_cell(2.0_f64);
        let b = sheet.add_cell(3_i32);
        let c = sheet.add_cell(true);
        let d = sheet.add_cell("hi".to_string());

        let mut cell_names = IndexMap::new();
        cell_names.insert("a".to_string(), (a, TypeShape::Named(TypeId::of::<f64>())));
        cell_names.insert("b".to_string(), (b, TypeShape::Named(TypeId::of::<i32>())));
        cell_names.insert("c".to_string(), (c, TypeShape::Named(TypeId::of::<bool>())));
        cell_names.insert(
            "d".to_string(),
            (d, TypeShape::Named(TypeId::of::<String>())),
        );

        let labels = labels_from_cell_names(&sheet, &cell_names);

        assert_eq!(labels.cells.len(), 4);
        assert_eq!((labels.cells[&a].display)(&sheet), "2");
        assert_eq!((labels.cells[&b].display)(&sheet), "3");
        assert_eq!((labels.cells[&c].display)(&sheet), "true");
        assert_eq!((labels.cells[&d].display)(&sheet), "hi");
        assert!(!labels.cells[&a].is_bool);
        assert!(!labels.cells[&b].is_bool);
        assert!(labels.cells[&c].is_bool);
        assert!(!labels.cells[&d].is_bool);
    }

    #[test]
    fn labels_from_cell_names_includes_tuple_typed_cells() {
        let mut sheet = AdamSheet::new();
        let pair = sheet.add_cell(cel_runtime::DynamicSequence::from_tuple((3i32, 4.5f64)));

        let mut cell_names = IndexMap::new();
        cell_names.insert(
            "pair".to_string(),
            (
                pair,
                TypeShape::Tuple(vec![
                    TypeShape::Named(TypeId::of::<i32>()),
                    TypeShape::Named(TypeId::of::<f64>()),
                ]),
            ),
        );

        let labels = labels_from_cell_names(&sheet, &cell_names);

        assert_eq!(labels.cells.len(), 1);
        assert_eq!((labels.cells[&pair].display)(&sheet), "(3, 4.5)");
        assert!(!labels.cells[&pair].is_bool);
    }

    #[test]
    fn labels_from_cell_names_preserves_declaration_order() {
        let mut sheet = AdamSheet::new();
        let z = sheet.add_cell(1_i32);
        let a = sheet.add_cell(2_i32);

        let mut cell_names = IndexMap::new();
        cell_names.insert("z".to_string(), (z, TypeShape::Named(TypeId::of::<i32>())));
        cell_names.insert("a".to_string(), (a, TypeShape::Named(TypeId::of::<i32>())));

        let labels = labels_from_cell_names(&sheet, &cell_names);
        let ids: Vec<_> = labels.cells.keys().copied().collect();
        assert_eq!(ids, vec![z, a]);
    }

    #[test]
    fn labels_from_cell_names_marks_numeric_cells_and_leaves_range_none_without_a_filter() {
        let mut sheet = AdamSheet::new();
        let a = sheet.add_cell(3_i32);
        let b = sheet.add_cell(true);

        let mut cell_names = IndexMap::new();
        cell_names.insert("a".to_string(), (a, TypeShape::Named(TypeId::of::<i32>())));
        cell_names.insert("b".to_string(), (b, TypeShape::Named(TypeId::of::<bool>())));

        let labels = labels_from_cell_names(&sheet, &cell_names);

        assert!(labels.cells[&a].is_numeric);
        assert!(labels.cells[&a].range.is_none());
        assert!(!labels.cells[&b].is_numeric);
    }

    #[test]
    fn labels_from_cell_names_populates_range_for_a_range_filtered_cell() {
        use adam_rs::Filter;
        use std::any::Any;

        let mut sheet = AdamSheet::new();
        let a = sheet.add_cell(50_i32);
        let filter = Filter::range(
            TypeId::of::<i32>(),
            vec![],
            vec![],
            |value, _args| Ok(Box::new(*value.downcast_ref::<i32>().unwrap()) as Box<dyn Any>),
            |_args| {
                Some((
                    Box::new(0i32) as Box<dyn Any>,
                    Box::new(100i32) as Box<dyn Any>,
                ))
            },
        );
        sheet.add_filter(a, filter).unwrap();

        let mut cell_names = IndexMap::new();
        cell_names.insert("a".to_string(), (a, TypeShape::Named(TypeId::of::<i32>())));

        let labels = labels_from_cell_names(&sheet, &cell_names);

        let range_fn = labels.cells[&a].range.as_ref().expect("range populated");
        assert_eq!(range_fn(&sheet), (0.0, 100.0));
    }

    fn sheet_with_one_cell() -> (AdamSheet, Labels) {
        let mut sheet = AdamSheet::new();
        let mut labels = Labels::new();
        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        (sheet, labels)
    }

    #[test]
    fn display_closure_returns_value_string() {
        let (sheet, labels) = sheet_with_one_cell();
        let a_id = *labels.cells.keys().next().unwrap();
        let display = &labels.cells[&a_id].display;
        assert_eq!(display(&sheet), "2");
    }

    #[test]
    fn write_str_closure_parses_and_writes() {
        let (mut sheet, labels) = sheet_with_one_cell();
        let a_id = *labels.cells.keys().next().unwrap();
        assert!((labels.cells[&a_id].write_str)(&mut sheet, "5.0").is_ok());
        let display = &labels.cells[&a_id].display;
        assert_eq!(display(&sheet), "5");
    }

    #[test]
    fn add_tuple_cell_display_returns_rust_debug_formatted_string() {
        let mut sheet = AdamSheet::new();
        let cell_id = sheet.add_cell(cel_runtime::DynamicSequence::from_tuple((3i32, 4.5f64)));
        let mut labels = Labels::new();
        labels.add_tuple_cell(cell_id, "pair");
        let meta = labels.cells.get(&cell_id).unwrap();
        assert_eq!((meta.display)(&sheet), "(3, 4.5)");
    }

    #[test]
    fn add_tuple_cell_write_str_always_errs_without_mutating_the_sheet() {
        let mut sheet = AdamSheet::new();
        let cell_id = sheet.add_cell(cel_runtime::DynamicSequence::from_tuple((3i32, 4.5f64)));
        let mut labels = Labels::new();
        labels.add_tuple_cell(cell_id, "pair");
        let meta = labels.cells.get(&cell_id).unwrap();
        let before = sheet
            .read::<cel_runtime::DynamicSequence>(cell_id)
            .unwrap()
            .clone();
        let result = (meta.write_str)(&mut sheet, "(1, 2.0)");
        assert!(result.is_err());
        let after = sheet.read::<cel_runtime::DynamicSequence>(cell_id).unwrap();
        assert_eq!(&before, after);
    }
}
```

- [ ] **Step 2: Declare the module in `adam-web-ui/src/lib.rs`**

Add:

```rust
pub mod labels;

pub use labels::{CellMeta, Labels, WriteStrFn, format_adam_error, format_rounded, labels_from_cell_names};
```

- [ ] **Step 3: Trim `begin/src/bridge.rs`**

Delete everything from the top of the file through the end of the `mark_numeric` function
(the block that starts with the module doc comment `//! Serialization bridge...` and ends
just before `/// Formats an [`Error`] as a rustc-style diagnostic when possible.` — i.e.
delete `format_adam_error` too). Replace the deleted module doc + `use` block with:

```rust
//! Serialization bridge from [`adam_rs::Sheet`] to D3-ready JSON, for [`crate::graph_view`].

use adam_rs::{CellId, ConditionalId, RelationshipId, Sheet};
use adam_web_ui::Labels;
use slotmap::Key;
```

(`Error`, `TypeShape`, `Renderer`, `FormatRustcStyle`, `IndexMap`, `TypeId` are no longer
needed in this file — they were only used by the code that just moved.)

- [ ] **Step 4: Remove the moved tests from `begin/src/bridge.rs`'s `#[cfg(test)] mod tests`**

Delete these test functions and the now-unused `use adam_rs::{MatchExpr, Method, Sheet};` →
keep `Sheet` (still needed by `to_graph_data`'s remaining tests) but the module already
imports `Sheet` at the top now, so change the test module's own `use` line from
`use adam_rs::{MatchExpr, Method, Sheet};` to `use adam_rs::{MatchExpr, Method};` (avoiding a
duplicate-import warning):

- `format_adam_error_invalid_id_falls_back_to_display`
- `format_adam_error_method_failed_renders_caret_diagnostic`
- `format_rounded_trims_trailing_zeros_and_point`
- `format_rounded_negative_zero_has_no_minus_sign`
- `labels_from_cell_names_rounds_float_display_to_two_decimals`
- `labels_from_cell_names_builds_entries_for_supported_types`
- `labels_from_cell_names_includes_tuple_typed_cells`
- `labels_from_cell_names_preserves_declaration_order`
- `labels_from_cell_names_marks_numeric_cells_and_leaves_range_none_without_a_filter`
- `labels_from_cell_names_populates_range_for_a_range_filtered_cell`
- `display_closure_returns_value_string`
- `add_tuple_cell_display_returns_rust_debug_formatted_string`
- `add_tuple_cell_write_str_always_errs_without_mutating_the_sheet`
- `write_str_closure_parses_and_writes`

Everything else in `bridge.rs` (the `NodeKind`/`NodeData`/`LinkKind`/`LinkData`/`GraphData`
types, `cell_node_id`/`rel_node_id`/`cond_node_id`/`branch_node_id`, `push_branch_links`,
`to_graph_data`, and the `to_graph_data_*` tests plus their `demo_sheet*`/`sheet_with_*`
helpers) is unchanged.

- [ ] **Step 5: Update `begin/src/example_source.rs`'s import**

Change:

```rust
use crate::bridge::{Labels, format_adam_error, labels_from_cell_names};
```

to:

```rust
use adam_web_ui::{Labels, format_adam_error, labels_from_cell_names};
```

(This import becomes fully unused once Task 5 extracts `build_sheet` out of this file too —
leave it for now; Task 5 removes it.)

- [ ] **Step 6: Verify**

Run: `cargo build -p adam-web-ui`, `cargo test -p adam-web-ui`, `cargo build -p begin
--no-default-features`
Expected: all succeed with zero warnings; `adam-web-ui`'s new tests pass.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "Extract Labels/format_adam_error into adam-web-ui"
```

---

### Task 5: Extract `build_sheet`/`BuildOutcome`/`op_lookup` into `adam-web-ui/src/build.rs`

**Files:**
- Create: `adam-web-ui/src/build.rs`
- Modify: `begin/src/example_source.rs` (remove the extracted block; adjust imports and the
  test module)
- Modify: `adam-web-ui/src/lib.rs`
- Modify: `begin/src/app.rs:7-9` (import)

**Interfaces:**
- Consumes: `adam_lang::{AdamParser, TypeRegistry}`, `cel_parser::OpLookup`, `cel_std::install`,
  `annotate_snippets::Renderer`, `cel_parser::FormatRustcStyle`, this crate's own
  `labels_from_cell_names`/`format_adam_error`/`Labels`.
- Produces: `adam_web_ui::build::{BuildOutcome, op_lookup, build_sheet}` — identical
  signatures to today's `begin::example_source` versions.

- [ ] **Step 1: Create `adam-web-ui/src/build.rs`**

```rust
//! Parses adam-lang source into a live [`adam_rs::Sheet`], formatting any failure as a
//! diagnostic instead of a bare error.

use crate::labels::{Labels, format_adam_error, labels_from_cell_names};
use adam_lang::{AdamParser, TypeRegistry};
use adam_rs::Sheet;
use annotate_snippets::Renderer;
use cel_parser::FormatRustcStyle;

/// The result of parsing and building a sheet from adam-lang source.
///
/// `sheet_labels` is `None` only on parse failure. A successful parse that
/// then fails to propagate still returns the built sheet and labels alongside
/// the formatted error, matching how [`crate::SheetInspector`] already tolerates
/// propagate failures during cell edits.
pub struct BuildOutcome {
    /// The built sheet and its UI labels, if parsing succeeded.
    pub sheet_labels: Option<(Sheet, Labels)>,
    /// A formatted rustc-style diagnostic, if parsing or propagation failed.
    pub error: Option<String>,
}

/// Builds a [`cel_parser::OpLookup`] with the CEL standard library installed, so every
/// adam-lang source [`build_sheet`] parses has the same function set (`min`, `max`, `clamp`,
/// etc.) available.
///
/// Every source parsed through this function therefore has `cel-std` installed — a source
/// that deliberately relies on the standard library being *absent* cannot use this function
/// or [`build_sheet`]; construct an `AdamParser` directly with an empty `OpLookup` instead.
pub fn op_lookup() -> cel_parser::OpLookup {
    let mut lookup = cel_parser::OpLookup::new();
    cel_std::install(&mut lookup);
    lookup
}

/// Parses `source` as adam-lang, builds a `Sheet` and `Labels`, and propagates
/// once so initial derived values are populated. `file_name` is used only to
/// build diagnostic headers (e.g. `--> begin/examples/toy_example.adm2:8:11`),
/// not to locate `source` itself.
///
/// - Complexity: O(n) in the length of `source` plus the cost of one `propagate()`.
pub fn build_sheet(source: &str, file_name: &str) -> BuildOutcome {
    let mut parser = AdamParser::new(TypeRegistry::new(), op_lookup());
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
    let labels = labels_from_cell_names(&parsed.sheet, &parsed.cell_names);
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

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SOURCE: &str = r#"
        sheet s {
            cell a: f64 = 2.0;
            cell b: f64 = 3.0;
            cell c: f64;
            relationship {
                c := a * b;
                a := c / b;
                b := c / a;
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
        let source = "sheet s { cell x: i32 = 0; cell y: i32; relationship { y := 10i32 / x; } }";
        let outcome = build_sheet(source, "test.adm2");
        assert!(
            outcome.sheet_labels.is_some(),
            "sheet should still be built after a propagate error"
        );
        assert!(outcome.error.is_some());
    }
}
```

- [ ] **Step 2: Declare the module in `adam-web-ui/src/lib.rs`**

Add:

```rust
pub mod build;

pub use build::{BuildOutcome, build_sheet};
```

- [ ] **Step 3: Remove the extracted items from `begin/src/example_source.rs`**

Delete the `BuildOutcome` struct, the `op_lookup` function, and the `build_sheet` function
(currently lines ~143–200 as of this plan's writing — confirm exact range before editing).

Change the top-of-file import from:

```rust
use crate::bridge::{Labels, format_adam_error, labels_from_cell_names};
```

to nothing (delete the line entirely — nothing left in this file's non-test code needs
`Labels`/`format_adam_error`/`labels_from_cell_names` directly once `BuildOutcome`/
`build_sheet` are gone).

Move the three now-orphaned tests (`build_sheet_valid_source_succeeds_with_no_error`,
`build_sheet_parse_error_has_no_sheet_and_formatted_message`,
`build_sheet_runtime_error_still_returns_sheet_and_message`) — delete them from
`begin/src/example_source.rs`'s test module; they now live in `adam-web-ui/src/build.rs`
(Step 1 above already added them there).

In the same test module, add an import for the tests that still call `op_lookup()` directly
(`image_resize_constrain_is_relevant_despite_only_being_a_conditional_expression_input`,
`image_resize_relevance_does_not_depend_on_which_cell_currently_holds_strength`):

```rust
use adam_web_ui::build::op_lookup;
```

(Add this inside `#[cfg(test)] mod tests { use super::*; ... }`, not at the top of the file —
nothing outside tests calls `op_lookup` anymore, and a top-level import would trigger an
unused-import warning on a non-test build.)

`every_bundled_example_parses_successfully` needs `build_sheet` — add, inside the same test
module:

```rust
use adam_web_ui::build_sheet;
```

- [ ] **Step 4: Update `begin/src/app.rs`'s import**

Change:

```rust
use crate::example_source::{
    ActiveSource, SourceOrigin, available_examples, build_sheet, load_example_source,
};
```

to:

```rust
use crate::example_source::{ActiveSource, SourceOrigin, available_examples, load_example_source};
use adam_web_ui::build_sheet;
```

- [ ] **Step 5: Verify**

Run: `cargo build -p adam-web-ui`, `cargo test -p adam-web-ui`, `cargo build -p begin
--no-default-features`, `cargo test -p begin --no-default-features`
Expected: all succeed with zero warnings; all tests pass.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "Extract build_sheet into adam-web-ui"
```

---

### Task 6: Extract `Inspector` into `adam-web-ui/src/inspector.rs` as `SheetInspector`

**Files:**
- Move: `begin/src/inspector.rs` → `adam-web-ui/src/inspector.rs`
- Modify: `adam-web-ui/src/inspector.rs` (decouple from `ActiveSource`)
- Modify: `adam-web-ui/src/lib.rs`
- Modify: `begin/src/main.rs` (remove `mod inspector;`)
- Modify: `begin/src/app.rs` (import + two new memos + call-site change)

**Interfaces:**
- Consumes: `adam_rs::{CellId, FilterViolation, Sheet}`, this crate's `Labels`/
  `format_adam_error`/`format_rounded`, `crate::spectrum::*`, `crate::diagnostics`.
- Produces: `adam_web_ui::SheetInspector { sheet: Signal<Sheet>, labels: Signal<Labels>,
  source_text: Memo<String>, source_name: Memo<String> } -> Element`.

- [ ] **Step 1: Move the file**

```bash
git mv begin/src/inspector.rs adam-web-ui/src/inspector.rs
```

- [ ] **Step 2: Update the moved file's import and module doc**

Change:

```rust
use crate::bridge::{Labels, format_adam_error, format_rounded};
```

to:

```rust
use crate::labels::{Labels, format_adam_error, format_rounded};
```

(`use crate::spectrum::{...}` is unchanged — `spectrum.rs` already lives in this same crate
since Task 2.)

Also update the file's module doc comment (its very first line), which contains a rustdoc
intra-doc link to the symbol being renamed in Step 4 below — leaving it as `[`Inspector`]`
would be a broken intra-doc link once that symbol no longer exists, which
`RUSTDOCFLAGS="-D warnings"` treats as a build failure. Change:

```rust
//! [`Inspector`] — sidebar listing all cells with their current values and a write form.
```

to:

```rust
//! [`SheetInspector`] — a live, editable list of a sheet's cells with a write form.
```

- [ ] **Step 3: Decouple `write_and_propagate` from `ActiveSource`**

Change its signature from:

```rust
fn write_and_propagate(
    mut sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    id: CellId,
    val: &str,
    mut has_error: Signal<bool>,
    active_source: Signal<crate::example_source::ActiveSource>,
) {
```

to:

```rust
fn write_and_propagate(
    mut sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    id: CellId,
    val: &str,
    mut has_error: Signal<bool>,
    source_text: Memo<String>,
    source_name: Memo<String>,
) {
```

and its error-branch body from:

```rust
        Err(e) => {
            has_error.set(true);
            let active = active_source.read();
            crate::diagnostics::report_error(&format_adam_error(
                &e,
                &active.text,
                &active.file_name(),
            ));
        }
```

to:

```rust
        Err(e) => {
            has_error.set(true);
            crate::diagnostics::report_error(&format_adam_error(
                &e,
                &source_text.read(),
                &source_name.read(),
            ));
        }
```

- [ ] **Step 4: Rename `Inspector` to `SheetInspector` and decouple its props**

Change:

```rust
#[component]
pub fn Inspector(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<crate::example_source::ActiveSource>,
) -> Element {
    let ids: Vec<CellId> = labels.read().cells.keys().copied().collect();
    let output_status = use_memo(move || compute_output_status(&sheet.read()));

    rsx! {
        div {
            style: "width: 260px; min-width: 260px; height: 100%; overflow-y: auto; padding: 12px; box-sizing: border-box;",
            SpHeading { "Cells" }
            SpDivider {}
            for id in ids {
                CellRow { key: "{id:?}", id, sheet, labels, active_source, output_status }
            }
        }
    }
}
```

to:

```rust
#[component]
pub fn SheetInspector(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    source_text: Memo<String>,
    source_name: Memo<String>,
) -> Element {
    let ids: Vec<CellId> = labels.read().cells.keys().copied().collect();
    let output_status = use_memo(move || compute_output_status(&sheet.read()));

    rsx! {
        div {
            style: "width: 260px; min-width: 260px; height: 100%; overflow-y: auto; padding: 12px; box-sizing: border-box;",
            SpHeading { "Cells" }
            SpDivider {}
            for id in ids {
                CellRow { key: "{id:?}", id, sheet, labels, source_text, source_name, output_status }
            }
        }
    }
}
```

(Its doc comment above `pub fn Inspector` is unchanged apart from the name.)

- [ ] **Step 5: Update `CellRow`'s props and its two `write_and_propagate` call sites**

Change:

```rust
#[component]
fn CellRow(
    id: CellId,
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<crate::example_source::ActiveSource>,
    output_status: Memo<OutputStatus>,
) -> Element {
```

to:

```rust
#[component]
fn CellRow(
    id: CellId,
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    source_text: Memo<String>,
    source_name: Memo<String>,
    output_status: Memo<OutputStatus>,
) -> Element {
```

Then replace both occurrences of `write_and_propagate(sheet, labels, id, next, has_error,
active_source);`/`write_and_propagate(sheet, labels, id, &val, has_error, active_source);`
(the checkbox `onclick`, the number field `oninput`, the slider `oninput`, and the text field
`oninput` — 4 call sites total) with the same call but `source_text, source_name` in place of
`active_source`.

- [ ] **Step 6: Declare the module in `adam-web-ui/src/lib.rs`**

Add:

```rust
mod inspector;

pub use inspector::SheetInspector;
```

(Note `mod inspector;`, not `pub mod` — only `SheetInspector` itself is part of this crate's
public surface; `CellRow` and the helper functions stay private, exactly as `begin`'s
original module only exported `Inspector`.)

- [ ] **Step 7: Remove `begin/src/inspector.rs`'s `mod` declaration**

In `begin/src/main.rs`, remove the line `mod inspector;`.

- [ ] **Step 8: Update `begin/src/app.rs`**

Change the import:

```rust
use crate::inspector::Inspector;
```

to:

```rust
use adam_web_ui::SheetInspector;
```

Add two new memos, next to the existing `graph_data`/`source_id` memos:

```rust
let source_text = use_memo(move || active_source.read().text.clone());
let source_name = use_memo(move || active_source.read().file_name());
```

Change the `Inspector` usage:

```rust
Inspector { sheet, labels, active_source }
```

to:

```rust
SheetInspector { sheet, labels, source_text, source_name }
```

- [ ] **Step 9: Verify**

Run: `cargo build -p adam-web-ui`, `cargo test -p adam-web-ui`, `cargo build -p begin
--no-default-features`, `cargo test -p begin --no-default-features`
Expected: all succeed with zero warnings; all tests pass.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "Extract Inspector into adam-web-ui as SheetInspector"
```

---

### Task 7: Full verification of the extraction

**Files:** none (verification only).

- [ ] **Step 1: Run the full check suite**

```bash
cargo fmt --all -- --check
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace
```

Expected: everything passes with zero warnings. Read the `cargo build`/`cargo test` output
directly (not just the exit code) — this repo requires zero compiler warnings, which
`clippy -D warnings` alone does not guarantee.

- [ ] **Step 2: Visually verify `begin` is unaffected**

Use the `verifying-begin-ui` skill: serve `begin` as a web app, screenshot it, and dump its
DOM. Confirm the Inspector panel on the right renders identically to before this phase
(cell fields, checkboxes, sliders, forced/invalid/warning styling all present) — this phase
is a pure refactor, so nothing about `begin`'s rendered output should have changed.

- [ ] **Step 3: Commit** (only if Step 1/2 required fixes; otherwise nothing to commit)

---

## Phase 2 — Spike: multiple independent Dioxus mounts per page

### Task 8: Prove multiple `dioxus_web::launch_virtual_dom` calls can coexist in one wasm module

This de-risks Phase 4's mounting design before building the real wasm crate around it.
`dioxus-web` 0.7.10's `launch_virtual_dom(vdom: VirtualDom, platform_config: Config)`
(`dioxus-web/src/launch.rs`) spawns each app via `wasm_bindgen_futures::spawn_local` with no
shared/global mount-tracking state visible in `dioxus-web/src/dom.rs`'s `WebsysDom::new`, and
`Config::rootname(id)` (`dioxus-web/src/cfg.rs`) targets a specific `#id` element — this spike
confirms that holds in practice, not just by reading the source.

**Files:**
- Create: `scratch/dioxus-multi-mount-spike/Cargo.toml`
- Create: `scratch/dioxus-multi-mount-spike/src/lib.rs`
- Create: `scratch/dioxus-multi-mount-spike/index.html`

This is throwaway code — do not add it to the workspace `members` list; it's built directly
with `wasm-pack` or `dx build` from its own directory, and deleted once this task confirms the
approach (Step 4 below).

**Interfaces:** none — this doesn't feed later tasks' code, only their design assumption.

- [ ] **Step 1: Create the scratch crate**

`scratch/dioxus-multi-mount-spike/Cargo.toml`:

```toml
[package]
name = "dioxus-multi-mount-spike"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
dioxus = { version = "0.7.10", features = ["web"] }
wasm-bindgen = "0.2"
```

`scratch/dioxus-multi-mount-spike/src/lib.rs`:

```rust
use dioxus::prelude::*;
use wasm_bindgen::prelude::*;

#[derive(Clone, PartialEq, Props)]
struct CounterProps {
    label: String,
}

#[component]
fn Counter(props: CounterProps) -> Element {
    let mut count = use_signal(|| 0);
    rsx! {
        div {
            "{props.label}: {count}"
            button { onclick: move |_| count += 1, "+1" }
        }
    }
}

#[wasm_bindgen]
pub fn mount(element_id: &str, label: &str) {
    let vdom = VirtualDom::new_with_props(
        Counter,
        CounterProps {
            label: label.to_string(),
        },
    );
    let config = dioxus::web::Config::new().rootname(element_id);
    dioxus::web::launch::launch_virtual_dom(vdom, config);
}
```

`scratch/dioxus-multi-mount-spike/index.html`:

```html
<!DOCTYPE html>
<html>
  <body>
    <div id="mount-a"></div>
    <div id="mount-b"></div>
    <script type="module">
      import init, { mount } from "./pkg/dioxus_multi_mount_spike.js";
      await init();
      mount("mount-a", "Counter A");
      mount("mount-b", "Counter B");
    </script>
  </body>
</html>
```

- [ ] **Step 2: Build it**

```bash
cd scratch/dioxus-multi-mount-spike
wasm-pack build --target web
```

Expected: builds successfully, producing `pkg/dioxus_multi_mount_spike.js` and `.wasm`.

- [ ] **Step 3: Serve and manually verify**

Serve the directory (e.g. `python3 -m http.server 8000`) and open it in a browser. Click
"+1" in both "Counter A" and "Counter B" independently.

Expected: each counter increments independently — clicking A's button never affects B's
count, and both remain interactive simultaneously. This confirms one loaded wasm module can
drive multiple independent, live `VirtualDom` instances mounted at different elements.

If this does **not** hold (e.g. only the first mount responds, or the second overwrites the
first), stop and re-open the design conversation before proceeding to Phase 4 — the mounting
approach in Tasks 17–19 below assumes this works.

- [ ] **Step 4: Delete the scratch crate**

```bash
rm -rf scratch/dioxus-multi-mount-spike
```

This was throwaway code per the spike's own scope — nothing here is reused. (If
`scratch/` is now empty, remove it too.)

- [ ] **Step 5: Report the finding**

No commit needed for the deleted scratch code. In the PR description or a code comment on
Task 17, note that this spike was run and confirmed the approach (or, if it didn't confirm,
what the fallback design ended up being).

---

## Phase 3 — Convert the book's examples to standalone `.adm2` files

Each task below follows the same shape: create the chapter's `.adm2` files under
`adam-lang-book/book-src/examples/<chapter>/`, replace each test's inline sheet-source string
literal with `include_str!` of the new file, and replace the chapter's `{{#include
tests/<chapter>.rs:anchor}}` directives with `{{#include examples/<chapter>/<name>.adm2}}`.

### Task 9: Convert `cells` chapter (3 examples)

**Files:**
- Create: `adam-lang-book/book-src/examples/cells/type_mismatch_is_a_parse_error.adm2`
- Create: `adam-lang-book/book-src/examples/cells/tuple_typed_cell.adm2`
- Create: `adam-lang-book/book-src/examples/cells/no_forward_references.adm2`
- Modify: `adam-lang-book/tests/cells.rs`
- Modify: `adam-lang-book/book-src/cells.md:63,90,113`

- [ ] **Step 1: Create the `.adm2` files**

`adam-lang-book/book-src/examples/cells/type_mismatch_is_a_parse_error.adm2`:

```
sheet s { cell x: i32 = 1.0; }
```

`adam-lang-book/book-src/examples/cells/tuple_typed_cell.adm2`:

```
sheet s { cell point: (f64, f64) = (0.0, 0.0); }
```

`adam-lang-book/book-src/examples/cells/no_forward_references.adm2`:

```
sheet s { relationship { y := x; } cell x = 0; cell y = 0; } 
```

(Note the trailing space in the last file — preserved exactly from the original string
literal, since it's genuinely part of what's being parsed.)

- [ ] **Step 2: Update `adam-lang-book/tests/cells.rs`**

Change:

```rust
#[test]
fn type_mismatch_is_a_parse_error() {
    // ANCHOR: type_mismatch_is_a_parse_error
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str("sheet s { cell x: i32 = 1.0; }")
        .err()
        .unwrap();
    assert!(format!("{err}").contains("type mismatch"));
    // ANCHOR_END: type_mismatch_is_a_parse_error
}
```

to:

```rust
#[test]
fn type_mismatch_is_a_parse_error() {
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/cells/type_mismatch_is_a_parse_error.adm2"
        ))
        .err()
        .unwrap();
    assert!(format!("{err}").contains("type mismatch"));
}
```

Change:

```rust
#[test]
fn tuple_typed_cell() {
    // ANCHOR: tuple_typed_cell
    let mut parser = adam_lang_book::support::parser();
    let parsed = parser
        .parse_str("sheet s { cell point: (f64, f64) = (0.0, 0.0); }")
        .unwrap();
    let point = parsed.cell_names["point"].0;
    let value = parsed
        .read::<cel_runtime::DynamicSequence>(point)
        .unwrap()
        .clone();
    assert_eq!(value.try_to_tuple::<(f64, f64)>().unwrap(), (0.0, 0.0));
    // ANCHOR_END: tuple_typed_cell
}
```

to:

```rust
#[test]
fn tuple_typed_cell() {
    let mut parser = adam_lang_book::support::parser();
    let parsed = parser
        .parse_str(include_str!("../book-src/examples/cells/tuple_typed_cell.adm2"))
        .unwrap();
    let point = parsed.cell_names["point"].0;
    let value = parsed
        .read::<cel_runtime::DynamicSequence>(point)
        .unwrap()
        .clone();
    assert_eq!(value.try_to_tuple::<(f64, f64)>().unwrap(), (0.0, 0.0));
}
```

Change:

```rust
#[test]
fn no_forward_references() {
    // ANCHOR: no_forward_references
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str("sheet s { relationship { y := x; } cell x = 0; cell y = 0; } ")
        .err()
        .unwrap();
    assert!(format!("{err}").contains("undeclared cell"));
    // ANCHOR_END: no_forward_references
}
```

to:

```rust
#[test]
fn no_forward_references() {
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/cells/no_forward_references.adm2"
        ))
        .err()
        .unwrap();
    assert!(format!("{err}").contains("undeclared cell"));
}
```

- [ ] **Step 3: Update `adam-lang-book/book-src/cells.md`**

Replace:
- `{{#include ../tests/cells.rs:type_mismatch_is_a_parse_error}}` with
  `{{#include examples/cells/type_mismatch_is_a_parse_error.adm2}}`
- `{{#include ../tests/cells.rs:tuple_typed_cell}}` with
  `{{#include examples/cells/tuple_typed_cell.adm2}}`
- `{{#include ../tests/cells.rs:no_forward_references}}` with
  `{{#include examples/cells/no_forward_references.adm2}}`

- [ ] **Step 4: Verify**

Run: `cargo test -p adam-lang-book cells` and `mdbook build adam-lang-book`
Expected: tests pass; `mdbook build` succeeds and `cells.md`'s rendered HTML shows only the
Adam source (check `adam-lang-book/book-dist/cells.html`), not Rust.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Convert cells.md examples to standalone .adm2 files"
```

---

### Task 10: Convert `expressions` chapter (2 examples)

**Files:**
- Create: `adam-lang-book/book-src/examples/expressions/no_standard_library.adm2`
- Create: `adam-lang-book/book-src/examples/expressions/initializer_sees_no_cells.adm2`
- Modify: `adam-lang-book/tests/expressions.rs`
- Modify: `adam-lang-book/book-src/expressions.md:25,36`

- [ ] **Step 1: Create the `.adm2` files**

`adam-lang-book/book-src/examples/expressions/no_standard_library.adm2`:

```
sheet s { cell x: i32 = min(1, 2); }
```

`adam-lang-book/book-src/examples/expressions/initializer_sees_no_cells.adm2`:

```
sheet s { cell x = 1; cell y = x + 1; }
```

- [ ] **Step 2: Update `adam-lang-book/tests/expressions.rs`**

Change:

```rust
#[test]
fn no_standard_library() {
    // ANCHOR: no_standard_library
    let mut parser =
        adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(), cel_parser::OpLookup::new());
    let err = parser
        .parse_str("sheet s { cell x: i32 = min(1, 2); }")
        .err()
        .unwrap();
    assert!(format!("{err}").to_lowercase().contains("min"));
    // ANCHOR_END: no_standard_library
}
```

to:

```rust
#[test]
fn no_standard_library() {
    let mut parser =
        adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(), cel_parser::OpLookup::new());
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/expressions/no_standard_library.adm2"
        ))
        .err()
        .unwrap();
    assert!(format!("{err}").to_lowercase().contains("min"));
}
```

Change:

```rust
#[test]
fn initializer_sees_no_cells() {
    // ANCHOR: initializer_sees_no_cells
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str("sheet s { cell x = 1; cell y = x + 1; }")
        .err()
        .unwrap();
    assert!(format!("{err}").to_lowercase().contains("x"));
    // ANCHOR_END: initializer_sees_no_cells
}
```

to:

```rust
#[test]
fn initializer_sees_no_cells() {
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/expressions/initializer_sees_no_cells.adm2"
        ))
        .err()
        .unwrap();
    assert!(format!("{err}").to_lowercase().contains("x"));
}
```

- [ ] **Step 3: Update `adam-lang-book/book-src/expressions.md`**

Replace:
- `{{#include ../tests/expressions.rs:no_standard_library}}` with
  `{{#include examples/expressions/no_standard_library.adm2}}`
- `{{#include ../tests/expressions.rs:initializer_sees_no_cells}}` with
  `{{#include examples/expressions/initializer_sees_no_cells.adm2}}`

- [ ] **Step 4: Verify**

Run: `cargo test -p adam-lang-book expressions` and `mdbook build adam-lang-book`
Expected: tests pass; build succeeds.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Convert expressions.md examples to standalone .adm2 files"
```

---

### Task 11: Convert `conditionals` chapter (2 examples)

**Files:**
- Create: `adam-lang-book/book-src/examples/conditionals/multi_cell_match_subject.adm2`
- Create: `adam-lang-book/book-src/examples/conditionals/default_branch_and_spring_back.adm2`
- Modify: `adam-lang-book/tests/conditionals.rs`
- Modify: `adam-lang-book/book-src/conditionals.md:25,46`

- [ ] **Step 1: Create the `.adm2` files**

`adam-lang-book/book-src/examples/conditionals/multi_cell_match_subject.adm2`:

```
sheet resample_demo {
    cell resample: bool = true;
    cell constrain: bool = true;
    cell locked: bool = false;

    conditional resample && constrain {
        true => {
            relationship { locked := true; }
        }
        false => {
            relationship { locked := false; }
        }
    }
}
```

`adam-lang-book/book-src/examples/conditionals/default_branch_and_spring_back.adm2`:

```
sheet no_default {
    cell mode: i32 = 0;
    cell x: f64 = 1.0;

    conditional mode {
        0i32 => {
            relationship { x := 100.0; }
        }
    }
}
```

- [ ] **Step 2: Update `adam-lang-book/tests/conditionals.rs`**

Change:

```rust
#[test]
fn multi_cell_match_subject() {
    // ANCHOR: multi_cell_match_subject
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet resample_demo {
                cell resample: bool = true;
                cell constrain: bool = true;
                cell locked: bool = false;

                conditional resample && constrain {
                    true => {
                        relationship { locked := true; }
                    }
                    false => {
                        relationship { locked := false; }
                    }
                }
            }
            "#,
        )
        .unwrap();
    parsed.propagate().unwrap();
    let locked = parsed.cell_names["locked"].0;
    assert!(*parsed.read::<bool>(locked).unwrap());

    let resample = parsed.cell_names["resample"].0;
    parsed.write(resample, false).unwrap();
    parsed.propagate().unwrap();
    assert!(!*parsed.read::<bool>(locked).unwrap());
    // ANCHOR_END: multi_cell_match_subject
}
```

to:

```rust
#[test]
fn multi_cell_match_subject() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/conditionals/multi_cell_match_subject.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();
    let locked = parsed.cell_names["locked"].0;
    assert!(*parsed.read::<bool>(locked).unwrap());

    let resample = parsed.cell_names["resample"].0;
    parsed.write(resample, false).unwrap();
    parsed.propagate().unwrap();
    assert!(!*parsed.read::<bool>(locked).unwrap());
}
```

Change:

```rust
#[test]
fn default_branch_and_spring_back() {
    // ANCHOR: default_branch_and_spring_back
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet no_default {
                cell mode: i32 = 0;
                cell x: f64 = 1.0;

                conditional mode {
                    0i32 => {
                        relationship { x := 100.0; }
                    }
                }
            }
            "#,
        )
        .unwrap();
    let mode = parsed.cell_names["mode"].0;
    let x = parsed.cell_names["x"].0;

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 100.0); // mode == 0: branch active
    assert!(!parsed.is_source(x));

    parsed.write(mode, 7_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 1.0); // no branch matches; x reverts to its
    // own declared default, not 100.0
    assert!(parsed.is_source(x));
    // ANCHOR_END: default_branch_and_spring_back
}
```

to:

```rust
#[test]
fn default_branch_and_spring_back() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/conditionals/default_branch_and_spring_back.adm2"
        ))
        .unwrap();
    let mode = parsed.cell_names["mode"].0;
    let x = parsed.cell_names["x"].0;

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 100.0); // mode == 0: branch active
    assert!(!parsed.is_source(x));

    parsed.write(mode, 7_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 1.0); // no branch matches; x reverts to its
    // own declared default, not 100.0
    assert!(parsed.is_source(x));
}
```

- [ ] **Step 3: Update `adam-lang-book/book-src/conditionals.md`**

Replace:
- `{{#include ../tests/conditionals.rs:multi_cell_match_subject}}` with
  `{{#include examples/conditionals/multi_cell_match_subject.adm2}}`
- `{{#include ../tests/conditionals.rs:default_branch_and_spring_back}}` with
  `{{#include examples/conditionals/default_branch_and_spring_back.adm2}}`

- [ ] **Step 4: Verify**

Run: `cargo test -p adam-lang-book conditionals` and `mdbook build adam-lang-book`
Expected: tests pass; build succeeds.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Convert conditionals.md examples to standalone .adm2 files"
```

---

### Task 12: Convert `relationships` chapter (4 examples)

**Files:**
- Create: `adam-lang-book/book-src/examples/relationships/shared_cell_example.adm2`
- Create: `adam-lang-book/book-src/examples/relationships/conflict_error.adm2`
- Create: `adam-lang-book/book-src/examples/relationships/cycle_error.adm2`
- Create: `adam-lang-book/book-src/examples/relationships/destructuring_binding.adm2`
- Modify: `adam-lang-book/tests/relationships.rs`
- Modify: `adam-lang-book/book-src/relationships.md:71,86,94,108`

- [ ] **Step 1: Create the `.adm2` files**

`adam-lang-book/book-src/examples/relationships/shared_cell_example.adm2`:

```
sheet diamond {
    cell a = 0.0;
    cell b = 0.0;
    cell c = 2.0;
    cell d = 3.0;

    relationship {
        c := a * b;
        b := c / a;
        a := c / b;
    }

    relationship {
        d := b * c;
        c := d / b;
        b := d / c;
    }
}
```

`adam-lang-book/book-src/examples/relationships/conflict_error.adm2`:

```
sheet conflict {
    cell x = 1.0;

    relationship { x := 1.0; }
    relationship { x := 2.0; }
}
```

`adam-lang-book/book-src/examples/relationships/cycle_error.adm2`:

```
sheet cycle {
    cell x = 1.0;
    cell y = 1.0;
    cell z = 1.0;

    relationship { y := x; }
    relationship { z := y; }
    relationship { x := z; }
}
```

`adam-lang-book/book-src/examples/relationships/destructuring_binding.adm2`:

```
sheet swap_demo {
    cell a: i32 = 1;
    cell b: i32 = 2;

    relationship {
        (a, b) := (b, a);
    }
}
```

- [ ] **Step 2: Update `adam-lang-book/tests/relationships.rs`**

Change:

```rust
#[test]
fn shared_cell_example() {
    // ANCHOR: shared_cell_example
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet diamond {
                cell a = 0.0;
                cell b = 0.0;
                cell c = 2.0;
                cell d = 3.0;

                relationship {
                    c := a * b;
                    b := c / a;
                    a := c / b;
                }

                relationship {
                    d := b * c;
                    c := d / b;
                    b := d / c;
                }
            }
            "#,
        )
        .unwrap();
    parsed.propagate().unwrap();

    let (a, b, c, d) = (
        parsed.cell_names["a"].0,
        parsed.cell_names["b"].0,
        parsed.cell_names["c"].0,
        parsed.cell_names["d"].0,
    );
    assert!(parsed.is_source(c) && parsed.is_source(d));
    assert!(!parsed.is_source(a) && !parsed.is_source(b));
    assert_eq!(*parsed.read::<f64>(c).unwrap(), 2.0); // untouched
    assert_eq!(*parsed.read::<f64>(d).unwrap(), 3.0); // untouched
    assert_eq!(*parsed.read::<f64>(b).unwrap(), 1.5); // d / c
    assert_eq!(*parsed.read::<f64>(a).unwrap(), 4.0 / 3.0); // c / b
    // ANCHOR_END: shared_cell_example
}
```

to:

```rust
#[test]
fn shared_cell_example() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/relationships/shared_cell_example.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();

    let (a, b, c, d) = (
        parsed.cell_names["a"].0,
        parsed.cell_names["b"].0,
        parsed.cell_names["c"].0,
        parsed.cell_names["d"].0,
    );
    assert!(parsed.is_source(c) && parsed.is_source(d));
    assert!(!parsed.is_source(a) && !parsed.is_source(b));
    assert_eq!(*parsed.read::<f64>(c).unwrap(), 2.0); // untouched
    assert_eq!(*parsed.read::<f64>(d).unwrap(), 3.0); // untouched
    assert_eq!(*parsed.read::<f64>(b).unwrap(), 1.5); // d / c
    assert_eq!(*parsed.read::<f64>(a).unwrap(), 4.0 / 3.0); // c / b
}
```

Change:

```rust
#[test]
fn conflict_error() {
    // ANCHOR: conflict_error
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(
            r#"
            sheet conflict {
                cell x = 1.0;

                relationship { x := 1.0; }
                relationship { x := 2.0; }
            }
            "#,
        )
        .unwrap()
        .propagate()
        .unwrap_err();
    assert!(matches!(err, adam_rs::Error::Conflict));
    // ANCHOR_END: conflict_error
}
```

to:

```rust
#[test]
fn conflict_error() {
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/relationships/conflict_error.adm2"
        ))
        .unwrap()
        .propagate()
        .unwrap_err();
    assert!(matches!(err, adam_rs::Error::Conflict));
}
```

Change:

```rust
#[test]
fn cycle_error() {
    // ANCHOR: cycle_error
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(
            r#"
            sheet cycle {
                cell x = 1.0;
                cell y = 1.0;
                cell z = 1.0;

                relationship { y := x; }
                relationship { z := y; }
                relationship { x := z; }
            }
            "#,
        )
        .unwrap()
        .propagate()
        .unwrap_err();
    assert!(matches!(err, adam_rs::Error::Cycle));
    // ANCHOR_END: cycle_error
}
```

to:

```rust
#[test]
fn cycle_error() {
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/relationships/cycle_error.adm2"
        ))
        .unwrap()
        .propagate()
        .unwrap_err();
    assert!(matches!(err, adam_rs::Error::Cycle));
}
```

Change:

```rust
#[test]
fn destructuring_binding() {
    // ANCHOR: destructuring_binding
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet swap_demo {
                cell a: i32 = 1;
                cell b: i32 = 2;

                relationship {
                    (a, b) := (b, a);
                }
            }
            "#,
        )
        .unwrap();
    parsed.propagate().unwrap();
    let (a, b) = (parsed.cell_names["a"].0, parsed.cell_names["b"].0);
    assert_eq!(*parsed.read::<i32>(a).unwrap(), 2);
    assert_eq!(*parsed.read::<i32>(b).unwrap(), 1);
    // ANCHOR_END: destructuring_binding
}
```

to:

```rust
#[test]
fn destructuring_binding() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/relationships/destructuring_binding.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();
    let (a, b) = (parsed.cell_names["a"].0, parsed.cell_names["b"].0);
    assert_eq!(*parsed.read::<i32>(a).unwrap(), 2);
    assert_eq!(*parsed.read::<i32>(b).unwrap(), 1);
}
```

- [ ] **Step 3: Update `adam-lang-book/book-src/relationships.md`**

Replace:
- `{{#include ../tests/relationships.rs:shared_cell_example}}` with
  `{{#include examples/relationships/shared_cell_example.adm2}}`
- `{{#include ../tests/relationships.rs:conflict_error}}` with
  `{{#include examples/relationships/conflict_error.adm2}}`
- `{{#include ../tests/relationships.rs:cycle_error}}` with
  `{{#include examples/relationships/cycle_error.adm2}}`
- `{{#include ../tests/relationships.rs:destructuring_binding}}` with
  `{{#include examples/relationships/destructuring_binding.adm2}}`

- [ ] **Step 4: Verify**

Run: `cargo test -p adam-lang-book relationships` and `mdbook build adam-lang-book`
Expected: tests pass; build succeeds.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Convert relationships.md examples to standalone .adm2 files"
```

---

### Task 13: Convert `filters` chapter (6 examples)

**Files:**
- Create: `adam-lang-book/book-src/examples/filters/write_never_filters.adm2`
- Create: `adam-lang-book/book-src/examples/filters/raw_value_never_lost.adm2`
- Create: `adam-lang-book/book-src/examples/filters/range_filter_kind.adm2`
- Create: `adam-lang-book/book-src/examples/filters/derived_cell_diagnosed_not_corrected.adm2`
- Create: `adam-lang-book/book-src/examples/filters/must_reference_underscore.adm2`
- Create: `adam-lang-book/book-src/examples/filters/tuple_filter_not_supported.adm2`
- Modify: `adam-lang-book/tests/filters.rs`
- Modify: `adam-lang-book/book-src/filters.md:33,52,68,83,98,102`

- [ ] **Step 1: Create the `.adm2` files**

`adam-lang-book/book-src/examples/filters/write_never_filters.adm2`:

```
sheet s { cell level: i32 = 50 filter 0..=100; }
```

`adam-lang-book/book-src/examples/filters/raw_value_never_lost.adm2`:

```
sheet spring_back {
    cell max: i32 = 100 filter 0..=200;
    cell level: i32 = 50 filter 0..=max;
}
```

`adam-lang-book/book-src/examples/filters/range_filter_kind.adm2`:

```
sheet s { cell level: i32 = 50 filter 0..=100; }
```

`adam-lang-book/book-src/examples/filters/derived_cell_diagnosed_not_corrected.adm2`:

```
sheet diagnose_only {
    cell bound: i32 = 100 filter 0..=100;
    cell driver: i32 = 500;

    relationship {
        bound := driver;
    }
}
```

`adam-lang-book/book-src/examples/filters/must_reference_underscore.adm2`:

```
sheet s { cell x: i32 = 0 filter 5; }
```

`adam-lang-book/book-src/examples/filters/tuple_filter_not_supported.adm2`:

```
sheet s { cell x: (i32, i32) = (0, 0) filter _; }
```

- [ ] **Step 2: Update `adam-lang-book/tests/filters.rs`**

Change:

```rust
#[test]
fn write_never_filters() {
    // ANCHOR: write_never_filters
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str("sheet s { cell level: i32 = 50 filter 0..=100; }")
        .unwrap();
    let level = parsed.cell_names["level"].0;

    parsed.write(level, 500_i32).unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 500); // the raw value, unfiltered

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 100); // now conformed
    // ANCHOR_END: write_never_filters
}
```

to:

```rust
#[test]
fn write_never_filters() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/filters/write_never_filters.adm2"
        ))
        .unwrap();
    let level = parsed.cell_names["level"].0;

    parsed.write(level, 500_i32).unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 500); // the raw value, unfiltered

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 100); // now conformed
}
```

Change:

```rust
#[test]
fn raw_value_never_lost() {
    // ANCHOR: raw_value_never_lost
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet spring_back {
                cell max: i32 = 100 filter 0..=200;
                cell level: i32 = 50 filter 0..=max;
            }
            "#,
        )
        .unwrap();
    let (max, level) = (parsed.cell_names["max"].0, parsed.cell_names["level"].0);

    parsed.write(max, 10_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 10); // clamped down

    parsed.write(max, 100_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 50); // back to the original 50, not 10
    // ANCHOR_END: raw_value_never_lost
}
```

to:

```rust
#[test]
fn raw_value_never_lost() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/filters/raw_value_never_lost.adm2"
        ))
        .unwrap();
    let (max, level) = (parsed.cell_names["max"].0, parsed.cell_names["level"].0);

    parsed.write(max, 10_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 10); // clamped down

    parsed.write(max, 100_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 50); // back to the original 50, not 10
}
```

Change:

```rust
#[test]
fn range_filter_kind() {
    // ANCHOR: range_filter_kind
    let mut parser = adam_lang_book::support::parser();
    let parsed = parser
        .parse_str("sheet s { cell level: i32 = 50 filter 0..=100; }")
        .unwrap();
    let level = parsed.cell_names["level"].0;
    assert!(matches!(
        parsed.filter_kind(level),
        Some(adam_rs::FilterKind::Range { .. })
    ));
    assert_eq!(parsed.filter_range::<i32>(level), Some((0, 100)));
    // ANCHOR_END: range_filter_kind
}
```

to:

```rust
#[test]
fn range_filter_kind() {
    let mut parser = adam_lang_book::support::parser();
    let parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/filters/range_filter_kind.adm2"
        ))
        .unwrap();
    let level = parsed.cell_names["level"].0;
    assert!(matches!(
        parsed.filter_kind(level),
        Some(adam_rs::FilterKind::Range { .. })
    ));
    assert_eq!(parsed.filter_range::<i32>(level), Some((0, 100)));
}
```

Change:

```rust
#[test]
fn derived_cell_diagnosed_not_corrected() {
    // ANCHOR: derived_cell_diagnosed_not_corrected
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet diagnose_only {
                cell bound: i32 = 100 filter 0..=100;
                cell driver: i32 = 500;

                relationship {
                    bound := driver;
                }
            }
            "#,
        )
        .unwrap();
    parsed.propagate().unwrap();

    let bound = parsed.cell_names["bound"].0;
    assert_eq!(*parsed.read::<i32>(bound).unwrap(), 500); // not clamped
    assert!(parsed.filter_violated_cells().any(|id| id == bound));
    // ANCHOR_END: derived_cell_diagnosed_not_corrected
}
```

to:

```rust
#[test]
fn derived_cell_diagnosed_not_corrected() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/filters/derived_cell_diagnosed_not_corrected.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();

    let bound = parsed.cell_names["bound"].0;
    assert_eq!(*parsed.read::<i32>(bound).unwrap(), 500); // not clamped
    assert!(parsed.filter_violated_cells().any(|id| id == bound));
}
```

Change:

```rust
#[test]
fn must_reference_underscore() {
    // ANCHOR: must_reference_underscore
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str("sheet s { cell x: i32 = 0 filter 5; }") // never mentions `_`
        .err()
        .unwrap();
    assert!(format!("{err}").contains("must reference `_`"));
    // ANCHOR_END: must_reference_underscore
}
```

to:

```rust
#[test]
fn must_reference_underscore() {
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/filters/must_reference_underscore.adm2"
        )) // never mentions `_`
        .err()
        .unwrap();
    assert!(format!("{err}").contains("must reference `_`"));
}
```

Change:

```rust
#[test]
fn tuple_filter_not_supported() {
    // ANCHOR: tuple_filter_not_supported
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str("sheet s { cell x: (i32, i32) = (0, 0) filter _; }") // tuple-typed cell
        .err()
        .unwrap();
    assert!(format!("{err}").contains("tuple"));
    // ANCHOR_END: tuple_filter_not_supported
}
```

to:

```rust
#[test]
fn tuple_filter_not_supported() {
    let mut parser = adam_lang_book::support::parser();
    let err = parser
        .parse_str(include_str!(
            "../book-src/examples/filters/tuple_filter_not_supported.adm2"
        )) // tuple-typed cell
        .err()
        .unwrap();
    assert!(format!("{err}").contains("tuple"));
}
```

- [ ] **Step 3: Update `adam-lang-book/book-src/filters.md`**

Replace:
- `{{#include ../tests/filters.rs:write_never_filters}}` with
  `{{#include examples/filters/write_never_filters.adm2}}`
- `{{#include ../tests/filters.rs:raw_value_never_lost}}` with
  `{{#include examples/filters/raw_value_never_lost.adm2}}`
- `{{#include ../tests/filters.rs:range_filter_kind}}` with
  `{{#include examples/filters/range_filter_kind.adm2}}`
- `{{#include ../tests/filters.rs:derived_cell_diagnosed_not_corrected}}` with
  `{{#include examples/filters/derived_cell_diagnosed_not_corrected.adm2}}`
- `{{#include ../tests/filters.rs:must_reference_underscore}}` with
  `{{#include examples/filters/must_reference_underscore.adm2}}`
- `{{#include ../tests/filters.rs:tuple_filter_not_supported}}` with
  `{{#include examples/filters/tuple_filter_not_supported.adm2}}`

- [ ] **Step 4: Verify**

Run: `cargo test -p adam-lang-book filters` and `mdbook build adam-lang-book`
Expected: tests pass; build succeeds.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Convert filters.md examples to standalone .adm2 files"
```

---

### Task 14: Convert `outputs` chapter (4 examples)

**Files:**
- Create: `adam-lang-book/book-src/examples/outputs/basic_output.adm2`
- Create: `adam-lang-book/book-src/examples/outputs/output_cell_is_terminal.adm2`
- Create: `adam-lang-book/book-src/examples/outputs/requirement_diagnostic.adm2`
- Create: `adam-lang-book/book-src/examples/outputs/multiple_requirements.adm2`
- Modify: `adam-lang-book/tests/outputs.rs`
- Modify: `adam-lang-book/book-src/outputs.md:18,30,45,61`

- [ ] **Step 1: Create the `.adm2` files**

`adam-lang-book/book-src/examples/outputs/basic_output.adm2`:

```
sheet area_demo {
    cell width: i32 = 10;
    cell height: i32 = 20;

    out area := width * height;
}
```

`adam-lang-book/book-src/examples/outputs/output_cell_is_terminal.adm2`:

```
sheet s { cell width: i32 = 10; out area := width * 2; }
```

`adam-lang-book/book-src/examples/outputs/requirement_diagnostic.adm2`:

```
sheet area_demo {
    cell width: i32 = 10;
    cell height: i32 = 20;

    out area: i32 := width * height require {
        not_too_big: area <= 300;
    };
}
```

`adam-lang-book/book-src/examples/outputs/multiple_requirements.adm2`:

```
sheet bounds_demo {
    cell x: i32 = 50;

    out clamped: i32 := x require {
        not_negative: clamped >= 0;
        not_too_big: clamped <= 100;
    };
}
```

- [ ] **Step 2: Update `adam-lang-book/tests/outputs.rs`**

Change:

```rust
#[test]
fn basic_output() {
    // ANCHOR: basic_output
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet area_demo {
                cell width: i32 = 10;
                cell height: i32 = 20;

                out area := width * height;
            }
            "#,
        )
        .unwrap();
    parsed.propagate().unwrap();
    let area = parsed.cell_names["area"].0;
    assert_eq!(*parsed.read::<i32>(area).unwrap(), 200);
    // ANCHOR_END: basic_output
}
```

to:

```rust
#[test]
fn basic_output() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!("../book-src/examples/outputs/basic_output.adm2"))
        .unwrap();
    parsed.propagate().unwrap();
    let area = parsed.cell_names["area"].0;
    assert_eq!(*parsed.read::<i32>(area).unwrap(), 200);
}
```

Change:

```rust
#[test]
fn output_cell_is_terminal() {
    // ANCHOR: output_cell_is_terminal
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str("sheet s { cell width: i32 = 10; out area := width * 2; }")
        .unwrap();
    let output = parsed.output_names["area"];
    let area_cell = parsed.output_cell(output).unwrap();
    let err = parsed.write(area_cell, 999_i32).unwrap_err();
    assert!(matches!(err, adam_rs::Error::TerminalCell));
    // ANCHOR_END: output_cell_is_terminal
}
```

to:

```rust
#[test]
fn output_cell_is_terminal() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/outputs/output_cell_is_terminal.adm2"
        ))
        .unwrap();
    let output = parsed.output_names["area"];
    let area_cell = parsed.output_cell(output).unwrap();
    let err = parsed.write(area_cell, 999_i32).unwrap_err();
    assert!(matches!(err, adam_rs::Error::TerminalCell));
}
```

Change:

```rust
#[test]
fn requirement_diagnostic() {
    // ANCHOR: requirement_diagnostic
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet area_demo {
                cell width: i32 = 10;
                cell height: i32 = 20;

                out area: i32 := width * height require {
                    not_too_big: area <= 300;
                };
            }
            "#,
        )
        .unwrap();
    parsed.propagate().unwrap();
    let output = parsed.output_names["area"];
    assert!(parsed.output_valid(output));

    let width = parsed.cell_names["width"].0;
    parsed.write(width, 50_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(!parsed.output_valid(output));
    // ANCHOR_END: requirement_diagnostic
}
```

to:

```rust
#[test]
fn requirement_diagnostic() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/outputs/requirement_diagnostic.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();
    let output = parsed.output_names["area"];
    assert!(parsed.output_valid(output));

    let width = parsed.cell_names["width"].0;
    parsed.write(width, 50_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(!parsed.output_valid(output));
}
```

Change:

```rust
#[test]
fn multiple_requirements() {
    // ANCHOR: multiple_requirements
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet bounds_demo {
                cell x: i32 = 50;

                out clamped: i32 := x require {
                    not_negative: clamped >= 0;
                    not_too_big: clamped <= 100;
                };
            }
            "#,
        )
        .unwrap();
    let output = parsed.output_names["clamped"];
    let x = parsed.cell_names["x"].0;

    parsed.write(x, -10_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(parsed.violated_requirements(output).count(), 1);
    assert!(!parsed.output_valid(output));
    // ANCHOR_END: multiple_requirements
}
```

to:

```rust
#[test]
fn multiple_requirements() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/outputs/multiple_requirements.adm2"
        ))
        .unwrap();
    let output = parsed.output_names["clamped"];
    let x = parsed.cell_names["x"].0;

    parsed.write(x, -10_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(parsed.violated_requirements(output).count(), 1);
    assert!(!parsed.output_valid(output));
}
```

- [ ] **Step 3: Update `adam-lang-book/book-src/outputs.md`**

Replace:
- `{{#include ../tests/outputs.rs:basic_output}}` with
  `{{#include examples/outputs/basic_output.adm2}}`
- `{{#include ../tests/outputs.rs:output_cell_is_terminal}}` with
  `{{#include examples/outputs/output_cell_is_terminal.adm2}}`
- `{{#include ../tests/outputs.rs:requirement_diagnostic}}` with
  `{{#include examples/outputs/requirement_diagnostic.adm2}}`
- `{{#include ../tests/outputs.rs:multiple_requirements}}` with
  `{{#include examples/outputs/multiple_requirements.adm2}}`

- [ ] **Step 4: Verify**

Run: `cargo test -p adam-lang-book outputs` and `mdbook build adam-lang-book`
Expected: tests pass; build succeeds.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Convert outputs.md examples to standalone .adm2 files"
```

---

### Task 15: Convert `style` chapter (1 example)

**Files:**
- Create: `adam-lang-book/book-src/examples/style/canonical_formatting.adm2`
- Modify: `adam-lang-book/tests/style.rs`
- Modify: `adam-lang-book/book-src/style.md:38`

- [ ] **Step 1: Create the `.adm2` file**

`adam-lang-book/book-src/examples/style/canonical_formatting.adm2`:

```
sheet s{cell x:i32=1;cell y:i32=2;}
```

- [ ] **Step 2: Update `adam-lang-book/tests/style.rs`**

Change:

```rust
#[test]
fn canonical_formatting() {
    // ANCHOR: canonical_formatting
    let mut ast_parser = adam_lang::AdamAstParser::new();
    let sheet = ast_parser
        .parse_str("sheet s{cell x:i32=1;cell y:i32=2;}")
        .unwrap();
    assert!(sheet.errors.is_empty());

    let formatted = adam_lang::format_sheet(&sheet);
    assert_eq!(
        formatted,
        "sheet s {\n    cell x: i32 = 1;\n    cell y: i32 = 2;\n}\n"
    );
    // ANCHOR_END: canonical_formatting
}
```

to:

```rust
#[test]
fn canonical_formatting() {
    let mut ast_parser = adam_lang::AdamAstParser::new();
    let sheet = ast_parser
        .parse_str(include_str!(
            "../book-src/examples/style/canonical_formatting.adm2"
        ))
        .unwrap();
    assert!(sheet.errors.is_empty());

    let formatted = adam_lang::format_sheet(&sheet);
    assert_eq!(
        formatted,
        "sheet s {\n    cell x: i32 = 1;\n    cell y: i32 = 2;\n}\n"
    );
}
```

- [ ] **Step 3: Update `adam-lang-book/book-src/style.md`**

Replace `{{#include ../tests/style.rs:canonical_formatting}}` with
`{{#include examples/style/canonical_formatting.adm2}}`.

- [ ] **Step 4: Verify**

Run: `cargo test -p adam-lang-book style` and `mdbook build adam-lang-book`
Expected: tests pass; build succeeds.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Convert style.md example to a standalone .adm2 file"
```

---

### Task 16: Convert `tutorial` chapter (5 examples)

**Files:**
- Create: `adam-lang-book/book-src/examples/tutorial/first_sheet.adm2`
- Create: `adam-lang-book/book-src/examples/tutorial/multiplication_triangle.adm2`
- Create: `adam-lang-book/book-src/examples/tutorial/mode_demo.adm2`
- Create: `adam-lang-book/book-src/examples/tutorial/clamp_demo.adm2`
- Create: `adam-lang-book/book-src/examples/tutorial/area_with_requirement.adm2`
- Modify: `adam-lang-book/tests/tutorial.rs`
- Modify: `adam-lang-book/book-src/tutorial.md:40,81,125,145,191`

- [ ] **Step 1: Create the `.adm2` files**

`adam-lang-book/book-src/examples/tutorial/first_sheet.adm2`:

```
sheet hello {
    cell width: i32 = 1920;
    cell height: i32 = 1080;
}
```

`adam-lang-book/book-src/examples/tutorial/multiplication_triangle.adm2`:

```
sheet triangle {
    cell c = 0.0;
    cell a = 2.0;
    cell b = 3.0;

    relationship {
        c := a * b;
        a := c / b;
        b := c / a;
    }
}
```

`adam-lang-book/book-src/examples/tutorial/mode_demo.adm2`:

```
sheet mode_demo {
    cell p: i32 = 0;
    cell x: f64 = 1.0;
    cell y: f64 = 2.0;

    conditional p {
        0i32 => {
            relationship {
                x := y;
            }
        }
        1i32 => {
            relationship {
                y := x;
            }
        }
        _ => {
            relationship {
                x := 0.0;
            }
        }
    }
}
```

`adam-lang-book/book-src/examples/tutorial/clamp_demo.adm2`:

```
sheet volume { cell level: i32 = 50 filter 0..=100; }
```

`adam-lang-book/book-src/examples/tutorial/area_with_requirement.adm2`:

```
sheet area_demo {
    cell width: i32 = 10;
    cell height: i32 = 20;

    out area: i32 := width * height require {
        not_too_big: area <= 300;
    };
}
```

- [ ] **Step 2: Update `adam-lang-book/tests/tutorial.rs`**

Change:

```rust
#[test]
fn first_sheet() {
    // ANCHOR: first_sheet
    let mut parser = adam_lang_book::support::parser();
    let parsed = parser
        .parse_str(
            r#"
            sheet hello {
                cell width: i32 = 1920;
                cell height: i32 = 1080;
            }
            "#,
        )
        .unwrap();

    let width = parsed.cell_names["width"].0;
    assert_eq!(*parsed.read::<i32>(width).unwrap(), 1920);
    // ANCHOR_END: first_sheet
}
```

to:

```rust
#[test]
fn first_sheet() {
    let mut parser = adam_lang_book::support::parser();
    let parsed = parser
        .parse_str(include_str!("../book-src/examples/tutorial/first_sheet.adm2"))
        .unwrap();

    let width = parsed.cell_names["width"].0;
    assert_eq!(*parsed.read::<i32>(width).unwrap(), 1920);
}
```

Change:

```rust
#[test]
fn multiplication_triangle() {
    // ANCHOR: multiplication_triangle
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet triangle {
                cell c = 0.0;
                cell a = 2.0;
                cell b = 3.0;

                relationship {
                    c := a * b;
                    a := c / b;
                    b := c / a;
                }
            }
            "#,
        )
        .unwrap();
    parsed.propagate().unwrap();

    let (a, b, c) = (
        parsed.cell_names["a"].0,
        parsed.cell_names["b"].0,
        parsed.cell_names["c"].0,
    );
    assert_eq!(*parsed.read::<f64>(c).unwrap(), 6.0); // 2.0 * 3.0, derived

    // Write b: it becomes the freshest cell, so the solver keeps both a and b as
    // sources and re-derives c from them.
    parsed.write(b, 5.0).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(a).unwrap(), 2.0); // untouched
    assert_eq!(*parsed.read::<f64>(b).unwrap(), 5.0); // just written
    assert_eq!(*parsed.read::<f64>(c).unwrap(), 10.0); // 2.0 * 5.0, re-derived
    // ANCHOR_END: multiplication_triangle
}
```

to:

```rust
#[test]
fn multiplication_triangle() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/tutorial/multiplication_triangle.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();

    let (a, b, c) = (
        parsed.cell_names["a"].0,
        parsed.cell_names["b"].0,
        parsed.cell_names["c"].0,
    );
    assert_eq!(*parsed.read::<f64>(c).unwrap(), 6.0); // 2.0 * 3.0, derived

    // Write b: it becomes the freshest cell, so the solver keeps both a and b as
    // sources and re-derives c from them.
    parsed.write(b, 5.0).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(a).unwrap(), 2.0); // untouched
    assert_eq!(*parsed.read::<f64>(b).unwrap(), 5.0); // just written
    assert_eq!(*parsed.read::<f64>(c).unwrap(), 10.0); // 2.0 * 5.0, re-derived
}
```

Change:

```rust
#[test]
fn mode_demo() {
    // ANCHOR: mode_demo
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet mode_demo {
                cell p: i32 = 0;
                cell x: f64 = 1.0;
                cell y: f64 = 2.0;

                conditional p {
                    0i32 => {
                        relationship {
                            x := y;
                        }
                    }
                    1i32 => {
                        relationship {
                            y := x;
                        }
                    }
                    _ => {
                        relationship {
                            x := 0.0;
                        }
                    }
                }
            }
            "#,
        )
        .unwrap();

    let p = parsed.cell_names["p"].0;
    let x = parsed.cell_names["x"].0;

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 2.0); // p == 0: x := y

    parsed.write(p, 2_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 0.0); // p matches no named branch: default
    // ANCHOR_END: mode_demo
}
```

to:

```rust
#[test]
fn mode_demo() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!("../book-src/examples/tutorial/mode_demo.adm2"))
        .unwrap();

    let p = parsed.cell_names["p"].0;
    let x = parsed.cell_names["x"].0;

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 2.0); // p == 0: x := y

    parsed.write(p, 2_i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(x).unwrap(), 0.0); // p matches no named branch: default
}
```

Change:

```rust
#[test]
fn clamp_demo() {
    // ANCHOR: clamp_demo
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str("sheet volume { cell level: i32 = 50 filter 0..=100; }")
        .unwrap();
    let level = parsed.cell_names["level"].0;

    parsed.write(level, 500_i32).unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 500); // still raw

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 100); // now clamped
    // ANCHOR_END: clamp_demo
}
```

to:

```rust
#[test]
fn clamp_demo() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!("../book-src/examples/tutorial/clamp_demo.adm2"))
        .unwrap();
    let level = parsed.cell_names["level"].0;

    parsed.write(level, 500_i32).unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 500); // still raw

    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(level).unwrap(), 100); // now clamped
}
```

Change:

```rust
#[test]
fn area_with_requirement() {
    // ANCHOR: area_with_requirement
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(
            r#"
            sheet area_demo {
                cell width: i32 = 10;
                cell height: i32 = 20;

                out area: i32 := width * height require {
                    not_too_big: area <= 300;
                };
            }
            "#,
        )
        .unwrap();
    parsed.propagate().unwrap();

    let output = parsed.output_names["area"];
    assert!(parsed.output_valid(output)); // 10 * 20 == 200 <= 300

    let width = parsed.cell_names["width"].0;
    parsed.write(width, 50_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(!parsed.output_valid(output)); // 50 * 20 == 1000 > 300
    // ANCHOR_END: area_with_requirement
}
```

to:

```rust
#[test]
fn area_with_requirement() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/tutorial/area_with_requirement.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();

    let output = parsed.output_names["area"];
    assert!(parsed.output_valid(output)); // 10 * 20 == 200 <= 300

    let width = parsed.cell_names["width"].0;
    parsed.write(width, 50_i32).unwrap();
    parsed.propagate().unwrap();
    assert!(!parsed.output_valid(output)); // 50 * 20 == 1000 > 300
}
```

- [ ] **Step 3: Update `adam-lang-book/book-src/tutorial.md`**

Replace:
- `{{#include ../tests/tutorial.rs:first_sheet}}` with
  `{{#include examples/tutorial/first_sheet.adm2}}`
- `{{#include ../tests/tutorial.rs:multiplication_triangle}}` with
  `{{#include examples/tutorial/multiplication_triangle.adm2}}`
- `{{#include ../tests/tutorial.rs:mode_demo}}` with
  `{{#include examples/tutorial/mode_demo.adm2}}`
- `{{#include ../tests/tutorial.rs:clamp_demo}}` with
  `{{#include examples/tutorial/clamp_demo.adm2}}`
- `{{#include ../tests/tutorial.rs:area_with_requirement}}` with
  `{{#include examples/tutorial/area_with_requirement.adm2}}`

- [ ] **Step 4: Verify**

Run: `cargo test -p adam-lang-book tutorial` and `mdbook build adam-lang-book`
Expected: tests pass; build succeeds.

Run the full book suite once more to confirm all 27 examples now come from `.adm2` files:
`cargo test -p adam-lang-book` and `grep -rn "tests/.*\.rs:" adam-lang-book/book-src/*.md`
Expected: the `grep` returns no results — every `{{#include}}` in the book now points at a
`.adm2` file, not a Rust test anchor.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Convert tutorial.md examples to standalone .adm2 files"
```

---

## Phase 4 — Live mounting: wasm crate, preprocessor, CI

### Task 17: Build the `adam-lang-book-live` wasm mount crate

**Files:**
- Create: `adam-lang-book-live/Cargo.toml`
- Create: `adam-lang-book-live/src/lib.rs`
- Modify: `Cargo.toml:22-35` (workspace `members`)

**Interfaces:**
- Consumes: `adam_web_ui::{SheetInspector, build_sheet}`, `dioxus::web::{Config,
  launch::launch_virtual_dom}` (confirmed working for multiple independent mounts by Task 8).
- Produces: a `#[wasm_bindgen] pub fn mount(element_id: &str, source: &str)` exported from the
  compiled `.wasm`, called once per `.adam-live` div by the bootstrap script (Task 19).

- [ ] **Step 1: Create `adam-lang-book-live/Cargo.toml`**

```toml
[package]
name = "adam-lang-book-live"
version = "0.1.0"
edition = "2024"
description = "wasm-bindgen entry point mounting a live SheetInspector for one adam-lang-book example"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
adam-web-ui = { path = "../adam-web-ui" }
adam-rs = { path = "../adam-rs" }
dioxus = { version = "0.7.10", features = ["web"] }
wasm-bindgen = "0.2"

[lints]
workspace = true
```

- [ ] **Step 2: Create `adam-lang-book-live/src/lib.rs`**

```rust
//! wasm-bindgen entry point that mounts one live [`adam_web_ui::SheetInspector`] per call, for
//! `adam-lang-book`'s live examples. Each `.adm2` example on a book page gets its own
//! independent mount (confirmed to coexist safely — see
//! `docs/superpowers/plans/2026-08-27-live-adam-book-examples.md`'s Task 8 spike); there is no
//! shared state between them.

use adam_web_ui::{SheetInspector, build_sheet};
use dioxus::prelude::*;
use wasm_bindgen::prelude::*;

#[derive(Clone, PartialEq, Props)]
struct RootProps {
    source: String,
    name: String,
}

/// Parses `props.source` once, then renders either a live [`SheetInspector`] (on success) or
/// the formatted diagnostic (on parse failure), matching how a propagate failure alongside a
/// successfully built sheet renders both.
#[component]
fn Root(props: RootProps) -> Element {
    let outcome = use_hook(|| build_sheet(&props.source, &props.name));
    let source_text = use_memo({
        let source = props.source.clone();
        move || source.clone()
    });
    let source_name = use_memo({
        let name = props.name.clone();
        move || name.clone()
    });

    match outcome.sheet_labels {
        Some((sheet, labels)) => {
            let sheet = use_signal(|| sheet);
            let labels = use_signal(|| labels);
            let error = outcome.error.clone();
            rsx! {
                SheetInspector { sheet, labels, source_text, source_name }
                if let Some(err) = error {
                    pre { class: "adam-live-error", "{err}" }
                }
            }
        }
        None => {
            let error = outcome.error.unwrap_or_default();
            rsx! {
                pre { class: "adam-live-error", "{error}" }
            }
        }
    }
}

/// Mounts a live [`SheetInspector`] for `source` into the DOM element with id `element_id`.
///
/// - Precondition: an element with id `element_id` already exists in the document — the
///   mdBook `live-examples` preprocessor is what creates it (see
///   `adam-lang-book-preprocessor`).
#[wasm_bindgen]
pub fn mount(element_id: &str, source: &str) {
    let props = RootProps {
        source: source.to_string(),
        name: format!("{element_id}.adm2"),
    };
    let vdom = VirtualDom::new_with_props(Root, props);
    let config = dioxus::web::Config::new().rootname(element_id);
    dioxus::web::launch::launch_virtual_dom(vdom, config);
}
```

- [ ] **Step 3: Add the crate to the workspace**

In `Cargo.toml`, add `"adam-lang-book-live"` to `[workspace] members` (alphabetically, after
`"adam-lang-book"`).

- [ ] **Step 4: Verify it builds for both native and wasm targets**

Run: `cargo check -p adam-lang-book-live` (native, catches ordinary type errors quickly) and
`cargo build -p adam-lang-book-live --target wasm32-unknown-unknown` (installing the target
first if needed: `rustup target add wasm32-unknown-unknown`)
Expected: both succeed with zero warnings.

**Note on test coverage for this crate:** the design spec called for a unit test of the
parse-error-renders-diagnostic-instead-of-panicking path. That path is fully exercised
already: `Root`'s body never calls `.unwrap()`/`.expect()` on `outcome` — it match-branches on
`outcome.sheet_labels` and falls back to `outcome.error.unwrap_or_default()` only in the
`None` arm, where `error` is guaranteed `Some` by `build_sheet`'s own contract (`sheet_labels`
is `None` only on parse failure, and a parse failure always sets `error`). `Root`'s branching
is a direct, unwrap-free passthrough of `BuildOutcome`'s already-contract-tested shape (see
`adam-web-ui/src/build.rs`'s `build_sheet_parse_error_has_no_sheet_and_formatted_message`), so
per this repo's rule that a genuine passthrough doesn't need its own dedicated test, no new
test is added here — Task 21's real-browser check of `relationships.html`'s `conflict_error`
example is what confirms this end-to-end.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Add the adam-lang-book-live wasm mount crate"
```

---

### Task 18: Build the `adam-lang-book-preprocessor` mdBook preprocessor

**Files:**
- Create: `adam-lang-book-preprocessor/Cargo.toml`
- Create: `adam-lang-book-preprocessor/src/main.rs`
- Modify: `Cargo.toml:22-36` (workspace `members`)

**Interfaces:**
- Produces: a `live-examples` mdBook preprocessor binary that, for every `{{#include
  examples/<chapter>/<name>.adm2}}` occurrence in a chapter's raw markdown, inserts
  immediately after it: `<div class="adam-live" data-example="<chapter>/<name>"></div>` —
  except for the excluded `expressions/no_standard_library` example (see Global Constraints).

- [ ] **Step 1: Create `adam-lang-book-preprocessor/Cargo.toml`**

```toml
[package]
name = "adam-lang-book-preprocessor"
version = "0.1.0"
edition = "2024"
description = "mdBook preprocessor inserting a live-mount div after each .adm2 example include"

[[bin]]
name = "mdbook-live-examples"
path = "src/main.rs"

[dependencies]
mdbook = { version = "0.4", default-features = false }
regex = "1"
serde_json = "1"

[lints]
workspace = true
```

(`default-features = false` on `mdbook`: this preprocessor only needs the `Preprocessor`
trait and book data model, not mdBook's own CLI/rendering machinery.)

- [ ] **Step 2: Create `adam-lang-book-preprocessor/src/main.rs`**

```rust
//! `mdbook-live-examples`: an mdBook preprocessor that inserts a live-mount `<div>`
//! immediately after every `{{#include examples/<chapter>/<name>.adm2}}` directive in a
//! chapter, so the pairing between a shown example and its live widget can never drift as
//! examples move or new ones are added.
//!
//! Registered in `book.toml` as `[preprocessor.live-examples]`.

use mdbook::book::{Book, BookItem};
use mdbook::errors::Error;
use mdbook::preprocess::{CmdPreprocessor, Preprocessor, PreprocessorContext};
use regex::Regex;
use std::io;

/// Examples deliberately excluded from live mounting: sources whose whole point depends on a
/// parser configuration [`adam_web_ui::build_sheet`] doesn't use (here, the *absence* of
/// `cel-std`), so mounting them live would silently show different behavior than the
/// surrounding prose describes. See this plan's Global Constraints for why.
const NO_LIVE_MOUNT: &[&str] = &["expressions/no_standard_library"];

/// Matches an `.adm2` include directive, capturing `<chapter>/<name>` (without the
/// `.adm2` extension) for use as both the mount div's `data-example` value and the
/// [`NO_LIVE_MOUNT`] lookup key.
///
/// - Postcondition: only matches includes ending in `.adm2` — an ordinary
///   `{{#include ../tests/foo.rs:anchor}}` (if any remain elsewhere in the book) never
///   matches.
fn adm2_include_regex() -> Regex {
    Regex::new(r"\{\{#include\s+examples/([A-Za-z0-9_]+/[A-Za-z0-9_]+)\.adm2\s*\}\}").unwrap()
}

/// Inserts a live-mount `<div>` immediately after each `.adm2` include in `content`, except
/// for names in [`NO_LIVE_MOUNT`].
///
/// - Complexity: O(n + m) in the length of `content` plus the number of matches.
fn inject_mount_points(content: &str, re: &Regex) -> String {
    let mut out = String::with_capacity(content.len());
    let mut last_end = 0;
    for capture in re.captures_iter(content) {
        let whole = capture.get(0).unwrap();
        let name = &capture[1];
        out.push_str(&content[last_end..whole.end()]);
        if !NO_LIVE_MOUNT.contains(&name) {
            out.push_str(&format!(
                "\n<div class=\"adam-live\" data-example=\"{name}\"></div>\n"
            ));
        }
        last_end = whole.end();
    }
    out.push_str(&content[last_end..]);
    out
}

struct LiveExamples;

impl Preprocessor for LiveExamples {
    fn name(&self) -> &str {
        "live-examples"
    }

    fn run(&self, _ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        let re = adm2_include_regex();
        book.for_each_mut(|item| {
            if let BookItem::Chapter(chapter) = item {
                chapter.content = inject_mount_points(&chapter.content, &re);
            }
        });
        Ok(book)
    }

    fn supports_renderer(&self, renderer: &str) -> bool {
        renderer == "html"
    }
}

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("supports") {
        // mdBook calls `mdbook-live-examples supports <renderer>` to ask whether this
        // preprocessor applies; exit 0 to say yes, non-zero to say no.
        let renderer = args.get(2).map(String::as_str).unwrap_or_default();
        std::process::exit(if LiveExamples.supports_renderer(renderer) {
            0
        } else {
            1
        });
    }

    let (ctx, book) = CmdPreprocessor::parse_input(io::stdin())?;
    let processed = LiveExamples.run(&ctx, book)?;
    serde_json::to_writer(io::stdout(), &processed)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_mount_points_inserts_a_div_after_an_adm2_include() {
        let re = adm2_include_regex();
        let content = "some prose\n\n{{#include examples/cells/tuple_typed_cell.adm2}}\n\nmore prose";
        let result = inject_mount_points(content, &re);
        assert!(result.contains(
            "{{#include examples/cells/tuple_typed_cell.adm2}}\n<div class=\"adam-live\" data-example=\"cells/tuple_typed_cell\"></div>"
        ));
    }

    #[test]
    fn inject_mount_points_leaves_non_adm2_includes_untouched() {
        let re = adm2_include_regex();
        let content = "{{#include ../tests/tutorial.rs:first_sheet}}";
        let result = inject_mount_points(content, &re);
        assert_eq!(result, content);
    }

    #[test]
    fn inject_mount_points_skips_the_no_live_mount_list() {
        let re = adm2_include_regex();
        let content = "{{#include examples/expressions/no_standard_library.adm2}}";
        let result = inject_mount_points(content, &re);
        assert!(!result.contains("adam-live"));
    }

    #[test]
    fn inject_mount_points_handles_multiple_includes_in_one_chapter() {
        let re = adm2_include_regex();
        let content = "{{#include examples/cells/a.adm2}}\n\ntext\n\n{{#include examples/cells/b.adm2}}";
        let result = inject_mount_points(content, &re);
        assert_eq!(result.matches("adam-live").count(), 2);
        assert!(result.contains("data-example=\"cells/a\""));
        assert!(result.contains("data-example=\"cells/b\""));
    }

    #[test]
    fn live_examples_only_supports_html_renderer() {
        assert!(LiveExamples.supports_renderer("html"));
        assert!(!LiveExamples.supports_renderer("epub"));
    }
}
```

- [ ] **Step 3: Add the crate to the workspace**

In `Cargo.toml`, add `"adam-lang-book-preprocessor"` to `[workspace] members`.

- [ ] **Step 4: Verify**

Run: `cargo test -p adam-lang-book-preprocessor` and `cargo build -p
adam-lang-book-preprocessor`
Expected: all tests pass; build succeeds with zero warnings.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Add the mdbook-live-examples preprocessor"
```

---

### Task 19: Wire the preprocessor and wasm bundle into the book build

**Files:**
- Modify: `adam-lang-book/book.toml`
- Create: `adam-lang-book/book-src/theme/adam-live-bootstrap.js`
- Create: `adam-lang-book/book-src/theme/adam-live.css`

**Interfaces:**
- Consumes: `adam-lang-book-live`'s compiled wasm/js output (built by CI in Task 20; built
  locally for this task's own verification via `dx build`), `begin/assets/swc.js` and
  `begin/assets/inspector.css` (the same Spectrum bundle and inspector stylesheet `begin`
  already loads).

- [ ] **Step 1: Update `adam-lang-book/book.toml`**

```toml
[book]
title = "The Adam Programming Language"
description = "A tutorial and reference manual for the Adam language"
authors = ["stlab"]
language = "en"
src = "book-src"

[build]
build-dir = "book-dist"

[preprocessor.live-examples]
command = "mdbook-live-examples"

[output.html]
git-repository-url = "https://github.com/stlab/cel-rs"
edit-url-template = "https://github.com/stlab/cel-rs/edit/main/adam-lang-book/{path}"
additional-css = ["book-src/theme/adam-live.css"]
additional-js = ["book-src/theme/adam-live-bootstrap.js"]
```

(mdBook resolves `mdbook-live-examples` on `$PATH`; CI installs it via `cargo install --path
adam-lang-book-preprocessor` before running `mdbook build` — see Task 20. Locally, run
`cargo install --path adam-lang-book-preprocessor --force` once before `mdbook build
adam-lang-book`.)

- [ ] **Step 2: Create `adam-lang-book/book-src/theme/adam-live-bootstrap.js`**

```javascript
// Mounts a live SheetInspector into every `.adam-live` div the live-examples preprocessor
// inserted. Each div's `data-example` (e.g. "cells/tuple_typed_cell") names one of the
// `.wasm-embedded-examples.json` manifest's entries; the manifest and the compiled
// adam-lang-book-live wasm/js bundle are both copied alongside this script by the book build
// (see the CI workflow changes).
(async () => {
  const mounts = document.querySelectorAll(".adam-live");
  if (mounts.length === 0) {
    return;
  }

  const [{ default: init, mount }, manifest] = await Promise.all([
    import("./adam_lang_book_live.js"),
    fetch("./adam-live-examples.json").then((r) => r.json()),
  ]);
  await init();

  mounts.forEach((div, index) => {
    const name = div.dataset.example;
    const source = manifest[name];
    if (source === undefined) {
      console.error(`adam-live: no embedded source for "${name}"`);
      return;
    }
    const id = `adam-live-${index}`;
    div.id = id;
    mount(id, source);
  });
})();
```

- [ ] **Step 3: Create `adam-lang-book/book-src/theme/adam-live.css`**

```css
.adam-live {
  margin: 0.5em 0 1.5em 0;
  padding: 12px;
  border: 1px solid #ccc;
  border-radius: 4px;
}

.adam-live-error {
  color: #a4262c;
  white-space: pre-wrap;
  font-family: monospace;
}
```

- [ ] **Step 4: Verify locally**

This task's own verification requires artifacts Task 20 formalizes as a CI step; do it once
by hand now to confirm the wiring is correct before automating it:

1. `cargo install --path adam-lang-book-preprocessor --force`
2. `dx build --platform web --release --package adam-lang-book-live` and locate its output
   `.wasm`/`.js` (confirm the exact output directory by running this and inspecting
   `target/dx/adam-lang-book-live/release/web/public/` — dioxus-cli's exact layout can vary by
   version; adjust the paths below to match what's actually produced).
3. Copy the produced `adam_lang_book_live.js`/`.wasm` into `adam-lang-book/book-src/theme/`.
4. Generate `adam-lang-book/book-src/theme/adam-live-examples.json`: a flat `{"chapter/name":
   "<.adm2 source>", ...}` map covering every file under `adam-lang-book/book-src/examples/`
   except `expressions/no_standard_library` (matching `NO_LIVE_MOUNT`) — a one-off shell
   script is fine for this manual check (Task 20 formalizes generating it in CI).
5. Copy `begin/assets/swc.js` and `begin/assets/inspector.css` into
   `adam-lang-book/book-src/theme/` too, and add `document`-level `<script type="module">`/
   `<link>` tags for them via `book.toml`'s `additional-js`/`additional-css` (same mechanism
   as Step 1) — the mounted `SheetInspector` needs the same Spectrum custom-element bundle
   `begin` loads, or its fields render as unstyled/undefined elements.
6. `mdbook build adam-lang-book`
7. Serve `adam-lang-book/book-dist/` (e.g. `python3 -m http.server 8000 --directory
   adam-lang-book/book-dist`) and open `cells.html` in a real browser.

Expected: a live, editable cell inspector appears below at least one `.adm2` example on the
page, and editing a value updates it live. Delete the manually copied
`.wasm`/`.js`/`swc.js`/`inspector.css`/`adam-live-examples.json` files from `book-src/theme/`
after confirming — Task 20 makes their generation part of the CI build instead of a committed
artifact.

- [ ] **Step 5: Commit**

```bash
git add adam-lang-book/book.toml adam-lang-book/book-src/theme/adam-live-bootstrap.js adam-lang-book/book-src/theme/adam-live.css
git commit -m "Wire the live-examples preprocessor and bootstrap script into the book build"
```

---

### Task 20: Extend CI to build and publish the live bundle

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/docs.yml`

**Interfaces:** none — this only changes CI orchestration.

- [ ] **Step 1: Add a manifest-generation step**

Since both workflows need the same `adam-live-examples.json` manifest and the same copied
`begin/assets/swc.js`/`inspector.css`, add this as an `xtask` subcommand rather than
duplicating a shell one-liner in two YAML files: create `xtask/src/live_book_assets.rs` with
a function that walks `adam-lang-book/book-src/examples/`, builds the `{"chapter/name":
source}` JSON map (skipping `expressions/no_standard_library.adm2`, matching the
preprocessor's own `NO_LIVE_MOUNT`), writes it to
`adam-lang-book/book-src/theme/adam-live-examples.json`, and copies
`begin/assets/swc.js`/`begin/assets/inspector.css` into `adam-lang-book/book-src/theme/`.
Wire it into `xtask`'s existing command dispatch (check `xtask/src/main.rs` for the
established pattern) as `cargo run -p xtask -- prepare-live-book-assets`.

- [ ] **Step 2: Update `.github/workflows/ci.yml`**

Insert these steps after "Install mdBook" and before "Build the adam-lang book" (so the
preprocessor and assets exist before `mdbook build` runs):

```yaml
    - name: Install the live-examples mdBook preprocessor
      run: cargo install --path adam-lang-book-preprocessor

    - name: Build the adam-lang-book-live wasm bundle
      run: |
        cargo install dioxus-cli --locked
        dx build --platform web --release --package adam-lang-book-live

    - name: Prepare live-book assets (manifest + Spectrum bundle + wasm/js output)
      run: cargo run -p xtask -- prepare-live-book-assets
```

(The exact `dx build` output path that `prepare-live-book-assets` copies the `.wasm`/`.js`
from must be confirmed once via a real run — see Task 19 Step 4's note on this — and encoded
in the `xtask` subcommand itself, not duplicated in the workflow YAML.)

- [ ] **Step 3: Update `.github/workflows/docs.yml`**

Insert the same three steps (Install the live-examples mdBook preprocessor / Build the
adam-lang-book-live wasm bundle / Prepare live-book assets) after "Install mdBook" and before
"Build the adam-lang book".

- [ ] **Step 4: Verify**

Push this branch (or open a draft PR) and confirm the `ci.yml` workflow run's "Build the
adam-lang book" step succeeds with the new preprocessor and wasm bundle in place — the
existing "Catches a broken `{{#include}}`..." comment on that step now also implicitly covers
a broken live-mount wiring, since a missing `mdbook-live-examples` binary or wasm bundle would
fail `mdbook build` outright the same way a broken include does today.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Build and publish the live-book wasm bundle in CI"
```

---

## Phase 5 — End-to-end verification and handoff

### Task 21: Full end-to-end verification and handoff doc

**Files:**
- Create: `docs/superpowers/2026-08-27-live-adam-book-examples-handoff.md`

- [ ] **Step 1: Run the complete check suite one more time**

```bash
cargo fmt --all -- --check
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace
```

Expected: everything passes with zero warnings.

- [ ] **Step 2: Manually verify the published-shape output in a real browser**

Follow Task 19 Step 4's procedure once more end-to-end (via the now-automated
`xtask prepare-live-book-assets` instead of the manual copy), serving the built
`adam-lang-book/book-dist/` and checking, in a real browser, across at least three different
chapter pages (e.g. `tutorial.html`, `relationships.html`, `filters.html`):

- Every live-mounted example renders an editable cell list below its static code block.
- Editing a value updates dependent cells live (e.g. `tutorial.html`'s multiplication
  triangle: writing `b` re-derives `c`).
- A deliberately-invalid example (e.g. `relationships.html`'s `conflict_error`) renders the
  formatted diagnostic, not a blank page or a JS console error.
- `expressions.html`'s `no_standard_library` example shows **no** live widget below it (per
  the `NO_LIVE_MOUNT` exclusion).
- Each mounted widget is independent — editing one example's cells never affects another's.

- [ ] **Step 3: Write the handoff doc**

Create `docs/superpowers/2026-08-27-live-adam-book-examples-handoff.md` following this
repo's established handoff format (see `docs/superpowers/2026-07-18-phase-3-handoff.md` for
the template), summarizing: all 21 tasks completed, the `adam-web-ui`/
`adam-lang-book-live`/`adam-lang-book-preprocessor` crates now in the workspace, the graph
view and native-platform bindings explicitly deferred (per the spec's Non-goals), and any
follow-up issues opened along the way (e.g. if the Task 8 spike's fallback design had to be
used instead of the primary one, or if `dx build`'s actual output layout differed from what
Task 19/20 assumed and required an `xtask` adjustment).

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/2026-08-27-live-adam-book-examples-handoff.md
git commit -m "Add handoff doc for the live Adam book examples work"
```
