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
//! # Composable canonical values
//!
//! [`Array<T>`] owns actual contiguous `T` values. [`Array::into_vec`] and
//! [`Array::from_vec`] transfer allocations in O(1); [`Array::as_slice`] borrows
//! them. A nested array uses `Array<Array<T>>`: its vector view is
//! `Vec<Array<T>>`, not `Vec<Vec<T>>`. Deep native conversion builds a new outer
//! vector. Tuples use source-ordered [`CStackList`] values ending in [`CNil<()>`],
//! bridged to native tuples by [`IntoCStackList`] and [`IntoRustTuple`].
//! Source unit uses [`Nil`], not an opaque host [`Leaf<()>`].
//!
//! [`ValueType`] records complete recursive identity/layout and optional
//! capabilities; [`ArrayOf`], [`Cons`], [`Nil`], [`Leaf`], and [`FnOf`] witness
//! concrete representations. Moving storage requires no clone/equality/debug
//! bounds. [`OwnedValue`] checks arbitrary erased external values, whereas
//! [`DynSegment::prepare`] binds a [`PreparedSegment`] once. Repeated prepared
//! calls borrow arguments and return independent owners without comparing
//! descriptors or downcasting. Runtime-bound hosts can instead prepare a
//! [`prepared_segment::PreparedRawSegment`] and uphold its unsafe pointer contract.
//! [`Callable`] binds native or prepared bodies to the same typed interface.
//! This is interoperability with native Rust, not a macro-compiled CEL backend.
//!
//! ```
//! use cel_runtime::{ArrayOf, Cons, IntoCStackList, IntoRustTuple, Leaf, Nil, DynSegment};
//! type Row = Cons<Leaf<i32>, Cons<Leaf<bool>, Nil>>;
//! let mut body = DynSegment::new::<()>();
//! body.just(7i32); body.just(true); body.make_tuple(2, 0).unwrap();
//! body.make_array(1, 0).unwrap();
//! let mut prepared = body.prepare::<Nil, ArrayOf<Row>>().unwrap();
//! let row = prepared.call(&().into_c_stack_list()).unwrap().into_vec().pop().unwrap();
//! assert_eq!(row.into_rust_tuple(), (7, true));
//! ```
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

/// Concrete typed ownership for homogeneous, contiguous arrays.
pub mod array;
/// Compile-time stack list implementation for type-safe stack operations.
pub mod c_stack_list;
/// Private validated canonical host-operation emission into the existing raw segment.
mod canonical_operation;
/// Private canonical container construction and trusted frame moves.
mod container_execution;
/// First-class callable CEL values with declared signatures.
pub mod dyn_closure;
/// Dynamic segment implementation with runtime type checking.
pub mod dyn_segment;
/// Checked adapters for the existing external host boundaries.
mod host_value_bridge;
/// Traits for working with type lists and type information.
pub mod list_traits;
/// Memory management and alignment utilities for the runtime.
pub mod memory;
/// Whole-value ownership with complete checked representation witnesses.
pub mod owned_value;
/// Prepared typed execution over the existing raw interpreter.
pub mod prepared_segment;
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
/// Canonical tuple-list layout support.
mod tuple_value;
/// Complete recursive canonical metadata and sealed concrete representation witnesses.
pub mod value_type;

pub use array::Array;
/// Signature-typed callable facade over the canonical shared carrier.
pub mod callable;
pub use c_stack_list::*;
pub use callable::Callable;
pub use dyn_closure::{CallDynFn, DynClosure};
pub use dyn_segment::*;
pub use list_traits::*;
pub use memory::*;
pub use owned_value::OwnedValue;
pub use prepared_segment::PreparedSegment;
pub use raw_segment::*;
pub use raw_sequence::*;
pub use raw_stack::*;
pub use raw_vec::*;
pub use segment::*;
pub use tuple_list::*;
pub use value_type::{
    ArgumentList, ArrayOf, AssociatedType, Cons, FnOf, Leaf, Nil, ValueKind, ValueRepr, ValueType,
};
