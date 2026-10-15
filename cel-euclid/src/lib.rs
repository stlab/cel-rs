//! Opt-in `f64` / [`euclid::UnknownUnit`] geometry for CEL.
//!
//! Install once on a fresh lookup; no parser or runtime built-ins change.
//! Named operations support ordinary calls and receiver calls (including
//! zero-argument property syntax). Operators require exact operand types.
//!
//! ```
//! use cel_parser::{CELParser, OpLookup};
//! let mut lookup = OpLookup::new();
//! cel_euclid::install(&mut lookup);
//! let mut parser = CELParser::new(lookup);
//! let mut segment = parser.parse_str(
//!     "(Point2D { y: 2.0, x: 1.0 } + Vector2D { x: 3.0, y: 4.0 }).x"
//! ).unwrap();
//! assert_eq!(segment.call0::<f64>().unwrap(), 4.0);
//! ```
//!
//! Geometry arithmetic, bounds and containment retain Euclid semantics, including
//! non-finite arithmetic and half-open containment. Three operations are checked:
//! `normalize` rejects zero/non-finite lengths and non-finite results; `inverse`
//! rejects non-finite coefficients, zero/non-finite determinants, non-finite
//! inverses and zero inverse coefficients with nonzero exact cofactors;
//! `outer_transformed_rect` rejects invalid or unrepresentable bounds.
//! Ordinary point/vector mapping, constructors and composition remain unchecked.
//! Checked inversion permits exact-zero cofactors and nonzero subnormal results.
//! Translation cofactor checks compare exact products, so underflow or rounding
//! before determinant scaling cannot disguise a lost nonzero coefficient.
//! Other rounding and conditioning errors retain Euclid semantics: checked
//! inversion does not guarantee exact-real arithmetic or numerical stability.
//!
//! `degrees` and `radians` construct angles from `f64` and extract the respective
//! scalar from angles. Matrix accessors use Euclid's six coefficient names.
//! `a.then(b)` applies `a` first, then `b`; vectors ignore translation.
//!
//! ```
//! use cel_parser::{CELParser, OpLookup};
//! let mut lookup = OpLookup::new();
//! cel_euclid::install(&mut lookup);
//! let mut parser = CELParser::new(lookup);
//! let mut segment = parser.parse_str(
//!     "translation(Vector2D { x: 10.0, y: 0.0 }).then(scale(2.0, 2.0))\
//!      .transform_point(Point2D { x: 1.0, y: 0.0 }).x"
//! ).unwrap();
//! assert_eq!(segment.call0::<f64>().unwrap(), 22.0);
//! ```
//!
//! Checked outer bounds enclose all four Euclid-mapped corners, not just a
//! diagonal, for arbitrary affine rotation, shear and reflection. They are not
//! an interior-containment test. Inputs must have finite matrix coefficients,
//! finite source origins/extents/endpoints and nonnegative source extents.
//! Positive source extents must advance their endpoints. Projected edges must
//! not overflow or completely underflow, and positive projected spans must not
//! collapse after mapping. Mapped corners and resulting bounds must be finite.
//! Zero-size rectangles and singular transforms are allowed. If reconstructing
//! a bound's maximum rounds inward, its extent advances one ULP to enclose that
//! maximum; unrepresentable extents/endpoints return evaluation errors.
//! The `adam` feature only adds optional type registration; it is disabled by default.

#[cfg(feature = "adam")]
mod adam;
mod geometry;
mod records;
mod transforms;

#[cfg(feature = "adam")]
pub use adam::register_adam_types;

/// A two-dimensional point with `f64` coordinates and unknown units.
///
/// # Examples
/// ```
/// assert_eq!(cel_euclid::Point2D::default(), cel_euclid::Point2D::new(0.0, 0.0));
/// ```
pub type Point2D = euclid::default::Point2D<f64>;
/// A two-dimensional vector with `f64` components and unknown units.
///
/// # Examples
/// ```
/// assert_eq!(cel_euclid::Vector2D::default(), cel_euclid::Vector2D::zero());
/// ```
pub type Vector2D = euclid::default::Vector2D<f64>;
/// A two-dimensional size with `f64` extents and unknown units.
///
/// # Examples
/// ```
/// assert_eq!(cel_euclid::Size2D::default(), cel_euclid::Size2D::zero());
/// ```
pub type Size2D = euclid::default::Size2D<f64>;
/// A rectangle represented by an origin and size in unknown units.
///
/// # Examples
/// ```
/// assert_eq!(cel_euclid::Rect::default().size, cel_euclid::Size2D::zero());
/// ```
pub type Rect = euclid::default::Rect<f64>;
/// An axis-aligned box represented by minimum and maximum points in unknown units.
///
/// # Examples
/// ```
/// assert_eq!(cel_euclid::Box2D::default().max, cel_euclid::Point2D::zero());
/// ```
pub type Box2D = euclid::default::Box2D<f64>;
/// An angle stored as `f64` radians.
///
/// # Examples
/// ```
/// assert_eq!(cel_euclid::Angle::default().radians, 0.0);
/// ```
pub type Angle = euclid::Angle<f64>;
/// A two-dimensional affine matrix mapping unknown units to unknown units.
///
/// # Examples
/// ```
/// assert_eq!(cel_euclid::Transform2D::default(), cel_euclid::Transform2D::identity());
/// ```
pub type Transform2D = euclid::default::Transform2D<f64>;

/// Installs seven record schemas and the complete exact geometry signature table.
///
/// - Precondition: this library has not already been installed on `lookup`.
/// - Postcondition: unrelated signatures, including scalar `min`/`max`, remain available.
/// - Complexity: O(s²) time and O(s) space in the total installed signatures.
///
/// # Examples
/// ```
/// let mut lookup = cel_parser::OpLookup::new();
/// cel_euclid::install(&mut lookup);
/// assert!(lookup.record_schema("Rect").is_some());
/// ```
pub fn install(lookup: &mut cel_parser::OpLookup) {
    debug_assert!(lookup.record_schema("Point2D").is_none());
    records::install(lookup).expect("cel-euclid record schemas must be installed exactly once");
    geometry::install(lookup)
        .expect("cel-euclid operation signatures must be installed exactly once");
    transforms::install(lookup)
        .expect("cel-euclid transform signatures must be installed exactly once");
}

/// Describes a concrete leaf without erasing its Rust identity.
fn leaf<T: 'static>(name: &'static str) -> cel_parser::ResolvedLeafType {
    cel_parser::ResolvedLeafType::new(
        name,
        cel_runtime::ArrayElementType::leaf::<T>().expect("Euclid types are non-array leaves"),
    )
}
