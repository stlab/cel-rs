# Reference Manual

This appendix collects Adam's grammar, type rules, solver behavior, and error messages.

## Grammar

```text
module             = { use_decl | sheet }.
use_decl           = "use" qualified_name [ "as" identifier ] ";".
sheet              = "sheet" identifier [ "(" [ parameter { "," parameter } ] ")" ]
                       "{" { sheet_item } "}".
parameter          = identifier ":" type_expr.
sheet_item         = [ doc_comment ] (cell_decl | relationship_decl | conditional_decl | out_decl
                       | source_decl | relationship_use).

cell_decl          = "cell" identifier cell_type_init [ cell_filter ] [ require_block ] ";".
cell_type_init     = (":" type_expr ["=" expression]) | ("=" expression).
cell_filter        = "filter" expression.
source_decl        = "source" identifier cell_type_init [ cell_filter ] [ require_block ] ";".

type_expr          = identifier [ "(" type_expr { "," type_expr } ")" ]
                    | "[" type_expr "]"
                    | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".

relationship_decl  = "relationship" "{" { binding } "}".
relationship_use   = "relationship" qualified_name "(" [ actual { "," actual } ] ")" ";".
actual             = identifier ":" identifier.
qualified_name     = identifier { "::" identifier }.
binding            = binding_target ":=" expression ";".
binding_target     = identifier | "(" identifier "," [ identifier { "," identifier } ] ")".

conditional_decl   = "conditional" expression "{" { conditional_branch } "}".
conditional_branch = (literal_pattern | "_") "=>" "{"
                       { relationship_decl | relationship_use } "}".

out_decl           = "out" identifier [ ":" type_expr ] ":=" expression
                       [ cell_filter ] [ require_block ] ";".
require_block      = "require" "{" { requirement } "}".
requirement        = [ "@" identifier ] expression ";".
```

`expression`, literals, identifiers, operators, `if`/`else`, casts, ranges, closures,
and function calls use CEL grammar; see
[expressions are CEL](expressions.md#expressions-are-cel).

`cell_filter` and `require_block` may attach to `cell_decl`, `source_decl`, and `out_decl` (see
[a source cell can be filtered too](source.md#a-source-cell-can-be-filtered-too)).

<!-- [knowledge]
Document only implemented productions here. A combined cell declaration with a
trailing := expression is a design proposal, not accepted syntax; keep it out of
the grammar until the parser implements it.
-->

## Sheets and namespaces

- Root names identify explicit host selections. Parameterized sheet names identify
  components used by named `relationship` invocations.
- `cell`, `source`, and `out` declarations share one namespace. Declaring the same name twice,
  in any combination of the three, is a "duplicate cell" error.
- Record constructor/type names occupy a separate namespace from values.
  `TypeName { field: expr, ... }` uses an installed schema; its name and field
  labels are not dependencies. Members `expr.name` and `expr.name()` dispatch
  a unary operation directly, ignoring same-named cells; `expr.name(a)` passes
  the receiver first. `(expr.name)()` instead calls the operation's result.
  Only value references contribute dependencies, including inside closures.
- Cell dependencies must be declared earlier in the same sheet.
  Referencing an undeclared name is
  an "undeclared cell" error. See [names and declaration order](cells.md#names-and-declaration-order).

## Components, modules and roots

- `sheet app { ... }` defines a root; `sheet component(...) { ... }` defines a
  reusable component, even with an empty parameter list. A root is not an importable
  component. Imports refer to declared exports, not imported aliases.
- `relationship component(formal: actual, ...);` binds each formal exactly once.
  Binding order does not matter. Actuals must be distinct, previously declared
  ordinary cells or fixed sources with exactly matching complete concrete types.
  Expressions, literals and `out` cells cannot be actuals. Parameters have no
  initializers, filters or requirements, and cannot become private outputs.
- Components may own private cells, outputs and policies. Their local names do not
  enter the root's public name maps. Each use allocates fresh private state and
  relationships; repeated uses do not merge relationships.
- Callers retain boundary filters, requirements, strength and source aspirations.
  Fixed-source actuals remove every method that would write them, including nested
  writes. This specialization uses cell kind, not the last round's `is_source`.
  An empty specialized relationship fails construction, even in an inactive branch.
- Conditional uses require a transitively relationship-only component. Private
  cells, outputs or policies make a definition ineligible there. Unconditional uses
  may contain those declarations. Final combined graphs must satisfy the ordinary
  conditional guard-independence rules.
- Hosts supply explicit `ModulePath` identities and source owners to `ModuleSet`.
  The core performs no filesystem IO, path inference or package discovery.
  `ModuleCompiler::compile` checks all definitions, including unused ones, without
  executing host expressions. Install operation libraries and register custom types
  before creating the immutable compiler or parser.
- `CompiledModules::roots` enumerates root names;
  `instantiate_root(&module, name)` selects one explicitly. Component-only modules
  need no root. `AdamParser::parse_str` is the convenience entry for a source with
  exactly one root and no externally supplied imports.
- Construction runs defaults, initializers and factories, but never propagates.
  Compiled templates and live methods retain their immutable environments after
  earlier owners drop. Native factories must create fresh execution state and obey
  the pure-method contract. A failed native instantiation leaves its destination
  safely discardable, not necessarily unchanged.
- Compile/construction diagnostics own their actual source identities and labels;
  runtime errors use `ParsedSheet::locate_error`. Source-free native diagnostics
  retain native context without manufactured locations. Checked division, inversion
  and conversion failures still require handling. Requirement violations instead
  set validity flags, which callers inspect after propagation.
- Components have concrete types, not generic type parameters. They do not infer
  equations, inject caller policies or turn private outputs into bindable cells.

The [relationships chapter](relationships.md#reuse-a-component) executes inline and
imported examples. For VS Code, map every module, including the importing root:

```json
{
  "adam-lang.modules": {
    "relationships": "adam-lang/examples/relationships.adm2",
    "composed": "adam-lang/examples/composed.adm2",
    "image_constraints": "cel-euclid/examples/image_constraints.adm2",
    "resize_straighten": "cel-euclid/examples/resize_straighten.adm2"
  }
}
```

Paths are relative to the first workspace folder. The extension sends owned
`path`/`uri`/`text` snapshots to the language server; the core does not read files.
UI hosts render owning module errors with
`adam_web_ui::format_module_diagnostic(&diagnostic, &Renderer::plain())`.
The separate `format_parse_error` adapter takes the original text and display name
for an anonymous source-only parse; do not substitute root text for imported
definition locations.

## Cells and source cells

- `cell name: T;`: requires `T` to have a registered default; the cell starts at that default.
- `cell name = expr;`: `expr` is evaluated with _no cell scope_: it may not
  reference any other cell. The cell's type is inferred from `expr`'s result type.
- `cell name: T = expr;`: both forms combined; `expr`'s inferred type must equal `T` exactly,
  or the sheet fails to parse with a "type mismatch" error.
- Compound cells ([tuple types](cells.md#tuple-types), arrays, and callable signatures) use
  canonical whole-value ownership. Host code accesses them through `ParsedSheet::read_compound`,
  `write_compound`, and `source_compound` with exact recursive representation witnesses.
  Arrays default to empty, tuples default recursively, and callables require an initializer.
  Every stored cell requires cloning and value equality.
- Compound reads return independent owners; `source_compound` reads the original
  aspiration, even when filtering changes the effective value. Wrong complete
  read/write witnesses fail at the external boundary before a write mutates state.
- Prepared relationship, output, filter, and requirement frames bind recursive
  storage once during sheet construction. Propagation executes the raw CEL
  engine without recursive shape checks. Value-dependent failures still propagate.
- `Fn((A,), R)` and `Fn((), R)` denote one and zero parameters. Callable cells,
  including empty arrays of callables, fail without equality; CEL container
  storage alone does not require equality. Debug formatting is independent.
- `source name: T;`, `source name = expr;`, and `source name: T = expr;` use the
  corresponding `cell` type and initializer rules, but have a fixed source kind.
- Every cell has a fixed _kind_, assigned once at declaration and never reassigned: a plain
  `cell` may be a planner source or claimed as a method's output, chosen per round; a `source`
  cell is always a source, never claimable as any method's output; an `out` cell is always
  derived by its own fixed writer and rejects direct writes. See [source cells](source.md) for
  `source` and [outputs and requirements](outputs.md) for `out`.

See [sheets, cells, and types](cells.md) and [the type registry](#the-type-registry) for the built-in type table.

## The type registry

| Type name | Default |
|---|---|
| `i8` `i16` `i32` `i64` `i128` `isize` | `0` |
| `u8` `u16` `u32` `u64` `u128` `usize` | `0` |
| `f32` `f64` | `0.0` |
| `bool` | `false` |
| `String` | `""` |

A host registers additional Rust types under Adam names with
[`TypeRegistry::register`](../adam_lang/type_registry/struct.TypeRegistry.html#method.register) (with a
`Default`) or
[`TypeRegistry::register_no_default`](../adam_lang/type_registry/struct.TypeRegistry.html#method.register_no_default)
(without one). A type without a default requires an initializer; a bare `: T` declaration
fails at parse time. Registration happens in the host before parsing, not in sheet source.

Record syntax requires a separate `OpLookup::register_record` schema, and dot
operations require host implementations (optionally with registered operation
signatures). The host passes its lookup as the third argument to
`check_sheet(&sheet, &registry, &lookup)` so static checking sees the same
metadata as execution. Known custom annotations and results retain complete
leaf descriptors and do not unify with unrelated types; truly unresolved values
and custom calls without metadata remain best-effort, with all children checked.

Record fields are required exactly once and match their types exactly: no
implicit integer-to-float conversion. Unknown constructors and
unknown/duplicate/missing/mistyped fields are errors. Field evaluation follows
written order. Empty schemas accept `{}`; record fields and call arguments reject
trailing commas. Records must be parenthesized in `if` conditions
and `conditional` subjects unless inside a parenthesized, call, tuple, or array
subexpression. Members are never assignment targets; binding targets remain
whole cell names or tuple patterns. See the
[executable host example](expressions.md#host-records-and-members).

`RangeInclusive<T>` recognition for [range filters](filters.md#range-filters) is
pre-registered for exactly the built-in numeric types above and is not extensible per custom
type in the current design.

## Relationships and the solver

- A `relationship` names one or more `binding`s; exactly one is selected each time the sheet
  resolves.
- Selection is driven by cell _strength_, a write-recency counter: a cell's own declaration
  and any explicit write both bump it, so before any explicit write, declaration order alone
  ranks every cell: later declared is "fresher." The solver tries, freshest first, to leave
  each cell a source (unclaimed by any binding), keeping the attempt only if a valid, acyclic
  assignment still exists across every active relationship. See
  [strength: who gets to stay a source](relationships.md#strength-who-gets-to-stay-a-source).
- Every method in a relationship must reference the same `inputs ∪ outputs` cell set as
  every other method in that relationship, or parsing fails with `` `methods in a
  relationship must reference the same set of cells` ``. See
  [the rules a relationship's methods must satisfy](relationships.md#the-rules-a-relationships-methods-must-satisfy).
- A method's own `outputs` must be duplicate-free, and no two methods in a relationship may
  share an identical `outputs` set, or parsing fails with `` `a method's outputs
  must be duplicate-free, and no two methods in a relationship may share an outputs set` ``.
  See [the rules a relationship's methods must satisfy](relationships.md#the-rules-a-relationships-methods-must-satisfy).
- A *self-referencing method* includes a cell in both its inputs and outputs. See
  [self-referencing methods](relationships-continued.md#self-referencing-methods).
- Resolving the sheet fails with a _conflict_ if no valid assignment exists at all, or a
  _cycle_ if every valid assignment forms a closed dependency loop with no source anywhere in
  it. See [when no assignment exists](relationships.md#when-no-assignment-exists).
- A binding's left-hand side destructures a tuple result element-wise when parenthesized with
  more than one name, or exactly one name plus its required comma. A bare name binds
  the whole result directly; a single parenthesized name without a comma is invalid. See
  [destructuring bindings](relationships-continued.md#destructuring-bindings).
- A `source` cell can never be a binding's output: a `relationship` (or `conditional` branch)
  naming one as an output is a parse-time error. See [source cells](source.md).
- Whether a cell was left unclaimed (a source) by the last resolution is queryable by a host;
  see [the host embedding API](#the-host-embedding-api).

## Conditionals

- The match subject is a cell or a deduced expression; each branch's literal must match its
  inferred type exactly.
- Only the currently-active branch's `relationship`s participate in a round's solve; every
  other branch's relationships are invisible to the planner that round.
- `_ => { ... }`, if present, must be the last branch and matches any value no named branch
  lists. With no default and no match, none of the conditional's relationships are active, and
  their would-be output cells are left as sources.
- A branch body holds only `relationship` declarations: no `cell` declarations, no nested
  `conditional`.
- A relationship with exactly one method is _forced_: its output cell is claimed every
  round, regardless of strength. `Sheet::is_forced` reports this. See
  [forced cells](conditionals.md#forced-cells).

See [conditionals](conditionals.md).

## Filters

- A `filter` clause may follow a `cell`, `source`, or `out` initializer.
  Filters have no names. `_` denotes a candidate of the cell's declared type;
  other cell references are deduced dependencies. A non-range filter must reference `_`
  and return exactly the cell's type. A range filter `lo..=hi` need not reference `_`;
  its bounds must match the cell's numeric type. Violations are parse-time errors.
- Writing a cell never applies its filter. Filtering occurs during resolution.
- A filtered cell preserves its last-written *source* value separately from its
  computed *derived* value. Reads return the derived value when present, otherwise the source.
- A filter attached to a cell a relationship currently claims (a *derived* cell that round) is
  diagnostic-only: it never corrects the value, only flags a mismatch, queryable by a host; see
  [the host embedding API](#the-host-embedding-api). The same is true, unconditionally, of a filter
  on an `out` cell, since an `out` cell is always derived.
- At most one filter per cell. Compound filters must produce the cell's exact recursive shape.

See the [filters chapter](filters.md) for the full model and worked examples.

## Outputs and requirements

- `out name := expr;` declares a cell derived only by `expr`.
  Host writes, relationships, and additional `out` declarations cannot write it.
  Later declarations may read it; see [outputs and requirements](outputs.md).
- `require { [ "@" name ] expr; ... }` attaches boolean checks, each with an optional
  `@name` label to a `cell`, `source`, or `out` declaration, after any filter.
  Each requirement deduces its dependencies separately from the declaration's expression.
  A failing requirement does not stop resolution or value computation; hosts query its
  diagnostic through the [embedding API](#the-host-embedding-api).

See [requirements: diagnostics, not gates](outputs.md#requirements-diagnostics-not-gates) and [cell declarations](cells.md#cell-declarations)
for `require` on a plain `cell`, and [source cells](source.md) for `require` on a `source` cell.

## Error messages

`parse_str` reports malformed sheets as [`ParseError`](../cel_parser/struct.ParseError.html)
with source spans. Success establishes syntactic and structural validity, but resolution
may still fail; see [relationships and the solver](#relationships-and-the-solver).
The following table lists selected messages:

| Message (abbreviated) | Cause |
|---|---|
| `duplicate cell \`name\`` | a `cell`/`source`/`out` name reused |
| `undeclared cell \`name\`` | a name referenced before its declaration |
| `expected \`:\` or \`=\` in cell declaration` | a `cell` with neither a type nor an initializer |
| `expected \`:\` or \`=\` in source declaration` | a `source` with neither a type nor an initializer |
| `type \`T\` has no default; provide \`= ...\`` | a bare `: T` cell where `T` has no `Default` |
| `type mismatch: expected \`T\`, got \`U\`` | a declared type disagreeing with an inferred one |
| `expression produced no value` | a body expression that isn't a value-producing CEL expression |
| `cannot infer a type: unregistered type \`T\`` | a result containing a leaf without an Adam registration |
| `filter must reference \`_\`` | a non-range filter body that never mentions `_` |
| `filter must produce \`T\`` | a filter body whose result type doesn't match the cell |
| `filter range bounds must be \`T\`` | a `lo..=hi` filter whose element type doesn't match the cell |
| `Adam cell equality capability unavailable for \`T\`` | a compound value, such as a callable, lacking value equality |
| `output \`name\`: type mismatch: ...` | a `relationship` binding output's declared vs. actual type |
| `output expression has arity N but M output(s) declared` | a destructuring binding's tuple arity mismatch |
| `requirement[ \`name\`]: expected \`bool\`, got \`T\`` | a `require`ment body that isn't boolean — the `` `name` `` segment appears only when the requirement is labeled; the colon is always present |
| `methods in a relationship must reference the same set of cells` | a relationship's methods have different `inputs ∪ outputs` sets |
| `a method's outputs must be duplicate-free, and no two methods in a relationship may share an outputs set` | two methods in one relationship claim the same `outputs` set, or one method repeats a cell in its own `outputs` |

## The host embedding API

The crate documentation describes the Rust APIs for parsing and driving sheets:

- [`AdamParser`](../adam_lang/struct.AdamParser.html): parses source into a live
  [`ParsedSheet`](../adam_lang/struct.ParsedSheet.html), which derefs to
  [`Sheet`](../adam_rs/sheet/struct.Sheet.html).
- [`TypeRegistry`](../adam_lang/type_registry/struct.TypeRegistry.html): the type-name-to-Rust-type table a
  parser is built with;
  [`TypeRegistry::new`](../adam_lang/type_registry/struct.TypeRegistry.html#method.new) pre-populates the
  built-ins in [the type registry](#the-type-registry).
- [`OpLookup`](../cel_parser/op_table/struct.OpLookup.html): the parser's function-library table.
  This book's examples install `cel-std`.
- [`Sheet`](../adam_rs/sheet/struct.Sheet.html): `read`, `write`, `propagate`, `is_source`,
  `is_forced`, `cell_kind`, `filter_*`, `cell_requirements`, `cell_requirements_valid`,
  `violated_requirements`, and other runtime operations.
- [`AdamAstParser`](../adam_lang/struct.AdamAstParser.html) /
  [`format_sheet`](../adam_lang/fn.format_sheet.html) /
  [`check_sheet`](../adam_lang/fn.check_sheet.html): the span-carrying CST, formatter, and
  static type checker behind the language server and `adam fmt` (see [program style](style.md)),
  distinct from `AdamParser`'s eager compile-to-`Sheet` path.
