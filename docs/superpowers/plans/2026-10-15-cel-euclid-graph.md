# Adam-Constrained Live Graph Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make an Adam/Euclid sheet authoritative for live graph fit, minimum zoom and pan bounds without regressing gestures or rendering.

**Architecture:** Keep D3 layout, bounds measurement and gesture proposals in JavaScript. Each shared `GraphView` owns a parsed Adam constraint model and a lifecycle-owned bidirectional evaluation channel. Only model-approved views reach the zoom layer.

**Tech Stack:** Rust 2024, Adam, `cel-euclid/adam`, Dioxus desktop/web, existing vendored D3, Serde, Vitest, headless Edge through `verifying-begin-ui`.

**Spec:** `docs\superpowers\specs\2026-10-15-cel-euclid-design.md`; prerequisite: `2026-10-15-cel-euclid-library.md`.

## Global Constraints

- The bridge must run on desktop and web, including book-hosted graphs.
- Never render an unconstrained proposed view as though the model had approved it.
- Minimum zoom is fit scale; maximum is `max(fit_scale, 8)`.
- First render waits for a real nonzero viewport.
- Updating D3's internal transform must not recursively resubmit the accepted view.
- Avoid per-gesture reparsing or rebuilding the sheet.
- No Spectrum bundle regeneration unless bundle inputs actually change.

## Review Focus

- Content narrower on only one axis must center on that axis and clamp on the other.
- Bounds-only updates must preserve Adam's original pan/zoom aspiration, not write back its previous constrained display value.
- Reset and +/- animations must survive accepted replies without `zoom.transform` interrupting their transitions.
- A disposed/replaced graph's reply must never affect its replacement with the same container id.
- Bursty input must render approved progress during interaction with bounded queue size, not freeze until the gesture ends.

---

## Interfaces

In `adam-web-ui\src\graph\constraints.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ViewTransform { pub x: f64, pub y: f64, pub k: f64 }
pub enum ViewAction { Propose { view: ViewTransform }, Refresh, Fit }
pub struct ViewInput {
    pub bounds: cel_euclid::Rect,
    pub viewport: cel_euclid::Size2D,
    pub action: ViewAction,
}
pub struct ViewOutput {
    pub view: ViewTransform,
    pub fit: ViewTransform,
    pub min_zoom: f64,
    pub max_zoom: f64,
}
pub struct ViewConstraints { /* parsed sheet and cached cell IDs */ }
impl ViewConstraints {
    pub fn new() -> anyhow::Result<Self>;
    pub fn update(&mut self, input: ViewInput) -> anyhow::Result<ViewOutput>;
}
```

`Refresh` writes bounds/viewport only. `Propose` additionally writes the user's
new source pan/zoom. `Fit` propagates current bounds, copies the sheet's derived
fit values into pan/zoom source cells, and propagates again. It does not
calculate fit policy in Rust. Cache the sheet and cell IDs across requests.

In `adam-web-ui\src\graph\bridge.rs`, use tagged Serde messages:

```rust
pub struct ViewRequest {
    pub generation: u32,
    pub revision: u32,
    pub sequence: u32,
    pub bounds: [f64; 4], // min_x, min_y, max_x, max_y
    pub viewport: [f64; 2], // width, height
    pub action: ViewAction,
}
pub enum ViewMessage {
    Start { generation: u32 },
    Update { request: ViewRequest },
    Dispose { generation: u32 },
}
pub enum ViewReply {
    Accepted { generation: u32, revision: u32, sequence: u32, output: ViewOutput },
    Failed { generation: u32, revision: u32, sequence: u32, message: String },
}
```

Derive Serde on actions/output and use `#[serde(tag = "kind",
rename_all = "snake_case")]` for enums, with named variant fields in the wire
representation. The Rust `Propose { view: ViewTransform }` action is encoded
as `{ "kind": "propose", "view": { "x": ..., "y": ..., "k": ... } }`.
Generation and sequence counters use checked increment with an explicit
error on exhaustion; JS validates the `u32` range.

A channel begins with a typed `Start { generation }` lifecycle message;
`Dispose { generation }` closes that generation. Unknown/stale generations
cannot implicitly create a model. A container's generation number increases
across JS instance replacement during the mounted channel's lifetime.
The channel is container-scoped; another container's messages never enter it.

Implement a pure `BridgeState` that owns the active generation, model and
last processed sequence, with
`start(generation: u32) -> anyhow::Result<()>`,
`handle(request: ViewRequest) -> anyhow::Result<Option<ViewReply>>`, and
`dispose(generation: u32)`. Stale work yields `None` as a documented protocol
case; malformed current messages and propagation failures produce visible
errors, not success-shaped replies.

### Task 1: Executable Adam view constraints and pure model owner

**Prerequisites:** Completed library plan.

**Files:** Create `adam-web-ui\src\graph\view_constraints.adm2`,
`adam-web-ui\src\graph\constraints.rs`,
`adam-web-ui\tests\view_constraints.rs`; modify
`adam-web-ui\src\graph\mod.rs`.

**Interfaces:** Produces `ViewTransform`, `ViewAction`, `ViewInput`,
`ViewOutput`, `ViewConstraints` above.

- [ ] **Step 1: Add contract tests** `fit_centers_content`,
  `pan_centers_short_axis_and_clamps_long_axis`,
  `fit_can_exceed_nominal_zoom_limit`, `zero_extent_uses_one_unit`,
  `refresh_preserves_source_aspiration`, and
  `invalid_view_input_is_reported`.
  Bounds `[0,200] x [0,100]` in viewport `(400,300)` fit to
  `{ x: 0, y: 50, k: 2 }`. At `k=3`, pan limits are `x in [-200,0]`,
  `y=0`. Bounds `(10,10)` in viewport `(400,300)` yield minimum/maximum
  zoom `30`, not an invalid `[30,8]` range.
- [ ] **Step 2: Run** `cargo test -p adam-web-ui --test view_constraints`.
  Expected: missing model/module.
- [ ] **Step 3: Write `view_constraints.adm2`.** Use source `bounds: Rect`,
  `viewport: Size2D`, `max_zoom: f64 = 8.0`; ordinary cells
  `pan: Vector2D`, `zoom: f64`; derived outputs
  `fit_zoom`, `fit_pan`, `min_zoom`, `effective_max_zoom`, and
  `view: Transform2D`.
  Fit uses extents bounded below by `1.0`. Self-reference `zoom` through
  `clamp(zoom, min_zoom, effective_max_zoom)`. Self-reference `pan` through
  a `Vector2D` literal whose axes either center smaller content or clamp to
  `[viewport_extent - zoom * bounds_max, -zoom * bounds_min]`.
  Derive `view = scale(zoom, zoom).then(translation(pan))`.
  Declare relationships in a dependency-safe order without relying on order
  as solver behavior.
- [ ] **Step 4: Implement `ViewConstraints`.** Use the library stage's
  `build::type_registry()` and `build::op_lookup()`; parse the embedded sheet
  once and cache IDs. Validate finite ordered bounds with nonnegative
  representable extents, positive finite viewport, and positive finite
  proposed zoom. Validate finite model outputs and surface formatted
  sheet errors. `Fit` copies derived fit outputs; `Refresh` never writes
  an effective pan/zoom back as a new source.
- [ ] **Step 5: Pin spring-back**: propose an excessive pan at a valid zoom,
  shrink bounds to clamp it, expand bounds, and assert the original proposal
  returns when feasible. Assert `Fit` intentionally replaces that aspiration
  and subsequent refreshes do not restore the pre-reset source.
- [ ] **Step 6: Run** `cargo test -p adam-web-ui --test view_constraints`;
  `cargo clippy -p adam-web-ui --all-targets -- -D warnings`.
  Expected: all pass with the actual sheet, not a Rust duplicate of its math.
- [ ] **Step 7: Format and commit**:
  `feat(graph): model view constraints in Adam`.

### Task 2: Lifecycle-safe proposal and approval bridge

**Prerequisites:** Task 1.

**Files:** Create `adam-web-ui\src\graph\bridge.rs`;
modify `adam-web-ui\src\graph\{mod,view}.rs`,
`begin\assets\graph.js`, `begin\js\graph.test.js`.

**Interfaces:** Produces `ViewRequest`, `ViewReply`, `ViewMessage` and
`BridgeState` above; JS `GraphInstance` gains proposal submission,
approved-view application and subscription disposal. Preserve existing
`window.beginGraph` driver method names.

- [ ] **Step 1: Add Rust protocol tests**
  `stale_generation_is_ignored`, `disposed_generation_cannot_restart`,
  `stale_sequence_does_not_mutate_model`, `current_failure_is_a_failed_reply`,
  `message_numbers_are_validated`, and `two_containers_are_independent`.
  Serialize/deserialize real request/reply fixtures and assert field names,
  actions and ordering exactly match the JS fixtures.
- [ ] **Step 2: Add Vitest cases**
  `proposed_view_is_not_rendered_before_approval`,
  `bursts_keep_one_in_flight_and_one_pending_snapshot`,
  `old_revision_reply_does_not_apply`, `approval_does_not_resubmit`,
  `old_revision_reply_releases_inflight_and_pumps_latest`,
  `refresh_does_not_drop_pending_fit`, and
  `destroyed_instance_ignores_late_reply`.
  Drive 100 proposals against a delayed fake channel: assert queue size
  remains two, a valid completed reply renders before the burst finishes,
  and the eventual view corresponds to the latest proposal.
- [ ] **Step 3: Run** `cargo test -p adam-web-ui graph::bridge`;
  from `begin`, `npm test -- graph.test.js`.
  Expected: missing bridge/adapter failures.
- [ ] **Step 4: Implement pure protocol state and JS coalescing.**
  Process one request at a time. Retain one latest pending snapshot; merge
  newer bounds into pending actions. A bounds refresh cannot erase a queued
  reset. A genuinely newer user gesture may intentionally cancel an ongoing
  reset, matching existing D3 behavior. Reject older geometry revisions and
  already-applied sequences. A reply matching the outstanding request clears
  its in-flight slot and pumps the latest pending snapshot even when its old
  geometry revision prevents rendering; otherwise a resize can deadlock the
  queue. An unrelated old-generation reply cannot clear a new request's slot.
  A newer merely pending gesture does not
  invalidate the current completed reply: render approved progress while
  input continues.
- [ ] **Step 5: Own the channel in `GraphView`.** Install the evaluation
  listener before JS initialization. JS uses `dioxus.send` and the matching
  `dioxus.recv` callback to exchange typed messages. Start/dispose a model
  with each graph instance, and cancel subscriptions/tasks on unmount,
  container change or source replacement. Keep branch decisions in
  `BridgeState`/tested pure helpers rather than untested framework callbacks.
  Report errors via `crate::diagnostics::report_error` and a visible graph
  error state; do not restore JS policy silently.
- [ ] **Step 6: Separate proposal and rendering paths in D3.**
  Gestures submit absolute proposed transforms; only accepted views set
  `.zoom-layer` transforms. Apply acknowledged views under a guard so
  programmatic synchronization cannot emit another request. During a D3
  transition, render approved progress but defer `zoom.transform` baseline
  synchronization until the transition ends: calling it mid-transition
  interrupts that transition. Outside transitions, synchronize accepted
  baselines under the guard. Feed model-provided zoom limits back as gesture
  hints, not independently calculated policy.
- [ ] **Step 7: Run** the Rust protocol/model tests and the complete
  `begin` Vitest suite. Expected: no stale applications, recursive message
  loops, cross-instance updates, lost terminal request, or unbounded queues.
- [ ] **Step 8: Format and commit**:
  `feat(graph): bridge view proposals to Adam approvals`.

### Task 3: Migrate every live control and verify desktop/web rendering

**Prerequisites:** Tasks 1 and 2.

**Files:** Modify `begin\assets\graph.js`, `begin\js\graph.test.js`,
`adam-web-ui\src\graph\view.rs`, directly affected graph host tests;
modify `begin\src\app.rs` and `adam-lang-book-live\src\lib.rs` only if
their existing `GraphView` use needs a lifecycle/error prop.
Update `docs\VISION.md`, relevant graph documentation, and
`docs\superpowers\2026-10-15-cel-euclid-handoff.md`.

**Interfaces:** Retains `beginGraph.init/update/zoomIn/zoomOut/resetZoom`
and multi-instance container IDs. Every rendering path consumes the bridge
approval, including initial fit and animated controls.

- [ ] **Step 1: Add migration tests**
  `initial_fit_waits_for_real_viewport_and_approval`,
  `resize_and_visibility_refresh_constraints`,
  `structural_change_preserves_current_view_when_valid`,
  `reset_animation_is_not_interrupted_by_approvals`,
  `zoom_buttons_and_wheel_use_the_same_model`, and
  `late_source_reply_cannot_move_replacement_graph`.
  Use fake transition timing and delayed model replies; assert multiple
  approved intermediate frames and the final fit rather than merely a
  queued transition call.
- [ ] **Step 2: Run** from `begin`, `npm test -- graph.test.js`.
  Expected: old direct-fit/constraint paths fail the new approval assertions.
- [ ] **Step 3: Route all view changes through the model.** Keep `computeBBox`
  and layout measurement in JS. Replace `fitTransformFor` and
  `updateZoomConstraints` arithmetic with requests. Remove the old
  `scaleExtent`/`translateExtent` policy calculation and default D3
  conformance dependency. Start/reset obtain their target from `Fit`.
  First fit displays immediately on approval; reset animates toward the
  approved target, submitting intermediate proposals for approval.
  Zoom +/- retain `1.3` and `1/1.3`, `200 ms`; reset retains `300 ms`.
  Bounds changes preserve the current aspiration unless a user explicitly
  resets or makes a new gesture.
- [ ] **Step 4: Keep shared hosts consistent.** Verify direct book graphs
  also install the channel; no Begin-only global solver hook. Preserve
  node dragging and inactive-node filtering. Show a useful failure in the
  graph container if its model cannot start or propagate; keep diagnostics
  visible on both browser and native paths.
- [ ] **Step 5: Run** `cargo test -p adam-web-ui -p adam-lang-book-live`;
  from `begin`, `npm test`;
  `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`;
  `cargo clippy -p begin --all-targets -- -D warnings`.
  Expected: all pass, with no remaining production JS copy of the policy.
- [ ] **Step 6: Invoke `verifying-begin-ui` and perform rendered checks.**
  Inspect screenshots, DOM and live zoom transforms for initial fit,
  sustained wheel/drag, zoom bounds, all buttons and animations, viewport
  resize, graph changes, inactive-node hiding, source replacement and two
  simultaneous book graphs. Delay bridge replies in a test harness and
  observe approved frames during the gesture; assert no out-of-bounds
  rendered frame beyond numerical tolerance. Verify no console/bridge
  errors and that a deliberate error is surfaced rather than hidden.
  Exercise the desktop evaluation transport too; web rendering alone does
  not prove desktop messages work.
- [ ] **Step 7: Run the master plan's full gate**, document actual rendered
  evidence and any platform blockers in the dated handoff, review the whole
  branch, fix in-scope findings, and commit:
  `feat(graph): enforce live pan and zoom through Adam`.

No PR or merge is part of executing this task unless the user requests it.
