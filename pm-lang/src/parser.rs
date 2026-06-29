//! pm-lang parser — grammar productions and sheet construction.
//!
//! ```ebnf
//! sheet          = "sheet" identifier "{" { sheet_item } "}".
//! sheet_item     = cell_decl | relationship_decl | conditional_decl.
//! cell_decl      = "cell" identifier cell_type_init ";".
//! cell_type_init = ":" type_name "=" literal
//!                | ":" type_name
//!                | "=" literal.
//! type_name = identifier.
//! relationship_decl = "relationship" [ identifier ] "{" { method_decl } "}".
//! method_decl = "method" cell_list "->" cell_list method_body.
//! cell_list   = "[" identifier { "," identifier } "]".
//! method_body = "{" output_list "}".
//! output_list = "(" or_expression "," or_expression { "," or_expression } ")"
//!             | or_expression.
//! conditional_decl   = "conditional" identifier "{" { conditional_branch } [ default_branch ] "}".
//! conditional_branch = literal "=>" "{" { method_decl } "}" [ "," ].
//! default_branch     = "_"   "=>" "{" { method_decl } "}" [ "," ].
//! ```

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::iter::Peekable;
use std::str::FromStr;

use cel_parser::lex_lexer::{HasSpan, LexLexer, Literal, Token};
use cel_parser::{CELParser, OpLookup, ParseError};
use cel_runtime::DynSegment;
use proc_macro2::{Delimiter, Span, TokenStream};
use property_model::{CellId, Method, RelationshipId, Sheet};

use crate::TypeRegistry;
use crate::type_registry::{AddCellFn, AddConditionalFn, CallDynFn, PushArgFn, TypeEntry};

/// Parser result type.
pub type Result<T> = std::result::Result<T, ParseError>;

// ---------------------------------------------------------------------------
// ParseContext — mutable state for one parse_str call
// ---------------------------------------------------------------------------

struct ParseContext {
    /// Token stream; `None` while temporarily owned by CELParser.
    tokens: Option<Peekable<LexLexer>>,
    sheet: Sheet,
    /// Maps cell name → (CellId, TypeId) for method and conditional compilation.
    cell_names: HashMap<String, (CellId, TypeId)>,
}

impl ParseContext {
    fn peek_token(&mut self) -> Option<&Token> {
        self.tokens.as_mut()?.peek()
    }

    fn advance(&mut self) -> Option<Token> {
        self.tokens.as_mut()?.next()
    }

    fn peek_span(&mut self) -> Span {
        self.tokens
            .as_mut()
            .and_then(|t| t.peek())
            .map(|t| t.span())
            .unwrap_or_else(Span::call_site)
    }

    fn err_at(&mut self, msg: impl Into<String>) -> ParseError {
        ParseError::new(msg.into(), self.peek_span())
    }

    /// Consumes and returns `true` if the next token is an identifier matching `kw`.
    fn is_keyword(&mut self, kw: &str) -> bool {
        let ok = matches!(
            self.tokens.as_mut().and_then(|t| t.peek()),
            Some(Token::Identifier(id)) if id == kw
        );
        if ok {
            self.advance();
        }
        ok
    }

    /// Consumes any identifier.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not an identifier.
    fn consume_ident(&mut self) -> Result<(String, Span)> {
        let span = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::Identifier(id)) => {
                let s = id.span();
                let _ = id;
                s
            }
            other => {
                let s = other.map(|t| t.span()).unwrap_or(Span::call_site());
                return Err(ParseError::new("expected identifier", s));
            }
        };
        if let Some(Token::Identifier(id)) = self.advance() {
            return Ok((id.to_string(), span));
        }
        unreachable!("peeked identifier, advance must return it")
    }

    /// Consumes a specific punctuation token.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token does not match `p`.
    fn expect_punct(&mut self, p: &str) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::Punct { op, span }) if op == p => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            Ok(span)
        } else {
            Err(ParseError::new(format!("expected `{p}`"), span))
        }
    }

    /// Consumes and returns `true` if the next token is punctuation matching `p`.
    fn consume_punct(&mut self, p: &str) -> bool {
        let ok = matches!(
            self.tokens.as_mut().and_then(|t| t.peek()),
            Some(Token::Punct { op, .. }) if op == p
        );
        if ok {
            self.advance();
        }
        ok
    }

    /// Consumes `{`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not `{`.
    fn expect_open_brace(&mut self) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::OpenDelim {
                delimiter: Delimiter::Brace,
                span,
            }) => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            Ok(span)
        } else {
            Err(ParseError::new("expected `{`", span))
        }
    }

    /// Consumes `}`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not `}`.
    fn expect_close_brace(&mut self) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::CloseDelim {
                delimiter: Delimiter::Brace,
                span,
            }) => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            Ok(span)
        } else {
            Err(ParseError::new("expected `}`", span))
        }
    }

    /// Consumes `[`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not `[`.
    fn expect_open_bracket(&mut self) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::OpenDelim {
                delimiter: Delimiter::Bracket,
                span,
            }) => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            Ok(span)
        } else {
            Err(ParseError::new("expected `[`", span))
        }
    }

    /// Consumes `]`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not `]`.
    fn expect_close_bracket(&mut self) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::CloseDelim {
                delimiter: Delimiter::Bracket,
                span,
            }) => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            Ok(span)
        } else {
            Err(ParseError::new("expected `]`", span))
        }
    }

    fn consume_open_paren(&mut self) -> bool {
        let ok = matches!(
            self.tokens.as_mut().and_then(|t| t.peek()),
            Some(Token::OpenDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            })
        );
        if ok {
            self.advance();
        }
        ok
    }

    fn peek_close_paren(&mut self) -> bool {
        matches!(
            self.tokens.as_mut().and_then(|t| t.peek()),
            Some(Token::CloseDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            })
        )
    }

    /// Consumes and returns a literal token.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not a literal.
    fn consume_literal(&mut self) -> Result<(Literal, Span)> {
        let span = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::Literal(lit)) => lit.span(),
            other => {
                let s = other.map(|t| t.span()).unwrap_or(Span::call_site());
                return Err(ParseError::new("expected literal", s));
            }
        };
        if let Some(Token::Literal(lit)) = self.advance() {
            return Ok((lit, span));
        }
        unreachable!("peeked literal, advance must return it")
    }

    fn at_close_brace(&mut self) -> bool {
        matches!(
            self.tokens.as_mut().and_then(|t| t.peek()),
            Some(Token::CloseDelim {
                delimiter: Delimiter::Brace,
                ..
            }) | None
        )
    }
}

// ---------------------------------------------------------------------------
// PmParser
// ---------------------------------------------------------------------------

/// Parses pm-lang source strings into live [`Sheet`]s.
///
/// # Example
///
/// ```rust,no_run
/// use pm_lang::{PmParser, TypeRegistry};
/// use cel_parser::OpLookup;
///
/// let mut parser = PmParser::new(TypeRegistry::new(), OpLookup::new());
/// let sheet = parser.parse_str("sheet s { cell x: i32 = 0; }").unwrap();
/// ```
pub struct PmParser {
    pub(crate) types: TypeRegistry,
    pub(crate) cel: CELParser,
}

impl PmParser {
    /// Creates a parser with the given type registry and operation lookup.
    ///
    /// `op_lookup` is forwarded to the embedded [`CELParser`] when compiling method
    /// body expressions.
    pub fn new(types: TypeRegistry, op_lookup: OpLookup) -> Self {
        PmParser {
            types,
            cel: CELParser::new(op_lookup),
        }
    }

    /// Returns a mutable reference to the embedded CEL operation lookup.
    pub fn op_lookup_mut(&mut self) -> &mut OpLookup {
        self.cel.op_lookup_mut()
    }

    /// Parses a pm-lang source string into a live [`Sheet`].
    ///
    /// Resets internal parse state on each call.
    ///
    /// # Errors
    ///
    /// Returns `Err` on any syntax error, unknown type name, type mismatch between a
    /// cell annotation and its initializer, undeclared cell name in a method cell list,
    /// or arity mismatch in an `output_list` tuple.
    pub fn parse_str(&mut self, source: &str) -> Result<Sheet> {
        let stream =
            TokenStream::from_str(source).map_err(|e| ParseError::new(e.to_string(), e.span()))?;
        let mut ctx = ParseContext {
            tokens: Some(LexLexer::new(stream.into_iter()).peekable()),
            sheet: Sheet::new(),
            cell_names: HashMap::new(),
        };
        self.parse_sheet(&mut ctx)?;
        if let Some(tok) = ctx.peek_token() {
            return Err(ParseError::new("unexpected token", tok.span()));
        }
        Ok(ctx.sheet)
    }

    // -----------------------------------------------------------------------
    // Grammar productions
    // -----------------------------------------------------------------------

    /// `sheet = "sheet" identifier "{" { sheet_item } "}".`
    fn parse_sheet(&mut self, ctx: &mut ParseContext) -> Result<()> {
        if !ctx.is_keyword("sheet") {
            return Err(ctx.err_at("expected `sheet`"));
        }
        ctx.consume_ident()?; // sheet name (ignored at runtime)
        ctx.expect_open_brace()?;
        while !ctx.at_close_brace() {
            self.parse_sheet_item(ctx)?;
        }
        ctx.expect_close_brace()?;
        Ok(())
    }

    /// `sheet_item = cell_decl | relationship_decl | conditional_decl.`
    fn parse_sheet_item(&mut self, ctx: &mut ParseContext) -> Result<()> {
        match ctx.peek_token() {
            Some(Token::Identifier(id)) if id == "cell" => self.parse_cell_decl(ctx),
            Some(Token::Identifier(id)) if id == "relationship" => {
                self.parse_relationship_decl(ctx)
            }
            Some(Token::Identifier(id)) if id == "conditional" => self.parse_conditional_decl(ctx),
            Some(tok) => Err(ParseError::new(
                "expected `cell`, `relationship`, or `conditional`",
                tok.span(),
            )),
            None => Err(ParseError::new(
                "unexpected end of input",
                Span::call_site(),
            )),
        }
    }

    /// `cell_decl = "cell" identifier cell_type_init ";".`
    ///
    /// `cell_type_init = ":" type_name "=" literal`
    ///                 `| ":" type_name`
    ///                 `| "=" literal.`
    fn parse_cell_decl(&mut self, ctx: &mut ParseContext) -> Result<()> {
        ctx.is_keyword("cell"); // consume
        let (name, name_span) = ctx.consume_ident()?;
        let _ = name_span;

        let (type_id, add_fn, initial_value): (TypeId, AddCellFn, Box<dyn Any>) =
            if ctx.consume_punct(":") {
                let (type_name, type_span) = ctx.consume_ident()?;
                let entry = self.types.get(&type_name).ok_or_else(|| {
                    ParseError::new(format!("unknown type `{type_name}`"), type_span)
                })?;
                let tid = entry.type_id;
                let add_fn = entry.add_cell_fn;
                if ctx.consume_punct("=") {
                    let (lit, lit_span) = ctx.consume_literal()?;
                    let val = parse_literal_as(entry, &lit, lit_span)?;
                    (tid, add_fn, val)
                } else {
                    let default_fn = entry.default_fn.ok_or_else(|| {
                        ParseError::new(
                            format!("type `{type_name}` has no default; provide `= literal`"),
                            type_span,
                        )
                    })?;
                    (tid, add_fn, default_fn())
                }
            } else if ctx.consume_punct("=") {
                let (lit, lit_span) = ctx.consume_literal()?;
                let (tid, add_fn, val) = infer_and_parse_literal(&self.types, &lit, lit_span)?;
                (tid, add_fn, val)
            } else {
                return Err(ctx.err_at("expected `:` or `=` in cell declaration"));
            };

        ctx.expect_punct(";")?;

        let cell_id = add_fn(&mut ctx.sheet, initial_value);
        ctx.cell_names.insert(name, (cell_id, type_id));
        Ok(())
    }

    /// `relationship_decl = "relationship" [ identifier ] "{" { method_decl } "}".`
    fn parse_relationship_decl(&mut self, ctx: &mut ParseContext) -> Result<()> {
        ctx.is_keyword("relationship"); // consume
        if matches!(ctx.peek_token(), Some(Token::Identifier(_))) {
            ctx.consume_ident()?; // optional name
        }
        ctx.expect_open_brace()?;
        let mut methods = Vec::new();
        while !ctx.at_close_brace() {
            methods.push(self.parse_method_decl(ctx)?);
        }
        ctx.expect_close_brace()?;
        ctx.sheet
            .add_relationship(methods)
            .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
        Ok(())
    }

    /// `conditional_decl = "conditional" identifier "{" { conditional_branch } [ default_branch ] "}".`
    fn parse_conditional_decl(&mut self, ctx: &mut ParseContext) -> Result<()> {
        ctx.is_keyword("conditional"); // consume
        let (match_name, match_span) = ctx.consume_ident()?;
        let (match_cell_id, match_type_id) =
            ctx.cell_names.get(&match_name).copied().ok_or_else(|| {
                ParseError::new(format!("undeclared cell `{match_name}`"), match_span)
            })?;
        let add_cond_fn: AddConditionalFn = self
            .types
            .entry_by_type_id(match_type_id)
            .ok_or_else(|| ParseError::new("match cell type not in TypeRegistry", match_span))?
            .add_conditional_fn;
        ctx.expect_open_brace()?;

        let mut branches: Vec<(Box<dyn Any>, RelationshipId)> = Vec::new();
        let mut default_rel_ids: Vec<RelationshipId> = Vec::new();

        while !ctx.at_close_brace() {
            // Check for default branch `_ => { ... }`
            if matches!(ctx.peek_token(), Some(Token::Identifier(id)) if id == "_") {
                ctx.advance(); // consume `_`
                ctx.expect_punct("=>")?;
                ctx.expect_open_brace()?;
                let mut methods = Vec::new();
                while !ctx.at_close_brace() {
                    methods.push(self.parse_method_decl(ctx)?);
                }
                ctx.expect_close_brace()?;
                ctx.consume_punct(",");
                let rel_id = ctx
                    .sheet
                    .add_relationship(methods)
                    .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
                default_rel_ids.push(rel_id);
                break; // default branch is always last
            }

            // Named branch: `literal => { ... }`
            let (lit, lit_span) = ctx.consume_literal()?;
            let entry = self
                .types
                .entry_by_type_id(match_type_id)
                .ok_or_else(|| ParseError::new("match cell type not in TypeRegistry", lit_span))?;
            let branch_val = parse_literal_as(entry, &lit, lit_span)?;
            ctx.expect_punct("=>")?;
            ctx.expect_open_brace()?;
            let mut methods = Vec::new();
            while !ctx.at_close_brace() {
                methods.push(self.parse_method_decl(ctx)?);
            }
            ctx.expect_close_brace()?;
            ctx.consume_punct(",");
            let rel_id = ctx
                .sheet
                .add_relationship(methods)
                .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
            branches.push((branch_val, rel_id));
        }
        ctx.expect_close_brace()?;

        add_cond_fn(&mut ctx.sheet, match_cell_id, branches, default_rel_ids)
            .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
        Ok(())
    }

    /// `method_decl = "method" cell_list "->" cell_list method_body.`
    fn parse_method_decl(&mut self, ctx: &mut ParseContext) -> Result<Method> {
        if !ctx.is_keyword("method") {
            return Err(ctx.err_at("expected `method`"));
        }
        let inputs = self.parse_cell_list(ctx)?;
        ctx.expect_punct("->")?;
        let outputs = self.parse_cell_list(ctx)?;
        let (segments, call_fns) = self.parse_method_body(ctx, &inputs, &outputs)?;
        Ok(build_method(inputs, outputs, segments, call_fns))
    }

    /// `cell_list = "[" identifier { "," identifier } "]".`
    fn parse_cell_list(&self, ctx: &mut ParseContext) -> Result<Vec<(String, CellId, TypeId)>> {
        ctx.expect_open_bracket()?;
        let mut cells = Vec::new();
        loop {
            let (name, span) = ctx.consume_ident()?;
            let (cell_id, type_id) = ctx
                .cell_names
                .get(&name)
                .copied()
                .ok_or_else(|| ParseError::new(format!("undeclared cell `{name}`"), span))?;
            cells.push((name, cell_id, type_id));
            if !ctx.consume_punct(",") {
                break;
            }
        }
        ctx.expect_close_bracket()?;
        Ok(cells)
    }

    /// `method_body = "{" output_list "}".`
    ///
    /// Returns `(segments, call_dyn_fns)` — one segment and one `call_dyn_fn` per output.
    fn parse_method_body(
        &mut self,
        ctx: &mut ParseContext,
        inputs: &[(String, CellId, TypeId)],
        outputs: &[(String, CellId, TypeId)],
    ) -> Result<(Vec<DynSegment>, Vec<CallDynFn>)> {
        ctx.expect_open_brace()?;

        // Pre-compute push_arg dispatch table for input scope.
        let scope_data: Vec<(String, PushArgFn, usize)> = inputs
            .iter()
            .enumerate()
            .map(|(idx, (name, _, type_id))| {
                let fn_ptr = self
                    .types
                    .entry_by_type_id(*type_id)
                    .expect("input cell type registered")
                    .push_arg_fn;
                (name.clone(), fn_ptr, idx)
            })
            .collect();

        // Push scope: CELParser resolves input cell names to push_arg ops.
        self.cel
            .op_lookup_mut()
            .push_scope(move |name, segment, arity, _span| {
                if arity != 0 {
                    return Ok(false);
                }
                for (n, fn_ptr, idx) in &scope_data {
                    if n == name {
                        fn_ptr(segment, *idx);
                        return Ok(true);
                    }
                }
                Ok(false)
            });

        let result = self.parse_output_list(ctx);
        self.cel.op_lookup_mut().pop_scope();
        let segments = result?;

        ctx.expect_close_brace()?;

        if segments.len() != outputs.len() {
            return Err(ctx.err_at(format!(
                "output list has {} expression(s) but method declares {} output(s)",
                segments.len(),
                outputs.len()
            )));
        }

        // Verify output types and collect call_dyn_fn per output.
        let mut call_fns = Vec::with_capacity(outputs.len());
        for (i, (seg, (out_name, _, out_type_id))) in
            segments.iter().zip(outputs.iter()).enumerate()
        {
            let actual_type_id = seg.peek_output_type_id().ok_or_else(|| {
                ctx.err_at(format!(
                    "output {i} `{out_name}`: expression produced no value"
                ))
            })?;
            if actual_type_id != *out_type_id {
                let expected = self
                    .types
                    .entry_by_type_id(*out_type_id)
                    .map(|e| e.type_name)
                    .unwrap_or("?");
                let got = self
                    .types
                    .entry_by_type_id(actual_type_id)
                    .map(|e| e.type_name)
                    .unwrap_or("?");
                return Err(ctx.err_at(format!(
                    "output {i} `{out_name}`: type mismatch: expected `{expected}`, got `{got}`"
                )));
            }
            let call_fn = self
                .types
                .entry_by_type_id(*out_type_id)
                .expect("output cell type registered")
                .call_dyn_fn;
            call_fns.push(call_fn);
        }

        Ok((segments, call_fns))
    }

    /// `output_list = "(" or_expression "," or_expression { "," or_expression } ")" | or_expression.`
    fn parse_output_list(&mut self, ctx: &mut ParseContext) -> Result<Vec<DynSegment>> {
        if ctx.consume_open_paren() {
            let seg1 = self.parse_cel_or_expression(ctx)?;
            if ctx.peek_close_paren() {
                ctx.advance(); // parenthesized single expression — not a tuple
                return Ok(vec![seg1]);
            }
            ctx.expect_punct(",")?;
            let mut segs = vec![seg1];
            loop {
                segs.push(self.parse_cel_or_expression(ctx)?);
                if ctx.peek_close_paren() {
                    ctx.advance();
                    break;
                }
                ctx.expect_punct(",")?;
            }
            Ok(segs)
        } else {
            Ok(vec![self.parse_cel_or_expression(ctx)?])
        }
    }

    /// Delegates one `or_expression` to CELParser, sharing the token stream.
    fn parse_cel_or_expression(&mut self, ctx: &mut ParseContext) -> Result<DynSegment> {
        let tokens = ctx.tokens.take().expect("tokens present");
        self.cel.set_lex_tokens(tokens);
        let result = self.cel.parse_or_expression();
        ctx.tokens = Some(self.cel.take_lex_tokens().expect("tokens set"));
        result
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Parses `lit` as the type described by `entry`.
///
/// # Errors
///
/// Returns `Err` if the literal kind does not match the expected type.
fn parse_literal_as(entry: &TypeEntry, lit: &Literal, span: Span) -> Result<Box<dyn Any>> {
    use syn::Lit;
    let val = match lit {
        Lit::Int(i) => parse_int_literal(entry, i),
        Lit::Float(f) => parse_float_literal(entry, f),
        Lit::Bool(b) if entry.type_id == TypeId::of::<bool>() => {
            Some(Box::new(b.value) as Box<dyn Any>)
        }
        Lit::Str(s) if entry.type_id == TypeId::of::<String>() => {
            Some(Box::new(s.value()) as Box<dyn Any>)
        }
        _ => None,
    };
    val.ok_or_else(|| {
        ParseError::new(
            format!("literal cannot be used as type `{}`", entry.type_name),
            span,
        )
    })
}

macro_rules! try_parse_int {
    ($i:expr, $T:ty) => {
        $i.base10_parse::<$T>()
            .ok()
            .map(|v| Box::new(v) as Box<dyn Any>)
    };
}

macro_rules! try_parse_float {
    ($f:expr, $T:ty) => {
        $f.base10_parse::<$T>()
            .ok()
            .map(|v| Box::new(v) as Box<dyn Any>)
    };
}

/// Parses `i` as the integer or float type described by `entry`, returning `None` on mismatch.
fn parse_int_literal(entry: &TypeEntry, i: &syn::LitInt) -> Option<Box<dyn Any>> {
    match entry.type_id {
        t if t == TypeId::of::<i8>() => try_parse_int!(i, i8),
        t if t == TypeId::of::<i16>() => try_parse_int!(i, i16),
        t if t == TypeId::of::<i32>() => try_parse_int!(i, i32),
        t if t == TypeId::of::<i64>() => try_parse_int!(i, i64),
        t if t == TypeId::of::<i128>() => try_parse_int!(i, i128),
        t if t == TypeId::of::<isize>() => try_parse_int!(i, isize),
        t if t == TypeId::of::<u8>() => try_parse_int!(i, u8),
        t if t == TypeId::of::<u16>() => try_parse_int!(i, u16),
        t if t == TypeId::of::<u32>() => try_parse_int!(i, u32),
        t if t == TypeId::of::<u64>() => try_parse_int!(i, u64),
        t if t == TypeId::of::<u128>() => try_parse_int!(i, u128),
        t if t == TypeId::of::<usize>() => try_parse_int!(i, usize),
        t if t == TypeId::of::<f64>() => try_parse_float!(i, f64),
        t if t == TypeId::of::<f32>() => try_parse_float!(i, f32),
        _ => None,
    }
}

/// Parses `f` as the float type described by `entry`, returning `None` on mismatch.
fn parse_float_literal(entry: &TypeEntry, f: &syn::LitFloat) -> Option<Box<dyn Any>> {
    match entry.type_id {
        t if t == TypeId::of::<f64>() => try_parse_float!(f, f64),
        t if t == TypeId::of::<f32>() => try_parse_float!(f, f32),
        _ => None,
    }
}

/// Infers the type from the literal (matching CEL defaults) and parses the value.
///
/// - Unsuffixed integer → `i32`; suffixed integer → suffix type.
/// - Unsuffixed float → `f64`; suffixed float → suffix type.
/// - `bool` literal → `bool`.
/// - String literal → `String`.
///
/// # Errors
///
/// Returns `Err` if the suffix names a type not in the registry.
fn infer_and_parse_literal(
    types: &TypeRegistry,
    lit: &Literal,
    span: Span,
) -> Result<(TypeId, AddCellFn, Box<dyn Any>)> {
    use syn::Lit;
    match lit {
        Lit::Int(i) => {
            let type_name = if i.suffix().is_empty() {
                "i32"
            } else {
                i.suffix()
            };
            let entry = types.get(type_name).ok_or_else(|| {
                ParseError::new(
                    format!("no type registered for integer suffix `{type_name}`"),
                    span,
                )
            })?;
            let val = parse_int_literal(entry, i).ok_or_else(|| {
                ParseError::new(format!("literal `{i}` does not fit in `{type_name}`"), span)
            })?;
            Ok((entry.type_id, entry.add_cell_fn, val))
        }
        Lit::Float(f) => {
            let type_name = if f.suffix().is_empty() {
                "f64"
            } else {
                f.suffix()
            };
            let entry = types.get(type_name).ok_or_else(|| {
                ParseError::new(
                    format!("no type registered for float suffix `{type_name}`"),
                    span,
                )
            })?;
            let val = parse_float_literal(entry, f).ok_or_else(|| {
                ParseError::new(format!("invalid `{type_name}` literal `{f}`"), span)
            })?;
            Ok((entry.type_id, entry.add_cell_fn, val))
        }
        Lit::Bool(b) => {
            let entry = types
                .get("bool")
                .ok_or_else(|| ParseError::new("bool not in TypeRegistry", span))?;
            Ok((
                entry.type_id,
                entry.add_cell_fn,
                Box::new(b.value) as Box<dyn Any>,
            ))
        }
        Lit::Str(s) => {
            let entry = types
                .get("String")
                .ok_or_else(|| ParseError::new("String not in TypeRegistry", span))?;
            Ok((
                entry.type_id,
                entry.add_cell_fn,
                Box::new(s.value()) as Box<dyn Any>,
            ))
        }
        _ => Err(ParseError::new(
            "unsupported literal kind in initializer",
            span,
        )),
    }
}

/// Builds a [`Method`] from parsed inputs, outputs, compiled segments, and call_dyn functions.
fn build_method(
    inputs: Vec<(String, CellId, TypeId)>,
    outputs: Vec<(String, CellId, TypeId)>,
    segments: Vec<DynSegment>,
    call_fns: Vec<CallDynFn>,
) -> Method {
    let input_ids: Vec<CellId> = inputs.iter().map(|(_, id, _)| *id).collect();
    let output_ids: Vec<CellId> = outputs.iter().map(|(_, id, _)| *id).collect();
    let input_types: Vec<TypeId> = inputs.iter().map(|(_, _, tid)| *tid).collect();
    let output_types: Vec<TypeId> = outputs.iter().map(|(_, _, tid)| *tid).collect();

    // Wrap each segment in RefCell: MethodFn is Fn (not FnMut), so interior mutability
    // is required to call call_dyn(&mut self) from an immutable closure reference.
    let cells: Vec<RefCell<DynSegment>> = segments.into_iter().map(RefCell::new).collect();

    let f =
        move |inputs_any: &[&dyn Any]| -> std::result::Result<Vec<Box<dyn Any>>, anyhow::Error> {
            let mut results = Vec::with_capacity(cells.len());
            for (cell, call_fn) in cells.iter().zip(call_fns.iter()) {
                let seg = &mut *cell.borrow_mut();
                results.push(call_fn(seg, inputs_any)?);
            }
            Ok(results)
        };

    Method::new(input_ids, output_ids, input_types, output_types, f)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TypeRegistry;
    use cel_parser::OpLookup;

    fn parser() -> PmParser {
        PmParser::new(TypeRegistry::new(), OpLookup::new())
    }

    #[test]
    fn parse_empty_sheet() {
        let _sheet = parser().parse_str("sheet empty {}").unwrap();
    }

    #[test]
    fn parse_cell_with_annotation_and_initializer() {
        let _sheet = parser()
            .parse_str("sheet s { cell width: f64 = 1920.0; }")
            .unwrap();
    }

    #[test]
    fn parse_cell_annotation_only_uses_default() {
        let _sheet = parser().parse_str("sheet s { cell area: f64; }").unwrap();
    }

    #[test]
    fn parse_cell_initializer_infers_type() {
        let _sheet = parser().parse_str("sheet s { cell mode = 0i32; }").unwrap();
    }

    #[test]
    fn parse_cell_unknown_type_is_error() {
        let result = parser().parse_str("sheet s { cell x: unknown_type; }");
        assert!(result.is_err());
        let err = result.err().expect("expected Err");
        let msg = err.message().to_lowercase();
        assert!(
            msg.contains("unknown type") || msg.contains("unknown_type"),
            "{msg}"
        );
    }

    #[test]
    fn parse_cell_missing_default_is_error() {
        #[derive(PartialEq, Clone)]
        struct NoDef(i32);
        let mut reg = TypeRegistry::new();
        reg.register_no_default::<NoDef>("NoDef");
        let mut p = PmParser::new(reg, OpLookup::new());
        let result = p.parse_str("sheet s { cell x: NoDef; }");
        assert!(result.is_err());
    }

    #[test]
    fn parse_multiple_cells() {
        let _sheet = parser()
            .parse_str(
                r#"
            sheet image_resize {
                cell width:  f64 = 1920.0;
                cell height: f64 = 1080.0;
                cell area:   f64;
                cell mode:   i32 = 0;
            }
        "#,
            )
            .unwrap();
    }

    #[test]
    fn parse_relationship_single_method() {
        let _sheet = parser()
            .parse_str(
                r#"
            sheet s {
                cell width:  f64 = 4.0;
                cell height: f64 = 3.0;
                cell area:   f64;
                relationship {
                    method [width, height] -> [area]   { width * height }
                    method [area, height]  -> [width]  { area / height }
                    method [width, area]   -> [height] { area / width }
                }
            }
        "#,
            )
            .unwrap();
    }

    #[test]
    fn parse_method_undeclared_input_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell x: f64 = 1.0;
                relationship { method [x, bogus] -> [x] { x } }
            }
        "#,
        );
        assert!(result.is_err());
        let err = result.err().expect("expected Err");
        let msg = err.message().to_lowercase();
        assert!(msg.contains("bogus") || msg.contains("undeclared"), "{msg}");
    }

    #[test]
    fn parse_method_output_type_mismatch_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell x: f64 = 0.0;
                cell n: i32 = 0;
                relationship { method [x] -> [n] { x } }
            }
        "#,
        );
        assert!(result.is_err(), "f64 body for i32 output must be an error");
    }

    #[test]
    fn parse_relationship_multi_output_tuple() {
        let _sheet = parser()
            .parse_str(
                r#"
            sheet s {
                cell a:    i32 = 3;
                cell b:    i32 = 4;
                cell sum:  i32;
                cell diff: i32;
                relationship { method [a, b] -> [sum, diff] { (a + b, a - b) } }
            }
        "#,
            )
            .unwrap();
    }

    #[test]
    fn parse_conditional_decl() {
        let _sheet = parser()
            .parse_str(
                r#"
            sheet image_resize {
                cell width:  f64 = 1920.0;
                cell height: f64 = 1080.0;
                cell ratio:  f64 = 1.0;
                cell mode:   i32 = 0;
                conditional mode {
                    0i32 => {
                        method [width] -> [height] { width }
                    },
                    1i32 => {
                        method [width, ratio] -> [height] { width * ratio }
                    },
                    _ => {
                        method [width] -> [height] { width }
                    },
                }
            }
        "#,
            )
            .unwrap();
    }

    #[test]
    fn conditional_undeclared_match_cell_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell x: i32 = 0;
                conditional bogus { 0i32 => { method [x] -> [x] { x } } }
            }
        "#,
        );
        assert!(result.is_err());
        let err = result.err().expect("expected Err");
        let msg = err.message().to_lowercase();
        assert!(msg.contains("bogus") || msg.contains("undeclared"), "{msg}");
    }

    #[test]
    fn conditional_branch_literal_type_mismatch_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell mode: i32 = 0;
                cell x:    f64 = 0.0;
                conditional mode { 1.0 => { method [x] -> [x] { x } } }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "float literal for i32 match cell must be an error"
        );
    }

    #[test]
    fn parse_cell_literal_type_mismatch_is_error() {
        // Float literal for an i32 cell should be a parse error.
        let result = parser().parse_str("sheet s { cell x: i32 = 1.0; }");
        assert!(
            result.is_err(),
            "float literal for i32 annotation must be an error"
        );
    }
}
