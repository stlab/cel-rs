# cel-rs Workspace Vision

This is a standing reference for Claude Code, not primarily for human readers. It captures
the *why* and *where this is headed* behind the workspace's components — complement it with
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

Dependency flow: `cel-parser`/`cel-runtime` → `property-model` → `pm-lang` → `begin`.
`pm-lang` is currently the only client of `cel-parser`/`cel-runtime`; more are expected over
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

**Directions being explored:**
- The second parser backend: compiling CEL directly to Rust via a macro, rather than to
  runtime segments. `cel-rs-macros` — already a proc-macro crate that parses CEL at compile
  time — is the likely home for this, though that's not committed.
- Language surface: tuples, arrays, and method calls.
- First-class functions.
- Closure syntax (open question, not yet designed).
- Borrowing array-language concepts from J.

## property-model

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
- Additional clients beyond `pm-lang`, over time.

## pm-lang

**Mission:** the DSL, built on `cel-parser` infrastructure, that expresses `property-model`
constraint systems as source text (`sheet { cell ...; relationship { method ... } }`).

**Current state:** functional parser covering cells, relationships, methods, and conditional
groups; the only `cel-parser` client so far.

**Directions being explored:**
- General syntax improvements (open-ended, no fixed list yet).
- Parameterized, composable sheets: treating a `sheet` definition as a reusable relationship
  template that can be instantiated by name with cell arguments, e.g.:

  ```text
  sheet multiply(a, b, c) {
      relationship {
          method [a, b] -> [c] { a * b }
          method [b, c] -> [a] { c / b }
          method [a, c] -> [b] { c / a }
      }
  }

  sheet demo {
      cell e: f64;
      cell f = 5.0;
      cell g = 6.0;

      relationship multiply(f, g, e);
  }
  ```

## begin

**Mission:** a development tool with two intended halves — developing property models
(implemented, in progress), and developing Dioxus applications that use property models
(not yet implemented).

**Current state:** desktop-first Dioxus app rendering the property-model graph (D3
force-directed layout) with an Inspector sidebar for reading/writing cells. A live-editable
pm-lang source panel (`SourcePanel`) with rustc-style error diagnostics has just landed.

**Pivot — VSCode interop over in-app editing:** further investment shifts away from
deepening the in-app editor toward interop with VSCode: edit `.pm` sources in VSCode,
hot-load them into a running Dioxus app, bind them against `rsx!{}` descriptions, and open a
side panel or window to visualize the property-model graph and edit cells directly, with
errors and diagnostics reported back through the terminal/console where `dx serve` was
invoked. **The in-app `SourcePanel` is scaffolding, not a permanent feature** — it's expected
to be retired once VSCode interop covers the same editing/diagnostics ground.

**Long-term aspiration (speculative — no path yet):** fully configuring an application from
`rsx!{}` and `.pm` files via a prebuilt Begin-like application, so no hand-written Dioxus
component code is needed at all. This is blocked on Dioxus/rsx not yet exposing a mechanism
to do this; absent that, the focus stays on building tools that Dioxus developers can adopt
directly, with `begin` itself serving as the example of how to use them.

## Deferred, unordered relative to each other

- The `cel-parser` → Rust macro backend.
- The Dioxus-app-development half of `begin`.

Both matter; neither is scheduled ahead of the other yet.
