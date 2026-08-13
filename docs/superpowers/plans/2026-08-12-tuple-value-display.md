# Tuple Value Display Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `cel_runtime::DynamicSequence` a real `Debug` impl (matching Rust's own tuple `Debug`
formatting exactly), thread the new per-element `ElementDebug` capability through `cel-runtime` and
`adam-lang`'s `TypeRegistry`, and make `begin`'s Inspector display any tuple-typed cell's value
using it — closing the "tuple cells are invisible in the sidebar" gap. Editing tuple-typed cells
stays out of scope (tracked as a follow-up GitHub issue opened in the final task).

**Architecture:** `SequenceElement`/`DynElementSpec` (the per-element descriptors every
`DynamicSequence` already carries for `Drop`/`Clone`/`PartialEq`) gain a fourth function pointer,
`ElementDebug`, following the exact existing pattern. `DynamicSequence`'s current placeholder
`Debug` impl (`DynamicSequence { arity: 2 }`) is replaced with a hand-written one that iterates
elements and calls each one's stored `debug` fn, formatting exactly like a real Rust anonymous
tuple (`()`, `(3,)`, `(3, 4.5)`) — a nested tuple element is itself a `DynamicSequence` value, so
this recurses for free. `adam-lang`'s `TypeRegistry` threads the same capability through
registration (`TypeEntry.element_debug`) and its tuple-construction helpers
(`element_descriptor`/`element_descriptors_for`/`default_dyn_element`), and `parser.rs`'s
`CompiledOutputs`/`DynExtractor` plumbing that builds a tuple `out` cell's live value. `begin`'s
`Labels` gains a `add_tuple_cell` method so `labels_from_cell_names` stops skipping
`TypeShape::Tuple` cells.

**Tech Stack:** Rust, `cel-runtime`, `adam-lang` (`TypeRegistry`, `parser.rs`), `begin` (Dioxus 0.7,
`bridge.rs`/`inspector.rs`).

**Reference:** `docs/superpowers/specs/2026-08-12-tuple-value-display-design.md`.

## Global Constraints

- Format with `cargo fmt --all` before every commit (enforced by pre-commit hook).
- Every function/trait/struct needs a contract-style `///` doc comment (Summary, Preconditions as
  `debug_assert!`, `# Errors`/`# Safety` where applicable, Postconditions, Complexity if not O(1))
  per the root `CLAUDE.md`.
- Unit tests are derived from contract/public interface only — never from implementation
  internals.
- `cargo build`/`cargo test --workspace` must produce zero compiler warnings; run
  `cargo test --workspace`, `cargo test --doc --workspace`, and all three `cargo clippy`
  invocations from the root `CLAUDE.md` before the final commit of the whole plan (Task 6).
- Adding `Debug` as a hard bound on `TypeRegistry::register`/`register_no_default` (alongside the
  `Clone + PartialEq` they already require) is an accepted, deliberate API change — every built-in
  primitive already satisfies it; see the design doc's confirmed decision.
- Any tuple-typed cell (out or plain) gets a display-only Inspector entry — no output-cell-identity
  plumbing; see the design doc's confirmed scope decision.
- Per `begin/CLAUDE.md`: a UI change to `begin` is not complete until actually rendered and looked
  at via the `verifying-begin-ui` skill — passing `cargo build`/`clippy` proves nothing about what
  renders.

---

### Task 1: `cel-runtime` — `ElementDebug` primitive and a real `Debug` for `DynamicSequence`

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`

**Interfaces:**
- Produces: `pub type ElementDebug = unsafe fn(*const u8, &mut std::fmt::Formatter<'_>) -> std::fmt::Result;`;
  `pub fn element_debug_for<T: 'static + std::fmt::Debug>() -> ElementDebug`; `SequenceElement.debug: ElementDebug`;
  `DynElementSpec.debug: ElementDebug`; a real `impl std::fmt::Debug for DynamicSequence` replacing
  the current placeholder.

This task is confined to `dynamic_sequence.rs` — it does not touch `dyn_segment.rs` (Task 2's job).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dynamic_sequence.rs` (right after
`element_writer_for_moves_a_boxed_value_without_dropping_it`):

```rust
#[test]
fn element_debug_for_formats_the_correct_type() {
    struct Wrapper(*const u8, ElementDebug);
    impl std::fmt::Debug for Wrapper {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            unsafe { (self.1)(self.0, f) }
        }
    }
    let value = 7i32;
    let debug = element_debug_for::<i32>();
    let wrapper = Wrapper((&raw const value).cast::<u8>(), debug);
    assert_eq!(format!("{wrapper:?}"), "7");
}

#[test]
fn debug_formats_the_empty_tuple() {
    let seq = DynamicSequence::from_tuple(());
    assert_eq!(format!("{seq:?}"), "()");
}

#[test]
fn debug_formats_a_1_tuple_with_a_trailing_comma() {
    let seq = DynamicSequence::from_tuple((3i32,));
    assert_eq!(format!("{seq:?}"), "(3,)");
}

#[test]
fn debug_formats_an_n_tuple_without_a_trailing_comma() {
    let seq = DynamicSequence::from_tuple((3i32, 4.5f64));
    assert_eq!(format!("{seq:?}"), "(3, 4.5)");
}

#[test]
fn debug_quotes_a_string_element_like_rust_debug_does() {
    let seq = DynamicSequence::from_tuple((1i32, "hello".to_string()));
    assert_eq!(format!("{seq:?}"), "(1, \"hello\")");
}

#[test]
fn debug_recurses_into_a_nested_tuple() {
    let seq = DynamicSequence::from_tuple((1i32, (2.5f64, "x".to_string())));
    assert_eq!(format!("{seq:?}"), "(1, (2.5, \"x\"))");
}

#[test]
fn debug_formats_a_sequence_built_via_from_dyn_elements() {
    let spec_i32 = DynElementSpec {
        type_id: TypeId::of::<i32>(),
        type_name: Cow::Borrowed("i32"),
        size: size_of::<i32>(),
        align: align_of::<i32>(),
        drop: element_dropper_for::<i32>(),
        clone: element_cloner_for::<i32>(),
        eq: element_eq_for::<i32>(),
        debug: element_debug_for::<i32>(),
        write: element_writer_for::<i32>(),
    };
    let seq = DynamicSequence::from_dyn_elements(vec![(
        spec_i32,
        Box::new(3i32) as Box<dyn std::any::Any>,
    )]);
    assert_eq!(format!("{seq:?}"), "(3,)");
}
```

`from_dyn_elements_builds_a_matching_sequence` and
`from_dyn_elements_moves_boxed_values_without_double_dropping` (already in this file) construct
`DynElementSpec` literals that will no longer compile once `DynElementSpec` gains a `debug` field
in Step 3 — update both to add `debug: element_debug_for::<i32>()`/`debug:
element_debug_for::<f64>()`/`debug: element_debug_for::<DropCounter>()` respectively (matching each
literal's own type), and add `#[derive(Debug)]` to `from_dyn_elements_moves_boxed_values_without_double_dropping`'s
local `DropCounter` struct (it currently only derives `Clone` and hand-impls `PartialEq`/`Drop`; it
needs `Debug` too now that `element_debug_for::<DropCounter>()` requires it).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime --lib dynamic_sequence::tests::`
Expected: FAIL to compile — `ElementDebug`/`element_debug_for` don't exist yet, `DynElementSpec`
has no `debug` field yet.

- [ ] **Step 3: Add the primitive, the struct fields, and bound propagation**

In `cel-runtime/src/dynamic_sequence.rs`, right after `pub type ElementEq = ...;`:

```rust
/// Debug-formats a value in place, given a pointer to its bytes.
///
/// # Safety
/// `ptr` must point to a valid, live, properly aligned value of the type this formatter was
/// generated for.
pub type ElementDebug = unsafe fn(*const u8, &mut std::fmt::Formatter<'_>) -> std::fmt::Result;
```

Right after `pub fn element_eq_for<T: 'static + PartialEq>() -> ElementEq { ... }`:

```rust
/// Returns an [`ElementDebug`] that debug-formats a value of type `T` in place.
pub fn element_debug_for<T: 'static + std::fmt::Debug>() -> ElementDebug {
    |ptr, f| unsafe { std::fmt::Debug::fmt(&*ptr.cast::<T>(), f) }
}
```

Add a `pub debug: ElementDebug` field to `SequenceElement`, right after its existing `pub eq:
ElementEq` field (with a doc comment: `/// Debug-formatter for this element, callable at its own
start address.`).

Add a `pub debug: ElementDebug` field to `DynElementSpec`, right after its existing `pub eq:
ElementEq` field (doc comment: `/// Debug-formatter for this element.`).

Update `push_element`'s signature and body:

```rust
fn push_element<T: 'static + Clone + PartialEq + std::fmt::Debug>(
    out: &mut Vec<SequenceElement>,
    offset: usize,
    max_align: &mut usize,
) -> usize {
    use std::mem::{align_of, size_of};

    let align = align_of::<T>();
    let aligned_offset = align_index(align, offset);
    *max_align = (*max_align).max(align);
    out.push(SequenceElement {
        type_id: TypeId::of::<T>(),
        type_name: Cow::Borrowed(std::any::type_name::<T>()),
        offset: aligned_offset,
        size: size_of::<T>(),
        align,
        drop: element_dropper_for::<T>(),
        clone: element_cloner_for::<T>(),
        eq: element_eq_for::<T>(),
        debug: element_debug_for::<T>(),
    });
    aligned_offset + size_of::<T>()
}
```

Update the `SequenceList` impl for `(H, T)`'s generic bound (its body is unchanged):

```rust
impl<H: 'static + Clone + PartialEq + std::fmt::Debug, T: SequenceList> SequenceList for (H, T) {
```

Update every one of the twelve `TupleSequence` arity impls (`(A,)` through the 12-tuple) the same
way: append `+ std::fmt::Debug` immediately after each type parameter's existing `+ PartialEq`
bound. For example, the 1-tuple impl:

```rust
impl<A: 'static + Clone + PartialEq + std::fmt::Debug> TupleSequence for (A,) {
    fn from_list(list: Self::Output) -> Self {
        let (a, ()) = list;
        (a,)
    }
}
```

None of these twelve impls' *bodies* change — only each `impl<...>` line's bound list. Apply the
identical edit (`+ PartialEq` → `+ PartialEq + std::fmt::Debug` on every type parameter) to the
2-tuple through 12-tuple impls that follow. Run `cargo build -p cel-runtime --lib --tests` after
editing all twelve and fix any impl you missed — a missing bound shows up as a clear "the trait
bound `X: Debug` is not satisfied" compile error naming the exact impl block.

Update `DynamicSequence::from_dyn_elements`'s per-element `SequenceElement` construction to copy
the new field, right after its existing `eq: spec.eq,` line: `debug: spec.debug,`.

- [ ] **Step 4: Replace the placeholder `Debug` impl**

Replace the existing placeholder in `cel-runtime/src/dynamic_sequence.rs`:

```rust
impl std::fmt::Debug for DynamicSequence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynamicSequence")
            .field("arity", &self.shape.len())
            .finish()
    }
}
```

with:

```rust
/// Formats this sequence exactly like Rust's own anonymous-tuple `Debug`: `()` for arity 0,
/// `(a,)` for arity 1 (the trailing comma disambiguates a real 1-tuple from mere grouping,
/// matching `format!("{:?}", (3,))`), `(a, b, ...)` otherwise. A nested tuple element (itself a
/// `DynamicSequence` value) recurses through this same impl.
///
/// Not implementable via `f.debug_tuple()`: that builder is for tuple *structs*, which never
/// need the arity-1 trailing comma (`Foo(3)` is never ambiguous), so it does not reproduce real
/// anonymous-tuple formatting.
impl std::fmt::Debug for DynamicSequence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(")?;
        for (i, elem) in self.shape.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            unsafe {
                self.buffer.read_at(elem.offset, |ptr| (elem.debug)(ptr, f))?;
            }
        }
        if self.shape.len() == 1 {
            f.write_str(",")?;
        }
        f.write_str(")")
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p cel-runtime --lib dynamic_sequence::`
Expected: PASS (every pre-existing test in this file plus the 7 new ones).

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dynamic_sequence.rs
git commit -m "feat(cel-runtime): add ElementDebug and a real Debug impl for DynamicSequence"
```

---

### Task 2: `cel-runtime` — thread `ElementDebug` through `dyn_segment.rs`'s tuple-construction machinery

**Files:**
- Modify: `cel-runtime/src/dyn_segment.rs`

**Interfaces:**
- Consumes: `ElementDebug`, `element_debug_for` (Task 1).
- Produces: `DynSegment::call_dyn_as_dynamic_sequence`'s `leaf` parameter type widens to
  `Fn(TypeId) -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)>`;
  `DynExtractor::Tuple`'s closure type widens identically.

`build_dynamic_sequence` and `validate_associated_shape` are the two private helpers shared by
both `call_dyn_as_dynamic_sequence` and `call_dyn_tuple_mixed`'s `DynExtractor::Tuple` handling —
updating their shared `leaf` parameter type covers both public entry points in one edit.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dyn_segment.rs`, right after
`call_dyn_as_dynamic_sequence_builds_a_flat_sequence`:

```rust
#[test]
fn call_dyn_as_dynamic_sequence_result_debug_formats_correctly() -> anyhow::Result<()> {
    let mut seg = DynSegment::new::<()>();
    let ambient_start = seg.current_stack_offset();
    seg.op0(|| 10i32);
    seg.op0(|| 2.5f64);
    seg.make_tuple(2, ambient_start);

    let leaf = |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> {
        if type_id == TypeId::of::<i32>() {
            Some((
                element_dropper_for::<i32>(),
                element_cloner_for::<i32>(),
                element_eq_for::<i32>(),
                element_debug_for::<i32>(),
            ))
        } else if type_id == TypeId::of::<f64>() {
            Some((
                element_dropper_for::<f64>(),
                element_cloner_for::<f64>(),
                element_eq_for::<f64>(),
                element_debug_for::<f64>(),
            ))
        } else {
            None
        }
    };
    let seq = seg.call_dyn_as_dynamic_sequence(&[], &leaf)?;
    assert_eq!(format!("{seq:?}"), "(10, 2.5)");
    Ok(())
}
```

Add to the `tests` module, right after `call_dyn_tuple_mixed_splits_a_tuple_output_among_scalar_and_tuple_slots`:

```rust
#[test]
fn call_dyn_tuple_mixed_tuple_slot_result_debug_formats_correctly() -> anyhow::Result<()> {
    let mut seg = DynSegment::new::<()>();
    let ambient_start = seg.current_stack_offset();
    seg.op0(|| 1i32);
    let inner_start = seg.current_stack_offset();
    seg.op0(|| 2i32);
    seg.op0(|| 3i32);
    seg.make_tuple(2, inner_start);
    seg.make_tuple(2, ambient_start);

    let leaf = |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> {
        (type_id == TypeId::of::<i32>()).then(|| {
            (
                element_dropper_for::<i32>(),
                element_cloner_for::<i32>(),
                element_eq_for::<i32>(),
                element_debug_for::<i32>(),
            )
        })
    };
    fn extract_i32(ptr: *const u8) -> Box<dyn Any> {
        Box::new(unsafe { *ptr.cast::<i32>() })
    }
    let extractors = [
        DynExtractor::Scalar(TypeId::of::<i32>(), extract_i32 as BoxExtractor),
        DynExtractor::Tuple(Box::new(leaf)),
    ];
    let results = unsafe { seg.call_dyn_tuple_mixed(&[], &extractors) }?;
    let nested = results[1]
        .downcast_ref::<DynamicSequence>()
        .expect("slot 1 is a DynamicSequence");
    assert_eq!(format!("{nested:?}"), "(2, 3)");
    Ok(())
}
```

This mirrors the exact local-`fn`/array-literal style
`call_dyn_tuple_mixed_splits_a_tuple_output_among_scalar_and_tuple_slots` (immediately above)
already uses for its own `extract_i32`/`extractors` — this new test defines its own copy of
`extract_i32` rather than sharing one, matching how every other test in this module is
self-contained.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime --lib dyn_segment::tests::call_dyn_as_dynamic_sequence_result_debug_formats_correctly dyn_segment::tests::call_dyn_tuple_mixed_tuple_slot_result_debug_formats_correctly`
Expected: FAIL to compile — the closures' 4-element return type doesn't match the still-3-element
parameter types in `call_dyn_as_dynamic_sequence`/`DynExtractor::Tuple`.

- [ ] **Step 3: Widen every `Option<(ElementDropper, ElementCloner, ElementEq)>` occurrence**

In `cel-runtime/src/dyn_segment.rs`, this exact type appears in four production signatures and
eleven existing test closures — replace every occurrence of the text
`Option<(ElementDropper, ElementCloner, ElementEq)>` with
`Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)>`. The four production sites need
their surrounding *bodies* updated too (below); the eleven test-closure sites need each closure's
returned tuple literal widened by one element (`element_debug_for::<T>()` appended after that
element's `element_eq_for::<T>()`), or left as bare `None` where the closure only ever returns
`None` (three sites: `call_dyn_as_dynamic_sequence_errors_if_result_is_not_a_tuple`,
`call_dyn_as_dynamic_sequence_errors_on_unregistered_leaf_type`, and one inside
`call_dyn_tuple_mixed_never_executes_when_a_tuple_slots_leaf_is_unregistered` — `None` needs no
change since it fits any `Option<T>`).

**Production site 1 — `call_dyn_as_dynamic_sequence`'s signature** (only the parameter type
changes; the body is untouched):

```rust
    pub fn call_dyn_as_dynamic_sequence(
        &mut self,
        inputs: &[&dyn Any],
        leaf: &impl Fn(TypeId) -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)>,
    ) -> anyhow::Result<DynamicSequence> {
```

**Production site 2 — `build_dynamic_sequence`** (signature and its `SequenceElement`
construction both change):

```rust
unsafe fn build_dynamic_sequence(
    base: *const u8,
    associated: &[AssociatedType],
    leaf: &(impl Fn(TypeId) -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> + ?Sized),
) -> anyhow::Result<DynamicSequence> {
    enum Built {
        Leaf,
        Tuple(DynamicSequence),
    }

    let mut shape = Vec::with_capacity(associated.len());
    let mut built: Vec<Built> = Vec::with_capacity(associated.len());
    let mut max_align = 1usize;
    let mut offset = 0usize;
    for elem in associated {
        let is_tuple = elem.type_id == TypeId::of::<DynTuple>();
        let (size, align, drop, clone, eq, debug, value) = if is_tuple {
            let nested =
                unsafe { build_dynamic_sequence(base.add(elem.offset), &elem.associated, leaf)? };
            (
                size_of::<DynamicSequence>(),
                align_of::<DynamicSequence>(),
                element_dropper_for::<DynamicSequence>(),
                element_cloner_for::<DynamicSequence>(),
                element_eq_for::<DynamicSequence>(),
                element_debug_for::<DynamicSequence>(),
                Built::Tuple(nested),
            )
        } else {
            let (drop, clone, eq, debug) = leaf(elem.type_id).ok_or_else(|| {
                anyhow!(
                    "call_dyn_as_dynamic_sequence: no Clone/PartialEq registered for element \
                     type `{}`",
                    elem.type_name
                )
            })?;
            (elem.size, elem.align, drop, clone, eq, debug, Built::Leaf)
        };
        let aligned = align_index(align, offset);
        max_align = max_align.max(align);
        shape.push(SequenceElement {
            type_id: if is_tuple {
                TypeId::of::<DynamicSequence>()
            } else {
                elem.type_id
            },
            type_name: if is_tuple {
                Cow::Borrowed(std::any::type_name::<DynamicSequence>())
            } else {
                elem.type_name.clone()
            },
            offset: aligned,
            size,
            align,
            drop,
            clone,
            eq,
            debug,
        });
        built.push(value);
        offset = aligned + size;
    }
    let total_size = align_index(max_align, offset);

    let mut buffer = RawStack::with_base_alignment(max_align);
    unsafe {
        buffer.reserve_and_write(max_align, total_size, |dst| {
            for ((elem, src_elem), value) in shape.iter().zip(associated).zip(built) {
                match value {
                    Built::Tuple(nested) => {
                        std::ptr::write(dst.add(elem.offset).cast::<DynamicSequence>(), nested);
                    }
                    Built::Leaf => {
                        std::ptr::copy_nonoverlapping(
                            base.add(src_elem.offset),
                            dst.add(elem.offset),
                            src_elem.size,
                        );
                    }
                }
            }
        });
    }

    Ok(unsafe { DynamicSequence::from_raw_parts(buffer, shape, max_align) })
}
```

(Only the `leaf` parameter's type, the `(size, align, drop, clone, eq, debug, value)` tuple
destructuring gaining `debug`, and the `SequenceElement { ..., debug, }` field change from the
current code — every other line is unchanged from what's already there.)

**Production site 3 — `validate_associated_shape`** (only the parameter type changes; body
untouched):

```rust
fn validate_associated_shape(
    associated: &[AssociatedType],
    leaf: &(impl Fn(TypeId) -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> + ?Sized),
) -> anyhow::Result<()> {
```

**Production site 4 — `DynExtractor::Tuple`'s variant declaration** (only the closure type
changes):

```rust
    /// A nested-tuple element: the closure supplies each of *its own* leaves'
    /// `Drop`/`Clone`/`PartialEq`/`Debug` function pointers by `TypeId`, exactly like
    /// `call_dyn_as_dynamic_sequence`'s `leaf` parameter.
    Tuple(Box<dyn Fn(TypeId) -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)>>),
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime --lib dyn_segment::`
Expected: PASS (every pre-existing test in this file, plus the 2 new ones). If a test closure was
missed in Step 3's widening, the compiler reports a precise type mismatch naming the test function
— fix each one the same way (append `element_debug_for::<T>()` to that closure's returned tuple).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dyn_segment.rs
git commit -m "feat(cel-runtime): thread ElementDebug through dyn_segment's tuple-construction machinery"
```

---

### Task 3: `adam-lang` — `TypeRegistry` wiring

**Files:**
- Modify: `adam-lang/src/type_registry.rs`

**Interfaces:**
- Consumes: `cel_runtime::{ElementDebug, element_debug_for}` (Tasks 1–2).
- Produces: `TypeEntry.element_debug: cel_runtime::ElementDebug`; `TypeRegistry::element_descriptor`
  returns `Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)>`;
  `TypeRegistry::element_descriptors_for` returns
  `Vec<(TypeId, ElementDropper, ElementCloner, ElementEq, ElementDebug)>`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `adam-lang/src/type_registry.rs`:

```rust
#[test]
fn new_registers_i32_with_a_debug_descriptor() {
    let reg = TypeRegistry::new();
    let entry = reg.get("i32").unwrap();
    let value = 7i32;
    struct Wrapper(*const u8, cel_runtime::ElementDebug);
    impl std::fmt::Debug for Wrapper {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            unsafe { (self.1)(self.0, f) }
        }
    }
    let wrapper = Wrapper((&raw const value).cast::<u8>(), entry.element_debug);
    assert_eq!(format!("{wrapper:?}"), "7");
}

#[test]
fn element_descriptor_includes_a_working_debug_formatter() {
    let reg = TypeRegistry::new();
    let (_, _, _, debug) = reg.element_descriptor(TypeId::of::<i32>()).unwrap();
    let value = 7i32;
    struct Wrapper(*const u8, cel_runtime::ElementDebug);
    impl std::fmt::Debug for Wrapper {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            unsafe { (self.1)(self.0, f) }
        }
    }
    let wrapper = Wrapper((&raw const value).cast::<u8>(), debug);
    assert_eq!(format!("{wrapper:?}"), "7");
}

#[test]
fn element_descriptors_for_a_tuple_shape_includes_debug_formatters() {
    let reg = TypeRegistry::new();
    let shape = TypeShape::Tuple(vec![
        TypeShape::Named(TypeId::of::<i32>()),
        TypeShape::Named(TypeId::of::<f64>()),
    ]);
    let table = reg.element_descriptors_for(&shape);
    assert_eq!(table.len(), 2);
    assert_eq!(table[0].0, TypeId::of::<i32>());
    assert_eq!(table[1].0, TypeId::of::<f64>());
}

#[test]
fn default_dynamic_sequence_result_debug_formats_correctly() {
    let reg = TypeRegistry::new();
    let shape = TypeShape::Tuple(vec![
        TypeShape::Named(TypeId::of::<i32>()),
        TypeShape::Named(TypeId::of::<f64>()),
    ]);
    let seq = reg.default_dynamic_sequence(&shape).unwrap();
    // i32::default() Debug-formats as "0"; f64::default() (0.0) Debug-formats as "0.0", not "0".
    assert_eq!(format!("{seq:?}"), "(0, 0.0)");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p adam-lang type_registry::tests::new_registers_i32_with_a_debug_descriptor type_registry::tests::element_descriptor_includes_a_working_debug_formatter type_registry::tests::element_descriptors_for_a_tuple_shape_includes_debug_formatters type_registry::tests::default_dynamic_sequence_result_debug_formats_correctly`
Expected: FAIL to compile — `TypeEntry` has no `element_debug` field yet, `element_descriptor`/
`element_descriptors_for` still return 3-/4-tuples.

- [ ] **Step 3: Implement**

Add a `pub element_debug: cel_runtime::ElementDebug` field to `TypeEntry`, right after its existing
`pub element_eq: cel_runtime::ElementEq` field (doc comment: `/// Debug-formatter for this type as
a \`DynamicSequence\` tuple element.`).

Add `+ std::fmt::Debug` to `register`'s and `register_no_default`'s generic bounds:

```rust
    pub fn register<T: Any + PartialEq + Default + Clone + std::fmt::Debug + 'static>(&mut self, name: &str) {
```

```rust
    pub fn register_no_default<T: Any + PartialEq + Clone + std::fmt::Debug + 'static>(&mut self, name: &str) {
```

In both functions' `TypeEntry { ... }` literals, add `element_debug: cel_runtime::element_debug_for::<T>(),`
right after the existing `element_eq: cel_runtime::element_eq_for::<T>(),` line (both functions
build an identical literal today — add the same line to both).

Update `element_descriptor`'s signature and body:

```rust
    /// Returns the `(Drop, Clone, PartialEq, Debug)` quadruple registered for `type_id`, for use
    /// as the `leaf` callback `cel_runtime::DynSegment::call_dyn_as_dynamic_sequence` needs.
    #[must_use]
    pub fn element_descriptor(
        &self,
        type_id: TypeId,
    ) -> Option<(
        cel_runtime::ElementDropper,
        cel_runtime::ElementCloner,
        cel_runtime::ElementEq,
        cel_runtime::ElementDebug,
    )> {
        self.entry_by_type_id(type_id)
            .map(|e| (e.element_drop, e.element_clone, e.element_eq, e.element_debug))
    }
```

Update `element_descriptors_for`'s signature and body:

```rust
    /// Builds an owned table of every leaf `TypeId` in `shape` paired with its
    /// `Drop`/`Clone`/`PartialEq`/`Debug` descriptor, for a closure that must outlive this
    /// registry (e.g. a `Method`'s stored output-extraction closure).
    ///
    /// - Precondition: every leaf `TypeId` in `shape` is registered (already resolved via
    ///   `TypeRegistry::resolve`, which would have already errored otherwise).
    ///
    /// - Complexity: O(n) in the number of leaves in `shape`.
    #[must_use]
    pub fn element_descriptors_for(
        &self,
        shape: &TypeShape,
    ) -> Vec<(
        TypeId,
        cel_runtime::ElementDropper,
        cel_runtime::ElementCloner,
        cel_runtime::ElementEq,
        cel_runtime::ElementDebug,
    )> {
        match shape {
            TypeShape::Named(type_id) => {
                let (drop, clone, eq, debug) = self
                    .element_descriptor(*type_id)
                    .expect("element_descriptors_for: type registered");
                vec![(*type_id, drop, clone, eq, debug)]
            }
            TypeShape::Tuple(elements) => elements
                .iter()
                .flat_map(|e| self.element_descriptors_for(e))
                .collect(),
        }
    }
```

Update `default_dyn_element`'s two `DynElementSpec { ... }` literals to add a `debug` field, right
after each literal's existing `eq: ...,` line:

- In the `TypeShape::Named` branch: `debug: entry.element_debug,`.
- In the `TypeShape::Tuple` branch: `debug: cel_runtime::element_debug_for::<cel_runtime::DynamicSequence>(),`
  (mirroring the existing `drop`/`clone`/`eq`/`write` lines in that same branch, which all call the
  analogous `cel_runtime::element_*_for::<cel_runtime::DynamicSequence>()`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p adam-lang type_registry::`
Expected: PASS (every pre-existing test in this file plus the 4 new ones). Run `cargo check -p
adam-lang --lib --tests 2>&1 | head -50` to confirm remaining compile errors (if any) are confined
to `parser.rs` (Task 4's job).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-lang/src/type_registry.rs
git commit -m "feat(adam-lang): thread ElementDebug through TypeRegistry"
```

---

### Task 4: `adam-lang` — `parser.rs` consumption

**Files:**
- Modify: `adam-lang/src/parser.rs`

**Interfaces:**
- Consumes: `TypeRegistry::element_descriptor`/`element_descriptors_for`'s widened return types
  (Task 3).
- Produces: `CompiledOutputs::SingleTuple`'s payload type widens to
  `Vec<(TypeId, ElementDropper, ElementCloner, ElementEq, ElementDebug)>`.

`eval_segment_boxed`'s `let leaf = |type_id: TypeId| self.types.element_descriptor(type_id);` needs
no change — it returns `self.types.element_descriptor(...)`'s value directly with no destructuring,
so it widens automatically once Task 3 lands. Two sites do need edits.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `adam-lang/src/parser.rs`:

```rust
#[test]
fn parse_out_with_tuple_type_value_debug_formats_correctly() {
    let mut sheet = parser()
        .parse_str(
            r#"
            sheet s {
                cell x: i32 = 3;
                out pair: (i32, i32) { method [x] { (x, x) } }
            }
        "#,
        )
        .unwrap();
    sheet.propagate().unwrap();
    let output_id = *sheet.output_names.get("pair").unwrap();
    let cell_id = sheet.output_cell(output_id).unwrap();
    let value = sheet.read::<cel_runtime::DynamicSequence>(cell_id).unwrap();
    assert_eq!(format!("{value:?}"), "(3, 3)");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p adam-lang parser::tests::parse_out_with_tuple_type_value_debug_formats_correctly`
Expected: FAIL to compile — `CompiledOutputs::SingleTuple`'s payload and the `DynExtractor::Tuple`
closure built in `parse_method_body` are still 4-/3-element tuples, mismatched against Task 3's
now-widened `element_descriptors_for`.

- [ ] **Step 3: Widen the two consuming sites**

Update `CompiledOutputs::SingleTuple`'s payload type:

```rust
    /// One output, tuple-typed: the segment's whole tuple result, moved into one
    /// `DynamicSequence` via `call_dyn_as_dynamic_sequence`.
    SingleTuple(
        Vec<(
            TypeId,
            cel_runtime::ElementDropper,
            cel_runtime::ElementCloner,
            cel_runtime::ElementEq,
            cel_runtime::ElementDebug,
        )>,
    ),
```

Update its execution arm in `build_method`'s closure:

```rust
                CompiledOutputs::SingleTuple(table) => {
                    let leaf = |type_id: TypeId| {
                        table
                            .iter()
                            .find(|(tid, ..)| *tid == type_id)
                            .map(|(_, d, c, e, dbg)| (*d, *c, *e, *dbg))
                    };
                    let seq = seg.call_dyn_as_dynamic_sequence(inputs_any, &leaf)?;
                    Ok(vec![Box::new(seq) as Box<dyn Any>])
                }
```

Update `parse_method_body`'s multi-output branch's `DynExtractor::Tuple` construction:

```rust
                    TypeShape::Tuple(_) => {
                        let table = self.types.element_descriptors_for(out_shape);
                        cel_runtime::DynExtractor::Tuple(Box::new(move |type_id: TypeId| {
                            table
                                .iter()
                                .find(|(tid, ..)| *tid == type_id)
                                .map(|(_, d, c, e, dbg)| (*d, *c, *e, *dbg))
                        }))
                    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p adam-lang parser::`
Expected: PASS (every pre-existing test in this file plus the new one). Then run `cargo test -p
adam-lang` and `cargo build --workspace` to confirm the whole crate/workspace compiles clean
again (no remaining `Task N` gaps anywhere).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add adam-lang/src/parser.rs
git commit -m "feat(adam-lang): widen CompiledOutputs::SingleTuple/DynExtractor::Tuple for ElementDebug"
```

---

### Task 5: `begin` — display tuple-typed cells in the Inspector

**Files:**
- Modify: `begin/src/bridge.rs`

**Interfaces:**
- Consumes: `cel_runtime::DynamicSequence`'s real `Debug` impl (Task 1).
- Produces: `Labels::add_tuple_cell(&mut self, id: CellId, label: &str)`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `begin/src/bridge.rs`, right after
`display_closure_returns_value_string`:

```rust
#[test]
fn add_tuple_cell_display_returns_rust_debug_formatted_string() {
    let mut sheet = Sheet::new();
    let cell_id = sheet.add_cell(cel_runtime::DynamicSequence::from_tuple((3i32, 4.5f64)));
    let mut labels = Labels::new();
    labels.add_tuple_cell(cell_id, "pair");
    let meta = labels.cells.get(&cell_id).unwrap();
    assert_eq!((meta.display)(&sheet), "(3, 4.5)");
}

#[test]
fn add_tuple_cell_write_str_always_errs_without_mutating_the_sheet() {
    let mut sheet = Sheet::new();
    let cell_id = sheet.add_cell(cel_runtime::DynamicSequence::from_tuple((3i32, 4.5f64)));
    let mut labels = Labels::new();
    labels.add_tuple_cell(cell_id, "pair");
    let meta = labels.cells.get(&cell_id).unwrap();
    let before = sheet.read::<cel_runtime::DynamicSequence>(cell_id).unwrap().clone();
    let result = (meta.write_str)(&mut sheet, "(1, 2.0)");
    assert!(result.is_err());
    let after = sheet.read::<cel_runtime::DynamicSequence>(cell_id).unwrap();
    assert_eq!(&before, after);
}
```

`Sheet::new() -> Self` and `Sheet::add_cell<T: Any + PartialEq + 'static>(&mut self, value: T) ->
CellId` (both `pub`, in `adam-rs/src/sheet.rs`) are the two calls the snippets above use;
`DynamicSequence` satisfies `add_cell`'s bound (it already implements both `Any` and `PartialEq`).
This file's own existing tests (`display_closure_returns_value_string`,
`write_str_closure_parses_and_writes`) instead build a fixture via a shared `demo_sheet()` helper
already in this file — that helper builds a fixed demo sheet with no tuple cell in it, so it isn't
reusable for these two new tests; constructing a fresh `Sheet` directly, as above, is the right
choice here.

Also add this new test, right after `labels_from_cell_names_builds_entries_for_supported_types`
(that existing test only exercises four scalar-typed cells — `f64`/`i32`/`bool`/`String` — and
makes no claim about tuple cells either way, so it needs no changes; this is a new, separate test
covering the tuple case `labels_from_cell_names` didn't handle before):

```rust
#[test]
fn labels_from_cell_names_includes_tuple_typed_cells() {
    use std::any::TypeId;

    let mut sheet = Sheet::new();
    let pair = sheet.add_cell(cel_runtime::DynamicSequence::from_tuple((3i32, 4.5f64)));

    let mut cell_names = IndexMap::new();
    cell_names.insert(
        "pair".to_string(),
        (
            pair,
            TypeShape::Tuple(vec![
                TypeShape::Named(TypeId::of::<i32>()),
                TypeShape::Named(TypeId::of::<f64>()),
            ]),
        ),
    );

    let labels = labels_from_cell_names(&cell_names);

    assert_eq!(labels.cells.len(), 1);
    assert_eq!((labels.cells[&pair].display)(&sheet), "(3, 4.5)");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p begin --lib bridge::tests::add_tuple_cell_display_returns_rust_debug_formatted_string bridge::tests::add_tuple_cell_write_str_always_errs_without_mutating_the_sheet bridge::tests::labels_from_cell_names_includes_tuple_typed_cells`
Expected: FAIL to compile — `Labels::add_tuple_cell` doesn't exist yet.

- [ ] **Step 3: Implement**

Add to `impl Labels` in `begin/src/bridge.rs`, right after `add_cell`:

```rust
    /// Registers display-only metadata for a tuple-typed cell of any shape.
    ///
    /// `write_str` always returns `Err` — no tuple-literal parser exists yet (tracked as a
    /// follow-up: see the "Support editing tuple-typed cells in `begin`" GitHub issue). The
    /// field still participates fully in the Inspector's existing invalid/warning/disabled
    /// machinery, since that's entirely keyed on `CellId`, not on any per-type behavior.
    ///
    /// - Precondition: `id` is a live cell in the sheet this `Labels` will be used with, holding
    ///   a `cel_runtime::DynamicSequence`.
    pub fn add_tuple_cell(&mut self, id: CellId, label: &str) {
        self.cells.insert(
            id,
            CellMeta {
                label: label.to_owned(),
                display: Box::new(move |sheet| {
                    sheet
                        .read::<cel_runtime::DynamicSequence>(id)
                        .map(|v| format!("{v:?}"))
                        .unwrap_or_else(|_| "?".to_owned())
                }),
                write_str: Box::new(|_sheet, _s| {
                    Err(Error::MethodFailed(anyhow::anyhow!(
                        "editing tuple-typed cells is not yet supported"
                    )))
                }),
            },
        );
    }
```

Update `labels_from_cell_names`'s tuple branch:

```rust
    for (name, (id, shape)) in cell_names {
        let id = *id;
        let type_id = match shape {
            TypeShape::Named(type_id) => *type_id,
            TypeShape::Tuple(_) => {
                labels.add_tuple_cell(id, name);
                continue;
            }
        };
```

Update `labels_from_cell_names`'s doc comment — remove the sentence "and any tuple-typed cell
(`TypeShape::Tuple`), not yet supported in the sidebar — are silently skipped, so they simply
won't appear in the sidebar", replacing it with a note that a tuple-typed cell now appears with a
Debug-formatted, display-only entry via `Labels::add_tuple_cell`.

Update `CellMeta::write_str`'s doc comment to note it may always return `Err` for a cell type with
no write support yet (e.g. tuples).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p begin --lib bridge::`
Expected: PASS (every pre-existing test in this file, unchanged, plus the 3 new ones from Step 1).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add begin/src/bridge.rs
git commit -m "feat(begin): display tuple-typed cell values in the Inspector"
```

---

### Task 6: Full verification, UI check, and the follow-up issue

**Files:** none (verification only, plus filing a GitHub issue).

- [ ] **Step 1: Run the full test suite**

Run: `cargo test --workspace` and `cargo test --doc --workspace`.
Expected: PASS, zero compiler warnings.

- [ ] **Step 2: Run all three clippy invocations**

Run, in order:
```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: PASS.

- [ ] **Step 3: Format**

Run: `cargo fmt --all`.

- [ ] **Step 4: Render and verify the UI**

Using the `verifying-begin-ui` skill, load or construct an example `.adm2` sheet containing an
`out` cell with a tuple type (e.g. `out pair: (i32, i32) { method [x, y] -> ... }`, plus a
`condition` on that output that can be made to fail) and confirm:
- The tuple `out` cell's field shows its Rust-Debug-formatted value (e.g. `(3, 4)`), not blank or
  an error.
- Violating the output's condition still shows the existing warning-border affordance on the
  tuple field, exactly as it already does for a scalar `out` cell.
- Typing into the tuple field and blurring shows the existing invalid-then-revert behavior
  (matching how a scalar `out` cell already behaves when written to), not a crash or a stuck bad
  value.

If any of these don't hold, fix the specific gap found (in whichever file from Tasks 1–5 is
responsible) and re-verify — do not report this task done on the strength of `cargo build`/`clippy`
alone.

- [ ] **Step 5: File the follow-up GitHub issue**

Run:

```bash
gh issue create --repo stlab/cel-rs --title "Support editing tuple-typed cells in begin" --body "$(cat <<'EOF'
Tuple-typed cells in begin's Inspector are currently display-only: \`Labels::add_tuple_cell\`
(begin/src/bridge.rs) formats a cell's DynamicSequence value via its new Debug impl, but
\`write_str\` always returns \`Err\` without attempting to parse anything -- no tuple-literal
parser exists yet.

This issue tracks adding real editing support: parse user input into a \`DynamicSequence\`
matching a cell's declared \`TypeShape\` (arity + per-element leaf type, recursively for nested
tuples), then call \`Sheet::write\`. See docs/superpowers/specs/2026-08-12-tuple-value-display-design.md
for the display-only design this issue follows on from.
EOF
)"
```

Report the issue URL `gh` prints.

- [ ] **Step 6: Final commit (if Step 4 required fixes)**

```bash
cargo fmt --all
git add -A
git commit -m "fix(begin): address UI verification findings for tuple value display"
```

Skip this step if Step 4 found nothing to fix.
