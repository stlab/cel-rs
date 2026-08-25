//! adam-lang parser — grammar productions and sheet construction.
//!
//! See the crate root's [`# Grammar`](crate#grammar) section for the full EBNF.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;

use adam_rs::{CellId, MatchExpr, Method, OutputId, RelationshipId, Requirement, Sheet};
use cel_parser::lex_lexer::{HasSpan, LexLexer, Token};
use cel_parser::{CELParser, OpLookup, ParseError};
use cel_runtime::DynSegment;
use proc_macro2::{Span, TokenStream};

use crate::TypeRegistry;
use crate::type_registry::{AddConditionalFn, CallDynFn, PushArgFn, TypeShape};

/// Parser result type.
pub type Result<T> = std::result::Result<T, ParseError>;

/// A parsed identifier's resolved `(CellId, TypeShape)`, paired with its source name — used both
/// for a construct's declared outputs and for the inputs [`AdamParser::parse_deduced_expr`]
/// deduces from whichever already-declared cells an expression references.
type NamedCells = Vec<(String, CellId, TypeShape)>;

// ---------------------------------------------------------------------------
// ParsedSheet
// ---------------------------------------------------------------------------

/// The result of [`AdamParser::parse_str`]: a live [`Sheet`] plus the declared
/// cell names, in source declaration order.
///
/// Derefs to [`Sheet`] so callers that only need sheet methods (e.g.
/// `propagate`) can use the result exactly as if it were a `Sheet`.
pub struct ParsedSheet {
    /// The constructed sheet.
    pub sheet: Sheet,
    /// Cell name → `(CellId, TypeShape)`, in declaration order.
    pub cell_names: IndexMap<String, (CellId, TypeShape)>,
    /// Output name → `OutputId`, in declaration order — parity with `cell_names`, for callers
    /// that need to look up `Sheet::output_valid`/`Sheet::violated_requirements` by name.
    pub output_names: IndexMap<String, OutputId>,
}

impl std::ops::Deref for ParsedSheet {
    type Target = Sheet;

    fn deref(&self) -> &Sheet {
        &self.sheet
    }
}

impl std::ops::DerefMut for ParsedSheet {
    fn deref_mut(&mut self) -> &mut Sheet {
        &mut self.sheet
    }
}

// ---------------------------------------------------------------------------
// ParseContext — mutable state for one parse_str call
// ---------------------------------------------------------------------------

struct ParseContext {
    cursor: crate::token_cursor::TokenCursor,
    sheet: Sheet,
    /// Maps cell name → (CellId, TypeShape), in declaration order, for method and
    /// conditional compilation and for exposing to callers via `ParsedSheet`.
    cell_names: IndexMap<String, (CellId, TypeShape)>,
    /// Maps output name → `OutputId`, in declaration order, for exposing to callers via
    /// `ParsedSheet`.
    output_names: IndexMap<String, OutputId>,
}

impl std::ops::Deref for ParseContext {
    type Target = crate::token_cursor::TokenCursor;

    fn deref(&self) -> &crate::token_cursor::TokenCursor {
        &self.cursor
    }
}

impl std::ops::DerefMut for ParseContext {
    fn deref_mut(&mut self) -> &mut crate::token_cursor::TokenCursor {
        &mut self.cursor
    }
}

// ---------------------------------------------------------------------------
// AdamParser
// ---------------------------------------------------------------------------

/// Parses adam-lang source strings into live [`ParsedSheet`]s (sheet + cell names).
///
/// # Example
///
/// ```rust
/// use adam_lang::{AdamParser, TypeRegistry};
/// use cel_parser::OpLookup;
///
/// let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
/// let parsed = parser.parse_str("sheet s { cell x: i32 = 0; }").unwrap();
/// ```
pub struct AdamParser {
    pub(crate) types: TypeRegistry,
    pub(crate) cel: CELParser,
}

impl AdamParser {
    /// Creates a parser with the given type registry and operation lookup.
    ///
    /// `op_lookup` is forwarded to the embedded [`CELParser`] when compiling method
    /// body expressions. See
    /// [`OpLookup::push_library_scope`](cel_parser::OpLookup::push_library_scope) for how to
    /// install a function library (e.g. `cel-std`) before parsing.
    pub fn new(types: TypeRegistry, op_lookup: OpLookup) -> Self {
        AdamParser {
            types,
            cel: CELParser::new(op_lookup),
        }
    }

    /// Returns a mutable reference to the embedded CEL operation lookup.
    pub fn op_lookup_mut(&mut self) -> &mut OpLookup {
        self.cel.op_lookup_mut()
    }

    /// Parses an adam-lang source string into a live [`ParsedSheet`].
    ///
    /// Resets internal parse state on each call.
    ///
    /// # Errors
    ///
    /// Returns `Err` on any syntax error, unknown type name, type mismatch between a
    /// cell annotation and its initializer, undeclared cell name in a `relationship` binding's
    /// output list, or a tuple arity/element-type mismatch between the output expression
    /// and its declared outputs.
    pub fn parse_str(&mut self, source: &str) -> Result<ParsedSheet> {
        let stream =
            TokenStream::from_str(source).map_err(|e| ParseError::from_lex_error(source, e))?;
        let mut ctx = ParseContext {
            cursor: crate::token_cursor::TokenCursor::new(
                LexLexer::new(stream.into_iter()).peekable(),
            ),
            sheet: Sheet::new(),
            cell_names: IndexMap::new(),
            output_names: IndexMap::new(),
        };
        let _ = ctx.consume_doc_comment_run(true); // sheet-level `//!` docs (ignored at runtime)
        self.parse_sheet(&mut ctx)?;
        if let Some(tok) = ctx.peek_token() {
            return Err(ParseError::new("unexpected token", tok.span()));
        }
        Ok(ParsedSheet {
            sheet: ctx.sheet,
            cell_names: ctx.cell_names,
            output_names: ctx.output_names,
        })
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

    /// `sheet_item = [ doc_comment ] (cell_decl | relationship_decl | conditional_decl | out_decl).`
    fn parse_sheet_item(&mut self, ctx: &mut ParseContext) -> Result<()> {
        let _ = ctx.consume_doc_comment_run(false); // outer `///` docs (ignored at runtime)
        match ctx.peek_token() {
            Some(Token::Identifier(id)) if id == "cell" => self.parse_cell_decl(ctx),
            Some(Token::Identifier(id)) if id == "relationship" => {
                self.parse_relationship_decl(ctx).map(|_| ())
            }
            Some(Token::Identifier(id)) if id == "conditional" => self.parse_conditional_decl(ctx),
            Some(Token::Identifier(id)) if id == "out" => self.parse_out_decl(ctx),
            Some(tok) => Err(ParseError::new(
                "expected `cell`, `relationship`, `conditional`, or `out`",
                tok.span(),
            )),
            None => Err(ParseError::new(
                "unexpected end of input",
                Span::call_site(),
            )),
        }
    }

    /// `cell_decl = "cell" identifier cell_type_init [ cell_filter ] ";".`
    ///
    /// `cell_type_init = (":" type_expr ["=" expression]) | ("=" expression).`
    fn parse_cell_decl(&mut self, ctx: &mut ParseContext) -> Result<()> {
        ctx.is_keyword("cell"); // consume
        let (name, name_span) = ctx.consume_ident()?;
        if ctx.cell_names.contains_key(&name) {
            return Err(ParseError::new(
                format!("duplicate cell `{name}`"),
                name_span,
            ));
        }

        let declared_shape: Option<TypeShape> = if ctx.consume_punct(":") {
            let type_expr = self.parse_type_expr(ctx)?;
            Some(
                self.types
                    .resolve(&type_expr)
                    .map_err(|(msg, span)| ParseError::new(msg, span))?,
            )
        } else {
            None
        };

        let has_initializer = ctx.consume_punct("=");
        let (shape, cell_id) = if has_initializer {
            let segment = self.parse_cel_expression(ctx)?;
            let (actual_shape, cell_id) = self.build_cell_from_segment(segment, ctx)?;
            if let Some(declared) = &declared_shape
                && declared != &actual_shape
            {
                return Err(ParseError::new(
                    format!(
                        "cell `{name}`: type mismatch: expected `{}`, got `{}`",
                        self.types.display_name(declared),
                        self.types.display_name(&actual_shape)
                    ),
                    name_span,
                ));
            }
            (actual_shape, cell_id)
        } else {
            let declared = declared_shape.ok_or_else(|| {
                ParseError::new("expected `:` or `=` in cell declaration", name_span)
            })?;
            let cell_id = self.build_default_cell(&declared, name_span, ctx)?;
            (declared, cell_id)
        };

        let filter = if ctx.is_keyword("filter") {
            Some(self.parse_cell_filter(ctx, &name, name_span, &shape)?)
        } else {
            None
        };

        ctx.expect_punct(";")?;
        ctx.cell_names.insert(name, (cell_id, shape));
        if let Some(filter) = filter {
            ctx.sheet
                .add_filter(cell_id, filter)
                .map_err(|e| ParseError::new(e.to_string(), name_span))?;
        }
        Ok(())
    }

    /// `cell_filter = "filter" expression.`
    ///
    /// Builds an [`adam_rs::Filter`] from a single deduced expression: `_` denotes the candidate
    /// value being conformed (of `declared_shape`'s type); every other identifier that names an
    /// already-declared cell is a deduced dependency, exactly as [`Self::parse_deduced_expr`]
    /// resolves them for a `relationship` binding or `out` declaration — see
    /// [`Self::parse_filter_expr`]. `declared_shape` is the filtered cell's own declared type,
    /// already resolved by the caller in [`parse_cell_decl`]. The filtered cell's own `CellId` is
    /// not needed here: the caller attaches the returned `Filter` to it afterwards, via
    /// `Sheet::add_filter`.
    ///
    /// # Errors
    /// Returns `Err` if `declared_shape` is a tuple (not yet supported by this builder), if an
    /// identifier inside the expression names neither `_` nor an already-declared cell, if `_` is
    /// never referenced, or if the expression's inferred type doesn't match `declared_shape`.
    ///
    /// - Complexity: O(m) in the number of distinct cell identifiers the expression references,
    ///   for this method's own bookkeeping (on top of the expression's own parse/compile cost).
    fn parse_cell_filter(
        &mut self,
        ctx: &mut ParseContext,
        cell_name: &str,
        cell_span: Span,
        declared_shape: &TypeShape,
    ) -> Result<adam_rs::Filter> {
        if matches!(declared_shape, TypeShape::Tuple(_)) {
            return Err(ParseError::new(
                format!("cell `{cell_name}`: filter on a tuple-typed cell is not yet supported"),
                cell_span,
            ));
        }

        let (segment, inputs, underscore_used) = self.parse_filter_expr(ctx, declared_shape)?;
        if !underscore_used {
            return Err(ParseError::new(
                "filter must reference `_` (the value being filtered)",
                cell_span,
            ));
        }

        let value_type_id = cell_type_id(declared_shape);
        let output_type_id = segment.peek_output_type_id().ok_or_else(|| {
            ParseError::new(
                format!("cell `{cell_name}`: filter produced no value"),
                cell_span,
            )
        })?;
        if output_type_id != value_type_id {
            return Err(ParseError::new(
                format!(
                    "cell `{cell_name}`: filter must produce `{}`",
                    self.types.display_name(declared_shape)
                ),
                cell_span,
            ));
        }

        // `call_dyn_fn` is the same monomorphized-per-registered-type dispatcher `build_method`/
        // `build_match_expr` already use for a deduced expression's scalar output.
        let call_fn = self
            .types
            .entry_by_type_id(value_type_id)
            .expect("declared cell type registered")
            .call_dyn_fn;

        let arg_ids: Vec<CellId> = inputs.iter().map(|(_, id, _)| *id).collect();
        let arg_type_ids: Vec<TypeId> = inputs
            .iter()
            .map(|(_, _, shape)| cell_type_id(shape))
            .collect();

        // `RefCell`, not a plain `move` capture: `call_fn` takes `&mut DynSegment`, unlike
        // `DynClosure::call_boxed`'s `&self` the old closure-literal path used.
        let segment = RefCell::new(segment);

        Ok(adam_rs::Filter::new(
            value_type_id,
            arg_ids,
            arg_type_ids,
            move |value, args| {
                let mut call_args: Vec<&dyn Any> = Vec::with_capacity(1 + args.len());
                call_args.push(value);
                call_args.extend_from_slice(args);
                call_fn(&mut segment.borrow_mut(), &call_args)
            },
        ))
    }

    /// Evaluates `segment` eagerly with no inputs, inferring its result's `TypeShape` from the
    /// segment's own tuple stack info (read *before* consuming the segment) for a tuple result,
    /// or from `peek_output_type_id` for a scalar result. Returns the result boxed (`Box<dyn
    /// Any>` holding either a scalar `T` or a `DynamicSequence`) — adding it to a `Sheet` or
    /// using it as a conditional branch key is the caller's job (see
    /// [`build_cell_from_segment`](Self::build_cell_from_segment)).
    ///
    /// - Precondition: `segment` requires no pre-loaded arguments (a `cell` initializer never
    ///   has an input-cell scope pushed, unlike a `relationship` binding's, `out` declaration's, or
    ///   `require`ment's body).
    ///
    /// # Errors
    /// Returns `Err` if the segment's result type isn't registered (scalar case) or contains an
    /// unregistered leaf type at any nesting depth (tuple case).
    fn eval_segment_boxed(&self, mut segment: DynSegment) -> Result<(TypeShape, Box<dyn Any>)> {
        if segment.peek_tuple_arity().is_some() {
            let associated = segment.peek_stack_infos(1)[0].associated.clone();
            let shape = self
                .shape_of_associated(&associated)
                .map_err(|msg| ParseError::new(msg, Span::call_site()))?;
            let leaf = |type_id: TypeId| self.types.element_descriptor(type_id);
            let seq = segment
                .call_dyn_as_dynamic_sequence(&[], &leaf)
                .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
            Ok((shape, Box::new(seq) as Box<dyn Any>))
        } else {
            let type_id = segment.peek_output_type_id().ok_or_else(|| {
                ParseError::new("expression produced no value", Span::call_site())
            })?;
            let entry = self.types.entry_by_type_id(type_id).ok_or_else(|| {
                ParseError::new(
                    "cannot infer a type for this expression; register a type name for it or \
                     add an explicit `: type_expr` annotation",
                    Span::call_site(),
                )
            })?;
            let boxed = (entry.call_dyn_fn)(&mut segment, &[])
                .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
            Ok((TypeShape::Named(type_id), boxed))
        }
    }

    /// Evaluates `segment` via [`eval_segment_boxed`](Self::eval_segment_boxed) and adds a
    /// matching cell to `ctx.sheet`, using the registered `add_cell_fn` for a `TypeShape::Named`
    /// result, or `Sheet::add_cell::<DynamicSequence>` directly for a `TypeShape::Tuple` result
    /// (tuple-typed cells are never themselves registered in `TypeRegistry` — every distinct
    /// shape shares the one concrete storage type, `DynamicSequence`).
    ///
    /// - Precondition: see [`eval_segment_boxed`](Self::eval_segment_boxed).
    ///
    /// # Errors
    /// See [`eval_segment_boxed`](Self::eval_segment_boxed).
    fn build_cell_from_segment(
        &self,
        segment: DynSegment,
        ctx: &mut ParseContext,
    ) -> Result<(TypeShape, CellId)> {
        let (shape, boxed) = self.eval_segment_boxed(segment)?;
        let cell_id = match &shape {
            TypeShape::Named(type_id) => {
                let entry = self.types.entry_by_type_id(*type_id).expect("registered");
                (entry.add_cell_fn)(&mut ctx.sheet, boxed)
            }
            TypeShape::Tuple(_) => {
                let seq = *boxed
                    .downcast::<cel_runtime::DynamicSequence>()
                    .expect("eval_segment_boxed: a Tuple shape always boxes a DynamicSequence");
                ctx.sheet.add_cell(seq)
            }
        };
        Ok((shape, cell_id))
    }

    /// Recursively converts a live tuple's `AssociatedType` shape into a `TypeShape`, by looking
    /// up each leaf's `TypeId` against `self.types`.
    ///
    /// # Errors
    /// Returns an error naming any element's `TypeId` (at any nesting depth) that isn't
    /// registered.
    fn shape_of_associated(
        &self,
        associated: &[cel_runtime::AssociatedType],
    ) -> std::result::Result<TypeShape, String> {
        let elements = associated
            .iter()
            .map(|elem| {
                if elem.type_id == TypeId::of::<cel_runtime::DynTuple>() {
                    self.shape_of_associated(&elem.associated)
                } else {
                    self.types
                        .entry_by_type_id(elem.type_id)
                        .map(|entry| TypeShape::Named(entry.type_id))
                        .ok_or_else(|| format!("unregistered type `{}`", elem.type_name))
                }
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(TypeShape::Tuple(elements))
    }

    /// Builds a default-valued cell for `shape` (scalar or tuple, recursively), adding it to
    /// `ctx.sheet`.
    ///
    /// # Errors
    /// Returns `Err` naming the type/leaf that has no registered default.
    fn build_default_cell(
        &self,
        shape: &TypeShape,
        span: Span,
        ctx: &mut ParseContext,
    ) -> Result<CellId> {
        match shape {
            TypeShape::Named(type_id) => {
                let entry = self
                    .types
                    .entry_by_type_id(*type_id)
                    .expect("build_default_cell: type registered (resolved via TypeRegistry)");
                let default_fn = entry.default_fn.ok_or_else(|| {
                    ParseError::new(
                        format!("type `{}` has no default; provide `= ...`", entry.type_name),
                        span,
                    )
                })?;
                Ok((entry.add_cell_fn)(&mut ctx.sheet, default_fn()))
            }
            TypeShape::Tuple(_) => {
                let seq = self
                    .types
                    .default_dynamic_sequence(shape)
                    .map_err(|msg| ParseError::new(msg, span))?;
                Ok(ctx.sheet.add_cell(seq))
            }
        }
    }

    /// `type_expr = identifier | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".`
    ///
    /// `()` is the empty tuple type (0 elements); `(T)` is grouping (same as bare `T`); `(T,)`
    /// is a 1-element tuple; `(T, U, ...)` is n-element, no trailing comma.
    fn parse_type_expr(&mut self, ctx: &mut ParseContext) -> Result<crate::ast::TypeExpr> {
        if matches!(ctx.peek_token(), Some(Token::Identifier(_))) {
            let (name, span) = ctx.consume_ident()?;
            return Ok(crate::ast::TypeExpr::Named(name, point(span)));
        }

        let open_span = ctx.expect_open_paren()?;
        if ctx.at_close_paren() {
            let close_span = ctx.expect_close_paren()?;
            return Ok(crate::ast::TypeExpr::Tuple(
                Vec::new(),
                crate::ast::ExprSpan {
                    start: open_span,
                    end: close_span,
                },
            ));
        }

        let first = self.parse_type_expr(ctx)?;
        if ctx.at_close_paren() {
            // Grouping: exactly one type, no comma.
            ctx.expect_close_paren()?;
            return Ok(first);
        }
        if !ctx.consume_punct(",") {
            return Err(ctx.err_at("expected ',' or closing parenthesis"));
        }
        if ctx.at_close_paren() {
            // Single element + trailing comma: 1-tuple.
            let close_span = ctx.expect_close_paren()?;
            return Ok(crate::ast::TypeExpr::Tuple(
                vec![first],
                crate::ast::ExprSpan {
                    start: open_span,
                    end: close_span,
                },
            ));
        }
        let mut elements = vec![first];
        loop {
            elements.push(self.parse_type_expr(ctx)?);
            if ctx.at_close_paren() {
                break;
            }
            if !ctx.consume_punct(",") {
                return Err(ctx.err_at("expected ',' or closing parenthesis"));
            }
        }
        let close_span = ctx.expect_close_paren()?;
        Ok(crate::ast::TypeExpr::Tuple(
            elements,
            crate::ast::ExprSpan {
                start: open_span,
                end: close_span,
            },
        ))
    }

    /// `relationship_decl = "relationship" "{" { binding } "}".`
    ///
    /// - Postcondition: the returned `RelationshipId` identifies the relationship just added to
    ///   `ctx.sheet`.
    fn parse_relationship_decl(&mut self, ctx: &mut ParseContext) -> Result<RelationshipId> {
        ctx.is_keyword("relationship"); // consume
        ctx.expect_open_brace()?;
        let mut methods = Vec::new();
        while !ctx.at_close_brace() {
            methods.push(self.parse_binding(ctx)?);
        }
        ctx.expect_close_brace()?;
        ctx.sheet
            .add_relationship(methods)
            .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))
    }

    /// `binding = binding_target ":=" expression ";".`
    fn parse_binding(&mut self, ctx: &mut ParseContext) -> Result<Method> {
        let (names, destructure) = parse_binding_target(ctx)?;
        let mut outputs: NamedCells = Vec::with_capacity(names.len());
        for (name, span) in names {
            let (cell_id, shape) = ctx
                .cell_names
                .get(&name)
                .cloned()
                .ok_or_else(|| ParseError::new(format!("undeclared cell `{name}`"), span))?;
            outputs.push((name, cell_id, shape));
        }
        ctx.expect_punct(":=")?;
        let (segment, inputs) = self.parse_deduced_expr(ctx)?;
        ctx.expect_punct(";")?;
        let compiled = self.compile_outputs(ctx, &segment, &outputs, destructure)?;
        Ok(build_method(inputs, outputs, segment, compiled))
    }

    /// Parses an `expression` whose input cells are deduced from whichever already-declared
    /// cell identifiers it references, rather than an explicit `cell_list` — the mechanism
    /// shared by a conditional's match-subject expression ([`Self::parse_match_expr`]), a
    /// `relationship` binding's right-hand side, an `out` declaration's initializer, and a
    /// `require`ment body.
    ///
    /// Each 0-arity identifier lookup that names an already-declared cell is assigned the
    /// next argument index on first reference within this expression and reuses it on repeat
    /// reference (e.g. `a && a` allocates one argument slot, not two), via a scope pushed
    /// onto the CEL operation lookup for the duration of this parse.
    ///
    /// # Errors
    /// Returns `Err` if the expression fails to parse.
    ///
    /// - Complexity: O(k) in the number of distinct cell identifiers referenced, for this
    ///   method's own bookkeeping (on top of `cel-parser`'s own parse cost).
    fn parse_deduced_expr(&mut self, ctx: &mut ParseContext) -> Result<(DynSegment, NamedCells)> {
        // Precompute how to push each currently-declared cell, keyed by name. Built before
        // the scope closure captures anything, since `push_scope` requires `'static` (the
        // closure can't borrow `self.types`).
        let push_table: std::collections::HashMap<String, (CellId, TypeShape, InputPush)> = ctx
            .cell_names
            .iter()
            .map(|(name, (cell_id, shape))| {
                let push = match shape {
                    TypeShape::Named(type_id) => InputPush::Scalar(
                        self.types
                            .entry_by_type_id(*type_id)
                            .expect("declared cell type registered")
                            .push_arg_fn,
                    ),
                    TypeShape::Tuple(_) => InputPush::Tuple(self.types.associated_prototype(shape)),
                };
                (name.clone(), (*cell_id, shape.clone(), push))
            })
            .collect();

        let accumulator: Arc<Mutex<NamedCells>> = Arc::new(Mutex::new(Vec::new()));
        let scope_accumulator = Arc::clone(&accumulator);

        self.cel
            .op_lookup_mut()
            .push_scope(move |name, segment, arity, _span| {
                if arity != 0 {
                    return Ok(false);
                }
                let Some((cell_id, shape, push)) = push_table.get(name) else {
                    return Ok(false);
                };
                let idx = {
                    let mut acc = scope_accumulator.lock().expect("scope mutex not poisoned");
                    match acc.iter().position(|(n, ..)| n == name) {
                        Some(pos) => pos,
                        None => {
                            acc.push((name.to_string(), *cell_id, shape.clone()));
                            acc.len() - 1
                        }
                    }
                };
                match push {
                    InputPush::Scalar(fn_ptr) => fn_ptr(segment, idx),
                    InputPush::Tuple(associated) => {
                        segment.push_arg_as_dynamic_sequence_tuple(idx, associated.clone())
                    }
                }
                Ok(true)
            });

        let result = self.parse_cel_expression(ctx);
        self.cel.op_lookup_mut().pop_scope();
        let segment = result?;

        let inputs = accumulator
            .lock()
            .expect("scope mutex not poisoned")
            .clone();
        Ok((segment, inputs))
    }

    /// Parses a `filter` clause's body expression, deducing its dependencies exactly as
    /// [`Self::parse_deduced_expr`] does, plus one reserved identifier: `_` always resolves to
    /// argument slot 0 (the candidate value being conformed, of `declared_shape`'s type), ahead
    /// of any cell-derived slots, which start at slot 1. Returns whether `_` was referenced at
    /// least once, alongside the compiled segment and its deduced cell inputs — the caller
    /// decides whether that occurrence count is acceptable.
    ///
    /// # Errors
    /// Returns `Err` if the expression fails to parse.
    ///
    /// - Complexity: O(k) in the number of distinct cell identifiers referenced, for this
    ///   method's own bookkeeping (on top of `cel-parser`'s own parse cost).
    fn parse_filter_expr(
        &mut self,
        ctx: &mut ParseContext,
        declared_shape: &TypeShape,
    ) -> Result<(DynSegment, NamedCells, bool)> {
        let push_table: std::collections::HashMap<String, (CellId, TypeShape, InputPush)> = ctx
            .cell_names
            .iter()
            .map(|(name, (cell_id, shape))| {
                let push = match shape {
                    TypeShape::Named(type_id) => InputPush::Scalar(
                        self.types
                            .entry_by_type_id(*type_id)
                            .expect("declared cell type registered")
                            .push_arg_fn,
                    ),
                    TypeShape::Tuple(_) => InputPush::Tuple(self.types.associated_prototype(shape)),
                };
                (name.clone(), (*cell_id, shape.clone(), push))
            })
            .collect();

        let value_push = match declared_shape {
            TypeShape::Named(type_id) => InputPush::Scalar(
                self.types
                    .entry_by_type_id(*type_id)
                    .expect("declared cell type registered")
                    .push_arg_fn,
            ),
            TypeShape::Tuple(_) => {
                InputPush::Tuple(self.types.associated_prototype(declared_shape))
            }
        };

        let accumulator: Arc<Mutex<NamedCells>> = Arc::new(Mutex::new(Vec::new()));
        let scope_accumulator = Arc::clone(&accumulator);
        let underscore_used: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let scope_underscore_used = Arc::clone(&underscore_used);

        self.cel
            .op_lookup_mut()
            .push_scope(move |name, segment, arity, _span| {
                if arity != 0 {
                    return Ok(false);
                }
                if name == "_" {
                    *scope_underscore_used
                        .lock()
                        .expect("scope mutex not poisoned") = true;
                    match &value_push {
                        InputPush::Scalar(fn_ptr) => fn_ptr(segment, 0),
                        InputPush::Tuple(associated) => {
                            segment.push_arg_as_dynamic_sequence_tuple(0, associated.clone())
                        }
                    }
                    return Ok(true);
                }
                let Some((cell_id, shape, push)) = push_table.get(name) else {
                    return Ok(false);
                };
                let idx = {
                    let mut acc = scope_accumulator.lock().expect("scope mutex not poisoned");
                    match acc.iter().position(|(n, ..)| n == name) {
                        Some(pos) => pos + 1,
                        None => {
                            acc.push((name.to_string(), *cell_id, shape.clone()));
                            acc.len()
                        }
                    }
                };
                match push {
                    InputPush::Scalar(fn_ptr) => fn_ptr(segment, idx),
                    InputPush::Tuple(associated) => {
                        segment.push_arg_as_dynamic_sequence_tuple(idx, associated.clone())
                    }
                }
                Ok(true)
            });

        let result = self.parse_cel_expression(ctx);
        self.cel.op_lookup_mut().pop_scope();
        let segment = result?;

        let inputs = accumulator
            .lock()
            .expect("scope mutex not poisoned")
            .clone();
        let used = *underscore_used.lock().expect("scope mutex not poisoned");
        Ok((segment, inputs, used))
    }

    /// Compiles a conditional's match-subject expression — a bare identifier (`mode`) is the
    /// degenerate single-cell case; anything more (`a && b`) draws on however many
    /// already-declared cells it references, via [`Self::parse_deduced_expr`].
    ///
    /// `match_span` is used to report errors raised by this method or the shape inference it
    /// delegates to; the caller already has it (from before parsing the expression) for its own
    /// error reporting, so it's threaded through rather than recomputed.
    ///
    /// # Errors
    /// Returns `Err` if the expression fails to parse, produced no value, or (for a `Named`
    /// output shape) its type isn't registered in the `TypeRegistry`.
    fn parse_match_expr(
        &mut self,
        ctx: &mut ParseContext,
        match_span: proc_macro2::Span,
    ) -> Result<(TypeShape, MatchExpr)> {
        let (segment, inputs) = self.parse_deduced_expr(ctx)?;
        self.build_match_expr(segment, inputs, match_span)
    }

    /// Builds a `(TypeShape, MatchExpr)` from a compiled match-expression segment and its
    /// deduced input cells, dispatching on the segment's inferred output shape — mirrors
    /// `build_method`'s single-output dispatch (`CompiledOutputs::Single`/`SingleTuple`), but
    /// for a match value read repeatedly across `propagate()` calls rather than written once
    /// per method call.
    ///
    /// - Precondition: `segment` was compiled with no pre-loaded arguments (`push_arg`-based),
    ///   matching every input in `inputs` by index.
    ///
    /// # Errors
    /// Returns `Err` if the segment produced no value, or (`Named` shape only) if its output
    /// type isn't registered in the `TypeRegistry`.
    fn build_match_expr(
        &self,
        segment: DynSegment,
        inputs: NamedCells,
        match_span: proc_macro2::Span,
    ) -> Result<(TypeShape, MatchExpr)> {
        let input_ids: Vec<CellId> = inputs.iter().map(|(_, id, _)| *id).collect();
        let input_types: Vec<TypeId> = inputs
            .iter()
            .map(|(_, _, shape)| cell_type_id(shape))
            .collect();

        if segment.peek_tuple_arity().is_some() {
            let associated = segment.peek_stack_infos(1)[0].associated.clone();
            let shape = self
                .shape_of_associated(&associated)
                .map_err(|msg| ParseError::new(msg, match_span))?;
            let table = self.types.element_descriptors_for(&shape);
            let segment = RefCell::new(segment);

            let function =
                move |args: &[&dyn Any]| -> std::result::Result<Box<dyn Any>, anyhow::Error> {
                    let leaf = |type_id: TypeId| {
                        table
                            .iter()
                            .find(|(tid, ..)| *tid == type_id)
                            .map(|(_, d, c, e, dbg)| (*d, *c, *e, *dbg))
                    };
                    let seq = segment
                        .borrow_mut()
                        .call_dyn_as_dynamic_sequence(args, &leaf)?;
                    Ok(Box::new(seq) as Box<dyn Any>)
                };

            fn dynamic_sequence_eq(a: &dyn Any, b: &dyn Any) -> bool {
                a.downcast_ref::<cel_runtime::DynamicSequence>()
                    == b.downcast_ref::<cel_runtime::DynamicSequence>()
            }

            let match_expr = MatchExpr::new(
                input_ids,
                input_types,
                TypeId::of::<cel_runtime::DynamicSequence>(),
                dynamic_sequence_eq,
                function,
            );
            Ok((shape, match_expr))
        } else {
            let type_id = segment
                .peek_output_type_id()
                .ok_or_else(|| ParseError::new("match expression produced no value", match_span))?;
            let entry = self.types.entry_by_type_id(type_id).ok_or_else(|| {
                ParseError::new("match expression type not in TypeRegistry", match_span)
            })?;
            let call_fn = entry.call_dyn_fn;
            let eq_fn = entry.eq_dyn_fn;
            let segment = RefCell::new(segment);

            let function =
                move |args: &[&dyn Any]| -> std::result::Result<Box<dyn Any>, anyhow::Error> {
                    call_fn(&mut segment.borrow_mut(), args)
                };

            let match_expr = MatchExpr::new(input_ids, input_types, type_id, eq_fn, function);
            Ok((TypeShape::Named(type_id), match_expr))
        }
    }

    /// `conditional_decl = "conditional" expression "{" { conditional_branch } "}".`
    fn parse_conditional_decl(&mut self, ctx: &mut ParseContext) -> Result<()> {
        ctx.is_keyword("conditional"); // consume
        let match_span = ctx.peek_span();
        let (match_shape, match_expr) = self.parse_match_expr(ctx, match_span)?;
        ctx.expect_open_brace()?;

        let mut branches: Vec<(Box<dyn Any>, Vec<RelationshipId>)> = Vec::new();
        let mut default_rel_ids: Vec<RelationshipId> = Vec::new();

        while !ctx.at_close_brace() {
            // Check for default branch `_ => { ... }`
            if matches!(ctx.peek_token(), Some(Token::Identifier(id)) if id == "_") {
                ctx.advance(); // consume `_`
                ctx.expect_punct("=>")?;
                ctx.expect_open_brace()?;
                let rel_ids = self.parse_branch_relationships(ctx)?;
                ctx.expect_close_brace()?;
                ctx.consume_punct(",");
                default_rel_ids = rel_ids;
                break; // default branch is always last
            }

            // Named branch: `expression "=>" "{" ... "}"` — an expression covers both a
            // bare literal (`0i32 =>`) and a tuple value (`(0, 0) =>`) via the same grammar cell
            // initializers already use.
            let branch_span = ctx.peek_span();
            let segment = self.parse_cel_expression(ctx)?;
            let (branch_shape, branch_val) = self.eval_segment_boxed(segment)?;
            if branch_shape != match_shape {
                return Err(ParseError::new(
                    format!(
                        "conditional branch: type mismatch: expected `{}`, got `{}`",
                        self.types.display_name(&match_shape),
                        self.types.display_name(&branch_shape)
                    ),
                    branch_span,
                ));
            }
            ctx.expect_punct("=>")?;
            ctx.expect_open_brace()?;
            let rel_ids = self.parse_branch_relationships(ctx)?;
            ctx.expect_close_brace()?;
            ctx.consume_punct(",");
            branches.push((branch_val, rel_ids));
        }
        ctx.expect_close_brace()?;

        match &match_shape {
            TypeShape::Named(type_id) => {
                let add_cond_fn: AddConditionalFn = self
                    .types
                    .entry_by_type_id(*type_id)
                    .ok_or_else(|| {
                        ParseError::new("match cell type not in TypeRegistry", match_span)
                    })?
                    .add_conditional_fn;
                add_cond_fn(&mut ctx.sheet, match_expr, branches, default_rel_ids)
                    .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
            }
            TypeShape::Tuple(_) => {
                let typed_branches: Vec<(Vec<cel_runtime::DynamicSequence>, Vec<RelationshipId>)> =
                    branches
                        .into_iter()
                        .map(|(val, rel_ids)| {
                            let seq = *val.downcast::<cel_runtime::DynamicSequence>().expect(
                                "eval_segment_boxed: a Tuple shape always boxes a \
                                     DynamicSequence",
                            );
                            (vec![seq], rel_ids)
                        })
                        .collect();
                ctx.sheet
                    .add_conditional::<cel_runtime::DynamicSequence>(
                        match_expr,
                        typed_branches,
                        default_rel_ids,
                    )
                    .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
            }
        }

        Ok(())
    }

    /// Parses one `conditional_branch`/`default_branch`'s shared body: `"{" { relationship_decl }
    /// "}"`, up to (not including) the closing `}`.
    fn parse_branch_relationships(
        &mut self,
        ctx: &mut ParseContext,
    ) -> Result<Vec<RelationshipId>> {
        let mut rel_ids = Vec::new();
        while !ctx.at_close_brace() {
            if !matches!(ctx.peek_token(), Some(Token::Identifier(id)) if id == "relationship") {
                return Err(ctx.err_at("expected `relationship`"));
            }
            rel_ids.push(self.parse_relationship_decl(ctx)?);
        }
        Ok(rel_ids)
    }

    /// `out_decl = "out" identifier [ ":" type_expr ] ":=" expression [ "require" "{" {
    /// requirement } "}" ] ";".`
    fn parse_out_decl(&mut self, ctx: &mut ParseContext) -> Result<()> {
        ctx.is_keyword("out"); // consume
        let (name, name_span) = ctx.consume_ident()?;
        if ctx.cell_names.contains_key(&name) {
            return Err(ParseError::new(
                format!("duplicate cell `{name}`"),
                name_span,
            ));
        }

        let declared_shape: Option<TypeShape> = if ctx.consume_punct(":") {
            let type_expr = self.parse_type_expr(ctx)?;
            Some(
                self.types
                    .resolve(&type_expr)
                    .map_err(|(msg, span)| ParseError::new(msg, span))?,
            )
        } else {
            None
        };

        ctx.expect_punct(":=")?;
        let (segment, inputs) = self.parse_deduced_expr(ctx)?;

        // Unlike a `cell` initializer's segment (zero-argument, safe to evaluate once eagerly
        // via `eval_segment_boxed`/`build_cell_from_segment`), an `out` writer's segment takes
        // real cell inputs (via `push_arg`) and must stay live for repeated re-evaluation by the
        // `Method` built below on every `Sheet::propagate` — so only its *shape* is inferred
        // here, from stack info, never actually executed.
        let actual_shape = if segment.peek_tuple_arity().is_some() {
            let associated = segment.peek_stack_infos(1)[0].associated.clone();
            self.shape_of_associated(&associated)
                .map_err(|msg| ctx.err_at(msg))?
        } else {
            let type_id = segment
                .peek_output_type_id()
                .ok_or_else(|| ctx.err_at(format!("out `{name}`: expression produced no value")))?;
            if self.types.entry_by_type_id(type_id).is_none() {
                return Err(ctx.err_at(format!(
                    "out `{name}`: cannot infer a type for this expression; register a type \
                     name for it or add an explicit `: type_expr` annotation"
                )));
            }
            TypeShape::Named(type_id)
        };

        let out_shape = match &declared_shape {
            Some(declared) => {
                if declared != &actual_shape {
                    return Err(ctx.err_at(format!(
                        "out `{name}`: type mismatch: expected `{}`, got `{}`",
                        self.types.display_name(declared),
                        self.types.display_name(&actual_shape)
                    )));
                }
                declared.clone()
            }
            None => actual_shape,
        };

        let cell_id = self.build_default_cell(&out_shape, name_span, ctx)?;
        ctx.cell_names
            .insert(name.clone(), (cell_id, out_shape.clone()));

        let compiled = match &out_shape {
            TypeShape::Named(type_id) => {
                let call_fn = self
                    .types
                    .entry_by_type_id(*type_id)
                    .expect("output cell type registered")
                    .call_dyn_fn;
                CompiledOutputs::Single(call_fn)
            }
            TypeShape::Tuple(_) => {
                CompiledOutputs::SingleTuple(self.types.element_descriptors_for(&out_shape))
            }
        };
        let writer = build_method(
            inputs,
            vec![(name.clone(), cell_id, out_shape)],
            segment,
            compiled,
        );

        let mut requirement_names: Vec<String> = Vec::new();
        let mut requirements: Vec<Requirement> = Vec::new();
        if ctx.is_keyword("require") {
            ctx.expect_open_brace()?;
            while !ctx.at_close_brace() {
                let (req_name, requirement) = self.parse_requirement(ctx)?;
                requirement_names.push(req_name);
                requirements.push(requirement);
            }
            ctx.expect_close_brace()?;
        }

        ctx.expect_punct(";")?;

        let named_requirements: Vec<(&str, Requirement)> = requirement_names
            .iter()
            .map(String::as_str)
            .zip(requirements)
            .collect();

        let output_id = ctx
            .sheet
            .add_output(writer, named_requirements)
            .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
        ctx.output_names.insert(name, output_id);

        Ok(())
    }

    /// `requirement = identifier ":" expression ";".`
    fn parse_requirement(&mut self, ctx: &mut ParseContext) -> Result<(String, Requirement)> {
        let (name, _name_span) = ctx.consume_ident()?;
        ctx.expect_punct(":")?;
        let (segment, inputs) = self.parse_deduced_expr(ctx)?;
        ctx.expect_punct(";")?;

        let bool_type_id = TypeId::of::<bool>();
        let actual_type_id = segment.peek_output_type_id().ok_or_else(|| {
            ctx.err_at(format!(
                "requirement `{name}`: expression produced no value"
            ))
        })?;
        if actual_type_id != bool_type_id {
            let got = self
                .types
                .entry_by_type_id(actual_type_id)
                .map(|e| e.type_name)
                .unwrap_or("?");
            return Err(ctx.err_at(format!(
                "requirement `{name}`: expected `bool`, got `{got}`"
            )));
        }

        let call_fn = self
            .types
            .get("bool")
            .expect("bool always registered")
            .call_dyn_fn;
        let input_ids: Vec<CellId> = inputs.iter().map(|(_, id, _)| *id).collect();
        let input_types: Vec<TypeId> = inputs
            .iter()
            .map(|(_, _, shape)| cell_type_id(shape))
            .collect();
        let segment = RefCell::new(segment);
        let requirement = Requirement::new(input_ids, input_types, move |args| {
            let seg = &mut *segment.borrow_mut();
            let boxed = call_fn(seg, args)?;
            Ok(*boxed
                .downcast::<bool>()
                .expect("checked TypeId::of::<bool>() above"))
        });

        Ok((name, requirement))
    }

    /// Determines how to split a compiled body segment's result across `outputs`, given their
    /// declared shapes — used by `parse_binding` to dispatch a `relationship` binding's direct-bind vs.
    /// destructuring cases against a single compiled `expression`. Written generically so any
    /// future N-output construct can reuse it; `out` declarations are always single-output,
    /// never destructuring, and currently use their own simpler, separate dispatch instead.
    ///
    /// A non-destructuring single output (`destructure` false; always `outputs.len() == 1`)
    /// takes the segment's single result directly (scalar via `call_dyn`, tuple-typed via
    /// `call_dyn_as_dynamic_sequence`, or the trivial empty-tuple case). A destructuring binding
    /// (`destructure` true, one or more outputs — see `ast::BindingDecl::destructure`) requires
    /// the result to be a tuple of matching arity and element shapes, split element-wise via
    /// `call_dyn_tuple_mixed`.
    ///
    /// # Errors
    /// Returns `Err` if any output's declared shape doesn't structurally match the body's actual
    /// result (scalar type mismatch, tuple arity mismatch, or tuple element shape mismatch, at
    /// any nesting depth), or if a single non-destructuring scalar/empty-tuple output's
    /// expression produced no value.
    fn compile_outputs(
        &self,
        ctx: &mut ParseContext,
        segment: &DynSegment,
        outputs: &[(String, CellId, TypeShape)],
        destructure: bool,
    ) -> Result<CompiledOutputs> {
        if outputs.len() == 1 && !destructure {
            let (out_name, _, out_shape) = &outputs[0];
            match out_shape {
                TypeShape::Named(out_type_id) => {
                    let actual_type_id = segment.peek_output_type_id().ok_or_else(|| {
                        ctx.err_at(format!("output `{out_name}`: expression produced no value"))
                    })?;
                    if actual_type_id != *out_type_id {
                        let expected = self.types.display_name(out_shape);
                        let got = self
                            .types
                            .entry_by_type_id(actual_type_id)
                            .map(|e| e.type_name.to_string())
                            .unwrap_or_else(|| "?".to_string());
                        return Err(ctx.err_at(format!(
                            "output `{out_name}`: type mismatch: expected `{expected}`, got `{got}`"
                        )));
                    }
                    let call_fn = self
                        .types
                        .entry_by_type_id(*out_type_id)
                        .expect("registered")
                        .call_dyn_fn;
                    Ok(CompiledOutputs::Single(call_fn))
                }
                TypeShape::Tuple(elements) if elements.is_empty() => {
                    // () is CEL's concrete unit type, a distinct leaf TypeId -- not DynTuple.
                    let actual_type_id = segment.peek_output_type_id().ok_or_else(|| {
                        ctx.err_at(format!("output `{out_name}`: expression produced no value"))
                    })?;
                    if actual_type_id != TypeId::of::<()>() {
                        return Err(ctx.err_at(format!(
                            "output `{out_name}`: type mismatch: expected `()`, got a non-`()` \
                             value"
                        )));
                    }
                    Ok(CompiledOutputs::EmptyTuple)
                }
                TypeShape::Tuple(_) => {
                    let stack_info = segment.peek_stack_infos(1).first();
                    let matches = stack_info.is_some_and(|info| {
                        tuple_shape_matches_associated(out_shape, &info.associated)
                    });
                    if !matches {
                        let actual = stack_info
                            .and_then(|info| self.shape_of_associated(&info.associated).ok())
                            .map(|s| self.types.display_name(&s))
                            .unwrap_or_else(|| "a non-matching value".to_string());
                        return Err(ctx.err_at(format!(
                            "output `{out_name}`: type mismatch: expected `{}`, got `{actual}`",
                            self.types.display_name(out_shape)
                        )));
                    }
                    Ok(CompiledOutputs::SingleTuple(
                        self.types.element_descriptors_for(out_shape),
                    ))
                }
            }
        } else {
            let arity = segment.peek_tuple_arity().unwrap_or(0);
            if arity != outputs.len() {
                return Err(ctx.err_at(format!(
                    "output expression has arity {arity} but {} output(s) declared",
                    outputs.len()
                )));
            }
            let associated = segment.peek_stack_infos(1)[0].associated.clone();
            let mut extractors = Vec::with_capacity(outputs.len());
            for (i, ((out_name, _, out_shape), elem)) in outputs.iter().zip(&associated).enumerate()
            {
                if !element_shape_matches(out_shape, elem) {
                    return Err(ctx.err_at(format!(
                        "output {i} `{out_name}`: type mismatch: expected `{}`, got `{}`",
                        self.types.display_name(out_shape),
                        elem.type_name
                    )));
                }
                extractors.push(match out_shape {
                    TypeShape::Named(type_id) => {
                        let entry = self.types.entry_by_type_id(*type_id).expect("registered");
                        cel_runtime::DynExtractor::Scalar(*type_id, entry.extract_box_fn)
                    }
                    TypeShape::Tuple(_) => {
                        let table = self.types.element_descriptors_for(out_shape);
                        cel_runtime::DynExtractor::Tuple(Box::new(move |type_id: TypeId| {
                            table
                                .iter()
                                .find(|(tid, ..)| *tid == type_id)
                                .map(|(_, d, c, e, dbg)| (*d, *c, *e, *dbg))
                        }))
                    }
                });
            }
            Ok(CompiledOutputs::Tuple(extractors))
        }
    }

    /// Delegates one `expression` to CELParser, sharing the token stream.
    fn parse_cel_expression(&mut self, ctx: &mut ParseContext) -> Result<DynSegment> {
        let tokens = ctx.cursor.take_tokens().expect("tokens present");
        self.cel.set_lex_tokens(tokens);
        let result = self.cel.parse_expression();
        ctx.cursor
            .set_tokens(self.cel.take_lex_tokens().expect("tokens set"));
        result
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// How one input cell's identifier scope entry pushes its value onto a deduced-expression body's
/// segment: a scalar cell via a plain `push_arg`-family function pointer, or a
/// tuple-typed cell via `cel_runtime::DynSegment::push_arg_as_dynamic_sequence_tuple` (given the
/// declared shape's `AssociatedType` prototype, so ordinary CEL tuple indexing/operators work on
/// it exactly as on an inline tuple literal).
enum InputPush {
    /// A scalar input cell's `push_arg` function pointer.
    Scalar(PushArgFn),
    /// A tuple-typed input cell's declared shape, as an `AssociatedType` prototype (cloned per
    /// call, since `push_arg_as_dynamic_sequence_tuple` consumes its argument by value).
    Tuple(Vec<cel_runtime::AssociatedType>),
}

/// Returns whether one live tuple element `a` structurally matches one declared leaf/tuple
/// `shape` — the base case `tuple_shape_matches_associated` recurses into.
fn element_shape_matches(shape: &TypeShape, a: &cel_runtime::AssociatedType) -> bool {
    match shape {
        TypeShape::Named(type_id) => a.type_id == *type_id,
        TypeShape::Tuple(_) => {
            a.type_id == TypeId::of::<cel_runtime::DynTuple>()
                && tuple_shape_matches_associated(shape, &a.associated)
        }
    }
}

/// Returns whether a whole tuple's element list `associated` structurally matches
/// `shape`'s own top-level element list (same arity, each pair checked via
/// `element_shape_matches`) — `shape` must be `TypeShape::Tuple`.
fn tuple_shape_matches_associated(
    shape: &TypeShape,
    associated: &[cel_runtime::AssociatedType],
) -> bool {
    let TypeShape::Tuple(elements) = shape else {
        return false;
    };
    elements.len() == associated.len()
        && elements
            .iter()
            .zip(associated)
            .all(|(e, a)| element_shape_matches(e, a))
}

/// How to turn one compiled `expression`'s result into per-output values.
enum CompiledOutputs {
    /// One output, scalar: the segment's single result, boxed via `call_dyn`.
    Single(CallDynFn),
    /// One output, tuple-typed: the segment's whole tuple result, moved into one
    /// `DynamicSequence` via `call_dyn_as_dynamic_sequence`.
    SingleTuple(
        Vec<(
            TypeId,
            cel_runtime::ElementDropper,
            cel_runtime::ElementCloner,
            cel_runtime::ElementEq,
            cel_runtime::ElementDebug,
        )>,
    ),
    /// The declared output is the empty tuple `()`: no CEL expression can produce a live
    /// `DynTuple`-tagged 0-arity value (CEL's own `()` literal is the concrete Rust unit type,
    /// a distinct leaf `TypeId`, not `DynTuple`) — so this is its own case, matched directly
    /// against a `()`-typed body result and stored as a trivially-empty `DynamicSequence`.
    EmptyTuple,
    /// A destructuring binding (one or more outputs): the segment's tuple result, split
    /// element-wise via `call_dyn_tuple_mixed`.
    Tuple(Vec<cel_runtime::DynExtractor>),
}

/// Builds a [`Method`] from parsed inputs, outputs, the compiled body segment, and how
/// to split its result across `outputs`.
fn build_method(
    inputs: NamedCells,
    outputs: NamedCells,
    segment: DynSegment,
    compiled: CompiledOutputs,
) -> Method {
    let input_ids: Vec<CellId> = inputs.iter().map(|(_, id, _)| *id).collect();
    let output_ids: Vec<CellId> = outputs.iter().map(|(_, id, _)| *id).collect();
    let input_types: Vec<TypeId> = inputs
        .iter()
        .map(|(_, _, shape)| cell_type_id(shape))
        .collect();
    let output_types: Vec<TypeId> = outputs
        .iter()
        .map(|(_, _, shape)| cell_type_id(shape))
        .collect();

    // Wrap in RefCell: MethodFn is Fn (not FnMut), so interior mutability is required
    // to call call_dyn/call_dyn_tuple(&mut self) from an immutable closure reference.
    let segment = RefCell::new(segment);

    let f =
        move |inputs_any: &[&dyn Any]| -> std::result::Result<Vec<Box<dyn Any>>, anyhow::Error> {
            let seg = &mut *segment.borrow_mut();
            match &compiled {
                CompiledOutputs::Single(call_fn) => Ok(vec![call_fn(seg, inputs_any)?]),
                CompiledOutputs::EmptyTuple => {
                    Ok(vec![
                        Box::new(cel_runtime::DynamicSequence::from_dyn_elements(Vec::new()))
                            as Box<dyn Any>,
                    ])
                }
                CompiledOutputs::SingleTuple(table) => {
                    let leaf = |type_id: TypeId| {
                        table
                            .iter()
                            .find(|(tid, ..)| *tid == type_id)
                            .map(|(_, d, c, e, dbg)| (*d, *c, *e, *dbg))
                    };
                    let seq = seg.call_dyn_as_dynamic_sequence(inputs_any, &leaf)?;
                    Ok(vec![Box::new(seq) as Box<dyn Any>])
                }
                CompiledOutputs::Tuple(extractors) => {
                    // Safety: every DynExtractor::Scalar extractor here is extract_box_impl::<T>
                    // (via TypeEntry::extract_box_fn), which clones rather than moves --
                    // satisfying call_dyn_tuple_mixed's contract, exactly like the pre-tuple
                    // call_dyn_tuple call site this replaces.
                    unsafe { seg.call_dyn_tuple_mixed(inputs_any, extractors) }
                }
            }
        };

    Method::new(input_ids, output_ids, input_types, output_types, f)
}

/// Flattens a declared cell's `TypeShape` to the single `TypeId` `adam_rs` itself needs for its
/// own (CEL-agnostic) type bookkeeping — a `Tuple` shape always flattens to
/// `TypeId::of::<cel_runtime::DynamicSequence>()`, since every tuple shape shares that one
/// concrete storage type regardless of arity/element types.
fn cell_type_id(shape: &TypeShape) -> TypeId {
    match shape {
        TypeShape::Named(type_id) => *type_id,
        TypeShape::Tuple(_) => TypeId::of::<cel_runtime::DynamicSequence>(),
    }
}

/// A single-token `ExprSpan` where start and end coincide.
fn point(span: Span) -> crate::ast::ExprSpan {
    crate::ast::ExprSpan {
        start: span,
        end: span,
    }
}

/// `binding_target = identifier | "(" identifier { "," identifier } [ "," ] ")".`
///
/// Returns the output names in declaration order alongside whether the left-hand side requests
/// destructuring: `false` for a bare identifier or a single parenthesized identifier with no
/// comma (mere grouping, matching Rust's `(a)` pattern); `true` for `(a,)` (a 1-tuple pattern,
/// trailing comma mandatory) or `(a, b, ...)`.
fn parse_binding_target(ctx: &mut ParseContext) -> Result<(Vec<(String, Span)>, bool)> {
    if !ctx.at_open_paren() {
        let (name, span) = ctx.consume_ident()?;
        return Ok((vec![(name, span)], false));
    }

    ctx.expect_open_paren()?;
    let (first_name, first_span) = ctx.consume_ident()?;
    if ctx.at_close_paren() {
        // Grouping: exactly one identifier, no comma -- same as the bare form.
        ctx.expect_close_paren()?;
        return Ok((vec![(first_name, first_span)], false));
    }
    if !ctx.consume_punct(",") {
        return Err(ctx.err_at("expected ',' or closing parenthesis"));
    }
    if ctx.at_close_paren() {
        // Single identifier + trailing comma: destructures a 1-tuple.
        ctx.expect_close_paren()?;
        return Ok((vec![(first_name, first_span)], true));
    }
    let mut outputs = vec![(first_name, first_span)];
    loop {
        let (name, span) = ctx.consume_ident()?;
        outputs.push((name, span));
        if ctx.at_close_paren() {
            break;
        }
        if !ctx.consume_punct(",") {
            return Err(ctx.err_at("expected ',' or closing parenthesis"));
        }
    }
    ctx.expect_close_paren()?;
    Ok((outputs, true))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TypeRegistry;
    use cel_parser::OpLookup;

    fn parser() -> AdamParser {
        AdamParser::new(TypeRegistry::new(), OpLookup::new())
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
        #[derive(PartialEq, Clone, Debug)]
        struct NoDef(i32);
        let mut reg = TypeRegistry::new();
        reg.register_no_default::<NoDef>("NoDef");
        let mut p = AdamParser::new(reg, OpLookup::new());
        let result = p.parse_str("sheet s { cell x: NoDef; }");
        assert!(result.is_err());
    }

    #[test]
    fn cell_filter_with_no_named_dependency_clamps_on_write() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str(
                "sheet s { cell a: i32 filter if _ < 1 { 1 } else if _ > 100 { 100 } else { _ }; }",
            )
            .unwrap();
        let (cell_id, _) = parsed.cell_names["a"];
        parsed.sheet.write(cell_id, 500i32).unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(cell_id).unwrap(), 100);
    }

    #[test]
    fn cell_filter_referencing_a_cell_tracks_its_current_value() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str(
                "sheet s { \
                     cell hi: i32 = 100; \
                     cell a: i32 filter if _ < 1 { 1 } else if _ > hi { hi } else { _ }; \
                 }",
            )
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        let (hi_id, _) = parsed.cell_names["hi"];

        parsed.sheet.write(a_id, 500i32).unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 100);

        parsed.sheet.write(hi_id, 10i32).unwrap();
        parsed.sheet.write(a_id, 500i32).unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 10);
    }

    #[test]
    fn cell_filter_referencing_the_same_value_twice_is_idempotent() {
        // Snap-to-grid: `_ - (_ % step)` — `_` referenced twice must denote the same value both
        // times, not two independent parameters.
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str("sheet s { cell step: i32 = 10; cell a: i32 filter _ - (_ % step); }")
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        parsed.sheet.write(a_id, 27i32).unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 20);
    }

    #[test]
    fn cell_filter_without_underscore_is_a_parse_error() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let err = parser.parse_str("sheet s { cell a: i32 filter 1; }");
        assert!(err.is_err());
    }

    #[test]
    fn cell_filter_body_type_mismatch_is_a_parse_error() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let err = parser.parse_str("sheet s { cell a: i32 filter _ > 0; }");
        assert!(err.is_err());
    }

    #[test]
    fn cell_filter_undeclared_identifier_is_a_parse_error() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let err = parser.parse_str("sheet s { cell a: i32 filter _ + nope; }");
        assert!(err.is_err());
    }

    #[test]
    fn filter_tracks_a_tuple_typed_range_cell_dynamically() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str(
                "sheet s { \
                     cell a_range: (i32, i32) = (1, 100); \
                     cell max: i32 = 100; \
                     relationship { a_range := (1, max); } \
                     cell a: i32 filter if _ < a_range.0 { a_range.0 } \
                         else if _ > a_range.1 { a_range.1 } else { _ }; \
                 }",
            )
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        let (max_id, _) = parsed.cell_names["max"];

        parsed.sheet.write(a_id, 500i32).unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 100);

        parsed.sheet.write(max_id, 10i32).unwrap();
        parsed.sheet.propagate().unwrap();
        parsed.sheet.write(a_id, 500i32).unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 10);
    }

    #[test]
    fn cell_filter_on_a_tuple_typed_cell_is_a_parse_error() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let err = parser.parse_str("sheet s { cell a: (i32, i32) filter (_.0, _.1); }");
        assert!(err.is_err());
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
    fn parse_relationship_with_a_single_binding() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 2;
                    cell b: i32 = 0;
                    relationship {
                        b := a;
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (b_id, _) = sheet.cell_names["b"].clone();
        assert_eq!(*sheet.read::<i32>(b_id).unwrap(), 2);
    }

    #[test]
    fn parse_relationship_deduces_inputs_from_referenced_identifiers() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 2;
                    cell b: i32 = 3;
                    cell c: i32 = 0;
                    relationship {
                        c := a * b;
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (c_id, _) = sheet.cell_names["c"].clone();
        assert_eq!(*sheet.read::<i32>(c_id).unwrap(), 6);
    }

    #[test]
    fn parse_relationship_with_multiple_bindings_lets_the_planner_pick_a_direction() {
        // `c` is declared first (and so has the weakest cell strength — `adam_rs`'s
        // planner processes cells in *descending* strength order, preferentially
        // leaving the strongest cells as sources, so the earliest-declared cell is the
        // one left to be computed): with `a`/`b` fixed as sources, the only
        // self-consistent choice among these three mutually-referencing bindings is to
        // compute `c := a * b`.
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell c: i32 = 0;
                    cell a: i32 = 2;
                    cell b: i32 = 3;
                    relationship {
                        c := a * b;
                        a := c / b;
                        b := c / a;
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (c_id, _) = sheet.cell_names["c"].clone();
        assert_eq!(*sheet.read::<i32>(c_id).unwrap(), 6);
    }

    #[test]
    fn parse_binding_undeclared_output_is_an_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: i32 = 1;
                relationship {
                    missing := a;
                }
            }
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn parse_binding_multi_output_tuple_matches_existing_tuple_shape_rules() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell w: i32 = 4;
                    cell x: i32 = 0;
                    cell y: i32 = 0;
                    relationship {
                        (x, y) := (w, w * 2);
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (x_id, _) = sheet.cell_names["x"].clone();
        let (y_id, _) = sheet.cell_names["y"].clone();
        assert_eq!(*sheet.read::<i32>(x_id).unwrap(), 4);
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 8);
    }

    #[test]
    fn parse_binding_multi_output_arity_mismatch_is_an_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell w: i32 = 4;
                cell x: i32 = 0;
                cell y: i32 = 0;
                relationship {
                    (x, y) := w;
                }
            }
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn parse_binding_multi_output_without_parens_is_a_parse_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell w: i32 = 4;
                cell x: i32 = 0;
                cell y: i32 = 0;
                relationship {
                    x, y := (w, w * 2);
                }
            }
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn parse_binding_single_element_tuple_destructure_extracts_the_element() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell w: i32 = 4;
                    cell x: i32 = 0;
                    relationship {
                        (x,) := (w,);
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (x_id, _) = sheet.cell_names["x"].clone();
        assert_eq!(*sheet.read::<i32>(x_id).unwrap(), 4);
    }

    #[test]
    fn parse_binding_single_parenthesized_identifier_without_comma_is_a_direct_bind() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell w: i32 = 4;
                    cell x: i32 = 0;
                    relationship {
                        (x) := w;
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (x_id, _) = sheet.cell_names["x"].clone();
        assert_eq!(*sheet.read::<i32>(x_id).unwrap(), 4);
    }

    #[test]
    fn parse_out_with_direct_initializer_and_no_require_block() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    cell b: i32 = 4;
                    out area: i32 := a * b;
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = sheet.output_names["area"];
        let cell_id = sheet.sheet.output_cell(output_id).unwrap();
        assert_eq!(*sheet.sheet.read::<i32>(cell_id).unwrap(), 12);
    }

    #[test]
    fn parse_out_with_no_type_annotation_infers_from_initializer() {
        let sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    out doubled := a * 2;
                }
            "#,
            )
            .unwrap();
        let (_, shape) = sheet.cell_names["doubled"].clone();
        assert_eq!(
            shape,
            crate::type_registry::TypeShape::Named(std::any::TypeId::of::<i32>())
        );
    }

    #[test]
    fn parse_out_with_a_require_block_registers_named_requirements() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    cell b: i32 = 4;
                    out area: i32 := a * b require {
                        positive: area > 0;
                        small: area < 1000;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = sheet.output_names["area"];
        assert!(sheet.sheet.output_requirements(output_id).unwrap().len() == 2);
        assert!(
            sheet
                .sheet
                .violated_requirements(output_id)
                .next()
                .is_none()
        );
    }

    #[test]
    fn parse_out_require_block_requirement_can_violate() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    cell b: i32 = 4;
                    out area: i32 := a * b require {
                        too_small: area > 1000;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = sheet.output_names["area"];
        assert_eq!(sheet.sheet.violated_requirements(output_id).count(), 1);
    }

    #[test]
    fn parse_requirement_non_bool_body_is_an_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: i32 = 3;
                out x: i32 := a require {
                    bad: a;
                };
            }
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn parses_a_sheet_with_an_outer_doc_comment_on_a_cell() {
        let parsed = parser()
            .parse_str("sheet s {\n    /// the total\n    cell x: i32 = 1;\n}")
            .unwrap();
        assert_eq!(parsed.cell_names.len(), 1);
    }

    #[test]
    fn parses_a_sheet_with_an_inner_doc_comment() {
        let parsed = parser()
            .parse_str("//! module docs\nsheet s {\n    cell x: i32 = 1;\n}")
            .unwrap();
        assert_eq!(parsed.cell_names.len(), 1);
    }

    #[test]
    fn parses_a_sheet_with_doc_comments_on_every_declaration_kind() {
        let source = "//! module docs\nsheet s {\n    /// a cell\n    cell x: i32 = 1;\n\n    /// another cell\n    cell y: i32 = 2;\n\n    /// a relationship\n    relationship { y := x; }\n}";
        let parsed = parser().parse_str(source).unwrap();
        assert_eq!(parsed.cell_names.len(), 2);
    }

    #[test]
    fn parse_binding_undeclared_input_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell x: f64 = 1.0;
                relationship { x := bogus; }
            }
        "#,
        );
        assert!(result.is_err());
        let err = result.err().expect("expected Err");
        let msg = err.message().to_lowercase();
        assert!(msg.contains("bogus") || msg.contains("undefined"), "{msg}");
    }

    #[test]
    fn parse_method_output_type_mismatch_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell x: f64 = 0.0;
                cell n: i32 = 0;
                relationship { n := x; }
            }
        "#,
        );
        assert!(result.is_err(), "f64 body for i32 output must be an error");
    }

    #[test]
    fn parse_and_propagate_multi_output_tuple_sheet() {
        let mut sheet = parser()
            .parse_str(
                r#"
            sheet s {
                cell a:    i32 = 3;
                cell b:    i32 = 4;
                cell sum:  i32;
                cell diff: i32;
                relationship { (sum, diff) := (a + b, a - b); }
            }
        "#,
            )
            .unwrap();

        // sum = a + b = 7, diff = a - b = -1. This exercises the CompiledOutputs::Tuple
        // runtime path end to end (RefCell wrapping + seg.call_dyn_tuple); see
        // `existing_multi_output_scalar_methods_still_work_unchanged` for the value-level
        // assertion of the same mechanism.
        sheet.propagate().unwrap();
        let _ = sheet; // sheet is live and propagated
    }

    #[test]
    fn parse_method_output_tuple_arity_mismatch_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: i32 = 1;
                cell b: i32 = 2;
                cell x: i32;
                cell y: i32;
                cell z: i32;
                relationship { (x, y, z) := (a + b, a - b); }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "2-tuple body for 3 declared outputs must be an error"
        );
        let err = result.err().expect("expected Err");
        let msg = err.message().to_lowercase();
        assert!(msg.contains("arity"), "{msg}");
    }

    #[test]
    fn parse_method_output_tuple_element_type_mismatch_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: i32 = 1;
                cell b: f64 = 2.0;
                cell x: i32;
                cell y: i32;
                relationship { (x, y) := (a, b); }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "f64 tuple element for an i32 output must be an error"
        );
        let err = result.err().expect("expected Err");
        let msg = err.message().to_lowercase();
        assert!(msg.contains("type mismatch"), "{msg}");
    }

    #[test]
    fn parse_method_single_output_rejects_tuple_body() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell x: i32 = 1;
                cell y: i32;
                relationship { y := (x,); }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "1-tuple body for a single declared output must be an error"
        );
    }

    #[test]
    fn parse_method_single_tuple_typed_output() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    cell b: i32 = 4;
                    cell pair: (i32, i32);
                    relationship { pair := (a, b); }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (cell_id, _) = sheet.cell_names["pair"].clone();
        let value = sheet.read::<cel_runtime::DynamicSequence>(cell_id).unwrap();
        let (a, b): (i32, i32) = value.try_to_tuple().unwrap();
        assert_eq!((a, b), (3, 4));
    }

    #[test]
    fn parse_method_tuple_typed_output_among_several() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    cell b: i32 = 4;
                    cell pair: (i32, i32);
                    cell extra: i32;
                    relationship { (pair, extra) := ((a, b), a); }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (pair_id, _) = sheet.cell_names["pair"].clone();
        let (extra_id, _) = sheet.cell_names["extra"].clone();
        let pair = sheet.read::<cel_runtime::DynamicSequence>(pair_id).unwrap();
        let (a, b): (i32, i32) = pair.try_to_tuple().unwrap();
        assert_eq!((a, b), (3, 4));
        assert_eq!(*sheet.read::<i32>(extra_id).unwrap(), 3);
    }

    #[test]
    fn parse_method_with_tuple_typed_input_supports_field_indexing() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell pair: (i32, i32) = (10, 20);
                    cell sum: i32;
                    relationship { sum := pair.0 + pair.1; }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (sum_id, _) = sheet.cell_names["sum"].clone();
        assert_eq!(*sheet.read::<i32>(sum_id).unwrap(), 30);
    }

    #[test]
    fn parse_method_tuple_output_shape_mismatch_is_an_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: i32 = 1;
                cell b: f64 = 2.0;
                cell pair: (i32, i32);
                relationship { pair := (a, b); }
            }
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn existing_multi_output_scalar_methods_still_work_unchanged() {
        // Regression: today's N-scalar-outputs mechanism must still behave identically after the
        // CompiledOutputs refactor.
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    cell b: i32 = 4;
                    cell sum: i32;
                    cell diff: i32;
                    relationship { (sum, diff) := (a + b, a - b); }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (sum_id, _) = sheet.cell_names["sum"].clone();
        let (diff_id, _) = sheet.cell_names["diff"].clone();
        assert_eq!(*sheet.read::<i32>(sum_id).unwrap(), 7);
        assert_eq!(*sheet.read::<i32>(diff_id).unwrap(), -1);
    }

    #[test]
    fn parse_method_single_empty_tuple_typed_output() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell x: i32 = 1;
                    cell nothing: ();
                    // `()` alone references no cell, and adam_rs rejects a method with no
                    // inputs -- reference `x` via an if/else whose branches both still
                    // evaluate to `()`, so the binding has a deduced input.
                    relationship { nothing := if x > 0 { () } else { () }; }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (cell_id, _) = sheet.cell_names["nothing"].clone();
        let value = sheet.read::<cel_runtime::DynamicSequence>(cell_id).unwrap();
        assert_eq!(value.arity(), 0);
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
                        relationship { height := width; }
                    },
                    1i32 => {
                        relationship { height := width * ratio; }
                    },
                    _ => {
                        relationship { height := width; }
                    },
                }
            }
        "#,
            )
            .unwrap();
    }

    #[test]
    fn parse_conditional_branch_with_multiple_relationships() {
        let mut sheet = parser()
            .parse_str(
                r#"
            sheet s {
                cell mode: i32 = 0;
                cell a: f64 = 2.0;
                cell b: f64 = 3.0;
                cell c: f64;
                cell d: f64 = 4.0;
                cell e: f64 = 5.0;
                cell f: f64;
                conditional mode {
                    0i32 => {
                        relationship { c := a * b; }
                        relationship { f := d * e; }
                    }
                }
            }
        "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
    }

    #[test]
    fn conditional_branch_bare_binding_without_relationship_wrapper_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell x: i32 = 0;
                conditional x { 0i32 => { x := x; } }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "a conditional_branch body now requires relationship_decl, not a bare binding"
        );
    }

    #[test]
    fn conditional_undeclared_match_cell_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell x: i32 = 0;
                conditional bogus { 0i32 => { relationship { x := x; } } }
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
                conditional mode { 1.0 => { relationship { x := x; } } }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "float literal for i32 match cell must be an error"
        );
    }

    #[test]
    fn parse_conditional_with_tuple_typed_match_cell() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell mode: (i32, i32) = (0, 0);
                    cell x: f64 = 1.0;
                    cell y: f64;
                    conditional mode {
                        (0, 0) => { relationship { y := x; } },
                        _ => { relationship { y := x * 2.0; } },
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (y_id, _) = sheet.cell_names["y"].clone();
        assert_eq!(*sheet.read::<f64>(y_id).unwrap(), 1.0);
    }

    #[test]
    fn conditional_on_a_two_cell_boolean_expression_activates_and_reacts_to_writes() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: bool = false;
                    cell b: bool = false;
                    cell x: i32 = 1;
                    cell y: i32 = 0;
                    conditional a && b {
                        true => { relationship { y := x; } },
                    }
                }
            "#,
            )
            .unwrap();
        let (a_id, _) = sheet.cell_names["a"].clone();
        let (b_id, _) = sheet.cell_names["b"].clone();
        let (y_id, _) = sheet.cell_names["y"].clone();

        sheet.write(a_id, true).unwrap();
        sheet.write(b_id, false).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 0);

        sheet.write(b_id, true).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 1);
    }

    #[test]
    fn conditional_expression_referencing_the_same_cell_twice_compiles_and_evaluates() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: bool = true;
                    cell x: i32 = 1;
                    cell y: i32 = 0;
                    conditional a && a {
                        true => { relationship { y := x; } },
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (y_id, _) = sheet.cell_names["y"].clone();
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 1);
    }

    #[test]
    fn conditional_bare_identifier_match_subject_still_works() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell mode: i32 = 0;
                    cell x: i32 = 1;
                    cell y: i32 = 0;
                    conditional mode {
                        1i32 => { relationship { y := x; } },
                    }
                }
            "#,
            )
            .unwrap();
        let (mode_id, _) = sheet.cell_names["mode"].clone();
        let (y_id, _) = sheet.cell_names["y"].clone();

        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 0);

        sheet.write(mode_id, 1_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 1);
    }

    #[test]
    fn conditional_tuple_expression_match_subject_drives_branch_selection() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 0;
                    cell b: i32 = 0;
                    cell x: i32 = 1;
                    cell y: i32 = 0;
                    conditional (a, b) {
                        (1i32, 2i32) => { relationship { y := x; } },
                    }
                }
            "#,
            )
            .unwrap();
        let (a_id, _) = sheet.cell_names["a"].clone();
        let (b_id, _) = sheet.cell_names["b"].clone();
        let (y_id, _) = sheet.cell_names["y"].clone();

        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 0);

        sheet.write(a_id, 1_i32).unwrap();
        sheet.write(b_id, 2_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 1);
    }

    #[test]
    fn conditional_client_registered_type_match_expression_dispatches_correctly() {
        // `Mode(1)` isn't valid CEL syntax here: adam-lang has no literal-construction syntax
        // for a client-registered (non-built-in) type, so a bare "TypeName(args)" call parses
        // as an unresolved 1-arity identifier lookup, not a constructor. Instead, register a
        // stand-in 0-arity identifier `mode_one` directly on the CEL op lookup (the same
        // mechanism `parse_deduced_expr`'s grow-on-demand scope uses for resolving bare
        // identifiers) that pushes a `Mode(1)` constant via
        // `DynSegment::just`. This lets the DSL source below produce a `Mode` value for both
        // the cell initializer and the branch key, exercising `TypeRegistry`'s
        // `entry_by_type_id`/`eq_dyn_fn`/`call_dyn_fn` dispatch for a client-registered type —
        // the actual point of this test, independent of how the value is spelled.
        #[derive(PartialEq, Clone, Debug, Default)]
        struct Mode(i32);

        let mut reg = TypeRegistry::new();
        reg.register::<Mode>("Mode");
        let mut parser = AdamParser::new(reg, OpLookup::new());
        parser
            .op_lookup_mut()
            .push_scope(|name, segment, arity, _span| {
                if name == "mode_one" && arity == 0 {
                    segment.just(Mode(1));
                    return Ok(true);
                }
                Ok(false)
            });
        let mut sheet = parser
            .parse_str(
                r#"
                sheet s {
                    cell m: Mode = mode_one;
                    cell x: i32 = 1;
                    cell y: i32 = 0;
                    conditional m {
                        mode_one => { relationship { y := x; } },
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (y_id, _) = sheet.cell_names["y"].clone();
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 1);
    }

    #[test]
    fn conditional_expression_referencing_an_undeclared_identifier_is_a_parse_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: bool = true;
                conditional a && nope {
                    true => { relationship { a := a; } },
                }
            }
        "#,
        );
        assert!(result.is_err());
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

    #[test]
    fn parse_and_propagate_sheet() {
        let mut p = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut sheet = p
            .parse_str(
                r#"
            sheet image_resize {
                cell width:  f64 = 4.0;
                cell height: f64 = 3.0;
                cell area:   f64;
                relationship {
                    area := width * height;
                    width := area / height;
                    height := area / width;
                }
            }
        "#,
            )
            .unwrap();

        // The relationship should compute area = width * height = 4.0 * 3.0 = 12.0.
        // We can't read cells by name yet (no name→CellId API on Sheet), so we verify
        // the sheet propagated without error, which exercises the full call_dyn path.
        // A future test can assert specific cell values once a name-lookup API exists.
        sheet.propagate().unwrap();
        let _ = sheet; // sheet is live and propagated
    }

    #[test]
    fn parse_duplicate_cell_is_error() {
        let result = parser().parse_str("sheet s { cell x: i32; cell x: f64; }");
        match result {
            Ok(_) => panic!("expected error for duplicate cell name"),
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("duplicate cell `x`"),
                    "error mentions duplicate name: {msg}"
                );
            }
        }
    }

    #[test]
    fn parse_str_returns_cell_names_in_declaration_order() {
        let parsed = parser()
            .parse_str("sheet s { cell z: i32 = 1; cell a: i32 = 2; cell m: i32 = 3; }")
            .unwrap();
        let names: Vec<&str> = parsed.cell_names.keys().map(String::as_str).collect();
        assert_eq!(names, vec!["z", "a", "m"]);
    }

    #[test]
    fn parsed_sheet_derefs_to_sheet_for_propagate() {
        let mut parsed = parser().parse_str("sheet s { cell x: i32 = 1; }").unwrap();
        // Deref/DerefMut must make Sheet's methods directly callable.
        parsed.propagate().unwrap();
    }

    #[test]
    fn parse_out_with_explicit_type_propagates_correctly() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell width: f64 = 4.0;
                    cell height: f64 = 3.0;
                    out area: f64 := width * height;
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = *sheet.output_names.get("area").expect("area registered");
        let cell_id = sheet.output_cell(output_id).expect("output has a cell");
        assert_eq!(*sheet.read::<f64>(cell_id).unwrap(), 12.0);
    }

    #[test]
    fn parse_out_with_tuple_type_value_debug_formats_correctly() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell x: i32 = 3;
                    out pair: (i32, i32) := (x, x);
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = *sheet.output_names.get("pair").unwrap();
        let cell_id = sheet.output_cell(output_id).unwrap();
        let value = sheet.read::<cel_runtime::DynamicSequence>(cell_id).unwrap();
        assert_eq!(format!("{value:?}"), "(3, 3)");
    }

    #[test]
    fn parse_out_with_no_annotation_infers_type_from_writer_body() {
        let sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell width: f64 = 4.0;
                    out doubled := width + width;
                }
            "#,
            )
            .unwrap();
        let (_, shape) = sheet.cell_names.get("doubled").unwrap().clone();
        assert_eq!(shape, TypeShape::Named(std::any::TypeId::of::<f64>()));
    }

    #[test]
    fn parse_out_type_mismatch_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell width: f64 = 4.0;
                out area: i32 := width;
            }
        "#,
        );
        assert!(
            result.is_err(),
            "f64 body for an i32 annotation must be an error"
        );
    }

    #[test]
    fn parse_out_with_requirements_reports_output_valid_and_violated() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell width: f64 = 4.0;
                    cell height: f64 = 3.0;
                    cell max_area: f64 = 100.0;
                    out area: f64 := width * height require {
                        max_area: width * height <= max_area;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = *sheet.output_names.get("area").unwrap();
        assert!(sheet.output_valid(output_id));
        assert_eq!(sheet.violated_requirements(output_id).count(), 0);
    }

    #[test]
    fn parse_out_requirement_violation_is_reported_after_propagate() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell width: f64 = 40.0;
                    cell height: f64 = 30.0;
                    cell max_area: f64 = 100.0;
                    out area: f64 := width * height require {
                        max_area: width * height <= max_area;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = *sheet.output_names.get("area").unwrap();
        assert!(!sheet.output_valid(output_id));
        assert_eq!(sheet.violated_requirements(output_id).count(), 1);
    }

    #[test]
    fn parse_out_duplicate_requirement_names_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell width: f64 = 4.0;
                out area: f64 := width require {
                    dup: width <= 10.0;
                    dup: width >= 0.0;
                };
            }
        "#,
        );
        assert!(
            result.is_err(),
            "two requirements sharing a name must be an error"
        );
    }

    #[test]
    fn parse_out_cell_referenced_elsewhere_is_terminal_cell_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell width: f64 = 4.0;
                cell height: f64 = 3.0;
                out area: f64 := width * height;
                relationship { width := area; }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "referencing an out cell as another relationship's input must be an error"
        );
    }

    #[test]
    fn parse_out_with_no_annotation_and_unregistered_type_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell dummy: i32 = 0;
                out x := ();
            }
        "#,
        );
        assert!(
            result.is_err(),
            "an out with no annotation whose writer body produces an unregistered type must be an error"
        );
    }

    #[test]
    fn parse_out_cell_referenced_in_conditional_is_terminal_cell_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell width: f64 = 4.0;
                cell height: f64 = 3.0;
                cell mode: i32 = 0;
                out area: f64 := width * height;
                conditional mode {
                    0i32 => { relationship { width := area; } }
                }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "referencing an out cell as a conditional branch relationship's input must be an error"
        );
    }

    #[test]
    fn parse_cell_with_explicit_tuple_type_and_initializer() {
        let mut p = parser();
        let parsed = p
            .parse_str("sheet s { cell a: (i32, f64) = (1, 2.5); }")
            .unwrap();
        let (cell_id, shape) = parsed.cell_names["a"].clone();
        assert_eq!(
            shape,
            TypeShape::Tuple(vec![
                TypeShape::Named(std::any::TypeId::of::<i32>()),
                TypeShape::Named(std::any::TypeId::of::<f64>()),
            ])
        );
        let value = parsed
            .sheet
            .read::<cel_runtime::DynamicSequence>(cell_id)
            .unwrap();
        let (a, b): (i32, f64) = value.try_to_tuple().unwrap();
        assert_eq!((a, b), (1, 2.5));
    }

    #[test]
    fn parse_cell_with_tuple_type_and_no_initializer_uses_recursive_default() {
        let parsed = parser()
            .parse_str("sheet s { cell a: (i32, f64); }")
            .unwrap();
        let (cell_id, _) = parsed.cell_names["a"].clone();
        let value = parsed
            .sheet
            .read::<cel_runtime::DynamicSequence>(cell_id)
            .unwrap();
        let (a, b): (i32, f64) = value.try_to_tuple().unwrap();
        assert_eq!((a, b), (0, 0.0));
    }

    #[test]
    fn parse_cell_with_tuple_initializer_arity_mismatch_is_an_error() {
        let result = parser().parse_str("sheet s { cell a: (i32, f64, i32) = (1, 2.5); }");
        assert!(result.is_err());
    }

    #[test]
    fn parse_cell_with_nested_tuple_type_round_trips() {
        let parsed = parser()
            .parse_str("sheet s { cell a: (i32, (f64, String)) = (1, (2.5, \"x\")); }")
            .unwrap();
        let (cell_id, _) = parsed.cell_names["a"].clone();
        let value = parsed
            .sheet
            .read::<cel_runtime::DynamicSequence>(cell_id)
            .unwrap();
        let (a, nested): (i32, cel_runtime::DynamicSequence) = value.try_to_tuple().unwrap();
        assert_eq!(a, 1);
        let (b, c): (f64, String) = nested.try_to_tuple().unwrap();
        assert_eq!((b, c), (2.5, "x".to_string()));
    }

    #[test]
    fn parse_out_with_explicit_tuple_type_infers_and_stores_correctly() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell x: i32 = 3;
                    out pair: (i32, i32) := (x, x);
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = *sheet.output_names.get("pair").unwrap();
        let cell_id = sheet.output_cell(output_id).unwrap();
        let value = sheet.read::<cel_runtime::DynamicSequence>(cell_id).unwrap();
        let (a, b): (i32, i32) = value.try_to_tuple().unwrap();
        assert_eq!((a, b), (3, 3));
    }

    #[test]
    fn parse_cell_range_initializer_fails_cleanly_at_type_inference_not_grammar() {
        // `Range<i32>` isn't a registered adam-lang type — this must still fail today, but only
        // at `eval_segment_boxed`'s existing "cannot infer a type" check, proving the CEL-level
        // range parsing itself succeeded (rather than failing as "unexpected token" or similar
        // at the grammar level, which would indicate the entry-point swap didn't take effect).
        let result = parser().parse_str("sheet s { cell x = 1i32..5i32; }");
        assert!(result.is_err());
        let err = result.err().expect("expected Err");
        assert_eq!(
            err.message(),
            "cannot infer a type for this expression; register a type name for it or add an \
             explicit `: type_expr` annotation"
        );
    }
}
