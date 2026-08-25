//! Cell data: a named, typed value in the property model, independent of
//! where it's placed on the canvas (see
//! [`crate::model::cell_node::CellNode`]).

use serde::{Deserialize, Serialize};
use slotmap::new_key_type;

new_key_type! {
    /// A stable handle to a [`Cell`] in a
    /// [`crate::model::document::Document`].
    pub struct CellId;
}

/// Optional lower/upper bounds for a numeric cell, stored in the cell's own
/// concrete type so an out-of-range or fractional bound for that type
/// cannot be represented.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ClampRange<T> {
    /// The minimum bound for this range, if any.
    pub min: Option<T>,
    /// The maximum bound for this range, if any.
    pub max: Option<T>,
}

/// The value type of a [`Cell`], carrying a numeric variant's clamp range
/// inline so a clamp bound can't be attached to a `Bool`/`Text` cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CellType {
    /// A 64-bit floating-point cell type with optional clamp bounds.
    F64 {
        /// The clamp range for this F64 cell.
        clamp: ClampRange<f64>,
    },
    /// A 64-bit signed integer cell type with optional clamp bounds.
    I64 {
        /// The clamp range for this I64 cell.
        clamp: ClampRange<i64>,
    },
    /// A boolean cell type.
    Bool,
    /// A text/string cell type.
    Text,
}

impl CellType {
    /// An `F64` cell type with no clamp bounds.
    #[must_use]
    pub fn f64() -> Self {
        CellType::F64 {
            clamp: ClampRange::default(),
        }
    }

    /// An `I64` cell type with no clamp bounds.
    #[must_use]
    pub fn i64() -> Self {
        CellType::I64 {
            clamp: ClampRange::default(),
        }
    }
}

/// A named, typed value in the property model.
///
/// A `Cell`'s data is shared by every
/// [`CellNode`](crate::model::cell_node::CellNode) that places it on the
/// canvas — editing a `Cell`'s properties through any one of its nodes
/// updates the single shared value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    /// The name of this cell.
    pub name: String,
    /// The type of value this cell holds.
    pub ty: CellType,
    /// Whether this cell is an output cell.
    pub output: bool,
    /// Raw CEL boolean expression text; `_` refers to this cell's own
    /// value. Not currently emitted by `generate_adm2` — see
    /// <https://github.com/stlab/cel-rs/issues/146>.
    pub restrict: Option<String>,
}

impl Cell {
    /// Creates a new, non-output cell with no restriction.
    #[must_use]
    pub fn new(name: impl Into<String>, ty: CellType) -> Self {
        Cell {
            name: name.into(),
            ty,
            output: false,
            restrict: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f64_has_no_clamp_bounds_by_default() {
        assert_eq!(
            CellType::f64(),
            CellType::F64 {
                clamp: ClampRange {
                    min: None,
                    max: None
                }
            }
        );
    }

    #[test]
    fn i64_has_no_clamp_bounds_by_default() {
        assert_eq!(
            CellType::i64(),
            CellType::I64 {
                clamp: ClampRange {
                    min: None,
                    max: None
                }
            }
        );
    }

    #[test]
    fn new_is_not_output_and_has_no_restrict() {
        let cell = Cell::new("width_pixels", CellType::i64());
        assert_eq!(cell.name, "width_pixels");
        assert!(!cell.output);
        assert!(cell.restrict.is_none());
    }

    #[test]
    fn clamp_range_can_hold_only_a_minimum() {
        let range = ClampRange {
            min: Some(0i64),
            max: None,
        };
        assert_eq!(range.min, Some(0));
        assert_eq!(range.max, None);
    }
}
