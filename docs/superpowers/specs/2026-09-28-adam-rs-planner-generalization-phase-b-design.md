# Adam-RS Planner Generalization — Phase B Design

Date: 2026-09-28
Status: Approved for implementation
Scope: Seedfill generalization from `docs/superpowers/specs/2026-09-27-adam-rs-planner-generalization-design.md` §1

## Goal

Generalize seedfill so sibling relationships are selected and folded from planner state
rather than declaration order, reject ambiguous nested method output sets, and report
non-self-referencing seed dependency cycles instead of silently substituting source values.
Phase C automatic plan reuse is deliberately excluded.

## Architecture

### Relationship output validation

`Sheet::add_relationship` validates every pair of methods in a relationship. A pair is
invalid when one method's output set is equal to or a strict subset of the other's output
set. The error reports both methods and the relevant output cells. Output sets that
overlap without containment remain valid, including the six-method rotation relationship.
The existing output-validation error is generalized and its `Display`, public contract,
`adam-lang` labels, and tests describe duplicate or nested output sets.

The check is construction-time and does not mutate the sheet on failure. It remains
independent of method declaration order.

### Planner elimination state

The release pass already evaluates cells in deterministic strength order and repeatedly
solves the active relationship assignment while adding released cells. It will expose the
elimination order/state needed by seedfill alongside the chosen assignment.

For a sibling relationship and target cell `x`, seed selection starts with methods whose
output sets contain `x`. It eliminates the relationship's other referenced cells in the
planner's release order, retaining only methods compatible with each elimination. The
antichain validation rule guarantees that exactly one method remains whenever the
relationship can seed `x`; no declaration-order fallback is permitted. If no method
survives, propagation returns a planner error rather than silently omitting that sibling.

### Seed dependency graph and cycle errors

`build_seeds` becomes fallible. Its recursive computation records the active
cell-to-sibling-relationship-to-input path. Encountering a cell already on the current
path produces `Error::SeedCycle { sites }`, with the participating `Cell` and
`Relationship` sites in deterministic traversal order. Direct self-reference remains
valid and is not reported as a seed cycle because the target cell's own claimant is
excluded from sibling dependencies.

The recursion still memoizes completed seeds and evaluates each selected method once.
Method failures, wrong output counts, and output type mismatches continue to contribute no
seed value under existing behavior; only a structural non-self dependency cycle becomes
an error.

### Sibling fold ordering

For each target `x`, sibling relationships are sorted in ascending strength of their
strongest non-`x` input. The fold applies weaker influences first and the strongest
influence last. Equal-primary ties are broken first by the selected sibling method's
ordered input/output signature and then by the entire relationship's ordered
method-signature sequence, so two different relationships with the same selected
signature still sort deterministically without falling back to adjacency order. If two
equal-primary siblings are still structurally identical under that comparison,
propagation reports `Error::Conflict` naming both relationships instead of folding in
arbitrary order. The current explicit-strength gate remains in force and is evaluated
for the selected sibling method.

## Data flow and error handling

`Sheet::propagate` obtains the plan and then calls fallible `build_seeds`. A seed-cycle
error is returned before execution mutates propagation state. The error uses the same
`Error::sites()` mechanism as dependency and planner cycles, and its `Display` text
identifies a seed dependency cycle.

The release state is internal; no public planner type or propagation API changes beyond
the generalized output-validation error and the new `Error::SeedCycle` variant.

## Tests

Add or update contract-derived tests for:

1. Strict subset and superset method output pairs are rejected, with both method sites
   reported.
2. Identical output sets remain rejected, while overlapping non-nested output sets are
   accepted.
3. A sibling whose first declared `x`-producing method is not the elimination-selected
   method seeds through the selected method.
4. A non-self-referencing seed dependency cycle returns `Error::SeedCycle` with the
   expected cycle sites.
5. Sibling fold results are independent of relationship insertion order and follow
   ascending-strength order.
6. Existing self-referencing chain, spring-back, reversal, and diamond behavior remains
   correct.

Update related module, public API, and language-diagnostic documentation so the contracts
match the new error and output-set rules.

## Non-goals

- Automatic plan reuse and cache invalidation from phase C.
- Static method reachability analysis.
- The two-dimensional rectangle-containment stress test.
