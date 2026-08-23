# adam-lang/adam-lsp Filter Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `adam-lsp`'s diagnostics and formatting understand `cell` filter clauses
correctly, and add `filter` to the VS Code TextMate grammar's syntax coloring — closing the gap
where filters work in the runtime-building parser but are invisible to the CST parser the LSP is
built on.

**Architecture:** Give closure literals a real AST shape in `cel-parser` (`Expr::Closure`,
threaded through `AstContext`, `format_expr`, and `check_expr`), then build `adam-lang`'s
`cell_filter` clause (`ast::CellFilter`, its `ast_parser.rs`/`fmt.rs`/`typecheck.rs` support) on
top of that, and finally add the one missing TextMate keyword.

**Tech Stack:** Rust (`cel-parser`, `adam-lang` crates), no new dependencies. TextMate grammar
JSON for `editors/vscode-adam-lang`.

**Spec:** `docs/superpowers/specs/2026-08-22-adam-lang-lsp-filter-support-design.md`

## Global Constraints

- `cargo fmt --all` before every commit (pre-commit hook enforces this).
- `cargo build --workspace` / `cargo test --workspace` must produce zero compiler warnings.
- Every public function needs a contract-style `///` doc comment (Summary / Preconditions /
  Postconditions / Complexity, per `CLAUDE.md`); every non-trivial private function too, per this
  codebase's existing style (see the doc comments already on `check_cell_initializer`,
  `expr_matches_shape`, etc.).
- Unit tests are derived from the contract and public interface only, not the implementation.
- No new heap allocations beyond what's structurally necessary (this plan borrows `&str`/spans
  wherever the existing code already does).

---

## Task 1: `cel-parser`: give closures an AST shape (`Expr::Closure`)

**Files:**
- Modify: `cel-parser/src/ast.rs`
- Modify: `cel-parser/src/lib.rs:100` (re-export)
- Test: inline `#[cfg(test)] mod tests` in `cel-parser/src/ast.rs`

**Interfaces:**
- Produces: `Expr::Closure { params: Vec<ClosureParam>, body: Box<Expr>, span: ExprSpan }`,
  `pub struct ClosureParam { pub name: String, pub name_span: ExprSpan, pub type_expr:
  ClosureParamTypeExpr }`, `pub enum ClosureParamTypeExpr { Named(String, ExprSpan),
  Tuple(Vec<ClosureParamTypeExpr>, ExprSpan) }` — all re-exported at the crate root
  (`cel_parser::{ClosureParam, ClosureParamTypeExpr}`, `Expr` unchanged in name).

- [ ] **Step 1: Write the failing test**

In `cel-parser/src/ast.rs`'s `mod tests`, add (near `span_returns_the_range_stored_on_a_composite_variant`):

```rust
    #[test]
    fn closure_span_is_the_stored_span() {
        let target = ExprSpan {
            start: Span::call_site(),
            end: Span::call_site(),
        };
        let expr = Expr::Closure {
            params: Vec::new(),
            body: Box::new(Expr::Literal {
                value: Literal::I32(1),
                span: target,
            }),
            span: target,
        };
        assert_eq!(format!("{:?}", expr.span()), format!("{target:?}"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cel-parser closure_span_is_the_stored_span`
Expected: FAIL to compile — `Expr::Closure` does not exist yet.

- [ ] **Step 3: Add the variant and its supporting types**

In `cel-parser/src/ast.rs`, add a new `Closure` variant to the `Expr` enum (after `Cast`):

```rust
    /// A closure literal (`|x: i32| x + 1`, or `|| 1i32` with no parameters).
    Closure {
        /// The closure's declared parameters, in source order.
        params: Vec<ClosureParam>,
        /// The closure's body expression.
        body: Box<Expr>,
        /// The span of the whole closure literal, from its opening `|`/`||` through `body`.
        span: ExprSpan,
    },
```

Add `| Expr::Closure { span, .. }` to the `Expr::span()` match arm chain (it's one big
`|`-separated pattern ending in `=> *span`).

Immediately after the `impl Expr { ... }` block (before `to_literal`), add:

```rust
/// One `closure_param = identifier ":" closure_type_expression` — a closure literal's declared
/// parameter.
#[derive(Clone, Debug)]
pub struct ClosureParam {
    /// The parameter's declared name.
    pub name: String,
    /// The name token's span.
    pub name_span: ExprSpan,
    /// The parameter's declared, unresolved type.
    pub type_expr: ClosureParamTypeExpr,
}

/// `closure_type_expression = identifier | "(" [ closure_type_expression { "," closure_type_expression } ] ")".`
///
/// Unresolved — mirrors `adam_lang::ast::TypeExpr`'s shape exactly (a bare name, or a
/// recursively-nested tuple), but lives here because closures are a `cel-parser` construct, not
/// an `adam-lang` one. A bare name is only ever a `crate::op_table::builtin_scalar_type` name,
/// already validated during parsing — see `Parser::parse_closure_type_expression`.
#[derive(Clone, Debug)]
pub enum ClosureParamTypeExpr {
    /// A single built-in scalar type name (e.g. `"i32"`, `"bool"`).
    Named(String, ExprSpan),
    /// A (possibly nested) tuple of parameter types — `Vec::new()` for `()`. Note: unlike
    /// `adam_lang::ast::TypeExpr::Tuple`, this production has no dedicated 1-element form; see
    /// `Parser::parse_closure_type_expression`'s doc comment (added in Task 2).
    Tuple(Vec<ClosureParamTypeExpr>, ExprSpan),
}
```

In `cel-parser/src/lib.rs:100`, change:

```rust
pub use ast::{AstContext, Expr, ExprSpan, Literal, LogicalOp};
```

to:

```rust
pub use ast::{AstContext, ClosureParam, ClosureParamTypeExpr, Expr, ExprSpan, Literal, LogicalOp};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p cel-parser closure_span_is_the_stored_span`
Expected: PASS

- [ ] **Step 5: Run the full `cel-parser` suite to check for exhaustive-match breakage**

Run: `cargo build -p cel-parser 2>&1 | grep -A3 "non-exhaustive"`
Expected: two hits — `cel-parser/src/fmt.rs`'s `render` function and `cel-parser/src/ty.rs`'s
`check_expr` function both match on `Expr` exhaustively and now need a `Closure` arm. These are
fixed in Tasks 3 and 4 respectively; for this task only, confirm there are no *other* exhaustive
matches over `Expr` elsewhere in `cel-parser` or `adam-lang` (there shouldn't be — `AstContext`'s
own methods build nodes, they don't match on the enum).

- [ ] **Step 6: Commit**

```bash
git add cel-parser/src/ast.rs cel-parser/src/lib.rs
git commit -m "feat(cel-parser): add Expr::Closure AST shape"
```

---

## Task 2: `cel-parser`: parse closures into `Expr::Closure` (the coupled refactor)

This task changes `ParserContext::push_closure`'s signature, both of its implementations, and
the shared grammar production that calls it, all at once — they must move together to keep the
crate compiling at every step, so this task has one larger "implement" step rather than several
small ones.

**Files:**
- Modify: `cel-parser/src/parser_context.rs` (trait + both impls + the one existing test)
- Modify: `cel-parser/src/ast.rs` (new `AstContext::push_closure` impl)
- Modify: `cel-parser/src/lib.rs:1333-1422` (`is_closure_expression`, `parse_closure_type_expression`)
- Test: inline `#[cfg(test)] mod tests` in `cel-parser/src/lib.rs`

**Interfaces:**
- Consumes: `Expr::Closure`/`ClosureParam`/`ClosureParamTypeExpr` from Task 1.
- Produces: `Parser::<AstContext>::new(..).parse_str_ast("|x: i32| x")` now returns
  `Ok(Expr::Closure { .. })` instead of an "unsupported" error.

- [ ] **Step 1: Write the failing tests**

In `cel-parser/src/lib.rs`'s test module (near the other `AstContext`-based parse tests, e.g.
alongside `closure_literal_with_one_param_compiles_and_calls`), add:

```rust
    #[test]
    fn ast_context_parses_a_one_param_closure() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("|x: i32| x").unwrap();
        let Expr::Closure { params, body, .. } = expr else {
            panic!("expected Closure");
        };
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "x");
        assert!(matches!(
            &params[0].type_expr,
            ClosureParamTypeExpr::Named(n, _) if n == "i32"
        ));
        assert!(matches!(*body, Expr::Ident { ref name, .. } if name == "x"));
    }

    #[test]
    fn ast_context_parses_a_zero_param_closure() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("|| 1i32").unwrap();
        let Expr::Closure { params, .. } = expr else {
            panic!("expected Closure");
        };
        assert!(params.is_empty());
    }

    #[test]
    fn ast_context_parses_a_tuple_typed_closure_param() {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast("|x: (i32, f64)| x.0").unwrap();
        let Expr::Closure { params, .. } = expr else {
            panic!("expected Closure");
        };
        match &params[0].type_expr {
            ClosureParamTypeExpr::Tuple(elements, _) => assert_eq!(elements.len(), 2),
            other => panic!("expected Tuple, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-parser ast_context_parses`
Expected: FAIL — `is_closure_expression` currently calls `self.context.push_closure(...)` on
`AstContext`, which hits the trait's default `Err("closures are not supported in this
context")`.

- [ ] **Step 3: Implement — restructure `push_closure` and the closure grammar**

In `cel-parser/src/parser_context.rs`, add `use crate::ast::ClosureParam;` to the top-of-file
`use` block, then replace the trait's `push_closure` (currently at line 136) with:

```rust
    /// Packages a fully-parsed, independent nested context — the body of a closure literal — as
    /// a value pushed onto `self`, given the closure's declared parameter types in two parallel
    /// forms: `param_types` (each parameter's runtime `TypeId`, in order) for an implementation
    /// that executes, and `params` (each parameter's name, span, and unresolved type expression)
    /// for one that builds an AST instead. An implementation uses whichever it needs and ignores
    /// the other.
    ///
    /// The default implementation reports closures as unsupported, so a `ParserContext`
    /// implementation that has no use for them needs no changes to keep compiling.
    ///
    /// - Precondition: `body` was built via `Self::new_context()` and its own argument-binding
    ///   mechanism, in the same style [`Self::new_context`]'s other consumers already use.
    ///
    /// # Errors
    ///
    /// Returns `Err` if this `ParserContext` implementation doesn't support closures, or (for an
    /// implementation that validates during parsing) if `body` is otherwise unsuitable — e.g.
    /// [`DynSegmentContext`] rejects a `body` that doesn't produce exactly one value.
    fn push_closure(
        &mut self,
        param_types: Vec<std::any::TypeId>,
        params: Vec<ClosureParam>,
        body: Self,
        span: Span,
    ) -> crate::Result<()> {
        let _ = (param_types, params, body);
        Err(crate::ParseError::new_range(
            "closures are not supported in this context".to_string(),
            span,
            span,
        ))
    }
```

Replace `DynSegmentContext::push_closure` (currently at line 295) with:

```rust
    fn push_closure(
        &mut self,
        param_types: Vec<std::any::TypeId>,
        params: Vec<ClosureParam>,
        body: Self,
        span: Span,
    ) -> crate::Result<()> {
        let _ = params;
        let return_type = body.output_type_id().ok_or_else(|| {
            crate::ParseError::new_range(
                "closure body must produce exactly one value".to_string(),
                span,
                span,
            )
        })?;
        self.0.just(cel_runtime::DynClosure::new(
            param_types,
            return_type,
            body.into_inner(),
        ));
        Ok(())
    }
```

Update the existing test `dyn_segment_context_push_closure_builds_a_callable_closure` (line
~454) to the new signature:

```rust
    #[test]
    fn dyn_segment_context_push_closure_builds_a_callable_closure() {
        let mut outer = DynSegmentContext::new_context();
        let mut body = DynSegmentContext::new_context();
        body.0.push_arg::<i32>(0);
        body.0.op1(|x: i32| x + 1).unwrap();

        outer
            .push_closure(vec![TypeId::of::<i32>()], Vec::new(), body, Span::call_site())
            .unwrap();

        let closure: cel_runtime::DynClosure = outer.into_inner().call0().unwrap();
        let x = 5i32;
        assert_eq!(closure.call::<i32>(&[&x]).unwrap(), 6);
    }
```

In `cel-parser/src/ast.rs`, add a `push_closure` method to `AstContext`'s `impl ParserContext for
AstContext` block (after `apply_cast`):

```rust
    fn push_closure(
        &mut self,
        param_types: Vec<std::any::TypeId>,
        params: Vec<ClosureParam>,
        body: Self,
        span: Span,
    ) -> crate::Result<()> {
        let _ = param_types;
        let body_expr = body.into_expr();
        let end = body_expr.span().end;
        self.values.push(Expr::Closure {
            params,
            body: Box::new(body_expr),
            span: ExprSpan { start: span, end },
        });
        Ok(())
    }
```

In `cel-parser/src/lib.rs`, replace `is_closure_expression` (line 1333) with:

```rust
    /// `closure_expression = ("||" | "|" [ closure_param { "," closure_param } ] "|") expression .`
    /// `closure_param = identifier ":" closure_type_expression .`
    ///
    /// Compiles the body as a fully independent nested context (via
    /// [`parse_nested_context`](Self::parse_nested_context)) whose only visible names are its
    /// own declared parameters plus whatever library/built-in functions are always reachable —
    /// [`OpLookup::isolate_scopes`] hides every other transient scope (including one an enclosing
    /// caller, e.g. adam-lang, pushed around this whole parse) for the duration of the body
    /// parse, so a closure never resolves a free variable from its lexical surroundings.
    ///
    /// Each parameter's declared type is threaded through in two parallel forms: the existing
    /// runtime-facing `ClosureParamType` (which `DynSegmentContext`'s `push_closure` needs a
    /// concrete `TypeId` from) and the unresolved, span-carrying `ClosureParamTypeExpr` (which
    /// `AstContext`'s needs instead) — both built from the same tokens in the same
    /// [`parse_closure_type_expression`](Self::parse_closure_type_expression) call, so nothing
    /// is parsed twice.
    ///
    /// - Precondition: the opening `|` (`params_already_closed == false`) or the combined `||`
    ///   token naming an empty parameter list (`params_already_closed == true`) has already been
    ///   consumed by [`is_primary_expression`](Self::is_primary_expression); `self.last_span` is
    ///   its span.
    ///
    /// # Errors
    ///
    /// Returns an error if a parameter name, its `:`, its type, or the closing `|` is malformed
    /// or missing; if a parameter's type names an unrecognized type; if the body expression is
    /// missing or malformed; or if this `ParserContext` implementation's `push_closure` rejects
    /// the closure (e.g. `DynSegmentContext` when the body doesn't produce exactly one value).
    ///
    /// - Postcondition: Returns `Ok(true)` on success; `Ok(false)` is never returned.
    fn is_closure_expression(&mut self, params_already_closed: bool) -> Result<bool> {
        let start_span = self.last_span;
        let mut params: Vec<(String, Span, ClosureParamType, ClosureParamTypeExpr)> = Vec::new();
        if !params_already_closed {
            loop {
                let name = self.expect_identifier("expected closure parameter name")?;
                let name_span = self.last_span;
                if !self.is_punctuation(":") {
                    return Err(self.error_at("expected ':' after closure parameter name"));
                }
                let (ty, ty_ast) = self.parse_closure_type_expression()?;
                params.push((name, name_span, ty, ty_ast));
                if self.is_punctuation(",") {
                    continue;
                }
                break;
            }
            if !self.is_punctuation("|") {
                return Err(self.error_at("expected ',' or closing '|'"));
            }
        }

        let param_types: Vec<TypeId> = params.iter().map(|(_, _, ty, _)| ty.type_id()).collect();
        let ast_params: Vec<ClosureParam> = params
            .iter()
            .map(|(name, name_span, _, ty_ast)| ClosureParam {
                name: name.clone(),
                name_span: ExprSpan {
                    start: *name_span,
                    end: *name_span,
                },
                type_expr: ty_ast.clone(),
            })
            .collect();
        let isolated = self.op_lookup.isolate_scopes();
        let param_table: HashMap<String, (usize, ClosureParamType)> = params
            .into_iter()
            .enumerate()
            .map(|(idx, (name, _, ty, _))| (name, (idx, ty)))
            .collect();
        self.op_lookup
            .push_scope(move |name, segment, arity, _span| {
                if arity != 0 {
                    return Ok(false);
                }
                let Some((idx, ty)) = param_table.get(name) else {
                    return Ok(false);
                };
                match ty {
                    ClosureParamType::Scalar(scalar) => (scalar.push_arg)(segment, *idx),
                    ClosureParamType::Tuple(elements) => segment
                        .push_arg_as_dynamic_sequence_tuple(*idx, elements_to_associated(elements)),
                }
                Ok(true)
            });

        let body_result = self.parse_nested_context(|p| p.is_or_expression());
        self.op_lookup.pop_scope();
        self.op_lookup.restore_scopes(isolated);
        let body = body_result?;

        self.context
            .push_closure(param_types, ast_params, body, start_span)?;
        Ok(true)
    }
```

Replace `parse_closure_type_expression` (line 1397) with:

```rust
    /// `closure_type_expression = identifier | "(" [ closure_type_expression { "," closure_type_expression } ] ")" .`
    ///
    /// Builds both the runtime-facing `ClosureParamType` and the unresolved, span-carrying
    /// `ClosureParamTypeExpr` from the same tokens in one pass (see
    /// [`is_closure_expression`](Self::is_closure_expression)'s doc comment for why both are
    /// needed). Note this production has no 1-element-tuple form (unlike
    /// `adam_lang::ast::TypeExpr`): the element loop here continues on a trailing `,` rather
    /// than treating one as a terminator, so `(i32,)` fails to parse as a closure parameter
    /// type — an existing grammar quirk, unchanged by this addition.
    ///
    /// # Errors
    ///
    /// Returns an error if a bare identifier doesn't name a recognized built-in scalar type, or
    /// if the parenthesized element list is malformed or missing its closing `)`.
    fn parse_closure_type_expression(&mut self) -> Result<(ClosureParamType, ClosureParamTypeExpr)> {
        if let Some(Token::Identifier(ident)) = self.peek_token() {
            let name = ident.to_string();
            self.advance();
            let name_span = self.last_span;
            let scalar = crate::op_table::builtin_scalar_type(&name)
                .ok_or_else(|| self.error_at(&format!("unknown type `{name}`")))?;
            return Ok((
                ClosureParamType::Scalar(scalar),
                ClosureParamTypeExpr::Named(
                    name,
                    ExprSpan {
                        start: name_span,
                        end: name_span,
                    },
                ),
            ));
        }
        if !self.is_open_paren() {
            return Err(self.error_at("expected a type name or '('"));
        }
        let open_span = self.last_span;
        let mut elements = Vec::new();
        let mut element_asts = Vec::new();
        if !self.is_close_paren() {
            loop {
                let (ty, ty_ast) = self.parse_closure_type_expression()?;
                elements.push(ty);
                element_asts.push(ty_ast);
                if self.is_punctuation(",") {
                    continue;
                }
                break;
            }
            if !self.is_close_paren() {
                return Err(self.error_at("expected ',' or closing ')'"));
            }
        }
        let close_span = self.last_span;
        Ok((
            ClosureParamType::Tuple(elements),
            ClosureParamTypeExpr::Tuple(
                element_asts,
                ExprSpan {
                    start: open_span,
                    end: close_span,
                },
            ),
        ))
    }
```

- [ ] **Step 4: Run tests to verify they pass, plus the full regression suite**

Run: `cargo test -p cel-parser`
Expected: PASS, including the three new tests, the updated `DynSegmentContext` test, and every
pre-existing closure test (`closure_literal_with_one_param_compiles_and_calls`,
`closure_literal_with_zero_params_compiles_and_calls`,
`closure_literal_with_two_params_compiles_and_calls_in_order`,
`closure_literal_with_tuple_typed_param_compiles_and_calls`,
`nested_closure_referencing_only_its_own_param_compiles_and_calls`,
`closure_body_referencing_an_undeclared_name_is_a_parse_error`,
`closure_body_cannot_see_an_enclosing_scopes_names`) — none of these exercise `AstContext`, so
their behavior is unchanged.

- [ ] **Step 5: Commit**

```bash
git add cel-parser/src/parser_context.rs cel-parser/src/ast.rs cel-parser/src/lib.rs
git commit -m "feat(cel-parser): parse closure literals into Expr::Closure for AstContext"
```

---

## Task 3: `cel-parser`: pretty-print `Expr::Closure` (`format_expr`)

**Files:**
- Modify: `cel-parser/src/fmt.rs`
- Test: inline `#[cfg(test)] mod tests` in `cel-parser/src/fmt.rs`

**Interfaces:**
- Consumes: `Expr::Closure`, `ClosureParam`, `ClosureParamTypeExpr` from Tasks 1–2.
- Produces: `format_expr(&Expr::Closure { .. })` now returns `"|x: i32| body"` /
  `"|| body"` instead of hitting a non-exhaustive-match compile error.

- [ ] **Step 1: Write the failing tests**

In `cel-parser/src/fmt.rs`'s test module, add:

```rust
    #[test]
    fn closure_with_one_param_reprints_with_its_type() {
        assert_eq!(
            format_expr(&parse("|x: i32| x + 1i32")),
            "|x: i32| x + 1i32"
        );
    }

    #[test]
    fn closure_with_no_params_reprints_with_double_pipe() {
        assert_eq!(format_expr(&parse("|| 1i32")), "|| 1i32");
    }

    #[test]
    fn closure_with_multiple_params_joins_them_with_commas() {
        assert_eq!(
            format_expr(&parse("|x: i32, y: i32| x + y")),
            "|x: i32, y: i32| x + y"
        );
    }

    #[test]
    fn closure_with_a_tuple_typed_param_reprints_the_tuple_type() {
        assert_eq!(
            format_expr(&parse("|x: (i32, f64)| x.0")),
            "|x: (i32, f64)| x.0"
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-parser closure_with`
Expected: FAIL to compile — `render`'s `match expr { ... }` is non-exhaustive without a
`Expr::Closure` arm.

- [ ] **Step 3: Implement**

In `cel-parser/src/fmt.rs`, add a `render_closure_param_type` helper (near the top, after
`render_literal`):

```rust
/// Renders a closure parameter's unresolved type expression, e.g. `"i32"` or `"(i32, f64)"`.
fn render_closure_param_type(type_expr: &crate::ClosureParamTypeExpr) -> String {
    match type_expr {
        crate::ClosureParamTypeExpr::Named(name, _) => name.clone(),
        crate::ClosureParamTypeExpr::Tuple(elements, _) => {
            let inner = elements
                .iter()
                .map(render_closure_param_type)
                .collect::<Vec<_>>()
                .join(", ");
            format!("({inner})")
        }
    }
}
```

In `render`'s `match expr { ... }`, add (after the `Expr::If` arm):

```rust
        Expr::Closure { params, body, .. } => {
            let body_s = format_at(body, Level::OR);
            let text = if params.is_empty() {
                format!("|| {body_s}")
            } else {
                let params_s = params
                    .iter()
                    .map(|p| format!("{}: {}", p.name, render_closure_param_type(&p.type_expr)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("|{params_s}| {body_s}")
            };
            (text, Level::PRIMARY)
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-parser closure_with`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add cel-parser/src/fmt.rs
git commit -m "feat(cel-parser): format_expr support for closure literals"
```

---

## Task 4: `cel-parser`: type-check `Expr::Closure` (`check_expr`)

**Files:**
- Modify: `cel-parser/src/ty.rs`
- Test: inline `#[cfg(test)] mod tests` in `cel-parser/src/ty.rs`

**Interfaces:**
- Consumes: `Expr::Closure`, `ClosureParam`, `ClosureParamTypeExpr` from Tasks 1–2.
- Produces: `check_expr(&Expr::Closure{..}, resolve_ident)` returns `(Ty::Any, diagnostics)` where
  `diagnostics` reflects checking the closure's own body against a resolver that binds its own
  parameter names first (shadowing `resolve_ident`) — the building block
  `adam_lang::typecheck::check_filter` (Task 8) relies on to check a filter's own body.

- [ ] **Step 1: Write the failing tests**

In `cel-parser/src/ty.rs`'s test module, add (near the other `check_expr` tests, using the
existing `point`/`lit_i32`/`op` helpers already defined there):

```rust
    fn closure_param(name: &str, type_name: &str) -> crate::ClosureParam {
        crate::ClosureParam {
            name: name.to_string(),
            name_span: point(proc_macro2::Span::call_site()),
            type_expr: crate::ClosureParamTypeExpr::Named(
                type_name.to_string(),
                point(proc_macro2::Span::call_site()),
            ),
        }
    }

    fn closure(params: Vec<crate::ClosureParam>, body: Expr) -> Expr {
        Expr::Closure {
            params,
            body: Box::new(body),
            span: point(proc_macro2::Span::call_site()),
        }
    }

    #[test]
    fn closure_literal_itself_infers_as_any() {
        let expr = closure(vec![closure_param("x", "i32")], Expr::Ident {
            name: "x".to_string(),
            span: point(proc_macro2::Span::call_site()),
        });
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Any);
        assert!(diags.is_empty());
    }

    #[test]
    fn closure_body_type_error_surfaces_through_the_closure() {
        let body = op("+", vec![
            Expr::Ident {
                name: "x".to_string(),
                span: point(proc_macro2::Span::call_site()),
            },
            lit_str("s"),
        ]);
        let expr = closure(vec![closure_param("x", "i32")], body);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Any, "the closure's own type is always Any");
        assert_eq!(diags.len(), 1, "the body's i32 + String mismatch surfaces");
    }

    #[test]
    fn closure_param_shadows_the_outer_resolver() {
        // Outer resolver claims "x" is a String; the closure's own "x: i32" parameter must win
        // inside the body, so `x + 1i32` type-checks cleanly with no diagnostic.
        let outer_resolver = |_: &str| Ty::String;
        let body = op("+", vec![
            Expr::Ident {
                name: "x".to_string(),
                span: point(proc_macro2::Span::call_site()),
            },
            lit_i32(1),
        ]);
        let expr = closure(vec![closure_param("x", "i32")], body);
        let (_, diags) = check_expr(&expr, &outer_resolver);
        assert!(diags.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-parser closure_`
Expected: FAIL to compile — `check_expr`'s `match expr { ... }` is non-exhaustive without a
`Expr::Closure` arm.

- [ ] **Step 3: Implement**

In `cel-parser/src/ty.rs`, add a `Expr::Closure` arm to `check_expr`'s match (after the `Expr::If`
arm):

```rust
        Expr::Closure { params, body, .. } => {
            let resolve_with_params = |name: &str| -> Ty {
                params
                    .iter()
                    .find(|p| p.name == name)
                    .map(|p| closure_param_ty(&p.type_expr))
                    .unwrap_or_else(|| resolve_ident(name))
            };
            let (_, diagnostics) = check_expr(body, &resolve_with_params);
            (Ty::Any, diagnostics)
        }
```

Add a helper function (near `result_ty_for_op`):

```rust
/// Approximates a closure parameter's declared type as a [`Ty`], for use as the identifier
/// resolver when checking a closure's own body: a tuple-shaped parameter has no `Ty` variant
/// (`Ty` has none) and maps to [`Ty::Any`]; a scalar parameter maps via [`Ty::from_name`] —
/// always `Some` in practice, since a [`crate::ClosureParamTypeExpr::Named`] is only ever built
/// from a name `crate::op_table::builtin_scalar_type` already validated during parsing, the
/// identical name set `Ty::from_name` recognizes.
fn closure_param_ty(type_expr: &crate::ClosureParamTypeExpr) -> Ty {
    match type_expr {
        crate::ClosureParamTypeExpr::Named(name, _) => Ty::from_name(name).unwrap_or(Ty::Any),
        crate::ClosureParamTypeExpr::Tuple(..) => Ty::Any,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-parser closure_`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add cel-parser/src/ty.rs
git commit -m "feat(cel-parser): check_expr support for closure literals"
```

---

## Task 5: `adam-lang`: `CellDecl.filter` / `CellFilter` AST shape

**Files:**
- Modify: `adam-lang/src/ast.rs`
- Test: inline `#[cfg(test)] mod tests` in `adam-lang/src/ast.rs`

**Interfaces:**
- Produces: `ast::CellDecl.filter: Option<ast::CellFilter>`, `pub struct CellFilter { pub
  arg_cells: Vec<(String, ExprSpan)>, pub closure: cel_parser::Expr, pub span: ExprSpan }`.

- [ ] **Step 1: Write the failing test**

In `adam-lang/src/ast.rs`'s test module, add (near `cell_decl_initializer_holds_a_parsed_expr`):

```rust
    #[test]
    fn cell_decl_filter_field_holds_a_cell_filter() {
        let span = point(Span::call_site());
        let cell = CellDecl {
            name: "a".to_string(),
            name_span: span,
            type_name: None,
            initializer: None,
            filter: Some(CellFilter {
                arg_cells: vec![("hi".to_string(), span)],
                closure: cel_parser::Expr::Ident {
                    name: "x".to_string(),
                    span,
                },
                span,
            }),
            leading_comment: None,
            doc_comment: None,
            blank_line_before: false,
            span,
        };
        let filter = cell.filter.as_ref().expect("filter present");
        assert_eq!(filter.arg_cells[0].0, "hi");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p adam-lang cell_decl_filter_field_holds_a_cell_filter`
Expected: FAIL to compile — `CellDecl` has no `filter` field and `CellFilter` doesn't exist; the
other 5 existing `CellDecl { .. }` literals in this file (the pre-existing tests) will also fail
to compile once the field is added but before they're updated (Step 3 fixes all of them
together).

- [ ] **Step 3: Implement**

In `adam-lang/src/ast.rs`, add a `filter` field to `CellDecl` (after `initializer`):

```rust
    /// The `filter` clause, if present.
    pub filter: Option<CellFilter>,
```

Add a new struct after `CellDecl`:

```rust
/// `cell_filter = "filter" [ "(" identifier { "," identifier } ")" ] closure_expression.`
#[derive(Debug, Clone)]
pub struct CellFilter {
    /// The filter's declared argument-cell names, in source order (empty if the `(...)` list
    /// was omitted).
    pub arg_cells: Vec<(String, ExprSpan)>,
    /// The filter's closure literal.
    pub closure: cel_parser::Expr,
    /// The span of the whole `filter ...` clause.
    pub span: ExprSpan,
}
```

Update the 5 pre-existing `CellDecl { .. }` test literals in this file
(`sheet_item_span_reads_the_cell_variant`, `set_leading_comment_sets_the_cell_variant`,
`set_blank_line_before_sets_the_cell_variant`, `cell_decl_type_name_holds_a_nested_tuple_type_expr`,
`cell_decl_initializer_holds_a_parsed_expr`) by adding `filter: None,` to each (right after their
existing `initializer: ...,` line).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-lang --lib ast::`
Expected: PASS — all 6 `CellDecl`-touching tests in `ast.rs` (the 5 updated ones plus the new
one).

- [ ] **Step 5: Commit**

```bash
git add adam-lang/src/ast.rs
git commit -m "feat(adam-lang): add CellFilter and CellDecl.filter to the CST"
```

---

## Task 6: `adam-lang`: parse `cell_filter` in the CST parser

**Files:**
- Modify: `adam-lang/src/ast_parser.rs`
- Modify: `adam-lang/src/lib.rs:11` (grammar doc comment)
- Test: inline `#[cfg(test)] mod tests` in `adam-lang/src/ast_parser.rs`

**Interfaces:**
- Consumes: `ast::CellFilter` from Task 5.
- Produces: `AdamAstParser::parse_str("sheet s { cell a: i32 = 1 filter |x: i32| x; }")` returns a
  `Sheet` whose `CellDecl.filter` is `Some(..)`.

- [ ] **Step 1: Write the failing tests**

In `adam-lang/src/ast_parser.rs`'s test module, add (near `parse_cell_with_explicit_tuple_type`):

```rust
    #[test]
    fn parse_cell_with_a_filter_and_no_arg_list() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell a: i32 = 1 filter |x: i32| x; }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        let filter = cell.filter.as_ref().expect("filter present");
        assert!(filter.arg_cells.is_empty());
        assert!(matches!(filter.closure, Expr::Closure { .. }));
    }

    #[test]
    fn parse_cell_with_a_filter_and_an_arg_list() {
        let sheet = AdamAstParser::new()
            .parse_str(
                "sheet s { cell hi: i32 = 100; cell a: i32 = 1 filter(hi) |x: i32, h: i32| x; }",
            )
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        let filter = cell.filter.as_ref().expect("filter present");
        assert_eq!(filter.arg_cells.len(), 1);
        assert_eq!(filter.arg_cells[0].0, "hi");
    }

    #[test]
    fn parse_cell_without_a_filter_leaves_it_none() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell a: i32 = 1; }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert!(cell.filter.is_none());
    }

    #[test]
    fn recovery_malformed_filter_recovers_at_the_next_sheet_item() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    cell good_before: i32 = 1;
                    cell bad: i32 = 1 filter |x: i32|;
                    cell good_after: i32 = 2;
                }
            "#,
            )
            .unwrap();
        assert_eq!(sheet.errors.len(), 1);
        assert_eq!(sheet.items.len(), 3);
        assert!(matches!(sheet.items[0], ast::SheetItem::Cell(_)));
        assert!(matches!(sheet.items[1], ast::SheetItem::Error { .. }));
        assert!(matches!(sheet.items[2], ast::SheetItem::Cell(_)));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang parse_cell_with_a_filter parse_cell_without_a_filter recovery_malformed_filter`
Expected: FAIL — `cell.filter` doesn't compile yet (`CellDecl` construction in `parse_cell_decl`
doesn't set it, and there is no `filter:` field being populated), or (once `filter` is added by
Step 3) the filter clause simply isn't parsed and the first two tests panic on `.expect("filter
present")`.

- [ ] **Step 3: Implement**

In `adam-lang/src/ast_parser.rs`, update `parse_cell_decl`'s doc comment and body:

```rust
    /// `cell_decl = "cell" identifier cell_type_init [ cell_filter ] ";".`
    fn parse_cell_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::CellDecl> {
        let decl_start = cursor.peek_span();
        cursor.is_keyword("cell");
        let (name, name_span) = cursor.consume_ident()?;
        let (type_name, initializer) = if cursor.consume_punct(":") {
            let type_name = self.parse_type_expr(cursor)?;
            let initializer = if cursor.consume_punct("=") {
                Some(self.parse_cel_or_expression(cursor)?)
            } else {
                None
            };
            (Some(type_name), initializer)
        } else if cursor.consume_punct("=") {
            (None, Some(self.parse_cel_or_expression(cursor)?))
        } else {
            return Err(cursor.err_at("expected `:` or `=` in cell declaration"));
        };
        let filter = if cursor.is_keyword("filter") {
            let filter_start = cursor.last_span();
            Some(self.parse_cell_filter(cursor, filter_start)?)
        } else {
            None
        };
        let semi_span = cursor.expect_punct(";")?;
        Ok(ast::CellDecl {
            name,
            name_span: point(name_span),
            type_name,
            initializer,
            filter,
            leading_comment: None,
            doc_comment: None,
            blank_line_before: false,
            span: ast::ExprSpan {
                start: decl_start,
                end: semi_span,
            },
        })
    }

    /// `cell_filter = "filter" [ "(" identifier { "," identifier } ")" ] closure_expression.`
    ///
    /// - Precondition: the `filter` keyword has already been consumed by the caller; `filter_start`
    ///   is its span.
    fn parse_cell_filter(
        &mut self,
        cursor: &mut TokenCursor,
        filter_start: proc_macro2::Span,
    ) -> Result<ast::CellFilter> {
        let mut arg_cells = Vec::new();
        if cursor.at_open_paren() {
            cursor.expect_open_paren()?;
            loop {
                let (name, span) = cursor.consume_ident()?;
                arg_cells.push((name, point(span)));
                if cursor.consume_punct(",") {
                    continue;
                }
                break;
            }
            cursor.expect_close_paren()?;
        }
        let closure = self.parse_cel_or_expression(cursor)?;
        let closure_end = closure.span().end;
        Ok(ast::CellFilter {
            arg_cells,
            closure,
            span: ast::ExprSpan {
                start: filter_start,
                end: closure_end,
            },
        })
    }
```

In `adam-lang/src/lib.rs:11`, update the grammar doc comment:

```rust
//! cell_decl          = "cell" identifier cell_type_init [ cell_filter ] ";".
//! cell_filter        = "filter" [ "(" identifier { "," identifier } ")" ] closure_expression.
```

(inserted as its own line right after the existing `cell_decl` line, before `cell_type_init`).

- [ ] **Step 4: Run tests to verify they pass, plus the full regression suite**

Run: `cargo test -p adam-lang`
Expected: PASS, including every pre-existing `ast_parser.rs` test (unaffected — `filter` is
optional and every existing fixture omits it).

- [ ] **Step 5: Commit**

```bash
git add adam-lang/src/ast_parser.rs adam-lang/src/lib.rs
git commit -m "feat(adam-lang): parse cell_filter clauses in the CST parser"
```

---

## Task 7: `adam-lang`: pretty-print `cell_filter` (`fmt.rs`)

**Files:**
- Modify: `adam-lang/src/fmt.rs`
- Test: inline `#[cfg(test)] mod tests` in `adam-lang/src/fmt.rs`

**Interfaces:**
- Consumes: `ast::CellFilter` from Task 5, populated by Task 6.
- Produces: `format_sheet` now round-trips a filter-bearing sheet instead of the field being
  silently ignored.

- [ ] **Step 1: Write the failing tests**

In `adam-lang/src/fmt.rs`'s test module, add (near `formats_a_cell_with_an_explicit_tuple_type`):

```rust
    #[test]
    fn formats_a_cell_with_a_filter_and_no_arg_list() {
        assert_eq!(
            format("sheet s { cell a: i32 = 1 filter |x: i32| x; }"),
            "sheet s {\n    cell a: i32 = 1 filter |x: i32| x;\n}\n"
        );
    }

    #[test]
    fn formats_a_cell_with_a_filter_and_an_arg_list() {
        assert_eq!(
            format("sheet s { cell hi: i32 = 100; cell a: i32 = 1 filter(hi) |x: i32, h: i32| x; }"),
            "sheet s {\n    cell hi: i32 = 100;\n    cell a: i32 = 1 filter (hi) |x: i32, h: i32| x;\n}\n"
        );
    }

    #[test]
    fn format_is_idempotent_through_a_reparse_with_a_filter() {
        let source = "sheet s {\n    cell a: i32 = 1 filter |x: i32| x;\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang formats_a_cell_with_a_filter format_is_idempotent_through_a_reparse_with_a_filter`
Expected: FAIL — the filter clause is silently dropped by `write_cell` today, so the actual
output is missing ` filter ...` entirely.

- [ ] **Step 3: Implement**

In `adam-lang/src/fmt.rs`, update `write_cell`'s doc comment and body to add the filter clause
after the initializer, before the trailing `;\n`:

```rust
/// Writes one `cell name[: type][ = initializer][ filter [(args)] closure];` declaration,
/// delegating its type annotation to [`source_text_or_empty`] via `TypeExpr::span()` and its
/// initializer/filter closure to [`cel_parser::format_expr`].
fn write_cell(out: &mut String, cell: &ast::CellDecl, depth: usize) {
    write_trivia(
        out,
        cell.blank_line_before,
        cell.leading_comment.as_ref(),
        depth,
    );
    write_doc_comment(out, "///", cell.doc_comment.as_deref(), depth);
    out.push_str(&indent(depth));
    out.push_str("cell ");
    out.push_str(&cell.name);
    if let Some(type_expr) = &cell.type_name {
        out.push_str(": ");
        out.push_str(&source_text_or_empty(type_expr.span()));
    }
    if let Some(expr) = &cell.initializer {
        out.push_str(" = ");
        out.push_str(&cel_parser::format_expr(expr));
    }
    if let Some(filter) = &cell.filter {
        out.push_str(" filter ");
        if !filter.arg_cells.is_empty() {
            out.push('(');
            for (i, (name, _)) in filter.arg_cells.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(name);
            }
            out.push_str(") ");
        }
        out.push_str(&cel_parser::format_expr(&filter.closure));
    }
    out.push_str(";\n");
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-lang formats_a_cell_with_a_filter format_is_idempotent_through_a_reparse_with_a_filter`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add adam-lang/src/fmt.rs
git commit -m "feat(adam-lang): format_sheet support for cell_filter clauses"
```

---

## Task 8: `adam-lang`: type-check `cell_filter` (`typecheck.rs`)

**Files:**
- Modify: `adam-lang/src/typecheck.rs`
- Test: inline `#[cfg(test)] mod tests` in `adam-lang/src/typecheck.rs`

**Interfaces:**
- Consumes: `ast::CellFilter` (Task 5), `cel_parser::{Expr::Closure, ClosureParamTypeExpr}`
  (Tasks 1–2), `TypeRegistry::resolve`/`display_name`/`entry_by_type_id` (pre-existing).
- Produces: `check_sheet` now reports diagnostics for a filter's undeclared argument cells,
  parameter/return type mismatches — mirroring `adam_lang::parser::AdamParser::parse_cell_filter`'s
  runtime validation, but as non-fatal diagnostics instead of a hard parse error, and with no
  declaration-order constraint (matching every other check in this file).

- [ ] **Step 1: Write the failing tests**

In `adam-lang/src/typecheck.rs`'s test module, add (near
`binding_single_tuple_typed_output_matching_body_has_no_diagnostic`):

```rust
    #[test]
    fn filter_with_matching_types_has_no_diagnostic() {
        let sheet = parse("sheet s { cell a: i32 = 1 filter |x: i32| x; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn filter_with_matching_named_arg_cell_has_no_diagnostic() {
        let sheet = parse(
            "sheet s { cell hi: i32 = 100; cell a: i32 = 1 filter(hi) |x: i32, h: i32| x; }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn filter_undeclared_arg_cell_is_a_diagnostic() {
        let sheet = parse("sheet s { cell a: i32 = 1 filter(nope) |x: i32, h: i32| x; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn filter_first_param_type_mismatch_is_two_diagnostics() {
        // Two independent checks both catch the same root mismatch — the parameter-type check
        // (x: f64 vs. a's declared i32) and the return-type check (body `x` is f64, but `a` is
        // i32) — mirroring this file's existing precedent of not de-duplicating diagnostics from
        // genuinely distinct checks (see
        // `binding_single_tuple_typed_output_if_else_body_element_mismatch_in_each_branch_is_two_diagnostics`).
        let sheet = parse("sheet s { cell a: i32 = 1 filter |x: f64| x; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 2);
    }

    #[test]
    fn filter_named_arg_type_mismatch_is_a_diagnostic() {
        let sheet = parse(
            "sheet s { cell hi: f64 = 100.0; cell a: i32 = 1 filter(hi) |x: i32, h: i32| x; }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn filter_wrong_arity_is_a_diagnostic() {
        let sheet = parse("sheet s { cell a: i32 = 1 filter |x: i32, extra: i32| x; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn filter_tuple_typed_cell_with_matching_shape_has_no_diagnostic() {
        let sheet =
            parse("sheet s { cell a: (i32, f64) = (1, 2.5) filter |x: (i32, f64)| (x.0, x.1); }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn filter_tuple_typed_cell_with_arity_mismatch_is_a_diagnostic() {
        let sheet =
            parse("sheet s { cell a: (i32, f64) = (1, 2.5) filter |x: (i32, f64)| (x.0,); }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang filter_`
Expected: FAIL — `check_sheet` never looks at `CellDecl.filter` today, so every "should be a
diagnostic" test gets `diags.is_empty()` instead.

- [ ] **Step 3: Implement**

In `adam-lang/src/typecheck.rs`, add `cel_parser::ClosureParamTypeExpr` to the top `use`:

```rust
use cel_parser::{ClosureParamTypeExpr, Expr, ExprSpan, Literal, ParseError, Ty, ty::check_expr};
```

In `check_sheet`, compute the cell-only name set once and call the new `check_filter` alongside
`check_cell_initializer`:

```rust
pub fn check_sheet(sheet: &Sheet, registry: &TypeRegistry) -> Vec<ParseError> {
    let mut diagnostics = Vec::new();
    let (cell_types, shapes) = declared_cell_types(sheet, registry);
    let cell_names = declared_cell_names(sheet);
    let resolve = |name: &str| -> Ty { cell_types.get(name).copied().unwrap_or(Ty::Any) };
    for item in &sheet.items {
        match item {
            SheetItem::Cell(cell) => {
                check_cell_initializer(cell, registry, &mut diagnostics);
                check_filter(
                    cell,
                    registry,
                    &cell_names,
                    &cell_types,
                    &shapes,
                    &resolve,
                    &mut diagnostics,
                );
            }
            SheetItem::Relationship(rel) => {
                for binding in &rel.bindings {
                    check_binding(binding, registry, &shapes, &resolve, &mut diagnostics);
                }
            }
            SheetItem::Conditional(cond) => {
                for branch in &cond.branches {
                    for rel in &branch.relationships {
                        for binding in &rel.bindings {
                            check_binding(binding, registry, &shapes, &resolve, &mut diagnostics);
                        }
                    }
                }
                if let Some(default) = &cond.default {
                    for rel in &default.relationships {
                        for binding in &rel.bindings {
                            check_binding(binding, registry, &shapes, &resolve, &mut diagnostics);
                        }
                    }
                }
            }
            SheetItem::Out(out_decl) => {
                check_out(out_decl, registry, &shapes, &resolve, &mut diagnostics)
            }
            SheetItem::Error { .. } => {} // already reported as a syntax error; nothing to type-check
        }
    }
    diagnostics
}
```

Add the new functions (near `check_cell_initializer`):

```rust
/// Every declared `cell`'s name (not `out`s) — used to validate a `filter` clause's argument-cell
/// list, which (mirroring `adam_lang::parser::AdamParser::parse_cell_filter`'s real runtime
/// restriction) may only reference other `cell`s, not `out`s.
fn declared_cell_names(sheet: &Sheet) -> std::collections::HashSet<String> {
    sheet
        .items
        .iter()
        .filter_map(|item| match item {
            SheetItem::Cell(cell) => Some(cell.name.clone()),
            _ => None,
        })
        .collect()
}

/// The expected `TypeShape` for one filter-closure parameter position (the filtered cell itself,
/// or one of its declared argument cells) — `Some` only when a concrete shape is known: a
/// tuple-typed annotation (from `shapes`), or a scalar type either annotated or already resolved
/// to a concrete `Ty` (from `cell_types`, converted via `Ty::type_id`). `None` when nothing is
/// known (an unannotated cell, or a name neither map has an entry for), mirroring `Ty::Any`'s
/// existing "never flagged" leniency elsewhere in this file.
fn expected_shape(
    name: &str,
    cell_types: &std::collections::HashMap<String, Ty>,
    shapes: &std::collections::HashMap<String, TypeShape>,
) -> Option<TypeShape> {
    if let Some(shape) = shapes.get(name) {
        return Some(shape.clone());
    }
    cell_types
        .get(name)
        .and_then(Ty::type_id)
        .map(TypeShape::Named)
}

/// Converts an unresolved closure-parameter type expression into `adam_lang`'s own `TypeExpr`, so
/// it can be resolved via the same `TypeRegistry::resolve` every other type annotation in this
/// crate already uses — `cel_parser::ClosureParamTypeExpr` mirrors `TypeExpr`'s shape exactly (a
/// bare name, or a recursively-nested tuple) but lives in `cel-parser` since closures are that
/// crate's own construct.
fn closure_param_type_expr_to_type_expr(expr: &ClosureParamTypeExpr) -> crate::ast::TypeExpr {
    match expr {
        ClosureParamTypeExpr::Named(name, span) => {
            crate::ast::TypeExpr::Named(name.clone(), *span)
        }
        ClosureParamTypeExpr::Tuple(elements, span) => crate::ast::TypeExpr::Tuple(
            elements
                .iter()
                .map(closure_param_type_expr_to_type_expr)
                .collect(),
            *span,
        ),
    }
}

/// Approximates a closure parameter's declared type as a `Ty`, for use as the identifier resolver
/// when checking a filter closure's own body against its own parameter names: a tuple-shaped
/// parameter has no `Ty` variant and maps to `Ty::Any`; a scalar parameter maps via
/// `Ty::from_name`.
fn closure_param_ty(type_expr: &ClosureParamTypeExpr) -> Ty {
    match type_expr {
        ClosureParamTypeExpr::Named(name, _) => Ty::from_name(name).unwrap_or(Ty::Any),
        ClosureParamTypeExpr::Tuple(..) => Ty::Any,
    }
}

/// Checks one `cell`'s `filter` clause, if present, mirroring
/// `adam_lang::parser::AdamParser::parse_cell_filter`'s runtime validation: each argument-cell
/// name must name an already-declared `cell` (not an `out`, and — unlike the runtime path — with
/// no declaration-order constraint, matching every other check in this file); the closure's own
/// parameter types must line up, in order, with `[this cell's own declared/inferred shape, the
/// first argument cell's shape, the second's, ...]`; and the closure body's checked type must
/// unify with this cell's own declared/inferred shape. A malformed filter closure (not an
/// `Expr::Closure`) is silently skipped — already reported as a recovered syntax error, not a
/// type error.
fn check_filter(
    cell: &CellDecl,
    registry: &TypeRegistry,
    cell_names: &std::collections::HashSet<String>,
    cell_types: &std::collections::HashMap<String, Ty>,
    shapes: &std::collections::HashMap<String, TypeShape>,
    resolve: &impl Fn(&str) -> Ty,
    diagnostics: &mut Vec<ParseError>,
) {
    let Some(filter) = &cell.filter else {
        return;
    };
    for (arg_name, arg_span) in &filter.arg_cells {
        if !cell_names.contains(arg_name) {
            diagnostics.push(ParseError::new_range(
                format!("undeclared cell `{arg_name}`"),
                arg_span.start,
                arg_span.end,
            ));
        }
    }

    let Expr::Closure { params, body, .. } = &filter.closure else {
        return;
    };

    let mut expected: Vec<Option<TypeShape>> =
        vec![expected_shape(&cell.name, cell_types, shapes)];
    for (arg_name, _) in &filter.arg_cells {
        expected.push(expected_shape(arg_name, cell_types, shapes));
    }

    if params.len() != expected.len() {
        diagnostics.push(ParseError::new_range(
            format!(
                "cell `{}`: filter closure takes {} parameter(s), but {} are expected (the \
                 cell's own value, plus its declared argument cells)",
                cell.name,
                params.len(),
                expected.len()
            ),
            filter.span.start,
            filter.span.end,
        ));
        return;
    }

    for (param, expected_shape) in params.iter().zip(&expected) {
        let param_shape = registry
            .resolve(&closure_param_type_expr_to_type_expr(&param.type_expr))
            .ok();
        if let (Some(expected_shape), Some(param_shape)) = (expected_shape, &param_shape)
            && expected_shape != param_shape
        {
            diagnostics.push(ParseError::new_range(
                format!(
                    "cell `{}`: filter closure parameter `{}` is `{}`, but `{}` was expected",
                    cell.name,
                    param.name,
                    registry.display_name(param_shape),
                    registry.display_name(expected_shape),
                ),
                param.name_span.start,
                param.name_span.end,
            ));
        }
    }

    let bound: std::collections::HashMap<&str, Ty> = params
        .iter()
        .map(|p| (p.name.as_str(), closure_param_ty(&p.type_expr)))
        .collect();
    let body_resolve =
        |name: &str| -> Ty { bound.get(name).copied().unwrap_or_else(|| resolve(name)) };

    match &expected[0] {
        Some(shape @ TypeShape::Tuple(_)) => {
            expr_matches_shape(body, shape, registry, &body_resolve, diagnostics);
        }
        Some(TypeShape::Named(type_id)) => {
            let (body_ty, body_diags) = check_expr(body, &body_resolve);
            diagnostics.extend(body_diags);
            let declared = Ty::from_type_id(*type_id);
            if !declared.unifies_with(&body_ty) {
                diagnostics.push(ParseError::new_range(
                    format!(
                        "cell `{}`: filter closure body produces `{}`, but `{}` is declared `{}`",
                        cell.name,
                        body_ty.name(),
                        cell.name,
                        declared.name()
                    ),
                    body.span().start,
                    body.span().end,
                ));
            }
        }
        None => {
            let (_, body_diags) = check_expr(body, &body_resolve);
            diagnostics.extend(body_diags);
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass, plus the full regression suite**

Run: `cargo test -p adam-lang`
Expected: PASS, including every pre-existing `typecheck.rs` test (unaffected — `filter` is
optional and every existing fixture omits it).

- [ ] **Step 5: Commit**

```bash
git add adam-lang/src/typecheck.rs
git commit -m "feat(adam-lang): type-check cell_filter clauses"
```

---

## Task 9: `editors/vscode-adam-lang`: `filter` syntax coloring

**Files:**
- Modify: `editors/vscode-adam-lang/syntaxes/adam-lang.tmLanguage.json`

**Interfaces:**
- None (leaf change; no other task depends on this one).

- [ ] **Step 1: Implement**

In `editors/vscode-adam-lang/syntaxes/adam-lang.tmLanguage.json`, change the
`keyword.declaration.adam-lang` pattern's `match` (currently line 31):

```json
          "match": "\\b(sheet|cell|relationship|conditional|out|require)\\b"
```

to:

```json
          "match": "\\b(sheet|cell|relationship|conditional|out|require|filter)\\b"
```

- [ ] **Step 2: Verify manually**

This project has no automated TextMate grammar test tooling (confirmed:
`editors/vscode-adam-lang/test/` contains only `serverPath.test.ts`, unrelated pure logic). Open
`editors/vscode-adam-lang` in VS Code with the extension running (`F5` / Extension Development
Host, per `editors/vscode-adam-lang/README.md`), open a `.adm2` file containing `cell a: i32 = 1
filter |x: i32| x;`, and confirm `filter` renders in the same color as `cell`/`out`/`require`.

- [ ] **Step 3: Commit**

```bash
git add editors/vscode-adam-lang/syntaxes/adam-lang.tmLanguage.json
git commit -m "feat(vscode-adam-lang): color the filter keyword"
```

---

## Task 10: `adam-lsp`: end-to-end filter fixtures

**Files:**
- Modify: `adam-lsp/src/diagnostics.rs`
- Modify: `adam-lsp/src/dispatch.rs`

**Interfaces:**
- Consumes: everything from Tasks 1–8 (no `adam-lsp` source changes — it already calls
  `AdamAstParser`/`check_sheet`/`format_sheet` generically; this task only proves the fix reaches
  the LSP's own public functions).

- [ ] **Step 1: Write the failing tests**

In `adam-lsp/src/diagnostics.rs`'s test module, add (near
`type_mismatched_cell_initializer_is_a_diagnostic`):

```rust
    #[test]
    fn filter_clause_with_matching_types_has_no_diagnostics() {
        assert!(
            diagnostics_for_source("sheet s { cell a: i32 = 1 filter |x: i32| x; }").is_empty()
        );
    }

    #[test]
    fn filter_clause_with_an_undeclared_arg_cell_is_a_diagnostic() {
        let diags =
            diagnostics_for_source("sheet s { cell a: i32 = 1 filter(nope) |x: i32, h: i32| x; }");
        assert_eq!(diags.len(), 1);
    }
```

In `adam-lsp/src/dispatch.rs`'s test module, add (near
`format_edits_returns_one_edit_replacing_the_whole_document`):

```rust
    #[test]
    fn format_edits_formats_a_cell_with_a_filter() {
        let edits = format_edits("sheet s { cell a:i32=1 filter |x:i32| x; }");
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].new_text,
            "sheet s {\n    cell a: i32 = 1 filter |x: i32| x;\n}\n"
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lsp filter_clause format_edits_formats_a_cell_with_a_filter`
Expected: FAIL, *before* Tasks 1–8 land (this task is meant to run last, so in practice these
should already PASS once run — if any fails, it means one of Tasks 1–8's behavior doesn't match
this plan's assumptions and needs revisiting before proceeding).

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p adam-lsp`
Expected: PASS — no source changes needed in this crate; these tests exist purely to pin the
end-to-end behavior at the LSP boundary.

- [ ] **Step 4: Commit**

```bash
git add adam-lsp/src/diagnostics.rs adam-lsp/src/dispatch.rs
git commit -m "test(adam-lsp): add end-to-end filter fixtures"
```

---

## Final verification (before opening a PR)

Run the full check suite per `CLAUDE.md`:

```bash
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

Confirm zero compiler warnings from `build`/`test` (not just clippy), per `CLAUDE.md`'s explicit
requirement.
