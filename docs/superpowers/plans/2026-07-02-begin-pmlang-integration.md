# begin: pm-lang Integration & Error Reporting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `begin`'s hard-coded demo property model with one described in
live-editable pm-lang source, and surface formatted (rustc-style, caret-annotated)
parser and runtime diagnostics in the UI.

**Architecture:** `pm-lang::PmParser::parse_str` is extended to return a
`ParsedSheet` (sheet + declaration-ordered cell names) instead of a bare `Sheet`.
`begin` gains a pure `build_sheet(source: &str) -> BuildOutcome` function that
parses, builds `Labels` from the parsed cell names, and propagates once,
producing either a `(Sheet, Labels)` pair, a formatted error string, or both. A
new collapsible `SourcePanel` component drives this function on an explicit
"Apply" click; the `Inspector`'s existing cell-write path is extended to format
runtime errors through the same helper.

**Tech Stack:** Rust 2024, Dioxus 0.7 (desktop), `pm-lang`, `property-model`,
`cel-parser` (`FormatRustcStyle`, `ParseError::format_rustc_style`),
`annotate-snippets` (`Renderer::plain()`), `indexmap`.

## Global Constraints

- Rust edition: 2024 (matches workspace)
- `cargo fmt --all` required before every commit (pre-commit hook enforced)
- `cargo clippy --workspace --exclude begin -- -D warnings` must pass
- `cargo clippy -p begin --no-default-features -- -D warnings` must pass (begin is checked separately to avoid platform-specific renderer deps)
- `cargo test --workspace` must pass before each commit
- `missing_docs = "warn"` is a workspace lint — every `pub` item needs a `///` contract-style doc comment (Summary, `# Errors` for fallible fns, `/// - Complexity: O(?)` if non-O(1))
- No `unsafe`, no `unwrap` in production code paths (tests may use `unwrap`/`expect`)
- Design reference: `docs/superpowers/specs/2026-07-02-begin-pmlang-integration-design.md`

---

## File Map

| File | Responsibility |
|---|---|
| `pm-lang/Cargo.toml` | Add `indexmap` dependency |
| `pm-lang/src/parser.rs` | `ParsedSheet` (sheet + ordered cell names); `parse_str` returns it |
| `pm-lang/src/lib.rs` | Re-export `ParsedSheet` |
| `begin/Cargo.toml` | Add `pm-lang`, `cel-parser`, `annotate-snippets` dependencies |
| `begin/src/bridge.rs` | `labels_from_cell_names`; `format_property_model_error` |
| `begin/src/source_panel.rs` | `build_sheet`/`BuildOutcome`; `SourcePanel` component |
| `begin/src/app.rs` | `DEMO_SOURCE`; wires `SourcePanel` and new signals into the layout |
| `begin/src/inspector.rs` | Threads `error`/`applied_source` props; formats write/propagate errors |
| `begin/src/main.rs` | Registers `source_panel` module |

---

### Task 1: pm-lang — `ParsedSheet` with declaration-ordered cell names

**Files:**
- Modify: `pm-lang/Cargo.toml`
- Modify: `pm-lang/src/parser.rs`
- Modify: `pm-lang/src/lib.rs`

**Interfaces:**
- Consumes: nothing new
- Produces: `pub struct ParsedSheet { pub sheet: Sheet, pub cell_names: IndexMap<String, (CellId, TypeId)> }` implementing `Deref`/`DerefMut<Target = Sheet>`; `PmParser::parse_str(&mut self, source: &str) -> Result<ParsedSheet>` (was `Result<Sheet>`)

- [ ] **Step 1: Add the `indexmap` dependency**

In `pm-lang/Cargo.toml`, add to `[dependencies]`:

```toml
indexmap = "2"
```

- [ ] **Step 2: Write failing tests**

Add to the `#[cfg(test)] mod tests` block at the bottom of `pm-lang/src/parser.rs` (after `parse_duplicate_cell_is_error`):

```rust
    #[test]
    fn parse_str_returns_cell_names_in_declaration_order() {
        let parsed = parser()
            .parse_str("sheet s { cell z: i32 = 1; cell a: i32 = 2; cell m: i32 = 3; }")
            .unwrap();
        let names: Vec<&str> = parsed.cell_names.keys().map(String::as_str).collect();
        assert_eq!(names, vec!["z", "a", "m"]);
    }

    #[test]
    fn parsed_sheet_derefs_to_sheet_for_propagate() {
        let mut parsed = parser().parse_str("sheet s { cell x: i32 = 1; }").unwrap();
        // Deref/DerefMut must make Sheet's methods directly callable.
        parsed.propagate().unwrap();
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p pm-lang parse_str_returns_cell_names_in_declaration_order`
Expected: compile error — `no field \`cell_names\` on type \`Sheet\`` (parse_str still returns a bare `Sheet`)

- [ ] **Step 4: Change `cell_names` to an ordered map**

In `pm-lang/src/parser.rs`, replace the `use std::collections::HashMap;` import with:

```rust
use indexmap::IndexMap;
```

Change the `ParseContext` field:

```rust
struct ParseContext {
    /// Token stream; `None` while temporarily owned by CELParser.
    tokens: Option<Peekable<LexLexer>>,
    sheet: Sheet,
    /// Maps cell name → (CellId, TypeId), in declaration order, for method and
    /// conditional compilation and for exposing to callers via `ParsedSheet`.
    cell_names: IndexMap<String, (CellId, TypeId)>,
}
```

- [ ] **Step 5: Add `ParsedSheet` and change `parse_str`'s return type**

Add above the `ParseContext` struct in `pm-lang/src/parser.rs`:

```rust
/// The result of [`PmParser::parse_str`]: a live [`Sheet`] plus the declared
/// cell names, in source declaration order.
///
/// Derefs to [`Sheet`] so callers that only need sheet methods (e.g.
/// `propagate`) can use the result exactly as if it were a `Sheet`.
pub struct ParsedSheet {
    /// The constructed sheet.
    pub sheet: Sheet,
    /// Cell name → `(CellId, TypeId)`, in declaration order.
    pub cell_names: IndexMap<String, (CellId, TypeId)>,
}

impl std::ops::Deref for ParsedSheet {
    type Target = Sheet;

    fn deref(&self) -> &Sheet {
        &self.sheet
    }
}

impl std::ops::DerefMut for ParsedSheet {
    fn deref_mut(&mut self) -> &mut Sheet {
        &mut self.sheet
    }
}
```

Update `parse_str` (in `impl PmParser`):

```rust
    pub fn parse_str(&mut self, source: &str) -> Result<ParsedSheet> {
        let stream =
            TokenStream::from_str(source).map_err(|e| ParseError::new(e.to_string(), e.span()))?;
        let mut ctx = ParseContext {
            tokens: Some(LexLexer::new(stream.into_iter()).peekable()),
            sheet: Sheet::new(),
            cell_names: IndexMap::new(),
        };
        self.parse_sheet(&mut ctx)?;
        if let Some(tok) = ctx.peek_token() {
            return Err(ParseError::new("unexpected token", tok.span()));
        }
        Ok(ParsedSheet {
            sheet: ctx.sheet,
            cell_names: ctx.cell_names,
        })
    }
```

Also update the doc comment's `# Errors` section is unaffected; no other changes needed in this function.

- [ ] **Step 6: Re-export `ParsedSheet`**

In `pm-lang/src/lib.rs`, change:

```rust
pub use parser::PmParser;
```

to:

```rust
pub use parser::{ParsedSheet, PmParser};
```

- [ ] **Step 7: Run all pm-lang tests and verify they pass**

Run: `cargo test -p pm-lang`
Expected: all tests pass, including the two new ones

- [ ] **Step 8: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy --workspace --exclude begin -- -D warnings
git add pm-lang/Cargo.toml pm-lang/src/parser.rs pm-lang/src/lib.rs
git commit -m "feat(pm-lang): return ParsedSheet with ordered cell names from parse_str"
```

---

### Task 2: begin — `labels_from_cell_names` bridge

**Files:**
- Modify: `begin/src/bridge.rs`

**Interfaces:**
- Consumes: `indexmap::IndexMap<String, (CellId, TypeId)>` (shape matches `pm_lang::ParsedSheet::cell_names`, but this function takes the map directly and has no `pm-lang` dependency)
- Produces: `pub fn labels_from_cell_names(cell_names: &IndexMap<String, (CellId, TypeId)>) -> Labels`

- [ ] **Step 1: Write failing tests**

Add to the `#[cfg(test)] mod tests` block in `begin/src/bridge.rs` (after the existing `use` lines inside that module):

```rust
    #[test]
    fn labels_from_cell_names_builds_entries_for_supported_types() {
        use std::any::TypeId;

        let mut sheet = Sheet::new();
        let a = sheet.add_cell(2.0_f64);
        let b = sheet.add_cell(3_i32);
        let c = sheet.add_cell(true);
        let d = sheet.add_cell("hi".to_string());

        let mut cell_names = IndexMap::new();
        cell_names.insert("a".to_string(), (a, TypeId::of::<f64>()));
        cell_names.insert("b".to_string(), (b, TypeId::of::<i32>()));
        cell_names.insert("c".to_string(), (c, TypeId::of::<bool>()));
        cell_names.insert("d".to_string(), (d, TypeId::of::<String>()));

        let labels = labels_from_cell_names(&cell_names);

        assert_eq!(labels.cells.len(), 4);
        assert_eq!((labels.cells[&a].display)(&sheet), "2");
        assert_eq!((labels.cells[&b].display)(&sheet), "3");
        assert_eq!((labels.cells[&c].display)(&sheet), "true");
        assert_eq!((labels.cells[&d].display)(&sheet), "hi");
    }

    #[test]
    fn labels_from_cell_names_preserves_declaration_order() {
        use std::any::TypeId;

        let mut sheet = Sheet::new();
        let z = sheet.add_cell(1_i32);
        let a = sheet.add_cell(2_i32);

        let mut cell_names = IndexMap::new();
        cell_names.insert("z".to_string(), (z, TypeId::of::<i32>()));
        cell_names.insert("a".to_string(), (a, TypeId::of::<i32>()));

        let labels = labels_from_cell_names(&cell_names);
        let ids: Vec<_> = labels.cells.keys().copied().collect();
        assert_eq!(ids, vec![z, a]);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p begin labels_from_cell_names`
Expected: compile error — `cannot find function \`labels_from_cell_names\``

- [ ] **Step 3: Implement `labels_from_cell_names`**

Add near the top of `begin/src/bridge.rs`, after the `Labels` impl block and before `NodeKind`:

```rust
/// Builds a [`Labels`] from a pm-lang-style declaration-ordered cell name map.
///
/// Matches each `TypeId` against the built-in primitive types
/// `pm_lang::TypeRegistry::new()` registers. Cells whose `TypeId` is not one
/// of these are silently skipped — they simply won't appear in the sidebar.
///
/// - Complexity: O(n) in the number of cells.
pub fn labels_from_cell_names(cell_names: &IndexMap<String, (CellId, TypeId)>) -> Labels {
    let mut labels = Labels::new();
    for (name, &(id, type_id)) in cell_names {
        macro_rules! try_ty {
            ($T:ty) => {
                if type_id == TypeId::of::<$T>() {
                    labels.add_cell::<$T>(id, name);
                    continue;
                }
            };
        }
        try_ty!(i8);
        try_ty!(i16);
        try_ty!(i32);
        try_ty!(i64);
        try_ty!(i128);
        try_ty!(isize);
        try_ty!(u8);
        try_ty!(u16);
        try_ty!(u32);
        try_ty!(u64);
        try_ty!(u128);
        try_ty!(usize);
        try_ty!(f32);
        try_ty!(f64);
        try_ty!(bool);
        try_ty!(String);
    }
    labels
}
```

Add `use std::any::TypeId;` to the top-level imports of `begin/src/bridge.rs` (alongside the existing `use indexmap::IndexMap;` and `use property_model::{CellId, ConditionalId, Error, RelationshipId, Sheet};`).

- [ ] **Step 4: Run tests and verify they pass**

Run: `cargo test -p begin labels_from_cell_names`
Expected: both tests pass

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy -p begin --no-default-features -- -D warnings
git add begin/src/bridge.rs
git commit -m "feat(begin): add labels_from_cell_names bridge for pm-lang-parsed sheets"
```

---

### Task 3: begin — `format_property_model_error`

**Files:**
- Modify: `begin/Cargo.toml`
- Modify: `begin/src/bridge.rs`

**Interfaces:**
- Consumes: `property_model::Error` (aliased `Error` in `bridge.rs`); `cel_parser::FormatRustcStyle`; `annotate_snippets::Renderer`
- Produces: `pub fn format_property_model_error(e: &Error, source: &str) -> String`

- [ ] **Step 1: Add dependencies**

In `begin/Cargo.toml`, add to `[dependencies]`:

```toml
cel-parser = { path = "../cel-parser" }
annotate-snippets = "0.12"
```

- [ ] **Step 2: Write failing tests**

Add to the `#[cfg(test)] mod tests` block in `begin/src/bridge.rs`:

```rust
    #[test]
    fn format_property_model_error_invalid_id_falls_back_to_display() {
        let msg = format_property_model_error(&Error::InvalidId, "source text");
        assert_eq!(msg, "invalid cell or relationship id");
    }

    #[test]
    fn format_property_model_error_method_failed_renders_caret_diagnostic() {
        use cel_parser::{SourceSpan, SpanContext};

        let source = "1i32 / 0i32";
        let span = SourceSpan::new(1, 0, 1, 11);
        let inner = anyhow::anyhow!("division by zero").context(SpanContext::new(span));
        let err = Error::MethodFailed(inner);

        let msg = format_property_model_error(&err, source);

        assert!(msg.contains("division by zero"), "{msg}");
        assert!(msg.contains(source), "{msg}");
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p begin format_property_model_error`
Expected: compile error — `cannot find function \`format_property_model_error\``

- [ ] **Step 4: Implement `format_property_model_error`**

Add to `begin/src/bridge.rs`, near `labels_from_cell_names`:

```rust
use annotate_snippets::Renderer;
use cel_parser::FormatRustcStyle;

/// Formats a [`Error`] as a rustc-style diagnostic when possible.
///
/// `Error::MethodFailed` wraps an `anyhow::Error` raised by a compiled method
/// body; when that error carries a `SpanContext` (attached automatically by
/// cel-parser's `span-diagnostics` feature for built-in arithmetic ops) this
/// renders a full caret diagnostic against `source`. All other variants have
/// no source span and fall back to their `Display` message.
pub fn format_property_model_error(e: &Error, source: &str) -> String {
    match e {
        Error::MethodFailed(inner) => {
            inner.format_rustc_style(source, "<pm-lang source>", 1, &Renderer::plain())
        }
        other => other.to_string(),
    }
}
```

Add the two new `use` lines (`annotate_snippets::Renderer`, `cel_parser::FormatRustcStyle`) to the top-level imports of `begin/src/bridge.rs`.

- [ ] **Step 5: Run tests and verify they pass**

Run: `cargo test -p begin format_property_model_error`
Expected: both tests pass

- [ ] **Step 6: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy -p begin --no-default-features -- -D warnings
git add begin/Cargo.toml begin/src/bridge.rs
git commit -m "feat(begin): add format_property_model_error for runtime diagnostics"
```

---

### Task 4: begin — `build_sheet`/`BuildOutcome` and the demo pm-lang source

**Files:**
- Modify: `begin/Cargo.toml`
- Create: `begin/src/source_panel.rs`
- Modify: `begin/src/main.rs`

**Interfaces:**
- Consumes: `pm_lang::{PmParser, ParsedSheet, TypeRegistry}`; `cel_parser::OpLookup`; `crate::bridge::{labels_from_cell_names, format_property_model_error}`
- Produces: `pub struct BuildOutcome { pub sheet_labels: Option<(Sheet, Labels)>, pub error: Option<String> }`; `pub fn build_sheet(source: &str) -> BuildOutcome`

- [ ] **Step 1: Add the `pm-lang` dependency**

In `begin/Cargo.toml`, add to `[dependencies]`:

```toml
pm-lang = { path = "../pm-lang" }
```

- [ ] **Step 2: Register the new module**

In `begin/src/main.rs`, add `mod source_panel;` alongside the existing `mod` declarations:

```rust
//! Entry point for the `begin` property model development environment.
mod app;
mod bridge;
mod graph_view;
mod inspector;
mod source_panel;
mod spectrum;

fn main() {
    dioxus::launch(app::App);
}
```

- [ ] **Step 3: Write failing tests**

Create `begin/src/source_panel.rs` with just the function signatures' test module first:

```rust
//! [`SourcePanel`] — collapsible bottom panel for editing and applying pm-lang source.

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
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test -p begin --lib source_panel::`
Expected: compile error — `cannot find function \`build_sheet\``

- [ ] **Step 5: Implement `build_sheet` and `BuildOutcome`**

Add above the `#[cfg(test)]` block in `begin/src/source_panel.rs`:

```rust
use annotate_snippets::Renderer;
use pm_lang::{PmParser, TypeRegistry};
use property_model::Sheet;

use crate::bridge::{Labels, format_property_model_error, labels_from_cell_names};

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
            let msg = e.format_rustc_style(source, "<pm-lang source>", 1, &Renderer::plain());
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
```

- [ ] **Step 6: Run tests and verify they pass**

Run: `cargo test -p begin --lib source_panel::`
Expected: all three tests pass

- [ ] **Step 7: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy -p begin --no-default-features -- -D warnings
git add begin/Cargo.toml begin/src/main.rs begin/src/source_panel.rs
git commit -m "feat(begin): add build_sheet for parsing pm-lang source into a live sheet"
```

---

### Task 5: begin — `SourcePanel` UI component

**Files:**
- Modify: `begin/src/source_panel.rs`

**Interfaces:**
- Consumes: `build_sheet` (Task 4)
- Produces: `#[component] pub fn SourcePanel(editor_source: Signal<String>, applied_source: Signal<String>, sheet: Signal<Sheet>, labels: Signal<Labels>, error: Signal<Option<String>>, open: Signal<bool>) -> Element`

This task has no automated test (no headless Dioxus render harness exists in
this codebase); it is verified by compilation here and by manual QA in Task 8.

- [ ] **Step 1: Add the `SourcePanel` component**

Add to `begin/src/source_panel.rs`, above the `#[cfg(test)]` block, after `build_sheet`:

```rust
use dioxus::prelude::*;

/// Collapsible bottom panel: a pm-lang source textarea, an Apply button, and
/// a rustc-style diagnostic for the most recent parse or runtime failure.
///
/// Clicking Apply parses `editor_source`, builds a new sheet and labels via
/// [`build_sheet`], and — on success or on a runtime (propagate) failure —
/// replaces `sheet`/`labels` and updates `applied_source` to match. On a
/// parse failure, `sheet`/`labels` are left unchanged.
#[component]
pub fn SourcePanel(
    editor_source: Signal<String>,
    applied_source: Signal<String>,
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    error: Signal<Option<String>>,
    open: Signal<bool>,
) -> Element {
    let mut editor_source = editor_source;
    let mut applied_source = applied_source;
    let mut sheet = sheet;
    let mut labels = labels;
    let mut error = error;
    let mut open = open;

    rsx! {
        div {
            style: "border-top: 1px solid #ccc; display: flex; flex-direction: column; flex-shrink: 0;",
            div {
                style: "display: flex; align-items: center; gap: 8px; padding: 4px 8px;",
                button {
                    onclick: move |_| open.set(!*open.read()),
                    if *open.read() { "▼ Source" } else { "▶ Source" }
                }
                if *open.read() {
                    button {
                        onclick: move |_| {
                            let source = editor_source.read().clone();
                            applied_source.set(source.clone());
                            let outcome = build_sheet(&source);
                            if let Some((new_sheet, new_labels)) = outcome.sheet_labels {
                                sheet.set(new_sheet);
                                labels.set(new_labels);
                            }
                            error.set(outcome.error);
                        },
                        "Apply"
                    }
                }
            }
            if *open.read() {
                textarea {
                    style: "width: 100%; height: 160px; font-family: monospace; box-sizing: border-box; margin: 0; border: none; border-top: 1px solid #ccc;",
                    value: "{editor_source}",
                    oninput: move |evt: FormEvent| editor_source.set(evt.value()),
                }
                if let Some(msg) = error.read().as_ref() {
                    pre {
                        style: "margin: 0; padding: 8px; background: #fee; color: #900; overflow: auto; max-height: 200px; white-space: pre-wrap; font-family: monospace;",
                        "{msg}"
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p begin --no-default-features`
Expected: success (no errors; `dioxus` desktop feature is not required to type-check the component)

- [ ] **Step 3: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy -p begin --no-default-features -- -D warnings
git add begin/src/source_panel.rs
git commit -m "feat(begin): add SourcePanel component for editing pm-lang source"
```

---

### Task 6: begin — wire `SourcePanel` into `App`, replace the hard-coded demo

**Files:**
- Modify: `begin/src/app.rs`

**Interfaces:**
- Consumes: `crate::source_panel::{build_sheet, SourcePanel}`
- Produces: `pub const DEMO_SOURCE: &str`; `App` renders `SourcePanel` below the existing graph+inspector row

- [ ] **Step 1: Replace `app.rs`'s content**

Replace the full contents of `begin/src/app.rs` with:

```rust
//! Root [`App`] component and demo pm-lang source.

use dioxus::prelude::*;

use crate::bridge::to_graph_data;
use crate::graph_view::GraphView;
use crate::inspector::Inspector;
use crate::source_panel::{SourcePanel, build_sheet};
use crate::spectrum::SpTheme;

/// Default pm-lang source: two independent bidirectional constraint systems
/// (`a × b = c` and `d × e = f`) linked by a conditional on `p`.
///
/// - `p = 0`: the relationship `c = f` (bidirectional) becomes active.
/// - `p = 1`: the relationship `c = f × 2` (bidirectional) becomes active.
/// - Any other `p`: the two systems are independent.
pub const DEMO_SOURCE: &str = r#"sheet demo {
    cell a: f64 = 2.0;
    cell b: f64 = 3.0;
    cell c: f64;
    cell d: f64 = 4.0;
    cell e: f64 = 5.0;
    cell f: f64;
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
}
"#;

/// Root component: Spectrum theme wrapper, graph+inspector row on top and a
/// collapsible pm-lang source panel docked at the bottom.
#[component]
pub fn App() -> Element {
    let editor_source = use_signal(|| DEMO_SOURCE.to_string());
    let applied_source = use_signal(|| DEMO_SOURCE.to_string());
    let error = use_signal(|| None::<String>);
    let source_panel_open = use_signal(|| true);

    let initial = build_sheet(DEMO_SOURCE);
    let (initial_sheet, initial_labels) = initial
        .sheet_labels
        .expect("DEMO_SOURCE must parse and build a sheet without error");
    let sheet = use_signal(|| initial_sheet);
    let labels = use_signal(|| initial_labels);

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
                style: "position: fixed; inset: 0; display: flex; flex-direction: column; overflow: hidden;",
                div {
                    style: "flex: 1; display: flex; overflow: hidden; min-height: 0;",
                    GraphView { data: graph_data }
                    Inspector { sheet, labels }
                }
                SourcePanel {
                    editor_source,
                    applied_source,
                    sheet,
                    labels,
                    error,
                    open: source_panel_open,
                }
            }
        }
    }
}
```

Note: `Inspector { sheet, labels }` keeps its current (pre-Task-7) signature
here; Task 7 updates both `Inspector`'s definition and this call site together.

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p begin --no-default-features`
Expected: success

- [ ] **Step 3: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy -p begin --no-default-features -- -D warnings
git add begin/src/app.rs
git commit -m "feat(begin): replace hard-coded demo sheet with pm-lang source + SourcePanel"
```

---

### Task 7: begin — thread error reporting through the `Inspector`

**Files:**
- Modify: `begin/src/inspector.rs`
- Modify: `begin/src/app.rs`

**Interfaces:**
- Consumes: `crate::bridge::format_property_model_error` (Task 3)
- Produces: `Inspector(sheet: Signal<Sheet>, labels: Signal<Labels>, error: Signal<Option<String>>, applied_source: Signal<String>) -> Element` (was `Inspector(sheet, labels)`)

- [ ] **Step 1: Update `Inspector` and `CellRow` signatures**

In `begin/src/inspector.rs`, change the `Inspector` component:

```rust
#[component]
pub fn Inspector(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    error: Signal<Option<String>>,
    applied_source: Signal<String>,
) -> Element {
    let ids: Vec<CellId> = labels.read().cells.keys().copied().collect();

    rsx! {
        div {
            style: "width: 260px; min-width: 260px; height: 100%; overflow-y: auto; padding: 12px; box-sizing: border-box;",
            SpHeading { "Cells" }
            SpDivider {}
            for id in ids {
                CellRow { key: "{id:?}", id, sheet, labels, error, applied_source }
            }
        }
    }
}
```

And the `CellRow` component signature:

```rust
#[component]
fn CellRow(
    id: CellId,
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    error: Signal<Option<String>>,
    applied_source: Signal<String>,
) -> Element {
```

(the rest of `CellRow`'s body up to the `oninput` handler is unchanged)

- [ ] **Step 2: Format runtime errors in the write handler**

Replace the `oninput` handler's body inside `CellRow` (the closure passed to
`spawn(async move { ... })`) with:

```rust
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
                                error.set(None);
                            }
                            Err(e) => {
                                has_error.set(true);
                                let source = applied_source.read().clone();
                                error.set(Some(crate::bridge::format_property_model_error(&e, &source)));
                            }
                        }
                    });
                },
```

This changes the prior nested `if (meta.write_str)(...).is_ok() { ... } else { has_error.set(true) }`
into a single `match` covering both the write and the propagate step, so both
failure points produce a formatted diagnostic in `error`.

- [ ] **Step 3: Update the `App` call site**

In `begin/src/app.rs`, change:

```rust
                    Inspector { sheet, labels }
```

to:

```rust
                    Inspector { sheet, labels, error, applied_source }
```

- [ ] **Step 4: Verify it compiles and existing bridge tests still pass**

Run: `cargo check -p begin --no-default-features`
Expected: success

Run: `cargo test -p begin`
Expected: all tests pass (this task adds no new automated tests — Dioxus event
handlers aren't exercised by `cargo test` in this codebase; behavior is
verified manually in Task 8)

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy -p begin --no-default-features -- -D warnings
git add begin/src/inspector.rs begin/src/app.rs
git commit -m "feat(begin): surface formatted runtime errors from cell edits"
```

---

### Task 8: Full verification pass and manual QA

**Files:** none (verification only)

- [ ] **Step 1: Run the full workspace check suite**

```bash
cargo fmt --all -- --check
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin -- -D warnings
cargo clippy -p begin --no-default-features -- -D warnings
```

Expected: all commands succeed with no warnings or failures.

- [ ] **Step 2: Manual QA via the desktop app**

Run the `begin: serve (desktop)` VS Code task, or from the repo root:

```bash
cd begin && dx serve --platform desktop
```

Verify, in the running app:

1. **Baseline:** the graph and sidebar show the same demo cells (`a, b, c, d,
   e, f, p`) and values as before this change; the Source panel is open by
   default and shows the `DEMO_SOURCE` text.
2. **Valid edit:** change `cell a: f64 = 2.0;` to `cell a: f64 = 10.0;` in the
   Source panel, click Apply. The sidebar repopulates, `c` recomputes to
   reflect the new `a`, and the error panel is empty.
3. **Syntax error:** delete a `;` after a `cell` declaration, click Apply. The
   error panel shows a caret-annotated diagnostic pointing at the offending
   location; the graph/sidebar remain on the last successfully-applied sheet.
4. **Runtime error:** replace a method body with one that divides by a cell
   currently holding `0` (e.g. change a division's denominator to a cell
   initialized to `0.0`), click Apply. The error panel shows a caret-annotated
   diagnostic pointing at the division expression in the source; the sidebar
   still updates to the new (partially-derived) sheet.
5. **Inspector edit error:** with a sheet applied, edit a cell's value in the
   sidebar text field to a value that drives a downstream division by zero.
   The field shows its invalid state and the error panel shows the same kind
   of formatted diagnostic as step 4.

- [ ] **Step 3: Report results**

No commit for this task (verification only). If any manual QA step fails,
return to the relevant earlier task, fix, and re-run this task's checklist
from Step 1.
