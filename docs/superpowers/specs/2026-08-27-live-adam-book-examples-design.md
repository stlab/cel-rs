# Live Adam examples in adam-lang-book

## Context

`adam-lang-book` renders every code example in `book-src/*.md` from a real, compiled
`#[test]` function in `tests/*.rs`: the sheet source lives as an inline Rust string literal
inside the test body, the test asserts on its behavior via `adam_rs`'s Rust API
(`propagate`, `read`, `is_source`, `parse_str(..).unwrap_err()`, ...), and the chapter pulls
a slice of that function into the page via `{{#include tests/chapter.rs:anchor}}`. This
guarantees the prose never drifts from working code, but it also means every example the
reader sees is Rust-shaped — sheet source wrapped in `parser().parse_str(r#"..."#)`,
assertions written against `adam_rs::Sheet` — even though the book's subject is the Adam
language, not its Rust embedding API.

Separately, `begin` (the Dioxus desktop/web app) already has a live, editable view of a
running sheet: `Inspector` renders each cell as a Spectrum Web Component field bound to an
`adam_rs::Sheet`, with write-and-propagate wiring and forced/invalid/warning styling.
`begin` already builds for the web via `dioxus/web` + `wasm-bindgen` (see
`.claude/skills/verifying-begin-ui/SKILL.md`), which is what makes embedding this kind of
live view directly in the book's HTML output technically possible.

This spec covers making every example in the book live and interactive the same way, while
first converting the example corpus to plain `.adm2` files so the book's markdown shows only
Adam source, never Rust. The constraint-graph view (`begin/src/graph_view.rs`) is explicitly
out of scope: the book gets a live cell inspector only, not a live graph, for this pass.

## Decisions

- **Examples become standalone files, not Rust string literals.** Every one of the book's 27
  examples moves to `adam-lang-book/book-src/examples/<chapter>/<name>.adm2`, containing
  exactly the Adam source shown in the book and nothing else — no Rust wrapper, no
  assertions. `{{#include}}` in the markdown points at the `.adm2` file directly.
- **Tests load and execute those files instead of embedding source.** Each existing
  `#[test]` in `tests/*.rs` keeps its existing assertions but loads its sheet source via
  `include_str!("../book-src/examples/<chapter>/<name>.adm2")` instead of an inline string
  literal. No new test DSL: the assertions already in place (`propagate`, `read`,
  `is_source`, `parse_str(..).unwrap_err()`, `matches!(err, ...)`) are ordinary Rust and stay
  that way — only the *source* moves out of the Rust file, not the test logic.
- **A new reusable UI crate: `adam-web-ui`.** Named for the web/WASM rendering target
  specifically (not `adam-ui`), leaving room for a future native-platform binding crate
  without a naming collision. Added as a workspace member alongside `adam-rs`/`adam-lang`
  per this repo's Library-First Design principle: it is a second, independent consumer of
  `adam-rs` (the book), not a `begin`-only helper, so it belongs at that layer rather than
  inside `begin`.
- **Graph view stays `begin`-only.** `graph_view.rs`, its D3/`graph.js` assets, and the
  zoom/show-inactive toolbar are not touched or extracted in this pass.
- **New mount points are generated, not hand-authored.** A small mdBook preprocessor injects
  a live-mount `<div>` immediately after every `{{#include .../*.adm2}}`, so the pairing
  between a shown example and its live widget can never drift as examples move or new ones
  are added — matching the book's existing "nothing here can silently drift" property.
- **CI builds and publishes the live bundle now**, not as a deferred follow-up: both the
  PR-check workflow and the Pages deploy workflow gain a wasm build step ahead of
  `mdbook build`.

## Components

### `adam-web-ui` (new crate)

Extracted from `begin/src/`:

- **`spectrum.rs`** moves verbatim. It's already `Sheet`-agnostic — pure Dioxus wrappers
  around Spectrum Web Components (`SpTextfield`, `SpNumberfield`, `SpCheckbox`, `SpSlider`,
  `SpTheme`, etc.) — so nothing about it needs to change to be reusable.
- **`bridge.rs`**'s `Labels`, `format_adam_error`, `format_rounded`, and the sheet-building
  path (`build_sheet`'s core: parse source, construct a `Sheet`, run the initial
  `propagate`, produce `Labels`) move over. Anything specific to `begin`'s desktop file
  model (path handling, `SourceOrigin`, watcher plumbing) stays in `begin`.
- **`inspector.rs`**'s cell-list rendering becomes a new `SheetInspector` component:
  `SheetInspector(sheet: Signal<Sheet>, labels: Signal<Labels>) -> Element`. It owns exactly
  what today's `Inspector` does *except* the parts that only make sense inside `begin`'s full
  shell: no examples picker, no open-file controls, no graph-selection interplay. The
  existing branching logic — `OutputStatus`, `cell_flags`, `cell_needs_full_propagate` — moves
  with it unchanged; these are already pure functions with their own doc comments, matching
  this repo's rule that framework-coupled branching logic must be extracted and tested on its
  own.
- `begin` depends on `adam-web-ui` and composes `SheetInspector` inside its own `Inspector`
  usage (or `Inspector` becomes a thin wrapper if the desktop-only additions are small enough
  to layer on top — decided during implementation, not this spec, since it doesn't change any
  observable behavior).

Left in `begin` (not extracted): `graph_view.rs`, `open_file.rs`, `example_source.rs`'s
directory-scanning/file-watch logic, `diagnostics.rs` (stderr-based, desktop/dev-oriented),
and `app.rs`'s shell (examples picker, toolbar, layout).

### Book examples (`adam-lang-book/book-src/examples/<chapter>/<name>.adm2`)

One file per existing example, named after its current test function (e.g.
`relationships/shared_cell_example.adm2`, `relationships/conflict_error.adm2`). Content is
exactly today's inline sheet-source string, unindented and de-escaped.

### `tests/*.rs`

Each test's sheet-source string literal is replaced by
`include_str!("../book-src/examples/<chapter>/<name>.adm2")`; assertions are otherwise
unchanged. `support::parser()` continues to back every test as it does today.

### `book-src/*.md`

Every `{{#include tests/chapter.rs:anchor}}` becomes
`{{#include ../book-src/examples/chapter/name.adm2}}`. Any prose that currently references
Rust-level details only relevant to embedding (e.g. `register_no_default`, `TypeRegistry`)
is left as prose pointing at Appendix A.5, as it already does — this spec doesn't change
that boundary, just removes the incidental Rust the `{{#include}}`s were pulling in.

### New wasm mount crate

A thin crate (working name `adam-lang-book-live`, final name/location decided during
implementation — either its own workspace member or a `[[bin]]`/sub-crate under
`adam-lang-book`) exposing one `#[wasm_bindgen]` entry point:

```rust
#[wasm_bindgen]
pub fn mount(element_id: &str, source: &str)
```

`mount` parses `source` via the same path `adam-web-ui`'s bridge code uses, renders
`SheetInspector` bound to the result into the DOM element named `element_id`, and — on a
parse/build failure — renders the formatted diagnostic (`format_adam_error`) in place of the
inspector instead of panicking, since some examples are deliberately invalid Adam
(`conflict_error`, `cycle_error`, `type_mismatch_is_a_parse_error`, ...) and the live widget
must show that failure gracefully rather than crash the page.

Before finalizing the mounting mechanism: spike whether one loaded wasm module can drive
multiple independent Dioxus `VirtualDom` mounts (one per `mount()` call, one per live
example on a page) — this is a supported Dioxus embedding pattern in general, but unused
anywhere in this codebase so far, and it decides the shape of the JS bootstrap below. If it
doesn't hold up, the fallback is a single Dioxus root per page owning a fixed content area
that internally lists that page's examples — worse ergonomically, only used if the spike
fails.

### mdBook preprocessor (new)

A small binary implementing the mdBook `Preprocessor` trait, registered via
`[preprocessor.live-examples]` in `book.toml`. It walks each chapter's raw markdown,
finds every `{{#include ../book-src/examples/.../*.adm2}}` occurrence, and inserts
immediately after it:

```html
<div class="adam-live" data-example="chapter/name"></div>
```

A small bootstrap script (loaded once via `book.toml`'s `[output.html] additional-js`,
alongside the compiled wasm/js bundle) scans the rendered page on load for every
`div.adam-live`, fetches that example's source (embedded into the wasm bundle at build time
via the same `include_str!`/build.rs-manifest pattern `begin` already uses for its own
examples), and calls `mount(id, source)` for each.

## Data flow

1. **Book build:** `mdbook build` runs the `live-examples` preprocessor first (inserting
   mount divs), then renders normally; the CI wasm build step produces the `mount()` bundle
   and copies it alongside the book's other `additional-js`/`additional-css` assets.
2. **Page load:** the bootstrap script finds each `.adam-live` div and mounts an independent
   `SheetInspector` for that example's source.
3. **Reader interaction:** editing a cell in a mounted `SheetInspector` writes and propagates
   against that example's own `Sheet` instance, exactly as `begin`'s `Inspector` does today —
   each mounted widget is fully independent; there is no cross-example or cross-page state.
4. **Example test suite:** unchanged in shape from today, just loading source from a file
   instead of an inline literal.

## Testing

- `adam-web-ui`: contract-style doc comments and unit tests per this repo's convention,
  ported/adjusted from `begin`'s existing `inspector.rs` tests where the logic moved
  unchanged (`cell_flags`, `cell_needs_full_propagate`, `compute_output_status`).
- `begin`: existing test suite must pass unchanged after the extraction — this is a refactor
  of `begin`, not a behavior change. Verify visually via the `verifying-begin-ui` skill
  (screenshot + DOM dump) to confirm the desktop/web app is unaffected.
- Book examples: the existing 27 `#[test]`s continue to assert real behavior, now against
  `include_str!`-loaded `.adm2` files.
- Wasm mount crate: unit tests for the parse-error-renders-diagnostic-instead-of-panicking
  path (the one behavior this crate adds beyond what `adam-web-ui` already provides), run as
  plain `cargo test` against the non-wasm-target logic where possible.
- mdBook preprocessor: unit tests asserting the mount `<div>` is inserted immediately after
  the matching `{{#include}}` and only for `.adm2` includes (not e.g. rustdoc includes
  elsewhere in the book, if any exist).
- End-to-end: after wiring, serve the built book locally and visually confirm at least one
  live example renders and responds to edits in a real browser — `cargo build`/`mdbook
  build` succeeding is not sufficient evidence per this repo's UI verification rule.

## Removed

- Every inline sheet-source string literal in `tests/*.rs` (replaced by `include_str!` of the
  new `.adm2` files).
- Every `{{#include tests/chapter.rs:anchor}}` directive in `book-src/*.md` (replaced by
  `{{#include}}` of the corresponding `.adm2` file).

## Non-goals (this pass)

- Live constraint graph in the book — `begin`'s `GraphView`/`graph.js` stay `begin`-only.
- Native (non-web) bindings for `adam-web-ui` — the crate name leaves room for this later but
  none is built now.
- Any scripted "walkthrough" or preset-interaction affordance on top of the plain live
  inspector (e.g. a "try setting `p` to 1" hint) — out of scope until the plain live view is
  in place and proven useful.

## Phases

1. Spike: verify multiple independent Dioxus `VirtualDom` mounts from one wasm module.
2. Extract `adam-web-ui` from `begin`; verify `begin` unaffected.
3. Convert the book's 27 examples to standalone `.adm2` files + `include_str!`-backed tests;
   update `book-src/*.md` includes.
4. Build the wasm mount crate on top of `adam-web-ui`.
5. Build and register the mdBook preprocessor; wire the bootstrap script and `book.toml`
   asset registration.
6. Extend both GitHub Actions workflows (PR-check, Pages deploy) with the wasm build step.
7. End-to-end verification in a real browser; write the implementation handoff doc if the
   work spans multiple sessions, per this repo's multi-phase work convention.
