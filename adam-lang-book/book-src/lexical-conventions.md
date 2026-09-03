# Chapter 10: Lexical Conventions

An Adam source file is tokenized as Rust/CEL tokens (via `proc_macro2`): identifiers, integer
and float literals (with optional type suffixes), string literals, and punctuation, exactly as
[`cel-parser`'s own lexical grammar](../cel_parser/index.html) defines them. Adam adds exactly
one lexical extension on top of CEL's own conventions: doc comments.

## 10.1 Comments

`//` starts a line comment; `/* ... */` a block comment — the same two forms C, Rust, and CEL
all share:

```adam
// a whole-line comment
cell width: i32 = 1920; // a trailing comment
/* a block comment, on one line or several */
```

## 10.2 Doc comments

`///` immediately before a `cell`, `source`, `relationship`, `conditional`, or `out`
declaration, and `//!` immediately before the `sheet` keyword itself, are Adam's own addition to
CEL's lexical grammar: doc comments, recovered by the language server and the formatter, and
otherwise inert — they carry no meaning when the sheet resolves:

```adam
{{#include examples/lexical-conventions/doc_comments.adm2}}
```

## 10.3 Keywords and reserved identifiers

**Keywords**: `sheet`, `cell`, `source`, `relationship`, `conditional`, `out`, `require`,
`filter`. None of these can be used as a cell or sheet name. `_` is not a keyword but is
reserved in two specific positions: a `conditional`'s default branch
(`_ => { ... }`, [Chapter 9 §9.5](conditionals.md#95-the-default-branch-and-reverting-to-source)),
and inside a `filter` expression (the candidate value,
[Chapter 5 §5.1](filters.md#51-grammar)); elsewhere it is an ordinary identifier.

## 10.4 Punctuation

`:` (type annotation), `=` (cell initializer), `:=` (binding/output body), `=>` (conditional
branch), `;` (declaration terminator), `,` (list separator), `{ }` (block delimiters), `( )`
(tuple/grouping delimiters).
