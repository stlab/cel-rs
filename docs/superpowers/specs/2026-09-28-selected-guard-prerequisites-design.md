# Selected guard prerequisites and staged propagation

Date: 2026-09-28
Status: Proposed for written-spec review
Scope: Resolve the phase-B conditional pre-plan filter-cache regression without
reintroducing repeated user callback evaluation.

## Goal and boundary

An unconditional relationship may compute a conditional's match value. Only the
selected prerequisites of that value may execute before the conditional is resolved.
In particular, a filter unrelated to a match value cannot clamp using an argument
that a subsequent conditional relationship may change. The selected active plan must
still order filters after their actual argument producers.

Retain the existing all-method static `Sheet::validate` guard-independence check.
This change does not broaden the set of statically valid sheets or implement the
future analyzer. Detect selected-prerequisite and reuse conflicts at propagation
time; a conservative conflict can reject a sheet for which a different assignment
would work. Report the conflict rather than guessing a value or rerunning an
arbitrary user callback.

## Candidate approaches

1. **Selected prerequisite cone (chosen).** Plan unconditional relationships,
   traverse the chosen method/filter dependencies backward from match values, and
   stage only steps in this cone. Evaluate guards, then execute the remaining active
   plan. This supports unconditional guard derivations while excluding unrelated
   filters. Its bookkeeping is linear in the selected plan graph.
2. **Source-only guards.** Resolve all conditionals before running any relationships
   or filters. This is simple but rejects a valid sheet that computes a match value
   through an unconditional relationship.
3. **Delay all non-guard filters.** Avoid the immediate stale-filter failure, but
   leave the boundary of pre-executed methods and seed evaluations implicit. This
   makes callback reuse and selected-plan consistency harder to establish.

## Planning and execution

Use the existing planner to select the unconditional assignment once. Build reverse
producer edges from its selected methods and filter reclamps: a method output depends
on its non-self inputs, and a filtered source depends on its filter arguments. Starting
from every conditional match cell, traverse the selected edges and identify the
minimal prerequisite steps in the plan's topological order. For expression-sourced
conditions, start from every expression input. Include a source-cell filter when its
clamped value is read by a guard, including the match cell itself. Do not execute a
filter merely because the unconditional planner emitted it.

Stage only the identified method and filter steps, including values and diagnostics;
do not change live values or publish `changed()`. The existing static guard check
already rejects a conditional branch feeding its own match prerequisites. If the
selected prerequisite traversal nevertheless discovers a dependency cycle, report an
explicit cycle error. A prerequisite seed must use the planner's chosen assignment
and its normal seed dependencies; seed callbacks run only for seeds needed by staged
prerequisite steps.

Evaluate each conditional once against the staged match values. Plan the resulting
active relationship set with existing assignment semantics. Check that any
pre-executed method or filter needed by the final plan remains compatible with its
selected method, producer inputs, source/derived classification, and dependency
ordering. Reuse its staged result only when compatible; if the final assignment or
its producers require another value, return an explicit conflict with implicated
relationship/cell sites, without rerunning the callback. The same rule applies to
seed-method callbacks used to compute prerequisite seeds. A step executed to select
a guard must not be dropped or silently overwritten by the final assignment.

Execute the remaining final-plan steps in order into the same private stage. Filters
outside the guard-prerequisite cone therefore run only after the selected conditional
relationships producing their arguments. A logical callback evaluation already used
to establish a guard is reused, not replayed solely because the final plan also
contains that step. Distinct seed and selected-method evaluations can legitimately
invoke the same function with different inputs; record their inputs and identities
separately. Failed seed construction or compatibility checking publishes no staged
values. Clear prior `changed()` results at the start of propagation as in the
current implementation. Commit staged values only after planning, seed validation,
and staged execution succeed.

## Errors and complexity

Use `Error::Conflict` with concrete implicated sites for a selected-prerequisite
assignment or callback-reuse incompatibility; use the existing cycle errors when
their respective selected graphs are cyclic. No mismatch returns success, falls back
to declaration order, or re-evaluates a callback. Document that such conflicts can
be conservative: another assignment might have been feasible without prior guard
evaluation.

Constructing the reverse selected dependency graph, walking the prerequisite cone,
and checking recorded producers/reuse are O(V + E) time and space per plan (V is
selected cells/steps; E is their selected dependencies), not an all-method
prerequisite analysis. Existing planner assignment-search complexity and its two
planning phases are unchanged. Avoid rescanning every relationship for each match
cell; use indexed producers and one visited set for all guards.

## Contract tests

- A match cell computed by an unconditional relationship selects the correct branch
  and executes the prerequisite callback once.
- A match cell dependent on a filtered source observes its reclamped value before
  selecting a branch; the filter callback runs once.
- A filtered cell outside the guard cone reads a bound written by its selected
  conditional relationship, irrespective of relationship insertion order, and
  is not evaluated in the pre-plan.
- A stateful prerequisite callback is not repeated when its step also occurs in
  the final plan; an incompatible final assignment reports a conflict rather than
  executing it again.
- A needed seed callback is not repeated or silently reused with changed inputs.
- A selected prerequisite cycle or seed cycle produces an error without publishing
  staged writes; `changed()` contains no stale results from an earlier call.
- Existing self-referencing chains, guard-independent conditionals, filtered source
  cells with no relationship membership, and conditional branch reversals still
  propagate correctly.

## Out of scope

- Replacing the static all-method guard-independence check with a selected-plan check.
- An analyzer that proves a different assignment can satisfy a conservative
  prerequisite conflict.
- Automatic plan reuse from planner generalization phase C.
