# Chapter 8: Outputs and Requirements

## 8.1 Grammar

```text
out_decl    = "out" identifier [ ":" type_expr ] ":=" expression
               [ cell_filter ] [ "require" "{" { requirement } "}" ] ";".
requirement = identifier ":" expression ";".
```

An `out` declares a new cell computed by exactly one expression: there's no alternative
binding to choose between, unlike a `relationship`. Its dependencies are
[deduced](expressions.md#44-deduced-dependencies) the same way. The `: type_expr` annotation is
optional; when absent, the output's type is inferred from the initializer, the same rule
[Chapter 2](cells.md#23-built-in-types-and-inference) gives for a plain `cell`. An `out` may
also carry a `filter` clause, exactly like a plain `cell`; see [Chapter 7](filters.md).

```adam
{{#include examples/outputs/basic_output.adm2}}
```

## 8.2 An output cell can be read anywhere, written nowhere

`out` shares one namespace with `cell`: declaring `out result := ...;` after (or before) a
`cell result` in the same sheet is a duplicate-name error, exactly like two `cell` declarations
would be. An output cell is an ordinary cell for *reading*: a later declaration in the same
sheet can reference an earlier `out` by name in its own expression — as a relationship input, a
conditional's match subject, a filter argument, or another `out`'s own initializer — exactly
like referencing any other already-declared cell. What stays restricted is *writing*: an
output's cell can never be produced by more than one method, and can never be written directly,
not by a host write, not by a `relationship` binding, not by a second `out`. It's computed
exactly once each time the sheet resolves, by its own initializer, and nothing else:

```adam
{{#include examples/outputs/output_cell_can_be_referenced.adm2}}
```

## 8.3 Requirements: diagnostics, not gates

`require` is not an outputs-only mechanism: a `require { ... }` block can trail a plain `cell`
or a [`source`](source.md) declaration's initializer too, with the same meaning described here
— see [Chapter 2](cells.md#22-cell-declarations) and [Chapter 3](source.md) for those forms.
Trailing an `out`'s initializer, `require { ... }` names zero or more boolean checks. Each
`requirement`'s own dependencies are deduced separately from the output's initializer; a
requirement commonly reads the output's own value by name, alongside whatever other cells it
needs:

```adam
{{#include examples/outputs/requirement_diagnostic.adm2}}
```

A failed requirement never stops the sheet from resolving, and never stops `area` from being
computed and readable; a host can query which requirements are currently failing precisely
because nothing else in the sheet notices a requirement failing on its own (see
[Appendix A.11](reference.md#a11-the-host-embedding-api)). A requirement's `name` is just a
label surfaced through that query; it happens to read naturally when it echoes a cell name
(`not_too_big`, `width_max`), but it isn't a cell reference and doesn't have to match one.

## 8.4 Multiple requirements

An output can list any number of requirements; a host can query exactly the ones currently
failing, identified individually (see [Appendix A.11](reference.md#a11-the-host-embedding-api)):

```adam
{{#include examples/outputs/multiple_requirements.adm2}}
```
