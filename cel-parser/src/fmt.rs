//! Pretty-prints a [`crate::Expr`] tree back to CEL source text: precedence-aware
//! parenthesization (added only where required, not exhaustively), single-space-around-operator
//! normalization, and no line-wrapping (every expression is emitted on one line regardless of
//! length — see the design doc's "Line wrapping" decision). Literal leaves are re-emitted via
//! [`proc_macro2::Span::source_text`] rather than synthesized from [`crate::Literal`], so exact
//! original notation (`1920.0` vs `1920.0f64`, a byte literal's spelling) round-trips.

use crate::ast::{Expr, LogicalOp};

/// Binding-strength level, loosest first, mirroring `lib.rs`'s grammar chain from
/// `range_expression` (via `expression = range_expression`) through `primary_expression`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Level(u8);

impl Level {
    const RANGE: Level = Level(0);
    const OR: Level = Level(1);
    const AND: Level = Level(2);
    const COMPARISON: Level = Level(3);
    const BIT_OR: Level = Level(4);
    const BIT_XOR: Level = Level(5);
    const BIT_AND: Level = Level(6);
    const SHIFT: Level = Level(7);
    const ADDITIVE: Level = Level(8);
    const MULTIPLICATIVE: Level = Level(9);
    const CAST: Level = Level(10);
    const UNARY: Level = Level(11);
    const POSTFIX: Level = Level(12);
    const PRIMARY: Level = Level(13);

    /// The next level up (strictly tighter-binding than `self`).
    fn tighter(self) -> Level {
        Level(self.0 + 1)
    }
}

/// Returns the binding-strength level of a binary (two-operand) operator.
///
/// - Precondition: `name` is one of the binary operator tokens `lib.rs`'s grammar recognizes,
///   excluding `"range"`/`"range_inclusive"` — those, and every other range-family operator
///   (`"range_from"`/`"range_to"`/`"range_to_inclusive"`/`"range_full"`), are rendered by their
///   own dedicated [`render`] match arms before any operator ever reaches this function, since
///   their printed form (`x..y`, no surrounding spaces) doesn't fit this function's shared
///   `"{lhs} {name} {rhs}"` shape the way every other binary operator here does.
fn binary_op_level(name: &str) -> Level {
    match name {
        "|" => Level::BIT_OR,
        "^" => Level::BIT_XOR,
        "&" => Level::BIT_AND,
        "<<" | ">>" => Level::SHIFT,
        "+" | "-" => Level::ADDITIVE,
        "*" | "/" | "%" => Level::MULTIPLICATIVE,
        "==" | "!=" | "<" | ">" | "<=" | ">=" => Level::COMPARISON,
        other => unreachable!("binary_op_level called with unknown operator `{other}`"),
    }
}

/// Re-emits a literal's exact original text via its span, falling back to an empty string when
/// none is recoverable (spans built without a live source file — never a real parse; see the
/// module doc).
fn render_literal(span: crate::ExprSpan) -> String {
    span.start.source_text().unwrap_or_default()
}

/// Renders a closure parameter's unresolved type expression, e.g. `"i32"` or `"(i32, f64)"`.
///
/// - Complexity: O(n) in the number of (nested) tuple elements in the type expression.
fn render_closure_param_type(type_expr: &crate::ClosureParamTypeExpr) -> String {
    match type_expr {
        crate::ClosureParamTypeExpr::Named(name, _) => name.clone(),
        crate::ClosureParamTypeExpr::Tuple(elements, _) => {
            let inner = elements
                .iter()
                .map(render_closure_param_type)
                .collect::<Vec<_>>()
                .join(", ");
            format!("({inner})")
        }
    }
}

/// Renders `expr` on its own, returning its text alongside its binding-strength level, so the
/// caller ([`format_at`]) can decide whether the context it's being placed in requires parens.
fn render(expr: &Expr) -> (String, Level) {
    match expr {
        Expr::Literal { span, .. } => (render_literal(*span), Level::PRIMARY),
        // `AstContext::apply_op` records every arity-0 operator as a plain `Expr::Ident` (see
        // its own test `apply_op_with_arity_zero_records_an_ident_node`) — `"range_full"` (`..`)
        // is the only one this grammar has, so it's intercepted here rather than in the
        // `Expr::Op` arms below, which a real parse never produces it as.
        //
        // Known gap (tracked as issue #155): this is indistinguishable from a genuine bare
        // identifier reference literally named `range_full`, since `AstContext` deliberately
        // does no scope resolution during parsing (unlike the runtime path, where
        // `OpLookup::lookup` checks user scopes before builtins and so correctly lets a real
        // variable named `range_full` shadow this operator). Narrow in practice — the other
        // five range-family operators have no such collision, since a real function call is
        // never represented as `Expr::Op`.
        Expr::Ident { name, .. } if name == "range_full" => ("..".to_string(), Level::RANGE),
        Expr::Ident { name, .. } => (name.clone(), Level::PRIMARY),
        Expr::Logical { op, lhs, rhs, .. } => {
            let level = match op {
                LogicalOp::Or => Level::OR,
                LogicalOp::And => Level::AND,
            };
            let op_str = match op {
                LogicalOp::Or => "||",
                LogicalOp::And => "&&",
            };
            let lhs_s = format_at(lhs, level);
            let rhs_s = format_at(rhs, level.tighter());
            (format!("{lhs_s} {op_str} {rhs_s}"), level)
        }
        Expr::Op { operands, .. } if operands.is_empty() => {
            // Defensive only: a real parse never produces an arity-0 `Expr::Op` (see the
            // `Expr::Ident` arm above for how `"range_full"` actually arrives), but a hand-built
            // tree using this representation still round-trips correctly, matching this file's
            // existing convention of also handling non-parser-producible tree shapes (e.g.
            // `nested_comparison_needs_parens_on_both_sides`).
            ("..".to_string(), Level::RANGE)
        }
        Expr::Op { name, operands, .. }
            if operands.len() == 1
                && matches!(
                    name.as_str(),
                    "range_from" | "range_to" | "range_to_inclusive"
                ) =>
        {
            // Range's own endpoints are always `or_expression`s (never chained further range
            // expressions — see `is_range_expression`'s doc comment), so an operand only needs
            // rendering strictly tighter than Range itself, the same non-chaining treatment
            // `Level::COMPARISON` gets below.
            let operand_s = format_at(&operands[0], Level::RANGE.tighter());
            let text = match name.as_str() {
                "range_from" => format!("{operand_s}.."),
                "range_to" => format!("..{operand_s}"),
                "range_to_inclusive" => format!("..={operand_s}"),
                _ => unreachable!("guarded by the outer match arm"),
            };
            (text, Level::RANGE)
        }
        Expr::Op { name, operands, .. } if operands.len() == 1 => {
            let operand_s = format_at(&operands[0], Level::UNARY);
            // A bare "-"/"!" glued directly onto an operand that itself starts with "-"/"!"
            // would re-tokenize as one run of punctuation; a single space disambiguates.
            let sep = if operand_s.starts_with('-') || operand_s.starts_with('!') {
                " "
            } else {
                ""
            };
            (format!("{name}{sep}{operand_s}"), Level::UNARY)
        }
        Expr::Op { name, operands, .. } if name == "range" || name == "range_inclusive" => {
            // Same non-chaining reasoning as the arity-1 range arm above: both endpoints render
            // strictly tighter than Range.
            let lhs_s = format_at(&operands[0], Level::RANGE.tighter());
            let rhs_s = format_at(&operands[1], Level::RANGE.tighter());
            let op_str = if name == "range_inclusive" {
                "..="
            } else {
                ".."
            };
            (format!("{lhs_s}{op_str}{rhs_s}"), Level::RANGE)
        }
        Expr::Op { name, operands, .. } => {
            let level = binary_op_level(name);
            // Comparison can't chain — the grammar parses at most one per comparison_expression —
            // so both operands must be strictly tighter than Comparison itself, unlike the other,
            // left-associative (chaining) binary levels below, where only the right operand does.
            let (lhs_min, rhs_min) = if level == Level::COMPARISON {
                (level.tighter(), level.tighter())
            } else {
                (level, level.tighter())
            };
            let lhs_s = format_at(&operands[0], lhs_min);
            let rhs_s = format_at(&operands[1], rhs_min);
            (format!("{lhs_s} {name} {rhs_s}"), level)
        }
        Expr::Cast {
            expr, type_name, ..
        } => {
            // Left-associative, like multiplicative/additive: the operand only needs to be at
            // least as tight as Cast itself, so a chain like `x as i32 as f64` reprints without
            // extra parens.
            let expr_s = format_at(expr, Level::CAST);
            (format!("{expr_s} as {type_name}"), Level::CAST)
        }
        Expr::Apply { callee, args, .. } => {
            let callee_s = format_at(callee, Level::POSTFIX);
            let args_s = args
                .iter()
                .map(|a| format_at(a, Level::OR))
                .collect::<Vec<_>>()
                .join(", ");
            (format!("{callee_s}({args_s})"), Level::POSTFIX)
        }
        Expr::Tuple { elements, .. } => {
            let inner = elements
                .iter()
                .map(|e| format_at(e, Level::OR))
                .collect::<Vec<_>>()
                .join(", ");
            let text = if elements.len() == 1 {
                format!("({inner},)")
            } else {
                format!("({inner})")
            };
            (text, Level::PRIMARY)
        }
        Expr::TupleIndex { base, index, .. } => (
            format!("{}.{}", format_at(base, Level::POSTFIX), index),
            Level::POSTFIX,
        ),
        Expr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            let cond_s = format_at(cond, Level::OR);
            let then_s = format_at(then_branch, Level::OR);
            let mut text = format!("if {cond_s} {{ {then_s} }}");
            if let Some(else_branch) = else_branch {
                if matches!(else_branch.as_ref(), Expr::If { .. }) {
                    let (else_s, _) = render(else_branch);
                    text.push_str(&format!(" else {else_s}"));
                } else {
                    let else_s = format_at(else_branch, Level::OR);
                    text.push_str(&format!(" else {{ {else_s} }}"));
                }
            }
            (text, Level::PRIMARY)
        }
        Expr::Closure { params, body, .. } => {
            let body_s = format_at(body, Level::OR);
            let text = if params.is_empty() {
                format!("|| {body_s}")
            } else {
                let params_s = params
                    .iter()
                    .map(|p| format!("{}: {}", p.name, render_closure_param_type(&p.type_expr)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("|{params_s}| {body_s}")
            };
            (text, Level::PRIMARY)
        }
    }
}

/// Renders `expr`, wrapping it in parens if its own level is looser than `min_level` requires.
fn format_at(expr: &Expr, min_level: Level) -> String {
    let (text, level) = render(expr);
    if level < min_level {
        format!("({text})")
    } else {
        text
    }
}

/// Pretty-prints `expr` back to CEL source text — see the module doc for the printing rules.
///
/// # Examples
///
/// ```
/// use cel_parser::{AstContext, OpLookup, Parser, format_expr};
///
/// let mut parser = Parser::<AstContext>::new(OpLookup::new());
/// let expr = parser.parse_str_ast("(1i32 + 2i32) * 3i32").unwrap();
/// assert_eq!(format_expr(&expr), "(1i32 + 2i32) * 3i32");
/// ```
pub fn format_expr(expr: &Expr) -> String {
    format_at(expr, Level::RANGE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AstContext, OpLookup, Parser};

    fn parse(source: &str) -> Expr {
        Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap()
    }

    #[test]
    fn additive_and_multiplicative_reprint_without_extra_parens() {
        let expr = parse("1i32 + 2i32 * 3i32");
        assert_eq!(format_expr(&expr), "1i32 + 2i32 * 3i32");
    }

    #[test]
    fn explicit_grouping_that_changes_precedence_keeps_its_parens() {
        let expr = parse("(1i32 + 2i32) * 3i32");
        assert_eq!(format_expr(&expr), "(1i32 + 2i32) * 3i32");
    }

    #[test]
    fn left_associative_chain_at_the_same_precedence_has_no_parens() {
        let expr = parse("1i32 - 2i32 - 3i32");
        assert_eq!(format_expr(&expr), "1i32 - 2i32 - 3i32");
    }

    #[test]
    fn a_right_leaning_tree_at_the_same_precedence_needs_parens() {
        // Not producible by real parsing (the grammar's additive_expression loop is always
        // left-associative) — built by hand to prove the printer round-trips a tree shape it
        // didn't itself produce. Uses Ident operands (rendered from `name`, not a span) so the
        // assertion reads as real text rather than the no-source-text fallback.
        fn ident(name: &str) -> Expr {
            Expr::Ident {
                name: name.to_string(),
                span: point(),
            }
        }
        fn point() -> crate::ExprSpan {
            crate::ExprSpan {
                start: proc_macro2::Span::call_site(),
                end: proc_macro2::Span::call_site(),
            }
        }
        let expr = Expr::Op {
            name: "-".to_string(),
            operands: vec![
                ident("a"),
                Expr::Op {
                    name: "-".to_string(),
                    operands: vec![ident("b"), ident("c")],
                    span: point(),
                },
            ],
            span: point(),
        };
        assert_eq!(format_expr(&expr), "a - (b - c)");
    }

    #[test]
    fn nested_comparison_needs_parens_on_both_sides() {
        // Also not producible by real parsing (comparison_expression allows at most one
        // comparison, never a nested one) — proves format_expr stays reparseable even for a
        // hand-built tree shape the grammar itself can't emit.
        fn ident(name: &str) -> Expr {
            Expr::Ident {
                name: name.to_string(),
                span: crate::ExprSpan {
                    start: proc_macro2::Span::call_site(),
                    end: proc_macro2::Span::call_site(),
                },
            }
        }
        let inner = Expr::Op {
            name: "==".to_string(),
            operands: vec![ident("a"), ident("b")],
            span: crate::ExprSpan {
                start: proc_macro2::Span::call_site(),
                end: proc_macro2::Span::call_site(),
            },
        };
        let expr = Expr::Op {
            name: "==".to_string(),
            operands: vec![inner, ident("c")],
            span: crate::ExprSpan {
                start: proc_macro2::Span::call_site(),
                end: proc_macro2::Span::call_site(),
            },
        };
        assert_eq!(format_expr(&expr), "(a == b) == c");
    }

    #[test]
    fn literal_notation_is_preserved_exactly() {
        assert_eq!(format_expr(&parse("1920.0")), "1920.0");
        assert_eq!(format_expr(&parse("1920.0f64")), "1920.0f64");
        assert_eq!(format_expr(&parse("1i32")), "1i32");
    }

    #[test]
    fn unary_minus_of_a_binary_expression_needs_parens() {
        assert_eq!(format_expr(&parse("-(1i32 + 2i32)")), "-(1i32 + 2i32)");
    }

    #[test]
    fn double_unary_minus_keeps_a_separating_space() {
        assert_eq!(format_expr(&parse("- -1i32")), "- -1i32");
    }

    #[test]
    fn cast_chain_reprints_without_extra_parens() {
        assert_eq!(format_expr(&parse("x as i32 as f64")), "x as i32 as f64");
    }

    #[test]
    fn unary_minus_before_a_cast_needs_no_parens() {
        // Matches Rust: `-x as f64` parses as `(-x) as f64` - unary already binds tighter than
        // Cast, so the printer doesn't need to add parens to preserve that grouping.
        assert_eq!(format_expr(&parse("-x as f64")), "-x as f64");
    }

    #[test]
    fn explicit_grouping_before_a_cast_keeps_its_parens() {
        // `as` binds tighter than `+`, so without parens `(a + b) as i32` would reprint as
        // `a + b as i32` - a different expression (`a + (b as i32)`). The parens must survive.
        assert_eq!(format_expr(&parse("(a + b) as i32")), "(a + b) as i32");
    }

    #[test]
    fn cast_operand_of_an_additive_expression_needs_no_parens() {
        // `a + b as i32` already parses as `a + (b as i32)` (`as` binds tighter than `+`), so no
        // parens are needed around the cast when reprinting.
        assert_eq!(format_expr(&parse("a + b as i32")), "a + b as i32");
    }

    #[test]
    fn explicit_grouping_around_a_cast_before_multiplicative_is_redundant_and_dropped() {
        // `(x as i32) * y` parses to the exact same tree as `x as i32 * y` (cast already binds
        // tighter than `*`), so the now-redundant parens are dropped on reprint - matching the
        // module doc's "parens added only where required, not exhaustively".
        assert_eq!(format_expr(&parse("(x as i32) * y")), "x as i32 * y");
    }

    #[test]
    fn one_tuple_keeps_its_trailing_comma() {
        assert_eq!(format_expr(&parse("(1i32,)")), "(1i32,)");
    }

    #[test]
    fn multi_element_tuple_has_no_trailing_comma() {
        assert_eq!(format_expr(&parse("(1i32, 2i32)")), "(1i32, 2i32)");
    }

    #[test]
    fn if_without_else_omits_the_else_clause() {
        assert_eq!(format_expr(&parse("if true { 1i32 }")), "if true { 1i32 }");
    }

    #[test]
    fn if_else_reprints_both_branches() {
        assert_eq!(
            format_expr(&parse("if true { 1i32 } else { 2i32 }")),
            "if true { 1i32 } else { 2i32 }"
        );
    }

    #[test]
    fn else_if_chain_has_no_braces_around_the_nested_if() {
        let source = "if true { 1i32 } else if false { 2i32 } else { 3i32 }";
        assert_eq!(format_expr(&parse(source)), source);
    }

    #[test]
    fn logical_or_and_and_are_not_desugared_and_need_no_extra_parens() {
        assert_eq!(format_expr(&parse("a || b && c")), "a || b && c");
    }

    #[test]
    fn format_is_idempotent_through_a_reparse() {
        let source = "(1i32 + 2i32) * 3i32 - -4i32";
        let once = format_expr(&parse(source));
        let twice = format_expr(&parse(&once));
        assert_eq!(once, twice);
    }

    #[test]
    fn closure_with_one_param_reprints_with_its_type() {
        assert_eq!(
            format_expr(&parse("|x: i32| x + 1i32")),
            "|x: i32| x + 1i32"
        );
    }

    #[test]
    fn closure_with_no_params_reprints_with_double_pipe() {
        assert_eq!(format_expr(&parse("|| 1i32")), "|| 1i32");
    }

    #[test]
    fn closure_with_multiple_params_joins_them_with_commas() {
        assert_eq!(
            format_expr(&parse("|x: i32, y: i32| x + y")),
            "|x: i32, y: i32| x + y"
        );
    }

    #[test]
    fn closure_with_a_tuple_typed_param_reprints_the_tuple_type() {
        assert_eq!(
            format_expr(&parse("|x: (i32, f64)| x.0")),
            "|x: (i32, f64)| x.0"
        );
    }

    #[test]
    fn range_inclusive_reprints_without_spaces() {
        assert_eq!(format_expr(&parse("1i32..=5i32")), "1i32..=5i32");
    }

    #[test]
    fn range_reprints_without_spaces() {
        assert_eq!(format_expr(&parse("1i32..5i32")), "1i32..5i32");
    }

    #[test]
    fn range_from_reprints_without_spaces() {
        assert_eq!(format_expr(&parse("1i32..")), "1i32..");
    }

    #[test]
    fn range_to_reprints_without_spaces() {
        assert_eq!(format_expr(&parse("..5i32")), "..5i32");
    }

    #[test]
    fn range_to_inclusive_reprints_without_spaces() {
        assert_eq!(format_expr(&parse("..=5i32")), "..=5i32");
    }

    #[test]
    fn range_full_reprints_as_two_dots() {
        assert_eq!(format_expr(&parse("..")), "..");
    }

    #[test]
    fn range_endpoints_that_are_comparisons_need_no_parens() {
        // From `is_range_expression`'s own doc comment: `a == b..c == d` parses as
        // `(a == b)..(c == d)` -- comparison already binds tighter than range, so the printer
        // doesn't need parens to preserve that grouping.
        assert_eq!(format_expr(&parse("a == b..c == d")), "a == b..c == d");
    }

    #[test]
    fn range_endpoints_that_are_arithmetic_need_no_parens() {
        // From the same doc comment: `1 + 2..3 * 4` parses as `(1 + 2)..(3 * 4)` -- arithmetic
        // binds well inside range's own endpoints, so no parens are needed on reprint either.
        assert_eq!(
            format_expr(&parse("1i32 + 2i32..3i32 * 4i32")),
            "1i32 + 2i32..3i32 * 4i32"
        );
    }
}
