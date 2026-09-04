# Dynamic live adam-lang-book graphs — handoff

Status snapshot as of 2026-09-03, written after Task 5 (end-to-end verification) on
`worktree-adam-lang-book/add-graphs`. Read
`docs/superpowers/specs/2026-09-03-dynamic-live-adam-book-graphs-design.md` first for the full
design and non-goals — this doc only summarizes what shipped, what's deliberately out of scope,
and the verification status.

## What shipped

**Task 1 (commits `8da005f..926943a`).** Extracted `graph_drive_script(container_id: &str, data:
&GraphData, call: &str) -> String` in `adam-web-ui::graph` — the single place that builds the JS
seeding `window.__beginGraphData[id]` and invoking `window.beginGraph.init`/`update`.
`GraphView`'s effect and `onmounted` handler now call it instead of an inline `format!`; no
behavior change for `begin`. Contract-tested for the `init`/`update` calls and for embedding
`container_id`/JSON.

**Task 2 (commits `926943a..aa5b942c`).** `adam-lang-book-preprocessor`'s `<graph sheet="name">`
pass gained a paired-include check: alongside its existing file-existence check, it now verifies
the same chapter's raw content also contains `{{#include ...<chapter>/<name>.adm2}}` — the
inspector mount that will own the sheet. A `<graph>` with no paired include makes `run()` return
`Err`, failing the `mdbook build`, exactly like the existing bad-file-reference case. 16 unit
tests cover both the failing (no include) and passing paths alongside the existing tag-rewrite
tests.

**Task 3 (commits `aa5b942c..9cbbe5ac`).** `adam-lang-book-live` now owns the sheet and drives the
graph itself: `mount_graph`, `GraphRootProps`, and `GraphRoot` are deleted. `mount(element_id,
source, name, graph_ids: Vec<String>)` gained the `graph_ids` parameter. The inspector `Root`
component's effect mirrors `begin`'s `App`: on every `sheet` change it recomputes
`to_graph_data(&sheet.read(), &labels.read())` once, serializes it, and drives every id in
`graph_ids` via `graph_drive_script` — `init` on the first run (guarded by a `use_signal<bool>`),
`update` on every later run. A parse failure still renders the diagnostic `<pre>` and drives no
graph.

**Task 4 (commits `9cbbe5ac..eb642350`).** `adam-live-bootstrap.js`'s mount loop was reordered:
it first assigns each `.adam-live-graph` div's container id and builds a `data-example → [id…]`
map, then mounts each `.adam-live` inspector with that example's graph ids (empty when none). The
separate `mountGraph` pass is gone; graph divs are now plain containers the inspector mount
drives directly (the div's own id, no `-container` child).

Net effect: a `<graph sheet="name">` on a book page is driven entirely by the inspector mount for
the same resolved example (`data-example`, e.g. `tutorial/first_sheet`) on the same page — one
`Sheet`, matching `begin`'s single-`Signal<Sheet>` model, restored across the book's DOM gap
described in the design doc.

## Deliberately left out (spec non-goals, unchanged)

- **Editing a sheet's cells by clicking graph nodes.** The graph stays display-only; values change
  only through the inspector's widgets. No JS-to-wasm write-back seam was introduced.
- **Structural edits** (adding/removing cells or relationships) from a live page — that needs a
  re-parse and stays out of scope; only values and the derived plan change live.
- **Cross-page or cross-chapter graph/inspector pairing.** A `<graph>` binds only to an inspector
  on the same chapter page, consistent with the prior pass's bare-name-resolved-to-current-chapter
  model.

## Verification status

### Full check suite — all clean, zero warnings

Run from the worktree root:

- `cargo build --workspace` — clean, no warnings.
- `cargo test --workspace` — all crates pass (unit + doc tests interleaved in this invocation;
  e.g. `adam-rs` 367, `adam-lang` 233/85/90/20 across its suites, `cel-parser` 152, `cel-runtime`
  365, plus every doc-test crate at 0 failures).
- `cargo test --doc --workspace` — run separately per the brief; all doc tests pass (same set as
  above).
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` — clean.
- `cargo clippy -p begin --no-default-features --all-targets -- -D warnings` — clean.
- `cargo clippy -p begin --all-targets -- -D warnings` — clean.

No plain-build warnings beyond what clippy already reports (checked the raw build/test output,
not just clippy's exit code, per repo convention).

### `begin` — verified via the `verifying-begin-ui` skill (headless Edge + raw CDP)

Served `begin` as a web app (`dx serve --platform web --no-default-features --features web`) and
drove it with headless Edge over the DevTools Protocol. One environment wrinkle worth recording
for the next session: this machine's Edge is enterprise-managed, and a headless tab that isn't
brought to the front has its `requestAnimationFrame`/D3-transition timers throttled to nearly
nothing — a zoom-button click or `window.beginGraph.zoomIn` call would appear to do nothing until
`Page.bringToFront` was called on that specific CDP target. Once foregrounded, everything below
confirmed working:

- Graph renders for the `diamond` example (screenshot confirmed: cell/relationship nodes, legend,
  correct values).
- **Fit**: clicking the button re-fit the zoom transform (scale snapped to the fit value).
- **Zoom in/out**: `window.beginGraph.zoomIn`/the toolbar button change the D3 zoom scale.
- **Show inactive**: toggling it on `toy_example` (which has a `conditional` with an inactive
  branch) changed the rendered node count from 13 to 16, confirming inactive nodes are actually
  hidden/shown, not just dimmed.
- **Switching examples resets cleanly**: `diamond` → `toy_example` rebuilt the graph (node count
  7 → 13, cell values changed to match the new sheet, a fresh fit was applied) with no stale nodes
  left over from the old example.

This confirms the `graph_drive_script` extraction (Task 1) is a true no-behavior-change refactor.

### Book end-to-end — verified live, not just a successful build

Tooling used: `wasm-pack` 0.15.0, `mdbook` 0.5.4, `cargo install --path
adam-lang-book-preprocessor` (reinstalled fresh so the mdbook preprocessor binary reflects
Task 2's current source, not a stale earlier install), `cargo run -p xtask --
prepare-live-book-assets`, all matching `.github/workflows/docs.yml`'s canonical steps. All
tooling was present in this environment — no tool-missing gaps to report for this step.

- `wasm-pack build --target web --release` (from `adam-lang-book-live/`) — succeeded.
- `cargo run -p xtask -- prepare-live-book-assets` — succeeded, copied the wasm bundle and JS/CSS
  assets into `adam-lang-book/book-src/theme`.
- `mdbook build adam-lang-book` — succeeded. This exercises Task 2's paired-include guard: every
  `<graph sheet="...">` currently in the book has a matching `{{#include}}` in the same chapter,
  or this step would have failed with a named error.
- Served `book-dist/` on a plain static server and drove the tutorial chapter with headless
  Edge/CDP (same `Page.bringToFront` caveat as above applied here too).
- **§1.1 `first_sheet` live update**: clicked the `width` inspector widget's step-up spinner
  (real synthetic mouse clicks, not `.click()` — `.click()` worked for `begin`'s own
  `sp-action-button`/`sp-sidenav-item` once foregrounded, but real coordinate-based clicks were
  used throughout the book verification for consistency). The field went 1920 → 1921 → 1930, and
  the graph's `width` node text tracked it exactly at each step; `height` stayed unchanged at
  1080. Screenshots taken before and after confirm this visually.
  - Caveat: `first_sheet` (`sheet hello { source width; source height; }`) has no relationships,
    so arrow direction / active branch / forced cells / forced relationships have nothing to
    exercise on this specific page. That derived state was confirmed instead through `begin`'s
    `toy_example` (a real `conditional`), via the identical `graph.js`/`to_graph_data` code path
    the book shares with `begin` — not observed directly on a book page. The changed-cell pulse
    animation itself (a 200 ms/400 ms fill transition) wasn't captured mid-transition by a
    screenshot; the value-update pipeline that drives it was confirmed by the exact value match
    above.
- **Two graphs on one page stay independent**: temporarily added `<graph sheet="clamp_demo">`
  right after its existing (already-paired) `{{#include}}` in `tutorial.md`, rebuilt, and
  reloaded. Confirmed:
  - Editing `first_sheet`'s `width` (1920 → 1923) changed only its own graph; `clamp_demo`'s graph
    stayed at `level = 50`.
  - Editing `clamp_demo`'s `level` (50 → 55) changed only its own graph; `first_sheet`'s graph
    stayed at `width = 1923`.
  - Dragging the `width` node in `first_sheet`'s graph moved its position (435.6, 152.6 → 481.0,
    199.7) while `clamp_demo`'s single node stayed at exactly (300.5, 222) — unmoved.
  - Reverted the temporary `<graph sheet="clamp_demo">` line from `tutorial.md` and rebuilt;
    `git status` confirms `tutorial.md` is back to matching `HEAD` (no diff left behind).

### Not independently re-verified

- The paired-include guard's *failure* path (a `<graph>` with no include) was not re-triggered as
  a live negative test in this session — it's already covered by Task 2's own unit tests (16/16
  passing, including that case), and a real `mdbook build` succeeding here already confirms every
  `<graph>` currently in the book satisfies the guard.

## Follow-ups discovered (non-blocking)

- **Headless-Edge background-tab throttling.** Worth folding into the `verifying-begin-ui` skill's
  gotchas: on this (enterprise-managed) Edge install, a headless CDP target that isn't the
  frontmost tab has its rAF/D3-transition timers throttled almost to a halt, making zoom/fit
  interactions silently appear to do nothing until `Page.bringToFront` is called on that target.
  Not a code defect — a tooling note for whoever runs this skill next.
- **Pre-existing untracked file**, unrelated to this feature: `begin/examples/
  aaa_two_disconnected_sources.adm2` was already present (untracked) in this worktree before this
  session's verification work started. Left untouched — it isn't part of this task's scope and
  wasn't committed.
- **Concurrent unrelated edit observed mid-session**: `begin/assets/graph.js` picked up an
  uncommitted change (a `CENTER_PULL_STRENGTH` `forceX`/`forceY` addition to keep disconnected
  nodes from drifting outward — evidently in-progress work paired with the untracked
  `aaa_two_disconnected_sources.adm2` example above) partway through this verification session,
  from outside this task. It doesn't touch anything Task 1–4 changed and doesn't affect what this
  handoff verifies, but both the `begin` and book verification runs above executed against that
  modified `graph.js` (it's a static asset, not part of the Rust build the check suite covers).
  Left as-is, unstaged, and not committed — not this task's change to make.
- A leftover `mdbook serve .` process (bound to port 3000) was already running in this environment
  before this session began; left untouched since it predates this task and stopping someone
  else's dev server wasn't in scope.

No code defects were found. All six check-suite commands, the `begin` re-verification, and the
book end-to-end walkthrough came back clean.
