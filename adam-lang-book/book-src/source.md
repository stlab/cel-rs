# Chapter 3: Source Cells

## 3.1 Grammar

```text
source_decl = "source" identifier cell_type_init [ require_block ] ";".
```

A `source` declaration looks exactly like a [`cell`](cells.md#22-cell-declarations)
declaration minus the optional `filter` clause: the same `cell_type_init` (a type, an
initializer, or both), evaluated the same way, at parse time, with no cell scope; the same
optional trailing `require` block. `source` and `cell` even share one namespace with `out`, so
declaring `source width` and `cell width` in the same sheet is the same "duplicate cell" error
two `cell width` declarations would be.

```
{{#include examples/source/basic_source.adm2}}
```

`width` above is declared `source`, `height` a plain `cell`; both are ordinary inputs to the
`out` that multiplies them.

## 3.2 Always a source, never derived

A `source` cell is the mirror image of an [`out`](outputs.md) cell. Where an `out` cell is
always derived by its own fixed writer, a `source` cell is always left alone by the solver:
never claimable as a `relationship` binding's output, a `conditional` branch's output, or an
`out`'s own writer target. This is checked once, structurally, the moment the offending
declaration is parsed — resolving the sheet is never reached:

```
{{#include examples/source/source_cannot_be_derived.adm2}}
```

A plain `cell`'s source/derived status is a per-round decision the solver makes from
[strength](relationships.md#52-strength-who-gets-to-stay-a-source) — the same cell might be a
source in one round and derived in the next, depending on what's been written recently. A
`source` cell opts out of that entirely: whatever a host last wrote to it (or its own declared
initializer, before any write) is always its value, unconditionally.

## 3.3 What source cells can't do

A `source` cell is ordinary in every other respect: it can be read anywhere a plain `cell`
can, written directly at any time (that's the whole point — a `source` cell always reflects
whatever was last supplied from outside the sheet), and can carry a `require` block for
domain diagnostics, exactly like a `cell` or `out`:

```
{{#include examples/source/source_with_a_requirement.adm2}}
```

The one thing a `source` cell cannot carry is a `filter` clause: `source_decl`'s grammar (3.1)
has no `cell_filter` slot at all, unlike `cell_decl` and `out_decl`. A filter's job is to
correct a value the sheet is willing to treat as *its own* domain constraint; a `source`
cell's value is definitionally supplied from outside the sheet, so there's nothing for a
filter to conform it against. See [Chapter 7](filters.md) for `filter` as it applies to `cell`
and `out`.
