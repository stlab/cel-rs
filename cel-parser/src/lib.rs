#![warn(missing_docs)]

//! A recursive descent parser for CEL (Common Expression Language) expressions.
//!
//! This crate provides a parser that can parse CEL expressions into token streams
//! suitable for use in procedural macros. The parser follows the CEL grammar
//! specification and provides detailed error reporting with source location information.
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
use lex_lexer::{LexLexer, Literal as CelLiteral, Token};

use anyhow::Result;
use cel_runtime::DynSegment;
use owo_colors::OwoColorize;
use proc_macro2::{Delimiter, Ident, Literal, Span, TokenStream, TokenTree};
use quote::quote_spanned;
use std::iter::Peekable;

fn push_literal(output: &mut DynSegment, lit: CelLiteral) {
    match lit {
        CelLiteral::Integer {
            parsed: integer, ..
        } => {
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
        CelLiteral::Float { parsed: float, .. } => {
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
        CelLiteral::String { parsed: string, .. } => {
            // Store the string value (without quotes)
            output.just(string.value());
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
    last_error_span: Option<Span>,
}
pub enum PrimaryExpression {
    Literal(Literal),
    Ident(Ident),
}

pub enum Probe<T> {
    NoMatch,
    Match,
    Value(T),
}

pub type PrimaryProbe = Probe<PrimaryExpression>;

impl<I: Iterator<Item = TokenTree>> CELParser<I> {
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
                    // Clean extraction using syn
                    if let Ok(cel_lit) = CelLiteral::from_proc_macro_literal(lit) {
                        if let CelLiteral::String { parsed, .. } = cel_lit {
                            return Some(parsed.value());
                        }
                    }
                }
            }
        }
        None
    }

    /// <https://github.com/rust-lang/rustc-dev-guide/blob/master/src/diagnostics.md>
    pub fn format_error(
        &self,
        source_code: &str,
        filename: &str,
        start_line: u32,
    ) -> Option<String> {
        if let Some(error_msg) = self.extract_error_message()
            && let Some(span) = self.get_error_span()
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

    fn get_error_span(&self) -> Option<Span> {
        // The compile_error! TokenStream structure is:
        // TokenTree::Ident("compile_error") - with the span we want
        // TokenTree::Punct('!')
        // TokenTree::Group(...) - containing the message, also with the span

        let mut tokens = self.output.clone().into_iter();

        // Look for the first token (should be "compile_error" ident)
        if let Some(first_token) = tokens.next() {
            match first_token {
                TokenTree::Ident(ident) if ident == "compile_error" => {
                    return Some(ident.span());
                }
                _ => {
                    // Fallback: try to get span from any token in the stream
                    return Some(first_token.span());
                }
            }
        }
        None
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
            last_error_span: None,
        }
    }

    fn advance(&mut self) -> Result<()> {
        match self.tokens.next() {
            Some(Ok(_)) => Ok(()),
            Some(Err(e)) => Err(e),
            None => Ok(()),
        }
    }

    /// Peek at the current token, handling any lexer errors.
    /// Returns an error if the lexer encountered an error.
    /// Returns Ok(None) if there are no more tokens.
    fn peek_token(&mut self) -> Result<Option<&Token>> {
        // Check if peek returns an error without consuming
        let has_error = matches!(self.tokens.peek(), Some(Err(_)));
        
        if has_error {
            // Now we can consume the error
            if let Some(Err(e)) = self.tokens.next() {
                return Err(e);
            } else {
                unreachable!()
            }
        }
        
        // Safe to peek now - we know it's either Some(Ok) or None
        match self.tokens.peek() {
            Some(Ok(token)) => Ok(Some(token)),
            None => Ok(None),
            Some(Err(_)) => unreachable!("Error should have been handled above"),
        }
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
            Ok(Some(token)) => {
                use lex_lexer::HasSpan;
                let span = token.span();
                self.last_error_span = Some(span);
                span
            }
            _ => self
                .last_error_span
                .unwrap_or_else(proc_macro2::Span::call_site),
        };
        self.output = quote_spanned!(span => compile_error!(#message));
        anyhow::anyhow!(message.to_string())
    }

    fn is_punctuation(&mut self, target: &str) -> Result<bool> {
        // Simply check if the current token is a Punct with the target operator
        match self.peek_token()? {
            Some(Token::Punct { op, .. }) if op == target => {
                self.advance()?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn is_one_of_punctuation(&mut self, sequence: &[&str]) -> Result<bool> {
        for s in sequence {
            if self.is_punctuation(s)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

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
        if self.peek_token()?.is_some() {
            return Err(self.report_error("unexpected token"));
        }
        Ok(true)
    }

    /// `or_expression = and_expression { "||" and_expression }.`
    fn is_or_expression(&mut self) -> Result<bool> {
        if self.is_and_expression()? {
            while self.is_one_of_punctuation(&["||"])? {
                if !self.is_and_expression()? {
                    return Err(self.report_error("expected and_expression"));
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
            while self.is_one_of_punctuation(&["&&"])? {
                if !self.is_comparison_expression()? {
                    return Err(self.report_error("expected comparison_expression"));
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
            // Check longer operators first to avoid matching "<" when we have "<="
            if self.is_one_of_punctuation(&["==", "!=", "<=", ">=", "<", ">"])? 
                && !self.is_bitwise_or_expression()? 
            {
                return Err(self.report_error("expected bitwise_or_expression"));
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `bitwise_or_expression = bitwise_xor_expression { "|" bitwise_xor_expression }.`
    fn is_bitwise_or_expression(&mut self) -> Result<bool> {
        if self.is_bitwise_xor_expression()? {
            while self.is_one_of_punctuation(&["|"])? {
                if !self.is_bitwise_xor_expression()? {
                    return Err(self.report_error("expected bitwise_xor_expression"));
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
            while self.is_one_of_punctuation(&["^"])? {
                if !self.is_bitwise_and_expression()? {
                    return Err(self.report_error("expected bitwise_and_expression"));
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
            while self.is_one_of_punctuation(&["&"])? {
                if !self.is_bitwise_shift_expression()? {
                    return Err(self.report_error("expected bitwise_shift_expression"));
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
            while self.is_one_of_punctuation(&["<<", ">>"])? {
                if !self.is_additive_expression()? {
                    return Err(self.report_error("expected additive_expression"));
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
            while self.is_one_of_punctuation(&["+", "-"])? {
                if !self.is_multiplicative_expression()? {
                    return Err(self.report_error("expected multiplicative_expression"));
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
            while self.is_one_of_punctuation(&["*", "/", "%"])? {
                if !self.is_unary_expression()? {
                    return Err(self.report_error("expected unary_expression"));
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `unary_expression = (("-" | "!") unary_expression) | primary_expression.`
    fn is_unary_expression(&mut self) -> Result<bool> {
        if self.is_one_of_punctuation(&["-", "!"])? {
            if !self.is_unary_expression()? {
                return Err(self.report_error("expected unary_expression"));
            }
            Ok(true)
        } else {
            self.is_primary_expression()
        }
    }

    /// `primary_expression = literal | identifier | "(" expression ")".`
    fn is_primary_expression(&mut self) -> Result<bool> {
        match self.peek_token()? {
            Some(Token::Literal(lit)) => {
                // Clone the literal data we need before advancing
                let lit_clone = match lit {
                    CelLiteral::Integer { parsed, .. } => CelLiteral::Integer { 
                        parsed: parsed.clone(), 
                        span: parsed.span() 
                    },
                    CelLiteral::String { parsed, .. } => CelLiteral::String {
                        parsed: parsed.clone(),
                        span: parsed.span()
                    },
                    CelLiteral::Float { parsed, .. } => CelLiteral::Float {
                        parsed: parsed.clone(),
                        span: parsed.span()
                    },
                };
                self.advance()?;
                // Push the literal to the context
                push_literal(&mut self.context, lit_clone);
                Ok(true)
            }
            Some(Token::Identifier { ident, .. }) => {
                // Check if this is a boolean identifier (true/false)
                let ident_str = ident.to_string();
                if ident_str == "true" {
                    self.advance()?;
                    self.context.just(true);
                    Ok(true)
                } else if ident_str == "false" {
                    self.advance()?;
                    self.context.just(false);
                    Ok(true)
                } else {
                    // Regular identifier - just advance for now
                    self.advance()?;
                    Ok(true)
                }
            }
            Some(Token::OpenDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            }) => {
                self.advance()?; // consume OpenDelim
                // Recursively parse the expression inside parentheses
                if !self.is_or_expression()? {
                    return Err(self.report_error("expected expression"));
                }
                // Expect CloseDelim
                match self.peek_token()? {
                    Some(Token::CloseDelim {
                        delimiter: Delimiter::Parenthesis,
                        ..
                    }) => {
                        self.advance()?; // consume CloseDelim
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
        let input = TokenStream::from_str("a && b || c").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression().unwrap());
    }

    #[test]
    fn comparison_expression() {
        let input = TokenStream::from_str("a == b && c > d").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression().unwrap());
    }

    #[test]
    fn bitwise_expression() {
        let input = TokenStream::from_str("a | b & c ^ d").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression().unwrap());
    }

    #[test]
    fn shift_expression() {
        let input = TokenStream::from_str("a << 2 + b >> 1").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression().unwrap());
    }

    #[test]
    fn unary_expression() {
        let input = TokenStream::from_str("-a + !b").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression().unwrap());
    }

    #[test]
    fn double_negation() {
        let input = TokenStream::from_str("!!a").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let result = parser.is_expression();
        assert!(result.is_ok(), "Failed to parse !!a: {:?}", result);
        assert!(result.unwrap(), "!!a returned false");
    }

    #[test]
    fn double_minus() {
        let input = TokenStream::from_str("--b").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        let result = parser.is_expression();
        assert!(result.is_ok(), "Failed to parse --b: {:?}", result);
        assert!(result.unwrap(), "--b returned false");
    }

    #[test]
    fn chained_unary_expression() {
        let input = TokenStream::from_str("!!a + --b").unwrap();
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
        let source = "a + b c"; // Missing operator between b and c
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
        assert!(formatted.contains("42 | a + b c")); // Should show the line with offset line number
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

        if parser.is_expression().is_err() {
            if let Some(formatted_error) = parser.format_error(source, file!(), line) {
                println!("{}", formatted_error);
            }
        }
    }
}
