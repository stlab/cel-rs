# Appendix A: Reference Manual

This appendix is for looking things up, not reading start to finish: it restates the rules
from Chapters 1–8 in one terse pass, plus the grammar and error messages in full. Where a rule
needs justification or an example, the appropriate chapter is linked instead of repeating it
here.

## A.1 Lexical conventions

An Adam source file is a UTF-8 text file, tokenized as Rust/CEL tokens (via
`proc_macro2`): identifiers, integer and float literals (with optional type suffixes), string
literals, and punctuation, with `//`/`/* */` comments and `///`/`//!` doc comments stripped or
captured as trivia before parsing proper begins. See [Chapter 8](style.md) for comments and
[cel-parser's own lexical grammar](../cel_parser/index.html) for literals.

**Keywords**: `sheet`, `cell`, `relationship`, `conditional`, `out`, `require`, `filter`. None
of these can be used as a cell or sheet name. `_` is not a keyword but is reserved in two
specific positions: a `conditional`'s default branch (`_ => { ... }`,
[5.3](conditionals.md#53-the-default-branch)), and inside a `filter` expression (the candidate
value, [6.1](filters.md#61-grammar)); elsewhere it is an ordinary identifier.

**Punctuation**: `:` (type annotation), `=` (cell initializer), `:=` (binding/output body),
`=>` (conditional branch), `;` (declaration terminator), `,` (list separator), `{ }` (block
delimiters), `( )` (tuple/grouping delimiters).

## A.2 Grammar

```text
sheet              = "sheet" identifier "{" { sheet_item } "}".
sheet_item         = [ doc_comment ] (cell_decl | relationship_decl | conditional_decl | out_decl).

cell_decl          = "cell" identifier cell_type_init [ cell_filter ] ";".
cell_type_init     = (":" type_expr ["=" expression]) | ("=" expression).
cell_filter        = "filter" expression.

type_expr          = identifier
                    | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".

relationship_decl  = "relationship" "{" { binding } "}".
binding            = binding_target ":=" expression ";".
binding_target     = identifier | "(" identifier { "," identifier } [ "," ] ")".

conditional_decl   = "conditional" expression "{" { conditional_branch } "}".
conditional_branch = (expression | "_") "=>" "{" { relationship_decl } "}" [ "," ].

out_decl           = "out" identifier [ ":" type_expr ] ":=" expression
                       [ "require" "{" { requirement } "}" ] ";".
requirement        = identifier ":" expression ";".
```

`expression` and everything it expands to (`literal`, `identifier`, operators, `if`/`else`,
`as` casts, ranges, closures, function calls) is CEL grammar, defined by `cel-parser`; see
[Chapter 3](expressions.md#31-expressions-are-cel).

A `cell_decl`'s grammar also has a design-level provision for an optional trailing
`":=" expression` clause (making a `cell` double as a relationship-bound output in one
declaration), **not implemented** as of this writing; only the `"=" expression` one-time
initializer and the `cell_filter` clause shown above exist today.

## A.3 Sheets and namespaces

- A sheet's own name has no runtime meaning; it is not otherwise referenceable.
- `cell` and `out` declarations share one namespace. Declaring the same name twice, in any
  combination of the two, is a "duplicate cell" error.
- **No forward references.** An identifier is only recognized as a cell dependency if that
  cell was declared earlier in the same sheet's token order. Referencing an undeclared name is
  an "undeclared cell" error. See [2.6](cells.md#26-names-and-declaration-order).

## A.4 Cells

- `cell name: T;`: requires `T` to have a registered default; the cell starts at that default.
- `cell name = expr;`: `expr` is evaluated once, eagerly, with **no cell scope**: it may not
  reference any other cell. The cell's type is inferred from `expr`'s result type.
- `cell name: T = expr;`: both forms combined; `expr`'s inferred type must equal `T` exactly,
  or the sheet fails to parse with a "type mismatch" error.
- A tuple-typed cell (`T` a parenthesized list, [2.5](cells.md#25-tuple-types)) is stored as a
  [`DynamicSequence`](../cel_runtime/dynamic_sequence/struct.DynamicSequence.html) regardless of arity or
  element types.

See [Chapter 2](cells.md) and [A.5](#a5-the-type-registry) for the built-in type table.

## A.5 The type registry

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

`RangeInclusive<T>` recognition for [range filters](filters.md#64-range-filters) is
pre-registered for exactly the built-in numeric types above and is not extensible per custom
type in the current design.

## A.6 Relationships and the solver

- A `relationship` names one or more `binding`s; exactly one is selected each `propagate()`.
- Selection is driven by cell **strength**, a write-recency counter: `add_cell` (i.e. a cell's
  own declaration) and `write()` both bump it, so before any explicit `write()`, declaration
  order alone ranks every cell: later declared is "fresher." The solver tries, freshest
  first, to leave each cell a source (unclaimed by any binding), keeping the attempt only if a
  valid, acyclic assignment still exists across every active relationship. See
  [Chapter 4](relationships.md#42-strength-who-gets-to-stay-a-source).
- `propagate()` fails with [`Error::Conflict`](../adam_rs/error/enum.Error.html#variant.Conflict) if
  no valid assignment exists at all, or
  [`Error::Cycle`](../adam_rs/error/enum.Error.html#variant.Cycle) if every valid assignment forms a
  closed dependency loop with no source anywhere in it. See
  [4.4](relationships.md#44-when-no-assignment-exists).
- A binding's left-hand side destructures a tuple result element-wise when parenthesized with
  more than one name, or exactly one name plus a trailing comma; a bare name or a single
  parenthesized name with no comma binds the whole result directly. See
  [4.5](relationships.md#45-destructuring-bindings).
- [`Sheet::is_source`](../adam_rs/sheet/struct.Sheet.html#method.is_source) reports whether the last
  `propagate()` left a given cell unclaimed.

## A.7 Conditionals

- The match subject is a cell or a deduced expression; each branch's literal must match its
  inferred type exactly.
- Only the currently-active branch's `relationship`s participate in a round's solve; every
  other branch's relationships are invisible to the planner that round.
- `_ => { ... }`, if present, must be the last branch and matches any value no named branch
  lists. With no default and no match, none of the conditional's relationships are active, and
  their would-be output cells are left as sources.
- A branch body holds only `relationship` declarations: no `cell` declarations, no nested
  `conditional`.

See [Chapter 5](conditionals.md).

## A.8 Filters

- `cell_filter = "filter" expression`, trailing a `cell_decl`. `_` inside it denotes the
  candidate value (of the cell's own declared type); every other identifier is a deduced
  dependency. The expression must reference `_` at least once (unless it's a range expression,
  `lo..=hi`, which is exempt) and must produce the filtered cell's own type.
- **`write()` never applies a filter.** A filter is applied live, by `propagate()`, against the
  cell's current value.
- Internally, a filtered cell keeps a raw `source` (last written, untouched by any filter
  forever) and a computed `derived` (the filter's live output, recomputed every `propagate()`);
  `read()` returns `derived` when present, `source` otherwise.
- A filter attached to a cell a relationship currently claims (a *derived* cell that round) is
  diagnostic-only: it never corrects the value, only flags a mismatch via
  [`Sheet::filter_violation`](../adam_rs/sheet/struct.Sheet.html#method.filter_violation) /
  [`Sheet::filter_violated_cells`](../adam_rs/sheet/struct.Sheet.html#method.filter_violated_cells).
- At most one filter per cell; a filter cannot attach to an output cell; a filter cannot (yet)
  attach to a tuple-typed cell.

See [Chapter 6](filters.md) for the full model and worked examples.

## A.9 Outputs and requirements

- `out name := expr;` declares a new, terminal cell: nothing may ever write it directly, not
  a host `write()`, not a `relationship`, not another `out`.
- `require { name: expr; ... }` attaches named boolean checks, each with its own deduced
  dependencies. A failing requirement never stops `propagate()` or the output's own value from
  being computed: it's reported via
  [`Sheet::output_valid`](../adam_rs/sheet/struct.Sheet.html#method.output_valid) /
  [`Sheet::violated_requirements`](../adam_rs/sheet/struct.Sheet.html#method.violated_requirements),
  nothing more.

See [Chapter 7](outputs.md).

## A.10 Error messages

Adam reports every diagnostic as a [`ParseError`](../cel_parser/struct.ParseError.html)
carrying a source span; there is no separate runtime error type for a malformed sheet; if
`parse_str` returns `Ok`, the sheet is syntactically and structurally valid (though it may
still fail *at `propagate()`* for the solver reasons in [A.6](#a6-relationships-and-the-solver)).
Selected messages, verbatim:

| Message (abbreviated) | Cause |
|---|---|
| `duplicate cell \`name\`` | a `cell`/`out` name reused |
| `undeclared cell \`name\`` | a name referenced before its declaration |
| `expected \`:\` or \`=\` in cell declaration` | a `cell` with neither a type nor an initializer |
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

## A.11 The host embedding API

This book documents the *language*; the Rust API a host application uses to parse and drive a
sheet is documented by the crates themselves:

- [`AdamParser`](../adam_lang/struct.AdamParser.html): parses source into a live
  [`ParsedSheet`](../adam_lang/struct.ParsedSheet.html), which derefs to
  [`Sheet`](../adam_rs/sheet/struct.Sheet.html).
- [`TypeRegistry`](../adam_lang/type_registry/struct.TypeRegistry.html): the type-name-to-Rust-type table a
  parser is built with;
  [`TypeRegistry::new`](../adam_lang/type_registry/struct.TypeRegistry.html#method.new) pre-populates the
  built-ins in [A.5](#a5-the-type-registry).
- [`OpLookup`](../cel_parser/op_table/struct.OpLookup.html): the function-library table a parser is
  built with; this book's own examples install `cel-std` via `support::parser` (see
  `adam-lang-book`'s own crate source).
- [`Sheet`](../adam_rs/sheet/struct.Sheet.html): `read`, `write`, `propagate`, `is_source`,
  `filter_*`, `output_*`, and every other runtime operation this book has used throughout.
- [`AdamAstParser`](../adam_lang/struct.AdamAstParser.html) /
  [`format_sheet`](../adam_lang/fn.format_sheet.html) /
  [`check_sheet`](../adam_lang/fn.check_sheet.html): the span-carrying CST, formatter, and
  static type checker behind the language server and `adam fmt` ([Chapter 8](style.md)),
  distinct from `AdamParser`'s eager compile-to-`Sheet` path.
