//! CEL standard library: `min`, `max`, `clamp`, and related numeric functions built on
//! Rust's standard library, registered via [`cel_parser::OpLookup::push_scope`].
//!
//! # Examples
//!
//! ```rust
//! use cel_parser::OpLookup;
//!
//! let mut lookup = OpLookup::new();
//! cel_std::install(&mut lookup);
//! ```

mod math;

/// Registers every CEL standard-library function on `lookup`: `min`, `max`, `clamp`,
/// `abs`, `signum`, `sqrt`, `floor`, `ceil`, `trunc`.
pub fn install(lookup: &mut cel_parser::OpLookup) {
    lookup.push_scope(math::min_max_scope);
    lookup.push_scope(math::clamp_scope);
    lookup.push_scope(math::abs_scope);
    lookup.push_scope(math::unary_math_scope);
}
