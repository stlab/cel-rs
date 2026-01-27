#![warn(missing_docs)]

//! A recursive descent parser for CEL (Common Expression Language) expressions.
//!
//! This crate provides a parser that can parse CEL expressions into executable segments. 
//! The parser follows the CEL grammar specification and provides detailed error reporting 
//! with source location information.
//!
//! # Error Handling
//!
//! Parse errors are returned as `anyhow::Error` to keep the interface flexible as the 
//! grammar evolves. All errors result from malformed input (syntax errors, type mismatches, 
//! undefined identifiers). The lexer itself cannot produce errors since input is pre-validated 
//! by `proc_macro2` before reaching the parser.
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
//! unary_expression = (("-" | "!") unary_expression) | primary_expression.
//! primary_expression = literal | identifier | "(" expression ")".
//! ```
//!
//! # Note

//! `?eos?` denotes end of stream. Because parenthesized expressions are nested by the lexer as a
//! separate token stream, this parses correctly in `primary_expression`.
//!
//! # Examples
//!
//! ## Basic Usage
//!
//! ```rust
//! use cel_parser::CELParser;
//! use proc_macro2::TokenStream;
//! use std::str::FromStr;
//!
//! let input = TokenStream::from_str("10").unwrap();
//! let mut parser = CELParser::new(input.into_iter());
//! let result = parser.is_expression();
//! assert!(result.is_ok());
//! ```
//!
//! ## Error Formatting
//!
//! ```rust
//! use cel_parser::CELParser;
//! use proc_macro2::TokenStream;
//! use std::str::FromStr;
//!
//! let line = line!() + 1;
//! let source = r#"
//!   10 20
//! "#; // Invalid: missing operator
//! let input = TokenStream::from_str(source).unwrap();
//! let mut parser = CELParser::new(input.into_iter());
//!
//! if parser.is_expression().is_err() {
//!     // Format error starting at line 1
//!     if let Some(formatted_error) = parser.format_error(source, file!(), line) {
//!         println!("{}", formatted_error);
//!         // Output:
//!         // error: unexpected token
//!         //  --> example.cel:1:4
//!         //   |
//!         // 1 | 10 20
//!         //   |    ^^
//!     }
//! }
//! ```

mod lex_lexer;
pub mod op_table;

use lex_lexer::{LexLexer, Literal as CelLiteral, Token};
use op_table::OpLookup;

use anyhow::Result;
use cel_runtime::DynSegment;
use owo_colors::OwoColorize;
use proc_macro2::{Delimiter, Ident, Literal, Span, TokenStream, TokenTree};
use quote::quote_spanned;
use std::iter::Peekable;
use syn::Lit;

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
/// Grammar:
/// ```text
/// expression = or_expression ?eos?.
/// or_expression = and_expression { "||" and_expression }.
/// and_expression = comparison_expression { "&&" comparison_expression }.
/// comparison_expression = bitwise_or_expression
///     [ ("==" | "!=" | "<" | ">" | "<=" | ">=") bitwise_or_expression ].
/// bitwise_or_expression = bitwise_xor_expression { "|" bitwise_xor_expression }.
/// bitwise_xor_expression = bitwise_and_expression { "^" bitwise_and_expression }.
/// bitwise_and_expression = bitwise_shift_expression { "&" bitwise_shift_expression }.
/// bitwise_shift_expression = additive_expression { ("<<" | ">>") additive_expression }.
/// additive_expression = multiplicative_expression { ("+" | "-") multiplicative_expression }.
/// multiplicative_expression = unary_expression { ("*" | "/" | "%") unary_expression }.
/// unary_expression = (("-" | "!") unary_expression) | primary_expression.
/// primary_expression = literal | identifier | "(" expression ")".
/// ```
///
/// # Examples
///
/// ## Basic Usage
///
/// ```rust
/// use cel_parser::CELParser;
/// use proc_macro2::TokenStream;
/// use std::str::FromStr;
///
/// let input = TokenStream::from_str("10").unwrap();
/// let mut parser = CELParser::new(input.into_iter());
/// let result = parser.is_expression();
/// assert!(result.is_ok());
/// ```
///
/// ## Error Formatting
///
/// ```rust
/// use cel_parser::CELParser;
/// use proc_macro2::TokenStream;
/// use std::str::FromStr;
///
/// let line = line!() + 1;
/// let source = r#"
///   10 + 20 30
/// "#; // Invalid: missing operator
/// let input = TokenStream::from_str(source).unwrap();
/// let mut parser = CELParser::new(input.into_iter());
///
/// if parser.is_expression().is_err() {
///     // Format error starting at line 1
///     if let Some(formatted_error) = parser.format_error(source, file!(), line) {
///         println!("{}", formatted_error);
///         // Output:
///         // error: Unexpected token
///         //  --> example.cel:1:8
///         //   |
///         // 1 | 10 + 20 30
///         //   |         ^^
///     }
/// }
/// ```
pub struct CELParser<I: Iterator<Item = TokenTree>> {
    tokens: Peekable<LexLexer<I>>,
    output: TokenStream,
    context: DynSegment,
    op_lookup: OpLookup,
    last_error_span: Option<Span>,
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

impl<I: Iterator<Item = TokenTree>> CELParser<I> {
    /// Returns the output token stream generated by the parser.
    ///
    /// The output contains either the successfully parsed expression or a
    /// `compile_error!` macro with the error message if parsing failed.
    pub fn get_output(&self) -> &TokenStream {
        &self.output
    }

    /// Extracts the error message from the parser's output token stream.
    ///
    /// This method searches for a `compile_error!` macro call in the output
    /// and extracts the string literal argument as the error message.
    ///
    /// # Returns
    ///
    /// Returns `Some(message)` if an error message was found, or `None` if
    /// no error was present in the output.
    pub fn extract_error_message(&self) -> Option<String> {
        let mut tokens = self.output.clone().into_iter();

        while let Some(token) = tokens.next() {
            if let TokenTree::Ident(ident) = token
                && ident == "compile_error"
                && let Some(TokenTree::Punct(punct)) = tokens.next()
                && punct.as_char() == '!'
                && let Some(TokenTree::Group(group)) = tokens.next()
                && group.delimiter() == Delimiter::Parenthesis
            {
                let mut group_tokens = group.stream().into_iter();
                if let Some(TokenTree::Literal(lit)) = group_tokens.next() {
                    // Parse the literal using syn
                    if let Ok(syn_lit) = syn::parse_str::<Lit>(&lit.to_string())
                        && let Lit::Str(parsed) = syn_lit {
                            return Some(parsed.value());
                        }
                }
            }
        }
        None
    }

    /// Formats a parser error in rustc diagnostic style with source context.
    ///
    /// Creates a formatted error message similar to Rust compiler diagnostics,
    /// including the source file location, error message, and visual indication
    /// of the error position within the source code.
    ///
    /// # Arguments
    ///
    /// * `source_code` - The original source code being parsed
    /// * `filename` - The name of the file being parsed (for display)
    /// * `start_line` - The starting line number in the original file (1-based)
    ///
    /// # Returns
    ///
    /// Returns `Some(formatted_error)` if an error was recorded, or `None` if
    /// no error occurred during parsing.
    ///
    /// # References
    ///
    /// See the [rustc diagnostic formatting guide](https://github.com/rust-lang/rustc-dev-guide/blob/master/src/diagnostics.md)
    /// for details on the error format.
    pub fn format_error(
        &self,
        source_code: &str,
        filename: &str,
        start_line: u32,
    ) -> Option<String> {
        if let Some(error_msg) = self.extract_error_message()
            && let Some(span) = self.last_error_span
        {
            return Some(self.format_rustc_style(
                &error_msg,
                span,
                source_code,
                filename,
                start_line,
            ));
        }

        None
    }

    fn format_rustc_style(
        &self,
        message: &str,
        span: Span,
        source: &str,
        filename: &str,
        start_line: u32,
    ) -> String {
        let start = span.start();
        let end = span.end();

        let lines: Vec<&str> = source.lines().collect();

        let mut output = String::new();

        // Calculate offset line numbers (start_line is 1-based)
        let error_line = start_line + (start.line as u32) - 1;
        let error_column = start.column + 1; // +1 because the column is 0-based but the error is 1-based

        // Calculate the width needed for line numbers
        // end.line is the last line within the source span (1-based)
        // start_line is the offset to get actual file line numbers
        // The maximum displayed line number will be: start_line + end.line - 1
        let max_line_num = start_line + (end.line as u32) - 1;
        let line_width = max_line_num.to_string().len();

        // Error header with red and bold "error:"
        output.push_str(&format!("{}: {}\n", "error".red().bold(), message));
        output.push_str(&format!(
            " {} {}:{}:{}\n",
            "-->".blue().bold(),
            filename.blue(),
            error_line.to_string().blue(),
            error_column.to_string().blue()
        ));
        output.push_str(&format!(
            "{:width$} {}\n",
            "",
            "|".blue().bold(),
            width = line_width
        ));

        // Show the problematic line(s)
        for line_num in start.line..=end.line {
            if let Some(line_content) = lines.get(line_num.saturating_sub(1)) {
                let display_line_num = start_line + (line_num as u32) - 1;
                output.push_str(&format!(
                    "{} {} {}\n",
                    display_line_num.to_string().blue().bold(),
                    "|".blue().bold(),
                    line_content
                ));

                // Add caret indicators
                if line_num == start.line {
                    output.push_str(&format!(
                        "{:width$} {} ",
                        "",
                        "|".blue().bold(),
                        width = line_width
                    ));

                    // Add spaces up to start column
                    output.push_str(&" ".repeat(start.column));

                    // Add carets in red
                    let caret_len = if start.line == end.line {
                        end.column.saturating_sub(start.column).max(1)
                    } else {
                        line_content
                            .len()
                            .saturating_sub(start.column.saturating_sub(1))
                    };

                    output.push_str(&"^".repeat(caret_len).red().bold().to_string());
                    output.push('\n');
                }
            }
        }

        output
    }

    /// Creates a new CEL parser with the given token iterator.
    ///
    /// # Arguments
    ///
    /// * `tokens` - An iterator over `TokenTree` items to parse
    ///
    /// # Returns
    ///
    /// A new `CELParser` instance ready to parse the tokens.
    pub fn new(tokens: I) -> Self {
        let lexer = LexLexer::new(tokens);
        let output = TokenStream::new();
        CELParser {
            tokens: lexer.peekable(),
            output,
            context: DynSegment::new::<()>(),
            op_lookup: OpLookup::new(),
            last_error_span: None,
        }
    }

    /// Returns a mutable reference to the operation lookup.
    ///
    /// This allows customization of the operations available during parsing,
    /// such as adding new scopes for custom operations or identifiers.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::CELParser;
    /// use cel_runtime::DynSegment;
    /// use proc_macro2::TokenStream;
    /// use std::any::TypeId;
    /// use std::str::FromStr;
    ///
    /// let input = TokenStream::from_str("10 + 20").unwrap();
    /// let mut parser = CELParser::new(input.into_iter());
    ///
    /// // Add a custom scope
    /// parser.op_lookup_mut().push_scope(Box::new(|name, types, segment| {
    ///     if name == "+" && types.len() == 2 && types[0] == TypeId::of::<i32>() {
    ///         segment.op2(|a: i32, b: i32| a + b + 1)?; // Custom addition
    ///         Ok(true)
    ///     } else {
    ///         Ok(false)
    ///     }
    /// }));
    /// ```
    pub fn op_lookup_mut(&mut self) -> &mut OpLookup {
        &mut self.op_lookup
    }

    fn advance(&mut self) {
        self.tokens.next();
    }

    /// Peeks at the current token without consuming it.
    /// 
    /// Returns `None` if there are no more tokens.
    fn peek_token(&mut self) -> Option<&Token> {
        self.tokens.peek()
    }

    /// Reports a parsing error by adding a `compile_error!` macro to the output.
    ///
    /// This method creates a compile-time error with the given message at the
    /// current token's span location.
    ///
    /// # Arguments
    ///
    /// * `message` - The error message to report
    ///
    /// # Returns
    ///
    /// Always returns an error to indicate parsing failure.
    pub fn report_error(&mut self, message: &str) -> anyhow::Error {
        let span = match self.peek_token() {
            Some(token) => {
                use lex_lexer::HasSpan;
                token.span()
            }
            None => proc_macro2::Span::call_site(),
        };
        // Store span for format_error to use
        self.last_error_span = Some(span);
        // Create compile_error output for proc macro usage
        self.output = quote_spanned!(span => compile_error!(#message));
        anyhow::anyhow!(message.to_string())
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

    /// Parses a complete CEL expression and returns the resulting segment.
    ///
    /// This is the main entry point for parsing. It attempts to parse a full
    /// expression and returns the dynamic segment containing the parsed result.
    ///
    /// # Errors
    ///
    /// Returns an error if the input does not contain a valid CEL expression.
    pub fn parse(&mut self) -> Result<DynSegment> {
        if !self.is_expression()? {
            return Err(self.report_error("expression expected"));
        }
        Ok(std::mem::replace(
            &mut self.context,
            DynSegment::new::<()>(),
        ))
    }

    /// `expression = or_expression <EOF>.`
    pub fn is_expression(&mut self) -> Result<bool> {
        if !self.is_or_expression()? {
            return Ok(false);
        }
        if self.peek_token().is_some() {
            return Err(self.report_error("unexpected token"));
        }
        Ok(true)
    }

    /// `or_expression = and_expression { "||" and_expression }.`
    fn is_or_expression(&mut self) -> Result<bool> {
        if self.is_and_expression()? {
            while self.is_punctuation("||") {
                if !self.is_and_expression()? {
                    return Err(self.report_error("expected and_expression"));
                }
                let types = self.context.peek_types_vec(2);
                if !types.is_empty()
                    && let Err(e) = self
                        .op_lookup
                        .lookup("||", &types, &mut self.context)
                    {
                        return Err(self.report_error(&format!("operation error: {}", e)));
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
                    return Err(self.report_error("expected comparison_expression"));
                }
                let types = self.context.peek_types_vec(2);
                if !types.is_empty()
                    && let Err(e) = self
                        .op_lookup
                        .lookup("&&", &types, &mut self.context)
                    {
                        return Err(self.report_error(&format!("operation error: {}", e)));
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
                    return Err(self.report_error("expected bitwise_or_expression"));
                }
                let types = self.context.peek_types_vec(2);
                if !types.is_empty()
                    && let Err(e) = self.op_lookup.lookup(op_name, &types, &mut self.context) {
                        return Err(self.report_error(&format!("operation error: {}", e)));
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
                    return Err(self.report_error("expected bitwise_xor_expression"));
                }
                let types = self.context.peek_types_vec(2);
                if !types.is_empty()
                    && let Err(e) = self
                        .op_lookup
                        .lookup("|", &types, &mut self.context)
                    {
                        return Err(self.report_error(&format!("operation error: {}", e)));
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
                    return Err(self.report_error("expected bitwise_and_expression"));
                }
                let types = self.context.peek_types_vec(2);
                if !types.is_empty()
                    && let Err(e) = self
                        .op_lookup
                        .lookup("^", &types, &mut self.context)
                    {
                        return Err(self.report_error(&format!("operation error: {}", e)));
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
                    return Err(self.report_error("expected bitwise_shift_expression"));
                }
                let types = self.context.peek_types_vec(2);
                if !types.is_empty()
                    && let Err(e) = self
                        .op_lookup
                        .lookup("&", &types, &mut self.context)
                    {
                        return Err(self.report_error(&format!("operation error: {}", e)));
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
                        return Err(self.report_error("expected additive_expression"));
                    }
                    let types = self.context.peek_types_vec(2);
                    if !types.is_empty()
                        && let Err(e) = self.op_lookup.lookup(op_name, &types, &mut self.context) {
                            return Err(self.report_error(&format!("operation error: {}", e)));
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
                        return Err(self.report_error("expected multiplicative_expression"));
                    }

                    // Get the top two types from the stack
                    let types = self.context.peek_types_vec(2);

                    // Apply the operation using the table (only if we have types)
                    if !types.is_empty()
                        && let Err(e) = self.op_lookup.lookup(op_name, &types, &mut self.context) {
                            return Err(self.report_error(&format!("operation error: {}", e)));
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
                        return Err(self.report_error("expected unary_expression"));
                    }

                    // Get the top two types from the stack
                    let types = self.context.peek_types_vec(2);

                    // Apply the operation using the table (only if we have types)
                    if !types.is_empty()
                        && let Err(e) = self.op_lookup.lookup(op_name, &types, &mut self.context) {
                            return Err(self.report_error(&format!("operation error: {}", e)));
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
                return Err(self.report_error("expected unary_expression"));
            }
            // Apply the unary operation (only if we have types)
            let types = self.context.peek_types_vec(1);
            if !types.is_empty()
                && let Err(e) = self.op_lookup.lookup(op_name, &types, &mut self.context) {
                    return Err(self.report_error(&format!("operation error: {}", e)));
                }
            Ok(true)
        } else {
            self.is_primary_expression()
        }
    }

    /// `primary_expression = literal | identifier | "(" expression ")".`
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
                // Look up identifier as 0-ary operation
                let ident_name = ident.to_string();
                self.advance();
                
                // Try to lookup the identifier (0-ary operation with empty type list)
                self.op_lookup.lookup(&ident_name, &[], &mut self.context)
                    .map_err(|_| {
                        self.report_error(&format!("Undefined identifier: {}", ident_name))
                    })?;
                
                Ok(true)
            }
            Some(Token::OpenDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            }) => {
                self.advance(); // consume OpenDelim
                // Recursively parse the expression inside parentheses
                if !self.is_or_expression()? {
                    return Err(self.report_error("expected expression"));
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
                    _ => Err(self.report_error("expected closing parenthesis")),
                }
            }
            _ => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::TokenStream;
    use std::str::FromStr;

    #[test]
    fn simple_expression() {
        let input = TokenStream::from_str("10").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let result = parser.parse();
        assert!(result.is_ok());
        assert_eq!(result.unwrap().call0::<i32>().unwrap(), 10);
    }

    #[test]
    fn float_literal() {
        let input = TokenStream::from_str("3.14").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let result = parser.parse();
        assert!(result.is_ok());
        let value = result.unwrap().call0::<f64>().unwrap();
        assert!((value - 3.14).abs() < 1e-10);
    }

    #[test]
    fn boolean_literal() {
        let input = TokenStream::from_str("true").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let result = parser.parse();
        assert!(result.is_ok());
        assert_eq!(result.unwrap().call0::<bool>().unwrap(), true);
    }

    #[test]
    fn string_literal() {
        let input = TokenStream::from_str(r#""hello""#).unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let result = parser.parse();
        assert!(result.is_ok());
        assert_eq!(result.unwrap().call0::<String>().unwrap(), "hello");
    }

    #[test]
    fn incomplete_expression() {
        let input = TokenStream::from_str("10 + 25 25").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression().is_err());
        assert_eq!(
            parser.output.to_string(),
            "compile_error ! (\"unexpected token\")"
        );
    }

    #[test]
    fn arithmetic_expression() {
        let input = TokenStream::from_str("10 + 20 * 30").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression().unwrap());
    }

    #[test]
    fn parenthesized_expression() {
        let input = TokenStream::from_str("(10 + 20) * 30").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression().unwrap());
    }

    #[test]
    fn complex_expression() {
        let input = TokenStream::from_str("10 + 20 * (30 - 5) / 2").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression().unwrap());
    }

    #[test]
    fn logical_expression() {
        let input = TokenStream::from_str("true && false || true").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression().unwrap());
    }

    #[test]
    fn comparison_expression() {
        let input = TokenStream::from_str("10 == 20 && 30 > 40").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression().unwrap());
    }

    #[test]
    fn bitwise_expression() {
        let input = TokenStream::from_str("1 | 2 & 3 ^ 4").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression().unwrap());
    }

    #[test]
    fn shift_expression() {
        let input = TokenStream::from_str("8 << 2 + 16 >> 1").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression().unwrap());
    }

    #[test]
    fn unary_expression() {
        let input = TokenStream::from_str("-10 + -20").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression().unwrap());
    }

    #[test]
    fn double_negation() {
        let input = TokenStream::from_str("!!true").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let result = parser.is_expression();
        assert!(result.is_ok(), "Failed to parse !!true: {:?}", result);
        assert!(result.unwrap(), "!!true returned false");
    }

    #[test]
    fn double_minus() {
        let input = TokenStream::from_str("--5").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let result = parser.is_expression();
        assert!(result.is_ok(), "Failed to parse --5: {:?}", result);
        assert!(result.unwrap(), "--5 returned false");
    }

    #[test]
    fn chained_unary_expression() {
        let input = TokenStream::from_str("!!false || !!true").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let result = parser.is_expression();
        if let Err(ref e) = result {
            eprintln!("Error: {:?}", e);
            if let Some(msg) = parser.extract_error_message() {
                eprintln!("Error message: {}", msg);
            }
        }
        assert!(result.is_ok(), "Failed to parse: {:?}", result);
        assert!(result.unwrap(), "Expression returned false");
    }

    #[test]
    fn invalid_expression() {
        let input = TokenStream::from_str("+").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let result = parser.is_expression();
        assert!(result.is_ok());
        assert!(!result.unwrap());
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
        let input = TokenStream::from_str(source).unwrap();
        let mut parser = CELParser::new(input.into_iter());

        // This should fail parsing
        assert!(parser.is_expression().is_err());

        // Test error message extraction
        let error_msg = parser.extract_error_message();
        assert!(error_msg.is_some());
        assert_eq!(error_msg.unwrap(), "unexpected token");

        // Test error formatting
        let formatted_error = parser.format_error(source, "test.cel", 1u32);
        assert!(formatted_error.is_some());

        // Strip ANSI codes for testing
        let formatted = strip_ansi_codes(&formatted_error.unwrap());
        assert!(formatted.contains("error: unexpected token"));
        assert!(formatted.contains("test.cel:1:")); // Should include line number
        assert!(formatted.contains("1 | 10 + 20 30")); // Should show the line with line number
        assert!(formatted.contains("^")); // Should have carets pointing to the error
    }

    #[test]
    fn error_formatting_with_line_offset() {
        let source = "10 + 20 30"; // Missing operator between 20 and 30
        let input = TokenStream::from_str(source).unwrap();
        let mut parser = CELParser::new(input.into_iter());

        // This should fail parsing
        assert!(parser.is_expression().is_err());

        // Test error formatting with line offset (as if expression starts at line 42)
        let formatted_error = parser.format_error(source, "large_file.rs", 42u32);
        assert!(formatted_error.is_some());

        // Strip ANSI codes for testing
        let formatted = strip_ansi_codes(&formatted_error.unwrap());
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

        let input = TokenStream::from_str(source).unwrap();
        let mut parser = CELParser::new(input.into_iter());

        // Parse should fail due to unexpected token
        assert!(parser.is_expression().is_err(), "Expected parsing to fail");
        
        // Debug: print the stored span
        if let Some(span) = parser.last_error_span {
            eprintln!("DEBUG: span.start().line = {}, span.start().column = {}", 
                span.start().line, span.start().column);
        }
        
        // Format the error
        if let Some(formatted_error) = parser.format_error(source, file!(), line) {
            println!("{}", formatted_error);
            
            // Strip ANSI codes for testing
            let formatted = strip_ansi_codes(&formatted_error);
            
            // The source string has 3 lines:
            // Line 0: empty
            // Line 1: empty  
            // Line 2: "         10 + 20  30 // Unexpected token"
            // So the error should be on line + 2
            let expected_line = line + 2;
            
            assert!(formatted.contains("error: unexpected token"), 
                "Should contain error message, got: {}", formatted);
            assert!(formatted.contains(&format!("{}:", expected_line)),
                "Should show error on line {}, got: {}", expected_line, formatted);
            assert!(formatted.contains("30"),
                "Should show the source line with '30', got: {}", formatted);
            assert!(formatted.contains("^"),
                "Should have carets pointing to error, got: {}", formatted);
        } else {
            panic!("format_error returned None");
        }
    }

    #[test]
    fn test_addition_execution() -> Result<()> {
        let input = TokenStream::from_str("10 + 20").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let mut segment = parser.parse()?;
        let result = segment.call0::<i32>()?;
        assert_eq!(result, 30);
        Ok(())
    }

    #[test]
    fn test_multiplication_execution() -> Result<()> {
        let input = TokenStream::from_str("3 * 7").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let mut segment = parser.parse()?;
        let result = segment.call0::<i32>()?;
        assert_eq!(result, 21);
        Ok(())
    }

    #[test]
    fn test_complex_arithmetic_execution() -> Result<()> {
        let input = TokenStream::from_str("10 + 20 * 3").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let mut segment = parser.parse()?;
        let result = segment.call0::<i32>()?;
        assert_eq!(result, 70); // 10 + (20 * 3) = 10 + 60 = 70
        Ok(())
    }

    #[test]
    fn test_parenthesized_arithmetic_execution() -> Result<()> {
        let input = TokenStream::from_str("(10 + 20) * 3").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let mut segment = parser.parse()?;
        let result = segment.call0::<i32>()?;
        assert_eq!(result, 90); // (10 + 20) * 3 = 30 * 3 = 90
        Ok(())
    }

    #[test]
    fn test_comparison_execution() -> Result<()> {
        let input = TokenStream::from_str("10 < 20").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let mut segment = parser.parse()?;
        let result = segment.call0::<bool>()?;
        assert_eq!(result, true);
        Ok(())
    }

    #[test]
    fn test_logical_and_execution() -> Result<()> {
        let input = TokenStream::from_str("true && false").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let mut segment = parser.parse()?;
        let result = segment.call0::<bool>()?;
        assert_eq!(result, false);
        Ok(())
    }

    #[test]
    fn test_unary_negation_execution() -> Result<()> {
        let input = TokenStream::from_str("-42").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let mut segment = parser.parse()?;
        let result = segment.call0::<i32>()?;
        assert_eq!(result, -42);
        Ok(())
    }

    #[test]
    fn test_logical_not_execution() -> Result<()> {
        let input = TokenStream::from_str("!true").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let mut segment = parser.parse()?;
        let result = segment.call0::<bool>()?;
        assert_eq!(result, false);
        Ok(())
    }

    #[test]
    fn test_u32_addition_execution() -> Result<()> {
        let input = TokenStream::from_str("10u32 + 20u32").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let mut segment = parser.parse()?;
        let result = segment.call0::<u32>()?;
        assert_eq!(result, 30);
        Ok(())
    }

    #[test]
    fn test_identifier_with_scope() -> Result<()> {
        let input = TokenStream::from_str("x + y").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        
        // Add a scope that provides variable values
        parser.op_lookup_mut().push_scope(Box::new(|name, types, segment| {
            // 0-ary lookup means identifier
            if types.is_empty() {
                match name {
                    "x" => {
                        segment.op0(|| 10i32);
                        Ok(true)
                    }
                    "y" => {
                        segment.op0(|| 20i32);
                        Ok(true)
                    }
                    _ => Ok(false)
                }
            } else {
                Ok(false)
            }
        }));
        
        let mut segment = parser.parse()?;
        let result = segment.call0::<i32>()?;
        assert_eq!(result, 30);
        Ok(())
    }

    #[test]
    fn test_undefined_identifier_error() {
        let input = TokenStream::from_str("undefined_var + 10").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let result = parser.parse();
        
        assert!(result.is_err());
        if let Err(e) = result {
            let error_msg = format!("{:?}", e);
            assert!(error_msg.contains("Undefined identifier: undefined_var"), 
                "Error message should contain 'Undefined identifier: undefined_var', got: {}", error_msg);
        }
    }

    #[test]
    fn test_undefined_identifier_error_formatting() {
        let input = "undefined_var + 10";
        let token_stream = TokenStream::from_str(input).unwrap();
        let mut parser = CELParser::new(token_stream.into_iter());
        let result = parser.parse();
        
        assert!(result.is_err());
        if let Err(_) = result {
            // Test that format_error works correctly
            if let Some(formatted_error) = parser.format_error(input, "test.cel", 1) {
                assert!(formatted_error.contains("Undefined identifier"));
                assert!(formatted_error.contains("undefined_var"));
                assert!(formatted_error.contains("test.cel"));
            }
        }
    }

    #[test]
    fn test_float_arithmetic_execution() -> Result<()> {
        let input = TokenStream::from_str("3.5 * 2.0").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let mut segment = parser.parse()?;
        let result = segment.call0::<f64>()?;
        assert_eq!(result, 7.0);
        Ok(())
    }
}
