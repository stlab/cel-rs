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
default-branch token). A cell can have at most one filter; filters have no names.

```adam
cell level: i32 = 50 filter 0..=100;             // a fixed range
cell level: i32 = 50 filter 0..=max;             // upper bound is another cell
cell level: i32 = 50 filter clamp(_, 0, max);    // an arbitrary expression over `_`
```

A filter expression must reference `_` at least once (unless it's a range expression, see
[range filters](#range-filters)) and must produce a value of exactly the filtered cell's own type; violating either is a
parse-time error, not a runtime one.

Filters, including range-bound evaluators, must be purely functional: equal candidate
and argument values produce equivalent values or errors, without externally observable
side effects or reads of mutable external state. A conforming filter must also be
idempotent: filtering its result again with the same arguments leaves that result
unchanged. There is no guarantee that a filter executes, or when or how often it
executes. The sheet may reuse a result while preserving conformance and diagnostics.

## Writing never filters

Writing a cell stores exactly the supplied value without applying its filter.
The filter applies when the sheet resolves.

```adam
{{#include examples/filters/write_never_filters.adm2}}
```

The filter cannot cause a write to fail or report an error.
A read returns the written value until the sheet next resolves.

## The raw value is never lost

A filtered cell keeps two values: its raw last-written value, the _source_,
and, when something currently claims it, a computed override, the _derived_ value. Reading
the cell always returns the derived value if one is present, the source value otherwise. A
filter writes only the derived value, preserving the original input across resolutions.
If a dynamic bound loosens, the cell returns toward its last-written value rather than
an intermediate clamped value:

```adam
{{#include examples/filters/raw_value_never_lost.adm2}}
```

Self-referencing and forced methods also preserve source values; see
[reverting to source](conditionals.md#the-default-branch-and-reverting-to-source).

## Range filters

A CEL inclusive range over a [built-in numeric type](cells.md#built-in-types-and-inference)
defines a *range filter*. Resolution clamps the candidate into `[lo, hi]`.
The sheet can report the current bounds independently of a candidate value:

```adam
{{#include examples/filters/range_filter_kind.adm2}}
```

A range filter need not reference `_`; its endpoints are independent of the candidate.
The [tutorial's live example](tutorial.md#filters) displays bounds that track the range.

## Derived cells: diagnosed, never corrected

When a relationship claims a filtered cell, the filter diagnoses violations without
changing the derived value. A violation does not prevent resolution; reads still return
the relationship's result:

```adam
{{#include examples/filters/derived_cell_diagnosed_not_corrected.adm2}}
```

A filter violation on a source or derived cell never causes resolution to fail.
Hosts can query violations through the [embedding API](reference.md#the-host-embedding-api).

## A filter on an output cell

An [`out`](outputs.md) declaration accepts a filter after its `:=` initializer.
An output cell is always derived, so its filter only diagnoses violations:

```adam
{{#include examples/filters/filter_on_an_out_cell.adm2}}
```

## Errors

The parser catches filter-declaration mistakes before resolving the sheet: a non-range
filter body that never references `_` fails with "filter must reference `_`", and a body
with the wrong complete type fails with "filter must produce `T`". Filters accept tuples
and arrays as well as concrete leaves; compound results must match every nested field and
element type. At most one filter may be attached per cell.
