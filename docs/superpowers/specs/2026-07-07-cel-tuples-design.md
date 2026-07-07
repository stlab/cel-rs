# Design: Tuple Types in CEL

**Date:** 2026-07-07
**Branch:** `worktree-cel-tuples`

## Summary

Add tuple literals (`("Hello", 42)`) and `.N` field indexing to CEL, backed by a
type-erased, in-place runtime representation on `RawStack` — no heap allocation for
the tuple value itself. Interop with user-supplied Rust operations that take or
return a tuple goes through a generalization of the existing built-in operator
dispatch (`OpSignature`/`BuiltinScope`), bridging to a concrete `repr(C)` Rust type
(e.g. `CStackList<...>`) rather than a plain Rust tuple, because plain Rust tuples
have unspecified (`repr(Rust)`) layout and can't be soundly reinterpreted from raw
bytes.

This spec covers the interpreted `DynSegment` backend only — the only backend that
exists today (see `docs/VISION.md`). First-class functions, arrays/vecs, and
method-call syntax are related future directions but out of scope here; the
metadata introduced by this design (`AssociatedType`) is shaped so those features
can reuse it later.

## Background

Three existing pieces of the runtime are directly relevant:

- **`AssociatedType`** (`cel-runtime/src/dyn_segment.rs`) already exists as a
  placeholder — "reserved for parse-time call checking and richer error reporting
  ... tuple elements ... first-class functions" — but today carries only
  `type_id`/`type_name`/`associated: Vec<AssociatedType>`, with no offset or
  dropper. It's unused.
- **`CStackList<H, T>`** (`cel-runtime/src/c_stack_list.rs`) is a `repr(C)`,
  tail-first cons-list with typenum-based indexing and existing conversions
  to/from real Rust tuples (`IntoCStackList`). A test in that module proves
  transmute-compatibility with a `#[repr(C)]` struct of matching field order.
- **`op1`/`op2`/`op3`** on `DynSegment` are Rust-generic over any concrete
  `'static` type known at the call site (e.g. the hand-written signature tables
  in `cel-parser/src/op_table.rs`). CEL tuple element types are discovered only
  at *parse time*, from arbitrary script text, with unbounded shape combinations
  — there is no finite set of `(A, B, ...)` combinations to pre-enumerate in Rust
  source the way `op_table.rs` enumerates `u8 + u8`, `u8 + u16`, etc. Tuple
  construction and indexing therefore can't be ordinary generic ops; they need a
  new runtime, byte-level, type-erased primitive that carries its own metadata —
  which is exactly what `AssociatedType` was reserved for.

A key correctness constraint (caught during design review): a tuple's internal
layout (the offsets between its elements) must depend only on the tuple's own
element types, never on how much was already on the ambient stack when
construction started. Naively pushing elements with `RawStack::push`'s ordinary
alignment (relative to the *current* stack length) would give a layout that
varies with stack depth — unusable for matching against a fixed concrete
`repr(C)` type at interop boundaries.

## Grammar

```
tuple_or_group = ")"
                | or_expression ["," [ or_expression { "," or_expression } ]] ")"
```

Extends the existing `(` handling in `is_primary_expression`:

- `()` → unit (unchanged).
- `(x)` → grouping (unchanged) — a single expression with no comma is never a
  tuple.
- `(x,)` → 1-tuple. The trailing comma is *mandatory* here, exactly as in Rust,
  because `(x)` already means grouping — there's no ambiguity-free way to spell a
  1-tuple otherwise.
- `(x, y)`, `(x, y, z)`, ... → tuple, arity ≥ 2. **No trailing comma allowed** in
  this branch: inside the `{ "," or_expression }` loop, a comma always means
  "another element follows," never "maybe end." This keeps every decision point
  resolvable with a single token of lookahead (the same reason `parameter_list`
  already disallows trailing commas today) and avoids the "did we just consume a
  separator or a terminator" ambiguity a fully-optional trailing comma would
  introduce.

Indexing extends `is_postfix_expression`:

```
postfix_expression = primary_expression { "(" parameter_list ")" | "." unsuffixed_integer }.
```

`.N` requires the top-of-stack entry to carry tuple metadata (see below) with at
least `N + 1` elements. Because tuple shape is always known at parse time, an
out-of-range or non-tuple `.N` is a **parse-time** error, never a runtime one.

Method-call syntax (`.name(...)`) is explicitly out of scope — VISION.md lists
"method calls" as a separate future direction.

## Runtime Representation

Extend `AssociatedType` with the two fields it's missing:

```rust
pub struct AssociatedType {
    pub type_id: TypeId,
    pub type_name: Cow<'static, str>,
    pub offset: usize,           // new: byte offset from the tuple's own base
    pub dropper: fn(&mut RawStack), // new: reuses the same per-type dropper push_type generates
    pub associated: Vec<AssociatedType>,
}
```

A tuple's own `StackInfo` entry uses a marker type for `type_id`
(`TypeId::of::<DynTuple>()`) — the tuple's real type identity is the *ordered
sequence* of element `AssociatedType`s, not one flat `TypeId`, so flat-`TypeId`
comparisons (`pop_types`, `call0::<R>()`, etc.) must special-case `DynTuple` and
compare the `associated` shape instead when they encounter one.

This is deliberately the same shape a later first-class-function design could
reuse for argument-list metadata, or an array design for a repeated-element
descriptor — but neither is built now.

## Construction

Parse each element normally — this is already exactly how `parameter_list`
evaluates call arguments today, nothing new there. By the closing `)`, all `N`
element types/sizes/alignments are known at parse time, so the parser computes
the *ideal* self-contained layout (offsets computed from a hypothetical zero
base, using the same `align_index` math `RawStack`/`RawSequence` already use) as
constants baked into one runtime op. That op:

1. Reserves enough leading padding to align the tuple's start to its own max
   element alignment.
2. Moves each already-evaluated element from its ambient (context-dependent)
   offset to its ideal (context-independent) offset, via the new `push_raw`
   primitive (below) rather than ad hoc `ptr::copy` calls.
3. Adjusts the tracked stack length and replaces the `N` individual `StackInfo`
   entries with one aggregate entry carrying the `associated` list.

No heap allocation: the repack happens in place within the existing `RawStack`
buffer.

### New primitive: `RawStack::push_raw`

`RawStack::push<T>` is generic over a compile-time `T`. Construction (and
indexing, below) need a type-erased equivalent:

```rust
/// Pushes `size` bytes from `src`, aligned to `align`, using the same
/// padding/marker-byte bookkeeping as `push::<T>`.
///
/// # Safety
/// `src` must be valid for `size` bytes and not overlap the stack's own buffer
/// in a way that `push::<T>` wouldn't already guard against.
pub unsafe fn push_raw(&mut self, align: usize, size: usize, src: *const u8) -> bool
```

Both the construction repack and `.N` indexing route through this one
primitive instead of separate raw-pointer logic.

## Indexing (`.N`)

An earlier version of this design assumed elements below the target index could
be `drop_in_place`d without being removed from the stack, since dropping them
"only" wastes space. **That's wrong**: leftover bytes below the target corrupt
`RawStack::pop`'s backward marker-byte scan for whatever pops *next* — the scan
looks for a specific padding-marker pattern to compute how much padding preceded
a value, and garbage left behind by a manually-destroyed element isn't a valid
marker. Concretely, `5 + (0, 1).1` must still correctly pop `5` after evaluating
`.1`, and it wouldn't if `0`'s dead bytes were left sitting under `1`.

The corrected algorithm pops the whole tuple and pushes just the result:

1. Copy the target element's raw bytes (known fixed offset/size from
   `associated`) into a small scratch buffer, *before* dropping anything.
2. `drop_in_place` every other element at its own known offset (the target's
   bytes were only copied, not consumed, so it's skipped here).
3. Truncate the stack back to exactly where it was before the tuple started,
   using the tuple's own aggregate `padding` flag — the same bookkeeping any
   ordinary pop already uses, since that marker was correctly established when
   the tuple was originally pushed as a unit.
4. Push the scratch bytes back via `push_raw`, computing correct
   alignment/padding for the *current* (now properly restored) stack position.

This leaves no unreclaimed dead space and keeps every subsequent pop's
marker-byte scan valid. Reclaiming space via compaction instead of a full
pop-and-repush is a possible future optimization, not needed for correctness.

## Interop / Op Registration

Per the "generalize existing mechanisms, don't bolt on a new one" direction:

- **Matching:** `OpSignature` (`cel-parser/src/op_table.rs`) currently matches an
  operand by one flat `TypeId`. Add a second operand-shape kind — "tuple of
  `TypeId`s `[T0, T1, ...]` in order" — matched against a stack slot's
  `associated` list instead of its flat `type_id`. `BuiltinScope::lookup`'s scan
  extends naturally to check either kind per operand position.
- **Registration surface:** a public API alongside `push_scope` lets host code
  (pm-lang or elsewhere) register `(shape: &[TypeId], op_fn)` entries scanned the
  same array-scan way `BUILTINS`/`ADD_SIGNATURES` are scanned today — user
  tuple-typed ops go through the same dispatch as built-ins, not a side
  mechanism.
- **Bridging to a concrete Rust type:** a registrant's `op_fn` wants a genuine
  concrete `T` (a `CStackList<...>` chain, or their own `#[repr(C)]` struct) to
  write ordinary Rust code against — including converting to a plain Rust tuple
  *inside* their own function body, which is sound there since at that point
  it's an ordinary in-process value with compiler-chosen layout, not a
  cross-boundary reinterpretation. One new `DynSegment` primitive,
  `pop_tuple_as::<T: 'static>()`, derives `T`'s expected element-`TypeId` shape
  via the existing `ToTypeIdList` trait (already used by
  `DynSegment::new::<Args>()` for argument seeding), checks it against the
  runtime tuple's `associated` shape, and — if they match — does a raw byte
  reinterpretation. This is sound specifically because both sides use the same
  natural-alignment, declaration-order layout established above; it would *not*
  be sound against a plain Rust tuple type, whose layout is unspecified. The
  reverse, `push_tuple::<T: IntoCStackList>`, builds the outgoing
  `AssociatedType` list from `T`'s own `ToTypeIdList` output for ops that return
  a tuple.

This reuses `OpSignature`-style scanning, `ToTypeIdList`, and `CStackList`
rather than inventing new interop machinery.

## Testing

Contract-only, per this repo's convention (derived from the public
interface/grammar, not implementation internals):

- **`cel-parser` grammar:** `()`, `(x)` vs `(x,)` vs `(x,y)`/`(x,y,z)`,
  missing/extra commas, `.N` on tuples of various arities, parse-time errors for
  `.N` on non-tuples and out-of-range `N`.
- **Indexing combined with another operation** — the case that caught the
  original design flaw — e.g. `5 + (0, 1).1` → `6`, and similar combinations
  exercising indexing followed by arithmetic/comparison ops, to confirm the
  stack is left in a state subsequent ops can correctly consume.
- **`cel-runtime`:** `RawStack::push_raw` behaves like `push::<T>` for
  equivalent size/align (alignment/padding correctness, round-trip via existing
  `pop`); tuple construction + `.N` indexing exercised through `DynSegment`'s
  public API, including drop-count assertions (extending the existing
  `DropCounter` pattern already used for `op1r`/`op2r` unwind tests) to verify
  every element is dropped exactly once whether or not it's the one extracted;
  `pop_tuple_as`/`push_tuple` round-trip tests against a concrete `CStackList`
  shape.
- **`cel-parser` op-table:** signature matching against registered tuple-shaped
  operands.

## Out of Scope

Deferred, per VISION.md's own separation of these as distinct future
directions: arrays/vecs, first-class functions, method-call syntax
(`.name(...)`), the compiled/macro backend, and the compaction optimization
noted above (reclaiming dead space below an extracted element instead of a full
pop-and-repush). The `AssociatedType` shape introduced here is meant to support
these later without re-deriving it.
