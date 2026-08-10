# DynamicSequence Tuple Cells Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `DynamicSequence` type to `cel-runtime` that lets an `adam-rs` cell hold a CEL
tuple value of arbitrary (compile-time-known, for this plan) shape, converts type-safely to/from
concrete Rust tuples (nestable, arity 1–12), and can be extracted from a live `DynSegment`
evaluation — all demonstrated end-to-end through `adam-rs`'s existing, unmodified `Sheet`/`Method`/
`Condition` API.

**Architecture:** `DynamicSequence` owns a `RawStack` (sized to its own elements' actual max
alignment, never a blanket constant) plus a `Vec<SequenceElement>` shape descriptor carrying each
element's `TypeId`, offset, size, align, and per-element `drop`/`clone`/`eq` function pointers. A
new `TupleSequence` trait (hand-written per arity, mirroring `list_traits.rs`'s existing `IntoList`
convention) converts a concrete Rust tuple to/from that byte layout. `DynSegment` gains a method to
reconstruct a live on-stack CEL tuple result directly as a concrete Rust tuple. `adam-rs` itself is
not modified at all — its `Sheet`/`Method`/`Condition` API is already fully generic over
`Any + PartialEq + 'static`.

**Tech Stack:** Rust, `cel-runtime` (`RawStack`, `DynSegment`), `anyhow` for fallible ops,
`adam-rs` (dev-dependency only, for the acceptance tests).

**Reference:** `docs/superpowers/specs/2026-08-10-dynamic-sequence-tuple-cells-design.md`.

## Global Constraints

- Format with `cargo fmt --all` before every commit (enforced by pre-commit hook).
- Every function/trait/struct needs a contract-style `///` doc comment (Summary, Preconditions as
  `debug_assert!`, `# Errors`/`# Safety` where applicable, Postconditions, Complexity if not O(1))
  per the root `CLAUDE.md`. Trait *impls* don't repeat the trait's own doc comments (matching
  `list_traits.rs`'s existing `IntoList` impls).
- Unit tests are derived from contract/public interface only — never from implementation internals.
- Run `cargo test -p cel-runtime` after every task's implementation step; run the full
  `cargo test --workspace` and `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
  before the final commit of the whole plan (Task 13).
- No heap allocation beyond exactly one per `DynamicSequence` (its own `RawStack`'s backing buffer).

---

### Task 1: `RawStack::reserve_and_write`

**Files:**
- Modify: `cel-runtime/src/raw_stack.rs`

**Interfaces:**
- Produces: `unsafe fn RawStack::reserve_and_write(&mut self, align: usize, size: usize, write: impl FnOnce(*mut u8)) -> bool`.

`DynamicSequence` needs to write each tuple element via a per-element *clone function pointer*
(for `Clone`), not by copying already-formed bytes from a source (`push_raw` only supports the
latter). This task adds the write-via-callback primitive, mirroring `push_raw`'s existing
alignment/padding bookkeeping exactly.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/raw_stack.rs`:

```rust
#[test]
fn reserve_and_write_matches_push_raw_padding_and_bytes() {
    let mut stack_a = RawStack::with_base_alignment(align_of::<f64>());
    let _ = stack_a.push(1u8);
    let value = 2.5f64;
    let padding_a = unsafe {
        stack_a.push_raw(
            align_of::<f64>(),
            size_of::<f64>(),
            (&value as *const f64).cast::<MaybeUninit<u8>>(),
        )
    };

    let mut stack_b = RawStack::with_base_alignment(align_of::<f64>());
    let _ = stack_b.push(1u8);
    let padding_b = unsafe {
        stack_b.reserve_and_write(align_of::<f64>(), size_of::<f64>(), |dst| {
            std::ptr::write(dst.cast::<f64>(), 2.5f64);
        })
    };

    assert_eq!(padding_a, padding_b);
    let popped_a: f64 = unsafe { stack_a.pop(padding_a) };
    let popped_b: f64 = unsafe { stack_b.pop(padding_b) };
    assert_eq!(popped_a, popped_b);
}

#[test]
fn reserve_and_write_runs_write_exactly_once_with_correct_size() {
    let mut stack = RawStack::with_base_alignment(align_of::<u32>());
    let mut call_count = 0;
    let padding = unsafe {
        stack.reserve_and_write(align_of::<u32>(), size_of::<u32>(), |dst| {
            call_count += 1;
            std::ptr::write(dst.cast::<u32>(), 42);
        })
    };
    assert_eq!(call_count, 1);
    let result: u32 = unsafe { stack.pop(padding) };
    assert_eq!(result, 42);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime raw_stack:: -- --exact reserve_and_write_matches_push_raw_padding_and_bytes reserve_and_write_runs_write_exactly_once_with_correct_size`
Expected: FAIL with "no method named `reserve_and_write`".

- [ ] **Step 3: Implement `reserve_and_write`**

Add to `impl RawStack` in `cel-runtime/src/raw_stack.rs`, immediately after `push_raw`:

```rust
    /// Reserves aligned space for `size` bytes (using the same padding/marker
    /// bookkeeping as [`push`](Self::push)) and calls `write` with a pointer
    /// to that space instead of copying from a source buffer.
    ///
    /// - Precondition: `align` is a power of two.
    /// - Postcondition: `write` is called exactly once, before this method
    ///   returns, with a pointer to `size` freshly-reserved bytes.
    ///
    /// # Safety
    /// `write` must fully initialize all `size` bytes at the pointer it's
    /// given before returning.
    pub unsafe fn reserve_and_write(
        &mut self,
        align: usize,
        size: usize,
        write: impl FnOnce(*mut u8),
    ) -> bool {
        debug_assert!(align.is_power_of_two());
        let len = self.buffer.len();
        let aligned_index = align_index(align, len);
        let new_len = aligned_index + size;

        self.buffer.reserve(new_len - len);
        unsafe {
            self.buffer.set_len(new_len);
            if aligned_index - len > 0 {
                self.buffer[len].write(1);
                self.buffer[len + 1..aligned_index].fill(MaybeUninit::new(0));
            }
            write(self.buffer.as_mut_ptr().add(aligned_index).cast::<u8>());
        }
        aligned_index - len > 0
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime raw_stack::`
Expected: PASS (all tests in the module, including the 2 new ones).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/raw_stack.rs
git commit -m "feat(cel-runtime): add RawStack::reserve_and_write for write-via-callback construction"
```

---

### Task 2: `dynamic_sequence` module skeleton — `SequenceElement` and `push_element`

**Files:**
- Create: `cel-runtime/src/dynamic_sequence.rs`
- Modify: `cel-runtime/src/lib.rs` (register the module)

**Interfaces:**
- Produces: `pub type ElementDropper = unsafe fn(*mut u8)`, `pub type ElementCloner = unsafe fn(*const u8, *mut u8)`, `pub type ElementEq = unsafe fn(*const u8, *const u8) -> bool`, `pub struct SequenceElement { type_id, type_name, offset, size, align, drop, clone, eq }`, private `fn push_element<T: 'static + Clone + PartialEq>(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize`.

- [ ] **Step 1: Write the failing tests**

Create `cel-runtime/src/dynamic_sequence.rs` with this content (module doc, imports, and a `tests`
module only — no production code yet):

```rust
//! `DynamicSequence`: an owned, type-erased CEL tuple value that persists beyond any single
//! `DynSegment` evaluation, and can be stored directly as an `adam_rs::Sheet` cell's value.
//!
//! Converts type-safely to and from concrete, nestable Rust tuples via the [`TupleSequence`]
//! trait, implemented for arities 1 through 12.

use crate::memory::align_index;
use std::any::TypeId;
use std::borrow::Cow;

/// Drops a value in place, given a pointer to its bytes.
///
/// # Safety
/// `ptr` must point to a valid, live, properly aligned value of the type this dropper was
/// generated for.
pub type ElementDropper = unsafe fn(*mut u8);

/// Clones a value in place: reads a live value at `src`, writes a fresh clone of it at `dst`.
///
/// # Safety
/// `src` must point to a valid, live, properly aligned value of the type this cloner was
/// generated for; `dst` must be valid for writes of that same type's size and alignment.
pub type ElementCloner = unsafe fn(*const u8, *mut u8);

/// Compares two values of the same type for equality.
///
/// # Safety
/// `a` and `b` must each point to a valid, live, properly aligned value of the type this
/// comparator was generated for.
pub type ElementEq = unsafe fn(*const u8, *const u8) -> bool;

/// Describes one element of a [`DynamicSequence`]: its type identity, byte layout, and
/// in-place drop/clone/equality functions.
#[derive(Clone)]
pub struct SequenceElement {
    /// Runtime type id for this element.
    pub type_id: TypeId,
    /// Human-readable name for error reporting.
    pub type_name: Cow<'static, str>,
    /// Byte offset from the start of the enclosing sequence.
    pub offset: usize,
    /// Size in bytes of this element's value.
    pub size: usize,
    /// Required alignment in bytes of this element's value.
    pub align: usize,
    /// In-place dropper for this element, callable at its own start address.
    pub drop: ElementDropper,
    /// In-place cloner for this element.
    pub clone: ElementCloner,
    /// Equality comparator for this element.
    pub eq: ElementEq,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

    #[test]
    fn push_element_records_layout_and_generates_working_fn_pointers() {
        let mut out = Vec::new();
        let mut max_align = 1usize;
        let next = push_element::<i32>(&mut out, 0, &mut max_align);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].type_id, TypeId::of::<i32>());
        assert_eq!(out[0].offset, 0);
        assert_eq!(out[0].size, size_of::<i32>());
        assert_eq!(out[0].align, align_of::<i32>());
        assert_eq!(next, size_of::<i32>());
        assert_eq!(max_align, align_of::<i32>());

        let mut value = 7i32;
        let mut cloned = 0i32;
        unsafe {
            (out[0].clone)(
                (&raw mut value).cast::<u8>(),
                (&raw mut cloned).cast::<u8>(),
            );
        }
        assert_eq!(cloned, 7);
        assert!(unsafe {
            (out[0].eq)(
                (&raw const value).cast::<u8>(),
                (&raw const cloned).cast::<u8>(),
            )
        });
        unsafe { (out[0].drop)((&raw mut value).cast::<u8>()) };
    }

    #[test]
    fn push_element_computes_alignment_padding_between_calls() {
        let mut out = Vec::new();
        let mut max_align = 1usize;
        let after_u8 = push_element::<u8>(&mut out, 0, &mut max_align);
        let after_u32 = push_element::<u32>(&mut out, after_u8, &mut max_align);

        assert_eq!(out[0].offset, 0);
        assert_eq!(out[1].offset, 4); // u8 at [0,1); u32 aligned up to 4
        assert_eq!(after_u32, 8);
        assert_eq!(max_align, 4);
    }
}
```

Add to `cel-runtime/src/lib.rs`, alongside the other `pub mod` declarations:

```rust
/// Owned, type-erased CEL tuple value that can outlive a `DynSegment` evaluation.
pub mod dynamic_sequence;
```

and to the `pub use` block:

```rust
pub use dynamic_sequence::*;
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime dynamic_sequence::`
Expected: FAIL with "cannot find function `push_element`".

- [ ] **Step 3: Implement `push_element`**

Add to `cel-runtime/src/dynamic_sequence.rs`, after the `SequenceElement` struct definition (before
the `tests` module):

```rust
/// Computes the next aligned offset for a `'static + Clone + PartialEq` field of type `T`,
/// appends its [`SequenceElement`] to `out`, folds `T`'s alignment into `*max_align`, and returns
/// the byte position immediately after this element.
///
/// - Complexity: O(1).
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
        drop: |ptr| unsafe { std::ptr::drop_in_place(ptr.cast::<T>()) },
        clone: |src, dst| unsafe { std::ptr::write(dst.cast::<T>(), (*src.cast::<T>()).clone()) },
        eq: |a, b| unsafe { *a.cast::<T>() == *b.cast::<T>() },
    });
    aligned_offset + size_of::<T>()
}
```

Add `use std::mem::{align_of, size_of};` to the imports at the top of the file.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime dynamic_sequence::`
Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dynamic_sequence.rs cel-runtime/src/lib.rs
git commit -m "feat(cel-runtime): add dynamic_sequence module with SequenceElement and push_element"
```

---

### Task 3: `TupleSequence` trait + arities 1–3

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`

**Interfaces:**
- Consumes: `push_element`, `SequenceElement` (Task 2).
- Produces: `pub trait TupleSequence { fn append_shape(...) -> usize; unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]); unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self; unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self; }`, implemented for `(A,)`, `(A, B)`, `(A, B, C)`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dynamic_sequence.rs`:

```rust
    #[test]
    fn arity_1_shape_write_read_clone_round_trip() {
        let mut shape = Vec::new();
        let mut max_align = 1usize;
        <(i32,)>::append_shape(&mut shape, 0, &mut max_align);
        let offsets: Vec<usize> = shape.iter().map(|e| e.offset).collect();

        let mut buf = [0u8; 4];
        unsafe { (42i32,).write_into(buf.as_mut_ptr(), &offsets) };
        let cloned: (i32,) = unsafe { <(i32,)>::clone_from(buf.as_ptr(), &offsets) };
        assert_eq!(cloned, (42,));
        let read: (i32,) = unsafe { <(i32,)>::read_from(buf.as_ptr(), &offsets) };
        assert_eq!(read, (42,));
    }

    #[test]
    fn arity_2_shape_write_read_clone_round_trip() {
        let mut shape = Vec::new();
        let mut max_align = 1usize;
        <(i32, f64)>::append_shape(&mut shape, 0, &mut max_align);
        assert_eq!(shape[0].offset, 0);
        assert_eq!(shape[1].offset, 8); // i32 at [0,4), f64 aligned up to 8
        let offsets: Vec<usize> = shape.iter().map(|e| e.offset).collect();

        let mut buf = [0u8; 16];
        unsafe { (7i32, 2.5f64).write_into(buf.as_mut_ptr(), &offsets) };
        let cloned: (i32, f64) = unsafe { <(i32, f64)>::clone_from(buf.as_ptr(), &offsets) };
        assert_eq!(cloned, (7, 2.5));
        let read: (i32, f64) = unsafe { <(i32, f64)>::read_from(buf.as_ptr(), &offsets) };
        assert_eq!(read, (7, 2.5));
    }

    #[test]
    fn arity_3_with_nested_tuple_element_round_trips() {
        // A nested tuple field needs no special handling: (i32, i32) is just an
        // ordinary 'static + Clone + PartialEq element type here.
        let mut shape = Vec::new();
        let mut max_align = 1usize;
        <(i32, (i32, i32), bool)>::append_shape(&mut shape, 0, &mut max_align);
        let offsets: Vec<usize> = shape.iter().map(|e| e.offset).collect();

        let mut buf = [0u8; 16];
        let value = (1i32, (2i32, 3i32), true);
        unsafe { value.write_into(buf.as_mut_ptr(), &offsets) };
        let cloned: (i32, (i32, i32), bool) =
            unsafe { <(i32, (i32, i32), bool)>::clone_from(buf.as_ptr(), &offsets) };
        assert_eq!(cloned, (1, (2, 3), true));
        let read: (i32, (i32, i32), bool) =
            unsafe { <(i32, (i32, i32), bool)>::read_from(buf.as_ptr(), &offsets) };
        assert_eq!(read, (1, (2, 3), true));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime dynamic_sequence:: -- --exact arity_1_shape_write_read_clone_round_trip arity_2_shape_write_read_clone_round_trip arity_3_with_nested_tuple_element_round_trips`
Expected: FAIL with "no method named `append_shape`" (trait doesn't exist yet).

- [ ] **Step 3: Implement the trait and arities 1–3**

Add to `cel-runtime/src/dynamic_sequence.rs`, after `push_element` (before the `tests` module):

```rust
/// Describes a concrete, `'static` Rust tuple's byte layout as a sequence of
/// [`SequenceElement`]s, and converts between that layout and `Self` by value.
///
/// Implemented for tuples of arity 1 through 12 — the same range `cel-runtime`'s `IntoList`
/// supports. A nested tuple field needs no special handling: it's simply an ordinary
/// `'static + Clone + PartialEq` element type to its enclosing tuple's own impl.
pub trait TupleSequence: Sized {
    /// Appends this tuple's elements (declaration order) to `out`, computing each one's offset
    /// from `offset` (the byte position immediately after the previous element) and folding each
    /// element's alignment into `*max_align`. Returns the byte position immediately after the
    /// last element appended.
    ///
    /// - Complexity: O(arity).
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize;

    /// Writes this tuple's fields into `dst`, at the positions in `offsets` (which must be
    /// exactly this tuple's own element offsets, in declaration order, as produced by
    /// [`append_shape`](Self::append_shape)), consuming `self`.
    ///
    /// - Complexity: O(arity).
    ///
    /// # Safety
    /// `dst` must be valid for writes covering every offset + size of this tuple's elements.
    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]);

    /// Reads this tuple back out of `src` by moving each field's bytes, at the positions in
    /// `offsets`.
    ///
    /// - Complexity: O(arity).
    ///
    /// # Safety
    /// `src` must point to a live value whose layout matches `offsets`; the caller must not
    /// separately drop those bytes afterward.
    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self;

    /// Reads this tuple back out of `src` by cloning each field's bytes, at the positions in
    /// `offsets`, leaving `src` untouched.
    ///
    /// - Complexity: O(arity).
    ///
    /// # Safety
    /// `src` must point to a live value whose layout matches `offsets`.
    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self;
}

impl<A: 'static + Clone + PartialEq> TupleSequence for (A,) {
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize {
        push_element::<A>(out, offset, max_align)
    }

    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]) {
        unsafe { std::ptr::write(dst.add(offsets[0]).cast::<A>(), self.0) };
    }

    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self {
        (unsafe { std::ptr::read(src.add(offsets[0]).cast::<A>()) },)
    }

    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self {
        (unsafe { (*src.add(offsets[0]).cast::<A>()).clone() },)
    }
}

impl<A: 'static + Clone + PartialEq, B: 'static + Clone + PartialEq> TupleSequence for (A, B) {
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize {
        let offset = push_element::<A>(out, offset, max_align);
        push_element::<B>(out, offset, max_align)
    }

    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]) {
        unsafe {
            std::ptr::write(dst.add(offsets[0]).cast::<A>(), self.0);
            std::ptr::write(dst.add(offsets[1]).cast::<B>(), self.1);
        }
    }

    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                std::ptr::read(src.add(offsets[0]).cast::<A>()),
                std::ptr::read(src.add(offsets[1]).cast::<B>()),
            )
        }
    }

    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                (*src.add(offsets[0]).cast::<A>()).clone(),
                (*src.add(offsets[1]).cast::<B>()).clone(),
            )
        }
    }
}

impl<A: 'static + Clone + PartialEq, B: 'static + Clone + PartialEq, C: 'static + Clone + PartialEq>
    TupleSequence for (A, B, C)
{
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize {
        let offset = push_element::<A>(out, offset, max_align);
        let offset = push_element::<B>(out, offset, max_align);
        push_element::<C>(out, offset, max_align)
    }

    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]) {
        unsafe {
            std::ptr::write(dst.add(offsets[0]).cast::<A>(), self.0);
            std::ptr::write(dst.add(offsets[1]).cast::<B>(), self.1);
            std::ptr::write(dst.add(offsets[2]).cast::<C>(), self.2);
        }
    }

    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                std::ptr::read(src.add(offsets[0]).cast::<A>()),
                std::ptr::read(src.add(offsets[1]).cast::<B>()),
                std::ptr::read(src.add(offsets[2]).cast::<C>()),
            )
        }
    }

    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                (*src.add(offsets[0]).cast::<A>()).clone(),
                (*src.add(offsets[1]).cast::<B>()).clone(),
                (*src.add(offsets[2]).cast::<C>()).clone(),
            )
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime dynamic_sequence::`
Expected: PASS (all tests in the module).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dynamic_sequence.rs
git commit -m "feat(cel-runtime): add TupleSequence trait and impls for arity 1-3"
```

---

### Task 4: `TupleSequence` arities 4–6

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`

**Interfaces:**
- Consumes: `TupleSequence`, `push_element` (Tasks 2–3).
- Produces: `TupleSequence` impls for `(A,B,C,D)`, `(A,B,C,D,E)`, `(A,B,C,D,E,F)`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `cel-runtime/src/dynamic_sequence.rs`:

```rust
    #[test]
    fn arity_6_shape_write_read_clone_round_trip() {
        let mut shape = Vec::new();
        let mut max_align = 1usize;
        <(i32, i32, i32, i32, i32, i32)>::append_shape(&mut shape, 0, &mut max_align);
        let offsets: Vec<usize> = shape.iter().map(|e| e.offset).collect();

        let mut buf = [0u8; 24];
        let value = (1i32, 2i32, 3i32, 4i32, 5i32, 6i32);
        unsafe { value.write_into(buf.as_mut_ptr(), &offsets) };
        let cloned: (i32, i32, i32, i32, i32, i32) =
            unsafe { <(i32, i32, i32, i32, i32, i32)>::clone_from(buf.as_ptr(), &offsets) };
        assert_eq!(cloned, (1, 2, 3, 4, 5, 6));
        let read: (i32, i32, i32, i32, i32, i32) =
            unsafe { <(i32, i32, i32, i32, i32, i32)>::read_from(buf.as_ptr(), &offsets) };
        assert_eq!(read, (1, 2, 3, 4, 5, 6));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cel-runtime dynamic_sequence::tests::arity_6_shape_write_read_clone_round_trip`
Expected: FAIL — `TupleSequence` is not implemented for a 6-tuple.

- [ ] **Step 3: Implement arities 4–6**

Add to `cel-runtime/src/dynamic_sequence.rs`, after the arity-3 impl:

```rust
impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D)
{
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize {
        let offset = push_element::<A>(out, offset, max_align);
        let offset = push_element::<B>(out, offset, max_align);
        let offset = push_element::<C>(out, offset, max_align);
        push_element::<D>(out, offset, max_align)
    }

    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]) {
        unsafe {
            std::ptr::write(dst.add(offsets[0]).cast::<A>(), self.0);
            std::ptr::write(dst.add(offsets[1]).cast::<B>(), self.1);
            std::ptr::write(dst.add(offsets[2]).cast::<C>(), self.2);
            std::ptr::write(dst.add(offsets[3]).cast::<D>(), self.3);
        }
    }

    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                std::ptr::read(src.add(offsets[0]).cast::<A>()),
                std::ptr::read(src.add(offsets[1]).cast::<B>()),
                std::ptr::read(src.add(offsets[2]).cast::<C>()),
                std::ptr::read(src.add(offsets[3]).cast::<D>()),
            )
        }
    }

    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                (*src.add(offsets[0]).cast::<A>()).clone(),
                (*src.add(offsets[1]).cast::<B>()).clone(),
                (*src.add(offsets[2]).cast::<C>()).clone(),
                (*src.add(offsets[3]).cast::<D>()).clone(),
            )
        }
    }
}

impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
    E: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D, E)
{
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize {
        let offset = push_element::<A>(out, offset, max_align);
        let offset = push_element::<B>(out, offset, max_align);
        let offset = push_element::<C>(out, offset, max_align);
        let offset = push_element::<D>(out, offset, max_align);
        push_element::<E>(out, offset, max_align)
    }

    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]) {
        unsafe {
            std::ptr::write(dst.add(offsets[0]).cast::<A>(), self.0);
            std::ptr::write(dst.add(offsets[1]).cast::<B>(), self.1);
            std::ptr::write(dst.add(offsets[2]).cast::<C>(), self.2);
            std::ptr::write(dst.add(offsets[3]).cast::<D>(), self.3);
            std::ptr::write(dst.add(offsets[4]).cast::<E>(), self.4);
        }
    }

    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                std::ptr::read(src.add(offsets[0]).cast::<A>()),
                std::ptr::read(src.add(offsets[1]).cast::<B>()),
                std::ptr::read(src.add(offsets[2]).cast::<C>()),
                std::ptr::read(src.add(offsets[3]).cast::<D>()),
                std::ptr::read(src.add(offsets[4]).cast::<E>()),
            )
        }
    }

    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                (*src.add(offsets[0]).cast::<A>()).clone(),
                (*src.add(offsets[1]).cast::<B>()).clone(),
                (*src.add(offsets[2]).cast::<C>()).clone(),
                (*src.add(offsets[3]).cast::<D>()).clone(),
                (*src.add(offsets[4]).cast::<E>()).clone(),
            )
        }
    }
}

impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
    E: 'static + Clone + PartialEq,
    F: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D, E, F)
{
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize {
        let offset = push_element::<A>(out, offset, max_align);
        let offset = push_element::<B>(out, offset, max_align);
        let offset = push_element::<C>(out, offset, max_align);
        let offset = push_element::<D>(out, offset, max_align);
        let offset = push_element::<E>(out, offset, max_align);
        push_element::<F>(out, offset, max_align)
    }

    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]) {
        unsafe {
            std::ptr::write(dst.add(offsets[0]).cast::<A>(), self.0);
            std::ptr::write(dst.add(offsets[1]).cast::<B>(), self.1);
            std::ptr::write(dst.add(offsets[2]).cast::<C>(), self.2);
            std::ptr::write(dst.add(offsets[3]).cast::<D>(), self.3);
            std::ptr::write(dst.add(offsets[4]).cast::<E>(), self.4);
            std::ptr::write(dst.add(offsets[5]).cast::<F>(), self.5);
        }
    }

    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                std::ptr::read(src.add(offsets[0]).cast::<A>()),
                std::ptr::read(src.add(offsets[1]).cast::<B>()),
                std::ptr::read(src.add(offsets[2]).cast::<C>()),
                std::ptr::read(src.add(offsets[3]).cast::<D>()),
                std::ptr::read(src.add(offsets[4]).cast::<E>()),
                std::ptr::read(src.add(offsets[5]).cast::<F>()),
            )
        }
    }

    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                (*src.add(offsets[0]).cast::<A>()).clone(),
                (*src.add(offsets[1]).cast::<B>()).clone(),
                (*src.add(offsets[2]).cast::<C>()).clone(),
                (*src.add(offsets[3]).cast::<D>()).clone(),
                (*src.add(offsets[4]).cast::<E>()).clone(),
                (*src.add(offsets[5]).cast::<F>()).clone(),
            )
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p cel-runtime dynamic_sequence::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dynamic_sequence.rs
git commit -m "feat(cel-runtime): add TupleSequence impls for arity 4-6"
```

---

### Task 5: `TupleSequence` arities 7–9

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`

**Interfaces:**
- Consumes: `TupleSequence`, `push_element` (Tasks 2–4).
- Produces: `TupleSequence` impls for `(A..G)`, `(A..H)`, `(A..I)` (7, 8, 9 elements).

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `cel-runtime/src/dynamic_sequence.rs`:

```rust
    #[test]
    fn arity_9_shape_write_read_clone_round_trip() {
        let mut shape = Vec::new();
        let mut max_align = 1usize;
        <(i32, i32, i32, i32, i32, i32, i32, i32, i32)>::append_shape(&mut shape, 0, &mut max_align);
        let offsets: Vec<usize> = shape.iter().map(|e| e.offset).collect();

        let mut buf = [0u8; 36];
        let value = (1i32, 2i32, 3i32, 4i32, 5i32, 6i32, 7i32, 8i32, 9i32);
        unsafe { value.write_into(buf.as_mut_ptr(), &offsets) };
        let cloned: (i32, i32, i32, i32, i32, i32, i32, i32, i32) = unsafe {
            <(i32, i32, i32, i32, i32, i32, i32, i32, i32)>::clone_from(buf.as_ptr(), &offsets)
        };
        assert_eq!(cloned, (1, 2, 3, 4, 5, 6, 7, 8, 9));
        let read: (i32, i32, i32, i32, i32, i32, i32, i32, i32) = unsafe {
            <(i32, i32, i32, i32, i32, i32, i32, i32, i32)>::read_from(buf.as_ptr(), &offsets)
        };
        assert_eq!(read, (1, 2, 3, 4, 5, 6, 7, 8, 9));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cel-runtime dynamic_sequence::tests::arity_9_shape_write_read_clone_round_trip`
Expected: FAIL — `TupleSequence` is not implemented for a 9-tuple.

- [ ] **Step 3: Implement arities 7–9**

Add to `cel-runtime/src/dynamic_sequence.rs`, after the arity-6 impl:

```rust
impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
    E: 'static + Clone + PartialEq,
    F: 'static + Clone + PartialEq,
    G: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D, E, F, G)
{
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize {
        let offset = push_element::<A>(out, offset, max_align);
        let offset = push_element::<B>(out, offset, max_align);
        let offset = push_element::<C>(out, offset, max_align);
        let offset = push_element::<D>(out, offset, max_align);
        let offset = push_element::<E>(out, offset, max_align);
        let offset = push_element::<F>(out, offset, max_align);
        push_element::<G>(out, offset, max_align)
    }

    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]) {
        unsafe {
            std::ptr::write(dst.add(offsets[0]).cast::<A>(), self.0);
            std::ptr::write(dst.add(offsets[1]).cast::<B>(), self.1);
            std::ptr::write(dst.add(offsets[2]).cast::<C>(), self.2);
            std::ptr::write(dst.add(offsets[3]).cast::<D>(), self.3);
            std::ptr::write(dst.add(offsets[4]).cast::<E>(), self.4);
            std::ptr::write(dst.add(offsets[5]).cast::<F>(), self.5);
            std::ptr::write(dst.add(offsets[6]).cast::<G>(), self.6);
        }
    }

    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                std::ptr::read(src.add(offsets[0]).cast::<A>()),
                std::ptr::read(src.add(offsets[1]).cast::<B>()),
                std::ptr::read(src.add(offsets[2]).cast::<C>()),
                std::ptr::read(src.add(offsets[3]).cast::<D>()),
                std::ptr::read(src.add(offsets[4]).cast::<E>()),
                std::ptr::read(src.add(offsets[5]).cast::<F>()),
                std::ptr::read(src.add(offsets[6]).cast::<G>()),
            )
        }
    }

    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                (*src.add(offsets[0]).cast::<A>()).clone(),
                (*src.add(offsets[1]).cast::<B>()).clone(),
                (*src.add(offsets[2]).cast::<C>()).clone(),
                (*src.add(offsets[3]).cast::<D>()).clone(),
                (*src.add(offsets[4]).cast::<E>()).clone(),
                (*src.add(offsets[5]).cast::<F>()).clone(),
                (*src.add(offsets[6]).cast::<G>()).clone(),
            )
        }
    }
}

impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
    E: 'static + Clone + PartialEq,
    F: 'static + Clone + PartialEq,
    G: 'static + Clone + PartialEq,
    H: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D, E, F, G, H)
{
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize {
        let offset = push_element::<A>(out, offset, max_align);
        let offset = push_element::<B>(out, offset, max_align);
        let offset = push_element::<C>(out, offset, max_align);
        let offset = push_element::<D>(out, offset, max_align);
        let offset = push_element::<E>(out, offset, max_align);
        let offset = push_element::<F>(out, offset, max_align);
        let offset = push_element::<G>(out, offset, max_align);
        push_element::<H>(out, offset, max_align)
    }

    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]) {
        unsafe {
            std::ptr::write(dst.add(offsets[0]).cast::<A>(), self.0);
            std::ptr::write(dst.add(offsets[1]).cast::<B>(), self.1);
            std::ptr::write(dst.add(offsets[2]).cast::<C>(), self.2);
            std::ptr::write(dst.add(offsets[3]).cast::<D>(), self.3);
            std::ptr::write(dst.add(offsets[4]).cast::<E>(), self.4);
            std::ptr::write(dst.add(offsets[5]).cast::<F>(), self.5);
            std::ptr::write(dst.add(offsets[6]).cast::<G>(), self.6);
            std::ptr::write(dst.add(offsets[7]).cast::<H>(), self.7);
        }
    }

    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                std::ptr::read(src.add(offsets[0]).cast::<A>()),
                std::ptr::read(src.add(offsets[1]).cast::<B>()),
                std::ptr::read(src.add(offsets[2]).cast::<C>()),
                std::ptr::read(src.add(offsets[3]).cast::<D>()),
                std::ptr::read(src.add(offsets[4]).cast::<E>()),
                std::ptr::read(src.add(offsets[5]).cast::<F>()),
                std::ptr::read(src.add(offsets[6]).cast::<G>()),
                std::ptr::read(src.add(offsets[7]).cast::<H>()),
            )
        }
    }

    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                (*src.add(offsets[0]).cast::<A>()).clone(),
                (*src.add(offsets[1]).cast::<B>()).clone(),
                (*src.add(offsets[2]).cast::<C>()).clone(),
                (*src.add(offsets[3]).cast::<D>()).clone(),
                (*src.add(offsets[4]).cast::<E>()).clone(),
                (*src.add(offsets[5]).cast::<F>()).clone(),
                (*src.add(offsets[6]).cast::<G>()).clone(),
                (*src.add(offsets[7]).cast::<H>()).clone(),
            )
        }
    }
}

impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
    E: 'static + Clone + PartialEq,
    F: 'static + Clone + PartialEq,
    G: 'static + Clone + PartialEq,
    H: 'static + Clone + PartialEq,
    I: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D, E, F, G, H, I)
{
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize {
        let offset = push_element::<A>(out, offset, max_align);
        let offset = push_element::<B>(out, offset, max_align);
        let offset = push_element::<C>(out, offset, max_align);
        let offset = push_element::<D>(out, offset, max_align);
        let offset = push_element::<E>(out, offset, max_align);
        let offset = push_element::<F>(out, offset, max_align);
        let offset = push_element::<G>(out, offset, max_align);
        let offset = push_element::<H>(out, offset, max_align);
        push_element::<I>(out, offset, max_align)
    }

    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]) {
        unsafe {
            std::ptr::write(dst.add(offsets[0]).cast::<A>(), self.0);
            std::ptr::write(dst.add(offsets[1]).cast::<B>(), self.1);
            std::ptr::write(dst.add(offsets[2]).cast::<C>(), self.2);
            std::ptr::write(dst.add(offsets[3]).cast::<D>(), self.3);
            std::ptr::write(dst.add(offsets[4]).cast::<E>(), self.4);
            std::ptr::write(dst.add(offsets[5]).cast::<F>(), self.5);
            std::ptr::write(dst.add(offsets[6]).cast::<G>(), self.6);
            std::ptr::write(dst.add(offsets[7]).cast::<H>(), self.7);
            std::ptr::write(dst.add(offsets[8]).cast::<I>(), self.8);
        }
    }

    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                std::ptr::read(src.add(offsets[0]).cast::<A>()),
                std::ptr::read(src.add(offsets[1]).cast::<B>()),
                std::ptr::read(src.add(offsets[2]).cast::<C>()),
                std::ptr::read(src.add(offsets[3]).cast::<D>()),
                std::ptr::read(src.add(offsets[4]).cast::<E>()),
                std::ptr::read(src.add(offsets[5]).cast::<F>()),
                std::ptr::read(src.add(offsets[6]).cast::<G>()),
                std::ptr::read(src.add(offsets[7]).cast::<H>()),
                std::ptr::read(src.add(offsets[8]).cast::<I>()),
            )
        }
    }

    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                (*src.add(offsets[0]).cast::<A>()).clone(),
                (*src.add(offsets[1]).cast::<B>()).clone(),
                (*src.add(offsets[2]).cast::<C>()).clone(),
                (*src.add(offsets[3]).cast::<D>()).clone(),
                (*src.add(offsets[4]).cast::<E>()).clone(),
                (*src.add(offsets[5]).cast::<F>()).clone(),
                (*src.add(offsets[6]).cast::<G>()).clone(),
                (*src.add(offsets[7]).cast::<H>()).clone(),
                (*src.add(offsets[8]).cast::<I>()).clone(),
            )
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p cel-runtime dynamic_sequence::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dynamic_sequence.rs
git commit -m "feat(cel-runtime): add TupleSequence impls for arity 7-9"
```

---

### Task 6: `TupleSequence` arities 10–12

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`

**Interfaces:**
- Consumes: `TupleSequence`, `push_element` (Tasks 2–5).
- Produces: `TupleSequence` impls for `(A..J)`, `(A..K)`, `(A..L)` (10, 11, 12 elements).

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `cel-runtime/src/dynamic_sequence.rs`:

```rust
    #[test]
    fn arity_12_shape_write_read_clone_round_trip() {
        let mut shape = Vec::new();
        let mut max_align = 1usize;
        <(i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32)>::append_shape(
            &mut shape, 0, &mut max_align,
        );
        let offsets: Vec<usize> = shape.iter().map(|e| e.offset).collect();

        let mut buf = [0u8; 48];
        let value = (1i32, 2i32, 3i32, 4i32, 5i32, 6i32, 7i32, 8i32, 9i32, 10i32, 11i32, 12i32);
        unsafe { value.write_into(buf.as_mut_ptr(), &offsets) };
        let cloned: (i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32) = unsafe {
            <(i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32)>::clone_from(
                buf.as_ptr(),
                &offsets,
            )
        };
        assert_eq!(cloned, (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12));
        let read: (i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32) = unsafe {
            <(i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32)>::read_from(
                buf.as_ptr(),
                &offsets,
            )
        };
        assert_eq!(read, (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cel-runtime dynamic_sequence::tests::arity_12_shape_write_read_clone_round_trip`
Expected: FAIL — `TupleSequence` is not implemented for a 12-tuple.

- [ ] **Step 3: Implement arities 10–12**

Add to `cel-runtime/src/dynamic_sequence.rs`, after the arity-9 impl:

```rust
impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
    E: 'static + Clone + PartialEq,
    F: 'static + Clone + PartialEq,
    G: 'static + Clone + PartialEq,
    H: 'static + Clone + PartialEq,
    I: 'static + Clone + PartialEq,
    J: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D, E, F, G, H, I, J)
{
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize {
        let offset = push_element::<A>(out, offset, max_align);
        let offset = push_element::<B>(out, offset, max_align);
        let offset = push_element::<C>(out, offset, max_align);
        let offset = push_element::<D>(out, offset, max_align);
        let offset = push_element::<E>(out, offset, max_align);
        let offset = push_element::<F>(out, offset, max_align);
        let offset = push_element::<G>(out, offset, max_align);
        let offset = push_element::<H>(out, offset, max_align);
        let offset = push_element::<I>(out, offset, max_align);
        push_element::<J>(out, offset, max_align)
    }

    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]) {
        unsafe {
            std::ptr::write(dst.add(offsets[0]).cast::<A>(), self.0);
            std::ptr::write(dst.add(offsets[1]).cast::<B>(), self.1);
            std::ptr::write(dst.add(offsets[2]).cast::<C>(), self.2);
            std::ptr::write(dst.add(offsets[3]).cast::<D>(), self.3);
            std::ptr::write(dst.add(offsets[4]).cast::<E>(), self.4);
            std::ptr::write(dst.add(offsets[5]).cast::<F>(), self.5);
            std::ptr::write(dst.add(offsets[6]).cast::<G>(), self.6);
            std::ptr::write(dst.add(offsets[7]).cast::<H>(), self.7);
            std::ptr::write(dst.add(offsets[8]).cast::<I>(), self.8);
            std::ptr::write(dst.add(offsets[9]).cast::<J>(), self.9);
        }
    }

    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                std::ptr::read(src.add(offsets[0]).cast::<A>()),
                std::ptr::read(src.add(offsets[1]).cast::<B>()),
                std::ptr::read(src.add(offsets[2]).cast::<C>()),
                std::ptr::read(src.add(offsets[3]).cast::<D>()),
                std::ptr::read(src.add(offsets[4]).cast::<E>()),
                std::ptr::read(src.add(offsets[5]).cast::<F>()),
                std::ptr::read(src.add(offsets[6]).cast::<G>()),
                std::ptr::read(src.add(offsets[7]).cast::<H>()),
                std::ptr::read(src.add(offsets[8]).cast::<I>()),
                std::ptr::read(src.add(offsets[9]).cast::<J>()),
            )
        }
    }

    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                (*src.add(offsets[0]).cast::<A>()).clone(),
                (*src.add(offsets[1]).cast::<B>()).clone(),
                (*src.add(offsets[2]).cast::<C>()).clone(),
                (*src.add(offsets[3]).cast::<D>()).clone(),
                (*src.add(offsets[4]).cast::<E>()).clone(),
                (*src.add(offsets[5]).cast::<F>()).clone(),
                (*src.add(offsets[6]).cast::<G>()).clone(),
                (*src.add(offsets[7]).cast::<H>()).clone(),
                (*src.add(offsets[8]).cast::<I>()).clone(),
                (*src.add(offsets[9]).cast::<J>()).clone(),
            )
        }
    }
}

impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
    E: 'static + Clone + PartialEq,
    F: 'static + Clone + PartialEq,
    G: 'static + Clone + PartialEq,
    H: 'static + Clone + PartialEq,
    I: 'static + Clone + PartialEq,
    J: 'static + Clone + PartialEq,
    K: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D, E, F, G, H, I, J, K)
{
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize {
        let offset = push_element::<A>(out, offset, max_align);
        let offset = push_element::<B>(out, offset, max_align);
        let offset = push_element::<C>(out, offset, max_align);
        let offset = push_element::<D>(out, offset, max_align);
        let offset = push_element::<E>(out, offset, max_align);
        let offset = push_element::<F>(out, offset, max_align);
        let offset = push_element::<G>(out, offset, max_align);
        let offset = push_element::<H>(out, offset, max_align);
        let offset = push_element::<I>(out, offset, max_align);
        let offset = push_element::<J>(out, offset, max_align);
        push_element::<K>(out, offset, max_align)
    }

    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]) {
        unsafe {
            std::ptr::write(dst.add(offsets[0]).cast::<A>(), self.0);
            std::ptr::write(dst.add(offsets[1]).cast::<B>(), self.1);
            std::ptr::write(dst.add(offsets[2]).cast::<C>(), self.2);
            std::ptr::write(dst.add(offsets[3]).cast::<D>(), self.3);
            std::ptr::write(dst.add(offsets[4]).cast::<E>(), self.4);
            std::ptr::write(dst.add(offsets[5]).cast::<F>(), self.5);
            std::ptr::write(dst.add(offsets[6]).cast::<G>(), self.6);
            std::ptr::write(dst.add(offsets[7]).cast::<H>(), self.7);
            std::ptr::write(dst.add(offsets[8]).cast::<I>(), self.8);
            std::ptr::write(dst.add(offsets[9]).cast::<J>(), self.9);
            std::ptr::write(dst.add(offsets[10]).cast::<K>(), self.10);
        }
    }

    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                std::ptr::read(src.add(offsets[0]).cast::<A>()),
                std::ptr::read(src.add(offsets[1]).cast::<B>()),
                std::ptr::read(src.add(offsets[2]).cast::<C>()),
                std::ptr::read(src.add(offsets[3]).cast::<D>()),
                std::ptr::read(src.add(offsets[4]).cast::<E>()),
                std::ptr::read(src.add(offsets[5]).cast::<F>()),
                std::ptr::read(src.add(offsets[6]).cast::<G>()),
                std::ptr::read(src.add(offsets[7]).cast::<H>()),
                std::ptr::read(src.add(offsets[8]).cast::<I>()),
                std::ptr::read(src.add(offsets[9]).cast::<J>()),
                std::ptr::read(src.add(offsets[10]).cast::<K>()),
            )
        }
    }

    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                (*src.add(offsets[0]).cast::<A>()).clone(),
                (*src.add(offsets[1]).cast::<B>()).clone(),
                (*src.add(offsets[2]).cast::<C>()).clone(),
                (*src.add(offsets[3]).cast::<D>()).clone(),
                (*src.add(offsets[4]).cast::<E>()).clone(),
                (*src.add(offsets[5]).cast::<F>()).clone(),
                (*src.add(offsets[6]).cast::<G>()).clone(),
                (*src.add(offsets[7]).cast::<H>()).clone(),
                (*src.add(offsets[8]).cast::<I>()).clone(),
                (*src.add(offsets[9]).cast::<J>()).clone(),
                (*src.add(offsets[10]).cast::<K>()).clone(),
            )
        }
    }
}

impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
    E: 'static + Clone + PartialEq,
    F: 'static + Clone + PartialEq,
    G: 'static + Clone + PartialEq,
    H: 'static + Clone + PartialEq,
    I: 'static + Clone + PartialEq,
    J: 'static + Clone + PartialEq,
    K: 'static + Clone + PartialEq,
    L: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D, E, F, G, H, I, J, K, L)
{
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize {
        let offset = push_element::<A>(out, offset, max_align);
        let offset = push_element::<B>(out, offset, max_align);
        let offset = push_element::<C>(out, offset, max_align);
        let offset = push_element::<D>(out, offset, max_align);
        let offset = push_element::<E>(out, offset, max_align);
        let offset = push_element::<F>(out, offset, max_align);
        let offset = push_element::<G>(out, offset, max_align);
        let offset = push_element::<H>(out, offset, max_align);
        let offset = push_element::<I>(out, offset, max_align);
        let offset = push_element::<J>(out, offset, max_align);
        let offset = push_element::<K>(out, offset, max_align);
        push_element::<L>(out, offset, max_align)
    }

    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]) {
        unsafe {
            std::ptr::write(dst.add(offsets[0]).cast::<A>(), self.0);
            std::ptr::write(dst.add(offsets[1]).cast::<B>(), self.1);
            std::ptr::write(dst.add(offsets[2]).cast::<C>(), self.2);
            std::ptr::write(dst.add(offsets[3]).cast::<D>(), self.3);
            std::ptr::write(dst.add(offsets[4]).cast::<E>(), self.4);
            std::ptr::write(dst.add(offsets[5]).cast::<F>(), self.5);
            std::ptr::write(dst.add(offsets[6]).cast::<G>(), self.6);
            std::ptr::write(dst.add(offsets[7]).cast::<H>(), self.7);
            std::ptr::write(dst.add(offsets[8]).cast::<I>(), self.8);
            std::ptr::write(dst.add(offsets[9]).cast::<J>(), self.9);
            std::ptr::write(dst.add(offsets[10]).cast::<K>(), self.10);
            std::ptr::write(dst.add(offsets[11]).cast::<L>(), self.11);
        }
    }

    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                std::ptr::read(src.add(offsets[0]).cast::<A>()),
                std::ptr::read(src.add(offsets[1]).cast::<B>()),
                std::ptr::read(src.add(offsets[2]).cast::<C>()),
                std::ptr::read(src.add(offsets[3]).cast::<D>()),
                std::ptr::read(src.add(offsets[4]).cast::<E>()),
                std::ptr::read(src.add(offsets[5]).cast::<F>()),
                std::ptr::read(src.add(offsets[6]).cast::<G>()),
                std::ptr::read(src.add(offsets[7]).cast::<H>()),
                std::ptr::read(src.add(offsets[8]).cast::<I>()),
                std::ptr::read(src.add(offsets[9]).cast::<J>()),
                std::ptr::read(src.add(offsets[10]).cast::<K>()),
                std::ptr::read(src.add(offsets[11]).cast::<L>()),
            )
        }
    }

    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            (
                (*src.add(offsets[0]).cast::<A>()).clone(),
                (*src.add(offsets[1]).cast::<B>()).clone(),
                (*src.add(offsets[2]).cast::<C>()).clone(),
                (*src.add(offsets[3]).cast::<D>()).clone(),
                (*src.add(offsets[4]).cast::<E>()).clone(),
                (*src.add(offsets[5]).cast::<F>()).clone(),
                (*src.add(offsets[6]).cast::<G>()).clone(),
                (*src.add(offsets[7]).cast::<H>()).clone(),
                (*src.add(offsets[8]).cast::<I>()).clone(),
                (*src.add(offsets[9]).cast::<J>()).clone(),
                (*src.add(offsets[10]).cast::<K>()).clone(),
                (*src.add(offsets[11]).cast::<L>()).clone(),
            )
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p cel-runtime dynamic_sequence::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dynamic_sequence.rs
git commit -m "feat(cel-runtime): add TupleSequence impls for arity 10-12"
```

---

### Task 7: `DynamicSequence` struct, `from_tuple`, and `Drop`

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`

**Interfaces:**
- Consumes: `TupleSequence`, `SequenceElement` (Tasks 2–6); `RawStack::{with_base_alignment, reserve_and_write, drop_at}` (Task 1, and existing `cel-runtime` API).
- Produces: `pub struct DynamicSequence`, `DynamicSequence::from_tuple<T: TupleSequence>(value: T) -> Self`, `DynamicSequence::arity(&self) -> usize`, `impl Drop for DynamicSequence`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dynamic_sequence.rs`:

```rust
    #[test]
    fn from_tuple_records_correct_arity() {
        let seq = DynamicSequence::from_tuple((1i32, 2.5f64, true));
        assert_eq!(seq.arity(), 3);
    }

    #[test]
    fn from_tuple_and_drop_drops_every_element_exactly_once_in_reverse_order() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Clone, PartialEq)]
        struct DropCounter(Arc<AtomicUsize>, Arc<std::sync::Mutex<Vec<u8>>>, u8);
        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
                self.1.lock().unwrap().push(self.2);
            }
        }

        let count = Arc::new(AtomicUsize::new(0));
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let a = DropCounter(count.clone(), order.clone(), 1);
        let b = DropCounter(count.clone(), order.clone(), 2);

        let seq = DynamicSequence::from_tuple((a, b));
        drop(seq);

        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(*order.lock().unwrap(), vec![2, 1]); // reverse of declaration order
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime dynamic_sequence:: -- --exact from_tuple_records_correct_arity from_tuple_and_drop_drops_every_element_exactly_once_in_reverse_order`
Expected: FAIL with "cannot find struct/function `DynamicSequence`".

- [ ] **Step 3: Implement `DynamicSequence`, `from_tuple`, `arity`, `Drop`**

Add to `cel-runtime/src/dynamic_sequence.rs`, after the arity-12 impl (before the `tests` module):

```rust
/// An owned, type-erased CEL tuple value that persists beyond any single `DynSegment`
/// evaluation, suitable for storing directly as an `adam_rs::Sheet` cell's value.
///
/// Converts type-safely to and from concrete Rust tuples via [`TupleSequence`] — see
/// [`from_tuple`](Self::from_tuple), [`try_into_tuple`](Self::try_into_tuple), and
/// [`try_to_tuple`](Self::try_to_tuple).
pub struct DynamicSequence {
    buffer: crate::raw_stack::RawStack,
    shape: Vec<SequenceElement>,
    max_align: usize,
}

impl DynamicSequence {
    /// Builds a `DynamicSequence` from a concrete Rust tuple, consuming it.
    ///
    /// - Complexity: O(arity) time; exactly one heap allocation.
    #[must_use]
    pub fn from_tuple<T: TupleSequence>(value: T) -> Self {
        let mut shape = Vec::new();
        let mut max_align = 1usize;
        let end = T::append_shape(&mut shape, 0, &mut max_align);
        let total_size = align_index(max_align, end);
        let offsets: Vec<usize> = shape.iter().map(|e| e.offset).collect();

        let mut buffer = crate::raw_stack::RawStack::with_base_alignment(max_align);
        unsafe {
            buffer.reserve_and_write(max_align, total_size, |dst| {
                unsafe { value.write_into(dst, &offsets) };
            });
        }
        DynamicSequence {
            buffer,
            shape,
            max_align,
        }
    }

    /// Returns the number of elements this sequence holds.
    #[must_use]
    pub fn arity(&self) -> usize {
        self.shape.len()
    }
}

impl Drop for DynamicSequence {
    fn drop(&mut self) {
        for elem in self.shape.iter().rev() {
            unsafe { self.buffer.drop_at(elem.offset, |ptr| (elem.drop)(ptr)) };
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime dynamic_sequence::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dynamic_sequence.rs
git commit -m "feat(cel-runtime): add DynamicSequence struct, from_tuple, and Drop"
```

---

### Task 8: `DynamicSequence::Clone` and `PartialEq`

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`

**Interfaces:**
- Consumes: `DynamicSequence` (Task 7); `RawStack::{read_at, reserve_and_write, with_base_alignment}`.
- Produces: `impl Clone for DynamicSequence`, `impl PartialEq for DynamicSequence`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dynamic_sequence.rs`:

```rust
    #[test]
    fn clone_produces_an_independently_droppable_equal_copy() {
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
        let original = DynamicSequence::from_tuple((DropCounter(count.clone()), 7i32));
        let cloned = original.clone();

        assert_eq!(original, cloned);

        drop(original);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        drop(cloned);
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn partial_eq_is_false_for_different_arity_or_element_type() {
        let a = DynamicSequence::from_tuple((1i32, 2i32));
        let b = DynamicSequence::from_tuple((1i32,));
        let c = DynamicSequence::from_tuple((1i32, 2.0f64));
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn partial_eq_is_false_for_different_values_of_the_same_shape() {
        let a = DynamicSequence::from_tuple((1i32, 2i32));
        let b = DynamicSequence::from_tuple((1i32, 3i32));
        assert_ne!(a, b);
        let c = DynamicSequence::from_tuple((1i32, 2i32));
        assert_eq!(a, c);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime dynamic_sequence:: -- --exact clone_produces_an_independently_droppable_equal_copy partial_eq_is_false_for_different_arity_or_element_type partial_eq_is_false_for_different_values_of_the_same_shape`
Expected: FAIL — `DynamicSequence` doesn't implement `Clone`/`PartialEq` yet.

- [ ] **Step 3: Implement `Clone` and `PartialEq`**

Add to `cel-runtime/src/dynamic_sequence.rs`, after `impl Drop for DynamicSequence`:

```rust
impl Clone for DynamicSequence {
    fn clone(&self) -> Self {
        let total_size = self.buffer.len();
        let mut buffer = crate::raw_stack::RawStack::with_base_alignment(self.max_align);
        unsafe {
            buffer.reserve_and_write(self.max_align, total_size, |dst| {
                for elem in &self.shape {
                    unsafe {
                        self.buffer.read_at(elem.offset, |src| {
                            (elem.clone)(src, dst.add(elem.offset));
                        });
                    }
                }
            });
        }
        DynamicSequence {
            buffer,
            shape: self.shape.clone(),
            max_align: self.max_align,
        }
    }
}

impl PartialEq for DynamicSequence {
    fn eq(&self, other: &Self) -> bool {
        self.shape.len() == other.shape.len()
            && self
                .shape
                .iter()
                .zip(&other.shape)
                .all(|(a, b)| a.type_id == b.type_id)
            && self.shape.iter().all(|elem| unsafe {
                self.buffer.read_at(elem.offset, |a| {
                    other
                        .buffer
                        .read_at(elem.offset, |b| (elem.eq)(a, b))
                })
            })
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime dynamic_sequence::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dynamic_sequence.rs
git commit -m "feat(cel-runtime): add Clone and PartialEq for DynamicSequence"
```

---

### Task 9: `DynamicSequence::try_into_tuple` and `try_to_tuple`

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`

**Interfaces:**
- Consumes: `DynamicSequence`, `TupleSequence` (Tasks 3–8).
- Produces: `DynamicSequence::try_into_tuple<T: TupleSequence>(self) -> anyhow::Result<T>`, `DynamicSequence::try_to_tuple<T: TupleSequence>(&self) -> anyhow::Result<T>`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dynamic_sequence.rs`:

```rust
    #[test]
    fn try_into_tuple_round_trips_for_a_matching_shape() -> anyhow::Result<()> {
        let seq = DynamicSequence::from_tuple((3i32, 4.5f64));
        let result: (i32, f64) = seq.try_into_tuple()?;
        assert_eq!(result, (3, 4.5));
        Ok(())
    }

    #[test]
    fn try_into_tuple_errs_on_shape_mismatch() {
        let seq = DynamicSequence::from_tuple((3i32, 4i32));
        let result = seq.try_into_tuple::<(i32, f64)>();
        assert!(result.is_err());
    }

    #[test]
    fn try_to_tuple_clones_without_consuming_the_sequence() -> anyhow::Result<()> {
        let seq = DynamicSequence::from_tuple((1i32, "hello".to_string()));
        let a: (i32, String) = seq.try_to_tuple()?;
        let b: (i32, String) = seq.try_to_tuple()?;
        assert_eq!(a, (1, "hello".to_string()));
        assert_eq!(b, (1, "hello".to_string()));
        Ok(())
    }

    #[test]
    fn try_to_tuple_errs_on_shape_mismatch() {
        let seq = DynamicSequence::from_tuple((1i32,));
        let result = seq.try_to_tuple::<(i32, i32)>();
        assert!(result.is_err());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime dynamic_sequence:: -- --exact try_into_tuple_round_trips_for_a_matching_shape try_into_tuple_errs_on_shape_mismatch try_to_tuple_clones_without_consuming_the_sequence try_to_tuple_errs_on_shape_mismatch`
Expected: FAIL — the methods don't exist yet.

- [ ] **Step 3: Implement `try_into_tuple` and `try_to_tuple`**

Add to `impl DynamicSequence` in `cel-runtime/src/dynamic_sequence.rs`, after `arity`:

```rust
    /// Returns whether `T`'s element `TypeId` sequence matches this sequence's actual elements
    /// exactly (same arity, same type at each position, in order).
    fn shape_matches<T: TupleSequence>(&self) -> bool {
        let mut expected = Vec::new();
        let mut max_align = 1usize;
        T::append_shape(&mut expected, 0, &mut max_align);
        self.shape.len() == expected.len()
            && self
                .shape
                .iter()
                .zip(&expected)
                .all(|(a, b)| a.type_id == b.type_id)
    }

    /// Consumes this sequence and reconstructs it as the concrete tuple `T`.
    ///
    /// - Complexity: O(arity).
    ///
    /// # Errors
    /// Returns `Err` if `T`'s element `TypeId` sequence doesn't match this sequence's actual
    /// elements (different arity, or a different type at some position).
    pub fn try_into_tuple<T: TupleSequence>(mut self) -> anyhow::Result<T> {
        anyhow::ensure!(
            self.shape_matches::<T>(),
            "DynamicSequence::try_into_tuple: shape mismatch"
        );
        let offsets: Vec<usize> = self.shape.iter().map(|e| e.offset).collect();
        let result = unsafe { self.buffer.read_at(0, |base| T::read_from(base, &offsets)) };
        // Fields were just moved out of `self.buffer`'s bytes above; clearing `shape` makes
        // `Drop`'s element loop a no-op so those fields aren't dropped a second time. `buffer`'s
        // own backing allocation is still freed normally when `self` goes out of scope below.
        self.shape.clear();
        Ok(result)
    }

    /// Reconstructs the concrete tuple `T` by cloning this sequence's elements, leaving `self`
    /// untouched.
    ///
    /// - Complexity: O(arity).
    ///
    /// # Errors
    /// Returns `Err` if `T`'s element `TypeId` sequence doesn't match this sequence's actual
    /// elements (different arity, or a different type at some position).
    pub fn try_to_tuple<T: TupleSequence>(&self) -> anyhow::Result<T> {
        anyhow::ensure!(
            self.shape_matches::<T>(),
            "DynamicSequence::try_to_tuple: shape mismatch"
        );
        let offsets: Vec<usize> = self.shape.iter().map(|e| e.offset).collect();
        Ok(unsafe { self.buffer.read_at(0, |base| T::clone_from(base, &offsets)) })
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime dynamic_sequence::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dynamic_sequence.rs
git commit -m "feat(cel-runtime): add DynamicSequence::try_into_tuple and try_to_tuple"
```

---

### Task 10: `DynamicSequence::adapt_fn_1`

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`

**Interfaces:**
- Consumes: `DynamicSequence::try_to_tuple` (Task 9).
- Produces: `DynamicSequence::adapt_fn_1<A: TupleSequence, R, F: Fn(&A) -> anyhow::Result<R>>(f: F) -> impl Fn(&DynamicSequence) -> anyhow::Result<R>`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dynamic_sequence.rs`:

```rust
    #[test]
    fn adapt_fn_1_calls_the_wrapped_closure_with_a_concrete_tuple() -> anyhow::Result<()> {
        let seq = DynamicSequence::from_tuple((3i32, 4.5f64));
        let wrapped = DynamicSequence::adapt_fn_1(|t: &(i32, f64)| Ok(t.0 as f64 + t.1));
        assert_eq!(wrapped(&seq)?, 7.5);
        Ok(())
    }

    #[test]
    fn adapt_fn_1_returns_err_on_shape_mismatch() {
        let seq = DynamicSequence::from_tuple((3i32, 4i32));
        let wrapped = DynamicSequence::adapt_fn_1(|t: &(i32, f64)| Ok(t.0 as f64 + t.1));
        assert!(wrapped(&seq).is_err());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime dynamic_sequence:: -- --exact adapt_fn_1_calls_the_wrapped_closure_with_a_concrete_tuple adapt_fn_1_returns_err_on_shape_mismatch`
Expected: FAIL — `adapt_fn_1` doesn't exist yet.

- [ ] **Step 3: Implement `adapt_fn_1`**

Add to `impl DynamicSequence` in `cel-runtime/src/dynamic_sequence.rs`, after `try_to_tuple`:

```rust
    /// Wraps a closure over a concrete tuple `A` so it can be passed directly as the `F` in
    /// `adam_rs::Method::from_fn_1_1::<DynamicSequence, R, _>` or
    /// `adam_rs::Condition::from_fn_1::<DynamicSequence, _>`.
    ///
    /// Every call clones `A`'s fields out of the `&DynamicSequence` (via
    /// [`try_to_tuple`](Self::try_to_tuple)) into a temporary `A`, calls `f` with a reference to
    /// it, then drops the temporary.
    ///
    /// # Errors
    /// The returned closure returns `Err` if `A`'s element `TypeId` sequence doesn't match the
    /// `DynamicSequence`'s actual elements.
    pub fn adapt_fn_1<A, R, F>(f: F) -> impl Fn(&DynamicSequence) -> anyhow::Result<R>
    where
        A: TupleSequence,
        F: Fn(&A) -> anyhow::Result<R>,
    {
        move |seq: &DynamicSequence| {
            let a: A = seq.try_to_tuple()?;
            f(&a)
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime dynamic_sequence::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dynamic_sequence.rs
git commit -m "feat(cel-runtime): add DynamicSequence::adapt_fn_1"
```

---

### Task 11: `DynSegment::call_dyn_as_tuple`

**Files:**
- Modify: `cel-runtime/src/dyn_segment.rs`

**Interfaces:**
- Consumes: `TupleSequence` (Task 3); existing `DynSegment` internals (`stack_ids`, `argument_ids`, `segment`, `CALL_DYN_PTR`/`CALL_DYN_LEN`/`DynCallGuard`, `drop_tuple`, `DynTuple`).
- Produces: `DynSegment::call_dyn_as_tuple<T: TupleSequence>(&mut self, inputs: &[&dyn Any]) -> anyhow::Result<T>`.

This mirrors the existing `call_dyn_tuple` (which splits a tuple result into N separately-boxed
elements) but reconstructs one concrete `T` directly, by move, with no heap allocation of its own.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dyn_segment.rs`:

```rust
#[test]
fn call_dyn_as_tuple_reconstructs_the_tuple_result() -> Result<(), anyhow::Error> {
    let mut seg = DynSegment::new::<()>();
    let ambient_start = seg.current_stack_offset();
    seg.op0(|| 10i32);
    seg.op0(|| 2.5f64);
    seg.make_tuple(2, ambient_start);

    let result: (i32, f64) = seg.call_dyn_as_tuple(&[])?;
    assert_eq!(result, (10, 2.5));
    Ok(())
}

#[test]
fn call_dyn_as_tuple_is_repeatable() -> Result<(), anyhow::Error> {
    let mut seg = DynSegment::new::<()>();
    let ambient_start = seg.current_stack_offset();
    seg.op0(|| 1i32);
    seg.op0(|| 2i32);
    seg.make_tuple(2, ambient_start);

    let first: (i32, i32) = seg.call_dyn_as_tuple(&[])?;
    let second: (i32, i32) = seg.call_dyn_as_tuple(&[])?;
    assert_eq!(first, (1, 2));
    assert_eq!(second, (1, 2));
    Ok(())
}

#[test]
fn call_dyn_as_tuple_errors_if_result_is_not_a_tuple() {
    let mut seg = DynSegment::new::<()>();
    seg.op0(|| 5i32);
    let result = seg.call_dyn_as_tuple::<(i32,)>(&[]);
    assert!(result.is_err());
}

#[test]
fn call_dyn_as_tuple_errors_on_shape_mismatch() {
    let mut seg = DynSegment::new::<()>();
    let ambient_start = seg.current_stack_offset();
    seg.op0(|| 1i32);
    seg.op0(|| 2i32);
    seg.make_tuple(2, ambient_start);

    let result = seg.call_dyn_as_tuple::<(i32, f64)>(&[]);
    assert!(result.is_err(), "(i32, i32) should not match (i32, f64)");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime dyn_segment:: -- --exact call_dyn_as_tuple_reconstructs_the_tuple_result call_dyn_as_tuple_is_repeatable call_dyn_as_tuple_errors_if_result_is_not_a_tuple call_dyn_as_tuple_errors_on_shape_mismatch`
Expected: FAIL — `call_dyn_as_tuple` doesn't exist yet.

- [ ] **Step 3: Implement `call_dyn_as_tuple`**

Add `use crate::dynamic_sequence::TupleSequence;` to the imports at the top of
`cel-runtime/src/dyn_segment.rs`.

Add to `impl DynSegment` in `cel-runtime/src/dyn_segment.rs`, immediately after `call_dyn_tuple`:

```rust
    /// Executes the segment once and reconstructs its tuple result directly as the concrete
    /// tuple `T`, moving each element's bytes rather than splitting them into separate boxed
    /// values (contrast with [`call_dyn_tuple`](Self::call_dyn_tuple)).
    ///
    /// # Errors
    /// Returns `Err` if:
    /// - The segment requires pre-loaded arguments (created with a non-unit `Args` type).
    /// - The stack does not contain exactly one value after expression compilation.
    /// - That value is not a tuple, or its element `TypeId` sequence doesn't match `T`'s.
    /// - Any op returns an error during execution.
    ///
    /// - Complexity: O(n) in the number of ops, plus O(arity) to reconstruct `T`.
    pub fn call_dyn_as_tuple<T: TupleSequence>(&mut self, inputs: &[&dyn Any]) -> anyhow::Result<T> {
        ensure!(
            self.argument_ids.is_empty(),
            "call_dyn_as_tuple: segment requires {} pre-loaded argument(s); \
             use call_dyn_as_tuple only with push_arg-based segments",
            self.argument_ids.len()
        );
        ensure!(
            self.stack_ids.len() == 1,
            "call_dyn_as_tuple: expected exactly 1 value on stack, got {}",
            self.stack_ids.len()
        );
        let info = &self.stack_ids[0];
        ensure!(
            info.type_id == TypeId::of::<DynTuple>(),
            "call_dyn_as_tuple: expected a tuple result, got {}",
            info.type_name,
        );

        let mut expected = Vec::new();
        let mut max_align = 1usize;
        T::append_shape(&mut expected, 0, &mut max_align);
        ensure!(
            info.associated.len() == expected.len()
                && info
                    .associated
                    .iter()
                    .zip(&expected)
                    .all(|(a, b)| a.type_id == b.type_id),
            "call_dyn_as_tuple: tuple shape does not match `{}`",
            std::any::type_name::<T>(),
        );

        let tuple_size = info.size;
        let tuple_padding = info.padding;
        let associated = info.associated.clone();

        CALL_DYN_PTR.with(|c| c.set(inputs.as_ptr() as usize));
        CALL_DYN_LEN.with(|c| c.set(inputs.len()));
        let _guard = DynCallGuard;

        let mut stack = RawStack::with_base_alignment(self.segment.base_alignment());
        // Safety: the checks above verified the segment builds exactly one tuple value whose
        // shape matches T; call_dyn's own argument preconditions (no pre-loaded arguments) hold
        // identically here.
        unsafe {
            self.segment.call0_stack(&mut stack)?;
        }

        let tuple_base = stack.len() - tuple_size;
        // `associated`'s offsets describe the tuple's actual, already-correct on-stack layout
        // (computed by `make_tuple`) — using them, not a freshly-recomputed layout, is what
        // makes this sound regardless of any layout convention `TupleSequence` uses internally.
        let offsets: Vec<usize> = associated.iter().map(|a| a.offset).collect();
        let result: T = unsafe { stack.read_at(tuple_base, |base| T::read_from(base, &offsets)) };

        unsafe {
            stack.drop_at(tuple_base, |ptr| drop_tuple(ptr, &associated));
            stack.truncate_to(tuple_base, tuple_padding);
        }

        Ok(result)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime dyn_segment::`
Expected: PASS — all 4 new tests plus the full existing `dyn_segment.rs` suite.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dyn_segment.rs
git commit -m "feat(cel-runtime): add DynSegment::call_dyn_as_tuple"
```

---

### Task 12: End-to-end acceptance test — `Method::from_fn_1_1` via `adapt_fn_1`

**Files:**
- Modify: `cel-runtime/Cargo.toml` (add `adam-rs` dev-dependency)
- Create: `cel-runtime/tests/dynamic_sequence_adam_rs.rs`

**Interfaces:**
- Consumes: `DynamicSequence::{from_tuple, adapt_fn_1}` (Tasks 7, 10); `adam_rs::{Sheet, Method}` (existing, unmodified).

This is the concrete demonstration that a `DynamicSequence`-typed cell works with `adam-rs`'s
existing, unmodified `Method`/`Sheet` API — no `adam-rs` source changes, no adam-lang text parsing.

- [ ] **Step 1: Add the dev-dependency**

Add to `cel-runtime/Cargo.toml`:

```toml
[dev-dependencies]
adam-rs = { path = "../adam-rs" }
```

- [ ] **Step 2: Write the failing test**

Create `cel-runtime/tests/dynamic_sequence_adam_rs.rs`:

```rust
use adam_rs::{Method, Sheet};
use cel_runtime::DynamicSequence;

#[test]
fn dynamic_sequence_cell_works_with_unmodified_method_from_fn_1_1() {
    let mut sheet = Sheet::new();
    let input = sheet.add_cell(DynamicSequence::from_tuple((3i32, 4.5f64)));
    let output = sheet.add_cell(0.0f64);

    let f = DynamicSequence::adapt_fn_1(|t: &(i32, f64)| Ok(t.0 as f64 + t.1));
    sheet
        .add_relationship(vec![Method::from_fn_1_1(input, output, f)])
        .unwrap();

    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<f64>(output).unwrap(), 7.5);
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p cel-runtime --test dynamic_sequence_adam_rs`
Expected: FAIL to compile (`adam-rs` not yet a dependency) until Step 1 is applied; after Step 1,
FAIL only if `DynamicSequence`/`adapt_fn_1` have a bug — but per Tasks 1–10 these already exist and
pass their own unit tests, so this test is expected to already PASS once the dev-dependency is
added. Run it anyway to confirm.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p cel-runtime --test dynamic_sequence_adam_rs`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/Cargo.toml cel-runtime/tests/dynamic_sequence_adam_rs.rs
git commit -m "test(cel-runtime): add end-to-end DynamicSequence + adam-rs Method acceptance test"
```

---

### Task 13: Acceptance test — `add_conditional` match-cell via `PartialEq`, and final workspace verification

**Files:**
- Modify: `cel-runtime/tests/dynamic_sequence_adam_rs.rs`

**Interfaces:**
- Consumes: `DynamicSequence: PartialEq` (Task 8); `adam_rs::{Sheet, Method}` (existing, unmodified).

This demonstrates the specific reason `DynamicSequence` must implement `PartialEq` (per the design
spec): `Sheet::add_conditional`'s branch-key matching, via `CellData.eq_fn`.

- [ ] **Step 1: Write the failing test**

Add to `cel-runtime/tests/dynamic_sequence_adam_rs.rs`:

```rust
#[test]
fn dynamic_sequence_cell_selects_conditional_branch_via_partial_eq() {
    let mut sheet = Sheet::new();
    let match_cell = sheet.add_cell(DynamicSequence::from_tuple((1i32, 2i32)));
    let output = sheet.add_cell(0i32);

    let f = DynamicSequence::adapt_fn_1(|_: &(i32, i32)| Ok(99i32));
    let rel = sheet
        .add_relationship(vec![Method::from_fn_1_1(match_cell, output, f)])
        .unwrap();

    sheet
        .add_conditional::<DynamicSequence>(
            match_cell,
            vec![(vec![DynamicSequence::from_tuple((1i32, 2i32))], vec![rel])],
            vec![],
        )
        .unwrap();

    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(output).unwrap(), 99);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cel-runtime --test dynamic_sequence_adam_rs dynamic_sequence_cell_selects_conditional_branch_via_partial_eq`
Expected: given `PartialEq` already exists from Task 8, this is expected to already PASS — run it
to confirm the whole path (branch matching, relationship activation, propagation) behaves as
described.

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo test -p cel-runtime --test dynamic_sequence_adam_rs`
Expected: PASS (both acceptance tests).

- [ ] **Step 4: Run full workspace verification**

Run, in order:

```bash
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

Expected: all PASS, zero compiler warnings, zero clippy warnings.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/tests/dynamic_sequence_adam_rs.rs
git commit -m "test(cel-runtime): add conditional-branch-matching acceptance test for DynamicSequence"
```

---

## Self-Review Notes

- **Spec coverage:** `DynamicSequence` type built on `RawStack` (Tasks 1, 7–9); type-safe,
  nestable, arity 1–12 conversions to/from Rust tuples (Tasks 2–6, 9); extraction from a live
  `DynSegment` evaluation (Task 11); `adapt_fn_1` (Task 10); demonstration through unmodified
  `adam-rs` `Method`/`Sheet` (Task 12); the `PartialEq`/`add_conditional` rationale from the spec
  (Task 13). No `adam-rs` source file is modified anywhere in this plan.
- **Deferred per spec's "Out of scope":** adam-lang grammar/parser changes, the broader
  `RawStack`/`RawSequence`/CEL-tuple-representation redesign (tracked in
  [stlab/cel-rs#80](https://github.com/stlab/cel-rs/issues/80)), and `adapt_fn_2`/`adapt_fn_2_1`
  are intentionally not tasks here.
- **Type/name consistency:** `SequenceElement`'s fields (`type_id`, `type_name`, `offset`, `size`,
  `align`, `drop`, `clone`, `eq`) introduced in Task 2 are used identically by `push_element`
  (Task 2), every `TupleSequence` impl (Tasks 3–6), and `DynamicSequence`'s `Drop`/`Clone`/
  `PartialEq` (Tasks 7–8). `TupleSequence::{append_shape, write_into, read_from, clone_from}`
  signatures introduced in Task 3 are used identically through Tasks 4–11.
