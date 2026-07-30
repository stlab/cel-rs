//! A span-carrying adam-lang structural AST, built by [`crate::AdamAstParser`] as an alternative to
//! [`crate::AdamParser`]'s direct `adam_rs::Sheet` construction. Method bodies and cell
//! initializers reference [`cel_parser::Expr`]/[`cel_parser::lex_lexer::Literal`] directly.
//! Carries no resolved types, no `TypeRegistry` lookups, and never fails on semantic grounds
//! (unknown type name, literal/type mismatch, undeclared cell, arity mismatch) — those checks
//! are deferred to a later, separate compile-to-`Sheet` phase, mirroring
//! [`cel_parser::AstContext`]'s design.

pub use cel_parser::ExprSpan;
use cel_parser::lex_lexer::Literal;

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
    /// A syntax error recovered at declaration granularity; `span` covers the skipped tokens.
    Error {
        /// The span of the skipped, malformed item.
        span: ExprSpan,
        /// A leading `//`/`/* */` comment immediately preceding this item, if recovered by
        /// [`crate::trivia::attach_trivia`]. Preserved even though the item failed to parse, so
        /// a comment explaining a broken declaration (e.g. `// TODO: fix this`) isn't silently
        /// dropped.
        leading_comment: Option<String>,
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
            SheetItem::Error { span, .. } => *span,
        }
    }

    /// Sets this item's leading comment.
    pub(crate) fn set_leading_comment(&mut self, comment: String) {
        match self {
            SheetItem::Cell(c) => c.leading_comment = Some(comment),
            SheetItem::Relationship(r) => r.leading_comment = Some(comment),
            SheetItem::Conditional(c) => c.leading_comment = Some(comment),
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
            SheetItem::Error {
                blank_line_before, ..
            } => *blank_line_before = value,
        }
    }
}

/// `cell_decl = "cell" identifier cell_type_init ";".`
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
    /// The `: type_name` annotation, if present.
    pub type_name: Option<(String, ExprSpan)>,
    /// The `= literal` initializer, if present.
    pub initializer: Option<(Literal, ExprSpan)>,
    /// A leading `//`/`/* */` comment immediately preceding this declaration, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<String>,
    /// Whether a blank line preceded this declaration, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `cell ...;` declaration.
    pub span: ExprSpan,
}

/// `relationship_decl = "relationship" [ identifier ] "{" { method_decl } "}".`
#[derive(Debug, Clone)]
pub struct RelationshipDecl {
    /// The relationship's optional name.
    pub name: Option<(String, ExprSpan)>,
    /// The relationship's methods, in declaration order.
    pub methods: Vec<MethodDecl>,
    /// A leading comment immediately preceding this declaration, if recovered.
    pub leading_comment: Option<String>,
    /// Whether a blank line preceded this declaration, if recovered.
    pub blank_line_before: bool,
    /// The span of the whole `relationship { ... }` declaration.
    pub span: ExprSpan,
}

/// `conditional_decl = "conditional" identifier "{" { conditional_branch } [ default_branch ] "}".`
#[derive(Debug, Clone)]
pub struct ConditionalDecl {
    /// The name of the cell this conditional matches on.
    pub match_name: String,
    /// The match cell name token's span.
    pub match_name_span: ExprSpan,
    /// The named (literal `=>`) branches, in declaration order.
    pub branches: Vec<ConditionalBranch>,
    /// The `_ => { ... }` default branch's relationships, if present.
    pub default: Option<Vec<RelationshipDecl>>,
    /// A leading comment immediately preceding this declaration, if recovered.
    pub leading_comment: Option<String>,
    /// Whether a blank line preceded this declaration, if recovered.
    pub blank_line_before: bool,
    /// The span of the whole `conditional ... { ... }` declaration.
    pub span: ExprSpan,
}

/// `conditional_branch = literal "=>" "{" { relationship_decl } "}" [ "," ].`
#[derive(Debug, Clone)]
pub struct ConditionalBranch {
    /// The branch's unresolved match literal.
    pub literal: Literal,
    /// The literal token's span.
    pub literal_span: ExprSpan,
    /// The branch's relationships, in declaration order.
    pub relationships: Vec<RelationshipDecl>,
    /// A leading comment immediately preceding this branch, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<String>,
    /// Whether a blank line preceded this branch, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span from the branch's literal through its closing `}`.
    pub span: ExprSpan,
}

/// `method_decl = "method" cell_list "->" cell_list method_body.`
///
/// `method_body = "{" or_expression "}"`, parsed via `cel_parser::Parser<AstContext>` into
/// `body`. Cell names referenced in `body` (e.g. `width` in `{ width * height }`) are recorded
/// as plain `Expr::Ident` nodes — resolving them against `inputs` is deferred to the compile
/// phase, exactly as `cel_parser::AstContext` defers all identifier resolution.
#[derive(Debug, Clone)]
pub struct MethodDecl {
    /// The method's input cell names (the first `cell_list`).
    pub inputs: Vec<(String, ExprSpan)>,
    /// The method's output cell names (the second `cell_list`).
    pub outputs: Vec<(String, ExprSpan)>,
    /// The parsed method body expression.
    pub body: cel_parser::Expr,
    /// A leading comment immediately preceding this method, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<String>,
    /// Whether a blank line preceded this method, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `method [...] -> [...] { ... }` declaration.
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
            leading_comment: None,
            blank_line_before: false,
            span,
        });
        assert_eq!(format!("{:?}", item.span()), format!("{span:?}"));
    }

    #[test]
    fn sheet_item_span_reads_the_relationship_variant() {
        let span = point(Span::call_site());
        let item = SheetItem::Relationship(RelationshipDecl {
            name: None,
            methods: Vec::new(),
            leading_comment: None,
            blank_line_before: false,
            span,
        });
        assert_eq!(format!("{:?}", item.span()), format!("{span:?}"));
    }

    #[test]
    fn sheet_item_span_reads_the_conditional_variant() {
        let span = point(Span::call_site());
        let item = SheetItem::Conditional(ConditionalDecl {
            match_name: "m".to_string(),
            match_name_span: span,
            branches: Vec::new(),
            default: None,
            leading_comment: None,
            blank_line_before: false,
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
            leading_comment: None,
            blank_line_before: false,
            span,
        });
        item.set_leading_comment("hi".to_string());
        match item {
            SheetItem::Cell(c) => assert_eq!(c.leading_comment.as_deref(), Some("hi")),
            other => panic!("expected Cell, got {other:?}"),
        }
    }

    #[test]
    fn set_leading_comment_sets_the_error_variant() {
        let span = point(Span::call_site());
        let mut item = SheetItem::Error {
            span,
            leading_comment: None,
            blank_line_before: false,
        };
        item.set_leading_comment("hi".to_string());
        match item {
            SheetItem::Error {
                leading_comment, ..
            } => {
                assert_eq!(leading_comment.as_deref(), Some("hi"))
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
            leading_comment: None,
            blank_line_before: false,
            span,
        });
        item.set_blank_line_before(true);
        match item {
            SheetItem::Cell(c) => assert!(c.blank_line_before),
            other => panic!("expected Cell, got {other:?}"),
        }
    }
}
