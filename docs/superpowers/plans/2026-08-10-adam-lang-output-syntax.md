# adam-lang Output/Condition Syntax Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `out`/`condition` DSL syntax to `adam-lang` for the already-implemented `adam-rs` `Sheet::add_output`/`Condition` API, across the full `adam-lang` tooling stack (direct-to-`Sheet` parser, span-carrying AST parser, formatter, typechecker).

**Architecture:** Two independent grammar implementations already exist side by side for `cell`/`relationship`/`conditional` — `parser.rs` (builds a live `adam_rs::Sheet` directly) and `ast_parser.rs` (builds a span-carrying `ast::Sheet`, consumed by `fmt.rs`/`typecheck.rs`/the LSP). This plan extends both in the same style, plus `fmt.rs` and `typecheck.rs`.

**Tech Stack:** Rust, `cel_parser`/`cel_runtime` (CEL expression compilation), `adam_rs` (constraint-graph runtime), `slotmap`/`indexmap`.

## Global Constraints

- Format with `cargo fmt --all` before every commit (enforced by a pre-commit hook).
- Every function gets a `///` doc comment in contract style (Summary; Preconditions as `- Precondition:` bullets checked via `debug_assert!`, never documented as causing a specific failure; `# Errors` for `Err`-returning conditions; Postconditions as `- Postcondition:` bullets; `- Complexity:` bullet whenever not O(1)).
- Unit tests are derived only from a function's contract and public interface, never from reading its implementation.
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings; `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` must pass.
- No heap allocation beyond what the existing code in the touched files already does (borrow `&str`/`&[T]` over owning `String`/`Vec<T>` where a choice exists).
- This plan only touches the `adam-lang` crate (`adam-lang/src/ast.rs`, `ast_parser.rs`, `token_cursor.rs`, `fmt.rs`, `typecheck.rs`, `parser.rs`, `lib.rs`). No other crate changes — see the design doc's §13 "Deferred / out of scope" (`begin`'s graph view, `adam-lsp`, the VS Code extension) for what is explicitly not part of this plan.
- Spec: `docs/superpowers/specs/2026-08-09-adam-lang-output-syntax-design.md`. Read it before starting — this plan implements its grammar (§3), AST (§4), parser (§5), AST parser (§6), formatter (§7), and typechecker (§8) sections verbatim.

---

### Task 1: AST types (`ast.rs`)

**Files:**
- Modify: `adam-lang/src/ast.rs`

**Interfaces:**
- Produces: `ast::OutDecl { name: String, name_span: ExprSpan, type_name: Option<(String, ExprSpan)>, writer: OutMethodDecl, conditions: Vec<ConditionDecl>, leading_comment: Option<String>, blank_line_before: bool, span: ExprSpan }`
- Produces: `ast::OutMethodDecl { inputs: Vec<(String, ExprSpan)>, body: cel_parser::Expr, leading_comment: Option<String>, blank_line_before: bool, span: ExprSpan }`
- Produces: `ast::ConditionDecl { name: String, name_span: ExprSpan, inputs: Vec<(String, ExprSpan)>, body: cel_parser::Expr, leading_comment: Option<String>, blank_line_before: bool, span: ExprSpan }`
- Produces: `ast::SheetItem::Out(OutDecl)` variant, handled by `SheetItem::span()`/`set_leading_comment()`/`set_blank_line_before()`.
- Consumed by: Task 2 (`ast_parser.rs` builds these), Task 3 (`fmt.rs` reads them), Task 4 (`typecheck.rs` reads them).

- [ ] **Step 1: Write the failing test**

Add to `adam-lang/src/ast.rs`'s existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn sheet_item_span_reads_the_out_variant() {
    let span = point(Span::call_site());
    let item = SheetItem::Out(OutDecl {
        name: "o".to_string(),
        name_span: span,
        type_name: None,
        writer: OutMethodDecl {
            inputs: Vec::new(),
            body: cel_parser::Expr::Ident {
                name: "x".to_string(),
                span,
            },
            leading_comment: None,
            blank_line_before: false,
            span,
        },
        conditions: Vec::new(),
        leading_comment: None,
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
        writer: OutMethodDecl {
            inputs: Vec::new(),
            body: cel_parser::Expr::Ident {
                name: "x".to_string(),
                span,
            },
            leading_comment: None,
            blank_line_before: false,
            span,
        },
        conditions: Vec::new(),
        leading_comment: None,
        blank_line_before: false,
        span,
    });
    item.set_leading_comment("hi".to_string());
    match item {
        SheetItem::Out(o) => assert_eq!(o.leading_comment.as_deref(), Some("hi")),
        other => panic!("expected Out, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p adam-lang sheet_item_span_reads_the_out_variant`
Expected: FAIL with a compile error (`OutDecl`/`OutMethodDecl`/`SheetItem::Out` don't exist yet).

- [ ] **Step 3: Add the new AST types and wire up `SheetItem`**

In `adam-lang/src/ast.rs`, after the existing `RelationshipDecl` struct and before `ConditionalDecl` (or anywhere alongside the other decl structs — exact position doesn't matter, grouping matters for readability), add:

```rust
/// `out_decl = "out" identifier [ ":" type_name ] "{" out_method { condition_decl } "}".`
///
/// `type_name` is unresolved here (no `TypeRegistry` lookup), matching `CellDecl`. When
/// absent, the cell's type is inferred from `writer.body`'s result type by the compile phase
/// (`crate::parser::AdamParser`) — never here.
#[derive(Debug, Clone)]
pub struct OutDecl {
    /// The declared cell's name.
    pub name: String,
    /// The name token's span.
    pub name_span: ExprSpan,
    /// The `: type_name` annotation, if present.
    pub type_name: Option<(String, ExprSpan)>,
    /// The single writer method that computes this cell's value.
    pub writer: OutMethodDecl,
    /// This output's conditions, in declaration order.
    pub conditions: Vec<ConditionDecl>,
    /// A leading `//`/`/* */` comment immediately preceding this declaration, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<String>,
    /// Whether a blank line preceded this declaration, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `out ... { ... }` declaration.
    pub span: ExprSpan,
}

/// `out_method = "method" cell_list method_body.`
///
/// Unlike [`MethodDecl`], carries no `outputs` list: an out cell's writer always writes
/// exactly the enclosing [`OutDecl`]'s cell, so naming it again would be redundant.
#[derive(Debug, Clone)]
pub struct OutMethodDecl {
    /// The method's input cell names.
    pub inputs: Vec<(String, ExprSpan)>,
    /// The parsed method body expression.
    pub body: cel_parser::Expr,
    /// A leading comment immediately preceding this method, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<String>,
    /// Whether a blank line preceded this method, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `method [...] { ... }` declaration.
    pub span: ExprSpan,
}

/// `condition_decl = "condition" identifier cell_list "{" or_expression "}".`
///
/// `name` is a plain string label passed to `adam_rs::Sheet::add_output`, not a cell
/// reference — it may coincide with a cell name declared elsewhere in the sheet but doesn't
/// have to.
#[derive(Debug, Clone)]
pub struct ConditionDecl {
    /// The condition's declared name.
    pub name: String,
    /// The name token's span.
    pub name_span: ExprSpan,
    /// The condition's input cell names.
    pub inputs: Vec<(String, ExprSpan)>,
    /// The parsed condition body expression; must type-check as `bool`.
    pub body: cel_parser::Expr,
    /// A leading comment immediately preceding this condition, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<String>,
    /// Whether a blank line preceded this condition, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `condition ... { ... }` declaration.
    pub span: ExprSpan,
}
```

Update the `SheetItem` enum:

```rust
pub enum SheetItem {
    /// A `cell` declaration.
    Cell(CellDecl),
    /// A `relationship` declaration.
    Relationship(RelationshipDecl),
    /// A `conditional` declaration.
    Conditional(ConditionalDecl),
    /// An `out` declaration.
    Out(OutDecl),
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

Update its three inherent methods:

```rust
pub fn span(&self) -> ExprSpan {
    match self {
        SheetItem::Cell(c) => c.span,
        SheetItem::Relationship(r) => r.span,
        SheetItem::Conditional(c) => c.span,
        SheetItem::Out(o) => o.span,
        SheetItem::Error { span, .. } => *span,
    }
}
```

```rust
pub(crate) fn set_leading_comment(&mut self, comment: String) {
    match self {
        SheetItem::Cell(c) => c.leading_comment = Some(comment),
        SheetItem::Relationship(r) => r.leading_comment = Some(comment),
        SheetItem::Conditional(c) => c.leading_comment = Some(comment),
        SheetItem::Out(o) => o.leading_comment = Some(comment),
        SheetItem::Error {
            leading_comment, ..
        } => *leading_comment = Some(comment),
    }
}
```

```rust
pub(crate) fn set_blank_line_before(&mut self, value: bool) {
    match self {
        SheetItem::Cell(c) => c.blank_line_before = value,
        SheetItem::Relationship(r) => r.blank_line_before = value,
        SheetItem::Conditional(c) => c.blank_line_before = value,
        SheetItem::Out(o) => o.blank_line_before = value,
        SheetItem::Error {
            blank_line_before, ..
        } => *blank_line_before = value,
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p adam-lang sheet_item_span_reads_the_out_variant set_leading_comment_sets_the_out_variant`
Expected: PASS (2 tests).

- [ ] **Step 5: Run the full existing `adam-lang` test suite to check for regressions**

Run: `cargo test -p adam-lang`
Expected: PASS, same count as before plus the 2 new tests (adding an enum variant with an exhaustive `match` in every existing arm is a compile-time-checked, behavior-preserving change).

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add adam-lang/src/ast.rs
git commit -m "feat(adam-lang): add OutDecl/OutMethodDecl/ConditionDecl AST types"
```

---

### Task 2: AST parser (`ast_parser.rs`) + recovery keyword + grammar doc + trivia

**Files:**
- Modify: `adam-lang/src/ast_parser.rs`
- Modify: `adam-lang/src/token_cursor.rs`
- Modify: `adam-lang/src/lib.rs`
- Modify: `adam-lang/src/trivia.rs`

**Interfaces:**
- Consumes: `ast::OutDecl`/`OutMethodDecl`/`ConditionDecl`/`SheetItem::Out` (Task 1).
- Produces: `AdamAstParser::parse_str` now accepts `out`/`condition` source and returns `SheetItem::Out` items. `attach_trivia` now recovers comments/blank-lines for an `out`'s `condition`s (parity with how it already handles a `relationship`'s `method`s and a `conditional`'s `branches`).
- Consumed by: Task 3 (`fmt.rs`), Task 4 (`typecheck.rs`), both operate on `AdamAstParser`'s output — Task 3's formatter output depends on this task's `trivia.rs` change to reproduce comments/blank-lines around `condition`s correctly.

**Note:** Task 1's implementer discovered that `adam-lang/src/trivia.rs`'s `attach_trivia` also exhaustively matches on `SheetItem` (missed when this plan was originally written) and added a placeholder no-op arm for `SheetItem::Out` to keep the crate compiling. This task replaces that placeholder with real trivia recovery — see Step 3b below.

- [ ] **Step 1: Write the failing tests**

Add to `adam-lang/src/ast_parser.rs`'s existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn parse_out_with_explicit_type_and_no_conditions() {
    let sheet = AdamAstParser::new()
        .parse_str(
            r#"
            sheet s {
                cell width: f64 = 4.0;
                cell height: f64 = 3.0;
                out area: f64 {
                    method [width, height] { width * height }
                }
            }
        "#,
        )
        .unwrap();
    assert!(sheet.errors.is_empty());
    let ast::SheetItem::Out(out) = &sheet.items[2] else {
        panic!("expected Out");
    };
    assert_eq!(out.name, "area");
    assert_eq!(out.type_name.as_ref().map(|(n, _)| n.as_str()), Some("f64"));
    assert_eq!(out.writer.inputs.len(), 2);
    assert!(out.conditions.is_empty());
}

#[test]
fn parse_out_with_no_type_annotation() {
    let sheet = AdamAstParser::new()
        .parse_str("sheet s { out area { width } }")
        .unwrap();
    let ast::SheetItem::Out(out) = &sheet.items[0] else {
        panic!("expected Out");
    };
    assert!(out.type_name.is_none());
}

#[test]
fn parse_out_with_conditions_in_declaration_order() {
    let sheet = AdamAstParser::new()
        .parse_str(
            r#"
            sheet s {
                out area: f64 {
                    method [width, height] { width * height }
                    condition max_area [width, height, max_area] { width * height <= max_area }
                    condition max_width [width, max_width] { width <= max_width }
                }
            }
        "#,
        )
        .unwrap();
    let ast::SheetItem::Out(out) = &sheet.items[0] else {
        panic!("expected Out");
    };
    assert_eq!(out.conditions.len(), 2);
    assert_eq!(out.conditions[0].name, "max_area");
    assert_eq!(out.conditions[0].inputs.len(), 3);
    assert_eq!(out.conditions[1].name, "max_width");
}

#[test]
fn parse_malformed_out_is_recorded_as_an_error_item() {
    let sheet = AdamAstParser::new()
        .parse_str(
            r#"
            sheet s {
                cell good_before: i32 = 1;
                out area { bad }
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

Run: `cargo test -p adam-lang parse_out_with_explicit_type_and_no_conditions parse_out_with_no_type_annotation parse_out_with_conditions_in_declaration_order parse_malformed_out_is_recorded_as_an_error_item`
Expected: FAIL — `sheet.items[2]`/`sheet.items[0]` is `SheetItem::Error` (unrecognized `out` keyword) or a compile error, not `SheetItem::Out`.

- [ ] **Step 3: Add `out`/`condition` parsing to `ast_parser.rs`**

In `adam-lang/src/ast_parser.rs`, update `parse_sheet_item`'s match and error message:

```rust
/// `sheet_item = cell_decl | relationship_decl | conditional_decl | out_decl.`
fn parse_sheet_item(&mut self, cursor: &mut TokenCursor) -> Result<ast::SheetItem> {
    use cel_parser::lex_lexer::{HasSpan, Token};
    match cursor.peek_token() {
        Some(Token::Identifier(id)) if id == "cell" => {
            self.parse_cell_decl(cursor).map(ast::SheetItem::Cell)
        }
        Some(Token::Identifier(id)) if id == "relationship" => self
            .parse_relationship_decl(cursor)
            .map(ast::SheetItem::Relationship),
        Some(Token::Identifier(id)) if id == "conditional" => self
            .parse_conditional_decl(cursor)
            .map(ast::SheetItem::Conditional),
        Some(Token::Identifier(id)) if id == "out" => {
            self.parse_out_decl(cursor).map(ast::SheetItem::Out)
        }
        Some(tok) => Err(cel_parser::ParseError::new(
            "expected `cell`, `relationship`, `conditional`, or `out`",
            tok.span(),
        )),
        None => Err(cel_parser::ParseError::new(
            "unexpected end of input",
            proc_macro2::Span::call_site(),
        )),
    }
}
```

Add three new methods (placed after `parse_conditional_decl`/`parse_branch_relationships`, before `parse_method_decl` — grouping the `out`-related productions together):

```rust
/// `out_decl = "out" identifier [ ":" type_name ] "{" out_method { condition_decl } "}".`
fn parse_out_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::OutDecl> {
    let decl_start = cursor.peek_span();
    cursor.is_keyword("out");
    let (name, name_span) = cursor.consume_ident()?;
    let type_name = if cursor.consume_punct(":") {
        let (type_name, type_span) = cursor.consume_ident()?;
        Some((type_name, point(type_span)))
    } else {
        None
    };
    cursor.expect_open_brace()?;
    let writer = self.parse_out_method(cursor)?;
    let mut conditions = Vec::new();
    while matches!(cursor.peek_token(), Some(cel_parser::lex_lexer::Token::Identifier(id)) if id == "condition")
    {
        conditions.push(self.parse_condition_decl(cursor)?);
    }
    let close_span = cursor.expect_close_brace()?;
    Ok(ast::OutDecl {
        name,
        name_span: point(name_span),
        type_name,
        writer,
        conditions,
        leading_comment: None,
        blank_line_before: false,
        span: ast::ExprSpan {
            start: decl_start,
            end: close_span,
        },
    })
}

/// `out_method = "method" cell_list method_body.`
fn parse_out_method(&mut self, cursor: &mut TokenCursor) -> Result<ast::OutMethodDecl> {
    let decl_start = cursor.peek_span();
    if !cursor.is_keyword("method") {
        return Err(cursor.err_at("expected `method`"));
    }
    let inputs = parse_cell_list(cursor)?;
    cursor.expect_open_brace()?;
    let body = self.parse_cel_or_expression(cursor)?;
    let close_span = cursor.expect_close_brace()?;
    Ok(ast::OutMethodDecl {
        inputs,
        body,
        leading_comment: None,
        blank_line_before: false,
        span: ast::ExprSpan {
            start: decl_start,
            end: close_span,
        },
    })
}

/// `condition_decl = "condition" identifier cell_list "{" or_expression "}".`
fn parse_condition_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::ConditionDecl> {
    let decl_start = cursor.peek_span();
    cursor.is_keyword("condition");
    let (name, name_span) = cursor.consume_ident()?;
    let inputs = parse_cell_list(cursor)?;
    cursor.expect_open_brace()?;
    let body = self.parse_cel_or_expression(cursor)?;
    let close_span = cursor.expect_close_brace()?;
    Ok(ast::ConditionDecl {
        name,
        name_span: point(name_span),
        inputs,
        body,
        leading_comment: None,
        blank_line_before: false,
        span: ast::ExprSpan {
            start: decl_start,
            end: close_span,
        },
    })
}
```

In `adam-lang/src/token_cursor.rs`, update `skip_to_recovery_point`'s sheet-item-boundary keyword check (the `Some(Token::Identifier(id)) if at_or_below_target && (id == "cell" || ...)` arm) to also stop at `out` — a malformed item must not swallow a following, well-formed `out` declaration:

```rust
Some(Token::Identifier(id))
    if at_or_below_target
        && (id == "cell"
            || id == "relationship"
            || id == "conditional"
            || id == "out") =>
{
    return last;
}
```

(`condition` is deliberately *not* added here: it only ever appears nested inside an `out` block, never as a sheet-item boundary — a malformed `condition_decl` fails its whole enclosing `out_decl`, exactly like a malformed `method_decl` fails its whole enclosing `relationship_decl` today.)

In `adam-lang/src/lib.rs`, extend the `# Grammar` doc comment:

```text
//! sheet              = "sheet" identifier "{" { sheet_item } "}".
//! sheet_item         = cell_decl | relationship_decl | conditional_decl | out_decl.
//! cell_decl          = "cell" identifier cell_type_init ";".
//! cell_type_init     = (":" type_name [ "=" literal ]) | ("=" literal).
//! type_name          = identifier.
//! relationship_decl  = "relationship" [ identifier ] "{" { method_decl } "}".
//! conditional_decl   = "conditional" identifier "{" { conditional_branch } [ default_branch ] "}".
//! conditional_branch = literal "=>" "{" { relationship_decl } "}" [ "," ].
//! default_branch     = "_"   "=>" "{" { relationship_decl } "}" [ "," ].
//! method_decl        = "method" cell_list "->" cell_list method_body.
//! out_decl           = "out" identifier [ ":" type_name ] "{" out_method { condition_decl } "}".
//! out_method         = "method" cell_list method_body.
//! condition_decl     = "condition" identifier cell_list "{" or_expression "}".
//! cell_list          = "[" identifier { "," identifier } "]".
//! method_body        = "{" or_expression "}".
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-lang parse_out_with_explicit_type_and_no_conditions parse_out_with_no_type_annotation parse_out_with_conditions_in_declaration_order parse_malformed_out_is_recorded_as_an_error_item`
Expected: PASS (4 tests).

- [ ] **Step 5: Write the failing trivia test**

`adam-lang/src/trivia.rs`'s `attach_trivia` currently has a placeholder no-op arm for `SheetItem::Out` (added by Task 1 only to keep the crate compiling — see this task's Note above). Add to `adam-lang/src/trivia.rs`'s existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn attaches_a_comment_to_a_condition_inside_an_out_block() {
    let source = "sheet s {\n    out area: f64 {\n        method [width, height] { width * height }\n        // second\n        condition c [width] { width <= 10.0 }\n    }\n}";
    let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
    attach_trivia(source, &mut sheet);
    let crate::ast::SheetItem::Out(out) = &sheet.items[0] else {
        panic!("expected Out");
    };
    assert_eq!(out.conditions[0].leading_comment.as_deref(), Some("second"));
}
```

- [ ] **Step 6: Run the test to verify it fails**

Run: `cargo test -p adam-lang attaches_a_comment_to_a_condition_inside_an_out_block`
Expected: FAIL — the placeholder `SheetItem::Out(_out) => { /* TODO */ }` arm in `attach_trivia` never visits `out.conditions`, so `leading_comment` stays `None`.

- [ ] **Step 7: Add `attach_out` to `trivia.rs`**

In `adam-lang/src/trivia.rs`, update the `use` list to bring in the two new AST types:

```rust
use crate::ast::{
    ConditionDecl, ConditionalBranch, ConditionalDecl, ExprSpan, MethodDecl, OutDecl,
    RelationshipDecl, Sheet,
};
```

Add a `TriviaTarget` impl for `ConditionDecl` (placed after the existing `impl TriviaTarget for ConditionalBranch`):

```rust
impl TriviaTarget for ConditionDecl {
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
```

Replace the placeholder arm in `attach_trivia`'s match:

```rust
crate::ast::SheetItem::Out(out_decl) => attach_out(source, &line_starts, out_decl),
```

Add `attach_out` (placed after `attach_conditional`) — the writer method is always the out block's first item (nothing precedes it to attach a comment to, the same "first item in a list is never attached" limitation the module doc already documents for every other list), so only the `conditions` list needs a gap pass:

```rust
/// Recovers trivia for an out declaration's conditions. The writer method itself is always
/// first in the block — nothing precedes it to attach a comment to, the same limitation
/// documented in this module's doc comment for the first item of any sibling list.
fn attach_out(source: &str, line_starts: &[usize], out_decl: &mut OutDecl) {
    attach_gaps(source, line_starts, &mut out_decl.conditions);
}
```

- [ ] **Step 8: Run the test to verify it passes**

Run: `cargo test -p adam-lang attaches_a_comment_to_a_condition_inside_an_out_block`
Expected: PASS.

- [ ] **Step 9: Run the full existing `adam-lang` test suite to check for regressions**

Run: `cargo test -p adam-lang`
Expected: PASS, same pre-existing count plus the 4 `ast_parser.rs` tests from Step 4 and the 1 `trivia.rs` test from Step 8. In particular, every existing `recovery_*`/`parse_unknown_sheet_item_is_recorded_as_an_error_item` test in `ast_parser.rs`, and every existing `attach*`/`attaches_*` test in `trivia.rs`, must still pass unchanged — adding `out`/`condition` as new possibilities doesn't change how any `cell`/`relationship`/`conditional`/error case is recognized or how trivia is recovered for them.

- [ ] **Step 10: Commit**

```bash
cargo fmt --all
git add adam-lang/src/ast_parser.rs adam-lang/src/token_cursor.rs adam-lang/src/lib.rs adam-lang/src/trivia.rs
git commit -m "feat(adam-lang): parse out/condition declarations into the AST"
```

---

### Task 3: Formatter (`fmt.rs`)

**Files:**
- Modify: `adam-lang/src/fmt.rs`

**Interfaces:**
- Consumes: `ast::OutDecl`/`OutMethodDecl`/`ConditionDecl`/`SheetItem::Out` (Task 1), `AdamAstParser::parse_str` producing them (Task 2).
- Produces: `format_sheet` now round-trips `out`/`condition` source text.

- [ ] **Step 1: Write the failing tests**

Add to `adam-lang/src/fmt.rs`'s existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn formats_an_out_with_explicit_type_and_no_conditions() {
    let source = "sheet s {\n    out area: f64 {\n        method [width, height] { width * height }\n    }\n}";
    let expected = "sheet s {\n    out area: f64 {\n        method [width, height] { width * height }\n    }\n}\n";
    assert_eq!(format(source), expected);
}

#[test]
fn formats_an_out_with_no_type_annotation() {
    let source = "sheet s {\n    out area {\n        method [width] { width }\n    }\n}";
    let expected = "sheet s {\n    out area {\n        method [width] { width }\n    }\n}\n";
    assert_eq!(format(source), expected);
}

#[test]
fn formats_an_out_with_conditions_in_declaration_order() {
    let source = "sheet s {\n    out area: f64 {\n        method [width, height] { width * height }\n        condition max_area [width, height, max_area] { width * height <= max_area }\n    }\n}";
    let expected = "sheet s {\n    out area: f64 {\n        method [width, height] { width * height }\n        condition max_area [width, height, max_area] { width * height <= max_area }\n    }\n}\n";
    assert_eq!(format(source), expected);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang formats_an_out_with_explicit_type_and_no_conditions formats_an_out_with_no_type_annotation formats_an_out_with_conditions_in_declaration_order`
Expected: FAIL — `write_sheet_item`'s `match` has no `SheetItem::Out` arm (compile error) until Step 3 lands.

- [ ] **Step 3: Add `write_out`/`write_out_method`/`write_condition`**

In `adam-lang/src/fmt.rs`, add (placed after `write_cell`, before `write_sheet_item`):

```rust
/// Writes one `method [...] { ... }` writer declaration inside an `out` block — like
/// `write_method`, but with no `-> [...]` half: an out cell's writer always writes exactly the
/// enclosing declaration's cell, so naming it again would be redundant.
fn write_out_method(out: &mut String, method: &ast::OutMethodDecl, depth: usize) {
    write_trivia(
        out,
        method.blank_line_before,
        method.leading_comment.as_deref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str("method ");
    write_cell_list(out, &method.inputs);
    out.push_str(" { ");
    out.push_str(&cel_parser::format_expr(&method.body));
    out.push_str(" }\n");
}

/// Writes one `condition name [...] { ... }` declaration.
fn write_condition(out: &mut String, cond: &ast::ConditionDecl, depth: usize) {
    write_trivia(
        out,
        cond.blank_line_before,
        cond.leading_comment.as_deref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str("condition ");
    out.push_str(&cond.name);
    out.push(' ');
    write_cell_list(out, &cond.inputs);
    out.push_str(" { ");
    out.push_str(&cel_parser::format_expr(&cond.body));
    out.push_str(" }\n");
}

/// Writes one `out name[: type] { ... }` declaration: its writer method followed by its
/// conditions, in declaration order.
fn write_out(out: &mut String, decl: &ast::OutDecl, depth: usize) {
    write_trivia(
        out,
        decl.blank_line_before,
        decl.leading_comment.as_deref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str("out ");
    out.push_str(&decl.name);
    if let Some((type_name, _)) = &decl.type_name {
        out.push_str(": ");
        out.push_str(type_name);
    }
    out.push_str(" {\n");
    write_out_method(out, &decl.writer, depth + 1);
    for cond in &decl.conditions {
        write_condition(out, cond, depth + 1);
    }
    out.push_str(&indent(depth));
    out.push_str("}\n");
}
```

Update `write_sheet_item`'s match:

```rust
fn write_sheet_item(out: &mut String, item: &ast::SheetItem, depth: usize) {
    match item {
        ast::SheetItem::Cell(cell) => write_cell(out, cell, depth),
        ast::SheetItem::Relationship(rel) => write_relationship(out, rel, depth),
        ast::SheetItem::Conditional(cond) => write_conditional(out, cond, depth),
        ast::SheetItem::Out(out_decl) => write_out(out, out_decl, depth),
        ast::SheetItem::Error { .. } => {
            unreachable!("format_sheet is only called on a sheet with no recorded syntax errors")
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-lang formats_an_out_with_explicit_type_and_no_conditions formats_an_out_with_no_type_annotation formats_an_out_with_conditions_in_declaration_order`
Expected: PASS (3 tests).

- [ ] **Step 5: Run the full existing `adam-lang` test suite to check for regressions**

Run: `cargo test -p adam-lang`
Expected: PASS, same pre-existing count plus the 3 new tests.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add adam-lang/src/fmt.rs
git commit -m "feat(adam-lang): format out/condition declarations"
```

---

### Task 4: Typechecker (`typecheck.rs`)

**Files:**
- Modify: `adam-lang/src/typecheck.rs`

**Interfaces:**
- Consumes: `ast::OutDecl`/`ConditionDecl`/`SheetItem::Out` (Task 1), `AdamAstParser::parse_str` producing them (Task 2).
- Produces: `check_sheet` now flags a type mismatch between an `out`'s `: type_name` annotation and its writer body, and a condition body that doesn't type-check as `bool`.

- [ ] **Step 1: Write the failing tests**

Add to `adam-lang/src/typecheck.rs`'s existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn out_body_matching_its_annotation_has_no_diagnostic() {
    let sheet = parse(
        "sheet s { cell width: f64; cell height: f64; \
         out area: f64 { method [width, height] { width * height } } }",
    );
    let diags = check_sheet(&sheet, &TypeRegistry::new());
    assert!(diags.is_empty());
}

#[test]
fn out_body_mismatched_with_its_annotation_is_a_diagnostic() {
    let sheet = parse(
        "sheet s { cell width: f64; cell height: f64; \
         out area: i32 { method [width, height] { width * height } } }",
    );
    let diags = check_sheet(&sheet, &TypeRegistry::new());
    assert_eq!(diags.len(), 1);
}

#[test]
fn out_with_no_annotation_infers_its_type_and_has_no_diagnostic() {
    // No `: type_name` to cross-check against — nothing to flag, and a later reference to
    // `area`'s name (were one added) would resolve through the inferred f64, not Ty::Any.
    let sheet = parse(
        "sheet s { cell width: f64; cell height: f64; \
         out area { method [width, height] { width * height } } }",
    );
    let diags = check_sheet(&sheet, &TypeRegistry::new());
    assert!(diags.is_empty());
}

#[test]
fn condition_with_bool_body_has_no_diagnostic() {
    let sheet = parse(
        "sheet s { cell width: f64; cell max_width: f64; \
         out area: f64 { \
             method [width] { width } \
             condition max_width [width, max_width] { width <= max_width } \
         } }",
    );
    let diags = check_sheet(&sheet, &TypeRegistry::new());
    assert!(diags.is_empty());
}

#[test]
fn condition_with_non_bool_body_is_a_diagnostic() {
    let sheet = parse(
        "sheet s { cell width: f64; \
         out area: f64 { \
             method [width] { width } \
             condition bogus [width] { width } \
         } }",
    );
    let diags = check_sheet(&sheet, &TypeRegistry::new());
    assert_eq!(diags.len(), 1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang out_body_matching_its_annotation_has_no_diagnostic out_body_mismatched_with_its_annotation_is_a_diagnostic out_with_no_annotation_infers_its_type_and_has_no_diagnostic condition_with_bool_body_has_no_diagnostic condition_with_non_bool_body_is_a_diagnostic`
Expected: FAIL — `check_sheet`'s `match` has no `SheetItem::Out` arm (compile error) until Step 3 lands.

- [ ] **Step 3: Add the `Out` check**

In `adam-lang/src/typecheck.rs`, update the import line:

```rust
use crate::ast::{CellDecl, MethodDecl, OutDecl, Sheet, SheetItem};
```

Update `check_sheet`'s dispatch:

```rust
pub fn check_sheet(sheet: &Sheet, registry: &TypeRegistry) -> Vec<ParseError> {
    let mut diagnostics = Vec::new();
    let cell_types = declared_cell_types(sheet, registry);
    let resolve = |name: &str| -> Ty { cell_types.get(name).copied().unwrap_or(Ty::Any) };
    for item in &sheet.items {
        match item {
            SheetItem::Cell(cell) => check_cell_initializer(cell, registry, &mut diagnostics),
            SheetItem::Relationship(rel) => {
                for method in &rel.methods {
                    check_method(method, &resolve, &mut diagnostics);
                }
            }
            SheetItem::Conditional(cond) => {
                for branch in &cond.branches {
                    for rel in &branch.relationships {
                        for method in &rel.methods {
                            check_method(method, &resolve, &mut diagnostics);
                        }
                    }
                }
                if let Some(default_rels) = &cond.default {
                    for rel in default_rels {
                        for method in &rel.methods {
                            check_method(method, &resolve, &mut diagnostics);
                        }
                    }
                }
            }
            SheetItem::Out(out_decl) => check_out(out_decl, &resolve, &mut diagnostics),
            SheetItem::Error { .. } => {} // already reported as a syntax error; nothing to type-check
        }
    }
    diagnostics
}
```

Replace `declared_cell_types` with a version that also maps `out` names (second pass, `resolve_cells` deliberately sees only plain `cell` types — an `out` referencing *another* `out`'s inferred type resolves to `Ty::Any`, consistent with this checker's documented "not a complete type system" scope, matching every other unresolved identifier here):

```rust
/// Maps every declared cell name — from a `cell` or an `out` — to its `Ty`, for use as the
/// identifier resolver method/condition bodies are checked against. A `cell` with no
/// annotation, or one naming a type `registry` doesn't recognize, maps to `Ty::Any`. An `out`
/// with an annotation resolves the same way; one without is inferred from its writer body's
/// checked type, using only `cell`-declared types as context (not other `out`s' inferred
/// types — see this function's own note above).
fn declared_cell_types(
    sheet: &Sheet,
    registry: &TypeRegistry,
) -> std::collections::HashMap<String, Ty> {
    let mut map = std::collections::HashMap::new();
    for item in &sheet.items {
        if let SheetItem::Cell(cell) = item {
            let ty = cell
                .type_name
                .as_ref()
                .and_then(|(name, _)| registry.get(name))
                .map(|entry| Ty::from_type_id(entry.type_id))
                .unwrap_or(Ty::Any);
            map.insert(cell.name.clone(), ty);
        }
    }
    let resolve_cells = |name: &str| -> Ty { map.get(name).copied().unwrap_or(Ty::Any) };
    let mut out_types = std::collections::HashMap::new();
    for item in &sheet.items {
        if let SheetItem::Out(out_decl) = item {
            let ty = out_decl
                .type_name
                .as_ref()
                .and_then(|(name, _)| registry.get(name))
                .map(|entry| Ty::from_type_id(entry.type_id))
                .unwrap_or_else(|| check_expr(&out_decl.writer.body, &resolve_cells).0);
            out_types.insert(out_decl.name.clone(), ty);
        }
    }
    map.extend(out_types);
    map
}
```

Add `check_out` (placed after `check_method`):

```rust
/// Checks one `out`'s writer body against its optional `: type_name` annotation — mirroring
/// `check_method`'s single-output branch, since an out's writer is structurally a `method`
/// with one implicit output (the out cell itself) — and each of its conditions' bodies
/// against `Ty::Bool`. Operator-level diagnostics from inside any body (via `check_expr`) are
/// always included, regardless of whether a mismatch diagnostic is also added.
fn check_out(out_decl: &OutDecl, resolve: &impl Fn(&str) -> Ty, diagnostics: &mut Vec<ParseError>) {
    let (body_ty, body_diags) = check_expr(&out_decl.writer.body, resolve);
    diagnostics.extend(body_diags);
    if out_decl.type_name.is_some() {
        let declared = resolve(&out_decl.name);
        if !declared.unifies_with(&body_ty) {
            diagnostics.push(ParseError::new_range(
                format!(
                    "out `{}` body produces `{}`, but is declared `{}`",
                    out_decl.name,
                    body_ty.name(),
                    declared.name()
                ),
                out_decl.writer.body.span().start,
                out_decl.writer.body.span().end,
            ));
        }
    }
    for condition in &out_decl.conditions {
        let (cond_ty, cond_diags) = check_expr(&condition.body, resolve);
        diagnostics.extend(cond_diags);
        if !cond_ty.unifies_with(&Ty::Bool) {
            diagnostics.push(ParseError::new_range(
                format!(
                    "condition `{}` produces `{}`, but conditions must be `bool`",
                    condition.name,
                    cond_ty.name()
                ),
                condition.body.span().start,
                condition.body.span().end,
            ));
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-lang out_body_matching_its_annotation_has_no_diagnostic out_body_mismatched_with_its_annotation_is_a_diagnostic out_with_no_annotation_infers_its_type_and_has_no_diagnostic condition_with_bool_body_has_no_diagnostic condition_with_non_bool_body_is_a_diagnostic`
Expected: PASS (5 tests).

- [ ] **Step 5: Run the full existing `adam-lang` test suite to check for regressions**

Run: `cargo test -p adam-lang`
Expected: PASS, same pre-existing count plus the 5 new tests.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add adam-lang/src/typecheck.rs
git commit -m "feat(adam-lang): typecheck out/condition bodies"
```

---

### Task 5: Direct-to-`Sheet` parser (`parser.rs`)

**Files:**
- Modify: `adam-lang/src/parser.rs`

**Interfaces:**
- Consumes: `adam_rs::{Condition, OutputId}` (already public, per `adam-rs/src/lib.rs`), `adam_rs::Sheet::add_output(writer: Method, conditions: Vec<(&str, Condition)>) -> Result<OutputId, Error>`.
- Produces: `AdamParser::parse_str` accepts `out`/`condition` source, constructing real `adam_rs::Sheet` outputs/conditions. `ParsedSheet` gains `pub output_names: IndexMap<String, OutputId>` (parity with the existing `cell_names`).

This task is independent of Tasks 1–4 (see the design doc §5/§6: `parser.rs` and `ast_parser.rs` are two separate implementations of the same grammar, sharing only `token_cursor.rs`'s tokenizing primitives — already updated in Task 2, but that change doesn't affect `parser.rs`, which never calls `skip_to_recovery_point`).

- [ ] **Step 1: Write the failing tests**

Add to `adam-lang/src/parser.rs`'s existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn parse_out_with_explicit_type_propagates_correctly() {
    let mut sheet = parser()
        .parse_str(
            r#"
            sheet s {
                cell width: f64 = 4.0;
                cell height: f64 = 3.0;
                out area: f64 {
                    method [width, height] { width * height }
                }
            }
        "#,
        )
        .unwrap();
    sheet.propagate().unwrap();
    let output_id = *sheet.output_names.get("area").expect("area registered");
    let cell_id = sheet.output_cell(output_id).expect("output has a cell");
    assert_eq!(*sheet.read::<f64>(cell_id).unwrap(), 12.0);
}

#[test]
fn parse_out_with_no_annotation_infers_type_from_writer_body() {
    let sheet = parser()
        .parse_str(
            r#"
            sheet s {
                cell width: f64 = 4.0;
                out doubled { width + width }
            }
        "#,
        )
        .unwrap();
    let (_, type_id) = *sheet.cell_names.get("doubled").unwrap();
    assert_eq!(type_id, std::any::TypeId::of::<f64>());
}

#[test]
fn parse_out_type_mismatch_is_error() {
    let result = parser().parse_str(
        r#"
        sheet s {
            cell width: f64 = 4.0;
            out area: i32 { method [width] { width } }
        }
    "#,
    );
    assert!(result.is_err(), "f64 body for an i32 annotation must be an error");
}

#[test]
fn parse_out_with_conditions_reports_output_valid_and_violated() {
    let mut sheet = parser()
        .parse_str(
            r#"
            sheet s {
                cell width: f64 = 4.0;
                cell height: f64 = 3.0;
                cell max_area: f64 = 100.0;
                out area: f64 {
                    method [width, height] { width * height }
                    condition max_area [width, height, max_area] { width * height <= max_area }
                }
            }
        "#,
        )
        .unwrap();
    sheet.propagate().unwrap();
    let output_id = *sheet.output_names.get("area").unwrap();
    assert!(sheet.output_valid(output_id));
    assert_eq!(sheet.violated_conditions(output_id).count(), 0);
}

#[test]
fn parse_out_condition_violation_is_reported_after_propagate() {
    let mut sheet = parser()
        .parse_str(
            r#"
            sheet s {
                cell width: f64 = 40.0;
                cell height: f64 = 30.0;
                cell max_area: f64 = 100.0;
                out area: f64 {
                    method [width, height] { width * height }
                    condition max_area [width, height, max_area] { width * height <= max_area }
                }
            }
        "#,
        )
        .unwrap();
    sheet.propagate().unwrap();
    let output_id = *sheet.output_names.get("area").unwrap();
    assert!(!sheet.output_valid(output_id));
    assert_eq!(sheet.violated_conditions(output_id).count(), 1);
}

#[test]
fn parse_condition_non_bool_body_is_error() {
    let result = parser().parse_str(
        r#"
        sheet s {
            cell width: f64 = 4.0;
            out area: f64 {
                method [width] { width }
                condition bogus [width] { width }
            }
        }
    "#,
    );
    assert!(result.is_err(), "an f64 condition body must be an error");
}

#[test]
fn parse_out_duplicate_condition_names_is_error() {
    let result = parser().parse_str(
        r#"
        sheet s {
            cell width: f64 = 4.0;
            out area: f64 {
                method [width] { width }
                condition dup [width] { width <= 10.0 }
                condition dup [width] { width >= 0.0 }
            }
        }
    "#,
    );
    assert!(result.is_err(), "two conditions sharing a name must be an error");
}

#[test]
fn parse_out_cell_referenced_elsewhere_is_terminal_cell_error() {
    let result = parser().parse_str(
        r#"
        sheet s {
            cell width: f64 = 4.0;
            cell height: f64 = 3.0;
            out area: f64 { method [width, height] { width * height } }
            relationship { method [area] -> [width] { area } }
        }
    "#,
    );
    assert!(
        result.is_err(),
        "referencing an out cell as another relationship's input must be an error"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang parse_out_with_explicit_type_propagates_correctly parse_out_with_no_annotation_infers_type_from_writer_body parse_out_type_mismatch_is_error parse_out_with_conditions_reports_output_valid_and_violated parse_out_condition_violation_is_reported_after_propagate parse_condition_non_bool_body_is_error parse_out_duplicate_condition_names_is_error parse_out_cell_referenced_elsewhere_is_terminal_cell_error`
Expected: FAIL — `out` is not yet a recognized `parse_sheet_item` keyword (`ParsedSheet` also has no `output_names` field), so every test errors out or fails to compile.

- [ ] **Step 3: Extract the shared input-scope body parser**

`parse_method_body` currently combines two responsibilities: pushing an input-name scope and compiling the `{ or_expression }` body (needed by `out`'s writer and `condition` too, neither of which has a declared `outputs` list to check against yet), and checking the compiled result against a declared `outputs` list (specific to `relationship`'s `method_decl`). Extract the first half.

Replace the existing `parse_method_body` in `adam-lang/src/parser.rs` with:

```rust
/// Parses a `{ or_expression }` body with `inputs` cells available as a resolvable
/// identifier scope, pushed for the duration of the parse and popped afterward regardless of
/// success or failure of the body's own parse (mirrors the push/pop pairing already used by
/// `parse_method_body`'s single call site before this extraction).
///
/// - Complexity: O(k) to build the scope's dispatch table, where k = `inputs.len()`.
fn parse_body_with_input_scope(
    &mut self,
    ctx: &mut ParseContext,
    inputs: &[(String, CellId, TypeId)],
) -> Result<DynSegment> {
    ctx.expect_open_brace()?;

    let scope_data: Vec<(String, PushArgFn, usize)> = inputs
        .iter()
        .enumerate()
        .map(|(idx, (name, _, type_id))| {
            let fn_ptr = self
                .types
                .entry_by_type_id(*type_id)
                .expect("input cell type registered")
                .push_arg_fn;
            (name.clone(), fn_ptr, idx)
        })
        .collect();

    self.cel
        .op_lookup_mut()
        .push_scope(move |name, segment, arity, _span| {
            if arity != 0 {
                return Ok(false);
            }
            for (n, fn_ptr, idx) in &scope_data {
                if n == name {
                    fn_ptr(segment, *idx);
                    return Ok(true);
                }
            }
            Ok(false)
        });

    let result = self.parse_cel_or_expression(ctx);
    self.cel.op_lookup_mut().pop_scope();
    let segment = result?;

    ctx.expect_close_brace()?;
    Ok(segment)
}

/// `method_body = "{" or_expression "}".`
///
/// Returns the compiled body segment and how to split its result across `outputs`:
/// one output takes the segment's single result directly; more than one requires
/// the result to be a tuple of matching arity and element types, split via
/// `call_dyn_tuple`.
fn parse_method_body(
    &mut self,
    ctx: &mut ParseContext,
    inputs: &[(String, CellId, TypeId)],
    outputs: &[(String, CellId, TypeId)],
) -> Result<(DynSegment, CompiledOutputs)> {
    let segment = self.parse_body_with_input_scope(ctx, inputs)?;

    let compiled = if outputs.len() == 1 {
        let (out_name, _, out_type_id) = &outputs[0];
        let actual_type_id = segment.peek_output_type_id().ok_or_else(|| {
            ctx.err_at(format!("output `{out_name}`: expression produced no value"))
        })?;
        if actual_type_id != *out_type_id {
            let expected = self
                .types
                .entry_by_type_id(*out_type_id)
                .map(|e| e.type_name)
                .unwrap_or("?");
            let got = self
                .types
                .entry_by_type_id(actual_type_id)
                .map(|e| e.type_name)
                .unwrap_or("?");
            return Err(ctx.err_at(format!(
                "output `{out_name}`: type mismatch: expected `{expected}`, got `{got}`"
            )));
        }
        let call_fn = self
            .types
            .entry_by_type_id(*out_type_id)
            .expect("output cell type registered")
            .call_dyn_fn;
        CompiledOutputs::Single(call_fn)
    } else {
        let arity = segment.peek_tuple_arity().unwrap_or(0);
        if arity != outputs.len() {
            return Err(ctx.err_at(format!(
                "output expression has arity {arity} but method declares {} output(s)",
                outputs.len()
            )));
        }
        let element_type_ids: Vec<TypeId> = segment.peek_stack_infos(1)[0]
            .associated
            .iter()
            .map(|a| a.type_id)
            .collect();

        let mut extractors = Vec::with_capacity(outputs.len());
        for (i, ((out_name, _, out_type_id), actual_type_id)) in
            outputs.iter().zip(&element_type_ids).enumerate()
        {
            if actual_type_id != out_type_id {
                let expected = self
                    .types
                    .entry_by_type_id(*out_type_id)
                    .map(|e| e.type_name)
                    .unwrap_or("?");
                let got = self
                    .types
                    .entry_by_type_id(*actual_type_id)
                    .map(|e| e.type_name)
                    .unwrap_or("?");
                return Err(ctx.err_at(format!(
                    "output {i} `{out_name}`: type mismatch: expected `{expected}`, got `{got}`"
                )));
            }
            let entry = self
                .types
                .entry_by_type_id(*out_type_id)
                .expect("output cell type registered");
            extractors.push((*out_type_id, entry.extract_box_fn));
        }
        CompiledOutputs::Tuple(extractors)
    };

    Ok((segment, compiled))
}
```

- [ ] **Step 4: Run the full existing test suite to verify the refactor is behavior-preserving**

Run: `cargo test -p adam-lang`
Expected: PASS, exact same test count as before Step 3 (this step only extracted a helper; it must not change any observable behavior).

- [ ] **Step 5: Add `parse_out_decl`/`parse_condition_decl`, wire up dispatch, and add `output_names`**

In `adam-lang/src/parser.rs`, update the imports at the top of the file:

```rust
use adam_rs::{CellId, Condition, Method, OutputId, RelationshipId, Sheet};
```

Add `output_names` to `ParsedSheet` and `ParseContext`:

```rust
pub struct ParsedSheet {
    /// The constructed sheet.
    pub sheet: Sheet,
    /// Cell name → `(CellId, TypeId)`, in declaration order.
    pub cell_names: IndexMap<String, (CellId, TypeId)>,
    /// Output name → `OutputId`, in declaration order — parity with `cell_names`, for callers
    /// that need to look up `Sheet::output_valid`/`Sheet::violated_conditions` by name.
    pub output_names: IndexMap<String, OutputId>,
}
```

```rust
struct ParseContext {
    cursor: crate::token_cursor::TokenCursor,
    sheet: Sheet,
    cell_names: IndexMap<String, (CellId, TypeId)>,
    output_names: IndexMap<String, OutputId>,
}
```

Update both places `ParseContext`/`ParsedSheet` are constructed in `AdamParser::parse_str`:

```rust
pub fn parse_str(&mut self, source: &str) -> Result<ParsedSheet> {
    let stream =
        TokenStream::from_str(source).map_err(|e| ParseError::from_lex_error(source, e))?;
    let mut ctx = ParseContext {
        cursor: crate::token_cursor::TokenCursor::new(
            LexLexer::new(stream.into_iter()).peekable(),
        ),
        sheet: Sheet::new(),
        cell_names: IndexMap::new(),
        output_names: IndexMap::new(),
    };
    self.parse_sheet(&mut ctx)?;
    if let Some(tok) = ctx.peek_token() {
        return Err(ParseError::new("unexpected token", tok.span()));
    }
    Ok(ParsedSheet {
        sheet: ctx.sheet,
        cell_names: ctx.cell_names,
        output_names: ctx.output_names,
    })
}
```

Update `parse_sheet_item`'s dispatch and error message:

```rust
/// `sheet_item = cell_decl | relationship_decl | conditional_decl | out_decl.`
fn parse_sheet_item(&mut self, ctx: &mut ParseContext) -> Result<()> {
    match ctx.peek_token() {
        Some(Token::Identifier(id)) if id == "cell" => self.parse_cell_decl(ctx),
        Some(Token::Identifier(id)) if id == "relationship" => {
            self.parse_relationship_decl(ctx).map(|_| ())
        }
        Some(Token::Identifier(id)) if id == "conditional" => self.parse_conditional_decl(ctx),
        Some(Token::Identifier(id)) if id == "out" => self.parse_out_decl(ctx),
        Some(tok) => Err(ParseError::new(
            "expected `cell`, `relationship`, `conditional`, or `out`",
            tok.span(),
        )),
        None => Err(ParseError::new(
            "unexpected end of input",
            Span::call_site(),
        )),
    }
}
```

Add `parse_out_decl` and `parse_condition_decl` (placed after `parse_branch_relationships`, before `parse_method_decl` — grouping the new productions with the shared body-parsing helper they both call):

```rust
/// `out_decl = "out" identifier [ ":" type_name ] "{" out_method { condition_decl } "}".`
fn parse_out_decl(&mut self, ctx: &mut ParseContext) -> Result<()> {
    ctx.is_keyword("out"); // consume
    let (name, name_span) = ctx.consume_ident()?;
    if ctx.cell_names.contains_key(&name) {
        return Err(ParseError::new(
            format!("duplicate cell `{name}`"),
            name_span,
        ));
    }

    let declared: Option<(TypeId, AddCellFn)> = if ctx.consume_punct(":") {
        let (type_name, type_span) = ctx.consume_ident()?;
        let entry = self.types.get(&type_name).ok_or_else(|| {
            ParseError::new(format!("unknown type `{type_name}`"), type_span)
        })?;
        Some((entry.type_id, entry.add_cell_fn))
    } else {
        None
    };

    ctx.expect_open_brace()?;

    if !ctx.is_keyword("method") {
        return Err(ctx.err_at("expected `method`"));
    }
    let inputs = self.parse_cell_list(ctx)?;
    let segment = self.parse_body_with_input_scope(ctx, &inputs)?;

    let actual_type_id = segment
        .peek_output_type_id()
        .ok_or_else(|| ctx.err_at(format!("out `{name}`: expression produced no value")))?;

    let (out_type_id, add_fn) = match declared {
        Some((declared_type_id, add_fn)) => {
            if actual_type_id != declared_type_id {
                let expected = self
                    .types
                    .entry_by_type_id(declared_type_id)
                    .map(|e| e.type_name)
                    .unwrap_or("?");
                let got = self
                    .types
                    .entry_by_type_id(actual_type_id)
                    .map(|e| e.type_name)
                    .unwrap_or("?");
                return Err(ctx.err_at(format!(
                    "out `{name}`: type mismatch: expected `{expected}`, got `{got}`"
                )));
            }
            (declared_type_id, add_fn)
        }
        None => {
            let entry = self.types.entry_by_type_id(actual_type_id).ok_or_else(|| {
                ctx.err_at(format!(
                    "out `{name}`: cannot infer a type for this expression; register a type \
                     name for it or add an explicit `: type_name` annotation"
                ))
            })?;
            (entry.type_id, entry.add_cell_fn)
        }
    };

    let default_fn = self
        .types
        .entry_by_type_id(out_type_id)
        .and_then(|e| e.default_fn)
        .ok_or_else(|| ctx.err_at(format!("out `{name}`: type has no default value")))?;

    let cell_id = add_fn(&mut ctx.sheet, default_fn());
    ctx.cell_names.insert(name.clone(), (cell_id, out_type_id));

    let call_fn = self
        .types
        .entry_by_type_id(out_type_id)
        .expect("output cell type registered")
        .call_dyn_fn;
    let writer = build_method(
        inputs,
        vec![(name.clone(), cell_id, out_type_id)],
        segment,
        CompiledOutputs::Single(call_fn),
    );

    let mut condition_names: Vec<String> = Vec::new();
    let mut conditions: Vec<Condition> = Vec::new();
    while matches!(ctx.peek_token(), Some(Token::Identifier(id)) if id == "condition") {
        let (cond_name, condition) = self.parse_condition_decl(ctx)?;
        condition_names.push(cond_name);
        conditions.push(condition);
    }

    ctx.expect_close_brace()?;

    let named_conditions: Vec<(&str, Condition)> = condition_names
        .iter()
        .map(String::as_str)
        .zip(conditions)
        .collect();

    let output_id = ctx
        .sheet
        .add_output(writer, named_conditions)
        .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
    ctx.output_names.insert(name, output_id);

    Ok(())
}

/// `condition_decl = "condition" identifier cell_list "{" or_expression "}".`
fn parse_condition_decl(&mut self, ctx: &mut ParseContext) -> Result<(String, Condition)> {
    ctx.is_keyword("condition"); // consume
    let (name, _name_span) = ctx.consume_ident()?;
    let inputs = self.parse_cell_list(ctx)?;
    let segment = self.parse_body_with_input_scope(ctx, &inputs)?;

    let bool_type_id = TypeId::of::<bool>();
    let actual_type_id = segment.peek_output_type_id().ok_or_else(|| {
        ctx.err_at(format!("condition `{name}`: expression produced no value"))
    })?;
    if actual_type_id != bool_type_id {
        let got = self
            .types
            .entry_by_type_id(actual_type_id)
            .map(|e| e.type_name)
            .unwrap_or("?");
        return Err(ctx.err_at(format!(
            "condition `{name}`: expected `bool`, got `{got}`"
        )));
    }

    let call_fn = self
        .types
        .get("bool")
        .expect("bool always registered")
        .call_dyn_fn;
    let input_ids: Vec<CellId> = inputs.iter().map(|(_, id, _)| *id).collect();
    let input_types: Vec<TypeId> = inputs.iter().map(|(_, _, tid)| *tid).collect();
    let segment = RefCell::new(segment);
    let condition = Condition::new(input_ids, input_types, move |args| {
        let seg = &mut *segment.borrow_mut();
        let boxed = call_fn(seg, args)?;
        Ok(*boxed
            .downcast::<bool>()
            .expect("checked TypeId::of::<bool>() above"))
    });

    Ok((name, condition))
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p adam-lang parse_out_with_explicit_type_propagates_correctly parse_out_with_no_annotation_infers_type_from_writer_body parse_out_type_mismatch_is_error parse_out_with_conditions_reports_output_valid_and_violated parse_out_condition_violation_is_reported_after_propagate parse_condition_non_bool_body_is_error parse_out_duplicate_condition_names_is_error parse_out_cell_referenced_elsewhere_is_terminal_cell_error`
Expected: PASS (8 tests).

- [ ] **Step 7: Run the full existing `adam-lang` test suite, then the whole workspace, to check for regressions**

Run: `cargo test -p adam-lang`
Expected: PASS, same pre-existing count plus all new tests from Tasks 1–5.

Run: `cargo test --workspace` and `cargo test --doc --workspace`
Expected: PASS — no other crate references `adam-lang`'s `ParsedSheet`/`AdamParser` in a way this change could break (`begin` uses `ParsedSheet` via `Deref`/`DerefMut` to `Sheet` only, per `adam-lang/src/parser.rs`'s own doc comment; adding a field doesn't affect that).

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Expected: PASS, zero warnings.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add adam-lang/src/parser.rs
git commit -m "feat(adam-lang): parse out/condition declarations into a live Sheet"
```

---

## Post-plan verification

After Task 5, run the full project check suite from `CLAUDE.md` before considering this feature done:

```bash
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

All must pass with zero warnings before opening a PR (per `CLAUDE.md`'s Git Workflow section). This feature does not touch `begin`, so the two `begin`-specific clippy invocations are expected to be unaffected — run them anyway, per the standing project rule.
