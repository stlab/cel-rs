# adam-lang conditional match-expressions

**Status:** Approved for implementation planning
**Crate:** `adam-lang` (no `adam-rs` changes needed)

## Motivation

[#99](https://github.com/stlab/cel-rs/issues/99) landed native `adam-rs` infrastructure
(`MatchExpr`, merged in this same branch/PR) for a conditional's match subject to be a
method-like expression over multiple input cells, instead of only a single existing cell.
That work deliberately stopped at `adam-rs`: `adam-lang`'s grammar still only accepts
`conditional identifier { ... }`, wrapping the resolved cell in `MatchExpr::cell(...)`.

This spec is the follow-through: let `adam-lang` actually parse
`conditional <or_expression> { ... }` (e.g. `conditional a && b { ... }`), deducing which
declared cells the expression depends on directly from the expression itself, with no
separate `[a, b]`-style declaration. Per the user's explicit direction, this is scoped to:

- **Full stack** — the AST-only parser (`ast_parser.rs`, backing the formatter and
  potentially the LSP) and the formatter (`fmt.rs`) are updated alongside the real,
  Sheet-building parser (`parser.rs`), not deferred to a follow-up.
- **Any output type** — scalar, tuple, or client-registered types, with the same generality
  today's plain-cell conditionals already have for their match cell's type, not just the
  boolean case the issue's own example uses.

## Why this needs new mechanism, not just wiring

Nothing in `adam-lang` today deduces a set of input cells from an expression. Both
`method [a, b] -> [c] { a + b }` and `condition name [a, b] { a + b }` require an *explicit*
cell list; the parser compiles the body by pushing a **fixed-index** identifier scope onto
`cel_parser`'s `OpLookup` — one `push_arg(idx)` op per declared input name, `idx` fixed
before the body is parsed (`parser.rs::parse_body_with_input_scope`).

A conditional match-expression has nowhere to declare that list — `a && b` *is* the whole
syntax. The only way to know which cells it depends on is to discover them while parsing
the expression itself. This spec generalizes the existing fixed-index scope into a
**grow-on-demand** scope: input cells are discovered, and assigned argument indices, during
the same single parse that compiles the expression — no separate expression-tree walker is
introduced (which would otherwise have to be kept in sync with `cel-parser`'s grammar as it
grows; the identifier-resolution hook `cel-parser` already calls on every 0-arity symbol
reference can't miss one by construction).

## 1. Grammar and AST

`conditional_decl`'s grammar production changes from:

```text
conditional_decl = "conditional" identifier "{" { conditional_branch } [ default_branch ] "}".
```

to:

```text
conditional_decl = "conditional" or_expression "{" { conditional_branch } [ default_branch ] "}".
```

This is backward compatible: a bare identifier already *is* a trivial `or_expression`, so
every existing `conditional p { ... }` source keeps parsing identically (as a 1-input
identity expression — see §3 on why this case is not special-cased away). Update the EBNF
comment at `adam-lang/src/lib.rs:15`.

`ast::ConditionalDecl`'s `match_name: String` / `match_name_span: ExprSpan` fields are
replaced with a single `match_expr: cel_parser::Expr`, mirroring `ast::ConditionDecl.body`'s
existing shape exactly (an `Expr`'s own span covers what `match_name_span` covered before, so
no separate span field is needed — same pattern `ConditionDecl` already uses for `body`).

## 2. `ast_parser.rs` and `fmt.rs` (the AST-only path)

`AdamAstParser::parse_conditional_decl` (`ast_parser.rs:290`) replaces
`cursor.consume_ident()` with `self.parse_cel_or_expression(cursor)` — the identical
one-line shape `parse_condition_decl` already uses for `ConditionDecl.body`.

`fmt.rs::write_conditional` replaces `out.push_str(&cond.match_name)` with
`out.push_str(&cel_parser::format_expr(&cond.match_expr))` — the identical pattern
`write_condition` already uses for `cond.body`.

Neither of these compiles anything or touches cell resolution — they only produce/print an
AST node, exactly as they do today for `ConditionDecl`'s body expression.

## 3. The real parser: grow-on-demand identifier scope (`parser.rs`)

`AdamParser::parse_conditional_decl` (`parser.rs:462`) currently does:

```rust
let (match_cell_id, match_shape) =
    ctx.cell_names.get(&match_name).cloned().ok_or_else(...)?;
```

This is replaced by a new method, `parse_match_expr`, that compiles the match expression
into a `DynSegment` while simultaneously discovering its input cells:

1. Create a shared accumulator `Arc<Mutex<Vec<(String, CellId, TypeShape)>>>`, initially
   empty. (`Arc<Mutex<_>>`, not `Rc<RefCell<_>>`, because `cel_parser::OpLookup::push_scope`
   requires its closure to be `Fn(...) + Send + Sync + 'static` — a hard constraint from the
   existing `ScopeFn` type alias, not a new design choice; `RefCell` is not `Sync` and would
   not compile as a scope's captured state.)
2. Push a scope closure (mirroring `parse_body_with_input_scope`'s shape and its `InputPush`
   scalar/tuple dispatch) that, for each 0-arity identifier lookup:
   - looks the name up in `ctx.cell_names`;
   - if not yet present in the accumulator, locks the mutex, assigns it the next index
     (`accumulator.len()`), appends `(name, cell_id, shape)`, and emits the corresponding
     push op for that new index (`InputPush::Scalar`/`InputPush::Tuple`, exactly as
     `parse_body_with_input_scope` already does per input);
   - if already present, reuses its previously assigned index and re-emits the push op for
     it (an expression referencing the same cell twice, e.g. `a && a`, still allocates only
     one argument slot);
   - if not a declared cell name at all, returns `Ok(false)` (falls through — surfaces as
     `cel-parser`'s own "undefined identifier" error, since there's no outer scope at this
     parse point).
3. Call `self.parse_cel_or_expression(ctx)` (the segment-compiling entry point already used
   for method bodies and branch keys in this file — distinct from `ast_parser.rs`'s
   AST-only entry point of the same name) with the scope active.
4. Pop the scope; take the accumulator's contents as the ordered
   `inputs: Vec<(String, CellId, TypeShape)>`.

## 4. Output-shape inference and dispatch

After compiling, the segment's output shape is read from its metadata — `peek_output_type_id`
/ `peek_tuple_arity` / `peek_stack_infos`, the same **non-executing** queries
`eval_segment_boxed` already uses — never `call_dyn`/`call_dyn_as_dynamic_sequence` at parse
time, since the expression must be re-evaluated on every `propagate()`, not once now.

- **`TypeShape::Named(type_id)`** — build the `MatchExprFn` via
  `self.types.entry_by_type_id(type_id).call_dyn_fn` (already used for `Method`'s
  `CompiledOutputs::Single`), and the `eq_fn` via a **new** `TypeEntry.eq_dyn_fn` field
  (§5) — both already-registered dispatch-table lookups, so any client-registered type
  (not just built-ins) works with no extra code, exactly as the existing plain-cell path
  already gets this for free via the same `TypeRegistry`.
- **`TypeShape::Tuple(_)`** — build the `MatchExprFn` via `call_dyn_as_dynamic_sequence`
  (mirroring `CompiledOutputs::SingleTuple`), and use a single fixed (non-generic)
  comparator `fn dynamic_sequence_eq(a: &dyn Any, b: &dyn Any) -> bool` comparing two
  `cel_runtime::DynamicSequence`s via its own `PartialEq` impl — `DynamicSequence` is one
  concrete Rust type regardless of tuple arity/element types, so no per-shape dispatch table
  entry is needed here, matching how today's existing tuple-shaped single-cell conditional
  path already just uses `add_conditional::<cel_runtime::DynamicSequence>` directly.

The compiled closure (regardless of shape) is built exactly like `build_method` builds its
`f`: wrap the `DynSegment` in a `RefCell` (single-threaded reentrant-call safety, matching
`build_method`'s own documented reason), move it into a closure matching
`Fn(&[&dyn Any]) -> Result<Box<dyn Any>, anyhow::Error>`.

## 5. `TypeRegistry`: new `eq_dyn_fn` dispatch entry

```rust
/// Compares two type-erased values of `T`, for `TypeEntry::eq_dyn_fn`.
fn eq_dyn_impl<T: PartialEq + 'static>(a: &dyn Any, b: &dyn Any) -> bool {
    a.downcast_ref::<T>() == b.downcast_ref::<T>()
}
```

Added as a new field `eq_dyn_fn: fn(&dyn Any, &dyn Any) -> bool` on `TypeEntry`, populated
as `eq_dyn_impl::<T>` in both `TypeRegistry::register`/`register_no_default` — no new trait
bound is needed, since both already require `T: PartialEq`. This mirrors `call_dyn_fn`'s
existing pattern exactly: a generic function monomorphized per registered type, with no
captured state, so it coerces to a bare `fn` pointer — `MatchExpr::new`'s existing
`eq_fn: fn(&dyn Any, &dyn Any) -> bool` parameter (unchanged, no `adam-rs` edit required)
already accepts exactly this shape.

## 6. Wiring into `Sheet::add_conditional`

`type_registry.rs`'s `AddConditionalFn` type alias and `add_conditional_impl` change from
taking a bare `CellId` (with `add_conditional_impl` wrapping it internally via
`MatchExpr::cell(cell)`, added as a temporary shim in the prior PR) to taking a `MatchExpr`
value directly — the caller (this new `parse_match_expr` machinery) now always constructs
the right `MatchExpr` itself (`MatchExpr::new(...)`, built per §3/§4), so the internal
wrapping shim is removed.

**Every conditional built by the real parser goes through this one `MatchExpr::new` path
uniformly — including the trivial single-identifier case** (`conditional p { ... }` compiles
to a 1-input identity expression through the same grow-on-demand scope). This is a
deliberate choice not to special-case back to the zero-allocation `MatchExpr::cell` path:
one code path is simpler than two, and the cost is one extra (cheap) closure call per
`propagate()` for what used to be a direct cell read — not worth the special-casing given
nothing here is performance-sensitive.

`parser.rs`'s existing `TypeShape::Tuple(_)` branch in `parse_conditional_decl` (which today
hardcodes `ctx.sheet.add_conditional::<cel_runtime::DynamicSequence>(MatchExpr::cell(match_cell_id), ...)`)
is updated the same way, using the `MatchExpr::new(...)` built per §3/§4 for the tuple case.

## 7. Out of scope

- `adam-lsp`: does not reference `ConditionalDecl`/`match_name` at all today (confirmed via
  workspace search), so no changes are anticipated there. If diagnostics/hover regress in
  practice, that's a bug to fix, not a planned change.
- `editors/vscode-adam-lang`'s TextMate grammar: best-effort syntax highlighting, not a
  strict parser; not expected to need changes for this grammar generalization, and not
  covered by this spec's testing plan.
- No `adam-rs` changes (see §5 — the existing `MatchExpr::new` signature is already
  sufficient).

## 8. Testing plan

- **`ast_parser.rs`**: parsing `conditional a && b { true => {...} }` produces a
  `ConditionalDecl` with `match_expr` reflecting the parsed expression (existing
  single-identifier parse tests continue to pass unchanged, since the old syntax is a
  degenerate case of the new grammar).
- **`fmt.rs`**: round-trip formatting of both a bare-identifier conditional (output
  unchanged from today) and an expression-based conditional.
- **`parser.rs` / real `Sheet` construction**:
  - Two-cell boolean expression (`conditional a && b { true => { relationship { ... } } }`)
    activates the correct branch and reacts to writes on either input cell.
  - An expression referencing the same cell twice (`a && a`, or similar) compiles and
    evaluates correctly with only one argument slot allocated.
  - A tuple-producing match expression (e.g. `(a, b)`) drives branch selection against
    tuple-literal branch keys, matching today's existing tuple-shaped single-cell coverage.
  - A client-registered (non-built-in) type used as a match expression's output type
    dispatches correctly through `TypeRegistry`.
  - An expression referencing an undeclared identifier produces a clear parse error.
  - The existing bare-identifier `conditional p { ... }` tests continue to pass unchanged
    (regression backstop — proves the "no special case" uniform path is behavior-preserving
    for the case it replaces).
- **Grammar doc**: `adam-lang/src/lib.rs`'s `# Grammar` EBNF comment updated to match.
