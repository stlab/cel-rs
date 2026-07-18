# pm-lang Structural AST Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `pm-lang` a span-carrying structural AST (`Sheet`/`CellDecl`/`RelationshipDecl`/`ConditionalDecl`/`MethodDecl`, referencing `cel_parser::Expr` for method bodies) built by a new, standalone parser, with coarse per-declaration error recovery and a comment/trivia reattachment pass — completing the `pm-lang` half of the design doc's Phase 2 (the `cel-parser` half — `Expr`/`AstContext` — already shipped in PR #42).

**Architecture:** Unlike `cel-parser` (whose deeply recursive expression grammar was genericized into `Parser<C: ParserContext>` so one grammar drives two backends), `pm-lang`'s declaration grammar is flat and non-recursive, and its existing `DynSegment`-executing path leans on `TypeRegistry` to eagerly resolve types and mutate a live `Sheet` mid-parse — logic an AST-building backend must not run at all (see design doc: `AstContext` "carries no resolved types... deferred to a later phase"). Genericizing over a shared trait would therefore thread a `&TypeRegistry` parameter through methods half the implementations ignore, for little real sharing.

Instead, this plan extracts the one piece of logic that genuinely is identical between backends — the pure token-stream cursor (peek/advance/expect-punctuation/etc., zero `Sheet`/`TypeRegistry` dependency) — into a shared `TokenCursor`, used by both today's existing `PmParser` (via a `Deref`, unchanged externally) and this plan's new `PmAstParser`. The two parsers' higher-level grammar-production functions (`parse_cell_decl`, `parse_relationship_decl`, ...) remain separate, because what each one *builds* (a live `Sheet` mutation vs. a syntax-tree node) is genuinely different work, not incidental duplication.

This intentionally leaves two grammar implementations in `pm-lang` (today's `PmParser`, and this plan's new `PmAstParser`) — the follow-on plan mentioned in the Self-Review below retires `PmParser`'s inline `Sheet`-building grammar entirely, replacing it with "parse via `PmAstParser`, then compile the AST into a `Sheet`," leaving exactly one grammar implementation. That migration is a separate, larger, higher-risk task (it touches `TypeRegistry` resolution, `Method` compilation, and every existing `pm-lang` test) and is out of scope here; this plan only adds the new, purely-additive AST path required by Phase 2.

**Tech Stack:** Rust, existing `cel-parser`/`pm-lang` crates (`cel_parser::Parser<AstContext>`/`cel_parser::Expr`, already shipped). No new dependencies.

## Global Constraints

- `cargo fmt --all` before every commit (enforced by pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` must pass (this plan
  never touches `begin`; its two `begin`-specific clippy invocations aren't relevant here but cost
  nothing to run).
- Never commit directly to `main`; this work happens on the current worktree branch.
- Doc comments follow the project's contract style (Summary / Preconditions / Postconditions /
  Complexity) — see `CLAUDE.md`.
- Every existing `cel-parser` and `pm-lang` test must keep passing **completely unchanged** —
  this plan is purely additive except for Task 1's `TokenCursor` extraction, which must be a
  behavior-preserving refactor with zero call-site changes anywhere in `pm-lang/src/parser.rs`
  outside the two spots explicitly listed in Task 1.
- `pm_lang::ast` types carry no resolved types, no operator overloads, and never fail parsing on
  semantic grounds (unknown type name, literal/type mismatch, undeclared cell, arity mismatch) —
  those checks are deferred to the (separate, future) compile-to-`Sheet` phase, mirroring
  `cel_parser::AstContext`'s "carries no resolved types... deferred to a later phase" design.

---

### Task 1: Extract `TokenCursor` from `pm-lang`'s `ParseContext`

**Files:**
- Create: `pm-lang/src/token_cursor.rs`
- Modify: `pm-lang/src/parser.rs` (replace `ParseContext`'s token-handling fields/methods with a
  `cursor: TokenCursor` field plus `Deref`/`DerefMut`; two call sites that touch `.tokens`
  directly are updated to use `TokenCursor`'s new `take_tokens`/`set_tokens`)
- Modify: `pm-lang/src/lib.rs` (add `mod token_cursor;` — the module is private; Task 3's
  `ast_parser` module reaches it via `crate::token_cursor::TokenCursor`, not a public re-export)

**Interfaces:**
- Produces (used by Task 3): `pub(crate) struct TokenCursor` with `pub(crate)` methods `new`,
  `take_tokens`, `set_tokens`, `peek_token`, `advance`, `peek_span`, `err_at`, `is_keyword`,
  `consume_ident`, `expect_punct`, `consume_punct`, `expect_open_brace`, `expect_close_brace`,
  `expect_open_bracket`, `expect_close_bracket`, `consume_literal`, `at_close_brace` — every one
  of these is today's `ParseContext` method, moved verbatim (bodies byte-for-byte identical; only
  `self.tokens` becomes the same field, now on `TokenCursor` instead of `ParseContext`).

- [ ] **Step 1: Create `pm-lang/src/token_cursor.rs`**

```rust
//! Pure token-stream cursor shared by pm-lang's `Sheet`-building parser (`parser.rs`) and its
//! AST-building parser (`ast_parser.rs`), so tokenizing/peeking/expecting logic — which has no
//! dependency on what each parser *builds* — is written exactly once.

use cel_parser::ParseError;
use cel_parser::lex_lexer::{HasSpan, LexLexer, Literal, Token};
use proc_macro2::{Delimiter, Span};

/// Parser result type, matching `cel_parser::ParseError`.
type Result<T> = std::result::Result<T, ParseError>;

/// A peekable pm-lang token stream plus the primitive lookahead/consume operations every
/// pm-lang grammar production needs, independent of what each production builds (a live
/// `property_model::Sheet` mutation, or a syntax tree node).
pub(crate) struct TokenCursor {
    tokens: Option<std::iter::Peekable<LexLexer>>,
}

impl TokenCursor {
    /// Creates a cursor over `tokens`.
    pub(crate) fn new(tokens: std::iter::Peekable<LexLexer>) -> Self {
        TokenCursor {
            tokens: Some(tokens),
        }
    }

    /// Takes the token stream, leaving `None` behind — used to hand the stream to an embedded
    /// `cel_parser::Parser` for one CEL sub-expression, then reclaim it via `set_tokens`.
    ///
    /// - Precondition: a token stream is set.
    pub(crate) fn take_tokens(&mut self) -> Option<std::iter::Peekable<LexLexer>> {
        self.tokens.take()
    }

    /// Restores a previously-taken token stream.
    pub(crate) fn set_tokens(&mut self, tokens: std::iter::Peekable<LexLexer>) {
        self.tokens = Some(tokens);
    }

    pub(crate) fn peek_token(&mut self) -> Option<&Token> {
        self.tokens.as_mut()?.peek()
    }

    pub(crate) fn advance(&mut self) -> Option<Token> {
        self.tokens.as_mut()?.next()
    }

    pub(crate) fn peek_span(&mut self) -> Span {
        self.tokens
            .as_mut()
            .and_then(|t| t.peek())
            .map(|t| t.span())
            .unwrap_or_else(Span::call_site)
    }

    pub(crate) fn err_at(&mut self, msg: impl Into<String>) -> ParseError {
        ParseError::new(msg.into(), self.peek_span())
    }

    /// Consumes and returns `true` if the next token is an identifier matching `kw`.
    pub(crate) fn is_keyword(&mut self, kw: &str) -> bool {
        let ok = matches!(
            self.tokens.as_mut().and_then(|t| t.peek()),
            Some(Token::Identifier(id)) if id == kw
        );
        if ok {
            self.advance();
        }
        ok
    }

    /// Consumes any identifier.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not an identifier.
    pub(crate) fn consume_ident(&mut self) -> Result<(String, Span)> {
        let span = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::Identifier(id)) => {
                let s = id.span();
                let _ = id;
                s
            }
            other => {
                let s = other.map(|t| t.span()).unwrap_or(Span::call_site());
                return Err(ParseError::new("expected identifier", s));
            }
        };
        if let Some(Token::Identifier(id)) = self.advance() {
            return Ok((id.to_string(), span));
        }
        unreachable!("peeked identifier, advance must return it")
    }

    /// Consumes a specific punctuation token.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token does not match `p`.
    pub(crate) fn expect_punct(&mut self, p: &str) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::Punct { op, span }) if op == p => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            Ok(span)
        } else {
            Err(ParseError::new(format!("expected `{p}`"), span))
        }
    }

    /// Consumes and returns `true` if the next token is punctuation matching `p`.
    pub(crate) fn consume_punct(&mut self, p: &str) -> bool {
        let ok = matches!(
            self.tokens.as_mut().and_then(|t| t.peek()),
            Some(Token::Punct { op, .. }) if op == p
        );
        if ok {
            self.advance();
        }
        ok
    }

    /// Consumes `{`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not `{`.
    pub(crate) fn expect_open_brace(&mut self) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::OpenDelim {
                delimiter: Delimiter::Brace,
                span,
            }) => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            Ok(span)
        } else {
            Err(ParseError::new("expected `{`", span))
        }
    }

    /// Consumes `}`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not `}`.
    pub(crate) fn expect_close_brace(&mut self) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::CloseDelim {
                delimiter: Delimiter::Brace,
                span,
            }) => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            Ok(span)
        } else {
            Err(ParseError::new("expected `}`", span))
        }
    }

    /// Consumes `[`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not `[`.
    pub(crate) fn expect_open_bracket(&mut self) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::OpenDelim {
                delimiter: Delimiter::Bracket,
                span,
            }) => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            Ok(span)
        } else {
            Err(ParseError::new("expected `[`", span))
        }
    }

    /// Consumes `]`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not `]`.
    pub(crate) fn expect_close_bracket(&mut self) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::CloseDelim {
                delimiter: Delimiter::Bracket,
                span,
            }) => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            Ok(span)
        } else {
            Err(ParseError::new("expected `]`", span))
        }
    }

    /// Consumes and returns a literal token.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not a literal.
    pub(crate) fn consume_literal(&mut self) -> Result<(Literal, Span)> {
        let span = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::Literal(lit)) => lit.span(),
            other => {
                let s = other.map(|t| t.span()).unwrap_or(Span::call_site());
                return Err(ParseError::new("expected literal", s));
            }
        };
        if let Some(Token::Literal(lit)) = self.advance() {
            return Ok((lit, span));
        }
        unreachable!("peeked literal, advance must return it")
    }

    pub(crate) fn at_close_brace(&mut self) -> bool {
        matches!(
            self.tokens.as_mut().and_then(|t| t.peek()),
            Some(Token::CloseDelim {
                delimiter: Delimiter::Brace,
                ..
            }) | None
        )
    }
}
```

- [ ] **Step 2: Update `pm-lang/src/parser.rs`'s `ParseContext`**

Find this struct definition:

```rust
struct ParseContext {
    /// Token stream; `None` while temporarily owned by CELParser.
    tokens: Option<Peekable<LexLexer>>,
    sheet: Sheet,
    /// Maps cell name → (CellId, TypeId), in declaration order, for method and
    /// conditional compilation and for exposing to callers via `ParsedSheet`.
    cell_names: IndexMap<String, (CellId, TypeId)>,
}
```

Replace it, and delete every method in the `impl ParseContext { ... }` block that follows it
(`peek_token`, `advance`, `peek_span`, `err_at`, `is_keyword`, `consume_ident`, `expect_punct`,
`consume_punct`, `expect_open_brace`, `expect_close_brace`, `expect_open_bracket`,
`expect_close_bracket`, `consume_literal`, `at_close_brace` — every method up to but not
including `parse_sheet`'s enclosing `impl PmParser` block), with:

```rust
struct ParseContext {
    cursor: crate::token_cursor::TokenCursor,
    sheet: Sheet,
    /// Maps cell name → (CellId, TypeId), in declaration order, for method and
    /// conditional compilation and for exposing to callers via `ParsedSheet`.
    cell_names: IndexMap<String, (CellId, TypeId)>,
}

impl std::ops::Deref for ParseContext {
    type Target = crate::token_cursor::TokenCursor;

    fn deref(&self) -> &crate::token_cursor::TokenCursor {
        &self.cursor
    }
}

impl std::ops::DerefMut for ParseContext {
    fn deref_mut(&mut self) -> &mut crate::token_cursor::TokenCursor {
        &mut self.cursor
    }
}
```

Every other call site in this file (`ctx.peek_token()`, `ctx.consume_ident()`, `ctx.err_at(...)`,
etc.) keeps compiling unchanged via `Deref`/`DerefMut` — exactly the pattern already used by
`DynSegmentContext` (in `cel-parser`) and `ParsedSheet` (in this same file) for transparent
access to a wrapped type.

- [ ] **Step 3: Update the two spots that touch `.tokens` directly**

In `PmParser::parse_str`, find:

```rust
        let mut ctx = ParseContext {
            tokens: Some(LexLexer::new(stream.into_iter()).peekable()),
            sheet: Sheet::new(),
            cell_names: IndexMap::new(),
        };
```

Replace with:

```rust
        let mut ctx = ParseContext {
            cursor: crate::token_cursor::TokenCursor::new(
                LexLexer::new(stream.into_iter()).peekable(),
            ),
            sheet: Sheet::new(),
            cell_names: IndexMap::new(),
        };
```

In `PmParser::parse_cel_or_expression`, find:

```rust
    fn parse_cel_or_expression(&mut self, ctx: &mut ParseContext) -> Result<DynSegment> {
        let tokens = ctx.tokens.take().expect("tokens present");
        self.cel.set_lex_tokens(tokens);
        let result = self.cel.parse_or_expression();
        ctx.tokens = Some(self.cel.take_lex_tokens().expect("tokens set"));
        result
    }
```

Replace with:

```rust
    fn parse_cel_or_expression(&mut self, ctx: &mut ParseContext) -> Result<DynSegment> {
        let tokens = ctx.cursor.take_tokens().expect("tokens present");
        self.cel.set_lex_tokens(tokens);
        let result = self.cel.parse_or_expression();
        ctx.cursor
            .set_tokens(self.cel.take_lex_tokens().expect("tokens set"));
        result
    }
```

- [ ] **Step 4: Wire the new module into `pm-lang/src/lib.rs`**

Find:

```rust
mod parser;
pub mod type_registry;
```

Replace with:

```rust
mod parser;
mod token_cursor;
pub mod type_registry;
```

- [ ] **Step 5: Run the full existing `pm-lang` test suite to confirm zero regressions**

Run: `cargo test -p pm-lang`
Expected: every test that existed before this task still passes — no test code changed at all.

- [ ] **Step 6: Format, lint, and build checks**

Run:
```bash
cargo fmt --all
cargo build --workspace
cargo clippy -p pm-lang --all-targets -- -D warnings
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
```
Expected: `cargo fmt` makes no further changes beyond what it applies; zero warnings from build
and both clippy invocations.

- [ ] **Step 7: Commit**

```bash
git add pm-lang/src/token_cursor.rs pm-lang/src/parser.rs pm-lang/src/lib.rs
git commit -m "$(cat <<'EOF'
refactor(pm-lang): extract TokenCursor from ParseContext

Pulls the pure token-stream lookahead/consume operations (no Sheet or
TypeRegistry dependency) out of ParseContext into a standalone
TokenCursor, so the upcoming AST-building parser can reuse them
without duplicating tokenizing logic. ParseContext keeps every method
name via Deref/DerefMut; no call site elsewhere in this file changes.
Zero behavior change.
EOF
)"
```

---

### Task 2: pm-lang AST types

**Files:**
- Create: `pm-lang/src/ast.rs`
- Modify: `pm-lang/src/lib.rs` (add `pub mod ast;`)

**Interfaces:**
- Consumes: `cel_parser::{Expr, ExprSpan}` (already shipped), `cel_parser::lex_lexer::Literal`
  (the raw, unresolved literal token type — same type `pm-lang/src/parser.rs` already imports).
- Produces (used by Task 3): `pub struct Sheet { pub name: String, pub name_span: ExprSpan, pub
  items: Vec<SheetItem>, pub span: ExprSpan, pub errors: Vec<cel_parser::ParseError> }`;
  `pub enum SheetItem { Cell(CellDecl), Relationship(RelationshipDecl),
  Conditional(ConditionalDecl), Error { span: ExprSpan } }` with methods
  `pub fn span(&self) -> ExprSpan` and `pub(crate) fn set_leading_comment(&mut self, comment:
  String)`; `pub struct CellDecl { pub name: String, pub name_span: ExprSpan, pub type_name:
  Option<(String, ExprSpan)>, pub initializer: Option<(cel_parser::lex_lexer::Literal, ExprSpan)>,
  pub leading_comment: Option<String>, pub span: ExprSpan }`; `pub struct RelationshipDecl { pub
  name: Option<(String, ExprSpan)>, pub methods: Vec<MethodDecl>, pub leading_comment:
  Option<String>, pub span: ExprSpan }`; `pub struct ConditionalDecl { pub match_name: String, pub
  match_name_span: ExprSpan, pub branches: Vec<ConditionalBranch>, pub default:
  Option<Vec<MethodDecl>>, pub leading_comment: Option<String>, pub span: ExprSpan }`; `pub struct
  ConditionalBranch { pub literal: cel_parser::lex_lexer::Literal, pub literal_span: ExprSpan, pub
  methods: Vec<MethodDecl>, pub span: ExprSpan }`; `pub struct MethodDecl { pub inputs:
  Vec<(String, ExprSpan)>, pub outputs: Vec<(String, ExprSpan)>, pub body: cel_parser::Expr, pub
  span: ExprSpan }`.

- [ ] **Step 1: Write the test module for the AST types**

Create `pm-lang/src/ast.rs` with only this content (the types it references don't exist yet —
this is the intended failing state):

```rust
//! A span-carrying pm-lang structural AST, built by [`crate::PmAstParser`] as an alternative to
//! [`crate::PmParser`]'s direct `property_model::Sheet` construction. Method bodies and cell
//! initializers reference [`cel_parser::Expr`]/[`cel_parser::lex_lexer::Literal`] directly.
//! Carries no resolved types, no `TypeRegistry` lookups, and never fails on semantic grounds
//! (unknown type name, literal/type mismatch, undeclared cell, arity mismatch) — those checks
//! are deferred to a later, separate compile-to-`Sheet` phase, mirroring
//! [`cel_parser::AstContext`]'s design.

use cel_parser::ExprSpan;
use cel_parser::lex_lexer::Literal;

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::Span;

    fn point(span: Span) -> ExprSpan {
        ExprSpan {
            start: span,
            end: span,
        }
    }

    #[test]
    fn sheet_item_span_reads_the_cell_variant() {
        let span = point(Span::call_site());
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
        let item = SheetItem::Error { span };
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
    fn set_leading_comment_on_error_variant_is_a_no_op() {
        let span = point(Span::call_site());
        let mut item = SheetItem::Error { span };
        item.set_leading_comment("hi".to_string()); // must not panic
        assert!(matches!(item, SheetItem::Error { .. }));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p pm-lang ast::`
Expected: compile errors — `cannot find struct \`CellDecl\`` (and similarly for every other type
referenced), since none of them exist yet.

- [ ] **Step 3: Implement the AST types**

Add this content **above** the `#[cfg(test)] mod tests { ... }` block already in
`pm-lang/src/ast.rs` (the module doc comment and `use` lines from Step 1 stay where they are):

```rust
/// A parsed pm-lang sheet declaration, with source spans on every node.
///
/// Built by [`crate::PmAstParser`]; consumed by the language server, the formatter, and the
/// (separate, future) compile-to-`Sheet` phase.
#[derive(Debug, Clone)]
pub struct Sheet {
    /// The sheet's declared name.
    pub name: String,
    /// The name token's span.
    pub name_span: ExprSpan,
    /// The sheet's items, in declaration order.
    pub items: Vec<SheetItem>,
    /// The span of the whole `sheet ... { ... }` construct.
    pub span: ExprSpan,
    /// Syntax errors recovered while parsing, in source order. Empty for a syntactically clean
    /// sheet.
    pub errors: Vec<cel_parser::ParseError>,
}

/// One top-level item inside a `sheet { ... }` body.
#[derive(Debug, Clone)]
pub enum SheetItem {
    /// A `cell` declaration.
    Cell(CellDecl),
    /// A `relationship` declaration.
    Relationship(RelationshipDecl),
    /// A `conditional` declaration.
    Conditional(ConditionalDecl),
    /// A syntax error recovered at declaration granularity; `span` covers the skipped tokens.
    Error {
        /// The span of the skipped, malformed item.
        span: ExprSpan,
    },
}

impl SheetItem {
    /// Returns this item's source span.
    pub fn span(&self) -> ExprSpan {
        match self {
            SheetItem::Cell(c) => c.span,
            SheetItem::Relationship(r) => r.span,
            SheetItem::Conditional(c) => c.span,
            SheetItem::Error { span } => *span,
        }
    }

    /// Sets this item's leading comment, if the variant carries one. A no-op for the `Error`
    /// variant, which has no comment field.
    pub(crate) fn set_leading_comment(&mut self, comment: String) {
        match self {
            SheetItem::Cell(c) => c.leading_comment = Some(comment),
            SheetItem::Relationship(r) => r.leading_comment = Some(comment),
            SheetItem::Conditional(c) => c.leading_comment = Some(comment),
            SheetItem::Error { .. } => {}
        }
    }
}

/// `cell_decl = "cell" identifier cell_type_init ";".`
///
/// `type_name`/`initializer` are unresolved — no `TypeRegistry` lookup, no literal validation.
/// Exactly one of `type_name`, `initializer` may be absent, per the grammar's three
/// `cell_type_init` forms, but this is not enforced here (an all-`None` `CellDecl` cannot be
/// produced by [`crate::PmAstParser`], which requires at least one of `:`/`=`).
#[derive(Debug, Clone)]
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

/// `relationship_decl = "relationship" [ identifier ] "{" { method_decl } "}".`
#[derive(Debug, Clone)]
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

/// `conditional_decl = "conditional" identifier "{" { conditional_branch } [ default_branch ] "}".`
#[derive(Debug, Clone)]
pub struct ConditionalDecl {
    /// The name of the cell this conditional matches on.
    pub match_name: String,
    /// The match cell name token's span.
    pub match_name_span: ExprSpan,
    /// The named (literal `=>`) branches, in declaration order.
    pub branches: Vec<ConditionalBranch>,
    /// The `_ => { ... }` default branch's methods, if present.
    pub default: Option<Vec<MethodDecl>>,
    /// A leading comment immediately preceding this declaration, if recovered.
    pub leading_comment: Option<String>,
    /// The span of the whole `conditional ... { ... }` declaration.
    pub span: ExprSpan,
}

/// `conditional_branch = literal "=>" "{" { method_decl } "}" [ "," ].`
#[derive(Debug, Clone)]
pub struct ConditionalBranch {
    /// The branch's unresolved match literal.
    pub literal: Literal,
    /// The literal token's span.
    pub literal_span: ExprSpan,
    /// The branch's methods, in declaration order.
    pub methods: Vec<MethodDecl>,
    /// The span from the branch's literal through its closing `}`.
    pub span: ExprSpan,
}

/// `method_decl = "method" cell_list "->" cell_list method_body.`
///
/// `method_body = "{" or_expression "}"`, parsed via `cel_parser::Parser<AstContext>` into
/// `body`. Cell names referenced in `body` (e.g. `width` in `{ width * height }`) are recorded
/// as plain `Expr::Ident` nodes — resolving them against `inputs` is deferred to the compile
/// phase, exactly as `cel_parser::AstContext` defers all identifier resolution.
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

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pm-lang ast::`
Expected: 6 tests pass (`sheet_item_span_reads_the_cell_variant`,
`sheet_item_span_reads_the_relationship_variant`, `sheet_item_span_reads_the_conditional_variant`,
`sheet_item_span_reads_the_error_variant`, `set_leading_comment_sets_the_cell_variant`,
`set_leading_comment_on_error_variant_is_a_no_op`).

- [ ] **Step 5: Wire the new module into `pm-lang/src/lib.rs`**

Find:

```rust
mod parser;
mod token_cursor;
pub mod type_registry;
```

Replace with:

```rust
pub mod ast;
mod parser;
mod token_cursor;
pub mod type_registry;
```

- [ ] **Step 6: Run the full existing `pm-lang` test suite to confirm zero regressions**

Run: `cargo test -p pm-lang`
Expected: every test that existed before this task still passes, plus the 6 new `ast` tests.

- [ ] **Step 7: Format and lint**

Run:
```bash
cargo fmt --all
cargo clippy -p pm-lang --all-targets -- -D warnings
```
Expected: `cargo fmt` makes no further changes beyond what it applies; `clippy` reports zero
warnings.

- [ ] **Step 8: Commit**

```bash
git add pm-lang/src/ast.rs pm-lang/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(pm-lang): add structural AST types

Sheet/SheetItem/CellDecl/RelationshipDecl/ConditionalDecl/
ConditionalBranch/MethodDecl, referencing cel_parser::Expr for method
bodies. Carries no resolved types or TypeRegistry lookups; semantic
validation is deferred to a later, separate compile-to-Sheet phase.
EOF
)"
```

---

### Task 3: `PmAstParser` — the AST-building parser

**Files:**
- Create: `pm-lang/src/ast_parser.rs`
- Modify: `pm-lang/src/lib.rs` (add `mod ast_parser;` and `pub use ast_parser::PmAstParser;`)

**Interfaces:**
- Consumes: `crate::token_cursor::TokenCursor` (Task 1), `crate::ast::*` (Task 2),
  `cel_parser::{Parser, AstContext, OpLookup}` (already shipped).
- Produces (used by Task 4, Task 5): `pub struct PmAstParser { .. }` with `pub fn new() -> Self`
  and `pub fn parse_str(&mut self, source: &str) -> cel_parser::Result<crate::ast::Sheet>`. No
  recovery yet in this task — any syntax error still aborts the whole parse (`Sheet.errors` is
  always empty); Task 4 adds recovery without changing this signature.

- [ ] **Step 1: Write the failing end-to-end tests**

Create `pm-lang/src/ast_parser.rs` with only this content (the parser doesn't exist yet — this is
the intended failing state):

```rust
//! [`PmAstParser`]: parses pm-lang source into [`crate::ast::Sheet`] instead of executing into a
//! live `property_model::Sheet`. Shares [`crate::token_cursor::TokenCursor`] with
//! [`crate::PmParser`] for pure tokenizing; the two parsers' grammar-production functions are
//! separate because what each one builds is genuinely different (see this plan's Architecture
//! section).

use cel_parser::{AstContext, OpLookup, Parser as CelParser};

use crate::ast;
use crate::token_cursor::TokenCursor;

/// Parser result type, matching `cel_parser::ParseError`.
pub type Result<T> = std::result::Result<T, cel_parser::ParseError>;

#[cfg(test)]
mod tests {
    use super::*;
    use cel_parser::Expr;

    #[test]
    fn parse_empty_sheet_has_no_items() {
        let sheet = PmAstParser::new().parse_str("sheet empty {}").unwrap();
        assert_eq!(sheet.name, "empty");
        assert!(sheet.items.is_empty());
        assert!(sheet.errors.is_empty());
    }

    #[test]
    fn parse_cell_with_annotation_and_initializer() {
        let sheet = PmAstParser::new()
            .parse_str("sheet s { cell width: f64 = 1920.0; }")
            .unwrap();
        assert_eq!(sheet.items.len(), 1);
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert_eq!(cell.name, "width");
        assert_eq!(cell.type_name.as_ref().map(|(n, _)| n.as_str()), Some("f64"));
        assert!(cell.initializer.is_some());
    }

    #[test]
    fn parse_cell_annotation_only_has_no_initializer() {
        let sheet = PmAstParser::new()
            .parse_str("sheet s { cell area: f64; }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert!(cell.type_name.is_some());
        assert!(cell.initializer.is_none());
    }

    #[test]
    fn parse_cell_initializer_only_has_no_type_name() {
        let sheet = PmAstParser::new()
            .parse_str("sheet s { cell mode = 0i32; }")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert!(cell.type_name.is_none());
        assert!(cell.initializer.is_some());
    }

    #[test]
    fn parse_relationship_records_methods_in_order() {
        let sheet = PmAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    relationship {
                        method [width, height] -> [area]   { width * height }
                        method [area, height]  -> [width]  { area / height }
                    }
                }
            "#,
            )
            .unwrap();
        let ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert_eq!(rel.methods.len(), 2);
        assert_eq!(rel.methods[0].inputs[0].0, "width");
        assert_eq!(rel.methods[0].outputs[0].0, "area");
        assert!(matches!(rel.methods[0].body, Expr::Op { ref name, .. } if name == "*"));
    }

    #[test]
    fn parse_relationship_optional_name() {
        let sheet = PmAstParser::new()
            .parse_str("sheet s { relationship r { method [x] -> [y] { x } } }")
            .unwrap();
        let ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert_eq!(rel.name.as_ref().map(|(n, _)| n.as_str()), Some("r"));
    }

    #[test]
    fn parse_conditional_records_branches_and_default() {
        let sheet = PmAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    conditional mode {
                        0i32 => { method [width] -> [height] { width } },
                        _ => { method [width] -> [height] { width } },
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

    #[test]
    fn parse_method_body_is_a_cel_expr_tree() {
        let sheet = PmAstParser::new()
            .parse_str("sheet s { relationship { method [a, b] -> [c] { (a + b, a - b) } } }")
            .unwrap();
        let ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert!(matches!(rel.methods[0].body, Expr::Tuple { .. }));
    }

    #[test]
    fn parse_unknown_sheet_item_is_an_error() {
        let result = PmAstParser::new().parse_str("sheet s { bogus x; }");
        assert!(result.is_err());
    }

    #[test]
    fn parse_malformed_cell_is_an_error() {
        let result = PmAstParser::new().parse_str("sheet s { cell x unknown_syntax }");
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test -p pm-lang ast_parser::`
Expected: compile errors — `cannot find struct \`PmAstParser\`` — since it doesn't exist yet.

- [ ] **Step 3: Implement `PmAstParser`**

Add this content **above** the `#[cfg(test)] mod tests { ... }` block already in
`pm-lang/src/ast_parser.rs` (the module doc comment, `use` lines, and `Result` alias from Step 1
stay where they are):

```rust
/// Parses pm-lang source strings into [`ast::Sheet`] trees, instead of executing into a live
/// `property_model::Sheet` (see [`crate::PmParser`] for that path).
///
/// # Example
///
/// ```rust
/// use pm_lang::PmAstParser;
///
/// let sheet = PmAstParser::new().parse_str("sheet s { cell x: i32 = 0; }").unwrap();
/// assert_eq!(sheet.name, "s");
/// ```
pub struct PmAstParser {
    cel: CelParser<AstContext>,
}

impl Default for PmAstParser {
    fn default() -> Self {
        Self::new()
    }
}

impl PmAstParser {
    /// Creates a new AST-building parser.
    ///
    /// Unlike [`crate::PmParser::new`], this takes no `TypeRegistry`/`OpLookup` — `AstContext`
    /// resolves no identifiers and validates nothing during parsing (see this plan's Architecture
    /// section), so there is nothing for either to configure.
    #[must_use]
    pub fn new() -> Self {
        PmAstParser {
            cel: CelParser::new(OpLookup::new()),
        }
    }

    /// Parses a pm-lang source string into an [`ast::Sheet`].
    ///
    /// # Errors
    ///
    /// Returns `Err` on any syntax error. (Coarse recovery — continuing past a malformed
    /// declaration instead of aborting — lands in a later task of this same plan.)
    pub fn parse_str(&mut self, source: &str) -> Result<ast::Sheet> {
        use std::str::FromStr;
        let stream = proc_macro2::TokenStream::from_str(source)
            .map_err(|e| cel_parser::ParseError::new(e.to_string(), e.span()))?;
        let mut cursor = TokenCursor::new(cel_parser::lex_lexer::LexLexer::new(stream.into_iter()).peekable());
        let sheet = self.parse_sheet(&mut cursor)?;
        if let Some(tok) = cursor.peek_token() {
            use cel_parser::lex_lexer::HasSpan;
            return Err(cel_parser::ParseError::new("unexpected token", tok.span()));
        }
        Ok(sheet)
    }

    /// `sheet = "sheet" identifier "{" { sheet_item } "}".`
    fn parse_sheet(&mut self, cursor: &mut TokenCursor) -> Result<ast::Sheet> {
        let sheet_start = cursor.peek_span();
        if !cursor.is_keyword("sheet") {
            return Err(cursor.err_at("expected `sheet`"));
        }
        let (name, name_span) = cursor.consume_ident()?;
        cursor.expect_open_brace()?;
        let mut items = Vec::new();
        while !cursor.at_close_brace() {
            items.push(self.parse_sheet_item(cursor)?);
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::Sheet {
            name,
            name_span: point(name_span),
            items,
            span: ast::ExprSpan {
                start: sheet_start,
                end: close_span,
            },
            errors: Vec::new(),
        })
    }

    /// `sheet_item = cell_decl | relationship_decl | conditional_decl.`
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
            Some(tok) => Err(cel_parser::ParseError::new(
                "expected `cell`, `relationship`, or `conditional`",
                tok.span(),
            )),
            None => Err(cel_parser::ParseError::new(
                "unexpected end of input",
                proc_macro2::Span::call_site(),
            )),
        }
    }

    /// `cell_decl = "cell" identifier cell_type_init ";".`
    fn parse_cell_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::CellDecl> {
        let decl_start = cursor.peek_span();
        cursor.is_keyword("cell");
        let (name, name_span) = cursor.consume_ident()?;
        let (type_name, initializer) = if cursor.consume_punct(":") {
            let (type_name, type_span) = cursor.consume_ident()?;
            let initializer = if cursor.consume_punct("=") {
                let (lit, lit_span) = cursor.consume_literal()?;
                Some((lit, point(lit_span)))
            } else {
                None
            };
            (Some((type_name, point(type_span))), initializer)
        } else if cursor.consume_punct("=") {
            let (lit, lit_span) = cursor.consume_literal()?;
            (None, Some((lit, point(lit_span))))
        } else {
            return Err(cursor.err_at("expected `:` or `=` in cell declaration"));
        };
        let semi_span = cursor.expect_punct(";")?;
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
    }

    /// `relationship_decl = "relationship" [ identifier ] "{" { method_decl } "}".`
    fn parse_relationship_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::RelationshipDecl> {
        use cel_parser::lex_lexer::Token;
        let decl_start = cursor.peek_span();
        cursor.is_keyword("relationship");
        let name = if matches!(cursor.peek_token(), Some(Token::Identifier(_))) {
            let (n, s) = cursor.consume_ident()?;
            Some((n, point(s)))
        } else {
            None
        };
        cursor.expect_open_brace()?;
        let mut methods = Vec::new();
        while !cursor.at_close_brace() {
            methods.push(self.parse_method_decl(cursor)?);
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::RelationshipDecl {
            name,
            methods,
            leading_comment: None,
            span: ast::ExprSpan {
                start: decl_start,
                end: close_span,
            },
        })
    }

    /// `conditional_decl = "conditional" identifier "{" { conditional_branch } [ default_branch ] "}".`
    fn parse_conditional_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::ConditionalDecl> {
        use cel_parser::lex_lexer::Token;
        let decl_start = cursor.peek_span();
        cursor.is_keyword("conditional");
        let (match_name, match_span) = cursor.consume_ident()?;
        cursor.expect_open_brace()?;
        let mut branches = Vec::new();
        let mut default = None;
        while !cursor.at_close_brace() {
            if matches!(cursor.peek_token(), Some(Token::Identifier(id)) if id == "_") {
                cursor.advance();
                cursor.expect_punct("=>")?;
                cursor.expect_open_brace()?;
                let mut methods = Vec::new();
                while !cursor.at_close_brace() {
                    methods.push(self.parse_method_decl(cursor)?);
                }
                cursor.expect_close_brace()?;
                cursor.consume_punct(",");
                default = Some(methods);
                break; // default branch is always last
            }
            let (lit, lit_span) = cursor.consume_literal()?;
            cursor.expect_punct("=>")?;
            cursor.expect_open_brace()?;
            let mut methods = Vec::new();
            while !cursor.at_close_brace() {
                methods.push(self.parse_method_decl(cursor)?);
            }
            let close = cursor.expect_close_brace()?;
            cursor.consume_punct(",");
            branches.push(ast::ConditionalBranch {
                literal: lit,
                literal_span: point(lit_span),
                methods,
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
    }

    /// `method_decl = "method" cell_list "->" cell_list method_body.`
    fn parse_method_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::MethodDecl> {
        let decl_start = cursor.peek_span();
        if !cursor.is_keyword("method") {
            return Err(cursor.err_at("expected `method`"));
        }
        let inputs = parse_cell_list(cursor)?;
        cursor.expect_punct("->")?;
        let outputs = parse_cell_list(cursor)?;
        cursor.expect_open_brace()?;
        let body = self.parse_cel_or_expression(cursor)?;
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::MethodDecl {
            inputs,
            outputs,
            body,
            span: ast::ExprSpan {
                start: decl_start,
                end: close_span,
            },
        })
    }

    /// Delegates one `or_expression` to `cel_parser::Parser<AstContext>`, sharing the token
    /// stream (the same take/set-tokens handoff `crate::PmParser` uses for the `DynSegment`
    /// path).
    fn parse_cel_or_expression(&mut self, cursor: &mut TokenCursor) -> Result<cel_parser::Expr> {
        let tokens = cursor.take_tokens().expect("tokens present");
        self.cel.set_lex_tokens(tokens);
        let result = self.cel.parse_or_expression_ast();
        cursor.set_tokens(self.cel.take_lex_tokens().expect("tokens set"));
        result
    }
}

/// `cell_list = "[" identifier { "," identifier } "]".`
fn parse_cell_list(cursor: &mut TokenCursor) -> Result<Vec<(String, ast::ExprSpan)>> {
    cursor.expect_open_bracket()?;
    let mut cells = Vec::new();
    loop {
        let (name, span) = cursor.consume_ident()?;
        cells.push((name, point(span)));
        if !cursor.consume_punct(",") {
            break;
        }
    }
    cursor.expect_close_bracket()?;
    Ok(cells)
}

/// A single-token `ExprSpan` where start and end coincide.
fn point(span: proc_macro2::Span) -> ast::ExprSpan {
    ast::ExprSpan {
        start: span,
        end: span,
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pm-lang ast_parser::`
Expected: all 9 tests pass (`parse_empty_sheet_has_no_items`,
`parse_cell_with_annotation_and_initializer`, `parse_cell_annotation_only_has_no_initializer`,
`parse_cell_initializer_only_has_no_type_name`, `parse_relationship_records_methods_in_order`,
`parse_relationship_optional_name`, `parse_conditional_records_branches_and_default`,
`parse_method_body_is_a_cel_expr_tree`, `parse_unknown_sheet_item_is_an_error`,
`parse_malformed_cell_is_an_error`).

- [ ] **Step 5: Wire the new module into `pm-lang/src/lib.rs`**

Find:

```rust
pub mod ast;
mod parser;
mod token_cursor;
pub mod type_registry;
```

Replace with:

```rust
pub mod ast;
mod ast_parser;
mod parser;
mod token_cursor;
pub mod type_registry;
```

Then find:

```rust
pub use cel_parser::ParseError;
pub use parser::{ParsedSheet, PmParser};
pub use type_registry::TypeRegistry;
```

Replace with:

```rust
pub use ast_parser::PmAstParser;
pub use cel_parser::ParseError;
pub use parser::{ParsedSheet, PmParser};
pub use type_registry::TypeRegistry;
```

- [ ] **Step 6: Run the full existing `pm-lang` test suite to confirm zero regressions**

Run: `cargo test -p pm-lang`
Expected: every test that existed before this task still passes, plus this task's 9 new tests.

- [ ] **Step 7: Run doc tests**

Run: `cargo test --doc -p pm-lang`
Expected: the new doc example on `PmAstParser` passes.

- [ ] **Step 8: Format, lint, and build checks**

Run:
```bash
cargo fmt --all
cargo build --workspace
cargo clippy -p pm-lang --all-targets -- -D warnings
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
```
Expected: zero warnings from all three.

- [ ] **Step 9: Commit**

```bash
git add pm-lang/src/ast_parser.rs pm-lang/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(pm-lang): add PmAstParser, the AST-building parser

Parses pm-lang source into ast::Sheet instead of executing into a
live property_model::Sheet. Method bodies delegate to
cel_parser::Parser<AstContext>, sharing the token stream the same way
PmParser's DynSegment path already does. No error recovery yet (a
syntax error still aborts the whole parse) — that lands in the next
task.
EOF
)"
```

---

### Task 4: Coarse error recovery

**Files:**
- Modify: `pm-lang/src/token_cursor.rs` (add `skip_to_recovery_point`)
- Modify: `pm-lang/src/ast_parser.rs` (`parse_sheet`'s loop catches per-item errors instead of
  propagating the first one)

**Interfaces:**
- Produces (used by this task only): `TokenCursor::skip_to_recovery_point(&mut self) ->
  proc_macro2::Span`.
- Recovery granularity: **declaration** (`sheet_item`) level only — a syntax error anywhere
  inside a `cell`/`relationship`/`conditional` item (including inside one of its nested
  `method_decl`s, since `method_decl` has no `;`/statement separator of its own) causes that
  *whole* item to become a single `SheetItem::Error`, and parsing resumes at the next sibling
  item. This matches the design doc's "statement-level granularity is enough" scope; it does not
  promise per-`method_decl` recovery inside a single `relationship`/`conditional` block.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests { ... }` block in `pm-lang/src/ast_parser.rs` (append; do not
remove any existing tests):

```rust
    #[test]
    fn recovery_records_an_error_item_and_continues_parsing() {
        let sheet = PmAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    cell good_before: i32 = 1;
                    cell bad unknown_syntax
                    cell good_after: i32 = 2;
                }
            "#,
            )
            .unwrap();
        assert_eq!(sheet.items.len(), 3);
        assert!(matches!(sheet.items[0], ast::SheetItem::Cell(_)));
        assert!(matches!(sheet.items[1], ast::SheetItem::Error { .. }));
        assert!(matches!(sheet.items[2], ast::SheetItem::Cell(_)));
        assert_eq!(sheet.errors.len(), 1);
    }

    #[test]
    fn recovery_collects_multiple_errors_from_multiple_malformed_items() {
        let sheet = PmAstParser::new()
            .parse_str(
                r#"
                sheet s {
                    cell bad1 unknown_syntax;
                    cell bad2 unknown_syntax;
                    cell good: i32 = 1;
                }
            "#,
            )
            .unwrap();
        assert_eq!(sheet.errors.len(), 2);
        assert!(matches!(sheet.items.last(), Some(ast::SheetItem::Cell(_))));
    }

    #[test]
    fn well_formed_input_has_empty_errors() {
        let sheet = PmAstParser::new()
            .parse_str("sheet s { cell x: i32 = 1; }")
            .unwrap();
        assert!(sheet.errors.is_empty());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pm-lang ast_parser::`
Expected: `recovery_records_an_error_item_and_continues_parsing` and
`recovery_collects_multiple_errors_from_multiple_malformed_items` FAIL (today, the first
malformed `cell bad unknown_syntax` aborts the whole `parse_str` with `Err`, so `.unwrap()`
panics). `well_formed_input_has_empty_errors` already passes (a no-op assertion against Task 3's
behavior) — that's expected; it exists to guard this task's change doesn't introduce false
positives.

- [ ] **Step 3: Add `skip_to_recovery_point` to `TokenCursor`**

In `pm-lang/src/token_cursor.rs`, add this method inside `impl TokenCursor { ... }` (anywhere
after `at_close_brace`):

```rust
    /// Skips tokens until a declaration-boundary recovery point: a `;` at the current nesting
    /// depth (consumed), or a `}` that closes back to the current nesting depth (not consumed,
    /// so the caller's `at_close_brace` check still sees it). Used only by
    /// [`crate::PmAstParser`]'s coarse error recovery.
    ///
    /// - Postcondition: returns the span of the last token inspected, so an `Error` placeholder
    ///   node can cover the skipped range.
    ///
    /// - Complexity: O(n) in the number of tokens skipped.
    pub(crate) fn skip_to_recovery_point(&mut self) -> Span {
        let mut last = self.peek_span();
        let mut depth: i32 = 0;
        loop {
            match self.peek_token() {
                None => return last,
                Some(Token::CloseDelim { .. }) if depth == 0 => return last,
                Some(Token::CloseDelim { .. }) => {
                    depth -= 1;
                    last = self.peek_span();
                    self.advance();
                }
                Some(Token::OpenDelim { .. }) => {
                    depth += 1;
                    last = self.peek_span();
                    self.advance();
                }
                Some(Token::Punct { op, .. }) if op == ";" && depth == 0 => {
                    last = self.peek_span();
                    self.advance();
                    return last;
                }
                _ => {
                    last = self.peek_span();
                    self.advance();
                }
            }
        }
    }
```

- [ ] **Step 4: Change `parse_sheet`'s loop to recover**

In `pm-lang/src/ast_parser.rs`, find:

```rust
        let (name, name_span) = cursor.consume_ident()?;
        cursor.expect_open_brace()?;
        let mut items = Vec::new();
        while !cursor.at_close_brace() {
            items.push(self.parse_sheet_item(cursor)?);
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::Sheet {
            name,
            name_span: point(name_span),
            items,
            span: ast::ExprSpan {
                start: sheet_start,
                end: close_span,
            },
            errors: Vec::new(),
        })
```

Replace with:

```rust
        let (name, name_span) = cursor.consume_ident()?;
        cursor.expect_open_brace()?;
        let mut items = Vec::new();
        let mut errors = Vec::new();
        while !cursor.at_close_brace() {
            let item_start = cursor.peek_span();
            match self.parse_sheet_item(cursor) {
                Ok(item) => items.push(item),
                Err(e) => {
                    errors.push(e);
                    let item_end = cursor.skip_to_recovery_point();
                    items.push(ast::SheetItem::Error {
                        span: ast::ExprSpan {
                            start: item_start,
                            end: item_end,
                        },
                    });
                }
            }
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::Sheet {
            name,
            name_span: point(name_span),
            items,
            span: ast::ExprSpan {
                start: sheet_start,
                end: close_span,
            },
            errors,
        })
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p pm-lang ast_parser::`
Expected: all tests pass, including the 3 added in Step 1.

- [ ] **Step 6: Run the full existing `pm-lang` test suite to confirm zero regressions**

Run: `cargo test -p pm-lang`
Expected: every test that existed before this task still passes (in particular,
`parse_unknown_sheet_item_is_an_error` and `parse_malformed_cell_is_an_error` from Task 3 must
still pass — the top-level `parse_str` still returns `Err` overall whenever `sheet.errors` is
non-empty is NOT the behavior; re-read: those two tests call `PmAstParser::new().parse_str(...)`
and assert `result.is_err()`. After this task, a single malformed cell no longer aborts the whole
parse — `parse_str` now returns `Ok(Sheet { errors: [..], .. })` instead. This is an intentional
behavior change from Task 3, so Task 3's two error-path tests must be updated in this same step,
not left failing.)

Update the two tests in the same `mod tests` block:

Find:

```rust
    #[test]
    fn parse_unknown_sheet_item_is_an_error() {
        let result = PmAstParser::new().parse_str("sheet s { bogus x; }");
        assert!(result.is_err());
    }

    #[test]
    fn parse_malformed_cell_is_an_error() {
        let result = PmAstParser::new().parse_str("sheet s { cell x unknown_syntax }");
        assert!(result.is_err());
    }
```

Replace with:

```rust
    #[test]
    fn parse_unknown_sheet_item_is_recorded_as_an_error_item() {
        let sheet = PmAstParser::new()
            .parse_str("sheet s { bogus x; }")
            .unwrap();
        assert_eq!(sheet.errors.len(), 1);
        assert!(matches!(sheet.items[0], ast::SheetItem::Error { .. }));
    }

    #[test]
    fn parse_malformed_cell_is_recorded_as_an_error_item() {
        let sheet = PmAstParser::new()
            .parse_str("sheet s { cell x unknown_syntax }")
            .unwrap();
        assert_eq!(sheet.errors.len(), 1);
        assert!(matches!(sheet.items[0], ast::SheetItem::Error { .. }));
    }
```

Re-run: `cargo test -p pm-lang`
Expected: every test passes.

- [ ] **Step 7: Format, lint, and build checks**

Run:
```bash
cargo fmt --all
cargo build --workspace
cargo clippy -p pm-lang --all-targets -- -D warnings
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
```
Expected: zero warnings from all three.

- [ ] **Step 8: Commit**

```bash
git add pm-lang/src/token_cursor.rs pm-lang/src/ast_parser.rs
git commit -m "$(cat <<'EOF'
feat(pm-lang): coarse declaration-level error recovery in PmAstParser

A syntax error inside a cell/relationship/conditional item no longer
aborts the whole parse: it's recorded in Sheet.errors, a SheetItem::
Error placeholder covers the skipped tokens, and parsing resumes at
the next sheet item (recovery point: a ';' or balancing '}' at the
current nesting depth). This is what lets the future pm-lsp report
every syntax error in a file instead of just the first.
EOF
)"
```

---

### Task 5: Comment/trivia reattachment

**Files:**
- Modify: `cel-parser/src/error.rs` (expose the existing private `span_to_byte_range` logic as a
  public `SourceSpan::to_byte_range` method — no behavior change, purely a visibility/API
  addition)
- Create: `pm-lang/src/trivia.rs`
- Modify: `pm-lang/src/lib.rs` (add `mod trivia;` and `pub use trivia::attach_trivia;`)

**Interfaces:**
- Produces: `cel_parser::SourceSpan::to_byte_range(&self, source: &str) ->
  std::ops::Range<usize>` (used by this task only, in `pm-lang`); `pm_lang::attach_trivia(source:
  &str, sheet: &mut ast::Sheet)`.
- Scope: only gaps **between** two consecutive `sheet.items` are recovered and attached as the
  `leading_comment` of the following item. A comment right after the sheet's opening `{` (before
  its first item) is not yet recovered — `ast::Sheet` does not carry the opening brace's span,
  and adding it is not needed for this plan's tests; deferred to whichever future task actually
  needs it.

- [ ] **Step 1: Write the failing tests**

Add a test module to `cel-parser/src/error.rs` — find the existing `#[cfg(test)] mod tests { ...
}` block at the end of the file and add this test inside it (do not remove any existing test):

```rust
    #[test]
    fn to_byte_range_matches_manual_slice() {
        let source = "line one\nline two\nline three";
        let span = SourceSpan::new(2, 5, 3, 4);
        let range = span.to_byte_range(source);
        assert_eq!(&source[range], "two\nline");
    }
```

Run: `cargo test -p cel-parser to_byte_range`
Expected: compile error — `no method named \`to_byte_range\` found for struct \`SourceSpan\``.

Then create `pm-lang/src/trivia.rs` with only this content:

```rust
//! Recovers comments discarded by `proc_macro2`'s tokenizer and attaches them to the nearest
//! following [`crate::ast::SheetItem`], the same re-slicing-the-gap technique `rustfmt` uses for
//! the identical problem (see `cel-parser/src/lex_lexer.rs`'s `test_span_preservation`).

use cel_parser::SourceSpan;

use crate::ast::Sheet;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PmAstParser;

    #[test]
    fn attaches_a_line_comment_immediately_before_a_cell_decl() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // the total\n    cell b: i32 = 2;\n}";
        let mut sheet = PmAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment.as_deref(), Some("the total"));
    }

    #[test]
    fn attaches_a_multi_line_comment_block() {
        let source =
            "sheet s {\n    cell a: i32 = 1;\n    // line one\n    // line two\n    cell b: i32 = 2;\n}";
        let mut sheet = PmAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment.as_deref(), Some("line one\nline two"));
    }

    #[test]
    fn attaches_a_single_line_block_comment() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    /* the total */\n    cell b: i32 = 2;\n}";
        let mut sheet = PmAstParser::new().parse_str(source).unwrap();
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
        let mut sheet = PmAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment, None);
    }

    #[test]
    fn no_comment_in_the_gap_leaves_leading_comment_none() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    cell b: i32 = 2;\n}";
        let mut sheet = PmAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(b.leading_comment, None);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test -p pm-lang trivia::`
Expected: compile error — `cannot find function \`attach_trivia\``, since it doesn't exist yet.

- [ ] **Step 3: Expose `SourceSpan::to_byte_range` in `cel-parser/src/error.rs`**

Find:

```rust
/// Converts a [`SourceSpan`] (1-based lines, 0-based character columns) to a byte-offset
/// range within `source`.
///
/// - Complexity: O(n) in the length of `source`.
fn span_to_byte_range(source: &str, span: SourceSpan) -> std::ops::Range<usize> {
```

Leave this free function's body exactly as-is, but change its signature line to:

```rust
pub(crate) fn span_to_byte_range(source: &str, span: SourceSpan) -> std::ops::Range<usize> {
```

Then, in `impl SourceSpan { ... }`, add this method (anywhere after `from_proc_macro2_range`):

```rust
    /// Converts this span (1-based lines, 0-based character columns) to a byte-offset range
    /// within `source`.
    ///
    /// - Precondition: `source` is the exact original text this span's line/column positions
    ///   were recorded against.
    ///
    /// - Complexity: O(n) in the length of `source`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::SourceSpan;
    ///
    /// let span = SourceSpan::new(1, 0, 1, 5);
    /// let range = span.to_byte_range("hello world");
    /// assert_eq!(&"hello world"[range], "hello");
    /// ```
    pub fn to_byte_range(&self, source: &str) -> std::ops::Range<usize> {
        span_to_byte_range(source, *self)
    }
```

- [ ] **Step 4: Run the `cel-parser` test to verify it passes**

Run: `cargo test -p cel-parser to_byte_range`
Expected: `to_byte_range_matches_manual_slice` passes.

- [ ] **Step 5: Implement `attach_trivia`**

Add this content **above** the `#[cfg(test)] mod tests { ... }` block already in
`pm-lang/src/trivia.rs` (the module doc comment and `use` lines from Step 1 stay where they are):

```rust
/// Recovers comments from the gaps between consecutive [`crate::ast::SheetItem`]s in `sheet` and
/// attaches the trailing contiguous comment block immediately preceding each item as that item's
/// `leading_comment`.
///
/// A comment is attached only if nothing but whitespace-on-the-same-line separates it from the
/// following item — a blank line between an earlier comment and the item breaks the attachment,
/// matching the common convention that a blank line ends a comment's association with what
/// follows.
///
/// - Precondition: `sheet` was parsed from exactly `source` (unmodified), so its items' spans'
///   line/column positions resolve correctly against it.
///
/// - Complexity: O(n) in the length of `source`.
pub fn attach_trivia(source: &str, sheet: &mut Sheet) {
    for i in 1..sheet.items.len() {
        let gap = SourceSpan {
            start: sheet.items[i - 1].span().end.end(),
            end: sheet.items[i].span().start.start(),
        };
        let byte_range = gap.to_byte_range(source);
        let gap_text = &source[byte_range];
        if let Some(comment) = trailing_comment_block(gap_text) {
            sheet.items[i].set_leading_comment(comment);
        }
    }
}

/// Returns the maximal trailing run of `//` line comments (or a single `/* ... */` block
/// comment) in `gap`, joined with `\n`, or `None` if `gap`'s last non-blank line isn't a
/// comment. A blank line breaks the run.
fn trailing_comment_block(gap: &str) -> Option<String> {
    let mut lines: Vec<&str> = gap.lines().collect();
    while matches!(lines.last(), Some(l) if l.trim().is_empty()) {
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
    if collected.is_empty() {
        return None;
    }
    collected.reverse();
    Some(collected.join("\n"))
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p pm-lang trivia::`
Expected: all 5 tests pass (`attaches_a_line_comment_immediately_before_a_cell_decl`,
`attaches_a_multi_line_comment_block`, `attaches_a_single_line_block_comment`,
`does_not_attach_a_comment_separated_by_a_blank_line`,
`no_comment_in_the_gap_leaves_leading_comment_none`).

- [ ] **Step 7: Wire the new module into `pm-lang/src/lib.rs`**

Find:

```rust
pub mod ast;
mod ast_parser;
mod parser;
mod token_cursor;
pub mod type_registry;
```

Replace with:

```rust
pub mod ast;
mod ast_parser;
mod parser;
mod token_cursor;
mod trivia;
pub mod type_registry;
```

Then find:

```rust
pub use ast_parser::PmAstParser;
pub use cel_parser::ParseError;
pub use parser::{ParsedSheet, PmParser};
pub use type_registry::TypeRegistry;
```

Replace with:

```rust
pub use ast_parser::PmAstParser;
pub use cel_parser::ParseError;
pub use parser::{ParsedSheet, PmParser};
pub use trivia::attach_trivia;
pub use type_registry::TypeRegistry;
```

- [ ] **Step 8: Run the full existing `cel-parser` and `pm-lang` test suites to confirm zero
  regressions**

Run: `cargo test -p cel-parser` then `cargo test -p pm-lang`
Expected: every test that existed before this task still passes, plus this task's new tests.

- [ ] **Step 9: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: every test in every crate passes.

- [ ] **Step 10: Format, lint, and build checks**

Run:
```bash
cargo fmt --all
cargo build --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: zero warnings from all five commands.

- [ ] **Step 11: Format and commit**

```bash
git add cel-parser/src/error.rs pm-lang/src/trivia.rs pm-lang/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(pm-lang): comment/trivia reattachment pass

attach_trivia re-slices the gap between consecutive SheetItems' spans
(via the newly-public SourceSpan::to_byte_range) to recover comments
proc_macro2's tokenizer discards, attaching the trailing contiguous
comment block as the following item's leading_comment. Only
inter-item gaps are covered for now; a comment before the sheet's
first item is deferred.
EOF
)"
```

---

## Self-Review

**Spec coverage:** Covers the `pm-lang` half of the design doc's Phase 2 exactly: "Add ... pm-lang
structural AST (`pm-lang`) [Task 2], coarse error recovery [Task 4], and the comment/trivia
reattachment pass [Task 5]. Unit tests assert AST shape per grammar construct [Task 3], recovery
producing multiple diagnostics from one file [Task 4], and trivia round-tripping [Task 5]." The
`cel-parser` half of Phase 2 (`Expr`/`AstContext`) already shipped in PR #42 and is reused as-is
(Task 3 delegates method bodies to `cel_parser::Parser<AstContext>::parse_or_expression_ast`).
Task 1 is new scope beyond the design doc's phasing text, added specifically per your request to
avoid the pm-lang grammar living in two un-synchronized places going forward — see the
Architecture section and the note below on the required follow-on plan.

**Not in scope (explicitly deferred, per this plan's Architecture section):** replacing
`PmParser`'s inline `Sheet`-building grammar with "parse via `PmAstParser`, then compile the AST
into a `Sheet`" — the change that would let `pm-lang`'s grammar live in exactly one place. That
migration touches `TypeRegistry` resolution, `Method`/`DynSegment` compilation from an `Expr`
tree, and every existing `pm-lang` test, and deserves its own plan and review pass; this plan's
Task 1 (`TokenCursor` extraction) is the first, low-risk step toward it, not the whole thing. I
recommend writing that follow-on plan immediately after this one lands.

**Known limitation (accepted):** Task 4's coarse error recovery cannot isolate a CEL expression
error that leaves a dangling, unmatched `}` behind — e.g. `if a { }` (an `if`-expression whose
then-branch fails to parse) inside a method body. `cel_parser`'s `is_if_expression` consumes the
then-branch's opening `{` directly, bypassing `TokenCursor`, but fails before consuming the
matching `}`; because CEL's `if`/`else` grammar reuses `Delimiter::Brace` — the same kind
pm-lang's own `relationship`/`conditional`/method-body braces use — `skip_to_recovery_point` has
no way to distinguish a stray CEL-left brace from a real pm-lang-tracked one by delimiter kind
alone, unlike the analogous dangling-`)` case (parens belong solely to CEL, so kind alone suffices
there). Closing this gap in general requires `cel_parser`'s `Parser<C>` to report back exactly
what it left unbalanced on a failed parse — a larger, cross-crate API change out of scope for this
plan. This is documented as a known limitation with a regression test locking in today's behavior
(`recovery_known_limitation_if_expr_dangling_brace_aborts_whole_parse` in `ast_parser.rs`), and a
GitHub issue tracks the general fix.

**Placeholder scan:** No TBD/TODO; every step shows complete code; no step says "similar to Task
N" without the code (Task 1's `token_cursor.rs` methods are shown in full despite being moved
verbatim, since the "No Placeholders" rule requires complete code in every step regardless of
provenance).

**Type consistency:** `ast::ExprSpan`/`ast::Expr` are `cel_parser::ExprSpan`/`cel_parser::Expr`
re-exports (not redefined), used identically across Tasks 2–5. `TokenCursor`'s method names and
signatures (Task 1) match every call site added in Task 3 (`cursor.peek_span()`,
`cursor.consume_ident()`, `cursor.expect_punct()`, etc.) and Task 4
(`cursor.skip_to_recovery_point()`). `PmAstParser::parse_str`'s signature
(`Result<ast::Sheet>`) is declared in Task 3 and never changes in Task 4/5 — only the populated
value of `Sheet.errors` and `CellDecl`/etc.'s `leading_comment` fields change, exactly as the
Interfaces blocks state.

---

Plan complete and saved to `docs/superpowers/plans/2026-07-18-pm-lang-ast.md`. Two execution options:

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
