//! CEL standard library: `min`, `max`, `clamp`, and related numeric functions built on
//! Rust's standard library, registered via [`cel_parser::OpLookup::push_library_scope`].
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
///
/// These functions are registered as library scopes, meaning they are always reachable,
/// including inside closure bodies. Library functions should never be isolated; only
/// transient, per-declaration scopes (pushed via [`cel_parser::OpLookup::push_scope`])
/// are isolated during nested body compilation.
pub fn install(lookup: &mut cel_parser::OpLookup) {
    lookup.push_library_scope(math::min_max_scope);
    lookup.push_library_scope(math::clamp_scope);
    lookup.push_library_scope(math::abs_scope);
    lookup.push_library_scope(math::unary_math_scope);
}
