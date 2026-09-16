# CEL Homogeneous Arrays Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add non-empty, homogeneous CEL array literals backed by a type-erased allocation that transfers to and from `Vec<T>` without moving elements.

**Architecture:** `cel-runtime` gains `DynamicArray`, which owns Vec-compatible raw parts and one recursive element descriptor. `DynSegment` collects equal-shaped stack values into that allocation, while the shared parser emits either an executable collector or `Expr::Array`; the static checker represents arrays recursively as `Ty::Array`.

**Tech Stack:** Rust 2024, `std::alloc`, `std::any::TypeId`, `proc_macro2`, the existing `RawStack`/`DynSegment` runtime, and the existing recursive-descent parser.

**Spec:** `docs/superpowers/specs/2026-09-15-cel-homogeneous-arrays-design.md`

## Global Constraints

- Preserve zero-copy ownership transfer in both directions between `Vec<T>` and `DynamicArray`.
- Require only `T: 'static`; do not add `Clone`, `PartialEq`, or `Debug` bounds to array elements.
- Keep array literals homogeneous by complete recursive runtime type shape.
- Support nested rank-one arrays as `Vec<DynamicArray>`; do not add multidimensional shape storage.
- Reject `[]` with a targeted diagnostic; contextually typed empty literals are deferred to #212.
- Reject CEL tuple literals as array elements; their materialization is deferred to #213.
- Reject trailing commas so the grammar remains Wirth-style LL(1).
- Do not add repeat syntax, indexing, comprehensions, mutation syntax, or J-style operations.
- Keep all unsafe raw-parts construction crate-private and document every safety invariant.
- Add contract-style documentation and public examples for every new public API.
- Use checked size arithmetic and preserve exact once-only destruction on all paths.
- Add no dependencies.

---

## File Structure

- Create `cel-runtime/src/dynamic_array.rs`: erased Vec ownership, recursive element descriptors,
  conversion/access APIs, allocation, and drop logic.
- Modify `cel-runtime/src/lib.rs`: publish and re-export the dynamic-array module.
- Modify `cel-runtime/src/dyn_segment.rs`: represent recursive value shapes and add the array
  collector.
- Modify `cel-parser/src/parser_context.rs`: add the shared `make_array` emission hook.
- Modify `cel-parser/src/ast.rs`: add `Expr::Array` and AST-context construction.
- Modify `cel-parser/src/lib.rs`: add the LL(1) bracket grammar and parser diagnostics.
- Modify `cel-parser/src/fmt.rs`: format array AST nodes with bracket syntax.
- Modify `cel-parser/src/ty.rs`: add recursive array types and homogeneous-element checking.
- Modify `adam-lang/src/typecheck.rs`: replace assumptions that `Ty` is `Copy` with borrowing or
  cloning.
- Modify `src/lib.rs` only if rustdoc needs an explicit facade example; the existing
  `cel_rs::runtime::*` glob already exports new `cel-runtime` items.

---

### Task 1: Implement the `DynamicArray` Ownership Type

**Files:**
- Create: `cel-runtime/src/dynamic_array.rs`
- Modify: `cel-runtime/src/lib.rs`

**Interfaces:**
- Produces: `ArrayElementType`, `ArrayTypeError`, `ArrayBuildError<T>`, and `DynamicArray`.
- Produces:
  `DynamicArray::try_from_vec<T: 'static>(Vec<T>) -> Result<Self, ArrayBuildError<T>>`.
- Produces:
  `DynamicArray::try_from_vec_with_element_type<T: 'static>(Vec<T>, ArrayElementType) -> Result<Self, ArrayBuildError<T>>`.
- Produces:
  `DynamicArray::try_into_vec<T: 'static>(self) -> Result<Vec<T>, ArrayTypeError>`.
- Produces:
  `DynamicArray::{try_as_slice, try_as_mut_slice, len, capacity, is_empty, element_type}`.
- Produces crate-private raw construction used by Task 4.

- [ ] **Step 1: Add failing tests for zero-copy leaf-vector conversion**

Create the module with test-only expectations first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct NoTraits(u32);

    #[test]
    fn vec_round_trip_preserves_allocation_and_capacity() {
        let mut values = Vec::with_capacity(8);
        values.extend([NoTraits(1), NoTraits(2)]);
        let ptr = values.as_ptr();
        let capacity = values.capacity();

        let array = DynamicArray::try_from_vec(values).unwrap();
        assert_eq!(array.len(), 2);
        assert_eq!(array.capacity(), capacity);

        let values = array.try_into_vec::<NoTraits>().unwrap();
        assert_eq!(values.as_ptr(), ptr);
        assert_eq!(values.capacity(), capacity);
        assert_eq!(values[0].0, 1);
        assert_eq!(values[1].0, 2);
    }

    #[test]
    fn typed_slice_access_checks_the_element_type() {
        let array = DynamicArray::try_from_vec(vec![1i32, 2]).unwrap();
        assert_eq!(array.try_as_slice::<i32>().unwrap(), &[1, 2]);
        assert!(array.try_as_slice::<u32>().is_err());
    }
}
```

- [ ] **Step 2: Run the tests to verify the module/API is missing**

Run:

```bash
cargo test -p cel-runtime --lib dynamic_array::tests::vec_round_trip_preserves_allocation_and_capacity
```

Expected: compilation fails because `dynamic_array`/`DynamicArray` is not implemented.

- [ ] **Step 3: Define the descriptor and error contracts**

Implement opaque metadata and errors with no element trait bounds:

```rust
#[derive(Clone)]
pub struct ArrayElementType {
    type_id: TypeId,
    type_name: Cow<'static, str>,
    size: usize,
    align: usize,
    drop: RawDropper,
    nested: Option<Box<ArrayElementType>>,
}

pub struct ArrayBuildError<T> {
    kind: ArrayBuildErrorKind,
    values: Vec<T>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArrayBuildErrorKind {
    MissingNestedElementType,
    HeterogeneousNestedElement {
        index: usize,
        expected: Cow<'static, str>,
        found: Cow<'static, str>,
    },
    DescriptorTypeMismatch {
        expected: Cow<'static, str>,
        found: Cow<'static, str>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArrayTypeError {
    expected: Cow<'static, str>,
    found: Cow<'static, str>,
}
```

Give `ArrayBuildError<T>` a manual `Debug` implementation that does not require `T: Debug`, a
`Display` implementation that reports `kind`, and `into_vec(self) -> Vec<T>`. Reuse
`dyn_segment::RawDropper` so Task 4 can transfer a stack entry's existing drop function without
recovering a monomorphized `T` from `TypeId`; leaf and `DynamicArray` elements pass an empty
associated-type slice to it. Give
`ArrayElementType` safe `leaf<T: 'static>()` and `array_of(ArrayElementType)` constructors;
`leaf::<DynamicArray>()` must return an error because a nested descriptor is required.

- [ ] **Step 4: Implement raw ownership and checked leaf access**

Implement:

```rust
pub struct DynamicArray {
    ptr: NonNull<u8>,
    len: usize,
    capacity: usize,
    element: ArrayElementType,
}
```

Use `ManuallyDrop<Vec<T>>` to capture raw parts. On successful extraction, wrap `self` in
`ManuallyDrop`, cast the pointer only after `TypeId::of::<T>() == self.element.type_id`, and call
`Vec::from_raw_parts`. Implement immutable slices for every matching concrete `T`; implement
mutable slices only when `element.nested.is_none()`. Implement `Drop` by dropping indices in reverse
and deallocating the exact capacity layout. For ZSTs, use a non-null pointer aligned to
`element.align`, skip allocation/deallocation, but run drop glue `len` times.

- [ ] **Step 5: Add drop, alignment, ZST, mismatch, and mutable-leaf tests**

Add tests that:

```rust
#[test]
fn mutable_leaf_slice_updates_the_owned_values() {
    let mut array = DynamicArray::try_from_vec(vec![1i32, 2]).unwrap();
    array.try_as_mut_slice::<i32>().unwrap()[1] = 9;
    assert_eq!(array.try_into_vec::<i32>().unwrap(), vec![1, 9]);
}
```

Also use an `#[repr(align(64))]` element to assert pointer alignment; an
`Arc<AtomicUsize>` drop counter to assert exact once-only destruction; a ZST with `Drop` to assert
`len` destructor calls; and a mismatched `try_into_vec::<u32>` to assert no invalid cast occurs.
Test an empty `Vec<i32>` round-trip.

- [ ] **Step 6: Run all `DynamicArray` tests**

Run:

```bash
cargo test -p cel-runtime --lib dynamic_array::tests
```

Expected: all dynamic-array tests pass.

- [ ] **Step 7: Export and document the module**

In `cel-runtime/src/lib.rs`, add:

```rust
/// Owned, homogeneous, type-erased arrays with zero-copy `Vec<T>` conversion.
pub mod dynamic_array;
pub use dynamic_array::*;
```

Add module-level examples for conversion, slice access, and type mismatch.

- [ ] **Step 8: Commit**

```bash
git add cel-runtime/src/dynamic_array.rs cel-runtime/src/lib.rs
git commit -m "feat(cel-runtime): add zero-copy dynamic arrays"
```

---

### Task 2: Support Recursive Nested-Array Descriptors

**Files:**
- Modify: `cel-runtime/src/dynamic_array.rs`

**Interfaces:**
- Consumes: Task 1's `ArrayElementType` and `DynamicArray`.
- Produces: recursively checked `Vec<DynamicArray>` construction.
- Produces: explicit-descriptor construction for empty nested vectors.

- [ ] **Step 1: Write failing nested-array tests**

```rust
#[test]
fn nested_vec_round_trip_preserves_outer_and_inner_allocations() {
    let left = DynamicArray::try_from_vec(vec![0i32]).unwrap();
    let right = DynamicArray::try_from_vec(vec![1i32]).unwrap();
    let outer_values = vec![left, right];
    let outer_ptr = outer_values.as_ptr();

    let outer = DynamicArray::try_from_vec(outer_values).unwrap();
    let inner = outer.element_type().nested().unwrap();
    assert_eq!(inner.type_id(), TypeId::of::<i32>());

    let values = outer.try_into_vec::<DynamicArray>().unwrap();
    assert_eq!(values.as_ptr(), outer_ptr);
    assert_eq!(values[0].try_as_slice::<i32>().unwrap(), &[0]);
    assert_eq!(values[1].try_as_slice::<i32>().unwrap(), &[1]);
}

#[test]
fn heterogeneous_nested_vec_is_returned_on_error() {
    let values = vec![
        DynamicArray::try_from_vec(vec![0i32]).unwrap(),
        DynamicArray::try_from_vec(vec![1f64]).unwrap(),
    ];
    let error = DynamicArray::try_from_vec(values).unwrap_err();
    assert!(matches!(
        error.kind(),
        ArrayBuildErrorKind::HeterogeneousNestedElement {
            index: 1,
            expected,
            found,
        } if expected == "i32" && found == "f64"
    ));
    assert_eq!(error.into_vec().len(), 2);
}
```

- [ ] **Step 2: Run the nested tests to verify failure**

Run:

```bash
cargo test -p cel-runtime --lib dynamic_array::tests
```

Expected: tests fail because nested descriptor inference/validation is absent.

- [ ] **Step 3: Implement recursive construction validation**

When `T` is `DynamicArray`, use `&value as &dyn Any` and checked `downcast_ref` after the exact
`TypeId` comparison. Derive the outer descriptor from the first inner value and compare every
remaining inner `element_type()`. Return `MissingNestedElementType` for an empty
`Vec<DynamicArray>`, preserving the vector in the error.

Implement `try_from_vec_with_element_type` to validate the supplied descriptor's concrete
`TypeId` against `T`; for nested arrays, validate every contained descriptor against the supplied
nested descriptor. This is the path for empty `Vec<DynamicArray>`.

- [ ] **Step 4: Test explicit empty nesting and mutation protection**

```rust
#[test]
fn empty_nested_vec_uses_an_explicit_inner_type() {
    let element = ArrayElementType::array_of(ArrayElementType::leaf::<i32>().unwrap());
    let array =
        DynamicArray::try_from_vec_with_element_type(Vec::<DynamicArray>::new(), element).unwrap();
    assert!(array.try_into_vec::<DynamicArray>().unwrap().is_empty());
}

#[test]
fn nested_array_does_not_expose_an_unchecked_mutable_slice() {
    let inner = DynamicArray::try_from_vec(vec![1i32]).unwrap();
    let mut outer = DynamicArray::try_from_vec(vec![inner]).unwrap();
    assert!(outer.try_as_mut_slice::<DynamicArray>().is_err());
}
```

- [ ] **Step 5: Run the complete module tests**

Run:

```bash
cargo test -p cel-runtime --lib dynamic_array::tests
```

Expected: all dynamic-array tests pass, including nested and leaf cases.

- [ ] **Step 6: Commit**

```bash
git add cel-runtime/src/dynamic_array.rs
git commit -m "feat(cel-runtime): validate nested dynamic arrays"
```

---

### Task 3: Generalize Runtime Type Shapes

**Files:**
- Modify: `cel-runtime/src/dyn_segment.rs`
- Modify: `cel-parser/src/op_table.rs`
- Modify: `adam-lang/src/type_registry.rs`
- Modify: `adam-lang/src/parser.rs`

**Interfaces:**
- Consumes: Task 1's `ArrayElementType`.
- Produces: explicit leaf/tuple/array semantic shape metadata on `StackInfo` and
  `AssociatedType`.
- Preserves: existing tuple construction, indexing, `DynamicSequence` extraction, closure tuple
  arguments, and adam-lang tuple-cell behavior.

- [ ] **Step 1: Add failing recursive-shape tests**

Add tests in `dyn_segment.rs` proving:

```rust
#[test]
fn value_shapes_distinguish_nested_array_element_types() {
    let i32_array = ValueType::array(ArrayElementType::leaf::<i32>().unwrap());
    let f64_array = ValueType::array(ArrayElementType::leaf::<f64>().unwrap());
    assert!(!i32_array.same_shape(&f64_array));
}
```

Add a regression test that two existing nested tuple shapes still compare equal and that changing
one nested leaf makes them unequal.

- [ ] **Step 2: Run focused tests to verify the new shape API is missing**

Run:

```bash
cargo test -p cel-runtime --lib dyn_segment::tests::value_shapes_distinguish_nested_array_element_types
```

Expected: compilation fails because `ValueType` and its array constructor do not exist.

- [ ] **Step 3: Introduce an explicit aggregate-kind model**

Replace implicit "`DynTuple` plus non-empty `associated`" interpretation with:

```rust
#[derive(Clone, Debug)]
pub enum ValueKind {
    Leaf,
    Tuple(Vec<AssociatedType>),
    Array(ArrayElementType),
}

#[derive(Clone, Debug)]
pub struct ValueType {
    type_id: TypeId,
    type_name: Cow<'static, str>,
    size: usize,
    align: usize,
    raw_dropper: RawDropper,
    kind: ValueKind,
}

#[derive(Clone, Debug)]
pub struct AssociatedType {
    pub offset: usize,
    pub value_type: ValueType,
}

pub struct StackInfo {
    pub(crate) padding: bool,
    pub value_type: ValueType,
}
```

Implement `ValueType::same_shape` recursively. A leaf compares `TypeId`; a tuple compares ordered
children recursively; an array compares its recursive `ArrayElementType`. Keep physical
size/alignment/drop metadata available without deriving semantic identity from byte layout.

`ValueType`'s fields are private: values are built only through the validating constructors
`ValueType::leaf`, `leaf_from_parts`, `tuple`, and `array`, and read through the
`type_id()`/`type_name()`/`size()`/`align()`/`kind()` accessors, so layout, dropper, and kind can
never disagree.

- [ ] **Step 4: Migrate tuple runtime code without behavior changes**

Update `ToTypeIdList`, tuple layout/drop helpers, `make_tuple`, `tuple_index`, `join2`,
`peek_output_type_id`, `peek_stack_infos`, dynamic-sequence conversion, and tuple argument
expansion to use `value_type.kind`. Update `cel-parser/src/op_table.rs`'s scalar and tuple signature
matching, and adam-lang's construction and traversal of `AssociatedType`, to use the new fields.
Do not change tuple layout or ownership algorithms.

- [ ] **Step 5: Run tuple and downstream regression suites**

Run:

```bash
cargo test -p cel-runtime --lib
cargo test -p cel-parser --lib
cargo test -p adam-lang --lib tuple
```

Expected: all existing tuple/runtime tests pass unchanged in observable behavior.

- [ ] **Step 6: Commit**

```bash
git add cel-runtime/src/dyn_segment.rs cel-parser/src/op_table.rs adam-lang/src/type_registry.rs adam-lang/src/parser.rs
git commit -m "refactor(cel-runtime): model aggregate value shapes explicitly"
```

---

### Task 4: Add the `DynSegment` Array Collector

**Files:**
- Modify: `cel-runtime/src/dynamic_array.rs`
- Modify: `cel-runtime/src/dyn_segment.rs`
- Modify: `cel-runtime/src/raw_stack.rs` only if an existing raw move/truncate primitive cannot
  express ownership transfer without duplicating pointer arithmetic.

**Interfaces:**
- Consumes: `DynamicArray`, `ArrayElementType`, `ValueType`, and `ValueKind`.
- Produces:
  `DynSegment::make_array(&mut self, n: usize, ambient_start: usize) -> anyhow::Result<()>`.
- Produces a crate-private `DynamicArray` constructor that takes one fully initialized erased
  allocation and a validated element descriptor.

- [ ] **Step 1: Write failing direct-runtime tests**

```rust
#[test]
fn make_array_collects_homogeneous_values() -> anyhow::Result<()> {
    let mut segment = DynSegment::new::<()>();
    let start = segment.current_stack_offset();
    segment.just(0i32);
    segment.just(1i32);
    segment.just(2i32);
    segment.make_array(3, start)?;

    let array: DynamicArray = segment.call0()?;
    assert_eq!(array.try_into_vec::<i32>()?, vec![0, 1, 2]);
    Ok(())
}

#[test]
fn make_array_rejects_mismatched_values_before_execution() {
    let mut segment = DynSegment::new::<()>();
    let start = segment.current_stack_offset();
    segment.just(0i32);
    segment.just(1f64);
    assert!(segment.make_array(2, start).is_err());
}
```

Also add direct tests for a non-`Clone` result produced by `op0`, over-aligned values, a
drop-counted value, a ZST with drop glue, nested arrays, recursively mismatched nested arrays, and
rejection of a `DynTuple` element.

- [ ] **Step 2: Run the focused tests to verify failure**

Run:

```bash
cargo test -p cel-runtime --lib dyn_segment::tests::make_array
```

Expected: compilation fails because `DynSegment::make_array` is missing.

- [ ] **Step 3: Implement parse-time collector validation**

In `make_array`, reject `n == 0`, ensure the final `n` stack entries exist, compare every
`ValueType` with the first via `same_shape`, and reject `ValueKind::Tuple`. Derive one
`ArrayElementType`, capture absolute source offsets and padding, and only then drain the element
entries and push a `StackInfo` for concrete `DynamicArray` with `ValueKind::Array`.

Return errors that name the first conflicting index and expected/found recursive types.

- [ ] **Step 4: Implement the runtime move**

Queue one `raw0_` operation that allocates an exact-capacity destination through a crate-private
`DynamicArray` builder. Copy each element's bytes into fixed-stride destination storage, transfer
ownership by truncating source values in reverse order without calling their droppers, finish the
array guard, and push the resulting `DynamicArray` onto `RawStack`.

For ZSTs, transfer logical ownership counts without copying bytes. The guard must drop only its
initialized prefix if construction exits early. Use checked multiplication before creating the
allocation layout.

- [ ] **Step 5: Run all runtime tests**

Run:

```bash
cargo test -p cel-runtime --lib
```

Expected: all runtime, tuple, sequence, and new array tests pass.

- [ ] **Step 6: Commit**

```bash
git add cel-runtime/src/dynamic_array.rs cel-runtime/src/dyn_segment.rs cel-runtime/src/raw_stack.rs
git commit -m "feat(cel-runtime): collect homogeneous stack arrays"
```

---

### Task 5: Add Shared Array Grammar and AST Nodes

**Files:**
- Modify: `cel-parser/src/parser_context.rs`
- Modify: `cel-parser/src/ast.rs`
- Modify: `cel-parser/src/lib.rs`

**Interfaces:**
- Consumes: `DynSegment::make_array`.
- Produces:
  `ParserContext::make_array(&mut self, n, ambient_start, start, end) -> crate::Result<()>`.
- Produces: `Expr::Array { elements: Vec<Expr>, span: ExprSpan }`.
- Produces: `array_expression = "[" expression { "," expression } "]".`

- [ ] **Step 1: Write failing grammar and AST tests**

Add tests that parse `[0, 1, 2]` with both `DynSegmentContext` and `AstContext`, assert source-order
elements and enclosing span, and evaluate the runtime result to `Vec<i32>`.

Add rejection tests with exact message fragments:

```rust
assert!(parser.parse_str("[]").unwrap_err().message().contains("empty array"));
assert!(parser.parse_str("[0,]").unwrap_err().message().contains("expected expression after ','"));
assert!(parser.parse_str("[0, 1.0]").unwrap_err().message().contains("array element 1"));
assert!(parser.parse_str("[(0, 1)]").unwrap_err().message().contains("tuple"));
```

Add a nested success test for `[[0], [1]]` and recursive mismatch test for `[[0], [1.0]]`.

- [ ] **Step 2: Run the parser tests to verify failure**

Run:

```bash
cargo test -p cel-parser --lib array
```

Expected: tests fail because bracket groups are not primary expressions.

- [ ] **Step 3: Extend `ParserContext` and both implementations**

Add:

```rust
fn make_array(
    &mut self,
    n: usize,
    ambient_start: usize,
    start: Span,
    end: Span,
) -> crate::Result<()>;
```

`DynSegmentContext` maps `DynSegment::make_array` errors to `ParseError::new_range`. `AstContext`
uses `split_off(ambient_start)`, asserts `elements.len() == n`, and pushes `Expr::Array`.
Update `Expr::span` to include the new variant.

- [ ] **Step 4: Implement the LL(1) array production**

Add a bracket arm in `is_primary_expression`, then implement:

```rust
/// `array_expression = "[" expression { "," expression } "]".`
fn is_array_expression(&mut self) -> Result<bool>
```

After consuming `[`, reject an immediate `]` with the #212 diagnostic. Parse the first
`expression`, then repeatedly inspect one token: `]` completes the literal; otherwise require and
consume `,`, reject an immediate `]`, and require the next `expression`. Do not add speculative
lookahead or accept a trailing comma.

- [ ] **Step 5: Test value-producing grammar positions**

Add parser tests for arrays inside a call argument, tuple element, `if` branch, and closure body.
Use existing registered operations or AST parsing where runtime operation registration would
obscure the grammar assertion.

- [ ] **Step 6: Run parser and runtime tests**

Run:

```bash
cargo test -p cel-parser --lib array
cargo test -p cel-runtime --lib
```

Expected: all selected tests pass.

- [ ] **Step 7: Commit**

```bash
git add cel-parser/src/parser_context.rs cel-parser/src/ast.rs cel-parser/src/lib.rs
git commit -m "feat(cel-parser): parse homogeneous array literals"
```

---

### Task 6: Format Array Expressions

**Files:**
- Modify: `cel-parser/src/fmt.rs`

**Interfaces:**
- Consumes: `Expr::Array`.
- Produces: canonical bracket formatting without trailing commas.

- [ ] **Step 1: Write failing formatter tests**

```rust
#[test]
fn arrays_format_with_brackets_and_normalized_commas() {
    assert_eq!(fmt("[1i32,2i32]"), "[1i32, 2i32]");
    assert_eq!(fmt("[[1i32], [2i32]]"), "[[1i32], [2i32]]");
}

#[test]
fn array_comments_remain_in_source_order() {
    assert_eq!(
        fmt("[1i32 /* first */, /* second */ 2i32]"),
        "[1i32 /* first */, /* second */ 2i32]"
    );
}
```

- [ ] **Step 2: Run the formatter tests to verify failure**

Run:

```bash
cargo test -p cel-parser --lib fmt::tests
```

Expected: compilation fails because `render` has no `Expr::Array` arm.

- [ ] **Step 3: Render arrays through the existing list helper**

Add an `Expr::Array` arm adjacent to `Expr::Tuple`:

```rust
Expr::Array { elements, span } => {
    let list = emit_list(
        source,
        depth,
        ("[", "]"),
        (span.start.start(), span.end.end()),
        elements,
        false,
    );
    (list, Level::PRIMARY)
}
```

Update `emit_list` documentation so delimiters include brackets. Do not alter the tuple-only
`trailing_comma` behavior used for one-tuples.

- [ ] **Step 4: Run formatter and parser tests**

Run:

```bash
cargo test -p cel-parser --lib fmt::tests
cargo test -p cel-parser --lib array
```

Expected: formatter and array parser tests pass.

- [ ] **Step 5: Commit**

```bash
git add cel-parser/src/fmt.rs
git commit -m "feat(cel-parser): format array expressions"
```

---

### Task 7: Add Recursive Static Array Types

**Files:**
- Modify: `cel-parser/src/ty.rs`
- Modify: `adam-lang/src/typecheck.rs`

**Interfaces:**
- Consumes: `Expr::Array` and `DynamicArray`.
- Produces: `Ty::Array(Box<Ty>)`.
- Produces: recursive array unification and first-conflict diagnostics.

- [ ] **Step 1: Write failing type-checker tests**

Add helpers that build `Expr::Array`, then test:

```rust
#[test]
fn homogeneous_array_infers_its_element_type() {
    let (ty, diagnostics) = check_expr(
        &array(vec![lit_i32(1), lit_i32(2)]),
        &any_resolver,
    );
    assert_eq!(ty, Ty::Array(Box::new(Ty::I32)));
    assert!(diagnostics.is_empty());
}

#[test]
fn nested_array_types_unify_recursively() {
    let expr = array(vec![
        array(vec![lit_i32(1)]),
        array(vec![lit_i32(2)]),
    ]);
    assert_eq!(check_expr(&expr, &any_resolver).0, Ty::Array(Box::new(Ty::Array(Box::new(Ty::I32)))));
}

#[test]
fn heterogeneous_array_reports_the_conflicting_element() {
    let (_, diagnostics) =
        check_expr(&array(vec![lit_i32(1), lit_f64(2.0)]), &any_resolver);
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].message().contains("expected `i32`, found `f64`"));
}
```

Add a test showing `Ty::Any` remains conservative for call results and a recursive mismatch test.

- [ ] **Step 2: Run the focused tests to verify failure**

Run:

```bash
cargo test -p cel-parser --lib ty::tests
```

Expected: compilation fails because `Ty::Array` is missing.

- [ ] **Step 3: Extend `Ty` and its contracts**

Remove `Copy`, retain `Clone`, and add `Array(Box<Ty>)`. Update:

```rust
pub fn type_id(&self) -> Option<TypeId>
pub fn name(&self) -> Cow<'static, str>
pub fn unifies_with(&self, other: &Ty) -> bool
```

`type_id` maps arrays to `TypeId::of::<DynamicArray>()`; `name` recursively renders `[T]`;
`unifies_with` treats `Any` as a wildcard and compares two arrays recursively. `from_type_id` maps
the wrapper's flat ID to `Array(Box::new(Ty::Any))`, because no inner descriptor is available from
the ID alone.

- [ ] **Step 4: Implement array inference**

Add an `Expr::Array` arm to `check_expr`. Check every child first and retain all nested
diagnostics. Fold element types with a helper that keeps the more concrete side when one side is
`Any`, recursively merges arrays, and reports a diagnostic at the first incompatible element.
Return `Ty::Array(Box::new(element_ty))`.

- [ ] **Step 5: Update non-`Copy` downstream uses**

In `cel-parser/src/ty.rs`, replace indexing moves such as `operand_tys[0]` with borrowing or
`.clone()`. In `adam-lang/src/typecheck.rs`, replace `.copied()` with `.cloned()`, clone captured
`Ty` values only where ownership is required, and adjust matches to borrow. Do not add array type
annotations or `TypeShape::Array` to adam-lang in this task.

- [ ] **Step 6: Run static-checker and adam-lang tests**

Run:

```bash
cargo test -p cel-parser --lib ty::tests
cargo test -p adam-lang --lib typecheck
```

Expected: all type-checking tests pass and existing scalar/tuple diagnostics are unchanged.

- [ ] **Step 7: Commit**

```bash
git add cel-parser/src/ty.rs adam-lang/src/typecheck.rs
git commit -m "feat(cel-parser): infer homogeneous array types"
```

---

### Task 8: Complete Public Integration and Verification

**Files:**
- Modify: `cel-runtime/src/dynamic_array.rs`
- Modify: `cel-parser/src/lib.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: all prior tasks.
- Produces: documented end-to-end array behavior through both `cel_runtime` and `cel_rs`.

- [ ] **Step 1: Add end-to-end parser tests**

Register nullary `f`, `g`, and `h` operations returning the same custom `'static` type and assert
`[f(), g(), h()]` converts to the expected `Vec<T>`. Add a second registration where `h` returns a
different type and assert parsing fails before execution with the expected/found type names.

Add a public-facade doctest:

```rust
use cel_rs::parser::{CELParser, OpLookup};
use cel_rs::runtime::DynamicArray;

let mut segment = CELParser::new(OpLookup::new()).parse_str("[0, 1, 2]").unwrap();
let array: DynamicArray = segment.call0().unwrap();
assert_eq!(array.try_into_vec::<i32>().unwrap(), vec![0, 1, 2]);
```

- [ ] **Step 2: Run focused acceptance tests**

Run:

```bash
cargo test -p cel-parser --lib array
cargo test -p cel-rs --doc
```

Expected: homogeneous literals, custom-returning calls, nested arrays, and facade usage pass.

- [ ] **Step 3: Format the workspace**

Run:

```bash
cargo fmt --all
```

Expected: command exits successfully with no remaining formatting diff.

- [ ] **Step 4: Run the complete build and test suite**

Run:

```bash
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
```

Expected: all commands exit successfully with no compiler warnings and no test failures.

- [ ] **Step 5: Run all required clippy configurations**

Run:

```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

Expected: all three commands exit successfully with no warnings.

- [ ] **Step 6: Check library documentation**

Run:

```bash
$env:RUSTDOCFLAGS="-D warnings"; cargo doc --lib --no-deps --workspace
```

Expected: documentation builds without warnings.

- [ ] **Step 7: Review the complete branch diff**

Run:

```bash
git --no-pager diff main...HEAD --check
git --no-pager diff main...HEAD --stat
```

Confirm every acceptance criterion in
`docs/superpowers/specs/2026-09-15-cel-homogeneous-arrays-design.md` maps to a passing test and no
unrelated files changed.

- [ ] **Step 8: Commit final documentation or integration adjustments**

```bash
git add cel-runtime/src/dynamic_array.rs cel-parser/src/lib.rs src/lib.rs
git commit -m "docs(cel): document homogeneous arrays"
```
