//! cel-rs provides a stack-based runtime for developing domain specific languages. A program is composed of segments, where each segment is a sequence of operations.
//!
//! Segments can be created in two ways.
//!
//! 1. Using the `[Self::DynSegment]` struct which validates the type safety of the operations at runtime as the segment is built.
//! 2. Using the `[Self::Segment]` struct, which validates the type safety of the operations at compile time.
//!
//! The two types of segments can be converted to each other [not yet implemented from `Segment` to `DynSegment`].
//!
//! # Examples
//!
//! ```rust
//! use cel_rs::segment::Segment;
//!
//! // Create a segment that takes a u32 and &str as arguments
//! let segment = Segment::<(u32, &str)>::new();
//!
//! ```
pub mod c_stack_list;
pub mod dyn_sement;
pub mod memory;
pub mod raw_segment;
pub mod raw_sequence;
pub mod raw_stack;
pub mod raw_vec;
pub mod segment;
pub mod type_list;
