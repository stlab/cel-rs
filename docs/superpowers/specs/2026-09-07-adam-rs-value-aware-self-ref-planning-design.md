# Value-Aware Source Selection for Self-Referencing Blocks (adam-rs)

**Date:** 2026-09-07
**Author:** Sean Parent (with Claude)
**Status:** Draft
**Fixes:** #182

## Problem

`release::resolve` (`adam-rs/src/planner/release.rs`) chooses which cells stay literal
sources by processing every cell in descending write-recency (strength) order and greedily
locking each one in as a source the moment *any* structurally valid, acyclic method
assignment still exists with it (and every previously locked cell) forbidden from being
claimed. This is provably optimal (lexicographically maximizes released-cell strength) for
relationships whose methods are ordinary functional equalities (`a * b = c`), because a
functional method's output is fully determined by its inputs: there is no independent
notion of "what this cell used to hold" that the choice of source could get wrong.

Self-referencing methods break this. A method `a := min(a, b)` reads `a`'s own pre-round
value as one input and can leave it unchanged (idempotent), so unlike a functional output, a
self-referencing cell has a real prior value (its `source`) that a bad source choice can
silently discard even though an equally structurally-valid alternative would have preserved
it.

### Reproduction

`adam-lang-book/book-src/examples/tutorial/inequality.adm2`:

```
sheet inequality {
    cell a = 10 filter clamp: 0..=100;
    cell b = 20 filter clamp: 0..=100;
    cell c = 30 filter clamp: 0..=100;

    relationship { a := min(a, b); b := max(a, b); }   // a <= b
    relationship { b := min(b, c); c := max(b, c); }   // b <= c
}
```

`write(a, 25)`, `propagate()`, `write(c, 40)`, `propagate()` produces `a=20, b=20, c=40`.
Expected `a=25, b=25, c=40`: `a=25` and `c=40` are jointly consistent (any `b ∈ [25, 40]`
satisfies `a ≤ b ≤ c`), so both edits should survive. Verified independently by adding the
two cases from the issue as `adam-rs/tests/integration.rs` tests and running them: the bug
case fails exactly as described (`a=20` instead of `25`), while the discriminating case
(`write(a, 11)`, `write(c, 0)`, `write(c, 100)`, expecting `a=11, b=20, c=100`, unchanged)
already passes today, confirming the fix must be value-aware rather than "always prefer the
higher-strength cell."

### Root cause

With 3 cells and 2 relationships here, exactly one cell can be a source (any two would force
both relationships to derive `b`, a double claim). `release::resolve` processes cells by
strength (`c > a > b`, since `c` was written last): it tries `c` first, finds a valid acyclic
assignment exists with `c` forced to remain a source (`b := min(b, c)`, `a := min(a, b)`),
and locks it in irrevocably. When `a` is processed next, forcing it to *also* be a source is
structurally infeasible (both relationships would need to claim `b`), so `a` is left derived,
and its written value is overwritten via `a := min(a, source-shadowed-b=20) = 20`.

But `{a}` alone is an equally valid source set: forcing only `a` gives
`b := max(a, b) = 25`, `c := max(b, c) = 40`. `c` ends up *derived*, but its derived value
happens to equal its own `source` (40), so nothing is actually lost. `{a}` strictly
dominates `{c}` here (zero stay violations vs. one), yet plain strength order can never
discover it, because `c`'s strength genuinely is higher than `a`'s: this is not a tie the
existing tie-break machinery can catch, and `resolve` never revisits a lock once made.

Because `Method`'s function is an opaque, type-erased closure (`adam-rs/src/relationship.rs`,
`MethodFn = Box<dyn Fn(&[&dyn Any]) -> Result<Vec<Box<dyn Any>>, anyhow::Error>>`), no
graph-only rule can predict whether a given source choice will preserve or discard a
self-referencing cell's stay. The only way to find out is to run the method.

### Relation to prior art

The existing `docs/superpowers/specs/2026-08-04-cyclic-constraint-planner-design.md` already
grounds `adam-rs`'s tearing of overlapping cyclic ("diamond") structures in Dulmage–Mendelsohn
decomposition and gammoid-matroid greedy release, and notes that Borning et al.'s
DeltaBlue/SkyBlue lineage (Freeman-Benson, Maloney & Borning, *The DeltaBlue Algorithm: An
Incremental Constraint Hierarchy Solver*, CACM 1990; Sannella, SkyBlue, 1994) faces a version
of this same problem but is documented as *incomplete*: SkyBlue can give up on a solvable
system depending on visitation order.

DeltaBlue's own answer is close in spirit to what this design needs. Every variable carries an
implicit weak "stay" constraint, and the solver's *walkabout strength* propagation compares
constraint hierarchies to decide which constraints to satisfy and which (weakest) to violate
when they conflict, precisely the "violate the weakest stays" framing this design adopts.
DeltaBlue can do this comparison symbolically, without executing anything, because its
constraint methods are ordinary invertible arithmetic relations with no self-reference: a
method's output is always fully determined by its inputs, exactly like `adam-rs`'s functional
relationships. `adam-rs`'s self-referencing methods are strictly more general (arbitrary
idempotent closures), so the same "weakest stay violated" criterion has to be evaluated by
concrete execution rather than symbolic strength propagation. This is the specific respect in
which `adam-rs`'s planner, following the same lineage as Järvi, Foust & Haveraaen,
*Specializing Planners for Hierarchical Multi-way Dataflow Constraint Systems* (GPCE 2014), and
Järvi's earlier *Property Models: From Incidental Algorithms to Reusable Components* (GPCE
2008, the direct academic description of the Adam property model this crate implements), has
to go beyond the classical multi-way constraint literature rather than merely apply it.

## Design

Scope the new mechanism to relationships containing at least one self-referencing method
(`planner::matching::pure_outputs` already identifies self-referencing outputs by exclusion).
A relationship with none is untouched, governed by exactly today's algorithm, so every
existing functional-diamond test keeps passing unchanged.

### 1. Partition `active` into connected components

Union-find over `active` relationships, joining any two that share a cell. Two relationships
sharing a cell are always in the same component regardless of whether either is
self-referencing, so a self-referencing chain overlapping a functional diamond (sharing a cut
vertex) is naturally one component: no separate cascade step is needed to handle that overlap,
since it falls directly out of using full connected components as the partition unit
(narrower than what was scoped in initial discussion, and simpler). The "cascade" the
2026-08-04 design describes turns out to be internal fixpoint behavior *within* one connected
component, which the existing exhaustive `solve_acyclic` search already re-derives from
scratch on each trial release, not a mechanism that ever needs to cross component boundaries.

- Complexity: O(R · α(R)) (union-find), R = active relationships.

### 2. Route each component

A component with no self-referencing method runs today's `resolve` loop exactly as it exists
today, scoped to that component's cells and relationships. Mathematically this is what
already happens today, since gammoid/matroid union over disjoint components decomposes into
independent per-component greedy; restating it per-component here doesn't change behavior, it
only identifies where the new mechanism does *not* apply.

A component with at least one self-referencing method applies the new value-aware selection
below to the *entire* component in one call, including any functional relationships inside it.

### 3. Enumerate candidate source-sets for a self-referencing component

Reuse `Assignment::solve_acyclic`'s existing exhaustive method-choice search
(`planner/matching.rs`), restricted to the component's relationships, but collect *every*
maximal acyclic assignment it can find for the component (today it stops at the first). Each
distinct assignment corresponds to a distinct choice of which cell(s) end up literal sources.

### 4. Score each candidate by concrete execution

For each candidate assignment, execute its self-referencing methods in the assignment's
topological order against current cell values, using a small local overlay
(`HashMap<CellId, Box<dyn Any>>`, scoped to the component, discarded once scored) so a later
method in the same candidate sees an earlier one's tentative output. This mirrors
`Sheet::execute_plan`'s existing input rule exactly: a self-referencing input always reads the
cell's real, unmutated `source` (never the overlay, never a same-round `derived`); every other
input reads the overlay if present, else the cell's real current `effective()` value. Nothing
on the real `Sheet` is mutated by this step.

For each self-referencing output, compare the tentative value against that cell's own
`source` using the existing `eq_fn` (already captured per-cell at `add_cell` time for exactly
this kind of type-erased comparison). A mismatch is a *violated stay* at that cell's
`strength`. Purely functional outputs in the same component are executed, since their results
may feed a downstream self-referencing method, but never scored: they have no stay to
violate, consistent with the existing invariant that value-independence is confined to
functional relationships.

A candidate whose execution returns `Err` from any method is scored as worst-case (sorted
above every strength value) rather than aborting selection. If every candidate for a
component errors, fall back to that component's best candidate under today's plain
strength-lexicographic source-set rule, so `plan()`/`propagate()` keep their existing error
contract (`Error::Conflict`, `Error::Cycle`, `Error::FilterCycle` only); the real error still
surfaces normally from `Sheet::execute_plan` once the chosen plan actually runs.

- Complexity: O(N · (M^R · R·K + E)) where N = candidates found in step 3, M = methods per
  relationship, R = relationships in the component, K = cells per method, E = self-referencing
  outputs scored per candidate. This is the same exponential-in-R worst case `solve_acyclic`
  already has, now paid once per candidate rather than once total; acceptable at `adam-rs`'s
  target scale (tens of cells). See Future Work.

### 5. Select

Pick the candidate whose violated-stay strengths, sorted descending, are lexicographically
smallest (fewest and weakest violations wins). Break remaining ties (including the
all-candidates-errored fallback, and the case where a component has no self-referencing
methods to violate at all) using today's existing rule: lexicographically largest sorted
source-set strengths. This tie-break is what makes a no-self-reference component's outcome
identical to running it through the new machinery: the violated-stay key is always empty
there, so selection degenerates to exactly today's criterion. The fast path in step 2 exists
purely for performance, not because the algorithms disagree.

### Module layout

New `adam-rs/src/planner/stay.rs`, following the existing `planner/{digraph,matching,scc}.rs`
split:

- `has_self_reference(rel: &RelationshipData) -> bool`
- `partition_components(relationships, active) -> Vec<HashSet<RelationshipId>>`
- `resolve_component(cells, relationships, component, released_elsewhere) -> Assignment`, the
  enumerate/execute/score/select pipeline (steps 3-5) for one self-referencing component.

`release::resolve` becomes: partition, run today's loop per non-self-referencing component
(or just once over their union, since they never interact), call `stay::resolve_component`
per self-referencing component, merge the resulting `Assignment`s. `Plan`, `Error`, and the
public `Sheet` API are unchanged: this is entirely internal to `release::resolve`.

## Alternatives considered

The issue's Option 2 models inequalities as filters instead of self-referencing
relationships. Filters are already value-aware and side-step the combinatorial planning
problem entirely, since a filter has no method choice to plan, just one deterministic reclamp
step. But a filter is a fixed, one-directional correction: `b`'s filter can clamp `b` into
`[a, c]`, while `a` and `c` can never be pushed around by a stronger edit to `b`, unlike a
genuine multi-way relationship where whichever cell was edited most recently propagates
outward. That asymmetry is the entire point of a *multi-way* constraint system, the property
this crate is named for and built around (`docs/VISION.md`), so routing around the bug by
demoting inequalities to one-directional filters would quietly give up a core capability
rather than fix it. Rejected; kept as the documented alternative for callers who explicitly
want one-directional bounds (already fully supported today, unchanged by this design).

A narrow fix scoped to exactly the chain shape in the issue was also considered and rejected,
per this project's Library-First Design principle: `adam-rs` is a reusable component, not a
harness for one tutorial example, and the connected-component-based design above is no more
complex to reason about than a shape-specific tie-break while covering arbitrary
self-referencing structures (see Testing below for a case beyond the 2-relationship chain).

## Sheet invariants (new `adam-rs/src/lib.rs` section)

Add an `## Invariants` section to the crate-level doc comment, gathering what today is
scattered across code comments and design docs.

Enforced by code:

- Every relationship's methods share the same `inputs ∪ outputs` cell set
  (`Sheet::add_relationship` validation; `Error::MismatchedMethodCells`).
- No two relationships may claim the same cell as a pure output in one round
  (`planner::matching::Assignment`; `Error::Conflict` when infeasible).
- The selected methods' induced dependency digraph is acyclic before execution
  (`planner::digraph::is_acyclic`; `Error::Cycle`/`Error::FilterCycle` when not).
- A self-referencing input always reads the pre-round `source` value, never a same-round
  `derived` value (`Sheet::execute_plan`).
- `source` is written only by `write()`/`add_cell`, never by method or filter execution; the
  entire `source`/`derived` shadow-state split exists to uphold this.
- An `Out`-kind cell can never be `write()`-ed or claimed as another method's output
  (`Error::InvalidCellKind`).

Enforced only by convention (caller contract, not checked by the runtime):

- A self-referencing method must be idempotent: applying it twice to the same inputs must
  produce the same result as applying it once.
- A filter must be a pure, conforming function of its cell's value and its argument cells'
  values.
- Iteration order used to break ties among equal-strength cells is not stable API and must
  not be relied on by callers.

The source-capture property, amended:

Capturing every cell's `source` value and reapplying the highest-strength sources reconstructs
the sheet's *current* state, but not what a subsequent edit will do, since no method-selection
state is captured, only values. This property does not hold as stated for a self-referencing
cell: because which method a relationship selects is chosen by strength, but a strictly weaker
self-referenced cell can still contribute to that method's output (issue #182), the *source*
value alone is not sufficient to reconstruct even the current state for such a cell.

Amended: for a self-referencing cell, the value that must be captured to reconstruct current
state is its *derived* value (`Sheet::read`'s effective value), not its raw `source`. Because a
self-referencing method is required to be idempotent, replaying that derived value through the
same method again is guaranteed to reproduce it: the capture is stable under replay even
though it discards the cell's original written value.

## Testing

Contract-derived, per this repo's convention.

New in `adam-rs/tests/integration.rs` (needs interleaved `write`/`propagate` calls per step,
unlike the existing batched `self_ref_le_chain`):

- `issue_182_inequality_chain_preserves_a_consistent_edit`: the bug case,
  `write(a,25)`, propagate, `write(c,40)`, propagate, expecting `a=25, b=25, c=40`.
- `issue_182_inequality_chain_discriminating_case_unchanged`: the case that must stay
  correct, `write(a,11)`, `write(c,0)`, `write(c,100)`, expecting `a=11, b=20, c=100`.
- `value_aware_selection_generalizes_beyond_a_two_relationship_chain`: a 3-relationship
  self-referencing chain (`a<=b<=c<=d`) where the naive highest-strength-first choice would
  again pick the wrong single source, confirming the fix isn't shape-specific to 2
  relationships.
- `self_referencing_component_overlapping_a_functional_diamond_scores_only_the_stay`: a
  self-referencing pair sharing a cell with a functional (`a*b=c`-style) relationship in the
  same connected component, asserting the functional cell's outcome is governed by the
  strength-lexicographic tie-break (unaffected by value comparison) while the self-referencing
  part is chosen by violated-stay strength.
- `candidate_execution_error_is_scored_worst_case_not_propagated`: a self-referencing method
  that returns `Err` for one candidate's inputs, asserting `plan()` still succeeds by picking a
  different candidate, and that a candidate error never appears as `plan()`'s own error.

Unchanged and must keep passing exactly as-is (no self-reference, or single-relationship
self-reference; both take the fast path from step 2): `strength_prefers_the_higher_strength_cell_as_source`,
`diamond_collision_pattern_resolves_instead_of_failing`, `self_ref_le_chain`,
`self_ref_pressure_persists_without_rewriting_anchor`, and every existing `planner.rs`/
`matching.rs`/`digraph.rs` unit test.

## Non-goals

- No change to `Method`, `Filter`, `Plan`, or any public `Sheet` API: entirely internal to
  `release::resolve`.
- No change to how functional (non-self-referencing) relationships are planned.
- No incremental re-planning across `propagate()` calls: out of scope here exactly as it is
  for the rest of the planner (see 2026-08-04 design's Future Work).

## Future Work

- If profiling on a large, heavily self-referencing sheet ever shows the per-candidate
  concrete-execution cost mattering, the enumeration in step 3 could prune candidates whose
  structural source-set strength is already dominated by a better-scoring one found so far,
  rather than scoring every structurally valid candidate to completion. Not pursued now,
  since `adam-rs` targets UI-scale property models.
- Surfacing *which* candidate was rejected and why (beyond the existing `is_source`/`is_forced`
  accessors) to the `begin` Inspector UI, if there's a concrete need to explain a tear
  decision to an end user.
