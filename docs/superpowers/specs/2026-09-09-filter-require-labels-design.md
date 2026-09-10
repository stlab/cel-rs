# Filter and Requirement Label Cleanup

**Date:** 2026-09-09
**Branch:** worktree-adam-lang/filter-require-labels
**Status:** Implemented

## Summary

Two related grammar/API simplifications to `adam-lang`'s `cell_filter` and
`requirement` productions:

1. **Filters lose their label entirely**, at every layer. `cell_filter = "filter"
   identifier ":" expression.` becomes `cell_filter = "filter" expression.`. A cell can
   have at most one filter, so a per-filter name never disambiguated anything a caller
   couldn't already get from the cell's own identity — confirmed by call-graph analysis:
   `adam_rs::Sheet::filter_name` has zero callers outside adam-rs's and adam-lang's own
   test suites. The `name` concept is removed from `adam_rs`'s `Filter`/`FilterData` and
   from `Sheet::add_filter`'s signature, not just hidden by the DSL.

2. **Requirement labels become optional**, via a new leading-marker syntax:
   `requirement = identifier ":" expression ";".` becomes `requirement = [ "@"
   identifier ] expression ";".`. Unlike filters, a `require` block can hold multiple
   requirements per cell, and applications genuinely consume requirement names today
   (`adam-web-ui/src/inspector.rs`'s `compute_output_status` joins violated requirement
   names for display) — so the name stays, but becomes `Option<String>` end to end
   instead of a mandatory, uniqueness-checked string.

Both changes ripple from `adam-lang`'s grammar down through `adam_rs`'s public `Sheet`
API and back up through every consumer: `ez-adam`'s codegen, `adam-web-ui`, `begin`'s
bundled `.adm2` examples, and the `adam-lang-book` prose and its own example sheets.

---

## 1. Motivation

`filter identifier ":" expression` requires a sheet author to invent and type a name
that nothing downstream reads — dead ceremony on every filter clause. `requirement
identifier ":" expression` is not ceremony (the name lets an application report *which*
requirement failed), but making a sheet author name a requirement whose violation the
application never surfaces is unnecessary friction. Making the label optional, while
keeping it available for the requirements that do want app-facing reporting, matches
how the construct is actually used.

The two productions currently share `identifier ":" expression` shape, but the fix
diverges: a filter's label is deleted outright (grammar, AST, and the `adam_rs` runtime
API all lose the concept); a requirement's label survives, but the surface syntax
changes shape entirely, from a mandatory leading `identifier ":"` to an optional leading
`"@" identifier`.

### 1.1 Why `requirement` can't just make its existing `identifier ":"` optional

Both of adam-lang's two independent parsers (`ast_parser.rs`'s CST builder and
`parser.rs`'s direct-to-`Sheet` compiler) hand off the *body* expression to an embedded
`cel_parser::Parser` that owns the token stream outright for the duration of one
expression parse (`TokenCursor::take_tokens`/`set_tokens`). `TokenCursor` itself only
supports one token of lookahead (`std::iter::Peekable`). A bare leading identifier is
also always a valid, complete CEL expression on its own (a variable reference), so
`[identifier ":"] expression` is genuinely ambiguous at one token of lookahead: seeing
an identifier doesn't say whether it's a label or the entire body.

Two markers were considered and rejected before `@`:

- **Trailing `expression ["as" identifier] ";"`.** Parse the full expression first (no
  upfront ambiguity), then check for a trailing label keyword. Rejected: `as` is already
  CEL's cast-expression keyword (`cast_expression = unary_expression { "as" identifier
  }.`, binding inside `multiplicative_expression`), so `x >= 0 as nonneg` already
  reparses as `x >= (0 as nonneg)` — a real grammar collision, not just a style
  preference.
- **Leading `["[" identifier "]"] expression ";"`.** Rejected: `[a] == [b];` is a valid,
  unlabeled bool-typed requirement body (comparing two 1-element list literals), so a
  bracketed label needs the same lookahead-past-the-bracket the plain-identifier form
  needed — no improvement.

`@` is a valid single-character Rust/`proc_macro2` punctuation token (used in Rust's own
`x @ pattern` binding syntax) that is not used anywhere in `cel-parser`'s or
`adam-lang`'s existing grammar — confirmed by search. It can never be the first token of
a valid CEL expression, so `["@" identifier] expression ";"` is unambiguous at exactly
one token of lookahead: peek `@`, and if present, consume `@ identifier` as the label;
otherwise dispatch straight to expression parsing. No backtracking, no `TokenCursor`
changes.

---

## 2. Grammar

```text
cell_filter = "filter" expression.
requirement = [ "@" identifier ] expression ";".
```

Example:

```text
require {
    @nonneg x >= 0;
    y < 100;
}
```

---

## 3. AST (`adam-lang/src/ast.rs`)

- `CellFilter`: remove `name` and `name_span` fields entirely. Only `body` and `span`
  remain.
- `RequirementDecl`: `name: String` → `name: Option<String>`; `name_span: ExprSpan` →
  `name_span: Option<ExprSpan>`.
- Grammar doc comments on both structs, and on `Sheet`'s top-level grammar listing in
  `adam-lang/src/lib.rs`, updated to match §2.

---

## 4. Parsers

Both parsers carry this grammar independently and need the same shape of change.

### 4.1 `adam-lang/src/ast_parser.rs`

- `parse_cell_filter`: stop consuming a leading identifier/colon. The `filter` keyword
  is already consumed by the caller before this is called (existing precondition); this
  method now does nothing but delegate straight to `parse_cel_expression`.
- `parse_requirement`: peek the next token via `TokenCursor::peek_token`. If it's a
  `Punct` matching `@`, consume it, then `consume_ident()` for the label, recording
  `name: Some(name)` / `name_span: Some(point(span))`. Otherwise `name: None`,
  `name_span: None`. Either way, continue with `parse_cel_expression` for the body.

### 4.2 `adam-lang/src/parser.rs` (direct-to-`Sheet` compiler)

- `parse_cell_filter`: same shape of change — drop the `consume_ident()` +
  `expect_punct(":")` pair at the top; return `(adam_rs::Filter, ...)` without a name
  (return type drops the `String` element of its tuple, or drops the tuple wrapper
  entirely if a filter is the only thing it returns — check current callers when
  implementing).
- `parse_requirement`: same `@`-peek logic as §4.1, returning `(Option<String>,
  Requirement)` instead of `(String, Requirement)`. Diagnostic messages that currently
  interpolate the name (e.g. `` "requirement `{name}`: expected `bool`, got `{got}`" ``)
  drop the name segment when `None` — these errors already carry a source span via
  `ctx.err_at`, so the span, not the interpolated name, is what actually localizes the
  error.
- The `out`-declaration call site that currently builds `Vec<(&str, Requirement)>` for
  `Sheet::add_out` changes to `Vec<(Option<&str>, Requirement)>`.
- The call site that does `ctx.sheet.add_filter(out_cell, filter_name, filter)` drops
  the `filter_name` argument.

---

## 5. Formatter (`adam-lang/src/fmt.rs`)

- `write_cell`, `write_out`, `write_source`: change `" filter "` + name + `": "` + body
  to `" filter "` + body (no colon, no name).
- `write_requirement`: emit `"@"` + name + `" "` before the body only when
  `req.name.is_some()`; otherwise emit the body directly with no prefix.

---

## 6. `adam_rs` public API

Per explicit direction: name-strings should not be part of `adam_rs`'s public API for
sheets, filters, or requirements. Sheets are already referenced by instance; filters (at
most one per cell) are referenced by the cell's own `CellId`; requirements (any number
per cell) keep `RequirementId` as their identity, with the label reduced to optional
display metadata.

### 6.1 Filters (`adam-rs/src/filter.rs`, `adam-rs/src/sheet.rs`)

- `FilterData`: remove the `name` field.
- `Filter::new` / `from_fn_0` / `from_fn_1` / `from_fn_2` (and `range`, if it separately
  initializes `name`): drop the `name: String::new()` initialization.
- `Sheet::add_filter(&mut self, cell: CellId, filter: Filter) -> Result<(), Error>` —
  drop the `name: impl Into<String>` parameter and the "name is empty" branch of
  `Error::InvalidFilter`. `Error::InvalidFilter`'s doc comment drops that condition from
  its enumerated list.
- `Sheet::filter_name` — removed.

### 6.2 Requirements (`adam-rs/src/requirement.rs`, `adam-rs/src/sheet.rs`)

- `RequirementData.name: String` → `name: Option<String>`.
- `Sheet::add_requirement(&mut self, cell: CellId, name: Option<&str>, requirement:
  Requirement) -> Result<RequirementId, Error>` — signature changes from `name: impl
  Into<String>`. Drop the "name is empty" `InvalidRequirement` branch entirely (an
  absent name is no longer an error state). The "cell already has a same-named
  requirement" duplicate check only runs when both the new name and an existing
  requirement's name are `Some` and equal — two unlabeled requirements on the same cell
  never collide.
- `Sheet::add_out(&mut self, writer: Method, requirements: Vec<(&str, Requirement)>) ->
  Result<CellId, Error>` — `requirements` becomes `Vec<(Option<&str>, Requirement)>`.
- `Sheet::requirement_name(&self, id: RequirementId) -> Option<&str>` — signature
  unchanged; body becomes `self.requirements.get(id)?.name.as_deref()`. Now naturally
  returns `None` both for an invalid id and for a live-but-unlabeled requirement — the
  one existing production consumer, `adam-web-ui/src/inspector.rs`'s
  `compute_output_status` (`sheet.violated_requirements(cell).filter_map(|rid|
  sheet.requirement_name(rid))`), already treats `None` as "contributes nothing to the
  joined name list," so it needs no code change, only updated call sites where it
  constructs filters/requirements with names.

---

## 7. Downstream call sites

- **`ez-adam/src/codegen/ast_builder.rs::clamp_filter`** — currently builds `CellFilter {
  name: "clamp".to_string(), name_span: ExprSpan::for_text("clamp"), body, span }`
  directly; drop the two removed fields.
- **`adam-web-ui`, `begin`, `adam-rs` tests** — every `add_filter(cell, "name", filter)`
  call drops the name argument; every `add_requirement`/`add_out` call wraps its name
  argument in `Some("...")` or passes `None`.
- **`.adm2` files** (`begin/examples/*.adm2`, `adam-lang-book`'s bundled examples) —
  `filter name: expr` → `filter expr`; requirement lines `name: expr;` → `@name expr;`,
  preserving every existing label as-is (this is a syntax migration, not an editorial
  pass over which requirements deserve a name).
- **`adam-lang-book/book-src/filters.md`, `outputs.md`, `reference.md`** (and any other
  prose describing this grammar) — updated to the new syntax and to explain the
  requirement label's optionality.
- **`editors/vscode-adam-lang`'s TextMate grammar** — no label-aware patterns found by
  search; expected no-op, verified during implementation.
- **`adam-lsp`** — checked during implementation for any completion/hover text that
  quotes the old grammar.

---

## 8. Non-goals

- No change to `require`'s block structure, or to how many requirements a cell may
  carry.
- No change to filter/requirement *semantics* (write-time transform, derived-value
  diagnostic, violation reporting) — this is a syntax and identity-model cleanup only.
- No retroactive rewrite of `@`-labeled requirements back into some other display
  convention in `begin`'s UI — `adam-web-ui` already joins whatever names
  `requirement_name` returns; unlabeled requirements simply contribute nothing to that
  join, which is the desired behavior, not a new feature.

---

## 9. Testing

Contract-derived, per this repo's convention:

- `ast_parser.rs` / `parser.rs`: a filter clause with no label parses correctly (and a
  `filter identifier: expr` clause is now a parse error, not a name — replaces the
  existing "attaches a named filter" tests); a requirement with `@label` parses with
  `name: Some("label")`; a requirement with no `@` parses with `name: None`; a
  requirement whose body itself is a bare identifier (e.g. `is_valid;`) still parses as
  an unlabeled requirement, not a label with a missing body.
- `adam_rs::Sheet::add_filter`: no `name` parameter to test for emptiness; existing
  "already has a filter" / type-mismatch tests unaffected.
- `adam_rs::Sheet::add_requirement`: `None` name succeeds; two `None`-named requirements
  on the same cell both succeed (no false collision); two requirements with the same
  `Some` name on the same cell still errors; `requirement_name` returns `None` for an
  unlabeled live requirement (distinct from the existing "returns `None` for an invalid
  id" case).
- `fmt.rs`: round-trip a filter with no label; round-trip a requirement with and without
  `@label`.
- `adam-web-ui`: existing `compute_output_status`-family tests re-verified with a mix of
  labeled/unlabeled requirements feeding `invalid_output_requirement_names`.
