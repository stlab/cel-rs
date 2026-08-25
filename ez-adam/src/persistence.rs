//! Save/load [`Document`]s as JSON — `ez-adam`'s native document format.
//! `.adm2` export ([`crate::codegen::generate_adm2`]) is a separate,
//! one-way operation; it is never read back in.

use crate::model::document::Document;

/// Serializes `doc` to pretty-printed JSON.
#[must_use]
pub fn to_json(doc: &Document) -> String {
    serde_json::to_string_pretty(doc).expect("Document always serializes")
}

/// Deserializes a `Document` from JSON text produced by [`to_json`].
///
/// # Errors
///
/// Returns an error if `text` is not valid JSON, or does not match
/// [`Document`]'s current shape. Only
/// [`crate::model::document::CURRENT_FORMAT_VERSION`] is currently
/// supported — no migration path exists yet for older versions.
pub fn from_json(text: &str) -> Result<Document, serde_json::Error> {
    serde_json::from_str(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::cell::CellType;
    use crate::model::geometry::Point;
    use crate::ops::cells::{add_cell, add_cell_node};
    use crate::ops::relationships::{create_relationship, set_member_formula};

    #[test]
    fn round_trips_a_document_with_cells_and_a_relationship() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group, a_node, "height_pixels * 2");

        let json = to_json(&doc);
        let back = from_json(&json).unwrap();
        assert_eq!(doc, back);
    }

    #[test]
    fn from_json_rejects_malformed_json() {
        assert!(from_json("not json").is_err());
    }

    #[test]
    fn round_trips_a_document_with_a_formula_conditional_and_a_text_cell() {
        use crate::model::conditional_group::CellValueLiteral;
        use crate::ops::cells::set_restrict;
        use crate::ops::conditionals::{
            add_branch, add_conditional_with_formula, toggle_enabled_group,
        };

        let mut doc = Document::new("demo");
        let label = add_cell(&mut doc, "label", CellType::Text);
        set_restrict(&mut doc, label, Some("_ != \"\"".to_string()));
        let aspect = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let cond = add_conditional_with_formula(
            &mut doc,
            vec![aspect],
            "aspect_ratio > 2.0",
            Point::new(0.0, 20.0),
        );
        add_branch(
            &mut doc,
            cond,
            vec![CellValueLiteral::Text("wide".to_string())],
        );
        toggle_enabled_group(&mut doc, cond, 0, group);

        let json = to_json(&doc);
        let back = from_json(&json).unwrap();
        assert_eq!(doc, back);
    }
}
