//! [`PmAstParser`]: parses pm-lang source into [`crate::ast::Sheet`] instead of executing into a
//! live `property_model::Sheet`. Shares [`crate::token_cursor::TokenCursor`] with
//! [`crate::PmParser`] for pure tokenizing; the two parsers' grammar-production functions are
//! separate because what each one builds is genuinely different (see this plan's Architecture
//! section).

use cel_parser::{AstContext, OpLookup, Parser as CelParser};

use crate::ast;
use crate::token_cursor::TokenCursor;

/// Parser result type, matching `cel_parser::ParseError`.
pub type Result<T> = std::result::Result<T, cel_parser::ParseError>;

/// Parses pm-lang source strings into [`ast::Sheet`] trees, instead of executing into a live
/// `property_model::Sheet` (see [`crate::PmParser`] for that path).
///
/// # Example
///
/// ```rust
/// use pm_lang::PmAstParser;
///
/// let sheet = PmAstParser::new().parse_str("sheet s { cell x: i32 = 0; }").unwrap();
/// assert_eq!(sheet.name, "s");
/// ```
pub struct PmAstParser {
    cel: CelParser<AstContext>,
}

impl Default for PmAstParser {
    fn default() -> Self {
        Self::new()
    }
}

impl PmAstParser {
    /// Creates a new AST-building parser.
    ///
    /// Unlike [`crate::PmParser::new`], this takes no `TypeRegistry`/`OpLookup` — `AstContext`
    /// resolves no identifiers and validates nothing during parsing (see this plan's Architecture
    /// section), so there is nothing for either to configure.
    #[must_use]
    pub fn new() -> Self {
        PmAstParser {
            cel: CelParser::new(OpLookup::new()),
        }
    }

    /// Parses a pm-lang source string into an [`ast::Sheet`].
    ///
    /// A syntax error inside one `cell`/`relationship`/`conditional` item is recorded in
    /// `Sheet.errors` and replaced by a `SheetItem::Error` placeholder covering the skipped
    /// tokens; parsing resumes at the next sheet item instead of aborting (see
    /// [`TokenCursor::skip_to_recovery_point`]). This recovery is declaration-level only: a
    /// malformed `method_decl` inside a `relationship`/`conditional` block causes the whole
    /// enclosing item to become one `SheetItem::Error`.
    ///
    /// Recovery is reliable for syntax errors pm-lang's own grammar detects directly (malformed
    /// `cell` declarations; `relationship`/`conditional`/`method_decl` structure outside their CEL
    /// expression bodies) and for CEL expression errors that don't leave an unbalanced delimiter of
    /// a kind CEL also uses for its own internal grouping — the common case, including a dangling
    /// unmatched `)` (e.g. `(+)`). It is **not** guaranteed when a CEL expression's failure leaves
    /// a dangling, unmatched delimiter of a kind CEL reuses for its own internal structure — e.g.
    /// an `if`/`else` expression's braces, which are the same `Delimiter::Brace` kind pm-lang uses
    /// for its own `relationship`/`conditional`/method bodies (`if a { }` is one such case). In
    /// that narrower case recovery may abort the entire parse (returning `Err`) rather than
    /// isolating the one malformed item; see [`TokenCursor::skip_to_recovery_point`]'s doc comment
    /// for why a kind-based fix can't close this in general, and the tracking issue for the
    /// general fix.
    ///
    /// # Errors
    ///
    /// Returns `Err` for structural errors outside any sheet item (e.g. a missing `sheet`
    /// keyword, missing sheet name, missing top-level braces, or trailing tokens after the
    /// sheet closes) — these can't be attributed to a single recoverable item. Also returns `Err`
    /// in the known-limitation case described above.
    pub fn parse_str(&mut self, source: &str) -> Result<ast::Sheet> {
        use std::str::FromStr;
        let stream = proc_macro2::TokenStream::from_str(source)
            .map_err(|e| cel_parser::ParseError::new(e.to_string(), e.span()))?;
        let mut cursor =
            TokenCursor::new(cel_parser::lex_lexer::LexLexer::new(stream.into_iter()).peekable());
        let sheet = self.parse_sheet(&mut cursor)?;
        if let Some(tok) = cursor.peek_token() {
            use cel_parser::lex_lexer::HasSpan;
            return Err(cel_parser::ParseError::new("unexpected token", tok.span()));
        }
        Ok(sheet)
    }

    /// `sheet = "sheet" identifier "{" { sheet_item } "}".`
    fn parse_sheet(&mut self, cursor: &mut TokenCursor) -> Result<ast::Sheet> {
        let sheet_start = cursor.peek_span();
        if !cursor.is_keyword("sheet") {
            return Err(cursor.err_at("expected `sheet`"));
        }
        let (name, name_span) = cursor.consume_ident()?;
        cursor.expect_open_brace()?;
        let mut items = Vec::new();
        let mut errors = Vec::new();
        while !cursor.at_close_brace() {
            let item_start = cursor.peek_span();
            let target_depth = cursor.depth();
            match self.parse_sheet_item(cursor) {
                Ok(item) => items.push(item),
                Err(e) => {
                    errors.push(e);
                    let item_end = cursor.skip_to_recovery_point(target_depth);
                    items.push(ast::SheetItem::Error {
                        span: ast::ExprSpan {
                            start: item_start,
                            end: item_end,
                        },
                    });
                }
            }
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::Sheet {
            name,
            name_span: point(name_span),
            items,
            span: ast::ExprSpan {
                start: sheet_start,
                end: close_span,
            },
            errors,
        })
    }

    /// `sheet_item = cell_decl | relationship_decl | conditional_decl.`
    fn parse_sheet_item(&mut self, cursor: &mut TokenCursor) -> Result<ast::SheetItem> {
        use cel_parser::lex_lexer::{HasSpan, Token};
        match cursor.peek_token() {
            Some(Token::Identifier(id)) if id == "cell" => {
                self.parse_cell_decl(cursor).map(ast::SheetItem::Cell)
            }
            Some(Token::Identifier(id)) if id == "relationship" => self
                .parse_relationship_decl(cursor)
                .map(ast::SheetItem::Relationship),
            Some(Token::Identifier(id)) if id == "conditional" => self
                .parse_conditional_decl(cursor)
                .map(ast::SheetItem::Conditional),
            Some(tok) => Err(cel_parser::ParseError::new(
                "expected `cell`, `relationship`, or `conditional`",
                tok.span(),
            )),
            None => Err(cel_parser::ParseError::new(
                "unexpected end of input",
                proc_macro2::Span::call_site(),
            )),
        }
    }

    /// `cell_decl = "cell" identifier cell_type_init ";".`
    fn parse_cell_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::CellDecl> {
        let decl_start = cursor.peek_span();
        cursor.is_keyword("cell");
        let (name, name_span) = cursor.consume_ident()?;
        let (type_name, initializer) = if cursor.consume_punct(":") {
            let (type_name, type_span) = cursor.consume_ident()?;
            let initializer = if cursor.consume_punct("=") {
                let (lit, lit_span) = cursor.consume_literal()?;
                Some((lit, point(lit_span)))
            } else {
                None
            };
            (Some((type_name, point(type_span))), initializer)
        } else if cursor.consume_punct("=") {
            let (lit, lit_span) = cursor.consume_literal()?;
            (None, Some((lit, point(lit_span))))
        } else {
            return Err(cursor.err_at("expected `:` or `=` in cell declaration"));
        };
        let semi_span = cursor.expect_punct(";")?;
        Ok(ast::CellDecl {
            name,
            name_span: point(name_span),
            type_name,
            initializer,
            leading_comment: None,
            span: ast::ExprSpan {
                start: decl_start,
                end: semi_span,
            },
        })
    }

    /// `relationship_decl = "relationship" [ identifier ] "{" { method_decl } "}".`
    fn parse_relationship_decl(
        &mut self,
        cursor: &mut TokenCursor,
    ) -> Result<ast::RelationshipDecl> {
        use cel_parser::lex_lexer::Token;
        let decl_start = cursor.peek_span();
        cursor.is_keyword("relationship");
        let name = if matches!(cursor.peek_token(), Some(Token::Identifier(_))) {
            let (n, s) = cursor.consume_ident()?;
            Some((n, point(s)))
        } else {
            None
        };
        cursor.expect_open_brace()?;
        let mut methods = Vec::new();
        while !cursor.at_close_brace() {
            methods.push(self.parse_method_decl(cursor)?);
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::RelationshipDecl {
            name,
            methods,
            leading_comment: None,
            span: ast::ExprSpan {
                start: decl_start,
                end: close_span,
            },
        })
    }

    /// `conditional_decl = "conditional" identifier "{" { conditional_branch } [ default_branch ] "}".`
    fn parse_conditional_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::ConditionalDecl> {
        use cel_parser::lex_lexer::Token;
        let decl_start = cursor.peek_span();
        cursor.is_keyword("conditional");
        let (match_name, match_span) = cursor.consume_ident()?;
        cursor.expect_open_brace()?;
        let mut branches = Vec::new();
        let mut default = None;
        while !cursor.at_close_brace() {
            if matches!(cursor.peek_token(), Some(Token::Identifier(id)) if id == "_") {
                cursor.advance();
                cursor.expect_punct("=>")?;
                cursor.expect_open_brace()?;
                let mut methods = Vec::new();
                while !cursor.at_close_brace() {
                    methods.push(self.parse_method_decl(cursor)?);
                }
                cursor.expect_close_brace()?;
                cursor.consume_punct(",");
                default = Some(methods);
                break; // default branch is always last
            }
            let (lit, lit_span) = cursor.consume_literal()?;
            cursor.expect_punct("=>")?;
            cursor.expect_open_brace()?;
            let mut methods = Vec::new();
            while !cursor.at_close_brace() {
                methods.push(self.parse_method_decl(cursor)?);
            }
            let close = cursor.expect_close_brace()?;
            cursor.consume_punct(",");
            branches.push(ast::ConditionalBranch {
                literal: lit,
                literal_span: point(lit_span),
                methods,
                span: ast::ExprSpan {
                    start: lit_span,
                    end: close,
                },
            });
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::ConditionalDecl {
            match_name,
            match_name_span: point(match_span),
            branches,
            default,
            leading_comment: None,
            span: ast::ExprSpan {
                start: decl_start,
                end: close_span,
            },
        })
    }

    /// `method_decl = "method" cell_list "->" cell_list method_body.`
    fn parse_method_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::MethodDecl> {
        let decl_start = cursor.peek_span();
        if !cursor.is_keyword("method") {
            return Err(cursor.err_at("expected `method`"));
        }
        let inputs = parse_cell_list(cursor)?;
        cursor.expect_punct("->")?;
        let outputs = parse_cell_list(cursor)?;
        cursor.expect_open_brace()?;
        let body = self.parse_cel_or_expression(cursor)?;
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::MethodDecl {
            inputs,
            outputs,
            body,
            span: ast::ExprSpan {
                start: decl_start,
                end: close_span,
            },
        })
    }

    /// Delegates one `or_expression` to `cel_parser::Parser<AstContext>`, sharing the token
    /// stream (the same take/set-tokens handoff `crate::PmParser` uses for the `DynSegment`
    /// path).
    fn parse_cel_or_expression(&mut self, cursor: &mut TokenCursor) -> Result<cel_parser::Expr> {
        let tokens = cursor.take_tokens().expect("tokens present");
        self.cel.set_lex_tokens(tokens);
        let result = self.cel.parse_or_expression_ast();
        cursor.set_tokens(self.cel.take_lex_tokens().expect("tokens set"));
        result
    }
}

/// `cell_list = "[" identifier { "," identifier } "]".`
fn parse_cell_list(cursor: &mut TokenCursor) -> Result<Vec<(String, ast::ExprSpan)>> {
    cursor.expect_open_bracket()?;
    let mut cells = Vec::new();
    loop {
        let (name, span) = cursor.consume_ident()?;
        cells.push((name, point(span)));
        if !cursor.consume_punct(",") {
            break;
        }
    }
    cursor.expect_close_bracket()?;
    Ok(cells)
}

/// A single-token `ExprSpan` where start and end coincide.
fn point(span: proc_macro2::Span) -> ast::ExprSpan {
    ast::ExprSpan {
        start: span,
        end: span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cel_parser::Expr;

    #[test]
    fn parse_empty_sheet_has_no_items() {
        let sheet = PmAstParser::new().parse_str("sheet empty {}").unwrap();
        assert_eq!(sheet.name, "empty");
        assert!(sheet.items.is_empty());
        assert!(sheet.errors.is_empty());
    }

    #[test]
    fn parse_cell_with_annotation_and_initializer() {
        let sheet = PmAstParser::new()
            .parse_str("sheet s { cell width: f64 = 1920.0; }")
            .unwrap();
        assert_eq!(sheet.items.len(), 1);
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert_eq!(cell.name, "width");
        assert_eq!(
            cell.type_name.as_ref().map(|(n, _)| n.as_str()),
            Some("f64")
        );
        assert!(cell.initializer.is_some());
    }

    #[test]
    fn parse_cell_annotation_only_has_no_initializer() {
        let sheet = PmAstParser::new()
            .parse_str("sheet s { cell area: f64; }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert!(cell.type_name.is_some());
        assert!(cell.initializer.is_none());
    }

    #[test]
    fn parse_cell_initializer_only_has_no_type_name() {
        let sheet = PmAstParser::new()
            .parse_str("sheet s { cell mode = 0i32; }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert!(cell.type_name.is_none());
        assert!(cell.initializer.is_some());
    }

    #[test]
    fn parse_relationship_records_methods_in_order() {
        let sheet = PmAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    relationship {
                        method [width, height] -> [area]   { width * height }
                        method [area, height]  -> [width]  { area / height }
                    }
                }
            "#,
            )
            .unwrap();
        let ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert_eq!(rel.methods.len(), 2);
        assert_eq!(rel.methods[0].inputs[0].0, "width");
        assert_eq!(rel.methods[0].outputs[0].0, "area");
        assert!(matches!(rel.methods[0].body, Expr::Op { ref name, .. } if name == "*"));
    }

    #[test]
    fn parse_relationship_optional_name() {
        let sheet = PmAstParser::new()
            .parse_str("sheet s { relationship r { method [x] -> [y] { x } } }")
            .unwrap();
        let ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert_eq!(rel.name.as_ref().map(|(n, _)| n.as_str()), Some("r"));
    }

    #[test]
    fn parse_conditional_records_branches_and_default() {
        let sheet = PmAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    conditional mode {
                        0i32 => { method [width] -> [height] { width } },
                        _ => { method [width] -> [height] { width } },
                    }
                }
            "#,
            )
            .unwrap();
        let ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(cond.match_name, "mode");
        assert_eq!(cond.branches.len(), 1);
        assert!(cond.default.is_some());
    }

    #[test]
    fn parse_method_body_is_a_cel_expr_tree() {
        let sheet = PmAstParser::new()
            .parse_str("sheet s { relationship { method [a, b] -> [c] { (a + b, a - b) } } }")
            .unwrap();
        let ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert!(matches!(rel.methods[0].body, Expr::Tuple { .. }));
    }

    #[test]
    fn parse_unknown_sheet_item_is_recorded_as_an_error_item() {
        let sheet = PmAstParser::new()
            .parse_str("sheet s { bogus x; }")
            .unwrap();
        assert_eq!(sheet.errors.len(), 1);
        assert!(matches!(sheet.items[0], ast::SheetItem::Error { .. }));
    }

    #[test]
    fn parse_malformed_cell_is_recorded_as_an_error_item() {
        let sheet = PmAstParser::new()
            .parse_str("sheet s { cell x unknown_syntax }")
            .unwrap();
        assert_eq!(sheet.errors.len(), 1);
        assert!(matches!(sheet.items[0], ast::SheetItem::Error { .. }));
    }

    #[test]
    fn recovery_records_an_error_item_and_continues_parsing() {
        let sheet = PmAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    cell good_before: i32 = 1;
                    cell bad unknown_syntax
                    cell good_after: i32 = 2;
                }
            "#,
            )
            .unwrap();
        assert_eq!(sheet.items.len(), 3);
        assert!(matches!(sheet.items[0], ast::SheetItem::Cell(_)));
        assert!(matches!(sheet.items[1], ast::SheetItem::Error { .. }));
        assert!(matches!(sheet.items[2], ast::SheetItem::Cell(_)));
        assert_eq!(sheet.errors.len(), 1);
    }

    #[test]
    fn recovery_collects_multiple_errors_from_multiple_malformed_items() {
        let sheet = PmAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    cell bad1 unknown_syntax;
                    cell bad2 unknown_syntax;
                    cell good: i32 = 1;
                }
            "#,
            )
            .unwrap();
        assert_eq!(sheet.errors.len(), 2);
        assert!(matches!(sheet.items.last(), Some(ast::SheetItem::Cell(_))));
    }

    #[test]
    fn well_formed_input_has_empty_errors() {
        let sheet = PmAstParser::new()
            .parse_str("sheet s { cell x: i32 = 1; }")
            .unwrap();
        assert!(sheet.errors.is_empty());
    }

    /// Regression test for a bug where `skip_to_recovery_point` tracked nesting depth with a
    /// fresh local counter starting at 0 on every call, instead of the cursor's actual running
    /// depth. A malformed `relationship { .. }` item opens its own `{` before the inner error
    /// (on `bad`, which isn't the `method` keyword) is detected, so recovery begins already one
    /// delimiter deep; the old code treated the relationship's own closing `}` as if it were
    /// back at sheet-item level, leaving it and everything after unconsumed and causing the
    /// whole parse to abort with `Err` instead of recovering just this one item.
    #[test]
    fn recovery_malformed_relationship_item_recovers() {
        let sheet = PmAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    cell good_before: i32 = 1;
                    relationship { bad }
                    cell good_after: i32 = 2;
                }
            "#,
            )
            .unwrap();
        assert_eq!(sheet.errors.len(), 1);
        assert_eq!(sheet.items.len(), 3);
        assert!(matches!(sheet.items[0], ast::SheetItem::Cell(_)));
        assert!(matches!(sheet.items[1], ast::SheetItem::Error { .. }));
        assert!(matches!(sheet.items[2], ast::SheetItem::Cell(_)));
    }

    /// Same regression as `recovery_malformed_relationship_item_recovers`, but for a malformed
    /// `conditional` item, which — like `relationship` — unconditionally opens its own `{`
    /// before any inner error can occur.
    #[test]
    fn recovery_malformed_conditional_item_recovers() {
        let sheet = PmAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    cell good_before: i32 = 1;
                    conditional m { bad }
                    cell good_after: i32 = 2;
                }
            "#,
            )
            .unwrap();
        assert_eq!(sheet.errors.len(), 1);
        assert_eq!(sheet.items.len(), 3);
        assert!(matches!(sheet.items[0], ast::SheetItem::Cell(_)));
        assert!(matches!(sheet.items[1], ast::SheetItem::Error { .. }));
        assert!(matches!(sheet.items[2], ast::SheetItem::Cell(_)));
    }

    /// Deeper regression case: the syntax error occurs inside a method's own body brace (an
    /// incomplete CEL expression), two delimiters below the sheet-item level (the relationship's
    /// `{` plus the method body's own `{`). Recovery must still land at sheet-item level rather
    /// than aborting the whole parse.
    #[test]
    fn recovery_malformed_method_body_recovers_at_sheet_item_level() {
        let sheet = PmAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    cell good_before: i32 = 1;
                    relationship {
                        method [a] -> [b] { a + }
                    }
                    cell good_after: i32 = 2;
                }
            "#,
            )
            .unwrap();
        assert_eq!(sheet.errors.len(), 1);
        assert_eq!(sheet.items.len(), 3);
        assert!(matches!(sheet.items[0], ast::SheetItem::Cell(_)));
        assert!(matches!(sheet.items[1], ast::SheetItem::Error { .. }));
        assert!(matches!(sheet.items[2], ast::SheetItem::Cell(_)));
    }

    /// Regression test for a bug where `skip_to_recovery_point`'s fallback token-skipping loop
    /// adjusted `depth` for *any* `OpenDelim`/`CloseDelim`, regardless of delimiter kind. A
    /// malformed CEL expression like `(+)` causes the embedded CEL sub-parser to consume the
    /// opening `(` (via `is_tuple_or_group`) but fail before consuming the matching `)`, since it
    /// never went through `TokenCursor` (see `TokenCursor::depth`'s own docs). That leftover,
    /// PM-untracked `)` then reached the old kind-agnostic match during recovery, which decremented
    /// `depth` for it exactly as if it were a real, PM-tracked `}`/`]` closing — desyncing the
    /// counter one level below where it should be, and causing recovery to mistake an inner item's
    /// closing brace for the sheet's own, aborting the whole parse with `Err`.
    #[test]
    fn recovery_malformed_cel_expr_with_dangling_paren_recovers() {
        let sheet = PmAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    cell good_before: i32 = 1;
                    relationship { method [a] -> [b] { (+) } }
                    cell good_after: i32 = 2;
                }
            "#,
            )
            .unwrap();
        assert_eq!(sheet.errors.len(), 1);
        assert_eq!(sheet.items.len(), 3);
        assert!(matches!(sheet.items[0], ast::SheetItem::Cell(_)));
        assert!(matches!(sheet.items[1], ast::SheetItem::Error { .. }));
        assert!(matches!(sheet.items[2], ast::SheetItem::Cell(_)));
    }

    /// Documents a KNOWN, accepted limitation of coarse error recovery — this test is not
    /// "passing by accident"; it pins down today's actual (still-buggy) behavior so a future fix
    /// has a concrete regression test to flip from "documents the bug" to "documents the fix."
    ///
    /// Unlike `recovery_malformed_cel_expr_with_dangling_paren_recovers` (a dangling `)` left by
    /// a failed CEL sub-expression), a dangling `}` left by a failed CEL `if`-expression cannot be
    /// fixed by the same kind-based approach: `is_if_expression` consumes the then-branch's
    /// opening `{` directly (bypassing `TokenCursor`, exactly like the paren case) but fails
    /// before consuming the matching `}` when the then-branch itself fails to parse (here, an
    /// empty `{ }`). Because CEL's `if`/`else` grammar reuses `Delimiter::Brace` — the same kind
    /// pm-lang's own `relationship`/`conditional`/method-body braces use — `skip_to_recovery_point`
    /// cannot tell "a stray brace CEL left dangling" apart from "a real pm-lang-tracked brace" by
    /// delimiter kind alone (contrast `Delimiter::Parenthesis`/`Delimiter::None`, which are never
    /// used by pm-lang's own grammar and so could safely be made depth-neutral). The stray `}`
    /// here is mistaken for the `relationship`'s own closing brace, so recovery stops one brace
    /// early and the whole parse aborts with `Err` instead of isolating just this one item.
    ///
    /// Fixing this in general requires `cel_parser`'s `Parser<C>` to report back exactly what
    /// delimiters it left unbalanced on a failed parse — a larger, cross-crate API change out of
    /// scope for this recovery feature. See the tracking issue for the general fix.
    #[test]
    fn recovery_known_limitation_if_expr_dangling_brace_aborts_whole_parse() {
        let result = PmAstParser::new().parse_str(
            r#"
                sheet s {
                    cell good_before: i32 = 1;
                    relationship { method [a] -> [b] { if a { } } }
                    cell good_after: i32 = 2;
                }
            "#,
        );
        assert!(
            result.is_err(),
            "expected the whole parse to abort with Err (known limitation); got {result:?}"
        );
    }
}
