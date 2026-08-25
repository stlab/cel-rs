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

    for (_, cell) in doc.cells_in_order() {
        if cell.output {
            out.push_str(&format!("    out {name} := {name};\n", name = cell.name));
        }
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

/// Returns every relationship group referenced by some conditional group's
/// `default` or a branch's `enabled_groups`, so [`generate_adm2`] can skip
/// them when emitting top-level relationship blocks (they're emitted
/// nested inside their owning `conditional` block instead).
///
/// - Complexity: O(n) in the total number of conditional-group branches.
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

/// Returns `ty`'s `.adm2` type-name spelling (`f64`, `i64`, `bool`, or
/// `String`).
fn type_name(ty: &CellType) -> &'static str {
    match ty {
        CellType::F64 { .. } => "f64",
        CellType::I64 { .. } => "i64",
        CellType::Bool => "bool",
        CellType::Text => "String",
    }
}

/// Renders `cell` as a `cell <name>: <type> [filter ...];` declaration,
/// including a clamp `filter` clause (see [`clamp_filter_clause`]) when
/// `cell`'s type has clamp bounds set.
fn generate_cell_decl(cell: &Cell) -> String {
    let ty = type_name(&cell.ty);
    match clamp_filter_clause(&cell.ty) {
        Some(filter) => format!("cell {}: {} {};", cell.name, ty, filter),
        None => format!("cell {}: {};", cell.name, ty),
    }
}

/// Returns a `filter |_: <type>| ...` clause clamping `ty`'s value to its
/// clamp bounds, or `None` if `ty` is `Bool`/`Text` or has no bounds set.
fn clamp_filter_clause(ty: &CellType) -> Option<String> {
    match ty {
        // `{:?}` (not `{}`) for f64 bounds: `f64::Display` drops the
        // trailing `.0` for whole numbers (`100.0` prints as `100`), which
        // risks the literal being lexed as an integer, not a float —
        // `f64::Debug` always includes a decimal point.
        CellType::F64 { clamp } => match (clamp.min, clamp.max) {
            (None, None) => None,
            (Some(min), None) => Some(format!("filter |_: f64| max(_, {min:?})")),
            (None, Some(max)) => Some(format!("filter |_: f64| min(_, {max:?})")),
            (Some(min), Some(max)) => Some(format!("filter |_: f64| clamp(_, {min:?}, {max:?})")),
        },
        // Explicit `i64` suffixes: bare integer literals are not
        // guaranteed to default to `i64` (the one confirmed example in
        // this codebase, `0i32`/`1i32` in `begin/examples/toy_example.adm2`,
        // suffixes every typed integer literal), so an unsuffixed `100`
        // risks a type mismatch against an `i64` filter parameter.
        CellType::I64 { clamp } => match (clamp.min, clamp.max) {
            (None, None) => None,
            (Some(min), None) => Some(format!("filter |_: i64| max(_, {min}i64)")),
            (None, Some(max)) => Some(format!("filter |_: i64| min(_, {max}i64)")),
            (Some(min), Some(max)) => Some(format!("filter |_: i64| clamp(_, {min}i64, {max}i64)")),
        },
        CellType::Bool | CellType::Text => None,
    }
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

    #[test]
    fn generates_an_out_decl_for_an_output_cell() {
        use crate::ops::cells::set_output;

        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        set_output(&mut doc, cell, true);

        let out = generate_adm2(&doc);
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: i64;\n    out width_pixels := width_pixels;\n}\n"
        );
    }

    #[test]
    fn generates_a_clamp_filter_with_both_bounds() {
        let mut doc = Document::new("demo");
        let _ = add_cell(
            &mut doc,
            "width_pixels",
            CellType::I64 {
                clamp: crate::model::cell::ClampRange {
                    min: Some(0),
                    max: Some(100),
                },
            },
        );
        let out = generate_adm2(&doc);
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: i64 filter |_: i64| clamp(_, 0i64, 100i64);\n}\n"
        );
    }

    #[test]
    fn generates_a_clamp_filter_with_only_a_minimum() {
        let mut doc = Document::new("demo");
        let _ = add_cell(
            &mut doc,
            "width_pixels",
            CellType::I64 {
                clamp: crate::model::cell::ClampRange {
                    min: Some(0),
                    max: None,
                },
            },
        );
        let out = generate_adm2(&doc);
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: i64 filter |_: i64| max(_, 0i64);\n}\n"
        );
    }

    #[test]
    fn generates_a_clamp_filter_with_only_a_maximum() {
        let mut doc = Document::new("demo");
        let _ = add_cell(
            &mut doc,
            "width_pixels",
            CellType::F64 {
                clamp: crate::model::cell::ClampRange {
                    min: None,
                    max: Some(100.0),
                },
            },
        );
        let out = generate_adm2(&doc);
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: f64 filter |_: f64| min(_, 100.0);\n}\n"
        );
    }

    #[test]
    fn omits_the_filter_clause_when_no_clamp_bounds_are_set() {
        let mut doc = Document::new("demo");
        let _ = add_cell(&mut doc, "width_pixels", CellType::i64());
        let out = generate_adm2(&doc);
        assert_eq!(out, "sheet demo {\n    cell width_pixels: i64;\n}\n");
    }
}
