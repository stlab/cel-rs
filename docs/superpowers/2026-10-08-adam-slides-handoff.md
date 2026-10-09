# Adam slides handoff - 2026-10-08

## Initial delivery

- `adam-slides` contains nine Marp slides in the active tutorial's order.
  The generator reads canonical `.adm2` source and rejects coverage drift.
- Same-site static example pages reuse the book's WebAssembly inspector and
  graph driver. Cell edits propagate, graphs update, forced values disable
  editing, diagnostics remain visible, and visited slides preserve edits.
- The complete static output lives in ignored `adam-slides/dist`. It needs
  HTTP hosting, not a backend, published book, or runtime CDN.
- `prepare-live-slides-assets` shares the book's asset staging implementation;
  the existing book command retains its destination and behavior.
- The Docs workflow builds and verifies the presentation and copies it into
  `target/doc/adam-slides` before the existing Pages upload. Deployment occurs
  only when that workflow publishes the changes; this worktree is not deployed.
- Build and preview commands are in `adam-slides/README.md`.
- Live panes fit their allotted size without the graph SVG's inline baseline
  adding a scrollbar. Slide numbers sit below the panes in the bottom margin.

## Verification

- Clean `npm ci`, `npm test` (9 tests), and `npm run build` succeeded.
- Chromium acceptance checks passed (25 tests) against the generated output and
  a separately copied artifact under a nested Pages-style URL.
- Screenshots were inspected for every slide, including the smaller viewport.
  Browser checks cover actual keyboard edits, graph direction, conditional
  graph activation, preserved values, diagnostics, navigation isolation,
  resizing, and visible startup failures.
- `cargo test -p xtask` passed (5 tests).
- `cargo test -p adam-lang-book --test tutorial` passed (6 tests).
- `cargo clippy -p xtask --all-targets -- -D warnings` passed.
- After the user requested a PR, the complete pre-PR suite passed: workspace
  build and tests without compiler warnings, workspace doctests, all five
  clippy invocations, formatting, and rustdoc with warnings denied.
- Independent whole-branch review identified three important gaps: dependency
  execution failures, unsupported tutorial references silently disappearing,
  and a missing host bootstrap preventing its own error reporting. Regression
  tests reproduced each gap before fixes; the complete suites passed afterward.
  Startup now awaits module execution, checks required UI/graph capabilities,
  and reports failures from an HTML-level boundary.
- Layout regressions reproduced the unnecessary scrollbar and overlapping
  pagination before the fixes. All nine slides fit without frame overflow and
  keep pagination below the panes at 1280x720 and 960x540.

## Deliberately deferred

- In-browser source editing remains outside scope. File-backed source editing
  and examples beyond the tutorial are now supported by the follow-up below.
- PDF and PowerPoint exports do not provide live examples.
- `npm audit` still reports upstream Marp build-tool advisories in `extract-zip`
  and KaTeX, tracked in #249. Compatible fixes were applied, including patched
  `basic-ftp` 6.2.2. The build does not use Marp's optional archive/browser-export
  or authored-math paths, and Node dependencies are not shipped to the site.
  This bounds exposure but is not a clean dependency audit.
- The user requested a PR after local completion. No merge or Pages deployment
  was requested. Non-Chromium behavior and actual deployment remain unverified.

## Design records

- Spec: `docs/superpowers/specs/2026-10-08-adam-slides-design.md`.
- Plan: `docs/superpowers/plans/2026-10-08-adam-slides.md`.

## Follow-up: live editing and flexible authoring

- The user's revised requirements supersede the original fixed nine-slide
  count, tutorial coverage, and ordering constraints. Slides can contain text
  alone or freely selected, repeated, and reordered example directives.
- Book references resolve through the existing staged manifest. New `.adm2`
  files under `adam-slides/examples` use reserved `local/` keys; nested local
  directories are supported. Unsafe names and unknown or unsupported references
  remain explicit errors.
- Graphs default to off and use a per-directive `graph=1` option. Existing
  tutorial directives retain their previous graph choices. The user's
  introductory slide and additional prose remain intact.
- `slides: serve (full)` reuses the book WASM build, restores locked dependencies,
  builds the deck, and starts source, Marp, and server tasks sequentially using
  readiness patterns measured from their actual output. The README documents
  VS Code launch, manual startup, shutdown, and value resets on refresh.
- Flexible slide columns accommodate extra prose. Graphs now consume the
  remaining frame space instead of enforcing a 240px minimum; the existing
  SVG baseline and pagination fixes remain intact.
- Browser checks use a separate acceptance deck with a text introduction,
  reordered examples, extra prose, and a local source. A separate check mounts
  every example in the actual authored deck, so removing tutorial slides does
  not invalidate the behavioral regression coverage.
- Shared Spectrum number fields fill their container's inline size so formatted
  readonly values such as `10,000` do not truncate. A rendered browser regression
  measures the available text width after editing the requirement example.
- The root README now puts language guides, live slides, component APIs, and
  application/editor documentation immediately below its badges. The existing
  Docs workflow already builds, checks, and publishes the slide artifact with
  the language books and APIs; no duplicate workflow was added.

### Follow-up verification and boundaries

- Locked `npm ci`, all 10 Node tests, the authored deck build, and all 28
  Chromium tests passed. The shared inspector WASM was rebuilt for the
  number-field sizing change.
- A real watched browser verified Markdown saves adding a slide and `.adm2`
  saves changing both displayed source and mounted values automatically.
  Temporary validation content was removed and owned watcher/server processes
  were stopped. A rendered local-example screenshot was inspected.
- Task dependency wiring and readiness patterns were checked, but the VS Code
  task UI itself was not launched.
- Node's `--watch-path` supports this live-editing workflow on Windows and macOS,
  not Linux. Static builds and browser checks retain Linux support.
- This follow-up targets existing PR #250. Full Rust validation and a fresh
  published Talos review gate publication of each new head; the previous
  validation and review do not certify later changes. No merge or deployment
  is authorized.
