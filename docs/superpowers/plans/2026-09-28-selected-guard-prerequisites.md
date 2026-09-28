# Selected Guard Prerequisites Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Execute only selected conditional guard prerequisites before branch choice, so unrelated filters read their final arguments and no callback is replayed merely because its step appears in both plans.

**Architecture:** Preserve static guard validation and existing two planning passes. Build one reverse producer index over the *selected unconditional plan*, walk backward from all guard inputs, and execute only the corresponding plan steps. Check the final plan against staged prerequisite provenance; return a diagnosed `Conflict` rather than rerun an incompatible callback.

**Tech Stack:** Rust 2024, `slotmap`, existing `adam-rs` planner, Cargo tests and clippy.

**Spec:** `docs/superpowers/specs/2026-09-28-selected-guard-prerequisites-design.md`

## Global Constraints

- Retain the existing all-method static `Sheet::validate` guard-independence check.
- An unconditional relationship may compute a conditional's match value.
- Use the selected method/filter dependencies, not every possible method, for prerequisites.
- Prerequisite index construction and traversal are O(V + E) per selected plan; do not scan all relationships separately for each match input.
- Never rerun a logical callback evaluation merely because both plans contain its step.
- A conservative incompatible-assignment conflict must be explicit and carry implicated sites.
- Publish no staged values on selected-prerequisite or seed-cycle failure; clear prior `changed()` at propagation start.
- Do not implement phase-C automatic plan reuse.

---

### Task 1: Select the guard prerequisite cone

**Files:**
- Create: `adam-rs/src/sheet/prerequisites.rs` (private `Sheet` helper and tests).
- Modify: `adam-rs/src/sheet.rs` (private module declaration and Phase 1 call).
- Test: `adam-rs/tests/integration.rs`.

**Interfaces:**
- Consumes: `PlanStep`, `Plan::execution_order`, `MatchSource`, `Method::{inputs,outputs}`, `CellData::filter`.
- Produces: `Sheet::guard_prerequisite_steps(&self, order: &[PlanStep]) -> Result<Vec<PlanStep>, Error>`; `Vec` preserves the full plan's topological order.

- [ ] **Step 1: Write failing public regression.** Add an integration fixture with source `x=8`, source `bound=10`, and a filter equivalent to `x := min(x, bound)`. An unconditional `mode -> match` method derives a conditional guard; its true branch has `mode -> bound` producing `3`. Assert that after propagation `bound=3`, `x=3`, and a filter callback counter records one invocation during propagation (exclude setup/attachment effects).
- [ ] **Step 2: Confirm RED.** Run `cargo test -p adam-rs --test integration conditional_bound_filter_runs_after_branch -- --exact`. Expect stale `x=8` before the fix.
- [ ] **Step 3: Index selected producers in one pass.** For each `PlanStep::Method(r,m)`, map its output cells to that step; for each `FilterReclamp(c)`, map `c` to that step. Start the traversal from `MatchSource::Cell(id)` or every `MatchSource::Expr(expr).inputs` for each conditional. Follow selected non-self method inputs and filter arguments only, using a visited set. Propagate the selected planner's cycle error. Filter `order` by the visited step set to preserve topological order; do not include unrelated reclamps.

```rust
fn guard_prerequisite_steps(&self, order: &[PlanStep]) -> Result<Vec<PlanStep>, Error> {
    let mut producer = HashMap::new();
    for &step in order {
        match step {
            PlanStep::Method(rel, index) => {
                for &out in &self.relationships[rel].methods[index].outputs {
                    producer.insert(out, step);
                }
            }
            PlanStep::FilterReclamp(id) => { producer.insert(id, step); }
        }
    }
    let mut pending = Vec::new();
    for conditional in self.conditionals.values() {
        match &conditional.source {
            MatchSource::Cell(id) => pending.push(*id),
            MatchSource::Expr(expr) => pending.extend(expr.inputs.iter().copied()),
        }
    }
    let (mut seen_cells, mut needed) = (HashSet::new(), HashSet::new());
    while let Some(cell) = pending.pop() {
        if !seen_cells.insert(cell) { continue; }
        if let Some(&step) = producer.get(&cell) {
            if needed.insert(step) {
                match step {
                    PlanStep::Method(rel, index) => {
                        let m = &self.relationships[rel].methods[index];
                        pending.extend(m.inputs.iter().copied().filter(|id| !m.outputs.contains(id)));
                    }
                    PlanStep::FilterReclamp(id) => {
                        pending.extend(self.cells[id].filter.as_ref().unwrap().args.iter().copied());
                    }
                }
            }
        }
    }
    Ok(order.iter().copied().filter(|step| needed.contains(step)).collect())
}
```

Derive `Hash` for crate-private `PlanStep` to permit `needed: HashSet<PlanStep>`.
The selected planner order is already topological; retain its existing cycle error
rather than adding a second redundant cycle detector in this helper.

- [ ] **Step 4: Stage only those steps.** Pass the selected step list to Phase 1 execution, but keep the entire unconditional assignment available for seed selection. Where `build_seeds` would evaluate unrelated callbacks, restrict its *requested self-reference roots* to selected prerequisite steps while retaining claimant/active context from the complete unconditional plan.
- [ ] **Step 5: Test a filtered guard.** Add a fixture where `match` is a filtered source whose value controls a conditional branch. Assert its filter runs before the guard and once, while an unrelated filtered source is deferred until the final plan.
- [ ] **Step 6: Verify and commit.** Run `cargo test -p adam-rs --test integration conditional_bound_filter_runs_after_branch`, `cargo test -p adam-rs --test integration filtered_guard`, `cargo test -p adam-rs --lib sheet::`, `cargo fmt --all`. Commit with `feat: evaluate only selected guard prerequisites` and the required Copilot co-author trailer.

### Task 2: Make staged prerequisite reuse explicit and safe

**Files:**
- Modify: `adam-rs/src/sheet.rs` (stage provenance and final-plan compatibility).
- Modify: `adam-rs/src/planner/seed.rs` (restrict requested roots while keeping the complete assignment's claimant/active context).
- Test: `adam-rs/tests/integration.rs`.
- Test: `adam-rs/src/planner/seed.rs`.

**Interfaces:**
- Consumes: `guard_prerequisite_steps`, `PropagationStage`, `Plan::execution_order`, `build_seeds`.
- Produces: a checked reuse path; `Error::Conflict { sites }` when the final plan cannot reuse a staged prerequisite without changing its producer inputs, output classification or method selection.

- [ ] **Step 1: Add failing tests.** Use a counter in an unconditional `mode -> match` callback to assert one invocation when the same step occurs in both plans. Add a fixture whose final-plan selected method differs from a pre-executed prerequisite method; assert `Error::Conflict` names its relationship and no staged values reach `read()` or `changed()`. Add a self-referencing guard fixture to count prerequisite seed evaluations.
- [ ] **Step 2: Confirm RED.** Run `cargo test -p adam-rs --test integration guard_prerequisite_reuse` and `cargo test -p adam-rs --test integration incompatible_guard_assignment`; expect at least the incompatibility case to fail.
- [ ] **Step 3: Record provenance and check it before final execution.** Keep a per-step record of the selected relationship/method, actual input producers, and whether the output was staged as source or derived. Before skipping a pre-executed step in the final plan, compare its selected method and producer path; reject a mismatch rather than repeat the callback. In particular, do not skip a filtered step if a newly selected branch can produce an argument. Use existing cell equality functions only for value comparisons where needed; do not downcast or clone arbitrary `dyn Any` values. Introduce `prerequisite_step_is_compatible(&self, pre_plan: &Plan, final_plan: &Plan, step: PlanStep, stage: &PropagationStage) -> bool` and `conflict_sites_for_step(step: PlanStep) -> Vec<ErrorSite>`.

```rust
if !prerequisite_step_is_compatible(&pre_plan, &final_plan, step, &stage) {
    return Err(Error::Conflict { sites: conflict_sites_for_step(step) });
}
```

- [ ] **Step 4: Check seed identity.** Keep claimant/active information from the full selected pre-plan, but request only guard-cone self-reference seeds. Record the identity and input provenance of any seed method actually evaluated; on final planning, reuse the cached logical seed evaluation or return explicit `Conflict` if its inputs/selection changed. Do not substitute a source for a `SeedCycle`.
- [ ] **Step 5: Verify and commit.** Run `cargo test -p adam-rs guard_prerequisite`, `cargo test -p adam-rs seed_cycle`, `cargo test -p adam-rs issue_182`, `cargo fmt --all`. Commit with `fix: validate staged guard prerequisite reuse` and the required Copilot co-author trailer.

### Task 3: Contracts, regression coverage and workspace verification

**Files:**
- Modify: `adam-rs/src/sheet.rs` and `adam-rs/src/sheet/prerequisites.rs` (contracts).
- Modify: `docs/superpowers/2026-09-27-planner-generalization-phase-a-handoff.md` (phase-B follow-up state).
- Test: `adam-rs/tests/integration.rs`.

**Interfaces:**
- Consumes: Tasks 1–2.
- Produces: documented runtime-conservative errors and a clean verified branch.

- [ ] **Step 1: Add an insertion-order and failure-state regression.** Reverse insertion of the unrelated filter and branch relationship in the `conditional_bound_filter_runs_after_branch` fixture; assert `x=3` both ways. Starting after a successful propagation with nonempty `changed()`, trigger a seed cycle or prerequisite conflict; assert `changed().count()==0` and all previously read values stay the same.
- [ ] **Step 2: Run targeted regressions.** Run `cargo test -p adam-rs --test integration conditional_bound_filter`, `cargo test -p adam-rs --test integration conditional_seed_cycle`, `cargo test -p adam-rs --test integration inequality_chain`, and `cargo test -p adam-rs --lib planner::`.
- [ ] **Step 3: Update contracts.** Document the selected guard cone, conservative conflict boundary, filter ordering, O(V+E) graph bookkeeping and the fact that seed/method invocations with *different* inputs are distinct evaluations. Keep static validation and phase C explicitly unchanged.
- [ ] **Step 4: Run full checks.** Run `cargo fmt --all`, `cargo build --workspace`, `cargo test --workspace`, `cargo test --doc --workspace`, `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`, `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`, `cargo clippy -p begin --all-targets -- -D warnings`, `$env:RUSTDOCFLAGS="-D warnings"; cargo doc --lib --no-deps --workspace`, and `cargo test -p begin --no-default-features`. Inspect plain build/test output for warnings.
- [ ] **Step 5: Review and commit.** Run `git --no-pager diff --check` and review all changes against the approved spec; commit with `docs: complete selected guard prerequisite follow-up` and the required Copilot co-author trailer. Do not open a PR until the full suite and a final code review pass.
