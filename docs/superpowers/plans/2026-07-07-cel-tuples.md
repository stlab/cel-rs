# CEL Tuples Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add tuple literals (`("Hello", 42)`), `.N` field indexing, and a generalized interop-registration mechanism for tuple-typed user operations to CEL, per `docs/superpowers/specs/2026-07-07-cel-tuples-design.md`.

**Architecture:** Tuples are a type-erased, in-place runtime representation on `RawStack` — no heap allocation for the tuple value itself (one small, transient, per-element scratch buffer is used during `.N` extraction; see Task 5). `StackInfo` and a new `AssociatedType` carry per-element offset/size/align/dropper metadata computed at parse time. Construction repacks already-evaluated elements into a context-independent layout; indexing pops the whole tuple and pushes back just the extracted element. Interop with user Rust code goes through a generalization of the existing `OpSignature`/`BuiltinScope` array-scan dispatch, bridging to a concrete `CStackList<...>` type.

**Tech Stack:** Rust, `cel-runtime` (stack/segment machinery), `cel-parser` (recursive-descent parser, `proc_macro2` tokens), `anyhow` for fallible ops, existing `phf`/signature-table patterns in `cel-parser/src/op_table.rs`.

## Global Constraints

- Format with `cargo fmt --all` before every commit (enforced by pre-commit hook).
- Every public function needs a contract-style `///` doc comment (Summary, Preconditions as `debug_assert!`, `# Errors`/`# Safety` where applicable, Postconditions, Complexity if not O(1)) per the root `CLAUDE.md`.
- Unit tests are derived from contract/public interface only — never from implementation internals.
- No heap allocation for the tuple's *persistent* on-stack representation. A small, transient, single-element scratch buffer during `.N` extraction (Task 5) is an accepted, documented exception — not the "boxed tuple" pattern the design avoids.
- Run `cargo test --workspace` and `cargo clippy --workspace --exclude begin -- -D warnings` before the final commit of each task.

---

### Task 1: `RawStack` — type-erased push and length

**Files:**
- Modify: `cel-runtime/src/raw_stack.rs`

**Interfaces:**
- Produces: `RawStack::len(&self) -> usize`, `unsafe fn RawStack::push_raw(&mut self, align: usize, size: usize, src: *const u8) -> bool`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/raw_stack.rs`:

```rust
#[test]
fn len_reflects_pushed_bytes() {
    let mut stack = RawStack::with_base_alignment(align_of::<u32>());
    assert_eq!(stack.len(), 0);
    let _ = stack.push(7u32);
    assert_eq!(stack.len(), size_of::<u32>());
}

#[test]
fn push_raw_round_trips_like_push() {
    let mut stack = RawStack::with_base_alignment(align_of::<f64>());
    let padding1 = stack.push(1u8);
    let value = 3.14f64;
    let padding2 =
        unsafe { stack.push_raw(align_of::<f64>(), size_of::<f64>(), (&value as *const f64).cast::<u8>()) };
    let popped: f64 = unsafe { stack.pop(padding2) };
    assert_eq!(popped, 3.14);
    let popped_u8: u8 = unsafe { stack.pop(padding1) };
    assert_eq!(popped_u8, 1);
}

#[test]
fn push_raw_padding_matches_typed_push() {
    let mut stack_a = RawStack::with_base_alignment(align_of::<f64>());
    let _ = stack_a.push(1u8);
    let padding_typed = stack_a.push(2.5f64);

    let mut stack_b = RawStack::with_base_alignment(align_of::<f64>());
    let _ = stack_b.push(1u8);
    let value = 2.5f64;
    let padding_raw = unsafe {
        stack_b.push_raw(align_of::<f64>(), size_of::<f64>(), (&value as *const f64).cast::<u8>())
    };
    assert_eq!(padding_typed, padding_raw);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime raw_stack:: -- --exact len_reflects_pushed_bytes push_raw_round_trips_like_push push_raw_padding_matches_typed_push`
Expected: FAIL with "no method named `len`" / "no method named `push_raw`".

- [ ] **Step 3: Implement `len` and `push_raw`**

Add to `impl RawStack` in `cel-runtime/src/raw_stack.rs`, right after `with_base_alignment`:

```rust
    /// Returns the number of bytes currently on the stack.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }
```

Add after the existing `push<T>` method:

```rust
    /// Pushes `size` raw bytes from `src`, aligned to `align`, using the same
    /// padding/marker-byte bookkeeping as [`push`](Self::push).
    ///
    /// - Precondition: `align` is a power of two.
    ///
    /// # Safety
    /// `src` must be valid for reads of `size` bytes.
    pub unsafe fn push_raw(&mut self, align: usize, size: usize, src: *const u8) -> bool {
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
            std::ptr::copy_nonoverlapping(
                src,
                self.buffer.as_mut_ptr().add(aligned_index).cast::<u8>(),
                size,
            );
        }
        aligned_index - len > 0
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime raw_stack::`
Expected: PASS (all tests in the module, including the 3 new ones and existing ones).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/raw_stack.rs
git commit -m "feat(cel-runtime): add RawStack::len and push_raw"
```

---

### Task 2: `RawStack` — surgical drop/copy/truncate primitives

**Files:**
- Modify: `cel-runtime/src/raw_stack.rs`

**Interfaces:**
- Consumes: `RawStack::len` (Task 1).
- Produces: `unsafe fn RawStack::copy_from(&self, offset: usize, size: usize, dst: *mut u8)`, `unsafe fn RawStack::drop_at(&mut self, offset: usize, run_drop: impl FnOnce(*mut u8))`, `unsafe fn RawStack::truncate_to(&mut self, new_len: usize, padding: bool)`, `unsafe fn RawStack::drop_sized(&mut self, size: usize, padding: bool, run_drop: impl FnOnce(*mut u8))`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/raw_stack.rs`:

```rust
#[test]
fn copy_from_reads_bytes_at_offset() {
    let mut stack = RawStack::with_base_alignment(align_of::<u32>());
    let _ = stack.push(10u32);
    let _ = stack.push(20u32);
    let mut buf = [0u8; 4];
    unsafe { stack.copy_from(0, 4, buf.as_mut_ptr()) };
    assert_eq!(u32::from_ne_bytes(buf), 10);
}

#[test]
fn drop_at_runs_destructor_without_changing_length() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct DropCounter(Arc<AtomicUsize>);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let count = Arc::new(AtomicUsize::new(0));
    let mut stack = RawStack::with_base_alignment(align_of::<DropCounter>());
    let _ = stack.push(DropCounter(count.clone()));
    let len_before = stack.len();
    unsafe {
        stack.drop_at(0, |ptr| unsafe { std::ptr::drop_in_place(ptr.cast::<DropCounter>()) });
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(stack.len(), len_before);
}

#[test]
fn truncate_to_strips_recorded_padding() {
    let mut stack = RawStack::with_base_alignment(align_of::<f64>());
    let _ = stack.push(1u8);
    let padding = stack.push(2.5f64); // padding == true: 7 bytes inserted before the f64
    let len_with_value = stack.len();
    unsafe { stack.truncate_to(len_with_value - size_of::<f64>(), padding) };
    assert_eq!(stack.len(), 1); // back to just the u8
}

#[test]
fn drop_sized_combines_drop_at_and_truncate_to() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct DropCounter(Arc<AtomicUsize>);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let count = Arc::new(AtomicUsize::new(0));
    let mut stack = RawStack::with_base_alignment(align_of::<DropCounter>());
    let _ = stack.push(1u8);
    let padding = stack.push(DropCounter(count.clone()));
    unsafe {
        stack.drop_sized(size_of::<DropCounter>(), padding, |ptr| unsafe {
            std::ptr::drop_in_place(ptr.cast::<DropCounter>())
        });
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(stack.len(), 1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime raw_stack:: -- --exact copy_from_reads_bytes_at_offset drop_at_runs_destructor_without_changing_length truncate_to_strips_recorded_padding drop_sized_combines_drop_at_and_truncate_to`
Expected: FAIL with "no method named" errors.

- [ ] **Step 3: Implement the four methods**

Add to `impl RawStack` in `cel-runtime/src/raw_stack.rs`, after `push_raw`:

```rust
    /// Copies `size` bytes starting at absolute buffer offset `offset` into `dst`.
    ///
    /// # Safety
    /// `offset..offset + size` must be within the currently-initialized buffer;
    /// `dst` must be valid for writes of `size` bytes.
    pub unsafe fn copy_from(&self, offset: usize, size: usize, dst: *mut u8) {
        unsafe {
            std::ptr::copy_nonoverlapping(self.buffer.as_ptr().add(offset).cast::<u8>(), dst, size);
        }
    }

    /// Drops a value in place at absolute buffer offset `offset`, without
    /// altering the stack's tracked length.
    ///
    /// # Safety
    /// `offset` must point to a live, valid value; `run_drop` must correctly
    /// run that value's destructor given a pointer to its start.
    pub unsafe fn drop_at(&mut self, offset: usize, run_drop: impl FnOnce(*mut u8)) {
        unsafe { run_drop(self.buffer.as_mut_ptr().add(offset).cast::<u8>()) };
    }

    /// Truncates the stack back to `new_len`, additionally stripping `padding`
    /// bytes that preceded the removed region (scanned the same way
    /// [`pop`](Self::pop) does).
    ///
    /// # Safety
    /// No live (undropped) value may exist at or above `new_len`.
    pub unsafe fn truncate_to(&mut self, new_len: usize, padding: bool) {
        let padding_count = if padding {
            self.buffer[..new_len]
                .iter()
                .rev()
                .take_while(|&x| unsafe { x.assume_init() == 0 })
                .count()
                + 1
        } else {
            0
        };
        self.buffer.truncate(new_len - padding_count);
    }

    /// Drops a value of `size` bytes at the top of the stack in place, then
    /// removes it (and any padding that preceded it).
    ///
    /// # Safety
    /// The top `size` bytes (plus padding if `padding` is true) must be a
    /// live, valid value; `run_drop` must correctly run its destructor given a
    /// pointer to its start.
    pub unsafe fn drop_sized(&mut self, size: usize, padding: bool, run_drop: impl FnOnce(*mut u8)) {
        let p = self.buffer.len() - size;
        unsafe {
            self.drop_at(p, run_drop);
            self.truncate_to(p, padding);
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime raw_stack::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/raw_stack.rs
git commit -m "feat(cel-runtime): add RawStack drop/copy/truncate primitives"
```

---

### Task 3: `RawStack::repack` — context-independent layout construction

**Files:**
- Modify: `cel-runtime/src/raw_stack.rs`

**Interfaces:**
- Consumes: `RawStack::len` (Task 1).
- Produces: `unsafe fn RawStack::repack(&mut self, ambient_start: usize, dest_base: usize, total_size: usize, src_offsets: &[usize], dest_offsets: &[usize], sizes: &[usize]) -> bool`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `cel-runtime/src/raw_stack.rs`:

```rust
#[test]
fn repack_moves_elements_to_ideal_offsets_and_reports_padding() {
    // Ambient layout: [u8 @0][pad][u32 @4][u8 @8] — u8 then u32 then u8, each
    // pushed with ordinary alignment relative to a 1-byte ambient start.
    let mut stack = RawStack::with_base_alignment(align_of::<u32>());
    let ambient_start = stack.len(); // 0
    let _ = stack.push(0xAAu8); // element 0: ambient offset 0
    let _p1 = stack.push(0xBBBB_BBBBu32); // element 1: ambient offset 4 (1 byte padded to 4)
    let _p2 = stack.push(0xCCu8); // element 2: ambient offset 8

    // Ideal (self-contained) layout for (u8, u32, u8) from a zero base:
    // offset 0 (u8), offset 4 (u32, aligned up from 1), offset 8 (u8) -> total 9,
    // rounded to the tuple's own max align (4) -> total_size 12.
    let src_offsets = [0usize, 4, 8];
    let dest_offsets = [0usize, 4, 8];
    let sizes = [1usize, 4, 1];
    let total_size = 12usize;
    let dest_base = 0usize; // ambient_start (0) is already 4-aligned

    let padding = unsafe {
        stack.repack(ambient_start, dest_base, total_size, &src_offsets, &dest_offsets, &sizes)
    };
    assert!(!padding, "ambient_start was already aligned; no leading pad expected");
    assert_eq!(stack.len(), dest_base + total_size);

    let mut a = [0u8; 1];
    let mut b = [0u8; 4];
    let mut c = [0u8; 1];
    unsafe {
        stack.copy_from(dest_base, 1, a.as_mut_ptr());
        stack.copy_from(dest_base + 4, 4, b.as_mut_ptr());
        stack.copy_from(dest_base + 8, 1, c.as_mut_ptr());
    }
    assert_eq!(a[0], 0xAA);
    assert_eq!(u32::from_ne_bytes(b), 0xBBBB_BBBB);
    assert_eq!(c[0], 0xCC);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cel-runtime raw_stack::repack_moves_elements_to_ideal_offsets_and_reports_padding`
Expected: FAIL with "no method named `repack`".

- [ ] **Step 3: Implement `repack`**

Add to `impl RawStack` in `cel-runtime/src/raw_stack.rs`, after `drop_sized`:

```rust
    /// Repacks `sizes.len()` already-pushed values (currently at the absolute
    /// byte offsets in `src_offsets`) into one contiguous, self-contained
    /// region of `total_size` bytes starting at `dest_base`, placing element
    /// `i` at `dest_base + dest_offsets[i]`. Adjusts the tracked length to
    /// `dest_base + total_size` and returns whether leading padding was
    /// inserted between `ambient_start` and `dest_base`.
    ///
    /// - Precondition: `src_offsets`, `dest_offsets`, and `sizes` have equal
    ///   length; `dest_base >= ambient_start`; the source ranges are
    ///   currently valid, initialized bytes.
    ///
    /// - Complexity: O(n) in `sizes.len()`.
    ///
    /// # Safety
    /// The offsets and sizes must correctly describe the actual bytes in the
    /// buffer; no two destination ranges may overlap.
    pub unsafe fn repack(
        &mut self,
        ambient_start: usize,
        dest_base: usize,
        total_size: usize,
        src_offsets: &[usize],
        dest_offsets: &[usize],
        sizes: &[usize],
    ) -> bool {
        debug_assert!(dest_base >= ambient_start);
        debug_assert_eq!(src_offsets.len(), sizes.len());
        debug_assert_eq!(dest_offsets.len(), sizes.len());

        let target_len = dest_base + total_size;
        let current_len = self.buffer.len();
        let grown_len = current_len.max(target_len);
        unsafe {
            if grown_len > current_len {
                self.buffer.reserve(grown_len - current_len);
                self.buffer.set_len(grown_len);
            }
            let base_ptr = self.buffer.as_mut_ptr().cast::<u8>();
            for i in 0..sizes.len() {
                std::ptr::copy(
                    base_ptr.add(src_offsets[i]),
                    base_ptr.add(dest_base + dest_offsets[i]),
                    sizes[i],
                );
            }
            if dest_base > ambient_start {
                self.buffer[ambient_start].write(1);
                self.buffer[ambient_start + 1..dest_base].fill(MaybeUninit::new(0));
            }
            self.buffer.set_len(target_len);
        }
        dest_base > ambient_start
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p cel-runtime raw_stack::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/raw_stack.rs
git commit -m "feat(cel-runtime): add RawStack::repack for layout-independent tuples"
```

---

### Task 4: `StackInfo`/`AssociatedType` metadata + `RawDropper`/`DynTuple`

**Files:**
- Modify: `cel-runtime/src/dyn_segment.rs`

**Interfaces:**
- Produces: `pub type RawDropper = unsafe fn(*mut u8, &[AssociatedType])`, `pub struct DynTuple`, `AssociatedType { type_id, type_name, offset, size, align, dropper: RawDropper, associated }`, `StackInfo { type_id, type_name, padding, size, align, raw_dropper: RawDropper, associated }` (public fields: `type_id`, `type_name`, `size`, `align`, `associated`; `padding`/`raw_dropper` stay `pub(crate)`).
- Removes: the old `type Dropper = fn(&mut RawStack);` mechanism and `StackInfo.dropper`/per-closure droppers generated in `push_type`/`to_stack_info_list` — replaced by `raw_dropper` + `RawStack::drop_sized`.

This task is a mechanical widening: every existing behavior must stay observably identical (all current `dyn_segment.rs` tests continue passing unchanged), while adding the fields tuples need.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dyn_segment.rs`:

```rust
#[test]
fn stack_info_records_size_and_align() {
    let mut seg = DynSegment::new::<()>();
    seg.op0(|| 7u32);
    let infos = seg.peek_stack_infos(1);
    assert_eq!(infos[0].size, size_of::<u32>());
    assert_eq!(infos[0].align, align_of::<u32>());
    assert!(infos[0].associated.is_empty());
}

#[test]
fn associated_type_carries_offset_size_align_dropper() {
    // Exercises the new AssociatedType shape directly — no runtime behavior
    // yet, just the data shape this task adds.
    let a = AssociatedType {
        type_id: std::any::TypeId::of::<u32>(),
        type_name: std::borrow::Cow::Borrowed("u32"),
        offset: 4,
        size: 4,
        align: 4,
        dropper: |ptr, _associated| unsafe { std::ptr::drop_in_place(ptr.cast::<u32>()) },
        associated: Vec::new(),
    };
    assert_eq!(a.offset, 4);
    assert_eq!(a.size, 4);
    assert_eq!(a.align, 4);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime dyn_segment:: -- --exact stack_info_records_size_and_align associated_type_carries_offset_size_align_dropper`
Expected: FAIL — `size`/`align` fields don't exist yet on `StackInfo`, and `AssociatedType`'s struct literal doesn't have `offset`/`size`/`align`/`dropper` fields yet.

- [ ] **Step 3: Widen the types**

Replace the `AssociatedType` struct and the `Dropper`/`StackInfo` definitions in `cel-runtime/src/dyn_segment.rs`:

```rust
/// Drops a value in place, given a pointer to its bytes and (for tuple
/// values) its own element metadata for recursive drops.
///
/// # Safety
/// `ptr` must point to a valid, live, properly aligned value of the type this
/// dropper was generated for; `associated` must be that same value's own
/// element list (empty for non-tuple values).
pub type RawDropper = unsafe fn(*mut u8, &[AssociatedType]);

/// Recursive type node carrying a [`TypeId`], display name, byte layout, and
/// an in-place dropper — describes one element of a tuple (or, nested, one
/// element of a tuple element).
#[derive(Clone, Debug)]
pub struct AssociatedType {
    /// Runtime type id for this node.
    pub type_id: TypeId,
    /// Human-readable name for error reporting (borrowed when from `type_name::<T>()`).
    pub type_name: Cow<'static, str>,
    /// Byte offset from the start of the enclosing tuple.
    pub offset: usize,
    /// Size in bytes of this element's value.
    pub size: usize,
    /// Required alignment in bytes of this element's value.
    pub align: usize,
    /// In-place dropper for this element, callable at `base + offset`.
    pub dropper: RawDropper,
    /// Child types, for a nested tuple element.
    pub associated: Vec<AssociatedType>,
}

/// Marker type used as the `TypeId` for tuple aggregate stack entries.
///
/// A tuple's real type identity is the ordered `associated` list on its
/// [`StackInfo`], not this marker's `TypeId` — comparisons that need to
/// distinguish tuple shapes must inspect `associated`, not `type_id`.
#[derive(Debug)]
pub struct DynTuple;

/// Information about a type on the stack, including its cleanup function.
///
/// Holds metadata for a value pushed onto the stack: runtime type id, display
/// name for errors, padding, size/alignment, an in-place dropper, and an
/// optional list of associated element types (populated for tuples).
pub struct StackInfo {
    /// Runtime type id for this stack slot (e.g. for scope matching).
    pub type_id: TypeId,
    /// Human-readable type name for error reporting (borrowed when from `type_name::<T>()`).
    pub type_name: Cow<'static, str>,
    /// Whether padding was inserted before this value for alignment.
    pub(crate) padding: bool,
    /// Size in bytes of this stack slot's value.
    pub size: usize,
    /// Required alignment in bytes of this stack slot's value.
    pub align: usize,
    /// In-place dropper for this value, callable at its own start address.
    pub(crate) raw_dropper: RawDropper,
    /// Associated element types (populated for tuples; empty otherwise).
    pub associated: Vec<AssociatedType>,
}
```

Update `ToTypeIdList for CNil<()>` / `ToTypeIdList for CStackList<H, T>`:

```rust
impl ToTypeIdList for CNil<()> {
    fn to_stack_info_list() -> Vec<StackInfo> {
        Vec::new()
    }
}

impl<H: 'static, T: ToTypeIdList + 'static + CStackListHeadLimit> ToTypeIdList
    for CStackList<H, T>
{
    fn to_stack_info_list() -> Vec<StackInfo> {
        let mut list = T::to_stack_info_list();
        list.push(StackInfo {
            type_id: TypeId::of::<H>(),
            type_name: Cow::Borrowed(std::any::type_name::<H>()),
            padding: Self::HEAD_PADDED,
            size: size_of::<H>(),
            align: align_of::<H>(),
            raw_dropper: |ptr, _associated| unsafe { std::ptr::drop_in_place(ptr.cast::<H>()) },
            associated: Vec::new(),
        });
        list
    }
}
```

Update `DynSegment::push_type`:

```rust
    /// Push type to stack and register dropper.
    fn push_type<T>(&mut self)
    where
        T: 'static,
    {
        let aligned_index = align_index(align_of::<T>(), self.stack_index);
        let padded = aligned_index != self.stack_index;

        self.stack_ids.push(StackInfo {
            type_id: TypeId::of::<T>(),
            type_name: Cow::Borrowed(std::any::type_name::<T>()),
            padding: padded,
            size: size_of::<T>(),
            align: align_of::<T>(),
            raw_dropper: |ptr, _associated| unsafe { std::ptr::drop_in_place(ptr.cast::<T>()) },
            associated: Vec::new(),
        });
        self.stack_index = aligned_index + size_of::<T>();
    }
```

Update `capture_unwind`/`unwind_on_err`:

```rust
    /// Captures the current stack droppers for use when unwinding on error.
    ///
    /// - Complexity: O(n) in the current stack depth.
    fn capture_unwind(&self) -> Vec<(usize, bool, RawDropper, Vec<AssociatedType>)> {
        self.stack_ids
            .iter()
            .map(|info| (info.size, info.padding, info.raw_dropper, info.associated.clone()))
            .collect()
    }

    /// Runs the captured droppers in reverse order on error, then propagates the error.
    fn unwind_on_err<R>(
        unwind: &[(usize, bool, RawDropper, Vec<AssociatedType>)],
        stack: &mut RawStack,
        result: Result<R>,
    ) -> Result<R> {
        match result {
            Ok(r) => Ok(r),
            Err(e) => {
                for (size, padding, raw_dropper, associated) in unwind.iter().rev() {
                    unsafe {
                        stack.drop_sized(*size, *padding, |ptr| unsafe {
                            raw_dropper(ptr, associated)
                        });
                    }
                }
                Err(e)
            }
        }
    }
```

No other call sites need to change — `op0r`/`op1r`/`op2r` already just do `let unwind = self.capture_unwind();` and pass `&unwind` through, which still type-checks with the new return type.

- [ ] **Step 4: Run tests to verify everything passes**

Run: `cargo test -p cel-runtime`
Expected: PASS — every pre-existing test in `dyn_segment.rs` (drop-on-error, op1r/op2r unwind, etc.) plus the two new tests from Step 1.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dyn_segment.rs
git commit -m "refactor(cel-runtime): widen StackInfo/AssociatedType with size/align/raw_dropper"
```

---

### Task 5: Tuple construction and `.N` indexing (`DynSegment` API)

**Files:**
- Modify: `cel-runtime/src/dyn_segment.rs`

**Interfaces:**
- Consumes: `RawStack::{repack, copy_from, drop_at, truncate_to, push_raw, len}` (Tasks 1-3); `StackInfo`/`AssociatedType`/`RawDropper`/`DynTuple` (Task 4).
- Produces: `DynSegment::current_stack_offset(&self) -> usize`, `DynSegment::make_tuple(&mut self, n: usize, ambient_start: usize)`, `DynSegment::tuple_index(&mut self, index: usize)`, `DynSegment::peek_tuple_arity(&self) -> Option<usize>`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dyn_segment.rs`:

```rust
#[test]
fn make_tuple_then_index_each_element() {
    let mut seg = DynSegment::new::<()>();
    let ambient_start = seg.current_stack_offset();
    seg.op0(|| 10u32);
    seg.op0(|| "hello");
    seg.make_tuple(2, ambient_start);
    assert_eq!(seg.peek_tuple_arity(), Some(2));

    // Index element 1 first on a clone-free single segment isn't possible
    // (tuple_index consumes the tuple), so build two segments to check both.
    let mut seg0 = DynSegment::new::<()>();
    let ambient_start0 = seg0.current_stack_offset();
    seg0.op0(|| 10u32);
    seg0.op0(|| "hello");
    seg0.make_tuple(2, ambient_start0);
    seg0.tuple_index(0);
    assert_eq!(seg0.call0::<u32>().unwrap(), 10);

    seg.tuple_index(1);
    assert_eq!(seg.call0::<&'static str>().unwrap(), "hello");
}

#[test]
fn tuple_layout_is_independent_of_ambient_stack_depth() {
    // (u8, u32): with nothing ahead of it vs. with a u8 already on the stack,
    // internal padding between elements must be identical either way.
    let mut seg_a = DynSegment::new::<()>();
    let ambient_a = seg_a.current_stack_offset();
    seg_a.op0(|| 1u8);
    seg_a.op0(|| 2u32);
    seg_a.make_tuple(2, ambient_a);

    let mut seg_b = DynSegment::new::<()>();
    seg_b.op0(|| 99u8); // extra value ahead, shifts ambient depth
    let ambient_b = seg_b.current_stack_offset();
    seg_b.op0(|| 1u8);
    seg_b.op0(|| 2u32);
    seg_b.make_tuple(2, ambient_b);

    seg_a.tuple_index(1);
    seg_b.tuple_index(1);
    assert_eq!(seg_a.call0::<u32>().unwrap(), 2);
    seg_b.op2(|_extra: u8, x: u32| x).unwrap();
    assert_eq!(seg_b.call0::<u32>().unwrap(), 2);
}

#[test]
fn tuple_index_drops_every_other_element_exactly_once() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone)]
    struct DropCounter(Arc<AtomicUsize>);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let drop_count = Arc::new(AtomicUsize::new(0));
    let tracker = DropCounter(drop_count.clone());

    let mut seg = DynSegment::new::<()>();
    let ambient_start = seg.current_stack_offset();
    seg.op0(move || tracker.clone());
    seg.op0(|| 42u32);
    seg.make_tuple(2, ambient_start);
    seg.tuple_index(1); // keep the u32, drop the DropCounter

    assert_eq!(seg.call0::<u32>().unwrap(), 42);
    assert_eq!(drop_count.load(Ordering::SeqCst), 1);
}

#[test]
fn tuple_index_combined_with_another_op() {
    // Mirrors the spec's `5 + (0, 1).1` case: indexing must leave the stack in
    // a state a subsequent op can correctly consume.
    let mut seg = DynSegment::new::<()>();
    seg.op0(|| 5u32);
    let ambient_start = seg.current_stack_offset();
    seg.op0(|| 0u32);
    seg.op0(|| 1u32);
    seg.make_tuple(2, ambient_start);
    seg.tuple_index(1);
    seg.op2(|a: u32, b: u32| a + b).unwrap();
    assert_eq!(seg.call0::<u32>().unwrap(), 6);
}

#[test]
fn one_tuple_round_trips() {
    let mut seg = DynSegment::new::<()>();
    let ambient_start = seg.current_stack_offset();
    seg.op0(|| 99u32);
    seg.make_tuple(1, ambient_start);
    assert_eq!(seg.peek_tuple_arity(), Some(1));
    seg.tuple_index(0);
    assert_eq!(seg.call0::<u32>().unwrap(), 99);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime dyn_segment:: -- --exact make_tuple_then_index_each_element tuple_layout_is_independent_of_ambient_stack_depth tuple_index_drops_every_other_element_exactly_once tuple_index_combined_with_another_op one_tuple_round_trips`
Expected: FAIL with "no method named `current_stack_offset`" / `make_tuple` / `tuple_index` / `peek_tuple_arity`.

- [ ] **Step 3: Implement `current_stack_offset`, `peek_tuple_arity`, `make_tuple`, `tuple_index`**

Add `use anyhow::anyhow;` to the top-level imports if not already present (it is not — `dyn_segment.rs` currently imports `anyhow::Result` and `anyhow::ensure`; leave those, this task doesn't need `anyhow!` yet).

Add a free function near the top of `cel-runtime/src/dyn_segment.rs` (outside `impl DynSegment`), after the `AssociatedType`/`DynTuple` definitions from Task 4:

```rust
/// `RawDropper` for a tuple value: drops each element at `ptr + element.offset`
/// in reverse order, recursing into nested tuples via their own droppers.
///
/// # Safety
/// `ptr` must point to a live tuple value whose layout matches `associated`.
unsafe fn drop_tuple(ptr: *mut u8, associated: &[AssociatedType]) {
    for elem in associated.iter().rev() {
        unsafe { (elem.dropper)(ptr.add(elem.offset), &elem.associated) };
    }
}

/// Extracts element `index` from the tuple currently on top of `stack`,
/// dropping every other element, leaving just the extracted value on top.
///
/// - Complexity: O(n) in the tuple's arity.
///
/// # Safety
/// The top `tuple_size` bytes (plus `tuple_padding` if set) of `stack` must be
/// a live tuple value whose layout matches `associated`.
unsafe fn extract_tuple_element(
    stack: &mut RawStack,
    tuple_size: usize,
    tuple_padding: bool,
    associated: &[AssociatedType],
    index: usize,
) {
    let tuple_base = stack.len() - tuple_size;
    let target = &associated[index];

    let mut scratch = vec![0u8; target.size];
    unsafe {
        stack.copy_from(tuple_base + target.offset, target.size, scratch.as_mut_ptr());
    }

    for (i, elem) in associated.iter().enumerate().rev() {
        if i == index {
            continue;
        }
        let elem_associated = &elem.associated;
        unsafe {
            stack.drop_at(tuple_base + elem.offset, |ptr| unsafe {
                (elem.dropper)(ptr, elem_associated)
            });
        }
    }

    unsafe {
        stack.truncate_to(tuple_base, tuple_padding);
        stack.push_raw(target.align, target.size, scratch.as_ptr());
    }
}
```

Add these methods to `impl DynSegment` in `cel-runtime/src/dyn_segment.rs`, near `push_type`:

```rust
    /// Returns the current parse-time stack byte offset.
    ///
    /// Snapshot this before parsing a tuple's first element and pass it to
    /// [`make_tuple`](Self::make_tuple).
    #[must_use]
    pub fn current_stack_offset(&self) -> usize {
        self.stack_index
    }

    /// Returns the arity of the tuple on top of the stack, or `None` if the
    /// top value isn't a tuple.
    #[must_use]
    pub fn peek_tuple_arity(&self) -> Option<usize> {
        let info = self.stack_ids.last()?;
        (info.type_id == TypeId::of::<DynTuple>()).then(|| info.associated.len())
    }

    /// Collapses the top `n` stack values (pushed starting at byte offset
    /// `ambient_start`, e.g. via [`current_stack_offset`](Self::current_stack_offset)
    /// captured before parsing the first element) into one tuple value.
    ///
    /// The tuple's internal layout (offsets between elements) depends only on
    /// the elements' own types — never on `ambient_start` — matching the
    /// layout a `#[repr(C)]` struct with these fields, in this order, would
    /// use.
    ///
    /// - Precondition: at least `n` values are on the stack, pushed
    ///   contiguously starting at `ambient_start` with no other values
    ///   interleaved.
    ///
    /// - Complexity: O(n).
    pub fn make_tuple(&mut self, n: usize, ambient_start: usize) {
        debug_assert!(n <= self.stack_ids.len());
        let start = self.stack_ids.len() - n;
        let elems: Vec<StackInfo> = self.stack_ids.drain(start..).collect();

        let mut ambient_offset = ambient_start;
        let mut ideal_offset = 0usize;
        let mut tuple_align = 1usize;
        let mut src_offsets = Vec::with_capacity(n);
        let mut associated = Vec::with_capacity(n);
        for elem in &elems {
            ambient_offset = align_index(elem.align, ambient_offset);
            ideal_offset = align_index(elem.align, ideal_offset);
            tuple_align = tuple_align.max(elem.align);

            src_offsets.push(ambient_offset);
            associated.push(AssociatedType {
                type_id: elem.type_id,
                type_name: elem.type_name.clone(),
                offset: ideal_offset,
                size: elem.size,
                align: elem.align,
                dropper: elem.raw_dropper,
                associated: elem.associated.clone(),
            });

            ambient_offset += elem.size;
            ideal_offset += elem.size;
        }
        let total_size = align_index(tuple_align, ideal_offset);
        let dest_base = align_index(tuple_align, ambient_start);

        let dest_offsets: Vec<usize> = associated.iter().map(|a| a.offset).collect();
        let sizes: Vec<usize> = elems.iter().map(|e| e.size).collect();

        self.segment.raw0_(move |stack| {
            unsafe {
                stack.repack(ambient_start, dest_base, total_size, &src_offsets, &dest_offsets, &sizes);
            }
            Ok(())
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
        self.stack_index = dest_base + total_size;
    }

    /// Extracts element `index` from the tuple on top of the stack, replacing
    /// the whole tuple with just that element's value.
    ///
    /// - Precondition: the top-of-stack value is a tuple with at least
    ///   `index + 1` elements.
    ///
    /// - Complexity: O(n) in the tuple's arity.
    pub fn tuple_index(&mut self, index: usize) {
        let info = self
            .stack_ids
            .pop()
            .expect("tuple_index requires a value on the stack");
        debug_assert_eq!(
            info.type_id,
            TypeId::of::<DynTuple>(),
            "tuple_index requires a tuple on top of the stack"
        );
        debug_assert!(index < info.associated.len(), "tuple_index out of range");

        let tuple_start = self.stack_index - info.size;
        let target = info.associated[index].clone();
        let associated = info.associated.clone();
        let tuple_padding = info.padding;
        let tuple_size = info.size;

        self.segment.raw0_(move |stack| {
            unsafe {
                extract_tuple_element(stack, tuple_size, tuple_padding, &associated, index);
            }
            Ok(())
        });

        let target_start = align_index(target.align, tuple_start);
        self.stack_ids.push(StackInfo {
            type_id: target.type_id,
            type_name: target.type_name,
            padding: target_start != tuple_start,
            size: target.size,
            align: target.align,
            raw_dropper: target.dropper,
            associated: target.associated,
        });
        self.stack_index = target_start + target.size;
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime dyn_segment::`
Expected: PASS — all 5 new tests plus every pre-existing test in the module.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dyn_segment.rs
git commit -m "feat(cel-runtime): add tuple construction (make_tuple) and indexing (tuple_index)"
```

---

### Task 6: Parser grammar — tuple literals

**Files:**
- Modify: `cel-parser/src/lib.rs`

**Interfaces:**
- Consumes: `DynSegment::{current_stack_offset, make_tuple}` (Task 5).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-parser/src/lib.rs` (find the existing `#[cfg(test)] mod tests` block and add alongside the other parser tests):

```rust
#[test]
fn unit_still_parses_as_unit() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut seg = parser.parse_str("()").unwrap();
    seg.call0::<()>().unwrap();
}

#[test]
fn single_paren_expression_is_grouping_not_tuple() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut seg = parser.parse_str("(1i32 + 2i32)").unwrap();
    assert_eq!(seg.call0::<i32>().unwrap(), 3);
}

#[test]
fn one_tuple_requires_trailing_comma() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut seg = parser.parse_str("(1i32,)").unwrap();
    assert_eq!(seg.peek_tuple_arity(), Some(1));
    seg.tuple_index(0);
    assert_eq!(seg.call0::<i32>().unwrap(), 1);
}

#[test]
fn two_element_tuple_no_trailing_comma() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut seg = parser.parse_str(r#"("Hello", 42i32)"#).unwrap();
    assert_eq!(seg.peek_tuple_arity(), Some(2));
}

#[test]
fn trailing_comma_rejected_for_arity_two() {
    let mut parser = CELParser::new(OpLookup::new());
    let result = parser.parse_str("(1i32, 2i32,)");
    assert!(result.is_err(), "trailing comma is only valid for 1-tuples");
}

#[test]
fn missing_comma_between_elements_is_an_error() {
    let mut parser = CELParser::new(OpLookup::new());
    let result = parser.parse_str("(1i32 2i32)");
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-parser one_tuple_requires_trailing_comma two_element_tuple_no_trailing_comma trailing_comma_rejected_for_arity_two missing_comma_between_elements_is_an_error`
Expected: FAIL — `(1i32,)` and `("Hello", 42i32)` currently error ("expected closing parenthesis"), and `peek_tuple_arity`/`tuple_index` calls won't compile against the current `DynSegment` re-export until Task 5 lands (already done) — these tests specifically fail on parse behavior, not missing methods.

- [ ] **Step 3: Implement the grammar**

Replace the `Parenthesis` arm of `is_primary_expression` in `cel-parser/src/lib.rs`:

```rust
            Some(Token::OpenDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            }) => {
                self.advance();
                // Unit expression: ()
                if matches!(
                    self.peek_token(),
                    Some(Token::CloseDelim {
                        delimiter: Delimiter::Parenthesis,
                        ..
                    })
                ) {
                    self.advance();
                    self.context.just(());
                    return Ok(true);
                }
                let ambient_start = self.context.current_stack_offset();
                if !self.is_or_expression()? {
                    return Err(self.error_at("expected expression"));
                }
                if matches!(
                    self.peek_token(),
                    Some(Token::CloseDelim {
                        delimiter: Delimiter::Parenthesis,
                        ..
                    })
                ) {
                    // Grouping: exactly one expression, no comma.
                    self.advance();
                    return Ok(true);
                }
                if !self.is_punctuation(",") {
                    return Err(self.error_at("expected ',' or closing parenthesis"));
                }
                let mut count = 1;
                if matches!(
                    self.peek_token(),
                    Some(Token::CloseDelim {
                        delimiter: Delimiter::Parenthesis,
                        ..
                    })
                ) {
                    // Single element + trailing comma: 1-tuple.
                    self.advance();
                    self.context.make_tuple(count, ambient_start);
                    return Ok(true);
                }
                loop {
                    if !self.is_or_expression()? {
                        return Err(self.error_at("expected expression after ','"));
                    }
                    count += 1;
                    if matches!(
                        self.peek_token(),
                        Some(Token::CloseDelim {
                            delimiter: Delimiter::Parenthesis,
                            ..
                        })
                    ) {
                        self.advance();
                        break;
                    }
                    if !self.is_punctuation(",") {
                        return Err(self.error_at("expected ',' or closing parenthesis"));
                    }
                }
                self.context.make_tuple(count, ambient_start);
                Ok(true)
            }
```

Update the grammar doc comment at the top of `cel-parser/src/lib.rs` (in the module-level `//!` block):

```
//! primary_expression = literal | identifier | tuple_or_group | if_expression.
//! tuple_or_group = "(" [ or_expression ["," [ or_expression { "," or_expression } ]] ] ")".
```

(Replace the existing `primary_expression = literal | identifier | "(" or_expression ")" | if_expression.` line and add the new `tuple_or_group` line right after it.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-parser`
Expected: PASS — all 6 new tests plus the full existing `cel-parser` suite (unchanged grouping/if/call behavior).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-parser/src/lib.rs
git commit -m "feat(cel-parser): parse tuple literals"
```

---

### Task 7: Parser grammar — `.N` indexing

**Files:**
- Modify: `cel-parser/src/lib.rs`

**Interfaces:**
- Consumes: `DynSegment::{peek_tuple_arity, tuple_index}` (Task 5); tuple literal parsing (Task 6).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-parser/src/lib.rs`:

```rust
#[test]
fn index_first_element_of_tuple() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut seg = parser.parse_str("(10i32, 20i32).0").unwrap();
    assert_eq!(seg.call0::<i32>().unwrap(), 10);
}

#[test]
fn index_second_element_of_tuple() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut seg = parser.parse_str("(10i32, 20i32).1").unwrap();
    assert_eq!(seg.call0::<i32>().unwrap(), 20);
}

#[test]
fn indexing_combined_with_addition() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut seg = parser.parse_str("5i32 + (0i32, 1i32).1").unwrap();
    assert_eq!(seg.call0::<i32>().unwrap(), 6);
}

#[test]
fn indexing_combined_with_addition_on_the_right() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut seg = parser.parse_str("(0i32, 1i32).1 + 5i32").unwrap();
    assert_eq!(seg.call0::<i32>().unwrap(), 6);
}

#[test]
fn out_of_range_index_is_a_parse_error() {
    let mut parser = CELParser::new(OpLookup::new());
    let result = parser.parse_str("(1i32, 2i32).5");
    assert!(result.is_err());
}

#[test]
fn indexing_a_non_tuple_is_a_parse_error() {
    let mut parser = CELParser::new(OpLookup::new());
    let result = parser.parse_str("1i32.0");
    assert!(result.is_err());
}

#[test]
fn suffixed_index_is_a_parse_error() {
    let mut parser = CELParser::new(OpLookup::new());
    let result = parser.parse_str("(1i32, 2i32).0i32");
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-parser index_first_element_of_tuple index_second_element_of_tuple indexing_combined_with_addition indexing_combined_with_addition_on_the_right out_of_range_index_is_a_parse_error indexing_a_non_tuple_is_a_parse_error suffixed_index_is_a_parse_error`
Expected: FAIL — `.N` isn't recognized as postfix syntax yet, so these parse as `1i32` followed by a stray `.0`/etc. and error or misparse.

- [ ] **Step 3: Implement `.N` postfix indexing**

Replace `is_postfix_expression` in `cel-parser/src/lib.rs`:

```rust
    /// `postfix_expression = primary_expression { "(" parameter_list ")" | "." unsuffixed_integer }.`
    fn is_postfix_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if !self.is_primary_expression()? {
            return Ok(false);
        }
        loop {
            if matches!(
                self.peek_token(),
                Some(Token::OpenDelim {
                    delimiter: Delimiter::Parenthesis,
                    ..
                })
            ) {
                self.advance(); // consume "("
                let arg_count = self.parameter_list()?;
                match self.peek_token() {
                    Some(Token::CloseDelim {
                        delimiter: Delimiter::Parenthesis,
                        ..
                    }) => {
                        self.advance(); // consume ")"
                    }
                    _ => return Err(self.error_at("expected closing parenthesis")),
                }
                // Stack order is [callee, arg1, arg2, ...]; lookup peeks top (arg_count + 1) entries.
                self.op_lookup.lookup(
                    "()",
                    &mut self.context,
                    arg_count + 1,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            } else if self.is_punctuation(".") {
                let index = match self.peek_token() {
                    Some(Token::Literal(CelLiteral::Int(integer))) => {
                        let integer = integer.clone();
                        if !integer.suffix().is_empty() {
                            return Err(self.error_at("tuple index must be an unsuffixed integer"));
                        }
                        self.advance();
                        integer.base10_parse::<usize>().map_err(|e| {
                            self.error_at(&format!("invalid tuple index `{integer}`: {e}"))
                        })?
                    }
                    _ => return Err(self.error_at("expected integer after '.'")),
                };
                let arity = self
                    .context
                    .peek_tuple_arity()
                    .ok_or_else(|| self.error_at("'.N' requires a tuple"))?;
                if index >= arity {
                    return Err(self.error_at(&format!(
                        "tuple index `{index}` out of range for tuple of arity {arity}"
                    )));
                }
                self.context.tuple_index(index);
            } else {
                break;
            }
        }
        Ok(true)
    }
```

Update the grammar doc comment at the top of `cel-parser/src/lib.rs`:

```
//! postfix_expression = primary_expression { "(" parameter_list ")" | "." unsuffixed_integer }.
```

(Replace the existing `postfix_expression = primary_expression { "(" parameter_list ")" }.` line.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-parser`
Expected: PASS — all 7 new tests plus the full existing suite.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-parser/src/lib.rs
git commit -m "feat(cel-parser): parse .N tuple indexing"
```

---

### Task 8: Generalized tuple-shaped op registration

**Files:**
- Modify: `cel-parser/src/op_table.rs`

**Interfaces:**
- Consumes: `StackInfo.associated` (Task 4/5).
- Produces: `pub struct TupleOpSignature { name, shape, tuple_operand_index, operand_type_ids, op_fn }`, `OpLookup::register_tuple_op(&mut self, signature: TupleOpSignature)`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `cel-parser/src/op_table.rs`:

```rust
#[test]
fn tuple_shaped_signature_matches_and_dispatches() -> Result<()> {
    let mut lookup = OpLookup::new();
    lookup.register_tuple_op(TupleOpSignature {
        name: "greet".to_string(),
        shape: vec![TypeId::of::<String>(), TypeId::of::<i32>()],
        tuple_operand_index: 0,
        operand_type_ids: vec![],
        op_fn: |seg, _span| {
            seg.tuple_index(1);
            seg.op1(|_ignored: i32| true)
        },
    });

    let mut segment = DynSegment::new::<()>();
    let ambient_start = segment.current_stack_offset();
    segment.op0(|| "hi".to_string());
    segment.op0(|| 7i32);
    segment.make_tuple(2, ambient_start);

    lookup.lookup("greet", &mut segment, 1, Span::call_site(), Span::call_site())?;
    assert!(segment.call0::<bool>()?);
    Ok(())
}

#[test]
fn tuple_shaped_signature_does_not_match_wrong_shape() {
    let mut lookup = OpLookup::new();
    lookup.register_tuple_op(TupleOpSignature {
        name: "greet".to_string(),
        shape: vec![TypeId::of::<String>(), TypeId::of::<i32>()],
        tuple_operand_index: 0,
        operand_type_ids: vec![],
        op_fn: |seg, _span| {
            seg.tuple_index(1);
            seg.op1(|_ignored: i32| true)
        },
    });

    let mut segment = DynSegment::new::<()>();
    let ambient_start = segment.current_stack_offset();
    segment.op0(|| 1i32);
    segment.op0(|| 2i32);
    segment.make_tuple(2, ambient_start);

    let result = lookup.lookup("greet", &mut segment, 1, Span::call_site(), Span::call_site());
    assert!(result.is_err(), "shape (i32, i32) should not match (String, i32)");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-parser tuple_shaped_signature_matches_and_dispatches tuple_shaped_signature_does_not_match_wrong_shape`
Expected: FAIL with "no method named `register_tuple_op`" / `TupleOpSignature` not found.

- [ ] **Step 3: Implement `TupleOpSignature` and matching**

Add near the top of `cel-parser/src/op_table.rs`, after the `OpFn` type alias:

```rust
/// A signature for an operator/function whose selected operand is a tuple.
///
/// Matches when the operand at `tuple_operand_index` (0-based, in the same
/// stack order [`DynSegment::peek_stack_infos`] returns) is a tuple whose
/// element `TypeId`s equal `shape`, in order, and every other peeked operand's
/// flat `TypeId` equals the corresponding entry in `operand_type_ids` (the
/// entry at `tuple_operand_index` in `operand_type_ids` is ignored).
pub struct TupleOpSignature {
    /// Operator/function name this signature is registered under.
    pub name: String,
    /// Expected element `TypeId`s, in order, for the tuple-shaped operand.
    pub shape: Vec<TypeId>,
    /// Which peeked operand position must be the tuple.
    pub tuple_operand_index: usize,
    /// Flat `TypeId`s expected for the non-tuple operands, in stack order
    /// (the `tuple_operand_index` entry is ignored).
    pub operand_type_ids: Vec<TypeId>,
    /// Function that pushes the operation onto the segment.
    pub op_fn: OpFn,
}
```

Add `tuple_signatures: Vec<TupleOpSignature>` to `OpLookup`, its constructor, and a matching method + call site:

```rust
pub struct OpLookup {
    scopes: Vec<ScopeFn>,
    builtin_scope: BuiltinScope,
    tuple_signatures: Vec<TupleOpSignature>,
}

impl OpLookup {
    pub fn new() -> Self {
        OpLookup {
            scopes: Vec::new(),
            builtin_scope: BuiltinScope,
            tuple_signatures: Vec::new(),
        }
    }

    /// Registers a tuple-shaped operator signature, matched by element
    /// `TypeId` sequence the same way built-in operators are matched by flat
    /// `TypeId`.
    pub fn register_tuple_op(&mut self, signature: TupleOpSignature) {
        self.tuple_signatures.push(signature);
    }

    /// Attempts to find and apply a registered tuple-shaped signature.
    ///
    /// Returns `Ok(true)` if found and applied, `Ok(false)` if not found.
    ///
    /// - Complexity: O(s) where s is the number of registered tuple signatures.
    fn lookup_tuple_signature(
        &self,
        name: &str,
        segment: &mut DynSegment,
        num_operands: usize,
        span: SourceSpan,
    ) -> Result<bool> {
        let stack_infos = segment.peek_stack_infos(num_operands);
        for sig in &self.tuple_signatures {
            if sig.name != name || sig.tuple_operand_index >= stack_infos.len() {
                continue;
            }
            let tuple_info = &stack_infos[sig.tuple_operand_index];
            let shape_matches = tuple_info.associated.len() == sig.shape.len()
                && tuple_info
                    .associated
                    .iter()
                    .zip(&sig.shape)
                    .all(|(a, t)| a.type_id == *t);
            if !shape_matches {
                continue;
            }
            let others_match = stack_infos.iter().enumerate().all(|(i, info)| {
                i == sig.tuple_operand_index || info.type_id == sig.operand_type_ids[i]
            });
            if others_match {
                (sig.op_fn)(segment, span)?;
                return Ok(true);
            }
        }
        Ok(false)
    }
```

In `OpLookup::lookup`, insert the new check between the custom-scope loop and the built-in scope lookup:

```rust
        for scope in self.scopes.iter().rev() {
            match scope(name, segment, num_operands, source_span) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(e) => return Err(crate::ParseError::new_range(e.to_string(), start, end)),
            }
        }

        match self.lookup_tuple_signature(name, segment, num_operands, source_span) {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(e) => {
                return Err(crate::ParseError::new_range(
                    format!("operation error: {}", e),
                    start,
                    end,
                ));
            }
        }

        match self
            .builtin_scope
            .lookup(name, segment, num_operands, source_span)
        {
            // ... unchanged from here
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-parser op_table::`
Expected: PASS — both new tests plus the full existing `op_table.rs` suite (built-in operator dispatch is untouched, since `tuple_signatures` is empty by default).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-parser/src/op_table.rs
git commit -m "feat(cel-parser): generalize op dispatch for tuple-shaped operands"
```

---

### Task 9: `pop_tuple_as`/`push_tuple` — sound `CStackList` bridge

**Files:**
- Modify: `cel-runtime/src/dyn_segment.rs`

**Interfaces:**
- Consumes: `ToTypeIdList` (existing), `List` (existing), `StackInfo`/`AssociatedType` (Task 4).
- Produces: `DynSegment::pop_tuple_as::<L: List + ToTypeIdList + 'static>(&mut self) -> anyhow::Result<()>`, `DynSegment::push_tuple::<L: List + ToTypeIdList + 'static>(&mut self)`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cel-runtime/src/dyn_segment.rs`:

```rust
#[test]
fn push_tuple_then_pop_tuple_as_round_trips() -> Result<(), anyhow::Error> {
    let mut seg = DynSegment::new::<()>();
    // Build a concrete CStackList<u32, CStackList<&str, CNil<()>>> by pushing
    // fields in declaration order (NOT via into_c_stack_list, which reverses
    // order — see pop_tuple_as's doc comment). `CNil`'s inner field is
    // private, so build the empty base via the public `IntoCStackList`
    // conversion on `()` rather than the tuple-struct constructor.
    seg.op0(|| CStackList(().into_c_stack_list(), 7u32).push("hi"));
    seg.push_tuple::<CStackList<&str, CStackList<u32, CNil<()>>>>();
    assert_eq!(seg.peek_tuple_arity(), Some(2));

    seg.pop_tuple_as::<CStackList<&str, CStackList<u32, CNil<()>>>>()?;
    let result = seg.call0::<CStackList<&str, CStackList<u32, CNil<()>>>>()?;
    assert_eq!(result.head(), &"hi");
    assert_eq!(result.tail().head(), &7u32);
    Ok(())
}

#[test]
fn pop_tuple_as_rejects_shape_mismatch() {
    let mut seg = DynSegment::new::<()>();
    let ambient_start = seg.current_stack_offset();
    seg.op0(|| 1u32);
    seg.op0(|| 2u32);
    seg.make_tuple(2, ambient_start);

    let result = seg.pop_tuple_as::<CStackList<&str, CStackList<u32, CNil<()>>>>();
    assert!(result.is_err(), "(u32, u32) should not match (u32, &str)");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-runtime push_tuple_then_pop_tuple_as_round_trips pop_tuple_as_rejects_shape_mismatch`
Expected: FAIL with "no method named `push_tuple`" / `pop_tuple_as`.

- [ ] **Step 3: Implement `pop_tuple_as` and `push_tuple`**

Add `use anyhow::anyhow;` to the imports at the top of `cel-runtime/src/dyn_segment.rs` (alongside the existing `use anyhow::Result;` / `use anyhow::ensure;`).

Add these methods to `impl DynSegment`:

```rust
    /// Reinterprets the tuple on top of the stack as a concrete `L`
    /// (typically a `CStackList<...>` chain), replacing its `StackInfo` with
    /// `L`'s. No bytes move: both sides already use the same
    /// natural-alignment, declaration-order layout, so this is a relabel, not
    /// a copy.
    ///
    /// - Precondition: `L` was assembled via sequential `.push()` calls in
    ///   the same field order as the tuple (not via `into_c_stack_list()` on
    ///   a same-order plain tuple, which reverses element order).
    ///
    /// # Errors
    /// Returns an error if the top of stack isn't a tuple, or its element
    /// `TypeId`s (in order) don't match `L`'s.
    pub fn pop_tuple_as<L: List + ToTypeIdList + 'static>(&mut self) -> Result<()> {
        let info = self
            .stack_ids
            .last()
            .ok_or_else(|| anyhow!("pop_tuple_as: stack is empty"))?;
        ensure!(
            info.type_id == TypeId::of::<DynTuple>(),
            "pop_tuple_as: top of stack is not a tuple"
        );
        let expected: Vec<TypeId> = L::to_stack_info_list().iter().map(|s| s.type_id).collect();
        let actual: Vec<TypeId> = info.associated.iter().map(|a| a.type_id).collect();
        ensure!(
            expected == actual,
            "pop_tuple_as: tuple element types do not match `{}`",
            std::any::type_name::<L>()
        );
        debug_assert_eq!(info.size, size_of::<L>());
        debug_assert_eq!(info.align, align_of::<L>());

        let info = self.stack_ids.last_mut().expect("checked above");
        info.type_id = TypeId::of::<L>();
        info.type_name = Cow::Borrowed(std::any::type_name::<L>());
        info.raw_dropper = |ptr, _associated| unsafe { std::ptr::drop_in_place(ptr.cast::<L>()) };
        info.associated = Vec::new();
        Ok(())
    }

    /// Relabels the concrete `L` value on top of the stack as a tuple,
    /// exposing its elements for `.N` indexing and tuple-shaped op matching.
    /// No bytes move — see [`pop_tuple_as`](Self::pop_tuple_as) for why this
    /// is sound.
    ///
    /// - Precondition: the top of the stack currently holds a value of type
    ///   `L`, assembled via sequential `.push()` calls (not
    ///   `into_c_stack_list()` on a same-order plain tuple).
    pub fn push_tuple<L: List + ToTypeIdList + 'static>(&mut self) {
        let info = self
            .stack_ids
            .last_mut()
            .expect("push_tuple requires a value on the stack");
        debug_assert_eq!(
            info.type_id,
            TypeId::of::<L>(),
            "push_tuple: top of stack is not the expected type"
        );
        let element_infos = L::to_stack_info_list();
        let mut offset = 0usize;
        let associated = element_infos
            .iter()
            .map(|elem_info| {
                offset = align_index(elem_info.align, offset);
                let a = AssociatedType {
                    type_id: elem_info.type_id,
                    type_name: elem_info.type_name.clone(),
                    offset,
                    size: elem_info.size,
                    align: elem_info.align,
                    dropper: elem_info.raw_dropper,
                    associated: elem_info.associated.clone(),
                };
                offset += elem_info.size;
                a
            })
            .collect();
        info.type_id = TypeId::of::<DynTuple>();
        info.type_name = Cow::Borrowed(std::any::type_name::<DynTuple>());
        info.raw_dropper = drop_tuple;
        info.associated = associated;
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-runtime`
Expected: PASS — both new tests plus the entire `cel-runtime` suite.

- [ ] **Step 5: Run full workspace verification**

Run:
```bash
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin -- -D warnings
cargo clippy -p begin --no-default-features -- -D warnings
```
Expected: all PASS with zero warnings.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add cel-runtime/src/dyn_segment.rs
git commit -m "feat(cel-runtime): add pop_tuple_as/push_tuple CStackList bridge"
```

---

## Self-Review Notes

- **Spec coverage:** Grammar (Tasks 6-7), runtime representation / `AssociatedType` (Task 4), construction (Task 5/`make_tuple`), indexing incl. the corrected pop-and-repush algorithm (Task 5/`tuple_index`), `push_raw`/`repack` primitives (Tasks 1-3), interop registration (Task 8), `CStackList` bridge (Task 9), and the combined-indexing-with-another-op test the spec calls out explicitly (Task 5's `tuple_index_combined_with_another_op` and Task 7's `indexing_combined_with_addition*`) are all covered.
- **Deferred per spec's "Out of Scope":** arrays/vecs, first-class functions, method-call syntax, the compiled/macro backend, and reclaiming dead space via compaction (Task 5 pops-and-repushes the whole tuple instead) are intentionally not tasks here.
- **Type consistency:** `make_tuple(n, ambient_start)` / `tuple_index(index)` / `current_stack_offset()` / `peek_tuple_arity()` signatures introduced in Task 5 are used identically (same names, same parameter order) in Tasks 6-9. `RawDropper`/`AssociatedType`/`StackInfo` field names introduced in Task 4 are used consistently through Tasks 5 and 9.
