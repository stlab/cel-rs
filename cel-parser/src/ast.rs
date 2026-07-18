//! A span-carrying CEL expression AST, built by [`AstContext`](crate::ast::AstContext) as an
//! alternative to [`DynSegmentContext`](crate::parser_context::DynSegmentContext)'s direct
//! execution. Consumed as-is by pm-lang (method bodies/initializers), the language server, the
//! formatter, and the future macro-compilation backend. Carries no resolved types or operator
//! overloads: resolution and type/range validation are deferred to a later, separate phase.

use proc_macro2::Span;
use std::ffi::CString;

/// Source range of an AST node: start of its first token to end of its last.
///
/// Two `proc_macro2::Span`s (not a [`crate::SourceSpan`]) so the future macro-compilation backend
/// can attach `compile_error!`/`quote_spanned!`, and so the (separate, deferred) comment/trivia
/// pass can call `Span::source_text()`. `crate::SourceSpan` remains the `Send + Sync` wire format
/// used only at the `CELError` diagnostic boundary; this AST is an internal parser artifact using
/// the same span currency the parser itself already does.
///
/// `proc_macro2::Span` isn't `PartialEq`, so neither is this type — shape tests should match
/// structurally and ignore spans rather than assert exact span equality.
#[derive(Clone, Copy, Debug)]
pub struct ExprSpan {
    /// Start of the first token of the node.
    pub start: Span,
    /// End of the last token of the node.
    pub end: Span,
}

impl ExprSpan {
    /// A single-token range where start and end coincide.
    // Not yet called from this crate: reserved for `AstContext` (Task 3), which builds
    // leaf `Expr` nodes from single tokens.
    #[allow(dead_code)]
    fn point(span: Span) -> Self {
        ExprSpan {
            start: span,
            end: span,
        }
    }
}

/// A CEL literal, one variant per concrete Rust type [`crate::ParserContext::push_literal`] can
/// receive from `push_literal_token` in `lib.rs`.
///
/// `Byte` literals (`b'A'`) and `u8`-suffixed integer literals (`65u8`) both arrive as `u8` and
/// are indistinguishable here — both become `Literal::U8`; a formatter that must reproduce exact
/// original syntax should re-slice the node's original source text from its span instead of
/// relying on this enum to recover lexical form.
#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    /// `i8` literal (e.g. `1i8`).
    I8(i8),
    /// `i16` literal (e.g. `1i16`).
    I16(i16),
    /// `i32` literal, the default integer suffix (e.g. `1` or `1i32`).
    I32(i32),
    /// `i64` literal (e.g. `1i64`).
    I64(i64),
    /// `i128` literal (e.g. `1i128`).
    I128(i128),
    /// `isize` literal (e.g. `1isize`).
    Isize(isize),
    /// `u8` literal or byte literal (e.g. `1u8` or `b'A'`).
    U8(u8),
    /// `u16` literal (e.g. `1u16`).
    U16(u16),
    /// `u32` literal (e.g. `1u32`).
    U32(u32),
    /// `u64` literal (e.g. `1u64`).
    U64(u64),
    /// `u128` literal (e.g. `1u128`).
    U128(u128),
    /// `usize` literal (e.g. `1usize`).
    Usize(usize),
    /// `f32` literal (e.g. `1.0f32`).
    F32(f32),
    /// `f64` literal, the default float suffix (e.g. `1.0` or `1.0f64`).
    F64(f64),
    /// Boolean literal (`true`/`false`).
    Bool(bool),
    /// Character literal (e.g. `'a'`).
    Char(char),
    /// String literal (e.g. `"x"`).
    Str(String),
    /// Byte-string literal (e.g. `b"x"`).
    ByteStr(Vec<u8>),
    /// C-string literal (e.g. `c"x"`).
    CStr(CString),
    /// Unit (`()`).
    Unit,
}

/// The two short-circuiting logical operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicalOp {
    /// `&&`.
    And,
    /// `||`.
    Or,
}

/// A parsed CEL expression with source spans on every node.
///
/// Built by [`AstContext`](crate::ast::AstContext); consumed as-is by pm-lang (method
/// bodies/initializers), the language server (hover/goto), the formatter, and the future
/// macro-compilation backend. Carries no resolved types or operator overloads — resolution is
/// deferred to a later, separate type-checking phase.
///
/// `Logical` is kept distinct from `If` (rather than desugaring `a || b` to
/// `if a { true } else { b }`, which is how [`DynSegmentContext`](crate::parser_context::DynSegmentContext)
/// executes it) so a formatter can round-trip `a || b` as `a || b`.
#[derive(Clone, Debug)]
pub enum Expr {
    /// A literal value (`10i32`, `"x"`, `true`, `()`, ...).
    Literal {
        /// The literal's value.
        value: Literal,
        /// The literal token's span.
        span: ExprSpan,
    },
    /// A bare identifier reference or zero-arg builtin lookup (`x`, `pi`) — unresolved.
    Ident {
        /// The identifier's name.
        name: String,
        /// The identifier token's span.
        span: ExprSpan,
    },
    /// A prefix (arity 1) or infix (arity 2) operator application (`-x`, `a + b`, `a == b`).
    Op {
        /// The operator's name (e.g. `"+"`, `"-"`, `"=="`).
        name: String,
        /// The operand sub-expressions, in source order.
        operands: Vec<Expr>,
        /// The span of the whole operator application.
        span: ExprSpan,
    },
    /// A call: `callee(args...)` — the grammar's `"()"` operator.
    Apply {
        /// The expression being called.
        callee: Box<Expr>,
        /// The argument sub-expressions, in source order.
        args: Vec<Expr>,
        /// The span of the whole call, including its argument list.
        span: ExprSpan,
    },
    /// A tuple literal (`(a, b, ...)`; a 1-tuple is `(a,)`).
    Tuple {
        /// The element sub-expressions, in source order.
        elements: Vec<Expr>,
        /// The span of the whole tuple, including its parentheses.
        span: ExprSpan,
    },
    /// A tuple index (`base.N`). Whether `base` is actually a tuple and `N` is in range is
    /// unchecked here — deferred to the type-checking phase (see the module doc comment).
    TupleIndex {
        /// The expression being indexed.
        base: Box<Expr>,
        /// The index.
        index: usize,
        /// The span from the start of `base` through the index token.
        span: ExprSpan,
    },
    /// An `if cond { then } else { else_ }` expression (implicit else is `Literal(Unit)`).
    If {
        /// The condition.
        cond: Box<Expr>,
        /// The then-branch.
        then_branch: Box<Expr>,
        /// The else-branch (a synthesized `Literal(Unit)` node if no `else` was written).
        else_branch: Box<Expr>,
        /// The span of the whole `if`/`else` construct.
        span: ExprSpan,
    },
    /// A short-circuiting `&&`/`||`.
    Logical {
        /// Which logical operator.
        op: LogicalOp,
        /// The left-hand side.
        lhs: Box<Expr>,
        /// The right-hand side.
        rhs: Box<Expr>,
        /// The span of the whole logical expression.
        span: ExprSpan,
    },
}

impl Expr {
    /// Returns this node's source span.
    pub fn span(&self) -> ExprSpan {
        match self {
            Expr::Literal { span, .. }
            | Expr::Ident { span, .. }
            | Expr::Op { span, .. }
            | Expr::Apply { span, .. }
            | Expr::Tuple { span, .. }
            | Expr::TupleIndex { span, .. }
            | Expr::If { span, .. }
            | Expr::Logical { span, .. } => *span,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::Span;

    #[test]
    fn span_returns_the_range_stored_on_a_leaf_variant() {
        let target = ExprSpan {
            start: Span::call_site(),
            end: Span::call_site(),
        };
        let expr = Expr::Ident {
            name: "x".to_string(),
            span: target,
        };
        assert_eq!(format!("{:?}", expr.span()), format!("{target:?}"));
    }

    #[test]
    fn span_returns_the_range_stored_on_a_composite_variant() {
        let target = ExprSpan {
            start: Span::call_site(),
            end: Span::call_site(),
        };
        let expr = Expr::If {
            cond: Box::new(Expr::Literal {
                value: Literal::Bool(true),
                span: target,
            }),
            then_branch: Box::new(Expr::Literal {
                value: Literal::I32(1),
                span: target,
            }),
            else_branch: Box::new(Expr::Literal {
                value: Literal::I32(2),
                span: target,
            }),
            span: target,
        };
        assert_eq!(format!("{:?}", expr.span()), format!("{target:?}"));
    }

    #[test]
    fn literal_variants_are_distinguishable_and_comparable() {
        assert_eq!(Literal::I32(1), Literal::I32(1));
        assert_ne!(Literal::I32(1), Literal::I32(2));
        assert_ne!(Literal::I32(1), Literal::U8(1));
        assert_eq!(Literal::Unit, Literal::Unit);
    }

    #[test]
    fn logical_op_equality() {
        assert_eq!(LogicalOp::And, LogicalOp::And);
        assert_ne!(LogicalOp::And, LogicalOp::Or);
    }
}
