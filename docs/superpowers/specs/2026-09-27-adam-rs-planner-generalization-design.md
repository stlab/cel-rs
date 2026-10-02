# adam-rs planner generalization design

Date: 2026-09-27
Status: Phases A–C implemented; automatic prepared-plan reuse completed 2026-10-01
Issues: #186 (seedfill generalization), #152 (automatic plan reuse). #153 was closed as
not applicable (see §2); #18 was closed as a duplicate of #152.

This document supersedes, for seedfill, the "Superseded" note in
`2026-09-07-adam-rs-value-aware-self-ref-planning-design.md`: it documents seedfill as
it exists after PR #185 and settles the questions #186 raises.

## Validity rules

Every valid sheet satisfies the following. A sheet that violates any rule must produce an
error (at construction/parse time where the rule is statically checkable, otherwise from
`propagate()`) for violations the runtime can detect; no path may silently violate a
checked rule. Callback purity and idempotence are caller obligations, not runtime checks.

1. **Highest priority is a source.** The strongest cell is always a source, unless it is
   forced (a forced cell is not writable, so it has no priority to honor).
2. **Exactly one method per active relationship.** Every active relationship selects
   exactly one method per plan phase. Selection does not guarantee callback execution.
3. **Every method is reachable.** Each method is selected under some combination of cell
   strengths and enabled relationships. (Full checking is analyzer scope; see
   "Out of scope".)
4. **No iterative value solving.** Plan execution establishes selected-method results
   in dependency order, evaluating or reusing results as appropriate. The planner may
   search (and backtrack) over method assignments and source sets, but never
   re-evaluates methods to converge on values.
   The only cycles permitted in an executed plan are self-referencing methods
   (`x := f(x, ..)`), which must be idempotent. If every assignment that preserves the
   required sources is cyclic, that is an error. The diamond (`begin/examples/diamond.adm2`)
   is resolved this way: with `d > a > c > b`, `{d, a}` admits no acyclic assignment, so
   `a` is derived and `c` becomes a source (see
   `diamond_relationships_resolve_when_outer_cells_outrank_shared_cells`).
5. **Filters and conditions are independent of what they filter.** See §2.
6. **All callbacks are purely functional.** Method, filter, conditional-expression,
   requirement, range-bound, and conditional-equality callbacks depend only on explicit
   input values and immutable captured constants. Equal inputs produce equivalent
   values or errors. Callbacks have no externally observable side effects and do not
   read mutable external state. There is no guarantee that they execute, or when or how
   often they execute. The runtime may omit evaluation, reuse results, or reevaluate
   while preserving value and diagnostic semantics. Changing dependencies belong in
   input cells. This contract supersedes callback-count guarantees in earlier designs.

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

Historically, Phase B replaced first-declared sibling selection with elimination replay: start from
every method whose outputs contain `x`, then eliminate every other referenced cell in
the planner's complete release order. Elimination removes any candidate with a pure
output on that cell; it does not stop merely because one candidate remains. A pure
output is an output not also listed as an input.

The antichain rule remains a relationship-validity check. Seed selection separately
requires exactly one survivor: zero or multiple survivors produce `Error::Conflict`,
not a declaration-order fallback. A sibling with no `x`-producing method contributes
no seed.

**Implemented Phase C refinement:** complete replay is order-independent. A candidate survives
exactly when its pure-output set is a subset of `{x}`. §3 uses this equivalent
structural predicate to prepare sibling choices without replaying or sorting the
global cell order on each propagation. Strength-sensitive gating and sibling fold
order remain separate, dynamic decisions under §1.4.

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

`propagate()` automatically reuses eligible prepared phase plans and replans on a miss;
`propagate_without_replan()` no longer exists. Plan reuse is an internal optimization
with no API change. The implemented 2026-10-01 refinements
minimize repeated structural work while preserving Phase B's staged propagation and
the pure-callback contract in rule 6.

### 3.1 Prepared plans

`Sheet` retains one prepared main plan, keyed by its active relationship set, and one
prepared unconditional pre-plan. Do not retain a cache of branch combinations.
Preparation records:

- the selected assignment and topological execution order;
- forced cells and relationships, and O(1)-expected released-source membership;
- producer provenance and input/output classifications;
- the pre-plan's selected guard-prerequisite cone;
- self-referencing seed roots, claimant/sibling indices, and structural seed recipes;
- method-produced output cells needed for filter diagnostics; and
- the source-prefix certificate described below.

A released source is a cell not claimed by any selected method. `FilterReclamp`
does not change that classification; a self-referencing claimant is not released.
Each phase has its own source set: a conditional branch may claim a cell that the
unconditional pre-plan leaves released.

Build assignment-dependent metadata only when preparing a new assignment. Retain
stable method and relationship signatures with structural metadata instead of
reallocating them for each seed fold. Cache hits must not rebuild producer maps,
rediscover guard cones, rescan selected outputs for diagnostic membership, or clone
the complete prepared artifact merely to execute it.

### 3.2 Eligibility and source-prefix certificates

**Released-source promotion lemma.** For fixed structure and active relationships,
promoting a released source to the highest priority preserves the released set.
Feasibility is downward-closed: forbidding fewer cells never makes an assignment
harder. Induction over greedy release decisions preserves each other cell's
accept/reject decision. The assignment solver does not read strengths; with the same
final released set, its deterministic acyclic search returns the same assignment.
No matroid property is required.

**General certificate.** Retain the original release order and its accepted/rejected
decisions. For each originally rejected cell `r`, let `P(r)` be the released cells
that preceded it. A sufficient condition for reuse is that every member of `P(r)`
still outranks `r` under the current planner priority order. Use the release pass's
existing equal-strength tie-break as well; numeric strengths alone do not define
the ordering when they tie.

Proof: induct over the new priority order. An originally accepted cell remains
feasible because all accepted cells together form a feasible set. At an originally
rejected cell, the certificate ensures that all of `P(r)` have already been accepted;
adding `r` was infeasible with that prefix and remains infeasible with any superset.
Thus the accepted set is unchanged. A certificate failure is inconclusive, not proof
that the old assignment is wrong; conservatively replan.

Check the certificate in O(C) time and O(1) additional space by scanning the stored
original order and maintaining the weakest current priority among preceding accepted
cells. Each rejected cell must rank below that running minimum; an empty prefix
imposes no condition. Do not store a separate source-prefix set for every cell or
sort all cells to check eligibility.

Each successful write checks each phase's released-source membership in expected
O(1) time. A released-source promotion preserves an already valid certificate; a
write to a derived, forced, or self-referencing claimed cell invalidates that phase.
Rejected writes neither invalidate nor restore eligibility.

Internal strength post-processing also affects eligibility. A main-plan output can
be a pre-plan source, and a derived numeric strength may change without affecting
release decisions. Mark affected certificates for checking and use the certificate,
not blanket invalidation on every numeric change. Harmless demotions must not cause
an unconditional stabilization replan. Check only when intervening strength changes
are not already covered by the promotion lemma; otherwise retain validity.

Any actual structural mutation invalidates both prepared plans and their structural
metadata, including cells, relationships, filters, conditionals, requirements, and
output-cell construction. This also covers a mutator that changes structure before
returning an error. Static guard validation has a separate lifetime: `validate()`
cannot restore a stale prepared plan's eligibility.

The unconditional cache additionally requires its active subgraph to be unchanged.
The main cache requires the current conditional active set to equal its cached set.
Assignment eligibility is not evidence that values or seed folds are unchanged.

### 3.3 Structural seed recipes

Prepare sibling method choices using the complete-replay equivalence in §1.2:
consider methods that output the target, and retain those whose pure outputs are
a subset of that target alone. No target-producing candidate means no contribution;
otherwise zero or multiple survivors retain the existing conflict behavior.
This removes global elimination-order replay from the seed hot path. The release
order remains useful for certificates, not for per-propagation seed selection.

Recipes retain structural inputs and recursive dependencies, not old seed values.
Scope preparation and validation to the roots needed by each phase: Phase 1 requires
only guard-prerequisite seeds. Do not surface errors in unused recipes earlier than
the existing propagation algorithm would.

Each propagation applies current strengths to sibling gates and strongest-input
fold keys, preserves §1.4's structural tie-breaks and ambiguity errors, and establishes
seed values from current staged sources. Strength changes can reorder folds even
when the selected assignment remains reusable. Preserve the existing order of seed
dependency/cycle validation and strength gating; do not remove a dependency merely
because its eventual fold is gated out.

For this phase, evaluate recipes with fresh per-propagation seed state and one
`SeedEvaluationCache` shared between the pre-plan and main plan. Structural recipe
reuse is not permission to retain an old evaluated seed unconditionally.

### 3.4 Propagation and failure boundaries

Every call validates structure and clears changed-state at the existing boundary.
Phase 1 obtains an eligible prepared unconditional plan or prepares a new one, then
establishes its selected guard-prerequisite values in private staged state. Phase 2
establishes current conditional results and active relationships. Phase 3 obtains an
eligible prepared main plan or prepares a new one, validates prerequisite provenance,
establishes seeds, and executes the remaining selected plan into the same stage.

Prepared metadata replaces repeated structural discovery, not staged producer
compatibility checks. Preserve the current diagnosed conflicts when a pre-executed
method, filter, or seed result is incompatible with the final plan. These checks
protect values, not callback effects or invocation counts.

Commit, strength post-processing, reversion change tracking, requirement results, and
filter diagnostics retain their existing semantics. `changed()` continues to report
the outputs the current propagation publishes, even if a result is reused rather
than freshly evaluated. Public source, selected-method, forced-cell, and
forced-relationship queries reflect the completed propagation exactly as on the
full-replan path.

Publish refreshed prepared entries only after propagation succeeds, with certificates
checked or marked for checking against post-processed strengths. Failed propagation
conservatively invalidates reuse for both phases, including failures after commit.
Static validation still fails before clearing changed-state; staged method, seed,
and prerequisite failures still do not commit values; requirement evaluation retains
its existing post-commit error behavior. Do not move these boundaries to ease caching.

### 3.5 Evaluation reuse and scope

Purity permits omission, memoization, and reevaluation of method, filter, guard, and
requirement callbacks. There is no contract requiring full callback execution on
every propagation. Evaluation reuse nevertheless needs evidence that the callback
identity, current input values, seed context, and applicable diagnostic semantics are
unchanged. Assignment reuse alone does not provide that evidence; neither do raw cell
IDs, pointer equality to potentially mutable contents, or an empty `changed()` set.

The required Phase C refinement caches prepared plans and structural seed work while
retaining current intra-call evaluation reuse. Cross-propagation value memoization
and dirty-cone execution remain separate implementation scope, not forbidden behavior.
They need a design for immutable retained inputs/results, type-erased ownership without
new public `Clone` bounds, invalidation after staged source writes and strength-sensitive
seed changes, diagnostic refresh, and unchanged change-tracking/failure boundaries.
Do not introduce partial value caching without those proofs.

Likewise, incremental assignment repair after a derived-cell write is not part of this
refinement. A safe full replan remains the fallback.

### 3.6 Cost and alternatives

Writes to sources released by an eligible phase add expected O(1) membership
bookkeeping and retain that phase. Invalidating writes or structural mutations may
destroy O(A) prepared artifacts when their final owners are released; replacing
user values also has its own destruction cost. Eligible propagations skip
assignment solving, producer/guard-cone preparation, structural seed selection, and
global strength sorting. If certification is needed, it costs O(C), not an assignment
search. Comparing each phase's active set takes expected O(R) for R active
relationships; building current active sets and evaluating guards remain necessary.
Dynamic seed gates, local sibling-fold ordering, fresh staged execution, commit, and
diagnostics still contribute their existing costs. Local fold ordering can require
O(S log S) comparisons for S siblings, plus structural signature comparison costs;
eliminating a global sort does not eliminate these local sorts. Preparation retains
O(C + E) plan metadata plus structural recipe/signature
storage, where E counts the selected dependency edges.

This targets the expensive structural work first: the current release pass can make
O(C) acyclic-assignment searches, each exponential in the active relationship count
in the worst case. Do not claim a measured speedup without runtime benchmarks.

**Alternatives considered.** Caching only the assignment leaves repeated producer,
guard-cone, diagnostic-membership, and seed-selection work on every hit. Blanket
strength invalidation discards valid assignments after harmless post-processing.
Caching seed values without input validation can publish stale results; disabling
reuse for every self-referencing plan unnecessarily restricts the library. Prepared
plans, source-prefix certificates, and structural seed recipes remove that work
without changing the public execution API or assuming callback side effects.

### 3.7 Tests and acceptance criteria

- Compare automatic reuse against a test-only forced-full-replan path over identical
  write sequences. Compare effective and source values, selected methods, source and
  forced classifications, changed-cell sets, requirement/filter diagnostics, and errors.
- Test-only instrumentation establishes actual optimization: initial propagation
  prepares plans; eligible repeats and released-source writes skip planning and
  preparation; writes to derived, forced, or self-referencing claimed cells replan.
  Cache hits perform no global strength sort or complete seed-elimination replay.
  Instrumentation remains private, works with parallel tests, and imposes no public
  callback invocation guarantee.
- Compare certified reuse against full greedy release over reordered priorities,
  including released-source promotions, harmless demotions, equal-strength ties, and
  failing certificates that trigger conservative replanning. Verify that initial
  post-processing does not force a stabilization replan when its certificate passes.
- Compare structural sibling selection against complete elimination replay under
  multiple cell orders. Cover absent target-producing methods, one survivor, zero
  survivors, and ambiguous survivors.
- Exercise main/pre-plan eligibility independently, source-priority reordering,
  unchanged and flipped conditional branches, derived guard prerequisites, and
  main-plan strength demotion of unconditional pre-plan sources.
- Exercise successful structural mutators, including mutations followed by explicit
  `validate()`, partial structural mutation before an error, and rejected writes without
  accidental invalidation.
- Exercise filtered released sources and changing filter arguments, requirements that
  change validity, self-referencing seed values, strength-sensitive gates and fold order,
  requested seed roots, and correct results independent of callback invocation counts.
- Exercise seed-cycle, prerequisite-conflict, callback, and requirement failures,
  verifying existing transactional boundaries and successful recovery without stale
  cache reuse.

The algorithmic evaluation checked 59,259 certified reorderings of generated
downward-closed feasible families and 12,898 seed method/order cases. These are model
checks, not runtime benchmarks or substitutes for repository contract tests.

Update the public propagation contract and the dated phase handoff alongside implementation.
No UI change or caller-side optimization decision is needed.

### 2026-10-01 Phase C completion

Tasks 1–4 implement and test source certificates, structural seed recipes, independent
prepared phase caches, invalidation, and differential full-replan equivalence. Task 5
updates the contracts and completion record and runs the full repository checks.
The existing `2026-09-27-planner-generalization-phase-a-handoff.md` retains historical
A/B verification and records current Phase C coverage and validation separately.
No cross-propagation values are cached, no incremental assignment repair is introduced,
and callback purity supplies no invocation guarantee. Independent whole-branch review
through `dda39a6` confirmed compliance without actionable findings. Any PR decision
remains separate from this implementation completion.

## Out of scope

- Static reachability analysis of every method (rule 3) and earlier detection of
  runtime conflicts: future analyzer.
- 2D rectangle containment stress test (#186 follow-up).
