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
//! # Outputs and requirements
//!
//! An output is a terminal cell written by a single method, with named requirements
//! checked after every `propagate()`. Unlike an ordinary derived cell, an output's cell
//! can never be used as an input elsewhere in the sheet.
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
//! let output = sheet
//!     .add_output(
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
//! assert!(sheet.output_valid(output));
//!
//! sheet.write(height, 30_i32).unwrap();
//! sheet.propagate().unwrap();
//! assert!(!sheet.output_valid(output));
//! ```
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

pub mod cell;
pub mod conditional;
pub mod error;
pub mod filter;
pub mod output;
mod planner;
pub mod relationship;
pub mod requirement;
pub mod sheet;

pub use cell::CellId;
pub use conditional::{ConditionalId, MatchExpr};
pub use error::Error;
pub use filter::{Filter, FilterViolation};
pub use output::OutputId;
pub use relationship::{Method, RelationshipId};
pub use requirement::{Requirement, RequirementId};
pub use sheet::Sheet;
