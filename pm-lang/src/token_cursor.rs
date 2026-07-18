//! Pure token-stream cursor shared by pm-lang's `Sheet`-building parser (`parser.rs`) and its
//! AST-building parser (`ast_parser.rs`), so tokenizing/peeking/expecting logic — which has no
//! dependency on what each parser *builds* — is written exactly once.

use cel_parser::ParseError;
use cel_parser::lex_lexer::{HasSpan, LexLexer, Literal, Token};
use proc_macro2::{Delimiter, Span};

/// Parser result type, matching `cel_parser::ParseError`.
type Result<T> = std::result::Result<T, ParseError>;

/// A peekable pm-lang token stream plus the primitive lookahead/consume operations every
/// pm-lang grammar production needs, independent of what each production builds (a live
/// `property_model::Sheet` mutation, or a syntax tree node).
pub(crate) struct TokenCursor {
    tokens: Option<std::iter::Peekable<LexLexer>>,
    /// Running brace/bracket nesting depth, incremented/decremented only by this cursor's own
    /// `expect_open_brace`/`expect_close_brace`/`expect_open_bracket`/`expect_close_bracket`
    /// (brace and bracket delimiters are tracked uniformly as one counter). Tokens consumed
    /// directly by an embedded `cel_parser::Parser` while it temporarily owns the stream (see
    /// `take_tokens`/`set_tokens`) never pass through these methods, so they don't affect this
    /// counter — which is exactly what callers like [`skip_to_recovery_point`] need: a depth
    /// that reflects only pm-lang-grammar nesting, not CEL sub-expression internals.
    depth: i32,
}

impl TokenCursor {
    /// Creates a cursor over `tokens`, at nesting depth 0.
    pub(crate) fn new(tokens: std::iter::Peekable<LexLexer>) -> Self {
        TokenCursor {
            tokens: Some(tokens),
            depth: 0,
        }
    }

    /// Returns the cursor's current brace/bracket nesting depth.
    ///
    /// - Postcondition: reflects only delimiters consumed via this cursor's own
    ///   `expect_open_brace`/`expect_close_brace`/`expect_open_bracket`/`expect_close_bracket`
    ///   (and by [`skip_to_recovery_point`] internally); unaffected by tokens the embedded CEL
    ///   sub-parser consumes directly while it owns the stream.
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
        self.tokens.as_mut()?.next()
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

    /// Consumes `[`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not `[`.
    ///
    /// - Postcondition: on success, increments [`Self::depth`] by 1.
    pub(crate) fn expect_open_bracket(&mut self) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::OpenDelim {
                delimiter: Delimiter::Bracket,
                span,
            }) => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            self.depth += 1;
            Ok(span)
        } else {
            Err(ParseError::new("expected `[`", span))
        }
    }

    /// Consumes `]`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not `]`.
    ///
    /// - Postcondition: on success, decrements [`Self::depth`] by 1.
    pub(crate) fn expect_close_bracket(&mut self) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::CloseDelim {
                delimiter: Delimiter::Bracket,
                span,
            }) => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            self.depth -= 1;
            Ok(span)
        } else {
            Err(ParseError::new("expected `]`", span))
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
    /// The keyword check matters when the malformed item has no `;` of its own — e.g.
    /// `cell bad unknown_syntax` immediately followed by a sibling `cell` declaration — so
    /// recovery stops before the next item instead of skipping past it in search of a `;`
    /// belonging to that sibling. Used only by [`crate::PmAstParser`]'s coarse error recovery.
    ///
    /// - Precondition: `target_depth` is the value [`Self::depth`] held immediately before the
    ///   caller dispatched to the production that produced the error being recovered from.
    /// - Postcondition: returns the span of the last token inspected, so an `Error` placeholder
    ///   node can cover the skipped range.
    /// - Postcondition: [`Self::depth`] is left at (or, only on malformed input, possibly below)
    ///   `target_depth`, kept consistent with every `OpenDelim`/`CloseDelim` consumed here.
    ///
    /// - Complexity: O(n) in the number of tokens skipped.
    pub(crate) fn skip_to_recovery_point(&mut self, target_depth: i32) -> Span {
        let mut last = self.peek_span();
        loop {
            let at_or_below_target = self.depth <= target_depth;
            match self.peek_token() {
                None => return last,
                Some(Token::CloseDelim { .. }) if at_or_below_target => return last,
                Some(Token::CloseDelim { .. }) => {
                    self.depth -= 1;
                    last = self.peek_span();
                    self.advance();
                }
                Some(Token::OpenDelim { .. }) => {
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
                        && (id == "cell" || id == "relationship" || id == "conditional") =>
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
