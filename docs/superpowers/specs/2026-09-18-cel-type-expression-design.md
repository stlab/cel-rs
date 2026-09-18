# A unified, generic-capable `type_expression` grammar

## Status

Proposed design for the `type_expr` grammar production referenced (but never defined) at
`cel-parser/src/lib.rs:40-41`.

## Goal

`cel-parser` and `adam-lang` each independently parse and represent type expressions today,
despite needing the same recursive shape:

- `cel_parser::type_expr::TypeExpr` — named / array (`[T]`) / tuple (`(T, U)`) — used by array
  type ascriptions (`[0, 1]: [i32]`).
- `cel_parser::ast::ClosureParamTypeExpr` — named / tuple only, no arrays — used by closure
  parameter annotations (`|x: i32| ...`).
- `adam_lang::ast::TypeExpr` — named / tuple only, no arrays — used by `cell`/`out` declaration
  type annotations, hand-parsed twice (`ast_parser.rs` and `parser.rs`).

None of the three can express a parameterized type such as `RangeInclusive<f64>` — the concrete
value type the `..=` range operator already constructs at runtime (see
`cel-parser/src/op_table.rs`'s `RANGE_INCLUSIVE_SIGNATURES` and friends). There is currently no
way to name a range's type in a closure parameter, an adam-lang cell declaration, or an array
element position (`[RangeInclusive(f64)]`), even though the runtime value already exists.

This design defines one shared, reusable `type_expression` grammar production — covering plain
names, arrays, tuples, and parameterized (generic) types — used uniformly by all three call
sites, and extends the built-in type resolver so parameterized range types are ordinary,
fully-capable named types: usable as a closure parameter type, an adam-lang cell type, or an array
element type, with no second-class restriction.

## Grammar

```text
type_expression = identifier [ "(" [ type_expression { "," type_expression } ] ")" ]
                 | "[" type_expression "]"
                 | "(" [ type_expression ["," [ type_expression { "," type_expression } ]] ] ")".
```

- A bare `identifier` names a type with no parameters (`i32`, `Custom`).
- `identifier "(" ... ")"` names a parameterized (generic) type applying zero or more type
  arguments (`RangeInclusive(f64)`, `RangeFull()`, `RangeFull`).
- `"[" type_expression "]"` is an array type, recursively composable (`[i32]`, `[[i32]]`).
- `"(" ... ")"` is tuple/grouping syntax, unchanged from today: `()` is the empty tuple, `(T)` is
  grouping, `(T,)` is a 1-tuple, `(T, U, ...)` is n-ary with no trailing comma.

Dispatch is LL(1): `identifier` / `[` / `(` select the alternative, and an identifier immediately
followed by `(` is a combination with no existing meaning in type position, so
`Name(args)` is unambiguous with adjacent tuple-grouping syntax.

### Why parentheses rather than `Name<T>` or `Name[T]`

Three syntaxes were considered for parameterized types:

- **`Name<T, U>`** (Rust-like angle brackets). Rejected: `>>` is already lexed as a single
  bitwise-shift token by `LexLexer`, so nested generics (`Foo<Bar<T>>`) would require
  token-splitting the same way Rust's own parser must. Solvable, but adds lexer-level complexity
  for no benefit over the alternative below, since type expressions are always parsed from a
  dedicated entry point (never interleaved with value-expression parsing, so there is no
  precedent ambiguity to resolve — only the lexer-level `>>` splitting).
- **`Name[T]`** (bracket application). Rejected: `[T]` already has a fixed, different meaning
  (array-of-`T`), so reusing brackets for generic application would be visually and structurally
  confusing right next to the array production.
- **`Name(T, U)`** (parenthesized application). Chosen: reuses the existing parenthesis/comma-list
  machinery the tuple production already has, introduces no lexer changes (ordinary `)` always
  closes it, no multi-char token to split), and generalizes to any arity and to nesting
  (`InclusiveRange(InclusiveRange(f64))`) for free.

## AST

`cel_parser::type_expr::TypeExpr` gains an `args` field on its named variant:

```rust
pub enum TypeExpr {
    Named {
        name: String,
        /// Type arguments, e.g. `f64` in `RangeInclusive(f64)`. Empty for a plain name.
        args: Vec<TypeExpr>,
        span: ExprSpan,
    },
    Array { element: Box<TypeExpr>, span: ExprSpan },
    Tuple { elements: Vec<TypeExpr>, span: ExprSpan },
}
```

An empty `args` is exactly today's bare-name behavior; every existing construction site
(`TypeExpr::Named { name, span }`) becomes `TypeExpr::Named { name, args: Vec::new(), span }`.

`cel_parser::ast::ClosureParamTypeExpr` and `adam_lang::ast::TypeExpr` are deleted; both call sites
use `cel_parser::TypeExpr` directly (see "Call sites" below).

## Resolver: argument-aware, uniformly capable

```rust
pub trait TypeResolver: Send + Sync {
    fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType>;
}
```

`args` is the already-resolved list of type arguments (empty for a plain name). Existing
implementations (the closure-based blanket impl, the `[(&str, ResolvedLeafType); N]` array impl)
gain the new parameter and ignore it, preserving today's behavior for every existing bare-name
registration.

`TypeExpr::resolve` passes the recursively resolved `args` through to `resolve_named_type` instead
of only handling the no-argument case.

### Built-in generic types

`op_table.rs` already has a `builtin_scalars!` macro that builds, per scalar name, a
`BuiltinScalarType { type_id, type_name, size, align, dropper, push_arg, element_type }` — where
`element_type` is `ArrayElementType::leaf::<T>()`, which has no restriction beyond `T: 'static`.
Nothing about being a parameterized type prevents a value from having full array-element support,
so built-in generics get the same complete descriptor scalars get — not a reduced one.

Add a parallel `builtin_generics!` table, keyed by `(generic name, argument TypeId)`, covering
every numeric type each range operator already supports (`u8`..`u128`, `usize`, `i8`..`i128`,
`isize`, `f32`, `f64`):

- `Range(T)` → `std::ops::Range<T>`
- `RangeInclusive(T)` → `std::ops::RangeInclusive<T>`
- `RangeFrom(T)` → `std::ops::RangeFrom<T>`
- `RangeTo(T)` → `std::ops::RangeTo<T>`
- `RangeToInclusive(T)` → `std::ops::RangeToInclusive<T>`
- `RangeFull` (0 args) → `std::ops::RangeFull`

Each entry is built the same way a scalar entry is: `type_id`, `type_name`, `size`, `align`,
`dropper`, `push_arg`, and `element_type` (a real `ArrayElementType::leaf::<Range<T>>()`, etc.).
`BuiltinTypeResolver::resolve_named_type` looks up the scalar table when `args` is empty and the
generic table when `args` is non-empty (or the type is `RangeFull`, which is generic-shaped but
0-ary), producing a `ResolvedLeafType` exactly as it does for scalars today.

`ResolvedLeafType` itself is unchanged (its `ArrayElementType` field stays non-optional) — every
resolvable named type, parameterized or not, now genuinely has one, so there is no special case to
carry.

This makes `[RangeInclusive(f64)]` (an array of ranges — e.g. representing a multi-part selection)
work with no additional plumbing: it is an ordinary array-of-named-type, exactly like
`[i32]`.

Hosts extending their own custom generic types implement `resolve_named_type(name, args)` directly
and build whatever `ResolvedLeafType` (including its `ArrayElementType`) fits their type, exactly
as they already do for plain named types.

## Call sites

### Array annotations (`cel-parser`)

No behavior change beyond what the grammar/AST changes already provide: `[T]` continues to work
exactly as today, and `[RangeInclusive(f64)]` now resolves as a new capability with no special
casing in `parse_array_expression` or `ParserContext`.

### Closure parameters (`cel-parser`)

`ClosureParam::type_expr` changes from `ClosureParamTypeExpr` to `cel_parser::TypeExpr`. Parsing
uses the shared `parse_type_expression` instead of a separate `parse_closure_type_expression`.
`ty.rs`'s `closure_param_ty` (which maps a closure parameter's declared type to a `Ty` for static
checking) extends its existing `TypeExpr::Named`/`TypeExpr::Array` handling; any type this pass
does not yet model as a concrete `Ty` (a parameterized type, for example) maps to `Ty::Any` —
consistent with the existing "unresolved falls to `Any`" rule (see `ty.rs`'s doc comments), not a
new behavior.

The formatter (`fmt.rs`'s `render_closure_param_type`) is replaced by the existing
`render_type_expr`, extended to render `Name(arg, ...)` for a non-empty `args` list.

### adam-lang cell/out declarations

`adam_lang::ast::TypeExpr` and its two duplicated `parse_type_expr` functions
(`ast_parser.rs`, `parser.rs`) are deleted. Both parsers call `cel_parser`'s shared
`parse_type_expression` (already exposed as `Parser::parse_type_expr`/`parse_type_expr_tokens`).
`CellDecl`/`OutDecl`/`SourceDecl` (and any other adam-lang node currently holding
`adam_lang::ast::TypeExpr`) hold `cel_parser::TypeExpr` instead; span access uses its existing
`.span()`.

`TypeRegistry::resolve` takes `&cel_parser::TypeExpr` and implements `cel_parser::TypeResolver`
(it must gain the `args` parameter on `resolve_named_type` to satisfy the trait; today's registry
has no generic entries of its own, so it simply returns `None` whenever `args` is non-empty, same
as an unrecognized name).

`TypeShape` (`Named(TypeId)` / `Tuple(Vec<TypeShape>)`) is **not** extended with new variants in
this change. A `TypeExpr::Array { .. }` or a `TypeExpr::Named` with non-empty `args` used in
**cell/out declaration position** (i.e. passed to `TypeRegistry::resolve`) is an explicit
resolve-time error: `"array and generic cell types are not yet supported (see issue #227)"`.
This preserves every capability adam-lang has today (named and tuple cell types keep working
exactly as before) while adopting the shared grammar/parser, and turns "not yet handled" into a
clear, documented, actionable diagnostic rather than a silent misinterpretation.

CEL *expression* bodies embedded in adam-lang source (array annotations, casts, closures inside a
cell's initializer expression) are unaffected by this restriction and get full generic/array-of-
generic support immediately, since those go through `cel-parser`'s own resolution, not
`TypeRegistry::resolve`.

Issue #227 tracks extending `TypeShape` (and `type_registry.rs`/`parser.rs`'s cell-construction
machinery) to support array- and generic-typed adam-lang cells natively — a materially larger
change (new `TypeShape` variant, `Sheet`/`DynamicArray`-backed cell storage, conditional/
default-value support for the new shapes) out of scope for this grammar unification. The error
message and the code both reference issue #227.

## Compatibility

- Every existing bare-name type expression (`i32`, `Custom`, `[i32]`, `(i32, f64)`) parses and
  resolves identically; `args` is simply empty.
- `TypeResolver` implementors must add the `args: &[ResolvedType]` parameter to
  `resolve_named_type`; every current implementation in this repository ignores it (source
  change only, no behavior change) except the new built-in generic table.
- `ClosureParamTypeExpr` and `adam_lang::ast::TypeExpr` are removed; both are internal/AST types
  with no stable-API guarantees given the project's pre-release status (see repository
  conventions: prefer clean redesigns over compatibility layers).

## Tests

- `cel-parser::type_expr`: parsing `Name(Arg)`, `Name()`, nested generics
  (`RangeInclusive(RangeInclusive(f64))` as a parse-level exercise, even though it resolves to an
  unregistered type), and existing named/array/tuple cases unaffected.
- `cel-parser::op_table`/`type_expr` resolution: every built-in generic name resolves for every
  supported numeric argument type; an unsupported argument type or unknown generic name is a clear
  "unknown type" diagnostic; `[RangeInclusive(f64)]` round-trips through array construction exactly
  like `[f64]`.
- `cel-parser::fmt`: round-trip formatting of `Name(Arg, ...)` in both array-annotation and
  closure-parameter position.
- `cel-parser::ty`/closures: a closure parameter typed with a built-in generic infers `Ty::Any`
  (documented, not a regression); existing closure param tests continue to pass against the shared
  `TypeExpr`.
- `adam-lang::ast_parser`/`parser`/`type_registry`: shared-parser parity tests (existing named/
  tuple cell declarations parse and resolve identically); a new test asserting the explicit
  "array and generic cell types are not yet supported" diagnostic for both an array-typed and a
  generic-typed cell declaration.
- `adam-lang::typecheck`: unaffected for named/tuple annotations; confirms the new diagnostic does
  not silently fall back to `Ty::Any` acceptance.
