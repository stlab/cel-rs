//! `ParserContext`: the pluggable target a CEL grammar production emits into.
//!
//! The recursive-descent grammar in `lib.rs` is generic over `C: ParserContext` so the same
//! grammar can drive different backends without duplicating it. [`DynSegmentContext`] is the
//! first implementation: it reproduces exactly what `CELParser` did before this trait existed,
//! wrapping a [`DynSegment`] one-for-one. A future AST-building context (for the language
//! server, formatter, and eventual macro-compilation backend) is expected to be the second.

use cel_runtime::DynSegment;
use proc_macro2::Span;

use crate::op_table::OpLookup;

/// The pluggable target a grammar production emits into.
///
/// Each method mirrors one operation the grammar in `lib.rs` needs. Implementations decide what
/// "emitting" means: [`DynSegmentContext`] executes immediately into a stack machine; a future
/// AST-building context would instead record a tree node.
pub trait ParserContext: Sized {
    /// Creates a fresh, empty context with no operations recorded yet.
    fn new_context() -> Self;

    /// Creates an empty fragment for building an alternate branch (one side of a
    /// short-circuiting `||`/`&&`, or an `if`/`else` branch), independent of `self`.
    ///
    /// - Precondition: `self` matches whatever precondition the implementation's equivalent of
    ///   `DynSegment::new_fragment` requires (for `DynSegmentContext`, a condition value already
    ///   present).
    fn new_fragment(&self) -> Self;

    /// Pushes a literal value with the source span of the token it came from.
    fn push_literal<T: 'static + Clone>(&mut self, value: T, span: Span);

    /// Applies a named operator or zero-arity identifier lookup, using `op_lookup` to resolve it
    /// against whatever this context currently holds.
    ///
    /// # Errors
    ///
    /// Returns `Err` if `op_lookup` cannot resolve `name` for `arity` operands.
    fn apply_op(
        &mut self,
        op_lookup: &OpLookup,
        name: &str,
        arity: usize,
        start: Span,
        end: Span,
    ) -> crate::Result<()>;

    /// Applies a short-circuiting logical operator (`"||"` or `"&&"`), consuming a leading
    /// condition value already present on `self` and folding in `rhs`, the already-parsed
    /// right-hand-side fragment.
    ///
    /// - Precondition: `name` is `"||"` or `"&&"`, and `rhs` produces exactly one value.
    ///
    /// # Errors
    ///
    /// Implementations that validate operand types during parsing (e.g. [`DynSegmentContext`])
    /// return `Err` if the leading condition value isn't a `bool`. Implementations that defer
    /// type validation to a later phase (e.g. [`crate::ast::AstContext`]) never return `Err`
    /// here.
    fn apply_logical(&mut self, name: &str, rhs: Self, start: Span, end: Span)
    -> crate::Result<()>;

    /// Joins a previously-built fragment into `self`, consuming a leading condition value already
    /// present on `self`. `then_fragment`'s contribution is used when the condition is `true`;
    /// `else_fragment`'s when `false`, or `None` if the source had no `else`/`else if` at all.
    /// `start`/`end` cover the whole `if`/`else` construct.
    ///
    /// - Precondition: neither fragment takes arguments, and each produces exactly one value.
    ///
    /// # Errors
    ///
    /// Implementations that validate operand types during parsing (e.g. [`DynSegmentContext`])
    /// return `Err` if the leading condition value isn't a `bool` or if the fragments' produced
    /// types don't match — including when `else_fragment` is `None`, which such implementations
    /// treat as an implicit `()` fragment (so a non-`()` then-branch with no `else` is still an
    /// error). Implementations that defer type validation to a later phase (e.g.
    /// [`crate::ast::AstContext`]) never return `Err` here, and record whether `else_fragment` was
    /// `None` directly (see [`crate::Expr::If`]) instead of synthesizing anything.
    fn join2(
        &mut self,
        then_fragment: Self,
        else_fragment: Option<Self>,
        start: Span,
        end: Span,
    ) -> anyhow::Result<()>;

    /// Combines the last `n` emitted values into a single tuple value. `start`/`end` cover the
    /// whole `(...)` construct.
    fn make_tuple(&mut self, n: usize, ambient_start: usize, start: Span, end: Span);

    /// Returns the arity of the tuple currently on top, or `None` if the top value isn't a
    /// tuple.
    fn peek_tuple_arity(&self) -> Option<usize>;

    /// Replaces the tuple on top with its `index`-th element. `start`/`end` cover the base
    /// expression through the index token.
    ///
    /// - Precondition: `peek_tuple_arity()` returns `Some(arity)` with `index < arity`.
    fn tuple_index(&mut self, index: usize, start: Span, end: Span);

    /// Returns the current stack offset, used to compute tuple layouts.
    fn current_stack_offset(&self) -> usize;

    /// Applies a cast (`expr as Type`), consuming the operand already present on `self`
    /// and replacing it with the converted value. `type_name` is the target type's bare
    /// identifier text (e.g. `"i32"`, `"bool"`, `"String"`), unresolved by the grammar itself.
    ///
    /// # Errors
    ///
    /// Implementations that validate operand types during parsing (e.g. [`DynSegmentContext`])
    /// return `Err` if `type_name` isn't a recognized cast-target type, or if no conversion
    /// from the operand's current type to it is registered. Implementations that defer type
    /// validation to a later phase (e.g. [`crate::ast::AstContext`]) never return `Err` here.
    fn apply_cast(
        &mut self,
        op_lookup: &OpLookup,
        type_name: &str,
        start: Span,
        end: Span,
    ) -> crate::Result<()>;

    /// Packages a fully-parsed, independent nested context — the body of a closure literal — as a
    /// value pushed onto `self`, given the closure's declared parameter/return types.
    ///
    /// The default implementation reports closures as unsupported, so a `ParserContext`
    /// implementation that has no use for them (e.g. an AST-building context for the formatter or
    /// language server) needs no changes to keep compiling.
    ///
    /// - Precondition: `body` was built via `Self::new_context()` and its own argument-binding
    ///   mechanism, in the same style [`Self::new_context`]'s other consumers already use.
    ///
    /// # Errors
    ///
    /// Returns `Err` if this `ParserContext` implementation doesn't support closures.
    fn push_closure(
        &mut self,
        param_types: Vec<std::any::TypeId>,
        return_type: std::any::TypeId,
        body: Self,
        span: Span,
    ) -> crate::Result<()> {
        let _ = (param_types, return_type, body);
        Err(crate::ParseError::new_range(
            "closures are not supported in this context".to_string(),
            span,
            span,
        ))
    }

    /// Returns the `TypeId` of the single value `self` currently holds, or `None` if that isn't
    /// known/applicable for this `ParserContext` implementation.
    ///
    /// Used by the closure-literal grammar production to infer a closure's return type from its
    /// already-compiled body, mirroring [`Self::peek_tuple_arity`]'s "ask the context what it
    /// currently holds" shape, generalized from "is it a tuple" to "what type is it".
    ///
    /// The default implementation returns `None`, matching [`Self::push_closure`]'s default
    /// "unsupported" behavior — an implementation with no notion of a parse-time runtime type
    /// (e.g. an AST-building context) simply can't answer this, which surfaces to the caller the
    /// same way an empty or multi-valued body does (a plain parse error), and in practice never
    /// matters for such an implementation anyway, since its `push_closure` override (if any)
    /// would reject the closure regardless.
    fn output_type_id(&self) -> Option<std::any::TypeId> {
        None
    }
}

/// [`ParserContext`] implementation that executes directly into a [`DynSegment`], reproducing
/// the runtime-execution behavior `CELParser` always had before this trait existed.
///
/// # Examples
///
/// ```rust
/// use cel_parser::parser_context::{DynSegmentContext, ParserContext};
/// use proc_macro2::Span;
///
/// let mut ctx = DynSegmentContext::new_context();
/// ctx.push_literal(10i32, Span::call_site());
/// ```
pub struct DynSegmentContext(pub(crate) DynSegment);

impl DynSegmentContext {
    /// Returns the wrapped [`DynSegment`], consuming `self`.
    pub fn into_inner(self) -> DynSegment {
        self.0
    }
}

impl std::ops::Deref for DynSegmentContext {
    type Target = DynSegment;

    fn deref(&self) -> &DynSegment {
        &self.0
    }
}

impl std::ops::DerefMut for DynSegmentContext {
    fn deref_mut(&mut self) -> &mut DynSegment {
        &mut self.0
    }
}

impl ParserContext for DynSegmentContext {
    fn new_context() -> Self {
        DynSegmentContext(DynSegment::new::<()>())
    }

    fn new_fragment(&self) -> Self {
        DynSegmentContext(self.0.new_fragment())
    }

    fn push_literal<T: 'static + Clone>(&mut self, value: T, _span: Span) {
        self.0.just(value);
    }

    fn apply_op(
        &mut self,
        op_lookup: &OpLookup,
        name: &str,
        arity: usize,
        start: Span,
        end: Span,
    ) -> crate::Result<()> {
        op_lookup.lookup(name, &mut self.0, arity, start, end)
    }

    fn apply_logical(
        &mut self,
        name: &str,
        rhs: Self,
        start: Span,
        end: Span,
    ) -> crate::Result<()> {
        let mut bypass = self.new_fragment();
        let result = match name {
            "||" => {
                bypass.0.just(true);
                self.0.join2(bypass.0, rhs.0)
            }
            "&&" => {
                bypass.0.just(false);
                self.0.join2(rhs.0, bypass.0)
            }
            other => unreachable!("apply_logical called with unsupported operator `{other}`"),
        };
        result.map_err(|e| crate::ParseError::new_range(e.to_string(), start, end))
    }

    fn join2(
        &mut self,
        then_fragment: Self,
        else_fragment: Option<Self>,
        start: Span,
        _end: Span,
    ) -> anyhow::Result<()> {
        // No `else`/`else if` in the source: synthesize the implicit `()` fragment here instead
        // of receiving one pre-built from `is_if_expression` (lib.rs) — execution still needs a
        // concrete fragment to select on `false`, even though `AstContext::join2` (ast.rs)
        // records this case as `None` rather than a synthesized node.
        let else_fragment = else_fragment.unwrap_or_else(|| {
            let mut fragment = self.new_fragment();
            fragment.push_literal((), start);
            fragment
        });
        self.0.join2(then_fragment.0, else_fragment.0)
    }

    fn make_tuple(&mut self, n: usize, ambient_start: usize, _start: Span, _end: Span) {
        self.0.make_tuple(n, ambient_start);
    }

    fn peek_tuple_arity(&self) -> Option<usize> {
        self.0.peek_tuple_arity()
    }

    fn tuple_index(&mut self, index: usize, _start: Span, _end: Span) {
        self.0.tuple_index(index);
    }

    fn current_stack_offset(&self) -> usize {
        self.0.current_stack_offset()
    }

    fn apply_cast(
        &mut self,
        op_lookup: &OpLookup,
        type_name: &str,
        start: Span,
        end: Span,
    ) -> crate::Result<()> {
        op_lookup.lookup_cast(type_name, &mut self.0, start, end)
    }

    fn push_closure(
        &mut self,
        param_types: Vec<std::any::TypeId>,
        return_type: std::any::TypeId,
        body: Self,
        span: Span,
    ) -> crate::Result<()> {
        self.0.just(cel_runtime::DynClosure::new(
            param_types,
            return_type,
            body.into_inner(),
        ));
        let _ = span;
        Ok(())
    }

    fn output_type_id(&self) -> Option<std::any::TypeId> {
        self.0.peek_output_type_id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op_table::OpLookup;
    use proc_macro2::Span;
    use std::any::TypeId;

    #[test]
    fn new_context_is_empty_and_ready_for_literals() {
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(10i32, Span::call_site());
        assert_eq!(ctx.into_inner().call0::<i32>().unwrap(), 10);
    }

    #[test]
    fn apply_op_dispatches_builtin_addition() {
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(10i32, Span::call_site());
        ctx.push_literal(20i32, Span::call_site());
        let lookup = OpLookup::new();
        ctx.apply_op(&lookup, "+", 2, Span::call_site(), Span::call_site())
            .unwrap();
        assert_eq!(ctx.into_inner().call0::<i32>().unwrap(), 30);
    }

    #[test]
    fn apply_op_propagates_lookup_error() {
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(10i32, Span::call_site());
        ctx.push_literal("hi".to_string(), Span::call_site());
        let lookup = OpLookup::new();
        let err = ctx
            .apply_op(&lookup, "+", 2, Span::call_site(), Span::call_site())
            .expect_err("mismatched operand types must fail");
        assert!(err.message().starts_with("no operation"));
    }

    #[test]
    fn make_tuple_and_tuple_index_roundtrip() {
        let mut ctx = DynSegmentContext::new_context();
        let ambient_start = ctx.current_stack_offset();
        ctx.push_literal(1i32, Span::call_site());
        ctx.push_literal(2i32, Span::call_site());
        ctx.make_tuple(2, ambient_start, Span::call_site(), Span::call_site());
        assert_eq!(ctx.peek_tuple_arity(), Some(2));
        ctx.tuple_index(1, Span::call_site(), Span::call_site());
        assert_eq!(ctx.into_inner().call0::<i32>().unwrap(), 2);
    }

    #[test]
    fn peek_tuple_arity_is_none_for_non_tuple() {
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(5i32, Span::call_site());
        assert_eq!(ctx.peek_tuple_arity(), None);
    }

    #[test]
    fn join2_selects_then_fragment_when_condition_true() {
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(true, Span::call_site());
        let mut then_fragment = ctx.new_fragment();
        then_fragment.push_literal(1i32, Span::call_site());
        let mut else_fragment = ctx.new_fragment();
        else_fragment.push_literal(2i32, Span::call_site());
        ctx.join2(
            then_fragment,
            Some(else_fragment),
            Span::call_site(),
            Span::call_site(),
        )
        .unwrap();
        assert_eq!(ctx.into_inner().call0::<i32>().unwrap(), 1);
    }

    #[test]
    fn join2_selects_else_fragment_when_condition_false() {
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(false, Span::call_site());
        let mut then_fragment = ctx.new_fragment();
        then_fragment.push_literal(1i32, Span::call_site());
        let mut else_fragment = ctx.new_fragment();
        else_fragment.push_literal(2i32, Span::call_site());
        ctx.join2(
            then_fragment,
            Some(else_fragment),
            Span::call_site(),
            Span::call_site(),
        )
        .unwrap();
        assert_eq!(ctx.into_inner().call0::<i32>().unwrap(), 2);
    }

    #[test]
    fn join2_with_none_synthesizes_an_implicit_unit_else_fragment() {
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(false, Span::call_site());
        let mut then_fragment = ctx.new_fragment();
        then_fragment.push_literal((), Span::call_site());
        ctx.join2(then_fragment, None, Span::call_site(), Span::call_site())
            .unwrap();
        ctx.into_inner().call0::<()>().unwrap();
    }

    #[test]
    fn deref_gives_transparent_access_to_dyn_segment_methods() {
        // Proves DynSegmentContext doesn't need `.into_inner()` for read-only DynSegment
        // methods not part of ParserContext itself (e.g. peek_output_type_id).
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(7i32, Span::call_site());
        assert_eq!(
            ctx.peek_output_type_id(),
            Some(std::any::TypeId::of::<i32>())
        );
    }

    #[test]
    fn apply_logical_or_short_circuits_to_lhs_when_true() {
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(true, Span::call_site());
        let mut rhs = ctx.new_fragment();
        rhs.push_literal(false, Span::call_site());
        ctx.apply_logical("||", rhs, Span::call_site(), Span::call_site())
            .unwrap();
        assert!(ctx.into_inner().call0::<bool>().unwrap());
    }

    #[test]
    fn apply_logical_and_short_circuits_to_false_when_lhs_false() {
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(false, Span::call_site());
        let mut rhs = ctx.new_fragment();
        rhs.push_literal(true, Span::call_site());
        ctx.apply_logical("&&", rhs, Span::call_site(), Span::call_site())
            .unwrap();
        assert!(!ctx.into_inner().call0::<bool>().unwrap());
    }

    #[test]
    fn dyn_segment_context_push_closure_builds_a_callable_closure() {
        let mut outer = DynSegmentContext::new_context();
        let mut body = DynSegmentContext::new_context();
        body.0.push_arg::<i32>(0);
        body.0.op1(|x: i32| x + 1).unwrap();

        outer
            .push_closure(
                vec![TypeId::of::<i32>()],
                TypeId::of::<i32>(),
                body,
                Span::call_site(),
            )
            .unwrap();

        let closure: cel_runtime::DynClosure = outer.into_inner().call0().unwrap();
        let x = 5i32;
        assert_eq!(closure.call::<i32>(&[&x]).unwrap(), 6);
    }
}
