//! Exercises authored numeric and geometry components through explicit module roots.
#![cfg(feature = "adam")]

use adam_lang::{ModuleCompiler, ModuleSet, ParsedSheet, TypeRegistry};
use cel_euclid::{Angle, Point2D, Rect, Size2D, Transform2D};

/// Constructs an explicitly selected root with the installed standard and geometry libraries.
fn root(name: &str) -> ParsedSheet {
    let mut modules = ModuleSet::new();
    for (path, file, text) in [
        (
            "relationships",
            "relationships.adm2",
            include_str!("../../adam-lang/examples/relationships.adm2"),
        ),
        (
            "composed",
            "composed.adm2",
            include_str!("../../adam-lang/examples/composed.adm2"),
        ),
        (
            "image_constraints",
            "image_constraints.adm2",
            include_str!("../examples/image_constraints.adm2"),
        ),
        (
            "resize_straighten",
            "resize_straighten.adm2",
            include_str!("../examples/resize_straighten.adm2"),
        ),
    ] {
        modules
            .insert_source(path.parse().unwrap(), file, text)
            .unwrap();
    }
    let mut types = TypeRegistry::new();
    cel_euclid::register_adam_types(&mut types);
    let mut lookup = cel_parser::OpLookup::new();
    cel_std::install(&mut lookup);
    cel_euclid::install(&mut lookup);
    let compiled = ModuleCompiler::new(types, lookup).compile(modules).unwrap();
    let module = if name == "resize_straighten" {
        "resize_straighten"
    } else {
        "composed"
    };
    compiled
        .instantiate_root(&module.parse().unwrap(), name)
        .unwrap()
}

/// Detects reversed bounds or replacement of a caller's requested endpoint.
#[test]
fn ordered_bounds_preserve_aspirations() {
    let mut parsed = root("ordered_demo");
    let low = parsed.cell_names["low"].0;
    let high = parsed.cell_names["high"].0;
    parsed.propagate().unwrap();
    parsed.sheet.write(low, 20.0f64).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(low).unwrap(), 20.0);
    assert_eq!(*parsed.read::<f64>(high).unwrap(), 20.0);
    assert_eq!(*parsed.source::<f64>(high).unwrap(), 10.0);
    parsed.sheet.write(low, 0.0f64).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(high).unwrap(), 10.0);
    parsed.sheet.write(high, -10.0f64).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(low).unwrap(), -10.0);
    assert_eq!(*parsed.source::<f64>(low).unwrap(), 0.0);
    parsed.sheet.write(high, 10.0f64).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(low).unwrap(), 0.0);
}

/// Detects lost source specialization or a missing reverse multiplication direction.
#[test]
fn source_bound_factor_specializes_to_two_directions() {
    let mut parsed = root("source_specialized");
    let value = parsed.cell_names["value"].0;
    let product = parsed.cell_names["product"].0;
    let factor = parsed.cell_names["factor"].0;
    parsed.sheet.write(value, 7.0f64).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(product).unwrap(), 21.0);
    parsed.sheet.write(product, 12.0f64).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(value).unwrap(), 4.0);
    assert_eq!(*parsed.read::<f64>(factor).unwrap(), 3.0);
    for (input, expected) in [(-2.0, -6.0), (0.0, 0.0)] {
        parsed.sheet.write(value, input).unwrap();
        parsed.propagate().unwrap();
        assert_eq!(*parsed.read::<f64>(product).unwrap(), expected);
    }
}

/// Detects accidental sharing between uses and loss of any floating multiplication method.
#[test]
fn repeated_three_way_multiplication_has_distinct_actuals() {
    let mut parsed = root("repeated_products");
    for (a, b, c, x, y, z) in [
        ("a", "b", "c", 2.0, 3.0, 6.0),
        ("d", "e", "f", 4.0, 5.0, 20.0),
    ] {
        let a = parsed.cell_names[a].0;
        let b = parsed.cell_names[b].0;
        let c = parsed.cell_names[c].0;
        parsed.sheet.write(a, x).unwrap();
        parsed.sheet.write(b, y).unwrap();
        parsed.propagate().unwrap();
        assert_eq!(*parsed.read::<f64>(c).unwrap(), z);
        parsed.sheet.write(c, z * 2.0).unwrap();
        parsed.propagate().unwrap();
        assert_eq!(*parsed.read::<f64>(a).unwrap(), x * 2.0);
        parsed.sheet.write(a, x).unwrap();
        parsed.propagate().unwrap();
        assert_eq!(*parsed.read::<f64>(b).unwrap(), y * 2.0);
    }
    assert_eq!(*parsed.read::<f64>(parsed.cell_names["c"].0).unwrap(), 12.0);
    assert_eq!(*parsed.read::<f64>(parsed.cell_names["f"].0).unwrap(), 40.0);
}

/// Checks closed-edge coverage and the original overlay within the existing 64-epsilon budget.
fn assert_crop_contract(parsed: &ParsedSheet, requested_crop: Rect) {
    let crop = parsed.cell_names["crop"].0;
    let coverage = parsed.cell_names["coverage"].0;
    assert!(parsed.cell_requirements_valid(crop));
    assert!(*parsed.read::<bool>(coverage).unwrap());
    assert_eq!(*parsed.source::<Rect>(crop).unwrap(), requested_crop);
    for (id, _) in parsed.cell_names.values() {
        assert!(parsed.cell_requirements_valid(*id));
    }
    assert!(*parsed.read::<bool>(parsed.cell_names["precise"].0).unwrap());
    let fitted = *parsed.read::<Rect>(crop).unwrap();
    let image = *parsed.read::<Rect>(parsed.cell_names["image"].0).unwrap();
    let transform = *parsed
        .read::<Transform2D>(parsed.cell_names["transform"].0)
        .unwrap();
    let presentation = *parsed
        .read::<Transform2D>(parsed.cell_names["presentation"].0)
        .unwrap();
    let inverse = transform.inverse().unwrap();
    let tolerance = 64.0 * f64::EPSILON * requested_crop.width().max(requested_crop.height());
    let coverage_tolerance = 64.0
        * f64::EPSILON
        * image
            .width()
            .max(image.height())
            .max(image.min_x().abs())
            .max(image.max_x().abs())
            .max(image.min_y().abs())
            .max(image.max_y().abs());
    let anchor = inverse.transform_point(requested_crop.center());
    // Independent affine support extents bound the maximal center-preserving uniform fit.
    let ex = (inverse.m11.abs() * requested_crop.width()
        + inverse.m21.abs() * requested_crop.height())
        * 0.5;
    let ey = (inverse.m12.abs() * requested_crop.width()
        + inverse.m22.abs() * requested_crop.height())
        * 0.5;
    let expected_scale = 1.0f64
        .min((anchor.x - image.min_x()).min(image.max_x() - anchor.x) / ex)
        .min((anchor.y - image.min_y()).min(image.max_y() - anchor.y) / ey);
    assert!((fitted.width() - requested_crop.width() * expected_scale).abs() <= tolerance);
    assert!((fitted.height() - requested_crop.height() * expected_scale).abs() <= tolerance);
    for (point, overlay) in [
        (fitted.origin, requested_crop.origin),
        (
            Point2D::new(fitted.max_x(), fitted.min_y()),
            Point2D::new(requested_crop.max_x(), requested_crop.min_y()),
        ),
        (
            Point2D::new(fitted.min_x(), fitted.max_y()),
            Point2D::new(requested_crop.min_x(), requested_crop.max_y()),
        ),
        (
            Point2D::new(fitted.max_x(), fitted.max_y()),
            Point2D::new(requested_crop.max_x(), requested_crop.max_y()),
        ),
    ] {
        let p = inverse.transform_point(point);
        assert!(
            p.x >= image.min_x() - coverage_tolerance && p.x <= image.max_x() + coverage_tolerance
        );
        assert!(
            p.y >= image.min_y() - coverage_tolerance && p.y <= image.max_y() + coverage_tolerance
        );
        let p = presentation.transform_point(p);
        assert!((p.x - overlay.x).abs() <= tolerance);
        assert!((p.y - overlay.y).abs() <= tolerance);
    }
    assert!((fitted.center().x - requested_crop.center().x).abs() <= tolerance);
    assert!((fitted.center().y - requested_crop.center().y).abs() <= tolerance);
}

/// Detects an absent width-to-height constraint or overwritten edited/crop aspirations.
#[test]
fn aspect_width_edit_drives_height_and_preserves_sources() {
    let mut parsed = root("resize_straighten");
    let width = parsed.cell_names["width_inches"].0;
    let height = parsed.cell_names["height_inches"].0;
    let requested_crop = Rect::new(Point2D::new(0.0, 0.0), Size2D::new(1600.0, 1200.0));
    parsed.sheet.write(width, 4.0f64).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(height).unwrap(), 3.0);
    assert_eq!(
        *parsed
            .read::<i32>(parsed.cell_names["width_pixels"].0)
            .unwrap(),
        1200
    );
    assert_eq!(
        *parsed
            .read::<i32>(parsed.cell_names["height_pixels"].0)
            .unwrap(),
        900
    );
    assert_eq!(*parsed.source::<f64>(width).unwrap(), 4.0);
    assert_eq!(
        *parsed
            .source::<f64>(parsed.cell_names["width_per_height"].0)
            .unwrap(),
        1600.0 / 1200.0
    );
    assert_crop_contract(&parsed, requested_crop);
}

/// Detects an absent height-to-width constraint or overwritten edited/crop aspirations.
#[test]
fn aspect_height_edit_drives_width_and_preserves_sources() {
    let mut parsed = root("resize_straighten");
    let width = parsed.cell_names["width_inches"].0;
    let height = parsed.cell_names["height_inches"].0;
    let requested_crop = Rect::new(Point2D::new(0.0, 0.0), Size2D::new(1600.0, 1200.0));
    parsed.sheet.write(height, 3.0f64).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(width).unwrap(), 4.0);
    assert_eq!(
        *parsed
            .read::<i32>(parsed.cell_names["width_pixels"].0)
            .unwrap(),
        1200
    );
    assert_eq!(
        *parsed
            .read::<i32>(parsed.cell_names["height_pixels"].0)
            .unwrap(),
        900
    );
    assert_eq!(*parsed.source::<f64>(height).unwrap(), 3.0);
    assert_eq!(
        *parsed
            .source::<f64>(parsed.cell_names["width_per_height"].0)
            .unwrap(),
        1600.0 / 1200.0
    );
    assert_crop_contract(&parsed, requested_crop);
}

/// Detects a hard-coded aspect instead of a reusable caller-supplied ratio.
#[test]
fn aspect_uses_the_callers_ratio_source() {
    let mut parsed = root("resize_straighten");
    let width = parsed.cell_names["width_inches"].0;
    let height = parsed.cell_names["height_inches"].0;
    let ratio = parsed.cell_names["width_per_height"].0;
    let requested_crop = Rect::new(Point2D::new(0.0, 0.0), Size2D::new(1600.0, 1200.0));
    parsed.sheet.write(ratio, 2.0f64).unwrap();
    parsed.sheet.write(width, 6.0f64).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(height).unwrap(), 3.0);
    assert_eq!(*parsed.source::<f64>(width).unwrap(), 6.0);
    assert_eq!(*parsed.source::<f64>(ratio).unwrap(), 2.0);
    assert_crop_contract(&parsed, requested_crop);
}

/// Detects lost aspect propagation or crop aspirations across resize, rotation and reversal.
#[test]
fn resize_rotation_and_reversal_preserve_requested_crop() {
    let mut parsed = root("resize_straighten");
    let crop = parsed.cell_names["crop"].0;
    let requested_crop = Rect::new(Point2D::new(0.0, 0.0), Size2D::new(1600.0, 1200.0));
    parsed.propagate().unwrap();
    assert_crop_contract(&parsed, requested_crop);
    parsed
        .sheet
        .write(parsed.cell_names["width_inches"].0, 4.0f64)
        .unwrap();
    parsed.propagate().unwrap();
    assert_eq!(
        *parsed
            .read::<i32>(parsed.cell_names["width_pixels"].0)
            .unwrap(),
        1200
    );
    assert_eq!(
        *parsed
            .read::<i32>(parsed.cell_names["height_pixels"].0)
            .unwrap(),
        900
    );
    assert!(parsed.read::<Rect>(crop).unwrap().width() < requested_crop.width());
    assert_crop_contract(&parsed, requested_crop);
    let rotation = Transform2D::translation(-800.0, -600.0)
        .then(&Transform2D::rotation(Angle::degrees(30.0)))
        .then(&Transform2D::translation(800.0, 600.0));
    parsed
        .sheet
        .write(parsed.cell_names["transform"].0, rotation)
        .unwrap();
    parsed.propagate().unwrap();
    assert_crop_contract(&parsed, requested_crop);
    parsed
        .sheet
        .write(parsed.cell_names["transform"].0, Transform2D::identity())
        .unwrap();
    parsed
        .sheet
        .write(parsed.cell_names["height_inches"].0, 4.0f64)
        .unwrap();
    parsed.propagate().unwrap();
    assert_crop_contract(&parsed, requested_crop);
    assert_eq!(*parsed.read::<Rect>(crop).unwrap(), requested_crop);
}

/// Detects unchecked inverse failures hidden by the composed component.
#[test]
fn singular_transform_reports_checked_failure() {
    let mut parsed = root("resize_straighten");
    parsed
        .sheet
        .write(
            parsed.cell_names["transform"].0,
            Transform2D::scale(0.0, 1.0),
        )
        .unwrap();
    assert!(parsed.propagate().is_err());
}

/// Detects unchecked float-to-pixel conversion after composing the resampling axis.
#[test]
fn unrepresentable_pixel_count_reports_checked_failure() {
    let mut parsed = root("resize_straighten");
    parsed
        .sheet
        .write(parsed.cell_names["width_inches"].0, 1.0e20f64)
        .unwrap();
    assert!(parsed.propagate().is_err());
}

/// Detects replaced caller filters or source aspirations after component construction.
#[test]
fn caller_pixel_filter_retains_oversize_aspiration() {
    let mut parsed = root("resize_straighten");
    let width = parsed.cell_names["width_pixels"].0;
    parsed.sheet.write(width, 400000i32).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<i32>(width).unwrap(), 300000);
    assert_eq!(*parsed.source::<i32>(width).unwrap(), 400000);
    assert_eq!(
        *parsed
            .read::<f64>(parsed.cell_names["width_inches"].0)
            .unwrap(),
        1000.0
    );
    assert!(parsed.cell_requirements_valid(parsed.cell_names["image"].0));
}

/// Detects precision-policy loss when finite affine coefficients suffer cancellation.
#[test]
fn insufficient_fit_precision_is_not_success() {
    let mut parsed = root("resize_straighten");
    let requested = Rect::new(Point2D::new(1.0e18, 1.0e18), Size2D::new(1600.0, 1200.0));
    parsed
        .sheet
        .write(
            parsed.cell_names["transform"].0,
            Transform2D::translation(1.0e18, 1.0e18),
        )
        .unwrap();
    parsed
        .sheet
        .write(parsed.cell_names["crop"].0, requested)
        .unwrap();
    parsed.propagate().unwrap();
    assert_eq!(
        *parsed.source::<Rect>(parsed.cell_names["crop"].0).unwrap(),
        requested
    );
    assert!(!*parsed.read::<bool>(parsed.cell_names["precise"].0).unwrap());
    assert!(!parsed.cell_requirements_valid(parsed.cell_names["precise"].0));
}
