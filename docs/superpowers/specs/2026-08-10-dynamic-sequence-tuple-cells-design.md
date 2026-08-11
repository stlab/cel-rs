# `DynamicSequence`: type-safe CEL tuple cells (Rust API layer)

## Problem

adam-lang cannot currently declare a cell with a CEL tuple type, either explicitly
(`cell a: (i32, i32);`) or by deduction from an initializer (`cell a = (1, 2);`). Both fail today.

This spec covers only the **Rust API layer** the adam-lang grammar/parser will eventually build
on: making it possible, from Rust code (no adam-lang text involved), to hold a CEL tuple value in
an `adam-rs` cell and to use it — via a concrete Rust tuple type — from ordinary
`adam_rs::Method`/`Condition` closures. Extending adam-lang's grammar and `TypeRegistry` to parse
and dispatch tuple syntax from DSL text is explicitly deferred to a follow-up; see "Out of scope."

## Background: what already exists

`cel-runtime`/`cel-parser` already fully support tuples *at the CEL expression level*:

- Tuple literals (`(1, 2)`, including nested tuples), `.N` field indexing, and a
  `TupleOpSignature`-based dispatch mechanism for tuple-shaped operator overloads
  (`cel-parser/src/op_table.rs`).
- The runtime representation lives entirely on `DynSegment`'s `RawStack`: `DynSegment::make_tuple`
  lays out elements at ascending, self-contained (zero-based), naturally-aligned offsets — the same
  convention a `#[repr(C)]` struct in declaration order would use — recorded in a `StackInfo` whose
  `associated: Vec<AssociatedType>` describes each element's `TypeId`, name, offset, size, align,
  and an in-place dropper (`cel-runtime/src/dyn_segment.rs`). `DynSegment::tuple_index` extracts one
  element; `DynSegment::{push_tuple, pop_tuple_as}` relabel a concrete `CStackList<...>` chain as a
  tuple and back, with no bytes moved.
- `RawStack` (`cel-runtime/src/raw_stack.rs`) already provides the type-erased, runtime-offset-driven
  primitives this design reuses: `push_raw`, `copy_from`, `drop_at`, `truncate_to`, `repack`. Its
  `with_base_alignment` takes an arbitrary, caller-computed alignment — there is no hardcoded
  constant anywhere in `RawStack`.

None of this reaches `adam-rs`. `adam-rs` has **zero dependency on `cel-runtime`/`cel-parser`** by
design (per the workspace `CLAUDE.md`: it is "the constraint-graph runtime," CEL-agnostic). Its
core API is already fully generic:

- `Sheet::add_cell<T: Any + PartialEq + 'static>(value: T) -> CellId`
- `Method::from_fn_1_1<A, B, F>(...)`, `Condition::from_fn_1<A, F>(...)`, etc., where `A`/`B` are
  ordinary Rust types matched by exact `TypeId` equality against the cell's fixed, registered type.

Nothing here needs to change for a new type to work as a cell value — `T` can already be anything
that is `Any + PartialEq + 'static`. The actual gap is that there is currently no type that can hold
"a CEL tuple whose shape is only known at runtime" persistently (a `DynSegment`'s on-stack tuple is
transient — it doesn't outlive that segment's evaluation), nor a type-safe way to convert such a
value to/from a concrete Rust tuple.

### Why not `RawSequence`

`cel-runtime` already has a different type named `RawSequence` (`cel-runtime/src/raw_sequence.rs`),
used by `RawSegment` purely to store compiled op closures. It was considered and rejected as a
foundation for this work:

- Its `push<T>`/`next<T>`/`drop_in_place<T>` all require `T` as a **static Rust generic** at the
  call site — incompatible with a shape discovered only at runtime from a `TypeId` list.
- `RawSequence::new()` hardcodes `RawVec::with_base_alignment(4096)` — every instance reserves up to
  ~4095 bytes of alignment slop regardless of its actual elements' alignment needs, which would be
  wasteful if every tuple-typed cell got one.

`RawStack` was chosen instead: it already has the type-erased raw primitives this design needs, and
already supports precise, per-instance alignment sizing. It does carry LIFO-specific complexity
(padding-byte bookkeeping for `pop`) that a build-once/read-many/drop-once value doesn't need, but
using it as-is is the shortest path to a working implementation. Revisiting the container
primitives (and, at the same time, whether the CEL tuple representation itself should change) is
tracked separately in
[stlab/cel-rs#80](https://github.com/stlab/cel-rs/issues/80) and is explicitly out of scope here.

## Goals

- A new `DynamicSequence` type in `cel-runtime` that can hold a CEL tuple value of a shape
  determined at runtime, own it persistently (independent of any `DynSegment`'s lifetime), and be
  stored directly as an `adam_rs::Sheet` cell's value with no changes to `adam-rs`.
- Type-safe, by-value conversion between `DynamicSequence` and concrete, nestable Rust tuples of
  arbitrary arity (matching whatever arity range `cel-runtime`'s existing `IntoList` blanket impls
  already support).
- A one-allocation extraction path from a live `DynSegment` evaluation's on-stack tuple into an
  owned `DynamicSequence`.
- An ergonomic adapter (`adapt_fn_1`) so a plain closure typed at a concrete tuple
  (`Fn(&(i32, f64)) -> Result<R, _>`) can be wired directly into the existing, unmodified
  `Method::from_fn_1_1::<DynamicSequence, R, _>` / `Condition::from_fn_1::<DynamicSequence, _>`,
  without hand-writing the shape-check-and-convert boilerplate at every call site.
- Demonstrate the whole path end-to-end purely through direct Rust API calls — an `adam_rs::Sheet`
  with a `DynamicSequence`-typed cell, a `Method`/`Condition` built via `adapt_fn_1`, and
  `propagate()` producing the right result. No adam-lang text parsing involved.

## Non-goals

- Extending adam-lang's grammar/parser or `TypeRegistry` to accept tuple syntax in DSL text
  (`cell a = (1, 2);`, `cell a: (i32, i32);`). This is the deferred follow-up this spec unblocks.
- Any change to `adam-rs` itself. `Sheet`, `Method`, `Condition`, `Relationship` are already generic
  enough; none of them need to know `DynamicSequence` exists.
- Redesigning `RawStack`/`RawSequence` or the on-stack CEL tuple representation
  (`make_tuple`/`tuple_index`/`AssociatedType`) — tracked in
  [stlab/cel-rs#80](https://github.com/stlab/cel-rs/issues/80).
- `adapt_fn_2`, `adapt_fn_2_1`, and other multi-input/multi-output adapter variants. Only
  `adapt_fn_1` is in scope now (it's what the acceptance test needs); later adapters follow the
  same pattern and can be added on demand.

## Design

### `DynamicSequence`

A new, owned, type-erased tuple value, in a new `cel-runtime` module (e.g. `dynamic_sequence.rs`):

- Internally holds a `RawStack`, sized via `RawStack::with_base_alignment(max_element_align)`
  computed from its *own* elements (never a blanket constant), plus an ordered shape descriptor —
  a new struct (e.g. `SequenceElement`) distinct from `dyn_segment::AssociatedType`, carrying
  `type_id`, `type_name`, `offset`, `size`, `align`, and three function pointers: `drop`, `clone`,
  and `eq`. It is a deliberate, separate type rather than an extension of `AssociatedType`: the
  latter describes a *transient* on-stack tuple element (drop-only), while `DynamicSequence`'s
  elements must additionally support `Clone` and `PartialEq` to satisfy `Sheet::add_cell`'s bounds
  (see "Where the `PartialEq` bound comes from," below) — entangling the two would force every
  transient, drop-only on-stack tuple to also carry unused clone/eq function pointers.
- Uses only `RawStack`'s raw/offset-based primitives (`push_raw`, `copy_from`, `drop_at`) — never
  its typed `push<T>`/`pop<T>`, since element types are known only from the runtime shape
  descriptor, not as Rust generics. No LIFO discipline is used: the sequence is built once (all
  elements written during construction, at precomputed, ascending, self-contained offsets — the
  same convention `make_tuple` already uses), read arbitrarily many times by offset, and dropped
  exactly once.
- Implements:
  - `Drop`: runs each element's `drop` fn pointer in reverse order (mirroring the existing
    `drop_tuple` pattern in `dyn_segment.rs`).
  - `Clone`: allocates a new `RawStack` of the same shape and runs each element's `clone` fn
    pointer to populate it.
  - `PartialEq`: `false` on shape mismatch (different arity or `TypeId` sequence); otherwise
    elementwise via each element's `eq` fn pointer.

### Extraction from a live CEL evaluation

`DynSegment` gains a method (exact name decided during planning, e.g.
`extract_tuple_as_dynamic_sequence`) with the precondition that `peek_tuple_arity()` is `Some`.
Because `make_tuple`'s offsets are already zero-based and self-contained (independent of the
segment's ambient stack depth), extraction is exactly one allocation plus one `copy_from` of the
tuple's `size` bytes into the new `DynamicSequence`'s `RawStack`, reusing the existing
`AssociatedType` list to build the `SequenceElement` shape (this is where the `clone`/`eq` function
pointers must be newly supplied — `AssociatedType` doesn't carry them, so this path requires the
element's concrete type to be known via the same `TypeId`-keyed mechanism `push_tuple`/`pop_tuple_as`
already use to bridge to concrete Rust types). The original on-stack bytes are then truncated off
`DynSegment`'s stack without invoking their droppers a second time (ownership transferred, not
duplicated).

### Conversions to/from Rust tuples

Generic, nestable, by-value conversions — modeled on the existing `IntoList`/`IntoTupleList`
blanket-impl pattern for tuples (`cel-runtime/src/list_traits.rs`, `tuple_list.rs`), covering the
same arity range those already support:

- `DynamicSequence::from_tuple<T>(value: T) -> Self` — decomposes `T`'s fields by value into a
  fresh `DynamicSequence`.
- `DynamicSequence::try_into_tuple<T>(self) -> Result<T, anyhow::Error>` — consumes `self`, checks
  the element `TypeId` sequence against `T`'s, and moves each field out by value on success.
- `DynamicSequence::try_to_tuple<T>(&self) -> Result<T, anyhow::Error>` — same shape check, but
  **clones** each field out instead of consuming `self` (requires `T`'s element types to be
  `Clone`, which `DynamicSequence`'s own `Clone` impl already requires of every element anyway).

Nesting requires no special handling: a nested tuple element (e.g. `(i32, (i32, i32))`) is just "one
element whose `TypeId` happens to be a tuple type" — the shape check compares `TypeId`s, not
recursive structure, so a concrete nested tuple type is opaque to it either way. (A tuple whose
shape is *itself* only known at runtime nests as an *element* that is another `DynamicSequence`,
which needs no special-casing either — it's simply an element of type `DynamicSequence`.)

Conversions never use `mem::transmute` (or any raw reinterpretation) into an actual native Rust
tuple: Rust's tuple field layout is unspecified, so the only sound way to produce a `(A, B, ...)`
is to move or clone each field into it by value — exactly what `cel-runtime`'s own `CStackList`
code already does (it transmutes only between its own `#[repr(C)]`-equivalent types, never into a
bare native tuple).

### `adapt_fn_1`

```rust
impl DynamicSequence {
    /// Wraps a closure over a concrete tuple `A` so it can be used directly as the `F` in
    /// `adam_rs::Method::from_fn_1_1::<DynamicSequence, R, _>` or
    /// `adam_rs::Condition::from_fn_1::<DynamicSequence, _>`.
    pub fn adapt_fn_1<A, R, F>(f: F) -> impl Fn(&DynamicSequence) -> Result<R, anyhow::Error>
    where
        F: Fn(&A) -> Result<R, anyhow::Error>,
        /* A: bounds matching try_to_tuple's requirements */
    {
        move |seq: &DynamicSequence| {
            let a: A = seq.try_to_tuple()?;
            f(&a)
        }
    }
}
```

Built directly on `try_to_tuple` — every call clones the tuple's elements into a fresh, temporary
`A`, calls `f`, and drops the temporary. This is the only shape that works: `Method`/`Condition`
give the closure a `&A` derived from `args[0].downcast_ref::<A>()`, so the adapter must hand back a
real `&A` — which means materializing an owned `A` from the borrowed `&DynamicSequence` (cloning),
not consuming the cell's stored value.

### Where the `PartialEq` bound comes from

`Sheet::add_cell<T: Any + PartialEq + 'static>` requires `PartialEq` not for general change
detection (`CellData.changed` is a plain `bool` set directly by `write`/`propagate`) but
specifically for `Sheet::add_conditional`: `CellData.eq_fn` (captured from `T`'s own `PartialEq` at
`add_cell` time) is what `Sheet::build_active_set` (`adam-rs/src/sheet.rs:794-799`) uses to compare
a conditional's match-cell value against each branch's key values, to decide which branch's
relationships are active. So `DynamicSequence: PartialEq` is what allows a tuple-typed cell to serve
as a conditional's match cell — the acceptance tests should exercise this path explicitly, not just
`Method`/`Condition` reads.

## Error handling

Shape mismatches (`try_into_tuple`/`try_to_tuple` called with a `T` whose `TypeId` sequence doesn't
match the `DynamicSequence`'s actual elements) return `Err` via `anyhow::Result`, consistent with
the existing tuple-mismatch error conventions elsewhere in `cel-runtime` (e.g. `pop_tuple_as`'s
`ensure!` checks). Per the workspace's contract-style doc convention, this is documented as an
`# Errors` case, not a precondition — it's driven by data (the tuple's actual runtime shape), not a
caller-side invariant to `debug_assert!`.

## Testing strategy

Per the workspace's contract-only testing convention, tests are derived from each new function's
public contract:

- `DynamicSequence` construction/extraction round-trips for several arities, including a nested
  tuple (e.g. `(i32, (i32, i32))`), via `from_tuple`/`try_into_tuple`/`try_to_tuple`.
- Shape-mismatch `Err` cases for `try_into_tuple`/`try_to_tuple` (wrong arity, wrong element type at
  some position).
- `Clone`/`PartialEq`/`Drop` correctness, including a `DropCounter`-style test (matching the
  existing pattern in `dyn_segment.rs`'s tuple tests) verifying every element is dropped exactly
  once, in both the plain-drop and post-`Clone` cases.
- Extraction from a live `DynSegment`: build a CEL tuple via `make_tuple` (or by parsing a literal
  through `cel-parser`), extract it into a `DynamicSequence`, and verify the values round-trip
  correctly with no double-free (drop-counter check).
- `adapt_fn_1` end-to-end, exercising both call paths through unmodified `adam-rs`:
  - A `Method::from_fn_1_1::<DynamicSequence, _, _>` built via `adapt_fn_1`, registered on an
    `adam_rs::Sheet` cell holding a `DynamicSequence` (built via `DynamicSequence::from_tuple`),
    driven through `Sheet::propagate()`, producing the expected output.
  - A `DynamicSequence`-typed cell used as an `add_conditional` match cell, verifying `PartialEq`
    correctly selects the matching branch.

## Out of scope (deferred)

- adam-lang grammar/parser support for tuple literals and type annotations
  (`cell a = (1, 2);`, `cell a: (i32, i32);`) and the corresponding `TypeRegistry` wiring to
  register arbitrary, runtime-discovered tuple shapes. This is the natural next step once this spec
  lands, but is a separate piece of work.
- Revisiting `RawStack`/`RawSequence` and the broader CEL tuple representation as a "family of
  heterogeneous type-safe containers" — tracked in
  [stlab/cel-rs#80](https://github.com/stlab/cel-rs/issues/80).
- `adapt_fn_2`, `adapt_fn_2_1`, and other multi-arity adapter variants beyond `adapt_fn_1`.
