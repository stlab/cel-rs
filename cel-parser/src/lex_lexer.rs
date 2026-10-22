//! Flattens Rust token trees into CEL tokens while preserving source spans.
//!
//! Construct a [`LexLexer`] from a `proc_macro2::TokenStream` iterator and consume
//! it as an iterator of [`Token`] values. Groups emit matching opening and closing
//! delimiters, booleans become literals, and adjacent punctuation combines into
//! the supported operators. Documentation attributes become comment tokens.
//!
//! ```
//! use cel_parser::lex_lexer::{LexLexer, Token};
//!
//! let input: proc_macro2::TokenStream = "value && true".parse()?;
//! let mut lexer = LexLexer::new(input.into_iter());
//! assert!(matches!(lexer.next(), Some(Token::Identifier(id)) if id == "value"));
//! assert!(matches!(lexer.next(), Some(Token::Punct { op, .. }) if op == "&&"));
//! assert!(matches!(lexer.next(), Some(Token::Literal(_))));
//! assert!(lexer.next().is_none());
//! # Ok::<(), proc_macro2::LexError>(())
//! ```
//!
//! # Error Handling
//!
//! A `#` commits to a documentation attribute: `#[doc = "..."]` or `#![doc = "..."]`.
//! Other attribute shapes and literals Syn cannot parse produce [`Token::Error`].
//! Errors consume the inspected token trees; iteration may continue with the
//! remaining input, but does not roll back a malformed attribute.

use proc_macro2::{Delimiter, Ident, Spacing, Span, TokenTree};
use syn::Lit;

/// Provides a token's source span for diagnostics.
///
/// # Examples
///
/// ```
/// use cel_parser::lex_lexer::HasSpan;
///
/// let input: proc_macro2::TokenStream = "name".parse()?;
/// let token = input.into_iter().next().unwrap();
/// assert_eq!(HasSpan::span(&token).source_text().as_deref(), Some("name"));
/// # Ok::<(), proc_macro2::LexError>(())
/// ```
pub trait HasSpan {
    /// Returns the source span associated with this token.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_parser::lex_lexer::HasSpan;
    ///
    /// let input: proc_macro2::TokenStream = "42".parse()?;
    /// let token = input.into_iter().next().unwrap();
    /// assert_eq!(HasSpan::span(&token).source_text().as_deref(), Some("42"));
    /// # Ok::<(), proc_macro2::LexError>(())
    /// ```
    fn span(&self) -> Span;
}

/// Owns the unconsumed contents and matching close delimiter of one open group.
struct GroupLevel {
    iter: TokenStreamIter,
    delimiter: Delimiter,
    span: Span,
}

/// Owns a token stream's consuming iterator.
///
/// # Examples
///
/// ```
/// use cel_parser::lex_lexer::TokenStreamIter;
///
/// let input: proc_macro2::TokenStream = "42".parse()?;
/// let mut iter: TokenStreamIter = input.into_iter();
/// assert!(iter.next().is_some());
/// assert!(iter.next().is_none());
/// # Ok::<(), proc_macro2::LexError>(())
/// ```
pub type TokenStreamIter = proc_macro2::token_stream::IntoIter;

/// Stores one to three operator characters inline.
///
/// Equality with a string compares the complete character sequence, not a prefix.
///
/// # Examples
///
/// ```
/// use cel_parser::lex_lexer::PunctOp;
///
/// let op = PunctOp::Two(['&', '&']);
/// assert!(op == "&&");
/// assert!(op != "&");
/// assert!(op != "&&&");
/// ```
#[derive(Clone, Debug)]
pub enum PunctOp {
    /// Single character (e.g. `+`, `-`).
    One(char),
    /// Two characters (e.g. `&&`, `<=`, `..`).
    Two([char; 2]),
    /// Three characters (e.g. `..=`).
    Three([char; 3]),
}

impl PartialEq<str> for PunctOp {
    /// Returns whether the string contains exactly this operator's characters.
    fn eq(&self, other: &str) -> bool {
        let chars: &[char] = match self {
            PunctOp::One(c) => std::slice::from_ref(c),
            PunctOp::Two(chars) => chars,
            PunctOp::Three(chars) => chars,
        };
        chars.iter().copied().eq(other.chars())
    }
}

impl PartialEq<&str> for PunctOp {
    /// Returns whether the borrowed string equals this operator's characters.
    fn eq(&self, other: &&str) -> bool {
        self.eq(*other)
    }
}

/// Lazily flattens token trees and combines adjacent supported punctuation.
///
/// Each group emits matching delimiters, including `Delimiter::None`. Operators
/// combine only across joint punctuation within the same group. Combined tokens
/// retain the first punctuation's span; delimiters retain their group's span.
///
/// - Invariant: The group stack owns open groups in nesting order; lookahead and
///   pending close tokens precede any further tokens from their parent iterator.
/// - Complexity: Traversal visits O(n) token trees and retains O(d) group levels
///   for nesting depth d, excluding token payloads and upstream iterator storage.
///
/// # Examples
///
/// ```
/// use cel_parser::lex_lexer::{LexLexer, Token};
///
/// let input: proc_macro2::TokenStream = "(true)".parse()?;
/// let mut lexer = LexLexer::new(input.into_iter());
/// assert!(matches!(lexer.next(), Some(Token::OpenDelim { .. })));
/// assert!(matches!(lexer.next(), Some(Token::Literal(syn::Lit::Bool(b))) if b.value));
/// assert!(matches!(lexer.next(), Some(Token::CloseDelim { .. })));
/// assert!(lexer.next().is_none());
/// # Ok::<(), proc_macro2::LexError>(())
/// ```
pub struct LexLexer {
    input: TokenStreamIter,
    /// Stack of iterators for nested groups - allows lazy flattening.
    /// Each entry tracks the iterator and its close delimiter info.
    group_stack: Vec<GroupLevel>,
    /// Pending close delimiter to emit when we've just exhausted a group iterator.
    pending_close: Option<(Delimiter, Span)>,
    /// Pending token that was consumed while looking ahead.
    pending_token: Option<TokenTree>,
}

impl LexLexer {
    /// Takes ownership of an iterator without consuming any tokens.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_parser::lex_lexer::LexLexer;
    ///
    /// let lexer = LexLexer::new(proc_macro2::TokenStream::new().into_iter());
    /// assert_eq!(lexer.count(), 0);
    /// ```
    pub fn new(input: TokenStreamIter) -> Self {
        Self {
            input,
            group_stack: Vec::new(),
            pending_close: None,
            pending_token: None,
        }
    }

    /// Parses a literal while preserving its span.
    ///
    /// Returns `Token::Error` if Syn cannot interpret the literal.
    ///
    /// - Complexity: O(n) time and space for a literal of n characters.
    fn convert_literal(literal: proc_macro2::Literal) -> Token {
        let span = literal.span();
        match syn::parse2(TokenTree::Literal(literal).into()) {
            Ok(literal) => Token::Literal(literal),
            Err(error) => Token::Error {
                message: error.to_string(),
                span,
            },
        }
    }

    /// Converts boolean keywords to literals and preserves every other identifier.
    ///
    /// - Complexity: O(n) time for an identifier of n characters.
    fn convert_identifier(ident: Ident) -> Token {
        if ident == "true" || ident == "false" {
            Token::Literal(Lit::Bool(syn::LitBool::new(ident == "true", ident.span())))
        } else {
            Token::Identifier(ident)
        }
    }

    /// Returns whether two characters form a supported compound operator.
    fn is_compound_operator(first: char, second: char) -> bool {
        matches!(
            (first, second),
            ('&', '&')
                | ('|', '|')
                | ('=', '=')
                | ('!', '=')
                | ('<', '=')
                | ('>', '=')
                | ('<', '<')
                | ('>', '>')
                | ('-', '>')
                | ('=', '>')
                | (':', '=')
                | ('.', '.')
        )
    }

    /// Returns the next lookahead, innermost group, or root token, in that order.
    ///
    /// On group exhaustion, returns `None` and schedules its closing delimiter
    /// before any further parent tokens.
    fn next_token_tree(&mut self) -> Option<TokenTree> {
        if let Some(token) = self.pending_token.take() {
            return Some(token);
        }

        if let Some(level) = self.group_stack.last_mut() {
            if let Some(tt) = level.iter.next() {
                return Some(tt);
            }
            let level = self.group_stack.pop().unwrap();
            self.pending_close = Some((level.delimiter, level.span));
            return None;
        }

        self.input.next()
    }

    /// Consumes the longest supported operator beginning with `first`.
    ///
    /// Preserves the first punctuation's span and retains unmatched lookahead.
    /// Only joint punctuation within the current group can extend the operator.
    fn combine_punctuation(&mut self, first: proc_macro2::Punct) -> Token {
        let ch = first.as_char();
        let mut op = PunctOp::One(ch);
        if first.spacing() == Spacing::Joint {
            match self.next_token_tree() {
                Some(TokenTree::Punct(second))
                    if Self::is_compound_operator(ch, second.as_char()) =>
                {
                    let next_ch = second.as_char();
                    op = PunctOp::Two([ch, next_ch]);
                    if [ch, next_ch] == ['.', '.'] && second.spacing() == Spacing::Joint {
                        match self.next_token_tree() {
                            Some(TokenTree::Punct(third)) if third.as_char() == '=' => {
                                op = PunctOp::Three(['.', '.', '=']);
                            }
                            other => self.pending_token = other,
                        }
                    }
                }
                other => self.pending_token = other,
            }
        }
        Token::Punct {
            op,
            span: first.span(),
        }
    }

    /// `doc_attribute_tail = ["!"] "[" "doc" "=" string_literal "]".`
    ///
    /// Returns a documentation token or `Token::Error` for any mismatch, consuming
    /// the inspected token trees without rolling back.
    ///
    /// - Complexity: O(n) time and space for an attribute containing n characters.
    fn parse_doc_comment_attribute(&mut self, hash_span: Span) -> Token {
        let next = self.next_token_tree();
        let (inner, group_token) = match next {
            Some(TokenTree::Punct(p)) if p.as_char() == '!' => (true, self.next_token_tree()),
            other => (false, other),
        };
        let group = match group_token {
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket => g,
            Some(other) => {
                return Token::Error {
                    message: "expected `[` after `#`".to_string(),
                    span: other.span(),
                };
            }
            None => {
                return Token::Error {
                    message: "expected `[` after `#`".to_string(),
                    span: hash_span,
                };
            }
        };
        let mut inner_tokens = group.stream().into_iter();
        let ident_ok = match inner_tokens.next() {
            Some(TokenTree::Ident(id)) => id == "doc",
            _ => false,
        };
        if !ident_ok {
            return Token::Error {
                message: "expected `doc` after `#[`".to_string(),
                span: group.span(),
            };
        }
        let eq_ok = matches!(
            inner_tokens.next(),
            Some(TokenTree::Punct(p)) if p.as_char() == '='
        );
        if !eq_ok {
            return Token::Error {
                message: "expected `=` after `doc`".to_string(),
                span: group.span(),
            };
        }
        let text = match inner_tokens.next() {
            Some(TokenTree::Literal(lit)) => {
                let token_stream: proc_macro2::TokenStream = TokenTree::Literal(lit).into();
                match syn::parse2::<Lit>(token_stream) {
                    Ok(Lit::Str(s)) => s.value(),
                    _ => {
                        return Token::Error {
                            message: "expected a string literal after `doc =`".to_string(),
                            span: group.span(),
                        };
                    }
                }
            }
            _ => {
                return Token::Error {
                    message: "expected a string literal after `doc =`".to_string(),
                    span: group.span(),
                };
            }
        };
        if inner_tokens.next().is_some() {
            return Token::Error {
                message: "unexpected token after `doc = \"...\"`".to_string(),
                span: group.span(),
            };
        }
        Token::DocComment {
            text,
            inner,
            span: hash_span,
        }
    }
}

/// Represents a parsed literal, including boolean keywords.
///
/// # Examples
///
/// ```
/// use cel_parser::lex_lexer::{HasSpan, Literal};
///
/// let literal: Literal = syn::parse_str("true")?;
/// assert!(matches!(literal, Literal::Bool(ref b) if b.value));
/// let span = HasSpan::span(&literal);
/// # Ok::<(), syn::Error>(())
/// ```
pub type Literal = Lit;

impl HasSpan for Literal {
    /// Returns the literal's span, including uninterpreted verbatim literals.
    fn span(&self) -> Span {
        syn::spanned::Spanned::span(self)
    }
}

/// A flattened token that represents elements from a TokenTree stream.
///
/// Groups are flattened into OpenDelim and CloseDelim tokens, making parsing
/// simpler by removing nesting from the token stream.
///
/// # Examples
///
/// ```
/// use cel_parser::lex_lexer::{LexLexer, Token};
///
/// let input: proc_macro2::TokenStream = "name".parse()?;
/// let token = LexLexer::new(input.into_iter()).next().unwrap();
/// assert!(matches!(token, Token::Identifier(id) if id == "name"));
/// # Ok::<(), proc_macro2::LexError>(())
/// ```
#[derive(Debug)]
pub enum Token {
    /// A literal value (integer, string, boolean, or float) with eager discrimination.
    Literal(Literal),

    /// An identifier.
    Identifier(Ident),

    /// A punctuation operator storing all one to three characters inline.
    Punct {
        /// The operator (e.g., "+", "&&", "<=").
        op: PunctOp,
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

    /// A recognized `///`/`//!` doc comment, unwrapped from the `#[doc = "..."]`/
    /// `#![doc = "..."]` attribute-shaped token sequence `proc_macro2` produces for it. General
    /// `#[...]` attribute syntax is not supported — see `Token::Error`.
    DocComment {
        /// The comment's text: the doc-attribute string literal's value verbatim (including its
        /// leading space), e.g. `" the total"` for `/// the total`.
        text: String,
        /// `true` for `//!`/`#![doc]` (inner), `false` for `///`/`#[doc]` (outer).
        inner: bool,
        /// The `#` token's span.
        span: Span,
    },

    /// An unsupported attribute shape or a literal Syn cannot parse.
    Error {
        /// A description of what was expected instead.
        message: String,
        /// Where the mismatch was detected.
        span: Span,
    },
}

impl HasSpan for Token {
    /// Returns the token's preserved source or diagnostic span.
    fn span(&self) -> Span {
        match self {
            Token::Literal(lit) => lit.span(),
            Token::Identifier(ident) => ident.span(),
            Token::Punct { span, .. } => *span,
            Token::OpenDelim { span, .. } => *span,
            Token::CloseDelim { span, .. } => *span,
            Token::DocComment { span, .. } => *span,
            Token::Error { span, .. } => *span,
        }
    }
}

impl HasSpan for TokenTree {
    /// Returns the source span of any token-tree variant.
    fn span(&self) -> Span {
        TokenTree::span(self)
    }
}

impl Iterator for LexLexer {
    type Item = Token;

    /// Returns the next flattened token or `None` after exhausting the input.
    ///
    /// - Complexity: O(n) time and space for the consumed token payload; entering
    ///   a group also retains its iterator and may grow the O(d) nesting stack.
    fn next(&mut self) -> Option<Self::Item> {
        if let Some((delimiter, span)) = self.pending_close.take() {
            return Some(Token::CloseDelim { delimiter, span });
        }

        let token = match self.next_token_tree() {
            Some(token) => token,
            None => {
                return self
                    .pending_close
                    .take()
                    .map(|(delimiter, span)| Token::CloseDelim { delimiter, span });
            }
        };

        Some(match token {
            TokenTree::Group(group) => {
                let delimiter = group.delimiter();
                let span = group.span();
                self.group_stack.push(GroupLevel {
                    iter: group.stream().into_iter(),
                    delimiter,
                    span,
                });
                Token::OpenDelim { delimiter, span }
            }
            TokenTree::Punct(punct) if punct.as_char() == '#' => {
                self.parse_doc_comment_attribute(punct.span())
            }
            TokenTree::Punct(punct) => self.combine_punctuation(punct),
            TokenTree::Literal(literal) => Self::convert_literal(literal),
            TokenTree::Ident(ident) => Self::convert_identifier(ident),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::TokenStream;
    use std::str::FromStr;

    /// Compares complete character sequences, including non-ASCII characters.
    #[test]
    fn punctuation_equality_compares_characters_without_prefix_matches() {
        let cases = [
            (PunctOp::One('+'), "+", true),
            (PunctOp::One('+'), "++", false),
            (PunctOp::One('+'), "", false),
            (PunctOp::One('\u{a7}'), "\u{a7}", true),
            (PunctOp::Two(['&', '&']), "&&", true),
            (PunctOp::Two(['&', '&']), "&", false),
            (PunctOp::Two(['&', '&']), "&&&", false),
            (PunctOp::Two(['&', '&']), "&|", false),
            (PunctOp::Three(['.', '.', '=']), "..=", true),
            (PunctOp::Three(['.', '.', '=']), "..", false),
            (PunctOp::Three(['.', '.', '=']), "..==", false),
            (PunctOp::Three(['.', '.', '=']), "...", false),
        ];
        // Check both supported string comparison interfaces against the same contract.
        for (op, text, expected) in cases {
            assert_eq!(op == *text, expected, "{op:?} versus {text:?}");
            assert_eq!(op == text, expected, "{op:?} versus {text:?}");
        }
    }

    /// Preserves the source span of a literal not interpreted by Syn.
    #[test]
    fn verbatim_literal_retains_its_span() {
        let literal = proc_macro2::Literal::from_str("42").unwrap();
        let literal = Lit::Verbatim(literal);
        assert_eq!(HasSpan::span(&literal).source_text().as_deref(), Some("42"));
    }

    /// Clones punctuation values without sharing mutable character storage.
    #[test]
    fn punctuation_clones_are_independent_values() {
        let cases = [
            (PunctOp::One('+'), "+"),
            (PunctOp::Two(['&', '&']), "&&"),
            (PunctOp::Three(['.', '.', '=']), "..="),
        ];
        // Mutate each original and its copy independently through their public variants.
        for (mut original, text) in cases {
            let mut copied = original.clone();
            assert!(copied == text);
            match &mut original {
                PunctOp::One(c) => *c = '!',
                PunctOp::Two(chars) => chars[0] = '!',
                PunctOp::Three(chars) => chars[0] = '!',
            }
            assert!(copied == text);
            copied = PunctOp::One('?');
            assert!(copied == "?");
            assert!(original != "?");
            assert!(original != text);
        }
    }

    /// Preserves literal values and spans across every supported literal category.
    #[test]
    fn literal_spans_preserve_the_source() {
        // All literal variants use the same source-span contract.
        for source in [
            "42",
            "1.5",
            "\"text\"",
            "b\"bytes\"",
            "c\"cstr\"",
            "'x'",
            "b'x'",
            "true",
            "false",
        ] {
            let mut lexer = LexLexer::new(TokenStream::from_str(source).unwrap().into_iter());
            let Some(Token::Literal(literal)) = lexer.next() else {
                panic!("expected literal for {source}");
            };
            assert_eq!(
                HasSpan::span(&literal).source_text().as_deref(),
                Some(source)
            );
            assert_eq!(
                HasSpan::span(&Token::Literal(literal))
                    .source_text()
                    .as_deref(),
                Some(source)
            );
            assert!(lexer.next().is_none());
        }
    }

    /// Preserves token-tree spans for groups, identifiers, punctuation, and literals.
    #[test]
    fn token_tree_spans_cover_each_variant() {
        // Verify delegation for every token-tree variant.
        for source in ["(x)", "name", "+", "42"] {
            let tree = TokenStream::from_str(source)
                .unwrap()
                .into_iter()
                .next()
                .unwrap();
            assert_eq!(HasSpan::span(&tree).source_text().as_deref(), Some(source));
        }
    }

    /// Preserves diagnostic and delimiter spans rather than substituting call-site spans.
    #[test]
    fn flattened_tokens_preserve_their_associated_spans() {
        let cases: &[(&str, &[&str])] = &[
            ("(x)", &["(x)", "x", "(x)"]),
            ("&&", &["&"]),
            ("#[bad]", &["[bad]"]),
        ];
        // Group delimiters use group spans; combined punctuation uses its first character.
        for &(source, spans) in cases {
            let mut lexer = LexLexer::new(TokenStream::from_str(source).unwrap().into_iter());
            for &expected in spans {
                let token = lexer.next().unwrap();
                assert_eq!(
                    HasSpan::span(&token).source_text().as_deref(),
                    Some(expected)
                );
            }
            assert!(lexer.next().is_none());
        }
    }

    /// Produces no tokens for empty input, including repeated exhaustion checks.
    #[test]
    fn empty_input_remains_exhausted() {
        let mut lexer = LexLexer::new(TokenStream::new().into_iter());
        assert!(lexer.next().is_none());
        assert!(lexer.next().is_none());
    }

    /// Combines exactly the supported adjacent operators.
    #[test]
    fn combines_each_supported_operator() {
        // Each operator must consume its complete spelling, but not the following identifier.
        for text in [
            "&&", "||", "==", "!=", "<=", ">=", "<<", ">>", "->", "=>", ":=", "..", "..=",
        ] {
            let input = TokenStream::from_str(&format!("{text} tail")).unwrap();
            let mut lexer = LexLexer::new(input.into_iter());
            assert!(matches!(lexer.next(), Some(Token::Punct { op, .. }) if op == text));
            assert!(matches!(lexer.next(), Some(Token::Identifier(id)) if id == "tail"));
            assert!(lexer.next().is_none());
        }
    }

    /// Preserves uncombined lookahead tokens and respects punctuation spacing.
    #[test]
    fn punctuation_lookahead_preserves_the_remaining_stream() {
        let cases: &[(&str, &[&str])] = &[
            ("& &", &["&", "&"]),
            (".. =", &["..", "="]),
            ("..+", &["..", "+"]),
            ("&&=", &["&&", "="]),
            ("+=", &["+", "="]),
            ("...", &["..", "."]),
        ];
        // Compare the entire punctuation stream so no lookahead token can disappear.
        for &(input, expected) in cases {
            let mut lexer = LexLexer::new(TokenStream::from_str(input).unwrap().into_iter());
            for &text in expected {
                assert!(matches!(lexer.next(), Some(Token::Punct { op, .. }) if op == text));
            }
            assert!(lexer.next().is_none(), "{input}");
        }
    }

    /// Emits every empty group's matching delimiters before resuming its parent.
    #[test]
    fn empty_groups_preserve_delimiter_order() {
        let mut lexer = LexLexer::new(TokenStream::from_str("([]{}) tail").unwrap().into_iter());
        let expected = [
            (true, Delimiter::Parenthesis),
            (true, Delimiter::Bracket),
            (false, Delimiter::Bracket),
            (true, Delimiter::Brace),
            (false, Delimiter::Brace),
            (false, Delimiter::Parenthesis),
        ];
        // Verify both delimiter kind and nesting order.
        for (open, expected_delimiter) in expected {
            match lexer.next() {
                Some(Token::OpenDelim { delimiter, .. }) if open => {
                    assert_eq!(delimiter, expected_delimiter);
                }
                Some(Token::CloseDelim { delimiter, .. }) if !open => {
                    assert_eq!(delimiter, expected_delimiter);
                }
                other => panic!("expected delimiter, got {other:?}"),
            }
        }
        assert!(matches!(lexer.next(), Some(Token::Identifier(id)) if id == "tail"));
        assert!(lexer.next().is_none());
    }

    /// Emits a group close after punctuation lookahead or an incomplete attribute.
    #[test]
    fn lookahead_does_not_consume_a_group_close() {
        // Joint punctuation is constructible even when no following token exists.
        for punctuation in ["&", "..", "#", "#!"] {
            let group = proc_macro2::Group::new(
                Delimiter::Parenthesis,
                punctuation
                    .chars()
                    .map(|ch| TokenTree::Punct(proc_macro2::Punct::new(ch, Spacing::Joint)))
                    .collect(),
            );
            let input: TokenStream = [
                TokenTree::Group(group),
                TokenTree::Ident(Ident::new("tail", Span::call_site())),
            ]
            .into_iter()
            .collect();
            let mut lexer = LexLexer::new(input.into_iter());
            assert!(matches!(lexer.next(), Some(Token::OpenDelim { .. })));
            match lexer.next() {
                Some(Token::Punct { op, .. }) if !punctuation.starts_with('#') => {
                    assert!(op == punctuation);
                }
                Some(Token::Error { message, .. }) if punctuation.starts_with('#') => {
                    assert!(!message.is_empty());
                }
                other => panic!("expected punctuation or attribute error, got {other:?}"),
            }
            assert!(matches!(lexer.next(), Some(Token::CloseDelim { .. })));
            assert!(matches!(lexer.next(), Some(Token::Identifier(id)) if id == "tail"));
            assert!(lexer.next().is_none());
        }
    }

    /// Emits delimiters for invisible groups without combining across their boundaries.
    #[test]
    fn invisible_groups_bound_operator_combining() {
        let contents = [
            TokenTree::Punct(proc_macro2::Punct::new('.', Spacing::Joint)),
            TokenTree::Punct(proc_macro2::Punct::new('.', Spacing::Joint)),
        ]
        .into_iter()
        .collect();
        let input: TokenStream = [
            TokenTree::Group(proc_macro2::Group::new(Delimiter::None, contents)),
            TokenTree::Punct(proc_macro2::Punct::new('=', Spacing::Alone)),
        ]
        .into_iter()
        .collect();
        let mut lexer = LexLexer::new(input.into_iter());
        assert!(matches!(
            lexer.next(),
            Some(Token::OpenDelim {
                delimiter: Delimiter::None,
                ..
            })
        ));
        assert!(matches!(lexer.next(), Some(Token::Punct { op, .. }) if op == ".."));
        assert!(matches!(
            lexer.next(),
            Some(Token::CloseDelim {
                delimiter: Delimiter::None,
                ..
            })
        ));
        assert!(matches!(lexer.next(), Some(Token::Punct { op, .. }) if op == "="));
        assert!(lexer.next().is_none());
    }

    /// Reports incomplete attributes at end of input without terminating iteration early.
    #[test]
    fn incomplete_attributes_at_end_of_input_report_errors() {
        // Neither form supplies the required bracket group.
        for source in ["#", "#!"] {
            let mut lexer = LexLexer::new(TokenStream::from_str(source).unwrap().into_iter());
            assert!(
                matches!(lexer.next(), Some(Token::Error { message, .. }) if !message.is_empty())
            );
            assert!(lexer.next().is_none());
            assert!(lexer.next().is_none());
        }
    }

    /// Decodes explicit documentation strings while preserving the inner marker.
    #[test]
    fn explicit_documentation_attributes_decode_string_values() {
        let input =
            TokenStream::from_str(r##"#[doc = "line\nnext"] #![doc = r#" raw "#]"##).unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        assert!(
            matches!(lexer.next(), Some(Token::DocComment { text, inner: false, .. })
                if text == "line\nnext")
        );
        assert!(
            matches!(lexer.next(), Some(Token::DocComment { text, inner: true, .. })
                if text == " raw ")
        );
        assert!(lexer.next().is_none());
    }

    /// Reports each malformed attribute shape without losing the next outer token.
    #[test]
    fn malformed_attributes_report_errors_and_resume() {
        // Attribute contents are untrusted input, not caller preconditions.
        for input in [
            "#[] tail",
            "#[doc] tail",
            "#[doc \"x\"] tail",
            "#[doc =] tail",
            "#[doc = value] tail",
            "#[doc = b\"x\"] tail",
            "#![doc = 1] tail",
            "#[doc = \"x\" extra] tail",
        ] {
            let mut lexer = LexLexer::new(TokenStream::from_str(input).unwrap().into_iter());
            assert!(
                matches!(lexer.next(), Some(Token::Error { message, .. }) if !message.is_empty()),
                "{input}"
            );
            assert!(matches!(lexer.next(), Some(Token::Identifier(id)) if id == "tail"));
            assert!(lexer.next().is_none());
        }
    }

    /// Recognizes booleans without treating raw or longer identifiers as literals.
    #[test]
    fn boolean_recognition_preserves_other_identifiers() {
        let input = TokenStream::from_str("true false r#true true_value").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        assert!(matches!(lexer.next(), Some(Token::Literal(Lit::Bool(lit))) if lit.value));
        assert!(matches!(lexer.next(), Some(Token::Literal(Lit::Bool(lit))) if !lit.value));
        assert!(matches!(lexer.next(), Some(Token::Identifier(id)) if id == "r#true"));
        assert!(matches!(lexer.next(), Some(Token::Identifier(id)) if id == "true_value"));
        assert!(lexer.next().is_none());
    }

    /// Recognizes an integer literal.
    #[test]
    fn test_literal_integer() {
        let input = TokenStream::from_str("42").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());

        let token = lexer.next().unwrap();
        match token {
            Token::Literal(Lit::Int(..)) => {}
            _ => panic!("Expected integer literal, got {:?}", token),
        }
    }

    /// Recognizes a string literal.
    #[test]
    fn test_literal_string() {
        let input = TokenStream::from_str(r#""hello""#).unwrap();
        let mut lexer = LexLexer::new(input.into_iter());

        let token = lexer.next().unwrap();
        match token {
            Token::Literal(Lit::Str(..)) => {}
            _ => panic!("Expected string literal"),
        }
    }

    /// Recognizes both boolean keyword values.
    #[test]
    fn test_literal_boolean() {
        // Test 'true' boolean literal
        let input = TokenStream::from_str("true").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());

        let token = lexer.next().unwrap();
        match token {
            Token::Literal(Lit::Bool(lit_bool)) => {
                assert!(lit_bool.value);
            }
            _ => panic!("Expected boolean literal for 'true', got {:?}", token),
        }

        // Test 'false' boolean literal
        let input = TokenStream::from_str("false").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());

        let token = lexer.next().unwrap();
        match token {
            Token::Literal(Lit::Bool(lit_bool)) => {
                assert!(!lit_bool.value);
            }
            _ => panic!("Expected boolean literal for 'false', got {:?}", token),
        }
    }

    /// Recognizes a floating-point literal.
    #[test]
    fn test_literal_float() {
        let input = TokenStream::from_str("3.14").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());

        let token = lexer.next().unwrap();
        match token {
            Token::Literal(Lit::Float(..)) => {}
            _ => panic!("Expected float literal"),
        }
    }

    /// Preserves an identifier's spelling.
    #[test]
    fn test_identifier() {
        let input = TokenStream::from_str("foo").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());

        let token = lexer.next().unwrap();
        match token {
            Token::Identifier(ident) => {
                assert_eq!(ident.to_string(), "foo");
            }
            _ => panic!("Expected identifier"),
        }
    }

    /// Emits a single punctuation character.
    #[test]
    fn test_punct() {
        let input = TokenStream::from_str("+").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());

        let token = lexer.next().unwrap();
        match token {
            Token::Punct { op, .. } => {
                assert!(op == "+");
            }
            _ => panic!("Expected punctuation"),
        }
    }

    /// Combines a logical operator between identifiers.
    #[test]
    fn test_compound_operator() {
        let input = TokenStream::from_str("a && b").unwrap();
        let lexer = LexLexer::new(input.into_iter());

        let tokens: Vec<_> = lexer.collect();
        assert_eq!(tokens.len(), 3);

        match &tokens[1] {
            Token::Punct { op, .. } => {
                assert!(op == "&&");
            }
            _ => panic!("Expected && operator"),
        }
    }

    /// Emits matching delimiters around a group's flattened contents.
    #[test]
    fn test_group_flattening() {
        let input = TokenStream::from_str("(10 + 20)").unwrap();
        let lexer = LexLexer::new(input.into_iter());

        // Should get: OpenDelim, Integer, Punct, Integer, CloseDelim
        let tokens: Vec<_> = lexer.collect();
        assert_eq!(tokens.len(), 5);

        assert!(matches!(
            tokens[0],
            Token::OpenDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            }
        ));
        assert!(matches!(tokens[1], Token::Literal(Lit::Int(..))));
        assert!(matches!(&tokens[2], Token::Punct { op, .. } if op == "+"));
        assert!(matches!(tokens[3], Token::Literal(Lit::Int(..))));
        assert!(matches!(
            tokens[4],
            Token::CloseDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            }
        ));
    }

    /// Preserves the nesting order of groups and their contents.
    #[test]
    fn test_nested_groups() {
        let input = TokenStream::from_str("(10 + (20 * 30))").unwrap();
        let lexer = LexLexer::new(input.into_iter());

        let tokens: Vec<_> = lexer.collect();

        // Should have: OpenDelim, 10, +, OpenDelim, 20, *, 30, CloseDelim, CloseDelim
        assert_eq!(tokens.len(), 9);

        // Verify structure
        assert!(matches!(tokens[0], Token::OpenDelim { .. }));
        assert!(matches!(tokens[1], Token::Literal(Lit::Int(..))));
        assert!(matches!(&tokens[2], Token::Punct { op, .. } if op == "+"));
        assert!(matches!(tokens[3], Token::OpenDelim { .. }));
        assert!(matches!(tokens[4], Token::Literal(Lit::Int(..))));
        assert!(matches!(&tokens[5], Token::Punct { op, .. } if op == "*"));
        assert!(matches!(tokens[6], Token::Literal(Lit::Int(..))));
        assert!(matches!(tokens[7], Token::CloseDelim { .. }));
        assert!(matches!(tokens[8], Token::CloseDelim { .. }));
    }

    /// Retains an identifier's source span.
    #[test]
    fn test_span_preservation() {
        let input = TokenStream::from_str("foo").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());

        let token = lexer.next().unwrap();

        // HasSpan trait should provide span
        let span = HasSpan::span(&token);
        assert!(!span.source_text().unwrap_or_default().is_empty());
    }

    /// Provides a source span through the token-tree trait implementation.
    #[test]
    fn test_haspan_trait_for_tokentree() {
        let input = TokenStream::from_str("42").unwrap();
        let tt = input.into_iter().next().unwrap();

        assert_eq!(HasSpan::span(&tt).source_text().as_deref(), Some("42"));
    }

    /// Preserves the order of identifiers, punctuation, and literals.
    #[test]
    fn test_mixed_tokens() {
        let input = TokenStream::from_str("foo + 42").unwrap();
        let lexer = LexLexer::new(input.into_iter());

        let tokens: Vec<_> = lexer.collect();
        assert_eq!(tokens.len(), 3);

        assert!(matches!(tokens[0], Token::Identifier(_)));
        assert!(matches!(&tokens[1], Token::Punct { op, .. } if op == "+"));
        assert!(matches!(tokens[2], Token::Literal(Lit::Int(..))));
    }

    /// Emits an arrow as one operator.
    #[test]
    fn arrow_is_two_char_punct() {
        let stream: TokenStream = "->".parse().unwrap();
        let mut lexer = LexLexer::new(stream.into_iter());
        let tok = lexer.next().expect("one token");
        assert!(
            matches!(
                tok,
                Token::Punct {
                    op: PunctOp::Two(['-', '>']),
                    ..
                }
            ),
            "expected PunctOp::Two([\"->\"])), got {tok:?}"
        );
        assert!(lexer.next().is_none(), "expected no more tokens");
    }

    /// Emits a fat arrow as one operator.
    #[test]
    fn fat_arrow_is_two_char_punct() {
        let stream: TokenStream = "=>".parse().unwrap();
        let mut lexer = LexLexer::new(stream.into_iter());
        let tok = lexer.next().expect("one token");
        assert!(
            matches!(
                tok,
                Token::Punct {
                    op: PunctOp::Two(['=', '>']),
                    ..
                }
            ),
            "expected PunctOp::Two([\"=>\"]), got {tok:?}"
        );
        assert!(lexer.next().is_none(), "expected no more tokens");
    }

    /// Preserves outer documentation text and its outer marker.
    #[test]
    fn recognizes_an_outer_doc_comment() {
        let input = TokenStream::from_str("/// the total").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        let token = lexer.next().unwrap();
        match token {
            Token::DocComment { text, inner, .. } => {
                assert_eq!(text, " the total");
                assert!(!inner);
            }
            other => panic!("expected DocComment, got {other:?}"),
        }
        assert!(lexer.next().is_none());
    }

    /// Preserves inner documentation text and its inner marker.
    #[test]
    fn recognizes_an_inner_doc_comment() {
        let input = TokenStream::from_str("//! module docs").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        let token = lexer.next().unwrap();
        match token {
            Token::DocComment { text, inner, .. } => {
                assert_eq!(text, " module docs");
                assert!(inner);
            }
            other => panic!("expected DocComment, got {other:?}"),
        }
    }

    /// Resumes ordinary tokenization after documentation.
    #[test]
    fn doc_comment_run_is_followed_by_the_next_real_token() {
        let input = TokenStream::from_str("/// x\ncell").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        assert!(matches!(lexer.next(), Some(Token::DocComment { .. })));
        match lexer.next() {
            Some(Token::Identifier(ident)) => assert_eq!(ident.to_string(), "cell"),
            other => panic!("expected Identifier(cell), got {other:?}"),
        }
    }

    /// Rejects a non-documentation attribute.
    #[test]
    fn a_non_doc_attribute_is_a_lex_error() {
        let input = TokenStream::from_str("#[foo]").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        let token = lexer.next().unwrap();
        assert!(
            matches!(token, Token::Error { .. }),
            "expected Error, got {token:?}"
        );
    }

    /// Rejects a numeric documentation value.
    #[test]
    fn a_non_string_doc_value_is_a_lex_error() {
        let input = TokenStream::from_str("#[doc = 5]").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        let token = lexer.next().unwrap();
        assert!(
            matches!(token, Token::Error { .. }),
            "expected Error, got {token:?}"
        );
    }

    /// Rejects an attribute marker followed by an identifier instead of a group.
    #[test]
    fn a_bare_hash_with_no_group_is_a_lex_error() {
        let input = TokenStream::from_str("# foo").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        let token = lexer.next().unwrap();
        assert!(
            matches!(token, Token::Error { .. }),
            "expected Error, got {token:?}"
        );
    }

    /// Rejects an attribute group with parentheses instead of brackets.
    #[test]
    fn a_hash_led_group_with_the_wrong_delimiter_is_a_lex_error() {
        let input = TokenStream::from_str("#(foo)").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        let token = lexer.next().unwrap();
        assert!(
            matches!(token, Token::Error { .. }),
            "expected Error, got {token:?}"
        );
    }

    /// Rejects trailing tokens in a documentation attribute.
    #[test]
    fn extra_tokens_after_the_doc_string_are_a_lex_error() {
        let input = TokenStream::from_str("#[doc = \"x\", extra]").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        let token = lexer.next().unwrap();
        assert!(
            matches!(token, Token::Error { .. }),
            "expected Error, got {token:?}"
        );
    }

    /// Retains a documentation comment's source span.
    #[test]
    fn doc_comment_token_has_a_span() {
        let input = TokenStream::from_str("/// x").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        let token = lexer.next().unwrap();
        let span = HasSpan::span(&token);
        assert!(!span.source_text().unwrap_or_default().is_empty());
    }

    /// Emits the assignment operator as one token.
    #[test]
    fn walrus_operator_lexes_as_one_compound_token() {
        let stream: TokenStream = ":=".parse().unwrap();
        let mut lexer = LexLexer::new(stream.into_iter());
        let token = lexer.next().expect("one token");
        assert!(matches!(
            token,
            Token::Punct { ref op, .. } if op == ":="
        ));
        assert!(lexer.next().is_none());
    }

    /// Preserves an identifier following a lone colon.
    #[test]
    fn colon_followed_by_non_equals_stays_two_separate_tokens() {
        let stream: TokenStream = ": x".parse().unwrap();
        let mut lexer = LexLexer::new(stream.into_iter());
        let first = lexer.next().expect("colon token");
        assert!(matches!(first, Token::Punct { ref op, .. } if op == ":"));
        assert!(matches!(lexer.next(), Some(Token::Identifier(id)) if id == "x"));
        assert!(lexer.next().is_none());
    }

    /// Keeps the endpoints of a range as integer literals.
    #[test]
    fn digit_followed_by_double_dot_does_not_lex_as_a_float_literal() {
        let input = TokenStream::from_str("5..10").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        match lexer.next() {
            Some(Token::Literal(Lit::Int(lit))) => {
                assert_eq!(lit.base10_parse::<i32>().unwrap(), 5)
            }
            other => panic!("expected an integer literal, got {other:?}"),
        }
        assert!(matches!(lexer.next(), Some(Token::Punct { op, .. }) if op == ".."));
        assert!(matches!(lexer.next(), Some(Token::Literal(Lit::Int(lit)))
                if lit.base10_parse::<i32>().unwrap() == 10));
        assert!(lexer.next().is_none());
    }

    /// Emits an exclusive range operator as one token.
    #[test]
    fn double_dot_combines_into_two_char_punct() {
        let stream: TokenStream = "..".parse().unwrap();
        let mut lexer = LexLexer::new(stream.into_iter());
        let tok = lexer.next().expect("one token");
        assert!(
            matches!(
                tok,
                Token::Punct {
                    op: PunctOp::Two(['.', '.']),
                    ..
                }
            ),
            "expected PunctOp::Two(['.', '.']), got {tok:?}"
        );
        assert!(lexer.next().is_none(), "expected no more tokens");
    }

    /// Emits an inclusive range operator as one token.
    #[test]
    fn double_dot_equals_combines_into_three_char_punct() {
        let stream: TokenStream = "..=".parse().unwrap();
        let mut lexer = LexLexer::new(stream.into_iter());
        let tok = lexer.next().expect("one token");
        assert!(
            matches!(
                tok,
                Token::Punct {
                    op: PunctOp::Three(['.', '.', '=']),
                    ..
                }
            ),
            "expected PunctOp::Three(['.', '.', '=']), got {tok:?}"
        );
        assert!(lexer.next().is_none(), "expected no more tokens");
    }

    /// Preserves the literal after an exclusive range operator.
    #[test]
    fn double_dot_not_followed_by_equals_stays_two_char_and_does_not_consume_the_next_token() {
        let stream: TokenStream = "..5".parse().unwrap();
        let mut lexer = LexLexer::new(stream.into_iter());
        let first = lexer.next().expect("dot-dot token");
        assert!(
            matches!(
                first,
                Token::Punct {
                    op: PunctOp::Two(['.', '.']),
                    ..
                }
            ),
            "expected PunctOp::Two(['.', '.']), got {first:?}"
        );
        match lexer.next() {
            Some(Token::Literal(Lit::Int(lit))) => {
                assert_eq!(lit.base10_parse::<i32>().unwrap(), 5)
            }
            other => panic!("expected integer literal 5, got {other:?}"),
        }
    }

    /// Preserves an integer field index after a single dot.
    #[test]
    fn single_dot_before_a_digit_is_unaffected_by_range_combining() {
        // Regression guard: tuple field access (`x.0`) must still lex as a lone `.` Punct
        // followed by an integer literal, not get swept into the new `..`/`..=` combining.
        let stream: TokenStream = ".0".parse().unwrap();
        let mut lexer = LexLexer::new(stream.into_iter());
        let first = lexer.next().expect("dot token");
        assert!(
            matches!(
                first,
                Token::Punct {
                    op: PunctOp::One('.'),
                    ..
                }
            ),
            "expected PunctOp::One('.'), got {first:?}"
        );
        assert!(matches!(lexer.next(), Some(Token::Literal(Lit::Int(lit)))
                if lit.base10_parse::<i32>().unwrap() == 0));
        assert!(lexer.next().is_none());
    }
}
