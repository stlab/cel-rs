# Conditional Forced Priority Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve a conditionally forced cell's source priority and restore it during guard and release planning.

**Architecture:** Keep remembered source priority distinct from the temporary priority of a forced output. Thread the appropriate priority through both planning phases, seed ordering, and cache certificates; preserve ordinary reversible derivation and the existing error boundary. Test public outcomes and compare cached execution with forced replanning.

**Tech Stack:** Rust 2024, `adam-rs`, `adam-web-ui`, Cargo.

**Spec:** `docs/superpowers/specs/2026-11-10-conditional-forced-priority-design.md`.

## Global Constraints

- Ordinary reversible derivation overwrites the losing source value and assigns derived priority; do not restore an older value or write priority from before that derivation.
- A conditionally forced cell retains its priority from immediately before forcing; a successful write while forced replaces that priority with the write's actual recency.
- The pre-plan uses remembered priority for previously conditionally forced cells when computing guards; the final plan uses remembered priority for cells released from conditional forcing.
- Keep static guard independence and existing incompatible-pre-plan conflict errors.
- Keep self-reference aspirations, active forcing, overlapping and cascaded forcing, and out-cell write restrictions intact.
- Cached execution and forced replanning must agree. Host recovery retains accepted edit order; effective-value recordings remain unchanged.
- Never inspect or reuse a sheet after a fallible mutable operation returns an error.
- Follow `CLAUDE.md` and `.github/instructions/` Rust contracts, allocation, and testing rules; do not introduce a public memory API.

## Review Focus

- A same-value write while forced changes priority even though its value is unchanged: cover in Task 1.
- A cell demoted by ordinary derivation before forcing must not revive an older explicit priority: cover in Task 1.
- A second active forcing cause must not overwrite remembered priority or cause early restoration: cover in Task 1.
- Guard pre-planning must respect remembered priority without silently accepting an incompatible final plan: cover in Task 1.
- Recovery and cached execution must preserve the observable result and direction, not merely equal scalar values: cover in Task 2.

## File map

- `adam-rs/src/cell.rs`: internal current and remembered priority representation.
- `adam-rs/src/sheet.rs`, `adam-rs/src/sheet/cached.rs`: forcing transitions, propagation phases, commit and cache lifecycle.
- `adam-rs/src/planner/` and `adam-rs/src/planner.rs`: priority selection, seed ordering, and reuse certificates, if the selected representation requires them.
- `adam-rs/tests/integration.rs` and `adam-rs/tests/contract_regressions.rs`: public behavior and existing contracts.
- `adam-web-ui/src/binding/tests.rs`: accepted-state recovery contract.
- `docs/superpowers/2026-11-10-issue-219-handoff.md`: completed work, explicit deferrals, and continuation.

---

### Task 1: Remember priority through conditional forcing

**Files:**
- Modify: `adam-rs/src/cell.rs`, `adam-rs/src/sheet.rs`, `adam-rs/src/sheet/cached.rs`; planner files where required.
- Test: `adam-rs/tests/integration.rs`, `adam-rs/tests/contract_regressions.rs`.

**Interfaces:**
- Consumes: `Sheet::{add_cell, add_relationship, add_conditional, write, propagate, read, source, is_source, is_forced}` and the approved spec.
- Produces: the same public APIs with the approved value and priority behavior; any internal representation is private.

- [ ] **Step 1: Write a failing release-direction test.** Add this fixture and test to `adam-rs/tests/integration.rs`; retain the project's documentation-comment convention:

```rust
/// Checks restored write priority when conditional forcing changes to a reversible relationship.
#[test]
fn conditional_forcing_restores_priority_for_reversible_release() {
    let mut sheet = Sheet::new();
    let mode = sheet.add_cell(0_i32);
    let input = sheet.add_cell(30_i32);
    let a = sheet.add_cell(1_i32);
    let b = sheet.add_cell(2_i32);
    let force = sheet
        .add_relationship(vec![Method::from_fn_1_1(input, a, |v: &i32| Ok(*v))])
        .unwrap();
    let reversible = sheet
        .add_relationship(vec![
            Method::from_fn_1_1(a, b, |v: &i32| Ok(*v)),
            Method::from_fn_1_1(b, a, |v: &i32| Ok(*v)),
        ])
        .unwrap();
    sheet
        .add_conditional(
            MatchExpr::cell(mode),
            vec![(vec![1_i32], vec![force]), (vec![2_i32], vec![reversible])],
            vec![],
        )
        .unwrap();
    sheet.write(b, 22_i32).unwrap();
    sheet.write(a, 11_i32).unwrap();
    sheet.write(mode, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 30);
    assert_eq!(*sheet.source::<i32>(a).unwrap(), 11);
    sheet.write(input, 40_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 40);
    sheet.write(mode, 2_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(sheet.is_source(a));
    assert_eq!((*sheet.read::<i32>(a).unwrap(), *sheet.read::<i32>(b).unwrap()), (11, 11));
}
```

- [ ] **Step 2: Run the test before implementation.** Run `cargo test -p adam-rs --test integration conditional_forcing_restores_priority_for_reversible_release`. It must fail because `b` wins the released relationship under the old forced-output priority. If the fixture fails for a different reason, diagnose and correct it before proceeding.

- [ ] **Step 3: Add the remaining contract tests.** Use public observations and independent fresh sheets:
  - With the Step 1 fixture, write `a = 11` while forced, then `b = 22`, and release: `b` wins and both read 22. Reverse those writes: `a` wins and both read 11. Repeat with `a` already equal to 11 before its write to prove same-value writes affect priority.
  - Force `a` through two independently controlled conditionals that do not produce `a` simultaneously: switch directly from the first to the second, update its upstream value, then release both. Also cover two simultaneously active, valid forcing causes that share a cascaded output without selecting competing writers. Assert memory survives until the final cause ends. If no such graph is valid, document the graph constraint and discuss it before narrowing the spec. Add a cascaded-forcing case based on `cascaded_forced_output_of_an_unconditional_relationship_shadows_and_reverts`: after release, initially newer `c` remains source at 9 and `b` becomes 10. Update that existing test's old `b = 0, c = 1` expectation and explanation.
  - First use an ordinary reversible relationship to demote a previously edited cell and overwrite its source, then force and release it into a reversible contest. Assert the prior ordinary-derived priority and value apply, not the older edit. Retain `unforced_conditional_relationship_does_not_shadow_its_losing_cell`.
  - Force and release a previously edited self-referencing input. Assert its source aspiration and subsequent spring-back remain intact. Retain the existing inequality-chain tests.
  - Construct and validate a guard-independent sheet with `a <-> guard`, a conditional on independent `mode` forcing `a`, and a separate conditional on `guard` controlling an unrelated output. Set distinct `a` and `guard` aspirations and make `a` more recent before activation. Verify that activation succeeds, then release forcing and assert that the pre-plan selects `a -> guard` from `a`'s remembered aspiration and takes the corresponding branch. This proposed fixture has not yet been executed; confirm its validity and distinguishing observations before relying on it. If it cannot express the contract, find another valid fixture or discuss the limitation; do not silently weaken the requirement. Retain `incompatible_guard_assignment_returns_conflict` and add a previously forced variant that checks `Error::Conflict` without observing the failed sheet.
  - Exercise active, repeated forced, and released states through `read`, `source`, `is_source`, and `is_forced`, including no explicit write between forcing and release.

- [ ] **Step 4: Implement the contract.** Trace `CellData::strength`, `Sheet::write`, propagation stage/commit, both `acquire_prepared_plan` calls, `planner::release`, seed evaluation, and `SourceCertificate::is_valid`. Store the priority actually in effect at the transition into conditional forcing, preserve it until the final forcing cause ends, and replace it on successful writes while forced. Ensure both planning phases and certificates use the phase-appropriate priority; do not merely repair strength after a release plan has already been selected. Preserve prior commit/error boundaries and avoid allocating a per-propagation copy of every cell.

- [ ] **Step 5: Run focused and crate tests.** Run `cargo test -p adam-rs --test integration`, `cargo test -p adam-rs --test contract_regressions`, and `cargo test -p adam-rs --lib`. All must pass, including the updated cascade contract.

- [ ] **Step 6: Commit.** Format with `cargo fmt --all`, stage only Task 1 files, and commit with the required Copilot co-author trailer. Preserve the spec and plan for their own commit.

### Task 2: Verify cache parity and host recovery

**Files:**
- Test: `adam-rs/src/sheet/cached.rs`, `adam-web-ui/src/binding/tests.rs`.
- Modify: affected public Rust contract documentation if needed.
- Create: `docs/superpowers/2026-11-10-issue-219-handoff.md`.

**Interfaces:**
- Consumes: Task 1's public behavior and existing `propagate_forced_replan` and `assert_same_snapshot` test helpers.
- Produces: parity and host recovery regressions, contract documentation, and a handoff for #186.

- [ ] **Step 1: Add cache parity tests.** Build paired equivalent sheets. Run ordinary propagation on one and `propagate_forced_replan` on the other after each of: initial writes `b = 22`, `a = 11`; activate forcing; two upstream changes; same-value write to forced `a`; write `b = 22`; release; reactivate; release again. Use `assert_same_snapshot` to compare effective and source values, source/forced classifications, selected method, changed membership, and diagnostics at every successful step. Include the independent-guard case and the case where the guarded cell was already demoted before forcing.

- [ ] **Step 2: Add host recovery test.** Use a `SheetOwner` factory with a conditionally forced numeric cell and a reversible release relationship. Submit accepted transactions in the order `b = 22`, `a = 11`, force, change upstream, release; capture the published values and direction. Call `restore_accepted()` and verify the reconstructed sheet has the same values and source direction. Repeat with a successful same-value write while forced and an intervening write to the competing cell. Do not equate `source_cells` recordings with this ordered recovery journal.

- [ ] **Step 3: Document the contract.** Update the adjacent public `Sheet` documentation and any affected crate-level examples to distinguish forced, self-referencing, and ordinary reversible outcomes. Add the handoff summarizing the accepted contract, tests, and remaining #186 work.

- [ ] **Step 4: Run focused checks.** Run `cargo test -p adam-rs --lib sheet::cached::tests`, `cargo test -p adam-web-ui`, and `cargo doc -p adam-rs --no-deps --lib`. All must pass.

- [ ] **Step 5: Commit.** Run `cargo fmt --all`, stage the Task 2 files and approved design and plan, and commit with the required Copilot co-author trailer.

### Task 3: Whole-change verification

**Files:** No new product files.

**Interfaces:** Consumes Tasks 1–2 and the repository's documented checks.

- [ ] Run `cargo build --workspace`, `cargo test --workspace`, `cargo test --doc --workspace`, and `cargo fmt --all --check`; check exit statuses and plain build/test warnings.
- [ ] Run all five documented Clippy commands: workspace excluding `begin`, `begin` with and without default features, and `ez-adam` with and without default features, each with `--all-targets -- -D warnings`.
- [ ] Run `RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace`.
- [ ] Review the complete branch against the approved spec, address in-scope findings, and report any remaining limitations before opening a PR.
