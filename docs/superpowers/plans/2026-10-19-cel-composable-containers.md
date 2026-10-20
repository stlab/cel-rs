# Composable CEL Containers Implementation Plan

> **Execution status (2026-10-19):** Tasks 1–8 are implemented and reviewed.
> Task 4 was decomposed into bounded prerequisites and one atomic canonical
> cutover. Task 9 verifies the final APIs rather than copying prerequisite-era
> names. See the [handoff](../2026-10-19-composable-containers-handoff.md)
> for actual commands/results and remaining gate/review limits. The original
> approved plan and checkboxes below are preserved as history.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make CEL arrays and tuples compose over all expressible value types with concrete Rust interoperability and construction-validated, check-free segment execution.

**Architecture:** Use `Array<T>` for homogeneous storage and logical-source-order `CStackList` values for tuples. One recursive descriptor describes layout, identity, ownership, optional capabilities, and callable signatures; typed registration witnesses validate concrete host adapters once. CEL and Adam share that descriptor rather than maintaining narrower array-only or tuple-only type systems.

**Tech Stack:** Rust 2024, the existing CEL stack runtime/parser, Adam libraries, Cargo tests/Clippy/rustdoc; no new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-19-cel-composable-containers-design.md` (approved 2026-10-19).

**Execution approval:** The user approved this plan and selected subagent-driven execution on 2026-10-19.

## Global Constraints

- "Arrays are homogeneous, and tuples retain each positional element's type."
- "Both containers can hold any expressible CEL value type recursively."
- "Executing a validated segment does not repeat type checking."
- "Future macro-compiled CEL uses the same Rust-facing operations as interpreted CEL, without user-code modifications."
- Canonical nested arrays are `Array<Array<T>>`; their vector view is `Vec<Array<T>>`, not `Vec<Vec<T>>`.
- The canonical tuple is `CNil<()>` or `CStackList<Head, Tail>` in logical source order, physically tail-first.
- `Array<T>` transfers allocations to/from `Vec<T>` and lends `&[T]`; never reinterpret a Rust vector header.
- Construction requires ownership/layout support, not `Clone`, `PartialEq`, or `Debug`.
- Keep reverse-element interpreter cleanup; transferred vectors follow Rust's vector destruction rules.
- No capturing closures, iterator operations, mutable CEL containers, deep native-vector conversion, or macro backend.
- Keep `cel-euclid/tests/crop_straighten_math.rs` unchanged and exclude it from container commits.
- Work in the existing worktree, never commit to `main`, and include the required Copilot co-author trailer on implementation commits.
- Each commit step means `cargo fmt --all`, explicit staging of that task's files, and a non-amended commit with its listed subject and `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`.

## Review Focus

1. Foreign registered types and generic leaves must compose without orphan-rule-violating trait implementations; exercise an actual Euclid point and a custom move-only leaf (Tasks 3, 6).
2. Equal aggregate marker IDs or total layouts must not make distinct nested tuple/callable types interchangeable; reject them before side effects (Tasks 3, 5, 6).
3. Logical tuple index order must survive tail-first layout, trailing padding, nesting, and zero-sized fields; compare against actual Rust lists (Tasks 2, 4).
4. Failure or panic after partial nested initialization must destroy every initialized owner exactly once without reading padding (Tasks 1, 3, 4).
5. Repeated prepared execution and borrowed slices must not retain expired frames, duplicate owners, or revalidate signatures; exercise native/interpreted callables and Adam updates (Tasks 4, 5, 7).

---

## File Structure and Migration Rules

| Area | Files and responsibility |
|---|---|
| Typed arrays | Create `cel-runtime/src/array.rs`: fixed-layout raw header, typed owner, vector/slice access, raw allocation builder. |
| Tuple values | Modify `cel-runtime/src/c_stack_list.rs`; create `cel-runtime/src/tuple_value.rs`: native tuple bridges and canonical layout calculation. |
| Shared types | Create `cel-runtime/src/value_type.rs`: replacement complete descriptors and typed representation witnesses; switch the public re-exports and retire the old definitions during Task 4's atomic interpreter cutover. |
| Persistent erased ownership | Create `cel-runtime/src/owned_value.rs`: one whole-value owner with a frozen descriptor, never element boxing. |
| Prepared execution | Create `cel-runtime/src/prepared_segment.rs`; modify `dyn_segment.rs`, `segment.rs`, and `raw_stack.rs`: trusted frames, packing, typed adapters. |
| Callables | Create `cel-runtime/src/callable.rs`; migrate `dyn_closure.rs`: common carrier, concrete signature-typed facade, native/interpreted dispatch. |
| CEL | Modify `type_expr.rs`, `parser_context.rs`, `lib.rs`, `ty.rs`, `registered_ops.rs`, `op_table.rs`, `ast.rs`, and `fmt.rs` in `cel-parser/src`. |
| Adam | Modify `adam-lang/src/{type_registry,parser,typecheck,lib}.rs`; create `adam-lang/src/cell_value.rs`: capability-validated persistent cells and prepared bridges. |
| Direct clients | Update `cel-std/src/math.rs`, `cel-euclid/src/{lib,geometry,transforms,records,adam}.rs`, and `adam-web-ui/src/labels.rs` only where descriptors/storage changed. |
| Contract tests | Create focused integration tests named in each task; migrate old positive/negative fixtures with each relevant cutover. |
| Guidance | Update CEL/Adam books, `docs/VISION.md`, affected historical specs, and a dated implementation handoff. |

Tasks 1-3 introduce independently testable foundations. Task 4 changes the
interpreter representation and must migrate its immediate parser/test callers
in the same commit. Task 5 completes the callable execution path; Task 6
completes public syntax, operation registration, and diagnostic coverage.
Tasks 7-8 wire persistent consumers and remove superseded storage.

Dependencies: Tasks 1 and 2 have none; Task 3 requires both; each remaining
task requires its predecessor. In Task 3, the replacement descriptors are
module-scoped until Task 4 replaces the old interpreter metadata. This avoids
changing the layout descriptor before the operations using that layout change.

Keep the workspace compiling at each task boundary. Update direct call sites
of a changed API in that task, even when their behavioral coverage belongs to
a later task. Do not introduce new compatibility shims or retain two final
container ABIs. Existing legacy entry points may remain only until their
consumers migrate in Task 8; they are not the trusted path for new containers.

## Task 1: Concrete Typed Array Ownership

**Files:** Create `cel-runtime/src/array.rs`, `cel-runtime/tests/array_value.rs`; modify `cel-runtime/src/lib.rs`.

**Interfaces:**
- Produces `Array<T>` with `from_vec(Vec<T>) -> Self`, `into_vec(self) -> Vec<T>`, `as_slice(&self) -> &[T]`, `len() -> usize`, `capacity() -> usize`, and `is_empty() -> bool`.
- Produces crate-private `RawArray` with a documented `repr(C)` pointer/length/capacity layout and no descriptor or destructor.
- The transparent typed owner implements destruction; erased execution later supplies the same destruction through its descriptor.

- [ ] **Step 1: Write contract tests** for pointer/capacity preservation and nested typed ownership:

```rust
let mut values = Vec::with_capacity(8);
values.extend([3_i32, 5]);
let pointer = values.as_ptr();
let capacity = values.capacity();
let array = Array::from_vec(values);
assert_eq!(array.as_slice(), &[3, 5]);
assert_eq!(array.as_slice().as_ptr(), pointer);
let values = array.into_vec();
assert_eq!(values.as_ptr(), pointer);
assert_eq!(values.capacity(), capacity);
assert_eq!(values, vec![3, 5]);
```

Add `nested_array_keeps_outer_allocation`, `empty_and_zst_arrays_round_trip`,
`overaligned_move_only_elements_round_trip`, and
`typed_array_drop_runs_once_in_reverse_order`. Test ZST destructors separately
from pointer identity; do not assume a nonallocated pointer is unique. Add
compile-time assertions that `Array<String>` is `Send + Sync` and compile-fail
examples ensuring non-`Send`/non-`Sync` elements do not acquire those traits.

- [ ] **Step 2: Run** `cargo test -p cel-runtime --test array_value`; first establish missing-API failure, then a behavioral failure for an incorrect ownership implementation.
- [ ] **Step 3: Implement** the listed APIs with `ManuallyDrop`, stable vector raw-parts APIs, `PhantomData<T>`, and typed slice construction. Add conditional `Clone`, `PartialEq`, and `Debug` implementations only under the respective `T` bounds; typed `Send`/`Sync` must follow element ownership as for `Vec<T>`.
- [ ] **Step 4: Verify** `cargo test -p cel-runtime --test array_value` and `cargo clippy -p cel-runtime --test array_value -- -D warnings` pass.
- [ ] **Step 5: Format and commit** only this task as `feat(runtime): add concrete typed array ownership`.

## Task 2: Canonical Tuple Lists and Native Tuple Bridges

**Files:** Create `cel-runtime/src/tuple_value.rs`, `cel-runtime/tests/tuple_value.rs`; modify `cel-runtime/src/{c_stack_list,lib}.rs`.

**Interfaces:**
- Consumes existing `IntoCStackList`.
- Produces `IntoRustTuple` with associated `Tuple` and `into_rust_tuple(self) -> Self::Tuple` for canonical lists corresponding to native tuple arities 0 through 12.
- Produces crate-private `TupleLayout::from_fields(&[Layout]) -> anyhow::Result<TupleLayout>` with source-indexed offsets and aggregate layout.
- Arbitrary list lengths remain supported; only native tuple convenience bridges have the existing arity limit.

- [ ] **Step 1: Write tests** whose observable round trip is:

```rust
let list = (String::from("left"), 7_u8, 2.5_f64).into_c_stack_list();
let value: (String, u8, f64) = list.into_rust_tuple();
assert_eq!(value, (String::from("left"), 7, 2.5));
```

Add move-only, nested-list, empty, singleton, 12-field, and over-aligned cases.
In `tuple_value.rs` unit tests, compare calculated source-index offsets for
`(u8, f64, u16)` with references to the actual list fields and calculated
aggregate layout with `Layout::new`. Include a zero-sized head, middle field,
and tail. Keep integration tests on public conversion APIs.

- [ ] **Step 2: Run** `cargo test -p cel-runtime --test tuple_value` and confirm the intended failures.
- [ ] **Step 3: Implement** conversions without cloning or transmutation. Calculate layout from the tail using checked `Layout::extend`/padding, not Rust tuple layout or the old prefix-rounding algorithm. Add contract comments, examples, and no new bounds on stored heads.
- [ ] **Step 4: Verify** `cargo test -p cel-runtime --test tuple_value` and the existing `c_stack_list` unit tests pass.
- [ ] **Step 5: Format and commit** as `feat(runtime): define canonical tuple list interop`.

## Task 3: Complete Type Descriptors and Whole-Value Ownership

**Files:** Create `cel-runtime/src/{value_type,owned_value}.rs`, `cel-runtime/tests/value_types.rs`; modify `cel-runtime/src/lib.rs`.

**Interfaces:**
- Define replacement `ValueType`, `ValueKind`, and `AssociatedType` in `value_type.rs`; expose them through that module rather than root re-exports until Task 4's cutover. Include structural callable signatures now; Task 5 adds callable value construction.
- `ValueType::array(element: ValueType) -> Self` and `ValueType::tuple(elements: Vec<AssociatedType>) -> anyhow::Result<Self>` use canonical layouts; tuple layout overflow is an error. `same_shape(&self, other: &Self) -> bool` compares full identities.
- Produce a sealed `ValueRepr` trait with `type Value: 'static` and `fn value_type() -> ValueType`.
- Produce type-only witnesses `Leaf<T>`, `ArrayOf<E>`, `Nil`, and `Cons<H, T>`. Their values are respectively `T`, `Array<E::Value>`, `CNil<()>`, and `CStackList<H::Value, T::Value>`. Task 5 adds `FnOf<A, R>` after defining its value type.
- Restrict tuple tails to canonical lists. Witnesses add no value wrappers and never require an implementation on a foreign Rust leaf.
- Provide `ValueType::leaf<T: 'static>() -> Self` and `cloneable_leaf<T: Clone + 'static>() -> Self`. Add `with_clone<R: ValueRepr>(self) -> anyhow::Result<Self>` where `R::Value: Clone`, `with_equality<R: ValueRepr>(self) -> anyhow::Result<Self>` where `R::Value: PartialEq`, and `with_debug<R: ValueRepr>(self) -> anyhow::Result<Self>` where `R::Value: Debug`. Each validates complete identity/layout against its witness during construction before installing a hook; optional capability absence is independent.
- Produce `OwnedValue::new<R: ValueRepr>(value: R::Value) -> Self`, `value_type(&self) -> &ValueType`, and checked `try_into_typed<R: ValueRepr>(self) -> anyhow::Result<R::Value>`.
- Provide private raw borrow/move and prevalidated clone/drop plans for execution; checked external conversion is not the segment-internal API.

- [ ] **Step 1: Write tests** for `ArrayOf<Cons<Leaf<i32>, Cons<ArrayOf<Leaf<f64>>, Nil>>>`, including identical layouts with different leaf identities and differently nested arrays. Assert a successful `OwnedValue` typed move preserves an array's allocated pointer, and a mismatch cannot execute a supplied operation.
- [ ] **Step 2: Test** a custom leaf with `Drop` but no `Clone`, `PartialEq`, or `Debug`; it must store/move/drop in either container. Test optional capability absence returns an explicit error only when that capability is requested. Add panic-cleanup tests for recursively cloning a prefix whose next leaf clone panics.
- [ ] **Step 3: Run** `cargo test -p cel-runtime --test value_types` to establish the intended failures.
- [ ] **Step 4: Implement** one descriptor tree with frozen ownership plans. Derive tuple layout through Task 2 and array layout through Task 1. Structural callable metadata is available now; Task 5 supplies its execution carrier. Separate a leaf's semantic `TypeId` from aggregate physical carriers; complete descriptors, not physical marker IDs, authorize casts.
- [ ] **Step 5: Verify** the three new runtime integration targets and `cargo clippy -p cel-runtime --all-targets -- -D warnings`. Document any existing API migration in the same commit; no source-level `dyn`/`Any` type is introduced.
- [ ] **Step 6: Format and commit** as `refactor(runtime): unify recursive value type and ownership metadata`.

## Task 4: Canonical Construction and Prepared Typed Execution

**Files:** Create `cel-runtime/src/prepared_segment.rs`, `cel-parser/tests/prepared_containers.rs`; modify `cel-runtime/src/{dyn_segment,segment,raw_stack,lib}.rs`, `cel-parser/src/{parser_context,lib}.rs`, and immediate compile/test callers. Keep raw-frame unit tests in `prepared_segment.rs`; do not add a parser dependency to the runtime.

**Interfaces:**
- `DynSegment::make_array`/`make_typed_array` consume complete element `ValueType` and produce canonical raw array headers.
- `make_tuple`, `tuple_index`, and tuple relabeling use source-order canonical lists, including `()` as `CNil<()>`.
- Produce `DynSegment::prepare<A: ValueRepr, R: ValueRepr>(self) -> anyhow::Result<PreparedSegment<A, R>>`, where `A` describes a canonical argument list.
- `PreparedSegment<A, R>::call(&mut self, args: &A::Value) -> anyhow::Result<R::Value>` performs no shape checks or `Any` downcasts. Preparation validates clone capabilities for borrowed argument reads.
- Add descriptor-aware `op0_repr`, `op1_repr`, `op2_repr`, `op3_repr` and fallible counterparts alongside ordinary leaf registration, preserving existing arities and fallible cleanup.
- Add `op1_vec<E, R, F>` and `op1_slice<E, R, F>` adapters for `F: Fn(Vec<E::Value>) -> R::Value` and `F: Fn(&[E::Value]) -> R::Value`; supply fallible variants. Results are owned `'static` values; borrowed frames cannot escape.

The unary descriptor-aware signature is
`op1_repr<A: ValueRepr, R: ValueRepr, F>(&mut self, f: F) -> anyhow::Result<()>`
where `F: Fn(A::Value) -> R::Value + 'static`. Higher arities use one witness
per source-order operand; `opNr_repr` counterparts return `anyhow::Result<R::Value>`.
Vector/slice adapters have the same return/setup contract and generic `F`
ownership as this unary signature.

- [ ] **Step 1: Write a parser-to-Rust test** preparing `[(1i32, 2.0f64), (3i32, 4.0f64)]` as `ArrayOf<Cons<Leaf<i32>, Cons<Leaf<f64>, Nil>>>`; consume the allocation as a vector of concrete lists and assert both native tuples equal `(1, 2.0)` and `(3, 4.0)`.
- [ ] **Step 2: Add tests** for `[((), [1i32]), ((), [2i32])]`, typed empty nested arrays, tuple `.0`/`.1` order, and array/tuple construction above ambient stack values. Prepare once, call repeatedly, retain the first returned owner, and verify later calls do not alias or mutate it.
- [ ] **Step 3: Add failure tests** in which a later element returns `Err` after earlier nested values initialize. Assert exact construction/destruction events and preserved error context. A borrowed-slice host function must observe the original element pointer and drop the consumed owner after returning.
- [ ] **Step 4: Run** `cargo test -p cel-parser --test prepared_containers` to establish behavioral failures before replacing packing.
- [ ] **Step 5: Implement** header construction, precomputed tuple moves, trusted raw argument/result frames, and monomorphized adapters. Validate complete descriptors in `prepare` and typed-segment conversions. Route emitted operations to private trusted moves, not checked `DynamicArray`/`DynamicSequence` conversion APIs.
- [ ] **Step 6: Switch** runtime re-exports to Task 3's descriptors and migrate every dependent descriptor call site. Remove executable tuple-array rejection now and update existing runtime/parser fixtures affected by the canonical ABI. The full callable syntax/checker work remains Task 6, not a reason to weaken a successful segment's type validation.
- [ ] **Step 7: Verify** `cargo test -p cel-runtime -p cel-parser`. Review the prepared call graph to confirm identity comparison and checked conversion occur only during construction/preparation; use a test-only comparison probe if necessary to demonstrate zero comparisons during repeated calls.
- [ ] **Step 8: Format and commit** as `feat(runtime): execute canonical containers through prepared typed boundaries`.

### Task 4A: Descriptor Factory and Callable Prerequisites

**Execution refinement:** Inspection found that Task 4's atomic switch needs
three separately tested prerequisite units. Tasks 4A-4D implement the existing
Task 4 contract; they do not add product scope or a second executable ABI.

**Files:** Modify `cel-runtime/src/{value_type,dyn_closure}.rs`,
`cel-parser/src/{op_table,type_expr}.rs`, and `adam-lang/src/type_registry.rs`;
add tests alongside those APIs.

**Interfaces:** Add complete-descriptor factories at concrete generic leaf
registration sites, without changing emitted interpreter operations yet.
Add `ValueType::callable(parameters: Vec<ValueType>, result: ValueType) -> Self`
as a crate-private constructor for the existing shared `DynClosure` carrier.
Keep complete callable metadata with its clone/drop plan, not a leaf marker.
Expose a conversion from a known legacy closure signature into complete new
metadata; unavailable signature metadata returns an error.

For legacy `TryFrom<DynSegment> for Segment<Args, Stack>`, ordinary opaque
leaf types can retain concrete identity validation. Structural aggregates
must use explicit witnesses through `prepare`; the legacy generic conversion
must reject structural values it cannot describe rather than authorize them
by shared marker IDs. Direct statically built `Segment` operations are
unaffected. Apply this rejection in 4D, with tests.

- [ ] Write tests for foreign registered leaf factories, preserved optional capabilities, structural closure argument/result mismatch, and source unit versus opaque host unit.
- [ ] Run focused runtime/parser/Adam selectors to establish RED.
- [ ] Implement factories while concrete `T` is known; no dynamic `TypeId` monomorphization or execution-path change.
- [ ] Verify `cargo test -p cel-runtime -p cel-parser -p adam-lang`; format and commit as `refactor: prepare complete container descriptor factories`.

### Task 4B: Private Canonical Frame and Move Primitives

**Files:** Create `cel-runtime/src/container_execution.rs`; modify
`cel-runtime/src/{owned_value,value_type,dyn_closure,lib}.rs` as needed.

**Interfaces:** Private construction plans consume complete descriptors,
initialized raw frames, canonical offsets, and allocation lengths. They
produce owned canonical array headers or packed list regions, with a matching
frozen drop plan. Add a trusted existing-closure invocation entry validated
by the emitter; retain checked external entry points separately.

- [ ] Write runtime unit tests for nested tuple/array packing, ambient storage, alignment, ZST destruction, repeated independent owners, and initialized-prefix cleanup on error and panic.
- [ ] Establish focused RED with `cargo test -p cel-runtime --lib container_execution`.
- [ ] Implement checked construction-time layouts and guarded raw ownership moves; do not switch the public interpreter or introduce a second public execution API.
- [ ] Verify `cargo test -p cel-runtime` and runtime all-target Clippy; format and commit as `feat(runtime): prepare canonical container move and frame plans`.

### Task 4C: Existing External Tuple Boundary Migration

**Files:** Create `cel-runtime/src/host_value_bridge.rs`; modify existing
`dynamic_sequence.rs` and registration-owned descriptor integration as needed.

**Interfaces:** Isolated existing external-host import/export uses complete
descriptors and canonical offsets. Preserve native leaf values and legacy
external tuple owners until transfer succeeds. Internal arrays and tuples
never store legacy wrapper elements. No new public compatibility interface.

- [ ] Write tests for nested tuple external round trips, foreign leaf identity, mismatches rejected before expression execution, and exact prefix cleanup after a conversion error.
- [ ] Establish focused RED with `cargo test -p cel-runtime --lib host_value_bridge`.
- [ ] Implement explicit moves/header conversion where necessary, never reinterpret different wrapper layouts. Trusted segment operations do not call these checked helpers.
- [ ] Verify `cargo test -p cel-runtime -p adam-lang`; format and commit as `refactor(runtime): prepare existing host boundaries for canonical values`.

### Task 4D: Atomic Interpreter Switch

**Files/interfaces:** Those in the parent Task 4, using 4A factories, 4B move
plans, and 4C existing-host boundaries. This is the only executable ABI switch.

- [ ] Add the parent Task 4 parser-to-Rust fixtures and prepared-call tests; verify their intended failures.
- [ ] Implement the parent Task 4 interfaces, switch root exports and all immediate metadata/emission callers atomically, and reject unsupported structural legacy typed conversions explicitly.
- [ ] Verify the parent Task 4 acceptance commands plus `cargo check -p adam-lang -p cel-euclid --features cel-euclid/adam`; ensure no second internal ABI, silent fallback, or runtime signature checks.
- [ ] Format and commit as `feat(runtime): execute canonical containers through prepared typed boundaries`; review all 4A-4D changes against the parent task before marking Task 4 complete.

## Task 5: Signature-Typed Callable Values

**Files:** Create `cel-runtime/src/callable.rs`, `cel-runtime/tests/callable_values.rs`; modify `cel-runtime/src/{dyn_closure,dyn_segment,value_type,lib}.rs`.

**Interfaces:**
- `Callable<Args, R>` has a representation independent of `Args`/`R`, but its type fixes both. It owns a shared private carrier whose dispatch accepts already-validated raw frames.
- `Callable::from_fn(f: fn(&Args) -> anyhow::Result<R>) -> Self`.
- `Callable::<A::Value, R::Value>::from_prepared(body: PreparedSegment<A, R>) -> Self` for tuple-argument witness `A`.
- `Callable::call(&self, args: &Args) -> anyhow::Result<R>` dispatches without comparing signatures; native and interpreted bodies use the same interface.
- Produce the Task 3 witness extension `FnOf<A, R>` whose value is `Callable<A::Value, R::Value>`; require `A` to describe a canonical argument list. Its `value_type()` retains full signatures, with clone support for the shared carrier and no implicit equality/default capability.

- [ ] **Step 1: Write tests** using both native and prepared bodies through the same Rust function:

```rust
fn apply(f: &Callable<CNil<()>, i32>) -> anyhow::Result<i32> {
    f.call(&CNil::root_empty())
}
```

Assert each body returns `7`; store both in `Array<Callable<CNil<()>, i32>>`
and tuple lists. Test a callable returning `Array<i32>` and one accepting a
tuple containing an array.

- [ ] **Step 2: Add tests** rejecting different callable arguments/results inside nested containers before any body executes. Reentrant invocation returns the documented error; invoking later after a body error remains valid. Add compile-fail rustdoc coverage for a typed callable invoked with the wrong Rust argument type.
- [ ] **Step 3: Run** `cargo test -p cel-runtime --test callable_values` and confirm failures.
- [ ] **Step 4: Implement** typed facade and carrier using function-pointer dispatch over a validated body. Sharing/cloning the carrier must not add `Clone` bounds to `Args` or `R`. Do not add a CEL dynamic function type or per-invocation `same_signature` check. Migrate CEL-emitted calls from `DynClosure::call_raw` to the prepared entry; arbitrary external erased-call APIs remain explicitly checked.
- [ ] **Step 5: Verify** callable tests, runtime tests, and the prepared execution comparison probe.
- [ ] **Step 6: Format and commit** as `feat(runtime): add typed native and interpreted callable values`.

## Task 6: Complete CEL Syntax, Registration, and Static Checking

**Files:** Modify `cel-parser/src/{type_expr,parser_context,lib,ty,registered_ops,op_table,ast,fmt}.rs`, `cel-std/src/math.rs`, and affected descriptor construction in `cel-euclid/src/{lib,geometry,transforms,records}.rs`; create `cel-parser/tests/container_composition.rs` and `cel-euclid/tests/container_interop.rs`; extend `cel-parser/tests/{registered_ops,records,closure_invocation}.rs`.

**Interfaces:**
- `ResolvedType` represents leaf, array, tuple, and callable shapes; `into_value_type(self) -> anyhow::Result<ValueType>` replaces narrow array-only resolution.
- `Fn((A, B), R)` describes two parameters; `Fn((), R)` describes zero. `Fn((A,), R)` describes one parameter, and singleton tuple types use `(T,)` consistently.
- `OperationSignature.operands`/`result` and `RecordField.ty` accept complete resolved types; record identity remains a registered leaf.
- `Ty` gains complete tuple/callable forms and retains unresolved state only for incomplete diagnostics, not executable runtime typing.
- Registered host leaf lookup stays available through `ResolvedLeafType`; aggregate composition needs no per-shape host registration or foreign trait impl.

- [ ] **Step 1: Write exact positive fixtures** for `[]: [(i32, [f64])]`, `[]: [Fn((i32,), [f64])]`, `((), [()])`, `[(1i32,), (2i32,)]`, and `[|x: i32| x + 1, |x: i32| x + 2]`. Check formatting round trips, AST types, executable descriptors, and typed Rust results.
- [ ] **Step 2: Write negative fixtures** for tuple arity/leaf mismatch, callable parameter/result mismatch, wrong empty-array annotation, and mismatched `if` branches. Assert source-span diagnostics and zero operand side effects when executable construction rejects them. An unresolved editor expression must not be accepted by executable parsing.
- [ ] **Step 3: Add real host-operation tests** for owned vectors, borrowed slices, tuple-list arguments/results, and compound record fields. In `cel-euclid/tests/container_interop.rs`, register an external Euclid `Point2D` leaf without implementing a runtime trait on it and round-trip `[(point, [point])]` through an operation; do not add a geometry dependency to CEL. Keep ordinary/member-call overload resolution equivalent.
- [ ] **Step 4: Run** `cargo test -p cel-parser --test container_composition` to confirm failures.
- [ ] **Step 5: Implement** structural callable resolution before host generic-leaf fallback; validate `Fn` argument count and list shape. Remove tuple-array rejection in both executable parsing and AST checking. Update operation duplicate detection, overload matching, record fields, branch joining, and closure intermediates to use complete descriptors.
- [ ] **Step 6: Verify** `cargo test -p cel-parser -p cel-std -p cel-euclid -p cel-lang-book` and `cargo clippy -p cel-parser -p cel-std -p cel-euclid --all-targets -- -D warnings`. Include `cargo test -p cel-euclid --features adam` after Task 7 migrates its Adam consumer.
- [ ] **Step 7: Format and commit** as `feat(cel): support recursively composable typed containers and callables`.

## Task 7: Adam Compound Cells and Prepared Evaluation

**Files:** Create `adam-lang/src/cell_value.rs`, `adam-lang/tests/container_cells.rs`; modify `adam-lang/src/{type_registry,parser,typecheck,lib}.rs`, `adam-lang/tests/registered_checking.rs`, and `cel-euclid/src/adam.rs` if its registration descriptors change.

**Interfaces:**
- Extend `TypeShape` structurally for arrays and callable signatures; derive it from the shared resolved type rather than synthesizing new aggregate Rust `TypeId`s.
- Internal `CellValue` owns one `OwnedValue` plus prevalidated clone/equality plans. Constructing a cell lacking required capabilities returns a source diagnostic before adding it to the sheet.
- `TypeRegistry::resolve` accepts registered generic leaves and composed arrays/tuples/callables; no aggregate instantiation registration is required.
- Registered primitive cells retain their concrete Rust read/write APIs. Compound cells use validated whole-value storage and typed read/write conversion at the external boundary.
- Compile relationships, initializers, outputs, filters, requirements, and conditional expressions into prepared raw frames. Existing Adam host-boundary `Any` access is not a substitute for a recursive CEL type check.

- [ ] **Step 1: Write a sheet fixture** with `source pairs: [(i32, [f64])]`, a tuple containing `pairs`, a relationship copying the source into an array-typed output, and a requirement consuming it through a registered slice operation. Update the source twice and assert output values, original source aspirations, and independent owned arrays.
- [ ] **Step 2: Add tests** for default empty arrays, default tuples containing arrays, singleton/empty tuples, explicit callable initializers, generic registered leaves, and destructuring multi-output methods whose fields include arrays of tuples. A missing leaf default identifies that leaf; a callable without a default requires an initializer.
- [ ] **Step 3: Test capability boundaries:** constructing/storing a callable array in CEL succeeds without equality; requesting an Adam cell of a type lacking required equality returns an explicit diagnostic. Never invent value equality or silently skip propagation. Registered function markers retain their registered capabilities.
- [ ] **Step 4: Run** `cargo test -p adam-lang --test container_cells` to establish failures.
- [ ] **Step 5: Implement** persistent ownership, shared resolution, prepared adapters, default construction, and output moves. Validate compound shapes and capability plans when building the sheet. Do not recursively downcast container elements or compare descriptors on every propagation. Do not redesign the Adam planner or globally remove `Any` from `adam-rs`.
- [ ] **Step 6: Verify** `cargo test -p adam-lang -p adam-lsp -p adam-lang-book`, `cargo test -p cel-euclid --features adam`, and `cargo clippy -p adam-lang -p adam-lsp --all-targets -- -D warnings`.
- [ ] **Step 7: Format and commit** as `feat(adam): preserve recursive container types in prepared cell evaluation`.

## Task 8: Migrate Persistent Clients and Retire Superseded Containers

**Files:** Modify `adam-web-ui/src/labels.rs`, `cel-runtime/src/lib.rs`, `cel-parser/src/lib.rs`, affected runtime/parser/Adam tests; remove superseded portions of `cel-runtime/src/{dynamic_array,dynamic_sequence,dyn_closure}.rs` once no production consumers remain.

**Interfaces:**
- Inspector compound-value metadata reads the Task 7 cell owner with a prepared formatting capability; it no longer assumes every tuple is `DynamicSequence`.
- One canonical array ABI and one canonical tuple ABI remain. A checked whole-value external boundary may remain; legacy array/tuple element representations may not.
- Preserve explicit unsupported-edit errors for compound cells; container composition does not add a compound-value editor.

- [ ] **Step 1: Extend label contract tests** with a tuple containing an array of tuples and an empty array. Assert meaningful formatted values and explicit unsupported-edit errors, while existing scalar read/write labels still work.
- [ ] **Step 2: Run** `cargo test -p adam-web-ui --lib labels` and confirm its failure before migrating storage.
- [ ] **Step 3: Migrate** direct consumers, test helpers, defaults, closure adapters, and crate exports. Remove old rejection fixtures or rewrite them as positive contract tests; retain mismatch/error coverage. Remove obsolete tuple conversion bounds and production use of `ArrayElementType`.
- [ ] **Step 4: Search** the workspace for `DynamicArray`, `DynamicSequence`, `ArrayElementType`, `DynTuple`, and the old tuple-array restriction. Every production hit must be removed or explicitly justified as the single checked whole-value boundary, not retained as a second element representation. Historical docs may mention the old names.
- [ ] **Step 5: Verify** affected crate tests and no-default-feature checks for `begin` and `ez-adam`. If changing rendered Inspector behavior rather than just storage plumbing, invoke `verifying-begin-ui` and perform rendered verification.
- [ ] **Step 6: Format and commit** as `refactor: retire superseded CEL container storage and migrate clients`.

## Task 9: Executable Documentation and Final Cross-Backend Contract Gate

**Files:** Modify `cel-lang-book/book-src/{collections,closures,literals-and-types,reference}.md`, `adam-lang-book/book-src/{cells,reference}.md`, associated book tests, `docs/VISION.md`, adjacent library docs, and historical container/type/closure specs; create `docs/superpowers/2026-10-19-composable-containers-handoff.md`.

**Interfaces:** Documentation uses the final Task 1-8 API names and the approved recursive Rust mapping. The handoff records completed tasks, actual commands/results, deliberate non-goals, and any issue-backed remaining work; it must not present partial implementation as complete.

- [ ] **Step 1: Add executable book examples** for arrays of tuple lists, typed empty callable arrays, nested array vector conversion, and a shared typed Rust operation used by native construction and interpreted CEL. Assert values, not exact source spelling.
- [ ] **Step 2: Run** the focused book example tests before updating implementation/example glue; confirm any new example fails for the intended unsupported path.
- [ ] **Step 3: Update** current guidance, historical status notes, module tutorials, public examples, and function contracts. Explain `Array<Array<T>>` versus deep native vectors, unit normalization, explicit capabilities, external checks versus prepared execution, and the absence of the macro backend.
- [ ] **Step 4: Run the required pre-PR gate**, reading all output and requiring zero build/test warnings:

```bash
cargo fmt --all --check
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
cargo clippy -p ez-adam --no-default-features --all-targets -- -D warnings
cargo clippy -p ez-adam --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace
git diff --check
```

The mutation-oriented review must reject missing nested identity checks,
incorrect canonical offsets, accidental clone/equality requirements, and
reintroduced checks in prepared invocation. Use existing sanitizer support
for the changed allocation/drop/layout tests when a configured nightly is
available; do not install tooling merely for this plan.

- [ ] **Step 5: Write the dated handoff**, fix in-scope review findings, and create GitHub issues for genuinely out-of-scope findings rather than silently deferring them. No PR or push happens without the user's request.
- [ ] **Step 6: Format and commit** documentation and final contract tests as `docs: document composable CEL container interoperability`.

## Execution and Review Handoff

This plan has nine reviewable tasks. Tasks 1 and 2 can be implemented
independently; Tasks 3-8 share concrete interfaces and should land in order.
Task 9 closes the full approved contract, not just the tuple-array example.

Recommend **subagent-driven** execution: the allocation/layout and prepared
callable boundaries warrant independent review, and their mistakes can cause
memory unsafety. Keep dispatch bounded to one task at a time after the two
independent foundations; no multi-agent workflow is required.

Before writing implementation code, invoke `algorithm-research` for a focused
pass over standard layout/raw-parts/heterogeneous-list solutions and retain
the existing standard-library approach unless evidence warrants a change.
Do not implement until the user reviews this plan and chooses the execution
method.
