# adam-lang Deduced Filter Dependencies + `_` Placeholder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `adam-lang`'s closure-literal `filter(arg_cells) |params| body` clause with a
single deduced expression — dependencies on other cells are inferred exactly as a `relationship`
binding's/`out` declaration's/conditional's already are, and the candidate value being conformed
is written `_` instead of a named closure parameter.

**Architecture:** Change `ast::CellFilter` from `{ arg_cells, closure }` to `{ body }` (a bare
`cel_parser::Expr`), then rewrite the four places that build or consume it — the CST parser
(`ast_parser.rs`), the formatter (`fmt.rs`), the CST type checker (`typecheck.rs`), and the
runtime `Sheet`-building parser (`parser.rs`, which gains a `parse_filter_expr` sibling to the
existing `parse_deduced_expr`) — to use the new shape, then update `adam-lsp`'s fixtures. No
`cel-parser` changes are needed: general CEL closures (`Expr::Closure`) stay as-is for other uses,
and range syntax (`..=`) already parses through every adam-lang entry point per the merged
`2026-08-24-cel-range-syntax`/`2026-08-24-range-expression-precedence-fix` plans.

**Tech Stack:** Rust (`adam-lang`, `adam-lsp` crates). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-22-filter-deduction-range-slider-design.md` (§1,
"Deduced Filter Dependencies + `_` Placeholder"). This plan implements §1 only — §2
(`RangeInclusive<T>`/`..=`) is already merged; §3 (`FilterKind`, `adam-rs`/`Sheet` query API) and
§4 (`begin` UI) are separate, later plans, exactly as the cel-range-syntax plan's own "Out of
Scope" note anticipated. A `lo..=hi` filter expression will fail this plan's own type check (its
inferred type is `RangeInclusive<T>`, not `T`) until the §3 plan adds that recognition — this is
expected and correct for this plan's scope, not a bug to fix here.

## Global Constraints

- `cargo fmt --all` before every commit (pre-commit hook enforces this).
- `cargo build --workspace` / `cargo test --workspace` must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` must stay clean after
  every task (this plan never touches `begin`, so its two extra clippy invocations are unaffected).
- Every public function needs a contract-style `///` doc comment (Summary / Preconditions /
  Postconditions / Complexity, per root `CLAUDE.md`); non-trivial private functions too, matching
  this codebase's existing style on the functions this plan touches.
- Unit tests are derived from the contract and public interface only, not the implementation.
- The old `filter(arg_cells) |params| body` syntax is removed outright, not kept alongside the new
  form (per root `CLAUDE.md`, "Project Status" — no releases, no clients yet).
- `_` is a reserved identifier inside a filter expression only — it is not looked up as a cell
  name there, mirroring the wildcard `_` adam-lang already uses for a conditional's default
  branch. Outside a filter expression, `_` is unaffected (still an ordinary identifier / the
  conditional-default token).

---

### Task 1: `ast.rs` — `CellFilter` loses `arg_cells`; `closure` becomes `body`

**Files:**
- Modify: `adam-lang/src/ast.rs`

**Interfaces:**
- Produces: `pub struct CellFilter { pub body: cel_parser::Expr, pub span: ExprSpan }` (replacing
  today's `{ arg_cells: Vec<(String, ExprSpan)>, closure: cel_parser::Expr, span: ExprSpan }`).

- [ ] **Step 1: Update the failing test**

In `adam-lang/src/ast.rs`'s `mod tests`, replace `cell_decl_filter_field_holds_a_cell_filter`
(currently asserting `filter.arg_cells[0].0 == "hi"`) with:

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
                body: cel_parser::Expr::Ident {
                    name: "_".to_string(),
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
        assert!(matches!(
            &filter.body,
            cel_parser::Expr::Ident { name, .. } if name == "_"
        ));
    }
```

- [ ] **Step 2: Run it to verify it fails to compile**

Run: `cargo test -p adam-lang cell_decl_filter_field_holds_a_cell_filter`
Expected: compile error — `CellFilter` has no field `arg_cells`/`closure` yet named that way (the
struct definition hasn't changed yet).

- [ ] **Step 3: Change the `CellFilter` struct**

Replace the struct at `adam-lang/src/ast.rs` (currently lines 213-223):

```rust
/// `cell_filter = "filter" expression.`
#[derive(Debug, Clone)]
pub struct CellFilter {
    /// The filter's body expression. `_` inside it denotes the candidate value being conformed;
    /// every other identifier that names an already-declared cell is a deduced dependency.
    pub body: cel_parser::Expr,
    /// The span of the whole `filter ...` clause.
    pub span: ExprSpan,
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p adam-lang cell_decl_filter_field_holds_a_cell_filter`
Expected: PASS.

- [ ] **Step 5: Run the full `adam-lang` test suite to confirm the expected breakage**

Run: `cargo test -p adam-lang 2>&1 | tail -n 60`
Expected: many FAILs to compile in `ast_parser.rs`, `fmt.rs`, `typecheck.rs` — every other
reference to `arg_cells`/`closure` on `ast::CellFilter`. This is expected; Tasks 2-4 fix them one
crate-module at a time, and since `adam-lang/src/parser.rs` never references `ast::CellFilter` at
all (it builds `adam_rs::Filter` directly from tokens via its own, unrelated code path — see Task
5), it is not expected to be among the files failing here.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add adam-lang/src/ast.rs
git commit -m "refactor(adam-lang): CellFilter becomes a single body expression"
```

---

### Task 2: `ast_parser.rs` — CST `parse_cell_filter` grammar rewrite

**Files:**
- Modify: `adam-lang/src/ast_parser.rs`

**Interfaces:**
- Consumes: `ast::CellFilter { body, span }` from Task 1; `Self::parse_cel_expression` (unchanged
  — already the range-aware `expression` entry point per the precedence-fix plan).
- Produces: `AdamAstParser::parse_cell_filter` returning `Result<ast::CellFilter>`.

- [ ] **Step 1: Update the failing tests**

In `adam-lang/src/ast_parser.rs`'s `mod tests`, replace `parse_cell_with_a_filter_and_no_arg_list`
and `parse_cell_with_a_filter_and_an_arg_list` with:

```rust
    #[test]
    fn parse_cell_with_a_filter() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell a: i32 = 1 filter _; }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        let filter = cell.filter.as_ref().expect("filter present");
        assert!(matches!(&filter.body, Expr::Ident { name, .. } if name == "_"));
    }

    #[test]
    fn parse_cell_with_a_filter_referencing_a_cell() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s { cell hi: i32 = 100; cell a: i32 = 1 filter _ + hi; }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        let filter = cell.filter.as_ref().expect("filter present");
        match &filter.body {
            Expr::Op { name, operands, .. } => {
                assert_eq!(name, "+");
                assert!(matches!(&operands[0], Expr::Ident { name, .. } if name == "_"));
                assert!(matches!(&operands[1], Expr::Ident { name, .. } if name == "hi"));
            }
            other => panic!("expected Op, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run them to verify they fail to compile**

Run: `cargo test -p adam-lang parse_cell_with_a_filter`
Expected: compile error — `AdamAstParser::parse_cell_filter` still returns the old shape.

- [ ] **Step 3: Rewrite `parse_cell_filter`**

Replace `adam-lang/src/ast_parser.rs`'s `parse_cell_filter` (currently lines 224-256):

```rust
    /// `cell_filter = "filter" expression.`
    ///
    /// - Precondition: the `filter` keyword has already been consumed by the caller; `filter_start`
    ///   is its span.
    fn parse_cell_filter(
        &mut self,
        cursor: &mut TokenCursor,
        filter_start: proc_macro2::Span,
    ) -> Result<ast::CellFilter> {
        let body = self.parse_cel_expression(cursor)?;
        let body_end = body.span().end;
        Ok(ast::CellFilter {
            body,
            span: ast::ExprSpan {
                start: filter_start,
                end: body_end,
            },
        })
    }
```

- [ ] **Step 4: Run the new tests to verify they pass**

Run: `cargo test -p adam-lang parse_cell_with_a_filter parse_cell_with_a_filter_referencing_a_cell parse_cell_without_a_filter_leaves_it_none`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-lang/src/ast_parser.rs
git commit -m "refactor(adam-lang): rewrite CST cell_filter grammar to a bare expression"
```

---

### Task 3: `fmt.rs` — formatter rewrite

**Files:**
- Modify: `adam-lang/src/fmt.rs`

**Interfaces:**
- Consumes: `ast::CellFilter { body, span }` from Task 1.

- [ ] **Step 1: Update the failing tests**

In `adam-lang/src/fmt.rs`'s `mod tests`, replace `formats_a_cell_with_a_filter_and_no_arg_list`,
`formats_a_cell_with_a_filter_and_an_arg_list`, and `format_is_idempotent_through_a_reparse_with_a_filter`:

```rust
    #[test]
    fn formats_a_cell_with_a_filter() {
        assert_eq!(
            format("sheet s { cell a: i32 = 1 filter _; }"),
            "sheet s {\n    cell a: i32 = 1 filter _;\n}\n"
        );
    }

    #[test]
    fn formats_a_cell_with_a_filter_referencing_a_cell() {
        assert_eq!(
            format("sheet s { cell hi: i32 = 100; cell a: i32 = 1 filter min(_, hi); }"),
            "sheet s {\n    cell hi: i32 = 100;\n    cell a: i32 = 1 filter min(_, hi);\n}\n"
        );
    }

    #[test]
    fn format_is_idempotent_through_a_reparse_with_a_filter() {
        let source = "sheet s {\n    cell a: i32 = 1 filter _;\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }
```

- [ ] **Step 2: Run them to verify they fail to compile**

Run: `cargo test -p adam-lang formats_a_cell_with_a_filter`
Expected: compile error — `write_cell` still reads `filter.arg_cells`/`filter.closure`.

- [ ] **Step 3: Rewrite `write_cell`'s filter clause**

In `adam-lang/src/fmt.rs`, replace the doc comment and filter block of `write_cell` (currently
lines 246-282):

```rust
/// Writes one `cell name[: type][ = initializer][ filter body];` declaration, delegating its
/// type annotation to [`source_text_or_empty`] via `TypeExpr::span()` and its initializer/filter
/// body to [`cel_parser::format_expr`].
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
        out.push_str(&cel_parser::format_expr(&filter.body));
    }
    out.push_str(";\n");
}
```

- [ ] **Step 4: Run the new tests to verify they pass**

Run: `cargo test -p adam-lang formats_a_cell_with_a_filter formats_a_cell_with_a_filter_referencing_a_cell format_is_idempotent_through_a_reparse_with_a_filter`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-lang/src/fmt.rs
git commit -m "refactor(adam-lang): format the new single-expression filter clause"
```

---

### Task 4: `typecheck.rs` — CST type-checker rewrite

**Files:**
- Modify: `adam-lang/src/typecheck.rs`

**Interfaces:**
- Consumes: `ast::CellFilter { body, span }` from Task 1; `cel_parser::ty::check_expr(expr,
  resolve) -> (Ty, Vec<ParseError>)` (unchanged); `expr_matches_shape` (unchanged, already in this
  file).
- Produces: `check_filter(cell, registry, cell_types, shapes, resolve, diagnostics)` — drops the
  `cell_names: &HashSet<String>` parameter (no longer needed: an unrecognized identifier inside a
  deduced filter body is left as `Ty::Any` by `resolve`'s existing fallback, exactly like every
  other deduced expression in this file — bindings, `out` initializers, conditional match
  expressions already behave this way and are not flagged either).

- [ ] **Step 1: Update the failing tests**

In `adam-lang/src/typecheck.rs`'s `mod tests`, replace the eight filter tests
(`filter_with_matching_types_has_no_diagnostic` through
`filter_tuple_typed_cell_with_arity_mismatch_is_a_diagnostic`, currently lines 957-1021) with:

```rust
    #[test]
    fn filter_with_matching_types_has_no_diagnostic() {
        let sheet = parse("sheet s { cell a: i32 = 1 filter _; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn filter_referencing_a_cell_has_no_diagnostic() {
        let sheet = parse(
            "sheet s { cell hi: i32 = 100; cell a: i32 = 1 filter if _ > hi { hi } else { _ }; }",
        );
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn filter_body_type_mismatch_is_a_diagnostic() {
        // Body is `bool`-typed (a comparison), but `a` is declared `i32`.
        let sheet = parse("sheet s { cell a: i32 = 1 filter _ > 0; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn filter_without_underscore_is_a_diagnostic() {
        let sheet = parse("sheet s { cell a: i32 = 1 filter 1; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn filter_tuple_typed_cell_with_matching_shape_has_no_diagnostic() {
        let sheet =
            parse("sheet s { cell a: (i32, f64) = (1, 2.5) filter (_.0, _.1); }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }

    #[test]
    fn filter_tuple_typed_cell_with_arity_mismatch_is_a_diagnostic() {
        let sheet = parse("sheet s { cell a: (i32, f64) = (1, 2.5) filter (_.0,); }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert_eq!(diags.len(), 1);
    }
```

- [ ] **Step 2: Run them to verify they fail to compile**

Run: `cargo test -p adam-lang filter_`
Expected: compile error — `check_filter` still destructures `Expr::Closure` and reads
`filter.arg_cells`.

- [ ] **Step 3: Remove the now-dead closure-param helpers**

Delete `closure_param_type_expr_to_type_expr` (currently lines 387-405) and `closure_param_ty`
(currently lines 407-416) from `adam-lang/src/typecheck.rs` entirely — both existed only to
resolve a filter closure's declared parameter types, which no longer exist.

- [ ] **Step 4: Rewrite `check_filter`**

**Corrected during task review** (round 1 of the fix loop for this task): an earlier version of
this step tracked whether `_` was referenced via a `std::cell::Cell<bool>` set from inside
`body_resolve`, the same closure passed to `check_expr`/`expr_matches_shape` for type-checking.
That double-purposes one closure for two unrelated jobs and breaks on the tuple-shaped path:
`expr_matches_shape`'s arity-mismatch case returns before visiting any element, so `body_resolve`
is never called and `_` incorrectly appears unreferenced; fixing that by pre-running `check_expr`
over the whole body before `expr_matches_shape` (to force every identifier to be visited at least
once) then double-invokes type-checking on every element `expr_matches_shape` itself also checks,
duplicating that element's diagnostics whenever it has its own type error. The fix below tracks
`_`-usage as a separate, dedicated structural check — a plain tree walk with no interaction with
type-checking at all — so type-checking runs exactly once, on every path.

Add this new function immediately above `check_filter` (it has no other caller in this file):

```rust
/// Returns whether `expr` contains a reference to the identifier `name` anywhere in its tree.
/// Used by `check_filter` to check whether a filter's body references `_` — deliberately a plain
/// structural walk, not built on `check_expr`'s identifier resolution, so checking for `_`'s
/// presence never runs type-checking a second time over any part of `expr`.
///
/// - Complexity: O(n) in the number of sub-expressions in `expr`.
fn expr_references_ident(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Literal { .. } => false,
        Expr::Ident { name: ident, .. } => ident == name,
        Expr::Op { operands, .. } => operands.iter().any(|e| expr_references_ident(e, name)),
        Expr::Apply { callee, args, .. } => {
            expr_references_ident(callee, name)
                || args.iter().any(|e| expr_references_ident(e, name))
        }
        Expr::Tuple { elements, .. } => elements.iter().any(|e| expr_references_ident(e, name)),
        Expr::TupleIndex { base, .. } => expr_references_ident(base, name),
        Expr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            expr_references_ident(cond, name)
                || expr_references_ident(then_branch, name)
                || else_branch
                    .as_deref()
                    .is_some_and(|e| expr_references_ident(e, name))
        }
        Expr::Logical { lhs, rhs, .. } => {
            expr_references_ident(lhs, name) || expr_references_ident(rhs, name)
        }
        Expr::Cast { expr, .. } => expr_references_ident(expr, name),
        Expr::Closure { body, .. } => expr_references_ident(body, name),
    }
}
```

Then replace `check_filter`'s doc comment and body (currently lines 418-533) with:

```rust
/// Checks one `cell`'s `filter` clause, if present: the body's inferred type must unify with
/// this cell's own declared/inferred shape (`_`'s type, via `body_resolve`'s special case
/// below), and the body must reference `_` — the value being filtered — at least once. Every
/// other identifier is resolved exactly as any other deduced expression in this file (a
/// `relationship` binding, an `out` initializer): via `resolve`, which leaves an unrecognized
/// name as `Ty::Any` rather than raising a diagnostic — the runtime `Sheet`-building parser
/// (`adam_lang::parser::AdamParser::parse_cell_filter`) is what raises "undeclared cell" for a
/// name that isn't actually a declared cell, mirroring how it (not this file) is the one that
/// raises that error for bindings' deduced expressions too.
fn check_filter(
    cell: &CellDecl,
    registry: &TypeRegistry,
    cell_types: &std::collections::HashMap<String, Ty>,
    shapes: &std::collections::HashMap<String, TypeShape>,
    resolve: &impl Fn(&str) -> Ty,
    diagnostics: &mut Vec<ParseError>,
) {
    let Some(filter) = &cell.filter else {
        return;
    };

    let own_ty = resolve(&cell.name);
    let body_resolve = |name: &str| -> Ty {
        if name == "_" { own_ty } else { resolve(name) }
    };

    match expected_shape(&cell.name, cell_types, shapes) {
        Some(shape @ TypeShape::Tuple(_)) => {
            expr_matches_shape(&filter.body, &shape, registry, &body_resolve, diagnostics);
        }
        Some(TypeShape::Named(type_id)) => {
            let (body_ty, body_diags) = check_expr(&filter.body, &body_resolve);
            diagnostics.extend(body_diags);
            let declared = Ty::from_type_id(type_id);
            if !declared.unifies_with(&body_ty) {
                diagnostics.push(ParseError::new_range(
                    format!("cell `{}`: filter must produce `{}`", cell.name, declared.name()),
                    filter.body.span().start,
                    filter.body.span().end,
                ));
            }
        }
        None => {
            let (_, body_diags) = check_expr(&filter.body, &body_resolve);
            diagnostics.extend(body_diags);
        }
    }

    if !expr_references_ident(&filter.body, "_") {
        diagnostics.push(ParseError::new_range(
            "filter must reference `_` (the value being filtered)".to_string(),
            filter.span.start,
            filter.span.end,
        ));
    }
}
```

Add one more test (this task's original test list above already has the six needed for coverage
of the type-check/arity paths; this one specifically covers the new helper's own tree-walk
correctness on a nested case none of the six exercise):

```rust
    #[test]
    fn filter_references_underscore_nested_inside_a_call_has_no_missing_underscore_diagnostic() {
        // `_` appears only inside an `if`'s then-branch, not as the whole body or a bare
        // operand — exercises `expr_references_ident`'s `Expr::If` arm specifically.
        let sheet = parse("sheet s { cell a: i32 = 1 filter if true { _ } else { 1 }; }");
        let diags = check_sheet(&sheet, &TypeRegistry::new());
        assert!(diags.is_empty());
    }
```

Also update `expected_shape`'s doc comment (currently describing "one filter-closure parameter
position (the filtered cell itself, or one of its declared argument cells)" — both concepts this
task removes; `expected_shape` is now called from exactly one place, with only `cell.name`):
reword its first sentence to describe the current single-argument usage, e.g. "The expected
`TypeShape` for a filtered cell's own declared/inferred shape (`_`'s type inside its filter
body)." Keep the rest of that doc comment (the `Some`/`None` semantics) unchanged — only the
now-stale first sentence needs rewording.

Note: `expected_shape` already returns an owned `Option<TypeShape>` (see its existing doc comment
above this function), so the `Some(shape @ TypeShape::Tuple(_))` arm binds an owned `TypeShape`,
not a reference — pass `&shape` to `expr_matches_shape` as shown (matching how `check_binding`
already calls it elsewhere in this file).

- [ ] **Step 5: Drop the now-unused `cell_names` plumbing**

In `adam-lang/src/typecheck.rs`:
- Delete the `declared_cell_names` function (currently lines 348-362) — it existed only to build
  `check_filter`'s old `cell_names` argument.
- In `check_sheet` (currently lines 36-83), delete the line
  `let cell_names = declared_cell_names(sheet);` and remove the `&cell_names,` argument from the
  `check_filter(...)` call site.
- Remove `ClosureParamTypeExpr` from the `use cel_parser::{...}` import list (line 12) — no longer
  referenced now that Step 3 deleted its only two call sites.

- [ ] **Step 6: Run the filter tests, then the full `adam-lang` suite**

Run: `cargo test -p adam-lang filter_`
Expected: PASS (seven tests, including the nested-`_`-reference test added above).

Run: `cargo test -p adam-lang`
Expected: **Correction — verified during execution:** `adam-lang/src/parser.rs` (the runtime
`AdamParser`) never references `ast::CellFilter` at all — it builds `adam_rs::Filter` directly
from tokens via its own, separate `parse_cell_filter`/`DynClosure`-based path, untouched by this
task's or Tasks 1-3's changes. So `cargo test -p adam-lang` is expected to **compile and pass in
full** at this point, not fail — the crate is left in a valid but inconsistent intermediate state
where the CST parser/formatter/type-checker (Tasks 1-4) already speak the new `filter _` grammar,
while the runtime parser (Task 5, next) still only accepts the old `filter(args) |params| body`
closure syntax. Confirm all tests pass, with no failures anywhere in the crate.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add adam-lang/src/typecheck.rs
git commit -m "refactor(adam-lang): type-check filters as a deduced expression with '_'"
```

---

### Task 5: `parser.rs` — runtime `Sheet`-building rewrite

**Files:**
- Modify: `adam-lang/src/parser.rs`

**Interfaces:**
- Consumes: `ast::CellFilter` is not used here (this parser builds `adam_rs::Filter` directly from
  tokens, not from the CST) — but the same grammar/semantics apply. Reuses `NamedCells`,
  `InputPush`, `TypeShape`, `cell_type_id` (all already defined in this file), and
  `CallDynFn`/`PushArgFn` from `crate::type_registry` (already imported).
- Produces: a new private method `AdamParser::parse_filter_expr(&mut self, ctx, declared_shape:
  &TypeShape) -> Result<(DynSegment, NamedCells, bool)>` (the `bool` is whether `_` was
  referenced) — a sibling to the existing `parse_deduced_expr`, not a modification of it (other
  four call sites of `parse_deduced_expr` are untouched). Rewrites `parse_cell_filter` to use it.

- [ ] **Step 1: Update the failing tests**

In `adam-lang/src/parser.rs`'s `mod tests`, replace the six filter tests
(`cell_filter_with_no_extra_args_clamps_on_write` through
`filter_tracks_a_tuple_typed_range_cell_dynamically`, currently lines 1435-1518, immediately
before `parse_multiple_cells`) with:

```rust
    #[test]
    fn cell_filter_with_no_named_dependency_clamps_on_write() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str(
                "sheet s { cell a: i32 filter if _ < 1 { 1 } else if _ > 100 { 100 } else { _ }; }",
            )
            .unwrap();
        let (cell_id, _) = parsed.cell_names["a"];
        parsed.sheet.write(cell_id, 500i32).unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(cell_id).unwrap(), 100);
    }

    #[test]
    fn cell_filter_referencing_a_cell_tracks_its_current_value() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str(
                "sheet s { \
                     cell hi: i32 = 100; \
                     cell a: i32 filter if _ < 1 { 1 } else if _ > hi { hi } else { _ }; \
                 }",
            )
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        let (hi_id, _) = parsed.cell_names["hi"];

        parsed.sheet.write(a_id, 500i32).unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 100);

        parsed.sheet.write(hi_id, 10i32).unwrap();
        parsed.sheet.write(a_id, 500i32).unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 10);
    }

    #[test]
    fn cell_filter_referencing_the_same_value_twice_is_idempotent() {
        // Snap-to-grid: `_ - (_ % step)` — `_` referenced twice must denote the same value both
        // times, not two independent parameters.
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str(
                "sheet s { cell step: i32 = 10; cell a: i32 filter _ - (_ % step); }",
            )
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        parsed.sheet.write(a_id, 27i32).unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 20);
    }

    #[test]
    fn cell_filter_without_underscore_is_a_parse_error() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let err = parser.parse_str("sheet s { cell a: i32 filter 1; }");
        assert!(err.is_err());
    }

    #[test]
    fn cell_filter_body_type_mismatch_is_a_parse_error() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let err = parser.parse_str("sheet s { cell a: i32 filter _ > 0; }");
        assert!(err.is_err());
    }

    #[test]
    fn cell_filter_undeclared_identifier_is_a_parse_error() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let err = parser.parse_str("sheet s { cell a: i32 filter _ + nope; }");
        assert!(err.is_err());
    }

    #[test]
    fn filter_tracks_a_tuple_typed_range_cell_dynamically() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let mut parsed = parser
            .parse_str(
                "sheet s { \
                     cell a_range: (i32, i32) = (1, 100); \
                     cell max: i32 = 100; \
                     relationship { a_range := (1, max); } \
                     cell a: i32 filter if _ < a_range.0 { a_range.0 } \
                         else if _ > a_range.1 { a_range.1 } else { _ }; \
                 }",
            )
            .unwrap();
        let (a_id, _) = parsed.cell_names["a"];
        let (max_id, _) = parsed.cell_names["max"];

        parsed.sheet.write(a_id, 500i32).unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 100);

        parsed.sheet.write(max_id, 10i32).unwrap();
        parsed.sheet.propagate().unwrap();
        parsed.sheet.write(a_id, 500i32).unwrap();
        assert_eq!(*parsed.sheet.read::<i32>(a_id).unwrap(), 10);
    }

    #[test]
    fn cell_filter_on_a_tuple_typed_cell_is_a_parse_error() {
        let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
        let err = parser.parse_str("sheet s { cell a: (i32, i32) filter (_.0, _.1); }");
        assert!(err.is_err());
    }
```

`cell_filter_undeclared_identifier_is_a_parse_error` deliberately uses `+` (an already-registered
numeric operator), not an illustrative stand-in like `min`/`.clamp` (per the spec, none of those
are registered CEL builtins yet) — using an unregistered function here would make the test fail
for the wrong reason (the function itself being unresolvable) rather than isolating the specific
behavior under test: an identifier that resolves to neither `_` nor a declared cell.

- [ ] **Step 2: Run them to verify they fail (some to compile, some at runtime)**

Run: `cargo test -p adam-lang cell_filter_ filter_tracks_a_tuple_typed_range_cell_dynamically`
Expected: compile error — `parse_cell_filter` still builds a `DynClosure`.

- [ ] **Step 3: Remove the `DynClosure` import**

In `adam-lang/src/parser.rs`, change line 15 from:

```rust
use cel_runtime::{DynClosure, DynSegment};
```

to:

```rust
use cel_runtime::DynSegment;
```

- [ ] **Step 4: Add `parse_filter_expr`**

Add this new method to `impl<'a> AdamParser<'a>` (or wherever `parse_deduced_expr` lives — place
it immediately after `parse_deduced_expr`, since it's a sibling of it):

```rust
    /// Parses a `filter` clause's body expression, deducing its dependencies exactly as
    /// [`Self::parse_deduced_expr`] does, plus one reserved identifier: `_` always resolves to
    /// argument slot 0 (the candidate value being conformed, of `declared_shape`'s type), ahead
    /// of any cell-derived slots, which start at slot 1. Returns whether `_` was referenced at
    /// least once, alongside the compiled segment and its deduced cell inputs — the caller
    /// decides whether that occurrence count is acceptable.
    ///
    /// # Errors
    /// Returns `Err` if the expression fails to parse.
    ///
    /// - Complexity: O(k) in the number of distinct cell identifiers referenced, for this
    ///   method's own bookkeeping (on top of `cel-parser`'s own parse cost).
    fn parse_filter_expr(
        &mut self,
        ctx: &mut ParseContext,
        declared_shape: &TypeShape,
    ) -> Result<(DynSegment, NamedCells, bool)> {
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
                    TypeShape::Tuple(_) => InputPush::Tuple(self.types.associated_prototype(shape)),
                };
                (name.clone(), (*cell_id, shape.clone(), push))
            })
            .collect();

        let value_push = match declared_shape {
            TypeShape::Named(type_id) => InputPush::Scalar(
                self.types
                    .entry_by_type_id(*type_id)
                    .expect("declared cell type registered")
                    .push_arg_fn,
            ),
            TypeShape::Tuple(_) => {
                InputPush::Tuple(self.types.associated_prototype(declared_shape))
            }
        };

        let accumulator: Arc<Mutex<NamedCells>> = Arc::new(Mutex::new(Vec::new()));
        let scope_accumulator = Arc::clone(&accumulator);
        let underscore_used: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let scope_underscore_used = Arc::clone(&underscore_used);

        self.cel
            .op_lookup_mut()
            .push_scope(move |name, segment, arity, _span| {
                if arity != 0 {
                    return Ok(false);
                }
                if name == "_" {
                    *scope_underscore_used
                        .lock()
                        .expect("scope mutex not poisoned") = true;
                    match &value_push {
                        InputPush::Scalar(fn_ptr) => fn_ptr(segment, 0),
                        InputPush::Tuple(associated) => {
                            segment.push_arg_as_dynamic_sequence_tuple(0, associated.clone())
                        }
                    }
                    return Ok(true);
                }
                let Some((cell_id, shape, push)) = push_table.get(name) else {
                    return Ok(false);
                };
                let idx = {
                    let mut acc = scope_accumulator.lock().expect("scope mutex not poisoned");
                    match acc.iter().position(|(n, ..)| n == name) {
                        Some(pos) => pos + 1,
                        None => {
                            acc.push((name.to_string(), *cell_id, shape.clone()));
                            acc.len()
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

        let result = self.parse_cel_expression(ctx);
        self.cel.op_lookup_mut().pop_scope();
        let segment = result?;

        let inputs = accumulator
            .lock()
            .expect("scope mutex not poisoned")
            .clone();
        let used = *underscore_used.lock().expect("scope mutex not poisoned");
        Ok((segment, inputs, used))
    }
```

- [ ] **Step 5: Rewrite `parse_cell_filter`**

Replace the doc comment and body of `parse_cell_filter` (currently lines 267-379) with:

```rust
    /// `cell_filter = "filter" expression.`
    ///
    /// Builds an [`adam_rs::Filter`] from a single deduced expression: `_` denotes the candidate
    /// value being conformed (of `declared_shape`'s type); every other identifier that names an
    /// already-declared cell is a deduced dependency, exactly as [`Self::parse_deduced_expr`]
    /// resolves them for a `relationship` binding or `out` declaration — see
    /// [`Self::parse_filter_expr`]. `declared_shape` is the filtered cell's own declared type,
    /// already resolved by the caller in [`parse_cell_decl`]. The filtered cell's own `CellId` is
    /// not needed here: the caller attaches the returned `Filter` to it afterwards, via
    /// `Sheet::add_filter`.
    ///
    /// # Errors
    /// Returns `Err` if `declared_shape` is a tuple (not yet supported by this builder), if an
    /// identifier inside the expression names neither `_` nor an already-declared cell, if `_` is
    /// never referenced, or if the expression's inferred type doesn't match `declared_shape`.
    ///
    /// - Complexity: O(m) in the number of distinct cell identifiers the expression references,
    ///   for this method's own bookkeeping (on top of the expression's own parse/compile cost).
    fn parse_cell_filter(
        &mut self,
        ctx: &mut ParseContext,
        cell_name: &str,
        cell_span: Span,
        declared_shape: &TypeShape,
    ) -> Result<adam_rs::Filter> {
        if matches!(declared_shape, TypeShape::Tuple(_)) {
            return Err(ParseError::new(
                format!("cell `{cell_name}`: filter on a tuple-typed cell is not yet supported"),
                cell_span,
            ));
        }

        let (segment, inputs, underscore_used) = self.parse_filter_expr(ctx, declared_shape)?;
        if !underscore_used {
            return Err(ParseError::new(
                "filter must reference `_` (the value being filtered)",
                cell_span,
            ));
        }

        let value_type_id = cell_type_id(declared_shape);
        let output_type_id = segment.peek_output_type_id().ok_or_else(|| {
            ParseError::new(format!("cell `{cell_name}`: filter produced no value"), cell_span)
        })?;
        if output_type_id != value_type_id {
            return Err(ParseError::new(
                format!(
                    "cell `{cell_name}`: filter must produce `{}`",
                    self.types.display_name(declared_shape)
                ),
                cell_span,
            ));
        }

        // `call_dyn_fn` is the same monomorphized-per-registered-type dispatcher `build_method`/
        // `build_match_expr` already use for a deduced expression's scalar output.
        let call_fn = self
            .types
            .entry_by_type_id(value_type_id)
            .expect("declared cell type registered")
            .call_dyn_fn;

        let arg_ids: Vec<CellId> = inputs.iter().map(|(_, id, _)| *id).collect();
        let arg_type_ids: Vec<TypeId> = inputs
            .iter()
            .map(|(_, _, shape)| cell_type_id(shape))
            .collect();

        // `RefCell`, not a plain `move` capture: `call_fn` takes `&mut DynSegment`, unlike
        // `DynClosure::call_boxed`'s `&self` the old closure-literal path used.
        let segment = RefCell::new(segment);

        Ok(adam_rs::Filter::new(
            value_type_id,
            arg_ids,
            arg_type_ids,
            move |value, args| {
                let mut call_args: Vec<&dyn Any> = Vec::with_capacity(1 + args.len());
                call_args.push(value);
                call_args.extend_from_slice(args);
                call_fn(&mut segment.borrow_mut(), &call_args)
            },
        ))
    }
```

- [ ] **Step 6: Run the new filter tests, then the full workspace suite**

Run: `cargo test -p adam-lang cell_filter_ filter_tracks_a_tuple_typed_range_cell_dynamically`
Expected: PASS (eight tests).

Run: `cargo test --workspace`
Expected: PASS, zero warnings. (This also re-checks `adam-lsp` and `begin`, which depend on
`adam-lang` — `adam-lsp`'s own filter fixtures are updated next, in Task 6; expect its two
filter-related tests to still fail here.)

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Expected: clean.

Run: `cargo fmt --all -- --check`
Expected: clean (or run `cargo fmt --all` and include the diff in this commit).

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add adam-lang/src/parser.rs
git commit -m "feat(adam-lang): build filters from a deduced expression with '_'"
```

---

### Task 6: `adam-lsp` — update filter fixtures

**Files:**
- Modify: `adam-lsp/src/diagnostics.rs`
- Modify: `adam-lsp/src/dispatch.rs`

**Interfaces:**
- Consumes: the new `filter` grammar from Tasks 2-4 (`AdamAstParser`/`check_sheet`, which
  `adam-lsp` calls unchanged — only the fixture *source strings* need updating).

- [ ] **Step 1: Update `diagnostics.rs`'s filter tests**

In `adam-lsp/src/diagnostics.rs`, replace `filter_clause_with_matching_types_has_no_diagnostics`
and `filter_clause_with_an_undeclared_arg_cell_is_a_diagnostic` (currently lines 107-119):

```rust
    #[test]
    fn filter_clause_with_matching_types_has_no_diagnostics() {
        assert!(diagnostics_for_source("sheet s { cell a: i32 = 1 filter _; }").is_empty());
    }

    #[test]
    fn filter_clause_without_underscore_is_a_diagnostic() {
        let diags = diagnostics_for_source("sheet s { cell a: i32 = 1 filter 1; }");
        assert_eq!(diags.len(), 1);
    }
```

(`filter_clause_with_an_undeclared_arg_cell_is_a_diagnostic` is removed outright, not replaced —
per Task 4, the CST type checker no longer diagnoses an unrecognized identifier inside a deduced
filter body at all, matching every other deduced expression's existing leniency in this codebase;
that check now lives only in the runtime parser, which `adam-lsp` doesn't exercise.)

- [ ] **Step 2: Update `dispatch.rs`'s filter formatting test**

In `adam-lsp/src/dispatch.rs`, replace `format_edits_formats_a_cell_with_a_filter` (currently
lines 285-293):

```rust
    #[test]
    fn format_edits_formats_a_cell_with_a_filter() {
        let edits = format_edits("sheet s { cell a:i32=1 filter _; }");
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].new_text,
            "sheet s {\n    cell a: i32 = 1 filter _;\n}\n"
        );
    }
```

- [ ] **Step 3: Run the updated tests, then the full workspace suite**

Run: `cargo test -p adam-lsp filter`
Expected: PASS.

Run: `cargo test --workspace`
Expected: PASS, zero warnings.

Run: `cargo test --doc --workspace`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add adam-lsp/src/diagnostics.rs adam-lsp/src/dispatch.rs
git commit -m "test(adam-lsp): update filter fixtures for the deduced-expression grammar"
```

---

### Task 7: Full verification and handoff

**Files:**
- Modify (if needed): `docs/superpowers/2026-08-24-filter-deduction-phase-1-handoff.md` (new)

- [ ] **Step 1: Run the full check suite**

Run, in order, exactly as root `CLAUDE.md`'s "Commands" section requires before any PR:

```bash
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

Expected: every command PASSes/is clean, with zero compiler warnings from the plain
build/test runs (not just clippy) — read the output, don't just check exit codes.

- [ ] **Step 2: Write a phase handoff doc**

Create `docs/superpowers/2026-08-24-filter-deduction-phase-1-handoff.md` (format matching
`docs/superpowers/2026-07-18-phase-3-handoff.md`), summarizing:
- Done: §1 of `2026-08-22-filter-deduction-range-slider-design.md` — filters are now a single
  deduced expression with `_`; the old `filter(args) |params| body` syntax is gone.
- Deliberately deferred: a `lo..=hi` filter expression parses (range syntax is already merged) but
  fails this plan's own type check (`RangeInclusive<T>` ≠ `T`) — expected until §3 lands.
- Left: §3 (`FilterKind` tag + `Sheet::filter_kind`/`filter_range` query API in `adam-rs`,
  recognizing a `RangeInclusive`-typed filter body and building it via a new `Filter::range`
  constructor instead of failing the type check) and §4 (`begin`'s number-field/slider UI, which
  depends on §3's query API).

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/2026-08-24-filter-deduction-phase-1-handoff.md
git commit -m "docs: add phase 1 handoff for deduced filter args"
```

---

## Out of Scope (confirmed, not deferred by accident)

- `cel-parser` changes: none needed. `Expr::Closure`/`ClosureParam` stay as general-purpose CEL
  constructs for other uses; range syntax already parses through every adam-lang entry point.
- Recognizing a `RangeInclusive<T>`-typed filter body as a clamp (`FilterKind::Range`,
  `Filter::range`, `Sheet::filter_kind`/`filter_range`) — spec §3, a separate later plan. This
  plan's own type check correctly rejects `filter lo..=hi` today (wrong type, not a clamp) — that
  rejection is expected to be *replaced*, not merely loosened, by the §3 plan (which recognizes
  the shape and builds a real clamp `Filter`, rather than just relaxing the type check).
  `_`'s "must be referenced" requirement is also expected to gain an exception for that case in
  the §3 plan, per the spec.
- `begin` UI (number field, slider) — spec §4, depends on §3.
- Tuple-typed *filtered cells* (`declared_shape` itself a `TypeShape::Tuple`) at the runtime
  layer: turned into a clean `ParseError` in this plan rather than left to panic (the old
  closure-based code would have panicked here too, via `entry_by_type_id(TypeId::of::<
  DynamicSequence>()).expect(...)`, since tuple types have no `TypeRegistry` entry — this was
  never actually reachable working code, only an untested path). The CST type checker still
  accepts a tuple-typed filtered cell structurally (`filter_tuple_typed_cell_with_matching_shape_
  has_no_diagnostic`) since it never builds a runtime segment — this is a pre-existing
  CST-vs-runtime asymmetry, not introduced by this plan, and fixing it (teaching the runtime
  parser to actually build tuple-typed filters) is out of scope here.
