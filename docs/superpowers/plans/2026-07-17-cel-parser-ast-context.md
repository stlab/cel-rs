# cel-parser `AstContext` (Phase 2a) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a span-carrying CEL expression AST (`cel_parser::ast::Expr`) and `AstContext`, a second implementation of the existing `ParserContext` trait that builds this tree instead of executing — entirely within `cel-parser`, with zero behavior change to the existing `DynSegmentContext`/`CELParser` execution path.

**Architecture:** `ParserContext` gains span parameters on its four node-building methods (`push_literal`, `make_tuple`, `tuple_index`, `join2`) and one new method (`apply_logical`, which replaces the `&&`/`||` desugaring that currently lives inline in the grammar). `DynSegmentContext`'s implementations of the changed methods are trivial (ignore the new spans; `apply_logical` reproduces today's short-circuit behavior verbatim). `AstContext` is a new implementation, in a new `cel-parser/src/ast.rs` module, that maintains its own flat `Vec<Expr>` value stack (mirroring the physical stack `DynSegment` gives `DynSegmentContext` for free) and never consults `OpLookup` or fails on semantic grounds — every operator/identifier/tuple-index/if node is recorded structurally, deferring all resolution and type/range validation to a later, separate phase.

**Tech Stack:** Rust, existing `cel-parser`/`cel-runtime` crates. No new dependencies (the literal-payload enum and its `TypeId`-based dispatch use only `std::any::Any`).

## Global Constraints

- `cargo fmt --all` before every commit (enforced by pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` must pass (this plan
  never touches `begin`, so its two `begin`-specific clippy invocations aren't relevant here, but
  running them costs nothing and catches surprises).
- Never commit directly to `main`; this work happens on the current worktree branch.
- Doc comments follow the project's contract style (Summary / Preconditions / Postconditions /
  Complexity / `# Errors` / `# Examples` for public APIs) — see `CLAUDE.md`.
- Prefer concrete/generic types over `Box<dyn Trait>` when the type set is statically known (it
  is here, for the literal-payload enum) — see `CLAUDE.md`'s "Avoid heap allocations" section.
- Every existing `cel-parser` test, and every existing test in every crate that depends on it
  (`pm-lang`, `cel-rs-macros`, `begin`), must keep passing **completely unchanged**, with one
  documented exception: the 8 `DynSegmentContext` unit tests in `parser_context.rs` (added in the
  prior phase) call the trait methods directly and must gain `Span::call_site()` arguments for the
  new span parameters — this is a mechanical signature update, not a behavior change, and is the
  only test edit this plan makes to already-passing tests.
- `AstContext` never consults `OpLookup` and never returns `Err` for semantic reasons (unresolved
  operator, non-tuple `.N` base, out-of-range tuple index, branch type mismatch) — only a genuine
  syntax error (missing token) can fail parsing through `AstContext`. This is a deliberate,
  documented behavior *difference* from `DynSegmentContext` for inputs that are syntactically
  valid but semantically invalid (e.g. `(1, 2).5`), not a regression of any existing test (those
  all go through `DynSegmentContext`).

---

### Task 1: Widen `ParserContext` with spans and `apply_logical`

**Files:**
- Modify: `cel-parser/src/parser_context.rs` (trait signatures, `DynSegmentContext` impl, its 8
  existing tests, 2 new tests)
- Modify: `cel-parser/src/lib.rs` (every grammar-production call site of the changed methods)

**Interfaces:**
- Produces (used by Task 3): the widened `ParserContext` trait —
  `push_literal<T: 'static + Clone>(&mut self, value: T, span: Span)`,
  `make_tuple(&mut self, n: usize, ambient_start: usize, start: Span, end: Span)`,
  `tuple_index(&mut self, index: usize, start: Span, end: Span)`,
  `join2(&mut self, then_fragment: Self, else_fragment: Self, start: Span, end: Span) -> anyhow::Result<()>`,
  `apply_logical(&mut self, name: &str, rhs: Self, start: Span, end: Span) -> crate::Result<()>`
  (new). `apply_op`, `new_context`, `new_fragment`, `peek_tuple_arity`, `current_stack_offset` are
  unchanged.

This task is a pure, behavior-preserving refactor — it introduces no new public type and changes
no observable behavior of `CELParser`/`DynSegmentContext`. It only widens the trait surface so
Task 3's `AstContext` (which has no runtime values to inspect for span/type information) can build
correct span-carrying nodes. Every existing `cel-parser` test (other than the 8 called out above)
must pass with **zero changes**.

- [ ] **Step 1: Update the `ParserContext` trait definition**

In `cel-parser/src/parser_context.rs`, replace the trait definition (currently lines 19–74) with:

```rust
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
    /// - Precondition: `name` is `"||"` or `"&&"`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the leading condition value isn't a `bool`, or if `rhs` doesn't produce
    /// exactly one value.
    fn apply_logical(&mut self, name: &str, rhs: Self, start: Span, end: Span) -> crate::Result<()>;

    /// Joins two previously-built fragments into `self`, consuming a leading condition value
    /// already present on `self`. `then_fragment`'s contribution is used when the condition is
    /// `true`; `else_fragment`'s when `false`. `start`/`end` cover the whole `if`/`else`
    /// construct.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the leading condition value isn't a `bool`, if either fragment takes
    /// arguments, if either fragment doesn't produce exactly one value, or if the fragments'
    /// produced types don't match.
    fn join2(
        &mut self,
        then_fragment: Self,
        else_fragment: Self,
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
}
```

- [ ] **Step 2: Update `DynSegmentContext`'s `impl ParserContext`**

In the same file, replace the `impl ParserContext for DynSegmentContext` block (currently lines
110–153) with:

```rust
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
        else_fragment: Self,
        _start: Span,
        _end: Span,
    ) -> anyhow::Result<()> {
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
}
```

This reproduces today's short-circuit behavior exactly: `"||"` still builds `if cond { true }
else { rhs }` (bypass = `true`, `join2(bypass, rhs)`), `"&&"` still builds `if cond { rhs } else
{ false }` (`join2(rhs, bypass)` with bypass = `false`), and the resulting error still carries a
range span via `ParseError::new_range`, exactly matching what the grammar productions built
inline before this task.

- [ ] **Step 3: Update the existing `DynSegmentContext` tests and add 2 new ones for `apply_logical`**

In the same file, replace the `#[cfg(test)] mod tests { ... }` block (currently lines 155–245)
with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::op_table::OpLookup;
    use proc_macro2::Span;

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
            else_fragment,
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
            else_fragment,
            Span::call_site(),
            Span::call_site(),
        )
        .unwrap();
        assert_eq!(ctx.into_inner().call0::<i32>().unwrap(), 2);
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
}
```

- [ ] **Step 4: Run the `parser_context` tests to confirm they pass**

Run: `cargo test -p cel-parser parser_context`
Expected: 10 tests pass (the 8 existing ones, now taking span arguments, plus the 2 new
`apply_logical` tests).

- [ ] **Step 5: Update every grammar call site in `cel-parser/src/lib.rs`**

Each edit below is independent; make all of them (the crate will not compile with only some
applied, since the trait signatures already changed in Step 1).

**5a. `push_literal_token`** — find the whole function (currently lines 108–240, starting
`fn push_literal_token<C: ParserContext>(output: &mut C, lit: CelLiteral) -> Result<()> {`) and
replace it with:

```rust
/// Pushes a literal value from `token` onto `output`.
///
/// # Errors
///
/// Returns `Err` if the literal type is unsupported or if a suffixed numeric
/// literal cannot be parsed.
fn push_literal_token<C: ParserContext>(output: &mut C, lit: CelLiteral) -> Result<()> {
    match lit {
        CelLiteral::Int(integer) => {
            let span = integer.span();
            match integer.suffix() {
                "" | "i32" => output.push_literal(
                    integer.base10_parse::<i32>().map_err(|e| {
                        ParseError::new(
                            format!("invalid i32 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "u8" => output.push_literal(
                    integer.base10_parse::<u8>().map_err(|e| {
                        ParseError::new(
                            format!("invalid u8 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "u16" => output.push_literal(
                    integer.base10_parse::<u16>().map_err(|e| {
                        ParseError::new(
                            format!("invalid u16 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "u32" => output.push_literal(
                    integer.base10_parse::<u32>().map_err(|e| {
                        ParseError::new(
                            format!("invalid u32 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "u64" => output.push_literal(
                    integer.base10_parse::<u64>().map_err(|e| {
                        ParseError::new(
                            format!("invalid u64 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "u128" => output.push_literal(
                    integer.base10_parse::<u128>().map_err(|e| {
                        ParseError::new(
                            format!("invalid u128 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "usize" => output.push_literal(
                    integer.base10_parse::<usize>().map_err(|e| {
                        ParseError::new(
                            format!("invalid usize literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "i8" => output.push_literal(
                    integer.base10_parse::<i8>().map_err(|e| {
                        ParseError::new(
                            format!("invalid i8 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "i16" => output.push_literal(
                    integer.base10_parse::<i16>().map_err(|e| {
                        ParseError::new(
                            format!("invalid i16 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "i64" => output.push_literal(
                    integer.base10_parse::<i64>().map_err(|e| {
                        ParseError::new(
                            format!("invalid i64 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "i128" => output.push_literal(
                    integer.base10_parse::<i128>().map_err(|e| {
                        ParseError::new(
                            format!("invalid i128 literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                "isize" => output.push_literal(
                    integer.base10_parse::<isize>().map_err(|e| {
                        ParseError::new(
                            format!("invalid isize literal `{integer}`: {e}"),
                            integer.span(),
                        )
                    })?,
                    span,
                ),
                suffix => {
                    return Err(ParseError::new(
                        format!("invalid integer literal suffix: `{suffix}`"),
                        integer.span(),
                    ));
                }
            };
        }
        CelLiteral::Float(float) => {
            let span = float.span();
            match float.suffix() {
                "" | "f64" => output.push_literal(
                    float.base10_parse::<f64>().map_err(|e| {
                        ParseError::new(format!("invalid f64 literal `{float}`: {e}"), float.span())
                    })?,
                    span,
                ),
                "f32" => output.push_literal(
                    float.base10_parse::<f32>().map_err(|e| {
                        ParseError::new(format!("invalid f32 literal `{float}`: {e}"), float.span())
                    })?,
                    span,
                ),
                suffix => {
                    return Err(ParseError::new(
                        format!("invalid float literal suffix: `{suffix}`"),
                        float.span(),
                    ));
                }
            };
        }
        CelLiteral::Str(string) => {
            let span = string.span();
            output.push_literal(string.value(), span);
        }
        CelLiteral::Bool(lit_bool) => {
            let span = lit_bool.span();
            output.push_literal(lit_bool.value, span);
        }
        CelLiteral::Char(ch) => {
            let span = ch.span();
            output.push_literal(ch.value(), span);
        }
        CelLiteral::Byte(byte) => {
            let span = byte.span();
            output.push_literal(byte.value(), span);
        }
        CelLiteral::ByteStr(byte_str) => {
            let span = byte_str.span();
            output.push_literal(byte_str.value(), span);
        }
        CelLiteral::CStr(c_str) => {
            let span = c_str.span();
            output.push_literal(c_str.value(), span);
        }
        other => {
            return Err(ParseError::new(
                format!("unsupported literal: {other:?}"),
                other.span(),
            ));
        }
    }
    Ok(())
}
```

**5b. `is_or_expression`** — find the method (currently lines 508–540) and replace it with:

```rust
    /// `or_expression = and_expression { "||" and_expression }.`
    ///
    /// # Errors
    ///
    /// Returns an error if the RHS is missing after `||`, if the RHS does not
    /// produce a `bool`, or if any sub-expression returns an error.
    fn is_or_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_and_expression()? {
            while self.is_punctuation("||") {
                let mut rhs_fragment = self.context.new_fragment();
                std::mem::swap(&mut self.context, &mut rhs_fragment);
                if !self.is_and_expression()? {
                    return Err(self.error_at("expected and_expression"));
                }
                std::mem::swap(&mut self.context, &mut rhs_fragment);
                self.context.apply_logical(
                    "||",
                    rhs_fragment,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
```

**5c. `is_and_expression`** — find the method (currently lines 542–574) and replace it with:

```rust
    /// `and_expression = comparison_expression { "&&" comparison_expression }.`
    ///
    /// # Errors
    ///
    /// Returns an error if the RHS is missing after `&&`, if the RHS does not
    /// produce a `bool`, or if any sub-expression returns an error.
    fn is_and_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_comparison_expression()? {
            while self.is_punctuation("&&") {
                let mut rhs_fragment = self.context.new_fragment();
                std::mem::swap(&mut self.context, &mut rhs_fragment);
                if !self.is_comparison_expression()? {
                    return Err(self.error_at("expected comparison_expression"));
                }
                std::mem::swap(&mut self.context, &mut rhs_fragment);
                self.context.apply_logical(
                    "&&",
                    rhs_fragment,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
```

**5d. `is_postfix_expression` and `apply_tuple_index`** — find both methods together (currently
lines 814–922, from the doc comment `/// The repetition allows chained indices...` through the
end of `apply_tuple_index`) and replace with:

```rust
    /// The repetition allows chained indices (`t.0.1`): each `"." unsuffixed_integer`
    /// is applied in turn to whatever value the previous step left on top of the
    /// stack. Source text like `.0.1` tokenizes as a single `.` followed by one
    /// float literal `0.1` (Rust's own lexer maximally munches the digits after
    /// the second `.`), so that case is detected and split back into its two
    /// integer indices — see the `Token::Literal(CelLiteral::Float(..))` arm below.
    fn is_postfix_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if !self.is_primary_expression()? {
            return Ok(false);
        }
        loop {
            if matches!(
                self.peek_token(),
                Some(Token::OpenDelim {
                    delimiter: Delimiter::Parenthesis,
                    ..
                })
            ) {
                self.advance(); // consume "("
                let arg_count = self.parameter_list()?;
                match self.peek_token() {
                    Some(Token::CloseDelim {
                        delimiter: Delimiter::Parenthesis,
                        ..
                    }) => {
                        self.advance(); // consume ")"
                    }
                    _ => return Err(self.error_at("expected closing parenthesis")),
                }
                // Stack order is [callee, arg1, arg2, ...]; lookup peeks top (arg_count + 1) entries.
                self.context.apply_op(
                    &self.op_lookup,
                    "()",
                    arg_count + 1,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            } else if self.is_punctuation(".") {
                match self.peek_token() {
                    Some(Token::Literal(CelLiteral::Int(integer))) => {
                        let integer = integer.clone();
                        if !integer.suffix().is_empty() {
                            return Err(self.error_at("tuple index must be an unsuffixed integer"));
                        }
                        self.advance();
                        let index = integer.base10_parse::<usize>().map_err(|e| {
                            self.error_at(&format!("invalid tuple index `{integer}`: {e}"))
                        })?;
                        self.apply_tuple_index(
                            index,
                            start_span.expect("production has token at start"),
                        )?;
                    }
                    Some(Token::Literal(CelLiteral::Float(float))) => {
                        let float = float.clone();
                        if !float.suffix().is_empty() {
                            return Err(self.error_at("tuple index must be an unsuffixed integer"));
                        }
                        // base10_digits() returns the decimal digits with underscores
                        // stripped and the suffix removed, e.g. "0.1" or "10.25" for
                        // ordinary decimal floats — splitting on '.' recovers the two
                        // chained integer indices. Scientific-notation floats (e.g.
                        // `1e2`) normalize to digits with no '.' at all; reject those
                        // as a parse error (checked before advancing, so the error
                        // span still points at the float token) rather than assuming
                        // a '.' is always present.
                        let digits = float.base10_digits();
                        let Some((first, second)) = digits.split_once('.') else {
                            return Err(self.error_at(
                                "tuple index chain must use decimal notation (e.g. `.0.1`)",
                            ));
                        };
                        self.advance();
                        let first_index = first.parse::<usize>().map_err(|e| {
                            self.error_at(&format!("invalid tuple index `{first}`: {e}"))
                        })?;
                        let second_index = second.parse::<usize>().map_err(|e| {
                            self.error_at(&format!("invalid tuple index `{second}`: {e}"))
                        })?;
                        let idx_start = start_span.expect("production has token at start");
                        self.apply_tuple_index(first_index, idx_start)?;
                        self.apply_tuple_index(second_index, idx_start)?;
                    }
                    _ => return Err(self.error_at("expected integer after '.'")),
                }
            } else {
                break;
            }
        }
        Ok(true)
    }

    /// Applies a single `.N` tuple-index operation to the value currently on
    /// top of the stack, replacing it with element `index`. `start` is the span
    /// of the base expression the index chain is rooted at.
    ///
    /// # Errors
    /// Returns an error if the top of stack isn't a tuple, or if `index` is
    /// out of range for its arity.
    fn apply_tuple_index(&mut self, index: usize, start: Span) -> Result<()> {
        let arity = self
            .context
            .peek_tuple_arity()
            .ok_or_else(|| self.error_at("'.N' requires a tuple"))?;
        if index >= arity {
            return Err(self.error_at(&format!(
                "tuple index `{index}` out of range for tuple of arity {arity}"
            )));
        }
        self.context.tuple_index(index, start, self.last_span);
        Ok(())
    }
```

**5e. `is_primary_expression`** — find the line (currently line 967):

```rust
                if ident_name == "if" {
                    return self.is_if_expression();
                }
```

Replace with:

```rust
                if ident_name == "if" {
                    return self.is_if_expression(ident_span);
                }
```

**5f. `is_tuple_or_group`** — find the method (currently lines 983–1060) and replace it with:

```rust
    /// `tuple_or_group = "(" [ or_expression ["," [ or_expression { "," or_expression } ]] ] ")".`
    ///
    /// `()` parses as unit, `(expr)` as grouping, `(expr,)` as a 1-tuple, and
    /// `(expr, expr, ...)` as an n-tuple.
    ///
    /// - Precondition: The next token is `Token::OpenDelim` with `Delimiter::Parenthesis`.
    ///
    /// # Errors
    ///
    /// Returns an error if the parenthesized expression or tuple literal is malformed, has a
    /// missing or misplaced comma, or is missing its closing `)`.
    fn is_tuple_or_group(&mut self) -> Result<bool> {
        let open_span = self
            .peek_span()
            .expect("tuple_or_group requires an opening '(' token");
        self.advance();
        // Unit expression: ()
        if matches!(
            self.peek_token(),
            Some(Token::CloseDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            })
        ) {
            self.advance();
            self.context.push_literal((), self.last_span);
            return Ok(true);
        }
        let ambient_start = self.context.current_stack_offset();
        if !self.is_or_expression()? {
            return Err(self.error_at("expected expression"));
        }
        if matches!(
            self.peek_token(),
            Some(Token::CloseDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            })
        ) {
            // Grouping: exactly one expression, no comma.
            self.advance();
            return Ok(true);
        }
        if !self.is_punctuation(",") {
            return Err(self.error_at("expected ',' or closing parenthesis"));
        }
        let mut count = 1;
        if matches!(
            self.peek_token(),
            Some(Token::CloseDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            })
        ) {
            // Single element + trailing comma: 1-tuple.
            self.advance();
            self.context
                .make_tuple(count, ambient_start, open_span, self.last_span);
            return Ok(true);
        }
        loop {
            if !self.is_or_expression()? {
                return Err(self.error_at("expected expression after ','"));
            }
            count += 1;
            if matches!(
                self.peek_token(),
                Some(Token::CloseDelim {
                    delimiter: Delimiter::Parenthesis,
                    ..
                })
            ) {
                self.advance();
                break;
            }
            if !self.is_punctuation(",") {
                return Err(self.error_at("expected ',' or closing parenthesis"));
            }
        }
        self.context
            .make_tuple(count, ambient_start, open_span, self.last_span);
        Ok(true)
    }
```

**5g. `is_if_expression`** — find the method (currently lines 1062–1147) and replace it with:

```rust
    /// `if_expression = "if" or_expression "{" or_expression "}" [ "else" ( "{" or_expression "}" | if_expression ) ].`
    ///
    /// - Precondition: The `if` keyword has already been consumed by the caller; `if_span` is
    ///   its span.
    ///
    /// # Errors
    ///
    /// Returns an error if the condition is missing, if a `{` or `}` delimiter is missing,
    /// if the then-branch or else-branch expression is missing, or if the then and else
    /// branch types do not match (as detected by `join2`).
    ///
    /// - Postcondition: Returns `Ok(true)` on success; `Ok(false)` is never returned.
    fn is_if_expression(&mut self, if_span: Span) -> Result<bool> {
        if !self.is_or_expression()? {
            return Err(self.error_at("expected condition after `if`"));
        }
        match self.peek_token() {
            Some(Token::OpenDelim {
                delimiter: Delimiter::Brace,
                ..
            }) => {
                self.advance();
            }
            _ => return Err(self.error_at("expected `{` after if condition")),
        }
        let mut then_fragment = self.context.new_fragment();
        std::mem::swap(&mut self.context, &mut then_fragment);
        if !self.is_or_expression()? {
            return Err(self.error_at("expected expression in then-branch"));
        }
        std::mem::swap(&mut self.context, &mut then_fragment);
        match self.peek_token() {
            Some(Token::CloseDelim {
                delimiter: Delimiter::Brace,
                ..
            }) => {
                self.advance();
            }
            _ => return Err(self.error_at("expected `}` after then-branch")),
        }
        let else_fragment = if self.is_keyword("else") {
            if self.is_keyword("if") {
                // else if: recursively parse another if_expression
                let elif_span = self.last_span;
                let mut fragment = self.context.new_fragment();
                std::mem::swap(&mut self.context, &mut fragment);
                self.is_if_expression(elif_span)?;
                std::mem::swap(&mut self.context, &mut fragment);
                fragment
            } else {
                // else { expr }
                match self.peek_token() {
                    Some(Token::OpenDelim {
                        delimiter: Delimiter::Brace,
                        ..
                    }) => {
                        self.advance();
                    }
                    _ => return Err(self.error_at("expected `{` or `if` after `else`")),
                }
                let mut fragment = self.context.new_fragment();
                std::mem::swap(&mut self.context, &mut fragment);
                if !self.is_or_expression()? {
                    return Err(self.error_at("expected expression in else-branch"));
                }
                std::mem::swap(&mut self.context, &mut fragment);
                match self.peek_token() {
                    Some(Token::CloseDelim {
                        delimiter: Delimiter::Brace,
                        ..
                    }) => {
                        self.advance();
                    }
                    _ => return Err(self.error_at("expected `}` after else-branch")),
                }
                fragment
            }
        } else {
            // Implicit else: () — then-branch must also return ()
            let mut fragment = self.context.new_fragment();
            fragment.push_literal((), self.last_span);
            fragment
        };
        self.context
            .join2(then_fragment, else_fragment, if_span, self.last_span)
            .map_err(|e| ParseError::new(e.to_string(), self.last_span))?;
        Ok(true)
    }
```

- [ ] **Step 6: Run the full `cel-parser` test suite to confirm zero regressions**

Run: `cargo test -p cel-parser`
Expected: every test that existed before this task still passes — including `and_lhs_type_error`
and `or_lhs_type_error`, which assert `err.end_span().is_some()` and must still see that (the
`apply_logical` error path uses `ParseError::new_range`, matching the old inline `.map_err` block
exactly). No existing test's assertions need editing.

- [ ] **Step 7: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: every test in every crate passes, including `pm-lang`, `cel-rs-macros`, `property-model`,
and `begin` — this task only touches `cel-parser` internals behind its unchanged public API.

- [ ] **Step 8: Lint and build checks**

Run:
```bash
cargo build --workspace
cargo clippy -p cel-parser --all-targets -- -D warnings
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
```
Expected: zero warnings from all three.

- [ ] **Step 9: Format and commit**

```bash
cargo fmt --all
git add cel-parser/src/parser_context.rs cel-parser/src/lib.rs
git commit -m "$(cat <<'EOF'
refactor(cel-parser): thread spans through ParserContext, add apply_logical

Widens push_literal/make_tuple/tuple_index/join2 with span parameters and
adds apply_logical (replacing the &&/|| desugaring inline in the grammar)
so a future AST-building context can record span-carrying nodes without
touching runtime values. DynSegmentContext's behavior is unchanged.
EOF
)"
```

---

### Task 2: `Expr`/`Literal`/`ExprSpan`/`LogicalOp` AST types

**Files:**
- Create: `cel-parser/src/ast.rs`
- Modify: `cel-parser/src/lib.rs` (add `pub mod ast;` and a re-export — nothing else in this file
  changes in this task)

**Interfaces:**
- Produces (used by Task 3): `pub struct ExprSpan { pub start: Span, pub end: Span }`;
  `pub enum Expr { Literal { value: Literal, span: ExprSpan }, Ident { name: String, span: ExprSpan },
  Op { name: String, operands: Vec<Expr>, span: ExprSpan }, Apply { callee: Box<Expr>, args: Vec<Expr>, span: ExprSpan },
  Tuple { elements: Vec<Expr>, span: ExprSpan }, TupleIndex { base: Box<Expr>, index: usize, span: ExprSpan },
  If { cond: Box<Expr>, then_branch: Box<Expr>, else_branch: Box<Expr>, span: ExprSpan },
  Logical { op: LogicalOp, lhs: Box<Expr>, rhs: Box<Expr>, span: ExprSpan } }` with a
  `pub fn span(&self) -> ExprSpan` method; `pub enum LogicalOp { And, Or }`;
  `pub enum Literal { I8(i8), I16(i16), I32(i32), I64(i64), I128(i128), Isize(isize), U8(u8),
  U16(u16), U32(u32), U64(u64), U128(u128), Usize(usize), F32(f32), F64(f64), Bool(bool),
  Char(char), Str(String), ByteStr(Vec<u8>), CStr(CString), Unit }`.

- [ ] **Step 1: Write the test module for the AST types**

Create `cel-parser/src/ast.rs` with only this content (the types it references don't exist yet —
this is the intended failing state):

```rust
//! A span-carrying CEL expression AST, built by [`AstContext`](crate::ast::AstContext) as an
//! alternative to [`DynSegmentContext`](crate::parser_context::DynSegmentContext)'s direct
//! execution. Consumed as-is by pm-lang (method bodies/initializers), the language server, the
//! formatter, and the future macro-compilation backend. Carries no resolved types or operator
//! overloads: resolution and type/range validation are deferred to a later, separate phase.

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
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p cel-parser ast::tests`
Expected: compile errors — `cannot find struct \`ExprSpan\`` / `cannot find enum \`Expr\`` /
`cannot find enum \`Literal\`` / `cannot find enum \`LogicalOp\`` — none of these exist yet.

- [ ] **Step 3: Implement the AST types**

Add this content **above** the `#[cfg(test)] mod tests { ... }` block already in
`cel-parser/src/ast.rs` (the module doc comment at the top of the file from Step 1 stays where it
is):

```rust
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser ast::tests`
Expected: 4 tests pass (`span_returns_the_range_stored_on_a_leaf_variant`,
`span_returns_the_range_stored_on_a_composite_variant`,
`literal_variants_are_distinguishable_and_comparable`, `logical_op_equality`).

- [ ] **Step 5: Wire the new module into `cel-parser/src/lib.rs`**

Find this line (in the module declarations, added by Task 1's Step 5 changes are elsewhere in the
file — this line itself is unchanged from the prior phase):

```rust
pub mod parser_context;
```

Add immediately after it:

```rust
pub mod ast;
```

Then find this line (in the `pub use` block just below the module declarations):

```rust
pub use parser_context::{DynSegmentContext, ParserContext};
```

Add immediately after it:

```rust
pub use ast::{Expr, ExprSpan, Literal, LogicalOp};
```

- [ ] **Step 6: Run the full existing `cel-parser` test suite to confirm zero regressions**

Run: `cargo test -p cel-parser`
Expected: every test that existed before this task still passes, plus the 4 new `ast::tests` —
no failures, no changes to any existing test needed.

- [ ] **Step 7: Format and lint**

Run:
```bash
cargo fmt --all
cargo clippy -p cel-parser --all-targets -- -D warnings
```
Expected: `cargo fmt` makes no further changes beyond what it applies; `clippy` reports zero
warnings.

- [ ] **Step 8: Commit**

```bash
git add cel-parser/src/ast.rs cel-parser/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(cel-parser): add Expr/Literal/ExprSpan/LogicalOp AST types

Adds the span-carrying CEL expression AST that AstContext (next commit)
will build. Pure data types with no parsing logic yet.
EOF
)"
```

---

### Task 3: `AstContext` implementing `ParserContext`

**Files:**
- Modify: `cel-parser/src/ast.rs` (add `AstContext` above the existing `#[cfg(test)] mod tests`
  block, and add new tests to that block)

**Interfaces:**
- Consumes: `Expr`/`Literal`/`ExprSpan`/`LogicalOp` (Task 2); `ParserContext` (Task 1);
  `crate::op_table::OpLookup` (unused by `AstContext` but required by the trait signature).
- Produces (used by Task 4): `pub struct AstContext { .. }` implementing `ParserContext`, plus
  `pub fn into_expr(self) -> Expr`.

`AstContext` never consults `op_lookup` and never returns `Err` — every method that could
semantically fail in `DynSegmentContext` (unresolved operator, non-tuple `.N` base, out-of-range
index, branch type mismatch) instead records the node unconditionally, deferring validation to a
later, separate type-checking phase. `peek_tuple_arity` returns `Some(usize::MAX)` unconditionally
so `apply_tuple_index`'s `index < arity` check in `lib.rs` never rejects `.N` at parse time — e.g.
`some_call().0` must still produce an AST, even though its actual tuple-ness can't be known without
static type inference.

- [ ] **Step 1: Write the test module additions for `AstContext`**

Append this to the existing `#[cfg(test)] mod tests { ... }` block in `cel-parser/src/ast.rs`
(insert these `#[test]` functions alongside the ones already there, keeping the same `use super::*;`
and `use proc_macro2::Span;` at the top of the module):

```rust
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
                assert!(matches!(*lhs, Expr::Literal { value: Literal::Bool(true), .. }));
                assert!(matches!(*rhs, Expr::Literal { value: Literal::Bool(false), .. }));
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
                assert!(matches!(*cond, Expr::Literal { value: Literal::Bool(true), .. }));
                assert!(matches!(*then_branch, Expr::Literal { value: Literal::I32(1), .. }));
                assert!(matches!(*else_branch, Expr::Literal { value: Literal::I32(2), .. }));
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
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test -p cel-parser ast::tests`
Expected: compile errors — `cannot find struct \`AstContext\` in this scope` (and similarly for
every method call on it), since `AstContext` doesn't exist yet.

- [ ] **Step 3: Implement `AstContext`**

Add this content to `cel-parser/src/ast.rs`, immediately **above** the `#[cfg(test)] mod tests {
... }` block (after the `impl Expr { .. }` block from Task 2):

```rust
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
        self.values.pop().expect("operand present on AST value stack")
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
        debug_assert_eq!(rhs.values.len(), 1, "rhs fragment produces exactly one value");
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
        debug_assert_eq!(elements.len(), n, "make_tuple splits off exactly n elements");
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser ast::tests`
Expected: all tests pass — the 4 from Task 2 plus the 8 new ones from this task
(`push_literal_dispatches_every_concrete_literal_type`,
`apply_op_with_arity_zero_records_an_ident_node`,
`apply_op_with_the_call_operator_records_an_apply_node`,
`apply_op_with_arity_two_records_an_op_node`, `apply_logical_records_a_logical_node`,
`join2_records_an_if_node`, `make_tuple_and_tuple_index_roundtrip`,
`peek_tuple_arity_is_always_some_since_types_are_unresolved_during_parsing`).

- [ ] **Step 5: Re-export `AstContext` from `cel-parser/src/lib.rs`**

Find this line (added in Task 2's Step 5):

```rust
pub use ast::{Expr, ExprSpan, Literal, LogicalOp};
```

Replace it with:

```rust
pub use ast::{AstContext, Expr, ExprSpan, Literal, LogicalOp};
```

- [ ] **Step 6: Run the full existing `cel-parser` test suite to confirm zero regressions**

Run: `cargo test -p cel-parser`
Expected: every test passes — all tests from Tasks 1–2 unchanged, plus this task's 8 new tests.

- [ ] **Step 7: Format and lint**

Run:
```bash
cargo fmt --all
cargo clippy -p cel-parser --all-targets -- -D warnings
```
Expected: zero warnings.

- [ ] **Step 8: Commit**

```bash
git add cel-parser/src/ast.rs cel-parser/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(cel-parser): implement AstContext as a ParserContext

AstContext builds a span-carrying Expr tree instead of executing: it
never consults OpLookup and never fails on semantic grounds (unresolved
operators, non-tuple .N bases, out-of-range indices, branch type
mismatches are all deferred to a later type-checking phase). Verified
directly against the trait, independent of the grammar.
EOF
)"
```

---

### Task 4: Wire `AstContext` into `Parser`'s public API and add end-to-end grammar tests

**Files:**
- Modify: `cel-parser/src/lib.rs` (add `impl Parser<AstContext>` block, mirroring the existing
  `impl Parser<DynSegmentContext>` block; nothing else in this file changes in this task)
- Modify: `cel-parser/src/ast.rs` (append end-to-end tests to the existing `#[cfg(test)] mod
  tests` block)

**Interfaces:**
- Consumes: `AstContext`, `Expr` (Tasks 2–3); `Parser<C>`'s generic `parse_or_expression_ctx`,
  `parse_tokens_ctx`, `parse_str_ctx` (already exist from the prior phase, unchanged).
- Produces: `impl Parser<AstContext> { pub fn parse_or_expression_ast(&mut self) -> Result<Expr>;
  pub fn parse_tokens_ast(&mut self, tokens: TokenStreamIter) -> Result<Expr>;
  pub fn parse_str_ast(&mut self, s: &str) -> Result<Expr>; }` — this is the first public,
  end-to-end entry point into the AST-building path; nothing outside `cel-parser` consumes it yet
  (pm-lang's analogous work is a separate, later plan).

- [ ] **Step 1: Write the failing end-to-end tests**

Append this to the existing `#[cfg(test)] mod tests { ... }` block in `cel-parser/src/ast.rs`
(these use the crate's top-level `Parser`/`OpLookup`, not `AstContext` directly):

```rust
    use crate::{OpLookup, Parser};

    #[test]
    fn additive_binds_looser_than_multiplicative() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("1 + 2 * 3").unwrap();
        let Expr::Op { name, operands, .. } = expr else {
            panic!("expected Op");
        };
        assert_eq!(name, "+");
        assert!(matches!(operands[0], Expr::Literal { value: Literal::I32(1), .. }));
        let Expr::Op {
            name: inner_name,
            operands: mul_operands,
            ..
        } = &operands[1]
        else {
            panic!("expected nested Op");
        };
        assert_eq!(inner_name, "*");
        assert!(matches!(mul_operands[0], Expr::Literal { value: Literal::I32(2), .. }));
        assert!(matches!(mul_operands[1], Expr::Literal { value: Literal::I32(3), .. }));
    }

    #[test]
    fn unary_minus_and_not_are_recorded_as_arity_one_op_nodes() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("-1i32").unwrap();
        let Expr::Op { name, operands, .. } = expr else {
            panic!("expected Op");
        };
        assert_eq!(name, "-");
        assert_eq!(operands.len(), 1);

        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("!true").unwrap();
        let Expr::Op { name, operands, .. } = expr else {
            panic!("expected Op");
        };
        assert_eq!(name, "!");
        assert_eq!(operands.len(), 1);
    }

    #[test]
    fn bare_identifier_is_an_ident_node() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("x").unwrap();
        assert!(matches!(expr, Expr::Ident { name, .. } if name == "x"));
    }

    #[test]
    fn call_with_no_args_and_with_args() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("f()").unwrap();
        let Expr::Apply { callee, args, .. } = expr else {
            panic!("expected Apply");
        };
        assert!(matches!(*callee, Expr::Ident { ref name, .. } if name == "f"));
        assert_eq!(args.len(), 0);

        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("f(1i32, 2i32)").unwrap();
        let Expr::Apply { args, .. } = expr else {
            panic!("expected Apply");
        };
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn chained_calls_nest_apply_nodes() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("f()()").unwrap();
        let Expr::Apply { callee, args, .. } = expr else {
            panic!("expected outer Apply");
        };
        assert_eq!(args.len(), 0);
        assert!(matches!(*callee, Expr::Apply { .. }));
    }

    #[test]
    fn unit_grouping_and_tuples() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        assert!(matches!(
            parser.parse_str_ast("()").unwrap(),
            Expr::Literal { value: Literal::Unit, .. }
        ));

        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        // Grouping: (1i32 + 2i32) has no Tuple wrapper, just the inner Op.
        assert!(matches!(
            parser.parse_str_ast("(1i32 + 2i32)").unwrap(),
            Expr::Op { .. }
        ));

        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("(1i32,)").unwrap();
        assert!(matches!(expr, Expr::Tuple { ref elements, .. } if elements.len() == 1));

        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("(1i32, 2i32, 3i32)").unwrap();
        assert!(matches!(expr, Expr::Tuple { ref elements, .. } if elements.len() == 3));
    }

    #[test]
    fn tuple_index_single_and_chained() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("(1i32, 2i32).1").unwrap();
        let Expr::TupleIndex { base, index, .. } = expr else {
            panic!("expected TupleIndex");
        };
        assert_eq!(index, 1);
        assert!(matches!(*base, Expr::Tuple { ref elements, .. } if elements.len() == 2));

        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        // Chained `.0.1` tokenizes as one float literal `0.1` split back into two indices.
        let expr = parser.parse_str_ast("((1i32, 2i32), 3i32).0.1").unwrap();
        let Expr::TupleIndex { base, index, .. } = expr else {
            panic!("expected outer TupleIndex");
        };
        assert_eq!(index, 1);
        assert!(matches!(*base, Expr::TupleIndex { index: 0, .. }));
    }

    #[test]
    fn tuple_index_on_a_call_result_still_builds_a_tree() {
        // Deferred validation: DynSegmentContext would reject an out-of-range index at parse
        // time, but AstContext can't know a call's return arity without type inference, so it
        // must still build a tree here rather than fail to parse.
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("f().5").unwrap();
        assert!(matches!(expr, Expr::TupleIndex { index: 5, .. }));
    }

    #[test]
    fn if_without_else_has_a_unit_else_branch() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("if true { 1i32 }").unwrap();
        let Expr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } = expr
        else {
            panic!("expected If");
        };
        assert!(matches!(*cond, Expr::Literal { value: Literal::Bool(true), .. }));
        assert!(matches!(*then_branch, Expr::Literal { value: Literal::I32(1), .. }));
        assert!(matches!(*else_branch, Expr::Literal { value: Literal::Unit, .. }));
    }

    #[test]
    fn if_else_and_else_if_chain() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser
            .parse_str_ast("if true { 1i32 } else { 2i32 }")
            .unwrap();
        let Expr::If { else_branch, .. } = expr else {
            panic!("expected If");
        };
        assert!(matches!(*else_branch, Expr::Literal { value: Literal::I32(2), .. }));

        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser
            .parse_str_ast("if true { 1i32 } else if false { 2i32 } else { 3i32 }")
            .unwrap();
        let Expr::If { else_branch, .. } = expr else {
            panic!("expected outer If");
        };
        assert!(matches!(*else_branch, Expr::If { .. }));
    }

    #[test]
    fn logical_or_is_not_desugared_to_if() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("a || b").unwrap();
        let Expr::Logical { op, lhs, rhs, .. } = expr else {
            panic!("expected Logical");
        };
        assert_eq!(op, LogicalOp::Or);
        assert!(matches!(*lhs, Expr::Ident { ref name, .. } if name == "a"));
        assert!(matches!(*rhs, Expr::Ident { ref name, .. } if name == "b"));
    }

    #[test]
    fn logical_and_is_not_desugared_to_if() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("a && b").unwrap();
        let Expr::Logical { op, .. } = expr else {
            panic!("expected Logical");
        };
        assert_eq!(op, LogicalOp::And);
    }

    #[test]
    fn logical_mixes_with_comparison_at_the_right_precedence() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        // `&&` binds looser than `==`, so this is `(a == 1i32) && (b == 2i32)`.
        let expr = parser.parse_str_ast("a == 1i32 && b == 2i32").unwrap();
        let Expr::Logical { op, lhs, rhs, .. } = expr else {
            panic!("expected Logical");
        };
        assert_eq!(op, LogicalOp::And);
        assert!(matches!(*lhs, Expr::Op { ref name, .. } if name == "=="));
        assert!(matches!(*rhs, Expr::Op { ref name, .. } if name == "=="));
    }
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test -p cel-parser ast::tests`
Expected: compile errors — `no method named \`parse_str_ast\` found for struct \`Parser<AstContext>\``
— the method doesn't exist yet.

- [ ] **Step 3: Add `impl Parser<AstContext>` to `cel-parser/src/lib.rs`**

Find the end of the existing `impl Parser<DynSegmentContext> { ... }` block (currently ends at
line 1189, immediately before `#[cfg(test)]\nmod tests {`). Add this new block immediately after
it (and before `#[cfg(test)]\nmod tests {`):

```rust

impl Parser<AstContext> {
    /// Parses one `or_expression` from the current token stream and returns the built [`Expr`].
    ///
    /// Unlike [`parse_str_ast`](Self::parse_str_ast), this method does not require
    /// end-of-stream, allowing pm-lang to parse an expression embedded within a larger token
    /// stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the input does not contain a valid `or_expression`.
    ///
    /// - Complexity: O(n) in the number of tokens in the expression.
    pub fn parse_or_expression_ast(&mut self) -> Result<Expr> {
        self.parse_or_expression_ctx().map(AstContext::into_expr)
    }

    /// Parses a token stream into an [`Expr`] tree.
    ///
    /// Sets the token source, runs the expression grammar, and returns the tree on success.
    ///
    /// # Errors
    ///
    /// Returns an error if the input does not contain a valid CEL expression.
    pub fn parse_tokens_ast(&mut self, tokens: TokenStreamIter) -> Result<Expr> {
        self.parse_tokens_ctx(tokens).map(AstContext::into_expr)
    }

    /// Parses a string into an [`Expr`] tree.
    ///
    /// Tokenizes the string then parses; equivalent to
    /// `parse_tokens_ast(TokenStream::from_str(s)?.into_iter())`.
    ///
    /// # Errors
    ///
    /// Returns an error on lex failure or if the input does not contain a valid CEL expression.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::{AstContext, Expr, OpLookup, Parser};
    ///
    /// let mut parser = Parser::<AstContext>::new(OpLookup::new());
    /// let expr = parser.parse_str_ast("1 + 2").unwrap();
    /// assert!(matches!(expr, Expr::Op { .. }));
    /// ```
    pub fn parse_str_ast(&mut self, s: &str) -> Result<Expr> {
        self.parse_str_ctx(s).map(AstContext::into_expr)
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser ast::tests`
Expected: all tests pass — the 12 from Tasks 2–3 plus the 13 new end-to-end tests from this task.

- [ ] **Step 5: Run the full `cel-parser` test suite**

Run: `cargo test -p cel-parser`
Expected: every test passes — the complete regression suite from the prior phase (untouched
throughout this whole plan) plus every test this plan added across Tasks 1–4.

- [ ] **Step 6: Run doc tests**

Run: `cargo test --doc -p cel-parser`
Expected: every doc example compiles and passes, including the new `parse_str_ast` and
`AstContext` examples.

- [ ] **Step 7: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: every test in every crate passes, including `pm-lang`, `cel-rs-macros`,
`property-model`, and `begin` — none of these crates' source is touched by this plan, so this
step is purely a safety-net confirmation that `CELParser`'s public API is unchanged and the new
`AstContext` path is additive only.

- [ ] **Step 8: Lint and build checks**

Run:
```bash
cargo build --workspace
cargo clippy -p cel-parser --all-targets -- -D warnings
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: zero warnings from all five invocations.

- [ ] **Step 9: Format and commit**

```bash
cargo fmt --all
git add cel-parser/src/lib.rs cel-parser/src/ast.rs
git commit -m "$(cat <<'EOF'
feat(cel-parser): expose Parser<AstContext>'s parse_*_ast entry points

Adds parse_or_expression_ast/parse_tokens_ast/parse_str_ast, mirroring
the existing DynSegmentContext entry points. This is the first public,
end-to-end way to parse CEL source into an Expr tree, verified against
one test per grammar construct (literals, idents, unary/binary ops,
calls, tuples, tuple indexing, if/else/else-if, && / ||).
EOF
)"
```

---

## Self-Review

**Spec coverage:** This plan covers the `cel-parser` half of the design doc's Phase 2
("`AstContext` + AST types" — the CEL expression AST, `AstContext`, and the trait widening it
needs). The `pm-lang`-side structural AST, its declaration-level parser generalization, coarse
error recovery, and the comment/trivia reattachment pass are all explicitly out of scope here and
will each be their own follow-on plan, exactly mirroring how the prior phase's plan split
`cel-parser` from `pm-lang`. Phases 3–5 of the design doc (`pm-lsp` diagnostics, formatter, richer
LSP features) remain out of scope and unaffected.

**Placeholder scan:** No TBD/TODO; every step shows complete code; no step says "similar to Task
N" without the code.

**Type consistency:** `ParserContext`'s widened methods (`push_literal`, `make_tuple`,
`tuple_index`, `join2`) and the new `apply_logical` are named and typed identically between
Task 1's trait definition, `DynSegmentContext`'s impl, every `lib.rs` call site, and Task 3's
`AstContext` impl. `Expr`/`Literal`/`ExprSpan`/`LogicalOp` (Task 2) are used with the same field
names and variant shapes in Task 3's `AstContext` impl and Task 4's end-to-end tests (e.g.
`Expr::Op { name, operands, span }` never drifts to a different field set). `Parser<AstContext>`'s
`parse_or_expression_ast`/`parse_tokens_ast`/`parse_str_ast` (Task 4) consistently mirror the
existing `Parser<DynSegmentContext>` entry points' names and signatures, substituting `Expr` for
`DynSegment`.

---

Plan complete and saved to `docs/superpowers/plans/2026-07-17-cel-parser-ast-context.md`. Two execution options:

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
