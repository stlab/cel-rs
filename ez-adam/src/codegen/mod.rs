//! Generates `.adm2` source text from a
//! [`crate::model::document::Document`].
//!
//! Generation is one-way: `.adm2` output is never parsed back into a
//! `Document`. See `docs/superpowers/specs/2026-08-24-ez-adam-design.md`.

use std::collections::HashSet;

use crate::model::cell::{Cell, CellType};
use crate::model::conditional_group::{CellValueLiteral, ConditionExpr, ConditionalGroup};
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

    // `Cell.output` is intentionally not reflected here: `adam-lang`'s `out`
    // declares a *new* identifier (it can't reuse an existing `cell`'s own
    // name) and is always a derived/computed value, never a flag on a plain
    // writable cell, so `out <name> := <name>;` doesn't parse. Deferred to
    // future design work — see <https://github.com/stlab/cel-rs/issues/147>.
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

    for (_, cond) in doc.conditional_groups_in_order() {
        out.push_str("    ");
        out.push_str(&generate_conditional_block(doc, cond));
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

/// Renders `cond` as a `conditional <expr> { <literal> => {...} ... _ => {...} }`
/// block, nesting each branch's (and the default's) enabled relationship
/// groups via [`generate_relationship_block`].
///
/// - Complexity: O(n) in the number of branches and their enabled groups.
fn generate_conditional_block(doc: &Document, cond: &ConditionalGroup) -> String {
    let mut s = String::from("conditional ");
    s.push_str(&condition_expr_text(doc, &cond.condition));
    s.push_str(" {\n");
    for branch in &cond.branches {
        s.push_str("        ");
        s.push_str(&branch_literal_text(&branch.values));
        s.push_str(" => {\n");
        for &group_id in &branch.enabled_groups {
            s.push_str("            ");
            s.push_str(&generate_relationship_block(
                doc,
                &doc.relationship_groups[group_id],
                "            ",
            ));
            s.push('\n');
        }
        s.push_str("        }\n");
    }
    s.push_str("        _ => {\n");
    for &group_id in &cond.default {
        s.push_str("            ");
        s.push_str(&generate_relationship_block(
            doc,
            &doc.relationship_groups[group_id],
            "            ",
        ));
        s.push('\n');
    }
    s.push_str("        }\n");
    s.push_str("    }");
    s
}

/// Returns `condition`'s `.adm2` spelling: the referenced cells' names (see
/// [`cell_names_text`]) for [`ConditionExpr::Cells`], or the raw CEL
/// expression text for [`ConditionExpr::Formula`].
fn condition_expr_text(doc: &Document, condition: &ConditionExpr) -> String {
    match condition {
        ConditionExpr::Cells(cells) => cell_names_text(doc, cells),
        ConditionExpr::Formula { expr, .. } => expr.clone(),
    }
}

/// Returns `cells`' names joined for use as a conditional's condition
/// expression: a bare name for a single cell, or a parenthesized
/// comma-separated tuple for multiple cells.
fn cell_names_text(doc: &Document, cells: &[crate::model::cell::CellId]) -> String {
    let names: Vec<&str> = cells.iter().map(|c| doc.cells[*c].name.as_str()).collect();
    if names.len() == 1 {
        names[0].to_string()
    } else {
        format!("({})", names.join(", "))
    }
}

/// Returns `values`' `.adm2` spelling for use as a branch's match arm: a
/// bare literal (see [`literal_text`]) for a single value, or a
/// parenthesized comma-separated tuple for multiple values.
fn branch_literal_text(values: &[CellValueLiteral]) -> String {
    let literals: Vec<String> = values.iter().map(literal_text).collect();
    if literals.len() == 1 {
        literals[0].clone()
    } else {
        format!("({})", literals.join(", "))
    }
}

/// Returns `value`'s `.adm2` literal spelling (`true`/`false` for
/// [`CellValueLiteral::Bool`], an `i64`-suffixed integer for
/// [`CellValueLiteral::I64`], or a quoted/escaped string for
/// [`CellValueLiteral::Text`]).
fn literal_text(value: &CellValueLiteral) -> String {
    match value {
        CellValueLiteral::Bool(b) => b.to_string(),
        CellValueLiteral::I64(n) => format!("{n}i64"),
        CellValueLiteral::Text(s) => format!("{s:?}"),
    }
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
    fn does_not_emit_an_out_decl_for_an_output_cell_yet() {
        // `out` codegen for the `output` flag is deferred — see
        // https://github.com/stlab/cel-rs/issues/147. `out` always declares a
        // new identifier and can't reuse an existing cell's own name, so
        // `out <name> := <name>;` (the original approach) doesn't parse.
        use crate::ops::cells::set_output;

        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        set_output(&mut doc, cell, true);

        let out = generate_adm2(&doc);
        assert_eq!(out, "sheet demo {\n    cell width_pixels: i64;\n}\n");
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

    #[test]
    fn generates_a_conditional_group_with_bool_condition() {
        use crate::ops::conditionals::add_conditional_from_bool_cells;

        let mut doc = Document::new("demo");
        let flag = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group, a_node, "height_pixels * 2");
        let _ = add_conditional_from_bool_cells(&mut doc, vec![flag], group, Point::new(0.0, 20.0));

        let out = generate_adm2(&doc);
        // Branches are generated in the order `add_conditional_from_bool_cells`
        // enumerated them: `combo` counts up from 0, so the all-`false`
        // combination (combo == 0) comes first, `true` (combo == 1) second.
        assert_eq!(
            out,
            "sheet demo {\n    cell constrain_proportions: bool;\n    cell width_pixels: i64;\n    cell height_pixels: i64;\n    conditional constrain_proportions {\n        false => {\n        }\n        true => {\n            relationship {\n                width_pixels := height_pixels * 2;\n                height_pixels := ;\n            }\n        }\n        _ => {\n        }\n    }\n}\n"
        );
    }

    #[test]
    fn generates_a_conditional_group_with_a_multi_cell_tuple_condition() {
        use crate::ops::conditionals::add_conditional_from_bool_cells;

        let mut doc = Document::new("demo");
        let flag_a = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
        let flag_b = add_cell(&mut doc, "lock_aspect", CellType::Bool);
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        let _ = add_conditional_from_bool_cells(
            &mut doc,
            vec![flag_a, flag_b],
            group,
            Point::new(0.0, 20.0),
        );

        let out = generate_adm2(&doc);
        assert!(out.contains("conditional (constrain_proportions, lock_aspect) {\n"));
        // combo 0..4: (false,false), (true,false), (false,true), (true,true) —
        // bit i of combo selects cells[i]'s value.
        assert!(out.contains("        (false, false) => {\n        }\n"));
        assert!(out.contains("        (true, false) => {\n        }\n"));
        assert!(out.contains("        (false, true) => {\n        }\n"));
        assert!(out.contains("        (true, true) => {\n            relationship {\n"));
    }

    #[test]
    fn generates_a_conditional_group_with_a_formula_condition() {
        use crate::model::conditional_group::CellValueLiteral;
        use crate::ops::conditionals::{
            add_branch, add_conditional_with_formula, toggle_enabled_group,
        };

        let mut doc = Document::new("demo");
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
        add_branch(&mut doc, cond, vec![CellValueLiteral::Bool(true)]);
        toggle_enabled_group(&mut doc, cond, 0, group);

        let out = generate_adm2(&doc);
        assert!(out.contains("conditional aspect_ratio > 2.0 {\n"));
        assert!(out.contains("        true => {\n            relationship {\n"));
        assert!(out.contains("        _ => {\n        }\n"));
    }
}
