//! A `lex-lexer` is a lexer taking a lex token stream and returning a token stream of a different
//! type. Initially this is used to convert the TokenTree from Rust's proc_macro into a higher level
//! token stream. The goal, however, is to be able to specify with a grammar how to process a token
//! stream.

use proc_macro2::{Delimiter, Ident, Spacing, Span, TokenTree};
use syn::Lit;

/// A trait for token types that provides access to span information for error reporting.
///
/// This trait is minimal by design - token type discrimination is done through pattern
/// matching on enum variants, not through trait methods. This keeps the trait simple
/// and allows different token types to have their own specific fields and methods.
pub trait HasSpan {
    /// Get the span for error reporting.
    fn span(&self) -> Span;
}

/// A group iterator with its associated close delimiter information.
struct GroupLevel {
    iter: proc_macro2::token_stream::IntoIter,
    delimiter: Delimiter,
    span: Span,
}

/// A lexer that transforms a `TokenTree` stream into a flattened `Token` stream.
///
/// Groups are flattened into OpenDelim and CloseDelim tokens, and literals are
/// eagerly discriminated into specific types. Flattening is lazy - group iterators
/// are pushed onto a stack and processed one token at a time.
///
/// Multi-character operators are combined at this level (e.g., `&` + `&` -> `&&`).
pub struct LexLexer<I: Iterator<Item = TokenTree>> {
    input: I,
    /// Stack of iterators for nested groups - allows lazy flattening.
    /// Each entry tracks the iterator and its close delimiter info.
    group_stack: Vec<GroupLevel>,
    /// Pending close delimiter to emit when we've just exhausted a group iterator.
    pending_close: Option<(Delimiter, Span)>,
    /// Pending token that was consumed while looking ahead.
    pending_token: Option<TokenTree>,
}

impl<I: Iterator<Item = TokenTree>> LexLexer<I> {
    /// Creates a new lexer from a token tree iterator.
    ///
    /// # Arguments
    ///
    /// * `input` - An iterator over `TokenTree` items to be lexed into `Token`s
    pub fn new(input: I) -> Self {
        Self {
            input,
            group_stack: Vec::new(),
            pending_close: None,
            pending_token: None,
        }
    }

    /// Converts a single TokenTree into a Token (except Punct and Group which are handled specially).
    ///
    /// This handles Literal and Identifier tokens. Boolean identifiers (`true`, `false`) are
    /// converted to Boolean literals. Punct tokens need special handling for combining
    /// multi-char operators, and Groups are handled by the iterator.
    fn convert_token(token: TokenTree) -> Result<Token, anyhow::Error> {
        match token {
            TokenTree::Literal(lit) => {
                // Wrap in TokenTree and convert to TokenStream to preserve span information
                let token_stream: proc_macro2::TokenStream = TokenTree::Literal(lit).into();
                let syn_lit: Lit = syn::parse2(token_stream)?;
                
                // Verbatim literals should never occur when parsing proc_macro2::Literal
                debug_assert!(
                    !matches!(syn_lit, Lit::Verbatim(_)),
                    "Unexpected Verbatim literal from proc_macro2::Literal"
                );
                
                Ok(Token::Literal(syn_lit))
            }
            TokenTree::Ident(ident) => {
                let ident_str = ident.to_string();
                
                // Check if this is a boolean literal
                match ident_str.as_str() {
                    "true" | "false" => {
                        // Wrap in TokenTree and convert to TokenStream to preserve span information
                        let token_stream: proc_macro2::TokenStream = TokenTree::Ident(ident).into();
                        let syn_lit: Lit = syn::parse2(token_stream)?;
                        Ok(Token::Literal(syn_lit))
                    }
                    _ => Ok(Token::Identifier(ident)),
                }
            }
            TokenTree::Punct(_) | TokenTree::Group(_) => {
                // These should be handled by the iterator
                Err(anyhow::anyhow!("Unexpected Punct or Group token in convert_token"))
            }
        }
    }
    
    /// Check if two characters form a known multi-character operator.
    fn is_compound_operator(first: char, second: char) -> bool {
        matches!(
            (first, second),
            ('&', '&') | ('|', '|') | ('=', '=') | ('!', '=') |
            ('<', '=') | ('>', '=') | ('<', '<') | ('>', '>')
        )
    }

    /// Get the next TokenTree from the current iterator (top of stack or main input).
    /// Returns None and sets pending_close when an iterator is exhausted.
    fn next_token_tree(&mut self) -> Option<TokenTree> {
        // Check if we have a pending token from lookahead
        if let Some(token) = self.pending_token.take() {
            return Some(token);
        }

        // Try to get from the top of the group stack first
        if let Some(level) = self.group_stack.last_mut() {
            if let Some(tt) = level.iter.next() {
                return Some(tt);
            }
            // Current iterator exhausted, pop it and set pending close
            let level = self.group_stack.pop().unwrap();
            self.pending_close = Some((level.delimiter, level.span));
            return None; // Signal that we need to emit close delimiter
        }
        
        // Stack is empty, get from main input
        self.input.next()
    }
}

/// A parsed literal value using syn's Lit enum.
///
/// This is a simple wrapper around syn's `Lit` type that includes boolean literals
/// even though they appear as identifiers in proc_macro2 (converted during lexing).
pub type Literal = Lit;

impl HasSpan for Literal {
    fn span(&self) -> Span {
        match self {
            Lit::Str(lit) => lit.span(),
            Lit::ByteStr(lit) => lit.span(),
            Lit::CStr(lit) => lit.span(),
            Lit::Byte(lit) => lit.span(),
            Lit::Char(lit) => lit.span(),
            Lit::Int(lit) => lit.span(),
            Lit::Float(lit) => lit.span(),
            Lit::Bool(lit) => lit.span(),
            Lit::Verbatim(_) => unreachable!("Verbatim literals should never occur"),
            _ => Span::call_site(),
        }
    }
}

/// A flattened token that represents elements from a TokenTree stream.
///
/// Groups are flattened into OpenDelim and CloseDelim tokens, making parsing
/// simpler by removing nesting from the token stream.
#[derive(Debug)]
pub enum Token {
    /// A literal value (integer, string, boolean, or float) with eager discrimination.
    Literal(Literal),
    
    /// An identifier.
    Identifier(Ident),
    
    /// A punctuation operator (single or multi-character).
    Punct {
        /// The operator string (e.g., "+", "&&", "<=").
        op: String,
        /// Span for error reporting.
        span: Span,
    },
    
    /// Opening delimiter (flattened from Group).
    OpenDelim {
        /// The type of delimiter (Parenthesis, Brace, Bracket).
        delimiter: Delimiter,
        /// Span for error reporting.
        span: Span,
    },
    
    /// Closing delimiter (flattened from Group).
    CloseDelim {
        /// The type of delimiter (Parenthesis, Brace, Bracket).
        delimiter: Delimiter,
        /// Span for error reporting.
        span: Span,
    },
}

impl HasSpan for Token {
    fn span(&self) -> Span {
        match self {
            Token::Literal(lit) => lit.span(),
            Token::Identifier(ident) => ident.span(),
            Token::Punct { span, .. } => *span,
            Token::OpenDelim { span, .. } => *span,
            Token::CloseDelim { span, .. } => *span,
        }
    }
}

impl HasSpan for TokenTree {
    fn span(&self) -> Span {
        match self {
            TokenTree::Group(g) => g.span(),
            TokenTree::Ident(i) => i.span(),
            TokenTree::Punct(p) => p.span(),
            TokenTree::Literal(l) => l.span(),
        }
    }
}

impl<I: Iterator<Item = TokenTree>> Iterator for LexLexer<I> {
    type Item = Result<Token, anyhow::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        // Check if we have a pending close delimiter to emit
        if let Some((delimiter, span)) = self.pending_close.take() {
            return Some(Ok(Token::CloseDelim { delimiter, span }));
        }

        // Get next token tree from current iterator
        let token = match self.next_token_tree() {
            Some(tt) => tt,
            None if self.pending_close.is_some() => {
                // An iterator was exhausted and pending_close was set
                // Emit the close delimiter on the next call (recursive call)
                return self.next();
            }
            None => {
                // All iterators exhausted, no more tokens
                return None;
            }
        };

        // Handle Groups by pushing their iterator onto the stack
        if let TokenTree::Group(group) = token {
            let delimiter = group.delimiter();
            let span = group.span();
            
            // Push the group's iterator and close info onto the stack
            self.group_stack.push(GroupLevel {
                iter: group.stream().into_iter(),
                delimiter,
                span,
            });
            
            // Return OpenDelim immediately
            return Some(Ok(Token::OpenDelim { delimiter, span }));
        }
        
        // Handle Punct tokens with potential combining
        if let TokenTree::Punct(punct) = token {
            let ch = punct.as_char();
            let spacing = punct.spacing();
            let span = punct.span();
            
            // If spacing is Joint, try to combine with next punct
            if spacing == Spacing::Joint {
                // Get next token to see if we can combine
                match self.next_token_tree() {
                    Some(TokenTree::Punct(next_punct)) => {
                        let next_ch = next_punct.as_char();
                        
                        // Check if they form a compound operator
                        if Self::is_compound_operator(ch, next_ch) {
                            // Combine them
                            let mut op = String::new();
                            op.push(ch);
                            op.push(next_ch);
                            return Some(Ok(Token::Punct { op, span }));
                        } else {
                            // Can't combine - emit first and save second for next iteration
                            self.pending_token = Some(TokenTree::Punct(next_punct));
                            return Some(Ok(Token::Punct { 
                                op: ch.to_string(), 
                                span 
                            }));
                        }
                    }
                    Some(other_token) => {
                        // Next token is not punct - emit our punct and save other for next iteration
                        self.pending_token = Some(other_token);
                        return Some(Ok(Token::Punct { 
                            op: ch.to_string(), 
                            span 
                        }));
                    }
                    None => {
                        // No next token - emit single punct
                        return Some(Ok(Token::Punct { 
                            op: ch.to_string(), 
                            span 
                        }));
                    }
                }
            } else {
                // Spacing is Alone - emit single character operator
                return Some(Ok(Token::Punct { 
                    op: ch.to_string(), 
                    span 
                }));
            }
        }
        
        // Not a group or punct, convert directly (Literal or Ident)
        Some(Self::convert_token(token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::TokenStream;
    use std::str::FromStr;

    #[test]
    fn test_literal_integer() {
        let input = TokenStream::from_str("42").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        
        let token = lexer.next().unwrap().unwrap();
        match token {
            Token::Literal(Lit::Int(..)) => {}
            _ => panic!("Expected integer literal, got {:?}", token),
        }
    }

    #[test]
    fn test_literal_string() {
        let input = TokenStream::from_str(r#""hello""#).unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        
        let token = lexer.next().unwrap().unwrap();
        match token {
            Token::Literal(Lit::Str(..)) => {}
            _ => panic!("Expected string literal"),
        }
    }

    #[test]
    fn test_literal_boolean() {
        // Test 'true' boolean literal
        let input = TokenStream::from_str("true").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        
        let token = lexer.next().unwrap().unwrap();
        match token {
            Token::Literal(Lit::Bool(lit_bool)) => {
                assert_eq!(lit_bool.value, true);
            }
            _ => panic!("Expected boolean literal for 'true', got {:?}", token),
        }
        
        // Test 'false' boolean literal
        let input = TokenStream::from_str("false").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        
        let token = lexer.next().unwrap().unwrap();
        match token {
            Token::Literal(Lit::Bool(lit_bool)) => {
                assert_eq!(lit_bool.value, false);
            }
            _ => panic!("Expected boolean literal for 'false', got {:?}", token),
        }
    }

    #[test]
    fn test_literal_float() {
        let input = TokenStream::from_str("3.14").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        
        let token = lexer.next().unwrap().unwrap();
        match token {
            Token::Literal(Lit::Float(..)) => {}
            _ => panic!("Expected float literal"),
        }
    }

    #[test]
    fn test_identifier() {
        let input = TokenStream::from_str("foo").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        
        let token = lexer.next().unwrap().unwrap();
        match token {
            Token::Identifier(ident) => {
                assert_eq!(ident.to_string(), "foo");
            }
            _ => panic!("Expected identifier"),
        }
    }

    #[test]
    fn test_punct() {
        let input = TokenStream::from_str("+").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        
        let token = lexer.next().unwrap().unwrap();
        match token {
            Token::Punct { op, .. } => {
                assert_eq!(op, "+");
            }
            _ => panic!("Expected punctuation"),
        }
    }

    #[test]
    fn test_compound_operator() {
        let input = TokenStream::from_str("a && b").unwrap();
        let lexer = LexLexer::new(input.into_iter());
        
        let tokens: Vec<_> = lexer.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(tokens.len(), 3);
        
        match &tokens[1] {
            Token::Punct { op, .. } => {
                assert_eq!(op, "&&");
            }
            _ => panic!("Expected && operator"),
        }
    }

    #[test]
    fn test_group_flattening() {
        let input = TokenStream::from_str("(10 + 20)").unwrap();
        let lexer = LexLexer::new(input.into_iter());
        
        // Should get: OpenDelim, Integer, Punct, Integer, CloseDelim
        let tokens: Vec<_> = lexer.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(tokens.len(), 5);
        
        matches!(tokens[0], Token::OpenDelim { delimiter: Delimiter::Parenthesis, .. });
        matches!(tokens[1], Token::Literal(Lit::Int(..)));
        assert!(matches!(&tokens[2], Token::Punct { op, .. } if op == "+"));
        matches!(tokens[3], Token::Literal(Lit::Int(..)));
        matches!(tokens[4], Token::CloseDelim { delimiter: Delimiter::Parenthesis, .. });
    }

    #[test]
    fn test_nested_groups() {
        let input = TokenStream::from_str("(10 + (20 * 30))").unwrap();
        let lexer = LexLexer::new(input.into_iter());
        
        let tokens: Vec<_> = lexer.collect::<Result<Vec<_>, _>>().unwrap();
        
        // Should have: OpenDelim, 10, +, OpenDelim, 20, *, 30, CloseDelim, CloseDelim
        assert_eq!(tokens.len(), 9);
        
        // Verify structure
        matches!(tokens[0], Token::OpenDelim { .. });
        matches!(tokens[1], Token::Literal(Lit::Int(..)));
        assert!(matches!(&tokens[2], Token::Punct { op, .. } if op == "+"));
        matches!(tokens[3], Token::OpenDelim { .. });
        matches!(tokens[4], Token::Literal(Lit::Int(..)));
        assert!(matches!(&tokens[5], Token::Punct { op, .. } if op == "*"));
        matches!(tokens[6], Token::Literal(Lit::Int(..)));
        matches!(tokens[7], Token::CloseDelim { .. });
        matches!(tokens[8], Token::CloseDelim { .. });
    }

    #[test]
    fn test_span_preservation() {
        let input = TokenStream::from_str("foo").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        
        let token = lexer.next().unwrap().unwrap();
        
        // HasSpan trait should provide span
        let span = HasSpan::span(&token);
        assert!(!span.source_text().unwrap_or_default().is_empty());
    }

    #[test]
    fn test_haspan_trait_for_tokentree() {
        let input = TokenStream::from_str("42").unwrap();
        let tt = input.into_iter().next().unwrap();
        
        // TokenTree implements HasSpan trait
        let _span = HasSpan::span(&tt);
    }

    #[test]
    fn test_mixed_tokens() {
        let input = TokenStream::from_str("foo + 42").unwrap();
        let lexer = LexLexer::new(input.into_iter());
        
        let tokens: Vec<_> = lexer.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(tokens.len(), 3);
        
        matches!(tokens[0], Token::Identifier(_));
        assert!(matches!(&tokens[1], Token::Punct { op, .. } if op == "+"));
        matches!(tokens[2], Token::Literal(Lit::Int(..)));
    }
}
