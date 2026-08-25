//! Confirms `generate_adm2`'s output is syntactically valid `.adm2` source,
//! for every construct it can emit.

use adam_lang::{AdamParser, TypeRegistry};
use cel_parser::OpLookup;
use ez_adam::codegen::generate_adm2;
use ez_adam::model::cell::CellType;
use ez_adam::model::document::Document;
use ez_adam::model::geometry::Point;
use ez_adam::ops::cells::{add_cell, add_cell_node, set_output};
use ez_adam::ops::conditionals::add_conditional_from_bool_cells;
use ez_adam::ops::relationships::{create_relationship, set_member_formula};

/// Asserts that `adm2_text` parses successfully as `.adm2` source, via
/// `adam-lang`'s real parser with `cel-std` installed.
fn assert_parses(adm2_text: &str) {
    let mut lookup = OpLookup::new();
    cel_std::install(&mut lookup);
    let mut parser = AdamParser::new(TypeRegistry::new(), lookup);
    let result = parser.parse_str(adm2_text);
    assert!(
        result.is_ok(),
        "failed to parse:\n{adm2_text}\n\nerror: {:?}",
        result.err()
    );
}

#[test]
fn a_document_with_every_construct_generates_valid_adm2() {
    let mut doc = Document::new("resize");

    let width = add_cell(
        &mut doc,
        "width_pixels",
        CellType::I64 {
            clamp: ez_adam::model::cell::ClampRange {
                min: Some(0),
                max: Some(4096),
            },
        },
    );
    let height = add_cell(&mut doc, "height_pixels", CellType::i64());
    let aspect = add_cell(&mut doc, "aspect_ratio", CellType::f64());
    set_output(&mut doc, width, true);

    let width_node = add_cell_node(&mut doc, width, Point::new(0.0, 0.0));
    let height_node = add_cell_node(&mut doc, height, Point::new(10.0, 0.0));
    let aspect_node = add_cell_node(&mut doc, aspect, Point::new(20.0, 0.0));

    let r1 = create_relationship(&mut doc, width_node, height_node, Point::new(5.0, 5.0));
    ez_adam::ops::relationships::add_member(&mut doc, r1, aspect_node);
    // Every member needs a non-empty formula: an empty RHS (the sketch's
    // "[ ]" placeholder for an as-yet-unfilled-in formula) is valid
    // intermediate editor state, but isn't valid CEL syntax, so it can't
    // appear in text this test actually parses. Explicit `as f64`/`as i64`
    // casts are required too: `width_pixels`/`height_pixels` are `i64` and
    // `aspect_ratio` is `f64`, and CEL has no implicit int/float widening
    // (matching the convention in `begin/examples/image_resize.adm2`).
    set_member_formula(
        &mut doc,
        r1,
        width_node,
        "(aspect_ratio * (height_pixels as f64)) as i64",
    );
    set_member_formula(
        &mut doc,
        r1,
        height_node,
        "(aspect_ratio / (width_pixels as f64)) as i64",
    );
    set_member_formula(
        &mut doc,
        r1,
        aspect_node,
        "(width_pixels as f64) / (height_pixels as f64)",
    );

    let flag = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
    let _ = add_conditional_from_bool_cells(&mut doc, vec![flag], r1, Point::new(0.0, 40.0));

    let adm2_text = generate_adm2(&doc);
    assert_parses(&adm2_text);
}

#[test]
fn a_bare_cell_only_document_generates_valid_adm2() {
    let mut doc = Document::new("empty_ish");
    let _ = add_cell(&mut doc, "a", CellType::f64());
    let adm2_text = generate_adm2(&doc);
    assert_parses(&adm2_text);
}
