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
/// Emits no code on success and a spanned compiler diagnostic on invalid input.
///
/// - Complexity: Includes the parser's time and temporary storage for the input.
///
/// # Examples
/// ```rust
/// use cel_rs_macros::expression;
/// expression! {
///     54 + 25 * (11 + 6 * 6)
/// };
/// ```
#[proc_macro]
pub fn expression(input: ProcMacroTokenStream) -> ProcMacroTokenStream {
    expression_tokens(input.into()).into()
}

/// Returns no tokens for valid CEL, or spanned compile errors for invalid CEL.
///
/// - Complexity: Includes the parser's time and temporary storage for the input.
fn expression_tokens(input: TokenStream) -> TokenStream {
    let mut parser = CELParser::new(OpLookup::new());
    match parser.parse_tokens(input.into_iter()) {
        Ok(_) => TokenStream::new(),
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
                quote!({ #start_error; #end_error })
            } else {
                start_error
            }
        }
    }
}

/// Prints the tokens for debugging purposes.
///
/// Prints the source to stdout and top-level token kinds to stderr, then emits no code.
///
/// - Complexity: O(n) formatting time and output for n debug-representation bytes.
///
/// # Examples
/// ```rust
/// use cel_rs_macros::print_tokens;
/// print_tokens! {
///     "hello"_key
/// };
/// ```
#[proc_macro]
pub fn print_tokens(input: ProcMacroTokenStream) -> ProcMacroTokenStream {
    println!("{input}");
    write_token_kinds(input.into(), std::io::stderr().lock())
        .expect("failed to print CEL token kinds");
    ProcMacroTokenStream::new()
}

/// Writes top-level token kinds and their debug representations in source order.
///
/// - Complexity: O(n) formatting time and output for n debug-representation bytes.
///
/// # Errors
/// Returns an error when the destination cannot accept the diagnostic output.
fn write_token_kinds(input: TokenStream, mut output: impl std::io::Write) -> std::io::Result<()> {
    for e in input {
        match e {
            proc_macro2::TokenTree::Punct(punct) => {
                writeln!(output, "punct: {punct:?}")?;
            }
            proc_macro2::TokenTree::Ident(ident) => {
                writeln!(output, "ident: {ident:?}")?;
            }
            proc_macro2::TokenTree::Group(group) => {
                writeln!(output, "group: {group:?}")?;
            }
            proc_macro2::TokenTree::Literal(lit) => {
                writeln!(output, "literal: {lit:?}")?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Validates expressions without generating executable runtime code.
    #[test]
    fn valid_expression_emits_no_tokens() {
        assert!(expression_tokens("54 + 25 * (11 + 6 * 6)".parse().unwrap()).is_empty());
    }

    /// Reports malformed, undefined and ill-typed input through compile-error expansions.
    #[test]
    fn invalid_expressions_emit_compiler_diagnostics() {
        for source in ["54 +", "missing_name", "1 + true"] {
            let expansion = expression_tokens(source.parse().unwrap()).to_string();
            assert!(expansion.contains("compile_error"), "{source}: {expansion}");
        }
    }

    /// Writes each top-level token kind in source order without flattening groups.
    #[test]
    fn token_kind_diagnostics() {
        let mut output = Vec::new();
        write_token_kinds("name + (42) \"text\"".parse().unwrap(), &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        let kinds: Vec<_> = output
            .lines()
            .map(|line| line.split_once(':').unwrap().0)
            .collect();
        assert_eq!(kinds, ["ident", "punct", "group", "literal"]);
        let mut empty = Vec::new();
        write_token_kinds(TokenStream::new(), &mut empty).unwrap();
        assert!(empty.is_empty());
    }

    /// Reports an output failure instead of silently omitting token diagnostics.
    #[test]
    fn token_kind_output_error() {
        let mut buffer = [];
        let output = std::io::Cursor::new(&mut buffer[..]);
        assert!(write_token_kinds("name".parse().unwrap(), output).is_err());
    }
}
