# Lexical Conventions

An Adam source file is tokenized as Rust/CEL tokens (via `proc_macro2`): identifiers, integer
and float literals (with optional type suffixes), string literals, and punctuation, exactly as
[`cel-parser`'s own lexical grammar](../cel_parser/index.html) defines them. Adam adds exactly
one lexical extension on top of CEL's own conventions: doc comments.

## Comments

`//` starts a line comment; `/* ... */` a block comment — the same two forms C, Rust, and CEL
all share:

```adam
// a whole-line comment
cell width: i32 = 1920; // a trailing comment
/* a block comment, on one line or several */
```

## Doc comments

`///` immediately before a `cell`, `source`, `relationship`, `conditional`, or `out`
declaration, and `//!` immediately before the `sheet` keyword itself, are Adam's own addition to
CEL's lexical grammar: doc comments, recovered by the language server and the formatter, and
otherwise inert — they carry no meaning when the sheet resolves:

```adam
{{#include examples/lexical-conventions/doc_comments.adm2}}
```

## Keywords and reserved identifiers

**Keywords**: `sheet`, `cell`, `source`, `relationship`, `conditional`, `out`, `require`,
`filter`. None of these can be used as a cell or sheet name. `_` is not a keyword but is
reserved in two specific positions: a `conditional`'s default branch
(`_ => { ... }`, the [default branch and reverting to source](conditionals.md#the-default-branch-and-reverting-to-source) section),
and inside a `filter` expression (the candidate value,
the [filter grammar](filters.md#grammar)); elsewhere it is an ordinary identifier.

## Punctuation

`:` (type annotation), `=` (cell initializer), `:=` (binding/output body), `=>` (conditional
branch), `;` (declaration terminator), `,` (list separator), `{ }` (block delimiters), `( )`
(tuple/grouping delimiters).
