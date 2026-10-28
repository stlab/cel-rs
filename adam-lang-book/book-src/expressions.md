# Expressions and Dependency Deduction

## Expressions are CEL

Everywhere Adam's grammar calls for `expression` (a cell initializer, a relationship
binding's right-hand side, a conditional's match subject or branch literal, an `out`
declaration's body, a `require`ment, a filter's body), the expression itself is parsed and
evaluated by `cel-parser`/`cel-runtime`, not by Adam. The standalone CEL
[reference manual](../cel-book/reference.html) and its book root
[here](../cel-book/index.html) document literals, arithmetic and comparison operators,
`if`/`else`, `as` casts, ranges (`lo..=hi`), function calls, and closures. This chapter covers
only what Adam does *around* an expression: deciding which cells it may read, and what it does
with the value it produces.

## No standard library of its own

A parser built with a bare [`OpLookup::new()`](../cel_parser/op_table/struct.OpLookup.html#method.new) and no library installed can still parse and run
every construct in this book except a function call — any attempt to call an undefined
function fails to parse with an error naming the missing function. This book's own examples
always install `cel-std` (see `support::parser` in `adam-lang-book`'s own source).

Host-registered records and members additionally require their own schemas and
operations; neither the bare language nor `cel-std` supplies application types.

## Cell initializers see no cells

A `cell`'s `= expression` initializer is evaluated with no cell scope at all. Referencing any
identifier that would otherwise name a cell is an "undeclared cell `name`" error, exactly
as if the cell had never been declared; see [error messages](reference.md#error-messages).

To compute one cell's value from another's, write a [`relationship`](relationships.md) or an
[`out`](outputs.md) declaration; both of those *do* get a live cell scope, per the next
section.

## Deduced dependencies

A `relationship` binding, a `conditional`'s match subject, an `out` declaration's body, and a
`require`ment body all share one mechanism for deciding which cells they read: **every**
identifier the expression references that names an already-declared cell becomes an input,
automatically; there is no explicit parameter list to write. Referencing the same cell more
than once (`a && a`) still counts as one input, not two. This is why Adam has no forward
references (see [names and declaration order](cells.md#names-and-declaration-order)): an identifier can only be recognized as a cell dependency if that cell was
declared earlier in the same sheet.

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
`conditional` subjects so `{` can begin the following body without backtracking.
Parenthesize the constructor-containing expression, for example
`conditional (Sample { value: 3.0 }.value) { ... }`. Parentheses, call arguments,
tuple elements, and array elements permit record expressions within such
headers. Unparenthesized operators, ranges, and closure bodies inherit the
header restriction.

Member assignment is not part of the language: update the whole cell from the
host or construct a new record as a relationship's output.

This executable host example registers the type, constructor, and accessor, then
checks the sheet with the same metadata used to run it. The `Sample` and `value`
cells do not shadow either constructor or member resolution:

```rust
{{#include ../tests/expressions.rs:registered-record}}
```

## "Expression produced no value"

Every place an `expression` is required must produce a value one of Adam's registered
types recognizes, at the end of parsing: a bare CEL statement with no trailing value, or an
expression whose result type isn't registered with the
[`TypeRegistry`](../adam_lang/type_registry/struct.TypeRegistry.html) in use, is a parse error naming the
construct it came from: `expression produced no value`, or
``cannot infer a type: unregistered type `T` ``.
Register the result's leaf types with the `TypeRegistry`; a type annotation alone
does not register an unknown host type.
