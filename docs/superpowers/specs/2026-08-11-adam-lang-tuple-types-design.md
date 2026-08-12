# adam-lang tuple types

## Problem

adam-lang cannot declare a cell whose type is a CEL tuple, either explicitly
(`cell a: (i32, (f64, String));`) or by deduction from an initializer (`cell a = (1, 2.5);`).

## Background: what already exists

`cel-runtime` fully supports CEL tuples *as expression values* and, separately, as a
persistent, owned cell-storable value:

- **Expression level** (`cel-parser`): tuple literals (`(1, 2)`, arbitrarily nested), `.N` field
  indexing, `TupleOpSignature`-based operator overloads. On `DynSegment`, a tuple lives as an
  ordinary stack value tagged with the `DynTuple` marker `TypeId`; its *real* shape is the
  `associated: Vec<AssociatedType>` list on its `StackInfo`, which nests recursively for nested
  tuples (`AssociatedType.associated`). `tuple_shapes_match`/`drop_tuple` already recurse into
  this nesting; nothing here is limited to a single level.
- **Persistent storage** (`cel-runtime::DynamicSequence`, merged in PR #81 /
  `docs/superpowers/specs/2026-08-10-dynamic-sequence-tuple-cells-design.md`): an owned,
  `Any + Clone + PartialEq + 'static` type that can hold a tuple value independent of any
  `DynSegment`'s lifetime, and converts to/from concrete Rust tuples (`from_tuple::<T>`,
  `try_into_tuple::<T>`, `try_to_tuple::<T>`, `adapt_fn_1`) or a live stack tuple
  (`DynSegment::call_dyn_as_tuple::<T>`). Every one of these requires the tuple's Rust type `T`
  known at **Rust compile time** via a generic.
- **adam-lang today**: `cell`/`out` type annotations are a single identifier looked up in
  `TypeRegistry` (`by_name: HashMap<String, TypeEntry>`), which stores per-type function pointers
  (`push_arg_fn`, `add_cell_fn`, `call_dyn_fn`, `extract_box_fn`, `default_fn`,
  `add_conditional_fn`), each monomorphized for one concrete registered `T`. Cell type identity
  everywhere (`cell_names: IndexMap<String, (CellId, TypeId)>`, method input/output checks,
  conditional match type) is a flat `std::any::TypeId`. Multi-output methods
  (`method [a,b] -> [sum,diff] { (a+b, a-b) }`) already exist, but as a separate, ad hoc mechanism
  (`CompiledOutputs::{Single,Tuple}`, `DynSegment::call_dyn_tuple` with a
  `&[(TypeId, BoxExtractor)]` list) that predates `DynamicSequence` and doesn't reuse it.

### The core gap

Every existing tuple primitive needs `T` known at Rust-compile-time. adam-lang's tuple *shapes*
are only known once DSL text is parsed, at Rust run time, and are combined by DSL authors in
arbitrarily many ways — there is no way to pre-monomorphize a `TupleSequence` impl for every shape
a user might type. Closing this gap is genuinely new `cel-runtime` surface (a handful of small,
runtime-shape-driven primitives), not just adam-lang-side wiring. This design covers both.

## Goals

- `cell`/`out` type annotations accept a recursive tuple type expression, any element type
  (primitive or previously-registered custom type), arbitrary nesting.
- Cell initializers accept CEL tuple literals (and, as a natural consequence of the chosen
  approach, arbitrary constant-foldable CEL expressions), for both scalar and tuple cells.
- Tuple-typed cells work as method/`out`/`condition` inputs (including `.N` field indexing inside
  the body) and as method/`out` outputs, on equal footing with any other registered type.
- Multi-output methods and single-tuple-typed-output methods are unified into one mechanism.
- Good error messages for tuple shape mismatches (arity, element type, at any nesting depth).

## Non-goals

- Redesigning `RawStack`/`RawSequence` (tracked in stlab/cel-rs#80).
- Tuple *element* wildcards or generics.
- Changing how CEL expressions themselves type-check (`cel_parser::Ty` stays `Any` for tuples,
  exactly as documented today — adam-lang does its own, separate tuple-shape checking).

## Design

### 1. Grammar

```ebnf
type_expr    = identifier | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")" .
cell_type_init = (":" type_expr ["=" or_expression]) | ("=" or_expression) .
out_decl     = "out" identifier [ ":" type_expr ] "{" out_method { condition_decl } "}" .
```

`type_expr`'s shape is deliberately identical to `cel-parser`'s existing `tuple_or_group`
(`cel-parser/src/lib.rs:1065`): `()` is the empty tuple type (0 elements); `(T)` is grouping
(same type as bare `T` — types have no precedence to disambiguate, but staying symmetric with
expression grammar costs nothing and avoids a bespoke second convention); `(T,)` is a 1-element
tuple; `(T, U, ...)` is n-element, no trailing comma. Every decision is a single-token peek taken
*after* consuming the preceding token — never two tokens inspected simultaneously — matching
`is_tuple_or_group`'s existing, working implementation.

Cell/out initializers become a full `or_expression`, reusing `parse_cel_or_expression` verbatim
(the same function method/`out` bodies already call) instead of a new parallel literal grammar —
`is_tuple_or_group` already correctly handles `()`/`(x)`/`(x,)`/`(x,y,...)` including nesting, so
duplicating that logic for "just literals" would be pure waste. `AdamParser` parses the
initializer's `or_expression` into a `DynSegment` with **no cell scope pushed** (initializers
cannot reference other cells; a bare identifier fails to resolve, which is exactly the desired
error) and evaluates it **eagerly, once, at parse time** to obtain the concrete initial value
(mirroring how method/`out` bodies are compiled once and evaluated later, except an initializer's
zero-input segment can be evaluated immediately). `AdamAstParser` stores the parsed `Expr`
unevaluated, exactly as it already does for method bodies.

### 2. AST

```rust
pub enum TypeExpr {
    Named(String, ExprSpan),
    Tuple(Vec<TypeExpr>, ExprSpan),
}
```

`CellDecl.type_name`/`OutDecl.type_name`: `Option<(String, ExprSpan)>` → `Option<TypeExpr>`.
`CellDecl.initializer`: `Option<(Literal, ExprSpan)>` → `Option<cel_parser::Expr>` (parsed via
`parse_cel_or_expression`, matching `MethodDecl.body`'s existing representation). Mechanical
fan-out into `ast_parser.rs`, `fmt.rs`, `typecheck.rs`.

### 3. Type identity: `TypeShape`

Every tuple shape erases to the same Rust type (`DynamicSequence`), so `TypeId` alone can no
longer identify a declared cell type once tuples exist. A new recursive identity replaces bare
`TypeId` wherever cell types are tracked:

```rust
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum TypeShape {
    Named(TypeId),
    Tuple(Vec<TypeShape>),
}
```

`cell_names: IndexMap<String, (CellId, TypeShape)>` (was `TypeId`); method input/output checks,
conditional match type, and `out` type checks all switch from `TypeId ==` to `TypeShape ==`
(structural, recursive) wherever a declared cell type is involved. `TypeRegistry` gains:

- `resolve(&self, expr: &TypeExpr) -> Result<TypeShape, String>` — recursive resolution, erroring
  on an unknown leaf name (same message as today's unknown-type error).
- `display_name(&self, shape: &TypeShape) -> String` — recursive, e.g. `"(i32, (f64, String))"`,
  used everywhere a `TypeEntry::type_name` is used for error text today.

`cel_parser::Ty` is untouched — it's not extended with a tuple variant; adam-lang's tuple shape
checking lives entirely in `TypeShape`/`parser.rs`/`typecheck.rs`, parallel to (not integrated
into) `check_expr`, exactly matching the already-documented "tuples aren't type-checked in v1" at
the CEL-expression level.

### 4. New `cel-runtime` primitives

Two additions, both driven by runtime data (a `TypeId`/shape list) rather than a compile-time
generic `T`, both recursive to match the existing `AssociatedType`/`SequenceElement` nesting:

**Output direction** — building a `DynamicSequence` from a live, possibly-nested on-stack tuple
(used by both tuple-typed initializers and tuple-typed method/`out` outputs, since both now
compile to "evaluate a `DynSegment`, then extract its result"):

```rust
impl DynSegment {
    /// Moves the top-of-stack tuple's bytes into an owned `DynamicSequence`, recursing into
    /// nested tuple elements (their own `AssociatedType.associated`) as nested `DynamicSequence`
    /// leaves. `leaf_eq_clone(type_id)` supplies the `Clone`/`PartialEq` function pointers for
    /// each non-tuple leaf (`AssociatedType` itself carries no clone/eq, only a dropper).
    pub fn extract_as_dynamic_sequence(
        &mut self,
        leaf_eq_clone: &impl Fn(TypeId) -> Option<(ElementCloner, ElementEq)>,
    ) -> anyhow::Result<DynamicSequence>;
}
```

Internally built on a lower-level, reusable recursive helper — read a `(base: *const u8,
associated: &[AssociatedType])` region and produce a `DynamicSequence`, recursing whenever an
element's `type_id == TypeId::of::<DynTuple>()`. This same helper is reused by the N-way
multi-output split (section 5) to convert *one element* of a larger top-level tuple into a nested
`DynamicSequence`, not just the whole top-of-stack value.

**Input direction** — pushing a tuple-typed cell's stored `DynamicSequence` onto the stack as a
live, indexable tuple (so a tuple-typed input cell supports `.0`/`.1` inside a method body,
recursing so a nested `DynamicSequence` leaf becomes a nested on-stack tuple, not an opaque blob):

```rust
impl DynSegment {
    /// Pushes a clone of `inputs[index]` (downcast to `&DynamicSequence`) onto the stack as a
    /// tagged `DynTuple`, expanding any nested `DynamicSequence` element into its own nested
    /// on-stack tuple recursively.
    pub fn push_arg_as_dynamic_sequence_tuple(&mut self, index: usize);
}
```

**Defaults** — a tuple-typed cell with no initializer (`cell a: (i32, f64);`) needs a default
value built with no CEL expression to evaluate. One small additional builder, used only for this
case:

```rust
pub struct DynElementSpec { pub type_id: TypeId, pub type_name: Cow<'static, str>,
    pub size: usize, pub align: usize, pub drop: ElementDropper, pub clone: ElementCloner,
    pub eq: ElementEq, pub write: unsafe fn(Box<dyn Any>, *mut u8) }

impl DynamicSequence {
    /// Builds a sequence from boxed leaf values, each paired with its own descriptor.
    pub fn from_dyn_elements(elements: Vec<(DynElementSpec, Box<dyn Any>)>) -> Self;
}
```

adam-lang recurses over a `TypeShape`, gathering each `Named` leaf's `default_fn()` (already a
`Box<dyn Any>` today) plus its descriptor; a `Tuple` leaf recurses into this same construction
first, then boxes the resulting `DynamicSequence` with a descriptor for `DynamicSequence` itself
(reusing its own `Clone`/`PartialEq`/a trivial move-write). `TypeEntry` gains the three new
function-pointer fields (`element_clone`, `element_eq`, `element_write`) needed to populate these
descriptors generically per registered leaf `T` — mechanically identical to what
`push_element::<T>` already computes internally in `cel-runtime`, exposed via a small `pub`
helper rather than duplicated.

### 5. Method-output unification

Today's split (`outputs.len() == 1` → scalar `call_dyn`; `> 1` → `call_dyn_tuple` against N
separate cells, via `CompiledOutputs::{Single, Tuple}`) and the new "one tuple-typed output" case
are one mechanism: **the body must produce a tuple matching the declared outputs' shapes,
combined; each output then takes its corresponding slice.**

- Each declared output's `TypeShape` is checked against the body's result structurally (not
  flat `TypeId ==`): `Named` → leaf `TypeId` equality (unchanged); `Tuple` → the corresponding
  stack element must itself be a nested tuple whose children recursively match (generalizing the
  existing `tuple_shapes_match`, which already recurses this way).
- Single output: if `Named`, unchanged (`call_dyn::<T>`); if `Tuple`, the *whole* top-of-stack
  tuple must match, extracted via `extract_as_dynamic_sequence` into one `DynamicSequence`, stored
  via the already-generic `add_cell_impl::<DynamicSequence>`.
- Multiple outputs (N > 1): body must be an N-arity top-level tuple (unchanged); each element is
  extracted per its own output's `TypeShape` — `Named` outputs use the existing `BoxExtractor`
  path unchanged; a `Tuple`-shaped output among several reuses the same recursive helper from
  section 4 on *that one element's* own `(base, associated)`, then boxes the resulting
  `DynamicSequence` — fitting the existing `Vec<Box<dyn Any>>` return convention `Method`'s
  closure already uses, unchanged.

This replaces `CompiledOutputs::Tuple`'s `Vec<(TypeId, BoxExtractor)>` with a
`Vec<(TypeShape, ExtractorKind)>` where `ExtractorKind` is `Scalar(BoxExtractor)` (existing) or
`Tuple` (new, using the recursive helper). `CompiledOutputs::Single` gains a sibling `SingleTuple`
case using the whole-tuple extraction. Test coverage for today's existing multi-output behavior
must keep passing unchanged — this is a refactor of its implementation, not its observable
behavior.

One tradeoff worth naming: unlike `DynamicSequence::from_tuple::<T>` (compile-time `T`, one flat
inline allocation for an entire nested tuple), this runtime-shape-discovered path allocates once
per `DynamicSequence` nesting level (each nested tuple owns its own heap buffer). This is
inherent to shapes only being known after parsing DSL text — not a correctness concern, just a
minor cost difference from the compile-time-`T` API PR #81 already shipped.

### 6. Parser wiring (`adam-lang/src/parser.rs`)

- `parse_cell_decl`: resolve `TypeExpr` → `TypeShape` via `TypeRegistry::resolve`. With an
  initializer, parse it as an `or_expression` (no scope), check its result's shape against the
  declared `TypeShape` (or infer the cell's type from it, when no annotation is given — mirroring
  today's `infer_and_parse_literal`, generalized to tuples: an unannotated tuple initializer's
  element types are inferred the same way a bare scalar literal is today, recursively), then
  evaluate eagerly and store via the scalar or tuple construction path. Without an initializer,
  build a default recursively (section 4) — erroring if any leaf lacks a default, same as today.
- `parse_out_decl`: same `TypeExpr` resolution; writer body checked/extracted per section 5's
  single-output case (an `out` cell is structurally "a method with one implicit output").
- `parse_method_body`: generalized per section 5.
- `parse_body_with_input_scope` (drives `push_arg` wiring for method/`out`/`condition` inputs):
  when a cell's `TypeShape` is `Tuple`, push via `push_arg_as_dynamic_sequence_tuple` instead of
  the scalar `push_arg_fn`.
- `parse_conditional_decl`: match-cell type resolved to `TypeShape`; branch literal values parsed
  the same way cell initializers now are (an `or_expression`, evaluated eagerly, shape-checked
  against the match cell's `TypeShape`). `add_conditional_fn` needs no change for tuple types —
  `add_conditional_impl::<DynamicSequence>` already works today since `DynamicSequence: PartialEq`.

`AdamAstParser`/`ast_parser.rs` mirrors the grammar changes structurally (recursive `TypeExpr`,
`or_expression`-based initializer) with no type resolution or evaluation, matching its existing
"no semantic checks" design.

### 7. `typecheck.rs` and `fmt.rs`

`typecheck.rs`: `declared_cell_types`/`check_cell_initializer`/`check_method`/`check_out` extended
to compare recursively against `TypeShape` instead of a flat `Ty`/`TypeId`, in parallel to (not
routed through) `cel_parser::ty::check_expr` — mirroring section 5's real-parser logic so
diagnostics match runtime errors. `fmt.rs`: recursive pretty-printing for `TypeExpr` (`(i32, (f64,
String))`) and the now-`Expr`-typed initializer (reusing the existing CEL `Expr` formatter used
for method bodies).

## Error handling

Unknown leaf type name in a `type_expr` → same "unknown type" error as today, at the leaf's span.
Shape mismatch (arity or element type, at any nesting depth) between a declared `TypeShape` and an
initializer/method-output body → a new diagnostic naming the full expected vs. actual shape via
`TypeRegistry::display_name`, at the mismatched sub-expression's span (not just the whole
initializer's span, when the mismatch is nested). A tuple-typed cell missing a default (any leaf
lacking one) → same "type has no default; provide `= ...`" error as today, naming the specific
leaf that lacks one.

## Testing strategy

Contract-level tests per the workspace convention:

- Grammar round-trips: explicit tuple type + initializer, deduced (unannotated) tuple type,
  nested tuples, `()`, `(T)` grouping, `(T,)` singleton — both `AdamParser` and `AdamAstParser`.
- Type/shape mismatch diagnostics: arity, element type, at top level and nested, for cell
  initializers and for method/`out` outputs.
- Tuple cell as a method input, exercising `.0`/`.1` field access inside the body.
- Tuple cell as a single method/`out` output (whole-tuple extraction) and as one of several
  outputs alongside scalar outputs (nested-element extraction).
- Existing multi-output-method tests continue passing unchanged (refactor, not behavior change).
- Tuple cell as a `conditional` match cell (exercises `DynamicSequence: PartialEq` through the new
  DSL path, complementing PR #81's direct-Rust-API coverage of the same mechanism).
- Formatter round-trips for tuple type annotations and tuple initializers.
- `cel-runtime`: `extract_as_dynamic_sequence`/`push_arg_as_dynamic_sequence_tuple`/
  `from_dyn_elements` — round-trips, nesting, shape-mismatch `Err` cases, drop-safety
  (no double-free/leak) via the existing `DropCounter` pattern.

## Out of scope (deferred)

- General CEL-expression cell initializers *beyond* what falls out of reusing `or_expression`
  (e.g. no attempt to support referencing other cells, sheet-level constants, or functions with
  side effects in an initializer — an initializer that can't be evaluated with zero inputs is a
  parse error, same as an unresolvable identifier is today).
- Tuple element wildcards/generics.
- `RawStack`/`RawSequence` redesign (stlab/cel-rs#80).
