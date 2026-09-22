# Typed Array Annotations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add unambiguous array type ascriptions such as `[0, 1]: [i32]` and `[]: [i32]`, including nested arrays and reusable tuple type-expression syntax, with registry-backed runtime construction.

**Architecture:** Keep `DynamicArray` as the runtime value and add typed-empty construction from an `ArrayElementType`. Introduce a generic CEL-facing type resolver/descriptor interface and recursive parser `TypeExpr`; `cel-parser` uses built-in registrations by default, while `adam-lang::TypeRegistry` adapts its generic type metadata without moving sheet-specific constructors into CEL. Array parsing stores annotations in the AST and validates them in direct execution or deferred static checking.

**Tech Stack:** Rust 2024 workspace, `cel-parser` recursive-descent parser, `cel-runtime::DynamicArray`, `proc_macro2` spans, `adam-lang::TypeRegistry`, Cargo unit/integration/doc tests.

**Spec:** `docs/superpowers/specs/2026-09-16-typed-array-suffixes-design.md`

## Global Constraints

- Preserve unsuffixed non-empty array inference and existing nested-array behavior.
- Empty arrays require a complete array type annotation.
- The annotation names the complete array type: `[0, 1]: [i32]`, not `[0, 1]: i32`.
- Scalar leaves resolve through a configured registry; array and tuple type expressions compose recursively. Tuple-valued array elements remain rejected by the existing runtime limitation tracked in issue #213.
- Do not perform implicit numeric conversion; annotated element types must match exactly.
- Keep `adam-lang` sheet-specific constructors out of the generic CEL type layer.
- Preserve zero-copy `Vec<T>` conversion, zero-sized element handling, alignment, and drop invariants.
- Use contract-style documentation for every new public function/type and derive tests from public behavior.
- Run `cargo fmt --all` before commits and keep build/test warnings at zero.

---

### Task 1: Add typed-empty runtime array construction

**Files:**
- Modify: `cel-runtime/src/dynamic_array.rs`
- Modify: `cel-runtime/src/dyn_segment.rs`
- Modify: `cel-runtime/src/lib.rs` only if new public types need re-export changes
- Test: existing `#[cfg(test)]` modules in `cel-runtime/src/dynamic_array.rs` and `cel-runtime/src/dyn_segment.rs`

**Interfaces:**
- Consumes: existing `ArrayElementType`, `DynamicArray::try_from_vec_with_element_type`, and `DynSegment::make_array`.
- Produces: a public or crate-visible constructor that creates an empty `DynamicArray` from an `ArrayElementType`, plus a `DynSegment` operation that collects zero elements with an explicit descriptor.

- [ ] **Step 1: Write failing runtime tests**

Add tests that require:

```rust
let element = ArrayElementType::leaf::<i32>().unwrap();
let array = DynamicArray::empty_with_element_type(element.clone());
assert!(array.is_empty());
assert_eq!(array.element_type(), &element);

let mut segment = DynSegment::new::<()>();
segment.make_typed_array(0, segment.current_stack_offset(), element)?;
let array: DynamicArray = segment.call0()?;
assert!(array.try_into_vec::<i32>()?.is_empty());
```

Also cover an empty nested descriptor created with
`ArrayElementType::array_of(ArrayElementType::leaf::<i32>().unwrap())`, zero-sized leaves, and
rejection of a typed descriptor whose marker/layout does not match the collected values.

- [ ] **Step 2: Run the focused tests and verify failure**

Run:

```text
cargo test -p cel-runtime empty_with_element_type
cargo test -p cel-runtime make_typed_array
```

Expected: compilation failures naming the missing constructor/operation.

- [ ] **Step 3: Implement the minimal typed-empty path**

Reuse the existing allocation/layout helpers and descriptor ownership rules. Add a constructor that
creates a zero-length array with the supplied descriptor and no element allocation. Add a
`DynSegment` method whose zero-count path emits that array while preserving the current stack
offset/precondition checks. Keep the existing inferred `make_array` path unchanged.

- [ ] **Step 4: Run runtime tests**

Run:

```text
cargo test -p cel-runtime
```

Expected: PASS with no warnings.

- [ ] **Step 5: Commit**

```text
git add cel-runtime/src/dynamic_array.rs cel-runtime/src/dyn_segment.rs cel-runtime/src/lib.rs
git commit -m "feat(cel-runtime): construct typed empty arrays"
```

### Task 2: Define recursive type expressions and generic CEL resolution

**Files:**
- Create: `cel-parser/src/type_expr.rs`
- Modify: `cel-parser/src/lib.rs`
- Modify: `cel-parser/src/parser_context.rs`
- Modify: `cel-parser/src/op_table.rs` if built-in scalar metadata is centralized there
- Test: `cel-parser/src/type_expr.rs`, `cel-parser/src/lib.rs`, and `cel-parser/src/parser_context.rs`

**Interfaces:**
- Consumes: runtime `ArrayElementType`, `TypeId`, existing built-in scalar signatures, and
  `ParserContext`.
- Produces:
  - `pub enum TypeExpr { Named { name: String, span: ExprSpan }, Array { element: Box<TypeExpr>, span: ExprSpan }, Tuple { elements: Vec<TypeExpr>, span: ExprSpan } }`;
  - a parser-facing `TypeResolver` trait or equivalent callback returning a resolved descriptor;
  - recursive type-expression parsing for scalar names, `[TypeExpr]`, and `(TypeExpr, ...)`;
  - a `ParserContext` operation accepting an optional resolved array descriptor.

- [ ] **Step 1: Write failing type-expression tests**

Test the public shape and errors for:

```text
i32
[i32]
[[i32]]
(i32, f64)
[(i32, f64)]
```

Cover missing closing delimiters, empty/invalid tuple forms according to the existing tuple type
grammar, and unknown scalar names through the resolver. Assert spans cover the complete type
expression.

- [ ] **Step 2: Run focused parser tests and verify failure**

Run:

```text
cargo test -p cel-parser type_expr
```

Expected: failure because the recursive type-expression model and parser entry point do not exist.

- [ ] **Step 3: Implement the generic resolver boundary**

Define the smallest resolver contract needed by both direct execution and AST checking. Its
resolved result must expose the leaf `TypeId`/runtime descriptor and permit recursive array
composition. Keep built-in scalar resolution in `cel-parser`; do not import `adam-lang`.

Extend `ParserContext` with a method equivalent to:

```rust
fn make_annotated_array(
    &mut self,
    n: usize,
    ambient_start: usize,
    element_type: Option<ResolvedArrayType>,
    start: Span,
    end: Span,
) -> crate::Result<()>;
```

Preserve `make_array` for unannotated inference or route it through the new operation with
`None`. `DynSegmentContext` validates the complete descriptor; `AstContext` records unresolved
type syntax and does not resolve it during parsing.

- [ ] **Step 4: Run focused tests**

Run:

```text
cargo test -p cel-parser type_expr
cargo test -p cel-parser parser_context
```

Expected: PASS with no warnings.

- [ ] **Step 5: Commit**

```text
git add cel-parser/src/type_expr.rs cel-parser/src/lib.rs cel-parser/src/parser_context.rs cel-parser/src/op_table.rs
git commit -m "feat(cel-parser): add recursive type expressions"
```

### Task 3: Parse and preserve array type ascriptions

**Files:**
- Modify: `cel-parser/src/lib.rs`
- Modify: `cel-parser/src/ast.rs`
- Modify: `cel-parser/src/fmt.rs`
- Modify: `cel-parser/src/ty.rs`
- Test: parser, AST, formatter, and type-checking test modules in those files

**Interfaces:**
- Consumes: Task 2's `TypeExpr`, resolver, and annotated-array context operation.
- Produces: array grammar of the form `array_expression = "[" [ expression { "," expression } ] "]" [ ":" type_expression ]`; `Expr::Array` retains `Option<TypeExpr>` and its annotation span; `check_expr` validates the complete annotated array type.

- [ ] **Step 1: Write failing end-to-end parser tests**

Add direct execution tests for:

```text
[0, 1]: [i32]
[1.0, 42.5]: [f64]
[]: [i32]
[]: [[i32]]
```

Add rejection tests for `[]`, `[0, 1]: [f64]`, unknown type names, malformed annotations, and
trailing tokens. Preserve existing tests proving `[0, 1]` still infers `i32`.

- [ ] **Step 2: Run focused tests and verify failure**

Run:

```text
cargo test -p cel-parser array
cargo test -p cel-parser --lib ty
```

Expected: the new forms fail to parse or type-check, while existing unannotated tests remain
passing.

- [ ] **Step 3: Implement annotation parsing**

After consuming the closing `]`, consume `:` only when present, parse one complete `TypeExpr`, and
pass the resolved complete array descriptor to the context. Reject an empty array without an
annotation before attempting runtime construction. Ensure `:` is not consumed inside tuple or
array type-expression delimiters.

- [ ] **Step 4: Extend AST and formatting**

Store the unresolved type expression and annotation span in `Expr::Array`. Update every AST
pattern match, span calculation, formatter branch, and test helper. Format annotations as
`: Type`, including nested arrays and tuples, while preserving comments and normalized commas.

- [ ] **Step 5: Extend static checking**

Resolve `TypeExpr` recursively into `Ty`, including nested `Ty::Array`; preserve tuple type
expressions for reusable type syntax, but report the existing unsupported tuple-element diagnostic
when a tuple type is used as an array element. Report unknown leaves and exact element mismatches.
Return the annotated array type on success; keep unannotated empty arrays diagnostic.

- [ ] **Step 6: Run parser, formatter, and type-check tests**

Run:

```text
cargo test -p cel-parser
```

Expected: PASS with no warnings.

- [ ] **Step 7: Commit**

```text
git add cel-parser/src/lib.rs cel-parser/src/ast.rs cel-parser/src/fmt.rs cel-parser/src/ty.rs
git commit -m "feat(cel-parser): support typed array annotations"
```

### Task 4: Adapt `adam-lang::TypeRegistry` without moving sheet behavior

**Files:**
- Modify: `adam-lang/src/type_registry.rs`
- Modify: `adam-lang/src/parser.rs`
- Modify: `adam-lang/src/typecheck.rs`
- Modify: `adam-lang/src/lib.rs` if the generic adapter is publicly exposed
- Test: `adam-lang/src/type_registry.rs`, `adam-lang/src/parser.rs`, and `adam-lang/src/typecheck.rs`

**Interfaces:**
- Consumes: Task 2's generic CEL resolver/descriptor contract and Task 3's `TypeExpr`.
- Produces: an adapter from registered `TypeEntry` metadata to the parser resolver; custom registered
  types work in direct parsing and deferred AST/type checking without exposing Adam sheet
  constructors to `cel-parser`.

- [ ] **Step 1: Write failing Adam integration tests**

Register a custom type with `TypeRegistry::register_no_default`, then assert:

```text
cell values: [Custom, Custom]: [Custom]
cell empty: []: [Custom]
```

(or the equivalent existing Adam source syntax) parses and constructs `DynamicArray` values. Add a
negative test for an unknown custom type and a type-checking mismatch.

- [ ] **Step 2: Run focused tests and verify failure**

Run:

```text
cargo test -p adam-lang typed_array
cargo test -p adam-lang type_registry
```

Expected: the parser cannot resolve the custom annotation before the adapter exists.

- [ ] **Step 3: Implement the adapter**

Expose only generic descriptor metadata from each `TypeEntry` needed by CEL array construction.
Thread the adapter into the embedded CEL parser for `AdamParser::parse_str` and into the AST
checker’s resolver. Keep `add_cell_fn`, conditional construction, output extraction, and other
sheet functions in `adam-lang`.

- [ ] **Step 4: Run Adam tests**

Run:

```text
cargo test -p adam-lang
```

Expected: PASS with no warnings.

- [ ] **Step 5: Commit**

```text
git add adam-lang/src/type_registry.rs adam-lang/src/parser.rs adam-lang/src/typecheck.rs adam-lang/src/lib.rs
git commit -m "feat(adam-lang): resolve typed array annotations from registry"
```

### Task 5: Add reusable type-ascription coverage and documentation

**Files:**
- Modify: `cel-parser/src/lib.rs` module grammar and examples
- Modify: `cel-parser/src/ast.rs` and `cel-parser/src/fmt.rs` API documentation
- Modify: `adam-lang/src/lib.rs` grammar documentation and examples
- Modify: relevant `docs/` array documentation if the merged array design references the old empty-array limitation
- Test: existing doctests and focused unit tests

**Interfaces:**
- Consumes: completed runtime, parser, and registry behavior from Tasks 1–4.
- Produces: synchronized grammar, examples, diagnostics, and public contracts for `[0, 1]: [i32]`,
  `[]: [i32]`, nested array types, reusable tuple type expressions, and future reuse of `TypeExpr`.

- [ ] **Step 1: Search and update stale documentation**

Search for the old issue-212 empty-array diagnostic and old suffix forms:

```text
rg "empty array|issue.*212|\[\]i32|\]i32|suffix" cel-parser adam-lang docs
```

Replace only statements made obsolete by this implementation; retain limitations unrelated to
typed annotations.

- [ ] **Step 2: Add contract examples**

Document successful scalar, nested-array, custom-registry, and empty-array annotations, plus tuple
type-expression parsing and the explicit unsupported tuple-array diagnostic. Document the exact
mismatch/unknown-type error behavior. Reuse the same recursive type grammar in the module-level
grammar blocks.

- [ ] **Step 3: Run documentation tests**

Run:

```text
cargo test --doc -p cel-parser -p adam-lang -p cel-runtime
```

Expected: PASS with no warnings.

- [ ] **Step 4: Commit**

```text
git add cel-parser adam-lang cel-runtime docs
git commit -m "docs: document typed array annotations"
```

### Task 6: Full verification and issue handoff

**Files:**
- Modify: none unless verification exposes a regression in the files above
- Test: workspace build, tests, doctests, and clippy commands

**Interfaces:**
- Consumes: all implementation commits from Tasks 1–5.
- Produces: verified issue resolution with no unaddressed warnings or stale behavior.

- [ ] **Step 1: Format**

Run:

```text
cargo fmt --all
```

If formatting changes tracked files, review and commit those changes before testing.

- [ ] **Step 2: Build and test the workspace**

Run:

```text
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
```

Expected: all commands pass with no compiler warnings.

- [ ] **Step 3: Run all required clippy checks**

Run:

```text
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

Expected: all commands pass.

- [ ] **Step 4: Inspect the final diff**

Run:

```text
git status --short
git diff main...HEAD --stat
git diff --check
```

Confirm the diff contains only the typed-array implementation, directly related documentation,
tests, and the approved design/plan artifacts.

- [ ] **Step 5: Update issue workflow state**

Mark the implementation and verification todos complete only after the commands above pass. If a
requirement cannot be implemented without changing the approved architecture, stop and revise the
spec rather than silently narrowing the behavior.
