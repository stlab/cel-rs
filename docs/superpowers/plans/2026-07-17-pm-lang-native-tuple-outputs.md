# pm-lang Native Tuple Outputs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace pm-lang's hand-rolled `output_list` grammar (which compiles each
comma-separated output expression into its own independent `DynSegment`) with
`cel-parser`'s native tuple support, so a multi-output method body is one
`or_expression`, evaluated exactly once per `propagate()` call.

**Architecture:** Add a new `DynSegment::call_dyn_tuple` primitive to `cel-runtime`
that runs a segment once and splits its tuple result into per-element
`Box<dyn Any>` values (backed by a new `RawStack::read_at` accessor). Add a matching
`extract_box_fn` to pm-lang's `TypeRegistry`. Rewrite `pm-lang`'s `method_body`
grammar/parsing to compile one `or_expression`, dispatching to either the existing
`call_dyn` path (1 output) or the new `call_dyn_tuple` path (N>1 outputs).

**Tech Stack:** Rust, `anyhow` for error handling, existing `cel-parser`/`cel-runtime`/
`pm-lang` crates in this workspace. No new dependencies.

## Global Constraints

- `cargo fmt --all` must pass before every commit (enforced by the pre-commit hook).
- Every public function needs a contract-style `///` doc comment (Summary /
  Preconditions / `# Errors` / Postconditions / Complexity as applicable) per the
  project's `CLAUDE.md`.
- Precondition violations get `debug_assert!`, not documented consequences — never
  write "causes undefined behavior" in a doc comment; state the precondition and stop.
- Fallible ops return `Result`; arithmetic on signed integers uses `checked_*` (not
  applicable to this change, no new arithmetic is added, but keep in mind for any
  incidental code).
- Unit tests are derived from the contract/public interface only — do not encode
  implementation details into assertions.
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler
  warnings.

---

## File Structure

| File | Change |
| --- | --- |
| `cel-runtime/src/raw_stack.rs` | Add `RawStack::read_at` (read-only sibling of `drop_at`) + test. |
| `cel-runtime/src/dyn_segment.rs` | Add `BoxExtractor` type alias + `DynSegment::call_dyn_tuple` + tests. |
| `pm-lang/src/type_registry.rs` | Add `TypeEntry::extract_box_fn` + `extract_box_impl` + test. |
| `pm-lang/src/parser.rs` | Remove `output_list`/`parse_output_list`; add `CompiledOutputs`; rewrite `parse_method_body`/`build_method`/`parse_method_decl`; add tests. |
| `docs/superpowers/specs/2026-06-28-pm-lang-design.md` | Add a pointer note to the superseding spec (no rewrite of history). |

No new files. No files are deleted.

---

## Task 1: `DynSegment::call_dyn_tuple` (cel-runtime)

**Files:**
- Modify: `cel-runtime/src/raw_stack.rs:142` (after `drop_at`, before `truncate_to`)
- Modify: `cel-runtime/src/raw_stack.rs:395` (test module, after `copy_from_reads_bytes_at_offset`)
- Modify: `cel-runtime/src/dyn_segment.rs:768` (after `call_dyn`, before `op1`)
- Modify: `cel-runtime/src/dyn_segment.rs:1331` (test module, after `call_dyn_errors_if_op_returns_error`)

**Interfaces:**
- Consumes: `RawStack::{buffer, push, drop_at, truncate_to}` (existing, private/pub(crate) to the crate), `RawSegment::{call0_stack, base_alignment}` (existing, `pub`/`pub(crate)`), `DynSegment::{argument_ids, stack_ids, segment}` fields (existing), `AssociatedType`, `StackInfo`, `DynTuple`, `drop_tuple` (existing, same module).
- Produces:
  - `pub unsafe fn RawStack::read_at<R>(&self, offset: usize, read: impl FnOnce(*const u8) -> R) -> R`
  - `pub type BoxExtractor = unsafe fn(*const u8) -> Box<dyn Any>;` (in `cel_runtime::dyn_segment`, re-exported from crate root via the existing `pub use dyn_segment::*;` in `cel-runtime/src/lib.rs` — no `lib.rs` edit needed)
  - `pub fn DynSegment::call_dyn_tuple(&mut self, inputs: &[&dyn Any], extractors: &[(TypeId, BoxExtractor)]) -> anyhow::Result<Vec<Box<dyn Any>>>`

Later tasks (pm-lang's `TypeRegistry`/`parser.rs`) call `call_dyn_tuple` with
`extractors: &Vec<(TypeId, BoxExtractor)>` built from `TypeEntry::extract_box_fn`.

- [ ] **Step 1: Write the failing test for `RawStack::read_at`**

Open `cel-runtime/src/raw_stack.rs`. In the `#[cfg(test)] mod tests` block, insert this
test immediately after `copy_from_reads_bytes_at_offset` (which ends at line 395) and
before `drop_at_runs_destructor_without_changing_length` (which starts at line 397):

```rust
    #[test]
    fn read_at_gives_a_pointer_to_the_value_without_copying() {
        let mut stack = RawStack::with_base_alignment(align_of::<u32>());
        let _ = stack.push(10u32);
        let _ = stack.push(20u32);
        let first: u32 = unsafe { stack.read_at(0, |ptr| *ptr.cast::<u32>()) };
        let second: u32 = unsafe { stack.read_at(4, |ptr| *ptr.cast::<u32>()) };
        assert_eq!(first, 10);
        assert_eq!(second, 20);
    }
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p cel-runtime read_at_gives_a_pointer_to_the_value_without_copying`
Expected: FAIL — `error[E0599]: no method named 'read_at' found for struct 'RawStack'`

- [ ] **Step 3: Implement `RawStack::read_at`**

In `cel-runtime/src/raw_stack.rs`, insert immediately after the closing `}` of
`drop_at` (line 142) and before the doc comment for `truncate_to` (line 144):

```rust

    /// Reads a value at absolute buffer offset `offset`, given a callback that
    /// receives a pointer to its bytes.
    ///
    /// - Precondition: `offset` points to a live, valid, properly-aligned value.
    /// - Precondition: `read` does not retain the pointer beyond the call.
    ///
    /// # Safety
    /// `offset` must point to a live, valid, properly-aligned value for the type the
    /// caller will reinterpret it as; `read` must not retain the pointer beyond the
    /// call.
    pub unsafe fn read_at<R>(&self, offset: usize, read: impl FnOnce(*const u8) -> R) -> R {
        unsafe { read(self.buffer.as_ptr().add(offset).cast::<u8>()) }
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p cel-runtime read_at_gives_a_pointer_to_the_value_without_copying`
Expected: PASS (1 passed)

- [ ] **Step 5: Write the failing tests for `DynSegment::call_dyn_tuple`**

Open `cel-runtime/src/dyn_segment.rs`. In the `#[cfg(test)] mod tests` block, insert
these tests immediately after `call_dyn_errors_if_op_returns_error` (which ends at
line 1331) and before `stack_info_records_size_and_align` (which starts at line 1333):

```rust

    unsafe fn extract_u32(ptr: *const u8) -> Box<dyn Any> {
        unsafe { Box::new(*ptr.cast::<u32>()) }
    }

    unsafe fn extract_str(ptr: *const u8) -> Box<dyn Any> {
        unsafe { Box::new(*ptr.cast::<&'static str>()) }
    }

    #[test]
    fn call_dyn_tuple_splits_result_into_boxed_elements() -> Result<(), anyhow::Error> {
        let mut seg = DynSegment::new::<()>();
        let ambient_start = seg.current_stack_offset();
        seg.op0(|| 10u32);
        seg.op0(|| "hello");
        seg.make_tuple(2, ambient_start);

        let extractors = [
            (TypeId::of::<u32>(), extract_u32 as BoxExtractor),
            (TypeId::of::<&'static str>(), extract_str as BoxExtractor),
        ];
        let results = seg.call_dyn_tuple(&[], &extractors)?;
        assert_eq!(results.len(), 2);
        assert_eq!(*results[0].downcast_ref::<u32>().unwrap(), 10);
        assert_eq!(*results[1].downcast_ref::<&'static str>().unwrap(), "hello");
        Ok(())
    }

    #[test]
    fn call_dyn_tuple_is_repeatable() -> Result<(), anyhow::Error> {
        let mut seg = DynSegment::new::<()>();
        let ambient_start = seg.current_stack_offset();
        seg.op0(|| 1u32);
        seg.op0(|| 2u32);
        seg.make_tuple(2, ambient_start);

        let extractors = [
            (TypeId::of::<u32>(), extract_u32 as BoxExtractor),
            (TypeId::of::<u32>(), extract_u32 as BoxExtractor),
        ];
        let r1 = seg.call_dyn_tuple(&[], &extractors)?;
        let r2 = seg.call_dyn_tuple(&[], &extractors)?;
        assert_eq!(*r1[0].downcast_ref::<u32>().unwrap(), 1);
        assert_eq!(*r2[1].downcast_ref::<u32>().unwrap(), 2);
        Ok(())
    }

    #[test]
    fn call_dyn_tuple_errors_if_result_is_not_a_tuple() {
        let mut seg = DynSegment::new::<()>();
        seg.op0(|| 5u32);
        let extractors = [(TypeId::of::<u32>(), extract_u32 as BoxExtractor)];
        let result = seg.call_dyn_tuple(&[], &extractors);
        assert!(result.is_err(), "expected Err when result is not a tuple");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("tuple"),
            "error message should mention tuple: {msg}"
        );
    }

    #[test]
    fn call_dyn_tuple_errors_on_arity_mismatch() {
        let mut seg = DynSegment::new::<()>();
        let ambient_start = seg.current_stack_offset();
        seg.op0(|| 1u32);
        seg.op0(|| 2u32);
        seg.make_tuple(2, ambient_start);

        let extractors = [(TypeId::of::<u32>(), extract_u32 as BoxExtractor)];
        let result = seg.call_dyn_tuple(&[], &extractors);
        assert!(result.is_err(), "expected Err on arity mismatch");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("2") && msg.contains("1"),
            "error should mention both arities: {msg}"
        );
    }

    #[test]
    fn call_dyn_tuple_errors_on_element_type_mismatch() {
        let mut seg = DynSegment::new::<()>();
        let ambient_start = seg.current_stack_offset();
        seg.op0(|| 1u32);
        seg.op0(|| 2u32);
        seg.make_tuple(2, ambient_start);

        // Element 1 is u32 at runtime, but the extractor table claims &str.
        let extractors = [
            (TypeId::of::<u32>(), extract_u32 as BoxExtractor),
            (TypeId::of::<&'static str>(), extract_str as BoxExtractor),
        ];
        let result = seg.call_dyn_tuple(&[], &extractors);
        assert!(result.is_err(), "expected Err on element type mismatch");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("type mismatch"),
            "error should mention type mismatch: {msg}"
        );
    }

    #[test]
    fn call_dyn_tuple_errors_if_op_returns_error() {
        let mut seg = DynSegment::new::<()>();
        let ambient_start = seg.current_stack_offset();
        seg.op0(|| 1u32);
        seg.op0r(|| -> anyhow::Result<u32> { Err(anyhow::anyhow!("op failed deliberately")) });
        seg.make_tuple(2, ambient_start);

        let extractors = [
            (TypeId::of::<u32>(), extract_u32 as BoxExtractor),
            (TypeId::of::<u32>(), extract_u32 as BoxExtractor),
        ];
        let result = seg.call_dyn_tuple(&[], &extractors);
        assert!(result.is_err(), "expected Err when op fails");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("op failed"),
            "error message should propagate op error: {msg}"
        );
    }
```

- [ ] **Step 6: Run the tests to verify they fail to compile**

Run: `cargo test -p cel-runtime call_dyn_tuple`
Expected: FAIL — `error[E0599]: no method named 'call_dyn_tuple' found for struct 'DynSegment'` (and `BoxExtractor` unresolved)

- [ ] **Step 7: Implement `BoxExtractor` and `DynSegment::call_dyn_tuple`**

In `cel-runtime/src/dyn_segment.rs`, insert immediately after the closing `}` of
`call_dyn` (line 768) and before the doc comment for `op1` (line 770):

```rust

    /// Reads and clones a value of some fixed type from `ptr`, boxing it as
    /// `Box<dyn Any>`.
    ///
    /// # Safety
    /// `ptr` must point to a valid, live, properly aligned value of the type this
    /// function was generated for.
    pub type BoxExtractor = unsafe fn(*const u8) -> Box<dyn Any>;

    /// Executes the segment once and splits its tuple result into one boxed value
    /// per element, using `extractors[i].1` to read element `i`, after checking that
    /// `extractors[i].0` matches element `i`'s runtime type.
    ///
    /// Unlike [`tuple_index`](Self::tuple_index), which permanently specializes a
    /// segment to extract one fixed element at parse time, this runs the segment
    /// exactly once at call time and reads every element from that one evaluation.
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - The segment requires pre-loaded arguments (created with a non-unit `Args` type).
    /// - The stack does not contain exactly one value after expression compilation.
    /// - That value is not a tuple, or its arity does not equal `extractors.len()`.
    /// - Element `i`'s runtime `TypeId` does not equal `extractors[i].0`.
    /// - Any op returns an error during execution.
    ///
    /// - Complexity: O(n) in the number of ops, plus O(extractors.len()) to split the result.
    pub fn call_dyn_tuple(
        &mut self,
        inputs: &[&dyn Any],
        extractors: &[(TypeId, BoxExtractor)],
    ) -> anyhow::Result<Vec<Box<dyn Any>>> {
        ensure!(
            self.argument_ids.is_empty(),
            "call_dyn_tuple: segment requires {} pre-loaded argument(s); \
             use call_dyn_tuple only with push_arg-based segments",
            self.argument_ids.len()
        );
        ensure!(
            self.stack_ids.len() == 1,
            "call_dyn_tuple: expected exactly 1 value on stack, got {}",
            self.stack_ids.len()
        );
        let info = &self.stack_ids[0];
        ensure!(
            info.type_id == TypeId::of::<DynTuple>(),
            "call_dyn_tuple: expected a tuple result, got {}",
            info.type_name,
        );
        ensure!(
            info.associated.len() == extractors.len(),
            "call_dyn_tuple: tuple has {} element(s) but {} extractor(s) were supplied",
            info.associated.len(),
            extractors.len(),
        );
        for (i, (elem, (expected_type_id, _))) in
            info.associated.iter().zip(extractors).enumerate()
        {
            ensure!(
                elem.type_id == *expected_type_id,
                "call_dyn_tuple: element {i} type mismatch: expected type {:?}, got `{}`",
                expected_type_id,
                elem.type_name,
            );
        }

        let tuple_size = info.size;
        let tuple_padding = info.padding;
        let associated = info.associated.clone();

        CALL_DYN_PTR.with(|c| c.set(inputs.as_ptr() as usize));
        CALL_DYN_LEN.with(|c| c.set(inputs.len()));
        let _guard = DynCallGuard;

        let mut stack = RawStack::with_base_alignment(self.segment.base_alignment());
        // Safety: the checks above verified the segment builds exactly one tuple
        // value with `extractors.len()` matching elements; call_dyn's own argument
        // preconditions (no pre-loaded arguments) hold identically here.
        unsafe {
            self.segment.call0_stack(&mut stack)?;
        }

        let tuple_base = stack.len() - tuple_size;
        let results: Vec<Box<dyn Any>> = associated
            .iter()
            .zip(extractors)
            .map(|(elem, (_, extractor))| unsafe {
                stack.read_at(tuple_base + elem.offset, |ptr| extractor(ptr))
            })
            .collect();

        unsafe {
            stack.drop_at(tuple_base, |ptr| drop_tuple(ptr, &associated));
            stack.truncate_to(tuple_base, tuple_padding);
        }

        Ok(results)
    }
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p cel-runtime call_dyn_tuple`
Expected: PASS (6 passed: `call_dyn_tuple_splits_result_into_boxed_elements`,
`call_dyn_tuple_is_repeatable`, `call_dyn_tuple_errors_if_result_is_not_a_tuple`,
`call_dyn_tuple_errors_on_arity_mismatch`, `call_dyn_tuple_errors_on_element_type_mismatch`,
`call_dyn_tuple_errors_if_op_returns_error`)

- [ ] **Step 9: Run the full cel-runtime test suite and lints**

Run: `cargo test -p cel-runtime`
Expected: all tests pass, no new failures.

Run: `cargo clippy -p cel-runtime --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 10: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/raw_stack.rs cel-runtime/src/dyn_segment.rs
git commit -m "$(cat <<'EOF'
feat(cel-runtime): add DynSegment::call_dyn_tuple

Runs a tuple-producing segment once and splits its result into one boxed
value per element, checked against a caller-supplied TypeId per element.
Backed by a new RawStack::read_at accessor. Lets pm-lang evaluate a
multi-output method body once instead of once per output.
EOF
)"
```

---

## Task 2: `TypeRegistry::extract_box_fn` (pm-lang)

**Files:**
- Modify: `pm-lang/src/type_registry.rs`

**Interfaces:**
- Consumes: `cel_runtime::BoxExtractor` (from Task 1).
- Produces: `TypeEntry::extract_box_fn: cel_runtime::BoxExtractor`, populated by both
  `TypeRegistry::register` and `TypeRegistry::register_no_default`. Later tasks
  (`pm-lang/src/parser.rs`) read `entry.extract_box_fn` when compiling a multi-output
  method body.

- [ ] **Step 1: Write the failing test**

Open `pm-lang/src/type_registry.rs`. In the `#[cfg(test)] mod tests` block, insert
this test immediately after `call_dyn_fn_returns_boxed_result` (which ends at line 349)
and before `entry_by_type_id_roundtrip` (which starts at line 351):

```rust

    #[test]
    fn extract_box_fn_reads_and_clones_value() {
        use std::any::Any;

        let reg = TypeRegistry::new();
        let entry = reg.get("i32").unwrap();
        let value: i32 = 42;
        let boxed = unsafe { (entry.extract_box_fn)((&value as *const i32).cast::<u8>()) };
        let result: Box<i32> = boxed.downcast::<i32>().expect("i32");
        assert_eq!(*result, 42);
    }
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p pm-lang extract_box_fn_reads_and_clones_value`
Expected: FAIL — `error[E0609]: no field 'extract_box_fn' on type '&TypeEntry'`

- [ ] **Step 3: Implement `extract_box_fn`**

In `pm-lang/src/type_registry.rs`:

1. Change the import at line 23 from:

```rust
use cel_runtime::DynSegment;
```

to:

```rust
use cel_runtime::{BoxExtractor, DynSegment};
```

2. Add a new field to `TypeEntry`, immediately after `call_dyn_fn` (line 56) and
   before the doc comment for `default_fn` (line 57):

```rust
    /// Reads and clones a `T` from a raw pointer into a type-erased box; used to
    /// split a multi-output method's tuple result into per-cell values.
    pub extract_box_fn: BoxExtractor,
```

3. Add a new free function immediately after `call_dyn_impl` (which ends at line 117):

```rust

/// Reads and clones a `T` from `ptr`, boxing it as `Box<dyn Any>`.
///
/// # Safety
/// `ptr` must point to a valid, live, properly aligned `T`.
unsafe fn extract_box_impl<T: Clone + 'static>(ptr: *const u8) -> Box<dyn Any> {
    Box::new(unsafe { (*ptr.cast::<T>()).clone() })
}
```

4. In `TypeRegistry::register`, add `extract_box_fn: extract_box_impl::<T>,`
   immediately after `call_dyn_fn: call_dyn_impl::<T>,` (line 173):

```rust
                call_dyn_fn: call_dyn_impl::<T>,
                extract_box_fn: extract_box_impl::<T>,
                default_fn: Some(|| Box::new(T::default()) as Box<dyn Any>),
```

5. In `TypeRegistry::register_no_default`, add the same field immediately after
   `call_dyn_fn: call_dyn_impl::<T>,` (line 210):

```rust
                call_dyn_fn: call_dyn_impl::<T>,
                extract_box_fn: extract_box_impl::<T>,
                default_fn: None,
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p pm-lang extract_box_fn_reads_and_clones_value`
Expected: PASS (1 passed)

- [ ] **Step 5: Run the full pm-lang test suite and lints**

Run: `cargo test -p pm-lang`
Expected: all tests pass, no new failures (existing tests are unaffected by this
additive change).

Run: `cargo clippy -p pm-lang --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add pm-lang/src/type_registry.rs
git commit -m "$(cat <<'EOF'
feat(pm-lang): add TypeRegistry::extract_box_fn

Mirrors call_dyn_fn but reads a value from a raw pointer instead of
running a segment, for splitting a multi-output method's tuple result.
EOF
)"
```

---

## Task 3: Native tuple outputs in pm-lang's parser

**Files:**
- Modify: `pm-lang/src/parser.rs`

**Interfaces:**
- Consumes: `cel_runtime::{BoxExtractor, DynSegment}` (`DynSegment::{peek_tuple_arity, peek_stack_infos, peek_output_type_id, call_dyn_tuple}` from Task 1), `crate::type_registry::{CallDynFn, TypeEntry}` (`TypeEntry::extract_box_fn` from Task 2).
- Produces: updated `PmParser::parse_str` behavior — `method_body = "{" or_expression "}"` — consumed by `PmParser`'s existing public API (no signature change to `parse_str` itself).

- [ ] **Step 1: Write the failing tests**

Open `pm-lang/src/parser.rs`. In the `#[cfg(test)] mod tests` block, insert these
tests immediately after `parse_relationship_multi_output_tuple` (which ends at
line 1024) and before `parse_conditional_decl` (which starts at line 1026):

```rust

    #[test]
    fn parse_method_output_tuple_arity_mismatch_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: i32 = 1;
                cell b: i32 = 2;
                cell x: i32;
                cell y: i32;
                cell z: i32;
                relationship { method [a, b] -> [x, y, z] { (a + b, a - b) } }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "2-tuple body for 3 declared outputs must be an error"
        );
        let err = result.err().expect("expected Err");
        let msg = err.message().to_lowercase();
        assert!(msg.contains("arity"), "{msg}");
    }

    #[test]
    fn parse_method_output_tuple_element_type_mismatch_is_error() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell a: i32 = 1;
                cell b: f64 = 2.0;
                cell x: i32;
                cell y: i32;
                relationship { method [a, b] -> [x, y] { (a, b) } }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "f64 tuple element for an i32 output must be an error"
        );
        let err = result.err().expect("expected Err");
        let msg = err.message().to_lowercase();
        assert!(msg.contains("type mismatch"), "{msg}");
    }

    #[test]
    fn parse_method_single_output_rejects_tuple_body() {
        let result = parser().parse_str(
            r#"
            sheet s {
                cell x: i32 = 1;
                cell y: i32;
                relationship { method [x] -> [y] { (x,) } }
            }
        "#,
        );
        assert!(
            result.is_err(),
            "1-tuple body for a single declared output must be an error"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pm-lang parse_method_output_tuple parse_method_single_output_rejects_tuple_body`
Expected: FAIL — `parse_method_output_tuple_arity_mismatch_is_error` panics with
"expected Err for..." false (the old `output_list` grammar rejects
`(a + b, a - b)` against 3 outputs as an arity mismatch already via
`segments.len() != outputs.len()`, so this specific one may already pass — the
important new failures are `parse_method_output_tuple_element_type_mismatch_is_error`
and `parse_method_single_output_rejects_tuple_body`, which currently fail to parse
at all under the old grammar since `(a, b)` and `(x,)` aren't valid `output_list`
syntax without `parse_output_list`'s own splitting, or hit different error text).
Confirm each test's actual current outcome by running them individually before
proceeding — the point of this step is to have a documented red baseline, not a
specific error string.

- [ ] **Step 3: Rewrite the grammar comment**

In `pm-lang/src/parser.rs`, replace lines 14–16:

```rust
//! method_body = "{" output_list "}".
//! output_list = "(" or_expression "," or_expression { "," or_expression } ")"
//!             | or_expression.
```

with:

```rust
//! method_body = "{" or_expression "}".
```

- [ ] **Step 4: Update the import**

Change line 31 from:

```rust
use cel_runtime::DynSegment;
```

to:

```rust
use cel_runtime::{BoxExtractor, DynSegment};
```

- [ ] **Step 5: Update `parse_str`'s doc comment**

Change line 353 from:

```rust
    /// or arity mismatch in an `output_list` tuple.
```

to:

```rust
    /// or a tuple arity/element-type mismatch between the output expression and the
    /// method's declared outputs.
```

- [ ] **Step 6: Add the `CompiledOutputs` enum**

Insert immediately before `build_method` (the free function starting at line 855, doc
comment `/// Builds a [`Method`] from parsed inputs, outputs, compiled segments, and
call_dyn functions.`):

```rust
/// How to turn one compiled `or_expression`'s result into per-output values.
enum CompiledOutputs {
    /// One output: the segment's single result, boxed via `call_dyn`.
    Single(CallDynFn),
    /// N > 1 outputs: the segment's tuple result, split via `call_dyn_tuple`. Each
    /// entry pairs an output's expected `TypeId` with the extractor that reads it.
    Tuple(Vec<(TypeId, BoxExtractor)>),
}

```

- [ ] **Step 7: Replace `parse_method_decl`**

Replace lines 544–554 (the whole `parse_method_decl` function):

```rust
    /// `method_decl = "method" cell_list "->" cell_list method_body.`
    fn parse_method_decl(&mut self, ctx: &mut ParseContext) -> Result<Method> {
        if !ctx.is_keyword("method") {
            return Err(ctx.err_at("expected `method`"));
        }
        let inputs = self.parse_cell_list(ctx)?;
        ctx.expect_punct("->")?;
        let outputs = self.parse_cell_list(ctx)?;
        let (segments, call_fns) = self.parse_method_body(ctx, &inputs, &outputs)?;
        Ok(build_method(inputs, outputs, segments, call_fns))
    }
```

with:

```rust
    /// `method_decl = "method" cell_list "->" cell_list method_body.`
    fn parse_method_decl(&mut self, ctx: &mut ParseContext) -> Result<Method> {
        if !ctx.is_keyword("method") {
            return Err(ctx.err_at("expected `method`"));
        }
        let inputs = self.parse_cell_list(ctx)?;
        ctx.expect_punct("->")?;
        let outputs = self.parse_cell_list(ctx)?;
        let (segment, compiled) = self.parse_method_body(ctx, &inputs, &outputs)?;
        Ok(build_method(inputs, outputs, segment, compiled))
    }
```

- [ ] **Step 8: Replace `parse_method_body`**

Replace lines 576–665 (the whole `parse_method_body` function, from its doc comment
through its closing `}`):

```rust
    /// `method_body = "{" output_list "}".`
    ///
    /// Returns `(segments, call_dyn_fns)` — one segment and one `call_dyn_fn` per output.
    fn parse_method_body(
        &mut self,
        ctx: &mut ParseContext,
        inputs: &[(String, CellId, TypeId)],
        outputs: &[(String, CellId, TypeId)],
    ) -> Result<(Vec<DynSegment>, Vec<CallDynFn>)> {
        ctx.expect_open_brace()?;

        // Pre-compute push_arg dispatch table for input scope.
        let scope_data: Vec<(String, PushArgFn, usize)> = inputs
            .iter()
            .enumerate()
            .map(|(idx, (name, _, type_id))| {
                let fn_ptr = self
                    .types
                    .entry_by_type_id(*type_id)
                    .expect("input cell type registered")
                    .push_arg_fn;
                (name.clone(), fn_ptr, idx)
            })
            .collect();

        // Push scope: CELParser resolves input cell names to push_arg ops.
        self.cel
            .op_lookup_mut()
            .push_scope(move |name, segment, arity, _span| {
                if arity != 0 {
                    return Ok(false);
                }
                for (n, fn_ptr, idx) in &scope_data {
                    if n == name {
                        fn_ptr(segment, *idx);
                        return Ok(true);
                    }
                }
                Ok(false)
            });

        let result = self.parse_output_list(ctx);
        self.cel.op_lookup_mut().pop_scope();
        let segments = result?;

        ctx.expect_close_brace()?;

        if segments.len() != outputs.len() {
            return Err(ctx.err_at(format!(
                "output list has {} expression(s) but method declares {} output(s)",
                segments.len(),
                outputs.len()
            )));
        }

        // Verify output types and collect call_dyn_fn per output.
        let mut call_fns = Vec::with_capacity(outputs.len());
        for (i, (seg, (out_name, _, out_type_id))) in
            segments.iter().zip(outputs.iter()).enumerate()
        {
            let actual_type_id = seg.peek_output_type_id().ok_or_else(|| {
                ctx.err_at(format!(
                    "output {i} `{out_name}`: expression produced no value"
                ))
            })?;
            if actual_type_id != *out_type_id {
                let expected = self
                    .types
                    .entry_by_type_id(*out_type_id)
                    .map(|e| e.type_name)
                    .unwrap_or("?");
                let got = self
                    .types
                    .entry_by_type_id(actual_type_id)
                    .map(|e| e.type_name)
                    .unwrap_or("?");
                return Err(ctx.err_at(format!(
                    "output {i} `{out_name}`: type mismatch: expected `{expected}`, got `{got}`"
                )));
            }
            let call_fn = self
                .types
                .entry_by_type_id(*out_type_id)
                .expect("output cell type registered")
                .call_dyn_fn;
            call_fns.push(call_fn);
        }

        Ok((segments, call_fns))
    }

    /// `output_list = "(" or_expression "," or_expression { "," or_expression } ")" | or_expression.`
    fn parse_output_list(&mut self, ctx: &mut ParseContext) -> Result<Vec<DynSegment>> {
        if ctx.consume_open_paren() {
            let seg1 = self.parse_cel_or_expression(ctx)?;
            if ctx.peek_close_paren() {
                ctx.advance(); // parenthesized single expression — not a tuple
                return Ok(vec![seg1]);
            }
            ctx.expect_punct(",")?;
            let mut segs = vec![seg1];
            loop {
                segs.push(self.parse_cel_or_expression(ctx)?);
                if ctx.peek_close_paren() {
                    ctx.advance();
                    break;
                }
                ctx.expect_punct(",")?;
            }
            Ok(segs)
        } else {
            Ok(vec![self.parse_cel_or_expression(ctx)?])
        }
    }
```

with:

```rust
    /// `method_body = "{" or_expression "}".`
    ///
    /// Returns the compiled body segment and how to split its result across `outputs`:
    /// one output takes the segment's single result directly; more than one requires
    /// the result to be a tuple of matching arity and element types, split via
    /// `call_dyn_tuple`.
    fn parse_method_body(
        &mut self,
        ctx: &mut ParseContext,
        inputs: &[(String, CellId, TypeId)],
        outputs: &[(String, CellId, TypeId)],
    ) -> Result<(DynSegment, CompiledOutputs)> {
        ctx.expect_open_brace()?;

        // Pre-compute push_arg dispatch table for input scope.
        let scope_data: Vec<(String, PushArgFn, usize)> = inputs
            .iter()
            .enumerate()
            .map(|(idx, (name, _, type_id))| {
                let fn_ptr = self
                    .types
                    .entry_by_type_id(*type_id)
                    .expect("input cell type registered")
                    .push_arg_fn;
                (name.clone(), fn_ptr, idx)
            })
            .collect();

        // Push scope: CELParser resolves input cell names to push_arg ops.
        self.cel
            .op_lookup_mut()
            .push_scope(move |name, segment, arity, _span| {
                if arity != 0 {
                    return Ok(false);
                }
                for (n, fn_ptr, idx) in &scope_data {
                    if n == name {
                        fn_ptr(segment, *idx);
                        return Ok(true);
                    }
                }
                Ok(false)
            });

        let result = self.parse_cel_or_expression(ctx);
        self.cel.op_lookup_mut().pop_scope();
        let segment = result?;

        ctx.expect_close_brace()?;

        let compiled = if outputs.len() == 1 {
            let (out_name, _, out_type_id) = &outputs[0];
            let actual_type_id = segment.peek_output_type_id().ok_or_else(|| {
                ctx.err_at(format!("output `{out_name}`: expression produced no value"))
            })?;
            if actual_type_id != *out_type_id {
                let expected = self
                    .types
                    .entry_by_type_id(*out_type_id)
                    .map(|e| e.type_name)
                    .unwrap_or("?");
                let got = self
                    .types
                    .entry_by_type_id(actual_type_id)
                    .map(|e| e.type_name)
                    .unwrap_or("?");
                return Err(ctx.err_at(format!(
                    "output `{out_name}`: type mismatch: expected `{expected}`, got `{got}`"
                )));
            }
            let call_fn = self
                .types
                .entry_by_type_id(*out_type_id)
                .expect("output cell type registered")
                .call_dyn_fn;
            CompiledOutputs::Single(call_fn)
        } else {
            let arity = segment.peek_tuple_arity().unwrap_or(0);
            if arity != outputs.len() {
                return Err(ctx.err_at(format!(
                    "output expression has arity {arity} but method declares {} output(s)",
                    outputs.len()
                )));
            }
            let element_type_ids: Vec<TypeId> = segment.peek_stack_infos(1)[0]
                .associated
                .iter()
                .map(|a| a.type_id)
                .collect();

            let mut extractors = Vec::with_capacity(outputs.len());
            for (i, ((out_name, _, out_type_id), actual_type_id)) in
                outputs.iter().zip(&element_type_ids).enumerate()
            {
                if actual_type_id != out_type_id {
                    let expected = self
                        .types
                        .entry_by_type_id(*out_type_id)
                        .map(|e| e.type_name)
                        .unwrap_or("?");
                    let got = self
                        .types
                        .entry_by_type_id(*actual_type_id)
                        .map(|e| e.type_name)
                        .unwrap_or("?");
                    return Err(ctx.err_at(format!(
                        "output {i} `{out_name}`: type mismatch: expected `{expected}`, got `{got}`"
                    )));
                }
                let entry = self
                    .types
                    .entry_by_type_id(*out_type_id)
                    .expect("output cell type registered");
                extractors.push((*out_type_id, entry.extract_box_fn));
            }
            CompiledOutputs::Tuple(extractors)
        };

        Ok((segment, compiled))
    }
```

- [ ] **Step 9: Replace `build_method`**

Replace lines 855–882 (the whole `build_method` function):

```rust
/// Builds a [`Method`] from parsed inputs, outputs, compiled segments, and call_dyn functions.
fn build_method(
    inputs: Vec<(String, CellId, TypeId)>,
    outputs: Vec<(String, CellId, TypeId)>,
    segments: Vec<DynSegment>,
    call_fns: Vec<CallDynFn>,
) -> Method {
    let input_ids: Vec<CellId> = inputs.iter().map(|(_, id, _)| *id).collect();
    let output_ids: Vec<CellId> = outputs.iter().map(|(_, id, _)| *id).collect();
    let input_types: Vec<TypeId> = inputs.iter().map(|(_, _, tid)| *tid).collect();
    let output_types: Vec<TypeId> = outputs.iter().map(|(_, _, tid)| *tid).collect();

    // Wrap each segment in RefCell: MethodFn is Fn (not FnMut), so interior mutability
    // is required to call call_dyn(&mut self) from an immutable closure reference.
    let cells: Vec<RefCell<DynSegment>> = segments.into_iter().map(RefCell::new).collect();

    let f =
        move |inputs_any: &[&dyn Any]| -> std::result::Result<Vec<Box<dyn Any>>, anyhow::Error> {
            let mut results = Vec::with_capacity(cells.len());
            for (cell, call_fn) in cells.iter().zip(call_fns.iter()) {
                let seg = &mut *cell.borrow_mut();
                results.push(call_fn(seg, inputs_any)?);
            }
            Ok(results)
        };

    Method::new(input_ids, output_ids, input_types, output_types, f)
}
```

with:

```rust
/// Builds a [`Method`] from parsed inputs, outputs, the compiled body segment, and how
/// to split its result across `outputs`.
fn build_method(
    inputs: Vec<(String, CellId, TypeId)>,
    outputs: Vec<(String, CellId, TypeId)>,
    segment: DynSegment,
    compiled: CompiledOutputs,
) -> Method {
    let input_ids: Vec<CellId> = inputs.iter().map(|(_, id, _)| *id).collect();
    let output_ids: Vec<CellId> = outputs.iter().map(|(_, id, _)| *id).collect();
    let input_types: Vec<TypeId> = inputs.iter().map(|(_, _, tid)| *tid).collect();
    let output_types: Vec<TypeId> = outputs.iter().map(|(_, _, tid)| *tid).collect();

    // Wrap in RefCell: MethodFn is Fn (not FnMut), so interior mutability is required
    // to call call_dyn/call_dyn_tuple(&mut self) from an immutable closure reference.
    let segment = RefCell::new(segment);

    let f =
        move |inputs_any: &[&dyn Any]| -> std::result::Result<Vec<Box<dyn Any>>, anyhow::Error> {
            let seg = &mut *segment.borrow_mut();
            match &compiled {
                CompiledOutputs::Single(call_fn) => Ok(vec![call_fn(seg, inputs_any)?]),
                CompiledOutputs::Tuple(extractors) => seg.call_dyn_tuple(inputs_any, extractors),
            }
        };

    Method::new(input_ids, output_ids, input_types, output_types, f)
}
```

- [ ] **Step 10: Run the new tests and confirm they pass**

Run: `cargo test -p pm-lang parse_method_output_tuple parse_method_single_output_rejects_tuple_body`
Expected: PASS (3 passed)

- [ ] **Step 11: Run the full pm-lang test suite**

Run: `cargo test -p pm-lang`
Expected: all tests pass, including the unchanged
`parse_relationship_multi_output_tuple` and `parse_and_propagate_sheet` tests (the
latter exercises `Sheet::propagate()` end-to-end through the single-output
`CompiledOutputs::Single` path).

- [ ] **Step 12: Run lints**

Run: `cargo clippy -p pm-lang --all-targets -- -D warnings`
Expected: no warnings.

Run: `cargo clippy -p cel-runtime -p cel-parser --all-targets -- -D warnings`
Expected: no warnings (confirms Task 1's changes are still clean after pm-lang now
exercises them through a second call site).

- [ ] **Step 13: Commit**

```bash
cargo fmt --all
git add pm-lang/src/parser.rs
git commit -m "$(cat <<'EOF'
feat(pm-lang): replace output_list with native tuple method bodies

method_body is now a single or_expression, evaluated once per propagate()
regardless of output count. cel-parser's tuple_or_group grammar replaces
pm-lang's own comma-split parsing for multi-output methods; arity and
per-element type checks move from segment count to DynSegment::peek_tuple_arity
plus each element's TypeId.
EOF
)"
```

---

## Task 4: Documentation and full verification

**Files:**
- Modify: `docs/superpowers/specs/2026-06-28-pm-lang-design.md`

**Interfaces:** None (documentation-only change plus verification of Tasks 1–3).

- [ ] **Step 1: Add a pointer note to the superseded spec**

Open `docs/superpowers/specs/2026-06-28-pm-lang-design.md`. Immediately after the
`**Status:** Approved` line near the top (line 5), insert:

```markdown

> **Superseded in part:** the `output_list` grammar and "Multi-output tuple
> methods" section below describe pm-lang's original hand-rolled tuple splitting,
> written before `cel-parser` had native tuple support. See
> `docs/superpowers/specs/2026-07-17-pm-lang-native-tuple-outputs-design.md` for the
> current design.
```

- [ ] **Step 2: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: all tests pass, zero compiler warnings in the output.

Run: `cargo test --doc --workspace`
Expected: all doctests pass.

- [ ] **Step 3: Run the full workspace build**

Run: `cargo build --workspace`
Expected: builds cleanly, zero compiler warnings.

- [ ] **Step 4: Run the full clippy suite**

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Expected: no warnings.

Run: `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
Expected: no warnings (this change does not touch `begin`, but the project's
pre-PR checklist requires confirming it stays clean).

Run: `cargo clippy -p begin --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Format**

Run: `cargo fmt --all`
Expected: no diff (everything already formatted from prior task commits).

- [ ] **Step 6: Commit**

```bash
git add docs/superpowers/specs/2026-06-28-pm-lang-design.md
git commit -m "$(cat <<'EOF'
docs: point the original pm-lang spec at the native-tuple-outputs design

The output_list grammar and multi-output tuple section it describes are
superseded now that cel-parser has native tuple support.
EOF
)"
```

---

## Self-Review Notes

- **Spec coverage:** grammar change (Task 3, Step 3), `cel-runtime` primitive (Task 1),
  `TypeRegistry` addition (Task 2), parser/`build_method` rewrite (Task 3),
  compatibility of existing multi-output syntax (Task 3, Step 11 — the unchanged
  `parse_relationship_multi_output_tuple` test), new arity/type-mismatch/single-output
  tests (Task 3, Steps 1 and 10), doc pointer note (Task 4, Step 1). All spec sections
  are covered.
- **Placeholder scan:** no TBDs; every step has complete, runnable code and an exact
  file/line anchor.
- **Type consistency:** `CompiledOutputs::Tuple` carries `Vec<(TypeId, BoxExtractor)>`
  consistently from its definition (Task 3, Step 6) through `parse_method_body`'s
  construction (Task 3, Step 8) and `build_method`'s consumption (Task 3, Step 9),
  matching `DynSegment::call_dyn_tuple`'s parameter type defined in Task 1, Step 7.
  `TypeEntry::extract_box_fn: BoxExtractor` (Task 2) is read directly into that tuple
  in Task 3, Step 8.
