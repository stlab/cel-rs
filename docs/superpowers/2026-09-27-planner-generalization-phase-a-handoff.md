# Planner Generalization — Phase A Handoff

Status snapshot as of 2026-09-27, written for whoever picks up the remaining planner
generalization phases in a new conversation/context. Read
`docs/superpowers/specs/2026-09-27-adam-rs-planner-generalization-design.md` first for the full
design and phasing — this doc only summarizes what's done, what's deliberately deferred, and
what's left before the whole design is complete.

## Done

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

The all-method static guard-independence validation remains unchanged, and Phase C automatic plan
reuse remains deferred. Contract coverage preserves filtered-guard ordering, current prerequisite
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
execution is unchanged in this update; Phase C may optimize evaluation as well as planning when
it preserves current values, diagnostics, and failure boundaries.

## Remaining

**Phase C (§3 automatic plan reuse, #152):**

- Implement the refined §3 spec: cache prepared main/unconditional plans, including producer
  metadata, selected guard cones, source membership, diagnostic-output membership, and structural
  seed recipes.
- Use source-prefix certificates for internal strength changes instead of blanket invalidation;
  eligible released-source writes retain expected O(1) cache bookkeeping.
- Replace complete seed-elimination replay with its equivalent structural survivor predicate;
  retain current strength gates, local sibling-fold ordering, staged values, and errors.
- Verify eligible propagation skips planning, repeated preparation, and global strength sorting
  while matching full-replan values, diagnostics, change tracking, and failure boundaries.
- `propagate_without_replan()` is already gone; no caller-side optimization API is needed.
  Cross-propagation value memoization and incremental assignment repair remain separate scope,
  enabled by purity but not included in the prepared-plan refinement.
