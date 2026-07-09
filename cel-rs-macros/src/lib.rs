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

use cel_parser::{CELParser, OpLookup};
use proc_macro::TokenStream as ProcMacroTokenStream;
use proc_macro2::{Literal, TokenStream};
use quote::{quote, quote_spanned};

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
            let start_error = quote_spanned!(e.span() => compile_error!(#msg_lit));
            if let Some(end) = e.end_span() {
                // Intentional second diagnostic at the expression end span. A single merged
                // underline requires `Span::join()`, which is not stable; until then we emit
                // two `compile_error!` invocations so both start and end locations are reported.
                let end_lit = Literal::string("expression continues here");
                let end_error = quote_spanned!(end => compile_error!(#end_lit));
                // Two bare compile_error!() are not valid in expression context;
                // wrapping in a block makes the expansion a valid block expression
                // while still causing the compiler to expand both invocations.
                quote!({ #start_error; #end_error }).into()
            } else {
                start_error.into()
            }
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
                eprintln!("punct: {punct:?}");
            }
            proc_macro2::TokenTree::Ident(ident) => {
                eprintln!("ident: {ident:?}");
            }
            proc_macro2::TokenTree::Group(group) => {
                eprintln!("group: {group:?}");
            }
            proc_macro2::TokenTree::Literal(lit) => {
                eprintln!("literal: {lit:?}");
            }
        }
    }
    ProcMacroTokenStream::new()
}
