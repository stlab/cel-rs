//! Executable geometry chapter examples share the library's actual sheets.

/// The book host executes self-referencing fitting and retains source aspiration.
#[test]
fn crop_chapter_executes_and_springs_back() {
    let mut parsed = adam_lang_book::support::parser()
        .parse_str(include_str!(
            "../../cel-euclid/examples/crop_straighten.adm2"
        ))
        .unwrap();
    let crop = parsed.cell_names["crop"].0;
    let transform = parsed.cell_names["transform"].0;
    let anchor = cel_euclid::Point2D::new(50.0, 50.0);
    let rotation = cel_euclid::Transform2D::translation(-anchor.x, -anchor.y)
        .then(&cel_euclid::Transform2D::rotation(
            cel_euclid::Angle::degrees(45.0),
        ))
        .then(&cel_euclid::Transform2D::translation(anchor.x, anchor.y));
    parsed.write(transform, rotation).unwrap();
    parsed.propagate().unwrap();
    assert!(
        (parsed.read::<cel_euclid::Rect>(crop).unwrap().width() - 100.0 / 2.0_f64.sqrt()).abs()
            < 1e-12
    );
    assert_eq!(
        parsed
            .sheet
            .source::<cel_euclid::Rect>(crop)
            .unwrap()
            .width(),
        100.0
    );
    parsed
        .write(transform, cel_euclid::Transform2D::identity())
        .unwrap();
    parsed.propagate().unwrap();
    assert_eq!(
        parsed.read::<cel_euclid::Rect>(crop).unwrap().width(),
        100.0
    );
}

/// The chapter accepts resolved extreme shrink and reports reciprocal-scale precision loss.
#[test]
fn crop_chapter_checks_extreme_ratio_presentation_precision() {
    use cel_euclid::{Point2D, Rect, Size2D, Transform2D};
    for (center, accepted) in [(0.0, true), (0.09999999997671694, false)] {
        let mut parsed = adam_lang_book::support::parser()
            .parse_str(include_str!(
                "../../cel-euclid/examples/crop_straighten.adm2"
            ))
            .unwrap();
        let image = parsed.cell_names["image"].0;
        let crop = parsed.cell_names["crop"].0;
        let transform = parsed.cell_names["transform"].0;
        let precise = parsed.cell_names["precise"].0;
        let requested = Rect::new(
            Point2D::new(center - 5e5, center - 5e5),
            Size2D::new(1e6, 1e6),
        );
        parsed
            .write(
                image,
                Rect::new(
                    Point2D::new(center / 3.0 - 5e-7, center / 3.0 - 5e-7),
                    Size2D::new(1e-6, 1e-6),
                ),
            )
            .unwrap();
        parsed.write(crop, requested).unwrap();
        parsed
            .write(transform, Transform2D::scale(3.0, 3.0))
            .unwrap();
        parsed.propagate().unwrap();
        assert_eq!(parsed.cell_requirements_valid(precise), accepted);
        assert_eq!(*parsed.sheet.source::<Rect>(crop).unwrap(), requested);
        if accepted {
            assert!(
                parsed
                    .cell_names
                    .values()
                    .all(|(id, _)| parsed.cell_requirements_valid(*id))
            );
            let fitted = *parsed.read::<Rect>(crop).unwrap();
            let p = *parsed
                .read::<Transform2D>(parsed.cell_names["presentation"].0)
                .unwrap();
            for (point, expected) in [
                (fitted.center(), requested.center()),
                (fitted.origin, requested.origin),
                (
                    Point2D::new(fitted.max_x(), fitted.max_y()),
                    Point2D::new(requested.max_x(), requested.max_y()),
                ),
            ] {
                let displayed = p.transform_point(
                    Transform2D::scale(3.0, 3.0)
                        .inverse()
                        .unwrap()
                        .transform_point(point),
                );
                let e = 64.0 * f64::EPSILON * requested.width();
                assert!((displayed.x - expected.x).abs() <= e);
                assert!((displayed.y - expected.y).abs() <= e);
            }
        } else {
            let requirement = parsed.violated_requirements(precise).next().unwrap();
            assert_eq!(
                parsed.requirement_names[&requirement],
                "sufficient_fit_and_presentation_precision"
            );
        }
    }
}

/// The graph-policy chapter executes with equivalent book registrations.
#[test]
fn pan_zoom_chapter_executes() {
    let mut parsed = adam_lang_book::support::parser()
        .parse_str(include_str!("../../cel-euclid/examples/pan_zoom.adm2"))
        .unwrap();
    parsed.propagate().unwrap();
    assert_eq!(
        *parsed
            .read::<f64>(parsed.cell_names["fit_scale"].0)
            .unwrap(),
        2.0
    );
}
