# Registry-backed typed array suffixes

## Status

Approved design for issue #212.

## Goal

Extend the existing homogeneous CEL array literals with an optional element-type suffix:

```text
[0, 1]i32
[1.0, 42.5]f64
[]i32
[[]i32][i32]
```

Non-empty arrays without a suffix retain their current inference behavior. An empty array
requires a suffix because it has no element expression from which to infer a runtime descriptor.
The suffix is a recursive type expression: a registered scalar name or a bracketed array type.
Thus `[[]i32][i32]` is an outer array of inner arrays of `i32`, and `[][i32]` is an empty outer
array with that element type. Whitespace between the literal and a bracketed suffix is accepted
where the token stream permits it.

## Boundaries and data flow

`cel-parser` must remain independent of `adam-lang`, while `adam-lang` must be able to use its
custom `TypeRegistry` entries. The parser therefore receives a small type-resolution/descriptor
adapter rather than a concrete registry:

1. The lexer/parser recognizes a recursive type expression immediately following an array's
   closing `]`.
2. The adapter resolves that expression to a runtime element descriptor and its `TypeId`/nested
   descriptors.
3. The parser context validates non-empty elements against the resolved descriptor, or constructs a
   typed empty array from it.
4. `DynSegmentContext` emits a `DynamicArray` operation/value.
5. `AstContext` stores the unresolved suffix type expression and span; semantic checking resolves
   it later.

The default `CELParser` continues to work with built-in registered scalar names. `AdamParser`
passes an adapter over its `TypeRegistry`, so custom `register` and `register_no_default` leaf
types are available in both non-empty and empty suffixed arrays.

### Registry ownership

Do not move `adam-lang::TypeRegistry` wholesale into `cel-parser`: its entries include
sheet-specific constructors (`add_cell_fn`, conditional construction, and output extraction) that
are not CEL concerns. Instead, extract or define a small generic CEL-facing registry/trait whose
entries contain only type-name resolution and the runtime descriptor required to build and
validate arrays (type identity, layout, drop/clone/equality/debug hooks, and recursive array
composition). `adam-lang::TypeRegistry` implements or adapts that interface. A future `cel-std`
type package can register Euclid and other custom types through the same generic interface without
depending on Adam's sheet machinery.

## Runtime contract

`DynamicArray` already stores recursive element metadata and supports zero-copy conversion to and
from `Vec<T>`. Add a typed-empty constructor (or equivalent builder operation) that accepts an
`ArrayElementType` without requiring a `Vec<T>` element to infer it. Preserve the existing
allocation, zero-sized-type, drop, and nested-array invariants.

The parser context operation accepts an optional resolved descriptor:

- no suffix: current inferred homogeneous-array operation;
- suffix with elements: validate every element's complete recursive runtime shape against the
  descriptor, then collect the values;
- suffix with no elements: emit an empty `DynamicArray` carrying the descriptor;
- unknown suffix: return a span-labelled parse error;
- mismatching element: return a span-labelled parse error naming the expected and found types.

No implicit numeric conversion is performed. A literal's type must exactly match the suffix type;
for example, `[1, 2]f64` is rejected unless the expressions explicitly produce `f64`.

The suffix type expression is parsed independently from the array body and must be fully
consumed. Its scalar leaves resolve through the configured registry; array nodes recursively
compose the corresponding `ArrayElementType` descriptors.

## AST and static checking

Change `Expr::Array` to retain:

- `elements: Vec<Expr>`;
- `element_type: Option<ArrayTypeExpr>` where `ArrayTypeExpr` is a recursive unresolved scalar or
  array type expression;
- the suffix span when present;
- the enclosing array span.

`AstContext` continues to defer semantic resolution. `check_expr` resolves a suffix through the
configured type resolver, reports unknown scalar leaves, recursively validates the suffix shape,
requires every element to unify with the declared element type, and returns nested `Ty::Array`
values. A suffix on a non-empty array is still checked even when inference would otherwise
succeed. An unsuffixed empty array remains a diagnostic.

The formatter emits the suffix as a normalized scalar or bracketed type expression after `]`,
preserving the existing comment/trivia behavior around the array body.

## Errors and compatibility

Existing unsuffixed non-empty arrays and nested arrays retain their behavior. New diagnostics
cover malformed suffix placement, unknown registered names, empty arrays without suffixes, and
element/suffix type mismatches. A complete scalar or bracketed type expression immediately
following `]` is consumed as a suffix; any other token remains a normal parser error.

The direct `cel-parser` API exposes a resolver configuration with built-in defaults. Existing
constructors remain source-compatible where practical; the new resolver is an explicit opt-in for
custom types, and `AdamParser` configures it automatically.

## Tests

Add contract tests covering:

- `[0, 1]i32`, `[1.0, 42.5]f64`, and `[]i32`;
- optional suffixes on non-empty arrays and preservation of unsuffixed inference;
- suffix mismatch, unknown suffix, missing empty-array suffix, and malformed trailing tokens;
- custom `TypeRegistry` types in non-empty and empty arrays;
- nested suffixes (`[[]i32][i32]`, `[][i32]`) and recursive descriptor mismatch;
- AST shape, type diagnostics, and formatter round trips;
- zero-copy conversion and empty-array capacity/layout invariants through the existing runtime
  tests.
