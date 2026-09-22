//! Confirms `generate_adm2`'s output is syntactically valid `.adm2` source,
//! for every construct it can emit.

use adam_lang::{AdamParser, TypeRegistry};
use cel_parser::OpLookup;
use ez_adam::codegen::generate_adm2;
use ez_adam::model::cell::CellType;
use ez_adam::model::conditional_group::CellValueLiteral;
use ez_adam::model::document::Document;
use ez_adam::model::geometry::Point;
use ez_adam::ops::cells::{add_cell, add_cell_node, set_output};
use ez_adam::ops::conditionals::{
    add_branch, add_conditional_from_bool_cells, add_conditional_with_formula, toggle_enabled_group,
};
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
        "((width_pixels as f64) / aspect_ratio) as i64",
    );
    set_member_formula(
        &mut doc,
        r1,
        aspect_node,
        "(width_pixels as f64) / (height_pixels as f64)",
    );

    let flag = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
    let _ = add_conditional_from_bool_cells(&mut doc, vec![flag], r1, Point::new(0.0, 40.0));

    let adm2_text = generate_adm2(&doc).expect("document should export cleanly");
    assert_parses(&adm2_text);
}

#[test]
fn a_bare_cell_only_document_generates_valid_adm2() {
    let mut doc = Document::new("empty_ish");
    let _ = add_cell(&mut doc, "a", CellType::f64());
    let adm2_text = generate_adm2(&doc).expect("document should export cleanly");
    assert_parses(&adm2_text);
}

#[test]
fn a_multi_cell_cells_mode_conditional_group_generates_valid_adm2() {
    let mut doc = Document::new("resize");

    let width = add_cell(&mut doc, "width_pixels", CellType::i64());
    let height = add_cell(&mut doc, "height_pixels", CellType::i64());
    let width_node = add_cell_node(&mut doc, width, Point::new(0.0, 0.0));
    let height_node = add_cell_node(&mut doc, height, Point::new(10.0, 0.0));

    let r1 = create_relationship(&mut doc, width_node, height_node, Point::new(5.0, 5.0));
    set_member_formula(&mut doc, r1, width_node, "height_pixels * 2i64");
    set_member_formula(&mut doc, r1, height_node, "width_pixels / 2i64");

    let flag_a = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
    let flag_b = add_cell(&mut doc, "lock_aspect", CellType::Bool);
    let _ =
        add_conditional_from_bool_cells(&mut doc, vec![flag_a, flag_b], r1, Point::new(0.0, 40.0));

    let adm2_text = generate_adm2(&doc).expect("document should export cleanly");
    assert_parses(&adm2_text);
}

/// Builds a document whose one relationship (`width_pixels`/`height_pixels`)
/// is enabled by a single-branch `Formula`-mode conditional matching a
/// separate `mode` cell against `key`. The match cell isn't part of the
/// enabled relationship, so the conditional is structurally valid; the only
/// variable across calls is the branch key literal.
fn doc_with_i64_branch_key(key: i64) -> Document {
    let mut doc = Document::new("repro");
    let mode = add_cell(&mut doc, "mode", CellType::i64());
    let width = add_cell(&mut doc, "width_pixels", CellType::i64());
    let width_node = add_cell_node(&mut doc, width, Point::new(0.0, 0.0));
    let height = add_cell(&mut doc, "height_pixels", CellType::i64());
    let height_node = add_cell_node(&mut doc, height, Point::new(10.0, 0.0));
    let r1 = create_relationship(&mut doc, width_node, height_node, Point::new(5.0, 5.0));
    set_member_formula(&mut doc, r1, width_node, "height_pixels * 2i64");
    set_member_formula(&mut doc, r1, height_node, "width_pixels / 2i64");
    let cond = add_conditional_with_formula(&mut doc, vec![mode], "mode", Point::new(0.0, 40.0));
    add_branch(&mut doc, cond, vec![CellValueLiteral::I64(key)]);
    toggle_enabled_group(&mut doc, cond, 0, r1);
    doc
}

#[test]
fn an_i64_min_branch_key_reports_an_unrepresentable_error_rather_than_emitting_unparsable_adm2() {
    // `adam_lang`'s branch grammar stores the sign separately from an
    // *unsigned* literal token, and `i64::MIN`'s magnitude is out of range
    // for an `i64` literal — so it can't be spelled as a branch key. Export
    // must surface this as an error, not silently emit `.adm2` that fails to
    // parse. See https://github.com/stlab/cel-rs/issues/175.
    let doc = doc_with_i64_branch_key(i64::MIN);
    let result = generate_adm2(&doc);
    assert!(
        matches!(
            result,
            Err(
                ez_adam::codegen::ExportError::UnrepresentableBranchLiteral {
                    value: i64::MIN,
                    ..
                }
            )
        ),
        "expected UnrepresentableBranchLiteral, got {result:?}"
    );
}

#[test]
fn an_i64_max_branch_key_still_exports_and_parses() {
    // The boundary just inside the representable range: `i64::MAX`'s
    // magnitude equals `i64::MAX`, so it is a valid literal token and must
    // still round-trip cleanly.
    let doc = doc_with_i64_branch_key(i64::MAX);
    let adm2_text = generate_adm2(&doc).expect("i64::MAX branch key should export cleanly");
    assert_parses(&adm2_text);
}

#[test]
fn a_formula_mode_conditional_group_generates_valid_adm2() {
    let mut doc = Document::new("resize");

    let aspect = add_cell(&mut doc, "aspect_ratio", CellType::f64());
    let width = add_cell(&mut doc, "width_pixels", CellType::i64());
    let height = add_cell(&mut doc, "height_pixels", CellType::i64());
    let width_node = add_cell_node(&mut doc, width, Point::new(0.0, 0.0));
    let height_node = add_cell_node(&mut doc, height, Point::new(10.0, 0.0));

    let r1 = create_relationship(&mut doc, width_node, height_node, Point::new(5.0, 5.0));
    set_member_formula(&mut doc, r1, width_node, "height_pixels * 2i64");
    set_member_formula(&mut doc, r1, height_node, "width_pixels / 2i64");

    let cond = add_conditional_with_formula(
        &mut doc,
        vec![aspect],
        "aspect_ratio > 2.0",
        Point::new(0.0, 40.0),
    );
    add_branch(&mut doc, cond, vec![CellValueLiteral::Bool(true)]);
    toggle_enabled_group(&mut doc, cond, 0, r1);

    let adm2_text = generate_adm2(&doc).expect("document should export cleanly");
    assert_parses(&adm2_text);
}
