//! Generates `.adm2` source text from a
//! [`crate::model::document::Document`].
//!
//! Generation is one-way: `.adm2` output is never parsed back into a
//! `Document`. See `docs/superpowers/specs/2026-08-24-ez-adam-design.md`.

use std::collections::HashSet;

use adam_lang::ast::{Sheet, SheetItem};

use crate::model::cell::CellId;
use crate::model::conditional_group::ConditionalGroupId;
use crate::model::document::Document;
use crate::model::relationship_group::RelationshipGroupId;

mod ast_builder;

#[cfg(test)]
mod identifier_contract_tests {
    use super::*;
    use crate::{model::cell::CellType, ops::cells::add_cell};

    /// Rejects invalid or duplicate export names without rejecting editing drafts.
    #[test]
    fn invalid_export_names_return_errors() {
        for name in [
            "",
            "two words",
            "a-b",
            "true",
            "42",
            "x; cell injected: i64",
        ] {
            let mut doc = Document::new("demo");
            let _ = add_cell(&mut doc, name, CellType::i64());
            assert!(generate_adm2(&doc).is_err(), "{name:?}");
            let mut doc = Document::new(name);
            let _ = add_cell(&mut doc, "x", CellType::i64());
            assert!(generate_adm2(&doc).is_err(), "sheet {name:?}");
        }
        let mut doc = Document::new("demo");
        let _ = add_cell(&mut doc, "same", CellType::i64());
        let _ = add_cell(&mut doc, "same", CellType::i64());
        assert!(generate_adm2(&doc).is_err());
    }
}

/// A reason [`generate_adm2`] could not produce `.adm2` text for a
/// [`crate::model::document::Document`].
#[derive(Debug)]
///
/// # Examples
///
/// Display an error returned by `generate_adm2(&document)` with `error.to_string()`
/// so an invalid draft produces the contextual message rather than debug internals.
pub enum ExportError {
    /// The document has invalid references, declaration order, or numeric data.
    InvalidDocument(String),
    /// A sheet or cell name is not a single non-reserved expression identifier.
    InvalidIdentifier {
        /// Identifies the named declaration.
        kind: &'static str,
        /// Contains the rejected draft.
        name: String,
    },
    /// Two cell declarations use the same name.
    DuplicateCellName(String),
    /// `group`'s binding for `cell` is not valid CEL (e.g. still empty).
    InvalidFormula {
        /// The relationship group containing the invalid formula.
        group: RelationshipGroupId,
        /// The cell whose formula is invalid.
        cell: CellId,
        /// The parse error that occurred.
        source: cel_parser::ParseError,
    },
    /// `conditional`'s `Formula`-mode condition expression is not valid CEL.
    InvalidCondition {
        /// The conditional group with the invalid condition.
        conditional: ConditionalGroupId,
        /// The parse error that occurred.
        source: cel_parser::ParseError,
    },
    /// `conditional` matches on more than one value per branch (a
    /// malformed `Formula` branch key, or a multi-cell `Cells`-mode condition
    /// with a non-empty `default`) —
    /// `adam_lang`'s conditional-branch grammar only accepts a single,
    /// optionally negated literal per branch key, and no general codegen
    /// strategy exists yet for this case. See
    /// <https://github.com/stlab/cel-rs/issues/173>.
    UnsupportedMultiValueCondition {
        /// The conditional group with the unsupported condition.
        conditional: ConditionalGroupId,
    },
    /// `conditional` has a branch keyed on an `i64` value that
    /// `adam_lang`'s conditional-branch grammar can't represent. Its
    /// `["-"] literal` form stores the sign separately from an *unsigned*
    /// literal token, and `i64::MIN`'s magnitude (`9223372036854775808`) is
    /// out of range for an `i64` literal, so the emitted key would fail to
    /// parse. `i64::MIN` is the only such value. See
    /// <https://github.com/stlab/cel-rs/issues/175>.
    UnrepresentableBranchLiteral {
        /// The conditional group with the unrepresentable branch key.
        conditional: ConditionalGroupId,
        /// The offending branch value (always `i64::MIN`).
        value: i64,
    },
    /// The cell named `cell_name` has a non-finite (`NaN`/`±inf`) `f64`
    /// clamp bound. `.adm2` has no literal for a non-finite float — Debug
    /// formatting emits bare `NaN`/`inf` tokens, which parse as identifiers
    /// rather than numeric literals — so the bound can't be exported.
    NonFiniteClampBound {
        /// The name of the cell whose clamp bound is non-finite.
        cell_name: String,
        /// The offending bound value.
        bound: f64,
    },
    /// The cell named `cell_name` has an `i64::MIN` clamp bound. `.adm2`'s
    /// `["-"] literal` grammar stores the sign separately from an
    /// *unsigned* literal token, and `i64::MIN`'s magnitude
    /// (`9223372036854775808`) is out of range for an `i64` literal, so the
    /// synthesized clamp expression would fail to parse. `i64::MIN` is the
    /// only such value. See <https://github.com/stlab/cel-rs/issues/175>.
    UnrepresentableClampBound {
        /// The name of the cell whose clamp bound is `i64::MIN`.
        cell_name: String,
        /// The offending bound value (always `i64::MIN`).
        bound: i64,
    },
}

impl std::fmt::Display for ExportError {
    /// Formats a contextual export failure for the editor.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::InvalidDocument(message) => write!(f, "invalid document: {message}"),
            ExportError::InvalidIdentifier { kind, name } => {
                write!(f, "invalid {kind} identifier: {name:?}")
            }
            ExportError::DuplicateCellName(name) => {
                write!(f, "duplicate cell identifier: {name:?}")
            }
            ExportError::InvalidFormula {
                group,
                cell,
                source,
            } => write!(
                f,
                "invalid formula for cell {cell:?} in relationship group {group:?}: {source}"
            ),
            ExportError::InvalidCondition {
                conditional,
                source,
            } => write!(
                f,
                "invalid condition expression in conditional group {conditional:?}: {source}"
            ),
            ExportError::UnsupportedMultiValueCondition { conditional } => write!(
                f,
                "conditional group {conditional:?} matches on more than one value per branch, which .adm2 cannot represent"
            ),
            ExportError::UnrepresentableBranchLiteral { conditional, value } => write!(
                f,
                "conditional group {conditional:?} has a branch keyed on {value}, which .adm2's branch grammar cannot represent"
            ),
            ExportError::NonFiniteClampBound { cell_name, bound } => write!(
                f,
                "cell `{cell_name}` has a non-finite f64 clamp bound ({bound}), which .adm2 cannot represent"
            ),
            ExportError::UnrepresentableClampBound { cell_name, bound } => write!(
                f,
                "cell `{cell_name}` has a clamp bound of {bound}, which .adm2's literal grammar cannot represent"
            ),
        }
    }
}

impl std::error::Error for ExportError {
    /// Returns the underlying formula parse failure, when present.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ExportError::InvalidFormula { source, .. }
            | ExportError::InvalidCondition { source, .. } => Some(source),
            ExportError::InvalidDocument(_)
            | ExportError::InvalidIdentifier { .. }
            | ExportError::DuplicateCellName(_)
            | ExportError::UnsupportedMultiValueCondition { .. }
            | ExportError::UnrepresentableBranchLiteral { .. }
            | ExportError::NonFiniteClampBound { .. }
            | ExportError::UnrepresentableClampBound { .. } => None,
        }
    }
}

/// Returns `.adm2` source text for `doc`, by constructing an
/// `adam_lang::ast::Sheet` and rendering it via the shared
/// `adam_lang::format_sheet` — the same formatter `adam-fmt`/the VS Code
/// extension already use.
///
/// # Errors
///
/// Returns [`ExportError`] for invalid document references/order/numeric data,
/// malformed or duplicate identifiers, invalid CEL formulas, unsupported
/// multi-value conditions, or unrepresentable branch literals.
///
/// - Complexity: expected O(d + p + e) time and space for document contents d,
///   name/formula parsing work p, and emitted syntax/text e. A group enabled in
///   several branches is parsed and emitted separately at every occurrence.
///
/// # Examples
///
/// ```
/// let doc = ez_adam::model::document::Document::new("example");
/// assert!(ez_adam::codegen::generate_adm2(&doc)?.contains("sheet example"));
/// # Ok::<(), ez_adam::codegen::ExportError>(())
/// ```
pub fn generate_adm2(doc: &Document) -> Result<String, ExportError> {
    validate_export_names(doc)?;
    crate::persistence::validate_document(doc)
        .map_err(|error| ExportError::InvalidDocument(error.to_string()))?;
    Ok(adam_lang::format_sheet(&build_sheet(doc)?, ""))
}

/// Validates names before constructing spans or synthesizing expressions.
///
/// # Errors
///
/// Returns contextual errors for malformed/reserved identifiers or duplicate cell names.
///
/// - Complexity: O(t + c) expected time and O(c) space for name bytes t and cell count c.
fn validate_export_names(doc: &Document) -> Result<(), ExportError> {
    /// Accepts exactly one CEL identifier, including Unicode identifiers supported by CEL.
    ///
    /// # Errors
    ///
    /// Returns `InvalidIdentifier` when the entire input is not one identifier.
    ///
    /// - Complexity: O(name.len()) time and space.
    fn identifier(name: &str, kind: &'static str) -> Result<(), ExportError> {
        let mut parser =
            cel_parser::Parser::<cel_parser::AstContext>::new(cel_parser::OpLookup::new());
        if matches!(parser.parse_str_ast(name), Ok(cel_parser::Expr::Ident { name: parsed, .. }) if parsed == name)
        {
            Ok(())
        } else {
            Err(ExportError::InvalidIdentifier {
                kind,
                name: name.to_owned(),
            })
        }
    }
    identifier(&doc.sheet_name, "sheet")?;
    let mut names = HashSet::with_capacity(doc.cells.len());
    for cell in doc.cells.values() {
        identifier(&cell.name, "cell")?;
        if !names.insert(cell.name.as_str()) {
            return Err(ExportError::DuplicateCellName(cell.name.clone()));
        }
    }
    Ok(())
}

/// Builds the `adam_lang::ast::Sheet` [`generate_adm2`] renders for `doc`.
///
/// # Errors
///
/// Propagates [`ExportError`] from [`ast_builder::build_relationship_decl`]/
/// [`ast_builder::build_conditional_decl`] for an invalid formula or
/// condition expression.
///
/// - Complexity: expected O(d + p + e) time and space for document contents,
///   parsing work, and emitted syntax; repeated group occurrences repeat parsing.
fn build_sheet(doc: &Document) -> Result<Sheet, ExportError> {
    let mut items = Vec::new();

    // `Cell.output` is intentionally not reflected here: `adam-lang`'s `out`
    // declares a *new* identifier (it can't reuse an existing `cell`'s own
    // name) and is always a derived/computed value, never a flag on a plain
    // writable cell, so `out <name> := <name>;` doesn't parse. Deferred to
    // future design work — see <https://github.com/stlab/cel-rs/issues/147>.
    for (_, cell) in doc.cells_in_order() {
        items.push(SheetItem::Cell(ast_builder::build_cell_decl(cell)?));
    }

    let owned = groups_owned_by_conditionals(doc);
    for (id, group) in doc.relationship_groups_in_order() {
        if owned.contains(&id) {
            continue;
        }
        items.push(SheetItem::Relationship(
            ast_builder::build_relationship_decl(doc, group, id)?,
        ));
    }

    for (id, cond) in doc.conditional_groups_in_order() {
        for decl in ast_builder::build_conditional_decl(doc, id, cond)? {
            items.push(SheetItem::Conditional(decl));
        }
    }

    Ok(Sheet {
        keyword_span: cel_parser::ExprSpan::for_text("sheet"),
        parameters: None,
        name: doc.sheet_name.clone(),
        name_span: cel_parser::ExprSpan::for_text(&doc.sheet_name),
        items,
        leading_comment: None,
        doc_comment: None,
        trailing_comment: None,
        blank_line_before_close: false,
        open_brace_span: cel_parser::ExprSpan::for_text("sheet"),
        span: cel_parser::ExprSpan::for_text("sheet"),
        errors: vec![],
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Multi-cell selectors support integer/text equality without tuple branch syntax or panics.
    #[test]
    fn multi_cell_conditions_support_integer_and_escaped_text_keys() {
        use crate::model::{
            cell::CellType,
            conditional_group::{
                CellValueLiteral, ConditionExpr, ConditionalBranch, ConditionalGroup,
            },
            geometry::Point,
        };
        use crate::ops::{
            cells::{add_cell, add_cell_node},
            relationships::{create_relationship, set_member_formula},
        };
        for text in ["", "quoted \"text\"\\\n雪"] {
            let mut doc = Document::new("mixed_keys");
            let a = add_cell(&mut doc, "amount", CellType::i64());
            let b = add_cell(&mut doc, "other", CellType::i64());
            let derived = add_cell(&mut doc, "derived", CellType::i64());
            let title = add_cell(&mut doc, "title", CellType::Text);
            let na = add_cell_node(&mut doc, b, Point::new(0.0, 0.0));
            let nb = add_cell_node(&mut doc, derived, Point::new(100.0, 0.0));
            let group = create_relationship(&mut doc, na, nb, Point::new(50.0, 0.0));
            set_member_formula(&mut doc, group, na, "derived");
            set_member_formula(&mut doc, group, nb, "other");
            let conditional = doc.conditional_groups.insert(ConditionalGroup {
                display_name: "mixed".into(),
                position: Point::new(50.0, 50.0),
                condition: ConditionExpr::Cells(vec![a, title]),
                branches: vec![ConditionalBranch {
                    values: vec![
                        CellValueLiteral::I64(0),
                        CellValueLiteral::Text(text.into()),
                    ],
                    enabled_groups: vec![group],
                }],
                default: vec![],
            });
            doc.conditional_group_order.push(conditional);
            let source = generate_adm2(&doc).unwrap();
            let mut parsed = adam_lang::AdamParser::new(
                adam_lang::TypeRegistry::new(),
                cel_parser::OpLookup::new(),
            )
            .parse_str(&source)
            .unwrap();
            let title = parsed.cell_names["title"].0;
            parsed.write(title, text.to_owned()).unwrap();
            let conditional = parsed.conditionals().next().unwrap();
            assert_eq!(
                parsed.conditional_active_branch(conditional).unwrap(),
                Some(0)
            );
            parsed.write(title, format!("{text} different")).unwrap();
            assert_eq!(parsed.conditional_active_branch(conditional).unwrap(), None);
            doc.conditional_groups[doc.conditional_group_order[0]].branches[0].values[0] =
                CellValueLiteral::I64(i64::MIN);
            assert!(matches!(
                generate_adm2(&doc),
                Err(ExportError::UnrepresentableBranchLiteral { .. })
            ));
        }
    }
    use crate::model::cell::CellType;
    use crate::model::geometry::Point;
    use crate::ops::cells::{add_cell, add_cell_node};
    use crate::ops::relationships::{create_relationship, set_member_formula};

    #[test]
    /// Ordered cells export their names, types, and initial values.
    fn generates_bare_cell_declarations() {
        let mut doc = Document::new("demo");
        let _ = add_cell(&mut doc, "width_pixels", CellType::i64());
        let _ = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let out = generate_adm2(&doc).expect("valid document should export cleanly");
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: i64;\n    cell aspect_ratio: f64;\n}\n"
        );
    }

    #[test]
    /// Unconditional groups export a top-level relationship block.
    fn generates_a_top_level_relationship_block() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group, a_node, "height_pixels * 2i64");
        set_member_formula(&mut doc, group, b_node, "width_pixels / 2i64");

        let out = generate_adm2(&doc).expect("valid document should export cleanly");
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: i64;\n    cell height_pixels: i64;\n    relationship {\n        width_pixels := height_pixels * 2i64;\n        height_pixels := width_pixels / 2i64;\n    }\n}\n"
        );
    }

    #[test]
    /// The current export contract preserves cells without generating out declarations.
    fn does_not_emit_an_out_decl_for_an_output_cell_yet() {
        // `out` codegen for the `output` flag is deferred — see
        // https://github.com/stlab/cel-rs/issues/147. `out` always declares a
        // new identifier and can't reuse an existing cell's own name, so
        // `out <name> := <name>;` (the original approach) doesn't parse.
        use crate::ops::cells::set_output;

        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        set_output(&mut doc, cell, true);

        let out = generate_adm2(&doc).expect("valid document should export cleanly");
        assert_eq!(out, "sheet demo {\n    cell width_pixels: i64;\n}\n");
    }

    #[test]
    /// Two-sided clamps export both bound predicates.
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
        let out = generate_adm2(&doc).expect("valid document should export cleanly");
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: i64 filter clamp(_, 0i64, 100i64);\n}\n"
        );
    }

    #[test]
    /// Lower-only clamps export only a minimum predicate.
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
        let out = generate_adm2(&doc).expect("valid document should export cleanly");
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: i64 filter max(_, 0i64);\n}\n"
        );
    }

    #[test]
    /// Upper-only clamps export only a maximum predicate.
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
        let out = generate_adm2(&doc).expect("valid document should export cleanly");
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: f64 filter min(_, 100.0);\n}\n"
        );
    }

    #[test]
    /// Boolean selector groups export their enabled relationships.
    fn generates_a_conditional_group_with_bool_condition() {
        use crate::ops::conditionals::add_conditional_from_bool_cells;

        let mut doc = Document::new("demo");
        let flag = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group, a_node, "height_pixels * 2i64");
        set_member_formula(&mut doc, group, b_node, "width_pixels / 2i64");
        let _ = add_conditional_from_bool_cells(&mut doc, vec![flag], group, Point::new(0.0, 20.0));

        let out = generate_adm2(&doc).expect("valid document should export cleanly");
        // Branches are generated in the order `add_conditional_from_bool_cells`
        // enumerated them: `combo` counts up from 0, so the all-`false`
        // combination (combo == 0) comes first, `true` (combo == 1) second.
        assert_eq!(
            out,
            "sheet demo {\n    cell constrain_proportions: bool;\n    cell width_pixels: i64;\n    cell height_pixels: i64;\n    conditional constrain_proportions {\n        false => {\n        }\n        true => {\n            relationship {\n                width_pixels := height_pixels * 2i64;\n                height_pixels := width_pixels / 2i64;\n            }\n        }\n        _ => {\n        }\n    }\n}\n"
        );
    }

    #[test]
    /// Multi-cell conditions export conjunctions for enabled truth-table rows.
    fn generates_a_conditional_group_with_a_multi_cell_condition_as_a_conjunction() {
        // `adam_lang`'s conditional-branch grammar rejects tuple branch keys
        // (`(false, true) => { ... }` is a parse error), so a multi-cell
        // `Cells` condition is decomposed into one top-level conditional per
        // non-empty branch, keyed by a boolean conjunction over the branch's
        // cell values instead of a tuple literal — see
        // `ast_builder::build_decomposed_multi_cell_conditionals`. Only the
        // all-true branch has a non-empty `enabled_groups`
        // (`add_conditional_from_bool_cells`'s contract), so exactly one
        // conditional is emitted here.
        use crate::ops::conditionals::add_conditional_from_bool_cells;

        let mut doc = Document::new("demo");
        let flag_a = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
        let flag_b = add_cell(&mut doc, "lock_aspect", CellType::Bool);
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group, a_node, "height_pixels * 2i64");
        set_member_formula(&mut doc, group, b_node, "width_pixels / 2i64");
        let _ = add_conditional_from_bool_cells(
            &mut doc,
            vec![flag_a, flag_b],
            group,
            Point::new(0.0, 20.0),
        );

        let out = generate_adm2(&doc).expect("valid document should export cleanly");
        assert_eq!(
            out,
            "sheet demo {\n    cell constrain_proportions: bool;\n    cell lock_aspect: bool;\n    cell width_pixels: i64;\n    cell height_pixels: i64;\n    conditional constrain_proportions && lock_aspect {\n        true => {\n            relationship {\n                width_pixels := height_pixels * 2i64;\n                height_pixels := width_pixels / 2i64;\n            }\n        }\n        _ => {\n        }\n    }\n}\n"
        );
    }

    #[test]
    /// Formula conditions export the authored scalar expression.
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
        set_member_formula(&mut doc, group, a_node, "height_pixels * 2i64");
        set_member_formula(&mut doc, group, b_node, "width_pixels / 2i64");

        let cond = add_conditional_with_formula(
            &mut doc,
            vec![aspect],
            "aspect_ratio > 2.0",
            Point::new(0.0, 20.0),
        );
        add_branch(&mut doc, cond, vec![CellValueLiteral::Bool(true)]);
        toggle_enabled_group(&mut doc, cond, 0, group);

        let out = generate_adm2(&doc).expect("valid document should export cleanly");
        assert_eq!(
            out,
            "sheet demo {\n    cell aspect_ratio: f64;\n    cell width_pixels: i64;\n    cell height_pixels: i64;\n    conditional aspect_ratio > 2.0 {\n        true => {\n            relationship {\n                width_pixels := height_pixels * 2i64;\n                height_pixels := width_pixels / 2i64;\n            }\n        }\n        _ => {\n        }\n    }\n}\n"
        );
    }

    #[test]
    /// Invalid formulas fail the complete document export.
    fn generate_adm2_reports_an_invalid_formula() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let _ = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        // Formulas left empty.

        let result = generate_adm2(&doc);
        assert!(matches!(result, Err(ExportError::InvalidFormula { .. })));
    }

    #[test]
    /// Export diagnostics provide readable messages and their underlying source errors.
    fn export_error_implements_display_and_error_source() {
        use std::error::Error;

        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let _ = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        // Formulas left empty, so export fails with `InvalidFormula`.
        let err = generate_adm2(&doc).expect_err("empty formulas should fail export");

        // `Display` is implemented (not just `Debug`), and the `InvalidFormula`
        // variant chains to the underlying parse error via `Error::source`.
        assert!(!err.to_string().is_empty());
        assert!(err.source().is_some());
    }
}
