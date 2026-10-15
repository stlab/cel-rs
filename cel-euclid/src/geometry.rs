//! Exact geometry signatures shared by ordinary and direct receiver dispatch.
//!
//! All arithmetic delegates to Euclid; checked normalization rejects unusable
//! lengths and results rather than modifying ordinary floating-point semantics.

use crate::{Angle, Box2D, Point2D, Rect, Size2D, Transform2D, Vector2D, leaf};
use anyhow::{Result, ensure};
use cel_parser::{OpLookup, OperationSignature};
use std::sync::Arc;

/// Declares one distinct thread-safe callable marker per identifier.
macro_rules! callables {
    ($($marker:ident => $name:literal),+ $(,)?) => {
        $(
            #[doc = concat!("Identifies the `", $name, "` callable independently of other libraries.")]
            #[derive(Clone)]
            struct $marker;
        )+
        /// Registers each geometry callable once, independently of its overload count.
        ///
        /// - Complexity: O(s) in existing registrations.
        ///
        /// # Errors
        /// Returns conflicting marker registration errors.
        fn register_callables(lookup: &mut OpLookup) -> Result<()> {
            $(lookup.register_function($name, $marker)?;)+
            Ok(())
        }
    };
}

callables! {
    IsFinite => "is_finite", X => "x", Y => "y", Width => "width", Height => "height",
    Origin => "origin", Size => "size", Min => "min", Max => "max",
    MinX => "min_x", MinY => "min_y", MaxX => "max_x", MaxY => "max_y", Center => "center",
    Dot => "dot", Cross => "cross", SquareLength => "square_length", Length => "length",
    Normalize => "normalize", DistanceTo => "distance_to", Translate => "translate",
    Inflate => "inflate", ToBox => "to_box2d", ToRect => "to_rect", Contains => "contains",
    ContainsRect => "contains_rect", ContainsBox => "contains_box",
    Intersects => "intersects", Union => "union",
}

/// Registers exact operand descriptors with a unary Euclid implementation.
///
/// - Complexity: O(s) in existing signatures.
///
/// # Errors
/// Returns errors for a duplicate operand signature.
pub(crate) fn unary<A: 'static, R: 'static>(
    lookup: &mut OpLookup,
    name: &'static str,
    op: fn(A) -> R,
) -> Result<()> {
    lookup.register_operation(OperationSignature {
        name,
        operands: vec![leaf::<A>(std::any::type_name::<A>())],
        result: leaf::<R>(std::any::type_name::<R>()),
        emit: Arc::new(move |s, _| s.op1(op)),
    })
}

/// Registers exact operand descriptors with a binary Euclid implementation.
///
/// - Complexity: O(s) in existing signatures.
///
/// # Errors
/// Returns errors for a duplicate operand signature.
pub(crate) fn binary<A: 'static, B: 'static, R: 'static>(
    lookup: &mut OpLookup,
    name: &'static str,
    op: fn(A, B) -> R,
) -> Result<()> {
    lookup.register_operation(OperationSignature {
        name,
        operands: vec![
            leaf::<A>(std::any::type_name::<A>()),
            leaf::<B>(std::any::type_name::<B>()),
        ],
        result: leaf::<R>(std::any::type_name::<R>()),
        emit: Arc::new(move |s, _| s.op2(op)),
    })
}

/// Registers exact operand descriptors with a ternary Euclid implementation.
///
/// - Complexity: O(s) in existing signatures.
///
/// # Errors
/// Returns errors for a duplicate operand signature.
fn ternary<A: 'static, B: 'static, C: 'static, R: 'static>(
    lookup: &mut OpLookup,
    name: &'static str,
    op: fn(A, B, C) -> R,
) -> Result<()> {
    lookup.register_operation(OperationSignature {
        name,
        operands: vec![
            leaf::<A>(std::any::type_name::<A>()),
            leaf::<B>(std::any::type_name::<B>()),
            leaf::<C>(std::any::type_name::<C>()),
        ],
        result: leaf::<R>(std::any::type_name::<R>()),
        emit: Arc::new(move |s, _| s.op3(op)),
    })
}

/// Returns a normalized vector only when its length and result are usable.
///
/// # Errors
/// Returns an error for zero/non-finite length or non-finite result components.
fn normalize(vector: Vector2D) -> Result<Vector2D> {
    let length = vector.length();
    ensure!(
        length > 0.0 && length.is_finite(),
        "normalize requires a positive finite length"
    );
    let result = vector / length;
    ensure!(result.is_finite(), "normalize requires a finite result");
    Ok(result)
}

/// Installs only the non-transform geometry table, plus all seven equality pairs.
///
/// - Complexity: O(s²) in existing and new signatures.
///
/// # Errors
/// Returns conflicting callable or duplicate signature registration errors.
pub(crate) fn install(lookup: &mut OpLookup) -> Result<()> {
    register_callables(lookup)?;
    macro_rules! equality {
        ($($ty:ty),+ $(,)?) => {$(
            binary(lookup, "==", |a: $ty, b: $ty| a == b)?;
            binary(lookup, "!=", |a: $ty, b: $ty| a != b)?;
        )+};
    }
    equality!(Point2D, Vector2D, Size2D, Rect, Box2D, Angle, Transform2D);
    unary(lookup, "is_finite", |a: Point2D| a.is_finite())?;
    unary(lookup, "is_finite", |a: Vector2D| a.is_finite())?;
    unary(lookup, "is_finite", |a: Size2D| a.is_finite())?;
    unary(lookup, "is_finite", |a: Rect| {
        a.origin.is_finite() && a.size.is_finite()
    })?;
    unary(lookup, "is_finite", |a: Box2D| {
        a.min.is_finite() && a.max.is_finite()
    })?;
    unary(lookup, "is_finite", |a: Angle| a.radians.is_finite())?;

    binary(lookup, "+", |a: Point2D, b: Vector2D| a + b)?;
    binary(lookup, "+", |a: Vector2D, b: Vector2D| a + b)?;
    binary(lookup, "+", |a: Size2D, b: Size2D| a + b)?;
    binary(lookup, "-", |a: Point2D, b: Point2D| a - b)?;
    binary(lookup, "-", |a: Point2D, b: Vector2D| a - b)?;
    binary(lookup, "-", |a: Vector2D, b: Vector2D| a - b)?;
    binary(lookup, "-", |a: Size2D, b: Size2D| a - b)?;
    unary(lookup, "-", |a: Vector2D| -a)?;
    binary(lookup, "*", |a: Vector2D, b: f64| a * b)?;
    binary(lookup, "/", |a: Vector2D, b: f64| a / b)?;
    binary(lookup, "*", |a: Size2D, b: f64| a * b)?;
    binary(lookup, "/", |a: Size2D, b: f64| a / b)?;

    unary(lookup, "x", |a: Point2D| a.x)?;
    unary(lookup, "y", |a: Point2D| a.y)?;
    unary(lookup, "x", |a: Vector2D| a.x)?;
    unary(lookup, "y", |a: Vector2D| a.y)?;
    unary(lookup, "width", |a: Size2D| a.width)?;
    unary(lookup, "height", |a: Size2D| a.height)?;
    unary(lookup, "origin", |a: Rect| a.origin)?;
    unary(lookup, "size", |a: Rect| a.size)?;
    unary(lookup, "size", |a: Box2D| a.size())?;
    unary(lookup, "min_x", |a: Rect| a.min_x())?;
    unary(lookup, "min_y", |a: Rect| a.min_y())?;
    unary(lookup, "max_x", |a: Rect| a.max_x())?;
    unary(lookup, "max_y", |a: Rect| a.max_y())?;
    unary(lookup, "min_x", |a: Box2D| a.min.x)?;
    unary(lookup, "min_y", |a: Box2D| a.min.y)?;
    unary(lookup, "max_x", |a: Box2D| a.max.x)?;
    unary(lookup, "max_y", |a: Box2D| a.max.y)?;
    unary(lookup, "min", |a: Box2D| a.min)?;
    unary(lookup, "max", |a: Box2D| a.max)?;
    macro_rules! bounds {
        ($ty:ty) => {
            unary(lookup, "width", |a: $ty| a.width())?;
            unary(lookup, "height", |a: $ty| a.height())?;
            unary(lookup, "center", |a: $ty| a.center())?;
            binary(lookup, "translate", |a: $ty, b: Vector2D| a.translate(b))?;
            ternary(lookup, "inflate", |a: $ty, x: f64, y: f64| a.inflate(x, y))?;
            binary(lookup, "contains", |a: $ty, b: Point2D| a.contains(b))?;
            binary(lookup, "intersects", |a: $ty, b: $ty| a.intersects(&b))?;
            binary(lookup, "union", |a: $ty, b: $ty| a.union(&b))?;
        };
    }
    bounds!(Rect);
    bounds!(Box2D);
    binary(lookup, "dot", |a: Vector2D, b: Vector2D| a.dot(b))?;
    binary(lookup, "cross", |a: Vector2D, b: Vector2D| a.cross(b))?;
    unary(lookup, "square_length", |a: Vector2D| a.square_length())?;
    unary(lookup, "length", |a: Vector2D| a.length())?;
    binary(lookup, "distance_to", |a: Point2D, b: Point2D| {
        a.distance_to(b)
    })?;
    unary(lookup, "to_box2d", |a: Rect| a.to_box2d())?;
    unary(lookup, "to_rect", |a: Box2D| a.to_rect())?;
    binary(lookup, "contains_rect", |a: Rect, b: Rect| {
        a.contains_rect(&b)
    })?;
    binary(lookup, "contains_box", |a: Box2D, b: Box2D| {
        a.contains_box(&b)
    })?;
    lookup.register_operation(OperationSignature {
        name: "normalize",
        operands: vec![leaf::<Vector2D>("Vector2D")],
        result: leaf::<Vector2D>("Vector2D"),
        emit: Arc::new(|s, _| s.op1r(normalize)),
    })?;
    Ok(())
}
