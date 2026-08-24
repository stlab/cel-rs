//! A recursive descent parser for CEL (Common Expression Language) expressions.
//!
//! This crate provides a parser that can parse CEL expressions into executable segments.
//! The parser follows the CEL grammar specification and provides detailed error reporting
//! with source location information.
//!
//! # Error Handling
//!
//! Parse errors are returned as [`ParseError`], which carries a `proc_macro2::Span` for precise diagnostics.
//! Convert to [`CELError`] (via `From`) when the error must be stored or sent across thread boundaries.
//! All errors result from malformed input (syntax errors, type mismatches, undefined identifiers).
//!
//! # Grammar
//!
//! ```text
//! expression = or_expression ?eos?.
//! or_expression = and_expression { "||" and_expression }.
//! and_expression = comparison_expression { "&&" comparison_expression }.
//! comparison_expression = bitwise_or_expression
//!     [ ("==" | "!=" | "<" | ">" | "<=" | ">=") bitwise_or_expression ].
//! bitwise_or_expression = bitwise_xor_expression { "|" bitwise_xor_expression }.
//! bitwise_xor_expression = bitwise_and_expression { "^" bitwise_and_expression }.
//! bitwise_and_expression = bitwise_shift_expression { "&" bitwise_shift_expression }.
//! bitwise_shift_expression = additive_expression { ("<<" | ">>") additive_expression }.
//! additive_expression = multiplicative_expression { ("+" | "-") multiplicative_expression }.
//! multiplicative_expression = cast_expression { ("*" | "/" | "%") cast_expression }.
//! cast_expression = unary_expression { "as" identifier }.
//! unary_expression = (("-" | "!") unary_expression) | postfix_expression.
//! postfix_expression = primary_expression { "(" [ parameter_list ] ")" | "." unsuffixed_integer }.
//! primary_expression = literal | identifier | tuple_or_group | if_expression | closure_expression.
//! tuple_or_group = "(" [ or_expression ["," [ or_expression { "," or_expression } ]] ] ")".
//! if_expression = "if" or_expression "{" or_expression "}" [ "else" ( "{" or_expression "}" | if_expression ) ].
//! closure_expression = ("||" | "|" [ closure_param { "," closure_param } ] "|") expression.
//! closure_param = identifier ":" closure_type_expression.
//! closure_type_expression = identifier | "(" [ closure_type_expression { "," closure_type_expression } ] ")".
//! parameter_list = or_expression { "," or_expression }.
//! ```
//!
//! # Note
//!
//! `?eos?` denotes end of stream.
//!
//! # Examples
//!
//! ```rust
//! use cel_parser::{CELParser, OpLookup};
//!
//! let mut segment = CELParser::new(OpLookup::new()).parse_str("10u32 + 20u32 * 5u32").unwrap();
//! let result = segment.call0::<u32>();
//! assert!(result.is_ok());
//! assert_eq!(result.unwrap(), 110); // 10 + 20 * 5 = 10 + 100
//! ```
//!
//! ## Basic Usage
//!
//! ```rust
//! use cel_parser::CELParser;
//! use cel_parser::OpLookup;
//! use proc_macro2::TokenStream;
//! use std::str::FromStr;
//!
//! let input = TokenStream::from_str("10").unwrap();
//! let mut parser = CELParser::new(OpLookup::new());
//! parser.set_tokens(input.into_iter());
//! let result = parser.is_expression();
//! assert!(result.is_ok());
//! ```
//!
//! ## Error Formatting
//!
//! ```rust
//! use annotate_snippets::Renderer;
//! use cel_parser::CELParser;
//! use cel_parser::OpLookup;
//! use proc_macro2::TokenStream;
//! use std::str::FromStr;
//!
//! let line = line!() + 1;
//! let source = r#"
//!   10 20
//! "#; // Invalid: missing operator
//! let input = TokenStream::from_str(source).unwrap();
//! let mut parser = CELParser::new(OpLookup::new());
//! parser.set_tokens(input.into_iter());
//!
//! if let Err(e) = parser.is_expression() {
//!     // Format error starting at line 1
//!     println!("{}", e.format_rustc_style(source, file!(), line, &Renderer::plain()));
//! }
//! ```

pub mod ast;
mod error;
mod fmt;
pub mod lex_lexer;
pub mod op_table;
pub mod parser_context;
pub mod ty;

pub use ast::{AstContext, ClosureParam, ClosureParamTypeExpr, Expr, ExprSpan, Literal, LogicalOp};
pub use error::{CELError, FormatRustcStyle, ParseError, SourceSpan, SpanContext};
pub use fmt::format_expr;
pub use op_table::{OpLookup, OperandTypes, builtin_operand_types};
pub use parser_context::{DynSegmentContext, ParserContext};
pub use proc_macro2::LineColumn;
pub use ty::Ty;

use lex_lexer::{LexLexer, Literal as CelLiteral, Token, TokenStreamIter};

use cel_runtime::DynSegment;
use proc_macro2::{Delimiter, Span, TokenStream};
use std::any::TypeId;
use std::collections::HashMap;
use std::iter::Peekable;
use std::str::FromStr;

/// Parser result type.
pub type Result<T> = std::result::Result<T, ParseError>;

/// Pushes a literal value from `token` onto `output`.
///
/// # Errors
///
/// Returns `Err` if the literal type is unsupported or if a suffixed numeric
/// literal cannot be parsed.
fn push_literal_token<C: ParserContext>(output: &mut C, lit: CelLiteral) -> Result<()> {
    match lit {
        CelLiteral::Int(integer) => {
            let span = integer.span();
            match integer.suffix() {
                "" | "i32" => output.push_literal(
                    integer.base10_parse::<i32>().map_err(|e| {
                        ParseError::new(
                            format!("invalid i32 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "u8" => output.push_literal(
                    integer.base10_parse::<u8>().map_err(|e| {
                        ParseError::new(
                            format!("invalid u8 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "u16" => output.push_literal(
                    integer.base10_parse::<u16>().map_err(|e| {
                        ParseError::new(
                            format!("invalid u16 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "u32" => output.push_literal(
                    integer.base10_parse::<u32>().map_err(|e| {
                        ParseError::new(
                            format!("invalid u32 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "u64" => output.push_literal(
                    integer.base10_parse::<u64>().map_err(|e| {
                        ParseError::new(
                            format!("invalid u64 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "u128" => output.push_literal(
                    integer.base10_parse::<u128>().map_err(|e| {
                        ParseError::new(
                            format!("invalid u128 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "usize" => output.push_literal(
                    integer.base10_parse::<usize>().map_err(|e| {
                        ParseError::new(
                            format!("invalid usize literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "i8" => output.push_literal(
                    integer.base10_parse::<i8>().map_err(|e| {
                        ParseError::new(
                            format!("invalid i8 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "i16" => output.push_literal(
                    integer.base10_parse::<i16>().map_err(|e| {
                        ParseError::new(
                            format!("invalid i16 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "i64" => output.push_literal(
                    integer.base10_parse::<i64>().map_err(|e| {
                        ParseError::new(
                            format!("invalid i64 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "i128" => output.push_literal(
                    integer.base10_parse::<i128>().map_err(|e| {
                        ParseError::new(
                            format!("invalid i128 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "isize" => output.push_literal(
                    integer.base10_parse::<isize>().map_err(|e| {
                        ParseError::new(
                            format!("invalid isize literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                suffix => {
                    return Err(ParseError::new(
                        format!("invalid integer literal suffix: `{suffix}`"),
                        integer.span(),
                    ));
                }
            };
        }
        CelLiteral::Float(float) => {
            let span = float.span();
            match float.suffix() {
                "" | "f64" => output.push_literal(
                    float.base10_parse::<f64>().map_err(|e| {
                        ParseError::new(format!("invalid f64 literal `{float}`: {e}"), float.span())
                    })?,
                    span,
                ),
                "f32" => output.push_literal(
                    float.base10_parse::<f32>().map_err(|e| {
                        ParseError::new(format!("invalid f32 literal `{float}`: {e}"), float.span())
                    })?,
                    span,
                ),
                suffix => {
                    return Err(ParseError::new(
                        format!("invalid float literal suffix: `{suffix}`"),
                        float.span(),
                    ));
                }
            };
        }
        CelLiteral::Str(string) => {
            let span = string.span();
            output.push_literal(string.value(), span);
        }
        CelLiteral::Bool(lit_bool) => {
            let span = lit_bool.span();
            output.push_literal(lit_bool.value, span);
        }
        CelLiteral::Char(ch) => {
            let span = ch.span();
            output.push_literal(ch.value(), span);
        }
        CelLiteral::Byte(byte) => {
            let span = byte.span();
            output.push_literal(byte.value(), span);
        }
        CelLiteral::ByteStr(byte_str) => {
            let span = byte_str.span();
            output.push_literal(byte_str.value(), span);
        }
        CelLiteral::CStr(c_str) => {
            let span = c_str.span();
            output.push_literal(c_str.value(), span);
        }
        other => {
            return Err(ParseError::new(
                format!("unsupported literal: {other:?}"),
                other.span(),
            ));
        }
    }
    Ok(())
}

/// One resolved closure parameter type: a built-in scalar, or a (possibly nested) tuple of them
/// — the result of resolving a `closure_type_expression` production (see
/// [`Parser::parse_closure_type_expression`]).
enum ClosureParamType {
    /// A single built-in scalar type (e.g. `i32`, `bool`).
    Scalar(crate::op_table::BuiltinScalarType),
    /// A (possibly nested) tuple of closure parameter types.
    Tuple(Vec<ClosureParamType>),
}

impl ClosureParamType {
    /// Returns the `TypeId` this parameter type resolves to for [`cel_runtime::DynClosure`]'s
    /// declared parameter list.
    ///
    /// Every tuple shape resolves to [`cel_runtime::DynamicSequence`]'s `TypeId` — the actual
    /// runtime type of the boxed `dyn Any` argument a caller must hand to
    /// [`cel_runtime::DynClosure::call`]/`call_boxed` for this parameter, matching what
    /// [`is_closure_expression`](Self::is_closure_expression) pushes via
    /// `DynSegment::push_arg_as_dynamic_sequence_tuple`. [`cel_runtime::DynTuple`] is a distinct,
    /// stack-internal marker type used only for compile-time tuple-boundary tracking inside a
    /// `DynSegment`'s own operand stack — it is never the type of a boxed argument crossing this
    /// boundary, so using it here would make every tuple-typed closure parameter's declared type
    /// unmatchable against its real argument. The tuple's real element shape lives in the
    /// `AssociatedType` list built by [`elements_to_associated`], not in this `TypeId`.
    fn type_id(&self) -> TypeId {
        match self {
            ClosureParamType::Scalar(s) => s.type_id,
            ClosureParamType::Tuple(_) => TypeId::of::<cel_runtime::DynamicSequence>(),
        }
    }
}

/// Builds a fresh `AssociatedType` prototype list from resolved closure parameter element
/// types, for [`cel_runtime::DynSegment::push_arg_as_dynamic_sequence_tuple`] — leaf
/// `size`/`align` are the scalar's real values (that method's own precondition), while a nested
/// tuple's are placeholders (`push_arg_as_dynamic_sequence_tuple` recomputes them recursively
/// from `associated`).
///
/// - Complexity: O(n) in the total (nested) element count.
fn elements_to_associated(elements: &[ClosureParamType]) -> Vec<cel_runtime::AssociatedType> {
    elements
        .iter()
        .map(|ty| match ty {
            ClosureParamType::Scalar(s) => cel_runtime::AssociatedType {
                type_id: s.type_id,
                type_name: std::borrow::Cow::Borrowed(s.type_name),
                offset: 0,
                size: s.size,
                align: s.align,
                dropper: s.dropper,
                associated: Vec::new(),
            },
            ClosureParamType::Tuple(nested) => cel_runtime::AssociatedType {
                type_id: TypeId::of::<cel_runtime::DynTuple>(),
                type_name: std::borrow::Cow::Borrowed("tuple"),
                offset: 0,
                size: 0,
                align: 1,
                dropper: cel_runtime::drop_tuple,
                associated: elements_to_associated(nested),
            },
        })
        .collect()
}

/// A recursive descent parser for expressions, generic over the [`ParserContext`] it emits
/// into.
///
/// [`CELParser`] is a type alias for `Parser<DynSegmentContext>` and remains the concrete type
/// most callers use; it behaves identically to how `CELParser` always has, before this type
/// became generic.
///
/// # Examples
///
/// ## Basic Usage
///
/// ```rust
/// use cel_parser::OpLookup;
/// use cel_parser::CELParser;
/// use proc_macro2::TokenStream;
/// use std::str::FromStr;
///
/// let input = TokenStream::from_str("10").unwrap();
/// let mut parser = CELParser::new(OpLookup::new());
/// parser.set_tokens(input.into_iter());
/// let result = parser.is_expression();
/// assert!(result.is_ok());
/// ```
///
/// ## Error Formatting
///
/// ```rust
/// use annotate_snippets::Renderer;
/// use cel_parser::OpLookup;
/// use cel_parser::CELParser;
/// use proc_macro2::TokenStream;
/// use std::str::FromStr;
///
/// let line = line!() + 1;
/// let source = r#"
///   10 + 20 30
/// "#; // Invalid: missing operator
/// let input = TokenStream::from_str(source).unwrap();
/// let mut parser = CELParser::new(OpLookup::new());
/// parser.set_tokens(input.into_iter());
///
/// if let Err(e) = parser.is_expression() {
///     // Format error starting at line 1
///     println!("{}", e.format_rustc_style(source, file!(), line, &Renderer::plain()));
/// }
/// ```
pub struct Parser<C: ParserContext> {
    tokens: Option<Peekable<LexLexer>>,
    context: C,
    op_lookup: OpLookup,
    last_span: Span,
}

/// A recursive descent parser that executes directly into a [`DynSegment`].
///
/// This is the parser every existing caller uses; behavior is unchanged from before [`Parser`]
/// became generic over [`ParserContext`].
pub type CELParser = Parser<DynSegmentContext>;

impl<C: ParserContext> Parser<C> {
    /// Creates a new CEL parser with the given operation lookup.
    ///
    /// No tokens are set at construction; use [`set_tokens`](Self::set_tokens),
    /// [`parse_tokens_ctx`](Self::parse_tokens_ctx), or [`parse_str_ctx`](Self::parse_str_ctx)
    /// to parse.
    ///
    /// # Arguments
    ///
    /// * `op_lookup` - Operation lookup for resolving operators and identifiers
    pub fn new(op_lookup: OpLookup) -> Self {
        Parser {
            tokens: None,
            context: C::new_context(),
            op_lookup,
            last_span: Span::call_site(),
        }
    }

    /// Sets the token stream for parsing, resetting internal state.
    ///
    /// Call before [`is_expression`](Self::is_expression) or use
    /// [`parse_tokens_ctx`](Self::parse_tokens_ctx) which sets tokens and parses in one step.
    pub fn set_tokens(&mut self, tokens: TokenStreamIter) {
        self.tokens = Some(LexLexer::new(tokens).peekable());
        self.context = C::new_context();
        self.last_span = Span::call_site();
    }

    /// Sets the token stream from an existing [`LexLexer`] iterator for inline expression parsing.
    ///
    /// Resets the context. Use together with [`parse_or_expression_ctx`](Self::parse_or_expression_ctx)
    /// and [`take_lex_tokens`](Self::take_lex_tokens) to share a token stream between adam-lang and
    /// [`CELParser`].
    pub fn set_lex_tokens(&mut self, tokens: std::iter::Peekable<lex_lexer::LexLexer>) {
        self.tokens = Some(tokens);
        self.context = C::new_context();
        self.last_span = Span::call_site();
    }

    /// Parses one `or_expression` from the current token stream and returns the built context.
    ///
    /// Unlike [`parse_str_ctx`](Self::parse_str_ctx), this method does not require
    /// end-of-stream, allowing adam-lang to parse an expression embedded within a larger token
    /// stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the input does not contain a valid `or_expression`.
    ///
    /// - Complexity: O(n) in the number of tokens in the expression.
    pub fn parse_or_expression_ctx(&mut self) -> Result<C> {
        if !self.is_or_expression()? {
            return Err(self.error_at("expression expected"));
        }
        Ok(std::mem::replace(&mut self.context, C::new_context()))
    }

    /// Compiles a fully independent nested context — used for a closure literal's body — by
    /// swapping `self.context` out for a fresh one, running `f` against it, then swapping the
    /// original context back in and returning the finished nested one.
    ///
    /// Unlike [`parse_or_expression_ctx`](Self::parse_or_expression_ctx), this does not reset
    /// `self.tokens`/`self.op_lookup`/`self.last_span` — it's for compiling a sub-expression in the
    /// middle of an already-in-progress outer parse, not starting a fresh top-level parse.
    ///
    /// # Errors
    ///
    /// Returns `Err` if `f` does, or if `f` returns `Ok(false)` (no expression found) — in both
    /// cases the outer context is still restored before returning.
    ///
    /// - Complexity: whatever `f`'s own parse cost is.
    pub(crate) fn parse_nested_context<F>(&mut self, f: F) -> Result<C>
    where
        F: FnOnce(&mut Self) -> Result<bool>,
    {
        let outer = std::mem::replace(&mut self.context, C::new_context());
        let outcome = f(self);
        let nested = std::mem::replace(&mut self.context, outer);
        match outcome {
            Ok(true) => Ok(nested),
            Ok(false) => Err(self.error_at("expected expression")),
            Err(e) => Err(e),
        }
    }

    /// Returns the remaining token stream after expression parsing.
    ///
    /// Call after [`parse_or_expression_ctx`](Self::parse_or_expression_ctx) to recover the
    /// shared [`LexLexer`] for continued adam-lang parsing.
    pub fn take_lex_tokens(&mut self) -> Option<std::iter::Peekable<lex_lexer::LexLexer>> {
        self.tokens.take()
    }

    /// Parses a token stream into a context value.
    ///
    /// Sets the token source, runs the expression grammar, and returns the context on success.
    ///
    /// # Errors
    ///
    /// Returns an error if the input does not contain a valid CEL expression.
    pub fn parse_tokens_ctx(&mut self, tokens: TokenStreamIter) -> Result<C> {
        self.set_tokens(tokens);
        if !self.is_expression()? {
            return Err(self.error_at("expression expected"));
        }
        Ok(std::mem::replace(&mut self.context, C::new_context()))
    }

    /// Parses a string into a context value.
    ///
    /// Tokenizes the string then parses; equivalent to
    /// `parse_tokens_ctx(TokenStream::from_str(s)?.into_iter())`.
    ///
    /// # Errors
    ///
    /// Returns an error on lex failure or if the input does not contain a valid CEL expression.
    pub fn parse_str_ctx(&mut self, s: &str) -> Result<C> {
        let input = TokenStream::from_str(s).map_err(|e| ParseError::from_lex_error(s, e))?;
        self.parse_tokens_ctx(input.into_iter())
    }

    /// Returns a mutable reference to the operation lookup.
    ///
    /// This allows customization of the operations available during parsing,
    /// such as adding new scopes for custom operations or identifiers.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::op_table::OpLookup;
    /// use cel_parser::CELParser;
    /// use cel_runtime::DynSegment;
    /// use proc_macro2::TokenStream;
    /// use std::any::TypeId;
    /// use std::str::FromStr;
    ///
    /// let input = TokenStream::from_str("10 + 20").unwrap();
    /// let mut lookup = OpLookup::new();
    /// lookup.push_scope(|name, segment, num_operands, _span| {
    ///     let matches = {
    ///         let top = segment.peek_stack_infos(num_operands);
    ///         name == "+" && top.len() == 2 && top[0].type_id == TypeId::of::<i32>()
    ///     };
    ///     if matches {
    ///         segment.op2(|a: i32, b: i32| a + b + 1)?; // Custom addition
    ///         Ok(true)
    ///     } else {
    ///         Ok(false)
    ///     }
    /// });
    /// let mut parser = CELParser::new(lookup);
    /// parser.set_tokens(input.into_iter());
    /// ```
    pub fn op_lookup_mut(&mut self) -> &mut OpLookup {
        &mut self.op_lookup
    }

    /// Advances past the current token, recording its span in `last_span`.
    ///
    /// # Panics
    ///
    /// Panics if no token stream has been set or if there is no current token.
    fn advance(&mut self) {
        use lex_lexer::HasSpan;
        self.last_span = self
            .tokens
            .as_mut()
            .expect("tokens set")
            .next()
            .expect("token required to advance")
            .span();
    }

    /// Returns the span of the next token without consuming it, or `None` if exhausted.
    fn peek_span(&mut self) -> Option<Span> {
        self.peek_token().map(|token| {
            use lex_lexer::HasSpan;
            token.span()
        })
    }

    /// Peeks at the current token without consuming it.
    ///
    /// Returns `None` if there are no more tokens.
    fn peek_token(&mut self) -> Option<&Token> {
        self.tokens.as_mut().expect("tokens set").peek()
    }

    /// Builds a [`ParseError`] at the current token's span (or call_site if no token).
    fn error_at(&mut self, message: &str) -> ParseError {
        let span = match self.peek_token() {
            Some(token) => {
                use lex_lexer::HasSpan;
                token.span()
            }
            None => Span::call_site(),
        };
        ParseError::new(message, span)
    }

    /// Consumes and returns `true` if the next token is punctuation matching `target`.
    fn is_punctuation(&mut self, target: &str) -> bool {
        match self.peek_token() {
            Some(Token::Punct { op, .. }) if op == target => {
                self.advance();
                true
            }
            _ => false,
        }
    }

    /// Consumes and returns `true` if the next token is an identifier matching `keyword`.
    fn is_keyword(&mut self, keyword: &str) -> bool {
        match self.peek_token() {
            Some(Token::Identifier(ident)) if ident == keyword => {
                self.advance();
                true
            }
            _ => false,
        }
    }

    /// Consumes and returns the next token's text if it is an identifier.
    ///
    /// # Errors
    ///
    /// Returns an error with `message` if the next token is not an identifier.
    fn expect_identifier(&mut self, message: &str) -> Result<String> {
        match self.peek_token() {
            Some(Token::Identifier(ident)) => {
                let name = ident.to_string();
                self.advance();
                Ok(name)
            }
            _ => Err(self.error_at(message)),
        }
    }

    /// Consumes and returns `true` if the next token is an opening parenthesis `(`.
    fn is_open_paren(&mut self) -> bool {
        if matches!(
            self.peek_token(),
            Some(Token::OpenDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            })
        ) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Consumes and returns `true` if the next token is a closing parenthesis `)`.
    fn is_close_paren(&mut self) -> bool {
        if matches!(
            self.peek_token(),
            Some(Token::CloseDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            })
        ) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// `expression = or_expression <EOF>.`
    pub fn is_expression(&mut self) -> Result<bool> {
        if !self.is_or_expression()? {
            return Ok(false);
        }
        if self.peek_token().is_some() {
            return Err(self.error_at("unexpected token"));
        }
        Ok(true)
    }

    /// `or_expression = and_expression { "||" and_expression }.`
    ///
    /// # Errors
    ///
    /// Returns an error if the RHS is missing after `||`, if the RHS does not
    /// produce a `bool`, or if any sub-expression returns an error.
    fn is_or_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_and_expression()? {
            while self.is_punctuation("||") {
                let mut rhs_fragment = self.context.new_fragment();
                std::mem::swap(&mut self.context, &mut rhs_fragment);
                if !self.is_and_expression()? {
                    return Err(self.error_at("expected and_expression"));
                }
                std::mem::swap(&mut self.context, &mut rhs_fragment);
                self.context.apply_logical(
                    "||",
                    rhs_fragment,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `and_expression = comparison_expression { "&&" comparison_expression }.`
    ///
    /// # Errors
    ///
    /// Returns an error if the RHS is missing after `&&`, if the RHS does not
    /// produce a `bool`, or if any sub-expression returns an error.
    fn is_and_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_comparison_expression()? {
            while self.is_punctuation("&&") {
                let mut rhs_fragment = self.context.new_fragment();
                std::mem::swap(&mut self.context, &mut rhs_fragment);
                if !self.is_comparison_expression()? {
                    return Err(self.error_at("expected comparison_expression"));
                }
                std::mem::swap(&mut self.context, &mut rhs_fragment);
                self.context.apply_logical(
                    "&&",
                    rhs_fragment,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `comparison_expression = bitwise_or_expression [ ("==" | "!=" | "<" | ">" | "<=" | ">=") bitwise_or_expression ].`
    fn is_comparison_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_bitwise_or_expression()? {
            // Longer operators first: must check "==" before "=", "<=" before "<", etc.
            let op_name = if self.is_punctuation("==") {
                Some("==")
            } else if self.is_punctuation("!=") {
                Some("!=")
            } else if self.is_punctuation("<=") {
                Some("<=")
            } else if self.is_punctuation(">=") {
                Some(">=")
            } else if self.is_punctuation("<") {
                Some("<")
            } else if self.is_punctuation(">") {
                Some(">")
            } else {
                None
            };

            if let Some(op_name) = op_name {
                if !self.is_bitwise_or_expression()? {
                    return Err(self.error_at("expected bitwise_or_expression"));
                }
                self.context.apply_op(
                    &self.op_lookup,
                    op_name,
                    2,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `bitwise_or_expression = bitwise_xor_expression { "|" bitwise_xor_expression }.`
    fn is_bitwise_or_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_bitwise_xor_expression()? {
            while self.is_punctuation("|") {
                if !self.is_bitwise_xor_expression()? {
                    return Err(self.error_at("expected bitwise_xor_expression"));
                }
                self.context.apply_op(
                    &self.op_lookup,
                    "|",
                    2,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `bitwise_xor_expression = bitwise_and_expression { "^" bitwise_and_expression }.`
    fn is_bitwise_xor_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_bitwise_and_expression()? {
            while self.is_punctuation("^") {
                if !self.is_bitwise_and_expression()? {
                    return Err(self.error_at("expected bitwise_and_expression"));
                }
                self.context.apply_op(
                    &self.op_lookup,
                    "^",
                    2,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `bitwise_and_expression = bitwise_shift_expression { "&" bitwise_shift_expression }.`
    fn is_bitwise_and_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_bitwise_shift_expression()? {
            while self.is_punctuation("&") {
                if !self.is_bitwise_shift_expression()? {
                    return Err(self.error_at("expected bitwise_shift_expression"));
                }
                self.context.apply_op(
                    &self.op_lookup,
                    "&",
                    2,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `bitwise_shift_expression = additive_expression { ("<<" | ">>") additive_expression }.`
    fn is_bitwise_shift_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_additive_expression()? {
            loop {
                let op_name = if self.is_punctuation("<<") {
                    Some("<<")
                } else if self.is_punctuation(">>") {
                    Some(">>")
                } else {
                    None
                };

                if let Some(op_name) = op_name {
                    if !self.is_additive_expression()? {
                        return Err(self.error_at("expected additive_expression"));
                    }
                    self.context.apply_op(
                        &self.op_lookup,
                        op_name,
                        2,
                        start_span.expect("production has token at start"),
                        self.last_span,
                    )?;
                } else {
                    break;
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `additive_expression = multiplicative_expression { ("+" | "-") multiplicative_expression }.`
    fn is_additive_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_multiplicative_expression()? {
            loop {
                let op_name = if self.is_punctuation("+") {
                    Some("+")
                } else if self.is_punctuation("-") {
                    Some("-")
                } else {
                    None
                };

                if let Some(op_name) = op_name {
                    if !self.is_multiplicative_expression()? {
                        return Err(self.error_at("expected multiplicative_expression"));
                    }
                    self.context.apply_op(
                        &self.op_lookup,
                        op_name,
                        2,
                        start_span.expect("production has token at start"),
                        self.last_span,
                    )?;
                } else {
                    break;
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `multiplicative_expression = cast_expression { ("*" | "/" | "%") cast_expression }.`
    fn is_multiplicative_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_cast_expression()? {
            loop {
                let op_name = if self.is_punctuation("*") {
                    Some("*")
                } else if self.is_punctuation("/") {
                    Some("/")
                } else if self.is_punctuation("%") {
                    Some("%")
                } else {
                    None
                };

                if let Some(op_name) = op_name {
                    if !self.is_cast_expression()? {
                        return Err(self.error_at("expected cast_expression"));
                    }
                    self.context.apply_op(
                        &self.op_lookup,
                        op_name,
                        2,
                        start_span.expect("production has token at start"),
                        self.last_span,
                    )?;
                } else {
                    break;
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `cast_expression = unary_expression { "as" identifier }.`
    ///
    /// Binds tighter than `* / %` but looser than unary prefix and postfix, matching Rust: `-x as
    /// f64` is `(-x) as f64`, and `x as i32 as f64` applies left-to-right (`(x as i32) as f64`).
    ///
    /// # Errors
    ///
    /// Returns an error if `"as"` isn't followed by an identifier, or if any sub-expression
    /// returns an error.
    fn is_cast_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_unary_expression()? {
            while self.is_keyword("as") {
                let type_name = match self.peek_token() {
                    Some(Token::Identifier(ident)) => ident.to_string(),
                    _ => return Err(self.error_at("expected type name after `as`")),
                };
                self.advance();
                self.context.apply_cast(
                    &self.op_lookup,
                    &type_name,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `unary_expression = (("-" | "!") unary_expression) | primary_expression.`
    fn is_unary_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        let op_name = if self.is_punctuation("-") {
            Some("-")
        } else if self.is_punctuation("!") {
            Some("!")
        } else {
            None
        };

        if let Some(op_name) = op_name {
            if !self.is_unary_expression()? {
                return Err(self.error_at("expected unary_expression"));
            }
            self.context.apply_op(
                &self.op_lookup,
                op_name,
                1,
                start_span.expect("production has token at start"),
                self.last_span,
            )?;
            Ok(true)
        } else {
            self.is_postfix_expression()
        }
    }

    /// `postfix_expression = primary_expression { "(" [ parameter_list ] ")" | "." unsuffixed_integer }.`
    ///
    /// The repetition allows chained indices (`t.0.1`): each `"." unsuffixed_integer`
    /// is applied in turn to whatever value the previous step left on top of the
    /// stack. Source text like `.0.1` tokenizes as a single `.` followed by one
    /// float literal `0.1` (Rust's own lexer maximally munches the digits after
    /// the second `.`), so that case is detected and split back into its two
    /// integer indices — see the `Token::Literal(CelLiteral::Float(..))` arm below.
    fn is_postfix_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if !self.is_primary_expression()? {
            return Ok(false);
        }
        loop {
            if matches!(
                self.peek_token(),
                Some(Token::OpenDelim {
                    delimiter: Delimiter::Parenthesis,
                    ..
                })
            ) {
                self.advance(); // consume "("
                let arg_count = if matches!(
                    self.peek_token(),
                    Some(Token::CloseDelim {
                        delimiter: Delimiter::Parenthesis,
                        ..
                    })
                ) {
                    0
                } else {
                    self.parameter_list()?
                };
                match self.peek_token() {
                    Some(Token::CloseDelim {
                        delimiter: Delimiter::Parenthesis,
                        ..
                    }) => {
                        self.advance(); // consume ")"
                    }
                    _ => return Err(self.error_at("expected closing parenthesis")),
                }
                // Stack order is [callee, arg1, arg2, ...]; lookup peeks top (arg_count + 1) entries.
                self.context.apply_op(
                    &self.op_lookup,
                    "()",
                    arg_count + 1,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            } else if self.is_punctuation(".") {
                match self.peek_token() {
                    Some(Token::Literal(CelLiteral::Int(integer))) => {
                        let integer = integer.clone();
                        if !integer.suffix().is_empty() {
                            return Err(self.error_at("tuple index must be an unsuffixed integer"));
                        }
                        self.advance();
                        let index = integer.base10_parse::<usize>().map_err(|e| {
                            self.error_at(&format!("invalid tuple index `{integer}`: {e}"))
                        })?;
                        self.apply_tuple_index(
                            index,
                            start_span.expect("production has token at start"),
                        )?;
                    }
                    Some(Token::Literal(CelLiteral::Float(float))) => {
                        let float = float.clone();
                        if !float.suffix().is_empty() {
                            return Err(self.error_at("tuple index must be an unsuffixed integer"));
                        }
                        // base10_digits() returns the decimal digits with underscores
                        // stripped and the suffix removed, e.g. "0.1" or "10.25" for
                        // ordinary decimal floats — splitting on '.' recovers the two
                        // chained integer indices. Scientific-notation floats (e.g.
                        // `1e2`) normalize to digits with no '.' at all; reject those
                        // as a parse error (checked before advancing, so the error
                        // span still points at the float token) rather than assuming
                        // a '.' is always present.
                        let digits = float.base10_digits();
                        let Some((first, second)) = digits.split_once('.') else {
                            return Err(self.error_at(
                                "tuple index chain must use decimal notation (e.g. `.0.1`)",
                            ));
                        };
                        self.advance();
                        let first_index = first.parse::<usize>().map_err(|e| {
                            self.error_at(&format!("invalid tuple index `{first}`: {e}"))
                        })?;
                        let second_index = second.parse::<usize>().map_err(|e| {
                            self.error_at(&format!("invalid tuple index `{second}`: {e}"))
                        })?;
                        let idx_start = start_span.expect("production has token at start");
                        self.apply_tuple_index(first_index, idx_start)?;
                        self.apply_tuple_index(second_index, idx_start)?;
                    }
                    _ => return Err(self.error_at("expected integer after '.'")),
                }
            } else {
                break;
            }
        }
        Ok(true)
    }

    /// Applies a single `.N` tuple-index operation to the value currently on
    /// top of the stack, replacing it with element `index`. `start` is the span
    /// of the base expression the index chain is rooted at.
    ///
    /// # Errors
    /// Returns an error if the top of stack isn't a tuple, or if `index` is
    /// out of range for its arity.
    fn apply_tuple_index(&mut self, index: usize, start: Span) -> Result<()> {
        let arity = self
            .context
            .peek_tuple_arity()
            .ok_or_else(|| self.error_at("'.N' requires a tuple"))?;
        if index >= arity {
            return Err(self.error_at(&format!(
                "tuple index `{index}` out of range for tuple of arity {arity}"
            )));
        }
        self.context.tuple_index(index, start, self.last_span);
        Ok(())
    }

    /// `parameter_list = or_expression { "," or_expression }.`
    ///
    /// Always parses at least one `or_expression` — callers that need to allow zero
    /// arguments (`postfix_expression`'s `"(" [ parameter_list ] ")"`) check for that
    /// possibility themselves before calling, rather than `parameter_list` swallowing it.
    ///
    /// Returns the argument count.
    ///
    /// # Errors
    /// Returns an error if the first token can't start an `or_expression`, or if a comma
    /// isn't followed by one.
    fn parameter_list(&mut self) -> Result<usize> {
        if !self.is_or_expression()? {
            return Err(self.error_at("expected expression"));
        }
        let mut count = 1;
        while self.is_punctuation(",") {
            if !self.is_or_expression()? {
                return Err(self.error_at("expected expression after comma"));
            }
            count += 1;
        }
        Ok(count)
    }

    /// `primary_expression = literal | identifier | tuple_or_group | if_expression |
    /// closure_expression.`
    ///
    /// Dispatches to [`is_if_expression`](Self::is_if_expression) when the `if` keyword is seen,
    /// to [`is_tuple_or_group`](Self::is_tuple_or_group) when `(` is seen, and to
    /// [`is_closure_expression`](Self::is_closure_expression) when `|` or `||` is seen.
    ///
    /// A zero-parameter closure's opening and closing pipes (`||`) have nothing between them to
    /// keep them apart, so the lexer combines them into one two-character token exactly like it
    /// does for `&&`/`<=`/etc. elsewhere in this grammar — checked first, before the
    /// one-character `|` case, so `is_punctuation("|")` doesn't (and can't) partially match it.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - A literal value cannot be parsed (e.g., integer out of range).
    /// - An identifier is not found in the op lookup table.
    /// - A tuple-or-group expression fails to parse.
    /// - An `if` expression fails to parse.
    /// - A closure expression fails to parse.
    fn is_primary_expression(&mut self) -> Result<bool> {
        if self.is_punctuation("||") {
            return self.is_closure_expression(true);
        }
        if self.is_punctuation("|") {
            return self.is_closure_expression(false);
        }
        match self.peek_token() {
            Some(Token::Literal(lit)) => {
                let lit_clone = lit.clone();
                self.advance();
                push_literal_token(&mut self.context, lit_clone)?;
                Ok(true)
            }
            Some(Token::Identifier(ident)) => {
                let ident_name = ident.to_string();
                let ident_span = ident.span();
                self.advance();

                if ident_name == "if" {
                    return self.is_if_expression(ident_span);
                }

                self.context
                    .apply_op(&self.op_lookup, &ident_name, 0, ident_span, ident_span)?;

                Ok(true)
            }
            Some(Token::OpenDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            }) => self.is_tuple_or_group(),
            _ => Ok(false),
        }
    }

    /// `tuple_or_group = "(" [ or_expression ["," [ or_expression { "," or_expression } ]] ] ")".`
    ///
    /// `()` parses as unit, `(expr)` as grouping, `(expr,)` as a 1-tuple, and
    /// `(expr, expr, ...)` as an n-tuple.
    ///
    /// - Precondition: The next token is `Token::OpenDelim` with `Delimiter::Parenthesis`.
    ///
    /// # Errors
    ///
    /// Returns an error if the parenthesized expression or tuple literal is malformed, has a
    /// missing or misplaced comma, or is missing its closing `)`.
    fn is_tuple_or_group(&mut self) -> Result<bool> {
        let open_span = self
            .peek_span()
            .expect("tuple_or_group requires an opening '(' token");
        self.advance();
        // Unit expression: ()
        if matches!(
            self.peek_token(),
            Some(Token::CloseDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            })
        ) {
            self.advance();
            self.context.push_literal((), self.last_span);
            return Ok(true);
        }
        let ambient_start = self.context.current_stack_offset();
        if !self.is_or_expression()? {
            return Err(self.error_at("expected expression"));
        }
        if matches!(
            self.peek_token(),
            Some(Token::CloseDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            })
        ) {
            // Grouping: exactly one expression, no comma.
            self.advance();
            return Ok(true);
        }
        if !self.is_punctuation(",") {
            return Err(self.error_at("expected ',' or closing parenthesis"));
        }
        let mut count = 1;
        if matches!(
            self.peek_token(),
            Some(Token::CloseDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            })
        ) {
            // Single element + trailing comma: 1-tuple.
            self.advance();
            self.context
                .make_tuple(count, ambient_start, open_span, self.last_span);
            return Ok(true);
        }
        loop {
            if !self.is_or_expression()? {
                return Err(self.error_at("expected expression after ','"));
            }
            count += 1;
            if matches!(
                self.peek_token(),
                Some(Token::CloseDelim {
                    delimiter: Delimiter::Parenthesis,
                    ..
                })
            ) {
                self.advance();
                break;
            }
            if !self.is_punctuation(",") {
                return Err(self.error_at("expected ',' or closing parenthesis"));
            }
        }
        self.context
            .make_tuple(count, ambient_start, open_span, self.last_span);
        Ok(true)
    }

    /// `closure_expression = ("||" | "|" [ closure_param { "," closure_param } ] "|") expression .`
    /// `closure_param = identifier ":" closure_type_expression .`
    ///
    /// Compiles the body as a fully independent nested context (via
    /// [`parse_nested_context`](Self::parse_nested_context)) whose only visible names are its
    /// own declared parameters plus whatever library/built-in functions are always reachable —
    /// [`OpLookup::isolate_scopes`] hides every other transient scope (including one an enclosing
    /// caller, e.g. adam-lang, pushed around this whole parse) for the duration of the body
    /// parse, so a closure never resolves a free variable from its lexical surroundings.
    ///
    /// Each parameter's declared type is threaded through in two parallel forms: the existing
    /// runtime-facing `ClosureParamType` (which `DynSegmentContext`'s `push_closure` needs a
    /// concrete `TypeId` from) and the unresolved, span-carrying `ClosureParamTypeExpr` (which
    /// `AstContext`'s needs instead) — both built from the same tokens in the same
    /// [`parse_closure_type_expression`](Self::parse_closure_type_expression) call, so nothing
    /// is parsed twice.
    ///
    /// - Precondition: the opening `|` (`params_already_closed == false`) or the combined `||`
    ///   token naming an empty parameter list (`params_already_closed == true`) has already been
    ///   consumed by [`is_primary_expression`](Self::is_primary_expression); `self.last_span` is
    ///   its span.
    ///
    /// # Errors
    ///
    /// Returns an error if a parameter name, its `:`, its type, or the closing `|` is malformed
    /// or missing; if a parameter's type names an unrecognized type; if the body expression is
    /// missing or malformed; or if this `ParserContext` implementation's `push_closure` rejects
    /// the closure (e.g. `DynSegmentContext` when the body doesn't produce exactly one value).
    ///
    /// - Postcondition: Returns `Ok(true)` on success; `Ok(false)` is never returned.
    fn is_closure_expression(&mut self, params_already_closed: bool) -> Result<bool> {
        let start_span = self.last_span;
        let mut params: Vec<(String, Span, ClosureParamType, ClosureParamTypeExpr)> = Vec::new();
        if !params_already_closed {
            loop {
                let name = self.expect_identifier("expected closure parameter name")?;
                let name_span = self.last_span;
                if !self.is_punctuation(":") {
                    return Err(self.error_at("expected ':' after closure parameter name"));
                }
                let (ty, ty_ast) = self.parse_closure_type_expression()?;
                params.push((name, name_span, ty, ty_ast));
                if self.is_punctuation(",") {
                    continue;
                }
                break;
            }
            if !self.is_punctuation("|") {
                return Err(self.error_at("expected ',' or closing '|'"));
            }
        }

        let param_types: Vec<TypeId> = params.iter().map(|(_, _, ty, _)| ty.type_id()).collect();
        let ast_params: Vec<ClosureParam> = params
            .iter()
            .map(|(name, name_span, _, ty_ast)| ClosureParam {
                name: name.clone(),
                name_span: ExprSpan {
                    start: *name_span,
                    end: *name_span,
                },
                type_expr: ty_ast.clone(),
            })
            .collect();
        let isolated = self.op_lookup.isolate_scopes();
        let param_table: HashMap<String, (usize, ClosureParamType)> = params
            .into_iter()
            .enumerate()
            .map(|(idx, (name, _, ty, _))| (name, (idx, ty)))
            .collect();
        self.op_lookup
            .push_scope(move |name, segment, arity, _span| {
                if arity != 0 {
                    return Ok(false);
                }
                let Some((idx, ty)) = param_table.get(name) else {
                    return Ok(false);
                };
                match ty {
                    ClosureParamType::Scalar(scalar) => (scalar.push_arg)(segment, *idx),
                    ClosureParamType::Tuple(elements) => segment
                        .push_arg_as_dynamic_sequence_tuple(*idx, elements_to_associated(elements)),
                }
                Ok(true)
            });

        let body_result = self.parse_nested_context(|p| p.is_or_expression());
        self.op_lookup.pop_scope();
        self.op_lookup.restore_scopes(isolated);
        let body = body_result?;

        self.context
            .push_closure(param_types, ast_params, body, start_span)?;
        Ok(true)
    }

    /// `closure_type_expression = identifier | "(" [ closure_type_expression { "," closure_type_expression } ] ")" .`
    ///
    /// Builds both the runtime-facing `ClosureParamType` and the unresolved, span-carrying
    /// `ClosureParamTypeExpr` from the same tokens in one pass (see
    /// [`is_closure_expression`](Self::is_closure_expression)'s doc comment for why both are
    /// needed). Note this production has no 1-element-tuple form (unlike
    /// `adam_lang::ast::TypeExpr`): the element loop here continues on a trailing `,` rather
    /// than treating one as a terminator, so `(i32,)` fails to parse as a closure parameter
    /// type — an existing grammar quirk, unchanged by this addition.
    ///
    /// # Errors
    ///
    /// Returns an error if a bare identifier doesn't name a recognized built-in scalar type, or
    /// if the parenthesized element list is malformed or missing its closing `)`.
    fn parse_closure_type_expression(
        &mut self,
    ) -> Result<(ClosureParamType, ClosureParamTypeExpr)> {
        if let Some(Token::Identifier(ident)) = self.peek_token() {
            let name = ident.to_string();
            self.advance();
            let name_span = self.last_span;
            let scalar = crate::op_table::builtin_scalar_type(&name)
                .ok_or_else(|| self.error_at(&format!("unknown type `{name}`")))?;
            return Ok((
                ClosureParamType::Scalar(scalar),
                ClosureParamTypeExpr::Named(
                    name,
                    ExprSpan {
                        start: name_span,
                        end: name_span,
                    },
                ),
            ));
        }
        if !self.is_open_paren() {
            return Err(self.error_at("expected a type name or '('"));
        }
        let open_span = self.last_span;
        let mut elements = Vec::new();
        let mut element_asts = Vec::new();
        if !self.is_close_paren() {
            loop {
                let (ty, ty_ast) = self.parse_closure_type_expression()?;
                elements.push(ty);
                element_asts.push(ty_ast);
                if self.is_punctuation(",") {
                    continue;
                }
                break;
            }
            if !self.is_close_paren() {
                return Err(self.error_at("expected ',' or closing ')'"));
            }
        }
        let close_span = self.last_span;
        Ok((
            ClosureParamType::Tuple(elements),
            ClosureParamTypeExpr::Tuple(
                element_asts,
                ExprSpan {
                    start: open_span,
                    end: close_span,
                },
            ),
        ))
    }

    /// `if_expression = "if" or_expression "{" or_expression "}" [ "else" ( "{" or_expression "}" | if_expression ) ].`
    ///
    /// - Precondition: The `if` keyword has already been consumed by the caller; `if_span` is
    ///   its span.
    ///
    /// # Errors
    ///
    /// Returns an error if the condition is missing, if a `{` or `}` delimiter is missing,
    /// if the then-branch or else-branch expression is missing, or if the then and else
    /// branch types do not match (as detected by `join2`).
    ///
    /// - Postcondition: Returns `Ok(true)` on success; `Ok(false)` is never returned.
    fn is_if_expression(&mut self, if_span: Span) -> Result<bool> {
        if !self.is_or_expression()? {
            return Err(self.error_at("expected condition after `if`"));
        }
        match self.peek_token() {
            Some(Token::OpenDelim {
                delimiter: Delimiter::Brace,
                ..
            }) => {
                self.advance();
            }
            _ => return Err(self.error_at("expected `{` after if condition")),
        }
        let mut then_fragment = self.context.new_fragment();
        std::mem::swap(&mut self.context, &mut then_fragment);
        if !self.is_or_expression()? {
            return Err(self.error_at("expected expression in then-branch"));
        }
        std::mem::swap(&mut self.context, &mut then_fragment);
        match self.peek_token() {
            Some(Token::CloseDelim {
                delimiter: Delimiter::Brace,
                ..
            }) => {
                self.advance();
            }
            _ => return Err(self.error_at("expected `}` after then-branch")),
        }
        let else_fragment = if self.is_keyword("else") {
            if self.is_keyword("if") {
                // else if: recursively parse another if_expression
                let elif_span = self.last_span;
                let mut fragment = self.context.new_fragment();
                std::mem::swap(&mut self.context, &mut fragment);
                self.is_if_expression(elif_span)?;
                std::mem::swap(&mut self.context, &mut fragment);
                Some(fragment)
            } else {
                // else { expr }
                match self.peek_token() {
                    Some(Token::OpenDelim {
                        delimiter: Delimiter::Brace,
                        ..
                    }) => {
                        self.advance();
                    }
                    _ => return Err(self.error_at("expected `{` or `if` after `else`")),
                }
                let mut fragment = self.context.new_fragment();
                std::mem::swap(&mut self.context, &mut fragment);
                if !self.is_or_expression()? {
                    return Err(self.error_at("expected expression in else-branch"));
                }
                std::mem::swap(&mut self.context, &mut fragment);
                match self.peek_token() {
                    Some(Token::CloseDelim {
                        delimiter: Delimiter::Brace,
                        ..
                    }) => {
                        self.advance();
                    }
                    _ => return Err(self.error_at("expected `}` after else-branch")),
                }
                Some(fragment)
            }
        } else {
            // No `else`/`else if` in the source — each ParserContext::join2 impl decides what
            // this means (DynSegmentContext synthesizes an implicit `()` fragment;
            // AstContext records `None` directly on Expr::If).
            None
        };
        self.context
            .join2(then_fragment, else_fragment, if_span, self.last_span)
            .map_err(|e| ParseError::new(e.to_string(), self.last_span))?;
        Ok(true)
    }
}

impl Parser<DynSegmentContext> {
    /// Parses one `or_expression` from the current token stream and returns the segment.
    ///
    /// Unlike [`parse_str`](Self::parse_str), this method does not require end-of-stream,
    /// allowing adam-lang to parse an expression embedded within a larger token stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the input does not contain a valid `or_expression`.
    ///
    /// - Complexity: O(n) in the number of tokens in the expression.
    pub fn parse_or_expression(&mut self) -> Result<DynSegment> {
        self.parse_or_expression_ctx()
            .map(DynSegmentContext::into_inner)
    }

    /// Parses a token stream into a [`DynSegment`].
    ///
    /// Sets the token source, runs the expression grammar, and returns the segment on success.
    ///
    /// # Errors
    ///
    /// Returns an error if the input does not contain a valid CEL expression.
    ///
    /// - Complexity: O(n) in the number of tokens.
    pub fn parse_tokens(&mut self, tokens: TokenStreamIter) -> Result<DynSegment> {
        self.parse_tokens_ctx(tokens)
            .map(DynSegmentContext::into_inner)
    }

    /// Parses a string into a [`DynSegment`].
    ///
    /// Tokenizes the string then parses; equivalent to
    /// `parse_tokens(TokenStream::from_str(s)?.into_iter())`.
    ///
    /// # Errors
    ///
    /// Returns an error on lex failure or if the input does not contain a valid CEL expression.
    ///
    /// - Complexity: O(n) in the length of `s`.
    pub fn parse_str(&mut self, s: &str) -> Result<DynSegment> {
        self.parse_str_ctx(s).map(DynSegmentContext::into_inner)
    }
}

impl Parser<AstContext> {
    /// Parses one `or_expression` from the current token stream and returns the built [`Expr`].
    ///
    /// Unlike [`parse_str_ast`](Self::parse_str_ast), this method does not require
    /// end-of-stream, allowing adam-lang to parse an expression embedded within a larger token
    /// stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the input does not contain a valid `or_expression`.
    ///
    /// - Complexity: O(n) in the number of tokens in the expression.
    pub fn parse_or_expression_ast(&mut self) -> Result<Expr> {
        self.parse_or_expression_ctx().map(AstContext::into_expr)
    }

    /// Parses a token stream into an [`Expr`] tree.
    ///
    /// Sets the token source, runs the expression grammar, and returns the tree on success.
    ///
    /// # Errors
    ///
    /// Returns an error if the input does not contain a valid CEL expression.
    ///
    /// - Complexity: O(n) in the number of tokens.
    pub fn parse_tokens_ast(&mut self, tokens: TokenStreamIter) -> Result<Expr> {
        self.parse_tokens_ctx(tokens).map(AstContext::into_expr)
    }

    /// Parses a string into an [`Expr`] tree.
    ///
    /// Tokenizes the string then parses; equivalent to
    /// `parse_tokens_ast(TokenStream::from_str(s)?.into_iter())`.
    ///
    /// # Errors
    ///
    /// Returns an error on lex failure or if the input does not contain a valid CEL expression.
    ///
    /// - Complexity: O(n) in the length of `s`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::{AstContext, Expr, OpLookup, Parser};
    ///
    /// let mut parser = Parser::<AstContext>::new(OpLookup::new());
    /// let expr = parser.parse_str_ast("1 + 2").unwrap();
    /// assert!(matches!(expr, Expr::Op { .. }));
    /// ```
    pub fn parse_str_ast(&mut self, s: &str) -> Result<Expr> {
        self.parse_str_ctx(s).map(AstContext::into_expr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use annotate_snippets::Renderer;

    #[test]
    fn simple_expression() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("10");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().call0::<i32>().unwrap(), 10);
    }

    #[test]
    fn parse_nested_context_compiles_an_independent_segment_without_disturbing_the_outer_one()
    -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        parser.set_tokens(quote::quote! { 1 + 2 }.into_iter());
        // Start the outer expression: push a literal directly onto the (not-yet-swapped) context.
        parser
            .context
            .push_literal(100i32, proc_macro2::Span::call_site());

        let nested = parser
            .parse_nested_context(|p| p.is_or_expression())
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        // The outer context still only has the one literal pushed before the nested parse.
        assert_eq!(parser.context.into_inner().call0::<i32>()?, 100);
        // The nested context has its own, independently-evaluated result.
        assert_eq!(nested.into_inner().call0::<i32>()?, 3);
        Ok(())
    }

    #[test]
    fn integer_literal_i32_suffix() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("10i32");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().call0::<i32>().unwrap(), 10);
    }

    #[test]
    fn invalid_integer_suffix() {
        let mut parser = CELParser::new(OpLookup::new());
        let err = match parser.parse_str("10xyz") {
            Err(e) => e,
            Ok(_) => panic!("expected parse error for invalid integer suffix"),
        };
        assert!(err.message().contains("invalid integer literal suffix"));
        assert!(err.message().contains("xyz"));
    }

    #[test]
    fn float_literal() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("42.14");
        assert!(result.is_ok());
        let value = result.unwrap().call0::<f64>().unwrap();
        assert!((value - 42.14).abs() < 1e-10);
    }

    #[test]
    fn float_literal_f64_suffix() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("42.14f64");
        assert!(result.is_ok());
        let value = result.unwrap().call0::<f64>().unwrap();
        assert!((value - 42.14).abs() < 1e-10);
    }

    #[test]
    fn float_literal_f32_suffix() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("42.14f32");
        assert!(result.is_ok());
        let value = result.unwrap().call0::<f32>().unwrap();
        assert!((value - 42.14f32).abs() < 1e-6);
    }

    #[test]
    fn invalid_float_suffix() {
        let mut parser = CELParser::new(OpLookup::new());
        let err = match parser.parse_str("3.14xyz") {
            Err(e) => e,
            Ok(_) => panic!("expected parse error for invalid float suffix"),
        };
        assert!(err.message().contains("invalid float literal suffix"));
        assert!(err.message().contains("xyz"));
    }

    #[test]
    fn boolean_literal() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("true");
        assert!(result.is_ok());
        assert!(result.unwrap().call0::<bool>().unwrap());
    }

    #[test]
    fn string_literal() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str(r#""hello""#);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().call0::<String>().unwrap(), "hello");
    }

    #[test]
    fn string_concatenation() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str(r#""a" + "b""#);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().call0::<String>().unwrap(), "ab");
    }

    #[test]
    fn incomplete_expression() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("10 + 25 25");
        let err = match result {
            Ok(_) => panic!("expected parse error"),
            Err(e) => e,
        };
        assert_eq!(err.message(), "unexpected token");
    }

    #[test]
    fn arithmetic_expression() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("10 + 20 * 30");
        assert!(result.is_ok());
    }

    #[test]
    fn parenthesized_expression() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("(10 + 20) * 30");
        assert!(result.is_ok());
    }

    #[test]
    fn unit_still_parses_as_unit() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser.parse_str("()").unwrap();
        seg.call0::<()>().unwrap();
    }

    #[test]
    fn single_paren_expression_is_grouping_not_tuple() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser.parse_str("(1i32 + 2i32)").unwrap();
        assert_eq!(seg.call0::<i32>().unwrap(), 3);
    }

    #[test]
    fn one_tuple_requires_trailing_comma() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser.parse_str("(1i32,)").unwrap();
        assert_eq!(seg.peek_tuple_arity(), Some(1));
        seg.tuple_index(0);
        assert_eq!(seg.call0::<i32>().unwrap(), 1);
    }

    #[test]
    fn two_element_tuple_no_trailing_comma() {
        let mut parser = CELParser::new(OpLookup::new());
        let seg: DynSegment = parser.parse_str(r#"("Hello", 42i32)"#).unwrap();
        assert_eq!(seg.peek_tuple_arity(), Some(2));
    }

    #[test]
    fn trailing_comma_rejected_for_arity_two() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("(1i32, 2i32,)");
        assert!(result.is_err(), "trailing comma is only valid for 1-tuples");
    }

    #[test]
    fn missing_comma_between_elements_is_an_error() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("(1i32 2i32)");
        assert!(result.is_err());
    }

    #[test]
    fn index_first_element_of_tuple() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser.parse_str("(10i32, 20i32).0").unwrap();
        assert_eq!(seg.call0::<i32>().unwrap(), 10);
    }

    #[test]
    fn tuple_element_can_be_arithmetic_expression() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser.parse_str("(1i32 + 2i32, 3i32).0").unwrap();
        assert_eq!(seg.call0::<i32>().unwrap(), 3);
    }

    #[test]
    fn tuple_second_element_can_be_arithmetic_expression() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser.parse_str("(1i32 + 2i32, 3i32).1").unwrap();
        assert_eq!(seg.call0::<i32>().unwrap(), 3);
    }

    #[test]
    fn tuple_ambient_start_correct_after_sibling_expression() {
        // Regression test: a fully-evaluated sibling subexpression earlier in
        // the same additive chain must not shift where the following tuple
        // literal thinks its elements land on the real stack.
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser
            .parse_str("(1i32 + 2i32) + 3i32 + (4i32, 5i32).0")
            .unwrap();
        assert_eq!(seg.call0::<i32>().unwrap(), 10);
    }

    #[test]
    fn tuple_index_inside_if_then_branch() {
        // Regression test: `join2` pops the condition bool before running
        // the chosen fragment, so a tuple literal inside that fragment must
        // compute its layout as if that pop already happened.
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser
            .parse_str("if true { (10i32, 20i32).1 } else { 0i32 }")
            .unwrap();
        assert_eq!(seg.call0::<i32>().unwrap(), 20);
    }

    #[test]
    fn tuple_index_inside_if_else_branch() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser
            .parse_str("if false { 0i32 } else { (10i32, 20i32).1 }")
            .unwrap();
        assert_eq!(seg.call0::<i32>().unwrap(), 20);
    }

    #[test]
    fn tuple_index_inside_and_rhs() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser
            .parse_str("(1i32, 2i32).1 == 2i32 && (10i32, 20i32).1 == 20i32")
            .unwrap();
        assert!(seg.call0::<bool>().unwrap());
    }

    #[test]
    fn tuple_index_inside_or_rhs() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser
            .parse_str("(1i32, 2i32).1 == 99i32 || (10i32, 20i32).1 == 20i32")
            .unwrap();
        assert!(seg.call0::<bool>().unwrap());
    }

    #[test]
    fn tuple_containing_indexed_nested_tuple_result() {
        // Regression test: extracting an element from a misaligned nested
        // tuple must not leave the tuple's own leading padding as dead space
        // on the stack — otherwise a later tuple literal built from this
        // result computes its element offsets against the wrong ambient
        // start and reads garbage. (7, not 1: the dead gap's own marker
        // byte is hardcoded to value 1, so a value of 1 here would pass
        // even when reading the wrong offset.)
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser.parse_str("(0u8, (7u8, 2u64).0).1").unwrap();
        assert_eq!(seg.call0::<u8>().unwrap(), 7);
    }

    #[test]
    fn index_second_element_of_tuple() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser.parse_str("(10i32, 20i32).1").unwrap();
        assert_eq!(seg.call0::<i32>().unwrap(), 20);
    }

    #[test]
    fn indexing_combined_with_addition() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser.parse_str("5i32 + (0i32, 1i32).1").unwrap();
        assert_eq!(seg.call0::<i32>().unwrap(), 6);
    }

    #[test]
    fn indexing_combined_with_addition_on_the_right() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser.parse_str("(0i32, 1i32).1 + 5i32").unwrap();
        assert_eq!(seg.call0::<i32>().unwrap(), 6);
    }

    #[test]
    fn out_of_range_index_is_a_parse_error() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("(1i32, 2i32).5");
        assert!(result.is_err());
    }

    #[test]
    fn indexing_a_non_tuple_is_a_parse_error() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("1i32.0");
        assert!(result.is_err());
    }

    #[test]
    fn suffixed_index_is_a_parse_error() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("(1i32, 2i32).0i32");
        assert!(result.is_err());
    }

    #[test]
    fn chained_tuple_index_into_nested_tuple() {
        // `.1` selects the inner tuple `(10i32, 20i32)`, then `.0` selects its
        // first element. Source text `.1.0` tokenizes as a single float
        // literal `1.0`, which must be split back into the chained indices
        // `1` then `0` — using a shape/value where applying the indices to
        // the wrong operand or in the wrong order gives a different (wrong)
        // answer than 10 (e.g. swapping order would try `.0` on an i32 and
        // fail to parse; picking element 0 first would return "a" or 10
        // depending on order, not confusably 10 either way, so pick values
        // that make a mix-up obvious).
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser
            .parse_str(r#"("a", (10i32, 20i32)).1.0"#)
            .expect("chained .1.0 index should parse");
        assert_eq!(seg.call0::<i32>().unwrap(), 10);
    }

    #[test]
    fn chained_tuple_index_suffixed_second_part_is_a_parse_error() {
        // The suffix lands on the whole `0.1i32` float token; the existing
        // unsuffixed-integer rule must still reject it.
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("(1i32, 2i32).0.1i32");
        assert!(result.is_err());
    }

    #[test]
    fn chained_tuple_index_scientific_notation_is_a_parse_error_not_a_panic() {
        // `1e2` normalizes to digits with no '.' at all (scientific notation),
        // unlike ordinary decimal floats like `0.1` — must be a graceful parse
        // error, not a panic on the assumption that '.' is always present.
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("(1i32, 2i32).1e2");
        assert!(result.is_err());
    }

    #[test]
    fn complex_expression() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("10 + 20 * (30 - 5) / 2");
        assert!(result.is_ok());
    }

    #[test]
    fn logical_expression() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("true && false || true");
        assert!(result.is_ok());
    }

    #[test]
    fn comparison_expression() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("10 == 20 && 30 > 40");
        assert!(result.is_ok());
    }

    #[test]
    fn bitwise_expression() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("1 | 2 & 3 ^ 4");
        assert!(result.is_ok());
    }

    #[test]
    fn shift_expression() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("8u32 << 2u32 + 16u32 >> 1u32");
        assert!(result.is_ok());
    }

    #[test]
    fn unary_expression() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("-10 + -20");
        assert!(result.is_ok());
    }

    #[test]
    fn double_negation() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("!!true");
        assert!(
            result.is_ok(),
            "Failed to parse !!true: {}",
            result.err().unwrap()
        );
        assert!(result.unwrap().call0::<bool>().unwrap());
    }

    #[test]
    fn double_minus() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("--5");
        assert!(
            result.is_ok(),
            "Failed to parse --5: {}",
            result.err().unwrap()
        );
        assert_eq!(result.unwrap().call0::<i32>().unwrap(), 5);
    }

    #[test]
    fn chained_unary_expression() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("!!false || !!true");
        if let Err(ref e) = result {
            eprintln!("Error: {:?}", e);
            eprintln!("Error message: {}", e.message());
        }
        assert!(result.is_ok(), "Failed to parse: {}", result.err().unwrap());
        assert!(result.unwrap().call0::<bool>().unwrap());
    }

    #[test]
    fn cast_expression_parses_and_executes() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("1024i32 as f64");
        assert_eq!(result.unwrap().call0::<f64>().unwrap(), 1024.0);
    }

    #[test]
    fn cast_expression_chains_left_to_right() {
        // `x as i32 as f64` must apply left-to-right: `(x as i32) as f64`, not
        // `x as (i32 as f64)` (which isn't even expressible - a cast target is a bare
        // identifier, not another cast expression).
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("1.5f64 as i32 as f64");
        assert_eq!(result.unwrap().call0::<f64>().unwrap(), 1.0);
    }

    #[test]
    fn cast_binds_tighter_than_unary_negation() {
        // `-x as u32` must parse as `(-x) as u32` (matching Rust): the negative operand hits the
        // checked int->int cast's own out-of-range check at execution, not a "no operation `-`
        // for `u32`" error at parse time, which is what parsing it as `-(x as u32)` would
        // produce instead - `u32` has no registered unary negation, matching Rust (u32 doesn't
        // implement `Neg`).
        let mut parser = CELParser::new(OpLookup::new());
        let mut seg = parser.parse_str("-1i32 as u32").expect(
            "parses: unary negation is registered for i32, and i32 -> u32 is a valid cast pair",
        );
        let err = seg.call0::<u32>().unwrap_err();
        // The cast's own error is wrapped in a `SpanContext` (see `span_err`), which becomes the
        // outermost context and so is what plain `Display`/`to_string()` shows - the underlying
        // "does not fit" message is further down the `anyhow` chain, hence `{:#}`.
        let full = format!("{err:#}");
        assert!(
            full.contains("does not fit"),
            "expected an out-of-range cast error, got: {full}"
        );
    }

    #[test]
    fn cast_binds_tighter_than_multiplicative_operators() {
        // `*` requires homogeneous operand types, so if `as` binds tighter than `*` (matching
        // Rust), this parses as `2i32 * (3i32 as f64)` - an `i32 * f64` mismatch, which this
        // grammar's eager, statically-typed segment construction rejects immediately at parse
        // time (operator/operand-type matching isn't deferred to execution, unlike a cast's own
        // value-range check). If `as` bound looser instead, this would parse as
        // `(2i32 * 3i32) as f64` and succeed.
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("2i32 * 3i32 as f64");
        assert!(result.is_err(), "expected a type-mismatch parse error");
    }

    #[test]
    fn cast_binds_tighter_than_additive_operators() {
        // Same reasoning as the multiplicative case: `1i32 + 2i32 as f64` must parse as
        // `1i32 + (2i32 as f64)` (an `i32 + f64` mismatch, a parse-time error), not
        // `(1i32 + 2i32) as f64` (which would succeed).
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("1i32 + 2i32 as f64");
        assert!(result.is_err(), "expected a type-mismatch parse error");
    }

    #[test]
    fn invalid_expression() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("+");
        assert!(result.is_err());
    }

    #[test]
    fn error_formatting() {
        let source = "10 + 20 30";
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str(source);

        assert!(result.is_err());

        let err = match &result {
            Ok(_) => panic!("expected parse error"),
            Err(e) => e,
        };
        assert_eq!(err.message(), "unexpected token");

        let formatted = err.format_rustc_style(source, "test.cel", 1u32, &Renderer::plain());
        assert!(formatted.contains("error: unexpected token"));
        assert!(formatted.contains("test.cel:1:"));
        assert!(formatted.contains("1 | 10 + 20 30"));
        assert!(formatted.contains("^"));
    }

    #[test]
    fn error_formatting_with_line_offset() {
        let source = "10 + 20 30";
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str(source);

        assert!(result.is_err());

        let err = match &result {
            Ok(_) => panic!("expected parse error"),
            Err(e) => e,
        };
        let formatted = err.format_rustc_style(source, "large_file.rs", 42u32, &Renderer::plain());
        assert!(formatted.contains("error: unexpected token"));
        assert!(formatted.contains("large_file.rs:42:"));
        assert!(formatted.contains("42 | 10 + 20 30"));
        assert!(formatted.contains("^"));
    }

    #[test]
    fn print_error_formatting() {
        let line = line!() + 1;
        let source = r#"

         10 + 20  30 // Unexpected token

     "#;

        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str(source);

        assert!(result.is_err(), "Expected parsing to fail");

        let err = match &result {
            Ok(_) => panic!("expected parse error"),
            Err(e) => e,
        };
        eprintln!(
            "DEBUG: span.start.line = {}, span.start.column = {}",
            err.span().start().line,
            err.span().start().column
        );

        let formatted_error = err.format_rustc_style(source, file!(), line, &Renderer::plain());
        println!("{}", formatted_error);

        let formatted = formatted_error;

        let expected_line = line + 2;

        assert!(
            formatted.contains("error: unexpected token"),
            "Should contain error message, got: {}",
            formatted
        );
        assert!(
            formatted.contains(&format!("{}:", expected_line)),
            "Should show error on line {}, got: {}",
            expected_line,
            formatted
        );
        assert!(
            formatted.contains("30"),
            "Should show the source line with '30', got: {}",
            formatted
        );
        assert!(
            formatted.contains("^"),
            "Should have carets pointing to error, got: {}",
            formatted
        );
    }

    #[test]
    fn test_addition_execution() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("10 + 20")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let result = segment.call0::<i32>()?;
        assert_eq!(result, 30);
        Ok(())
    }

    #[test]
    fn test_multiplication_execution() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("3 * 7")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let result = segment.call0::<i32>()?;
        assert_eq!(result, 21);
        Ok(())
    }

    #[test]
    fn test_complex_arithmetic_execution() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("10 + 20 * 3")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let result = segment.call0::<i32>()?;
        assert_eq!(result, 70);
        Ok(())
    }

    #[test]
    fn test_parenthesized_arithmetic_execution() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("(10 + 20) * 3")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let result = segment.call0::<i32>()?;
        assert_eq!(result, 90);
        Ok(())
    }

    #[test]
    fn test_comparison_execution() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("10 < 20")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let result = segment.call0::<bool>()?;
        assert!(result);
        Ok(())
    }

    #[test]
    fn test_logical_and_execution() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("true && false")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let result = segment.call0::<bool>()?;
        assert!(!result);
        Ok(())
    }

    #[test]
    fn test_unary_negation_execution() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("-42")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let result = segment.call0::<i32>()?;
        assert_eq!(result, -42);
        Ok(())
    }

    #[test]
    fn test_logical_not_execution() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("!true")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let result = segment.call0::<bool>()?;
        assert!(!result);
        Ok(())
    }

    #[test]
    fn test_u32_addition_execution() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("10u32 + 20u32")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let result = segment.call0::<u32>()?;
        assert_eq!(result, 30);
        Ok(())
    }

    #[test]
    fn test_identifier_with_scope() -> anyhow::Result<()> {
        let mut lookup = OpLookup::new();
        lookup.push_scope(|name, segment, num_operands, _span| {
            if num_operands == 0 {
                match name {
                    "x" => {
                        segment.op0(|| 10i32);
                        Ok(true)
                    }
                    "y" => {
                        segment.op0(|| 20i32);
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            } else {
                Ok(false)
            }
        });
        let mut parser = CELParser::new(lookup);
        let mut segment = parser
            .parse_str("x + y")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let result = segment.call0::<i32>()?;
        assert_eq!(result, 30);
        Ok(())
    }

    #[test]
    fn test_undefined_identifier_error() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("undefined_var + 10");

        assert!(result.is_err());
        if let Err(e) = result {
            let error_msg = format!("{:?}", e);
            assert!(
                error_msg.contains("undefined identifier: `undefined_var`"),
                "Error message should contain 'undefined identifier: `undefined_var`', got: {}",
                error_msg
            );
        }
    }

    #[test]
    fn test_identifier_scope_error_propagated() {
        let mut lookup = OpLookup::new();
        lookup.push_scope(|name, _segment, num_operands, _span| {
            if name == "bad_id" && num_operands == 0 {
                return Err(anyhow::anyhow!("custom identifier rejected"));
            }
            Ok(false)
        });
        let mut parser = CELParser::new(lookup);
        let err = match parser.parse_str("bad_id + 1") {
            Err(e) => e,
            Ok(_) => panic!("scope Err should propagate, not become Undefined identifier"),
        };
        assert!(
            err.message().contains("custom identifier rejected"),
            "expected scope error message, got: {}",
            err.message()
        );
        assert!(
            !err.message().contains("undefined identifier:"),
            "scope Err must not be rewritten as undefined identifier"
        );
    }

    #[test]
    fn test_undefined_identifier_error_formatting() {
        let input = "undefined_var + 10";
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str(input);

        assert!(result.is_err());
        if let Err(e) = result {
            let formatted_error = e.format_rustc_style(input, "test.cel", 1, &Renderer::plain());
            assert!(formatted_error.contains("undefined identifier"));
            assert!(formatted_error.contains("undefined_var"));
            assert!(formatted_error.contains("test.cel"));
        }
    }

    #[test]
    fn test_float_arithmetic_execution() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("3.5 * 2.0")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let result = segment.call0::<f64>()?;
        assert_eq!(result, 7.0);
        Ok(())
    }

    #[test]
    fn call_empty_arg_list() -> anyhow::Result<()> {
        let mut lookup = OpLookup::new();
        lookup.push_scope(
            |name, segment, num_operands, _span| match (name, num_operands) {
                ("f", 0) => {
                    segment.op0(|| 0i32);
                    Ok(true)
                }
                ("()", 1) => {
                    segment.op1(|_callee: i32| 99i32)?;
                    Ok(true)
                }
                _ => Ok(false),
            },
        );
        let mut parser = CELParser::new(lookup);
        let mut segment = parser
            .parse_str("f()")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        assert_eq!(segment.call0::<i32>()?, 99);
        Ok(())
    }

    #[test]
    fn call_single_arg() -> anyhow::Result<()> {
        let mut lookup = OpLookup::new();
        lookup.push_scope(
            |name, segment, num_operands, _span| match (name, num_operands) {
                ("f", 0) => {
                    segment.op0(|| 0i32);
                    Ok(true)
                }
                ("()", 2) => {
                    segment.op2(|_callee: i32, arg: i32| arg)?;
                    Ok(true)
                }
                _ => Ok(false),
            },
        );
        let mut parser = CELParser::new(lookup);
        let mut segment = parser
            .parse_str("f(42)")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        assert_eq!(segment.call0::<i32>()?, 42);
        Ok(())
    }

    #[test]
    fn call_multiple_args() -> anyhow::Result<()> {
        let mut lookup = OpLookup::new();
        lookup.push_scope(
            |name, segment, num_operands, _span| match (name, num_operands) {
                ("f", 0) => {
                    segment.op0(|| 0i32);
                    Ok(true)
                }
                ("()", 3) => {
                    segment.op3(|_callee: i32, arg1: i32, arg2: i32| arg1 + arg2)?;
                    Ok(true)
                }
                _ => Ok(false),
            },
        );
        let mut parser = CELParser::new(lookup);
        let mut segment = parser
            .parse_str("f(10, 32)")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        assert_eq!(segment.call0::<i32>()?, 42);
        Ok(())
    }

    #[test]
    fn call_missing_closing_paren() {
        let mut lookup = OpLookup::new();
        lookup.push_scope(
            |name, segment, num_operands, _span| match (name, num_operands) {
                ("f", 0) => {
                    segment.op0(|| 0i32);
                    Ok(true)
                }
                _ => Ok(false),
            },
        );
        let mut parser = CELParser::new(lookup);
        let err = match parser.parse_str("f(42 43)") {
            Err(e) => e,
            Ok(_) => panic!("expected parse error for missing closing parenthesis"),
        };
        assert_eq!(err.message(), "expected closing parenthesis");
    }

    #[test]
    fn call_trailing_comma() {
        let mut lookup = OpLookup::new();
        lookup.push_scope(
            |name, segment, num_operands, _span| match (name, num_operands) {
                ("f", 0) => {
                    segment.op0(|| 0i32);
                    Ok(true)
                }
                _ => Ok(false),
            },
        );
        let mut parser = CELParser::new(lookup);
        let err = match parser.parse_str("f(42,)") {
            Err(e) => e,
            Ok(_) => panic!("expected parse error for trailing comma"),
        };
        assert_eq!(err.message(), "expected expression after comma");
    }

    #[test]
    fn call_undefined_call_op() {
        let mut lookup = OpLookup::new();
        lookup.push_scope(
            |name, segment, num_operands, _span| match (name, num_operands) {
                ("f", 0) => {
                    segment.op0(|| 0i32);
                    Ok(true)
                }
                _ => Ok(false),
            },
        );
        let mut parser = CELParser::new(lookup);
        let err = match parser.parse_str("f()") {
            Err(e) => e,
            Ok(_) => panic!("expected error when () operator is not registered"),
        };
        assert!(
            err.message().starts_with("no operation"),
            "error should report no operation found, got: {}",
            err.message()
        );
    }

    #[test]
    fn call_leading_comma_reports_expected_expression_at_the_comma() {
        let mut lookup = OpLookup::new();
        lookup.push_scope(
            |name, segment, num_operands, _span| match (name, num_operands) {
                ("f", 0) => {
                    segment.op0(|| 0i32);
                    Ok(true)
                }
                _ => Ok(false),
            },
        );
        let mut parser = CELParser::new(lookup);
        let err = match parser.parse_str("f(,5)") {
            Err(e) => e,
            Ok(_) => panic!("expected parse error for leading comma"),
        };
        assert_eq!(err.message(), "expected expression");
    }

    #[test]
    fn call_chained() -> anyhow::Result<()> {
        let mut lookup = OpLookup::new();
        lookup.push_scope(
            |name, segment, num_operands, _span| match (name, num_operands) {
                ("f", 0) => {
                    segment.op0(|| 0i32);
                    Ok(true)
                }
                ("()", 1) => {
                    segment.op1(|_callee: i32| 7i32)?;
                    Ok(true)
                }
                _ => Ok(false),
            },
        );
        let mut parser = CELParser::new(lookup);
        let mut segment = parser
            .parse_str("f()()")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        assert_eq!(segment.call0::<i32>()?, 7);
        Ok(())
    }

    #[test]
    fn op_type_mismatch_error_spans_full_expression() {
        let source = r#""Hello" + 32.0"#;
        let mut parser = CELParser::new(OpLookup::new());
        let err = match parser.parse_str(source) {
            Err(e) => e,
            Ok(_) => panic!("expected parse error for type mismatch"),
        };
        assert!(
            err.message().starts_with("no operation"),
            "expected 'no operation' prefix, got: {}",
            err.message()
        );
        let end_span = err.end_span().expect("op-lookup errors carry an end span");
        assert!(
            end_span.end().column >= 14,
            "end span should reach the end of 32.0 (expected end.column >= 14, got {})",
            end_span.end().column
        );
    }

    #[test]
    fn and_short_circuits_on_false() {
        // Without short-circuit the RHS executes and division-by-zero errors.
        // With short-circuit the RHS fragment is skipped, returning false directly.
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("false && (1i32 / 0i32 == 0i32)")
            .expect("should parse");
        assert!(!segment.call0::<bool>().unwrap());
    }

    #[test]
    fn and_evaluates_rhs_when_lhs_true() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser.parse_str("true && false").expect("should parse");
        assert!(!segment.call0::<bool>().unwrap());
    }

    #[test]
    fn and_chained_short_circuits() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("false && false && false")
            .expect("should parse");
        assert!(!segment.call0::<bool>().unwrap());
    }

    #[test]
    fn and_lhs_type_error() {
        // LHS is i32, not bool — join2 must reject it at parse time.
        let mut parser = CELParser::new(OpLookup::new());
        let err = match parser.parse_str("1i32 && true") {
            Err(e) => e,
            Ok(_) => panic!("lhs i32 should fail for &&"),
        };
        assert!(err.end_span().is_some());
    }

    #[test]
    fn or_lhs_type_error() {
        // LHS is i32, not bool — join2 must reject it at parse time.
        let mut parser = CELParser::new(OpLookup::new());
        let err = match parser.parse_str("1i32 || true") {
            Err(e) => e,
            Ok(_) => panic!("lhs i32 should fail for ||"),
        };
        assert!(err.end_span().is_some());
    }

    #[test]
    fn or_short_circuits_on_true() {
        // Without short-circuit the RHS executes and division-by-zero errors.
        // With short-circuit the RHS fragment is skipped, returning true directly.
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("true || (1i32 / 0i32 == 0i32)")
            .expect("should parse");
        assert!(segment.call0::<bool>().unwrap());
    }

    #[test]
    fn or_evaluates_rhs_when_lhs_false() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser.parse_str("false || true").expect("should parse");
        assert!(segment.call0::<bool>().unwrap());
    }

    #[test]
    fn or_chained() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("true || false || false")
            .expect("should parse");
        assert!(segment.call0::<bool>().unwrap());
    }

    #[test]
    fn if_true_branch_selected() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("if true { 1i32 } else { 2i32 }")
            .expect("should parse");
        assert_eq!(segment.call0::<i32>().unwrap(), 1);
    }

    #[test]
    fn if_false_branch_selected() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("if false { 1i32 } else { 2i32 }")
            .expect("should parse");
        assert_eq!(segment.call0::<i32>().unwrap(), 2);
    }

    #[test]
    fn if_else_if_first_branch() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("if true { 1i32 } else if false { 2i32 } else { 3i32 }")
            .expect("should parse");
        assert_eq!(segment.call0::<i32>().unwrap(), 1);
    }

    #[test]
    fn if_else_if_middle_branch() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("if false { 1i32 } else if true { 2i32 } else { 3i32 }")
            .expect("should parse");
        assert_eq!(segment.call0::<i32>().unwrap(), 2);
    }

    #[test]
    fn if_else_if_last_branch() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("if false { 1i32 } else if false { 2i32 } else { 3i32 }")
            .expect("should parse");
        assert_eq!(segment.call0::<i32>().unwrap(), 3);
    }

    #[test]
    fn if_omitted_else_unit_branch() {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser.parse_str("if true { () }").expect("should parse");
        segment.call0::<()>().expect("should execute");
    }

    #[test]
    fn if_omitted_else_rejects_non_unit_then() {
        // then-branch returns i32, implicit else returns () — types must match.
        let mut parser = CELParser::new(OpLookup::new());
        assert!(parser.parse_str("if false { 1i32 }").is_err());
    }

    #[test]
    fn if_branch_type_mismatch_is_error() {
        let mut parser = CELParser::new(OpLookup::new());
        assert!(parser.parse_str("if true { 1i32 } else { true }").is_err());
    }

    #[test]
    fn if_branch_tuple_arity_mismatch_is_error() {
        // Regression test: every tuple shares the same erased `DynTuple`
        // marker type, so a naive type_id comparison would accept branches
        // with genuinely different tuple shapes — join2 must compare shapes,
        // not just the marker type, and reject this.
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("if false { (1i32, 2i32) } else { (3i64, 4i64, 5i64) }.0");
        assert!(
            result.is_err(),
            "branches with different tuple shapes must not be accepted"
        );
    }

    #[test]
    fn if_branch_tuple_element_type_mismatch_is_error() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("if false { (1i32, 2i32) } else { (3i64, 4i64) }.0");
        assert!(
            result.is_err(),
            "branches with the same arity but different element types must not be accepted"
        );
    }

    #[test]
    fn if_missing_open_brace_is_error() {
        let mut parser = CELParser::new(OpLookup::new());
        assert!(parser.parse_str("if true 1i32 } else { 2i32 }").is_err());
    }

    #[test]
    fn if_missing_else_after_brace_is_fine() {
        // Omitting else is allowed; result type must be ().
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser.parse_str("if false { () }").expect("should parse");
        segment.call0::<()>().expect("should execute");
    }

    #[test]
    fn if_trailing_else_is_error() {
        // `else` with no body is a parse error.
        let mut parser = CELParser::new(OpLookup::new());
        assert!(parser.parse_str("if true { () } else").is_err());
    }

    #[test]
    fn parse_or_expression_stops_before_comma() -> anyhow::Result<()> {
        use lex_lexer::LexLexer;
        let stream: proc_macro2::TokenStream = "10i32 + 20i32, 5i32".parse().unwrap();
        let mut parser = CELParser::new(OpLookup::new());
        parser.set_lex_tokens(LexLexer::new(stream.into_iter()).peekable());
        let mut seg = parser
            .parse_or_expression()
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let result: i32 = seg.call0()?;
        assert_eq!(result, 30);
        let remaining: Vec<_> = parser.take_lex_tokens().expect("tokens present").collect();
        // The comma and "5i32" should remain unconsumed.
        assert_eq!(
            remaining.len(),
            2,
            "expected 2 remaining tokens (comma and 5i32)"
        );
        Ok(())
    }

    #[test]
    fn parse_or_expression_on_empty_input_returns_error() {
        use lex_lexer::LexLexer;
        let stream: proc_macro2::TokenStream = "".parse().unwrap();
        let mut parser = CELParser::new(OpLookup::new());
        parser.set_lex_tokens(LexLexer::new(stream.into_iter()).peekable());
        let result = parser.parse_or_expression();
        assert!(result.is_err(), "expected Err for empty input");
    }

    #[test]
    fn closure_literal_with_one_param_compiles_and_calls() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("|x: i32| x + 1")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let closure: cel_runtime::DynClosure = segment.call0()?;
        let x = 5i32;
        assert_eq!(closure.call::<i32>(&[&x])?, 6);
        Ok(())
    }

    #[test]
    fn closure_literal_with_zero_params_compiles_and_calls() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("|| 42")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let closure: cel_runtime::DynClosure = segment.call0()?;
        assert_eq!(closure.call::<i32>(&[])?, 42);
        Ok(())
    }

    #[test]
    fn closure_literal_with_two_params_compiles_and_calls_in_order() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("|a: i32, b: i32| a - b")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let closure: cel_runtime::DynClosure = segment.call0()?;
        let (a, b) = (10i32, 3i32);
        assert_eq!(closure.call::<i32>(&[&a, &b])?, 7);
        Ok(())
    }

    #[test]
    fn closure_literal_with_tuple_typed_param_compiles_and_calls() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("|r: (i32, i32)| r.0 + r.1")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let closure: cel_runtime::DynClosure = segment.call0()?;

        // Build a `DynamicSequence` shaped like `(i32, i32)` — the runtime-erased tuple value a
        // real caller (e.g. adam-lang, reading a tuple-typed cell) would hand a closure whose
        // parameter type is a tuple.
        let mut pair = DynSegment::new::<()>();
        let ambient_start = pair.current_stack_offset();
        pair.op0(|| 10i32);
        pair.op0(|| 20i32);
        pair.make_tuple(2, ambient_start);
        let leaf = |type_id: TypeId| {
            (type_id == TypeId::of::<i32>()).then(|| {
                (
                    cel_runtime::element_dropper_for::<i32>(),
                    cel_runtime::element_cloner_for::<i32>(),
                    cel_runtime::element_eq_for::<i32>(),
                    cel_runtime::element_debug_for::<i32>(),
                )
            })
        };
        let pair: cel_runtime::DynamicSequence = pair.call_dyn_as_dynamic_sequence(&[], &leaf)?;

        assert_eq!(closure.call::<i32>(&[&pair])?, 30);
        Ok(())
    }

    #[test]
    fn closure_body_referencing_an_undeclared_name_is_a_parse_error() {
        let mut parser = CELParser::new(OpLookup::new());
        let err = parser.parse_str("|x: i32| x + y");
        assert!(err.is_err());
    }

    #[test]
    fn nested_closure_referencing_only_its_own_param_compiles_and_calls() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        // The outer closure's own parameter `x` must NOT be visible inside the inner closure
        // body — a bare nested closure literal parses fine directly as the outer body's
        // `expression`, with no extra block grouping needed (this grammar has no bare
        // block-expression production, only `if`'s own braces, so `{ ... }` grouping isn't an
        // option here anyway).
        let mut segment = parser
            .parse_str("|x: i32| |y: i32| y + 1")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let outer: cel_runtime::DynClosure = segment.call0()?;
        let x = 0i32;
        let inner: cel_runtime::DynClosure = outer.call(&[&x])?;
        let y = 41i32;
        assert_eq!(inner.call::<i32>(&[&y])?, 42);
        Ok(())
    }

    #[test]
    fn closure_body_cannot_see_an_enclosing_scopes_names() {
        // A scope pushed before parsing (standing in for e.g. adam-lang's own cell-name scope)
        // must not leak into a closure body's name resolution.
        let mut lookup = OpLookup::new();
        lookup.push_scope(|name, segment, arity, _span| {
            if name == "outer_only" && arity == 0 {
                segment.just(1i32);
                Ok(true)
            } else {
                Ok(false)
            }
        });
        let mut parser = CELParser::new(lookup);
        let err = parser.parse_str("|x: i32| x + outer_only");
        assert!(err.is_err());
    }

    #[test]
    fn ast_context_parses_a_one_param_closure() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("|x: i32| x").unwrap();
        let Expr::Closure { params, body, .. } = expr else {
            panic!("expected Closure");
        };
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "x");
        assert!(matches!(
            &params[0].type_expr,
            ClosureParamTypeExpr::Named(n, _) if n == "i32"
        ));
        assert!(matches!(*body, Expr::Ident { ref name, .. } if name == "x"));
    }

    #[test]
    fn ast_context_parses_a_zero_param_closure() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("|| 1i32").unwrap();
        let Expr::Closure { params, .. } = expr else {
            panic!("expected Closure");
        };
        assert!(params.is_empty());
    }

    #[test]
    fn ast_context_parses_a_tuple_typed_closure_param() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("|x: (i32, f64)| x.0").unwrap();
        let Expr::Closure { params, .. } = expr else {
            panic!("expected Closure");
        };
        match &params[0].type_expr {
            ClosureParamTypeExpr::Tuple(elements, _) => assert_eq!(elements.len(), 2),
            other => panic!("expected Tuple, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod playground {
    use super::*;
    use annotate_snippets::Renderer;

    #[test]
    fn custom_scope_identifier() -> Result<()> {
        let mut lookup = OpLookup::new();
        lookup.push_scope(|name, segment, _num_operands, _span| {
            if name == "constant" {
                segment.just(42i64);
                return Ok(true);
            }
            Ok(false)
        });
        let mut parser = CELParser::new(lookup);
        let line = line!() + 1;
        let source = r#"
            (("hello" + " world") == constant) && (15i64 < constant)
        "#;
        match parser.parse_str(source) {
            Ok(mut seg) => println!("{:?}", seg.call0::<bool>()),
            Err(e) => println!(
                "{}",
                e.format_rustc_style(source, file!(), line, &Renderer::styled())
            ),
        }
        Ok(())
    }

    #[test]
    fn expression_macro_error3() {
        use CELParser;
        use op_table::OpLookup;

        let line = line!() + 1;
        let source = r#"
            "Hello" + "World" + 32.0
        "#;
        match CELParser::new(OpLookup::new()).parse_str(source) {
            Ok(_) => panic!("expected parse error"),
            Err(e) => println!(
                "{}",
                e.format_rustc_style(source, file!(), line, &Renderer::styled())
            ),
        }
    }

    #[test]
    fn arithmetic_overflow_error() {
        use error::FormatRustcStyle;

        let line = line!() + 1;
        let source = r#"
           1 + 1 +
                2147483646 + 1
        "#;
        let mut seg = CELParser::new(OpLookup::new())
            .parse_str(source)
            .expect("parses successfully");
        match seg.call0::<i32>() {
            Ok(v) => panic!("expected overflow, got {v}"),
            Err(e) => println!(
                "{}",
                e.format_rustc_style(source, file!(), line, &Renderer::styled())
            ),
        }
    }
}
