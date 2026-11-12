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

/// Registers the CEL numeric standard library on `lookup`.
///
/// Includes `min`, `max`, `clamp`, `abs`, `signum`, `sqrt`, `floor`, `ceil`,
/// `trunc`, `fract`, `round_ties_even`, `round`, `sin`, `cos`, `tan`, `asin`,
/// `acos`, `atan`, `atan2`, `sin_cos`, `to_radians`, `to_degrees`, `hypot`,
/// `cbrt`, `powi`, `powf`, `exp`, `exp2`, `ln`, `log`, `log2`, `log10`,
/// `recip`, `mul_add`, `rem_euclid`, and `copysign`.
///
/// Both `sin(x)` and `x.sin()` use the same Rust floating-point semantics.
/// Float operations accept `f32` and `f64` without implicit conversions.
/// `powi` takes an `i32` exponent; `sin_cos` returns a CEL `(F, F)` tuple
/// ordered as sine then cosine, where `F` is the input float type.
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
/// let mut parser = cel_parser::CELParser::new(lookup);
/// let mut value = parser.parse_str("(90.0).to_radians().sin()").unwrap();
/// assert!((value.call0::<f64>().unwrap() - 1.0).abs() < 1e-12);
/// ```
pub fn install(lookup: &mut cel_parser::OpLookup) {
    math::install(lookup).expect("CEL standard-library signatures must be installed once");
}
