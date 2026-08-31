# Live Adam book examples — handoff

Status snapshot as of 2026-08-27, written for whoever needs to pick this work up (open the PR,
respond to CI, or extend it further). Read
`docs/superpowers/specs/2026-08-27-live-adam-book-examples-design.md` first for the full design
and phasing — this doc only summarizes what's done, what's deliberately deferred, and what's
left. All 21 tasks in the plan are complete; this branch (`worktree-live-book`) has not been
pushed or opened as a PR yet.

## What's done

**Phase 1 (extract `adam-web-ui` from `begin`) — Tasks 1–7, complete.**
A new workspace crate, `adam-web-ui`, now holds everything the book's live examples and
`begin`'s own UI both need: the Spectrum component wrappers (`adam_web_ui::spectrum`, moved
verbatim from `begin`), `adam_web_ui::diagnostics::report_error`, `adam_web_ui::labels`
(`Labels`, `format_adam_error`, `labels_from_cell_names`), `adam_web_ui::{build_sheet,
BuildOutcome}` (parses a sheet source string, reports parse/propagate errors alongside a
successfully-built `Sheet` when both occur), and `adam_web_ui::SheetInspector` — the live,
editable cell-list component itself (moved from `begin/src/inspector.rs`). `begin` now depends
on `adam-web-ui` for all of this rather than owning it; `cargo build`/`test`/`clippy`/`doc`
(desktop and web feature sets) all stayed clean throughout the extraction, confirmed by Task 7's
full-suite + `verifying-begin-ui` visual check.

**Phase 2 (multi-mount spike) — Task 8, complete, no commits (scratch-only).**
Confirmed the design's core technical bet before building on it: two independent Dioxus
`VirtualDom` instances, mounted from one wasm module, coexist with zero cross-talk (clicked
independently via CDP, ended at Counter A=3/Counter B=1). This is what makes per-example
independent mounting (rather than one shared app instance) safe; the scratch crate was deleted
per its own plan step.

**Phase 3 (book conversion) — Tasks 9–16, complete.**
All 27 of `adam-lang-book`'s inline code examples are now standalone `.adm2` files under
`adam-lang-book/book-src/examples/<chapter>/`, each backed by an `include_str!`-based
`#[test]` in `adam-lang-book/tests/<chapter>.rs` (27/27 passing) and included into
`book-src/<chapter>.md` via `{{#include}}` instead of being typed inline. A minor finding from
this phase (all 27 fences still labeled ` ```rust ` despite rendering Adam source, not Rust) was
deliberately deferred to the final review pass rather than blocking Phase 3 — see below.

**Phase 4 (wasm mount crate + preprocessor + CI) — Tasks 17–20, complete.**

- `adam-lang-book-live` — the wasm-bindgen crate wasm-pack builds. `mount(element_id, source)`
  parses `source` via `adam_web_ui::build_sheet` and renders either a live `SheetInspector` or
  the formatted diagnostic (or both, if the sheet built but a later `propagate` failed) into a
  freshly-created `VirtualDom` rooted at `element_id`. Each call is fully independent (Task 8's
  spike result, now exercised for real).
- `adam-lang-book-preprocessor` — an mdBook preprocessor binary (`mdbook-live-examples`) that
  scans rendered chapter HTML for Adam-source code fences and inserts a `<div class="adam-live"
  data-example="...">` mount point immediately *after* each fence (not inside it — see the
  Task 18 amendment below), skipping any example named in a `NO_LIVE_MOUNT` exclusion list
  (currently just `expressions/no_standard_library`, whose entire didactic point is behavior
  *without* the standard library the shared live parser always installs).
- `adam-lang-book/book.toml` wires the preprocessor in (`before = ["links"]`, so mdBook's
  built-in `{{#include}}` expansion doesn't run first and hide the fences) plus
  `adam-live-bootstrap.js`/`adam-live.css` as `additional-js`/`additional-css`. The bootstrap
  script dynamically imports the compiled wasm module and fetches a generated
  `adam-live-examples.json` manifest (example name -> source text), both resolved to absolute
  URLs from `document.baseURI` to sidestep a `fetch()`-vs-`import()` base-URL mismatch (see
  Task 19).
- `xtask prepare-live-book-assets` — the CI-facing automation Task 20 added: builds the
  live-example manifest, copies `swc.js`/`inspector.css` from `begin/assets/`, and copies
  `adam-lang-book-live/pkg/` (the `wasm-pack build --target web --release` output) into
  `adam-lang-book/book-src/theme/`, all as one step both workflows (PR-check and Pages deploy)
  now run before `mdbook build`. **Confirmed `dx build` does not work for this crate** (tried
  twice across this plan) — the pipeline uses `wasm-pack` exclusively, never `dx`.

**Task 18 amendment (found during Task 19, fixed immediately) — closes
[stlab/cel-rs#161](https://github.com/stlab/cel-rs/issues/161).**
Task 19's own integration testing discovered that Task 18's already-reviewed preprocessor put
the mount `<div>` *inside* the code fence instead of after it — inert on every real chapter
page, since none of Task 18's own unit tests covered fenced content (only unfenced). Fixed
immediately rather than deferred to a filed issue, per this project's CLAUDE.md Code Review
Findings policy (small, in-scope, already diagnosed and validated) — this is an explicit,
recorded override of the implementer's own more conservative "file an issue" instinct. The fix
went through two follow-up review rounds of its own (an added unterminated-fence regression
test, then a trailing-whitespace Minor fixed opportunistically) before landing clean;
9/9 preprocessor tests pass and issue #161 is closed.

**Deferred-Minors pass (after Phase 4, before Task 21) — fixed proactively.**
Two Minor findings that earlier task reviews had explicitly parked ("fix opportunistically",
"flag for final review triage") were fixed proactively rather than left for Task 21 to
rediscover, per CLAUDE.md's "fix small in-scope findings immediately" policy:

- A stale `[`Sheet`]`/`[`Labels`]` intra-doc-link doc comment in `begin/src/example_source.rs`
  (dangling since Task 5 moved those types into `adam-web-ui`).
- All 27 `` ```rust `` code fences across `book-src/*.md` un-tagged to a plain fence, since they
  render Adam sheet source, not Rust. The preprocessor's fence detection is delimiter-only (it
  never reads the info string), so this had zero effect on mount-div placement — independently
  confirmed by the reviewer.

**Task 21 (this task) — full end-to-end verification, complete.**
Re-ran the entire check suite from Step 1 fresh (`fmt --check`, `build`, `test`, `test --doc`,
all three `clippy` invocations, `RUSTDOCFLAGS="-D warnings" cargo doc`, the last one forced to a
full non-cached rebuild) — all eight clean, no code changes needed for that step. Then ran the
now-fully-automated pipeline (`cargo install --path adam-lang-book-preprocessor`, `wasm-pack
build --target web --release`, `cargo run -p xtask -- prepare-live-book-assets`, `mdbook build`)
and verified the real `book-dist/` output in headless Edge via raw CDP across four chapter pages
(`tutorial.html`, `relationships.html`, `filters.html`, `expressions.html`) — editable live cell
lists, live re-derivation on edit, clean diagnostic rendering for two different intentionally-
invalid examples, the `no_standard_library` exclusion, and mount independence all confirmed. See
`.superpowers/sdd/2026-08-27-live-adam-book-examples/task-21-report.md` for full command
transcripts and browser-verification evidence.

**Genuine defect found and fixed during Task 21 — closes
[stlab/cel-rs#162](https://github.com/stlab/cel-rs/issues/162).**
The first real interactivity check (editing `tutorial.html`'s multiplication-triangle example)
silently failed: writing `b` never re-derived `c`. Root cause: `adam-lang-book-live`'s `Root`
component never loaded `swc.js` (the Spectrum Web Components bundle) the way `begin/src/app.rs`
does for its own single, page-wide `App` — and structurally can't, since each `.adm2` example is
deliberately its own independent `VirtualDom` (Task 8), so a per-mount `document::Script` would
redefine the same custom elements repeatedly and throw. `adam-live-bootstrap.js` already copied
`swc.js` into the theme output but never loaded it. Every `sp-*` element on every book page was
therefore an *undefined* custom element: no shadow DOM, and — since an undefined custom element
renders none of its would-be shadow content — **no visible input box at all**, not just a
functionality gap. Fixed by loading `swc.js` once per page from `adam-live-bootstrap.js` itself
(in parallel with the existing wasm-module `import()`/manifest `fetch()`), before any `mount()`
call. Committed as `b009b57`, fixed immediately per the same CLAUDE.md policy Task 19's #161 fix
followed, rather than deferred. Verified after the fix: real click+keyboard input into the
triangle's `b` field correctly re-derives `c` (`a=2, b=9 -> c=18`), and a fresh screenshot shows
real styled Spectrum number-field inputs instead of blank space.

## What's deferred (per the spec's own Non-goals — not started, not planned for this pass)

1. **Live constraint graph in the book.** `begin`'s `GraphView`/`graph.js` stay `begin`-only;
   the book's live examples only ever show the flat `SheetInspector` cell list.
2. **Native (non-web) bindings for `adam-web-ui`.** The crate name deliberately leaves room for
   a future non-wasm consumer, but none exists yet — `adam-web-ui` today is exercised only by
   `begin` (desktop WebView2 + web/wasm builds) and `adam-lang-book-live` (wasm-only).
3. **Any scripted "walkthrough" or preset-interaction affordance** on top of the plain live
   inspector (e.g. a "try setting `p` to 1" hint) — explicitly out of scope until the plain live
   view is in place and proven useful, which Task 21 is what confirms.

## What's left

- **Real GitHub Actions validation of the CI changes is still pending.** Task 20 added the
  `wasm-pack build` + `xtask prepare-live-book-assets` steps to both the PR-check and Pages-
  deploy workflows and reasoned through their correctness locally, but neither workflow has
  actually run on GitHub's runners yet — that only happens once this branch is pushed and a PR
  is opened, which has **not** happened as of this handoff. Whoever picks this up next should
  push the branch, open the PR, and watch both workflows run for real before merging — CI
  environment differences (available tools, working-directory assumptions, path casing) are
  exactly the kind of thing local verification in this worktree can't fully rule out.
- No other work is outstanding against the plan; all 21 tasks are complete.

## Key files for anyone extending this work

- `adam-web-ui/src/{lib.rs, spectrum.rs, diagnostics.rs, labels.rs, inspector.rs}` — the
  reusable live-sheet UI: `build_sheet`/`BuildOutcome`, `SheetInspector`, the Spectrum component
  wrappers, and the desktop-vs-web `report_error` split.
- `adam-lang-book-live/src/lib.rs` — the wasm-bindgen `mount(element_id, source)` entry point
  and its `Root` component; now also where `swc.js` loading is documented as page-level, not
  per-mount (see the code comment added alongside the Task 21 fix, in
  `adam-lang-book/book-src/theme/adam-live-bootstrap.js`, not this file).
- `adam-lang-book-preprocessor/src/lib.rs` — the `mdbook-live-examples` preprocessor;
  `NO_LIVE_MOUNT` is the exclusion list for examples that must never get a live mount.
- `adam-lang-book/book-src/theme/{adam-live-bootstrap.js, adam-live.css}` — the two hand-written,
  committed files that drive the book-side mounting; everything else under
  `adam-lang-book/book-src/theme/` is a build output (`.gitignore`d, regenerated by
  `xtask prepare-live-book-assets` + `wasm-pack`, safe to delete locally at any time).
- `xtask/src/live_book_assets.rs` — `prepare-live-book-assets`, the CI-facing asset-preparation
  subcommand.
- `.github/workflows/` — both workflows now run `wasm-pack build` + `cargo install --path
  adam-lang-book-preprocessor` + `xtask prepare-live-book-assets` before `mdbook build`; not yet
  validated on a real GitHub Actions run (see "What's left" above).
- `adam-lang-book/book-src/examples/<chapter>/*.adm2` + `adam-lang-book/tests/<chapter>.rs` —
  the 27 converted examples and their `include_str!`-backed regression tests.
