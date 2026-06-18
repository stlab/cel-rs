//! Procedural macros for the cel-rs crate.
//!
//! This crate provides procedural macros for working with CEL (Common Expression Language)
//! expressions in Rust. It includes macros for validating CEL expressions at compile time
//! and debugging token parsing.
//!
//! # Examples
//!
//! ## Validating CEL expressions
//!
//! ```rust
//! use cel_rs_macros::expression;
//!
//! // This will compile only if the expression is valid CEL
//! expression! {
//!     54 + 25 * (11 + 6 * 6)
//! };
//! ```
//!
//! ## Debugging token parsing
//!
//! ```rust
//! use cel_rs_macros::print_tokens;
//!
//! // This will print the parsed tokens to stdout during compilation
//! print_tokens! {
//!     10 + 20
//! };
//! ```

use cel_runtime::{CELParser, OpLookup};
use proc_macro::TokenStream as ProcMacroTokenStream;
use proc_macro2::{Literal, TokenStream};
use quote::quote_spanned;

/// Validates that the input contains a valid CEL expression.
///
/// ```rust
/// use cel_rs_macros::expression;
/// expression! {
///     54 + 25 * (11 + 6 * 6)
/// };
/// ```
#[proc_macro]
pub fn expression(input: ProcMacroTokenStream) -> ProcMacroTokenStream {
    let input = TokenStream::from(input);
    let mut parser = CELParser::new(OpLookup::new());
    match parser.parse_tokens(input.into_iter()) {
        Ok(_) => ProcMacroTokenStream::new(),
        Err(e) => {
            let msg_lit = Literal::string(e.message());
            let mut tokens = quote_spanned!(e.span() => compile_error!(#msg_lit));
            if let Some(end) = e.end_span() {
                let end_lit = Literal::string("expression continues here");
                tokens.extend(quote_spanned!(end => compile_error!(#end_lit)));
            }
            tokens.into()
        }
    }
}

/// Prints the tokens for debugging purposes.
///
/// # Example
/// ```rust
/// use cel_rs_macros::print_tokens;
/// print_tokens! {
///     "hello"_key
/// };
/// ```
#[proc_macro]
pub fn print_tokens(input: ProcMacroTokenStream) -> ProcMacroTokenStream {
    println!("{input}");
    let input = TokenStream::from(input);
    for e in input {
        match e {
            proc_macro2::TokenTree::Punct(punct) => {
                println!("punct: {punct:?}");
            }
            proc_macro2::TokenTree::Ident(ident) => {
                println!("ident: {ident:?}");
            }
            proc_macro2::TokenTree::Group(group) => {
                println!("group: {group:?}");
            }
            proc_macro2::TokenTree::Literal(lit) => {
                println!("literal: {lit:?}");
            }
        }
    }
    ProcMacroTokenStream::new()
}
