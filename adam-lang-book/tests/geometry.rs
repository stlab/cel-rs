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
