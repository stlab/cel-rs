# Host-visible propagation outcomes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define safe host recovery after `Sheet` mutation errors without exposing propagation commit phases.

**Architecture:** Keep the existing `Sheet` methods and `Result<(), Error>` API. A mutating operation that returns an error leaves the sheet's state unspecified; the host drops it and rebuilds from host-owned inputs. Preserve successful validation and filter diagnostics.

**Tech Stack:** Rust 2024, `adam-rs`, Cargo tests and rustdoc.

**Spec:** `docs/superpowers/specs/2026-11-04-host-visible-propagation-outcomes-design.md`

## Global Constraints

- The host does not need to know whether propagation published derived values before returning an error.
- Each successful write takes effect independently.
- A false requirement and a filter failure remain nonfatal diagnostics.
- Do not add a stronger failure guarantee than the project-wide error contract.

## Review Focus

- A write or graph declaration returns `Err`: keep `write_wrong_type_returns_type_mismatch` and `add_out_returns_invalid_cell_kind_for_a_second_writer` limited to their error assertions.
- A method or planning callback errors before derived-value publication: report the existing `Error`, discard the sheet, and do not read values or metadata.
- A requirement callback errors after derived values may have been published: report the existing `Error` and take the same discard-and-rebuild action.
- Cache unit tests may compare returned errors, but must not inspect diagnostic sites through or otherwise read/reuse a failed `Sheet`; retain cache assertions on successful paths.
- A false requirement completes propagation: retain the successful diagnostics and apply the host's validation policy without treating it as a runtime error; keep `add_out_with_a_requirement_that_would_fail_still_succeeds_and_propagate_reports_it`.
- A filter callback fails: propagation succeeds with filter diagnostics; keep `filter_reclamp_failure_is_recorded_without_aborting_propagate_or_changing_the_cell` distinct from a callback `Err`.

---

## File responsibilities

- `adam-rs/src/sheet.rs` states the `Sheet` mutation recovery contract, removes unsupported post-error state promises from `propagate` and `changed`, and updates unit regressions.
- `adam-rs/tests/contract_regressions.rs` verifies error reporting, successful contributor updates, and host recovery by constructing a fresh sheet without reading or reusing a failed sheet.
- `adam-rs/src/sheet/cached.rs` keeps cache-equivalence assertions on successful paths and removes assertions or retries that depend on state after an error.
- `#272` owns the runnable host adapter and recording example in its own approved spec and worktree; this plan does not modify its files.

### Task 1: Align mutation recovery tests and documentation

**Files:**
- Modify: `adam-rs/src/sheet.rs`
- Modify: `adam-rs/tests/contract_regressions.rs`
- Modify: `adam-rs/src/sheet/cached.rs`

**Interfaces:**
- Consumes: `Sheet::propagate(&mut self) -> Result<(), Error>` and the existing `Error` variants.
- Produces: no new runtime API. The public contract states that the host drops a `Sheet` after an error from a mutating operation and rebuilds from host-owned inputs.

- [ ] **Step 1: Rewrite the staged output failure regression**

Rename `late_output_type_failure_preserves_published_state_and_recovers` to
`late_output_type_failure_requires_rebuilding_the_sheet`. Keep the assertion that
the failing `propagate` returns `Error::TypeMismatch`, then drop the failed sheet
without reading cells, strengths, selected methods, or changed-state. Construct a
fresh sheet from valid host-owned inputs, propagate it, and assert the resulting
output values.

- [ ] **Step 2: Keep contributor assertions on successful propagation**

Rename `filtered_contributors_keep_completed_active_graph_until_success` to
`filtered_contributors_update_after_successful_propagation`. Keep its assertions
that a completed graph remains observable while successful edits are pending,
then complete propagation successfully and assert the updated contributor set.
Remove the injected callback failures and all reads or retries after an error.

- [ ] **Step 3: Add a public recovery regression**

Add `failed_propagation_requires_rebuild` to `contract_regressions.rs`. Exercise
a method callback error and a requirement callback error on separate sheets;
assert that each returns `Error::MethodFailed`, then drop each sheet without
reading values or metadata. Construct a fresh sheet from the last accepted
host-owned inputs, propagate successfully, and assert its effective values and
contributor set. Keep the existing write and graph-declaration error tests
limited to their returned errors; do not use their failed sheets afterwards.

- [ ] **Step 4: Remove cache tests that rely on failed-sheet state**

Update `assert_same_result` to fail immediately if either propagation returns
`Err`, because all its callsites expect success and continue using the sheets.
Dedicated failure regressions compare returned error values and end without
reusing the failed sheets. In
`differential_successful_structural_mutations`, retain the
successful structural mutation cases and remove the rejected-operation probes
that continue using either sheet after `Err`; preserve the successful cache
reuse and snapshot assertions. Remove the partial-error branch from
`mutate_fixture`.

In `differential_structural_cycles_match_forced_replan`,
`differential_pure_failures_match_forced_replan`,
`differential_requirement_error_matches_forced_replan`,
`differential_restaged_seed_provenance_errors_match_forced_replan`, and
`differential_seed_cycle_matches_forced_replan`, keep returned-error comparisons but
remove failed-sheet diagnostic-site, value, metadata, cache, or recovery
assertions. Remove the unsupported exact selected-method assertions from the
requirement-error fixture. Rewrite `rejected_writes_and_attachments_preserve_reuse`
as `rejected_mutations_return_expected_errors`, using an independent sheet for
each returned-error assertion and making no cache or post-error state
assertions. In `partially_failing_add_out_returns_error`,
`cached_method_failure_returns_error`, and
`cached_requirement_failure_returns_error`, assert
the expected error and end the fixture without further reads, mutations, or
propagation. Preserve successful cache-path checks.

- [ ] **Step 5: Run the focused recovery tests**

Run: `cargo test -p adam-rs --lib late_output_type_failure_requires_rebuilding_the_sheet`

Expected: the staged type-error regression passes without inspecting the failed
sheet and the fresh sheet propagates successfully.

Run: `cargo test -p adam-rs --test contract_regressions failed_propagation_requires_rebuild`

Expected: both callback-error cases return the existing `Error`, and the fresh
sheet propagates from host-owned inputs.

Run: `cargo test -p adam-rs --lib sheet::cached::tests`

Expected: successful cache reuse remains covered, and error comparisons do not
inspect or reuse failed sheets.

- [ ] **Step 6: Document the public recovery contract**

Add a contract to `Sheet` stating that after a mutating operation returns `Err`,
its state is unspecified; the host must not inspect, read, retry, or capture from
it, and must rebuild from host-owned inputs. Include a `# Examples` reference to
the existing crate-level graph example. In `Sheet::propagate`, remove guarantees
about state after an error: values or changed-state untouched before commit,
published values or the prior display snapshot retained after commit,
prepared-plan eligibility after partial structural mutation, and cache
eligibility after a rejected write. In `Sheet::changed`, remove the guarantee
that `Error::SeedCycle` leaves the iterator empty. Keep the error variants,
successful propagation behavior, false-requirement diagnostics, and filter
diagnostics documented.

- [ ] **Step 7: Run crate tests, formatting, and documentation checks**

Run: `cargo fmt --all`

Run: `cargo test -p adam-rs`

Expected: all `adam-rs` unit, integration, and documentation tests pass.

Run: `RUSTDOCFLAGS="-D warnings" cargo doc -p adam-rs --lib --no-deps`

Expected: documentation builds without warnings or broken links.

- [ ] **Step 8: Commit the recovery contract**

```bash
git add adam-rs/src/sheet.rs adam-rs/src/sheet/cached.rs \
  adam-rs/tests/contract_regressions.rs \
  docs/superpowers/specs/2026-11-04-host-visible-propagation-outcomes-design.md \
  docs/superpowers/plans/2026-11-04-host-visible-propagation-outcomes.md
git commit -m "docs(adam-rs): define propagation recovery contract" \
  -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

## Cross-issue handoff

After #273 lands, discuss and approve #272's spec before implementing its
framework adapter. The #272 host should keep authoritative inputs outside
`Sheet`, discard the sheet after a mutating-operation error, and demonstrate
validation rejection with a false requirement.
