use cel_parser::CELParser;
use proc_macro::TokenStream as ProcMacroTokenStream;
use proc_macro2::TokenStream;

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
    let mut parser = CELParser::new(input.into_iter());
    if !parser.is_expression() {
        parser.report_error("Expected expression");
    }
    parser.get_output().clone().into()
}

/// Prints the tokens for debugging purposes.
///
/// # Example
/// ```rust
/// use cel_rs_macros::print_tokens;
/// print_tokens! {
///     10
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
