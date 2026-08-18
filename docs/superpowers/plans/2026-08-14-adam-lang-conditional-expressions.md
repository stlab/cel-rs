# adam-lang conditional match-expressions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let `adam-lang` parse `conditional <or_expression> { ... }` (e.g. `conditional a && b { ... }`), deducing the match subject's input cells directly from the identifiers the expression references, with no `adam-rs` changes.

**Architecture:** Generalize the existing fixed-index identifier scope (already used to compile method/condition bodies against an explicit `[a, b]` cell list) into a grow-on-demand scope: each identifier reference is checked against already-declared cells, assigned an argument index on first reference, and the index is reused on repeat reference — all within one parse pass that simultaneously compiles the expression. The AST-only parser (`ast_parser.rs`, backing the formatter) and the real Sheet-building parser (`parser.rs`) are independent implementations that don't share code, so they're separate tasks.

**Tech Stack:** Rust, `adam-lang` crate (touches `cel-parser`'s existing public `OpLookup`/`DynSegment` APIs, no changes to those crates). No `adam-rs` changes.

**Spec:** [docs/superpowers/specs/2026-08-14-adam-lang-conditional-expressions-design.md](../specs/2026-08-14-adam-lang-conditional-expressions-design.md)

## Global Constraints

- `cargo fmt --all` before every commit (enforced by pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` (incl. `cargo test --doc --workspace`) must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`, `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`, and `cargo clippy -p begin --all-targets -- -D warnings` must all pass with zero warnings.
- Every public function needs a contract-style `///` doc comment (Summary / Preconditions / `# Errors` / Postconditions / Complexity, as applicable) per the project's CLAUDE.md convention.
- Unit tests are derived from contract and public interface only, not implementation.
- `conditional identifier { ... }` (today's grammar) must keep parsing and behaving identically — it's a degenerate case of the new grammar, not a separate code path.
- No `adam-rs` changes are needed or in scope for this plan (see spec §5/§7).

---

## Task 1: `TypeRegistry` — a per-type dynamic equality comparator

**Files:**
- Modify: `adam-lang/src/type_registry.rs`

**Interfaces:**
- Produces: `TypeEntry.eq_dyn_fn: fn(&dyn Any, &dyn Any) -> bool`, populated by both `TypeRegistry::register`/`register_no_default` — consumed by Task 3 (`parser.rs`'s `Named`-shape match-expression dispatch).

This is a small, fully additive change: a new dispatch-table field, mirroring the existing `call_dyn_fn` field's exact pattern (a generic function monomorphized per registered type, with no captured state, coercing to a bare `fn` pointer).

- [ ] **Step 1: Write the failing test**

Add to `adam-lang/src/type_registry.rs`'s `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn eq_dyn_fn_compares_equal_i32_values_as_equal() {
        let reg = TypeRegistry::new();
        let entry = reg.get("i32").unwrap();
        let a: i32 = 7;
        let b: i32 = 7;
        assert!((entry.eq_dyn_fn)(&a, &b));
    }

    #[test]
    fn eq_dyn_fn_compares_unequal_i32_values_as_unequal() {
        let reg = TypeRegistry::new();
        let entry = reg.get("i32").unwrap();
        let a: i32 = 7;
        let b: i32 = 8;
        assert!(!(entry.eq_dyn_fn)(&a, &b));
    }

    #[test]
    fn register_no_default_also_populates_eq_dyn_fn() {
        #[derive(PartialEq, Clone, Debug)]
        struct NoDefault(i32);

        let mut reg = TypeRegistry::new();
        reg.register_no_default::<NoDefault>("NoDefault");
        let entry = reg.get("NoDefault").unwrap();
        let a = NoDefault(1);
        let b = NoDefault(1);
        let c = NoDefault(2);
        assert!((entry.eq_dyn_fn)(&a, &b));
        assert!(!(entry.eq_dyn_fn)(&a, &c));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-lang eq_dyn_fn --lib`
Expected: FAIL to compile — `no field `eq_dyn_fn` on type `&TypeEntry``.

- [ ] **Step 3: Add the `eq_dyn_impl` helper and the `eq_dyn_fn` field**

In `adam-lang/src/type_registry.rs`, directly above `fn call_dyn_impl`, add:

```rust
/// Compares two type-erased values of `T`, for `TypeEntry::eq_dyn_fn`.
///
/// A generic function monomorphized per registered `T`, with no captured state — this is
/// what lets it coerce to a bare `fn` pointer despite `T` only being known via a runtime
/// `TypeId` at the call site (exactly like `call_dyn_impl` already does for calling a
/// compiled segment).
fn eq_dyn_impl<T: PartialEq + 'static>(a: &dyn Any, b: &dyn Any) -> bool {
    a.downcast_ref::<T>() == b.downcast_ref::<T>()
}
```

Add the field to the `TypeEntry` struct (directly below `call_dyn_fn`):

```rust
    /// Calls `DynSegment::call_dyn::<T>` and boxes the result.
    pub call_dyn_fn: CallDynFn,
    /// Compares two type-erased values of this type for equality.
    pub eq_dyn_fn: fn(&dyn Any, &dyn Any) -> bool,
```

Add `eq_dyn_fn: eq_dyn_impl::<T>,` to both struct-literal blocks inside `TypeRegistry::register` and `TypeRegistry::register_no_default`, directly below their existing `call_dyn_fn: call_dyn_impl::<T>,` lines.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p adam-lang eq_dyn_fn --lib` and `cargo test -p adam-lang register_no_default_also_populates_eq_dyn_fn --lib`
Expected: PASS (3 new tests).

- [ ] **Step 5: Run the full `adam-lang` test suite**

Run: `cargo test -p adam-lang`
Expected: PASS — this change is purely additive, so nothing existing should break.

- [ ] **Step 6: Format and lint**

Run: `cargo fmt --all` then `cargo clippy -p adam-lang --all-targets -- -D warnings`.
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add adam-lang/src/type_registry.rs
git commit -m "feat(adam-lang): add a per-type dynamic equality comparator to TypeRegistry"
```

---

## Task 2: AST-only path — `ast.rs`, `ast_parser.rs`, `fmt.rs`, grammar doc

**Files:**
- Modify: `adam-lang/src/ast.rs`
- Modify: `adam-lang/src/ast_parser.rs`
- Modify: `adam-lang/src/fmt.rs`
- Modify: `adam-lang/src/lib.rs`

**Interfaces:**
- Produces: `ast::ConditionalDecl.match_expr: cel_parser::Expr` (replacing `match_name`/`match_name_span`) — consumed only within this task (`ast_parser.rs`, `fmt.rs`); the real parser (`parser.rs`, Task 3) is an independent implementation that does not use `ast::ConditionalDecl` at all.

This task is fully independent of Tasks 1 and 3 — it doesn't touch `parser.rs` or `type_registry.rs`, and doesn't require a `TypeRegistry` lookup, since it never compiles or resolves cell identifiers (the AST-only parser just records the parsed expression tree).

- [ ] **Step 1: Write the failing tests**

In `adam-lang/src/ast_parser.rs`'s `mod tests`, change:

```rust
    #[test]
    fn parse_conditional_records_branches_and_default() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    conditional mode {
                        0i32 => { relationship { method [width] -> [height] { width } } },
                        _ => { relationship { method [width] -> [height] { width } } },
                    }
                }
            "#,
            )
            .unwrap();
        let ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(cond.match_name, "mode");
        assert_eq!(cond.branches.len(), 1);
        assert!(cond.default.is_some());
    }
```

to:

```rust
    #[test]
    fn parse_conditional_records_branches_and_default() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    conditional mode {
                        0i32 => { relationship { method [width] -> [height] { width } } },
                        _ => { relationship { method [width] -> [height] { width } } },
                    }
                }
            "#,
            )
            .unwrap();
        let ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert!(matches!(&cond.match_expr, Expr::Ident { name, .. } if name == "mode"));
        assert_eq!(cond.branches.len(), 1);
        assert!(cond.default.is_some());
    }

    #[test]
    fn parse_conditional_records_an_expression_match_subject() {
        let sheet = AdamAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    conditional a && b {
                        _ => { relationship { method [width] -> [height] { width } } },
                    }
                }
            "#,
            )
            .unwrap();
        let ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert!(matches!(
            &cond.match_expr,
            Expr::Logical {
                op: cel_parser::LogicalOp::And,
                ..
            }
        ));
    }
```

(`Expr` is already imported in this test module via `use cel_parser::Expr;` at the top of `mod tests` — confirm it's still there; if not, add it.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-lang parse_conditional_records --lib`
Expected: FAIL to compile — `no field `match_expr` on type `&ConditionalDecl`` (and the new test fails the same way).

- [ ] **Step 3: Change `ast::ConditionalDecl`**

In `adam-lang/src/ast.rs`, change:

```rust
pub struct ConditionalDecl {
    /// The name of the cell this conditional matches on.
    pub match_name: String,
    /// The match cell name token's span.
    pub match_name_span: ExprSpan,
    /// The named (literal `=>`) branches, in declaration order.
    pub branches: Vec<ConditionalBranch>,
```

to:

```rust
pub struct ConditionalDecl {
    /// The match subject: an arbitrary expression over already-declared cells (a bare
    /// identifier, e.g. `mode`, is the degenerate single-cell case).
    pub match_expr: Expr,
    /// The named (literal `=>`) branches, in declaration order.
    pub branches: Vec<ConditionalBranch>,
```

(`Expr` here is `cel_parser::Expr` — confirm it's already imported at the top of `ast.rs`; `ConditionDecl.body: Expr` in the same file already uses it, so the import should already be present.)

- [ ] **Step 4: Update `ast_parser.rs`'s `parse_conditional_decl`**

Change:

```rust
    /// `conditional_decl = "conditional" identifier "{" { conditional_branch } [ default_branch ] "}".`
    fn parse_conditional_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::ConditionalDecl> {
        use cel_parser::lex_lexer::Token;
        let decl_start = cursor.peek_span();
        cursor.is_keyword("conditional");
        let (match_name, match_span) = cursor.consume_ident()?;
        cursor.expect_open_brace()?;
```

to:

```rust
    /// `conditional_decl = "conditional" or_expression "{" { conditional_branch } [ default_branch ] "}".`
    fn parse_conditional_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::ConditionalDecl> {
        use cel_parser::lex_lexer::Token;
        let decl_start = cursor.peek_span();
        cursor.is_keyword("conditional");
        let match_expr = self.parse_cel_or_expression(cursor)?;
        cursor.expect_open_brace()?;
```

And change the struct literal at the end of the same function:

```rust
        Ok(ast::ConditionalDecl {
            match_name,
            match_name_span: point(match_span),
            branches,
```

to:

```rust
        Ok(ast::ConditionalDecl {
            match_expr,
            branches,
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p adam-lang parse_conditional_records --lib`
Expected: PASS (2 tests, including the new expression-match-subject test).

- [ ] **Step 6: Update `fmt.rs`'s `write_conditional`**

Change:

```rust
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
```

to:

```rust
fn write_conditional(out: &mut String, cond: &ast::ConditionalDecl, depth: usize) {
    write_trivia(
        out,
        cond.blank_line_before,
        cond.leading_comment.as_deref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str("conditional ");
    out.push_str(&cel_parser::format_expr(&cond.match_expr));
    out.push_str(" {\n");
```

- [ ] **Step 7: Write a failing test for expression-match-subject formatting**

Add to `adam-lang/src/fmt.rs`'s `mod tests`, directly after
`formats_a_conditional_with_branches_and_a_default_and_no_trailing_commas`:

```rust
    #[test]
    fn formats_a_conditional_with_an_expression_match_subject() {
        let source = "sheet s {\n    conditional a && b {\n        _ => { relationship { method [c] -> [d] { c } } },\n    }\n}";
        let expected = "sheet s {\n    conditional a && b {\n        _ => {\n            relationship {\n                method [c] -> [d] { c }\n            }\n        }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }
```

- [ ] **Step 8: Run the fmt tests to verify everything passes**

Run: `cargo test -p adam-lang fmt:: --lib`
Expected: PASS, including `formats_a_conditional_with_branches_and_a_default_and_no_trailing_commas`,
`preserves_a_comment_on_a_conditional_branch`, and `format_is_idempotent_through_a_reparse_with_a_conditional`
unchanged (they all use a bare-identifier match subject, which round-trips identically to before), plus
the new test from Step 7.

- [ ] **Step 9: Update the grammar doc**

In `adam-lang/src/lib.rs`, change:

```rust
//! conditional_decl   = "conditional" identifier "{" { conditional_branch } [ default_branch ] "}".
```

to:

```rust
//! conditional_decl   = "conditional" or_expression "{" { conditional_branch } [ default_branch ] "}".
```

- [ ] **Step 10: Run the full `adam-lang` test suite**

Run: `cargo test -p adam-lang`
Expected: PASS.

- [ ] **Step 11: Format and lint**

Run: `cargo fmt --all` then `cargo clippy -p adam-lang --all-targets -- -D warnings`.
Expected: clean.

- [ ] **Step 12: Commit**

```bash
git add adam-lang/src/ast.rs adam-lang/src/ast_parser.rs adam-lang/src/fmt.rs adam-lang/src/lib.rs
git commit -m "feat(adam-lang): parse and format conditional match-expressions (AST path)"
```

---

## Task 3: The real parser — grow-on-demand scope, compilation, and `Sheet` wiring

**Files:**
- Modify: `adam-lang/src/type_registry.rs` (`AddConditionalFn`/`add_conditional_impl`)
- Modify: `adam-lang/src/parser.rs`
- Modify (conditionally — see Step 12): `begin/examples/image_resize.adm2`, `begin/src/example_source.rs`

**Interfaces:**
- Consumes: `TypeEntry.eq_dyn_fn` (Task 1); `adam_rs::MatchExpr` (already imported in `parser.rs`, from the prior PR's work).
- Produces: `AdamParser::parse_match_expr(&mut self, ctx: &mut ParseContext) -> Result<(TypeShape, MatchExpr)>` and `AdamParser::build_match_expr(&self, segment: DynSegment, inputs: Vec<(String, CellId, TypeShape)>, match_span: proc_macro2::Span) -> Result<(TypeShape, MatchExpr)>` — both private to this task, not consumed elsewhere. `AddConditionalFn`'s type alias changes from `fn(&mut Sheet, CellId, ...)` to `fn(&mut Sheet, MatchExpr, ...)`.

This is the core, highest-risk task — it introduces the grow-on-demand scope mechanism and wires it into `Sheet` construction.

- [ ] **Step 1: Change `AddConditionalFn` and `add_conditional_impl`**

In `adam-lang/src/type_registry.rs`, add `MatchExpr` to the import:

```rust
use adam_rs::{CellId, ConditionalId, MatchExpr, RelationshipId, Sheet};
```

Change:

```rust
/// Calls `Sheet::add_conditional` with the appropriate concrete type.
///
/// Each branch carries a single boxed key value and the `RelationshipId`s active for that
/// branch. The default is a list of `RelationshipId`s active when no branch key matches.
pub type AddConditionalFn = fn(
    &mut Sheet,
    CellId,
    Vec<(Box<dyn Any>, Vec<RelationshipId>)>,
    Vec<RelationshipId>,
) -> Result<ConditionalId, adam_rs::Error>;
```

to:

```rust
/// Calls `Sheet::add_conditional` with the appropriate concrete type.
///
/// Each branch carries a single boxed key value and the `RelationshipId`s active for that
/// branch. The default is a list of `RelationshipId`s active when no branch key matches.
pub type AddConditionalFn = fn(
    &mut Sheet,
    MatchExpr,
    Vec<(Box<dyn Any>, Vec<RelationshipId>)>,
    Vec<RelationshipId>,
) -> Result<ConditionalId, adam_rs::Error>;
```

Change:

```rust
/// Calls `Sheet::add_conditional::<T>` from type-erased branch data.
///
/// - Precondition: each `Box<dyn Any>` in `branches` holds a value of type `T`.
fn add_conditional_impl<T: Any + PartialEq + 'static>(
    sheet: &mut Sheet,
    cell: CellId,
    branches: Vec<(Box<dyn Any>, Vec<RelationshipId>)>,
    default: Vec<RelationshipId>,
) -> Result<ConditionalId, adam_rs::Error> {
    let typed_branches: Vec<(Vec<T>, Vec<RelationshipId>)> = branches
        .into_iter()
        .map(|(val, rel_ids)| {
            let v = *val
                .downcast::<T>()
                .expect("add_conditional_impl: type matches registration");
            (vec![v], rel_ids)
        })
        .collect();
    sheet.add_conditional::<T>(MatchExpr::cell(cell), typed_branches, default)
}
```

to:

```rust
/// Calls `Sheet::add_conditional::<T>` from type-erased branch data.
///
/// - Precondition: each `Box<dyn Any>` in `branches` holds a value of type `T`.
fn add_conditional_impl<T: Any + PartialEq + 'static>(
    sheet: &mut Sheet,
    source: MatchExpr,
    branches: Vec<(Box<dyn Any>, Vec<RelationshipId>)>,
    default: Vec<RelationshipId>,
) -> Result<ConditionalId, adam_rs::Error> {
    let typed_branches: Vec<(Vec<T>, Vec<RelationshipId>)> = branches
        .into_iter()
        .map(|(val, rel_ids)| {
            let v = *val
                .downcast::<T>()
                .expect("add_conditional_impl: type matches registration");
            (vec![v], rel_ids)
        })
        .collect();
    sheet.add_conditional::<T>(source, typed_branches, default)
}
```

- [ ] **Step 2: Run `adam-lang` to confirm the expected compile break in `parser.rs`**

Run: `cargo build -p adam-lang 2>&1 | head -30`
Expected: FAIL — `parser.rs`'s call `add_cond_fn(&mut ctx.sheet, match_cell_id, branches, default_rel_ids)` no longer type-checks (`match_cell_id: CellId`, but `AddConditionalFn` now expects `MatchExpr`). This confirms Step 1 landed and the next steps are necessary.

- [ ] **Step 3: Add imports to `parser.rs`**

Change:

```rust
use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::str::FromStr;
```

to:

```rust
use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
```

- [ ] **Step 4: Write `parse_match_expr` and `build_match_expr`**

Add these two methods to `AdamParser`'s `impl` block in `adam-lang/src/parser.rs`, directly above `fn parse_conditional_decl` (line 462):

```rust
    /// Compiles a conditional's match-subject expression, deducing its input cells from the
    /// identifiers it references — a bare identifier (`mode`) is the degenerate single-cell
    /// case; anything more (`a && b`) draws on however many already-declared cells it
    /// references.
    ///
    /// Each 0-arity identifier lookup that names an already-declared cell is assigned the
    /// next argument index on first reference within this expression and reuses it on repeat
    /// reference (e.g. `a && a` allocates one argument slot, not two), via a scope pushed
    /// onto the CEL operation lookup for the duration of this parse — generalizing the
    /// fixed-index scope `parse_body_with_input_scope` already uses for method/condition
    /// bodies, where the input list is instead explicitly declared.
    ///
    /// # Errors
    /// Returns `Err` if the expression fails to parse, produced no value, or (for a `Named`
    /// output shape) its type isn't registered in the `TypeRegistry`.
    ///
    /// - Complexity: O(k) in the number of distinct cell identifiers referenced, for this
    ///   method's own bookkeeping (on top of `cel-parser`'s own parse cost).
    fn parse_match_expr(&mut self, ctx: &mut ParseContext) -> Result<(TypeShape, MatchExpr)> {
        let match_span = ctx.peek_span();

        // Precompute how to push each currently-declared cell, keyed by name. Built before
        // the scope closure captures anything, since `push_scope` requires `'static` (the
        // closure can't borrow `self.types`).
        let push_table: std::collections::HashMap<String, (CellId, TypeShape, InputPush)> = ctx
            .cell_names
            .iter()
            .map(|(name, (cell_id, shape))| {
                let push = match shape {
                    TypeShape::Named(type_id) => InputPush::Scalar(
                        self.types
                            .entry_by_type_id(*type_id)
                            .expect("declared cell type registered")
                            .push_arg_fn,
                    ),
                    TypeShape::Tuple(_) => {
                        InputPush::Tuple(self.types.associated_prototype(shape))
                    }
                };
                (name.clone(), (*cell_id, shape.clone(), push))
            })
            .collect();

        let accumulator: Arc<Mutex<Vec<(String, CellId, TypeShape)>>> =
            Arc::new(Mutex::new(Vec::new()));
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
                match push {
                    InputPush::Scalar(fn_ptr) => fn_ptr(segment, idx),
                    InputPush::Tuple(associated) => {
                        segment.push_arg_as_dynamic_sequence_tuple(idx, associated.clone())
                    }
                }
                Ok(true)
            });

        let result = self.parse_cel_or_expression(ctx);
        self.cel.op_lookup_mut().pop_scope();
        let segment = result?;

        let inputs = accumulator
            .lock()
            .expect("scope mutex not poisoned")
            .clone();

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
        inputs: Vec<(String, CellId, TypeShape)>,
        match_span: proc_macro2::Span,
    ) -> Result<(TypeShape, MatchExpr)> {
        let input_ids: Vec<CellId> = inputs.iter().map(|(_, id, _)| *id).collect();
        let input_types: Vec<TypeId> = inputs
            .iter()
            .map(|(_, _, shape)| cell_type_id(shape))
            .collect();

        if segment.peek_tuple_arity().is_some() {
            let associated = segment.peek_stack_infos(1)[0].associated.clone();
            let shape = self
                .shape_of_associated(&associated)
                .map_err(|msg| ParseError::new(msg, match_span))?;
            let table = self.types.element_descriptors_for(&shape);
            let segment = RefCell::new(segment);

            let function = move |args: &[&dyn Any]| -> std::result::Result<Box<dyn Any>, anyhow::Error> {
                let leaf = |type_id: TypeId| {
                    table
                        .iter()
                        .find(|(tid, ..)| *tid == type_id)
                        .map(|(_, d, c, e, dbg)| (*d, *c, *e, *dbg))
                };
                let seq = segment.borrow_mut().call_dyn_as_dynamic_sequence(args, &leaf)?;
                Ok(Box::new(seq) as Box<dyn Any>)
            };

            fn dynamic_sequence_eq(a: &dyn Any, b: &dyn Any) -> bool {
                a.downcast_ref::<cel_runtime::DynamicSequence>()
                    == b.downcast_ref::<cel_runtime::DynamicSequence>()
            }

            let match_expr = MatchExpr::new(
                input_ids,
                input_types,
                TypeId::of::<cel_runtime::DynamicSequence>(),
                dynamic_sequence_eq,
                function,
            );
            Ok((shape, match_expr))
        } else {
            let type_id = segment.peek_output_type_id().ok_or_else(|| {
                ParseError::new("match expression produced no value", match_span)
            })?;
            let entry = self.types.entry_by_type_id(type_id).ok_or_else(|| {
                ParseError::new("match expression type not in TypeRegistry", match_span)
            })?;
            let call_fn = entry.call_dyn_fn;
            let eq_fn = entry.eq_dyn_fn;
            let segment = RefCell::new(segment);

            let function = move |args: &[&dyn Any]| -> std::result::Result<Box<dyn Any>, anyhow::Error> {
                call_fn(&mut segment.borrow_mut(), args)
            };

            let match_expr = MatchExpr::new(input_ids, input_types, type_id, eq_fn, function);
            Ok((TypeShape::Named(type_id), match_expr))
        }
    }
```

- [ ] **Step 5: Rewrite `parse_conditional_decl` to use the new methods**

Change:

```rust
    /// `conditional_decl = "conditional" identifier "{" { conditional_branch } [ default_branch ] "}".`
    fn parse_conditional_decl(&mut self, ctx: &mut ParseContext) -> Result<()> {
        ctx.is_keyword("conditional"); // consume
        let (match_name, match_span) = ctx.consume_ident()?;
        let (match_cell_id, match_shape) =
            ctx.cell_names.get(&match_name).cloned().ok_or_else(|| {
                ParseError::new(format!("undeclared cell `{match_name}`"), match_span)
            })?;
        ctx.expect_open_brace()?;
```

to:

```rust
    /// `conditional_decl = "conditional" or_expression "{" { conditional_branch } [ default_branch ] "}".`
    fn parse_conditional_decl(&mut self, ctx: &mut ParseContext) -> Result<()> {
        ctx.is_keyword("conditional"); // consume
        let match_span = ctx.peek_span();
        let (match_shape, match_expr) = self.parse_match_expr(ctx)?;
        ctx.expect_open_brace()?;
```

Change the `Named` dispatch:

```rust
        match &match_shape {
            TypeShape::Named(type_id) => {
                let add_cond_fn: AddConditionalFn = self
                    .types
                    .entry_by_type_id(*type_id)
                    .ok_or_else(|| {
                        ParseError::new("match cell type not in TypeRegistry", match_span)
                    })?
                    .add_conditional_fn;
                add_cond_fn(&mut ctx.sheet, match_cell_id, branches, default_rel_ids)
                    .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
            }
```

to:

```rust
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
                    .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
            }
```

And the `Tuple` dispatch:

```rust
                ctx.sheet
                    .add_conditional::<cel_runtime::DynamicSequence>(
                        MatchExpr::cell(match_cell_id),
                        typed_branches,
                        default_rel_ids,
                    )
                    .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
```

to:

```rust
                ctx.sheet
                    .add_conditional::<cel_runtime::DynamicSequence>(
                        match_expr,
                        typed_branches,
                        default_rel_ids,
                    )
                    .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
```

(`match_expr` is moved into whichever arm of the `match &match_shape { ... }` block runs — since each arm consumes it exactly once and the two arms are mutually exclusive, this compiles without a "use of moved value" error. If the compiler disagrees because `match_shape`'s borrow in the `match &match_shape` scrutinee overlaps `match_expr`'s move, restructure to match on `match_shape` by value — `TypeShape` is `Clone`, so `match match_shape.clone() { ... }` is an acceptable minimal fix; note which one was needed in the task report.)

- [ ] **Step 6: Run `adam-lang` to confirm it builds**

Run: `cargo build -p adam-lang`
Expected: builds cleanly (no more errors from the `AddConditionalFn` signature mismatch).

- [ ] **Step 7: Write the failing tests for the new mechanism**

`adam-lang/src/parser.rs`'s test module already has a `fn parser() -> AdamParser` helper
(`AdamParser::new(TypeRegistry::new(), OpLookup::new())`) and an established convention for
conditional tests — see `parse_conditional_with_tuple_typed_match_cell` (around line 1545) for
the exact pattern to match: `parser().parse_str(...)` into a `let mut sheet = ...`, then
`sheet.propagate().unwrap()`, then `let (id, _) = sheet.cell_names["name"].clone();`, then
`sheet.read::<T>(id)`/`sheet.write(id, value)`. Add these tests directly after
`parse_conditional_with_tuple_typed_match_cell`, following that exact style:

```rust
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
                        true => { relationship { method [x] -> [y] { x } } },
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
                        true => { relationship { method [x] -> [y] { x } } },
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
                        1i32 => { relationship { method [x] -> [y] { x } } },
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

    #[test]
    fn conditional_tuple_expression_match_subject_drives_branch_selection() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 0;
                    cell b: i32 = 0;
                    cell x: i32 = 1;
                    cell y: i32 = 0;
                    conditional (a, b) {
                        (1i32, 2i32) => { relationship { method [x] -> [y] { x } } },
                    }
                }
            "#,
            )
            .unwrap();
        let (a_id, _) = sheet.cell_names["a"].clone();
        let (b_id, _) = sheet.cell_names["b"].clone();
        let (y_id, _) = sheet.cell_names["y"].clone();

        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 0);

        sheet.write(a_id, 1_i32).unwrap();
        sheet.write(b_id, 2_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 1);
    }

    #[test]
    fn conditional_client_registered_type_match_expression_dispatches_correctly() {
        #[derive(PartialEq, Clone, Debug, Default)]
        struct Mode(i32);

        let mut reg = TypeRegistry::new();
        reg.register::<Mode>("Mode");
        let mut sheet = AdamParser::new(reg, OpLookup::new())
            .parse_str(
                r#"
                sheet s {
                    cell m: Mode = Mode(1);
                    cell x: i32 = 1;
                    cell y: i32 = 0;
                    conditional m {
                        Mode(1) => { relationship { method [x] -> [y] { x } } },
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
    fn conditional_expression_referencing_an_undeclared_identifier_is_a_parse_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: bool = true;
                conditional a && nope {
                    true => { relationship { method [a] -> [a] { a } } },
                }
            }
        "#,
        );
        assert!(result.is_err());
    }
```

If the client-registered-type test's literal syntax (`Mode(1)`) isn't valid CEL syntax for
constructing/matching a client type in this grammar (client types typically aren't
literal-constructible from source — check how other tests in this file construct/compare
non-built-in registered types, e.g. via `register_no_default`/`register` in nearby tests), adapt
this one test to whatever construction mechanism this codebase actually supports for a
client-registered type as a cell's initializer and a branch key; the point of the test is that
`TypeRegistry::entry_by_type_id`/`eq_dyn_fn`/`call_dyn_fn` dispatch correctly for a type that
isn't one of `TypeRegistry::new()`'s built-ins, not the specific literal spelling.

- [ ] **Step 8: Run the tests to verify they fail**

Run: `cargo test -p adam-lang conditional_ --lib`
Expected: FAIL to compile — `parse_match_expr`/`build_match_expr`/the new `AddConditionalFn`
plumbing don't exist yet if Steps 4-5 haven't landed, or (if run after Steps 4-6) the new
assertions fail until the mechanism is correct.

- [ ] **Step 9: Fix any issues found and run again until green**

Run: `cargo test -p adam-lang conditional_ --lib`
Expected: PASS — all 6 new tests.

- [ ] **Step 10: Run the full `adam-lang` test suite**

Run: `cargo test -p adam-lang`
Expected: PASS.

- [ ] **Step 11: Format, lint, and commit the core mechanism**

Run: `cargo fmt --all` then `cargo clippy -p adam-lang --all-targets -- -D warnings`.
Expected: clean.

```bash
git add adam-lang/src/type_registry.rs adam-lang/src/parser.rs
git commit -m "feat(adam-lang): compile conditional match-expressions via a grow-on-demand identifier scope"
```

- [ ] **Step 12: Check `begin/examples/image_resize.adm2` for an uncommitted change**

Run: `git status --short begin/examples/image_resize.adm2`

If it shows modified (not committed), it already anticipates this feature: its `conditional`
declaration was rewritten from the old helper-cell workaround
(`conditional resample_and_constrain { ... }` plus a `relationship { method [resample,
constrain] -> [resample_and_constrain] { resample && constrain } }`) to the direct form
`conditional resample && constrain { ... }`, with the helper cell and relationship removed
entirely. Leave this change in place — it's exactly what this feature is for. If the file is
*not* already modified (e.g. this plan is being run against a fresh checkout), skip Steps 12–14
and go directly to Step 15 — updating this example is optional polish, not a requirement, and
nothing later in this plan depends on it.

- [ ] **Step 13: Run `begin`'s tests to confirm the example now parses**

Run: `cargo test -p begin --no-default-features` and `cargo test -p begin`
Expected: PASS, including `example_source::tests::every_bundled_example_parses_successfully`,
`example_source::tests::image_resize_constrain_is_relevant_despite_only_feeding_a_conditional_helper_cell`,
and `example_source::tests::image_resize_relevance_does_not_depend_on_which_cell_currently_holds_strength`
— these three currently fail against the modified example file (parse error: `expected `{``,
since nothing understands the new syntax yet), and should now pass with no code changes to the
tests' own assertions, since `Sheet::contributing_cells`/`output_relevant_cells` already trace
through a conditional's match cells generically regardless of whether the match subject is one
cell or an expression over several.

- [ ] **Step 14: Refresh the two tests' now-stale doc comments**

These two tests' doc comments (not their assertions — those stay as-is) describe the old
helper-cell mechanism, which no longer exists in the example. In `begin/src/example_source.rs`,
change:

```rust
    #[test]
    fn image_resize_constrain_is_relevant_despite_only_feeding_a_conditional_helper_cell() {
        // Regression test: `constrain` only ever feeds `resample_and_constrain` (a derived
        // helper cell used as a conditional match cell) — it is never itself a relationship
        // output or a match cell. `Sheet::contributing_cells` must still trace through the
        // match cell to find it, or it wrongly shows as an irrelevant/disabled field even
        // though editing it changes which branch is active.
```

to:

```rust
    #[test]
    fn image_resize_constrain_is_relevant_despite_only_being_a_conditional_expression_input() {
        // Regression test: `constrain` is never itself a relationship output or a plain
        // match cell — it's one of two inputs to `conditional resample && constrain`'s match
        // expression. `Sheet::contributing_cells` must still trace through every expression
        // input to find it, or it wrongly shows as an irrelevant/disabled field even though
        // editing it changes which branch is active.
```

(Renaming the test function itself, not just the comment, since the old name's "conditional
helper cell" phrase is now inaccurate — this requires no other changes since the function body/
assertions are unaffected.)

Change:

```rust
    #[test]
    fn image_resize_relevance_does_not_depend_on_which_cell_currently_holds_strength() {
        // Regression test: `dim_width_pixels`/`dim_width_percent`/`doc_width_inches`/
        // `doc_resolution` form a strength-ambiguous diamond (any two determine the rest).
        // In the default state, `dim_width_pixels` and `doc_resolution` happen to be the
        // strength-chosen sources — but every cell in the diamond, plus every cell feeding
        // a conditional match cell (`resample`, `constrain`, `auto_quality`), must show as
        // relevant regardless of which specific cells the *current* strengths picked.
```

to:

```rust
    #[test]
    fn image_resize_relevance_does_not_depend_on_which_cell_currently_holds_strength() {
        // Regression test: `dim_width_pixels`/`dim_width_percent`/`doc_width_inches`/
        // `doc_resolution` form a strength-ambiguous diamond (any two determine the rest).
        // In the default state, `dim_width_pixels` and `doc_resolution` happen to be the
        // strength-chosen sources — but every cell in the diamond, plus every cell feeding a
        // conditional match subject (`resample` and `constrain` are both inputs to one
        // conditional's match expression; `auto_quality` is a plain match cell), must show as
        // relevant regardless of which specific cells the *current* strengths picked.
```

- [ ] **Step 15: Run `begin`'s tests again to confirm the rename didn't break anything**

Run: `cargo test -p begin --no-default-features` and `cargo test -p begin`
Expected: PASS (same tests as Step 13, one renamed).

- [ ] **Step 16: Format, lint, and commit the example/test-comment refresh**

Run: `cargo fmt --all` then `cargo clippy -p begin --no-default-features --all-targets -- -D warnings` and `cargo clippy -p begin --all-targets -- -D warnings`.
Expected: clean.

```bash
git add begin/examples/image_resize.adm2 begin/src/example_source.rs
git commit -m "docs(begin): use a direct conditional expression in image_resize, drop the helper cell"
```

(If Step 12 found the example file unmodified and Steps 13–15 were skipped, skip this commit too — there's nothing to commit.)

---

## Task 4: Full workspace validation

**Files:** none (verification only; fix any residual warnings found in whichever files they appear).

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Expected: no diff (or a clean formatting-only diff — commit it if so).

- [ ] **Step 2: Build the whole workspace**

Run: `cargo build --workspace`
Expected: zero warnings.

- [ ] **Step 3: Test the whole workspace, including doc tests**

Run: `cargo test --workspace` then `cargo test --doc --workspace`
Expected: all tests pass, zero warnings.

- [ ] **Step 4: Lint (all three required invocations)**

Run, in order:

```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

Expected: zero warnings from all three.

- [ ] **Step 5: Doc build sanity check**

Run: `cargo doc --lib --no-deps --workspace`
Expected: builds cleanly.

- [ ] **Step 6: Commit any residual fixes**

If Steps 1–5 required any code changes beyond formatting, commit them:

```bash
git add -A
git commit -m "chore: fix residual warnings from adam-lang conditional-expression work"
```

If no changes were needed, skip this step.
