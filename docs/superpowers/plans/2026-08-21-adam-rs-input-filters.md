# Input Filters (adam-rs) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a per-cell input filter to `adam-rs` that transforms/rejects externally-written values and, non-fatally, diagnoses derived values that don't conform.

**Architecture:** A new `filter.rs` module defines `Filter`/`FilterData`/`FilterViolation`. `CellData` gains an inline `Option<FilterData>` (at most one filter per cell, no separate `SlotMap`). `Sheet::write()` and the new `Sheet::add_filter()` both conform-or-reject through the filter before storing a value; `Sheet::propagate()` gains a new, non-gating diagnostic phase that re-checks the filter against method-derived values without ever mutating them.

**Tech Stack:** Rust, `adam-rs` crate only (`slotmap`, `anyhow`, `std::any::Any`) — no new dependencies.

**Spec:** [docs/superpowers/specs/2026-08-21-adam-rs-input-filters-design.md](../specs/2026-08-21-adam-rs-input-filters-design.md)

## Global Constraints

- `cargo fmt --all` must be clean before any commit (enforced by the pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce **zero** compiler
  warnings (not just clippy-clean).
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` must pass.
- Every `pub`/`pub(crate)` function needs a contract-style `///` doc comment: summary
  sentence, `- Precondition:`/`- Postcondition:`/`# Errors`/`- Complexity:` bullets only
  where non-obvious or non-O(1); `debug_assert!` for precondition checks, never runtime
  errors for them.
- Unit tests are derived from the contract/public interface only — never from reading
  the implementation.
- Arithmetic on signed integers uses `checked_*`, not wrapping — not applicable here (no
  new arithmetic), noted for completeness.
- **Deliberate refinement from the spec:** the spec's §2.1 describes `Error::InvalidFilter`
  as covering "cell not found... an argument cell was not found... a type mismatch [on
  args]." Translating that into code surfaced an existing, stricter precedent this plan
  follows instead: `add_relationship`/`add_conditional`/`add_output` all route "id not
  found" through the shared `Error::InvalidId`, "terminal cell" through the shared
  `Error::TerminalCell`, and **argument**-type mismatches through the shared
  `Error::TypeMismatch` — reserving their own `InvalidConditional`/`InvalidOutput`
  variant only for a construct's genuinely own-identity check (e.g. `Conditional`'s own
  match-value type) and structural rules with no generic equivalent. `Error::InvalidFilter`
  follows that same split: it's used only for "cell already has a filter" and "the
  filter's own value type doesn't match the cell's" — everything else in `add_filter`
  uses the existing shared variants. Flagging this here since it narrows the spec's
  wording; the observable behavior it protects (one catch-all struct-level error) is
  unaffected for anything the spec's own examples described.

---

### Task 1: `Error::InvalidFilter`

**Files:**
- Modify: `adam-rs/src/error.rs`

**Interfaces:**
- Produces: `Error::InvalidFilter` (unit variant), usable everywhere `Error` is already
  matched (the enum is `#[non_exhaustive]` so no other file needs updating for
  exhaustiveness).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module at the bottom of `adam-rs/src/error.rs`, after
`terminal_cell_has_no_source`:

```rust
    #[test]
    fn invalid_filter_display_contains_filter() {
        assert!(Error::InvalidFilter.to_string().contains("filter"));
    }

    #[test]
    fn invalid_filter_has_no_source() {
        assert!(std::error::Error::source(&Error::InvalidFilter).is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs invalid_filter`
Expected: FAIL to compile — `no variant named InvalidFilter found for enum Error`.

- [ ] **Step 3: Add the variant and its `Display` arm**

In `adam-rs/src/error.rs`, add the variant right after `TerminalCell` inside `pub enum
Error`:

```rust
    /// An `add_filter` call is structurally invalid: the cell already has a filter, or
    /// the filter's own value type does not match the cell's registered type. (An
    /// unknown cell, a terminal cell, or an argument-cell type mismatch use the shared
    /// `InvalidId`/`TerminalCell`/`TypeMismatch` variants instead, matching
    /// `add_relationship`/`add_conditional`'s existing convention.)
    InvalidFilter,
```

And add the matching arm inside `impl std::fmt::Display for Error`, right after the
`TerminalCell` arm:

```rust
            Error::InvalidFilter => write!(f, "filter is structurally invalid"),
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-rs invalid_filter`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/error.rs
git commit -m "feat(adam-rs): add Error::InvalidFilter"
```

---

### Task 2: `filter.rs` module — `Filter`, `FilterData`, `FilterViolation`

**Files:**
- Create: `adam-rs/src/filter.rs`

**Interfaces:**
- Consumes: `crate::cell::CellId` (existing).
- Produces:
  - `pub struct Filter(pub(crate) FilterData)` with `pub fn new`, `pub fn from_fn_0`,
    `pub fn from_fn_1`, `pub fn from_fn_2`.
  - `pub(crate) struct FilterData { value_type: TypeId, args: Vec<CellId>, arg_types:
    Vec<TypeId>, function: FilterFn }` — consumed by Task 3's `CellData.filter` field
    and `Sheet::add_filter`, Task 4's `Sheet::write`, and Task 5's `propagate` diagnostic
    phase.
  - `pub enum FilterViolation { NotConformed, Failed(anyhow::Error) }` (derives `Debug`)
    — consumed by Task 5/6.

- [ ] **Step 1: Write the failing tests**

Create `adam-rs/src/filter.rs` with just the test module first (everything else added in
Step 3), so Step 2 shows a real compile failure rather than an empty file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;
    use std::any::TypeId;

    use crate::cell::CellId;

    #[test]
    fn from_fn_0_stores_correct_value_type_and_computes_value() {
        let filter = Filter::from_fn_0(|x: &i32| Ok(*x * 2));
        assert_eq!(filter.0.value_type, TypeId::of::<i32>());
        assert!(filter.0.args.is_empty());
        let x: i32 = 5;
        let result = (filter.0.function)(&x, &[]).unwrap();
        assert_eq!(*result.downcast_ref::<i32>().unwrap(), 10);
    }

    #[test]
    fn from_fn_1_stores_correct_type_ids_and_computes_value() {
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let arg = map.insert(());

        let filter = Filter::from_fn_1(arg, |x: &i32, bound: &i32| Ok((*x).min(*bound)));
        assert_eq!(filter.0.value_type, TypeId::of::<i32>());
        assert_eq!(filter.0.args, vec![arg]);
        assert_eq!(filter.0.arg_types, vec![TypeId::of::<i32>()]);

        let x: i32 = 50;
        let bound: i32 = 10;
        let result = (filter.0.function)(&x, &[&bound]).unwrap();
        assert_eq!(*result.downcast_ref::<i32>().unwrap(), 10);
    }

    #[test]
    fn from_fn_2_stores_correct_type_ids_and_computes_value() {
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let lo = map.insert(());
        let hi = map.insert(());

        let filter = Filter::from_fn_2([lo, hi], |x: &i32, lo: &i32, hi: &i32| {
            Ok((*x).clamp(*lo, *hi))
        });
        assert_eq!(filter.0.value_type, TypeId::of::<i32>());
        assert_eq!(filter.0.args, vec![lo, hi]);
        assert_eq!(
            filter.0.arg_types,
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()]
        );

        let x: i32 = 500;
        let lo_v: i32 = 0;
        let hi_v: i32 = 100;
        let result = (filter.0.function)(&x, &[&lo_v, &hi_v]).unwrap();
        assert_eq!(*result.downcast_ref::<i32>().unwrap(), 100);
    }

    #[test]
    fn from_fn_0_reports_the_error_a_failing_function_returns() {
        let filter = Filter::from_fn_0(|_x: &i32| Err(anyhow::anyhow!("cannot conform")));
        let x: i32 = 1;
        let err = (filter.0.function)(&x, &[]).unwrap_err();
        assert_eq!(err.to_string(), "cannot conform");
    }

    #[test]
    fn new_stores_explicit_value_type_and_arg_types() {
        let filter = Filter::new(
            TypeId::of::<i32>(),
            vec![],
            vec![],
            |value, _args| {
                let v = value.downcast_ref::<i32>().unwrap();
                Ok(Box::new(*v) as Box<dyn std::any::Any>)
            },
        );
        assert_eq!(filter.0.value_type, TypeId::of::<i32>());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs --lib filter::`
Expected: FAIL to compile — `cannot find type Filter in this scope` (the module has no
non-test code yet).

- [ ] **Step 3: Write the implementation**

Prepend this to `adam-rs/src/filter.rs`, above the existing `#[cfg(test)]` block:

```rust
//! Input filters: idempotent, per-cell domain constraints.
//!
//! A [`Filter`] conforms or rejects a value written externally to its cell (see
//! [`crate::sheet::Sheet::write`]), and is re-evaluated as a non-gating diagnostic
//! against a value a relationship's method derives for that cell (see
//! [`crate::sheet::Sheet::propagate`]). See [`crate::sheet::Sheet::add_filter`].

use std::any::{Any, TypeId};

use crate::cell::CellId;

/// Type-erased function stored inside a [`FilterData`].
///
/// Takes the candidate value and a slice of the filter's argument cells' current
/// effective values, and returns the conformed value or an error.
type FilterFn = Box<dyn Fn(&dyn Any, &[&dyn Any]) -> Result<Box<dyn Any>, anyhow::Error>>;

/// An idempotent, per-cell domain constraint with optional dynamic arguments.
///
/// Constructed via [`Filter::from_fn_0`]/[`Filter::from_fn_1`]/[`Filter::from_fn_2`] for
/// the common typed cases, or [`Filter::new`] for the fully type-erased form. Attached
/// to a cell with [`crate::sheet::Sheet::add_filter`].
pub struct Filter(pub(crate) FilterData);

pub(crate) struct FilterData {
    /// The `TypeId` of the value this filter operates on, validated against its cell's
    /// registered type by `add_filter`.
    pub(crate) value_type: TypeId,
    /// Dynamic argument cells, resolved via `effective()` wherever the filter runs.
    pub(crate) args: Vec<CellId>,
    pub(crate) arg_types: Vec<TypeId>,
    pub(crate) function: FilterFn,
}

impl Filter {
    /// Creates a filter from an explicit value `TypeId`, argument `TypeId`s, and a
    /// type-erased function.
    ///
    /// - Precondition: `args.len() == arg_types.len()`.
    /// - Precondition: `f` returns a value whose runtime type matches `value_type`.
    #[must_use]
    pub fn new<F>(value_type: TypeId, args: Vec<CellId>, arg_types: Vec<TypeId>, f: F) -> Self
    where
        F: Fn(&dyn Any, &[&dyn Any]) -> Result<Box<dyn Any>, anyhow::Error> + 'static,
    {
        debug_assert_eq!(args.len(), arg_types.len());
        Filter(FilterData {
            value_type,
            args,
            arg_types,
            function: Box::new(f),
        })
    }

    /// Creates a filter with no dynamic arguments from a typed closure.
    ///
    /// The `TypeId` for `T` is captured automatically. The filter is validated against
    /// its cell registration when passed to [`crate::sheet::Sheet::add_filter`].
    #[must_use]
    pub fn from_fn_0<T, F>(f: F) -> Self
    where
        T: Any + 'static,
        F: Fn(&T) -> Result<T, anyhow::Error> + 'static,
    {
        Filter::new(TypeId::of::<T>(), vec![], vec![], move |value, _args| {
            let value = value
                .downcast_ref::<T>()
                .expect("type checked at add_filter");
            Ok(Box::new(f(value)?) as Box<dyn Any>)
        })
    }

    /// Creates a filter with one dynamic argument cell from a typed closure.
    ///
    /// `TypeId`s for `A` and `T` are captured automatically. The filter is validated
    /// against its cell registration when passed to [`crate::sheet::Sheet::add_filter`].
    #[must_use]
    pub fn from_fn_1<A, T, F>(arg: CellId, f: F) -> Self
    where
        A: Any + 'static,
        T: Any + 'static,
        F: Fn(&T, &A) -> Result<T, anyhow::Error> + 'static,
    {
        Filter::new(
            TypeId::of::<T>(),
            vec![arg],
            vec![TypeId::of::<A>()],
            move |value, args| {
                let value = value
                    .downcast_ref::<T>()
                    .expect("type checked at add_filter");
                let a = args[0]
                    .downcast_ref::<A>()
                    .expect("type checked at add_filter");
                Ok(Box::new(f(value, a)?) as Box<dyn Any>)
            },
        )
    }

    /// Creates a filter with two dynamic argument cells from a typed closure.
    ///
    /// `args[0]` maps to `A` and `args[1]` maps to `B`. `TypeId`s for `A`, `B`, and `T`
    /// are captured automatically. The filter is validated when passed to
    /// [`crate::sheet::Sheet::add_filter`].
    #[must_use]
    pub fn from_fn_2<A, B, T, F>(args: [CellId; 2], f: F) -> Self
    where
        A: Any + 'static,
        B: Any + 'static,
        T: Any + 'static,
        F: Fn(&T, &A, &B) -> Result<T, anyhow::Error> + 'static,
    {
        Filter::new(
            TypeId::of::<T>(),
            args.to_vec(),
            vec![TypeId::of::<A>(), TypeId::of::<B>()],
            move |value, cell_args| {
                let value = value
                    .downcast_ref::<T>()
                    .expect("type checked at add_filter");
                let a = cell_args[0]
                    .downcast_ref::<A>()
                    .expect("type checked at add_filter");
                let b = cell_args[1]
                    .downcast_ref::<B>()
                    .expect("type checked at add_filter");
                Ok(Box::new(f(value, a, b)?) as Box<dyn Any>)
            },
        )
    }
}

/// The outcome of re-checking a filter against a value a relationship's method
/// derived, rather than a value written externally.
///
/// See [`crate::sheet::Sheet::filter_violation`].
#[derive(Debug)]
pub enum FilterViolation {
    /// The filter succeeded but its output differs from the cell's current value.
    NotConformed,
    /// The filter's function itself returned an error, or returned a value of a
    /// different type than the cell — both treated as an equally soft diagnostic (see
    /// the design spec §4 for why a filter's own `Err` is not a propagation-aborting
    /// failure the way a `Condition`'s is).
    Failed(anyhow::Error),
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-rs --lib filter::`
Expected: PASS (5 tests).

- [ ] **Step 5: Wire the module into the crate root (needed for the tests above to even
  compile as part of the crate, and for Task 3 onward to reference it)**

In `adam-rs/src/lib.rs`, add the module declaration in alphabetical order among the
existing ones:

```rust
pub mod cell;
pub mod condition;
pub mod conditional;
pub mod error;
pub mod filter;
pub mod output;
mod planner;
pub mod relationship;
pub mod sheet;
```

(The `pub use filter::{Filter, FilterViolation};` re-export is added in Task 7, once
`Sheet` actually has methods that use them — adding the re-export now would be dead
code the compiler warns about, violating the zero-warnings constraint.)

- [ ] **Step 6: Run the full test suite to confirm nothing else broke**

Run: `cargo test -p adam-rs`
Expected: PASS, same test count as before plus the 5 new ones.

- [ ] **Step 7: Commit**

```bash
git add adam-rs/src/filter.rs adam-rs/src/lib.rs
git commit -m "feat(adam-rs): add Filter/FilterData/FilterViolation"
```

---

### Task 3: `CellData.filter` field and `Sheet::add_filter`

**Files:**
- Modify: `adam-rs/src/cell.rs`
- Modify: `adam-rs/src/sheet.rs`

**Interfaces:**
- Consumes: `crate::filter::{Filter, FilterData}` (Task 2).
- Produces:
  - `CellData.filter: Option<FilterData>` (`pub(crate)`), read by Task 4 (`write`) and
    Task 5 (`propagate`'s diagnostic phase) and Task 6 (`filter_args`).
  - `Sheet::add_filter(&mut self, cell: CellId, filter: Filter) -> Result<(), Error>`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module at the bottom of `adam-rs/src/sheet.rs` (find the closing
`}` of `mod tests` and add before it; these tests use `Filter`, imported alongside the
other test-module imports at the top of that `mod tests` block — add `crate::Filter` to
that existing `use` statement):

```rust
    #[test]
    fn add_filter_conforms_the_cells_current_value_immediately() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(500_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 100);
    }

    #[test]
    fn add_filter_leaves_a_conforming_value_unchanged() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    }

    #[test]
    fn add_filter_returns_method_failed_when_current_value_cannot_conform() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let result = sheet.add_filter(
            a,
            Filter::from_fn_0(|_x: &i32| Err(anyhow::anyhow!("cannot conform"))),
        );
        assert!(matches!(result, Err(Error::MethodFailed(_))));
        // Rejected: the cell's original value must survive untouched.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    }

    #[test]
    fn add_filter_returns_invalid_id_for_missing_cell() {
        let mut sheet = Sheet::new();
        let result = sheet.add_filter(CellId::default(), Filter::from_fn_0(|x: &i32| Ok(*x)));
        assert!(matches!(result, Err(Error::InvalidId)));
    }

    #[test]
    fn add_filter_returns_terminal_cell_for_an_output_cell() {
        let mut sheet = Sheet::new();
        let writer_input = sheet.add_cell(1_i32);
        let out_cell = sheet.add_cell(0_i32);
        let out = sheet
            .add_output(
                Method::from_fn_1_1(writer_input, out_cell, |x: &i32| Ok(*x)),
                vec![],
            )
            .unwrap();
        let terminal = sheet.output_cell(out).unwrap();
        let result = sheet.add_filter(terminal, Filter::from_fn_0(|x: &i32| Ok(*x)));
        assert!(matches!(result, Err(Error::TerminalCell)));
    }

    #[test]
    fn add_filter_returns_invalid_filter_when_cell_already_has_a_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok(*x)))
            .unwrap();
        let result = sheet.add_filter(a, Filter::from_fn_0(|x: &i32| Ok(*x)));
        assert!(matches!(result, Err(Error::InvalidFilter)));
    }

    #[test]
    fn add_filter_returns_invalid_filter_for_mismatched_value_type() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let result = sheet.add_filter(a, Filter::from_fn_0(|x: &f64| Ok(*x)));
        assert!(matches!(result, Err(Error::InvalidFilter)));
    }

    #[test]
    fn add_filter_returns_invalid_id_for_missing_arg_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let result = sheet.add_filter(
            a,
            Filter::from_fn_1(CellId::default(), |x: &i32, bound: &i32| Ok((*x).min(*bound))),
        );
        assert!(matches!(result, Err(Error::InvalidId)));
    }

    #[test]
    fn add_filter_returns_type_mismatch_for_wrong_arg_cell_type() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let bound = sheet.add_cell(1.0_f64); // wrong type: filter declares i32
        let result = sheet.add_filter(
            a,
            Filter::from_fn_1(bound, |x: &i32, bound: &i32| Ok((*x).min(*bound))),
        );
        assert!(matches!(result, Err(Error::TypeMismatch { .. })));
    }

    #[test]
    fn add_filter_resolves_a_dynamic_argument_cells_current_value() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(500_i32);
        let bound = sheet.add_cell(10_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |x: &i32, bound: &i32| Ok((*x).min(*bound))),
            )
            .unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 10);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs add_filter`
Expected: FAIL to compile — `no method named add_filter found for struct Sheet`.

- [ ] **Step 3: Add the `filter` field to `CellData`**

In `adam-rs/src/cell.rs`, add the import and field:

```rust
use crate::filter::FilterData;
```

(add this alongside the existing `use crate::relationship::RelationshipId;` line), and
inside `pub(crate) struct CellData { ... }`, add after `eq_fn`:

```rust
    /// This cell's filter, if one is attached via `Sheet::add_filter`. At most one per
    /// cell.
    pub(crate) filter: Option<FilterData>,
```

Then fix the pre-existing struct literal in `cell.rs`'s own `cell_data_initial_state`
test (in its `#[cfg(test)] mod tests` block) — add `filter: None,` after the `eq_fn:
...` line in that literal, so `cell.rs`'s own tests keep compiling.

- [ ] **Step 4: Initialize the field at every other `CellData` construction site**

In `adam-rs/src/sheet.rs`, `Sheet::add_cell` constructs a `CellData` literal — add
`filter: None,` after its `eq_fn: ...` line.

- [ ] **Step 5: Implement `Sheet::add_filter`**

In `adam-rs/src/sheet.rs`, add the import (`Filter` alongside the module's existing
`use crate::{ ... }` block — add `filter::{Filter, FilterData},` in alphabetical
position, i.e. right after `error::Error,`), then add the method, placed after
`add_output`'s helper methods and before `output_cell` (i.e. right after the closing
`}` of `add_output`):

```rust
    /// Attaches `filter` to `cell`.
    ///
    /// Immediately applies `filter` to `cell`'s current `source` value, exactly as
    /// [`Sheet::write`] would, so a filtered cell's value is guaranteed to conform from
    /// this call onward — not just from the next external write.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `cell`, or one of `filter`'s argument cells, is not a
    ///   live cell in this sheet.
    /// - `Error::TerminalCell` — `cell` already belongs to an existing output.
    /// - `Error::InvalidFilter` — `cell` already has a filter, or `filter`'s own value
    ///   type does not match `cell`'s registered type.
    /// - `Error::TypeMismatch` — an argument cell's registered type does not match the
    ///   type `filter` declared for it, or (defensively) `filter`'s function returned
    ///   a value of a different type than `cell`'s registered type.
    /// - `Error::MethodFailed` — `filter` rejected `cell`'s current value.
    ///
    /// - Complexity: O(a) where a is the number of `filter`'s argument cells.
    pub fn add_filter(&mut self, cell: CellId, filter: Filter) -> Result<(), Error> {
        let cell_type = self.cells.get(cell).ok_or(Error::InvalidId)?.type_id;
        if self.terminal_cells.contains(&cell) {
            return Err(Error::TerminalCell);
        }
        if self.cells[cell].filter.is_some() {
            return Err(Error::InvalidFilter);
        }
        if filter.0.value_type != cell_type {
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

        let args: Vec<&dyn Any> = filter
            .0
            .args
            .iter()
            .map(|&a| self.cells[a].effective())
            .collect();
        let conformed = (filter.0.function)(self.cells[cell].source.as_ref(), &args)
            .map_err(Error::MethodFailed)?;
        if conformed.as_ref().type_id() != cell_type {
            return Err(Error::TypeMismatch {
                expected: cell_type,
                found: conformed.as_ref().type_id(),
            });
        }

        let cell_data = &mut self.cells[cell];
        cell_data.source = conformed;
        cell_data.derived = None;
        cell_data.filter = Some(filter.0);
        Ok(())
    }
```

- [ ] **Step 6: Fix the `add_filter_returns_terminal_cell_for_an_output_cell` test**

That test as drafted in Step 1 references `sheet.add_output` with a self-referencing
writer as a first (throwaway) attempt — simplify it to just the working version before
running anything. Replace the whole test body with:

```rust
    #[test]
    fn add_filter_returns_terminal_cell_for_an_output_cell() {
        let mut sheet = Sheet::new();
        let writer_input = sheet.add_cell(1_i32);
        let out_cell = sheet.add_cell(0_i32);
        let out = sheet
            .add_output(
                Method::from_fn_1_1(writer_input, out_cell, |x: &i32| Ok(*x)),
                vec![],
            )
            .unwrap();
        let terminal = sheet.output_cell(out).unwrap();
        let result = sheet.add_filter(terminal, Filter::from_fn_0(|x: &i32| Ok(*x)));
        assert!(matches!(result, Err(Error::TerminalCell)));
    }
```

(This replaces the erroneous draft from Step 1 — the plan calls it out explicitly so
the two versions are never both present.)

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p adam-rs add_filter`
Expected: PASS (10 tests).

- [ ] **Step 8: Run the full test suite**

Run: `cargo test -p adam-rs`
Expected: PASS, no regressions.

- [ ] **Step 9: Commit**

```bash
git add adam-rs/src/cell.rs adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): add CellData.filter and Sheet::add_filter"
```

---

### Task 4: `Sheet::write()` filter integration

**Files:**
- Modify: `adam-rs/src/sheet.rs`

**Interfaces:**
- Consumes: `CellData.filter` (Task 3).
- Produces: no new public signature — `Sheet::write`'s existing signature and error
  set gain `Error::MethodFailed` (filter rejection) as a new possible `Err`, already
  documented as a general-purpose variant so no doc-comment restructuring is needed
  beyond adding one `# Errors` bullet.

- [ ] **Step 1: Write the failing tests**

Add to `sheet.rs`'s `mod tests`:

```rust
    #[test]
    fn write_conforms_a_value_through_the_cells_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet.write(a, 500_i32).unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 100);
    }

    #[test]
    fn write_rejects_a_value_the_filter_cannot_conform() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        // `add_filter` re-checks the cell's *current* value immediately (see §3.2),
        // so a filter that unconditionally errors would reject at attach time —
        // accept anything up to 100 so attach succeeds, and let the write below (500)
        // be the one that trips the filter. (Task 4's implementation, commit
        // 620b7bbd, made exactly this fix after hitting the same bug live.)
        sheet
            .add_filter(
                a,
                Filter::from_fn_0(|x: &i32| {
                    if *x > 100 {
                        Err(anyhow::anyhow!("value exceeds maximum"))
                    } else {
                        Ok(*x)
                    }
                }),
            )
            .unwrap();
        let result = sheet.write(a, 500_i32);
        assert!(matches!(result, Err(Error::MethodFailed(_))));
        // Rejected write: cell fully untouched.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    }

    #[test]
    fn write_without_a_filter_behaves_exactly_as_before() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        sheet.write(a, 42_i32).unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 42);
    }

    #[test]
    fn write_through_a_filter_still_bumps_strength() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet.write(b, 1_i32).unwrap();
        sheet.write(a, 500_i32).unwrap();
        // `a` was written after `b`, so its strength must be higher even though its
        // stored value was conformed away from what was passed in.
        assert!(sheet.cells[a].strength > sheet.cells[b].strength);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs write_`
Expected: FAIL — `write_conforms_a_value_through_the_cells_filter` and
`write_rejects_a_value_the_filter_cannot_conform` fail their assertions (today's
`write` has no filter step at all, so the value is stored unconformed); the other two
already pass.

- [ ] **Step 3: Implement the filter step in `write`**

Replace the current body of `Sheet::write` in `adam-rs/src/sheet.rs`:

```rust
    pub fn write<T: Any + 'static>(&mut self, id: CellId, value: T) -> Result<(), Error> {
        if self.terminal_cells.contains(&id) {
            return Err(Error::TerminalCell);
        }
        let cell_type = self.cells.get(id).ok_or(Error::InvalidId)?.type_id;
        if cell_type != TypeId::of::<T>() {
            return Err(Error::TypeMismatch {
                expected: cell_type,
                found: TypeId::of::<T>(),
            });
        }

        let boxed: Box<dyn Any> = if let Some(filter) = self.cells[id].filter.as_ref() {
            let args: Vec<&dyn Any> = filter
                .args
                .iter()
                .map(|&a| self.cells[a].effective())
                .collect();
            let conformed = (filter.function)(&value, &args).map_err(Error::MethodFailed)?;
            if conformed.as_ref().type_id() != cell_type {
                return Err(Error::TypeMismatch {
                    expected: cell_type,
                    found: conformed.as_ref().type_id(),
                });
            }
            conformed
        } else {
            Box::new(value)
        };

        self.next_strength += 1;
        let cell = &mut self.cells[id];
        cell.strength = self.next_strength | (1u64 << 63);
        cell.source = boxed;
        cell.derived = None;
        Ok(())
    }
```

Also add one bullet to `write`'s existing `# Errors` doc list:

```rust
    /// - `Error::MethodFailed` — the cell has a filter and it rejected `value`; the
    ///   cell is left completely unchanged (no strength bump, no `source` change).
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-rs write_`
Expected: PASS (4 tests).

- [ ] **Step 5: Run the full test suite**

Run: `cargo test -p adam-rs`
Expected: PASS, no regressions.

- [ ] **Step 6: Commit**

```bash
git add adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): apply a cell's filter on write()"
```

---

### Task 5: `propagate()` derived-value diagnostic phase

**Files:**
- Modify: `adam-rs/src/sheet.rs`

**Interfaces:**
- Consumes: `CellData.filter` (Task 3), `FilterViolation` (Task 2).
- Produces:
  - `Sheet.last_filter_violations: HashMap<CellId, FilterViolation>` (new private
    field), consumed by Task 6's query methods.
  - No change to `propagate`'s or `propagate_without_replan`'s public signatures.

- [ ] **Step 1: Write the failing tests**

Add to `sheet.rs`'s `mod tests`. These reach into `self.last_filter_violations`
directly (a `pub(crate)` field is not needed — the test module is inside the same
crate and already accesses other private fields like `sheet.cells[a].strength` above,
so a private field access here matches existing test style). These tests reference
`FilterViolation` unqualified, so also add it to the test module's own `use crate::{
...};` import list (the one at the very top of `mod tests`, separate from the
file-level import touched in Step 3 below — the test module has no `use super::*;`)
— it becomes `use crate::{ConditionalId, Error, Filter, FilterViolation, MatchExpr,
Method, Sheet, cell::CellId, relationship::RelationshipId};`:

```rust
    #[test]
    fn propagate_reports_no_violation_when_a_derived_value_conforms() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(10_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_filter(b, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 10);
        assert!(sheet.last_filter_violations.is_empty());
    }

    #[test]
    fn propagate_reports_not_conformed_when_a_derived_value_violates_its_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(60_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_filter(b, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        sheet.propagate().unwrap();
        // 60 * 2 = 120, clamp(0, 100) => 100 != 120.
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 120);
        assert!(matches!(
            sheet.last_filter_violations.get(&b),
            Some(FilterViolation::NotConformed)
        ));
    }

    #[test]
    fn propagate_reports_failed_when_the_filter_errors_on_a_derived_value() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(1_i32);
        let b = sheet.add_cell(0_i32);
        // `add_filter` re-checks the cell's *current* value immediately (see §3.2 of
        // the design), so a filter that unconditionally errors would reject at
        // attach time (b's initial value is 0) before propagate() ever runs. Accept
        // exactly 0 so attach succeeds, and let the relationship's derived value (1,
        // copied from `a`) be the one that trips the filter.
        sheet
            .add_filter(
                b,
                Filter::from_fn_0(|x: &i32| {
                    if *x == 0 {
                        Ok(*x)
                    } else {
                        Err(anyhow::anyhow!("cannot conform"))
                    }
                }),
            )
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        // propagate() must not abort even though the filter errors.
        sheet.propagate().unwrap();
        assert!(matches!(
            sheet.last_filter_violations.get(&b),
            Some(FilterViolation::Failed(_))
        ));
    }

    #[test]
    fn propagate_never_flags_a_filtered_cell_that_stayed_a_plain_source() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(60_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet.propagate().unwrap();
        assert!(sheet.last_filter_violations.is_empty());
    }

    #[test]
    fn propagate_without_replan_does_not_recompute_filter_violations() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(60_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_filter(b, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        sheet.propagate().unwrap();
        assert!(sheet.last_filter_violations.contains_key(&b));

        // Rewrite `a` back into range and re-run only the cached plan.
        sheet.write(a, 10_i32).unwrap();
        sheet.propagate_without_replan().unwrap();
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 20);
        // Still reports the *old* violation: propagate_without_replan doesn't
        // recompute it, matching last_violated's existing behavior.
        assert!(sheet.last_filter_violations.contains_key(&b));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs propagate`
Expected: FAIL to compile — `no field last_filter_violations on type Sheet`.

- [ ] **Step 3: Add the `last_filter_violations` field**

In `adam-rs/src/sheet.rs`, add the import (`FilterViolation` alongside the `Filter`
import added in Task 3, so that line becomes
`filter::{Filter, FilterData, FilterViolation},`), add the field to `struct Sheet`
right after `last_violated`:

```rust
    /// Filter violations recorded against a derived value as of the last `propagate()`
    /// call. Not recomputed by `propagate_without_replan`, consistent with
    /// `last_violated`.
    last_filter_violations: HashMap<CellId, FilterViolation>,
```

and initialize it in `Sheet::new()` right after `last_violated: HashMap::new(),`:

```rust
            last_filter_violations: HashMap::new(),
```

- [ ] **Step 4: Add the diagnostic phase to `propagate()`**

In `adam-rs/src/sheet.rs`, inside `propagate()`, insert this block immediately after
`self.last_violated = last_violated;` and before `self.last_forced = Some(plan.forced_outputs);`:

```rust
        // Phase 6b: evaluate every filter against a value derived by a method this
        // round — a non-gating diagnostic. A filter is never re-checked against a
        // value that came from a plain external write: `write`/`add_filter` already
        // conformed it, and nothing here ever mutates a cell.
        let mut derived_this_round: HashSet<CellId> = HashSet::new();
        for &(rel_id, method_idx) in &plan.execution_order {
            if let Some(method) = self
                .relationships
                .get(rel_id)
                .and_then(|r| r.methods.get(method_idx))
            {
                derived_this_round.extend(method.outputs.iter().copied());
            }
        }
        let mut last_filter_violations: HashMap<CellId, FilterViolation> = HashMap::new();
        for &cell_id in &derived_this_round {
            let Some(filter) = self.cells[cell_id].filter.as_ref() else {
                continue;
            };
            let args: Vec<&dyn Any> = filter
                .args
                .iter()
                .map(|&a| self.cells[a].effective())
                .collect();
            let current = self.cells[cell_id].effective();
            match (filter.function)(current, &args) {
                Ok(conformed) => {
                    let cell = &self.cells[cell_id];
                    if conformed.as_ref().type_id() != cell.type_id {
                        last_filter_violations.insert(
                            cell_id,
                            FilterViolation::Failed(anyhow::anyhow!(
                                "filter returned a value of a different type than the cell"
                            )),
                        );
                    } else if !(cell.eq_fn)(conformed.as_ref(), current) {
                        last_filter_violations.insert(cell_id, FilterViolation::NotConformed);
                    }
                }
                Err(e) => {
                    last_filter_violations.insert(cell_id, FilterViolation::Failed(e));
                }
            }
        }
        self.last_filter_violations = last_filter_violations;

```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p adam-rs propagate`
Expected: PASS.

- [ ] **Step 6: Run the full test suite**

Run: `cargo test -p adam-rs`
Expected: PASS, no regressions.

- [ ] **Step 7: Commit**

```bash
git add adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): diagnose filter violations on derived values in propagate()"
```

---

### Task 6: Query API — `filter_args`, `filter_violation`, `filter_violated_cells`, `filter_violation_cells`

**Files:**
- Modify: `adam-rs/src/sheet.rs`

**Interfaces:**
- Consumes: `CellData.filter` (Task 3), `Sheet.last_filter_violations` (Task 5),
  `Sheet::contributing_cells` (existing).
- Produces: the four public methods below, consumed by Task 7's crate-doc example.

- [ ] **Step 1: Write the failing tests**

Add to `sheet.rs`'s `mod tests`:

```rust
    #[test]
    fn filter_args_returns_the_filters_argument_cells() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let bound = sheet.add_cell(10_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |x: &i32, bound: &i32| Ok((*x).min(*bound))),
            )
            .unwrap();
        assert_eq!(sheet.filter_args(a), Some(&[bound][..]));
    }

    #[test]
    fn filter_args_returns_none_for_a_cell_with_no_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        assert_eq!(sheet.filter_args(a), None);
    }

    #[test]
    fn filter_args_returns_none_for_an_invalid_cell() {
        let sheet = Sheet::new();
        assert_eq!(sheet.filter_args(CellId::default()), None);
    }

    #[test]
    fn filter_violation_returns_none_before_any_propagate() {
        let sheet = Sheet::new();
        assert!(sheet.filter_violation(CellId::default()).is_none());
    }

    #[test]
    fn filter_violated_cells_reports_a_currently_violated_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(60_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_filter(b, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        sheet.propagate().unwrap();
        assert!(sheet.filter_violated_cells().any(|id| id == b));
        assert!(matches!(
            sheet.filter_violation(b),
            Some(FilterViolation::NotConformed)
        ));
    }

    #[test]
    fn filter_violation_cells_is_empty_when_nothing_is_violated() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(10_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet.propagate().unwrap();
        assert!(sheet.filter_violation_cells().is_empty());
    }

    #[test]
    fn filter_violation_cells_includes_root_causes_of_a_violation() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(60_i32);
        let bound = sheet.add_cell(100_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_filter(
                b,
                Filter::from_fn_1(bound, |x: &i32, bound: &i32| Ok((*x).min(*bound))),
            )
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        sheet.propagate().unwrap();
        let violation_cells = sheet.filter_violation_cells();
        // `b` is forced (its relationship has only one method), so — mirroring
        // `contributing_cells`'s existing semantics — it is `a` and `bound` that
        // appear as the upstream root causes, not `b` itself. `b`'s own membership
        // is already answered by `filter_violated_cells()`, tested separately above.
        assert!(violation_cells.contains(&a));
        assert!(violation_cells.contains(&bound));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-rs filter_`
Expected: FAIL to compile — `no method named filter_args found for struct Sheet`
(and similarly for the other three).

- [ ] **Step 3: Implement the four query methods**

In `adam-rs/src/sheet.rs`, add these right after `filter_violation_cells`'s natural
neighbor — place them immediately after `add_filter` (added in Task 3):

```rust
    /// Returns the argument cells of `id`'s filter, in declaration order.
    ///
    /// Returns `None` if `id` is not a live cell in this sheet, or has no filter.
    pub fn filter_args(&self, id: CellId) -> Option<&[CellId]> {
        self.cells.get(id)?.filter.as_ref().map(|f| f.args.as_slice())
    }

    /// Returns the filter violation recorded for `id` as of the last full
    /// `propagate()` call, if any.
    ///
    /// - Postcondition: `None` if `id` has no filter, `id`'s filter's last-checked
    ///   value held, or no full `propagate()` has run since `id` was last a plain
    ///   external write.
    pub fn filter_violation(&self, id: CellId) -> Option<&FilterViolation> {
        self.last_filter_violations.get(&id)
    }

    /// Iterates cells whose filter is currently violated, as of the last full
    /// `propagate()` call.
    ///
    /// - Complexity: O(n) where n is the number of currently-violated filters.
    pub fn filter_violated_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.last_filter_violations.keys().copied()
    }

    /// Returns the set of root cells currently determining a violated filter's own
    /// value or any of its argument values, as of the last full `propagate()` call —
    /// the same "which upstream cells caused this" query
    /// `condition_contributing_cells`/`output_violation_cells` already provide for
    /// `Condition`.
    ///
    /// - Postcondition: empty if no filter is currently violated.
    /// - Complexity: O(sum of `contributing_cells` cost over every violated filter and
    ///   its argument cells).
    pub fn filter_violation_cells(&self) -> HashSet<CellId> {
        let mut result = HashSet::new();
        for cell_id in self.filter_violated_cells() {
            result.extend(self.contributing_cells(cell_id));
            if let Some(args) = self.filter_args(cell_id) {
                for &arg in args {
                    result.extend(self.contributing_cells(arg));
                }
            }
        }
        result
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-rs filter_`
Expected: PASS (7 tests).

- [ ] **Step 5: Run the full test suite**

Run: `cargo test -p adam-rs`
Expected: PASS, no regressions.

- [ ] **Step 6: Commit**

```bash
git add adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): add filter_args/filter_violation/filter_violated_cells/filter_violation_cells"
```

---

### Task 7: Crate exports and doctest example

**Files:**
- Modify: `adam-rs/src/lib.rs`

**Interfaces:**
- Consumes: `Filter`, `FilterViolation` (Task 2), `Sheet::add_filter`,
  `Sheet::filter_violated_cells` (Tasks 3, 6).
- Produces: `pub use filter::{Filter, FilterViolation};` — the crate's public surface
  for this feature.

- [ ] **Step 1: Add the re-export**

In `adam-rs/src/lib.rs`, add in alphabetical position among the existing `pub use`
lines:

```rust
pub use filter::{Filter, FilterViolation};
```

- [ ] **Step 2: Add a `# Filters` doctest section to the crate-level docs**

In `adam-rs/src/lib.rs`, add this new section after the existing `# Outputs and
conditions` section (i.e. at the end of the crate doc comment, before the `pub mod
cell;` line):

```rust
//!
//! # Filters
//!
//! A filter conforms or rejects a value written externally to its cell. It's also
//! re-checked, as a non-gating diagnostic only, against a value a relationship's
//! method derives for that cell — a derived value is never corrected, only flagged.
//!
//! ```rust
//! use adam_rs::{Filter, Method, Sheet};
//!
//! let mut sheet = Sheet::new();
//! let a = sheet.add_cell(0_i32);
//! let b = sheet.add_cell(0_i32);
//! sheet
//!     .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
//!     .unwrap();
//! sheet
//!     .add_filter(b, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
//!     .unwrap();
//! sheet
//!     .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
//!     .unwrap();
//!
//! // An out-of-range external write is silently conformed...
//! sheet.write(a, 500_i32).unwrap();
//! assert_eq!(*sheet.read::<i32>(a).unwrap(), 100);
//!
//! // ...but a derived value that would fail the same filter is only diagnosed, never
//! // corrected: `b` doubles `a`'s already-conformed value, exceeding the filter's range.
//! sheet.propagate().unwrap();
//! assert_eq!(*sheet.read::<i32>(b).unwrap(), 200);
//! assert!(sheet.filter_violated_cells().any(|id| id == b));
//! ```
```

- [ ] **Step 3: Run the doctests**

Run: `cargo test --doc -p adam-rs`
Expected: PASS, including the new doctest.

- [ ] **Step 4: Run the full test suite**

Run: `cargo test -p adam-rs`
Expected: PASS, no regressions.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/lib.rs
git commit -m "feat(adam-rs): export Filter/FilterViolation, add crate-doc example"
```

---

### Task 8: Full verification sweep

**Files:** none (verification only).

**Interfaces:** none.

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Expected: no diff (or, if it reformats something, review the diff, then re-run to
confirm idempotence).

- [ ] **Step 2: Build the whole workspace and check for warnings**

Run: `cargo build --workspace`
Expected: builds clean, **zero warnings** in the output (not just no errors).

- [ ] **Step 3: Test the whole workspace, including doctests, and check for warnings**

Run: `cargo test --workspace`
Run: `cargo test --doc --workspace`
Expected: all pass, zero warnings in either run's output.

- [ ] **Step 4: Clippy — all three required invocations**

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Run: `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
Run: `cargo clippy -p begin --all-targets -- -D warnings`
Expected: all three pass with no warnings. (`begin` isn't touched by this plan, but
CLAUDE.md requires all three invocations pass before any PR from this branch.)

- [ ] **Step 5: Fix anything Steps 2–4 surfaced**

If any warning or clippy lint appears, fix it in the relevant task's file and re-run
that specific check before moving on. Do not proceed to Step 6 with any warning
outstanding.

- [ ] **Step 6: Final commit, if Step 5 made changes**

```bash
git add -A
git commit -m "chore(adam-rs): fix warnings/lints found by the full verification sweep"
```

If Step 5 made no changes, skip this step — there's nothing to commit.
