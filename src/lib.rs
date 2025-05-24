//! cel-rs provides a stack-based runtime for developing domain specific languages, including
//! concative languages to describe concurrent processes. A sequence is a list of operations (the
//! machine instructions). Each operation is a closure that takes it's arguments from the stack and
//! the result is pushed back onto the stack.
//!
//! Segments can be created in two ways.
//!
//! 1. Using the [`DynSegment`] struct which validates the type safety of the
//!    operations at runtime as the segment is built.
//! 2. Using the [`Segment`] struct, which validates the type safety of the
//!    operations at compile time.
//!
//! The two types of segments can be converted to each other [not yet implemented from `Segment` to
//! `DynSegment`].
//!
//! Operations are added using the form `op#[r]` where # is the arity of the operation and the
//! optional `r` signifies that the operation returns a [`std::result::Result`] type and may fail.
//!
//! # Examples
//!
//! ```rust
//! use cel_rs::*;
//!
//! // Create a segment that takes a u32 and &str as arguments
//! let segment = Segment::<(u32, &str)>::new()
//!     .op1r(|s| {
//!         let r = s.parse::<u32>()?;
//!         Ok(r)
//!     })
//!     .op2(|a, b| a + b)
//!     .op1(|r| r.to_string());
//! assert_eq!(segment.call((1u32, "2")).unwrap(), "3");
//! ```

// #![warn(missing_docs)]
pub mod c_stack_list;
pub mod dyn_sement;
pub mod list_traits;
pub mod memory;
pub mod raw_segment;
pub mod raw_sequence;
pub mod raw_stack;
pub mod raw_vec;
pub mod segment;
pub mod tuple_list;

pub use c_stack_list::*;
pub use dyn_sement::*;
pub use list_traits::*;
pub use memory::*;
pub use raw_segment::*;
pub use raw_sequence::*;
pub use raw_stack::*;
pub use raw_vec::*;
pub use segment::*;
//pub use tuple_list::*;
