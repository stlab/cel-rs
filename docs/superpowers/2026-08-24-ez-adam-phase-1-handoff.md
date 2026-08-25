# ez-adam Phase 1 (core, headless) — Handoff into Phase 2 (UI)

Status snapshot as of 2026-08-24, written for whoever picks up Phase 2 (the Dioxus UI) in a new
conversation/context. Read `docs/superpowers/specs/2026-08-24-ez-adam-design.md` first for the
full design, and `docs/superpowers/plans/2026-08-24-ez-adam-core.md` for how Phase 1 was broken
into tasks — this doc only summarizes what's done, what's deliberately deferred, and what's left.

## What's done

**Phase 1 (core, headless) — complete, 19 tasks, built via subagent-driven development, each
task independently reviewed plus a final whole-branch review whose findings were fixed in a
follow-up polish pass.** `cargo test -p ez-adam` passes 61 tests (55 unit tests in `src/`, plus 4
in `tests/adm2_round_trip.rs` and 2 in `tests/end_to_end.rs`); `cargo fmt --all -- --check`,
`cargo build -p ez-adam`, and `cargo clippy -p ez-adam --all-targets -- -D warnings` are all clean
(clippy's underlying `adam-lang` compile failure is a separate, pre-existing, already-tracked
issue — see "Deliberately deferred" below).

The new `ez-adam` crate (`ez-adam/src/`) implements the full document model, mutation ops,
validation, codegen, and persistence layers from the design spec, with no UI yet:

- **Document model** (`model/`): `Document`, `Cell`/`CellType`/`ClampRange<T>`, `CellNode`,
  `RelationshipGroup`, `ConditionalGroup`/`ConditionExpr`/`ConditionalBranch`/`CellValueLiteral`,
  and `geometry::Point` — all `serde`-derived, `SlotMap`-keyed, matching §3 of the design spec.
- **Ops** (`ops/`): pure mutation functions over a `Document` — `cells::{add_cell, add_cell_node,
  set_output, set_restrict}`, `relationships::{create_relationship, add_member,
  set_member_formula, duplicate_relationship_group}`, `conditionals::{add_conditional_from_bool_cells,
  add_conditional_with_formula, add_branch, toggle_enabled_group}`. Each has its own doc-comment
  contract and contract-derived unit tests, per this workspace's rule against untested framework
  glue (no framework glue exists yet in Phase 1, but the same rule shaped these as small pure
  functions ready for thin Dioxus event-handler wrappers in Phase 2).
- **Validation** (`validation.rs`): `validate_cel_expression` — syntax-only CEL validation via
  `cel-parser`, for relationship-group formula text and restrict-expression text. Does not
  type-check against a sheet's declared cell types — see issue #148 below.
- **Codegen** (`codegen/mod.rs`): `generate_adm2(doc: &Document) -> String`, one-way `Document` →
  `.adm2` text, per §5 of the design spec — cell decls (with clamp `filter` clauses), top-level
  `relationship` blocks, and `conditional` blocks (both `Cells` mode, single- and multi-cell
  tuple, and `Formula` mode). Tested both by exact-output-text unit tests in `codegen/mod.rs` and
  by round-tripping generated text through `adam-lang`'s real parser
  (`tests/adm2_round_trip.rs`, `tests/end_to_end.rs`) to confirm it's syntactically valid `.adm2`
  — now covering every construct `generate_adm2` can emit, including the multi-cell `Cells`-mode
  tuple condition and `Formula`-mode condition (added in this final-review polish pass; previously
  those two forms were only checked via `out.contains(...)` string assertions, never actually
  parsed).
- **Persistence** (`persistence.rs`): `to_json`/`from_json` — `serde_json` round-trip of a
  `Document`, tested for a representative document covering multiple `CellType`/`ConditionExpr`
  variants and a multi-branch `ConditionalGroup`.

Per-task briefs and reports for all 19 tasks, plus the final whole-branch review's diffs, live
under `.superpowers/sdd/2026-08-24-ez-adam-core/` in this worktree (task-1..19-brief/report.md,
`review-*.diff`).

**Final-review polish pass (this handoff's own changes):**

- Added the two missing round-trip regression tests noted above
  (`a_multi_cell_cells_mode_conditional_group_generates_valid_adm2`,
  `a_formula_mode_conditional_group_generates_valid_adm2` in `tests/adm2_round_trip.rs`).
- Fixed a stale doc comment on `ops::cells::set_output` that still claimed `.adm2` emission
  happens for the `output` flag (removed during Task 17 — see issue #147); it now matches
  `Cell::output`'s already-correct doc comment.
- Updated `docs/superpowers/plans/2026-08-24-ez-adam-core.md`: Task 15's section now notes the
  `out` emission it describes was removed during Task 17 (doesn't parse — see #147), and the
  "Deferred pending upstream work" section now lists all three tracked gaps (#146, #147, #148),
  not just #146.

## What's deliberately deferred

**Phase 2 (UI) — a separate, not-yet-written implementation plan.** Per §4 of the design spec:
the Dioxus desktop app shell, SVG canvas rendering of cells/relationship-groups/conditional-groups
from `Document` positions, tool-dependent click/drag dispatch (Select/Add Relationship/Add
Conditional/Duplicate), the context-sensitive side panel, `rfd`-based native open/save dialogs
wired to `persistence::to_json`/`from_json`, and an `.adm2` export action wired to
`codegen::generate_adm2`. Live-diagnostics rendering of `validate_cel_expression`'s `Err` case via
`annotate-snippets` (matching `begin`'s `SourcePanel`) is part of this same follow-up plan.

**Three tracked upstream/design gaps, each a GitHub issue, none blocking Phase 2 from starting:**

- **#146** — `adam-lang` has no boolean-rejecting cell filter syntax, so `Cell.restrict` has no
  `.adm2` codegen target yet. The field is captured in the document model and round-trips through
  save/load, but `generate_adm2` emits nothing for it.
- **#147** — `adam-lang`'s `out` can't reuse an existing `cell`'s own name (and always models a
  derived value, never a flag on a plain writable cell), so `Cell.output`'s originally-planned
  `out <name> := <name>;` codegen doesn't parse and was removed during Task 17. The field is
  captured in the document model and round-trips through save/load, but `generate_adm2` emits
  nothing for it.
- **#148** — `validate_cel_expression` checks CEL syntax only, not types against a sheet's
  declared cell types (e.g. `f64 * i64` with no cast is accepted as "valid CEL" even though it
  won't compile against real cell declarations). Discovered while writing Task 17's round-trip
  test fixtures; whether Phase 2's formula-editing UI needs live type-checking (reusing
  `adam_lang::typecheck::check_sheet`) is an open design question for that plan.

**One pre-existing, unrelated blocker, not fixable within this crate:**

- **#116** — `cargo clippy -p begin ...`/anything pulling in `adam-lang` fails with two
  `clippy::only_used_in_recursion` errors in `adam-lang/src/{ast_parser,parser}.rs`'s
  `parse_type_expr` methods. Confirmed pre-existing and unmodified on `origin/main` (independently
  reconfirmed during this final-review pass); reopened during this work since it also blocks a
  clean `cargo clippy -p ez-adam --all-targets -- -D warnings` run (transitively, via `adam-lang`).
  Out of scope for `ez-adam` — needs a fix inside `adam-lang` itself.

## What's left

Write and execute the Phase 2 (UI) implementation plan (Dioxus desktop app shell, canvas, toolbar,
side panel, persistence/export wiring — see "What's deliberately deferred" above for the full
scope pulled from the design spec). Nothing in Phase 1's core API blocks starting that plan.
