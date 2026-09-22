# Filters — Self-Correcting Cells

## Grammar

```text
cell_filter = "filter" expression.
```

A `filter` clause is optional and trails a `cell`, [`source`](source.md), or
[`out`](outputs.md) declaration's type/initializer. Its `expression` is
[deduced](expressions.md#deduced-dependencies) exactly like a relationship binding's, plus
one reserved identifier: `_` always refers to the *candidate value being conformed* (of the
filtered cell's own declared type), never a cell. `_` is reserved inside a filter expression
only; outside one it's an ordinary identifier (or the [conditional](conditionals.md)
default-branch token). A filter carries no name — a cell can have at most one, so nothing
ever needs to distinguish "which filter."

```adam
cell level: i32 = 50 filter 0..=100;             // a fixed range
cell level: i32 = 50 filter 0..=max;             // upper bound is another cell
cell level: i32 = 50 filter clamp(_, 0, max);    // an arbitrary expression over `_`
```

A filter expression must reference `_` at least once (unless it's a range expression, see
[range filters](#range-filters)) and must produce a value of exactly the filtered cell's own type; violating either is a
parse-time error, not a runtime one.

## Writing never filters

This is the single most important rule in this chapter: **writing a cell always stores exactly
the value it was given**, filter or no filter. A filter is applied live, when the sheet
resolves, against the cell's own current value, never synchronously at the moment of the write.

```adam
{{#include examples/filters/write_never_filters.adm2}}
```

Writing a filtered cell never fails because of the filter, and never reports an error on the
filter's account. Whatever you write is exactly what a read shows until the sheet next
resolves: the same "a read reflects the last full resolution, not a per-write side effect" rule
every other cell in a sheet already follows.

## The raw value is never lost

A filtered cell keeps two values: its raw last-written value, the _source_,
and, when something currently claims it, a computed override, the _derived_ value. Reading
the cell always returns the derived value if one is present, the source value otherwise. A
filter's live output always lands in the derived value, **never** in the source, so a filtered
cell's original input is never destroyed, even after many rounds of clamping. If a dynamic
bound loosens back up, the cell springs back to exactly what was last written, not to some
intermediate clamped value:

```adam
{{#include examples/filters/raw_value_never_lost.adm2}}
```

This is the same rule the [default branch and reverting to source](conditionals.md#the-default-branch-and-reverting-to-source) section already showed for a
relationship's method: a method's output (and a filter's output) always lands in the derived
value, so nothing a *computation* produces can ever permanently overwrite what was actually
written.

## Range filters

A filter expression whose type is CEL's `lo..=hi` range (over any type this book's
[built-in numeric types](cells.md#built-in-types-and-inference) supports) is recognized
structurally as a _range filter_: resolving the sheet clamps into `[lo, hi]` instead of
running the expression as an arbitrary function of `_`, and the sheet can report the range's
current live bounds without needing a candidate value at all:

```adam
{{#include examples/filters/range_filter_kind.adm2}}
```

A range filter's body is exempt from the "must reference `_`" rule stated in the [Grammar](#grammar) above: a genuine range
expression like `0..=max` has no reason to mention `_` at all, since both endpoints are
independent of the value being conformed. This book's own live examples mount an editable
widget bound to the filtered cell whose displayed `min`/`max` track the range's current live
bounds — try the example in the [Filters](tutorial.md#filters) section of the
tutorial.

## Derived cells: diagnosed, never corrected

A filter attaches to *one* cell, but that cell isn't always a source: a relationship may claim
it instead (see the [relationships chapter](relationships.md)). When that happens, the filter no longer has any authority to change the
value: it only *observes*. The sheet still resolves successfully, and the out-of-range value is
still what a read returns; the sheet simply records that the filter is violated:

```adam
{{#include examples/filters/derived_cell_diagnosed_not_corrected.adm2}}
```

Resolving the sheet never fails because of a filter violation, on either side (source or
derived): a filter is a diagnostic and a self-correction mechanism, never a gate. A host UI can
query which cells currently have a violated filter; see the
[host embedding API](reference.md#the-host-embedding-api) that exposes
this.

## A filter on an output cell

A filter isn't limited to a plain `cell`: an [`out`](outputs.md) declaration's grammar carries
the same optional `cell_filter` clause (see [Grammar](#grammar)), trailing its `:=` initializer instead of a `cell`
declaration's own initializer. Everything above applies unchanged — an out cell is always
derived ([derived cells: diagnosed, never corrected](#derived-cells-diagnosed-never-corrected) is the only case that ever actually applies to one), so a filter attached to an
out cell is a pure diagnostic, exactly like a filter on any other cell a relationship currently
claims:

```adam
{{#include examples/filters/filter_on_an_out_cell.adm2}}
```

A [`source`](source.md) declaration's grammar carries the same optional `cell_filter` clause
too — see [a source cell can be filtered too](source.md#a-source-cell-can-be-filtered-too). A `filter` attaches to
any cell kind (`cell`, `source`, or `out`) exactly the same way; the grammar has no
per-kind restriction.

## Errors

Two filter-declaration mistakes are caught while parsing the sheet, before it is ever
resolved: a non-range filter body that never references `_` fails with "filter must
reference `_`", and a `filter` attached to a tuple-typed cell fails with "filter on a
tuple-typed cell is not yet supported". At most one filter may be attached per cell.
