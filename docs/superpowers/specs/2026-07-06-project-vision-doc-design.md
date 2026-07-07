# Project Vision Document Design

**Date:** 2026-07-06
**Author:** Sean Parent
**Status:** Approved

## Overview

Add `docs/VISION.md`: a standing, undated reference capturing the long-term goals and
direction behind the workspace's components, for Claude's own consumption across sessions.
It fills a gap between `CLAUDE.md` (build mechanics, code style, always auto-loaded) and
`docs/superpowers/specs/` (dated, per-feature designs) — neither currently records *why*
each crate exists or where it's headed.

## Placement

- New file: `docs/VISION.md`.
- `CLAUDE.md` gains a one-line pointer to it (not inlined, to keep the auto-loaded context
  lean — `VISION.md` is read on demand when direction/roadmap questions come up).
- Unlike `docs/superpowers/specs/*-design.md`, `VISION.md` has no date or status field and
  is expected to be edited in place as direction changes, rather than superseded by a new
  dated file.

## Content Structure

One section per vision-bearing crate (`cel-runtime`/`cel-parser` together, `property-model`,
`pm-lang`, `begin`); `cel-rs-macros` and `xtask` are mentioned only where relevant to another
crate's direction, not given their own section. Each crate section has: a one-paragraph
mission, a one-line current-state note, and an unordered "directions being explored" list.
The list is explicitly *not* a committed sequence — the author works multiple components in
parallel across separate git worktrees, so forcing one global ordering across crates would
misrepresent how work actually proceeds. A short cross-cutting section covers the dependency
flow between crates and flags the handful of larger items with no committed relative order.

## Key Decisions From Discussion

- **begin's in-app `SourcePanel` editor** (landed via the `begin`↔`pm-lang` integration,
  `docs/superpowers/specs/2026-07-02-begin-pmlang-integration-design.md`) is explicitly
  flagged as scaffolding to retire once VSCode interop covers the same ground — not a
  permanent feature. Further investment in `begin` shifts to VSCode interop (edit pm-lang in
  VSCode, hot-load into a running Dioxus app, bind against `rsx!{}`, a side panel for
  graph/cell visualization and editing, diagnostics routed back through the `dx serve`
  terminal).
- **cel-runtime/cel-parser's DSL philosophy** is "interpreted by default, optionally
  compiled" in the same spirit as Dioxus RSX: `DynSegment` interpretation gives fast dev-time
  iteration; a not-yet-implemented second parser backend compiles the same DSL surface syntax
  down to native Rust for release/performance. `cel-rs-macros` (already a proc-macro crate
  that parses CEL at compile time) is the likely home for that backend, though this isn't
  committed.
- **property-model's conditional relationship groups** (`when`/`otherwise`) have already
  landed — not a future direction. Current focus is planner correctness for single-method
  relationships and priorities, querying which cells are pinned by a single-method
  relationship (so the UI can disable/gray those widgets), and "memory" so a derived cell's
  value/priority survives being pinned or self-referential (the working hypothesis for the
  Adam-solver "unlink" behavior).
- **pm-lang's directions** are open-ended syntax improvements plus parameterized/composable
  sheets — treating a `sheet` definition as a reusable relationship template that can be
  instantiated by name with cell arguments (`relationship multiply(f, g, e);`).
- **Long-term, explicitly speculative aspiration for `begin`**: fully configuring an app from
  `rsx!{}` + `.pm` files via a prebuilt Begin-like application, blocked on Dioxus/rsx not yet
  exposing a mechanism to do this — noted as aspirational, not a planned deliverable.

## Why Not a Full writing-plans Cycle

The deliverable is a single documentation file with no code changes, tests, or multi-step
sequencing. Routing it through the full plan → subagent-execution cycle would add process
overhead disproportionate to the task; the content itself is written directly following this
design's approval.
