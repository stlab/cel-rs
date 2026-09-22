# A Tutorial Introduction

Adam programs are called _sheets_, a term borrowed from spreadsheets. This chapter is an informal
tour of Adam; later chapters go over the same ground in more detail, and the [reference
manual](reference.md) provides a full specification.

An Adam sheet declares the relationships among a set of properties called _cells_. These
relationships are maintained when edits occur, providing correct behaviors for user interfaces and
scripting without having to code complex event handling logic.

Every source fragment below is the complete description used to generate the UI, UI behavior, and
graph visualization. The UI is live, so you can change the values and explore the behavior. Within
an application, a sheet instance is typically bound to a UI construct or scripting system with human-
readable text labels, instead of cell identifiers.

## A first sheet

An Adam program is a single `sheet`, named, with a body of declarations between braces. A simple
sheet declares a couple of source cells:

```adam
{{#include examples/tutorial/first_sheet.adm2}}
```

A _cell_ is a named, typed storage location: the basic unit of state in a property model. `width`
and `height` are `i32`-typed cells, each given an initial value (the types are deduced from the
initial value). A `source` cell is like a spreadsheet's value cell: it holds a value written into
it.

A sheet's body is a sequence of declarations. A sheet describes a _graph_ of cells and the
constraints between them. The graph for `hello` is just the two unconnected cells.

<graph sheet="first_sheet">

## Filters

A `filter` clause attaches a domain constraint to a cell, most commonly a range:

```adam
{{#include examples/tutorial/clamp_demo.adm2}}
```

`0..=100` is an _inclusive_ range. Try writing a value outside `[0, 100]` into `level` above and
watch it snap back into range.

A filter's bounds don't have to be constants: `0..=max` references another cell. The [filters
chapter](filters.md) covers filters in full.

## Out Cells

An `out` declaration is like a spreadsheet's equation cell. Its value is computed from the provided
_method_.

```adam
{{#include examples/tutorial/basic_output.adm2}}
```

The method on the out cell can reference other cells in the sheet, and the calculation is reapplied
when those values change. In the graph representation, the method is a relationship and drawn as a
circle between the cells. The heavy arrows and border around the out cell indicate that the value is
_forced_ by the relationship. A forced cell's value is not editable.

<graph sheet="basic_output">

See the [outputs chapter](outputs.md) for the full treatment.

## Cells and Relationships

A plain `cell` declaration acts as a source _or_ out cell. Cells are connected by one or more
_relationships_, each a bundle of methods that satisfy the relationship but solve for a different
term (or set of terms).

For example, if we have two values \\(a\\) and \\(b\\) where \\(a = 2b\\), that can be represented
as:

```adam
{{#include examples/tutorial/basic_relationship.adm2}}
```

For any active `relationship`, exactly one method is selected to execute. The method chosen is based
on the _strength_ of the cells. Cells that have been written more recently have a higher strength.
The declaration order determines the cells' initial strength. Cells declared later have
a higher strength.

In the graph, you can see the flow change as you write `a` or `b`.

<graph sheet="basic_relationship">

The methods in a relationship must be _consistent_. If the result of the selected method is used to
recalculate the non-selected methods, the result should not change the value of the assigned cells
within an error epsilon.

Relationships can be chained together. We can express the relationship `a <= b <= c` like this:

```adam
{{#include examples/tutorial/inequality.adm2}}
```

<graph sheet="inequality">

This example also demonstrates two additional features.

- A method can be _self-referential_, naming a cell as both a dependency and a result. In such a
  case, the method must be idempotent.
- When a cell value is derived in terms of itself via a self-referential method (or filter). The
  last written value is preserved.

You can see the effect of the second behavior by sliding `a` to `100` which will pull `b` and `c` to
`100` and then slide `a` back to `0`. `b` and `c` will return to their prior values.

In the [relationships chapter](relationships.md), you will see relationships are not limited in their
arity (you can have n-way relationships with each method solving for 1 or more cells).

## Conditionals

A `conditional` groups relationships that are only active under a matching condition. It evaluates a
_match subject_, then activates whichever branch's literal equals the current match value:

```adam
{{#include examples/tutorial/constrain.adm2}}
```

Only the active branch participates. The `_` branch, if present, catches any value
not in the named branches list, and must be written last. If no branch is matched, the conditional
has no effect.

<graph sheet="constrain">

A conditional relationship may force a cell value, in this case any source cell value is preserved
and restored when the conditional is removed. The UI for a forced cell value will typically disable
the control.

```adam
{{#include examples/tutorial/conditional_forced.adm2}}
```

## Requirements

Filters on source cells were introduced in the above [[Filters]] section. For an out cell or a cell
in an out role, violating a filter will trigger a diagnostic.

```adam
{{#include examples/tutorial/requirements_filter_diagnostic.adm2}}
```

A cell may also have one or more requirements. For source cell or when a cell is writable, the requirements act are a boolean expression that act as a filter rejecting any input that doesn't satisfy the requirements.

For an out cell, violating a requirement will trigger a diagnostic. Requirements have an optional name that can be associated with a message explaining the issue.

```adam
{{#include examples/tutorial/area_with_requirement.adm2}}
```

<!-- This section disabled - it will be relocated to another chapter.

### Relationship Rules

Every relationship's methods must satisfy a set of rules:

- Every method's \\(inputs \cup outputs\\) must be the same set of cells as every other
  method's in the same relationship.

```adam
{{#include examples/tutorial/fail_mismatched_cells.adm2}}
```

> Note: The incorrect span in the error reporting is actively being fixed.

- No two methods in the same relationship may share an identical output set.

```adam
{{#include examples/tutorial/fail_overlapping_outputs.adm2}}
```

- A cell can appear as the output of at most one selected method.

```adam
{{#include examples/tutorial/fail_contradiction.adm2}}
```

## Relationships continued: destructuring and self-reference

A binding's left-hand side can name more than one output cell by parenthesizing it, splitting a
tuple-valued expression on the right into its parts, one cell per element, using the same `(a, b)`
syntax Rust uses for tuple patterns:

```adam
{{#include examples/tutorial/destructuring_demo.adm2}}
```

Tuple _types_ (`cell point: (f64, f64) = (0.0, 0.0);`) are a CEL feature, documented in the [types
chapter](cells.md); destructuring is the relationship-binding syntax built on top of them, and could
one day extend to struct patterns too. See [destructuring
bindings](relationships-continued.md#destructuring-bindings) for the full
destructuring-vs-direct-bind distinction.

A binding may also name the same cell on both sides of `:=`: a _self-referencing method_, deriving a
cell's own next value from its own current one. The [relationships-continued
chapter](relationships-continued.md) walks through a full worked example with its own
`self_referencing_method.adm2`, rather than repeating one here; the [Conditionals](#conditionals)
section below shows the same pattern once more, inside a conditional branch. The obligation on a
self-referencing method is stricter than an ordinary one: the method's own job is to correct a value
into whatever set the relationship enforces, and if reapplying it to its own already-corrected
output would change the value again, the "correction" was never well-defined in the first place. The
solver never checks this; it's on the sheet author.

-->

## Where to go next

That's the whole language. The remaining chapters, starting with [sheets, cells, and
types](cells.md), cover each construct in the depth this chapter skipped past, and the [reference
manual](reference.md) gives you the full grammar and every built-in type in one place.
