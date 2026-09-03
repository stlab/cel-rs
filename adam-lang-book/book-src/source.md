# Chapter 3: Source Cells

## 3.1 Grammar

```text
source_decl = "source" identifier cell_type_init [ cell_filter ] [ require_block ] ";".
```

A `source` declaration looks exactly like a [`cell`](cells.md#22-cell-declarations)
declaration: the same `cell_type_init` (a type, an initializer, or both), evaluated the same
way, at parse time, with no cell scope; the same optional trailing `filter` clause and `require`
block. `source` and `cell` even share one namespace with `out`, so declaring `source width` and
`cell width` in the same sheet is the same "duplicate cell" error two `cell width` declarations
would be.

```
{{#include examples/source/basic_source.adm2}}
```

Both `width` and `height` above are declared `source`: ordinary inputs to the `out` that
multiplies them, with nothing in this sheet ever claiming either as a relationship or
conditional output.

## 3.2 Always a source, never derived

A `source` cell is the mirror image of an [`out`](outputs.md) cell. Where an `out` cell is
always derived by its own fixed writer, a `source` cell is always left alone by the solver:
never claimable as a `relationship` binding's output, a `conditional` branch's output, or an
`out`'s own writer target. This is checked once, structurally, the moment the offending
declaration is parsed — resolving the sheet is never reached; naming a `source` cell on a
binding's left-hand side is rejected before the sheet can ever be resolved.

A plain `cell`'s source/derived status is a per-round decision the solver makes from
[strength](relationships.md#72-strength-who-gets-to-stay-a-source) — the same cell might be a
source in one round and derived in the next, depending on what's been written recently. A
`source` cell opts out of that entirely: whatever a host last wrote to it (or its own declared
initializer, before any write) is always its value, unconditionally.

## 3.3 A source cell can be filtered too

A `source` cell is ordinary in every other respect: it can be read anywhere a plain `cell`
can, written directly at any time (that's the whole point — a `source` cell always reflects
whatever was last supplied from outside the sheet), can carry a `require` block for domain
diagnostics, exactly like a `cell` or `out`:

```
{{#include examples/source/source_with_a_requirement.adm2}}
```

and can carry a [`filter`](filters.md) clause, exactly like a `cell` or `out`: whatever a host
writes to a filtered `source` cell is conformed the same way a filtered plain `cell`'s value
is, live, whenever the sheet resolves:

```
{{#include examples/source/source_with_a_filter.adm2}}
```

See [Chapter 5](filters.md) for `filter`'s full rules — everything there applies to a `source`
cell unchanged.
