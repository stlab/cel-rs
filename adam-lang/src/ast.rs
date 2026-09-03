//! A span-carrying adam-lang structural AST, built by [`crate::AdamAstParser`] as an alternative to
//! [`crate::AdamParser`]'s direct `adam_rs::Sheet` construction. Method bodies and cell
//! initializers reference [`cel_parser::Expr`]/[`cel_parser::lex_lexer::Literal`] directly.
//! Carries no resolved types, no `TypeRegistry` lookups, and never fails on semantic grounds
//! (unknown type name, literal/type mismatch, undeclared cell, arity mismatch) — those checks
//! are deferred to a later, separate compile-to-`Sheet` phase, mirroring
//! [`cel_parser::AstContext`]'s design.

pub use cel_parser::ExprSpan;
use cel_parser::lex_lexer::Literal;

/// A recovered `//`/`/* */` comment, remembering which delimiter style the source used so the
/// formatter can reproduce it instead of normalizing everything to `//`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Comment {
    /// One or more consecutive `// text` lines, joined by `\n`, each with its leading `//`/space
    /// stripped.
    Line(String),
    /// A single `/* text */` block comment (single- or multi-line), its inner text joined by
    /// `\n` with the opening `/*`/closing `*/` and per-line indentation stripped.
    Block(String),
}

/// A parsed adam-lang sheet declaration, with source spans on every node.
///
/// Built by [`crate::AdamAstParser`]; consumed by the language server, the formatter, and the
/// (separate, future) compile-to-`Sheet` phase.
#[derive(Debug, Clone)]
pub struct Sheet {
    /// The sheet's declared name.
    pub name: String,
    /// The name token's span.
    pub name_span: ExprSpan,
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
#[derive(Debug, Clone)]
pub enum SheetItem {
    /// A `cell` declaration.
    Cell(CellDecl),
    /// A `relationship` declaration.
    Relationship(RelationshipDecl),
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
        /// Whether the gap before this item contained a blank line, if recovered by
        /// [`crate::trivia::attach_trivia`].
        blank_line_before: bool,
    },
}

impl SheetItem {
    /// Returns this item's source span.
    pub fn span(&self) -> ExprSpan {
        match self {
            SheetItem::Cell(c) => c.span,
            SheetItem::Relationship(r) => r.span,
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
            SheetItem::Conditional(c) => c.leading_comment = Some(comment),
            SheetItem::Out(o) => o.leading_comment = Some(comment),
            SheetItem::Source(s) => s.leading_comment = Some(comment),
            SheetItem::Error {
                leading_comment, ..
            } => *leading_comment = Some(comment),
        }
    }

    /// Sets whether a blank line preceded this item.
    pub(crate) fn set_blank_line_before(&mut self, value: bool) {
        match self {
            SheetItem::Cell(c) => c.blank_line_before = value,
            SheetItem::Relationship(r) => r.blank_line_before = value,
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

/// `type_expr = identifier | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".`
///
/// `()` is the empty tuple type (0 elements); `(T)` is grouping (same as bare `T` — types have
/// no precedence to disambiguate, but staying symmetric with `cel_parser`'s expression grammar
/// costs nothing); `(T,)` is a 1-element tuple; `(T, U, ...)` is n-element, no trailing comma.
#[derive(Debug, Clone)]
pub enum TypeExpr {
    /// A single type name, resolved later against a `TypeRegistry`.
    Named(String, ExprSpan),
    /// A tuple type, recursively — `Vec::new()` for `()`.
    Tuple(Vec<TypeExpr>, ExprSpan),
}

impl TypeExpr {
    /// Returns this type expression's source span.
    pub fn span(&self) -> ExprSpan {
        match self {
            TypeExpr::Named(_, span) | TypeExpr::Tuple(_, span) => *span,
        }
    }
}

/// `cell_decl = "cell" identifier cell_type_init [ cell_filter ] [ "require" "{" { requirement }
/// "}" ] ";".`
///
/// `type_name`/`initializer` are unresolved — no `TypeRegistry` lookup, no literal validation.
/// Exactly one of `type_name`, `initializer` may be absent, per the grammar's two
/// `cell_type_init` forms, but this is not enforced here (an all-`None` `CellDecl` cannot be
/// produced by [`crate::AdamAstParser`], which requires at least one of `:`/`=`).
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
    /// Whether a blank line preceded this declaration, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `cell ...;` declaration.
    pub span: ExprSpan,
}

/// `source_decl = "source" identifier cell_type_init [ cell_filter ] [ "require" "{" {
/// requirement } "}" ] ";".`
///
/// Same shape as [`CellDecl`]: a `source` cell's initializer is a one-time literal exactly like a
/// plain `cell`'s, and it supports the same `filter` clause and `require` block.
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
    /// Whether a blank line preceded this declaration, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `source ...;` declaration.
    pub span: ExprSpan,
}

/// `cell_filter = "filter" identifier ":" expression.`
#[derive(Debug, Clone)]
pub struct CellFilter {
    /// The filter's declared name.
    pub name: String,
    /// The name token's span.
    pub name_span: ExprSpan,
    /// The filter's body expression. `_` inside it denotes the candidate value being conformed;
    /// every other identifier that names an already-declared cell is a deduced dependency.
    pub body: cel_parser::Expr,
    /// The span of the whole `filter ...` clause.
    pub span: ExprSpan,
}

/// `relationship_decl = "relationship" "{" { binding } "}".`
#[derive(Debug, Clone)]
pub struct RelationshipDecl {
    /// The relationship block's bindings, in declaration order.
    pub bindings: Vec<BindingDecl>,
    /// A leading comment immediately preceding this declaration, if recovered.
    pub leading_comment: Option<Comment>,
    /// A leading `///` doc comment immediately preceding this declaration, if recovered by
    /// [`crate::AdamAstParser`].
    pub doc_comment: Option<String>,
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
/// `binding_target = identifier | "(" identifier { "," identifier } [ "," ] ")".`
///
/// Unlike the old `method_decl` this replaces, a binding names no explicit input cell list —
/// its inputs are whichever already-declared cells `body` references, deduced at compile time
/// (see `crate::parser::AdamParser::parse_deduced_expr`); this untyped CST parser has no cell
/// declarations to resolve against, so it records no input list at all, only the outputs.
///
/// Parenthesizing the left-hand side requests tuple destructuring, matching Rust's tuple-pattern
/// syntax: `(a, b) := ...` and the single-element `(a,) := ...` (trailing comma mandatory,
/// exactly as in a Rust 1-tuple pattern) both destructure `body`'s tuple result element-wise into
/// `outputs`. A bare identifier, or a single parenthesized identifier with no comma (`(a) :=
/// ...`, mere grouping), binds `body`'s whole result directly to that one output instead — see
/// [`Self::destructure`].
#[derive(Debug, Clone)]
pub struct BindingDecl {
    /// The binding's output cell names (the left-hand side), in declaration order.
    pub outputs: Vec<(String, ExprSpan)>,
    /// Whether the left-hand side requests destructuring (`(a, b) := ...` or `(a,) := ...`) as
    /// opposed to a direct bind (`a := ...` or the equivalent grouping `(a) := ...`). Always
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
    /// Whether a blank line preceded this declaration, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `out ... ;` declaration.
    pub span: ExprSpan,
}

/// The optional `require { ... }` block trailing an `out` declaration's initializer.
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

/// `requirement = identifier ":" expression ";".`
///
/// `name` is a plain string label passed to `adam_rs::Sheet::add_requirement`, not a cell
/// reference — it may coincide with a cell name declared elsewhere in the sheet but doesn't
/// have to.
#[derive(Debug, Clone)]
pub struct RequirementDecl {
    /// The requirement's declared name.
    pub name: String,
    /// The name token's span.
    pub name_span: ExprSpan,
    /// The parsed requirement body expression; must type-check as `bool`.
    pub body: cel_parser::Expr,
    /// A leading comment immediately preceding this requirement, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<Comment>,
    /// Whether a blank line preceded this requirement, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `name: ...;` declaration.
    pub span: ExprSpan,
}

/// `conditional_decl = "conditional" expression "{" { conditional_branch } "}".`
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
#[derive(Debug, Clone)]
pub struct DefaultBranch {
    /// The default branch's relationships, in declaration order.
    pub relationships: Vec<RelationshipDecl>,
    /// A trailing comment immediately preceding this branch's own closing `}`, if recovered.
    /// See <https://github.com/stlab/cel-rs/issues/52>.
    pub trailing_comment: Option<Comment>,
    /// Whether a blank line preceded this branch's own closing `}`, if recovered.
    pub blank_line_before_close: bool,
    /// The span of this branch's own opening `{`, used to recover trailing trivia when
    /// `relationships` is empty.
    pub open_brace_span: ExprSpan,
    /// The span from the `_` token through this branch's own closing `}`.
    pub span: ExprSpan,
}

/// `conditional_branch = literal_pattern "=>" "{" { relationship_decl } "}" [ "," ].`
/// `literal_pattern = ["-"] literal.`
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
    pub relationships: Vec<RelationshipDecl>,
    /// A leading comment immediately preceding this branch, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<Comment>,
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

    fn point(span: Span) -> ExprSpan {
        ExprSpan {
            start: span,
            end: span,
        }
    }

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
            blank_line_before: false,
            span,
        });
        assert_eq!(format!("{:?}", item.span()), format!("{span:?}"));
    }

    #[test]
    fn sheet_item_span_reads_the_relationship_variant() {
        let span = point(Span::call_site());
        let item = SheetItem::Relationship(RelationshipDecl {
            bindings: Vec::new(),
            leading_comment: None,
            doc_comment: None,
            blank_line_before: false,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: span,
            span,
        });
        assert_eq!(format!("{:?}", item.span()), format!("{span:?}"));
    }

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
            blank_line_before: false,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: span,
            span,
        });
        assert_eq!(format!("{:?}", item.span()), format!("{span:?}"));
    }

    #[test]
    fn sheet_item_span_reads_the_error_variant() {
        let span = point(Span::call_site());
        let item = SheetItem::Error {
            span,
            leading_comment: None,
            doc_comment: None,
            blank_line_before: false,
        };
        assert_eq!(format!("{:?}", item.span()), format!("{span:?}"));
    }

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

    #[test]
    fn set_leading_comment_sets_the_error_variant() {
        let span = point(Span::call_site());
        let mut item = SheetItem::Error {
            span,
            leading_comment: None,
            doc_comment: None,
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
            blank_line_before: false,
            span,
        });
        item.set_blank_line_before(true);
        match item {
            SheetItem::Cell(c) => assert!(c.blank_line_before),
            other => panic!("expected Cell, got {other:?}"),
        }
    }

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
            blank_line_before: false,
            span,
        });
        assert_eq!(format!("{:?}", item.span()), format!("{span:?}"));
    }

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

    #[test]
    fn type_expr_named_span_is_its_own_span() {
        let span = point(Span::call_site());
        let expr = TypeExpr::Named("i32".to_string(), span);
        assert_eq!(format!("{:?}", expr.span()), format!("{span:?}"));
    }

    #[test]
    fn type_expr_tuple_span_is_the_whole_parenthesized_span() {
        let span = point(Span::call_site());
        let expr = TypeExpr::Tuple(Vec::new(), span);
        assert_eq!(format!("{:?}", expr.span()), format!("{span:?}"));
    }

    #[test]
    fn cell_decl_type_name_holds_a_nested_tuple_type_expr() {
        let span = point(Span::call_site());
        let cell = CellDecl {
            name: "a".to_string(),
            name_span: span,
            type_name: Some(TypeExpr::Tuple(
                vec![
                    TypeExpr::Named("i32".to_string(), span),
                    TypeExpr::Named("f64".to_string(), span),
                ],
                span,
            )),
            initializer: None,
            filter: None,
            require: None,
            leading_comment: None,
            doc_comment: None,
            blank_line_before: false,
            span,
        };
        match cell.type_name {
            Some(TypeExpr::Tuple(elements, _)) => assert_eq!(elements.len(), 2),
            other => panic!("expected Tuple, got {other:?}"),
        }
    }

    #[test]
    fn cell_decl_initializer_holds_a_parsed_expr() {
        let span = point(Span::call_site());
        let cell = CellDecl {
            name: "a".to_string(),
            name_span: span,
            type_name: None,
            initializer: Some(cel_parser::Expr::Ident {
                name: "x".to_string(),
                span,
            }),
            filter: None,
            require: None,
            leading_comment: None,
            doc_comment: None,
            blank_line_before: false,
            span,
        };
        assert!(matches!(
            cell.initializer,
            Some(cel_parser::Expr::Ident { .. })
        ));
    }

    #[test]
    fn cell_decl_filter_field_holds_a_cell_filter() {
        let span = point(Span::call_site());
        let cell = CellDecl {
            name: "a".to_string(),
            name_span: span,
            type_name: None,
            initializer: None,
            filter: Some(CellFilter {
                name: "clamp".to_string(),
                name_span: span,
                body: cel_parser::Expr::Ident {
                    name: "_".to_string(),
                    span,
                },
                span,
            }),
            require: None,
            leading_comment: None,
            doc_comment: None,
            blank_line_before: false,
            span,
        };
        let filter = cell.filter.as_ref().expect("filter present");
        assert!(matches!(
            &filter.body,
            cel_parser::Expr::Ident { name, .. } if name == "_"
        ));
    }
}
