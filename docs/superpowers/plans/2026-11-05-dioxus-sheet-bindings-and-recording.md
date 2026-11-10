# Dioxus sheet bindings and recording implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bind Dioxus widgets to named Adam cells, demonstrate recording/playback,
and replace Spectrum controls in the existing application and publishing hosts.

**Architecture:** `adam-web-ui` owns reusable Dioxus controls and a separate
Sheet binding layer. A host-owned controller retains accepted input history,
publishes accepted value/state snapshots, and forwards typed outcomes to the
application boundary. Model construction returns a `Result`; errors never
carry a sheet.

**Tech stack:** Rust 2024, Dioxus 0.7.10, official Dioxus component sources and
primitives, browser `Intl.NumberFormat`, existing serde/serde_json, Rust unit
and Dioxus VirtualDom tests, existing JavaScript test runners.

**Spec:** [Approved Dioxus sheet bindings and recording design](../specs/2026-11-05-rust-host-integration-and-recording-design.md).

**Status:** Approved on 2026-11-05 for subagent-driven execution. Implementation
is in progress in the issue-272 worktree.

## Global constraints

- `adam-rs` and `adam-lang` remain independent of Dioxus.
- Model construction returns a `Result`.
- An `Err` contains a structured error and any independently owned diagnostic
  metadata needed by the host; it never contains a sheet, whether as a partial
  result or an optional recovery payload.
- Any `Sheet` whose mutating operation returns `Err` is unspecified but safely
  discardable.
- The component must not read, inspect, capture, retry, return as active, or
  otherwise reuse that sheet.
- A propagation `Err` follows the fatal path and does not become a replayable
  action. Do not rebuild or retry around it.
- The component forwards all errors and validation outcomes to the client.
- Recovery retains accepted aspirations and input ordering. Recordings capture
  effective source values after successful propagation.
- Focus loss and programmatic synchronization never count as user edits.
- Keep `ez-adam` changes limited to the agreed component migration and directly
  dependent code; open a separate issue for unrelated runtime recovery work.
- No component promises that a `Sheet`, adapter, or callback is `Send` or `Sync`.
- Additional component types, including dates, belong to #279.
- Work only in `.claude/worktrees/issue-272`, branch `worktree-issue-272`.
  Run `cargo fmt --all` before every commit and include the Copilot coauthor.
- Before product code, invoke `algorithm-research` and the applicable
  implementation skills. Research may simplify an internal implementation;
  changing an approved observable contract requires returning to the user.

## Review focus

These failure classes require explicit tests in their owning tasks:

1. A callback or formatter reply arrives after source replacement: it cannot
   update the replacement, even when both roots use the same cell names
   (Tasks 4 and 6).
2. A rejected edit follows an accepted filtered aspiration or direction change:
   restoring effective values alone must not erase the accepted aspiration or
   change relationship direction (Task 2).
3. A browser locale uses space grouping, localized digits, or bidirectional
   marks: the editor round-trips the number without rounding wide integers
   or converting a blank draft to zero (Task 4).
4. An input method is composing text, or a stale formatting reply arrives during
   an incomplete draft: no partial composition is submitted and the draft
   survives until valid submission, blur, or disabling (Tasks 4 and 5).
5. EZ Adam clears an optional clamp bound: the document property becomes unset;
   this must not change the bound Sheet editor's rule that a blank draft sends
   no numeric value (Tasks 5 and 9).

## File responsibilities and execution order

All paths in this plan are relative to the issue-272 worktree. New paths are
proposed files; source line ranges identify the baseline and will move.

| Area | Files | Responsibility |
| --- | --- | --- |
| Diagnostic ownership | `adam-lang/src/sheet_diagnostics.rs` (new), `adam-lang/src/error_labels.rs`, `adam-lang/src/parser.rs`, `adam-lang/src/lib.rs` | Retain static source/provenance independently of a live Sheet; reuse the existing error-location resolver. |
| Construction | `adam-web-ui/src/build.rs`, `adam-web-ui/src/labels.rs`, `adam-web-ui/src/lib.rs` | Result-only initialization, typed failures, exact value codecs and UI metadata. |
| Host controller | `adam-web-ui/src/binding/mod.rs`, `owner.rs`, `types.rs`, `recording.rs`, `dioxus.rs`, `tests.rs` (new) | Transactions, accepted history, recording, reactive handles and outcome forwarding. |
| Numeric behavior | `adam-web-ui/src/input.rs` (new), `adam-web-ui/src/components/number/{mod.rs,locale.rs,draft.rs,locale.js}` (new), `adam-web-ui/js/number-locale.test.mjs` (new) | Shared input errors/types, locale symbols/formatting, exact normalization, numeric editing state machine. |
| Base controls | `adam-web-ui/src/components/mod.rs`, generated control subdirectories, `numeric_editor.rs`, `theme.rs`, `tests.rs` (new), `adam-web-ui/assets/` | Official components plus the reusable numeric editor and shared theme. |
| Bound controls | `adam-web-ui/src/binding/widgets.rs` (new), `adam-web-ui/src/inspector.rs`, `adam-web-ui/src/inspector_tests.rs` | Concise binding props and the generated Inspector built from those same controls. |
| Runnable example | `adam-web-ui/examples/sheet_binding.rs`, `sheet_binding.adm2` (new), `adam-web-ui/Cargo.toml` | Consumer setup, explicit widget bindings, recording/playback and visible outcomes. |
| Begin | `begin/src/app.rs`, `app_tests.rs`, `example_source.rs`, `recovery.rs` (new), `begin/Cargo.toml`, `begin/assets/{app-shell.css,inspector.css,graph.css}` | Central host policy, accepted state persistence, migrated application controls. |
| Graph boundary | `adam-web-ui/src/graph/{constraints.rs,bridge.rs,view.rs}`, `adam-web-ui/tests/view_constraints.rs` | Discard failed graph constraint models and forward failures to the owning application. |
| EZ Adam | `ez-adam/src/ui/{app.rs,toolbar.rs,side_panel.rs}`, directly dependent UI tests | Base controls over existing Document operations, without runtime Sheet integration. |
| Publishing | `adam-lang-book-live/src/lib.rs`, `adam-lang-book/book-src/theme/{adam-live-bootstrap.js,inspector.css}`, `adam-lang-book/book.toml`, `xtask/src/live_book_assets.rs`, existing bootstrap/slides tests | Migrated live roots, theme assets, independent examples and graph associations. |
| Guidance and cleanup | `docs/dioxus-integration.md` (new), `README.md`, `adam-lang-book/book-src/reference.md`, `xtask/src/main.rs`, application package manifests/locks and active setup instructions | Consumer guide, obsolete Spectrum removal, build/test guidance and handoff. |

Task dependencies: (1, 4) -> 2 -> 3; 4 -> 5; (3, 5) -> 6;
(3, 6) -> 7; 6 -> 8; 5 -> 9; (6, 8) -> 10;
(7, 8, 9, 10) -> 11. Tasks 1 and 4 can run independently. Keep commits
buildable by updating directly affected call sites in the task changing an API.

## Task 1: Result-only construction and independent diagnostics

**Files:** Modify `adam-lang/src/{parser.rs,error_labels.rs,lib.rs}`,
`adam-web-ui/src/{build.rs,labels.rs,lib.rs}` and their existing tests. Create
`adam-lang/src/sheet_diagnostics.rs`. Update construction callers in
`begin/src/{app.rs,app_tests.rs,example_source.rs}`,
`adam-lang-book-live/src/lib.rs`,
`adam-web-ui/src/{graph/data.rs,inspector.rs,inspector_tests.rs}`.

**Interfaces:**
- Produce `SheetDiagnostics`, an owned snapshot of static names, sources, spans
  and native provenance, with no `Sheet` field or reference.
- Add `ParsedSheet::diagnostics(&self) -> SheetDiagnostics` and
  `SheetDiagnostics::locate_error(&self, error: &adam_rs::Error) -> Vec<SourceLabel>`.
  Both call the same metadata-only resolver, not two copies of location logic.
- Produce `RuntimeFailure { error: adam_rs::Error, diagnostics: SheetDiagnostics }`.
- Produce `ValidationIssue { key: String, requirement: Option<String>,
  labels: Vec<SourceLabel>, message: String }`.
- Produce `BuiltSheet { parsed: ParsedSheet, labels: Labels,
  diagnostics: SheetDiagnostics, keys: IndexMap<String, CellId> }`.
  Initialize stable keys from public root names; a host factory can explicitly
  add schema keys/codecs for private source cells before returning its model.
- Produce `BuildError::{Parse { error: cel_parser::ParseError, source: String,
  name: String }, Module(adam_lang::ModuleDiagnostic), Runtime(RuntimeFailure),
  Validation(Vec<ValidationIssue>)}` and
  `BuildError::render(&self, renderer: &Renderer) -> String`.
- Replace construction with
  `build_sheet(source: &str, file_name: &str) -> Result<BuiltSheet, BuildError>`.
  Add `initialize_sheet(parsed: ParsedSheet) -> Result<BuiltSheet, BuildError>`
  for explicitly selected, application-configured roots.
- Change `format_adam_error` to consume `&SheetDiagnostics` instead of
  `&ParsedSheet`; preserve diagnostic content and source ownership.

- [ ] **Write the failing contract tests.** Add
  `diagnostics_survive_dropping_the_model`,
  `construction_rejects_initial_runtime_failure`, and
  `construction_rejects_initial_requirement_violation`.
  Their essential assertions are:

  ```rust
  assert!(matches!(build_sheet("sheet s { out x: i64 := 1i64 / 0i64; }", "s.adm2"),
      Err(BuildError::Runtime(_))));
  assert!(matches!(build_sheet(
      "sheet s { cell x: i64 = -1i64 require { @positive x > 0i64; }; }", "s.adm2"),
      Err(BuildError::Validation(_))));
  assert_eq!(render_after_model_drop, render_before_model_drop);
  ```

  The diagnostic fixture retains context before the failing mutation, drops the
  candidate on `Err`, and never inspects it afterward. Keep existing component
  definition/invocation span tests as regression coverage.
- [ ] **Run the red tests:** `cargo test -p adam-web-ui --lib construction`
  and `cargo test -p adam-lang --lib diagnostics_survive_dropping_the_model`.
  Expect missing APIs/tests or the existing failed-sheet behavior to fail.
- [ ] **Implement the interfaces.** Capture diagnostics before propagation;
  check all reported requirement violations only after successful propagation.
  Drop the candidate on `Err`. Remove `BuildOutcome`, its optional sheet/error
  fields, and string-only error handling at callers. An initial build failure
  renders an error-only state; a rejected reload keeps the accepted model.
  Preserve the distinct fatal classification for propagation failures.
- [ ] **Run green tests:** `cargo test -p adam-lang -p adam-web-ui --lib`,
  `cargo test -p begin --no-default-features`, and
  `cargo check -p adam-lang-book-live`. Intentional error examples must assert
  the documented error rather than require every example to build successfully.
- [ ] **Commit** the diagnostic and construction redesign with its callers.

## Task 2: Host transactions, cell codecs and accepted recovery history

**Files:** Create `adam-web-ui/src/binding/{mod.rs,types.rs,owner.rs,tests.rs}`.
Modify `adam-web-ui/src/{labels.rs,lib.rs}` and metadata tests.
**Depends on:** Tasks 1 and 4.

**Interfaces:**
- `CellEdit { key: String, canonical: String }` addresses a stable host schema
  key, never a persisted `CellId`.
- `UpdateOrigin::{User, Playback, Recovery}` identifies feedback-loop policy.
- Consume Task 4's `InputError`, `NumberKind` and `NumericRange` from `input.rs`.
- `PreparedWrite::apply(self, sheet: &mut Sheet, id: CellId) -> Result<(), Error>`
  consumes an already parsed, type-erased value. Keep its storage private.
- Separate `CellMeta` parsing from mutation:
  `prepare: Box<dyn Fn(&str) -> Result<PreparedWrite, InputError>>` and
  `capture: Box<dyn Fn(&Sheet) -> Result<String, InputError>>`.
  Display formatting remains distinct from exact capture encoding.
- Add `ScalarCodec<T> { encode: fn(&T) -> String,
  decode: fn(&str) -> Result<T, InputError> }` and
  `Labels::add_cell_with_codec<T: Any>(&mut self, id: CellId, label: &str,
  codec: ScalarCodec<T>)`. The encoder/decoder contract preserves values.
  Built-in bool, String and all 14 primitive numeric types get exact codecs.
  Preserve compound display; unsupported writes/captures report explicit errors.
- `ModelFactory = Rc<dyn Fn() -> Result<BuiltSheet, BuildError>>`.
- `RecoveryState { schema_key: String, transactions: Vec<Vec<CellEdit>> }`
  records accepted input groups, including their unclamped aspirations and order.
- `CellFlags { disabled: bool, readonly: bool, relevant: bool, invalid: bool,
  forced: bool }` and `CellView { key: String, text: String,
  is_source: bool, flags: CellFlags,
  number_kind: Option<NumberKind>, range: Option<NumericRange>,
  issues: Vec<ValidationIssue> }` are published views.
- `HostOutcome::{Accepted, Rejected(HostRejection), Fatal(RuntimeFailure)}`;
  `HostRejection::{Input(InputError), Binding(BindingError),
  Validation(Vec<ValidationIssue>), Mutation(RuntimeFailure)}`.
  `BindingError` distinguishes unknown key, incompatible editor, unsupported
  codec, retired instance and reentrant mutation.
- `SheetOwner::new(factory: ModelFactory, schema_key: String) -> Result<Self, BuildError>`.
- `SheetOwner::apply(&mut self, edits: &[CellEdit], origin: UpdateOrigin) -> HostOutcome`.
- `SheetOwner::restore_accepted(&mut self) -> Result<(), BuildError>`.
- `SheetOwner::accepted_state(&self) -> &RecoveryState` and
  `SheetOwner::views(&self) -> &[Rc<CellView>]`.
- `SheetOwner::inspect<R>(&self, f: impl FnOnce(&ParsedSheet, &Labels) -> R)
  -> Result<R, BindingError>` exposes read-only access only to an accepted,
  live model. It reports unavailable after rejection/failure until explicit
  restoration, and never invokes the callback on the discarded candidate.

- [ ] **Write failing tests** named `conversion_rejection_does_not_mutate`,
  `requirement_rejection_preserves_accepted_aspiration_and_direction`, and
  `propagation_failure_retires_model_without_rebuilding`.
  Use a factory counter and a model with a filtered cell accepting aspiration
  `20` but displaying `10`, plus a two-way relationship. After a later rejected
  transaction and an explicit host call to restore, assert:

  ```rust
  assert_eq!(restored_aspiration, 20);
  assert_eq!(restored_effective_value, 10);
  assert_eq!(restored_direction, accepted_direction);
  assert_eq!(factory_calls_after_fatal, factory_calls_before_fatal);
  ```

  Observe successful rebuilt state through public APIs, never the failed sheet.
  Also cover a write error, two edits in one transaction, initialization with
  no outputs, and a restoration failure that reports rather than substitutes.
- [ ] **Run red:** `cargo test -p adam-web-ui --lib binding::tests`.
- [ ] **Implement the owner and codecs.** Parse every edit before the first
  mutation. Take the live candidate out of active storage while mutating;
  put it back only on acceptance. On rejection retain published accepted views
  and history, but do not choose recovery for the host. On fatal propagation
  drop the model and reject subsequent mutation. `restore_accepted` rebuilds
  with the same factory and replays accepted transaction groups in order.
  Separate parsing failures from mutating failures; no guessed error recovery.
  Published views retain unsupported named cells with an explicit capability
  diagnostic instead of silently omitting them.
- [ ] **Run green:** `cargo test -p adam-web-ui --lib`.
- [ ] **Commit** the tested host controller and value codec changes.

## Task 3: Effective-source recording and fresh-model playback

**Files:** Create `adam-web-ui/src/binding/recording.rs`; extend
`binding/{types.rs,owner.rs,tests.rs}`.
**Depends on:** Task 2.

**Interfaces:**
- `Recording { schema_key: String, values: Vec<CellEdit> }` uses exact canonical
  values and stable keys and supports serde persistence.
- `CaptureError` distinguishes unknown result, unnamed source, missing codec,
  encoding failure, schema mismatch and unavailable model.
- `SheetOwner::capture(&self, result_keys: &[&str]) -> Result<Recording, CaptureError>`.
- `SheetOwner::playback(factory: ModelFactory, schema_key: String,
  recording: &Recording) -> Result<Self, PlaybackError>`.
- `PlaybackError::{Capture(CaptureError), Build(BuildError),
  Rejected(HostRejection), Fatal(RuntimeFailure)}` retains error classification.
- `SheetOwner::recordings(&self) -> &[Rc<Recording>]`; recording is explicitly
  enabled for selected result keys through
  `start_recording(&mut self, result_keys: &[&str]) -> Result<(), CaptureError>`
  and disabled through `stop_recording(&mut self)`.
- Extend the outcome enum with `HostOutcome::RecordingFailed(CaptureError)`.
  This reports a recorder failure after accepting a model transaction; the
  accepted history/views remain current and the host must not roll them back
  merely because recording stopped.

- [ ] **Write failing tests** `recording_captures_effective_not_aspired_values`,
  `playback_writes_all_sources_before_propagating`, and
  `playback_does_not_record_itself`. For aspiration `20` filtered to `10`:

  ```rust
  assert_eq!(captured_value, "10");
  assert_eq!(replayed_effective_value, 10);
  assert!(played_back_owner.recordings().is_empty());
  ```

  Include a self-referencing producer, a conditional source change, a source
  with no stable key, a schema/type mismatch, and values whose float display
  rounds but whose exact capture encoding must round-trip.
- [ ] **Run red:** `cargo test -p adam-web-ui --lib recording`.
- [ ] **Implement capture using only `source_cells` and public typed reads
  through the codecs.** Deduplicate stable keys, reject unsupported sources
  explicitly, validate playback schema/decoding before mutation, write the
  complete group, then propagate and validate once. Playback builds a fresh
  owner and never substitutes effective recordings for recovery history.
  A recording failure is reported separately from an already accepted model
  transaction; stop recording rather than silently dropping an action.
- [ ] **Run green:** `cargo test -p adam-web-ui --lib`.
- [ ] **Commit** recording and playback with their tests.

## Task 4: Locale-aware numeric draft state machine

**Files:** Create `adam-web-ui/src/input.rs`, `adam-web-ui/src/components/mod.rs`,
`adam-web-ui/src/components/number/{mod.rs,locale.rs,draft.rs,locale.js}`
and `adam-web-ui/js/number-locale.test.mjs`; expose the module from `lib.rs`.
**Depends on:** No other task; coordinate shared `lib.rs` edits.

**Interfaces:**
- `InputError` in `input.rs` distinguishes lexical, type-range,
  unsupported-editor and locale failures with model-independent diagnostics.
- `NumberKind::{Signed(u32), Unsigned(u32), F32, F64}` preserves concrete
  primitive range information. `NumericRange { min: String, max: String }`
  carries exact canonical endpoints. Define both in `input.rs`.
- `NumberLocale` owns resolved locale, digit tokens, decimal/group separators,
  signs and ignorable formatting marks.
- `normalize_draft(raw: &str, locale: &NumberLocale, kind: NumberKind)
  -> Result<NormalizedDraft, InputError>` returns
  `NormalizedDraft::{Incomplete, Complete(String)}` without numeric coercion.
- `step_number(current: &str, kind: NumberKind, step: &str,
  increment: bool) -> Result<String, InputError>` uses checked integer
  arithmetic and concrete float-range checks.
- `NumberDraft::reduce(&mut self, event: DraftEvent,
  locale: &NumberLocale, kind: NumberKind) -> DraftEffect`.
  Define `DraftEvent::{Input(String), Focus, Blur, CompositionStart,
  CompositionEnd(String), ModelUpdate { canonical: String, disabled: bool,
  readonly: bool, generation: u64 }, Formatted { text: String,
  generation: u64, revision: u64 }}` and
  `DraftEffect::{None, Submit(String), Report(InputError)}`.
- `locale.js` supplies `numberLocale(locale)` and
  `formatNumber(canonical, integer, locale, options)`, without inspecting an
  input element. Use `Intl.NumberFormat.formatToParts` for symbols and
  `BigInt(canonical)` for integer formatting; never `Number` for integer text.
  Float formatting may use `Number` only after concrete float validation.

- [ ] **Write failing reducer and JavaScript tests.** Pin these assertions:

  ```rust
  assert_eq!(normalize(en_us, "-"), Incomplete);
  assert_eq!(normalize(en_us, "."), Incomplete);
  assert_eq!(normalize(de_de, "1.234,5"), Complete("1234.5"));
  assert_eq!(normalize(en_us, "18,446,744,073,709,551,615"),
      Complete("18446744073709551615"));
  assert_eq!(effects_after_incomplete_input_and_blur, [None, None]);
  assert_eq!(display_after_blur, latest_model_display);
  ```

  The helpers call the interfaces above with fixture `NumberLocale` values.
  Add en-US/de-DE/fr-FR/ar-EG symbol/format tests, i128/u128 boundaries,
  malformed grouping, exponent prefixes, empty input, typed overflow,
  composition, a disabled update during a draft, and stale formatter replies.
  Model-provided NaN/infinity display explicitly and disable stepping for them;
  textual non-number input must not become a finite Sheet write.
- [ ] **Run red:** `cargo test -p adam-web-ui --lib components::number`
  and `node --test adam-web-ui/js/number-locale.test.mjs`.
- [ ] **Implement the reducer and locale helpers.** Reuse the token
  normalization knowledge in the existing `numeric_input.js` rather than its
  Spectrum/shadow-DOM bridge. A draft retains its originating locale while
  focused so a locale change cannot reinterpret its digits. Tag asynchronous
  formatting with instance and edit revisions. JSON-encode strings embedded
  in an eval script; locale/transport failures are explicit errors.
- [ ] **Run green** with the same two commands.
- [ ] **Commit** the numeric behavior independently of widget styling.

## Task 5: Shared Dioxus controls and reusable numeric editor

**Files:** Create `adam-web-ui/src/components/{numeric_editor.rs,theme.rs,tests.rs}`,
generated component subdirectories and shared theme assets. Modify
`adam-web-ui/src/components/mod.rs`,
`adam-web-ui/Cargo.toml`, `Cargo.lock`, and `lib.rs`.
**Depends on:** Task 4.

**Interfaces:**
- Import the official Button, Input, Checkbox, Switch, Slider, Label and
  Separator components. Their generated props remain the base-control APIs.
- `Theme(children: Element) -> Element` installs shared styling without
  requiring a Spectrum script.
  Introduce the `components/mod.rs` module skeleton in Task 4; Task 5 fills
  out its component exports.
- `NumericEditor` props: `id: String`, `label: String`,
  `value: ReadSignal<Option<String>>`, `kind: NumberKind`,
  `range: ReadSignal<Option<NumericRange>>`, `step: String`,
  `disabled: ReadSignal<bool>`, `readonly: ReadSignal<bool>`,
  `invalid: ReadSignal<bool>`, `locale: Option<String>`,
  `optional: bool`, `on_value: EventHandler<Option<String>>`,
  `on_error: EventHandler<InputError>`.
  Values and callbacks use canonical numeric strings; the visible draft is
  localized. A required editor treats blank input as incomplete. An optional
  document property treats blank input as an accepted `None`.
- `NumericSlider` composes the official slider and `NumericEditor`, sharing
  these canonical value/error callbacks. Use its f64 handle only for float and
  <=32-bit integer domains; wide integers retain the exact editor, as before.

- [ ] **Write failing VirtualDom tests** for controlled value updates, numeric
  submission, blur without submission, disabled/read-only behavior, labels and
  help/error associations. Assert optional clearing and required blank behavior:

  ```rust
  assert_eq!(optional_callbacks_after_clear, [None]);
  assert!(required_callbacks_after_clear_and_blur.is_empty());
  assert!(callbacks_after_programmatic_update.is_empty());
  ```

- [ ] **Run red:** `cargo test -p adam-web-ui --lib components::tests`.
- [ ] **Import and implement the controls.** Use the verified importer command
  from `adam-web-ui/`:

  ```bash
  dx components add button input checkbox switch slider label separator \
    --git https://github.com/DioxusLabs/dioxus-components \
    --rev 9a758255ea26e2b20c8cecf4c1c946feb9e71da7
  ```

  Retain upstream notices and licenses. Pin a generated git dependency on
  `dioxus-primitives` to that revision rather than leaving a moving branch;
  disable its unused default router feature. Keep the existing Dioxus version.
  Adapt generated CSS/assets for a library used by multiple roots; add adjacent
  contracts required by repository conventions. Capture full native text events
  synchronously in a shared web/desktop source adapter; transport immutable
  snapshots through the component-owned Dioxus eval channel. Retain native
  selection, composition passage, input type, and composition state. Validate
  actual full snapshots against the owned replacement span for either
  composition event order; never use a timer or reconstruct a submitted value.
  Correlate terminal echoes without suppressing ordinary equal-valued edits.
  Retire sessions on reset/blur/replacement and acknowledge source epochs.
  Report unsupported native ranges and channel failures as `EventSource` errors.
  This user-authorized Task 5 extension replaces the lossy FormEvent path, not
  the prohibition on asynchronous DOM-value reads. Resolve locale once per
  locale change and feed the reducer. Preserve focused text and selection.
- [ ] **Run green:** `cargo test -p adam-web-ui --lib` and the locale JS tests.
  Render a controlled numeric editor with disabled and validation states using
  the existing rendered-verification workflow; retain evidence in session files.
- [ ] **Commit** controls, imported source provenance and lockfile changes.

## Task 6: Reactive cell handles, bound widgets and Inspector

**Files:** Create `adam-web-ui/src/binding/{dioxus.rs,widgets.rs}`; modify
`binding/{mod.rs,tests.rs}`, `inspector.rs`, `inspector_tests.rs`, `lib.rs`.
**Depends on:** Tasks 3 and 5.

**Interfaces:**
- `SheetHandle` is a cloneable Dioxus owner handle, not a mutable Sheet.
- `use_sheet(factory: ModelFactory, schema_key: String,
  on_outcome: EventHandler<SheetEvent>) -> Result<SheetHandle, Rc<BuildError>>`
  caches initialization across renders.
- `SheetEvent { sheet: SheetHandle, outcome: HostOutcome }` lets the outer
  handler choose recovery without circular initialization of callbacks.
- `SheetHandle::cell(&self, name: &str) -> CellBinding` creates a symbolic
  instance-owned binding; `CellBinding::view(&self) -> Result<Rc<CellView>, BindingError>`
  performs checked lookup. Unknown names render/report configuration errors,
  never panic or disappear silently.
- `SheetHandle::restore_accepted(&self) -> Result<(), BuildError>` and
  `with_accepted_state<R>(&self, f: impl FnOnce(&RecoveryState) -> R) -> Result<R, BindingError>`
  expose host recovery without copying the full journal on each publication.
  Checked reentrant access returns `ReentrantMutation` without invoking the callback.
- `SheetHandle::inspect<R>(&self, f: impl FnOnce(&ParsedSheet, &Labels) -> R)
  -> Result<R, BindingError>` tracks accepted-model reads for the host's
  application-specific derived views, including graph projection.
- `SheetHandle::capture(&self, result_keys: &[&str]) -> Result<Recording, CaptureError>`
  exposes Task 3; `replace(&self, factory: ModelFactory,
  schema_key: String) -> Result<(), BuildError>` validates before replacing.
- `SheetHandle::start_recording(&self, result_keys: &[&str]) -> Result<(), CaptureError>`,
  `stop_recording(&self)`, and `last_recording(&self) -> Option<Rc<Recording>>`
  expose recorder controls without cloning the recording history.
- `SheetHandle::playback(&self, factory: ModelFactory, schema_key: String,
  recording: &Recording) -> Result<(), PlaybackError>` installs a fresh,
  successfully replayed owner and retires the prior instance on success.
- `EditNumber`, `EditText`, `CheckBox`, and `EditSlider` each expose
  `bind: CellBinding`, plus ordinary labeling/style props.
- `SheetInspector(sheet: SheetHandle) -> Element` uses those bound widgets.

- [ ] **Write failing event-dispatch tests** in `inspector_tests.rs` for:
  `binding_edit_updates_other_widget`, `derived_cell_write_changes_direction`,
  `unknown_or_incompatible_binding_reports_error`,
  `retired_binding_cannot_write_replacement`, and
  `host_callback_runs_after_model_borrow_is_released`. Assert:

  ```rust
  assert_eq!(other_widget_value_after_edit, expected_derived_value);
  assert_eq!(replacement_value_after_old_event, replacement_initial_value);
  assert!(host_reentrant_restore_result.is_ok());
  ```

  Also mount two roots with identical source names and verify independent
  drafts, control IDs, callback routing and shared-style installation.
  Add a rejected checkbox edit and assert that the actual DOM checked state
  returns to the accepted value even when that value did not change; a VDOM
  equality check alone does not prove a controlled input reset.
- [ ] **Run red:** `cargo test -p adam-web-ui --lib inspector_tests`.
- [ ] **Implement bindings and refactor Inspector** to consume published views,
  not raw `Signal<ParsedSheet>`. Reject stale instance generations and release
  owner borrows before invoking the host callback. Formatting and widget refresh
  never submit another edit. Reuse existing output relevance, forced-state and
  invalid-contributor rules; publish them in `CellFlags`.
- [ ] **Run green:** `cargo test -p adam-web-ui --lib`.
- [ ] **Commit** bindings and Inspector redesign with updated public examples.

## Task 7: Standalone Dioxus consumer example and integration guide

**Files:** Create `adam-web-ui/examples/{sheet_binding.rs,sheet_binding.adm2}`,
`docs/dioxus-integration.md`; modify `adam-web-ui/Cargo.toml`,
`README.md`, `adam-lang-book/book-src/reference.md`.
**Depends on:** Tasks 3 and 6.

**Interfaces:**
- Add example target `sheet-binding`, path `examples/sheet_binding.rs`,
  `required-features = ["desktop"]`. Forward `adam-web-ui`'s existing desktop
  feature to `dioxus/desktop`; no renderer is enabled in the feature-free build.
- The example owns a `ModelFactory` over compiled modules and an explicit
  `ModulePath`/root selection, then calls `initialize_sheet`.
- Register an application scalar type using `TypeRegistry` plus its UI codec
  and one application operation in `OpLookup`.
- Use `EditNumber { bind: sheet.cell("width") }` and other explicit bindings.
  Demonstrate width/height/area direction changes, a conditional mode, a range
  filter retaining an aspiration, and a named requirement.

- [ ] **Write a public-API example test** named
  `demo_recording_replays_selected_root`. Use deterministic initial dimensions
  `width = 4`, `height = 3`, `area = 12`, then edit `area` to `24` and assert
  the selected producer yields `width = 8` when height is fixed.
  Record effective sources and replay into a fresh factory; assert the same
  outputs and zero playback-generated records. Pin the filter/requirement
  example's values in the same fixture and in the guide.
- [ ] **Run red:** `cargo test -p adam-web-ui --example sheet-binding --features desktop`.
- [ ] **Implement the example and guide.** Show all imports, dependencies,
  operation/type registration, named-root selection, host outcome handler,
  accepted input history, capture and fresh playback. Explain incomplete drafts,
  source classification versus writability, ownership and no reentrancy.
  The example reports recoverable outcomes, explicitly restores when needed,
  and on fatal failure attempts to write accepted recovery JSON before exiting
  nonzero; persistence errors are reported too. The file is
  `sheet-binding-recovery.json` in the launch directory; document it.
- [ ] **Verify and document the run command:**

  ```bash
  cargo run -p adam-web-ui --example sheet-binding --features desktop
  ```

  Render-verify live dependent updates, a rejected requirement, localized entry,
  recording/playback, and a deliberately injected runtime failure. Use a
  subprocess for termination checks; never terminate the test harness.
- [ ] **Commit** the runnable consumer path and guide links.

## Task 8: Begin policy boundary and graph failure containment

**Files:** Modify `begin/src/{app.rs,app_tests.rs,example_source.rs}`,
`begin/Cargo.toml`, its shell/graph/inspector CSS, and
`adam-web-ui/src/graph/{constraints.rs,bridge.rs,view.rs}`.
Create `begin/src/recovery.rs` and add it to the application modules.
Extend `adam-web-ui/tests/view_constraints.rs` and existing bridge tests.
**Depends on:** Task 6; use recording APIs after Task 3.

**Interfaces:**
- Begin owns `SheetHandle`, accepted source/configuration and `RecoveryState`;
  it no longer passes mutable ParsedSheet signals to UI components.
- `RecoveryDocument` serializes accepted source texts/root identity plus
  `RecoveryState`, not closures, `TypeId`s or live cell IDs.
- `persist_recovery(path: &Path, document: &RecoveryDocument) -> io::Result<()>`
  writes JSON through a same-directory `tempfile::NamedTempFile`, synchronizes
  the file and atomically persists it over the destination; serialization and
  IO failures remain explicit. Add optional `tempfile = "3"` and
  `directories = "6"` under the desktop feature for
  `ProjectDirs::from("org", "stlab", "begin").data_local_dir()/recovery.json`.
  Missing platform directories are an error, not a current-directory fallback.
- On the web, persist the accepted document under localStorage key
  `stlab.begin.recovery.v1`; report denied/quota errors and retire the runtime.
- Add required `GraphView` prop `on_error: EventHandler<GraphFailure>`.
  `GraphFailure` distinguishes invalid input/transport failures from
  `RuntimeFailure`; thread it through the bridge without formatting away cause.
- `ViewConstraints` holds its candidate in optional storage while mutating;
  after a mutating `Err` it is retired. A subsequent update reports unavailable,
  rather than reconstructing or reusing the failed sheet.

- [ ] **Write failing tests** for rejected source reload retaining accepted
  provenance and edits, false requirements restoring accepted history, fatal
  propagation attempting persistence exactly once, persistence failure still
  causing fatal shutdown, and stale opened-file responses. Track the accepted
  revision's durability and test that an already durable revision skips the
  redundant persistence attempt.
  Keep the shutdown decision in a pure function so tests can assert:

  ```rust
  assert_eq!(fatal_actions, [ReportFatal, PersistAccepted, Terminate]);
  assert_eq!(rebuild_attempts_after_fatal, 0);
  assert_eq!(displayed_source_after_rejected_reload, accepted_source);
  ```

  Add a graph failure fixture whose subsequent update returns unavailable
  without invoking its Sheet again. Preserve existing malformed-wire and
  stale-generation tests; invalid inputs rejected before mutation remain
  recoverable.
- [ ] **Run red:** `cargo test -p begin --no-default-features`,
  `cargo test -p begin`,
  and `cargo test -p adam-web-ui --test view_constraints`.
- [ ] **Implement the host boundary and migrate Begin controls.** A centralized
  outcome handler reports input/configuration errors, restores accepted state
  only for recoverable model rejection, and performs persistence plus desktop
  nonzero termination on fatal propagation. Web termination disables/unmounts
  the failed owner and shows a fatal diagnostic. Do not automatically reload a
  model around that failure. Replace all Spectrum shell and inspector widgets
  while retaining layout, file actions, example selection and zoom behavior.
  Update GraphView callers, including the book host, for the required callback.
  Surface a requested restoration's failure instead of retrying restoration
  recursively.
  Derive graph data through the handle's read-only accepted-model inspection,
  not a second mutable ParsedSheet signal. A `RecordingFailed` event reports
  the recording error without undoing an accepted model transaction.
- [ ] **Run green** with the same targeted tests, then render-verify desktop
  policy via subprocess and the web Begin build using `verifying-begin-ui`.
- [ ] **Commit** Begin integration and graph failure forwarding.

## Task 9: EZ Adam component-only migration

**Files:** Modify `ez-adam/src/ui/{app.rs,toolbar.rs,side_panel.rs}` and
directly affected existing UI tests and CSS.
**Depends on:** Task 5.

**Interfaces:** Use Task 5's base controls and `Theme`, never a `SheetHandle`.
Optional clamp bounds use `NumericEditor { optional: true, ... }`.
Formula and property text controls retain direct Document operations.

- [ ] **Write failing public UI tests** covering selection/toolbar actions,
  property edits, localized numeric bounds, clearing a bound to unset, and
  error presentation. Assert:

  ```rust
  assert_eq!(document_bound_after_clear, None);
  assert_eq!(serialized_document_after_blur, serialized_document_before_blur);
  assert_eq!(serialized_document_after_old_event, serialized_replacement_document);
  ```

  Preserve existing Document JSON round-trip tests rather than introducing a
  runtime recovery system.
- [ ] **Run red:** `cargo test -p ez-adam --no-default-features`.
- [ ] **Replace Spectrum wrappers and DOM-value reader callbacks** with base
  controls and native events. Preserve epoch ownership so an old panel cannot
  edit a replacement Document. Retire only guards that existed solely for
  asynchronous Spectrum DOM reads; keep genuine asynchronous owner checks.
- [ ] **Run green:** `cargo test -p ez-adam --no-default-features`.
  Render-verify side-panel editing, optional bounds and toolbar/canvas behavior
  with the migrated theme. Use the same rendered evidence standard as Begin.
- [ ] **Commit** the document-only component migration.

## Task 10: Book/slides live roots and asset migration

**Files:** Modify `adam-lang-book-live/src/lib.rs`,
`adam-lang-book/book-src/theme/{adam-live-bootstrap.js,inspector.css}`,
`adam-lang-book/book.toml`, `xtask/src/live_book_assets.rs`,
and the existing bootstrap, live-asset and slides publication tests.
**Depends on:** Tasks 6 and 8.

**Interfaces:** Each live root owns its own `SheetHandle` and `SheetEvent`
handler. Shared `Theme`/control CSS replaces `SpTheme` and `swc.js`.
`prepare_live_assets` stages the actual shared control assets along with
existing graph/WASM files. Graph-free roots do not load graph dependencies.

- [ ] **Write failing tests** for graph-free dependency lists without Spectrum,
  multiple independent roots with the same diagnostic name, paired graph
  associations, shared control CSS staging, and one root failing without
  retiring the other. Assert:

  ```javascript
  assert.equal(dependencies.some(item => item.url.endsWith("swc.js")), false);
  assert.equal(unaffectedRootIsInteractive, true);
  ```

  Retain the existing missing-input, malformed-manifest and destination-IO
  error checks, updating their asset fixtures.
- [ ] **Run red:** `cargo test -p xtask live_book_assets`,
  `cargo test -p adam-lang-book-live`, and the existing bootstrap Node tests.
- [ ] **Implement root policy and asset changes.** Build failures show typed
  diagnostics without a model. Recoverable rejection restores that root's
  accepted inputs; fatal propagation retires it, attempts browser persistence
  of accepted state and displays the failure. Keep the rest of the page usable.
  Remove Spectrum loading and replace the staged inspector/theme styles.
  Preserve D3, graph ownership and publication path resolution.
- [ ] **Run green** with the same tests and the existing slides tests, then
  build the actual embeds:

  ```bash
  (cd adam-lang-book-live && wasm-pack build --target web --release)
  cargo xtask prepare-live-book-assets
  mdbook build adam-lang-book
  ```

  Run these commands from the worktree root. Build slides through
  their existing package command. Render a graph-free embed, an inspector
  with a graph, and multiple independent embeds; check console/network failures.
- [ ] **Commit** migrated publishing hosts and staged-asset contracts.

## Task 11: Remove Spectrum infrastructure and complete the integration gate

**Files:** Remove `adam-web-ui/src/spectrum.rs`,
the obsolete `numeric_input.js` bridge and Spectrum-specific tests,
`begin/assets/swc.js`, `ez-adam/assets/swc.js`,
`begin/js/spectrum-entry.js`, `ez-adam/js/spectrum-entry.js`, and their
now-unused icon/bundling inputs after confirming consumers are migrated.
Modify application package manifests/locks, `xtask/src/main.rs`, active README
and setup instructions, and any directly dependent publication tests.
Create `docs/superpowers/2026-11-05-issue-272-handoff.md`.
**Depends on:** Tasks 7 through 10.

**Interfaces:** There is no Spectrum compatibility layer or Spectrum build
requirement. Keep non-Spectrum JavaScript test runners, graph/file scripts and
other vendored assets. Remove the obsolete `cargo xtask build-js` command
rather than preserving a success-shaped no-op.

- [ ] **Write/update failing infrastructure tests** that prove live asset
  staging no longer requires `swc.js`, current application markup contains no
  `sp-*` controls, and graph/file JavaScript tests remain runnable.
- [ ] **Run the targeted red commands** from the owning earlier task.
- [ ] **Remove obsolete infrastructure and update lockfiles** through the
  existing package managers. Remove only Spectrum/esbuild dependencies and
  commands that have no remaining consumers. Keep historical design documents
  unchanged; update active instructions that otherwise tell contributors to
  regenerate a deleted bundle. Verify no remaining production caller imports
  `spectrum`, `BuildOutcome`, or the old DOM-value bridge.
- [ ] **Run the complete Rust gate from the worktree root:**

  ```bash
  cargo fmt --all
  cargo build --workspace
  cargo test --workspace
  cargo test --doc --workspace
  cargo clippy --workspace --exclude begin --all-targets -- -D warnings
  cargo clippy -p begin --no-default-features --all-targets -- -D warnings
  cargo clippy -p begin --all-targets -- -D warnings
  cargo clippy -p ez-adam --no-default-features --all-targets -- -D warnings
  cargo clippy -p ez-adam --all-targets -- -D warnings
  RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace
  ```

  Plain build and test output must contain zero compiler warnings. Run the
  existing application JS suites, new locale tests, bootstrap/slides tests,
  publishing builds and documented example command. Use subprocess tests for
  fatal shutdown; do not broaden or repeat passing suites without a reason.
- [ ] **Complete rendered verification and final review.** Retain screenshots,
  DOM/console evidence, test commands and results in session artifacts. The
  committed handoff records implemented scope, explicit remaining boundaries,
  #279, and how to resume any deliberately unfinished work. Resolve in-scope
  review findings; open issues for substantive out-of-scope findings.
- [ ] **Commit** cleanup and handoff. Do not open a PR or seek merge approval
  until the requested local review and required CI gates are clear.

## Plan self-review and execution handoff

Spec coverage is assigned as follows: construction/error ownership (1, 2, 8),
binding interface/state
(2, 6), numeric editing/locale (4, 5), source recording (3, 7), component
migration (5, 8, 9, 10, 11), thread/reentrancy boundaries (2, 6, 7),
consumer guidance (7), and full verification (11). The five review-focus
classes have tests in their named tasks. Shared numeric/error types precede
their consumers, and the reactive binding task depends on both recording
and base-control interfaces.

The base optional numeric editor serves existing document properties; the
bound numeric widget remains required and never submits a blank as zero or
as an unset cell. Capture failures on unsupported/private sources remain
explicit; a host must supply a stable key and exact codec before recording.
These are implementation boundaries to document and test, not silent fallbacks.

The user selected subagent-driven execution. Fresh task reviews protect the
shared error and binding interfaces before the application migrations use them.
Execute one implementation task at a time in this worktree.
