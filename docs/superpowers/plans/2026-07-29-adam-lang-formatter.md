# adam-lang Formatter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `docs/superpowers/specs/2026-07-29-adam-lang-formatter-design.md` — Phase 4
("Formatter") of `docs/superpowers/specs/2026-07-17-pm-lang-language-server-design.md` — a
`cargo fmt`-style formatter for `.adm2` source, wired into `adam-lsp`'s `textDocument/formatting`
and enabled by default for format-on-save in `editors/vscode-adam-lang`.

**Architecture:** A precedence-aware pretty-printer for `cel_parser::Expr`
(`cel_parser::format_expr`), a structural pretty-printer for `adam_lang::ast::Sheet`
(`adam_lang::format_sheet`) that delegates method bodies/initializers to the former, a
generalization of `adam-lang`'s existing comment-recovery pass (`attach_trivia`) so it also
attaches comments and blank-line-before flags inside nested `relationship`/`conditional` bodies
(not just top-level sheet items), and an `adam-lsp` `textDocument/formatting` handler backed by a
small `Uri -> text` document store. A prerequisite fix to `cel_parser::Expr::If` (making its
implicit `else` distinguishable from an explicit one) lands first, since the formatter cannot
correctly print an `if` without it.

**Tech Stack:** Rust 2024, existing `cel-parser`/`adam-lang`/`adam-lsp` crates. No new
dependencies — `proc-macro2`'s `span-locations` feature (already enabled in `cel-parser`'s and
`adam-lang`'s `Cargo.toml`) is what makes `Span::source_text()` work outside a real proc-macro
invocation.

## Global Constraints

- Every function gets a contract-style `///` doc comment (Summary sentence; `- Precondition:` /
  `- Postcondition:` bullets only where non-obvious; `# Errors` for `Result`-returning functions;
  `# Examples` on public API). Modules use `//!` with a short usage tutorial. (Repo `CLAUDE.md`.)
- Unit tests are derived from each function's contract and public interface only — not from
  reading the implementation.
- `cargo fmt --all` before every commit (enforced by the pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` must pass before this
  work is handed off for a PR (run once at the end of the plan, not per-task).
- Never commit directly to `main`; this work lands on the current worktree branch.
- Fallible arithmetic on signed integers uses `checked_*`, not wrapping — not exercised by this
  plan (no signed-integer arithmetic is performed; `index: usize` and line/column counters are the
  only integer math involved).
- No file-based test fixtures — this repo's existing convention (see `cel-parser`/`adam-lang`'s
  test suites) is inline string-literal golden tests in `#[cfg(test)]`, not external files.

---

### Task 1: Prerequisite fix — `Expr::If`'s implicit else becomes `None`, not a synthesized `Literal::Unit`

**Files:**
- Modify: `cel-parser/src/parser_context.rs` (`ParserContext::join2` trait signature + doc;
  `DynSegmentContext::join2` impl; two existing tests)
- Modify: `cel-parser/src/ast.rs` (`Expr::If::else_branch` field type + doc; `AstContext::join2`
  impl; four existing tests; one new test)
- Modify: `cel-parser/src/lib.rs` (`is_if_expression`'s `else_fragment` construction)
- Modify: `cel-parser/src/ty.rs` (`check_expr`'s `Expr::If` handling; one existing test)

**Interfaces:**
- Produces (used by every later task in this plan): `Expr::If { cond: Box<Expr>, then_branch:
  Box<Expr>, else_branch: Option<Box<Expr>>, span: ExprSpan }` — `None` means no `else`/`else if`
  was written in the source.
- Consumes: nothing new — this task only changes an existing field's type and the trait method
  that produces it.

- [ ] **Step 1: Change the `ParserContext::join2` trait signature**

In `cel-parser/src/parser_context.rs`, find:

```rust
    /// Joins two previously-built fragments into `self`, consuming a leading condition value
    /// already present on `self`. `then_fragment`'s contribution is used when the condition is
    /// `true`; `else_fragment`'s when `false`. `start`/`end` cover the whole `if`/`else`
    /// construct.
    ///
    /// - Precondition: neither fragment takes arguments, and each produces exactly one value.
    ///
    /// # Errors
    ///
    /// Implementations that validate operand types during parsing (e.g. [`DynSegmentContext`])
    /// return `Err` if the leading condition value isn't a `bool` or if the fragments' produced
    /// types don't match. Implementations that defer type validation to a later phase (e.g.
    /// [`crate::ast::AstContext`]) never return `Err` here.
    fn join2(
        &mut self,
        then_fragment: Self,
        else_fragment: Self,
        start: Span,
        end: Span,
    ) -> anyhow::Result<()>;
```

Replace with:

```rust
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
```

- [ ] **Step 2: Update `DynSegmentContext::join2` to synthesize the implicit fragment itself**

In the same file, find:

```rust
    fn join2(
        &mut self,
        then_fragment: Self,
        else_fragment: Self,
        _start: Span,
        _end: Span,
    ) -> anyhow::Result<()> {
        self.0.join2(then_fragment.0, else_fragment.0)
    }
```

Replace with:

```rust
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
```

- [ ] **Step 3: Update the two `DynSegmentContext` tests that call `join2` directly**

In the same file's `#[cfg(test)] mod tests`, find:

```rust
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
```

Replace with (only the two `ctx.join2(...)` calls change, wrapping `else_fragment` in `Some`):

```rust
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
```

- [ ] **Step 4: Run `cel-parser`'s tests to confirm Steps 1–3 compile and pass so far**

Run: `cargo test -p cel-parser parser_context::`
Expected: compile errors in `ast.rs`, `lib.rs`, and `ty.rs` (not yet updated) — this is expected;
proceed to the next steps before running the full suite.

- [ ] **Step 5: Update `Expr::If`'s `else_branch` field and `AstContext::join2`**

In `cel-parser/src/ast.rs`, find:

```rust
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
```

Replace with:

```rust
    /// An `if cond { then } else { else_ }` expression.
    If {
        /// The condition.
        cond: Box<Expr>,
        /// The then-branch.
        then_branch: Box<Expr>,
        /// The else-branch, or `None` if no `else`/`else if` was written.
        else_branch: Option<Box<Expr>>,
        /// The span of the whole `if`/`else` construct.
        span: ExprSpan,
    },
```

Find:

```rust
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
```

Replace with:

```rust
    fn join2(
        &mut self,
        mut then_fragment: Self,
        else_fragment: Option<Self>,
        start: Span,
        end: Span,
    ) -> anyhow::Result<()> {
        let cond = self.pop();
        debug_assert_eq!(
            then_fragment.values.len(),
            1,
            "then fragment produces exactly one value"
        );
        let then_branch = then_fragment.pop();
        let else_branch = else_fragment.map(|mut fragment| {
            debug_assert_eq!(
                fragment.values.len(),
                1,
                "else fragment produces exactly one value"
            );
            Box::new(fragment.pop())
        });
        self.values.push(Expr::If {
            cond: Box::new(cond),
            then_branch: Box::new(then_branch),
            else_branch,
            span: ExprSpan { start, end },
        });
        Ok(())
    }
```

- [ ] **Step 6: Update `ast.rs`'s tests that construct or match on `Expr::If`**

Find:

```rust
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
```

Replace with:

```rust
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
            else_branch: Some(Box::new(Expr::Literal {
                value: Literal::I32(2),
                span: target,
            })),
            span: target,
        };
        assert_eq!(format!("{:?}", expr.span()), format!("{target:?}"));
    }
```

Find:

```rust
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
```

Replace with:

```rust
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
            Some(else_fragment),
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
                let else_branch = else_branch.expect("explicit else was given");
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
    fn join2_with_none_records_no_else_branch() {
        let mut ctx = AstContext::new_context();
        ctx.push_literal(true, Span::call_site());
        let mut then_fragment = ctx.new_fragment();
        then_fragment.push_literal(1i32, Span::call_site());
        ctx.join2(then_fragment, None, Span::call_site(), Span::call_site())
            .unwrap();
        match ctx.into_expr() {
            Expr::If { else_branch, .. } => assert!(else_branch.is_none()),
            other => panic!("expected If, got {other:?}"),
        }
    }
```

Find:

```rust
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
                value: Literal::Unit,
                ..
            }
        ));
    }
```

Replace with:

```rust
    #[test]
    fn if_without_else_has_no_else_branch() {
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
        assert!(else_branch.is_none());
    }
```

Find:

```rust
    #[test]
    fn if_else_and_else_if_chain() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser
            .parse_str_ast("if true { 1i32 } else { 2i32 }")
            .unwrap();
        let Expr::If { else_branch, .. } = expr else {
            panic!("expected If");
        };
        assert!(matches!(
            *else_branch,
            Expr::Literal {
                value: Literal::I32(2),
                ..
            }
        ));

        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser
            .parse_str_ast("if true { 1i32 } else if false { 2i32 } else { 3i32 }")
            .unwrap();
        let Expr::If { else_branch, .. } = expr else {
            panic!("expected outer If");
        };
        assert!(matches!(*else_branch, Expr::If { .. }));
    }
```

Replace with:

```rust
    #[test]
    fn if_else_and_else_if_chain() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser
            .parse_str_ast("if true { 1i32 } else { 2i32 }")
            .unwrap();
        let Expr::If { else_branch, .. } = expr else {
            panic!("expected If");
        };
        assert!(matches!(
            else_branch.as_deref(),
            Some(Expr::Literal {
                value: Literal::I32(2),
                ..
            })
        ));

        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser
            .parse_str_ast("if true { 1i32 } else if false { 2i32 } else { 3i32 }")
            .unwrap();
        let Expr::If { else_branch, .. } = expr else {
            panic!("expected outer If");
        };
        assert!(matches!(else_branch.as_deref(), Some(Expr::If { .. })));
    }
```

- [ ] **Step 7: Update `is_if_expression` in `cel-parser/src/lib.rs`**

Find:

```rust
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
```

Replace with:

```rust
        let else_fragment = if self.is_keyword("else") {
            if self.is_keyword("if") {
                // else if: recursively parse another if_expression
                let elif_span = self.last_span;
                let mut fragment = self.context.new_fragment();
                std::mem::swap(&mut self.context, &mut fragment);
                self.is_if_expression(elif_span)?;
                std::mem::swap(&mut self.context, &mut fragment);
                Some(fragment)
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
                Some(fragment)
            }
        } else {
            // No `else`/`else if` in the source — each ParserContext::join2 impl decides what
            // this means (DynSegmentContext synthesizes an implicit `()` fragment;
            // AstContext records `None` directly on Expr::If).
            None
        };
        self.context
            .join2(then_fragment, else_fragment, if_span, self.last_span)
            .map_err(|e| ParseError::new(e.to_string(), self.last_span))?;
        Ok(true)
```

- [ ] **Step 8: Update `cel-parser/src/ty.rs`'s `check_expr` and its one `Expr::If` test**

Find:

```rust
        Expr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            let mut diagnostics = check_expr(cond, resolve_ident).1;
            diagnostics.extend(check_expr(then_branch, resolve_ident).1);
            diagnostics.extend(check_expr(else_branch, resolve_ident).1);
            (Ty::Any, diagnostics)
        }
```

Replace with:

```rust
        Expr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            let mut diagnostics = check_expr(cond, resolve_ident).1;
            diagnostics.extend(check_expr(then_branch, resolve_ident).1);
            if let Some(else_branch) = else_branch {
                diagnostics.extend(check_expr(else_branch, resolve_ident).1);
            }
            (Ty::Any, diagnostics)
        }
```

Find:

```rust
    #[test]
    fn a_broken_op_nested_inside_an_if_condition_still_surfaces_a_diagnostic() {
        let expr = Expr::If {
            cond: Box::new(op("+", vec![lit_i32(1), lit_str("s")])),
            then_branch: Box::new(lit_i32(1)),
            else_branch: Box::new(lit_i32(2)),
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Any, "If itself is not type-checked in v1");
        assert_eq!(diags.len(), 1);
    }
```

Replace with:

```rust
    #[test]
    fn a_broken_op_nested_inside_an_if_condition_still_surfaces_a_diagnostic() {
        let expr = Expr::If {
            cond: Box::new(op("+", vec![lit_i32(1), lit_str("s")])),
            then_branch: Box::new(lit_i32(1)),
            else_branch: Some(Box::new(lit_i32(2))),
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Any, "If itself is not type-checked in v1");
        assert_eq!(diags.len(), 1);
    }
```

- [ ] **Step 9: Run the full `cel-parser` test suite**

Run: `cargo test -p cel-parser`
Expected: every test passes, including the new `join2_with_none_synthesizes_an_implicit_unit_else_fragment`
(`parser_context.rs`) and `join2_with_none_records_no_else_branch` (`ast.rs`).

- [ ] **Step 10: Confirm the rest of the workspace still builds**

Run: `cargo build --workspace`
Expected: zero warnings — `Expr::If` isn't constructed or matched anywhere outside `cel-parser`
(confirmed by search before writing this plan), so no other crate needs a change.

- [ ] **Step 11: Format, lint, and commit**

```bash
cargo fmt --all
cargo clippy -p cel-parser --all-targets -- -D warnings
```

```bash
git add cel-parser/src/parser_context.rs cel-parser/src/ast.rs cel-parser/src/lib.rs cel-parser/src/ty.rs
git commit -m "$(cat <<'EOF'
fix(cel-parser): make Expr::If's implicit else distinguishable from an explicit one

else_branch is now Option<Box<Expr>> instead of always a Box<Expr>
synthesized as Literal::Unit when no `else` was written. The
synthetic node's span pointed at the then-branch's closing `}`, not
an actual `()` token, so a formatter re-slicing literal text from
spans would have printed it verbatim as `}`. DynSegmentContext still
synthesizes an implicit `()` fragment internally (moved from the
shared is_if_expression into DynSegmentContext::join2 itself) since
execution still needs a concrete fragment to select on `false`;
AstContext now records None directly instead.
EOF
)"
```

---

### Task 2: `cel_parser::format_expr` — the CEL expression pretty-printer

**Files:**
- Create: `cel-parser/src/fmt.rs`
- Modify: `cel-parser/src/lib.rs` (add `pub mod fmt;` and `pub use fmt::format_expr;`)

**Interfaces:**
- Consumes: `crate::ast::{Expr, ExprSpan, Literal, LogicalOp}` (Task 1's updated `Expr::If`
  shape), `proc_macro2::Span::source_text(&self) -> Option<String>`.
- Produces (used by Task 4): `pub fn cel_parser::format_expr(expr: &Expr) -> String`.

- [ ] **Step 1: Write the failing tests**

Create `cel-parser/src/fmt.rs` with only its module doc, imports, and test module (the real
functions don't exist yet — this is the intended failing state):

```rust
//! Pretty-prints a [`crate::Expr`] tree back to CEL source text: precedence-aware
//! parenthesization (added only where required, not exhaustively), single-space-around-operator
//! normalization, and no line-wrapping (every expression is emitted on one line regardless of
//! length — see the design doc's "Line wrapping" decision). Literal leaves are re-emitted via
//! [`proc_macro2::Span::source_text`] rather than synthesized from [`crate::Literal`], so exact
//! original notation (`1920.0` vs `1920.0f64`, a byte literal's spelling) round-trips.

use crate::ast::{Expr, Literal, LogicalOp};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AstContext, OpLookup, Parser};

    fn parse(source: &str) -> Expr {
        Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap()
    }

    #[test]
    fn additive_and_multiplicative_reprint_without_extra_parens() {
        let expr = parse("1i32 + 2i32 * 3i32");
        assert_eq!(format_expr(&expr), "1i32 + 2i32 * 3i32");
    }

    #[test]
    fn explicit_grouping_that_changes_precedence_keeps_its_parens() {
        let expr = parse("(1i32 + 2i32) * 3i32");
        assert_eq!(format_expr(&expr), "(1i32 + 2i32) * 3i32");
    }

    #[test]
    fn left_associative_chain_at_the_same_precedence_has_no_parens() {
        let expr = parse("1i32 - 2i32 - 3i32");
        assert_eq!(format_expr(&expr), "1i32 - 2i32 - 3i32");
    }

    #[test]
    fn a_right_leaning_tree_at_the_same_precedence_needs_parens() {
        // Not producible by real parsing (the grammar's additive_expression loop is always
        // left-associative) — built by hand to prove the printer round-trips a tree shape it
        // didn't itself produce. Uses Ident operands (rendered from `name`, not a span) so the
        // assertion reads as real text rather than the no-source-text fallback.
        fn ident(name: &str) -> Expr {
            Expr::Ident {
                name: name.to_string(),
                span: point(),
            }
        }
        fn point() -> crate::ExprSpan {
            crate::ExprSpan {
                start: proc_macro2::Span::call_site(),
                end: proc_macro2::Span::call_site(),
            }
        }
        let expr = Expr::Op {
            name: "-".to_string(),
            operands: vec![
                ident("a"),
                Expr::Op {
                    name: "-".to_string(),
                    operands: vec![ident("b"), ident("c")],
                    span: point(),
                },
            ],
            span: point(),
        };
        assert_eq!(format_expr(&expr), "a - (b - c)");
    }

    #[test]
    fn nested_comparison_needs_parens_on_both_sides() {
        // Also not producible by real parsing (comparison_expression allows at most one
        // comparison, never a nested one) — proves format_expr stays reparseable even for a
        // hand-built tree shape the grammar itself can't emit.
        fn ident(name: &str) -> Expr {
            Expr::Ident {
                name: name.to_string(),
                span: crate::ExprSpan {
                    start: proc_macro2::Span::call_site(),
                    end: proc_macro2::Span::call_site(),
                },
            }
        }
        let inner = Expr::Op {
            name: "==".to_string(),
            operands: vec![ident("a"), ident("b")],
            span: crate::ExprSpan {
                start: proc_macro2::Span::call_site(),
                end: proc_macro2::Span::call_site(),
            },
        };
        let expr = Expr::Op {
            name: "==".to_string(),
            operands: vec![inner, ident("c")],
            span: crate::ExprSpan {
                start: proc_macro2::Span::call_site(),
                end: proc_macro2::Span::call_site(),
            },
        };
        assert_eq!(format_expr(&expr), "(a == b) == c");
    }

    #[test]
    fn literal_notation_is_preserved_exactly() {
        assert_eq!(format_expr(&parse("1920.0")), "1920.0");
        assert_eq!(format_expr(&parse("1920.0f64")), "1920.0f64");
        assert_eq!(format_expr(&parse("1i32")), "1i32");
    }

    #[test]
    fn unary_minus_of_a_binary_expression_needs_parens() {
        assert_eq!(format_expr(&parse("-(1i32 + 2i32)")), "-(1i32 + 2i32)");
    }

    #[test]
    fn double_unary_minus_keeps_a_separating_space() {
        assert_eq!(format_expr(&parse("- -1i32")), "- -1i32");
    }

    #[test]
    fn one_tuple_keeps_its_trailing_comma() {
        assert_eq!(format_expr(&parse("(1i32,)")), "(1i32,)");
    }

    #[test]
    fn multi_element_tuple_has_no_trailing_comma() {
        assert_eq!(format_expr(&parse("(1i32, 2i32)")), "(1i32, 2i32)");
    }

    #[test]
    fn if_without_else_omits_the_else_clause() {
        assert_eq!(format_expr(&parse("if true { 1i32 }")), "if true { 1i32 }");
    }

    #[test]
    fn if_else_reprints_both_branches() {
        assert_eq!(
            format_expr(&parse("if true { 1i32 } else { 2i32 }")),
            "if true { 1i32 } else { 2i32 }"
        );
    }

    #[test]
    fn else_if_chain_has_no_braces_around_the_nested_if() {
        let source = "if true { 1i32 } else if false { 2i32 } else { 3i32 }";
        assert_eq!(format_expr(&parse(source)), source);
    }

    #[test]
    fn logical_or_and_and_are_not_desugared_and_need_no_extra_parens() {
        assert_eq!(format_expr(&parse("a || b && c")), "a || b && c");
    }

    #[test]
    fn format_is_idempotent_through_a_reparse() {
        let source = "(1i32 + 2i32) * 3i32 - -4i32";
        let once = format_expr(&parse(source));
        let twice = format_expr(&parse(&once));
        assert_eq!(once, twice);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test -p cel-parser fmt::`
Expected: compile error — `cannot find function \`format_expr\`` — it doesn't exist yet.

- [ ] **Step 3: Implement the formatter**

Add this content **above** the `#[cfg(test)] mod tests { ... }` block already in
`cel-parser/src/fmt.rs` (the module doc comment and `use` line from Step 1 stay where they are):

```rust
/// Binding-strength level, loosest first, mirroring `lib.rs`'s grammar chain from
/// `or_expression` through `primary_expression`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Level(u8);

impl Level {
    const OR: Level = Level(0);
    const AND: Level = Level(1);
    const COMPARISON: Level = Level(2);
    const BIT_OR: Level = Level(3);
    const BIT_XOR: Level = Level(4);
    const BIT_AND: Level = Level(5);
    const SHIFT: Level = Level(6);
    const ADDITIVE: Level = Level(7);
    const MULTIPLICATIVE: Level = Level(8);
    const UNARY: Level = Level(9);
    const POSTFIX: Level = Level(10);
    const PRIMARY: Level = Level(11);

    /// The next level up (strictly tighter-binding than `self`).
    fn tighter(self) -> Level {
        Level(self.0 + 1)
    }
}

/// Returns the binding-strength level of a binary (two-operand) operator.
///
/// - Precondition: `name` is one of the binary operator tokens `lib.rs`'s grammar recognizes.
fn binary_op_level(name: &str) -> Level {
    match name {
        "|" => Level::BIT_OR,
        "^" => Level::BIT_XOR,
        "&" => Level::BIT_AND,
        "<<" | ">>" => Level::SHIFT,
        "+" | "-" => Level::ADDITIVE,
        "*" | "/" | "%" => Level::MULTIPLICATIVE,
        "==" | "!=" | "<" | ">" | "<=" | ">=" => Level::COMPARISON,
        other => unreachable!("binary_op_level called with unknown operator `{other}`"),
    }
}

/// Re-emits a literal's exact original text via its span, falling back to an empty string when
/// none is recoverable (spans built without a live source file — never a real parse; see the
/// module doc).
fn render_literal(span: crate::ExprSpan) -> String {
    span.start.source_text().unwrap_or_default()
}

/// Renders `expr` on its own, returning its text alongside its binding-strength level, so the
/// caller ([`format_at`]) can decide whether the context it's being placed in requires parens.
fn render(expr: &Expr) -> (String, Level) {
    match expr {
        Expr::Literal { span, .. } => (render_literal(*span), Level::PRIMARY),
        Expr::Ident { name, .. } => (name.clone(), Level::PRIMARY),
        Expr::Logical { op, lhs, rhs, .. } => {
            let level = match op {
                LogicalOp::Or => Level::OR,
                LogicalOp::And => Level::AND,
            };
            let op_str = match op {
                LogicalOp::Or => "||",
                LogicalOp::And => "&&",
            };
            let lhs_s = format_at(lhs, level);
            let rhs_s = format_at(rhs, level.tighter());
            (format!("{lhs_s} {op_str} {rhs_s}"), level)
        }
        Expr::Op { name, operands, .. } if operands.len() == 1 => {
            let operand_s = format_at(&operands[0], Level::UNARY);
            // A bare "-"/"!" glued directly onto an operand that itself starts with "-"/"!"
            // would re-tokenize as one run of punctuation; a single space disambiguates.
            let sep = if operand_s.starts_with('-') || operand_s.starts_with('!') {
                " "
            } else {
                ""
            };
            (format!("{name}{sep}{operand_s}"), Level::UNARY)
        }
        Expr::Op { name, operands, .. } => {
            let level = binary_op_level(name);
            // Comparison can't chain — the grammar parses at most one per comparison_expression —
            // so both operands must be strictly tighter than Comparison itself, unlike the other,
            // left-associative (chaining) binary levels below, where only the right operand does.
            let (lhs_min, rhs_min) = if level == Level::COMPARISON {
                (level.tighter(), level.tighter())
            } else {
                (level, level.tighter())
            };
            let lhs_s = format_at(&operands[0], lhs_min);
            let rhs_s = format_at(&operands[1], rhs_min);
            (format!("{lhs_s} {name} {rhs_s}"), level)
        }
        Expr::Apply { callee, args, .. } => {
            let callee_s = format_at(callee, Level::POSTFIX);
            let args_s = args
                .iter()
                .map(|a| format_at(a, Level::OR))
                .collect::<Vec<_>>()
                .join(", ");
            (format!("{callee_s}({args_s})"), Level::POSTFIX)
        }
        Expr::Tuple { elements, .. } => {
            let inner = elements
                .iter()
                .map(|e| format_at(e, Level::OR))
                .collect::<Vec<_>>()
                .join(", ");
            let text = if elements.len() == 1 {
                format!("({inner},)")
            } else {
                format!("({inner})")
            };
            (text, Level::PRIMARY)
        }
        Expr::TupleIndex { base, index, .. } => (
            format!("{}.{}", format_at(base, Level::POSTFIX), index),
            Level::POSTFIX,
        ),
        Expr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            let cond_s = format_at(cond, Level::OR);
            let then_s = format_at(then_branch, Level::OR);
            let mut text = format!("if {cond_s} {{ {then_s} }}");
            if let Some(else_branch) = else_branch {
                if matches!(else_branch.as_ref(), Expr::If { .. }) {
                    let (else_s, _) = render(else_branch);
                    text.push_str(&format!(" else {else_s}"));
                } else {
                    let else_s = format_at(else_branch, Level::OR);
                    text.push_str(&format!(" else {{ {else_s} }}"));
                }
            }
            (text, Level::PRIMARY)
        }
    }
}

/// Renders `expr`, wrapping it in parens if its own level is looser than `min_level` requires.
fn format_at(expr: &Expr, min_level: Level) -> String {
    let (text, level) = render(expr);
    if level < min_level {
        format!("({text})")
    } else {
        text
    }
}

/// Pretty-prints `expr` back to CEL source text — see the module doc for the printing rules.
///
/// # Examples
///
/// ```
/// use cel_parser::{AstContext, OpLookup, Parser, format_expr};
///
/// let mut parser = Parser::<AstContext>::new(OpLookup::new());
/// let expr = parser.parse_str_ast("(1i32 + 2i32) * 3i32").unwrap();
/// assert_eq!(format_expr(&expr), "(1i32 + 2i32) * 3i32");
/// ```
pub fn format_expr(expr: &Expr) -> String {
    format_at(expr, Level::OR)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser fmt::`
Expected: all 15 tests pass.

- [ ] **Step 5: Wire the new module into `cel-parser/src/lib.rs`**

Find:

```rust
pub mod ast;
mod error;
pub mod lex_lexer;
pub mod op_table;
pub mod parser_context;
pub mod ty;

pub use ast::{AstContext, Expr, ExprSpan, Literal, LogicalOp};
```

Replace with:

```rust
pub mod ast;
mod error;
mod fmt;
pub mod lex_lexer;
pub mod op_table;
pub mod parser_context;
pub mod ty;

pub use ast::{AstContext, Expr, ExprSpan, Literal, LogicalOp};
pub use fmt::format_expr;
```

- [ ] **Step 6: Run the full `cel-parser` test suite and its doc tests**

Run: `cargo test -p cel-parser`
Run: `cargo test --doc -p cel-parser`
Expected: every existing test still passes, plus `fmt`'s 15 new tests and `format_expr`'s doctest.

- [ ] **Step 7: Format and lint**

```bash
cargo fmt --all
cargo clippy -p cel-parser --all-targets -- -D warnings
```

- [ ] **Step 8: Commit**

```bash
git add cel-parser/src/fmt.rs cel-parser/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(cel-parser): add format_expr, a precedence-aware Expr pretty-printer

Re-emits literals via Span::source_text() (preserving exact numeric
notation), adds parens only where the grammar's twelve binding-
strength levels actually require them, and never line-wraps (v1
scope — see the formatter design doc).
EOF
)"
```

---

### Task 3: Generalize `adam-lang`'s trivia attachment to nested blocks, plus `blank_line_before`

**Files:**
- Modify: `adam-lang/src/ast.rs` (add `leading_comment`/`blank_line_before` to `MethodDecl` and
  `ConditionalBranch`; add `blank_line_before` to `CellDecl`, `RelationshipDecl`,
  `ConditionalDecl`, `SheetItem::Error`; add `SheetItem::set_blank_line_before`; update 6 existing
  test literal constructions)
- Modify: `adam-lang/src/ast_parser.rs` (6 construction sites gain the new fields, all `false`)
- Modify: `adam-lang/src/trivia.rs` (generalize `attach_trivia`'s gap-walking loop into
  `attach_gaps<T: TriviaTarget>`, called recursively for every nested list)

**Interfaces:**
- Produces (used by Task 4): every `ast::MethodDecl`/`ast::ConditionalBranch` now has `pub
  leading_comment: Option<String>` and `pub blank_line_before: bool`; every `ast::CellDecl`/
  `ast::RelationshipDecl`/`ast::ConditionalDecl`/`ast::SheetItem::Error` now also has `pub
  blank_line_before: bool`. `attach_trivia(source: &str, sheet: &mut Sheet)`'s signature is
  unchanged, but it now also populates these fields on every `RelationshipDecl.methods`,
  `ConditionalDecl.branches`/`default`, and `ConditionalBranch.relationships` entry, not just
  `Sheet.items`.

- [ ] **Step 1: Add the new fields to `adam-lang/src/ast.rs`**

Find:

```rust
    /// A syntax error recovered at declaration granularity; `span` covers the skipped tokens.
    Error {
        /// The span of the skipped, malformed item.
        span: ExprSpan,
        /// A leading `//`/`/* */` comment immediately preceding this item, if recovered by
        /// [`crate::trivia::attach_trivia`]. Preserved even though the item failed to parse, so
        /// a comment explaining a broken declaration (e.g. `// TODO: fix this`) isn't silently
        /// dropped.
        leading_comment: Option<String>,
    },
}
```

Replace with:

```rust
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
```

Find:

```rust
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
}
```

Replace with:

```rust
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
```

Find:

```rust
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
    /// The span of the whole `cell ...;` declaration.
    pub span: ExprSpan,
}
```

Replace with:

```rust
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
```

Find:

```rust
pub struct RelationshipDecl {
    /// The relationship's optional name.
    pub name: Option<(String, ExprSpan)>,
    /// The relationship's methods, in declaration order.
    pub methods: Vec<MethodDecl>,
    /// A leading comment immediately preceding this declaration, if recovered.
    pub leading_comment: Option<String>,
    /// The span of the whole `relationship { ... }` declaration.
    pub span: ExprSpan,
}
```

Replace with:

```rust
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
```

Find:

```rust
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
    /// The span of the whole `conditional ... { ... }` declaration.
    pub span: ExprSpan,
}
```

Replace with:

```rust
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
```

Find:

```rust
/// `conditional_branch = literal "=>" "{" { relationship_decl } "}" [ "," ].`
#[derive(Debug, Clone)]
pub struct ConditionalBranch {
    /// The branch's unresolved match literal.
    pub literal: Literal,
    /// The literal token's span.
    pub literal_span: ExprSpan,
    /// The branch's relationships, in declaration order.
    pub relationships: Vec<RelationshipDecl>,
    /// The span from the branch's literal through its closing `}`.
    pub span: ExprSpan,
}
```

Replace with:

```rust
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
```

Find:

```rust
#[derive(Debug, Clone)]
pub struct MethodDecl {
    /// The method's input cell names (the first `cell_list`).
    pub inputs: Vec<(String, ExprSpan)>,
    /// The method's output cell names (the second `cell_list`).
    pub outputs: Vec<(String, ExprSpan)>,
    /// The parsed method body expression.
    pub body: cel_parser::Expr,
    /// The span of the whole `method [...] -> [...] { ... }` declaration.
    pub span: ExprSpan,
}
```

Replace with:

```rust
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
```

- [ ] **Step 2: Update `ast.rs`'s 6 test literal constructions to include the new fields**

Find each of these six struct literals in `#[cfg(test)] mod tests` and add the field shown
(`blank_line_before: false,` in each case — none of these tests exercise blank-line behavior, so
`false` is the trivial, uninteresting value):

Find:

```rust
        let item = SheetItem::Cell(CellDecl {
            name: "x".to_string(),
            name_span: span,
            type_name: None,
            initializer: None,
            leading_comment: None,
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
}
```

Replace with (every literal gains `blank_line_before: false,`; assertions are unchanged):

```rust
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
```

- [ ] **Step 3: Run `cargo test -p adam-lang` to confirm it fails to compile**

Run: `cargo test -p adam-lang`
Expected: compile errors in `ast_parser.rs` — every `CellDecl`/`RelationshipDecl`/
`ConditionalDecl`/`ConditionalBranch`/`MethodDecl`/`SheetItem::Error` literal it constructs is
now missing the new field(s). Fixed in the next step.

- [ ] **Step 4: Update `adam-lang/src/ast_parser.rs`'s 6 construction sites**

In `parse_sheet`, find:

```rust
                    items.push(ast::SheetItem::Error {
                        span: ast::ExprSpan {
                            start: item_start,
                            end: item_end,
                        },
                        leading_comment: None,
                    });
```

Replace with:

```rust
                    items.push(ast::SheetItem::Error {
                        span: ast::ExprSpan {
                            start: item_start,
                            end: item_end,
                        },
                        leading_comment: None,
                        blank_line_before: false,
                    });
```

In `parse_cell_decl`, find:

```rust
        Ok(ast::CellDecl {
            name,
            name_span: point(name_span),
            type_name,
            initializer,
            leading_comment: None,
            span: ast::ExprSpan {
                start: decl_start,
                end: semi_span,
            },
        })
```

Replace with:

```rust
        Ok(ast::CellDecl {
            name,
            name_span: point(name_span),
            type_name,
            initializer,
            leading_comment: None,
            blank_line_before: false,
            span: ast::ExprSpan {
                start: decl_start,
                end: semi_span,
            },
        })
```

In `parse_relationship_decl`, find:

```rust
        Ok(ast::RelationshipDecl {
            name,
            methods,
            leading_comment: None,
            span: ast::ExprSpan {
                start: decl_start,
                end: close_span,
            },
        })
```

Replace with:

```rust
        Ok(ast::RelationshipDecl {
            name,
            methods,
            leading_comment: None,
            blank_line_before: false,
            span: ast::ExprSpan {
                start: decl_start,
                end: close_span,
            },
        })
```

In `parse_conditional_decl`, find:

```rust
            branches.push(ast::ConditionalBranch {
                literal: lit,
                literal_span: point(lit_span),
                relationships,
                span: ast::ExprSpan {
                    start: lit_span,
                    end: close,
                },
            });
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::ConditionalDecl {
            match_name,
            match_name_span: point(match_span),
            branches,
            default,
            leading_comment: None,
            span: ast::ExprSpan {
                start: decl_start,
                end: close_span,
            },
        })
```

Replace with:

```rust
            branches.push(ast::ConditionalBranch {
                literal: lit,
                literal_span: point(lit_span),
                relationships,
                leading_comment: None,
                blank_line_before: false,
                span: ast::ExprSpan {
                    start: lit_span,
                    end: close,
                },
            });
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::ConditionalDecl {
            match_name,
            match_name_span: point(match_span),
            branches,
            default,
            leading_comment: None,
            blank_line_before: false,
            span: ast::ExprSpan {
                start: decl_start,
                end: close_span,
            },
        })
```

In `parse_method_decl`, find:

```rust
        Ok(ast::MethodDecl {
            inputs,
            outputs,
            body,
            span: ast::ExprSpan {
                start: decl_start,
                end: close_span,
            },
        })
```

Replace with:

```rust
        Ok(ast::MethodDecl {
            inputs,
            outputs,
            body,
            leading_comment: None,
            blank_line_before: false,
            span: ast::ExprSpan {
                start: decl_start,
                end: close_span,
            },
        })
```

- [ ] **Step 5: Run `cargo test -p adam-lang` to confirm `ast`/`ast_parser` compile and pass**

Run: `cargo test -p adam-lang ast:: ast_parser::`
Expected: all pass (no test asserted on the old field shape in a way Step 2/4 didn't already fix).

- [ ] **Step 6: Rewrite `adam-lang/src/trivia.rs` to generalize `attach_trivia`**

Replace the entire file (its module doc, `line_start_byte_offsets`, and `line_column_to_byte`
helpers are unchanged from today — only `attach_trivia` itself, the renamed
`trailing_comment_block` → `analyze_gap`, and the test module change):

```rust
//! Recovers comments and blank-line-before flags discarded/erased by `proc_macro2`'s tokenizer,
//! re-slicing the gap between two consecutive AST nodes' spans (the same technique `rustfmt` uses
//! for the identical problem — see `cel-parser/src/lex_lexer.rs`'s `test_span_preservation`), and
//! attaches each to the nearest following node. Applied recursively to every sibling list in the
//! tree — `Sheet.items`, a `RelationshipDecl`'s `methods`, a `ConditionalDecl`'s `branches` and
//! `default`, and each `ConditionalBranch`'s `relationships` — not just the top level.
//!
//! A comment is attached only if nothing but whitespace-on-the-same-line separates it from the
//! following item — a blank line between an earlier comment and the item breaks the attachment,
//! matching the common convention that a blank line ends a comment's association with what
//! follows. `blank_line_before` is set independently: it reflects whether *any* blank line
//! remained in the gap once the (possibly absent) attached trailing comment run was accounted
//! for, so `cell a;\n\n// c\ncell b;`'s blank line (before the comment, not after it) still marks
//! `b.blank_line_before` true even though the comment still attaches to `b`.
//!
//! A comment or blank line in the gap between a block's *last* item and that block's closing `}`
//! (nothing follows it) is not attached to anything and is dropped — see
//! <https://github.com/stlab/cel-rs/issues/52>.

use proc_macro2::LineColumn;

use crate::ast::{ConditionalBranch, ConditionalDecl, ExprSpan, MethodDecl, RelationshipDecl, Sheet};

/// An AST node that can carry recovered leading trivia, attached by [`attach_gaps`].
trait TriviaTarget {
    fn span(&self) -> ExprSpan;
    fn set_leading_comment(&mut self, comment: String);
    fn set_blank_line_before(&mut self, value: bool);
}

impl TriviaTarget for crate::ast::SheetItem {
    fn span(&self) -> ExprSpan {
        crate::ast::SheetItem::span(self)
    }
    fn set_leading_comment(&mut self, comment: String) {
        crate::ast::SheetItem::set_leading_comment(self, comment)
    }
    fn set_blank_line_before(&mut self, value: bool) {
        crate::ast::SheetItem::set_blank_line_before(self, value)
    }
}

impl TriviaTarget for MethodDecl {
    fn span(&self) -> ExprSpan {
        self.span
    }
    fn set_leading_comment(&mut self, comment: String) {
        self.leading_comment = Some(comment);
    }
    fn set_blank_line_before(&mut self, value: bool) {
        self.blank_line_before = value;
    }
}

impl TriviaTarget for RelationshipDecl {
    fn span(&self) -> ExprSpan {
        self.span
    }
    fn set_leading_comment(&mut self, comment: String) {
        self.leading_comment = Some(comment);
    }
    fn set_blank_line_before(&mut self, value: bool) {
        self.blank_line_before = value;
    }
}

impl TriviaTarget for ConditionalBranch {
    fn span(&self) -> ExprSpan {
        self.span
    }
    fn set_leading_comment(&mut self, comment: String) {
        self.leading_comment = Some(comment);
    }
    fn set_blank_line_before(&mut self, value: bool) {
        self.blank_line_before = value;
    }
}

/// Recovers comments/blank-lines from every gap in `sheet` — its own top-level items, and every
/// nested `relationship`/`conditional` body — attaching each to the nearest following node.
///
/// - Precondition: `sheet` was parsed from exactly `source` (unmodified), so its items' spans'
///   line/column positions resolve correctly against it.
///
/// - Complexity: O(n) in the length of `source` plus the number of nested lists — every gap's
///   `LineColumn -> byte offset` conversion reuses the shared `line_starts` table computed once
///   up front (see [`line_start_byte_offsets`]), rather than rescanning `source` per gap.
pub fn attach_trivia(source: &str, sheet: &mut Sheet) {
    let line_starts = line_start_byte_offsets(source);
    attach_gaps(source, &line_starts, &mut sheet.items);
    for item in &mut sheet.items {
        match item {
            crate::ast::SheetItem::Relationship(rel) => {
                attach_relationship(source, &line_starts, rel)
            }
            crate::ast::SheetItem::Conditional(cond) => {
                attach_conditional(source, &line_starts, cond)
            }
            crate::ast::SheetItem::Cell(_) | crate::ast::SheetItem::Error { .. } => {}
        }
    }
}

fn attach_relationship(source: &str, line_starts: &[usize], rel: &mut RelationshipDecl) {
    attach_gaps(source, line_starts, &mut rel.methods);
}

fn attach_conditional(source: &str, line_starts: &[usize], cond: &mut ConditionalDecl) {
    attach_gaps(source, line_starts, &mut cond.branches);
    for branch in &mut cond.branches {
        attach_gaps(source, line_starts, &mut branch.relationships);
        for rel in &mut branch.relationships {
            attach_relationship(source, line_starts, rel);
        }
    }
    if let Some(default) = &mut cond.default {
        attach_gaps(source, line_starts, default);
        for rel in default.iter_mut() {
            attach_relationship(source, line_starts, rel);
        }
    }
}

/// Recovers comments/blank-lines from the gaps between consecutive `items`, attaching each to the
/// nearest following item. The first item in `items` never gets a blank-line-before or comment —
/// nothing in this list precedes it (a blank line or comment between it and this list's own
/// enclosing `{` is a separate, untracked case; see the module doc's linked issue).
fn attach_gaps<T: TriviaTarget>(source: &str, line_starts: &[usize], items: &mut [T]) {
    if items.len() < 2 {
        return;
    }
    for i in 1..items.len() {
        let start = line_column_to_byte(source, line_starts, items[i - 1].span().end.end());
        let end = line_column_to_byte(source, line_starts, items[i].span().start.start());
        let gap_text = &source[start..end];
        let (comment, blank_line_before) = analyze_gap(gap_text);
        items[i].set_blank_line_before(blank_line_before);
        if let Some(comment) = comment {
            items[i].set_leading_comment(comment);
        }
    }
}

/// Returns the byte offset of the start of each line in `source`: `result[line - 1]` is the
/// start of 1-based line `line` (matching [`proc_macro2::LineColumn::line`]'s convention).
///
/// - Complexity: O(n) in the length of `source`.
fn line_start_byte_offsets(source: &str) -> Vec<usize> {
    let mut offsets = vec![0usize];
    let mut byte = 0usize;
    for line in source.split_inclusive('\n') {
        byte += line.len();
        offsets.push(byte);
    }
    offsets
}

/// Converts a [`LineColumn`] (1-based line, 0-based character column) to a byte offset in
/// `source`, using `line_starts` (from [`line_start_byte_offsets`]) instead of rescanning
/// `source` from byte 0.
///
/// - Precondition: `line_starts` was built from exactly `source`, and `pos` was recorded
///   against `source` (so `pos.line - 1` is in range).
///
/// - Complexity: O(k), where k is `pos.column` — bounded by that one line's length, not the
///   whole of `source`.
fn line_column_to_byte(source: &str, line_starts: &[usize], pos: LineColumn) -> usize {
    let line_start = line_starts[pos.line - 1];
    line_start
        + source[line_start..]
            .chars()
            .take(pos.column)
            .map(char::len_utf8)
            .sum::<usize>()
}

/// Analyzes one gap between two consecutive items: the maximal trailing run of `//` line
/// comments (or a single `/* ... */` block comment) immediately preceding the next item, if any,
/// and whether a blank line remains anywhere in what's left of the gap once that trailing run is
/// accounted for (see the module doc for why the scan order matters).
fn analyze_gap(gap: &str) -> (Option<String>, bool) {
    let mut lines: Vec<&str> = gap.lines().collect();
    // `gap` ends exactly where the following item's first token begins. When that token isn't
    // at column 0, `lines()`'s final entry is only the leading whitespace before it on its own
    // line, not a blank source line — drop that fragment before scanning for a trailing comment
    // run so a real blank line (a genuine empty entry from `lines()`) still breaks the run.
    if !gap.ends_with('\n') {
        lines.pop();
    }
    let mut collected = Vec::new();
    while let Some(line) = lines.last() {
        let trimmed = line.trim();
        if let Some(text) = trimmed.strip_prefix("//") {
            collected.push(text.trim().to_string());
            lines.pop();
        } else if let Some(text) = trimmed
            .strip_prefix("/*")
            .and_then(|s| s.strip_suffix("*/"))
        {
            collected.push(text.trim().to_string());
            lines.pop();
            break; // a block comment is one unit; don't merge with an earlier `//` run
        } else {
            break;
        }
    }
    let comment = if collected.is_empty() {
        None
    } else {
        collected.reverse();
        Some(collected.join("\n"))
    };
    let blank_line_before = lines.iter().any(|l| l.trim().is_empty());
    (comment, blank_line_before)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AdamAstParser;

    #[test]
    fn attaches_a_line_comment_immediately_before_a_cell_decl() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // the total\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment.as_deref(), Some("the total"));
    }

    #[test]
    fn attaches_a_multi_line_comment_block() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // line one\n    // line two\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment.as_deref(), Some("line one\nline two"));
    }

    #[test]
    fn attaches_a_single_line_block_comment() {
        let source =
            "sheet s {\n    cell a: i32 = 1;\n    /* the total */\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment.as_deref(), Some("the total"));
    }

    #[test]
    fn does_not_attach_a_comment_separated_by_a_blank_line() {
        let source =
            "sheet s {\n    cell a: i32 = 1;\n    // stale comment\n\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment, None);
    }

    #[test]
    fn no_comment_in_the_gap_leaves_leading_comment_none() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment, None);
    }

    #[test]
    fn attaches_comments_correctly_across_more_than_one_gap() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // first\n    cell b: i32 = 2;\n    // second\n    cell c: i32 = 3;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment.as_deref(), Some("first"));
        let crate::ast::SheetItem::Cell(c) = &sheet.items[2] else {
            panic!("expected Cell");
        };
        assert_eq!(c.leading_comment.as_deref(), Some("second"));
    }

    #[test]
    fn attaches_a_comment_preceding_a_recovered_error_item() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // fix me\n    cell bad unknown_syntax\n    cell c: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Error {
            leading_comment, ..
        } = &sheet.items[1]
        else {
            panic!("expected Error");
        };
        assert_eq!(leading_comment.as_deref(), Some("fix me"));
    }

    #[test]
    fn recovery_span_that_abuts_the_next_keyword_does_not_invert_the_gap() {
        let source = "sheet s { cell bad relationship { method [x] -> [y] { x } } }";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet); // must not panic
        assert!(matches!(
            sheet.items[0],
            crate::ast::SheetItem::Error { .. }
        ));
        assert!(matches!(
            sheet.items[1],
            crate::ast::SheetItem::Relationship(_)
        ));
    }

    #[test]
    fn sets_blank_line_before_true_when_a_blank_line_separates_two_items() {
        let source = "sheet s {\n    cell a: i32 = 1;\n\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert!(b.blank_line_before);
    }

    #[test]
    fn sets_blank_line_before_false_when_items_are_packed_tight() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert!(!b.blank_line_before);
    }

    #[test]
    fn a_run_of_several_blank_lines_still_just_sets_the_flag_true() {
        let source = "sheet s {\n    cell a: i32 = 1;\n\n\n\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert!(b.blank_line_before);
    }

    #[test]
    fn blank_line_before_an_attached_comment_still_sets_the_flag_true() {
        // The blank line precedes the comment, not the item — the comment still attaches to b
        // (nothing separates the comment itself from b), but the blank line further back in the
        // gap still counts as separating a from (comment + b) as a group.
        let source = "sheet s {\n    cell a: i32 = 1;\n\n    // c\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment.as_deref(), Some("c"));
        assert!(b.blank_line_before);
    }

    #[test]
    fn attaches_a_comment_and_blank_line_to_a_method_inside_a_relationship() {
        let source = "sheet s {\n    relationship {\n        method [a] -> [b] { a }\n\n        // second\n        method [b] -> [a] { b }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert_eq!(rel.methods[1].leading_comment.as_deref(), Some("second"));
        assert!(rel.methods[1].blank_line_before);
    }

    #[test]
    fn attaches_a_comment_to_a_conditional_branch() {
        let source = "sheet s {\n    conditional m {\n        0i32 => { relationship { method [a] -> [b] { a } } }\n        // one\n        1i32 => { relationship { method [a] -> [b] { a } } }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(cond.branches[1].leading_comment.as_deref(), Some("one"));
    }

    #[test]
    fn attaches_a_comment_to_a_relationship_nested_inside_a_conditional_branch() {
        let source = "sheet s {\n    conditional m {\n        0i32 => {\n            relationship { method [a] -> [b] { a } }\n            // second\n            relationship { method [b] -> [a] { b } }\n        }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(
            cond.branches[0].relationships[1].leading_comment.as_deref(),
            Some("second")
        );
    }

    #[test]
    fn attaches_a_comment_to_a_relationship_nested_inside_the_default_branch() {
        let source = "sheet s {\n    conditional m {\n        _ => {\n            relationship { method [a] -> [b] { a } }\n            // second\n            relationship { method [b] -> [a] { b } }\n        }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        let default = cond.default.as_ref().expect("default branch present");
        assert_eq!(default[1].leading_comment.as_deref(), Some("second"));
    }
}
```

- [ ] **Step 7: Run the full `adam-lang` test suite**

Run: `cargo test -p adam-lang`
Expected: every existing test still passes, plus the new blank-line and nested-attachment tests
(15 tests total in `trivia::tests`).

- [ ] **Step 8: Format and lint**

```bash
cargo fmt --all
cargo clippy -p adam-lang --all-targets -- -D warnings
```

- [ ] **Step 9: Commit**

```bash
git add adam-lang/src/ast.rs adam-lang/src/ast_parser.rs adam-lang/src/trivia.rs
git commit -m "$(cat <<'EOF'
feat(adam-lang): generalize trivia attachment to nested blocks; add blank_line_before

attach_trivia now recurses into a RelationshipDecl's methods, a
ConditionalDecl's branches/default, and each ConditionalBranch's
relationships, not just top-level Sheet.items. Alongside
leading_comment, every one of these node types now also records
blank_line_before, needed so the upcoming formatter can collapse runs
of blank lines to at most one without fabricating separators that
were never there.
EOF
)"
```

---

### Task 4: `adam_lang::format_sheet` — the sheet pretty-printer

**Files:**
- Create: `adam-lang/src/fmt.rs`
- Modify: `adam-lang/src/lib.rs` (add `mod fmt;` and `pub use fmt::format_sheet;`)

**Interfaces:**
- Consumes: `cel_parser::format_expr` (Task 2), `crate::ast::*` with its new
  `leading_comment`/`blank_line_before` fields (Task 3), `cel_parser::ExprSpan::start.source_text()`.
- Produces (used by Task 5): `pub fn adam_lang::format_sheet(sheet: &ast::Sheet) -> String`.

- [ ] **Step 1: Write the failing tests**

Create `adam-lang/src/fmt.rs` with only its module doc, imports, and test module:

```rust
//! Pretty-prints an [`crate::ast::Sheet`] back to adam-lang source text: 4-space indentation,
//! opening braces on the same line, `leading_comment`/`blank_line_before` reproduced exactly as
//! [`crate::trivia::attach_trivia`] recovered them, and method bodies/cell initializers delegated
//! to [`cel_parser::format_expr`] (bodies) or re-emitted via `Span::source_text()` directly
//! (initializers/branch-match literals — see the design doc for why no `Literal` value is
//! needed). Conditional branches omit the grammar's optional trailing `,`, matching
//! `begin/assets/demo.adm2`'s existing style.
//!
//! Never called on a sheet with any recorded syntax errors — see `adam-lsp`'s
//! `textDocument/formatting` handler, which refuses to format in that case.

use crate::ast;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AdamAstParser;

    fn format(source: &str) -> String {
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        crate::attach_trivia(source, &mut sheet);
        format_sheet(&sheet)
    }

    #[test]
    fn formats_an_empty_sheet() {
        assert_eq!(format("sheet s {}"), "sheet s {\n}\n");
    }

    #[test]
    fn formats_a_cell_with_type_and_initializer() {
        assert_eq!(
            format("sheet s { cell width: f64 = 1920.0; }"),
            "sheet s {\n    cell width: f64 = 1920.0;\n}\n"
        );
    }

    #[test]
    fn formats_a_cell_with_only_a_type_annotation() {
        assert_eq!(
            format("sheet s { cell area: f64; }"),
            "sheet s {\n    cell area: f64;\n}\n"
        );
    }

    #[test]
    fn formats_a_cell_with_only_an_initializer() {
        assert_eq!(
            format("sheet s { cell mode = 0i32; }"),
            "sheet s {\n    cell mode = 0i32;\n}\n"
        );
    }

    #[test]
    fn packed_cells_stay_packed_and_a_blank_line_before_a_relationship_is_preserved() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    cell b: i32 = 2;\n\n    relationship { method [a] -> [b] { a } }\n}";
        let expected = "sheet s {\n    cell a: i32 = 1;\n    cell b: i32 = 2;\n\n    relationship {\n        method [a] -> [b] { a }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn a_run_of_blank_lines_collapses_to_one() {
        let source = "sheet s {\n    cell a: i32 = 1;\n\n\n\n    cell b: i32 = 2;\n}";
        let expected = "sheet s {\n    cell a: i32 = 1;\n\n    cell b: i32 = 2;\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_named_relationship_with_multiple_methods() {
        let source = "sheet s {\n    relationship r {\n        method [width, height] -> [area] { width * height }\n        method [area, height] -> [width] { area / height }\n    }\n}";
        let expected = "sheet s {\n    relationship r {\n        method [width, height] -> [area] { width * height }\n        method [area, height] -> [width] { area / height }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn preserves_a_comment_on_a_nested_method() {
        let source = "sheet s {\n    relationship {\n        method [a] -> [b] { a }\n\n        // second\n        method [b] -> [a] { b }\n    }\n}";
        let expected = "sheet s {\n    relationship {\n        method [a] -> [b] { a }\n\n        // second\n        method [b] -> [a] { b }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_conditional_with_branches_and_a_default_and_no_trailing_commas() {
        let source = "sheet s {\n    conditional p {\n        0i32 => { relationship { method [a] -> [b] { a } } },\n        _ => { relationship { method [b] -> [a] { b } } },\n    }\n}";
        let expected = "sheet s {\n    conditional p {\n        0i32 => {\n            relationship {\n                method [a] -> [b] { a }\n            }\n        }\n        _ => {\n            relationship {\n                method [b] -> [a] { b }\n            }\n        }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn preserves_a_comment_on_a_conditional_branch() {
        let source = "sheet s {\n    conditional m {\n        0i32 => { relationship { method [a] -> [b] { a } } }\n        // one\n        1i32 => { relationship { method [a] -> [b] { a } } }\n    }\n}";
        let expected = "sheet s {\n    conditional m {\n        0i32 => {\n            relationship {\n                method [a] -> [b] { a }\n            }\n        }\n        // one\n        1i32 => {\n            relationship {\n                method [a] -> [b] { a }\n            }\n        }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn method_body_delegates_precedence_aware_parenthesization_to_cel_parser() {
        let source = "sheet s { relationship { method [a, b] -> [c] { (a + b) * 2i32 } } }";
        let expected = "sheet s {\n    relationship {\n        method [a, b] -> [c] { (a + b) * 2i32 }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn format_is_idempotent_through_a_reparse() {
        let source = "sheet demo {\n    cell a: f64 = 2.0;\n    cell b: f64 = 3.0;\n\n    relationship {\n        method [a, b] -> [c] { a * b }\n    }\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test -p adam-lang fmt::`
Expected: compile error — `cannot find function \`format_sheet\`` — it doesn't exist yet.

- [ ] **Step 3: Implement the formatter**

Add this content **above** the `#[cfg(test)] mod tests { ... }` block already in
`adam-lang/src/fmt.rs`:

```rust
/// 4 spaces per nesting level.
fn indent(depth: usize) -> String {
    "    ".repeat(depth)
}

/// Emits `blank_line_before`/`leading_comment` ahead of an item, if either is present.
fn write_trivia(out: &mut String, blank_line_before: bool, leading_comment: Option<&str>, depth: usize) {
    if blank_line_before {
        out.push('\n');
    }
    if let Some(comment) = leading_comment {
        for line in comment.split('\n') {
            out.push_str(&indent(depth));
            out.push_str("// ");
            out.push_str(line);
            out.push('\n');
        }
    }
}

/// Re-emits a literal's exact original text via its span, falling back to an empty string when
/// none is recoverable — mirrors `cel_parser::fmt`'s identical fallback (see the module doc for
/// why no `Literal` value is needed here).
fn source_text_or_empty(span: ast::ExprSpan) -> String {
    span.start.source_text().unwrap_or_default()
}

fn write_cell_list(out: &mut String, cells: &[(String, ast::ExprSpan)]) {
    out.push('[');
    for (i, (name, _)) in cells.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(name);
    }
    out.push(']');
}

fn write_method(out: &mut String, method: &ast::MethodDecl, depth: usize) {
    write_trivia(
        out,
        method.blank_line_before,
        method.leading_comment.as_deref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str("method ");
    write_cell_list(out, &method.inputs);
    out.push_str(" -> ");
    write_cell_list(out, &method.outputs);
    out.push_str(" { ");
    out.push_str(&cel_parser::format_expr(&method.body));
    out.push_str(" }\n");
}

fn write_relationship(out: &mut String, rel: &ast::RelationshipDecl, depth: usize) {
    write_trivia(
        out,
        rel.blank_line_before,
        rel.leading_comment.as_deref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str("relationship ");
    if let Some((name, _)) = &rel.name {
        out.push_str(name);
        out.push(' ');
    }
    out.push_str("{\n");
    for method in &rel.methods {
        write_method(out, method, depth + 1);
    }
    out.push_str(&indent(depth));
    out.push_str("}\n");
}

fn write_branch_relationships(out: &mut String, relationships: &[ast::RelationshipDecl], depth: usize) {
    out.push_str("{\n");
    for rel in relationships {
        write_relationship(out, rel, depth + 1);
    }
    out.push_str(&indent(depth));
    out.push_str("}\n");
}

fn write_branch(out: &mut String, branch: &ast::ConditionalBranch, depth: usize) {
    write_trivia(
        out,
        branch.blank_line_before,
        branch.leading_comment.as_deref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str(&source_text_or_empty(branch.literal_span));
    out.push_str(" => ");
    write_branch_relationships(out, &branch.relationships, depth);
}

fn write_conditional(out: &mut String, cond: &ast::ConditionalDecl, depth: usize) {
    write_trivia(
        out,
        cond.blank_line_before,
        cond.leading_comment.as_deref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str("conditional ");
    out.push_str(&cond.match_name);
    out.push_str(" {\n");
    for branch in &cond.branches {
        write_branch(out, branch, depth + 1);
    }
    if let Some(default) = &cond.default {
        out.push_str(&indent(depth + 1));
        out.push_str("_ => ");
        write_branch_relationships(out, default, depth + 1);
    }
    out.push_str(&indent(depth));
    out.push_str("}\n");
}

fn write_cell(out: &mut String, cell: &ast::CellDecl, depth: usize) {
    write_trivia(
        out,
        cell.blank_line_before,
        cell.leading_comment.as_deref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str("cell ");
    out.push_str(&cell.name);
    if let Some((type_name, _)) = &cell.type_name {
        out.push_str(": ");
        out.push_str(type_name);
    }
    if let Some((_, span)) = &cell.initializer {
        out.push_str(" = ");
        out.push_str(&source_text_or_empty(*span));
    }
    out.push_str(";\n");
}

fn write_sheet_item(out: &mut String, item: &ast::SheetItem, depth: usize) {
    match item {
        ast::SheetItem::Cell(cell) => write_cell(out, cell, depth),
        ast::SheetItem::Relationship(rel) => write_relationship(out, rel, depth),
        ast::SheetItem::Conditional(cond) => write_conditional(out, cond, depth),
        ast::SheetItem::Error { .. } => {
            unreachable!("format_sheet is only called on a sheet with no recorded syntax errors")
        }
    }
}

/// Pretty-prints `sheet` back to adam-lang source text — see the module doc for the printing
/// rules.
///
/// - Precondition: `sheet` has no recorded syntax errors (`sheet.errors.is_empty()`) — a sheet
///   with a `SheetItem::Error` placeholder cannot be printed back to valid source.
///
/// # Examples
///
/// ```
/// use adam_lang::{AdamAstParser, attach_trivia, format_sheet};
///
/// let source = "sheet s { cell x: i32 = 1; }";
/// let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
/// attach_trivia(source, &mut sheet);
/// assert_eq!(format_sheet(&sheet), "sheet s {\n    cell x: i32 = 1;\n}\n");
/// ```
pub fn format_sheet(sheet: &ast::Sheet) -> String {
    debug_assert!(
        sheet.errors.is_empty(),
        "format_sheet's precondition: no recorded syntax errors"
    );
    let mut out = format!("sheet {} {{\n", sheet.name);
    for item in &sheet.items {
        write_sheet_item(&mut out, item, 1);
    }
    out.push_str("}\n");
    out
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p adam-lang fmt::`
Expected: all 12 tests pass. If a golden-string test fails, compare the actual vs. expected output
character-by-character — indentation/newline mismatches are the most likely culprit; do not loosen
an assertion to make it pass.

- [ ] **Step 5: Wire the new module into `adam-lang/src/lib.rs`**

Find:

```rust
pub mod ast;
mod ast_parser;
mod parser;
mod token_cursor;
mod trivia;
pub mod type_registry;
mod typecheck;

// adam-lang reuses cel_parser::ParseError directly; no new error type is introduced.
// All parse errors carry a proc_macro2::Span for source-location diagnostics.
pub use ast_parser::AdamAstParser;
pub use cel_parser::ParseError;
pub use parser::{AdamParser, ParsedSheet};
pub use trivia::attach_trivia;
pub use type_registry::TypeRegistry;
pub use typecheck::check_sheet;
```

Replace with:

```rust
pub mod ast;
mod ast_parser;
mod fmt;
mod parser;
mod token_cursor;
mod trivia;
pub mod type_registry;
mod typecheck;

// adam-lang reuses cel_parser::ParseError directly; no new error type is introduced.
// All parse errors carry a proc_macro2::Span for source-location diagnostics.
pub use ast_parser::AdamAstParser;
pub use cel_parser::ParseError;
pub use fmt::format_sheet;
pub use parser::{AdamParser, ParsedSheet};
pub use trivia::attach_trivia;
pub use type_registry::TypeRegistry;
pub use typecheck::check_sheet;
```

- [ ] **Step 6: Run the full `adam-lang` test suite and its doc tests**

Run: `cargo test -p adam-lang`
Run: `cargo test --doc -p adam-lang`
Expected: every existing test still passes, plus `fmt`'s 12 new tests and `format_sheet`'s
doctest.

- [ ] **Step 7: Format and lint**

```bash
cargo fmt --all
cargo clippy -p adam-lang --all-targets -- -D warnings
```

- [ ] **Step 8: Commit**

```bash
git add adam-lang/src/fmt.rs adam-lang/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(adam-lang): add format_sheet, the adam-lang structural pretty-printer

Delegates method bodies to cel_parser::format_expr; re-emits cell
initializers and conditional-branch match literals via
Span::source_text() directly (no Literal value needed, since
adam-lang's Literal is just an alias for syn::Lit); reproduces
leading_comment/blank_line_before exactly as attach_trivia recovered
them; omits the grammar's optional trailing comma after a conditional
branch, matching begin/assets/demo.adm2's existing style.
EOF
)"
```

---

### Task 5: `adam-lsp` `textDocument/formatting` wiring

**Files:**
- Modify: `adam-lsp/src/dispatch.rs` (add a `Uri -> String` document store; a
  `textDocument/formatting` request handler; advertise the capability)

**Interfaces:**
- Consumes: `adam_lang::{AdamAstParser, attach_trivia, format_sheet}` (Task 3, Task 4).
- Produces (new, exercised only by this task's own tests): a private `fn
  format_edits(source: &str) -> Vec<lsp_types::TextEdit>` in `dispatch.rs` (not `pub` — reached by
  the test submodule via `super::format_edits`, same as `handle_request`/`main_loop` are already
  private to this module) — pure, no transport knowledge, matching `diagnostics_for_source`'s
  existing split between logic and transport.

- [ ] **Step 1: Write the failing tests**

In `adam-lsp/src/dispatch.rs`, add this new `#[cfg(test)]` module content and these new imports —
the code under test (`format_edits`, the new request handling) doesn't exist yet, so this fails to
compile until Step 2/3:

Find the top-of-file imports:

```rust
use lsp_server::{Connection, Message, Notification as ServerNotification, Response};
use lsp_types::{
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, PublishDiagnosticsParams,
    ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
    notification::{
        DidChangeTextDocument, DidOpenTextDocument, Notification as _, PublishDiagnostics,
    },
};

use crate::diagnostics::diagnostics_for_source;

/// The JSON-RPC "Method not found" error code, reused by LSP for unhandled request methods.
const METHOD_NOT_FOUND: i32 = -32601;
```

Replace with:

```rust
use std::collections::HashMap;

use adam_lang::{AdamAstParser, attach_trivia, format_sheet};
use lsp_server::{Connection, Message, Notification as ServerNotification, Request, Response};
use lsp_types::{
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, DocumentFormattingParams,
    OneOf, Position, PublishDiagnosticsParams, Range, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Uri,
    notification::{
        DidChangeTextDocument, DidOpenTextDocument, Notification as _, PublishDiagnostics,
    },
    request::{Formatting, Request as _},
};

use crate::diagnostics::diagnostics_for_source;

/// The JSON-RPC "Method not found" error code, reused by LSP for unhandled request methods.
const METHOD_NOT_FOUND: i32 = -32601;
/// The JSON-RPC "Invalid params" error code, used when `textDocument/formatting`'s params fail
/// to deserialize.
const INVALID_PARAMS: i32 = -32602;
```

Then, in the existing `#[cfg(test)] mod tests` block, find:

```rust
    use lsp_server::{Connection, Message, Notification as ServerNotification, Request, RequestId};
    use lsp_types::{
        DidChangeTextDocumentParams, DidOpenTextDocumentParams, PublishDiagnosticsParams,
        TextDocumentContentChangeEvent, TextDocumentItem, VersionedTextDocumentIdentifier,
        notification::{
            DidChangeTextDocument, DidOpenTextDocument, Notification as _, PublishDiagnostics,
        },
    };

    use super::serve;
```

Replace with (this test module has its own explicit imports, separate from the outer module's —
it does not `use super::*;` — so the new test code below needs everything it references named
here, including the previously-private `format_edits`):

```rust
    use lsp_server::{Connection, Message, Notification as ServerNotification, Request, RequestId};
    use lsp_types::{
        DidChangeTextDocumentParams, DidOpenTextDocumentParams, DocumentFormattingParams,
        PublishDiagnosticsParams, TextDocumentContentChangeEvent, TextDocumentIdentifier,
        TextDocumentItem, TextEdit, VersionedTextDocumentIdentifier,
        notification::{
            DidChangeTextDocument, DidOpenTextDocument, Notification as _, PublishDiagnostics,
        },
        request::{Formatting, Request as _},
    };

    use super::{format_edits, serve};

    #[test]
    fn format_edits_is_empty_for_a_syntax_error() {
        assert!(format_edits("not a sheet at all").is_empty());
    }

    #[test]
    fn format_edits_is_empty_for_a_recovered_syntax_error() {
        assert!(format_edits("sheet s { cell x unknown_syntax }").is_empty());
    }

    #[test]
    fn format_edits_returns_one_edit_replacing_the_whole_document() {
        let edits = format_edits("sheet   s{cell x:i32=1;}");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "sheet s {\n    cell x: i32 = 1;\n}\n");
    }

    #[test]
    fn formatting_request_returns_the_edit_for_a_previously_opened_document() {
        let (server, client) = Connection::memory();
        let server_thread = std::thread::spawn(move || serve(&server));
        initialize(&client);

        let uri: lsp_types::Uri = "file:///test.adm2".parse().unwrap();
        client
            .sender
            .send(Message::Notification(ServerNotification::new(
                DidOpenTextDocument::METHOD.to_string(),
                DidOpenTextDocumentParams {
                    text_document: TextDocumentItem {
                        uri: uri.clone(),
                        language_id: "adam-lang".to_string(),
                        version: 1,
                        text: "sheet s { cell x: i32 = 1; }".to_string(),
                    },
                },
            )))
            .unwrap();
        expect_published(&client); // the didOpen's diagnostics notification

        client
            .sender
            .send(Message::Request(Request::new(
                RequestId::from(3),
                Formatting::METHOD.to_string(),
                DocumentFormattingParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                    options: lsp_types::FormattingOptions {
                        tab_size: 4,
                        insert_spaces: true,
                        ..Default::default()
                    },
                    work_done_progress_params: Default::default(),
                },
            )))
            .unwrap();
        let response = match client.receiver.recv().unwrap() {
            Message::Response(r) => r,
            other => panic!("expected a response, got {other:?}"),
        };
        let edits: Vec<TextEdit> =
            serde_json::from_value(response.response_result.unwrap()).unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "sheet s {\n    cell x: i32 = 1;\n}\n");

        shut_down(&client);
        server_thread.join().unwrap().unwrap();
    }
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test -p adam-lsp dispatch::`
Expected: compile error — `cannot find function \`format_edits\`` and `serve` no longer matches
the request-handling behavior these tests assume (it doesn't handle `textDocument/formatting` at
all yet, and no document store exists to look text up from).

- [ ] **Step 3: Implement `format_edits`, the document store, and the request handler**

Add this function anywhere above the test module (e.g. directly below the `publish` function):

```rust
/// Computes the `textDocument/formatting` edit for adam-lang `source`.
///
/// - Postcondition: returns an empty `Vec` if `source` doesn't parse (`AdamAstParser::parse_str`
///   returns `Err`) or parses with any recovered syntax error (`Sheet.errors` non-empty) —
///   refusing to format code it can't fully understand, matching `rustfmt`. Otherwise returns
///   exactly one [`TextEdit`] replacing the whole document with [`format_sheet`]'s output.
fn format_edits(source: &str) -> Vec<TextEdit> {
    let mut parser = AdamAstParser::new();
    let mut sheet = match parser.parse_str(source) {
        Ok(sheet) if sheet.errors.is_empty() => sheet,
        _ => return Vec::new(),
    };
    attach_trivia(source, &mut sheet);
    vec![TextEdit {
        range: whole_document_range(),
        new_text: format_sheet(&sheet),
    }]
}

/// A `Range` guaranteed to cover an entire document regardless of its actual length — LSP
/// clients clamp an out-of-bounds end position to the document's real end, so this avoids
/// needing to compute the exact last line/column of `source` (and getting it wrong for the
/// common trailing-newline edge case).
fn whole_document_range() -> Range {
    Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: u32::MAX,
            character: u32::MAX,
        },
    }
}
```

Now find `serve`'s capabilities:

```rust
pub fn serve(connection: &Connection) -> anyhow::Result<()> {
    let capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        ..Default::default()
    })?;
    connection.initialize(capabilities)?;
    main_loop(connection)
}
```

Replace with:

```rust
pub fn serve(connection: &Connection) -> anyhow::Result<()> {
    let capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        document_formatting_provider: Some(OneOf::Left(true)),
        ..Default::default()
    })?;
    connection.initialize(capabilities)?;
    main_loop(connection)
}
```

Now find `main_loop` and `handle_notification`:

```rust
fn main_loop(connection: &Connection) -> anyhow::Result<()> {
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                let response = Response::new_err(
                    req.id.clone(),
                    METHOD_NOT_FOUND,
                    format!("unhandled method: {}", req.method),
                );
                connection.sender.send(Message::Response(response))?;
            }
            Message::Notification(not) => handle_notification(connection, not)?,
            Message::Response(_) => {}
        }
    }
    Ok(())
}

/// Handles one client notification, publishing fresh diagnostics on `didOpen`/`didChange`.
///
/// # Errors
///
/// Returns `Err` only if sending the resulting `publishDiagnostics` notification fails (a broken
/// transport). A `didOpen`/`didChange` notification whose params fail to deserialize is logged to
/// stderr and skipped rather than propagated, so one malformed client message can't take down the
/// server.
fn handle_notification(connection: &Connection, not: ServerNotification) -> anyhow::Result<()> {
    match not.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let params: DidOpenTextDocumentParams = match not.extract(DidOpenTextDocument::METHOD) {
                Ok(params) => params,
                Err(error) => {
                    eprintln!(
                        "adam-lsp: ignoring malformed {}: {error}",
                        DidOpenTextDocument::METHOD
                    );
                    return Ok(());
                }
            };
            publish(
                connection,
                &params.text_document.uri,
                &params.text_document.text,
            )?;
        }
        DidChangeTextDocument::METHOD => {
            let params: DidChangeTextDocumentParams =
                match not.extract(DidChangeTextDocument::METHOD) {
                    Ok(params) => params,
                    Err(error) => {
                        eprintln!(
                            "adam-lsp: ignoring malformed {}: {error}",
                            DidChangeTextDocument::METHOD
                        );
                        return Ok(());
                    }
                };
            if let Some(change) = params.content_changes.into_iter().last() {
                publish(connection, &params.text_document.uri, &change.text)?;
            }
        }
        _ => {}
    }
    Ok(())
}
```

Replace with:

```rust
fn main_loop(connection: &Connection) -> anyhow::Result<()> {
    let mut documents: HashMap<Uri, String> = HashMap::new();
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                handle_request(connection, &documents, req)?;
            }
            Message::Notification(not) => handle_notification(connection, &mut documents, not)?,
            Message::Response(_) => {}
        }
    }
    Ok(())
}

/// Handles one client request. Only `textDocument/formatting` is implemented; every other method
/// gets a JSON-RPC "Method not found" response (`shutdown` is intercepted earlier, in
/// `main_loop`, and never reaches here).
///
/// # Errors
///
/// Returns `Err` only if sending the response fails (a broken transport).
fn handle_request(
    connection: &Connection,
    documents: &HashMap<Uri, String>,
    req: Request,
) -> anyhow::Result<()> {
    match req.method.as_str() {
        Formatting::METHOD => {
            let id = req.id.clone();
            match req.extract::<DocumentFormattingParams>(Formatting::METHOD) {
                Ok((id, params)) => {
                    let edits = documents
                        .get(&params.text_document.uri)
                        .map(|source| format_edits(source))
                        .unwrap_or_default();
                    connection
                        .sender
                        .send(Message::Response(Response::new_ok(id, edits)))?;
                }
                Err(error) => {
                    connection.sender.send(Message::Response(Response::new_err(
                        id,
                        INVALID_PARAMS,
                        error.to_string(),
                    )))?;
                }
            }
        }
        _ => {
            let response = Response::new_err(
                req.id.clone(),
                METHOD_NOT_FOUND,
                format!("unhandled method: {}", req.method),
            );
            connection.sender.send(Message::Response(response))?;
        }
    }
    Ok(())
}

/// Handles one client notification, publishing fresh diagnostics on `didOpen`/`didChange` and
/// recording the document's current text in `documents` so a later `textDocument/formatting`
/// request can look it up by URI (that request's params carry only a URI, not the text).
///
/// # Errors
///
/// Returns `Err` only if sending the resulting `publishDiagnostics` notification fails (a broken
/// transport). A `didOpen`/`didChange` notification whose params fail to deserialize is logged to
/// stderr and skipped rather than propagated, so one malformed client message can't take down the
/// server.
fn handle_notification(
    connection: &Connection,
    documents: &mut HashMap<Uri, String>,
    not: ServerNotification,
) -> anyhow::Result<()> {
    match not.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let params: DidOpenTextDocumentParams = match not.extract(DidOpenTextDocument::METHOD) {
                Ok(params) => params,
                Err(error) => {
                    eprintln!(
                        "adam-lsp: ignoring malformed {}: {error}",
                        DidOpenTextDocument::METHOD
                    );
                    return Ok(());
                }
            };
            documents.insert(
                params.text_document.uri.clone(),
                params.text_document.text.clone(),
            );
            publish(
                connection,
                &params.text_document.uri,
                &params.text_document.text,
            )?;
        }
        DidChangeTextDocument::METHOD => {
            let params: DidChangeTextDocumentParams =
                match not.extract(DidChangeTextDocument::METHOD) {
                    Ok(params) => params,
                    Err(error) => {
                        eprintln!(
                            "adam-lsp: ignoring malformed {}: {error}",
                            DidChangeTextDocument::METHOD
                        );
                        return Ok(());
                    }
                };
            if let Some(change) = params.content_changes.into_iter().last() {
                documents.insert(params.text_document.uri.clone(), change.text.clone());
                publish(connection, &params.text_document.uri, &change.text)?;
            }
        }
        _ => {}
    }
    Ok(())
}
```

Note: `handle_request`'s `Formatting::METHOD` arm binds `id` twice (once from `req.id.clone()`
before the match, once as `Ok((id, params))`'s tuple element) — the inner one shadows the outer
and is what's actually used on the success path; the outer one is only reached on `Err`. This is
intentional (it's the simplest way to have an id available in both branches) but if `clippy`
flags the outer binding as unused on some toolchain version, prefix it `_id` instead — do not
remove it, since the `Err` branch needs it.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p adam-lsp`
Expected: every existing test still passes, plus the 4 new tests
(`format_edits_is_empty_for_a_syntax_error`, `format_edits_is_empty_for_a_recovered_syntax_error`,
`format_edits_returns_one_edit_replacing_the_whole_document`,
`formatting_request_returns_the_edit_for_a_previously_opened_document`).

- [ ] **Step 5: Run doc tests**

Run: `cargo test --doc -p adam-lsp`
Expected: unchanged from before this task (no new public doc-comment examples were added here).

- [ ] **Step 6: Verify the binary still builds**

Run: `cargo build -p adam-lsp`
Expected: builds with no warnings.

- [ ] **Step 7: Format and lint**

```bash
cargo fmt --all
cargo clippy -p adam-lsp --all-targets -- -D warnings
```

- [ ] **Step 8: Commit**

```bash
git add adam-lsp/src/dispatch.rs
git commit -m "$(cat <<'EOF'
feat(adam-lsp): wire textDocument/formatting to adam_lang::format_sheet

Adds a Uri -> text document store (updated from didOpen/didChange) so
the formatting request -- which carries only a URI, not the document
text -- can look up what to format. Refuses to format (returns no
edits) when the source has any recorded syntax error, matching
rustfmt's behavior of declining to reformat code it can't fully
understand.
EOF
)"
```

---

### Task 6: `editors/vscode-adam-lang` format-on-save default

**Files:**
- Modify: `editors/vscode-adam-lang/package.json` (add a `configurationDefaults` block)
- Modify: `editors/vscode-adam-lang/README.md` (mention format-on-save, if the file has a
  relevant section — see Step 2)

**Interfaces:** None — this task is pure configuration; no TypeScript or Rust code changes.
`vscode-languageclient`'s `LanguageClient` automatically proxies VS Code's document-formatting
command to any server advertising `documentFormattingProvider` (set in Task 5), so no client-side
provider registration code is needed.

- [ ] **Step 1: Add `configurationDefaults` to `package.json`**

In `editors/vscode-adam-lang/package.json`, find:

```json
    "configuration": {
      "title": "adam-lang",
      "properties": {
        "adam-lang.serverPath": {
          "type": "string",
          "default": "",
          "description": "Path to the adam-lsp language server binary. If empty, the extension searches the workspace's Cargo target directory (target/debug, then target/release) and then PATH."
        }
      }
    }
```

Replace with:

```json
    "configuration": {
      "title": "adam-lang",
      "properties": {
        "adam-lang.serverPath": {
          "type": "string",
          "default": "",
          "description": "Path to the adam-lsp language server binary. If empty, the extension searches the workspace's Cargo target directory (target/debug, then target/release) and then PATH."
        }
      }
    },
    "configurationDefaults": {
      "[adam-lang]": {
        "editor.formatOnSave": true
      }
    }
```

- [ ] **Step 2: Check for a README section to update**

Run: `grep -n "Trying it out" editors/vscode-adam-lang/README.md`

If a "Trying it out" (or similarly named manual-verification) section exists, add one sentence
noting that saving a `.adm2` file now formats it automatically (`adam-lsp`'s
`textDocument/formatting`, enabled by this extension's `editor.formatOnSave` default), placed
alongside whatever it already says about diagnostics. If no such section exists, skip this step —
do not invent a new README section structure for one sentence.

- [ ] **Step 3: Validate `package.json`'s JSON syntax**

Run: `node -e "JSON.parse(require('fs').readFileSync('editors/vscode-adam-lang/package.json', 'utf8'))"`
Expected: no output (valid JSON); a `SyntaxError` means the edit introduced a typo — fix it before
proceeding.

- [ ] **Step 4: Compile the extension's TypeScript to confirm nothing else broke**

Run: `cd editors/vscode-adam-lang && npm run compile`
Expected: succeeds with no errors (this task didn't touch any `.ts` file, so this is a sanity
check, not expected to surface anything).

- [ ] **Step 5: Commit**

```bash
git add editors/vscode-adam-lang/package.json editors/vscode-adam-lang/README.md
git commit -m "$(cat <<'EOF'
feat(vscode-adam-lang): enable format-on-save by default for adam-lang

No client-side provider code needed -- vscode-languageclient already
proxies VS Code's document-formatting command to any server
advertising documentFormattingProvider (adam-lsp now does, per the
previous commit). This just contributes the editor.formatOnSave
default for the adam-lang language id.
EOF
)"
```

(If Step 2 found no README section to touch, drop `editors/vscode-adam-lang/README.md` from the
`git add`.)

---

## After all tasks: full check suite

Before considering this phase done, per repo `CLAUDE.md`, run the complete check suite — not just
each crate's own tests:

```bash
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

All must report zero warnings/errors before this is handed off for a PR. Then, per `CLAUDE.md`'s
multi-phase-work convention, add a dated handoff document under `docs/superpowers/` (e.g.
`docs/superpowers/2026-07-29-phase-4-formatter-handoff.md`, matching
`docs/superpowers/2026-07-18-phase-3-handoff.md`'s format) summarizing what's done (the formatter,
end-to-end) and what's left of the parent design doc (Phase 5: hover/goto-def/completion; the four
tracked known-limitation issues #52–#55) before opening a PR.
