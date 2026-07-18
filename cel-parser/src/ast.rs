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

use std::any::Any;

use crate::op_table::OpLookup;
use crate::parser_context::ParserContext;

/// Converts a statically-known literal value into its [`Literal`] variant.
///
/// - Precondition: `T` is one of the concrete types `push_literal_token` (`lib.rs`) pushes:
///   the signed/unsigned integer widths, `f32`/`f64`, `bool`, `char`, `String`, `Vec<u8>`,
///   `CString`, or `()`.
fn to_literal<T: 'static + Clone>(value: &T) -> Literal {
    let any = value as &dyn Any;
    macro_rules! map {
        ($($t:ty => $variant:path),+ $(,)?) => {
            $( if let Some(x) = any.downcast_ref::<$t>() { return $variant(x.clone()); } )+
        };
    }
    map! {
        i8 => Literal::I8, i16 => Literal::I16, i32 => Literal::I32, i64 => Literal::I64,
        i128 => Literal::I128, isize => Literal::Isize,
        u8 => Literal::U8, u16 => Literal::U16, u32 => Literal::U32, u64 => Literal::U64,
        u128 => Literal::U128, usize => Literal::Usize,
        f32 => Literal::F32, f64 => Literal::F64,
        bool => Literal::Bool, char => Literal::Char, String => Literal::Str,
        Vec<u8> => Literal::ByteStr, CString => Literal::CStr,
    }
    if any.is::<()>() {
        return Literal::Unit;
    }
    unreachable!("push_literal called with an unsupported literal type")
}

/// [`ParserContext`] implementation that builds a span-carrying [`Expr`] tree instead of
/// executing. Consults no [`OpLookup`], inspects no runtime types, and never fails on semantic
/// grounds — resolution and type/range checking are deferred to a later, separate
/// type-checking phase (see the module doc comment).
///
/// # Examples
///
/// ```rust
/// use cel_parser::{AstContext, ParserContext};
/// use proc_macro2::Span;
///
/// let mut ctx = AstContext::new_context();
/// ctx.push_literal(10i32, Span::call_site());
/// ```
#[derive(Debug, Default)]
pub struct AstContext {
    /// Finished sub-trees; a completed parse leaves exactly one.
    values: Vec<Expr>,
}

impl AstContext {
    /// Removes and returns the top node.
    ///
    /// - Precondition: at least one node is present.
    fn pop(&mut self) -> Expr {
        self.values
            .pop()
            .expect("operand present on AST value stack")
    }

    /// Removes and returns the top `n` nodes, in source order.
    ///
    /// - Precondition: at least `n` nodes are present.
    fn pop_n(&mut self, n: usize) -> Vec<Expr> {
        let at = self.values.len() - n;
        self.values.split_off(at)
    }

    /// Consumes the context, returning the single parsed expression.
    ///
    /// - Precondition: parsing completed successfully (exactly one node remains).
    pub fn into_expr(mut self) -> Expr {
        debug_assert_eq!(
            self.values.len(),
            1,
            "a successfully parsed expression leaves exactly one node"
        );
        self.pop()
    }
}

impl ParserContext for AstContext {
    fn new_context() -> Self {
        AstContext { values: Vec::new() }
    }

    fn new_fragment(&self) -> Self {
        AstContext { values: Vec::new() }
    }

    fn push_literal<T: 'static + Clone>(&mut self, value: T, span: Span) {
        self.values.push(Expr::Literal {
            value: to_literal(&value),
            span: ExprSpan::point(span),
        });
    }

    fn apply_op(
        &mut self,
        _op_lookup: &OpLookup,
        name: &str,
        arity: usize,
        start: Span,
        end: Span,
    ) -> crate::Result<()> {
        let span = ExprSpan { start, end };
        if arity == 0 {
            self.values.push(Expr::Ident {
                name: name.to_string(),
                span,
            });
        } else if name == "()" {
            let mut operands = self.pop_n(arity); // [callee, arg1, ...]
            let callee = operands.remove(0);
            self.values.push(Expr::Apply {
                callee: Box::new(callee),
                args: operands,
                span,
            });
        } else {
            let operands = self.pop_n(arity); // arity 1 = prefix, 2 = infix
            self.values.push(Expr::Op {
                name: name.to_string(),
                operands,
                span,
            });
        }
        Ok(())
    }

    fn apply_logical(
        &mut self,
        name: &str,
        mut rhs: Self,
        start: Span,
        end: Span,
    ) -> crate::Result<()> {
        let op = match name {
            "||" => LogicalOp::Or,
            "&&" => LogicalOp::And,
            other => unreachable!("apply_logical called with unsupported operator `{other}`"),
        };
        let lhs = self.pop();
        debug_assert_eq!(
            rhs.values.len(),
            1,
            "rhs fragment produces exactly one value"
        );
        let rhs_expr = rhs.pop();
        self.values.push(Expr::Logical {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs_expr),
            span: ExprSpan { start, end },
        });
        Ok(())
    }

    fn join2(
        &mut self,
        mut then_fragment: Self,
        mut else_fragment: Self,
        start: Span,
        end: Span,
    ) -> anyhow::Result<()> {
        let cond = self.pop();
        debug_assert_eq!(
            then_fragment.values.len(),
            1,
            "then fragment produces exactly one value"
        );
        debug_assert_eq!(
            else_fragment.values.len(),
            1,
            "else fragment produces exactly one value"
        );
        let then_branch = then_fragment.pop();
        let else_branch = else_fragment.pop();
        self.values.push(Expr::If {
            cond: Box::new(cond),
            then_branch: Box::new(then_branch),
            else_branch: Box::new(else_branch),
            span: ExprSpan { start, end },
        });
        Ok(())
    }

    fn make_tuple(&mut self, n: usize, ambient_start: usize, start: Span, end: Span) {
        let elements = self.values.split_off(ambient_start);
        debug_assert_eq!(
            elements.len(),
            n,
            "make_tuple splits off exactly n elements"
        );
        self.values.push(Expr::Tuple {
            elements,
            span: ExprSpan { start, end },
        });
    }

    fn peek_tuple_arity(&self) -> Option<usize> {
        // No static type info is available during parsing (full type inference is a later,
        // separate phase), so a real arity can't be reported. usize::MAX makes
        // `apply_tuple_index`'s `index < arity` check in lib.rs always pass, so `.N` is always
        // recorded rather than rejected at parse time — e.g. `some_call().0` must still produce
        // an AST even though whether the call actually returns a tuple isn't known here.
        Some(usize::MAX)
    }

    fn tuple_index(&mut self, index: usize, start: Span, end: Span) {
        let base = self.pop();
        self.values.push(Expr::TupleIndex {
            base: Box::new(base),
            index,
            span: ExprSpan { start, end },
        });
    }

    fn current_stack_offset(&self) -> usize {
        self.values.len()
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

    use crate::op_table::OpLookup;
    use crate::parser_context::ParserContext;

    #[test]
    fn push_literal_dispatches_every_concrete_literal_type() {
        fn literal_of<T: 'static + Clone>(value: T) -> Literal {
            let mut ctx = AstContext::new_context();
            ctx.push_literal(value, Span::call_site());
            match ctx.into_expr() {
                Expr::Literal { value, .. } => value,
                other => panic!("expected Literal, got {other:?}"),
            }
        }
        assert_eq!(literal_of(1i8), Literal::I8(1));
        assert_eq!(literal_of(1i16), Literal::I16(1));
        assert_eq!(literal_of(1i32), Literal::I32(1));
        assert_eq!(literal_of(1i64), Literal::I64(1));
        assert_eq!(literal_of(1i128), Literal::I128(1));
        assert_eq!(literal_of(1isize), Literal::Isize(1));
        assert_eq!(literal_of(1u8), Literal::U8(1));
        assert_eq!(literal_of(1u16), Literal::U16(1));
        assert_eq!(literal_of(1u32), Literal::U32(1));
        assert_eq!(literal_of(1u64), Literal::U64(1));
        assert_eq!(literal_of(1u128), Literal::U128(1));
        assert_eq!(literal_of(1usize), Literal::Usize(1));
        assert_eq!(literal_of(1.0f32), Literal::F32(1.0));
        assert_eq!(literal_of(1.0f64), Literal::F64(1.0));
        assert_eq!(literal_of(true), Literal::Bool(true));
        assert_eq!(literal_of('a'), Literal::Char('a'));
        assert_eq!(literal_of("s".to_string()), Literal::Str("s".to_string()));
        assert_eq!(literal_of(vec![1u8, 2u8]), Literal::ByteStr(vec![1, 2]));
        assert_eq!(
            literal_of(CString::new("c").unwrap()),
            Literal::CStr(CString::new("c").unwrap())
        );
        assert_eq!(literal_of(()), Literal::Unit);
    }

    #[test]
    fn apply_op_with_arity_zero_records_an_ident_node() {
        let mut ctx = AstContext::new_context();
        let lookup = OpLookup::new();
        ctx.apply_op(&lookup, "x", 0, Span::call_site(), Span::call_site())
            .unwrap();
        assert!(matches!(ctx.into_expr(), Expr::Ident { name, .. } if name == "x"));
    }

    #[test]
    fn apply_op_with_the_call_operator_records_an_apply_node() {
        let mut ctx = AstContext::new_context();
        let lookup = OpLookup::new();
        ctx.apply_op(&lookup, "x", 0, Span::call_site(), Span::call_site())
            .unwrap(); // callee
        ctx.push_literal(1i32, Span::call_site()); // arg
        ctx.apply_op(&lookup, "()", 2, Span::call_site(), Span::call_site())
            .unwrap();
        match ctx.into_expr() {
            Expr::Apply { callee, args, .. } => {
                assert!(matches!(*callee, Expr::Ident { ref name, .. } if name == "x"));
                assert_eq!(args.len(), 1);
            }
            other => panic!("expected Apply, got {other:?}"),
        }
    }

    #[test]
    fn apply_op_with_arity_two_records_an_op_node() {
        let mut ctx = AstContext::new_context();
        let lookup = OpLookup::new();
        ctx.push_literal(1i32, Span::call_site());
        ctx.push_literal(2i32, Span::call_site());
        ctx.apply_op(&lookup, "+", 2, Span::call_site(), Span::call_site())
            .unwrap();
        match ctx.into_expr() {
            Expr::Op { name, operands, .. } => {
                assert_eq!(name, "+");
                assert_eq!(operands.len(), 2);
            }
            other => panic!("expected Op, got {other:?}"),
        }
    }

    #[test]
    fn apply_logical_records_a_logical_node() {
        let mut ctx = AstContext::new_context();
        ctx.push_literal(true, Span::call_site());
        let mut rhs = ctx.new_fragment();
        rhs.push_literal(false, Span::call_site());
        ctx.apply_logical("||", rhs, Span::call_site(), Span::call_site())
            .unwrap();
        match ctx.into_expr() {
            Expr::Logical { op, lhs, rhs, .. } => {
                assert_eq!(op, LogicalOp::Or);
                assert!(matches!(
                    *lhs,
                    Expr::Literal {
                        value: Literal::Bool(true),
                        ..
                    }
                ));
                assert!(matches!(
                    *rhs,
                    Expr::Literal {
                        value: Literal::Bool(false),
                        ..
                    }
                ));
            }
            other => panic!("expected Logical, got {other:?}"),
        }
    }

    #[test]
    fn join2_records_an_if_node() {
        let mut ctx = AstContext::new_context();
        ctx.push_literal(true, Span::call_site());
        let mut then_fragment = ctx.new_fragment();
        then_fragment.push_literal(1i32, Span::call_site());
        let mut else_fragment = ctx.new_fragment();
        else_fragment.push_literal(2i32, Span::call_site());
        ctx.join2(
            then_fragment,
            else_fragment,
            Span::call_site(),
            Span::call_site(),
        )
        .unwrap();
        match ctx.into_expr() {
            Expr::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                assert!(matches!(
                    *cond,
                    Expr::Literal {
                        value: Literal::Bool(true),
                        ..
                    }
                ));
                assert!(matches!(
                    *then_branch,
                    Expr::Literal {
                        value: Literal::I32(1),
                        ..
                    }
                ));
                assert!(matches!(
                    *else_branch,
                    Expr::Literal {
                        value: Literal::I32(2),
                        ..
                    }
                ));
            }
            other => panic!("expected If, got {other:?}"),
        }
    }

    #[test]
    fn make_tuple_and_tuple_index_roundtrip() {
        let mut ctx = AstContext::new_context();
        let ambient_start = ctx.current_stack_offset();
        ctx.push_literal(1i32, Span::call_site());
        ctx.push_literal(2i32, Span::call_site());
        ctx.make_tuple(2, ambient_start, Span::call_site(), Span::call_site());
        assert_eq!(ctx.peek_tuple_arity(), Some(usize::MAX));
        ctx.tuple_index(1, Span::call_site(), Span::call_site());
        match ctx.into_expr() {
            Expr::TupleIndex { base, index, .. } => {
                assert_eq!(index, 1);
                assert!(matches!(*base, Expr::Tuple { ref elements, .. } if elements.len() == 2));
            }
            other => panic!("expected TupleIndex, got {other:?}"),
        }
    }

    #[test]
    fn peek_tuple_arity_is_always_some_since_types_are_unresolved_during_parsing() {
        let mut ctx = AstContext::new_context();
        ctx.push_literal(5i32, Span::call_site());
        assert_eq!(ctx.peek_tuple_arity(), Some(usize::MAX));
    }
}
