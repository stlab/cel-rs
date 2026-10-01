# adam-rs planner generalization design

Date: 2026-09-27
Status: Phases A–B implemented; Phase C design refined 2026-10-01, implementation pending
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
equal-primary ties are broken first by the selected method's ordered signature and then
by the relationship's full ordered method-signature sequence. If two equal-primary
siblings are still structurally identical under that comparison, propagation reports
`Error::Conflict` naming both relationships rather than folding in arbitrary order.

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
  method edges may close a path for an existing guard) no longer re-check the invariant
  or roll back. They mark the structure as not yet validated. `Sheet::validate` and
  `Sheet::propagate` report the new `Error::DependencyCycle { sites }` on violation.
  `sites` is the violating cycle in dependency order, guard target first:
  `[Cell(t), Relationship(r₁), Cell(c₁), …, Cell(g)]`.
- Validation builds the static graph once, computes strongly connected components once,
  then checks each guard edge: `g → t` violates iff `g == t` or both endpoints share an
  SCC. Only the error path runs a BFS to recover the shortest `t → ... → g` path for
  labels. This avoids the previous superlinear sheet-construction cost from running a
  full reachability search after every guard-affecting mutator.
- This subsumes `add_conditional`'s "branch relationship with more than one method
  touching a match-subject contributor" rule, which is removed.
- adam-lang validates once after parsing the whole sheet and reports the first cycle with
  the governed cell declaration as the primary label and the remaining sites as
  secondary labels.

Consequences: a filter-induced cycle can no longer arise at plan time, so
`Error::FilterCycle` is removed and the planner's post-filter-edge topological sort
treats a cycle as an unreachable invariant violation. `release::resolve` needs no
filter-aware search, and the module/error docs that reference #153 are updated.

Tests: both invalid #153 examples (`z = x + y` with filter on `y` by `z`, and the
`R1{x,y,z} + R2{w,y}` variant) are accepted by their mutators and rejected by
`validate`/`propagate`; a filter whose argument is genuinely upstream-independent is
accepted; a relationship added after the filter that creates the dependency is reported
by the next validation; a conditional whose branch feeds its own match cell is reported
by validation.

## §3 Automatic plan reuse (#152)

`propagate()` replans every call today; `propagate_without_replan()` no longer exists.
Plan reuse is an internal optimization with no API change. The 2026-10-01 refinement
below incorporates Phase B's staged execution and strength-sensitive seed construction.

**Lemma.** The plan depends on structure, the active relationship set, and the released
source set (the assignment solver does not read strengths). Writing to a cell that is
already a released source moves it to the top of the strength order but leaves the
released set unchanged. Proof: feasibility is downward-closed (forbidding fewer cells
never makes an assignment harder), so by induction over the greedy order each other cell
receives the same accept/reject decision. No matroid property is required.

**Design.**

1. `Sheet` caches the main selected plan and its active relationship set, and separately
   caches the Phase 1 unconditional pre-plan. Each cache records its own released-source
   set: cells not claimed by a selected method. A filtered released source remains a
   released source even though a `FilterReclamp` step shadows its value. A
   self-referencing claimant is not a released source.
2. Any successful structural mutation (cells, relationships, filters, conditionals,
   requirements, or output-cell construction) invalidates both caches. A successful
   `write` invalidates each cache independently when its target was not a released
   source in that cache. This distinction matters when a conditional branch claims a
   cell that the unconditional pre-plan leaves released. Rejected writes do not
   invalidate a cache. Structural guard-validation state and plan-cache validity have
   separate lifetimes: `validate()` must not restore a stale plan's validity.
   Internal strength post-processing must also preserve cache eligibility. A main-plan
   output can be a released source in the unconditional pre-plan, so main-plan demotion
   is not automatically harmless to the pre-plan. Invalidate a phase when an actual
   strength change falls outside the proved released-source promotion rule, unless a
   separate proof establishes that its assignment remains valid. A newly computed plan
   can therefore require a stabilization replan after its first strength post-processing.
3. `propagate()` always validates structure, clears changed-state, and runs staged
   Phases 0–2. Phase 1 reuses an eligible unconditional assignment, but still executes
   its selected guard-prerequisite cone. Phase 2 evaluates conditional callbacks and
   rebuilds the active set every call. Phase 3 reuses an eligible main assignment only
   when the rebuilt active set equals its cached active set; otherwise it replans.
   Retain only the latest assignment for each phase, not a cache of branch combinations.
4. Cache assignments, not seed values or callback results across propagation calls.
   Rebuild seeds from current staged source values every call, using one fresh
   `SeedEvaluationCache` shared only between phases of that call. Seed sibling selection
   replays the current deterministic strength order, including on a reused assignment;
   refresh that order without rerunning the assignment solver. Current strengths also
   determine the sibling strength gate and fold order. The released-source lemma
   justifies assignment reuse, not reuse of old elimination order or seed evaluations.
5. Preserve Phase B's staged producer compatibility checks: a guard-prerequisite method,
   filter, or seed callback already evaluated in this call is reusable only under the
   existing provenance contract. Cache hits must not bypass seed-cycle checks, replay
   stateful callbacks, publish stale staged values, or suppress prerequisite conflicts.
6. Phases 4–6 always run: commit, strength post-processing, reversion change tracking,
   requirement evaluation, and filter diagnostics retain their existing behavior.
   Public selected-method, source, forced-cell, and forced-relationship queries reflect
   the completed propagation exactly as on the full-replan path.
7. Publish refreshed cache entries only after propagation succeeds. A failed propagation
   conservatively invalidates reuse for both phases, including failures after commit.
   Preserve the existing failure boundaries: static validation fails before clearing
   changed-state; staged method, seed, and prerequisite failures do not commit values;
   requirement evaluation retains its existing post-commit error behavior. Never
   change these boundaries merely to make caching easier.

**Alternatives considered.** Retaining seed results, as the original wording suggested,
can reuse stale values or obsolete strength-sensitive folds. Disabling reuse for all
self-referencing plans avoids that risk but unnecessarily restricts the general library
optimization. Reusing assignments while rebuilding seeds preserves the general behavior
without introducing a new public execution mode.

**Tests.**

- Compare automatic reuse against a test-only forced-full-replan path over identical
  write sequences. Compare effective and source values, selected methods, source and
  forced classifications, changed-cell sets, requirement/filter diagnostics, and errors.
- Test-only planner invocation counters establish actual optimization: initial
  propagation plans, eligible repeats and released-source writes skip planning, and
  writes to derived, forced, or self-referencing claimed cells replan. Instrumentation
  remains private and must not interfere with parallel tests. Establish eligibility
  after any required strength stabilization rather than assuming the first plan survives
  its own post-processing.
- Exercise main/pre-plan eligibility independently, source-priority reordering,
  unchanged and flipped conditional branches, derived guard prerequisites, and
  main-plan strength demotion of unconditional pre-plan sources.
- Exercise successful structural mutators, including mutations followed by explicit
  `validate()`, and rejected writes without accidental invalidation.
- Exercise filtered released sources and changing filter arguments, requirements that
  change validity, self-referencing seed values, strength-sensitive sibling selection
  and fold order, and callback counts within and across propagation calls.
- Exercise seed-cycle, prerequisite-conflict, callback, and requirement failures,
  verifying existing transactional boundaries and successful recovery without stale
  cache reuse.

Update the public propagation contract and the dated phase handoff alongside implementation.
No UI change or caller-side optimization decision is needed.

## Out of scope

- Static reachability analysis of every method (rule 3) and earlier detection of
  runtime conflicts: future analyzer.
- 2D rectangle containment stress test (#186 follow-up).
