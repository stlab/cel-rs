# Tuple Value Display Design

**Status:** Approved
**Date:** 2026-08-12

## Problem

Tuple-typed cells never appear in `begin`'s Inspector sidebar. `begin/src/bridge.rs`'s
`labels_from_cell_names` explicitly skips any cell whose `TypeShape` is `Tuple(_)`:

```rust
let type_id = match shape {
    TypeShape::Named(type_id) => *type_id,
    TypeShape::Tuple(_) => continue,
};
```

This means an `out` cell declared with a tuple type (e.g. `out pair: (i32, i32) { ... }`) is
silently invisible in the UI, even though it computes correctly at runtime. There is also no way
today to format a `cel_runtime::DynamicSequence`'s actual element values — `DynamicSequence`
already implements `Debug`, but as a placeholder:

```rust
impl std::fmt::Debug for DynamicSequence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynamicSequence")
            .field("arity", &self.shape.len())
            .finish()
    }
}
```

which prints `DynamicSequence { arity: 2 }`, not the tuple's actual contents.

## Goal

Any tuple-typed cell (out or plain) displays its current value in `begin`'s Inspector as a string
matching exactly how Rust's own `{:?}` would format the equivalent concrete tuple — `(3, 4.5)`,
`(1, "hello")`, `(1, (2.5, "x"))`, `()` for the empty tuple, `(3,)` for a 1-tuple (trailing comma,
matching real Rust anonymous-tuple `Debug`, not tuple-struct `Debug`). No write/edit support is
added in this pass; a tuple field's existing `SpTextfield` keeps its full `invalid`/`warning`/
`disabled` visual affordance (unchanged, no new UI logic needed), and any edit attempt fails
cleanly, mirroring how a scalar `out` cell already behaves when written to (a `Sheet::write` on a
terminal cell already returns `Error::TerminalCell`, which the Inspector already surfaces via its
existing invalid/revert-on-blur flow).

Editing tuple-typed cells (parsing user input back into a `DynamicSequence` and writing it) is
explicitly out of scope — tracked as a follow-up GitHub issue instead.

## Non-goals

- No tuple-literal parser/editor in this pass.
- No change to `CellRow`/`CellFlags`/`SpTextfield` — the existing disabled/invalid/warning
  machinery is already generic over `CellId` and needs no tuple-specific logic.
- No pretty/multi-line (`{:#?}`) formatting — single-line only, matching every other cell's
  Inspector display.

## Part A — `cel-runtime`: real `Debug` for `DynamicSequence`

### New primitive: `ElementDebug`

Alongside the existing `ElementDropper`/`ElementCloner`/`ElementEq` (`cel-runtime/src/dynamic_sequence.rs`):

```rust
/// Debug-formats a value in place, given a pointer to its bytes.
///
/// # Safety
/// `ptr` must point to a valid, live, properly aligned value of the type this formatter was
/// generated for.
pub type ElementDebug = unsafe fn(*const u8, &mut std::fmt::Formatter<'_>) -> std::fmt::Result;

/// Returns an [`ElementDebug`] that debug-formats a value of type `T` in place.
pub fn element_debug_for<T: 'static + std::fmt::Debug>() -> ElementDebug {
    |ptr, f| unsafe { std::fmt::Debug::fmt(&*ptr.cast::<T>(), f) }
}
```

### `SequenceElement` / `DynElementSpec` gain a `debug` field

Both structs gain `pub debug: ElementDebug`, populated everywhere their sibling `drop`/`clone`/`eq`
fields already are:

- `push_element<T: 'static + Clone + PartialEq>` (the generic static-tuple-arity constructor) →
  bound gains `+ Debug`; populates `debug: element_debug_for::<T>()`.
- `DynamicSequence::from_dyn_elements`'s per-element `SequenceElement` construction copies
  `spec.debug` through, same as it already copies `spec.drop`/`spec.clone`/`spec.eq`.

### Bound propagation

`push_element`'s new `+ Debug` bound ripples exactly where `Clone + PartialEq` already sit, since
every tuple element ultimately funnels through it:

- `impl<H: 'static + Clone + PartialEq, T: SequenceList> SequenceList for (H, T)` → `H: ... + Debug`.
- All twelve `TupleSequence` arity impls (`(A,)` through 12-tuples) → each type parameter gains
  `+ Debug`.

This is purely additive: every type this codebase currently constructs a `DynamicSequence` from
(`i8`..`i128`, `u8`..`u128`, `f32`/`f64`, `bool`, `String`, and `DynamicSequence` itself for nested
tuples) already implements `Debug`. No existing caller changes.

### The real `Debug` impl

Replaces the placeholder. Hand-written against `Formatter` (not `f.debug_tuple()`, which produces
tuple-*struct* formatting — no trailing comma on a single field — not real anonymous-tuple
formatting, where `(3,)` needs that comma):

```rust
impl std::fmt::Debug for DynamicSequence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(")?;
        for (i, elem) in self.shape.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            unsafe {
                self.buffer.read_at(elem.offset, |ptr| (elem.debug)(ptr, f))?;
            }
        }
        if self.shape.len() == 1 {
            f.write_str(",")?;
        }
        f.write_str(")")
    }
}
```

A nested tuple element is itself stored as a `DynamicSequence` value (confirmed: `default_dyn_element`
and the runtime tuple-construction paths already store a nested tuple element with
`type_id: TypeId::of::<DynamicSequence>()` and `DynamicSequence`'s own drop/clone/eq/write function
pointers) — so `element_debug_for::<DynamicSequence>()` recurses through this same `Debug` impl with
no special-casing.

### `call_dyn_as_dynamic_sequence`'s per-leaf callback

The callback that builds a live, output-bound `DynamicSequence` (used for a method's *result* — the
value `begin` actually reads and displays) currently has shape `Fn(TypeId) -> Option<(ElementDropper,
ElementCloner, ElementEq)>`. It grows a fourth tuple element: `Option<(ElementDropper, ElementCloner,
ElementEq, ElementDebug)>`. `push_arg_as_dynamic_sequence_tuple`'s `AssociatedType`-based path (an
*ephemeral, on-stack* tuple built for a method's *input*, never persisted past one evaluation, never
displayed) is untouched — it has no use for `ElementDebug`.

## Part B — `adam-lang`: `TypeRegistry` wiring

- **`TypeEntry`** gains `pub element_debug: cel_runtime::ElementDebug`, populated in both
  `register`/`register_no_default` via `cel_runtime::element_debug_for::<T>()`.
- **`register`/`register_no_default`**'s generic bound gains `+ std::fmt::Debug`. Every built-in
  primitive `TypeRegistry::new()` registers already satisfies this — no change needed there.
- **`TypeRegistry::element_descriptor`** — currently `Option<(ElementDropper, ElementCloner,
  ElementEq)>` — grows to `Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)>`. Its
  callers (`parser.rs`'s `eval_segment_boxed`, building the `leaf` closure for a cell initializer's
  eager tuple evaluation; and `element_descriptors_for`, used by `parse_out_decl`/`parse_method_body`'s
  `SingleTuple`/`Tuple` output paths — the live-recomputed value `begin` displays for a tuple `out`)
  thread the extra element through mechanically.
- **`default_dyn_element`** (builds a `DynElementSpec` for a tuple cell's *default* value) sets
  `.debug: entry.element_debug` for a leaf, and `cel_runtime::element_debug_for::<cel_runtime::DynamicSequence>()`
  for a nested tuple — mirroring the existing `drop`/`clone`/`eq`/`write` lines immediately above it.

This is additive, mechanical threading through already-existing call sites — the same shape as
Tasks 4–5 of the prior `2026-08-11-adam-lang-tuple-types` plan.

## Part C — `begin`: display + the follow-up issue

### `bridge.rs`

`labels_from_cell_names` drops its `TypeShape::Tuple(_) => continue` skip. For a tuple-typed cell
(any tuple cell — out or plain; no output-cell identity plumbing needed), it calls a new method:

```rust
impl Labels {
    /// Registers display-only metadata for a tuple-typed cell of any shape.
    ///
    /// `write_str` always returns `Err` — no tuple-literal parser exists yet (tracked as a
    /// follow-up: see the GitHub issue referenced in this crate's docs). The field still
    /// participates fully in the Inspector's existing invalid/warning/disabled machinery,
    /// since that's entirely keyed on `CellId`, not on any per-type behavior.
    pub fn add_tuple_cell(&mut self, id: CellId, label: &str) {
        self.cells.insert(
            id,
            CellMeta {
                label: label.to_owned(),
                display: Box::new(move |sheet| {
                    sheet
                        .read::<cel_runtime::DynamicSequence>(id)
                        .map(|v| format!("{v:?}"))
                        .unwrap_or_else(|_| "?".to_owned())
                }),
                write_str: Box::new(|_sheet, _s| {
                    Err(Error::MethodFailed(anyhow::anyhow!(
                        "editing tuple-typed cells is not yet supported"
                    )))
                }),
            },
        );
    }
}
```

`labels_from_cell_names`'s `match shape { TypeShape::Tuple(_) => continue, ... }` becomes
`TypeShape::Tuple(_) => { labels.add_tuple_cell(id, name); continue; }`.

### No changes to `CellRow` / `CellFlags` / `SpTextfield`

`cell_flags` is already generic over `CellId` — a tuple cell backing an `out` gets `invalid`/
`warning` exactly like a scalar `out` cell today (both driven by `OutputStatus`, computed from
`Sheet::output_valid`/`Sheet::output_violation_cells`, agnostic of the cell's Rust type). An edit
attempt on a tuple field calls the always-`Err` `write_str`, which flips `has_error` exactly as a
real `Error::TerminalCell` would for a scalar out cell — same error-then-revert-on-blur UX, no new
code path.

### Doc updates

- `labels_from_cell_names`'s doc comment loses its "and any tuple-typed cell ... not yet supported
  ... silently skipped" line.
- `CellMeta::write_str`'s doc comment gains a note that a cell type may have no real write support
  yet (always returns `Err`).

### Follow-up GitHub issue

Filed against `stlab/cel-rs`: "Support editing tuple-typed cells in `begin`." Scope: a way to parse
user input into a `DynamicSequence` matching a cell's declared `TypeShape` (arity + per-element leaf
type, recursively for nested tuples), then call `Sheet::write`. References this design doc and notes
the starting state (Debug-only display, `write_str` always errors).

## Testing

- **`cel-runtime`**: unit tests for `element_debug_for` (mirroring `element_eq_for`'s own test
  shape); `Debug` impl tests at arity 0 (`"()"`), 1 (`"(3,)"`), 2+ (`"(3, 4.5)"`), and nested
  (`"(1, (2.5, \"x\"))"`) — both via `DynamicSequence::from_tuple` and `from_dyn_elements`, so both
  construction paths are covered.
- **`adam-lang`**: a `TypeRegistry::new()` test asserting `entry.element_debug` produces the
  expected text for a registered primitive; a `default_dynamic_sequence`/`element_descriptor` test
  confirming the fourth tuple element round-trips correctly.
- **`begin`**: `bridge.rs` unit tests for `add_tuple_cell` (mirroring
  `display_closure_returns_value_string`'s existing shape) — `display` returns the expected
  Rust-tuple-formatted string for a built `Sheet`; `write_str` always returns `Err` without
  mutating the sheet. `labels_from_cell_names_builds_entries_for_supported_types`-style test
  updated (or a new sibling test added) to confirm a tuple-typed cell now appears in `Labels`
  instead of being skipped.
- **UI verification**: per `begin/CLAUDE.md`, actually render the Inspector with a tuple-typed
  `out` cell (including one with a violated condition, to confirm the warning border still shows)
  using the `verifying-begin-ui` skill — passing `cargo build`/`clippy` does not prove anything
  renders correctly.
