# Planner: Cyclic (Diamond) Constraint Resolution Design

**Date:** 2026-08-04
**Author:** Sean Parent (with Claude)
**Status:** Draft

## Problem

`adam_rs::planner::plan` (`adam-rs/src/planner.rs`) selects one method per active
relationship using a single greedy pass: cells are visited in descending strength order,
the first undetermined cell becomes an independent *source*, and a flood-fill propagates
through relationships whose methods become eligible as a result. A `forced_output_cells`
fixpoint pre-pass and a reactive pre-claim mechanism patch two known gaps in that greedy
walk, but neither addresses a third, structural failure mode: **when two relationships
share more than one cell, the greedy walk can commit two independent sources that are
jointly infeasible, even though a valid plan exists.**

### Worked example: `begin/examples/diamond.adm2`

```
relationship R1 { [a,b]->[c]  [a,c]->[b]  [b,c]->[a] }   // any 2 of {a,b,c} determine the 3rd
relationship R2 { [b,c]->[d]  [b,d]->[c]  [c,d]->[b] }   // any 2 of {b,c,d} determine the 3rd
```

`R1` and `R2` share two cells, `b` and `c`. If `a` and `d` are the two highest-strength
cells (both outrank `b` and `c`), the outer loop commits `a` and then `d` as independent
sources before `b` or `c` ever gets a turn. This is provably unrecoverable, not just
badly ordered: with `a` fixed as `R1`'s input, `R1` must choose one of `b`/`c` as its
output and the other as its second input — but whichever of `b`/`c` is `R1`'s *input*
must already be determined, and the only relationship that could determine it is `R2`,
which needs the *other* of `b`/`c` as input — which only `R1` can determine. Neither can
go first. `{a, d}` is not a valid pair of independent sources for this structure under
any execution order; at least one of `b`/`c` must be a source too. The current planner
has no way to discover this before committing both `a` and `d`, so it reports
`Error::Conflict` even though `propagate()` would succeed cleanly if `b` or `c` outranked
`a`/`d` (verified by hand-trace against the current implementation).

This generalizes beyond the pairwise case: any number of relationships whose shared-cell
structure forms overlapping cycles can strand a relationship the same way, and detecting
this requires reasoning about the *global* structure of the active relationship set, not
a per-cell or per-relationship local check.

### Relation to prior art

This is the same structural problem equation-oriented simulators (Modelica-family DAE
solvers) solve when assigning equations to unknowns: **Dulmage–Mendelsohn decomposition**
(Dulmage & Mendelsohn, 1958) of a bipartite variable/equation graph, combined with
**Tarjan's SCC algorithm** (1972) over the induced assignment digraph to find minimal
"algebraic loops" (in chemical-process-flowsheeting literature this is called *tearing*:
Sargent & Westerberg 1964, Steward 1965). Multi-way UI constraint solvers (Borning et
al.'s DeltaBlue/SkyBlue lineage — Sannella 1994) face a version of this too, but their
algorithms are documented as *incomplete*: SkyBlue can give up on a solvable cyclic
system depending on visitation order, which is exactly today's `adam-rs` failure mode.
`adam-rs` differs from both DAE solvers (which tolerate a cycle by iterating numerically
within it) and SkyBlue (which is deliberately heuristic): it demands a strict,
closed-form, single-pass execution order, so a cycle must be *torn* — one member turned
into a source — rather than iterated or given up on.

## Design

Replace the greedy strength-ordered flood-fill with a structural algorithm in five steps,
run once per `plan()` call over the `active` relationship set.

### Step 1 — Feasibility via maximum bipartite matching

Build a bipartite graph: one node per active relationship, one node per cell, edge
`(R, cell)` iff some method of `R` has `cell` as its sole output and `R`'s other member
cells as inputs. (Multi-output methods are handled by the flow generalization below;
this simple form covers every method in the codebase's tests and the `diamond.adm2`
example.) Run Hopcroft–Karp to find a maximum matching. If it doesn't saturate every
relationship node, no valid assignment exists at all and `plan()` returns
`Error::Conflict` immediately — a direct global feasibility check, replacing today's
approach of running the greedy walk and discovering infeasibility only after silently
over-committing sources.

- Complexity: O(E·√V) where V = cells + relationships, E = candidate method edges.

### Step 2 — Find cycles via Tarjan SCC

Direct each matched edge as `(R's other inputs) → (R's matched output cell)` and run
Tarjan's SCC over the induced digraph. A trivial SCC (a single node, no self-loop) is
already correctly ordered — this is the non-cyclic majority of any sheet and needs no
new machinery. A **non-trivial SCC (size > 1) is exactly a "diamond"**: a minimal set of
relationships/cells that this particular matching cannot execute acyclically. Dulmage–
Mendelsohn's classical uniqueness result means this decomposition is a structural
invariant of the bipartite graph — it does not depend on which maximum matching Step 1
happened to find, so the diamonds found here are the "real" ones, not an artifact of
matching order.

- Complexity: O(V+E).

### Step 3 — Tear each diamond by strength

Within a non-trivial SCC, repeatedly release the highest-strength member cell (remove it
as a matching candidate; find an augmenting path re-routing its relationship to a
different output cell) until the block's induced digraph is acyclic (checked by re-
running SCC on just the shrunk block). This greedy-by-strength release is provably
optimal: the sets of cells that can *simultaneously* be released while every relationship
still finds a distinct output form the independent sets of a **gammoid** (Perfect 1968,
built on Rado's and Edmonds's transversal-matroid theory) — a matroid — so processing
candidates in descending strength and keeping each one free whenever the rest remains
feasible is guaranteed to produce the strength-lexicographically-optimal source set. This
both formalizes and directly generalizes the descending-strength loop the current
planner already uses.

This step also subsumes the existing self-referencing-method tie-break
(`planner.rs:154-168`, "prefer the method whose self-ref output is the currently-processed
cell"): a relationship offering two self-referencing candidate outputs is just two
candidate edges in the same bipartite graph, and the matroid-greedy rule already prefers
leaving the higher-strength cell free — the current bespoke tie-break becomes a special
case rather than separate logic. Self-referencing inputs do not contribute a dependency
edge in Step 2's digraph (they read the pre-round value, mirroring how `pure_outputs()`
already excludes them today), so they never create a spurious self-loop.

- Complexity: O(D · (E + V)) where D = total cells released across all blocks, bounded by
  the total cell count.

### Step 4 — Cascade

Releasing a cell can shrink an *adjacent, overlapping* block (one sharing a cut vertex or
the released cell itself). Recompute only the affected neighborhood and repeat Steps 2–3
to a fixpoint. This is the direct generalization of the existing `forced_output_cells`
fixpoint loop, and is what realizes "solving one cycle may resolve another."

### Step 5 — Emit execution order

Once every SCC is trivial, a topological sort of the induced digraph gives
`execution_order` directly. Today's flood-fill traversal logic can be reused essentially
as-is for this step, since by this point conflict-freedom is already guaranteed —
`is_eligible` no longer needs to guard against the diamond case, only walk a known-DAG.

### Multi-output methods

A method claiming several output cells at once is an all-or-nothing bundle that plain
bipartite matching can't express. Model it as a flow arc-group with lower bound = upper
bound = 1 on every output edge (Ford–Fulkerson's classical feasible-flow-with-lower-
bounds reduction), turning Step 1 into a max-flow computation instead of Hopcroft–Karp
and Step 3's release step into flow re-augmentation. Steps 2, 4, and 5 are unchanged —
they only depend on the induced digraph, not on how it was produced.

### Forced cells and forced relationships become byproducts

`Sheet::is_forced` / `is_relationship_forced` currently rely on the bespoke
`forced_output_cells` fixpoint (see `2026-07-09-planner-forced-outputs-design.md`). Under
this design they become direct byproducts of the matching: a cell is forced iff it is
matched in *every* maximum matching of the bipartite graph, and a relationship is forced
iff its matched output cell is the same across every maximum matching — both computable
via standard alternating-path reachability from unmatched vertices (the same
Dulmage–Mendelsohn machinery as Step 1), replacing the separate fixpoint entirely rather
than running alongside it.

### Conditionals

No changes needed. `Sheet::build_active_set` / `match_cell_subgraph` already compute the
active relationship subset upstream of `plan()`; the new `plan()` still receives `active`
and builds its graph/flow network over exactly that subset.

### Module layout

Split `planner.rs` into a small module tree:

- `planner/mod.rs` — public `plan()` entry point and the `Plan` struct (unchanged public
  shape: `execution_order`, `forced_outputs`, `forced_relationships`).
- `planner/matching.rs` — bipartite matching / flow-with-lower-bounds and the
  forced-cell/forced-relationship alternating-reachability computation.
- `planner/scc.rs` — Tarjan's SCC over the induced digraph.
- `planner/tear.rs` — the greedy strength-ordered release/re-augmentation step per
  non-trivial SCC, plus the Step 4 cascade fixpoint.

## Error Handling

No changes to `Error`. Genuine conflicts — no maximum matching saturates every
relationship (Step 1), or a non-trivial SCC remains cyclic under every possible release
order (a true algebraic loop with no external input, e.g. `x=f(y); y=g(x)`) — still
surface as `Error::Conflict`, now detected structurally instead of via a doomed greedy
walk.

## Testing

All existing `planner.rs` unit tests and `adam-rs/tests/integration.rs` tests must keep
passing unchanged — they assert observable contracts (`Plan.execution_order`,
`forced_outputs`, `forced_relationships`, `Error::Conflict`, `is_source`, `is_forced`,
`is_relationship_forced`), not implementation details, so they double as regression tests
for the rewrite: `plan_with_active_subset_ignores_inactive_relationship`,
`relationship_selected_at_most_once`, `conflict_returns_error`,
`single_method_output_is_forced_and_not_selected_as_source`,
`forced_outputs_cascade_through_adjacent_relationship`,
`forced_relationships_true_for_single_method_relationship`,
`forced_relationships_excludes_multi_method_relationship`,
`forced_relationships_cascade_through_adjacent_relationship`,
`dead_method_not_selected_before_owning_relationship`, plus the self-reference and
conditional-branch integration tests.

New tests, built directly from `diamond.adm2`'s shape:

- Shared cell (`b` or `c`) highest-strength: already solves today; confirms no
  regression.
- Both outer cells (`a`, `d`) highest-strength: today returns `Error::Conflict`; must
  return `Ok` with a valid plan (`R1` selecting the `a,b→c` or `a,c→b` method paired
  consistently with `R2`'s selection, whichever the strength-greedy tear produces).
- A 3-relationship chain of overlapping diamonds, to exercise the Step 4 cascade
  explicitly (releasing one cell shrinks a second, adjacent block).
- A self-referencing method inside a cyclic block, and a multi-output method inside a
  cyclic block, confirming both fold into the general model.
- A genuinely unsolvable cycle (every matching is cyclic, no external input available)
  still correctly reports `Error::Conflict`.
- `is_forced` / `is_relationship_forced` re-verified against the new matching-based
  computation for the existing forced-cell test cases, plus a case scoped to a
  currently-inactive conditional branch.

## Future Work

- Incremental re-matching across successive `propagate()` calls (currently every call
  rebuilds the graph from scratch) if profiling on large sheets ever shows this matters —
  not pursued now, since `adam-rs` targets UI-scale property models (tens of cells).
- Surfacing which specific cells are torn in a diamond (beyond the existing
  `is_source`/`is_forced` accessors) to the `begin` Inspector UI, if there's a concrete
  need to visualize *why* a particular cell ended up derived rather than exogenous.
