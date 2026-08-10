# Output Cells and Conditions

**Date:** 2026-08-07
**Branch:** worktree-out-cells
**Status:** Approved (design), not yet implemented

## Summary

Add an **output** construct to `adam-rs`: a cell written by exactly one method
(no planner arbitration) that is also a terminal — it can never be used as an
input to another relationship, conditional, or output. An output carries zero
or more named **conditions**: boolean checks, evaluated after every
`propagate()`, that may reference any cell in the sheet (not only the
writer's own inputs).

This is `adam-rs`'s analogue of the Adobe Source Libraries "output cell with
invariant," but at finer granularity: instead of one pass/fail invariant per
output, each output carries a set of independently named conditions, so a
caller can distinguish *which* precondition failed (e.g. `max_area` vs.
`max_width`) rather than just "invalid."

Also adds a general-purpose `contributing_cells` query, usable on *any* cell
(not just outputs), that answers "which root source cells currently
determine this cell's value" — generalizing a BFS that already exists
inline inside `add_conditional`'s validation.

---

## 1. Motivation

A sheet can represent the arguments to a function or application command,
and a single sheet may dispatch to more than one command. A command has
preconditions that aren't always desirable or possible to uphold directly
with bidirectional relationships (e.g. `width * height <= MaxArea` isn't
something you'd want the solver to "solve" by silently adjusting `width` —
it's a precondition on the command, not a relationship to maintain). Outputs
give a place to compute such derived values and check their preconditions
without pulling them into the constraint-solving graph.

---

## 2. Terminal cells

An output's cell can never be referenced as an input anywhere else in the
sheet: not as a `Method` input/output in another relationship, not as a
`Conditional`'s match cell or branch relationship, not as a `Condition`
input on another output, and not as the writer output of a second
`add_output` call. It also can never be `write()`-able directly — its value
is always and only produced by its writer method.

This makes outputs strict sinks. Nothing downstream can observe an invalid
output value and propagate it further, so condition failures never need to
gate or block propagation — they're purely an observation the caller queries
after the fact.

`Sheet` tracks this with a new field:

```rust
terminal_cells: HashSet<CellId>,
```

`add_relationship`, `add_conditional`, and `write` each gain a check: if any
`CellId` they reference is in `terminal_cells`, return `Error::TerminalCell`.

---

## 3. Data model

New stable-handle types, following the existing `CellId` / `RelationshipId`
/ `ConditionalId` pattern:

```rust
new_key_type! {
    /// A stable handle to an output in a `Sheet`.
    pub struct OutputId;
}

new_key_type! {
    /// A stable handle to a condition in a `Sheet`.
    pub struct ConditionId;
}

pub(crate) struct OutputData {
    /// The terminal cell this output writes.
    cell: CellId,
    /// The single-method relationship backing the writer.
    relationship: RelationshipId,
    /// This output's conditions, in declaration order.
    conditions: Vec<ConditionId>,
}

pub(crate) struct ConditionData {
    name: String,
    output: OutputId,
    /// Arbitrary cells — not necessarily the writer's own inputs.
    inputs: Vec<CellId>,
    input_types: Vec<TypeId>,
    function: Box<dyn Fn(&[&dyn Any]) -> Result<bool, anyhow::Error>>,
}
```

`Sheet` gains:

```rust
outputs: SlotMap<OutputId, OutputData>,
conditions: SlotMap<ConditionId, ConditionData>,
terminal_cells: HashSet<CellId>,
/// Conditions that evaluated false as of the last `propagate()` call, grouped
/// by output. Not recomputed by `propagate_without_replan`, consistent with
/// `last_forced` / `last_forced_relationships`.
last_violated: HashMap<OutputId, Vec<ConditionId>>,
```

---

## 4. `Condition`

A condition has no output, so it can't literally be a `Method` (which
requires at least one output). It mirrors `Method`'s shape and arity
helpers instead, in a new `condition.rs`:

```rust
pub struct Condition {
    pub(crate) inputs: Vec<CellId>,
    pub(crate) input_types: Vec<TypeId>,
    pub(crate) function: Box<dyn Fn(&[&dyn Any]) -> Result<bool, anyhow::Error>>,
}

impl Condition {
    pub fn new<F>(inputs: Vec<CellId>, input_types: Vec<TypeId>, f: F) -> Self
    where
        F: Fn(&[&dyn Any]) -> Result<bool, anyhow::Error> + 'static;

    pub fn from_fn_1<A, F>(input: CellId, f: F) -> Self
    where
        A: Any + 'static,
        F: Fn(&A) -> Result<bool, anyhow::Error> + 'static;

    pub fn from_fn_2<A, B, F>(inputs: [CellId; 2], f: F) -> Self
    where
        A: Any + 'static,
        B: Any + 'static,
        F: Fn(&A, &B) -> Result<bool, anyhow::Error> + 'static;

    // from_fn_3, ... following the same pattern as Method::from_fn_2_1
}
```

Returning `Result<bool, _>` (not plain `bool`) matters: a check like
`width * height <= max_area` needs `checked_mul` per this crate's
fallible-ops convention, and a genuine arithmetic failure is a different
kind of problem than "the condition evaluated to false."

---

## 5. `Sheet::add_output`

```rust
pub fn add_output(
    &mut self,
    writer: Method,
    conditions: Vec<(impl Into<String>, Condition)>,
) -> Result<OutputId, Error>
```

The output's cell must already exist (created via ordinary `add_cell`,
exactly like any other derived cell today — it starts with a placeholder
value that's only ever observed before the first `propagate()`).

Steps:

1. Validate `writer.outputs.len() == 1` — an output has exactly one output
   cell. (The type system can't express this since `Method` is shared with
   `add_relationship`; checked at runtime → `Error::InvalidOutput`.)
2. Validate condition names are non-empty and unique within this call →
   `Error::InvalidOutput` on duplicates.
3. Call the existing `self.add_relationship(vec![writer])`. This reuses all
   current type-checking and adjacency-tracking for the writer, and — since
   it's constructed from exactly one method — the relationship is
   automatically deterministic under the existing planner (a single-method
   relationship is always fully "forced," per the existing
   `is_relationship_forced` semantics). Terminal-cell checks inside
   `add_relationship` (§2) also cover the writer's inputs for free: none of
   them may already be another output's terminal cell.
4. Type-check each condition's `inputs` against registered cell `TypeId`s,
   the same way `add_relationship` checks `Method` inputs. This also runs
   the terminal-cell check from §2 against each condition input.
5. Mark the writer's output cell as terminal (insert into `terminal_cells`).
6. Insert `ConditionData` entries and the `OutputData` entry; return the new
   `OutputId`.

### Errors

```rust
/// An `add_output` call is structurally invalid: the writer method does not
/// have exactly one output cell, a condition has an empty name, or two
/// conditions in the same call share a name.
InvalidOutput,

/// A cell that belongs to an existing output (see `Sheet::add_output`) was
/// referenced as an input to a relationship, conditional, condition, or a
/// second output, or was the target of `Sheet::write`.
TerminalCell,
```

---

## 6. Propagation integration

No planner changes are needed for the writer itself — a single-method
relationship is already deterministically selected today. A new phase is
added after the existing Phase 5 (reversion change-tracking) in
`Sheet::propagate`:

**Phase 6 — Condition evaluation.** For every registered condition, evaluate
its function against the current effective values of its `inputs`. Rebuild
`last_violated` from scratch (sparse: an output with all conditions holding
has no entry; an output with one or more `false` conditions gets a
`Vec<ConditionId>` entry). If a condition's function returns `Err`,
`propagate()` aborts with `Error::MethodFailed` — the same variant already
used for a method's computation failure, since a condition evaluation
failure is the same kind of event (the check itself couldn't run), distinct
from a normal `false` result.

Because `last_violated` is sparse, it alone can't distinguish "never
propagated" from "propagated and fully valid" (both look like "no entry").
`output_valid` and `violated_conditions` (§7) resolve this the same way
`is_forced` already does: check `last_plan.is_none()` first and treat that
as "not yet propagated" (→ `output_valid` returns `false`, matching the
stated contract).

`propagate_without_replan()` does **not** re-run Phase 6 — same convention
as `last_forced` / `last_forced_relationships` not being recomputed there.
(Open question, deferred: see §9.)

---

## 7. Query API

```rust
impl Sheet {
    /// Returns the terminal cell backing output `id`. Read its value with
    /// the existing `Sheet::read`.
    pub fn output_cell(&self, id: OutputId) -> Option<CellId>;

    /// Returns `true` if every condition on `id` held as of the last
    /// `propagate()` call. Returns `false` if no propagation has run yet.
    pub fn output_valid(&self, id: OutputId) -> bool;

    /// Iterates the conditions on `id` that evaluated to `false` as of the
    /// last `propagate()` call. Empty if `id` is valid or no propagation has
    /// run yet.
    pub fn violated_conditions(&self, id: OutputId) -> impl Iterator<Item = ConditionId> + '_;

    /// Iterates every condition registered on output `id`, in declaration order.
    pub fn output_conditions(&self, id: OutputId) -> Option<&[ConditionId]>;

    pub fn condition_name(&self, id: ConditionId) -> Option<&str>;
    pub fn condition_output(&self, id: ConditionId) -> Option<OutputId>;
    pub fn condition_inputs(&self, id: ConditionId) -> Option<&[CellId]>;

    /// Returns the set of root source cells currently determining `id`'s
    /// value, as of the last `propagate()` call. See §8.
    pub fn contributing_cells(&self, id: CellId) -> HashSet<CellId>;

    /// Union of `contributing_cells` over condition `id`'s own declared
    /// inputs.
    pub fn condition_contributing_cells(&self, id: ConditionId) -> HashSet<CellId>;
}
```

Getting an output's *value* reuses `Sheet::read()` directly (via
`output_cell`) — no new value accessor is needed, since an output's cell is
an ordinary `CellId`.

---

## 8. `contributing_cells`: general, dynamic, not output-specific

This satisfies "for any cell, including out cells, query the set of source
cells currently contributing to that cell" — it is not restricted to
outputs.

It is **dynamic**: based on the last successful `propagate()`'s actual
selected methods (`last_plan`), not static structural adjacency. This
matters because a cell can be structurally adjacent to a relationship
without that relationship's method currently being the one that determines
its value (e.g. an unselected method in a multi-method relationship, or a
conditional branch that isn't currently active).

Algorithm, given a cell `id`:

1. If `is_source(id)` is `true` (no selected method in `last_plan` outputs
   `id`), the result is `{id}` — it's a root.
2. Otherwise, find the `last_plan` entry `(rel, method_idx)` whose method
   outputs `id`. Recurse into that method's inputs, **except** any input
   that is also one of that same method's outputs (a self-referencing
   input) — that input's contribution is its own pre-execution `source`,
   which for this purpose is treated as a root of itself, consistent with
   how `execute_plan` already reads self-referencing inputs from `source`
   rather than any derived value.
3. Union the recursive results. Since a plan is acyclic (the planner already
   rejects cycles — `Error::Cycle`), this recursion terminates; a
   `HashSet` accumulator with a visited guard avoids revisiting shared
   subexpressions redundantly.

This generalizes the BFS that already exists inline inside
`add_conditional`'s validation (`sheet.rs`, `contributing_cells` local —
computed there structurally/conservatively for a different purpose: deciding
whether a branch relationship may safely have more than one method). That
inline computation is unaffected by this change; the two serve different
needs (structural safety check at registration time vs. dynamic "what
currently determines this value" at query time) and are not unified here.

`condition_contributing_cells(id)` is simply the union of
`contributing_cells(input)` over `ConditionData::inputs` — a condition
itself has no cell of its own to walk from, only its declared inputs.

---

## 9. Illustrative `adam-lang` syntax (non-binding)

Not a proposed syntax — only shown to sanity-check that the Rust API shape
maps cleanly onto something DSL-shaped, for discussion in a future
`adam-lang` design pass:

```text
out image_size(width, height) -> ImageSize {
    condition max_area   { width * height <= max_area }
    condition max_width  { width <= max_width }
    condition max_height { height <= max_height }
}
```

---

## 10. Files changed / added

| File | Change |
| --- | --- |
| `src/output.rs` | New: `OutputId`, `OutputData` |
| `src/condition.rs` | New: `ConditionId`, `ConditionData`, `Condition` and its arity constructors |
| `src/sheet.rs` | New `outputs`, `conditions`, `terminal_cells`, `last_violated` fields; `add_output`; terminal-cell checks in `add_relationship`, `add_conditional`, `write`; Phase 6 in `propagate`; query methods (§7); `contributing_cells` (§8) |
| `src/error.rs` | Add `InvalidOutput`, `TerminalCell` variants |
| `src/lib.rs` | Re-export `OutputId`, `ConditionId`, `Condition`; add `pub mod output`, `pub mod condition` |
| `tests/integration.rs` | New tests (§11) |

---

## 11. Testing notes

Derived from the contract in §5–§8 only:

- `add_output` succeeds for a valid single-method writer with zero, one, and
  multiple conditions.
- `add_output` returns `Error::InvalidOutput` when the writer has zero or
  more than one output cell, and when two conditions share a name.
- `add_output`, `add_relationship`, `add_conditional`, and `write` each
  return `Error::TerminalCell` when referencing an already-terminal cell.
- After `propagate()`, `output_valid` reflects whether all conditions held;
  `violated_conditions` lists exactly the ones that evaluated `false`.
- `output_valid` and `violated_conditions` reflect an updated result after a
  second `propagate()` following a write that changes which conditions hold.
- A condition whose function returns `Err` aborts `propagate()` with
  `Error::MethodFailed`.
- `contributing_cells` on a plain written (never-derived) cell returns just
  that cell.
- `contributing_cells` on a cell derived through a chain of relationships
  returns the transitive set of root source cells, not any intermediate
  derived cell.
- `contributing_cells` on a self-referencing cell includes the
  self-referencing cell itself as one of its own roots.
- `contributing_cells` on a cell derived via a conditional's currently
  active branch reflects only that branch's inputs, not the inactive
  branches'.
- `condition_contributing_cells` returns the union of `contributing_cells`
  across a condition's own declared inputs, including inputs that are not
  among the writer method's own inputs.
- `output_valid` and `is_forced`/`condition_contributing_cells` return
  `false`/empty before any `propagate()` call has run, consistent with the
  existing "as of last propagate" convention elsewhere in `Sheet`.

---

## 12. Deferred / out of scope

- Conditionally-active outputs (gating an entire output's writer behind a
  `Conditional`, the way ordinary relationships can be). Not requested and
  not needed for the motivating use case (command preconditions); the
  writer relationship is always unconditionally active once registered, and
  its `RelationshipId` isn't exposed to callers, so this isn't possible to
  express even accidentally with this design.
- Final `adam-lang` DSL syntax for outputs/conditions — §9 is illustrative
  only.
- Whether `propagate_without_replan()` should re-run Phase 6. Left matching
  the existing convention (no) for this design; revisit if a caller needs
  condition results to stay current across `propagate_without_replan()`
  calls that change condition-relevant inputs (which would itself violate
  that method's existing "no source cell written since last propagate"
  precondition in the ordinary case, but conditions can reference cells
  outside the writer's own input set, so this isn't automatically covered
  by that existing precondition).
