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

## Remaining

**Phase B (§1 seedfill, #186):**

- Enforce antichain output sets for sibling relationship methods.
- Select seed methods from the planner's elimination state instead of declaration order.
- Report non-self-reference seed dependency cycles as `Error::SeedCycle`.
- Fold sibling seed methods in ascending-strength order so the strongest influence is applied
  last.

**Phase C (§3 automatic plan reuse, #152):**

- Fold cached-plan reuse into `propagate()` automatically.
- Replace the explicit stale-plan fast path with planner-owned reuse that preserves current
  semantics while avoiding unnecessary replanning.
