# A Tutorial Introduction

Adam programs are called *sheets*, a term borrowed from spreadsheets.
This tutorial introduces the language through live examples.

An Adam sheet declares the relationships among a set of properties called _cells_. These
relationships determine how edits propagate through a user interface or scripting system.

Each source fragment defines the cells and relationships shown in its live inspector and graph.
Edit values to explore how they propagate. Applications typically bind a sheet to controls
or scripts with human-readable labels rather than cell identifiers.

## A first sheet

The first example defines one named `sheet`, with declarations between braces.
Modules may contain multiple definitions and imports. This sheet declares two source cells:

```adam
{{#include examples/tutorial/first_sheet.adm2}}
```

A *cell* is a named, typed storage location. Adam infers `width` and `height` as `i32`
from their initial values. A `source` cell holds a value supplied from outside the sheet,
like a spreadsheet's value cell.

A sheet describes a *graph* of cells and constraints. The graph for `hello` has two
unconnected cells.

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

The output's method can read other cells; resolution updates its result when those inputs change.
The graph draws the relationship as a circle between cells.
Heavy arrows and the output cell's border mark a *forced* value.
The live inspector disables editing for forced cells.

<graph sheet="basic_output">

See the [outputs chapter](outputs.md) for the full treatment.

## Cells and Relationships

A plain `cell` may be a source or a derived output.
*Relationships* connect cells through alternative methods, each solving for a different
cell or set of cells.

This relationship maintains \\(a = 2b\\):

```adam
{{#include examples/tutorial/basic_relationship.adm2}}
```

Resolution selects exactly one method per active relationship, based on cell *strength*.
Cells written more recently have higher strength.
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

The methods here are *self-referencing*: each reads a cell it also writes.
Such a method must be *idempotent*, leaving its corrected output unchanged when reapplied.
Self-referencing methods and filters preserve the cell's last-written source value.

Slide `a` to `100` to pull `b` and `c` to `100`, then slide `a` back to `0`.
`b` and `c` return to their prior values.

Relationships may connect any number of cells, with each method solving for one or more;
see the [relationships chapter](relationships.md).

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

A conditional relationship may force a cell while preserving its source value.
Releasing the relationship restores that source. Host UIs commonly disable controls
for forced cells.

```adam
{{#include examples/tutorial/conditional_forced.adm2}}
```

## Requirements

A filter on a derived cell reports a violation without changing the value:

```adam
{{#include examples/tutorial/requirements_filter_diagnostic.adm2}}
```

A cell may also have one or more requirements. Each requirement is a Boolean diagnostic,
not an input gate: a source or writable cell retains a value that violates a requirement,
and propagation reports the requirement's failed validity.

The same rule applies to an out cell. Requirements have an optional name that can be
associated with a message explaining the issue.

```adam
{{#include examples/tutorial/area_with_requirement.adm2}}
```

<!-- [knowledge]
Keep relationship validation and destructuring details in relationships.md and
relationships-continued.md rather than duplicating them in this tutorial.
The tutorial's fail_mismatched_cells, fail_overlapping_outputs, fail_contradiction,
and destructuring_demo fixtures remain available for tests.
-->

## Where to go next

Continue with [sheets, cells, and types](cells.md) for declaration and type rules,
or use the [reference manual](reference.md) to look up grammar, built-in types, and errors.
