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
pub mod condition;
pub mod conditional;
pub mod error;
pub mod filter;
pub mod output;
mod planner;
pub mod relationship;
pub mod sheet;

pub use cell::CellId;
pub use condition::{Condition, ConditionId};
pub use conditional::{ConditionalId, MatchExpr};
pub use error::Error;
pub use output::OutputId;
pub use relationship::{Method, RelationshipId};
pub use sheet::Sheet;
