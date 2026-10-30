# Lexical Conventions

Adam uses [CEL's lexical conventions](../cel-book/lexical-conventions.html):
identifiers, integer and float literals with optional type suffixes, string literals,
and punctuation. Adam also recognizes doc comments.

## Comments

`//` starts a line comment; `/* ... */` encloses a block comment:

```adam
// a whole-line comment
cell width: i32 = 1920; // a trailing comment
/* a block comment, on one line or several */
```

## Doc comments

`///` immediately before a `cell`, `source`, `relationship`, `conditional`, or `out`
declaration and `//!` immediately before `sheet` introduce *doc comments*.
The language server and formatter retain them; they do not affect sheet resolution:

```adam
{{#include examples/lexical-conventions/doc_comments.adm2}}
```

## Keywords and reserved identifiers

Adam recognizes `sheet`, `use`, `as`, `cell`, `source`, `relationship`, `conditional`,
`out`, `require`, and `filter` in their grammar positions. They are contextual:
cell names may use these words. Sheet definitions and module names reject `_`,
`sheet`, `use`, `as`, empty names, and raw identifiers. `_` is also
reserved in two specific positions: a `conditional`'s default branch
(`_ => { ... }`, the [default branch and reverting to source](conditionals.md#the-default-branch-and-reverting-to-source) section),
and inside a `filter` expression (the candidate value,
the [filter grammar](filters.md#grammar)); elsewhere it is an ordinary identifier.

## Punctuation

`:` (type annotation), `=` (cell initializer), `:=` (binding/output body), `=>` (conditional
branch), `;` (declaration terminator), `,` (list separator), `{ }` (block delimiters), `( )`
(tuple/grouping delimiters).
