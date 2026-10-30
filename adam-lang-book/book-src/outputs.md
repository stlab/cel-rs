# Outputs and Requirements

## Grammar

```text
out_decl    = "out" identifier [ ":" type_expr ] ":=" expression
               [ cell_filter ] [ "require" "{" { requirement } "}" ] ";".
requirement = [ "@" identifier ] expression ";".
```

An `out` declares a cell computed by one expression, with
[deduced dependencies](expressions.md#deduced-dependencies).
The optional `: type_expr` annotation must match the expression's type; without it,
Adam infers the type. An output may also carry a [`filter`](filters.md) clause.

```adam
{{#include examples/outputs/basic_output.adm2}}
```

## An output cell can be read anywhere, written nowhere

An output shares the cell namespace and must have a unique name.
Later declarations may read it as a relationship input, conditional subject, filter
argument, or another output's input. Only its own expression may write it:
host writes, relationship bindings, and additional `out` declarations cannot.
The expression is purely functional; resolution may reuse its result:

```adam
{{#include examples/outputs/output_cell_can_be_referenced.adm2}}
```

## Requirements: diagnostics, not gates

A `require { ... }` block attaches zero or more boolean checks to a `cell`,
[`source`](source.md), or `out` declaration, after any filter.
Each *requirement* deduces its dependencies separately from the initializer.
It may read the cell it checks and any other declared cells:

```adam
{{#include examples/outputs/requirement_diagnostic.adm2}}
```

Requirements are purely functional predicates of their input values and immutable
constants, with no externally observable side effects or mutable external dependencies.
Equal inputs produce equivalent results or errors. There is no guarantee that a
requirement executes, or when or how often it executes; current diagnostics may use
a previously computed result.

A failed requirement does not stop resolution or prevent a cell's value from being
computed and read. Hosts can query failures through the
[embedding API](reference.md#the-host-embedding-api).
The optional `@name` identifies a requirement for reporting; it is not a cell reference.

## Multiple requirements

A declaration can list any number of requirements, which hosts can query individually:

```adam
{{#include examples/outputs/multiple_requirements.adm2}}
```
