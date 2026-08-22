# adam-lang Syntax Revision Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rewrite adam-lang's grammar per the design spec: `relationship` → `relate` with
deduced (not `[bracket]`-listed) inputs everywhere, `out`'s writer becomes a direct `:=`
initializer with a `require { ... }` validation block replacing repeated `condition` blocks,
and the corresponding `adam-rs` `Condition` API family renames to `Requirement`. The
`cell ... := expr;` sugar described in the spec is explicitly deferred — **not** built in this
pass (see spec's "Explicitly out of scope" section).

**Architecture:** The rewrite touches two independent adam-lang parsers that share no code
(the "live" `parser.rs`, which builds a real `adam_rs::Sheet` directly, and the lossless
CST parser behind the formatter: `ast.rs`/`ast_parser.rs`/`fmt.rs`/`trivia.rs`), plus a small
lexer addition (`cel-parser`), a mechanical rename in `adam_rs`, and cosmetic updates to the
VS Code syntax file and `begin`'s bundled example sheets. In the live parser, every deduced-input
construct (a conditional's match expression, a `relate` binding's RHS, an `out` initializer, a
`require`ment body) shares one grow-on-demand identifier-scope mechanism, already implemented
for conditionals as `parse_match_expr`; this plan extracts its scope-building core into a shared
`parse_deduced_expr` helper and extracts `parse_method_body`'s output-shape dispatch into a shared
`compile_outputs` helper, so `relate`'s new multi-output `binding` production reuses both instead
of duplicating either. The CST parser is untyped (never resolves identifiers against declared
cells), so its side of the rewrite needs no equivalent scope mechanism — it's a much simpler,
mostly mechanical grammar-shape change.

**Tech Stack:** Rust workspace (`cel-parser`, `adam-rs`, `adam-lang`, `begin`,
`editors/vscode-adam-lang`).

**Spec:** [docs/superpowers/specs/2026-08-19-adam-lang-syntax-design.md](../specs/2026-08-19-adam-lang-syntax-design.md)

## Global Constraints

- `cargo fmt --all` before every commit (enforced by pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` (incl. `cargo test --doc --workspace`) must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`, `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`, and `cargo clippy -p begin --all-targets -- -D warnings` must all pass with zero warnings.
- Every public function needs a contract-style `///` doc comment (Summary / Preconditions / `# Errors` / Postconditions / Complexity, as applicable) per the project's CLAUDE.md convention.
- Unit tests are derived from contract and public interface only, not implementation.
- No back-compat/migration path: this project has no clients yet. Every existing `.adam`-syntax
  test string and fixture gets **rewritten** to the new grammar, not dual-supported.
- The `cell ... := expr;` sugar is **out of scope** for this plan — do not implement it. Only
  `cell_decl`'s existing `= or_expression` initializer form is touched by this plan (not at all,
  in fact — `CellDecl`'s grammar/struct is unchanged end to end).
- `->` stays a valid lexer token (`cel-parser/src/lex_lexer.rs`'s `is_compound_operator` table
  keeps its `('-', '>')` entry) even though no adam-lang production references it anymore after
  this plan — `cel-parser` is a shared, adam-lang-agnostic crate, and removing lexer-level token
  support is out of scope and unnecessary (nothing breaks by leaving an unused capability there).

---

## Task 1: `cel-parser` — add the `:=` compound token

**Files:**
- Modify: `cel-parser/src/lex_lexer.rs`

**Interfaces:**
- Produces: `Token::Punct { op: ":=".to_string(), .. }` from adjacent `:`/`=` source
  characters — consumed by Task 3 and Task 4's `expect_punct(":=")`/`cursor.expect_punct(":=")`
  call sites.

- [ ] **Step 1: Write the failing test**

Add to `cel-parser/src/lex_lexer.rs`'s `#[cfg(test)] mod tests` (near the existing
`"->"`/`"=>"` compound-operator tests):

```rust
    #[test]
    fn walrus_operator_lexes_as_one_compound_token() {
        let stream: TokenStream = ":=".parse().unwrap();
        let mut lexer = LexLexer::new(stream.into_iter());
        let token = lexer.next().expect("one token");
        assert!(matches!(
            token,
            Token::Punct { ref op, .. } if op == ":="
        ));
        assert!(lexer.next().is_none());
    }

    #[test]
    fn colon_followed_by_non_equals_stays_two_separate_tokens() {
        let stream: TokenStream = ": x".parse().unwrap();
        let mut lexer = LexLexer::new(stream.into_iter());
        let first = lexer.next().expect("colon token");
        assert!(matches!(first, Token::Punct { ref op, .. } if op == ":"));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p cel-parser walrus_operator_lexes_as_one_compound_token --lib`
Expected: FAIL — the lexer currently emits `:` and `=` as two separate `Token::Punct`s, so
`lexer.next()` returns `Token::Punct { op: ":", .. }` (not `":="`) and `lexer.next()` after that
is `Some`, not `None`.

- [ ] **Step 3: Add the compound-operator table entry**

In `cel-parser/src/lex_lexer.rs`, change `is_compound_operator` (~line 147):

```rust
    fn is_compound_operator(first: char, second: char) -> bool {
        matches!(
            (first, second),
            ('&', '&')
                | ('|', '|')
                | ('=', '=')
                | ('!', '=')
                | ('<', '=')
                | ('>', '=')
                | ('<', '<')
                | ('>', '>')
                | ('-', '>')
                | ('=', '>')
        )
    }
```

to:

```rust
    fn is_compound_operator(first: char, second: char) -> bool {
        matches!(
            (first, second),
            ('&', '&')
                | ('|', '|')
                | ('=', '=')
                | ('!', '=')
                | ('<', '=')
                | ('>', '=')
                | ('<', '<')
                | ('>', '>')
                | ('-', '>')
                | ('=', '>')
                | (':', '=')
        )
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser walrus_operator_lexes_as_one_compound_token colon_followed_by_non_equals_stays_two_separate_tokens --lib`
Expected: PASS.

- [ ] **Step 5: Run the full `cel-parser` test suite**

Run: `cargo test -p cel-parser`
Expected: PASS — this is a strictly additive table entry; no existing token combination changes
meaning (`:` alone, `=` alone, and `==`/`=>` continue to lex exactly as before, since a compound
match only ever fires when *both* characters are adjacent with no intervening whitespace, per
`proc_macro2::Spacing`).

- [ ] **Step 6: Format and lint**

Run: `cargo fmt --all` then `cargo clippy -p cel-parser --all-targets -- -D warnings`.
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add cel-parser/src/lex_lexer.rs
git commit -m "feat(cel-parser): lex := as one compound token"
```

---

## Task 2: `adam-rs` — rename the `Condition` family to `Requirement`

**Files:**
- Rename: `adam-rs/src/condition.rs` → `adam-rs/src/requirement.rs`
- Modify: `adam-rs/src/lib.rs`
- Modify: `adam-rs/src/output.rs`
- Modify: `adam-rs/src/sheet.rs`
- Modify: `adam-rs/src/error.rs`

**Interfaces:**
- Produces: `adam_rs::{Requirement, RequirementId}` (renamed from `Condition`/`ConditionId`) —
  consumed by Task 4 (`adam-lang/src/parser.rs`'s `parse_requirement`/`parse_out_decl`).

This is a pure, mechanical rename — no behavior changes, no signature shape changes beyond the
names themselves. The unrelated `Conditional`/`ConditionalId`/`add_conditional`/branch-selection
family is untouched throughout.

- [ ] **Step 1: Rename the file and its contents**

Rename `adam-rs/src/condition.rs` to `adam-rs/src/requirement.rs`. Within it, rename every
occurrence:
- `Condition` → `Requirement` (struct, all doc comments, all test names/bodies)
- `ConditionId` → `RequirementId`
- `ConditionData` → `RequirementData`
- `ConditionFn` → `RequirementFn` (the private type alias)
- Module doc comment "Named boolean checks attached to outputs." stays as-is; update its body
  text's `[Condition]`/`Sheet::add_output` references (`[Condition]` → `[Requirement]`).
- Test names: `condition_id_is_copy` → `requirement_id_is_copy`,
  `condition_new_stores_types_and_cell_ids` → `requirement_new_stores_types_and_cell_ids`,
  `from_fn_1_stores_correct_type_ids`/`from_fn_2_stores_correct_type_ids` stay (already
  type-agnostic names), updating only the local `let condition = Condition::new(...)`
  variable/type references inside them to `requirement`/`Requirement`.

- [ ] **Step 2: Update `adam-rs/src/lib.rs`**

Change:

```rust
pub mod condition;
pub mod conditional;
```

to:

```rust
pub mod conditional;
pub mod requirement;
```

(Alphabetized; `conditional` sorts before `requirement`.)

Change:

```rust
pub use condition::{Condition, ConditionId};
pub use conditional::{ConditionalId, MatchExpr};
```

to:

```rust
pub use conditional::{ConditionalId, MatchExpr};
pub use requirement::{Requirement, RequirementId};
```

Update the module doc's example and prose (~lines 36-59):

```rust
//! # Outputs and conditions
//!
//! An output is a terminal cell written by a single method, with named conditions
```

to:

```rust
//! # Outputs and requirements
//!
//! An output is a terminal cell written by a single method, with named requirements
```

and:

```rust
//! use adam_rs::{Condition, Method, Sheet};
```
```rust
//!             Condition::from_fn_2([area, max_area], |a: &i32, max: &i32| Ok(a <= max)),
```

to:

```rust
//! use adam_rs::{Requirement, Method, Sheet};
```
```rust
//!             Requirement::from_fn_2([area, max_area], |a: &i32, max: &i32| Ok(a <= max)),
```

(Run `cargo doc --lib --no-deps -p adam-rs` after this step, or wait for Task 7's workspace-wide
doc build — either way, this doctest is exercised by `cargo test --doc --workspace`, so a typo
here surfaces as a doctest failure, not silently.)

- [ ] **Step 3: Update `adam-rs/src/output.rs`**

Change:

```rust
//! [`crate::condition::Condition`]s checked after every `Sheet::propagate`. An output's
```
```rust
//! conditional, condition, or output. See [`crate::sheet::Sheet::add_output`].
```
```rust
use crate::condition::ConditionId;
```
```rust
    /// This output's conditions, in declaration order.
    pub(crate) conditions: Vec<ConditionId>,
```

to:

```rust
//! [`crate::requirement::Requirement`]s checked after every `Sheet::propagate`. An output's
```
```rust
//! conditional, requirement, or output. See [`crate::sheet::Sheet::add_output`].
```
```rust
use crate::requirement::RequirementId;
```
```rust
    /// This output's requirements, in declaration order.
    pub(crate) requirements: Vec<RequirementId>,
```

- [ ] **Step 4: Update `adam-rs/src/sheet.rs`**

Change the import (~line 13):

```rust
    condition::{Condition, ConditionData, ConditionId},
```

to:

```rust
    requirement::{Requirement, RequirementData, RequirementId},
```

(Keep this line in whatever alphabetized position the surrounding `use` block's convention
requires relative to the `conditional::{...}` import on the next line.)

Rename the `Sheet` struct's fields (~lines 65-70):

```rust
    /// All conditions registered on this sheet, across all outputs.
    conditions: SlotMap<ConditionId, ConditionData>,
    /// Conditions that evaluated `false` as of the last `propagate()` call, grouped by
    /// output. Sparse: an output with no entry had all its conditions hold. Not
```

to:

```rust
    /// All requirements registered on this sheet, across all outputs.
    requirements: SlotMap<RequirementId, RequirementData>,
    /// Requirements that evaluated `false` as of the last `propagate()` call, grouped by
    /// output. Sparse: an output with no entry had all its requirements hold. Not
```

and its initializer (~line 105): `conditions: SlotMap::with_key(),` → `requirements: SlotMap::with_key(),`.

Rename `last_violated`'s doc comment and every downstream reference from `conditions`/`ConditionId`
to `requirements`/`RequirementId` — grep this file for `\bcondition` (case-sensitive, word-boundary,
to avoid touching `conditional`/`Conditional`) and update every hit in this exhaustive list (each
one already located precisely by this plan's own research; verify none were missed by re-grepping
after this step and confirming zero remaining case-sensitive `\bcondition\b`/`\bCondition\b` hits
that aren't part of `conditional`/`Conditional`):

- `add_output`'s parameter (~line 456): `conditions: Vec<(&str, Condition)>` → `requirements: Vec<(&str, Requirement)>`
- Its doc comment (~lines 428-452): every "condition(s)" → "requirement(s)" (e.g. "together with
  zero or more named conditions" → "named requirements"; "a condition's inputs" → "a requirement's
  inputs"; `Error::InvalidOutput — ... a condition name is empty, or two conditions share a name`
  → "a requirement name is empty, or two requirements share a name"; `Error::TerminalCell — a
  condition input` → "a requirement input"; the complexity note "k is the number of conditions" →
  "k is the number of requirements")
- The body's local variable `conditions`/`condition` (~lines 456-511): rename to
  `requirements`/`requirement` throughout, including `self.conditions.insert(ConditionData { ... })`
  → `self.requirements.insert(RequirementData { ... })`, and `self.outputs[output_id].conditions`
  → `self.outputs[output_id].requirements` (matching Step 3's field rename)
- `output_conditions` (~line 526) → `output_requirements`, its doc comment's "conditions
  registered on output" → "requirements registered on output", body `self.outputs.get(id).map(|o|
  o.conditions.as_slice())` → `o.requirements.as_slice()`
- `condition_name` (~line 533) → `requirement_name`, param/body `id: ConditionId` →
  `id: RequirementId`, `self.conditions.get(id)` → `self.requirements.get(id)`
- `condition_output` (~line 540) → `requirement_output`, same `ConditionId`/`self.conditions` rename
- `condition_inputs` (~line 547) → `requirement_inputs`, same rename
- `violated_conditions` (~line 568) → `violated_requirements`, its doc comment's "condition" →
  "requirement" throughout, return type `impl Iterator<Item = ConditionId>` →
  `impl Iterator<Item = RequirementId>`
- `condition_contributing_cells` (~line 684) → `requirement_contributing_cells`, param
  `id: ConditionId` → `id: RequirementId`, body `self.conditions.get(id)` → `self.requirements.get(id)`
- `output_relevant_cells`'s doc comment/body (~lines 708-719): "condition_contributing_cells" →
  "requirement_contributing_cells", "violated_conditions" → "violated_requirements"
- `propagate()`'s Phase 6 (~lines 1046-1122): doc comment "Condition evaluation" →
  "Requirement evaluation", "`Sheet::violated_conditions`" → "`Sheet::violated_requirements`",
  body's `for (condition_id, condition) in self.conditions.iter()` →
  `for (requirement_id, requirement) in self.requirements.iter()`, and every subsequent use of
  `condition`/`condition_id` in that loop body renamed to `requirement`/`requirement_id`
- Any remaining case-sensitive `\bcondition\b`/`\bCondition\b`/`\bConditionId\b` hit not listed
  above — there should be none; this list was built from an exhaustive grep of the file, but
  re-verify before moving on.

Do **not** touch anything spelled `conditional`/`Conditional`/`ConditionalId`/`add_conditional`/
`conditional_relationships`/`conditionals`/`conditional_match_cells`/`conditional_branch_count`/
`conditional_branch_relationships`/`conditional_default_relationships`/`conditional_active_branch`/
`evaluate_match_source`/`match_eq_fn` — these belong to the unrelated, unchanged branch-selection
family.

Update the test module's imports/helpers (~line 1426):

```rust
        ConditionalId, Error, MatchExpr, Method, Sheet, cell::CellId, relationship::RelationshipId,
```

This line only names `Conditional`-family items plus unrelated ones — confirm it needs no change
(it doesn't reference `Condition`/`ConditionId` at all), but double check by compiling after this
step.

- [ ] **Step 5: Update `adam-rs/src/error.rs`**

Change (~lines 54-57):

```rust
    /// An `add_output` call is structurally invalid: the writer method does not have
    /// exactly one output cell, a condition has an empty name, two conditions in the same
    /// call share a name, or a condition's `inputs` and `input_types` lengths differ.
    InvalidOutput,
```

to:

```rust
    /// An `add_output` call is structurally invalid: the writer method does not have
    /// exactly one output cell, a requirement has an empty name, two requirements in the same
    /// call share a name, or a requirement's `inputs` and `input_types` lengths differ.
    InvalidOutput,
```

Change (~lines 59-62):

```rust
    /// A cell belonging to an existing output (see `Sheet::add_output`) was referenced as
    /// an input to a relationship, conditional, condition, or a second output; was the
    /// target of `Sheet::write`; or an `add_output` call tried to reuse a cell that already
    /// had a relationship or conditional referencing it before becoming an output.
    TerminalCell,
```

to:

```rust
    /// A cell belonging to an existing output (see `Sheet::add_output`) was referenced as
    /// an input to a relationship, conditional, requirement, or a second output; was the
    /// target of `Sheet::write`; or an `add_output` call tried to reuse a cell that already
    /// had a relationship or conditional referencing it before becoming an output.
    TerminalCell,
```

- [ ] **Step 6: Build and fix any remaining compile errors**

Run: `cargo build -p adam-rs 2>&1 | head -80`

Fix any remaining `Condition`/`ConditionId`/`condition`-named reference the above steps missed
(the compiler will name every one precisely). Repeat until clean.

- [ ] **Step 7: Run the full `adam-rs` test suite**

Run: `cargo test -p adam-rs`
Expected: PASS — every test's assertions are unchanged; only names changed.

- [ ] **Step 8: Format and lint**

Run: `cargo fmt --all` then `cargo clippy -p adam-rs --all-targets -- -D warnings`.
Expected: clean.

- [ ] **Step 9: Confirm `adam-lang` still fails to build for the *expected* reason**

Run: `cargo build -p adam-lang 2>&1 | head -30`
Expected: FAIL — `adam-lang/src/parser.rs`'s `use adam_rs::{CellId, Condition, MatchExpr, ...}`
no longer resolves (`Condition` doesn't exist in `adam_rs` anymore). This confirms Step 1-8
landed correctly; Task 4 fixes this import as part of its own rewrite (don't fix it here — it's
folded into Task 4's grammar changes, not a bare rename, since `parser.rs`'s `Condition`-typed
code is being restructured, not just renamed).

- [ ] **Step 10: Commit**

```bash
git add adam-rs/src/requirement.rs adam-rs/src/lib.rs adam-rs/src/output.rs adam-rs/src/sheet.rs adam-rs/src/error.rs
git status --short adam-rs/src/condition.rs  # should show nothing (git mv tracked the rename)
git commit -m "refactor(adam-rs): rename the Condition family to Requirement"
```

---

## Task 3: `adam-lang` CST parser, formatter, and trivia — new grammar (untyped path)

**Files:**
- Modify: `adam-lang/src/ast.rs`
- Modify: `adam-lang/src/ast_parser.rs`
- Modify: `adam-lang/src/fmt.rs`
- Modify: `adam-lang/src/trivia.rs`
- Modify: `adam-lang/src/lib.rs` (grammar doc, at the end — see Task 7)

**Interfaces:**
- Produces: `ast::RelateDecl { bindings: Vec<BindingDecl>, .. }`,
  `ast::BindingDecl { outputs: Vec<(String, ExprSpan)>, body: cel_parser::Expr, .. }`,
  `ast::OutDecl { initializer: cel_parser::Expr, require: Option<RequireBlock>, .. }`,
  `ast::RequireBlock { requirements: Vec<RequirementDecl>, .. }`,
  `ast::RequirementDecl { name: String, body: cel_parser::Expr, .. }` — consumed only within
  this task (`ast_parser.rs`, `fmt.rs`, `trivia.rs`); the live parser (Task 4) is a fully
  independent implementation and does not use `ast::*` types at all.

This task is fully independent of Task 4 (different files, no shared code) but must land after
Task 1 (needs the `:=` token) and can proceed in parallel with Task 2 (no `adam_rs` dependency —
this parser is untyped and never touches `adam_rs::Condition`/`Requirement`).

**Decisions locked in before writing code** (so later steps don't re-litigate them):
- `SheetItem::Relationship(RelationshipDecl)` → `SheetItem::Relate(RelateDecl)` (variant renamed
  to match the new keyword, matching how `Out`/`Conditional`/`Cell` already match their keywords).
- `MethodDecl` (today: `inputs`/`outputs` cell-lists + body) is replaced by `BindingDecl`
  (`outputs` list only — no `inputs` field at all, since this untyped parser never resolves
  identifiers, so it has nothing to record for a deduced input list).
- `ConditionDecl` (today: name + `inputs` cell-list + body) is replaced by `RequirementDecl`
  (name + body only — same reasoning, no `inputs` field).
- `OutDecl` drops its own outer `{ }` block entirely (the new grammar is `;`-terminated, flat) —
  it no longer implements a "container with its own closing brace" trivia role. Its old
  `writer: OutMethodDecl` field is replaced by a flat `initializer: cel_parser::Expr` field (no
  `OutMethodDecl` struct survives — delete it). Its old `conditions: Vec<ConditionDecl>` field is
  replaced by `require: Option<RequireBlock>`, where `RequireBlock` is the new brace-delimited
  container (mirroring how `ConditionalDecl.default: Option<DefaultBranch>` already models an
  optional brace-delimited sub-block).
- `ConditionalBranch`/`DefaultBranch`'s `relationships: Vec<RelationshipDecl>` field **keeps its
  existing field name** (`relationships: Vec<RelateDecl>`, only the element type renames) — this
  is an internal Rust field name, not adam-lang surface syntax, and renaming it too would triple
  this task's diff for no user-visible benefit.

- [ ] **Step 1: Write the failing tests for the new AST shapes**

Replace `adam-lang/src/ast.rs`'s existing `sheet_item_span_reads_the_relationship_variant` test
with:

```rust
    #[test]
    fn sheet_item_span_reads_the_relate_variant() {
        let span = point(Span::call_site());
        let item = SheetItem::Relate(RelateDecl {
            bindings: Vec::new(),
            leading_comment: None,
            doc_comment: None,
            blank_line_before: false,
            trailing_comment: None,
            blank_line_before_close: false,
            open_brace_span: span,
            span,
        });
        assert_eq!(format!("{:?}", item.span()), format!("{span:?}"));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p adam-lang sheet_item_span_reads_the_relate_variant --lib`
Expected: FAIL to compile — `RelateDecl`/`SheetItem::Relate` don't exist yet.

- [ ] **Step 3: Rewrite `adam-lang/src/ast.rs`'s struct definitions**

Change the `SheetItem` enum (~lines 62-70):

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
```

to:

```rust
pub enum SheetItem {
    /// A `cell` declaration.
    Cell(CellDecl),
    /// A `relate` declaration.
    Relate(RelateDecl),
    /// A `conditional` declaration.
    Conditional(ConditionalDecl),
    /// An `out` declaration.
    Out(OutDecl),
```

Update every match arm in `SheetItem::span`/`set_leading_comment`/`set_blank_line_before`/
`set_doc_comment` (~lines 92-155): `SheetItem::Relationship(r) => ...` → `SheetItem::Relate(r) =>
...` (field accesses `r.span`/`r.leading_comment = ...`/etc. are unchanged — only the variant
name and pattern-bound type change).

Replace `RelationshipDecl` (~lines 211-236) with:

```rust
/// `relate_decl = "relate" "{" { binding } "}".`
#[derive(Debug, Clone)]
pub struct RelateDecl {
    /// The relate block's bindings, in declaration order.
    pub bindings: Vec<BindingDecl>,
    /// A leading comment immediately preceding this declaration, if recovered.
    pub leading_comment: Option<Comment>,
    /// A leading `///` doc comment immediately preceding this declaration, if recovered by
    /// [`crate::AdamAstParser`].
    pub doc_comment: Option<String>,
    /// Whether a blank line preceded this declaration, if recovered.
    pub blank_line_before: bool,
    /// A trailing comment immediately preceding this declaration's own closing `}`, if
    /// recovered. See <https://github.com/stlab/cel-rs/issues/52>.
    pub trailing_comment: Option<Comment>,
    /// Whether a blank line preceded this declaration's own closing `}`, if recovered. See
    /// <https://github.com/stlab/cel-rs/issues/52>.
    pub blank_line_before_close: bool,
    /// The span of this declaration's own opening `{`, used to recover trailing trivia when
    /// `bindings` is empty. See <https://github.com/stlab/cel-rs/issues/52>.
    pub open_brace_span: ExprSpan,
    /// The span of the whole `relate { ... }` declaration.
    pub span: ExprSpan,
}
```

Replace `MethodDecl` (~lines 395-417, move it to where `RelationshipDecl` used to sit isn't
required, but do keep the doc comment adjacent to `RelateDecl` for readability) with:

```rust
/// `binding = identifier { "," identifier } ":=" or_expression ";".`
///
/// Unlike the old `method_decl` this replaces, a binding names no explicit input cell list —
/// its inputs are whichever already-declared cells `body` references, deduced at compile time
/// (see `crate::parser::AdamParser::parse_deduced_expr`); this untyped CST parser has no cell
/// declarations to resolve against, so it records no input list at all, only the outputs.
#[derive(Debug, Clone)]
pub struct BindingDecl {
    /// The binding's output cell names (the comma-separated left-hand side), in declaration
    /// order.
    pub outputs: Vec<(String, ExprSpan)>,
    /// The parsed right-hand-side expression.
    pub body: cel_parser::Expr,
    /// A leading comment immediately preceding this binding, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<Comment>,
    /// Whether a blank line preceded this binding, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `a, b := ...;` declaration.
    pub span: ExprSpan,
}
```

Replace `OutDecl` and delete `OutMethodDecl` entirely (~lines 238-292) with:

```rust
/// `out_decl = "out" identifier [ ":" type_expr ] ":=" or_expression [ "require" "{" {
/// requirement } "}" ] ";".`
///
/// `type_expr` is unresolved here (no `TypeRegistry` lookup), matching `CellDecl`. When
/// absent, the cell's type is inferred from `initializer`'s result type by the compile phase
/// (`crate::parser::AdamParser`) — never here.
#[derive(Debug, Clone)]
pub struct OutDecl {
    /// The declared cell's name.
    pub name: String,
    /// The name token's span.
    pub name_span: ExprSpan,
    /// The `: type_expr` annotation, if present.
    pub type_name: Option<TypeExpr>,
    /// The parsed initializer expression that computes this cell's value.
    pub initializer: cel_parser::Expr,
    /// The `require { ... }` validation block, if present.
    pub require: Option<RequireBlock>,
    /// A leading `//`/`/* */` comment immediately preceding this declaration, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<Comment>,
    /// A leading `///` doc comment immediately preceding this declaration, if recovered by
    /// [`crate::AdamAstParser`].
    pub doc_comment: Option<String>,
    /// Whether a blank line preceded this declaration, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `out ... ;` declaration.
    pub span: ExprSpan,
}

/// The optional `require { ... }` block trailing an `out` declaration's initializer.
#[derive(Debug, Clone)]
pub struct RequireBlock {
    /// The block's requirements, in declaration order.
    pub requirements: Vec<RequirementDecl>,
    /// A trailing comment immediately preceding this block's own closing `}`, if recovered.
    /// See <https://github.com/stlab/cel-rs/issues/52>.
    pub trailing_comment: Option<Comment>,
    /// Whether a blank line preceded this block's own closing `}`, if recovered. See
    /// <https://github.com/stlab/cel-rs/issues/52>.
    pub blank_line_before_close: bool,
    /// The span of this block's own opening `{`, used to recover trailing trivia when
    /// `requirements` is empty.
    pub open_brace_span: ExprSpan,
    /// The span of the whole `require { ... }` block.
    pub span: ExprSpan,
}
```

Replace `ConditionDecl` (~lines 294-317) with:

```rust
/// `requirement = identifier ":" or_expression ";".`
///
/// `name` is a plain string label passed to `adam_rs::Sheet::add_output`, not a cell
/// reference — it may coincide with a cell name declared elsewhere in the sheet but doesn't
/// have to.
#[derive(Debug, Clone)]
pub struct RequirementDecl {
    /// The requirement's declared name.
    pub name: String,
    /// The name token's span.
    pub name_span: ExprSpan,
    /// The parsed requirement body expression; must type-check as `bool`.
    pub body: cel_parser::Expr,
    /// A leading comment immediately preceding this requirement, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub leading_comment: Option<Comment>,
    /// Whether a blank line preceded this requirement, if recovered by
    /// [`crate::trivia::attach_trivia`].
    pub blank_line_before: bool,
    /// The span of the whole `name: ...;` declaration.
    pub span: ExprSpan,
}
```

`ConditionalBranch`/`DefaultBranch` (~lines 349-393): change only their `relationships` field's
element type, `Vec<RelationshipDecl>` → `Vec<RelateDecl>` (field name unchanged, per the decision
above), and the grammar doc comment above `ConditionalBranch`:

```rust
/// `conditional_branch = literal "=>" "{" { relationship_decl } "}" [ "," ].`
```

to:

```rust
/// `conditional_branch = literal "=>" "{" { relate_decl } "}" [ "," ].`
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p adam-lang sheet_item_span_reads_the_relate_variant --lib`
Expected: still FAIL to compile — `ast_parser.rs`/`fmt.rs`/`trivia.rs` haven't been updated yet
and won't compile against the new `ast.rs` shapes. Continue to the next steps before expecting
green.

- [ ] **Step 5: Rewrite `adam-lang/src/ast_parser.rs`**

Update the `sheet_item` dispatch (~lines 157-181):

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

to:

```rust
    /// `sheet_item = cell_decl | relate_decl | conditional_decl | out_decl.`
    fn parse_sheet_item(&mut self, cursor: &mut TokenCursor) -> Result<ast::SheetItem> {
        use cel_parser::lex_lexer::{HasSpan, Token};
        match cursor.peek_token() {
            Some(Token::Identifier(id)) if id == "cell" => {
                self.parse_cell_decl(cursor).map(ast::SheetItem::Cell)
            }
            Some(Token::Identifier(id)) if id == "relate" => {
                self.parse_relate_decl(cursor).map(ast::SheetItem::Relate)
            }
            Some(Token::Identifier(id)) if id == "conditional" => self
                .parse_conditional_decl(cursor)
                .map(ast::SheetItem::Conditional),
            Some(Token::Identifier(id)) if id == "out" => {
                self.parse_out_decl(cursor).map(ast::SheetItem::Out)
            }
            Some(tok) => Err(cel_parser::ParseError::new(
                "expected `cell`, `relate`, `conditional`, or `out`",
                tok.span(),
            )),
            None => Err(cel_parser::ParseError::new(
                "unexpected end of input",
                proc_macro2::Span::call_site(),
            )),
        }
    }
```

Replace `parse_relationship_decl` and `parse_method_decl` (~lines 277-311, ~lines 480-503) with:

```rust
    /// `relate_decl = "relate" "{" { binding } "}".`
    fn parse_relate_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::RelateDecl> {
        let decl_start = cursor.peek_span();
        cursor.is_keyword("relate");
        let open_span = cursor.expect_open_brace()?;
        let mut bindings = Vec::new();
        while !cursor.at_close_brace() {
            bindings.push(self.parse_binding(cursor)?);
        }
        let close_span = cursor.expect_close_brace()?;
        Ok(ast::RelateDecl {
            bindings,
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

    /// `binding = identifier { "," identifier } ":=" or_expression ";".`
    fn parse_binding(&mut self, cursor: &mut TokenCursor) -> Result<ast::BindingDecl> {
        let decl_start = cursor.peek_span();
        let mut outputs = Vec::new();
        loop {
            let (name, span) = cursor.consume_ident()?;
            outputs.push((name, point(span)));
            if !cursor.consume_punct(",") {
                break;
            }
        }
        cursor.expect_punct(":=")?;
        let body = self.parse_cel_or_expression(cursor)?;
        let semi_span = cursor.expect_punct(";")?;
        Ok(ast::BindingDecl {
            outputs,
            body,
            leading_comment: None,
            blank_line_before: false,
            span: ast::ExprSpan {
                start: decl_start,
                end: semi_span,
            },
        })
    }
```

Update `parse_branch_relationships` (~lines 384-397), which is called from
`parse_conditional_decl` for both named and default branches:

```rust
    /// Parses one `conditional_branch`/`default_branch`'s shared body: `"{" { relationship_decl }
    /// "}"`, up to (not including) the closing `}`.
    fn parse_branch_relationships(
        &mut self,
        cursor: &mut TokenCursor,
    ) -> Result<Vec<ast::RelationshipDecl>> {
        use cel_parser::lex_lexer::Token;
        let mut relationships = Vec::new();
        while !cursor.at_close_brace() {
            if !matches!(cursor.peek_token(), Some(Token::Identifier(id)) if id == "relationship") {
                return Err(cursor.err_at("expected `relationship`"));
            }
            relationships.push(self.parse_relationship_decl(cursor)?);
        }
        Ok(relationships)
    }
```

to:

```rust
    /// Parses one `conditional_branch`/`default_branch`'s shared body: `"{" { relate_decl }
    /// "}"`, up to (not including) the closing `}`.
    fn parse_branch_relationships(
        &mut self,
        cursor: &mut TokenCursor,
    ) -> Result<Vec<ast::RelateDecl>> {
        use cel_parser::lex_lexer::Token;
        let mut relationships = Vec::new();
        while !cursor.at_close_brace() {
            if !matches!(cursor.peek_token(), Some(Token::Identifier(id)) if id == "relate") {
                return Err(cursor.err_at("expected `relate`"));
            }
            relationships.push(self.parse_relate_decl(cursor)?);
        }
        Ok(relationships)
    }
```

(Kept the function name and local variable `relationships` as-is — only its element type and
the keyword it checks for change; renaming the function too would ripple needlessly.)

Update the doc comment on `parse_conditional_decl` (~line 313) and `write_branch`/etc. call sites
are all still valid since only the *element type* of the `Vec` changed, not the function's own
signature shape.

Replace `parse_out_decl`, `parse_out_method`, and `parse_condition_decl` (~lines 399-478) with:

```rust
    /// `out_decl = "out" identifier [ ":" type_name ] ":=" or_expression [ "require" "{" {
    /// requirement } "}" ] ";".`
    fn parse_out_decl(&mut self, cursor: &mut TokenCursor) -> Result<ast::OutDecl> {
        let decl_start = cursor.peek_span();
        cursor.is_keyword("out");
        let (name, name_span) = cursor.consume_ident()?;
        let type_name = if cursor.consume_punct(":") {
            Some(self.parse_type_expr(cursor)?)
        } else {
            None
        };
        cursor.expect_punct(":=")?;
        let initializer = self.parse_cel_or_expression(cursor)?;
        let require = if cursor.is_keyword("require") {
            let open_span = cursor.expect_open_brace()?;
            let mut requirements = Vec::new();
            while !cursor.at_close_brace() {
                requirements.push(self.parse_requirement(cursor)?);
            }
            let close_span = cursor.expect_close_brace()?;
            Some(ast::RequireBlock {
                requirements,
                trailing_comment: None,
                blank_line_before_close: false,
                open_brace_span: point(open_span),
                span: ast::ExprSpan {
                    start: open_span,
                    end: close_span,
                },
            })
        } else {
            None
        };
        let semi_span = cursor.expect_punct(";")?;
        Ok(ast::OutDecl {
            name,
            name_span: point(name_span),
            type_name,
            initializer,
            require,
            leading_comment: None,
            doc_comment: None,
            blank_line_before: false,
            span: ast::ExprSpan {
                start: decl_start,
                end: semi_span,
            },
        })
    }

    /// `requirement = identifier ":" or_expression ";".`
    fn parse_requirement(&mut self, cursor: &mut TokenCursor) -> Result<ast::RequirementDecl> {
        let decl_start = cursor.peek_span();
        let (name, name_span) = cursor.consume_ident()?;
        cursor.expect_punct(":")?;
        let body = self.parse_cel_or_expression(cursor)?;
        let semi_span = cursor.expect_punct(";")?;
        Ok(ast::RequirementDecl {
            name,
            name_span: point(name_span),
            body,
            leading_comment: None,
            blank_line_before: false,
            span: ast::ExprSpan {
                start: decl_start,
                end: semi_span,
            },
        })
    }
```

Delete the free-standing `parse_cell_list` function (~line 518) and its doc comment — nothing
calls it anymore (bindings/out/requirement all use plain comma-separated identifier lists or a
single identifier, never a bracketed `cell_list`).

- [ ] **Step 6: Rewrite `adam-lang/src/fmt.rs`**

Delete `write_cell_list` (~line 111) — dead code, nothing calls it anymore.

Replace `write_method` (~lines 122-139) with:

```rust
/// Writes one `a, b := ...;` binding, delegating its body to [`cel_parser::format_expr`].
fn write_binding(out: &mut String, binding: &ast::BindingDecl, depth: usize) {
    write_trivia(
        out,
        binding.blank_line_before,
        binding.leading_comment.as_ref(),
        depth,
    );
    out.push_str(&indent(depth));
    for (i, (name, _)) in binding.outputs.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(name);
    }
    out.push_str(" := ");
    out.push_str(&cel_parser::format_expr(&binding.body));
    out.push_str(";\n");
}
```

Replace `write_relationship` (~lines 142-168) with:

```rust
/// Writes one `relate { ... }` declaration and its bindings, in declaration order.
fn write_relate(out: &mut String, rel: &ast::RelateDecl, depth: usize) {
    write_trivia(
        out,
        rel.blank_line_before,
        rel.leading_comment.as_ref(),
        depth,
    );
    write_doc_comment(out, "///", rel.doc_comment.as_deref(), depth);
    out.push_str(&indent(depth));
    out.push_str("relate {\n");
    for binding in &rel.bindings {
        write_binding(out, binding, depth + 1);
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

Update `write_branch_relationships` (~lines 172-186) and `write_branch` (~lines 190-207): change
every `ast::RelationshipDecl`/`write_relationship` reference to `ast::RelateDecl`/`write_relate`
(the `relationships: &[ast::RelateDecl]` parameter type and the internal `write_relate(out, rel,
depth + 1)` call site; nothing else in these two functions' logic changes).

Delete `write_out_method` (~lines 272-288) and `write_condition` (~lines 291-306). Replace them,
and `write_out` (~lines 310-338), with:

```rust
/// Writes one `name: ...;` requirement.
fn write_requirement(out: &mut String, req: &ast::RequirementDecl, depth: usize) {
    write_trivia(
        out,
        req.blank_line_before,
        req.leading_comment.as_ref(),
        depth,
    );
    out.push_str(&indent(depth));
    out.push_str(&req.name);
    out.push_str(": ");
    out.push_str(&cel_parser::format_expr(&req.body));
    out.push_str(";\n");
}

/// Writes one `out name[: type] := ...[ require { ... } ];` declaration.
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
    out.push_str(" := ");
    out.push_str(&cel_parser::format_expr(&decl.initializer));
    if let Some(require) = &decl.require {
        out.push_str(" require {\n");
        for req in &require.requirements {
            write_requirement(out, req, depth + 1);
        }
        write_trailing_trivia(
            out,
            require.blank_line_before_close,
            require.trailing_comment.as_ref(),
            depth + 1,
        );
        out.push_str(&indent(depth));
        out.push('}');
    }
    out.push_str(";\n");
}
```

Update `write_sheet_item`'s dispatch (~lines 344-354): `ast::SheetItem::Relationship(rel) =>
write_relationship(out, rel, depth)` → `ast::SheetItem::Relate(rel) => write_relate(out, rel,
depth)`.

- [ ] **Step 7: Rewrite `adam-lang/src/trivia.rs`**

Update the `use` block (~lines 28-31):

```rust
use crate::ast::{
    ConditionDecl, ConditionalBranch, ConditionalDecl, ExprSpan, MethodDecl, OutDecl,
    RelationshipDecl, Sheet,
};
```

to:

```rust
use crate::ast::{
    BindingDecl, ConditionalBranch, ConditionalDecl, ExprSpan, OutDecl, RelateDecl, RequireBlock,
    RequirementDecl, Sheet,
};
```

Rename the `TriviaTarget` impls (~lines 52-98): `impl TriviaTarget for MethodDecl` →
`impl TriviaTarget for BindingDecl` (body unchanged — `self.span`/`self.leading_comment =
Some(comment)`/`self.blank_line_before = value` are all still valid field names on
`BindingDecl`), `impl TriviaTarget for RelationshipDecl` → `impl TriviaTarget for RelateDecl`
(body unchanged), `impl TriviaTarget for ConditionDecl` → `impl TriviaTarget for RequirementDecl`
(body unchanged).

Rename `impl TrailingTriviaTarget for RelationshipDecl` (~lines 128-141) to
`impl TrailingTriviaTarget for RelateDecl` (body unchanged — same field names). Add a new impl
directly after it:

```rust
impl TrailingTriviaTarget for RequireBlock {
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
```

Delete `attach_out_trailing` (~lines 246-264) entirely — `OutDecl` no longer has its own closing
brace to recover trivia against (the new grammar is `;`-terminated and flat; only the optional
`require { ... }` sub-block has a brace pair, handled by the new `RequireBlock` impl above).

Update `attach_trivia`'s per-item dispatch (~lines 284-294):

```rust
    for item in &mut sheet.items {
        match item {
            crate::ast::SheetItem::Relationship(rel) => {
                attach_relationship(source, &line_starts, rel)
            }
            crate::ast::SheetItem::Conditional(cond) => {
                attach_conditional(source, &line_starts, cond)
            }
            crate::ast::SheetItem::Out(out_decl) => attach_out(source, &line_starts, out_decl),
            crate::ast::SheetItem::Cell(_) | crate::ast::SheetItem::Error { .. } => {}
        }
    }
```

to:

```rust
    for item in &mut sheet.items {
        match item {
            crate::ast::SheetItem::Relate(rel) => attach_relate(source, &line_starts, rel),
            crate::ast::SheetItem::Conditional(cond) => {
                attach_conditional(source, &line_starts, cond)
            }
            crate::ast::SheetItem::Out(out_decl) => attach_out(source, &line_starts, out_decl),
            crate::ast::SheetItem::Cell(_) | crate::ast::SheetItem::Error { .. } => {}
        }
    }
```

Rename `attach_relationship` (~lines 298-303) to `attach_relate`, its parameter type
`&mut RelationshipDecl` → `&mut RelateDecl`, and `rel.methods` → `rel.bindings` (both
occurrences: the `attach_gaps` call and the `last_child_end` computation):

```rust
/// Recovers trivia for a relate block's bindings.
fn attach_relate(source: &str, line_starts: &[usize], rel: &mut RelateDecl) {
    attach_gaps(source, line_starts, &mut rel.bindings);
    let last_child_end = rel.bindings.last().map(|b| b.span().end.end());
    attach_trailing(source, line_starts, last_child_end, rel);
}
```

Update `attach_conditional` (~lines 306-325): every internal `attach_relationship(source,
line_starts, rel)` call → `attach_relate(source, line_starts, rel)` (two call sites: inside the
`branches` loop and the `default` block); `branch.relationships`/`default.relationships` field
accesses are unchanged (per the field-name-stays decision above, only the element type changed).

Replace `attach_out` (~lines 327-355) entirely — delete the old manual "gap between writer and
first condition" logic (there's no more `writer` sibling to compute a gap from) with:

```rust
/// Recovers trivia for an `out` declaration's `require` block, if present — the gap before its
/// own closing `}`, and gaps between its requirements. An `out` with no `require` block has
/// nothing further to recover here: its `initializer` expression carries no trivia of its own,
/// matching `CellDecl.initializer`.
fn attach_out(source: &str, line_starts: &[usize], out_decl: &mut OutDecl) {
    let Some(require) = &mut out_decl.require else {
        return;
    };
    attach_gaps(source, line_starts, &mut require.requirements);
    let last_child_end = require.requirements.last().map(|r| r.span().end.end());
    attach_trailing(source, line_starts, last_child_end, require);
}
```

Update the module doc comment (~lines 5-6): `a `RelationshipDecl`'s `methods`` → `a `RelateDecl`'s
`bindings`` and `an `OutDecl`'s `conditions`` → `an `OutDecl`'s `require` block's `requirements``.

- [ ] **Step 8: Run the whole `adam-lang` build to find remaining breakage**

Run: `cargo build -p adam-lang 2>&1 | head -100`

Fix every remaining compile error this surfaces in `ast.rs`/`ast_parser.rs`/`fmt.rs`/`trivia.rs`
(there will be several — mostly in each file's own `#[cfg(test)] mod tests`, addressed next).

- [ ] **Step 9: Migrate every existing test in `ast_parser.rs`, `fmt.rs`, and `trivia.rs` to the new syntax**

None of these three files' existing tests can compile against the new `ast::*` shapes as-is —
every test that constructs or parses `relationship { method [...] -> [...] { ... } }`,
`out name: T { method [...] { ... } condition ... }`, or references `.methods`/`.conditions`/
`SheetItem::Relationship`/`RelationshipDecl`/`MethodDecl`/`ConditionDecl`/`OutMethodDecl` needs
updating. Apply these substitution rules mechanically to every test's source string and every
struct-literal/field-access in these three files' `mod tests`:

| Old | New |
|---|---|
| `relationship { method [a, b] -> [c] { expr } method [..] -> [..] { .. } ... }` | `relate { c := expr; .. := ..; ... }` |
| `relationship name { ... }` (named) | `relate { ... }` (name dropped — if a test specifically exercises the *optional name* feature, e.g. `parse_relationship_optional_name`, delete that test outright: there is no longer an optional name to parse) |
| `out name: T { method [inputs] { expr } condition c1 [in1] { e1 } condition c2 [in2] { e2 } }` | `out name: T := expr require { c1: e1; c2: e2; };` |
| `out name: T { method [inputs] { expr } }` (no conditions) | `out name: T := expr;` (no `require` block at all) |
| `SheetItem::Relationship(rel)` | `SheetItem::Relate(rel)` |
| `rel.methods` | `rel.bindings` |
| `out.conditions` | `out.require.as_ref().unwrap().requirements` (or restructure the assertion to check `out.require` directly, if the test's point is exercising presence/absence of `require` itself) |
| `out.writer.span`/`out.writer.inputs`/`out.writer.body` | `out.initializer` (a bare `cel_parser::Expr`, not a sub-struct — there's no `writer.span`/`writer.inputs` anymore) |
| `MethodDecl { inputs: ..., outputs: ..., body: ..., .. }` | `BindingDecl { outputs: ..., body: ..., .. }` (drop `inputs` entirely) |
| `ConditionDecl { name, inputs, body, .. }` | `RequirementDecl { name, body, .. }` (drop `inputs`) |
| `RelationshipDecl { name: Some(..)/None, methods, .. }` | `RelateDecl { bindings, .. }` (drop `name`) |

Work through `ast_parser.rs`'s test list one at a time (each name below is a hook to re-find it,
not a prescription for what its new body must literally be — rewrite each one's *source string
and assertions* using the table above, keeping each test's original intent): `parse_relationship_records_methods_in_order`
(rename to `parse_relate_records_bindings_in_order`, assert `rel.bindings.len()`), `parse_relationship_optional_name`
(delete — no longer applicable), `parse_conditional_branch_records_multiple_relationships`,
`parse_conditional_default_branch_records_multiple_relationships`,
`conditional_branch_bare_method_without_relationship_wrapper_recovers` (this one's whole *point*
was recovering from a bare `method [...]` appearing directly inside a conditional branch instead
of wrapped in `relationship { ... }` — rewrite its bad-syntax fixture to a bare `a := b;` binding
appearing directly inside a branch instead of wrapped in `relate { ... }`, keeping the same
recovery assertion shape), `parse_method_body_is_a_cel_expr_tree` (now exercises a binding's body
instead of a method's), `parse_out_with_explicit_type_and_no_conditions`,
`parse_out_with_no_type_annotation`, `parse_out_with_conditions_in_declaration_order` (rename to
`_requirements_`), `parse_malformed_out_is_recorded_as_an_error_item`,
`attaches_an_outer_doc_comment_to_a_relationship` (rename to `_to_a_relate`),
`attaches_an_outer_doc_comment_to_an_out_decl`,
`a_doc_comment_before_a_method_recovers_as_a_declaration_level_error` (rewrite the offending
fixture to a doc-comment directly before a bare binding). Run `cargo test -p adam-lang --lib
ast_parser` after rewriting each cluster; don't move to `fmt.rs` until this file is green.

Do the same for `fmt.rs`'s test list, using the same substitution table (its
`formats_a_named_relationship_with_multiple_methods`,
`preserves_a_comment_on_a_nested_method`, `formats_an_out_with_explicit_type_and_no_conditions`,
`formats_an_out_with_no_type_annotation`, `formats_an_out_with_conditions_in_declaration_order`,
`formats_doc_comments_on_a_relationship_conditional_and_out`,
`formats_a_trailing_comment_in_an_empty_relationship`,
`formats_a_trailing_comment_before_a_relationships_closing_brace`,
`formats_a_trailing_comment_before_an_outs_closing_brace` all need their fixture source strings
and expected-output strings rewritten to the new syntax; rename each to match, e.g.
`formats_a_named_relationship_with_multiple_methods` → `formats_a_relate_with_multiple_bindings`).
Note one structural change specific to `fmt.rs`: `formats_a_trailing_comment_before_an_outs_closing_brace`
must become "before the `require` block's closing brace", since `out` itself no longer has a
closing brace of its own — if a test needs an out-level trailing-comment case with no `require`
block at all, there is none to write (nothing to attach it to), so drop any such case rather than
inventing one. Run `cargo test -p adam-lang --lib fmt::` until green.

Do the same for `trivia.rs`'s test list (`attaches_a_comment_and_blank_line_to_a_method_inside_a_relationship`,
`attaches_a_comment_to_a_relationship_nested_inside_a_conditional_branch`,
`attaches_a_comment_to_a_relationship_nested_inside_the_default_branch`,
`attaches_a_comment_to_a_condition_inside_an_out_block` (rename `_to_a_requirement_inside_an_out_declaration`),
`recovery_span_that_abuts_the_next_keyword_does_not_invert_the_gap` (its fixture
`"sheet s { cell bad relationship { method [x] -> [y] { x } } }"` becomes
`"sheet s { cell bad relate { y := x; } }"`, and its assertion `sheet.items[1] ==
SheetItem::Relationship(_)` becomes `SheetItem::Relate(_)`),
`recovers_a_trailing_comment_in_an_empty_relationship_block`,
`recovers_a_trailing_comment_before_a_relationships_closing_brace`,
`recovers_a_trailing_comment_before_a_conditional_branchs_closing_brace`,
`recovers_a_trailing_comment_in_a_default_arm`,
`recovers_a_trailing_comment_before_an_outs_closing_brace_with_no_conditions` (rename/rewrite to
exercise the `require` block's own trailing brace instead, since there's no bare out-level brace
anymore — or delete if it becomes a duplicate of the requirement-block case once rewritten),
`recovers_a_trailing_comment_before_an_outs_closing_brace_after_a_condition`). Run `cargo test -p
adam-lang --lib trivia::` until green.

- [ ] **Step 10: Run the full `adam-lang` test suite**

Run: `cargo test -p adam-lang`
Expected: PASS (some tests renamed/rewritten per Step 9; `parser.rs`'s own tests still fail here
until Task 4 lands — that's expected and addressed there, not in this task).

Actually — `parser.rs` is compiled as part of the same crate, so a broken `parser.rs` (still
referencing the pre-Task-2 `adam_rs::Condition`) will make the whole crate fail to build, not
just fail its own tests. If `cargo build -p adam-lang` doesn't compile because of `parser.rs`,
that's expected at this point (Task 4 hasn't landed yet) — confirm the *only* compile errors are
inside `parser.rs`/its own test module by checking the error output names `parser.rs` exclusively,
then proceed to commit this task's work as-is; Task 4 restores a clean build.

- [ ] **Step 11: Format and lint the files this task touched**

Run: `cargo fmt --all` then, once Task 4 also lands (clippy needs the crate to build — if it
doesn't yet, skip this step here and fold it into Task 4's own lint step instead):
`cargo clippy -p adam-lang --all-targets -- -D warnings`.

- [ ] **Step 12: Commit**

```bash
git add adam-lang/src/ast.rs adam-lang/src/ast_parser.rs adam-lang/src/fmt.rs adam-lang/src/trivia.rs
git commit -m "feat(adam-lang): rewrite the CST parser and formatter for the new grammar"
```

---

## Task 4: `adam-lang` live parser — new grammar (typed, `Sheet`-building path)

**Files:**
- Modify: `adam-lang/src/parser.rs`

**Interfaces:**
- Consumes: `adam_rs::Requirement` (Task 2); the `:=` token (Task 1).
- Produces: `AdamParser::parse_deduced_expr(&mut self, ctx: &mut ParseContext) -> Result<(DynSegment,
  Vec<(String, CellId, TypeShape)>)>` and `AdamParser::compile_outputs(&self, ctx: &ParseContext,
  segment: &DynSegment, outputs: &[(String, CellId, TypeShape)]) -> Result<CompiledOutputs>` —
  both private, extracted from the existing `parse_match_expr`/`parse_method_body` to be shared
  by the conditional-match-expression path (unchanged behavior) and the new `relate`/`out`/
  `require` paths (this task's new code).

This is the highest-risk task: it reshapes the core compile-to-`Sheet` logic. Do the two
extraction refactors first (Steps 1-4), confirmed behavior-preserving by the *existing*
conditional-expression test suite staying green, before writing any new grammar on top.

- [ ] **Step 1: Extract `parse_deduced_expr` from `parse_match_expr`**

In `adam-lang/src/parser.rs`, change `parse_match_expr` (~line 486) from:

```rust
    fn parse_match_expr(
        &mut self,
        ctx: &mut ParseContext,
        match_span: proc_macro2::Span,
    ) -> Result<(TypeShape, MatchExpr)> {
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
                    TypeShape::Tuple(_) => InputPush::Tuple(self.types.associated_prototype(shape)),
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
```

to:

```rust
    /// Parses an `or_expression` whose input cells are deduced from whichever already-declared
    /// cell identifiers it references, rather than an explicit `cell_list` — the mechanism
    /// shared by a conditional's match-subject expression ([`Self::parse_match_expr`]), a
    /// `relate` binding's right-hand side, an `out` declaration's initializer, and a
    /// `require`ment body.
    ///
    /// Each 0-arity identifier lookup that names an already-declared cell is assigned the
    /// next argument index on first reference within this expression and reuses it on repeat
    /// reference (e.g. `a && a` allocates one argument slot, not two), via a scope pushed
    /// onto the CEL operation lookup for the duration of this parse.
    ///
    /// # Errors
    /// Returns `Err` if the expression fails to parse.
    ///
    /// - Complexity: O(k) in the number of distinct cell identifiers referenced, for this
    ///   method's own bookkeeping (on top of `cel-parser`'s own parse cost).
    fn parse_deduced_expr(
        &mut self,
        ctx: &mut ParseContext,
    ) -> Result<(DynSegment, Vec<(String, CellId, TypeShape)>)> {
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
                    TypeShape::Tuple(_) => InputPush::Tuple(self.types.associated_prototype(shape)),
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
        Ok((segment, inputs))
    }

    /// Compiles a conditional's match-subject expression — a bare identifier (`mode`) is the
    /// degenerate single-cell case; anything more (`a && b`) draws on however many
    /// already-declared cells it references, via [`Self::parse_deduced_expr`].
    ///
    /// `match_span` is used to report errors raised by this method or the shape inference it
    /// delegates to; the caller already has it (from before parsing the expression) for its own
    /// error reporting, so it's threaded through rather than recomputed.
    ///
    /// # Errors
    /// Returns `Err` if the expression fails to parse, produced no value, or (for a `Named`
    /// output shape) its type isn't registered in the `TypeRegistry`.
    fn parse_match_expr(
        &mut self,
        ctx: &mut ParseContext,
        match_span: proc_macro2::Span,
    ) -> Result<(TypeShape, MatchExpr)> {
        let (segment, inputs) = self.parse_deduced_expr(ctx)?;
        self.build_match_expr(segment, inputs, match_span)
    }
```

- [ ] **Step 2: Run the existing conditional-expression tests to confirm this refactor is behavior-preserving**

Run: `cargo test -p adam-lang conditional_ --lib` and `cargo test -p adam-lang parse_conditional
--lib`
Expected: PASS, unchanged — this step introduced no behavior change, only moved code.

- [ ] **Step 3: Extract `compile_outputs` from `parse_method_body`**

In `adam-lang/src/parser.rs`, `parse_method_body` (~line 1000) currently inlines both "parse the
body with a fixed input scope" and "figure out how to split the result across `outputs`" in one
function. Split it: add a new method directly above `parse_method_body`:

```rust
    /// Determines how to split a compiled body segment's result across `outputs`, given their
    /// declared shapes — shared by every construct that writes one or more named cells from a
    /// single compiled `or_expression` (a `relate` binding, an `out` declaration's initializer).
    ///
    /// One output takes the segment's single result directly (scalar via `call_dyn`, tuple-typed
    /// via `call_dyn_as_dynamic_sequence`, or the trivial empty-tuple case); more than one
    /// requires the result to be a tuple of matching arity and element shapes, split element-wise
    /// via `call_dyn_tuple_mixed`.
    ///
    /// # Errors
    /// Returns `Err` if any output's declared shape doesn't structurally match the body's actual
    /// result (scalar type mismatch, tuple arity mismatch, or tuple element shape mismatch, at
    /// any nesting depth), or if a single scalar/empty-tuple output's expression produced no
    /// value.
    fn compile_outputs(
        &self,
        ctx: &ParseContext,
        segment: &DynSegment,
        outputs: &[(String, CellId, TypeShape)],
    ) -> Result<CompiledOutputs> {
        if outputs.len() == 1 {
            let (out_name, _, out_shape) = &outputs[0];
            match out_shape {
                TypeShape::Named(out_type_id) => {
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
                    let call_fn = self
                        .types
                        .entry_by_type_id(*out_type_id)
                        .expect("registered")
                        .call_dyn_fn;
                    Ok(CompiledOutputs::Single(call_fn))
                }
                TypeShape::Tuple(elements) if elements.is_empty() => {
                    // () is CEL's concrete unit type, a distinct leaf TypeId -- not DynTuple.
                    let actual_type_id = segment.peek_output_type_id().ok_or_else(|| {
                        ctx.err_at(format!("output `{out_name}`: expression produced no value"))
                    })?;
                    if actual_type_id != TypeId::of::<()>() {
                        return Err(ctx.err_at(format!(
                            "output `{out_name}`: type mismatch: expected `()`, got a non-`()` \
                             value"
                        )));
                    }
                    Ok(CompiledOutputs::EmptyTuple)
                }
                TypeShape::Tuple(_) => {
                    let stack_info = segment.peek_stack_infos(1).first();
                    let matches = stack_info.is_some_and(|info| {
                        tuple_shape_matches_associated(out_shape, &info.associated)
                    });
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
                    Ok(CompiledOutputs::SingleTuple(
                        self.types.element_descriptors_for(out_shape),
                    ))
                }
            }
        } else {
            let arity = segment.peek_tuple_arity().unwrap_or(0);
            if arity != outputs.len() {
                return Err(ctx.err_at(format!(
                    "output expression has arity {arity} but {} output(s) declared",
                    outputs.len()
                )));
            }
            let associated = segment.peek_stack_infos(1)[0].associated.clone();
            let mut extractors = Vec::with_capacity(outputs.len());
            for (i, ((out_name, _, out_shape), elem)) in outputs.iter().zip(&associated).enumerate()
            {
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
                                .map(|(_, d, c, e, dbg)| (*d, *c, *e, *dbg))
                        }))
                    }
                });
            }
            Ok(CompiledOutputs::Tuple(extractors))
        }
    }
```

Then shrink `parse_method_body` to call it:

```rust
    fn parse_method_body(
        &mut self,
        ctx: &mut ParseContext,
        inputs: &[(String, CellId, TypeShape)],
        outputs: &[(String, CellId, TypeShape)],
    ) -> Result<(DynSegment, CompiledOutputs)> {
        let segment = self.parse_body_with_input_scope(ctx, inputs)?;
        let compiled = self.compile_outputs(ctx, &segment, outputs)?;
        Ok((segment, compiled))
    }
```

(Note: the error message inside the `else` branch changed from `"output expression has arity
{arity} but method declares {} output(s)"` to `"... but {} output(s) declared"`, since "method"
no longer names a grammar concept generic enough to cover both a binding and an out initializer.
If any existing test asserts this exact substring, update it in Step 8.)

- [ ] **Step 4: Run the existing method/out tests to confirm this refactor is behavior-preserving**

Run: `cargo test -p adam-lang parse_method --lib` and `cargo test -p adam-lang parse_out --lib`
and `cargo test -p adam-lang parse_relationship --lib`
Expected: PASS, except any test asserting the exact old arity-mismatch message text from Step 3's
parenthetical — fix those now (this is the only intentional behavior change in Steps 1-4; confirm
no other test broke).

- [ ] **Step 5: Update the `adam_rs` import**

Change:

```rust
use adam_rs::{CellId, Condition, MatchExpr, Method, OutputId, RelationshipId, Sheet};
```

to:

```rust
use adam_rs::{CellId, MatchExpr, Method, OutputId, RelationshipId, Requirement, Sheet};
```

- [ ] **Step 6: Write the failing tests for the new grammar**

Add to `adam-lang/src/parser.rs`'s `#[cfg(test)] mod tests`, directly after
`parse_relationship_single_method` (which Step 9 will also rewrite/rename — add these new ones
first so there's something green to compare against once the old test is gone):

```rust
    #[test]
    fn parse_relate_with_a_single_binding() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 2;
                    cell b: i32 = 0;
                    relate {
                        b := a;
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (b_id, _) = sheet.cell_names["b"].clone();
        assert_eq!(*sheet.read::<i32>(b_id).unwrap(), 2);
    }

    #[test]
    fn parse_relate_deduces_inputs_from_referenced_identifiers() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 2;
                    cell b: i32 = 3;
                    cell c: i32 = 0;
                    relate {
                        c := a * b;
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (c_id, _) = sheet.cell_names["c"].clone();
        assert_eq!(*sheet.read::<i32>(c_id).unwrap(), 6);
    }

    #[test]
    fn parse_relate_with_multiple_bindings_lets_the_planner_pick_a_direction() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 2;
                    cell b: i32 = 3;
                    cell c: i32 = 0;
                    relate {
                        c := a * b;
                        a := c / b;
                        b := c / a;
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (c_id, _) = sheet.cell_names["c"].clone();
        assert_eq!(*sheet.read::<i32>(c_id).unwrap(), 6);
    }

    #[test]
    fn parse_binding_undeclared_output_is_an_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: i32 = 1;
                relate {
                    missing := a;
                }
            }
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn parse_binding_multi_output_tuple_matches_existing_tuple_shape_rules() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell w: i32 = 4;
                    cell x: i32 = 0;
                    cell y: i32 = 0;
                    relate {
                        x, y := (w, w * 2);
                    }
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let (x_id, _) = sheet.cell_names["x"].clone();
        let (y_id, _) = sheet.cell_names["y"].clone();
        assert_eq!(*sheet.read::<i32>(x_id).unwrap(), 4);
        assert_eq!(*sheet.read::<i32>(y_id).unwrap(), 8);
    }

    #[test]
    fn parse_binding_multi_output_arity_mismatch_is_an_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell w: i32 = 4;
                cell x: i32 = 0;
                cell y: i32 = 0;
                relate {
                    x, y := w;
                }
            }
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn parse_out_with_direct_initializer_and_no_require_block() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    cell b: i32 = 4;
                    out area: i32 := a * b;
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = sheet.output_names["area"];
        let cell_id = sheet.sheet.output_cell(output_id).unwrap();
        assert_eq!(*sheet.sheet.read::<i32>(cell_id).unwrap(), 12);
    }

    #[test]
    fn parse_out_with_no_type_annotation_infers_from_initializer() {
        let sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    out doubled := a * 2;
                }
            "#,
            )
            .unwrap();
        let (_, shape) = sheet.cell_names["doubled"].clone();
        assert_eq!(shape, crate::type_registry::TypeShape::Named(std::any::TypeId::of::<i32>()));
    }

    #[test]
    fn parse_out_with_a_require_block_registers_named_requirements() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    cell b: i32 = 4;
                    out area: i32 := a * b require {
                        positive: area > 0;
                        small: area < 1000;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = sheet.output_names["area"];
        assert!(sheet.sheet.output_requirements(output_id).unwrap().len() == 2);
        assert!(sheet.sheet.violated_requirements(output_id).next().is_none());
    }

    #[test]
    fn parse_out_require_block_requirement_can_violate() {
        let mut sheet = parser()
            .parse_str(
                r#"
                sheet s {
                    cell a: i32 = 3;
                    cell b: i32 = 4;
                    out area: i32 := a * b require {
                        too_small: area > 1000;
                    };
                }
            "#,
            )
            .unwrap();
        sheet.propagate().unwrap();
        let output_id = sheet.output_names["area"];
        assert_eq!(sheet.sheet.violated_requirements(output_id).count(), 1);
    }

    #[test]
    fn parse_requirement_non_bool_body_is_an_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: i32 = 3;
                out x: i32 := a require {
                    bad: a;
                };
            }
        "#,
        );
        assert!(result.is_err());
    }
```

(`sheet.sheet`/`sheet.output_names` assume `ParsedSheet`'s existing public fields — confirm their
exact names by reading `ParsedSheet`'s definition near the top of `parser.rs` before writing
these; adjust field-access syntax if the actual names differ, keeping each test's assertions
intact.)

- [ ] **Step 7: Run the new tests to verify they fail**

Run: `cargo test -p adam-lang parse_relate_ parse_binding_ parse_out_with parse_requirement_
--lib`
Expected: FAIL to compile — `relate`/`:=`/`require` aren't recognized by the grammar yet.

- [ ] **Step 8: Rewrite the grammar productions**

Update `parse_sheet_item` (~line 174-193):

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

to:

```rust
    /// `sheet_item = [ doc_comment ] (cell_decl | relate_decl | conditional_decl | out_decl).`
    fn parse_sheet_item(&mut self, ctx: &mut ParseContext) -> Result<()> {
        let _ = ctx.consume_doc_comment_run(false); // outer `///` docs (ignored at runtime)
        match ctx.peek_token() {
            Some(Token::Identifier(id)) if id == "cell" => self.parse_cell_decl(ctx),
            Some(Token::Identifier(id)) if id == "relate" => {
                self.parse_relate_decl(ctx).map(|_| ())
            }
            Some(Token::Identifier(id)) if id == "conditional" => self.parse_conditional_decl(ctx),
            Some(Token::Identifier(id)) if id == "out" => self.parse_out_decl(ctx),
            Some(tok) => Err(ParseError::new(
                "expected `cell`, `relate`, `conditional`, or `out`",
                tok.span(),
            )),
            None => Err(ParseError::new(
                "unexpected end of input",
                Span::call_site(),
            )),
        }
    }
```

Replace `parse_relationship_decl` (~lines 444-462) and `parse_method_decl`/`parse_cell_list`
(~lines 893-923) with:

```rust
    /// `relate_decl = "relate" "{" { binding } "}".`
    ///
    /// - Postcondition: the returned `RelationshipId` identifies the relationship just added to
    ///   `ctx.sheet`.
    fn parse_relate_decl(&mut self, ctx: &mut ParseContext) -> Result<RelationshipId> {
        ctx.is_keyword("relate"); // consume
        ctx.expect_open_brace()?;
        let mut methods = Vec::new();
        while !ctx.at_close_brace() {
            methods.push(self.parse_binding(ctx)?);
        }
        ctx.expect_close_brace()?;
        ctx.sheet
            .add_relationship(methods)
            .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))
    }

    /// `binding = identifier { "," identifier } ":=" or_expression ";".`
    fn parse_binding(&mut self, ctx: &mut ParseContext) -> Result<Method> {
        let mut outputs: Vec<(String, CellId, TypeShape)> = Vec::new();
        loop {
            let (name, span) = ctx.consume_ident()?;
            let (cell_id, shape) = ctx
                .cell_names
                .get(&name)
                .cloned()
                .ok_or_else(|| ParseError::new(format!("undeclared cell `{name}`"), span))?;
            outputs.push((name, cell_id, shape));
            if !ctx.consume_punct(",") {
                break;
            }
        }
        ctx.expect_punct(":=")?;
        let (segment, inputs) = self.parse_deduced_expr(ctx)?;
        ctx.expect_punct(";")?;
        let compiled = self.compile_outputs(ctx, &segment, &outputs)?;
        Ok(build_method(inputs, outputs, segment, compiled))
    }
```

Update `parse_branch_relationships` (~lines 720-734), called from `parse_conditional_decl`:

```rust
    /// Parses one `conditional_branch`/`default_branch`'s shared body: `"{" { relationship_decl }
    /// "}"`, up to (not including) the closing `}`.
    fn parse_branch_relationships(
        &mut self,
        ctx: &mut ParseContext,
    ) -> Result<Vec<RelationshipId>> {
        let mut rel_ids = Vec::new();
        while !ctx.at_close_brace() {
            if !matches!(ctx.peek_token(), Some(Token::Identifier(id)) if id == "relationship") {
                return Err(ctx.err_at("expected `relationship`"));
            }
            rel_ids.push(self.parse_relationship_decl(ctx)?);
        }
        Ok(rel_ids)
    }
```

to:

```rust
    /// Parses one `conditional_branch`/`default_branch`'s shared body: `"{" { relate_decl }
    /// "}"`, up to (not including) the closing `}`.
    fn parse_branch_relationships(
        &mut self,
        ctx: &mut ParseContext,
    ) -> Result<Vec<RelationshipId>> {
        let mut rel_ids = Vec::new();
        while !ctx.at_close_brace() {
            if !matches!(ctx.peek_token(), Some(Token::Identifier(id)) if id == "relate") {
                return Err(ctx.err_at("expected `relate`"));
            }
            rel_ids.push(self.parse_relate_decl(ctx)?);
        }
        Ok(rel_ids)
    }
```

Replace `parse_out_decl` (~lines 736-849) and `parse_condition_decl` (~lines 851-891) with:

```rust
    /// `out_decl = "out" identifier [ ":" type_expr ] ":=" or_expression [ "require" "{" {
    /// requirement } "}" ] ";".`
    fn parse_out_decl(&mut self, ctx: &mut ParseContext) -> Result<()> {
        ctx.is_keyword("out"); // consume
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

        ctx.expect_punct(":=")?;
        let (segment, inputs) = self.parse_deduced_expr(ctx)?;

        // Unlike a `cell` initializer's segment (zero-argument, safe to evaluate once eagerly
        // via `eval_segment_boxed`/`build_cell_from_segment`), an `out` writer's segment takes
        // real cell inputs (via `push_arg`) and must stay live for repeated re-evaluation by the
        // `Method` built below on every `Sheet::propagate` — so only its *shape* is inferred
        // here, from stack info, never actually executed.
        let actual_shape = if segment.peek_tuple_arity().is_some() {
            let associated = segment.peek_stack_infos(1)[0].associated.clone();
            self.shape_of_associated(&associated)
                .map_err(|msg| ctx.err_at(msg))?
        } else {
            let type_id = segment
                .peek_output_type_id()
                .ok_or_else(|| ctx.err_at(format!("out `{name}`: expression produced no value")))?;
            if self.types.entry_by_type_id(type_id).is_none() {
                return Err(ctx.err_at(format!(
                    "out `{name}`: cannot infer a type for this expression; register a type \
                     name for it or add an explicit `: type_expr` annotation"
                )));
            }
            TypeShape::Named(type_id)
        };

        let out_shape = match &declared_shape {
            Some(declared) => {
                if declared != &actual_shape {
                    return Err(ctx.err_at(format!(
                        "out `{name}`: type mismatch: expected `{}`, got `{}`",
                        self.types.display_name(declared),
                        self.types.display_name(&actual_shape)
                    )));
                }
                declared.clone()
            }
            None => actual_shape,
        };

        let cell_id = self.build_default_cell(&out_shape, name_span, ctx)?;
        ctx.cell_names
            .insert(name.clone(), (cell_id, out_shape.clone()));

        let compiled = match &out_shape {
            TypeShape::Named(type_id) => {
                let call_fn = self
                    .types
                    .entry_by_type_id(*type_id)
                    .expect("output cell type registered")
                    .call_dyn_fn;
                CompiledOutputs::Single(call_fn)
            }
            TypeShape::Tuple(_) => {
                CompiledOutputs::SingleTuple(self.types.element_descriptors_for(&out_shape))
            }
        };
        let writer = build_method(
            inputs,
            vec![(name.clone(), cell_id, out_shape)],
            segment,
            compiled,
        );

        let mut requirement_names: Vec<String> = Vec::new();
        let mut requirements: Vec<Requirement> = Vec::new();
        if ctx.is_keyword("require") {
            ctx.expect_open_brace()?;
            while !ctx.at_close_brace() {
                let (req_name, requirement) = self.parse_requirement(ctx)?;
                requirement_names.push(req_name);
                requirements.push(requirement);
            }
            ctx.expect_close_brace()?;
        }

        ctx.expect_punct(";")?;

        let named_requirements: Vec<(&str, Requirement)> = requirement_names
            .iter()
            .map(String::as_str)
            .zip(requirements)
            .collect();

        let output_id = ctx
            .sheet
            .add_output(writer, named_requirements)
            .map_err(|e| ParseError::new(e.to_string(), Span::call_site()))?;
        ctx.output_names.insert(name, output_id);

        Ok(())
    }

    /// `requirement = identifier ":" or_expression ";".`
    fn parse_requirement(&mut self, ctx: &mut ParseContext) -> Result<(String, Requirement)> {
        let (name, _name_span) = ctx.consume_ident()?;
        ctx.expect_punct(":")?;
        let (segment, inputs) = self.parse_deduced_expr(ctx)?;
        ctx.expect_punct(";")?;

        let bool_type_id = TypeId::of::<bool>();
        let actual_type_id = segment.peek_output_type_id().ok_or_else(|| {
            ctx.err_at(format!("requirement `{name}`: expression produced no value"))
        })?;
        if actual_type_id != bool_type_id {
            let got = self
                .types
                .entry_by_type_id(actual_type_id)
                .map(|e| e.type_name)
                .unwrap_or("?");
            return Err(ctx.err_at(format!(
                "requirement `{name}`: expected `bool`, got `{got}`"
            )));
        }

        let call_fn = self
            .types
            .get("bool")
            .expect("bool always registered")
            .call_dyn_fn;
        let input_ids: Vec<CellId> = inputs.iter().map(|(_, id, _)| *id).collect();
        let input_types: Vec<TypeId> = inputs
            .iter()
            .map(|(_, _, shape)| cell_type_id(shape))
            .collect();
        let segment = RefCell::new(segment);
        let requirement = Requirement::new(input_ids, input_types, move |args| {
            let seg = &mut *segment.borrow_mut();
            let boxed = call_fn(seg, args)?;
            Ok(*boxed
                .downcast::<bool>()
                .expect("checked TypeId::of::<bool>() above"))
        });

        Ok((name, requirement))
    }
```

Delete `parse_method_body` and `parse_body_with_input_scope` (~lines 925-1104) entirely — no
production calls them anymore (every body-with-inputs construct now goes through
`parse_deduced_expr` + `compile_outputs`).

- [ ] **Step 9: Run the new tests to verify they pass**

Run: `cargo test -p adam-lang parse_relate_ parse_binding_ parse_out_with parse_requirement_
--lib`
Expected: PASS.

- [ ] **Step 10: Migrate every existing test in `parser.rs`'s `mod tests` to the new syntax**

Every test whose source string contains `relationship`, `method [`, `->`, or `condition ` needs
rewriting, using the same substitution table from Task 3 Step 9 (this file's tests exercise the
typed/`Sheet`-building path, so also update any assertion that reads `ConditionId`/`Condition`-
named identifiers to `RequirementId`/`Requirement`). Specifically (each name is a hook to re-find
the test, not a prescription for its new body — preserve each test's original assertions/intent):
`parse_relationship_single_method` (rename `parse_relate_single_binding` — this is now largely
redundant with the new `parse_relate_with_a_single_binding` from Step 6; keep whichever one
subsumes the other and delete the duplicate), `parse_method_undeclared_input_is_error` (rename
`parse_binding_undeclared_input_is_error` — an undeclared *input* identifier referenced in a
binding's RHS now surfaces as `parse_deduced_expr`'s scope simply not resolving it, so CEL's own
"undeclared identifier"-style error fires instead of the old `cell_list`-time "undeclared cell"
check; confirm the test still gets an `Err`, adjust any message-substring assertion),
`parse_method_output_type_mismatch_is_error`, `parse_relationship_multi_output_tuple` (likely
redundant with `parse_binding_multi_output_tuple_matches_existing_tuple_shape_rules` from Step 6
— keep one), `parse_method_output_tuple_arity_mismatch_is_error`,
`parse_method_output_tuple_element_type_mismatch_is_error`,
`parse_method_single_output_rejects_tuple_body`, `parse_method_single_tuple_typed_output`,
`parse_method_tuple_typed_output_among_several`,
`parse_method_with_tuple_typed_input_supports_field_indexing`,
`parse_method_tuple_output_shape_mismatch_is_an_error`,
`parse_method_single_empty_tuple_typed_output`, plus every out/condition test earlier in the
file and every test in the conditional-expression cluster whose fixture source strings embed
`relationship { method [...] -> [...] { ... } }` inside a branch body (these must become
`relate { ... := ...; }`, per the substitution table — only the *branch bodies'* syntax changes;
the conditional grammar itself, `parse_match_expr`/`build_match_expr`, is untouched by this plan).
Run `cargo test -p adam-lang --lib parser::` repeatedly while working through this list, fixing
compile errors first (they'll appear in large batches — old field/type names) and then assertion
failures, until the whole module is green.

- [ ] **Step 11: Run the full `adam-lang` test suite**

Run: `cargo test -p adam-lang`
Expected: PASS.

- [ ] **Step 12: Format and lint**

Run: `cargo fmt --all` then `cargo clippy -p adam-lang --all-targets -- -D warnings`.
Expected: clean. (If Task 3's Step 11 was deferred here because the crate didn't build yet, this
step covers it too.)

- [ ] **Step 13: Commit**

```bash
git add adam-lang/src/parser.rs
git commit -m "feat(adam-lang): rewrite the live parser for the new grammar"
```

---

## Task 5: `editors/vscode-adam-lang` — syntax highlighting keywords

**Files:**
- Modify: `editors/vscode-adam-lang/syntaxes/adam-lang.tmLanguage.json`

**Interfaces:** none (standalone JSON grammar file, no Rust code depends on it).

`adam-lsp` itself hardcodes no adam-lang keywords (confirmed: `diagnostics.rs`/`dispatch.rs`
delegate entirely to `adam-lang`'s own parser for error text) — only this TextMate grammar needs
updating.

- [ ] **Step 1: Update the keyword and operator patterns**

In `editors/vscode-adam-lang/syntaxes/adam-lang.tmLanguage.json`, change:

```json
        {
          "name": "keyword.declaration.adam-lang",
          "match": "\\b(sheet|cell|relationship|conditional|out|condition|method)\\b"
        },
```

to:

```json
        {
          "name": "keyword.declaration.adam-lang",
          "match": "\\b(sheet|cell|relate|conditional|out|require)\\b"
        },
```

Change:

```json
        {
          "name": "keyword.operator.arrow.adam-lang",
          "match": "->|=>"
        },
```

to:

```json
        {
          "name": "keyword.operator.arrow.adam-lang",
          "match": ":=|=>"
        },
```

(`->` no longer appears in any adam-lang production; `:=` replaces it as the other
two-character arrow-like operator alongside `=>`. The general operator class below already
matches bare `:`/`;`/`,` etc. and needs no change.)

- [ ] **Step 2: Manually verify the grammar file is still valid JSON**

Run: `node -e "JSON.parse(require('fs').readFileSync('editors/vscode-adam-lang/syntaxes/adam-lang.tmLanguage.json', 'utf8')); console.log('ok')"`
(or, if `node` isn't available in this environment, visually re-check bracket/comma balance —
this file has no automated test coverage).
Expected: prints `ok` (or, if `node` isn't installed, confirm by eye that no trailing/missing
comma was introduced).

- [ ] **Step 3: Commit**

```bash
git add editors/vscode-adam-lang/syntaxes/adam-lang.tmLanguage.json
git commit -m "chore(vscode-adam-lang): update keywords/operators for the new adam-lang grammar"
```

---

## Task 6: `begin` — rewrite bundled example sheets, fix stale `adam_rs` API calls, and their tests

**Files:**
- Modify: `begin/examples/diamond.adm2`
- Modify: `begin/examples/diamond-wing.adm2`
- Modify: `begin/examples/inequality.adm2`
- Modify: `begin/examples/toy_example.adm2`
- Modify: `begin/examples/out-cell.adm2`
- Modify: `begin/examples/image_resize.adm2`
- Modify: `begin/src/example_source.rs`
- Modify: `begin/src/inspector.rs`

**Interfaces:** consumes `adam_rs::{Requirement, RequirementId}` and `Sheet::output_requirements`/
`Sheet::requirement_inputs` (Task 2's rename) directly — `begin`'s Rust code that loads/parses
example sheets (`build_sheet`) is untouched, since it just calls `AdamParser::parse_str`, which now
accepts the new grammar after Task 4; but `begin/src/inspector.rs` calls the renamed `Sheet` API
*directly* (not through adam-lang's parser), so it needs its own small rename fix — a gap in this
plan's own pre-flight research, discovered while executing Task 4 (see the ledger).

**Note (added after Tasks 1-4 landed):** `begin/src/inspector.rs` has 5 direct references to the
pre-Task-2 `Condition` API that this plan's original file list never covered:
`cell_needs_full_propagate` (a real, non-test function, ~lines 99-115) calls
`sheet.output_conditions(oid)`/`sheet.condition_inputs(cid)`, and two `#[cfg(test)]` functions
import and call `adam_rs::Condition`/`Condition::from_fn_2`. Without this fix, `cargo build
--workspace` (Task 7's own requirement) cannot pass. Do this rename **first**, as Step 0, before
the example-sheet rewrite steps below (independent of them, but blocks Task 7 either way).

- [ ] **Step 0: Rename `begin/src/inspector.rs`'s stale `Condition`-family references**

Change (~lines 94-115, the doc comment and body of `cell_needs_full_propagate`):

```rust
/// conditions at all, per its own documented contract — so `output_valid`/
/// `output_violation_cells` would otherwise go stale after such a write).
///
/// - Complexity: O(number of conditionals + number of output conditions in the sheet).
fn cell_needs_full_propagate(sheet: &Sheet, id: CellId) -> bool {
    let is_match_cell = sheet.conditionals().any(|cid| {
        sheet
            .conditional_match_cells(cid)
            .is_some_and(|c| c.contains(&id))
    });
    let feeds_condition = sheet.outputs().any(|oid| {
        sheet.output_conditions(oid).is_some_and(|conditions| {
            conditions.iter().any(|&cid| {
                sheet
                    .condition_inputs(cid)
                    .is_some_and(|inputs| inputs.contains(&id))
            })
        })
    });
    is_match_cell || feeds_condition
}
```

to:

```rust
/// requirements at all, per its own documented contract — so `output_valid`/
/// `output_violation_cells` would otherwise go stale after such a write).
///
/// - Complexity: O(number of conditionals + number of output requirements in the sheet).
fn cell_needs_full_propagate(sheet: &Sheet, id: CellId) -> bool {
    let is_match_cell = sheet.conditionals().any(|cid| {
        sheet
            .conditional_match_cells(cid)
            .is_some_and(|c| c.contains(&id))
    });
    let feeds_requirement = sheet.outputs().any(|oid| {
        sheet.output_requirements(oid).is_some_and(|requirements| {
            requirements.iter().any(|&rid| {
                sheet
                    .requirement_inputs(rid)
                    .is_some_and(|inputs| inputs.contains(&id))
            })
        })
    });
    is_match_cell || feeds_requirement
}
```

Change the two test functions (~lines 429-449 and ~451-469): rename
`cell_needs_full_propagate_true_for_cell_feeding_an_output_condition` →
`cell_needs_full_propagate_true_for_cell_feeding_an_output_requirement` and
`cell_needs_full_propagate_false_for_cell_not_a_match_cell_or_condition_input` →
`cell_needs_full_propagate_false_for_cell_not_a_match_cell_or_requirement_input`; in both bodies,
change `use adam_rs::{Condition, Method};` to `use adam_rs::{Requirement, Method};` and
`Condition::from_fn_2([a, b], |x: &i32, y: &i32| Ok(x <= y))` to
`Requirement::from_fn_2([a, b], |x: &i32, y: &i32| Ok(x <= y))` (both occurrences).

Run: `cargo build -p begin --no-default-features 2>&1 | head -30`
Expected: no more `output_conditions`/`condition_inputs` errors (there may still be unrelated
errors if other steps haven't landed yet — but these two specific errors should be gone).

Run: `cargo test -p begin --no-default-features cell_needs_full_propagate --lib`
Expected: PASS (2 tests, renamed, same assertions as before).

Commit this step on its own before continuing to Step 1:

```bash
git add begin/src/inspector.rs
git commit -m "refactor(begin): follow adam-rs's Condition->Requirement rename in inspector.rs"
```

- [ ] **Step 1: Rewrite `begin/examples/diamond.adm2`**

Replace its entire contents with:

```
sheet diamond {
    cell a = 0.0;
    cell b = 0.0;
    cell c = 2.0;
    cell d = 3.0;

    relate {
        c := a * b;
        b := c / a;
        a := c / b;
    }

    relate {
        d := b * c;
        c := d / b;
        b := d / c;
    }
}
```

- [ ] **Step 2: Rewrite `begin/examples/diamond-wing.adm2`**

Replace its entire contents with:

```
sheet diamond_wing {
    cell a = 0.0;
    cell b = 0.0;
    cell c = 0.0;
    cell d = 2.0;
    cell e = 3.0;

    relate {
        e := b;
        b := e;
    }

    relate {
        c := a * b;
        b := c / a;
        a := c / b;
    }

    relate {
        d := b * c;
        c := d / b;
        b := d / c;
    }
}
```

- [ ] **Step 3: Rewrite `begin/examples/inequality.adm2`**

Replace its entire contents with:

```
sheet inequality {
    cell a = 0.0;
    cell b = 0.0;
    cell c = 2.0;

    relate {
        a := if a < b { a } else { b };
        b := if b < a { a } else { b };
    }
    relate {
        b := if b < c { b } else { c };
        c := if c < b { b } else { c };
    }
}
```

- [ ] **Step 4: Rewrite `begin/examples/toy_example.adm2`**

Replace its entire contents with:

```
sheet demo {
    cell a: f64 = 2.0;
    cell b: f64 = 3.0;
    cell c: f64;
    cell d: f64 = 4.0;
    cell e: f64 = 5.0;
    cell f: f64;
    cell g = 0.0;
    cell p: i32 = 0;

    relate {
        c := a * b;
        a := c / b;
        b := c / a;
    }

    relate {
        f := d * e;
        d := f / e;
        e := f / d;
    }

    conditional p {
        0i32 => {
            relate {
                c := f;
                f := c;
            }
        }
        1i32 => {
            relate {
                c := f * 2.0;
                f := c / 2.0;
            }
            relate {
                g := c * 10.0;
            }
        }
        _ => {
            relate {
                c := f;
            }
        }
    }
}
```

- [ ] **Step 5: Rewrite `begin/examples/out-cell.adm2`**

Replace its entire contents with:

```
// out cell with requirement and "don't care" value.
sheet out_cell {
    cell a = 0.0;
    cell b = 0.0;
    cell c = 0.0;
    cell p = false;

    conditional p {
        true => {
            relate {
                b := c;
            }
        }
    }

    out result := (a, b) require {
        min_a: a <= b;
    };
}
```

- [ ] **Step 6: Rewrite `begin/examples/image_resize.adm2`**

Replace its entire contents with (only the syntax changes; every comment, cell declaration, and
numeric/logic content is preserved verbatim):

```
// Copyright 2013 Adobe
// Distributed under the Boost Software License, Version 1.0.
// (See accompanying file LICENSE_1_0.txt or copy at http://www.boost.org/LICENSE_1_0.txt)
//
// Ported from ASL's classic `image_size` Adam sheet. adam-lang has no separate
// input/constant/interface/output/invariant sections and no `unlink`. The translation below:
// - ASL `constant`/one-directional `<==` rules -> a `relate` block with a single binding
// (the planner always derives that cell; there is no alternative binding to pick).
// - ASL `when (cond) relate {}` (arbitrary boolean expressions) -> `conditional <expr> {
// true => {...} false => {...} }`.
// - ASL `unlink x : init <== cond ? x : fallback;` -> `conditional cond`'s "false" branch
// forces `x` back to `fallback`; its "true" branch leaves `x` untouched (a free,
// directly writable cell), matching the checkbox-like behavior `unlink` describes.
// - ASL's `@bicubic`/`@draft`/etc. symbolic tags -> `String` literals (adam-lang has no
// atom/enum literal).
// The `result` output is approximated as a single tuple `(command, width, height,
// resolution, scale_styles, resample_method)`, command first — adam-lang has no
// tagged-union/struct output type, only tuples. Unlike ASL, the pixel/scale_styles/
// resample_method fields are present (and treated as relevant) even in the
// `set_resolution` case, where ASL would omit them entirely.
sheet image_resize {
    cell original_width: i32 = 1600;
    cell original_height: i32 = 1200;
    cell original_resolution: f64 = 300.0;

    // --- constant (derived once from input; never a source) ---
    cell original_doc_width: f64;
    cell original_doc_height: f64;

    // --- interface ---
    cell resample: bool = true;
    cell constrain: bool = true;
    cell scale_styles: bool = true;

    cell resample_method: String = "bicubic";

    cell width_pixels: i32 = 1600; // Photoshop: dim_width_pixels
    cell width_percent: f64 = 100.0; // Photoshop: dim_width_percent

    cell height_pixels: i32 = 1200; // Photoshop: dim_height_pixels
    cell height_percent: f64 = 100.0; // Photoshop: dim_height_percent

    cell doc_width_inches: f64 = 5.333333333333333;
    cell doc_width_percent: f64 = 100.0;

    // Resolution is declared before the height/inches cells below, mirroring the ASL
    // source's own comment: "Resolution must be initialized before width and height
    // inches to allow proportions to be constrained." adam-rs assigns each cell's initial
    // solver priority from declaration order, so this ordering is preserved best-effort;
    // the two solvers don't guarantee identical tie-breaking.
    cell doc_resolution: f64 = 300.0;

    cell doc_height_inches: f64 = 4.0;
    cell doc_height_percent: f64 = 100.0;

    cell auto_quality: String = "draft";

    // ASL leaves this uninitialized ("initialized from doc_resolution"), relying on
    // its solver to seed it from doc_resolution's *current* value before first use.
    // adam-lang cell initializers are literals only, and the planner's strength-based
    // release order can pick screen_lpi itself as the free source rather than deriving
    // it from doc_resolution — so unlike the sheet's other derived cells, leaving this
    // at a default 0.0 does NOT get corrected by the first propagate(); it must already
    // be consistent with doc_resolution's own default (300.0) at the default "draft"
    // quality (factor 1): 300.0 / 1 = 300.0.
    cell screen_lpi: f64 = 300.0;

    relate {
        original_doc_width := original_width as f64 / original_resolution;
    }
    relate {
        original_doc_height := original_height as f64 / original_resolution;
    }

    // Unconditional: doc_width_inches <-> doc_width_percent, pinned to original_doc_width.
    relate {
        doc_width_inches := doc_width_percent * original_doc_width / 100.0;
        doc_width_percent := doc_width_inches * 100.0 / original_doc_width;
    }

    // Unconditional: doc_height_inches <-> doc_height_percent, pinned to original_doc_height.
    relate {
        doc_height_inches := doc_height_percent * original_doc_height / 100.0;
        doc_height_percent := doc_height_inches * 100.0 / original_doc_height;
    }

    relate {
        screen_lpi := doc_resolution / if auto_quality == "draft" { 1.0 } else if auto_quality == "good" { 1.5 } else { 2.0 };
        doc_resolution := screen_lpi * if auto_quality == "draft" { 1.0 } else if auto_quality == "good" { 1.5 } else { 2.0 };
    }

    conditional resample {
        true => {
            relate {
                width_pixels := round(width_percent * original_width as f64 / 100.0) as i32;
                width_percent := width_pixels as f64 * 100.0 / original_width as f64;
            }
            relate {
                height_pixels := round(height_percent * original_height as f64 / 100.0) as i32;
                height_percent := height_pixels as f64 * 100.0 / original_height as f64;
            }
            relate {
                doc_width_inches := width_pixels as f64 / doc_resolution;
                width_pixels := round(doc_width_inches * doc_resolution) as i32;
                doc_resolution := width_pixels as f64 / doc_width_inches;
            }
            relate {
                doc_height_inches := height_pixels as f64 / doc_resolution;
                height_pixels := round(doc_height_inches * doc_resolution) as i32;
                doc_resolution := height_pixels as f64 / doc_height_inches;
            }
        }
        false => {
            relate {
                constrain := true;
            }
            // width_pixels/percent snap back to the un-resampled defaults.
            relate {
                width_pixels, width_percent := (original_width, 100.0);
            }
            relate {
                height_pixels, height_percent := (original_height, 100.0);
            }
            relate {
                doc_resolution := original_width as f64 / doc_width_inches;
                doc_width_inches := original_width as f64 / doc_resolution;
            }
            relate {
                doc_resolution := original_height as f64 / doc_height_inches;
                doc_height_inches := original_height as f64 / doc_resolution;
            }
        }
    }

    conditional resample && constrain {
        true => {
            relate {
                height_percent := width_percent;
                width_percent := height_percent;
            }
        }
        false => {
            relate {
                scale_styles := false;
            }
        }
    }

    out byte_count: i64 := width_pixels as i64 * height_pixels as i64 * 32i64;

    out result: (String, i32, i32, f64, bool, String) :=
        if resample { ("resize_image", width_pixels, height_pixels, doc_resolution, scale_styles, resample_method) } else { ("set_resolution", 0, 0, doc_resolution, false, "") }
        require {
            width_max: width_pixels <= 300000;
            height_max: height_pixels <= 300000;
        };
}
```

**Callout — one non-mechanical line in Step 6's rewrite:** the original
`relationship { method [resample] -> [constrain] { true } }` (inside `conditional resample {
false => { ... } }`) declared `resample` as a formal input even though its body (`true`) never
reads it — under the *old* explicit-`cell_list` grammar this was legal (an unused declared input
is simply never looked up). Under the new deduced-inputs grammar there is no way to declare an
unreferenced input at all — `constrain := true;`'s deduced input set is necessarily empty. This
is an intended consequence of "deduced inputs everywhere" (per the design spec), not a mistake to
work around, but it's worth confirming it doesn't matter here: the whole relationship is already
gated by the enclosing `conditional resample { false => { ... } }`, so `constrain`'s forcing only
ever activates when `resample` is already known to be `false` via the conditional's own match
mechanism (tracked separately from any `Method`'s `inputs`) — losing the redundant formal input
edge should be a no-op. Step 9's full `begin` test run (especially
`every_bundled_example_parses_successfully` and the two relevance-tracking regression tests) is
the actual verification; if either fails in a way traceable to this specific line, that's a
signal the input *was* load-bearing and needs a different fix than a literal one-line syntax
rewrite (flag it to the user rather than guessing at a workaround).

- [ ] **Step 7: Update `begin/src/example_source.rs`'s module doc comment**

Change (~lines 6-21):

```rust
//! `toy_example.adm2` demonstrates two independent bidirectional constraint
//! systems (`a × b = c` and `d × e = f`) linked by one conditional on `p`:
//!
//! - `p = 0`: the relationship `c = f` (bidirectional) becomes active.
//! - `p = 1`: the relationship `c = f × 2` (bidirectional) becomes active, alongside a
//!   second, independent relationship `g = c × 10` in the same branch — `g` is *forced*
//!   while this branch is active (see [`adam_rs::Sheet::is_forced`]), so its
//!   Inspector field is disabled and it is highlighted in the graph.
//! - Any other `p`: the two systems are independent and `g` is not forced.
//!
//! `g`'s relationship is its own `relationship { .. }` block within the `1i32` branch,
//! not folded into the `c`/`f` relationship's methods: a relationship's forced outputs
//! are the *intersection* of its methods' pure outputs, so mixing `[c] -> [g]` in with
//! the `c`/`f` methods would make that intersection empty, forcing nothing. A single
//! `conditional` branch can hold any number of `relationship` blocks, each contributing
//! its own independent forced-output set while that branch is active.
```

to:

```rust
//! `toy_example.adm2` demonstrates two independent bidirectional constraint
//! systems (`a × b = c` and `d × e = f`) linked by one conditional on `p`:
//!
//! - `p = 0`: the relationship `c = f` (bidirectional) becomes active.
//! - `p = 1`: the relationship `c = f × 2` (bidirectional) becomes active, alongside a
//!   second, independent relationship `g = c × 10` in the same branch — `g` is *forced*
//!   while this branch is active (see [`adam_rs::Sheet::is_forced`]), so its
//!   Inspector field is disabled and it is highlighted in the graph.
//! - Any other `p`: the two systems are independent and `g` is not forced.
//!
//! `g`'s relate block is its own `relate { .. }` block within the `1i32` branch, not
//! folded into the `c`/`f` relate block's bindings: a relationship's forced outputs
//! are the *intersection* of its bindings' pure outputs, so mixing `g := ..` in with
//! the `c`/`f` bindings would make that intersection empty, forcing nothing. A single
//! `conditional` branch can hold any number of `relate` blocks, each contributing its
//! own independent forced-output set while that branch is active.
```

- [ ] **Step 8: Update `begin/src/example_source.rs`'s inline test fixtures**

Change `VALID_SOURCE` (~lines 272-283):

```rust
    const VALID_SOURCE: &str = r#"
        sheet s {
            cell a: f64 = 2.0;
            cell b: f64 = 3.0;
            cell c: f64;
            relationship {
                method [a, b] -> [c] { a * b }
                method [b, c] -> [a] { c / b }
                method [a, c] -> [b] { c / a }
            }
        }
    "#;
```

to:

```rust
    const VALID_SOURCE: &str = r#"
        sheet s {
            cell a: f64 = 2.0;
            cell b: f64 = 3.0;
            cell c: f64;
            relate {
                c := a * b;
                a := c / b;
                b := c / a;
            }
        }
    "#;
```

Change `build_sheet_runtime_error_still_returns_sheet_and_message`'s inline source (~line 302):

```rust
        let source = "sheet s { cell x: i32 = 0; cell y: i32; relationship { method [x] -> [y] { 10i32 / x } } }";
```

to:

```rust
        let source = "sheet s { cell x: i32 = 0; cell y: i32; relate { y := 10i32 / x; } }";
```

- [ ] **Step 9: Run `begin`'s test suite**

Run: `cargo test -p begin --no-default-features` and `cargo test -p begin`
Expected: PASS, including `every_bundled_example_parses_successfully`,
`image_resize_constrain_is_relevant_despite_only_being_a_conditional_expression_input`, and
`image_resize_relevance_does_not_depend_on_which_cell_currently_holds_strength` (these last two
were already updated by a prior PR for the conditional-match-expression feature and need no
further changes — they assert on `Sheet::contributing_cells`/`output_relevant_cells` behavior
that this plan doesn't touch, not on adam-lang source syntax).

- [ ] **Step 10: Format and lint**

Run: `cargo fmt --all` then `cargo clippy -p begin --no-default-features --all-targets -- -D
warnings` and `cargo clippy -p begin --all-targets -- -D warnings`.
Expected: clean.

- [ ] **Step 11: Commit**

```bash
git add begin/examples/*.adm2 begin/src/example_source.rs
git commit -m "docs(begin): rewrite bundled example sheets for the new adam-lang grammar"
```

---

## Task 7: Update `adam-lang`'s grammar doc and validate the whole workspace

**Files:**
- Modify: `adam-lang/src/lib.rs`

**Interfaces:** none (doc-only + verification).

- [ ] **Step 1: Rewrite the grammar doc comment**

In `adam-lang/src/lib.rs`, change (~lines 6-24):

```rust
//! # Grammar
//!
//! ```text
//! sheet              = "sheet" identifier "{" { sheet_item } "}".
//! sheet_item         = cell_decl | relationship_decl | conditional_decl | out_decl.
//! cell_decl          = "cell" identifier cell_type_init ";".
//! cell_type_init     = (":" type_expr [ "=" or_expression ]) | ("=" or_expression).
//! type_expr          = identifier | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".
//! relationship_decl  = "relationship" [ identifier ] "{" { method_decl } "}".
//! conditional_decl   = "conditional" or_expression "{" { conditional_branch } [ default_branch ] "}".
//! conditional_branch = or_expression "=>" "{" { relationship_decl } "}" [ "," ].
//! default_branch     = "_"   "=>" "{" { relationship_decl } "}" [ "," ].
//! method_decl        = "method" cell_list "->" cell_list method_body.
//! out_decl           = "out" identifier [ ":" type_expr ] "{" out_method { condition_decl } "}".
//! out_method         = "method" cell_list method_body.
//! condition_decl     = "condition" identifier cell_list "{" or_expression "}".
//! cell_list          = "[" identifier { "," identifier } "]".
//! method_body        = "{" or_expression "}".
//! ```
```

to:

```rust
//! # Grammar
//!
//! ```text
//! sheet            = "sheet" identifier "{" { sheet_item } "}".
//! sheet_item       = [ doc_comment ] (cell_decl | relate_decl | conditional_decl | out_decl).
//! cell_decl        = "cell" identifier cell_type_init [ ":=" or_expression ] ";".
//! cell_type_init   = (":" type_expr ["=" or_expression]) | ("=" or_expression).
//! type_expr        = identifier | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".
//! relate_decl      = "relate" "{" { binding } "}".
//! binding          = identifier {"," identifier} ":=" or_expression ";".
//! conditional_decl = "conditional" or_expression "{" { conditional_branch } "}".
//! conditional_branch = (or_expression | "_") "=>" "{" { relate_decl } "}".
//! out_decl         = "out" identifier [":" type_expr] ":=" or_expression
//!                      [ "require" "{" { requirement } "}" ] ";".
//! requirement      = identifier ":" or_expression ";".
//! ```
//!
//! The `cell_decl` grammar shown above includes an optional trailing `":=" or_expression`
//! clause per the design spec, but **this crate does not yet implement it** — see
//! `docs/superpowers/specs/2026-08-19-adam-lang-syntax-design.md`'s "Explicitly out of scope"
//! section; it's deferred pending a forward-reference/hoisting decision. Only `cell_decl`'s
//! `"=" or_expression` one-time initializer is implemented today.
```

(The doc comment now matches the design spec's own grammar block, including its explicit note
about `cell`'s deferred `:=` sugar — copy the spec's wording rather than inventing new phrasing,
so the two documents stay in sync if the spec is revised later.)

- [ ] **Step 2: Format**

Run: `cargo fmt --all`
Expected: no diff (or a clean formatting-only diff — commit it if so).

- [ ] **Step 3: Build the whole workspace**

Run: `cargo build --workspace`
Expected: zero warnings.

- [ ] **Step 4: Test the whole workspace, including doc tests**

Run: `cargo test --workspace` then `cargo test --doc --workspace`
Expected: all tests pass, zero warnings.

- [ ] **Step 5: Lint (all three required invocations)**

Run, in order:

```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

Expected: zero warnings from all three.

- [ ] **Step 6: Doc build sanity check**

Run: `cargo doc --lib --no-deps --workspace`
Expected: builds cleanly (this exercises `adam-rs/src/lib.rs`'s renamed doctest from Task 2 and
`adam-lang/src/lib.rs`'s doctest from this task).

- [ ] **Step 7: Grep the whole workspace for stray old-syntax references**

Run: `git grep -n "\brelationship\b\|\bcondition\b\|method \[" -- '*.rs' '*.adm2' '*.md'` (exclude
`docs/superpowers/` — historical plan/spec docs intentionally describe the *old* grammar as
context and shouldn't be rewritten). Confirm every remaining hit is either: (a) inside
`docs/superpowers/`, (b) the still-valid English word "relationship"/"condition" in prose that
has nothing to do with adam-lang keywords (e.g. `adam-rs`'s `Error::InvalidConditional` doc
prose about "a relationship" in the branch-selection sense, which is intentionally unchanged),
or (c) a genuine miss from an earlier task — fix any case (c) hit found here before proceeding.

- [ ] **Step 8: Commit any residual fixes**

If Steps 2-7 required any code changes beyond formatting, commit them:

```bash
git add -A
git commit -m "chore: fix residual warnings and grammar-doc sync after the adam-lang syntax revision"
```

If no changes were needed, skip this step.

---

## Deferred (explicitly out of scope for this plan)

Per the design spec's own "Explicitly out of scope for this pass" section:
- The `cell ... := expr;` sugar (needs a forward-reference/hoisting decision first — see spec).
- Renaming `conditional` to something else (e.g. `switch`).
- `require`/validation on non-terminal, non-`out` cells.

If any of these becomes its own follow-up piece of work, it should get its own design-spec
addendum and plan, not be folded into this one after the fact.
