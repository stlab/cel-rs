# Chapter 1: A Tutorial Introduction

An Adam sheet declares the relationships among a set of properties (invariants in a document's
structure, constraints between a command's arguments and its result, or values useful for
constructing a new argument or document state), instead of the event logic that would otherwise
maintain them by hand.

Adam programs are called _sheets_, a term borrowed from spreasheets. This chapter is a fast,
informal tour of every construct Adam has; later chapters go back over the same ground in more
detail, and the [reference manual](reference.md) collects the precise rules for looking things up.

You don't need to install anything to follow along: every source fragment below stands on its own as
a `.adm2` file, and the UI controls are constructed entirely from the sheet declaration

## 1.1 A first sheet

An Adam program is a single `sheet`, named, with a body of declarations between braces. A simple sheet declars a couple of source cells:

```adam
{{#include examples/tutorial/first_sheet.adm2}}
```

A _cell_ is a named, typed storage location: the basic unit of state in a property model. `width`
and `height` are `i32`-typed cells, each given an initial value (the types are deduced from the
initial value). A `source` cell is like a spreadsheet's value cell: it holds a value written into it.

Semicolons end declarations, exactly as in Rust or C; a sheet's body is a sequence of declarations,
not a sequence of statements. A sheet describes a _graph_ of cells and the constraints between them. The graph for `hello` is just the two, unconnected, cells.

<graph sheet="first_sheet">

## 1.2 Filters

A `filter` clause attaches a standing domain constraint to a cell, most commonly a range:

```adam
{{#include examples/tutorial/clamp_demo.adm2}}
```

`0..=100` is an _inclusive_ range: both `0` and `100` are themselves valid values for `level`. Try
writing a value outside `[0, 100]` into `level` above and watch it snap back into range.

A filter's bounds don't have to be constants: `0..=max` references another cell, and the clamp
tracks it live. [Chapter 5](filters.md) covers filters in full.

## 1.3 Out Cells

An `out` declaration is like a spreadsheet's equation cell. Its value is computed from the provided _method_.

```adam
{{#include examples/tutorial/basic_output.adm2}}
```

The method on the out cell can reference other cells in the sheet and the calculation is reapplied when those values change. In the graph representation, the method is a relationship and drawn as a circle between the cells. The heavy arrows and border around the out cell indicate that the value is _forced_ by the relationship.

<graph sheet="basic_output">

See [Chapter 6](outputs.md) for the full treatment.

## 1.5 Cells and Relationships

A plain `cell` declaration acts as a source or derived cell. Cells are connected by one or more _relationship_ that is a bundle of methods that each satisfy the relationship but solve for a different term.

For example, if we have two values `a` and `b` where `a == 2b`, that can be represented as:

```adam
{{#include examples/tutorial/basic_relationship.adm2}}
```

For any active `relationship`, exactly one method is selected to execute. The method choosen is based on the _strength_ of the cells. Cells that have been written more recently have a higher strength. The initial strength of the cells is determined by the declaration order. Cells declared later have a higher strength.

In the graph, you can see the flow change as you write `a` or `b`.

<graph sheet="basic_relationship">

The methods in a relationship must be _consistant_. If the result of the selected methed is used to recalculate the non-selected methods, the result should not change the value of the assigned cells within an error epsilon.

Relationship can be chained together. We can express the relationship `a <= b <= c` like this:

```adam
{{#include examples/tutorial/inequality.adm2}}
```

<graph sheet="inequality">

This example also demonstrates two additional features.

- A method can be _self-referential_, naming a cell as both a dependency and a result. In such a case, the method must be idempotent.
- When a cell value is derived in terms of itself via a self-referential method (or filter). The last written value is preserved.

You can see the effect of the second behavior by sliding `a` to `100` which will pull `b` and `c` to `100` and then slide `a` back to `0`. `b` and `c` will return to their prior values.

> _Note: There is an [open issue](https://github.com/stlab/cel-rs/issues/182) with this example that is being actively worked on._

In [Chapter 7](relationships.md) you will see relationships are not limited in their arity (you can have n-way relationships with each method solving for 1 or more cells).

### 1.5.1 Two structural rules on a relationship's methods

Every relationship's methods must satisfy two structural rules, checked once, when the sheet is
parsed, well before it's ever resolved:

- Every method's `inputs ∪ outputs` must be exactly the same set of cells as every other method's in
  the same relationship. Violating this is `Error::MismatchedMethodCells`: "methods in a
  relationship must reference the same set of cells".
- No two methods in the same relationship may share an identical output set. Violating this is
  `Error::DuplicateMethodOutputs`: "a method's outputs must be duplicate-free, and no two methods in
  a relationship may share an outputs set".

`multiplication_triangle.adm2`'s own three bindings satisfy both. `c := a * b` reads `a` and `b` and
writes `c`; `a := c / b` reads `c` and `b` and writes `a`; `b := c / a` reads `c` and `a` and writes
`b`; every method's `inputs ∪ outputs` comes out to the same set, `{a, b, c}`. And the three output
sets, `{c}`, `{a}`, and `{b}`, are pairwise distinct.

## 1.6 Relationships continued: destructuring and self-reference

A binding's left-hand side can name more than one output cell by parenthesizing it, splitting a
tuple-valued expression on the right into its parts, one cell per element, using the same `(a, b)`
syntax Rust uses for tuple patterns:

```adam
{{#include examples/tutorial/destructuring_demo.adm2}}
```

Tuple _types_ (`cell point: (f64, f64) = (0.0, 0.0);`) are a CEL feature, documented in [Chapter
2](cells.md); destructuring is the relationship-binding syntax built on top of them, and could one
day extend to struct patterns too. See [Chapter 8,
§8.1](relationships-continued.md#81-destructuring-bindings) for the full
destructuring-vs-direct-bind distinction.

A binding may also name the same cell on both sides of `:=`: a _self-referencing method_, deriving a
cell's own next value from its own current one. [Chapter 8](relationships-continued.md) walks
through a full worked example with its own `self_referencing_method.adm2`, rather than repeating one
here; §1.7 below shows the same pattern once more, inside a conditional branch. The obligation on a
self-referencing method is stricter than an ordinary one: the method's own job is to correct a value
into whatever set the relationship enforces, and if reapplying it to its own already-corrected
output would change the value again, the "correction" was never well-defined in the first place. The
solver never checks this; it's on the sheet author.

## 1.7 Conditionals

A `conditional` groups relationships that are only active under a matching condition. It evaluates a
_match subject_, then activates whichever branch's literal equals the current match value:

```adam
{{#include examples/tutorial/mode_demo.adm2}}
```

Only the active branch's relationships participate in that round's solve; every other branch's
relationships are as if they weren't declared at all. The `_` branch, if present, catches any value
none of the named branches list, and must be written last.

See [Chapter 9](conditionals.md) for branch types, tuple match subjects, and what happens when no
branch matches and there's no default.

Some branches offer the solver no choice at all. A relationship with exactly one method is _forced_:
there's no alternative binding to try, so its output cell is claimed every round regardless of
strength, unlike the freely-chosen roles in §1.5's triangle. A host UI commonly disables the
editable widget for a forced cell, since writing it would have no lasting effect once the sheet
re-resolves.

```adam
{{#include examples/tutorial/forced_and_self_ref_shadow.adm2}}
```

With `mode == 0` (the declared default), `range_bounds`'s relationship has two self-referencing
methods, `low := min(low, high)` and `high := max(low, high)`: a relationship exactly like §1.5's
triangle, where either cell could end up derived, decided by strength, and never both at once.
Declared first, `low` is staler, so the solver derives it: `low := min(4, 9)`, which happens to
equal `low`'s own current value, so nothing visibly changes. `high`'s own `max` method is never
invoked at all this round; `high` is simply an ordinary, unclaimed source, reporting its own
untouched value, `9`.

Writing `high` to `42` and switching to `mode == 1` activates a relationship with a single method,
`low := high`: forced. `low` is claimed every round this branch is active, so it now reads back
`42`, `high`'s current value, no matter what strength would otherwise prefer. But `low`'s own
underlying raw value, its _source_, is untouched by any of this: it's still `4`, exactly where it
started, _shadowed_ by the forced derived value the same way a filter's correction shadows a cell's
raw value in §1.2; a derived value never destroys the source underneath it, whichever mechanism
produced that derived value.

Switching back to `mode == 0` reactivates the two-method relationship, and strength has changed in
the meantime: `high` was just written, so it's freshest now, and `low`, never itself explicitly
written, is stalest, so the solver again derives `low`. It derives it from each cell's own _source_,
not from the stale forced value `low` was shadowing a moment ago: `low := min(4, 42)`, using `low`'s
untouched source `4` and `high`'s actual current value `42`, giving `low = 4` and leaving `high =
42` alone as a source. The `42` `low` displayed while forced belonged to the now-inactive `mode ==
1` relationship, and simply stopped existing the moment that relationship stopped being selected.

Writing `low` to `100` promotes it to freshest, flipping the two-method relationship's choice: now
`high := max(low, high)` is the one selected instead, deriving `high` and leaving `low` as the
source: `high` reads `100`, pulled up to match. Either binding can fire; which one does is
strength's call, never both at once.

Adam's comment and doc-comment syntax is covered in [Chapter 10](lexical-conventions.md), not here.

## 1.4 Requirements

An `out` declaration (or a `cell`, or a `source`) can carry named `require`ments: boolean checks
re-evaluated and reported each time the sheet resolves, never enforced by rejecting a write or
blocking resolution:

```adam
{{#include examples/tutorial/area_with_requirement.adm2}}
```

A failed requirement never stops the sheet from resolving, and never stops `area` from being
computed and readable: it's a diagnostic, not a gate, exactly the way §1.2's filter corrects a value
rather than rejecting it. A host queries which requirements are currently failing after each
resolve.

Two facts here generalize past this one example. `require` isn't limited to `out`: the same block
can trail a `source` declaration too; see [§2.2](cells.md#22-cell-declarations) and [Chapter
3](source.md). And `filter` isn't limited to plain cells, either: the same clause can trail an `out`
declaration; see [Chapter 5, §5.6](filters.md#56-a-filter-on-an-output-cell). See [Chapter 6,
§6.3](outputs.md#63-requirements-diagnostics-not-gates) for the full rules governing requirements.

## 1.8 Where to go next

That's the whole language. [Chapter 2](cells.md) onward covers each construct in the depth this
chapter skipped past, and the [reference manual](reference.md) gives you the full grammar and every
built-in type in one place.
