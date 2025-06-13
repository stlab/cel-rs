use std::iter::Peekable;

use owo_colors::OwoColorize;
use proc_macro2::{Delimiter, Spacing, Span, TokenStream, TokenTree};
use quote::quote_spanned;

/// A recursive descent parser for expressions.
///
/// Grammar:
/// ```text
/// expression = or_expression <EOF>.
/// or_expression = and_expression { "||" and_expression }.
/// and_expression = comparison_expression { "&&" comparison_expression }.
/// comparison_expression = bitwise_or_expression [ ("==" | "!=" | "<" | ">" | "<=" | ">=") bitwise_or_expression ].
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
/// let input = TokenStream::from_str("10 + 20").unwrap();
/// let mut parser = CELParser::new(input.into_iter());
/// assert!(parser.is_expression());
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
/// if !parser.is_expression() {
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
    tokens: Peekable<I>,
    output: TokenStream,
}

impl<I: Iterator<Item = TokenTree> + Clone> CELParser<I> {
    pub fn extract_error_message(&self) -> Option<String> {
        let output_str = self.output.to_string();

        // Look for compile_error ! ("message") - note the spaces
        if let Some(start) = output_str.find("compile_error ! (\"") {
            let start = start + "compile_error ! (\"".len();
            if let Some(end) = output_str[start..].find("\")") {
                return Some(output_str[start..start + end].to_string());
            }
        }
        None
    }

    /// https://github.com/rust-lang/rustc-dev-guide/blob/master/src/diagnostics.md
    pub fn format_error(
        &self,
        source_code: &str,
        filename: &str,
        start_line: u32,
    ) -> Option<String> {
        if let Some(error_msg) = self.extract_error_message() {
            if let Some(span) = self.get_error_span() {
                return Some(self.format_rustc_style(
                    &error_msg,
                    span,
                    source_code,
                    filename,
                    start_line,
                ));
            }
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

    pub fn new(tokens: I) -> Self {
        let output = TokenStream::new();
        CELParser {
            tokens: tokens.peekable(),
            output,
        }
    }

    pub fn get_output(&self) -> &TokenStream {
        &self.output
    }

    fn advance(&mut self) {
        self.tokens.next();
    }

    pub fn report_error(&mut self, message: &str) -> bool {
        let span = self
            .tokens
            .peek()
            .map_or_else(proc_macro2::Span::call_site, |token| token.span());
        self.output = quote_spanned!(span => compile_error!(#message));
        false
    }

    fn is_one_of_punc(token: Option<&TokenTree>, sequence: &[char]) -> bool {
        match token {
            Some(TokenTree::Punct(punct)) => sequence.contains(&punct.as_char()),
            _ => false,
        }
    }

    fn is_punctuation(&mut self, string: &str) -> bool {
        let mut tmp = self.tokens.clone();
        let mut spacing = Spacing::Joint;
        for c in string.chars() {
            if spacing == Spacing::Alone {
                return false;
            }
            match tmp.peek() {
                Some(TokenTree::Punct(punct)) => {
                    if punct.as_char() != c {
                        return false;
                    }
                    spacing = punct.spacing();
                    tmp.next();
                }
                _ => return false,
            }
        }
        // filter false positives for compound operators
        if spacing == Spacing::Joint && string.len() == 1 {
            let compound_chars = [
                ('&', &['&'][..]),
                ('|', &['|'][..]),
                ('<', &['<', '='][..]),
                ('>', &['>', '='][..]),
            ];
            let c = string.chars().next().unwrap(); // safe since string.len() == 1

            if let Some((_, next_chars)) = compound_chars.iter().find(|(ch, _)| *ch == c) {
                if Self::is_one_of_punc(tmp.peek(), next_chars) {
                    return false;
                }
            }
        }
        self.tokens = tmp;
        true
    }

    fn is_one_of_punctuation(&mut self, sequence: &[&str]) -> bool {
        for s in sequence {
            if self.is_punctuation(s) {
                return true;
            }
        }
        false
    }

    /// `expression = or_expression <EOF>.`
    pub fn is_expression(&mut self) -> bool {
        if !self.is_or_expression() {
            return false;
        }
        if self.tokens.peek().is_some() {
            return self.report_error("unexpected token");
        }
        true
    }

    /// `or_expression = and_expression { "||" and_expression }.`
    fn is_or_expression(&mut self) -> bool {
        if self.is_and_expression() {
            while self.is_one_of_punctuation(&["||"]) {
                if !self.is_and_expression() {
                    return self.report_error("expected and_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// `and_expression = comparison_expression { "&&" comparison_expression }.`
    fn is_and_expression(&mut self) -> bool {
        if self.is_comparison_expression() {
            while self.is_one_of_punctuation(&["&&"]) {
                if !self.is_comparison_expression() {
                    return self.report_error("expected comparison_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// `comparison_expression = bitwise_or_expression [ ("==" | "!=" | "<" | ">" | "<=" | ">=") bitwise_or_expression ].`
    fn is_comparison_expression(&mut self) -> bool {
        if self.is_bitwise_or_expression() {
            if self.is_one_of_punctuation(&["==", "!=", "<", ">", "<=", ">="])
                && !self.is_bitwise_or_expression()
            {
                return self.report_error("expected bitwise_or_expression");
            }
            true
        } else {
            false
        }
    }

    /// `bitwise_or_expression = bitwise_xor_expression { "|" bitwise_xor_expression }.`
    fn is_bitwise_or_expression(&mut self) -> bool {
        if self.is_bitwise_xor_expression() {
            while self.is_one_of_punctuation(&["|"]) {
                if !self.is_bitwise_xor_expression() {
                    return self.report_error("expected bitwise_xor_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// `bitwise_xor_expression = bitwise_and_expression { "^" bitwise_and_expression }.`
    fn is_bitwise_xor_expression(&mut self) -> bool {
        if self.is_bitwise_and_expression() {
            while self.is_one_of_punctuation(&["^"]) {
                if !self.is_bitwise_and_expression() {
                    return self.report_error("expected bitwise_and_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// `bitwise_and_expression = bitwise_shift_expression { "&" bitwise_shift_expression }.`
    fn is_bitwise_and_expression(&mut self) -> bool {
        if self.is_bitwise_shift_expression() {
            while self.is_one_of_punctuation(&["&"]) {
                if !self.is_bitwise_shift_expression() {
                    return self.report_error("expected bitwise_shift_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// `bitwise_shift_expression = additive_expression { ("<<" | ">>") additive_expression }.`
    fn is_bitwise_shift_expression(&mut self) -> bool {
        if self.is_additive_expression() {
            while self.is_one_of_punctuation(&["<<", ">>"]) {
                if !self.is_additive_expression() {
                    return self.report_error("expected additive_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// `additive_expression = multiplicative_expression { ("+" | "-") multiplicative_expression }.`
    fn is_additive_expression(&mut self) -> bool {
        if self.is_multiplicative_expression() {
            while self.is_one_of_punctuation(&["+", "-"]) {
                if !self.is_multiplicative_expression() {
                    return self.report_error("expected multiplicative_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// `multiplicative_expression = unary_expression { ("*" | "/" | "%") unary_expression }.`
    fn is_multiplicative_expression(&mut self) -> bool {
        if self.is_unary_expression() {
            while self.is_one_of_punctuation(&["*", "/", "%"]) {
                if !self.is_unary_expression() {
                    return self.report_error("expected unary_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// `unary_expression = (("-" | "!") unary_expression) | primary_expression.`
    fn is_unary_expression(&mut self) -> bool {
        if self.is_one_of_punctuation(&["-", "!"]) {
            if !self.is_unary_expression() {
                return self.report_error("expected unary_expression");
            }
            true
        } else {
            self.is_primary_expression()
        }
    }

    /// `primary_expression = literal | identifier | "(" expression ")".`
    fn is_primary_expression(&mut self) -> bool {
        match self.tokens.peek() {
            Some(TokenTree::Literal(_)) => {
                self.advance();
                true
            }
            Some(TokenTree::Ident(_)) => {
                self.advance();
                true
            }
            Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis => {
                let mut parser = CELParser::new(group.stream().into_iter());
                if parser.is_expression() {
                    self.advance();
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::TokenStream;
    use std::str::FromStr;

    #[test]
    fn test_simple_expression() {
        let input = TokenStream::from_str("10").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression());
    }

    #[test]
    fn test_incomplete_expression() {
        let input = TokenStream::from_str("10 + 25 25").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(!parser.is_expression());
        assert_eq!(
            parser.output.to_string(),
            "compile_error ! (\"unexpected token\")"
        );
    }

    #[test]
    fn test_arithmetic_expression() {
        let input = TokenStream::from_str("10 + 20 * 30").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression());
    }

    #[test]
    fn test_parenthesized_expression() {
        let input = TokenStream::from_str("(10 + 20) * 30").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression());
    }

    #[test]
    fn test_complex_expression() {
        let input = TokenStream::from_str("10 + 20 * (30 - 5) / 2").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression());
    }

    #[test]
    fn test_logical_expression() {
        let input = TokenStream::from_str("a && b || c").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression());
    }

    #[test]
    fn test_comparison_expression() {
        let input = TokenStream::from_str("a == b && c > d").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression());
    }

    #[test]
    fn test_bitwise_expression() {
        let input = TokenStream::from_str("a | b & c ^ d").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression());
    }

    #[test]
    fn test_shift_expression() {
        let input = TokenStream::from_str("a << 2 + b >> 1").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression());
    }

    #[test]
    fn test_unary_expression() {
        let input = TokenStream::from_str("-a + !b").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression());
    }

    #[test]
    fn test_chained_unary_expression() {
        let input = TokenStream::from_str("!!a + --b").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(parser.is_expression());
    }

    #[test]
    fn test_invalid_expression() {
        let input = TokenStream::from_str("+").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(!parser.is_expression());
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
    fn test_error_formatting() {
        let source = "10 + 20 30"; // Missing operator between 20 and 30
        let input = TokenStream::from_str(source).unwrap();
        let mut parser = CELParser::new(input.into_iter());

        // This should fail parsing
        assert!(!parser.is_expression());

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
    fn test_error_formatting_with_line_offset() {
        let source = "a + b c"; // Missing operator between b and c
        let input = TokenStream::from_str(source).unwrap();
        let mut parser = CELParser::new(input.into_iter());

        // This should fail parsing
        assert!(!parser.is_expression());

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

            10 + 20 * 30 // Unexpected token

        "#;

        let input = TokenStream::from_str(source).unwrap();
        let mut parser = CELParser::new(input.into_iter());

        if !parser.is_expression() {
            if let Some(formatted_error) = parser.format_error(source, file!(), line) {
                println!("{}", formatted_error);
            }
        }
    }
}
