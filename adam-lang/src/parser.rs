//! adam-lang parser — grammar productions and sheet construction.
//!
//! See the crate root's [`# Grammar`](crate#grammar) section for the full EBNF.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;

use adam_rs::{
    CellId, ErrorSite, MatchExpr, Method, RelationshipId, Requirement, RequirementId, Sheet,
};
use cel_parser::lex_lexer::{HasSpan, LexLexer, Token};
use cel_parser::{CELParser, OpLookup, ParseError, SourceSpan};
use cel_runtime::DynSegment;
use proc_macro2::{Span, TokenStream};

use crate::TypeRegistry;
use crate::cell_value::{CellValue, Frame, RawFrame};
use crate::type_registry::{AddConditionalFn, TypeShape};

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
///
/// # Examples
///
/// ```
/// use adam_lang::{AdamParser, TypeRegistry};
/// use cel_parser::OpLookup;
/// let parsed = AdamParser::new(TypeRegistry::new(), OpLookup::new())
///     .parse_str("sheet s { cell x: i32 = 1 require { @positive x > 0i32; }; }")?;
/// let id = parsed.cell_requirements(parsed.cell_names["x"].0).unwrap()[0];
/// assert_eq!(parsed.requirement_names[&id], "positive");
/// # Ok::<(), cel_parser::ParseError>(())
/// ```
pub struct ParsedSheet {
    /// The constructed sheet.
    pub sheet: Sheet,
    /// Cell name → `(CellId, TypeShape)`, in declaration order.
    pub cell_names: IndexMap<String, (CellId, TypeShape)>,
    /// Output name → `CellId`, in declaration order — parity with `cell_names`, for callers
    /// that need to look up `Sheet::cell_requirements_valid`/`Sheet::violated_requirements` by
    /// name.
    pub output_names: IndexMap<String, CellId>,
    /// Explicit `@name` labels keyed by requirement ID; unnamed requirements have no entry.
    /// Lookup takes expected O(1) time. Iterate the sheet's requirement IDs, not this map,
    /// to preserve attachment or violation order.
    pub requirement_names: HashMap<RequirementId, String>,
    /// `(RelationshipId, method index)` → the source span of that binding, populated for
    /// every successfully-added relationship. Lets a caller translate an `adam_rs::Error`'s
    /// `ErrorSite::Method` (raised well after parsing, e.g. from `Sheet::propagate`) back
    /// to a source location.
    ///
    /// `out` declarations' internal writer relationships (created via `Sheet::add_out`, not
    /// `parse_relationship_decl`) are not recorded here, since `add_out` returns only a
    /// `CellId`, not the writer's `RelationshipId`. In practice this is narrow: an `out`
    /// writer's body is always a CEL expression, so arithmetic failures already carry a
    /// `cel_parser::SpanContext` regardless of this gap. A `MethodFailed`/`TypeMismatch` from
    /// an `out` writer without a `SpanContext` falls back to `Display` instead of a
    /// source-span diagnostic — never worse than resolving no site at all.
    pub method_spans: HashMap<(RelationshipId, usize), SourceSpan>,
    /// `RelationshipId` → the source span of that `relationship { ... }` block, populated for
    /// every successfully-added relationship. Resolves an `adam_rs::Error`'s
    /// `ErrorSite::Relationship` (e.g. from `Error::Conflict`) back to a source location.
    pub relationship_spans: HashMap<RelationshipId, SourceSpan>,
    /// `CellId` → the source span of that cell's declared name, populated for every declared
    /// `cell`, `source`, and `out`. Resolves an `adam_rs::Error`'s `ErrorSite::Cell` back to a
    /// source location.
    pub cell_spans: HashMap<CellId, SourceSpan>,
}

impl std::fmt::Debug for ParsedSheet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedSheet")
            .field("cell_names", &self.cell_names)
            .field("output_names", &self.output_names)
            .field("requirement_names", &self.requirement_names)
            .field("method_spans", &self.method_spans)
            .field("relationship_spans", &self.relationship_spans)
            .field("cell_spans", &self.cell_spans)
            .finish()
    }
}

impl ParsedSheet {
    /// Clones a compound cell into its exact canonical witness representation.
    ///
    /// # Errors
    /// Rejects unknown cells, concrete leaf storage, and recursive shape mismatches.
    /// - Complexity: O(n + v) validation and cloning for n shape nodes and v values.
    ///
    /// # Examples
    /// ```
    /// use adam_lang::{AdamParser, TypeRegistry};
    /// use cel_runtime::{ArrayOf, Leaf};
    /// let sheet = AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new())
    ///     .parse_str("sheet s { cell xs: [i32]; }").unwrap();
    /// assert!(sheet.read_compound::<ArrayOf<Leaf<i32>>>(sheet.cell_names["xs"].0)?.is_empty());
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn read_compound<R: cel_runtime::ValueRepr>(&self, id: CellId) -> anyhow::Result<R::Value> {
        let cell = self.sheet.read::<CellValue>(id)?;
        anyhow::ensure!(
            cell.0.value_type().same_shape(&R::value_type()),
            "compound cell representation mismatch"
        );
        // SAFETY: The checked host boundary just validated the complete witness.
        Ok(unsafe { cell.0.try_clone()?.into_typed_bound::<R>() })
    }

    /// Writes a compound aspiration after validating its complete witness identity.
    ///
    /// Existing cell capabilities, rather than witness defaults, govern propagation.
    /// # Errors
    /// Rejects unknown cells, leaf storage, or any recursive shape mismatch before writing.
    /// - Complexity: O(n) validation and descriptor construction for n shape nodes.
    ///
    /// # Examples
    /// ```
    /// use adam_lang::{AdamParser, TypeRegistry};
    /// use cel_runtime::{Array, ArrayOf, Leaf};
    /// let mut sheet = AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new())
    ///     .parse_str("sheet s { source xs: [i32]; }").unwrap();
    /// sheet.write_compound::<ArrayOf<Leaf<i32>>>(sheet.cell_names["xs"].0, Array::from_vec(vec![1]))?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn write_compound<R: cel_runtime::ValueRepr>(
        &mut self,
        id: CellId,
        value: R::Value,
    ) -> anyhow::Result<()> {
        let descriptor = self.sheet.read::<CellValue>(id)?.0.value_type().clone();
        let value = cel_runtime::OwnedValue::new::<R>(value).with_value_type(descriptor)?;
        self.sheet.write(id, CellValue(value))?;
        Ok(())
    }

    /// Clones the original compound aspiration, independently of any filter result.
    ///
    /// # Errors
    /// Rejects unknown cells, concrete leaf storage, or a mismatched recursive witness.
    /// - Complexity: O(n + v) for n descriptor nodes and v cloned values.
    ///
    /// # Examples
    /// ```
    /// use adam_lang::{AdamParser, TypeRegistry};
    /// use cel_runtime::{ArrayOf, Leaf};
    /// let sheet = AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new())
    ///     .parse_str("sheet s { source xs: [i32]; }").unwrap();
    /// assert!(sheet.source_compound::<ArrayOf<Leaf<i32>>>(sheet.cell_names["xs"].0)?.is_empty());
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn source_compound<R: cel_runtime::ValueRepr>(
        &self,
        id: CellId,
    ) -> anyhow::Result<R::Value> {
        let cell = self.sheet.source::<CellValue>(id)?;
        anyhow::ensure!(
            cell.0.value_type().same_shape(&R::value_type()),
            "compound cell representation mismatch"
        );
        // SAFETY: The complete witness was just checked at this host boundary.
        Ok(unsafe { cell.0.try_clone()?.into_typed_bound::<R>() })
    }

    /// Resolves each of `e`'s `ErrorSite`s to a source span and a human label, primary first.
    ///
    /// Sites whose span is not recorded are skipped, so the result may be shorter than
    /// `e.sites()`; empty when none resolves (the caller then falls back to `Display`).
    ///
    /// - Complexity: O(s) in the number of sites.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use adam_lang::{AdamParser, TypeRegistry};
    /// use cel_parser::OpLookup;
    ///
    /// let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    /// // Two relationships forming an algebraic loop with no external source for either
    /// // cell: `x` needs `y` and `y` needs `x`, so `propagate` can't pick a valid method
    /// // assignment and returns `Error::Cycle`.
    /// let mut parsed = parser
    ///     .parse_str(
    ///         "sheet s { cell x: i32 = 0; cell y: i32 = 0; \
    ///          relationship { x := y + 1i32; } relationship { y := x + 1i32; } }",
    ///     )
    ///     .unwrap();
    /// let e = parsed.propagate().unwrap_err();
    /// let located = parsed.locate_error(&e);
    /// assert!(!located.is_empty());
    /// assert!(!located[0].1.is_empty());
    /// ```
    pub fn locate_error(&self, e: &adam_rs::Error) -> Vec<(SourceSpan, String)> {
        let by_id: HashMap<CellId, String> = self
            .cell_names
            .iter()
            .map(|(n, (id, _))| (*id, n.clone()))
            .collect();
        let name = |id: CellId| by_id.get(&id).cloned();
        let mut out = Vec::new();
        for (i, site) in e.sites().iter().enumerate() {
            let span = match site {
                ErrorSite::Method(r, idx) => self.method_spans.get(&(*r, *idx)).copied(),
                ErrorSite::Relationship(r) => self.relationship_spans.get(r).copied(),
                ErrorSite::Cell(c) => self.cell_spans.get(c).copied(),
                ErrorSite::MethodIndex(_) => None,
                _ => None,
            };
            if let Some(span) = span {
                out.push((span, crate::error_labels::site_label(e, i, &name)));
            }
        }
        out
    }
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

/// Accumulates one sheet's declarations, display metadata, and source locations.
struct ParseContext {
    cursor: crate::token_cursor::TokenCursor,
    sheet: Sheet,
    /// Maps cell name → (CellId, TypeShape), in declaration order, for method and
    /// conditional compilation and for exposing to callers via `ParsedSheet`.
    cell_names: IndexMap<String, (CellId, TypeShape)>,
    /// Maps output name → `CellId`, in declaration order, for exposing to callers via
    /// `ParsedSheet`.
    output_names: IndexMap<String, CellId>,
    /// Accumulates only explicit labels against successfully registered requirement IDs.
    requirement_names: HashMap<RequirementId, String>,
    /// Accumulates spans for every successfully-added relationship's methods, for exposing to
    /// callers via `ParsedSheet::method_spans`.
    method_spans: HashMap<(RelationshipId, usize), SourceSpan>,
    /// Accumulates spans for every successfully-added relationship block, for exposing to
    /// callers via `ParsedSheet::relationship_spans`.
    relationship_spans: HashMap<RelationshipId, SourceSpan>,
    /// Accumulates spans for every declared cell's name, for exposing to callers via
    /// `ParsedSheet::cell_spans`.
    cell_spans: HashMap<CellId, SourceSpan>,
    /// Accumulates proc-macro spans for every declared cell's name, for primary parser errors.
    cell_token_spans: HashMap<CellId, Span>,
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
    /// install a function library (e.g. `cel-std`) before parsing. The embedded CEL parser also
    /// receives a snapshot of `types`' registered scalar leaf names so typed array annotations
    /// such as `[]: [Custom]` resolve custom Adam types during direct parsing.
    pub fn new(types: TypeRegistry, op_lookup: OpLookup) -> Self {
        let cel = CELParser::with_type_resolver(op_lookup, types.cel_type_resolver());
        AdamParser { types, cel }
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
            requirement_names: HashMap::new(),
            method_spans: HashMap::new(),
            relationship_spans: HashMap::new(),
            cell_spans: HashMap::new(),
            cell_token_spans: HashMap::new(),
        };
        let _ = ctx.consume_doc_comment_run(true); // sheet-level `//!` docs (ignored at runtime)
        self.parse_sheet(&mut ctx)?;
        if let Some(tok) = ctx.peek_token() {
            return Err(ParseError::new("unexpected token", tok.span()));
        }
        ctx.sheet
            .validate()
            .map_err(|e| Self::validated_sheet_error(&ctx, e))?;
        Ok(ParsedSheet {
            sheet: ctx.sheet,
            cell_names: ctx.cell_names,
            output_names: ctx.output_names,
            requirement_names: ctx.requirement_names,
            method_spans: ctx.method_spans,
            relationship_spans: ctx.relationship_spans,
            cell_spans: ctx.cell_spans,
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

    /// `sheet_item = [ doc_comment ] (cell_decl | relationship_decl | conditional_decl | out_decl
    /// | source_decl).`
    fn parse_sheet_item(&mut self, ctx: &mut ParseContext) -> Result<()> {
        let _ = ctx.consume_doc_comment_run(false); // outer `///` docs (ignored at runtime)
        match ctx.peek_token() {
            Some(Token::Identifier(id)) if id == "cell" => self.parse_cell_decl(ctx),
            Some(Token::Identifier(id)) if id == "relationship" => {
                self.parse_relationship_decl(ctx).map(|_| ())
            }
            Some(Token::Identifier(id)) if id == "conditional" => self.parse_conditional_decl(ctx),
            Some(Token::Identifier(id)) if id == "out" => self.parse_out_decl(ctx),
            Some(Token::Identifier(id)) if id == "source" => self.parse_source_decl(ctx),
            Some(tok) => Err(ParseError::new(
                "expected `cell`, `relationship`, `conditional`, `out`, or `source`",
                tok.span(),
            )),
            None => Err(ParseError::new(
                "unexpected end of input",
                Span::call_site(),
            )),
        }
    }

    /// `cell_decl = "cell" identifier cell_type_init [ cell_filter ] [ "require" "{" {
    /// requirement } "}" ] ";".`
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
            self.validate_initializer(&segment, declared_shape.as_ref(), &name, name_span)?;
            let (actual_shape, cell_id) = self
                .build_cell_from_segment(segment, ctx)
                .map_err(|e| ParseError::new(e.message(), name_span))?;
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
        ctx.cell_spans
            .insert(cell_id, SourceSpan::from_proc_macro2(name_span));
        ctx.cell_token_spans.insert(cell_id, name_span);

        let filter = if ctx.is_keyword("filter") {
            Some(self.parse_cell_filter(ctx, &name, name_span, &shape)?)
        } else {
            None
        };

        // Inserted before the `require` block is parsed (unlike `filter`, parsed just above with
        // `name` not yet in `ctx.cell_names`) so a requirement can reference the cell's own name
        // — e.g. `require { @positive x > 0; }` — exactly as `parse_out_decl`'s `require` block
        // can already reference its own out cell's name.
        ctx.cell_names.insert(name.clone(), (cell_id, shape));

        let require_names_and_reqs: Vec<(Option<String>, Requirement)> =
            if ctx.is_keyword("require") {
                self.parse_requirement_block(ctx)?
            } else {
                Vec::new()
            };

        ctx.expect_punct(";")?;
        if let Some(filter) = filter {
            ctx.sheet
                .add_filter(cell_id, filter)
                .map_err(|e| Self::sheet_error(ctx, name_span, e))?;
        }
        for (req_name, requirement) in require_names_and_reqs {
            Self::register_requirement(ctx, cell_id, req_name, requirement, name_span)?;
        }
        Ok(())
    }

    /// `source_decl = "source" identifier cell_type_init [ cell_filter ] [ "require" "{" {
    /// requirement } "}" ] ";".`
    ///
    /// `cell_type_init = (":" type_expr ["=" expression]) | ("=" expression).`
    ///
    /// Mirrors [`Self::parse_cell_decl`] exactly — the declared cell is added via
    /// [`Sheet::add_source`] instead of [`Sheet::add_cell`], so it is fixed as a planner source
    /// and can never be claimed as a relationship's output.
    fn parse_source_decl(&mut self, ctx: &mut ParseContext) -> Result<()> {
        ctx.is_keyword("source"); // consume
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
            self.validate_initializer(&segment, declared_shape.as_ref(), &name, name_span)?;
            let (actual_shape, cell_id) = self
                .build_source_cell_from_segment(segment, ctx)
                .map_err(|e| ParseError::new(e.message(), name_span))?;
            if let Some(declared) = &declared_shape
                && declared != &actual_shape
            {
                return Err(ParseError::new(
                    format!(
                        "source `{name}`: type mismatch: expected `{}`, got `{}`",
                        self.types.display_name(declared),
                        self.types.display_name(&actual_shape)
                    ),
                    name_span,
                ));
            }
            (actual_shape, cell_id)
        } else {
            let declared = declared_shape.ok_or_else(|| {
                ParseError::new("expected `:` or `=` in source declaration", name_span)
            })?;
            let cell_id = self.build_default_source_cell(&declared, name_span, ctx)?;
            (declared, cell_id)
        };
        ctx.cell_spans
            .insert(cell_id, SourceSpan::from_proc_macro2(name_span));
        ctx.cell_token_spans.insert(cell_id, name_span);

        let filter = if ctx.is_keyword("filter") {
            Some(self.parse_cell_filter(ctx, &name, name_span, &shape)?)
        } else {
            None
        };

        // Inserted before the `require` block is parsed so a requirement can reference the
        // cell's own name — e.g. `require { @positive x > 0; }` — exactly as `parse_cell_decl`.
        ctx.cell_names.insert(name.clone(), (cell_id, shape));

        let require_names_and_reqs: Vec<(Option<String>, Requirement)> =
            if ctx.is_keyword("require") {
                self.parse_requirement_block(ctx)?
            } else {
                Vec::new()
            };

        ctx.expect_punct(";")?;
        if let Some(filter) = filter {
            ctx.sheet
                .add_filter(cell_id, filter)
                .map_err(|e| Self::sheet_error(ctx, name_span, e))?;
        }
        for (req_name, requirement) in require_names_and_reqs {
            Self::register_requirement(ctx, cell_id, req_name, requirement, name_span)?;
        }
        Ok(())
    }

    /// `cell_filter = "filter" expression.`
    ///
    /// Builds an [`adam_rs::Filter`] from a single deduced expression: `_` denotes the candidate
    /// value being conformed (of `declared_shape`'s type); every other identifier that names an
    /// already-declared cell is a deduced dependency, exactly as [`Self::parse_deduced_expr`]
    /// resolves them for a `relationship` binding or `out` declaration — see
    /// [`Self::parse_filter_expr`]. `cell_name`/`cell_span`/`declared_shape` describe the
    /// *filtered cell* (for error-message context and the candidate value's type), already
    /// resolved by the caller — [`Self::parse_cell_decl`], [`Self::parse_source_decl`], or
    /// [`Self::parse_out_decl`]. The filtered cell's own `CellId` is not needed
    /// here: the caller attaches the returned `Filter` to it afterwards, via `Sheet::add_filter`.
    ///
    /// # Errors
    /// Returns `Err` if an identifier inside the expression names neither `_` nor an
    /// already-declared cell, if `_`
    /// is never referenced, if the expression's inferred type doesn't match `declared_shape`, or,
    /// if the expression is `RangeInclusive`-typed, if its element type doesn't match
    /// `declared_shape`.
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
        let (segment, inputs, underscore_used) = self.parse_filter_expr(ctx, declared_shape)?;

        let value_type_id = cell_type_id(declared_shape);
        let output_type_id = segment.peek_output_type_id().ok_or_else(|| {
            ParseError::new(
                format!("cell `{cell_name}`: filter produced no value"),
                cell_span,
            )
        })?;

        let arg_ids: Vec<CellId> = inputs.iter().map(|(_, id, _)| *id).collect();
        let arg_type_ids: Vec<TypeId> = inputs
            .iter()
            .map(|(_, _, shape)| cell_type_id(shape))
            .collect();
        let mut input_shapes = vec![declared_shape.clone()];
        input_shapes.extend(inputs.iter().map(|(_, _, shape)| shape.clone()));

        // A `RangeInclusive<T>`-shaped output is checked and built independently of
        // `underscore_used`: a genuine range filter body (`lo..=hi`) essentially never references
        // `_`, so gating this branch on `underscore_used` would make the more specific "range
        // bounds must be" error unreachable for the primary mistake it exists to catch.
        if let Some(range_shape) = self.types.range_entry(output_type_id) {
            if range_shape.element_type_id != value_type_id {
                return Err(ParseError::new(
                    format!(
                        "cell `{cell_name}`: filter range bounds must be `{}`",
                        self.types.display_name(declared_shape)
                    ),
                    cell_span,
                ));
            }

            let default_fn = self
                .types
                .entry_by_type_id(value_type_id)
                .expect("declared cell type registered")
                .default_fn
                .expect("numeric range-filter cell type has a default");
            let placeholder = default_fn();
            let result = segment.peek_stack_infos(1)[0].value_type.clone();
            let segment = std::rc::Rc::new(RefCell::new(
                RawFrame::new(segment, &input_shapes, &result, &self.types)
                    .map_err(|e| ParseError::new(e.to_string(), cell_span))?,
            ));
            let clamp_segment = std::rc::Rc::clone(&segment);
            let bounds_segment = std::rc::Rc::clone(&segment);
            let clamp_fn = range_shape.clamp_fn;
            let bounds_fn = range_shape.bounds_fn;

            return Ok(adam_rs::Filter::range(
                value_type_id,
                arg_ids,
                arg_type_ids,
                move |value, args| {
                    clamp_fn(
                        clamp_segment.borrow_mut().call_with_prefix(value, args)?,
                        value,
                    )
                },
                move |args| {
                    bounds_fn(
                        bounds_segment
                            .borrow_mut()
                            .call_with_prefix(placeholder.as_ref(), args)
                            .ok()?,
                    )
                    .ok()
                },
            ));
        }

        if !underscore_used {
            return Err(ParseError::new(
                "filter must reference `_` (the value being filtered)",
                cell_span,
            ));
        }

        if !segment.peek_stack_infos(1)[0]
            .value_type
            .same_shape(&self.types.value_type(declared_shape))
        {
            return Err(ParseError::new(
                format!(
                    "cell `{cell_name}`: filter must produce `{}`",
                    self.types.display_name(declared_shape)
                ),
                cell_span,
            ));
        }

        let frame = RefCell::new(
            Frame::new(
                segment,
                &input_shapes,
                std::slice::from_ref(declared_shape),
                false,
                &self.types,
            )
            .map_err(|e| ParseError::new(e.to_string(), cell_span))?,
        );

        Ok(adam_rs::Filter::new(
            value_type_id,
            arg_ids,
            arg_type_ids,
            move |value, args| {
                Ok(frame
                    .borrow_mut()
                    .call_with_prefix(value, args)?
                    .pop()
                    .expect("one result"))
            },
        ))
    }

    /// Prepares and evaluates a nullary expression into concrete leaf or canonical ownership.
    ///
    /// - Precondition: `segment` requires no pre-loaded arguments (a `cell` initializer never
    ///   has an input-cell scope pushed, unlike a `relationship` binding's, `out` declaration's, or
    ///   `require`ment's body).
    ///
    /// # Errors
    /// Rejects unregistered leaves, missing cell capabilities, or evaluation errors.
    /// - Complexity: O(n) preparation for n descriptor nodes, plus execution costs.
    fn eval_segment_boxed(&self, segment: DynSegment) -> Result<(TypeShape, Box<dyn Any>)> {
        let shape = self.segment_shape(&segment)?;
        let mut frame = Frame::new(
            segment,
            &[],
            std::slice::from_ref(&shape),
            false,
            &self.types,
        )
        .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
        let value = frame
            .call(&[])
            .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?
            .pop()
            .expect("one result");
        Ok((shape, value))
    }

    /// Infers complete recursive identity without executing the expression.
    ///
    /// # Errors
    /// Rejects absent results or unregistered leaves.
    /// - Complexity: O(n) in result descriptor nodes.
    fn segment_shape(&self, segment: &DynSegment) -> Result<TypeShape> {
        let info = segment
            .peek_stack_infos(1)
            .first()
            .ok_or_else(|| ParseError::new("expression produced no value", Span::call_site()))?;
        self.types
            .shape_of(&info.value_type)
            .map_err(|message| ParseError::new(message, Span::call_site()))
    }

    /// Validates an initializer's complete shape before any initializer effects.
    ///
    /// # Errors
    /// Reports a ranged type mismatch or missing cell capability.
    /// - Complexity: O(n) in result descriptor nodes.
    fn validate_initializer(
        &self,
        segment: &DynSegment,
        declared: Option<&TypeShape>,
        name: &str,
        span: Span,
    ) -> Result<()> {
        let actual = self.segment_shape(segment)?;
        if let Some(expected) = declared
            && expected != &actual
        {
            return Err(ParseError::new(
                format!(
                    "cell `{name}`: type mismatch: expected `{}`, got `{}`",
                    self.types.display_name(expected),
                    self.types.display_name(&actual)
                ),
                span,
            ));
        }
        if !matches!(actual, TypeShape::Named(_)) {
            CellValue::validate(&self.types.value_type(&actual))
                .map_err(|e| ParseError::new(e.to_string(), span))?;
        }
        Ok(())
    }

    /// Adds a prepared initializer as concrete leaf storage or private canonical ownership.
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
            _ => {
                let value = *boxed.downcast::<CellValue>().expect("canonical compound");
                ctx.sheet.add_cell(value)
            }
        };
        Ok((shape, cell_id))
    }

    /// Returns one live tuple element's type name in the same registered naming scheme
    /// [`TypeRegistry::display_name`] uses, recursing into nested tuples. Falls back to the
    /// value's own Rust type path for an unregistered leaf, which has no registered name.
    ///
    /// - Complexity: O(n) in the number of (nested) elements.
    fn associated_display_name(&self, elem: &cel_runtime::AssociatedType) -> String {
        self.types
            .shape_of(&elem.value_type)
            .map(|shape| self.types.display_name(&shape))
            .unwrap_or_else(|_| elem.value_type.type_name().to_string())
    }

    /// Validates an entire default cell before invoking any registered defaults.
    ///
    /// Missing defaults take precedence over missing clone/equality capabilities.
    /// # Errors
    /// Names the first missing default in source order, or an unavailable cell capability.
    /// - Complexity: O(n) in shape nodes.
    fn validate_default_cell(&self, shape: &TypeShape, span: Span) -> Result<()> {
        self.types
            .validate_default(shape)
            .and_then(|()| CellValue::validate(&self.types.value_type(shape)))
            .map_err(|error| ParseError::new(error.to_string(), span))
    }

    /// Builds a default-valued cell for `shape` (scalar or compound, recursively), adding it to
    /// `ctx.sheet`.
    ///
    /// - Complexity: O(n) in shape nodes, plus registered default construction costs.
    ///
    /// # Errors
    /// Rejects missing defaults or cell capabilities before invoking any defaults or adding a cell.
    fn build_default_cell(
        &self,
        shape: &TypeShape,
        span: Span,
        ctx: &mut ParseContext,
    ) -> Result<CellId> {
        self.validate_default_cell(shape, span)?;
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
            _ => {
                let mut body = DynSegment::new::<()>();
                self.types
                    .emit_default(shape, &mut body)
                    .map_err(|e| ParseError::new(e.to_string(), span))?;
                self.build_cell_from_segment(body, ctx)
                    .map(|(_, id)| id)
                    .map_err(|error| ParseError::new(error.message(), span))
            }
        }
    }

    /// Evaluates `segment` and adds concrete leaf or canonical compound source storage.
    /// Mirrors
    /// [`build_cell_from_segment`](Self::build_cell_from_segment) exactly, routed through
    /// `Sheet::add_source` instead of `Sheet::add_cell`.
    ///
    /// - Complexity: O(n) in shape nodes, plus registered default construction costs.
    ///
    /// - Precondition: see [`eval_segment_boxed`](Self::eval_segment_boxed).
    ///
    /// # Errors
    /// See [`eval_segment_boxed`](Self::eval_segment_boxed).
    fn build_source_cell_from_segment(
        &self,
        segment: DynSegment,
        ctx: &mut ParseContext,
    ) -> Result<(TypeShape, CellId)> {
        let (shape, boxed) = self.eval_segment_boxed(segment)?;
        let cell_id = match &shape {
            TypeShape::Named(type_id) => {
                let entry = self.types.entry_by_type_id(*type_id).expect("registered");
                (entry.add_source_fn)(&mut ctx.sheet, boxed)
            }
            _ => {
                let value = *boxed.downcast::<CellValue>().expect("canonical compound");
                ctx.sheet.add_source(value)
            }
        };
        Ok((shape, cell_id))
    }

    /// Builds a default-valued *source* cell for `shape` (scalar or compound, recursively), adding
    /// it to `ctx.sheet` (see [`Sheet::add_source`]). Mirrors
    /// [`build_default_cell`](Self::build_default_cell) exactly, routed through
    /// `Sheet::add_source` instead of `Sheet::add_cell`.
    ///
    /// # Errors
    /// Rejects missing defaults or cell capabilities before invoking any defaults or adding a cell.
    fn build_default_source_cell(
        &self,
        shape: &TypeShape,
        span: Span,
        ctx: &mut ParseContext,
    ) -> Result<CellId> {
        self.validate_default_cell(shape, span)?;
        match shape {
            TypeShape::Named(type_id) => {
                let entry = self.types.entry_by_type_id(*type_id).expect(
                    "build_default_source_cell: type registered (resolved via TypeRegistry)",
                );
                let default_fn = entry.default_fn.ok_or_else(|| {
                    ParseError::new(
                        format!("type `{}` has no default; provide `= ...`", entry.type_name),
                        span,
                    )
                })?;
                Ok((entry.add_source_fn)(&mut ctx.sheet, default_fn()))
            }
            _ => {
                let mut body = DynSegment::new::<()>();
                self.types
                    .emit_default(shape, &mut body)
                    .map_err(|e| ParseError::new(e.to_string(), span))?;
                self.build_source_cell_from_segment(body, ctx)
                    .map(|(_, id)| id)
                    .map_err(|error| ParseError::new(error.message(), span))
            }
        }
    }

    /// Delegates one `type_expr` to CELParser, sharing the token stream.
    fn parse_type_expr(&mut self, ctx: &mut ParseContext) -> Result<crate::ast::TypeExpr> {
        let tokens = ctx.cursor.take_tokens().expect("tokens present");
        self.cel.set_lex_tokens(tokens);
        let result = self.cel.parse_type_expression();
        ctx.cursor
            .set_tokens(self.cel.take_lex_tokens().expect("tokens set"));
        result
    }

    /// `relationship_decl = "relationship" "{" { binding } "}".`
    ///
    /// - Postcondition: the returned `RelationshipId` identifies the relationship just added to
    ///   `ctx.sheet`.
    /// - Postcondition: on success, `ctx.method_spans` gains one entry per parsed binding,
    ///   keyed by `(rel_id, binding_index)`, mapping to that binding's source span, and
    ///   `ctx.relationship_spans` gains one entry mapping `rel_id` to the whole block's span.
    /// - Postcondition: on failure, the returned error's span covers the first binding it
    ///   implicates (or the whole block, if none), with every other resolvable
    ///   binding/cell attached as a secondary label (see `error_labels::site_label`).
    fn parse_relationship_decl(&mut self, ctx: &mut ParseContext) -> Result<RelationshipId> {
        let block_start = ctx.peek_span();
        ctx.is_keyword("relationship"); // consume
        ctx.expect_open_brace()?;
        let mut methods = Vec::new();
        let mut spans: Vec<(Span, Span)> = Vec::new();
        while !ctx.at_close_brace() {
            let (method, start, end) = self.parse_binding(ctx)?;
            methods.push(method);
            spans.push((start, end));
        }
        let close_span = ctx.expect_close_brace()?;
        match ctx.sheet.add_relationship(methods) {
            Ok(rel_id) => {
                ctx.relationship_spans.insert(
                    rel_id,
                    SourceSpan::from_proc_macro2_range(block_start, close_span),
                );
                for (idx, (start, end)) in spans.into_iter().enumerate() {
                    ctx.method_spans.insert(
                        (rel_id, idx),
                        SourceSpan::from_proc_macro2_range(start, end),
                    );
                }
                Ok(rel_id)
            }
            Err(e) => {
                let by_id: HashMap<CellId, String> = ctx
                    .cell_names
                    .iter()
                    .map(|(n, (id, _))| (*id, n.clone()))
                    .collect();
                let name = |id: CellId| by_id.get(&id).cloned();

                // The primary site is the first binding this error implicates; every other
                // resolvable site (another binding, or a cell recorded so far) becomes a
                // secondary label. Falls back to the whole block when no site resolves — e.g.
                // `InvalidMethod` for an empty relationship body carries no sites at all.
                //
                // Resolved strictly from `sites().first()`, not by scanning for the first
                // `MethodIndex` anywhere in the list: every `Error` variant `add_relationship`
                // currently returns with a `MethodIndex` identifying the primary offending
                // binding. Making that dependence explicit means variants that lead with some
                // other site kind fail safe here (fall back to the whole block) instead of this
                // code silently treating a later, unrelated `MethodIndex` site as primary.
                let primary_idx = match e.sites().first() {
                    Some(ErrorSite::MethodIndex(i)) => Some(*i),
                    _ => None,
                };
                let (start, end) = primary_idx
                    .and_then(|i| spans.get(i).copied())
                    .unwrap_or((block_start, close_span));
                let secondary: Vec<cel_parser::SpanLabel> = e
                    .sites()
                    .iter()
                    .enumerate()
                    .filter_map(|(i, s)| {
                        let sp = match s {
                            ErrorSite::MethodIndex(idx) if Some(*idx) != primary_idx => spans
                                .get(*idx)
                                .map(|(st, en)| SourceSpan::from_proc_macro2_range(*st, *en)),
                            ErrorSite::Cell(c) => ctx.cell_spans.get(c).copied(),
                            _ => None,
                        }?;
                        Some(cel_parser::SpanLabel {
                            span: sp,
                            label: crate::error_labels::site_label(&e, i, &name),
                        })
                    })
                    .collect();
                Err(ParseError::new_range(e.to_string(), start, end).with_secondary(secondary))
            }
        }
    }

    /// `binding = binding_target ":=" expression ";".`
    ///
    /// - Postcondition: the returned `Span`s bound the binding's full source range, from its
    ///   first token through the terminating `;`, for use in error-location resolution.
    fn parse_binding(&mut self, ctx: &mut ParseContext) -> Result<(Method, Span, Span)> {
        let start_span = ctx.peek_span();
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
        let end_span = ctx.expect_punct(";")?;
        let compiled = self.compile_outputs(ctx, &segment, &outputs, destructure)?;
        Ok((
            build_method(inputs, outputs, segment, compiled, &self.types)?,
            start_span,
            end_span,
        ))
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
        self.parse_deduced_expr_restricted(ctx, cel_parser::ExpressionRestriction::None)
    }

    /// Compiles a dependency-deducing expression with the enclosing header's brace reservation.
    ///
    /// - Postcondition: temporary dependency scope is removed on success and error.
    /// - Complexity: O(k + n) for k declared cells and n expression tokens.
    ///
    /// # Errors
    /// Returns expression syntax, unknown cell, or operation/type errors.
    fn parse_deduced_expr_restricted(
        &mut self,
        ctx: &mut ParseContext,
        restriction: cel_parser::ExpressionRestriction,
    ) -> Result<(DynSegment, NamedCells)> {
        // Precompute how to push each currently-declared cell, keyed by name. Built before
        // the scope closure captures anything, since `push_scope` requires `'static` (the
        // closure can't borrow `self.types`).
        let push_table: std::collections::HashMap<String, (CellId, TypeShape, InputPush)> = ctx
            .cell_names
            .iter()
            .map(|(name, (cell_id, shape))| {
                let push = self.types.value_type(shape);
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
                segment.push_argument(idx, push.clone())?;
                Ok(true)
            });

        let result = self.parse_cel_expression_restricted(ctx, restriction);
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
                let push = self.types.value_type(shape);
                (name.clone(), (*cell_id, shape.clone(), push))
            })
            .collect();

        let value_push = self.types.value_type(declared_shape);

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
                    segment.push_argument(0, value_push.clone())?;
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
                segment.push_argument(idx, push.clone())?;
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
        let (segment, inputs) = self.parse_deduced_expr_restricted(
            ctx,
            cel_parser::ExpressionRestriction::NoStructLiteral,
        )?;
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

        let shape = self.segment_shape(&segment)?;
        let eq_fn = match &shape {
            TypeShape::Named(id) => {
                self.types
                    .entry_by_type_id(*id)
                    .expect("registered")
                    .eq_dyn_fn
            }
            _ => compound_eq,
        };
        let frame = RefCell::new(
            Frame::new(
                segment,
                &inputs
                    .iter()
                    .map(|(_, _, shape)| shape.clone())
                    .collect::<Vec<_>>(),
                std::slice::from_ref(&shape),
                false,
                &self.types,
            )
            .map_err(|e| ParseError::new(e.to_string(), match_span))?,
        );
        let match_expr = MatchExpr::new(
            input_ids,
            input_types,
            cell_type_id(&shape),
            eq_fn,
            move |args| Ok(frame.borrow_mut().call(args)?.pop().expect("one result")),
        );
        Ok((shape, match_expr))
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

            // Named branch: `literal_pattern "=>" "{" ... "}"` — a bare literal, optionally
            // negated by a leading `-`, matching Rust's own `LiteralPattern` rule. CEL has no
            // constant-expression syntax in pattern position (see
            // https://doc.rust-lang.org/reference/patterns.html), so `0i32 + 1 =>` is rejected
            // here exactly as it already is by the CST parser's `consume_literal_pattern`.
            let branch_span = ctx.peek_span();
            let segment = self.parse_literal_pattern(ctx)?;
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
                    .map_err(|e| Self::sheet_error(ctx, match_span, e))?;
            }
            _ => {
                let typed_branches: Vec<(Vec<CellValue>, Vec<RelationshipId>)> = branches
                    .into_iter()
                    .map(|(val, rel_ids)| {
                        let seq = *val.downcast::<CellValue>().expect("canonical compound");
                        (vec![seq], rel_ids)
                    })
                    .collect();
                ctx.sheet
                    .add_conditional::<CellValue>(match_expr, typed_branches, default_rel_ids)
                    .map_err(|e| Self::sheet_error(ctx, match_span, e))?;
            }
        }

        Ok(())
    }

    /// Turns a `Sheet` construction failure into a `ParseError` with `primary` as the primary
    /// span and every one of `e`'s `Relationship`/`Cell` sites that resolves against `ctx`'s
    /// span tables as a secondary label.
    ///
    /// - Complexity: O(s) in the number of `e`'s sites.
    fn sheet_error(ctx: &ParseContext, primary: Span, e: adam_rs::Error) -> ParseError {
        let by_id: HashMap<CellId, String> = ctx
            .cell_names
            .iter()
            .map(|(n, (id, _))| (*id, n.clone()))
            .collect();
        let name = |id: CellId| by_id.get(&id).cloned();
        let secondary: Vec<cel_parser::SpanLabel> = e
            .sites()
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let sp = match s {
                    ErrorSite::Relationship(r) => ctx.relationship_spans.get(r).copied(),
                    ErrorSite::Cell(c) => ctx.cell_spans.get(c).copied(),
                    _ => None,
                }?;
                Some(cel_parser::SpanLabel {
                    span: sp,
                    label: crate::error_labels::site_label(&e, i, &name),
                })
            })
            .collect();
        let message = e.to_string();
        ParseError::new(message, primary).with_secondary(secondary)
    }

    /// Turns a post-construction validation failure into a `ParseError`.
    ///
    /// The first resolvable site is the primary label, which for `DependencyCycle` is the
    /// governed cell declaration. Remaining resolvable sites become secondary labels.
    ///
    /// - Complexity: O(C + s), where C is the number of declared cells (each name is
    ///   copied into a lookup map) and s is the number of `e`'s sites.
    fn validated_sheet_error(ctx: &ParseContext, e: adam_rs::Error) -> ParseError {
        let by_id: HashMap<CellId, String> = ctx
            .cell_names
            .iter()
            .map(|(n, (id, _))| (*id, n.clone()))
            .collect();
        let name = |id: CellId| by_id.get(&id).cloned();
        let primary_index = e.sites().iter().position(|s| match s {
            ErrorSite::Cell(c) => ctx.cell_token_spans.contains_key(c),
            _ => false,
        });
        let primary = primary_index
            .and_then(|i| match e.sites()[i] {
                ErrorSite::Cell(c) => ctx.cell_token_spans.get(&c).copied(),
                _ => None,
            })
            .unwrap_or_else(Span::call_site);
        let secondary: Vec<cel_parser::SpanLabel> = e
            .sites()
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != primary_index)
            .filter_map(|(i, s)| {
                let span = match s {
                    ErrorSite::Relationship(r) => ctx.relationship_spans.get(r).copied(),
                    ErrorSite::Cell(c) => ctx.cell_spans.get(c).copied(),
                    _ => None,
                }?;
                Some(cel_parser::SpanLabel {
                    span,
                    label: crate::error_labels::site_label(&e, i, &name),
                })
            })
            .collect();
        ParseError::new(e.to_string(), primary).with_secondary(secondary)
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

    /// `out_decl = "out" identifier [ ":" type_expr ] ":=" expression [ cell_filter ] [ "require"
    /// "{" { requirement } "}" ] ";".`
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
        let actual_shape = self.segment_shape(&segment)?;

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

        if !matches!(out_shape, TypeShape::Named(_)) {
            CellValue::validate(&self.types.value_type(&out_shape))
                .map_err(|error| ParseError::new(error.to_string(), name_span))?;
        }
        let cell_id = self.build_default_cell(&out_shape, name_span, ctx)?;
        ctx.cell_spans
            .insert(cell_id, SourceSpan::from_proc_macro2(name_span));
        ctx.cell_token_spans.insert(cell_id, name_span);

        let filter = if ctx.is_keyword("filter") {
            Some(self.parse_cell_filter(ctx, &name, name_span, &out_shape)?)
        } else {
            None
        };

        ctx.cell_names
            .insert(name.clone(), (cell_id, out_shape.clone()));

        let compiled = CompiledOutputs::Single;
        let writer = build_method(
            inputs,
            vec![(name.clone(), cell_id, out_shape)],
            segment,
            compiled,
            &self.types,
        )?;

        let requirements = if ctx.is_keyword("require") {
            self.parse_requirement_block(ctx)?
        } else {
            Vec::new()
        };

        ctx.expect_punct(";")?;

        let out_cell = ctx
            .sheet
            .add_out(writer, Vec::new())
            .map_err(|e| ParseError::new(e.to_string(), name_span))?;
        for (req_name, requirement) in requirements {
            Self::register_requirement(ctx, out_cell, req_name, requirement, name_span)?;
        }
        if let Some(filter) = filter {
            ctx.sheet
                .add_filter(out_cell, filter)
                .map_err(|e| Self::sheet_error(ctx, name_span, e))?;
        }
        ctx.output_names.insert(name, out_cell);

        Ok(())
    }

    /// `requirement_block = "{" { requirement } "}".`
    ///
    /// # Errors
    /// Returns a parse error for invalid requirements or repeated explicit labels within
    /// this declaration; unnamed requirements do not participate in duplicate detection.
    ///
    /// - Complexity: Expected O(n) bookkeeping for n requirements, plus compilation cost.
    fn parse_requirement_block(
        &mut self,
        ctx: &mut ParseContext,
    ) -> Result<Vec<(Option<String>, Requirement)>> {
        ctx.expect_open_brace()?;
        let mut requirements = Vec::new();
        let mut names = HashSet::new();
        while !ctx.at_close_brace() {
            let span = ctx
                .peek_token()
                .ok_or_else(|| ParseError::new("unexpected end of input", Span::call_site()))?
                .span();
            let (name, requirement) = self.parse_requirement(ctx)?;
            if let Some(label) = &name
                && !names.insert(label.clone())
            {
                return Err(ParseError::new(
                    format!("duplicate requirement `{label}`"),
                    span,
                ));
            }
            requirements.push((name, requirement));
        }
        ctx.expect_close_brace()?;
        Ok(requirements)
    }

    /// Attaches a requirement and records its explicit label against the returned ID.
    ///
    /// # Errors
    /// Returns a parse error at `span` when sheet registration fails.
    ///
    /// - Complexity: The cost of `Sheet::add_requirement`, plus expected O(1) map insertion.
    fn register_requirement(
        ctx: &mut ParseContext,
        cell: CellId,
        name: Option<String>,
        requirement: Requirement,
        span: Span,
    ) -> Result<()> {
        let id = ctx
            .sheet
            .add_requirement(cell, requirement)
            .map_err(|e| ParseError::new(e.to_string(), span))?;
        if let Some(name) = name {
            ctx.requirement_names.insert(id, name);
        }
        Ok(())
    }

    /// `requirement = [ "@" identifier ] expression ";".`
    fn parse_requirement(
        &mut self,
        ctx: &mut ParseContext,
    ) -> Result<(Option<String>, Requirement)> {
        let name = if ctx.consume_punct("@") {
            let (name, _name_span) = ctx.consume_ident()?;
            Some(name)
        } else {
            None
        };
        let (segment, inputs) = self.parse_deduced_expr(ctx)?;
        ctx.expect_punct(";")?;

        let bool_type_id = TypeId::of::<bool>();
        let actual_type_id = segment.peek_output_type_id().ok_or_else(|| {
            ctx.err_at(match &name {
                Some(name) => format!("requirement `{name}`: expression produced no value"),
                None => "requirement: expression produced no value".to_string(),
            })
        })?;
        if actual_type_id != bool_type_id {
            let got = self
                .types
                .entry_by_type_id(actual_type_id)
                .map(|e| e.type_name)
                .unwrap_or("?");
            return Err(ctx.err_at(match &name {
                Some(name) => format!("requirement `{name}`: expected `bool`, got `{got}`"),
                None => format!("requirement: expected `bool`, got `{got}`"),
            }));
        }

        let input_ids: Vec<CellId> = inputs.iter().map(|(_, id, _)| *id).collect();
        let input_types: Vec<TypeId> = inputs
            .iter()
            .map(|(_, _, shape)| cell_type_id(shape))
            .collect();
        let frame = RefCell::new(
            RawFrame::new(
                segment,
                &inputs
                    .iter()
                    .map(|(_, _, shape)| shape.clone())
                    .collect::<Vec<_>>(),
                &cel_runtime::ValueType::leaf::<bool>(),
                &self.types,
            )
            .map_err(|e| ctx.err_at(e.to_string()))?,
        );
        let requirement = Requirement::new(input_ids, input_types, move |args| {
            let value = frame.borrow_mut().call(args)?;
            // SAFETY: The complete result was bound as the concrete bool leaf.
            Ok(unsafe { value.into_typed_bound::<cel_runtime::Leaf<bool>>() })
        });

        Ok((name, requirement))
    }

    /// Determines how to split a compiled body segment's result across `outputs`, given their
    /// declared shapes — used by `parse_binding` to dispatch a `relationship` binding's direct-bind vs.
    /// destructuring cases against a single compiled `expression`. Written generically so any
    /// future N-output construct can reuse it; `out` declarations are always single-output,
    /// never destructuring, and currently use their own simpler, separate dispatch instead.
    ///
    /// Direct bindings move the whole result, including source unit. Destructuring requires
    /// a tuple of exactly matching recursive field shapes and moves its field owners.
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
            let actual = self.segment_shape(segment)?;
            if &actual != out_shape {
                return Err(ctx.err_at(format!(
                    "output `{out_name}`: type mismatch: expected `{}`, got `{}`",
                    self.types.display_name(out_shape),
                    self.types.display_name(&actual)
                )));
            }
            Ok(CompiledOutputs::Single)
        } else {
            let arity = segment.peek_tuple_arity().unwrap_or(0);
            if arity != outputs.len() {
                return Err(ctx.err_at(format!(
                    "output expression has arity {arity} but {} output(s) declared",
                    outputs.len()
                )));
            }
            let associated = segment.peek_stack_infos(1)[0]
                .value_type
                .tuple_elements()
                .map(<[_]>::to_vec)
                .unwrap_or_default();
            for (i, ((out_name, _, out_shape), elem)) in outputs.iter().zip(&associated).enumerate()
            {
                if !self
                    .types
                    .value_type(out_shape)
                    .same_shape(&elem.value_type)
                {
                    return Err(ctx.err_at(format!(
                        "output {i} `{out_name}`: type mismatch: expected `{}`, got `{}`",
                        self.types.display_name(out_shape),
                        self.associated_display_name(elem)
                    )));
                }
            }
            Ok(CompiledOutputs::Tuple)
        }
    }

    /// Delegates one `expression` to CELParser, sharing the token stream.
    fn parse_cel_expression(&mut self, ctx: &mut ParseContext) -> Result<DynSegment> {
        self.parse_cel_expression_restricted(ctx, cel_parser::ExpressionRestriction::None)
    }

    /// Delegates a header expression while leaving its body brace in the shared token stream.
    ///
    /// - Complexity: O(n) in expression tokens.
    ///
    /// # Errors
    /// Returns the embedded CEL expression's syntax or type error.
    fn parse_cel_expression_restricted(
        &mut self,
        ctx: &mut ParseContext,
        restriction: cel_parser::ExpressionRestriction,
    ) -> Result<DynSegment> {
        let tokens = ctx.cursor.take_tokens().expect("tokens present");
        self.cel.set_lex_tokens(tokens);
        let result = self.cel.parse_expression_restricted(restriction);
        ctx.cursor
            .set_tokens(self.cel.take_lex_tokens().expect("tokens set"));
        let mut segment = result?;
        if let Some(info) = segment.peek_stack_infos(1).first() {
            let enriched = self
                .types
                .enrich_value_type(&info.value_type)
                .map_err(|error| ParseError::new(error.to_string(), Span::call_site()))?;
            segment
                .annotate_top(enriched)
                .map_err(|error| ParseError::new(error.to_string(), Span::call_site()))?;
        }
        Ok(segment)
    }

    /// Delegates one `literal_pattern` (a bare literal, optionally negated by a leading `-`) to
    /// CELParser, sharing the token stream.
    fn parse_literal_pattern(&mut self, ctx: &mut ParseContext) -> Result<DynSegment> {
        let tokens = ctx.cursor.take_tokens().expect("tokens present");
        self.cel.set_lex_tokens(tokens);
        let result = self.cel.parse_literal_pattern();
        ctx.cursor
            .set_tokens(self.cel.take_lex_tokens().expect("tokens set"));
        result
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Complete metadata for an input cell's canonical borrowed argument read.
type InputPush = cel_runtime::ValueType;

/// Compares private compound conditional keys at an already validated boundary.
///
/// - Complexity: O(v) for v contained values.
fn compound_eq(a: &dyn Any, b: &dyn Any) -> bool {
    a.downcast_ref::<CellValue>() == b.downcast_ref::<CellValue>()
}

/// How to turn one compiled `expression`'s result into per-output values.
enum CompiledOutputs {
    /// Moves the segment's whole result into one cell.
    Single,
    /// Moves the segment's tuple fields into source-ordered output cells.
    Tuple,
}

/// Builds a [`Method`] from parsed inputs, outputs, the compiled body segment, and how
/// to split its result across `outputs`.
fn build_method(
    inputs: NamedCells,
    outputs: NamedCells,
    segment: DynSegment,
    compiled: CompiledOutputs,
    types: &TypeRegistry,
) -> Result<Method> {
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

    let frame = RefCell::new(
        Frame::new(
            segment,
            &inputs.iter().map(|(_, _, s)| s.clone()).collect::<Vec<_>>(),
            &outputs
                .iter()
                .map(|(_, _, s)| s.clone())
                .collect::<Vec<_>>(),
            matches!(compiled, CompiledOutputs::Tuple),
            types,
        )
        .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?,
    );
    Ok(Method::new(
        input_ids,
        output_ids,
        input_types,
        output_types,
        move |inputs| frame.borrow_mut().call(inputs),
    ))
}

/// Flattens a declared cell's `TypeShape` to the single `TypeId` `adam_rs` itself needs for its
/// own (CEL-agnostic) storage bookkeeping. Compound shape identity remains separate from
/// the private common storage carrier and is validated before constructing any adapter.
fn cell_type_id(shape: &TypeShape) -> TypeId {
    match shape {
        TypeShape::Named(type_id) => *type_id,
        _ => TypeId::of::<CellValue>(),
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
    use cel_runtime::{ArrayOf, Leaf};

    fn parser() -> AdamParser {
        AdamParser::new(TypeRegistry::new(), OpLookup::new())
    }

    #[derive(Clone, Debug, PartialEq)]
    struct Custom(i32);

    #[derive(Clone)]
    struct CountFn;

    fn custom_lookup() -> OpLookup {
        let mut lookup = OpLookup::new();
        lookup.push_scope(|name, segment, arity, _span| match name {
            "left" if arity == 0 => {
                segment.op0(|| Custom(1));
                Ok(true)
            }
            "right" if arity == 0 => {
                segment.op0(|| Custom(2));
                Ok(true)
            }
            "count" if arity == 0 => {
                segment.op0(|| CountFn);
                Ok(true)
            }
            "()" if arity == 2 => {
                if segment.peek_stack_infos(1)[0]
                    .value_type
                    .array_element()
                    .is_some_and(|element| element.array_element().is_some())
                {
                    segment
                        .op2_repr::<Leaf<CountFn>, ArrayOf<ArrayOf<Leaf<Custom>>>, Leaf<i32>, _>(
                            |_callee, array| array.len() as i32,
                        )?;
                } else {
                    segment.op2_repr::<Leaf<CountFn>, ArrayOf<Leaf<Custom>>, Leaf<i32>, _>(
                        |_callee, array| array.len() as i32,
                    )?;
                }
                Ok(true)
            }
            _ => Ok(false),
        });
        lookup
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
        let err = result.expect_err("expected Err");
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
    fn parse_cell_initializer_supports_registry_typed_arrays() {
        let mut reg = TypeRegistry::new();
        reg.register_no_default::<Custom>("Custom");
        let mut parser = AdamParser::new(reg, custom_lookup());

        let parsed = parser
            .parse_str(
                "sheet s { \
                    cell values: i32 = count([left, right]: [Custom]); \
                    cell empty: i32 = count([]: [Custom]); \
                    cell nested: i32 = count([[]: [Custom]]: [[Custom]]); \
                }",
            )
            .unwrap();

        let (values_id, _) = parsed.cell_names["values"];
        assert_eq!(*parsed.read::<i32>(values_id).unwrap(), 2);

        let (empty_id, _) = parsed.cell_names["empty"];
        assert_eq!(*parsed.read::<i32>(empty_id).unwrap(), 0);

        let (nested_id, _) = parsed.cell_names["nested"];
        assert_eq!(*parsed.read::<i32>(nested_id).unwrap(), 1);
    }

    #[test]
    fn parse_cell_initializer_rejects_unknown_registry_typed_arrays() {
        let mut parser = AdamParser::new(TypeRegistry::new(), custom_lookup());
        let err = parser
            .parse_str("sheet s { cell values = []: [Custom]; }")
            .expect_err("unknown custom array annotations must fail");

        assert!(
            err.message().contains("unknown type `Custom`"),
            "got: {}",
            err.message()
        );
    }

    #[test]
    fn parse_cell_initializer_preserves_persistent_array_cells() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let parsed = parser
            .parse_str("sheet s { cell values = []: [(i32, f64)]; }")
            .unwrap();
        type Pairs = cel_runtime::ArrayOf<
            cel_runtime::Cons<
                cel_runtime::Leaf<i32>,
                cel_runtime::Cons<cel_runtime::Leaf<f64>, cel_runtime::Nil>,
            >,
        >;
        assert!(
            parsed
                .read_compound::<Pairs>(parsed.cell_names["values"].0)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn parse_source_decl_registers_a_source_kind_cell() {
        let sheet = parser()
            .parse_str("sheet s { source width: i32 = 4; }")
            .unwrap();
        let (width, _) = sheet.cell_names["width"];
        assert_eq!(sheet.cell_kind(width), Some(adam_rs::CellKind::Source));
    }

    #[test]
    fn parse_source_decl_requires_colon_or_equals() {
        let result = parser().parse_str("sheet s { source width; }");
        assert!(result.is_err());
    }

    #[test]
    fn parse_cell_decl_with_a_require_block_attaches_requirements() {
        let sheet = parser()
            .parse_str("sheet s { cell x: i32 = 5 require { @positive x > 0; }; }")
            .unwrap();
        let (x, _) = sheet.cell_names["x"];
        assert_eq!(sheet.cell_requirements(x).unwrap().len(), 1);
    }

    #[test]
    fn parse_source_decl_with_a_require_block_attaches_requirements() {
        let sheet = parser()
            .parse_str("sheet s { source x: i32 = 5 require { @positive x > 0; }; }")
            .unwrap();
        let (x, _) = sheet.cell_names["x"];
        assert_eq!(sheet.cell_requirements(x).unwrap().len(), 1);
    }

    #[test]
    fn parse_source_decl_with_a_filter_clause_attaches_a_filter() {
        let sheet = parser()
            .parse_str("sheet s { source x: i32 = 5 filter 0..=10; }")
            .unwrap();
        let (x, _) = sheet.cell_names["x"];
        assert!(sheet.filter_kind(x).is_some());
    }

    #[test]
    fn parse_out_decl_with_a_filter_clause_attaches_a_filter() {
        let sheet = parser()
            .parse_str("sheet s { cell width: i32 = 4; out area := width filter 0..=100; }")
            .unwrap();
        let area = sheet.output_names["area"];
        assert!(sheet.filter_kind(area).is_some());
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
        parsed.sheet.propagate().unwrap();
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
        parsed.sheet.propagate().unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 100);

        parsed.sheet.write(hi_id, 10i32).unwrap();
        parsed.sheet.write(a_id, 500i32).unwrap();
        parsed.sheet.propagate().unwrap();
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
        parsed.sheet.propagate().unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 20);
    }

    #[test]
    fn filter_dependency_cycle_reported_with_secondary_labels() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let ok_source = "sheet s {\n    cell b: i32 = 0;\n    cell a: i32 = 5;\n    relationship { b := a; }\n    cell c: i32 = 0 filter _ + b - b;\n}";
        assert!(parser.parse_str(ok_source).is_ok());

        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        // The parser only lets a filter reference already-declared cells, so the cycle becomes
        // visible when the later relationship adds the reverse dependency edge.
        let source = "sheet s {\n    cell b: i32 = 0;\n    cell a: i32 = 5 filter _ + b - b;\n    relationship { b := a; }\n}";
        let err = parser.parse_str(source).unwrap_err();
        assert_eq!(
            err.message(),
            "a filter or conditional depends on a cell it governs (dependency cycle)"
        );
        assert!(!err.secondary().is_empty());
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
    fn cell_filter_with_a_range_inclusive_body_clamps_on_write() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str("sheet s { cell a: i32 filter 0..=100; }")
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        assert!(matches!(
            parsed.sheet.filter_kind(a_id),
            Some(adam_rs::FilterKind::Range { .. })
        ));
        parsed.sheet.write(a_id, 500i32).unwrap();
        parsed.sheet.propagate().unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 100);
    }

    #[test]
    fn cell_filter_range_does_not_require_underscore() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let result = parser.parse_str("sheet s { cell a: i32 filter 0..=100; }");
        assert!(result.is_ok());
    }

    #[test]
    fn cell_filter_range_bounds_track_cell_dependencies_live() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str(
                "sheet s { cell lo: i32 = 0; cell hi: i32 = 100; cell a: i32 filter lo..=hi; }",
            )
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        let (hi_id, _) = parsed.cell_names["hi"];
        assert_eq!(parsed.sheet.filter_range::<i32>(a_id), Some((0, 100)));
        parsed.sheet.write(hi_id, 10i32).unwrap();
        assert_eq!(parsed.sheet.filter_range::<i32>(a_id), Some((0, 10)));
        parsed.sheet.write(a_id, 500i32).unwrap();
        parsed.sheet.propagate().unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 10);
    }

    #[test]
    fn cell_filter_range_with_float_cell_type_works() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str("sheet s { cell a: f64 filter 0.0..=100.0; }")
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        parsed.sheet.write(a_id, 500.0f64).unwrap();
        parsed.sheet.propagate().unwrap();
        assert_eq!(*parsed.sheet.read::<f64>(a_id).unwrap(), 100.0);
    }

    #[test]
    fn cell_filter_range_with_mismatched_element_type_is_a_parse_error() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let err = parser
            .parse_str("sheet s { cell a: f64 filter 0..=100; }")
            .expect_err("expected Err");
        assert!(
            err.message().contains("filter range bounds must be"),
            "{}",
            err.message()
        );
    }

    #[test]
    fn cell_filter_range_with_mismatched_element_type_and_underscore_is_still_a_range_error() {
        // Confirms the "range bounds must be" check fires unconditionally on a recognized
        // RangeInclusive-shaped output, even though `_` is referenced here (cast to `i32`, the
        // range's own element type, rather than left at the cell's declared `f64`) — it must not
        // be swallowed by the opaque `underscore_used` path.
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let err = parser
            .parse_str("sheet s { cell a: f64 filter (_ as i32)..=100; }")
            .expect_err("expected Err");
        assert!(
            err.message().contains("filter range bounds must be"),
            "{}",
            err.message()
        );
    }

    #[test]
    fn cell_filter_range_returns_none_instead_of_panicking_when_a_bound_expression_fails_to_evaluate()
     {
        // `100 / hi` is a registered CEL builtin (`/` on `i32`, see
        // `cel_parser::op_table::OpLookup`) that returns `Err` at runtime on division by zero
        // rather than panicking or wrapping — `parse_cell_filter` only validates this range
        // filter's *type* at parse time, so evaluating the bound expression with `hi == 0` must
        // still be handled at runtime. Before this fix, `FilterKind::Range::bounds` was
        // infallible and `parse_cell_filter`'s wrapping closure used `.expect(..)`, which would
        // panic here instead of degrading to `None`.
        // `hi` starts at a non-zero value so `add_filter`'s own immediate clamp of `a`'s default
        // value (0) succeeds during parsing (`0..=(100 / 1)`); the failing evaluation is
        // triggered afterwards, purely through `filter_range`, by writing `hi` to 0.
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str("sheet s { cell hi: i32 = 1; cell a: i32 filter 0..=(100 / hi); }")
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        let (hi_id, _) = parsed.cell_names["hi"];

        assert_eq!(parsed.sheet.filter_range::<i32>(a_id), Some((0, 100)));

        // Puts the range expression's own evaluation into a failing state (division by zero)
        // without going through `add_filter`'s clamp path at all — this is exactly the "some
        // other write puts an endpoint's dependency into a bad state" scenario from the review
        // finding, and must degrade to `None` rather than panicking.
        parsed.sheet.write(hi_id, 0i32).unwrap();
        assert_eq!(parsed.sheet.filter_range::<i32>(a_id), None);
    }

    #[test]
    fn cell_filter_general_expression_still_compiles_to_opaque_kind() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let parsed = parser
            .parse_str("sheet s { cell a: i32 filter if _ < 0 { 0 } else { _ }; }")
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        assert!(matches!(
            parsed.sheet.filter_kind(a_id),
            Some(adam_rs::FilterKind::Opaque)
        ));
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
        parsed.sheet.propagate().unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 100);

        parsed.sheet.write(max_id, 10i32).unwrap();
        parsed.sheet.propagate().unwrap();
        parsed.sheet.write(a_id, 500i32).unwrap();
        parsed.sheet.propagate().unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 10);
    }

    #[test]
    fn cell_filter_on_a_tuple_typed_cell_is_supported() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let sheet = parser
            .parse_str("sheet s { cell a: (i32, i32) filter (_.0, _.1); }")
            .unwrap();
        assert!(sheet.filter_kind(sheet.cell_names["a"].0).is_some());
    }

    #[test]
    fn parse_filter_clause_attaches_a_filter() {
        let sheet = parser()
            .parse_str("sheet s { cell x: i32 = 0 filter 0..=10; }")
            .unwrap();
        let (x, _) = sheet.cell_names["x"];
        assert!(sheet.filter_kind(x).is_some());
    }

    #[test]
    fn parse_filter_clause_with_a_label_is_now_a_parse_error() {
        let result = parser().parse_str("sheet s { cell x: i32 = 0 filter clamp: 0..=10; }");
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

    /// Verifies member labels never become dependencies or resolve same-named cells.
    #[test]
    fn member_labels_are_not_cell_dependencies() {
        let mut lookup = OpLookup::new();
        lookup.push_scope(|name, segment, arity, _| {
            if matches!(name, "x" | "width") && arity == 1 {
                segment.op1(|value: f64| value * 2.0)?;
                return Ok(true);
            }
            Ok(false)
        });
        let mut sheet = AdamParser::new(TypeRegistry::new(), lookup)
            .parse_str(
                "sheet s { cell input: f64 = 3.0; cell x: bool = true; \
                 cell width: bool = false; cell result: f64 = 0.0; \
                 relationship { result := input.x().width; } }",
            )
            .unwrap();
        let input = sheet.cell_names["input"].0;
        let result = sheet.cell_names["result"].0;
        let (&(relationship, method), _) = sheet.method_spans.iter().next().unwrap();
        assert_eq!(
            sheet.method_inputs(relationship, method),
            Some([input].as_slice())
        );
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<f64>(result).unwrap(), 12.0);
        sheet.write(input, 5.0f64).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<f64>(result).unwrap(), 20.0);
    }

    /// Tracks only value expressions in a record-member binding.
    #[test]
    fn field_labels_and_member_names_are_not_cell_dependencies() {
        use cel_parser::{OperationSignature, RecordField, RecordSchema, ResolvedLeafType};
        use cel_runtime::ValueType;
        use std::sync::Arc;
        /// A record independent of any geometry library.
        #[derive(Clone, Debug, PartialEq)]
        struct TestPoint {
            x: f64,
            y: f64,
        }
        let point = ResolvedLeafType::new("TestPoint", ValueType::leaf::<TestPoint>());
        let scalar = ResolvedLeafType::new("f64", ValueType::leaf::<f64>());
        let mut lookup = OpLookup::new();
        lookup
            .register_record(RecordSchema {
                ty: point.clone(),
                fields: vec![
                    RecordField {
                        name: "x",
                        ty: scalar.clone().into(),
                    },
                    RecordField {
                        name: "y",
                        ty: scalar.clone().into(),
                    },
                ],
                emit: Arc::new(|s, order, _, _| {
                    assert_eq!(order, [0, 1]);
                    s.op2(|x: f64, y: f64| TestPoint { x, y })
                }),
            })
            .unwrap();
        lookup
            .register_operation(OperationSignature {
                name: "x",
                operands: vec![point.into()],
                result: scalar.into(),
                emit: Arc::new(|s, _| s.op1(|p: TestPoint| p.x)),
            })
            .unwrap();
        let mut sheet = AdamParser::new(TypeRegistry::new(), lookup)
            .parse_str(
                "sheet s { cell a: f64 = 2.0; cell b: f64 = 3.0; \
             cell x: bool = false; cell y: bool = false; cell TestPoint: bool = true; \
             cell result: f64 = 0.0; \
             relationship { result := TestPoint { x: a, y: b }.x; } }",
            )
            .unwrap();
        let a = sheet.cell_names["a"].0;
        let b = sheet.cell_names["b"].0;
        let result = sheet.cell_names["result"].0;
        let (&(relationship, method), _) = sheet.method_spans.iter().next().unwrap();
        assert_eq!(
            sheet.method_inputs(relationship, method),
            Some([a, b].as_slice())
        );
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<f64>(result).unwrap(), 2.0);
        sheet.write(a, 7.0f64).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<f64>(result).unwrap(), 7.0);
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
    fn mismatched_method_cells_error_spans_the_relationship_block_not_the_sheet() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let source = "sheet s {\n    cell a: i32;\n    cell b: i32;\n\n    relationship {\n        a := b;\n        b := 42;\n    }\n}";
        let err = parser.parse_str(source).err().unwrap();
        // The mismatched binding (`b := 42;`, method index 1) is on line 7; the sheet's
        // opening line (1) must not be reported instead. `ParseError::span()` returns a
        // `proc_macro2::Span`, so `.start()` is a method call here, not a field access (unlike
        // `cel_parser::SourceSpan`, whose `start`/`end` are public `LineColumn` fields).
        assert_eq!(err.span().start().line, 7);
    }

    #[test]
    fn successful_relationship_parse_populates_method_spans() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let source = "sheet s {\n    cell a: i32;\n    cell b: i32;\n\n    relationship {\n        a := b;\n    }\n}";
        let parsed = parser.parse_str(source).unwrap();
        assert_eq!(parsed.method_spans.len(), 1);
        let ((_, idx), span) = parsed.method_spans.iter().next().unwrap();
        assert_eq!(*idx, 0);
        assert_eq!(span.start.line, 6);
    }

    #[test]
    fn parse_populates_relationship_and_cell_spans() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let parsed = parser
            .parse_str("sheet s { cell a: i32 = 0; cell b: i32; relationship { b := a; } }")
            .unwrap();
        assert_eq!(parsed.cell_spans.len(), 2);
        assert_eq!(parsed.relationship_spans.len(), 1);
        // every declared cell id has a span
        for (_, (id, _)) in &parsed.cell_names {
            assert!(parsed.cell_spans.contains_key(id));
        }
    }

    #[test]
    fn locate_error_resolves_a_cycle_to_multiple_ordered_spans() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str(
                "sheet s { cell x: i32 = 0; cell y: i32 = 0; relationship { x := y + 1i32; } \
                 relationship { y := x + 1i32; } }",
            )
            .unwrap();
        let err = parsed.propagate().unwrap_err();
        assert!(matches!(err, adam_rs::Error::Cycle { .. }));
        let located = parsed.locate_error(&err);
        // both relationship blocks resolve to spans
        let rel_spans = located.len();
        assert!(
            rel_spans >= 2,
            "expected >=2 spans, got {rel_spans}: {located:?}"
        );
    }

    #[test]
    fn duplicate_output_set_error_underlines_both_bindings() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let source =
            "sheet s { cell a: i32 = 0; cell b: i32; relationship { b := a; b := a + 1i32; } }";
        let err = parser.parse_str(source).unwrap_err();
        let out =
            err.format_rustc_style(source, "t.adm2", 1, &annotate_snippets::Renderer::plain());
        // Primary caret on the later binding, secondary carets on the earlier binding and the
        // shared output cell's own declaration.
        assert!(out.contains("outputs"), "{out}"); // message text
        assert!(out.contains("collides with this earlier method"), "{out}");
        assert!(
            out.contains("output cell `b` is claimed more than once"),
            "{out}"
        );
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
        let cell_id = sheet.output_names["area"];
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
                        @positive area > 0;
                        @small area < 1000;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = sheet.output_names["area"];
        assert!(sheet.sheet.cell_requirements(output_id).unwrap().len() == 2);
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
                        @too_small area > 1000;
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
                    @bad a;
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
        let err = result.expect_err("expected Err");
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
        let err = result.expect_err("expected Err");
        let msg = err.message().to_lowercase();
        assert!(msg.contains("arity"), "{msg}");
    }

    #[test]
    fn parse_method_output_type_mismatch_names_the_registered_type() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: String = "x";
                cell x: i32;
                relationship { x := a; }
            }
        "#,
        );
        let err = result.expect_err("a String body for an i32 output must be an error");
        let msg = err.message();
        assert!(msg.contains("got `String`"), "{msg}");
        assert!(!msg.contains("::"), "no Rust type path may leak: {msg}");
    }

    #[test]
    fn parse_method_destructured_output_type_mismatch_names_the_registered_type() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: String = "x";
                cell b: i32 = 1;
                cell x: i32;
                cell y: i32;
                relationship { (x, y) := (a, b); }
            }
        "#,
        );
        let err = result.expect_err("a String tuple element for an i32 output must be an error");
        let msg = err.message();
        assert!(msg.contains("got `String`"), "{msg}");
        assert!(!msg.contains("::"), "no Rust type path may leak: {msg}");
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
        let err = result.expect_err("expected Err");
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
        let value = sheet
            .read_compound::<cel_runtime::Cons<
                cel_runtime::Leaf<i32>,
                cel_runtime::Cons<cel_runtime::Leaf<i32>, cel_runtime::Nil>,
            >>(cell_id)
            .unwrap();
        let (a, b) = (value.1, value.0.1);
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
        let pair = sheet
            .read_compound::<cel_runtime::Cons<
                cel_runtime::Leaf<i32>,
                cel_runtime::Cons<cel_runtime::Leaf<i32>, cel_runtime::Nil>,
            >>(pair_id)
            .unwrap();
        let (a, b) = (pair.1, pair.0.1);
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
        sheet.read_compound::<cel_runtime::Nil>(cell_id).unwrap();
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
        let err = result.expect_err("expected Err");
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
    fn conditional_branch_negative_literal_key_matches() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell mode: i32 = -1;
                    cell x: i32 = 1;
                    cell y: i32 = 0;
                    conditional mode {
                        -1i32 => { relationship { y := x; } },
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
    fn conditional_branch_negating_unsigned_literal_key_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell mode: u32 = 0;
                cell x: i32 = 1;
                cell y: i32 = 0;
                conditional mode {
                    -1u32 => { relationship { y := x; } },
                }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "negating an unsigned literal must be a parse error"
        );
    }

    #[test]
    fn conditional_with_tuple_typed_match_cell_and_default_only_still_works() {
        // A tuple isn't a `literal_pattern`, so a tuple-typed match cell can no longer pair with
        // a named branch key — only the `_` default branch remains reachable.
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell mode: (i32, i32) = (0, 0);
                    cell x: f64 = 1.0;
                    cell y: f64;
                    conditional mode {
                        _ => { relationship { y := x * 2.0; } },
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (y_id, _) = sheet.cell_names["y"].clone();
        assert_eq!(*sheet.read::<f64>(y_id).unwrap(), 2.0);
    }

    #[test]
    fn conditional_branch_tuple_literal_key_is_error() {
        let result = parser().parse_str(
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
        );
        assert!(result.is_err(), "tuple branch key must be a parse error");
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

    /// Verifies runtime conditional headers accept grouped records and deduce field dependencies.
    #[test]
    fn adam_registered_match_header_reserves_braces() {
        let mut parser = parser();
        parser
            .op_lookup_mut()
            .register_record(cel_parser::RecordSchema {
                ty: cel_parser::ResolvedLeafType::new(
                    "Gate",
                    cel_runtime::ValueType::leaf::<bool>(),
                ),
                fields: vec![cel_parser::RecordField {
                    name: "value",
                    ty: cel_parser::ResolvedLeafType::new(
                        "bool",
                        cel_runtime::ValueType::leaf::<bool>(),
                    )
                    .into(),
                }],
                emit: Arc::new(|segment, _, _, _| segment.op1(|value: bool| value)),
            })
            .unwrap();
        let mut sheet = parser
            .parse_str(
                "sheet s {
                cell flag: bool = true;
                cell input: i32 = 7;
                cell output: i32 = 0;
                conditional (Gate { value: flag }) {
                    true => { relationship { output := if flag { input } else { 0 }; } }
                    _ => { relationship { output := 0; } }
                }
            }",
            )
            .unwrap();
        let output = sheet.cell_names["output"].0;
        let flag = sheet.cell_names["flag"].0;
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(output).unwrap(), 7);
        sheet.write(flag, false).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(output).unwrap(), 0);
    }

    #[test]
    fn conditional_branch_bare_identifier_key_is_error() {
        // A bare identifier isn't a `literal_pattern` either, even when it resolves (as a
        // client-registered constant would) to a value of the match subject's own type.
        let result = parser().parse_str(
            r#"
            sheet s {
                cell mode: i32 = 0;
                cell x: i32 = 1;
                cell y: i32 = 0;
                conditional mode {
                    mode => { relationship { y := x; } },
                }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "bare identifier branch key must be a parse error"
        );
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
        let cell_id = *sheet.output_names.get("area").expect("area registered");
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
        let cell_id = *sheet.output_names.get("pair").unwrap();
        let value = sheet.read::<CellValue>(cell_id).unwrap();
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
                        @max_area width * height <= max_area;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = *sheet.output_names.get("area").unwrap();
        assert!(sheet.cell_requirements_valid(output_id));
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
                        @max_area width * height <= max_area;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = *sheet.output_names.get("area").unwrap();
        assert!(!sheet.cell_requirements_valid(output_id));
        assert_eq!(sheet.violated_requirements(output_id).count(), 1);
    }

    #[test]
    /// Resolves explicit labels by returned IDs while preserving unnamed checks and order.
    fn requirement_names_are_metadata_for_explicit_labels_only() {
        let mut parsed = parser()
            .parse_str(
                "sheet s {
                    cell x: i32 = 1 require { @same x > 0i32; x > 0i32; x < 9i32; };
                    source y: i32 = 2 require { @same y > 0i32; y > 0i32; };
                    out z := x require { @last z < 0i32; z < 0i32; @first z > 9i32; };
                }",
            )
            .unwrap();
        let x = parsed.cell_names["x"].0;
        let y = parsed.cell_names["y"].0;
        let z = parsed.output_names["z"];
        assert_eq!(parsed.requirement_names.len(), 4);
        for cell in [x, y] {
            let ids = parsed.cell_requirements(cell).unwrap();
            assert_eq!(parsed.requirement_names[&ids[0]], "same");
            assert!(!parsed.requirement_names.contains_key(&ids[1]));
        }
        parsed.propagate().unwrap();
        let violated: Vec<_> = parsed.violated_requirements(z).collect();
        assert_eq!(violated, parsed.cell_requirements(z).unwrap());
        assert_eq!(parsed.requirement_names[&violated[0]], "last");
        assert!(!parsed.requirement_names.contains_key(&violated[1]));
        assert_eq!(parsed.requirement_names[&violated[2]], "first");
    }

    #[test]
    /// Rejects repeated explicit labels locally for every declaration kind.
    fn duplicate_requirement_labels_are_rejected_per_declaration() {
        for declaration in [
            "cell x: i32 = 1",
            "source x: i32 = 1",
            "cell y: i32 = 1; out x := y",
        ] {
            let source =
                format!("sheet s {{ {declaration} require {{ @dup x > 0i32; @dup x < 9i32; }}; }}");
            assert!(parser().parse_str(&source).is_err(), "{source}");
        }
    }

    #[test]
    fn parse_out_duplicate_requirement_names_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell width: f64 = 4.0;
                out area: f64 := width require {
                    @dup width <= 10.0;
                    @dup width >= 0.0;
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
    fn parse_out_with_an_unlabeled_requirement_is_enforced() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell width: f64 = 40.0;
                    cell height: f64 = 30.0;
                    cell max_area: f64 = 100.0;
                    out area: f64 := width * height require {
                        width * height <= max_area;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = *sheet.output_names.get("area").unwrap();
        assert_eq!(sheet.violated_requirements(output_id).count(), 1);
    }

    #[test]
    fn parse_out_with_a_bare_identifier_unlabeled_requirement_is_enforced() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell is_valid: bool = false;
                    out area: i32 := 4 require {
                        is_valid;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = *sheet.output_names.get("area").unwrap();
        assert_eq!(sheet.violated_requirements(output_id).count(), 1);
    }

    #[test]
    fn parse_out_cell_referenced_elsewhere_succeeds() {
        // `out` cells are no longer terminal (see `adam_rs::CellKind`): referencing an `out`
        // cell as another relationship's input is legal.
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
            result.is_ok(),
            "referencing an out cell as another relationship's input must not be an error"
        );
    }

    #[test]
    fn parse_out_infers_structural_array_without_registration() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell dummy: i32 = 0;
                out x := []: [i32];
            }
        "#,
        );
        let mut sheet = result.unwrap();
        sheet.propagate().unwrap();
        assert!(
            sheet
                .read_compound::<cel_runtime::ArrayOf<cel_runtime::Leaf<i32>>>(
                    sheet.cell_names["x"].0
                )
                .unwrap()
                .is_empty()
        );
    }

    /// Empty tuple writers execute their initializer and propagate runtime errors.
    #[test]
    fn empty_tuple_output_executes_its_writer() {
        let mut lookup = OpLookup::new();
        lookup.push_scope(|name, segment, arity, _| {
            if name == "bad" && arity == 0 {
                segment.op0r(|| -> anyhow::Result<i32> { anyhow::bail!("unit writer failed") });
                return Ok(true);
            }
            Ok(false)
        });
        let mut sheet = AdamParser::new(TypeRegistry::new(), lookup)
            .parse_str("sheet s { cell input: i32 = 0; cell nothing: (); relationship { nothing := (input, bad, ()).2; } }")
            .unwrap();
        let error = sheet.propagate().unwrap_err();
        assert!(error.to_string().contains("unit writer failed"), "{error}");
    }

    #[test]
    fn parse_out_cell_referenced_in_conditional_succeeds() {
        // `out` cells are no longer terminal (see `adam_rs::CellKind`): referencing an `out`
        // cell as a relationship input inside a conditional branch is legal.
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
            result.is_ok(),
            "referencing an out cell as a conditional branch relationship's input must not be an error"
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
            .read_compound::<cel_runtime::Cons<
                cel_runtime::Leaf<i32>,
                cel_runtime::Cons<cel_runtime::Leaf<f64>, cel_runtime::Nil>,
            >>(cell_id)
            .unwrap();
        let (a, b) = (value.1, value.0.1);
        assert_eq!((a, b), (1, 2.5));
    }

    #[test]
    fn parse_cell_with_tuple_type_and_no_initializer_uses_recursive_default() {
        let parsed = parser()
            .parse_str("sheet s { cell a: (i32, f64); }")
            .unwrap();
        let (cell_id, _) = parsed.cell_names["a"].clone();
        let value = parsed
            .read_compound::<cel_runtime::Cons<
                cel_runtime::Leaf<i32>,
                cel_runtime::Cons<cel_runtime::Leaf<f64>, cel_runtime::Nil>,
            >>(cell_id)
            .unwrap();
        let (a, b) = (value.1, value.0.1);
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
        type Inner = cel_runtime::Cons<
            cel_runtime::Leaf<f64>,
            cel_runtime::Cons<cel_runtime::Leaf<String>, cel_runtime::Nil>,
        >;
        type Outer =
            cel_runtime::Cons<cel_runtime::Leaf<i32>, cel_runtime::Cons<Inner, cel_runtime::Nil>>;
        let value = parsed.read_compound::<Outer>(cell_id).unwrap();
        assert_eq!(value.1, 1);
        let (b, c) = (value.0.1.1, value.0.1.0.1);
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
        let cell_id = *sheet.output_names.get("pair").unwrap();
        let value = sheet
            .read_compound::<cel_runtime::Cons<
                cel_runtime::Leaf<i32>,
                cel_runtime::Cons<cel_runtime::Leaf<i32>, cel_runtime::Nil>,
            >>(cell_id)
            .unwrap();
        let (a, b) = (value.1, value.0.1);
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
        let err = result.expect_err("expected Err");
        assert!(err.message().contains("cannot infer a type"));
    }

    #[test]
    fn dependency_cycle_validation_error_spans_the_governed_cell() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        // The branch writes the match cell `mode`, so whole-sheet validation returns
        // DependencyCycle on line 2 (`cell mode`), the governed cell that starts the cycle.
        let source = "sheet s {\n    cell mode: i32 = 0;\n    cell other: i32 = 0;\n\n    conditional mode {\n        0i32 => {\n            relationship {\n                mode := other;\n                other := mode;\n            }\n        }\n    }\n}";
        let err = parser.parse_str(source).unwrap_err();
        assert_eq!(err.span().start().line, 2);
    }

    // NOTE on coverage: `add_conditional`'s other `InvalidConditional` case with a
    // `Relationship`-only site list -- the same relationship reused across two branches (or a
    // branch and the default) -- is not reachable through adam-lang's own grammar today.
    // `parse_branch_relationships` calls `parse_relationship_decl`, which always creates a
    // *fresh* `RelationshipId` on `add_relationship`; adam-lang has no syntax for a `relationship`
    // to be *named* and then referenced again from a second branch. That case is exercised
    // directly against `Sheet::add_conditional` in
    // `add_conditional_returns_invalid_conditional_for_duplicate_relationship_across_branches`
    // (adam-rs/src/sheet.rs); it stays untested at the adam-lang parser level until adam-lang
    // grows named/referenceable relationships (tracked as a follow-up, not blocking this task).
    #[test]
    fn conditional_invalid_relationship_and_cell_sites_render_as_secondary_labels() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        // The governed branch output `a` reaches match cell `p` through the unconditional
        // relationship, so the diagnostic should use `a` as primary and attach the
        // remaining cycle sites as secondary labels.
        let source = "sheet s {\n    cell a: i32 = 0;\n    cell b: i32 = 0;\n    cell p: i32 = 0;\n    relationship {\n        p := a;\n    }\n\n    conditional p {\n        0i32 => {\n            relationship {\n                a := b;\n            }\n        }\n    }\n}";
        let err = parser.parse_str(source).unwrap_err();
        let out =
            err.format_rustc_style(source, "t.adm2", 1, &annotate_snippets::Renderer::plain());
        assert!(out.contains("cell `p` is part of the cycle"), "{out}");
    }

    #[test]
    /// Locates duplicate labels at the repeated requirement, not the sheet opening.
    fn out_decl_duplicate_requirement_error_spans_the_duplicate_label() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        // The parser owns display-label validation, so it can identify the repeated label
        // itself rather than blaming runtime registration or the sheet's opening line.
        let source = "sheet s {\n    cell w: i32 = 1;\n    out a: i32 := w require {\n        @pos a > 0;\n        @pos a > 0;\n    };\n}";
        let err = parser.parse_str(source).unwrap_err();
        assert_eq!(err.span().start().line, 5);
        assert!(err.to_string().contains("duplicate requirement `pos`"));
    }
}
