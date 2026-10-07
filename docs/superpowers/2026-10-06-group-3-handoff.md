# v1.0 Group 3 - Requirement Identity and Live Diagnostics

Status as of 2026-10-06: implementation complete in
`worktree-adam-rs-requirement-ids`, based on `8d94336c`. No commit, push, PR, or GitHub issue
closure occurred during this implementation.

## What's done

**#194 - caller-owned requirement names.** `adam-rs` stores requirement predicates and
structural identity only. `Sheet::add_requirement(cell, requirement)` returns a
`RequirementId`; `Sheet::add_out(writer, Vec<Requirement>)` retains its `CellId` return.
`RequirementData` no longer stores a name, and `Sheet::requirement_name` no longer exists.
Callers obtain attached IDs through `cell_requirements` and failed IDs through
`violated_requirements`. Registration, immediate validation, output enforcement, violation
ordering, and prepared-plan invalidation retain their existing contracts.

`adam-lang::ParsedSheet::requirement_names` maps IDs to explicit `@name` labels. Unnamed
requirements have no map entry. The parser rejects duplicate labels within a single cell,
source, or output declaration, but accepts the same label on different cells and multiple
unnamed predicates. Duplicate-label diagnostics point at the repeated label.

The Inspector resolves violated IDs through this caller-owned table, preserves attachment
order when joining names, and retains generic invalid messages for unnamed failures.
Workspace callers, examples, API documentation, and cached-plan tests use the new signatures.

**#191 - current source metadata in live diagnostics.** Commit `028db32f` already changed
`begin` to retain the whole `ParsedSheet` in one Dioxus signal. Example selection, opened-file
loading, and reloads replace this value, including its span tables. `write_and_propagate`
already releases its write guard before formatting errors against the current parsed sheet.
Adding a separate method-span signal would duplicate metadata and introduce synchronization
risk, so this implementation preserves the existing ownership.

`live_write_errors_use_current_parsed_sheet_spans` drives real Dioxus signals through the
live-write path. It checks `MethodFailed` without an inner CEL `SpanContext`, `TypeMismatch`,
and replacement of the sheet/source context, including the current filename and binding
location. The regression verifies existing behavior rather than claiming a new wiring fix.

## Verification

The new parser metadata regression first failed because `ParsedSheet::requirement_names`
did not exist. The completed change passed:

- `cargo fmt --all` and `git diff --check`.
- `cargo test -p adam-rs -p adam-lang -p adam-web-ui --quiet`.
- `cargo test --workspace --quiet`, including workspace doctests.
- `cargo build --workspace --quiet`, without compiler warnings.
- All five repository clippy checks with `--all-targets -- -D warnings`: workspace excluding
  `begin`, and both default/no-default-feature configurations of `begin` and `ez-adam`.
- `cargo doc --workspace --no-deps --lib --quiet` with `RUSTDOCFLAGS="-D warnings"`.

An independent whole-change review found no significant correctness or contract issues.

Rendered verification used the web build and isolated headless Edge. In `out-cell`, editing
`a` from `0` to `10` propagated `(10.0, 0.0)` and rendered invalid states on `a`, `b`, and
`result`; restoring `a` to `0` cleared them. The screenshot and DOM capture live in the
session artifacts, not the repository. Spectrum components upgraded and the theme had three
adopted stylesheets.

The initial browser readiness check mistakenly treated hidden development-template text
as a visible rebuild overlay. Final verification inspected the rendered screenshot and live
control state instead of relying on `body.innerText`.

## Deliberately unchanged

Output writers without an inner CEL span still use the existing diagnostic fallback:
`add_out` returns a cell ID, not its internal writer relationship ID, so the parser cannot
populate that writer's method-span entry. This existing boundary is documented on
`ParsedSheet::method_spans`; neither Group 3 issue requests changing it.

No redundant span signals, new dependencies, or planner algorithm changes were introduced.
The user approved the ownership design and waived further design/plan approval gates.

## What's left

Group 3 has no remaining implementation tasks. The milestone plan marks #194 and #191
complete in this worktree; their GitHub issues remain open pending integration. Before
opening a PR, follow the repository's PR workflow and rerun its required pre-PR checks.
