# Relationship method constraints and elimination-based plan algorithm

**Status:** Approved for implementation planning
**Crate:** `adam-rs`

## Motivation

`adam-rs`'s planner (`src/planner.rs`) currently selects one method per relationship using
three separate mechanisms layered together: a forward flood-fill eligibility check
(`is_eligible`, gated on all pure inputs being determined), a "pre-claiming" step for
sole-feasible methods, and a standalone fixpoint pass (`forced_output_cells`) that
precomputes cells/methods that can never be sources. This works but is hard to reason
about, and nothing currently prevents a relationship's methods from referencing
unrelated cells or producing colliding output sets.

Two new structural constraints on `Method`/`RelationshipData`, plus a single unified
selection algorithm driven by output-set elimination, let the planner drop the
input-eligibility/pre-claiming machinery in favor of one mechanism.

## 1. Structural validation (`Sheet::add_relationship`)

Three new checks are added alongside the existing ones (empty methods, empty
inputs/outputs, type mismatches):

- **Matching cell sets.** For every method `M` in a relationship,
  `inputs(M) ∪ outputs(M)` must equal the same set across all methods of that
  relationship. `RelationshipData::adj` — currently documented as "union of all cell
  IDs referenced by any method (union across all methods)" — becomes simply *the*
  shared cell set, since every method now spans it.
- **Unique output sets.** No two methods in the same relationship may have the same
  `outputs` set (as a set, ignoring order).
- **Duplicate-free outputs within a method.** A single method's own `outputs` list may
  not name the same cell twice. A duplicated output cell is either redundant (both
  occurrences would always agree, adding nothing) or contradictory (the method's
  function could return two different values for the same cell) — neither is useful, so
  it's rejected outright rather than left as unspecified behavior.

Violations return new `Error` variants (matching the existing granular style —
`TypeMismatch`, `InvalidId`, `InvalidConditional`, etc.):

- `Error::MismatchedMethodCells` — some method's `inputs ∪ outputs` differs from
  another method's in the same relationship.
- `Error::DuplicateMethodOutputs` — either a method's own `outputs` list names the same
  cell more than once, or two methods in the same relationship have identical `outputs`
  sets.

All three checks run in `add_relationship` alongside the existing per-method validation,
before the relationship is inserted into `self.relationships`.

## 2. Dynamic method selection: elimination by output set

Replaces `is_eligible`, `is_feasible`, and pre-claiming with one mechanism, using each
method's **full** `outputs` set (no self-referencing special case).

- Each relationship starts with all of its methods as candidates.
- Whenever a cell becomes **determined** — by any means: chosen as a source in the
  strength-ordered outer loop, or produced as the output of some relationship's already-
  selected method — every relationship adjacent to that cell eliminates any remaining
  candidate whose `outputs` set contains that cell.
- The instant a relationship's candidate set narrows to exactly one method, that method
  is *selected*. Each of its output cells not yet determined becomes determined too,
  cascading the same elimination to their adjacent relationships (queue-based, same
  shape as today's flood-fill BFS).
- If a relationship's candidate set narrows to **zero**, that relationship cannot be
  assigned — `Error::Conflict` (e.g. two relationships racing to produce the same cell;
  see the worked example below).

### Why no self-reference special case is needed

For a self-referencing pair like:

```
relationship {
    method [a, b] -> [a]   // M0, outputs = {a}
    method [a, b] -> [b]   // M1, outputs = {b}
}
```

If `b` is determined first (e.g. chosen as a source because it has higher strength),
`M1` is eliminated because `b ∈ outputs(M1)`, leaving `M0` as the sole candidate — which
reads the pre-execution value of `a` and `b`'s new value, and overwrites `a`. If `a` is
determined first instead, `M0` is eliminated and `M1` is selected symmetrically. Because
output sets are unique per relationship (constraint 2), whichever cell resolves first
eliminates exactly the method that would have produced it, and the other survives. No
input-readiness check or strength-based tie-break is needed to decide *which* method
runs — only to decide *which cell resolves first*, which is already handled by the
existing strength-ordered outer loop.

### Worked conflict example

Two relationships, each with a single method, both outputting the same cell `out`:
`R1: a -> out`, `R2: b -> out`. Whichever of `R1`/`R2` gets processed first in the
selection bootstrap (§3) determines `out`, which then eliminates the other
relationship's only candidate (its `outputs` set is `{out}`), leaving it with zero
candidates — `Error::Conflict`, matching today's behavior for this case
(`conflict_returns_error`).

## 3. Structural "forced" computation is retained

`Sheet::is_forced`, `forced_cells`, `is_relationship_forced`, and `forced_relationships`
must stay **strength-independent** — e.g. a UI disables an input field for a cell that
can *never* be a source, regardless of what the user might write. That is a different
question from the per-run dynamic elimination in §2 ("is this cell always someone's
output, no matter which cells end up as sources?"), so `forced_output_cells`'s fixpoint
(using `pure_outputs`, which excludes self-referencing cells) is kept conceptually as it
is today.

This structural pass seeds the dynamic pass: cells and relationships it marks forced are
selected/determined before the strength-ordered outer loop runs, and the outer loop
skips forced cells rather than adopting them as fresh sources — this part of the current
implementation's structure is unchanged.

## 4. Execution order requires an explicit topological sort

The current doc comment in `planner.rs` states "the selection order is already a valid
topological execution order." That invariant no longer holds: a relationship can now be
*selected* before its inputs are actually resolved (e.g. a structurally-forced
single-method relationship is selected immediately, regardless of when its input
arrives).

After all relationships are selected, a separate topological sort over the final
`(RelationshipId, method_idx)` assignments produces `execution_order`. This also gives
the currently-unused `Error::Cycle` variant a real purpose: two single-method
relationships `a -> b` and `b -> a` are each trivially selected (one candidate each, no
elimination needed) under the new algorithm — the selection-count check no longer
catches this case, since both *do* get selected. The topological sort detects the cycle
between them and returns `Error::Cycle` instead.

**Behavior change:** `mutually_dependent_relationships_return_conflict` (currently
expecting `Error::Conflict`) must be updated to expect `Error::Cycle`.

## 5. Test fallout

- **Remove** `dead_method_not_selected_before_owning_relationship`
  (`adam-rs/src/planner.rs`): its relationship has three methods spanning three disjoint
  cell sets (`{x,b}`, `{y,c}`, `{b,c,d}`), which constraint 1 makes inexpressible. The
  scenario it exercised (excluding a dead method from selection) is now handled
  structurally by §2/§3 together; confirm during implementation that
  `forced_outputs_cascade_through_adjacent_relationship` and
  `forced_relationships_cascade_through_adjacent_relationship` still exercise the
  cascading-elimination path adequately without it.
- **Update** `mutually_dependent_relationships_return_conflict`
  (`adam-rs/tests/integration.rs`): expect `Error::Cycle` instead of `Error::Conflict`.
- All other existing tests already satisfy both new structural constraints (verified by
  inspection of every `add_relationship` call site in `adam-rs/src`, `adam-rs/tests`,
  `adam-lang/src/parser.rs`, and `begin/src/bridge.rs`) and are expected to keep passing
  unchanged.

## Out of scope

- Reshaping `Plan`'s or `Sheet`'s public field/method shape beyond what's needed for the
  above (e.g. no new introspection API for per-relationship candidate state).
- Any change to conditional (`add_conditional`) validation logic — it already inspects
  `method.outputs.contains(&c)` in a way that is unaffected by these constraints.
- Cycle detection or reporting beyond what naturally falls out of the topological sort
  in §4 (no attempt to report *which* cells/relationships form the cycle beyond the
  `Error::Cycle` variant itself).
