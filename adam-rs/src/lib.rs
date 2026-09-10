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
//! An out cell is always derived by exactly one fixed writer method, with named
//! requirements checked after every `propagate()`. Unlike an ordinary derived cell, an
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
//!         vec![(
//!             "max_area",
//!             Requirement::from_fn_2([area, max_area], |a: &i32, max: &i32| Ok(a <= max)),
//!         )],
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
//!   ([`Error::Cycle`]/[`Error::FilterCycle`] when not).
//! - A self-referencing input never reads a same-round derived value. Instead it reads a
//!   seed reconstructed from `source`: `planner::build_seeds` folds every other
//!   relationship incident to the cell (excluding its own claimant) through that
//!   relationship's cell-producing method, giving the cell's aspiration before its
//!   claimant tightens it; a cell no other relationship contributes to just reads its
//!   own `source`. The seed is rebuilt from `source` every round, so it is never stale
//!   and never accumulates.
//! - `source` is written only by [`Sheet::write`]/[`Sheet::add_cell`], never by method
//!   or filter execution.
//! - An `Out`-kind cell can never be [`Sheet::write`]-ed or claimed as another method's
//!   output ([`Error::InvalidCellKind`]).
//!
//! Enforced only by convention (caller contract, not checked by the runtime):
//!
//! - A self-referencing method must be idempotent: applying it twice to the same inputs
//!   must produce the same result as applying it once.
//! - A filter must be a pure, conforming function of its cell's value and its argument
//!   cells' values.
//! - Iteration order used to break ties among equal-strength cells is not stable API
//!   and must not be relied on by callers.
//!
//! The source-capture property:
//!
//! Capturing every cell's `source` value and reapplying the highest-strength sources
//! reconstructs the sheet's *current* state, but not what a subsequent edit will do,
//! since no method-selection state is captured, only values. This holds even for a
//! self-referencing cell: `planner::build_seeds` reconstructs its contribution to a
//! sibling relationship's method purely from current `source` values (recursively, for
//! any other self-referencing cell that sibling method itself reads), so no state
//! beyond `source` needs to be captured to reproduce the derived state.

pub mod cell;
pub mod conditional;
pub mod error;
pub mod filter;
mod planner;
pub mod relationship;
pub mod requirement;
pub mod sheet;

pub use cell::{CellId, CellKind};
pub use conditional::{ConditionalId, MatchExpr};
pub use error::{Error, ErrorLocation};
pub use filter::{Filter, FilterKind, FilterViolation};
pub use relationship::{Method, RelationshipId};
pub use requirement::{Requirement, RequirementId};
pub use sheet::Sheet;
