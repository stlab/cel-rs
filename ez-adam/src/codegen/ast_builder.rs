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
    RelationshipDecl,
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
        require: None,
        leading_comment: None,
        doc_comment: None,
        blank_line_before: false,
        span: ExprSpan::for_text(&cell.name),
    }
}

/// Returns a hand-built, `"clamp"`-named `filter` clause clamping `ty`'s
/// value to its clamp bounds, or `None` if `ty` is `Bool`/`Text` or has no
/// bounds set. `adam-lang`'s `cell_filter` grammar
/// (`"filter" identifier ":" expression`) requires every filter clause to
/// carry a name; `"clamp"` is used uniformly here regardless of whether
/// one or both bounds are set, since every case serves the same purpose.
/// `_` inside the body implicitly refers to the candidate value being
/// conformed, deduced automatically by `adam-lang` itself. The clamp-call
/// text still uses explicit `i64` suffixes / `f64` `Debug` formatting to
/// avoid literal-type-inference ambiguity — see this function's own body —
/// and is parsed into an `Expr` to become the filter's `body` directly.
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
        name: "clamp".to_string(),
        name_span: ExprSpan::for_text("clamp"),
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

/// Builds every top-level `conditional <expr> { ... }` declaration needed
/// to express `cond` (identified by `conditional_id`, used only to label
/// parse errors, not part of the rendered output).
///
/// `adam_lang`'s conditional-branch grammar restricts branch keys to a
/// single, optionally negated `literal_pattern` — a tuple key like
/// `(false, true) => { ... }` is a parse error there (see
/// `adam-lang/src/parser.rs`'s
/// `conditional_branch_tuple_literal_key_is_error`). A multi-cell
/// `ConditionExpr::Cells` condition's truth table therefore can't be
/// expressed as a single `adam_lang` conditional: each of its non-empty
/// branches becomes its own top-level conditional here instead, keyed by a
/// synthesized boolean conjunction over that branch's cell values (e.g.
/// `flag_a && !flag_b`) rather than a tuple literal — see
/// [`build_decomposed_multi_cell_conditionals`].
///
/// # Errors
///
/// Propagates [`ExportError::InvalidFormula`] from any nested relationship
/// group's members, or returns [`ExportError::InvalidCondition`] if a
/// `Formula`-mode condition expression isn't valid CEL.
///
/// Returns [`ExportError::UnsupportedMultiValueCondition`] if a
/// `Formula`-mode condition's branches don't match on exactly one value
/// (no codegen strategy exists yet for that case — see
/// <https://github.com/stlab/cel-rs/issues/173>), or if a multi-cell
/// `Cells` condition has a non-empty `default` (unreachable in practice,
/// since `ops::conditionals::add_conditional_from_bool_cells` never
/// populates it, but not representable under the decomposed encoding if it
/// were).
///
/// - Complexity: O(n) in the total number of branches, their enabled
///   groups' members, and the default's members.
pub(crate) fn build_conditional_decl(
    doc: &Document,
    conditional_id: ConditionalGroupId,
    cond: &ConditionalGroup,
) -> Result<Vec<ConditionalDecl>, ExportError> {
    match &cond.condition {
        ConditionExpr::Cells(cells) if cells.len() > 1 => {
            build_decomposed_multi_cell_conditionals(doc, conditional_id, cond, cells)
        }
        ConditionExpr::Cells(cells) => {
            let match_expr = cells_tuple_expr(doc, cells);
            Ok(vec![build_single_conditional_decl(
                doc,
                conditional_id,
                cond,
                match_expr,
            )?])
        }
        ConditionExpr::Formula {
            expr,
            referenced_cells,
        } => {
            if referenced_cells.len() != 1 {
                return Err(ExportError::UnsupportedMultiValueCondition {
                    conditional: conditional_id,
                });
            }
            let match_expr =
                parse_expr_text(expr).map_err(|source| ExportError::InvalidCondition {
                    conditional: conditional_id,
                    source,
                })?;
            Ok(vec![build_single_conditional_decl(
                doc,
                conditional_id,
                cond,
                match_expr,
            )?])
        }
    }
}

/// Builds one `conditional <match_expr> { <literal> => {...} ... _ => {...} }`
/// declaration for a `cond` whose branches each match on exactly one
/// value — a single-cell `Cells` condition, or a `Formula` condition with
/// exactly one referenced cell.
///
/// # Errors
///
/// Propagates [`ExportError::InvalidFormula`] from any nested relationship
/// group's members. Returns
/// [`ExportError::UnrepresentableBranchLiteral`] if a branch is keyed on
/// `i64::MIN`, which `adam_lang`'s branch grammar can't spell.
///
/// - Complexity: O(n) in the total number of branches, their enabled
///   groups' members, and the default's members.
fn build_single_conditional_decl(
    doc: &Document,
    conditional_id: ConditionalGroupId,
    cond: &ConditionalGroup,
    match_expr: Expr,
) -> Result<ConditionalDecl, ExportError> {
    let mut branches = Vec::with_capacity(cond.branches.len());
    for branch in &cond.branches {
        debug_assert_eq!(
            branch.values.len(),
            1,
            "build_single_conditional_decl requires arity-1 branches"
        );
        if matches!(branch.values[0], CellValueLiteral::I64(i64::MIN)) {
            return Err(ExportError::UnrepresentableBranchLiteral {
                conditional: conditional_id,
                value: i64::MIN,
            });
        }
        let relationships = build_branch_relationships(doc, &branch.enabled_groups)?;
        let (negated, literal, literal_span) = literal_and_sign(&branch.values[0]);
        branches.push(ConditionalBranch {
            literal,
            negated,
            literal_span,
            relationships,
            leading_comment: None,
            blank_line_before: false,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: ExprSpan::for_text("_"),
            span: ExprSpan::for_text("_"),
        });
    }

    let default_relationships = build_branch_relationships(doc, &cond.default)?;

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

/// Builds one top-level `conditional <cellA> && !<cellB> && ... { true => {...} _ => {} }`
/// declaration per branch of a multi-cell `Cells` condition whose
/// `enabled_groups` is non-empty — see [`build_conditional_decl`]'s doc
/// comment for why a single tuple-keyed conditional can't express this.
/// Branches with no enabled groups contribute nothing and are skipped.
///
/// # Errors
///
/// Propagates [`ExportError::InvalidFormula`] from any nested relationship
/// group's members. Returns [`ExportError::UnsupportedMultiValueCondition`]
/// if `cond.default` is non-empty (see [`build_conditional_decl`]'s doc
/// comment).
///
/// - Precondition: every value in every branch of `cond` is
///   [`CellValueLiteral::Bool`] (guaranteed by
///   `ops::conditionals::add_conditional_from_bool_cells`, the only way to
///   construct a multi-cell `Cells` condition).
///
/// - Complexity: O(n) in the total number of branches and their enabled
///   groups' members.
fn build_decomposed_multi_cell_conditionals(
    doc: &Document,
    conditional_id: ConditionalGroupId,
    cond: &ConditionalGroup,
    cells: &[CellId],
) -> Result<Vec<ConditionalDecl>, ExportError> {
    if !cond.default.is_empty() {
        return Err(ExportError::UnsupportedMultiValueCondition {
            conditional: conditional_id,
        });
    }
    let mut decls = Vec::new();
    for branch in &cond.branches {
        if branch.enabled_groups.is_empty() {
            continue;
        }
        debug_assert_eq!(
            branch.values.len(),
            cells.len(),
            "branch arity must match cells.len()"
        );
        let conjunction_text = cells
            .iter()
            .zip(&branch.values)
            .map(|(cell_id, value)| {
                let CellValueLiteral::Bool(is_true) = value else {
                    unreachable!(
                        "multi-cell Cells conditions only ever hold Bool values (enforced by \
                         ops::conditionals::add_conditional_from_bool_cells)"
                    );
                };
                let name = &doc.cells[*cell_id].name;
                if *is_true {
                    name.clone()
                } else {
                    format!("!{name}")
                }
            })
            .collect::<Vec<_>>()
            .join(" && ");
        let match_expr = parse_expr_text(&conjunction_text).unwrap_or_else(|e| {
            panic!("synthesized condition expression {conjunction_text:?} failed to parse: {e:?}")
        });
        let relationships = build_branch_relationships(doc, &branch.enabled_groups)?;
        decls.push(ConditionalDecl {
            match_expr,
            branches: vec![ConditionalBranch {
                literal: lex_single_literal("true"),
                negated: false,
                literal_span: ExprSpan::for_text("true"),
                relationships,
                leading_comment: None,
                blank_line_before: false,
                trailing_comment: None,
                blank_line_before_close: false,
                open_brace_span: ExprSpan::for_text("_"),
                span: ExprSpan::for_text("_"),
            }],
            default: Some(DefaultBranch {
                relationships: Vec::new(),
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
        });
    }
    Ok(decls)
}

/// Builds one `RelationshipDecl` per id in `group_ids`, in order.
///
/// # Errors
///
/// Returns [`ExportError::InvalidFormula`] for the first member of any
/// referenced group whose formula text isn't valid CEL.
///
/// - Complexity: O(n) in the total number of members across every group in
///   `group_ids`.
fn build_branch_relationships(
    doc: &Document,
    group_ids: &[RelationshipGroupId],
) -> Result<Vec<RelationshipDecl>, ExportError> {
    group_ids
        .iter()
        .map(|&group_id| build_relationship_decl(doc, &doc.relationship_groups[group_id], group_id))
        .collect()
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

/// Splits `value` into `(negated, literal, literal_span)` for building an
/// `adam_lang::ast::ConditionalBranch`, whose grammar
/// (`literal_pattern = ["-"] literal.`) stores a leading `-` separately
/// from the literal token itself — a `Literal`/`syn::Lit` never carries a
/// sign. `literal`/`literal_span` always hold the *unsigned* spelling;
/// `negated` is `true` only for a negative [`CellValueLiteral::I64`].
///
/// - Precondition: `value`'s synthesized text always lexes to exactly one
///   literal token — a lex failure or non-literal token here indicates a
///   bug in this function, not bad user data, so it panics rather than
///   returning a `Result`.
/// - Precondition: `value` is not `CellValueLiteral::I64(i64::MIN)`. Its
///   magnitude (`9223372036854775808`) lexes fine here but is out of range
///   for the `i64` literal token `adam_lang` re-validates on parse; the
///   caller ([`build_single_conditional_decl`]) rejects it up front with
///   [`ExportError::UnrepresentableBranchLiteral`].
fn literal_and_sign(value: &CellValueLiteral) -> (bool, cel_parser::lex_lexer::Literal, ExprSpan) {
    let (negated, text) = match value {
        CellValueLiteral::Bool(b) => (false, b.to_string()),
        CellValueLiteral::I64(n) => (*n < 0, format!("{}i64", n.unsigned_abs())),
        CellValueLiteral::Text(s) => (false, format!("{s:?}")),
    };
    let literal = lex_single_literal(&text);
    (negated, literal, ExprSpan::for_text(&text))
}

/// Lexes `text` into `cel_parser`'s lexer-level `Literal` (`= syn::Lit`),
/// via [`cel_parser::lex_lexer::LexLexer`] — reusing the same
/// literal-formatting convention (`i64` suffixes, quoted/escaped strings)
/// `ez-adam` already relies on elsewhere.
///
/// This deliberately does not route through [`parse_expr_text`]/`Expr`:
/// `Expr::Literal`'s payload is `cel_parser::ast::Literal` (a distinct enum
/// of concrete Rust values, e.g. `I64(i64)`/`Bool(bool)`), not
/// `cel_parser::lex_lexer::Literal` (`syn::Lit`, the type
/// `adam_lang::ast::ConditionalBranch::literal` actually wraps), so
/// extracting one from the other would mean hand-mapping every variant
/// rather than re-lexing the same text once.
///
/// - Precondition: `text` lexes to exactly one literal token — a lex
///   failure or non-literal token here indicates a bug in the caller, not
///   bad user data, so it panics rather than returning a `Result`.
fn lex_single_literal(text: &str) -> cel_parser::lex_lexer::Literal {
    let tokens: proc_macro2::TokenStream = text
        .parse()
        .unwrap_or_else(|e| panic!("synthesized literal text {text:?} failed to tokenize: {e}"));
    let mut lexer = cel_parser::lex_lexer::LexLexer::new(tokens.into_iter());
    let lit = match lexer.next() {
        Some(cel_parser::lex_lexer::Token::Literal(lit)) => lit,
        other => panic!("expected a literal token for {text:?}, got {other:?}"),
    };
    debug_assert!(
        lexer.next().is_none(),
        "text {text:?} must lex to exactly one literal token"
    );
    lit
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
        let decls = build_conditional_decl(&doc, cond_id, cond).unwrap();
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].branches.len(), 2);
        assert!(decls[0].default.is_some());
    }

    #[test]
    fn build_conditional_decl_for_a_multi_cell_condition_emits_one_decl_per_enabled_branch() {
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
        let decls = build_conditional_decl(&doc, cond_id, cond).unwrap();
        // Per `add_conditional_from_bool_cells`'s contract, only the
        // all-true branch has a non-empty `enabled_groups` — every other
        // combination is empty and contributes no decl.
        assert_eq!(decls.len(), 1);
        assert_eq!(
            cel_parser::format_expr(&decls[0].match_expr),
            "constrain_proportions && lock_aspect"
        );
        assert_eq!(decls[0].branches.len(), 1);
        assert_eq!(
            decls[0].branches[0]
                .literal_span
                .start
                .source_text()
                .as_deref(),
            Some("true")
        );
        assert_eq!(decls[0].branches[0].relationships.len(), 1);
    }

    #[test]
    fn build_conditional_decl_for_a_multi_cell_condition_with_no_enabled_branches_emits_no_decls() {
        use crate::model::geometry::Point;
        use crate::ops::cells::add_cell;
        use crate::ops::conditionals::add_conditional_from_bool_cells;
        use crate::ops::relationships::create_relationship;

        let mut doc = Document::new("demo");
        let flag_a = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
        let flag_b = add_cell(&mut doc, "lock_aspect", CellType::Bool);
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = crate::ops::cells::add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = crate::ops::cells::add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group_id = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        let cond_id = add_conditional_from_bool_cells(
            &mut doc,
            vec![flag_a, flag_b],
            group_id,
            Point::new(0.0, 40.0),
        );
        // Immediately disable the one branch `add_conditional_from_bool_cells` enables.
        let enabled_branch_index = doc.conditional_groups[cond_id]
            .branches
            .iter()
            .position(|b| !b.enabled_groups.is_empty())
            .unwrap();
        crate::ops::conditionals::toggle_enabled_group(
            &mut doc,
            cond_id,
            enabled_branch_index,
            group_id,
        );

        let cond = &doc.conditional_groups[cond_id];
        let decls = build_conditional_decl(&doc, cond_id, cond).unwrap();
        assert!(decls.is_empty());
    }
}
