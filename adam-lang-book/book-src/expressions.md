# Expressions and Dependency Deduction

## Expressions are CEL

Adam uses CEL expressions in cell initializers, relationship bindings, conditional subjects,
outputs, requirements, and filters. `cel-parser` parses them; `cel-runtime` evaluates them.
The [CEL reference manual](../cel-book/reference.html) defines literals, operators,
`if`/`else`, casts, ranges, function calls, and closures. This chapter explains which cells
an expression may read and how Adam uses its result.

## No standard library of its own

A bare [`OpLookup::new()`](../cel_parser/op_table/struct.OpLookup.html#method.new)
provides CEL's built-in operations but no function library. Calling an undefined function
fails at parse time with an error naming the function. This book's examples install `cel-std`.

Host-registered records and members additionally require their own schemas and
operations; neither the bare language nor `cel-std` supplies application types.

## Cell initializers see no cells

A `cell`'s `= expression` initializer has no cell scope. Referencing a cell, even one
already declared, produces an "undeclared cell `name`" error at parse time;
see [error messages](reference.md#error-messages).

To compute one cell's value from another's, write a [`relationship`](relationships.md) or an
[`out`](outputs.md) declaration.

## Deduced dependencies

A relationship binding, conditional subject, output body, or requirement *deduces*
its inputs from references to already-declared cells. No explicit parameter list is needed.
Referencing the same cell more than once (`a && a`) counts as one input.
Cells must be [declared before use](cells.md#names-and-declaration-order).

A [`filter`](filters.md) clause uses the same deduction, plus one reserved identifier: `_`
always denotes the candidate value being conformed, never a cell; see the [filters chapter](filters.md).

Constructor names, record field labels, and member names are not value references.
The only dependencies of `TestPoint { x: a, y: b }.x` are `a` and `b`, even if
cells named `TestPoint`, `x`, and `y` exist. A closure parameter shadows only value
references in its body, not those labels or constructor names.

## Host records and members

Write `Sample { value: 3.0 }` when the host registers a `Sample` schema.
Supply every field exactly once; unknown constructors and unknown, duplicate,
missing, or incorrectly typed fields are errors. Field expressions evaluate in
written order. Empty schemas accept `{}`; non-empty field lists reject trailing
commas. An `f64` field requires `3.0`, not `3`: integers do not
implicitly convert to floats.

`reading.value` and `reading.value()` both invoke the host's unary `value`
operation. `reading.operation(a)` passes `reading` first, then `a`. A same-named
cell cannot hide a dot operation. `(reading.value)()` instead calls the
operation's result and fails when that value is not callable.

Bare record constructors are forbidden in `if` conditions and Adam
`conditional` subjects, where `{` begins the following body.
Parenthesize the constructor-containing expression, for example
`conditional (Sample { value: 3.0 }.value) { ... }`. Parentheses, call arguments,
tuple elements, and array elements permit record expressions within such
headers. Unparenthesized operators, ranges, and closure bodies inherit the
header restriction.

Member assignment is not part of the language: update the whole cell from the
host or construct a new record as a relationship's output.

This host example registers the type, constructor, and accessor, then
checks the sheet with the same metadata used to run it. The `Sample` and `value`
cells do not shadow either constructor or member resolution:

```rust
{{#include ../tests/expressions.rs:registered-record}}
```

## "Expression produced no value"

Every required expression must produce a value whose leaf types are registered with
the [`TypeRegistry`](../adam_lang/type_registry/struct.TypeRegistry.html).
Parsing reports `expression produced no value` for a body without a result, or
``cannot infer a type: unregistered type `T` `` for an unregistered result type.
These errors identify the enclosing construct. A type annotation does not register a host type.
