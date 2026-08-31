//! Translates a [`crate::model::document::Document`] into an
//! `adam_lang::ast::Sheet`, for [`super::generate_adm2`] to render via the
//! shared `adam_lang::format_sheet` — the same formatter `adam-fmt`/the VS
//! Code extension already use, rather than a second, independent one.

use crate::model::cell::{Cell, CellId, CellType};
use crate::model::conditional_group::{
    CellValueLiteral, ConditionExpr, ConditionalGroup, ConditionalGroupId,
};
use crate::model::document::Document;
use crate::model::relationship_group::{RelationshipGroup, RelationshipGroupId};
use adam_lang::ast::{
    BindingDecl, CellDecl, CellFilter, ConditionalBranch, ConditionalDecl, DefaultBranch,
    MatchLiteral, RelationshipDecl,
};
use cel_parser::{AstContext, Expr, ExprSpan, OpLookup, Parser};

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
pub(crate) fn build_cell_decl(cell: &Cell) -> CellDecl {
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

/// Returns a hand-built `filter <clamp-call>` clause clamping `ty`'s value
/// to its clamp bounds, or `None` if `ty` is `Bool`/`Text` or has no bounds
/// set. `adam-lang`'s current `cell_filter` grammar (`"filter" or_expression`)
/// takes a bare expression with no explicit closure/parameter list — `_`
/// implicitly refers to the candidate value being conformed, deduced
/// automatically by `adam-lang` itself, so no `ClosureParam`/type
/// annotation needs to be built here (unlike prior revisions of this
/// function, from before `adam-lang` simplified `cell_filter`'s grammar).
/// The clamp-call text still uses explicit `i64` suffixes / `f64` `Debug`
/// formatting to avoid literal-type-inference ambiguity — see this
/// function's own body — and is parsed into an `Expr` to become the
/// filter's `body` directly.
///
/// - Precondition: the synthesized clamp-call text is always valid CEL —
///   a parse failure here indicates a bug in this function, not bad user
///   data, so it panics rather than returning a `Result`.
fn clamp_filter(ty: &CellType) -> Option<CellFilter> {
    let body_text = match ty {
        CellType::F64 { clamp } => match (clamp.min, clamp.max) {
            (None, None) => return None,
            (Some(min), None) => format!("max(_, {min:?})"),
            (None, Some(max)) => format!("min(_, {max:?})"),
            (Some(min), Some(max)) => format!("clamp(_, {min:?}, {max:?})"),
        },
        CellType::I64 { clamp } => match (clamp.min, clamp.max) {
            (None, None) => return None,
            (Some(min), None) => format!("max(_, {min}i64)"),
            (None, Some(max)) => format!("min(_, {max}i64)"),
            (Some(min), Some(max)) => format!("clamp(_, {min}i64, {max}i64)"),
        },
        CellType::Bool | CellType::Text => return None,
    };
    let body = parse_expr_text(&body_text).unwrap_or_else(|e| {
        panic!("synthesized clamp expression {body_text:?} failed to parse: {e:?}")
    });
    Some(CellFilter {
        body,
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
///
/// - Complexity: O(n) in `group.members.len()`.
pub(crate) fn build_relationship_decl(
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

/// Builds a `conditional <expr> { <literal> => {...} ... _ => {...} }`
/// declaration for `cond` (identified by `conditional_id`, used only to
/// label a condition-parse error, not part of the rendered output).
///
/// # Errors
///
/// Propagates [`ExportError::InvalidFormula`] from any nested relationship
/// group's members, or returns [`ExportError::InvalidCondition`] if a
/// `Formula`-mode condition expression isn't valid CEL.
///
/// - Complexity: O(n) in the total number of branches, their enabled
///   groups' members, and the default's members.
pub(crate) fn build_conditional_decl(
    doc: &Document,
    conditional_id: ConditionalGroupId,
    cond: &ConditionalGroup,
) -> Result<ConditionalDecl, ExportError> {
    let match_expr = match &cond.condition {
        ConditionExpr::Cells(cells) => cells_tuple_expr(doc, cells),
        ConditionExpr::Formula { expr, .. } => {
            parse_expr_text(expr).map_err(|source| ExportError::InvalidCondition {
                conditional: conditional_id,
                source,
            })?
        }
    };

    let mut branches = Vec::with_capacity(cond.branches.len());
    for branch in &cond.branches {
        let mut relationships = Vec::with_capacity(branch.enabled_groups.len());
        for &group_id in &branch.enabled_groups {
            relationships.push(build_relationship_decl(
                doc,
                &doc.relationship_groups[group_id],
                group_id,
            )?);
        }
        branches.push(ConditionalBranch {
            literal: match_literal_for(&branch.values),
            literal_span: match_literal_span(&branch.values),
            relationships,
            leading_comment: None,
            blank_line_before: false,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: ExprSpan::for_text("_"),
            span: ExprSpan::for_text("_"),
        });
    }

    let mut default_relationships = Vec::with_capacity(cond.default.len());
    for &group_id in &cond.default {
        default_relationships.push(build_relationship_decl(
            doc,
            &doc.relationship_groups[group_id],
            group_id,
        )?);
    }

    Ok(ConditionalDecl {
        match_expr,
        branches,
        default: Some(DefaultBranch {
            relationships: default_relationships,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: ExprSpan::for_text("_"),
            span: ExprSpan::for_text("_"),
        }),
        leading_comment: None,
        doc_comment: None,
        blank_line_before: false,
        trailing_comment: None,
        blank_line_before_close: false,
        open_brace_span: ExprSpan::for_text("conditional"),
        span: ExprSpan::for_text("conditional"),
    })
}

/// Builds the `(a, b, ...)` tuple expression naming `cells`, for a
/// `Cells`-mode condition — a single cell renders as a bare identifier
/// reference instead of a one-element tuple.
///
/// - Precondition: the synthesized text is always valid CEL (a bare
///   identifier, or a parenthesized comma-list of them) — a parse failure
///   here indicates a bug in this function, not bad user data, so it
///   panics rather than returning a `Result`.
///
/// - Complexity: O(n) in `cells.len()`.
fn cells_tuple_expr(doc: &Document, cells: &[CellId]) -> Expr {
    let text = if cells.len() == 1 {
        doc.cells[cells[0]].name.clone()
    } else {
        let names: Vec<&str> = cells.iter().map(|c| doc.cells[*c].name.as_str()).collect();
        format!("({})", names.join(", "))
    };
    parse_expr_text(&text).unwrap_or_else(|e| {
        panic!("synthesized condition expression {text:?} failed to parse: {e:?}")
    })
}

/// Returns `value`'s `.adm2` literal spelling (`i64` suffixes,
/// `Debug`-quoted/escaped strings), the shared text synthesis used by both
/// [`literal_for`] (to re-lex it into a [`cel_parser::lex_lexer::Literal`])
/// and [`match_literal_span`] (to widen a `Tuple` branch's span over the
/// joined text of every element).
fn literal_text(value: &CellValueLiteral) -> String {
    match value {
        CellValueLiteral::Bool(b) => b.to_string(),
        CellValueLiteral::I64(n) => format!("{n}i64"),
        CellValueLiteral::Text(s) => format!("{s:?}"),
    }
}

/// Converts a branch's `CellValueLiteral`s into a `MatchLiteral` — a bare
/// scalar for a single value, or a `Tuple` for multiple.
///
/// - Complexity: O(n) in `values.len()`.
fn match_literal_for(values: &[CellValueLiteral]) -> MatchLiteral {
    if values.len() == 1 {
        MatchLiteral::Scalar(literal_for(&values[0]))
    } else {
        MatchLiteral::Tuple(
            values
                .iter()
                .map(|v| MatchLiteral::Scalar(literal_for(v)))
                .collect(),
        )
    }
}

/// Returns the span [`adam_lang::fmt`]'s `write_match_literal` re-emits as
/// `values`' `.adm2` spelling: a single literal's own text for one value,
/// or the whole parenthesized, comma-joined text (e.g. `"(true, false)"`)
/// for multiple — never a placeholder, since `write_match_literal` reads
/// the *entire* rendered literal back from this one span rather than from
/// the `MatchLiteral` value itself.
///
/// - Precondition: `values` is non-empty.
///
/// - Complexity: O(n) in `values.len()`.
fn match_literal_span(values: &[CellValueLiteral]) -> ExprSpan {
    debug_assert!(!values.is_empty(), "values must be non-empty");
    if values.len() == 1 {
        ExprSpan::for_text(&literal_text(&values[0]))
    } else {
        let joined = values
            .iter()
            .map(literal_text)
            .collect::<Vec<_>>()
            .join(", ");
        ExprSpan::for_text(&format!("({joined})"))
    }
}

/// Converts one `CellValueLiteral` into `cel_parser`'s lexer-level
/// `Literal` (`= syn::Lit`), by re-lexing its `.adm2` text spelling with
/// [`cel_parser::lex_lexer::LexLexer`] — reusing the same literal-formatting
/// convention (`i64` suffixes, quoted/escaped strings) `ez-adam` already
/// relies on elsewhere.
///
/// This deliberately does not route through [`parse_expr_text`]/`Expr`:
/// `Expr::Literal`'s payload is `cel_parser::ast::Literal` (a distinct enum
/// of concrete Rust values, e.g. `I64(i64)`/`Bool(bool)`), not
/// `cel_parser::lex_lexer::Literal` (`syn::Lit`, the type
/// `adam_lang::ast::MatchLiteral::Scalar` actually wraps — see
/// `adam-lang/src/ast_parser.rs`'s `parse_match_literal`), so extracting one
/// from the other would mean hand-mapping every variant rather than
/// re-lexing the same text once.
///
/// - Precondition: `value`'s synthesized text always lexes to exactly one
///   literal token — a lex failure or non-literal token here indicates a
///   bug in this function, not bad user data, so it panics rather than
///   returning a `Result`.
fn literal_for(value: &CellValueLiteral) -> cel_parser::lex_lexer::Literal {
    let text = literal_text(value);
    let tokens: proc_macro2::TokenStream = text
        .parse()
        .unwrap_or_else(|e| panic!("synthesized literal text {text:?} failed to tokenize: {e}"));
    let mut lexer = cel_parser::lex_lexer::LexLexer::new(tokens.into_iter());
    match lexer.next() {
        Some(cel_parser::lex_lexer::Token::Literal(lit)) => lit,
        other => panic!("expected a literal token for {text:?}, got {other:?}"),
    }
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
        assert!(matches!(filter.body, cel_parser::Expr::Apply { .. }));
        assert_eq!(
            cel_parser::format_expr(&filter.body),
            "clamp(_, 0i64, 100i64)"
        );
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

    #[test]
    fn build_conditional_decl_for_a_single_bool_condition_has_two_branches() {
        use crate::model::geometry::Point;
        use crate::ops::cells::{add_cell, add_cell_node};
        use crate::ops::conditionals::add_conditional_from_bool_cells;
        use crate::ops::relationships::{create_relationship, set_member_formula};

        let mut doc = Document::new("demo");
        let flag = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group_id = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group_id, a_node, "height_pixels * 2i64");
        set_member_formula(&mut doc, group_id, b_node, "width_pixels / 2i64");
        let cond_id =
            add_conditional_from_bool_cells(&mut doc, vec![flag], group_id, Point::new(0.0, 40.0));

        let cond = &doc.conditional_groups[cond_id];
        let decl = build_conditional_decl(&doc, cond_id, cond).unwrap();
        assert_eq!(decl.branches.len(), 2);
        assert!(decl.default.is_some());
    }

    #[test]
    fn build_conditional_decl_for_a_multi_cell_condition_uses_tuple_match_literals() {
        use crate::model::geometry::Point;
        use crate::ops::cells::{add_cell, add_cell_node};
        use crate::ops::conditionals::add_conditional_from_bool_cells;
        use crate::ops::relationships::{create_relationship, set_member_formula};

        let mut doc = Document::new("demo");
        let flag_a = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
        let flag_b = add_cell(&mut doc, "lock_aspect", CellType::Bool);
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group_id = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group_id, a_node, "height_pixels * 2i64");
        set_member_formula(&mut doc, group_id, b_node, "width_pixels / 2i64");
        let cond_id = add_conditional_from_bool_cells(
            &mut doc,
            vec![flag_a, flag_b],
            group_id,
            Point::new(0.0, 40.0),
        );

        let cond = &doc.conditional_groups[cond_id];
        let decl = build_conditional_decl(&doc, cond_id, cond).unwrap();
        assert_eq!(decl.branches.len(), 4);
        assert!(matches!(
            decl.branches[0].literal,
            adam_lang::ast::MatchLiteral::Tuple(_)
        ));
        // The bug this task's brief flags explicitly: `fmt.rs`'s
        // `write_match_literal` re-emits a `Tuple` branch's *whole* source
        // text from `literal_span` alone (see its doc comment), so the span
        // must cover the synthesized `(v0, v1)` text, not a placeholder.
        assert_eq!(
            decl.branches[0].literal_span.start.source_text().as_deref(),
            Some("(false, false)")
        );
    }
}
