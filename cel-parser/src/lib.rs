use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};
use quote::quote_spanned;

/// A recursive descent parser for arithmetic expressions.
///
/// For our arithmetic expression grammar:
/// ```text
/// expr = term {("+" | "-") term}.
/// term = factor {("*" | "/") factor}.
/// factor = NUMBER | IDENTIFIER | "(" expr ")".
/// ```
///
/// Where:
/// - NUMBER is any numeric literal
/// - IDENTIFIER is any valid Rust identifier
/// - Operators have standard precedence: * and / bind tighter than + and -
/// - Parentheses can be used to override precedence
///
/// # Example
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
pub struct CELParser<I: Iterator<Item = TokenTree>> {
    tokens: I,
    current: Option<TokenTree>,
    output: TokenStream,
}

impl<I: Iterator<Item = TokenTree> + Clone> CELParser<I> {
    pub fn new(mut tokens: I) -> Self {
        let current = tokens.next();
        let output = TokenStream::new();
        CELParser {
            tokens,
            current,
            output,
        }
    }

    pub fn get_output(&self) -> &TokenStream {
        &self.output
    }

    fn advance(&mut self) {
        self.current = self.tokens.next();
    }

    pub fn report_error(&mut self, message: &str) {
        let span = self
            .current
            .as_ref()
            .map_or_else(|| proc_macro2::Span::call_site(), |token| token.span());
        self.output = quote_spanned!(span => compile_error!(#message));
    }

    fn is_punctuation(&mut self, string: &str) -> bool {
        let mut tmp = self.tokens.clone();
        let mut current = self.current.clone();
        let mut spacing = Spacing::Joint;
        for c in string.chars() {
            if spacing == Spacing::Alone {
                return false;
            }
            match &current {
                Some(TokenTree::Punct(punct)) => {
                    if punct.as_char() != c {
                        return false;
                    }
                    spacing = punct.spacing();
                    current = tmp.next();
                }
                _ => return false,
            }
        }
        self.tokens = tmp;
        self.current = current;
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

    /// expression = or_expression.
    pub fn is_expression(&mut self) -> bool {
        self.is_or_expression()
    }

    /// or_expression = and_expression { "||" and_expression }.
    fn is_or_expression(&mut self) -> bool {
        if self.is_and_expression() {
            while self.is_one_of_punctuation(&["||"]) {
                if !self.is_and_expression() {
                    self.report_error("Expected and_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// and_expression = equality_expression { "&&" equality_expression }.
    fn is_and_expression(&mut self) -> bool {
        if self.is_comparison_expression() {
            while self.is_one_of_punctuation(&["&&"]) {
                if !self.is_comparison_expression() {
                    self.report_error("Expected comparison_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// comparison_expression = bitwise_or_expression { ("==" | "!=" | "<" | ">" | "<=" | ">=") bitwise_or_expression }.
    fn is_comparison_expression(&mut self) -> bool {
        if self.is_bitwise_or_expression() {
            while self.is_one_of_punctuation(&["==", "!=", "<", ">", "<=", ">="]) {
                if !self.is_bitwise_or_expression() {
                    self.report_error("Expected bitwise_or_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// bitwise_or_expression = bitwise_xor_expression { "|" bitwise_xor_expression }.
    fn is_bitwise_or_expression(&mut self) -> bool {
        if self.is_bitwise_xor_expression() {
            while self.is_one_of_punctuation(&["|"]) {
                if !self.is_bitwise_xor_expression() {
                    self.report_error("Expected bitwise_xor_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// bitwise_xor_expression = bitwise_and_expression { "^" bitwise_and_expression }.
    fn is_bitwise_xor_expression(&mut self) -> bool {
        if self.is_bitwise_and_expression() {
            while self.is_one_of_punctuation(&["^"]) {
                if !self.is_bitwise_and_expression() {
                    self.report_error("Expected bitwise_and_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// bitwise_and_expression = bitwise_shift_expression { "&" bitwise_shift_expression }.
    fn is_bitwise_and_expression(&mut self) -> bool {
        if self.is_bitwise_shift_expression() {
            while self.is_one_of_punctuation(&["&"]) {
                if !self.is_bitwise_shift_expression() {
                    self.report_error("Expected bitwise_shift_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// bitwise_shift_expression = add_expression { ("<<" | ">>") add_expression }.
    fn is_bitwise_shift_expression(&mut self) -> bool {
        if self.is_additive_expression() {
            while self.is_one_of_punctuation(&["<<", ">>"]) {
                if !self.is_additive_expression() {
                    self.report_error("Expected additive_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// additive_expression = multiplicative_expression { ("+" | "-") multiplicative_expression }.
    fn is_additive_expression(&mut self) -> bool {
        if self.is_multiplicative_expression() {
            while self.is_one_of_punctuation(&["+", "-"]) {
                if !self.is_multiplicative_expression() {
                    self.report_error("Expected multiplicative_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// multiplicative_expression = unary_expression { ("*" | "/" | "%") unary_expression }.
    fn is_multiplicative_expression(&mut self) -> bool {
        if self.is_unary_expression() {
            while self.is_one_of_punctuation(&["*", "/", "%"]) {
                if !self.is_unary_expression() {
                    self.report_error("Expected unary_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// unary_expression = ("-" | "!") primary_expression.
    fn is_unary_expression(&mut self) -> bool {
        if self.is_one_of_punctuation(&["-", "!"]) {
            if !self.is_primary_expression() {
                self.report_error("Expected primary_expression");
            }
            true
        } else {
            self.is_primary_expression()
        }
    }

    /// primary_expression = literal | identifier | "(" expression ")".
    fn is_primary_expression(&mut self) -> bool {
        match &self.current {
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
    fn test_invalid_expression() {
        let input = TokenStream::from_str("+").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(!parser.is_expression());
    }
}
