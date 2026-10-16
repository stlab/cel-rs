# Graph view ownership and verification

## One policy, shared hosts

`adam-web-ui` owns the graph-view policy in
`src/graph/view_constraints.adm2`. The sheet composes general Euclid and scalar
operations to calculate fit, minimum zoom (`fit`), maximum zoom (`max(fit, 8)`),
short-axis centering, and long-axis pan clamping. No specialized fitting helper
or JavaScript copy of this policy participates in rendering.

`begin/assets/graph.js` retains D3 layout, node measurement, dragging, visible-node
bounds, and gesture/animation mechanics. It submits complete geometry snapshots
and absolute proposals, then renders only matching Adam approvals. It overrides
D3's default conformance function; model-provided scale limits are gesture hints,
not approval to draw an unvalidated candidate.

`GraphView` installs its bidirectional evaluation listener before initializing
the driver. Each container owns a cached model and a generation-scoped queue:
one in-flight snapshot and one latest pending snapshot. Sequence and geometry
revision checks reject stale results without stranding the queue. Source changes,
container replacement, and unmount retire the old subscription and receiver.

Begin's desktop WebView2 and web builds use this component directly. The book's
inspector mounts one `GraphView` with `external_container: true` for each existing
chapter graph container. The inspector keeps its sheet; every graph keeps an
independent channel and model. No Begin-only solver global is required.

## Interaction contracts

- Initial construction waits for a real nonzero viewport. Content stays hidden
  until the first accepted Fit, which displays immediately.
- Resize, simulation ticks, pin/unpin, topology changes, label/value changes, and
  inactive-node visibility changes refresh geometry without replacing the user's
  source aspiration. An empty graph gaining nodes is not a new lifecycle.
- Zoom-in and zoom-out propose `1.3` and its reciprocal over `200 ms`.
- Reset requests a fresh Fit target, then proposes intermediate frames over
  `300 ms`. A completed reset submits a terminal Fit to retain fit aspiration.
  Repeated target acknowledgements or refreshes cannot snap the animation.
  An explicit reset replaces an older animation; a newer gesture/button cancels
  a waiting reset target without displaying that cancelled target.
- Approvals update the visible layer during gestures and transitions. Guarded
  D3 baseline synchronization waits until the gesture ends, preventing feedback
  and approval-driven transition interruption.
- Model/transport failures stop the graph and display a useful `role="alert"`.
  Owned containers render that diagnostic through Dioxus; external book
  containers also receive a safely text-encoded diagnostic at their location.
  A replacement clears only its predecessor's external diagnostic.

The public driver retains `init`, `update`, `zoomIn`, `zoomOut`, `resetZoom`,
`setShowInactive`, and `destroy`, keyed by container id.

## Verification

Run `cargo test -p adam-web-ui -p adam-lang-book-live` and, from `begin`,
`npm test`. The JavaScript tests cover delayed replies, bounded coalescing,
replacement, real data-update/filter paths, and deterministic approved animation
frames. Rust tests cover the model, wire protocol, lifecycle, and external host
DOM identity.

The 2026-10-15 graph task also exercises real Begin web and desktop WebView2
evaluation transports, plus multiple simultaneous release-built book owners.
Its fresh DOM/transform audits, screenshots opened for inspection, exact commands,
and any environment observations live in the local graph task workspace:
`.superpowers/sdd/2026-10-15-cel-euclid-graph/task-3-report.md`.
The [dated handoff](superpowers/2026-10-15-cel-euclid-handoff.md) separates this
rendered verification from the completed full workspace gate and final fix
wave. Controller approval of the final scoped fixes remains separate.
