# Chapter 7: Outputs and Requirements

## 7.1 Grammar

```text
out_decl    = "out" identifier [ ":" type_expr ] ":=" expression
               [ "require" "{" { requirement } "}" ] ";".
requirement = identifier ":" expression ";".
```

An `out` declares a new cell computed by exactly one expression — there's no alternative
binding to choose between, unlike a `relationship`. Its dependencies are
[deduced](expressions.md#34-deduced-dependencies) the same way. The `: type_expr` annotation is
optional; when absent, the output's type is inferred from the initializer, the same rule
[Chapter 2](cells.md#23-built-in-types-and-inference) gives for a plain `cell`.

```rust
{{#include ../tests/outputs.rs:basic_output}}
```

## 7.2 An output's cell is terminal

`out` shares one namespace with `cell` — declaring `out result := ...;` after (or before) a
`cell result` in the same sheet is a duplicate-name error, exactly like two `cell` declarations
would be. Unlike a plain cell, though, an output's cell can never be *written*: not by a host
`write()` call, not by a `relationship` binding, not by a second `out`. It's computed exactly
once per `propagate()`, by its own initializer, and nothing else:

```rust
{{#include ../tests/outputs.rs:output_cell_is_terminal}}
```

An output cell is nonetheless an ordinary cell for *reading*: a later declaration in the same
sheet can reference an earlier `out` by name in its own expression, exactly like referencing
any other already-declared cell.

## 7.3 Requirements: diagnostics, not gates

A `require { ... }` block trailing an `out`'s initializer names zero or more boolean checks.
Each `requirement`'s own dependencies are deduced separately from the output's initializer — a
requirement commonly reads the output's own value by name, alongside whatever other cells it
needs:

```rust
{{#include ../tests/outputs.rs:requirement_diagnostic}}
```

A failed requirement never stops `propagate()` from succeeding, and never stops `area` from
being computed and readable — `output_valid`/`violated_requirements` exist precisely because
nothing else in the sheet notices a requirement failing on its own. A requirement's `name` is
just a label passed through to the query API; it happens to read naturally when it echoes a
cell name (`not_too_big`, `width_max`), but it isn't a cell reference and doesn't have to match
one.

## 7.4 Multiple requirements

An output can list any number of requirements; `violated_requirements` reports exactly the ones
currently failing, by [`RequirementId`](../adam_rs/requirement/struct.RequirementId.html):

```rust
{{#include ../tests/outputs.rs:multiple_requirements}}
```
