//! Translates a [`crate::model::document::Document`] into an
//! `adam_lang::ast::Sheet`, for [`super::generate_adm2`] to render via the
//! shared `adam_lang::format_sheet` — the same formatter `adam-fmt`/the VS
//! Code extension already use, rather than a second, independent one.

use crate::model::cell::{Cell, CellType};
use crate::model::document::Document;
use crate::model::relationship_group::{RelationshipGroup, RelationshipGroupId};
use adam_lang::ast::{BindingDecl, CellDecl, CellFilter, RelationshipDecl};
use cel_parser::{
    AstContext, ClosureParam, ClosureParamTypeExpr, Expr, ExprSpan, OpLookup, Parser,
};

use super::ExportError;

/// Parses `text` as a standalone CEL expression, for use as a formula's or
/// filter's `Expr` body when hand-building an AST node.
///
/// # Errors
///
/// Returns the underlying [`cel_parser::ParseError`] if `text` is not
/// syntactically valid CEL.
fn parse_expr_text(text: &str) -> Result<cel_parser::Expr, cel_parser::ParseError> {
    let mut lookup = OpLookup::new();
    cel_std::install(&mut lookup);
    let mut parser = Parser::<AstContext>::new(lookup);
    parser.parse_str_ast(text)
}

/// Returns `ty`'s `.adm2` type-name spelling as a hand-built `TypeExpr`,
/// with a span whose source text is genuinely that name (see
/// [`cel_parser::ExprSpan::for_text`]) so `format_sheet` renders it
/// correctly.
fn type_expr_for(ty: &CellType) -> adam_lang::ast::TypeExpr {
    let name = match ty {
        CellType::F64 { .. } => "f64",
        CellType::I64 { .. } => "i64",
        CellType::Bool => "bool",
        CellType::Text => "String",
    };
    adam_lang::ast::TypeExpr::Named(name.to_string(), cel_parser::ExprSpan::for_text(name))
}

/// Builds a `cell <name>: <type> [filter ...];` declaration for `cell`,
/// including a clamp filter clause when its type has clamp bounds set.
///
/// - Postcondition: `filter` is `None` iff `cell.ty` is `Bool`/`Text` or
///   has no clamp bounds.
// Not yet called from non-test code — `codegen/mod.rs`'s `generate_cell_decl`
// (Task 8 in this same plan) will call this once `build_relationship_decl`/
// `build_conditional_decl` (Tasks 6/7) exist too, at which point this and its
// callees (`clamp_filter`, `type_expr_for`, `parse_expr_text`) become reachable
// from a live, non-test call path and this attribute should come off.
#[allow(dead_code)]
fn build_cell_decl(cell: &Cell) -> CellDecl {
    CellDecl {
        name: cell.name.clone(),
        name_span: ExprSpan::for_text(&cell.name),
        type_name: Some(type_expr_for(&cell.ty)),
        initializer: None,
        filter: clamp_filter(&cell.ty),
        leading_comment: None,
        doc_comment: None,
        blank_line_before: false,
        span: ExprSpan::for_text(&cell.name),
    }
}

/// Returns a hand-built `filter |_: <type>| <clamp-call>` clause clamping
/// `ty`'s value to its clamp bounds, or `None` if `ty` is `Bool`/`Text` or
/// has no bounds set. The clamp-call text is generated the same way as
/// before this revision (explicit `i64` suffixes / `f64` `Debug`
/// formatting to avoid literal-type-inference ambiguity — see this
/// function's own body) and then parsed into an `Expr`, rather than the
/// whole `filter |_: ...| ...` clause being formatted as one string.
///
/// - Precondition: the synthesized clamp-call text is always valid CEL —
///   a parse failure here indicates a bug in this function, not bad user
///   data, so it panics rather than returning a `Result`.
fn clamp_filter(ty: &CellType) -> Option<CellFilter> {
    let (ty_name, body_text) = match ty {
        CellType::F64 { clamp } => (
            "f64",
            match (clamp.min, clamp.max) {
                (None, None) => return None,
                (Some(min), None) => format!("max(_, {min:?})"),
                (None, Some(max)) => format!("min(_, {max:?})"),
                (Some(min), Some(max)) => format!("clamp(_, {min:?}, {max:?})"),
            },
        ),
        CellType::I64 { clamp } => (
            "i64",
            match (clamp.min, clamp.max) {
                (None, None) => return None,
                (Some(min), None) => format!("max(_, {min}i64)"),
                (None, Some(max)) => format!("min(_, {max}i64)"),
                (Some(min), Some(max)) => format!("clamp(_, {min}i64, {max}i64)"),
            },
        ),
        CellType::Bool | CellType::Text => return None,
    };
    let body = parse_expr_text(&body_text).unwrap_or_else(|e| {
        panic!("synthesized clamp expression {body_text:?} failed to parse: {e:?}")
    });
    Some(CellFilter {
        arg_cells: vec![],
        closure: Expr::Closure {
            params: vec![ClosureParam {
                name: "_".to_string(),
                name_span: ExprSpan::for_text("_"),
                type_expr: ClosureParamTypeExpr::Named(
                    ty_name.to_string(),
                    ExprSpan::for_text(ty_name),
                ),
            }],
            body: Box::new(body),
            span: ExprSpan::for_text("_"),
        },
        span: ExprSpan::for_text("_"),
    })
}

/// Builds a `relationship { ... }` block for `group` (identified by
/// `group_id`, used only to label a formula error, not part of the
/// rendered output), one binding per member.
///
/// # Errors
///
/// Returns [`ExportError::InvalidFormula`] for the first member whose
/// formula text isn't valid CEL.
// Not yet called from non-test code — `codegen/mod.rs`'s integration (Task 8
// in this plan) will call this once it wires together cell/relationship/
// conditional generation, at which point this attribute should come off.
#[allow(dead_code)]
fn build_relationship_decl(
    doc: &Document,
    group: &RelationshipGroup,
    group_id: RelationshipGroupId,
) -> Result<RelationshipDecl, ExportError> {
    let mut bindings = Vec::with_capacity(group.members.len());
    for (node, formula) in &group.members {
        let cell_id = doc.cell_nodes[*node].cell;
        let cell = &doc.cells[cell_id];
        let body = parse_expr_text(formula).map_err(|source| ExportError::InvalidFormula {
            group: group_id,
            cell: cell_id,
            source,
        })?;
        bindings.push(BindingDecl {
            outputs: vec![(cell.name.clone(), ExprSpan::for_text(&cell.name))],
            destructure: false,
            body,
            leading_comment: None,
            blank_line_before: false,
            span: ExprSpan::for_text(&cell.name),
        });
    }
    Ok(RelationshipDecl {
        bindings,
        leading_comment: None,
        doc_comment: None,
        blank_line_before: false,
        trailing_comment: None,
        blank_line_before_close: false,
        open_brace_span: ExprSpan::for_text("relationship"),
        span: ExprSpan::for_text("relationship"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::cell::{Cell, ClampRange};

    #[test]
    fn build_cell_decl_for_a_plain_cell_has_no_filter() {
        let cell = Cell::new("width_pixels", CellType::i64());
        let decl = build_cell_decl(&cell);
        assert_eq!(decl.name, "width_pixels");
        assert!(decl.filter.is_none());
    }

    #[test]
    fn build_cell_decl_for_a_clamped_i64_cell_has_a_filter_clause() {
        let cell = Cell::new(
            "width_pixels",
            CellType::I64 {
                clamp: ClampRange {
                    min: Some(0),
                    max: Some(100),
                },
            },
        );
        let decl = build_cell_decl(&cell);
        let filter = decl.filter.expect("expected a filter clause");
        assert!(matches!(filter.closure, cel_parser::Expr::Closure { .. }));
    }

    #[test]
    fn parse_expr_text_accepts_valid_cel() {
        assert!(parse_expr_text("width_pixels / height_pixels").is_ok());
    }

    #[test]
    fn parse_expr_text_rejects_invalid_cel() {
        assert!(parse_expr_text("width_pixels / ").is_err());
    }

    #[test]
    fn type_expr_for_i64_has_the_right_source_text() {
        let type_expr = type_expr_for(&CellType::i64());
        match type_expr {
            adam_lang::ast::TypeExpr::Named(name, span) => {
                assert_eq!(name, "i64");
                assert_eq!(span.start.source_text().as_deref(), Some("i64"));
            }
            adam_lang::ast::TypeExpr::Tuple(..) => panic!("expected Named"),
        }
    }

    #[test]
    fn build_relationship_decl_produces_one_binding_per_member() {
        use crate::model::document::Document;
        use crate::model::geometry::Point;
        use crate::ops::cells::{add_cell, add_cell_node};
        use crate::ops::relationships::{create_relationship, set_member_formula};

        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group_id = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group_id, a_node, "height_pixels * 2i64");
        set_member_formula(&mut doc, group_id, b_node, "width_pixels / 2i64");

        let group = &doc.relationship_groups[group_id];
        let decl = build_relationship_decl(&doc, group, group_id).unwrap();
        assert_eq!(decl.bindings.len(), 2);
        assert_eq!(decl.bindings[0].outputs[0].0, "width_pixels");
    }

    #[test]
    fn build_relationship_decl_reports_an_invalid_formula() {
        use crate::model::document::Document;
        use crate::model::geometry::Point;
        use crate::ops::cells::{add_cell, add_cell_node};
        use crate::ops::relationships::create_relationship;

        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group_id = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        // Leave both formulas empty (the sketch's "[ ]" placeholder state).

        let group = &doc.relationship_groups[group_id];
        let result = build_relationship_decl(&doc, group, group_id);
        assert!(matches!(
            result,
            Err(ExportError::InvalidFormula { group, .. }) if group == group_id
        ));
    }
}
