# `begin` Inspector Support for Out Cells

**Date:** 2026-08-11
**Branch:** worktree-begin-out-cells
**Status:** Approved (design), not yet implemented

## Summary

`adam-rs` already implements output cells and their conditions end to end
(`docs/superpowers/specs/2026-08-07-output-cells-design.md`,
`2026-08-09-adam-lang-output-syntax-design.md`) — `Sheet::add_output`,
`output_valid`, `violated_conditions`, `contributing_cells`,
`condition_contributing_cells` are all implemented and tested. `begin`'s
Inspector, however, has no awareness of out cells at all: every non-forced
cell renders as an enabled text field regardless of whether it currently
has any bearing on an out cell's value, and a violated condition is
invisible in the UI.

This doc adds two small aggregate queries to `adam-rs::Sheet` and wires them
into `begin`'s `Inspector` to produce three field states, layered on top of
the existing `disabled`/`invalid`:

1. **Don't-care fields.** If the sheet has at least one out cell, any
   non-forced field that isn't currently determining *any* out cell's value
   is disabled (showing its last value), reducing visual noise around
   inputs that are irrelevant to the sheet's out cells right now.
2. **Invalid out-cell fields.** An out cell's own (already-disabled) field
   turns invalid (red) when one or more of its conditions currently fail.
3. **Warning fields.** A field that *is* relevant (per #1) but specifically
   feeds a currently-failing condition gets a softer "warning" treatment,
   distinct from full invalid — it isn't itself broken, but it's implicated
   in a failing precondition.

---

## 1. New `adam-rs::Sheet` queries

Three additions to `adam-rs/src/sheet.rs`, alongside the existing
`cells()`/`relationships()`/`conditionals()` and
`contributing_cells`/`condition_contributing_cells`/`output_valid`/
`violated_conditions`.

### 1.1 `outputs()`

Follows the exact pattern of `conditionals()` (`sheet.rs:1124`):

```rust
/// Iterates all live output IDs in the sheet.
///
/// - Complexity: O(n) where n is the number of outputs.
pub fn outputs(&self) -> impl Iterator<Item = OutputId> + '_ {
    self.outputs.keys()
}
```

No ordering guarantee, consistent with `cells()`/`conditionals()`.

### 1.2 `output_relevant_cells()`

```rust
/// Returns the union of `contributing_cells` over every live output's cell — the set of
/// cells currently determining at least one output's value, as of the last `propagate()`
/// call.
///
/// - Postcondition: empty if the sheet has no outputs.
/// - Complexity: O(sum of `contributing_cells` cost over every output).
pub fn output_relevant_cells(&self) -> HashSet<CellId> {
    self.outputs()
        .filter_map(|id| self.output_cell(id))
        .flat_map(|cell| self.contributing_cells(cell))
        .collect()
}
```

Before any `propagate()` call, `contributing_cells(cell)` returns `{cell}`
itself (its documented postcondition), so `output_relevant_cells()` returns
the set of output cells themselves in that case, not their eventual
sources — consistent with every other "as of last propagate" query on
`Sheet` (e.g. `is_forced`, `output_valid`) being meaningless/conservative
before the first `propagate()`.

### 1.3 `output_violation_cells()`

```rust
/// Returns the union of `condition_contributing_cells` over every condition that
/// evaluated `false` as of the last `propagate()` call, across every output in the sheet.
///
/// - Postcondition: empty if the sheet has no outputs, or if every condition on every
///   output currently holds.
/// - Complexity: O(sum of `condition_contributing_cells` cost over every violated condition).
pub fn output_violation_cells(&self) -> HashSet<CellId> {
    self.outputs()
        .flat_map(|id| self.violated_conditions(id))
        .flat_map(|cid| self.condition_contributing_cells(cid))
        .collect()
}
```

Both are pure, read-only, dynamic queries — no new state, no changes to
`propagate()` or any existing method.

---

## 2. `begin`: `Inspector` field states

### 2.1 Computing status once per render

In `begin/src/inspector.rs`, `Inspector` (the parent component, not each
`CellRow`) computes:

```rust
struct OutputStatus {
    has_outputs: bool,
    relevant: HashSet<CellId>,        // Sheet::output_relevant_cells(), plus every
                                       // conditional's match cell (see below)
    warning: HashSet<CellId>,         // Sheet::output_violation_cells()
    invalid_outputs: HashSet<CellId>, // out cells whose output_valid() is false
}
```

**Match-cell carve-out.** `Sheet::contributing_cells` (and so
`output_relevant_cells`, which is built from it) only traces back through
relationship *method* inputs — it never includes a conditional's own match
cell, even when that conditional's currently-active branch is what's
feeding an out cell (e.g. `p` in `begin/examples/out-cell.adm2` switches
whether `c` feeds `b`, but `p` itself never appears in
`contributing_cells`). This is a correct, already-tested characteristic of
that `adam-rs` primitive, not a bug — but taken literally it would let the
Inspector's "don't care" rule disable the very toggle that controls which
branch is active. `begin` (not `adam-rs`) therefore unconditionally unions
every conditional's match cell into `relevant`, regardless of which branch
is currently active:

```rust
relevant: sheet
    .output_relevant_cells()
    .into_iter()
    .chain(sheet.conditionals().filter_map(|id| sheet.conditional_match_cell(id)))
    .collect(),
```

`invalid_outputs` is a trivial filter (no new domain logic, so it stays in
`begin` rather than moving into `adam-rs`):

```rust
sheet.outputs()
    .filter(|&id| !sheet.output_valid(id))
    .filter_map(|id| sheet.output_cell(id))
    .collect()
```

Computed via one `use_memo` keyed on `sheet`, passed down to every
`CellRow` as a single `Signal<OutputStatus>` (or `Memo<OutputStatus>`) prop
— avoids each row recomputing the same sheet-wide traversal.

### 2.2 Per-row flags

`CellRow` combines its existing `forced`/`has_error` memos with the shared
`OutputStatus`:

- `disabled = forced || (status.has_outputs && !status.relevant.contains(&id))`
- `invalid = has_error || status.invalid_outputs.contains(&id)`
- `warning = !invalid && status.warning.contains(&id)`

The `disabled` rule applies broadly: any non-forced cell not currently
relevant to any out cell is disabled, whether or not it's presently a
"source" in the planner's sense — consistent with `output_relevant_cells`
itself being dynamic/as-of-last-propagate rather than structural.

`warning` is suppressed when `invalid` is already true, so a field never
shows both states at once.

---

## 3. `SpTextfield`: `warning` prop

`begin/src/spectrum.rs`'s `SpTextfield` gains a fourth boolean prop,
parallel to `invalid`/`disabled`:

```rust
#[component]
pub fn SpTextfield(
    id: String,
    value: String,
    invalid: bool,
    warning: bool,
    disabled: bool,
    oninput: EventHandler<FormEvent>,
    onfocus: EventHandler<FocusEvent>,
    onblur: EventHandler<FocusEvent>,
) -> Element {
    rsx! {
        sp-textfield {
            "id": "{id}",
            "value": "{value}",
            "invalid": if invalid { "true" },
            "disabled": if disabled { "true" },
            class: if warning { "warning" },
            oninput: move |e| oninput.call(e),
            onfocus: move |e| onfocus.call(e),
            onblur: move |e| onblur.call(e),
        }
    }
}
```

A new stylesheet, `begin/assets/inspector.css`, linked from `App` next to
the existing `graph.css` `document::Link`, adds a rule scoped to
`sp-textfield.warning:not([invalid])` overriding the field's border color
to an amber/warning tone. `sp-textfield` is a Spectrum Web Component with
no built-in "warning" appearance (only default/invalid), so this is a
custom-property override on the host element, not a native SWC state. The
exact custom-property name(s) SWC's `sp-textfield` exposes for border
theming will be confirmed empirically against the rendered shadow DOM using
the `verifying-begin-ui` skill during implementation, per
`begin/CLAUDE.md`'s mandate to render and inspect UI changes rather than
infer them from RSX alone — this doc does not commit to a specific
property name.

---

## 4. Verification

No new demo is added to `begin/assets/` (out of scope by request).
`begin/examples/out-cell.adm2` already has exactly the shape needed to
exercise all three states (a don't-care cell, a conditional, and an `out`
with one condition) and can be loaded through `begin`'s existing "Open…"
file picker. Implementation verification uses the `verifying-begin-ui`
skill against that file:

1. Load `begin/examples/out-cell.adm2` via "Open…".
2. Confirm cell `a`/`b` (relevant to `result`) stay enabled; confirm any
   cell not feeding `result` (per the file's current contents at
   implementation time) shows disabled with its last value.
3. Write values that violate `result`'s `min_a` condition; confirm
   `result`'s own field turns invalid (red), and `a`'s field (the
   condition's contributing cell) shows the warning treatment.
4. Fix the values so the condition holds again; confirm both states clear.

---

## 5. Files changed / added

| File | Change |
| --- | --- |
| `adam-rs/src/sheet.rs` | New `outputs()`, `output_relevant_cells()`, `output_violation_cells()` |
| `begin/src/inspector.rs` | `OutputStatus` computation in `Inspector`; `CellRow` gains `disabled`/`invalid`/`warning` derivation |
| `begin/src/spectrum.rs` | `SpTextfield` gains `warning: bool` prop |
| `begin/assets/inspector.css` | New: `.warning:not([invalid])` border-color override |
| `begin/src/app.rs` | New `document::Link` for `inspector.css` |

---

## 6. Testing notes

Derived from the contract/public interface only:

- `outputs()` iterates exactly the `OutputId`s returned by live `add_output`
  calls; empty for a sheet with none.
- `output_relevant_cells()` is empty for a sheet with no outputs; before any
  `propagate()` call, returns the set of output cells themselves (per
  `contributing_cells`'s own pre-propagate postcondition); after
  `propagate()`, returns the union of root cells actually determining each
  output's value, and updates across subsequent `write`/`propagate` calls
  that change the selected plan.
- `output_violation_cells()` is empty for a sheet with no outputs, or when
  every condition on every output currently holds; after a `propagate()`
  that leaves one or more conditions false, returns the union of those
  conditions' `condition_contributing_cells`; updates across subsequent
  `propagate()` calls that change which conditions hold.
- `begin`'s `Inspector`/`CellRow`/`SpTextfield` changes are UI glue with no
  dedicated unit test, consistent with how the existing `forced`/`disabled`
  wiring landed (`docs/superpowers/plans/2026-07-09-begin-forced-cells-ui.md`,
  Task 2) — verified manually per §4 instead.

---

## 7. Deferred / out of scope

- Rendering out cells/conditions distinctly in the D3 graph view
  (`begin/src/bridge.rs`/`graph.js`) — already deferred by
  `2026-08-09-adam-lang-output-syntax-design.md` §13; this doc only covers
  the Inspector's text fields.
- Adding a bundled `assets/` demo exercising out cells / wiring
  `begin/examples/out-cell.adm2` into the demo picker — explicitly out of
  scope per discussion; that file is used ad hoc via "Open…" for
  verification only.
- Any new visual treatment distinguishing "forced" from "don't care" within
  the `disabled` state — both continue to map to the same `disabled: true`
  attribute, matching existing behavior for forced cells.
- Surfacing *which* condition(s) failed (e.g. a tooltip listing violated
  condition names) — not requested; `invalid`/`warning` are boolean-only for
  now.
- `warning` is computed independently of `disabled`/relevance (§2.2's
  formula checks membership in the violation set, not in `relevant`), so a
  cell that feeds a condition's inputs but not the output's own writer
  method (e.g. a bound like `max_area` in the worked example) can be both
  `disabled` (don't-care, since it's outside `output_relevant_cells`) and
  `warning` at once. Accepted as-is rather than gating `warning` on
  relevance: a don't-care field showing the softer warning treatment is a
  reasonable signal that it's still implicated in a failing condition, even
  though editing it currently has no effect on any out cell's value.
