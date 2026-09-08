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

**Superseded by Part 2 below**: implementation and further testing (against a live sheet
with interleaved `write`/`propagate` rounds, not just single-round `resolve_component` unit
tests) found that Part 1 alone is necessary but not sufficient — a second, deeper bug
surfaces across *successive* rounds, and fixing it does, after all, touch `Plan` and
`Sheet::propagate`'s Phase 0. See Part 2.

## Part 2: Self-referencing input across propagate() rounds

**Date added:** 2026-09-08
**Status:** Draft — found during PR review of Part 1's implementation, before merge.

### Second bug: a continuously-self-referenced cell loses its settled value

Reproduction, continuing directly from Part 1's example (same sheet, same first two steps):

```
write(a, 25), propagate()   // a=25, b=25, c=30 -- Part 1's fix, correct
write(c, 24), propagate()   // actual: a=20, b=20, c=24 -- still wrong
```

Expected: `a=24, b=24, c=24`. `c=24` is now *below* `a`'s stay (25), so `a<=b<=c` cannot hold
with `a` left at 25 — some violation of `a`'s stay is unavoidable. But the *minimal* correction
is `a=24` (pulled down exactly to meet `c`), not `a=20`.

After the first round, `b` is not a literal source: `rel1` claimed it (`b := max(a, b)`,
self-referencing on `b`), writing `25` into `b.derived` and leaving `b.source` at its original
`add_cell(20)` declaration, untouched (per the shadow-state split). At the *start* of the
second round, `Sheet::propagate`'s Phase 0 unconditionally resets every cell's `derived` to
`None` before planning — including `b`'s. By the time `stay::resolve_component` (or, on the
winning candidate, `Sheet::execute_plan`) evaluates `rel2`'s self-referencing method on `b`
(`b := min(b, c)`), the self-referencing-input rule reads `cells[b].source`, which is `20`:
`b`'s only real information about its own recent state — `derived = Some(25)` — was already
discarded by Phase 0, before planning ever got a chance to use it. `b := min(20, 24) = 20`,
then `a := min(25, 20) = 20`.

This is not a bug introduced by Part 1's `score_candidate`; it is a pre-existing gap in
`Sheet::propagate`'s Phase 0 / `execute_plan`'s self-referencing-input rule, which Part 1
inherited unchanged. It did not surface in Part 1's own test suite because every test there
exercises only a *single* value-aware round (or rounds where the numbers happen not to expose
it — `max(a, ghost_b)` and `max(a, true_b)` coincide whenever `a` dominates both, which was
true in every Part 1 scenario).

### Why "always prefer the last derived value" is also wrong

The obvious-looking fix — self-referencing input reads the cell's last *derived* value if it
has one, falling back to `source` — breaks the existing, tested
`self_ref_pressure_persists_without_rewriting_anchor` (`adam-rs/tests/integration.rs`):
`a := min(a, b)` is *repeatedly* self-referencing across many rounds where only `b` changes.
That test asserts `a` must keep reading its own frozen `source` every round specifically so it
can spring back to its original written value once `b`'s downward pressure relaxes; reading
`a`'s own last-*derived* (clamped) value instead would reintroduce the exact "shrinking
accumulator" bug the 2026-08-02 `source`/`derived` split was built to prevent — `a` would
ratchet down and never recover.

The two cases look identical at the mechanism level (a self-referencing input choosing between
`source` and a prior derived value) but need opposite answers. The distinguishing factor is
**not** whether the cell has ever been explicitly `write()`-ed (both `a` and `b` have — `b` via
its `add_cell` declaration, which sets `source` and bumps strength exactly like `write()`
does). It is a **relative-strength comparison against what the choice affects downstream**:

- In the anchor-pressure test, `b` (the cell applying pressure) is written on every round and
  so is *always* strictly stronger than `a`. Preferring `source` (the strength-dominant
  choice) is unconditionally right there; there is never a competing cell whose stay would be
  better served by the other option.
- In the inequality-chain case, `rel2`'s self-referencing choice for `b` is not really "about"
  `b` in isolation — `b`'s value flows into `rel1`, which self-references `a`. Reading `b`'s
  `source` (20) forces `a` down to 20, violating `a`'s stay (strength from `write(a, 25)`, the
  second-most-recent write). Reading `b`'s prior *derived* value (25, `b`'s actual settled
  state as of the end of the previous round) instead lets `rel1` derive `a := min(25, 24) =
  24` — the *same* stay-violation the chain cannot avoid regardless (since `c=24 < a=25`
  structurally forces some correction), but no worse than necessary, and it does not
  additionally clobber `a` down to `b`'s meaningless ghost floor.

So the choice for a *given* self-referencing cell cannot be decided locally, by inspecting only
that cell's own value: it depends on which relationship is asking. The distinguishing fact
turns out to be purely structural, not a value comparison at all, and needs no backtracking
search over the two options:

- In the anchor-pressure test, `a` is self-referenced by the *same* relationship every round
  (there is only one relationship touching `a`). "Same relationship as last time" always
  resolves to `source`, which is exactly the spring-back behavior that test requires.
- In the inequality-chain case, `b`'s prior derived value (25) was produced by `rel1`
  (`b := max(a, b)`), but the self-reference being resolved this round is `rel2`'s
  (`b := min(b, c)`) -- a *different* relationship. Reading the prior-derived value precisely
  here, instead of `source`, is what lets `rel1` re-derive `a` to 24 instead of 20.

So the rule is: a self-referencing input for cell `C`, read by relationship `R`'s method,
reads `prior_derived[C]` when a prior-derived value exists for `C` *and* it was produced by a
relationship other than `R`; otherwise it reads `cells[C].source`. This is a deterministic O(1)
lookup, decided once the method doing the reading is known -- no value comparison, no
backtracking, and no additional exponential factor on top of Part 1's candidate enumeration.

### Design

**1. `CellData` gains `derived_by: Option<RelationshipId>`,** recording which relationship
produced the cell's current `derived` value -- but only when that output is *genuinely*
self-referencing. A cell shadowed only because its relationship is a currently-active
conditional branch (not self-referencing) never gets `derived_by` set, so a stale value from a
now-inactive branch is never mistaken for a self-referencing producer (see item 6 below).

**2. `Sheet::propagate`'s Phase 0 stops discarding `derived` outright -- it relocates the
genuinely self-referencing part of it.**

Phase 0 still computes `previously_derived` (every cell with a live `derived`, feeding Phase
5's existing revert-tracking) exactly as before, by an independent scan of
`cell.derived.is_some()` -- this list must stay untouched by the new mechanism, since it also
covers conditional-only shadowing, which `derived_by` deliberately does not track. Separately,
draining `derived` also drains `derived_by`, and where *both* are `Some` the pair is kept in a
per-round snapshot:

```rust
let mut prior_derived: HashMap<CellId, (RelationshipId, Box<dyn Any>)> = HashMap::new();
for (id, cell) in self.cells.iter_mut() {
    let derived = cell.derived.take();
    let derived_by = cell.derived_by.take();
    if let (Some(v), Some(rel_id)) = (derived, derived_by) {
        prior_derived.insert(id, (rel_id, v));
    }
}
```

`cell.derived` ends up `None` either way (same observable state for `effective()`/`read()`
during planning), but a genuinely self-referencing value is no longer lost -- it is available,
for this round only, as `prior_derived`, threaded through `plan()` into
`stay::resolve_component`/`score_candidate` and back out to `execute_plan`.

**3. `stay::self_reference` replaces the unconditional "self-referencing input always reads
`source`" rule with the same-vs-different-relationship lookup:**

```rust
fn self_reference<'a>(
    output_id: CellId,
    rel_id: RelationshipId,
    cells: &'a SlotMap<CellId, CellData>,
    prior_derived: &'a HashMap<CellId, (RelationshipId, Box<dyn Any>)>,
) -> &'a dyn Any {
    match prior_derived.get(&output_id) {
        Some((derived_by, prior)) if *derived_by != rel_id => prior.as_ref(),
        _ => cells[output_id].source.as_ref(),
    }
}
```

`score_candidate` (Part 1, step 4) calls this for every self-referencing input instead of
reading `cells[id].source.as_ref()` directly; `Sheet::execute_plan` does the same for the
`PlanStep::Method` case. No search is layered on top: this is a single lookup per
self-referencing input, evaluated once per candidate execution exactly as the old
unconditional rule was.

**4. `stay::has_own_stay` decides whether a self-referencing output still has an independent
stay of its own to violate:**

```rust
fn has_own_stay(
    output_id: CellId,
    rel_id: RelationshipId,
    prior_derived: &HashMap<CellId, (RelationshipId, Box<dyn Any>)>,
) -> bool {
    !matches!(prior_derived.get(&output_id), Some((derived_by, _)) if *derived_by != rel_id)
}
```

When `self_reference` resolves to a *different* relationship's prior-derived value, that value
is itself derived, not an independently asserted stay -- scoring the tentative output against
`source` in that case would wrongly penalize the cell for not equalling a value that was never
its own. `score_candidate`'s violation check is gated on `has_own_stay`, so only cells that
still have a real stay of their own (no entry, or an entry produced by this same relationship)
are scored; a downstream cell's own violation already carries the real cost of the choice (see
`stay.rs`'s doc comments for the verification that skipping this check reintroduces a tie
between the correct and incorrect outcomes).

**5. `resolve_component` and `plan()`/`release::resolve()` are structurally unchanged.**
`Assignment` and `Plan` gain no new field: `resolve_component` still just enumerates
structural candidates via `solve_acyclic_all` and scores each with `score_candidate` (now
threading `prior_derived` through), exactly as Part 1 already did. `prior_derived` itself is
the only new value passed down the call chain (`plan()` takes it as a new parameter, threaded
from `Sheet::propagate`'s Phase 0 through `release::resolve` and `stay::resolve_component`) --
there is no extra thing to select among, so there is nothing new to merge across components or
carry on `Plan`.

**6. `Sheet::execute_plan` writes `derived_by` only for genuinely self-referencing outputs:**

```rust
let self_ref_outputs: Vec<bool> = method.outputs.iter().map(|o| method.inputs.contains(o)).collect();
let shadow_outputs: Vec<bool> = self_ref_outputs.iter().map(|&self_ref| self_ref || is_conditional).collect();
```

and, when writing each output:

```rust
if shadow {
    cell.derived = Some(new_value);
    cell.derived_by = self_ref.then_some(rel_id);
} else {
    cell.source = new_value;
}
```

A cell shadowed only because it belongs to the active branch of a conditional relationship
(`is_conditional`, not self-referencing) gets `derived` set but `derived_by` left `None` --
this is what keeps `prior_derived` (Part 2's mechanism) and `previously_derived` (Phase 5's
pre-existing revert-tracking) independent, so a branch deactivating still reverts its cells to
`source` exactly as it always has, unaffected by this design.

### Interaction with Part 1

This does not change Part 1's steps or its overall selection criterion (lexicographically
fewest/weakest violated stays, tie-broken by source-set strength): it only changes what value
a self-referencing input actually reads during execution, and which outputs count toward the
violated-stay vector. A component with no self-referencing method is completely unaffected
(such a component has no self-referencing outputs, so `self_reference`/`has_own_stay` are
never consulted for it). A self-referencing component whose cells have no live
`prior_derived` value at all (the very first `propagate()` call, or a chain where nothing was
actually derived last round) also degenerates to exactly Part 1's original behavior, since
`self_reference` falls through to `source` whenever `prior_derived` has no entry.

### Updated Non-goals

Part 1's claim "No change to `Method`, `Filter`, `Plan`, or any public `Sheet` API: entirely
internal to `release::resolve`" still holds for `Plan`'s fields and every *public* API.
`planner::plan()`'s and `release::resolve()`'s `pub(crate)` signatures gain a new
`prior_derived` parameter, and `CellData` (already `pub(crate)`) gains the `derived_by` field;
neither is visible outside the crate.

### Updated Testing

New in `adam-rs/tests/integration.rs` (extends the existing three-write-round shape):

- `issue_182_inequality_chain_later_edit_below_earlier_one_repropagates`: continuing directly
  from `issue_182_inequality_chain_preserves_a_consistent_edit`'s first two steps
  (`write(a, 25)`, propagate), `write(c, 24)`, propagate, expecting `a=24, b=24, c=24` -- the
  reproduction above, confirming the minimal-necessary-violation outcome once `c` drops below
  `a`'s stay.
- Unit-level tests in `adam-rs/src/planner/stay.rs` (`self_reference_reads_prior_derived_when_produced_by_a_different_relationship`,
  `self_reference_reads_source_when_prior_derived_was_produced_by_the_same_relationship`,
  `self_reference_reads_source_when_no_prior_derived_value_exists`, and the equivalent trio
  for `has_own_stay`) exercising the same-vs-different-relationship rule directly against a
  hand-built `prior_derived`, independent of any full `propagate()` round.
- A regression confirming `self_ref_pressure_persists_without_rewriting_anchor` still passes
  unmodified: verified against the implementation (not just argued from the design), since `a`
  there is self-referenced by the same relationship every round, so `self_reference` always
  falls through to `source`, exactly as before this design.
- A regression confirming `cell_shadowed_as_self_ref_in_one_branch_and_forced_output_in_another`
  and `changed_reports_cell_reverted_by_conditional_deactivation` still pass unmodified,
  guarding the `derived_by`/conditional-branch independence from item 6 above.

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
- No two relationships may claim the same cell as an output in one round — self-referencing
  outputs are claimed exactly like any other (`planner::matching::Assignment`;
  `Error::Conflict` when infeasible).
- The selected methods' induced dependency digraph is acyclic before execution
  (`planner::digraph::is_acyclic`; `Error::Cycle`/`Error::FilterCycle` when not).
- A self-referencing input never reads a same-round `derived` value (`Sheet::execute_plan`).
  Across rounds, it reads the pre-round `source` value, unless a *different* relationship
  produced the cell's derived value last round, in which case it reads that prior derived
  value instead (Part 2, above).
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
