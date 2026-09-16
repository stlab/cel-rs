# Registry-backed typed array suffixes

## Status

Approved design for issue #212.

## Goal

Extend the existing homogeneous CEL array literals with an optional element-type suffix:

```text
[0, 1]i32
[1.0, 42.5]f64
[]i32
```

Non-empty arrays without a suffix retain their current inference behavior. An empty array
requires a suffix because it has no element expression from which to infer a runtime descriptor.
The suffix names a registered scalar type. Nested arrays remain expressible by suffixing their
inner arrays (`[[]i32]`); recursive array-type suffix syntax is not introduced in this change.

## Boundaries and data flow

`cel-parser` must remain independent of `adam-lang`, while `adam-lang` must be able to use its
custom `TypeRegistry` entries. The parser therefore receives a small type-resolution/descriptor
adapter rather than a concrete registry:

1. The lexer/parser recognizes an identifier immediately following an array's closing `]`.
2. The adapter resolves that name to a runtime element descriptor and its `TypeId`.
3. The parser context validates non-empty elements against the resolved descriptor, or constructs a
   typed empty array from it.
4. `DynSegmentContext` emits a `DynamicArray` operation/value.
5. `AstContext` stores the unresolved suffix name and span; semantic checking resolves it later.

The default `CELParser` continues to work with built-in registered scalar names. `AdamParser`
passes an adapter over its `TypeRegistry`, so custom `register` and `register_no_default` types are
available in both non-empty and empty suffixed arrays.

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

## AST and static checking

Change `Expr::Array` to retain:

- `elements: Vec<Expr>`;
- `element_type: Option<String>`;
- the suffix span when present;
- the enclosing array span.

`AstContext` continues to defer semantic resolution. `check_expr` resolves a suffix through its
existing type resolver, reports unknown suffixes, requires every element to unify with the
declared element type, and returns `Ty::Array` of that type. A suffix on a non-empty array is
still checked even when inference would otherwise succeed. An unsuffixed empty array remains a
diagnostic.

The formatter emits the suffix exactly as a normalized identifier after `]`, preserving the
existing comment/trivia behavior around the array body.

## Errors and compatibility

Existing unsuffixed non-empty arrays and nested arrays retain their behavior. New diagnostics
cover malformed suffix placement, unknown registered names, empty arrays without suffixes, and
element/suffix type mismatches. A bare identifier immediately following `]` is consumed as a
suffix; any other token remains a normal parser error.

The direct `cel-parser` API exposes a resolver configuration with built-in defaults. Existing
constructors remain source-compatible where practical; the new resolver is an explicit opt-in for
custom types, and `AdamParser` configures it automatically.

## Tests

Add contract tests covering:

- `[0, 1]i32`, `[1.0, 42.5]f64`, and `[]i32`;
- optional suffixes on non-empty arrays and preservation of unsuffixed inference;
- suffix mismatch, unknown suffix, missing empty-array suffix, and malformed trailing tokens;
- custom `TypeRegistry` types in non-empty and empty arrays;
- nested arrays and recursive descriptor mismatch;
- AST shape, type diagnostics, and formatter round trips;
- zero-copy conversion and empty-array capacity/layout invariants through the existing runtime
  tests.

