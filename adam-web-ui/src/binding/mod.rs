//! Owns event-loop-confined models and publishes accepted cell views.
//!
//! Construct a [`SheetOwner`] with a repeatable [`ModelFactory`], submit canonical
//! [`CellEdit`] groups, and explicitly restore accepted history after rejection.

mod owner;
mod types;

pub use owner::SheetOwner;
pub use types::*;

#[cfg(test)]
mod tests;
