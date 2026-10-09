# Adam slides handoff - 2026-10-08

## Delivered

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

## Verification

- Clean `npm ci`, `npm test` (9 tests), and `npm run build` succeeded.
- Chromium acceptance checks passed (23 tests) against the generated output and
  a separately copied artifact under a nested Pages-style URL.
- Screenshots were inspected for every slide, including the smaller viewport.
  Browser checks cover actual keyboard edits, graph direction, conditional
  graph activation, preserved values, diagnostics, navigation isolation,
  resizing, and visible startup failures.
- `cargo test -p xtask` passed (5 tests).
- `cargo test -p adam-lang-book --test tutorial` passed (6 tests).
- `cargo clippy -p xtask --all-targets -- -D warnings` passed.
- Independent whole-branch review identified three important gaps: dependency
  execution failures, unsupported tutorial references silently disappearing,
  and a missing host bootstrap preventing its own error reporting. Regression
  tests reproduced each gap before fixes; the complete suites passed afterward.
  Startup now awaits module execution, checks required UI/graph capabilities,
  and reports failures from an HTML-level boundary.

## Deliberately deferred

- Source editing, other book chapters, and commented-out tutorial examples are
  outside the approved scope.
- PDF and PowerPoint exports do not provide live examples.
- `npm audit` still reports upstream Marp build-tool advisories in `extract-zip`
  and KaTeX, tracked in #249. Compatible fixes were applied, including patched
  `basic-ftp` 6.2.2. The build does not use Marp's optional archive/browser-export
  or authored-math paths, and Node dependencies are not shipped to the site.
  This bounds exposure but is not a clean dependency audit.
- No push, PR, merge, or Pages deployment was requested. Run the repository's
  complete pre-PR suite before opening a PR.

## Design records

- Spec: `docs/superpowers/specs/2026-10-08-adam-slides-design.md`.
- Plan: `docs/superpowers/plans/2026-10-08-adam-slides.md`.
