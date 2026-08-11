# cel-runtime: Runtime-Shape-Driven `DynamicSequence` Primitives Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `cel-runtime` primitives needed to build/consume a `DynamicSequence` whose
shape is discovered only at *Rust run time* (from a `TypeId`/layout list, not a compile-time
generic `T`) — the missing piece adam-lang needs to support DSL-declared tuple types, since every
existing `DynamicSequence`/`DynSegment` tuple API (`from_tuple::<T>`, `call_dyn_as_tuple::<T>`,
etc.) requires `T` known at compile time.

**Architecture:** Three new `DynSegment` methods, symmetric with the existing
`call_dyn_tuple`/`call_dyn_as_tuple`/`push_arg` family: `call_dyn_as_dynamic_sequence` (output —
moves a live on-stack tuple result into an owned `DynamicSequence`, recursing into nested tuple
elements as nested `DynamicSequence` leaves), `call_dyn_tuple_mixed` (output, N-way split — the
same recursive conversion applied to *one element* of a larger tuple result, so adam-lang's
existing multi-output-method mechanism can have a tuple-typed slot among several scalar ones), and
`push_arg_as_dynamic_sequence_tuple` (input — clones a stored `DynamicSequence` back onto the stack
as a live, indexable `DynTuple`, recursing the other way). A new `DynamicSequence::from_dyn_elements`
builds a sequence from already-boxed values (for the no-initializer default-value case, where
there is no CEL expression to evaluate). All of these are driven by runtime `TypeId` lists
(recursive `AssociatedType`, matching the existing on-stack tuple representation) rather than a
generic `T`. A small set of new, fully generic (`T`-parametrized, zero-capture) helper functions
generate the per-element `Clone`/`PartialEq`/`Drop` function pointers these primitives and their
callers need, replacing what was previously private, inlined logic in `push_element`/`push_type`.

**Tech Stack:** Rust, `cel-runtime` only. No new external dependencies. `anyhow` for fallible ops
(already a dependency).

**Reference:** `docs/superpowers/specs/2026-08-11-adam-lang-tuple-types-design.md` (sections 4).

## Global Constraints

- Format with `cargo fmt --all` before every commit (enforced by pre-commit hook).
- Every function/trait/struct needs a contract-style `///` doc comment (Summary, Preconditions as
  `debug_assert!`, `# Errors`/`# Safety` where applicable, Postconditions, Complexity if not O(1))
  per the root `CLAUDE.md`.
- Unit tests are derived from contract/public interface only — never from implementation
  internals.
- Run `cargo test -p cel-runtime` after every task's implementation step; run
  `cargo test --workspace`, `cargo test --doc --workspace`, and all three `cargo clippy`
  invocations from the root `CLAUDE.md` before the final commit of the whole plan (Task 6).
- **`cel-runtime` must never depend on `adam-rs`/`adam-lang`, even as a dev-dependency** — this
  exact coupling was tried and reverted twice in this repo (commits `9768dfd`, `9e0118c`): a
  cross-crate `DynamicSequence` + `Sheet`/`Method` acceptance test was added, then removed in
  both directions because `cel-runtime` and `adam-rs` are meant to stay fully independent. All
  tests in this plan exercise `DynSegment`/`DynamicSequence` directly, with no `Sheet`/`Method`
  involved. The true end-to-end proof (a real `Sheet` with a tuple-typed cell) belongs in the
  adam-lang follow-up plan, which already depends on both crates legitimately.
- No heap allocation beyond what's inherent to the primitive: one allocation per
  `DynamicSequence` (its own `RawStack`), plus one per nested tuple level (a runtime-discovered
  shape can't be packed into one flat buffer the way the compile-time-`T` path can, since a
  nested tuple's *destination* representation — `DynamicSequence` — has a fixed size/align
  unrelated to its own elements' sizes; this tradeoff is already documented in the spec).

---

### Task 1: Generic per-type descriptor helpers

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`
- Modify: `cel-runtime/src/dyn_segment.rs`

**Interfaces:**
- Produces: `pub fn element_dropper_for<T: 'static>() -> ElementDropper`,
  `pub fn element_cloner_for<T: 'static + Clone>() -> ElementCloner`,
  `pub fn element_eq_for<T: 'static + PartialEq>() -> ElementEq`,
  `pub fn element_writer_for<T: 'static>() -> unsafe fn(Box<dyn std::any::Any>, *mut u8)` (in
  `dynamic_sequence.rs`); `pub fn raw_dropper_for<T: 'static>() -> RawDropper` (in
  `dyn_segment.rs`); `pub fn drop_tuple` (was private `fn drop_tuple`, in `dyn_segment.rs` — made
  `pub` since later tasks and adam-lang's follow-up plan both need it); `pub(crate) unsafe fn
  DynamicSequence::from_raw_parts(buffer: RawStack, shape: Vec<SequenceElement>, max_align:
  usize) -> Self`; `pub(crate) fn DynamicSequence::shape(&self) -> &[SequenceElement]`;
  `pub(crate) fn DynamicSequence::read_element_at<R>(&self, offset: usize, read: impl
  FnOnce(*const u8) -> R) -> R`.

These four generic helpers are exactly what `push_element` (in `dynamic_sequence.rs`) and
`push_type`/`make_tuple` (in `dyn_segment.rs`) already generate inline, per concrete `T`, as
non-capturing closures — this task exposes that same generation as small, independently-testable
public functions and has the existing private code call them (DRY; no behavior change to
existing callers).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dynamic_sequence.rs`:

```rust
#[test]
fn element_dropper_for_drops_the_correct_type_exactly_once() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct DropCounter(Arc<AtomicUsize>);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let count = Arc::new(AtomicUsize::new(0));
    let mut value = DropCounter(count.clone());
    let dropper = element_dropper_for::<DropCounter>();
    unsafe { dropper((&raw mut value).cast::<u8>()) };
    std::mem::forget(value);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn element_cloner_for_clones_the_correct_type() {
    let cloner = element_cloner_for::<i32>();
    let src = 7i32;
    let mut dst = 0i32;
    unsafe { cloner((&raw const src).cast::<u8>(), (&raw mut dst).cast::<u8>()) };
    assert_eq!(dst, 7);
}

#[test]
fn element_eq_for_compares_the_correct_type() {
    let eq = element_eq_for::<i32>();
    let (a, b, c) = (5i32, 5i32, 6i32);
    assert!(unsafe { eq((&raw const a).cast::<u8>(), (&raw const b).cast::<u8>()) });
    assert!(!unsafe { eq((&raw const a).cast::<u8>(), (&raw const c).cast::<u8>()) });
}

#[test]
fn element_writer_for_moves_a_boxed_value_without_dropping_it() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone, PartialEq)]
    struct DropCounter(Arc<AtomicUsize>);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let count = Arc::new(AtomicUsize::new(0));
    let boxed: Box<dyn std::any::Any> = Box::new(DropCounter(count.clone()));
    let writer = element_writer_for::<DropCounter>();
    let mut dst = std::mem::MaybeUninit::<DropCounter>::uninit();
    unsafe { writer(boxed, dst.as_mut_ptr().cast::<u8>()) };
    assert_eq!(count.load(Ordering::SeqCst), 0, "the box's move must not run Drop");
    unsafe { dst.assume_init_drop() };
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
```

Add to the `tests` module in `cel-runtime/src/dyn_segment.rs`:

```rust
#[test]
fn raw_dropper_for_ignores_associated_and_drops_the_correct_type_exactly_once() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct DropCounter(Arc<AtomicUsize>);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let count = Arc::new(AtomicUsize::new(0));
    let mut value = DropCounter(count.clone());
    let dropper = raw_dropper_for::<DropCounter>();
    unsafe { dropper((&raw mut value).cast::<u8>(), &[]) };
    std::mem::forget(value);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime element_dropper_for element_cloner_for element_eq_for element_writer_for raw_dropper_for`
Expected: FAIL with "cannot find function" for each.

- [ ] **Step 3: Implement the four helpers, `drop_tuple` visibility, and the `DynamicSequence` crate-internal accessors**

In `cel-runtime/src/dynamic_sequence.rs`, add after the `SequenceElement` struct definition (before
`push_element`):

```rust
/// Returns an [`ElementDropper`] that drops a value of type `T` in place.
pub fn element_dropper_for<T: 'static>() -> ElementDropper {
    |ptr| unsafe { std::ptr::drop_in_place(ptr.cast::<T>()) }
}

/// Returns an [`ElementCloner`] that clones a value of type `T` in place.
pub fn element_cloner_for<T: 'static + Clone>() -> ElementCloner {
    |src, dst| unsafe { std::ptr::write(dst.cast::<T>(), (*src.cast::<T>()).clone()) }
}

/// Returns an [`ElementEq`] that compares two values of type `T` for equality.
pub fn element_eq_for<T: 'static + PartialEq>() -> ElementEq {
    |a, b| unsafe { *a.cast::<T>() == *b.cast::<T>() }
}

/// Returns a function that moves a boxed `T`'s bytes to `dst`, consuming the box without
/// running `T`'s destructor (ownership transfers to `dst`).
///
/// # Safety
/// The returned function's `dst` must be valid for writes of `size_of::<T>()` bytes at
/// `align_of::<T>()`; its `Box<dyn Any>` argument's runtime type must be `T`.
pub fn element_writer_for<T: 'static>() -> unsafe fn(Box<dyn std::any::Any>, *mut u8) {
    |boxed, dst| unsafe {
        let value = *boxed
            .downcast::<T>()
            .expect("element_writer_for: type mismatch");
        std::ptr::write(dst.cast::<T>(), value);
    }
}
```

Update `push_element` to call these instead of inlining the same three closures:

```rust
fn push_element<T: 'static + Clone + PartialEq>(
    out: &mut Vec<SequenceElement>,
    offset: usize,
    max_align: &mut usize,
) -> usize {
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
    });
    aligned_offset + size_of::<T>()
}
```

Add near the bottom of `impl DynamicSequence` (after `adapt_fn_1`):

```rust
    /// Assembles a `DynamicSequence` directly from an already-populated buffer and shape.
    ///
    /// # Safety
    /// `buffer` must contain exactly the bytes described by `shape`, laid out at each element's
    /// own `offset`; `max_align` must be at least as large as every element's `align`.
    pub(crate) unsafe fn from_raw_parts(
        buffer: crate::raw_stack::RawStack,
        shape: Vec<SequenceElement>,
        max_align: usize,
    ) -> Self {
        DynamicSequence {
            buffer,
            shape,
            max_align,
        }
    }

    /// Returns this sequence's own element shape, for use by `dyn_segment`'s tuple-expansion
    /// machinery.
    pub(crate) fn shape(&self) -> &[SequenceElement] {
        &self.shape
    }

    /// Reads this sequence's element at `offset` via `read`, given a pointer to its start.
    ///
    /// - Precondition: `offset` is one of `self.shape()`'s own recorded element offsets.
    pub(crate) fn read_element_at<R>(&self, offset: usize, read: impl FnOnce(*const u8) -> R) -> R {
        unsafe { self.buffer.read_at(offset, read) }
    }
```

In `cel-runtime/src/dyn_segment.rs`, change `fn drop_tuple` to `pub fn drop_tuple` (used by both
this crate's later tasks and adam-lang's own tuple-type-prototype construction). Add, right after
`drop_tuple`'s definition:

```rust
/// Returns a [`RawDropper`] that drops a value of type `T` in place, ignoring the `associated`
/// parameter (a non-tuple leaf value has no nested elements to recurse into).
pub fn raw_dropper_for<T: 'static>() -> RawDropper {
    |ptr, _associated| unsafe { std::ptr::drop_in_place(ptr.cast::<T>()) }
}
```

Update `push_type` to call it instead of inlining the same closure:

```rust
    fn push_type<T>(&mut self)
    where
        T: 'static,
    {
        let current = self.stack_offset_after(self.stack_ids.len());
        let aligned_index = align_index(align_of::<T>(), current);
        let padded = aligned_index != current;

        self.stack_ids.push(StackInfo {
            type_id: TypeId::of::<T>(),
            type_name: Cow::Borrowed(std::any::type_name::<T>()),
            padding: padded,
            size: size_of::<T>(),
            align: align_of::<T>(),
            raw_dropper: raw_dropper_for::<T>(),
            associated: Vec::new(),
        });
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime dynamic_sequence:: dyn_segment::`
Expected: PASS (new tests, plus every pre-existing test in both modules still passing — this
step is a pure refactor of `push_element`/`push_type`'s internals).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dynamic_sequence.rs cel-runtime/src/dyn_segment.rs
git commit -m "feat(cel-runtime): add generic per-type element/raw descriptor helpers"
```

---

### Task 2: `layout_associated` — shared offset/alignment computation

**Files:**
- Modify: `cel-runtime/src/dyn_segment.rs`

**Interfaces:**
- Consumes: `AssociatedType` (existing).
- Produces: `pub fn layout_associated(elements: &mut [AssociatedType]) -> (usize, usize)`.

`make_tuple` already computes exactly this layout inline (each element placed at its own
alignment, then padded up to the running max alignment seen so far — the convention matching
`CStackList`'s nested layout, per `make_tuple`'s own doc comment). This task extracts that one
piece of the computation into its own tested function, so Task 5's `push_arg_as_dynamic_sequence_tuple`
can reuse the *exact* convention without re-deriving it. `make_tuple` itself is not modified —
it's already shipped and tested; duplicating its offset formula here (verified against it by test)
is lower-risk than editing it.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `cel-runtime/src/dyn_segment.rs`:

```rust
#[test]
fn layout_associated_matches_make_tuples_own_padding_convention() {
    // Mirrors make_tuple's documented layout for (f64, i8, i8): each element at its own
    // alignment, then padded up to the running max alignment seen so far — inserting extra
    // padding between the two i8 elements once f64 raises the running max to 8.
    let mut elements = vec![
        AssociatedType {
            type_id: TypeId::of::<f64>(),
            type_name: Cow::Borrowed("f64"),
            offset: 0,
            size: size_of::<f64>(),
            align: align_of::<f64>(),
            dropper: raw_dropper_for::<f64>(),
            associated: Vec::new(),
        },
        AssociatedType {
            type_id: TypeId::of::<i8>(),
            type_name: Cow::Borrowed("i8"),
            offset: 0,
            size: size_of::<i8>(),
            align: align_of::<i8>(),
            dropper: raw_dropper_for::<i8>(),
            associated: Vec::new(),
        },
        AssociatedType {
            type_id: TypeId::of::<i8>(),
            type_name: Cow::Borrowed("i8"),
            offset: 0,
            size: size_of::<i8>(),
            align: align_of::<i8>(),
            dropper: raw_dropper_for::<i8>(),
            associated: Vec::new(),
        },
    ];
    let (total_size, align) = layout_associated(&mut elements);
    assert_eq!(elements.iter().map(|e| e.offset).collect::<Vec<_>>(), vec![0, 8, 16]);
    assert_eq!(total_size, 24);
    assert_eq!(align, 8);
}

#[test]
fn layout_associated_returns_zero_size_for_no_elements() {
    let mut elements: Vec<AssociatedType> = Vec::new();
    let (total_size, align) = layout_associated(&mut elements);
    assert_eq!(total_size, 0);
    assert_eq!(align, 1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime layout_associated`
Expected: FAIL with "cannot find function `layout_associated`".

- [ ] **Step 3: Implement `layout_associated`**

Add to `cel-runtime/src/dyn_segment.rs`, right after `raw_dropper_for`:

```rust
/// Computes each element's on-stack byte offset in place — the same convention [`make_tuple`]
/// already uses: place at this element's own alignment, then pad up to the running max
/// alignment seen so far (matching `CStackList`'s nested layout). Each element's `type_id`,
/// `size`, and `align` must already be set; `offset` is overwritten. Returns
/// `(total_size, max_align)`.
///
/// - Complexity: O(n).
pub fn layout_associated(elements: &mut [AssociatedType]) -> (usize, usize) {
    let mut offset = 0usize;
    let mut max_align = 1usize;
    for elem in elements.iter_mut() {
        offset = align_index(elem.align, offset);
        max_align = max_align.max(elem.align);
        elem.offset = offset;
        offset += elem.size;
        offset = align_index(max_align, offset);
    }
    (offset, max_align)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime layout_associated`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dyn_segment.rs
git commit -m "feat(cel-runtime): add layout_associated, matching make_tuple's padding convention"
```

---

### Task 3: `DynSegment::call_dyn_as_dynamic_sequence` and `call_dyn_tuple_mixed` — output direction

**Files:**
- Modify: `cel-runtime/src/dyn_segment.rs`

**Interfaces:**
- Consumes: `element_dropper_for`/`element_cloner_for`/`element_eq_for` (Task 1);
  `DynamicSequence::from_raw_parts` (Task 1); existing `DynSegment` internals
  (`stack_ids`, `argument_ids`, `segment.call0_stack`, `CALL_DYN_PTR`/`CALL_DYN_LEN`/`DynCallGuard`,
  `AssociatedType`, `DynTuple`).
- Produces: `DynSegment::call_dyn_as_dynamic_sequence(&mut self, inputs: &[&dyn Any], leaf: &impl
  Fn(TypeId) -> Option<(ElementDropper, ElementCloner, ElementEq)>) -> anyhow::Result<DynamicSequence>`;
  `pub enum DynExtractor { Scalar(TypeId, BoxExtractor), Tuple(Box<dyn Fn(TypeId) ->
  Option<(ElementDropper, ElementCloner, ElementEq)>>) }`; `DynSegment::call_dyn_tuple_mixed(&mut
  self, inputs: &[&dyn Any], extractors: &[DynExtractor]) -> anyhow::Result<Vec<Box<dyn Any>>>`.

Mirrors `call_dyn_as_tuple::<T>` exactly, except the destination shape is discovered recursively
from the live on-stack tuple's own `AssociatedType` list (via `leaf`, supplied by the caller for
each non-tuple element) instead of a compile-time `T::Output::append_shape`. A nested tuple
element becomes its own, independently-allocated nested `DynamicSequence` (see the plan header's
"no heap allocation beyond..." constraint) — reusing `DynamicSequence`'s own
`Clone`/`PartialEq`/`Drop` (via `element_*_for::<DynamicSequence>()`) rather than any per-shape
generated code.

This task also adds `call_dyn_tuple_mixed`: the adam-lang follow-up plan's method-output
unification needs to split an N>1-arity tuple result across several declared outputs where *one
of them* is itself tuple-typed (e.g. `method [a,b] -> [pair, extra] { ((a,a), a) }`) — a
generalization of the existing `call_dyn_tuple` (which only ever boxes scalar
`BoxExtractor`-shaped elements) that boxes a nested `DynamicSequence` for a `Tuple`-shaped slot
instead, by reusing this task's own `build_dynamic_sequence` on just that one element's own
`(base + offset, associated)` sub-region — not the whole top-of-stack value.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dyn_segment.rs`:

```rust
#[test]
fn call_dyn_as_dynamic_sequence_builds_a_flat_sequence() -> anyhow::Result<()> {
    let mut seg = DynSegment::new::<()>();
    let ambient_start = seg.current_stack_offset();
    seg.op0(|| 10i32);
    seg.op0(|| 2.5f64);
    seg.make_tuple(2, ambient_start);

    let leaf = |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq)> {
        if type_id == TypeId::of::<i32>() {
            Some((
                element_dropper_for::<i32>(),
                element_cloner_for::<i32>(),
                element_eq_for::<i32>(),
            ))
        } else if type_id == TypeId::of::<f64>() {
            Some((
                element_dropper_for::<f64>(),
                element_cloner_for::<f64>(),
                element_eq_for::<f64>(),
            ))
        } else {
            None
        }
    };
    let seq = seg.call_dyn_as_dynamic_sequence(&[], &leaf)?;
    assert_eq!(seq.arity(), 2);
    let (a, b): (i32, f64) = seq.try_to_tuple()?;
    assert_eq!((a, b), (10, 2.5));
    Ok(())
}

#[test]
fn call_dyn_as_dynamic_sequence_recurses_into_nested_tuples() -> anyhow::Result<()> {
    let mut seg = DynSegment::new::<()>();
    let ambient_start = seg.current_stack_offset();
    seg.op0(|| 1i32);
    let inner_start = seg.current_stack_offset();
    seg.op0(|| 2i32);
    seg.op0(|| 3i32);
    seg.make_tuple(2, inner_start);
    seg.make_tuple(2, ambient_start);

    let leaf = |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq)> {
        (type_id == TypeId::of::<i32>()).then(|| {
            (
                element_dropper_for::<i32>(),
                element_cloner_for::<i32>(),
                element_eq_for::<i32>(),
            )
        })
    };
    let seq = seg.call_dyn_as_dynamic_sequence(&[], &leaf)?;
    assert_eq!(seq.arity(), 2);
    let (a, nested): (i32, DynamicSequence) = seq.try_to_tuple()?;
    assert_eq!(a, 1);
    assert_eq!(nested.arity(), 2);
    let (b, c): (i32, i32) = nested.try_to_tuple()?;
    assert_eq!((b, c), (2, 3));
    Ok(())
}

#[test]
fn call_dyn_as_dynamic_sequence_errors_if_result_is_not_a_tuple() {
    let mut seg = DynSegment::new::<()>();
    seg.op0(|| 5i32);
    let leaf = |_: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq)> { None };
    let result = seg.call_dyn_as_dynamic_sequence(&[], &leaf);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("tuple"));
}

#[test]
fn call_dyn_as_dynamic_sequence_errors_on_unregistered_leaf_type() {
    let mut seg = DynSegment::new::<()>();
    let ambient_start = seg.current_stack_offset();
    seg.op0(|| 1i32);
    seg.op0(|| 2i32);
    seg.make_tuple(2, ambient_start);
    let leaf = |_: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq)> { None };
    let result = seg.call_dyn_as_dynamic_sequence(&[], &leaf);
    assert!(result.is_err());
}

#[test]
fn call_dyn_as_dynamic_sequence_drops_every_element_exactly_once() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone, PartialEq)]
    struct DropCounter(Arc<AtomicUsize>);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let count = Arc::new(AtomicUsize::new(0));
    let mut seg = DynSegment::new::<()>();
    let ambient_start = seg.current_stack_offset();
    let a = DropCounter(count.clone());
    seg.op0(move || a.clone());
    seg.op0(|| 7i32);
    seg.make_tuple(2, ambient_start);

    let leaf = |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq)> {
        if type_id == TypeId::of::<DropCounter>() {
            Some((
                element_dropper_for::<DropCounter>(),
                element_cloner_for::<DropCounter>(),
                element_eq_for::<DropCounter>(),
            ))
        } else if type_id == TypeId::of::<i32>() {
            Some((
                element_dropper_for::<i32>(),
                element_cloner_for::<i32>(),
                element_eq_for::<i32>(),
            ))
        } else {
            None
        }
    };
    let seq = seg.call_dyn_as_dynamic_sequence(&[], &leaf).unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 0, "moving out must not drop the element");
    drop(seq);
    assert_eq!(count.load(Ordering::SeqCst), 1, "must still drop exactly once");
}

#[test]
fn call_dyn_tuple_mixed_splits_a_tuple_output_among_scalar_and_tuple_slots() -> anyhow::Result<()> {
    // (i32, (i32, i32)) split into 2 declared outputs: a scalar, and a nested tuple.
    let mut seg = DynSegment::new::<()>();
    let ambient_start = seg.current_stack_offset();
    seg.op0(|| 1i32);
    let inner_start = seg.current_stack_offset();
    seg.op0(|| 2i32);
    seg.op0(|| 3i32);
    seg.make_tuple(2, inner_start);
    seg.make_tuple(2, ambient_start);

    fn extract_i32(ptr: *const u8) -> Box<dyn Any> {
        Box::new(unsafe { *ptr.cast::<i32>() })
    }
    let leaf = |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq)> {
        (type_id == TypeId::of::<i32>()).then(|| {
            (
                element_dropper_for::<i32>(),
                element_cloner_for::<i32>(),
                element_eq_for::<i32>(),
            )
        })
    };
    let extractors = [
        DynExtractor::Scalar(TypeId::of::<i32>(), extract_i32 as BoxExtractor),
        DynExtractor::Tuple(Box::new(leaf)),
    ];
    let results = seg.call_dyn_tuple_mixed(&[], &extractors)?;
    assert_eq!(results.len(), 2);
    assert_eq!(*results[0].downcast_ref::<i32>().unwrap(), 1);
    let nested = results[1].downcast_ref::<DynamicSequence>().unwrap();
    assert_eq!(nested.arity(), 2);
    let (b, c): (i32, i32) = nested.try_to_tuple()?;
    assert_eq!((b, c), (2, 3));
    Ok(())
}

#[test]
fn call_dyn_tuple_mixed_matches_call_dyn_tuple_for_all_scalar_slots() -> anyhow::Result<()> {
    // Regression: an all-scalar split must behave identically to today's call_dyn_tuple.
    let mut seg = DynSegment::new::<()>();
    let ambient_start = seg.current_stack_offset();
    seg.op0(|| 10u32);
    seg.op0(|| 20u32);
    seg.make_tuple(2, ambient_start);

    fn extract_u32(ptr: *const u8) -> Box<dyn Any> {
        Box::new(unsafe { *ptr.cast::<u32>() })
    }
    let extractors = [
        DynExtractor::Scalar(TypeId::of::<u32>(), extract_u32 as BoxExtractor),
        DynExtractor::Scalar(TypeId::of::<u32>(), extract_u32 as BoxExtractor),
    ];
    let results = seg.call_dyn_tuple_mixed(&[], &extractors)?;
    assert_eq!(*results[0].downcast_ref::<u32>().unwrap(), 10);
    assert_eq!(*results[1].downcast_ref::<u32>().unwrap(), 20);
    Ok(())
}

#[test]
fn call_dyn_tuple_mixed_errors_on_arity_mismatch() {
    let mut seg = DynSegment::new::<()>();
    seg.op0(|| 5u32);
    fn extract_u32(ptr: *const u8) -> Box<dyn Any> {
        Box::new(unsafe { *ptr.cast::<u32>() })
    }
    let extractors = [DynExtractor::Scalar(TypeId::of::<u32>(), extract_u32 as BoxExtractor)];
    let result = seg.call_dyn_tuple_mixed(&[], &extractors);
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime call_dyn_as_dynamic_sequence call_dyn_tuple_mixed`
Expected: FAIL with "no method named `call_dyn_as_dynamic_sequence`"/"`call_dyn_tuple_mixed`",
"cannot find type `DynExtractor`".

- [ ] **Step 3: Implement `call_dyn_as_dynamic_sequence` and its recursive helper**

Add to `cel-runtime/src/dyn_segment.rs`, in `impl DynSegment`, right after `call_dyn_as_tuple`:

```rust
    /// Executes the segment once and moves its tuple result into an owned `DynamicSequence`,
    /// recursing into nested tuple elements as nested `DynamicSequence` leaves. `leaf` supplies
    /// each non-tuple element's `Drop`/`Clone`/`PartialEq` function pointers by its runtime
    /// `TypeId` (`AssociatedType` itself carries only a 2-argument tuple-recursing dropper, not
    /// these three, and this method never calls that dropper — every leaf's bytes are moved, not
    /// cloned, exactly like [`call_dyn_as_tuple`](Self::call_dyn_as_tuple)).
    ///
    /// # Errors
    /// Returns `Err` if:
    /// - The segment requires pre-loaded arguments (created with a non-unit `Args` type).
    /// - The stack does not contain exactly one value after expression compilation.
    /// - That value is not a tuple.
    /// - `leaf` returns `None` for some non-tuple element's `TypeId`.
    /// - Any op returns an error during execution.
    ///
    /// - Complexity: O(n) in the number of ops, plus O(total element count, including nested) to
    ///   build the result.
    pub fn call_dyn_as_dynamic_sequence(
        &mut self,
        inputs: &[&dyn Any],
        leaf: &impl Fn(TypeId) -> Option<(ElementDropper, ElementCloner, ElementEq)>,
    ) -> anyhow::Result<DynamicSequence> {
        ensure!(
            self.argument_ids.is_empty(),
            "call_dyn_as_dynamic_sequence: segment requires {} pre-loaded argument(s); \
             use call_dyn_as_dynamic_sequence only with push_arg-based segments",
            self.argument_ids.len()
        );
        ensure!(
            self.stack_ids.len() == 1,
            "call_dyn_as_dynamic_sequence: expected exactly 1 value on stack, got {}",
            self.stack_ids.len()
        );
        let info = &self.stack_ids[0];
        ensure!(
            info.type_id == TypeId::of::<DynTuple>(),
            "call_dyn_as_dynamic_sequence: expected a tuple result, got {}",
            info.type_name,
        );

        let tuple_size = info.size;
        let tuple_padding = info.padding;
        let associated = info.associated.clone();

        CALL_DYN_PTR.with(|c| c.set(inputs.as_ptr() as usize));
        CALL_DYN_LEN.with(|c| c.set(inputs.len()));
        let _guard = DynCallGuard;

        let mut stack = RawStack::with_base_alignment(self.segment.base_alignment());
        // Safety: the checks above verified the segment builds exactly one tuple value;
        // call_dyn's own argument preconditions (no pre-loaded arguments) hold identically here.
        unsafe {
            self.segment.call0_stack(&mut stack)?;
        }

        let tuple_base = stack.len() - tuple_size;
        let result = unsafe {
            stack.read_at(tuple_base, |base| build_dynamic_sequence(base, &associated, leaf))
        }?;

        // Every leaf at every nesting depth was moved (not cloned) into a fresh
        // DynamicSequence above; the vacated bytes are dead space now, matching
        // call_dyn_as_tuple's own cleanup below it.
        unsafe {
            stack.truncate_to(tuple_base, tuple_padding);
        }

        Ok(result)
    }
```

Add as a free function in `cel-runtime/src/dyn_segment.rs`, right after `call_dyn_as_dynamic_sequence`:

```rust
/// Recursively converts a described tuple region at `base` into an owned `DynamicSequence`,
/// moving each leaf's bytes and recursing into nested tuple elements as nested `DynamicSequence`
/// values.
///
/// # Safety
/// `base` must point to a live value laid out exactly as described by `associated`.
unsafe fn build_dynamic_sequence(
    base: *const u8,
    associated: &[AssociatedType],
    leaf: &impl Fn(TypeId) -> Option<(ElementDropper, ElementCloner, ElementEq)>,
) -> anyhow::Result<DynamicSequence> {
    enum Built {
        Leaf,
        Tuple(DynamicSequence),
    }

    // Pass 1: recursively build nested values and resolve each leaf's descriptor, computing
    // this level's own destination shape/offsets as we go. All fallible work happens here,
    // before any bytes are written.
    let mut shape = Vec::with_capacity(associated.len());
    let mut built: Vec<Built> = Vec::with_capacity(associated.len());
    let mut max_align = 1usize;
    let mut offset = 0usize;
    for elem in associated {
        let is_tuple = elem.type_id == TypeId::of::<DynTuple>();
        let (size, align, drop, clone, eq, value) = if is_tuple {
            let nested =
                unsafe { build_dynamic_sequence(base.add(elem.offset), &elem.associated, leaf)? };
            (
                size_of::<DynamicSequence>(),
                align_of::<DynamicSequence>(),
                element_dropper_for::<DynamicSequence>(),
                element_cloner_for::<DynamicSequence>(),
                element_eq_for::<DynamicSequence>(),
                Built::Tuple(nested),
            )
        } else {
            let (drop, clone, eq) = leaf(elem.type_id).ok_or_else(|| {
                anyhow!(
                    "call_dyn_as_dynamic_sequence: no Clone/PartialEq registered for element \
                     type `{}`",
                    elem.type_name
                )
            })?;
            (elem.size, elem.align, drop, clone, eq, Built::Leaf)
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
        });
        built.push(value);
        offset = aligned + size;
    }
    let total_size = align_index(max_align, offset);

    // Pass 2: write bytes. Infallible -- everything fallible already happened in pass 1.
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

Add `use crate::dynamic_sequence::{DynamicSequence, ElementCloner, ElementDropper, ElementEq,
element_cloner_for, element_dropper_for, element_eq_for};` to `dyn_segment.rs`'s imports (extend
the existing `use crate::dynamic_sequence::{...}` line rather than adding a second one).

Add, right after `build_dynamic_sequence`:

```rust
/// One N>1-output slot's extraction, for [`DynSegment::call_dyn_tuple_mixed`]: either a scalar
/// leaf (the existing [`BoxExtractor`] path, identical to [`call_dyn_tuple`](DynSegment::call_dyn_tuple)),
/// or a nested tuple, converted to a boxed `DynamicSequence` via the same recursive machinery
/// [`call_dyn_as_dynamic_sequence`](DynSegment::call_dyn_as_dynamic_sequence) uses.
pub enum DynExtractor {
    /// A scalar element: `extractors[i].0` must match that element's runtime `TypeId`;
    /// `extractors[i].1` reads and clones it (see [`BoxExtractor`]'s own safety contract).
    Scalar(TypeId, BoxExtractor),
    /// A nested-tuple element: the closure supplies each of *its own* leaves'
    /// `Drop`/`Clone`/`PartialEq` function pointers by `TypeId`, exactly like
    /// `call_dyn_as_dynamic_sequence`'s `leaf` parameter.
    Tuple(Box<dyn Fn(TypeId) -> Option<(ElementDropper, ElementCloner, ElementEq)>>),
}

impl DynSegment {
    /// Executes the segment once and splits its tuple result into one boxed value per element —
    /// a scalar `Box<T>` for a `DynExtractor::Scalar` slot (identical to
    /// [`call_dyn_tuple`](Self::call_dyn_tuple)'s own behavior for an all-scalar split), or a
    /// boxed `DynamicSequence` for a `DynExtractor::Tuple` slot, built from just that one
    /// element's own nested region (not the whole top-of-stack value).
    ///
    /// # Safety
    /// Every `DynExtractor::Scalar`'s `BoxExtractor` must satisfy the same contract
    /// [`call_dyn_tuple`](Self::call_dyn_tuple) requires: clone rather than move.
    ///
    /// # Errors
    /// Returns `Err` if:
    /// - The segment requires pre-loaded arguments.
    /// - The stack does not contain exactly one value after expression compilation.
    /// - That value is not a tuple, or its arity does not equal `extractors.len()`.
    /// - Some element's runtime `TypeId` doesn't match its `DynExtractor::Scalar` type, or (for
    ///   `DynExtractor::Tuple`) the element isn't itself a tuple, or one of *its* leaves has no
    ///   registered descriptor.
    /// - Any op returns an error during execution.
    pub unsafe fn call_dyn_tuple_mixed(
        &mut self,
        inputs: &[&dyn Any],
        extractors: &[DynExtractor],
    ) -> anyhow::Result<Vec<Box<dyn Any>>> {
        ensure!(
            self.argument_ids.is_empty(),
            "call_dyn_tuple_mixed: segment requires {} pre-loaded argument(s); \
             use call_dyn_tuple_mixed only with push_arg-based segments",
            self.argument_ids.len()
        );
        ensure!(
            self.stack_ids.len() == 1,
            "call_dyn_tuple_mixed: expected exactly 1 value on stack, got {}",
            self.stack_ids.len()
        );
        let info = &self.stack_ids[0];
        ensure!(
            info.type_id == TypeId::of::<DynTuple>(),
            "call_dyn_tuple_mixed: expected a tuple result, got {}",
            info.type_name,
        );
        ensure!(
            info.associated.len() == extractors.len(),
            "call_dyn_tuple_mixed: tuple has {} element(s) but {} extractor(s) were supplied",
            info.associated.len(),
            extractors.len(),
        );
        for (i, (elem, extractor)) in info.associated.iter().zip(extractors).enumerate() {
            if let DynExtractor::Scalar(expected_type_id, _) = extractor {
                ensure!(
                    elem.type_id == *expected_type_id,
                    "call_dyn_tuple_mixed: element {i} type mismatch: expected type {:?}, got `{}`",
                    expected_type_id,
                    elem.type_name,
                );
            }
        }

        let tuple_size = info.size;
        let tuple_padding = info.padding;
        let associated = info.associated.clone();

        CALL_DYN_PTR.with(|c| c.set(inputs.as_ptr() as usize));
        CALL_DYN_LEN.with(|c| c.set(inputs.len()));
        let _guard = DynCallGuard;

        let mut stack = RawStack::with_base_alignment(self.segment.base_alignment());
        // Safety: the checks above verified the segment builds exactly one tuple value with
        // `extractors.len()` matching elements; call_dyn's own argument preconditions (no
        // pre-loaded arguments) hold identically here.
        unsafe {
            self.segment.call0_stack(&mut stack)?;
        }

        let tuple_base = stack.len() - tuple_size;
        let results: anyhow::Result<Vec<Box<dyn Any>>> = associated
            .iter()
            .zip(extractors)
            .map(|(elem, extractor)| match extractor {
                DynExtractor::Scalar(_, boxextractor) => Ok(unsafe {
                    stack.read_at(tuple_base + elem.offset, |ptr| boxextractor(ptr))
                }),
                DynExtractor::Tuple(leaf) => {
                    ensure!(
                        elem.type_id == TypeId::of::<DynTuple>(),
                        "call_dyn_tuple_mixed: expected a nested tuple, got `{}`",
                        elem.type_name,
                    );
                    let nested = unsafe {
                        stack.read_at(tuple_base + elem.offset, |base| {
                            build_dynamic_sequence(base, &elem.associated, leaf.as_ref())
                        })
                    }?;
                    Ok(Box::new(nested) as Box<dyn Any>)
                }
            })
            .collect();
        let results = results?;

        // Every DynExtractor::Scalar element's bytes were only cloned above (BoxExtractor's own
        // contract, matching call_dyn_tuple) -- those must still be dropped normally. Every
        // DynExtractor::Tuple element's bytes were *moved* into the nested DynamicSequence above
        // (build_dynamic_sequence's contract, matching call_dyn_as_dynamic_sequence) -- running
        // that element's own dropper again would double-drop/use-after-move, so its dropper is
        // replaced with a no-op for this cleanup pass only (mirroring how
        // DynamicSequence::try_into_tuple clears its own `shape` to make its `Drop` a no-op after
        // moving every element out).
        let drop_associated: Vec<AssociatedType> = associated
            .iter()
            .zip(extractors)
            .map(|(elem, extractor)| match extractor {
                DynExtractor::Scalar(..) => elem.clone(),
                DynExtractor::Tuple(_) => AssociatedType {
                    dropper: |_ptr, _associated| {},
                    ..elem.clone()
                },
            })
            .collect();
        unsafe {
            stack.drop_at(tuple_base, |ptr| drop_tuple(ptr, &drop_associated));
            stack.truncate_to(tuple_base, tuple_padding);
        }

        Ok(results)
    }
}
```

Add a drop-safety test to Step 1's test block (alongside the others already there, before Step 2
runs):

```rust
#[test]
fn call_dyn_tuple_mixed_drops_a_moved_out_tuple_slot_exactly_once() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone, PartialEq)]
    struct DropCounter(Arc<AtomicUsize>);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let count = Arc::new(AtomicUsize::new(0));
    let mut seg = DynSegment::new::<()>();
    let ambient_start = seg.current_stack_offset();
    let inner_start = seg.current_stack_offset();
    let a = DropCounter(count.clone());
    seg.op0(move || a.clone());
    seg.op0(|| 7i32);
    seg.make_tuple(2, inner_start);
    seg.make_tuple(1, ambient_start);

    let leaf = |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq)> {
        if type_id == TypeId::of::<DropCounter>() {
            Some((
                element_dropper_for::<DropCounter>(),
                element_cloner_for::<DropCounter>(),
                element_eq_for::<DropCounter>(),
            ))
        } else if type_id == TypeId::of::<i32>() {
            Some((
                element_dropper_for::<i32>(),
                element_cloner_for::<i32>(),
                element_eq_for::<i32>(),
            ))
        } else {
            None
        }
    };
    let extractors = [DynExtractor::Tuple(Box::new(leaf))];
    let results = unsafe { seg.call_dyn_tuple_mixed(&[], &extractors) }.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 0, "moving out must not drop the element");
    drop(results);
    assert_eq!(count.load(Ordering::SeqCst), 1, "must still drop exactly once, not zero or twice");
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime dyn_segment::`
Expected: PASS (all tests in the module, including the 9 new ones — 5 for
`call_dyn_as_dynamic_sequence`, 4 for `call_dyn_tuple_mixed`).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dyn_segment.rs
git commit -m "feat(cel-runtime): add call_dyn_as_dynamic_sequence and call_dyn_tuple_mixed"
```

---

### Task 4: `DynamicSequence::from_dyn_elements` — building from boxed defaults

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`

**Interfaces:**
- Consumes: `element_writer_for` (Task 1); `DynamicSequence::from_raw_parts` (Task 1).
- Produces: `pub struct DynElementSpec { pub type_id: TypeId, pub type_name: Cow<'static, str>,
  pub size: usize, pub align: usize, pub drop: ElementDropper, pub clone: ElementCloner, pub eq:
  ElementEq, pub write: unsafe fn(Box<dyn Any>, *mut u8) }`,
  `DynamicSequence::from_dyn_elements(elements: Vec<(DynElementSpec, Box<dyn Any>)>) -> Self`.

Used only when there's no CEL expression to evaluate (a tuple-typed cell with no initializer):
each leaf's already-boxed default value (adam-lang's existing `TypeEntry::default_fn`, unchanged)
is moved into a fresh `DynamicSequence`, recursing for nested tuple defaults by building the inner
`DynamicSequence` first and boxing *that*.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dynamic_sequence.rs`:

```rust
#[test]
fn from_dyn_elements_builds_a_matching_sequence() {
    let spec_i32 = DynElementSpec {
        type_id: TypeId::of::<i32>(),
        type_name: Cow::Borrowed("i32"),
        size: size_of::<i32>(),
        align: align_of::<i32>(),
        drop: element_dropper_for::<i32>(),
        clone: element_cloner_for::<i32>(),
        eq: element_eq_for::<i32>(),
        write: element_writer_for::<i32>(),
    };
    let spec_f64 = DynElementSpec {
        type_id: TypeId::of::<f64>(),
        type_name: Cow::Borrowed("f64"),
        size: size_of::<f64>(),
        align: align_of::<f64>(),
        drop: element_dropper_for::<f64>(),
        clone: element_cloner_for::<f64>(),
        eq: element_eq_for::<f64>(),
        write: element_writer_for::<f64>(),
    };
    let seq = DynamicSequence::from_dyn_elements(vec![
        (spec_i32, Box::new(3i32) as Box<dyn std::any::Any>),
        (spec_f64, Box::new(4.5f64) as Box<dyn std::any::Any>),
    ]);
    assert_eq!(seq.arity(), 2);
    let (a, b): (i32, f64) = seq.try_to_tuple().unwrap();
    assert_eq!((a, b), (3, 4.5));
}

#[test]
fn from_dyn_elements_moves_boxed_values_without_double_dropping() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone, PartialEq)]
    struct DropCounter(Arc<AtomicUsize>);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let count = Arc::new(AtomicUsize::new(0));
    let spec = DynElementSpec {
        type_id: TypeId::of::<DropCounter>(),
        type_name: Cow::Borrowed("DropCounter"),
        size: size_of::<DropCounter>(),
        align: align_of::<DropCounter>(),
        drop: element_dropper_for::<DropCounter>(),
        clone: element_cloner_for::<DropCounter>(),
        eq: element_eq_for::<DropCounter>(),
        write: element_writer_for::<DropCounter>(),
    };
    let boxed: Box<dyn std::any::Any> = Box::new(DropCounter(count.clone()));
    let seq = DynamicSequence::from_dyn_elements(vec![(spec, boxed)]);
    assert_eq!(count.load(Ordering::SeqCst), 0);
    drop(seq);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime from_dyn_elements`
Expected: FAIL with "cannot find struct `DynElementSpec`"/"no function `from_dyn_elements`".

- [ ] **Step 3: Implement `DynElementSpec` and `from_dyn_elements`**

Add to `cel-runtime/src/dynamic_sequence.rs`, right after the `SequenceElement` struct:

```rust
/// One element to be moved into a `DynamicSequence` being built from already-boxed values (used
/// when no CEL expression exists to evaluate — e.g. a tuple-typed cell's default value).
pub struct DynElementSpec {
    /// Runtime type id for this element.
    pub type_id: TypeId,
    /// Human-readable name for error reporting.
    pub type_name: Cow<'static, str>,
    /// Size in bytes of this element's value.
    pub size: usize,
    /// Required alignment in bytes of this element's value.
    pub align: usize,
    /// In-place dropper for this element.
    pub drop: ElementDropper,
    /// In-place cloner for this element.
    pub clone: ElementCloner,
    /// Equality comparator for this element.
    pub eq: ElementEq,
    /// Moves the boxed value's bytes to a destination pointer, consuming the box without
    /// dropping its contents.
    ///
    /// # Safety
    /// The destination must be valid for writes of `size` bytes at `align`; the boxed value's
    /// runtime type must match the type this `write` was generated for.
    pub write: unsafe fn(Box<dyn std::any::Any>, *mut u8),
}
```

Add to `impl DynamicSequence`, right after `arity`:

```rust
    /// Builds a sequence by moving each boxed value into a fresh buffer, per its own descriptor.
    ///
    /// - Complexity: O(n) time; one heap allocation for the sequence's own buffer.
    #[must_use]
    pub fn from_dyn_elements(elements: Vec<(DynElementSpec, Box<dyn std::any::Any>)>) -> Self {
        let mut shape = Vec::with_capacity(elements.len());
        let mut max_align = 1usize;
        let mut offset = 0usize;
        for (spec, _) in &elements {
            let aligned = align_index(spec.align, offset);
            max_align = max_align.max(spec.align);
            shape.push(SequenceElement {
                type_id: spec.type_id,
                type_name: spec.type_name.clone(),
                offset: aligned,
                size: spec.size,
                align: spec.align,
                drop: spec.drop,
                clone: spec.clone,
                eq: spec.eq,
            });
            offset = aligned + spec.size;
        }
        let total_size = align_index(max_align, offset);

        let mut buffer = crate::raw_stack::RawStack::with_base_alignment(max_align);
        unsafe {
            buffer.reserve_and_write(max_align, total_size, |dst| {
                for ((spec, value), elem) in elements.into_iter().zip(&shape) {
                    (spec.write)(value, dst.add(elem.offset));
                }
            });
        }
        unsafe { DynamicSequence::from_raw_parts(buffer, shape, max_align) }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime dynamic_sequence::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dynamic_sequence.rs
git commit -m "feat(cel-runtime): add DynamicSequence::from_dyn_elements for default-value construction"
```

---

### Task 5: `DynSegment::push_arg_as_dynamic_sequence_tuple` — input direction

**Files:**
- Modify: `cel-runtime/src/dyn_segment.rs`

**Interfaces:**
- Consumes: `layout_associated` (Task 2); `DynamicSequence::shape`/`read_element_at` (Task 1);
  `drop_tuple` (now `pub`, Task 1); existing `push_arg` pattern (`CALL_DYN_PTR`/`CALL_DYN_LEN`,
  `self.segment.raw0_`).
- Produces: `DynSegment::push_arg_as_dynamic_sequence_tuple(&mut self, index: usize, associated:
  Vec<AssociatedType>)`.

The reverse of Task 3: given a *declared* shape (built by the caller, recursively, from a
`TypeShape` — offsets in `associated` are ignored/overwritten via `layout_associated`), this
clones a stored `DynamicSequence` input's bytes onto the segment's stack as a live, tagged
`DynTuple`, so ordinary CEL tuple indexing (`.0`, `.1`) and tuple-shaped operators work on a
tuple-typed input cell exactly as they would on an inline tuple literal. A nested `DynamicSequence`
leaf (Task 3's representation for a nested tuple) is recursively expanded back into its own nested
on-stack tuple region — the exact inverse of Task 3's "nested tuple → nested `DynamicSequence`"
conversion.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dyn_segment.rs`:

```rust
#[test]
fn push_arg_as_dynamic_sequence_tuple_supports_tuple_indexing() -> anyhow::Result<()> {
    let seq = DynamicSequence::from_tuple((10i32, 2.5f64));
    let mut seg = DynSegment::new::<()>();
    let shape = vec![
        AssociatedType {
            type_id: TypeId::of::<i32>(),
            type_name: Cow::Borrowed("i32"),
            offset: 0,
            size: size_of::<i32>(),
            align: align_of::<i32>(),
            dropper: raw_dropper_for::<i32>(),
            associated: Vec::new(),
        },
        AssociatedType {
            type_id: TypeId::of::<f64>(),
            type_name: Cow::Borrowed("f64"),
            offset: 0,
            size: size_of::<f64>(),
            align: align_of::<f64>(),
            dropper: raw_dropper_for::<f64>(),
            associated: Vec::new(),
        },
    ];
    seg.push_arg_as_dynamic_sequence_tuple(0, shape);
    assert_eq!(seg.peek_tuple_arity(), Some(2));
    seg.tuple_index(1);
    let result: f64 = seg.call_dyn(&[&seq as &dyn Any])?;
    assert_eq!(result, 2.5);
    Ok(())
}

#[test]
fn push_arg_as_dynamic_sequence_tuple_recurses_into_nested_tuples() -> anyhow::Result<()> {
    // Build the same shape call_dyn_as_dynamic_sequence's nesting test produces: (i32, (i32,i32)).
    let mut source = DynSegment::new::<()>();
    let ambient_start = source.current_stack_offset();
    source.op0(|| 1i32);
    let inner_start = source.current_stack_offset();
    source.op0(|| 2i32);
    source.op0(|| 3i32);
    source.make_tuple(2, inner_start);
    source.make_tuple(2, ambient_start);
    let leaf = |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq)> {
        (type_id == TypeId::of::<i32>()).then(|| {
            (
                element_dropper_for::<i32>(),
                element_cloner_for::<i32>(),
                element_eq_for::<i32>(),
            )
        })
    };
    let seq = source.call_dyn_as_dynamic_sequence(&[], &leaf)?;

    let inner_shape = vec![
        AssociatedType {
            type_id: TypeId::of::<i32>(),
            type_name: Cow::Borrowed("i32"),
            offset: 0,
            size: size_of::<i32>(),
            align: align_of::<i32>(),
            dropper: raw_dropper_for::<i32>(),
            associated: Vec::new(),
        },
        AssociatedType {
            type_id: TypeId::of::<i32>(),
            type_name: Cow::Borrowed("i32"),
            offset: 0,
            size: size_of::<i32>(),
            align: align_of::<i32>(),
            dropper: raw_dropper_for::<i32>(),
            associated: Vec::new(),
        },
    ];
    let outer_shape = vec![
        AssociatedType {
            type_id: TypeId::of::<i32>(),
            type_name: Cow::Borrowed("i32"),
            offset: 0,
            size: size_of::<i32>(),
            align: align_of::<i32>(),
            dropper: raw_dropper_for::<i32>(),
            associated: Vec::new(),
        },
        AssociatedType {
            type_id: TypeId::of::<DynTuple>(),
            type_name: Cow::Borrowed("tuple"),
            offset: 0,
            size: 0,
            align: 1,
            dropper: drop_tuple,
            associated: inner_shape,
        },
    ];

    let mut seg = DynSegment::new::<()>();
    seg.push_arg_as_dynamic_sequence_tuple(0, outer_shape);
    seg.tuple_index(1); // the nested (i32, i32)
    seg.tuple_index(0); // its first element
    let result: i32 = seg.call_dyn(&[&seq as &dyn Any])?;
    assert_eq!(result, 2);
    Ok(())
}

#[test]
fn push_arg_as_dynamic_sequence_tuple_clones_leaving_the_input_usable() -> anyhow::Result<()> {
    let seq = DynamicSequence::from_tuple((1i32, 2i32));
    let shape = || {
        vec![
            AssociatedType {
                type_id: TypeId::of::<i32>(),
                type_name: Cow::Borrowed("i32"),
                offset: 0,
                size: size_of::<i32>(),
                align: align_of::<i32>(),
                dropper: raw_dropper_for::<i32>(),
                associated: Vec::new(),
            },
            AssociatedType {
                type_id: TypeId::of::<i32>(),
                type_name: Cow::Borrowed("i32"),
                offset: 0,
                size: size_of::<i32>(),
                align: align_of::<i32>(),
                dropper: raw_dropper_for::<i32>(),
                associated: Vec::new(),
            },
        ]
    };

    let mut seg_a = DynSegment::new::<()>();
    seg_a.push_arg_as_dynamic_sequence_tuple(0, shape());
    seg_a.tuple_index(0);
    let a: i32 = seg_a.call_dyn(&[&seq as &dyn Any])?;

    let mut seg_b = DynSegment::new::<()>();
    seg_b.push_arg_as_dynamic_sequence_tuple(0, shape());
    seg_b.tuple_index(1);
    let b: i32 = seg_b.call_dyn(&[&seq as &dyn Any])?;

    assert_eq!((a, b), (1, 2));
    Ok(())
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime push_arg_as_dynamic_sequence_tuple`
Expected: FAIL with "no method named `push_arg_as_dynamic_sequence_tuple`".

- [ ] **Step 3: Implement `push_arg_as_dynamic_sequence_tuple` and its recursive helper**

Add to `cel-runtime/src/dyn_segment.rs`, in `impl DynSegment`, right after `push_arg`:

```rust
    /// Emits an op that clones `inputs[index]` (downcast to `&DynamicSequence`) onto the stack
    /// as a live, tagged `DynTuple`, so ordinary CEL tuple indexing/operators work on a
    /// tuple-typed input cell exactly as they would on an inline tuple literal. `associated`
    /// describes the expected (declared) element types, recursively — a nested tuple element's
    /// own `associated` describes its inner shape the same way, and is expanded back into a
    /// nested on-stack tuple region (the inverse of
    /// [`call_dyn_as_dynamic_sequence`](Self::call_dyn_as_dynamic_sequence)'s "nested tuple →
    /// nested `DynamicSequence`" conversion). Offsets in `associated` are ignored and overwritten
    /// internally via [`layout_associated`] — callers only need to supply each element's
    /// `type_id`/`type_name`/`size`/`align`/`dropper` (or, for a nested tuple element,
    /// `type_id: TypeId::of::<DynTuple>()` with its own recursively-built `associated`).
    ///
    /// - Precondition: every call to a `call_dyn`-family execution supplies an `inputs` slice
    ///   where `inputs[index]` is a `DynamicSequence` whose own shape matches `associated`
    ///   exactly (same arity and element `TypeId`s, recursively).
    ///
    /// - Complexity: O(1) to register the op; the op itself is O(total element count, including
    ///   nested) at execution time.
    pub fn push_arg_as_dynamic_sequence_tuple(&mut self, index: usize, mut associated: Vec<AssociatedType>) {
        let (total_size, tuple_align) = layout_associated(&mut associated);
        let ambient_start = self.current_stack_offset();
        let dest_base = align_index(tuple_align, ambient_start);

        let write_shape = associated.clone();
        self.segment.raw0_(move |stack| {
            CALL_DYN_PTR.with(|ptr_cell| {
                CALL_DYN_LEN.with(|len_cell| -> anyhow::Result<()> {
                    let raw_ptr = ptr_cell.get() as *const &dyn Any;
                    let len = len_cell.get();
                    assert!(
                        !raw_ptr.is_null(),
                        "push_arg_as_dynamic_sequence_tuple invoked outside call_dyn"
                    );
                    debug_assert!(
                        index < len,
                        "push_arg_as_dynamic_sequence_tuple index {index} out of range {len}"
                    );
                    // Safety: raw_ptr is non-null (checked above) and valid for the duration of
                    // the enclosing call_dyn call; DynCallGuard clears it on return.
                    let slice = unsafe { std::slice::from_raw_parts(raw_ptr, len) };
                    let seq = slice[index]
                        .downcast_ref::<DynamicSequence>()
                        .expect("push_arg_as_dynamic_sequence_tuple: type mismatch at runtime");
                    unsafe {
                        stack.reserve_and_write(tuple_align, total_size, |dst| {
                            write_dynamic_sequence_as_tuple(seq, &write_shape, dst);
                        });
                    }
                    Ok(())
                })
            })
        });

        self.stack_ids.push(StackInfo {
            type_id: TypeId::of::<DynTuple>(),
            type_name: Cow::Borrowed(std::any::type_name::<DynTuple>()),
            padding: dest_base != ambient_start,
            size: total_size,
            align: tuple_align,
            raw_dropper: drop_tuple,
            associated,
        });
    }
```

Add as a free function in `cel-runtime/src/dyn_segment.rs`, right after
`push_arg_as_dynamic_sequence_tuple`:

```rust
/// Writes `seq`'s elements into `dst`, at the offsets in `dest_shape` (already computed via
/// [`layout_associated`]), cloning each leaf and recursively expanding each nested-tuple element
/// into its own nested on-stack tuple region.
///
/// # Safety
/// `dst` must be valid for writes covering every offset + size in `dest_shape`; `dest_shape`'s
/// element count and per-element shape (leaf vs. nested tuple, recursively) must match `seq`'s
/// own shape exactly.
unsafe fn write_dynamic_sequence_as_tuple(
    seq: &DynamicSequence,
    dest_shape: &[AssociatedType],
    dst: *mut u8,
) {
    for (dest_elem, src_elem) in dest_shape.iter().zip(seq.shape()) {
        if dest_elem.type_id == TypeId::of::<DynTuple>() {
            seq.read_element_at(src_elem.offset, |src| unsafe {
                let nested = &*src.cast::<DynamicSequence>();
                write_dynamic_sequence_as_tuple(
                    nested,
                    &dest_elem.associated,
                    dst.add(dest_elem.offset),
                );
            });
        } else {
            seq.read_element_at(src_elem.offset, |src| unsafe {
                (src_elem.clone)(src, dst.add(dest_elem.offset));
            });
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime dyn_segment::`
Expected: PASS (all tests in the module, including the 3 new ones).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dyn_segment.rs
git commit -m "feat(cel-runtime): add DynSegment::push_arg_as_dynamic_sequence_tuple"
```

---

### Task 6: Full workspace verification

**Files:** none (verification only).

- [ ] **Step 1: Run the full test suite**

Run: `cargo test --workspace` and `cargo test --doc --workspace`.
Expected: PASS, zero compiler warnings in the output (per the root `CLAUDE.md`: a plain
build/test compile must produce zero warnings, which `-D warnings` clippy runs don't fully
cover — e.g. an unused `mut`).

- [ ] **Step 2: Run all three clippy invocations**

Run, in order:
```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: PASS (no warnings). Fix any findings before proceeding — this plan only touches
`cel-runtime`, so a `begin`-related warning here would indicate an unrelated pre-existing issue;
confirm with `git stash` whether it predates this plan's changes before investigating further.

- [ ] **Step 3: Format**

Run: `cargo fmt --all`. Commit any formatting-only changes if `cargo fmt --all --check` reports
diffs not already committed.

- [ ] **Step 4: Final commit**

If steps 1–3 required any fixes, commit them:
```bash
git add -A
git commit -m "fix(cel-runtime): address workspace-wide lint/warning findings"
```
(Skip this step if nothing needed fixing.)
