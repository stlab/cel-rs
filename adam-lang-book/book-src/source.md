# Source Cells

## Grammar

```text
source_decl = "source" identifier cell_type_init [ cell_filter ] [ require_block ] ";".
```

A `source` declaration uses the same type and initializer rules as a
[`cell`](cells.md#cell-declarations), including evaluation without cell scope.
It accepts an optional `filter` clause and `require` block.
`source`, `cell`, and `out` share one namespace; declaring both `source width`
and `cell width` is a "duplicate cell" error.

```adam
{{#include examples/source/basic_source.adm2}}
```

`width` and `height` supply inputs to the output that multiplies them.

## Always a source, never derived

A `source` cell cannot be claimed as a method's output, including in a conditional branch.
Naming it on a binding's left-hand side is an error. An [`out`](outputs.md) cell,
by contrast, always has its own fixed writer.

A plain `cell`'s source/derived status is a per-round decision the solver makes from
[strength](relationships.md#strength-who-gets-to-stay-a-source) — the same cell might be a
source in one round and derived in the next, depending on what's been written recently.
A `source` cell retains its last external write, or its initializer before any write.
A filter may conform its effective value without changing that source value.

## A source cell can be filtered too

A `source` cell can be read wherever a plain cell can and written directly by the host.
A `require` block reports domain diagnostics:

```adam
{{#include examples/source/source_with_a_requirement.adm2}}
```

A [`filter`](filters.md) clause conforms its effective value during resolution,
not during a write:

```adam
{{#include examples/source/source_with_a_filter.adm2}}
```
