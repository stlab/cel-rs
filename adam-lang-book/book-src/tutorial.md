# Chapter 1: A Tutorial Introduction

An Adam sheet declares the relationships among a set of properties (invariants in a
document's structure, constraints between a command's arguments and its result, or values
useful for constructing a new argument or document state), instead of the event logic that
would otherwise maintain them by hand.

Let's get started. The best way to learn a new language is to write programs in it, and
Adam programs are called **sheets**. This chapter is a fast, informal tour of every
construct Adam has; later chapters go back over the same ground in more detail, and the
[reference manual](reference.md) collects the precise rules for looking things up.

You don't need to install anything to follow along: every source fragment below stands on its
own as a `.adm2` file, included directly from a test that also exercises it, so a chapter's
prose can never drift from code that actually parses and runs.

## 1.1 A first sheet

An Adam program is a single `sheet`, named, with a body of declarations between braces.
The simplest useful sheet declares a couple of inputs and nothing else:

```
{{#include examples/tutorial/first_sheet.adm2}}
```

A **cell** is a named, typed storage location: the basic unit of state in a property model.
`width` and `height` are `i32`-typed cells, each given an initial value, but notice the
keyword: `source`, not `cell`. A `source` cell is like a spreadsheet's value cell: a slot you
type a number into directly, with nothing else in the sheet computing it for you. Syntactically,
a `source` declaration looks exactly like a `cell` declaration: the same type, the same
`= initializer`, the same trailing semicolon. What sets it apart from a plain `cell` is a fact
about behavior, not spelling: a `source` cell is always an input, never a value some
relationship computes on your behalf. That distinction has no visible effect yet (there's
nothing here to compute `width` or `height` from), but it starts to matter the moment
relationships enter the picture, in [§1.5](#15-relationships-a-cell-that-can-be-either-a-source-or-derived)
below, where a plain `cell`'s role is decided fresh every time the sheet resolves and a
`source` cell's is not.

Semicolons end declarations, exactly as in Rust or C; a sheet's body is a sequence of
declarations, not a sequence of statements: there is no control flow at this level, no loops,
and no imperative execution order. A sheet describes a *graph* of cells and the constraints
between them, not a sequence of steps to run.

Parsing this text and reading or writing its cells is a Rust-level embedding concern, not
something a sheet author does; see [Appendix A.10](reference.md#a10-the-host-embedding-api)
for how a host application actually drives a parsed sheet.

## 1.2 Filters: self-correcting cells

A `filter` clause attaches a standing domain constraint to a cell, most commonly a range:

```
{{#include examples/tutorial/clamp_demo.adm2}}
```

`0..=100` is an **inclusive** range: both `0` and `100` are themselves valid values for
`level`, and only something outside that closed interval ever gets corrected. A host UI
commonly mounts a filtered cell like this one as a live, editable widget. If this page is
rendered live for you, try writing a value outside `[0, 100]` into `level` above and watch it
snap back into range, but not instantly: the correction only takes effect the next time the
sheet resolves, never at the moment of the write itself.

Write an out-of-range value and the cell keeps it, raw: a filter never inspects or blocks the
value at the moment it's written. The clamp only takes effect the next time the sheet resolves,
and it's that corrected value every read of the cell sees from then on. The raw value you
actually wrote is never lost, either: the filter's correction lands in a separate, computed
slot alongside it, so loosening the bound later snaps the cell straight back to what you
actually last wrote, not to some intermediate clamped value.

A filter's bounds don't have to be constants: `0..=max` references another cell, and the clamp
tracks it live. [Chapter 5](filters.md) covers filters in full, including the precise
source/derived model behind "the cell keeps its own raw value forever, and the filter only ever
corrects what you *read*," and how the same `filter` clause also attaches to an `out`
declaration.

## 1.3 Outputs: read-only, computed cells

An `out` declaration computes one final, read-only value from the rest of the sheet. An `out`
cell is like a spreadsheet's equation cell: you never type into it directly, and its value is
always whatever its formula currently computes.

Two rules hold without exception. First, nothing may ever write an `out` cell directly: not a
host write, not a `relationship` binding, not even another `out`'s own initializer; an output
has exactly one writer, itself, fixed forever at the point it's declared. Second, an `out` is
recomputed exactly once every time the sheet resolves, from its own initializer, using whatever
the rest of the sheet's cells currently hold.

[§1.4](#14-requirements) below puts an `out` to work in a worked example, once there's a
`require` block worth attaching to one. See [Chapter 6](outputs.md) for the full treatment.

## 1.4 Requirements

An `out` declaration (or a `cell`, or a `source`) can carry named `require`ments: boolean
checks re-evaluated and reported each time the sheet resolves, never enforced by rejecting a
write or blocking resolution:

```
{{#include examples/tutorial/area_with_requirement.adm2}}
```

A failed requirement never stops the sheet from resolving, and never stops `area` from being
computed and readable: it's a diagnostic, not a gate, exactly the way §1.2's filter corrects a
value rather than rejecting it. A host queries which requirements are currently failing after
each resolve.

Two facts here generalize past this one example. `require` isn't limited to `out`: the same
block can trail a `source` declaration too; see [§2.2](cells.md#22-cell-declarations) and
[Chapter 3](source.md). And `filter` isn't limited to plain cells, either: the same clause can
trail an `out` declaration; see [Chapter 5, §5.6](filters.md#56-a-filter-on-an-output-cell). See
[Chapter 6, §6.3](outputs.md#63-requirements-diagnostics-not-gates) for the full rules governing
requirements.

## 1.5 Relationships: a cell that can be either a source or derived

A sheet with only `source`/`cell` declarations and no relationships is just a struct. What
makes Adam interesting is the **relationship**: a set of alternative ways to keep a group of
cells consistent, any one of which the solver may pick at any given moment. A `relationship`
binding can never derive a [`source`](source.md) cell: that's the one kind of cell always left
alone as a source, unconditionally; more on that in [Chapter 3](source.md).

The classic example is three numbers related by multiplication (`a * b = c`), where any one of
the three can be computed from the other two, the same shape as `pixels == inches * resolution`
from [the introduction](intro.md#why-adam). As a sheet:

```
{{#include examples/tutorial/multiplication_triangle.adm2}}
```

The `relationship` block offers three **bindings**: `c := a * b`, `a := c / b`, and
`b := c / a`, each an alternative *method* for deriving one cell from the others. Only one
binding is active at a time. Unlike `source` and `out`, a plain `cell` inside a relationship
isn't fixed as a source or an output the way those two are: which role a given cell plays is
decided fresh every time the sheet resolves, driven by **strength**.

Every cell carries a strength, a write-recency counter. A cell's own *declaration* counts as a
write for this purpose, so before anything is ever explicitly written, declaration order alone
breaks the tie: cells declared earlier are staler than cells declared later. The solver prefers
to leave the freshest cells alone and derive the stalest one: here, `c`, declared first. Writing
a cell promotes it to freshest of all, which is what tells the solver "trust this one; recompute
something else instead." See [Chapter 7](relationships.md) for strength's full treatment,
including what happens when a cell is shared across more than one relationship, when no valid
assignment exists, and cycles.

### 1.5.1 Two structural rules on a relationship's methods

Every relationship's methods must satisfy two structural rules, checked once, when the sheet is
parsed, well before it's ever resolved:

- Every method's `inputs ∪ outputs` must be exactly the same set of cells as every other
  method's in the same relationship. Violating this is `Error::MismatchedMethodCells`:
  "methods in a relationship must reference the same set of cells".
- No two methods in the same relationship may share an identical output set. Violating this is
  `Error::DuplicateMethodOutputs`: "a method's outputs must be duplicate-free, and no two
  methods in a relationship may share an outputs set".

`multiplication_triangle.adm2`'s own three bindings satisfy both. `c := a * b` reads `a` and
`b` and writes `c`; `a := c / b` reads `c` and `b` and writes `a`; `b := c / a` reads `c` and
`a` and writes `b`; every method's `inputs ∪ outputs` comes out to the same set, `{a, b, c}`.
And the three output sets, `{c}`, `{a}`, and `{b}`, are pairwise distinct.

## 1.6 Relationships continued: destructuring and self-reference

A binding's left-hand side can name more than one output cell by parenthesizing it, splitting a
tuple-valued expression on the right into its parts, one cell per element, using the same
`(a, b)` syntax Rust uses for tuple patterns:

```
{{#include examples/tutorial/destructuring_demo.adm2}}
```

Tuple *types* (`cell point: (f64, f64) = (0.0, 0.0);`) are a CEL feature, documented in
[Chapter 2](cells.md); destructuring is the relationship-binding syntax built on top of them,
and could one day extend to struct patterns too. See
[Chapter 8, §8.1](relationships-continued.md#81-destructuring-bindings) for the full
destructuring-vs-direct-bind distinction.

A binding may also name the same cell on both sides of `:=`: a **self-referencing method**,
deriving a cell's own next value from its own current one. [Chapter 8](relationships-continued.md)
walks through a full worked example with its own `self_referencing_method.adm2`, rather than
repeating one here; §1.7 below shows the same pattern once more, inside a conditional branch.
The obligation on a self-referencing method is stricter than an ordinary one: the method's own
job is to correct a value into whatever set the relationship enforces, and if reapplying it to
its own already-corrected output would change the value again, the "correction" was never
well-defined in the first place. The solver never checks this; it's on the sheet author.

## 1.7 Conditionals

A `conditional` groups relationships that are only active under a matching condition. It
evaluates a **match subject**, then activates whichever branch's literal equals the current
match value:

```
{{#include examples/tutorial/mode_demo.adm2}}
```

Only the active branch's relationships participate in that round's solve; every other branch's
relationships are as if they weren't declared at all. The `_` branch, if present, catches any
value none of the named branches list, and must be written last.

See [Chapter 9](conditionals.md) for branch types, tuple match subjects, and what happens when
no branch matches and there's no default.

Some branches offer the solver no choice at all. A relationship with exactly one method is
**forced**: there's no alternative binding to try, so its output cell is claimed every round
regardless of strength, unlike the freely-chosen roles in §1.5's triangle. A host
UI commonly disables the editable widget for a forced cell, since writing it would have no
lasting effect once the sheet re-resolves.

```
{{#include examples/tutorial/forced_and_self_ref_shadow.adm2}}
```

With `mode == 0` (the declared default), `range_bounds`'s relationship has two self-referencing
methods, `low := min(low, high)` and `high := max(low, high)`: a relationship exactly like
§1.5's triangle, where either cell could end up derived, decided by strength, and never both at
once. Declared first, `low` is staler, so the solver derives it: `low := min(4, 9)`, which
happens to equal `low`'s own current value, so nothing visibly changes. `high`'s own `max`
method is never invoked at all this round; `high` is simply an ordinary, unclaimed source,
reporting its own untouched value, `9`.

Writing `high` to `42` and switching to `mode == 1` activates a relationship with a single
method, `low := high`: forced. `low` is claimed every round this branch is active, so it now
reads back `42`, `high`'s current value, no matter what strength would otherwise prefer. But
`low`'s own underlying raw value, its **source**, is untouched by any of this: it's still `4`,
exactly where it started, *shadowed* by the forced derived value the same way a filter's
correction shadows a cell's raw value in §1.2; a derived value never destroys the source
underneath it, whichever mechanism produced that derived value.

Switching back to `mode == 0` reactivates the two-method relationship, and strength has changed
in the meantime: `high` was just written, so it's freshest now, and `low`, never itself
explicitly written, is stalest, so the solver again derives `low`. It derives it
from each cell's own **source**, not from the stale forced value `low` was shadowing a moment
ago: `low := min(4, 42)`, using `low`'s untouched source `4` and `high`'s actual current value
`42`, giving `low = 4` and leaving `high = 42` alone as a source. The `42` `low` displayed while
forced belonged to the now-inactive `mode == 1` relationship, and simply stopped existing the
moment that relationship stopped being selected.

Writing `low` to `100` promotes it to freshest, flipping the two-method relationship's choice:
now `high := max(low, high)` is the one selected instead, deriving `high` and leaving `low` as
the source: `high` reads `100`, pulled up to match. Either binding can fire; which one does is
strength's call, never both at once.

Adam's comment and doc-comment syntax is covered in [Chapter 10](lexical-conventions.md), not
here.

## 1.8 Where to go next

That's the whole language. [Chapter 2](cells.md) onward covers each construct in the depth this
chapter skipped past, and the [reference manual](reference.md) gives you the full grammar and
every built-in type in one place.
