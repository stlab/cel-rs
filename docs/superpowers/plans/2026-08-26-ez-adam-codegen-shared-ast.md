# ez-adam Codegen Revision: Shared AST/Formatter Implementation Plan

> **Status (implemented, with one superseded task):** The shared-AST/`format_sheet` codegen landed in PR #150. **Task 2 (`adam-lang`: `MatchLiteral`) and its dependent `adam-lang` AST-parser/formatter changes were superseded** — `ez-adam` emits multi-cell `Cells`-mode conditionals as conjunction-based decomposition (`build_decomposed_multi_cell_conditionals`) instead of a tuple `MatchLiteral` branch key, so no `MatchLiteral` enum was added to `adam-lang`. Full tuple-branch-key support in `adam-lang` is tracked as [#173](https://github.com/stlab/cel-rs/issues/173). Read the `MatchLiteral` tasks/architecture below as historical planning context, not the shipped approach.

---

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `ez-adam`'s hand-rolled `.adm2` string generation with construction of `adam_lang::ast::Sheet` plus the existing, shared `format_sheet` — closing the "two independent serialization paths" gap — and extend `adam-lang`'s AST to support tuple conditional-branch keys, which the direct parser already accepts but the AST-only side cannot represent.

**Architecture:** A new `cel_parser::ExprSpan::for_text` helper produces spans with real backing source text for hand-built AST leaves. `adam_lang::ast::ConditionalBranch.literal` becomes a new `MatchLiteral` enum (`Scalar`/`Tuple`), with matching support in `ast_parser.rs` and `fmt.rs`. `ez-adam`'s `codegen` module gains an `ast_builder` submodule that translates `Document` into `ast::Sheet` (parsing stored formula/restrict/clamp-body text into `cel_parser::Expr` via the same parser `validation::validate_cel_expression` already uses), and `generate_adm2` becomes `Result<String, ExportError>`.

**Tech Stack:** Rust 2024, `proc_macro2` token spans, `adam-lang`'s existing AST/parser/formatter, `cel-parser`.

**Spec:** `docs/superpowers/specs/2026-08-26-ez-adam-codegen-shared-ast-design.md`

## Global Constraints

- Every function (public or private) needs a `///` contract-style doc comment — this workspace's `missing_docs` lint only catches public items; private helpers need docs too, per this repo's own convention (re-established after being caught missing multiple times during Phase 1).
- Every commit step runs `cargo fmt --all` first.
- Precondition violations are checked with `debug_assert!`, never a `Result` — this applies to `ExprSpan::for_text`'s "exactly one token" precondition.
- `generate_adm2`'s new fallibility is a genuine `Result`, not a precondition — a still-empty/invalid formula is expected, reachable state (the sketch's own "[ ]" placeholder), not a caller bug.
- Do not pre-trust any hand-computed "expected `.adm2` output" string for a nested (relationship-inside-conditional) construct — `format_sheet`'s exact whitespace/blank-line conventions for nesting are not fully known until observed. Tasks that need such a string say so explicitly: implement, run, print/inspect the actual output, then write the assertion to match what's actually produced (verifying it's still valid `.adm2` by eye against the grammar) — never guess and leave it unverified.
- `restrict` and `output` remain unemitted (issues #146, #147) — this plan changes serialization mechanism only, not what gets emitted.

---

### Task 1: `cel-parser`: `ExprSpan::for_text` helper

**Files:**
- Modify: `cel-parser/src/ast.rs` (wherever `ExprSpan` is defined)

**Interfaces:**
- Produces: `ExprSpan::for_text(text: &str) -> ExprSpan`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)]` module in the same file as `ExprSpan`'s definition (create one if none exists there yet, following this file's existing test-module convention):

```rust
#[test]
fn for_text_produces_a_span_whose_source_text_matches() {
    let span = ExprSpan::for_text("i64");
    assert_eq!(span.start.source_text().as_deref(), Some("i64"));
    assert_eq!(span.end.source_text().as_deref(), Some("i64"));
}

#[test]
fn for_text_works_for_a_string_literal() {
    let span = ExprSpan::for_text("\"hello\"");
    assert_eq!(span.start.source_text().as_deref(), Some("\"hello\""));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-parser for_text_produces_a_span_whose_source_text_matches`
Expected: FAIL (`ExprSpan::for_text` doesn't exist yet — compile error).

- [ ] **Step 3: Implement**

Add to `ExprSpan`'s `impl` block (or create one):

```rust
impl ExprSpan {
    /// Creates a span whose `source_text()` returns exactly `text`, by
    /// tokenizing it and taking the resulting token's span. For hand-built
    /// AST nodes (not produced by parsing real source), this gives leaves
    /// like a type name or literal a span the formatter can read back
    /// text from, the same way `cel-rs-macros` already does internally.
    ///
    /// - Precondition: `text` tokenizes to exactly one token tree.
    #[must_use]
    pub fn for_text(text: &str) -> Self {
        let tokens: proc_macro2::TokenStream =
            text.parse().expect("ExprSpan::for_text: text must be valid tokens");
        let mut iter = tokens.into_iter();
        let first = iter.next().expect("ExprSpan::for_text: text must be non-empty");
        debug_assert!(
            iter.next().is_none(),
            "ExprSpan::for_text: text must tokenize to exactly one token"
        );
        let span = first.span();
        ExprSpan { start: span, end: span }
    }
}
```

(Adjust the exact `use` path for `proc_macro2` to match this file's existing imports.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-parser for_text`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-parser/src/ast.rs
git commit -m "feat(cel-parser): add ExprSpan::for_text for hand-built AST spans"
```

---

### Task 2: `adam-lang`: `MatchLiteral` — type, parsing, formatting

**Files:**
- Modify: `adam-lang/src/ast.rs` (add `MatchLiteral`, change `ConditionalBranch.literal`'s type)
- Modify: `adam-lang/src/ast_parser.rs` (`parse_conditional_decl`'s branch-key parsing)
- Modify: `adam-lang/src/fmt.rs` (`write_branch`'s literal rendering)

**Interfaces:**
- Produces: `pub enum MatchLiteral { Scalar(cel_parser::lex_lexer::Literal), Tuple(Vec<MatchLiteral>) }`; `ConditionalBranch.literal: MatchLiteral` (was `cel_parser::lex_lexer::Literal`).

This task must land as one unit — changing the field type alone leaves `ast_parser.rs`/`fmt.rs` non-compiling until both are updated to match.

- [ ] **Step 1: Write the failing tests**

Add to `adam-lang/src/ast.rs`'s existing `#[cfg(test)]` module:

```rust
#[test]
fn match_literal_scalar_and_tuple_are_distinct() {
    let a = MatchLiteral::Scalar(cel_parser::lex_lexer::Literal::Bool(true));
    let b = MatchLiteral::Tuple(vec![
        MatchLiteral::Scalar(cel_parser::lex_lexer::Literal::Bool(true)),
        MatchLiteral::Scalar(cel_parser::lex_lexer::Literal::Bool(false)),
    ]);
    assert_ne!(format!("{a:?}"), format!("{b:?}"));
}
```

(Adjust `Literal::Bool`'s exact constructor to match `cel_parser::lex_lexer::Literal`'s real shape if it differs — check `cel-parser/src/ast.rs`'s `Literal` enum definition first; this is a `#[derive(Debug)]` sanity test only, not exercising real parsing yet.)

Add to `adam-lang/src/ast_parser.rs`'s test module (find or create it, following this file's existing convention — look at how other `parse_*_decl` functions are tested in this file for the exact harness/helper pattern used, e.g. constructing an `AdamAstParser` and calling `parse_str`):

```rust
#[test]
fn parses_a_multi_cell_tuple_conditional_branch() {
    let mut parser = AdamAstParser::new();
    let sheet = parser
        .parse_str("sheet s { cell a: bool; cell b: bool; conditional (a, b) { (true, false) => { relationship { a := b; } } _ => { } } }")
        .unwrap();
    let SheetItem::Conditional(cond) = &sheet.items[2] else {
        panic!("expected a Conditional item");
    };
    assert_eq!(cond.branches.len(), 1);
    match &cond.branches[0].literal {
        MatchLiteral::Tuple(elements) => assert_eq!(elements.len(), 2),
        MatchLiteral::Scalar(_) => panic!("expected a Tuple match literal"),
    }
}
```

Add to `adam-lang/src/fmt.rs`'s test module:

```rust
#[test]
fn formats_a_tuple_conditional_branch() {
    let source = "sheet s { cell a: bool; cell b: bool; conditional (a, b) { (true, false) => { relationship { a := b; } } _ => { } } }";
    let mut parser = AdamAstParser::new();
    let mut sheet = parser.parse_str(source).unwrap();
    attach_trivia(&mut sheet, source);
    let out = format_sheet(&sheet);
    assert!(out.contains("(true, false) => {"));
}
```

(Adjust `AdamAstParser::new()`'s exact constructor call and `attach_trivia`'s exact signature to match this file's existing doctest/tests — copy the pattern from `format_sheet`'s own doctest at `fmt.rs:349-366` rather than guessing.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang match_literal parses_a_multi_cell_tuple_conditional_branch formats_a_tuple_conditional_branch`
Expected: FAIL (compile error — `MatchLiteral` doesn't exist; `(a, b) { (true, false) => ...` doesn't parse yet in the AST-only parser).

- [ ] **Step 3: Implement — `ast.rs`**

Add near `ConditionalBranch`'s definition:

```rust
/// A conditional branch's match key: a single literal, or a parenthesized
/// tuple of them (mirroring a multi-cell condition's tuple value, e.g.
/// `(false, true) => { ... }`). The direct parser (`adam-lang/src/parser.rs`)
/// already accepts this via a general `or_expression`; this type brings the
/// AST-only side (used by `format_sheet`/`adam-fmt`) up to the same
/// capability for conditional branch keys specifically.
#[derive(Debug, Clone)]
pub enum MatchLiteral {
    Scalar(cel_parser::lex_lexer::Literal),
    Tuple(Vec<MatchLiteral>),
}
```

Change `ConditionalBranch.literal`'s field type from `cel_parser::lex_lexer::Literal` to `MatchLiteral`.

- [ ] **Step 4: Implement — `ast_parser.rs`**

Find `parse_conditional_decl`'s branch-key parsing (currently a single `cursor.consume_literal()` call). Replace with a new helper that tries a parenthesized group first:

```rust
/// `match_literal = literal | "(" match_literal { "," match_literal } [ "," ] ")".`
fn parse_match_literal(&mut self, cursor: &mut TokenCursor) -> Result<MatchLiteral> {
    if cursor.peek_is_open_paren() {
        cursor.consume_open_paren()?;
        let mut elements = Vec::new();
        while !cursor.peek_is_close_paren() {
            elements.push(self.parse_match_literal(cursor)?);
            if cursor.peek_is_comma() {
                cursor.consume_comma()?;
            } else {
                break;
            }
        }
        cursor.consume_close_paren()?;
        Ok(MatchLiteral::Tuple(elements))
    } else {
        Ok(MatchLiteral::Scalar(cursor.consume_literal()?))
    }
}
```

(This is a sketch — match the exact `TokenCursor` method names this file's existing parsing functions already use for open/close paren and comma handling; do not invent new `TokenCursor` methods without checking whether equivalents already exist under different names — e.g. how does `parse_type_expr`'s tuple-type parsing in this same file handle its parens/commas? Mirror that exactly, since `TypeExpr::Tuple` already parses a structurally identical `"(" X { "," X } ")"` shape.)

Update `parse_conditional_decl`'s call site to call `self.parse_match_literal(cursor)?` instead of `cursor.consume_literal()`.

- [ ] **Step 5: Implement — `fmt.rs`**

Find `write_branch`'s literal-rendering step (currently calling `source_text_or_empty(branch.literal_span)` or similar for a scalar `Literal`). Add a `write_match_literal` helper and call it instead:

```rust
/// Renders `literal`'s `.adm2` spelling: the branch's own recorded source
/// text for a scalar (see `source_text_or_empty`), or a parenthesized,
/// comma-separated rendering of each element for a tuple.
fn write_match_literal(literal: &MatchLiteral, span: &ExprSpan) -> String {
    match literal {
        MatchLiteral::Scalar(_) => source_text_or_empty(span),
        MatchLiteral::Tuple(elements) => {
            // Each element's own span isn't separately tracked yet (only
            // the whole branch's `literal_span` is) — for a Tuple, render
            // the whole tuple's source text directly, the same way a
            // scalar does, rather than recursing per-element.
            source_text_or_empty(span)
        }
    }
}
```

(This sketch renders a tuple branch key by reading back the WHOLE tuple's original source text via the branch's existing single `literal_span` — simpler than tracking a span per tuple element, and sufficient since `ConditionalBranch.literal_span` already spans the entire `(true, false)` text when parsed. Confirm during implementation whether `literal_span` as currently populated by `parse_conditional_decl` covers the whole parenthesized group or just the first literal — if it's the latter, `parse_match_literal`'s tuple branch in Step 4 must widen `literal_span` to cover the whole group before this will work; check and fix in the same task if so.)

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p adam-lang match_literal parses_a_multi_cell_tuple_conditional_branch formats_a_tuple_conditional_branch`
Expected: 3 passed.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add adam-lang/src/ast.rs adam-lang/src/ast_parser.rs adam-lang/src/fmt.rs
git commit -m "feat(adam-lang): support tuple match literals in conditional branches"
```

---

### Task 3: `adam-lang`: fix downstream breakage from the `MatchLiteral` type change

**Files:**
- Modify: whatever `cargo build --workspace --exclude begin` reveals (likely candidates: `adam-lsp/src/*.rs`, `editors/vscode-adam-lang` if it has Rust glue, any other `.literal` match-site on `ConditionalBranch`)

**Interfaces:**
- No new interfaces — this task only fixes compile breakage from Task 2's type change.

- [ ] **Step 1: Discover breakage**

Run: `cargo build --workspace --exclude begin 2>&1 | grep -B2 "ConditionalBranch\|MatchLiteral\|expected.*Literal"`

List every file:line the compiler flags.

- [ ] **Step 2: Fix each site**

For each call site that pattern-matches or constructs `ConditionalBranch.literal` expecting the old `cel_parser::lex_lexer::Literal` type (e.g. `adam-lsp`'s hover/diagnostics code, if any), update it to handle `MatchLiteral::Scalar`/`::Tuple` — for a site that only cared about the scalar case before (e.g. rendering a hover tooltip), handle `Tuple` by rendering it as a parenthesized list of recursively-rendered scalars (or whatever's the simplest correct behavior for that call site — use judgment, this is genuinely a "look at what the site does and adapt" task, not a mechanical rename).

- [ ] **Step 3: Verify**

Run: `cargo build --workspace --exclude begin` and `cargo test --workspace --exclude begin`
Expected: both succeed, zero warnings.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "fix: update MatchLiteral call sites after adam-lang's ConditionalBranch change"
```

(If Step 1 finds zero breakage, skip Steps 2–4 and note in the commit log / ledger that this task was a no-op verification pass — do not create an empty commit.)

---

### Task 4: `ez-adam`: promote `adam-lang` to a real dependency; add `ExportError`; scaffold `ast_builder`

**Files:**
- Modify: `ez-adam/Cargo.toml` (move `adam-lang` from `[dev-dependencies]` to `[dependencies]`)
- Modify: `ez-adam/src/codegen/mod.rs` (add `ExportError`, `mod ast_builder;`)
- Create: `ez-adam/src/codegen/ast_builder.rs`

**Interfaces:**
- Produces: `pub enum ExportError { InvalidFormula { group: RelationshipGroupId, cell: CellId, source: cel_parser::ParseError }, InvalidCondition { conditional: ConditionalGroupId, source: cel_parser::ParseError } }`, plus a private `fn parse_expr_text(text: &str) -> Result<cel_parser::Expr, cel_parser::ParseError>` and `fn type_expr_for(ty: &CellType) -> adam_lang::ast::TypeExpr` in `ast_builder.rs`.

- [ ] **Step 1: Move `adam-lang` to a real dependency**

In `ez-adam/Cargo.toml`, remove `adam-lang` from `[dev-dependencies]` and add it to `[dependencies]`:

```toml
[dependencies]
# ...existing deps...
adam-lang = { path = "../adam-lang" }
```

- [ ] **Step 2: Write the failing test for `parse_expr_text`**

Create `ez-adam/src/codegen/ast_builder.rs`:

```rust
//! Translates a [`crate::model::document::Document`] into an
//! `adam_lang::ast::Sheet`, for [`super::generate_adm2`] to render via the
//! shared `adam_lang::format_sheet` — the same formatter `adam-fmt`/the VS
//! Code extension already use, rather than a second, independent one.

use cel_parser::{AstContext, OpLookup, Parser};

/// Parses `text` as a standalone CEL expression, for use as a formula's or
/// filter's `Expr` body when hand-building an AST node.
///
/// # Errors
///
/// Returns the underlying [`cel_parser::ParseError`] if `text` is not
/// syntactically valid CEL.
fn parse_expr_text(text: &str) -> Result<cel_parser::Expr, cel_parser::ParseError> {
    let mut lookup = OpLookup::new();
    cel_std::install(&mut lookup);
    let mut parser = Parser::<AstContext>::new(lookup);
    parser.parse_str_ast(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_expr_text_accepts_valid_cel() {
        assert!(parse_expr_text("width_pixels / height_pixels").is_ok());
    }

    #[test]
    fn parse_expr_text_rejects_invalid_cel() {
        assert!(parse_expr_text("width_pixels / ").is_err());
    }
}
```

- [ ] **Step 3: Run test**

Run: `cargo test -p ez-adam --lib codegen::ast_builder::tests`
Expected: 2 passed.

- [ ] **Step 4: Write the failing test for `type_expr_for`**

Add to the same test module:

```rust
    #[test]
    fn type_expr_for_i64_has_the_right_source_text() {
        use crate::model::cell::CellType;
        let type_expr = type_expr_for(&CellType::i64());
        match type_expr {
            adam_lang::ast::TypeExpr::Named(name, span) => {
                assert_eq!(name, "i64");
                assert_eq!(span.start.source_text().as_deref(), Some("i64"));
            }
            adam_lang::ast::TypeExpr::Tuple(..) => panic!("expected Named"),
        }
    }
```

- [ ] **Step 5: Implement `type_expr_for`**

Add to `ast_builder.rs`:

```rust
use crate::model::cell::CellType;

/// Returns `ty`'s `.adm2` type-name spelling as a hand-built `TypeExpr`,
/// with a span whose source text is genuinely that name (see
/// [`cel_parser::ExprSpan::for_text`]) so `format_sheet` renders it
/// correctly.
fn type_expr_for(ty: &CellType) -> adam_lang::ast::TypeExpr {
    let name = match ty {
        CellType::F64 { .. } => "f64",
        CellType::I64 { .. } => "i64",
        CellType::Bool => "bool",
        CellType::Text => "String",
    };
    adam_lang::ast::TypeExpr::Named(name.to_string(), cel_parser::ExprSpan::for_text(name))
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p ez-adam --lib codegen::ast_builder::tests`
Expected: 3 passed.

- [ ] **Step 7: Add `ExportError` and wire up the module**

Add to `ez-adam/src/codegen/mod.rs` (near the top, before `generate_adm2`):

```rust
use crate::model::cell::CellId;
use crate::model::conditional_group::ConditionalGroupId;
use crate::model::relationship_group::RelationshipGroupId;

mod ast_builder;

/// A reason [`generate_adm2`] could not produce `.adm2` text for a
/// [`crate::model::document::Document`].
#[derive(Debug)]
pub enum ExportError {
    /// `group`'s binding for `cell` is not valid CEL (e.g. still empty).
    InvalidFormula {
        group: RelationshipGroupId,
        cell: CellId,
        source: cel_parser::ParseError,
    },
    /// `conditional`'s `Formula`-mode condition expression is not valid CEL.
    InvalidCondition {
        conditional: ConditionalGroupId,
        source: cel_parser::ParseError,
    },
}
```

- [ ] **Step 8: Verify the crate still builds**

Run: `cargo build -p ez-adam` and `cargo test -p ez-adam --lib`
Expected: builds; existing tests still pass (nothing consumes `ExportError`/`ast_builder` yet, so no behavior change).

- [ ] **Step 9: Commit**

```bash
cargo fmt --all
git add ez-adam/Cargo.toml ez-adam/src/codegen/mod.rs ez-adam/src/codegen/ast_builder.rs
git commit -m "feat(ez-adam): scaffold AST-based codegen (ExportError, ast_builder helpers)"
```

---

### Task 5: `ez-adam`: cell declarations + clamp filter via AST

**Files:**
- Modify: `ez-adam/src/codegen/ast_builder.rs`

**Interfaces:**
- Consumes: `type_expr_for`, `parse_expr_text` (Task 4).
- Produces: `fn build_cell_decl(cell: &Cell) -> Result<adam_lang::ast::CellDecl, ExportError>` (or a narrower error type local to this function if `ExportError`'s variants don't fit a cell-decl-level failure — clamp bodies are codegen-synthesized, not user text, so a parse failure here would indicate a bug in `ast_builder` itself, not bad user input; consider `.expect()`-ing that specific parse rather than propagating `Result`, since it's not a reachable "bad user data" case the way a relationship formula is — use judgment and document the choice in the function's contract).

- [ ] **Step 1: Write the failing tests**

Add to `ast_builder.rs`'s test module:

```rust
    use crate::model::cell::{Cell, ClampRange};

    #[test]
    fn build_cell_decl_for_a_plain_cell_has_no_filter() {
        let cell = Cell::new("width_pixels", CellType::i64());
        let decl = build_cell_decl(&cell);
        assert_eq!(decl.name, "width_pixels");
        assert!(decl.filter.is_none());
    }

    #[test]
    fn build_cell_decl_for_a_clamped_i64_cell_has_a_filter_clause() {
        let cell = Cell::new(
            "width_pixels",
            CellType::I64 {
                clamp: ClampRange { min: Some(0), max: Some(100) },
            },
        );
        let decl = build_cell_decl(&cell);
        let filter = decl.filter.expect("expected a filter clause");
        assert!(matches!(filter.closure, cel_parser::Expr::Closure { .. }));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ez-adam --lib codegen::ast_builder::tests::build_cell_decl`
Expected: FAIL (function doesn't exist).

- [ ] **Step 3: Implement**

Add to `ast_builder.rs`. Reuse the exact clamp-body-text logic from the current (pre-revision) `clamp_filter_clause` in `codegen/mod.rs` — the part that decides `clamp(_, min, max)` vs `max(_, min)` vs `min(_, max)` and formats each numeric literal with the `i64` suffix / `f64` `{:?}` convention — but instead of wrapping it in a `"filter |_: {ty}| {body}"` string, parse just the call-expression text and wrap it in a hand-built `Expr::Closure`:

```rust
use crate::model::cell::{Cell, CellType};
use adam_lang::ast::{CellDecl, CellFilter};
use cel_parser::{ClosureParam, ClosureParamTypeExpr, Expr, ExprSpan};

/// Builds a `cell <name>: <type> [filter ...];` declaration for `cell`,
/// including a clamp filter clause when its type has clamp bounds set.
///
/// - Postcondition: `filter` is `None` iff `cell.ty` is `Bool`/`Text` or
///   has no clamp bounds.
fn build_cell_decl(cell: &Cell) -> CellDecl {
    CellDecl {
        name: cell.name.clone(),
        name_span: ExprSpan::for_text(&cell.name),
        type_name: Some(type_expr_for(&cell.ty)),
        initializer: None,
        filter: clamp_filter(&cell.ty),
        leading_comment: None,
        doc_comment: None,
        blank_line_before: false,
        span: ExprSpan::for_text(&cell.name),
    }
}

/// Returns a hand-built `filter |_: <type>| <clamp-call>` clause clamping
/// `ty`'s value to its clamp bounds, or `None` if `ty` is `Bool`/`Text` or
/// has no bounds set. The clamp-call text is generated the same way as
/// before this revision (explicit `i64` suffixes / `f64` `Debug`
/// formatting to avoid literal-type-inference ambiguity — see this
/// function's own body) and then parsed into an `Expr`, rather than the
/// whole `filter |_: ...| ...` clause being formatted as one string.
///
/// - Precondition: the synthesized clamp-call text is always valid CEL —
///   a parse failure here indicates a bug in this function, not bad user
///   data, so it panics rather than returning a `Result`.
fn clamp_filter(ty: &CellType) -> Option<CellFilter> {
    let (ty_name, body_text) = match ty {
        CellType::F64 { clamp } => (
            "f64",
            match (clamp.min, clamp.max) {
                (None, None) => return None,
                (Some(min), None) => format!("max(_, {min:?})"),
                (None, Some(max)) => format!("min(_, {max:?})"),
                (Some(min), Some(max)) => format!("clamp(_, {min:?}, {max:?})"),
            },
        ),
        CellType::I64 { clamp } => (
            "i64",
            match (clamp.min, clamp.max) {
                (None, None) => return None,
                (Some(min), None) => format!("max(_, {min}i64)"),
                (None, Some(max)) => format!("min(_, {max}i64)"),
                (Some(min), Some(max)) => format!("clamp(_, {min}i64, {max}i64)"),
            },
        ),
        CellType::Bool | CellType::Text => return None,
    };
    let body = parse_expr_text(&body_text)
        .unwrap_or_else(|e| panic!("synthesized clamp expression {body_text:?} failed to parse: {e:?}"));
    Some(CellFilter {
        arg_cells: vec![],
        closure: Expr::Closure {
            params: vec![ClosureParam {
                name: "_".to_string(),
                name_span: ExprSpan::for_text("_"),
                type_expr: ClosureParamTypeExpr::Named(ty_name.to_string(), ExprSpan::for_text(ty_name)),
            }],
            body: Box::new(body),
            span: ExprSpan::for_text("_"),
        },
        span: ExprSpan::for_text("_"),
    })
}
```

(Adjust `ClosureParam`/`ClosureParamTypeExpr`/`Expr::Closure`'s exact import paths — confirm whether they're re-exported from `cel_parser`'s crate root or need a deeper path like `cel_parser::ast::...`, matching whatever `adam-lang/src/ast.rs`'s own imports of these types look like.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ez-adam --lib codegen::ast_builder::tests`
Expected: all passing (5 total: 2 from Task 4, 2 new, plus the earlier `type_expr_for` test).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add ez-adam/src/codegen/ast_builder.rs
git commit -m "feat(ez-adam): build CellDecl/clamp-filter AST nodes"
```

---

### Task 6: `ez-adam`: relationship groups via AST

**Files:**
- Modify: `ez-adam/src/codegen/ast_builder.rs`

**Interfaces:**
- Consumes: `parse_expr_text` (Task 4).
- Produces: `fn build_relationship_decl(doc: &Document, group: &RelationshipGroup, group_id: RelationshipGroupId) -> Result<adam_lang::ast::RelationshipDecl, ExportError>`.

- [ ] **Step 1: Write the failing tests**

Add to `ast_builder.rs`'s test module:

```rust
    use crate::model::document::Document;
    use crate::model::geometry::Point;
    use crate::ops::cells::{add_cell, add_cell_node};
    use crate::ops::relationships::{create_relationship, set_member_formula};

    #[test]
    fn build_relationship_decl_produces_one_binding_per_member() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group_id = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group_id, a_node, "height_pixels * 2i64");
        set_member_formula(&mut doc, group_id, b_node, "width_pixels / 2i64");

        let group = &doc.relationship_groups[group_id];
        let decl = build_relationship_decl(&doc, group, group_id).unwrap();
        assert_eq!(decl.bindings.len(), 2);
        assert_eq!(decl.bindings[0].outputs[0].0, "width_pixels");
    }

    #[test]
    fn build_relationship_decl_reports_an_invalid_formula() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group_id = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        // Leave both formulas empty (the sketch's "[ ]" placeholder state).

        let group = &doc.relationship_groups[group_id];
        let result = build_relationship_decl(&doc, group, group_id);
        assert!(matches!(
            result,
            Err(ExportError::InvalidFormula { group, .. }) if group == group_id
        ));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ez-adam --lib codegen::ast_builder::tests::build_relationship_decl`
Expected: FAIL (function doesn't exist).

- [ ] **Step 3: Implement**

```rust
use crate::model::document::Document;
use crate::model::relationship_group::{RelationshipGroup, RelationshipGroupId};
use adam_lang::ast::{BindingDecl, RelationshipDecl};

/// Builds a `relationship { ... }` block for `group` (identified by
/// `group_id`, used only to label a formula error, not part of the
/// rendered output), one binding per member.
///
/// # Errors
///
/// Returns [`ExportError::InvalidFormula`] for the first member whose
/// formula text isn't valid CEL.
fn build_relationship_decl(
    doc: &Document,
    group: &RelationshipGroup,
    group_id: RelationshipGroupId,
) -> Result<RelationshipDecl, ExportError> {
    let mut bindings = Vec::with_capacity(group.members.len());
    for (node, formula) in &group.members {
        let cell_id = doc.cell_nodes[*node].cell;
        let cell = &doc.cells[cell_id];
        let body = parse_expr_text(formula).map_err(|source| ExportError::InvalidFormula {
            group: group_id,
            cell: cell_id,
            source,
        })?;
        bindings.push(BindingDecl {
            outputs: vec![(cell.name.clone(), ExprSpan::for_text(&cell.name))],
            destructure: false,
            body,
            leading_comment: None,
            blank_line_before: false,
            span: ExprSpan::for_text(&cell.name),
        });
    }
    Ok(RelationshipDecl {
        bindings,
        leading_comment: None,
        doc_comment: None,
        blank_line_before: false,
        trailing_comment: None,
        blank_line_before_close: false,
        open_brace_span: ExprSpan::for_text("relationship"),
        span: ExprSpan::for_text("relationship"),
    })
}
```

(This requires `RelationshipGroupId: PartialEq` for the test's `if group == group_id` guard — already true, since `slotmap::new_key_type!`-generated keys derive `PartialEq`.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ez-adam --lib codegen::ast_builder::tests::build_relationship_decl`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add ez-adam/src/codegen/ast_builder.rs
git commit -m "feat(ez-adam): build RelationshipDecl AST nodes"
```

---

### Task 7: `ez-adam`: conditional groups via AST

**Files:**
- Modify: `ez-adam/src/codegen/ast_builder.rs`

**Interfaces:**
- Consumes: `build_relationship_decl` (Task 6), `parse_expr_text` (Task 4), `MatchLiteral` (adam-lang, Task 2).
- Produces: `fn build_conditional_decl(doc: &Document, conditional_id: ConditionalGroupId, cond: &ConditionalGroup) -> Result<adam_lang::ast::ConditionalDecl, ExportError>`.

- [ ] **Step 1: Write the failing tests**

Add to `ast_builder.rs`'s test module:

```rust
    use crate::model::cell::CellType as CT; // avoid name clash if needed
    use crate::ops::conditionals::add_conditional_from_bool_cells;

    #[test]
    fn build_conditional_decl_for_a_single_bool_condition_has_two_branches() {
        let mut doc = Document::new("demo");
        let flag = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group_id = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group_id, a_node, "height_pixels * 2i64");
        set_member_formula(&mut doc, group_id, b_node, "width_pixels / 2i64");
        let cond_id = add_conditional_from_bool_cells(&mut doc, vec![flag], group_id, Point::new(0.0, 40.0));

        let cond = &doc.conditional_groups[cond_id];
        let decl = build_conditional_decl(&doc, cond_id, cond).unwrap();
        assert_eq!(decl.branches.len(), 2);
        assert!(decl.default.is_some());
    }

    #[test]
    fn build_conditional_decl_for_a_multi_cell_condition_uses_tuple_match_literals() {
        let mut doc = Document::new("demo");
        let flag_a = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
        let flag_b = add_cell(&mut doc, "lock_aspect", CellType::Bool);
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group_id = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group_id, a_node, "height_pixels * 2i64");
        set_member_formula(&mut doc, group_id, b_node, "width_pixels / 2i64");
        let cond_id = add_conditional_from_bool_cells(
            &mut doc,
            vec![flag_a, flag_b],
            group_id,
            Point::new(0.0, 40.0),
        );

        let cond = &doc.conditional_groups[cond_id];
        let decl = build_conditional_decl(&doc, cond_id, cond).unwrap();
        assert_eq!(decl.branches.len(), 4);
        assert!(matches!(
            decl.branches[0].literal,
            adam_lang::ast::MatchLiteral::Tuple(_)
        ));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ez-adam --lib codegen::ast_builder::tests::build_conditional_decl`
Expected: FAIL (function doesn't exist).

- [ ] **Step 3: Implement**

```rust
use crate::model::cell::CellId;
use crate::model::conditional_group::{
    CellValueLiteral, ConditionExpr, ConditionalGroup, ConditionalGroupId,
};
use adam_lang::ast::{ConditionalBranch, ConditionalDecl, DefaultBranch, MatchLiteral};

/// Builds a `conditional <expr> { <literal> => {...} ... _ => {...} }`
/// declaration for `cond`.
///
/// # Errors
///
/// Propagates [`ExportError::InvalidFormula`] from any nested relationship
/// group's members, or returns [`ExportError::InvalidCondition`] if a
/// `Formula`-mode condition expression isn't valid CEL.
fn build_conditional_decl(
    doc: &Document,
    conditional_id: ConditionalGroupId,
    cond: &ConditionalGroup,
) -> Result<ConditionalDecl, ExportError> {
    let match_expr = match &cond.condition {
        ConditionExpr::Cells(cells) => cells_tuple_expr(doc, cells),
        ConditionExpr::Formula { expr, .. } => {
            parse_expr_text(expr).map_err(|source| ExportError::InvalidCondition {
                conditional: conditional_id,
                source,
            })?
        }
    };

    let mut branches = Vec::with_capacity(cond.branches.len());
    for branch in &cond.branches {
        let mut relationships = Vec::with_capacity(branch.enabled_groups.len());
        for &group_id in &branch.enabled_groups {
            relationships.push(build_relationship_decl(doc, &doc.relationship_groups[group_id], group_id)?);
        }
        branches.push(ConditionalBranch {
            literal: match_literal_for(&branch.values),
            literal_span: ExprSpan::for_text("_"), // widened in fmt.rs's Task 2 handling for Tuple — see note below
            relationships,
            leading_comment: None,
            blank_line_before: false,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: ExprSpan::for_text("_"),
            span: ExprSpan::for_text("_"),
        });
    }

    let mut default_relationships = Vec::with_capacity(cond.default.len());
    for &group_id in &cond.default {
        default_relationships.push(build_relationship_decl(doc, &doc.relationship_groups[group_id], group_id)?);
    }

    Ok(ConditionalDecl {
        match_expr,
        branches,
        default: Some(DefaultBranch {
            relationships: default_relationships,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: ExprSpan::for_text("_"),
            span: ExprSpan::for_text("_"),
        }),
        leading_comment: None,
        doc_comment: None,
        blank_line_before: false,
        trailing_comment: None,
        blank_line_before_close: false,
        open_brace_span: ExprSpan::for_text("conditional"),
        span: ExprSpan::for_text("conditional"),
    })
}

/// Builds the `(a, b, ...)` tuple expression naming `cells`, for a
/// `Cells`-mode condition — a single cell renders as a bare identifier
/// reference instead of a one-element tuple.
fn cells_tuple_expr(doc: &Document, cells: &[CellId]) -> Expr {
    let text = if cells.len() == 1 {
        doc.cells[cells[0]].name.clone()
    } else {
        let names: Vec<&str> = cells.iter().map(|c| doc.cells[*c].name.as_str()).collect();
        format!("({})", names.join(", "))
    };
    parse_expr_text(&text)
        .unwrap_or_else(|e| panic!("synthesized condition expression {text:?} failed to parse: {e:?}"))
}

/// Converts a branch's `CellValueLiteral`s into a `MatchLiteral` — a bare
/// scalar for a single value, or a `Tuple` for multiple.
fn match_literal_for(values: &[CellValueLiteral]) -> MatchLiteral {
    if values.len() == 1 {
        MatchLiteral::Scalar(literal_for(&values[0]))
    } else {
        MatchLiteral::Tuple(values.iter().map(|v| MatchLiteral::Scalar(literal_for(v))).collect())
    }
}

/// Converts one `CellValueLiteral` into `cel_parser`'s lexer-level
/// `Literal`, by parsing its `.adm2` text spelling — reusing the same
/// literal-formatting convention (`i64` suffixes, quoted/escaped strings)
/// `ez-adam` already relies on elsewhere, rather than constructing
/// `cel_parser::lex_lexer::Literal`'s variants by hand.
fn literal_for(value: &CellValueLiteral) -> cel_parser::lex_lexer::Literal {
    let text = match value {
        CellValueLiteral::Bool(b) => b.to_string(),
        CellValueLiteral::I64(n) => format!("{n}i64"),
        CellValueLiteral::Text(s) => format!("{s:?}"),
    };
    // `Literal` isn't itself an `Expr` — extract it from a parsed literal
    // expression. Confirm during implementation whether `cel_parser::Expr`
    // has a `Literal(Literal, ExprSpan)`-shaped variant to match against
    // here (check `cel-parser/src/ast.rs`'s `Expr` enum if unsure) rather
    // than guessing the exact pattern.
    match parse_expr_text(&text).expect("synthesized literal text is always valid CEL") {
        Expr::Literal(literal, _span) => literal,
        other => panic!("expected a literal expression for {text:?}, got {other:?}"),
    }
}
```

**Note on `literal_span` for `Tuple` branches:** Task 2's `fmt.rs` change reads back the *whole* tuple's source text from `literal_span` for a `Tuple` match literal. The sketch above uses a placeholder `ExprSpan::for_text("_")` for every branch's `literal_span`, which is wrong for `Tuple` branches specifically. Fix properly: for a `Tuple`, synthesize the full parenthesized text (e.g. `"(true, false)"`) the same way `match_literal_for`'s scalar case does per-element, and call `ExprSpan::for_text` on *that whole string* — confirm `ExprSpan::for_text`'s "exactly one token" precondition (Task 1) doesn't reject a parenthesized group as multiple tokens; if it does, `for_text` needs a either a second variant that accepts a `TokenStream`'s outer span (a parenthesized group is a *single* `Group` token tree in `proc_macro2`, so this should actually satisfy the existing "exactly one token" precondition as-is — confirm this by testing rather than assuming, and adjust `for_text`'s implementation in Task 1 retroactively if it doesn't handle a `Group` token correctly).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ez-adam --lib codegen::ast_builder::tests`
Expected: all passing.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add ez-adam/src/codegen/ast_builder.rs
git commit -m "feat(ez-adam): build ConditionalDecl AST nodes, using MatchLiteral for tuples"
```

---

### Task 8: `ez-adam`: wire up `generate_adm2`; update existing unit tests

**Files:**
- Modify: `ez-adam/src/codegen/mod.rs`

**Interfaces:**
- Produces: `pub fn generate_adm2(doc: &Document) -> Result<String, ExportError>` (was `-> String`).

- [ ] **Step 1: Implement `generate_adm2` and `build_sheet`**

Replace `generate_adm2`'s body and remove the now-unused hand-formatting helpers (`generate_cell_decl`, `clamp_filter_clause`, `generate_relationship_block`, `generate_conditional_block`, `condition_expr_text`, `cell_names_text`, `branch_literal_text`, `literal_text`, `type_name`, `groups_owned_by_conditionals` — everything `ast_builder.rs` now supersedes):

```rust
use adam_lang::ast::{Sheet, SheetItem};

/// Returns `.adm2` source text for `doc`, by constructing an
/// `adam_lang::ast::Sheet` and rendering it via the shared
/// `adam_lang::format_sheet` — the same formatter `adam-fmt`/the VS Code
/// extension already use.
///
/// # Errors
///
/// Returns [`ExportError`] if any stored formula or condition-formula text
/// is not valid CEL (e.g. a relationship member whose formula box is still
/// empty).
///
/// - Complexity: O(n) in the total number of cells, relationship groups,
///   and conditional-group branches.
pub fn generate_adm2(doc: &Document) -> Result<String, ExportError> {
    Ok(adam_lang::format_sheet(&build_sheet(doc)?))
}

fn build_sheet(doc: &Document) -> Result<Sheet, ExportError> {
    let mut items = Vec::new();

    for (_, cell) in doc.cells_in_order() {
        items.push(SheetItem::Cell(ast_builder::build_cell_decl(cell)));
    }

    let owned = groups_owned_by_conditionals(doc);
    for (id, group) in doc.relationship_groups_in_order() {
        if owned.contains(&id) {
            continue;
        }
        items.push(SheetItem::Relationship(ast_builder::build_relationship_decl(doc, group, id)?));
    }

    for (id, cond) in doc.conditional_groups_in_order() {
        items.push(SheetItem::Conditional(ast_builder::build_conditional_decl(doc, id, cond)?));
    }

    Ok(Sheet {
        name: doc.sheet_name.clone(),
        name_span: cel_parser::ExprSpan::for_text(&doc.sheet_name),
        items,
        leading_comment: None,
        doc_comment: None,
        trailing_comment: None,
        blank_line_before_close: false,
        open_brace_span: cel_parser::ExprSpan::for_text("sheet"),
        span: cel_parser::ExprSpan::for_text("sheet"),
        errors: vec![],
    })
}
```

Keep `groups_owned_by_conditionals` (unchanged — still needed to skip conditional-owned groups from the top-level loop) but make its functions/imports `pub(crate)` or move it into `ast_builder.rs` alongside the other builders if that reads more cleanly; either placement is fine as long as it compiles and is used exactly once.

Note `build_relationship_decl`'s signature grew a `group_id: RelationshipGroupId` parameter per Task 6's fix note — update this call site (and Task 7's internal calls to it) to match.

- [ ] **Step 2: Update every existing unit test in `codegen/mod.rs`'s `tests` module**

Every existing test currently does `let out = generate_adm2(&doc); assert_eq!(out, "...")`. Update each to:

```rust
let out = generate_adm2(&doc).expect("valid document should export cleanly");
assert_eq!(out, "...");
```

**For the exact-string assertions themselves:** do not assume `format_sheet`'s output matches the old hand-rolled strings byte-for-byte (different whitespace/blank-line conventions are likely, especially around relationship/conditional block nesting). For each test: run it once with a placeholder assertion (e.g. `assert_eq!(out, "");`) to make it fail and print the actual output via the test harness's failure diff, copy the ACTUAL observed output into the assertion, then re-run to confirm it now passes. Do this for every pre-existing exact-string test in this file (`generates_bare_cell_declarations`, `generates_a_top_level_relationship_block`, `does_not_emit_an_out_decl_for_an_output_cell_yet`, the three clamp-filter tests, `generates_a_conditional_group_with_bool_condition`, and the two `out.contains(...)` substring tests for multi-cell-tuple/formula conditionals — convert those last two to exact-string assertions now too, since the AST-based path makes exact output more predictable/worth locking down than the old string-templating approach was). Also update the one negative test (`omits_the_filter_clause_when_no_clamp_bounds_are_set`) the same way.

Before finalizing each captured string, eyeball it against `.adm2`'s grammar to confirm it's sensible output, not just "whatever the code happened to produce" — this is a real verification step, not a rubber stamp.

- [ ] **Step 3: Add a new test for the fallible path**

```rust
#[test]
fn generate_adm2_reports_an_invalid_formula() {
    let mut doc = Document::new("demo");
    let a = add_cell(&mut doc, "width_pixels", CellType::i64());
    let b = add_cell(&mut doc, "height_pixels", CellType::i64());
    let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
    let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
    let _ = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
    // Formulas left empty.

    let result = generate_adm2(&doc);
    assert!(matches!(result, Err(ExportError::InvalidFormula { .. })));
}
```

- [ ] **Step 4: Run all codegen tests**

Run: `cargo test -p ez-adam --lib codegen`
Expected: all passing, zero warnings.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add ez-adam/src/codegen/mod.rs
git commit -m "feat(ez-adam): wire generate_adm2 onto the shared AST/formatter path"
```

---

### Task 9: `ez-adam`: update integration tests for the fallible API

**Files:**
- Modify: `ez-adam/tests/adm2_round_trip.rs`
- Modify: `ez-adam/tests/end_to_end.rs`

**Interfaces:**
- Consumes: `generate_adm2`'s new `Result<String, ExportError>` return type.

- [ ] **Step 1: Update every call site**

In both files, every `let adm2_text = generate_adm2(&doc);` becomes:

```rust
let adm2_text = generate_adm2(&doc).expect("document should export cleanly");
```

(Every document these tests build already has every relationship member given a real, valid formula — per the comment already in `adm2_round_trip.rs` about empty formulas not being valid CEL — so `.expect(...)` is appropriate here, not a `Result`-propagating test signature.)

- [ ] **Step 2: Run both integration test files**

Run: `cargo test -p ez-adam --test adm2_round_trip --test end_to_end`
Expected: all 6 passing (4 + 2), zero warnings. These tests still parse the generated text through the *real* `adam_lang::AdamParser` (the direct parser, not the AST-only one) — confirming the shared-AST-formatted output is still valid `.adm2` from the runtime's own perspective, not just internally consistent with `adam-lang`'s own formatter.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all
git add ez-adam/tests/adm2_round_trip.rs ez-adam/tests/end_to_end.rs
git commit -m "test(ez-adam): update integration tests for generate_adm2's Result return type"
```

---

### Task 10: Full workspace verification

**Files:** none (verification only).

- [ ] **Step 1: Run the full check suite**

```bash
cargo fmt --all -- --check
cargo build --workspace --exclude begin
cargo test --workspace --exclude begin
cargo test --doc --workspace --exclude begin
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

Expected: all clean, except the pre-existing, already-tracked `adam-lang` `only_used_in_recursion` clippy failure (issue #116, unrelated to this plan) — if that's the *only* failure, this task is complete; any other failure must be fixed before proceeding.

- [ ] **Step 2: Confirm no dead code remains**

Run: `cargo build -p ez-adam 2>&1 | grep -i "never used\|dead_code"`
Expected: empty — confirms the old hand-formatting helpers removed in Task 8 left no orphaned private functions behind (e.g. if `type_name` or similar was accidentally left in `codegen/mod.rs` unused after the rewrite).

- [ ] **Step 3: Commit** (only if Step 2 found and fixed something; otherwise no commit needed for this task)

## Deferred / explicitly out of scope

- Extracting shared cell↔widget-binding logic between `begin` and `ez-adam` — tracked separately, sequenced after this plan.
- Further unifying `adam-lang`'s direct parser and AST-only parser beyond the `MatchLiteral` gap closed here.
- Issues #146 (restrict codegen), #147 (output codegen), #148 (formula type-validation), #116 (pre-existing adam-lang clippy issue) — unaffected by this plan; still open.
