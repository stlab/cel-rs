# adam-lang Formatter — Design

## Goal

Implement Phase 4 of `docs/superpowers/specs/2026-07-17-pm-lang-language-server-design.md`
("Formatter"): a `cargo fmt`-style auto-formatter for `.adm2` (adam-lang) source, wired into
`adam-lsp`'s `textDocument/formatting` and enabled by default for format-on-save in
`editors/vscode-adam-lang`.

## Background

Phases 1–3 of the parent design are complete: `cel-parser::AstContext` builds a span-carrying
`Expr` tree; `adam-lang::AdamAstParser` builds a span-carrying `Sheet` tree on top of it, with
declaration-level error recovery and a `trivia::attach_trivia` pass that recovers `//`/`/* */`
comments (discarded by `proc_macro2`'s tokenizer) and attaches them as `leading_comment` on
top-level `SheetItem`s; `adam-lsp` publishes syntax/type diagnostics but does not yet handle any
LSP request beyond `shutdown`.

This spec covers only the formatter. It does not touch hover/goto-def/completion (design doc
Phase 5).

## Prerequisite fix: `Expr::If`'s implicit else is not distinguishable from an explicit one

`AstContext`'s `Expr::If` always has a concrete `else_branch: Box<Expr>` — when no `else` was
written, `is_if_expression` (cel-parser/src/lib.rs) synthesizes one by pushing a `Literal::Unit`
node whose span is `self.last_span`, i.e. the then-branch's closing `}` token, not an actual `()`
token in the source. A formatter that re-slices literal text from spans (below) would print that
synthetic node's source text verbatim — literally the character `}` — if it tried to render an
omitted `else` at all.

Fix at the AST level rather than patching around it in the formatter (per this repo's "prefer
redesigning components over layering on top" project status): change

```rust
else_branch: Box<Expr>
```

to

```rust
else_branch: Option<Box<Expr>>
```

`AstContext::join2` takes `None` for `else_fragment` instead of receiving a synthesized
Unit-literal fragment; `is_if_expression` passes `None` when no `else`/`else if` was parsed.
`cel_parser::ty::check_expr`'s `If` handling treats `None` as `Ty::unit()` for the else branch's
type (its current behavior when the branch happens to be a Unit literal), so its type-checking
result is unchanged. `ast.rs`'s `if_without_else_has_a_unit_else_branch` test is renamed/rewritten
to assert `else_branch.is_none()`. No other consumer depends on the old always-`Box` shape (only
`cel-parser`'s own tests and `ty.rs` reference `else_branch`).

This is a small, contained change confined to `cel-parser` (the `AstContext`/`Expr` phase 2
surface), landed as the first task of the implementation plan before the formatter is written
against it.

## `cel-parser` expression formatter

New module `cel-parser/src/fmt.rs`, exported as `cel_parser::format_expr`:

```rust
pub fn format_expr(source: &str, expr: &Expr) -> String
```

- **Literals are re-emitted via `Span::source_text()`** on the node's `ExprSpan`, not synthesized
  from the `Literal` enum — this is what `Expr`'s own module doc already flags as necessary
  (`Literal` can't distinguish `1920.0` from `1920.0f64`, or a byte literal from a `u8`-suffixed
  integer literal, but the original span can). If `source_text()` returns `None` (possible for
  spans without a live source file, e.g. in unit tests using `Span::call_site()`), fall back to
  formatting the typed `Literal` value directly — this only affects hand-built `Expr` trees in
  tests, never a real parse from source text.
- **Idents/operators are synthesized**, not re-sliced — normalizing to single-space-around-operator
  spacing is the point of the exercise, not something to preserve from source.
- **Parenthesization is precedence-aware**, using a small table mirroring the grammar's twelve
  binding-strength levels (`or_expression` < `and_expression` < `comparison_expression` <
  `bitwise_or_expression` < `bitwise_xor_expression` < `bitwise_and_expression` <
  `bitwise_shift_expression` < `additive_expression` < `multiplicative_expression` <
  `unary_expression` < `postfix_expression` < `primary_expression`, per `lib.rs`'s grammar doc
  comment). Printing a child expression inside a parent context adds parens only when the child's
  level is looser than (or, for non-associative operators, equal-but-wrong-side of) what the
  parent position requires — e.g. `(1 + 2) * 3` re-emits its parens because `+` is looser than
  `*`'s left operand slot, but `1 + 2 * 3` does not, because the nested `*` is already tighter than
  its `+`-operator parent needs.
- **No line-wrapping.** Every expression is emitted on one line regardless of length; the design
  doc's 100-column target is aspirational only for this phase — real usage (`begin/assets/demo.adm2`)
  is exclusively short one-liners, and column-aware greedy line-breaking is deferred until an
  actual file needs it.
- `Expr::Tuple` reprints as `(a, b)`; a 1-tuple reprints with its trailing comma (`(a,)`), the
  syntax that distinguishes it from a grouped expression.
- `Expr::If`/`Expr::Logical` reprint as `if cond { then } else { else }` / `a || b`, `a && b`,
  never desugared (matches the `Expr` module doc's stated reason for keeping `Logical` distinct
  from `If`). An `Expr::If` with `else_branch: None` omits the `else` clause entirely.

## `adam-lang` trivia generalization

`adam-lang/src/trivia.rs`'s `attach_trivia` currently walks only `Sheet.items`. Generalize its
gap-walking loop into a private helper generic over "a slice of items each exposing a span and a
settable leading-comment/blank-line pair," and call it for every sibling list in the tree, not
just the top level:

- `Sheet.items` (already covered)
- `RelationshipDecl.methods`
- `ConditionalDecl.branches`, plus its `default`'s relationship list
- Each `ConditionalBranch.relationships`

This requires adding `leading_comment: Option<String>` to `MethodDecl` and `ConditionalBranch` —
the only two AST node types that don't already have it (`CellDecl`, `RelationshipDecl`,
`ConditionalDecl`, and `SheetItem::Error` already do).

Alongside `leading_comment`, the same generalized pass adds a new `blank_line_before: bool` field
to every one of those node types (mirroring `leading_comment`'s placement), recording whether the
gap before this item (before its leading comment, if any) contained at least one fully blank line.
This is what lets the formatter later collapse runs of blank lines to at most one while not
fabricating separators that were never there — `demo.adm2` packs `cell` declarations tight but
separates `relationship` blocks with a blank line, so preserving presence/absence (not a fixed
rule) is required for realistic output. The first item in any block never has a blank line before
it (no blank line is ever inserted directly after an opening `{`), matching `attach_trivia`'s
existing `for i in 1..items.len()` boundary.

**Known limitation, unchanged in spirit from the existing trivia docs:** a comment or blank line
in the gap between the last item of a block and that block's closing `}` (nothing follows it) is
not attached to anything and is dropped on format, the same way `attach_trivia` already only
attaches a comment when something follows it.

Block vs. line comment style is not preserved — `trivia.rs` already collapses both into one
`String` with no memory of which delimiter was used, so every recovered comment re-emits as one or
more `//` lines regardless of its original form. This is an existing normalization, not a new one
introduced by the formatter.

## `adam-lang` sheet formatter

New module `adam-lang/src/fmt.rs`, exported as `adam_lang::format_sheet`:

```rust
pub fn format_sheet(source: &str, sheet: &ast::Sheet) -> String
```

Walks `Sheet` top-down in declaration order, emitting:

- `sheet name {` / closing `}`.
- `cell name: type = literal;` (or whichever of `type`/`initializer` is present) — the literal is
  re-emitted via source-text re-slicing exactly as `cel_parser::fmt::format_expr` does for CEL
  literals, for the same reason (preserve exact numeric/suffix notation).
- `relationship [name] { method_decl* }`, one `method [inputs] -> [outputs] { body }` per line,
  `body` delegated to `cel_parser::fmt::format_expr`.
- `conditional match_name { branch* [default] }`, one `literal => { relationship_decl* }` per
  branch (trailing comma per branch, matching the grammar), and `_ => { ... }` last if a default
  branch is present.
- 4-space indentation per nesting level, opening braces on the same line, `leading_comment` lines
  and `blank_line_before` separators emitted ahead of each item exactly as recovered.

`SheetItem::Error` placeholders are never reached in normal operation — see below.

## `adam-lsp` wiring

`dispatch.rs`:

- `ServerCapabilities.document_formatting_provider = Some(OneOf::Left(true))`.
- `main_loop`'s `Message::Request(req)` arm gains a case for
  `req.method == "textDocument/formatting"`: parse `source`, and
  - if `AdamAstParser::parse_str` returns `Err`, or the parsed `Sheet.errors` is non-empty, respond
    with an empty edit list (`vec![]`) — **refuse to format code that doesn't parse cleanly**,
    matching `rustfmt`'s behavior of declining to reformat code it can't fully understand, rather
    than guessing at malformed input or reformatting only the recovered fragments.
  - otherwise, call `attach_trivia` (now recursive, see above) then `format_sheet`, and respond
    with a single `TextEdit` replacing the whole document range with the formatted text.

Handler-level unit tests call the formatting logic directly against source strings (same style as
`diagnostics.rs`'s existing tests), plus one or two real stdio-transport tests exercising
`textDocument/formatting` end-to-end, matching the parent design doc's stated LSP testing
strategy.

## `editors/vscode-adam-lang` wiring

No new client-side provider code is needed: `vscode-languageclient`'s `LanguageClient`
automatically proxies VS Code's document-formatting command to any server that advertises
`documentFormattingProvider`. Add a `configurationDefaults` entry in `package.json` for the
`adam-lang` language ID setting `"editor.formatOnSave": true`, and a short mention in the README's
"Trying it out" section.

## Testing strategy

Following this repo's existing convention (no file-based test fixtures exist anywhere in the
workspace today — `cel-parser`/`adam-lang`'s own test suites are all inline string literals),
golden-file-style tests are inline input/expected string-literal pairs within each module's
`#[cfg(test)]`, not external fixture files:

- `cel-parser/src/fmt.rs`: one test per precedence pair (mirroring `op_table`'s existing test
  matrix approach) asserting minimal-parens output; literal-notation-preservation tests (e.g.
  `1920.0` stays `1920.0`, not `1920`); `format(format(x)) == format(x)` idempotency tests; tuple/
  if/logical reprint tests.
- `adam-lang/src/trivia.rs`: new tests asserting comment attachment inside a relationship's
  methods and a conditional's branches, plus `blank_line_before` detection/collapse tests
  (including the "collapse 2+ blank lines to exactly 1" and "no blank line fabricated where none
  existed" cases).
- `adam-lang/src/fmt.rs`: golden input/expected pairs covering a full sheet with cells,
  relationships, and conditionals (e.g. a hand-written analogue of `demo.adm2`); idempotency;
  comment-preservation at every nesting level trivia now covers.
- `adam-lsp/src/dispatch.rs`: unit tests for the refuse-to-format-on-syntax-error case and the
  happy-path single-edit response, plus the diagnostics-style stdio round-trip test.

## Explicitly deferred / out of scope

- Column-aware line-wrapping (see above).
- Preserving block-vs-line comment style, and comments/blank lines in the trailing gap before a
  block's closing `}` (both existing normalizations/limitations, not newly introduced).
- A standalone formatting CLI — the parent design doc allows for one "later if wanted"; `format_sheet`
  and `format_expr` are plain functions, so nothing here blocks adding one, but it is not part of
  this phase.
- Range formatting (`textDocument/rangeFormatting`) — only whole-document formatting is wired up.
