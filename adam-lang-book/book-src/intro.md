# The Adam Programming Language

This book is a tutorial and reference manual for *Adam*, the declarative language for describing
property models (sheets of cells linked by multi-way constraints), implemented by the
[`adam_lang`](../adam_lang/index.html) crate and executed by [`adam_rs`](../adam_rs/index.html).

## Why Adam

Application properties constrain one another. For example,
`pixels == inches * resolution` lets any two values determine the third.
Such relationships preserve document invariants, constrain command arguments and results,
or construct new values.

Applications often implement these relationships as UI and scripting logic: one
event handler recomputes `pixels` when `resolution` changes, another recomputes `inches` when
`pixels` changes, and so on for every direction the relationship can be driven, repeated at
every control that touches the value. Adam models the relationship once, as a declarative
constraint; its solver derives whichever value the current interaction requires.

Adam borrows its `sheet` and `cell` terms from spreadsheets: a sheet holds named, typed cells,
much like a spreadsheet holds named, typed values. A `relationship` plays the role of a
spreadsheet's equation cell, but unlike a spreadsheet formula, which computes in one direction
only, a relationship is multi-way: `a == b` means that changing `a` updates `b` to match, and
changing `b` updates `a` to match. Cell strength determines which cell is the source
and which is derived. Typically, cells modified most recently have greater strength.

## About this book

The tutorial introduces Adam through examples; the following chapters explain each construct,
and the reference manual collects the grammar and rules.
Fenced `adam` blocks contain `.adm2` source; fenced `text` blocks contain grammar notation (EBNF).

## Expressions and the standard library

Adam adds a declarative shell (`sheet`, `cell`, `source`, `relationship`, `conditional`, `out`,
`require`, and `filter`) around expressions written in the
[Common Expression Language](../cel-book/index.html) (CEL), turning a set of CEL
expressions into a live, bidirectional constraint graph. See the
[CEL language book](../cel-book/index.html) for CEL's own grammar, operators,
literals, casts, and control-flow expressions.

Functions callable from inside an expression, such as `min`, `max`, `clamp`, and `round`, come
from a function library installed into the parser's `OpLookup`; this book's examples install
[`cel-std`](../cel_std/index.html).
