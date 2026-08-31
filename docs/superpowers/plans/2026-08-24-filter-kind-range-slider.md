# `FilterKind` + `Sheet` Query API + `begin` Number Field/Slider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Recognize a `RangeInclusive<T>`-typed `filter` expression (`lo..=hi`) as a distinct,
queryable `FilterKind::Range` clamp — rather than an opaque function — and use that in `begin` to
render numeric cells with a dedicated number field plus a live-bounds slider.

**Architecture:** Three layers, in dependency order. `adam-rs::Filter`/`FilterData` gains a
`FilterKind` tag (`Opaque` or `Range { bounds }`) and `Sheet` gains `filter_kind`/`filter_range`
queries (§3 of the design spec). `adam-lang`'s compile phase (`type_registry.rs`, `parser.rs`)
recognizes a `RangeInclusive<T>`-typed filter body structurally, by the expression's inferred
runtime type, and builds a `Filter::range` instead of failing today's "filter must produce"
type check; the CST type checker's separate "`_` must be referenced" diagnostic gains a matching
exception. `begin` (§4) then renders a number field for every numeric cell and a slider,
recomputed from the live filter bounds on every render, when one has a range filter.

**Tech Stack:** Rust (`adam-rs`, `adam-lang`, `begin` crates). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-22-filter-deduction-range-slider-design.md` (§3 and §4).
§1 (deduced filter dependencies + `_`) is merged to `main` via PR #149; §2 (`RangeInclusive<T>`/
`..=` syntax) is merged via the earlier `cel-range-syntax`/`range-expression-precedence-fix`
plans. This plan implements the two pieces the phase 1 handoff
(`docs/superpowers/2026-08-24-filter-deduction-phase-1-handoff.md`) calls out as "Left": §3 (this
plan's Tasks 1-5) and §4 (this plan's Tasks 6-7).

## Global Constraints

- `cargo fmt --all` before every commit (pre-commit hook enforces this).
- `cargo build --workspace` / `cargo test --workspace` must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` must stay clean after
  every task that doesn't touch `begin`; `cargo clippy -p begin --no-default-features --all-targets
  -- -D warnings` and `cargo clippy -p begin --all-targets -- -D warnings` must stay clean after
  every task from Task 6 onward (the first task that touches `begin`).
- Every public function needs a contract-style `///` doc comment (Summary / Preconditions /
  Postconditions / Complexity, per root `CLAUDE.md`); non-trivial private functions too, matching
  this codebase's existing style on the functions this plan touches.
- Unit tests are derived from the contract and public interface only, not the implementation.
- Per `begin/CLAUDE.md`, the UI tasks (6-7) must be verified by actually rendering `begin`
  (`verifying-begin-ui` skill), not just by compiling — this is done once, in Task 8, after both
  UI tasks land.

---

### Task 1: `adam-rs/src/filter.rs` — `FilterKind` enum, `kind` field, `Filter::range`

**Files:**
- Modify: `adam-rs/src/filter.rs`
- Modify: `adam-rs/src/lib.rs`

**Interfaces:**
- Produces: `pub enum FilterKind { Opaque, Range { bounds: Box<dyn Fn(&[&dyn Any]) -> (Box<dyn
  Any>, Box<dyn Any>)> } }`; `FilterData` gains `pub(crate) kind: FilterKind`; `pub fn
  Filter::range<F, B>(value_type: TypeId, args: Vec<CellId>, arg_types: Vec<TypeId>, clamp: F,
  bounds: B) -> Self` where `F: Fn(&dyn Any, &[&dyn Any]) -> Result<Box<dyn Any>, anyhow::Error> +
  'static, B: Fn(&[&dyn Any]) -> (Box<dyn Any>, Box<dyn Any>) + 'static`.

- [ ] **Step 1: Write the failing tests**

In `adam-rs/src/filter.rs`'s `mod tests`, add:

```rust
    #[test]
    fn new_defaults_to_opaque_kind() {
        let filter = Filter::new(TypeId::of::<i32>(), vec![], vec![], |value, _args| {
            Ok(Box::new(*value.downcast_ref::<i32>().unwrap()) as Box<dyn Any>)
        });
        assert!(matches!(filter.0.kind, FilterKind::Opaque));
    }

    #[test]
    fn range_stores_range_kind_and_clamps_via_function() {
        let filter = Filter::range(
            TypeId::of::<i32>(),
            vec![],
            vec![],
            |value: &dyn Any, _args: &[&dyn Any]| {
                let v = *value.downcast_ref::<i32>().unwrap();
                Ok(Box::new(v.clamp(0, 100)) as Box<dyn Any>)
            },
            |_args: &[&dyn Any]| {
                (
                    Box::new(0i32) as Box<dyn Any>,
                    Box::new(100i32) as Box<dyn Any>,
                )
            },
        );
        assert_eq!(filter.0.value_type, TypeId::of::<i32>());
        let x: i32 = 500;
        let result = (filter.0.function)(&x, &[]).unwrap();
        assert_eq!(*result.downcast_ref::<i32>().unwrap(), 100);
        let FilterKind::Range { bounds } = &filter.0.kind else {
            panic!("expected FilterKind::Range");
        };
        let (lo, hi) = bounds(&[]);
        assert_eq!(*lo.downcast_ref::<i32>().unwrap(), 0);
        assert_eq!(*hi.downcast_ref::<i32>().unwrap(), 100);
    }
```

- [ ] **Step 2: Run them to verify they fail to compile**

Run: `cargo test -p adam-rs new_defaults_to_opaque_kind range_stores_range_kind`
Expected: compile error — `FilterKind` and `Filter::range` don't exist yet.

- [ ] **Step 3: Add `FilterKind` and the `kind` field**

In `adam-rs/src/filter.rs`, add above `pub struct Filter`:

```rust
/// What shape of validation/derivation a [`Filter`] performs, beyond its opaque function — set by
/// `adam-lang`'s compile phase when a filter's expression matches a recognized structural form.
/// `Opaque` carries no extra information; consumers that don't care about structure treat every
/// kind identically at write/propagate time — `FilterKind` is purely informational, queried by
/// consumers like `begin`'s UI that want to render a specialized editor without inspecting the
/// filter's function.
pub enum FilterKind {
    /// The filter's expression wasn't a recognized structural form.
    Opaque,
    /// Compiled from a `RangeInclusive<T>`-typed expression (`lo..=hi`). `bounds` re-evaluates
    /// that expression against the filter's current argument values, returning the resulting
    /// `(lo, hi)` as type-erased values of the filtered cell's own type `T`.
    Range {
        bounds: Box<dyn Fn(&[&dyn Any]) -> (Box<dyn Any>, Box<dyn Any>)>,
    },
}
```

Add `kind` to `FilterData`:

```rust
pub(crate) struct FilterData {
    pub(crate) value_type: TypeId,
    pub(crate) args: Vec<CellId>,
    pub(crate) arg_types: Vec<TypeId>,
    pub(crate) function: FilterFn,
    /// What shape of validation/derivation this filter performs, beyond `function` — see
    /// [`FilterKind`]. Purely informational; never consulted by `write`/`propagate`/`add_filter`.
    pub(crate) kind: FilterKind,
}
```

Update `Filter::new` to set `kind: FilterKind::Opaque`:

```rust
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
            kind: FilterKind::Opaque,
        })
    }
```

- [ ] **Step 4: Add `Filter::range`**

Add below `Filter::from_fn_2`:

```rust
    /// Creates a range-clamp filter from an explicit value `TypeId`, argument `TypeId`s, a clamp
    /// function, and a `bounds` re-evaluator — the tagged counterpart of what [`Filter::new`]
    /// builds for [`FilterKind::Opaque`]. `clamp` is `Filter`'s actual per-write/per-propagate
    /// function (called exactly like an opaque filter's); `bounds` is called independently, with
    /// no candidate value, by [`crate::sheet::Sheet::filter_range`].
    ///
    /// - Precondition: `args.len() == arg_types.len()`.
    /// - Precondition: `clamp` returns a value whose runtime type matches `value_type`.
    /// - Precondition: `bounds` returns a pair of values whose runtime type matches `value_type`.
    #[must_use]
    pub fn range<F, B>(
        value_type: TypeId,
        args: Vec<CellId>,
        arg_types: Vec<TypeId>,
        clamp: F,
        bounds: B,
    ) -> Self
    where
        F: Fn(&dyn Any, &[&dyn Any]) -> Result<Box<dyn Any>, anyhow::Error> + 'static,
        B: Fn(&[&dyn Any]) -> (Box<dyn Any>, Box<dyn Any>) + 'static,
    {
        debug_assert_eq!(args.len(), arg_types.len());
        Filter(FilterData {
            value_type,
            args,
            arg_types,
            function: Box::new(clamp),
            kind: FilterKind::Range {
                bounds: Box::new(bounds),
            },
        })
    }
```

- [ ] **Step 5: Re-export `FilterKind`**

In `adam-rs/src/lib.rs`, change:

```rust
pub use filter::{Filter, FilterViolation};
```

to:

```rust
pub use filter::{Filter, FilterKind, FilterViolation};
```

- [ ] **Step 6: Run the new tests, then the full `adam-rs` suite**

Run: `cargo test -p adam-rs new_defaults_to_opaque_kind range_stores_range_kind`
Expected: PASS.

Run: `cargo test -p adam-rs`
Expected: PASS, zero warnings (the four existing `from_fn_*` tests still pass unchanged — they
never inspect `kind`).

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add adam-rs/src/filter.rs adam-rs/src/lib.rs
git commit -m "feat(adam-rs): add FilterKind and Filter::range"
```

---

### Task 2: `adam-rs/src/sheet.rs` — `Sheet::filter_kind`, `Sheet::filter_range`

**Files:**
- Modify: `adam-rs/src/sheet.rs`

**Interfaces:**
- Consumes: `FilterKind` from Task 1; `CellData::filter: Option<FilterData>` (existing),
  `CellData::effective()` (existing, used identically to `add_filter`'s own argument resolution).
- Produces: `pub fn Sheet::filter_kind(&self, id: CellId) -> Option<&FilterKind>`; `pub fn
  Sheet::filter_range<T: Any + Clone>(&self, id: CellId) -> Option<(T, T)>`.

- [ ] **Step 1: Write the failing tests**

In `adam-rs/src/sheet.rs`'s `mod tests` (near the existing filter-related tests), add:

```rust
    #[test]
    fn filter_kind_returns_none_for_a_cell_with_no_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        assert!(sheet.filter_kind(a).is_none());
    }

    #[test]
    fn filter_kind_returns_opaque_for_a_plain_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok(*x)))
            .unwrap();
        assert!(matches!(sheet.filter_kind(a), Some(FilterKind::Opaque)));
    }

    #[test]
    fn filter_kind_returns_range_for_a_range_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let filter = Filter::range(
            TypeId::of::<i32>(),
            vec![],
            vec![],
            |value, _args| Ok(Box::new(*value.downcast_ref::<i32>().unwrap()) as Box<dyn Any>),
            |_args| {
                (
                    Box::new(0i32) as Box<dyn Any>,
                    Box::new(100i32) as Box<dyn Any>,
                )
            },
        );
        sheet.add_filter(a, filter).unwrap();
        assert!(matches!(sheet.filter_kind(a), Some(FilterKind::Range { .. })));
    }

    #[test]
    fn filter_range_returns_live_bounds_from_argument_cells() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let lo = sheet.add_cell(0_i32);
        let hi = sheet.add_cell(100_i32);
        let filter = Filter::range(
            TypeId::of::<i32>(),
            vec![lo, hi],
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
            |value, args| {
                let v = *value.downcast_ref::<i32>().unwrap();
                let lo = *args[0].downcast_ref::<i32>().unwrap();
                let hi = *args[1].downcast_ref::<i32>().unwrap();
                Ok(Box::new(v.clamp(lo, hi)) as Box<dyn Any>)
            },
            |args| {
                (
                    Box::new(*args[0].downcast_ref::<i32>().unwrap()) as Box<dyn Any>,
                    Box::new(*args[1].downcast_ref::<i32>().unwrap()) as Box<dyn Any>,
                )
            },
        );
        sheet.add_filter(a, filter).unwrap();
        assert_eq!(sheet.filter_range::<i32>(a), Some((0, 100)));
        sheet.write(hi, 10_i32).unwrap();
        assert_eq!(sheet.filter_range::<i32>(a), Some((0, 10)));
    }

    #[test]
    fn filter_range_reflects_a_bound_derived_by_a_relationship_not_just_a_direct_write() {
        // `hi` isn't itself written — its value is derived from `hi_source` via a relationship —
        // exercising `filter_range`'s use of `effective()` (which sees a relationship's derived
        // override), not just `source`.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let lo = sheet.add_cell(0_i32);
        let hi = sheet.add_cell(100_i32);
        let hi_source = sheet.add_cell(100_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(hi_source, hi, |v: &i32| Ok(*v))])
            .unwrap();
        let filter = Filter::range(
            TypeId::of::<i32>(),
            vec![lo, hi],
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
            |value, args| {
                let v = *value.downcast_ref::<i32>().unwrap();
                let lo = *args[0].downcast_ref::<i32>().unwrap();
                let hi = *args[1].downcast_ref::<i32>().unwrap();
                Ok(Box::new(v.clamp(lo, hi)) as Box<dyn Any>)
            },
            |args| {
                (
                    Box::new(*args[0].downcast_ref::<i32>().unwrap()) as Box<dyn Any>,
                    Box::new(*args[1].downcast_ref::<i32>().unwrap()) as Box<dyn Any>,
                )
            },
        );
        sheet.add_filter(a, filter).unwrap();
        sheet.write(hi_source, 20_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(sheet.filter_range::<i32>(a), Some((0, 20)));
    }

    #[test]
    fn filter_range_returns_none_for_an_opaque_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok(*x)))
            .unwrap();
        assert!(sheet.filter_range::<i32>(a).is_none());
    }
```

Add `FilterKind` to this file's existing `use crate::filter::{Filter, FilterViolation};`-style
import if `filter_kind`'s test module doesn't already have a path to it (the production code's own
`use` — see Step 3 — covers the non-test code; tests use `super::*`, so no separate test import is
needed as long as `FilterKind` is `pub` re-exported from `crate::filter` for tests in this same
crate to name via `crate::filter::FilterKind` or the `adam_rs::FilterKind` prelude path already
established in Task 1).

- [ ] **Step 2: Run them to verify they fail to compile**

Run: `cargo test -p adam-rs filter_kind_ filter_range_`
Expected: compile error — `Sheet::filter_kind`/`Sheet::filter_range` don't exist yet.

- [ ] **Step 3: Add the two methods**

In `adam-rs/src/sheet.rs`, add `FilterKind` to the existing filter import:

```rust
use crate::{
    cell::{CellData, CellId},
    conditional::{Branch, ConditionalData, ConditionalId, MatchExpr, MatchSource},
    error::Error,
    filter::{Filter, FilterKind, FilterViolation},
    output::{OutputData, OutputId},
    relationship::{Method, RelationshipData, RelationshipId},
    requirement::{Requirement, RequirementData, RequirementId},
};
```

Add the two methods immediately after `filter_args` (currently around `sheet.rs:590-599`):

```rust
    /// Returns the kind of validation/derivation `id`'s filter performs, if it has one.
    ///
    /// Returns `None` if `id` is not a live cell in this sheet, or has no filter.
    pub fn filter_kind(&self, id: CellId) -> Option<&FilterKind> {
        self.cells.get(id)?.filter.as_ref().map(|f| &f.kind)
    }

    /// Returns `id`'s filter's current `(lo, hi)` bounds, if it has a [`FilterKind::Range`]
    /// filter.
    ///
    /// Resolves the filter's argument cells' current effective values via the same path
    /// [`Sheet::add_filter`] already uses, then calls the filter's `bounds` function.
    ///
    /// Returns `None` if `id` is not a live cell in this sheet, has no filter, or its filter's
    /// kind isn't [`FilterKind::Range`].
    ///
    /// - Complexity: O(a) where a is the number of the filter's argument cells.
    pub fn filter_range<T: Any + Clone>(&self, id: CellId) -> Option<(T, T)> {
        let filter = self.cells.get(id)?.filter.as_ref()?;
        let FilterKind::Range { bounds } = &filter.kind else {
            return None;
        };
        let args: Vec<&dyn Any> = filter
            .args
            .iter()
            .map(|&a| self.cells[a].effective())
            .collect();
        let (lo, hi) = bounds(&args);
        Some((*lo.downcast::<T>().ok()?, *hi.downcast::<T>().ok()?))
    }
```

- [ ] **Step 4: Run the new tests, then the full `adam-rs` suite**

Run: `cargo test -p adam-rs filter_kind_ filter_range_`
Expected: PASS.

Run: `cargo test -p adam-rs`
Expected: PASS, zero warnings.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): add Sheet::filter_kind and Sheet::filter_range"
```

---

### Task 3: `adam-lang/src/type_registry.rs` — `RangeEntry` table for numeric `RangeInclusive<T>`

**Files:**
- Modify: `adam-lang/src/type_registry.rs`

**Interfaces:**
- Consumes: `cel_runtime::DynSegment::call_dyn::<R>` (existing).
- Produces: `pub(crate) struct RangeEntry { pub(crate) element_type_id: TypeId, pub(crate)
  clamp_fn: fn(&mut DynSegment, &dyn Any, &[&dyn Any]) -> anyhow::Result<Box<dyn Any>>,
  pub(crate) bounds_fn: fn(&mut DynSegment, &dyn Any, &[&dyn Any]) -> anyhow::Result<(Box<dyn
  Any>, Box<dyn Any>)> }`; `pub(crate) fn TypeRegistry::range_entry(&self, range_type_id: TypeId)
  -> Option<&RangeEntry>` — populated in `TypeRegistry::new()` for exactly the 14 numeric
  primitives `cel-parser`'s `RANGE_INCLUSIVE_SIGNATURES` registers `..=` for (`u8`, `u16`, `u32`,
  `u64`, `u128`, `usize`, `i8`, `i16`, `i32`, `i64`, `i128`, `isize`, `f32`, `f64`).

- [ ] **Step 1: Write the failing tests**

In `adam-lang/src/type_registry.rs`'s `mod tests`, add:

```rust
    #[test]
    fn range_entry_recognizes_a_registered_numeric_range_inclusive_type() {
        let reg = TypeRegistry::new();
        let entry = reg
            .range_entry(TypeId::of::<std::ops::RangeInclusive<i32>>())
            .expect("i32 range recognized");
        assert_eq!(entry.element_type_id, TypeId::of::<i32>());
    }

    #[test]
    fn range_entry_returns_none_for_a_non_range_type() {
        let reg = TypeRegistry::new();
        assert!(reg.range_entry(TypeId::of::<i32>()).is_none());
    }

    #[test]
    fn range_entry_clamp_fn_clamps_a_value_into_the_evaluated_bounds() {
        let reg = TypeRegistry::new();
        let entry = reg
            .range_entry(TypeId::of::<std::ops::RangeInclusive<i32>>())
            .unwrap();
        let mut segment = DynSegment::new::<()>();
        segment.push_arg::<i32>(1);
        segment.push_arg::<i32>(2);
        segment.op2(|a: i32, b: i32| a..=b).unwrap();
        let value = 500i32;
        let lo = 0i32;
        let hi = 100i32;
        let result = (entry.clamp_fn)(&mut segment, &value, &[&lo, &hi]).unwrap();
        assert_eq!(*result.downcast_ref::<i32>().unwrap(), 100);
    }

    #[test]
    fn range_entry_bounds_fn_returns_the_evaluated_bounds() {
        let reg = TypeRegistry::new();
        let entry = reg
            .range_entry(TypeId::of::<std::ops::RangeInclusive<i32>>())
            .unwrap();
        let mut segment = DynSegment::new::<()>();
        segment.push_arg::<i32>(1);
        segment.push_arg::<i32>(2);
        segment.op2(|a: i32, b: i32| a..=b).unwrap();
        let placeholder = 0i32;
        let lo = 0i32;
        let hi = 100i32;
        let (lo_out, hi_out) = (entry.bounds_fn)(&mut segment, &placeholder, &[&lo, &hi]).unwrap();
        assert_eq!(*lo_out.downcast_ref::<i32>().unwrap(), 0);
        assert_eq!(*hi_out.downcast_ref::<i32>().unwrap(), 100);
    }
```

- [ ] **Step 2: Run them to verify they fail to compile**

Run: `cargo test -p adam-lang range_entry_`
Expected: compile error — `RangeEntry`/`range_entry` don't exist yet.

- [ ] **Step 3: Add `RangeEntry`, the registration helper, and the two dispatch functions**

In `adam-lang/src/type_registry.rs`, add near the top-level type aliases (after `CallDynFn`):

```rust
/// Per-numeric-type support for a `RangeInclusive<T>`-typed filter expression, keyed by the
/// range's own `TypeId` — populated in [`TypeRegistry::new`] for exactly the primitives
/// `cel-parser`'s `..=` operator supports (`cel_parser::op_table`'s `RANGE_INCLUSIVE_SIGNATURES`).
pub(crate) struct RangeEntry {
    /// `T`'s own `TypeId` — compared against a filtered cell's declared type by the caller.
    pub(crate) element_type_id: TypeId,
    /// Evaluates a compiled segment producing a `RangeInclusive<T>` against `value` (the
    /// candidate being conformed — read only if the segment's expression actually references
    /// `_`) and `args` (the filter's deduced cell dependencies, in declaration order), clamping
    /// `value` into the resulting bounds.
    pub(crate) clamp_fn: fn(&mut DynSegment, &dyn Any, &[&dyn Any]) -> anyhow::Result<Box<dyn Any>>,
    /// Evaluates the same kind of segment against `placeholder` (substituted for `_`, which a
    /// recognized range expression's bounds never actually depend on) and `args`, returning the
    /// resulting `(lo, hi)` bounds.
    pub(crate) bounds_fn: fn(
        &mut DynSegment,
        &dyn Any,
        &[&dyn Any],
    ) -> anyhow::Result<(Box<dyn Any>, Box<dyn Any>)>,
}
```

Add the two monomorphized dispatch functions near `call_dyn_impl`:

```rust
/// Evaluates `seg` (producing a `RangeInclusive<T>`) against `value` prepended to `args`, then
/// clamps `value` into the resulting bounds. For [`RangeEntry::clamp_fn`].
fn range_clamp_impl<T: Clone + PartialOrd + 'static>(
    seg: &mut DynSegment,
    value: &dyn Any,
    args: &[&dyn Any],
) -> anyhow::Result<Box<dyn Any>> {
    let mut call_args: Vec<&dyn Any> = Vec::with_capacity(1 + args.len());
    call_args.push(value);
    call_args.extend_from_slice(args);
    let range = seg.call_dyn::<std::ops::RangeInclusive<T>>(&call_args)?;
    let v = value.downcast_ref::<T>().expect("type checked at add_filter");
    let clamped = if v < range.start() {
        range.start().clone()
    } else if v > range.end() {
        range.end().clone()
    } else {
        v.clone()
    };
    Ok(Box::new(clamped) as Box<dyn Any>)
}

/// Evaluates `seg` (producing a `RangeInclusive<T>`) against `placeholder` prepended to `args`,
/// returning the resulting `(lo, hi)` bounds. For [`RangeEntry::bounds_fn`].
fn range_bounds_impl<T: Clone + 'static>(
    seg: &mut DynSegment,
    placeholder: &dyn Any,
    args: &[&dyn Any],
) -> anyhow::Result<(Box<dyn Any>, Box<dyn Any>)> {
    let mut call_args: Vec<&dyn Any> = Vec::with_capacity(1 + args.len());
    call_args.push(placeholder);
    call_args.extend_from_slice(args);
    let range = seg.call_dyn::<std::ops::RangeInclusive<T>>(&call_args)?;
    Ok((
        Box::new(range.start().clone()) as Box<dyn Any>,
        Box::new(range.end().clone()) as Box<dyn Any>,
    ))
}
```

Add the `range_inclusive` field to `TypeRegistry` and the registration helper:

```rust
pub struct TypeRegistry {
    by_name: HashMap<String, TypeEntry>,
    by_type_id: HashMap<TypeId, String>,
    range_inclusive: HashMap<TypeId, RangeEntry>,
}
```

```rust
    /// Registers `RangeInclusive<T>` support for `T`, keyed by `RangeInclusive<T>`'s own
    /// `TypeId`. Private — only called from [`TypeRegistry::new`] for the fixed set of numeric
    /// primitives `cel-parser`'s `..=` operator supports; unlike [`TypeRegistry::register`], this
    /// is not part of the public API a host binary extends for its own custom types, since
    /// `RangeInclusive<T>` recognition is not extensible per-type in this codebase's current
    /// design.
    fn register_range_inclusive<T: Clone + PartialOrd + 'static>(&mut self) {
        self.range_inclusive.insert(
            TypeId::of::<std::ops::RangeInclusive<T>>(),
            RangeEntry {
                element_type_id: TypeId::of::<T>(),
                clamp_fn: range_clamp_impl::<T>,
                bounds_fn: range_bounds_impl::<T>,
            },
        );
    }

    /// Looks up `RangeInclusive<T>` support by the range's own `TypeId`.
    ///
    /// Returns `None` if `range_type_id` is not `RangeInclusive<T>` for any `T` this registry
    /// recognizes range support for (see [`TypeRegistry::new`]).
    pub(crate) fn range_entry(&self, range_type_id: TypeId) -> Option<&RangeEntry> {
        self.range_inclusive.get(&range_type_id)
    }
```

Update `TypeRegistry::new()` to initialize the field and register the 14 numeric types:

```rust
    pub fn new() -> Self {
        let mut r = TypeRegistry {
            by_name: HashMap::new(),
            by_type_id: HashMap::new(),
            range_inclusive: HashMap::new(),
        };
        r.register::<i8>("i8");
        r.register::<i16>("i16");
        r.register::<i32>("i32");
        r.register::<i64>("i64");
        r.register::<i128>("i128");
        r.register::<isize>("isize");
        r.register::<u8>("u8");
        r.register::<u16>("u16");
        r.register::<u32>("u32");
        r.register::<u64>("u64");
        r.register::<u128>("u128");
        r.register::<usize>("usize");
        r.register::<f32>("f32");
        r.register::<f64>("f64");
        r.register::<bool>("bool");
        r.register::<String>("String");
        r.register_range_inclusive::<i8>();
        r.register_range_inclusive::<i16>();
        r.register_range_inclusive::<i32>();
        r.register_range_inclusive::<i64>();
        r.register_range_inclusive::<i128>();
        r.register_range_inclusive::<isize>();
        r.register_range_inclusive::<u8>();
        r.register_range_inclusive::<u16>();
        r.register_range_inclusive::<u32>();
        r.register_range_inclusive::<u64>();
        r.register_range_inclusive::<u128>();
        r.register_range_inclusive::<usize>();
        r.register_range_inclusive::<f32>();
        r.register_range_inclusive::<f64>();
        r
    }
```

- [ ] **Step 4: Run the new tests, then the full `adam-lang` suite**

Run: `cargo test -p adam-lang range_entry_`
Expected: PASS.

Run: `cargo test -p adam-lang`
Expected: PASS, zero warnings.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-lang/src/type_registry.rs
git commit -m "feat(adam-lang): register RangeInclusive<T> support for numeric primitives"
```

---

### Task 4: `adam-lang/src/parser.rs` — recognize a `RangeInclusive`-typed filter body

**Files:**
- Modify: `adam-lang/src/parser.rs`

**Interfaces:**
- Consumes: `TypeRegistry::range_entry` from Task 3; `adam_rs::Filter::range` from Task 1;
  `Self::parse_filter_expr` (unchanged, from §1).
- Produces: `parse_cell_filter` gains the range-recognition branch; no signature change.

- [ ] **Step 1: Write the failing tests**

In `adam-lang/src/parser.rs`'s `mod tests`, add (near the existing `cell_filter_*` tests):

```rust
    #[test]
    fn cell_filter_with_a_range_inclusive_body_clamps_on_write() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str("sheet s { cell a: i32 filter 0..=100; }")
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        assert!(matches!(
            parsed.sheet.filter_kind(a_id),
            Some(adam_rs::FilterKind::Range { .. })
        ));
        parsed.sheet.write(a_id, 500i32).unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 100);
    }

    #[test]
    fn cell_filter_range_does_not_require_underscore() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let result = parser.parse_str("sheet s { cell a: i32 filter 0..=100; }");
        assert!(result.is_ok());
    }

    #[test]
    fn cell_filter_range_bounds_track_cell_dependencies_live() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str(
                "sheet s { cell lo: i32 = 0; cell hi: i32 = 100; cell a: i32 filter lo..=hi; }",
            )
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        let (hi_id, _) = parsed.cell_names["hi"];
        assert_eq!(parsed.sheet.filter_range::<i32>(a_id), Some((0, 100)));
        parsed.sheet.write(hi_id, 10i32).unwrap();
        assert_eq!(parsed.sheet.filter_range::<i32>(a_id), Some((0, 10)));
        parsed.sheet.write(a_id, 500i32).unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 10);
    }

    #[test]
    fn cell_filter_range_with_float_cell_type_works() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str("sheet s { cell a: f64 filter 0.0..=100.0; }")
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        parsed.sheet.write(a_id, 500.0f64).unwrap();
        assert_eq!(*parsed.sheet.read::<f64>(a_id).unwrap(), 100.0);
    }

    #[test]
    fn cell_filter_range_with_mismatched_element_type_is_a_parse_error() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let err = parser.parse_str("sheet s { cell a: f64 filter 0..=100; }");
        assert!(err.is_err());
    }

    #[test]
    fn cell_filter_general_expression_still_compiles_to_opaque_kind() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str("sheet s { cell a: i32 filter if _ < 0 { 0 } else { _ }; }")
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        assert!(matches!(
            parsed.sheet.filter_kind(a_id),
            Some(adam_rs::FilterKind::Opaque)
        ));
    }
```

- [ ] **Step 2: Run them to verify they fail (compile or assert)**

Run: `cargo test -p adam-lang cell_filter_range cell_filter_general_expression_still_compiles_to_opaque_kind`
Expected: `cell_filter_with_a_range_inclusive_body_clamps_on_write`,
`cell_filter_range_does_not_require_underscore`, `cell_filter_range_bounds_track_cell_dependencies_live`,
and `cell_filter_range_with_float_cell_type_works` FAIL (today's code rejects `0..=100` as a type
mismatch and requires `_`); `cell_filter_range_with_mismatched_element_type_is_a_parse_error` and
`cell_filter_general_expression_still_compiles_to_opaque_kind` already PASS (no behavior change
needed for those two — keep them as regression coverage).

- [ ] **Step 3: Rewrite the tail of `parse_cell_filter`**

Replace the body of `parse_cell_filter` from the `underscore_used` check onward (currently
`adam-lang/src/parser.rs:299-353`, i.e. everything after `let (segment, inputs, underscore_used) =
self.parse_filter_expr(ctx, declared_shape)?;`) with:

```rust
        let value_type_id = cell_type_id(declared_shape);
        let output_type_id = segment.peek_output_type_id().ok_or_else(|| {
            ParseError::new(
                format!("cell `{cell_name}`: filter produced no value"),
                cell_span,
            )
        })?;

        let range_entry = self
            .types
            .range_entry(output_type_id)
            .filter(|e| e.element_type_id == value_type_id);

        if range_entry.is_none() && !underscore_used {
            return Err(ParseError::new(
                "filter must reference `_` (the value being filtered)",
                cell_span,
            ));
        }

        let arg_ids: Vec<CellId> = inputs.iter().map(|(_, id, _)| *id).collect();
        let arg_type_ids: Vec<TypeId> = inputs
            .iter()
            .map(|(_, _, shape)| cell_type_id(shape))
            .collect();

        if let Some(entry) = range_entry {
            let default_fn = self
                .types
                .entry_by_type_id(value_type_id)
                .expect("declared cell type registered")
                .default_fn
                .expect("numeric range-filter cell type has a default");
            let placeholder = default_fn();
            let segment = std::rc::Rc::new(RefCell::new(segment));
            let clamp_segment = std::rc::Rc::clone(&segment);
            let bounds_segment = std::rc::Rc::clone(&segment);
            let clamp_fn = entry.clamp_fn;
            let bounds_fn = entry.bounds_fn;

            return Ok(adam_rs::Filter::range(
                value_type_id,
                arg_ids,
                arg_type_ids,
                move |value, args| {
                    clamp_fn(&mut clamp_segment.borrow_mut(), value, args)
                },
                move |args| {
                    bounds_fn(&mut bounds_segment.borrow_mut(), placeholder.as_ref(), args)
                        .expect("range filter body already validated by parse_cell_filter")
                },
            ));
        }

        if output_type_id != value_type_id {
            if self.types.range_entry(output_type_id).is_some() {
                return Err(ParseError::new(
                    format!(
                        "cell `{cell_name}`: filter range bounds must be `{}`",
                        self.types.display_name(declared_shape)
                    ),
                    cell_span,
                ));
            }
            return Err(ParseError::new(
                format!(
                    "cell `{cell_name}`: filter must produce `{}`",
                    self.types.display_name(declared_shape)
                ),
                cell_span,
            ));
        }

        // `call_dyn_fn` is the same monomorphized-per-registered-type dispatcher `build_method`/
        // `build_match_expr` already use for a deduced expression's scalar output.
        let call_fn = self
            .types
            .entry_by_type_id(value_type_id)
            .expect("declared cell type registered")
            .call_dyn_fn;

        // `RefCell`, not a plain `move` capture: `call_fn` takes `&mut DynSegment`, unlike
        // `DynClosure::call_boxed`'s `&self` the old closure-literal path used.
        let segment = RefCell::new(segment);

        Ok(adam_rs::Filter::new(
            value_type_id,
            arg_ids,
            arg_type_ids,
            move |value, args| {
                let mut call_args: Vec<&dyn Any> = Vec::with_capacity(1 + args.len());
                call_args.push(value);
                call_args.extend_from_slice(args);
                call_fn(&mut segment.borrow_mut(), &call_args)
            },
        ))
```

Update the function's doc comment (`# Errors` section) to add: "or, if the expression is
`RangeInclusive`-typed, if its element type doesn't match `declared_shape`."

- [ ] **Step 4: Run the new tests, then the full workspace suite**

Run: `cargo test -p adam-lang cell_filter_range cell_filter_general_expression_still_compiles_to_opaque_kind`
Expected: PASS (6 tests).

Run: `cargo test -p adam-lang`
Expected: PASS, zero warnings — all existing (§1) filter tests still pass unchanged, since
`range_entry` is `None` for every non-`RangeInclusive` output type, preserving the exact old
control flow for them.

Run: `cargo test --workspace`
Expected: PASS, zero warnings.

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-lang/src/parser.rs
git commit -m "feat(adam-lang): build a range-clamp Filter from a RangeInclusive-typed filter body"
```

---

### Task 5: `adam-lang/src/typecheck.rs` — `_` exception for a range-inclusive filter body

**Files:**
- Modify: `adam-lang/src/typecheck.rs`

**Interfaces:**
- Consumes: `cel_parser::Expr` (existing import).
- Produces: `check_filter` skips its "`_` must be referenced" diagnostic when `filter.body` is
  structurally a top-level `range_inclusive` op.

- [ ] **Step 1: Write the failing test**

In `adam-lang/src/typecheck.rs`'s `mod tests`, add:

```rust
    #[test]
    fn filter_range_inclusive_body_does_not_require_underscore() {
        let sheet = parse("sheet s { cell a: i32 filter 0..=100; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p adam-lang filter_range_inclusive_body_does_not_require_underscore`
Expected: FAIL — `diags` contains the "filter must reference `_`" diagnostic (`0..=100` never
references `_`, and today's `check_filter` doesn't yet exempt it).

- [ ] **Step 3: Add the structural check and guard the diagnostic**

In `adam-lang/src/typecheck.rs`, add a small helper immediately above `check_filter` (currently at
`adam-lang/src/typecheck.rs:409`):

```rust
/// Returns `true` if `expr` is itself a top-level `..=` range expression (`Expr::Op { name:
/// "range_inclusive", .. }`) — the one structural shape `check_filter` exempts from its "must
/// reference `_`" requirement, mirroring `adam_lang::parser::AdamParser::parse_cell_filter`'s
/// matching exemption for the same reason: a `lo..=hi` filter body's bounds never depend on the
/// candidate value, only on its own two endpoints. Deliberately checks only the whole body, not
/// any nested sub-expression — matches the runtime layer's own recognition, which is keyed on the
/// *entire compiled expression's* inferred type, not a nested occurrence.
fn is_range_inclusive_body(expr: &Expr) -> bool {
    matches!(expr, Expr::Op { name, .. } if name == "range_inclusive")
}
```

Guard the diagnostic (currently `adam-lang/src/typecheck.rs:463-469`):

```rust
    if !is_range_inclusive_body(&filter.body) && !expr_references_ident(&filter.body, "_") {
        diagnostics.push(ParseError::new_range(
            "filter must reference `_` (the value being filtered)".to_string(),
            filter.span.start,
            filter.span.end,
        ));
    }
```

- [ ] **Step 4: Run the new test, then the full `adam-lang` suite**

Run: `cargo test -p adam-lang filter_range_inclusive_body_does_not_require_underscore`
Expected: PASS.

Run: `cargo test -p adam-lang`
Expected: PASS, zero warnings.

Run: `cargo test --doc --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-lang/src/typecheck.rs
git commit -m "fix(adam-lang): CST type-checker exempts a range-inclusive filter body from the underscore check"
```

---

### Task 6: `begin/src/bridge.rs` — `CellMeta` gains `is_numeric`/`range`

**Files:**
- Modify: `begin/src/bridge.rs`
- Modify: `begin/src/example_source.rs`

**Interfaces:**
- Consumes: `Sheet::filter_kind`/`Sheet::filter_range` from Task 2; `adam_rs::FilterKind` from
  Task 1.
- Produces: `CellMeta` gains `pub is_numeric: bool` and `pub range: Option<Box<dyn Fn(&Sheet) ->
  (f64, f64)>>`; `labels_from_cell_names` gains a leading `sheet: &Sheet` parameter.

- [ ] **Step 1: Write the failing tests**

In `begin/src/bridge.rs`'s `mod tests`, add:

```rust
    #[test]
    fn labels_from_cell_names_marks_numeric_cells_and_leaves_range_none_without_a_filter() {
        use std::any::TypeId;

        let mut sheet = Sheet::new();
        let a = sheet.add_cell(3_i32);
        let b = sheet.add_cell(true);

        let mut cell_names = IndexMap::new();
        cell_names.insert("a".to_string(), (a, TypeShape::Named(TypeId::of::<i32>())));
        cell_names.insert("b".to_string(), (b, TypeShape::Named(TypeId::of::<bool>())));

        let labels = labels_from_cell_names(&sheet, &cell_names);

        assert!(labels.cells[&a].is_numeric);
        assert!(labels.cells[&a].range.is_none());
        assert!(!labels.cells[&b].is_numeric);
    }

    #[test]
    fn labels_from_cell_names_populates_range_for_a_range_filtered_cell() {
        use adam_rs::Filter;
        use std::any::{Any, TypeId};

        let mut sheet = Sheet::new();
        let a = sheet.add_cell(50_i32);
        let filter = Filter::range(
            TypeId::of::<i32>(),
            vec![],
            vec![],
            |value, _args| Ok(Box::new(*value.downcast_ref::<i32>().unwrap()) as Box<dyn Any>),
            |_args| {
                (
                    Box::new(0i32) as Box<dyn Any>,
                    Box::new(100i32) as Box<dyn Any>,
                )
            },
        );
        sheet.add_filter(a, filter).unwrap();

        let mut cell_names = IndexMap::new();
        cell_names.insert("a".to_string(), (a, TypeShape::Named(TypeId::of::<i32>())));

        let labels = labels_from_cell_names(&sheet, &cell_names);

        let range_fn = labels.cells[&a].range.as_ref().expect("range populated");
        assert_eq!(range_fn(&sheet), (0.0, 100.0));
    }
```

- [ ] **Step 2: Run them, and the existing suite, to verify they fail to compile**

Run: `cargo test -p begin labels_from_cell_names`
Expected: compile error — `CellMeta` has no `is_numeric`/`range` fields yet, and the existing
tests' `labels_from_cell_names(&cell_names)` calls (missing the new leading `sheet` argument) also
fail to compile once the signature changes in the next step.

- [ ] **Step 3: Add the fields and a `ToF64Display` helper trait**

In `begin/src/bridge.rs`, add to `CellMeta`:

```rust
pub struct CellMeta {
    pub label: String,
    pub is_bool: bool,
    /// `true` if the cell holds one of the 14 numeric primitive types, so the Inspector can
    /// render it with [`crate::spectrum::SpNumberfield`] instead of a plain text field.
    pub is_numeric: bool,
    pub display: Box<dyn Fn(&Sheet) -> String>,
    pub write_str: WriteStrFn,
    /// Live slider bounds, present only for a numeric cell whose filter is a
    /// [`adam_rs::FilterKind::Range`] — recomputed from the filter's current argument values on
    /// every call, so a range driven by other cells or relationships stays live. Cast to `f64`
    /// for display, matching [`format_rounded`]'s existing all-numeric-types-as-`f64` convention.
    pub range: Option<Box<dyn Fn(&Sheet) -> (f64, f64)>>,
}
```

Update both `Labels::add_cell` and `Labels::add_tuple_cell`'s `CellMeta` literals to add
`is_numeric: false, range: None,` (both default to the non-numeric case; `labels_from_cell_names`
overrides them for numeric types in Step 4).

Add a small conversion trait near `format_rounded`, implemented for exactly the 14 numeric
primitives — mirroring `format_rounded`'s own doc comment, which already documents this codebase's
"treat every numeric type as `f64` for display" convention:

```rust
/// Converts a filter-recognized numeric primitive to `f64` for display — the same "every numeric
/// type displays as `f64`" convention [`format_rounded`] already documents. Implemented for
/// exactly the 14 primitives `TypeRegistry::range_entry` recognizes range support for; `i64`,
/// `u64`, `i128`, `u128`, `usize`, and `isize` lose precision beyond 2^53, identical to
/// `labels_from_cell_names`'s existing `try_float_ty!`-driven display path for those types.
trait ToF64Display {
    fn to_f64_display(&self) -> f64;
}

macro_rules! impl_to_f64_display {
    ($($T:ty),*) => {
        $(impl ToF64Display for $T {
            fn to_f64_display(&self) -> f64 {
                *self as f64
            }
        })*
    };
}
impl_to_f64_display!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64);
```

- [ ] **Step 4: Thread `&Sheet` through `labels_from_cell_names` and mark numeric cells**

Replace `labels_from_cell_names`'s signature and body:

```rust
pub fn labels_from_cell_names(
    sheet: &Sheet,
    cell_names: &IndexMap<String, (CellId, TypeShape)>,
) -> Labels {
    let mut labels = Labels::new();
    for (name, (id, shape)) in cell_names {
        let id = *id;
        let type_id = match shape {
            TypeShape::Named(type_id) => *type_id,
            TypeShape::Tuple(_) => {
                labels.add_tuple_cell(id, name);
                continue;
            }
        };
        macro_rules! try_numeric_ty {
            ($T:ty) => {
                if type_id == TypeId::of::<$T>() {
                    labels.add_cell::<$T>(id, name);
                    mark_numeric::<$T>(&mut labels, sheet, id);
                    continue;
                }
            };
        }
        try_numeric_ty!(i8);
        try_numeric_ty!(i16);
        try_numeric_ty!(i32);
        try_numeric_ty!(i64);
        try_numeric_ty!(i128);
        try_numeric_ty!(isize);
        try_numeric_ty!(u8);
        try_numeric_ty!(u16);
        try_numeric_ty!(u32);
        try_numeric_ty!(u64);
        try_numeric_ty!(u128);
        try_numeric_ty!(usize);

        macro_rules! try_numeric_float_ty {
            ($T:ty) => {
                if type_id == TypeId::of::<$T>() {
                    labels.add_cell::<$T>(id, name);
                    if let Some(meta) = labels.cells.get_mut(&id) {
                        meta.display = Box::new(move |sheet| {
                            sheet
                                .read::<$T>(id)
                                .map(|v| format_rounded(*v as f64))
                                .unwrap_or_else(|_| "?".to_owned())
                        });
                    }
                    mark_numeric::<$T>(&mut labels, sheet, id);
                    continue;
                }
            };
        }
        try_numeric_float_ty!(f32);
        try_numeric_float_ty!(f64);

        macro_rules! try_ty {
            ($T:ty) => {
                if type_id == TypeId::of::<$T>() {
                    labels.add_cell::<$T>(id, name);
                    continue;
                }
            };
        }
        try_ty!(bool);
        try_ty!(String);
    }
    labels
}

/// Marks `id`'s `CellMeta` as numeric and, if `sheet.filter_kind(id)` is a range clamp,
/// populates its live-range closure.
fn mark_numeric<T: std::any::Any + Clone + ToF64Display>(
    labels: &mut Labels,
    sheet: &Sheet,
    id: CellId,
) {
    let Some(meta) = labels.cells.get_mut(&id) else {
        return;
    };
    meta.is_numeric = true;
    if matches!(sheet.filter_kind(id), Some(adam_rs::FilterKind::Range { .. })) {
        meta.range = Some(Box::new(move |sheet: &Sheet| {
            sheet
                .filter_range::<T>(id)
                .map(|(lo, hi)| (lo.to_f64_display(), hi.to_f64_display()))
                .unwrap_or((0.0, 0.0))
        }));
    }
}
```

- [ ] **Step 5: Update the call site**

In `begin/src/example_source.rs`, change:

```rust
    let labels = labels_from_cell_names(&parsed.cell_names);
```

to:

```rust
    let labels = labels_from_cell_names(&parsed.sheet, &parsed.cell_names);
```

- [ ] **Step 6: Update the four existing tests' call sites**

In `begin/src/bridge.rs`'s `mod tests`, update every existing `labels_from_cell_names(&cell_names)`
call (`labels_from_cell_names_rounds_float_display_to_two_decimals`,
`labels_from_cell_names_builds_entries_for_supported_types`,
`labels_from_cell_names_includes_tuple_typed_cells`,
`labels_from_cell_names_preserves_declaration_order`) to `labels_from_cell_names(&sheet,
&cell_names)` — each test already has a `sheet` binding in scope.

- [ ] **Step 7: Run the new and existing tests, then the full workspace suite**

Run: `cargo test -p begin labels_from_cell_names`
Expected: PASS (6 tests: 4 updated + 2 new).

Run: `cargo test --workspace`
Expected: PASS, zero warnings.

Run: `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
Run: `cargo clippy -p begin --all-targets -- -D warnings`
Expected: both clean.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add begin/src/bridge.rs begin/src/example_source.rs
git commit -m "feat(begin): CellMeta tracks numeric cells and their live filter range"
```

---

### Task 7: `begin/src/spectrum.rs` and `begin/src/inspector.rs` — number field and slider

**Files:**
- Modify: `begin/src/spectrum.rs`
- Modify: `begin/src/inspector.rs`
- Modify: `begin/examples/inequality.adm2`

**Interfaces:**
- Consumes: `CellMeta::is_numeric`/`CellMeta::range` from Task 6.
- Produces: `pub fn SpNumberfield(...)`, `pub fn SpSlider(...)` in `spectrum.rs`; `CellRow` renders
  one of checkbox / number field (+ optional slider) / text field per cell.

This task is pure Dioxus component/glue code with no new branching logic beyond direct passthrough
of already-tested `CellMeta` fields (mirroring the existing, also-untested-at-this-level
`is_bool`-driven `if`/`else` in `CellRow`) — per root `CLAUDE.md`'s testing rule, no new dedicated
unit test is needed for this task; correctness is verified by actually rendering `begin` in
Task 8.

- [ ] **Step 1: Add `SpNumberfield` and `SpSlider` to `spectrum.rs`**

In `begin/src/spectrum.rs`, add after `SpTextfield`:

```rust
/// Single-line numeric input.
///
/// Maps to `<sp-number-field>`. Fires standard DOM `input`, `focus`, and `blur` events, exactly
/// like [`SpTextfield`] — including the same custom-element caveat: Dioxus's event serializer
/// never populates `event.target.value` for a custom element, so reading the live value off the
/// DOM (not the synthetic event) is the caller's job. `value` is passed as its string
/// representation; the element renders and edits it as a number internally.
#[component]
pub fn SpNumberfield(
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
        sp-number-field {
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

/// A draggable range slider for a numeric value with live min/max bounds.
///
/// Maps to `<sp-slider>`. `min`/`max` are passed as strings, recomputed by the caller on every
/// render from the cell's current filter bounds (see `begin/src/bridge.rs`'s `CellMeta::range`),
/// so a range driven by other cells stays live. Fires a standard DOM `input` event; reading the
/// live numeric value off the DOM (not the synthetic event) is the caller's job, mirroring
/// [`SpTextfield`]/[`SpNumberfield`].
#[component]
pub fn SpSlider(
    id: String,
    value: String,
    min: String,
    max: String,
    disabled: bool,
    oninput: EventHandler<FormEvent>,
) -> Element {
    rsx! {
        sp-slider {
            "id": "{id}",
            "value": "{value}",
            "min": "{min}",
            "max": "{max}",
            "disabled": if disabled { "true" },
            oninput: move |e| oninput.call(e),
        }
    }
}
```

- [ ] **Step 2: Wire `CellRow` to render them**

In `begin/src/inspector.rs`, add `SpNumberfield`/`SpSlider` to the existing `use` and add two more
per-row memos (immediately after the existing `is_bool` memo):

```rust
use crate::spectrum::{SpCheckbox, SpDivider, SpFieldLabel, SpHeading, SpNumberfield, SpSlider, SpTextfield};
```

```rust
    let is_numeric = use_memo(move || {
        labels
            .read()
            .cells
            .get(&id)
            .map(|m| m.is_numeric)
            .unwrap_or(false)
    });

    let range = use_memo(move || {
        labels
            .read()
            .cells
            .get(&id)
            .and_then(|m| m.range.as_ref())
            .map(|f| f(&sheet.read()))
    });
```

Replace the `if *is_bool.read() { ... } else { ... }` block's `else` branch (the current
`SpTextfield`, at `inspector.rs:321-353`) with a three-way branch:

```rust
            if *is_bool.read() {
                SpCheckbox {
                    id: field_id,
                    checked: *value.read() == "true",
                    invalid: flags.read().invalid,
                    warning: flags.read().warning,
                    disabled: flags.read().disabled,
                    onclick: move |_| {
                        let next = toggled_bool_value(&value.peek());
                        write_and_propagate(sheet, labels, id, next, has_error, active_source);
                        let checked = *value.read() == "true";
                        spawn(async move {
                            let _ = document::eval(&format!(
                                r#"document.getElementById("cell-{id:?}").checked = {checked};"#
                            ))
                            .await;
                        });
                    },
                }
            } else if *is_numeric.read() {
                SpNumberfield {
                    id: field_id.clone(),
                    value: input.read().clone(),
                    invalid: flags.read().invalid,
                    warning: flags.read().warning,
                    disabled: flags.read().disabled,
                    oninput: move |_: FormEvent| {
                        spawn(async move {
                            let mut eval = document::eval(&format!(
                                r#"dioxus.send(document.getElementById("cell-{id:?}").value.toString())"#
                            ));
                            let Ok(val) = eval.recv::<String>().await else { return; };
                            if !*is_focused.read() {
                                return;
                            }
                            input.set(val.clone());
                            write_and_propagate(sheet, labels, id, &val, has_error, active_source);
                        });
                    },
                    onfocus: move |_| is_focused.set(true),
                    onblur: move |_| {
                        is_focused.set(false);
                        has_error.set(false);
                    },
                }
                if let Some((lo, hi)) = *range.read() {
                    SpSlider {
                        id: format!("cell-{id:?}-slider"),
                        value: input.read().clone(),
                        min: format!("{lo}"),
                        max: format!("{hi}"),
                        disabled: flags.read().disabled,
                        oninput: move |_: FormEvent| {
                            spawn(async move {
                                let mut eval = document::eval(&format!(
                                    r#"dioxus.send(document.getElementById("cell-{id:?}-slider").value.toString())"#
                                ));
                                let Ok(val) = eval.recv::<String>().await else { return; };
                                input.set(val.clone());
                                write_and_propagate(sheet, labels, id, &val, has_error, active_source);
                            });
                        },
                    }
                }
            } else {
                SpTextfield {
                    id: field_id,
                    value: input.read().clone(),
                    invalid: flags.read().invalid,
                    warning: flags.read().warning,
                    disabled: flags.read().disabled,
                    oninput: move |_: FormEvent| {
                        spawn(async move {
                            let mut eval = document::eval(&format!(
                                r#"dioxus.send(document.getElementById("cell-{id:?}").value)"#
                            ));
                            let Ok(val) = eval.recv::<String>().await else { return; };
                            if !*is_focused.read() {
                                return;
                            }
                            input.set(val.clone());
                            write_and_propagate(sheet, labels, id, &val, has_error, active_source);
                        });
                    },
                    onfocus: move |_| is_focused.set(true),
                    onblur: move |_| {
                        is_focused.set(false);
                        has_error.set(false);
                    },
                }
            }
```

- [ ] **Step 3: Give `inequality.adm2` a range filter to exercise the slider**

In `begin/examples/inequality.adm2`, replace the three `filter clamp(_, range.0, range.1)` clauses
with the `..=` spelling — identical clamping behavior, now recognized as `FilterKind::Range`:

```
sheet inequality {
    cell range = (0.0, 100.0);
    cell a = 0.0 filter range.0..=range.1;
    cell b = 0.0 filter range.0..=range.1;
    cell c = 2.0 filter range.0..=range.1;

    relationship {
        a := min(a, b);
        b := max(a, b);
    }
    relationship {
        b := min(b, c);
        c := max(b, c);
    }
}
```

- [ ] **Step 4: Run the full workspace suite**

Run: `cargo build --workspace`
Expected: builds cleanly, zero warnings.

Run: `cargo test --workspace`
Expected: PASS, zero warnings.

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Run: `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
Run: `cargo clippy -p begin --all-targets -- -D warnings`
Expected: all three clean.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add begin/src/spectrum.rs begin/src/inspector.rs begin/examples/inequality.adm2
git commit -m "feat(begin): render a number field and live-range slider for numeric cells"
```

---

### Task 8: UI verification and handoff

**Files:**
- Modify (if needed): `docs/superpowers/2026-08-24-filter-deduction-phase-1-handoff.md`

- [ ] **Step 1: Render `begin` and verify the new UI**

Use the `verifying-begin-ui` skill to serve `begin` and load the `inequality` example. Confirm:
- Cells `a`, `b`, `c` render an `sp-number-field` (not a plain text field).
- Each also renders an `sp-slider` beneath it, with `min`/`max` matching `range`'s current
  `(0.0, 100.0)` tuple values.
- Editing the number field writes and propagates exactly like the old text field did (clamped to
  `[0, 100]` by the relationship chain, same as before the `inequality.adm2` edit).
- Dragging the slider writes and propagates identically.
- A non-numeric example (e.g. `toy_example`) still renders plain text fields for non-numeric,
  non-bool cells, and checkboxes for bool cells, unchanged.

- [ ] **Step 2: Update the phase handoff doc**

Update `docs/superpowers/2026-08-24-filter-deduction-phase-1-handoff.md`'s "Left" section to
record that §3 and §4 are now done, replacing the two "Left" bullets with a short "Done" note
pointing at this plan's commits, so a future reader doesn't need to re-derive status from git
history (per root `CLAUDE.md`'s multi-phase handoff requirement).

- [ ] **Step 3: Commit**

```bash
cargo fmt --all
git add docs/superpowers/2026-08-24-filter-deduction-phase-1-handoff.md
git commit -m "docs: record filter-kind range-slider (§3/§4) completion in the phase 1 handoff"
```
