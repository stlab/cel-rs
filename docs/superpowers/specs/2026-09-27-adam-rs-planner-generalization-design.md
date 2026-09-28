# adam-rs planner generalization design

Date: 2026-09-27
Status: Draft (sections 1–3 approved in brainstorming; open points resolved)
Issues: #186 (seedfill generalization), #152 (automatic plan reuse). #153 was closed as
not applicable (see §2); #18 was closed as a duplicate of #152.

This document supersedes, for seedfill, the "Superseded" note in
`2026-09-07-adam-rs-value-aware-self-ref-planning-design.md`: it documents seedfill as
it exists after PR #185 and settles the questions #186 raises.

## Validity rules

Every valid sheet satisfies the following. A sheet that violates any rule must produce an
error (at construction/parse time where the rule is statically checkable, otherwise from
`propagate()`); no path may silently violate a rule.

1. **Highest priority is a source.** The strongest cell is always a source, unless it is
   forced (a forced cell is not writable, so it has no priority to honor).
2. **Exactly one method per active relationship.** Every active relationship executes
   exactly one method per plan phase.
3. **Every method is reachable.** Each method is selected under some combination of cell
   strengths and enabled relationships. (Full checking is analyzer scope; see
   "Out of scope".)
4. **No iterative value solving.** Plan execution evaluates each selected method exactly
   once, in topological order. The planner may search (and backtrack) over method
   assignments and source sets, but never re-evaluates methods to converge on values.
   The only cycles permitted in an executed plan are self-referencing methods
   (`x := f(x, ..)`), which must be idempotent. If every assignment that preserves the
   required sources is cyclic, that is an error. The diamond (`begin/examples/diamond.adm2`)
   is resolved this way: with `d > a > c > b`, `{d, a}` admits no acyclic assignment, so
   `a` is derived and `c` becomes a source (see
   `diamond_relationships_resolve_when_outer_cells_outrank_shared_cells`).
5. **Filters and conditions are independent of what they filter.** See §2.

Backtracking to a lower-priority source assignment (the diamond case) is permitted as
long as rule 1 holds.

## §1 Seedfill (#186)

### 1.1 Method output sets form an antichain

`Sheet::add_relationship` rejects a relationship in which one method's output set is a
subset (or superset) of another's. Overlap is allowed. This strengthens today's
identical-output-set check (`Error::DuplicateMethodOutputs`), which is renamed or
generalized to report the offending pair.

Motivation: with a subset pair `{y}` ⊂ `{y, z}`, eliminating `y` leaves two candidate
methods, so method selection is ambiguous. The antichain rule admits relationships like
the old-Adam rotation relationship (six methods over overlapping two-cell output sets),
where eliminating one cell at a time always narrows to exactly one method.

### 1.2 Seed method selection follows elimination state

`compute_seed` currently picks the *first declared* method of a sibling relationship
that outputs `x` (`.position()`). Instead, it selects the method the planner's
cell-elimination would select for that relationship with `x` eliminated: starting from
the methods whose outputs contain `x`, eliminate the relationship's remaining cells in
the planner's release order (weakest first) until exactly one method remains. The
antichain rule guarantees the result is unique; if no method survives, that is an error
(the relationship cannot seed `x`), not a silent fallback.

### 1.3 Seed dependency cycles are errors

The `visiting` fallback (a cell already on the recursion stack contributes its `source`)
is removed. A seed dependency cycle other than direct self-reference is reported as a new
error (`Error::SeedCycle { sites }`, naming the cells and relationships on the cycle) from
`propagate()`. Under rule 4 such a cycle is never iterated.

With cycles excluded, seed dependencies form a DAG and the recursion terminates after
visiting each cell once; the result is deterministic given the fold order below.

### 1.4 Sibling fold order

When a cell has several sibling relationships, their seed methods are folded in
**ascending strength** of each sibling's strongest non-`x` input, so the strongest
influence is applied last. The order is independent of relationship insertion order;
ties cannot occur because cell strengths are distinct.

Example: cells `a=5, x=4, b=3, c=10` with strengths `c > b > a > x`; R0 `x ≤ c` claims
`x` via `x := min(x, c)`; siblings R1 `a ≤ x` (seed method `x := max(a, x)`) and R2
`x ≤ b` (seed method `x := min(x, b)`). Folding R1 (a) then R2 (b) gives
`min(max(5, 4), 3) = 3`, so `b`, the stronger cell, wins. (With `x > a`, R1 is excluded
by the existing strength gate and only R2 folds.)

### 1.5 Tests

- Antichain: subset/superset output pairs rejected; overlapping non-nested pairs (the
  rotation relationship) accepted.
- Seed selection: a sibling whose first-declared `x`-producing method is not the
  elimination-selected one seeds via the elimination-selected method.
- Seed cycle: a non-self-referencing seed cycle returns `Error::SeedCycle`.
- Fold order: the §1.4 example yields `x = 3` regardless of the insertion order
  of R1 and R2.
- Self-referencing diamond: spring-back and reversal coverage matching the `issue_182_*`
  chain tests.
- 2D rectangle containment is a later stress test (tracked on #186 follow-up, not part
  of this change).

## §2 Static filter and condition independence (replaces #153)

A filter's argument cells must not depend on the filtered cell, and a conditional's
match-subject cells must not depend on the relationships its branches enable, in the
**static dependency graph** over cells:

- every method of every relationship contributes `input → output` edges (a
  self-referencing input contributes no edge);
- every filter contributes *guard* edges `argument → filtered`;
- every conditional contributes *guard* edges `match cell → o` for every output `o` of
  every method of its branch and default relationships.

Invariant: no guard edge `g → t` lies on a cycle, i.e. `t` never reaches `g`. Cycles made
only of method edges are ordinary multi-way relationships and are allowed. The check is
plan-independent: deriving the filtered cell in some plan does not excuse the dependency,
because any multi-way relationship containing both cells can route the filtered value
into the argument.

- `Sheet::add_filter`, `Sheet::add_conditional`, and `Sheet::add_relationship` (whose new
  method edges may close a path for an existing guard) each re-check the invariant and
  return the new `Error::DependencyCycle { sites }` on violation, leaving the sheet
  unchanged. `sites` is the violating cycle in dependency order, guard target first:
  `[Cell(t), Relationship(r₁), Cell(c₁), …, Cell(g)]`.
- This subsumes `add_conditional`'s "branch relationship with more than one method
  touching a match-subject contributor" rule, which is removed.
- adam-lang already maps these calls' errors onto the declaration's span, so they are
  reported at parse time.

Consequences: a filter-induced cycle can no longer arise at plan time, so
`Error::FilterCycle` is removed and the planner's post-filter-edge topological sort
treats a cycle as an unreachable invariant violation. `release::resolve` needs no
filter-aware search, and the module/error docs that reference #153 are updated.

Tests: both invalid #153 examples (`z = x + y` with filter on `y` by `z`, and the
`R1{x,y,z} + R2{w,y}` variant) are rejected at `add_filter`; a filter whose argument is
genuinely upstream-independent is accepted; a relationship added after the filter that
creates the dependency is rejected; a conditional whose branch feeds its own match cell
is rejected.

## §3 Automatic plan reuse (#152)

`propagate()` replans every call today; `propagate_without_replan()` no longer exists.
Plan reuse is an internal optimization with no API change.

**Lemma.** The plan depends on structure, the active relationship set, and the released
source set (the assignment solver does not read strengths). Writing to a cell that is
already a released source moves it to the top of the strength order but leaves the
released set unchanged. Proof: feasibility is downward-closed (forbidding fewer cells
never makes an assignment harder), so by induction over the greedy order each other cell
receives the same accept/reject decision. No matroid property is required.

**Design.**

1. `Sheet` caches the last main plan with its seeds and active set, and the Phase 1
   pre-plan keyed by its active subgraph.
2. A validity flag is cleared by any structural mutation (cells, relationships, filters,
   conditionals, requirements) and by any `write`/strength change to a cell that was not
   a released source in the cached plan (derived, forced, or self-referencing claimant).
3. `propagate()` always runs Phases 0–2. It skips `planner::plan` and `build_seeds` only
   when the flag is set and the rebuilt active set equals the cached one. Phases 4–6
   always run.

**Tests.** Reuse equals forced full replan over write sequences; a derived-cell write
replans; a branch-flipping match-cell change replans; a structural edit invalidates;
optionally a debug-only replan-equality check.

## Out of scope

- Static reachability analysis of every method (rule 3) and earlier detection of
  runtime conflicts: future analyzer.
- 2D rectangle containment stress test (#186 follow-up).
