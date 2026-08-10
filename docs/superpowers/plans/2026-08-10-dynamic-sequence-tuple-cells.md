# DynamicSequence Tuple Cells Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `DynamicSequence` type to `cel-runtime` that lets an `adam-rs` cell hold a CEL
tuple value of arbitrary (compile-time-known, for this plan) shape, converts type-safely to/from
concrete Rust tuples (nestable, arity 1–12), and can be extracted from a live `DynSegment`
evaluation — all demonstrated end-to-end through `adam-rs`'s existing, unmodified `Sheet`/`Method`/
`Condition` API.

**Architecture:** `DynamicSequence` owns a `RawStack` (sized to its own elements' actual max
alignment, never a blanket constant) plus a `Vec<SequenceElement>` shape descriptor carrying each
element's `TypeId`, offset, size, align, and per-element `drop`/`clone`/`eq` function pointers. All
the actual byte-layout work (offsets, writes, reads, clones) is implemented exactly once,
generically, via a `SequenceList` trait over the cons-list `tuple_list.rs`'s existing
`IntoTupleList` already produces (`()` and `(H, T)` — two impls cover every arity). A thin
`TupleSequence` trait then bridges each concrete Rust tuple arity to/from that cons-list via a
single, fully-safe destructuring line per arity (the reverse of `IntoTupleList::into_tuple_list`).
`DynSegment` gains a method to reconstruct a live on-stack CEL tuple result directly as a concrete
Rust tuple. `adam-rs` itself is not modified at all — its `Sheet`/`Method`/`Condition` API is
already fully generic over `Any + PartialEq + 'static`.

**Tech Stack:** Rust, `cel-runtime` (`RawStack`, `DynSegment`, `tuple_list::IntoTupleList`),
`anyhow` for fallible ops, `adam-rs` (dev-dependency only, for the acceptance tests).

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
  before the final commit of the whole plan (Task 11).
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
//! trait, implemented for arities 1 through 12. All the actual byte-layout work is implemented
//! exactly once, generically, by [`SequenceList`], over the cons-list `tuple_list.rs`'s
//! `IntoTupleList` already produces.

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

### Task 3: `SequenceList` — the generic cons-list layout engine

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`

**Interfaces:**
- Consumes: `push_element`, `SequenceElement` (Task 2).
- Produces: `pub trait SequenceList: Sized { fn append_shape(...) -> usize; unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]); unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self; unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self; }`, implemented for `()` and `(H, T)`.

This is the entire mechanical, unsafe, byte-layout engine — written exactly twice, covering every
tuple arity generically. `()` and `(H, T)` are precisely the two shapes `tuple_list.rs`'s
`IntoTupleList::into_tuple_list()` ever produces (e.g. `(1, 2, 3).into_tuple_list() == (1, (2, (3,
())))`), so no per-arity duplication of this logic is needed anywhere in this plan.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dynamic_sequence.rs`:

```rust
    #[test]
    fn sequence_list_base_case_is_a_no_op() {
        let mut shape = Vec::new();
        let mut max_align = 1usize;
        let end = <()>::append_shape(&mut shape, 0, &mut max_align);
        assert_eq!(end, 0);
        assert!(shape.is_empty());
    }

    #[test]
    fn sequence_list_cons_case_round_trips_two_elements() {
        let mut shape = Vec::new();
        let mut max_align = 1usize;
        <(i32, (f64, ()))>::append_shape(&mut shape, 0, &mut max_align);
        assert_eq!(shape[0].offset, 0);
        assert_eq!(shape[1].offset, 8); // i32 at [0,4); f64 aligned up to 8
        let offsets: Vec<usize> = shape.iter().map(|e| e.offset).collect();

        let mut buf = [0u8; 16];
        let list = (7i32, (2.5f64, ()));
        unsafe { list.write_into(buf.as_mut_ptr(), &offsets) };
        let cloned = unsafe { <(i32, (f64, ()))>::clone_from(buf.as_ptr(), &offsets) };
        assert_eq!(cloned, (7, (2.5, ())));
        let read = unsafe { <(i32, (f64, ()))>::read_from(buf.as_ptr(), &offsets) };
        assert_eq!(read, (7, (2.5, ())));
    }

    #[test]
    fn sequence_list_supports_nested_tuple_elements() {
        // A nested tuple field needs no special handling: (i32, i32) is just an
        // ordinary 'static + Clone + PartialEq element type here.
        let mut shape = Vec::new();
        let mut max_align = 1usize;
        <(i32, ((i32, i32), (bool, ())))>::append_shape(&mut shape, 0, &mut max_align);
        let offsets: Vec<usize> = shape.iter().map(|e| e.offset).collect();

        let mut buf = [0u8; 16];
        let list = (1i32, ((2i32, 3i32), (true, ())));
        unsafe { list.write_into(buf.as_mut_ptr(), &offsets) };
        let read = unsafe { <(i32, ((i32, i32), (bool, ())))>::read_from(buf.as_ptr(), &offsets) };
        assert_eq!(read, (1, ((2, 3), (true, ()))));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime dynamic_sequence:: -- --exact sequence_list_base_case_is_a_no_op sequence_list_cons_case_round_trips_two_elements sequence_list_supports_nested_tuple_elements`
Expected: FAIL with "no method named `append_shape`" (trait doesn't exist yet).

- [ ] **Step 3: Implement `SequenceList`**

Add to `cel-runtime/src/dynamic_sequence.rs`, after `push_element` (before the `tests` module):

```rust
/// Cons-list (`()` or `(H, T)`, matching `tuple_list.rs`'s `IntoTupleList` output) that knows how
/// to lay itself out as a [`DynamicSequence`] shape and move/clone itself to and from raw bytes
/// at that layout.
///
/// Implemented exactly twice — for `()` and `(H, T)` — covering every tuple arity generically; no
/// per-arity unsafe code is needed here or anywhere downstream.
pub trait SequenceList: Sized {
    /// Appends this list's own elements (head-first order) to `out`, computing each one's offset
    /// from `offset` (the byte position immediately after the previous element) and folding each
    /// element's alignment into `*max_align`. Returns the byte position immediately after the
    /// last element appended.
    ///
    /// - Complexity: O(length).
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize;

    /// Writes this list's fields into `dst`, at the positions in `offsets` (which must be
    /// exactly this list's own element offsets, head-first, as produced by
    /// [`append_shape`](Self::append_shape)), consuming `self`.
    ///
    /// - Complexity: O(length).
    ///
    /// # Safety
    /// `dst` must be valid for writes covering every offset + size of this list's elements.
    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]);

    /// Reads this list back out of `src` by moving each field's bytes, at the positions in
    /// `offsets`.
    ///
    /// - Complexity: O(length).
    ///
    /// # Safety
    /// `src` must point to a live value whose layout matches `offsets`; the caller must not
    /// separately drop those bytes afterward.
    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self;

    /// Reads this list back out of `src` by cloning each field's bytes, at the positions in
    /// `offsets`, leaving `src` untouched.
    ///
    /// - Complexity: O(length).
    ///
    /// # Safety
    /// `src` must point to a live value whose layout matches `offsets`.
    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self;
}

impl SequenceList for () {
    fn append_shape(_out: &mut Vec<SequenceElement>, offset: usize, _max_align: &mut usize) -> usize {
        offset
    }

    unsafe fn write_into(self, _dst: *mut u8, _offsets: &[usize]) {}

    unsafe fn read_from(_src: *const u8, _offsets: &[usize]) -> Self {}

    unsafe fn clone_from(_src: *const u8, _offsets: &[usize]) -> Self {}
}

impl<H: 'static + Clone + PartialEq, T: SequenceList> SequenceList for (H, T) {
    fn append_shape(out: &mut Vec<SequenceElement>, offset: usize, max_align: &mut usize) -> usize {
        let offset = push_element::<H>(out, offset, max_align);
        T::append_shape(out, offset, max_align)
    }

    unsafe fn write_into(self, dst: *mut u8, offsets: &[usize]) {
        unsafe {
            std::ptr::write(dst.add(offsets[0]).cast::<H>(), self.0);
            T::write_into(self.1, dst, &offsets[1..]);
        }
    }

    unsafe fn read_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            let h = std::ptr::read(src.add(offsets[0]).cast::<H>());
            let t = T::read_from(src, &offsets[1..]);
            (h, t)
        }
    }

    unsafe fn clone_from(src: *const u8, offsets: &[usize]) -> Self {
        unsafe {
            let h = (*src.add(offsets[0]).cast::<H>()).clone();
            let t = T::clone_from(src, &offsets[1..]);
            (h, t)
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
git commit -m "feat(cel-runtime): add SequenceList generic cons-list layout engine"
```

---

### Task 4: `TupleSequence` — arity 1–12 via one-line reversal of `IntoTupleList`

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`
- Modify: `cel-runtime/src/lib.rs` (re-export `tuple_list`)

**Interfaces:**
- Consumes: `SequenceList` (Task 3); `crate::tuple_list::IntoTupleList` (existing).
- Produces: `pub trait TupleSequence: IntoTupleList + Sized where Self::Output: SequenceList { fn from_list(list: Self::Output) -> Self; }`, implemented for tuples of arity 1 through 12.

Each per-arity impl is now a single, fully-safe destructuring line — the reverse of
`IntoTupleList::into_tuple_list`. `IntoTupleList` itself (the forward direction, `T -> Self::Output`)
already exists for every one of these arities via the existing blanket
`impl<T: IntoList> IntoTupleList for T` in `tuple_list.rs`, backed by `list_traits.rs`'s existing
`IntoList` impls — nothing about that forward direction needs to be written in this plan.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dynamic_sequence.rs`:

```rust
    #[test]
    fn tuple_sequence_from_list_reverses_into_tuple_list_for_several_arities() {
        assert_eq!(<(i32,)>::from_list((1i32, ())), (1,));
        assert_eq!(
            <(i32, f64)>::from_list((1i32, (2.5f64, ()))),
            (1, 2.5)
        );
        assert_eq!(
            <(i32, f64, bool)>::from_list((1i32, (2.5f64, (true, ())))),
            (1, 2.5, true)
        );
        let full_arity_12 = (1i32, 2i32, 3i32, 4i32, 5i32, 6i32, 7i32, 8i32, 9i32, 10i32, 11i32, 12i32);
        assert_eq!(
            <(i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32)>::from_list(
                full_arity_12.into_tuple_list()
            ),
            full_arity_12
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cel-runtime dynamic_sequence::tests::tuple_sequence_from_list_reverses_into_tuple_list_for_several_arities`
Expected: FAIL with "no function or associated item named `from_list`" (trait doesn't exist yet).

- [ ] **Step 3: Implement `TupleSequence` and arities 1–12**

Add `use crate::tuple_list::IntoTupleList;` to the imports at the top of
`cel-runtime/src/dynamic_sequence.rs`.

Add to `cel-runtime/src/lib.rs`, in the `pub use` block, un-commenting the existing line:

```rust
pub use tuple_list::*;
```

Add to `cel-runtime/src/dynamic_sequence.rs`, after the `(H, T)` `SequenceList` impl (before the
`tests` module):

```rust
/// Bridges a concrete Rust tuple `T` to and from its [`IntoTupleList`] cons-list representation,
/// so [`DynamicSequence`] can convert to/from `T` while all the actual byte-layout work is
/// handled generically by [`SequenceList`].
///
/// Implemented for tuples of arity 1 through 12 — the same range `cel-runtime`'s `IntoList`
/// supports — via a single-line reversal of [`IntoTupleList::into_tuple_list`]; no unsafe code
/// appears in any of these impls. A nested tuple field needs no special handling: it's simply an
/// ordinary `'static + Clone + PartialEq` element type to its enclosing tuple's own impl.
pub trait TupleSequence: IntoTupleList + Sized
where
    Self::Output: SequenceList,
{
    /// Reconstructs `Self` from its cons-list representation — the reverse of
    /// [`IntoTupleList::into_tuple_list`].
    fn from_list(list: Self::Output) -> Self;
}

impl<A: 'static + Clone + PartialEq> TupleSequence for (A,) {
    fn from_list(list: Self::Output) -> Self {
        let (a, ()) = list;
        (a,)
    }
}

impl<A: 'static + Clone + PartialEq, B: 'static + Clone + PartialEq> TupleSequence for (A, B) {
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, ())) = list;
        (a, b)
    }
}

impl<A: 'static + Clone + PartialEq, B: 'static + Clone + PartialEq, C: 'static + Clone + PartialEq>
    TupleSequence for (A, B, C)
{
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, ()))) = list;
        (a, b, c)
    }
}

impl<
    A: 'static + Clone + PartialEq,
    B: 'static + Clone + PartialEq,
    C: 'static + Clone + PartialEq,
    D: 'static + Clone + PartialEq,
> TupleSequence for (A, B, C, D)
{
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, ())))) = list;
        (a, b, c, d)
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
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, (e, ()))))) = list;
        (a, b, c, d, e)
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
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, (e, (f, ())))))) = list;
        (a, b, c, d, e, f)
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
> TupleSequence for (A, B, C, D, E, F, G)
{
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, (e, (f, (g, ()))))))) = list;
        (a, b, c, d, e, f, g)
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
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, (e, (f, (g, (h, ())))))))) = list;
        (a, b, c, d, e, f, g, h)
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
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, (e, (f, (g, (h, (i, ()))))))))) = list;
        (a, b, c, d, e, f, g, h, i)
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
> TupleSequence for (A, B, C, D, E, F, G, H, I, J)
{
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, (e, (f, (g, (h, (i, (j, ())))))))))) = list;
        (a, b, c, d, e, f, g, h, i, j)
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
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, (e, (f, (g, (h, (i, (j, (k, ()))))))))))) = list;
        (a, b, c, d, e, f, g, h, i, j, k)
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
    fn from_list(list: Self::Output) -> Self {
        let (a, (b, (c, (d, (e, (f, (g, (h, (i, (j, (k, (l, ())))))))))))) = list;
        (a, b, c, d, e, f, g, h, i, j, k, l)
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p cel-runtime dynamic_sequence::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dynamic_sequence.rs cel-runtime/src/lib.rs
git commit -m "feat(cel-runtime): add TupleSequence for arity 1-12 via IntoTupleList reversal"
```

---

### Task 5: `DynamicSequence` struct, `from_tuple`, and `Drop`

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`

**Interfaces:**
- Consumes: `TupleSequence`, `SequenceList`, `SequenceElement` (Tasks 2–4); `RawStack::{with_base_alignment, reserve_and_write, drop_at}` (Task 1, and existing `cel-runtime` API).
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
        let list = value.into_tuple_list();
        let mut shape = Vec::new();
        let mut max_align = 1usize;
        let end = T::Output::append_shape(&mut shape, 0, &mut max_align);
        let total_size = align_index(max_align, end);
        let offsets: Vec<usize> = shape.iter().map(|e| e.offset).collect();

        let mut buffer = crate::raw_stack::RawStack::with_base_alignment(max_align);
        unsafe {
            buffer.reserve_and_write(max_align, total_size, |dst| {
                unsafe { list.write_into(dst, &offsets) };
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

### Task 6: `DynamicSequence::Clone` and `PartialEq`

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`

**Interfaces:**
- Consumes: `DynamicSequence` (Task 5); `RawStack::{read_at, reserve_and_write, with_base_alignment}`.
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
                self.buffer
                    .read_at(elem.offset, |a| other.buffer.read_at(elem.offset, |b| (elem.eq)(a, b)))
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

### Task 7: `DynamicSequence::try_into_tuple` and `try_to_tuple`

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`

**Interfaces:**
- Consumes: `DynamicSequence`, `TupleSequence` (Tasks 4–6).
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
        T::Output::append_shape(&mut expected, 0, &mut max_align);
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
        let list = unsafe { self.buffer.read_at(0, |base| T::Output::read_from(base, &offsets)) };
        // Fields were just moved out of `self.buffer`'s bytes above; clearing `shape` makes
        // `Drop`'s element loop a no-op so those fields aren't dropped a second time. `buffer`'s
        // own backing allocation is still freed normally when `self` goes out of scope below.
        self.shape.clear();
        Ok(T::from_list(list))
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
        let list = unsafe { self.buffer.read_at(0, |base| T::Output::clone_from(base, &offsets)) };
        Ok(T::from_list(list))
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

### Task 8: `DynamicSequence::adapt_fn_1`

**Files:**
- Modify: `cel-runtime/src/dynamic_sequence.rs`

**Interfaces:**
- Consumes: `DynamicSequence::try_to_tuple` (Task 7).
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

### Task 9: `DynSegment::call_dyn_as_tuple`

**Files:**
- Modify: `cel-runtime/src/dyn_segment.rs`

**Interfaces:**
- Consumes: `TupleSequence`, `SequenceList` (Tasks 3–4); existing `DynSegment` internals (`stack_ids`, `argument_ids`, `segment`, `CALL_DYN_PTR`/`CALL_DYN_LEN`/`DynCallGuard`, `drop_tuple`, `DynTuple`).
- Produces: `DynSegment::call_dyn_as_tuple<T: TupleSequence>(&mut self, inputs: &[&dyn Any]) -> anyhow::Result<T>`.

This mirrors the existing `call_dyn_tuple` (which splits a tuple result into N separately-boxed
elements) but reconstructs one concrete `T` directly, by move, with no heap allocation of its own.
Note that extraction reads bytes at the *live tuple's own* `associated` offsets (already correctly
computed by `make_tuple`), not at freshly-recomputed `TupleSequence`/`SequenceList` offsets — the
two are independent, purpose-specific offset computations that are never required to agree with
each other.

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

Add `use crate::dynamic_sequence::{SequenceList, TupleSequence};` to the imports at the top of
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
        T::Output::append_shape(&mut expected, 0, &mut max_align);
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
        // makes this sound regardless of any layout convention `SequenceList` uses internally.
        let offsets: Vec<usize> = associated.iter().map(|a| a.offset).collect();
        let list: T::Output =
            unsafe { stack.read_at(tuple_base, |base| T::Output::read_from(base, &offsets)) };
        let result = T::from_list(list);

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

### Task 10: End-to-end acceptance test — `Method::from_fn_1_1` via `adapt_fn_1`

**Files:**
- Modify: `cel-runtime/Cargo.toml` (add `adam-rs` dev-dependency)
- Create: `cel-runtime/tests/dynamic_sequence_adam_rs.rs`

**Interfaces:**
- Consumes: `DynamicSequence::{from_tuple, adapt_fn_1}` (Tasks 5, 8); `adam_rs::{Sheet, Method}` (existing, unmodified).

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
FAIL only if `DynamicSequence`/`adapt_fn_1` have a bug — but per Tasks 1–8 these already exist and
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

### Task 11: Acceptance test — `add_conditional` match-cell via `PartialEq`, and final workspace verification

**Files:**
- Modify: `cel-runtime/tests/dynamic_sequence_adam_rs.rs`

**Interfaces:**
- Consumes: `DynamicSequence: PartialEq` (Task 6); `adam_rs::{Sheet, Method}` (existing, unmodified).

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
Expected: given `PartialEq` already exists from Task 6, this is expected to already PASS — run it
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

- **Spec coverage:** `DynamicSequence` type built on `RawStack` (Tasks 1, 5–7); type-safe,
  nestable, arity 1–12 conversions to/from Rust tuples (Tasks 2–4, 7), implemented via a generic
  `SequenceList` layout engine (Task 3) plus a one-line-per-arity `TupleSequence` reversal of the
  existing `IntoTupleList` (Task 4) rather than 12 fully-duplicated unsafe impls; extraction from a
  live `DynSegment` evaluation (Task 9); `adapt_fn_1` (Task 8); demonstration through unmodified
  `adam-rs` `Method`/`Sheet` (Task 10); the `PartialEq`/`add_conditional` rationale from the spec
  (Task 11). No `adam-rs` source file is modified anywhere in this plan.
- **Deferred per spec's "Out of scope":** adam-lang grammar/parser changes, the broader
  `RawStack`/`RawSequence`/CEL-tuple-representation redesign (tracked in
  [stlab/cel-rs#80](https://github.com/stlab/cel-rs/issues/80)), and `adapt_fn_2`/`adapt_fn_2_1`
  are intentionally not tasks here.
- **Type/name consistency:** `SequenceElement`'s fields (`type_id`, `type_name`, `offset`, `size`,
  `align`, `drop`, `clone`, `eq`) introduced in Task 2 are used identically by `push_element`
  (Task 2), `SequenceList`'s two impls (Task 3), and `DynamicSequence`'s `Drop`/`Clone`/`PartialEq`
  (Tasks 5–6). `SequenceList::{append_shape, write_into, read_from, clone_from}` (Task 3) and
  `TupleSequence::from_list` (Task 4) signatures are used identically through Tasks 5–9 via
  `T::Output` (never re-derived or renamed).
