//! # adam-rs
//!
//! A library for constructing and executing property model constraint graphs.
//!
//! A property model is a bipartite graph of **value cells** and **relationships**.
//! Cells hold type-erased values. Relationships define multi-way constraints: each
//! relationship supplies multiple methods, and at propagation time the planner
//! selects one method per relationship based on cell write-recency (strength),
//! then executes the selected methods in dependency order.
//!
//! # Callback purity
//!
//! Every [`Method`], [`Filter`], [`MatchExpr`], and [`Requirement`] callback must be
//! purely functional: its result depends only on its explicit input values and
//! immutable captured constants. Equal input values must produce equivalent output
//! values or errors. This also applies to range-filter bound evaluators and
//! conditional equality functions.
//!
//! Callbacks must not mutate inputs or captured state, perform I/O, or read mutable
//! external state such as clocks, random generators, or independently changing shared
//! data. Represent changing dependencies as input cells instead.
//!
//! There is no guarantee that a callback will execute, or when or how often it will
//! execute. The runtime may omit evaluation, reuse a previously computed result, or
//! reevaluate a callback while preserving the sheet's value and diagnostic semantics.
//! Observe propagation through cell values and diagnostics, not callback side effects.
//! Rust's `Fn` bound does not enforce purity; this is a caller obligation.
//!
//! # Example
//!
//! ```rust
//! use adam_rs::{Sheet, Method};
//!
//! let mut sheet = Sheet::new();
//! let a = sheet.add_cell(2.0_f64);
//! let b = sheet.add_cell(3.0_f64);
//! let c = sheet.add_cell(0.0_f64);
//!
//! // Three methods encoding a × b = c in each direction.
//! let methods = vec![
//!     Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok((*x) * (*y))),
//!     Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok((*y) / (*x))),
//!     Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok((*y) / (*x))),
//! ];
//! sheet.add_relationship(methods).unwrap();
//!
//! sheet.write(a, 2.0_f64).unwrap();
//! sheet.write(b, 3.0_f64).unwrap();
//! sheet.propagate().unwrap();
//!
//! assert_eq!(*sheet.read::<f64>(c).unwrap(), 6.0);
//! ```
//!
//! # Out cells and requirements
//!
//! An out cell is always derived by exactly one fixed writer method, with optional
//! requirements checked after every `propagate()`. Display labels belong to the caller,
//! not the sheet. Unlike an ordinary derived cell, an
//! out cell can never be `write()`-ed or claimed as another method's output — but it
//! remains an ordinary, freely-referenceable cell everywhere else (as another
//! relationship's input, a conditional's match subject, and so on).
//!
//! ```rust
//! use adam_rs::{Requirement, Method, Sheet};
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
//! let area_cell = sheet
//!     .add_out(
//!         writer,
//!         vec![Requirement::from_fn_2([area, max_area], |a: &i32, max: &i32| Ok(a <= max))],
//!     )
//!     .unwrap();
//!
//! sheet.write(width, 20_i32).unwrap();
//! sheet.write(height, 3_i32).unwrap();
//! sheet.propagate().unwrap();
//! assert!(sheet.cell_requirements_valid(area_cell));
//!
//! sheet.write(height, 30_i32).unwrap();
//! sheet.propagate().unwrap();
//! assert!(!sheet.cell_requirements_valid(area_cell));
//! ```
//!
//! # Filters
//!
//! A filter conforms or rejects a value written externally to its cell — applied
//! live by `propagate()` against the cell's own current value, not synchronously by
//! `write()`. It's also re-checked, as a non-gating diagnostic only, against a value
//! a relationship's method derives for that cell — a derived value is never
//! corrected, only flagged.
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
//! // write() leaves the raw value in source; it is conformed by propagate()...
//! sheet.write(a, 500_i32).unwrap();
//! assert_eq!(*sheet.read::<i32>(a).unwrap(), 500);
//!
//! sheet.propagate().unwrap();
//! assert_eq!(*sheet.read::<i32>(a).unwrap(), 100);
//!
//! // A derived value that would fail the same filter is only diagnosed, never
//! // corrected: `b` doubles `a`'s now-conformed value, still exceeding the filter's range.
//! assert_eq!(*sheet.read::<i32>(b).unwrap(), 200);
//! assert!(sheet.filter_violated_cells().any(|id| id == b));
//! ```
//!
//! # Invariants
//!
//! Enforced by code:
//!
//! - Every relationship's methods share the same `inputs ∪ outputs` cell set
//!   ([`Sheet::add_relationship`] validation; [`Error::MismatchedMethodCells`]).
//! - No two relationships may claim the same cell as an output in one round —
//!   self-referencing outputs are claimed exactly like any other
//!   ([`Error::Conflict`] when infeasible).
//! - The selected methods' induced dependency digraph is acyclic before execution
//!   ([`Error::Cycle`] when not).
//! - A self-referencing input never reads a same-round derived value. Instead it reads a
//!   seed reconstructed from `source`: `planner::evaluate_seeds` folds every other
//!   relationship incident to the cell (excluding its own claimant) through that
//!   relationship's cell-producing method, giving the cell's aspiration before its
//!   claimant tightens it; a cell no other relationship contributes to just reads its
//!   own `source`. The seed is rebuilt from `source` every round, so it is never stale
//!   and never accumulates.
//! - [`Sheet::write`]/[`Sheet::add_cell`] establish `source` aspirations. Ordinary
//!   unshadowed method outputs replace `source` at commit; self-referencing outputs
//!   and source filters preserve aspirations and publish derived overrides instead.
//! - An `Out`-kind cell can never be [`Sheet::write`]-ed or claimed as another method's
//!   output ([`Error::InvalidCellKind`]).
//!
//! Enforced only by convention (caller contract, not checked by the runtime):
//!
//! - All method, filter, conditional-expression, and requirement callbacks satisfy
//!   the callback-purity contract above, with no invocation guarantee.
//! - Every method of a relationship conforms the same constraint and preserves an
//!   assignment that already satisfies it. This semantic consistency is stronger
//!   than the runtime's check that methods reference the same cell set.
//! - A self-referencing method must be idempotent: applying it twice to the same inputs
//!   must produce the same result as applying it once.
//! - A filter must be idempotent and conform its cell's value using its argument
//!   cells' values.
//! - Iteration order used to break ties among equal-strength cells is not stable API
//!   and must not be relied on by callers.
//!
//! # Capture and playback
//!
//! After successful propagation, [`Sheet::source_cells`] identifies unclaimed cells
//! and selected self-referencing inputs. Capture their effective [`Sheet::read`]
//! values, not their [`Sheet::source`] aspirations. Pure derived outputs need not
//! be recorded.
//!
//! Replay writes all captured values, in any order, before calling
//! [`Sheet::propagate`]. For a settled solution satisfying its relationships and
//! filters, a compatible sheet with matching uncaptured inputs reproduces the same
//! effective values. Semantic consistency and idempotence make the recorded solution
//! a fixed point even when the new write priorities select different methods. No
//! strengths or write history need to be recorded.
//!
//! Clients choose their initial state after construction and own change comparison,
//! relevance filtering, stable cell names or schema keys, and typed codecs. Do not
//! persist live [`CellId`] values. Filtering out an input makes its replay value
//! part of the context that must match; it is not reconstructed by capture.
//!
//! A value snapshot is not an editable-state checkpoint. It does not preserve
//! original aspirations, priority history, diagnostics, or behavior under subsequent
//! edits, including spring-back. Capture only after successful propagation and
//! before further edits; this API neither consumes a journal nor changes the sheet.

pub mod cell;
pub mod conditional;
pub mod error;
pub mod filter;
mod planner;
pub mod relationship;
pub mod requirement;
pub mod sheet;
pub mod template;

pub use cell::{CellId, CellKind};
pub use conditional::{ConditionalId, MatchExpr};
pub use error::{Error, ErrorSite};
pub use filter::{Filter, FilterKind, FilterViolation};
pub use relationship::{Method, RelationshipId};
pub use requirement::{Requirement, RequirementId};
pub use sheet::Sheet;
pub use template::{
    CellSlot, ConstructionSite, Instance, InstanceOccurrence, MethodOrigin, MethodRecipe,
    OccurrenceId, RelationshipSlot, SheetTemplate, TemplateBuilder, TemplateError, TemplateId,
    TemplateInclusion,
};
