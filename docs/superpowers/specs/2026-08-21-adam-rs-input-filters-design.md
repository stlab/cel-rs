# Input Filters (adam-rs)

**Date:** 2026-08-21
**Branch:** worktree-adam-rs-input-filters
**Status:** Approved (design), not yet implemented

## Summary

Add an **input filter** construct to `adam-rs`: an idempotent, type-erased function
attached to at most one non-terminal cell that runs in two different modes depending on
how the cell's current value came to be:

- **Write-time transform.** Every `Sheet::write()` call (and the cell's initial
  `add_cell` value, applied retroactively by `add_filter`) passes through the filter
  first. The filter may silently conform the value (e.g. clamp it into range) or reject
  it outright with an error, in which case the write has no effect.
- **Derived-value diagnostic.** When a filtered cell's value comes from a method this
  round instead of an external write, the filter runs again after `propagate()`, purely
  as an observation: if its output differs from the cell's current value (or the filter
  itself errors), a violation is recorded against that cell. Nothing is mutated and
  `propagate()` does not fail — this mirrors `Condition`'s "diagnostics never gate
  propagation" philosophy from the [output cells design](2026-08-07-output-cells-design.md).

A filter's dynamic arguments (e.g. the `1..100` in `clamp(_, 1..100)`) are ordinary
`CellId`s, resolved via each cell's current `effective()` value — the same "cell
reference, not a hardcoded constant" pattern `Conditional`'s `MatchExpr` already
established. This makes filter arguments externally inspectable for free (`read()`
already works on them) and gives literal-vs-named arguments a single uniform
representation: a literal like `1..100` is just an anonymous cell created by whichever
layer desugars the syntax (a later, non-`adam-rs` phase).

This is `adam-rs`'s analogue of a runtime-checked type constraint that can also *repair*
out-of-range input rather than merely reject it — distinct from `Condition`, which only
observes, never transforms.

---

## 1. Motivation

A cell's declared Rust type (`u32`, etc.) only constrains its *representation*, not its
*domain*. Many real inputs need a narrower, and sometimes dynamic, domain: a percentage
must sit in `0..=100`; a phone number must match a shape a `TypeId` can't express; a
slider's range should itself be adjustable at runtime by another cell. Filters give
`adam-rs` a single mechanism for both:

1. **Conforming external input** at the moment it enters the sheet, so a value written
   by a human, a UI slider, or an external system is guaranteed to satisfy its declared
   domain from that point on — or is rejected outright, if conforming isn't possible.
2. **Diagnosing internally-derived values** that a relationship's method computed and
   that don't happen to satisfy the same domain — without forcing the solver to correct
   it (a filter is a domain check, not a relationship the planner can pick a strength
   ordering to satisfy).

---

## 2. Data model

`CellData` (`adam-rs/src/cell.rs`) gains one field:

```rust
pub(crate) struct CellData {
    // ...existing fields...
    /// This cell's filter, if one is attached. At most one per cell.
    pub(crate) filter: Option<FilterData>,
}
```

A filter is inline on the cell, not a separate `SlotMap<FilterId, _>` the way
`Output`/`Condition` are — there's no multiplicity to manage, so no need for a stable
handle type at all.

New module `adam-rs/src/filter.rs`:

```rust
/// Type-erased function stored inside a `FilterData`.
///
/// Takes the candidate value and a slice of the filter's argument cells' current
/// effective values, and returns the conformed value or an error.
type FilterFn = Box<dyn Fn(&dyn Any, &[&dyn Any]) -> Result<Box<dyn Any>, anyhow::Error>>;

pub(crate) struct FilterData {
    /// Dynamic argument cells, resolved via `effective()` wherever the filter runs.
    pub(crate) args: Vec<CellId>,
    pub(crate) arg_types: Vec<TypeId>,
    pub(crate) function: FilterFn,
}
```

Note there is **no `eq_fn` field on `FilterData`.** `CellData` already stores an
`eq_fn` for the filtered cell's own type, captured from `T: PartialEq` at `add_cell`
time (`cell.rs`). The diagnostic phase (§4) reuses `self.cells[id].eq_fn` directly rather
than storing a second, redundant copy — which also means `Filter`'s typed constructors
below need no `PartialEq` bound, matching `Method::from_fn_1_1`'s existing bounds
(`Any + 'static` only).

Public builder, mirroring `MatchExpr`'s and `Method`'s existing arity-helper convention:

```rust
pub struct Filter(pub(crate) FilterData);

impl Filter {
    /// Type-erased constructor.
    pub fn new<F>(args: Vec<CellId>, arg_types: Vec<TypeId>, f: F) -> Self
    where
        F: Fn(&dyn Any, &[&dyn Any]) -> Result<Box<dyn Any>, anyhow::Error> + 'static;

    /// No dynamic arguments — a fixed transform (e.g. a hardcoded clamp range).
    pub fn from_fn_0<T, F>(f: F) -> Self
    where
        T: Any + 'static,
        F: Fn(&T) -> Result<T, anyhow::Error> + 'static;

    /// One dynamic argument cell.
    pub fn from_fn_1<A, T, F>(arg: CellId, f: F) -> Self
    where
        A: Any + 'static,
        T: Any + 'static,
        F: Fn(&T, &A) -> Result<T, anyhow::Error> + 'static;

    /// Two dynamic argument cells.
    pub fn from_fn_2<A, B, T, F>(args: [CellId; 2], f: F) -> Self
    where
        A: Any + 'static,
        B: Any + 'static,
        T: Any + 'static,
        F: Fn(&T, &A, &B) -> Result<T, anyhow::Error> + 'static;
}
```

`Sheet::add_filter`:

```rust
/// Attaches `filter` to `cell`, applying it immediately to the cell's current value
/// (see §3.1) so a filtered cell's value is guaranteed-conforming from this call
/// onward.
///
/// # Errors
///
/// - `Error::InvalidFilter` — `cell` is not a live cell in this sheet, `cell` is
///   terminal (belongs to an output), `cell` already has a filter, an argument cell is
///   not a live cell in this sheet, or a type mismatch exists between the filter's
///   value type / an argument's type and its cell's registered type.
/// - `Error::MethodFailed` — the filter rejected the cell's current value.
pub fn add_filter(&mut self, cell: CellId, filter: Filter) -> Result<(), Error>
```

### 2.1 `Error::InvalidFilter`

Following `InvalidConditional`/`InvalidOutput`'s existing convention — one catch-all
variant per construct, with the doc comment enumerating every structural failure mode —
`adam-rs/src/error.rs` gains:

```rust
/// An `add_filter` call is structurally invalid: the cell was not found, the cell is
/// terminal, the cell already has a filter, an argument cell was not found, or a type
/// mismatch exists between the filter's value type or an argument's type and its
/// cell's registered type.
InvalidFilter,
```

No new variant is needed for a filter rejecting a value — that already fits
`Error::MethodFailed(anyhow::Error)`, used identically for a `Method`'s function
failing.

---

## 3. Write-time transform

### 3.1 `Sheet::write()`

Before storing the value, if the target cell has a filter: resolve the filter's argument
cells' current `effective()` values, call the filter with the incoming value, and:

- `Ok(v)` — `v` (not necessarily the value passed in — silent conforming, e.g.
  clamping) becomes the new `source`; strength bumps exactly as it does today,
  unconditionally.
- `Err(e)` — `write()` returns `Err(Error::MethodFailed(e))` immediately. The cell is
  left completely untouched: no strength bump, no `source` change. This is a normal
  `Result` the caller already handles, not a soft diagnostic — rejection at write time
  means "this specific write did not happen."

### 3.2 `add_filter` and the initial value

`add_cell`'s initial value can never have passed through a filter, because no filter can
exist yet at that point. Left alone, this creates a gap: if that initial value doesn't
conform and the cell is never a method's output (an always-a-source cell, e.g. a filter
argument cell itself, or a plain unwired input), nothing ever catches it — the
write-time transform doesn't apply retroactively, and the diagnostic phase (§4) only
runs for cells a method actually produced this round.

`add_filter` closes this by running the filter against the cell's current `source`
value at attach time, updating `source` on success exactly as §3.1's conforming path
does — but, unlike an actual `write()`, it does **not** bump strength: attaching a
filter is not a fresh external input and shouldn't reprioritize the cell. This gives a
clean, checkable invariant:

> **A filtered cell's value is always conforming, except when a method produced it this
> round.**

which is exactly the case §4 exists to catch. This invariant has one known boundary:
a filter with a *dynamic* argument cell can go stale if that argument's value changes
after the filtered cell was last conformed, and the filtered cell is itself a plain,
always-source cell no method ever produces — tracked as
[#132](https://github.com/stlab/cel-rs/issues/132), out of scope for this phase.

---

## 4. Derived-value diagnostic

Added as a new phase in `Sheet::propagate()`, immediately alongside the existing Phase 6
`Condition` evaluation (which already establishes the precedent of reading
post-round `effective()` values, not a pre-plan snapshot — so no new snapshot phase is
needed for filters either):

For every cell with a filter that is **not** a source under this round's plan — the same
underlying check `Sheet::is_source` encapsulates (a cell is not a source if some selected
method's outputs include it), evaluated against the `plan` just computed in this round's
Phase 3, since `self.last_plan` itself isn't updated until the very end of
`propagate()` — resolve the filter's argument cells' post-round `effective()` values and
evaluate the filter against the cell's own post-round `effective()` value:

- `Ok(v)` where `(cell.eq_fn)(&v, effective)` holds — no violation.
- `Ok(v)` where it doesn't hold — violation: `FilterViolation::NotConformed`.
- `Err(e)` — violation: `FilterViolation::Failed(e)`.

Unlike `Condition`'s Phase 6, where a predicate returning `Err` aborts `propagate()`
via `Error::MethodFailed`, a filter returning `Err` during this diagnostic pass does
**not** abort propagation. A filter's `Err` is an expected, first-class outcome (it's
the exact mechanism that makes phone-number-style validation possible) — not a bug in
the check itself, unlike a `Condition` predicate blowing up. So both `NotConformed` and
`Failed` are equally soft, non-fatal diagnostics.

```rust
pub(crate) enum FilterViolation {
    /// The filter succeeded but its output differs from the cell's current value.
    NotConformed,
    /// The filter's function itself returned an error.
    Failed(anyhow::Error),
}
```

`Sheet` gains:

```rust
/// Cells whose filter did not hold as of the last full `propagate()` call.
last_filter_violations: HashMap<CellId, FilterViolation>,
```

Not recomputed by `propagate_without_replan()`, consistent with `last_violated`,
`last_forced`, and `last_forced_relationships` — confirmed by reading
`propagate_without_replan`, which only calls `execute_plan` and
`post_process_strengths`, never touching `last_violated`. Filter violations stay pinned
to whatever was true as of the last full `propagate()`.

### 4.1 Query API

```rust
/// Returns the filter violation recorded for `id` as of the last full `propagate()`
/// call, if any.
pub fn filter_violation(&self, id: CellId) -> Option<&FilterViolation>;

/// Iterates cells whose filter is currently violated, as of the last full
/// `propagate()` call.
pub fn filter_violated_cells(&self) -> impl Iterator<Item = CellId> + '_;

/// Returns the set of root cells currently determining a filter-violated cell's value
/// or its filter's argument values — reusing the existing `contributing_cells` BFS
/// (added for `condition_contributing_cells`/`output_violation_cells`), the same "which
/// upstream cells caused this" query `begin` already uses for condition violations.
pub fn filter_violation_cells(&self) -> HashSet<CellId>;

/// Returns the argument cells of `id`'s filter, if it has one — the direct answer to
/// "what determines this filter's bounds," for a UI (e.g. a slider) to inspect.
pub fn filter_args(&self, id: CellId) -> Option<&[CellId]>;
```

---

## 5. Non-goals for this phase

- **No builtin filter combinators.** `clamp`, or any other named transform, does not
  ship in `adam-rs` — a filter is just a closure. `clamp(_, 1..100)` becoming a filter
  is adam-lang syntax sugar for a later phase.
- **No adam-lang syntax.** Parsing `filter { ... }` blocks, resolving `_`, and
  desugaring literal arguments into anonymous cells are all later-phase concerns.
- **No `begin` UI.** Visualizing filter args (e.g. as a slider) or violations is a later
  phase, once the query API in §4.1 exists to build on.
- **No "synthetic cell" concept.** A literal filter argument's anonymous backing cell is
  indistinguishable from any other cell at this layer — hiding it from a UI's cell
  listing, if ever wanted, is entirely a later-phase display concern.
- **Filters do not attach to output (terminal) cells.** Outputs already have
  `Condition` for diagnostics and cannot be `write()`-ed, so a filter's write-time half
  would be moot there; revisit unifying the two only if a real use case shows up.
- **At most one filter per cell.** No composition/ordering semantics across multiple
  filters on the same cell — a single filter closure can call out to as many checks or
  transforms as it needs internally.

---

## 6. Testing

Contract-derived unit tests, following this repo's convention of testing observable
behavior only:

- `add_filter`: each `InvalidFilter` condition (missing cell, terminal cell,
  already-filtered cell, missing/mismatched argument cell, mismatched value type); the
  current-value-conforming behavior on attach (success case and the
  `Error::MethodFailed` rejection case from §3.2).
- `write()`: a filter silently conforming a value (stored value differs from the
  argument passed to `write`), and a filter rejecting a value (cell left completely
  unchanged — same `source`, same `strength`).
- `propagate()`: the three diagnostic outcomes (holds / `NotConformed` / `Failed`) for a
  cell produced by a method this round; confirming a filtered *source* cell (never
  touched by a method) never appears in `filter_violated_cells()`; confirming
  `propagate_without_replan()` leaves `last_filter_violations` untouched.
- `filter_args`, `filter_violation`, `filter_violated_cells`, `filter_violation_cells`:
  basic presence/absence and multi-cell aggregation cases mirroring the existing
  `condition_contributing_cells`/`output_violation_cells` tests.
