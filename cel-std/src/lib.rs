//! CEL standard library: `min`, `max`, `clamp`, and related numeric functions built on Rust's
//! standard library, registered via [`cel_parser::OpLookup::register_operation`].
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

/// Registers every CEL standard-library function on `lookup`: `min`, `max`, `clamp`, `abs`,
/// `signum`, `sqrt`, `floor`, `ceil`, `trunc`, `fract`, and `round_ties_even`.
///
/// These functions use persistent named registrations, so they are always reachable,
/// including inside closure bodies. Library functions are never isolated; only
/// transient, per-declaration scopes (pushed via [`cel_parser::OpLookup::push_scope`])
/// are isolated during nested body compilation.
///
/// - Precondition: No standard numeric signature is already installed.
///   Independent libraries may add disjoint overloads before or after installation.
/// - Complexity: O(s²) in the number of installed operation signatures.
///
/// # Examples
/// ```
/// let mut lookup = cel_parser::OpLookup::new();
/// cel_std::install(&mut lookup);
/// assert_eq!(lookup.operation_signatures("min").count(), 14);
/// ```
pub fn install(lookup: &mut cel_parser::OpLookup) {
    math::install(lookup).expect("CEL standard-library signatures must be installed once");
}
