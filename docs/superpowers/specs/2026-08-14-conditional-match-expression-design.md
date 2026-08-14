# Native match-expressions for `adam-rs` conditionals

**Status:** Approved for implementation planning
**Crate:** `adam-rs` (with mechanical follow-on changes in `adam-lang`, `begin`)

## Motivation

Fixes [#99](https://github.com/stlab/cel-rs/issues/99). `adam-lang`'s conditional grammar
and `adam-rs::Sheet::add_conditional` only accept a single existing cell as the match
subject. Branching on a combination of cells (e.g. `a && b`) today requires first declaring
a synthetic cell and a relationship to compute it, then conditioning on that derived cell —
boilerplate for something that should read as an inline expression:
`conditional a && b { true => { ... } }`.

Per the issue's own analysis, two shapes were considered: desugaring in `adam-lang` into a
synthetic cell (keeps `adam-rs` untouched, but the synthetic cell pollutes `begin`'s graph
and falsifies `conditional.rs`'s "each conditional binds to one cell" doc comment), or native
support in `adam-rs` (extend `ConditionalData`/`add_conditional` to hold a method-like
expression over a set of input cells). This spec implements the native-support shape — it's
the one the issue frames as keeping "the model surface honest" and is explicitly what this
task asks for. Grammar changes in `adam-lang` (parsing `conditional a && b { ... }` itself)
are **out of scope** for this spec; only the `adam-rs` infrastructure — and the mechanical
call-site fixes elsewhere needed to keep the workspace compiling — are covered.

## 1. `MatchExpr`: a method-like construct for match subjects (`adam-rs/src/conditional.rs`)

Mirrors the existing `Method` type (`relationship.rs`), but produces a single type-erased
value for comparison against branch keys instead of writing to output cells.

```rust
pub struct MatchExpr(pub(crate) MatchSource);

pub(crate) enum MatchSource {
    /// Today's plain case: the match value is an existing cell's current value, read
    /// directly with no allocation and no extra trait bounds.
    Cell(CellId),
    /// A computed value over multiple input cells.
    Expr(MatchExprData),
}

pub(crate) struct MatchExprData {
    inputs: Vec<CellId>,
    input_types: Vec<TypeId>,
    output_type: TypeId,
    eq_fn: fn(&dyn Any, &dyn Any) -> bool,
    function: Box<dyn Fn(&[&dyn Any]) -> Result<Box<dyn Any>, anyhow::Error>>,
}
```

Public constructors:

- `MatchExpr::cell(cell: CellId) -> Self` — wraps today's plain case. Takes **no** type
  parameter; `add_conditional::<T>` validates it against the cell's own registered type
  exactly as today. Every existing `add_conditional(cell, ...)` call site becomes
  `add_conditional(MatchExpr::cell(cell), ...)`.
- `MatchExpr::new<F>(inputs: Vec<CellId>, input_types: Vec<TypeId>, output_type: TypeId, eq_fn: fn(&dyn Any, &dyn Any) -> bool, f: F) -> Self` —
  the general type-erased constructor (mirrors `Method::new`). This is the one a future
  `adam-lang` expression-conditional will plug into: `adam-lang`'s `TypeRegistry` already
  builds `Method`s from compiled `cel_runtime::DynSegment`s via an identical
  `Fn(&[&dyn Any]) -> Result<Box<dyn Any>, _>` shape (`call_dyn_impl` in `type_registry.rs`),
  so a runtime-length list of dependency cells extracted from a parsed expression slots in
  directly, with no per-arity ceiling.
- `MatchExpr::from_fn_1<A, T, F>(input: CellId, f: F) -> Self` and
  `MatchExpr::from_fn_2<A, B, T, F>(inputs: [CellId; 2], f: F) -> Self` — typed convenience
  sugar for Rust-side use and tests, mirroring `Method::from_fn_1_1`/`from_fn_2_1`. `T`
  requires `Any + PartialEq + 'static` (captured into `eq_fn` the same way
  `Sheet::add_cell`/`CellData::eq_fn` already does it); `A`/`B` require only `Any + 'static`.

## 2. `ConditionalData` and dependency deduction

`ConditionalData.cell: CellId` is replaced with `source: MatchSource`, plus a
`match_cells(&self) -> &[CellId]` accessor (`[cell]` for `Cell`, `&expr.inputs` for `Expr`).
This is the actual mechanism the issue asks for — "deduce the `[a, b]` dependency from the
expression" — and every place that currently reads `.cell` generalizes to `match_cells()`:

- `add_conditional`'s upstream `contributing_cells` BFS (the guard that rejects a branch
  relationship touching a cell upstream of the match subject) is seeded with **every**
  match cell, not one, so a branch relationship touching either `a`'s or `b`'s upstream
  contributors is correctly rejected as ambiguous — generalizing the existing single-cell
  guard (`add_conditional_returns_error_when_branch_rel_involves_cell_upstream_of_match_cell`).
- `propagate()`'s phase-1 pre-plan (`match_cell_subgraph`, which already takes `&[CellId]` —
  no change needed there, a good sign this generalizes along an existing seam) is fed the
  flattened `match_cells()` of every conditional.
- `cell_has_prior_use` and `conditionals_potentially_producing` switch from `.cell` to
  `.match_cells()`.

## 3. `Sheet::add_conditional` signature and validation

```rust
pub fn add_conditional<T: Any + PartialEq + 'static>(
    &mut self,
    source: MatchExpr,          // was: cell: CellId
    branches: Vec<(Vec<T>, Vec<RelationshipId>)>,
    default: Vec<RelationshipId>,
) -> Result<ConditionalId, Error>
```

Validation branches on `source.0`:

- `MatchSource::Cell(cell)` — unchanged from today: `InvalidId` if not in the sheet,
  `TerminalCell` if terminal, `InvalidConditional` if the cell's registered type doesn't
  match `T`.
- `MatchSource::Expr(expr)` — `InvalidConditional` if `expr.output_type != TypeId::of::<T>()`;
  for each `(cell_id, declared_type)` in `expr.inputs`/`expr.input_types`: `InvalidId` if not
  in the sheet, `TerminalCell` if terminal, `Error::TypeMismatch` if the cell's registered
  type doesn't match `declared_type` (mirrors `add_relationship`'s per-method input
  validation).

## 4. Evaluation plumbing

`build_active_set` and `conditional_active_branch` need a value-or-reference helper so the
same `eq_fn` comparison works uniformly regardless of variant:

```rust
enum MatchValue<'a> { Ref(&'a dyn Any), Owned(Box<dyn Any>) }
```

`Cell` variant borrows `cell.effective()` directly (no allocation, same cost as today).
`Expr` variant gathers `expr.inputs.iter().map(|&id| self.cells[id].effective())` and calls
`expr.function` **once per conditional per propagate** (not once per branch key), producing
an owned `Box<dyn Any>`; failure maps to `Error::MethodFailed`.

This makes both evaluation paths fallible, with two different signature consequences:

- `build_active_set` becomes `fn(&self) -> Result<HashSet<RelationshipId>, Error>`, called
  with `?` from its one call site in `propagate()` (which already returns `Result`).
- `conditional_active_branch` (public, currently `Option<usize>`) becomes
  `pub fn conditional_active_branch(&self, id: ConditionalId) -> Result<Option<usize>, Error>`.
  `Ok(None)` still covers both "no live conditional with this id" and "no branch key
  matched" (collapsing these two is existing precedent — today's version already does this).
  `Err(Error::MethodFailed(_))` is returned only when a live conditional's `Expr` evaluation
  fails.

## 5. Call sites outside `adam-rs` that must keep compiling

No grammar or behavior change is intended here — these are mechanical signature follow-ons:

- `begin/src/bridge.rs:476` — `sheet.conditional_match_cell(cond_id)` (singular) becomes
  `conditional_match_cells(cond_id)` (plural, `Option<&[CellId]>`); the constraint-link-per-
  match-cell rendering loop iterates the slice instead of a single value.
- `begin/src/bridge.rs:486` — `sheet.conditional_active_branch(cond_id)` now returns
  `Result<Option<usize>, Error>`. `to_graph_data` is read-only display code, not the
  `propagate()` path itself — by the time it runs, `propagate()` has already evaluated the
  same expression successfully, so a fresh failure here would itself be a precondition
  violation. Folds to `.ok().flatten()` with a one-line comment explaining why an error is
  swallowed here specifically (it isn't swallowed at the `propagate()` call site, which is
  where it matters).
- `begin/src/inspector.rs:45,100` — same plural-accessor update, `contains(&id)` instead of
  `== Some(id)`.
- `adam-lang/src/type_registry.rs` (`AddConditionalFn`, `add_conditional_impl`) and
  `adam-lang/src/parser.rs::parse_conditional_decl` — the `CellId` parameter threaded through
  becomes `MatchExpr::cell(match_cell_id)` at construction; behavior is identical to today
  since `adam-lang` doesn't yet parse expression match-subjects.
- All existing `adam-rs` test call sites (`tests/integration.rs`, `src/sheet.rs` unit tests,
  ~25 in total) wrap their existing `CellId` argument in `MatchExpr::cell(...)`.

## 6. New public type export

`adam-rs/src/lib.rs` re-exports `MatchExpr` alongside `Method`/`RelationshipId`.

## 7. Testing plan

- Update all existing `add_conditional` call sites to `MatchExpr::cell(...)` — must pass
  unchanged (this is the regression backstop for the whole refactor).
- New: 2-cell `MatchExpr` (`from_fn_2`, e.g. `a && b`-shaped) drives branch activation
  correctly, and re-evaluates when either input changes.
- New: contributing-cells BFS rejects a multi-method branch relationship touching either
  expression input's upstream contributors (generalizes
  `add_conditional_returns_error_when_branch_rel_involves_cell_upstream_of_match_cell` to a
  2-input expression).
- New: `add_conditional` returns `Error::InvalidConditional` when `expr.output_type` doesn't
  match `T`; `Error::InvalidId`/`Error::TerminalCell`/`Error::TypeMismatch` for a bad/terminal/
  mistyped expression input cell.
- New: an expression function returning `Err` surfaces as `Error::MethodFailed` from
  `propagate()`.
- New: `conditional_active_branch` returns `Ok(None)` for an unmatched/absent conditional and
  propagates `Err` only for a live `Expr`-sourced conditional whose function fails.
- `begin` bridge/inspector tests continue to pass against the renamed/refallibilized
  accessors (mechanical updates, no new `begin`-side test cases required by this spec).
