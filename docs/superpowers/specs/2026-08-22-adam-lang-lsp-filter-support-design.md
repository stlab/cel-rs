# `adam-lang`/`adam-lsp` filter support (CST, formatter, type-checking, syntax coloring)

**Status:** Approved for implementation planning
**Crates:** `cel-parser`, `adam-lang`, `editors/vscode-adam-lang` (no `adam-rs`/`adam-lsp` source changes needed — `adam-lsp` inherits the fix for free by already depending on `AdamAstParser`/`format_sheet`/`check_sheet`)

## Motivation

`cell`-level `filter` clauses already work end-to-end for `adam-rs`/`begin`, via the
runtime-building parser (`adam-lang/src/parser.rs::parse_cell_filter`, `AdamParser`). They do
not work at all for the language server: `adam-lsp`'s diagnostics and `textDocument/formatting`
are both built on the separate, span-carrying CST parser (`AdamAstParser` in `ast_parser.rs` +
`ast.rs` + `fmt.rs` + `typecheck.rs`), and that parser's `cell_decl` production has no `filter`
case at all. A `.adm2` file containing a `filter` clause today gets a spurious "expected `;`"
syntax diagnostic, and `textDocument/formatting` silently refuses to format it (matching
`rustfmt`'s "won't format code it can't fully parse" behavior — but here that's a gap, not
`rustfmt`'s intended one-syntax-error-per-file case). The `editors/vscode-adam-lang` TextMate
grammar also doesn't list `filter` as a keyword, so it isn't colored as one.

Tracing why the CST parser can't just add a `filter` case surfaces the real blocker: a filter's
body is a closure literal (`|value: T, ...| ...`), and `cel_parser::Expr` — the shared,
span-carrying expression AST that `adam-lang`'s CST layer, its formatter, and its type checker
all build on — has **no `Closure` variant at all**. Closure literals are compiled directly to a
runtime `DynClosure` by `DynSegmentContext` (`cel-parser/src/parser_context.rs`); the
`ParserContext` trait's `push_closure` method defaults to `Err("closures are not supported in
this context")` specifically so `AstContext` (the AST-building implementation) compiles without
needing to override it — a placeholder the trait's own doc comment flags as expected to be filled
in later. So this is not filter-specific: **no closure literal survives `AdamAstParser` today**,
anywhere it might appear. Fixing `filter` for the LSP means giving closures a real AST shape in
`cel-parser` first, then building `filter`'s CST support on top of that — the same two-crate
shape this codebase has used for prior grammar additions (e.g. the conditional
match-expression work, `2026-08-14-adam-lang-conditional-expressions-design.md`).

Confirmed while investigating: `AstContext::apply_op` (`cel-parser/src/ast.rs:310`) already
builds a plain `Expr::Ident` for *any* arity-0 name, resolved or not, and never consults
`op_lookup` — so a closure body referencing its own parameters needs no new identifier-scoping
mechanism in `cel-parser` to parse correctly; it already "just works" the same way a
`relationship` binding referencing an as-yet-undeclared cell already does. The two real gaps are
narrower: (1) the shared `is_closure_expression` grammar production
(`cel-parser/src/lib.rs:1333`) requires `body.output_type_id()` to return a real `TypeId` before
it will even call `push_closure` — `AstContext`'s default `output_type_id` returns `None`, so
today a closure literal fails with a misleading "closure body must produce exactly one value"
error before `push_closure`'s own default is ever reached; and (2) `push_closure` itself has no
`AstContext` override.

## 1. `cel-parser`: give closures an AST shape

Add to `cel-parser/src/ast.rs`:

```rust
/// `closure_expression = ("||" | "|" [ closure_param { "," closure_param } ] "|") expression.`
Closure {
    /// The closure's declared parameters, in source order.
    params: Vec<ClosureParam>,
    /// The closure's body expression.
    body: Box<Expr>,
    /// The span of the whole closure literal, from its opening `|`/`||` through `body`.
    span: ExprSpan,
},
```

with

```rust
/// `closure_param = identifier ":" closure_type_expression.`
#[derive(Debug, Clone)]
pub struct ClosureParam {
    pub name: String,
    pub name_span: ExprSpan,
    pub type_expr: ClosureParamTypeExpr,
}

/// `closure_type_expression = identifier | "(" [ closure_type_expression { "," closure_type_expression } ] ")".`
///
/// Unresolved — mirrors `adam_lang::ast::TypeExpr`'s shape exactly (a bare name, or a
/// recursively-nested tuple), but lives here because closures are a `cel-parser` construct, not
/// an `adam-lang` one. `adam-lang`'s `typecheck.rs` resolves it against its own `TypeRegistry`
/// the same way it already resolves `TypeExpr`.
#[derive(Debug, Clone)]
pub enum ClosureParamTypeExpr {
    Named(String, ExprSpan),
    Tuple(Vec<ClosureParamTypeExpr>, ExprSpan),
}
```

`Expr::span()` gets a `Closure { span, .. } => *span` arm. Every other exhaustive `match` over
`Expr` in both crates (`format_expr`, `check_expr`, any `adam-lang` code matching on `Expr`
variants) needs its new arm added as a direct consequence of the compiler enforcing
exhaustiveness — an implementation-time checklist item, not a design decision.

## 2. `cel-parser`: `is_closure_expression` / `push_closure` — move return-type inference out of the shared path

`is_closure_expression` (`cel-parser/src/lib.rs:1333`) currently computes `return_type:
TypeId` itself (via `body.output_type_id()`) before calling `self.context.push_closure(param_types,
return_type, body, start_span)`. That's only meaningful for `DynSegmentContext`, which needs a
concrete `TypeId` to build a `DynClosure`. Restructure so each `ParserContext` impl decides for
itself:

- `push_closure`'s trait signature (`cel-parser/src/parser_context.rs:136`) drops the
  `return_type: TypeId` parameter, and its `param_types: Vec<TypeId>` parameter is replaced by a
  richer per-parameter list carrying both facets each impl needs — name, name span, the existing
  runtime-facing `ClosureParamType` (`DynSegmentContext` needs its `TypeId`), and the new
  `ClosureParamTypeExpr` (`AstContext` needs the unresolved, displayable shape). Both facets are
  parsed from the same tokens in the same pass — `parse_closure_type_expression`
  (`lib.rs:1397`) is extended to build a `ClosureParamTypeExpr` alongside the `ClosureParamType`
  it already returns, so no token is parsed twice. `push_closure`'s default still returns the
  same "closures are not supported in this context" `Err`.
- `DynSegmentContext::push_closure` (`parser_context.rs:295`) ignores the new list's
  `ClosureParamTypeExpr`/name-span facets, maps its `ClosureParamType`s to `TypeId`s exactly as
  `is_closure_expression` did before this change, calls `body.output_type_id()` itself and
  returns the same "closure body must produce exactly one value"-shaped error if it's `None`,
  then builds `DynClosure::new(param_types, return_type, body.into_inner())` exactly as today.
- `AstContext::push_closure` is added: ignores the list's `ClosureParamType` (runtime) facet,
  converts each entry's name/name-span/`ClosureParamTypeExpr` into a `ClosureParam`, and records
  `Expr::Closure { params, body: Box::new(body.into_expr()), span }`.

This keeps `AstContext` needing no runtime `TypeId`/type registry at all, consistent with its
existing "carries no resolved types, never fails on semantic grounds" design (module doc,
`cel-parser/src/ast.rs:1`).

## 3. `cel-parser`: formatter and type-checker

- `format_expr` (`cel-parser/src/fmt.rs`) gets an `Expr::Closure` arm: `|name: type, ...|
  format_expr(body)` (bare `||` when `params` is empty, matching the grammar's `"||"`
  alternative). Type names print via the same recursive `Named`/`Tuple` rendering
  `adam-lang::fmt`'s `TypeExpr` printing already uses (a small local helper, since
  `ClosureParamTypeExpr` lives in `cel-parser` and can't call into `adam-lang`).
- `check_expr` (`cel-parser/src/ty.rs`) gets an `Expr::Closure` arm returning `Ty::Any` — CEL has
  no first-class function type to check a closure's own type against, matching this checker's
  existing "unresolvable things default to `Ty::Any`, never flagged" policy (module doc,
  `adam-lang/src/typecheck.rs:1`). Filter-specific structural validation (below) happens in
  `adam-lang`, not here, exactly as tuple-shape checking already lives in `adam-lang`'s
  `expr_matches_shape` rather than in `cel-parser`.

## 4. `adam-lang`: grammar and AST

`adam-lang/src/lib.rs`'s `# Grammar` doc comment (line 11) gains:

```text
cell_decl   = "cell" identifier cell_type_init [ cell_filter ] ";".
cell_filter = "filter" [ "(" identifier { "," identifier } ")" ] closure_expression.
```

(`closure_expression` is already defined by `cel_parser`'s own `# Grammar` section, referenced
the same way `or_expression` already is.)

`adam-lang/src/ast.rs`'s `CellDecl` (line 187) gains one field:

```rust
/// The `filter` clause, if present.
pub filter: Option<CellFilter>,
```

with a new struct:

```rust
/// `cell_filter = "filter" [ "(" identifier { "," identifier } ")" ] closure_expression.`
#[derive(Debug, Clone)]
pub struct CellFilter {
    /// The filter's declared argument-cell names, in source order (empty if the `(...)` list
    /// was omitted).
    pub arg_cells: Vec<(String, ExprSpan)>,
    /// The filter's closure literal.
    pub closure: cel_parser::Expr,
    /// The span of the whole `filter ...` clause.
    pub span: ExprSpan,
}
```

## 5. `adam-lang`: `ast_parser.rs`

`AdamAstParser::parse_cell_decl` (`ast_parser.rs:184`) gains, mirroring the real parser's
`parse_cell_decl` (`parser.rs:251`) exactly at the same grammar point (after the initializer,
before the closing `;`):

```rust
let filter = if cursor.is_keyword("filter") {
    Some(self.parse_cell_filter(cursor)?)
} else {
    None
};
```

with a new `parse_cell_filter` mirroring `parser.rs::parse_cell_filter`'s arg-list loop
(`ctx.at_open_paren()` / `expect_open_paren` / `consume_ident` / `expect_close_paren`) but
collecting `(String, ExprSpan)` pairs with no cell-table lookup (no semantic resolution at this
layer, consistent with every other CST production), then `self.parse_cel_or_expression(cursor)`
for the closure — the same entry point `parse_cell_decl`'s own initializer already uses, now
closure-capable per §§1–2.

## 6. `adam-lang`: `fmt.rs`

`write_cell` (`fmt.rs:249`) gains, after the initializer block and before the trailing `;\n`:

```rust
if let Some(filter) = &cell.filter {
    out.push_str(" filter ");
    if !filter.arg_cells.is_empty() {
        out.push('(');
        // join arg_cells by ", "
        out.push_str(") ");
    }
    out.push_str(&cel_parser::format_expr(&filter.closure));
}
```

## 7. `adam-lang`: `typecheck.rs`

`check_sheet`'s `SheetItem::Cell(cell)` arm (`typecheck.rs:42`) gains a call to a new
`check_filter(cell, registry, &shapes, &mut diagnostics)`, run alongside the existing
`check_cell_initializer`. `check_filter`:

1. Resolves each `arg_cells` name against the sheet-wide declared-cell map (`declared_cell_types`,
   already built once per `check_sheet` call at line 38) — an unresolved name is one diagnostic,
   mirroring `parse_cell_filter`'s "undeclared cell `{name}`" runtime error
   (`parser.rs:299`) but, consistent with every other check in this file, with **no
   declaration-order constraint**: `check_binding`/`check_out` already resolve identifiers
   against the full sheet regardless of source order, so a filter's arg cell declared later in
   the same sheet is not flagged here (a stricter ordering check, if ever wanted, belongs to the
   real `parser.rs` path, which already enforces it at actual `Sheet`-build time).
2. Destructures `filter.closure` as `Expr::Closure { params, body, .. }` (always true by
   construction — `parse_cel_or_expression` at this grammar point can only ever produce a
   `Closure`, since `cell_filter`'s grammar requires `closure_expression` unconditionally, not
   `or_expression`; a malformed closure is already a recovered syntax error before type-checking
   runs).
3. Resolves each param's `ClosureParamTypeExpr` against `registry` (a small local recursive
   helper mirroring `TypeRegistry::resolve`'s existing `TypeExpr`-shaped logic) and compares the
   resulting list against `[cell's own declared/inferred `TypeShape`, arg_cells[0]'s shape, arg_cells[1]'s shape, ...]`
   — a length or per-position mismatch is one diagnostic, mirroring
   `parser.rs`'s "filter closure parameter types don't match..." error (`parser.rs:319-327`).
4. Type-checks `body` via `check_expr`, using a resolver that binds each closure param name to
   its own resolved type first, falling back to the sheet-wide `resolve` closure
   (`typecheck.rs:39`) for every other identifier — mirroring how `check_binding`/`check_out`
   already build a per-call resolver — and compares the result against the cell's own type,
   mirroring `parser.rs`'s return-type check (`parser.rs:328`).

## 8. `editors/vscode-adam-lang`

One-line change: add `filter` to the `keyword.declaration.adam-lang` alternation
(`syntaxes/adam-lang.tmLanguage.json:31`), next to `require`. No other grammar rule changes —
closure syntax (`|`, `:`) is already covered by the existing `operators` patterns.

## 9. Out of scope

- No new `adam-lsp` LSP capability (no hover, completion, or go-to-definition) — filters get the
  same diagnostics + formatting support level every other construct already has, matching this
  codebase's current `adam-lsp` feature set exactly.
- No change to closures' *runtime* behavior or to `parser.rs`/`AdamParser` — `parser.rs`'s
  existing `parse_cell_filter` is untouched; this spec only reaches parity for the CST/LSP path.
- No general "closures anywhere in an expression" audit beyond what §§1–3 already provide for
  free — any other CST production that can contain a closure literal (if any exist today) gets
  the same fix as a side effect, but this spec doesn't go hunting for additional call sites to
  change.
- No `TypeRegistry`/`TypeExpr` refactor to unify `adam_lang::ast::TypeExpr` and `cel_parser`'s
  new `ClosureParamTypeExpr` into one shared type — they're structurally identical but live in
  different crates for a good reason (closures are a `cel-parser` concept); a small duplicated
  resolver function in `adam-lang/src/typecheck.rs` is the accepted cost, not a follow-up TODO.

## 10. Testing plan

- **`cel-parser`**: `AstContext` builds `Expr::Closure` for zero-param, multi-param, and
  tuple-param-typed closures (mirroring the existing `DynSegmentContext` closure tests at
  `cel-parser/src/lib.rs:2797` onward, but asserting the parsed `Expr` shape instead of
  calling); `format_expr` round-trips each of those; `check_expr` types a closure literal as
  `Ty::Any` and still type-checks a nested closure inside a closure's body without erroring
  (mirroring `nested_closure_referencing_only_its_own_param_compiles_and_calls`,
  `cel-parser/src/lib.rs:2871`).
- **`adam-lang/src/ast_parser.rs`**: a cell with `filter |v: i32| v;` and one with
  `filter (a, b) |v: i32, a: i32, b: i32| v;` both produce a `CellDecl.filter`
  with the expected `arg_cells`/closure shape; a malformed `filter` clause is recovered as a
  syntax error the same way a malformed cell decl already is.
- **`adam-lang/src/fmt.rs`**: round-trip formatting for both filter forms above.
- **`adam-lang/src/typecheck.rs`**: one diagnostic-producing test per `parser.rs` filter test
  this mirrors — `cell_filter_undeclared_arg_cell_is_a_parse_error`,
  `cell_filter_first_param_type_mismatch_is_a_parse_error`,
  `cell_filter_named_arg_type_mismatch_is_a_parse_error` (`parser.rs:1469-1488`) — asserting a
  diagnostic is returned instead of asserting `Err` from a real parse; plus one clean/no-diagnostic
  case for a correctly-typed filter.
- **`adam-lsp`**: one filter-bearing fixture added to `diagnostics_for_source`'s existing test
  suite (a clean filter produces no diagnostics; a broken one produces exactly one) and to
  `format_edits`'s (a filter-bearing sheet formats instead of silently returning no edits).
- **`editors/vscode-adam-lang`**: no automated test suite exists for the TextMate grammar today
  (confirmed: `editors/vscode-adam-lang` has no grammar-test tooling); manual verification only,
  same as every other keyword already in the grammar.
