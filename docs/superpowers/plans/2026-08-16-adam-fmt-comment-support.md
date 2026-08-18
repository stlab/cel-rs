# adam-fmt Comment Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `adam-lang` first-class, narrowly-scoped `///`/`//!` doc-comment support and fix
three plain-comment recovery bugs (#105, #53, #52) in `adam-lang`'s trivia/formatter pipeline,
resolving issues #58, #105, #53, #52 (with #57 reviewed as background only).

**Architecture:** `cel-parser::lex_lexer::LexLexer` gains two new `Token` variants
(`DocComment`, `Error`) recognized via a committed (non-speculative) parse of `#`-led token
sequences. `adam-lang`'s two parsers (`AdamAstParser` for tooling, `AdamParser` for live
execution) both consume doc-comment token runs at the same grammar positions via a new shared
`TokenCursor::consume_doc_comment_run` primitive. `AdamAstParser` additionally attaches the
recovered text to new `doc_comment` AST fields and widens each item's span so the pre-existing
gap-scanning `trivia.rs` machinery (which recovers plain `//`/`/* */` comments as raw source text)
never sees the doc-comment tokens as text. Plain-comment recovery gains a `Comment` enum
(`Line`/`Block`) so block-comment style survives formatting, `analyze_gap` learns to recognize
multi-line block comments, and every block-shaped AST node (`Sheet`, `RelationshipDecl`,
`ConditionalDecl`, its new `DefaultBranch` arm, `ConditionalBranch`, `OutDecl`) gains a
trailing-trivia slot for the gap between its last child and its own closing `}`.

**Tech Stack:** Rust, `proc_macro2`/`syn` (token-stream parsing), the existing `adam-lang`/
`cel-parser` workspace crates. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-16-adam-fmt-comment-support-design.md`

## Global Constraints

- `cargo fmt --all` must be clean before every commit (enforced by the repo's pre-commit hook via
  `.githooks`) — run it as the last step before `git add`/`git commit` in every task.
- `cargo build --workspace` and `cargo test --workspace` must produce **zero** compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` must be clean; this
  plan only touches `cel-parser`/`adam-lang`, not `begin`, so the two `begin`-specific clippy
  invocations are unaffected but should still be run once at the end (Task 8) since `begin`
  transitively depends on `adam-lang`.
- Every new `pub`/`pub(crate)` function, struct, enum, and field gets a `///` doc comment in
  contract style (summary sentence; `- Precondition:`/`- Postcondition:`/`- Complexity:` bullets
  only where non-obvious or non-O(1); `debug_assert!` for precondition checks, never documented
  failure behavior for precondition violations).
- Arithmetic on signed integers uses `checked_*`, not wrapping — not applicable to any new code in
  this plan (no signed-integer arithmetic is introduced).
- Prefer `&str`/slices over owned `String`/`Vec` clones — the one unavoidable exception is
  `Comment`/doc-comment text, which must be owned (`String`) because it's recovered from a
  transient token/source-text scan and stored on a long-lived AST node.
- Unit tests are derived from each function's contract/public interface, not its implementation;
  precondition violations are not tested.

---

### Task 1: `cel-parser::lex_lexer` — recognize `///`/`//!` as `Token::DocComment`/`Token::Error`

**Files:**
- Modify: `cel-parser/src/lex_lexer.rs:210-253` (the `Token` enum and its `HasSpan` impl)
- Modify: `cel-parser/src/lex_lexer.rs:266-357` (`LexLexer`'s `Iterator::next`)
- Test: `cel-parser/src/lex_lexer.rs` (existing `#[cfg(test)] mod tests` at the bottom of the file)

**Interfaces:**
- Consumes: nothing new — `TokenTree`, `Delimiter`, `Spacing`, `Span` from `proc_macro2`; `Lit`
  from `syn` (all already imported).
- Produces: two new `Token` variants used by every later task —
  `Token::DocComment { text: String, inner: bool, span: Span }` and
  `Token::Error { message: String, span: Span }`.

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block at the bottom of `cel-parser/src/lex_lexer.rs`
(after the existing `fat_arrow_is_two_char_punct` test):

```rust
    #[test]
    fn recognizes_an_outer_doc_comment() {
        let input = TokenStream::from_str("/// the total").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        let token = lexer.next().unwrap();
        match token {
            Token::DocComment { text, inner, .. } => {
                assert_eq!(text, " the total");
                assert!(!inner);
            }
            other => panic!("expected DocComment, got {other:?}"),
        }
        assert!(lexer.next().is_none());
    }

    #[test]
    fn recognizes_an_inner_doc_comment() {
        let input = TokenStream::from_str("//! module docs").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        let token = lexer.next().unwrap();
        match token {
            Token::DocComment { text, inner, .. } => {
                assert_eq!(text, " module docs");
                assert!(inner);
            }
            other => panic!("expected DocComment, got {other:?}"),
        }
    }

    #[test]
    fn doc_comment_run_is_followed_by_the_next_real_token() {
        let input = TokenStream::from_str("/// x\ncell").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        assert!(matches!(lexer.next(), Some(Token::DocComment { .. })));
        match lexer.next() {
            Some(Token::Identifier(ident)) => assert_eq!(ident.to_string(), "cell"),
            other => panic!("expected Identifier(cell), got {other:?}"),
        }
    }

    #[test]
    fn a_non_doc_attribute_is_a_lex_error() {
        let input = TokenStream::from_str("#[foo]").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        let token = lexer.next().unwrap();
        assert!(
            matches!(token, Token::Error { .. }),
            "expected Error, got {token:?}"
        );
    }

    #[test]
    fn a_non_string_doc_value_is_a_lex_error() {
        let input = TokenStream::from_str("#[doc = 5]").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        let token = lexer.next().unwrap();
        assert!(
            matches!(token, Token::Error { .. }),
            "expected Error, got {token:?}"
        );
    }

    #[test]
    fn a_bare_hash_with_no_group_is_a_lex_error() {
        let input = TokenStream::from_str("# foo").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        let token = lexer.next().unwrap();
        assert!(
            matches!(token, Token::Error { .. }),
            "expected Error, got {token:?}"
        );
    }

    #[test]
    fn a_hash_led_group_with_the_wrong_delimiter_is_a_lex_error() {
        let input = TokenStream::from_str("#(foo)").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        let token = lexer.next().unwrap();
        assert!(
            matches!(token, Token::Error { .. }),
            "expected Error, got {token:?}"
        );
    }

    #[test]
    fn extra_tokens_after_the_doc_string_are_a_lex_error() {
        let input = TokenStream::from_str("#[doc = \"x\", extra]").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        let token = lexer.next().unwrap();
        assert!(
            matches!(token, Token::Error { .. }),
            "expected Error, got {token:?}"
        );
    }

    #[test]
    fn doc_comment_token_has_a_span() {
        let input = TokenStream::from_str("/// x").unwrap();
        let mut lexer = LexLexer::new(input.into_iter());
        let token = lexer.next().unwrap();
        let span = HasSpan::span(&token);
        assert!(!span.source_text().unwrap_or_default().is_empty());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cel-parser --lib lex_lexer:: -- --nocapture`
Expected: compile error — `Token::DocComment`/`Token::Error` don't exist yet.

- [ ] **Step 3: Add the two `Token` variants and update `HasSpan`**

In `cel-parser/src/lex_lexer.rs`, change the `Token` enum (currently lines 210-241):

```rust
/// A flattened token that represents elements from a TokenTree stream.
///
/// Groups are flattened into OpenDelim and CloseDelim tokens, making parsing
/// simpler by removing nesting from the token stream.
#[derive(Debug)]
pub enum Token {
    /// A literal value (integer, string, boolean, or float) with eager discrimination.
    Literal(Literal),

    /// An identifier.
    Identifier(Ident),

    /// A punctuation operator (single or multi-character; no heap for 1–2 chars).
    Punct {
        /// The operator (e.g., "+", "&&", "<=").
        op: PunctOp,
        /// Span for error reporting.
        span: Span,
    },

    /// Opening delimiter (flattened from Group).
    OpenDelim {
        /// The type of delimiter (Parenthesis, Brace, Bracket).
        delimiter: Delimiter,
        /// Span for error reporting.
        span: Span,
    },

    /// Closing delimiter (flattened from Group).
    CloseDelim {
        /// The type of delimiter (Parenthesis, Brace, Bracket).
        delimiter: Delimiter,
        /// Span for error reporting.
        span: Span,
    },

    /// A recognized `///`/`//!` doc comment, unwrapped from the `#[doc = "..."]`/
    /// `#![doc = "..."]` attribute-shaped token sequence `proc_macro2` produces for it. General
    /// `#[...]` attribute syntax is not supported — see `Token::Error`.
    DocComment {
        /// The comment's text: the doc-attribute string literal's value verbatim (including its
        /// leading space), e.g. `" the total"` for `/// the total`.
        text: String,
        /// `true` for `//!`/`#![doc]` (inner), `false` for `///`/`#[doc]` (outer).
        inner: bool,
        /// The `#` token's span.
        span: Span,
    },

    /// A `#`-led token sequence that isn't the doc-comment shape above. Flows through like any
    /// other token and fails to match whatever a grammar production expects, surfacing as a
    /// normal parse error at that point.
    Error {
        /// A description of what was expected instead.
        message: String,
        /// Where the mismatch was detected.
        span: Span,
    },
}
```

Then update `impl HasSpan for Token` (currently lines 243-253):

```rust
impl HasSpan for Token {
    fn span(&self) -> Span {
        match self {
            Token::Literal(lit) => lit.span(),
            Token::Identifier(ident) => ident.span(),
            Token::Punct { span, .. } => *span,
            Token::OpenDelim { span, .. } => *span,
            Token::CloseDelim { span, .. } => *span,
            Token::DocComment { span, .. } => *span,
            Token::Error { span, .. } => *span,
        }
    }
}
```

- [ ] **Step 4: Add the committed doc-comment parse to `LexLexer`**

Add a new method on `impl LexLexer` (place it right after `next_token_tree`, i.e. after line 180):

```rust
    /// Parses the token sequence following a `#` as a doc-comment attribute
    /// (`#[doc = "..."]`/`#![doc = "..."]`), the one shape `proc_macro2` produces for `///`/`//!`
    /// comments. Committed, not speculative: once `#` is seen, this always returns a `Token`
    /// (`DocComment` on success, `Error` on any mismatch) — it never falls back to re-emitting
    /// `#` itself as an ordinary `Punct` token. General `#[...]` attribute syntax is not
    /// supported; any other shape becomes a `Token::Error`.
    fn parse_doc_comment_attribute(&mut self, hash_span: Span) -> Token {
        let next = self.next_token_tree();
        let (inner, group_token) = match next {
            Some(TokenTree::Punct(p)) if p.as_char() == '!' => (true, self.next_token_tree()),
            other => (false, other),
        };
        let group = match group_token {
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket => g,
            Some(other) => {
                return Token::Error {
                    message: "expected `[` after `#`".to_string(),
                    span: other.span(),
                };
            }
            None => {
                return Token::Error {
                    message: "expected `[` after `#`".to_string(),
                    span: hash_span,
                };
            }
        };
        let mut inner_tokens = group.stream().into_iter();
        let ident_ok = match inner_tokens.next() {
            Some(TokenTree::Ident(id)) => id.to_string() == "doc",
            _ => false,
        };
        if !ident_ok {
            return Token::Error {
                message: "expected `doc` after `#[`".to_string(),
                span: group.span(),
            };
        }
        let eq_ok = matches!(
            inner_tokens.next(),
            Some(TokenTree::Punct(p)) if p.as_char() == '='
        );
        if !eq_ok {
            return Token::Error {
                message: "expected `=` after `doc`".to_string(),
                span: group.span(),
            };
        }
        let text = match inner_tokens.next() {
            Some(TokenTree::Literal(lit)) => {
                let token_stream: proc_macro2::TokenStream = TokenTree::Literal(lit).into();
                match syn::parse2::<Lit>(token_stream) {
                    Ok(Lit::Str(s)) => s.value(),
                    _ => {
                        return Token::Error {
                            message: "expected a string literal after `doc =`".to_string(),
                            span: group.span(),
                        };
                    }
                }
            }
            _ => {
                return Token::Error {
                    message: "expected a string literal after `doc =`".to_string(),
                    span: group.span(),
                };
            }
        };
        if inner_tokens.next().is_some() {
            return Token::Error {
                message: "unexpected token after `doc = \"...\"`".to_string(),
                span: group.span(),
            };
        }
        Token::DocComment {
            text,
            inner,
            span: hash_span,
        }
    }
```

Then, in `impl Iterator for LexLexer`'s `next` method, insert a check for `#` immediately after
the token is fetched and before the existing `Group`-handling block (i.e. right after the closing
brace of the `let token = match self.next_token_tree() { ... };` block, before
`if let TokenTree::Group(group) = token {`):

```rust
        // A `#` commits to parsing a doc-comment attribute (`#[doc = "..."]`/`#![doc = "..."]`)
        // — the only shape `proc_macro2` produces for `///`/`//!` comments. See
        // `parse_doc_comment_attribute`'s doc comment for why there is no fallback here.
        if let TokenTree::Punct(ref punct) = token {
            if punct.as_char() == '#' {
                let hash_span = punct.span();
                return Some(self.parse_doc_comment_attribute(hash_span));
            }
        }
```

Finally, update the module doc comment at the top of the file (lines 6-12) to carve out this one
exception:

```rust
//! # Error Handling
//!
//! This lexer does not produce errors for ordinary tokens. All input `TokenTree` items come
//! pre-validated from `proc_macro2`, which has already verified correct Rust lexical syntax; the
//! lexer only transforms and flattens those tokens, operations that cannot fail on valid input.
//! The one exception is a `#`-led token sequence that doesn't match the doc-comment attribute
//! shape (`#[doc = "..."]`/`#![doc = "..."]`) `proc_macro2` already produces for `///`/`//!`
//! comments — general `#[...]` attribute syntax is deliberately unsupported, so any other shape
//! becomes a `Token::Error` rather than an `unreachable!()`. Any other impossible state (like
//! receiving a `Punct` or `Group` in `convert_token`) still uses `unreachable!()`, since those
//! represent programming errors, not malformed or unsupported input.
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p cel-parser --lib lex_lexer:: -- --nocapture`
Expected: all `lex_lexer::tests::*` tests pass, including the 8 new ones.

- [ ] **Step 6: Run the full `cel-parser` test suite and lints**

Run: `cargo test -p cel-parser` — expect all pass, zero warnings.
Run: `cargo clippy -p cel-parser --all-targets -- -D warnings` — expect clean.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add cel-parser/src/lex_lexer.rs
git commit -m "$(cat <<'EOF'
feat(cel-parser): recognize /// and //! as Token::DocComment in LexLexer

proc_macro2 expands doc comments into #[doc = "..."]/#![doc = "..."]
attribute-shaped tokens before LexLexer ever sees them. Once `#` is
seen, LexLexer now commits to parsing exactly that shape, emitting
Token::DocComment on success or Token::Error on any mismatch -- never
falling back to re-emitting `#` as an ordinary token. General #[...]
attribute syntax stays unsupported. First step toward #58.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `adam-lang` — `Comment` enum, multi-line block-comment recovery, re-emission (#105, #53)

**Files:**
- Modify: `adam-lang/src/ast.rs` (add `Comment` enum; change every `leading_comment` field's type
  from `Option<String>` to `Option<Comment>`; update `SheetItem::set_leading_comment`; update
  existing tests)
- Modify: `adam-lang/src/trivia.rs` (rewrite `analyze_gap`; update `TriviaTarget::set_leading_comment`
  and all five impls; update existing tests' assertions)
- Modify: `adam-lang/src/fmt.rs` (factor `write_comment` out of `write_trivia`; update every
  `write_trivia` call site's `.as_deref()` → `.as_ref()`; add block-comment tests)

**Interfaces:**
- Consumes: nothing new from earlier tasks.
- Produces: `adam_lang::ast::Comment` (`Line(String)`/`Block(String)`), used by every later task
  that touches `leading_comment` or the new `trailing_comment` fields (Task 6, Task 7).
  `fmt.rs::write_comment(out: &mut String, comment: &ast::Comment, depth: usize)`, reused by
  Task 7's trailing-trivia writer.

- [ ] **Step 1: Write the failing tests**

In `adam-lang/src/trivia.rs`'s existing `#[cfg(test)] mod tests` block, add (after
`attaches_a_single_line_block_comment`):

```rust
    #[test]
    fn attaches_a_multi_line_block_comment() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    /*\n        line one\n        line two\n    */\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(
            b.leading_comment,
            Some(crate::ast::Comment::Block("line one\nline two".to_string()))
        );
    }

    #[test]
    fn attaches_the_issue_105_license_header_repro_as_a_block_comment() {
        let source = "/*\n    Copyright 2013 Adobe\n    ...\n*/\nsheet s {\n    cell a: i32 = 1;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        assert_eq!(
            sheet.leading_comment,
            Some(crate::ast::Comment::Block(
                "Copyright 2013 Adobe\n...".to_string()
            ))
        );
    }

    #[test]
    fn a_line_comment_is_recovered_as_comment_line() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // the total\n    cell b: i32 = 2;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Cell(b) = &sheet.items[1] else {
            panic!("expected Cell");
        };
        assert_eq!(
            b.leading_comment,
            Some(crate::ast::Comment::Line("the total".to_string()))
        );
    }
```

Also update the existing assertions that compare `leading_comment` against a bare string via
`.as_deref()`, replacing every one in this file's test module with the equivalent typed `Comment`
comparison, e.g. change:

```rust
        assert_eq!(b.leading_comment.as_deref(), Some("the total"));
```

to:

```rust
        assert_eq!(b.leading_comment, Some(crate::ast::Comment::Line("the total".to_string())));
```

This applies to every one of: `attaches_a_line_comment_immediately_before_a_cell_decl`,
`attaches_a_multi_line_comment_block` (its expected value becomes
`Comment::Line("line one\nline two".to_string())`), `attaches_a_single_line_block_comment` (becomes
`Comment::Block("the total".to_string())`),
`attaches_a_comment_to_a_relationship_nested_inside_a_conditional_branch`,
`attaches_a_comment_to_a_conditional_branch`,
`attaches_a_comment_to_a_relationship_nested_inside_the_default_branch`,
`attaches_a_leading_comment_before_the_sheet_itself`,
`attaches_a_multi_line_leading_comment_before_the_sheet_itself`,
`attaches_comments_correctly_across_more_than_one_gap` (two assertions),
`attaches_a_comment_preceding_a_recovered_error_item`,
`blank_line_before_an_attached_comment_still_sets_the_flag_true`,
`attaches_a_comment_and_blank_line_to_a_method_inside_a_relationship`,
`attaches_a_comment_to_a_condition_inside_an_out_block`. Each becomes a `Comment::Line(...)`
except the single-line-block-comment test, which becomes `Comment::Block(...)`. The
`does_not_attach_a_comment_separated_by_a_blank_line` and `no_comment_in_the_gap_leaves_leading_comment_none`
tests already assert `None` and need no change beyond the type inferring correctly.

In `adam-lang/src/fmt.rs`'s existing `#[cfg(test)] mod tests` block, add (after
`formats_a_conditional_with_branches_and_a_default_and_no_trailing_commas`):

```rust
    #[test]
    fn formats_a_single_line_block_comment_preserving_its_style() {
        assert_eq!(
            format("sheet s { /* the total */ cell x: i32 = 1; }"),
            "sheet s {\n    /* the total */\n    cell x: i32 = 1;\n}\n"
        );
    }

    #[test]
    fn formats_a_multi_line_block_comment_preserving_its_style() {
        let source =
            "sheet s {\n    /*\n        line one\n        line two\n    */\n    cell x: i32 = 1;\n}";
        let expected =
            "sheet s {\n    /*\n        line one\n        line two\n    */\n    cell x: i32 = 1;\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_the_issue_105_license_header_repro_without_dropping_it() {
        let source =
            "/*\n    Copyright 2013 Adobe\n    ...\n*/\nsheet s {\n    cell a: i32 = 1;\n}";
        let expected = "/*\n    Copyright 2013 Adobe\n    ...\n*/\nsheet s {\n    cell a: i32 = 1;\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn block_comment_formatting_is_idempotent_through_a_reparse() {
        let source = "sheet s {\n    /*\n        line one\n        line two\n    */\n    cell x: i32 = 1;\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-lang --lib trivia:: fmt:: -- --nocapture`
Expected: compile errors (`Comment` doesn't exist yet; type mismatches on `leading_comment`).

- [ ] **Step 3: Add the `Comment` enum and change every `leading_comment` field's type**

In `adam-lang/src/ast.rs`, add this new type right before the `Sheet` struct (before line 12):

```rust
/// A recovered `//`/`/* */` comment, remembering which delimiter style the source used so the
/// formatter can reproduce it instead of normalizing everything to `//`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Comment {
    /// One or more consecutive `// text` lines, joined by `\n`, each with its leading `//`/space
    /// stripped.
    Line(String),
    /// A single `/* text */` block comment (single- or multi-line), its inner text joined by
    /// `\n` with the opening `/*`/closing `*/` and per-line indentation stripped.
    Block(String),
}
```

Then change every `leading_comment: Option<String>` field declaration to
`leading_comment: Option<Comment>` — on `Sheet`, `SheetItem::Error`, `CellDecl`,
`RelationshipDecl`, `OutDecl`, `OutMethodDecl`, `ConditionDecl`, `ConditionalDecl`,
`ConditionalBranch`, and `MethodDecl` (ten fields total; every one is currently declared as
`pub leading_comment: Option<String>,` and its doc comment above it is unchanged).

Change `SheetItem::set_leading_comment`'s signature and body:

```rust
    /// Sets this item's leading comment.
    pub(crate) fn set_leading_comment(&mut self, comment: Comment) {
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

Finally, update `ast.rs`'s own `#[cfg(test)] mod tests` block: the two tests that call
`set_leading_comment` need their argument and assertion updated:

```rust
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
        item.set_leading_comment(Comment::Line("hi".to_string()));
        match item {
            SheetItem::Cell(c) => {
                assert_eq!(c.leading_comment, Some(Comment::Line("hi".to_string())))
            }
            other => panic!("expected Cell, got {other:?}"),
        }
    }
```

```rust
    #[test]
    fn set_leading_comment_sets_the_error_variant() {
        let span = point(Span::call_site());
        let mut item = SheetItem::Error {
            span,
            leading_comment: None,
            blank_line_before: false,
        };
        item.set_leading_comment(Comment::Line("hi".to_string()));
        match item {
            SheetItem::Error {
                leading_comment, ..
            } => {
                assert_eq!(leading_comment, Some(Comment::Line("hi".to_string())))
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }
```

```rust
    #[test]
    fn set_leading_comment_sets_the_out_variant() {
        // ... unchanged setup ...
        item.set_leading_comment(Comment::Line("hi".to_string()));
        match item {
            SheetItem::Out(o) => assert_eq!(o.leading_comment, Some(Comment::Line("hi".to_string()))),
            other => panic!("expected Out, got {other:?}"),
        }
    }
```

- [ ] **Step 4: Rewrite `trivia.rs`'s `TriviaTarget` and `analyze_gap`**

Change `TriviaTarget::set_leading_comment`'s signature (and all five impls — `SheetItem`,
`MethodDecl`, `RelationshipDecl`, `ConditionalBranch`, `ConditionDecl`) from
`fn set_leading_comment(&mut self, comment: String)` to
`fn set_leading_comment(&mut self, comment: crate::ast::Comment)`. Each impl body is unchanged
(they just assign/forward `comment`).

Replace the whole `analyze_gap` function with:

```rust
/// Analyzes one gap between two consecutive items: the maximal trailing run of `//` line
/// comments, or a single `/* ... */` block comment (possibly spanning multiple lines),
/// immediately preceding the next item, if any, and whether a blank line remains anywhere in
/// what's left of the gap once that trailing run is accounted for (see the module doc for why
/// the scan order matters).
///
/// - Complexity: O(n) in the length of `gap`.
fn analyze_gap(gap: &str) -> (Option<crate::ast::Comment>, bool) {
    use crate::ast::Comment;
    let mut lines: Vec<&str> = gap.lines().collect();
    // `gap` ends exactly where the following item's first token begins. When that token isn't
    // at column 0, `lines()`'s final entry is only the leading whitespace before it on its own
    // line, not a blank source line — drop that fragment before scanning for a trailing comment
    // run so a real blank line (a genuine empty entry from `lines()`) still breaks the run.
    if !gap.ends_with('\n') {
        lines.pop();
    }
    let mut comment = None;
    if let Some(last) = lines.last() {
        let trimmed = last.trim();
        if let Some(text) = trimmed.strip_prefix("//") {
            let mut collected = vec![text.trim().to_string()];
            lines.pop();
            while let Some(line) = lines.last() {
                let trimmed = line.trim();
                if let Some(text) = trimmed.strip_prefix("//") {
                    collected.push(text.trim().to_string());
                    lines.pop();
                } else {
                    break;
                }
            }
            collected.reverse();
            comment = Some(Comment::Line(collected.join("\n")));
        } else if let Some(inner) = trimmed
            .strip_prefix("/*")
            .and_then(|s| s.strip_suffix("*/"))
        {
            // A single-line block comment.
            comment = Some(Comment::Block(inner.trim().to_string()));
            lines.pop();
        } else if trimmed.ends_with("*/") {
            // The close of a block comment that opened on an earlier line — collect backwards
            // until the line that opens it (see #105). A block comment is one unit; don't merge
            // with an earlier `//` run.
            let mut collected = Vec::new();
            let closing_prefix = trimmed
                .strip_suffix("*/")
                .expect("checked ends_with(\"*/\") above")
                .trim();
            if !closing_prefix.is_empty() {
                collected.push(closing_prefix.to_string());
            }
            lines.pop();
            let mut found_open = false;
            while let Some(line) = lines.last() {
                let trimmed = line.trim();
                lines.pop();
                if let Some(text) = trimmed.strip_prefix("/*") {
                    let opening_suffix = text.trim();
                    if !opening_suffix.is_empty() {
                        collected.push(opening_suffix.to_string());
                    }
                    found_open = true;
                    break;
                }
                collected.push(trimmed.to_string());
            }
            // If the gap is exhausted without finding a matching `/*` (not expected for
            // well-formed input), `found_open` stays false and no comment is fabricated.
            if found_open {
                collected.reverse();
                comment = Some(Comment::Block(collected.join("\n")));
            }
        }
    }
    // `lines[0]` is always the trailing remainder of the *previous* item's own source line (the
    // fragment before the gap's first `\n`), which is empty whenever that item's last token is
    // already at end-of-line — the common case. It must be excluded here: only a line strictly
    // after it sits between two `\n`s in the original gap and can be a genuine blank line.
    let blank_line_before = lines.len() > 1 && lines[1..].iter().any(|l| l.trim().is_empty());
    (comment, blank_line_before)
}
```

- [ ] **Step 5: Factor `write_comment` out of `fmt.rs`'s `write_trivia` and fix call sites**

In `adam-lang/src/fmt.rs`, replace `write_trivia` with:

```rust
/// Writes one recovered `Comment` at `depth`'s indentation: a `Comment::Line` as one `// ` line
/// per stored line; a `Comment::Block` as `/* text */` on one line when its text has no internal
/// `\n`, or a multi-line `/*`/`*/`-delimited block (one line per stored line, indented one level
/// past `depth`) when it does.
fn write_comment(out: &mut String, comment: &ast::Comment, depth: usize) {
    match comment {
        ast::Comment::Line(text) => {
            for line in text.split('\n') {
                out.push_str(&indent(depth));
                out.push_str("// ");
                out.push_str(line);
                out.push('\n');
            }
        }
        ast::Comment::Block(text) => {
            out.push_str(&indent(depth));
            if text.contains('\n') {
                out.push_str("/*\n");
                for line in text.split('\n') {
                    out.push_str(&indent(depth + 1));
                    out.push_str(line);
                    out.push('\n');
                }
                out.push_str(&indent(depth));
                out.push_str("*/\n");
            } else {
                out.push_str("/* ");
                out.push_str(text);
                out.push_str(" */\n");
            }
        }
    }
}

/// Emits `blank_line_before`/`leading_comment` ahead of an item, if either is present.
fn write_trivia(
    out: &mut String,
    blank_line_before: bool,
    leading_comment: Option<&ast::Comment>,
    depth: usize,
) {
    if blank_line_before {
        out.push('\n');
    }
    if let Some(comment) = leading_comment {
        write_comment(out, comment, depth);
    }
}
```

Then change every call site's trailing `.as_deref()` to `.as_ref()` (nine call sites: in
`write_method`, `write_relationship`, `write_branch`, `write_conditional`, `write_cell`,
`write_out_method`, `write_condition`, `write_out`, and `format_sheet`), e.g.:

```rust
    write_trivia(
        out,
        method.blank_line_before,
        method.leading_comment.as_ref(),
        depth,
    );
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p adam-lang --lib`
Expected: all tests pass, including the new ones from Step 1.

- [ ] **Step 7: Run lints**

Run: `cargo clippy -p adam-lang --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add adam-lang/src/ast.rs adam-lang/src/trivia.rs adam-lang/src/fmt.rs
git commit -m "$(cat <<'EOF'
fix(adam-lang): preserve block-comment style and recover multi-line blocks

Adds a Comment::{Line,Block} enum so leading_comment remembers which
delimiter style the source used (fixes #53, where every /* */ block
comment was re-emitted as //), and teaches analyze_gap to recognize a
block comment whose /* and */ sit on different lines instead of
dropping it silently (fixes #105).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `adam-lang` — parse and attach `///`/`//!` doc comments (#58)

**Files:**
- Modify: `adam-lang/src/token_cursor.rs` (add `consume_doc_comment_run`)
- Modify: `adam-lang/src/ast.rs` (add `doc_comment: Option<String>` to `Sheet`, `CellDecl`,
  `RelationshipDecl`, `ConditionalDecl`, `OutDecl`, `SheetItem::Error`; add `SheetItem::set_doc_comment`)
- Modify: `adam-lang/src/ast_parser.rs` (`parse_str` peels sheet-level `//!`; `parse_sheet`'s item
  loop peels outer `///` runs and attaches them, widening the item's span)

**Interfaces:**
- Consumes: `Token::DocComment` from Task 1.
- Produces: `TokenCursor::consume_doc_comment_run(&mut self, inner: bool) -> Option<(String, Span)>`,
  used directly by Task 5 (`AdamParser`). `doc_comment: Option<String>` fields and
  `SheetItem::set_doc_comment`, used by Task 4 (formatter re-emission).

- [ ] **Step 1: Write the failing tests**

In `adam-lang/src/token_cursor.rs`'s existing `#[cfg(test)] mod tests` block, add:

```rust
    #[test]
    fn consume_doc_comment_run_returns_none_when_next_token_is_not_a_doc_comment() {
        let stream = proc_macro2::TokenStream::from_str("cell").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        assert!(cursor.consume_doc_comment_run(false).is_none());
    }

    #[test]
    fn consume_doc_comment_run_joins_consecutive_matching_doc_comments() {
        let stream = proc_macro2::TokenStream::from_str("/// a\n/// b\ncell").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        let (text, _) = cursor.consume_doc_comment_run(false).expect("doc comment run");
        assert_eq!(text, " a\n b");
        assert!(cursor.is_keyword("cell"));
    }

    #[test]
    fn consume_doc_comment_run_does_not_consume_a_mismatched_inner_flag() {
        let stream = proc_macro2::TokenStream::from_str("//! inner\ncell").unwrap();
        let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
        assert!(cursor.consume_doc_comment_run(false).is_none());
        let (text, _) = cursor.consume_doc_comment_run(true).expect("doc comment run");
        assert_eq!(text, " inner");
    }
```

In `adam-lang/src/ast_parser.rs`'s existing `#[cfg(test)] mod tests` block, add:

```rust
    #[test]
    fn attaches_an_outer_doc_comment_to_a_cell() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s {\n    /// the total\n    cell x: i32 = 1;\n}")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert_eq!(cell.doc_comment.as_deref(), Some(" the total"));
    }

    #[test]
    fn attaches_an_outer_doc_comment_to_a_relationship() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s {\n    /// docs\n    relationship { method [a] -> [b] { a } }\n}")
            .unwrap();
        let ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert_eq!(rel.doc_comment.as_deref(), Some(" docs"));
    }

    #[test]
    fn attaches_an_outer_doc_comment_to_a_conditional() {
        let sheet = AdamAstParser::new()
            .parse_str(
                "sheet s {\n    cell p: i32 = 0;\n    /// docs\n    conditional p {\n        _ => { relationship { method [a] -> [b] { a } } }\n    }\n}",
            )
            .unwrap();
        let ast::SheetItem::Conditional(cond) = &sheet.items[1] else {
            panic!("expected Conditional");
        };
        assert_eq!(cond.doc_comment.as_deref(), Some(" docs"));
    }

    #[test]
    fn attaches_an_outer_doc_comment_to_an_out_decl() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s {\n    /// docs\n    out area: f64 {\n        method [w] { w }\n    }\n}")
            .unwrap();
        let ast::SheetItem::Out(out) = &sheet.items[0] else {
            panic!("expected Out");
        };
        assert_eq!(out.doc_comment.as_deref(), Some(" docs"));
    }

    #[test]
    fn attaches_a_sheet_level_inner_doc_comment() {
        let sheet = AdamAstParser::new()
            .parse_str("//! module docs\nsheet s {\n    cell x: i32 = 1;\n}")
            .unwrap();
        assert_eq!(sheet.doc_comment.as_deref(), Some(" module docs"));
    }

    #[test]
    fn joins_consecutive_outer_doc_comment_lines() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s {\n    /// line one\n    /// line two\n    cell x: i32 = 1;\n}")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert_eq!(cell.doc_comment.as_deref(), Some(" line one\n line two"));
    }

    #[test]
    fn a_doc_comment_binds_forward_across_a_blank_line() {
        let sheet = AdamAstParser::new()
            .parse_str("sheet s {\n    /// docs\n\n    cell x: i32 = 1;\n}")
            .unwrap();
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert_eq!(cell.doc_comment.as_deref(), Some(" docs"));
    }

    #[test]
    fn a_plain_comment_and_a_doc_comment_coexist_in_source_order() {
        let source = "sheet s {\n    // TODO\n    /// docs\n    cell x: i32 = 1;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        crate::attach_trivia(source, &mut sheet);
        let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
            panic!("expected Cell");
        };
        assert_eq!(cell.doc_comment.as_deref(), Some(" docs"));
        assert_eq!(
            cell.leading_comment,
            Some(ast::Comment::Line("TODO".to_string()))
        );
    }

    #[test]
    fn a_doc_comment_before_a_method_recovers_as_a_declaration_level_error() {
        let sheet = AdamAstParser::new()
            .parse_str(
                "sheet s {\n    relationship {\n        /// not allowed here\n        method [a] -> [b] { a }\n    }\n}",
            )
            .unwrap();
        assert!(!sheet.errors.is_empty());
        assert!(matches!(sheet.items[0], ast::SheetItem::Error { .. }));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-lang --lib token_cursor:: ast_parser:: -- --nocapture`
Expected: compile errors (`consume_doc_comment_run`/`doc_comment` don't exist yet).

- [ ] **Step 3: Add `TokenCursor::consume_doc_comment_run`**

In `adam-lang/src/token_cursor.rs`, add this method to `impl TokenCursor` (place it after
`consume_literal`, before `at_close_brace`):

```rust
    /// Consumes a leading run of consecutive `Token::DocComment` tokens matching `inner`,
    /// returning their joined text (`\n`-separated) and the first token's span, or `None` if the
    /// next token isn't a matching doc comment.
    ///
    /// - Complexity: O(k), where k is the number of consecutive matching doc-comment tokens.
    pub(crate) fn consume_doc_comment_run(&mut self, inner: bool) -> Option<(String, Span)> {
        let first_span = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::DocComment { inner: i, span, .. }) if *i == inner => Some(*span),
            _ => None,
        }?;
        let mut lines = Vec::new();
        loop {
            let next_text = match self.tokens.as_mut().and_then(|t| t.peek()) {
                Some(Token::DocComment { inner: i, text, .. }) if *i == inner => {
                    Some(text.clone())
                }
                _ => None,
            };
            match next_text {
                Some(text) => {
                    lines.push(text);
                    self.advance();
                }
                None => break,
            }
        }
        Some((lines.join("\n"), first_span))
    }
```

- [ ] **Step 4: Add `doc_comment` fields and `SheetItem::set_doc_comment` in `ast.rs`**

Add a new field `doc_comment: Option<String>` right after the `leading_comment` field on: `Sheet`,
`CellDecl`, `RelationshipDecl`, `ConditionalDecl`, `OutDecl`. Give it a one-line doc comment
describing it as a leading doc comment immediately preceding the declaration, if recovered by
`AdamAstParser`. On `SheetItem::Error`, add the same field with a doc comment noting it may have
been consumed before parsing failed.

Add to `impl SheetItem` (after `set_blank_line_before`):

```rust
    /// Sets this item's doc comment and widens its span to start at `start` (the doc comment's
    /// own first token), so `trivia::attach_trivia`'s gap scan stops before the doc comment's
    /// source text instead of misparsing it as a plain `//` comment.
    pub(crate) fn set_doc_comment(&mut self, text: String, start: proc_macro2::Span) {
        match self {
            SheetItem::Cell(c) => {
                c.doc_comment = Some(text);
                c.span.start = start;
            }
            SheetItem::Relationship(r) => {
                r.doc_comment = Some(text);
                r.span.start = start;
            }
            SheetItem::Conditional(c) => {
                c.doc_comment = Some(text);
                c.span.start = start;
            }
            SheetItem::Out(o) => {
                o.doc_comment = Some(text);
                o.span.start = start;
            }
            SheetItem::Error {
                doc_comment, span, ..
            } => {
                *doc_comment = Some(text);
                span.start = start;
            }
        }
    }
```

Update every existing struct literal in `ast.rs`'s own tests that constructs `CellDecl`,
`RelationshipDecl`, `ConditionalDecl`, `OutDecl`, or `SheetItem::Error` directly, adding
`doc_comment: None,` alongside their existing `leading_comment: None,` line (this affects
`sheet_item_span_reads_the_cell_variant`, `sheet_item_span_reads_the_relationship_variant`,
`sheet_item_span_reads_the_conditional_variant`, `sheet_item_span_reads_the_error_variant`,
`set_leading_comment_sets_the_cell_variant`, `set_leading_comment_sets_the_error_variant`,
`sheet_item_span_reads_the_out_variant`, `set_leading_comment_sets_the_out_variant`,
`set_blank_line_before_sets_the_cell_variant`, `cell_decl_type_name_holds_a_nested_tuple_type_expr`,
`cell_decl_initializer_holds_a_parsed_expr`).

- [ ] **Step 5: Wire doc-comment peeling into `ast_parser.rs`**

Change `AdamAstParser::parse_str` (currently lines 78-90):

```rust
    pub fn parse_str(&mut self, source: &str) -> Result<ast::Sheet> {
        use std::str::FromStr;
        let stream = proc_macro2::TokenStream::from_str(source)
            .map_err(|e| cel_parser::ParseError::from_lex_error(source, e))?;
        let mut cursor =
            TokenCursor::new(cel_parser::lex_lexer::LexLexer::new(stream.into_iter()).peekable());
        let doc_comment = cursor.consume_doc_comment_run(true);
        let mut sheet = self.parse_sheet(&mut cursor)?;
        if let Some((text, span)) = doc_comment {
            sheet.doc_comment = Some(text);
            sheet.span.start = span;
        }
        if let Some(tok) = cursor.peek_token() {
            use cel_parser::lex_lexer::HasSpan;
            return Err(cel_parser::ParseError::new("unexpected token", tok.span()));
        }
        Ok(sheet)
    }
```

Change `parse_sheet`'s item loop and `Sheet` literal (currently lines 93-135):

```rust
    /// `sheet = "sheet" identifier "{" { sheet_item } "}".`
    fn parse_sheet(&mut self, cursor: &mut TokenCursor) -> Result<ast::Sheet> {
        let sheet_start = cursor.peek_span();
        if !cursor.is_keyword("sheet") {
            return Err(cursor.err_at("expected `sheet`"));
        }
        let (name, name_span) = cursor.consume_ident()?;
        cursor.expect_open_brace()?;
        let mut items = Vec::new();
        let mut errors = Vec::new();
        while !cursor.at_close_brace() {
            let doc = cursor.consume_doc_comment_run(false);
            let item_start = doc
                .as_ref()
                .map(|(_, span)| *span)
                .unwrap_or_else(|| cursor.peek_span());
            cursor.set_last_span(item_start);
            let target_depth = cursor.depth();
            match self.parse_sheet_item(cursor) {
                Ok(mut item) => {
                    if let Some((text, doc_span)) = &doc {
                        item.set_doc_comment(text.clone(), *doc_span);
                    }
                    items.push(item);
                }
                Err(e) => {
                    errors.push(e);
                    let recovery_fallback = cursor.last_span();
                    let item_end = cursor.skip_to_recovery_point(target_depth, recovery_fallback);
                    items.push(ast::SheetItem::Error {
                        span: ast::ExprSpan {
                            start: item_start,
                            end: item_end,
                        },
                        leading_comment: None,
                        doc_comment: doc.map(|(text, _)| text),
                        blank_line_before: false,
                    });
                }
            }
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::Sheet {
            name,
            name_span: point(name_span),
            items,
            leading_comment: None,
            doc_comment: None,
            span: ast::ExprSpan {
                start: sheet_start,
                end: close_span,
            },
            errors,
        })
    }
```

Finally, add `doc_comment: None,` to the four struct literals `parse_cell_decl`,
`parse_relationship_decl`, `parse_conditional_decl`, and `parse_out_decl` build (alongside their
existing `leading_comment: None,` line each).

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p adam-lang --lib`
Expected: all tests pass.

- [ ] **Step 7: Run lints**

Run: `cargo clippy -p adam-lang --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add adam-lang/src/token_cursor.rs adam-lang/src/ast.rs adam-lang/src/ast_parser.rs
git commit -m "$(cat <<'EOF'
feat(adam-lang): parse and attach /// and //! doc comments (#58)

AdamAstParser now peels a leading run of outer (///) doc-comment
tokens before each sheet item (cell/relationship/conditional/out) and
a leading run of inner (//!) tokens before the sheet keyword itself,
attaching the joined text to a new doc_comment field and widening the
item's span so attach_trivia's gap scan stops before the doc comment's
source text instead of misparsing it as a plain // comment.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: `adam-lang::fmt` — re-emit doc comments

**Files:**
- Modify: `adam-lang/src/fmt.rs` (add `write_doc_comment`; call it from `write_cell`,
  `write_relationship`, `write_conditional`, `write_out`, and `format_sheet`)

**Interfaces:**
- Consumes: `doc_comment: Option<String>` fields from Task 3.
- Produces: `write_doc_comment(out: &mut String, marker: &str, doc_comment: Option<&str>, depth: usize)`.

- [ ] **Step 1: Write the failing tests**

In `adam-lang/src/fmt.rs`'s test module, add:

```rust
    #[test]
    fn formats_a_cell_with_a_doc_comment() {
        assert_eq!(
            format("sheet s { /// the total\n cell x: i32 = 1; }"),
            "sheet s {\n    /// the total\n    cell x: i32 = 1;\n}\n"
        );
    }

    #[test]
    fn formats_a_sheet_level_doc_comment() {
        assert_eq!(
            format("//! module docs\nsheet s { cell x: i32 = 1; }"),
            "//! module docs\nsheet s {\n    cell x: i32 = 1;\n}\n"
        );
    }

    #[test]
    fn formats_a_plain_comment_and_doc_comment_together_in_source_order() {
        // Two items, not one: `trivia::attach_gaps` never attaches a leading plain comment to a
        // list's first element (a pre-existing, out-of-scope #52-adjacent limitation unrelated to
        // doc comments — see Tasks 2/3's identical fixture fix), so `x` needs a preceding sibling
        // for its `// TODO` to actually attach via the normal gap-scanning path.
        let source = "sheet s {\n    cell w: i32 = 0;\n    // TODO\n    /// docs\n    cell x: i32 = 1;\n}";
        let expected = "sheet s {\n    cell w: i32 = 0;\n    // TODO\n    /// docs\n    cell x: i32 = 1;\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_doc_comments_on_a_relationship_conditional_and_out() {
        let source = "sheet s {\n    /// r\n    relationship { method [a] -> [b] { a } }\n\n    /// o\n    out area: f64 {\n        method [w] { w }\n    }\n}";
        let expected = "sheet s {\n    /// r\n    relationship {\n        method [a] -> [b] { a }\n    }\n\n    /// o\n    out area: f64 {\n        method [w] { w }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn doc_comment_formatting_is_idempotent_through_a_reparse() {
        let source = "sheet s {\n    /// the total\n    cell x: i32 = 1;\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-lang --lib fmt:: -- --nocapture`
Expected: fail (no doc comment is emitted yet — output is missing the `///`/`//!` lines).

- [ ] **Step 3: Add `write_doc_comment` and wire it into every writer**

Add this function to `adam-lang/src/fmt.rs` (right after `write_comment`):

```rust
/// Writes `doc_comment`'s lines (if present) as one `marker` line per stored line, at `depth`'s
/// indentation. `marker` is `"///"` for a declaration's own doc comment or `"//!"` for the
/// sheet's own (each stored line already includes the space rustdoc puts after the marker, e.g.
/// `" the total"` for `/// the total`, so no extra space is inserted here).
fn write_doc_comment(out: &mut String, marker: &str, doc_comment: Option<&str>, depth: usize) {
    if let Some(doc) = doc_comment {
        for line in doc.split('\n') {
            out.push_str(&indent(depth));
            out.push_str(marker);
            out.push_str(line);
            out.push('\n');
        }
    }
}
```

Then, in each of `write_cell`, `write_relationship`, `write_conditional`, `write_out`, add a call
to `write_doc_comment(out, "///", <decl>.doc_comment.as_deref(), depth);` immediately after that
function's existing `write_trivia(...)` call and before it starts writing the declaration itself.
For example, `write_cell` becomes:

```rust
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
    // ... unchanged from here ...
}
```

Apply the same one-line insertion to `write_relationship` (after its `write_trivia` call, using
`rel.doc_comment`), `write_conditional` (using `cond.doc_comment`), and `write_out` (using
`decl.doc_comment`).

Finally, in `format_sheet`, add a call for the sheet's own `//!` doc comment right after its
existing `write_trivia` call:

```rust
pub fn format_sheet(sheet: &ast::Sheet) -> String {
    debug_assert!(
        sheet.errors.is_empty(),
        "format_sheet's precondition: no recorded syntax errors"
    );
    let mut out = String::new();
    write_trivia(&mut out, false, sheet.leading_comment.as_ref(), 0);
    write_doc_comment(&mut out, "//!", sheet.doc_comment.as_deref(), 0);
    out.push_str(&format!("sheet {} {{\n", sheet.name));
    for item in &sheet.items {
        write_sheet_item(&mut out, item, 1);
    }
    out.push_str("}\n");
    out
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p adam-lang --lib fmt::`
Expected: all pass.

- [ ] **Step 5: Run the full `adam-lang` suite and lints**

Run: `cargo test -p adam-lang` and `cargo clippy -p adam-lang --all-targets -- -D warnings`
Expected: all pass, clean.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add adam-lang/src/fmt.rs
git commit -m "$(cat <<'EOF'
feat(adam-lang): re-emit /// and //! doc comments in format_sheet

Completes #58 end-to-end for the formatter: a cell/relationship/
conditional/out's doc_comment (and the sheet's own) now round-trips
through format_sheet, printed ahead of any plain leading comment.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: `adam-lang::parser` (`AdamParser`) — accept doc comments at runtime

**Files:**
- Modify: `adam-lang/src/parser.rs:132-190` (`parse_str`, `parse_sheet_item`)

**Interfaces:**
- Consumes: `TokenCursor::consume_doc_comment_run` from Task 3 (via `ParseContext`'s `Deref` to
  `TokenCursor`).
- Produces: nothing new — this task only keeps `AdamParser`'s accepted grammar in sync with
  `AdamAstParser`'s.

- [ ] **Step 1: Write the failing tests**

In `adam-lang/src/parser.rs`'s existing `#[cfg(test)] mod tests` block, add (after
`parse_relationship_single_method`):

```rust
    #[test]
    fn parses_a_sheet_with_an_outer_doc_comment_on_a_cell() {
        let parsed = parser()
            .parse_str("sheet s {\n    /// the total\n    cell x: i32 = 1;\n}")
            .unwrap();
        assert_eq!(parsed.cell_names.len(), 1);
    }

    #[test]
    fn parses_a_sheet_with_an_inner_doc_comment() {
        let parsed = parser()
            .parse_str("//! module docs\nsheet s {\n    cell x: i32 = 1;\n}")
            .unwrap();
        assert_eq!(parsed.cell_names.len(), 1);
    }

    #[test]
    fn parses_a_sheet_with_doc_comments_on_every_declaration_kind() {
        let source = "//! module docs\nsheet s {\n    /// a cell\n    cell x: i32 = 1;\n\n    /// a relationship\n    relationship { method [x] -> [y] { x } }\n}";
        let parsed = parser().parse_str(source).unwrap();
        assert_eq!(parsed.cell_names.len(), 2);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-lang --lib parser:: -- --nocapture`
Expected: fail with a parse error (the doc-comment tokens are unexpected).

- [ ] **Step 3: Peel and discard doc-comment runs in `AdamParser`**

Change `parse_str` (currently lines 132-150):

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
        let _ = ctx.consume_doc_comment_run(true); // sheet-level `//!` docs (ignored at runtime)
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

Change `parse_sheet_item` (currently lines 173-190):

```rust
    /// `sheet_item = [ doc_comment ] (cell_decl | relationship_decl | conditional_decl | out_decl).`
    fn parse_sheet_item(&mut self, ctx: &mut ParseContext) -> Result<()> {
        let _ = ctx.consume_doc_comment_run(false); // outer `///` docs (ignored at runtime)
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

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p adam-lang --lib parser::`
Expected: all pass.

- [ ] **Step 5: Run the full `adam-lang` suite and lints**

Run: `cargo test -p adam-lang` and `cargo clippy -p adam-lang --all-targets -- -D warnings`
Expected: all pass, clean.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add adam-lang/src/parser.rs
git commit -m "$(cat <<'EOF'
fix(adam-lang): accept doc comments in AdamParser, the live-execution path

AdamAstParser (formatter/LSP) and AdamParser (begin's runtime path)
must accept the same grammar. Without this, a .adm2 file using ///
or //! would format correctly but fail to load in begin. AdamParser
peels and discards doc-comment token runs at the same positions --
it preserves no comments of any kind today, so this is a pure
skip-and-ignore, not new state.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: `adam-lang` — recover trailing trivia before a block's closing `}` (#52)

**Files:**
- Modify: `adam-lang/src/ast.rs` (add `DefaultBranch` struct; change
  `ConditionalDecl::default`'s type; add `trailing_comment`/`blank_line_before_close` fields to
  `Sheet`, `RelationshipDecl`, `ConditionalDecl`, `ConditionalBranch`, `OutDecl`; add
  `open_brace_span` to `Sheet`, `RelationshipDecl`, `ConditionalDecl`, `ConditionalBranch`)
- Modify: `adam-lang/src/ast_parser.rs` (capture each container's open-brace span; build
  `DefaultBranch`; populate the new fields with neutral initial values)
- Modify: `adam-lang/src/trivia.rs` (add `TrailingTriviaTarget` trait + impls; add
  `attach_trailing`, `attach_conditional_trailing`, `attach_out_trailing`; wire them into
  `attach_trivia`/`attach_relationship`/`attach_conditional`/`attach_out`)

**Interfaces:**
- Consumes: `Comment` and `analyze_gap` from Task 2.
- Produces: `trailing_comment: Option<Comment>`/`blank_line_before_close: bool` fields on every
  block-shaped container, populated by `attach_trivia`; consumed by Task 7 (formatter).

- [ ] **Step 1: Write the failing tests**

In `adam-lang/src/trivia.rs`'s test module, add:

```rust
    #[test]
    fn recovers_a_trailing_comment_before_a_sheets_closing_brace() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // trailing\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        assert_eq!(
            sheet.trailing_comment,
            Some(crate::ast::Comment::Line("trailing".to_string()))
        );
    }

    #[test]
    fn recovers_a_trailing_comment_in_an_empty_relationship_block() {
        let source = "sheet s {\n    relationship {\n        // only this\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert_eq!(
            rel.trailing_comment,
            Some(crate::ast::Comment::Line("only this".to_string()))
        );
    }

    #[test]
    fn recovers_a_trailing_comment_before_a_relationships_closing_brace() {
        let source = "sheet s {\n    relationship {\n        method [a] -> [b] { a }\n        // trailing\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
            panic!("expected Relationship");
        };
        assert_eq!(
            rel.trailing_comment,
            Some(crate::ast::Comment::Line("trailing".to_string()))
        );
    }

    #[test]
    fn recovers_a_trailing_comment_before_a_conditional_branchs_closing_brace() {
        let source = "sheet s {\n    conditional m {\n        0i32 => {\n            relationship { method [a] -> [b] { a } }\n            // trailing\n        }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(
            cond.branches[0].trailing_comment,
            Some(crate::ast::Comment::Line("trailing".to_string()))
        );
    }

    #[test]
    fn recovers_a_trailing_comment_in_a_default_arm() {
        let source = "sheet s {\n    conditional m {\n        _ => {\n            relationship { method [a] -> [b] { a } }\n            // trailing\n        }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        let default = cond.default.as_ref().expect("default branch present");
        assert_eq!(
            default.trailing_comment,
            Some(crate::ast::Comment::Line("trailing".to_string()))
        );
    }

    #[test]
    fn recovers_a_trailing_comment_before_a_conditionals_own_closing_brace() {
        let source = "sheet s {\n    conditional m {\n        0i32 => { relationship { method [a] -> [b] { a } } }\n        // trailing\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(
            cond.trailing_comment,
            Some(crate::ast::Comment::Line("trailing".to_string()))
        );
    }

    #[test]
    fn recovers_a_trailing_comment_before_an_outs_closing_brace_with_no_conditions() {
        let source = "sheet s {\n    out area: f64 {\n        method [w] { w }\n        // trailing\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Out(out) = &sheet.items[0] else {
            panic!("expected Out");
        };
        assert_eq!(
            out.trailing_comment,
            Some(crate::ast::Comment::Line("trailing".to_string()))
        );
    }

    #[test]
    fn recovers_a_trailing_comment_before_an_outs_closing_brace_after_a_condition() {
        let source = "sheet s {\n    out area: f64 {\n        method [w] { w }\n        condition c [w] { w <= 10.0 }\n        // trailing\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Out(out) = &sheet.items[0] else {
            panic!("expected Out");
        };
        assert_eq!(
            out.trailing_comment,
            Some(crate::ast::Comment::Line("trailing".to_string()))
        );
    }

    #[test]
    fn sets_blank_line_before_close_when_a_blank_line_precedes_the_closing_brace() {
        let source = "sheet s {\n    cell a: i32 = 1;\n\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        assert!(sheet.blank_line_before_close);
    }

    #[test]
    fn no_trailing_comment_leaves_trailing_comment_none() {
        let source = "sheet s {\n    cell a: i32 = 1;\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        assert_eq!(sheet.trailing_comment, None);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-lang --lib trivia:: -- --nocapture`
Expected: compile errors (`trailing_comment`/`DefaultBranch` don't exist yet).

- [ ] **Step 3: Add the new AST surface**

In `adam-lang/src/ast.rs`, add `trailing_comment: Option<Comment>`, `blank_line_before_close: bool`
fields (each with a one-line doc comment referencing #52) to `Sheet`, `RelationshipDecl`,
`ConditionalDecl`, `ConditionalBranch`, and `OutDecl`. Additionally add
`open_brace_span: ExprSpan` to `Sheet`, `RelationshipDecl`, `ConditionalDecl`, and
`ConditionalBranch` (not `OutDecl`, whose writer is grammar-mandatory so its block can never be
child-empty).

Replace `ConditionalDecl::default`'s field and add the new `DefaultBranch` struct:

```rust
/// The `_ => { ... }` default branch of a `conditional_decl`, mirroring `ConditionalBranch`'s
/// shape (it has no match literal of its own).
#[derive(Debug, Clone)]
pub struct DefaultBranch {
    /// The default branch's relationships, in declaration order.
    pub relationships: Vec<RelationshipDecl>,
    /// A trailing comment immediately preceding this branch's own closing `}`, if recovered.
    /// See <https://github.com/stlab/cel-rs/issues/52>.
    pub trailing_comment: Option<Comment>,
    /// Whether a blank line preceded this branch's own closing `}`, if recovered.
    pub blank_line_before_close: bool,
    /// The span of this branch's own opening `{`, used to recover trailing trivia when
    /// `relationships` is empty.
    pub open_brace_span: ExprSpan,
    /// The span from the `_` token through this branch's own closing `}`.
    pub span: ExprSpan,
}
```

And on `ConditionalDecl`:

```rust
    /// The `_ => { ... }` default branch, if present.
    pub default: Option<DefaultBranch>,
```

- [ ] **Step 4: Update `ast_parser.rs` to populate the new fields**

Change `parse_sheet`'s `Sheet` construction: capture `cursor.expect_open_brace()?`'s return value
(currently discarded) into `open_span`, and add `trailing_comment: None`,
`blank_line_before_close: false`, `open_brace_span: point(open_span)` to the `Ok(ast::Sheet { ... })`
literal (this is the version from Task 3, further extended):

```rust
    fn parse_sheet(&mut self, cursor: &mut TokenCursor) -> Result<ast::Sheet> {
        let sheet_start = cursor.peek_span();
        if !cursor.is_keyword("sheet") {
            return Err(cursor.err_at("expected `sheet`"));
        }
        let (name, name_span) = cursor.consume_ident()?;
        let open_span = cursor.expect_open_brace()?;
        let mut items = Vec::new();
        let mut errors = Vec::new();
        while !cursor.at_close_brace() {
            let doc = cursor.consume_doc_comment_run(false);
            let item_start = doc
                .as_ref()
                .map(|(_, span)| *span)
                .unwrap_or_else(|| cursor.peek_span());
            cursor.set_last_span(item_start);
            let target_depth = cursor.depth();
            match self.parse_sheet_item(cursor) {
                Ok(mut item) => {
                    if let Some((text, doc_span)) = &doc {
                        item.set_doc_comment(text.clone(), *doc_span);
                    }
                    items.push(item);
                }
                Err(e) => {
                    errors.push(e);
                    let recovery_fallback = cursor.last_span();
                    let item_end = cursor.skip_to_recovery_point(target_depth, recovery_fallback);
                    items.push(ast::SheetItem::Error {
                        span: ast::ExprSpan {
                            start: item_start,
                            end: item_end,
                        },
                        leading_comment: None,
                        doc_comment: doc.map(|(text, _)| text),
                        blank_line_before: false,
                    });
                }
            }
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::Sheet {
            name,
            name_span: point(name_span),
            items,
            leading_comment: None,
            doc_comment: None,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: point(open_span),
            span: ast::ExprSpan {
                start: sheet_start,
                end: close_span,
            },
            errors,
        })
    }
```

Change `parse_relationship_decl` similarly (capture `open_span`, add the three new fields):

```rust
    fn parse_relationship_decl(
        &mut self,
        cursor: &mut TokenCursor,
    ) -> Result<ast::RelationshipDecl> {
        use cel_parser::lex_lexer::Token;
        let decl_start = cursor.peek_span();
        cursor.is_keyword("relationship");
        let name = if matches!(cursor.peek_token(), Some(Token::Identifier(_))) {
            let (n, s) = cursor.consume_ident()?;
            Some((n, point(s)))
        } else {
            None
        };
        let open_span = cursor.expect_open_brace()?;
        let mut methods = Vec::new();
        while !cursor.at_close_brace() {
            methods.push(self.parse_method_decl(cursor)?);
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::RelationshipDecl {
            name,
            methods,
            leading_comment: None,
            doc_comment: None,
            blank_line_before: false,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: point(open_span),
            span: ast::ExprSpan {
                start: decl_start,
                end: close_span,
            },
        })
    }
```

Change `parse_conditional_decl` to capture each brace and build `DefaultBranch`:

```rust
    fn parse_conditional_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::ConditionalDecl> {
        use cel_parser::lex_lexer::Token;
        let decl_start = cursor.peek_span();
        cursor.is_keyword("conditional");
        let (match_name, match_span) = cursor.consume_ident()?;
        let outer_open = cursor.expect_open_brace()?;
        let mut branches = Vec::new();
        let mut default = None;
        while !cursor.at_close_brace() {
            if matches!(cursor.peek_token(), Some(Token::Identifier(id)) if id == "_") {
                let underscore_span = cursor.peek_span();
                cursor.advance();
                cursor.expect_punct("=>")?;
                let branch_open = cursor.expect_open_brace()?;
                let relationships = self.parse_branch_relationships(cursor)?;
                let close = cursor.expect_close_brace()?;
                cursor.consume_punct(",");
                default = Some(ast::DefaultBranch {
                    relationships,
                    trailing_comment: None,
                    blank_line_before_close: false,
                    open_brace_span: point(branch_open),
                    span: ast::ExprSpan {
                        start: underscore_span,
                        end: close,
                    },
                });
                break; // default branch is always last
            }
            let (lit, lit_span) = cursor.consume_literal()?;
            cursor.expect_punct("=>")?;
            let branch_open = cursor.expect_open_brace()?;
            let relationships = self.parse_branch_relationships(cursor)?;
            let close = cursor.expect_close_brace()?;
            cursor.consume_punct(",");
            branches.push(ast::ConditionalBranch {
                literal: lit,
                literal_span: point(lit_span),
                relationships,
                leading_comment: None,
                blank_line_before: false,
                trailing_comment: None,
                blank_line_before_close: false,
                open_brace_span: point(branch_open),
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
            doc_comment: None,
            blank_line_before: false,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: point(outer_open),
            span: ast::ExprSpan {
                start: decl_start,
                end: close_span,
            },
        })
    }
```

Change `parse_out_decl` to add the two new fields (no `open_brace_span`):

```rust
    fn parse_out_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::OutDecl> {
        let decl_start = cursor.peek_span();
        cursor.is_keyword("out");
        let (name, name_span) = cursor.consume_ident()?;
        let type_name = if cursor.consume_punct(":") {
            Some(self.parse_type_expr(cursor)?)
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
            doc_comment: None,
            blank_line_before: false,
            trailing_comment: None,
            blank_line_before_close: false,
            span: ast::ExprSpan {
                start: decl_start,
                end: close_span,
            },
        })
    }
```

- [ ] **Step 5: Add trailing-trivia recovery in `trivia.rs`**

Add a new trait (after the existing `TriviaTarget` trait and its impls):

```rust
/// A container whose own `{ ... }` block may carry trailing trivia — a comment or blank line
/// between its last child and its own closing `}` — recovered by [`attach_trailing`]. See
/// <https://github.com/stlab/cel-rs/issues/52>.
trait TrailingTriviaTarget {
    /// The span of this container's own opening `{`, used as the trailing gap's start when its
    /// child list is empty.
    fn open_brace_span(&self) -> proc_macro2::Span;
    /// The span of this container's own closing `}`.
    fn close_span(&self) -> proc_macro2::Span;
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment);
    fn set_blank_line_before_close(&mut self, value: bool);
}

impl TrailingTriviaTarget for Sheet {
    fn open_brace_span(&self) -> proc_macro2::Span {
        self.open_brace_span.end
    }
    fn close_span(&self) -> proc_macro2::Span {
        self.span.end
    }
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_comment = Some(comment);
    }
    fn set_blank_line_before_close(&mut self, value: bool) {
        self.blank_line_before_close = value;
    }
}

impl TrailingTriviaTarget for RelationshipDecl {
    fn open_brace_span(&self) -> proc_macro2::Span {
        self.open_brace_span.end
    }
    fn close_span(&self) -> proc_macro2::Span {
        self.span.end
    }
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_comment = Some(comment);
    }
    fn set_blank_line_before_close(&mut self, value: bool) {
        self.blank_line_before_close = value;
    }
}

impl TrailingTriviaTarget for ConditionalBranch {
    fn open_brace_span(&self) -> proc_macro2::Span {
        self.open_brace_span.end
    }
    fn close_span(&self) -> proc_macro2::Span {
        self.span.end
    }
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_comment = Some(comment);
    }
    fn set_blank_line_before_close(&mut self, value: bool) {
        self.blank_line_before_close = value;
    }
}

impl TrailingTriviaTarget for crate::ast::DefaultBranch {
    fn open_brace_span(&self) -> proc_macro2::Span {
        self.open_brace_span.end
    }
    fn close_span(&self) -> proc_macro2::Span {
        self.span.end
    }
    fn set_trailing_comment(&mut self, comment: crate::ast::Comment) {
        self.trailing_comment = Some(comment);
    }
    fn set_blank_line_before_close(&mut self, value: bool) {
        self.blank_line_before_close = value;
    }
}

/// Recovers trailing trivia (a comment/blank line between `items`' last element and
/// `container`'s own closing `}`, or between its opening `{` and closing `}` when `items` is
/// empty) and attaches it to `container`. See <https://github.com/stlab/cel-rs/issues/52>.
fn attach_trailing<T: TriviaTarget, C: TrailingTriviaTarget>(
    source: &str,
    line_starts: &[usize],
    items: &[T],
    container: &mut C,
) {
    let start_pos = match items.last() {
        Some(last) => last.span().end.end(),
        None => container.open_brace_span().end(),
    };
    let end_pos = container.close_span().start();
    let start = line_column_to_byte(source, line_starts, start_pos);
    let end = line_column_to_byte(source, line_starts, end_pos);
    if start < end {
        let gap_text = &source[start..end];
        let (comment, blank_line_before_close) = analyze_gap(gap_text);
        container.set_blank_line_before_close(blank_line_before_close);
        if let Some(comment) = comment {
            container.set_trailing_comment(comment);
        }
    }
}

/// Recovers `ConditionalDecl`'s own trailing trivia — the gap before its outer closing `}`,
/// after its default arm if present, else its last branch, else (an empty conditional) its own
/// opening `{`. Handled specially, like [`attach_out_trailing`], because a `ConditionalDecl`'s
/// "last child" isn't a single homogeneous list — it's whichever of `branches`/`default` came
/// last in declaration order.
fn attach_conditional_trailing(source: &str, line_starts: &[usize], cond: &mut ConditionalDecl) {
    let start_pos = if let Some(default) = &cond.default {
        default.span.end
    } else if let Some(last_branch) = cond.branches.last() {
        last_branch.span.end
    } else {
        cond.open_brace_span.end
    };
    let start = line_column_to_byte(source, line_starts, start_pos.end());
    let end = line_column_to_byte(source, line_starts, cond.span.end.start());
    if start < end {
        let gap_text = &source[start..end];
        let (comment, blank_line_before_close) = analyze_gap(gap_text);
        cond.blank_line_before_close = blank_line_before_close;
        if let Some(comment) = comment {
            cond.trailing_comment = Some(comment);
        }
    }
}

/// Recovers `OutDecl`'s own trailing trivia — the gap before its closing `}`, after its last
/// condition if any, else its mandatory writer method (an `OutDecl`'s block can never be
/// child-empty, since the writer is grammar-required).
fn attach_out_trailing(source: &str, line_starts: &[usize], out_decl: &mut OutDecl) {
    let start_pos = match out_decl.conditions.last() {
        Some(last) => last.span.end,
        None => out_decl.writer.span.end,
    };
    let start = line_column_to_byte(source, line_starts, start_pos.end());
    let end = line_column_to_byte(source, line_starts, out_decl.span.end.start());
    if start < end {
        let gap_text = &source[start..end];
        let (comment, blank_line_before_close) = analyze_gap(gap_text);
        out_decl.blank_line_before_close = blank_line_before_close;
        if let Some(comment) = comment {
            out_decl.trailing_comment = Some(comment);
        }
    }
}
```

Now wire these into the existing attach functions. `attach_trivia` gains one call after its
existing `attach_gaps(source, &line_starts, &mut sheet.items);`:

```rust
    attach_gaps(source, &line_starts, &mut sheet.items);
    attach_trailing(source, &line_starts, &sheet.items, sheet);
```

`attach_relationship` becomes:

```rust
fn attach_relationship(source: &str, line_starts: &[usize], rel: &mut RelationshipDecl) {
    attach_gaps(source, line_starts, &mut rel.methods);
    attach_trailing(source, line_starts, &rel.methods, rel);
}
```

`attach_conditional` becomes:

```rust
fn attach_conditional(source: &str, line_starts: &[usize], cond: &mut ConditionalDecl) {
    attach_gaps(source, line_starts, &mut cond.branches);
    for branch in &mut cond.branches {
        attach_gaps(source, line_starts, &mut branch.relationships);
        attach_trailing(source, line_starts, &branch.relationships, branch);
        for rel in &mut branch.relationships {
            attach_relationship(source, line_starts, rel);
        }
    }
    if let Some(default) = &mut cond.default {
        attach_gaps(source, line_starts, &mut default.relationships);
        attach_trailing(source, line_starts, &default.relationships, default);
        for rel in default.relationships.iter_mut() {
            attach_relationship(source, line_starts, rel);
        }
    }
    attach_conditional_trailing(source, line_starts, cond);
}
```

`attach_out` becomes:

```rust
fn attach_out(source: &str, line_starts: &[usize], out_decl: &mut OutDecl) {
    if !out_decl.conditions.is_empty() {
        let start = line_column_to_byte(source, line_starts, out_decl.writer.span.end.end());
        let end = line_column_to_byte(
            source,
            line_starts,
            out_decl.conditions[0].span.start.start(),
        );
        if start < end {
            let gap_text = &source[start..end];
            let (comment, blank_line_before) = analyze_gap(gap_text);
            out_decl.conditions[0].set_blank_line_before(blank_line_before);
            if let Some(comment) = comment {
                out_decl.conditions[0].set_leading_comment(comment);
            }
        }
    }
    attach_gaps(source, line_starts, &mut out_decl.conditions);
    attach_out_trailing(source, line_starts, out_decl);
}
```

Also add `use crate::ast::{ConditionDecl, ConditionalBranch, ConditionalDecl, ExprSpan, MethodDecl, OutDecl, RelationshipDecl, Sheet};`
at the top of the file already imports `Sheet`/`RelationshipDecl`/`ConditionalDecl`/
`ConditionalBranch`/`OutDecl` — no import changes are needed beyond what Task 3/existing code
already brought in, since `DefaultBranch` is referenced fully-qualified as `crate::ast::DefaultBranch`
above.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p adam-lang --lib trivia::`
Expected: all pass.

- [ ] **Step 7: Fix the two other places that read `ConditionalDecl::default` as a bare `Vec`**

Changing `default`'s type breaks two things outside `trivia.rs`/`ast_parser.rs` that this task
must also fix so the crate keeps compiling (Task 7 will later extend the `fmt.rs` line below to
be trailing-trivia-aware, but the crate must compile at the end of *this* task too).

In `adam-lang/src/fmt.rs`'s `write_conditional`, change:

```rust
    if let Some(default) = &cond.default {
        out.push_str(&indent(depth + 1));
        out.push_str("_ => ");
        write_branch_relationships(out, default, depth + 1);
    }
```

to:

```rust
    if let Some(default) = &cond.default {
        out.push_str(&indent(depth + 1));
        out.push_str("_ => ");
        write_branch_relationships(out, &default.relationships, depth + 1);
    }
```

In `adam-lang/src/trivia.rs`'s test module, the pre-existing
`attaches_a_comment_to_a_relationship_nested_inside_the_default_branch` test indexes `default[1]`
directly (valid when `default` was a bare `Vec<RelationshipDecl>`). Update it to go through the
new `.relationships` field:

```rust
    #[test]
    fn attaches_a_comment_to_a_relationship_nested_inside_the_default_branch() {
        let source = "sheet s {\n    conditional m {\n        _ => {\n            relationship { method [a] -> [b] { a } }\n            // second\n            relationship { method [b] -> [a] { b } }\n        }\n    }\n}";
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        attach_trivia(source, &mut sheet);
        let crate::ast::SheetItem::Conditional(cond) = &sheet.items[0] else {
            panic!("expected Conditional");
        };
        let default = cond.default.as_ref().expect("default branch present");
        assert_eq!(
            default.relationships[1].leading_comment,
            Some(crate::ast::Comment::Line("second".to_string()))
        );
    }
```

- [ ] **Step 8: Run the full `adam-lang` suite and lints**

Run: `cargo test -p adam-lang` and `cargo clippy -p adam-lang --all-targets -- -D warnings`
Expected: all pass, clean.

- [ ] **Step 9: Commit**

```bash
cargo fmt --all
git add adam-lang/src/ast.rs adam-lang/src/ast_parser.rs adam-lang/src/trivia.rs adam-lang/src/fmt.rs
git commit -m "$(cat <<'EOF'
fix(adam-lang): recover trailing comment/blank line before a block's } (#52)

Sheet, RelationshipDecl, ConditionalDecl (and its new DefaultBranch
arm), ConditionalBranch, and OutDecl all gain a trailing-trivia slot
for the gap between their last child and their own closing brace --
previously dropped silently since nothing followed it to attach to.
Also covers a comment inside a completely empty block.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: `adam-lang::fmt` — re-emit trailing trivia (#52 visible in the formatter)

**Files:**
- Modify: `adam-lang/src/fmt.rs` (add `write_trailing_trivia`; change `write_branch_relationships`'s
  signature; update `write_relationship`, `write_branch`, `write_conditional`, `write_out`,
  `format_sheet`)

**Interfaces:**
- Consumes: `trailing_comment`/`blank_line_before_close` fields and `write_comment` from Task 6/
  Task 2; `DefaultBranch` from Task 6.
- Produces: nothing new for later tasks — this is the last piece of the comment-support work.

- [ ] **Step 1: Write the failing tests**

In `adam-lang/src/fmt.rs`'s test module, add:

```rust
    #[test]
    fn formats_a_trailing_comment_before_the_sheets_closing_brace() {
        assert_eq!(
            format("sheet s {\n    cell x: i32 = 1;\n    // trailing\n}"),
            "sheet s {\n    cell x: i32 = 1;\n    // trailing\n}\n"
        );
    }

    #[test]
    fn formats_a_trailing_comment_in_an_empty_relationship() {
        assert_eq!(
            format("sheet s {\n    relationship {\n        // only this\n    }\n}"),
            "sheet s {\n    relationship {\n        // only this\n    }\n}\n"
        );
    }

    #[test]
    fn formats_a_trailing_comment_before_a_relationships_closing_brace() {
        let source = "sheet s {\n    relationship {\n        method [a] -> [b] { a }\n        // trailing\n    }\n}";
        let expected = "sheet s {\n    relationship {\n        method [a] -> [b] { a }\n        // trailing\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_trailing_comment_before_a_conditionals_own_closing_brace() {
        let source = "sheet s {\n    conditional m {\n        0i32 => { relationship { method [a] -> [b] { a } } }\n        // trailing\n    }\n}";
        let expected = "sheet s {\n    conditional m {\n        0i32 => {\n            relationship {\n                method [a] -> [b] { a }\n            }\n        }\n        // trailing\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_trailing_comment_in_a_default_arm() {
        let source = "sheet s {\n    conditional m {\n        _ => {\n            relationship { method [a] -> [b] { a } }\n            // trailing\n        }\n    }\n}";
        let expected = "sheet s {\n    conditional m {\n        _ => {\n            relationship {\n                method [a] -> [b] { a }\n            }\n            // trailing\n        }\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn formats_a_trailing_comment_before_an_outs_closing_brace() {
        let source = "sheet s {\n    out area: f64 {\n        method [w] { w }\n        // trailing\n    }\n}";
        let expected = "sheet s {\n    out area: f64 {\n        method [w] { w }\n        // trailing\n    }\n}\n";
        assert_eq!(format(source), expected);
    }

    #[test]
    fn trailing_trivia_formatting_is_idempotent_through_a_reparse() {
        let source = "sheet s {\n    cell a: i32 = 1;\n    // trailing\n}";
        let once = format(source);
        let twice = format(&once);
        assert_eq!(once, twice);
    }
```

Also update the existing `formats_a_conditional_with_branches_and_a_default_and_no_trailing_commas`
test: its `source`/`expected` strings are unaffected by this task (no trailing comment in that
fixture), but it exercises `cond.default` — verify it still compiles once `default` is a
`DefaultBranch`; no test-string changes are needed here since the field-access change is purely
internal to `write_conditional`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-lang --lib fmt:: -- --nocapture`
Expected: fail (no trailing trivia is emitted yet), or a compile error if Task 6's Step 7 note
about `cond.default` wasn't yet applied — apply Step 3 below in that case.

- [ ] **Step 3: Add `write_trailing_trivia` and wire it into every writer**

Add this function to `adam-lang/src/fmt.rs` (after `write_comment`):

```rust
/// Emits a container's trailing comment (honoring `blank_line_before_close`) immediately before
/// its closing `}` is written, reusing [`write_comment`] so block-vs-line style is preserved
/// exactly as for leading comments. See <https://github.com/stlab/cel-rs/issues/52>.
fn write_trailing_trivia(
    out: &mut String,
    blank_line_before_close: bool,
    trailing_comment: Option<&ast::Comment>,
    depth: usize,
) {
    if blank_line_before_close {
        out.push('\n');
    }
    if let Some(comment) = trailing_comment {
        write_comment(out, comment, depth);
    }
}
```

Change `write_relationship` to emit its trailing trivia before the closing brace:

```rust
fn write_relationship(out: &mut String, rel: &ast::RelationshipDecl, depth: usize) {
    write_trivia(
        out,
        rel.blank_line_before,
        rel.leading_comment.as_ref(),
        depth,
    );
    write_doc_comment(out, "///", rel.doc_comment.as_deref(), depth);
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
    write_trailing_trivia(
        out,
        rel.blank_line_before_close,
        rel.trailing_comment.as_ref(),
        depth + 1,
    );
    out.push_str(&indent(depth));
    out.push_str("}\n");
}
```

Change `write_branch_relationships`'s signature to accept the trailing-trivia fields (it's shared
by both a named branch and the default arm, whose trailing trivia differ per call):

```rust
fn write_branch_relationships(
    out: &mut String,
    relationships: &[ast::RelationshipDecl],
    trailing_comment: Option<&ast::Comment>,
    blank_line_before_close: bool,
    depth: usize,
) {
    out.push_str("{\n");
    for rel in relationships {
        write_relationship(out, rel, depth + 1);
    }
    write_trailing_trivia(out, blank_line_before_close, trailing_comment, depth + 1);
    out.push_str(&indent(depth));
    out.push_str("}\n");
}
```

Change `write_branch` to pass those through:

```rust
fn write_branch(out: &mut String, branch: &ast::ConditionalBranch, depth: usize) {
    write_trivia(
        out,
        branch.blank_line_before,
        branch.leading_comment.as_ref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str(&source_text_or_empty(branch.literal_span));
    out.push_str(" => ");
    write_branch_relationships(
        out,
        &branch.relationships,
        branch.trailing_comment.as_ref(),
        branch.blank_line_before_close,
        depth,
    );
}
```

Change `write_conditional` to read `default.relationships` (now a `DefaultBranch`, not a bare
`Vec`), pass its trailing trivia through, and emit its own trailing trivia before its closing
brace:

```rust
fn write_conditional(out: &mut String, cond: &ast::ConditionalDecl, depth: usize) {
    write_trivia(
        out,
        cond.blank_line_before,
        cond.leading_comment.as_ref(),
        depth,
    );
    write_doc_comment(out, "///", cond.doc_comment.as_deref(), depth);
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
        write_branch_relationships(
            out,
            &default.relationships,
            default.trailing_comment.as_ref(),
            default.blank_line_before_close,
            depth + 1,
        );
    }
    write_trailing_trivia(
        out,
        cond.blank_line_before_close,
        cond.trailing_comment.as_ref(),
        depth + 1,
    );
    out.push_str(&indent(depth));
    out.push_str("}\n");
}
```

Change `write_out` to emit its trailing trivia before its closing brace:

```rust
fn write_out(out: &mut String, decl: &ast::OutDecl, depth: usize) {
    write_trivia(
        out,
        decl.blank_line_before,
        decl.leading_comment.as_ref(),
        depth,
    );
    write_doc_comment(out, "///", decl.doc_comment.as_deref(), depth);
    out.push_str(&indent(depth));
    out.push_str("out ");
    out.push_str(&decl.name);
    if let Some(type_expr) = &decl.type_name {
        out.push_str(": ");
        out.push_str(&source_text_or_empty(type_expr.span()));
    }
    out.push_str(" {\n");
    write_out_method(out, &decl.writer, depth + 1);
    for cond in &decl.conditions {
        write_condition(out, cond, depth + 1);
    }
    write_trailing_trivia(
        out,
        decl.blank_line_before_close,
        decl.trailing_comment.as_ref(),
        depth + 1,
    );
    out.push_str(&indent(depth));
    out.push_str("}\n");
}
```

Change `format_sheet` to emit the sheet's own trailing trivia before its closing brace:

```rust
pub fn format_sheet(sheet: &ast::Sheet) -> String {
    debug_assert!(
        sheet.errors.is_empty(),
        "format_sheet's precondition: no recorded syntax errors"
    );
    let mut out = String::new();
    write_trivia(&mut out, false, sheet.leading_comment.as_ref(), 0);
    write_doc_comment(&mut out, "//!", sheet.doc_comment.as_deref(), 0);
    out.push_str(&format!("sheet {} {{\n", sheet.name));
    for item in &sheet.items {
        write_sheet_item(&mut out, item, 1);
    }
    write_trailing_trivia(
        &mut out,
        sheet.blank_line_before_close,
        sheet.trailing_comment.as_ref(),
        1,
    );
    out.push_str("}\n");
    out
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p adam-lang --lib fmt::`
Expected: all pass.

- [ ] **Step 5: Run the full `adam-lang` suite and lints**

Run: `cargo test -p adam-lang` and `cargo clippy -p adam-lang --all-targets -- -D warnings`
Expected: all pass, clean.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add adam-lang/src/fmt.rs
git commit -m "$(cat <<'EOF'
fix(adam-lang): re-emit trailing comment/blank line before a block's } (#52)

Completes #52: format_sheet now reproduces a trailing comment/blank
line recovered in Task 6 immediately before every container's closing
brace (sheet, relationship, conditional, its default arm, each branch,
and out), instead of silently dropping it.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: Full workspace verification

**Files:** none (verification only).

**Interfaces:** none.

- [ ] **Step 1: Format**

Run: `cargo fmt --all`
Expected: no changes (everything already formatted from each task's commit step).

- [ ] **Step 2: Build the whole workspace**

Run: `cargo build --workspace`
Expected: zero warnings.

- [ ] **Step 3: Test the whole workspace, including doc tests**

Run: `cargo test --workspace` then `cargo test --doc --workspace`
Expected: all pass, zero warnings.

- [ ] **Step 4: Lint the whole workspace**

Run each of:
```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: all three clean.

- [ ] **Step 5: Spot-check the issue repros end to end**

Run a quick manual check (not a committed test) that the four original issue repros now behave
as intended — e.g. in a scratch `fn main` or `cargo test -p adam-lang -- --nocapture` against the
exact snippets quoted in issues #105, #53, #58, and #52 — confirming each round-trips through
`format_sheet` without dropping content. This step produces no diff; it's a final human-readable
confidence check before considering the branch ready for `finishing-a-development-branch`.

- [ ] **Step 6: No commit for this task**

This task only verifies; if any step surfaces a problem, fix it as part of the task that
introduced it (amend that task's own commit's follow-up, or add a small new commit), then re-run
Steps 1-4 from the top.
