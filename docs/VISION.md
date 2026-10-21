# cel-rs Workspace Vision

This is a standing reference for Claude Code, not primarily for human readers. It captures
the _why_ and _where this is headed_ behind the workspace's components — complement it with
`CLAUDE.md` (build mechanics, code style) and `docs/superpowers/specs/` (dated, per-feature
designs). Unlike those specs, this file has no date or status: edit it in place as direction
changes rather than superseding it with a new dated copy.

Each section's "directions being explored" list is deliberately unordered across crates —
work generally proceeds on several components in parallel across separate git worktrees, so
a single global sequence would misrepresent how decisions actually get made. Within a
section, list order does hint at nearer-term vs. later interest where that's known.

## Overview

The workspace exists to support hot-loaded, optionally-compiled domain-specific languages,
in the style of Dioxus's developer experience — and its first serious application is
expressing multi-way UI constraints so a bound visual presentation becomes a functioning UI
without hand-written event logic.

Dependency flow: `cel-parser`/`cel-runtime` → `adam-rs` → `adam-lang` → `begin`.
`adam-lang` is currently the only client of `cel-parser`/`cel-runtime`; more are expected over
time as the DSL-hosting story matures.

## cel-runtime / cel-parser

**Mission:** `cel-runtime` is a stack-based expression evaluator; `cel-parser` is a
recursive-descent parser for CEL (Common Expression Language) that follows Rust syntax and
conventions, built on the proc-macro infrastructure. Together they let a DSL be interpreted
by default via `DynSegment` — giving fast dev-time iteration, analogous to Dioxus RSX's
hot-reload — with an opt-in path to compile the same DSL surface syntax down to native Rust
for release/performance. That second path does not yet exist.

**Current state:** interpreted backend (`DynSegment`/`Segment`) is the only backend.
`cel-rs-macros` currently does compile-time validation of CEL expressions only.
Tuples, arrays, registered records/members, and signature-typed noncapturing
callables compose recursively. CEL arrays own concrete contiguous `Array<T>`
elements; tuples use source-ordered canonical lists (`Nil`/`Cons` witnesses),
including unit and singleton tuples. `[[T]]` maps to `Array<Array<T>>`, whose
allocation-preserving vector view is `Vec<Array<T>>`, not `Vec<Vec<T>>`.
`Callable<Args, R>` / `FnOf` retain complete signatures for native and interpreted
bodies, including typed empty callable arrays. Clone, equality, and debug
formatting are independent capabilities, not universal storage requirements.
`ValueType`, `ResolvedType`, and checker `Ty` carry complete recursive shapes.
External erased ownership checks those shapes; `PreparedSegment` and
`prepared_segment::PreparedRawSegment` validate frames once and execute the
existing raw engine without repeating type checks. Native Rust operations share
typed adapters with interpreted CEL; this does not implement the future macro
backend. See the [collections tutorial](../cel-lang-book/book-src/collections.md)
and [implementation handoff](superpowers/2026-10-19-composable-containers-handoff.md).

The optional `cel-euclid` library exposes
seven concrete `f64` / `UnknownUnit` geometry types and general affine operations,
including checked inverse and four-corner outer bounds. Hosts explicitly opt in;
the core languages do not depend on geometry. See
[`cel-euclid/SIGNATURES.md`](../cel-euclid/SIGNATURES.md) for the supported boundary.

**Directions being explored:**

- The second parser backend: compiling CEL directly to Rust via a macro, rather than to
  runtime segments. `cel-rs-macros` — already a proc-macro crate that parses CEL at compile
  time — is the likely home for this, though that's not committed.
- Further first-class function capabilities beyond typed noncapturing closures.
- Borrowing array-language concepts from J.
- Geometry beyond the implemented Euclid boundary (for example, paths/curves).
  Euclid now supplies geometry values rather than the earlier exploratory
  `kurbo::TranslateScale` proposal; graph-specific clamping remains sheet policy,
  not a bespoke matrix operation.

## adam-rs

**Mission:** a multi-way constraint system for property models — a graph of cells and
relationships where writing one cell propagates derived values to others via whichever
method satisfies the relationship. This is what lets a bound UI behave correctly without
imperative event handlers.

**Current state:** conditional relationship groups (`when`/`otherwise`) have landed.

**Directions being explored:**

- Planner correctness for single-method relationships and priorities.
- A way to query which cells are currently pinned by a single-method relationship, so
  callers (i.e. `begin`'s Inspector) can render those as disabled/uneditable widgets.
- "Memory" so a derived cell's value/priority isn't clobbered when the cell is pinned or
  self-referential — the current working hypothesis for how to handle the Adam-solver
  "unlink" behavior automatically.
- Additional clients beyond `adam-lang`, over time.

## adam-lang

**Mission:** the DSL, built on `cel-parser` infrastructure, that expresses `adam-rs`
constraint systems as source text (`sheet { cell ...; relationship { name := expr; } }`).

**Current state:** functional parser covering cells, `relationship` blocks, `out` cells, and
conditional groups. Persistent arrays and tuples use private canonical
whole-value ownership; host code reads/writes them through complete
`read_compound` / `write_compound` / `source_compound` witnesses.
Recursive defaults and explicit clone/equality diagnostics preserve the same
type model through relationships, filters, requirements, and outputs.
Prepared frames avoid repeated shape validation during propagation.

**Directions being explored:**

- General syntax improvements (open-ended, no fixed list yet).
- Parameterized, composable sheets: treating a `sheet` definition as a reusable relationship
  template that can be instantiated by name with cell arguments, e.g.:

  ```text
  sheet multiply(a, b, c) {
      relationship {
          c := a * b;
          a := c / b;
          b := c / a;
      }
  }

  sheet demo {
      cell e: f64;
      cell f = 5.0;
      cell g = 6.0;

      relationship multiply(f, g, e);
  }
  ```

- View-constraint sheets now have executable
  [pan/zoom](../cel-euclid/examples/pan_zoom.adm2) and
  [crop/straighten](../cel-euclid/examples/crop_straighten.adm2) demonstrations.
  They compose general Euclid/scalar operations; self-reference preserves user
  aspirations, including crop spring-back. Crop fitting uses inverse source-image
  bounds for general invertible affine transforms, explicit numerical requirements,
  and reciprocal-shrink presentation without a Rust fitting function.
  Live graphs now use the same library-first approach: a cached Adam sheet owns
  fit, zoom limits, centering, and pan clamping. Shared `GraphView` evaluation
  channels connect measured bounds, viewport changes, gestures, and animated
  controls to approved transforms on desktop, web, and live book pages.
  JavaScript retains layout and measurement, not a second constraint policy.
  See [Graph view ownership and verification](graph-view.md).

## begin

**Mission:** a development tool with two intended halves — developing property models
(implemented, in progress), and developing Dioxus applications that use property models
(not yet implemented).

**Target platforms:** desktop, web, and mobile are all intended shipping targets, not just
desktop with web/mobile as dev-only conveniences. Desktop is furthest along today, so it's
where most of the developer-experience work (live-reload, file watching, diagnostics) lands
first — that work should be expected to extend to web and mobile over time rather than stay
desktop-only. See issue #96 for one concrete gap this creates: the web/test build embeds
`examples/*.adm2` at compile time (`include_str!`), so editing an example's content forces a
full rebuild, unlike desktop's live-reload-off-disk.

**Current state:** desktop-first Dioxus app rendering the adam-rs graph (D3
force-directed layout) with an Inspector sidebar for reading/writing cells. A live-editable
adam-lang source panel (`SourcePanel`) with rustc-style error diagnostics has just landed.
The Inspector formats nested arrays/tuples, empty arrays, and unit recursively
through explicit debug capabilities. Compound edits reject without mutation;
primitive editing remains supported.

**Pivot — VSCode interop over in-app editing:** investment has shifted away from deepening
the in-app editor toward interop with VS Code. The `adam-lang` VS Code extension (backed by
the `adam-lsp` language server, see `editors/vscode-adam-lang`) now provides syntax
highlighting and live diagnostics for `.adm2` sources. Still open: hot-loading edited `.adm2`
sources into a running Dioxus app, binding them against `rsx!{}` descriptions, and opening a
side panel or window to visualize the adam-rs graph and edit cells directly, with errors and
diagnostics reported back through the terminal/console where `dx serve` was invoked. **The
in-app `SourcePanel` is scaffolding, not a permanent feature** — it's expected to be retired
once VSCode interop covers the same editing/diagnostics ground.

**Long-term aspiration (speculative — no path yet):** fully configuring an application from
`rsx!{}` and `.adm2` files via a prebuilt Begin-like application, so no hand-written Dioxus
component code is needed at all. This is blocked on Dioxus/rsx not yet exposing a mechanism
to do this; absent that, the focus stays on building tools that Dioxus developers can adopt
directly, with `begin` itself serving as the example of how to use them.

## Deferred, unordered relative to each other

- The `cel-parser` → Rust macro backend.
- The Dioxus-app-development half of `begin`.

Both matter; neither is scheduled ahead of the other yet.
