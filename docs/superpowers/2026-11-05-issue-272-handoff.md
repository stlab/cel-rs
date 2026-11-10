# Issue #272 binding plan handoff

**Snapshot:** 2026-11-07
**Scope:** Task 11 cleanup, integration gate, and final-review fix wave
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

## Final-review fix wave

The final review found that reentrant `SheetHandle::apply` outcomes could call
the host while an enclosing owner borrow remained active. `HandleState` now
queues outcomes and drains them synchronously, in order, only after an exclusive
owner borrow can be acquired and released. Reentrant callbacks do not recurse;
they append outcomes for the active drain. Replacement, restoration, playback,
accepted-state/read callbacks, recording access, and widget error reporting all
use the same delivery boundary. Checked errors, accepted mutations, publication,
explicit recovery, and fatal retirement remain unchanged.

Contract tests cover `with_accepted_state`, successful and failed replacement,
restoration, playback, `stop_recording`, `last_recording`, and `CellBinding::report`.
They verify callback-time model/history access, queue order, and one delivery per
outcome. The focused binding tests and full Rust workspace gate pass. The plain
build and test output contains no workspace-source compiler warnings; Cargo
continues to report the existing future-incompatibility notice for `block
v0.1.6`. Exact red/green and full-gate output is retained at
`/Users/sean-parent/.copilot/session-state/ce98aff5-e017-4847-9219-df574e984ddc/files/issue272-final-fix/`.

The exact finding and fix evidence are recorded in
`.superpowers/sdd/2026-11-05-dioxus-sheet-bindings-and-recording/final-fix-report.md`.
The controller's scoped re-review of this fix remains pending; this update does
not claim the final review passed.

## Scope and preserved contracts

- Begin, EZ, and `adam-web-ui` no longer provide a Spectrum compatibility
  layer or require a JavaScript bundle. `Theme` and shared controls retain their
  official CSS, Lightning CSS compilation, and document-owned stylesheet
  delivery.
- Binding signatures and error categories remain unchanged. Host outcomes now
  wait until all enclosing owner borrows end before delivery.
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
- **[#283](https://github.com/stlab/cel-rs/issues/283):** Tracks the Begin npm
  audit findings. The four affected lockfile package versions match the base
  and are all development-only; no upgrade was made. The Task 11 report records
  the exact audit output, base comparison, dependency paths, and advisory
  identifiers.

Resume with the controller's scoped re-review. Keep these issue boundaries
explicit; do not repeat broad rendering solely to duplicate the retained Task
5–10 evidence.

## Control sizing and card layout

Plan: `docs/superpowers/plans/2026-11-09-control-card-layout.md`.

Done:

- **Host-scalable rem.** `adam-web-ui/build.rs` rewrites every embedded `<n>rem`
  length into `calc(<n> * var(--adam-rem, 1rem))` (`build/scale_rem.rs`). A host
  scales all controls by setting `--adam-rem`. The mdBook sets it to `1.6rem` on
  `:root` in `book-rem.css`; the slides deck never loads that file.
- **Vendored Card** component, plus component-local CSS modules for
  `numeric_editor` and `inspector` (scoped `adam_<module>_<local>` classes). The
  theme stylesheet now holds only tokens and the `.adam-theme` scope.
- **`InspectorPresentation { Panel, Card }`** (default `Panel`). The book mounts
  the card presentation without a header.
- **Numeric rows.** The slider sits left of a fixed-width number field. The help
  text is visually hidden but stays in the accessibility tree. Rows are separated
  by the inspector's 1.5rem gap; the host margin on `.adam-cell-row` was removed
  from Begin and the book.

Deferred: `scale_rem` is a byte scanner, not a CSS tokenizer. Its contract
excludes strings and comments that resemble `<n>rem` lengths; no vendored
stylesheet contains one.
