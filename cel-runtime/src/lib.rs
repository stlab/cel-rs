//! cel-rs provides a stack-based runtime for developing domain specific languages, including
//! concatenative languages to describe concurrent processes. A sequence is a list of operations
//! (the machine instructions). Each operation is a closure that takes it's arguments from the stack
//! and the result is pushed back onto the stack.
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
//! use cel_runtime::*;
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

#![warn(missing_docs)]
/// Compile-time stack list implementation for type-safe stack operations.
pub mod c_stack_list;
/// Dynamic segment implementation with runtime type checking.
pub mod dyn_segment;
/// Traits for working with type lists and type information.
pub mod list_traits;
/// Memory management and alignment utilities for the runtime.
pub mod memory;
/// Raw segment implementation without type safety.
pub mod raw_segment;
/// Raw sequence implementation for operation sequences.
pub mod raw_sequence;
/// Raw stack implementation for low-level stack operations.
pub mod raw_stack;
/// Raw vector implementation for dynamic memory allocation.
pub mod raw_vec;
/// Type-safe segment implementation with compile-time validation.
pub mod segment;
/// Tuple list implementation for type-safe tuple operations.
pub mod tuple_list;

pub use c_stack_list::*;
pub use dyn_segment::*;
pub use list_traits::*;
pub use memory::*;
pub use raw_segment::*;
pub use raw_sequence::*;
pub use raw_stack::*;
pub use raw_vec::*;
pub use segment::*;
//pub use tuple_list::*;
