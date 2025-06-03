use std::iter::Peekable;

use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};
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
    tokens: Peekable<I>,
    output: TokenStream,
}

impl<I: Iterator<Item = TokenTree> + Clone> CELParser<I> {
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

    /// expression = or_expression <EOF>.
    pub fn is_expression(&mut self) -> bool {
        if !self.is_or_expression() {
            return false;
        }
        if self.tokens.peek().is_some() {
            return self.report_error("Unexpected token");
        }
        true
    }

    /// or_expression = and_expression { "||" and_expression }.
    fn is_or_expression(&mut self) -> bool {
        if self.is_and_expression() {
            while self.is_one_of_punctuation(&["||"]) {
                if !self.is_and_expression() {
                    return self.report_error("Expected and_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// and_expression = comparison_expression { "&&" comparison_expression }.
    fn is_and_expression(&mut self) -> bool {
        if self.is_comparison_expression() {
            while self.is_one_of_punctuation(&["&&"]) {
                if !self.is_comparison_expression() {
                    return self.report_error("Expected comparison_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// comparison_expression = bitwise_or_expression [ ("==" | "!=" | "<" | ">" | "<=" | ">=") bitwise_or_expression ].
    fn is_comparison_expression(&mut self) -> bool {
        if self.is_bitwise_or_expression() {
            if self.is_one_of_punctuation(&["==", "!=", "<", ">", "<=", ">="])
                && !self.is_bitwise_or_expression()
            {
                return self.report_error("Expected bitwise_or_expression");
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
                    return self.report_error("Expected bitwise_xor_expression");
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
                    return self.report_error("Expected bitwise_and_expression");
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
                    return self.report_error("Expected bitwise_shift_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// bitwise_shift_expression = additive_expression { ("<<" | ">>") additive_expression }.
    fn is_bitwise_shift_expression(&mut self) -> bool {
        if self.is_additive_expression() {
            while self.is_one_of_punctuation(&["<<", ">>"]) {
                if !self.is_additive_expression() {
                    return self.report_error("Expected additive_expression");
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
                    return self.report_error("Expected multiplicative_expression");
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
                    return self.report_error("Expected unary_expression");
                }
            }
            true
        } else {
            false
        }
    }

    /// unary_expression = (("-" | "!") unary_expression) | primary_expression.
    fn is_unary_expression(&mut self) -> bool {
        if self.is_one_of_punctuation(&["-", "!"]) {
            if !self.is_unary_expression() {
                return self.report_error("Expected unary_expression");
            }
            true
        } else {
            self.is_primary_expression()
        }
    }

    /// primary_expression = literal | identifier | "(" expression ")".
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
        let input = TokenStream::from_str("10 +").unwrap();
        let mut parser = CELParser::new(input.into_iter());
        assert!(!parser.is_expression());
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
}
