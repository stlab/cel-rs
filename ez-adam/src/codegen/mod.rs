//! Generates `.adm2` source text from a
//! [`crate::model::document::Document`].
//!
//! Generation is one-way: `.adm2` output is never parsed back into a
//! `Document`. See `docs/superpowers/specs/2026-08-24-ez-adam-design.md`.

use std::collections::HashSet;

use crate::model::cell::{Cell, CellType};
use crate::model::document::Document;
use crate::model::relationship_group::{RelationshipGroup, RelationshipGroupId};

/// Returns `.adm2` source text for `doc`.
///
/// - Complexity: O(n) in the total number of cells, relationship groups,
///   and conditional-group branches.
#[must_use]
pub fn generate_adm2(doc: &Document) -> String {
    let mut out = String::new();
    out.push_str(&format!("sheet {} {{\n", doc.sheet_name));

    for (_, cell) in doc.cells_in_order() {
        out.push_str("    ");
        out.push_str(&generate_cell_decl(cell));
        out.push('\n');
    }

    let owned = groups_owned_by_conditionals(doc);
    for (id, group) in doc.relationship_groups_in_order() {
        if owned.contains(&id) {
            continue;
        }
        out.push_str("    ");
        out.push_str(&generate_relationship_block(doc, group, "    "));
        out.push('\n');
    }

    out.push_str("}\n");
    out
}

fn groups_owned_by_conditionals(doc: &Document) -> HashSet<RelationshipGroupId> {
    let mut owned = HashSet::new();
    for (_, cond) in doc.conditional_groups_in_order() {
        owned.extend(cond.default.iter().copied());
        for branch in &cond.branches {
            owned.extend(branch.enabled_groups.iter().copied());
        }
    }
    owned
}

fn type_name(ty: &CellType) -> &'static str {
    match ty {
        CellType::F64 { .. } => "f64",
        CellType::I64 { .. } => "i64",
        CellType::Bool => "bool",
        CellType::Text => "String",
    }
}

fn generate_cell_decl(cell: &Cell) -> String {
    format!("cell {}: {};", cell.name, type_name(&cell.ty))
}

/// Renders `group` as a `relationship { ... }` block, with `indent` as the
/// prefix for its opening/closing braces (member lines are indented one
/// level deeper than `indent`). Takes an explicit `indent` rather than a
/// fixed one so the same function produces correctly nested output whether
/// called at the top level ([`generate_adm2`]) or inside a `conditional`
/// branch ([`generate_conditional_block`], Task 17).
fn generate_relationship_block(doc: &Document, group: &RelationshipGroup, indent: &str) -> String {
    let mut s = String::from("relationship {\n");
    let member_indent = format!("{indent}    ");
    for (node, formula) in &group.members {
        let cell = &doc.cells[doc.cell_nodes[*node].cell];
        s.push_str(&format!("{member_indent}{} := {};\n", cell.name, formula));
    }
    s.push_str(indent);
    s.push('}');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::cell::CellType;
    use crate::model::geometry::Point;
    use crate::ops::cells::{add_cell, add_cell_node};
    use crate::ops::relationships::{create_relationship, set_member_formula};

    #[test]
    fn generates_bare_cell_declarations() {
        let mut doc = Document::new("demo");
        let _ = add_cell(&mut doc, "width_pixels", CellType::i64());
        let _ = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let out = generate_adm2(&doc);
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: i64;\n    cell aspect_ratio: f64;\n}\n"
        );
    }

    #[test]
    fn generates_a_top_level_relationship_block() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group, a_node, "height_pixels * 2");

        let out = generate_adm2(&doc);
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: i64;\n    cell height_pixels: i64;\n    relationship {\n        width_pixels := height_pixels * 2;\n        height_pixels := ;\n    }\n}\n"
        );
    }
}
