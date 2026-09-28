# Task 1 report — Select the guard prerequisite cone

## Outcome

`Sheet::propagate` now plans the complete unconditional assignment, indexes its selected producers, and stages only the steps needed to resolve all conditional guards. The selected subset preserves the planner's topological order. Static guard validation remains unchanged.

The seed builder now separates complete-plan claimant/active context from requested seed roots. Guard staging requests roots only from selected prerequisite methods, while seed dependencies continue to use the complete unconditional assignment.

## Changed files

- `adam-rs/src/sheet/prerequisites.rs` — private O(V + E) selected producer index and reverse prerequisite traversal for cell and expression guards.
- `adam-rs/src/sheet.rs` — replaced the all-method match-subgraph pre-plan with the complete unconditional plan and cone-only staging.
- `adam-rs/src/planner.rs` — made `PlanStep` hashable and re-exported the selected-root seed builder.
- `adam-rs/src/planner/seed.rs` — separated plan context from seed roots; existing callers retain the prior all-step behavior.
- `adam-rs/tests/integration.rs` — added bound-filter ordering/insertion-order regression coverage and filtered-guard callback coverage.
- `.superpowers/sdd/2026-09-28-selected-guard-prerequisites/task-1-report.md` — recorded requirements, RED/GREEN evidence, checks, and review notes.

## RED/GREEN evidence

Before implementation:

- `conditional_bound_filter_runs_after_branch` failed as expected: the unrelated filter left `x` at `8` instead of reclamping it to `3` after the selected branch changed `bound`.
- `filtered_guard_runs_before_branch_selection_once` failed as expected: the branch output remained `0` rather than reflecting filtered guard value `3`.

After implementation, both focused integration tests passed. The first exercises both possible insertion orders for the unconditional match producer and conditional bound producer, asserts `bound == 3`, `x == 3`, and exactly one filter callback. The second asserts a source-cell guard is filtered before branch selection and invokes that filter exactly once.

## Verification

- `cargo fmt --all` — passed.
- `cargo test -p adam-rs --test integration conditional_bound_filter_runs_after_branch -- --exact` — passed.
- `cargo test -p adam-rs --test integration filtered_guard_runs_before_branch_selection_once -- --exact` — passed.
- `cargo test -p adam-rs --test integration conditional_bound_filter` — passed.
- `cargo test -p adam-rs --test integration filtered_guard` — passed.
- `cargo test -p adam-rs --lib sheet::` — 152 passed.
- `cargo test -p adam-rs` — 255 unit tests, 14 dependency-guard tests, 106 integration tests, and 6 doctests passed.
- `cargo clippy -p adam-rs --all-targets -- -D warnings` — passed.
- `git diff --check` — passed.

## Algorithm research note

Producer lookup and prerequisite discovery use `HashMap`/`HashSet` from the Rust standard library. A stack-based graph traversal visits each needed cell and selected step once; filtering the original execution order retains its topological ordering. No graph package or all-method analysis is needed. Bookkeeping is O(V + E) in selected plan steps, cells, and selected producer-output/input/filter-argument edges.

## Self-review and concerns

The selected producer map is built from the planner's complete unconditional order, so multi-method relationships contribute only their chosen method and source filters contribute only when the planner emitted a reclamp. Method inputs that are also outputs are excluded as self-references; selected filter arguments are traversed. Expression guards contribute every input cell. Existing planning cycle diagnostics remain the only cycle check.

No Task 1 concerns remain.
