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
//!
//! # Optional Adam installation
//!
//! Hosts explicitly install both cell types and operations. `adam-lang` itself
//! has no geometry dependency. Its registration-aware checker consumes the same
//! registry and lookup as compilation; unresolved call results remain best-effort.
//! Bare CEL hosts that use registered closure annotations must also configure
//! their [`cel_parser::TypeResolver`]; installing record operations alone does
//! not replace the parser's annotation resolver.
//!
//! ```
//! # #[cfg(feature = "adam")] {
//! let mut types = adam_lang::TypeRegistry::new();
//! cel_euclid::register_adam_types(&mut types);
//! let mut lookup = cel_parser::OpLookup::new();
//! cel_euclid::install(&mut lookup);
//! let mut parser = adam_lang::AdamParser::new(types, lookup);
//! assert!(parser.parse_str("sheet s { cell p: Point2D; }").is_ok());
//! # }
//! ```
//!
//! Run `cargo run -p cel-euclid --features adam --example adam_geometry` for
//! source-preserving crop/straighten and pan/zoom sheets. Crop fitting, its
//! eight-epsilon inward guard, and precision/coverage requirements live in CEL,
//! not in library operations. The example's Rust boundary validates inputs only.
//!
//! # Composing numeric image constraints
//!
//! With `adam`, the shipped component-only `image_constraints` module composes
//! two resampling axes and crop fitting through caller-owned ordinary cells.
//! Register the importing root too, and install both optional operation libraries
//! before creating the immutable compiler:
//!
//! ```
//! # #[cfg(feature = "adam")] {
//! let mut types = adam_lang::TypeRegistry::new();
//! cel_euclid::register_adam_types(&mut types);
//! let mut lookup = cel_parser::OpLookup::new();
//! cel_std::install(&mut lookup);
//! cel_euclid::install(&mut lookup);
//! let mut modules = adam_lang::ModuleSet::new();
//! modules.insert_source("image_constraints".parse().unwrap(), "image_constraints.adm2",
//!     include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/image_constraints.adm2"))).unwrap();
//! modules.insert_source("resize_straighten".parse().unwrap(), "resize_straighten.adm2",
//!     include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/resize_straighten.adm2"))).unwrap();
//! let compiled = adam_lang::ModuleCompiler::new(types, lookup).compile(modules).unwrap();
//! let mut parsed = compiled.instantiate_root(
//!     &"resize_straighten".parse().unwrap(), "resize_straighten").unwrap();
//! drop(compiled);
//! let crop = parsed.cell_names["crop"].0;
//! let requested_crop = *parsed.source::<cel_euclid::Rect>(crop).unwrap();
//! parsed.propagate().unwrap();
//! assert!(parsed.cell_requirements_valid(crop));
//! assert!(*parsed.read::<bool>(parsed.cell_names["coverage"].0).unwrap());
//! assert_eq!(*parsed.source::<cel_euclid::Rect>(crop).unwrap(), requested_crop);
//! # }
//! ```
//!
//! The caller owns pixel filters, source resolution/transform, positive image/crop
//! requirements and precision requirements. The component retains ties-even pixel
//! conversion and checked casts; it does not choose resample/aspect UI policy.
//! The crop fit retains the existing 8/32/64-epsilon guards. Its requested image
//! anchor must lie strictly inside the image. Invalid inverses/conversions return
//! evaluation errors; inadequate finite precision invalidates an explicit
//! requirement. A successful propagation alone does not certify a valid result.
//! Neither example changes the general Euclid operations or permits `out` actuals.

#![doc = include_str!("../SIGNATURES.md")]

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
/// - Precondition: `lookup` has no record schemas named `Point2D`, `Vector2D`,
///   `Size2D`, `Rect`, `Box2D`, `Angle`, or `Transform2D`.
/// - Precondition: `lookup` has no operation with the same name and operand
///   identities as a signature in the [supported table](#supported-geometry-signatures).
///   Other overloads of those names may already be installed.
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
    debug_assert!(
        [
            "Point2D",
            "Vector2D",
            "Size2D",
            "Rect",
            "Box2D",
            "Angle",
            "Transform2D"
        ]
        .iter()
        .all(|name| lookup.record_schema(name).is_none())
    );
    records::install(lookup)
        .expect("cel-euclid record names must not conflict with existing schemas");
    geometry::install(lookup)
        .expect("cel-euclid operation signatures must not conflict with existing signatures");
    transforms::install(lookup)
        .expect("cel-euclid transform signatures must not conflict with existing signatures");
}

/// Describes a concrete leaf without erasing its Rust identity.
fn leaf<T: 'static>(name: &'static str) -> cel_parser::ResolvedLeafType {
    cel_parser::ResolvedLeafType::new(name, cel_runtime::ValueType::leaf::<T>())
}
