# Planner Generalization — Phase A Handoff

Phases A–C are implemented as of 2026-10-01. The A/B sections below preserve historical
implementation and verification evidence; the dated Phase C completion section records
the current runtime and validation. Read
`docs/superpowers/specs/2026-09-27-adam-rs-planner-generalization-design.md` first for the full
design and phasing — this doc summarizes completion and distinguishes future scope.

## Historical Phase A/B implementation and verification

**Phase A (§2 static guard independence) is implemented on this branch.** The sheet now reports
any filter or conditional whose guard inputs can depend on a cell the guard governs:

- `Error::DependencyCycle { sites }` replaces the old plan-time `Error::FilterCycle`. Guard
  violations are reported by `Sheet::validate` and by `Sheet::propagate`, which validates before
  mutating propagation state.
- `adam-rs/src/sheet/dependency.rs` contains the private static dependency graph traversal.
  The graph includes method `input -> output` edges, filter guard `arg -> filtered` edges, and
  conditional guard `match subject -> governed output` edges.
- `Sheet::add_filter`, `Sheet::add_relationship`, and `Sheet::add_conditional` no longer run the
  guard check or roll back. They mark the current structure unvalidated; `Sheet::validate` builds
  the static graph once, computes SCCs once with an iterative Tarjan pass, scans guard edges in a
  deterministic order, and runs BFS only on the error path to reconstruct `sites`.
- The lazy structural-validation flag added for Phase A can be shared with Phase C's plan-reuse
  structural-change flag.
- `Sheet::add_conditional` no longer uses the old multi-method branch relationship
  contributing-cells rule. It accepts read-only multi-method branches and rejects only the static
  dependency cycles that violate rule 5.
- The planner treats a post-filter-edge cycle as an internal invariant: valid sheets should not
  reach that branch because the static guard check rejects them during construction.
- `adam-lang` calls `Sheet::validate` once after parsing the whole sheet and reports
  `DependencyCycle` with the governed cell declaration as the primary label and remaining cycle
  sites as secondary labels.

Rewritten invalid-sheet tests/examples:

- No bundled `begin` example required rewriting; `every_bundled_example_parses_successfully`
  passed unchanged.
- `adam-rs::sheet` tests that previously expected `InvalidConditional` or per-add
  `DependencyCycle` for branch guard cycles now assert `validate`/`propagate`
  `DependencyCycle`:
  - `validate_returns_dependency_cycle_for_multi_method_relationship_involving_match_cell`
  - `dependency_cycle_from_a_conditional_names_the_match_cell`
  - `add_conditional_returns_error_when_branch_rel_writes_a_cell_upstream_of_match_cell`
  - `add_conditional_returns_error_when_branch_rel_writes_a_cell_upstream_of_either_expr_input`
- `adam-rs/tests/dependency_guards.rs` adds public-API coverage for invalid sheets that close
  guard cycles through filters, relationships, and conditionals, plus accepted upstream/read-only
  cases, repeated validation failures, invalid propagation leaving values and `changed()` intact,
  and mutation after a successful validation.
- `adam-lang` diagnostic tests now expect whole-sheet validation to surface dependency cycles
  with the governed cell as primary.

**Phase B (§1 seedfill, #186) is implemented on this branch.** The planner/runtime now treat
self-referencing seed construction as part of the same deterministic release-state story:

- `Error::InvalidMethodOutputs { sites }` now documents and enforces the intended antichain rule:
  duplicate per-method outputs and identical/nested sibling output sets are rejected, while
  overlapping non-nested output sets remain valid.
- `adam-rs/src/planner/seed.rs`'s `build_seeds` consumes the planner's recorded elimination order
  and replays that elimination state when choosing each sibling seed method, rather than picking
  whichever declaration appears first in the relationship.
- Seed DFS now reports `Error::SeedCycle { sites }` for non-self sibling cycles instead of
  substituting any revisited cell's `source` value.
- When multiple sibling folds survive, they run in ascending order of strongest non-target input
  strength; equal-primary ties break by the selected method's ordered signature and then the
  relationship's full ordered method-signature sequence, not by relationship insertion order.
- If two equal-primary sibling folds are still structurally identical under that comparison,
  `build_seeds` returns `Error::Conflict` naming the ambiguous relationships instead of silently
  preserving adjacency order.
- `Sheet::propagate()` evaluates pre-plan methods and conditional expressions into private
  staged state, validates the active plan and seeds against that state, and commits only after
  validation succeeds. A seed-cycle failure exposes no staged writes and leaves `changed()` empty.

### 2026-09-28 Task 6 final-review follow-up

The final Task 6 review found one remaining determinism gap in `adam-rs/src/planner/seed.rs`:
two equal-primary siblings could share the same selected-method signature while still differing as
relationships, letting the stable sort preserve `cells[x].adj` insertion order. This follow-up
fix closes that gap by:

- sorting equal-primary siblings by a lexicographic key of
  `(strongest_input_strength, selected_method_signature, full_relationship_signature)`;
- rejecting the only remaining ambiguous shape — structurally identical equal-primary siblings —
  with `Error::Conflict` naming the participating relationships; and
- locking both cases with new `planner::seed` unit coverage.

Verification for this follow-up:

- `cargo fmt --all`
- `cargo test -p adam-rs --lib planner::` (48 passed)
- `cargo test -p adam-rs --test integration` (102 passed)

Full verification for this handoff completed with no `warning:` lines observed:

- `cargo fmt --all`
- `cargo build --workspace`
- `cargo test --workspace`
- `cargo test --doc --workspace`
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
- `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
- `cargo clippy -p begin --all-targets -- -D warnings`
- `$env:RUSTDOCFLAGS="-D warnings"; cargo doc --lib --no-deps --workspace`
- `cargo test -p begin --no-default-features`

## Deferred

- Analyzer validation for rule 3 ("Every method is reachable"). The runtime now rejects static
  guard cycles, but deeper method-reachability checks remain analyzer scope.
- The 2D containment scenario remains a later stress test rather than a Phase A fixture.
- `adam-lsp` diagnostics do not yet report `DependencyCycle` or other sheet-construction errors,
  because the LSP never builds a live `Sheet`. This gap predates Phase A and is tracked as #239.

## Phase B follow-up — selected guard prerequisites

Phase B now executes only the selected unconditional-plan cone needed to resolve conditional
guards. The reverse producer index includes selected method outputs and filter reclamps, and the
backward walk follows non-self method inputs and filter arguments in O(V + E) bookkeeping per
selected plan. Unrelated filters remain deferred until the final active plan, so their callbacks
observe branch-produced arguments regardless of relationship insertion order.

Staged methods, filters, and seed callbacks carry producer provenance. A final plan reuses a
pre-executed step only when its selected method, producer path, output classification, and seed
inputs are identical; otherwise propagation returns a conservative `Error::Conflict` with
implicated sites instead of publishing stale staged values. This describes the current
implementation's reuse boundary, not a public callback invocation guarantee.
Failed prerequisite and seed-cycle propagation remains transactional: prior live values are
preserved and `changed()` is empty for the failing call.

At this historical Phase B checkpoint, automatic plan reuse was not yet implemented.
Contract coverage preserves filtered-guard ordering, current prerequisite
values, inequality chains, direct self-reference, insertion-order independence, and rollback
after both seed-cycle and prerequisite-conflict failures.

### 2026-10-01 callback contract update

All methods, filters, conditional expressions, and requirements must be purely functional:
results depend only on explicit input values and immutable captured constants, equal inputs
produce equivalent values or errors, and callbacks have no externally observable side effects.
This includes range-bound evaluators and conditional equality functions. There is no guarantee
that any callback executes, or when or how often it executes. Rust's `Fn` bound does not enforce
this caller obligation. Self-referencing methods and conforming filters remain idempotent.

Public integration coverage now asserts values, diagnostics, and rollback rather than invocation
counts, and no method computes its result from an invocation counter. Earlier designs and
implementation plans that required callback-once behavior are superseded on that point. Runtime
execution was unchanged in this historical contract-only update. The completed Phase C
implementation below preserves values, diagnostics, and failure boundaries.

## 2026-10-01 Phase C completion — automatic prepared-plan reuse

**Phase C (§3, #152) is implemented.** No public runtime interface changes or caller-side
optimization decisions were added. The implementation retains one prepared unconditional
plan and one prepared main plan, not a cache of branch combinations.

- Prepared artifacts own producer provenance, selected guard cones, released-source
  membership, diagnostic-output membership, and structural seed recipes. Method and
  relationship signatures share the current structure's lifetime.
- Each phase has its own active set and released sources. Filter reclamps leave sources
  released; self-referencing claimants are not released. Eligible released-source writes
  retain expected O(1) membership bookkeeping. Claimed writes invalidate their phase;
  rejected writes do not invalidate. Actual structural mutation invalidates both phases,
  including partial `add_out` failure. Dropping final artifact owners can cost O(A) for
  total artifact size A; constant-time bookkeeping is not a claim about destruction.
- Source-prefix certificates use original slot-enumeration tie ordinals and cost O(C)
  for C cells only when pending strength changes require checking. Harmless demotions
  preserve reuse; a failed certificate conservatively replans. Each phase's active-set
  comparison takes expected O(R) for R active relationships.
- Structural sibling selection replaces complete elimination replay. Hits perform no
  global strength sort or complete seed-elimination replay. Current strength gates,
  local sibling sorting (O(S log S) comparisons for S siblings, plus signature comparison
  costs), fresh staged values, commit, and diagnostics remain.
- Seed state is fresh each propagation, with one intra-call evaluation cache shared by
  the two phases. Provenance compatibility checks, seed-cycle/conflict errors, and
  filter diagnostics remain. Callback purity gives no invocation guarantee.
- Validation fails before changed-state clearing; staged method/seed/prerequisite errors
  do not commit; requirement callback errors occur after commit. All propagation
  errors discard phase eligibility but retain the last successful display snapshot.
  Queries for newly added cells use that snapshot without reviving eligibility.
- Cross-propagation value memoization, dirty-cone execution, and incremental assignment
  repair are not implemented here. The analyzer, 2D stress scenario, and #239 listed
  above remain separate scope, not unfinished Phase C acceptance.

### Contract and operation-count coverage

Task 4's focused verification passed with no compiler warnings: `sheet::cached::`
24 tests, `planner::reuse::` 10 tests, `dependency_guards` 14 tests, integration
117 tests, adam-lang 446 unit tests and 19 doctests, and adam-lang-book 28 tests
(658 passed across these selected groups). These are contract checks, not speed
benchmarks. The full-suite evidence below is separate from historical A/B runs.

The eleven private differential tests compare automatic reuse with independent
forced-full-replan sheets over identical writes, normalizing error sites by declaration
order and comparing values, aspirations, selected methods, source/forced classifications,
changed sets, requirement membership, filter diagnostics, and failure recovery:

- `differential_identity_write_sequence`
- `differential_phase_local_releases_and_branch_writes`
- `differential_derived_boolean_guard`
- `differential_structural_mutations_and_rejected_operations`
- `differential_structural_cycles_preserve_completed_display`
- `differential_pure_failures_and_recovery`
- `differential_requirement_error_retains_previous_direction`
- `differential_recursive_and_sibling_seed_writes`
- `differential_filtered_guard_diagnostics`
- `differential_restaged_seed_provenance_and_recovery`
- `differential_seed_cycle_after_warm_hits`

The thirteen private operation-count/eligibility tests passed. Per-sheet test-only
counters measure actual pre/main planning, preparation, signature construction, and
certificate checks; they impose no callback-count contract:

- `identity_reuses_until_a_claimed_cell_is_written`
- `empty_sheet_reuses_without_pre_plan_or_certificate_check`
- `filtered_released_source_preserves_reuse`
- `newly_added_cell_is_a_source_in_previous_display_snapshot`
- `settled_strengths_do_not_repeat_certificate_scans`
- `branch_changes_replace_only_main_plan_and_share_signatures`
- `claimed_main_write_preserves_unconditional_cache`
- `crossed_unconditional_certificate_replans_only_unconditional_phase`
- `rejected_writes_and_attachments_preserve_reuse`
- `partially_failing_add_out_drops_structural_eligibility`
- `cached_method_failure_preserves_values_and_display_then_replans`
- `cached_requirement_failure_commits_but_does_not_publish_new_display`
- `actual_structural_mutations_rebuild_shared_artifacts`

Integration additions `inequality_chain_warm_rounds_preserve_current_aspirations`
and `conditional_seed_cycle_preserves_completed_queries_and_recovers` passed.
Certificate coverage includes all strict three-cell priority permutations, released
promotions, harmless/crossing demotions, equal-strength original-ordinal ties, filtered
sources, self-referencing outputs, and newly added cell membership. Structural seed
tests compare prepared selection with the complete-replay oracle over multiple cell
orders, covering absent targets, unique survivors, no survivors, ambiguous survivors,
lazy unused conflicts, current inputs, and structural tie-break errors.

### Full repository validation and review status

Task 5 ran the exact full repository suite successfully: `cargo fmt --all`,
`cargo build --workspace`, `cargo test --workspace` (2,194 passed, zero failed),
`cargo test --doc --workspace` (160 passed, zero failed), all five strict all-target
clippy commands (workspace excluding begin; begin and ez-adam each with and without
default features), and `cargo doc --workspace --no-deps --lib` with
`RUSTDOCFLAGS=-D warnings`. Build, tests, clippy, and rustdoc emitted zero compiler
warnings. Both test commands reported the same three pre-existing ignored doctests:
the DynamicArrayBuilder example and two ez-adam side-panel examples. They were not
silently counted as passing. `git --no-pager diff --check` passed; Git reported an
LF-to-CRLF conversion notice for `sheet.rs` and `planner.rs`, not compiler or
whitespace warnings.

The Task 5 session report retains exact commands, full output logs, and per-criterion
self-review evidence. Independent whole-branch review and any PR decision are separate;
this completion does not authorize pushing, merging, or closing #152.
