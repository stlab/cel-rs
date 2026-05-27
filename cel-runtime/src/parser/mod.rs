#![warn(missing_docs)]

//! A recursive descent parser for CEL (Common Expression Language) expressions.
//!
//! This crate provides a parser that can parse CEL expressions into executable segments.
//! The parser follows the CEL grammar specification and provides detailed error reporting
//! with source location information.
//!
//! # Error Handling
//!
//! Parse errors are returned as [`CELError`], which carries a message and source span for diagnostics.
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
//! multiplicative_expression = unary_expression { ("*" | "/" | "%") unary_expression }.
//! unary_expression = (("-" | "!") unary_expression) | postfix_expression.
//! postfix_expression = primary_expression { "(" parameter_list ")" }.
//! primary_expression = literal | identifier | "(" or_expression ")".
//! parameter_list = [ or_expression { "," or_expression } ].
//! ```
//!
//! # Note

//! `?eos?` denotes end of stream.
//!
//! # Examples
//!
//! ```rust
//! use cel_runtime::DynSegment;
//! use std::str::FromStr;
//!
//! let mut segment: DynSegment = "10u32 + 20u32 * 5u32".parse().unwrap();
//! let result = segment.call0::<u32>();
//! assert!(result.is_ok());
//! assert_eq!(result.unwrap(), 110); // 10 + 20 * 5 = 10 + 100
//! ```
//!
//! ## Basic Usage
//!
//! ```rust
//! use cel_runtime::parser::CELParser;
//! use cel_runtime::OpLookup;
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
//! use cel_runtime::parser::CELParser;
//! use cel_runtime::OpLookup;
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
//!     println!("{}", e.format_rustc_style(source, file!(), line));
//!     // Output:
//!     // error: unexpected token
//!     //  --> example.cel:1:4
//!     //   |
//!     // 1 | 10 20
//!     //   |    ^^
//! }
//! ```

mod error;
mod lex_lexer;
pub mod op_table;

pub use error::{CELError, SourceSpan};
pub use proc_macro2::LineColumn;

use lex_lexer::{LexLexer, Literal as CelLiteral, Token, TokenStreamIter};
use op_table::OpLookup;

use crate::DynSegment;
use proc_macro2::{Delimiter, Ident, Literal, Span, TokenStream};
use std::iter::Peekable;
use std::str::FromStr;

/// Parser result type.
pub type Result<T> = std::result::Result<T, CELError>;

fn push_literal(output: &mut DynSegment, lit: CelLiteral) {
    match lit {
        CelLiteral::Int(integer) => {
            // Use syn's suffix() to determine the type
            match integer.suffix() {
                "u8" => output.just(
                    integer
                        .base10_parse::<u8>()
                        .expect("failed to parse u8 literal"),
                ),
                "u16" => output.just(
                    integer
                        .base10_parse::<u16>()
                        .expect("failed to parse u16 literal"),
                ),
                "u32" => output.just(
                    integer
                        .base10_parse::<u32>()
                        .expect("failed to parse u32 literal"),
                ),
                "u64" => output.just(
                    integer
                        .base10_parse::<u64>()
                        .expect("failed to parse u64 literal"),
                ),
                "u128" => output.just(
                    integer
                        .base10_parse::<u128>()
                        .expect("failed to parse u128 literal"),
                ),
                "usize" => output.just(
                    integer
                        .base10_parse::<usize>()
                        .expect("failed to parse usize literal"),
                ),
                "i8" => output.just(
                    integer
                        .base10_parse::<i8>()
                        .expect("failed to parse i8 literal"),
                ),
                "i16" => output.just(
                    integer
                        .base10_parse::<i16>()
                        .expect("failed to parse i16 literal"),
                ),
                "i64" => output.just(
                    integer
                        .base10_parse::<i64>()
                        .expect("failed to parse i64 literal"),
                ),
                "i128" => output.just(
                    integer
                        .base10_parse::<i128>()
                        .expect("failed to parse i128 literal"),
                ),
                "isize" => output.just(
                    integer
                        .base10_parse::<isize>()
                        .expect("failed to parse isize literal"),
                ),
                _ => {
                    // No suffix means i32 by default
                    output.just(
                        integer
                            .base10_parse::<i32>()
                            .expect("failed to parse i32 literal"),
                    )
                }
            }
        }
        CelLiteral::Float(float) => {
            // Use syn's suffix() to determine the type
            match float.suffix() {
                "f32" => output.just(
                    float
                        .base10_parse::<f32>()
                        .expect("failed to parse f32 literal"),
                ),
                _ => {
                    // No suffix or "f64" means f64 by default
                    output.just(
                        float
                            .base10_parse::<f64>()
                            .expect("failed to parse f64 literal"),
                    )
                }
            }
        }
        CelLiteral::Str(string) => {
            // Store the string value (without quotes)
            output.just(string.value());
        }
        CelLiteral::Bool(lit_bool) => {
            // Push the boolean value directly
            output.just(lit_bool.value);
        }
        CelLiteral::Char(ch) => {
            // Push character literal
            output.just(ch.value());
        }
        CelLiteral::Byte(byte) => {
            // Push byte literal (u8)
            output.just(byte.value());
        }
        CelLiteral::ByteStr(byte_str) => {
            // Push byte string as Vec<u8>
            output.just(byte_str.value());
        }
        CelLiteral::CStr(c_str) => {
            // Push C string directly
            output.just(c_str.value());
        }
        CelLiteral::Verbatim(_) => {
            unreachable!("Verbatim literals should never occur")
        }
        _ => {
            // Future literal types not yet handled
        }
    }
}

/// A recursive descent parser for expressions.
///
/// # Examples
///
/// ## Basic Usage
///
/// ```rust
/// use cel_runtime::OpLookup;
/// use cel_runtime::parser::CELParser;
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
/// use cel_runtime::OpLookup;
/// use cel_runtime::parser::CELParser;
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
///     println!("{}", e.format_rustc_style(source, file!(), line));
///     // Output:
///     // error: unexpected token
///     //  --> example.cel:1:8
///     //   |
///     // 1 | 10 + 20 30
///     //   |         ^^
/// }
/// ```
pub struct CELParser {
    tokens: Option<Peekable<LexLexer>>,
    context: DynSegment,
    op_lookup: OpLookup,
}

/// A primary expression representing the most basic expression types.
///
/// Primary expressions are the atomic building blocks of CEL expressions,
/// consisting of either literal values or identifiers.
pub enum PrimaryExpression {
    /// A literal value (integer, string, boolean, or float).
    Literal(Literal),
    /// An identifier referencing a variable or function.
    Ident(Ident),
}

/// Result type for parser probe operations.
///
/// A `Probe` represents the outcome of attempting to parse a specific grammar
/// production without committing to the parse. This enables backtracking and
/// alternative parsing strategies.
pub enum Probe<T> {
    /// The probe did not match the expected grammar production.
    NoMatch,
    /// The probe matched but produced no value (e.g., optional production absent).
    Match,
    /// The probe matched and produced a value.
    Value(T),
}

/// A probe result for primary expression parsing.
pub type PrimaryProbe = Probe<PrimaryExpression>;

impl CELParser {
    /// Creates a new CEL parser with the given operation lookup.
    ///
    /// No tokens are set at construction; use [`set_tokens`](Self::set_tokens),
    /// [`parse_tokens`](Self::parse_tokens), or [`parse_str`](Self::parse_str) to parse.
    ///
    /// # Arguments
    ///
    /// * `op_lookup` - Operation lookup for resolving operators and identifiers
    pub fn new(op_lookup: OpLookup) -> Self {
        CELParser {
            tokens: None,
            context: DynSegment::new::<()>(),
            op_lookup,
        }
    }

    /// Sets the token stream for parsing.
    ///
    /// Call before [`is_expression`](Self::is_expression) or use [`parse_tokens`](Self::parse_tokens)
    /// which sets tokens and parses in one step.
    pub fn set_tokens(&mut self, tokens: TokenStreamIter) {
        self.tokens = Some(LexLexer::new(tokens).peekable());
        self.context = DynSegment::new::<()>();
    }

    /// Parses a token stream into a [`DynSegment`].
    ///
    /// Sets the token source, runs the expression grammar, and returns the segment on success.
    ///
    /// # Errors
    ///
    /// Returns an error if the input does not contain a valid CEL expression.
    pub fn parse_tokens(&mut self, tokens: TokenStreamIter) -> Result<DynSegment> {
        self.set_tokens(tokens);
        if !self.is_expression()? {
            return Err(self.error_at("expression expected"));
        }
        Ok(std::mem::replace(
            &mut self.context,
            DynSegment::new::<()>(),
        ))
    }

    /// Parses a string into a [`DynSegment`].
    ///
    /// Tokenizes the string then parses; equivalent to `parse_tokens(TokenStream::from_str(s)?.into_iter())`.
    ///
    /// # Errors
    ///
    /// Returns an error on lex failure or if the input does not contain a valid CEL expression.
    pub fn parse_str(&mut self, s: &str) -> Result<DynSegment> {
        let input = TokenStream::from_str(s)
            .map_err(|e| CELError::with_proc_macro_span(format!("lex: {}", e), e.span()))?;
        self.parse_tokens(input.into_iter())
    }

    /// Returns a mutable reference to the operation lookup.
    ///
    /// This allows customization of the operations available during parsing,
    /// such as adding new scopes for custom operations or identifiers.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::parser::op_table::OpLookup;
    /// use cel_runtime::parser::CELParser;
    /// use cel_runtime::DynSegment;
    /// use proc_macro2::TokenStream;
    /// use std::any::TypeId;
    /// use std::str::FromStr;
    ///
    /// let input = TokenStream::from_str("10 + 20").unwrap();
    /// let mut lookup = OpLookup::new();
    /// lookup.push_scope(|name, segment, num_operands| {
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

    fn advance(&mut self) {
        self.tokens.as_mut().expect("tokens set").next();
    }

    /// Peeks at the current token without consuming it.
    ///
    /// Returns `None` if there are no more tokens.
    fn peek_token(&mut self) -> Option<&Token> {
        self.tokens.as_mut().expect("tokens set").peek()
    }

    /// Builds a [`CELError`] at the current token's span (or call_site if no token).
    fn error_at(&mut self, message: &str) -> CELError {
        let span = match self.peek_token() {
            Some(token) => {
                use lex_lexer::HasSpan;
                token.span()
            }
            None => Span::call_site(),
        };
        CELError::new(message, SourceSpan::from_proc_macro2(span))
    }

    fn is_punctuation(&mut self, target: &str) -> bool {
        // Simply check if the current token is a Punct with the target operator
        match self.peek_token() {
            Some(Token::Punct { op, .. }) if op == target => {
                self.advance();
                true
            }
            _ => false,
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
    fn is_or_expression(&mut self) -> Result<bool> {
        if self.is_and_expression()? {
            while self.is_punctuation("||") {
                if !self.is_and_expression()? {
                    return Err(self.error_at("expected and_expression"));
                }
                if self.context.stack_ids.len() >= 2
                    && let Err(e) = self.op_lookup.lookup("||", &mut self.context, 2)
                {
                    return Err(self.error_at(&format!("operation error: {}", e)));
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `and_expression = comparison_expression { "&&" comparison_expression }.`
    fn is_and_expression(&mut self) -> Result<bool> {
        if self.is_comparison_expression()? {
            while self.is_punctuation("&&") {
                if !self.is_comparison_expression()? {
                    return Err(self.error_at("expected comparison_expression"));
                }
                if self.context.stack_ids.len() >= 2
                    && let Err(e) = self.op_lookup.lookup("&&", &mut self.context, 2)
                {
                    return Err(self.error_at(&format!("operation error: {}", e)));
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `comparison_expression = bitwise_or_expression [ ("==" | "!=" | "<" | ">" | "<=" | ">=") bitwise_or_expression ].`
    fn is_comparison_expression(&mut self) -> Result<bool> {
        if self.is_bitwise_or_expression()? {
            // Check which operator we have (check longer operators first)
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
                if self.context.stack_ids.len() >= 2
                    && let Err(e) = self.op_lookup.lookup(op_name, &mut self.context, 2)
                {
                    return Err(self.error_at(&format!("operation error: {}", e)));
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `bitwise_or_expression = bitwise_xor_expression { "|" bitwise_xor_expression }.`
    fn is_bitwise_or_expression(&mut self) -> Result<bool> {
        if self.is_bitwise_xor_expression()? {
            while self.is_punctuation("|") {
                if !self.is_bitwise_xor_expression()? {
                    return Err(self.error_at("expected bitwise_xor_expression"));
                }
                if self.context.stack_ids.len() >= 2
                    && let Err(e) = self.op_lookup.lookup("|", &mut self.context, 2)
                {
                    return Err(self.error_at(&format!("operation error: {}", e)));
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `bitwise_xor_expression = bitwise_and_expression { "^" bitwise_and_expression }.`
    fn is_bitwise_xor_expression(&mut self) -> Result<bool> {
        if self.is_bitwise_and_expression()? {
            while self.is_punctuation("^") {
                if !self.is_bitwise_and_expression()? {
                    return Err(self.error_at("expected bitwise_and_expression"));
                }
                if self.context.stack_ids.len() >= 2
                    && let Err(e) = self.op_lookup.lookup("^", &mut self.context, 2)
                {
                    return Err(self.error_at(&format!("operation error: {}", e)));
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `bitwise_and_expression = bitwise_shift_expression { "&" bitwise_shift_expression }.`
    fn is_bitwise_and_expression(&mut self) -> Result<bool> {
        if self.is_bitwise_shift_expression()? {
            while self.is_punctuation("&") {
                if !self.is_bitwise_shift_expression()? {
                    return Err(self.error_at("expected bitwise_shift_expression"));
                }
                if self.context.stack_ids.len() >= 2
                    && let Err(e) = self.op_lookup.lookup("&", &mut self.context, 2)
                {
                    return Err(self.error_at(&format!("operation error: {}", e)));
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `bitwise_shift_expression = additive_expression { ("<<" | ">>") additive_expression }.`
    fn is_bitwise_shift_expression(&mut self) -> Result<bool> {
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
                    if self.context.stack_ids.len() >= 2
                        && let Err(e) = self.op_lookup.lookup(op_name, &mut self.context, 2)
                    {
                        return Err(self.error_at(&format!("operation error: {}", e)));
                    }
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
        if self.is_multiplicative_expression()? {
            loop {
                // Check which operator we have
                let op_name = if self.is_punctuation("+") {
                    Some("+")
                } else if self.is_punctuation("-") {
                    Some("-")
                } else {
                    None
                };

                // If we found an operator, parse the right operand and apply the operation
                if let Some(op_name) = op_name {
                    if !self.is_multiplicative_expression()? {
                        return Err(self.error_at("expected multiplicative_expression"));
                    }

                    if self.context.stack_ids.len() >= 2
                        && let Err(e) = self.op_lookup.lookup(op_name, &mut self.context, 2)
                    {
                        return Err(self.error_at(&format!("operation error: {}", e)));
                    }
                } else {
                    break;
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `multiplicative_expression = unary_expression { ("*" | "/" | "%") unary_expression }.`
    fn is_multiplicative_expression(&mut self) -> Result<bool> {
        if self.is_unary_expression()? {
            loop {
                // Check which operator we have
                let op_name = if self.is_punctuation("*") {
                    Some("*")
                } else if self.is_punctuation("/") {
                    Some("/")
                } else if self.is_punctuation("%") {
                    Some("%")
                } else {
                    None
                };

                // If we found an operator, parse the right operand and apply the operation
                if let Some(op_name) = op_name {
                    if !self.is_unary_expression()? {
                        return Err(self.error_at("expected unary_expression"));
                    }

                    if self.context.stack_ids.len() >= 2
                        && let Err(e) = self.op_lookup.lookup(op_name, &mut self.context, 2)
                    {
                        return Err(self.error_at(&format!("operation error: {}", e)));
                    }
                } else {
                    break;
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `unary_expression = (("-" | "!") unary_expression) | primary_expression.`
    fn is_unary_expression(&mut self) -> Result<bool> {
        // Check for unary operators
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
            // Apply the unary operation (only if we have types)
            if !self.context.stack_ids.is_empty()
                && let Err(e) = self.op_lookup.lookup(op_name, &mut self.context, 1)
            {
                return Err(self.error_at(&format!("operation error: {}", e)));
            }
            Ok(true)
        } else {
            self.is_postfix_expression()
        }
    }

    /// `postfix_expression = primary_expression { "(" parameter_list ")" }.`
    fn is_postfix_expression(&mut self) -> Result<bool> {
        if !self.is_primary_expression()? {
            return Ok(false);
        }
        while matches!(
            self.peek_token(),
            Some(Token::OpenDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            })
        ) {
            self.advance(); // consume "("
            let arg_count = self.parameter_list()?;
            match self.peek_token() {
                Some(Token::CloseDelim {
                    delimiter: Delimiter::Parenthesis,
                    ..
                }) => {
                    self.advance(); // consume ")"
                }
                _ => return Err(self.error_at("expected closing parenthesis")),
            }
            // Push the call operation: pops argument(s) then callee, invokes callee, pushes result.
            // Stack order is [callee, arg1, arg2, ...]; lookup peeks top (arg_count + 1) entries.
            if self.context.stack_ids.len() > arg_count
                && let Err(e) = self
                    .op_lookup
                    .lookup("()", &mut self.context, arg_count + 1)
            {
                return Err(self.error_at(&format!("call: {}", e)));
            }
        }
        Ok(true)
    }

    /// `parameter_list = [ or_expression { "," or_expression } ].`
    ///
    /// Returns the argument count.
    fn parameter_list(&mut self) -> Result<usize> {
        let mut count = 0;
        if self.is_or_expression()? {
            count += 1;
            while self.is_punctuation(",") {
                if !self.is_or_expression()? {
                    return Err(self.error_at("expected expression after comma"));
                }
                count += 1;
            }
        }
        Ok(count)
    }

    /// `primary_expression = literal | identifier | "(" or_expression ")".`
    fn is_primary_expression(&mut self) -> Result<bool> {
        match self.peek_token() {
            Some(Token::Literal(lit)) => {
                // Clone the literal - syn's Lit types are Clone
                let lit_clone = lit.clone();
                self.advance();
                // Push the literal to the context
                push_literal(&mut self.context, lit_clone);
                Ok(true)
            }
            Some(Token::Identifier(ident)) => {
                let ident_name = ident.to_string();
                self.advance();

                // Look up identifier (variable/0-ary); value is pushed and may be a function.
                self.op_lookup
                    .lookup(&ident_name, &mut self.context, 0)
                    .map_err(|_| self.error_at(&format!("Undefined identifier: {}", ident_name)))?;

                Ok(true)
            }
            Some(Token::OpenDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            }) => {
                self.advance(); // consume OpenDelim
                // Recursively parse the expression inside parentheses
                if !self.is_or_expression()? {
                    return Err(self.error_at("expected expression"));
                }
                // Expect CloseDelim
                match self.peek_token() {
                    Some(Token::CloseDelim {
                        delimiter: Delimiter::Parenthesis,
                        ..
                    }) => {
                        self.advance(); // consume CloseDelim
                        Ok(true)
                    }
                    _ => Err(self.error_at("expected closing parenthesis")),
                }
            }
            _ => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow;

    #[test]
    fn experiments() -> Result<()> {
        let mut lookup = OpLookup::new();
        lookup.push_scope(|name, segment, _num_operands| {
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
        // assert!(parser.parse_str(source)?.call0::<bool>()?);

        if let Err(e) = parser.parse_str(source) {
            println!("{}", e.format_rustc_style(source, file!(), line));
        }
        Ok(())
    }

    #[test]
    fn simple_expression() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("10");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().call0::<i32>().unwrap(), 10);
    }

    #[test]
    fn float_literal() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("3.14");
        assert!(result.is_ok());
        let value = result.unwrap().call0::<f64>().unwrap();
        assert!((value - 3.14).abs() < 1e-10);
    }

    #[test]
    fn boolean_literal() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("true");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().call0::<bool>().unwrap(), true);
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
        let result = parser.parse_str("8 << 2 + 16 >> 1");
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
        assert_eq!(result.unwrap().call0::<bool>().unwrap(), true); // !!true = true
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
        assert_eq!(result.unwrap().call0::<bool>().unwrap(), true);
    }

    #[test]
    fn invalid_expression() {
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str("+");
        assert!(result.is_err());
    }

    /// Helper function to strip ANSI escape codes from a string for testing purposes
    fn strip_ansi_codes(input: &str) -> String {
        // Basic regex to remove ANSI escape sequences
        // ANSI escape sequences start with ESC (0x1B) followed by '[' and end with a letter
        let mut result = String::new();
        let mut chars = input.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '\x1B' {
                // Found ESC, check if it's followed by '['
                if chars.peek() == Some(&'[') {
                    chars.next(); // consume '['
                    // Skip until we find a letter (which ends the escape sequence)
                    while let Some(ch) = chars.next() {
                        if ch.is_ascii_alphabetic() {
                            break;
                        }
                    }
                } else {
                    result.push(ch);
                }
            } else {
                result.push(ch);
            }
        }

        result
    }

    #[test]
    fn error_formatting() {
        let source = "10 + 20 30"; // Missing operator between 20 and 30
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str(source);

        // This should fail parsing
        assert!(result.is_err());

        // Test error message from result
        let err = match &result {
            Ok(_) => panic!("expected parse error"),
            Err(e) => e,
        };
        assert_eq!(err.message(), "unexpected token");

        // Test error formatting
        let formatted_error = err.format_rustc_style(source, "test.cel", 1u32);

        // Strip ANSI codes for testing
        let formatted = strip_ansi_codes(&formatted_error);
        assert!(formatted.contains("error: unexpected token"));
        assert!(formatted.contains("test.cel:1:")); // Should include line number
        assert!(formatted.contains("1 | 10 + 20 30")); // Should show the line with line number
        assert!(formatted.contains("^")); // Should have carets pointing to the error
    }

    #[test]
    fn error_formatting_with_line_offset() {
        let source = "10 + 20 30"; // Missing operator between 20 and 30
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str(source);

        // This should fail parsing
        assert!(result.is_err());

        // Test error formatting with line offset (as if expression starts at line 42)
        let err = match &result {
            Ok(_) => panic!("expected parse error"),
            Err(e) => e,
        };
        let formatted_error = err.format_rustc_style(source, "large_file.rs", 42u32);

        // Strip ANSI codes for testing
        let formatted = strip_ansi_codes(&formatted_error);
        assert!(formatted.contains("error: unexpected token"));
        assert!(formatted.contains("large_file.rs:42:")); // Should show offset line number
        assert!(formatted.contains("42 | 10 + 20 30")); // Should show the line with offset line number
        assert!(formatted.contains("^")); // Should have carets pointing to the error
    }

    #[test]
    fn print_error_formatting() {
        let line = line!() + 1;
        let source = r#"

         10 + 20  30 // Unexpected token

     "#;

        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str(source);

        // Parse should fail due to unexpected token
        assert!(result.is_err(), "Expected parsing to fail");

        let err = match &result {
            Ok(_) => panic!("expected parse error"),
            Err(e) => e,
        };
        eprintln!(
            "DEBUG: span.start.line = {}, span.start.column = {}",
            err.span().start.line,
            err.span().start.column
        );

        // Format the error
        let formatted_error = err.format_rustc_style(source, file!(), line);
        println!("{}", formatted_error);

        // Strip ANSI codes for testing
        let formatted = strip_ansi_codes(&formatted_error);

        // The source string has 3 lines:
        // Line 0: empty
        // Line 1: empty
        // Line 2: "         10 + 20  30 // Unexpected token"
        // So the error should be on line + 2
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
        assert_eq!(result, 70); // 10 + (20 * 3) = 10 + 60 = 70
        Ok(())
    }

    #[test]
    fn test_parenthesized_arithmetic_execution() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("(10 + 20) * 3")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let result = segment.call0::<i32>()?;
        assert_eq!(result, 90); // (10 + 20) * 3 = 30 * 3 = 90
        Ok(())
    }

    #[test]
    fn test_comparison_execution() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("10 < 20")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let result = segment.call0::<bool>()?;
        assert_eq!(result, true);
        Ok(())
    }

    #[test]
    fn test_logical_and_execution() -> anyhow::Result<()> {
        let mut parser = CELParser::new(OpLookup::new());
        let mut segment = parser
            .parse_str("true && false")
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let result = segment.call0::<bool>()?;
        assert_eq!(result, false);
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
        assert_eq!(result, false);
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
        lookup.push_scope(|name, segment, num_operands| {
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
                error_msg.contains("Undefined identifier: undefined_var"),
                "Error message should contain 'Undefined identifier: undefined_var', got: {}",
                error_msg
            );
        }
    }

    #[test]
    fn test_undefined_identifier_error_formatting() {
        let input = "undefined_var + 10";
        let mut parser = CELParser::new(OpLookup::new());
        let result = parser.parse_str(input);

        assert!(result.is_err());
        if let Err(e) = result {
            let formatted_error = e.format_rustc_style(input, "test.cel", 1);
            assert!(formatted_error.contains("Undefined identifier"));
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
}
