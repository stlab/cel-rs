//! End-to-end capstone: build the sketch's own example document through
//! `ops`, persist and reload it, and confirm it still generates valid
//! `.adm2`.

use adam_lang::{AdamParser, TypeRegistry};
use cel_parser::OpLookup;
use ez_adam::codegen::generate_adm2;
use ez_adam::model::cell::CellType;
use ez_adam::model::document::Document;
use ez_adam::model::geometry::Point;
use ez_adam::ops::cells::{add_cell, add_cell_node, set_output};
use ez_adam::ops::conditionals::add_conditional_from_bool_cells;
use ez_adam::ops::relationships::{add_member, create_relationship, set_member_formula};
use ez_adam::persistence::{from_json, to_json};

/// Asserts that `adm2_text` parses successfully as `.adm2` source, via
/// `adam-lang`'s real parser with `cel-std` installed.
fn parses_as_adm2(adm2_text: &str) {
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

/// Builds the "Property Model Visualization" sketch's own resize example:
/// `width_pixels`/`height_pixels`/`aspect_ratio` bound by one relationship
/// group, active only when `constrain_proportions` is `true`.
fn build_resize_sheet() -> Document {
    let mut doc = Document::new("resize");

    let width = add_cell(&mut doc, "width_pixels", CellType::i64());
    let height = add_cell(&mut doc, "height_pixels", CellType::i64());
    let aspect = add_cell(&mut doc, "aspect_ratio", CellType::f64());
    set_output(&mut doc, width, true);
    set_output(&mut doc, height, true);

    let width_node = add_cell_node(&mut doc, width, Point::new(0.0, 0.0));
    let aspect_node = add_cell_node(&mut doc, aspect, Point::new(20.0, 0.0));
    let r1 = create_relationship(&mut doc, width_node, aspect_node, Point::new(10.0, 0.0));
    let height_node = add_cell_node(&mut doc, height, Point::new(10.0, 10.0));
    add_member(&mut doc, r1, height_node);
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

    let constrain = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
    let _ = add_conditional_from_bool_cells(&mut doc, vec![constrain], r1, Point::new(0.0, 30.0));

    doc
}

#[test]
fn the_resize_sheet_generates_valid_adm2() {
    let doc = build_resize_sheet();
    let adm2_text = generate_adm2(&doc);
    parses_as_adm2(&adm2_text);
}

#[test]
fn the_resize_sheet_survives_a_save_and_load_round_trip() {
    let doc = build_resize_sheet();
    let reloaded = from_json(&to_json(&doc)).unwrap();
    assert_eq!(doc, reloaded);
    assert_eq!(generate_adm2(&doc), generate_adm2(&reloaded));
}
