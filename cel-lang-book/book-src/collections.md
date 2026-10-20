# Collections

CEL uses tuples and arrays for compound values. Tuples preserve fixed
positional shape. Arrays hold homogeneous sequences of values; an empty array
uses a type annotation to state its element type. For the complete grammar,
see the [Reference Manual](reference.md).

## Concept

Parentheses and brackets build different kinds of values. Parentheses can
mean unit, grouping, or tuples. Brackets always mean arrays. That shape
difference matters because tuple indexing uses `.N`, while arrays follow
their own literal rules and do not use tuple indexing syntax.

## Syntax

```text
tuple_or_group = "(" [ expression [ "," [ expression { "," expression } ] ] ] ")" .
array_expression = "[" [ expression { "," expression } ] "]"
                 [ ":" type_expr ] .
type_expr = identifier [ "(" type_expr { "," type_expr } ")" ]
          | "[" type_expr "]"
          | "(" [ type_expr [ "," [ type_expr { "," type_expr } ] ] ] ")" .
```

## Worked examples

```text
(1,)
(1, 2, 3)
[0, 1, 2]
[[0], [1]]
[0, 1]: [i32]
[]: [i32]
[]: [[i32]]
[(7, [1.5]), (8, []: [f64])]
[]: [(i32, [f64])]
[]: [Fn((i32,), [i32])]
if ready { [1, 2] } else { [3, 4] }
```

## Exact rules

- `()` is unit, `(expr)` is grouping, `(expr,)` is a 1-tuple, and
  `(a, b, c)` is a tuple.
- `[a, b, c]` is an array literal.
- An unannotated array is non-empty and every element must have the same type.
- A type annotation after `:` states the complete array type. It permits an
  empty array, as in `[]: [i32]`.
- A non-empty array annotation must match the element type exactly.
- Nested arrays are valid when each element is itself an array of the
  same element type.
- Tuples and arrays are distinct value forms: tuples are positional and
  use `.N`, while arrays use bracket literals and homogeneous element
  typing.
- Tuple indexing uses an unsuffixed integer such as `.0` or `.1`. Named
  postfixes such as `value.name` invoke host-provided operations; they do not
  select tuple fields.
- Arrays and tuples compose recursively over registered leaves, unit, other
  containers, and callables. Callable element types include their complete
  parameter and result shapes; storing a callable does not require equality.

## Rust interoperability

The recursive mapping uses concrete values, not per-element erased owners:

| CEL shape | Rust value | Representation witness |
| --- | --- | --- |
| registered leaf `T` | `T` | `Leaf<T>` |
| `[T]` | `Array<R(T)>` | `ArrayOf<W(T)>` |
| `()` | `CNil<()>` | `Nil` |
| `(T, U)` | `CStackList<R(T), CStackList<R(U), CNil<()>>>` | `Cons<W(T), Cons<W(U), Nil>>` |
| `Fn((A,), R)` | `Callable<CStackList<R(A), CNil<()>>, R(R)>` | `FnOf<Cons<W(A), Nil>, W(R)>` |

Here `R(T)` is the recursively canonical value and `W(T)` its witness.
`IntoCStackList` and `IntoRustTuple` move between lists and ordinary Rust tuples
without requiring `Clone`, equality, or formatting. Logical list order is source
order even though the `repr(C)` cons cell physically stores its tail first.
Never cast native Rust tuple bytes to a list.

This executable example prepares an array of actual lists, then moves each row
into a native Rust tuple:

```rust
{{#include ../tests/composable_containers.rs:tuple-arrays}}
```

### Nested vectors and slices

`Array<T>::from_vec` and `into_vec` transfer the allocation in O(1);
`as_slice` borrows the same elements as `&[T]`. They do not reinterpret Rust's
unspecified `Vec<T>` header.

The representation of `[[i32]]` is `Array<Array<i32>>`. Its allocation-preserving
vector view is `Vec<Array<i32>>`, and its slice view is `&[Array<i32>]`, **not**
`Vec<Vec<i32>>` or `&[Vec<i32>]`. An explicit deep conversion builds a new outer
vector while transferring each inner allocation independently:

```rust
{{#include ../tests/composable_containers.rs:nested-vector-view}}
```

### One operation for native and interpreted callers

`op1_repr` binds complete canonical witnesses, `op1_vec` moves an array into an
owned vector for a callback, and `op1_slice` borrows its elements for the callback
without consuming or rebuilding their allocation to obtain the borrow. The
operation still owns its operand and drops it after the slice callback.
The following ordinary Rust function serves both native construction and CEL.
`register_function` binds its ordinary-call name; `register_operation` supplies
the complete signature and emitter:

```rust
{{#include ../tests/composable_containers.rs:shared-operation}}
```

This is native Rust construction, **not** a macro-compiled CEL backend.
`cel-rs-macros` validates CEL at compile time; it does not generate native CEL.

### Checked boundaries and prepared execution

`ValueType` records complete identity, layout, ownership, and optional
capabilities. `ResolvedType` and the checker type `Ty` retain recursive shapes,
including callable signatures. Equal sizes or shared carrier IDs cannot prove
identity, even for empty arrays.

`OwnedValue::try_into_typed` checks arbitrary external ownership against a full
witness. `DynSegment::prepare::<Args, Result>` instead checks a typed frame once
and produces `PreparedSegment`; its repeated `call` borrows canonical arguments
and returns independent owners without descriptor comparisons, downcasts, or
per-element validation. Hosts with runtime-bound storage can use
`prepared_segment::PreparedRawSegment`: construction checks descriptors, but
its unsafe `call` requires exact, live, aligned pointers for that prepared frame.
Neither API removes value-dependent bounds checks or body errors.

Clone, equality, and debug formatting are separate capabilities. Construction
and moving ownership do not require them; borrowing an emitted argument read
requires cloning, and requesting comparison or formatting requires the matching
capability. The examples use built-in capabilities, not an implicit bound on
all CEL values.

## Edge cases

- `[]` is not valid because an empty array needs a type annotation.
- `[0,]` is not valid.
- `[]: [i32]` is a valid typed empty array.
- `[0, true]` is not valid because the element types differ.
- `[(0, 1)]` and `[]: [(i32, i32)]` are valid tuple arrays.
- `[]: [[i32]]` is a valid typed empty nested array.
- `[(0,), (true,)]` is invalid because the complete element shapes differ.
- Bracket postfix indexing such as `items[0]` is not part of the
  language.
- A suffixed tuple index such as `.0i32` is not valid.
