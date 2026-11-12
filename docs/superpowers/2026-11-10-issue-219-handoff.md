# Conditional forced priority — handoff for #186

Status as of 2026-11-10 for the #219 work. The approved design is
`docs/superpowers/specs/2026-11-10-conditional-forced-priority-design.md`; the plan is
`docs/superpowers/plans/2026-11-10-conditional-forced-priority.md`.

## Accepted contract

- A conditionally forced cell keeps the priority it had when forcing began, including a
  priority already derived by an ordinary reversible relationship.
- While forced, the forcing relationship supplies the effective value. `Sheet::source`
  keeps the aspiration. An explicit write, including a same-value write, replaces the aspiration
  and priority with the write's recency.
- Overlapping or cascaded forcing keeps the remembered priority until the last cause ends.
- On release, the aspiration and remembered priority compete in the first plan that can use the
  cell. Ordinary reversible publication, self-referencing seeds, and discard-on-error are unchanged.
- The pre-plan uses the remembered priority for a cell forced in the previous propagation. An
  independent guard whose prerequisite is incompatible with the final plan still reports the
  existing conflict.

## Done

- Task 1 (commits `08f74eb`, `1c6d711`): implementation and public-behavior tests in
  `adam-rs/tests/integration.rs`.
- Task 2:
  - `adam-rs/src/sheet/cached.rs`: cached-versus-forced-replanning parity after every step of the
    forcing lifecycle (activate, upstream changes, same-value write to the forced cell, competing
    write, release, reactivate, release), a cell demoted before forcing, two causes sharing a
    cascade, and a staged independent guard.
  - `adam-web-ui/src/binding/tests.rs`: `restore_accepted()` replays the ordered transaction
    journal and reproduces effective values, aspiration, and source direction, with and without a
    same-value write and a competing write while forced.
  - `adam-rs/src/sheet.rs`: the module documentation distinguishes forced, self-referencing, and
    ordinary reversible outcomes; `Sheet::is_forced` links to it.

## Deliberately unchanged

- `source_cells` recordings capture effective values, not aspirations or priorities. Recovery
  depends on the ordered accepted-transaction journal, not on recordings.
- A sheet is not observable after a failed fallible mutation; the host discards and rebuilds it.

## Remaining for #186

- Guard conflict boundary: registering an independent guard conditional *before* forcing
  activation, when remembered-priority pre-planning picks a prerequisite incompatible with the
  active final plan, still reports the existing conflict. The tests stage the guard after
  activation. #186's 2D seedfill acceptance work must either accept this boundary or relax the
  pre-plan/final-plan compatibility rule.
- Acceptance coverage of the full 2D seedfill scenario remains in #186.
