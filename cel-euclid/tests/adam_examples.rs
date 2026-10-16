//! Contract tests for the actual Adam geometry examples, without a crop scope.
#![cfg(feature = "adam")]

use adam_lang::{AdamParser, ParsedSheet, TypeRegistry};
use cel_euclid::{Angle, Point2D, Rect, Size2D, Transform2D, Vector2D};

/// Builds the explicitly opted-in environment.
fn parser() -> AdamParser {
    let mut types = TypeRegistry::new();
    cel_euclid::register_adam_types(&mut types);
    let mut lookup = cel_parser::OpLookup::new();
    cel_std::install(&mut lookup);
    cel_euclid::install(&mut lookup);
    AdamParser::new(types, lookup)
}

/// Parses the shipped crop sheet.
fn sheet() -> ParsedSheet {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join("crop_straighten.adm2"),
    )
    .expect("the actual crop example must exist");
    parser()
        .parse_str(&text)
        .unwrap_or_else(|error| panic!("{}", error.message()))
}

/// Constructs an axis-aligned rectangle.
fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect::new(Point2D::new(x, y), Size2D::new(w, h))
}

/// Constructs a rotation about a fixed image anchor.
fn rotation(degrees: f64, anchor: Point2D) -> Transform2D {
    Transform2D::translation(-anchor.x, -anchor.y)
        .then(&Transform2D::rotation(Angle::degrees(degrees)))
        .then(&Transform2D::translation(anchor.x, anchor.y))
}

/// Writes the candidate inputs and evaluates the sheet.
fn fit(sheet: &mut ParsedSheet, image: Rect, crop: Rect, transform: Transform2D) -> Rect {
    sheet
        .sheet
        .write(sheet.cell_names["image"].0, image)
        .unwrap();
    sheet
        .sheet
        .write(sheet.cell_names["transform"].0, transform)
        .unwrap();
    sheet.sheet.write(sheet.cell_names["crop"].0, crop).unwrap();
    sheet.propagate().unwrap();
    for (name, (id, _)) in &sheet.cell_names {
        assert!(sheet.cell_requirements_valid(*id), "invalid {name}");
    }
    *sheet.read(sheet.cell_names["crop"].0).unwrap()
}

/// Compares a scalar using the documented scale-relative tolerance.
fn close(a: f64, b: f64, scale: f64) {
    assert!((a - b).abs() <= 64.0 * f64::EPSILON * scale, "{a} != {b}");
}

/// Checks closed-edge inverse coverage, maximal uniform size, and center preservation.
fn contract(image: Rect, requested: Rect, transform: Transform2D, fitted: Rect) {
    let inv = transform.inverse().unwrap();
    let anchor = inv.transform_point(requested.center());
    let scale = image
        .size
        .width
        .max(image.size.height)
        .max(image.origin.x.abs())
        .max(image.origin.y.abs());
    for p in [
        fitted.origin,
        Point2D::new(fitted.max_x(), fitted.min_y()),
        Point2D::new(fitted.min_x(), fitted.max_y()),
        Point2D::new(fitted.max_x(), fitted.max_y()),
    ] {
        let p = inv.transform_point(p);
        let tol = 64.0 * f64::EPSILON * scale;
        assert!(
            p.x >= image.min_x() - tol && p.x <= image.max_x() + tol,
            "{p:?}"
        );
        assert!(
            p.y >= image.min_y() - tol && p.y <= image.max_y() + tol,
            "{p:?}"
        );
    }
    // Independent affine support-function oracle; no outer-bound implementation reuse.
    let ex = (inv.m11.abs() * requested.width() + inv.m21.abs() * requested.height()) * 0.5;
    let ey = (inv.m12.abs() * requested.width() + inv.m22.abs() * requested.height()) * 0.5;
    let expected = 1.0_f64
        .min((anchor.x - image.min_x()).min(image.max_x() - anchor.x) / ex)
        .min((anchor.y - image.min_y()).min(image.max_y() - anchor.y) / ey);
    close(fitted.width() / requested.width(), expected, 1.0);
    close(fitted.height() / requested.height(), expected, 1.0);
    close(fitted.center().x, requested.center().x, scale);
    close(fitted.center().y, requested.center().y, scale);
}

/// Loosening a rotation restores the user's original source crop.
#[test]
fn self_referencing_crop_springs_back() {
    let mut s = sheet();
    let crop = s.cell_names["crop"].0;
    let transform = s.cell_names["transform"].0;
    s.write(transform, rotation(45.0, Point2D::new(50.0, 50.0)))
        .unwrap();
    s.propagate().unwrap();
    close(
        s.read::<Rect>(crop).unwrap().width(),
        100.0 / 2.0_f64.sqrt(),
        100.0,
    );
    s.write(transform, Transform2D::identity()).unwrap();
    s.propagate().unwrap();
    assert_eq!(*s.read::<Rect>(crop).unwrap(), rect(0.0, 0.0, 100.0, 100.0));
    assert_eq!(
        *s.sheet.source::<Rect>(crop).unwrap(),
        rect(0.0, 0.0, 100.0, 100.0)
    );
}

/// Explicit crop edits replace the source aspiration rather than the conformed value.
#[test]
fn explicit_crop_edit_replaces_aspiration() {
    let mut s = sheet();
    let crop = s.cell_names["crop"].0;
    let transform = s.cell_names["transform"].0;
    s.write(transform, rotation(45.0, Point2D::new(50.0, 50.0)))
        .unwrap();
    s.propagate().unwrap();
    let edited = rect(10.0, 10.0, 80.0, 80.0);
    s.write(crop, edited).unwrap();
    s.propagate().unwrap();
    s.write(transform, Transform2D::identity()).unwrap();
    s.propagate().unwrap();
    assert_eq!(*s.read::<Rect>(crop).unwrap(), edited);
    assert_eq!(*s.sheet.source::<Rect>(crop).unwrap(), edited);
}

/// Presentation maps the fitted rectangle onto a fixed overlay and fixes its image anchor.
#[test]
fn presentation_keeps_overlay_and_image_anchor_fixed() {
    let mut s = sheet();
    let image = rect(0.0, 0.0, 100.0, 80.0);
    let requested = rect(-20.0, 20.0, 80.0, 40.0);
    let anchor = requested.center();
    let t = rotation(37.0, anchor);
    let fitted = fit(&mut s, image, requested, t);
    let presentation: Transform2D = *s.read(s.cell_names["presentation"].0).unwrap();
    let inv = t.inverse().unwrap();
    let source_anchor = inv.transform_point(requested.center());
    close(
        presentation.transform_point(source_anchor).x,
        requested.center().x,
        100.0,
    );
    close(
        presentation.transform_point(source_anchor).y,
        requested.center().y,
        100.0,
    );
    let displayed = presentation.transform_point(inv.transform_point(fitted.origin));
    close(displayed.x, requested.origin.x, 100.0);
    close(displayed.y, requested.origin.y, 100.0);
    assert_eq!(
        *s.sheet.source::<Rect>(s.cell_names["crop"].0).unwrap(),
        requested
    );
}

/// Finite reciprocal-scale cancellation fails a visible precision requirement.
#[test]
fn presentation_rejects_insufficient_precision_at_extreme_size_ratio() {
    let mut s = sheet();
    let image = rect(0.03333283332557231, 0.03333283332557231, 1e-6, 1e-6);
    let requested = rect(-499999.9, -499999.9, 1e6, 1e6);
    let t = Transform2D::scale(3.0, 3.0);
    s.sheet.write(s.cell_names["image"].0, image).unwrap();
    s.sheet.write(s.cell_names["crop"].0, requested).unwrap();
    s.sheet.write(s.cell_names["transform"].0, t).unwrap();
    s.propagate().unwrap();
    let p = *s
        .read::<Transform2D>(s.cell_names["presentation"].0)
        .unwrap();
    let anchor = t.inverse().unwrap().transform_point(requested.center());
    let error = p.transform_point(anchor) - requested.center();
    assert!(
        [p.m11, p.m12, p.m21, p.m22, p.m31, p.m32]
            .into_iter()
            .all(f64::is_finite)
    );
    assert!(error.x.abs() > 64.0 * f64::EPSILON * requested.width());
    assert!(
        !s.cell_requirements_valid(s.cell_names["precise"].0),
        "accepted finite presentation with anchor error {error:?}"
    );
    let requirement = s
        .violated_requirements(s.cell_names["precise"].0)
        .next()
        .unwrap();
    assert_eq!(
        s.requirement_names[&requirement],
        "sufficient_fit_and_presentation_precision"
    );
    assert_eq!(
        *s.sheet.source::<Rect>(s.cell_names["crop"].0).unwrap(),
        requested
    );
}

/// A fixed anchor alone cannot certify all four overlay corners.
#[test]
fn presentation_rejects_overlay_cancellation_even_with_fixed_anchor() {
    let mut s = sheet();
    let image = rect(-1.0000005, -1.0000005, 1e-6, 1e-6);
    let requested = rect(-5e5, -5e5, 1e6, 1e6);
    let t = Transform2D::new(3.0, 0.0, 0.0, 3.0, 3.0, 3.0);
    s.sheet.write(s.cell_names["image"].0, image).unwrap();
    s.sheet.write(s.cell_names["crop"].0, requested).unwrap();
    s.sheet.write(s.cell_names["transform"].0, t).unwrap();
    s.propagate().unwrap();
    let fitted = *s.read::<Rect>(s.cell_names["crop"].0).unwrap();
    let p = *s
        .read::<Transform2D>(s.cell_names["presentation"].0)
        .unwrap();
    let inv = t.inverse().unwrap();
    assert_eq!(
        p.transform_point(inv.transform_point(requested.center())),
        requested.center()
    );
    let displayed = p.transform_point(inv.transform_point(fitted.origin));
    assert!((displayed.x - requested.min_x()).abs() > 64.0 * f64::EPSILON * requested.width());
    assert!(!s.cell_requirements_valid(s.cell_names["precise"].0));
    assert_eq!(
        *s.sheet.source::<Rect>(s.cell_names["crop"].0).unwrap(),
        requested
    );
}

/// Representable extreme shrink fixes the center and all four overlay corners.
#[test]
fn presentation_maps_extreme_size_ratio_when_representable() {
    let mut s = sheet();
    let requested = rect(-524288.0, -262144.0, 1048576.0, 524288.0);
    let image = rect(-1e-6, -5e-7, 2e-6, 1e-6);
    let t = Transform2D::new(-3.0, 0.25, 0.5, 2.0, 0.0, 0.0);
    let fitted = fit(&mut s, image, requested, t);
    let p = *s
        .read::<Transform2D>(s.cell_names["presentation"].0)
        .unwrap();
    let inv = t.inverse().unwrap();
    let extent = requested.width().max(requested.height());
    for (effective, overlay) in [
        (fitted.center(), requested.center()),
        (fitted.origin, requested.origin),
        (
            Point2D::new(fitted.max_x(), fitted.min_y()),
            Point2D::new(requested.max_x(), requested.min_y()),
        ),
        (
            Point2D::new(fitted.min_x(), fitted.max_y()),
            Point2D::new(requested.min_x(), requested.max_y()),
        ),
        (
            Point2D::new(fitted.max_x(), fitted.max_y()),
            Point2D::new(requested.max_x(), requested.max_y()),
        ),
    ] {
        let displayed = p.transform_point(inv.transform_point(effective));
        close(displayed.x, overlay.x, extent);
        close(displayed.y, overlay.y, extent);
    }
    assert_eq!(
        *s.sheet.source::<Rect>(s.cell_names["crop"].0).unwrap(),
        requested
    );
}

/// General standard and Euclid operations alone execute the actual fixture.
#[test]
fn crop_fitting_uses_only_general_registered_operations() {
    let mut s = sheet();
    s.propagate().unwrap();
    assert!(!s.cell_names.contains_key("requested_crop"));
    assert_eq!(*s.read::<f64>(s.cell_names["shrink"].0).unwrap(), 1.0);
}

/// An already covered crop remains bit-for-bit unchanged, including touching edges.
#[test]
fn already_fitting_crop_is_unchanged() {
    let mut s = sheet();
    for crop in [rect(0.0, 0.0, 100.0, 100.0), rect(5.0, 7.0, 20.0, 30.0)] {
        assert_eq!(
            fit(
                &mut s,
                rect(0.0, 0.0, 100.0, 100.0),
                crop,
                Transform2D::identity()
            ),
            crop
        );
    }
}

/// A centered square rotated 45 degrees shrinks uniformly by one over root two.
#[test]
fn forty_five_degree_crop_scales_uniformly() {
    let mut s = sheet();
    let image = rect(0.0, 0.0, 100.0, 100.0);
    let t = rotation(45.0, image.center());
    let fitted = fit(&mut s, image, image, t);
    assert!((fitted.width() - 100.0 / 2.0_f64.sqrt()).abs() < 1e-12);
    close(fitted.height(), fitted.width(), 100.0);
    contract(image, image, t, fitted);
}

/// The nearer image edge limits an off-center crop without translating its center.
#[test]
fn off_center_anchor_uses_nearest_edge() {
    let mut s = sheet();
    let fitted = fit(
        &mut s,
        rect(0.0, 0.0, 100.0, 80.0),
        rect(-20.0, 20.0, 80.0, 40.0),
        Transform2D::identity(),
    );
    close(fitted.origin.x, 0.0, 100.0);
    close(fitted.origin.y, 30.0, 100.0);
    close(fitted.width(), 40.0, 100.0);
    close(fitted.height(), 20.0, 100.0);
}

/// Inverse bounds use actual affine edges, including reflections and unequal aspect ratios.
#[test]
fn sheared_image_uses_actual_edges() {
    let mut s = sheet();
    let image = rect(0.0, 0.0, 100.0, 80.0);
    for linear in [
        Transform2D::new(1.0, 0.3, 0.8, 1.0, 0.0, 0.0),
        Transform2D::new(-1.0, 0.2, 0.4, 1.2, 0.0, 0.0),
        Transform2D::rotation(Angle::degrees(90.0)),
    ] {
        let t = Transform2D::translation(-20.0, -40.0)
            .then(&linear)
            .then(&Transform2D::translation(20.0, 40.0));
        let r = rect(-20.0, 20.0, 80.0, 40.0);
        let fitted = fit(&mut s, image, r, t);
        contract(image, r, t, fitted);
    }
}

/// Reapplying the expression to its effective output stays within the documented tolerance.
#[test]
fn crop_expression_is_idempotent() {
    let mut s = sheet();
    let image = rect(0.0, 0.0, 100.0, 100.0);
    for t in [
        rotation(45.0, image.center()),
        Transform2D::new(-1.0, 0.3, 0.4, 1.5, 65.0, -40.0),
    ] {
        let fitted = fit(&mut s, image, image, t);
        let mut repeated = fitted;
        for _ in 0..16 {
            repeated = fit(&mut s, image, repeated, t);
            close(repeated.width(), fitted.width(), 100.0);
            close(repeated.height(), fitted.height(), 100.0);
            close(repeated.origin.x, fitted.origin.x, 100.0);
            close(repeated.origin.y, fitted.origin.y, 100.0);
        }
    }
}

/// Invalid, infeasible, nonfinite, underflowed, and unrepresentable geometry is reported.
#[test]
fn invalid_crop_inputs_are_reported() {
    for (image, crop, t) in [
        (
            rect(0.0, 0.0, -1.0, 10.0),
            rect(0.0, 0.0, 1.0, 1.0),
            Transform2D::identity(),
        ),
        (
            rect(0.0, 0.0, 100.0, 100.0),
            rect(0.0, 0.0, 0.0, 10.0),
            Transform2D::identity(),
        ),
        (
            rect(0.0, 0.0, 100.0, 100.0),
            rect(100.0, 0.0, 20.0, 20.0),
            Transform2D::identity(),
        ),
        (
            rect(0.0, 0.0, 100.0, 100.0),
            rect(f64::NAN, 0.0, 10.0, 10.0),
            Transform2D::identity(),
        ),
        (
            rect(0.0, 0.0, 100.0, 100.0),
            rect(0.0, 0.0, 10.0, 10.0),
            Transform2D::scale(0.0, 1.0),
        ),
        (
            rect(0.0, 0.0, 100.0, 100.0),
            rect(0.0, 0.0, 10.0, 10.0),
            Transform2D::translation(f64::INFINITY, 0.0),
        ),
        (
            rect(-1e-310, -1e-310, 2e-310, 2e-310),
            rect(-1e100, -1e100, 2e100, 2e100),
            Transform2D::identity(),
        ),
        (
            rect(-1e-310, -1e-310, 2e-310, 2e-310),
            rect(-1.0, -1.0, 2.0, 2.0),
            Transform2D::identity(),
        ),
        (
            rect(1e300, 1e300, 1.0, 1.0),
            rect(1e300, 1e300, 1.0, 1.0),
            Transform2D::identity(),
        ),
        (
            rect(f64::MAX, 0.0, f64::MAX, 100.0),
            rect(0.0, 0.0, 10.0, 10.0),
            Transform2D::identity(),
        ),
        // A nonzero subnormal width can round to the wrong aspect ratio.
        (
            rect(-1e-321, -0.6, 2e-321, 1.2),
            rect(-2e-323, -2.0, 4e-323, 4.0),
            Transform2D::identity(),
        ),
        // Advancing endpoints alone do not guarantee sufficient relative precision.
        (
            rect(1e16, 1e16, 100.0, 100.0),
            rect(1e16 - 50.0, 1e16 - 50.0, 201.0, 201.0),
            Transform2D::identity(),
        ),
    ] {
        let mut s = sheet();
        s.sheet.write(s.cell_names["image"].0, image).unwrap();
        s.sheet.write(s.cell_names["crop"].0, crop).unwrap();
        s.sheet.write(s.cell_names["transform"].0, t).unwrap();
        let reported = s.propagate().is_err()
            || s.cell_names
                .values()
                .any(|(id, _)| !s.cell_requirements_valid(*id));
        assert!(reported, "accepted {image:?}, {crop:?}, {t:?}");
    }
}

/// Arbitrary affine translation, nonuniform scale, shear, and reflection share one contract.
#[test]
fn affine_frames_need_not_fix_the_crop_center() {
    let mut s = sheet();
    let image = rect(-10.0, 20.0, 100.0, 80.0);
    for linear in [
        Transform2D::new(1.7, 0.4, -0.2, 0.8, 230.0, -170.0),
        Transform2D::new(-0.7, 0.3, 0.8, 2.0, -50.0, 310.0),
    ] {
        for a in [Point2D::new(10.0, 60.0), Point2D::new(70.0, 40.0)] {
            let c = linear.transform_point(a);
            let r = rect(c.x - 90.0, c.y - 30.0, 180.0, 60.0);
            let fitted = fit(&mut s, image, r, linear);
            contract(image, r, linear, fitted);
            let p: Transform2D = *s.read(s.cell_names["presentation"].0).unwrap();
            close(p.transform_point(a).x, c.x, 400.0);
            close(p.transform_point(a).y, c.y, 400.0);
        }
    }
}

/// Finite large and small representable geometry obeys the same affine contract.
#[test]
fn finite_extremes_keep_coverage_and_maximality() {
    let mut s = sheet();
    for unit in [1e-150, 1e150] {
        let image = rect(0.0, 0.0, 100.0 * unit, 80.0 * unit);
        let r = rect(-20.0 * unit, 20.0 * unit, 80.0 * unit, 40.0 * unit);
        let t = rotation(32.0, r.center());
        let fitted = fit(&mut s, image, r, t);
        contract(image, r, t, fitted);
    }
}

/// Pan and zoom use fit-scale limits and center or clamp each axis independently.
#[test]
fn pan_zoom_sheet_demonstrates_graph_policy() {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join("pan_zoom.adm2"),
    )
    .expect("the pan/zoom example must exist");
    let mut s = parser().parse_str(&text).unwrap();
    s.propagate().unwrap();
    assert_eq!(*s.read::<f64>(s.cell_names["fit_scale"].0).unwrap(), 2.0);
    s.sheet.write(s.cell_names["zoom"].0, 100.0).unwrap();
    s.sheet
        .write(s.cell_names["pan"].0, Vector2D::new(1000.0, -1000.0))
        .unwrap();
    s.propagate().unwrap();
    assert_eq!(*s.read::<f64>(s.cell_names["zoom"].0).unwrap(), 8.0);
    assert_eq!(
        *s.read::<Vector2D>(s.cell_names["pan"].0).unwrap(),
        Vector2D::new(0.0, -600.0)
    );
    let content = s.cell_names["content"].0;
    let zoom = s.cell_names["zoom"].0;
    let pan = s.cell_names["pan"].0;
    s.write(content, rect(10.0, -20.0, 10.0, 100.0)).unwrap();
    s.write(zoom, 4.0).unwrap();
    s.write(pan, Vector2D::new(1000.0, -1000.0)).unwrap();
    s.propagate().unwrap();
    assert_eq!(
        *s.read::<Vector2D>(pan).unwrap(),
        Vector2D::new(40.0, -120.0)
    );
    s.write(content, rect(0.0, 0.0, 0.25, 0.5)).unwrap();
    s.propagate().unwrap();
    assert_eq!(*s.read::<f64>(zoom).unwrap(), 200.0);
    s.write(content, Rect::zero()).unwrap();
    s.propagate().unwrap();
    assert_eq!(*s.read::<f64>(s.cell_names["fit_scale"].0).unwrap(), 1.0);
}

/// Translated horizontal bounds remain valid through proposals, Fit, and refresh.
#[test]
fn pan_zoom_translated_horizontal_fit_rounding() {
    translated_pan_zoom_contract(false);
}

/// Translated vertical bounds remain valid through proposals, Fit, and refresh.
#[test]
fn pan_zoom_translated_vertical_fit_rounding() {
    translated_pan_zoom_contract(true);
}

/// Executes the actual example with transposed geometry and retained source aspirations.
fn translated_pan_zoom_contract(transpose: bool) {
    let text = include_str!("../examples/pan_zoom.adm2");
    let mut s = parser().parse_str(text).unwrap();
    let (content, viewport, proposed_pan) = if transpose {
        (
            rect(0.0, 437.7687406100192, 100.0, 317.7044969997028),
            Size2D::new(712.0, 696.0),
            Vector2D::new(156.0, -4.0 * 437.7687406100192),
        )
    } else {
        (
            rect(437.7687406100192, 0.0, 317.7044969997028, 100.0),
            Size2D::new(696.0, 712.0),
            Vector2D::new(-4.0 * 437.7687406100192, 156.0),
        )
    };
    let content_id = s.cell_names["content"].0;
    let viewport_id = s.cell_names["viewport"].0;
    let zoom = s.cell_names["zoom"].0;
    let pan = s.cell_names["pan"].0;
    s.write(content_id, content).unwrap();
    s.write(viewport_id, viewport).unwrap();
    // Default zoom 2 conforms to fit without inverted clamp bounds.
    s.propagate().unwrap();
    let fit_scale = *s.read::<f64>(s.cell_names["fit_scale"].0).unwrap();
    let fit_pan = *s.read::<Vector2D>(s.cell_names["fit_pan"].0).unwrap();
    close(fit_scale, 2.19071497751148, 3.0);
    assert_eq!(*s.read::<f64>(zoom).unwrap(), fit_scale);
    assert_eq!(*s.read::<Vector2D>(pan).unwrap(), fit_pan);

    // Explicit proposals clamp the effective view, not the source aspiration.
    let aspiration = Vector2D::new(10000.0, 10000.0);
    s.write(zoom, 4.0).unwrap();
    s.write(pan, aspiration).unwrap();
    s.propagate().unwrap();
    assert_eq!(*s.read::<f64>(zoom).unwrap(), 4.0);
    assert_eq!(*s.read::<Vector2D>(pan).unwrap(), proposed_pan);
    s.write(viewport_id, viewport * 2.0).unwrap();
    s.propagate().unwrap();
    assert_eq!(*s.read::<f64>(zoom).unwrap(), fit_scale * 2.0);
    assert_eq!(*s.sheet.source::<f64>(zoom).unwrap(), 4.0);
    assert_eq!(*s.sheet.source::<Vector2D>(pan).unwrap(), aspiration);
    s.write(viewport_id, viewport).unwrap();
    s.propagate().unwrap();
    assert_eq!(*s.read::<f64>(zoom).unwrap(), 4.0);
    assert_eq!(*s.read::<Vector2D>(pan).unwrap(), proposed_pan);

    // Fit/reset replaces those aspirations using the sheet's own outputs.
    s.write(zoom, fit_scale).unwrap();
    s.write(pan, fit_pan).unwrap();
    s.propagate().unwrap();
    assert_eq!(*s.read::<f64>(zoom).unwrap(), fit_scale);
    assert_eq!(*s.read::<Vector2D>(pan).unwrap(), fit_pan);
    assert_eq!(*s.sheet.source::<f64>(zoom).unwrap(), fit_scale);
    assert_eq!(*s.sheet.source::<Vector2D>(pan).unwrap(), fit_pan);
    let view = *s.read::<Transform2D>(s.cell_names["view"].0).unwrap();
    let center = view.transform_point(content.center());
    close(center.x, viewport.width * 0.5, viewport.width);
    close(center.y, viewport.height * 0.5, viewport.height);
    assert!(
        s.cell_names
            .values()
            .all(|(id, _)| s.cell_requirements_valid(*id))
    );
}
