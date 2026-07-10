# `begin`: Surface Forced Cells in the UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Query `property_model::Sheet::is_forced`/`forced_cells` after every `propagate()` in the `begin` demo app, disable Inspector fields for forced cells, highlight forced cells and their producing edge in the D3 graph, and extend the demo source with a conditional relationship that forces a cell.

**Architecture:** The Inspector reads `Sheet::is_forced` directly per cell (no new plumbing); the D3 graph gets a new `GraphData::forced: Vec<String>` field populated from `Sheet::forced_cells()`, consumed by `graph.js`/`graph.css` the same way the existing `changed` field drives the pulse animation. The demo source (`DEMO_SOURCE` in `begin/src/app.rs`) gains one cell and one single-method relationship inside an existing conditional branch, so toggling `p` also toggles a forced cell.

**Tech Stack:** Rust, Dioxus 0.7 (`begin` crate), `property-model` crate (already exposes `is_forced`/`forced_cells`), D3.js v7 (`begin/assets/graph.js`), plain CSS (`begin/assets/graph.css`).

## Global Constraints

- `cargo fmt --all` must be run before every commit (enforced by pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin -- -D warnings` and `cargo clippy -p begin --no-default-features -- -D warnings` must both be clean before the branch is considered done.
- Every function needs a `///` contract-style doc comment (summary, preconditions/postconditions only where non-obvious, `Complexity` bullet whenever not O(1)).
- Unit tests are derived from the contract/public interface only, never from implementation details.
- Never commit directly to `main`; this work happens on the `worktree-forced-to-disabled` branch.

---

### Task 1: `GraphData` reports forced cells

**Files:**
- Modify: `begin/src/bridge.rs:186-197` (`GraphData` struct), `begin/src/bridge.rs:344-352` (`to_graph_data` tail)
- Test: `begin/src/bridge.rs` (`#[cfg(test)] mod tests`, same file)

**Interfaces:**
- Consumes: `property_model::Sheet::forced_cells(&self) -> impl Iterator<Item = CellId> + '_` (already implemented in `property-model/src/sheet.rs:692`); the existing private `cell_node_id(id: CellId) -> String` helper in `bridge.rs:199-201`.
- Produces: `GraphData::forced: Vec<String>` — stable cell-node IDs (`"c{ffi}"`) of cells forced as of the last `propagate()`. Later tasks (Task 3) consume this field by name.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block at the bottom of `begin/src/bridge.rs` (after the existing `sheet_with_conditional` helper, before its first use):

```rust
    fn sheet_with_forced_conditional() -> (Sheet, Labels) {
        let mut sheet = Sheet::new();
        let mut labels = Labels::new();

        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        let b = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(b, "b");
        let p = sheet.add_cell(0_i32);
        labels.add_cell::<i32>(p, "p");

        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |v: &f64| Ok(*v))])
            .unwrap();

        sheet
            .add_conditional(p, vec![(vec![0_i32], vec![rel])], vec![])
            .unwrap();

        (sheet, labels)
    }

    #[test]
    fn to_graph_data_forced_field_contains_forced_cell() {
        let (mut sheet, labels) = sheet_with_forced_conditional();
        sheet.propagate().unwrap();

        let b_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("b"))
            .unwrap();

        let data = to_graph_data(&sheet, &labels);
        assert!(data.forced.contains(&cell_node_id(b_id)));
    }

    #[test]
    fn to_graph_data_forced_field_excludes_cell_when_branch_inactive() {
        let (mut sheet, labels) = sheet_with_forced_conditional();
        let p_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("p"))
            .unwrap();
        sheet.write(p_id, 1_i32).unwrap();
        sheet.propagate().unwrap();

        let b_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("b"))
            .unwrap();

        let data = to_graph_data(&sheet, &labels);
        assert!(!data.forced.contains(&cell_node_id(b_id)));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p begin --no-default-features to_graph_data_forced_field`
Expected: compile error — `no field \`forced\` on type \`GraphData\`` (the struct literal at the end of `to_graph_data` doesn't build one yet, and the test reads `data.forced`).

- [ ] **Step 3: Add the `forced` field and populate it**

In `begin/src/bridge.rs`, add a field to `GraphData` (after the existing `changed` field, `bridge.rs:192-193`):

```rust
    /// Stable IDs of cells that changed during the last `propagate()` call.
    pub changed: Vec<String>,
    /// Stable IDs of cells forced by an active relationship (see
    /// [`property_model::Sheet::is_forced`]); consumers should disable input for these
    /// cells and may render them distinctly.
    pub forced: Vec<String>,
```

At the tail of `to_graph_data` (`bridge.rs:344-352`), populate it alongside `changed`:

```rust
    let changed = sheet.changed().map(cell_node_id).collect();
    let forced = sheet.forced_cells().map(cell_node_id).collect();

    GraphData {
        nodes,
        links,
        changed,
        forced,
        arrows,
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p begin --no-default-features to_graph_data_forced_field`
Expected: PASS (2 passed)

Run: `cargo test -p begin --no-default-features`
Expected: all existing `bridge.rs` tests still pass (the new field doesn't change any existing assertion, since none of them check `GraphData`'s field set directly except `to_graph_data_no_groups_field`, which only asserts the JSON doesn't contain `"groups"` — unaffected by adding `forced`).

- [ ] **Step 5: Commit**

```bash
git add begin/src/bridge.rs
git commit -m "$(cat <<'EOF'
feat(begin): report forced cells in GraphData

GraphData::forced mirrors the existing `changed` field, populated from
Sheet::forced_cells(), so graph_view/graph.js can highlight cells (and
their producing edge) that an active relationship guarantees will
always be overwritten by propagate().

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Inspector disables forced fields

**Files:**
- Modify: `begin/src/spectrum.rs:37-57` (`SpTextfield`)
- Modify: `begin/src/inspector.rs:43-95` (`CellRow`)

**Interfaces:**
- Consumes: `property_model::Sheet::is_forced(&self, id: CellId) -> bool` (already implemented, `property-model/src/sheet.rs:682`).
- Produces: `SpTextfield { disabled: bool, .. }` — a new required prop, mapped to the `disabled` boolean attribute. `CellRow` now disables its field whenever the cell is forced. No other task depends on new names from this task.

This task has no dedicated Rust unit test: `spectrum.rs` wraps a single custom element with no test infrastructure today (consistent with the rest of the file), and `CellRow`'s wiring is a one-line prop pass-through. Verify by building and, in Task 3's manual check, observing the field actually disable.

- [ ] **Step 1: Add the `disabled` prop to `SpTextfield`**

In `begin/src/spectrum.rs`, replace the `SpTextfield` component (lines 32-57):

```rust
/// Single-line text input.
///
/// Maps to `<sp-textfield>`. Fires standard DOM `input`, `focus`, and `blur`
/// events. Setting `invalid` to `true` renders the SWC error state (red ring
/// and `aria-invalid`). Setting `disabled` to `true` renders the SWC disabled
/// state and blocks focus/input at the DOM level.
#[component]
pub fn SpTextfield(
    id: String,
    value: String,
    invalid: bool,
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
            oninput: move |e| oninput.call(e),
            onfocus: move |e| onfocus.call(e),
            onblur: move |e| onblur.call(e),
        }
    }
}
```

- [ ] **Step 2: Wire `CellRow` to disable forced cells**

In `begin/src/inspector.rs`, add a memo next to the existing `value` memo (after `inspector.rs:61-68`, before `let mut input = ...` at `inspector.rs:70`):

```rust
    let forced = use_memo(move || sheet.read().is_forced(id));
```

Then update the `SpTextfield` call (`inspector.rs:89-95`) to pass it through:

```rust
            SpTextfield {
                id: field_id,
                value: input.read().clone(),
                invalid: *has_error.read(),
                disabled: *forced.read(),
                // Dioxus's event serializer only reads event.target.value for
                // HTMLInputElement — custom elements (sp-textfield) always give "".
                // Use dioxus.send() in JS and eval.recv() to read the live value.
                oninput: move |_: FormEvent| {
```

(leave the rest of the `oninput`/`onfocus`/`onblur` block unchanged).

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo build -p begin --no-default-features`
Expected: builds cleanly, no warnings.

- [ ] **Step 4: Commit**

```bash
git add begin/src/spectrum.rs begin/src/inspector.rs
git commit -m "$(cat <<'EOF'
feat(begin): disable Inspector fields for forced cells

A forced cell's value is always overwritten by an active relationship
on the next propagate(), regardless of what the user types, so the
field is disabled rather than silently discarding edits.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Graph highlights forced cells and their producing edge

**Files:**
- Modify: `begin/assets/graph.js:220-251` (near the existing inactive-relationship dimming block, inside `update()`)
- Modify: `begin/assets/graph.css:60-63` (end of file)

**Interfaces:**
- Consumes: `GraphData.forced: Vec<String>` (Task 1) as `data.forced` in JS.
- Produces: CSS classes `forced` (on cell `<rect>`s) and `forced-edge` (on constraint `<line>`s) — purely visual, no other task depends on these names.

- [ ] **Step 1: Build the forced set and toggle CSS classes**

In `begin/assets/graph.js`, inside `update()`, immediately after the existing inactive-relationship IIFE (the block ending at line 251, right before the `// NEW: Conditional diamond nodes` comment at line 253), add:

```javascript
        // Highlight forced cells (see property_model::Sheet::is_forced) and the
        // constraint edge that produces each one. Forced cells always belong to a
        // currently active relationship, so this never overlaps with the inactive-
        // relationship dimming above.
        (function () {
            var forcedSet = new Set(data.forced || []);
            cellLayer.selectAll('rect')
                .classed('forced', function (d) { return forcedSet.has(d.id); });
            linkLayer.selectAll('line')
                .classed('forced-edge', function (d) {
                    var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                    return forcedSet.has(tgtId);
                });
        }());
```

- [ ] **Step 2: Add the CSS rules**

In `begin/assets/graph.css`, after the existing `.link-control` rule (end of file, lines 60-63), add:

```css
.node-cell.forced {
    stroke: #8e44ad;
    stroke-width: 3;
}

.link.forced-edge {
    stroke: #8e44ad;
    stroke-width: 3;
}
```

- [ ] **Step 3: Manually verify in the running app**

Run: `dx serve --platform desktop` (from the `begin/` directory)
Steps:
1. Wait for the desktop window to open with the default demo graph.
2. In the Inspector, set `p` to `1` and confirm (from Task 4, once applied) `g`'s field becomes disabled and the `g` cell rect plus its incoming edge from the `[c] -> [g]` relationship turn purple with a thicker outline.
3. Set `p` back to `0` and confirm `g`'s field re-enables and the highlight disappears.

(This step is a manual check, not a `- [ ] Commit` gate on its own — it's re-run at the end of Task 4 once the demo source actually has `g`. Proceed to commit this task's JS/CSS changes now; the end-to-end visual behavior is confirmed once Task 4 lands.)

- [ ] **Step 4: Commit**

```bash
git add begin/assets/graph.js begin/assets/graph.css
git commit -m "$(cat <<'EOF'
feat(begin): highlight forced cells and their producing edge in the graph

Cells reported by GraphData::forced (see Task 1) get a distinct purple
outline, along with the constraint edge that feeds them, so it's
visible at a glance which cells can never be edited.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Demo source gains a conditional relationship that forces a cell

**Files:**
- Modify: `begin/src/app.rs:11-49` (`DEMO_SOURCE` and its doc comment)
- Test: `begin/src/app.rs` (new `#[cfg(test)] mod tests` block)

**Interfaces:**
- Consumes: `property_model::Sheet::is_forced`, `Sheet::write`, `Sheet::propagate`, `Sheet::cells` (all existing); `crate::source_panel::build_sheet` (already imported in `app.rs`).
- Produces: `DEMO_SOURCE` now declares a cell named `"g"`; no other task depends on this by name, but Task 3's manual verification (Step 3 above) exercises it end-to-end.

- [ ] **Step 1: Write the failing tests**

Add a new `#[cfg(test)] mod tests` block at the end of `begin/src/app.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_source_g_not_forced_when_p_is_zero() {
        let outcome = build_sheet(DEMO_SOURCE);
        let (sheet, labels) = outcome.sheet_labels.expect("DEMO_SOURCE must build");
        let g_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("g"))
            .unwrap();
        assert!(!sheet.is_forced(g_id), "g should not be forced when p == 0");
    }

    #[test]
    fn demo_source_g_forced_when_p_is_one() {
        let outcome = build_sheet(DEMO_SOURCE);
        let (mut sheet, labels) = outcome.sheet_labels.expect("DEMO_SOURCE must build");
        let p_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("p"))
            .unwrap();
        let g_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("g"))
            .unwrap();

        sheet.write(p_id, 1_i32).unwrap();
        sheet.propagate().unwrap();

        assert!(sheet.is_forced(g_id), "g should be forced when p == 1");
    }

    #[test]
    fn demo_source_g_unforced_again_after_p_returns_to_zero() {
        let outcome = build_sheet(DEMO_SOURCE);
        let (mut sheet, labels) = outcome.sheet_labels.expect("DEMO_SOURCE must build");
        let p_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("p"))
            .unwrap();
        let g_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("g"))
            .unwrap();

        sheet.write(p_id, 1_i32).unwrap();
        sheet.propagate().unwrap();
        sheet.write(p_id, 0_i32).unwrap();
        sheet.propagate().unwrap();

        assert!(!sheet.is_forced(g_id), "g should not be forced once p == 0 again");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p begin --no-default-features demo_source_g`
Expected: FAIL — `called \`Option::unwrap()\` on a \`None\` value` (no cell named `"g"` exists in `DEMO_SOURCE` yet).

- [ ] **Step 3: Update `DEMO_SOURCE`**

Replace `begin/src/app.rs:11-49` with:

```rust
/// Default pm-lang source: two independent bidirectional constraint systems
/// (`a × b = c` and `d × e = f`) linked by a conditional on `p`.
///
/// - `p = 0`: the relationship `c = f` (bidirectional) becomes active.
/// - `p = 1`: the relationship `c = f × 2` (bidirectional) becomes active, and a
///   single-method relationship `g = c × 10` also becomes active — `g` is *forced*
///   while this branch is active (see [`property_model::Sheet::is_forced`]), so its
///   Inspector field is disabled and it is highlighted in the graph.
/// - Any other `p`: the two systems are independent and `g` is not forced.
pub const DEMO_SOURCE: &str = r#"sheet demo {
    cell a: f64 = 2.0;
    cell b: f64 = 3.0;
    cell c: f64;
    cell d: f64 = 4.0;
    cell e: f64 = 5.0;
    cell f: f64;
    cell g: f64;
    cell p: i32 = 0;

    relationship {
        method [a, b] -> [c] { a * b }
        method [b, c] -> [a] { c / b }
        method [a, c] -> [b] { c / a }
    }

    relationship {
        method [d, e] -> [f] { d * e }
        method [e, f] -> [d] { f / e }
        method [d, f] -> [e] { f / d }
    }

    conditional p {
        0i32 => {
            method [f] -> [c] { f }
            method [c] -> [f] { c }
        }
        1i32 => {
            method [f] -> [c] { f * 2.0 }
            method [c] -> [f] { c / 2.0 }
            method [c] -> [g] { c * 10.0 }
        }
    }
}
"#;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p begin --no-default-features demo_source_g`
Expected: PASS (3 passed)

Run: `cargo test -p begin --no-default-features`
Expected: all tests pass, including `app.rs`'s new tests and the existing `bridge.rs`/`inspector.rs`/`source_panel.rs` suites (none reference `DEMO_SOURCE`'s exact cell set, so none regress).

- [ ] **Step 5: Manually re-verify the graph end-to-end**

Repeat Task 3 Step 3 (`dx serve --platform desktop`) now that `g` actually exists: toggling `p` between `0` and `1` should disable/highlight `g` as described.

- [ ] **Step 6: Commit**

```bash
git add begin/src/app.rs
git commit -m "$(cat <<'EOF'
feat(begin): add a forced cell to the demo source

DEMO_SOURCE's p == 1 branch now also activates a single-method
relationship g = c * 10, so g is forced (per Sheet::is_forced) only
while that branch is active — exercising the Inspector-disable and
graph-highlight behavior added in prior tasks.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Full workspace verification

**Files:** none (verification only)

**Interfaces:**
- Consumes: everything from Tasks 1-4.
- Produces: nothing new; confirms the branch is ready to hand off per root `CLAUDE.md`'s "Before creating a PR" checklist.

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Expected: no changes (already formatted per-task), or if it does reformat something, stage and include it in the commit below.

- [ ] **Step 2: Build the whole workspace**

Run: `cargo build --workspace`
Expected: builds cleanly, zero warnings.

- [ ] **Step 3: Run the full test suite**

Run: `cargo test --workspace`
Run: `cargo test --doc --workspace`
Expected: all pass, no regressions anywhere in the workspace.

- [ ] **Step 4: Lint the whole workspace**

Run: `cargo clippy --workspace --exclude begin -- -D warnings`
Run: `cargo clippy -p begin --no-default-features -- -D warnings`
Expected: no warnings from either invocation.

- [ ] **Step 5: Commit any formatting fixes (only if Step 1 produced changes)**

```bash
git add -A
git commit -m "$(cat <<'EOF'
style: cargo fmt

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```
