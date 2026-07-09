# `begin`: Surface Forced Cells in the UI

**Date:** 2026-07-09
**Author:** Sean Parent (with Claude)
**Status:** Approved

## Problem

`property_model::Sheet` already exposes `is_forced(id)` and `forced_cells()` (added in
the "Planner: Forced-Output Cells" work), reporting which cells some active
relationship's method structure guarantees will always be overwritten by `propagate()`,
regardless of write-recency strength. Writing to a forced cell is either silently
overwritten on the next propagate or edited by the user in a field that can never
actually take effect.

The `begin` demo app does not use this API at all: the Inspector sidebar renders every
cell as an editable text field, forced or not, and the D3 graph gives no visual
indication that a cell (or the edge feeding it) is forced. The demo source also has no
example of a conditional relationship that forces a cell, so there is nothing to
exercise this behavior against.

## Design

### Inspector: disable forced fields

`SpTextfield` (`begin/src/spectrum.rs`) gains a `disabled: bool` prop, mapped to the
`disabled` boolean attribute on `<sp-textfield>` the same way `invalid` already maps to
the `invalid` attribute (present when `true`, omitted when `false`).

`CellRow` (`begin/src/inspector.rs`) computes:

```rust
let forced = use_memo(move || sheet.read().is_forced(id));
```

and passes `disabled: *forced.read()` to `SpTextfield`. The custom element's native
disabled behavior blocks focus and input at the DOM level, so no additional guard is
needed in the `oninput` handler.

### Graph: highlight forced cells and their producing edge

`bridge::GraphData` (`begin/src/bridge.rs`) gains a `forced: Vec<String>` field —
stable cell-node IDs, populated the same way as the existing `changed` field:

```rust
let forced = sheet.forced_cells().map(cell_node_id).collect();
```

`graph.js`'s `update()` builds a `Set` from `data.forced` and:
- toggles a `forced` CSS class on cell `<rect>` elements whose ID is in the set
- toggles a `forced-edge` CSS class on constraint `<line>` elements whose *target* is a
  forced cell (the edge from the relationship that produces it; direction is already
  established by the existing directed-link logic when a plan is cached)

`graph.css` adds:

```css
.node-cell.forced { stroke: #8e44ad; stroke-width: 3; }
.link.forced-edge { stroke: #8e44ad; stroke-width: 3; }
```

A distinct purple, chosen not to collide with the existing pulse color (`#f90`) or
branch colors (`#4a90d9` / `#e67e22`). Forced cells are always part of a currently
*active* relationship, so this never conflicts with the existing inactive-relationship
dimming (which only applies to relationships an active control link has switched off).

### Demo source: a conditional relationship that forces a cell

`DEMO_SOURCE` (`begin/src/app.rs`) adds one cell and one method to the existing `p`
conditional's `1i32` branch, rather than introducing an unrelated second conditional:

```
cell g: f64;
...
conditional p {
    0i32 => { ... }
    1i32 => {
        method [f] -> [c] { f * 2.0 }
        method [c] -> [f] { c / 2.0 }
        method [c] -> [g] { c * 10.0 }
    }
}
```

`[c] -> [g]` is a single-method relationship, so `g` is forced whenever branch `1i32` is
active and not forced otherwise — directly exercising "forced only while its owning
conditional branch is active." Setting `p` to `1` in the running demo disables `g`'s
Inspector field and highlights `g` and its incoming edge in the graph; setting `p` back
to `0` (or anything else) re-enables it.

## Testing

- `begin/src/bridge.rs`: unit test that `to_graph_data` includes a forced cell's node ID
  in `GraphData::forced` after `propagate()` activates the forcing branch, and omits it
  when the branch is inactive.
- `begin/src/spectrum.rs` / `inspector.rs`: no new Rust-testable behavior beyond the
  `disabled` prop threading through `SpTextfield`, which is a thin wrapper already
  covered by existing component patterns; no dedicated unit test needed (rendering isn't
  exercised by `cargo test` for Dioxus components in this crate today, consistent with
  the rest of `inspector.rs`).
- Manual verification: run the app, toggle `p` between `0` and `1`, confirm `g`'s field
  disables/enables and the graph highlights `g` + its incoming edge accordingly.

## Out of Scope

- No changes to `property-model`; `is_forced`/`forced_cells` already exist.
- No tooltip or label annotation on forced fields — disabling the input is sufficient
  per current design; a label annotation can be added later if it proves necessary.
- No highlighting of the relationship node that produces a forced cell, only the cell
  and its incoming edge, per the approved design.
