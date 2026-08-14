# Native conditional match-expressions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an `adam-rs` conditional's match subject be a method-like expression over
multiple input cells (not just a single existing cell), deducing the dependency cells
directly from the expression, per [issue #99](https://github.com/stlab/cel-rs/issues/99).

**Architecture:** A new `MatchExpr` type (mirroring `Method`) wraps either a plain `CellId`
(today's case, zero-alloc) or a type-erased function over multiple input cells.
`ConditionalData` stores a `MatchSource` instead of a bare `CellId`, and every place that
reads the match cell generalizes to a `match_cells()` slice — this is what lets the
upstream-contributing-cells BFS in `add_conditional` correctly seed from every input, not
just one. Evaluating a computed match value is fallible, so `build_active_set` and the
public `conditional_active_branch` accessor both become `Result`-returning.

**Tech Stack:** Rust, `adam-rs`/`adam-lang`/`begin` crates in this Cargo workspace. No new
dependencies.

**Spec:** [docs/superpowers/specs/2026-08-14-conditional-match-expression-design.md](../specs/2026-08-14-conditional-match-expression-design.md)

## Global Constraints

- `cargo fmt --all` before every commit (enforced by pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` (including `cargo test --doc
  --workspace`) must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`,
  `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`, and
  `cargo clippy -p begin --all-targets -- -D warnings` must all pass with zero warnings.
- Every public function needs a contract-style `///` doc comment (Summary /
  Preconditions / `# Errors` / Postconditions / Complexity, as applicable) — see the
  project `CLAUDE.md` for the exact convention.
- Unit tests are derived from contract and public interface only, not implementation.
- No grammar or behavior change to `adam-lang`'s `conditional <ident> { ... }` syntax in
  this plan — every fix outside `adam-rs` is mechanical (wrap the existing `CellId` in
  `MatchExpr::cell(...)`, or a rename/`Result`-unwrap follow-on).

---

## Task 1: `MatchExpr` — new match-expression type

**Files:**
- Modify: `adam-rs/src/conditional.rs`
- Modify: `adam-rs/src/lib.rs`

**Interfaces:**
- Consumes: nothing new (uses existing `CellId` from `crate::cell`).
- Produces: `pub struct MatchExpr(pub(crate) MatchSource)`, `pub(crate) enum MatchSource {
  Cell(CellId), Expr(MatchExprData) }`, `pub(crate) struct MatchExprData { inputs:
  Vec<CellId>, input_types: Vec<TypeId>, output_type: TypeId, eq_fn: fn(&dyn Any, &dyn Any)
  -> bool, function: Box<dyn Fn(&[&dyn Any]) -> Result<Box<dyn Any>, anyhow::Error>> }` and
  constructors `MatchExpr::cell`, `MatchExpr::new`, `MatchExpr::from_fn_1`,
  `MatchExpr::from_fn_2` — consumed by Task 2 onward. `adam_rs::MatchExpr` is re-exported
  from the crate root.

This task is purely additive: it does not touch `ConditionalData`/`Branch`/`Sheet`, so the
crate keeps compiling and all existing tests keep passing throughout.

- [ ] **Step 1: Update the module doc comment and write the failing tests for `MatchExpr`**

Replace the top of `adam-rs/src/conditional.rs` (the `//!` doc comment) and insert the new
types/tests. The file currently starts:

```rust
//! Conditionals in the property model: match-cell branching.
//!
//! Each conditional binds to one cell (the *match cell*) and holds a list of
//! branches. During propagation the branch whose keys contain the current match
//! cell value is activated; its relationships participate in the general planning
//! pass.

use std::any::Any;

use slotmap::new_key_type;

use crate::{cell::CellId, relationship::RelationshipId};
```

Replace it with:

```rust
//! Conditionals in the property model: match-subject branching.
//!
//! Each conditional evaluates a match subject — either a single existing cell, read
//! directly, or a [`MatchExpr`] computed from multiple input cells — and holds a list of
//! branches. During propagation the branch whose keys contain the current match value is
//! activated; its relationships participate in the general planning pass.

use std::any::{Any, TypeId};

use slotmap::new_key_type;

use crate::{cell::CellId, relationship::RelationshipId};
```

Then, directly below the `new_key_type! { ... ConditionalId ... }` block and *before* the
existing `Branch` struct, insert:

```rust
/// A conditional's match subject: an existing cell, or a method-like expression computed
/// from a set of input cells.
///
/// Constructed via [`MatchExpr::cell`] for the common single-cell case, or
/// [`MatchExpr::new`]/[`MatchExpr::from_fn_1`]/[`MatchExpr::from_fn_2`] to compute the
/// match value from multiple cells (analogous to [`crate::relationship::Method`]).
pub struct MatchExpr(pub(crate) MatchSource);

pub(crate) enum MatchSource {
    /// The match value is `cell`'s current effective value, read directly with no
    /// allocation and no extra trait bounds.
    Cell(CellId),
    /// The match value is computed from `MatchExprData`.
    Expr(MatchExprData),
}

pub(crate) struct MatchExprData {
    pub(crate) inputs: Vec<CellId>,
    pub(crate) input_types: Vec<TypeId>,
    pub(crate) output_type: TypeId,
    pub(crate) eq_fn: fn(&dyn Any, &dyn Any) -> bool,
    pub(crate) function: Box<dyn Fn(&[&dyn Any]) -> Result<Box<dyn Any>, anyhow::Error>>,
}

impl MatchExpr {
    /// Wraps a single existing cell as the match subject.
    ///
    /// - Postcondition: behaves exactly as a plain-cell conditional does today — the match
    ///   value is `cell`'s current effective value, with no extra allocation or trait
    ///   bounds beyond what [`crate::sheet::Sheet::add_conditional`] itself requires.
    #[must_use]
    pub fn cell(cell: CellId) -> Self {
        MatchExpr(MatchSource::Cell(cell))
    }

    /// Creates a match expression from explicit `TypeId`s and a type-erased function.
    ///
    /// - Precondition: `inputs.len() == input_types.len()`.
    /// - Precondition: `f` returns a value whose runtime type matches `output_type`.
    /// - Precondition: `eq_fn` correctly compares two values of the type identified by
    ///   `output_type`.
    #[must_use]
    pub fn new<F>(
        inputs: Vec<CellId>,
        input_types: Vec<TypeId>,
        output_type: TypeId,
        eq_fn: fn(&dyn Any, &dyn Any) -> bool,
        f: F,
    ) -> Self
    where
        F: Fn(&[&dyn Any]) -> Result<Box<dyn Any>, anyhow::Error> + 'static,
    {
        debug_assert_eq!(inputs.len(), input_types.len());
        MatchExpr(MatchSource::Expr(MatchExprData {
            inputs,
            input_types,
            output_type,
            eq_fn,
            function: Box::new(f),
        }))
    }

    /// Creates a 1-input match expression from a typed closure.
    ///
    /// `TypeId`s for `A` and `T` are captured automatically, along with `T`'s equality
    /// function. The expression is validated against its cell registration when passed to
    /// [`crate::sheet::Sheet::add_conditional`].
    #[must_use]
    pub fn from_fn_1<A, T, F>(input: CellId, f: F) -> Self
    where
        A: Any + 'static,
        T: Any + PartialEq + 'static,
        F: Fn(&A) -> Result<T, anyhow::Error> + 'static,
    {
        MatchExpr::new(
            vec![input],
            vec![TypeId::of::<A>()],
            TypeId::of::<T>(),
            |a, b| a.downcast_ref::<T>() == b.downcast_ref::<T>(),
            move |args| {
                let a = args[0]
                    .downcast_ref::<A>()
                    .expect("type checked at add_conditional");
                Ok(Box::new(f(a)?) as Box<dyn Any>)
            },
        )
    }

    /// Creates a 2-input match expression from a typed closure.
    ///
    /// `inputs[0]` maps to `A` and `inputs[1]` maps to `B`. `TypeId`s for `A`, `B`, and `T`
    /// are captured automatically, along with `T`'s equality function. The expression is
    /// validated when passed to [`crate::sheet::Sheet::add_conditional`].
    #[must_use]
    pub fn from_fn_2<A, B, T, F>(inputs: [CellId; 2], f: F) -> Self
    where
        A: Any + 'static,
        B: Any + 'static,
        T: Any + PartialEq + 'static,
        F: Fn(&A, &B) -> Result<T, anyhow::Error> + 'static,
    {
        MatchExpr::new(
            inputs.to_vec(),
            vec![TypeId::of::<A>(), TypeId::of::<B>()],
            TypeId::of::<T>(),
            |a, b| a.downcast_ref::<T>() == b.downcast_ref::<T>(),
            move |args| {
                let a = args[0]
                    .downcast_ref::<A>()
                    .expect("type checked at add_conditional");
                let b = args[1]
                    .downcast_ref::<B>()
                    .expect("type checked at add_conditional");
                Ok(Box::new(f(a, b)?) as Box<dyn Any>)
            },
        )
    }
}
```

Then, inside the existing `#[cfg(test)] mod tests { use super::*; ... }` block at the
bottom of the file (which currently only has `conditional_id_is_copy`), add:

```rust
    #[test]
    fn match_expr_cell_wraps_a_single_cell() {
        use slotmap::SlotMap;
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let cell = map.insert(());
        let expr = MatchExpr::cell(cell);
        match expr.0 {
            MatchSource::Cell(id) => assert_eq!(id, cell),
            MatchSource::Expr(_) => panic!("expected Cell variant"),
        }
    }

    #[test]
    fn match_expr_from_fn_1_stores_correct_type_ids_and_computes_value() {
        use slotmap::SlotMap;
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());

        let expr = MatchExpr::from_fn_1(a, |x: &i32| Ok(*x * 2));
        match expr.0 {
            MatchSource::Expr(data) => {
                assert_eq!(data.inputs, vec![a]);
                assert_eq!(data.input_types, vec![TypeId::of::<i32>()]);
                assert_eq!(data.output_type, TypeId::of::<i32>());
                let x: i32 = 5;
                let result = (data.function)(&[&x]).unwrap();
                assert_eq!(*result.downcast_ref::<i32>().unwrap(), 10);
                let y: i32 = 10;
                assert!((data.eq_fn)(&y, &10_i32));
                assert!(!(data.eq_fn)(&y, &11_i32));
            }
            MatchSource::Cell(_) => panic!("expected Expr variant"),
        }
    }

    #[test]
    fn match_expr_from_fn_2_stores_correct_type_ids_and_computes_value() {
        use slotmap::SlotMap;
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());
        let b = map.insert(());

        let expr = MatchExpr::from_fn_2([a, b], |x: &bool, y: &bool| Ok(*x && *y));
        match expr.0 {
            MatchSource::Expr(data) => {
                assert_eq!(data.inputs, vec![a, b]);
                assert_eq!(
                    data.input_types,
                    vec![TypeId::of::<bool>(), TypeId::of::<bool>()]
                );
                assert_eq!(data.output_type, TypeId::of::<bool>());
                let x = true;
                let y = false;
                let result = (data.function)(&[&x, &y]).unwrap();
                assert!(!*result.downcast_ref::<bool>().unwrap());
            }
            MatchSource::Cell(_) => panic!("expected Expr variant"),
        }
    }

    #[test]
    fn match_expr_new_reports_the_error_a_failing_function_returns() {
        use slotmap::SlotMap;
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());

        let expr = MatchExpr::new(
            vec![a],
            vec![TypeId::of::<i32>()],
            TypeId::of::<i32>(),
            |x, y| x.downcast_ref::<i32>() == y.downcast_ref::<i32>(),
            |_args| Err(anyhow::anyhow!("boom")),
        );
        let MatchSource::Expr(data) = expr.0 else {
            panic!("expected Expr variant")
        };
        let x: i32 = 1;
        let err = (data.function)(&[&x]).unwrap_err();
        assert_eq!(err.to_string(), "boom");
    }
```

- [ ] **Step 2: Run the new tests to verify they fail to compile (types don't exist yet)**

Run: `cargo test -p adam-rs conditional:: --lib`
Expected: compile error, `MatchExpr`/`MatchSource`/`MatchExprData` not found (since Step 1's
test-writing and type-writing happen together here, this just confirms the test file was
saved — if you split writing the types from writing the tests, run this after only the
tests are in place and confirm the failure, then add the types and re-run).

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cargo test -p adam-rs conditional:: --lib`
Expected: PASS (5 new tests, plus the existing `conditional_id_is_copy`).

- [ ] **Step 4: Export `MatchExpr` from the crate root**

In `adam-rs/src/lib.rs`, change:

```rust
pub use conditional::ConditionalId;
```

to:

```rust
pub use conditional::{ConditionalId, MatchExpr};
```

- [ ] **Step 5: Run the full `adam-rs` test suite to confirm nothing else broke**

Run: `cargo test -p adam-rs`
Expected: PASS (all existing tests still pass — this task didn't touch `ConditionalData`,
`Sheet`, or any existing call site).

- [ ] **Step 6: Commit**

```bash
git add adam-rs/src/conditional.rs adam-rs/src/lib.rs
git commit -m "feat(adam-rs): add MatchExpr, a method-like conditional match subject"
```

---

## Task 2: Rewire `ConditionalData`/`Sheet` to use `MatchSource`, fix all in-crate call sites

**Files:**
- Modify: `adam-rs/src/conditional.rs`
- Modify: `adam-rs/src/sheet.rs`
- Modify: `adam-rs/tests/integration.rs`

**Interfaces:**
- Consumes: `MatchExpr`/`MatchSource`/`MatchExprData` from Task 1.
- Produces: `ConditionalData::match_cells(&self) -> &[CellId]`;
  `Sheet::add_conditional<T>(&mut self, source: MatchExpr, branches: Vec<(Vec<T>,
  Vec<RelationshipId>)>, default: Vec<RelationshipId>) -> Result<ConditionalId, Error>`;
  `Sheet::conditional_match_cells(&self, id: ConditionalId) -> Option<&[CellId]>` (replaces
  `conditional_match_cell`); `Sheet::conditional_active_branch(&self, id: ConditionalId) ->
  Result<Option<usize>, Error>` (was `Option<usize>`) — all consumed by Task 3 (`begin`) and
  Task 4 (`adam-lang`).

This is one atomic refactor: `ConditionalData`'s field shape and every internal consumer of
it change together, so the task isn't meaningfully splittable — but it's still driven
test-first, one behavior at a time.

- [ ] **Step 1: Change `ConditionalData`/`Branch` to hold a `MatchSource`, add `match_cells()`**

In `adam-rs/src/conditional.rs`, replace:

```rust
/// One arm of a [`ConditionalData`]: a set of key values and the relationships
/// to activate when the match cell equals any key.
pub(crate) struct Branch {
    /// Type-erased key values; each `TypeId` matches the match cell's registered type.
    pub(crate) keys: Vec<Box<dyn Any>>,
    /// Relationships activated when any key matches.
    pub(crate) relationships: Vec<RelationshipId>,
}

/// Internal storage for a conditional.
pub(crate) struct ConditionalData {
    /// The cell whose value is tested.
    pub(crate) cell: CellId,
    /// Branches evaluated in definition order; first match wins.
    pub(crate) branches: Vec<Branch>,
    /// Relationships activated when no branch matches. Empty means no default.
    pub(crate) default: Vec<RelationshipId>,
}
```

with:

```rust
/// One arm of a [`ConditionalData`]: a set of key values and the relationships
/// to activate when the match value equals any key.
pub(crate) struct Branch {
    /// Type-erased key values; each `TypeId` matches the match subject's output type.
    pub(crate) keys: Vec<Box<dyn Any>>,
    /// Relationships activated when any key matches.
    pub(crate) relationships: Vec<RelationshipId>,
}

/// Internal storage for a conditional.
pub(crate) struct ConditionalData {
    /// The match subject whose value is tested.
    pub(crate) source: MatchSource,
    /// Branches evaluated in definition order; first match wins.
    pub(crate) branches: Vec<Branch>,
    /// Relationships activated when no branch matches. Empty means no default.
    pub(crate) default: Vec<RelationshipId>,
}

impl ConditionalData {
    /// Returns the cells that determine this conditional's match value: a single cell for
    /// [`MatchSource::Cell`], or every input of the expression for [`MatchSource::Expr`].
    pub(crate) fn match_cells(&self) -> &[CellId] {
        match &self.source {
            MatchSource::Cell(id) => std::slice::from_ref(id),
            MatchSource::Expr(expr) => &expr.inputs,
        }
    }
}
```

Add two tests to the `mod tests` block (this won't compile yet — that's expected, fixed by
the rest of this step's changes):

```rust
    #[test]
    fn match_cells_returns_single_cell_for_cell_variant() {
        use slotmap::SlotMap;
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let cell = map.insert(());
        let data = ConditionalData {
            source: MatchSource::Cell(cell),
            branches: Vec::new(),
            default: Vec::new(),
        };
        assert_eq!(data.match_cells(), &[cell]);
    }

    #[test]
    fn match_cells_returns_all_inputs_for_expr_variant() {
        use slotmap::SlotMap;
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());
        let b = map.insert(());
        let expr = MatchExpr::from_fn_2([a, b], |x: &i32, y: &i32| Ok(x + y));
        let MatchSource::Expr(data) = expr.0 else {
            panic!("expected Expr variant")
        };
        let cond = ConditionalData {
            source: MatchSource::Expr(data),
            branches: Vec::new(),
            default: Vec::new(),
        };
        assert_eq!(cond.match_cells(), &[a, b]);
    }
```

- [ ] **Step 2: Update `sheet.rs`'s imports**

In `adam-rs/src/sheet.rs`, change:

```rust
use crate::{
    cell::{CellData, CellId},
    condition::{Condition, ConditionData, ConditionId},
    conditional::{Branch, ConditionalData, ConditionalId},
    error::Error,
    output::{OutputData, OutputId},
    relationship::{Method, RelationshipData, RelationshipId},
};
```

to:

```rust
use crate::{
    cell::{CellData, CellId},
    condition::{Condition, ConditionData, ConditionId},
    conditional::{Branch, ConditionalData, ConditionalId, MatchExpr, MatchSource},
    error::Error,
    output::{OutputData, OutputId},
    relationship::{Method, RelationshipData, RelationshipId},
};
```

- [ ] **Step 3: Rewrite `Sheet::add_conditional`**

Replace the entire `add_conditional` function body (`adam-rs/src/sheet.rs`, currently lines
239–366, from the doc comment through the closing `}`) with:

```rust
    /// Registers a conditional that activates relationships based on the value of a match
    /// subject: either a single existing cell, or a [`MatchExpr`] computed from multiple
    /// input cells.
    ///
    /// Each element of `branches` is `(keys, relationships)`: when the match subject's
    /// value equals any key in `keys`, the branch's `relationships` are added to the active
    /// set for `propagate`. Branches are evaluated in definition order; first match wins.
    /// `default` holds relationships activated when no branch matches; pass an empty `Vec`
    /// for no default.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — the match subject references a cell not in this sheet.
    /// - `Error::TerminalCell` — the match subject references a cell that already belongs
    ///   to an existing output.
    /// - `Error::TypeMismatch` — (expression match subject only) an input cell's registered
    ///   type doesn't match the expression's declared type for that input.
    /// - `Error::InvalidConditional` — the match subject's output type does not match `T`;
    ///   a branch relationship shares a cell with the match subject or any of its
    ///   unconditional upstream contributors and has more than one method; a referenced
    ///   relationship does not exist; a relationship already appears in another
    ///   conditional branch; or a branch has no keys.
    ///
    /// - Complexity: O(B·(K + R)) where B = branches, K = keys per branch, R =
    ///   relationships per branch.
    pub fn add_conditional<T: Any + PartialEq + 'static>(
        &mut self,
        source: MatchExpr,
        branches: Vec<(Vec<T>, Vec<RelationshipId>)>,
        default: Vec<RelationshipId>,
    ) -> Result<ConditionalId, Error> {
        let match_cells: Vec<CellId> = match &source.0 {
            MatchSource::Cell(cell) => {
                let cell_data = self.cells.get(*cell).ok_or(Error::InvalidId)?;
                if self.terminal_cells.contains(cell) {
                    return Err(Error::TerminalCell);
                }
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
                    if self.terminal_cells.contains(&cell_id) {
                        return Err(Error::TerminalCell);
                    }
                    let cell_data = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                    if cell_data.type_id != declared {
                        return Err(Error::TypeMismatch {
                            expected: cell_data.type_id,
                            found: declared,
                        });
                    }
                }
                expr.inputs.clone()
            }
        };

        // Collect and validate all relationship IDs (branches + default).
        let all_rels: Vec<RelationshipId> = branches
            .iter()
            .flat_map(|(_, rels)| rels.iter().copied())
            .chain(default.iter().copied())
            .collect();
        let all_rels_set: HashSet<RelationshipId> = all_rels.iter().copied().collect();

        // Compute the set of cells that contribute to the match subject: BFS upstream
        // through unconditional relationships (excluding already-committed conditional
        // relationships and the relationships currently being added), seeded from *every*
        // match cell. A branch relationship with multiple methods is invalid if any of its
        // adjacent cells is in this contributing set, because the branch could then flip
        // method selection in the match subject's upstream subgraph.
        let contributing_cells: HashSet<CellId> = {
            let mut cells: HashSet<CellId> = HashSet::new();
            let mut queue: std::collections::VecDeque<CellId> = std::collections::VecDeque::new();
            for &cell in &match_cells {
                if cells.insert(cell) {
                    queue.push_back(cell);
                }
            }
            while let Some(c) = queue.pop_front() {
                if let Some(cell_data) = self.cells.get(c) {
                    for &rel_id in &cell_data.adj {
                        if self.conditional_relationships.contains(&rel_id)
                            || all_rels_set.contains(&rel_id)
                        {
                            continue;
                        }
                        let rel = &self.relationships[rel_id];
                        if !rel.methods.iter().any(|m| m.outputs.contains(&c)) {
                            continue;
                        }
                        for &adj_cell in &rel.adj {
                            if cells.insert(adj_cell) {
                                queue.push_back(adj_cell);
                            }
                        }
                    }
                }
            }
            cells
        };

        for &rel_id in &all_rels {
            let rel = self
                .relationships
                .get(rel_id)
                .ok_or(Error::InvalidConditional)?;
            if rel.adj.iter().any(|c| contributing_cells.contains(c)) && rel.methods.len() != 1 {
                return Err(Error::InvalidConditional);
            }
            if self.conditional_relationships.contains(&rel_id) {
                return Err(Error::InvalidConditional);
            }
        }

        // Validate branch keys are non-empty.
        for (keys, _) in &branches {
            if keys.is_empty() {
                return Err(Error::InvalidConditional);
            }
        }

        // Check for duplicate relationship IDs within this call.
        let mut seen: HashSet<RelationshipId> = HashSet::new();
        for &rel_id in &all_rels {
            if !seen.insert(rel_id) {
                return Err(Error::InvalidConditional);
            }
        }

        // Type-erase branch keys.
        let typed_branches: Vec<Branch> = branches
            .into_iter()
            .map(|(keys, relationships)| Branch {
                keys: keys
                    .into_iter()
                    .map(|k| Box::new(k) as Box<dyn Any>)
                    .collect(),
                relationships,
            })
            .collect();

        // Record all relationships as conditional so they are excluded from the
        // unconditional active set in propagate().
        for &rel_id in &all_rels {
            self.conditional_relationships.insert(rel_id);
        }

        Ok(self.conditionals.insert(ConditionalData {
            source: source.0,
            branches: typed_branches,
            default,
        }))
    }
```

- [ ] **Step 4: Fix `cell_has_prior_use` and `conditionals_potentially_producing`**

In `adam-rs/src/sheet.rs`, change:

```rust
    fn cell_has_prior_use(&self, id: CellId) -> bool {
        self.cells.get(id).is_some_and(|cell| !cell.adj.is_empty())
            || self.conditionals.values().any(|c| c.cell == id)
    }
```

to:

```rust
    fn cell_has_prior_use(&self, id: CellId) -> bool {
        self.cells.get(id).is_some_and(|cell| !cell.adj.is_empty())
            || self
                .conditionals
                .values()
                .any(|c| c.match_cells().contains(&id))
    }
```

And change (same file, `conditionals_potentially_producing`):

```rust
    /// Returns the match cell of every conditional with at least one branch (or default)
    /// relationship that touches `cell` (as an input or output of any of its methods) —
    /// every conditional whose branch choice currently determines, or could determine,
    /// `cell`'s value or whether it has an active producer at all.
    ///
    /// - Complexity: O(B) where B is the total number of branch/default relationships
    ///   across all conditionals.
    fn conditionals_potentially_producing(&self, cell: CellId) -> Vec<CellId> {
        self.conditionals
            .values()
            .filter(|cond| {
                cond.branches
                    .iter()
                    .flat_map(|branch| branch.relationships.iter())
                    .chain(cond.default.iter())
                    .any(|&rel_id| self.relationships[rel_id].adj.contains(&cell))
            })
            .map(|cond| cond.cell)
            .collect()
    }
```

to:

```rust
    /// Returns the match cells of every conditional with at least one branch (or default)
    /// relationship that touches `cell` (as an input or output of any of its methods) —
    /// every conditional whose branch choice currently determines, or could determine,
    /// `cell`'s value or whether it has an active producer at all.
    ///
    /// - Complexity: O(B) where B is the total number of branch/default relationships
    ///   across all conditionals.
    fn conditionals_potentially_producing(&self, cell: CellId) -> Vec<CellId> {
        self.conditionals
            .values()
            .filter(|cond| {
                cond.branches
                    .iter()
                    .flat_map(|branch| branch.relationships.iter())
                    .chain(cond.default.iter())
                    .any(|&rel_id| self.relationships[rel_id].adj.contains(&cell))
            })
            .flat_map(|cond| cond.match_cells().iter().copied())
            .collect()
    }
```

- [ ] **Step 5: Add the `MatchValue` evaluation helper**

In `adam-rs/src/sheet.rs`, directly above the `build_active_set` method, insert:

```rust
/// A conditional's evaluated match value: borrowed (existing cell, no allocation) or owned
/// (freshly computed by a [`MatchExpr`] function).
enum MatchValue<'a> {
    Ref(&'a dyn Any),
    Owned(Box<dyn Any>),
}

impl MatchValue<'_> {
    fn as_dyn(&self) -> &dyn Any {
        match self {
            MatchValue::Ref(r) => *r,
            MatchValue::Owned(b) => b.as_ref(),
        }
    }
}
```

Then, as methods on `impl Sheet` (add them directly above `build_active_set` too, inside
the `impl Sheet { ... }` block):

```rust
    /// Evaluates conditional `cond`'s current match value: borrows the cell directly for a
    /// plain match subject (no allocation), or calls the expression's function once for a
    /// computed match subject.
    ///
    /// # Errors
    ///
    /// - `Error::MethodFailed` — the match subject is a [`MatchExpr`] whose function
    ///   returned an error.
    fn evaluate_match_source(&self, cond: &ConditionalData) -> Result<MatchValue<'_>, Error> {
        match &cond.source {
            MatchSource::Cell(id) => Ok(MatchValue::Ref(self.cells[*id].effective())),
            MatchSource::Expr(expr) => {
                let args: Vec<&dyn Any> = expr
                    .inputs
                    .iter()
                    .map(|&id| self.cells[id].effective())
                    .collect();
                let value = (expr.function)(&args).map_err(Error::MethodFailed)?;
                Ok(MatchValue::Owned(value))
            }
        }
    }

    /// Returns the equality function used to compare `cond`'s match value against branch
    /// keys: the match cell's own `eq_fn` for a plain match subject, or the expression's
    /// captured `eq_fn` for a computed one.
    fn match_eq_fn(&self, cond: &ConditionalData) -> fn(&dyn Any, &dyn Any) -> bool {
        match &cond.source {
            MatchSource::Cell(id) => self.cells[*id].eq_fn,
            MatchSource::Expr(expr) => expr.eq_fn,
        }
    }
```

- [ ] **Step 6: Make `build_active_set` fallible and use the new helpers**

Change:

```rust
    /// Builds the active relationship set for the general planning pass.
    ///
    /// Starts with all unconditional relationships (those not in
    /// `self.conditional_relationships`), then evaluates each conditional: the first
    /// branch whose keys contain the match cell's current value is selected, and its
    /// relationships are added. If no branch matches, the default relationships are added.
    ///
    /// - Complexity: O(R + C·B·K) where R = total relationships, C = conditionals,
    ///   B = branches per conditional, K = keys per branch.
    fn build_active_set(&self) -> HashSet<RelationshipId> {
        let mut active: HashSet<RelationshipId> = self
            .relationships
            .keys()
            .filter(|id| !self.conditional_relationships.contains(id))
            .collect();

        for (_, cond) in &self.conditionals {
            let cell = &self.cells[cond.cell];
            let eq_fn = cell.eq_fn;
            let value = cell.effective();

            let mut matched = false;
            for branch in &cond.branches {
                if branch.keys.iter().any(|key| eq_fn(value, key.as_ref())) {
                    for &rel_id in &branch.relationships {
                        active.insert(rel_id);
                    }
                    matched = true;
                    break;
                }
            }
            if !matched {
                for &rel_id in &cond.default {
                    active.insert(rel_id);
                }
            }
        }

        active
    }
```

to:

```rust
    /// Builds the active relationship set for the general planning pass.
    ///
    /// Starts with all unconditional relationships (those not in
    /// `self.conditional_relationships`), then evaluates each conditional: the first
    /// branch whose keys contain the match subject's current value is selected, and its
    /// relationships are added. If no branch matches, the default relationships are added.
    ///
    /// # Errors
    ///
    /// - `Error::MethodFailed` — an expression-sourced conditional's function returned an
    ///   error.
    ///
    /// - Complexity: O(R + C·B·K) where R = total relationships, C = conditionals,
    ///   B = branches per conditional, K = keys per branch.
    fn build_active_set(&self) -> Result<HashSet<RelationshipId>, Error> {
        let mut active: HashSet<RelationshipId> = self
            .relationships
            .keys()
            .filter(|id| !self.conditional_relationships.contains(id))
            .collect();

        for (_, cond) in &self.conditionals {
            let value = self.evaluate_match_source(cond)?;
            let value_ref = value.as_dyn();
            let eq_fn = self.match_eq_fn(cond);

            let mut matched = false;
            for branch in &cond.branches {
                if branch.keys.iter().any(|key| eq_fn(value_ref, key.as_ref())) {
                    for &rel_id in &branch.relationships {
                        active.insert(rel_id);
                    }
                    matched = true;
                    break;
                }
            }
            if !matched {
                for &rel_id in &cond.default {
                    active.insert(rel_id);
                }
            }
        }

        Ok(active)
    }
```

- [ ] **Step 7: Update `propagate()`'s two call sites**

Change (phase 1 pre-plan):

```rust
        // Phase 1: pre-plan for derived match cells.
        if !self.conditionals.is_empty() {
            let match_cells: Vec<CellId> = self.conditionals.values().map(|c| c.cell).collect();
            let pre_active = self.match_cell_subgraph(&match_cells);
```

to:

```rust
        // Phase 1: pre-plan for derived match cells.
        if !self.conditionals.is_empty() {
            let match_cells: Vec<CellId> = self
                .conditionals
                .values()
                .flat_map(|c| c.match_cells().iter().copied())
                .collect();
            let pre_active = self.match_cell_subgraph(&match_cells);
```

Change (phase 2, a few lines further down):

```rust
        // Phase 2: evaluate conditionals and build the active relationship set.
        let active = self.build_active_set();
```

to:

```rust
        // Phase 2: evaluate conditionals and build the active relationship set.
        let active = self.build_active_set()?;
```

- [ ] **Step 8: Rename `conditional_match_cell` to `conditional_match_cells`**

Change:

```rust
    /// Returns the match cell for conditional `id`.
    ///
    /// Returns `None` if `id` is not a live conditional in this sheet.
    pub fn conditional_match_cell(&self, id: ConditionalId) -> Option<CellId> {
        self.conditionals.get(id).map(|c| c.cell)
    }
```

to:

```rust
    /// Returns the match cells for conditional `id`: a single cell for a plain match
    /// subject, or every input of a [`MatchExpr`] match subject.
    ///
    /// Returns `None` if `id` is not a live conditional in this sheet.
    pub fn conditional_match_cells(&self, id: ConditionalId) -> Option<&[CellId]> {
        self.conditionals.get(id).map(|c| c.match_cells())
    }
```

- [ ] **Step 9: Make `conditional_active_branch` fallible**

Change:

```rust
    /// Returns the index of the currently matching branch for conditional `id`.
    ///
    /// Evaluates branch keys against the match cell's current value in definition order;
    /// returns the index of the first matching branch. Returns `None` if no branch key
    /// matches (the default branch is active) or if `id` is not a live conditional.
    ///
    /// - Complexity: O(B·K) where B = branches, K = keys per branch.
    pub fn conditional_active_branch(&self, id: ConditionalId) -> Option<usize> {
        let cond = self.conditionals.get(id)?;
        let cell = &self.cells[cond.cell];
        let eq_fn = cell.eq_fn;
        let value = cell.effective();
        cond.branches
            .iter()
            .enumerate()
            .find(|(_, branch)| branch.keys.iter().any(|key| eq_fn(value, key.as_ref())))
            .map(|(i, _)| i)
    }
```

to:

```rust
    /// Returns the index of the currently matching branch for conditional `id`.
    ///
    /// Evaluates branch keys against the match subject's current value in definition
    /// order; returns the index of the first matching branch. Returns `Ok(None)` if no
    /// branch key matches (the default branch is active) or if `id` is not a live
    /// conditional.
    ///
    /// # Errors
    ///
    /// - `Error::MethodFailed` — `id` is a live, expression-sourced conditional whose
    ///   function returned an error.
    ///
    /// - Complexity: O(B·K) where B = branches, K = keys per branch.
    pub fn conditional_active_branch(&self, id: ConditionalId) -> Result<Option<usize>, Error> {
        let Some(cond) = self.conditionals.get(id) else {
            return Ok(None);
        };
        let value = self.evaluate_match_source(cond)?;
        let value_ref = value.as_dyn();
        let eq_fn = self.match_eq_fn(cond);
        Ok(cond
            .branches
            .iter()
            .enumerate()
            .find(|(_, branch)| branch.keys.iter().any(|key| eq_fn(value_ref, key.as_ref())))
            .map(|(i, _)| i))
    }
```

- [ ] **Step 10: Add `MatchExpr` to the `sheet.rs` test module's imports**

Change:

```rust
    use crate::{ConditionalId, Error, Method, Sheet, cell::CellId, relationship::RelationshipId};
```

to:

```rust
    use crate::{
        ConditionalId, Error, MatchExpr, Method, Sheet, cell::CellId, relationship::RelationshipId,
    };
```

- [ ] **Step 11: Mechanically wrap every plain-cell `add_conditional` call site**

Every remaining call to `add_conditional`/`add_conditional::<T>` in
`adam-rs/src/sheet.rs` and `adam-rs/tests/integration.rs` passes a bare `CellId` (a local
variable or `CellId::default()`) as the first argument — these need wrapping in
`MatchExpr::cell(...)`. Run this from the repository root:

```powershell
$files = @(
  'adam-rs/src/sheet.rs',
  'adam-rs/tests/integration.rs'
)
$pattern = 'add_conditional(::<[^(]+>)?\(\s*(CellId::default\(\)|[A-Za-z_][A-Za-z0-9_]*)\s*,'
foreach ($f in $files) {
  $full = Resolve-Path $f
  $content = Get-Content $full -Raw
  $updated = $content -replace $pattern, 'add_conditional$1(MatchExpr::cell($2),'
  [System.IO.File]::WriteAllText($full, $updated)
}
```

This turns, e.g., `.add_conditional(mode, vec![...` into
`.add_conditional(MatchExpr::cell(mode), vec![...` and
`.add_conditional::<i32>(a, vec![...` into
`.add_conditional::<i32>(MatchExpr::cell(a), vec![...`, across every call site in both
files (roughly 14 in `sheet.rs`, 15 in `integration.rs`).

After running it, inspect the diff (`git diff adam-rs/src/sheet.rs
adam-rs/tests/integration.rs`) and confirm every changed line looks like one of the two
patterns above, with no unintended matches.

- [ ] **Step 12: Fix the four tests that call the renamed/refallibilized accessors directly**

The regex in Step 11 only fixes `add_conditional` call sites; these four tests in
`adam-rs/src/sheet.rs` call `conditional_match_cell`/`conditional_active_branch` directly
and need manual updates.

Change:

```rust
    #[test]
    fn conditional_match_cell_returns_correct_cell() {
        let mut sheet = Sheet::new();
        let p = sheet.add_cell(0_i32);
        let cid = sheet.add_conditional::<i32>(p, vec![], vec![]).unwrap();
        assert_eq!(sheet.conditional_match_cell(cid), Some(p));
    }

    #[test]
    fn conditional_match_cell_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert_eq!(sheet.conditional_match_cell(ConditionalId::default()), None);
    }
```

to:

```rust
    #[test]
    fn conditional_match_cells_returns_correct_cell() {
        let mut sheet = Sheet::new();
        let p = sheet.add_cell(0_i32);
        let cid = sheet
            .add_conditional::<i32>(MatchExpr::cell(p), vec![], vec![])
            .unwrap();
        assert_eq!(sheet.conditional_match_cells(cid), Some([p].as_slice()));
    }

    #[test]
    fn conditional_match_cells_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert_eq!(
            sheet.conditional_match_cells(ConditionalId::default()),
            None
        );
    }
```

(Step 11's regex will already have rewritten the `add_conditional::<i32>(p, ...)` inside
`conditional_match_cells_returns_correct_cell` to use `MatchExpr::cell(p)` if it ran first —
the snippet above already reflects the post-Step-11 state either way, so it's safe to apply
regardless of ordering.)

Change:

```rust
    #[test]
    fn conditional_active_branch_returns_matching_branch_index() {
        let (mut sheet, cid) = sheet_with_two_branch_conditional();
        let p = sheet.conditional_match_cell(cid).unwrap();
        sheet.write(p, 0_i32).unwrap();
        assert_eq!(sheet.conditional_active_branch(cid), Some(0));
        sheet.write(p, 1_i32).unwrap();
        assert_eq!(sheet.conditional_active_branch(cid), Some(1));
    }

    #[test]
    fn conditional_active_branch_returns_none_when_no_branch_matches() {
        let (mut sheet, cid) = sheet_with_two_branch_conditional();
        let p = sheet.conditional_match_cell(cid).unwrap();
        sheet.write(p, 99_i32).unwrap();
        assert_eq!(sheet.conditional_active_branch(cid), None);
    }

    #[test]
    fn conditional_active_branch_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert_eq!(
            sheet.conditional_active_branch(ConditionalId::default()),
            None
        );
    }
```

to:

```rust
    #[test]
    fn conditional_active_branch_returns_matching_branch_index() {
        let (mut sheet, cid) = sheet_with_two_branch_conditional();
        let p = sheet.conditional_match_cells(cid).unwrap()[0];
        sheet.write(p, 0_i32).unwrap();
        assert_eq!(sheet.conditional_active_branch(cid).unwrap(), Some(0));
        sheet.write(p, 1_i32).unwrap();
        assert_eq!(sheet.conditional_active_branch(cid).unwrap(), Some(1));
    }

    #[test]
    fn conditional_active_branch_returns_none_when_no_branch_matches() {
        let (mut sheet, cid) = sheet_with_two_branch_conditional();
        let p = sheet.conditional_match_cells(cid).unwrap()[0];
        sheet.write(p, 99_i32).unwrap();
        assert_eq!(sheet.conditional_active_branch(cid).unwrap(), None);
    }

    #[test]
    fn conditional_active_branch_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert_eq!(
            sheet
                .conditional_active_branch(ConditionalId::default())
                .unwrap(),
            None
        );
    }
```

- [ ] **Step 13: Add two new tests generalizing the upstream-contributing-cells guard and
  the expression-error path**

Add to `adam-rs/src/sheet.rs`'s `mod tests`, near
`add_conditional_returns_error_when_branch_rel_involves_cell_upstream_of_match_cell`:

```rust
    #[test]
    fn add_conditional_returns_error_when_branch_rel_involves_cell_upstream_of_either_expr_input()
     {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let p = sheet.add_cell(0_i32);
        let q = sheet.add_cell(0_i32);
        // Unconditional: a → q  (a contributes to expr input q).
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, q, |x: &i32| Ok(*x))])
            .unwrap();
        // Branch relationship has two methods and involves `a`, which feeds q, one of the
        // match expression's two inputs (p, q).
        let rel = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
                Method::from_fn_1_1(b, a, |x: &i32| Ok(*x)),
            ])
            .unwrap();
        let expr = MatchExpr::from_fn_2([p, q], |x: &i32, y: &i32| Ok(*x + *y));
        let result = sheet.add_conditional(expr, vec![(vec![0_i32], vec![rel])], vec![]);
        assert!(matches!(result, Err(Error::InvalidConditional)));
    }

    #[test]
    fn add_conditional_activates_branch_from_two_cell_expression() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(false);
        let b = sheet.add_cell(false);
        let x = sheet.add_cell(0_i32);
        let y = sheet.add_cell(0_i32);
        let rel_true = sheet
            .add_relationship(vec![Method::from_fn_1_1(x, y, |v: &i32| Ok(*v))])
            .unwrap();
        let expr = MatchExpr::from_fn_2([a, b], |p: &bool, q: &bool| Ok(*p && *q));
        let cid = sheet
            .add_conditional(expr, vec![(vec![true], vec![rel_true])], vec![])
            .unwrap();

        sheet.write(a, true).unwrap();
        sheet.write(b, false).unwrap();
        assert_eq!(sheet.conditional_active_branch(cid).unwrap(), None);

        sheet.write(b, true).unwrap();
        assert_eq!(sheet.conditional_active_branch(cid).unwrap(), Some(0));
        assert_eq!(sheet.conditional_match_cells(cid).unwrap(), &[a, b]);
    }

    #[test]
    fn add_conditional_returns_invalid_conditional_for_expr_output_type_mismatch() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        // Expression computes an i32, but branch keys below are f64.
        let expr = MatchExpr::from_fn_2([a, b], |x: &i32, y: &i32| Ok(x + y));
        let result = sheet.add_conditional::<f64>(expr, vec![(vec![0.0], vec![])], vec![]);
        assert!(matches!(result, Err(Error::InvalidConditional)));
    }

    #[test]
    fn add_conditional_returns_invalid_id_for_bad_expr_input_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let expr = MatchExpr::from_fn_2(
            [a, CellId::default()],
            |x: &i32, y: &i32| Ok(x + y),
        );
        let result = sheet.add_conditional::<i32>(expr, vec![], vec![]);
        assert!(matches!(result, Err(Error::InvalidId)));
    }

    #[test]
    fn propagate_surfaces_method_failed_from_a_failing_match_expression() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let expr = MatchExpr::from_fn_1(a, |_x: &i32| -> Result<i32, anyhow::Error> {
            Err(anyhow::anyhow!("boom"))
        });
        sheet
            .add_conditional::<i32>(expr, vec![(vec![0], vec![])], vec![])
            .unwrap();
        let result = sheet.propagate();
        assert!(matches!(result, Err(Error::MethodFailed(_))));
    }
```

- [ ] **Step 14: Run the full `adam-rs` test suite**

Run: `cargo test -p adam-rs`
Expected: PASS — every existing test (mechanically updated) plus the 7 new tests added in
Steps 1, 12/13.

- [ ] **Step 15: Format and lint**

Run: `cargo fmt --all` then
`cargo clippy --workspace --exclude begin --all-targets -- -D warnings` (this will still
fail on `adam-lang`/`begin` until Tasks 3–4 — for this step, just confirm there are no new
`-p adam-rs`-scoped warnings by running `cargo clippy -p adam-rs --all-targets -- -D
warnings` instead).

- [ ] **Step 16: Commit**

```bash
git add adam-rs/src/conditional.rs adam-rs/src/sheet.rs adam-rs/tests/integration.rs
git commit -m "feat(adam-rs): deduce conditional match cells from a MatchExpr"
```

---

## Task 3: Fix `begin` crate call sites

**Files:**
- Modify: `begin/src/bridge.rs`
- Modify: `begin/src/inspector.rs`

**Interfaces:**
- Consumes: `MatchExpr`, `Sheet::conditional_match_cells`,
  `Sheet::conditional_active_branch` (now `Result`-returning) from Task 2.
- Produces: nothing new — mechanical follow-on only.

- [ ] **Step 1: Add `MatchExpr` to `bridge.rs`'s test-module import**

Change (in `begin/src/bridge.rs`'s `mod tests`):

```rust
    use adam_rs::{Method, Sheet};
```

to:

```rust
    use adam_rs::{MatchExpr, Method, Sheet};
```

- [ ] **Step 2: Wrap `bridge.rs`'s test-only `add_conditional` call sites**

Run from the repository root:

```powershell
$f = Resolve-Path 'begin/src/bridge.rs'
$pattern = 'add_conditional(::<[^(]+>)?\(\s*(CellId::default\(\)|[A-Za-z_][A-Za-z0-9_]*)\s*,'
$content = Get-Content $f -Raw
$updated = $content -replace $pattern, 'add_conditional$1(MatchExpr::cell($2),'
[System.IO.File]::WriteAllText($f, $updated)
```

Inspect `git diff begin/src/bridge.rs` and confirm the four call sites in
`sheet_with_conditional`, `sheet_with_forced_conditional`,
`sheet_with_multi_relationship_branch`, and `sheet_with_multi_relationship_default` now
wrap their `p` argument in `MatchExpr::cell(p)`.

- [ ] **Step 3: Fix `to_graph_data`'s two call sites in `bridge.rs`**

Change:

```rust
        // Constraint link: match cell → conditional node
        if let Some(match_cell) = sheet.conditional_match_cell(cond_id) {
            links.push(LinkData {
                source: cell_node_id(match_cell),
                target: node_id.clone(),
                kind: LinkKind::Constraint,
                branch_index: None,
                branch_active: None,
            });
        }

        let active_branch = sheet.conditional_active_branch(cond_id);
```

to:

```rust
        // Constraint links: every match cell → conditional node
        if let Some(match_cells) = sheet.conditional_match_cells(cond_id) {
            for &match_cell in match_cells {
                links.push(LinkData {
                    source: cell_node_id(match_cell),
                    target: node_id.clone(),
                    kind: LinkKind::Constraint,
                    branch_index: None,
                    branch_active: None,
                });
            }
        }

        // `to_graph_data` is read-only display code, not the `propagate()` path: by the
        // time it runs, `propagate()` has already evaluated this same expression
        // successfully, so a fresh failure here would itself be a precondition violation.
        // Treat it as "no active branch" for rendering rather than threading Result through
        // graph construction.
        let active_branch = sheet.conditional_active_branch(cond_id).ok().flatten();
```

- [ ] **Step 4: Run `begin`'s tests to confirm the graph-rendering fix**

Run: `cargo test -p begin`
Expected: PASS. (This will only fully pass once Task 4 also compiles, since `begin`
depends on `adam-lang`; if `adam-lang` doesn't build yet, `cargo test -p begin` will fail to
compile with errors from `adam-lang`, not from `begin` — confirm any failures at this point
are `adam-lang`-only, then continue to Task 4 and re-run this command there.)

- [ ] **Step 5: Fix `inspector.rs`'s two call sites**

Change:

```rust
    let relevant = sheet
        .output_relevant_cells()
        .into_iter()
        .chain(
            sheet
                .conditionals()
                .filter_map(|id| sheet.conditional_match_cell(id)),
        )
        .collect();
```

to:

```rust
    let relevant = sheet
        .output_relevant_cells()
        .into_iter()
        .chain(
            sheet
                .conditionals()
                .filter_map(|id| sheet.conditional_match_cells(id))
                .flatten()
                .copied(),
        )
        .collect();
```

Change:

```rust
    let is_match_cell = sheet
        .conditionals()
        .any(|cid| sheet.conditional_match_cell(cid) == Some(id));
```

to:

```rust
    let is_match_cell = sheet
        .conditionals()
        .any(|cid| sheet.conditional_match_cells(cid).is_some_and(|c| c.contains(&id)));
```

- [ ] **Step 6: Wrap `inspector.rs`'s test-only `add_conditional` call site**

In `begin/src/inspector.rs`'s `cell_needs_full_propagate_true_for_conditional_match_cell`
test, change:

```rust
    fn cell_needs_full_propagate_true_for_conditional_match_cell() {
        use adam_rs::Method;

        let mut sheet = Sheet::new();
        let p = sheet.add_cell(0_i32);
        let a = sheet.add_cell(0.0_f64);
        let b = sheet.add_cell(0.0_f64);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |v: &f64| Ok(*v))])
            .unwrap();
        sheet
            .add_conditional(p, vec![(vec![0_i32], vec![rel])], vec![])
            .unwrap();

        assert!(cell_needs_full_propagate(&sheet, p));
    }
```

to:

```rust
    fn cell_needs_full_propagate_true_for_conditional_match_cell() {
        use adam_rs::{MatchExpr, Method};

        let mut sheet = Sheet::new();
        let p = sheet.add_cell(0_i32);
        let a = sheet.add_cell(0.0_f64);
        let b = sheet.add_cell(0.0_f64);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |v: &f64| Ok(*v))])
            .unwrap();
        sheet
            .add_conditional(MatchExpr::cell(p), vec![(vec![0_i32], vec![rel])], vec![])
            .unwrap();

        assert!(cell_needs_full_propagate(&sheet, p));
    }
```

- [ ] **Step 7: Run `begin`'s tests**

Run: `cargo test -p begin --no-default-features` and `cargo test -p begin`
Expected: PASS once Task 4 also lands (see Step 4's note above about the `adam-lang`
dependency — it's fine if this still fails on `adam-lang` compile errors until Task 4 is
done; re-run at the end of Task 4 to confirm green).

- [ ] **Step 8: Commit**

```bash
git add begin/src/bridge.rs begin/src/inspector.rs
git commit -m "fix(begin): adapt to adam-rs's plural conditional_match_cells API"
```

---

## Task 4: Fix `adam-lang` crate call sites

**Files:**
- Modify: `adam-lang/src/type_registry.rs`
- Modify: `adam-lang/src/parser.rs`

**Interfaces:**
- Consumes: `MatchExpr` from Task 1.
- Produces: nothing new — mechanical follow-on only. `AddConditionalFn`'s own signature
  (still keyed on `CellId`, per the design spec §5) does **not** change; only its
  implementation wraps the cell.

- [ ] **Step 1: Wrap the cell inside `add_conditional_impl`**

In `adam-lang/src/type_registry.rs`, add `MatchExpr` to the import:

```rust
use adam_rs::{CellId, ConditionalId, RelationshipId, Sheet};
```

becomes:

```rust
use adam_rs::{CellId, ConditionalId, MatchExpr, RelationshipId, Sheet};
```

Then change:

```rust
fn add_conditional_impl<T: Any + PartialEq + 'static>(
    sheet: &mut Sheet,
    cell: CellId,
    branches: Vec<(Box<dyn Any>, Vec<RelationshipId>)>,
    default: Vec<RelationshipId>,
) -> Result<ConditionalId, adam_rs::Error> {
    let typed_branches: Vec<(Vec<T>, Vec<RelationshipId>)> = branches
        .into_iter()
        .map(|(val, rel_ids)| {
            let v = *val
                .downcast::<T>()
                .expect("add_conditional_impl: type matches registration");
            (vec![v], rel_ids)
        })
        .collect();
    sheet.add_conditional::<T>(cell, typed_branches, default)
}
```

to:

```rust
fn add_conditional_impl<T: Any + PartialEq + 'static>(
    sheet: &mut Sheet,
    cell: CellId,
    branches: Vec<(Box<dyn Any>, Vec<RelationshipId>)>,
    default: Vec<RelationshipId>,
) -> Result<ConditionalId, adam_rs::Error> {
    let typed_branches: Vec<(Vec<T>, Vec<RelationshipId>)> = branches
        .into_iter()
        .map(|(val, rel_ids)| {
            let v = *val
                .downcast::<T>()
                .expect("add_conditional_impl: type matches registration");
            (vec![v], rel_ids)
        })
        .collect();
    sheet.add_conditional::<T>(MatchExpr::cell(cell), typed_branches, default)
}
```

- [ ] **Step 2: Wrap the cell in `parser.rs`'s tuple-shape branch**

In `adam-lang/src/parser.rs`, add `MatchExpr` to the import:

```rust
use adam_rs::{CellId, Condition, Method, OutputId, RelationshipId, Sheet};
```

becomes:

```rust
use adam_rs::{CellId, Condition, MatchExpr, Method, OutputId, RelationshipId, Sheet};
```

Then change:

```rust
            TypeShape::Tuple(_) => {
                let typed_branches: Vec<(Vec<cel_runtime::DynamicSequence>, Vec<RelationshipId>)> =
                    branches
                        .into_iter()
                        .map(|(val, rel_ids)| {
                            let seq = *val.downcast::<cel_runtime::DynamicSequence>().expect(
                                "eval_segment_boxed: a Tuple shape always boxes a \
                                     DynamicSequence",
                            );
                            (vec![seq], rel_ids)
                        })
                        .collect();
                ctx.sheet
                    .add_conditional::<cel_runtime::DynamicSequence>(
                        match_cell_id,
                        typed_branches,
                        default_rel_ids,
                    )
                    .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
            }
```

to:

```rust
            TypeShape::Tuple(_) => {
                let typed_branches: Vec<(Vec<cel_runtime::DynamicSequence>, Vec<RelationshipId>)> =
                    branches
                        .into_iter()
                        .map(|(val, rel_ids)| {
                            let seq = *val.downcast::<cel_runtime::DynamicSequence>().expect(
                                "eval_segment_boxed: a Tuple shape always boxes a \
                                     DynamicSequence",
                            );
                            (vec![seq], rel_ids)
                        })
                        .collect();
                ctx.sheet
                    .add_conditional::<cel_runtime::DynamicSequence>(
                        MatchExpr::cell(match_cell_id),
                        typed_branches,
                        default_rel_ids,
                    )
                    .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
            }
```

- [ ] **Step 3: Run `adam-lang`'s tests**

Run: `cargo test -p adam-lang`
Expected: PASS — the `Named` shape branch (line ~521, `add_cond_fn(&mut ctx.sheet,
match_cell_id, branches, default_rel_ids)`) is untouched since it goes through
`add_conditional_impl`, which now does the wrapping internally; the `AddConditionalFn`
function-pointer type itself stays `fn(&mut Sheet, CellId, ..., ...) -> Result<...>`.

- [ ] **Step 4: Re-run `begin`'s tests now that `adam-lang` compiles**

Run: `cargo test -p begin --no-default-features` and `cargo test -p begin`
Expected: PASS (this confirms Task 3's changes, which depend on `adam-lang` compiling).

- [ ] **Step 5: Commit**

```bash
git add adam-lang/src/type_registry.rs adam-lang/src/parser.rs
git commit -m "fix(adam-lang): wrap conditional match cells in MatchExpr::cell"
```

---

## Task 5: Full workspace validation

**Files:** none (verification only; fix any residual warnings found in whichever files
they appear).

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Expected: no diff (or a clean diff from formatting-only changes — commit those if any).

- [ ] **Step 2: Build the whole workspace**

Run: `cargo build --workspace`
Expected: builds with **zero warnings**. Fix any warning found (e.g. an unused import left
over from a rename) before continuing.

- [ ] **Step 3: Test the whole workspace, including doc tests**

Run: `cargo test --workspace` then `cargo test --doc --workspace`
Expected: all tests pass, zero warnings.

- [ ] **Step 4: Lint (all three required invocations)**

Run, in order:

```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

Expected: zero warnings from all three. Fix any lint findings (e.g. a needless `.clone()`
in one of the mechanical fixups) before continuing.

- [ ] **Step 5: Doc build sanity check**

Run: `cargo doc --lib --no-deps --workspace`
Expected: builds cleanly — confirms every new/changed `///` doc comment (in particular
`MatchExpr`'s intra-doc links to `Method`/`Sheet::add_conditional`) resolves.

- [ ] **Step 6: Commit any residual fixes**

If Steps 1–5 required any code changes beyond formatting, commit them:

```bash
git add -A
git commit -m "chore: fix residual warnings from conditional match-expression refactor"
```

If no changes were needed beyond what Tasks 1–4 already committed, skip this step — there's
nothing to commit.
