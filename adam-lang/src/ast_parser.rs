//! [`AdamAstParser`]: parses adam-lang source into [`crate::ast::Sheet`] instead of executing into a
//! live `adam_rs::Sheet`. Shares [`crate::token_cursor::TokenCursor`] with
//! [`crate::AdamParser`] for pure tokenizing; the two parsers' grammar-production functions are
//! separate because what each one builds is genuinely different (see this plan's Architecture
//! section).

use cel_parser::{AstContext, OpLookup, Parser as CelParser};

use crate::ast;
use crate::token_cursor::TokenCursor;

/// Parser result type, matching `cel_parser::ParseError`.
type Result<T> = std::result::Result<T, cel_parser::ParseError>;

/// Parses adam-lang source strings into [`ast::Sheet`] trees, instead of executing into a live
/// `adam_rs::Sheet` (see [`crate::AdamParser`] for that path).
///
/// # Example
///
/// ```rust
/// use adam_lang::AdamAstParser;
///
/// let sheet = AdamAstParser::new().parse_str("sheet s { cell x: i32 = 0; }").unwrap();
/// assert_eq!(sheet.name, "s");
/// ```
pub struct AdamAstParser {
    cel: CelParser<AstContext>,
}

impl Default for AdamAstParser {
    fn default() -> Self {
        Self::new()
    }
}

impl AdamAstParser {
    /// Creates a new AST-building parser.
    ///
    /// Unlike [`crate::AdamParser::new`], this takes no `TypeRegistry`/`OpLookup` — `AstContext`
    /// resolves no identifiers and validates nothing during parsing (see this plan's Architecture
    /// section), so there is nothing for either to configure.
    #[must_use]
    pub fn new() -> Self {
        AdamAstParser {
            cel: CelParser::new(OpLookup::new()),
        }
    }

    /// Parses an adam-lang source string into an [`ast::Sheet`].
    ///
    /// A syntax error inside one `cell`/`relationship`/`conditional` item is recorded in
    /// `Sheet.errors` and replaced by a `SheetItem::Error` placeholder covering the skipped
    /// tokens; parsing resumes at the next sheet item instead of aborting (see
    /// `TokenCursor::skip_to_recovery_point`). This recovery is declaration-level only: a
    /// malformed `binding` inside a `relationship`/`conditional` block causes the whole
    /// enclosing item to become one `SheetItem::Error`.
    ///
    /// Recovery is reliable for syntax errors adam-lang's own grammar detects directly (malformed
    /// `cell` declarations; `relationship`/`conditional`/`binding` structure outside their CEL
    /// expression bodies, including a malformed `type_expr`'s own dangling `(`/`)`) and for CEL
    /// expression errors that don't leave an unbalanced delimiter of a kind CEL also uses for its
    /// own internal grouping. It is **not** guaranteed when a CEL expression's failure leaves a
    /// dangling, unmatched delimiter of a kind CEL reuses for its own internal structure — e.g. an
    /// `if`/`else` expression's braces, which are the same `Delimiter::Brace` kind adam-lang uses
    /// for its own `relationship`/`conditional` blocks (`if a { }` is one such case), or a
    /// tuple/group literal's parens, the same `Delimiter::Parenthesis` kind `type_expr` uses
    /// (`(+)` is one such case). In that narrower case recovery may abort the entire parse
    /// (returning `Err`) rather than isolating the one malformed item; see
    /// `TokenCursor::skip_to_recovery_point`'s doc comment for why a kind-based fix can't close
    /// this in general, and the tracking issue for the general fix.
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
            .map_err(|e| cel_parser::ParseError::from_lex_error(source, e))?;
        let mut cursor =
            TokenCursor::new(cel_parser::lex_lexer::LexLexer::new(stream.into_iter()).peekable());
        let doc_comment = cursor.consume_doc_comment_run(true);
        let mut sheet = self.parse_sheet(&mut cursor)?;
        if let Some((text, span)) = doc_comment {
            sheet.doc_comment = Some(text);
            sheet.span.start = span;
        }
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
        let open_span = cursor.expect_open_brace()?;
        let mut items = Vec::new();
        let mut errors = Vec::new();
        while !cursor.at_close_brace() {
            let doc = cursor.consume_doc_comment_run(false);
            let item_start = doc
                .as_ref()
                .map(|(_, span)| *span)
                .unwrap_or_else(|| cursor.peek_span());
            cursor.set_last_span(item_start);
            let target_depth = cursor.depth();
            match self.parse_sheet_item(cursor) {
                Ok(mut item) => {
                    if let Some((text, doc_span)) = &doc {
                        item.set_doc_comment(text.clone(), *doc_span);
                    }
                    items.push(item);
                }
                Err(e) => {
                    errors.push(e);
                    let recovery_fallback = cursor.last_span();
                    let item_end = cursor.skip_to_recovery_point(target_depth, recovery_fallback);
                    items.push(ast::SheetItem::Error {
                        span: ast::ExprSpan {
                            start: item_start,
                            end: item_end,
                        },
                        leading_comment: None,
                        doc_comment: doc.map(|(text, _)| text),
                        blank_line_before: false,
                    });
                }
            }
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::Sheet {
            name,
            name_span: point(name_span),
            items,
            leading_comment: None,
            doc_comment: None,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: point(open_span),
            span: ast::ExprSpan {
                start: sheet_start,
                end: close_span,
            },
            errors,
        })
    }

    /// `sheet_item = cell_decl | relationship_decl | conditional_decl | out_decl | source_decl.`
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
            Some(Token::Identifier(id)) if id == "out" => {
                self.parse_out_decl(cursor).map(ast::SheetItem::Out)
            }
            Some(Token::Identifier(id)) if id == "source" => {
                self.parse_source_decl(cursor).map(ast::SheetItem::Source)
            }
            Some(tok) => Err(cel_parser::ParseError::new(
                "expected `cell`, `relationship`, `conditional`, `out`, or `source`",
                tok.span(),
            )),
            None => Err(cel_parser::ParseError::new(
                "unexpected end of input",
                proc_macro2::Span::call_site(),
            )),
        }
    }

    /// `cell_decl = "cell" identifier cell_type_init [ cell_filter ] [ "require" "{" {
    /// requirement } "}" ] ";".`
    fn parse_cell_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::CellDecl> {
        let decl_start = cursor.peek_span();
        cursor.is_keyword("cell");
        let (name, name_span) = cursor.consume_ident()?;
        let (type_name, initializer) = if cursor.consume_punct(":") {
            let type_name = self.parse_type_expr(cursor)?;
            let initializer = if cursor.consume_punct("=") {
                Some(self.parse_cel_expression(cursor)?)
            } else {
                None
            };
            (Some(type_name), initializer)
        } else if cursor.consume_punct("=") {
            (None, Some(self.parse_cel_expression(cursor)?))
        } else {
            return Err(cursor.err_at("expected `:` or `=` in cell declaration"));
        };
        let filter = if cursor.is_keyword("filter") {
            let filter_start = cursor.last_span();
            Some(self.parse_cell_filter(cursor, filter_start)?)
        } else {
            None
        };
        let require = if cursor.is_keyword("require") {
            Some(self.parse_require_block(cursor)?)
        } else {
            None
        };
        let semi_span = cursor.expect_punct(";")?;
        Ok(ast::CellDecl {
            name,
            name_span: point(name_span),
            type_name,
            initializer,
            filter,
            require,
            leading_comment: None,
            doc_comment: None,
            blank_line_before: false,
            span: ast::ExprSpan {
                start: decl_start,
                end: semi_span,
            },
        })
    }

    /// `source_decl = "source" identifier cell_type_init [ cell_filter ] [ "require" "{" {
    /// requirement } "}" ] ";".`
    ///
    /// Mirrors [`Self::parse_cell_decl`] exactly.
    fn parse_source_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::SourceDecl> {
        let decl_start = cursor.peek_span();
        cursor.is_keyword("source");
        let (name, name_span) = cursor.consume_ident()?;
        let (type_name, initializer) = if cursor.consume_punct(":") {
            let type_name = self.parse_type_expr(cursor)?;
            let initializer = if cursor.consume_punct("=") {
                Some(self.parse_cel_expression(cursor)?)
            } else {
                None
            };
            (Some(type_name), initializer)
        } else if cursor.consume_punct("=") {
            (None, Some(self.parse_cel_expression(cursor)?))
        } else {
            return Err(cursor.err_at("expected `:` or `=` in source declaration"));
        };
        let filter = if cursor.is_keyword("filter") {
            let filter_start = cursor.last_span();
            Some(self.parse_cell_filter(cursor, filter_start)?)
        } else {
            None
        };
        let require = if cursor.is_keyword("require") {
            Some(self.parse_require_block(cursor)?)
        } else {
            None
        };
        let semi_span = cursor.expect_punct(";")?;
        Ok(ast::SourceDecl {
            name,
            name_span: point(name_span),
            type_name,
            initializer,
            filter,
            require,
            leading_comment: None,
            doc_comment: None,
            blank_line_before: false,
            span: ast::ExprSpan {
                start: decl_start,
                end: semi_span,
            },
        })
    }

    /// `cell_filter = "filter" identifier ":" expression.`
    ///
    /// - Precondition: the `filter` keyword has already been consumed by the caller; `filter_start`
    ///   is its span.
    fn parse_cell_filter(
        &mut self,
        cursor: &mut TokenCursor,
        filter_start: proc_macro2::Span,
    ) -> Result<ast::CellFilter> {
        let (name, name_span) = cursor.consume_ident()?;
        cursor.expect_punct(":")?;
        let body = self.parse_cel_expression(cursor)?;
        let body_end = body.span().end;
        Ok(ast::CellFilter {
            name,
            name_span: point(name_span),
            body,
            span: ast::ExprSpan {
                start: filter_start,
                end: body_end,
            },
        })
    }

    /// `type_expr = identifier | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".`
    fn parse_type_expr(&mut self, cursor: &mut TokenCursor) -> Result<ast::TypeExpr> {
        use cel_parser::lex_lexer::Token;
        if matches!(cursor.peek_token(), Some(Token::Identifier(_))) {
            let (name, span) = cursor.consume_ident()?;
            return Ok(ast::TypeExpr::Named(name, point(span)));
        }

        let open_span = cursor.expect_open_paren()?;
        if cursor.at_close_paren() {
            let close_span = cursor.expect_close_paren()?;
            return Ok(ast::TypeExpr::Tuple(
                Vec::new(),
                ast::ExprSpan {
                    start: open_span,
                    end: close_span,
                },
            ));
        }

        let first = self.parse_type_expr(cursor)?;
        if cursor.at_close_paren() {
            // Grouping: exactly one type, no comma.
            cursor.expect_close_paren()?;
            return Ok(first);
        }
        if !cursor.consume_punct(",") {
            return Err(cursor.err_at("expected ',' or closing parenthesis"));
        }
        if cursor.at_close_paren() {
            // Single element + trailing comma: 1-tuple.
            let close_span = cursor.expect_close_paren()?;
            return Ok(ast::TypeExpr::Tuple(
                vec![first],
                ast::ExprSpan {
                    start: open_span,
                    end: close_span,
                },
            ));
        }
        let mut elements = vec![first];
        loop {
            elements.push(self.parse_type_expr(cursor)?);
            if cursor.at_close_paren() {
                break;
            }
            if !cursor.consume_punct(",") {
                return Err(cursor.err_at("expected ',' or closing parenthesis"));
            }
        }
        let close_span = cursor.expect_close_paren()?;
        Ok(ast::TypeExpr::Tuple(
            elements,
            ast::ExprSpan {
                start: open_span,
                end: close_span,
            },
        ))
    }

    /// `relationship_decl = "relationship" "{" { binding } "}".`
    fn parse_relationship_decl(
        &mut self,
        cursor: &mut TokenCursor,
    ) -> Result<ast::RelationshipDecl> {
        let decl_start = cursor.peek_span();
        cursor.is_keyword("relationship");
        let open_span = cursor.expect_open_brace()?;
        let mut bindings = Vec::new();
        while !cursor.at_close_brace() {
            bindings.push(self.parse_binding(cursor)?);
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::RelationshipDecl {
            bindings,
            leading_comment: None,
            doc_comment: None,
            blank_line_before: false,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: point(open_span),
            span: ast::ExprSpan {
                start: decl_start,
                end: close_span,
            },
        })
    }

    /// `binding = binding_target ":=" expression ";".`
    fn parse_binding(&mut self, cursor: &mut TokenCursor) -> Result<ast::BindingDecl> {
        let decl_start = cursor.peek_span();
        let (outputs, destructure) = parse_binding_target(cursor)?;
        cursor.expect_punct(":=")?;
        let body = self.parse_cel_expression(cursor)?;
        let semi_span = cursor.expect_punct(";")?;
        Ok(ast::BindingDecl {
            outputs,
            destructure,
            body,
            leading_comment: None,
            blank_line_before: false,
            span: ast::ExprSpan {
                start: decl_start,
                end: semi_span,
            },
        })
    }

    /// `conditional_decl = "conditional" expression "{" { conditional_branch } "}".`
    fn parse_conditional_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::ConditionalDecl> {
        use cel_parser::lex_lexer::Token;
        let decl_start = cursor.peek_span();
        cursor.is_keyword("conditional");
        let match_expr = self.parse_cel_expression(cursor)?;
        let outer_open = cursor.expect_open_brace()?;
        let mut branches = Vec::new();
        let mut default = None;
        while !cursor.at_close_brace() {
            if matches!(cursor.peek_token(), Some(Token::Identifier(id)) if id == "_") {
                let underscore_span = cursor.peek_span();
                cursor.advance();
                cursor.expect_punct("=>")?;
                let branch_open = cursor.expect_open_brace()?;
                let relationships = self.parse_branch_relationships(cursor)?;
                let close = cursor.expect_close_brace()?;
                cursor.consume_punct(",");
                default = Some(ast::DefaultBranch {
                    relationships,
                    trailing_comment: None,
                    blank_line_before_close: false,
                    open_brace_span: point(branch_open),
                    span: ast::ExprSpan {
                        start: underscore_span,
                        end: close,
                    },
                });
                break; // default branch is always last
            }
            let (negated, lit, pattern_start, lit_span) = cursor.consume_literal_pattern()?;
            cursor.expect_punct("=>")?;
            let branch_open = cursor.expect_open_brace()?;
            let relationships = self.parse_branch_relationships(cursor)?;
            let close = cursor.expect_close_brace()?;
            cursor.consume_punct(",");
            branches.push(ast::ConditionalBranch {
                literal: lit,
                negated,
                literal_span: point(lit_span),
                relationships,
                leading_comment: None,
                blank_line_before: false,
                trailing_comment: None,
                blank_line_before_close: false,
                open_brace_span: point(branch_open),
                span: ast::ExprSpan {
                    start: pattern_start,
                    end: close,
                },
            });
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::ConditionalDecl {
            match_expr,
            branches,
            default,
            leading_comment: None,
            doc_comment: None,
            blank_line_before: false,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: point(outer_open),
            span: ast::ExprSpan {
                start: decl_start,
                end: close_span,
            },
        })
    }

    /// Parses one `conditional_branch`/`default_branch`'s shared body: `"{" { relationship_decl }
    /// "}"`, up to (not including) the closing `}`.
    fn parse_branch_relationships(
        &mut self,
        cursor: &mut TokenCursor,
    ) -> Result<Vec<ast::RelationshipDecl>> {
        use cel_parser::lex_lexer::Token;
        let mut relationships = Vec::new();
        while !cursor.at_close_brace() {
            if !matches!(cursor.peek_token(), Some(Token::Identifier(id)) if id == "relationship") {
                return Err(cursor.err_at("expected `relationship`"));
            }
            relationships.push(self.parse_relationship_decl(cursor)?);
        }
        Ok(relationships)
    }

    /// `out_decl = "out" identifier [ ":" type_name ] ":=" expression [ cell_filter ] [ "require"
    /// "{" { requirement } "}" ] ";".`
    fn parse_out_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::OutDecl> {
        let decl_start = cursor.peek_span();
        cursor.is_keyword("out");
        let (name, name_span) = cursor.consume_ident()?;
        let type_name = if cursor.consume_punct(":") {
            Some(self.parse_type_expr(cursor)?)
        } else {
            None
        };
        cursor.expect_punct(":=")?;
        let initializer = self.parse_cel_expression(cursor)?;
        let filter = if cursor.is_keyword("filter") {
            let filter_start = cursor.last_span();
            Some(self.parse_cell_filter(cursor, filter_start)?)
        } else {
            None
        };
        let require = if cursor.is_keyword("require") {
            Some(self.parse_require_block(cursor)?)
        } else {
            None
        };
        let semi_span = cursor.expect_punct(";")?;
        Ok(ast::OutDecl {
            name,
            name_span: point(name_span),
            type_name,
            initializer,
            filter,
            require,
            leading_comment: None,
            doc_comment: None,
            blank_line_before: false,
            span: ast::ExprSpan {
                start: decl_start,
                end: semi_span,
            },
        })
    }

    /// `require_block = "require" "{" { requirement } "}".`
    ///
    /// - Precondition: the `require` keyword has already been consumed by the caller.
    fn parse_require_block(&mut self, cursor: &mut TokenCursor) -> Result<ast::RequireBlock> {
        let open_span = cursor.expect_open_brace()?;
        let mut requirements = Vec::new();
        while !cursor.at_close_brace() {
            requirements.push(self.parse_requirement(cursor)?);
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::RequireBlock {
            requirements,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: point(open_span),
            span: ast::ExprSpan {
                start: open_span,
                end: close_span,
            },
        })
    }

    /// `requirement = identifier ":" expression ";".`
    fn parse_requirement(&mut self, cursor: &mut TokenCursor) -> Result<ast::RequirementDecl> {
        let decl_start = cursor.peek_span();
        let (name, name_span) = cursor.consume_ident()?;
        cursor.expect_punct(":")?;
        let body = self.parse_cel_expression(cursor)?;
        let semi_span = cursor.expect_punct(";")?;
        Ok(ast::RequirementDecl {
            name,
            name_span: point(name_span),
            body,
            leading_comment: None,
            blank_line_before: false,
            span: ast::ExprSpan {
                start: decl_start,
                end: semi_span,
            },
        })
    }

    /// Delegates one `expression` to `cel_parser::Parser<AstContext>`, sharing the token
    /// stream (the same take/set-tokens handoff `crate::AdamParser` uses for the `DynSegment`
    /// path).
    fn parse_cel_expression(&mut self, cursor: &mut TokenCursor) -> Result<cel_parser::Expr> {
        let tokens = cursor.take_tokens().expect("tokens present");
        self.cel.set_lex_tokens(tokens);
        let result = self.cel.parse_expression_ast();
        cursor.set_tokens(self.cel.take_lex_tokens().expect("tokens set"));
        result
    }
}

/// A single-token `ExprSpan` where start and end coincide.
fn point(span: proc_macro2::Span) -> ast::ExprSpan {
    ast::ExprSpan {
        start: span,
        end: span,
    }
}

/// `binding_target = identifier | "(" identifier { "," identifier } [ "," ] ")".`
///
/// Returns the output names in declaration order alongside whether the left-hand side requests
/// destructuring: `false` for a bare identifier or a single parenthesized identifier with no
/// comma (mere grouping, matching Rust's `(a)` pattern); `true` for `(a,)` (a 1-tuple pattern,
/// trailing comma mandatory) or `(a, b, ...)`.
fn parse_binding_target(cursor: &mut TokenCursor) -> Result<(Vec<(String, ast::ExprSpan)>, bool)> {
    if !cursor.at_open_paren() {
        let (name, span) = cursor.consume_ident()?;
        return Ok((vec![(name, point(span))], false));
    }

    cursor.expect_open_paren()?;
    let (first_name, first_span) = cursor.consume_ident()?;
    if cursor.at_close_paren() {
        // Grouping: exactly one identifier, no comma -- same as the bare form.
        cursor.expect_close_paren()?;
        return Ok((vec![(first_name, point(first_span))], false));
    }
    if !cursor.consume_punct(",") {
        return Err(cursor.err_at("expected ',' or closing parenthesis"));
    }
    if cursor.at_close_paren() {
        // Single identifier + trailing comma: destructures a 1-tuple.
        cursor.expect_close_paren()?;
        return Ok((vec![(first_name, point(first_span))], true));
    }
    let mut outputs = vec![(first_name, point(first_span))];
    loop {
        let (name, span) = cursor.consume_ident()?;
        outputs.push((name, point(span)));
        if cursor.at_close_paren() {
            break;
        }
        if !cursor.consume_punct(",") {
            return Err(cursor.err_at("expected ',' or closing parenthesis"));
        }
    }
    cursor.expect_close_paren()?;
    Ok((outputs, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cel_parser::Expr;

    #[test]
    fn parse_empty_sheet_has_no_items() {
        let sheet = AdamAstParser::new().parse_str("sheet empty {}").unwrap();
        assert_eq!(sheet.name, "empty");
        assert!(sheet.items.is_empty());
        assert!(sheet.errors.is_empty());
    }

    #[test]
    fn parse_cell_with_annotation_and_initializer() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell width: f64 = 1920.0; }")
            .unwrap();
        assert_eq!(sheet.items.len(), 1);
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert_eq!(cell.name, "width");
        assert!(matches!(
            cell.type_name.as_ref().unwrap(),
            ast::TypeExpr::Named(n, _) if n == "f64"
        ));
        assert!(cell.initializer.is_some());
    }

    #[test]
    fn parse_cell_annotation_only_has_no_initializer() {
        let sheet = AdamAstParser::new()
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
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell mode = 0i32; }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert!(cell.type_name.is_none());
        assert!(cell.initializer.is_some());
    }

    #[test]
    fn parse_source_decl_produces_a_source_decl_sheet_item() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { source width: i32 = 4; }")
            .unwrap();
        assert!(matches!(sheet.items[0], ast::SheetItem::Source(_)));
    }

    #[test]
    fn parse_source_with_a_filter() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { source a: i32 = 1 filter clamp: _; }")
            .unwrap();
        let ast::SheetItem::Source(source) = &sheet.items[0] else {
            panic!("expected Source");
        };
        let filter = source.filter.as_ref().expect("filter present");
        assert_eq!(filter.name, "clamp");
        assert!(matches!(&filter.body, Expr::Ident { name, .. } if name == "_"));
    }

    #[test]
    fn parse_source_without_a_filter_leaves_it_none() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { source a: i32 = 1; }")
            .unwrap();
        let ast::SheetItem::Source(source) = &sheet.items[0] else {
            panic!("expected Source");
        };
        assert!(source.filter.is_none());
    }

    #[test]
    fn parse_relationship_records_bindings_in_order() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    relationship {
                        area := width * height;
                        width := area / height;
                    }
                }
            "#,
            )
            .unwrap();
        let ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert_eq!(rel.bindings.len(), 2);
        assert_eq!(rel.bindings[0].outputs[0].0, "area");
        assert!(matches!(rel.bindings[0].body, Expr::Op { ref name, .. } if name == "*"));
    }

    #[test]
    fn parse_binding_bare_identifier_target_is_not_a_destructure() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { relationship { x := a; } }")
            .unwrap();
        let ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert!(!rel.bindings[0].destructure);
        assert_eq!(rel.bindings[0].outputs.len(), 1);
    }

    #[test]
    fn parse_binding_grouped_single_identifier_target_is_not_a_destructure() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { relationship { (x) := a; } }")
            .unwrap();
        let ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert!(!rel.bindings[0].destructure);
        assert_eq!(rel.bindings[0].outputs.len(), 1);
    }

    #[test]
    fn parse_binding_single_element_tuple_target_is_a_destructure() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { relationship { (x,) := (a,); } }")
            .unwrap();
        let ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert!(rel.bindings[0].destructure);
        assert_eq!(rel.bindings[0].outputs.len(), 1);
        assert_eq!(rel.bindings[0].outputs[0].0, "x");
    }

    #[test]
    fn parse_binding_multi_identifier_target_is_a_destructure() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { relationship { (x, y) := (a, b); } }")
            .unwrap();
        let ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert!(rel.bindings[0].destructure);
        assert_eq!(
            rel.bindings[0]
                .outputs
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            vec!["x", "y"]
        );
    }

    #[test]
    fn multi_output_binding_without_parens_recovers_at_sheet_item_level() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    cell good_before: i32 = 1;
                    relationship {
                        x, y := (a, b);
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

    #[test]
    fn parse_conditional_records_branches_and_default() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    conditional mode {
                        0i32 => { relationship { height := width; } },
                        _ => { relationship { height := width; } },
                    }
                }
            "#,
            )
            .unwrap();
        let ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert!(matches!(&cond.match_expr, Expr::Ident { name, .. } if name == "mode"));
        assert_eq!(cond.branches.len(), 1);
        assert!(!cond.branches[0].negated);
        assert!(cond.default.is_some());
    }

    #[test]
    fn parse_conditional_records_a_negated_literal_branch_key() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    conditional mode {
                        -1i32 => { relationship { height := width; } },
                        _ => { relationship { height := width; } },
                    }
                }
            "#,
            )
            .unwrap();
        let ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(cond.branches.len(), 1);
        assert!(cond.branches[0].negated);
    }

    #[test]
    fn parse_conditional_branch_dash_not_followed_by_a_literal_is_error() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    conditional mode {
                        -width => { relationship { height := width; } },
                    }
                }
            "#,
            )
            .unwrap();
        assert!(matches!(sheet.items[0], ast::SheetItem::Error { .. }));
    }

    #[test]
    fn parse_conditional_branch_literal_with_invalid_suffix_is_error() {
        // `10xyz` isn't a `cel_parser`-recognized literal (unrecognized integer suffix) — must
        // be rejected here exactly as the runtime parser rejects it, not silently accepted into
        // the CST and later round-tripped by `fmt`.
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    conditional mode {
                        10xyz => { relationship { height := width; } },
                    }
                }
            "#,
            )
            .unwrap();
        assert!(matches!(sheet.items[0], ast::SheetItem::Error { .. }));
    }

    #[test]
    fn parse_conditional_branch_negating_an_unsigned_literal_is_error() {
        // `-1u32` has no `cel_parser` unary `-` overload — must be rejected here exactly as the
        // runtime parser rejects it, not silently accepted into the CST.
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    conditional mode {
                        -1u32 => { relationship { height := width; } },
                    }
                }
            "#,
            )
            .unwrap();
        assert!(matches!(sheet.items[0], ast::SheetItem::Error { .. }));
    }

    #[test]
    fn parse_conditional_records_an_expression_match_subject() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    conditional a && b {
                        _ => { relationship { height := width; } },
                    }
                }
            "#,
            )
            .unwrap();
        let ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert!(matches!(
            &cond.match_expr,
            Expr::Logical {
                op: cel_parser::LogicalOp::And,
                ..
            }
        ));
    }

    #[test]
    fn parse_conditional_branch_records_multiple_relationships() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    conditional mode {
                        0i32 => {
                            relationship { b := a; }
                            relationship { d := c; }
                        },
                    }
                }
            "#,
            )
            .unwrap();
        let ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(cond.branches[0].relationships.len(), 2);
    }

    #[test]
    fn parse_conditional_default_branch_records_multiple_relationships() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    conditional mode {
                        _ => {
                            relationship { b := a; }
                            relationship { d := c; }
                        },
                    }
                }
            "#,
            )
            .unwrap();
        let ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(
            cond.default.as_ref().map(|d| d.relationships.len()),
            Some(2)
        );
    }

    #[test]
    fn conditional_branch_bare_binding_without_relationship_wrapper_recovers() {
        // A branch body is now `{ relationship_decl }`, not `{ binding }` directly — a bare binding is
        // a syntax error, recovered at the enclosing conditional_decl's sheet-item level (see
        // `recovery_malformed_conditional_item_recovers`).
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    cell good_before: i32 = 1;
                    conditional mode { 0i32 => { b := a; } }
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

    #[test]
    fn parse_binding_body_is_a_cel_expr_tree() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { relationship { c := (a + b, a - b); } }")
            .unwrap();
        let ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert!(matches!(rel.bindings[0].body, Expr::Tuple { .. }));
    }

    #[test]
    fn parse_unknown_sheet_item_is_recorded_as_an_error_item() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { bogus x; }")
            .unwrap();
        assert_eq!(sheet.errors.len(), 1);
        assert!(matches!(sheet.items[0], ast::SheetItem::Error { .. }));
    }

    #[test]
    fn parse_malformed_cell_is_recorded_as_an_error_item() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell x unknown_syntax }")
            .unwrap();
        assert_eq!(sheet.errors.len(), 1);
        assert!(matches!(sheet.items[0], ast::SheetItem::Error { .. }));
    }

    #[test]
    fn recovery_records_an_error_item_and_continues_parsing() {
        let sheet = AdamAstParser::new()
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
    fn recovery_error_span_covers_tokens_actually_consumed_by_the_failed_item() {
        // When recovery stops immediately (the very next token is already the sibling
        // keyword, so skip_to_recovery_point consumes nothing itself), the Error item's span
        // must still cover whatever the failed production consumed before giving up (here,
        // "cell bad" -- the name it read before failing the ':'/'=' check) rather than
        // collapsing to just its first token ("cell").
        let source = "sheet s { cell bad relationship { y := x; } }";
        let sheet = AdamAstParser::new().parse_str(source).unwrap();
        let ast::SheetItem::Error { span, .. } = &sheet.items[0] else {
            panic!("expected Error");
        };
        let range = cel_parser::SourceSpan {
            start: span.start.start(),
            end: span.end.end(),
        }
        .to_byte_range(source);
        assert_eq!(&source[range], "cell bad");
    }

    #[test]
    fn recovery_collects_multiple_errors_from_multiple_malformed_items() {
        let sheet = AdamAstParser::new()
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
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell x: i32 = 1; }")
            .unwrap();
        assert!(sheet.errors.is_empty());
    }

    /// Regression test for a bug where `skip_to_recovery_point` tracked nesting depth with a
    /// fresh local counter starting at 0 on every call, instead of the cursor's actual running
    /// depth. A malformed `relationship { .. }` item opens its own `{` before the inner error (on
    /// `bad`, which isn't a valid binding) is detected, so recovery begins already one delimiter
    /// deep; the old code treated the relationship's own closing `}` as if it were back at
    /// sheet-item level, leaving it and everything after unconsumed and causing the whole parse
    /// to abort with `Err` instead of recovering just this one item.
    #[test]
    fn recovery_malformed_relationship_item_recovers() {
        let sheet = AdamAstParser::new()
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
        let sheet = AdamAstParser::new()
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

    /// Deeper regression case: the syntax error occurs inside a binding's own body expression (an
    /// incomplete CEL expression), one delimiter below the sheet-item level (the relationship's own
    /// `{`) — unlike the old `method_decl` grammar, a binding's body has no brace of its own, so
    /// there is one less delimiter to unwind through than before. Recovery must still land at
    /// sheet-item level rather than aborting the whole parse.
    #[test]
    fn recovery_malformed_binding_body_recovers_at_sheet_item_level() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    cell good_before: i32 = 1;
                    relationship {
                        b := a + ;
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

    /// Documents a KNOWN, accepted limitation of coarse error recovery, *reintroduced* by giving
    /// `Delimiter::Parenthesis` the same depth-tracking treatment `Delimiter::Brace`/
    /// `Delimiter::Bracket` already have in `skip_to_recovery_point`. That change is required so a
    /// malformed `type_expr`'s own dangling paren unwinds `TokenCursor::depth` correctly (see
    /// `malformed_tuple_type_recovers_at_the_next_sheet_item`) — `type_expr` is the first
    /// adam-lang-grammar production to use parens, so its own unmatched `(`/`)` must be tracked
    /// exactly like a malformed `relationship`/`conditional` block's brace. Previously this
    /// exact scenario recovered cleanly: `Delimiter::Parenthesis` was deliberately treated as
    /// depth-neutral during recovery, safe *only* because CEL owned every paren back then, so a
    /// dangling one could never be mistaken for an adam-lang-tracked one. Now that `type_expr` also
    /// uses parens at the adam-lang-grammar level, `skip_to_recovery_point` can no longer tell "a
    /// stray paren CEL left dangling" apart from "a real adam-lang-tracked paren" by delimiter kind
    /// alone — the same ambiguity `Delimiter::Brace` already has (see
    /// `recovery_known_limitation_if_expr_dangling_brace_aborts_whole_parse`). A malformed CEL
    /// expression like `(+)` causes the embedded CEL sub-parser to consume the opening `(` (via
    /// `is_tuple_or_group`) but fail before consuming the matching `)`, since it never went through
    /// `TokenCursor` (see `TokenCursor::depth`'s own docs); that leftover, untracked `)` is now
    /// mistaken by `skip_to_recovery_point` for the enclosing `relationship`'s own paren-tracked
    /// nesting closing, mis-stopping recovery one delimiter early and aborting the whole parse with
    /// `Err` rather than isolating just this one malformed item. Fixing this in general requires
    /// `cel_parser`'s `Parser<C>` to report back exactly what it left unbalanced on a failed parse —
    /// out of scope here; see the tracking issue for the general fix:
    /// <https://github.com/stlab/cel-rs/issues/43>.
    #[test]
    fn recovery_known_limitation_cel_dangling_paren_aborts_whole_parse() {
        let result = AdamAstParser::new().parse_str(
            r#"
                sheet s {
                    cell good_before: i32 = 1;
                    relationship { b := (+); }
                    cell good_after: i32 = 2;
                }
            "#,
        );
        assert!(
            result.is_err(),
            "expected the whole parse to abort with Err (known limitation); got {result:?}"
        );
    }

    /// Documents a KNOWN, accepted limitation of coarse error recovery — this test is not
    /// "passing by accident"; it pins down today's actual (still-buggy) behavior so a future fix
    /// has a concrete regression test to flip from "documents the bug" to "documents the fix."
    ///
    /// Like `recovery_known_limitation_cel_dangling_paren_aborts_whole_parse` (a dangling `)` left
    /// by a failed CEL sub-expression), a dangling `}` left by a failed CEL `if`-expression cannot
    /// be fixed by the same kind-based approach: `is_if_expression` consumes the then-branch's
    /// opening `{` directly (bypassing `TokenCursor`, exactly like the paren case) but fails
    /// before consuming the matching `}` when the then-branch itself fails to parse (here, an
    /// empty `{ }`). Because CEL's `if`/`else` grammar reuses `Delimiter::Brace` — the same kind
    /// adam-lang's own `relationship`/`conditional` blocks use — `skip_to_recovery_point`
    /// cannot tell "a stray brace CEL left dangling" apart from "a real adam-lang-tracked brace" by
    /// delimiter kind alone (`Delimiter::Parenthesis` now shares this exact ambiguity too, since
    /// `type_expr` started using parens at the adam-lang-grammar level; only `Delimiter::None`,
    /// never used by adam-lang's own grammar, remains safely depth-neutral). The stray `}` here is
    /// mistaken for the `relationship`'s own closing brace, so recovery stops one brace early and
    /// the whole parse aborts with `Err` instead of isolating just this one item.
    ///
    /// Fixing this in general requires `cel_parser`'s `Parser<C>` to report back exactly what
    /// delimiters it left unbalanced on a failed parse — a larger, cross-crate API change out of
    /// scope for this recovery feature. See the tracking issue for the general fix:
    /// <https://github.com/stlab/cel-rs/issues/43>.
    #[test]
    fn recovery_known_limitation_if_expr_dangling_brace_aborts_whole_parse() {
        let result = AdamAstParser::new().parse_str(
            r#"
                sheet s {
                    cell good_before: i32 = 1;
                    relationship { b := if a { }; }
                    cell good_after: i32 = 2;
                }
            "#,
        );
        assert!(
            result.is_err(),
            "expected the whole parse to abort with Err (known limitation); got {result:?}"
        );
    }

    /// A variant of the same known limitation (see
    /// `recovery_known_limitation_if_expr_dangling_brace_aborts_whole_parse`): here the CEL
    /// sub-expression fails on a bare `+` (no valid expression follows it), which the failed
    /// `is_or_expression` call doesn't consume — so the very next token `skip_to_recovery_point`
    /// sees is a keyword-shaped identifier (`cell`) written just after it. Because that
    /// identifier is encountered while `depth` is still elevated (still inside the
    /// `relationship`'s own brace, not yet unwound), the `at_or_below_target` guard on
    /// the `cell`/`relationship`/`conditional` stopping check doesn't fire for it, so it's
    /// swallowed as ordinary garbage rather than treated as a boundary.
    ///
    /// This does not corrupt the result or panic, and it does not silently drop a *subsequent,
    /// well-formed* sheet item either: the swallow only ever consumes tokens up to the next real
    /// adam-lang-tracked brace, at which point recovery hits the exact same dangling-brace
    /// mis-stop as the sibling test above and the whole parse aborts with `Err` — never `Ok`
    /// with a gap. Pinned here because the failure path differs (garbage-swallow before the
    /// mis-stop, rather than mis-stop directly), even though the externally observable outcome
    /// is the same accepted limitation tracked in
    /// <https://github.com/stlab/cel-rs/issues/43>.
    #[test]
    fn recovery_known_limitation_keyword_shaped_garbage_still_aborts_cleanly() {
        let result = AdamAstParser::new().parse_str(
            "sheet s { relationship { b := if a { + cell good: i32 = 1; }; } cell trailing: i32 = 2; }",
        );
        assert!(
            result.is_err(),
            "expected the whole parse to abort with Err (known limitation), not a corrupted \
             Ok result; got {result:?}"
        );
    }

    #[test]
    fn parse_out_with_explicit_type_and_no_requirements() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
            sheet s {
                cell width: f64 = 4.0;
                cell height: f64 = 3.0;
                out area: f64 := width * height;
            }
        "#,
            )
            .unwrap();
        assert!(sheet.errors.is_empty());
        let ast::SheetItem::Out(out) = &sheet.items[2] else {
            panic!("expected Out");
        };
        assert_eq!(out.name, "area");
        assert!(matches!(
            out.type_name.as_ref().unwrap(),
            ast::TypeExpr::Named(n, _) if n == "f64"
        ));
        assert!(matches!(out.initializer, Expr::Op { ref name, .. } if name == "*"));
        assert!(out.require.is_none());
    }

    #[test]
    fn parse_out_with_no_type_annotation() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { out area := width; }")
            .unwrap();
        let ast::SheetItem::Out(out) = &sheet.items[0] else {
            panic!("expected Out");
        };
        assert!(out.type_name.is_none());
    }

    #[test]
    fn parse_out_with_requirements_in_declaration_order() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
            sheet s {
                out area: f64 := width * height require {
                    max_area: width * height <= max_area;
                    max_width: width <= max_width;
                };
            }
        "#,
            )
            .unwrap();
        let ast::SheetItem::Out(out) = &sheet.items[0] else {
            panic!("expected Out");
        };
        let require = out.require.as_ref().expect("require block present");
        assert_eq!(require.requirements.len(), 2);
        assert_eq!(require.requirements[0].name, "max_area");
        assert_eq!(require.requirements[1].name, "max_width");
    }

    #[test]
    fn parse_malformed_out_is_recorded_as_an_error_item() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
            sheet s {
                cell good_before: i32 = 1;
                out area { bad }
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

    #[test]
    fn parse_cell_with_explicit_tuple_type() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell a: (i32, f64); }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        match cell.type_name.as_ref().unwrap() {
            ast::TypeExpr::Tuple(elements, _) => {
                assert_eq!(elements.len(), 2);
                assert!(matches!(&elements[0], ast::TypeExpr::Named(n, _) if n == "i32"));
                assert!(matches!(&elements[1], ast::TypeExpr::Named(n, _) if n == "f64"));
            }
            other => panic!("expected Tuple, got {other:?}"),
        }
    }

    #[test]
    fn parse_cell_with_a_filter() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell a: i32 = 1 filter clamp: _; }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        let filter = cell.filter.as_ref().expect("filter present");
        assert!(matches!(&filter.body, Expr::Ident { name, .. } if name == "_"));
    }

    #[test]
    fn parse_cell_filter_records_its_name() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell x: i32 = 0 filter clamp: 0..=10; }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected a cell decl");
        };
        assert_eq!(cell.filter.as_ref().unwrap().name, "clamp");
    }

    #[test]
    fn parse_cell_with_a_filter_referencing_a_cell() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell hi: i32 = 100; cell a: i32 = 1 filter sum: _ + hi; }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        let filter = cell.filter.as_ref().expect("filter present");
        match &filter.body {
            Expr::Op { name, operands, .. } => {
                assert_eq!(name, "+");
                assert!(matches!(&operands[0], Expr::Ident { name, .. } if name == "_"));
                assert!(matches!(&operands[1], Expr::Ident { name, .. } if name == "hi"));
            }
            other => panic!("expected Op, got {other:?}"),
        }
    }

    #[test]
    fn parse_cell_without_a_filter_leaves_it_none() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell a: i32 = 1; }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert!(cell.filter.is_none());
    }

    #[test]
    fn recovery_malformed_filter_recovers_at_the_next_sheet_item() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    cell good_before: i32 = 1;
                    cell bad: i32 = 1 filter |x: i32|;
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

    #[test]
    fn parse_cell_with_nested_tuple_type() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell a: (i32, (f64, String)); }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        let ast::TypeExpr::Tuple(elements, _) = cell.type_name.as_ref().unwrap() else {
            panic!("expected top-level Tuple");
        };
        assert_eq!(elements.len(), 2);
        assert!(matches!(&elements[0], ast::TypeExpr::Named(n, _) if n == "i32"));
        match &elements[1] {
            ast::TypeExpr::Tuple(inner, _) => assert_eq!(inner.len(), 2),
            other => panic!("expected nested Tuple, got {other:?}"),
        }
    }

    #[test]
    fn parse_cell_with_empty_tuple_type() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell a: (); }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        match cell.type_name.as_ref().unwrap() {
            ast::TypeExpr::Tuple(elements, _) => assert!(elements.is_empty()),
            other => panic!("expected empty Tuple, got {other:?}"),
        }
    }

    #[test]
    fn parse_cell_with_parenthesized_type_is_grouping_not_a_1_tuple() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell a: (i32); }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert!(
            matches!(cell.type_name.as_ref().unwrap(), ast::TypeExpr::Named(n, _) if n == "i32")
        );
    }

    #[test]
    fn parse_cell_with_1_tuple_type_requires_trailing_comma() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell a: (i32,); }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        match cell.type_name.as_ref().unwrap() {
            ast::TypeExpr::Tuple(elements, _) => assert_eq!(elements.len(), 1),
            other => panic!("expected 1-Tuple, got {other:?}"),
        }
    }

    #[test]
    fn parse_cell_initializer_is_a_tuple_expr() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell a = (1, 2.5); }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert!(matches!(cell.initializer, Some(Expr::Tuple { .. })));
    }

    #[test]
    fn parse_out_with_explicit_tuple_type() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { out a: (i32, f64) := (x, x); }")
            .unwrap();
        let ast::SheetItem::Out(out) = &sheet.items[0] else {
            panic!("expected Out");
        };
        assert!(
            matches!(out.type_name.as_ref().unwrap(), ast::TypeExpr::Tuple(elements, _) if elements.len() == 2)
        );
    }

    #[test]
    fn malformed_tuple_type_recovers_at_the_next_sheet_item() {
        // Note: unlike a typical "forgot the closing delimiter" typo, the `(`/`)` pair here must
        // stay balanced overall — `proc_macro2::TokenStream::from_str` tokenizes delimiters as a
        // matched tree up front, so a source string with a truly unmatched paren anywhere fails
        // to tokenize at all (`LexError`), before adam-lang-level parsing (and its recovery) ever
        // runs. The malformed part is the missing `,` between the two type names, not the parens.
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell good_before: i32 = 1; cell bad: (i32 i32); cell good_after: i32 = 2; }")
            .unwrap();
        assert_eq!(sheet.errors.len(), 1);
        assert_eq!(sheet.items.len(), 3);
        assert!(matches!(sheet.items[0], ast::SheetItem::Cell(_)));
        assert!(matches!(sheet.items[1], ast::SheetItem::Error { .. }));
        assert!(matches!(sheet.items[2], ast::SheetItem::Cell(_)));
    }

    #[test]
    fn attaches_an_outer_doc_comment_to_a_cell() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s {\n    /// the total\n    cell x: i32 = 1;\n}")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert_eq!(cell.doc_comment.as_deref(), Some(" the total"));
    }

    #[test]
    fn attaches_an_outer_doc_comment_to_a_relationship() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s {\n    /// docs\n    relationship { b := a; }\n}")
            .unwrap();
        let ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert_eq!(rel.doc_comment.as_deref(), Some(" docs"));
    }

    #[test]
    fn attaches_an_outer_doc_comment_to_a_conditional() {
        let sheet = AdamAstParser::new()
            .parse_str(
                "sheet s {\n    cell p: i32 = 0;\n    /// docs\n    conditional p {\n        _ => { relationship { b := a; } }\n    }\n}",
            )
            .unwrap();
        let ast::SheetItem::Conditional(cond) = &sheet.items[1] else {
            panic!("expected Conditional");
        };
        assert_eq!(cond.doc_comment.as_deref(), Some(" docs"));
    }

    #[test]
    fn attaches_an_outer_doc_comment_to_an_out_decl() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s {\n    /// docs\n    out area: f64 := w;\n}")
            .unwrap();
        let ast::SheetItem::Out(out) = &sheet.items[0] else {
            panic!("expected Out");
        };
        assert_eq!(out.doc_comment.as_deref(), Some(" docs"));
    }

    #[test]
    fn attaches_a_sheet_level_inner_doc_comment() {
        let sheet = AdamAstParser::new()
            .parse_str("//! module docs\nsheet s {\n    cell x: i32 = 1;\n}")
            .unwrap();
        assert_eq!(sheet.doc_comment.as_deref(), Some(" module docs"));
    }

    #[test]
    fn joins_consecutive_outer_doc_comment_lines() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s {\n    /// line one\n    /// line two\n    cell x: i32 = 1;\n}")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert_eq!(cell.doc_comment.as_deref(), Some(" line one\n line two"));
    }

    #[test]
    fn a_doc_comment_binds_forward_across_a_blank_line() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s {\n    /// docs\n\n    cell x: i32 = 1;\n}")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert_eq!(cell.doc_comment.as_deref(), Some(" docs"));
    }

    #[test]
    fn a_plain_comment_and_a_doc_comment_coexist_in_source_order() {
        // NOTE: deviates from the task brief's literal source string, which places `cell x` as
        // the sheet's *first* item. `trivia::attach_gaps` never attaches a leading comment to a
        // list's first element (nothing precedes it but the enclosing `{` — an out-of-scope,
        // pre-existing gap tracked by issue #52's "trailing" counterpart in trivia.rs, unrelated
        // to doc-comment parsing; see this task's report for the full diagnosis), so the literal
        // brief source can never satisfy the `leading_comment` assertion below regardless of this
        // task's changes. A leading sibling item here exercises the exact same doc-comment/
        // plain-comment interaction the brief intends (span-widening must stop the plain-comment
        // gap scan before the doc comment's own source text) via `attach_gaps`'s normal,
        // already-working non-first-item path instead.
        let source =
            "sheet s {\n    cell w: i32 = 0;\n    // TODO\n    /// docs\n    cell x: i32 = 1;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        crate::attach_trivia(source, &mut sheet);
        let ast::SheetItem::Cell(cell) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(cell.doc_comment.as_deref(), Some(" docs"));
        assert_eq!(
            cell.leading_comment,
            Some(ast::Comment::Line("TODO".to_string()))
        );
    }

    #[test]
    fn a_doc_comment_before_a_binding_recovers_as_a_declaration_level_error() {
        let sheet = AdamAstParser::new()
            .parse_str(
                "sheet s {\n    relationship {\n        /// not allowed here\n        b := a;\n    }\n}",
            )
            .unwrap();
        assert!(!sheet.errors.is_empty());
        assert!(matches!(sheet.items[0], ast::SheetItem::Error { .. }));
    }

    #[test]
    fn range_syntax_is_reachable_from_a_relationship_binding() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { relationship { x := 1..5; } }")
            .unwrap();
        let ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected a Relationship item, got {:?}", sheet.items[0]);
        };
        assert!(
            matches!(&rel.bindings[0].body, cel_parser::Expr::Op { name, .. } if name == "range"),
            "expected the binding body to be a range Expr::Op, got {:?}",
            rel.bindings[0].body
        );
    }
}
