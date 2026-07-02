# begin: pm-lang Integration & Error Reporting Design

**Date:** 2026-07-02
**Author:** Sean Parent
**Status:** Draft

## Overview

`begin`'s property model is currently hard-coded in Rust (`app.rs::make_demo_sheet`),
built by calling `property_model::Sheet` construction APIs directly. `pm-lang` — a
complete DSL parser crate — already exists in the workspace but `begin` does not
depend on it.

This design:

1. Adds a live-editable pm-lang source panel to `begin`, replacing the hard-coded
   demo sheet with one parsed from pm-lang source text.
2. Surfaces formatted (rustc-style, caret-annotated) parser and runtime error
   diagnostics in the UI, reusing existing formatting machinery in `cel-parser`
   (`ParseError::format_rustc_style`, `FormatRustcStyle for anyhow::Error`) rather
   than inventing a new error-reporting mechanism.

## Placement

- `begin/Cargo.toml` gains a dependency on `pm-lang` (and transitively needs
  `cel-parser`'s `OpLookup`/`Renderer` re-exports, and `annotate_snippets::Renderer`
  directly for `Renderer::plain()`).
- `pm-lang/Cargo.toml` gains a dependency on `indexmap` (already a `begin`
  dependency, used for the same ordering reason).

## Part 1 — `pm-lang`: expose cell names to callers

**Problem:** `PmParser::parse_str` returns `Result<Sheet>`, discarding the
name → `(CellId, TypeId)` map it builds internally (`ParseContext::cell_names`).
`begin` needs this to build UI labels and needs cells in declaration order for a
stable sidebar; a plain `HashMap` does not preserve order.

**Change** (`pm-lang/src/parser.rs`, `pm-lang/src/lib.rs`):

```rust
/// The result of parsing a pm-lang source string: a live `Sheet` plus the
/// declared cell names, in declaration order.
pub struct ParsedSheet {
    pub sheet: Sheet,
    pub cell_names: IndexMap<String, (CellId, TypeId)>,
}

impl std::ops::Deref for ParsedSheet {
    type Target = Sheet;
    fn deref(&self) -> &Sheet { &self.sheet }
}
impl std::ops::DerefMut for ParsedSheet {
    fn deref_mut(&mut self) -> &mut Sheet { &mut self.sheet }
}

impl PmParser {
    pub fn parse_str(&mut self, source: &str) -> Result<ParsedSheet>; // was Result<Sheet>
}
```

- `ParseContext::cell_names` changes from `HashMap<String, (CellId, TypeId)>` to
  `IndexMap<String, (CellId, TypeId)>` — insertion order is preserved, so
  `cell_names` iterates in source declaration order.
- `Deref`/`DerefMut` to `Sheet` keep every existing call site that does
  `parser.parse_str(...).unwrap().propagate()` compiling unchanged; only call
  sites that pattern-match the `Ok` value as a bare `Sheet` need updating (none
  currently do — tests bind `let _sheet = ...` or call methods through it).
- Existing pm-lang unit/doc tests are updated only where they need to be (none
  currently inspect cell names), per the file's contract-style doc comments.

This is a breaking change to `PmParser::parse_str`'s return type. Acceptable per
project status: pm-lang has no clients yet outside this workspace.

## Part 2 — `begin`: Labels bridge from parsed cell names

`bridge::Labels::add_cell::<T>()` is generic and needs a concrete `T: Display +
FromStr` at the call site — but pm-lang produces type-erased `TypeId`s at
runtime. Rather than teaching `pm-lang`/`property-model` about `Display`/
`FromStr` (a UI-only concern), `begin` gets a small dispatch table matching each
`TypeId` registered by `TypeRegistry::new()` to a monomorphized `add_cell::<T>`
call:

```rust
// begin/src/bridge.rs
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
        try_ty!(i8); try_ty!(i16); try_ty!(i32); try_ty!(i64); try_ty!(i128); try_ty!(isize);
        try_ty!(u8); try_ty!(u16); try_ty!(u32); try_ty!(u64); try_ty!(u128); try_ty!(usize);
        try_ty!(f32); try_ty!(f64); try_ty!(bool); try_ty!(String);
        // Unregistered/custom types are silently skipped — they simply won't
        // appear in the Inspector sidebar.
    }
    labels
}
```

This covers all 16 built-ins `TypeRegistry::new()` pre-registers. If `begin`
later registers custom types with a `TypeRegistry`, it extends this table.

## Part 3 — UI: source panel, Apply flow, error panel

### State (`App`)

- `editor_source: Signal<String>` — live textarea content, seeded with the demo
  pm-lang source (translated from the current `make_demo_sheet`, see Appendix).
- `applied_source: Signal<String>` — the source text behind the *currently live*
  `sheet`/`labels`; used only to render error snippets against a stable text,
  independent of further un-applied edits.
- `sheet: Signal<Sheet>`, `labels: Signal<Labels>` — as today.
- `error: Signal<Option<String>>` — last formatted diagnostic (parse or
  runtime), shown in the error panel; `None` hides it.
- `source_panel_open: Signal<bool>` — collapsed/expanded state of the bottom
  panel.

### Components

- **`SourcePanel`** (new, `begin/src/source_panel.rs`): bottom collapsible
  panel. Contains a toggle control, a `<textarea>` bound to `editor_source`, an
  **Apply** button, and the error region (a `<pre>` block rendering `error`
  when `Some`).
- **`App`** layout becomes three stacked regions: the existing graph+inspector
  row, plus `SourcePanel` docked at the bottom, collapsible.

### Apply flow

On Apply button click:

1. `applied_source.set(editor_source.read().clone())`.
2. `PmParser::new(TypeRegistry::new(), OpLookup::new()).parse_str(&applied_source)`:
   - `Err(e)` → `error.set(Some(e.format_rustc_style(&applied_source, "<pm-lang source>", 1, &Renderer::plain())))`. Existing `sheet`/`labels` are left untouched.
   - `Ok(mut parsed)`:
     - `let labels = labels_from_cell_names(&parsed.cell_names);`
     - `match parsed.propagate() { ... }`
       - `Err(e)` → `error.set(Some(format_property_model_error(&e, &applied_source)))`; still swap in the new `sheet`/`labels` (parse succeeded — the sheet is simply left with un-derived/partial values, matching the tolerance the Inspector already has for write-time propagate failures).
       - `Ok(())` → `parsed.clear_changed(); error.set(None);` then swap in `sheet`/`labels`.

### Inspector integration

`CellRow`'s existing write handler (`begin/src/inspector.rs`) currently sets a
bare `has_error: Signal<bool>` on write/propagate failure with no message. It
is extended to also format the `property_model::Error` and write it into the
same `App`-level `error` signal, threaded down as an additional prop —
`Inspector(sheet, labels, error)` → `CellRow(id, sheet, labels, error)` —
matching the existing convention of passing `sheet`/`labels` as signal props
rather than using Dioxus context. A failed edit (e.g. typing a value that
causes a division by zero downstream) then shows the same rustc-style
diagnostic as a bad Apply. The per-field `invalid` visual state on
`SpTextfield` is unchanged, and needs `applied_source` threaded down alongside
`error` so `format_property_model_error` has source text to render against.

## Part 4 — Error formatting

Two error sources, both rendered as plain-text rustc-style diagnostics
(`Renderer::plain()`, since output goes into an HTML `<pre>`, not a terminal):

- **Parse errors** (`pm_lang::ParseError`, re-exported from `cel_parser`):
  `.format_rustc_style(source, "<pm-lang source>", 1, &Renderer::plain())` —
  used as-is, no new code needed.
- **Runtime errors** (`property_model::Error`, returned by `Sheet::write` /
  `Sheet::propagate`): this type is not `anyhow::Error`, so the existing
  `FormatRustcStyle for anyhow::Error` impl doesn't apply directly. Add:

  ```rust
  // begin/src/bridge.rs (or begin/src/errors.rs)
  use cel_parser::FormatRustcStyle;
  use annotate_snippets::Renderer;

  pub fn format_property_model_error(e: &property_model::Error, source: &str) -> String {
      match e {
          property_model::Error::MethodFailed(inner) =>
              inner.format_rustc_style(source, "<pm-lang source>", 1, &Renderer::plain()),
          other => other.to_string(),
      }
  }
  ```

  `MethodFailed` wraps an `anyhow::Error` produced by a CEL-compiled method
  body; because `cel-parser`'s `span-diagnostics` feature is on by default,
  arithmetic errors (division by zero, overflow) raised inside method bodies
  already carry a `SpanContext` pointing back into the pm-lang source, so this
  renders a full caret diagnostic automatically. Structural errors
  (`Conflict`, `Cycle`, `InvalidId`, `TypeMismatch`) carry no span and fall back
  to `Display` — a plain one-line message, still an improvement over today's
  silent boolean flag.

`begin` needs no new dependency for this: `cel_parser::FormatRustcStyle` and
`annotate_snippets::Renderer` come in transitively through `pm-lang`.

## Testing Plan

- `pm-lang`: unit tests for `ParsedSheet::cell_names` ordering (declaration
  order preserved) and that `Deref`/`DerefMut` make existing `Sheet` method
  calls work unchanged through the wrapper.
- `begin`: unit test(s) for `labels_from_cell_names` covering at least one
  integer type, `f64`, `bool`, and `String`.
- Manual verification via `dx serve --platform desktop` (the `run` skill /
  `begin: serve (desktop)` VS Code task):
  - Valid source → Apply succeeds, sidebar repopulates with new cells, graph
    updates.
  - Syntax error (e.g. missing `;`) → formatted diagnostic with caret shown in
    the error panel; existing sheet/labels remain live.
  - A method body that divides by zero, triggered either via Apply or via
    editing a cell in the Inspector → runtime diagnostic shown with a caret
    pointing at the offending sub-expression in the applied source.

## Appendix — demo source translation

The current `make_demo_sheet` (two independent `x × y = z` bidirectional
systems linked by a conditional on `p`) translates to pm-lang source
byte-for-byte equivalently (cell strength starts at 0 regardless of add-order;
first-listed method in a relationship wins strength ties — both preserved by a
direct translation, so cells can be declared in natural reading order):

```text
sheet demo {
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
```

This is used as the initial `editor_source` value in `App`.
