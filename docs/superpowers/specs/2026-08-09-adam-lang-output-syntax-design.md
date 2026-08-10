# adam-lang Syntax for Output Cells and Conditions

**Date:** 2026-08-09
**Branch:** worktree-out-cells-lang
**Status:** Approved (design), not yet implemented

## Summary

Adds DSL syntax to `adam-lang` for the `adam-rs` output/condition feature
already implemented per
`docs/superpowers/specs/2026-08-07-output-cells-design.md` (`Sheet::add_output`,
`Condition`, `OutputId`, `ConditionId`). That design doc's §9 sketch was
explicitly non-binding ("only shown to sanity-check that the Rust API shape
maps cleanly onto something DSL-shaped"); this doc supersedes it with a
concrete, binding grammar plus the parser/AST/formatter/typechecker changes
needed to support it across `adam-lang`'s full tooling stack.

An **out cell** is declared like an ordinary `cell`, but is always computed
by exactly one writer method (never planner-arbitrated, never directly
writable) and may carry zero or more named boolean **conditions**, checked
after every `propagate()`.

```text
sheet resize_command {
    cell width: f64;
    cell height: f64;
    cell max_area: f64 = 100.0;
    cell max_width: f64 = 20.0;
    cell max_height: f64 = 20.0;

    out image_size: f64 {
        method [width, height] { width * height }

        condition max_area   [width, height, max_area] { width * height <= max_area }
        condition max_width  [width, max_width]         { width <= max_width }
        condition max_height [height, max_height]        { height <= max_height }
    }
}
```

---

## 1. Motivation

See §1 of the `adam-rs` design doc: a sheet can represent a command's
arguments, and a command's preconditions (e.g. `width * height <= max_area`)
aren't relationships the solver should silently satisfy by adjusting
`width`/`height` — they're checks on a derived value. `adam-rs` already
supports this at the runtime level; this doc gives it adam-lang source
syntax.

---

## 2. Design decisions (settled during brainstorming)

- **`out` is a cell-declaration kind, not a separate wrapping construct.**
  `adam-lang` is expected to grow other cell kinds over time (ASL Adam has
  `input`/`constant` cells too), so `out` reads as a qualifier on a cell
  declaration, parallel to plain `cell`, rather than an unrelated block that
  happens to reference a cell by name (the way `conditional <name> { .. }`
  does).
- **The keyword is `out`, not `output`**, at the user's explicit direction —
  a deliberate exception to the otherwise-consistent "keyword = lowercased
  Rust type/fn name" rule that holds for `cell`→`CellId`, `relationship`→
  `add_relationship`, `conditional`→`add_conditional`, `method`→`Method`.
  `condition` still follows that rule (`Condition`/`ConditionId`).
- **`condition` takes an explicit `[cell_list]`**, mirroring `method`'s
  shape, rather than resolving identifiers implicitly against the whole
  sheet. Chosen over implicit capture for internal consistency with
  `method`, even though `Condition`'s inputs may legally be any cell in the
  sheet (not just the writer's own inputs) — the explicit list is just this
  condition's own declared inputs, unrelated to the writer method's list.
- **The writer method inside `out` omits `-> [cell_list]`.** The writer's
  single output is always and only the enclosing `out` cell, so restating
  its name is pure redundancy — and dropping it is what makes the type
  annotation optional (see next point): keeping `-> [cell_list]` would
  require the out cell's `CellId` (hence its type) to exist *before* parsing
  the writer body, which is exactly what optional-type inference cannot
  provide.
- **`: type_name` is optional on `out`, exactly as it already is on `cell`.**
  Unlike `cell` (which can infer a literal's type without evaluating
  anything), `out` has no literal — so when the annotation is omitted, the
  type is inferred from the writer method body's actual result type after
  it's parsed and compiled, not before.

---

## 3. Grammar

Extends the EBNF in `adam-lang/src/lib.rs`'s crate-root doc comment:

```text
sheet_item         = cell_decl | relationship_decl | conditional_decl | out_decl.
out_decl           = "out" identifier [ ":" type_name ] "{" out_method { condition_decl } "}".
out_method         = "method" cell_list method_body.
condition_decl     = "condition" identifier cell_list "{" or_expression "}".
```

`cell_list` and `method_body` are unchanged (`method_body = "{" or_expression "}"`,
already defined). `out_method` is `method_decl` minus its `"->" cell_list` —
a distinct production, not a variant of `method_decl`, since the two are
used in different contexts (`relationship` allows any number of full
`method_decl`s; `out` allows exactly one `out_method`, no `->`).

A `condition_decl`'s `identifier` is the condition's *name* (a plain string
label passed to `Sheet::add_output`, not a cell reference) — it may
coincide with a cell name in the sheet (as in the worked example above,
where the condition `max_area` and the cell `max_area` share a name) but
doesn't have to.

---

## 4. AST (`ast.rs`)

```rust
/// `out_decl = "out" identifier [ ":" type_name ] "{" out_method { condition_decl } "}".`
pub struct OutDecl {
    pub name: String,
    pub name_span: ExprSpan,
    /// The `: type_name` annotation, if present. Absent means the type is inferred from
    /// `writer.body`'s result type during compilation.
    pub type_name: Option<(String, ExprSpan)>,
    pub writer: OutMethodDecl,
    /// This output's conditions, in declaration order.
    pub conditions: Vec<ConditionDecl>,
    pub leading_comment: Option<String>,
    pub blank_line_before: bool,
    pub span: ExprSpan,
}

/// `out_method = "method" cell_list method_body.`
pub struct OutMethodDecl {
    pub inputs: Vec<(String, ExprSpan)>,
    pub body: cel_parser::Expr,
    pub leading_comment: Option<String>,
    pub blank_line_before: bool,
    pub span: ExprSpan,
}

/// `condition_decl = "condition" identifier cell_list "{" or_expression "}".`
pub struct ConditionDecl {
    pub name: String,
    pub name_span: ExprSpan,
    pub inputs: Vec<(String, ExprSpan)>,
    pub body: cel_parser::Expr,
    pub leading_comment: Option<String>,
    pub blank_line_before: bool,
    pub span: ExprSpan,
}
```

`SheetItem` gains `Out(OutDecl)`; `span()`, `set_leading_comment()`, and
`set_blank_line_before()` each gain a matching arm, following the existing
`Cell`/`Relationship`/`Conditional` pattern exactly.

---

## 5. Direct parser (`parser.rs`)

`parse_sheet_item` gains an `"out"` branch dispatching to `parse_out_decl`.

`parse_out_decl`:

1. Consume `out`, then the name identifier — error (`duplicate cell` — same
   message/check as `parse_cell_decl`) if already in `ctx.cell_names`.
2. If `: type_name` follows, resolve it against the `TypeRegistry` now
   (unknown name → error, same as `cell`) and remember `(TypeId, AddCellFn)`.
   Otherwise leave both unresolved for now.
3. `expect_open_brace`, then parse the mandatory `out_method`: `method`
   keyword, `cell_list` inputs (each must already be declared, exactly like
   `method`'s inputs today — no forward references, consistent with every
   other cell-list resolution in the grammar), `method_body`. This reuses
   the existing input-scope-push machinery from `parse_method_body`, but
   with no declared output type to check against yet.
4. Determine the cell's type:
   - Annotation present: check the compiled body's
     `peek_output_type_id()` against the annotated type, same error as
     `method`'s single-output mismatch case.
   - Annotation absent: take the body's actual `peek_output_type_id()`
     directly. Look it up via `self.types.entry_by_type_id(..)`; if no type
     is registered under that `TypeId`, error ("cannot infer a type for
     `image_size`; register a type name for this type or add an explicit
     `: type_name` annotation").
5. Create the cell via the resolved entry's `add_cell_fn`/`default_fn`
   (same call shape as `parse_cell_decl`'s annotation-only branch), insert
   into `ctx.cell_names` — *after* this point `image_size` resolves like any
   other declared cell, including inside this same `out` block's own
   `condition` lists (self-reference; `add_output` explicitly allows a
   condition to reference the output's own cell — see the `adam-rs` design
   doc §11).
6. Build the writer `Method` (reusing `build_method`/`CompiledOutputs::Single`
   — an `out` writer is always single-output, so the tuple path never
   applies) with the newly created cell as its sole output.
7. Parse zero or more `condition_decl`s: `condition` keyword, name
   identifier, `cell_list` inputs (resolved against `ctx.cell_names`, same
   as any other cell list), `{ or_expression }`. The body is type-checked
   against `bool` specifically (not a declared output cell's type — there
   isn't one), using the same compiled-segment machinery as a method's
   single-output check, hardcoded to `TypeId::of::<bool>()`. Build a
   `Condition` via `Condition::new(inputs, input_types, closure)`, where the
   closure calls the compiled segment and downcasts its `bool` result,
   mirroring `build_method`'s `CompiledOutputs::Single` closure shape but
   returning `Result<bool, anyhow::Error>` directly instead of
   `Result<Vec<Box<dyn Any>>, _>`.
8. Call `ctx.sheet.add_output(writer, conditions)`, mapping any `Err` to
   `ParseError` exactly like `parse_relationship_decl`/`parse_conditional_decl`
   already do for `add_relationship`/`add_conditional`.
9. Insert the returned `OutputId` into a new `output_names: IndexMap<String,
   OutputId>` field on `ParseContext`/`ParsedSheet`, keyed by the cell name —
   parity with `cell_names`, so callers (the LSP, `begin`'s Inspector) can
   look up an output's `OutputId` by name without separately tracking it.

---

## 6. AST parser (`ast_parser.rs`)

`parse_out_decl` mirrors the grammar structurally with no `TypeRegistry`
involved, consistent with `AdamAstParser`'s existing "resolves no
identifiers, validates nothing during parsing" design (see `ast.rs`'s module
doc): it records the name, the optional raw `type_name` token, the writer's
`inputs`/`body`, and each condition's `name`/`inputs`/`body`, with spans
throughout. All semantic validation (type resolution, duplicate condition
names, terminal-cell checks) stays deferred to the direct parser / a future
compile-to-`Sheet` phase, exactly as it already is for `cell`/`method`.

Error recovery follows the same declaration-level granularity as
`relationship`/`conditional`: a malformed `out_method` or `condition_decl`
fails the whole enclosing `out_decl`, recorded as one `SheetItem::Error`.

---

## 7. Formatter (`fmt.rs`)

`write_out` (parallel to `write_relationship`) and `write_condition`
(parallel to `write_method`, minus the `->` half):

```text
out image_size: f64 {
    method [width, height] { width * height }

    condition max_area [width, height, max_area] { width * height <= max_area }
}
```

Omits `: f64` when `OutDecl.type_name` is `None`. Delegates `writer.body`/
each condition's `body` to `cel_parser::format_expr`, exactly like
`write_method` does today.

---

## 8. Typechecker (`typecheck.rs`)

`check_sheet` gains a `SheetItem::Out(out)` arm:

- Extend `declared_cell_types` to also map an `OutDecl`'s own name to a
  `Ty` — from its annotation when present, resolved through `registry`
  exactly like a `cell`'s annotation; when absent, from
  `check_expr(&out.writer.body, resolve)`'s *inferred* type (the same
  static inference the checker already performs for method bodies), so a
  later reference to this cell's name still type-checks sensibly even
  without an explicit annotation.
- When the annotation *is* present, cross-check it against the inferred
  body type exactly like `check_method`'s single-output branch (reusing
  that same code path, since an `out_method` is structurally a `method`
  with one implicit output — the out cell itself).
- For each `condition`, run `check_expr(&condition.body, resolve)` and flag
  a diagnostic if the inferred type doesn't unify with `Ty::Bool`, mirroring
  the existing per-output mismatch diagnostics' wording style ("condition
  `{name}` produces `{ty}`, but conditions must be `bool`").

`resolve` is already global across the whole sheet (see `check_method`'s
existing behavior — it is *not* restricted to a method's own declared
`inputs`), so no special resolution scope is needed for a condition's body
even though its `cell_list` is nominally separate from the writer's.

---

## 9. Errors and diagnostics

No new `ParseError`/diagnostic *kinds* — every failure mode maps to an
existing message shape:

| Failure | Surfaced as |
| --- | --- |
| Duplicate `out` name vs. an existing cell | `duplicate cell` `ParseError` (same as `cell`) |
| Unknown `: type_name` | `unknown type` `ParseError` (same as `cell`) |
| Writer body type doesn't match declared annotation | `type mismatch` `ParseError` (same as `method`'s single-output case) |
| Writer body's inferred type isn't registered under any name (annotation omitted) | new message, no new `ParseError` variant |
| Condition body isn't `bool` | new message, no new `ParseError` variant |
| `Error::InvalidOutput` / `Error::TerminalCell` from `Sheet::add_output` | `ParseError` via `.to_string()`, same pattern as `add_relationship`/`add_conditional` errors today |

---

## 10. Worked example

Rewriting the `adam-rs` design doc's motivating scenario end to end:

```text
sheet resize_command {
    cell width: f64;
    cell height: f64;
    cell max_area: f64 = 100.0;
    cell max_width: f64 = 20.0;
    cell max_height: f64 = 20.0;

    out image_size: f64 {
        method [width, height] { width * height }

        condition max_area   [width, height, max_area] { width * height <= max_area }
        condition max_width  [width, max_width]          { width <= max_width }
        condition max_height [height, max_height]         { height <= max_height }
    }
}
```

After `propagate()`, `sheet.output_valid(id)` (via `output_names["image_size"]`)
reflects whether all three conditions currently hold; `violated_conditions`
lists exactly the ones that don't.

---

## 11. Files changed / added

| File | Change |
| --- | --- |
| `adam-lang/src/lib.rs` | Extend grammar doc comment (§3 above) |
| `adam-lang/src/ast.rs` | New `OutDecl`, `OutMethodDecl`, `ConditionDecl`; `SheetItem::Out` variant + its `span`/`set_leading_comment`/`set_blank_line_before` arms |
| `adam-lang/src/parser.rs` | `parse_out_decl`; `ParsedSheet`/`ParseContext` gain `output_names: IndexMap<String, OutputId>` |
| `adam-lang/src/ast_parser.rs` | `parse_out_decl` (AST-only, no `TypeRegistry`) |
| `adam-lang/src/fmt.rs` | `write_out`, `write_condition` |
| `adam-lang/src/typecheck.rs` | `SheetItem::Out` arm in `check_sheet`; extend `declared_cell_types`; new condition-body-must-be-bool diagnostic |

---

## 12. Testing notes

Derived from the grammar and the (already-tested) `adam-rs` contract only:

- `out` with an explicit `: type_name` matching the writer body's actual
  type parses and propagates successfully.
- `out` with no annotation infers the correct type from the writer body.
- `out` with an explicit `: type_name` that mismatches the writer body's
  type is a parse error.
- `out` with no annotation whose writer body produces an unregistered type
  is a parse error.
- `out` with zero, one, and multiple `condition`s all parse; conditions
  evaluate correctly after `propagate()` (spot-checking `output_valid`/
  `violated_conditions` through the new `output_names` lookup).
- A condition naming a cell not yet declared is a parse error (`undeclared
  cell`), consistent with every other cell-list resolution in the grammar.
- Two conditions in the same `out` sharing a name surfaces `add_output`'s
  `Error::InvalidOutput` as a `ParseError`.
- A `method`/`relationship`/`conditional`/second `out` elsewhere in the
  sheet referencing an `out` cell's name as an input surfaces
  `Error::TerminalCell` as a `ParseError`.
- The AST parser (`ast_parser.rs`) parses the same source with no
  `TypeRegistry`, recording an `OutDecl` with unresolved `type_name`/body
  identifiers, and recovers at declaration granularity on a malformed
  `out_method`/`condition_decl`, consistent with `relationship`/`conditional`.
- The formatter round-trips a parsed `OutDecl` (with and without a `:
  type_name` annotation, and with zero/one/multiple conditions) back to
  source text matching the grammar's canonical spacing.
- The typechecker flags a condition body that doesn't infer as `bool`, and
  does not flag one that does; it cross-checks an explicit `out` annotation
  against the writer body's inferred type the same way it already does for
  `method`.

---

## 13. Deferred / out of scope

- `begin`'s graph visualization (`begin/src/bridge.rs`) rendering `out`
  cells/conditions as distinct node/link kinds in the D3 graph view — a
  separate, later pass over `begin`, not part of adam-lang's own grammar.
- `adam-lsp` diagnostics/hover and the VS Code extension's syntax
  highlighting (`editors/vscode-adam-lang`) picking up the new keywords —
  follows naturally once the AST/typechecker land, but is its own pass over
  those crates, not designed in detail here.
- Conditionally-active outputs, and any other item deliberately deferred by
  the `adam-rs` design doc's own §12 — unchanged; this doc only adds syntax
  for what `adam-rs` already implements.
- Parameterized/composable `sheet`s (`docs/VISION.md`'s `adam-lang`
  section) interacting with `out` — not addressed; that feature doesn't
  exist yet either.
