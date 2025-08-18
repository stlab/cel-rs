//! A `lex-lexer` is a lexer taking a lex token stream and returning a token stream of a different
//! type. Initially this is used to convert the TokenTree from Rust's proc_macro into a higher level
//! token stream. The goal, however, is to be able to specify with a grammar how to process a token
//! stream.

use proc_macro2::TokenTree;
use std::iter::Peekable;

pub(crate) struct LexLexer<I: Iterator<Item = TokenTree>> {
    input: Peekable<I>,
}

impl<I: Iterator<Item = TokenTree>> LexLexer<I> {
    pub(crate) fn new(input: I) -> Self {
        Self {
            input: input.peekable(),
        }
    }
}

pub(crate) enum Literal {
    Integer(IntegerLit),
    String(StringLit),
    Boolean(BooleanLit),
    Float(FloatLit),
}

pub(crate) enum Token {
    Literal(Literal),
    Identifier(Ident),
    Punct(Punct),
    Group(Group),
}

impl<I: Iterator<Item = TokenTree>> Iterator for LexLexer<I> {
    type Item = Result<Token, anyhow::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        self.parse_one()?;
    }
}
