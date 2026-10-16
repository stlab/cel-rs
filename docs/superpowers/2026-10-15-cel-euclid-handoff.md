# CEL Euclid implementation handoff

## Completed

The language stage is implemented and independently reviewed on
`worktree-euclid-library`, through commit `00111378`.

- Persistent shared operation registrations compose overloads across installers.
- Named receiver properties and calls preserve ordinary call and tuple semantics.
- Registered records support explicit fields in arbitrary written order.
- Predictive brace restrictions preserve CEL and embedded Adam control flow.
- Registration metadata informs checking, diagnostics, AST consumers, and books.
- Formatter fixes preserve closure receiver semantics and grouped comments.

Targeted suites, downstream checks, strict documentation, formatting, and
all-target lint checks passed for the affected language crates. The final
whole-workspace gate remains due after the library and graph stages.

The library stage is implemented and independently reviewed through `f5b48309`.
`cel-euclid` exposes the seven concrete geometry aliases, named records, the
documented operation table, checked primitives, and optional Adam registration.
Executable CEL-only crop/straighten and pan/zoom examples run in configured
web, LSP, and book environments. The crop preserves source aspiration and
checks fitting and fixed-overlay presentation precision explicitly.

A prerequisite adds registered closure parameter annotations and checked
general closure invocation, including recursive tuple intermediates.
Independent review also required rejecting ambient-dependent closure bodies.

The pure Adam graph view model is implemented and independently reviewed at
`2735e18962629b102c75365f372527c6c2ebf237`. Its sheet owns fit, zoom limits,
centering, and pan clamping; refresh preserves user source aspiration.
Task evidence records 126 crate tests and targeted lint, documentation, and
formatting checks.

The lifecycle-owned graph bridge is implemented and independently reviewed
through `633e786bf414978266c35f8eea6ec93723ceddca` (Task 2 implementation
`70c464fb`, followed by the retired-receive diagnostic guard).

Graph Task 3 now completes live control migration and shared book ownership:

- Every visible view transform comes from the cached Adam model, including
  initial Fit, wheel/pan, animated buttons/reset, and geometry refreshes.
- Reset obtains a fresh Fit target, renders approved intermediate frames, and
  preserves the final Fit aspiration. Refreshes and repeated target replies do
  not snap/interfere with the animation; newer user actions cancel waiting
  targets without rendering them.
- Book inspectors mount the shared `GraphView` channel against their existing
  external containers. Each graph has its own owner; failures appear at that
  graph's location without a Begin-only hook or a policy fallback.
- Empty-to-populated graphs, label/value changes, topology, visibility, resize,
  dragging, and unpinning refresh geometry without forced source resets.
- The unused JavaScript fit-policy helper and its old policy test are removed.
  The Spectrum bundle is unchanged.

### Fresh rendered evidence

Task 3 used isolated Edge CDP and native WebView2 remote debugging on Windows,
not a browser stand-in for desktop. Both real Dioxus evaluation transports
exercised the actual Spectrum controls and D3 wheel/drag events.

- Web: real viewport `856 × 859`; zoom-in produced 14 approved frames, 13
  during its transition. Reset produced 21 approved frames, 19 during its
  transition, ending exactly at the approved Fit.
- Native: real viewport `280 × 531`; zoom-in produced 13 approved frames, 12
  during its transition. Reset produced 20 approved frames, 18 during its
  transition, ending exactly at the approved Fit.
- Both hosts: 40 ms delayed replies still produced seven approved wheel frames
  during sustained input. Concurrent refresh/reset and cancellation of an
  80 ms delayed reset target passed. Node dragging pinned, double-click unpinned,
  source replacement rejected late old replies, and hiding inactive nodes
  reduced the conditional example's relationships from six to three.
- The rendered single-node web case reported fit/min/max
  `9.304347826086957`, confirming that fit can exceed the nominal maximum of 8.
- The release-built book mounted five simultaneous graph owners. Two were
  displayed together; editing the first inspector changed its graph to width
  `1234` without changing the second graph. Both made approved progress with
  delayed simultaneous wheel input. After a deliberate first-owner failure,
  the second owner rendered 14 additional approved zoom frames.
- Audits checked every observed transform write against its actual Rust
  approval and request geometry (`1e-6` pan tolerance, `1e-8` zoom-bound
  tolerance): zero approval/bounds violations. Final clean runs had no bridge
  or console errors. Intentional Rust diagnostics produced one visible graph
  alert and retired that graph, without a fallback.

Fresh final screenshots were opened and visually inspected:
`task-3-web-final.png`, `task-3-native-final.png`,
`task-3-book-two-owners.png`, and the corresponding final failure screenshots.
They live with exact commands, RED/GREEN logs, DOM/transform evidence, and
`task-3-report.md` under the git-ignored
`.superpowers\sdd\2026-10-15-cel-euclid-graph\` task workspace.
Book screenshots temporarily place two existing live containers side by side;
they do not represent a production layout change.

Scoped Rust/JavaScript tests, both Begin Clippy modes, affected host/Wasm lint,
strict affected-library documentation, and formatting passed. Task 3 received
direct self-review only; no nested agent or independent reviewer ran.

## Remaining

The controller still owns the final full master-workspace gate and independent
whole-branch review. Task 3 deliberately does not duplicate that stage or claim
PR readiness. There is no known parked in-scope finding and no desktop/web/book
platform blocker. No push, PR, or merge is requested.

The approved design and three stage plans remain authoritative. No
crop-specific constraint function or Rust fitting helper is permitted: the
example must compose general geometry operations in CEL.

## Implementation rulings

Function markers require `Send + Sync` to preserve scope thread-safety.
If unnecessarily strict, this restricts unusual host marker types; the planned
markers are zero-sized and satisfy the bounds.

Field-name validation precedes constructor emission, not streaming compilation
of field expressions. Invalid parsing discards its segment; the cost is
instructions appended to a segment that is subsequently discarded.

The approved crop composition requires typed immediately invoked closures.
Implementing their missing general-purpose runtime support was necessary
before the example could proceed. If that scope decision is wrong, it adds
broader closure/runtime work than originally anticipated; the prerequisite
and its safety fix received independent review.

## Recovery

### Offline pause (historical; resolved by Tasks 2 and 3 above)

The user requested a pause to disconnect from the internet. Local inspection
confirmed HEAD remains `2735e189` on `worktree-euclid-library`. Graph Task 2
has interrupted, uncommitted work in these files:

- `adam-web-ui\src\graph\mod.rs`
- `adam-web-ui\src\graph\view.rs`
- `begin\assets\graph.js`
- `begin\js\graph.test.js`
- `adam-web-ui\src\graph\bridge.rs` (untracked)
- `adam-web-ui\src\graph\channel.js` (untracked)
- `begin\js\graph-channel.test.js` (untracked)
- `begin\js\graph-protocol.json` (untracked)

All partial files are preserved in place. No Task 2 report exists at
`.superpowers\sdd\2026-10-15-cel-euclid-graph\task-2-report.md`.
Do not assume these changes compile, pass tests, or have received review.
The pause inspection found no running task agents or active shell commands.
No network calls, push, pull request, or new commit accompanied this pause.

On resumption, inspect the partial implementation and Graph Task 2 brief
before editing or dispatching more implementation work. Finish and review
Task 2, then execute Task 3 with rendered desktop and browser/book checks.
In particular, preserve the bounded one-in-flight/latest-pending queue,
generation/revision/sequence handling, obsolete-revision queue release,
reset preservation, and animation-safe D3 synchronization.

Task briefs, evidence, and review ledgers live in git-ignored
`.superpowers\sdd\` stage directories. Resume the first task without a completion
entry rather than repeating committed work. No push or pull request is requested.
