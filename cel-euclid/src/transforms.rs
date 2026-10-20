//! Angle conversion and affine operations for unknown-unit `f64` geometry.
//!
//! `a.then(b)` applies `a` first, then `b`, following Euclid's row-vector
//! convention. Ordinary operations preserve Euclid floating-point arithmetic.
//! Checked inverse and outer bounds return evaluation errors instead of NaNs,
//! infinities or unrepresentable geometry. Bounds enclose all four mapped corners;
//! an axis-aligned outer bound is not a test for a transformed shape's interior.

use crate::{
    Angle, Box2D, Point2D, Rect, Size2D, Transform2D, Vector2D,
    geometry::{binary, unary},
    leaf,
};
use anyhow::{Result, anyhow, ensure};
use cel_parser::{OpLookup, OperationSignature};
use std::sync::Arc;

/// Declares independent callable identities and registers their names.
macro_rules! callables {
    ($($marker:ident => $name:literal),+ $(,)?) => {
        $(
            #[doc = concat!("Identifies the `", $name, "` callable.")]
            #[derive(Clone)]
            struct $marker;
        )+
        /// Registers affine callable names once, regardless of overload count.
        ///
        /// - Complexity: O(s) in existing registrations.
        ///
        /// # Errors
        /// Returns conflicting callable registration errors.
        fn register_callables(lookup: &mut OpLookup) -> Result<()> {
            $(lookup.register_function($name, $marker)?;)+
            Ok(())
        }
    };
}

callables! {
    Degrees => "degrees", Radians => "radians",
    M11 => "m11", M12 => "m12", M21 => "m21", M22 => "m22", M31 => "m31", M32 => "m32",
    Identity => "identity_transform", Translation => "translation", Scale => "scale",
    Rotation => "rotation", Then => "then", Determinant => "determinant",
    Inverse => "inverse", TransformPoint => "transform_point",
    TransformVector => "transform_vector", OuterRect => "outer_transformed_rect",
}

/// Returns whether all six affine coefficients are finite.
fn is_finite(matrix: Transform2D) -> bool {
    matrix.to_array().into_iter().all(f64::is_finite)
}

/// Encodes an exact product of finite binary64 values without rounding.
///
/// - Precondition: Both operands are finite.
/// - Postcondition: Equal products have equal encodings, including signed zeros.
///
/// The tuple contains a negative-sign flag, an odd significand and an exponent
/// offset by 2150; zero has the canonical encoding `(false, 0, 0)`. Two 53-bit
/// significands fit exactly in `u128`, even when their floating product underflows.
fn exact_product(a: f64, b: f64) -> (bool, u128, u32) {
    debug_assert!(a.is_finite() && b.is_finite());
    let parts = |value: f64| {
        let bits = value.to_bits();
        let exponent = ((bits >> 52) & 0x7ff) as u32;
        let significand =
            u128::from(bits & ((1_u64 << 52) - 1)) | (u128::from(exponent != 0) << 52);
        (significand, exponent.max(1))
    };
    let (a_significand, a_exponent) = parts(a);
    let (b_significand, b_exponent) = parts(b);
    let product = a_significand * b_significand;
    if product == 0 {
        return (false, 0, 0);
    }
    let shift = product.trailing_zeros();
    (
        a.is_sign_negative() != b.is_sign_negative(),
        product >> shift,
        a_exponent + b_exponent + shift,
    )
}

/// Returns Euclid's inverse with finite coefficients and no lost nonzero cofactors.
///
/// A zero inverse coefficient is allowed only when its exact cofactor is zero.
/// Nonzero subnormal results are allowed. Other rounding and conditioning errors
/// remain Euclid's floating-point semantics; this is not exact-real inversion.
///
/// # Errors
/// Returns an error for non-finite coefficients, a zero/non-finite determinant,
/// non-finite inverse coefficients, or a zero inverse coefficient whose exact
/// cofactor is nonzero (including loss inside translation cofactor arithmetic).
fn inverse(matrix: Transform2D) -> Result<Transform2D> {
    ensure!(is_finite(matrix), "inverse requires finite coefficients");
    let determinant = matrix.determinant();
    ensure!(
        determinant != 0.0 && determinant.is_finite(),
        "inverse requires a nonzero finite determinant"
    );
    let result = matrix
        .inverse()
        .ok_or_else(|| anyhow!("inverse requires a nonsingular matrix"))?;
    ensure!(
        is_finite(result),
        "inverse requires finite inverse coefficients"
    );
    ensure!(
        [
            (matrix.m22, result.m11),
            (matrix.m12, result.m12),
            (matrix.m21, result.m21),
            (matrix.m11, result.m22),
        ]
        .into_iter()
        .all(|(cofactor, coefficient)| coefficient != 0.0 || cofactor == 0.0)
            && (result.m31 != 0.0
                || exact_product(matrix.m21, matrix.m32) == exact_product(matrix.m22, matrix.m31))
            && (result.m32 != 0.0
                || exact_product(matrix.m31, matrix.m12) == exact_product(matrix.m11, matrix.m32)),
        "inverse requires representable inverse coefficients"
    );
    Ok(result)
}

/// Returns the magnitude of one projected nonnegative edge extent.
///
/// - Precondition: `coefficient` is finite and `extent` is finite and nonnegative.
///
/// # Errors
/// Returns an error for overflow or complete underflow of a nonzero edge.
fn projected_edge(coefficient: f64, extent: f64) -> Result<f64> {
    debug_assert!(coefficient.is_finite() && extent.is_finite() && extent >= 0.0);
    let result = coefficient.abs() * extent;
    ensure!(
        result.is_finite() && (result != 0.0 || coefficient == 0.0 || extent == 0.0),
        "outer_transformed_rect requires representable projected edges"
    );
    Ok(result)
}

/// Returns a finite extent whose reconstructed endpoint encloses `max`.
///
/// - Precondition: `min` and `max` are finite and `min <= max`.
/// - Postcondition: `min + result` is finite and at least `max`.
///
/// # Errors
/// Returns an error when the extent or enclosing endpoint cannot be represented.
fn enclosing_extent(min: f64, max: f64) -> Result<f64> {
    debug_assert!(min.is_finite() && max.is_finite() && min <= max);
    let mut extent = max - min;
    if min + extent < max {
        extent = extent.next_up();
    }
    let endpoint = min + extent;
    ensure!(
        extent.is_finite() && endpoint.is_finite() && endpoint >= max,
        "outer_transformed_rect requires representable bounds"
    );
    Ok(extent)
}

/// Returns an axis-aligned rectangle enclosing all four Euclid-mapped corners.
///
/// Zero extents and singular transforms are allowed. Positive source extents
/// must advance their endpoints, and nonzero projected edges must not underflow.
/// A positive projected span must not collapse after mapping. Each output extent
/// is rounded upward by one ULP if necessary to enclose its mapped maximum.
/// This general outer bound does not establish containment in the image interior.
///
/// # Errors
/// Returns an error for non-finite coefficients, source components/endpoints,
/// mapped corners or output bounds; negative source extents; and unrepresentable
/// source endpoints, projected edges or output extents.
fn outer_transformed_rect(matrix: Transform2D, rect: Rect) -> Result<Rect> {
    ensure!(
        is_finite(matrix) && rect.origin.is_finite() && rect.size.is_finite(),
        "outer_transformed_rect requires finite inputs"
    );
    ensure!(
        rect.size.width >= 0.0 && rect.size.height >= 0.0,
        "outer_transformed_rect requires nonnegative source extents"
    );
    let max = Point2D::new(rect.max_x(), rect.max_y());
    ensure!(
        max.is_finite()
            && (rect.size.width == 0.0 || max.x > rect.origin.x)
            && (rect.size.height == 0.0 || max.y > rect.origin.y),
        "outer_transformed_rect requires representable source endpoints"
    );
    let span_x = projected_edge(matrix.m11, rect.size.width)?
        + projected_edge(matrix.m21, rect.size.height)?;
    let span_y = projected_edge(matrix.m12, rect.size.width)?
        + projected_edge(matrix.m22, rect.size.height)?;
    ensure!(
        span_x.is_finite() && span_y.is_finite(),
        "outer_transformed_rect requires finite projected spans"
    );
    let corners = [
        rect.origin,
        Point2D::new(max.x, rect.origin.y),
        Point2D::new(rect.origin.x, max.y),
        max,
    ]
    .map(|point| matrix.transform_point(point));
    ensure!(
        corners.iter().all(|point| point.is_finite()),
        "outer_transformed_rect requires finite mapped corners"
    );
    let bounds = Box2D::from_points(corners);
    let width = enclosing_extent(bounds.min.x, bounds.max.x)?;
    let height = enclosing_extent(bounds.min.y, bounds.max.y)?;
    ensure!(
        (span_x == 0.0 || width > 0.0) && (span_y == 0.0 || height > 0.0),
        "outer_transformed_rect requires noncollapsed projected spans"
    );
    Ok(Rect::new(bounds.min, Size2D::new(width, height)))
}

/// Installs exactly the angle and affine signature table.
///
/// - Precondition: geometry callables, including `is_finite`, are installed.
/// - Complexity: O(s²) in existing and new signatures.
///
/// # Errors
/// Returns conflicting callable or duplicate signature registration errors.
pub(crate) fn install(lookup: &mut OpLookup) -> Result<()> {
    register_callables(lookup)?;
    unary(lookup, "degrees", Angle::degrees)?;
    unary(lookup, "radians", Angle::radians)?;
    unary(lookup, "degrees", |angle: Angle| angle.to_degrees())?;
    unary(lookup, "radians", |angle: Angle| angle.radians)?;
    unary(lookup, "m11", |matrix: Transform2D| matrix.m11)?;
    unary(lookup, "m12", |matrix: Transform2D| matrix.m12)?;
    unary(lookup, "m21", |matrix: Transform2D| matrix.m21)?;
    unary(lookup, "m22", |matrix: Transform2D| matrix.m22)?;
    unary(lookup, "m31", |matrix: Transform2D| matrix.m31)?;
    unary(lookup, "m32", |matrix: Transform2D| matrix.m32)?;
    unary(lookup, "is_finite", is_finite)?;
    unary(lookup, "translation", |vector: Vector2D| {
        Transform2D::translation(vector.x, vector.y)
    })?;
    binary(lookup, "scale", Transform2D::scale)?;
    unary(lookup, "rotation", Transform2D::rotation)?;
    binary(lookup, "then", |a: Transform2D, b: Transform2D| a.then(&b))?;
    unary(lookup, "determinant", |matrix: Transform2D| {
        matrix.determinant()
    })?;
    binary(
        lookup,
        "transform_point",
        |matrix: Transform2D, point: Point2D| matrix.transform_point(point),
    )?;
    binary(
        lookup,
        "transform_vector",
        |matrix: Transform2D, vector: Vector2D| matrix.transform_vector(vector),
    )?;
    lookup.register_operation(OperationSignature {
        name: "identity_transform",
        operands: vec![],
        result: leaf::<Transform2D>("Transform2D").into(),
        emit: Arc::new(|segment, _| {
            segment.op0(Transform2D::identity);
            Ok(())
        }),
    })?;
    lookup.register_operation(OperationSignature {
        name: "inverse",
        operands: vec![leaf::<Transform2D>("Transform2D").into()],
        result: leaf::<Transform2D>("Transform2D").into(),
        emit: Arc::new(|segment, _| segment.op1r(inverse)),
    })?;
    lookup.register_operation(OperationSignature {
        name: "outer_transformed_rect",
        operands: vec![
            leaf::<Transform2D>("Transform2D").into(),
            leaf::<Rect>("Rect").into(),
        ],
        result: leaf::<Rect>("Rect").into(),
        emit: Arc::new(|segment, _| segment.op2r(outer_transformed_rect)),
    })?;
    Ok(())
}
