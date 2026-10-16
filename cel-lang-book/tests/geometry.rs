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

/// Evaluates each annotated reference example against its stated result or error.
///
/// - Complexity: Proportional to parsing and evaluating the chapter's examples.
#[test]
fn reference_examples_match_documented_results() {
    let mut lookup = cel_parser::OpLookup::new();
    cel_std::install(&mut lookup);
    cel_euclid::install(&mut lookup);
    let mut parser = cel_parser::CELParser::new(lookup);
    let mut checked = 0;
    for block in include_str!("../book-src/geometry.md")
        .split("```text")
        .skip(1)
    {
        let (example, _) = block.split_once("```").expect("closed example fence");
        if let Some((source, expected)) = example.split_once("// =>") {
            let expression = format!("({source}) == ({expected})");
            let mut segment = parser
                .parse_str(&expression)
                .unwrap_or_else(|error| panic!("{source}: {error}"));
            assert!(
                segment
                    .call0::<bool>()
                    .unwrap_or_else(|error| panic!("{source}: {error}")),
                "{source}: expected {expected}"
            );
            checked += 1;
        } else if let Some((source, _)) = example.split_once("// Evaluation error:") {
            let mut segment = parser
                .parse_str(&format!("({source}).is_finite"))
                .unwrap_or_else(|error| panic!("{source}: {error}"));
            assert!(segment.call0::<bool>().is_err(), "{source}: expected error");
            checked += 1;
        }
    }
    assert!(checked > 0, "the chapter must contain executable examples");
}
