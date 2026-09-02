//! Pure token-stream cursor shared by adam-lang's `Sheet`-building parser (`parser.rs`) and its
//! AST-building parser (`ast_parser.rs`), so tokenizing/peeking/expecting logic — which has no
//! dependency on what each parser *builds* — is written exactly once.

use cel_parser::ParseError;
use cel_parser::lex_lexer::{HasSpan, LexLexer, Literal, Token};
use proc_macro2::{Delimiter, Span};

/// Parser result type, matching `cel_parser::ParseError`.
type Result<T> = std::result::Result<T, ParseError>;

/// A peekable adam-lang token stream plus the primitive lookahead/consume operations every
/// adam-lang grammar production needs, independent of what each production builds (a live
/// `adam_rs::Sheet` mutation, or a syntax tree node).
pub(crate) struct TokenCursor {
    tokens: Option<std::iter::Peekable<LexLexer>>,
    /// Running brace/bracket/paren nesting depth, incremented/decremented only by this cursor's
    /// own `expect_open_brace`/`expect_close_brace`/`expect_open_bracket`/`expect_close_bracket`/
    /// `expect_open_paren`/`expect_close_paren` (all three delimiter kinds are tracked uniformly
    /// as one counter). Tokens consumed directly by an embedded `cel_parser::Parser` while it
    /// temporarily owns the stream (see `take_tokens`/`set_tokens`) never pass through these
    /// methods, so they don't affect this counter — which is exactly what callers like
    /// [`skip_to_recovery_point`] need: a depth that reflects only adam-lang-grammar nesting, not
    /// CEL sub-expression internals.
    ///
    /// This separation holds only as long as a failed CEL sub-expression doesn't leave a dangling,
    /// unmatched delimiter of a kind CEL also reuses for its own internal grouping. `type_expr`
    /// (the one adam-lang-grammar-level production that uses parens) is the sole
    /// adam-lang-grammar exception: those parens genuinely go through `expect_open_paren`/
    /// `expect_close_paren`, exactly like brace/bracket, so a malformed `type_expr` unwinds
    /// `depth` correctly. It does not hold for `Delimiter::Brace` or, now, `Delimiter::Parenthesis`
    /// when the dangling delimiter comes from CEL's own internal grouping instead: CEL's `if`/
    /// `else` expressions use braces for their branches, and CEL's tuple/group literals use
    /// parens, the same delimiter kinds adam-lang uses for `relationship`/`conditional`/`out`'s
    /// `require` bodies and for `type_expr`, respectively. A CEL `if` expression whose then-branch fails to
    /// parse (e.g. `if a { }`), or a CEL tuple/group literal that fails partway through (e.g.
    /// `(+)`), can leave a stray `}`/`)` in the stream that this counter — and
    /// [`skip_to_recovery_point`], which reads it — has no way to distinguish from a real
    /// adam-lang-tracked brace/paren. See [`skip_to_recovery_point`]'s doc comment for the
    /// resulting scope boundary.
    depth: i32,
    /// The span of the last token [`Self::advance`] actually consumed. Callers seed this to a
    /// known-good starting point via [`Self::set_last_span`] before dispatching to a production
    /// that might fail partway through, then read it back via [`Self::last_span`] after a
    /// failure — the result is the span of whatever the failed production genuinely consumed
    /// before giving up (or the seeded starting point, if it consumed nothing at all).
    last_span: Span,
}

impl TokenCursor {
    /// Creates a cursor over `tokens`, at nesting depth 0.
    pub(crate) fn new(tokens: std::iter::Peekable<LexLexer>) -> Self {
        TokenCursor {
            tokens: Some(tokens),
            depth: 0,
            last_span: Span::call_site(),
        }
    }

    /// Returns the span of the last token [`Self::advance`] actually consumed since it was last
    /// set via [`Self::set_last_span`].
    pub(crate) fn last_span(&self) -> Span {
        self.last_span
    }

    /// Sets the span [`Self::last_span`] returns until the next token is consumed. Callers use
    /// this to seed a known starting point (e.g. a sheet item's own first token) immediately
    /// before dispatching to a production that might fail partway through, so a subsequent
    /// [`Self::last_span`] read reflects genuine progress within that one attempt rather than
    /// carrying over a stale value from whatever was parsed previously.
    pub(crate) fn set_last_span(&mut self, span: Span) {
        self.last_span = span;
    }

    /// Returns the cursor's current brace/bracket/paren nesting depth.
    ///
    /// - Postcondition: reflects only delimiters consumed via this cursor's own
    ///   `expect_open_brace`/`expect_close_brace`/`expect_open_bracket`/`expect_close_bracket`/
    ///   `expect_open_paren`/`expect_close_paren` (and by [`skip_to_recovery_point`] internally);
    ///   unaffected by tokens the embedded CEL sub-parser consumes directly while it owns the
    ///   stream.
    pub(crate) fn depth(&self) -> i32 {
        self.depth
    }

    /// Takes the token stream, leaving `None` behind — used to hand the stream to an embedded
    /// `cel_parser::Parser` for one CEL sub-expression, then reclaim it via `set_tokens`.
    ///
    /// - Precondition: a token stream is set.
    pub(crate) fn take_tokens(&mut self) -> Option<std::iter::Peekable<LexLexer>> {
        self.tokens.take()
    }

    /// Restores a previously-taken token stream.
    pub(crate) fn set_tokens(&mut self, tokens: std::iter::Peekable<LexLexer>) {
        self.tokens = Some(tokens);
    }

    pub(crate) fn peek_token(&mut self) -> Option<&Token> {
        self.tokens.as_mut()?.peek()
    }

    pub(crate) fn advance(&mut self) -> Option<Token> {
        let token = self.tokens.as_mut()?.next()?;
        self.last_span = token.span();
        Some(token)
    }

    pub(crate) fn peek_span(&mut self) -> Span {
        self.tokens
            .as_mut()
            .and_then(|t| t.peek())
            .map(|t| t.span())
            .unwrap_or_else(Span::call_site)
    }

    pub(crate) fn err_at(&mut self, msg: impl Into<String>) -> ParseError {
        ParseError::new(msg.into(), self.peek_span())
    }

    /// Consumes and returns `true` if the next token is an identifier matching `kw`.
    pub(crate) fn is_keyword(&mut self, kw: &str) -> bool {
        let ok = matches!(
            self.tokens.as_mut().and_then(|t| t.peek()),
            Some(Token::Identifier(id)) if id == kw
        );
        if ok {
            self.advance();
        }
        ok
    }

    /// Consumes any identifier.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not an identifier.
    pub(crate) fn consume_ident(&mut self) -> Result<(String, Span)> {
        let span = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::Identifier(id)) => {
                let s = id.span();
                let _ = id;
                s
            }
            other => {
                let s = other.map(|t| t.span()).unwrap_or(Span::call_site());
                return Err(ParseError::new("expected identifier", s));
            }
        };
        if let Some(Token::Identifier(id)) = self.advance() {
            return Ok((id.to_string(), span));
        }
        unreachable!("peeked identifier, advance must return it")
    }

    /// Consumes a specific punctuation token.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token does not match `p`.
    pub(crate) fn expect_punct(&mut self, p: &str) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::Punct { op, span }) if op == p => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            Ok(span)
        } else {
            Err(ParseError::new(format!("expected `{p}`"), span))
        }
    }

    /// Consumes and returns `true` if the next token is punctuation matching `p`.
    pub(crate) fn consume_punct(&mut self, p: &str) -> bool {
        let ok = matches!(
            self.tokens.as_mut().and_then(|t| t.peek()),
            Some(Token::Punct { op, .. }) if op == p
        );
        if ok {
            self.advance();
        }
        ok
    }

    /// Consumes `{`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not `{`.
    ///
    /// - Postcondition: on success, increments [`Self::depth`] by 1.
    pub(crate) fn expect_open_brace(&mut self) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::OpenDelim {
                delimiter: Delimiter::Brace,
                span,
            }) => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            self.depth += 1;
            Ok(span)
        } else {
            Err(ParseError::new("expected `{`", span))
        }
    }

    /// Consumes `}`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not `}`.
    ///
    /// - Postcondition: on success, decrements [`Self::depth`] by 1.
    pub(crate) fn expect_close_brace(&mut self) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::CloseDelim {
                delimiter: Delimiter::Brace,
                span,
            }) => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            self.depth -= 1;
            Ok(span)
        } else {
            Err(ParseError::new("expected `}`", span))
        }
    }

    /// Consumes `(`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not `(`.
    ///
    /// - Postcondition: on success, increments [`Self::depth`] by 1.
    pub(crate) fn expect_open_paren(&mut self) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::OpenDelim {
                delimiter: Delimiter::Parenthesis,
                span,
            }) => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            self.depth += 1;
            Ok(span)
        } else {
            Err(ParseError::new("expected `(`", span))
        }
    }

    /// Returns whether the next token is `(`.
    pub(crate) fn at_open_paren(&mut self) -> bool {
        matches!(
            self.tokens.as_mut().and_then(|t| t.peek()),
            Some(Token::OpenDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            })
        )
    }

    /// Returns whether the next token is `)` (or the stream is exhausted).
    pub(crate) fn at_close_paren(&mut self) -> bool {
        matches!(
            self.tokens.as_mut().and_then(|t| t.peek()),
            Some(Token::CloseDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            }) | None
        )
    }

    /// Consumes `)`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not `)`.
    ///
    /// - Postcondition: on success, decrements [`Self::depth`] by 1.
    pub(crate) fn expect_close_paren(&mut self) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::CloseDelim {
                delimiter: Delimiter::Parenthesis,
                span,
            }) => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            self.depth -= 1;
            Ok(span)
        } else {
            Err(ParseError::new("expected `)`", span))
        }
    }

    /// Consumes and returns a literal token.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not a literal.
    pub(crate) fn consume_literal(&mut self) -> Result<(Literal, Span)> {
        let span = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::Literal(lit)) => lit.span(),
            other => {
                let s = other.map(|t| t.span()).unwrap_or(Span::call_site());
                return Err(ParseError::new("expected literal", s));
            }
        };
        if let Some(Token::Literal(lit)) = self.advance() {
            return Ok((lit, span));
        }
        unreachable!("peeked literal, advance must return it")
    }

    /// Consumes a `literal_pattern = ["-"] literal.` — Rust's own `LiteralPattern` grammar rule
    /// (a bare literal, or one directly negated by a leading `-`; see
    /// <https://doc.rust-lang.org/reference/patterns.html#literal-patterns>). The literal
    /// itself is returned unsigned; the leading `-`, if any, is reported separately since a
    /// `Literal` token never carries a sign.
    ///
    /// Returns `(negated, literal, pattern_span, literal_span)`: `pattern_span` is the span of
    /// the leading `-` when `negated`, otherwise equal to `literal_span`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if a leading `-` is not followed by a literal, if there is no literal at
    /// all, if the literal itself isn't one `cel_parser` can represent (unrecognized numeric
    /// suffix, or a value out of range for its width), or if a leading `-` negates a literal
    /// `cel_parser` has no unary `-` overload for (any non-numeric literal, or an integer
    /// literal with an unsigned suffix) — mirroring the runtime parser's
    /// `cel_parser::Parser::parse_literal_pattern`, which rejects the same cases via
    /// `push_literal_token`/its operator table.
    pub(crate) fn consume_literal_pattern(&mut self) -> Result<(bool, Literal, Span, Span)> {
        let minus_span = self.peek_span();
        let negated = self.consume_punct("-");
        let (lit, lit_span) = self.consume_literal()?;
        cel_parser::validate_literal(&lit)?;
        if negated && !literal_can_be_negated(&lit) {
            return Err(ParseError::new(
                "literal pattern: `-` can only negate a signed integer or float literal",
                minus_span,
            ));
        }
        Ok((
            negated,
            lit,
            if negated { minus_span } else { lit_span },
            lit_span,
        ))
    }

    /// Consumes a leading run of consecutive `Token::DocComment` tokens matching `inner`,
    /// returning their joined text (`\n`-separated) and the first token's span, or `None` if the
    /// next token isn't a matching doc comment.
    ///
    /// - Complexity: O(k), where k is the number of consecutive matching doc-comment tokens.
    pub(crate) fn consume_doc_comment_run(&mut self, inner: bool) -> Option<(String, Span)> {
        let first_span = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::DocComment { inner: i, span, .. }) if *i == inner => Some(*span),
            _ => None,
        }?;
        let mut lines = Vec::new();
        loop {
            let next_text = match self.tokens.as_mut().and_then(|t| t.peek()) {
                Some(Token::DocComment { inner: i, text, .. }) if *i == inner => Some(text.clone()),
                _ => None,
            };
            match next_text {
                Some(text) => {
                    lines.push(text);
                    self.advance();
                }
                None => break,
            }
        }
        Some((lines.join("\n"), first_span))
    }

    pub(crate) fn at_close_brace(&mut self) -> bool {
        matches!(
            self.tokens.as_mut().and_then(|t| t.peek()),
            Some(Token::CloseDelim {
                delimiter: Delimiter::Brace,
                ..
            }) | None
        )
    }

    /// Skips tokens until a declaration-boundary recovery point relative to `target_depth` — the
    /// cursor's [`Self::depth`] as observed by the caller *before* it dispatched to the
    /// production that failed. A recovery point is: a `;` seen while at or below `target_depth`
    /// (consumed); a `}` that closes back to at or below `target_depth` (not consumed, so the
    /// caller's `at_close_brace` check still sees it); or the `cell`/`relationship`/`conditional`
    /// keyword that starts the next sheet item, seen while at or below `target_depth` (not
    /// consumed).
    ///
    /// The failed production may have already consumed one or more of its own opening delimiters
    /// before the error occurred (e.g. a malformed `relationship { .. }`'s own `{`) — the running
    /// [`Self::depth`] this method reads and updates reflects that, so this method first skips
    /// back out through those still-open delimiters before applying the stopping conditions.
    /// Comparing with `<=` rather than strict equality is a defensive guard: malformed input with
    /// excess closing delimiters could otherwise dip `depth` below `target_depth` and never
    /// satisfy an exact-equality check.
    ///
    /// `Delimiter::Brace`, `Delimiter::Bracket`, and `Delimiter::Parenthesis` all affect
    /// [`Self::depth`] here, mirroring `expect_open_brace`/`expect_close_brace`/
    /// `expect_open_bracket`/`expect_close_bracket`/`expect_open_paren`/`expect_close_paren` —
    /// `type_expr` is the one adam-lang-grammar production that uses parens, so a malformed
    /// `type_expr`'s own unmatched `(`/`)` must unwind `depth` the same way a malformed
    /// `relationship`/`conditional` body or `out`'s `require` body's brace does.
    /// `Delimiter::None` is treated as an ordinary token (consumed, no depth change): it never
    /// appears in adam-lang's own grammar, or in a way `cel_parser` leaves dangling.
    ///
    /// **Known limitation (accepted scope boundary):** because CEL's own grammar reuses these
    /// same delimiter kinds for its internal grouping — `Delimiter::Brace` for `if`/`else`
    /// branches, `Delimiter::Parenthesis` for tuple/group literals — a CEL sub-expression that
    /// fails partway through (leaving a dangling, unmatched brace or paren behind) is
    /// indistinguishable, by delimiter kind alone, from a real adam-lang-tracked brace or paren.
    /// A CEL `if` expression whose then-branch fails to parse (`if a { }`) leaves a stray `}`
    /// this way; a CEL tuple/group literal that fails partway through (`(+)`, where
    /// `is_tuple_or_group` consumes `(` but the error occurs before its matching `)` is ever
    /// reached) leaves a stray `)` the same way. In either case this method (and the recovery it
    /// drives) may stop one delimiter too early, mistaking the stray one for a real
    /// adam-lang-tracked one, aborting the whole parse with `Err` rather than isolating just the
    /// one malformed item. Fixing this in general requires `cel_parser`'s `Parser<C>` to report
    /// back exactly what it left unbalanced on a failed parse — out of scope here; see the
    /// tracking issue for the general fix: <https://github.com/stlab/cel-rs/issues/43>.
    ///
    /// The keyword check matters when the malformed item has no `;` of its own — e.g.
    /// `cell bad unknown_syntax` immediately followed by a sibling `cell` declaration — so
    /// recovery stops before the next item instead of skipping past it in search of a `;`
    /// belonging to that sibling. Used only by [`crate::AdamAstParser`]'s coarse error recovery.
    ///
    /// `fallback` is returned as-is when this method consumes zero tokens — i.e. the very next
    /// token already satisfies a stopping condition (most commonly: the failed production
    /// didn't consume the token that turned out to be a sibling `cell`/`relationship`/
    /// `conditional` keyword). Without this, the first stopping check's `return last` would
    /// return `last`'s pre-loop initial value, which — if nothing has been consumed yet — would
    /// be the *next* item's own first token's span, not any part of the item that actually
    /// failed to parse. Callers should pass [`Self::last_span`] (read *after* the failed
    /// production returns), seeded via [`Self::set_last_span`] to the failed item's own first
    /// token immediately before dispatching to that production: this way, a zero-tokens-skipped
    /// recovery reports the last token the failed production genuinely consumed before giving up
    /// (e.g. a malformed `cell bad relationship { .. }`'s `Error` span covers `cell bad`, not
    /// just `cell`) — never the seeded starting point alone when more was actually parsed, and
    /// never overlapping into the sibling item that follows.
    ///
    /// - Precondition: `target_depth` is the value [`Self::depth`] held immediately before the
    ///   caller dispatched to the production that produced the error being recovered from.
    /// - Postcondition: returns the span of the last token actually consumed by this call, or
    ///   `fallback` if it consumed none, so an `Error` placeholder node can cover the skipped
    ///   range without ever extending past the start of the next, unconsumed token.
    /// - Postcondition: [`Self::depth`] is left at (or, only on malformed input, possibly below)
    ///   `target_depth`, kept consistent with every `OpenDelim`/`CloseDelim` consumed here.
    ///
    /// - Complexity: O(n) in the number of tokens skipped.
    pub(crate) fn skip_to_recovery_point(&mut self, target_depth: i32, fallback: Span) -> Span {
        let mut last = fallback;
        loop {
            let at_or_below_target = self.depth <= target_depth;
            match self.peek_token() {
                None => return last,
                Some(Token::CloseDelim {
                    delimiter: Delimiter::Brace | Delimiter::Bracket | Delimiter::Parenthesis,
                    ..
                }) if at_or_below_target => return last,
                Some(Token::CloseDelim {
                    delimiter: Delimiter::Brace | Delimiter::Bracket | Delimiter::Parenthesis,
                    ..
                }) => {
                    self.depth -= 1;
                    last = self.peek_span();
                    self.advance();
                }
                Some(Token::OpenDelim {
                    delimiter: Delimiter::Brace | Delimiter::Bracket | Delimiter::Parenthesis,
                    ..
                }) => {
                    self.depth += 1;
                    last = self.peek_span();
                    self.advance();
                }
                Some(Token::Punct { op, .. }) if op == ";" && at_or_below_target => {
                    last = self.peek_span();
                    self.advance();
                    return last;
                }
                Some(Token::Identifier(id))
                    if at_or_below_target
                        && (id == "cell"
                            || id == "relationship"
                            || id == "conditional"
                            || id == "out") =>
                {
                    return last;
                }
                _ => {
                    last = self.peek_span();
                    self.advance();
                }
            }
        }
    }
}

/// Returns whether `cel_parser` registers a unary `-` overload for `lit`'s type — the same
/// numeric-only, signed-only rule `cel_parser::Parser::parse_literal_pattern` enforces via its
/// operator table (see
/// `builtin_operand_types_includes_unary_negation_but_only_for_signed_and_float_types` in
/// `cel_parser::op_table`): any float literal, or an integer literal with no suffix (defaults to
/// `i32`) or an explicitly signed suffix. An integer literal with an unsigned suffix, and every
/// non-numeric literal (bool, string, char, byte), has no such overload.
fn literal_can_be_negated(lit: &Literal) -> bool {
    match lit {
        Literal::Int(int) => !matches!(
            int.suffix(),
            "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
        ),
        Literal::Float(_) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn expect_open_paren_increments_depth() {
        let stream = proc_macro2::TokenStream::from_str("( )").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        assert_eq!(cursor.depth(), 0);
        cursor.expect_open_paren().unwrap();
        assert_eq!(cursor.depth(), 1);
    }

    #[test]
    fn expect_close_paren_decrements_depth() {
        let stream = proc_macro2::TokenStream::from_str("( )").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        cursor.expect_open_paren().unwrap();
        cursor.expect_close_paren().unwrap();
        assert_eq!(cursor.depth(), 0);
    }

    #[test]
    fn at_open_paren_is_true_at_an_open_paren() {
        let stream = proc_macro2::TokenStream::from_str("( )").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        assert!(cursor.at_open_paren());
    }

    #[test]
    fn at_open_paren_is_false_after_the_open_paren_is_consumed() {
        let stream = proc_macro2::TokenStream::from_str("( )").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        cursor.expect_open_paren().unwrap();
        assert!(!cursor.at_open_paren());
    }

    #[test]
    fn at_close_paren_is_true_at_a_close_paren() {
        // `proc_macro2::TokenStream::from_str` tokenizes delimiters as a matched tree up front, so
        // a source string with a truly unmatched `)` (with no corresponding `(` anywhere in the
        // string) fails to tokenize at all rather than producing a lone `CloseDelim` token —
        // exercise the true case of the postcondition against a balanced `( )` pair instead,
        // advancing past the `(` first.
        let stream = proc_macro2::TokenStream::from_str("( )").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        cursor.expect_open_paren().unwrap();
        assert!(cursor.at_close_paren());
    }

    #[test]
    fn at_close_paren_is_true_at_end_of_input() {
        let stream = proc_macro2::TokenStream::from_str("").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        assert!(cursor.at_close_paren());
    }

    #[test]
    fn consume_literal_pattern_accepts_a_bare_literal() {
        let stream = proc_macro2::TokenStream::from_str("5i32").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        let (negated, ..) = cursor.consume_literal_pattern().unwrap();
        assert!(!negated);
    }

    #[test]
    fn consume_literal_pattern_accepts_a_negated_signed_integer() {
        let stream = proc_macro2::TokenStream::from_str("-5i32").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        let (negated, ..) = cursor.consume_literal_pattern().unwrap();
        assert!(negated);
    }

    #[test]
    fn consume_literal_pattern_accepts_a_negated_unsuffixed_integer() {
        let stream = proc_macro2::TokenStream::from_str("-5").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        assert!(cursor.consume_literal_pattern().is_ok());
    }

    #[test]
    fn consume_literal_pattern_accepts_a_negated_float() {
        let stream = proc_macro2::TokenStream::from_str("-1.5f64").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        assert!(cursor.consume_literal_pattern().is_ok());
    }

    #[test]
    fn consume_literal_pattern_rejects_a_negated_unsigned_integer() {
        let stream = proc_macro2::TokenStream::from_str("-5u32").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        assert!(cursor.consume_literal_pattern().is_err());
    }

    #[test]
    fn consume_literal_pattern_rejects_a_negated_bool() {
        let stream = proc_macro2::TokenStream::from_str("-true").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        assert!(cursor.consume_literal_pattern().is_err());
    }

    #[test]
    fn consume_doc_comment_run_returns_none_when_next_token_is_not_a_doc_comment() {
        let stream = proc_macro2::TokenStream::from_str("cell").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        assert!(cursor.consume_doc_comment_run(false).is_none());
    }

    #[test]
    fn consume_doc_comment_run_joins_consecutive_matching_doc_comments() {
        let stream = proc_macro2::TokenStream::from_str("/// a\n/// b\ncell").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        let (text, _) = cursor
            .consume_doc_comment_run(false)
            .expect("doc comment run");
        assert_eq!(text, " a\n b");
        assert!(cursor.is_keyword("cell"));
    }

    #[test]
    fn consume_doc_comment_run_does_not_consume_a_mismatched_inner_flag() {
        let stream = proc_macro2::TokenStream::from_str("//! inner\ncell").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        assert!(cursor.consume_doc_comment_run(false).is_none());
        let (text, _) = cursor
            .consume_doc_comment_run(true)
            .expect("doc comment run");
        assert_eq!(text, " inner");
    }
}
