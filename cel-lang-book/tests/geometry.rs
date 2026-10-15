//! Checked snippets included verbatim in the geometry chapter.

// ANCHOR: geometry
/// Installing geometry adds exact records and receiver syntax, not implicit casts.
#[test]
fn optional_geometry() {
    let mut lookup = cel_parser::OpLookup::new();
    cel_std::install(&mut lookup);
    cel_euclid::install(&mut lookup);
    let mut parser = cel_parser::CELParser::new(lookup);
    let mut segment = parser
        .parse_str(
            "translation(Vector2D { y: 0.0, x: 10.0 }).then(scale(2.0, 2.0))\
         .transform_point(Point2D { y: 0.0, x: 1.0 }).x",
        )
        .unwrap();
    assert_eq!(segment.call0::<f64>().unwrap(), 22.0);
    let mut segment = parser
        .parse_str("Point2D { y: 42.0, x: 0 as f64 }.y")
        .unwrap();
    assert_eq!(segment.call0::<f64>().unwrap(), 42.0);
}
// ANCHOR_END: geometry
