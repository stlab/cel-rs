# Issue #272 binding plan handoff

**Snapshot:** 2026-11-06
**Scope:** Task 11 cleanup and integration gate
**Worktree:** `.claude/worktrees/issue-272`
**Base:** `f1aebff9a940da34b7e483daefd42cb14e385435`

## Status

Task 11 removes unused Spectrum components, bridges, bundles, and build tooling.
The cleanup, focused rendered check, publication builds, and complete Rust gate
pass. The Task 11 report records the full changed-file manifest, commands,
results, and evidence paths:
`.superpowers/sdd/2026-11-05-dioxus-sheet-bindings-and-recording/task-11-report.md`.

The Task 10 independent approval remains the baseline. The controller owns the
independent final whole-branch review; this handoff does not claim that review
or approval. No issue was closed, and no PR, push, or merge was performed.

## Scope and preserved contracts

- Begin, EZ, and `adam-web-ui` no longer provide a Spectrum compatibility
  layer or require a JavaScript bundle. `Theme` and shared controls retain their
  official CSS, Lightning CSS compilation, and document-owned stylesheet
  delivery.
- The binding APIs, ownership contracts, and error behavior remain unchanged.
- The obsolete numeric input bridge and its Spectrum-only tests are removed.
  `numeric_events.js` remains the authoritative composition channel, and the
  locale JavaScript, graph/file scripts, and their tests remain.
- The committed Spectrum bundles, Spectrum entry points, and `xtask build-js`
  command are removed. Live-book staging succeeds without `swc.js`; the
  obsolete command returns the normal unknown-task usage error.
- The official CSS exports and their licensing/provenance remain unchanged.
  Historical design documents remain untouched. Active setup guidance no
  longer tells contributors to build or regenerate the deleted bundle.

## Validation

The unfiltered workspace build, tests, doctests, all five Clippy invocations,
and warnings-denying rustdoc command pass after the final test change. Plain
build/test output contains no workspace-source compiler warnings. Cargo
continues to disclose the existing upstream future-incompatibility notice for
`block v0.1.6`.

Retained JavaScript suites pass: Begin graph tests (64), example/file tests (8),
numeric-event and locale tests (20), mdBook bootstrap tests (13), and slides
tests (14). Publication verification builds the live-book WASM, stages
37 examples, builds the mdBook output, and builds slides. The documented fatal
example exits with status 1 after reporting division by zero and writes its
accepted recovery JSON.

The actual Begin web renderer shows the graph, Inspector, and shared controls.
Its DOM embeds `stlab-adam-controls-v1`, contains no `sp-*` controls, and
references only assets that returned HTTP 200. The screenshot and DOM are
retained under the session evidence directory listed in the Task 11 report.
The standalone native stylesheet rendering was already verified with actual
WKWebView evidence in Task 10; Task 11 preserves that delivery.

## Remaining boundaries

- **#279:** The added widget types are preserved; no issue closure was
  attempted.
- **#280:** When the native platform does not expose an authoritative IME
  replacement range, the binding fails closed and reports the limitation
  instead of guessing a write.
- **#281:** The original standalone Cargo stylesheet gap is now verified as
  fixed by the official stylesheet delivery, including actual native
  WKWebView rendering. The issue remains open and is not closed here.
- **#282:** The requirement-lifecycle fix is included, and the unfiltered
  workspace suite passes. The issue remains open and is not closed here.
- **#283:** Tracks the Begin npm audit findings. The four affected lockfile
  package versions match the base and are all development-only; no upgrade was
  made. The Task 11 report records the exact audit output, base comparison,
  dependency paths, and advisory identifiers.

Resume with the final independent branch review. Keep these issue boundaries
explicit; do not repeat broad rendering solely to duplicate the retained Task
5–10 evidence.
