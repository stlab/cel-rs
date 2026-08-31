# Chapter 7: Outputs and Requirements

## 7.1 Grammar

```text
out_decl    = "out" identifier [ ":" type_expr ] ":=" expression
               [ "require" "{" { requirement } "}" ] ";".
requirement = identifier ":" expression ";".
```

An `out` declares a new cell computed by exactly one expression: there's no alternative
binding to choose between, unlike a `relationship`. Its dependencies are
[deduced](expressions.md#34-deduced-dependencies) the same way. The `: type_expr` annotation is
optional; when absent, the output's type is inferred from the initializer, the same rule
[Chapter 2](cells.md#23-built-in-types-and-inference) gives for a plain `cell`.

```
{{#include examples/outputs/basic_output.adm2}}
```

## 7.2 An output's cell is terminal

`out` shares one namespace with `cell`: declaring `out result := ...;` after (or before) a
`cell result` in the same sheet is a duplicate-name error, exactly like two `cell` declarations
would be. Unlike a plain cell, though, an output's cell can never be *written*: not by a host write, not
by a `relationship` binding, not by a second `out`. It's computed exactly once each time the
sheet resolves, by its own initializer, and nothing else:

```
{{#include examples/outputs/output_cell_is_terminal.adm2}}
```

An output cell is nonetheless an ordinary cell for *reading*: a later declaration in the same
sheet can reference an earlier `out` by name in its own expression, exactly like referencing
any other already-declared cell.

## 7.3 Requirements: diagnostics, not gates

A `require { ... }` block trailing an `out`'s initializer names zero or more boolean checks.
Each `requirement`'s own dependencies are deduced separately from the output's initializer; a
requirement commonly reads the output's own value by name, alongside whatever other cells it
needs:

```
{{#include examples/outputs/requirement_diagnostic.adm2}}
```

A failed requirement never stops the sheet from resolving, and never stops `area` from being
computed and readable; a host can query which requirements are currently failing precisely
because nothing else in the sheet notices a requirement failing on its own (see
[Appendix A.11](reference.md#a11-the-host-embedding-api)). A requirement's `name` is just a
label surfaced through that query; it happens to read naturally when it echoes a cell name
(`not_too_big`, `width_max`), but it isn't a cell reference and doesn't have to match one.

## 7.4 Multiple requirements

An output can list any number of requirements; a host can query exactly the ones currently
failing, identified individually (see [Appendix A.11](reference.md#a11-the-host-embedding-api)):

```
{{#include examples/outputs/multiple_requirements.adm2}}
```
