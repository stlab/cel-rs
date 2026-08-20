# adam-lang Syntax Revision

## Motivation

Reduce syntactic redundancy in adam-lang and make it read more naturally to a Rust
developer, while keeping the grammar parseable with a single token of lookahead at
every decision point. Concretely:

- `method [inputs] -> [outputs] { expr }` repeats cell names that are already implied
  by the expression body, and separates a relationship's direction from the values it
  computes.
- `out cell: T { method [inputs] { expr } condition name [inputs] { expr } ... }` nests
  a writer method and its validation conditions inside redundant block structure.
- `default_branch` and `conditional_branch` are described as two different grammar
  productions when the implementation already treats them as one, with `_` as a
  special-cased wildcard pattern.
- `relationship` is longer than it needs to be for how often it's typed.

## Grammar (new)

```
sheet            = "sheet" identifier "{" { sheet_item } "}".
sheet_item       = [ doc_comment ] (cell_decl | relate_decl | conditional_decl | out_decl).

cell_decl        = "cell" identifier cell_type_init [ ":=" or_expression ] ";".
cell_type_init   = (":" type_expr ["=" or_expression]) | ("=" or_expression).

relate_decl      = "relate" "{" { binding } "}".
binding          = identifier {"," identifier} ":=" or_expression ";".

out_decl         = "out" identifier [":" type_expr] ":=" or_expression
                     [ "require" "{" { requirement } "}" ] ";".
requirement      = identifier ":" or_expression ";".

conditional_decl = "conditional" or_expression "{" { conditional_branch } "}".
conditional_branch = (or_expression | "_") "=>" "{" { relate_decl } "}".
```

Unchanged from today: `sheet`, `cell_type_init`, `type_expr`, and the mechanism by
which a body expression's free identifiers are resolved against already-declared
cells (the scope-pushing approach `parse_match_expr` already uses for a conditional's
match subject — extended to method/out/binding bodies rather than reinvented).

## Design decisions

**`relationship` → `relate`.** Pure rename, no grammar shape change.

**Deduced inputs everywhere.** `cell_list` (`"[" identifier {","} "]"`) is gone from
method/out/binding bodies. A binding or out-writer's inputs are whichever
already-declared cell names its expression references — reusing
`parse_match_expr`'s existing identifier-scope mechanism rather than adding new
machinery. `->` is retired from the grammar entirely (it had no other use).

**Multi-output bindings.** `sum, diff := a + b, a - b` becomes `sum, diff := expr;`
where the LHS is a comma-separated cell-name list and the RHS's inferred shape must
be a tuple type of matching arity/element types — this reuses the existing
`CompiledOutputs::Tuple` shape-matching, not new validation.

**Token: `:=` (not `<==`, not `<-`).** `<-` was rejected early: the lexer's
Joint-`Punct` combiner joins adjacent punctuation based on *source spacing*
(`proc_macro2::Spacing`), so `a<-1` (no space) would joint-combine into the arrow
token even when intended as "a is less than negative one" — a footgun that gets worse
the more naturally `<` and `-` occur adjacent to each other in ordinary expressions.
`<==` avoids that specific collision (no valid CEL expression has `<=` immediately
followed by `=`), but requires a new `PunctOp::Three` and a second lookahead step in
`lex_lexer.rs`'s combiner, which currently only ever joins exactly two characters via
a fixed `matches!` table (`is_compound_operator`). `:=` is a one-line addition to that
existing table — `:` is never adjacent to `=` in any current production (`cell_type_init`'s
`:` is always followed by a `type_expr` identifier before any `=` could appear) — so it
was chosen for implementation simplicity over `<==`'s marginally more evocative
"continuous inflow" visual.

**`require` — mandatory-named, `;`-terminated.**
`requirement = identifier ":" or_expression ";"`. Names are mandatory specifically
because it removes an LL(1) problem: an optional name would require peeking *past*
the leading identifier to see whether a `:` follows before knowing whether that
identifier is a label or the first token of a bare expression — two tokens, not one.
Mandatory naming also happens to fix the diagnostic-naming question for free (every
requirement is nameable via `Sheet::violated_requirements`, see rename below).
`;`-termination (not comma-separated) matches every other *sequence-of-statements*
construct in the grammar (`sheet_item`, `binding`) — commas are reserved for
single-line name lists (`binding`'s LHS).

**`cell ... := expr;` sugar.** Composes into the existing `cell_decl` grammar as one
more optional trailing clause rather than a new production, exactly because
`cell_type_init` already makes the default/initializer optional given a type
annotation (mirroring how `out` already seeds via a registered default with no `=`
syntax at all). Desugars to the unchanged `cell_decl` followed by a synthesized
`relate { name := expr; }`. **Open question, deliberately deferred:** whether the
synthesized binding is inserted at the declaration's point (forward references to
not-yet-declared cells stay illegal, consistent with everything else in the grammar
today) or hoisted to end-of-sheet (would need a new forward-reference concept that
nothing else in the grammar has). Needs a decision before implementation, not
included in this pass.

**`relate` drops its optional name entirely** (previously parsed and discarded —
"ignored at runtime"). Named relationships are a future feature to design when there's
an actual consumer for the name (e.g. diagnostics); the grammar carries no vestigial
syntax for it in the meantime.

**`conditional`/`default_branch` — grammar/doc consolidation only, no behavior
change.** `_ => { ... }` is documented as one case of `conditional_branch`'s pattern
(`or_expression | "_"`), matching what the implementation already does. `_` stays a
reserved sigil in that position (pre-existing quirk: a cell literally named `_` could
never be matched by value there — unchanged, out of scope for this pass).

## Explicitly out of scope for this pass

- Renaming `conditional` to something else (e.g. `switch`) — under consideration for
  a later pass, once there's more clarity on how CEL's own future match expressions
  will read alongside it.
- Validation (`require`) on non-terminal, non-`out` cells. Intentionally asymmetric:
  it's unclear what a requirement on an interior cell would mean operationally. The
  unimplemented feature this gap actually wants is **input filters** (a different,
  not-yet-designed concept) — not `require` extended to interior cells.
- The `cell ... := expr` sugar's forward-reference/hoisting semantics (see above).
- Any change to `cell_decl`'s existing `=` one-time initializer, which stays
  distinct from `:=`'s continuous re-evaluation by design.

## `adam-rs` renames (to keep `require` mapping to a like-named API)

`adam-rs` currently uses `Condition`/`condition_*` (singular) for exactly the
requirement concept, and a separate, unrelated `Conditional`/`conditional_*` family
for branch-selection (`add_conditional`, `ConditionalId`, `MatchExpr`-backed
switching). Only the first family renames; the second is untouched since it still
correctly corresponds to the (unchanged) `conditional` keyword.

| Before | After |
|---|---|
| `adam-rs/src/condition.rs` (file) | `requirement.rs` |
| `struct Condition` | `struct Requirement` |
| `struct ConditionId` | `struct RequirementId` |
| `struct ConditionData` | `struct RequirementData` |
| `Sheet::condition_name` | `Sheet::requirement_name` |
| `Sheet::condition_output` | `Sheet::requirement_output` |
| `Sheet::condition_inputs` | `Sheet::requirement_inputs` |
| `Sheet::condition_contributing_cells` | `Sheet::requirement_contributing_cells` |
| `Sheet::output_conditions` | `Sheet::output_requirements` |
| `Sheet::violated_conditions` | `Sheet::violated_requirements` |
| `Sheet::add_output`'s `conditions: Vec<(&str, Condition)>` | `requirements: Vec<(&str, Requirement)>` |
| `output.rs`'s `conditions: Vec<ConditionId>` field | `requirements: Vec<RequirementId>` |
| `adam-lang`'s `parse_condition_decl` | `parse_requirement` |
| `adam-lang`'s `ast::ConditionDecl` | `ast::RequirementDecl` |
| `adam-lang/src/fmt.rs`'s `write_condition` | `write_requirement` |

Untouched: `Conditional`, `ConditionalId`, `add_conditional`, `conditional_decl`,
`ConditionalDecl`, `parse_conditional_decl`, `Sheet::conditionals`,
`conditional_match_cells`, `conditional_branch_count`,
`conditional_branch_relationships`, `conditional_default_relationships`,
`conditional_active_branch`.

## Implementation notes (not design forks, but real work)

- `cel-parser/src/lex_lexer.rs`: add `(':', '=')` to `is_compound_operator`'s
  `matches!` table for `:=`.
- The grammar change touches both of `adam-lang`'s parsers: the "live" `parser.rs`
  (builds a `Sheet` directly) and the lossless CST parser (`ast.rs`, `ast_parser.rs`,
  `fmt.rs`, `trivia.rs`) used for formatting/comment-preservation. Both need the new
  productions.
- Likely downstream touch points to check during planning: `adam-lsp` and
  `editors/vscode-adam-lang` (syntax highlighting / keyword lists), if they hardcode
  the old keyword or token set (`method`, `relationship`, `->`, `condition`).
- No migration/back-compat path needed — the project has no clients yet (per root
  `CLAUDE.md`); existing `.adam` fixtures/tests in the repo need updating as part of
  the implementation, not a compatibility shim.
