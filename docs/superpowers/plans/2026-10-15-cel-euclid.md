# CEL Euclid Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add optional Euclid geometry to CEL/Adam and use an Adam sheet to constrain the live graph view.

**Architecture:** Deliver three sequential, independently testable stages: general language/registration support, the geometry library and Adam examples, and graph migration. Record/member metadata belongs to `cel-parser`, concrete geometry belongs to `cel-euclid`, and graph policy belongs to an Adam sheet owned by each shared `GraphView`.

**Tech Stack:** Rust 2024, Euclid 0.22.14, existing CEL stack runtime and LL(1) parser, Adam, Dioxus 0.7, vendored D3, Vitest.

**Spec:** `docs\superpowers\specs\2026-10-15-cel-euclid-design.md`.

## Global Constraints

- Expose only `f64` component types initially.
- Do not expose Euclid unit parameters or introduce generic language syntax.
- Cells and locals named `x`, `width`, or similar do not shadow dot operations.
- Include migration of the live graph view, not just library demonstrations.
- Do not add a separate requested-crop cell to simulate memory.
- Use the already locked Euclid 0.22 series as a direct dependency.
- No core parser or runtime module may depend on Euclid or hardcode its type names.
- The bridge must run on desktop and web, including book-hosted graphs.
- Never render an unconstrained proposed view as though the model had approved it.

## Review Focus

- Independently installed libraries extend a same-named overload set without hiding existing calls such as scalar `min`; language Task 1 pins this.
- Reversed heterogeneous record fields evaluate in written order without invalid layouts or duplicate drops; language Task 3 pins this.
- Nested parentheses and control-flow headers do not turn existing `if flag { ... }` into a constructor; language Task 3 pins this.
- Rotating away and back restores Adam's crop aspiration instead of cumulatively shrinking it; library Task 3 pins this.
- Slow bridge responses and reset animations remain responsive without stale views or transition interruption; graph Tasks 2 and 3 pin this.

---

## Stage map

| Order | Plan | Working deliverable |
| --- | --- | --- |
| 1 | [Language support](2026-10-15-cel-geometry-language.md) | General records/member operations tested with host-defined types; no Euclid dependency. |
| 2 | [Euclid library](2026-10-15-cel-euclid-library.md) | Standalone/Adam geometry, real crop constraint, executable examples and diagnostics. |
| 3 | [Graph migration](2026-10-15-cel-euclid-graph.md) | Live desktop/web graph constrained by the Adam sheet. |

Do not dispatch stages concurrently. Later stages consume the earlier stage's
published interfaces; tasks within a stage explicitly name their prerequisites.
No product implementation has been performed while writing these plans.

## File responsibility map

- `cel-parser\src\registered_ops.rs`: library operation signatures, named
  callable bindings, record schemas, and immutable metadata queries.
- `cel-parser\src\lib.rs`, `parser_context.rs`, `ast.rs`, `fmt.rs`, `ty.rs`:
  grammar, two emission contexts, source-preserving syntax, formatting and
  registration-aware checking.
- `cel-runtime\src\dyn_segment.rs`: at most a checked prefix-offset query for
  existing tuple primitives; no Euclid types or new unsafe record reflection.
- `cel-std\src\math.rs`: existing math semantics on shared registration so
  independently installed overloads compose.
- `cel-euclid\src\{lib,records,geometry,transforms,adam}.rs`:
  concrete library and optional Adam adapter, split by geometry responsibility.
- `cel-euclid\examples\crop_straighten.adm2`: crop policy composed in CEL;
  no crop-specific library operation or Rust constraint helper.
- `adam-lang`, `adam-lsp`, books and affected `ez-adam` AST consumers:
  registered geometry diagnostics and expression support.
- `adam-web-ui\src\graph\{constraints,bridge}.rs` and
  `view_constraints.adm2`: model owner, message protocol and graph policy.
- `begin\assets\graph.js`, `begin\js\graph.test.js`: proposal/render adapter,
  preserving existing layout and interaction responsibilities.

## Algorithm Research Note

### Covered by stdlib / OSS

- Type/name dispatch: Rust `HashMap` plus small signature vectors; avoid a
  second symbol graph.
- Construction: existing `DynSegment::make_tuple`, `pop_tuple_as`, and
  `tuple_index` safely adapt stack values without boxed fields per evaluation.
  `RawStack::repack` explicitly forbids reordering; do not use it as a
  permutation algorithm.
- Affine arithmetic: Euclid `Transform2D`, inverse, `then`, and point/vector
  transformation (MIT OR Apache-2.0).
- Gesture interpretation: existing D3 zoom (ISC). Its default constraint
  source is the graph-policy reference, not a second production solver.

### Covered by known algorithm (implement directly)

- Crop expression: intersection of finitely many half-line inequalities for
  one uniform scale, expressed using inverse-transformed crop bounds and
  scalar arithmetic. Convexity makes the four-corner bounds sufficient.
- Bridge: serial producer/consumer with one in-flight request and one latest
  pending snapshot; generation/revision/sequence checks reject stale work.

### Custom logic (no standard match)

- Left-factored Rust-like expressions, restriction contexts, AST preservation,
  and Adam source/seed integration follow this repository's language model.

### Complexity flags

| Work | Size | Target |
| --- | --- | --- |
| Parse/check/format | source nodes | O(n), plus bounded signature scans |
| Constructor validation | record fields | O(f) expected; no factorial permutations |
| Geometry/crop | fixed 2D data | O(1) time/space |
| Bounds measurement | graph nodes | existing O(n) in JS |
| View propagation/messages | fixed sheet | O(1) model work, O(1) queued snapshots |

### Recommendation

Reuse Euclid and existing stack primitives. Keep geometry constant-size,
avoid numerical search for crop scale, and never grow an event queue with
pointer frequency. D3 `zoom.transform` interrupts transitions: accepted
responses must not call it in the middle of a reset/zoom animation.

Sources inspected: [Euclid 0.22.14 manifest](https://github.com/servo/euclid/blob/master/Cargo.toml),
[Euclid API](https://docs.rs/euclid/0.22.14/euclid/),
[D3 zoom source](https://github.com/d3/d3-zoom/blob/main/src/zoom.js),
and local `raw_stack.rs:231-254`, `dyn_segment.rs:840-918,2050-2110`,
`adam-rs\src\lib.rs:140-158`.

## Rollout gates

- [x] Execute language plan, review it, and commit its independently working result.
- [x] Execute library plan against that result, review it, and commit.
- [x] Execute graph plan, including rendered verification and interaction checks.
- [x] Update `docs\superpowers\2026-10-15-cel-euclid-handoff.md` before a
  multi-session/phase handoff or PR, recording actual outcomes and deferrals.
- [x] Run the complete final gate below, fix in-scope findings, and record
  unrelated substantial findings as GitHub issues rather than dropping them.

The whole-branch review covered 90 files and found no Critical/Important issues.
The final fix wave addresses both actionable Minor findings; controller approval
of that scoped wave is pending as of its commit. See the dated handoff for
completed implementation, verification evidence, and limitations.

Run in PowerShell from the worktree:

```powershell
cargo fmt --all -- --check
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
cargo clippy -p ez-adam --no-default-features --all-targets -- -D warnings
cargo clippy -p ez-adam --all-targets -- -D warnings
$env:RUSTDOCFLAGS = '-D warnings'; cargo doc --lib --no-deps --workspace
```

Also run `npm test` from `begin`, and `npm run compile`/`npm test` from
`editors\vscode-adam-lang` when its files changed. Build `adam-lsp` before
exercising extension diagnostics. Check desktop and web graph hosts with
`verifying-begin-ui`; no Spectrum bundle regeneration unless bundle inputs
actually change.

Only restore dependencies after a missing-dependency failure. Every commit
requires `cargo fmt --all` and the Copilot coauthor trailer. Never commit to
`main`; this worktree is the intended isolated workspace. Do not open a PR
or merge without a new user request.
