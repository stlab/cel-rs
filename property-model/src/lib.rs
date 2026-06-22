//! # property-model
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
//! ```rust,ignore
//! use property_model::{Sheet, Method};
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

pub mod cell;
pub mod error;
mod planner;
pub mod relationship;
pub mod sheet;

pub use cell::CellId;
pub use error::Error;
pub use relationship::{Method, RelationshipId};
pub use sheet::Sheet;
