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
}

impl TokenCursor {
    /// Creates a cursor over `tokens`.
    pub(crate) fn new(tokens: std::iter::Peekable<LexLexer>) -> Self {
        TokenCursor {
            tokens: Some(tokens),
        }
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

    /// Skips tokens until a declaration-boundary recovery point: a `;` at the current nesting
    /// depth (consumed); a `}` that closes back to the current nesting depth (not consumed, so
    /// the caller's `at_close_brace` check still sees it); or the `cell`/`relationship`/
    /// `conditional` keyword that starts the next sheet item, at the current nesting depth (not
    /// consumed). The keyword check matters when the malformed item has no `;` of its own — e.g.
    /// `cell bad unknown_syntax` immediately followed by a sibling `cell` declaration — so
    /// recovery stops before the next item instead of skipping past it in search of a `;`
    /// belonging to that sibling. Used only by [`crate::PmAstParser`]'s coarse error recovery.
    ///
    /// - Postcondition: returns the span of the last token inspected, so an `Error` placeholder
    ///   node can cover the skipped range.
    ///
    /// - Complexity: O(n) in the number of tokens skipped.
    pub(crate) fn skip_to_recovery_point(&mut self) -> Span {
        let mut last = self.peek_span();
        let mut depth: i32 = 0;
        loop {
            match self.peek_token() {
                None => return last,
                Some(Token::CloseDelim { .. }) if depth == 0 => return last,
                Some(Token::CloseDelim { .. }) => {
                    depth -= 1;
                    last = self.peek_span();
                    self.advance();
                }
                Some(Token::OpenDelim { .. }) => {
                    depth += 1;
                    last = self.peek_span();
                    self.advance();
                }
                Some(Token::Punct { op, .. }) if op == ";" && depth == 0 => {
                    last = self.peek_span();
                    self.advance();
                    return last;
                }
                Some(Token::Identifier(id))
                    if depth == 0
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
