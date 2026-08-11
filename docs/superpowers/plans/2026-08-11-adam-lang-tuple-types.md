# adam-lang Tuple Type Syntax Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add adam-lang grammar/`TypeRegistry`/parser support for declaring a cell with a CEL
tuple type, explicitly (`cell a: (i32, (f64, String));`) or deduced from an initializer
(`cell a = (1, 2.5);`), fully interoperable with `method`/`out`/`condition` bodies as both an
input (including `.0`/`.1` field access) and an output — unifying today's ad hoc multi-output
splitting with the new mechanism.

**Architecture:** `TypeShape` (a new recursive enum) replaces flat `TypeId` as the identity of a
declared cell type wherever tuples can now appear. `TypeExpr`/an `Expr`-typed initializer replace
`CellDecl`/`OutDecl`'s flat `(String, ExprSpan)` type annotation and `(Literal, ExprSpan)`
initializer. `TypeRegistry` resolves a `TypeExpr` to a `TypeShape` and, recursively, builds the
runtime descriptors (`AssociatedType` "prototypes," per-leaf `Clone`/`PartialEq`/`Drop` function
pointers) the new `cel-runtime` primitives need. The real parser (`parser.rs`) uses these to
construct/consume tuple-typed cells; the existing multi-output-method splitting is generalized
into the same "produce a tuple, then destructure by declared shape" mechanism a single
tuple-typed output now uses.

**Tech Stack:** Rust, `adam-lang` (`cel_parser`, `cel_runtime`, `adam_rs`).

**Reference:** `docs/superpowers/specs/2026-08-11-adam-lang-tuple-types-design.md`. Depends on
`docs/superpowers/plans/2026-08-11-cel-runtime-dynamic-tuple-primitives.md` being complete first
(every task below calls a primitive that plan adds: `DynSegment::call_dyn_as_dynamic_sequence`,
`DynSegment::push_arg_as_dynamic_sequence_tuple`, `DynamicSequence::from_dyn_elements`,
`DynElementSpec`, `layout_associated`, `drop_tuple` (now `pub`), `element_dropper_for`,
`element_cloner_for`, `element_eq_for`, `element_writer_for`, `raw_dropper_for`).

## Global Constraints

- Format with `cargo fmt --all` before every commit (enforced by pre-commit hook).
- Every function/trait/struct needs a contract-style `///` doc comment (Summary, Preconditions as
  `debug_assert!`, `# Errors`/`# Safety` where applicable, Postconditions, Complexity if not O(1))
  per the root `CLAUDE.md`.
- For parser functions, the grammar production is the summary (e.g. `` /// `type_expr = ...`. ``).
- Unit tests are derived from contract/public interface only — never from implementation
  internals.
- Run `cargo test -p adam-lang` after every task's implementation step; run
  `cargo test --workspace`, `cargo test --doc --workspace`, and all three `cargo clippy`
  invocations from the root `CLAUDE.md` before the final commit of the whole plan (Task 10).
- Every existing test in `adam-lang/src/{ast,ast_parser,parser,typecheck,fmt}.rs` must keep
  passing unchanged throughout — every task here is additive/generalizing, not behavior-changing
  for non-tuple cells.

---

### Task 1: AST — `TypeExpr` and `Expr`-typed initializers

**Files:**
- Modify: `adam-lang/src/ast.rs`

**Interfaces:**
- Produces: `pub enum TypeExpr { Named(String, ExprSpan), Tuple(Vec<TypeExpr>, ExprSpan) }` with a
  `pub fn span(&self) -> ExprSpan` method; `CellDecl.type_name: Option<TypeExpr>` (was
  `Option<(String, ExprSpan)>`); `CellDecl.initializer: Option<cel_parser::Expr>` (was
  `Option<(Literal, ExprSpan)>`); `OutDecl.type_name: Option<TypeExpr>` (was
  `Option<(String, ExprSpan)>`).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `adam-lang/src/ast.rs`:

```rust
#[test]
fn type_expr_named_span_is_its_own_span() {
    let span = point(Span::call_site());
    let expr = TypeExpr::Named("i32".to_string(), span);
    assert_eq!(format!("{:?}", expr.span()), format!("{span:?}"));
}

#[test]
fn type_expr_tuple_span_is_the_whole_parenthesized_span() {
    let span = point(Span::call_site());
    let expr = TypeExpr::Tuple(Vec::new(), span);
    assert_eq!(format!("{:?}", expr.span()), format!("{span:?}"));
}

#[test]
fn cell_decl_type_name_holds_a_nested_tuple_type_expr() {
    let span = point(Span::call_site());
    let cell = CellDecl {
        name: "a".to_string(),
        name_span: span,
        type_name: Some(TypeExpr::Tuple(
            vec![
                TypeExpr::Named("i32".to_string(), span),
                TypeExpr::Named("f64".to_string(), span),
            ],
            span,
        )),
        initializer: None,
        leading_comment: None,
        blank_line_before: false,
        span,
    };
    match cell.type_name {
        Some(TypeExpr::Tuple(elements, _)) => assert_eq!(elements.len(), 2),
        other => panic!("expected Tuple, got {other:?}"),
    }
}

#[test]
fn cell_decl_initializer_holds_a_parsed_expr() {
    let span = point(Span::call_site());
    let cell = CellDecl {
        name: "a".to_string(),
        name_span: span,
        type_name: None,
        initializer: Some(cel_parser::Expr::Ident {
            name: "x".to_string(),
            span,
        }),
        leading_comment: None,
        blank_line_before: false,
        span,
    };
    assert!(matches!(cell.initializer, Some(cel_parser::Expr::Ident { .. })));
}
```

Every existing test in this file that constructs a `CellDecl`/`SheetItem::Cell`/`SheetItem::Out`
literal (`sheet_item_span_reads_the_cell_variant`, `set_leading_comment_sets_the_cell_variant`,
`set_blank_line_before_sets_the_cell_variant`, and the `Out` variants) currently writes `type_name:
None, initializer: None,` — these already compile against `Option<TypeExpr>`/`Option<Expr>`
without any change, since `None` is valid for any `Option<T>`. No edits needed to those tests.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang ast::tests::type_expr ast::tests::cell_decl`
Expected: FAIL to compile — `TypeExpr` doesn't exist, `CellDecl.type_name`/`.initializer` are
still the old flat types.

- [ ] **Step 3: Change the AST types**

In `adam-lang/src/ast.rs`, add right before `CellDecl`:

```rust
/// `type_expr = identifier | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".`
///
/// `()` is the empty tuple type (0 elements); `(T)` is grouping (same as bare `T` — types have
/// no precedence to disambiguate, but staying symmetric with `cel_parser`'s expression grammar
/// costs nothing); `(T,)` is a 1-element tuple; `(T, U, ...)` is n-element, no trailing comma.
#[derive(Debug, Clone)]
pub enum TypeExpr {
    /// A single type name, resolved later against a `TypeRegistry`.
    Named(String, ExprSpan),
    /// A tuple type, recursively — `Vec::new()` for `()`.
    Tuple(Vec<TypeExpr>, ExprSpan),
}

impl TypeExpr {
    /// Returns this type expression's source span.
    pub fn span(&self) -> ExprSpan {
        match self {
            TypeExpr::Named(_, span) | TypeExpr::Tuple(_, span) => *span,
        }
    }
}
```

Change `CellDecl`'s fields:

```rust
    /// The `: type_expr` annotation, if present.
    pub type_name: Option<TypeExpr>,
    /// The `= or_expression` initializer, if present. Unresolved and unevaluated here — see
    /// `crate::parser::AdamParser` for the compile-to-`Sheet` phase, which parses this with no
    /// cell scope pushed and evaluates it eagerly, once, at parse time.
    pub initializer: Option<cel_parser::Expr>,
```

Change `OutDecl`'s `type_name` field the same way, and update its doc comment's cross-reference to
say "`TypeExpr`" instead of "unresolved type name."

Remove the now-unused `use cel_parser::lex_lexer::Literal;` import if nothing else in the file
uses `Literal` (check with `cargo build -p adam-lang` after this step — `ConditionalBranch.literal`
still uses it, so the import likely stays; confirm rather than assume).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-lang ast::`
Expected: PASS (all tests in the module, including the 4 new ones). This will not yet compile the
rest of the crate (`ast_parser.rs`, `parser.rs`, `typecheck.rs`, `fmt.rs` all construct/consume the
old field shapes) — Tasks 2–7 fix each in turn. Use `cargo check -p adam-lang --lib --tests
2>&1 | head -50` to confirm the *only* remaining errors are in those other files, not in `ast.rs`
itself, before moving on.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-lang/src/ast.rs
git commit -m "feat(adam-lang): add TypeExpr and switch CellDecl/OutDecl to it; initializer becomes an Expr"
```

---

### Task 2: `TokenCursor` paren support + AST parser's `type_expr`/initializer parsing

**Files:**
- Modify: `adam-lang/src/token_cursor.rs`
- Modify: `adam-lang/src/ast_parser.rs`

**Interfaces:**
- Produces (in `token_cursor.rs`): `pub(crate) fn expect_open_paren(&mut self) -> Result<Span>`,
  `pub(crate) fn at_close_paren(&mut self) -> bool`, `pub(crate) fn expect_close_paren(&mut self)
  -> Result<Span>` — mirroring `expect_open_brace`/`expect_close_brace` exactly, including
  `depth` tracking.
- Produces (in `ast_parser.rs`): `AdamAstParser::parse_type_expr(&mut self, cursor: &mut
  TokenCursor) -> Result<ast::TypeExpr>`; `parse_cell_decl` and `parse_out_decl` updated to call
  it and to parse the initializer via `self.parse_cel_or_expression(cursor)` (the existing method
  bodies already use).

Unlike `brace`/`bracket`, adam-lang's grammar has never used parens before — they were previously
exclusively CEL's own internal territory (tuple/group literals inside method bodies, consumed by
the embedded `cel_parser::Parser` directly against the raw stream, never passing through
`TokenCursor`). `type_expr` is the first *adam-lang-grammar-level* use of parens, so they must be
tracked in `TokenCursor::depth` (for `skip_to_recovery_point` to stay correct on a malformed
`type_expr`) exactly like brace/bracket already are. This does not change how CEL's *own* internal
parens behave (those still go through `take_tokens`/`set_tokens` untouched, never touching
`depth`) — only type_expr's own, adam-lang-owned parens are newly tracked.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `adam-lang/src/ast_parser.rs`:

```rust
#[test]
fn parse_cell_with_explicit_tuple_type() {
    let sheet = AdamAstParser::new()
        .parse_str("sheet s { cell a: (i32, f64); }")
        .unwrap();
    let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
        panic!("expected Cell");
    };
    match cell.type_name.as_ref().unwrap() {
        ast::TypeExpr::Tuple(elements, _) => {
            assert_eq!(elements.len(), 2);
            assert!(matches!(&elements[0], ast::TypeExpr::Named(n, _) if n == "i32"));
            assert!(matches!(&elements[1], ast::TypeExpr::Named(n, _) if n == "f64"));
        }
        other => panic!("expected Tuple, got {other:?}"),
    }
}

#[test]
fn parse_cell_with_nested_tuple_type() {
    let sheet = AdamAstParser::new()
        .parse_str("sheet s { cell a: (i32, (f64, String)); }")
        .unwrap();
    let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
        panic!("expected Cell");
    };
    let ast::TypeExpr::Tuple(elements, _) = cell.type_name.as_ref().unwrap() else {
        panic!("expected top-level Tuple");
    };
    assert_eq!(elements.len(), 2);
    assert!(matches!(&elements[0], ast::TypeExpr::Named(n, _) if n == "i32"));
    match &elements[1] {
        ast::TypeExpr::Tuple(inner, _) => assert_eq!(inner.len(), 2),
        other => panic!("expected nested Tuple, got {other:?}"),
    }
}

#[test]
fn parse_cell_with_empty_tuple_type() {
    let sheet = AdamAstParser::new().parse_str("sheet s { cell a: (); }").unwrap();
    let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
        panic!("expected Cell");
    };
    match cell.type_name.as_ref().unwrap() {
        ast::TypeExpr::Tuple(elements, _) => assert!(elements.is_empty()),
        other => panic!("expected empty Tuple, got {other:?}"),
    }
}

#[test]
fn parse_cell_with_parenthesized_type_is_grouping_not_a_1_tuple() {
    let sheet = AdamAstParser::new().parse_str("sheet s { cell a: (i32); }").unwrap();
    let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
        panic!("expected Cell");
    };
    assert!(matches!(cell.type_name.as_ref().unwrap(), ast::TypeExpr::Named(n, _) if n == "i32"));
}

#[test]
fn parse_cell_with_1_tuple_type_requires_trailing_comma() {
    let sheet = AdamAstParser::new().parse_str("sheet s { cell a: (i32,); }").unwrap();
    let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
        panic!("expected Cell");
    };
    match cell.type_name.as_ref().unwrap() {
        ast::TypeExpr::Tuple(elements, _) => assert_eq!(elements.len(), 1),
        other => panic!("expected 1-Tuple, got {other:?}"),
    }
}

#[test]
fn parse_cell_initializer_is_a_tuple_expr() {
    let sheet = AdamAstParser::new()
        .parse_str("sheet s { cell a = (1, 2.5); }")
        .unwrap();
    let ast::SheetItem::Cell(cell) = &sheet.items[0] else {
        panic!("expected Cell");
    };
    assert!(matches!(cell.initializer, Some(Expr::Tuple { .. })));
}

#[test]
fn parse_out_with_explicit_tuple_type() {
    let sheet = AdamAstParser::new()
        .parse_str("sheet s { out a: (i32, f64) { method [x] { (x, x) } } }")
        .unwrap();
    let ast::SheetItem::Out(out) = &sheet.items[0] else {
        panic!("expected Out");
    };
    assert!(matches!(out.type_name.as_ref().unwrap(), ast::TypeExpr::Tuple(elements, _) if elements.len() == 2));
}

#[test]
fn malformed_tuple_type_recovers_at_the_next_sheet_item() {
    let sheet = AdamAstParser::new()
        .parse_str("sheet s { cell good_before: i32 = 1; cell bad: (i32, ; cell good_after: i32 = 2; }")
        .unwrap();
    assert_eq!(sheet.errors.len(), 1);
    assert_eq!(sheet.items.len(), 3);
    assert!(matches!(sheet.items[0], ast::SheetItem::Cell(_)));
    assert!(matches!(sheet.items[1], ast::SheetItem::Error { .. }));
    assert!(matches!(sheet.items[2], ast::SheetItem::Cell(_)));
}
```

Add to the `tests` module in `adam-lang/src/token_cursor.rs` (create one if none exists yet —
check the file first):

```rust
#[test]
fn expect_open_paren_increments_depth() {
    let stream = proc_macro2::TokenStream::from_str("( )").unwrap();
    let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
    assert_eq!(cursor.depth(), 0);
    cursor.expect_open_paren().unwrap();
    assert_eq!(cursor.depth(), 1);
}

#[test]
fn expect_close_paren_decrements_depth() {
    let stream = proc_macro2::TokenStream::from_str("( )").unwrap();
    let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
    cursor.expect_open_paren().unwrap();
    cursor.expect_close_paren().unwrap();
    assert_eq!(cursor.depth(), 0);
}

#[test]
fn at_close_paren_is_true_at_a_close_paren_or_end_of_input() {
    let stream = proc_macro2::TokenStream::from_str(")").unwrap();
    let mut cursor = TokenCursor::new(LexLexer::new(stream.into_iter()).peekable());
    assert!(cursor.at_close_paren());
}
```
(Adjust imports at the top of the test to match whatever `use` statements the surrounding module
already needs — mirror `expect_open_brace`'s own existing tests if any exist in this file, or the
pattern used by `ast_parser.rs`'s tests otherwise.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang ast_parser::tests::parse_cell_with_explicit_tuple_type
token_cursor::tests::expect_open_paren_increments_depth`
Expected: FAIL to compile — `parse_type_expr`/`expect_open_paren` don't exist yet.

- [ ] **Step 3: Add `TokenCursor` paren support**

In `adam-lang/src/token_cursor.rs`, add right after `expect_close_bracket`:

```rust
    /// Consumes `(`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not `(`.
    ///
    /// - Postcondition: on success, increments [`Self::depth`] by 1.
    pub(crate) fn expect_open_paren(&mut self) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::OpenDelim {
                delimiter: Delimiter::Parenthesis,
                span,
            }) => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            self.depth += 1;
            Ok(span)
        } else {
            Err(ParseError::new("expected `(`", span))
        }
    }

    /// Returns whether the next token is `)` (or the stream is exhausted).
    pub(crate) fn at_close_paren(&mut self) -> bool {
        matches!(
            self.tokens.as_mut().and_then(|t| t.peek()),
            Some(Token::CloseDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            }) | None
        )
    }

    /// Consumes `)`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the next token is not `)`.
    ///
    /// - Postcondition: on success, decrements [`Self::depth`] by 1.
    pub(crate) fn expect_close_paren(&mut self) -> Result<Span> {
        let (ok, span) = match self.tokens.as_mut().and_then(|t| t.peek()) {
            Some(Token::CloseDelim {
                delimiter: Delimiter::Parenthesis,
                span,
            }) => (true, *span),
            other => (false, other.map(|t| t.span()).unwrap_or(Span::call_site())),
        };
        if ok {
            self.advance();
            self.depth -= 1;
            Ok(span)
        } else {
            Err(ParseError::new("expected `)`", span))
        }
    }
```

Update `skip_to_recovery_point`'s two `OpenDelim`/`CloseDelim` match arms to also cover
`Delimiter::Parenthesis`:

```rust
                Some(Token::CloseDelim {
                    delimiter: Delimiter::Brace | Delimiter::Bracket | Delimiter::Parenthesis,
                    ..
                }) if at_or_below_target => return last,
                Some(Token::CloseDelim {
                    delimiter: Delimiter::Brace | Delimiter::Bracket | Delimiter::Parenthesis,
                    ..
                }) => {
                    self.depth -= 1;
                    last = self.peek_span();
                    self.advance();
                }
                Some(Token::OpenDelim {
                    delimiter: Delimiter::Brace | Delimiter::Bracket | Delimiter::Parenthesis,
                    ..
                }) => {
                    self.depth += 1;
                    last = self.peek_span();
                    self.advance();
                }
```

Update `depth`'s own doc comment and the "Known limitation" section of `skip_to_recovery_point`'s
doc comment: replace every claim that "adam-lang's grammar never uses parens"/"CEL owns all
parens" with a note that `type_expr` is now the one adam-lang-grammar-level exception — those
parens *are* tracked via `expect_open_paren`/`expect_close_paren`, exactly like brace/bracket;
only CEL's own *internal* expression parens (consumed while the embedded `cel_parser::Parser`
temporarily owns the stream via `take_tokens`/`set_tokens`) remain untracked, unaffected by this
change.

- [ ] **Step 4: Add `parse_type_expr` and wire it + the `Expr`-typed initializer into
  `parse_cell_decl`/`parse_out_decl`**

Add to `adam-lang/src/ast_parser.rs`, in `impl AdamAstParser`, right after `parse_cell_decl`:

```rust
    /// `type_expr = identifier | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".`
    fn parse_type_expr(&mut self, cursor: &mut TokenCursor) -> Result<ast::TypeExpr> {
        use cel_parser::lex_lexer::Token;
        if matches!(cursor.peek_token(), Some(Token::Identifier(_))) {
            let (name, span) = cursor.consume_ident()?;
            return Ok(ast::TypeExpr::Named(name, point(span)));
        }

        let open_span = cursor.expect_open_paren()?;
        if cursor.at_close_paren() {
            let close_span = cursor.expect_close_paren()?;
            return Ok(ast::TypeExpr::Tuple(
                Vec::new(),
                ast::ExprSpan {
                    start: open_span,
                    end: close_span,
                },
            ));
        }

        let first = self.parse_type_expr(cursor)?;
        if cursor.at_close_paren() {
            // Grouping: exactly one type, no comma.
            cursor.expect_close_paren()?;
            return Ok(first);
        }
        if !cursor.consume_punct(",") {
            return Err(cursor.err_at("expected ',' or closing parenthesis"));
        }
        if cursor.at_close_paren() {
            // Single element + trailing comma: 1-tuple.
            let close_span = cursor.expect_close_paren()?;
            return Ok(ast::TypeExpr::Tuple(
                vec![first],
                ast::ExprSpan {
                    start: open_span,
                    end: close_span,
                },
            ));
        }
        let mut elements = vec![first];
        loop {
            elements.push(self.parse_type_expr(cursor)?);
            if cursor.at_close_paren() {
                break;
            }
            if !cursor.consume_punct(",") {
                return Err(cursor.err_at("expected ',' or closing parenthesis"));
            }
        }
        let close_span = cursor.expect_close_paren()?;
        Ok(ast::TypeExpr::Tuple(
            elements,
            ast::ExprSpan {
                start: open_span,
                end: close_span,
            },
        ))
    }
```

Update `parse_cell_decl` (the `":" type_name` / `"=" literal` branches): replace `let (type_name,
type_span) = cursor.consume_ident()?;` with `let type_name = self.parse_type_expr(cursor)?;`
(dropping the separate `type_span` — `type_name.span()` covers it), and replace both `let (lit,
lit_span) = cursor.consume_literal()?; Some((lit, point(lit_span)))`/`(None, Some((lit,
point(lit_span))))` initializer arms with `Some(self.parse_cel_or_expression(cursor)?)` (the same
method `parse_out_method`/`parse_condition_decl` already call for a `{ or_expression }` body —
here there's no surrounding `{ }`, so call it directly; it already hands the token stream to the
embedded CEL parser and reclaims it, exactly as needed).

Update `parse_out_decl`'s `[":" type_name]` branch the same way: `let type_name =
self.parse_type_expr(cursor)?;`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p adam-lang ast_parser:: token_cursor::`
Expected: PASS (all tests, including the 11 new ones). `cargo check -p adam-lang --lib --tests`
should now show remaining errors confined to `parser.rs`/`typecheck.rs`/`fmt.rs` only.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add adam-lang/src/token_cursor.rs adam-lang/src/ast_parser.rs
git commit -m "feat(adam-lang): parse recursive type_expr and or_expression initializers (AST path)"
```

---

### Task 3: `fmt.rs` — pretty-print `TypeExpr` and the `Expr`-typed initializer

**Files:**
- Modify: `adam-lang/src/fmt.rs`

**Interfaces:**
- Consumes: `ast::TypeExpr::span` (Task 1); `cel_parser::format_expr` (existing, already used for
  method bodies).

`write_cell`/`write_out` currently re-emit a cell/out's type via the *string* `type_name` and the
initializer via `source_text_or_empty` on the *literal's own span* — since both are now spans
belonging to structured nodes, this task switches the type annotation to the same
span-based-re-emit style already used elsewhere in this file (`type_expr.span()` instead of a
bare string), and the initializer to `cel_parser::format_expr` (matching how method/out/condition
bodies are already formatted — a normalization improvement over the old span-based re-emit, which
was only ever a stopgap for when there was no parsed `Expr` to format).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `adam-lang/src/fmt.rs`:

```rust
#[test]
fn formats_a_cell_with_an_explicit_tuple_type() {
    assert_eq!(
        format("sheet s { cell a: (i32, f64) = (1, 2.5); }"),
        "sheet s {\n    cell a: (i32, f64) = (1, 2.5);\n}\n"
    );
}

#[test]
fn formats_a_cell_with_a_nested_tuple_type() {
    assert_eq!(
        format("sheet s { cell a: (i32, (f64, String)); }"),
        "sheet s {\n    cell a: (i32, (f64, String));\n}\n"
    );
}

#[test]
fn formats_an_out_with_an_explicit_tuple_type() {
    assert_eq!(
        format("sheet s { out a: (i32, i32) { method [x] { (x, x) } } }"),
        "sheet s {\n    out a: (i32, i32) {\n        method [x] { (x, x) }\n    }\n}\n"
    );
}

#[test]
fn format_is_idempotent_through_a_reparse_with_a_tuple_cell() {
    let source = "sheet s {\n    cell a: (i32, f64) = (1, 2.5);\n}";
    let once = format(source);
    let twice = format(&once);
    assert_eq!(once, twice);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang fmt::tests::formats_a_cell_with_an_explicit_tuple_type`
Expected: FAIL to compile (this file still references the old `CellDecl`/`OutDecl` field shapes).

- [ ] **Step 3: Update `write_cell` and `write_out`**

In `adam-lang/src/fmt.rs`, replace `write_cell`'s type/initializer block:

```rust
    if let Some(type_expr) = &cell.type_name {
        out.push_str(": ");
        out.push_str(&source_text_or_empty(type_expr.span()));
    }
    if let Some(expr) = &cell.initializer {
        out.push_str(" = ");
        out.push_str(&cel_parser::format_expr(expr));
    }
```

Replace `write_out`'s type block:

```rust
    if let Some(type_expr) = &decl.type_name {
        out.push_str(": ");
        out.push_str(&source_text_or_empty(type_expr.span()));
    }
```

Update the module doc comment's line "method bodies/cell initializers delegated to
[`cel_parser::format_expr`] (bodies) or re-emitted via `Span::source_text()` directly
(initializers/branch-match literals ...)" to say cell initializers are now also delegated to
`format_expr`, alongside method/out/condition bodies — only branch-match literals still use the
span-re-emit path.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-lang fmt::`
Expected: PASS (all tests, including the 4 new ones — every pre-existing scalar-cell formatting
test must still pass unchanged, since `format_expr` on a bare literal `Expr` reproduces the same
text `source_text_or_empty` did).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-lang/src/fmt.rs
git commit -m "feat(adam-lang): format recursive TypeExpr and Expr-typed cell initializers"
```

---

### Task 4: `TypeRegistry` — `TypeShape`, `resolve`, `display_name`, new per-type descriptors

**Files:**
- Modify: `adam-lang/src/type_registry.rs`

**Interfaces:**
- Consumes: `cel_runtime::{raw_dropper_for, element_dropper_for, element_cloner_for,
  element_eq_for, element_writer_for}` (from the cel-runtime plan's Task 1).
- Produces: `pub enum TypeShape { Named(TypeId), Tuple(Vec<TypeShape>) }` (`Clone, PartialEq, Eq,
  Hash, Debug`); `TypeEntry` gains `pub size: usize`, `pub align: usize`, `pub raw_dropper:
  cel_runtime::RawDropper`, `pub element_drop: cel_runtime::ElementDropper`, `pub element_clone:
  cel_runtime::ElementCloner`, `pub element_eq: cel_runtime::ElementEq`, `pub element_write:
  unsafe fn(Box<dyn Any>, *mut u8)`; `TypeRegistry::resolve(&self, expr: &crate::ast::TypeExpr) ->
  std::result::Result<TypeShape, (String, proc_macro2::Span)>`; `TypeRegistry::display_name(&self,
  shape: &TypeShape) -> String`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `adam-lang/src/type_registry.rs`:

```rust
#[test]
fn new_registers_i32_with_the_new_descriptor_fields() {
    let reg = TypeRegistry::new();
    let entry = reg.get("i32").unwrap();
    assert_eq!(entry.size, std::mem::size_of::<i32>());
    assert_eq!(entry.align, std::mem::align_of::<i32>());
    let mut a = 7i32;
    let mut b = 0i32;
    unsafe {
        (entry.element_clone)((&raw mut a).cast::<u8>(), (&raw mut b).cast::<u8>());
    }
    assert_eq!(b, 7);
    assert!(unsafe {
        (entry.element_eq)((&raw const a).cast::<u8>(), (&raw const b).cast::<u8>())
    });
}

#[test]
fn resolve_named_type_expr_returns_the_matching_type_shape() {
    let reg = TypeRegistry::new();
    let expr = crate::ast::TypeExpr::Named("i32".to_string(), point(Span::call_site()));
    let shape = reg.resolve(&expr).unwrap();
    assert_eq!(shape, TypeShape::Named(TypeId::of::<i32>()));
}

#[test]
fn resolve_unknown_named_type_expr_is_an_error() {
    let reg = TypeRegistry::new();
    let expr = crate::ast::TypeExpr::Named("bogus".to_string(), point(Span::call_site()));
    assert!(reg.resolve(&expr).is_err());
}

#[test]
fn resolve_tuple_type_expr_returns_a_nested_type_shape() {
    let reg = TypeRegistry::new();
    let span = point(Span::call_site());
    let expr = crate::ast::TypeExpr::Tuple(
        vec![
            crate::ast::TypeExpr::Named("i32".to_string(), span),
            crate::ast::TypeExpr::Tuple(
                vec![
                    crate::ast::TypeExpr::Named("f64".to_string(), span),
                    crate::ast::TypeExpr::Named("String".to_string(), span),
                ],
                span,
            ),
        ],
        span,
    );
    let shape = reg.resolve(&expr).unwrap();
    assert_eq!(
        shape,
        TypeShape::Tuple(vec![
            TypeShape::Named(TypeId::of::<i32>()),
            TypeShape::Tuple(vec![
                TypeShape::Named(TypeId::of::<f64>()),
                TypeShape::Named(TypeId::of::<String>()),
            ]),
        ])
    );
}

#[test]
fn display_name_formats_a_nested_tuple_shape() {
    let reg = TypeRegistry::new();
    let shape = TypeShape::Tuple(vec![
        TypeShape::Named(TypeId::of::<i32>()),
        TypeShape::Tuple(vec![
            TypeShape::Named(TypeId::of::<f64>()),
            TypeShape::Named(TypeId::of::<String>()),
        ]),
    ]);
    assert_eq!(reg.display_name(&shape), "(i32, (f64, String))");
}
```

Add `use proc_macro2::Span;` and a local `fn point(span: Span) -> crate::ast::ExprSpan { crate::ast::ExprSpan { start: span, end: span } }`
helper to the test module (mirroring `ast.rs`'s own test helper of the same shape).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang type_registry::`
Expected: FAIL to compile — `TypeShape`, `resolve`, `display_name`, and the new `TypeEntry` fields
don't exist yet.

- [ ] **Step 3: Implement**

Add to `adam-lang/src/type_registry.rs`, right after the module doc comment's imports:

```rust
/// The identity of a declared adam-lang cell type. Every distinct tuple *shape* erases to the
/// same Rust type (`cel_runtime::DynamicSequence`), so a flat `TypeId` alone can no longer
/// identify a declared cell type once tuples exist — this recursive identity replaces it
/// wherever a declared cell type is tracked.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum TypeShape {
    /// A registered leaf type, by its Rust `TypeId`.
    Named(TypeId),
    /// A tuple type, recursively — an empty `Vec` for `()`.
    Tuple(Vec<TypeShape>),
}
```

Add to `TypeEntry`'s field list:

```rust
    /// Size in bytes of a value of this type.
    pub size: usize,
    /// Required alignment in bytes of a value of this type.
    pub align: usize,
    /// In-place dropper taking an unused `associated` parameter, for building an `AssociatedType`
    /// "prototype" describing a tuple-typed cell's on-stack shape.
    pub raw_dropper: cel_runtime::RawDropper,
    /// In-place dropper for this type as a `DynamicSequence` tuple element.
    pub element_drop: cel_runtime::ElementDropper,
    /// In-place cloner for this type as a `DynamicSequence` tuple element.
    pub element_clone: cel_runtime::ElementCloner,
    /// Equality comparator for this type as a `DynamicSequence` tuple element.
    pub element_eq: cel_runtime::ElementEq,
    /// Moves a boxed value of this type into a `DynamicSequence` being built from boxed defaults.
    pub element_write: unsafe fn(Box<dyn Any>, *mut u8),
```

Update `register`/`register_no_default` to populate the five new fields (add these lines to each
function's `TypeEntry { ... }` literal, alongside the existing ones):

```rust
                size: std::mem::size_of::<T>(),
                align: std::mem::align_of::<T>(),
                raw_dropper: cel_runtime::raw_dropper_for::<T>(),
                element_drop: cel_runtime::element_dropper_for::<T>(),
                element_clone: cel_runtime::element_cloner_for::<T>(),
                element_eq: cel_runtime::element_eq_for::<T>(),
                element_write: cel_runtime::element_writer_for::<T>(),
```

Add to `impl TypeRegistry`, right after `entry_by_type_id`:

```rust
    /// Resolves a parsed `type_expr` against this registry, recursively.
    ///
    /// # Errors
    /// Returns the unknown type name and its span if some leaf name isn't registered.
    pub fn resolve(
        &self,
        expr: &crate::ast::TypeExpr,
    ) -> std::result::Result<TypeShape, (String, proc_macro2::Span)> {
        match expr {
            crate::ast::TypeExpr::Named(name, span) => {
                let entry = self
                    .get(name)
                    .ok_or_else(|| (format!("unknown type `{name}`"), span.start))?;
                Ok(TypeShape::Named(entry.type_id))
            }
            crate::ast::TypeExpr::Tuple(elements, _) => {
                let shapes = elements
                    .iter()
                    .map(|e| self.resolve(e))
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(TypeShape::Tuple(shapes))
            }
        }
    }

    /// Formats `shape` recursively, e.g. `"(i32, (f64, String))"`, for error messages.
    #[must_use]
    pub fn display_name(&self, shape: &TypeShape) -> String {
        match shape {
            TypeShape::Named(type_id) => self
                .entry_by_type_id(*type_id)
                .map(|e| e.type_name.to_string())
                .unwrap_or_else(|| "?".to_string()),
            TypeShape::Tuple(elements) => {
                let parts: Vec<String> = elements.iter().map(|e| self.display_name(e)).collect();
                format!("({})", parts.join(", "))
            }
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-lang type_registry::`
Expected: PASS (all tests, including the 6 new ones).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-lang/src/type_registry.rs
git commit -m "feat(adam-lang): add TypeShape and TypeRegistry::resolve/display_name"
```

---

### Task 5: `TypeRegistry` — tuple construction helpers

**Files:**
- Modify: `adam-lang/src/type_registry.rs`

**Interfaces:**
- Consumes: `TypeShape` (Task 4); `cel_runtime::{AssociatedType, DynTuple, DynamicSequence,
  DynElementSpec, layout_associated, drop_tuple, element_dropper_for, element_cloner_for,
  element_eq_for, element_writer_for}`.
- Produces: `TypeRegistry::element_descriptor(&self, type_id: TypeId) -> Option<(ElementDropper,
  ElementCloner, ElementEq)>`; `TypeRegistry::associated_prototype(&self, shape: &TypeShape) ->
  Vec<AssociatedType>`; `TypeRegistry::default_dynamic_sequence(&self, shape: &TypeShape) ->
  std::result::Result<DynamicSequence, String>`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `adam-lang/src/type_registry.rs`:

```rust
#[test]
fn element_descriptor_returns_the_registered_types_own_functions() {
    let reg = TypeRegistry::new();
    let (drop, clone, eq) = reg.element_descriptor(TypeId::of::<i32>()).unwrap();
    let (mut a, mut b) = (7i32, 0i32);
    unsafe {
        clone((&raw mut a).cast::<u8>(), (&raw mut b).cast::<u8>());
    }
    assert_eq!(b, 7);
    assert!(unsafe { eq((&raw const a).cast::<u8>(), (&raw const b).cast::<u8>()) });
    unsafe { drop((&raw mut b).cast::<u8>()) };
}

#[test]
fn element_descriptor_is_none_for_an_unregistered_type() {
    let reg = TypeRegistry::new();
    assert!(reg.element_descriptor(TypeId::of::<Vec<u8>>()).is_none());
}

#[test]
fn associated_prototype_describes_a_flat_tuple() {
    let reg = TypeRegistry::new();
    let shape = TypeShape::Tuple(vec![
        TypeShape::Named(TypeId::of::<i32>()),
        TypeShape::Named(TypeId::of::<f64>()),
    ]);
    let prototype = reg.associated_prototype(&shape);
    assert_eq!(prototype.len(), 2);
    assert_eq!(prototype[0].type_id, TypeId::of::<i32>());
    assert_eq!(prototype[1].type_id, TypeId::of::<f64>());
    assert_eq!(prototype[1].offset, 8); // i32 at [0,4); f64 aligned up to 8
}

#[test]
fn associated_prototype_describes_a_nested_tuple() {
    let reg = TypeRegistry::new();
    let shape = TypeShape::Tuple(vec![
        TypeShape::Named(TypeId::of::<i32>()),
        TypeShape::Tuple(vec![
            TypeShape::Named(TypeId::of::<i32>()),
            TypeShape::Named(TypeId::of::<i32>()),
        ]),
    ]);
    let prototype = reg.associated_prototype(&shape);
    assert_eq!(prototype.len(), 2);
    assert_eq!(prototype[1].type_id, TypeId::of::<cel_runtime::DynTuple>());
    assert_eq!(prototype[1].associated.len(), 2);
}

#[test]
fn default_dynamic_sequence_builds_a_matching_default_value() {
    let reg = TypeRegistry::new();
    let shape = TypeShape::Tuple(vec![
        TypeShape::Named(TypeId::of::<i32>()),
        TypeShape::Named(TypeId::of::<f64>()),
    ]);
    let seq = reg.default_dynamic_sequence(&shape).unwrap();
    assert_eq!(seq.arity(), 2);
    let (a, b): (i32, f64) = seq.try_to_tuple().unwrap();
    assert_eq!((a, b), (i32::default(), f64::default()));
}

#[test]
fn default_dynamic_sequence_recurses_into_nested_tuples() {
    let reg = TypeRegistry::new();
    let shape = TypeShape::Tuple(vec![
        TypeShape::Named(TypeId::of::<i32>()),
        TypeShape::Tuple(vec![TypeShape::Named(TypeId::of::<f64>())]),
    ]);
    let seq = reg.default_dynamic_sequence(&shape).unwrap();
    let (_, nested): (i32, cel_runtime::DynamicSequence) = seq.try_to_tuple().unwrap();
    assert_eq!(nested.arity(), 1);
}

#[test]
fn default_dynamic_sequence_errors_naming_a_leaf_with_no_default() {
    #[derive(PartialEq, Clone)]
    struct NoDefault(i32);
    let mut reg = TypeRegistry::new();
    reg.register_no_default::<NoDefault>("NoDefault");
    let shape = TypeShape::Tuple(vec![TypeShape::Named(TypeId::of::<NoDefault>())]);
    let result = reg.default_dynamic_sequence(&shape);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("NoDefault"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang type_registry::tests::element_descriptor type_registry::tests::associated_prototype type_registry::tests::default_dynamic_sequence`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

Add to `impl TypeRegistry`, right after `display_name`:

```rust
    /// Returns the `(Drop, Clone, PartialEq)` triple registered for `type_id`, for use as the
    /// `leaf` callback `cel_runtime::DynSegment::call_dyn_as_dynamic_sequence` needs.
    #[must_use]
    pub fn element_descriptor(
        &self,
        type_id: TypeId,
    ) -> Option<(
        cel_runtime::ElementDropper,
        cel_runtime::ElementCloner,
        cel_runtime::ElementEq,
    )> {
        self.entry_by_type_id(type_id)
            .map(|e| (e.element_drop, e.element_clone, e.element_eq))
    }

    /// Builds the recursive `AssociatedType` "prototype" describing `shape`'s on-stack tuple
    /// layout, for `cel_runtime::DynSegment::push_arg_as_dynamic_sequence_tuple`.
    ///
    /// - Precondition: `shape` is `TypeShape::Tuple(_)` — a scalar cell never needs this.
    #[must_use]
    pub fn associated_prototype(&self, shape: &TypeShape) -> Vec<cel_runtime::AssociatedType> {
        let TypeShape::Tuple(elements) = shape else {
            debug_assert!(false, "associated_prototype's precondition: shape is a Tuple");
            return Vec::new();
        };
        elements.iter().map(|e| self.one_associated(e)).collect()
    }

    fn one_associated(&self, shape: &TypeShape) -> cel_runtime::AssociatedType {
        match shape {
            TypeShape::Named(type_id) => {
                let entry = self
                    .entry_by_type_id(*type_id)
                    .expect("one_associated: type registered");
                cel_runtime::AssociatedType {
                    type_id: *type_id,
                    type_name: std::borrow::Cow::Owned(entry.type_name.to_string()),
                    offset: 0,
                    size: entry.size,
                    align: entry.align,
                    dropper: entry.raw_dropper,
                    associated: Vec::new(),
                }
            }
            TypeShape::Tuple(elements) => {
                let mut associated: Vec<_> =
                    elements.iter().map(|e| self.one_associated(e)).collect();
                let (size, align) = cel_runtime::layout_associated(&mut associated);
                cel_runtime::AssociatedType {
                    type_id: TypeId::of::<cel_runtime::DynTuple>(),
                    type_name: std::borrow::Cow::Borrowed("tuple"),
                    offset: 0,
                    size,
                    align,
                    dropper: cel_runtime::drop_tuple,
                    associated,
                }
            }
        }
    }

    /// Builds a default `DynamicSequence` for a tuple-typed cell, recursively, using each leaf's
    /// own registered default.
    ///
    /// - Precondition: `shape` is `TypeShape::Tuple(_)`.
    ///
    /// # Errors
    /// Returns an error naming the specific leaf type that has no registered default.
    pub fn default_dynamic_sequence(
        &self,
        shape: &TypeShape,
    ) -> std::result::Result<cel_runtime::DynamicSequence, String> {
        let TypeShape::Tuple(elements) = shape else {
            debug_assert!(
                false,
                "default_dynamic_sequence's precondition: shape is a Tuple"
            );
            return Ok(cel_runtime::DynamicSequence::from_dyn_elements(Vec::new()));
        };
        let built = elements
            .iter()
            .map(|e| self.default_dyn_element(e))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(cel_runtime::DynamicSequence::from_dyn_elements(built))
    }

    fn default_dyn_element(
        &self,
        shape: &TypeShape,
    ) -> std::result::Result<(cel_runtime::DynElementSpec, Box<dyn Any>), String> {
        match shape {
            TypeShape::Named(type_id) => {
                let entry = self
                    .entry_by_type_id(*type_id)
                    .expect("default_dyn_element: type registered");
                let default_fn = entry.default_fn.ok_or_else(|| {
                    format!(
                        "type `{}` has no default; provide `= ...`",
                        entry.type_name
                    )
                })?;
                Ok((
                    cel_runtime::DynElementSpec {
                        type_id: *type_id,
                        type_name: std::borrow::Cow::Owned(entry.type_name.to_string()),
                        size: entry.size,
                        align: entry.align,
                        drop: entry.element_drop,
                        clone: entry.element_clone,
                        eq: entry.element_eq,
                        write: entry.element_write,
                    },
                    default_fn(),
                ))
            }
            TypeShape::Tuple(_) => {
                let nested = self.default_dynamic_sequence(shape)?;
                Ok((
                    cel_runtime::DynElementSpec {
                        type_id: TypeId::of::<cel_runtime::DynamicSequence>(),
                        type_name: std::borrow::Cow::Borrowed("DynamicSequence"),
                        size: std::mem::size_of::<cel_runtime::DynamicSequence>(),
                        align: std::mem::align_of::<cel_runtime::DynamicSequence>(),
                        drop: cel_runtime::element_dropper_for::<cel_runtime::DynamicSequence>(),
                        clone: cel_runtime::element_cloner_for::<cel_runtime::DynamicSequence>(),
                        eq: cel_runtime::element_eq_for::<cel_runtime::DynamicSequence>(),
                        write: cel_runtime::element_writer_for::<cel_runtime::DynamicSequence>(),
                    },
                    Box::new(nested) as Box<dyn Any>,
                ))
            }
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-lang type_registry::`
Expected: PASS (all tests, including the 7 new ones).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-lang/src/type_registry.rs
git commit -m "feat(adam-lang): add TypeRegistry tuple-construction helpers"
```

---

### Task 6: `typecheck.rs` — recursive `TypeShape` checks

**Files:**
- Modify: `adam-lang/src/typecheck.rs`

**Interfaces:**
- Consumes: `TypeRegistry::resolve`/`display_name` (Task 4); `ast::TypeExpr` (Task 1).

`declared_cell_types` currently maps each cell to a `cel_parser::Ty` (via
`Ty::from_type_id(entry.type_id)`). `Ty` has no tuple variant and is not being extended (per the
spec's non-goals) — so this task keeps `declared_cell_types`'s existing `Ty`-based map for
*scalar* checks against method/condition bodies exactly as-is (unchanged behavior, since
`cel_parser::ty::check_expr` still treats any tuple sub-expression as `Ty::Any`, exactly as
before), and adds a **separate**, parallel `TypeShape`-based check alongside it for cases only a
recursive shape check can catch: a cell/out's own initializer against its own annotation (already
existed for scalars; now generalized to tuples), and a method's per-output check when an output's
declared type is itself a tuple (new — mirrors `check_method`'s existing arity/tuple-body logic,
generalized).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `adam-lang/src/typecheck.rs`:

```rust
#[test]
fn cell_tuple_initializer_matching_its_annotation_has_no_diagnostic() {
    let sheet = parse("sheet s { cell a: (i32, f64) = (1, 2.5); }");
    let diags = check_sheet(&sheet, &TypeRegistry::new());
    assert!(diags.is_empty());
}

#[test]
fn cell_tuple_initializer_arity_mismatch_is_a_diagnostic() {
    let sheet = parse("sheet s { cell a: (i32, f64, i32) = (1, 2.5); }");
    let diags = check_sheet(&sheet, &TypeRegistry::new());
    assert_eq!(diags.len(), 1);
}

#[test]
fn cell_tuple_initializer_element_type_mismatch_is_a_diagnostic() {
    let sheet = parse("sheet s { cell a: (i32, i32) = (1, 2.5); }");
    let diags = check_sheet(&sheet, &TypeRegistry::new());
    assert_eq!(diags.len(), 1);
}

#[test]
fn cell_nested_tuple_initializer_matching_its_annotation_has_no_diagnostic() {
    let sheet = parse("sheet s { cell a: (i32, (f64, String)) = (1, (2.5, \"x\")); }");
    let diags = check_sheet(&sheet, &TypeRegistry::new());
    assert!(diags.is_empty());
}

#[test]
fn method_single_tuple_typed_output_matching_body_has_no_diagnostic() {
    let sheet = parse(
        "sheet s { cell a: i32; cell b: i32; cell pair: (i32, i32); \
         relationship { method [a, b] -> [pair] { (a, b) } } }",
    );
    let diags = check_sheet(&sheet, &TypeRegistry::new());
    assert!(diags.is_empty());
}

#[test]
fn method_single_tuple_typed_output_element_type_mismatch_is_a_diagnostic() {
    let sheet = parse(
        "sheet s { cell a: i32; cell b: f64; cell pair: (i32, i32); \
         relationship { method [a, b] -> [pair] { (a, b) } } }",
    );
    let diags = check_sheet(&sheet, &TypeRegistry::new());
    assert_eq!(diags.len(), 1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang typecheck::tests::cell_tuple_initializer_matching_its_annotation_has_no_diagnostic`
Expected: FAIL to compile (this file still references the old `CellDecl`/`OutDecl`/`MethodDecl`
usage patterns predating `TypeExpr`/`Expr`-typed initializers).

- [ ] **Step 3: Implement**

Add a new function to `adam-lang/src/typecheck.rs`, right after `literal_matches_declared_ty`:

```rust
/// Checks whether `expr` structurally matches `shape`, recursively: a `TypeShape::Named` leaf
/// must be a non-tuple `Expr` whose checked `Ty` unifies with that leaf (mirroring
/// `literal_matches_declared_ty`'s spirit, generalized past bare literals now that initializers
/// are full `or_expression`s); a `TypeShape::Tuple` must be an `Expr::Tuple` of matching arity,
/// checked element-wise. `TypeShape::Named(TypeId)` with no registered entry (an unrecognized
/// custom type) always matches — not statically checked, mirroring `Ty::Any`'s existing
/// leniency.
///
/// - Complexity: O(n) in the number of (nested) tuple elements.
fn expr_matches_shape(
    expr: &Expr,
    shape: &TypeShape,
    registry: &TypeRegistry,
    resolve: &impl Fn(&str) -> Ty,
    diagnostics: &mut Vec<ParseError>,
) {
    match (expr, shape) {
        (Expr::Tuple { elements, .. }, TypeShape::Tuple(expected)) => {
            if elements.len() != expected.len() {
                diagnostics.push(ParseError::new_range(
                    format!(
                        "expected a {}-element tuple `{}`, got {}",
                        expected.len(),
                        registry.display_name(shape),
                        elements.len()
                    ),
                    expr.span().start,
                    expr.span().end,
                ));
                return;
            }
            for (element, element_shape) in elements.iter().zip(expected) {
                expr_matches_shape(element, element_shape, registry, resolve, diagnostics);
            }
        }
        (_, TypeShape::Tuple(_)) => {
            diagnostics.push(ParseError::new_range(
                format!("expected tuple `{}`", registry.display_name(shape)),
                expr.span().start,
                expr.span().end,
            ));
        }
        (Expr::Tuple { .. }, TypeShape::Named(_)) => {
            diagnostics.push(ParseError::new_range(
                format!(
                    "expected `{}`, got a tuple",
                    registry.display_name(shape)
                ),
                expr.span().start,
                expr.span().end,
            ));
        }
        (_, TypeShape::Named(type_id)) => {
            let Some(entry) = registry.entry_by_type_id(*type_id) else {
                return; // unrecognized custom type: never statically checked, matches Ty::Any
            };
            let declared = Ty::from_type_id(entry.type_id);
            let (actual, body_diags) = check_expr(expr, resolve);
            diagnostics.extend(body_diags);
            if !declared.unifies_with(&actual) {
                diagnostics.push(ParseError::new_range(
                    format!(
                        "expression produces `{}`, but `{}` was expected",
                        actual.name(),
                        declared.name()
                    ),
                    expr.span().start,
                    expr.span().end,
                ));
            }
        }
    }
}
```

Add `use crate::TypeRegistry;`'s companion import `use crate::type_registry::TypeShape;` to the
top of the file (alongside the existing `use crate::TypeRegistry;`).

Update `check_cell_initializer` to call it when a tuple type is involved (leave the existing
`literal_matches_declared_ty` path for the scalar case exactly as-is — this is additive, not a
replacement):

```rust
fn check_cell_initializer(
    cell: &CellDecl,
    registry: &TypeRegistry,
    diagnostics: &mut Vec<ParseError>,
) {
    let (Some(type_expr), Some(expr)) = (&cell.type_name, &cell.initializer) else {
        return;
    };
    let Ok(shape) = registry.resolve(type_expr) else {
        return; // unknown type name: already reported by the real parser's own error path
    };
    if let TypeShape::Tuple(_) = shape {
        let resolve = |_: &str| Ty::Any; // initializers reference no cells
        expr_matches_shape(expr, &shape, registry, &resolve, diagnostics);
        return;
    }
    // Scalar case: unchanged from before, still literal-shaped in practice (an initializer that
    // isn't a bare literal fails to constant-fold in the real parser; this checker only needs to
    // flag a literal/type mismatch, exactly as it always has).
    let Expr::Literal { value: literal, span: lit_span, .. } = expr else {
        return;
    };
    let declared = Ty::from_type_id(match registry.entry_by_type_id(match shape {
        TypeShape::Named(tid) => tid,
        TypeShape::Tuple(_) => unreachable!("handled above"),
    }) {
        Some(entry) => entry.type_id,
        None => return,
    });
    if !literal_matches_declared_ty(literal, declared) {
        diagnostics.push(ParseError::new_range(
            format!("literal cannot be used as type `{}`", declared.name()),
            lit_span.start,
            lit_span.end,
        ));
    }
}
```

**Note for the implementer:** confirm `cel_parser::Expr`'s actual literal-holding variant name and
field shape (this plan assumes `Expr::Literal { value: Literal, span, .. }` by analogy with
`Expr::Ident`/`Expr::Tuple`'s shapes already seen in `ast.rs`/`ast_parser.rs` — check
`cel-parser/src/ast.rs`'s real `Expr` enum definition and adjust the pattern match accordingly if
the actual variant/field names differ).

Update `declared_cell_types` to resolve tuple-typed cells' `Ty` as `Ty::Any` (unchanged from
today's behavior for any type `Ty::from_type_id` can't represent — tuples fall into that same
bucket, so no new code is needed here; `Ty::from_type_id` already returns `Ty::Any` for a
`TypeId` it doesn't recognize as one of its own scalar variants). Confirm this with a quick read
of `cel_parser::Ty::from_type_id` before assuming — if it panics instead of returning `Ty::Any`
for an unrecognized `TypeId` (e.g. `DynamicSequence`'s own `TypeId`), guard the call:
`registry.get(name).and_then(|e| Ty::from_type_id(e.type_id).ok())` or equivalent, matching
whatever that function's actual signature is.

Update `check_method`'s `[(name, _)]` (single-output) branch to also call `expr_matches_shape` when
the resolved output type is a tuple:

```rust
        [(name, _)] => {
            let Some(cell_decl) = /* look up this output's own declared TypeExpr, if any, via the
                sheet-level cell_types map already threaded through -- see the implementer note
                below */ else {
                // existing scalar-Ty path, unchanged
                let (body_ty, body_diags) = check_expr(&method.body, resolve);
                diagnostics.extend(body_diags);
                if let Expr::Tuple { elements, .. } = &method.body {
                    let n = elements.len();
                    diagnostics.push(ParseError::new_range(
                        format!("method declares 1 output but its body is a {n}-tuple"),
                        method.body.span().start,
                        method.body.span().end,
                    ));
                    return;
                }
                let declared = resolve(name);
                if !declared.unifies_with(&body_ty) {
                    diagnostics.push(ParseError::new_range(
                        format!(
                            "method body produces `{}`, but `{name}` is declared `{}`",
                            body_ty.name(),
                            declared.name()
                        ),
                        method.body.span().start,
                        method.body.span().end,
                    ));
                }
            };
        }
```

**Implementer note:** `check_method` currently receives only `resolve: &impl Fn(&str) -> Ty`
(scalar types), built once by `declared_cell_types` from every cell's annotation. To detect "this
output's declared type is a tuple" here, thread an additional `shapes: &HashMap<String,
TypeShape>` (built alongside `declared_cell_types`, same recursive-resolve call per cell,
`registry.resolve(type_expr).ok()`) through `check_sheet` → `check_method`/`check_out`, and check
`shapes.get(name)` for `Some(TypeShape::Tuple(_))` before falling back to the existing scalar `Ty`
path. Wire this by adding a `shapes: &std::collections::HashMap<String, TypeShape>` parameter to
`check_method`/`check_out` (and their one call site each in `check_sheet`), populated from a new
sibling function to `declared_cell_types` (or extending it to return `(HashMap<String, Ty>,
HashMap<String, TypeShape>)` — the two maps' keys are identical, built from the same loop over
`sheet.items`, so building them together in one pass is the natural shape). Add the analogous
tuple-checking branch to `check_out`'s single-output-shaped logic too (an `out` is structurally "a
method with one implicit output").

Update every existing call site inside this file that constructs a `CellDecl`/uses
`cell.initializer` as a `(Literal, ExprSpan)` tuple (the existing tests already construct sheets
via `parse(...)`/`AdamAstParser`, not literal `CellDecl { .. }` structs, so no test-fixture changes
should be needed beyond what Task 1 already handled — confirm by running `cargo check -p adam-lang
--tests` after this step and fixing any remaining compile errors it reports).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-lang typecheck::`
Expected: PASS (every pre-existing test plus the 6 new ones).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-lang/src/typecheck.rs
git commit -m "feat(adam-lang): add recursive TypeShape checks for tuple cells and outputs"
```

---

### Task 7: Real parser — `cell`/`out` declarations with tuple types

**Files:**
- Modify: `adam-lang/src/parser.rs`

**Interfaces:**
- Consumes: `TypeRegistry::{resolve, default_dynamic_sequence, associated_prototype,
  element_descriptor}` (Tasks 4–5); `cel_runtime::DynSegment::call_dyn_as_dynamic_sequence`
  (cel-runtime plan Task 3).
- Produces: `parse_cell_decl` and `parse_out_decl` generalized to `TypeExpr`/`TypeShape`, storing a
  `DynamicSequence` value (via `add_cell_impl::<DynamicSequence>`, already generic — no change
  needed there) for a tuple-typed cell. `ParseContext.cell_names`/`ParsedSheet.cell_names` change
  from `IndexMap<String, (CellId, TypeId)>` to `IndexMap<String, (CellId, TypeShape)>`.

This is the biggest single mechanical ripple in the plan: every place that currently reads a
`TypeId` out of `cell_names`/compares it with `==` now reads/compares a `TypeShape`. `TypeShape`
derives `PartialEq`, so `==` comparisons keep working verbatim wherever the code only ever compared
two `TypeShape`s (or, for scalar cells, effectively still compares two `TypeShape::Named(TypeId)`
values, which `==` handles exactly like the old flat `TypeId ==` did). This task covers only
`parse_cell_decl`/`parse_out_decl` and the `cell_names`/`ParsedSheet` type change; Task 8 covers
`parse_method_body`, and Task 9 covers `parse_conditional_decl` — both of which also read
`cell_names`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `adam-lang/src/parser.rs`:

```rust
#[test]
fn parse_cell_with_explicit_tuple_type_and_initializer() {
    let mut p = parser();
    let parsed = p
        .parse_str("sheet s { cell a: (i32, f64) = (1, 2.5); }")
        .unwrap();
    let (cell_id, shape) = parsed.cell_names["a"].clone();
    assert_eq!(
        shape,
        adam_lang::type_registry::TypeShape::Tuple(vec![
            adam_lang::type_registry::TypeShape::Named(std::any::TypeId::of::<i32>()),
            adam_lang::type_registry::TypeShape::Named(std::any::TypeId::of::<f64>()),
        ])
    );
    let value = parsed.sheet.read::<cel_runtime::DynamicSequence>(cell_id).unwrap();
    let (a, b): (i32, f64) = value.try_to_tuple().unwrap();
    assert_eq!((a, b), (1, 2.5));
}

#[test]
fn parse_cell_with_tuple_type_and_no_initializer_uses_recursive_default() {
    let parsed = parser().parse_str("sheet s { cell a: (i32, f64); }").unwrap();
    let (cell_id, _) = parsed.cell_names["a"].clone();
    let value = parsed.sheet.read::<cel_runtime::DynamicSequence>(cell_id).unwrap();
    let (a, b): (i32, f64) = value.try_to_tuple().unwrap();
    assert_eq!((a, b), (0, 0.0));
}

#[test]
fn parse_cell_with_tuple_initializer_arity_mismatch_is_an_error() {
    let result = parser().parse_str("sheet s { cell a: (i32, f64, i32) = (1, 2.5); }");
    assert!(result.is_err());
}

#[test]
fn parse_cell_with_nested_tuple_type_round_trips() {
    let parsed = parser()
        .parse_str("sheet s { cell a: (i32, (f64, String)) = (1, (2.5, \"x\")); }")
        .unwrap();
    let (cell_id, _) = parsed.cell_names["a"].clone();
    let value = parsed.sheet.read::<cel_runtime::DynamicSequence>(cell_id).unwrap();
    let (a, nested): (i32, cel_runtime::DynamicSequence) = value.try_to_tuple().unwrap();
    assert_eq!(a, 1);
    let (b, c): (f64, String) = nested.try_to_tuple().unwrap();
    assert_eq!((b, c), (2.5, "x".to_string()));
}

#[test]
fn parse_out_with_explicit_tuple_type_infers_and_stores_correctly() {
    let mut sheet = parser()
        .parse_str(
            r#"
            sheet s {
                cell x: i32 = 3;
                out pair: (i32, i32) { method [x] { (x, x) } }
            }
        "#,
        )
        .unwrap();
    sheet.propagate().unwrap();
    let output_id = *sheet.output_names.get("pair").unwrap();
    let cell_id = sheet.output_cell(output_id).unwrap();
    let value = sheet.read::<cel_runtime::DynamicSequence>(cell_id).unwrap();
    let (a, b): (i32, i32) = value.try_to_tuple().unwrap();
    assert_eq!((a, b), (3, 3));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang parser::tests::parse_cell_with_explicit_tuple_type_and_initializer`
Expected: FAIL to compile — `parser.rs` still resolves `type_name` as a flat string and
`initializer` as a `(Literal, ExprSpan)`.

- [ ] **Step 3: Implement**

Change `ParseContext`/`ParsedSheet`'s `cell_names` field type from `IndexMap<String, (CellId,
TypeId)>` to `IndexMap<String, (CellId, crate::type_registry::TypeShape)>` in both structs' field
declarations and doc comments.

Rewrite `parse_cell_decl` (the whole function): replace the `":" type_name` branch's ident-based
lookup with `self.types.resolve(&type_expr)` (mapping its `(String, Span)` error into a
`ParseError`), branch on whether the resolved `TypeShape` is `Named` or `Tuple`, and route the
initializer through `self.parse_cel_or_expression` (already used elsewhere in this file) followed
by an eager, zero-input evaluation:

```rust
    /// `cell_decl = "cell" identifier cell_type_init ";".`
    ///
    /// `cell_type_init = (":" type_expr ["=" or_expression]) | ("=" or_expression).`
    fn parse_cell_decl(&mut self, ctx: &mut ParseContext) -> Result<()> {
        ctx.is_keyword("cell"); // consume
        let (name, name_span) = ctx.consume_ident()?;
        if ctx.cell_names.contains_key(&name) {
            return Err(ParseError::new(
                format!("duplicate cell `{name}`"),
                name_span,
            ));
        }

        let declared_shape: Option<TypeShape> = if ctx.consume_punct(":") {
            let type_expr = self.parse_type_expr(ctx)?;
            Some(
                self.types
                    .resolve(&type_expr)
                    .map_err(|(msg, span)| ParseError::new(msg, span))?,
            )
        } else {
            None
        };

        let has_initializer = ctx.consume_punct("=");
        let (shape, cell_id) = if has_initializer {
            let segment = self.parse_cel_or_expression(ctx)?;
            let (actual_shape, cell_id) = self.build_cell_from_segment(segment, ctx)?;
            if let Some(declared) = &declared_shape {
                if declared != &actual_shape {
                    return Err(ParseError::new(
                        format!(
                            "cell `{name}`: type mismatch: expected `{}`, got `{}`",
                            self.types.display_name(declared),
                            self.types.display_name(&actual_shape)
                        ),
                        name_span,
                    ));
                }
            }
            (actual_shape, cell_id)
        } else {
            let declared = declared_shape.ok_or_else(|| {
                ParseError::new("expected `:` or `=` in cell declaration", name_span)
            })?;
            let cell_id = self.build_default_cell(&declared, name_span, ctx)?;
            (declared, cell_id)
        };

        ctx.expect_punct(";")?;
        ctx.cell_names.insert(name, (cell_id, shape));
        Ok(())
    }
```

Add these two private helper methods on `AdamParser`, right after `parse_cell_decl`, factoring out
the "given a compiled, zero-argument segment (or a declared shape with no initializer), produce a
cell in `ctx.sheet`" logic shared by `parse_cell_decl` and `parse_out_decl` (rewritten below):

```rust
    /// Evaluates `segment` eagerly with no inputs, inferring its result's `TypeShape` from the
    /// segment's own tuple stack info (read *before* consuming the segment) for a tuple result,
    /// or from `peek_output_type_id` for a scalar result. Returns the result boxed (`Box<dyn
    /// Any>` holding either a scalar `T` or a `DynamicSequence`) — adding it to a `Sheet` or
    /// using it as a conditional branch key is the caller's job (see
    /// [`build_cell_from_segment`](Self::build_cell_from_segment) and
    /// [`parse_conditional_decl`](Self::parse_conditional_decl)).
    ///
    /// # Errors
    /// Returns `Err` if the segment's result type isn't registered (scalar case) or contains an
    /// unregistered leaf type at any nesting depth (tuple case).
    fn eval_segment_boxed(&self, mut segment: DynSegment) -> Result<(TypeShape, Box<dyn Any>)> {
        if segment.peek_tuple_arity().is_some() {
            let associated = segment.peek_stack_infos(1)[0].associated.clone();
            let shape = self
                .shape_of_associated(&associated)
                .map_err(|msg| ParseError::new(msg, Span::call_site()))?;
            let leaf = |type_id: TypeId| self.types.element_descriptor(type_id);
            let seq = segment
                .call_dyn_as_dynamic_sequence(&[], &leaf)
                .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
            Ok((shape, Box::new(seq) as Box<dyn Any>))
        } else {
            let type_id = segment.peek_output_type_id().ok_or_else(|| {
                ParseError::new("expression produced no value", Span::call_site())
            })?;
            let entry = self.types.entry_by_type_id(type_id).ok_or_else(|| {
                ParseError::new(
                    "cannot infer a type for this expression; register a type name for it or \
                     add an explicit `: type_expr` annotation",
                    Span::call_site(),
                )
            })?;
            let boxed = (entry.call_dyn_fn)(&mut segment, &[])
                .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
            Ok((TypeShape::Named(type_id), boxed))
        }
    }

    /// Evaluates `segment` via [`eval_segment_boxed`](Self::eval_segment_boxed) and adds a
    /// matching cell to `ctx.sheet`, using the registered `add_cell_fn` for a `TypeShape::Named`
    /// result, or `Sheet::add_cell::<DynamicSequence>` directly for a `TypeShape::Tuple` result
    /// (tuple-typed cells are never themselves registered in `TypeRegistry` — every distinct
    /// shape shares the one concrete storage type, `DynamicSequence`).
    ///
    /// # Errors
    /// See `eval_segment_boxed`.
    fn build_cell_from_segment(
        &self,
        segment: DynSegment,
        ctx: &mut ParseContext,
    ) -> Result<(TypeShape, CellId)> {
        let (shape, boxed) = self.eval_segment_boxed(segment)?;
        let cell_id = match &shape {
            TypeShape::Named(type_id) => {
                let entry = self.types.entry_by_type_id(*type_id).expect("registered");
                (entry.add_cell_fn)(&mut ctx.sheet, boxed)
            }
            TypeShape::Tuple(_) => {
                let seq = *boxed
                    .downcast::<cel_runtime::DynamicSequence>()
                    .expect("eval_segment_boxed: a Tuple shape always boxes a DynamicSequence");
                ctx.sheet.add_cell(seq)
            }
        };
        Ok((shape, cell_id))
    }

    /// Recursively converts a live tuple's `AssociatedType` shape into a `TypeShape`, by looking
    /// up each leaf's `TypeId` against `self.types`.
    ///
    /// # Errors
    /// Returns an error naming any element's `TypeId` (at any nesting depth) that isn't
    /// registered.
    fn shape_of_associated(
        &self,
        associated: &[cel_runtime::AssociatedType],
    ) -> std::result::Result<TypeShape, String> {
        let elements = associated
            .iter()
            .map(|elem| {
                if elem.type_id == TypeId::of::<cel_runtime::DynTuple>() {
                    self.shape_of_associated(&elem.associated)
                } else {
                    self.types
                        .entry_by_type_id(elem.type_id)
                        .map(|entry| TypeShape::Named(entry.type_id))
                        .ok_or_else(|| format!("unregistered type `{}`", elem.type_name))
                }
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(TypeShape::Tuple(elements))
    }

    /// Builds a default-valued cell for `shape` (scalar or tuple, recursively), adding it to
    /// `ctx.sheet`.
    ///
    /// # Errors
    /// Returns `Err` naming the type/leaf that has no registered default.
    fn build_default_cell(
        &self,
        shape: &TypeShape,
        span: Span,
        ctx: &mut ParseContext,
    ) -> Result<CellId> {
        match shape {
            TypeShape::Named(type_id) => {
                let entry = self
                    .types
                    .entry_by_type_id(*type_id)
                    .expect("build_default_cell: type registered (resolved via TypeRegistry)");
                let default_fn = entry.default_fn.ok_or_else(|| {
                    ParseError::new(
                        format!("type `{}` has no default; provide `= ...`", entry.type_name),
                        span,
                    )
                })?;
                Ok((entry.add_cell_fn)(&mut ctx.sheet, default_fn()))
            }
            TypeShape::Tuple(_) => {
                let seq = self
                    .types
                    .default_dynamic_sequence(shape)
                    .map_err(|msg| ParseError::new(msg, span))?;
                Ok(ctx.sheet.add_cell(seq))
            }
        }
    }
```

`adam_rs` itself is CEL-agnostic and tracks input/output types as plain `TypeId`s
(`Method::new`/`Condition::new`'s `input_types`/`output_types` parameters, unchanged from before) —
it has no notion of `TypeShape`. Every place that currently flattens a cell's type to the single
`TypeId` handed to `adam_rs` (today: `*tid` off a `(String, CellId, TypeId)` triple) needs a
`TypeShape` → `TypeId` flattening rule instead: a `Named` shape flattens to its own `TypeId`
unchanged; a `Tuple` shape flattens to `TypeId::of::<DynamicSequence>()`, since every tuple shape
shares that one concrete storage type regardless of its own arity/element types (`adam_rs` only
ever needs to know "these two cells are/aren't the same Rust type," which is exactly what
`TypeId::of::<DynamicSequence>()` being shared across all tuple shapes correctly expresses — a
`(i32, f64)` cell and an `(i32, i32)` cell are both, correctly, "the same `adam_rs`-level type").
Add this small free function to `parser.rs`, right after `shape_of_associated`:

```rust
/// Flattens a declared cell's `TypeShape` to the single `TypeId` `adam_rs` itself needs for its
/// own (CEL-agnostic) type bookkeeping — a `Tuple` shape always flattens to
/// `TypeId::of::<DynamicSequence>()`, since every tuple shape shares that one concrete storage
/// type regardless of arity/element types.
fn cell_type_id(shape: &TypeShape) -> TypeId {
    match shape {
        TypeShape::Named(type_id) => *type_id,
        TypeShape::Tuple(_) => TypeId::of::<cel_runtime::DynamicSequence>(),
    }
}
```

Change `parse_cell_list`'s return type from `Result<Vec<(String, CellId, TypeId)>>` to
`Result<Vec<(String, CellId, TypeShape)>>` now (not deferred to Task 8): its one internal
`.copied()` on the `cell_names` lookup becomes `.cloned()` (`TypeShape` isn't `Copy`). This must
happen in this task rather than Task 8 because `parse_out_decl` (rewritten below) already calls
it for the writer method's `inputs`. Task 8's `parse_method_decl`/`parse_relationship_decl`
consume this same already-updated signature — no further change to `parse_cell_list` itself is
needed there.

Update `parse_condition_decl`'s `input_types: Vec<TypeId> = inputs.iter().map(|(_, _, tid)|
*tid).collect();` to `inputs.iter().map(|(_, _, shape)| cell_type_id(shape)).collect();` (`inputs`
is now `Vec<(String, CellId, TypeShape)>` per the `parse_cell_list` change just above).

Rewrite `parse_out_decl` analogously: replace its `":" type_name` ident lookup with
`self.parse_type_expr(ctx)` + `self.types.resolve(...)`, and its `declared: Option<(TypeId,
AddCellFn)>`/`actual_type_id`/type-mismatch-message logic with the `TypeShape`-based equivalent
(reuse `build_cell_from_segment` for the writer body's result, comparing against the declared
shape the same way `parse_cell_decl` now does). The rest of `parse_out_decl` (building the
`writer: Method`, conditions, `add_output`) stays structurally the same — only the type
identity/comparison changes from `TypeId` to `TypeShape`, and a tuple-typed `out` uses
`add_cell_impl::<DynamicSequence>`-equivalent storage exactly like a tuple-typed `cell`.

Update `parse_conditional_decl`'s two `entry_by_type_id(match_type_id)` calls' surrounding context:
`match_type_id` (from `ctx.cell_names.get(&match_name)`) is now a `TypeShape`, not a `TypeId` — for
this task, only make it *compile* by requiring `match_type_id` be `TypeShape::Named(_)` (a tuple
cell as a conditional match key is out of scope for this task; Task 9 below decides whether to
support it) with a clear `unimplemented!`/explicit error if it's `Tuple` for now, so this task's
own tests aren't blocked on Task 9's design. Leave a `// TODO(Task 9)` comment, not a silent gap.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-lang parser::`
Expected: Every pre-existing scalar-cell test still passes unchanged; the 5 new tests pass. Some
pre-existing conditional tests may need their match-cell type confirmed still `Named` (they
already all use scalar `i32` match cells today, so this should be a non-issue — verify by running
the full module, not just the new tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-lang/src/parser.rs
git commit -m "feat(adam-lang): parse and construct tuple-typed cell/out declarations"
```

---

### Task 8: Real parser — method-output unification and tuple-typed inputs

**Files:**
- Modify: `adam-lang/src/parser.rs`
- Modify: `adam-lang/src/type_registry.rs`

**Interfaces:**
- Consumes: `cel_runtime::DynSegment::push_arg_as_dynamic_sequence_tuple` (cel-runtime plan Task
  5); `cel_runtime::{DynSegment::call_dyn_tuple_mixed, DynExtractor}` (cel-runtime plan Task 3);
  `TypeRegistry::associated_prototype`/`element_descriptor` (Task 5); `AdamParser::shape_of_associated`
  (Task 7); everything from Task 7.
- Produces: `TypeRegistry::element_descriptors_for` (new, in `type_registry.rs`);
  `parse_method_body` generalized per the spec's section 5 (`CompiledOutputs` gains `SingleTuple`
  and `EmptyTuple` variants alongside `Single`, and `Tuple`'s own extractors generalized — via
  `cel_runtime::DynExtractor` — to allow a tuple-shaped element among several);
  `parse_body_with_input_scope`'s `push_arg` wiring generalized to dispatch to
  `push_arg_as_dynamic_sequence_tuple` for a tuple-typed input cell.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `adam-lang/src/parser.rs`:

```rust
#[test]
fn parse_method_single_tuple_typed_output() {
    let mut sheet = parser()
        .parse_str(
            r#"
            sheet s {
                cell a: i32 = 3;
                cell b: i32 = 4;
                cell pair: (i32, i32);
                relationship { method [a, b] -> [pair] { (a, b) } }
            }
        "#,
        )
        .unwrap();
    sheet.propagate().unwrap();
    let (cell_id, _) = sheet.cell_names["pair"].clone();
    let value = sheet.read::<cel_runtime::DynamicSequence>(cell_id).unwrap();
    let (a, b): (i32, i32) = value.try_to_tuple().unwrap();
    assert_eq!((a, b), (3, 4));
}

#[test]
fn parse_method_tuple_typed_output_among_several() {
    let mut sheet = parser()
        .parse_str(
            r#"
            sheet s {
                cell a: i32 = 3;
                cell b: i32 = 4;
                cell pair: (i32, i32);
                cell extra: i32;
                relationship { method [a, b] -> [pair, extra] { ((a, b), a) } }
            }
        "#,
        )
        .unwrap();
    sheet.propagate().unwrap();
    let (pair_id, _) = sheet.cell_names["pair"].clone();
    let (extra_id, _) = sheet.cell_names["extra"].clone();
    let pair = sheet.read::<cel_runtime::DynamicSequence>(pair_id).unwrap();
    let (a, b): (i32, i32) = pair.try_to_tuple().unwrap();
    assert_eq!((a, b), (3, 4));
    assert_eq!(*sheet.read::<i32>(extra_id).unwrap(), 3);
}

#[test]
fn parse_method_with_tuple_typed_input_supports_field_indexing() {
    let mut sheet = parser()
        .parse_str(
            r#"
            sheet s {
                cell pair: (i32, i32) = (10, 20);
                cell sum: i32;
                relationship { method [pair] -> [sum] { pair.0 + pair.1 } }
            }
        "#,
        )
        .unwrap();
    sheet.propagate().unwrap();
    let (sum_id, _) = sheet.cell_names["sum"].clone();
    assert_eq!(*sheet.read::<i32>(sum_id).unwrap(), 30);
}

#[test]
fn parse_method_tuple_output_shape_mismatch_is_an_error() {
    let result = parser().parse_str(
        r#"
        sheet s {
            cell a: i32 = 1;
            cell b: f64 = 2.0;
            cell pair: (i32, i32);
            relationship { method [a, b] -> [pair] { (a, b) } }
        }
    "#,
    );
    assert!(result.is_err());
}

#[test]
fn existing_multi_output_scalar_methods_still_work_unchanged() {
    // Regression: today's N-scalar-outputs mechanism must still behave identically after the
    // CompiledOutputs refactor.
    let mut sheet = parser()
        .parse_str(
            r#"
            sheet s {
                cell a: i32 = 3;
                cell b: i32 = 4;
                cell sum: i32;
                cell diff: i32;
                relationship { method [a, b] -> [sum, diff] { (a + b, a - b) } }
            }
        "#,
        )
        .unwrap();
    sheet.propagate().unwrap();
    let (sum_id, _) = sheet.cell_names["sum"].clone();
    let (diff_id, _) = sheet.cell_names["diff"].clone();
    assert_eq!(*sheet.read::<i32>(sum_id).unwrap(), 7);
    assert_eq!(*sheet.read::<i32>(diff_id).unwrap(), -1);
}

#[test]
fn parse_method_single_empty_tuple_typed_output() {
    let mut sheet = parser()
        .parse_str(
            r#"
            sheet s {
                cell x: i32 = 1;
                cell nothing: ();
                relationship { method [x] -> [nothing] { () } }
            }
        "#,
        )
        .unwrap();
    sheet.propagate().unwrap();
    let (cell_id, _) = sheet.cell_names["nothing"].clone();
    let value = sheet.read::<cel_runtime::DynamicSequence>(cell_id).unwrap();
    assert_eq!(value.arity(), 0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang parser::tests::parse_method_single_tuple_typed_output`
Expected: FAIL — `pair`'s output isn't yet wired to build a `DynamicSequence`; the input-indexing
test fails since `push_arg` still only handles scalar cells.

- [ ] **Step 3: Implement**

(`parse_cell_list` already returns `Vec<(String, CellId, TypeShape)>` as of Task 7, since
`parse_out_decl` needed it there first — `parse_method_decl`/`parse_relationship_decl`'s `inputs`/
`outputs` below are already that type with no further signature change.)

Add to `TypeRegistry` (`type_registry.rs`), right after `element_descriptor` (Task 5): an owned,
`'static`-safe lookup table builder, so a `Method`'s stored closure (which must outlive the
parser/registry) never needs to capture a reference back into `TypeRegistry` — it captures this
small owned `Vec` instead, exactly like `entry.call_dyn_fn`/`entry.add_cell_fn` are already copied
out as bare `fn` pointers rather than kept as registry references:

```rust
    /// Builds an owned table of every leaf `TypeId` in `shape` paired with its
    /// `Drop`/`Clone`/`PartialEq` descriptor, for a closure that must outlive this registry (e.g.
    /// a `Method`'s stored output-extraction closure).
    ///
    /// - Precondition: every leaf `TypeId` in `shape` is registered (already resolved via
    ///   `TypeRegistry::resolve`, which would have already errored otherwise).
    #[must_use]
    pub fn element_descriptors_for(
        &self,
        shape: &TypeShape,
    ) -> Vec<(
        TypeId,
        cel_runtime::ElementDropper,
        cel_runtime::ElementCloner,
        cel_runtime::ElementEq,
    )> {
        match shape {
            TypeShape::Named(type_id) => {
                let (drop, clone, eq) = self
                    .element_descriptor(*type_id)
                    .expect("element_descriptors_for: type registered");
                vec![(*type_id, drop, clone, eq)]
            }
            TypeShape::Tuple(elements) => elements
                .iter()
                .flat_map(|e| self.element_descriptors_for(e))
                .collect(),
        }
    }
```

Add to `parser.rs`, two small structural-match helpers (kept separate rather than one dual-purpose
recursive function, since they compare different things — a whole tuple's element list vs. one
single element):

```rust
/// Returns whether one live tuple element `a` structurally matches one declared leaf/tuple
/// `shape` — the base case `tuple_shape_matches_associated` recurses into.
fn element_shape_matches(shape: &TypeShape, a: &cel_runtime::AssociatedType) -> bool {
    match shape {
        TypeShape::Named(type_id) => a.type_id == *type_id,
        TypeShape::Tuple(_) => {
            a.type_id == TypeId::of::<cel_runtime::DynTuple>()
                && tuple_shape_matches_associated(shape, &a.associated)
        }
    }
}

/// Returns whether a whole tuple's element list `associated` structurally matches
/// `shape`'s own top-level element list (same arity, each pair checked via
/// `element_shape_matches`) — `shape` must be `TypeShape::Tuple`.
fn tuple_shape_matches_associated(shape: &TypeShape, associated: &[cel_runtime::AssociatedType]) -> bool {
    let TypeShape::Tuple(elements) = shape else {
        return false;
    };
    elements.len() == associated.len()
        && elements
            .iter()
            .zip(associated)
            .all(|(e, a)| element_shape_matches(e, a))
}
```

Generalize `CompiledOutputs` (in `parser.rs`), reusing `cel_runtime::DynExtractor` directly for the
N>1 case instead of a duplicate local enum:

```rust
/// How to turn one compiled `or_expression`'s result into per-output values.
enum CompiledOutputs {
    /// One output, scalar: the segment's single result, boxed via `call_dyn`.
    Single(CallDynFn),
    /// One output, tuple-typed: the segment's whole tuple result, moved into one
    /// `DynamicSequence` via `call_dyn_as_dynamic_sequence`.
    SingleTuple(
        Vec<(
            TypeId,
            cel_runtime::ElementDropper,
            cel_runtime::ElementCloner,
            cel_runtime::ElementEq,
        )>,
    ),
    /// The declared output is the empty tuple `()`: no CEL expression can produce a live
    /// `DynTuple`-tagged 0-arity value (CEL's own `()` literal is the concrete Rust unit type,
    /// a distinct leaf `TypeId`, not `DynTuple`) — so this is its own case, matched directly
    /// against a `()`-typed body result and stored as a trivially-empty `DynamicSequence`.
    EmptyTuple,
    /// N > 1 outputs: the segment's tuple result, split element-wise via `call_dyn_tuple_mixed`.
    Tuple(Vec<cel_runtime::DynExtractor>),
}
```

Update `parse_method_body`'s `outputs.len() == 1` branch:

```rust
        let compiled = if outputs.len() == 1 {
            let (out_name, _, out_shape) = &outputs[0];
            match out_shape {
                TypeShape::Named(out_type_id) => {
                    // unchanged from before: scalar single-output path
                    let actual_type_id = segment.peek_output_type_id().ok_or_else(|| {
                        ctx.err_at(format!("output `{out_name}`: expression produced no value"))
                    })?;
                    if actual_type_id != *out_type_id {
                        let expected = self.types.display_name(out_shape);
                        let got = self
                            .types
                            .entry_by_type_id(actual_type_id)
                            .map(|e| e.type_name.to_string())
                            .unwrap_or_else(|| "?".to_string());
                        return Err(ctx.err_at(format!(
                            "output `{out_name}`: type mismatch: expected `{expected}`, got `{got}`"
                        )));
                    }
                    let call_fn = self.types.entry_by_type_id(*out_type_id).expect("registered").call_dyn_fn;
                    CompiledOutputs::Single(call_fn)
                }
                TypeShape::Tuple(elements) if elements.is_empty() => {
                    // () is CEL's concrete unit type, a distinct leaf TypeId -- not DynTuple.
                    let actual_type_id = segment.peek_output_type_id().ok_or_else(|| {
                        ctx.err_at(format!("output `{out_name}`: expression produced no value"))
                    })?;
                    if actual_type_id != TypeId::of::<()>() {
                        return Err(ctx.err_at(format!(
                            "output `{out_name}`: type mismatch: expected `()`, got a non-`()` value"
                        )));
                    }
                    CompiledOutputs::EmptyTuple
                }
                TypeShape::Tuple(_) => {
                    let stack_info = segment.peek_stack_infos(1).first();
                    let matches = stack_info
                        .is_some_and(|info| tuple_shape_matches_associated(out_shape, &info.associated));
                    if !matches {
                        let actual = stack_info
                            .and_then(|info| self.shape_of_associated(&info.associated).ok())
                            .map(|s| self.types.display_name(&s))
                            .unwrap_or_else(|| "a non-matching value".to_string());
                        return Err(ctx.err_at(format!(
                            "output `{out_name}`: type mismatch: expected `{}`, got `{actual}`",
                            self.types.display_name(out_shape)
                        )));
                    }
                    CompiledOutputs::SingleTuple(self.types.element_descriptors_for(out_shape))
                }
            }
        } else {
            let arity = segment.peek_tuple_arity().unwrap_or(0);
            if arity != outputs.len() {
                return Err(ctx.err_at(format!(
                    "output expression has arity {arity} but method declares {} output(s)",
                    outputs.len()
                )));
            }
            let associated = segment.peek_stack_infos(1)[0].associated.clone();
            let mut extractors = Vec::with_capacity(outputs.len());
            for (i, ((out_name, _, out_shape), elem)) in outputs.iter().zip(&associated).enumerate() {
                if !element_shape_matches(out_shape, elem) {
                    return Err(ctx.err_at(format!(
                        "output {i} `{out_name}`: type mismatch: expected `{}`, got `{}`",
                        self.types.display_name(out_shape),
                        elem.type_name
                    )));
                }
                extractors.push(match out_shape {
                    TypeShape::Named(type_id) => {
                        let entry = self.types.entry_by_type_id(*type_id).expect("registered");
                        cel_runtime::DynExtractor::Scalar(*type_id, entry.extract_box_fn)
                    }
                    TypeShape::Tuple(_) => {
                        let table = self.types.element_descriptors_for(out_shape);
                        cel_runtime::DynExtractor::Tuple(Box::new(move |type_id: TypeId| {
                            table
                                .iter()
                                .find(|(tid, ..)| *tid == type_id)
                                .map(|(_, d, c, e)| (*d, *c, *e))
                        }))
                    }
                });
            }
            CompiledOutputs::Tuple(extractors)
        };
```

Update `build_method`'s signature from `inputs: Vec<(String, CellId, TypeId)>, outputs:
Vec<(String, CellId, TypeId)>` to `Vec<(String, CellId, TypeShape)>` for both (matching
`parse_cell_list`'s Task 7 change), and its `input_types`/`output_types` lines from `inputs.iter().
map(|(_, _, tid)| *tid).collect()` to `inputs.iter().map(|(_, _, shape)| cell_type_id(shape)).
collect()` (same for `outputs`), reusing Task 7's `cell_type_id` helper — `Method::new` itself is
unchanged, since it already takes plain `Vec<TypeId>` for both (`adam_rs` has no notion of
`TypeShape`).

Update the closure in `build_method` (the `f` returned to `Method::new`) to handle the two new
cases:

```rust
            match &compiled {
                CompiledOutputs::Single(call_fn) => Ok(vec![call_fn(seg, inputs_any)?]),
                CompiledOutputs::EmptyTuple => {
                    Ok(vec![Box::new(cel_runtime::DynamicSequence::from_dyn_elements(Vec::new())) as Box<dyn Any>])
                }
                CompiledOutputs::SingleTuple(table) => {
                    let leaf = |type_id: TypeId| {
                        table
                            .iter()
                            .find(|(tid, ..)| *tid == type_id)
                            .map(|(_, d, c, e)| (*d, *c, *e))
                    };
                    let seq = seg.call_dyn_as_dynamic_sequence(inputs_any, &leaf)?;
                    Ok(vec![Box::new(seq) as Box<dyn Any>])
                }
                CompiledOutputs::Tuple(extractors) => {
                    // Safety: every DynExtractor::Scalar extractor here is extract_box_impl::<T>
                    // (via TypeEntry::extract_box_fn), which clones rather than moves --
                    // satisfying call_dyn_tuple_mixed's contract, exactly like the pre-tuple
                    // call_dyn_tuple call site this replaces.
                    unsafe { seg.call_dyn_tuple_mixed(inputs_any, extractors) }
                }
            }
```

Update `parse_body_with_input_scope`'s `scope_data`/closure to dispatch per input cell's
`TypeShape`:

```rust
        let scope_data: Vec<(String, InputPush, usize)> = inputs
            .iter()
            .enumerate()
            .map(|(idx, (name, _, shape))| {
                let push: InputPush = match shape {
                    TypeShape::Named(type_id) => {
                        let fn_ptr = self.types.entry_by_type_id(*type_id).expect("registered").push_arg_fn;
                        InputPush::Scalar(fn_ptr)
                    }
                    TypeShape::Tuple(_) => {
                        InputPush::Tuple(self.types.associated_prototype(shape))
                    }
                };
                (name.clone(), push, idx)
            })
            .collect();
```

with a small `enum InputPush { Scalar(PushArgFn), Tuple(Vec<cel_runtime::AssociatedType>) }` and
the scope closure calling `fn_ptr(segment, *idx)` or `segment.push_arg_as_dynamic_sequence_tuple(*idx,
shape.clone())` accordingly (the closure needs to clone the prototype per call since
`push_arg_as_dynamic_sequence_tuple` consumes its `Vec<AssociatedType>` argument by value — this is
parse-time-only, called once per identifier reference in a method body, so the clone cost is
negligible).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-lang parser::`
Expected: PASS — every pre-existing test (including all of today's multi-output-method tests)
plus the 6 new ones from Step 1.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-lang/src/parser.rs
git commit -m "feat(adam-lang): unify multi-output methods with tuple-typed outputs; wire tuple inputs"
```

---

### Task 9: Real parser — `conditional` with a tuple-typed match cell

**Files:**
- Modify: `adam-lang/src/parser.rs`

**Interfaces:**
- Consumes: `AdamParser::eval_segment_boxed` (Task 7); everything else from Tasks 7–8.

Resolves the `// TODO(Task 9)` placeholder left in Task 7. A tuple-typed match cell is never
itself registered in `TypeRegistry` (every shape shares the one concrete storage type,
`DynamicSequence`), so its conditional is added via `Sheet::add_conditional::<DynamicSequence>`
directly rather than through a registry-stored `add_conditional_fn` — mirroring
`build_cell_from_segment`'s own `TypeShape::Tuple` arm in Task 7. Only the *branch key* parsing
needs to change: a branch's match value is now parsed the same way a cell initializer is (a full
`or_expression`, evaluated eagerly via `eval_segment_boxed`), instead of `ctx.consume_literal()` +
`parse_literal_as`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `adam-lang/src/parser.rs`:

```rust
#[test]
fn parse_conditional_with_tuple_typed_match_cell() {
    let mut sheet = parser()
        .parse_str(
            r#"
            sheet s {
                cell mode: (i32, i32) = (0, 0);
                cell x: f64 = 1.0;
                cell y: f64;
                conditional mode {
                    (0, 0) => { relationship { method [x] -> [y] { x } } },
                    _ => { relationship { method [x] -> [y] { x * 2.0 } } },
                }
            }
        "#,
        )
        .unwrap();
    sheet.propagate().unwrap();
    let (y_id, _) = sheet.cell_names["y"].clone();
    assert_eq!(*sheet.read::<f64>(y_id).unwrap(), 1.0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p adam-lang parser::tests::parse_conditional_with_tuple_typed_match_cell`
Expected: FAIL — the `TODO(Task 9)` placeholder rejects a tuple match cell.

- [ ] **Step 3: Implement**

Rewrite `parse_conditional_decl`'s branch-key parsing and its two dispatch points (named branch,
and the final `add_conditional_fn`/`add_conditional::<DynamicSequence>` call) to branch on
`match_shape` (the `TypeShape` half of `ctx.cell_names.get(&match_name)`, replacing the old
`match_type_id`):

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

        let mut branches: Vec<(Box<dyn Any>, Vec<RelationshipId>)> = Vec::new();
        let mut default_rel_ids: Vec<RelationshipId> = Vec::new();

        while !ctx.at_close_brace() {
            if matches!(ctx.peek_token(), Some(Token::Identifier(id)) if id == "_") {
                ctx.advance(); // consume `_`
                ctx.expect_punct("=>")?;
                ctx.expect_open_brace()?;
                let rel_ids = self.parse_branch_relationships(ctx)?;
                ctx.expect_close_brace()?;
                ctx.consume_punct(",");
                default_rel_ids = rel_ids;
                break; // default branch is always last
            }

            // Named branch: `or_expression "=>" "{" ... "}"` — an or_expression covers both a
            // bare literal (`0i32 =>`) and a tuple value (`(0, 0) =>`) via the same grammar cell
            // initializers already use.
            let branch_span = ctx.peek_span();
            let segment = self.parse_cel_or_expression(ctx)?;
            let (branch_shape, branch_val) = self.eval_segment_boxed(segment)?;
            if branch_shape != match_shape {
                return Err(ParseError::new(
                    format!(
                        "conditional branch: type mismatch: expected `{}`, got `{}`",
                        self.types.display_name(&match_shape),
                        self.types.display_name(&branch_shape)
                    ),
                    branch_span,
                ));
            }
            ctx.expect_punct("=>")?;
            ctx.expect_open_brace()?;
            let rel_ids = self.parse_branch_relationships(ctx)?;
            ctx.expect_close_brace()?;
            ctx.consume_punct(",");
            branches.push((branch_val, rel_ids));
        }
        ctx.expect_close_brace()?;

        match &match_shape {
            TypeShape::Named(type_id) => {
                let add_cond_fn: AddConditionalFn = self
                    .types
                    .entry_by_type_id(*type_id)
                    .ok_or_else(|| ParseError::new("match cell type not in TypeRegistry", match_span))?
                    .add_conditional_fn;
                add_cond_fn(&mut ctx.sheet, match_cell_id, branches, default_rel_ids)
                    .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
            }
            TypeShape::Tuple(_) => {
                let typed_branches: Vec<(Vec<cel_runtime::DynamicSequence>, Vec<RelationshipId>)> =
                    branches
                        .into_iter()
                        .map(|(val, rel_ids)| {
                            let seq = *val
                                .downcast::<cel_runtime::DynamicSequence>()
                                .expect("eval_segment_boxed: a Tuple shape always boxes a DynamicSequence");
                            (vec![seq], rel_ids)
                        })
                        .collect();
                ctx.sheet
                    .add_conditional::<cel_runtime::DynamicSequence>(
                        match_cell_id,
                        typed_branches,
                        default_rel_ids,
                    )
                    .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
            }
        }

        Ok(())
    }
```

**Note for the implementer:** confirm `adam_rs::Sheet::add_conditional::<T>`'s exact signature
against `type_registry.rs`'s existing `add_conditional_impl` (it wraps each branch's single value
as `vec![v]` before calling `sheet.add_conditional::<T>(cell, typed_branches, default)` — the
`TypeShape::Tuple` arm above mirrors that same `vec![seq]` wrapping) — adjust the exact call shape
if `add_conditional`'s real signature differs from what `add_conditional_impl` implies.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p adam-lang parser::`
Expected: PASS — the new test, plus every pre-existing conditional test (all scalar-keyed) still
passing unchanged.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-lang/src/parser.rs
git commit -m "feat(adam-lang): support a tuple-typed conditional match cell"
```

---

### Task 10: Full workspace verification

**Files:** none (verification only).

- [ ] **Step 1: Run the full test suite**

Run: `cargo test --workspace` and `cargo test --doc --workspace`.
Expected: PASS, zero compiler warnings.

- [ ] **Step 2: Run all three clippy invocations**

Run, in order:
```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: PASS.

- [ ] **Step 3: Format**

Run: `cargo fmt --all`.

- [ ] **Step 4: Update the crate root doc comment's grammar section**

`adam-lang/src/lib.rs`'s `# Grammar` section (referenced throughout this file's doc comments)
needs its `cell_decl`/`out_decl` productions updated to the new `type_expr`/`or_expression`-based
grammar, and a new `type_expr` production added, matching this plan's Task 1–2 grammar exactly.

- [ ] **Step 5: Final commit**

```bash
cargo fmt --all
git add -A
git commit -m "fix(adam-lang): address workspace-wide lint/warning findings; update grammar doc"
```
