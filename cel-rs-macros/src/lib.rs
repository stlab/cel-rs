use proc_macro::{Delimiter, TokenStream, TokenTree};

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
/// use cel_rs_macros::expr;
/// expr! {
///     54 + 25 * (11 + 5)
/// };
/// ```
///
/// Will be parsed as:
/// ```text
/// factor: 54
/// term: factor
/// factor: 25
/// factor: 11
/// term: factor
/// factor: 5
/// term: factor
/// expr: term + term
/// factor: ( expr )
/// term: factor * factor
/// expr: term + term
/// ```
struct Parser<I: Iterator<Item = TokenTree>> {
    tokens: I,
    current: Option<TokenTree>,
}

impl<I: Iterator<Item = TokenTree>> Parser<I> {
    fn new(mut tokens: I) -> Self {
        let current = tokens.next();
        Parser { tokens, current }
    }

    fn advance(&mut self) {
        self.current = self.tokens.next();
    }

    fn parse_expr(&mut self) {
        self.parse_term();

        let mut result = String::from("expr: term");

        while let Some(TokenTree::Punct(punct)) = &self.current {
            match punct.as_char() {
                '+' | '-' => {
                    let op = punct.as_char();
                    self.advance();
                    self.parse_term();
                    result.push_str(&format!(" {} term", op));
                }
                _ => break,
            }
        }
        println!("{}", result);
    }

    fn parse_term(&mut self) {
        self.parse_factor();

        let mut result = String::from("term: factor");

        while let Some(TokenTree::Punct(punct)) = &self.current {
            match punct.as_char() {
                '*' | '/' => {
                    let op = punct.as_char();
                    self.advance();
                    self.parse_factor();
                    result.push_str(&format!(" {} factor", op));
                }
                _ => break,
            }
        }
        println!("{}", result);
    }

    fn parse_factor(&mut self) {
        match self.current.take() {
            Some(TokenTree::Literal(lit)) => {
                println!("factor: {}", lit);
                self.advance();
            }
            Some(TokenTree::Ident(ident)) => {
                println!("factor: {}", ident);
                self.advance();
            }
            Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis => {
                let mut parser = Parser::new(group.stream().into_iter());
                parser.parse_expr();
                println!("factor: ( expr )");
                self.advance();
            }
            _ => panic!("Unexpected token in factor"),
        }
    }
}

/// Macro that parses an expression and prints the productions
///
/// # Example
/// ```rust
/// use cel_rs_macros::expr;
/// expr! {
///     54 + 25 * (11 + 5)
/// };
/// ```
#[proc_macro]
pub fn expr(input: TokenStream) -> TokenStream {
    let mut parser = Parser::new(input.into_iter());
    parser.parse_expr();
    parser.tokens.collect()
}

///
/// # Example
/// ```rust
/// use cel_rs_macros::print_tokens;
/// print_tokens! {
///     format < = = hello    <=== layout: /*comment */ view,
///     // comment
///     /// doc comment
/// };
/// ```
#[proc_macro]
pub fn print_tokens(input: TokenStream) -> TokenStream {
    println!("{}", input);
    for e in input {
        println!("{e}");
    }
    TokenStream::new()
}
