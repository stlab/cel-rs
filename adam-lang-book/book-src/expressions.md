# Chapter 4: Expressions and Dependency Deduction

## 4.1 Expressions are CEL

Everywhere Adam's grammar calls for `expression` (a cell initializer, a relationship
binding's right-hand side, a conditional's match subject or branch literal, an `out`
declaration's body, a `require`ment, a filter's body), the expression itself is parsed and
evaluated by `cel-parser`/`cel-runtime`, not by Adam. Literals, arithmetic and comparison
operators, `if`/`else`, `as` casts, ranges (`lo..=hi`), function calls, and closures are all
CEL, and are documented by `cel-parser`'s own crate documentation, not here. This chapter
covers only what Adam does *around* an expression: deciding which cells it may read, and
what it does with the value it produces.

## 4.2 No standard library of its own

Adam defines no functions. `min`, `max`, `clamp`, `round`, and anything else callable from
inside an expression come from a function library installed into the
[`OpLookup`](../cel_parser/op_table/struct.OpLookup.html) passed to
[`AdamParser::new`](../adam_lang/struct.AdamParser.html#method.new); this book's own examples
install `cel-std` (see `support::parser` in `adam-lang-book`'s own source). A parser built with
a bare `OpLookup::new()` and no library installed can still parse
and run every construct in this book except a function call:

```adam
{{#include examples/expressions/no_standard_library.adm2}}
```

## 4.3 Cell initializers see no cells

A `cell`'s `= expression` initializer is evaluated exactly once, eagerly, at the moment
`cell`'s own declaration is parsed, with no cell scope pushed at all. It may use literals,
operators, and library functions, but referencing *any* identifier that would otherwise name a
cell is unresolved:

```adam
{{#include examples/expressions/initializer_sees_no_cells.adm2}}
```

To compute one cell's value from another's, write a [`relationship`](relationships.md) or an
[`out`](outputs.md) declaration; both of those *do* get a live cell scope, per the next
section.

## 4.4 Deduced dependencies

A `relationship` binding, a `conditional`'s match subject, an `out` declaration's body, and a
`require`ment body all share one mechanism for deciding which cells they read: **every**
identifier the expression references that names an already-declared cell becomes an input,
automatically; there is no explicit parameter list to write. Referencing the same cell more
than once (`a && a`) still counts as one input, not two. This is why Adam has no forward
references (2.6): an identifier can only be recognized as a cell dependency if that cell was
declared earlier in the same sheet.

A [`filter`](filters.md) clause uses the same deduction, plus one reserved identifier: `_`
always denotes the candidate value being conformed, never a cell; see [Chapter 7](filters.md).

## 4.5 "Expression produced no value"

Every place an `expression` is required must produce a value one of Adam's registered
types recognizes, at the end of parsing: a bare CEL statement with no trailing value, or an
expression whose result type isn't registered with the
[`TypeRegistry`](../adam_lang/type_registry/struct.TypeRegistry.html) in use, is a parse error naming the
construct it came from: `expression produced no value`, or `cannot infer a type for this
expression; register a type name for it or add an explicit ": type_expr" annotation`.
