# CEL Homogeneous Arrays

**Date:** 2026-09-15
**Branch:** `worktree-cell-parser/arrays`

## Summary

Add Rust-style homogeneous array literals to `cel-parser`, backed by a new
`cel_runtime::DynamicArray` whose allocation can transfer to and from `Vec<T>` without moving
elements or reallocating. The element type remains known only at runtime: `DynamicArray` carries a
recursive element descriptor and validates typed conversions before reconstructing a `Vec<T>`.

The initial syntax supports non-empty literals such as `[0, 1, 2, 3]`, `[f(), g(), h()]`, and
recursively homogeneous nested literals such as `[[0], [1]]`. Each nesting level is a distinct
rank-one allocation. Flat multidimensional shape/rank metadata and J-style operations are future
work built on this representation.

Untyped empty literals are intentionally rejected. Contextually typed empty arrays are tracked by
[#212](https://github.com/stlab/cel-rs/issues/212). CEL tuple literals as array elements require
materialization from the runtime's internal `DynTuple` layout into concrete `DynamicSequence`
values and are tracked by [#213](https://github.com/stlab/cel-rs/issues/213).

## Goals

- Parse and format non-empty Rust-style array literals without trailing commas.
- Build the same array AST through `AstContext` and executable array values through
  `DynSegmentContext`.
- Require every element to have the same complete runtime type shape.
- Preserve the allocation in both `Vec<T> -> DynamicArray` and
  `DynamicArray -> Vec<T>` ownership transfers.
- Support any concrete `'static` host type without requiring `Clone`, `PartialEq`, or `Debug`.
- Support recursively homogeneous nested arrays as `Vec<DynamicArray>`.
- Isolate unsafe allocation, move, destruction, and conversion logic behind documented runtime
  APIs.
- Leave a clean base for future array operations and multidimensional shape semantics.

## Non-goals

- Empty array literal inference; see #212.
- CEL tuple literals as array elements; see #213.
- Repeat expressions such as `[value; count]`.
- Array indexing, slicing, comprehensions, or mutation syntax.
- J-style verbs, rank polymorphism, broadcasting, or flat multidimensional storage.
- Direct `DynSegment::call0::<Vec<T>>()`. Evaluation returns `DynamicArray`; callers explicitly
  request a checked conversion.
- A stable memory-layout promise for the `Vec<T>` struct header itself.
- Array support in the future native Rust macro backend.

## Terminology and Core Guarantee

`DynamicArray` is **allocation-compatible** with `Vec<T>`, not layout-compatible with the
`Vec<T>` struct. Rust does not guarantee `Vec<T>`'s field order or struct representation.

Allocation compatibility means:

1. `DynamicArray::try_from_vec` takes ownership of a valid homogeneous `Vec<T>`'s allocation,
   pointer, length, and capacity without moving any element.
2. A successful `DynamicArray::try_into_vec::<T>` returns ownership of those same raw parts
   without moving any element or reallocating.
3. An array literal allocates storage through Rust's global allocator with the exact element
   layout and capacity needed for a later sound `Vec<T>::from_raw_parts`.
4. Every deallocation uses the allocator and layout corresponding to the stored capacity and
   element descriptor.

The pointer value may follow normal `Vec` rules for zero-sized types and zero capacity; pointer
identity is only meaningful for allocated, non-zero-sized storage.

## Syntax and AST

Extend the grammar with:

```text
primary_expression = literal
                   | identifier
                   | tuple_or_group
                   | array_expression
                   | if_expression
                   | closure_expression.

array_expression = "[" expression { "," expression } "]".
```

Examples:

```text
[0]
[0, 1, 2, 3]
[f(), g(), h()]
[[0], [1]]
```

`[]` is recognized as an array literal and rejected with a targeted diagnostic that explains that
an element type cannot yet be inferred and references the future context-typing work represented
by #212. This is preferable to treating `[]` as an unrelated syntax error.

A comma always requires another expression. Trailing commas are intentionally unsupported so the
production remains Wirth-style LL(1), consistent with the parser's existing tuple and parameter
list grammar.

Add:

```rust
Expr::Array {
    elements: Vec<Expr>,
    span: ExprSpan,
}
```

`AstContext` records the elements in source order. The CEL formatter emits bracket syntax and
preserves its established trivia/comment behavior.

## Runtime Representation

Add a `dynamic_array` module exported by `cel-runtime`. Its principal type is conceptually:

```rust
pub struct DynamicArray {
    ptr: NonNull<u8>,
    len: usize,
    capacity: usize,
    element: ArrayElementType,
}
```

The exact private field names may change during implementation. Public behavior, rather than this
field spelling, is the contract.

### Element descriptor

One descriptor represents every element; metadata is O(1) with respect to array length. It carries
the information required to validate type identity and manage ownership:

```rust
pub struct ArrayElementType {
    type_id: TypeId,
    type_name: Cow<'static, str>,
    size: usize,
    align: usize,
    drop: unsafe fn(*mut u8),
    nested_array_element: Option<Box<ArrayElementType>>,
}
```

The recursive field is present only when `type_id == TypeId::of::<DynamicArray>()`. It makes the
semantic element type distinguish `[i32]` from `[f64]` even though both inner values have the same
concrete wrapper `TypeId`. Equality of array element types compares both the concrete `TypeId` and,
for arrays, the complete nested descriptor.

An empty `Vec<DynamicArray>` has no value from which to recover its inner element descriptor.
Constructing that case therefore requires an explicit inner descriptor. `ArrayElementType` is an
opaque, cloneable public value with safe constructors for a leaf `T` and for `array_of(inner)`;
callers cannot provide raw size, alignment, or drop glue. Raw descriptor construction remains
crate-private or unsafe.

The initial descriptor deliberately carries no mandatory clone, equality, or element-debug
function. This keeps construction available for every `'static` Rust type. Capabilities required
by later array operations may be represented by optional function pointers or operation registry
entries without changing the allocation model.

### Ownership and conversion API

The initial public API provides:

- `DynamicArray::try_from_vec<T: 'static>(Vec<T>) -> Result<DynamicArray, ArrayBuildError<T>>`.
- `DynamicArray::try_from_vec_with_element_type<T: 'static>(Vec<T>, ArrayElementType)`, primarily
  for empty nested arrays; it checks that the descriptor's concrete outer type is `T`.
- `DynamicArray::try_into_vec<T: 'static>(self) -> Result<Vec<T>, ArrayTypeError>`.
- `DynamicArray::try_as_slice<T: 'static>(&self) -> Result<&[T], ArrayTypeError>`.
- Mutable slice access for leaf element types only.
- Read-only length, capacity, emptiness, and element-type inspection.

`try_from_vec` validates recursive homogeneity when `T` is `DynamicArray`, then takes raw ownership
using stable `Vec`/`ManuallyDrop` primitives. On validation failure, its error preserves the input
vector. For every other concrete `T`, Rust's type system already establishes homogeneous storage.

A successful `try_into_vec` checks `T`'s concrete `TypeId`, suppresses `DynamicArray`'s destructor,
and reconstructs the vector with `Vec::from_raw_parts`. When `T` is `DynamicArray`, the generic Rust
type cannot encode the nested descriptor; the recursive invariant was established at construction
and remains available on the contained values. A failed conversion does not cast the allocation
and returns an explicit mismatch error naming expected and actual element types. The implementation
should preserve the value on mismatch if that can be expressed without making the API cumbersome;
otherwise the documented consuming operation may drop it on error, matching
`DynamicSequence::try_into_tuple`.

Borrowed slice access validates the concrete `TypeId` before calling `slice::from_raw_parts`.
Mutable slice access additionally rejects nested-array elements: an unrestricted
`&mut [DynamicArray]` could replace an inner array with one of a different recursive type and
invalidate the outer descriptor. Future checked element-replacement APIs may support nested
mutation while preserving that invariant.

`DynamicArray` implements `Drop`, but initially does not implement `Clone` or `PartialEq`.
`Debug` reports safe structural metadata such as the element type and length without reading
elements that are not known to implement `Debug`.

### Allocation and destruction

For non-zero-sized elements, an array literal allocation uses the global allocator and a checked
layout derived from element size, alignment, and capacity. Initial capacity equals literal length,
so the allocation is valid for later reconstruction as `Vec<T>` with the same capacity.

`Drop` calls element drop glue once per live element, then deallocates using the stored capacity
and element layout. Elements are dropped in reverse order, matching the runtime's existing
aggregate cleanup convention.

Zero-sized types use no allocation. The stored pointer must be non-null and aligned for the element
descriptor, and the stored capacity must obey the convention required to reconstruct a valid
`Vec<T>`. Destruction still invokes drop glue exactly `len` times because zero size does not imply
trivial destruction.

All constructors check multiplication and layout limits before allocation. Ordinary allocation
failure follows Rust's global allocation-error behavior; arithmetic/layout overflow is returned as
an explicit construction or parse error.

## Dynamic Stack Type Metadata

An evaluated array occupies one physical `DynamicArray` stack slot. The slot's concrete `TypeId`
is therefore `TypeId::of::<DynamicArray>()`, but that flat ID is insufficient for semantic type
checking.

Extend runtime stack metadata so an array slot also records its recursive element descriptor.
This may be implemented by generalizing the existing `AssociatedType` mechanism or by introducing
a small aggregate-kind enum. The implementation should preserve these invariants:

- Tuple shape remains ordered and recursive.
- Array shape contains exactly one repeated element shape.
- Full shape comparison distinguishes arrays with different nested element types.
- Existing scalar operation lookup can continue using flat `TypeId` values.
- Array-aware operations can inspect the complete semantic shape.
- Physical drop/layout metadata remains separate from semantic child shape where conflating them
  would obscure safety invariants.

A clean aggregate-kind representation is preferred over conventions such as interpreting an
`associated` vector of length one as an array without an explicit discriminator.

## Literal Compilation

Add a fallible `ParserContext::make_array` operation. `AstContext` creates `Expr::Array`;
`DynSegmentContext` delegates to `DynSegment::make_array`.

At parse/segment-build time, `DynSegment::make_array`:

1. Requires at least one element.
2. Reads the `StackInfo` for every element expression.
3. Compares their complete semantic shapes.
4. Rejects CEL `DynTuple` pseudo-values, which have no concrete Rust element representation in
   this version.
5. Captures each source offset, size, alignment, padding state, and drop function.
6. Replaces the element stack entries with one `DynamicArray` entry carrying the recursive element
   descriptor.
7. Queues one raw collector operation.

At evaluation time, the collector:

1. Allocates one destination buffer for the final vector capacity.
2. Moves each source value, in source order, into `ptr.add(index * element_size)`.
3. Removes the source bytes and padding from `RawStack` without invoking the moved values'
   destructors.
4. Pushes the completed `DynamicArray` as one ordinary Rust value.

The collector captures all source positions before metadata is collapsed. Its move algorithm must
account for stack padding and possible overlap while compacting the source stack. It should reuse
or generalize existing `RawStack` raw movement primitives rather than duplicate pointer arithmetic.

### Panic and partial-initialization safety

A private construction guard owns the destination allocation while elements are transferred. It
tracks the initialized prefix and, unless disarmed, drops exactly that prefix and deallocates the
buffer. The initial raw byte moves are not expected to panic, but encoding this invariant makes the
constructor robust if later element conversion or capability hooks become fallible.

If evaluating any element expression fails before the collector runs, existing `DynSegment`
unwind machinery drops already-produced stack values. Once collection begins, ownership of each
element changes exactly once from stack storage to array storage.

## Nested Arrays

Nested arrays are recursive rank-one arrays, not one multidimensional allocation:

```text
[[0], [1]]
```

evaluates to an outer `DynamicArray` whose concrete Rust element type is `DynamicArray`. The outer
allocation is therefore directly convertible, without moving elements, to
`Vec<DynamicArray>`. Each inner value can independently convert without moving elements to
`Vec<i32>`.

`try_from_vec::<DynamicArray>` accepts a non-empty vector only when every inner value has the same
complete recursive descriptor. An empty vector of dynamic arrays requires its intended inner
descriptor explicitly. Mutable slice access is unavailable for nested arrays because replacing an
inner value through `&mut [DynamicArray]` could violate the outer homogeneous-type invariant.

Homogeneity includes recursive descriptors:

- `[[0], [1]]` is valid.
- `[[0], [1.0]]` is invalid because the inner element types differ.
- `[[0], [[1]]]` is invalid because the recursive shapes differ.

This representation intentionally does not promise zero-copy conversion directly to
`Vec<Vec<i32>>`: the inner values are `DynamicArray` wrappers, not `Vec<i32>` headers. Adding a
flat shape vector and rank-polymorphic operations later can coexist with, or deliberately replace,
this nested representation after J-style semantics are designed.

## Static Type Checking

Extend `cel_parser::Ty` with an array case:

```rust
Array(Box<Ty>)
```

This requires changing `Ty` from `Copy` to `Clone` and updating callers accordingly. Array
inference folds over element types:

- Equal concrete types infer `Array(concrete)`.
- A known mismatch emits a diagnostic at the first conflicting element.
- `Ty::Any` continues to unify conservatively with concrete types.
- Nested `Array` values unify recursively.

The current AST checker deliberately infers function application results as `Ty::Any`. Therefore
it can diagnose `[1, 2.0]`, but it cannot prove the element type of `[f(), g(), h()]`. That
expression is represented as `Array(Any)` in the AST checker, while direct `DynSegment`
compilation remains authoritative and requires the compiled calls to leave matching runtime
shapes. Resolving arbitrary call return types is broader type-system work and is not introduced
only for arrays.

## Diagnostics

New diagnostics include:

- Empty literal: array element type cannot yet be inferred; see #212.
- Heterogeneous literal: expected the first element's recursive type, found the conflicting
  element's recursive type.
- Unsupported tuple element: CEL tuple literals cannot yet be array elements; see #213.
- Allocation layout overflow: requested array storage cannot be represented safely.
- Typed conversion/access mismatch: expected element type `T`, found the stored type.

Diagnostics should identify the conflicting element span rather than only the whole literal where
the parser has that span available. Runtime errors use stored human-readable type names.

## `cel-rs` Facade

Export `DynamicArray` and its public error/inspection types from `cel-runtime`. The root `cel-rs`
facade already re-exports the runtime; update explicit exports or documentation only if required by
the actual facade structure. No separate container implementation belongs in the facade crate.

## Testing Strategy

Tests derive from the public grammar and API contracts.

### `cel-runtime`

- `Vec<T> -> DynamicArray -> Vec<T>` preserves pointer, length, capacity, order, and values for an
  allocated non-zero-sized leaf `T`.
- `try_into_vec`, immutable slice access, and mutable slice access reject the wrong `T`.
- Mutable slice changes are visible after conversion back to `Vec<T>`.
- Non-`Copy`, non-`Clone`, non-`PartialEq`, and non-`Debug` element types are accepted.
- Drop-counted elements are destroyed exactly once after every ownership path.
- Over-aligned elements preserve alignment through construction and conversion.
- Zero-sized values, including zero-sized values with `Drop`, preserve length and exact drop count.
- Empty `Vec<T>` values for leaf `T` round-trip through the Rust API even though `[]` syntax is
  rejected.
- Empty `Vec<DynamicArray>` construction requires an explicit inner descriptor.
- Heterogeneous `Vec<DynamicArray>` construction returns the original vector in its error.
- Nested arrays convert to `Vec<DynamicArray>`, then each inner array converts to its typed vector.
- Nested arrays reject mutable slice access so their recursive descriptor cannot become stale.
- Partial construction cleanup drops only initialized elements.

### `cel-parser`

- `[0]` and `[0, 1, 2, 3]` parse and evaluate.
- `[0, 1,]` reports that an expression must follow the final comma.
- `[f(), g(), h()]` succeeds for equal registered return types and fails at the first mismatch.
- `[1, 2.0]` reports a heterogeneous-element diagnostic.
- `[]` reports the dedicated unsupported-empty-array diagnostic.
- `[[0], [1]]` evaluates successfully.
- `[[0], [1.0]]` and mismatched nesting depths fail with recursive type diagnostics.
- Tuple-valued elements report the dedicated #213 diagnostic.
- AST shape and spans are correct for scalar and nested arrays.
- Formatting emits bracket syntax and remains stable around comments.
- Array expressions work in every value-producing grammar position already routed through
  `expression`, including call arguments, tuple elements, condition branches, and closure bodies.

### Workspace integration

- Public `cel-runtime` APIs are visible through the `cel-rs` facade.
- Existing tuple behavior and `DynamicSequence` conversions remain unchanged.
- Existing parser contexts continue compiling after the new fallible `make_array` hook is added.

## Future J-style Processing

This design intentionally solves storage and type identity before defining array algebra. Future
work can register operations over `DynamicArray`, dispatching on recursive element descriptors and
optional capabilities. Likely follow-ups include:

- A flat allocation plus shape vector for rank greater than one.
- Scalar extension and broadcasting rules.
- Element-wise arithmetic and comparison dispatch.
- Reductions, scans, reshaping, transposition, and views.
- Copy-on-write or shared storage if array transformations make ownership copying significant.

Those choices should be designed together because they determine whether nested rank-one arrays
remain a language-visible representation or become only a construction syntax normalized into a
flat shaped array.

## Acceptance Criteria

1. Shared CEL grammar, AST, formatter, and direct `DynSegment` compilation support non-empty array
   literals while preserving Wirth-style LL(1) parsing; trailing commas are rejected.
2. Every array literal is homogeneous by complete recursive runtime type shape.
3. `DynamicArray` supports zero-copy ownership transfer to and from `Vec<T>` for concrete
   `'static` element types.
4. Array literals of arbitrary concrete host types require no traits beyond `'static`.
5. Nested literals such as `[[0], [1]]` work as recursively typed rank-one arrays.
6. Empty and CEL-tuple-valued literals fail with targeted diagnostics referencing #212 and #213.
7. All success and failure paths preserve alignment and drop every initialized value exactly once.
8. Existing scalar and tuple parsing/evaluation behavior remains unchanged.
