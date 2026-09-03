# The Adam Programming Language

A tutorial and reference manual for **Adam**, the declarative language for describing
property models (sheets of cells linked by multi-way constraints), implemented by the
[`adam_lang`](../adam_lang/index.html) crate and executed by [`adam_rs`](../adam_rs/index.html).

## Why Adam

Many of an application's properties are related to each other: a document's own structure
holds invariants among them, a command's arguments and result constrain one another, and some
relationships exist purely to construct a new argument or document value, such as
`pixels == inches * resolution` so that entering any two of the three determines the third.

An application usually hand-codes each of these relationships as UI and scripting logic: one
event handler recomputes `pixels` when `resolution` changes, another recomputes `inches` when
`pixels` changes, and so on for every direction the relationship can be driven, repeated at
every control that touches the value. Adam models the relationship once, as a declarative
constraint; its solver derives whichever value the current interaction requires, so behavior
that would otherwise be hand-written event logic happens automatically.

Adam borrows its `sheet` and `cell` terms from spreadsheets: a sheet holds named, typed cells,
much like a spreadsheet holds named, typed values. A `relationship` plays the role of a
spreadsheet's equation cell, but unlike a spreadsheet formula, which computes in one direction
only, a relationship is multi-way: `a == b` means that changing `a` updates `b` to match, and
changing `b` updates `a` to match. Which cell is the source and which is derived is decided
each time the sheet is solved, not fixed by the declaration.

## About this book

This book has three parts: a tutorial introduction you can read start to finish, chapters that
go back over the same ground in more detail, and a terse reference manual for looking things up.
Every Adam source fragment shown in a fenced `adam` block is exactly what you'd type into a
`.adm2` file; a fenced `text` block instead shows grammar notation (EBNF), not Adam source
itself.

## Expressions and the standard library

Adam adds a declarative shell (`sheet`, `cell`, `source`, `relationship`, `conditional`, `out`,
`require`, and `filter`) around expressions written in the
[Common Expression Language](https://github.com/google/cel-spec) (CEL), turning a set of CEL
expressions into a live, bidirectional constraint graph. See
[`cel-parser`'s crate documentation](../cel_parser/index.html) for CEL's own grammar, operators,
literals, casts, and control-flow expressions.

Functions callable from inside an expression, such as `min`, `max`, `clamp`, and `round`, come
from a function library installed into the parser's `OpLookup`; this book's own examples install
[`cel-std`](../cel_std/index.html).
