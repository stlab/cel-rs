/*!
A library for type-safe stack-based operations.

This library provides a type-safe way to work with stack-based operations,
allowing you to build and execute sequences of operations while maintaining
type safety at compile time.

# Examples

```rust
use cel_rs::type_list::{List, IntoList};

// Create a type list from a tuple
let list = (1, "hello", 3.14).into_list();
```
*/
mod dyn_segment;
mod raw_segment;
mod raw_sequence;
mod raw_stack;
mod segment;
pub mod type_list;

pub use dyn_segment::DynSegment;
pub use raw_segment::RawSegment;
pub use raw_sequence::RawSequence;
pub use raw_stack::RawStack;
pub use segment::Segment;
