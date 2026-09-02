# Appendix A: Reference Manual

This appendix is for looking things up, not reading start to finish: it restates the rules
from Chapters 1–11 in one terse pass, plus the grammar and error messages in full. Where a rule
needs justification or an example, the appropriate chapter is linked instead of repeating it
here.

## A.1 Grammar

```text
sheet              = "sheet" identifier "{" { sheet_item } "}".
sheet_item         = [ doc_comment ] (cell_decl | relationship_decl | conditional_decl | out_decl
                       | source_decl).

cell_decl          = "cell" identifier cell_type_init [ cell_filter ] [ require_block ] ";".
cell_type_init     = (":" type_expr ["=" expression]) | ("=" expression).
cell_filter        = "filter" identifier ":" expression.
source_decl        = "source" identifier cell_type_init [ cell_filter ] [ require_block ] ";".

type_expr          = identifier
                    | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".

relationship_decl  = "relationship" "{" { binding } "}".
binding            = binding_target ":=" expression ";".
binding_target     = identifier | "(" identifier { "," identifier } [ "," ] ")".

conditional_decl   = "conditional" expression "{" { conditional_branch } "}".
conditional_branch = (expression | "_") "=>" "{" { relationship_decl } "}" [ "," ].

out_decl           = "out" identifier [ ":" type_expr ] ":=" expression
                       [ cell_filter ] [ require_block ] ";".
require_block      = "require" "{" { requirement } "}".
requirement        = identifier ":" expression ";".
```

`expression` and everything it expands to (`literal`, `identifier`, operators, `if`/`else`,
`as` casts, ranges, closures, function calls) is CEL grammar, defined by `cel-parser`; see
[Chapter 4](expressions.md#41-expressions-are-cel).

`cell_filter` and `require_block` both attach identically to all three of `cell_decl`,
`source_decl`, and `out_decl` — neither has a cell-kind restriction (see
[Chapter 3](source.md#33-a-source-cell-can-be-filtered-too)).

A `cell_decl`'s grammar also has a design-level provision for an optional trailing
`":=" expression` clause (making a `cell` double as a relationship-bound output in one
declaration), **not implemented** as of this writing; only the `"=" expression` one-time
initializer and the `cell_filter`/`require_block` clauses shown above exist today.

## A.2 Sheets and namespaces

- A sheet's own name has no runtime meaning; it is not otherwise referenceable.
- `cell`, `source`, and `out` declarations share one namespace. Declaring the same name twice,
  in any combination of the three, is a "duplicate cell" error.
- **No forward references.** An identifier is only recognized as a cell dependency if that
  cell was declared earlier in the same sheet's token order. Referencing an undeclared name is
  an "undeclared cell" error. See [2.6](cells.md#26-names-and-declaration-order).

## A.3 Cells and source cells

- `cell name: T;`: requires `T` to have a registered default; the cell starts at that default.
- `cell name = expr;`: `expr` is evaluated once, eagerly, with **no cell scope**: it may not
  reference any other cell. The cell's type is inferred from `expr`'s result type.
- `cell name: T = expr;`: both forms combined; `expr`'s inferred type must equal `T` exactly,
  or the sheet fails to parse with a "type mismatch" error.
- A tuple-typed cell (`T` a parenthesized list, [2.5](cells.md#25-tuple-types)) is stored as a
  [`DynamicSequence`](../cel_runtime/dynamic_sequence/struct.DynamicSequence.html) regardless of arity or
  element types.
- `source name: T;` / `source name = expr;` / `source name: T = expr;`: identical rules to the
  three `cell` forms above, evaluated the same way — a `source` declaration is a `cell`
  declaration in every respect except its fixed `CellKind` (below).
- Every cell has a fixed **kind**, assigned once at declaration and never reassigned: a plain
  `cell` may be a planner source or claimed as a method's output, chosen per round; a `source`
  cell is always a source, never claimable as any method's output; an `out` cell is always
  derived by its own fixed writer, never `write()`-able. See [Chapter 3](source.md) for
  `source` and [Chapter 6](outputs.md) for `out`.

See [Chapter 2](cells.md) and [A.4](#a4-the-type-registry) for the built-in type table.

## A.4 The type registry

| Type name | Default |
|---|---|
| `i8` `i16` `i32` `i64` `i128` `isize` | `0` |
| `u8` `u16` `u32` `u64` `u128` `usize` | `0` |
| `f32` `f64` | `0.0` |
| `bool` | `false` |
| `String` | `""` |

A host application can register additional Rust types under new Adam type names via
[`TypeRegistry::register`](../adam_lang/type_registry/struct.TypeRegistry.html#method.register) (with a
`Default`) or
[`TypeRegistry::register_no_default`](../adam_lang/type_registry/struct.TypeRegistry.html#method.register_no_default)
(without one, such a type can only be used with an explicit initializer, never a bare `: T`
declaration). This is a Rust-level embedding decision made before parsing a sheet, not
something sheet source itself can do.

`RangeInclusive<T>` recognition for [range filters](filters.md#54-range-filters) is
pre-registered for exactly the built-in numeric types above and is not extensible per custom
type in the current design.

## A.5 Relationships and the solver

- A `relationship` names one or more `binding`s; exactly one is selected each time the sheet
  resolves.
- Selection is driven by cell **strength**, a write-recency counter: a cell's own declaration
  and any explicit write both bump it, so before any explicit write, declaration order alone
  ranks every cell: later declared is "fresher." The solver tries, freshest first, to leave
  each cell a source (unclaimed by any binding), keeping the attempt only if a valid, acyclic
  assignment still exists across every active relationship. See
  [Chapter 7](relationships.md#72-strength-who-gets-to-stay-a-source).
- Every method in a relationship must reference the same `inputs ∪ outputs` cell set as
  every other method in that relationship, or resolution fails to parse with `` `methods in a
  relationship must reference the same set of cells` ``. See
  [Chapter 7 §7.3](relationships.md#73-the-rules-a-relationships-methods-must-satisfy).
- A method's own `outputs` must be duplicate-free, and no two methods in a relationship may
  share an identical `outputs` set, or resolution fails to parse with `` `a method's outputs
  must be duplicate-free, and no two methods in a relationship may share an outputs set` ``.
  See [Chapter 7 §7.3](relationships.md#73-the-rules-a-relationships-methods-must-satisfy).
- A cell may appear in both a method's inputs and its own outputs — a self-referencing method
  — which is explicitly allowed. See
  [Chapter 8](relationships-continued.md#82-self-referencing-methods).
- Resolving the sheet fails with a **conflict** if no valid assignment exists at all, or a
  **cycle** if every valid assignment forms a closed dependency loop with no source anywhere in
  it. See [7.5](relationships.md#75-when-no-assignment-exists).
- A binding's left-hand side destructures a tuple result element-wise when parenthesized with
  more than one name, or exactly one name plus a trailing comma; a bare name or a single
  parenthesized name with no comma binds the whole result directly. See
  [8.1](relationships-continued.md#81-destructuring-bindings).
- A `source` cell can never be a binding's output: a `relationship` (or `conditional` branch)
  naming one as an output is a parse-time error. See [Chapter 3](source.md).
- Whether a cell was left unclaimed (a source) by the last resolution is queryable by a host;
  see [Appendix A.10](#a10-the-host-embedding-api).

## A.6 Conditionals

- The match subject is a cell or a deduced expression; each branch's literal must match its
  inferred type exactly.
- Only the currently-active branch's `relationship`s participate in a round's solve; every
  other branch's relationships are invisible to the planner that round.
- `_ => { ... }`, if present, must be the last branch and matches any value no named branch
  lists. With no default and no match, none of the conditional's relationships are active, and
  their would-be output cells are left as sources.
- A branch body holds only `relationship` declarations: no `cell` declarations, no nested
  `conditional`.
- A relationship with exactly one method is **forced**: its output cell is claimed every
  round, regardless of strength. `Sheet::is_forced` reports this. See
  [Chapter 9 §9.3](conditionals.md#93-forced-cells).

See [Chapter 9](conditionals.md).

## A.7 Filters

- `cell_filter = "filter" identifier ":" expression`, trailing a `cell_decl`, `source_decl`, or
  `out_decl` — a filter attaches to any cell kind, with no per-kind restriction (see
  [Chapter 3](source.md#33-a-source-cell-can-be-filtered-too)). The
  identifier names the filter, surfaced through the host embedding API
  ([A.10](#a10-the-host-embedding-api)); it is not a cell reference. `_` inside the expression
  denotes the candidate value (of the cell's own declared type); every other identifier is a
  deduced dependency. The expression must reference `_` at least once (unless it's a range
  expression, `lo..=hi`, which is exempt) and must produce the filtered cell's own type.
- **Writing a cell never applies a filter.** A filter is applied live, each time the sheet
  resolves, against the cell's current value.
- A filtered cell keeps a raw **source** value (last written, untouched by any filter forever)
  and a computed **derived** value (the filter's live output, recomputed every time the sheet
  resolves); reading the cell returns the derived value when present, the source value
  otherwise.
- A filter attached to a cell a relationship currently claims (a *derived* cell that round) is
  diagnostic-only: it never corrects the value, only flags a mismatch, queryable by a host; see
  [Appendix A.10](#a10-the-host-embedding-api). The same is true, unconditionally, of a filter
  on an `out` cell, since an `out` cell is always derived.
- At most one filter per cell; a filter cannot (yet) attach to a tuple-typed cell.

See [Chapter 5](filters.md) for the full model and worked examples.

## A.8 Outputs and requirements

- `out name := expr;` declares a new cell, always derived by `expr` and never writable
  directly — not by a host write, not a `relationship`, not another `out` — but otherwise an
  ordinary, freely-referenceable cell: any later declaration may read it by name exactly like
  any other already-declared cell. See [Chapter 6](outputs.md).
- `require { name: expr; ... }` attaches named boolean checks. Unlike `filter`, `require` is
  not tied to `out`: a `require` block may trail a `cell`, `source`, or `out` declaration's
  initializer, with the same meaning in every case. Each `requirement`'s own dependencies are
  deduced separately from its declaration's own expression. A failing requirement never stops
  the sheet from resolving, or its cell's own value from being computed: it's reported as a
  diagnostic, nothing more, queryable by a host (see
  [Appendix A.10](#a10-the-host-embedding-api)).

See [Chapter 6](outputs.md#63-requirements-diagnostics-not-gates) and [Chapter 2](cells.md#22-cell-declarations)
for `require` on a plain `cell`, and [Chapter 3](source.md) for `require` on a `source` cell.

## A.9 Error messages

Adam reports every diagnostic as a [`ParseError`](../cel_parser/struct.ParseError.html)
carrying a source span; there is no separate runtime error type for a malformed sheet; if
`parse_str` returns `Ok`, the sheet is syntactically and structurally valid (though it may
still fail *when resolved* for the solver reasons in [A.5](#a5-relationships-and-the-solver)).
Selected messages, verbatim:

| Message (abbreviated) | Cause |
|---|---|
| `duplicate cell \`name\`` | a `cell`/`source`/`out` name reused |
| `undeclared cell \`name\`` | a name referenced before its declaration |
| `expected \`:\` or \`=\` in cell declaration` | a `cell` with neither a type nor an initializer |
| `expected \`:\` or \`=\` in source declaration` | a `source` with neither a type nor an initializer |
| `type \`T\` has no default; provide \`= ...\`` | a bare `: T` cell where `T` has no `Default` |
| `type mismatch: expected \`T\`, got \`U\`` | a declared type disagreeing with an inferred one |
| `expression produced no value` | a body expression that isn't a value-producing CEL expression |
| `cannot infer a type for this expression; register a type name for it or add an explicit \`: type_expr\` annotation` | an expression whose result type isn't registered |
| `filter must reference \`_\`` | a non-range filter body that never mentions `_` |
| `filter must produce \`T\`` | a filter body whose result type doesn't match the cell |
| `filter range bounds must be \`T\`` | a `lo..=hi` filter whose element type doesn't match the cell |
| `filter on a tuple-typed cell is not yet supported` | `filter` attached to a tuple-typed `cell` |
| `output \`name\`: type mismatch: ...` | a `relationship` binding output's declared vs. actual type |
| `output expression has arity N but M output(s) declared` | a destructuring binding's tuple arity mismatch |
| `requirement \`name\`: expected \`bool\`, got \`T\`` | a `require`ment body that isn't boolean |
| `methods in a relationship must reference the same set of cells` | a relationship's methods have different `inputs ∪ outputs` sets |
| `a method's outputs must be duplicate-free, and no two methods in a relationship may share an outputs set` | two methods in one relationship claim the same `outputs` set, or one method repeats a cell in its own `outputs` |

## A.10 The host embedding API

This book documents the *language*; the Rust API a host application uses to parse and drive a
sheet is documented by the crates themselves:

- [`AdamParser`](../adam_lang/struct.AdamParser.html): parses source into a live
  [`ParsedSheet`](../adam_lang/struct.ParsedSheet.html), which derefs to
  [`Sheet`](../adam_rs/sheet/struct.Sheet.html).
- [`TypeRegistry`](../adam_lang/type_registry/struct.TypeRegistry.html): the type-name-to-Rust-type table a
  parser is built with;
  [`TypeRegistry::new`](../adam_lang/type_registry/struct.TypeRegistry.html#method.new) pre-populates the
  built-ins in [A.4](#a4-the-type-registry).
- [`OpLookup`](../cel_parser/op_table/struct.OpLookup.html): the function-library table a parser is
  built with; this book's own examples install `cel-std` via `support::parser` (see
  `adam-lang-book`'s own crate source).
- [`Sheet`](../adam_rs/sheet/struct.Sheet.html): `read`, `write`, `propagate`, `is_source`,
  `is_forced`, `cell_kind`, `filter_*`, `cell_requirements`, `cell_requirements_valid`,
  `violated_requirements`, and every other runtime operation this book has used throughout.
- [`AdamAstParser`](../adam_lang/struct.AdamAstParser.html) /
  [`format_sheet`](../adam_lang/fn.format_sheet.html) /
  [`check_sheet`](../adam_lang/fn.check_sheet.html): the span-carrying CST, formatter, and
  static type checker behind the language server and `adam fmt` ([Chapter 11](style.md)),
  distinct from `AdamParser`'s eager compile-to-`Sheet` path.
