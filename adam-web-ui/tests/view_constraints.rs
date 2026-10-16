//! Contracts for the graph's executable Adam view model.

use adam_web_ui::graph::{ViewAction, ViewConstraints, ViewInput, ViewTransform};
use cel_euclid::{Point2D, Rect, Size2D};

/// Constructs bounds from an origin and nonnegative extents.
fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect::new(Point2D::new(x, y), Size2D::new(width, height))
}

/// Constructs a request for the standard viewport.
fn input(bounds: Rect, action: ViewAction) -> ViewInput {
    ViewInput {
        bounds,
        viewport: Size2D::new(400.0, 300.0),
        action,
    }
}

/// Compares a complete transform to hand-derived coordinates.
fn assert_view(actual: ViewTransform, x: f64, y: f64, k: f64) {
    for (a, b) in [(actual.x, x), (actual.y, y), (actual.k, k)] {
        assert!((a - b).abs() < 1e-10, "{actual:?} != ({x}, {y}, {k})");
    }
}

/// Fit scales uniformly and centers both ordinary and translated bounds.
#[test]
fn fit_centers_content() {
    let mut model = ViewConstraints::new().unwrap();
    for (bounds, x, y) in [
        (rect(0.0, 0.0, 200.0, 100.0), 0.0, 50.0),
        (rect(-100.0, 20.0, 200.0, 100.0), 200.0, 10.0),
    ] {
        let out = model.update(input(bounds, ViewAction::Fit)).unwrap();
        assert_view(out.view, x, y, 2.0);
        assert_eq!(out.fit, out.view);
        assert_eq!(out.min_zoom, 2.0);
        assert_eq!(out.max_zoom, 8.0);
    }
}

/// Proposals clamp long axes and center short axes at the approved zoom.
#[test]
fn pan_centers_short_axis_and_clamps_long_axis() {
    let mut model = ViewConstraints::new().unwrap();
    for (x, y, k, want_x, want_y, want_k) in [
        (-999.0, 999.0, 3.0, -200.0, 0.0, 3.0),
        (999.0, -999.0, 3.0, 0.0, 0.0, 3.0),
        (-100.0, 999.0, 2.5, -100.0, 25.0, 2.5),
        (0.0, 0.0, 0.1, 0.0, 50.0, 2.0),
        (-9999.0, -9999.0, 100.0, -1200.0, -500.0, 8.0),
    ] {
        let out = model
            .update(input(
                rect(0.0, 0.0, 200.0, 100.0),
                ViewAction::Propose {
                    view: ViewTransform { x, y, k },
                },
            ))
            .unwrap();
        assert_view(out.view, want_x, want_y, want_k);
    }
}

/// The nominal maximum cannot exclude the fit scale.
#[test]
fn fit_can_exceed_nominal_zoom_limit() {
    let mut model = ViewConstraints::new().unwrap();
    let out = model
        .update(input(rect(0.0, 0.0, 10.0, 10.0), ViewAction::Fit))
        .unwrap();
    assert_view(out.view, 50.0, 0.0, 30.0);
    assert_eq!((out.min_zoom, out.max_zoom), (30.0, 30.0));
}

/// A point uses one-unit fit extents but centers its actual position.
#[test]
fn zero_extent_uses_one_unit() {
    let mut model = ViewConstraints::new().unwrap();
    let out = model
        .update(input(rect(2.0, -3.0, 0.0, 0.0), ViewAction::Fit))
        .unwrap();
    assert_view(out.view, -400.0, 1050.0, 300.0);
    assert_eq!((out.min_zoom, out.max_zoom), (300.0, 300.0));
}

/// Geometry-only refreshes retain pan and zoom aspirations across conformance.
#[test]
fn refresh_preserves_source_aspiration() {
    let mut model = ViewConstraints::new().unwrap();
    let large = rect(0.0, 0.0, 400.0, 200.0);
    let small = rect(0.0, 0.0, 200.0, 100.0);
    let proposal = ViewAction::Propose {
        view: ViewTransform {
            x: -700.0,
            y: -250.0,
            k: 3.0,
        },
    };
    assert_view(
        model.update(input(large, proposal)).unwrap().view,
        -700.0,
        -250.0,
        3.0,
    );
    assert_view(
        model
            .update(input(small, ViewAction::Refresh))
            .unwrap()
            .view,
        -200.0,
        0.0,
        3.0,
    );
    assert_view(
        model
            .update(input(large, ViewAction::Refresh))
            .unwrap()
            .view,
        -700.0,
        -250.0,
        3.0,
    );
    let tiny = rect(0.0, 0.0, 10.0, 10.0);
    assert_eq!(
        model
            .update(input(tiny, ViewAction::Refresh))
            .unwrap()
            .view
            .k,
        30.0
    );
    assert_view(
        model
            .update(input(large, ViewAction::Refresh))
            .unwrap()
            .view,
        -700.0,
        -250.0,
        3.0,
    );
}

/// Fit deliberately replaces both sources rather than merely displaying a reset.
#[test]
fn fit_replaces_source_aspiration() {
    let mut model = ViewConstraints::new().unwrap();
    let large = rect(0.0, 0.0, 400.0, 200.0);
    model
        .update(input(
            large,
            ViewAction::Propose {
                view: ViewTransform {
                    x: -700.0,
                    y: -250.0,
                    k: 3.0,
                },
            },
        ))
        .unwrap();
    model
        .update(input(rect(0.0, 0.0, 200.0, 100.0), ViewAction::Fit))
        .unwrap();
    assert_view(
        model
            .update(input(large, ViewAction::Refresh))
            .unwrap()
            .view,
        0.0,
        0.0,
        2.0,
    );
}

/// Invalid boundary values return useful errors instead of a fallback transform.
#[test]
fn invalid_view_input_is_reported() {
    let mut model = ViewConstraints::new().unwrap();
    for bounds in [
        rect(f64::NAN, 0.0, 1.0, 1.0),
        rect(0.0, f64::INFINITY, 1.0, 1.0),
        rect(0.0, 0.0, -1.0, 1.0),
        rect(0.0, 0.0, 1.0, f64::INFINITY),
        rect(f64::MAX, 0.0, f64::MAX, 1.0),
        rect(f64::MAX, 0.0, 1.0, 1.0),
    ] {
        let err = model
            .update(input(bounds, ViewAction::Refresh))
            .err()
            .unwrap();
        assert!(err.to_string().contains("bounds"), "{err}");
    }
    for value in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        for viewport in [Size2D::new(value, 300.0), Size2D::new(400.0, value)] {
            let mut request = input(rect(0.0, 0.0, 200.0, 100.0), ViewAction::Refresh);
            request.viewport = viewport;
            let err = model.update(request).err().unwrap();
            assert!(err.to_string().contains("viewport"), "{err}");
        }
        let err = model
            .update(input(
                rect(0.0, 0.0, 200.0, 100.0),
                ViewAction::Propose {
                    view: ViewTransform {
                        x: 0.0,
                        y: 0.0,
                        k: value,
                    },
                },
            ))
            .err()
            .unwrap();
        assert!(err.to_string().contains("zoom"), "{err}");
    }
    for (x, y) in [(f64::NAN, 0.0), (0.0, f64::INFINITY)] {
        assert!(
            model
                .update(input(
                    rect(0.0, 0.0, 200.0, 100.0),
                    ViewAction::Propose {
                        view: ViewTransform { x, y, k: 3.0 },
                    }
                ))
                .is_err()
        );
    }
    assert_view(
        model
            .update(input(rect(0.0, 0.0, 200.0, 100.0), ViewAction::Fit))
            .unwrap()
            .view,
        0.0,
        50.0,
        2.0,
    );
}

/// Finite inputs whose evaluation overflows or underflows fail explicitly.
#[test]
fn extreme_geometry_is_reported() {
    let mut model = ViewConstraints::new().unwrap();
    for request in [
        input(rect(f64::MAX * 0.5, 0.0, 0.0, 0.0), ViewAction::Fit),
        ViewInput {
            bounds: rect(0.0, 0.0, f64::MAX, f64::MAX),
            viewport: Size2D::new(f64::from_bits(1), f64::from_bits(1)),
            action: ViewAction::Fit,
        },
        ViewInput {
            bounds: rect(2.0, 2.0, 1.0, 1.0),
            viewport: Size2D::new(f64::MAX, f64::MAX),
            action: ViewAction::Fit,
        },
    ] {
        let err = model
            .update(request)
            .expect_err("unrepresentable output must fail");
        assert!(!err.to_string().is_empty());
    }
}
