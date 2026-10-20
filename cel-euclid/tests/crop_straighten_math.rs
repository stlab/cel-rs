//! Checks center-preserving crop rotation and fitting with Euclid alone.

use euclid::{
    Angle,
    default::{Point2D, Rect, Size2D, Transform2D},
};

/// Returns the largest scale in `[0, 1]` fitting the corners about `center`.
///
/// - Precondition: Coordinates and intermediate arithmetic are finite.
/// - Precondition: `image` has positive dimensions and contains `center`.
/// - Precondition: `corners` describe the rotated crop centered at `center`.
fn fitting_scale(image: Rect<f64>, center: Point2D<f64>, corners: [Point2D<f64>; 4]) -> f64 {
    debug_assert!(image.width() > 0.0 && image.height() > 0.0);
    debug_assert!(image.min_x() <= center.x && center.x <= image.max_x());
    debug_assert!(image.min_y() <= center.y && center.y <= image.max_y());
    corners
        .into_iter()
        .flat_map(|corner| {
            [
                (corner.x, center.x, image.min_x(), image.max_x()),
                (corner.y, center.y, image.min_y(), image.max_y()),
            ]
        })
        .map(|(component, center, min, max)| {
            if component > max {
                (max - center) / (component - center)
            } else if component < min {
                (center - min) / (center - component)
            } else {
                1.0
            }
        })
        .fold(1.0, f64::min)
}

/// Corner ratios preserve containment and give the largest scale no greater than one.
#[test]
fn rotated_crop_corner_ratios_give_maximal_fitting_scale() {
    let square: Rect<f64> = Rect::new(Point2D::new(10.0, 20.0), Size2D::new(100.0, 100.0));
    let inv_sqrt_2 = std::f64::consts::FRAC_1_SQRT_2;
    // Hand-derived scales at 45 and 90 degrees, respectively.
    for (image, crop, scale_45, scale_90) in [
        (square, square, inv_sqrt_2, 1.0),
        (
            square,
            Rect::new(Point2D::new(40.0, 50.0), Size2D::new(40.0, 40.0)),
            1.0,
            1.0,
        ),
        (
            square,
            Rect::new(Point2D::new(10.0, 60.0), Size2D::new(20.0, 20.0)),
            inv_sqrt_2,
            1.0,
        ),
        (
            square,
            Rect::new(Point2D::new(90.0, 60.0), Size2D::new(20.0, 20.0)),
            inv_sqrt_2,
            1.0,
        ),
        (
            square,
            Rect::new(Point2D::new(50.0, 20.0), Size2D::new(20.0, 20.0)),
            inv_sqrt_2,
            1.0,
        ),
        (
            square,
            Rect::new(Point2D::new(50.0, 100.0), Size2D::new(20.0, 20.0)),
            inv_sqrt_2,
            1.0,
        ),
        (
            Rect::new(Point2D::new(-30.0, -20.0), Size2D::new(100.0, 60.0)),
            Rect::new(Point2D::new(-20.0, 0.0), Size2D::new(80.0, 20.0)),
            0.6 * std::f64::consts::SQRT_2,
            0.75,
        ),
        (
            square,
            Rect::new(Point2D::new(10.0, 20.0), Size2D::new(80.0, 20.0)),
            0.2 * std::f64::consts::SQRT_2,
            0.25,
        ),
    ] {
        assert!(image.contains_rect(&crop));
        let center = crop.center();
        let corners = [
            crop.origin,
            Point2D::new(crop.max_x(), crop.min_y()),
            Point2D::new(crop.min_x(), crop.max_y()),
            Point2D::new(crop.max_x(), crop.max_y()),
        ];
        let epsilon = 64.0
            * f64::EPSILON
            * image
                .min_x()
                .abs()
                .max(image.max_x().abs())
                .max(image.min_y().abs())
                .max(image.max_y().abs())
                .max(image.width())
                .max(image.height());
        for degrees in (-180_i32..=180).step_by(5) {
            let rotation = Transform2D::rotation(Angle::degrees(f64::from(degrees)));
            let rotated = corners.map(|p| center + rotation.transform_vector(p - center));
            let scale = fitting_scale(image, center, rotated);
            assert!((0.0..=1.0).contains(&scale), "{scale}");
            let expected = match degrees.abs() {
                0 | 180 => Some(1.0),
                45 => Some(scale_45),
                90 => Some(scale_90),
                _ => None,
            };
            if let Some(expected) = expected {
                assert!(
                    (scale - expected).abs() <= 64.0 * f64::EPSILON,
                    "{crop:?}, {degrees} degrees: {scale} != {expected}"
                );
            }
            let mut edge_distance = f64::INFINITY;
            for point in rotated.map(|p| center + (p - center) * scale) {
                // Closed-edge containment; Euclid point containment is half-open.
                for distance in [
                    point.x - image.min_x(),
                    image.max_x() - point.x,
                    point.y - image.min_y(),
                    image.max_y() - point.y,
                ] {
                    assert!(
                        distance >= -epsilon,
                        "{crop:?}, {degrees} degrees: {point:?} outside {image:?}"
                    );
                    edge_distance = edge_distance.min(distance.abs());
                }
            }
            assert!(
                scale == 1.0 || edge_distance <= epsilon,
                "{crop:?}, {degrees} degrees: shrink stops short of every edge"
            );
        }
    }
}
