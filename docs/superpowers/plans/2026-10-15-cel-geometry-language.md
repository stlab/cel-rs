# CEL Geometry Language Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Support registered Rust-like records and receiver operations without knowing any geometry types.

**Architecture:** Extend `OpLookup` with shared signature/schema metadata and retain existing scopes. Both runtime and AST contexts receive dedicated record/member operations. Complete visitors, formatting and checking before downstream libraries adopt the syntax.

**Tech Stack:** Rust 2024, `proc_macro2`, current `DynSegment`, heterogeneous cons lists, existing parser/formatter tests.

**Spec:** `docs\superpowers\specs\2026-10-15-cel-euclid-design.md`; rollout: `2026-10-15-cel-euclid.md`.

## Global Constraints

- No core parser or runtime module may depend on Euclid or hardcode its type names.
- Cells and locals named `x`, `width`, or similar do not shadow dot operations.
- Each field is required exactly once.
- Evaluate field expressions exactly once in written order.
- Do not add speculative parsing or semantic type-name lookahead.
- Existing expression precedence remains unchanged.
- Do not require rewriting unrelated `cel-std` behavior.

## Review Focus

- Two libraries registering `min` must preserve scalar calls and newly added receiver overloads.
- A marker returned by a conditional/closure must retain its callable identity or fail explicitly, never invoke the wrong function.
- Constructor field reordering must not use `RawStack::repack` outside its documented safety contract.
- Parentheses re-enable records in restricted headers, while nested bodies regain ordinary parsing.
- Member names/field labels must not appear as Adam dependencies, including inside closures.

---

## Shared interfaces

Add `cel-parser\src\registered_ops.rs`, re-exported from `lib.rs`:

```rust
pub type OperationEmitter =
    std::sync::Arc<dyn Fn(&mut DynSegment, SourceSpan) -> anyhow::Result<()> + Send + Sync>;
pub type RecordEmitter =
    std::sync::Arc<dyn Fn(&mut DynSegment, &[usize], usize, SourceSpan)
        -> anyhow::Result<()> + Send + Sync>;

pub struct OperationSignature {
    pub name: &'static str,
    pub operands: Vec<ResolvedLeafType>,
    pub result: ResolvedLeafType,
    pub emit: OperationEmitter,
}
pub struct RecordField {
    pub name: &'static str,
    pub ty: ResolvedLeafType,
}
pub struct RecordSchema {
    pub ty: ResolvedLeafType,
    pub fields: Vec<RecordField>,
    pub emit: RecordEmitter,
}
```

`RecordEmitter` receives source-position -> schema-position indices, the
ambient stack offset before fields, and source span. It consumes source-order
values and produces one concrete record. This avoids a generic raw-memory
permutation facility. A host can handle heterogeneous orders with typed
adapters; homogeneous fields use one tuple/cons-list adapter and index mapping.
Do not generate every possible field ordering.

New `OpLookup` methods:

```rust
pub fn register_operation(&mut self, signature: OperationSignature) -> anyhow::Result<()>;
pub fn register_function<M: Clone + 'static>(
    &mut self, name: &'static str, marker: M
) -> anyhow::Result<()>;
pub fn register_record(&mut self, schema: RecordSchema) -> anyhow::Result<()>;
pub fn operation_signatures(&self, name: &str) -> impl Iterator<Item = &OperationSignature>;
pub fn record_schema(&self, name: &str) -> Option<&RecordSchema>;
pub fn lookup_member(
    &self, name: &str, segment: &mut DynSegment, arity: usize, start: Span, end: Span
) -> crate::Result<()>;
pub fn construct_record(
    &self, name: &str, fields: &[(String, ExprSpan)], ambient_start: usize,
    segment: &mut DynSegment, start: Span, end: Span
) -> crate::Result<()>;
```

Registrations are permanent across scope isolation. Reject duplicate operand
signatures, result-only overloads, duplicate schema fields/names, and a marker
type reused for distinct function names. Registering an already bound function
name retains its original marker, allowing later libraries to extend the
overload set with a different local marker type. Library installers run once
per lookup; duplicate installation is a setup error, not a second precedence layer.

### Task 1: Shared operation registration and composable ordinary calls

**Prerequisites:** None.

**Files:** Create `cel-parser\src\registered_ops.rs`,
`cel-parser\tests\registered_ops.rs`; modify `cel-parser\src\{lib,op_table}.rs`,
`cel-runtime\src\dyn_segment.rs`, `cel-std\src\math.rs`.

**Interfaces:** Produces registration, metadata and `lookup_member` methods
above; `construct_record` is completed in Task 3. Add
`DynSegment::stack_offset_before_top(n: usize) -> anyhow::Result<usize>`,
a checked query using the existing stack-offset calculation.

- [ ] **Step 1: Add contract tests** `overloads_from_two_installers_compose`,
  `value_binding_does_not_hide_direct_operation`,
  `duplicate_operand_signature_is_rejected`,
  `marker_cannot_name_two_functions`, and
  `failed_lookup_preserves_stack`. Add
  `zero_arg_function_invokes_operation_not_marker`.
  Assert `min(3.0, 7.0) == 3.0`, a newly registered unary `min(TestBox)`
  returns its corner, both installation orders work, and a declined signature
  leaves the same operand count/type identities.
- [ ] **Step 2: Run** `cargo test -p cel-parser --test registered_ops`.
  Expected: missing registration API failures.
- [ ] **Step 3: Implement registration and dispatch.** Existing transient
  scopes take precedence; member lookup never performs a zero-arity value
  lookup. Check complete operand descriptors before invoking an emitter.
  A normal registered call matches its existing marker type, dispatches the
  same direct operation on the arguments, then removes the marker using
  `make_tuple(2, marker_ambient_start)` and `tuple_index(1)`. Use the checked
  offset query, not raw casts. Metadata and constructor descriptors must
  preserve full type identity. Zero-argument calls select the registered
  zero-operand signature directly: repeating ordinary zero-arity identifier
  lookup would push another function marker instead of invoking it.
- [ ] **Step 4: Move existing `cel-std` math overloads onto this registration
  path**, keeping private per-name markers and exactly the existing
  numeric/error contracts. No new math operations. Add
  `registered_function_survives_scope_isolation` and retain existing tests
  for closure calls and all numeric types.
- [ ] **Step 5: Run** `cargo test -p cel-parser -p cel-runtime -p cel-std`.
  Expected: all old and new tests pass, including a conditional/closure
  choosing incompatible function markers producing the existing type error,
  not a wrong call.
- [ ] **Step 6: Format and commit** the listed files and inline contracts:
  `feat(cel): register composable named operation signatures`.

### Task 2: Receiver syntax through both parser contexts

**Prerequisites:** Task 1.

**Files:** Modify `cel-parser\src\{lib,parser_context,ast,fmt,ty}.rs`;
create `cel-parser\tests\members.rs`; modify
`adam-lang\src\typecheck.rs` and directly affected exhaustive AST consumers.

**Interfaces:** Add:

```rust
Expr::Member {
    receiver: Box<Expr>, name: String, name_span: ExprSpan,
    args: Option<Vec<Expr>>, span: ExprSpan
}
ParserContext::apply_member(
    &mut self, lookup: &OpLookup, name: &str, name_span: ExprSpan,
    explicit_args: Option<usize>, start: Span, end: Span
) -> crate::Result<()>;
```

`None` records `.x`; `Some(0)` records `.x()`; both dispatch with one operand.
The AST context pops already-emitted receiver/argument nodes without resolving
their types.

- [ ] **Step 1: Add tests** `member_property_and_zero_arg_call_agree`,
  `member_call_passes_receiver_first`, `member_chains_with_tuple_index`,
  `shadowed_free_function_does_not_hide_member`,
  `parenthesized_member_result_is_an_ordinary_callee`, and
  `receiver_and_arguments_evaluate_once_in_order`.
  Use custom `TestPoint`/`TestVector`, not Euclid; assert `(p.x)()` fails for
  a scalar while `p.x()` returns the scalar. Instrument test-only host
  operations to assert evaluation trace `[receiver, arg1, arg2]`.
- [ ] **Step 2: Run** `cargo test -p cel-parser --test members`.
  Expected: member syntax/AST variants unsupported.
- [ ] **Step 3: Extend the existing postfix loop.** Preserve `.N` and
  float-token index splitting; consume a named member and optional argument
  list as one production. Pass the receiver plus explicit arguments to
  `lookup_member`; do not emit an identifier lookup or nested scalar call.
- [ ] **Step 4: Preserve AST spelling and update consumers.** Implement
  `span()`, formatter/comment gaps and precedence, recursive checker walks,
  and all exhaustive matches, including `adam-lang::expr_references_ident`.
  Add format/parse assertions for `a.center().x`, `a.x()`, `(a.x)()`,
  comments around dots, and unknown member diagnostics at the name span.
- [ ] **Step 5: Run** `cargo test -p cel-parser -p adam-lang -p adam-fmt`;
  `cargo check -p ez-adam --no-default-features`.
  Expected: all pass; old tuple, callable and short-circuit tests unchanged.
- [ ] **Step 6: Format and commit**:
  `feat(cel): add receiver operation syntax`.

### Task 3: Registered record construction and brace restrictions

**Prerequisites:** Tasks 1 and 2.

**Files:** Modify `cel-parser\src\{registered_ops,lib,parser_context,ast,fmt,ty}.rs`,
`adam-lang\src\{parser,ast_parser,token_cursor,typecheck}.rs`;
create `cel-parser\tests\records.rs`.

**Interfaces:** Complete `construct_record`; add:

```rust
pub struct RecordInit {
    pub name: String, pub name_span: ExprSpan, pub value: Expr
}
Expr::Struct { name: String, name_span: ExprSpan,
    fields: Vec<RecordInit>, span: ExprSpan }
ParserContext::make_record(
    &mut self, lookup: &OpLookup, name: &str, name_span: ExprSpan,
    fields: Vec<(String, ExprSpan)>, ambient_start: usize, start: Span, end: Span
) -> crate::Result<()>;
pub enum ExpressionRestriction { None, NoStructLiteral }
Parser::parse_expression_ctx_restricted(
    &mut self, restriction: ExpressionRestriction
) -> crate::Result<C>;
```

Keep the existing unrestricted entry point delegating to `None`. Add the
corresponding `DynSegment`-returning restricted entry point for Adam.

- [ ] **Step 1: Add tests** `record_field_order_is_named`,
  `record_missing_duplicate_unknown_fields_are_errors`,
  `record_scalar_types_are_exact`, `nested_record_and_trailing_comma`,
  `record_type_namespace_ignores_value_shadowing`, and
  `heterogeneous_source_order_drops_once`.
  Register a record with a `String` and an aligned test value; reverse its
  fields, assert written evaluation order and one drop per constructed value,
  including a fallible second field. Test a zero-field registered record too.
- [ ] **Step 2: Run** `cargo test -p cel-parser --test records`.
  Expected: record syntax unsupported.
- [ ] **Step 3: Implement the left-factored primary and record schema checks.**
  Recognize `{` only after the identifier and only when allowed. Validate
  names/duplicates before emission and missing/type-mismatched fields before
  calling the emitter. Preserve source order; never repack a permutation.
  Add the dedicated AST representation, source spans and formatter support.
- [ ] **Step 4: Implement restricted-expression state with restoration on
  success and error.** CEL `if` conditions and Adam `when`/match headers
  reserve the opening body brace. Parenthesized expressions explicitly lift
  the restriction; field values and branch bodies use their own ordinary
  context. Inherit the restriction through binary/unary/closure expressions
  unless a delimiter establishes a new expression boundary.
- [ ] **Step 5: Pin grammar regressions** with
  `if_identifier_condition_keeps_its_body`,
  `parenthesized_record_in_header_is_allowed`,
  `record_in_branch_body_is_allowed`,
  `adam_condition_and_match_headers_reserve_braces`,
  and `restriction_does_not_leak_after_parse_error`.
  Assert existing `if true { 1 } else { 2 }` returns `1`; parse/format
  `if (TestPoint { x: 0.0, y: 1.0 }).x == 0.0 { 1 } else { 2 }`.
- [ ] **Step 6: Run** `cargo test -p cel-parser -p adam-lang -p adam-fmt`;
  `cargo check -p ez-adam --no-default-features`.
  Expected: all pass, including legacy condition recovery.
- [ ] **Step 7: Format and commit**:
  `feat(cel): construct registered records with predictive grammar`.

### Task 4: Metadata-aware checking, AST consumers and language documentation

**Prerequisites:** Tasks 1-3.

**Files:** Modify `cel-parser\src\{ty,type_expr,lib}.rs`,
`adam-lang\src\{typecheck,type_registry,lib}.rs`,
`adam-lsp\src\diagnostics.rs`, `ez-adam\src\codegen\ast_builder.rs`,
affected tests; modify `cel-lang-book\book-src\{reference,literals-and-types,operators}.md`,
`adam-lang-book\book-src\{expressions,reference}.md`.
Inspect `editors\vscode-adam-lang\syntaxes\adam-lang.tmLanguage.json`;
change it and its existing tests only if needed.

**Interfaces:** Add `Ty::Registered(ResolvedLeafType)` for known custom leaf
identity. Add `lookup: &OpLookup` as the fourth parameter of
`check_expr_with_type_resolver` and as the third parameter of
`adam_lang::check_sheet`; update callers, examples and recursive checks.
Primitive-only `check_expr` continues using an empty lookup. Use
`OpLookup::operation_signatures`/`record_schema`, not geometry special cases.

- [ ] **Step 1: Add tests** `known_record_member_types_are_checked`,
  `unknown_record_type_is_diagnostic`,
  `different_registered_types_do_not_unify`,
  `unresolved_member_checks_its_argument_expressions`,
  `field_labels_and_member_names_are_not_cell_dependencies`,
  `closure_parameter_shadows_only_value_references`, and
  `ez_adam_formula_round_trips_member_and_record_syntax`.
  Assert a known `f64` field initialized by `1` is diagnosed, unknown custom
  calls retain best-effort checking, and the only dependencies of
  `TestPoint { x: a, y: b }.x` are `a` and `b`.
- [ ] **Step 2: Run** `cargo test -p cel-parser -p adam-lang -p adam-lsp`;
  `cargo test -p ez-adam --no-default-features`.
  Expected: new checking assertions fail before implementation.
- [ ] **Step 3: Thread metadata through the checker and consumers.** Use
  registry descriptors for custom annotations and known constructor/member
  results, preserve `Any` for genuinely unresolved inputs, and report
  mismatched known operands without cascading fake errors.
  Check all new AST children; never treat field/member labels as identifiers.
  Update all callers found by compiler errors and a bounded `Expr::` audit.
- [ ] **Step 4: Document syntax and error contracts**, including bare vs
  parenthesized members, value-name shadowing, rejection of trailing commas, restricted
  headers, no implicit integer-to-float conversion, and no member assignment.
  Add examples to existing book example harnesses, not untested prose only.
- [ ] **Step 5: Run** `cargo test -p cel-parser -p cel-std -p adam-lang -p adam-lsp -p adam-fmt -p cel-lang-book -p adam-lang-book`;
  `cargo test -p ez-adam --no-default-features`;
  `$env:RUSTDOCFLAGS = '-D warnings'; cargo doc -p cel-parser -p adam-lang --no-deps --lib`.
  Run extension compile/tests if changed. Expected: all pass.
- [ ] **Step 6: Format, review the stage against the spec, fix findings, and
  commit**: `feat(cel): check and document registered expression forms`.

The resulting language stage must run with no `cel-euclid` package present.
Continue with `2026-10-15-cel-euclid-library.md` only after this gate.
