//! Contracts for the example's validation-only write boundary.
#![cfg(feature = "adam")]

#[path = "../examples/support/mod.rs"]
mod support;

use cel_euclid::{Point2D, Rect, Size2D, Transform2D};

/// A validation boundary accepts valid geometry without computing a fitted crop.
#[test]
fn boundary_accepts_valid_affine_candidates() {
    let boundary = support::InputBoundary::new().unwrap();
    let image = Rect::new(Point2D::zero(), Size2D::new(100.0, 80.0));
    let crop = Rect::new(Point2D::new(-20.0, 20.0), Size2D::new(80.0, 40.0));
    boundary
        .validate(image, crop, Transform2D::identity())
        .unwrap();
    boundary
        .validate(
            image,
            crop,
            Transform2D::new(-1.0, 0.0, 0.3, 1.0, 28.0, 0.0),
        )
        .unwrap();
}

/// Candidate validation reports bad geometry before a write changes source aspiration.
#[test]
fn boundary_rejects_invalid_and_unrepresentable_candidates() {
    let boundary = support::InputBoundary::new().unwrap();
    let image = Rect::new(Point2D::zero(), Size2D::new(100.0, 80.0));
    let crop = Rect::new(Point2D::new(10.0, 10.0), Size2D::new(20.0, 40.0));
    for (i, r, t) in [
        (image, crop, Transform2D::scale(0.0, 1.0)),
        (image, crop, Transform2D::scale(f64::INFINITY, 1.0)),
        (image, crop, Transform2D::translation(1e6, 0.0)),
        (
            image,
            Rect::new(Point2D::new(f64::NAN, 0.0), crop.size),
            Transform2D::identity(),
        ),
        (
            image,
            Rect::new(Point2D::zero(), Size2D::zero()),
            Transform2D::identity(),
        ),
        (
            Rect::new(Point2D::new(1e300, 1e300), Size2D::new(1.0, 1.0)),
            crop,
            Transform2D::identity(),
        ),
        (
            image,
            crop,
            Transform2D::new(1e308, 0.0, 0.0, 1e-308, 0.0, 0.0),
        ),
    ] {
        assert!(
            boundary.validate(i, r, t).is_err(),
            "accepted {i:?}, {r:?}, {t:?}"
        );
    }
}
