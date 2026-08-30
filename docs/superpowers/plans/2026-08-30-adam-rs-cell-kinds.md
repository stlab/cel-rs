# Cell Kinds (`source`, non-terminal `out`, generalized `filter`/`require`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `CellKind` (`Cell`/`Source`/`Out`) to `adam-rs`, weaken `out` from terminal to merely non-writable/single-writer, add a new always-source `source` cell kind, generalize `filter` (now named) and `require` (now multi-attachable) to every cell kind, and carry all of this through `adam-lang`'s grammar/parser/formatter/typechecker, `adam-web-ui`'s Inspector, the VS Code extension, and the live `adam-lang-book`.

**Architecture:** `adam-rs` cells already carry a per-round `source`/`derived` split; `Source` and `Out` just pin a cell permanently to one side instead of letting the planner choose per round, and `CellKind::Out` subsumes today's separate `OutputId`/`OutputData` handle type entirely (an out cell is just a `CellId` with `kind == Out`). No planner or propagation-phase changes are needed — the diagnostic phase that runs once per round for `Requirement` (today `out`-only) generalizes to run for every cell with requirements attached, and `Filter`'s existing per-round dispatch (self-correct for a source, diagnose for a derived value) already produces the right behavior on `Source`/`Out` cells once the preconditions blocking them are removed.

**Tech Stack:** Rust, `slotmap`, `anyhow`, `cel_parser`/`cel_runtime` (adam-lang's expression layer), `dioxus` (adam-web-ui).

**Spec:** [docs/superpowers/specs/2026-08-29-adam-rs-cell-kinds-design.md](../specs/2026-08-29-adam-rs-cell-kinds-design.md)

## Global Constraints

- Format with `cargo fmt --all` before every commit (enforced by a pre-commit hook).
- Every new/changed function gets a `///` doc comment in contract style: present-tense summary; `- Precondition:` bullets for non-obvious preconditions (checked with `debug_assert!`, never documented as causing a specific failure); `# Errors` for `Err`-returning conditions; `- Postcondition:` bullets where not implicit; `- Complexity:` bullet whenever not O(1).
- Unit tests are derived from the contract and public interface only — never from reading the implementation. Precondition violations are never tested.
- Arithmetic on signed integers uses `checked_*`, never wrapping. Fallible operations return `Result`.
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings; `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` and the two `begin`-specific clippy invocations must pass before a PR.
- No migration/back-compat shims — this project has no clients yet (root `CLAUDE.md`). Renamed/removed APIs are renamed/removed outright, not deprecated.
- `require` never gates `write()`, on any cell kind (spec §2). `add_requirement`'s attach-time hard-fail check runs only for `Cell`/`Source` kind cells, never for `Out`-kind cells (spec §2, §4.4).

---

## Task 1: `CellKind` and `add_source`

**Files:**
- Modify: `adam-rs/src/cell.rs`
- Modify: `adam-rs/src/sheet.rs:104-146` (`Sheet::new`, `Sheet::add_cell`)
- Modify: `adam-rs/src/lib.rs` (re-export `CellKind`)

**Interfaces:**
- Produces: `pub enum CellKind { Cell, Source, Out }` (re-exported from `adam_rs`), `CellData::kind: CellKind`, `CellData::requirements: Vec<RequirementId>` (unused until Task 5, added now so `CellData`'s shape only changes once), `Sheet::add_source<T: Any + PartialEq + 'static>(&mut self, value: T) -> CellId`, `Sheet::cell_kind(&self, id: CellId) -> Option<CellKind>`.
- Consumes: nothing new.

- [ ] **Step 1: Write the failing tests**

Add to `adam-rs/src/sheet.rs`'s `mod tests` (near the other `add_cell`-adjacent tests):

```rust
#[test]
fn add_cell_has_cell_kind() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    assert_eq!(sheet.cell_kind(a), Some(CellKind::Cell));
}

#[test]
fn add_source_has_source_kind() {
    let mut sheet = Sheet::new();
    let a = sheet.add_source(0_i32);
    assert_eq!(sheet.cell_kind(a), Some(CellKind::Source));
}

#[test]
fn cell_kind_returns_none_for_invalid_id() {
    let mut sheet = Sheet::new();
    sheet.add_cell(0_i32); // occupies slotmap index 0 in `sheet`
    let mut other = Sheet::new();
    other.add_cell(0_i32); // index 0 in `other`
    let bogus = other.add_cell(0_i32); // index 1 in `other` -- out of range for `sheet`,
                                        // which only ever allocated index 0, so this is
                                        // guaranteed invalid regardless of generation
                                        // (a same-index key from a second fresh SlotMap
                                        // would otherwise collide with `sheet`'s own key)
    assert_eq!(sheet.cell_kind(bogus), None);
}
```

Add to `adam-rs/src/cell.rs`'s `mod tests`, updating the existing `cell_data_initial_state` test (it constructs a `CellData` literal directly and will fail to compile once `kind`/`requirements` are added):

```rust
#[test]
fn cell_data_initial_state() {
    let data = CellData {
        source: Box::new(42_i32),
        derived: None,
        type_id: TypeId::of::<i32>(),
        strength: 0,
        changed: false,
        adj: vec![],
        eq_fn: |a, b| a.downcast_ref::<i32>() == b.downcast_ref::<i32>(),
        filter: None,
        kind: CellKind::Cell,
        requirements: Vec::new(),
    };
    assert_eq!(data.type_id, TypeId::of::<i32>());
    assert_eq!(data.strength, 0);
    assert!(!data.changed);
    assert!(data.adj.is_empty());
    assert!(data.derived.is_none());
    assert_eq!(*data.source.downcast_ref::<i32>().unwrap(), 42);
    assert_eq!(*data.effective().downcast_ref::<i32>().unwrap(), 42);
    assert_eq!(data.kind, CellKind::Cell);
    assert!(data.requirements.is_empty());
    let x: i32 = 42;
    let y: i32 = 99;
    assert!((data.eq_fn)(&x, &x));
    assert!(!(data.eq_fn)(&x, &y));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs cell_kind -- --nocapture` and `cargo test -p adam-rs cell_data_initial_state`
Expected: compile errors — `CellKind` doesn't exist, `CellData` has no `kind`/`requirements` fields, `Sheet` has no `add_source`/`cell_kind`.

- [ ] **Step 3: Implement `CellKind` and wire it through**

In `adam-rs/src/cell.rs`, add near the top (after the `new_key_type!` block) and update `CellData`:

```rust
/// A cell's fixed role in the planner's per-round source/derived assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    /// May be a source or derived, chosen per round by the planner. Default kind.
    Cell,
    /// Always a source: never claimable as any method's output.
    Source,
    /// Always derived by exactly one fixed writer method; never `write()`-able.
    Out,
}
```

Add two fields to `CellData` (after `filter`):

```rust
    /// This cell's fixed role in the planner's per-round source/derived assignment.
    pub(crate) kind: CellKind,
    /// This cell's requirements, in attachment order. Empty for most cells.
    pub(crate) requirements: Vec<RequirementId>,
```

Add the import `use crate::requirement::RequirementId;` to `cell.rs`'s existing `use` block.

In `adam-rs/src/sheet.rs`, update `add_cell`'s `CellData` literal (line ~136-145) to add `kind: CellKind::Cell, requirements: Vec::new(),`, and add `use crate::cell::CellKind;` to the existing `use crate::{ cell::{CellData, CellId}, ... }` import block. Then add two new public methods (near `add_cell`):

```rust
/// Registers a cell that can never be claimed as any method's output — always a
/// planner source, forever.
///
/// - Complexity: O(1).
pub fn add_source<T: Any + PartialEq + 'static>(&mut self, value: T) -> CellId {
    let id = self.add_cell(value);
    self.cells[id].kind = CellKind::Source;
    id
}

/// Returns `id`'s fixed cell kind.
///
/// Returns `None` if `id` is not a live cell in this sheet.
pub fn cell_kind(&self, id: CellId) -> Option<CellKind> {
    self.cells.get(id).map(|c| c.kind)
}
```

In `adam-rs/src/lib.rs`, add `CellKind` to the existing `pub use cell::{...}` re-export line.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-rs cell_kind add_source cell_data_initial_state`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/cell.rs adam-rs/src/sheet.rs adam-rs/src/lib.rs
git commit -m "feat(adam-rs): add CellKind and Sheet::add_source"
```

---

## Task 2: Rename `Error::TerminalCell` to `InvalidCellKind`; add `InvalidRequirement`

**Files:**
- Modify: `adam-rs/src/error.rs`
- Modify: `adam-rs/src/sheet.rs` (every `Error::TerminalCell` construction site, updated in place; the checks themselves are reworked in Task 3)

**Interfaces:**
- Produces: `Error::InvalidCellKind`, `Error::InvalidRequirement` (both `adam_rs::Error` variants).
- Consumes: nothing new.

- [ ] **Step 1: Write the failing test**

Add to `adam-rs/src/sheet.rs`'s `mod tests`, replacing the existing `add_relationship_returns_terminal_cell_for_terminal_input` test's assertion (find it via its current body — it does `sheet.terminal_cells.insert(a);` then asserts `Error::TerminalCell`) with the renamed variant name only — this is a mechanical rename, not new coverage, since Task 3 replaces the `terminal_cells` set entirely:

```rust
#[test]
fn error_variant_is_invalid_cell_kind_not_terminal_cell() {
    // Compile-time check that the rename landed; exercised for real once Task 3
    // wires up the CellKind-based checks that actually return this variant.
    let _err = Error::InvalidCellKind;
}

#[test]
fn error_has_invalid_requirement_variant() {
    let _err = Error::InvalidRequirement;
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p adam-rs error_variant_is_invalid_cell_kind error_has_invalid_requirement`
Expected: compile error — `Error::InvalidCellKind`/`Error::InvalidRequirement` don't exist yet.

- [ ] **Step 3: Rename the variant and add the new one**

In `adam-rs/src/error.rs`, find the `TerminalCell` variant and replace it:

```rust
/// A relationship or conditional attempted to claim a `Source`-kind cell as a
/// method's output, `write()` targeted an `Out`-kind cell, or `add_out` targeted a
/// cell that is already `Source`/`Out` kind or already claimed as another method's
/// output.
InvalidCellKind,
```

Add a new variant alongside `InvalidOutput`/`InvalidFilter`:

```rust
/// An `add_requirement` call is structurally invalid: the name is empty, `cell`
/// already has a same-named requirement, or (on a `Cell`/`Source` kind cell)
/// evaluating the requirement against current values returns `Ok(false)`.
InvalidRequirement,
```

In `adam-rs/src/sheet.rs`, do a project-wide rename of every `Error::TerminalCell` construction to `Error::InvalidCellKind`, and every `self.terminal_cells.contains(...)` guard's error return likewise (there are 5 call sites in non-test code: `add_relationship`'s two checks at what are currently lines ~194/207, `add_conditional`'s two checks at ~307/320, and `write`'s check at ~906 — the checks themselves are reworked to use `CellKind` in Task 3, so for this step, only rename the error variant each returns; leave the `terminal_cells.contains` checks as-is for now). Also rename every test function/assertion referencing `Error::TerminalCell` (e.g. `add_relationship_returns_terminal_cell_for_terminal_input`, `add_relationship_returns_terminal_cell_for_terminal_output`, `add_conditional_returns_terminal_cell_for_terminal_match_cell`) to construct/assert `Error::InvalidCellKind` instead — their bodies still use `sheet.terminal_cells.insert(...)` at this step; Task 3 rewrites those bodies to use `CellKind` instead and may delete some of them per the spec's §4.5 (the `add_conditional` match-cell checks are deleted outright, so `add_conditional_returns_terminal_cell_for_terminal_match_cell` is deleted, not renamed, in Task 3 — leave it renamed-but-passing for now).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-rs --lib`
Expected: full `adam-rs` test suite compiles and passes (every `Error::TerminalCell` reference is gone).

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/error.rs adam-rs/src/sheet.rs
git commit -m "refactor(adam-rs): rename Error::TerminalCell to InvalidCellKind, add InvalidRequirement"
```

---

## Task 3: `Source`-kind enforcement, non-terminal `out` input checks, `write()`, `cell_has_prior_use`

**Files:**
- Modify: `adam-rs/src/sheet.rs` (`add_relationship`, `add_conditional`, `write`, `cell_has_prior_use`, `Sheet` struct, `Sheet::new`)

**Interfaces:**
- Consumes: `CellKind` (Task 1), `Error::InvalidCellKind` (Task 2).
- Produces: the `terminal_cells: HashSet<CellId>` field is deleted from `Sheet`; all kind-based checks now read `CellData::kind` directly.

- [ ] **Step 1: Write the failing tests**

Add to `adam-rs/src/sheet.rs`'s `mod tests`. (`add_out` doesn't exist until Task 6, so this task's coverage of the deleted `add_conditional` match-cell check is indirect — a passing `add_conditional` call with no `terminal_cells` set to insert into is sufficient proof the check is gone; Task 6 adds direct out-cell-as-match-subject coverage once `add_out` exists.)

```rust
#[test]
fn add_relationship_returns_invalid_cell_kind_when_a_source_cell_is_an_output() {
    let mut sheet = Sheet::new();
    let a = sheet.add_source(0_i32);
    let b = sheet.add_cell(0_i32);
    let result = sheet.add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))]);
    assert!(matches!(result, Err(Error::InvalidCellKind)));
}

#[test]
fn add_relationship_allows_a_source_cell_as_an_input() {
    let mut sheet = Sheet::new();
    let a = sheet.add_source(5_i32);
    let b = sheet.add_cell(0_i32);
    let result = sheet.add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))]);
    assert!(result.is_ok());
}

#[test]
fn write_succeeds_on_a_source_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_source(0_i32);
    assert!(sheet.write(a, 5_i32).is_ok());
}
```

Also delete the now-obsolete `add_conditional_returns_terminal_cell_for_terminal_match_cell` test (it asserts a check §4.5 removes outright) and `add_relationship_returns_terminal_cell_for_terminal_output`'s counterpart for *input*-side terminality if one exists under that name (check for a test asserting `Error::...` for referencing an already-out cell as a relationship *input* — the spec removes that restriction, so delete any test asserting it).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs invalid_cell_kind source_cell`
Expected: `add_relationship_returns_invalid_cell_kind_when_a_source_cell_is_an_output` FAILs (no such check exists yet — `add_relationship` currently only checks `terminal_cells`, which `Source`-kind cells are never added to).

- [ ] **Step 3: Implement the `CellKind`-based checks**

In `adam-rs/src/sheet.rs`, remove the `terminal_cells: HashSet<CellId>` field from the `Sheet` struct and its initialization in `Sheet::new()` (`terminal_cells: HashSet::new(),`).

In `add_relationship`, replace the two existing blocks:

```rust
            for (&cell_id, &declared) in method.inputs.iter().zip(method.input_types.iter()) {
                if self.terminal_cells.contains(&cell_id) {
                    return Err(Error::TerminalCell);
                }
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
```

with (input side: no kind check at all now):

```rust
            for (&cell_id, &declared) in method.inputs.iter().zip(method.input_types.iter()) {
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
```

and:

```rust
            for (&cell_id, &declared) in method.outputs.iter().zip(method.output_types.iter()) {
                if self.terminal_cells.contains(&cell_id) {
                    return Err(Error::TerminalCell);
                }
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
```

with (output side: reject `Source`-kind):

```rust
            for (&cell_id, &declared) in method.outputs.iter().zip(method.output_types.iter()) {
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.kind == CellKind::Source {
                    return Err(Error::InvalidCellKind);
                }
```

(Note the `InvalidId` check now needs to run before the kind check since it borrows `cell`; keep the type-mismatch check that follows unchanged.)

In `add_conditional`, delete both `terminal_cells.contains` blocks outright (spec §4.5 — a match subject is a read, no longer restricted for any kind):

```rust
            MatchSource::Cell(cell) => {
                let cell_data = self.cells.get(*cell).ok_or(Error::InvalidId)?;
                if cell_data.type_id != TypeId::of::<T>() {
                    return Err(Error::InvalidConditional);
                }
                vec![*cell]
            }
            MatchSource::Expr(expr) => {
                if expr.output_type != TypeId::of::<T>() {
                    return Err(Error::InvalidConditional);
                }
                for (&cell_id, &declared) in expr.inputs.iter().zip(expr.input_types.iter()) {
                    let cell_data = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                    if cell_data.type_id != declared {
```

(i.e. delete the `if self.terminal_cells.contains(...) { return Err(...); }` block in each arm, keeping everything else unchanged.)

In `write`, replace:

```rust
    pub fn write<T: Any + 'static>(&mut self, id: CellId, value: T) -> Result<(), Error> {
        if self.terminal_cells.contains(&id) {
            return Err(Error::TerminalCell);
        }
```

with:

```rust
    pub fn write<T: Any + 'static>(&mut self, id: CellId, value: T) -> Result<(), Error> {
        if self.cells.get(id).is_some_and(|c| c.kind == CellKind::Out) {
            return Err(Error::InvalidCellKind);
        }
```

Update `write`'s doc comment's `# Errors` line from "`id` already belongs to an existing output" to "`id` is `Out`-kind."

Find `cell_has_prior_use` and replace its body per spec §4.5:

```rust
fn cell_has_prior_use(&self, id: CellId) -> bool {
    self.relationships
        .values()
        .any(|rel| rel.methods.iter().any(|m| m.outputs.contains(&id)))
}
```

(This function isn't called anywhere yet in this task's code — it becomes load-bearing again in Task 6's `add_out`. Leave its existing doc comment; Task 6 updates it to match the new semantics.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-rs --lib`
Expected: full suite passes, including the three new tests and the surviving renamed ones from Task 2.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): enforce Source cells can't be method outputs; drop out-cell input restrictions"
```

---

## Task 4: Named `filter`

**Files:**
- Modify: `adam-rs/src/filter.rs` (`FilterData::name`)
- Modify: `adam-rs/src/sheet.rs` (`add_filter` signature, new `filter_name` query, drop the terminal/kind check inside `add_filter`)

**Interfaces:**
- Produces: `FilterData::name: String`, `Sheet::add_filter(cell: CellId, name: impl Into<String>, filter: Filter) -> Result<(), Error>` (signature change), `Sheet::filter_name(id: CellId) -> Option<&str>`.
- Consumes: `CellKind` (Task 1).

- [ ] **Step 1: Write the failing tests**

Add to `adam-rs/src/sheet.rs`'s `mod tests`:

```rust
#[test]
fn add_filter_stores_and_reports_its_name() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    sheet
        .add_filter(a, "clamp", Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 10))))
        .unwrap();
    assert_eq!(sheet.filter_name(a), Some("clamp"));
}

#[test]
fn filter_name_returns_none_for_an_unfiltered_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    assert_eq!(sheet.filter_name(a), None);
}

#[test]
fn add_filter_returns_invalid_filter_for_an_empty_name() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let result = sheet.add_filter(a, "", Filter::from_fn_0(|x: &i32| Ok(*x)));
    assert!(matches!(result, Err(Error::InvalidFilter)));
}

#[test]
fn add_filter_succeeds_on_a_source_kind_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_source(5_i32);
    assert!(
        sheet
            .add_filter(a, "clamp", Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 10))))
            .is_ok()
    );
}

```

(An `Out`-kind cell doesn't exist yet at this point in the plan — `add_out` lands in Task 6. This task's coverage of the precondition removal is the `Source`-kind test above plus the four `Cell`-kind tests; Task 6 adds its own `add_filter`-on-an-`Out`-cell coverage once a real out cell exists to test against, per its own test list.)

Delete `add_filter_succeeds_on_an_out_kind_cell` for this task (as noted in its own body) — use only the first four tests. Also find and delete the existing test that asserts `add_filter` rejects a terminal cell (something like a test constructing `sheet.terminal_cells.insert(...)` then calling `add_filter` and expecting an error) — that precondition no longer exists once Task 6 lands, but since `terminal_cells` itself was already deleted in Task 3, that test should already be failing to compile; delete it now if it wasn't already caught in Task 3's step 1.

Every existing call site of `sheet.add_filter(cell, filter)` in `adam-rs/src/sheet.rs`'s test module (there are many, e.g. `from_fn_1_stores_correct_type_ids_and_computes_value`-style tests under `mod tests` that call `sheet.add_filter(id, Filter::from_fn_...)`) needs a name argument inserted — e.g. `sheet.add_filter(a, "test_filter", Filter::from_fn_0(...))`. Grep for `.add_filter(` across `adam-rs/src/sheet.rs` and update every call site.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs filter_name add_filter_returns_invalid_filter_for_an_empty_name`
Expected: compile errors (wrong arity for `add_filter`, no `filter_name` method, no `FilterData::name`).

- [ ] **Step 3: Implement**

In `adam-rs/src/filter.rs`, add a `name` field to `FilterData`:

```rust
pub(crate) struct FilterData {
    /// This filter's name, supplied at `Sheet::add_filter`.
    pub(crate) name: String,
    // ...existing fields unchanged (value_type, args, arg_types, function, kind)...
}
```

Every place `FilterData { ... }` is constructed inside `Filter::new`/`Filter::range` (the only two constructors that build a `FilterData` literal; `from_fn_0`/`from_fn_1`/`from_fn_2` delegate to `Filter::new`) needs a `name` field — but `Filter::new`/`Filter::range` don't take a name today, and per spec §3.4/§4.2a the name is supplied separately at `add_filter`, not baked into `Filter`'s constructors. Give `FilterData::name` a placeholder empty default inside `Filter::new`/`Filter::range` (`name: String::new()`), and have `Sheet::add_filter` overwrite it with the real name before storing:

```rust
// In Filter::new and Filter::range's FilterData literals, add:
name: String::new(),
```

In `adam-rs/src/sheet.rs`, change `add_filter`'s signature and body:

```rust
/// Attaches `filter` to `cell` under `name`.
///
/// # Errors
///
/// - `Error::InvalidId` — `cell`, or one of `filter`'s argument cells, is not a
///   live cell in this sheet.
/// - `Error::InvalidFilter` — `name` is empty, `cell` already has a filter,
///   `filter`'s own value type does not match `cell`'s registered type, or
///   `filter`'s argument list names `cell` itself.
/// - `Error::TypeMismatch` — an argument cell's registered type does not match the
///   type `filter` declared for it.
///
/// - Complexity: O(a) where a is the number of `filter`'s argument cells.
pub fn add_filter(
    &mut self,
    cell: CellId,
    name: impl Into<String>,
    mut filter: Filter,
) -> Result<(), Error> {
    let name = name.into();
    if name.is_empty() {
        return Err(Error::InvalidFilter);
    }
    let cell_type = self.cells.get(cell).ok_or(Error::InvalidId)?.type_id;
    if self.cells[cell].filter.is_some() {
        return Err(Error::InvalidFilter);
    }
    if filter.0.value_type != cell_type {
        return Err(Error::InvalidFilter);
    }
    if filter.0.args.contains(&cell) {
        return Err(Error::InvalidFilter);
    }
    for (&arg_id, &declared) in filter.0.args.iter().zip(filter.0.arg_types.iter()) {
        let arg_cell = self.cells.get(arg_id).ok_or(Error::InvalidId)?;
        if arg_cell.type_id != declared {
            return Err(Error::TypeMismatch {
                expected: arg_cell.type_id,
                found: declared,
            });
        }
    }

    filter.0.name = name;
    for &arg in &filter.0.args {
        self.filter_dependents.entry(arg).or_default().push(cell);
    }
    self.cells[cell].filter = Some(filter.0);
    Ok(())
}

/// Returns the name of `id`'s filter, if it has one.
///
/// Returns `None` if `id` is not a live cell in this sheet, or has no filter.
pub fn filter_name(&self, id: CellId) -> Option<&str> {
    self.cells.get(id)?.filter.as_ref().map(|f| f.name.as_str())
}
```

Note the `self.terminal_cells.contains(&cell)` check from the old body is gone entirely (no replacement — spec §4.2a states `add_filter` is unrestricted by kind).

Update every `sheet.add_filter(id, Filter::...)` call site across `adam-rs/src/sheet.rs`'s test module to `sheet.add_filter(id, "some_name", Filter::...)` (any non-empty string is fine; use a descriptive name per test, e.g. `"bound"`, `"range"`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-rs --lib`
Expected: full suite passes.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/filter.rs adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): add_filter takes a name; drop its terminal-cell restriction"
```

---

## Task 5: Generalized `Requirement` storage and `add_requirement`

**Files:**
- Modify: `adam-rs/src/requirement.rs` (`RequirementData::cell` replaces `::output`)
- Modify: `adam-rs/src/sheet.rs` (new `add_requirement`; `CellData::requirements` starts being populated)

**Interfaces:**
- Produces: `RequirementData::cell: CellId` (was `output: OutputId`), `Sheet::add_requirement(cell: CellId, name: impl Into<String>, requirement: Requirement) -> Result<RequirementId, Error>`.
- Consumes: `CellKind` (Task 1), `CellData::requirements` (Task 1), `Error::InvalidRequirement` (Task 2).

- [ ] **Step 1: Write the failing tests**

Add to `adam-rs/src/sheet.rs`'s `mod tests`:

```rust
#[test]
fn add_requirement_succeeds_on_a_plain_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let result = sheet.add_requirement(a, "positive", Requirement::from_fn_1(a, |x: &i32| Ok(*x > 0)));
    assert!(result.is_ok());
}

#[test]
fn add_requirement_returns_invalid_requirement_for_empty_name() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let result = sheet.add_requirement(a, "", Requirement::from_fn_1(a, |x: &i32| Ok(*x > 0)));
    assert!(matches!(result, Err(Error::InvalidRequirement)));
}

#[test]
fn add_requirement_returns_invalid_requirement_for_duplicate_name_on_same_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    sheet
        .add_requirement(a, "positive", Requirement::from_fn_1(a, |x: &i32| Ok(*x > 0)))
        .unwrap();
    let result = sheet.add_requirement(a, "positive", Requirement::from_fn_1(a, |x: &i32| Ok(*x < 100)));
    assert!(matches!(result, Err(Error::InvalidRequirement)));
}

#[test]
fn add_requirement_hard_fails_when_current_value_already_violates_it_on_a_plain_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(-5_i32);
    let result = sheet.add_requirement(a, "positive", Requirement::from_fn_1(a, |x: &i32| Ok(*x > 0)));
    assert!(matches!(result, Err(Error::InvalidRequirement)));
}

#[test]
fn add_requirement_hard_fails_when_current_value_already_violates_it_on_a_source_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_source(-5_i32);
    let result = sheet.add_requirement(a, "positive", Requirement::from_fn_1(a, |x: &i32| Ok(*x > 0)));
    assert!(matches!(result, Err(Error::InvalidRequirement)));
}

#[test]
fn add_requirement_propagates_method_failed_when_evaluation_errors() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let result = sheet.add_requirement(
        a,
        "always_errors",
        Requirement::from_fn_1(a, |_: &i32| Err(anyhow::anyhow!("boom"))),
    );
    assert!(matches!(result, Err(Error::MethodFailed(_))));
}

#[test]
fn cell_has_the_requirement_it_was_given() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let rid = sheet
        .add_requirement(a, "positive", Requirement::from_fn_1(a, |x: &i32| Ok(*x > 0)))
        .unwrap();
    assert_eq!(sheet.cells[a].requirements, vec![rid]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs add_requirement cell_has_the_requirement`
Expected: compile error — `Sheet::add_requirement` doesn't exist yet.

- [ ] **Step 3: Implement**

In `adam-rs/src/requirement.rs`, rename `RequirementData::output` to `cell`:

```rust
pub(crate) struct RequirementData {
    pub(crate) name: String,
    pub(crate) cell: CellId,
    pub(crate) inputs: Vec<CellId>,
    pub(crate) function: RequirementFn,
}
```

Remove the now-unused `use crate::output::OutputId;` import from `requirement.rs` if present (it isn't — `RequirementData` didn't import `OutputId` directly, `Sheet` did; double-check `requirement.rs`'s imports compile clean after the rename).

In `adam-rs/src/sheet.rs`, add:

```rust
/// Attaches a named requirement to `cell`. `requirement.inputs` may be any cells in
/// the sheet, not only `cell` itself. For a `Cell`/`Source` kind `cell`, also
/// evaluates `requirement` immediately against current effective values — its
/// value is already authoritative. Skipped for an `Out`-kind `cell`: its value
/// isn't authoritative until its writer next executes, so attachment always
/// succeeds structurally there, deferring to the first post-`propagate()`
/// diagnostic to report an initial violation if there is one.
///
/// # Errors
///
/// - `Error::InvalidId` — `cell`, or one of `requirement`'s input cells, is not a
///   live cell in this sheet.
/// - `Error::TypeMismatch` — an input's declared type does not match its cell's
///   registered type.
/// - `Error::InvalidRequirement` — `name` is empty, `cell` already has a
///   same-named requirement, or (`Cell`/`Source` kind only) evaluating
///   `requirement` against the referenced cells' current effective values
///   returns `Ok(false)`.
/// - `Error::MethodFailed` — (`Cell`/`Source` kind only) evaluating `requirement`
///   against current values returns `Err`.
///
/// - Complexity: O(k) where k is `requirement`'s input count.
pub fn add_requirement(
    &mut self,
    cell: CellId,
    name: impl Into<String>,
    requirement: Requirement,
) -> Result<RequirementId, Error> {
    let name = name.into();
    if name.is_empty() {
        return Err(Error::InvalidRequirement);
    }
    let cell_data = self.cells.get(cell).ok_or(Error::InvalidId)?;
    if cell_data
        .requirements
        .iter()
        .any(|&rid| self.requirements[rid].name == name)
    {
        return Err(Error::InvalidRequirement);
    }
    if requirement.inputs.len() != requirement.input_types.len() {
        return Err(Error::InvalidRequirement);
    }
    for (&input_id, &declared) in requirement.inputs.iter().zip(requirement.input_types.iter()) {
        let input_cell = self.cells.get(input_id).ok_or(Error::InvalidId)?;
        if input_cell.type_id != declared {
            return Err(Error::TypeMismatch {
                expected: input_cell.type_id,
                found: declared,
            });
        }
    }

    if self.cells[cell].kind != CellKind::Out {
        let inputs: Vec<&dyn Any> = requirement
            .inputs
            .iter()
            .map(|&id| self.cells[id].effective())
            .collect();
        let holds = (requirement.function)(&inputs).map_err(Error::MethodFailed)?;
        if !holds {
            return Err(Error::InvalidRequirement);
        }
    }

    let rid = self.requirements.insert(RequirementData {
        name,
        cell,
        inputs: requirement.inputs,
        function: requirement.function,
    });
    self.cells[cell].requirements.push(rid);
    Ok(rid)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-rs --lib`
Expected: full suite passes.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/requirement.rs adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): add Sheet::add_requirement, generalized off any cell kind"
```

---

## Task 6: `add_out` replaces `add_output`; remove `OutputId`/`OutputData`; rename the query surface; generalize Phase 6

This is the largest task — it removes a public type (`OutputId`) and touches every requirement/output query. Do it as one task since the file won't compile in an intermediate state otherwise (every query below reads `self.outputs`, which is deleted in the same step).

**Files:**
- Delete: `adam-rs/src/output.rs`
- Modify: `adam-rs/src/sheet.rs` (struct fields, `add_out`, all renamed queries, Phase 6 of `propagate`)
- Modify: `adam-rs/src/lib.rs` (remove `pub mod output;` and the `OutputId`/`OutputData` re-export)

**Interfaces:**
- Consumes: `CellKind`, `Sheet::add_requirement`, `Error::InvalidCellKind`/`InvalidRequirement`, `cell_has_prior_use` (Task 3's rewritten version).
- Produces: `Sheet::add_out(writer: Method, requirements: Vec<(&str, Requirement)>) -> Result<CellId, Error>`, `Sheet::cell_requirements_valid(CellId) -> bool`, `Sheet::violated_requirements(CellId) -> impl Iterator<Item = RequirementId> + '_`, `Sheet::cell_requirements(CellId) -> Option<&[RequirementId]>`, `Sheet::requirement_cell(RequirementId) -> Option<CellId>`, `Sheet::out_cells() -> impl Iterator<Item = CellId> + '_`, `Sheet::requirement_relevant_cells() -> HashSet<CellId>`, `Sheet::requirement_violation_cells() -> HashSet<CellId>`. Removes `OutputId`, `OutputData`, `Sheet::outputs` (field and old iterator method), `Sheet::output_cell`, `Sheet::add_output`, `Sheet::output_valid`, `Sheet::output_requirements`, `Sheet::requirement_output`, `Sheet::output_relevant_cells`, `Sheet::output_violation_cells`.

- [ ] **Step 1: Write the failing tests**

Add to `adam-rs/src/sheet.rs`'s `mod tests`:

```rust
#[test]
fn add_out_returns_the_cell_id_directly() {
    let mut sheet = Sheet::new();
    let width = sheet.add_cell(4_i32);
    let height = sheet.add_cell(5_i32);
    let area = sheet.add_cell(0_i32);
    let cell = sheet
        .add_out(
            Method::from_fn_2_1([width, height], area, |w: &i32, h: &i32| Ok(w * h)),
            vec![],
        )
        .unwrap();
    assert_eq!(cell, area);
    assert_eq!(sheet.cell_kind(area), Some(CellKind::Out));
}

#[test]
fn out_cell_is_referenceable_as_another_relationships_input() {
    let mut sheet = Sheet::new();
    let width = sheet.add_cell(4_i32);
    let height = sheet.add_cell(5_i32);
    let area = sheet.add_cell(0_i32);
    let doubled = sheet.add_cell(0_i32);
    sheet
        .add_out(
            Method::from_fn_2_1([width, height], area, |w: &i32, h: &i32| Ok(w * h)),
            vec![],
        )
        .unwrap();
    let result = sheet.add_relationship(vec![Method::from_fn_1_1(area, doubled, |a: &i32| Ok(a * 2))]);
    assert!(result.is_ok());
}

#[test]
fn out_cell_is_referenceable_as_a_conditional_match_subject() {
    let mut sheet = Sheet::new();
    let flag = sheet.add_cell(0_i32);
    let derived_flag = sheet.add_cell(0_i32);
    sheet
        .add_out(Method::from_fn_1_1(flag, derived_flag, |f: &i32| Ok(*f)), vec![])
        .unwrap();
    let result = sheet.add_conditional(
        MatchExpr::cell(derived_flag),
        Vec::<(Vec<i32>, Vec<RelationshipId>)>::new(),
        vec![],
    );
    assert!(result.is_ok());
}

#[test]
fn add_out_returns_invalid_cell_kind_for_a_write() {
    let mut sheet = Sheet::new();
    let width = sheet.add_cell(4_i32);
    let area = sheet.add_cell(0_i32);
    sheet
        .add_out(Method::from_fn_1_1(width, area, |w: &i32| Ok(*w)), vec![])
        .unwrap();
    assert!(matches!(sheet.write(area, 99_i32), Err(Error::InvalidCellKind)));
}

#[test]
fn add_out_returns_invalid_cell_kind_for_a_second_writer() {
    let mut sheet = Sheet::new();
    let width = sheet.add_cell(4_i32);
    let height = sheet.add_cell(5_i32);
    let area = sheet.add_cell(0_i32);
    sheet
        .add_out(Method::from_fn_1_1(width, area, |w: &i32| Ok(*w)), vec![])
        .unwrap();
    let result = sheet.add_out(Method::from_fn_1_1(height, area, |h: &i32| Ok(*h)), vec![]);
    assert!(matches!(result, Err(Error::InvalidCellKind)));
}

#[test]
fn add_out_succeeds_when_target_cell_was_previously_used_only_as_an_input() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(1_i32);
    let b = sheet.add_cell(2_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    let c = sheet.add_cell(0_i32);
    let result = sheet.add_out(Method::from_fn_1_1(a, c, |x: &i32| Ok(*x * 2)), vec![]);
    assert!(result.is_ok());
}

#[test]
fn add_out_with_a_requirement_that_would_fail_still_succeeds_and_propagate_reports_it() {
    let mut sheet = Sheet::new();
    let width = sheet.add_cell(4_i32);
    let area = sheet.add_cell(0_i32);
    let area_cell = sheet
        .add_out(
            Method::from_fn_1_1(width, area, |w: &i32| Ok(*w)),
            vec![("too_small", Requirement::from_fn_1(area, |a: &i32| Ok(*a > 100)))],
        )
        .unwrap();
    sheet.propagate().unwrap();
    assert!(!sheet.cell_requirements_valid(area_cell));
    assert_eq!(sheet.violated_requirements(area_cell).count(), 1);
}

#[test]
fn out_cells_iterates_only_out_kind_cells() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(1_i32);
    let b = sheet.add_cell(0_i32);
    let out_id = sheet.add_out(Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)), vec![]).unwrap();
    let ids: Vec<CellId> = sheet.out_cells().collect();
    assert_eq!(ids, vec![out_id]);
}

#[test]
fn add_filter_succeeds_on_a_real_out_kind_cell() {
    let mut sheet = Sheet::new();
    let width = sheet.add_cell(4_i32);
    let area = sheet.add_cell(0_i32);
    let out_cell = sheet
        .add_out(Method::from_fn_1_1(width, area, |w: &i32| Ok(*w)), vec![])
        .unwrap();
    let result = sheet.add_filter(out_cell, "clamp", Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 10))));
    assert!(result.is_ok());
}

#[test]
fn requirement_relevant_cells_covers_a_plain_cells_requirement_too() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    sheet
        .add_requirement(a, "positive", Requirement::from_fn_1(a, |x: &i32| Ok(*x > 0)))
        .unwrap();
    sheet.propagate().unwrap();
    assert!(sheet.requirement_relevant_cells().contains(&a));
}

#[test]
fn requirement_violation_cells_covers_a_plain_cells_violation_too() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    sheet
        .add_requirement(a, "too_big", Requirement::from_fn_1(a, |x: &i32| Ok(*x > 100)))
        .unwrap();
    sheet.propagate().unwrap();
    assert!(sheet.requirement_violation_cells().contains(&a));
}
```

Delete every existing test that references `OutputId`, `sheet.add_output(...)`, `sheet.output_cell(...)`, `sheet.output_valid(...)` (rename to `cell_requirements_valid`), `sheet.violated_requirements(output_id)` (update call sites to pass the `CellId` instead once `add_out` returns one), `sheet.output_requirements(...)` (rename to `cell_requirements`), `sheet.requirement_output(...)` (rename to `requirement_cell`), `sheet.outputs()` (rename to `out_cells()`), `sheet.output_relevant_cells()`/`sheet.output_violation_cells()` (rename per the table above) — update each call site's identifier and, where the old call took an `OutputId` and the new one takes a `CellId`, change the variable being passed to whatever `add_out` now returns directly (no separate `OutputId` variable to carry).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs add_out out_cells requirement_relevant_cells requirement_violation_cells`
Expected: compile errors — `Sheet::add_out` doesn't exist, `OutputId` still referenced by deleted-but-not-yet-updated call sites.

- [ ] **Step 3: Implement**

Delete `adam-rs/src/output.rs`.

In `adam-rs/src/sheet.rs`, remove the import `output::{OutputData, OutputId},` from the `use crate::{...}` block. Remove the `outputs: SlotMap<OutputId, OutputData>,` field and its doc comment from the `Sheet` struct, remove `outputs: SlotMap::with_key(),` from `Sheet::new()`. Rename the `last_violated: HashMap<OutputId, Vec<RequirementId>>` field to:

```rust
    /// Requirements that evaluated `false` as of the last `propagate()` call, grouped
    /// by cell. Sparse: a cell with no entry had all its requirements hold. Not
    /// recomputed by `propagate_without_replan`.
    last_requirement_violations: HashMap<CellId, Vec<RequirementId>>,
```

and its `Sheet::new()` initializer to `last_requirement_violations: HashMap::new(),`.

Update `cell_has_prior_use`'s doc comment (Task 3 already changed its body):

```rust
    /// Returns `true` if `id` is already claimed as some existing method's output —
    /// i.e. it cannot legally become an `out` cell's writer target, since that would
    /// leave two producers claiming the same cell.
```

Replace `add_output` entirely with:

```rust
    /// Registers `writer` as the sole producer of its one output cell, which becomes
    /// an `out` cell: always derived by `writer`, never `write()`-able, but otherwise
    /// an ordinary, freely-referenceable cell. `requirements` are attached to that
    /// cell one at a time, in order, via [`Sheet::add_requirement`].
    ///
    /// # Errors
    ///
    /// - `Error::InvalidOutput` — `writer` does not have exactly one output cell.
    /// - `Error::InvalidCellKind` — the writer's output cell is already `Source` or
    ///   `Out` kind, or already claimed as some existing method's output.
    /// - Any error [`Sheet::add_relationship`] or [`Sheet::add_requirement`] can
    ///   return.
    ///
    /// - Complexity: O(k + m²×c) where k is the number of requirements, plus the
    ///   cost of `add_relationship` for `writer` alone (m = 1 method, c = cells in
    ///   that method).
    pub fn add_out(
        &mut self,
        writer: Method,
        requirements: Vec<(&str, Requirement)>,
    ) -> Result<CellId, Error> {
        if writer.outputs.len() != 1 {
            return Err(Error::InvalidOutput);
        }
        let out_cell = writer.outputs[0];

        let kind = self.cells.get(out_cell).ok_or(Error::InvalidId)?.kind;
        if kind != CellKind::Cell || self.cell_has_prior_use(out_cell) {
            return Err(Error::InvalidCellKind);
        }

        self.add_relationship(vec![writer])?;
        self.cells[out_cell].kind = CellKind::Out;

        for (name, requirement) in requirements {
            self.add_requirement(out_cell, name, requirement)?;
        }

        Ok(out_cell)
    }
```

Replace the old `output_cell` (delete outright — no replacement), `output_requirements`, `requirement_output`, `output_valid`, `violated_requirements`, `output_relevant_cells`, `output_violation_cells`, and the old `outputs()` iterator with:

```rust
    /// Returns the requirements attached to `id`, in attachment order.
    ///
    /// Returns `None` if `id` is not a live cell in this sheet.
    pub fn cell_requirements(&self, id: CellId) -> Option<&[RequirementId]> {
        self.cells.get(id).map(|c| c.requirements.as_slice())
    }

    /// Returns the name of requirement `id`.
    ///
    /// Returns `None` if `id` is not a live requirement in this sheet.
    pub fn requirement_name(&self, id: RequirementId) -> Option<&str> {
        self.requirements.get(id).map(|c| c.name.as_str())
    }

    /// Returns the cell requirement `id` is attached to.
    ///
    /// Returns `None` if `id` is not a live requirement in this sheet.
    pub fn requirement_cell(&self, id: RequirementId) -> Option<CellId> {
        self.requirements.get(id).map(|c| c.cell)
    }

    /// Returns the cells requirement `id` reads.
    ///
    /// Returns `None` if `id` is not a live requirement in this sheet.
    pub fn requirement_inputs(&self, id: RequirementId) -> Option<&[CellId]> {
        self.requirements.get(id).map(|c| c.inputs.as_slice())
    }

    /// Returns `true` if every requirement on `id` held as of the last `propagate()`
    /// call.
    ///
    /// Returns `false` if no propagation has run yet. Also returns `true` for an
    /// `id` that is not a live cell in this sheet, since no requirement can have
    /// failed for a cell that doesn't exist.
    pub fn cell_requirements_valid(&self, id: CellId) -> bool {
        if self.last_plan.is_none() {
            return false;
        }
        !self.last_requirement_violations.contains_key(&id)
    }

    /// Iterates the requirements on `id` that evaluated to `false` as of the last
    /// `propagate()` call.
    ///
    /// - Postcondition: empty if `id`'s requirements all held, `id` is not a live
    ///   cell in this sheet, or no propagation has run yet.
    pub fn violated_requirements(&self, id: CellId) -> impl Iterator<Item = RequirementId> + '_ {
        self.last_requirement_violations
            .get(&id)
            .into_iter()
            .flatten()
            .copied()
    }

    /// Iterates all live `Out`-kind cells in the sheet.
    ///
    /// - Complexity: O(n) where n is the number of cells.
    pub fn out_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.cells
            .iter()
            .filter(|(_, c)| c.kind == CellKind::Out)
            .map(|(id, _)| id)
    }

    /// Returns the union of `contributing_cells` over every cell with at least one
    /// requirement — the set of cells currently determining at least one
    /// requirement-checked value, as of the last `propagate()` call.
    ///
    /// - Postcondition: empty if no cell in the sheet has any requirements.
    /// - Complexity: O(sum of `contributing_cells` cost over every cell with
    ///   requirements).
    pub fn requirement_relevant_cells(&self) -> HashSet<CellId> {
        self.cells
            .iter()
            .filter(|(_, c)| !c.requirements.is_empty())
            .flat_map(|(id, _)| self.contributing_cells(id))
            .collect()
    }

    /// Returns the union of `requirement_contributing_cells` over every requirement
    /// that evaluated `false` as of the last `propagate()` call, across every cell
    /// in the sheet.
    ///
    /// - Postcondition: empty if no requirement anywhere in the sheet currently
    ///   fails.
    /// - Complexity: O(sum of `requirement_contributing_cells` cost over every
    ///   violated requirement).
    pub fn requirement_violation_cells(&self) -> HashSet<CellId> {
        self.cells
            .keys()
            .flat_map(|id| self.violated_requirements(id))
            .flat_map(|rid| self.requirement_contributing_cells(rid))
            .collect()
    }
```

In `propagate`'s Phase 6, replace:

```rust
        // Phase 6: evaluate every registered requirement against current cell values.
        let mut last_violated: HashMap<OutputId, Vec<RequirementId>> = HashMap::new();
        for (requirement_id, requirement) in self.requirements.iter() {
            let inputs: Vec<&dyn Any> = requirement
                .inputs
                .iter()
                .map(|&id| self.cells[id].effective())
                .collect();
            let holds = (requirement.function)(&inputs).map_err(Error::MethodFailed)?;
            if !holds {
                last_violated
                    .entry(requirement.output)
                    .or_default()
                    .push(requirement_id);
            }
        }
        self.last_violated = last_violated;
```

with:

```rust
        // Phase 6: evaluate every registered requirement against current cell values.
        let mut last_requirement_violations: HashMap<CellId, Vec<RequirementId>> = HashMap::new();
        for (requirement_id, requirement) in self.requirements.iter() {
            let inputs: Vec<&dyn Any> = requirement
                .inputs
                .iter()
                .map(|&id| self.cells[id].effective())
                .collect();
            let holds = (requirement.function)(&inputs).map_err(Error::MethodFailed)?;
            if !holds {
                last_requirement_violations
                    .entry(requirement.cell)
                    .or_default()
                    .push(requirement_id);
            }
        }
        self.last_requirement_violations = last_requirement_violations;
```

Update `propagate`'s doc comment's "Phase 6" paragraph to say "rebuilding `last_requirement_violations` from scratch, so [`Sheet::cell_requirements_valid`] and [`Sheet::violated_requirements`] reflect this round" instead of citing `output_valid`.

In `adam-rs/src/lib.rs`, remove `pub mod output;` and remove `OutputId`/`OutputData` from whatever `pub use` line re-exports them (keep `RequirementId`/`Requirement` exports).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-rs --lib`
Expected: full suite passes. Run `cargo build -p adam-rs 2>&1 | grep -i output` to confirm no stray `OutputId`/`output.rs` references remain anywhere in `adam-rs`.

- [ ] **Step 5: Commit**

```bash
git add -A adam-rs/
git commit -m "feat(adam-rs): add_out replaces add_output; remove OutputId; generalize requirement queries to any cell"
```

---

## Task 7: `adam-lang` — `source_decl` end to end

**Files:**
- Modify: `adam-lang/src/ast.rs` (new `SourceDecl`, `SheetItem::Source` variant)
- Modify: `adam-lang/src/parser.rs` (new `parse_source_decl`, dispatch)
- Modify: `adam-lang/src/ast_parser.rs` (new `parse_source_decl`, dispatch)
- Modify: `adam-lang/src/fmt.rs` (new `write_source`, dispatch)
- Modify: `adam-lang/src/typecheck.rs` (new `SheetItem::Source` match arm)

**Interfaces:**
- Consumes: `adam_rs::Sheet::add_source` (Task 1).
- Produces: `ast::SourceDecl` (mirrors `ast::CellDecl`'s shape minus `filter`/`require`, added in Task 9), `ast::SheetItem::Source(SourceDecl)`.

- [ ] **Step 1: Write the failing tests**

Add to `adam-lang/src/parser.rs`'s `mod tests` (the direct-to-`Sheet` parser):

```rust
#[test]
fn parse_source_decl_registers_a_source_kind_cell() {
    let mut p = parser();
    let sheet = p
        .parse_str("sheet s { source width: i32 = 4; }")
        .unwrap();
    let width = p.cell_id("width").unwrap();
    assert_eq!(sheet.cell_kind(width), Some(adam_rs::CellKind::Source));
}

#[test]
fn parse_source_decl_requires_colon_or_equals() {
    let result = parser().parse_str("sheet s { source width; }");
    assert!(result.is_err());
}
```

(Adjust the exact test helper names — `parser()`, `p.cell_id(...)` — to match whatever helper functions `parser.rs`'s existing `mod tests` already use for looking up a cell by name after parsing; grep the file for how `parse_cell_decl`'s own tests retrieve a `CellId` post-parse and mirror that pattern exactly.)

Add to `adam-lang/src/ast_parser.rs`'s `mod tests`:

```rust
#[test]
fn parse_source_decl_produces_a_source_decl_sheet_item() {
    let sheet = AdamAstParser::new()
        .parse_str("sheet s { source width: i32 = 4; }")
        .unwrap();
    assert!(matches!(sheet.items[0], ast::SheetItem::Source(_)));
}
```

Add to `adam-lang/src/fmt.rs`'s `mod tests`:

```rust
#[test]
fn formats_a_source_decl() {
    let sheet = AdamAstParser::new()
        .parse_str("sheet s { source width: i32 = 4; }")
        .unwrap();
    assert_eq!(format_sheet(&sheet), "sheet s {\n    source width: i32 = 4;\n}\n");
}
```

(Match the exact expected-output indentation/newline convention already used by the neighboring `formats_a_cell` test in the same file — copy its literal formatting exactly rather than guessing.)

Add to `adam-lang/src/typecheck.rs`'s `mod tests`:

```rust
#[test]
fn source_initializer_mismatched_with_its_annotation_is_a_diagnostic() {
    let sheet = AdamAstParser::new()
        .parse_str("sheet s { source x: i32 = 1.0; }")
        .unwrap();
    let diagnostics = check_sheet(&sheet, &TypeRegistry::new());
    assert_eq!(diagnostics.len(), 1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang source_decl`
Expected: compile errors — `SourceDecl`/`SheetItem::Source`/`parse_source_decl` don't exist.

- [ ] **Step 3: Implement**

In `adam-lang/src/ast.rs`, add a new struct mirroring `CellDecl` (without `filter`, added in Task 9) and a new `SheetItem` variant:

```rust
/// `source_decl = "source" identifier cell_type_init ";".`
///
/// Same shape as [`CellDecl`] minus `filter`/`require` (added in a later pass) — a
/// `source` cell's initializer is a one-time literal exactly like a plain `cell`'s.
#[derive(Debug, Clone)]
pub struct SourceDecl {
    pub name: String,
    pub name_span: ExprSpan,
    pub type_name: Option<TypeExpr>,
    pub initializer: Option<cel_parser::Expr>,
    pub leading_comment: Option<Comment>,
    pub doc_comment: Option<String>,
    pub blank_line_before: bool,
    pub span: ExprSpan,
}
```

Find `SheetItem`'s enum definition and add a `Source(SourceDecl)` variant alongside `Cell(CellDecl)`.

In `adam-lang/src/parser.rs` (the direct-to-`Sheet` parser), find `parse_cell_decl` and its dispatch site (wherever `sheet_item` matches on the next keyword to choose `parse_cell_decl`/`parse_out_decl`/etc.) and add:

```rust
/// `source_decl = "source" identifier cell_type_init ";".`
fn parse_source_decl(&mut self, ctx: &mut ParseContext) -> Result<()> {
    ctx.is_keyword("source"); // consume
    let (name, name_span) = ctx.consume_ident()?;
    if ctx.cell_names.contains_key(&name) {
        return Err(ParseError::new(
            format!("duplicate cell `{name}`"),
            name_span,
        ));
    }

    let declared_shape: Option<TypeShape> = if ctx.consume_punct(":") {
        let type_expr = self.parse_type_expr(ctx)?;
        Some(
            self.types
                .resolve(&type_expr)
                .map_err(|(msg, span)| ParseError::new(msg, span))?,
        )
    } else {
        None
    };

    let has_initializer = ctx.consume_punct("=");
    let (shape, cell_id) = if has_initializer {
        let segment = self.parse_cel_expression(ctx)?;
        let (actual_shape, cell_id) = self.build_source_cell_from_segment(segment, ctx)?;
        if let Some(declared) = &declared_shape
            && declared != &actual_shape
        {
            return Err(ParseError::new(
                format!(
                    "source `{name}`: type mismatch: expected `{}`, got `{}`",
                    self.types.display_name(declared),
                    self.types.display_name(&actual_shape)
                ),
                name_span,
            ));
        }
        (actual_shape, cell_id)
    } else {
        let declared = declared_shape.ok_or_else(|| {
            ParseError::new("expected `:` or `=` in source declaration", name_span)
        })?;
        let cell_id = self.build_default_source_cell(&declared, name_span, ctx)?;
        (declared, cell_id)
    };

    ctx.expect_punct(";")?;
    ctx.cell_names.insert(name, (cell_id, shape));
    Ok(())
}
```

This calls two new helpers mirroring `build_cell_from_segment`/`build_default_cell` exactly, but routed through `Sheet::add_source` instead of `Sheet::add_cell` — find those two existing helpers and add sibling versions (`build_source_cell_from_segment`, `build_default_source_cell`) whose bodies are identical except for the `sheet.add_cell(...)` call becoming `sheet.add_source(...)`. Wire `parse_source_decl` into whatever `sheet_item` dispatch function currently chooses between `parse_cell_decl`/`parse_out_decl`/`parse_relationship_decl`/`parse_conditional_decl` by keyword, adding a `"source"` arm.

In `adam-lang/src/ast_parser.rs`, mirror `parse_cell_decl` (line 184) with a `parse_source_decl` producing `ast::SourceDecl` instead of `ast::CellDecl`, reusing the exact same body shape (it's structurally identical to `parse_cell_decl` today, minus the `filter` clause):

```rust
/// `source_decl = "source" identifier cell_type_init ";".`
fn parse_source_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::SourceDecl> {
    let decl_start = cursor.peek_span();
    cursor.is_keyword("source");
    let (name, name_span) = cursor.consume_ident()?;
    let (type_name, initializer) = if cursor.consume_punct(":") {
        let type_name = self.parse_type_expr(cursor)?;
        let initializer = if cursor.consume_punct("=") {
            Some(self.parse_cel_expression(cursor)?)
        } else {
            None
        };
        (Some(type_name), initializer)
    } else if cursor.consume_punct("=") {
        (None, Some(self.parse_cel_expression(cursor)?))
    } else {
        return Err(cursor.err_at("expected `:` or `=` in source declaration"));
    };
    let semi_span = cursor.expect_punct(";")?;
    Ok(ast::SourceDecl {
        name,
        name_span: point(name_span),
        type_name,
        initializer,
        leading_comment: None,
        doc_comment: None,
        blank_line_before: false,
        span: ast::ExprSpan {
            start: decl_start,
            end: semi_span,
        },
    })
}
```

Wire it into whatever top-level `sheet_item` dispatch function in `ast_parser.rs` chooses between `parse_cell_decl`/`parse_out_decl`/etc. by keyword, adding a `"source"` arm producing `ast::SheetItem::Source(...)`.

In `adam-lang/src/fmt.rs`, add a `write_source` mirroring `write_cell` (line 249) minus the filter clause:

```rust
/// Writes one `source name[: type][ = initializer];` declaration.
fn write_source(out: &mut String, decl: &ast::SourceDecl, depth: usize) {
    write_trivia(
        out,
        decl.blank_line_before,
        decl.leading_comment.as_ref(),
        depth,
    );
    write_doc_comment(out, "///", decl.doc_comment.as_deref(), depth);
    out.push_str(&indent(depth));
    out.push_str("source ");
    out.push_str(&decl.name);
    if let Some(type_expr) = &decl.type_name {
        out.push_str(": ");
        out.push_str(&source_text_or_empty(type_expr.span()));
    }
    if let Some(expr) = &decl.initializer {
        out.push_str(" = ");
        out.push_str(&cel_parser::format_expr(expr));
    }
    out.push_str(";\n");
}
```

Add `ast::SheetItem::Source(decl) => write_source(out, decl, depth),` to `write_sheet_item`'s match (line 329-338).

In `adam-lang/src/typecheck.rs`, add a `SheetItem::Source(source)` arm to `check_sheet`'s match (line 40-72), calling a new `check_source_initializer` that's identical in body to `check_cell_initializer` but takes a `&SourceDecl` — or, to avoid duplicating logic, change `check_cell_initializer`'s signature to accept the two fields it actually needs (`name: &str, type_name: Option<&TypeExpr>, initializer: Option<&Expr>`) rather than a whole `&CellDecl`, and call it from both arms with each decl kind's fields extracted. Prefer the shared-signature refactor — check `check_cell_initializer`'s current body first (grep for `fn check_cell_initializer`) before deciding which fields it needs, then apply the same refactor consistently.

`declared_cell_types` (line 86) also needs to see `source` declarations when building its name→type map, since a `source`'s name can be referenced by other cells' expressions exactly like a `cell`'s — add a `SheetItem::Source(source) => { ... }` arm there alongside its existing `SheetItem::Cell`/`SheetItem::Out` arms, inserting `source.name` into the same maps using the same shape-inference logic already used for `cell`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-lang --lib`
Expected: full suite passes.

- [ ] **Step 5: Commit**

```bash
git add adam-lang/src/ast.rs adam-lang/src/parser.rs adam-lang/src/ast_parser.rs adam-lang/src/fmt.rs adam-lang/src/typecheck.rs
git commit -m "feat(adam-lang): add source_decl grammar end to end"
```

---

## Task 8: `adam-lang` — named `filter` clause

**Files:**
- Modify: `adam-lang/src/ast.rs` (`CellFilter` gains `name`/`name_span`)
- Modify: `adam-lang/src/parser.rs` (`parse_cell_filter` reads the name)
- Modify: `adam-lang/src/ast_parser.rs` (`parse_cell_filter` reads the name)
- Modify: `adam-lang/src/fmt.rs` (`write_cell`'s filter clause includes the name)
- Modify: `adam-lang/src/typecheck.rs` (`check_filter` unaffected in logic, but its call sites' error messages may want the filter's name — optional, not required for correctness)

**Interfaces:**
- Consumes: `adam_rs::Sheet::add_filter(cell, name, filter)` (Task 4).
- Produces: `ast::CellFilter::name: String`, `ast::CellFilter::name_span: ExprSpan`.

- [ ] **Step 1: Write the failing tests**

Add to `adam-lang/src/parser.rs`'s `mod tests`:

```rust
#[test]
fn parse_named_filter_attaches_it_under_its_name() {
    let mut p = parser();
    let sheet = p
        .parse_str("sheet s { cell x: i32 = 0 filter clamp: 0..=10; }")
        .unwrap();
    let x = p.cell_id("x").unwrap();
    assert_eq!(sheet.filter_name(x), Some("clamp"));
}

#[test]
fn parse_filter_without_a_name_is_a_syntax_error() {
    let result = parser().parse_str("sheet s { cell x: i32 = 0 filter 0..=10; }");
    assert!(result.is_err());
}
```

Add to `adam-lang/src/ast_parser.rs`'s `mod tests`:

```rust
#[test]
fn parse_cell_filter_records_its_name() {
    let sheet = AdamAstParser::new()
        .parse_str("sheet s { cell x: i32 = 0 filter clamp: 0..=10; }")
        .unwrap();
    let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
        panic!("expected a cell decl");
    };
    assert_eq!(cell.filter.as_ref().unwrap().name, "clamp");
}
```

Add to `adam-lang/src/fmt.rs`'s `mod tests`:

```rust
#[test]
fn formats_a_named_filter() {
    let sheet = AdamAstParser::new()
        .parse_str("sheet s { cell x: i32 = 0 filter clamp: 0..=10; }")
        .unwrap();
    assert_eq!(
        format_sheet(&sheet),
        "sheet s {\n    cell x: i32 = 0 filter clamp: 0..=10;\n}\n"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang named_filter parse_cell_filter_records_its_name formats_a_named_filter`
Expected: compile errors / parse errors — the grammar doesn't require or consume a name yet.

- [ ] **Step 3: Implement**

In `adam-lang/src/ast.rs`, add `name`/`name_span` to `CellFilter`:

```rust
/// `cell_filter = "filter" identifier ":" expression.`
#[derive(Debug, Clone)]
pub struct CellFilter {
    /// The filter's declared name.
    pub name: String,
    /// The name token's span.
    pub name_span: ExprSpan,
    /// The filter's body expression. `_` inside it denotes the candidate value being conformed;
    /// every other identifier that names an already-declared cell is a deduced dependency.
    pub body: cel_parser::Expr,
    /// The span of the whole `filter ...` clause.
    pub span: ExprSpan,
}
```

In `adam-lang/src/parser.rs`, find `parse_cell_filter` (grammar comment: `` `cell_filter = "filter" expression.` ``). Its existing `name`/`name_span`/`declared_shape` parameters are the *filtered cell's* own name/span/type (used for error-message context, e.g. a message like `cell {name}: filter must reference _`, and to know the candidate value's declared type) — **not** a filter name, since filters were anonymous until now. The filter's own name is a new identifier this function must additionally consume, so it returns `(String, Filter)` instead of bare `Filter`:

```rust
/// `cell_filter = "filter" identifier ":" expression.`
///
/// Builds an [`adam_rs::Filter`] from a single deduced expression: `_` denotes the candidate
/// value being conformed (of `declared_shape`'s type); every other identifier that names an
/// already-declared cell is a deduced dependency, exactly as [`Self::parse_deduced_expr`]
/// resolves them for a `relationship` binding or `out` declaration — see
/// [`Self::parse_filter_expr`]. `name`/`name_span`/`declared_shape` describe the *filtered
/// cell* (for error-message context and the candidate value's type), already resolved by the
/// caller — unrelated to the filter's own name, consumed here and returned alongside the
/// built `Filter`.
fn parse_cell_filter(
    &mut self,
    ctx: &mut ParseContext,
    name: &str,
    name_span: proc_macro2::Span,
    declared_shape: &TypeShape,
) -> Result<(String, Filter)> {
    let (filter_name, _filter_name_span) = ctx.consume_ident()?;
    ctx.expect_punct(":")?;
    // ...existing body-parsing logic (builds and returns a `Filter` from the expression
    // that follows `:`, using `name`/`declared_shape` for error messages exactly as
    // before) is unchanged; wrap its final `Filter` result as `Ok((filter_name, that_filter))`.
}
```

(Read the function's current full body past its signature before editing — this plan only has its signature and doc comment in hand; the expression-parsing logic itself doesn't change, only the new leading `identifier ":"` consumption and the return type.) Update its call site in `parse_cell_decl`:

```rust
let filter = if ctx.is_keyword("filter") {
    Some(self.parse_cell_filter(ctx, &name, name_span, &shape)?)
} else {
    None
};
```

(unchanged call shape — only the return type changes, from `Option<Filter>` to `Option<(String, Filter)>`) and its attachment site:

```rust
if let Some((filter_name, filter)) = filter {
    ctx.sheet
        .add_filter(cell_id, filter_name, filter)
        .map_err(|e| ParseError::new(e.to_string(), name_span))?;
}
```

In `adam-lang/src/ast_parser.rs`, update `parse_cell_filter` (line 228):

```rust
/// `cell_filter = "filter" identifier ":" expression.`
///
/// - Precondition: the `filter` keyword has already been consumed by the caller; `filter_start`
///   is its span.
fn parse_cell_filter(
    &mut self,
    cursor: &mut TokenCursor,
    filter_start: proc_macro2::Span,
) -> Result<ast::CellFilter> {
    let (name, name_span) = cursor.consume_ident()?;
    cursor.expect_punct(":")?;
    let body = self.parse_cel_expression(cursor)?;
    let body_end = body.span().end;
    Ok(ast::CellFilter {
        name,
        name_span: point(name_span),
        body,
        span: ast::ExprSpan {
            start: filter_start,
            end: body_end,
        },
    })
}
```

In `adam-lang/src/fmt.rs`, update `write_cell`'s filter-writing block (line 268-271):

```rust
    if let Some(filter) = &cell.filter {
        out.push_str(" filter ");
        out.push_str(&filter.name);
        out.push_str(": ");
        out.push_str(&cel_parser::format_expr(&filter.body));
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-lang --lib`
Expected: full suite passes.

- [ ] **Step 5: Commit**

```bash
git add adam-lang/src/ast.rs adam-lang/src/parser.rs adam-lang/src/ast_parser.rs adam-lang/src/fmt.rs
git commit -m "feat(adam-lang): filter clauses require a name"
```

---

## Task 9: `adam-lang` — generalized `require` on `cell`/`source`, and `filter` on `out`

**Files:**
- Modify: `adam-lang/src/ast.rs` (`CellDecl`/`SourceDecl` gain `require: Option<RequireBlock>`; `OutDecl` gains `filter: Option<CellFilter>`)
- Modify: `adam-lang/src/parser.rs` (`parse_cell_decl`/`parse_source_decl` gain a trailing `require_block`; `parse_out_decl` gains a `cell_filter`)
- Modify: `adam-lang/src/ast_parser.rs` (same three productions)
- Modify: `adam-lang/src/fmt.rs` (`write_cell`/`write_source` gain the `require` clause; `write_out` gains the `filter` clause)
- Modify: `adam-lang/src/typecheck.rs` (factor `check_out`'s requirement-checking loop into a shared `check_requirements` helper, called from `check_cell`/`check_source` too)

**Interfaces:**
- Consumes: `adam_rs::Sheet::add_requirement` (Task 5), `SourceDecl` (Task 7).
- Produces: `ast::CellDecl::require`, `ast::SourceDecl::require`, `ast::OutDecl::filter`.

- [ ] **Step 1: Write the failing tests**

Add to `adam-lang/src/parser.rs`'s `mod tests`:

```rust
#[test]
fn parse_cell_decl_with_a_require_block_attaches_requirements() {
    let mut p = parser();
    let sheet = p
        .parse_str("sheet s { cell x: i32 = 5 require { positive: x > 0; } }")
        .unwrap();
    let x = p.cell_id("x").unwrap();
    assert_eq!(sheet.cell_requirements(x).unwrap().len(), 1);
}

#[test]
fn parse_source_decl_with_a_require_block_attaches_requirements() {
    let mut p = parser();
    let sheet = p
        .parse_str("sheet s { source x: i32 = 5 require { positive: x > 0; } }")
        .unwrap();
    let x = p.cell_id("x").unwrap();
    assert_eq!(sheet.cell_requirements(x).unwrap().len(), 1);
}

#[test]
fn parse_out_decl_with_a_filter_clause_attaches_a_named_filter() {
    let mut p = parser();
    let sheet = p
        .parse_str("sheet s { cell width: i32 = 4; out area := width filter clamp: 0..=100; }")
        .unwrap();
    let area = p.cell_id("area").unwrap();
    assert_eq!(sheet.filter_name(area), Some("clamp"));
}
```

Add to `adam-lang/src/typecheck.rs`'s `mod tests`:

```rust
#[test]
fn cell_requirement_non_bool_body_is_a_diagnostic() {
    let sheet = AdamAstParser::new()
        .parse_str("sheet s { cell x: i32 = 5 require { positive: x; } }")
        .unwrap();
    let diagnostics = check_sheet(&sheet, &TypeRegistry::new());
    assert_eq!(diagnostics.len(), 1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang require_block cell_requirement_non_bool`
Expected: compile/parse errors — `CellDecl`/`SourceDecl` have no `require` field, `parse_cell_decl` doesn't consume a trailing `require` block, `OutDecl` has no `filter` field.

- [ ] **Step 3: Implement**

In `adam-lang/src/ast.rs`, add `pub require: Option<RequireBlock>` to both `CellDecl` and `SourceDecl` (after `filter`/`initializer` respectively), and `pub filter: Option<CellFilter>` to `OutDecl` (after `initializer`, before `require`).

In `adam-lang/src/parser.rs`'s `parse_cell_decl`, after the existing filter-parsing block and before `ctx.expect_punct(";")?`, add:

```rust
        let require_names_and_reqs: Vec<(String, Requirement)> = if ctx.is_keyword("require") {
            ctx.expect_open_brace()?;
            let mut reqs = Vec::new();
            while !ctx.at_close_brace() {
                reqs.push(self.parse_requirement(ctx)?);
            }
            ctx.expect_close_brace()?;
            reqs
        } else {
            Vec::new()
        };
```

then, after `ctx.cell_names.insert(name, (cell_id, shape));` and the existing `if let Some((filter_name, filter)) = filter { ... }` block (Task 8's version), add:

```rust
        for (req_name, requirement) in require_names_and_reqs {
            ctx.sheet
                .add_requirement(cell_id, req_name, requirement)
                .map_err(|e| ParseError::new(e.to_string(), name_span))?;
        }
```

Apply the identical two additions to `parse_source_decl` (Task 7), inserting the `require`-block parsing after its initializer and the `add_requirement` loop after `ctx.cell_names.insert(...)`.

In `parse_out_decl`, after `let cell_id = self.build_default_cell(&out_shape, name_span, ctx)?;` and before the existing `if ctx.is_keyword("require")` block, add — reusing Task 8's `parse_cell_filter(ctx, name, name_span, declared_shape) -> Result<(String, Filter)>` exactly as `parse_cell_decl` does, passing the *out declaration's* own `name`/`name_span` (already in scope) and `out_shape` as the candidate value's type:

```rust
        let filter = if ctx.is_keyword("filter") {
            Some(self.parse_cell_filter(ctx, &name, name_span, &out_shape)?)
        } else {
            None
        };
```

and, after `add_out` is called (replacing today's `add_output` call — this depends on Task 6 having landed in `adam-rs`), attach the filter to the returned cell before attaching requirements. Note the `if let` below binds `filter_name`/`filter`, not `name`/`filter` — reusing `name` here would shadow the out declaration's own name, which `ctx.output_names.insert` still needs on the next line:

```rust
        let out_cell = ctx
            .sheet
            .add_out(writer, named_requirements)
            .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
        if let Some((filter_name, filter)) = filter {
            ctx.sheet
                .add_filter(out_cell, filter_name, filter)
                .map_err(|e| ParseError::new(e.to_string(), name_span))?;
        }
        ctx.output_names.insert(name, out_cell);
```

Apply the identical two additions to `ast_parser.rs`'s `parse_cell_decl`/`parse_source_decl` (a `require_block` producing `ast::RequireBlock`, reusing the existing `parse_requirement` helper already used by `parse_out_decl`) and `parse_out_decl` (a `cell_filter` before its existing `require` block).

In `adam-lang/src/fmt.rs`, add the `require` clause to `write_cell`/`write_source` (mirroring `write_out`'s existing require-writing block, lines 308-321) and the `filter` clause to `write_out` (mirroring `write_cell`'s filter-writing block from Task 8, placed after `write_str(" := ")`'s initializer and before the `require` block).

In `adam-lang/src/typecheck.rs`, factor `check_out`'s requirement loop (lines 632-644ish) into:

```rust
/// Checks every requirement in `require`'s body against `resolve`, appending a diagnostic for
/// each one that doesn't type-check as `bool`.
fn check_requirements(
    require: Option<&ast::RequireBlock>,
    resolve: &impl Fn(&str) -> Ty,
    diagnostics: &mut Vec<ParseError>,
) {
    let Some(require) = require else {
        return;
    };
    for requirement in &require.requirements {
        let (req_ty, req_diags) = check_expr(&requirement.body, resolve);
        diagnostics.extend(req_diags);
        if !req_ty.unifies_with(&Ty::Bool) {
            diagnostics.push(ParseError::new_range(
                format!(
                    "requirement `{}` produces `{}`, but requirements must be `bool`",
                    requirement.name,
                    req_ty.name()
                ),
                requirement.body.span().start,
                requirement.body.span().end,
            ));
        }
    }
}
```

Call it from `check_out` (replacing its inline loop) and from new `SheetItem::Cell`/`SheetItem::Source` handling in `check_sheet`'s match, passing each decl's own `require` field and `resolve`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-lang --lib`
Expected: full suite passes.

- [ ] **Step 5: Commit**

```bash
git add adam-lang/src/ast.rs adam-lang/src/parser.rs adam-lang/src/ast_parser.rs adam-lang/src/fmt.rs adam-lang/src/typecheck.rs
git commit -m "feat(adam-lang): generalize require to cell/source; add filter to out"
```

---

## Task 10: `adam-web-ui` Inspector update

**Files:**
- Modify: `adam-web-ui/src/inspector.rs:65-113` (`compute_output_status`)

**Interfaces:**
- Consumes: `Sheet::out_cells`, `requirement_relevant_cells`, `cell_requirements_valid`, `requirement_violation_cells`, `violated_requirements(CellId)`, `requirement_name` (all Task 6).

- [ ] **Step 1: Write the failing test**

`adam-web-ui/src/inspector.rs` likely has no existing unit test directly on `compute_output_status` (check first: grep the file's `mod tests` for `compute_output_status`). If one exists, it references `sheet.outputs()`/`sheet.output_cell(...)` and will already fail to compile once Task 6 lands upstream — update it in place rather than writing a new one. If none exists, add:

```rust
#[test]
fn compute_output_status_covers_a_plain_cells_requirement_violation() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    sheet
        .add_requirement(a, "too_big", adam_rs::Requirement::from_fn_1(a, |x: &i32| Ok(*x > 100)))
        .unwrap();
    sheet.propagate().unwrap();
    let status = compute_output_status(&sheet);
    assert!(status.invalid_contributors.contains(&a));
}
```

(`compute_output_status` is a private `fn` in this file per its current signature — this test lives in the same file's `mod tests` so it can call it directly.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p adam-web-ui compute_output_status`
Expected: compile error (`Sheet::outputs`/`output_cell`/`output_valid`/`output_violation_cells` no longer exist upstream in `adam-rs` after Task 6).

- [ ] **Step 3: Implement**

Replace `compute_output_status`'s body:

```rust
fn compute_output_status(sheet: &Sheet) -> OutputStatus {
    let out_cells: Vec<CellId> = sheet.out_cells().collect();
    let relevant = sheet
        .requirement_relevant_cells()
        .into_iter()
        .chain(
            sheet
                .conditionals()
                .filter_map(|id| sheet.conditional_match_cells(id))
                .flatten()
                .copied(),
        )
        .collect();
    let invalid_outputs = out_cells
        .iter()
        .copied()
        .filter(|&id| !sheet.cell_requirements_valid(id))
        .collect();
    let invalid_contributors = sheet
        .requirement_violation_cells()
        .into_iter()
        .chain(sheet.filter_violation_cells())
        .collect();
    let filter_violated = sheet.filter_violated_cells().collect();
    let output_cells = out_cells.iter().copied().collect();
    let invalid_output_requirement_names = out_cells
        .iter()
        .filter_map(|&cell| {
            let names: Vec<&str> = sheet
                .violated_requirements(cell)
                .filter_map(|rid| sheet.requirement_name(rid))
                .collect();
            (!names.is_empty()).then(|| (cell, names.join(", ")))
        })
        .collect();
    OutputStatus {
        has_outputs: !out_cells.is_empty(),
        relevant,
        invalid_contributors,
        invalid_outputs,
        filter_violated,
        output_cells,
        invalid_output_requirement_names,
    }
}
```

Update `OutputStatus`'s doc comments that reference `Sheet::output_relevant_cells`/`output_violation_cells`/`output_cell`/`output_valid`/`Sheet::add_output` to their renamed forms (`requirement_relevant_cells`/`requirement_violation_cells`/`out_cells`/`cell_requirements_valid`/`add_out`) — search for those four names elsewhere in the same file's doc comments (the struct-level comments on `relevant`, `invalid_contributors`, `invalid_outputs`, `output_cells`, `invalid_output_requirement_names`) and update each.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-web-ui --lib` and `cargo build -p begin -p adam-lang-book-live` (both crates depend on `adam-web-ui`; confirm neither has its own stale references).
Expected: all pass; both crates build.

- [ ] **Step 5: Commit**

```bash
git add adam-web-ui/src/inspector.rs
git commit -m "fix(adam-web-ui): update Inspector to the renamed Sheet requirement/out-cell API"
```

---

## Task 11: `editors/vscode-adam-lang` — `source` keyword

**Files:**
- Modify: `editors/vscode-adam-lang/syntaxes/adam-lang.tmLanguage.json`

**Interfaces:** none (syntax highlighting only).

- [ ] **Step 1: Write the failing test**

This extension has no automated grammar test in this repo (check `editors/vscode-adam-lang/package.json`'s scripts first to confirm) — skip the test-first steps and go straight to the change; verify manually per Step 4.

- [ ] **Step 2: N/A**

- [ ] **Step 3: Implement**

In `editors/vscode-adam-lang/syntaxes/adam-lang.tmLanguage.json`, find the keyword pattern:

```json
{"name":"keyword.declaration.adam-lang","match":"\\b(sheet|cell|relationship|conditional|out|require|filter)\\b"}
```

and add `source`:

```json
{"name":"keyword.declaration.adam-lang","match":"\\b(sheet|cell|source|relationship|conditional|out|require|filter)\\b"}
```

- [ ] **Step 4: Verify**

Open a `.adm2` file containing `source width: i32 = 4;` in VS Code with the extension loaded (or run its test harness if `package.json` defines one) and confirm `source` highlights the same as `cell`/`out`.

- [ ] **Step 5: Commit**

```bash
git add editors/vscode-adam-lang/syntaxes/adam-lang.tmLanguage.json
git commit -m "feat(vscode-adam-lang): highlight the source keyword"
```

---

## Task 12: `adam-lang-book` updates

**Files:**
- Modify: `adam-lang-book/book-src/filters.md` and its `examples/filters/*.adm2` files
- Modify: `adam-lang-book/book-src/outputs.md` and its `examples/outputs/*.adm2` files
- Modify: `adam-lang-book/book-src/cells.md`
- Modify: `adam-lang-book/book-src/reference.md`
- Modify: `adam-lang-book/book-src/SUMMARY.md`
- Create: `adam-lang-book/book-src/source.md` and `adam-lang-book/book-src/examples/source/*.adm2`
- Sweep: `adam-lang-book/book-src/tutorial.md`, `relationships.md`, `conditionals.md`, `expressions.md`, `style.md` for stale chapter-number cross-references

**Interfaces:** none (documentation only, but build-checked by `xtask prepare-live-book-assets`).

- [ ] **Step 1: Update every `.adm2` example that uses the old anonymous-filter grammar**

Update `examples/filters/write_never_filters.adm2`, `raw_value_never_lost.adm2`, `range_filter_kind.adm2`, `derived_cell_diagnosed_not_corrected.adm2`, `must_reference_underscore.adm2`, `tuple_filter_not_supported.adm2` — each currently has a bare `filter <expr>` clause; add a name to each, e.g.:

```
sheet s { cell level: i32 = 50 filter clamp: 0..=100; }
```

(`must_reference_underscore.adm2`'s whole point is a body with no `_` — keep that property, just add the name: `sheet s { cell x: i32 = 0 filter always_five: 5; }`.)

- [ ] **Step 2: Run the book build to confirm every example still parses**

Run: `cargo run -p xtask -- prepare-live-book-assets` (or whatever the exact command is — confirm via `cargo run -p xtask -- --help`)
Expected: no parse errors reported for any `.adm2` file under `filters/`.

- [ ] **Step 3: Rewrite `filters.md`**

Update §6.1's grammar block to:

```text
cell_filter = "filter" identifier ":" expression.
```

and its worked examples to include a name (`filter clamp: 0..=100;`, etc.). Delete §6.6's closing line ("a filter cannot attach to an output cell") and replace it with a short paragraph noting a filter may now attach to any cell kind — `cell`, `source`, or `out` — with a new example under `examples/filters/filter_on_an_out_cell.adm2`:

```
sheet s { cell width: i32 = 4; out area := width filter clamp: 0..=100; }
```

- [ ] **Step 4: Rewrite `outputs.md` §7.2 and add the `source`/generalized-`require` context to §7.3**

Replace the §7.2 heading "An output's cell is terminal" and its first paragraph (the one asserting the input-reference restriction) with a paragraph describing the actual, now-true behavior: an out cell can be read anywhere a plain cell can, and remains restricted only in that it can never be written directly and can never be produced by more than one method. Keep the existing `output_cell_is_terminal.adm2` example's file name if its content (a trivial passing sheet) still illustrates the point, or replace its content with something that actually exercises cross-referencing, e.g.:

```
sheet s {
    cell width: i32 = 10;
    out area := width * 2;
    out doubled_area := area * 2;
}
```

Add a lead-in sentence to §7.3 noting `require` is no longer `out`-only, with a forward reference to the new `source.md` chapter and to `cells.md`'s own `require` coverage.

- [ ] **Step 5: Update `cells.md` and `reference.md`'s grammar/keyword listings**

In `cells.md`, update the `cell_decl` grammar line (currently `cell_decl = "cell" identifier cell_type_init [ "filter" expression ] ";".`) to:

```
cell_decl = "cell" identifier cell_type_init [ cell_filter ] [ require_block ] ";".
```

and add a short paragraph pointing to `filters.md`/`outputs.md` for `filter`/`require` details now that both apply to plain cells too.

In `reference.md` (Appendix A), update: the keyword list (`sheet`, `cell`, `source`, `relationship`, `conditional`, `out`, `require`, `filter`), the full grammar block (`sheet_item`, `cell_decl`, add `source_decl`, `cell_filter` with the name, `out_decl`), and the `A.8`/`A.9` bullet lists — delete "a filter cannot attach to an output cell" from `A.8` and update `A.9`'s "outputs and requirements" framing to note `require` now generalizes to any cell (retitle the section if needed to "Filters" and "Requirements" as independent, cell-kind-agnostic mechanisms rather than nesting requirements under "Outputs").

- [ ] **Step 6: Add the `source` chapter and renumber**

Create `adam-lang-book/book-src/source.md` following the existing chapters' structure (grammar section, worked example, a section on what's still restricted — never derived, never a method's output — with a live `.adm2` example under a new `examples/source/` directory). Insert it into `SUMMARY.md` at the position that keeps the existing pedagogical order (after "Sheets, Cells, and Types," before "Expressions and Dependency Deduction," since `source` is a cell-declaration concept like `cell`, not a solver concept). Renumber every "Chapter N"/"N.M" cross-reference in every `.md` file in `adam-lang-book/book-src/` that comes after the insertion point (grep for `Chapter [4-9]` and `#4\.`–`#9\.`-style anchor links across the whole `book-src/` directory and shift each by one).

- [ ] **Step 7: Sweep remaining chapters for stale references**

Check `tutorial.md`, `relationships.md`, `conditionals.md`, `expressions.md`, `style.md` for any mention of "filter" (unnamed), "output"/"terminal," or a "Chapter N" cross-reference affected by Step 6's renumbering; fix each found.

- [ ] **Step 8: Run the full book build**

Run: `cargo run -p xtask -- prepare-live-book-assets` and whatever command builds the mdBook itself (check `adam-lang-book/README.md` for the exact `mdbook build`/`mdbook serve` invocation).
Expected: builds clean, no parse/propagate errors reported for any live-mounted example, no broken internal links.

- [ ] **Step 9: Commit**

```bash
git add adam-lang-book/
git commit -m "docs(adam-lang-book): update for source cells, named filters, and generalized require"
```

---

## Final Verification

- [ ] Run the full workspace check suite per root `CLAUDE.md`:

```bash
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace
```

- [ ] Confirm zero compiler warnings in the `cargo build --workspace`/`cargo test --workspace` output (not just clippy-clean).
- [ ] Manually verify the live book renders correctly per `verifying-begin-ui`-style spot checks: open `adam-lang-book`'s `filters.md`, `outputs.md`, and the new `source.md` pages and confirm each live-mounted example resolves and displays without an error diagnostic in place of the intended output.
