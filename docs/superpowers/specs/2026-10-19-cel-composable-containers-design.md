# Composable CEL Containers and Typed Rust Interoperability

**Date:** 2026-10-19
**Branch:** `worktree-simple-straighten`
**Status:** Approved by the user on 2026-10-19. Tasks 1–8 implemented and
reviewed; Task 9 executable documentation and final gate tracked in the
[handoff](../2026-10-19-composable-containers-handoff.md).
The original design below remains the historical approved contract; the handoff
records final API spellings, verification, and any completion limits.

**Builder check:** A source-backed review found no additional planning blockers.
The user approved the complete design, including the nested-array mapping,
canonical tuple-list representation, and callable scope.

## Intent

Make CEL tuples and arrays closed under composition over every value type CEL
can express. In particular, an array of tuples must not require boxing tuples
as `DynamicSequence`, and nested arrays must not depend on `DynamicArray`
values as their Rust-facing element type.

This work starts from crop fitting, but solves the library problem rather than
adding a geometry-specific exception.

## User requirements

- Arrays are homogeneous, and tuples retain each positional element's type.
- Both containers can hold any expressible CEL value type recursively.
- Container representations may be redesigned; compatibility layers are not
  a goal in this unreleased workspace.
- A CEL array of `T` supports allocation-preserving ownership conversion to
  `Vec<T>` and borrowed `&[T]` access. Borrowing an actual `&Vec<T>` is unnecessary.
- A CEL tuple supports a concrete heterogeneous type-list representation that
  converts to and from ordinary Rust tuples.
- Erasure happens at operation boundaries. The auxiliary type stack validates
  types and layouts while constructing the segment.
- Executing a validated segment does not repeat type checking.
- Future macro-compiled CEL uses the same Rust-facing operations as interpreted
  CEL, without user-code modifications.

## Approved representation choices

The assistant proposed the choices in this section, and the user approved
them with the complete design.

### Arrays

Introduce a concrete generic `Array<T>` with a controlled, element-independent
header layout. Its owned allocation contains actual contiguous `T` values,
with the size, alignment, and stride of `T`, not per-element wrappers.

The header contains a thin allocation pointer, length, and capacity. It does
not contain an element descriptor. A transparent generic wrapper around a
fixed-layout raw header can provide typed ownership and `PhantomData<T>`.
Keep raw construction crate-private and unsafe where it establishes ownership.

Public typed operations include:

- `Array<T>::from_vec(Vec<T>)`.
- `Array<T>::into_vec(self) -> Vec<T>`.
- `Array<T>::as_slice(&self) -> &[T]`.
- Length, emptiness, and capacity inspection.

These typed operations do not check types. `from_vec` and `into_vec` transfer
raw allocation parts using stable `Vec` APIs. They do not reinterpret the
unspecified layout of Rust's `Vec<T>` header.

The native and interpreted backends both use `Array<T>` as the canonical
representation of CEL `[T]`. Typed operation registration can adapt an
owned `Vec<T>` input or output through these same conversions. Borrowed slice
operations borrow from the array owner; they must not consume or reconstruct
the owner just to obtain the slice.

**Important nested-array choice:** the canonical representation of `[[i32]]`
is `Array<Array<i32>>`. Its allocation-preserving vector view is
`Vec<Array<i32>>`, and its slice view is `&[Array<i32>]`. Each inner array
independently converts to `Vec<i32>`. This does not promise allocation-preserving
conversion of the outer allocation to `Vec<Vec<i32>>`: those element headers
have different representations. Deep conversion to nested native vectors is
not part of this design. The user's approval covers this choice.

### Tuples

Reuse the existing `repr(C)` `CStackList<H, Tail>` and canonical empty
`CNil<()>` family as concrete tuple values, rather than the unspecified layout
of native Rust tuples or ordinary Rust `(Head, Tail)` pairs.

The logical head corresponds to CEL tuple element zero. The tail holds the
remaining elements in source order. `IntoCStackList` already follows that
logical convention, even though the struct physically stores the tail first.

Define the canonical mapping by logical source order, not by sequential
stack pushes:

| CEL type | Canonical Rust value type |
|---|---|
| Registered leaf `T` | Its registered concrete Rust type |
| `[T]` | `Array<R(T)>` |
| `()` | `CNil<()>` |
| `(T,)` | `CStackList<R(T), CNil<()>>` |
| `(T, U)` | `CStackList<R(T), CStackList<R(U), CNil<()>>>` |
| Callable with arguments `A` and result `R` | A signature-typed callable value |

Here `R(T)` means the recursively canonical Rust representation of CEL type
`T`, not an erased dynamic value.

For example, `[(i32, [f64])]` owns an array allocation of concrete list values
whose first logical field is `i32` and whose second is `Array<f64>`.

Add safe bidirectional conversions between the canonical list and native Rust
tuples. Moving conversions require neither `Clone`, `PartialEq`, nor `Debug`.
The recursive list implementation has no arity limit; convenience bridges to
native tuples cover the workspace's existing supported arities.

Change CEL tuple packing and indexing to this canonical layout. Compute the
layout recursively from the tail, matching the actual `repr(C)` structs,
including each cons cell's trailing padding. Do not retain the existing
flat declaration-order packing as a second tuple ABI.

Unit expressions and empty tuple types must describe the same CEL type.
Rust `()` interoperation uses the zero-element list bridge, not an independent
leaf identity that only happens to have size zero.

### Callable elements

"Any expressible value type" includes callable values and registered function
values with known signatures. Container composition must retain their complete
argument and result types, not just the `DynClosure` physical marker.

Provide a concrete signature-typed callable interface for Rust interoperation.
Its interpreter implementation can own a compiled body; a future native
implementation can own a compiled callable. Backend dispatch is not dynamic
CEL typing. Invocation through a validated signature does not compare runtime
argument types.

The exact Rust API spelling and backend carrier are implementation decisions,
but they must permit the same user operation signatures across both backends.
Do not require callables to implement equality in order to store them.

Use the existing generic type-expression grammar for an explicit callable
annotation, provisionally `Fn((A, B), R)` and `Fn((), R)`. Define and resolve
the signature structurally in CEL itself, not through a list of separately
registered composite instantiations. This allows typed empty callable arrays
and callable-containing tuple annotations.

## One complete type model

Replace the narrower array element shape with the same complete recursive type
information used for every value:

- Concrete registered leaf identity.
- Ordered tuple element types.
- Homogeneous array element type.
- Callable parameter and result types.
- Physical layout and the ownership operations required by that layout.
- Optional capabilities such as cloning, equality, and formatting.

Semantic identity is structural for aggregates and callables. A common erased
storage marker or missing concrete aggregate `TypeId` is not a substitute for
that identity. Synthesizing arbitrary Rust monomorphizations from runtime
`TypeId`s is neither required nor attempted.

Typed host registration derives its concrete recursive representation metadata
from a trait implemented for leaves, canonical lists, arrays, and typed
callables. CEL builds structural descriptors for arbitrary valid compositions;
matching those descriptors to an actual Rust operation installs monomorphized
adapters for the concrete types that operation names.

A descriptor must not eagerly instantiate an infinite chain of generic array
factory functions. Describing `Array<T>` recursively visits `T`, not
`Array<Array<T>>` and every possible enclosing composition.

Construction validates complete identity and physical representation before
emitting adapters. Thereafter erased internal storage is paired with its
already-validated ownership plan. Persistent host-erased values may retain
that plan for destruction; individual array elements do not carry type tags.

## Construction and execution

During segment construction:

1. Resolve/infer each complete recursive value type.
2. Require all array elements to have identical types, including callable
   signatures and nested aggregate shapes.
3. Validate explicit annotations recursively, including typed empty arrays.
4. Match operation operands/results to concrete registered representations.
5. Compute packing offsets, stride, allocation layout, and cleanup plans.
6. Emit allocation, move, borrow, invocation, and drop operations that assume
   those established invariants.

During execution:

- Evaluate operands once in source order.
- Allocate and initialize arrays according to the precomputed element layout.
- Pack tuples into their canonical list layout.
- Execute registered operations using validated adapters.
- Preserve existing fallible-operation errors and cleanup behavior.
- Perform bounds, allocation-overflow, and other value-dependent checks where
  required by an operation's contract.
- Do not compare `TypeId`s, recheck container homogeneity, downcast each element,
  or compare callable signatures along the validated execution path.

Public APIs that accept arbitrary erased external values may still validate at
the external entry boundary. Do not route trusted segment-internal operations
through those checked APIs on every execution. Provide a prepared typed
execution boundary whose validation happens once.

## Static diagnostics and integration

Wire this type model through runtime construction, the CEL AST checker,
type-expression resolution, closure annotation/invocation, operation
registration, and Adam's type registry, checker, and execution bridges.

The AST checker currently represents many unchecked results as `Ty::Any`.
An unresolved diagnostic state may remain while editing incomplete source,
but it is not a concrete CEL type and cannot certify a container or operation
as executable. Final segment construction must establish all required types.
Remove the tuple-array rejection rather than replacing it with an unknown type.

The following compositions must work without registration of each aggregate
shape:

- Arrays of tuples, including empty and singleton tuples.
- Tuples of arrays and tuples of tuples.
- Arrays of arrays of tuples and tuples containing arrays of tuples.
- Registered records and generic registered leaves at any nesting level.
- Callables inside either container, including callable results that are
  containers.
- Typed empty arrays of each of these element shapes.

Adam's cells, sources, requirements, relationships, and outputs must preserve
these shapes. Equality/clone/default/display operations may require explicitly
available capabilities; lacking a capability is an operation error, not a
prohibition on storing the value in a CEL container.

## Ownership and safety

The container construction contract is ownership and layout, not an implicit
requirement that every leaf implements `Clone`, `PartialEq`, or `Debug`.

Cover zero-sized values with destructors, over-aligned values, owned values,
padding bytes, nested empty arrays, and partial initialization.

Moving bytes transfers ownership exactly once. Padding remains uninitialized
storage; do not read it as initialized `u8` values. Failure after some operands
have evaluated drops each live value exactly once and releases each allocation.

Interpreter array destruction preserves its existing reverse-element cleanup
convention. Once ownership transfers to `Vec<T>`, Rust's vector destruction
rules apply, as with today's vector conversion. Tuple destruction follows the
canonical list's reverse logical-field order.

Safe public typed constructors cannot forge an unrelated descriptor. Raw
erased ownership APIs remain private or unsafe and document their invariants.
Never assume that matching total size and alignment proves matching field
layout or semantic identity.

## Verification and documentation

Derive tests from these contracts, using real Rust operations rather than only
examining metadata:

- Typed slice reads and vector ownership transfers preserve values, length,
  capacity, and allocated pointer identity.
- Arrays of tuple lists and nested containers round-trip through concrete Rust
  types and ordinary tuple conversions.
- A mismatched nested field, tuple arity, callable parameter, or callable result
  is rejected before executing any expression.
- Typed empty arrays retain their complete type.
- Registered leaves without clone/equality/debug support can be stored and moved.
- Source-order evaluation and error cleanup remain correct.
- Over-alignment, zero-sized destruction, and partial construction are sound.
- A prepared execution path does not call checked conversion/invocation APIs.
- Adam parser/typechecker/runtime agree on positive and negative compositions.
- Statically constructed Rust examples use the same public interop API proposed
  for macro-generated CEL. Implementing the macro backend itself is out of scope.

Update adjacent contracts and current CEL/Adam book guidance. Mark older
container designs as historical with a link to the approved replacement.
The crop-fitting test stays separate and unchanged by this redesign.

## Scope and approval

This is one cross-crate container/type-boundary redesign. Break implementation
into coherent plans only after approving the complete design.

Array iterators, `map`, `flat_map`, `fold`, capturing closures, mutable CEL
containers, J-style rank operations, and the Rust macro backend remain outside
this work.

The user approved the canonical nested-array mapping `Array<Array<T>>` rather
than `Vec<Vec<T>>`, the canonical existing list family, and including
signature-typed callable interoperation and annotations in this composition
guarantee. Implementation planning follows this approval.
