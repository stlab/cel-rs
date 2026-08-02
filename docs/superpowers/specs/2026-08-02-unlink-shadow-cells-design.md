# Automatic Shadow Values for Self-Reference and Conditional Forcing Design

**Date:** 2026-08-02
**Author:** Sean Parent
**Status:** Approved

## Overview

`execute_plan` currently overwrites a cell's only value slot (`CellData::value`) whenever a
method produces it. This is correct for ordinary derived cells, but loses information in two
cases:

1. **Self-referencing methods** — e.g. `method [a, b] -> [a] { min(a, b) }`. Once `a` is
   overwritten with `min(a, b)`, the next `propagate()` recomputes `min` against the *previous
   result*, not against the value the user actually set. `b` should apply constraining pressure
   against `a`'s original value every round, not against an ever-shrinking (or ever-growing)
   accumulator.
2. **Conditionally forced cells** — e.g. a branch active only when `p == 1` runs
   `method [b] -> [a] { b }`. While the branch is active, `a`'s value is `b`. Once `p` changes and
   the branch deactivates, `a` should revert to whatever it was before it was forced — not remain
   stuck at the last-forced value, which is what happens today since nothing ever restores it.

In Adobe Source Libraries' Adam, this is handled by an explicit `unlink` keyword that gives a
cell a shadow/source slot. Per this project's direction, `adam-rs` should detect both situations
automatically — no new DSL syntax, no opt-in call — the `Sheet` derives everything from
relationship and conditional structure it already tracks.

This design **supersedes** the "Migration path to a split-cell model" section of
[2026-06-25-self-reference-design.md](2026-06-25-self-reference-design.md#migration-path-to-a-split-cell-model).
That section proposed a `source_value` populated by `write()` alongside `value` (requiring a
clone of every written value). The design below instead uses two independently-owned slots that
are never copied into one another — `write()` and `execute_plan()` each own a different slot
outright — so no `Clone` bound is added anywhere.

## Semantics

Every cell has two conceptual slots:

- **`source`** — the value from the most recent `write()` (or the `add_cell` initializer). Set
  **only** by `write()`/`add_cell`. Never touched by `propagate()`.
- **`derived`** — the value most recently produced by a method, for this round, if any. Set
  **only** by `execute_plan()`, and reset to absent at the start of every `propagate()` call
  before planning begins.

`Sheet::read()` returns `derived` when present, else `source` — i.e. the "effective" value:
whatever a method most recently computed this round, or the user's own input if no method has
claimed the cell this round.

A cell's `derived` slot is populated for a given output cell of a firing method exactly when
either of the two cases applies to that specific `(relationship, method, output)` triple:

- **self-referencing**: the output is also one of that method's inputs
  (`method.inputs.contains(&output)`), or
- **conditionally forced**: the output is a *pure* output (not also an input) of a method whose
  relationship is registered under some `add_conditional` (`self.conditional_relationships.contains(&rel_id)`).

All other outputs (the common case: ordinary unconditional derived cells) continue to write
straight into `source`, exactly as `value` is written today — no behavior or performance change
for cells that need no shadowing.

This rule is evaluated fresh, per output, every time a method fires — there is no persistent
per-cell classification. A cell can be self-referencing in one relationship and a plain
conditionally-forced pure output in another (see "Overlapping cases" below); the correct slot is
chosen independently each round based on whichever method actually produces it that round.

### Self-referencing inputs always read `source`

When gathering a method's inputs, a self-referencing input (a cell that is also one of that same
method's outputs) is read from `source`, never from `derived`. Combined with the rule above (a
self-referencing output is *written* to `derived`, never to `source`), this means `source` is
simply never touched by any method for a self-referencing cell — it forever holds the user's last
explicit input, giving case 1's "pressure" semantics for free, with no restore step required
across any number of `propagate()` calls.

### Pure inputs read `derived.unwrap_or(source)` — and why the round-start reset makes this safe

Pure (non-self-referencing) inputs read the *effective* value: `derived` if some method has
already produced it this round, else `source`. This is only correct if `derived` cannot hold a
**stale** value left over from a previous round — which is why `derived` is unconditionally reset
to absent for every cell at the very start of `propagate()`, before Phase 1 planning begins (see
below). Because `execution_order` is already a valid topological order (a method that feeds
another method's input necessarily runs earlier — see the execution-order guarantee in
[2026-06-25-self-reference-design.md](2026-06-25-self-reference-design.md#execution-order-guarantee)),
by the time any method reads a pure input this round, that input's `derived` slot is either
freshly populated by an earlier step of *this same round*, or still absent because the cell is a
genuine source this round — never a leftover value from before the round started.

This reset is what replaces the (rejected) idea of restoring `derived` to `None` *after*
execution completes: doing it after execution is too late, because an in-round pure-input read
could observe the stale value before the restore ever runs. Resetting before planning starts
means there is nothing to restore — a cell that turns out to be a source this round was already
reset and never reclaimed.

### Overlapping cases

A single cell may be self-referencing in one relationship and a conditionally-forced pure output
in another, e.g.:

```
conditional p {
    0 => relationship {              // self-referential: a <= b
        [a, b] -> [a] { min(a, b) }
        [a, b] -> [b] { max(a, b) }
    }
    _ => relationship {              // plain pure-output forcing, either direction
        [b] -> [a] { b }
        [a] -> [b] { a }
    }
}
```

No special handling is needed: whichever method fires this round decides the slot per the rule
above (self-ref in the `p == 0` branch, conditional-pure-output in the default branch), and
`source` is left untouched by either branch either way.

## Data Model

`adam-rs/src/cell.rs` — `CellData` gains one field and one rename:

```rust
pub(crate) struct CellData {
    /// The value from the most recent `write()`/`add_cell`. Never written by `propagate()`.
    pub(crate) source: Box<dyn Any>,       // renamed from `value`
    /// The value most recently produced by a method this round, if this cell was shadowed.
    /// Reset to `None` at the start of every `propagate()`, before planning.
    pub(crate) derived: Option<Box<dyn Any>>,  // new
    // type_id, strength, changed, adj, eq_fn unchanged
}
```

No `Clone` bound anywhere: `source` is moved in once by `write()`; `derived` is moved in once by
`execute_plan()` from the method's own output. They are never copied into each other.

## Changes

### `Sheet::read()`

```rust
pub fn read<T: Any + 'static>(&self, id: CellId) -> Result<&T, Error> {
    let cell = /* ... */;
    let value: &dyn Any = cell.derived.as_deref().unwrap_or(cell.source.as_ref());
    Ok(value.downcast_ref::<T>().expect("type checked above"))
}
```

### `Sheet::source()` — new public accessor

```rust
/// Returns the last explicitly written (source) value, ignoring any derived override from
/// self-reference or conditional forcing.
pub fn source<T: Any + 'static>(&self, id: CellId) -> Result<&T, Error>
```

Lets UI code (e.g. `begin`) show both the constrained/forced value (`read()`) and the original
value it's being pulled from (`source()`), side by side.

### `Sheet::write()`

Sets `cell.source` (as `value` is set today) and additionally clears `cell.derived = None` — an
explicit write always takes immediate effect, matching today's "read-after-write before
propagate" behavior.

### `Sheet::propagate()`

New step at the very start, before Phase 1 pre-planning:

- Snapshot the set of cells currently holding `Some` in `derived` (needed only for change
  tracking, see below).
- Reset `derived = None` for every cell.

New step at the very end, after strength post-processing:

- For every cell in the snapshot whose `derived` is still `None` (it was not reclaimed by any
  method this round) and that isn't already marked `changed`, mark it `changed` and push it to
  `changed_cells`. This is the one case where a cell's effective value changes (forced value →
  source value) without any method writing to it this round, so it needs explicit bookkeeping to
  surface correctly through `Sheet::changed()`.

### `Sheet::execute_plan()`

Input gathering, per method input cell `id`:

```rust
if method.outputs.contains(&id) {
    cell.source.as_ref()                                  // self-referencing: always source
} else {
    cell.derived.as_deref().unwrap_or(cell.source.as_ref()) // pure input: effective value
}
```

Output writing, per method output cell `id` (with `rel_id` the relationship being executed):

```rust
let shadow = method.inputs.contains(&id) || self.conditional_relationships.contains(&rel_id);
if shadow {
    cell.derived = Some(new_value);
} else {
    cell.source = new_value; // today's behavior, unchanged
}
```

`changed` tracking is unaffected in shape — still set unconditionally whenever either slot is
written this round, matching today's "no equality check" semantics (documented on
`Sheet::changed()`).

## Non-Goals

- No new `adam-lang` syntax or opt-in mechanism — this is entirely automatic within `adam-rs`.
- No attempt to detect that a cell is *structurally always* forced across every branch of a
  conditional (which would make shadowing provably unnecessary for that cell). Per the original
  problem statement, an always-forced cell's `source` is simply never read in practice — shadowing
  it anyway is harmless (the restore-on-reset path never fires because the cell is never
  genuinely a source), just a small unused `Option` indirection.
- No validation added to reject cells that participate in both a self-referencing method and a
  conditionally-forced pure-output method across different relationships — this is an explicitly
  supported, in-scope combination (see "Overlapping cases").

## Test Plan

Derived from the contract above (no reference to implementation internals):

1. **Self-reference pressure persists across rounds without rewriting the anchor cell.**
   `method [a, b] -> [a] { min(a, b) }`. Write `a=10, b=3`, propagate, assert `a == 3`. Write only
   `b=20` (never rewrite `a`), propagate, assert `a == 10` (proves `a`'s original 10, not the
   previous derived 3, was used). Write only `b=5`, propagate, assert `a == 5`.
2. **`source()` exposes the original value while `read()` shows the constrained value**, in the
   scenario above: after `b=20` round, `sheet.source::<i32>(a) == 10` while
   `sheet.read::<i32>(a) == 10`; after `b=5` round, `sheet.source::<i32>(a) == 10` while
   `sheet.read::<i32>(a) == 5`.
3. **Conditional forcing reverts to the original value on deactivation**, not the last-forced
   value: write `a`'s initial value, activate the branch that forces `a` from `b`, propagate,
   assert `a == b`'s value. Deactivate the branch, propagate, assert `a` equals its original
   pre-force value (not `b`'s value, and not the value from any intermediate round).
4. **Explicit write to a currently-forced cell takes immediate effect and is not immediately
   reverted** by the next `propagate()` if the forcing relationship is still active (matches
   existing strength semantics — the forced relationship will simply re-force it next round,
   which is expected, not a regression).
5. **`changed()` reports the cell when a conditional deactivates**, even though no method wrote to
   it that round.
6. **Self-referencing input reads never see a stale cross-round `derived`** even when the
   self-referencing relationship shares cells with another relationship that writes `derived` for
   an unrelated reason in the same round (regression test for the staleness bug the round-start
   reset fixes).
7. **The overlapping self-ref / conditional-pure-output scenario** from "Overlapping cases" above:
   switching branches correctly re-derives the cell each round from whichever method is active,
   and `source()` remains the pristine original throughout both branches.
8. **Ordinary unconditional derived cells are unaffected**: existing tests
   (`strength_drives_method_selection`, `arity_3_2_1`, etc.) continue to pass unmodified,
   confirming zero behavior change for cells that need no shadowing.
