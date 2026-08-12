# `begin` Inspector Support for Out Cells Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `begin`'s Inspector three new out-cell-aware field states: disable any non-forced field that currently has no bearing on any out cell ("don't care"), flag an out cell's own field invalid when one of its conditions fails, and flag the fields of cells implicated in a failing condition with a softer "warning" state.

**Architecture:** Three small aggregate queries land on `adam_rs::Sheet` (`outputs`, `output_relevant_cells`, `output_violation_cells`), built purely from primitives that already exist and are already tested (`contributing_cells`, `condition_contributing_cells`, `output_valid`, `violated_conditions`). `begin`'s `Inspector` computes one `OutputStatus` snapshot per render (adding every conditional's match cell into "relevant", since `contributing_cells` never traces through those) and shares it across every `CellRow`, which combines it with its existing `forced`/`has_error` state into `disabled`/`invalid`/`warning` booleans. `SpTextfield` gains a fourth `warning` prop rendered as a CSS class, styled by a new stylesheet linked from `App`.

**Tech Stack:** Rust, `adam-rs` (Sheet/OutputId/ConditionId), Dioxus 0.7 (`begin` crate), Spectrum Web Components (`sp-textfield`).

## Global Constraints

- `cargo fmt --all` must be run before every commit (enforced by the pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`, `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`, and `cargo clippy -p begin --all-targets -- -D warnings` must all be clean before the branch is considered done.
- Every function needs a `///` contract-style doc comment: summary, preconditions/postconditions only where non-obvious, a `Complexity` bullet whenever the operation is not O(1).
- Unit tests are derived from the contract/public interface only, never from implementation details.
- Fallible ops use `.op1r`/`.op2r`/`checked_*` conventions — not touched by this plan, but any new arithmetic must follow it.
- Never commit directly to `main`; this work happens on the `worktree-begin-out-cells` branch.
- Before considering any UI change to `begin` complete, actually render it and look, using the `verifying-begin-ui` skill (`begin/CLAUDE.md`) — build/clippy passing proves nothing about what renders.

---

### Task 1: `Sheet::outputs()`

**Files:**
- Modify: `adam-rs/src/sheet.rs` (new method, placed immediately after `conditionals()` at `sheet.rs:1121-1126`)
- Test: `adam-rs/tests/integration.rs`

**Interfaces:**
- Consumes: the existing private field `outputs: SlotMap<OutputId, OutputData>` (`sheet.rs:64`).
- Produces: `Sheet::outputs(&self) -> impl Iterator<Item = OutputId> + '_`. Tasks 2 and 3 call this directly.

- [ ] **Step 1: Write the failing tests**

Add to `adam-rs/tests/integration.rs`, after `condition_contributing_cells_returns_empty_for_invalid_id` (the block ending at line 1683):

```rust
#[test]
fn outputs_empty_for_sheet_with_no_outputs() {
    let sheet = Sheet::new();
    assert_eq!(sheet.outputs().count(), 0);
}

#[test]
fn outputs_iterates_every_live_output_id() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let out_a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let out_b = sheet.add_cell(0_i32);

    let id_a = sheet
        .add_output(Method::from_fn_1_1(a, out_a, |x: &i32| Ok(*x)), vec![])
        .unwrap();
    let id_b = sheet
        .add_output(Method::from_fn_1_1(b, out_b, |x: &i32| Ok(*x)), vec![])
        .unwrap();

    let ids: HashSet<_> = sheet.outputs().collect();
    assert_eq!(ids, HashSet::from([id_a, id_b]));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-rs outputs_`
Expected: compile error — `no method named \`outputs\` found for struct \`Sheet\``.

- [ ] **Step 3: Implement `outputs()`**

In `adam-rs/src/sheet.rs`, immediately after `conditionals()` (ends at line 1126, right before the `conditional_match_cell` doc comment at line 1128):

```rust
    /// Iterates all live output IDs in the sheet.
    ///
    /// - Complexity: O(n) where n is the number of outputs.
    pub fn outputs(&self) -> impl Iterator<Item = OutputId> + '_ {
        self.outputs.keys()
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p adam-rs outputs_`
Expected: PASS (2 passed)

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/sheet.rs adam-rs/tests/integration.rs
git commit -m "$(cat <<'EOF'
feat(adam-rs): add Sheet::outputs

Iterates every live OutputId, mirroring the existing cells()/
relationships()/conditionals() pattern. Lets a caller discover
whether a sheet has any outputs at all without separately tracking
OutputIds returned by add_output.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `Sheet::output_relevant_cells()`

**Files:**
- Modify: `adam-rs/src/sheet.rs` (new method, placed immediately after `condition_contributing_cells` at `sheet.rs:570-586`, before the `write()` doc comment at `sheet.rs:588`)
- Test: `adam-rs/tests/integration.rs`

**Interfaces:**
- Consumes: `Sheet::outputs()` (Task 1), `Sheet::output_cell(&self, id: OutputId) -> Option<CellId>` (`sheet.rs:468`), `Sheet::contributing_cells(&self, id: CellId) -> HashSet<CellId>` (`sheet.rs:533`, already implemented and tested).
- Produces: `Sheet::output_relevant_cells(&self) -> HashSet<CellId>`. Task 6 (`begin`) calls this directly.

- [ ] **Step 1: Write the failing tests**

Add to `adam-rs/tests/integration.rs`, after Task 1's new tests:

```rust
#[test]
fn output_relevant_cells_empty_when_sheet_has_no_outputs() {
    let sheet = Sheet::new();
    assert_eq!(sheet.output_relevant_cells(), HashSet::new());
}

#[test]
fn output_relevant_cells_returns_output_cell_itself_before_propagate() {
    let (sheet, output, ..) = sheet_with_area_output();
    let area = sheet.output_cell(output).unwrap();
    assert_eq!(sheet.output_relevant_cells(), HashSet::from([area]));
}

#[test]
fn output_relevant_cells_returns_root_sources_after_propagate() {
    let (mut sheet, _output, width, height, _max_area) = sheet_with_area_output();
    sheet.write(width, 5_i32).unwrap();
    sheet.write(height, 4_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(sheet.output_relevant_cells(), HashSet::from([width, height]));
}

#[test]
fn output_relevant_cells_unions_across_multiple_outputs() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(1_i32);
    let b = sheet.add_cell(2_i32);
    let out_a = sheet.add_cell(0_i32);
    let out_b = sheet.add_cell(0_i32);
    sheet
        .add_output(Method::from_fn_1_1(a, out_a, |x: &i32| Ok(*x)), vec![])
        .unwrap();
    sheet
        .add_output(Method::from_fn_1_1(b, out_b, |x: &i32| Ok(*x)), vec![])
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(sheet.output_relevant_cells(), HashSet::from([a, b]));
}

#[test]
fn output_relevant_cells_updates_when_a_different_relationship_becomes_active() {
    let mut sheet = Sheet::new();
    let p = sheet.add_cell(0_i32);
    let a = sheet.add_cell(1_i32);
    let b = sheet.add_cell(2_i32);
    let c = sheet.add_cell(0_i32);
    let rel0 = sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    let rel1 = sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_conditional(
            p,
            vec![(vec![0_i32], vec![rel0]), (vec![1_i32], vec![rel1])],
            vec![],
        )
        .unwrap();
    sheet
        .add_output(Method::from_fn_1_1(b, c, |x: &i32| Ok(*x)), vec![])
        .unwrap();

    sheet.write(p, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(sheet.output_relevant_cells(), HashSet::from([a]));

    sheet.write(p, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(sheet.output_relevant_cells(), HashSet::from([b]));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-rs output_relevant_cells_`
Expected: compile error — `no method named \`output_relevant_cells\` found for struct \`Sheet\``.

- [ ] **Step 3: Implement `output_relevant_cells()`**

In `adam-rs/src/sheet.rs`, immediately after `condition_contributing_cells` (ends at line 586, right before the `write()` doc comment at line 588):

```rust
    /// Returns the union of `contributing_cells` over every live output's cell — the set
    /// of cells currently determining at least one output's value, as of the last
    /// `propagate()` call.
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

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p adam-rs output_relevant_cells_`
Expected: PASS (5 passed)

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/sheet.rs adam-rs/tests/integration.rs
git commit -m "$(cat <<'EOF'
feat(adam-rs): add Sheet::output_relevant_cells

Unions contributing_cells over every live output's cell, giving
callers a single query for "which cells currently determine at
least one output's value" instead of iterating outputs and unioning
themselves.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `Sheet::output_violation_cells()`

**Files:**
- Modify: `adam-rs/src/sheet.rs` (new method, placed immediately after `output_relevant_cells` from Task 2)
- Test: `adam-rs/tests/integration.rs`

**Interfaces:**
- Consumes: `Sheet::outputs()` (Task 1), `Sheet::violated_conditions(&self, id: OutputId) -> impl Iterator<Item = ConditionId> + '_` (`sheet.rs:517`), `Sheet::condition_contributing_cells(&self, id: ConditionId) -> HashSet<CellId>` (`sheet.rs:577`).
- Produces: `Sheet::output_violation_cells(&self) -> HashSet<CellId>`. Task 6 (`begin`) calls this directly.

- [ ] **Step 1: Write the failing tests**

Add to `adam-rs/tests/integration.rs`, after Task 2's new tests:

```rust
#[test]
fn output_violation_cells_empty_when_sheet_has_no_outputs() {
    let sheet = Sheet::new();
    assert_eq!(sheet.output_violation_cells(), HashSet::new());
}

#[test]
fn output_violation_cells_empty_when_all_conditions_hold() {
    let (mut sheet, _output, width, height, _max_area) = sheet_with_area_output();
    sheet.write(width, 5_i32).unwrap();
    sheet.write(height, 4_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(sheet.output_violation_cells(), HashSet::new());
}

#[test]
fn output_violation_cells_returns_contributing_cells_for_the_failing_condition() {
    let (mut sheet, output, width, height, max_area) = sheet_with_area_output();
    sheet.write(width, 50_i32).unwrap();
    sheet.write(height, 40_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(!sheet.output_valid(output));
    assert_eq!(
        sheet.output_violation_cells(),
        HashSet::from([width, height, max_area])
    );
}

#[test]
fn output_violation_cells_unions_across_multiple_violated_conditions() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let max_a = sheet.add_cell(5_i32);
    let out_a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let max_b = sheet.add_cell(5_i32);
    let out_b = sheet.add_cell(0_i32);

    sheet
        .add_output(
            Method::from_fn_1_1(a, out_a, |x: &i32| Ok(*x)),
            vec![(
                "max_a",
                Condition::from_fn_2([a, max_a], |v: &i32, max: &i32| Ok(v <= max)),
            )],
        )
        .unwrap();
    sheet
        .add_output(
            Method::from_fn_1_1(b, out_b, |x: &i32| Ok(*x)),
            vec![(
                "max_b",
                Condition::from_fn_2([b, max_b], |v: &i32, max: &i32| Ok(v <= max)),
            )],
        )
        .unwrap();

    sheet.write(a, 50_i32).unwrap();
    sheet.write(b, 50_i32).unwrap();
    sheet.propagate().unwrap();

    assert_eq!(
        sheet.output_violation_cells(),
        HashSet::from([a, max_a, b, max_b])
    );
}

#[test]
fn output_violation_cells_updates_across_propagate_calls() {
    let (mut sheet, _output, width, height, _max_area) = sheet_with_area_output();
    sheet.write(width, 50_i32).unwrap();
    sheet.write(height, 40_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(!sheet.output_violation_cells().is_empty());

    sheet.write(height, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(sheet.output_violation_cells(), HashSet::new());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-rs output_violation_cells_`
Expected: compile error — `no method named \`output_violation_cells\` found for struct \`Sheet\``.

- [ ] **Step 3: Implement `output_violation_cells()`**

In `adam-rs/src/sheet.rs`, immediately after `output_relevant_cells` from Task 2:

```rust
    /// Returns the union of `condition_contributing_cells` over every condition that
    /// evaluated `false` as of the last `propagate()` call, across every output in the
    /// sheet.
    ///
    /// - Postcondition: empty if the sheet has no outputs, or if every condition on
    ///   every output currently holds.
    /// - Complexity: O(sum of `condition_contributing_cells` cost over every violated
    ///   condition).
    pub fn output_violation_cells(&self) -> HashSet<CellId> {
        self.outputs()
            .flat_map(|id| self.violated_conditions(id).collect::<Vec<_>>())
            .flat_map(|cid| self.condition_contributing_cells(cid))
            .collect()
    }
```

`violated_conditions(id)` borrows `self`, so its returned iterator can't be
held across the nested `self.condition_contributing_cells(cid)` call inside
the same `flat_map` without first collecting it into an owned `Vec`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p adam-rs output_violation_cells_`
Expected: PASS (5 passed)

- [ ] **Step 5: Run the full adam-rs test suite**

Run: `cargo test -p adam-rs`
Expected: all tests pass, no regressions.

- [ ] **Step 6: Commit**

```bash
git add adam-rs/src/sheet.rs adam-rs/tests/integration.rs
git commit -m "$(cat <<'EOF'
feat(adam-rs): add Sheet::output_violation_cells

Unions condition_contributing_cells over every currently-violated
condition across every output, giving callers a single query for
"which cells are implicated in a failing precondition right now".

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: `SpTextfield` gains a `warning` prop

**Files:**
- Modify: `begin/src/spectrum.rs:39-61` (`SpTextfield`)

**Interfaces:**
- Consumes: nothing new.
- Produces: `SpTextfield { warning: bool, .. }` — a new required prop, rendered as a `class="warning"` attribute on the underlying `sp-textfield` element. Task 6 passes this prop.

This task has no dedicated Rust unit test: `spectrum.rs` wraps a single custom element with no test infrastructure today, consistent with how the existing `invalid`/`disabled` props landed (see `docs/superpowers/plans/2026-07-09-begin-forced-cells-ui.md`, Task 2). Verified in Task 7's manual check.

- [ ] **Step 1: Add the `warning` prop**

In `begin/src/spectrum.rs`, replace the `SpTextfield` component (lines 33-61):

```rust
/// Single-line text input.
///
/// Maps to `<sp-textfield>`. Fires standard DOM `input`, `focus`, and `blur`
/// events. Setting `invalid` to `true` renders the SWC error state (red ring
/// and `aria-invalid`). Setting `warning` to `true` (and `invalid` to `false`)
/// renders a softer amber treatment via the `warning` CSS class, styled in
/// `begin/assets/inspector.css` — not a native SWC state. Setting `disabled`
/// to `true` renders the SWC disabled state and blocks focus/input at the DOM
/// level.
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
            // Boolean attribute: omit entirely when false; presence = invalid.
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

- [ ] **Step 2: Build to verify it compiles**

Run: `cargo build -p begin --no-default-features`
Expected: fails — `CellRow` (the only caller) doesn't pass `warning` yet. This is expected; Task 6 fixes it. Confirm the *only* error is the missing `warning` field at `inspector.rs`'s `SpTextfield` call site, then proceed — do not add a temporary default here.

- [ ] **Step 3: Commit**

```bash
git add begin/src/spectrum.rs
git commit -m "$(cat <<'EOF'
feat(begin): add a warning prop to SpTextfield

A softer, non-native visual state (amber, via a CSS class) distinct
from SWC's built-in invalid (red) state, for fields implicated in a
failing out-cell condition without being invalid themselves. The
crate won't build again until Task 6 updates CellRow's call site.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: `inspector.css` stylesheet, linked from `App`

**Files:**
- Create: `begin/assets/inspector.css`
- Modify: `begin/src/app.rs:198-203` (the `document::Link`/`document::Script` block)

**Interfaces:**
- Consumes: nothing.
- Produces: a `.warning:not([invalid])` CSS rule targeting `sp-textfield`. No other task depends on this by name; Task 4's `class: "warning"` is what makes it apply.

- [ ] **Step 1: Create the stylesheet**

Create `begin/assets/inspector.css`:

```css
/* Softer alternative to sp-textfield's built-in `invalid` state: marks a field that
   currently contributes to a failing out-cell condition, without being itself invalid.
   `:not([invalid])` guards against ever showing both states at once — an invalid field
   always wins. The custom-property name below is a starting guess (Spectrum Web
   Components expose textfield border theming through custom properties); confirm and
   correct it against the rendered shadow DOM in Task 7 before relying on this rule. */
sp-textfield.warning:not([invalid]) {
    --spectrum-textfield-border-color: #e68619;
    --spectrum-textfield-border-color-hover: #e68619;
}
```

- [ ] **Step 2: Link it from `App`**

In `begin/src/app.rs`, add a new `document::Link` immediately after the existing `graph.css` link (line 199):

```rust
        document::Link { rel: "stylesheet", href: asset!("/assets/graph.css") }
        document::Link { rel: "stylesheet", href: asset!("/assets/inspector.css") }
```

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo build -p begin --no-default-features`
Expected: still fails with the same single error as Task 4 Step 2 (`CellRow` doesn't pass `warning` yet) — confirms this task introduced no new compile errors of its own.

- [ ] **Step 4: Commit**

```bash
git add begin/assets/inspector.css begin/src/app.rs
git commit -m "$(cat <<'EOF'
feat(begin): add inspector.css for the warning field state

Linked from App alongside graph.css. The exact custom-property name
is a starting guess, to be confirmed against the rendered shadow DOM
once Task 6 makes SpTextfield's warning prop reachable end to end.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: `Inspector`/`CellRow` compute and apply out-cell status

**Files:**
- Modify: `begin/src/inspector.rs` (whole file; see exact edits below)

**Interfaces:**
- Consumes: `Sheet::outputs()` (Task 1), `Sheet::output_relevant_cells()` (Task 2), `Sheet::output_violation_cells()` (Task 3), `Sheet::output_cell`/`Sheet::output_valid` (existing), `Sheet::conditionals()`/`Sheet::conditional_match_cell()` (existing), `SpTextfield { warning: bool, .. }` (Task 4).
- Produces: `OutputStatus` and `CellFlags` (both private to `inspector.rs`; no other file needs them). `CellRow` now derives `disabled`/`invalid`/`warning` via the pure `cell_flags` function instead of just `disabled = forced`/`invalid = has_error`.

Per root `CLAUDE.md`'s "Unit tests" section: `cell_flags` (Step 3 below) is
genuine branching/combining logic, not a passthrough, so it gets its own
contract and unit tests even though it's called from framework-coupled
Dioxus code. `compute_output_status` and `CellRow`'s wiring around it stay
untested UI glue (as `forced`/`disabled` did in
`docs/superpowers/plans/2026-07-09-begin-forced-cells-ui.md`, Task 2) —
verified instead in Task 7's manual check — because they're straight
plumbing: reading `Sheet`, constructing `OutputStatus`, and calling
`cell_flags`, with no decision of their own left untested once `cell_flags`
is covered.

- [ ] **Step 1: Add `OutputStatus` and its constructor**

In `begin/src/inspector.rs`, add after the `use` block (after line 7, before the `Inspector` doc comment at line 9):

```rust
use std::collections::HashSet;

/// Aggregate out-cell status for the whole sheet, computed once per render and shared by
/// every `CellRow` so `Sheet::output_relevant_cells`/`output_violation_cells` run once
/// instead of once per row.
#[derive(Clone, PartialEq)]
struct OutputStatus {
    /// `true` if the sheet has at least one output.
    has_outputs: bool,
    /// `Sheet::output_relevant_cells()`, plus every conditional's match cell.
    ///
    /// `Sheet::contributing_cells` never traces back through a conditional's match
    /// cell (it only follows relationship method inputs), so without this addition a
    /// conditional's own switch could be marked "don't care" and disabled once the
    /// sheet has any output — blocking the toggle that controls which branch is
    /// active. Match cells are therefore always treated as relevant, independent of
    /// which branch is currently active.
    relevant: HashSet<CellId>,
    /// Union of `Sheet::output_violation_cells()`.
    warning: HashSet<CellId>,
    /// Cells backing an output whose `Sheet::output_valid` is currently `false`.
    invalid_outputs: HashSet<CellId>,
}

/// Computes `sheet`'s current out-cell status for the Inspector.
///
/// - Complexity: O(`Sheet::output_relevant_cells` + `Sheet::output_violation_cells` +
///   the number of conditionals in the sheet).
fn compute_output_status(sheet: &Sheet) -> OutputStatus {
    let outputs: Vec<_> = sheet.outputs().collect();
    let relevant = sheet
        .output_relevant_cells()
        .into_iter()
        .chain(
            sheet
                .conditionals()
                .filter_map(|id| sheet.conditional_match_cell(id)),
        )
        .collect();
    let invalid_outputs = outputs
        .iter()
        .filter(|&&id| !sheet.output_valid(id))
        .filter_map(|&id| sheet.output_cell(id))
        .collect();
    OutputStatus {
        has_outputs: !outputs.is_empty(),
        relevant,
        warning: sheet.output_violation_cells(),
        invalid_outputs,
    }
}
```

- [ ] **Step 2: Write the failing tests for `cell_flags`**

Add a `#[cfg(test)] mod tests` block at the end of `begin/src/inspector.rs`
(this is a new block — the file has no tests today):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn status(has_outputs: bool, relevant: &[CellId], warning: &[CellId], invalid_outputs: &[CellId]) -> OutputStatus {
        OutputStatus {
            has_outputs,
            relevant: relevant.iter().copied().collect(),
            warning: warning.iter().copied().collect(),
            invalid_outputs: invalid_outputs.iter().copied().collect(),
        }
    }

    fn dummy_cell() -> CellId {
        let mut sheet = Sheet::new();
        sheet.add_cell(0_i32)
    }

    #[test]
    fn cell_flags_enabled_when_no_outputs_even_if_not_relevant() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, false, &status(false, &[], &[], &[]));
        assert!(!flags.disabled);
    }

    #[test]
    fn cell_flags_disabled_when_forced_regardless_of_outputs() {
        let id = dummy_cell();
        let flags = cell_flags(id, true, false, &status(false, &[], &[], &[]));
        assert!(flags.disabled);
    }

    #[test]
    fn cell_flags_disabled_when_has_outputs_and_cell_not_relevant() {
        // Both ids must come from the same Sheet: two fresh Sheets' first added cell
        // return equal CellId values (slotmap's key generation is deterministic per
        // map), which would make `id` and `other` indistinguishable below.
        let mut sheet = Sheet::new();
        let id = sheet.add_cell(0_i32);
        let other = sheet.add_cell(0_i32);
        let flags = cell_flags(id, false, false, &status(true, &[other], &[], &[]));
        assert!(flags.disabled);
    }

    #[test]
    fn cell_flags_enabled_when_has_outputs_and_cell_is_relevant() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, false, &status(true, &[id], &[], &[]));
        assert!(!flags.disabled);
    }

    #[test]
    fn cell_flags_invalid_when_has_error() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, true, &status(false, &[], &[], &[]));
        assert!(flags.invalid);
    }

    #[test]
    fn cell_flags_invalid_when_cell_is_an_invalid_output() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, false, &status(true, &[id], &[], &[id]));
        assert!(flags.invalid);
    }

    #[test]
    fn cell_flags_warning_when_in_warning_set_and_not_invalid() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, false, &status(true, &[id], &[id], &[]));
        assert!(flags.warning);
    }

    #[test]
    fn cell_flags_warning_suppressed_when_also_invalid() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, true, &status(true, &[id], &[id], &[]));
        assert!(!flags.warning);
        assert!(flags.invalid);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p begin --no-default-features cell_flags_`
Expected: compile error — `cannot find function \`cell_flags\` in this scope` (and `OutputStatus`'s fields aren't constructible yet if Step 1 hasn't landed the exact field names — it has, per Step 1 above, so this should be exactly one error: the missing `cell_flags` function).

- [ ] **Step 4: Implement `CellFlags` and `cell_flags`**

Add directly after `compute_output_status` (the function added in Step 1):

```rust
/// A cell's Inspector display flags, derived from its own forced/error state and the
/// sheet-wide out-cell status.
#[derive(Clone, Copy, PartialEq, Eq)]
struct CellFlags {
    disabled: bool,
    invalid: bool,
    warning: bool,
}

/// Derives `id`'s Inspector display flags from its own `forced`/`has_error` state and the
/// sheet-wide `status`.
///
/// - Postcondition: `warning` is `false` whenever `invalid` is `true` — a field never
///   shows both states at once.
fn cell_flags(id: CellId, forced: bool, has_error: bool, status: &OutputStatus) -> CellFlags {
    let disabled = forced || (status.has_outputs && !status.relevant.contains(&id));
    let invalid = has_error || status.invalid_outputs.contains(&id);
    let warning = !invalid && status.warning.contains(&id);
    CellFlags {
        disabled,
        invalid,
        warning,
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p begin --no-default-features cell_flags_`
Expected: PASS (8 passed)

- [ ] **Step 6: Compute `OutputStatus` once in `Inspector` and pass it to `CellRow`**

Replace the `Inspector` component (current lines 16-34) with:

```rust
#[component]
pub fn Inspector(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<crate::demo_source::ActiveSource>,
) -> Element {
    let ids: Vec<CellId> = labels.read().cells.keys().copied().collect();
    let output_status = use_memo(move || compute_output_status(&sheet.read()));

    rsx! {
        div {
            style: "width: 260px; min-width: 260px; height: 100%; overflow-y: auto; padding: 12px; box-sizing: border-box;",
            SpHeading { "Cells" }
            SpDivider {}
            for id in ids {
                CellRow { key: "{id:?}", id, sheet, labels, active_source, output_status }
            }
        }
    }
}
```

- [ ] **Step 7: Update `CellRow`'s signature to call `cell_flags`**

Replace the `CellRow` component's signature and the block between the existing `forced` memo (current lines 61-65) and the `field_id` line (current line 76) with:

```rust
#[component]
fn CellRow(
    id: CellId,
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<crate::demo_source::ActiveSource>,
    output_status: Memo<OutputStatus>,
) -> Element {
    let label = use_memo(move || {
        labels
            .read()
            .cells
            .get(&id)
            .map(|m| m.label.clone())
            .unwrap_or_default()
    });

    let value = use_memo(move || {
        let s = sheet.read();
        let l = labels.read();
        l.cells
            .get(&id)
            .map(|m| (m.display)(&s))
            .unwrap_or_default()
    });

    let forced = use_memo(move || sheet.read().is_forced(id));

    let mut input = use_signal(|| value.peek().clone());
    let mut is_focused = use_signal(|| false);
    let mut has_error = use_signal(|| false);

    let flags = use_memo(move || {
        cell_flags(id, *forced.read(), *has_error.read(), &output_status.read())
    });

    // Sync input to the computed value whenever it changes, but not while the user
    // is actively editing — that would interrupt mid-value typing (e.g. "1." → "1").
    use_effect(move || {
        let v = value.read().clone();
        if !*is_focused.read() {
            input.set(v);
        }
    });

    let field_id = format!("cell-{id:?}");
```

- [ ] **Step 8: Wire the three flags into `SpTextfield`**

Update the `SpTextfield` call (current lines 82-95) to:

```rust
            SpTextfield {
                id: field_id,
                value: input.read().clone(),
                invalid: flags.read().invalid,
                warning: flags.read().warning,
                disabled: flags.read().disabled,
                // Dioxus's event serializer only reads event.target.value for
                // HTMLInputElement — custom elements (sp-textfield) always give "".
                // Use dioxus.send() in JS and eval.recv() to read the live value.
                oninput: move |_: FormEvent| {
```

(leave the rest of the `oninput`/`onfocus`/`onblur` block unchanged — `has_error.set(...)` inside it still drives `flags`/`invalid` correctly, since `flags`'s memo reads `*has_error.read()`.)

- [ ] **Step 9: Build to verify it compiles**

Run: `cargo build -p begin --no-default-features`
Expected: builds cleanly, zero warnings.

- [ ] **Step 10: Run the begin test suite**

Run: `cargo test -p begin --no-default-features`
Expected: all tests pass, including the 8 new `cell_flags_*` tests from Step 5 and every existing test (none reference `CellRow`'s internals directly).

- [ ] **Step 11: Commit**

```bash
git add begin/src/inspector.rs
git commit -m "$(cat <<'EOF'
feat(begin): disable don't-care fields and flag out-cell violations

Inspector now computes one OutputStatus snapshot per render (cells
relevant to any out cell, cells implicated in a failing condition,
and which out cells are currently invalid) and shares it across every
CellRow. The pure, unit-tested cell_flags derives disabled/invalid/
warning from a cell's own forced/error state plus that snapshot: a
field is disabled once the sheet has an out cell and the field isn't
currently relevant to any of them (conditional match cells are always
exempted, since contributing_cells never traces through them); an out
cell's own field turns invalid when one of its conditions fails; a
contributing field gets the softer warning state when it feeds a
currently-failing condition.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: Manual verification against `begin/examples/out-cell.adm2`

**Files:** none (verification only; may produce a follow-up fix to `begin/assets/inspector.css`'s custom-property name from Task 5).

**Interfaces:**
- Consumes: everything from Tasks 1-6.
- Produces: confirmation that the three field states actually render as designed, or a corrected custom-property name in `inspector.css` if the Task 5 guess was wrong.

`begin/examples/out-cell.adm2` (not part of the demo picker — loaded via "Open…"):

```text
sheet out_cell {
    cell a = 0.0;
    cell b = 0.0;
    cell c = 0.0;
    cell p = false;

    conditional p {
        true => {
            relationship {
                method [c] -> [b] { c }
            }
        }
    }

    out result {
        method [a, b] { a + b }

        condition min_a [a, b] { a <= b }
    }
}
```

With `p = false` (the file's default), `result`'s writer inputs `a`/`b` are
both plain sources, so `output_relevant_cells()` is `{a, b}` and `c` is the
only "don't care" cell; `p`'s match-cell field must stay enabled regardless
(per the carve-out in Task 6).

- [ ] **Step 1: Invoke the `verifying-begin-ui` skill**

Follow `.claude/skills/verifying-begin-ui/SKILL.md` to serve `begin` as a
web app and drive headless Edge against it.

- [ ] **Step 2: Load the example and screenshot the default state**

Using the running app: click "Open…", pick `begin/examples/out-cell.adm2`.
Screenshot the Inspector. Confirm:
- `a` and `b` are enabled (they're in `output_relevant_cells()`).
- `c` is disabled, showing its last value (`0`) — it's a "don't care" cell.
- `p` is enabled (match-cell carve-out) despite not appearing in
  `output_relevant_cells()`.
- `result` is disabled (already forced, unchanged behavior) and not
  invalid (`min_a`: `0 <= 0` holds).

- [ ] **Step 3: Trigger the violated condition and screenshot**

Set `a` to `5` (making `a <= b` false since `b` is `0`). Confirm:
- `result`'s field turns invalid (red ring).
- `a`'s field (an input to the failing `min_a` condition, per
  `condition_contributing_cells`) shows the warning treatment, not the
  invalid one.
- `b`'s field also shows the warning treatment (it's the condition's other
  input).

If the warning color doesn't visibly change from the field's normal
border, use the skill's live computed-style query against the `sp-textfield`
element's shadow DOM to find the actual custom property `sp-textfield`
resolves its border color from, and correct `begin/assets/inspector.css`
accordingly (Task 5's `--spectrum-textfield-border-color` name was a
starting guess, not confirmed against the real component).

- [ ] **Step 4: Toggle the conditional branch and re-screenshot**

Set `p` to `true` (the field must still be enabled to do this). Confirm:
- `b` becomes disabled (now forced from `c`, via the existing forced-cell
  behavior — unchanged by this plan).
- `c` becomes enabled (it's now a root source feeding `result` through
  `b`, so it's in `output_relevant_cells()` and no longer "don't care").

- [ ] **Step 5: Fix the violation and re-screenshot**

Set `a` back to `0`. Confirm `result` and the warning-flagged fields all
return to their normal state.

- [ ] **Step 6: Commit any correction from Step 3**

Only if Step 3 required changing `inspector.css`:

```bash
git add begin/assets/inspector.css
git commit -m "$(cat <<'EOF'
fix(begin): correct the warning field's border-color custom property

Confirmed against the rendered sp-textfield shadow DOM via the
verifying-begin-ui skill; the name guessed in the original commit
didn't match what SWC actually resolves.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: Full workspace verification

**Files:** none (verification only).

**Interfaces:**
- Consumes: everything from Tasks 1-7.
- Produces: confirmation the branch meets root `CLAUDE.md`'s "Before creating a PR" checklist.

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Expected: no changes (already formatted per-task), or if it reformats something, stage and include it in the commit below.

- [ ] **Step 2: Build the whole workspace**

Run: `cargo build --workspace`
Expected: builds cleanly, zero warnings.

- [ ] **Step 3: Run the full test suite**

Run: `cargo test --workspace`
Run: `cargo test --doc --workspace`
Expected: all pass, no regressions anywhere in the workspace.

- [ ] **Step 4: Lint the whole workspace**

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Run: `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
Run: `cargo clippy -p begin --all-targets -- -D warnings`
Expected: no warnings from any of the three invocations.

- [ ] **Step 5: Commit any formatting fixes (only if Step 1 produced changes)**

```bash
git add -A
git commit -m "$(cat <<'EOF'
style: cargo fmt

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```
