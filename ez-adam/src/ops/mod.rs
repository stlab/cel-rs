//! Pure mutation functions over a [`crate::model::document::Document`].
//! Every editor interaction (toolbar clicks, side-panel edits) goes through
//! one of these functions rather than mutating `Document`'s fields
//! directly, so UI event handlers stay thin passthroughs.

pub mod cells;
pub mod relationships;
