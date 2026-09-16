# Registry-backed typed array annotations

## Status

Approved design for issue #212.

## Goal

Extend the existing homogeneous CEL array literals with an optional expression type ascription:

```text
[0, 1]: [i32]
[1.0, 42.5]: [f64]
[]: [i32]
[[]: [i32]]: [[i32]]
```

Non-empty arrays without an annotation retain their current inference behavior. An empty array
requires an annotation because it has no element expression from which to infer a runtime descriptor.
The annotation names the complete array type, not merely its element type. Its type expression is
recursive, so `[i32]` is an array of `i32` and `[[i32]]` is an array of arrays. Tuple type
expressions remain part of the reusable type grammar, but tuple-valued array elements remain
rejected by the existing runtime limitation tracked in issue #213. Whitespace around `:` is
accepted.

## Boundaries and data flow

`cel-parser` must remain independent of `adam-lang`, while `adam-lang` must be able to use its
custom `TypeRegistry` entries. The parser therefore receives a small type-resolution/descriptor
adapter rather than a concrete registry:

1. The lexer/parser recognizes `:` followed by a recursive type expression after an array
   expression.
2. The adapter resolves the array type's element expression to a runtime element descriptor and its
   `TypeId`/nested
   descriptors.
3. The parser context validates non-empty elements against the resolved descriptor, or constructs a
   typed empty array from it.
4. `DynSegmentContext` emits a `DynamicArray` operation/value.
5. `AstContext` stores the unresolved type annotation and span; semantic checking resolves it later.

The default `CELParser` continues to work with built-in registered scalar names. `AdamParser`
passes an adapter over its `TypeRegistry`, so custom `register` and `register_no_default` leaf
types are available in both non-empty and empty annotated arrays.

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

The parser context operation accepts an optional resolved array descriptor:

- no annotation: current inferred homogeneous-array operation;
- annotation with elements: validate every element's complete recursive runtime shape against the
  descriptor, then collect the values;
- annotation with no elements: emit an empty `DynamicArray` carrying the descriptor;
- unknown type leaf: return a span-labelled parse error;
- mismatching element: return a span-labelled parse error naming the expected and found types.

No implicit numeric conversion is performed. A literal's type must exactly match the annotated
array element type; for example, `[1, 2]: [f64]` is rejected unless the expressions explicitly
produce `f64`.

The type expression is parsed independently from the array body and must be fully consumed. Its
scalar leaves resolve through the configured registry; array nodes recursively compose the
corresponding `ArrayElementType` descriptors. The same type-expression grammar should be reusable
for future declarations, parameters, pattern bindings, and other typed expression forms.

## AST and static checking

Change `Expr::Array` to retain:

- `elements: Vec<Expr>`;
- `type_annotation: Option<TypeExpr>` where `TypeExpr` is a recursive unresolved scalar, array,
  or tuple type expression;
- the annotation span when present;
- the enclosing array span.

`AstContext` continues to defer semantic resolution. `check_expr` resolves an annotation through
the configured type resolver, reports unknown scalar leaves, recursively validates the annotated
array shape, requires every element to unify with the declared element type, and returns nested
`Ty::Array` values. An annotation on a non-empty array is still checked even when inference would
otherwise succeed. An unannotated empty array remains a diagnostic.

The formatter emits the annotation as `: Type` after the array expression, preserving the existing
comment/trivia behavior around the array body.

## Errors and compatibility

Existing unannotated non-empty arrays and nested arrays retain their behavior. New diagnostics
cover malformed annotation placement, unknown registered names, empty arrays without annotations,
and element/annotation type mismatches. A colon followed by a complete type expression is consumed
as an annotation; any other token remains a normal parser error. CEL's block-form `if`/`else`
conditional has no `?:` colon separator, so this postfix annotation does not conflict with the
existing conditional grammar.

The direct `cel-parser` API exposes a resolver configuration with built-in defaults. Existing
constructors remain source-compatible where practical; the new resolver is an explicit opt-in for
custom types, and `AdamParser` configures it automatically.

## Tests

Add contract tests covering:

- `[0, 1]: [i32]`, `[1.0, 42.5]: [f64]`, and `[]: [i32]`;
- optional annotations on non-empty arrays and preservation of unannotated inference;
- annotation mismatch, unknown type leaves, missing empty-array annotations, and malformed trailing
  tokens;
- custom `TypeRegistry` types in non-empty and empty arrays;
- nested array annotations (`[]: [[i32]]`) and recursive descriptor mismatch;
- reusable tuple type-expression parsing plus a clear rejection for tuple-valued array elements
  until issue #213 is implemented;
- AST shape, type diagnostics, and formatter round trips;
- zero-copy conversion and empty-array capacity/layout invariants through the existing runtime
  tests.
