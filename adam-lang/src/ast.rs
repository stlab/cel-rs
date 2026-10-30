//! A span-carrying adam-lang structural AST, built by [`crate::AdamAstParser`] as an alternative to
//! [`crate::AdamParser`]'s direct `adam_rs::Sheet` construction. Method bodies and cell
//! initializers reference [`cel_parser::Expr`]/[`cel_parser::lex_lexer::Literal`] directly.
//! Carries no resolved types, no `TypeRegistry` lookups, and never fails on semantic grounds
//! (unknown type name, literal/type mismatch, undeclared cell, arity mismatch) — those checks
//! are deferred to a later, separate compile-to-`Sheet` phase, mirroring
//! [`cel_parser::AstContext`]'s design.

pub use cel_parser::ExprSpan;
use cel_parser::lex_lexer::Literal;

pub use cel_parser::Comment;

/// An ordered file of imports and sheet definitions, without resolved names or types.
///
/// Errors include malformed file items and errors recovered inside definitions.
///
/// # Examples
///
/// ```
/// let module = adam_lang::AdamAstParser::new().parse_module_str(
///     "use math::ops as operations;
///      sheet component(input: i32, result: i32) {
///          relationship { result := input; }
///      }
///      sheet root {
///          source x: i32 = 1 filter 0..=10 require { @positive x > 0; };
///          cell y: i32;
///          cell z: i32;
///          out pair: (i32, i32) := (x, y);
///          relationship component(input: x, result: y);
///          conditional true {
///              true => { relationship { (y, z) := (x, x); } }
///              _ => { relationship { y := x; } }
///          }
///      }"
/// ).unwrap();
/// assert!(module.errors.is_empty());
/// assert_eq!(module.items.len(), 3);
/// ```
#[derive(Debug, Clone)]
pub struct Module {
    /// File items in source order, including recovery placeholders.
    pub items: Vec<ModuleItem>,
    /// Recovered syntax errors in source order.
    pub errors: Vec<cel_parser::ParseError>,
    /// Comments before each item and after the last item, populated by trivia attachment.
    pub comments: Vec<Vec<Comment>>,
}

/// A file-level declaration or a recovered malformed declaration.
///
/// # Examples
///
/// See [`Module`] for a parsed file containing imports and sheet definitions.
#[derive(Debug, Clone)]
pub enum ModuleItem {
    /// An explicit named import.
    Use(UseDecl),
    /// A root or parameterized definition.
    Sheet(Sheet),
    /// The span of a malformed file-level declaration.
    Error(ExprSpan),
}

impl ModuleItem {
    /// Returns the declaration or recovery span.
    ///
    /// # Examples
    ///
    /// ```
    /// let module = adam_lang::AdamAstParser::new().parse_module_str("sheet root {}").unwrap();
    /// assert_eq!(module.items[0].span().start.start().line, 1);
    /// ```
    pub fn span(&self) -> ExprSpan {
        match self {
            Self::Use(item) => item.span,
            Self::Sheet(item) => item.span,
            Self::Error(span) => *span,
        }
    }
}

/// `use_decl = "use" identifier { "::" identifier } [ "as" identifier ] ";".`
///
/// # Examples
///
/// See [`Module`] for an aliased import in a parsed file.
#[derive(Debug, Clone)]
pub struct UseDecl {
    /// The `use` keyword's span, excluding preceding documentation.
    pub keyword_span: ExprSpan,
    /// Leading inner documentation, if present.
    pub doc_comment: Option<String>,
    /// Unresolved path segments and their individual spans.
    pub path: Vec<(String, ExprSpan)>,
    /// The optional local alias and its span.
    pub alias: Option<(String, ExprSpan)>,
    /// The whole import's span.
    pub span: ExprSpan,
}

/// A required typed formal parameter, without a default or an initializer.
///
/// # Examples
///
/// See [`Module`] for the component's `input` and `result` parameters.
#[derive(Debug, Clone)]
pub struct ParameterDecl {
    /// The formal parameter's name.
    pub name: String,
    /// The name token's span.
    pub name_span: ExprSpan,
    /// The required unresolved type expression.
    pub type_name: TypeExpr,
}

/// A named binding from a formal parameter to a caller's cell identifier.
///
/// # Examples
///
/// See [`Module`] for `input: x` and `result: y` actual arguments.
#[derive(Debug, Clone)]
pub struct NamedArgument {
    /// The formal parameter's name.
    pub name: String,
    /// The formal name token's span.
    pub name_span: ExprSpan,
    /// The actual cell's name.
    pub cell: String,
    /// The actual cell token's span.
    pub cell_span: ExprSpan,
}

/// `relationship_use = "relationship" path "(" [ named_arguments ] ")" ";".`
///
/// # Examples
///
/// See [`Module`] for a component use with named actual arguments.
#[derive(Debug, Clone)]
pub struct RelationshipUse {
    /// The `relationship` keyword's span, excluding any preceding documentation.
    pub keyword_span: ExprSpan,
    /// Unresolved callee path segments and their spans.
    pub path: Vec<(String, ExprSpan)>,
    /// Named cell arguments in source order.
    pub arguments: Vec<NamedArgument>,
    /// Every comment in the opening-brace gap when this use is the first body item.
    /// Empty for a later item; its ordinary sibling trivia uses `leading_comment` instead.
    pub opening_comments: Vec<Comment>,
    /// A recovered leading comment.
    pub leading_comment: Option<Comment>,
    /// A leading documentation comment.
    pub doc_comment: Option<String>,
    /// A comment on the declaration's last source line.
    pub trailing_line_comment: Option<Comment>,
    /// Whether a blank line precedes this declaration.
    pub blank_line_before: bool,
    /// The whole declaration's span.
    pub span: ExprSpan,
}

/// A conditional branch item, retaining inline and included relationship order.
///
/// # Examples
///
/// See [`Module`] for inline relationships in named and default branches.
#[derive(Debug, Clone)]
pub enum BranchItem {
    /// An ordinary inline relationship.
    Relationship(RelationshipDecl),
    /// A named component use.
    RelationshipUse(RelationshipUse),
}

/// A parsed adam-lang sheet declaration, with source spans on every node.
///
/// Built by [`crate::AdamAstParser`]; consumed by the language server, the formatter,
/// and [`crate::ModuleCompiler`].
///
/// # Examples
///
/// See [`Module`] for a root sheet and a parameterized definition.
#[derive(Debug, Clone)]
pub struct Sheet {
    /// The `sheet` keyword's span, excluding any preceding documentation.
    pub keyword_span: ExprSpan,
    /// The sheet's declared name.
    pub name: String,
    /// The name token's span.
    pub name_span: ExprSpan,
    /// `None` for a root; `Some`, including an empty vector, for a component.
    pub parameters: Option<Vec<ParameterDecl>>,
    /// The sheet's items, in declaration order.
    pub items: Vec<SheetItem>,
    /// A leading `//`/`/* */` comment immediately preceding the `sheet` keyword (e.g. a file
    /// header), if recovered by [`crate::trivia::attach_trivia`]. Unlike every other node's
    /// `leading_comment`, this one has no enclosing sibling list to attach via — it covers the
    /// gap between the start of the source and the sheet's own span.
    pub leading_comment: Option<Comment>,
    /// A leading `//!` doc comment immediately preceding the `sheet` keyword, if recovered by
    /// [`crate::AdamAstParser`].
    pub doc_comment: Option<String>,
    /// A trailing comment immediately preceding the sheet's own closing `}`, if recovered. See
    /// <https://github.com/stlab/cel-rs/issues/52>.
    pub trailing_comment: Option<Comment>,
    /// Whether a blank line preceded the sheet's own closing `}`, if recovered. See
    /// <https://github.com/stlab/cel-rs/issues/52>.
    pub blank_line_before_close: bool,
    /// The span of the sheet's own opening `{`, used to recover trailing trivia when `items` is
    /// empty. See <https://github.com/stlab/cel-rs/issues/52>.
    pub open_brace_span: ExprSpan,
    /// The span of the whole `sheet ... { ... }` construct.
    pub span: ExprSpan,
    /// Syntax errors recovered while parsing, in source order. Empty for a syntactically clean
    /// sheet.
    pub errors: Vec<cel_parser::ParseError>,
}

/// One top-level item inside a `sheet { ... }` body.
///
/// # Examples
///
/// See [`Module`] for source, cell, output, relationship-use and conditional items.
#[derive(Debug, Clone)]
pub enum SheetItem {
    /// A `cell` declaration.
    Cell(CellDecl),
    /// A `relationship` declaration.
    Relationship(RelationshipDecl),
    /// A named component use.
    RelationshipUse(RelationshipUse),
    /// A `conditional` declaration.
    Conditional(ConditionalDecl),
    /// An `out` declaration.
    Out(OutDecl),
    /// A `source` declaration.
    Source(SourceDecl),
    /// A syntax error recovered at declaration granularity; `span` covers the skipped tokens.
    Error {
        /// The span of the skipped, malformed item.
        span: ExprSpan,
        /// A leading `//`/`/* */` comment immediately preceding this item, if recovered by
        /// [`crate::trivia::attach_trivia`]. Preserved even though the item failed to parse, so
        /// a comment explaining a broken declaration (e.g. `// TODO: fix this`) isn't silently
        /// dropped.
        leading_comment: Option<Comment>,
        /// A leading `///` doc comment immediately preceding this item, if recovered by
        /// [`crate::AdamAstParser`] before parsing failed.
        doc_comment: Option<String>,
        /// A `//`/`/* */` comment on the same source line as this item's own last token, if
        /// recovered by [`crate::trivia::attach_trivia`]. Distinct from `leading_comment`, which
        /// precedes an item on its own line. See <https://github.com/stlab/cel-rs/issues/59>.
        trailing_line_comment: Option<Comment>,
        /// Whether the gap before this item contained a blank line, if recovered by
        /// [`crate::trivia::attach_trivia`].
        blank_line_before: bool,
    },
}

impl SheetItem {
    /// Returns this item's source span.
    ///
    /// # Examples
    ///
    /// ```
    /// let sheet = adam_lang::AdamAstParser::new()
    ///     .parse_str("sheet s {\n    source x: i32;\n}").unwrap();
    /// assert_eq!(sheet.items[0].span().start.start().line, 2);
    /// assert_eq!(sheet.items[0].span().start.start().column, 4);
    /// ```
    pub fn span(&self) -> ExprSpan {
        match self {
            SheetItem::Cell(c) => c.span,
            SheetItem::Relationship(r) => r.span,
            SheetItem::RelationshipUse(r) => r.span,
            SheetItem::Conditional(c) => c.span,
            SheetItem::Out(o) => o.span,
            SheetItem::Source(s) => s.span,
            SheetItem::Error { span, .. } => *span,
        }
    }

    /// Sets this item's leading comment.
    pub(crate) fn set_leading_comment(&mut self, comment: Comment) {
        match self {
            SheetItem::Cell(c) => c.leading_comment = Some(comment),
            SheetItem::Relationship(r) => r.leading_comment = Some(comment),
            SheetItem::RelationshipUse(r) => r.leading_comment = Some(comment),
            SheetItem::Conditional(c) => c.leading_comment = Some(comment),
            SheetItem::Out(o) => o.leading_comment = Some(comment),
            SheetItem::Source(s) => s.leading_comment = Some(comment),
            SheetItem::Error {
                leading_comment, ..
            } => *leading_comment = Some(comment),
        }
    }

    /// Sets a `//`/`/* */` comment recovered on the same source line as this item's own last
    /// token. See <https://github.com/stlab/cel-rs/issues/59>.
    pub(crate) fn set_trailing_line_comment(&mut self, comment: Comment) {
        match self {
            SheetItem::Cell(c) => c.trailing_line_comment = Some(comment),
            SheetItem::Relationship(r) => r.trailing_line_comment = Some(comment),
            SheetItem::RelationshipUse(r) => r.trailing_line_comment = Some(comment),
            SheetItem::Conditional(c) => c.trailing_line_comment = Some(comment),
            SheetItem::Out(o) => o.trailing_line_comment = Some(comment),
            SheetItem::Source(s) => s.trailing_line_comment = Some(comment),
            SheetItem::Error {
                trailing_line_comment,
                ..
            } => *trailing_line_comment = Some(comment),
        }
    }

    /// Sets whether a blank line preceded this item.
    pub(crate) fn set_blank_line_before(&mut self, value: bool) {
        match self {
            SheetItem::Cell(c) => c.blank_line_before = value,
            SheetItem::Relationship(r) => r.blank_line_before = value,
            SheetItem::RelationshipUse(r) => r.blank_line_before = value,
            SheetItem::Conditional(c) => c.blank_line_before = value,
            SheetItem::Out(o) => o.blank_line_before = value,
            SheetItem::Source(s) => s.blank_line_before = value,
            SheetItem::Error {
                blank_line_before, ..
            } => *blank_line_before = value,
        }
    }

    /// Sets this item's doc comment and widens its span to start at `start` (the doc comment's
    /// own first token), so `trivia::attach_trivia`'s gap scan stops before the doc comment's
    /// source text instead of misparsing it as a plain `//` comment.
    pub(crate) fn set_doc_comment(&mut self, text: String, start: proc_macro2::Span) {
        match self {
            SheetItem::Cell(c) => {
                c.doc_comment = Some(text);
                c.span.start = start;
            }
            SheetItem::Relationship(r) => {
                r.doc_comment = Some(text);
                r.span.start = start;
            }
            SheetItem::RelationshipUse(r) => {
                r.doc_comment = Some(text);
                r.span.start = start;
            }
            SheetItem::Conditional(c) => {
                c.doc_comment = Some(text);
                c.span.start = start;
            }
            SheetItem::Out(o) => {
                o.doc_comment = Some(text);
                o.span.start = start;
            }
            SheetItem::Source(s) => {
                s.doc_comment = Some(text);
                s.span.start = start;
            }
            SheetItem::Error {
                doc_comment, span, ..
            } => {
                *doc_comment = Some(text);
                span.start = start;
            }
        }
    }
}

/// Re-exports CEL's shared named, generic, tuple, array, and callable type expressions.
///
/// `()` is the empty tuple type; `(T)` is grouping; `(T,)` is a singleton tuple.
/// Larger tuples prohibit trailing commas. Both languages use the same parser.
///
/// # Examples
///
/// See [`crate::TypeRegistry::resolve`] for parsing and resolving a nested array-and-tuple type.
pub use cel_parser::TypeExpr;

/// `cell_decl = "cell" identifier cell_type_init [ cell_filter ] [ "require" "{" { requirement }
/// "}" ] ";".`
///
/// `type_name`/`initializer` are unresolved — no `TypeRegistry` lookup, no literal validation.
/// Parsed declarations contain an annotation, an initializer, or both. Public field storage
/// does not enforce this invariant for manually constructed nodes.
///
/// # Examples
///
/// See [`Module`] for annotated cells in a parsed sheet.
#[derive(Debug, Clone)]
pub struct CellDecl {
    /// The cell's declared name.
    pub name: String,
    /// The name token's span.
    pub name_span: ExprSpan,
    /// The `: type_expr` annotation, if present.
    pub type_name: Option<TypeExpr>,
    /// The `= expression` initializer, if present. Unresolved and unevaluated here — see
    /// `crate::parser::AdamParser` for the compile-to-`Sheet` phase, which parses this with no
    /// cell scope pushed and evaluates it eagerly, once, at parse time.
    pub initializer: Option<cel_parser::Expr>,
    /// The `filter` clause, if present.
    pub filter: Option<CellFilter>,
    /// The `require { ... }` validation block, if present.
    pub require: Option<RequireBlock>,
    /// A leading `//`/`/* */` comment immediately preceding this declaration, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<Comment>,
    /// A leading `///` doc comment immediately preceding this declaration, if recovered by
    /// [`crate::AdamAstParser`].
    pub doc_comment: Option<String>,
    /// A `//`/`/* */` comment on the same source line as this declaration's own last token, if
    /// recovered by [`crate::trivia::attach_trivia`]. See
    /// <https://github.com/stlab/cel-rs/issues/59>.
    pub trailing_line_comment: Option<Comment>,
    /// Whether a blank line preceded this declaration, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `cell ...;` declaration.
    pub span: ExprSpan,
}

/// `source_decl = "source" identifier cell_type_init [ cell_filter ] [ "require" "{" {
/// requirement } "}" ] ";".`
///
/// Same shape as [`CellDecl`]: a `source` cell's initializer is a one-time expression exactly like a
/// plain `cell`'s, and it supports the same `filter` clause and `require` block.
///
/// # Examples
///
/// See [`Module`] for a source with an initializer, range filter and requirement.
#[derive(Debug, Clone)]
pub struct SourceDecl {
    /// The source cell's declared name.
    pub name: String,
    /// The name token's span.
    pub name_span: ExprSpan,
    /// The `: type_expr` annotation, if present.
    pub type_name: Option<TypeExpr>,
    /// The `= expression` initializer, if present. Unresolved and unevaluated here — see
    /// `crate::parser::AdamParser` for the compile-to-`Sheet` phase, which parses this with no
    /// cell scope pushed and evaluates it eagerly, once, at parse time.
    pub initializer: Option<cel_parser::Expr>,
    /// The `filter` clause, if present.
    pub filter: Option<CellFilter>,
    /// The `require { ... }` validation block, if present.
    pub require: Option<RequireBlock>,
    /// A leading `//`/`/* */` comment immediately preceding this declaration, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<Comment>,
    /// A leading `///` doc comment immediately preceding this declaration, if recovered by
    /// [`crate::AdamAstParser`].
    pub doc_comment: Option<String>,
    /// A `//`/`/* */` comment on the same source line as this declaration's own last token, if
    /// recovered by [`crate::trivia::attach_trivia`]. See
    /// <https://github.com/stlab/cel-rs/issues/59>.
    pub trailing_line_comment: Option<Comment>,
    /// Whether a blank line preceded this declaration, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `source ...;` declaration.
    pub span: ExprSpan,
}

/// `cell_filter = "filter" expression.`
///
/// # Examples
///
/// See [`Module`] for an inclusive range filter.
#[derive(Debug, Clone)]
pub struct CellFilter {
    /// The filter's body expression. `_` inside it denotes the candidate value being conformed;
    /// every other identifier that names an already-declared cell is a deduced dependency.
    pub body: cel_parser::Expr,
    /// The span of the whole `filter ...` clause.
    pub span: ExprSpan,
}

/// `relationship_decl = "relationship" "{" { binding } "}".`
///
/// # Examples
///
/// See [`Module`] for direct and destructuring bindings in relationships.
#[derive(Debug, Clone)]
pub struct RelationshipDecl {
    /// The relationship block's bindings, in declaration order.
    pub bindings: Vec<BindingDecl>,
    /// A leading comment immediately preceding this declaration, if recovered.
    pub leading_comment: Option<Comment>,
    /// A leading `///` doc comment immediately preceding this declaration, if recovered by
    /// [`crate::AdamAstParser`].
    pub doc_comment: Option<String>,
    /// A `//`/`/* */` comment on the same source line as this declaration's own closing `}`, if
    /// recovered. Distinct from `trailing_comment`, which precedes this declaration's own
    /// closing `}` on an earlier line. See <https://github.com/stlab/cel-rs/issues/59>.
    pub trailing_line_comment: Option<Comment>,
    /// Whether a blank line preceded this declaration, if recovered.
    pub blank_line_before: bool,
    /// A trailing comment immediately preceding this declaration's own closing `}`, if
    /// recovered. See <https://github.com/stlab/cel-rs/issues/52>.
    pub trailing_comment: Option<Comment>,
    /// Whether a blank line preceded this declaration's own closing `}`, if recovered. See
    /// <https://github.com/stlab/cel-rs/issues/52>.
    pub blank_line_before_close: bool,
    /// The span of this declaration's own opening `{`, used to recover trailing trivia when
    /// `bindings` is empty. See <https://github.com/stlab/cel-rs/issues/52>.
    pub open_brace_span: ExprSpan,
    /// The span of the whole `relationship { ... }` declaration.
    pub span: ExprSpan,
}

/// `binding = binding_target ":=" expression ";".`
/// `binding_target = identifier | "(" identifier "," [ identifier { "," identifier } ] ")".`
///
/// The compiler deduces inputs from resolved cell references in `body`. The structural
/// AST records only the explicit output names, without resolving cell declarations.
///
/// Parenthesizing the left-hand side requests tuple destructuring, matching Rust's tuple-pattern
/// syntax: `(a, b) := ...` and the single-element `(a,) := ...` (trailing comma mandatory,
/// exactly as in a Rust 1-tuple pattern) both destructure `body`'s tuple result element-wise into
/// `outputs`. A bare identifier binds `body`'s whole result directly to that one output instead.
/// A parenthesized binding target always requires a comma; `(a) := ...` is invalid.
/// See [`Self::destructure`].
///
/// # Examples
///
/// See [`Module`] for both direct and destructuring binding targets.
#[derive(Debug, Clone)]
pub struct BindingDecl {
    /// The binding's output cell names (the left-hand side), in declaration order.
    pub outputs: Vec<(String, ExprSpan)>,
    /// Whether the left-hand side requests destructuring (`(a, b) := ...` or `(a,) := ...`) as
    /// opposed to a direct bind (`a := ...`). Always
    /// `true` when `outputs.len() > 1`; when `outputs.len() == 1`, distinguishes "destructure
    /// this single-element tuple" from "bind this whole value directly" — a distinction the
    /// parenthesized-or-not left-hand side is the only way to express, since both forms name
    /// exactly one output.
    pub destructure: bool,
    /// The parsed right-hand-side expression.
    pub body: cel_parser::Expr,
    /// A leading comment immediately preceding this binding, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<Comment>,
    /// A `//`/`/* */` comment on the same source line as this binding's own last token, if
    /// recovered by [`crate::trivia::attach_trivia`]. See
    /// <https://github.com/stlab/cel-rs/issues/59>.
    pub trailing_line_comment: Option<Comment>,
    /// Whether a blank line preceded this binding, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `a := ...;` / `(a, b) := ...;` declaration.
    pub span: ExprSpan,
}

/// `out_decl = "out" identifier [ ":" type_expr ] ":=" expression [ cell_filter ] [ "require"
/// "{" { requirement } "}" ] ";".`
///
/// `type_expr` is unresolved here (no `TypeRegistry` lookup), matching `CellDecl`. When
/// absent, the cell's type is inferred from `initializer`'s result type by the compile phase
/// (`crate::parser::AdamParser`) — never here.
///
/// # Examples
///
/// See [`Module`] for a tuple-valued annotated output.
#[derive(Debug, Clone)]
pub struct OutDecl {
    /// The declared cell's name.
    pub name: String,
    /// The name token's span.
    pub name_span: ExprSpan,
    /// The `: type_expr` annotation, if present.
    pub type_name: Option<TypeExpr>,
    /// The parsed initializer expression that computes this cell's value.
    pub initializer: cel_parser::Expr,
    /// The `filter` clause, if present.
    pub filter: Option<CellFilter>,
    /// The `require { ... }` validation block, if present.
    pub require: Option<RequireBlock>,
    /// A leading `//`/`/* */` comment immediately preceding this declaration, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<Comment>,
    /// A leading `///` doc comment immediately preceding this declaration, if recovered by
    /// [`crate::AdamAstParser`].
    pub doc_comment: Option<String>,
    /// A `//`/`/* */` comment on the same source line as this declaration's own last token, if
    /// recovered by [`crate::trivia::attach_trivia`]. See
    /// <https://github.com/stlab/cel-rs/issues/59>.
    pub trailing_line_comment: Option<Comment>,
    /// Whether a blank line preceded this declaration, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `out ... ;` declaration.
    pub span: ExprSpan,
}

/// An optional `require { ... }` block on a cell, source or output declaration.
///
/// # Examples
///
/// See [`Module`] for a labeled requirement on a source.
#[derive(Debug, Clone)]
pub struct RequireBlock {
    /// The block's requirements, in declaration order.
    pub requirements: Vec<RequirementDecl>,
    /// A trailing comment immediately preceding this block's own closing `}`, if recovered.
    /// See <https://github.com/stlab/cel-rs/issues/52>.
    pub trailing_comment: Option<Comment>,
    /// Whether a blank line preceded this block's own closing `}`, if recovered. See
    /// <https://github.com/stlab/cel-rs/issues/52>.
    pub blank_line_before_close: bool,
    /// The span of this block's own opening `{`, used to recover trailing trivia when
    /// `requirements` is empty.
    pub open_brace_span: ExprSpan,
    /// The span of the whole `require { ... }` block.
    pub span: ExprSpan,
}

/// `requirement = [ "@" identifier ] expression ";".`
///
/// `name`, when present, is a plain string label passed to `adam_rs::Sheet::add_requirement`,
/// not a cell reference — it may coincide with a cell name declared elsewhere in the sheet but
/// doesn't have to.
///
/// # Examples
///
/// See [`Module`] for an explicitly labeled Boolean requirement.
#[derive(Debug, Clone)]
pub struct RequirementDecl {
    /// The requirement's declared label, if the `@identifier` marker was present.
    pub name: Option<String>,
    /// The `@identifier` marker's span, if present.
    pub name_span: Option<ExprSpan>,
    /// The parsed requirement body expression; must type-check as `bool`.
    pub body: cel_parser::Expr,
    /// A leading comment immediately preceding this requirement, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<Comment>,
    /// A `//`/`/* */` comment on the same source line as this requirement's own last token, if
    /// recovered by [`crate::trivia::attach_trivia`]. See
    /// <https://github.com/stlab/cel-rs/issues/59>.
    pub trailing_line_comment: Option<Comment>,
    /// Whether a blank line preceded this requirement, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `[ "@" identifier ] expr;` declaration.
    pub span: ExprSpan,
}

/// `conditional_decl = "conditional" expression "{" { conditional_branch } "}".`
///
/// # Examples
///
/// See [`Module`] for a conditional with named and default branches.
#[derive(Debug, Clone)]
pub struct ConditionalDecl {
    /// The match subject: an arbitrary expression over already-declared cells (a bare
    /// identifier, e.g. `mode`, is the degenerate single-cell case).
    pub match_expr: cel_parser::Expr,
    /// The named (literal `=>`) branches, in declaration order.
    pub branches: Vec<ConditionalBranch>,
    /// The `_ => { ... }` default branch, if present.
    pub default: Option<DefaultBranch>,
    /// A leading comment immediately preceding this declaration, if recovered.
    pub leading_comment: Option<Comment>,
    /// A leading `///` doc comment immediately preceding this declaration, if recovered by
    /// [`crate::AdamAstParser`].
    pub doc_comment: Option<String>,
    /// A `//`/`/* */` comment on the same source line as this declaration's own closing `}`, if
    /// recovered. Distinct from `trailing_comment`, which precedes this declaration's own
    /// closing `}` on an earlier line. See <https://github.com/stlab/cel-rs/issues/59>.
    pub trailing_line_comment: Option<Comment>,
    /// Whether a blank line preceded this declaration, if recovered.
    pub blank_line_before: bool,
    /// A trailing comment immediately preceding this declaration's own closing `}`, if
    /// recovered. See <https://github.com/stlab/cel-rs/issues/52>.
    pub trailing_comment: Option<Comment>,
    /// Whether a blank line preceded this declaration's own closing `}`, if recovered. See
    /// <https://github.com/stlab/cel-rs/issues/52>.
    pub blank_line_before_close: bool,
    /// The span of this declaration's own opening `{`, used to recover trailing trivia when
    /// there are no `branches`/`default`. See <https://github.com/stlab/cel-rs/issues/52>.
    pub open_brace_span: ExprSpan,
    /// The span of the whole `conditional ... { ... }` declaration.
    pub span: ExprSpan,
}

/// The `_ => { ... }` default branch of a `conditional_decl`, mirroring `ConditionalBranch`'s
/// shape (it has no match literal of its own).
///
/// # Examples
///
/// See [`Module`] for a conditional with a terminal default branch.
#[derive(Debug, Clone)]
pub struct DefaultBranch {
    /// Comments before the default pattern, in source order.
    pub leading_comment: Option<Comment>,
    /// Whether a blank line precedes the default pattern.
    pub blank_line_before: bool,
    /// The default branch's relationships, in declaration order.
    pub relationships: Vec<BranchItem>,
    /// A trailing comment immediately preceding this branch's own closing `}`, if recovered.
    /// See <https://github.com/stlab/cel-rs/issues/52>.
    pub trailing_comment: Option<Comment>,
    /// A `//`/`/* */` comment on the same source line as this branch's own closing `}` (between
    /// it and the enclosing conditional's own closing `}`), if recovered. See
    /// <https://github.com/stlab/cel-rs/issues/59>.
    pub trailing_line_comment: Option<Comment>,
    /// Whether a blank line preceded this branch's own closing `}`, if recovered.
    pub blank_line_before_close: bool,
    /// The span of this branch's own opening `{`, used to recover trailing trivia when
    /// `relationships` is empty.
    pub open_brace_span: ExprSpan,
    /// The span from the `_` token through this branch's own closing `}`.
    pub span: ExprSpan,
}

/// `conditional_branch = literal_pattern "=>" "{" { relationship_item } "}".`
/// `literal_pattern = ["-"] literal.`
///
/// # Examples
///
/// See [`Module`] for a Boolean literal branch containing a destructuring binding.
#[derive(Debug, Clone)]
pub struct ConditionalBranch {
    /// The branch's unresolved match literal, always stored unsigned; see [`Self::negated`]
    /// for whether a leading `-` applies to it.
    pub literal: Literal,
    /// Whether the branch key is negated by a leading `-` (Rust's own `LiteralPattern` rule —
    /// see <https://doc.rust-lang.org/reference/patterns.html#literal-patterns>). A `-`, if
    /// present, precedes [`Self::literal_span`] and is covered by [`Self::span`]'s start, but
    /// not by `literal_span` itself.
    pub negated: bool,
    /// The literal token's own span (never includes a leading `-`; see [`Self::negated`]).
    pub literal_span: ExprSpan,
    /// The branch's relationships, in declaration order.
    pub relationships: Vec<BranchItem>,
    /// A leading comment immediately preceding this branch, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<Comment>,
    /// A `//`/`/* */` comment on the same source line as this branch's own closing `}`, if
    /// recovered. Distinct from `trailing_comment`, which precedes this branch's own closing
    /// `}` on an earlier line. See <https://github.com/stlab/cel-rs/issues/59>.
    pub trailing_line_comment: Option<Comment>,
    /// Whether a blank line preceded this branch, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// A trailing comment immediately preceding this branch's own closing `}`, if recovered.
    /// See <https://github.com/stlab/cel-rs/issues/52>.
    pub trailing_comment: Option<Comment>,
    /// Whether a blank line preceded this branch's own closing `}`, if recovered. See
    /// <https://github.com/stlab/cel-rs/issues/52>.
    pub blank_line_before_close: bool,
    /// The span of this branch's own opening `{`, used to recover trailing trivia when
    /// `relationships` is empty. See <https://github.com/stlab/cel-rs/issues/52>.
    pub open_brace_span: ExprSpan,
    /// The span from the branch's literal through its closing `}`.
    pub span: ExprSpan,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::Span;

    /// Checks nondefault source coordinates and independent trivia replacement for every variant.
    #[test]
    fn sheet_item_operations_preserve_unrelated_fields_for_every_variant() {
        let source = "sheet s {\ncell a: i32;\nsource b: i32;\nout c := 1;\nrelationship {}\nrelationship component();\nconditional true {}\nbogus;\n}";
        let mut sheet = crate::AdamAstParser::new().parse_str(source).unwrap();
        assert_eq!(sheet.errors.len(), 1);
        assert_eq!(sheet.items.len(), 7);
        let replacement = "header"
            .parse::<proc_macro2::TokenStream>()
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
            .span();
        for (index, item) in sheet.items.iter_mut().enumerate() {
            let original = item.span();
            assert_eq!(original.start.start().line, index + 2);
            assert_eq!(original.start.start().column, 0);
            item.set_leading_comment(Comment::Line("old".into()));
            item.set_leading_comment(Comment::Block("leading".into()));
            item.set_trailing_line_comment(Comment::Line("trailing".into()));
            item.set_blank_line_before(true);
            item.set_blank_line_before(false);
            item.set_doc_comment("documentation".into(), replacement);
            assert_eq!(item.span().start.start(), replacement.start());
            assert_eq!(item.span().end.end(), original.end.end());
            let (leading, trailing, blank, doc) = item_trivia(item);
            assert_eq!(leading, Some(&Comment::Block("leading".into())));
            assert_eq!(trailing, Some(&Comment::Line("trailing".into())));
            assert!(!blank);
            assert_eq!(doc, Some("documentation"));
        }
    }

    /// Borrows each variant's trivia without changing its span or contents.
    fn item_trivia(item: &SheetItem) -> (Option<&Comment>, Option<&Comment>, bool, Option<&str>) {
        match item {
            SheetItem::Cell(c) => (
                c.leading_comment.as_ref(),
                c.trailing_line_comment.as_ref(),
                c.blank_line_before,
                c.doc_comment.as_deref(),
            ),
            SheetItem::Source(c) => (
                c.leading_comment.as_ref(),
                c.trailing_line_comment.as_ref(),
                c.blank_line_before,
                c.doc_comment.as_deref(),
            ),
            SheetItem::Out(c) => (
                c.leading_comment.as_ref(),
                c.trailing_line_comment.as_ref(),
                c.blank_line_before,
                c.doc_comment.as_deref(),
            ),
            SheetItem::Relationship(c) => (
                c.leading_comment.as_ref(),
                c.trailing_line_comment.as_ref(),
                c.blank_line_before,
                c.doc_comment.as_deref(),
            ),
            SheetItem::RelationshipUse(c) => (
                c.leading_comment.as_ref(),
                c.trailing_line_comment.as_ref(),
                c.blank_line_before,
                c.doc_comment.as_deref(),
            ),
            SheetItem::Conditional(c) => (
                c.leading_comment.as_ref(),
                c.trailing_line_comment.as_ref(),
                c.blank_line_before,
                c.doc_comment.as_deref(),
            ),
            SheetItem::Error {
                leading_comment,
                trailing_line_comment,
                blank_line_before,
                doc_comment,
                ..
            } => (
                leading_comment.as_ref(),
                trailing_line_comment.as_ref(),
                *blank_line_before,
                doc_comment.as_deref(),
            ),
        }
    }

    /// Wraps a token span as a point expression span.
    fn point(span: Span) -> ExprSpan {
        ExprSpan {
            start: span,
            end: span,
        }
    }

    /// Returns the stored cell declaration span.
    #[test]
    fn sheet_item_span_reads_the_cell_variant() {
        let span = point(Span::call_site());
        let item = SheetItem::Cell(CellDecl {
            name: "x".to_string(),
            name_span: span,
            type_name: None,
            initializer: None,
            filter: None,
            require: None,
            leading_comment: None,
            doc_comment: None,
            trailing_line_comment: None,
            blank_line_before: false,
            span,
        });
        assert_eq!(format!("{:?}", item.span()), format!("{span:?}"));
    }

    /// Returns the stored inline relationship span.
    #[test]
    fn sheet_item_span_reads_the_relationship_variant() {
        let span = point(Span::call_site());
        let item = SheetItem::Relationship(RelationshipDecl {
            bindings: Vec::new(),
            leading_comment: None,
            doc_comment: None,
            trailing_line_comment: None,
            blank_line_before: false,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: span,
            span,
        });
        assert_eq!(format!("{:?}", item.span()), format!("{span:?}"));
    }

    /// Returns the stored conditional declaration span.
    #[test]
    fn sheet_item_span_reads_the_conditional_variant() {
        let span = point(Span::call_site());
        let item = SheetItem::Conditional(ConditionalDecl {
            match_expr: cel_parser::Expr::Ident {
                name: "m".to_string(),
                span,
            },
            branches: Vec::new(),
            default: None,
            leading_comment: None,
            doc_comment: None,
            trailing_line_comment: None,
            blank_line_before: false,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: span,
            span,
        });
        assert_eq!(format!("{:?}", item.span()), format!("{span:?}"));
    }

    /// Returns the stored recovery range.
    #[test]
    fn sheet_item_span_reads_the_error_variant() {
        let span = point(Span::call_site());
        let item = SheetItem::Error {
            span,
            leading_comment: None,
            doc_comment: None,
            trailing_line_comment: None,
            blank_line_before: false,
        };
        assert_eq!(format!("{:?}", item.span()), format!("{span:?}"));
    }

    /// Attaches a leading comment to a cell.
    #[test]
    fn set_leading_comment_sets_the_cell_variant() {
        let span = point(Span::call_site());
        let mut item = SheetItem::Cell(CellDecl {
            name: "x".to_string(),
            name_span: span,
            type_name: None,
            initializer: None,
            filter: None,
            require: None,
            leading_comment: None,
            doc_comment: None,
            trailing_line_comment: None,
            blank_line_before: false,
            span,
        });
        item.set_leading_comment(Comment::Line("hi".to_string()));
        match item {
            SheetItem::Cell(c) => {
                assert_eq!(c.leading_comment, Some(Comment::Line("hi".to_string())))
            }
            other => panic!("expected Cell, got {other:?}"),
        }
    }

    /// Attaches a leading comment to a recovery placeholder.
    #[test]
    fn set_leading_comment_sets_the_error_variant() {
        let span = point(Span::call_site());
        let mut item = SheetItem::Error {
            span,
            leading_comment: None,
            doc_comment: None,
            trailing_line_comment: None,
            blank_line_before: false,
        };
        item.set_leading_comment(Comment::Line("hi".to_string()));
        match item {
            SheetItem::Error {
                leading_comment, ..
            } => {
                assert_eq!(leading_comment, Some(Comment::Line("hi".to_string())))
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    /// Marks a cell as preceded by a blank line.
    #[test]
    fn set_blank_line_before_sets_the_cell_variant() {
        let span = point(Span::call_site());
        let mut item = SheetItem::Cell(CellDecl {
            name: "x".to_string(),
            name_span: span,
            type_name: None,
            initializer: None,
            filter: None,
            require: None,
            leading_comment: None,
            doc_comment: None,
            trailing_line_comment: None,
            blank_line_before: false,
            span,
        });
        item.set_blank_line_before(true);
        match item {
            SheetItem::Cell(c) => assert!(c.blank_line_before),
            other => panic!("expected Cell, got {other:?}"),
        }
    }

    /// Returns the stored output declaration span.
    #[test]
    fn sheet_item_span_reads_the_out_variant() {
        let span = point(Span::call_site());
        let item = SheetItem::Out(OutDecl {
            name: "o".to_string(),
            name_span: span,
            type_name: None,
            initializer: cel_parser::Expr::Ident {
                name: "x".to_string(),
                span,
            },
            filter: None,
            require: None,
            leading_comment: None,
            doc_comment: None,
            trailing_line_comment: None,
            blank_line_before: false,
            span,
        });
        assert_eq!(format!("{:?}", item.span()), format!("{span:?}"));
    }

    /// Attaches a leading comment to an output.
    #[test]
    fn set_leading_comment_sets_the_out_variant() {
        let span = point(Span::call_site());
        let mut item = SheetItem::Out(OutDecl {
            name: "o".to_string(),
            name_span: span,
            type_name: None,
            initializer: cel_parser::Expr::Ident {
                name: "x".to_string(),
                span,
            },
            filter: None,
            require: None,
            leading_comment: None,
            doc_comment: None,
            trailing_line_comment: None,
            blank_line_before: false,
            span,
        });
        item.set_leading_comment(Comment::Line("hi".to_string()));
        match item {
            SheetItem::Out(o) => {
                assert_eq!(o.leading_comment, Some(Comment::Line("hi".to_string())))
            }
            other => panic!("expected Out, got {other:?}"),
        }
    }

    /// Accepts the CEL parser type expression without conversion.
    #[test]
    fn ast_type_expr_is_the_shared_cel_parser_type_expr() {
        /// Requires the expression's static type to match the shared parser type.
        fn assert_same_type<T>(_: &T) {}
        let span = cel_parser::ExprSpan {
            start: Span::call_site(),
            end: Span::call_site(),
        };
        let expr: TypeExpr = cel_parser::TypeExpr::Named {
            name: "i32".to_string(),
            args: Vec::new(),
            span,
        };
        assert_same_type::<cel_parser::TypeExpr>(&expr);
    }

    /// Returns a named type expression's stored span.
    #[test]
    fn type_expr_named_span_is_its_own_span() {
        let span = point(Span::call_site());
        let expr = TypeExpr::Named {
            name: "i32".to_string(),
            args: Vec::new(),
            span,
        };
        assert_eq!(format!("{:?}", expr.span()), format!("{span:?}"));
    }

    /// Returns a tuple type expression's stored span.
    #[test]
    fn type_expr_tuple_span_is_the_whole_parenthesized_span() {
        let span = point(Span::call_site());
        let expr = TypeExpr::Tuple {
            elements: Vec::new(),
            span,
        };
        assert_eq!(format!("{:?}", expr.span()), format!("{span:?}"));
    }
}
