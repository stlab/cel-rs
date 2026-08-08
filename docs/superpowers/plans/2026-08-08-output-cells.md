# Output Cells and Conditions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an `Output` construct to `adam-rs`: a cell written by exactly one method that is also a terminal (never usable as an input elsewhere), carrying named `Condition`s checked after every `propagate()`. Add a general `contributing_cells` query usable on any cell.

**Architecture:** `Condition` and `Output` are new leaf types (`condition.rs`, `output.rs`) mirroring the existing `Method`/`Relationship` split. `Sheet::add_output` wraps the existing `add_relationship` (reusing all its validation) with a "terminal cell" invariant enforced by new checks in `add_relationship`, `add_conditional`, and `write`. A new Phase 6 in `propagate()` evaluates conditions and caches violations, exposed via `output_valid`/`violated_conditions`. `contributing_cells` generalizes the BFS already used internally by `add_conditional`'s validation into a public, plan-aware query.

**Tech Stack:** Rust, `slotmap`, `anyhow`, `std::collections::{HashMap, HashSet}`. No new dependencies.

## Global Constraints

- Every function gets a `///` contract-style doc comment (Summary, Preconditions as `debug_assert!`-backed bullets or narrative, Postconditions, Complexity when not O(1)) — see root `CLAUDE.md` "Documentation comments".
- Tests are derived from the contract/public interface only — do not test implementation details, do not test precondition violations.
- Arithmetic on signed integers must use `checked_*` operations, not wrapping arithmetic — see root `CLAUDE.md` "Fallible ops".
- No heap allocations beyond what's already idiomatic here (`HashSet`/`HashMap`/`Vec`/`String` for owned state are fine; avoid unnecessary clones).
- `cargo fmt --all` must be run before every commit (enforced by the pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`, `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`, and `cargo clippy -p begin --all-targets -- -D warnings` must all be clean before the branch is considered done.
- Full spec: `docs/superpowers/specs/2026-08-07-output-cells-design.md`.

---

### Task 1: `Error` variants

**Files:**
- Modify: `adam-rs/src/error.rs`

**Interfaces:**
- Produces: `Error::InvalidOutput`, `Error::TerminalCell` — consumed by every later task.

- [ ] **Step 1: Write the failing tests**

Open `adam-rs/src/error.rs`. In the existing `#[cfg(test)] mod tests` block, add these tests after `duplicate_method_outputs_display_contains_outputs` (the last test in the file, just before the module's closing `}`):

```rust
    #[test]
    fn invalid_output_display_contains_invalid() {
        assert!(Error::InvalidOutput.to_string().contains("invalid"));
    }

    #[test]
    fn invalid_output_has_no_source() {
        assert!(std::error::Error::source(&Error::InvalidOutput).is_none());
    }

    #[test]
    fn terminal_cell_display_contains_terminal() {
        assert!(Error::TerminalCell.to_string().contains("terminal"));
    }

    #[test]
    fn terminal_cell_has_no_source() {
        assert!(std::error::Error::source(&Error::TerminalCell).is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs`
Expected: compile error — `no variant named \`InvalidOutput\` found for enum \`Error\`` (and similarly for `TerminalCell`).

- [ ] **Step 3: Add the new variants**

In `adam-rs/src/error.rs`, change the end of the `Error` enum from:

```rust
    /// A conditional is structurally invalid: the cell was not found, a referenced
    /// relationship was not found, a branch relationship that shares a cell with the match
    /// cell or any of its unconditional upstream contributors has more than one method, a
    /// relationship appears in more than one conditional branch, a branch key's type does
    /// not match the cell's registered type, or a branch has no keys.
    InvalidConditional,
}
```

to:

```rust
    /// A conditional is structurally invalid: the cell was not found, a referenced
    /// relationship was not found, a branch relationship that shares a cell with the match
    /// cell or any of its unconditional upstream contributors has more than one method, a
    /// relationship appears in more than one conditional branch, a branch key's type does
    /// not match the cell's registered type, or a branch has no keys.
    InvalidConditional,

    /// An `add_output` call is structurally invalid: the writer method does not have
    /// exactly one output cell, a condition has an empty name, two conditions in the same
    /// call share a name, or a condition's `inputs` and `input_types` lengths differ.
    InvalidOutput,

    /// A cell belonging to an existing output (see `Sheet::add_output`) was referenced as
    /// an input to a relationship, conditional, condition, or a second output; was the
    /// target of `Sheet::write`; or an `add_output` call tried to reuse a cell that already
    /// had a relationship or conditional referencing it before becoming an output.
    TerminalCell,
}
```

- [ ] **Step 4: Add the `Display` arms**

In `adam-rs/src/error.rs`, change the end of the `Display` impl from:

```rust
            Error::InvalidConditional => write!(f, "conditional is structurally invalid"),
        }
    }
}
```

to:

```rust
            Error::InvalidConditional => write!(f, "conditional is structurally invalid"),
            Error::InvalidOutput => write!(f, "output is structurally invalid"),
            Error::TerminalCell => write!(
                f,
                "cell belongs to a terminal output and cannot be used as an input or written directly"
            ),
        }
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p adam-rs`
Expected: PASS for all tests, including the four new ones.

- [ ] **Step 6: Format and lint**

Run: `cargo fmt --all`
Run: `cargo clippy -p adam-rs --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add adam-rs/src/error.rs
git commit -m "$(cat <<'EOF'
feat(adam-rs): add InvalidOutput and TerminalCell error variants

Groundwork for output cells: InvalidOutput covers malformed
add_output calls, TerminalCell covers illegal references to a cell
that belongs to an existing output.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `Condition` and `Output` core types

**Files:**
- Create: `adam-rs/src/condition.rs`
- Create: `adam-rs/src/output.rs`
- Modify: `adam-rs/src/lib.rs`

**Interfaces:**
- Consumes: `crate::cell::CellId` (existing), `crate::relationship::RelationshipId` (existing).
- Produces: `pub struct Condition` with `from_fn_1`/`from_fn_2`/`new` constructors and `pub(crate)` fields `inputs: Vec<CellId>`, `input_types: Vec<TypeId>`, `function: Box<dyn Fn(&[&dyn Any]) -> Result<bool, anyhow::Error>>`; `pub struct ConditionId` (slotmap key); `pub(crate) struct ConditionData { name: String, output: OutputId, inputs: Vec<CellId>, input_types: Vec<TypeId>, function: ... }`; `pub struct OutputId` (slotmap key); `pub(crate) struct OutputData { cell: CellId, relationship: RelationshipId, conditions: Vec<ConditionId> }`. Task 4 constructs `OutputData`/`ConditionData` inside `Sheet::add_output`.

- [ ] **Step 1: Write the failing tests**

Create `adam-rs/src/condition.rs` with this content (implementation + its own tests together, matching how `relationship.rs` is structured):

```rust
//! Named boolean checks attached to outputs.
//!
//! Each [`Condition`] is a pure predicate over some set of cells, evaluated after every
//! `Sheet::propagate` to determine whether an output's preconditions currently hold. A
//! condition's inputs may be any cells in the sheet, not only the inputs of the output's
//! writer method. See [`crate::sheet::Sheet::add_output`].

use std::any::{Any, TypeId};

use slotmap::new_key_type;

use crate::cell::CellId;
use crate::output::OutputId;

new_key_type! {
    /// A stable handle to a condition in a [`crate::sheet::Sheet`].
    pub struct ConditionId;
}

/// Type-erased predicate stored inside a [`Condition`].
type ConditionFn = Box<dyn Fn(&[&dyn Any]) -> Result<bool, anyhow::Error>>;

/// A single named boolean check over some set of cells, attached to an output.
#[allow(dead_code)]
pub struct Condition {
    pub(crate) inputs: Vec<CellId>,
    pub(crate) input_types: Vec<TypeId>,
    pub(crate) function: ConditionFn,
}

impl Condition {
    /// Creates a condition from explicit TypeIds and a type-erased predicate.
    ///
    /// - Precondition: `inputs.len() == input_types.len()`.
    pub fn new<F>(inputs: Vec<CellId>, input_types: Vec<TypeId>, f: F) -> Self
    where
        F: Fn(&[&dyn Any]) -> Result<bool, anyhow::Error> + 'static,
    {
        debug_assert_eq!(inputs.len(), input_types.len());
        Condition {
            inputs,
            input_types,
            function: Box::new(f),
        }
    }

    /// Creates a 1-input condition from a typed closure.
    ///
    /// The TypeId for `A` is captured automatically. The condition is validated against
    /// its cell registration when passed to [`crate::sheet::Sheet::add_output`].
    pub fn from_fn_1<A, F>(input: CellId, f: F) -> Self
    where
        A: Any + 'static,
        F: Fn(&A) -> Result<bool, anyhow::Error> + 'static,
    {
        Condition {
            inputs: vec![input],
            input_types: vec![TypeId::of::<A>()],
            function: Box::new(move |args| {
                let a = args[0]
                    .downcast_ref::<A>()
                    .expect("type checked at add_output");
                f(a)
            }),
        }
    }

    /// Creates a 2-input condition from a typed closure.
    ///
    /// `inputs[0]` maps to `A` and `inputs[1]` maps to `B`. TypeIds are captured
    /// automatically. The condition is validated when passed to
    /// [`crate::sheet::Sheet::add_output`].
    pub fn from_fn_2<A, B, F>(inputs: [CellId; 2], f: F) -> Self
    where
        A: Any + 'static,
        B: Any + 'static,
        F: Fn(&A, &B) -> Result<bool, anyhow::Error> + 'static,
    {
        Condition {
            inputs: inputs.to_vec(),
            input_types: vec![TypeId::of::<A>(), TypeId::of::<B>()],
            function: Box::new(move |args| {
                let a = args[0]
                    .downcast_ref::<A>()
                    .expect("type checked at add_output");
                let b = args[1]
                    .downcast_ref::<B>()
                    .expect("type checked at add_output");
                f(a, b)
            }),
        }
    }
}

/// Internal storage for a single condition.
#[allow(dead_code)]
pub(crate) struct ConditionData {
    pub(crate) name: String,
    pub(crate) output: OutputId,
    pub(crate) inputs: Vec<CellId>,
    pub(crate) input_types: Vec<TypeId>,
    pub(crate) function: ConditionFn,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn condition_id_is_copy() {
        fn takes_copy<T: Copy>(_: T) {}
        takes_copy(ConditionId::default());
    }

    #[test]
    fn condition_new_stores_types_and_cell_ids() {
        use slotmap::SlotMap;

        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());
        let b = map.insert(());

        let condition = Condition::new(
            vec![a, b],
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
            |args| {
                let x = args[0].downcast_ref::<i32>().unwrap();
                let y = args[1].downcast_ref::<i32>().unwrap();
                Ok(x + y <= 10)
            },
        );

        assert_eq!(condition.inputs, vec![a, b]);
        assert_eq!(
            condition.input_types,
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()]
        );

        let x: i32 = 3;
        let y: i32 = 4;
        assert!((condition.function)(&[&x, &y]).unwrap());
        let x: i32 = 8;
        let y: i32 = 8;
        assert!(!(condition.function)(&[&x, &y]).unwrap());
    }

    #[test]
    fn from_fn_1_stores_correct_type_ids() {
        use slotmap::SlotMap;

        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());

        let condition = Condition::from_fn_1(a, |x: &i32| Ok(*x <= 5));

        assert_eq!(condition.inputs, vec![a]);
        assert_eq!(condition.input_types, vec![TypeId::of::<i32>()]);

        let x: i32 = 3;
        assert!((condition.function)(&[&x]).unwrap());
        let x: i32 = 9;
        assert!(!(condition.function)(&[&x]).unwrap());
    }

    #[test]
    fn from_fn_2_stores_correct_type_ids() {
        use slotmap::SlotMap;

        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());
        let b = map.insert(());

        let condition = Condition::from_fn_2([a, b], |x: &i32, y: &i32| Ok(x * y <= 20));

        assert_eq!(condition.inputs, vec![a, b]);
        assert_eq!(
            condition.input_types,
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()]
        );

        let x: i32 = 4;
        let y: i32 = 5;
        assert!((condition.function)(&[&x, &y]).unwrap());
        let x: i32 = 5;
        let y: i32 = 5;
        assert!(!(condition.function)(&[&x, &y]).unwrap());
    }
}
```

Create `adam-rs/src/output.rs` with this content:

```rust
//! Terminal output cells in the property model bipartite graph.
//!
//! An output is a cell written by exactly one method, together with zero or more named
//! [`crate::condition::Condition`]s checked after every `Sheet::propagate`. An output's
//! cell is terminal: it can never be used as an input to another relationship,
//! conditional, condition, or output. See [`crate::sheet::Sheet::add_output`].

use slotmap::new_key_type;

use crate::cell::CellId;
use crate::condition::ConditionId;
use crate::relationship::RelationshipId;

new_key_type! {
    /// A stable handle to an output in a [`crate::sheet::Sheet`].
    pub struct OutputId;
}

/// Internal storage for a single output.
#[allow(dead_code)]
pub(crate) struct OutputData {
    /// The terminal cell this output writes.
    pub(crate) cell: CellId,
    /// The single-method relationship backing the writer.
    pub(crate) relationship: RelationshipId,
    /// This output's conditions, in declaration order.
    pub(crate) conditions: Vec<ConditionId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_id_is_copy() {
        fn takes_copy<T: Copy>(_: T) {}
        takes_copy(OutputId::default());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs`
Expected: compile error — the crate doesn't yet declare `mod condition;`/`mod output;`, so neither file is part of the crate and their tests aren't picked up (or, depending on how cargo reports it, "file not found for module" if you also try referencing them). This is expected RED until Step 3.

- [ ] **Step 3: Wire the new modules into `lib.rs`**

In `adam-rs/src/lib.rs`, change:

```rust
pub mod cell;
pub mod conditional;
pub mod error;
mod planner;
pub mod relationship;
pub mod sheet;

pub use cell::CellId;
pub use conditional::ConditionalId;
pub use error::Error;
pub use relationship::{Method, RelationshipId};
pub use sheet::Sheet;
```

to:

```rust
pub mod cell;
pub mod condition;
pub mod conditional;
pub mod error;
mod planner;
pub mod output;
pub mod relationship;
pub mod sheet;

pub use cell::CellId;
pub use condition::{Condition, ConditionId};
pub use conditional::ConditionalId;
pub use error::Error;
pub use output::OutputId;
pub use relationship::{Method, RelationshipId};
pub use sheet::Sheet;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-rs`
Expected: PASS for all tests, including `condition_id_is_copy`, `condition_new_stores_types_and_cell_ids`, `from_fn_1_stores_correct_type_ids`, `from_fn_2_stores_correct_type_ids`, `output_id_is_copy`.

- [ ] **Step 5: Format and lint**

Run: `cargo fmt --all`
Run: `cargo clippy -p adam-rs --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add adam-rs/src/condition.rs adam-rs/src/output.rs adam-rs/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(adam-rs): add Condition and Output core types

Condition mirrors Method's shape (inputs, input_types, a type-erased
function) but returns bool instead of writing a cell — the predicate
shape needed for output preconditions. OutputId/OutputData are the
stable-handle/storage pair Sheet::add_output will populate next.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Terminal-cell enforcement in `Sheet`

**Files:**
- Modify: `adam-rs/src/sheet.rs`

**Interfaces:**
- Consumes: `Error::TerminalCell` (Task 1).
- Produces: a private `terminal_cells: HashSet<CellId>` field on `Sheet`; `add_relationship`, `add_conditional`, and `write` all return `Error::TerminalCell` when a referenced cell is in it. Task 4 populates this set for real via `add_output`; this task tests it by inserting into the field directly (white-box), the same way existing tests reach into `sheet.cells[a].strength`.

- [ ] **Step 1: Write the failing tests**

Open `adam-rs/src/sheet.rs`. In the `#[cfg(test)] mod tests` block, add these tests after `add_cell_returns_distinct_ids`:

```rust
    #[test]
    fn write_returns_terminal_cell_for_terminal_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        sheet.terminal_cells.insert(a);
        assert!(matches!(sheet.write(a, 1_i32), Err(Error::TerminalCell)));
    }

    #[test]
    fn add_relationship_returns_terminal_cell_for_terminal_input() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet.terminal_cells.insert(a);
        let result = sheet.add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))]);
        assert!(matches!(result, Err(Error::TerminalCell)));
    }

    #[test]
    fn add_relationship_returns_terminal_cell_for_terminal_output() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet.terminal_cells.insert(b);
        let result = sheet.add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))]);
        assert!(matches!(result, Err(Error::TerminalCell)));
    }

    #[test]
    fn add_conditional_returns_terminal_cell_for_terminal_match_cell() {
        let mut sheet = Sheet::new();
        let p = sheet.add_cell(0_i32);
        sheet.terminal_cells.insert(p);
        let result = sheet.add_conditional::<i32>(p, vec![], vec![]);
        assert!(matches!(result, Err(Error::TerminalCell)));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs`
Expected: compile error — `no field \`terminal_cells\` on type \`Sheet\``.

- [ ] **Step 3: Add the `terminal_cells` field**

In `adam-rs/src/sheet.rs`, change the `Sheet` struct from:

```rust
pub struct Sheet {
    pub(crate) cells: SlotMap<CellId, CellData>,
    pub(crate) relationships: SlotMap<RelationshipId, RelationshipData>,
    pub(crate) changed_cells: Vec<CellId>,
    /// Monotonic counter incremented by both `add_cell` and `write`; cells added
    /// later and cells written later have strictly higher strength, making the
    /// default method-selection direction deterministic.
    next_strength: u64,
    last_plan: Option<Vec<(RelationshipId, usize)>>,
    /// Cells reported forced (see [`Sheet::is_forced`]) by the last full `propagate()`
    /// call. Not recomputed by `propagate_without_replan`.
    last_forced: Option<HashSet<CellId>>,
    /// Relationships reported forced (see [`Sheet::is_relationship_forced`]) by the
    /// last full `propagate()` call. Not recomputed by `propagate_without_replan`.
    last_forced_relationships: Option<HashSet<RelationshipId>>,
    /// All conditionals registered on this sheet.
    pub(crate) conditionals: SlotMap<ConditionalId, ConditionalData>,
    /// Union of all RelationshipIds assigned to any conditional branch or default.
    /// Used to exclude them from the unconditional active set.
    pub(crate) conditional_relationships: HashSet<RelationshipId>,
}
```

to:

```rust
pub struct Sheet {
    pub(crate) cells: SlotMap<CellId, CellData>,
    pub(crate) relationships: SlotMap<RelationshipId, RelationshipData>,
    pub(crate) changed_cells: Vec<CellId>,
    /// Monotonic counter incremented by both `add_cell` and `write`; cells added
    /// later and cells written later have strictly higher strength, making the
    /// default method-selection direction deterministic.
    next_strength: u64,
    last_plan: Option<Vec<(RelationshipId, usize)>>,
    /// Cells reported forced (see [`Sheet::is_forced`]) by the last full `propagate()`
    /// call. Not recomputed by `propagate_without_replan`.
    last_forced: Option<HashSet<CellId>>,
    /// Relationships reported forced (see [`Sheet::is_relationship_forced`]) by the
    /// last full `propagate()` call. Not recomputed by `propagate_without_replan`.
    last_forced_relationships: Option<HashSet<RelationshipId>>,
    /// All conditionals registered on this sheet.
    pub(crate) conditionals: SlotMap<ConditionalId, ConditionalData>,
    /// Union of all RelationshipIds assigned to any conditional branch or default.
    /// Used to exclude them from the unconditional active set.
    pub(crate) conditional_relationships: HashSet<RelationshipId>,
    /// Cells belonging to a registered output (see [`Sheet::add_output`]). Such a cell
    /// can never be referenced as an input to a relationship, conditional, condition, or
    /// another output, and can never be the target of `write`.
    terminal_cells: HashSet<CellId>,
}
```

- [ ] **Step 4: Initialize it in `Sheet::new()`**

In `adam-rs/src/sheet.rs`, change `Sheet::new()` from:

```rust
    pub fn new() -> Self {
        Sheet {
            cells: SlotMap::with_key(),
            relationships: SlotMap::with_key(),
            changed_cells: Vec::new(),
            next_strength: 0,
            last_plan: None,
            last_forced: None,
            last_forced_relationships: None,
            conditionals: SlotMap::with_key(),
            conditional_relationships: HashSet::new(),
        }
    }
```

to:

```rust
    pub fn new() -> Self {
        Sheet {
            cells: SlotMap::with_key(),
            relationships: SlotMap::with_key(),
            changed_cells: Vec::new(),
            next_strength: 0,
            last_plan: None,
            last_forced: None,
            last_forced_relationships: None,
            conditionals: SlotMap::with_key(),
            conditional_relationships: HashSet::new(),
            terminal_cells: HashSet::new(),
        }
    }
```

- [ ] **Step 5: Check terminal cells in `add_relationship`**

In `adam-rs/src/sheet.rs`, change the two per-cell validation loops inside `add_relationship` from:

```rust
            for (&cell_id, &declared) in method.inputs.iter().zip(method.input_types.iter()) {
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                    });
                }
            }

            for (&cell_id, &declared) in method.outputs.iter().zip(method.output_types.iter()) {
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                    });
                }
            }
```

to:

```rust
            for (&cell_id, &declared) in method.inputs.iter().zip(method.input_types.iter()) {
                if self.terminal_cells.contains(&cell_id) {
                    return Err(Error::TerminalCell);
                }
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                    });
                }
            }

            for (&cell_id, &declared) in method.outputs.iter().zip(method.output_types.iter()) {
                if self.terminal_cells.contains(&cell_id) {
                    return Err(Error::TerminalCell);
                }
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                    });
                }
            }
```

- [ ] **Step 6: Check the terminal cell in `add_conditional`**

In `adam-rs/src/sheet.rs`, change the start of `add_conditional` from:

```rust
        let cell_data = self.cells.get(cell).ok_or(Error::InvalidId)?;
        if cell_data.type_id != TypeId::of::<T>() {
            return Err(Error::InvalidConditional);
        }
```

to:

```rust
        let cell_data = self.cells.get(cell).ok_or(Error::InvalidId)?;
        if self.terminal_cells.contains(&cell) {
            return Err(Error::TerminalCell);
        }
        if cell_data.type_id != TypeId::of::<T>() {
            return Err(Error::InvalidConditional);
        }
```

- [ ] **Step 7: Check the terminal cell in `write`**

In `adam-rs/src/sheet.rs`, change the start of `write` from:

```rust
    pub fn write<T: Any + 'static>(&mut self, id: CellId, value: T) -> Result<(), Error> {
        let cell = self.cells.get_mut(id).ok_or(Error::InvalidId)?;
```

to:

```rust
    pub fn write<T: Any + 'static>(&mut self, id: CellId, value: T) -> Result<(), Error> {
        if self.terminal_cells.contains(&id) {
            return Err(Error::TerminalCell);
        }
        let cell = self.cells.get_mut(id).ok_or(Error::InvalidId)?;
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test -p adam-rs`
Expected: PASS for all tests, including the four new ones.

- [ ] **Step 9: Format and lint**

Run: `cargo fmt --all`
Run: `cargo clippy -p adam-rs --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 10: Commit**

```bash
git add adam-rs/src/sheet.rs
git commit -m "$(cat <<'EOF'
feat(adam-rs): enforce terminal cells in add_relationship/add_conditional/write

An output's cell (added in the next commit) must never be usable as an
input elsewhere. terminal_cells tracks which cells are already
terminal; add_relationship, add_conditional, and write now reject any
reference to one with Error::TerminalCell. Sheet::add_output (next)
is what actually populates this set.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: `Sheet::add_output` and accessors

**Files:**
- Modify: `adam-rs/src/sheet.rs`
- Modify: `adam-rs/tests/integration.rs`

**Interfaces:**
- Consumes: `Condition`, `ConditionId`, `ConditionData` (Task 2), `OutputId`, `OutputData` (Task 2), `terminal_cells` enforcement (Task 3).
- Produces: `pub fn Sheet::add_output(&mut self, writer: Method, conditions: Vec<(&str, Condition)>) -> Result<OutputId, Error>`; `pub fn Sheet::output_cell(&self, id: OutputId) -> Option<CellId>`; `pub fn Sheet::output_conditions(&self, id: OutputId) -> Option<&[ConditionId]>`; `pub fn Sheet::condition_name(&self, id: ConditionId) -> Option<&str>`; `pub fn Sheet::condition_output(&self, id: ConditionId) -> Option<OutputId>`; `pub fn Sheet::condition_inputs(&self, id: ConditionId) -> Option<&[CellId]>`. Task 5 reads `self.outputs`/`self.conditions` from `propagate()`.

- [ ] **Step 1: Write the failing tests**

Open `adam-rs/tests/integration.rs`. Change the top-level imports from:

```rust
use std::any::TypeId;

use adam_rs::{Error, Method, Sheet};
```

to:

```rust
use std::any::TypeId;

use adam_rs::{CellId, Condition, ConditionId, Error, Method, OutputId, Sheet};
```

(`HashSet` is not needed yet — Task 6 adds it when it's actually used, to avoid an `unused_imports` warning in the meantime.)

At the end of `adam-rs/tests/integration.rs`, add:

```rust
#[test]
fn add_output_succeeds_with_no_conditions() {
    let mut sheet = Sheet::new();
    let width = sheet.add_cell(0_i32);
    let height = sheet.add_cell(0_i32);
    let area = sheet.add_cell(0_i32);
    let writer = Method::from_fn_2_1([width, height], area, |w: &i32, h: &i32| Ok(w * h));
    let output = sheet
        .add_output(writer, Vec::<(&str, Condition)>::new())
        .unwrap();
    assert_eq!(sheet.output_cell(output), Some(area));
}

#[test]
fn add_output_succeeds_with_one_condition() {
    let mut sheet = Sheet::new();
    let width = sheet.add_cell(0_i32);
    let height = sheet.add_cell(0_i32);
    let max_area = sheet.add_cell(100_i32);
    let area = sheet.add_cell(0_i32);
    let writer = Method::from_fn_2_1([width, height], area, |w: &i32, h: &i32| {
        w.checked_mul(*h).ok_or_else(|| anyhow::anyhow!("overflow"))
    });
    let output = sheet
        .add_output(
            writer,
            vec![(
                "max_area",
                Condition::from_fn_2([area, max_area], |a: &i32, max: &i32| Ok(a <= max)),
            )],
        )
        .unwrap();
    assert_eq!(sheet.output_conditions(output).unwrap().len(), 1);
}

#[test]
fn add_output_succeeds_with_multiple_conditions() {
    let mut sheet = Sheet::new();
    let width = sheet.add_cell(0_i32);
    let height = sheet.add_cell(0_i32);
    let max_width = sheet.add_cell(50_i32);
    let max_height = sheet.add_cell(50_i32);
    let area = sheet.add_cell(0_i32);
    let writer = Method::from_fn_2_1([width, height], area, |w: &i32, h: &i32| {
        w.checked_mul(*h).ok_or_else(|| anyhow::anyhow!("overflow"))
    });
    let output = sheet
        .add_output(
            writer,
            vec![
                (
                    "max_width",
                    Condition::from_fn_2([width, max_width], |w: &i32, max: &i32| Ok(w <= max)),
                ),
                (
                    "max_height",
                    Condition::from_fn_2([height, max_height], |h: &i32, max: &i32| Ok(h <= max)),
                ),
            ],
        )
        .unwrap();
    assert_eq!(sheet.output_conditions(output).unwrap().len(), 2);
}

#[test]
fn add_output_returns_invalid_output_for_writer_with_zero_outputs() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let writer = Method::new(
        vec![a],
        vec![],
        vec![TypeId::of::<i32>()],
        vec![],
        |_| Ok(vec![]),
    );
    let result = sheet.add_output(writer, Vec::<(&str, Condition)>::new());
    assert!(matches!(result, Err(Error::InvalidOutput)));
}

#[test]
fn add_output_returns_invalid_output_for_writer_with_two_outputs() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);
    let writer = Method::new(
        vec![a],
        vec![b, c],
        vec![TypeId::of::<i32>()],
        vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
        |args| {
            let x = args[0].downcast_ref::<i32>().unwrap();
            Ok(vec![Box::new(*x), Box::new(*x)])
        },
    );
    let result = sheet.add_output(writer, Vec::<(&str, Condition)>::new());
    assert!(matches!(result, Err(Error::InvalidOutput)));
}

#[test]
fn add_output_returns_invalid_output_for_duplicate_condition_names() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let writer = Method::from_fn_1_1(a, b, |x: &i32| Ok(*x));
    let result = sheet.add_output(
        writer,
        vec![
            ("check", Condition::from_fn_1(a, |x: &i32| Ok(*x >= 0))),
            ("check", Condition::from_fn_1(a, |x: &i32| Ok(*x < 100))),
        ],
    );
    assert!(matches!(result, Err(Error::InvalidOutput)));
}

#[test]
fn add_output_returns_invalid_output_for_empty_condition_name() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let writer = Method::from_fn_1_1(a, b, |x: &i32| Ok(*x));
    let result = sheet.add_output(
        writer,
        vec![("", Condition::from_fn_1(a, |x: &i32| Ok(*x >= 0)))],
    );
    assert!(matches!(result, Err(Error::InvalidOutput)));
}

#[test]
fn add_output_returns_terminal_cell_when_output_cell_already_has_a_relationship() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    // b already has an incoming relationship before add_output is attempted on it.
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    let c = sheet.add_cell(0_i32);
    let writer = Method::from_fn_1_1(c, b, |x: &i32| Ok(*x));
    let result = sheet.add_output(writer, Vec::<(&str, Condition)>::new());
    assert!(matches!(result, Err(Error::TerminalCell)));
}

#[test]
fn add_output_returns_terminal_cell_when_output_cell_is_a_conditional_match_cell() {
    let mut sheet = Sheet::new();
    let mode = sheet.add_cell(0_i32);
    sheet.add_conditional::<i32>(mode, vec![], vec![]).unwrap();
    let a = sheet.add_cell(0_i32);
    let writer = Method::from_fn_1_1(a, mode, |x: &i32| Ok(*x));
    let result = sheet.add_output(writer, Vec::<(&str, Condition)>::new());
    assert!(matches!(result, Err(Error::TerminalCell)));
}

#[test]
fn add_output_returns_terminal_cell_when_writer_input_is_already_an_output_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_output(
            Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
            Vec::<(&str, Condition)>::new(),
        )
        .unwrap();
    let c = sheet.add_cell(0_i32);
    // b is already terminal; using it as a new writer's input must be rejected.
    let result = sheet.add_output(
        Method::from_fn_1_1(b, c, |x: &i32| Ok(*x)),
        Vec::<(&str, Condition)>::new(),
    );
    assert!(matches!(result, Err(Error::TerminalCell)));
}

#[test]
fn add_output_returns_terminal_cell_when_condition_input_is_already_an_output_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_output(
            Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
            Vec::<(&str, Condition)>::new(),
        )
        .unwrap();
    let c = sheet.add_cell(0_i32);
    let d = sheet.add_cell(0_i32);
    // b is already terminal; referencing it from another output's condition must be rejected.
    let result = sheet.add_output(
        Method::from_fn_1_1(c, d, |x: &i32| Ok(*x)),
        vec![("uses_b", Condition::from_fn_1(b, |x: &i32| Ok(*x >= 0)))],
    );
    assert!(matches!(result, Err(Error::TerminalCell)));
}

#[test]
fn add_output_allows_a_condition_to_reference_the_outputs_own_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let result = sheet.add_output(
        Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
        vec![("positive", Condition::from_fn_1(b, |x: &i32| Ok(*x >= 0)))],
    );
    assert!(result.is_ok());
}

#[test]
fn write_returns_terminal_cell_for_an_output_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_output(
            Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
            Vec::<(&str, Condition)>::new(),
        )
        .unwrap();
    assert!(matches!(sheet.write(b, 5_i32), Err(Error::TerminalCell)));
}

#[test]
fn add_relationship_returns_terminal_cell_for_an_output_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_output(
            Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
            Vec::<(&str, Condition)>::new(),
        )
        .unwrap();
    let c = sheet.add_cell(0_i32);
    let result = sheet.add_relationship(vec![Method::from_fn_1_1(b, c, |x: &i32| Ok(*x))]);
    assert!(matches!(result, Err(Error::TerminalCell)));
}

#[test]
fn output_cell_returns_none_for_invalid_id() {
    let sheet = Sheet::new();
    assert_eq!(sheet.output_cell(OutputId::default()), None);
}

#[test]
fn output_conditions_returns_condition_ids_in_declaration_order() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let output = sheet
        .add_output(
            Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
            vec![
                ("first", Condition::from_fn_1(a, |x: &i32| Ok(*x >= 0))),
                ("second", Condition::from_fn_1(a, |x: &i32| Ok(*x < 100))),
            ],
        )
        .unwrap();
    let ids = sheet.output_conditions(output).unwrap();
    assert_eq!(sheet.condition_name(ids[0]), Some("first"));
    assert_eq!(sheet.condition_name(ids[1]), Some("second"));
}

#[test]
fn condition_output_and_inputs_return_correct_values() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let output = sheet
        .add_output(
            Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
            vec![("check", Condition::from_fn_1(a, |x: &i32| Ok(*x >= 0)))],
        )
        .unwrap();
    let id = sheet.output_conditions(output).unwrap()[0];
    assert_eq!(sheet.condition_output(id), Some(output));
    assert_eq!(sheet.condition_inputs(id), Some([a].as_slice()));
}

#[test]
fn condition_name_output_inputs_return_none_for_invalid_id() {
    let sheet = Sheet::new();
    let id = ConditionId::default();
    assert_eq!(sheet.condition_name(id), None);
    assert_eq!(sheet.condition_output(id), None);
    assert_eq!(sheet.condition_inputs(id), None);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs`
Expected: compile error — `no method named \`add_output\` found for struct \`Sheet\``.

- [ ] **Step 3: Add imports and new `Sheet` fields**

In `adam-rs/src/sheet.rs`, change the top-of-file imports from:

```rust
use std::any::{Any, TypeId};
use std::collections::HashSet;

use slotmap::SlotMap;

use crate::{
    cell::{CellData, CellId},
    conditional::{Branch, ConditionalData, ConditionalId},
    error::Error,
    relationship::{Method, RelationshipId},
};
```

to:

```rust
use std::any::{Any, TypeId};
use std::collections::HashSet;

use slotmap::SlotMap;

use crate::{
    cell::{CellData, CellId},
    condition::{Condition, ConditionData, ConditionId},
    conditional::{Branch, ConditionalData, ConditionalId},
    error::Error,
    output::{OutputData, OutputId},
    relationship::{Method, RelationshipId},
};
```

In `adam-rs/src/sheet.rs`, change the `Sheet` struct's tail (as left by Task 3) from:

```rust
    /// Cells belonging to a registered output (see [`Sheet::add_output`]). Such a cell
    /// can never be referenced as an input to a relationship, conditional, condition, or
    /// another output, and can never be the target of `write`.
    terminal_cells: HashSet<CellId>,
}
```

to:

```rust
    /// Cells belonging to a registered output (see [`Sheet::add_output`]). Such a cell
    /// can never be referenced as an input to a relationship, conditional, condition, or
    /// another output, and can never be the target of `write`.
    terminal_cells: HashSet<CellId>,
    /// All outputs registered on this sheet.
    outputs: SlotMap<OutputId, OutputData>,
    /// All conditions registered on this sheet, across all outputs.
    conditions: SlotMap<ConditionId, ConditionData>,
}
```

Change `Sheet::new()` from:

```rust
            conditional_relationships: HashSet::new(),
            terminal_cells: HashSet::new(),
        }
    }
```

to:

```rust
            conditional_relationships: HashSet::new(),
            terminal_cells: HashSet::new(),
            outputs: SlotMap::with_key(),
            conditions: SlotMap::with_key(),
        }
    }
```

- [ ] **Step 4: Add `add_output` and its accessors**

In `adam-rs/src/sheet.rs`, find the end of `add_conditional`:

```rust
        Ok(self.conditionals.insert(ConditionalData {
            cell,
            branches: typed_branches,
            default,
        }))
    }

    /// Writes a value to a cell, incrementing the cell's write-recency strength.
```

Insert the following block between the closing `}` of `add_conditional` and the doc comment of `write`:

```rust
        Ok(self.conditionals.insert(ConditionalData {
            cell,
            branches: typed_branches,
            default,
        }))
    }

    /// Returns `true` if `id` already has adjacency (a relationship referencing it, or
    /// use as some conditional's match cell) — i.e. it cannot legally become an output's
    /// terminal cell, since that would retroactively violate the terminal invariant for
    /// whatever already references it.
    fn cell_has_prior_use(&self, id: CellId) -> bool {
        self.cells.get(id).is_some_and(|cell| !cell.adj.is_empty())
            || self.conditionals.values().any(|c| c.cell == id)
    }

    /// Registers an output: a cell written by exactly one method, together with zero or
    /// more named conditions checked after every `propagate()`.
    ///
    /// `writer` must have exactly one output cell — that cell becomes terminal: it can
    /// never afterward be referenced as an input to a relationship, conditional,
    /// condition, or another output, nor be the target of `write`. A condition's inputs
    /// may be any cells in the sheet, including the output's own cell, but not a cell that
    /// already belongs to a different output.
    ///
    /// - Precondition: no two conditions in `conditions` share a name.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidOutput` — `writer` does not have exactly one output cell, a
    ///   condition name is empty, or two conditions share a name.
    /// - `Error::TerminalCell` — a condition input is already another output's cell, or
    ///   the writer's output cell already has prior use (see [`Sheet::cell_has_prior_use`])
    ///   and so cannot become terminal.
    /// - `Error::InvalidId` — a cell referenced by `writer` or a condition is not in this
    ///   sheet.
    /// - `Error::TypeMismatch` — a condition input's declared type does not match the
    ///   cell's registered type.
    /// - Any error `add_relationship` can return, for `writer`'s own validation.
    pub fn add_output(
        &mut self,
        writer: Method,
        conditions: Vec<(&str, Condition)>,
    ) -> Result<OutputId, Error> {
        if writer.outputs.len() != 1 {
            return Err(Error::InvalidOutput);
        }
        let output_cell = writer.outputs[0];

        let mut seen_names: HashSet<&str> = HashSet::new();
        for &(name, _) in &conditions {
            if name.is_empty() || !seen_names.insert(name) {
                return Err(Error::InvalidOutput);
            }
        }

        for (_, condition) in &conditions {
            if condition.inputs.len() != condition.input_types.len() {
                return Err(Error::InvalidOutput);
            }
            for (&cell_id, &declared) in condition.inputs.iter().zip(condition.input_types.iter())
            {
                if self.terminal_cells.contains(&cell_id) {
                    return Err(Error::TerminalCell);
                }
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                    });
                }
            }
        }

        if self.cell_has_prior_use(output_cell) {
            return Err(Error::TerminalCell);
        }

        let relationship = self.add_relationship(vec![writer])?;
        self.terminal_cells.insert(output_cell);

        let output_id = self.outputs.insert(OutputData {
            cell: output_cell,
            relationship,
            conditions: Vec::new(),
        });

        let condition_ids: Vec<ConditionId> = conditions
            .into_iter()
            .map(|(name, condition)| {
                self.conditions.insert(ConditionData {
                    name: name.to_string(),
                    output: output_id,
                    inputs: condition.inputs,
                    input_types: condition.input_types,
                    function: condition.function,
                })
            })
            .collect();
        self.outputs[output_id].conditions = condition_ids;

        Ok(output_id)
    }

    /// Returns the terminal cell backing output `id`. Read its value with [`Sheet::read`].
    ///
    /// Returns `None` if `id` is not a live output in this sheet.
    pub fn output_cell(&self, id: OutputId) -> Option<CellId> {
        self.outputs.get(id).map(|o| o.cell)
    }

    /// Returns the conditions registered on output `id`, in declaration order.
    ///
    /// Returns `None` if `id` is not a live output in this sheet.
    pub fn output_conditions(&self, id: OutputId) -> Option<&[ConditionId]> {
        self.outputs.get(id).map(|o| o.conditions.as_slice())
    }

    /// Returns the name of condition `id`.
    ///
    /// Returns `None` if `id` is not a live condition in this sheet.
    pub fn condition_name(&self, id: ConditionId) -> Option<&str> {
        self.conditions.get(id).map(|c| c.name.as_str())
    }

    /// Returns the output that condition `id` belongs to.
    ///
    /// Returns `None` if `id` is not a live condition in this sheet.
    pub fn condition_output(&self, id: ConditionId) -> Option<OutputId> {
        self.conditions.get(id).map(|c| c.output)
    }

    /// Returns the cells condition `id` reads.
    ///
    /// Returns `None` if `id` is not a live condition in this sheet.
    pub fn condition_inputs(&self, id: ConditionId) -> Option<&[CellId]> {
        self.conditions.get(id).map(|c| c.inputs.as_slice())
    }

    /// Writes a value to a cell, incrementing the cell's write-recency strength.
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p adam-rs`
Expected: PASS for all tests, including the eighteen new ones added in Step 1.

- [ ] **Step 6: Format and lint**

Run: `cargo fmt --all`
Run: `cargo clippy -p adam-rs --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add adam-rs/src/sheet.rs adam-rs/tests/integration.rs
git commit -m "$(cat <<'EOF'
feat(adam-rs): add Sheet::add_output and output/condition accessors

add_output wraps the existing add_relationship (reusing its
validation) with the terminal-cell invariant: the writer's output
cell can never have had prior use (a relationship, or use as a
conditional's match cell) before becoming an output, and can never be
referenced anywhere afterward. Conditions may reference any cell in
the sheet, including the output's own cell.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Condition evaluation in `propagate()`

**Files:**
- Modify: `adam-rs/src/sheet.rs`
- Modify: `adam-rs/tests/integration.rs`

**Interfaces:**
- Consumes: `self.conditions: SlotMap<ConditionId, ConditionData>` (Task 4).
- Produces: a private `last_violated: HashMap<OutputId, Vec<ConditionId>>` field; `pub fn Sheet::output_valid(&self, id: OutputId) -> bool`; `pub fn Sheet::violated_conditions(&self, id: OutputId) -> impl Iterator<Item = ConditionId> + '_`.

- [ ] **Step 1: Write the failing tests**

Open `adam-rs/tests/integration.rs`. Add this helper function above the `#[test]` functions (anywhere at module scope, e.g. right after the `use` block):

```rust
fn sheet_with_area_output() -> (Sheet, OutputId, CellId, CellId, CellId) {
    let mut sheet = Sheet::new();
    let width = sheet.add_cell(0_i32);
    let height = sheet.add_cell(0_i32);
    let max_area = sheet.add_cell(100_i32);
    let area = sheet.add_cell(0_i32);
    let writer = Method::from_fn_2_1([width, height], area, |w: &i32, h: &i32| {
        w.checked_mul(*h).ok_or_else(|| anyhow::anyhow!("overflow"))
    });
    let output = sheet
        .add_output(
            writer,
            vec![(
                "max_area",
                Condition::from_fn_2([area, max_area], |a: &i32, max: &i32| Ok(a <= max)),
            )],
        )
        .unwrap();
    (sheet, output, width, height, max_area)
}
```

Add these tests at the end of the file:

```rust
#[test]
fn output_valid_false_before_propagate() {
    let (sheet, output, ..) = sheet_with_area_output();
    assert!(!sheet.output_valid(output));
}

#[test]
fn output_valid_true_when_condition_holds() {
    let (mut sheet, output, width, height, _max_area) = sheet_with_area_output();
    sheet.write(width, 5_i32).unwrap();
    sheet.write(height, 4_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(sheet.output_valid(output));
    assert_eq!(sheet.violated_conditions(output).count(), 0);
}

#[test]
fn output_valid_false_when_condition_fails() {
    let (mut sheet, output, width, height, _max_area) = sheet_with_area_output();
    sheet.write(width, 50_i32).unwrap();
    sheet.write(height, 40_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(!sheet.output_valid(output));
}

#[test]
fn violated_conditions_lists_the_failing_condition() {
    let (mut sheet, output, width, height, _max_area) = sheet_with_area_output();
    sheet.write(width, 50_i32).unwrap();
    sheet.write(height, 40_i32).unwrap();
    sheet.propagate().unwrap();
    let violated: Vec<_> = sheet.violated_conditions(output).collect();
    assert_eq!(violated.len(), 1);
    assert_eq!(sheet.condition_name(violated[0]), Some("max_area"));
}

#[test]
fn output_valid_updates_across_propagate_calls() {
    let (mut sheet, output, width, height, _max_area) = sheet_with_area_output();
    sheet.write(width, 50_i32).unwrap();
    sheet.write(height, 40_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(!sheet.output_valid(output));

    sheet.write(height, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert!(sheet.output_valid(output));
}

#[test]
fn condition_function_error_aborts_propagate_with_method_failed() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_output(
            Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
            vec![(
                "always_errors",
                Condition::from_fn_1(a, |_: &i32| Err(anyhow::anyhow!("check failed"))),
            )],
        )
        .unwrap();
    assert!(matches!(sheet.propagate(), Err(Error::MethodFailed(_))));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs`
Expected: compile error — `no method named \`output_valid\` found for struct \`Sheet\``.

- [ ] **Step 3: Add the `last_violated` field**

In `adam-rs/src/sheet.rs`, change the `HashSet` import to also bring in `HashMap`:

```rust
use std::collections::HashSet;
```

to:

```rust
use std::collections::{HashMap, HashSet};
```

Change the `Sheet` struct's tail (as left by Task 4) from:

```rust
    /// All outputs registered on this sheet.
    outputs: SlotMap<OutputId, OutputData>,
    /// All conditions registered on this sheet, across all outputs.
    conditions: SlotMap<ConditionId, ConditionData>,
}
```

to:

```rust
    /// All outputs registered on this sheet.
    outputs: SlotMap<OutputId, OutputData>,
    /// All conditions registered on this sheet, across all outputs.
    conditions: SlotMap<ConditionId, ConditionData>,
    /// Conditions that evaluated `false` as of the last `propagate()` call, grouped by
    /// output. Sparse: an output with no entry had all its conditions hold. Not
    /// recomputed by `propagate_without_replan`.
    last_violated: HashMap<OutputId, Vec<ConditionId>>,
}
```

Change `Sheet::new()` from:

```rust
            outputs: SlotMap::with_key(),
            conditions: SlotMap::with_key(),
        }
    }
```

to:

```rust
            outputs: SlotMap::with_key(),
            conditions: SlotMap::with_key(),
            last_violated: HashMap::new(),
        }
    }
```

- [ ] **Step 4: Add Phase 6 to `propagate()`**

In `adam-rs/src/sheet.rs`, change the tail of `propagate()` from:

```rust
        for id in previously_derived {
            if let Some(cell) = self.cells.get_mut(id)
                && cell.derived.is_none()
                && !cell.changed
            {
                cell.changed = true;
                self.changed_cells.push(id);
            }
        }

        self.last_forced = Some(plan.forced_outputs);
        self.last_forced_relationships = Some(plan.forced_relationships);
        self.last_plan = Some(plan.execution_order);
        Ok(())
    }
```

to:

```rust
        for id in previously_derived {
            if let Some(cell) = self.cells.get_mut(id)
                && cell.derived.is_none()
                && !cell.changed
            {
                cell.changed = true;
                self.changed_cells.push(id);
            }
        }

        // Phase 6: evaluate every registered condition against current cell values.
        let mut last_violated: HashMap<OutputId, Vec<ConditionId>> = HashMap::new();
        for (condition_id, condition) in self.conditions.iter() {
            let inputs: Vec<&dyn Any> = condition
                .inputs
                .iter()
                .map(|&id| self.cells[id].effective())
                .collect();
            let holds = (condition.function)(&inputs).map_err(Error::MethodFailed)?;
            if !holds {
                last_violated
                    .entry(condition.output)
                    .or_default()
                    .push(condition_id);
            }
        }
        self.last_violated = last_violated;

        self.last_forced = Some(plan.forced_outputs);
        self.last_forced_relationships = Some(plan.forced_relationships);
        self.last_plan = Some(plan.execution_order);
        Ok(())
    }
```

- [ ] **Step 5: Add `output_valid` and `violated_conditions`**

In `adam-rs/src/sheet.rs`, change the end of the `condition_inputs` accessor (added in Task 4) from:

```rust
    pub fn condition_inputs(&self, id: ConditionId) -> Option<&[CellId]> {
        self.conditions.get(id).map(|c| c.inputs.as_slice())
    }

    /// Writes a value to a cell, incrementing the cell's write-recency strength.
```

to:

```rust
    pub fn condition_inputs(&self, id: ConditionId) -> Option<&[CellId]> {
        self.conditions.get(id).map(|c| c.inputs.as_slice())
    }

    /// Returns `true` if every condition on `id` held as of the last `propagate()` call.
    ///
    /// - Precondition: `id` is a live output in this sheet.
    ///
    /// Returns `false` if no propagation has run yet.
    pub fn output_valid(&self, id: OutputId) -> bool {
        if self.last_plan.is_none() {
            return false;
        }
        !self.last_violated.contains_key(&id)
    }

    /// Iterates the conditions on `id` that evaluated to `false` as of the last
    /// `propagate()` call.
    ///
    /// - Precondition: `id` is a live output in this sheet.
    ///
    /// - Postcondition: empty if `id` is valid or no propagation has run yet.
    pub fn violated_conditions(&self, id: OutputId) -> impl Iterator<Item = ConditionId> + '_ {
        self.last_violated.get(&id).into_iter().flatten().copied()
    }

    /// Writes a value to a cell, incrementing the cell's write-recency strength.
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p adam-rs`
Expected: PASS for all tests, including the six new ones added in Step 1.

- [ ] **Step 7: Format and lint**

Run: `cargo fmt --all`
Run: `cargo clippy -p adam-rs --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add adam-rs/src/sheet.rs adam-rs/tests/integration.rs
git commit -m "$(cat <<'EOF'
feat(adam-rs): evaluate output conditions every propagate()

Phase 6 runs after the existing five phases: every registered
condition is checked against current cell values and last_violated is
rebuilt from scratch. output_valid/violated_conditions expose the
result, following the same "as of last propagate()" convention as
is_forced. A condition whose function itself errors aborts
propagate() with Error::MethodFailed, distinct from an ordinary
`false` result.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: `contributing_cells` and `condition_contributing_cells`

**Files:**
- Modify: `adam-rs/src/sheet.rs`
- Modify: `adam-rs/tests/integration.rs`

**Interfaces:**
- Consumes: `self.last_plan`, `self.is_source` (existing), `self.conditions` (Task 4).
- Produces: `pub fn Sheet::contributing_cells(&self, id: CellId) -> HashSet<CellId>`; `pub fn Sheet::condition_contributing_cells(&self, id: ConditionId) -> HashSet<CellId>`.

- [ ] **Step 1: Write the failing tests**

Open `adam-rs/tests/integration.rs`. Change the top-level imports from:

```rust
use std::any::TypeId;

use adam_rs::{CellId, Condition, ConditionId, Error, Method, OutputId, Sheet};
```

to:

```rust
use std::any::TypeId;
use std::collections::HashSet;

use adam_rs::{CellId, Condition, ConditionId, Error, Method, OutputId, Sheet};
```

Add these tests at the end of the file:

```rust
#[test]
fn contributing_cells_returns_self_for_plain_source_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(sheet.contributing_cells(a), HashSet::from([a]));
}

#[test]
fn contributing_cells_returns_singleton_before_propagate() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    assert_eq!(sheet.contributing_cells(b), HashSet::from([b]));
}

#[test]
fn contributing_cells_returns_root_sources_for_derived_chain() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(2_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
        .unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(b, c, |x: &i32| Ok(*x + 1))])
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(sheet.contributing_cells(c), HashSet::from([a]));
}

#[test]
fn contributing_cells_includes_self_and_other_inputs_for_self_referencing_cell() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(10_i32);
    let b = sheet.add_cell(3_i32);
    sheet
        .add_relationship(vec![Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| {
            Ok((*x).min(*y))
        })])
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 3);
    let contrib = sheet.contributing_cells(a);
    assert!(contrib.contains(&a));
    assert!(contrib.contains(&b));
}

#[test]
fn contributing_cells_scoped_to_active_conditional_branch() {
    let mut sheet = Sheet::new();
    let p = sheet.add_cell(0_i32);
    let a = sheet.add_cell(1_i32);
    let b = sheet.add_cell(2_i32);
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

    sheet.write(p, 0_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(sheet.contributing_cells(b), HashSet::from([a]));

    sheet.write(p, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(sheet.contributing_cells(a), HashSet::from([b]));
}

#[test]
fn condition_contributing_cells_unions_inputs_outside_writer() {
    let (mut sheet, output, width, height, max_area) = sheet_with_area_output();
    sheet.write(width, 5_i32).unwrap();
    sheet.write(height, 4_i32).unwrap();
    sheet.propagate().unwrap();
    let id = sheet.output_conditions(output).unwrap()[0];
    let contrib = sheet.condition_contributing_cells(id);
    assert_eq!(contrib, HashSet::from([width, height, max_area]));
}

#[test]
fn condition_contributing_cells_returns_empty_for_invalid_id() {
    let sheet = Sheet::new();
    assert_eq!(
        sheet.condition_contributing_cells(ConditionId::default()),
        HashSet::new()
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs`
Expected: compile error — `no method named \`contributing_cells\` found for struct \`Sheet\``.

- [ ] **Step 3: Add `contributing_cells` and `condition_contributing_cells`**

In `adam-rs/src/sheet.rs`, change the end of `violated_conditions` (added in Task 5) from:

```rust
    pub fn violated_conditions(&self, id: OutputId) -> impl Iterator<Item = ConditionId> + '_ {
        self.last_violated.get(&id).into_iter().flatten().copied()
    }

    /// Writes a value to a cell, incrementing the cell's write-recency strength.
```

to:

```rust
    pub fn violated_conditions(&self, id: OutputId) -> impl Iterator<Item = ConditionId> + '_ {
        self.last_violated.get(&id).into_iter().flatten().copied()
    }

    /// Returns the set of root source cells currently determining `id`'s value, as of the
    /// last `propagate()` call.
    ///
    /// Walks backward from `id` through the last plan's selected methods. A
    /// self-referencing input (present in both a method's inputs and its outputs) is
    /// treated as one of its own roots, since it is read at its pre-execution value rather
    /// than derived further.
    ///
    /// - Postcondition: returns `{id}` if no propagation has run yet, or if `id` is
    ///   currently a source.
    ///
    /// - Complexity: O(N) where N is the number of cells reachable upstream of `id`.
    pub fn contributing_cells(&self, id: CellId) -> HashSet<CellId> {
        let mut result = HashSet::new();
        let mut visited: HashSet<CellId> = HashSet::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            if !visited.insert(current) {
                continue;
            }
            if self.is_source(current) {
                result.insert(current);
                continue;
            }
            let producing = self.last_plan.as_ref().and_then(|plan| {
                plan.iter()
                    .find(|&&(rel, idx)| {
                        self.relationships[rel].methods[idx]
                            .outputs
                            .contains(&current)
                    })
                    .copied()
            });
            let Some((rel_id, method_idx)) = producing else {
                result.insert(current);
                continue;
            };
            let method = &self.relationships[rel_id].methods[method_idx];
            for &input in &method.inputs {
                if method.outputs.contains(&input) {
                    result.insert(input);
                } else {
                    stack.push(input);
                }
            }
        }
        result
    }

    /// Returns the union of [`Sheet::contributing_cells`] over condition `id`'s own
    /// declared inputs.
    ///
    /// Returns an empty set if `id` is not a live condition in this sheet.
    ///
    /// - Complexity: O(K·N) where K is the condition's input count and N is the size of
    ///   each input's contributing set.
    pub fn condition_contributing_cells(&self, id: ConditionId) -> HashSet<CellId> {
        let Some(condition) = self.conditions.get(id) else {
            return HashSet::new();
        };
        condition
            .inputs
            .iter()
            .flat_map(|&input| self.contributing_cells(input))
            .collect()
    }

    /// Writes a value to a cell, incrementing the cell's write-recency strength.
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-rs`
Expected: PASS for all tests, including the seven new ones added in Step 1.

- [ ] **Step 5: Format and lint**

Run: `cargo fmt --all`
Run: `cargo clippy -p adam-rs --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add adam-rs/src/sheet.rs adam-rs/tests/integration.rs
git commit -m "$(cat <<'EOF'
feat(adam-rs): add Sheet::contributing_cells

Generalizes the BFS already used internally by add_conditional's
validation into a public, plan-aware query: which root source cells
currently determine a given cell's value. Works on any cell (not only
outputs); condition_contributing_cells unions it over a condition's
own declared inputs, which may include cells outside the output's
writer method.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: Crate documentation example and full workspace verification

**Files:**
- Modify: `adam-rs/src/lib.rs`

**Interfaces:**
- Consumes: everything from Tasks 1-6.
- Produces: nothing new; confirms the branch is ready to hand off per root `CLAUDE.md`'s "Before creating a PR" checklist.

- [ ] **Step 1: Add an outputs-and-conditions doctest**

In `adam-rs/src/lib.rs`, change the end of the crate-level doc comment from:

```rust
//! sheet.write(a, 2.0_f64).unwrap();
//! sheet.write(b, 3.0_f64).unwrap();
//! sheet.propagate().unwrap();
//!
//! assert_eq!(*sheet.read::<f64>(c).unwrap(), 6.0);
//! ```

pub mod cell;
```

to:

```rust
//! sheet.write(a, 2.0_f64).unwrap();
//! sheet.write(b, 3.0_f64).unwrap();
//! sheet.propagate().unwrap();
//!
//! assert_eq!(*sheet.read::<f64>(c).unwrap(), 6.0);
//! ```
//!
//! # Outputs and conditions
//!
//! An output is a terminal cell written by a single method, with named conditions
//! checked after every `propagate()`. Unlike an ordinary derived cell, an output's cell
//! can never be used as an input elsewhere in the sheet.
//!
//! ```rust
//! use adam_rs::{Condition, Method, Sheet};
//!
//! let mut sheet = Sheet::new();
//! let width = sheet.add_cell(0_i32);
//! let height = sheet.add_cell(0_i32);
//! let max_area = sheet.add_cell(100_i32);
//! let area = sheet.add_cell(0_i32);
//!
//! let writer = Method::from_fn_2_1([width, height], area, |w: &i32, h: &i32| {
//!     w.checked_mul(*h).ok_or_else(|| anyhow::anyhow!("overflow"))
//! });
//! let output = sheet
//!     .add_output(
//!         writer,
//!         vec![(
//!             "max_area",
//!             Condition::from_fn_2([area, max_area], |a: &i32, max: &i32| Ok(a <= max)),
//!         )],
//!     )
//!     .unwrap();
//!
//! sheet.write(width, 20_i32).unwrap();
//! sheet.write(height, 3_i32).unwrap();
//! sheet.propagate().unwrap();
//! assert!(sheet.output_valid(output));
//!
//! sheet.write(height, 30_i32).unwrap();
//! sheet.propagate().unwrap();
//! assert!(!sheet.output_valid(output));
//! ```

pub mod cell;
```

- [ ] **Step 2: Run the doctest**

Run: `cargo test --doc -p adam-rs`
Expected: PASS.

- [ ] **Step 3: Format**

Run: `cargo fmt --all`
Expected: no changes (already formatted per-task), or if it does reformat something, stage and include it in the commit below.

- [ ] **Step 4: Build the whole workspace**

Run: `cargo build --workspace`
Expected: builds cleanly, zero warnings.

- [ ] **Step 5: Run the full test suite**

Run: `cargo test --workspace`
Run: `cargo test --doc --workspace`
Expected: all pass, zero warnings, no regressions in `cel-rs`, `cel-runtime`, `cel-parser`, `adam-lang`, `adam-lsp`, or `begin`.

- [ ] **Step 6: Lint the whole workspace**

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Run: `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
Run: `cargo clippy -p begin --all-targets -- -D warnings`
Expected: no warnings from any of the three invocations.

- [ ] **Step 7: Commit**

```bash
git add adam-rs/src/lib.rs
git commit -m "$(cat <<'EOF'
docs(adam-rs): add an outputs-and-conditions example to the crate docs

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

If Step 3 produced additional formatting changes beyond `lib.rs`, stage and include them in this same commit (`git add -u`) rather than a separate one.
