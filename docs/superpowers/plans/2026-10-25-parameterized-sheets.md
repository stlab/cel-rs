# Parameterized Sheets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build typed reusable sheet templates that compose into one live Adam graph through named cell bindings, from either native construction or `.adm2` modules.

**Architecture:** Add an owning graph-recipe abstraction to `adam-rs`, with source-aware language adapters in `adam-lang`. Parse complete modules structurally, compile retained CEL ASTs into fresh instance frames, and expand uses in declaration order. Module lookup is client-supplied; live sheets retain their executable and source owners independently of module lifetime.

**Tech Stack:** Rust 2024, existing SlotMap/IndexMap/HashMap collections, `proc_macro2`, the CEL runtime/parser, existing LSP transport and TypeScript extension.

**Spec:** `docs/superpowers/specs/2026-10-25-parameterized-sheets-design.md` (approved 2026-10-25).

## Global Constraints

- Every parameter has an explicit concrete type.
- Each parameter is supplied exactly once, and one actual cell cannot fill two parameters in a single use.
- Actual cells may be shared between different component instances.
- Callers supply boundary-cell initial values, filters, and requirements.
- Out-cell arguments are unsupported initially, including read-only uses of caller outputs.
- Source arguments remove whole methods that write them. A relationship left without a method produces an instantiation diagnostic.
- Native construction and instantiation do not require parsing source text.
- Composition follows inline-at-use declaration order and preserves bound-cell identities and priorities.
- A use within a conditional branch may expand only to relationships, with no private cells or conditionals.
- No CEL or filesystem dependency in `adam-rs`; no new third-party dependencies are needed.
- Follow Better Code and Writing: adjacent contracts, public examples, at least one unit test per component, checked signed arithmetic, no success-shaped error fallbacks.
- #261–#264 are deferred outside v1.0; do not implement out arguments, language generics, inferred equations, or general conditional component state.

## Review Focus

- Distinct compound shapes with the same erased carrier must fail binding before construction (Tasks 5–6).
- A source appearing among several outputs removes the entire method, never one tuple field (Task 2).
- An eligible wrapper around an ineligible nested component must not bypass conditional restrictions (Tasks 2 and 6).
- A definition imported into a different environment must resolve its own dependencies, not caller names (Task 5).
- Runtime CEL spans from imported definitions must render against their own source, including repeated instances (Task 7).

---

## File responsibilities and execution order

This is one cohesive feature, not independent subsystem projects. Tasks 1–2 form
the native library substrate. Task 3 and Task 4 can proceed independently after
their interfaces are agreed; Tasks 5–6 join the language and native paths.
Tasks 7–9 complete user-facing integration before Task 10's whole-feature gate.

| Files | Responsibility |
|---|---|
| New `adam-rs/src/template.rs`, `template/{builder,instantiate,error}.rs` | Owned local handles, ordered graph recipes, validation, expansion, binding errors. |
| Existing `adam-rs/src/{lib,sheet,relationship}.rs` | Exports, destination type query, reusable factory integration without cloning live methods. |
| New `adam-rs/tests/templates.rs` | Public native composition contracts and inline equivalence. |
| Existing `adam-lang/src/{ast,ast_parser,token_cursor,trivia,fmt}.rs` | Module/import/signature/use syntax, recovery and comment-preserving formatting. |
| New `cel-parser/src/compile.rs`, `cel-parser/tests/ast_compile.rs` | Compile retained expression ASTs with borrowed operation/type environments. |
| New `adam-lang/src/module.rs`, `module/{resolve,compile}.rs` | Module collection, resolution, typed templates and CEL/native recipe adapters. |
| New `adam-lang/src/source_map.rs`, `composition.rs` | Multi-file provenance and source-free programmatic language-shape adapters. |
| Existing `adam-lang/src/{lib,parser,type_registry,typecheck,cell_value}.rs` | Reuse current frame/value logic; replace direct grammar-to-live-sheet construction with common module lowering. |
| New `adam-lang/tests/{modules,composition,module_formatting}.rs` | Syntax, lookup, type/binding and propagation contracts. |
| Existing `adam-web-ui/src/labels.rs` | Correct cross-file runtime error rendering; no new controls. |
| Existing `adam-lsp/src/{diagnostics,dispatch}.rs`, new `adam-lsp/src/modules.rs` | Module-aware diagnostics and explicit client module source registration. |
| Existing extension `src/extension.ts`, `package.json`, grammar; new `src/moduleSources.ts`, `test/moduleSources.test.ts` | Explicit module mapping, source updates and highlighting. |
| New `adam-lang/examples/{relationships.adm2,composed.adm2,templates.rs}` | Small reusable relationships and native construction. |
| New `cel-euclid/examples/{image_constraints.adm2,resize_straighten.adm2}` | Realistic reusable image components with caller-owned boundary checks. |
| Books, `docs/VISION.md`, milestone plan, dated handoff | Executable guidance and completion record. |

Use the existing crate test runners. File paths in task lists are relative to
the worktree; locations were inspected on the planning snapshot, not inferred
from Scout's index.

## Shared interface decisions

Public names below are the cross-task contract. Internal helper types remain
the implementer's choice; do not introduce a second competing template model.

### Native graph recipes (`adam_rs::template`)

Use owner-checked local `CellSlot` and `RelationshipSlot` handles. A local
index alone is not sufficient: two builders can otherwise accidentally accept
each other's handles. A private shared immutable owner token plus local index
is sufficient without a global counter or synchronization.

```rust
pub struct TemplateBuilder { /* owning ordered graph recipe */ }
pub struct SheetTemplate { /* immutable finalized recipe */ }
pub struct CellSlot { /* owner token, local index */ }
pub struct RelationshipSlot { /* owner token, local index */ }
pub struct TemplateInclusion { /* mapped local slots */ }
pub struct Instance { /* live graph handles and opaque identity */ }
pub struct MethodRecipe { /* local edges and repeatable factory */ }
pub enum TemplateError { /* errors below, with local/actual sites */ }

impl TemplateBuilder {
    pub fn new() -> Self;
    pub fn parameter<T: 'static>(&mut self, name: &str) -> Result<CellSlot, TemplateError>;
    pub fn cell<T, F>(&mut self, name: &str, initial: F) -> Result<CellSlot, TemplateError>
    where T: Any + PartialEq + 'static, F: Fn() -> anyhow::Result<T> + 'static;
    pub fn source<T, F>(&mut self, name: &str, initial: F) -> Result<CellSlot, TemplateError>
    where T: Any + PartialEq + 'static, F: Fn() -> anyhow::Result<T> + 'static;
    pub fn relationship(&mut self, methods: Vec<MethodRecipe>)
        -> Result<RelationshipSlot, TemplateError>;
    pub fn include(&mut self, child: Rc<SheetTemplate>, bindings: &[(&str, CellSlot)])
        -> Result<TemplateInclusion, TemplateError>;
    pub fn finish(self) -> Result<SheetTemplate, TemplateError>;
}
impl MethodRecipe {
    pub fn new<F>(inputs: &[CellSlot], outputs: &[CellSlot], make: F)
        -> Result<Self, TemplateError>
    where F: Fn(&[CellId], &[CellId]) -> anyhow::Result<Method> + 'static;
}
impl SheetTemplate {
    pub fn instantiate(&self, destination: &mut Sheet, bindings: &[(&str, CellId)])
        -> Result<Instance, TemplateError>;
    pub fn is_relationship_only(&self) -> bool;
}
impl TemplateInclusion {
    pub fn relationships(&self) -> &[RelationshipSlot];
}
impl Instance {
    pub fn relationships(&self) -> &[RelationshipId];
    pub fn cells(&self) -> &[CellId];
}
```

Keep local and live handles distinct. The `Instance` owner supplies its identity;
no global counter or caller-visible instance-name syntax is required.

Task 2 extends this builder with private filter/requirement/output and
conditional recipes using the existing runtime construction types, as specified
there. The implementation shares immutable factories with `Rc` as needed, but a
factory creates each instance's fresh executable and initial values.

`TemplateError` distinguishes `UnknownParameter`, `MissingParameter`,
`DuplicateParameter`, `AliasedArgument`, `ForeignHandle`, `InvalidActual`,
`TypeMismatch`, `OutArgument`, `EmptyRelationship`, `ConditionalComponent`,
`InvalidDefinition`, `Capacity`, `Factory`, and wrapped runtime construction/validation
errors. Store implicated formal/local handles and actual IDs, not language spans.

### Module and source layer (`adam_lang`)

```rust
pub struct ModulePath { /* validated identifier segments */ }
pub struct ModuleSet { /* owns parsed and native module definitions */ }
pub struct CompiledModules { /* resolved immutable templates, roots, sources */ }
pub struct TypedTemplate { /* Rc<SheetTemplate>, full formal shapes, metadata */ }
pub struct ModuleCompiler { /* owned TypeRegistry and OpLookup */ }
pub struct SourceMap { /* immutable source owners indexed by SourceId */ }
pub struct SourceLocation { pub source: SourceId, pub span: SourceSpan }
pub struct SourceLabel { pub location: Option<SourceLocation>, pub label: String }
pub struct ModuleDiagnostic { pub message: String, pub labels: Vec<SourceLabel> }

impl AdamAstParser {
    pub fn parse_module_str(&mut self, source: &str) -> Result<ast::Module, ParseError>;
}
impl ModuleSet {
    pub fn new() -> Self;
    pub fn insert_source(&mut self, path: ModulePath, file_name: &str, source: &str)
        -> Result<(), ModuleDiagnostic>;
    pub fn insert_template(&mut self, path: ModulePath, name: &str, template: TypedTemplate)
        -> Result<(), ModuleDiagnostic>;
}
impl ModuleCompiler {
    pub fn new(types: TypeRegistry, lookup: OpLookup) -> Self;
    pub fn compile(self, modules: ModuleSet) -> Result<CompiledModules, Vec<ModuleDiagnostic>>;
}
impl CompiledModules {
    pub fn template(&self, module: &ModulePath, name: &str) -> Option<&TypedTemplate>;
    pub fn instantiate_root(&self, module: &ModulePath, name: &str)
        -> Result<ParsedSheet, ModuleDiagnostic>;
}
impl TypedTemplate {
    pub fn instantiate(&self, destination: &mut ParsedSheet, bindings: &[(&str, CellId)])
        -> Result<Instance, ModuleDiagnostic>;
}
```

Add a language `CompositionBuilder` in `composition.rs` for programmatic
templates with complete `TypeShape`s, including compounds. It owns a native
`TemplateBuilder` and registry, maps its language slots to native slots, and
emits the same `TypedTemplate` as parsed definitions:

```rust
pub fn new(types: TypeRegistry) -> CompositionBuilder;
pub fn parameter(&mut self, name: &str, shape: TypeShape)
    -> Result<CellSlot, ModuleDiagnostic>;
pub fn method<F>(&self, inputs: &[CellSlot], outputs: &[CellSlot], emit: F)
    -> Result<MethodRecipe, ModuleDiagnostic>
where F: Fn() -> anyhow::Result<DynSegment> + 'static;
pub fn finish(self) -> Result<TypedTemplate, ModuleDiagnostic>;
```

Here `CellSlot`/`MethodRecipe` are language adapters, not aliases that discard
shapes. Complete the other builder operations by delegation to the native
builder rather than implementing a second expansion algorithm. The methods
above belong to `impl CompositionBuilder`, not free functions.

`ParsedSheet` retains the source map and instance origins. Its root-cell lookup
maps exclude private names. Change `locate_error` to return `Vec<SourceLabel>`;
update direct consumers rather than add ambiguous source-free fallback APIs.
Legacy `AdamParser::parse_str(&mut self, &str)` remains the single-root convenience
entry point, using the same lowering, not a second composition implementation.

---

### Task 1: Owning native template recipes and basic instantiation

**Files:** create `adam-rs/src/template.rs`, `template/{builder,instantiate,error}.rs`,
`adam-rs/tests/templates.rs`; modify `adam-rs/src/{lib,sheet,relationship}.rs`.

**Interfaces:** produce the native interfaces above. Add
`Sheet::cell_type_id(&self, id: CellId) -> Option<TypeId>` for checked binding.
No new dependency.

- [ ] **Step 1:** Add public tests `named_bindings_preserve_caller_cells`,
  `private_cells_are_disjoint`, `foreign_slots_are_rejected`, and
  `native_use_requires_no_parser`. Build an `i32` identity component using a
  `Method::from_fn_1_1` factory, bind arguments in reversed name order, propagate,
  assert output `7`, retained caller source `7`, and independent instance values.
  Assert each missing/extra/repeated formal, repeated actual, bad ID and wrong
  concrete type produces the corresponding `TemplateError`.

  The native smoke contract starts with:

  ```rust
  let mut builder = TemplateBuilder::new();
  let input = builder.parameter::<i32>("input").unwrap();
  let output = builder.parameter::<i32>("output").unwrap();
  let method = MethodRecipe::new(&[input], &[output], |ins, outs| {
      Ok(Method::from_fn_1_1(ins[0], outs[0], |x: &i32| Ok(*x)))
  }).unwrap();
  builder.relationship(vec![method]).unwrap();
  let template = builder.finish().unwrap();
  let mut sheet = Sheet::new();
  let input = sheet.add_source(7i32);
  let output = sheet.add_cell(0i32);
  template.instantiate(&mut sheet, &[("output", output), ("input", input)]).unwrap();
  sheet.propagate().unwrap();
  assert_eq!(*sheet.read::<i32>(output).unwrap(), 7);
  assert_eq!(*sheet.source::<i32>(input).unwrap(), 7);
  ```
- [ ] **Step 2:** Run `cargo test -p adam-rs --test templates`; expect compile
  failures for absent template APIs, not a failing unrelated runtime test.
- [ ] **Step 3:** Implement ordered recipe construction, owner validation,
  inclusion remapping and instantiation through existing `Sheet` mutators.
  Factory positional edges must match their declared recipe inputs/outputs and
  types; report a factory/definition error on mismatch. Use a binding map, not
  cloned live methods or separate sheets. Parameters allocate/write nothing.
- [ ] **Step 4:** Run `cargo test -p adam-rs --test templates` and
  `cargo clippy -p adam-rs --all-targets -- -D warnings`; expect success.
- [ ] **Step 5:** Format and commit only Task 1 files, including contracts and
  examples. Include the repository-required Copilot coauthor trailer.

### Task 2: Specialization, complete private bodies and branch inclusion

**Files:** extend `adam-rs/src/template/{builder,instantiate,error}.rs` and tests.

**Interfaces:** consume Task 1. Add:

```rust
pub fn out(&mut self, writer: MethodRecipe) -> Result<RelationshipSlot, TemplateError>;
pub fn filter<F>(&mut self, cell: &CellSlot, inputs: &[CellSlot], make: F)
    -> Result<(), TemplateError>
where F: Fn(CellId, &[CellId]) -> anyhow::Result<Filter> + 'static;
pub fn require<F>(&mut self, cell: &CellSlot, inputs: &[CellSlot], make: F)
    -> Result<(), TemplateError>
where F: Fn(&[CellId]) -> anyhow::Result<Requirement> + 'static;
pub fn conditional<T, F>(&mut self, inputs: &[CellSlot], make: F,
    branches: Vec<(Vec<T>, Vec<RelationshipSlot>)>, default: Vec<RelationshipSlot>)
    -> Result<(), TemplateError>
where T: Clone + Any + PartialEq + 'static,
      F: Fn(&[CellId]) -> anyhow::Result<MatchExpr> + 'static;
```

`out` promotes a previously declared private ordinary slot through its sole
writer, matching `Sheet::add_out`; it cannot target a formal slot. `filter` and
`require` likewise target only private slots. Initial source slots cannot be
written by surviving methods. Finalization checks private ownership and every
authored method's structure before destination specialization.

- [ ] **Step 1:** Add `source_binding_removes_whole_multi_output_method`: use
  three equality methods with outputs `(a,b)`, `(a,c)`, `(b,c)`; bind `b` as
  source `10`, initialize `a=4,c=4`, and assert both become `10`.
  Add `all_directions_removed_is_error`, `out_argument_is_rejected_even_read_only`,
  `private_outputs_filters_requirements_work`, and
  `conditional_wrapper_rejects_transitive_state`.
  Compare native composed and manually inlined sheets after identical edits,
  including `source()` values and public source/direction status.
  The multi-output test's postconditions are:

  ```rust
  assert_eq!(*sheet.read::<i32>(a).unwrap(), 10);
  assert_eq!(*sheet.read::<i32>(b).unwrap(), 10);
  assert_eq!(*sheet.read::<i32>(c).unwrap(), 10);
  assert_eq!(*sheet.source::<i32>(b).unwrap(), 10);
  ```
- [ ] **Step 2:** Run `cargo test -p adam-rs --test templates`; confirm failures
  isolate the new specialization/private-body contracts.
- [ ] **Step 3:** Preflight all bindings and transitive structural eligibility.
  Traverse included recipe edges and registration steps in written order,
  retaining the definition DAG rather than eagerly duplicating descendants.
  Remove whole source-writing methods and reject emptied relationships before
  registration. Preserve each constituent relationship. Eligible branch
  inclusion returns its relationship slots for the existing conditional API;
  include no independent activation layer.
- [ ] **Step 4:** Run `cargo test -p adam-rs` and the Task 1 clippy command.
  Verify branch switching for nested relationship-only uses, no-state and
  zero-parameter templates, construction errors, and template-owner drop.
  Add `expansion_size_overflow_is_diagnostic` using a compact repeated-child
  DAG whose checked expanded-item count exceeds `usize`; preflight must reject
  it without allocating or traversing its full expansion. Use fallible
  reservation for the live expansion and report allocation failures.
- [ ] **Step 5:** Format, review the native public contract, and commit Task 2.

### Task 3: Module/signature/use syntax and formatter

**Files:** modify `adam-lang/src/{ast,ast_parser,token_cursor,trivia,fmt,lib}.rs`;
create `adam-lang/tests/{modules,module_formatting}.rs`.

**Interfaces:** add `ast::Module` with ordered `ModuleItem::{Use,Sheet}` and
recoverable errors; `ast::Sheet.parameters: Option<Vec<ParameterDecl>>`;
`SheetItem::RelationshipUse` and the corresponding conditional-branch variant.
A `ParameterDecl` owns `name`, name span and required `TypeExpr`; a
`RelationshipUse` owns path segments, named cell arguments and their spans.
Produce `parse_module_str`, `attach_module_trivia`, and `format_module`.

- [ ] **Step 1:** Parse and assert AST shapes for a file with an aliased import,
  a forward-used typed definition, a zero-parameter component and two roots.
  Assert `None` versus `Some(vec![])` distinguishes roots from `sheet helper()`.
  Add `module_comments_round_trip` asserting `format_source(formatted) ==
  formatted` and each leading, signature, argument, inter-definition and
  trailing comment appears exactly once. Malformed imports/parameters/uses
  must recover without losing the following definition.

  ```rust
  let source = "use constraints::ordered as ord;\n\
      sheet helper() {}\nsheet first {}\nsheet second {}";
  let module = AdamAstParser::new().parse_module_str(source).unwrap();
  assert_eq!(module.items.len(), 4);
  let formatted = format_source(source).unwrap();
  assert_eq!(format_source(&formatted).unwrap(), formatted);
  ```
- [ ] **Step 2:** Run
  `cargo test -p adam-lang --test modules --test module_formatting`; expect the
  missing AST/parser interfaces to fail compilation.
- [ ] **Step 3:** Implement the explicit grammar: file-level `use path [as name];`,
  `sheet name [ "(" typed_parameters ")" ] body`, and
  `relationship path "(" name ":" cell { "," name ":" cell } [ "," ] ")" ";"`.
  Accept trailing commas in signatures and named argument lists. Do not add
  positional arguments, generic syntax, glob imports or parameter clauses.
  Keep the existing ordinary relationship grammar unchanged.
- [ ] **Step 4:** Run `cargo test -p adam-lang` and
  `cargo clippy -p adam-lang --all-targets -- -D warnings`. Existing `parse_str`
  AST convenience accepts exactly one root and no ambiguous multi-definition
  selection; `format_source` uses full-module parsing and refuses recovered
  syntax errors as before.
- [ ] **Step 5:** Format and commit syntax/formatter changes with contracts.

### Task 4: Source-independent retained CEL expression compilation

**Files:** create `cel-parser/src/compile.rs`, `cel-parser/tests/ast_compile.rs`;
modify `cel-parser/src/{lib,parser_context,op_table}.rs` only as needed for shared
emission helpers.

**Interfaces:** produce:

```rust
pub fn compile_expr<F>(expr: &Expr, lookup: &OpLookup,
    types: &dyn TypeResolver, resolve: &mut F) -> Result<DynSegment>
where F: FnMut(&str, &mut DynSegment, SourceSpan) -> anyhow::Result<bool>;
```

`resolve` handles external cell references, returning false for other names.
Closure formals use their own environment; enclosing external cell resolution
is disabled inside noncapturing bodies. Compile through existing contexts and
operation dispatch; do not serialize AST back to text or clone `OpLookup`.

- [ ] **Step 1:** Add table-driven contracts comparing direct CEL parsing and
  retained-AST compilation: arithmetic/casts, tuples and indices, typed arrays,
  registered records/members, callable locals, higher-order calls, logical
  short-circuiting, conditionals, and ranges. Assert exact results and matching
  rejection for incompatible operands. Include `false && (1i32 / 0i32 == 0i32)`
  returning false and `true || ...` returning true. Compile the same AST twice,
  drop its owner, and execute independent returned segments.

  ```rust
  let expr = Parser::<AstContext>::new(OpLookup::new())
      .parse_str_ast("false && (1i32 / 0i32 == 0i32)").unwrap();
  let types = builtin_type_resolver();
  let mut resolve = |_: &str, _: &mut DynSegment, _: SourceSpan| Ok(false);
  let mut segment = compile_expr(&expr, &OpLookup::new(), types.as_ref(), &mut resolve).unwrap();
  assert!(!segment.call0::<bool>().unwrap());
  ```
- [ ] **Step 2:** Run `cargo test -p cel-parser --test ast_compile`; expect the
  absent compiler API to fail compilation.
- [ ] **Step 3:** Implement an exhaustive `Expr` visitor in evaluation order.
  Reuse `DynSegmentContext` and exact lookup/type/closure helpers. Preserve
  source spans, independent closure scopes and branch laziness. Factor shared
  emission helpers rather than duplicate checked operator implementations.
- [ ] **Step 4:** Run `cargo test -p cel-parser -p cel-runtime` and
  `cargo clippy -p cel-parser -p cel-runtime --all-targets -- -D warnings`.
  Every expression variant must have a contract test, including rejected free
  capture and partial failure cleanup. No new runtime cloning abstraction.
- [ ] **Step 5:** Format and commit the AST compiler and focused refactors.

### Task 5: Resolve modules and build typed reusable definitions

Execute this task through three serial implementation/review gates. Initial
inspection identified native occurrence metadata and registry adapters as
prerequisites; no Task 5 implementation was produced during that inspection.
These gates preserve the approved feature scope and public interfaces:

1. **5A: Sources and native provenance.** Add owning, collision-free source
   identities and diagnostics, then capture parent-linked inclusion occurrences,
   local-to-live mappings and authored-to-surviving method indices during native
   expansion. Preserve checked expansion accounting and iterative owner teardown.
2. **5B: Source-free complete-shape composition.** Add registry-driven native
   builder adapters, language slots/recipes, `CompositionBuilder`, checked
   `TypedTemplate::instantiate`, and retained `ParsedSheet` sources/origins.
   Cover registered host leaves and compound shapes without parsing source text.
3. **5C: Module resolution and retained-AST lowering.** Add module collection,
   imports, definition-use cycle checks and nonexecuting body compilation through
   5B's adapters. Keep root instantiation with Task 6.

Within 5C, review collection/name resolution, borrowed signature/body analysis,
and owned factory materialization as separate prerequisites. Resolution alone
does not complete 5C. The borrowed analysis returns a checked-body plan that
both the nonexecuting checker and owning compiler consume.

Each gate supplies focused contract tests, exact validation evidence, an
implementation commit and an independent review before the next gate begins.
Source-free diagnostics carry absent locations rather than invented spans.

**Files:** create `adam-lang/src/module.rs`, `module/{resolve,compile}.rs`,
`composition.rs`, `source_map.rs`; modify `type_registry.rs`, `lib.rs`;
extend `adam-lang/tests/modules.rs`.

**Interfaces:** produce `ModuleSet`, `ModulePath`, `ModuleCompiler`,
`CompiledModules`, `TypedTemplate`, source ownership types and
`CompositionBuilder` from the shared interface block, including checked
`TypedTemplate::instantiate`. Root instantiation belongs to Task 6. Retain one immutable
`Rc` compilation environment with owned registry/lookup; body factories borrow
that environment to invoke Task 4 and prepare a fresh `Frame`.

- [ ] **Step 1:** Add `imports_bind_in_definition_environment`: two modules
  export same-named helpers with different results; an imported wrapper must
  use its defining module's helper and produce `7`, not caller helper `9`.
  Obtain the wrapper through `CompiledModules::template` and instantiate it
  into a simple existing `ParsedSheet`; this test does not require Task 6's
  module-root compilation.
  Test aliases, forward references, missing modules/symbols, duplicate names,
  use cycles, zero-parameter components, multiple roots and explicit selection.
  Add programmatic native and compound-shape templates to the same module
  collection; bind `[i32]` where `[bool]` is expected and assert rejection even
  though both use the same erased language carrier.

  ```rust
  let mut builder = CompositionBuilder::new(TypeRegistry::new());
  builder.parameter("xs", TypeShape::Array(Box::new(
      TypeShape::Named(TypeId::of::<i32>())
  ))).unwrap();
  let template = builder.finish().unwrap();
  let mut parsed = AdamParser::new(TypeRegistry::new(), OpLookup::new())
      .parse_str("sheet s { cell xs = [true]; }").unwrap();
  let xs = parsed.cell_names["xs"].0;
  assert!(template.instantiate(&mut parsed, &[("xs", xs)]).is_err());
  ```
- [ ] **Step 2:** Run `cargo test -p adam-lang --test modules`; confirm failures
  are for semantic/module construction, not already implemented syntax.
- [ ] **Step 3:** Resolve all module symbols before resolving definition uses.
  Use standard hash lookup plus tri-color DFS over definition-use edges;
  report cycles with their use-site chain. A cycle of module imports without
  a component-use cycle is not inherently an error. Lower bodies to Task 1–2
  recipes; use complete `TypeShape`s at every language boundary. Native
  `CompositionBuilder::method` accepts repeatable segment emitters, not source.
  Validate all directions even when a particular invocation will prune some.
- [ ] **Step 4:** Run `cargo test -p adam-lang --test modules` and affected
  clippy. Assert modules/templates can be dropped after instantiation without
  invalidating bodies or diagnostic sources. Assert instance factories do not
  share mutable frames and retained ASTs need not remain externally owned.
- [ ] **Step 5:** Format and commit resolver, adapters and module ownership.

### Task 6: Common root lowering, native/language equivalence and conditionals

**Files:** modify `adam-lang/src/{parser,module/compile,composition,cell_value,typecheck}.rs`;
create `adam-lang/tests/composition.rs`; extend existing container/registry tests.

**Interfaces:** implement `CompiledModules::instantiate_root`, consuming Task 5's
checked `TypedTemplate::instantiate` for root uses.
Keep `AdamParser::parse_str` as a one-root convenience by using the common
lowering; retire its separate grammar implementation once equivalence passes.
Store the compiler environment under shared immutable ownership in
`AdamParser`; the convenience path borrows that environment through the same
internal compilation entry point. Remove `AdamParser::op_lookup_mut` and
install libraries before `AdamParser::new`, updating its current test caller.
Do not clone an owning `OpLookup` or add shared mutable registry state to keep
the old mutation API.
Use `check_module(&ModulePath, &ModuleSet, &TypeRegistry, &OpLookup)
-> Vec<ModuleDiagnostic>` for nonexecuting tooling checks.
Gate 5C's borrowed analysis implements this entry point; Task 6 integrates it.
The explicit path selects the AST and source owner already registered in the
module collection, rather than guessing which file an unassociated AST replaces.

- [ ] **Step 1:** Add `composed_matches_manual_inline`: typed `ordered`, two
  independently used multiply components, a wrapper with a private helper, and
  mixed source/ordinary bindings. After each identical edit, assert composed
  and inline values, source aspirations and diagnostics agree.
  Add `conditional_uses_only_relationships`, accepting a nested multiply
  wrapper and rejecting a transitive private cell, private output, or internal
  conditional even in an inactive branch. Assert unknown, missing, duplicate,
  aliased and out actuals produce located diagnostics before live construction.
  Pin every edit in the inline-equivalence test with the same public queries:

  ```rust
  assert_eq!(composed.read::<f64>(composed_value).unwrap(),
             manual.read::<f64>(manual_value).unwrap());
  assert_eq!(composed.source::<f64>(composed_value).unwrap(),
             manual.source::<f64>(manual_value).unwrap());
  assert_eq!(composed.is_source(composed_value), manual.is_source(manual_value));
  ```
- [ ] **Step 2:** Run `cargo test -p adam-lang --test composition`; confirm new
  semantics fail before implementing root expansion.
- [ ] **Step 3:** Lower roots in declaration order and carry formal-to-actual
  scope maps through each use. Keep root name maps separate from private
  instance metadata; preserve caller filters/requirements without attaching
  parameter-level declarations. Gather eligible expanded relationships into
  existing branch groups. Validate the final combined sheet, not isolated
  template solvability. Reuse existing initializer/frame/default/filter code,
  moving helpers into focused compiler modules rather than duplicating them.
  Update programmatic `ast::Sheet` constructors in `ez-adam/src/codegen` for
  the new `parameters: None` field; retain ordinary generated-sheet behavior.
- [ ] **Step 4:** Run `cargo test -p adam-lang -p adam-rs -p cel-euclid --features
  cel-euclid/adam`; all existing single-sheet/container/crop contracts must
  retain their behavior. The nonexecuting checker must not run user methods,
  initializers or requirements to decide whether code is well typed.
- [ ] **Step 5:** Format and commit the common compiler and composition contracts.

### Task 7: Source-aware construction and runtime diagnostics

**Files:** extend `adam-lang/src/{source_map,module/compile,parser,error_labels}.rs`;
modify `adam-web-ui/src/labels.rs`; extend `adam-lang/tests/composition.rs` and
the existing label tests.

**Interfaces:** store `Origin { definition: Option<SourceLocation>,
invocations: Vec<SourceLocation> }` for instantiated cells, relationships and
methods. `ParsedSheet::locate_error(&adam_rs::Error) -> Vec<SourceLabel>` returns
definition labels with use-site context. `SourceMap` resolves each `SourceId`
to owned file name/text. Update all affected direct callers/field accesses.

- [ ] **Step 1:** Construct an imported component whose division by a caller
  cell fails at zero. Assert diagnostics name the definition file and exact
  division span plus the root use site. Invoke it twice and assert the
  failing instance's path is identified. Repeat without an inner CEL
  `SpanContext` using a native method error. Verify root-only errors retain
  current filenames, labels and locations.
  Assert labels resolve to the right source owners, not just matching numbers:

  ```rust
  let labels = parsed.locate_error(&error);
  assert!(labels.iter().any(|label| label.location.as_ref()
      .is_some_and(|location| location.source == definition_source)));
  assert!(labels.iter().any(|label| label.location.as_ref()
      .is_some_and(|location| location.source == invocation_source)));
  ```
- [ ] **Step 2:** Run the composition diagnostic selector and
  `cargo test -p adam-web-ui --lib labels`; expect failures demonstrating the
  current single-source renderer is insufficient.
- [ ] **Step 3:** Attach definition provenance to CEL errors at the method
  boundary without replacing the underlying error. Resolve inner spans only
  against their definition source; render file groups with invocation context.
  Do not reuse the caller's text merely because its span coordinates fit.
  Programmatic origins retain labels without manufactured spans.
- [ ] **Step 4:** Run `cargo test -p adam-lang -p adam-web-ui` and affected clippy.
  Add pure renderer unit tests; do not change UI controls or presentation.
  If implementation changes rendered UI behavior, use `verifying-begin-ui`
  rather than claiming unit tests alone cover the change.
- [ ] **Step 5:** Format and commit source-aware diagnostic integration.

### Task 8: Module-aware LSP and editor support

**Files:** modify `adam-lsp/src/{lib,diagnostics,dispatch}.rs`, create `modules.rs`;
modify extension `src/extension.ts`, `package.json`, grammar and create
`src/moduleSources.ts`, `test/moduleSources.test.ts`.

**Interfaces:** add
`diagnostics_for_module(module: &ModulePath, modules: &ModuleSet)
-> Vec<ModuleDiagnostic>` with the same configured libraries as current
`diagnostics_for_source`. Preserve the source-only convenience for existing
hosts. Define explicit initialization option `moduleSources` as records of
`{ path: string, uri: string, text: string }`; document this wire contract in
both caller and receiver.

- [ ] **Step 1:** Add LSP tests for named binding type mismatch, relationship-only
  branch rejection, unresolved imports, aliased imports and definition/use-site
  related locations. Add a transport test that changes a registered definition
  and republishes affected dependent diagnostics. Add extension tests that
  explicitly configured module paths map to owned source records or a visible
  configuration/read error, never an empty successful source.
  The pure extension source-loader test compares the exact wire records:

  ```typescript
  assert.deepEqual(records, [{
      path: "constraints",
      uri: definitionUri,
      text: "sheet ordered(a: f64, b: f64) {}",
  }]);
  ```
- [ ] **Step 2:** Run `cargo test -p adam-lsp` and, from the extension directory,
  `npm test`; confirm each new contract fails for the intended missing feature.
- [ ] **Step 3:** Register client-supplied module sources and overlay open-document
  edits by URI. Recheck dependents when a definition changes; include related
  LSP locations across files. The extension's explicit `adam-lang.modules`
  mapping supplies module path to workspace-relative file path; loading is
  tooling policy, not core-library IO. Do not add implicit package discovery,
  glob imports or automatic version migration. Highlight `use`, `as`, parameter
  types and named cell bindings; keep module formatting safe on recovered errors.
- [ ] **Step 4:** Run `cargo build -p adam-lsp`, `cargo test -p adam-lsp`, then
  extension `npm run compile` and `npm test`. Do not install dependencies unless
  the manifest changed or a command reports missing dependencies/tools.
- [ ] **Step 5:** Format and commit the transport/editor integration.

### Task 9: Executable examples and library guidance

**Files:** create examples from the file map and a contract test
`cel-euclid/tests/composition.rs`; modify
`adam-lang-book/book-src/{relationships,reference}.md`, `docs/VISION.md`,
crate tutorial docs and example registration only where required.

**Interfaces:** consume the completed public native/module APIs. No new runtime
fitting operations or out-argument exceptions.

- [ ] **Step 1:** Add example-backed contracts for ordered bounds,
  two-way multiplication with a source-bound factor, and three-way floating
  multiplication. Bind the same multiplication component at least twice with
  distinct named actuals. For the image example combine width/height/resolution
  relationships with crop/straighten through caller-owned ordinary boundary
  cells and source inputs. Assert a resize, a rotation, and a reversal preserve
  caller aspiration and satisfy documented coverage/precision requirements.
  Keep the image composition's public contract visible in assertions:

  ```rust
  assert!(parsed.cell_requirements_valid(crop));
  assert!(*parsed.read::<bool>(coverage).unwrap());
  assert_eq!(*parsed.source::<Rect>(crop).unwrap(), requested_crop);
  ```
- [ ] **Step 2:** Run `cargo test -p cel-euclid --features adam --test composition`;
  expect the absent fixture/API integration to fail before completing examples.
- [ ] **Step 3:** Extract authored components from existing example operations,
  retaining current numerical contracts and checked errors. Results that callers
  bind are ordinary cells produced by explicit relationships, not private
  outputs illicitly passed as actuals. Keep diagnostics-only outputs internal.
  Teach native construction, imported/inline definitions, parameter preconditions,
  restrictions and module/root selection with executable snippets.
- [ ] **Step 4:** Run the new tests, `cargo run -p adam-lang --example templates`,
  `cargo test --doc -p adam-lang -p adam-rs -p cel-parser`, and
  `RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace`.
  Run existing book executable-example tests covering changed chapters.
- [ ] **Step 5:** Format and commit examples and guidance; do not rewrite unrelated
  image policies or generated Spectrum assets.

### Task 10: Whole-feature review, gate and handoff

**Files:** milestone plan, approved spec status, a dated implementation handoff,
and only fixes identified in the completed feature.

**Interfaces:** consume all prior tasks; no new feature surface.

- [ ] **Step 1:** Compare every approved spec requirement with implemented
  public tests. Perform a whole-branch review; resolve in-scope findings now
  and open issues for genuine out-of-scope work. Record exact coverage and
  limitations, not just a reviewer verdict.
- [ ] **Step 2:** Run the complete repository gate before a PR:
  `cargo fmt --all`; `cargo build --workspace`; `cargo test --workspace`;
  `cargo test --doc --workspace`; all five clippy invocations from `CLAUDE.md`;
  `RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace`.
  Build/test must have zero project warnings. Also rerun extension compilation/
  tests and `cargo test -p cel-euclid --features adam`.

  ```sh
  cargo clippy --workspace --exclude begin --all-targets -- -D warnings
  cargo clippy -p begin --no-default-features --all-targets -- -D warnings
  cargo clippy -p begin --all-targets -- -D warnings
  cargo clippy -p ez-adam --no-default-features --all-targets -- -D warnings
  cargo clippy -p ez-adam --all-targets -- -D warnings
  ```
- [ ] **Step 3:** Write the dated handoff with implemented contracts, actual
  commands/results, deliberate deferred issues and any partial work. Update
  #29 and the milestone plan only to the status the evidence supports.
- [ ] **Step 4:** Commit the completion record with the required trailer after
  formatting and whitespace checks. Use `pr-open` only when opening the PR is
  authorized; use `verification-before-completion` before success claims.

## Algorithm Research Note

### Covered by stdlib / OSS

- Name/argument uniqueness and lookup: `HashMap`/`HashSet`; declaration order:
  existing `IndexMap` and `Vec`. No additional dependency or license review.
- Method specialization: iterator filtering over declared output edges; reject
  empty retained lists. Never mutate the method's output tuple.
- Scope mapping: indexed slot tables under an owning template; borrow immutable
  recipes rather than copy live graphs.

### Covered by known algorithm (implement directly)

- Definition-use recursion and transitive eligibility: tri-color DFS with memoized
  completed definitions, O(V+E); use an explicit stack for untrusted deep graphs.
- Expansion: ordered depth-first traversal, O(N+M) for expanded declarations and
  referenced edges, excluding CEL compilation and existing runtime validation.

### Custom logic

- Cell-kind pruning, exact language-shape validation, and source/use provenance
  are domain rules over those ordinary traversals, not new solver algorithms.

### Complexity flags

- Finite acyclic definitions can still expand exponentially. Use checked size
  accounting and fallible allocation with a diagnostic; do not eagerly
  materialize the full expansion during eligibility checks. Avoid recursive host
  stacks for definition traversal/expansion.
- Re-emitting retained ASTs costs construction time; propagation must never do it.
  No new performance guarantee is needed beyond documented actual complexity.

### Recommendation

Reuse the existing planner and CEL emission helpers. Do not add a nested solver,
symbolic algebra dependency, or global mutable template registry.

## Plan self-review

The tasks cover every spec section: native ownership/specialization (1–2),
syntax/formatting (3), repeatable expression frames (4), module/type adapters (5),
inlining and conditional behavior (6), provenance (7), tooling (8), demonstrators
and docs (9), verification and persistent handoff (10). The five Review Focus
cases have named owning test steps. All deferred features remain explicitly
excluded. No product implementation is authorized merely by saving this plan.
