//! Public contracts for angle conversion and general affine geometry.

use cel_euclid::{Angle, Point2D, Rect, Size2D, Transform2D, Vector2D, install};
use cel_parser::{CELParser, OpLookup};
use std::fmt::Debug;

const P: &str = "Point2D { x: 1.0, y: 0.0 }";
const V: &str = "Vector2D { x: 10.0, y: 0.0 }";
const T: &str = "Transform2D { m11: 2.0, m12: 1.0, m21: 3.0, m22: 4.0, m31: 5.0, m32: 6.0 }";

/// Creates a fresh geometry parser with host-supplied test values.
fn parser(matrix: Transform2D, rect: Rect) -> CELParser {
    let mut lookup = OpLookup::new();
    install(&mut lookup);
    lookup.push_scope(move |name, segment, arity, _| {
        if arity != 0 {
            return Ok(false);
        }
        match name {
            "matrix" => segment.op0(move || matrix),
            "rect" => segment.op0(move || rect),
            _ => return Ok(false),
        }
        Ok(true)
    });
    CELParser::new(lookup)
}

/// Evaluates an expression against default host values.
fn evaluate<T: 'static>(source: &str) -> T {
    parser(Transform2D::identity(), Rect::zero())
        .parse_str(source)
        .unwrap_or_else(|e| panic!("{source}: {e}"))
        .call0::<T>()
        .unwrap()
}

/// Checks ordinary/receiver signatures and rejects every incorrectly typed operand.
///
/// - Complexity: O(n²) in the operand count.
fn named<T: 'static + PartialEq + Debug + Clone>(name: &str, args: &[&str], expected: T) {
    assert_eq!(
        evaluate::<T>(&format!("{name}({})", args.join(", "))),
        expected
    );
    assert_eq!(
        evaluate::<T>(&format!("({}).{name}({})", args[0], args[1..].join(", "))),
        expected
    );
    for index in 0..args.len() {
        let mut wrong = args.to_vec();
        wrong[index] = "true";
        for source in [
            format!("{name}({})", wrong.join(", ")),
            format!("({}).{name}({})", wrong[0], wrong[1..].join(", ")),
        ] {
            assert!(
                parser(Transform2D::identity(), Rect::zero())
                    .parse_str(&source)
                    .is_err(),
                "{source}"
            );
        }
    }
}

/// Checks coordinates with a scale-relative tolerance.
fn close(actual: Point2D, expected: Point2D) {
    for (a, b) in [(actual.x, expected.x), (actual.y, expected.y)] {
        assert!(
            (a - b).abs() <= 1e-12 * b.abs().max(1.0),
            "{actual:?} != {expected:?}"
        );
    }
}

/// Applying the left transform first makes composition order observable.
#[test]
fn transform_then_order_is_observable() {
    assert_eq!(
        evaluate::<Point2D>(&format!(
            "translation({V}).then(scale(2.0, 2.0)).transform_point({P})"
        )),
        Point2D::new(22.0, 0.0)
    );
    assert_eq!(
        evaluate::<Point2D>(&format!(
            "scale(2.0, 2.0).then(translation({V})).transform_point({P})"
        )),
        Point2D::new(12.0, 0.0)
    );
}

/// Translating to/from an arbitrary anchor preserves that anchor under rotation.
#[test]
fn rotation_preserves_anchor() {
    close(
        evaluate(
            "translation(Vector2D { x: -3.0, y: -7.0 }).then(rotation(degrees(37.0))).then(translation(Vector2D { x: 3.0, y: 7.0 })).transform_point(Point2D { x: 3.0, y: 7.0 })",
        ),
        Point2D::new(3.0, 7.0),
    );
}

/// Inverses round-trip points through nonsingular rotation, shear and reflection.
#[test]
fn inverse_round_trips_points() {
    for matrix in [
        Transform2D::rotation(Angle::degrees(37.0)),
        Transform2D::new(-2.0, 0.5, 1.25, 3.0, 17.0, -9.0),
        Transform2D::scale(1e-150, 1e-150),
        Transform2D::scale(1e150, 1e150),
    ] {
        let point = parser(matrix, Rect::zero())
            .parse_str(&format!(
                "matrix.inverse().transform_point(matrix.transform_point({P}))"
            ))
            .unwrap()
            .call0::<Point2D>()
            .unwrap();
        close(point, Point2D::new(1.0, 0.0));
    }
}

/// Singular matrices, invalid determinants and unrepresentable inverses report errors.
#[test]
fn singular_or_nonfinite_inverse_errors() {
    let mut matrices = vec![
        Transform2D::scale(0.0, 1.0),
        Transform2D::new(1.0, 2.0, 2.0, 4.0, 3.0, 4.0),
        Transform2D::scale(1e200, 1e200),
        Transform2D::new(1e200, 1e200, 1e200, 1e200, 0.0, 0.0),
        Transform2D::scale(1e-200, 1e-200),
        Transform2D::scale(f64::from_bits(1), 1.0),
        Transform2D::new(1e-150, 0.0, 0.0, 1e-150, 1e200, 0.0),
    ];
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for index in 0..6 {
            let mut fields = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
            fields[index] = value;
            matrices.push(Transform2D::new(
                fields[0], fields[1], fields[2], fields[3], fields[4], fields[5],
            ));
        }
    }
    for matrix in matrices {
        assert!(
            parser(matrix, Rect::zero())
                .parse_str("inverse(matrix)")
                .unwrap()
                .call0::<Transform2D>()
                .is_err(),
            "{matrix:?}"
        );
    }
}

/// Checked inversion rejects complete loss of any nonzero inverse coefficient.
#[test]
fn inverse_rejects_complete_coefficient_underflow() {
    let mut accepted = Vec::new();
    for matrix in [
        Transform2D::new(1e150, 1e-100, 0.0, 1e150, 0.0, 0.0),
        Transform2D::new(1e150, 0.0, 1e-100, 1e150, 0.0, 0.0),
        Transform2D::new(1.0, 1e150, -1e150, 1e-100, 0.0, 0.0),
        Transform2D::new(1e-100, 1e150, -1e150, 1.0, 0.0, 0.0),
        Transform2D::new(1e150, 0.0, 0.0, 1e150, 1e-250, 0.0),
        Transform2D::new(1e150, 0.0, 0.0, 1e150, 0.0, 1e-250),
        // Translation cofactors can lose their nonzero products before scaling.
        Transform2D::new(1.0, 0.0, 0.0, 1e-200, 1e-200, 0.0),
        Transform2D::new(1e-200, 0.0, 0.0, 1.0, 0.0, 1e-200),
        // Distinct exact products can also round to the same cofactor terms.
        Transform2D::new(2.0, 0.0, 1.0 + f64::EPSILON, 1.0, 1.0, 1.0 - f64::EPSILON),
        Transform2D::new(1.0, 1.0 + f64::EPSILON, 0.0, 2.0, 1.0 - f64::EPSILON, 1.0),
    ] {
        assert!(matrix.determinant().is_finite() && matrix.determinant() != 0.0);
        assert!(
            matrix
                .inverse()
                .unwrap()
                .to_array()
                .into_iter()
                .all(f64::is_finite)
        );
        for source in ["inverse(matrix)", "matrix.inverse()"] {
            if parser(matrix, Rect::zero())
                .parse_str(source)
                .unwrap()
                .call0::<Transform2D>()
                .is_ok()
            {
                accepted.push((source, matrix));
            }
        }
    }
    assert!(
        accepted.is_empty(),
        "accepted unrepresentable inverses: {accepted:?}"
    );
    assert!(
        parser(Transform2D::identity(), Rect::zero())
            .parse_str(
                "Transform2D { m11: 1e150, m12: 1e-100, m21: 0.0, m22: 1e150, m31: 0.0, m32: 0.0 }.inverse()"
            )
            .unwrap()
            .call0::<Transform2D>()
            .is_err()
    );
}

/// Exact-zero cofactors and nonzero subnormal coefficients retain Euclid results.
#[test]
fn inverse_preserves_exact_zeros_and_representable_subnormals() {
    let tiny = f64::from_bits(1);
    for matrix in [
        Transform2D::identity(),
        Transform2D::rotation(Angle::degrees(90.0)),
        Transform2D::new(1.0, 0.0, 2.0, 1.0, 6.0, 3.0),
        Transform2D::new(1.0, 0.0, -2.0, 1.0, -6.0, 3.0),
        Transform2D::new(1.0, 2.0, 0.0, 1.0, 3.0, 6.0),
        Transform2D::new(1.0, tiny, 0.0, 1.0, 0.0, 0.0),
        Transform2D::new(1.0, 0.0, tiny, 1.0, 0.0, 0.0),
        Transform2D::new(1.0, 0.0, 0.0, 1.0, tiny, -tiny),
        Transform2D::new(1.0, 0.0, tiny, 1.0, tiny, 1.0),
        Transform2D::scale(1e308, 1.0),
        Transform2D::scale(1.0, 1e308),
        // The two nonzero translation products cancel exactly despite underflow.
        Transform2D::new(2.0, 0.0, 1e-200, 1e-200, 1e-200, 1e-200),
    ] {
        let actual = parser(matrix, Rect::zero())
            .parse_str("matrix.inverse()")
            .unwrap()
            .call0::<Transform2D>()
            .unwrap();
        assert_eq!(actual, matrix.inverse().unwrap(), "{matrix:?}");
    }
    assert_eq!(
        evaluate::<Transform2D>(
            "Transform2D { m11: 1.0, m12: 5e-324, m21: 0.0, m22: 1.0, m31: 0.0, m32: 0.0 }.inverse()"
        ).m12,
        -tiny
    );
}

/// Every remaining ordinary signature matches Euclid and supports receiver syntax.
#[test]
fn angle_and_transform_signature_table() {
    named("degrees", &["90.0"], Angle::degrees(90.0));
    named("radians", &["1.25"], Angle::radians(1.25));
    named(
        "degrees",
        &["Angle { radians: 1.25 }"],
        Angle::radians(1.25).to_degrees(),
    );
    named("radians", &["Angle { radians: 1.25 }"], 1.25);
    assert_eq!(
        evaluate::<Transform2D>("identity_transform()"),
        Transform2D::identity()
    );
    named("translation", &[V], Transform2D::translation(10.0, 0.0));
    named("scale", &["2.0", "3.0"], Transform2D::scale(2.0, 3.0));
    named(
        "rotation",
        &["degrees(37.0)"],
        Transform2D::rotation(Angle::degrees(37.0)),
    );
    let matrix = Transform2D::new(2.0, 1.0, 3.0, 4.0, 5.0, 6.0);
    for (field, value) in [
        ("m11", 2.0),
        ("m12", 1.0),
        ("m21", 3.0),
        ("m22", 4.0),
        ("m31", 5.0),
        ("m32", 6.0),
    ] {
        named(field, &[T], value);
    }
    named(
        "then",
        &[T, "scale(2.0, 3.0)"],
        matrix.then(&Transform2D::scale(2.0, 3.0)),
    );
    named("determinant", &[T], 5.0);
    named("inverse", &[T], matrix.inverse().unwrap());
    named("transform_point", &[T, P], Point2D::new(7.0, 7.0));
    named("transform_vector", &[T, V], Vector2D::new(20.0, 10.0));
    named("is_finite", &[T], true);
    named(
        "outer_transformed_rect",
        &["identity_transform()", "rect"],
        Rect::zero(),
    );
    for source in [
        "identity_transform(true)",
        "scale(2, 3)",
        "translation(1.0, 2.0)",
        "rotation(1.0)",
        "inverse(1.0)",
        "transform_point(matrix, Vector2D { x: 1.0, y: 2.0 })",
    ] {
        assert!(
            parser(matrix, Rect::zero()).parse_str(source).is_err(),
            "{source}"
        );
    }
}

/// Inverse bounds enclose all four corners under rotation, reflection and shear.
#[test]
fn inverse_crop_bounds_enclose_all_corners() {
    let rect = Rect::new(Point2D::new(-7.0, 3.0), Size2D::new(13.0, 11.0));
    for matrix in [
        Transform2D::rotation(Angle::degrees(37.0)),
        Transform2D::new(-2.0, 0.5, 1.25, 3.0, 17.0, -9.0),
        Transform2D::new(1.0, 0.0, 2.0, 1.0, -3.0, 7.0),
    ] {
        let inverse = matrix.inverse().unwrap();
        let corners = [
            rect.origin,
            Point2D::new(rect.max_x(), rect.min_y()),
            Point2D::new(rect.min_x(), rect.max_y()),
            Point2D::new(rect.max_x(), rect.max_y()),
        ]
        .map(|point| inverse.transform_point(point));
        let min = Point2D::new(
            corners.iter().map(|p| p.x).fold(f64::INFINITY, f64::min),
            corners.iter().map(|p| p.y).fold(f64::INFINITY, f64::min),
        );
        let max = Point2D::new(
            corners
                .iter()
                .map(|p| p.x)
                .fold(f64::NEG_INFINITY, f64::max),
            corners
                .iter()
                .map(|p| p.y)
                .fold(f64::NEG_INFINITY, f64::max),
        );
        let bound = parser(matrix, rect)
            .parse_str("matrix.inverse().outer_transformed_rect(rect)")
            .unwrap()
            .call0::<Rect>()
            .unwrap();
        close(bound.origin, min);
        close(Point2D::new(bound.max_x(), bound.max_y()), max);
        for corner in corners {
            assert!(corner.x >= bound.min_x() && corner.x <= bound.max_x());
            assert!(corner.y >= bound.min_y() && corner.y <= bound.max_y());
        }
    }
}

/// Outer bounds include space outside a rotated image; they are not an interior test.
#[test]
fn transformed_image_bounds_are_not_an_interior_test() {
    let rect = Rect::new(Point2D::zero(), Size2D::new(100.0, 100.0));
    let matrix = Transform2D::rotation(Angle::degrees(45.0));
    let bound = parser(matrix, rect)
        .parse_str("matrix.outer_transformed_rect(rect)")
        .unwrap()
        .call0::<Rect>()
        .unwrap();
    let outside = bound.origin + Vector2D::new(1.0, 1.0);
    assert!(bound.contains(outside));
    assert!(!rect.contains(matrix.inverse().unwrap().transform_point(outside)));
}

/// Every stored scalar participates in its finite predicate, including matrix translation.
#[test]
fn finite_geometry_predicates_reject_nonfinite_components() {
    for bad in ["(0.0 / 0.0)", "(1.0 / 0.0)", "(-1.0 / 0.0)"] {
        for record in [
            "Point2D { x: 1.0, y: 2.0 }",
            "Vector2D { x: 1.0, y: 2.0 }",
            "Size2D { width: 1.0, height: 2.0 }",
            "Rect { origin: Point2D { x: 1.0, y: 2.0 }, size: Size2D { width: 3.0, height: 4.0 } }",
            "Box2D { min: Point2D { x: 1.0, y: 2.0 }, max: Point2D { x: 3.0, y: 4.0 } }",
            "Angle { radians: 1.0 }",
            T,
        ] {
            for value in ["1.0", "2.0", "3.0", "4.0", "5.0", "6.0"] {
                if record.contains(value) {
                    let invalid = record.replacen(value, bad, 1);
                    named("is_finite", &[&invalid], false);
                }
            }
        }
    }
}

/// Bounds reject invalid geometry, overflow, collapsed positive extents and underflow.
#[test]
fn checked_outer_bounds_errors() {
    let unit = Rect::new(Point2D::zero(), Size2D::new(1.0, 1.0));
    let identity = Transform2D::identity();
    let mut cases = vec![
        (identity, Rect::new(Point2D::zero(), Size2D::new(-1.0, 1.0))),
        (
            identity,
            Rect::new(Point2D::new(f64::MAX, 0.0), Size2D::new(f64::MAX, 1.0)),
        ),
        (
            identity,
            Rect::new(Point2D::new(1e100, 0.0), Size2D::new(1.0, 1.0)),
        ),
        (
            Transform2D::scale(f64::MAX, 1.0),
            Rect::new(Point2D::new(2.0, 0.0), Size2D::new(1.0, 1.0)),
        ),
        (Transform2D::translation(1e100, 0.0), unit),
        (
            Transform2D::scale(1e-200, 1.0),
            Rect::new(Point2D::zero(), Size2D::new(1e-200, 1.0)),
        ),
    ];
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for index in 0..6 {
            let mut fields = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
            fields[index] = bad;
            cases.push((
                Transform2D::new(
                    fields[0], fields[1], fields[2], fields[3], fields[4], fields[5],
                ),
                unit,
            ));
        }
        for index in 0..4 {
            let mut fields = [0.0, 0.0, 1.0, 1.0];
            fields[index] = bad;
            cases.push((
                identity,
                Rect::new(
                    Point2D::new(fields[0], fields[1]),
                    Size2D::new(fields[2], fields[3]),
                ),
            ));
        }
    }
    for (matrix, rect) in cases {
        let result = parser(matrix, rect)
            .parse_str("matrix.outer_transformed_rect(rect)")
            .unwrap()
            .call0::<Rect>();
        assert!(result.is_err(), "{matrix:?}, {rect:?}: {result:?}");
    }
}

/// Finite tiny geometry and degenerate affine images remain usable.
#[test]
fn outer_bounds_accept_tiny_and_degenerate_geometry() {
    for (matrix, rect, expected) in [
        (
            Transform2D::scale(1e-150, 1e-150),
            Rect::new(Point2D::zero(), Size2D::new(1e-150, 1e-150)),
            Rect::new(Point2D::zero(), Size2D::new(1e-300, 1e-300)),
        ),
        (
            Transform2D::scale(0.0, 1.0),
            Rect::new(Point2D::zero(), Size2D::new(1.0, 1.0)),
            Rect::new(Point2D::zero(), Size2D::new(0.0, 1.0)),
        ),
        (
            Transform2D::translation(3.0, 4.0),
            Rect::zero(),
            Rect::new(Point2D::new(3.0, 4.0), Size2D::zero()),
        ),
        (
            Transform2D::identity(),
            Rect::new(Point2D::zero(), Size2D::new(1e-310, 1e-310)),
            Rect::new(Point2D::zero(), Size2D::new(1e-310, 1e-310)),
        ),
        (
            Transform2D::scale(2.0, 3.0),
            Rect::new(Point2D::zero(), Size2D::new(1e150, 1e150)),
            Rect::new(Point2D::zero(), Size2D::new(1e150 * 2.0, 1e150 * 3.0)),
        ),
        (
            Transform2D::identity(),
            Rect::new(Point2D::new(f64::MAX, -f64::MAX), Size2D::zero()),
            Rect::new(Point2D::new(f64::MAX, -f64::MAX), Size2D::zero()),
        ),
    ] {
        let actual = parser(matrix, rect)
            .parse_str("matrix.outer_transformed_rect(rect)")
            .unwrap()
            .call0::<Rect>()
            .unwrap();
        assert_eq!(actual, expected);
    }
}

/// Ordinary conversion, composition and mapping preserve non-finite Euclid arithmetic.
#[test]
fn ordinary_transform_operations_do_not_gain_finite_checks() {
    assert!(!evaluate::<bool>("degrees(1.0 / 0.0).is_finite()"));
    assert!(!evaluate::<bool>("radians(0.0 / 0.0).is_finite()"));
    assert!(!evaluate::<bool>("scale(1.0 / 0.0, 1.0).is_finite()"));
    assert!(!evaluate::<bool>(&format!(
        "scale(1.0 / 0.0, 1.0).transform_point({P}).is_finite()"
    )));
    assert!(!evaluate::<bool>(&format!(
        "scale(1.0 / 0.0, 1.0).transform_vector({V}).is_finite()"
    )));
    assert!(!evaluate::<bool>(
        "scale(1.0 / 0.0, 1.0).then(identity_transform()).is_finite()"
    ));
    assert!(evaluate::<f64>("scale(1.0 / 0.0, 1.0).determinant()").is_infinite());
}

/// Reconstructing a bound's endpoint never rounds inward past a mapped corner.
#[test]
fn outer_bounds_round_outward_when_extent_subtraction_rounds_inward() {
    let rect = Rect::new(Point2D::zero(), Size2D::new(1.0, 1.0));
    let matrix = Transform2D::new(
        -2.8245766933675043,
        7.683856543964339,
        9.154624079279824,
        -6.981581884177821,
        -6.475645430192594,
        -5.360862663609285,
    );
    let bounds = parser(matrix, rect)
        .parse_str("outer_transformed_rect(matrix, rect)")
        .unwrap()
        .call0::<Rect>()
        .unwrap();
    for corner in [
        Point2D::new(0.0, 0.0),
        Point2D::new(1.0, 0.0),
        Point2D::new(0.0, 1.0),
        Point2D::new(1.0, 1.0),
    ] {
        let mapped = matrix.transform_point(corner);
        assert!(bounds.min_x() <= mapped.x && bounds.max_x() >= mapped.x);
        assert!(bounds.min_y() <= mapped.y && bounds.max_y() >= mapped.y);
    }
}

/// A finite pair of extrema must still produce a representable rectangle size.
#[test]
fn outer_bounds_reject_overflowing_extents_between_finite_corners() {
    let matrix = Transform2D::scale(2.0, 1.0);
    let rect = Rect::new(
        Point2D::new(-f64::MAX * 0.5, 0.0),
        Size2D::new(f64::MAX, 1.0),
    );
    assert!(matrix.transform_point(rect.origin).is_finite());
    assert!(
        matrix
            .transform_point(Point2D::new(rect.max_x(), rect.max_y()))
            .is_finite()
    );
    let error = parser(matrix, rect)
        .parse_str("outer_transformed_rect(matrix, rect)")
        .unwrap()
        .call0::<Rect>()
        .unwrap_err();
    assert!(
        error.to_string().contains("outer_transformed_rect"),
        "{error}"
    );
}
