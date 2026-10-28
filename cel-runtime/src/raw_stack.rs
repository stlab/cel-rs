use crate::memory::align_index;
use crate::raw_vec::RawVec;
use crate::value_type::ValueType;
use std::mem::MaybeUninit;
use std::mem::size_of;

/// A simple raw stack that stores values as raw bytes. Each value is naturally aligned given the
/// base alignment of the stack, which is the maximum alignment of any value stored in the stack.
/// Live values are owned until popped, explicitly released, or destroyed in reverse push order.
#[derive(Debug)]
/// # Thread confinement
///
/// The erased owners may contain non-`Send` values, so the stack remains on its
/// creating thread.
///
/// ```compile_fail
/// let mut stack = cel_runtime::RawStack::with_base_alignment(4096);
/// stack.push(std::rc::Rc::new(()));
/// std::thread::spawn(move || drop(stack));
/// ```
pub struct RawStack {
    /// Owns the aligned bytes occupied by live stack values.
    buffer: RawVec,
    /// Destroys one initialized logical slot per entry, in reverse order.
    owners: Vec<fn(&mut RawStack)>,
    /// Retains layout and destruction metadata for erased raw slots.
    raw_owners: Vec<RawOwner>,
    /// Prevents erased non-`Send`/non-`Sync` owners from crossing thread boundaries.
    thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

/// Records layout and optional erased destruction only for a raw-byte slot.
#[derive(Debug)]
struct RawOwner {
    /// Identifies this raw slot's logical position among all owners.
    depth: usize,
    /// Marks the byte prefix before this slot's alignment padding.
    before: usize,
    /// Records the slot's required alignment.
    align: usize,
    /// Records the initialized value's byte size.
    size: usize,
    /// Supplies recursive destruction when the raw slot owns a canonical value.
    ty: Option<ValueType>,
}

/// Restores a stack checkpoint unless explicitly disarmed.
struct UnwindGuard<'a> {
    /// Exclusively borrows the stack whose suffix this guard owns.
    stack: &'a mut RawStack,
    /// Marks the byte prefix preserved by cleanup.
    byte_origin: usize,
    /// Marks the logical owner prefix preserved by cleanup.
    depth: usize,
    /// Distinguishes rollback cleanup from a committed successful call.
    armed: bool,
}

impl Drop for UnwindGuard<'_> {
    /// Resumes suffix cleanup, including while another destructor unwinds.
    fn drop(&mut self) {
        if self.armed {
            self.stack.unwind_to(self.byte_origin, self.depth);
        }
    }
}

impl Drop for RawStack {
    /// Destroys every live owner in reverse order.
    ///
    /// - Complexity: O(n) plus destructor costs for n live slots.
    fn drop(&mut self) {
        self.unwind_to(0, 0);
    }
}

impl RawStack {
    /// Creates a new `RawStack` with base alignment.
    ///
    /// - Precondition: `base_alignment` is a power of two.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::raw_stack::RawStack;
    /// let stack = RawStack::with_base_alignment(align_of::<u32>());
    /// ```
    #[must_use]
    pub fn with_base_alignment(base_alignment: usize) -> Self {
        RawStack {
            buffer: RawVec::with_base_alignment(base_alignment),
            owners: Vec::new(),
            raw_owners: Vec::new(),
            thread_bound: std::marker::PhantomData,
        }
    }

    /// Returns the logical slot count, including zero-sized values.
    pub(crate) fn depth(&self) -> usize {
        self.owners.len()
    }

    /// Destroys the top owner after removing its ownership and preceding padding.
    ///
    /// - Precondition: The stack has at least one logical slot.
    /// - Postcondition: The preceding prefix retains its bytes and ownership,
    ///   including when the top value is zero-sized or its destructor panics.
    /// - Complexity: O(p) for p padding bytes, plus destructor costs.
    pub(crate) fn drop_top(&mut self) {
        debug_assert!(self.depth() > 0);
        let destroy = *self.owners.last().expect("top owner exists");
        destroy(self);
    }

    /// Removes the latest ownership entries without destroying or changing their bytes.
    ///
    /// - Precondition: `count` does not exceed the logical depth.
    /// - Complexity: O(count), excluding descriptor destruction.
    pub(crate) fn release_top(&mut self, count: usize) {
        debug_assert!(count <= self.depth());
        let depth = self.depth() - count;
        while self
            .raw_owners
            .last()
            .is_some_and(|owner| owner.depth > depth)
        {
            self.raw_owners.pop();
        }
        self.owners.truncate(depth);
    }

    /// Installs erased destruction for the latest raw-byte slot.
    ///
    /// - Precondition: The top slot is raw, has no erased owner yet, and contains a
    ///   valid value with the shape and layout described by `ty`.
    pub(crate) fn own_top(&mut self, ty: ValueType) {
        let depth = self.depth();
        let owner = self.raw_owners.last_mut().expect("top slot must be raw");
        debug_assert_eq!(owner.depth, depth);
        debug_assert!(owner.ty.is_none());
        debug_assert_eq!(owner.size, ty.size());
        debug_assert_eq!(owner.align, ty.align());
        owner.ty = Some(ty);
    }

    /// Destroys the suffix after `depth` and restores its preceding byte origin.
    ///
    /// Remaining destruction resumes if one destructor panics.
    ///
    /// - Precondition: `(byte_origin, depth)` identifies a live prefix checkpoint.
    /// - Complexity: O(n) plus destructor costs for n removed slots and padding scans.
    pub(crate) fn unwind_to(&mut self, byte_origin: usize, depth: usize) {
        debug_assert!(depth <= self.depth());
        debug_assert!(byte_origin <= self.len());
        let mut guard = UnwindGuard {
            stack: self,
            byte_origin,
            depth,
            armed: true,
        };
        while guard.stack.depth() > depth {
            guard.stack.drop_top();
        }
        guard.stack.buffer.truncate(byte_origin);
        guard.armed = false;
    }

    /// Runs a callback, restoring a live prefix checkpoint on error or panic.
    ///
    /// - Precondition: `(byte_origin, depth)` identifies a live prefix that the
    ///   callback preserves.
    /// - Postcondition: Success retains all callback changes.
    /// - Complexity: Callback costs, plus suffix destruction on failure.
    ///
    /// # Errors
    ///
    /// Returns the callback's error after destroying its suffix.
    pub(crate) fn with_unwind<R>(
        &mut self,
        byte_origin: usize,
        depth: usize,
        callback: impl FnOnce(&mut RawStack) -> anyhow::Result<R>,
    ) -> anyhow::Result<R> {
        debug_assert!(depth <= self.depth());
        debug_assert!(byte_origin <= self.len());
        let mut guard = UnwindGuard {
            stack: self,
            byte_origin,
            depth,
            armed: true,
        };
        let result = callback(guard.stack);
        if result.is_ok() {
            guard.armed = false;
        }
        result
    }

    /// Pops and then destroys a typed owner with its statically known padding.
    fn destroy_typed<T, const PADDING: bool>(&mut self) {
        drop(unsafe { self.pop::<T>(PADDING) });
    }

    /// Removes raw ownership and bytes before invoking any erased destructor.
    fn destroy_raw(&mut self) {
        let owner = self.raw_owners.pop().expect("raw owner exists");
        debug_assert_eq!(owner.depth, self.depth());
        self.owners.pop();
        let offset = align_index(owner.align, owner.before);
        let pointer = unsafe { self.buffer.as_mut_ptr().add(offset).cast::<u8>() };
        self.buffer.truncate(owner.before);
        if let Some(ty) = owner.ty {
            unsafe { crate::owned_value::drop_raw(&ty, pointer) };
        }
    }

    /// Returns the number of bytes currently on the stack.
    #[must_use]
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Pushes a value of type `T` onto the stack.
    ///
    /// The stack owns the value until it is popped using the correct type or destroyed.
    /// `T: 'static` prevents owned destructors from retaining nonstatic borrows;
    /// borrowed inputs must instead use explicitly borrowed adapters.
    ///
    /// - Precondition: `T` requires no greater alignment than this stack's base alignment.
    ///   This requirement is checked in release builds to preserve memory safety.
    ///
    /// # Type Parameters
    ///
    /// * `T`: The type of the value to push.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::raw_stack::RawStack;
    /// let mut stack = RawStack::with_base_alignment(align_of::<u32>());
    /// let _ = stack.push(42u32);
    /// ```
    ///
    /// A value whose destructor borrows nonstatic data cannot enter this owning stack:
    ///
    /// ```compile_fail,E0597
    /// use cel_runtime::raw_stack::RawStack;
    ///
    /// /// Borrows text that must remain live until destruction.
    /// struct BorrowedDrop<'a>(&'a str);
    /// impl Drop for BorrowedDrop<'_> {
    ///     /// Reads the borrowed text during destruction.
    ///     fn drop(&mut self) {
    ///         assert!(!self.0.is_empty());
    ///     }
    /// }
    ///
    /// let mut stack = RawStack::with_base_alignment(align_of::<BorrowedDrop<'static>>());
    /// {
    ///     let text = String::from("borrowed");
    ///     stack.push(BorrowedDrop(&text));
    /// }
    /// drop(stack);
    /// ```
    ///
    /// # Complexity
    ///
    /// The function has an amortized O(1) time complexity.
    pub fn push<T: 'static>(&mut self, value: T) -> bool {
        assert!(align_of::<T>() <= self.buffer.base_alignment());
        let len = self.buffer.len();
        let aligned_index = align_index(align_of::<T>(), len);
        let new_len = aligned_index
            .checked_add(size_of::<T>())
            .expect("stack size overflow");

        self.owners.reserve(1);
        self.buffer.reserve((new_len - len).max(1));
        unsafe {
            self.buffer.set_len(new_len);
            if aligned_index - len > 0 {
                // write a 1 in the first padding byte and 0 in the rest
                self.buffer[len].write(1);
                self.buffer[len + 1..aligned_index].fill(MaybeUninit::new(0));
            }

            std::ptr::write(
                self.buffer.as_mut_ptr().add(aligned_index).cast::<T>(),
                value,
            );
        }
        let padding = aligned_index - len > 0;
        self.owners.push(if padding {
            Self::destroy_typed::<T, true>
        } else {
            Self::destroy_typed::<T, false>
        });
        padding
    }

    /// Pushes `size` raw bytes from `src`, aligned to `align`, using the same
    /// padding/marker-byte bookkeeping as [`push`](Self::push).
    ///
    /// `src` is typed as `MaybeUninit<u8>` rather than `u8` because the bytes
    /// being copied may include a source value's interior padding, which is
    /// itself uninitialized — reading it through a `u8` pointer instead would
    /// be undefined behavior even though this function never inspects the
    /// bytes' values.
    ///
    /// - Precondition: `align` is a power of two no greater than the stack's base alignment.
    /// - Postcondition: A raw-byte slot is registered; its bytes are not destroyed
    ///   as a typed value unless upgraded with `own_top`.
    /// - Complexity: Amortized O(size) time and space.
    ///
    /// # Safety
    /// `src` must be valid for reads of `size` bytes, and must not overlap the
    /// stack's internal buffer.
    pub unsafe fn push_raw(
        &mut self,
        align: usize,
        size: usize,
        src: *const MaybeUninit<u8>,
    ) -> bool {
        debug_assert!(align.is_power_of_two());
        assert!(align <= self.buffer.base_alignment());
        let len = self.buffer.len();
        let aligned_index = align_index(align, len);
        let new_len = aligned_index + size;

        self.owners.reserve(1);
        self.raw_owners.reserve(1);
        self.buffer.reserve((new_len - len).max(1));
        unsafe {
            self.buffer.set_len(new_len);
            if aligned_index - len > 0 {
                self.buffer[len].write(1);
                self.buffer[len + 1..aligned_index].fill(MaybeUninit::new(0));
            }
            std::ptr::copy_nonoverlapping(src, self.buffer.as_mut_ptr().add(aligned_index), size);
        }
        self.owners.push(Self::destroy_raw);
        self.raw_owners.push(RawOwner {
            depth: self.depth(),
            before: len,
            align,
            size,
            ty: None,
        });
        aligned_index - len > 0
    }

    /// Reserves aligned space for `size` bytes (using the same padding/marker
    /// bookkeeping as [`push`](Self::push)) and calls `write` with a pointer
    /// to that space instead of copying from a source buffer.
    ///
    /// - Precondition: `align` is a power of two no greater than the stack's base alignment.
    /// - Postcondition: `write` is called exactly once, before this method
    ///   returns, with a pointer to `size` freshly-reserved bytes.
    /// - Postcondition: Normal return registers a raw-byte slot; a panicking
    ///   initializer registers no slot, leaving byte restoration to its caller.
    /// - Complexity: Amortized O(size) plus initializer costs.
    ///
    /// # Safety
    /// `write` must fully initialize all `size` bytes at the pointer it's
    /// given before returning, except interior padding. If it panics, the caller
    /// must guard partially initialized resources.
    pub unsafe fn reserve_and_write(
        &mut self,
        align: usize,
        size: usize,
        write: impl FnOnce(*mut u8),
    ) -> bool {
        debug_assert!(align.is_power_of_two());
        assert!(align <= self.buffer.base_alignment());
        let len = self.buffer.len();
        let aligned_index = align_index(align, len);
        let new_len = aligned_index + size;

        self.owners.reserve(1);
        self.raw_owners.reserve(1);
        self.buffer.reserve((new_len - len).max(1));
        unsafe {
            self.buffer.set_len(new_len);
            if aligned_index - len > 0 {
                self.buffer[len].write(1);
                self.buffer[len + 1..aligned_index].fill(MaybeUninit::new(0));
            }
            write(self.buffer.as_mut_ptr().add(aligned_index).cast::<u8>());
        }
        self.owners.push(Self::destroy_raw);
        self.raw_owners.push(RawOwner {
            depth: self.depth(),
            before: len,
            align,
            size,
            ty: None,
        });
        aligned_index - len > 0
    }

    /// Copies `size` bytes starting at absolute buffer offset `offset` into `dst`.
    ///
    /// `dst` is typed as `MaybeUninit<u8>` rather than `u8` because the bytes
    /// being copied may be a value's interior padding, which is itself
    /// uninitialized — reading it through a `u8` pointer instead would be
    /// undefined behavior even though this function never inspects the
    /// bytes' values.
    ///
    /// # Safety
    /// `offset..offset + size` must be within the currently-initialized buffer;
    /// `dst` must be valid for writes of `size` bytes and must not overlap the
    /// stack's internal buffer.
    pub unsafe fn copy_from(&self, offset: usize, size: usize, dst: *mut MaybeUninit<u8>) {
        debug_assert!(offset + size <= self.buffer.len());
        unsafe {
            std::ptr::copy_nonoverlapping(self.buffer.as_ptr().add(offset), dst, size);
        }
    }

    /// Drops a value in place at absolute buffer offset `offset`, without
    /// altering the stack's tracked length.
    ///
    /// # Safety
    /// `offset` must point to a live, valid value; `run_drop` must correctly
    /// run that value's destructor given a pointer to its start. Its ownership
    /// entry must already have been removed with `release_top`.
    pub unsafe fn drop_at(&mut self, offset: usize, run_drop: impl FnOnce(*mut u8)) {
        unsafe { run_drop(self.buffer.as_mut_ptr().add(offset).cast::<u8>()) };
    }

    /// Reads a value at absolute buffer offset `offset`, given a callback that
    /// receives a pointer to its bytes.
    ///
    /// # Safety
    /// `offset` must point to a live, valid, properly-aligned value for the type the
    /// caller will reinterpret it as; `read` must not retain the pointer beyond the
    /// call. This byte operation does not transfer ownership; a caller moving out
    /// a value must explicitly release its ownership entry before destruction or
    /// stack cleanup can occur.
    pub unsafe fn read_at<R>(&self, offset: usize, read: impl FnOnce(*const u8) -> R) -> R {
        unsafe { read(self.buffer.as_ptr().add(offset).cast::<u8>()) }
    }

    /// Truncates the stack back to `new_len`, additionally stripping `padding`
    /// bytes that preceded the removed region (scanned the same way
    /// [`pop`](Self::pop) does).
    ///
    /// # Safety
    /// No live (undropped) value may exist at or above `new_len`; the caller must
    /// explicitly release all removed ownership entries first. This byte operation
    /// does not infer ownership transfer, including for zero-sized values.
    pub unsafe fn truncate_to(&mut self, new_len: usize, padding: bool) {
        debug_assert!(new_len <= self.buffer.len());
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

    /// Removes the top owner's bytes and metadata, then destroys its value in place.
    ///
    /// # Safety
    /// The top `size` bytes (plus padding if `padding` is true) must be a
    /// live, valid value; `run_drop` must correctly run its destructor given a
    /// pointer to its start. The value must have a registered top ownership entry.
    pub unsafe fn drop_sized(
        &mut self,
        size: usize,
        padding: bool,
        run_drop: impl FnOnce(*mut u8),
    ) {
        debug_assert!(size <= self.buffer.len());
        let p = self.buffer.len() - size;
        let pointer = unsafe { self.buffer.as_mut_ptr().add(p).cast::<u8>() };
        self.release_top(1);
        unsafe {
            self.truncate_to(p, padding);
            run_drop(pointer);
        }
    }

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
    /// buffer; no two destination ranges may overlap. `src_offsets` and
    /// `dest_offsets` must be in the same relative element order (element `i`'s
    /// source and destination must both be the `i`-th non-overlapping range in
    /// their respective layouts) — this method does not reorder elements, only
    /// re-pads between them. Given that and `dest_base >= ambient_start`, each
    /// element's destination start is guaranteed to be at or after every
    /// earlier element's source end, which is what makes processing in reverse
    /// index order below safe against clobbering not-yet-read source bytes.
    /// The caller must release ownership entries for every moved value before
    /// repacking, then register the resulting representation separately. This
    /// byte operation never infers ownership transfer.
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
            let base_ptr = self.buffer.as_mut_ptr();
            // Process highest index first: each element's destination is
            // provably at or after every earlier element's source end (see
            // `# Safety` above), so this order never overwrites source bytes
            // an earlier iteration still needs to read.
            for i in (0..sizes.len()).rev() {
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

    /// Pops a value of type `T` from the stack. Does not change the stack capacity.
    ///
    /// # Safety
    ///
    /// The type `T` must be the same type as the value on the top of the stack.
    /// `padding` must match its push result, and the value must have a registered
    /// ownership entry. Incorrect usage can lead to undefined behavior.
    ///
    /// # Type Parameters
    ///
    /// * `T`: The type of the value to pop.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::raw_stack::RawStack;
    /// let mut stack = RawStack::with_base_alignment(align_of::<u32>());
    /// let padding = stack.push(100u32);
    /// let value: u32 = unsafe { stack.pop(padding) };
    /// ```
    pub unsafe fn pop<T>(&mut self, padding: bool) -> T {
        let p: usize = self.buffer.len() - size_of::<T>();
        let result = unsafe { std::ptr::read(self.buffer.as_ptr().add(p).cast::<T>()) };
        // count the number of trailing 0s in the buffer before the result
        let padding_count = if padding {
            self.buffer[..p]
                .iter()
                .rev()
                .take_while(|&x| unsafe { x.assume_init() == 0 })
                .count()
                + 1
        } else {
            0
        };
        self.buffer.truncate(p - padding_count);
        self.release_top(1);
        result
    }

    /// Pops a value of type `T` from the stack and drops it.
    ///
    /// # Safety
    ///
    /// The type `T` must be the same type as the value on the top of the stack.
    /// Incorrect usage can lead to undefined behavior.
    ///
    /// Removes ownership and bytes before invoking the value's destructor.
    pub unsafe fn drop<T>(&mut self, padding: bool) {
        unsafe { self.pop::<T>(padding) };
    }
}

/* Test module */
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::cmp::max;
    use std::rc::Rc;

    /// Records destruction order and optionally panics after recording its identifier.
    struct Tracked {
        id: usize,
        drops: Rc<RefCell<Vec<usize>>>,
        panic: bool,
    }

    impl Drop for Tracked {
        /// Records this owner's destruction before optionally panicking.
        fn drop(&mut self) {
            self.drops.borrow_mut().push(self.id);
            assert!(!self.panic, "destructor panic");
        }
    }

    /// Pushes a tracked owner onto the stack.
    fn track(
        stack: &mut RawStack,
        drops: &Rc<RefCell<Vec<usize>>>,
        id: usize,
        panic: bool,
    ) -> bool {
        stack.push(Tracked {
            id,
            drops: drops.clone(),
            panic,
        })
    }

    #[test]
    /// Destroys live typed owners in reverse order across mixed alignments.
    fn live_owners_drop_in_reverse_order() {
        let drops = Rc::new(RefCell::new(Vec::new()));
        {
            let mut stack = RawStack::with_base_alignment(align_of::<Tracked>());
            track(&mut stack, &drops, 1, false);
            stack.push(1u8);
            track(&mut stack, &drops, 2, false);
        }
        assert_eq!(*drops.borrow(), [2, 1]);
    }

    #[test]
    /// Transfers a popped owner's destruction obligation to its caller.
    fn popped_owner_is_not_dropped_twice() {
        let drops = Rc::new(RefCell::new(Vec::new()));
        let mut stack = RawStack::with_base_alignment(align_of::<Tracked>());
        let padding = track(&mut stack, &drops, 1, false);
        let value: Tracked = unsafe { stack.pop(padding) };
        drop(stack);
        assert!(drops.borrow().is_empty());
        drop(value);
        assert_eq!(*drops.borrow(), [1]);
    }

    #[test]
    /// Destroys the top owner and restores its preceding byte boundary, including ZSTs.
    fn drop_top_preserves_prefix_and_padding() {
        ZST_DROPS.with(|count| count.set(0));
        let drops = Rc::new(RefCell::new(Vec::new()));
        let mut stack = RawStack::with_base_alignment(align_of::<Tracked>());
        stack.push(42u8);
        track(&mut stack, &drops, 1, false);
        stack.push(ZeroSized);
        let before_zero = stack.len();
        stack.drop_top();
        assert_eq!(stack.len(), before_zero);
        assert_eq!(stack.depth(), 2);
        ZST_DROPS.with(|count| assert_eq!(count.get(), 1));
        stack.drop_top();
        assert_eq!(stack.len(), 1);
        assert_eq!(stack.depth(), 1);
        assert_eq!(*drops.borrow(), [1]);
        let prefix: u8 = unsafe { stack.pop(false) };
        assert_eq!(prefix, 42);
        drop(stack);
        assert_eq!(*drops.borrow(), [1]);
    }

    #[test]
    /// Removes a top owner's obligation before its destructor panics.
    fn drop_top_panic_does_not_repeat_destruction() {
        let drops = Rc::new(RefCell::new(Vec::new()));
        let mut stack = RawStack::with_base_alignment(align_of::<Tracked>());
        track(&mut stack, &drops, 1, false);
        let origin = stack.len();
        track(&mut stack, &drops, 2, true);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            stack.drop_top();
        }));
        assert!(result.is_err());
        assert_eq!(stack.len(), origin);
        assert_eq!(stack.depth(), 1);
        assert_eq!(*drops.borrow(), [2]);
        drop(stack);
        assert_eq!(*drops.borrow(), [2, 1]);
    }

    thread_local! {
        static ZST_DROPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// Counts destruction of a value that occupies no stack bytes.
    struct ZeroSized;

    impl Drop for ZeroSized {
        /// Increments the thread-local destruction count.
        fn drop(&mut self) {
            ZST_DROPS.with(|count| count.set(count.get() + 1));
        }
    }

    #[test]
    /// Distinguishes zero-sized owners by depth and preserves the caller's prefix.
    fn scoped_unwind_preserves_zero_sized_prefix() {
        ZST_DROPS.with(|count| count.set(0));
        let mut stack = RawStack::with_base_alignment(align_of::<usize>());
        stack.push(42usize);
        stack.push(ZeroSized);
        let origin = stack.len();
        let depth = stack.depth();
        let result: anyhow::Result<()> = stack.with_unwind(origin, depth, |stack| {
            stack.push(ZeroSized);
            stack.push(ZeroSized);
            anyhow::bail!("callback failed")
        });
        assert!(result.is_err());
        assert_eq!(stack.len(), origin);
        assert_eq!(stack.depth(), depth);
        ZST_DROPS.with(|count| assert_eq!(count.get(), 2));
        drop(stack);
        ZST_DROPS.with(|count| assert_eq!(count.get(), 3));
    }

    #[test]
    /// Cleans a panicking callback's suffix without destroying the caller's owner.
    fn callback_panic_cleans_suffix() {
        let drops = Rc::new(RefCell::new(Vec::new()));
        let mut stack = RawStack::with_base_alignment(align_of::<Tracked>());
        track(&mut stack, &drops, 1, false);
        let origin = stack.len();
        let depth = stack.depth();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: anyhow::Result<()> = stack.with_unwind(origin, depth, |stack| {
                track(stack, &drops, 2, false);
                panic!("callback panic")
            });
        }));
        assert!(result.is_err());
        assert_eq!(*drops.borrow(), [2]);
        assert_eq!(stack.depth(), depth);
        assert_eq!(stack.len(), origin);
        drop(stack);
        assert_eq!(*drops.borrow(), [2, 1]);
    }

    #[test]
    /// Resumes remaining destruction after one destructor panics.
    fn destructor_panic_resumes_cleanup() {
        let drops = Rc::new(RefCell::new(Vec::new()));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut stack = RawStack::with_base_alignment(align_of::<Tracked>());
            track(&mut stack, &drops, 1, false);
            track(&mut stack, &drops, 2, true);
            track(&mut stack, &drops, 3, false);
        }));
        assert!(result.is_err());
        assert_eq!(*drops.borrow(), [3, 2, 1]);
    }

    #[test]
    /// Upgrades copied raw storage into an erased destruction obligation.
    fn raw_bytes_become_erased_owner() {
        let drops = Rc::new(RefCell::new(Vec::new()));
        let value = std::mem::ManuallyDrop::new(Tracked {
            id: 1,
            drops: drops.clone(),
            panic: false,
        });
        let mut stack = RawStack::with_base_alignment(align_of::<Tracked>());
        unsafe {
            stack.push_raw(
                align_of::<Tracked>(),
                size_of::<Tracked>(),
                (&*value as *const Tracked).cast(),
            );
        }
        stack.own_top(crate::value_type::ValueType::leaf::<Tracked>());
        drop(stack);
        assert_eq!(*drops.borrow(), [1]);
    }

    #[test]
    /// Destroys an erased top owner without retaining its layout at the call site.
    fn drop_top_destroys_erased_owner_and_padding() {
        let drops = Rc::new(RefCell::new(Vec::new()));
        let mut stack = RawStack::with_base_alignment(align_of::<Tracked>());
        stack.push(42u8);
        unsafe {
            stack.reserve_and_write(align_of::<Tracked>(), size_of::<Tracked>(), |pointer| {
                pointer.cast::<Tracked>().write(Tracked {
                    id: 1,
                    drops: drops.clone(),
                    panic: false,
                });
            });
        }
        stack.own_top(ValueType::leaf::<Tracked>());
        stack.drop_top();
        assert_eq!(stack.len(), 1);
        assert_eq!(stack.depth(), 1);
        assert_eq!(*drops.borrow(), [1]);
        drop(stack);
        assert_eq!(*drops.borrow(), [1]);
    }

    #[test]
    /// Retains successful callback pushes until normal stack destruction.
    fn successful_scope_keeps_owners() {
        let drops = Rc::new(RefCell::new(Vec::new()));
        let mut stack = RawStack::with_base_alignment(align_of::<Tracked>());
        let value = stack
            .with_unwind(0, 0, |stack| {
                track(stack, &drops, 1, false);
                Ok(42)
            })
            .unwrap();
        assert_eq!(value, 42);
        assert_eq!(stack.depth(), 1);
        assert!(drops.borrow().is_empty());
        drop(stack);
        assert_eq!(*drops.borrow(), [1]);
    }

    #[test]
    /// Removes erased side metadata when transferring the top value to typed code.
    fn typed_pop_transfers_erased_owner() {
        let drops = Rc::new(RefCell::new(Vec::new()));
        let mut stack = RawStack::with_base_alignment(align_of::<Tracked>());
        let padding = unsafe {
            stack.reserve_and_write(align_of::<Tracked>(), size_of::<Tracked>(), |pointer| {
                pointer.cast::<Tracked>().write(Tracked {
                    id: 1,
                    drops: drops.clone(),
                    panic: false,
                });
            })
        };
        stack.own_top(ValueType::leaf::<Tracked>());
        let value: Tracked = unsafe { stack.pop(padding) };
        assert_eq!(stack.depth(), 0);
        track(&mut stack, &drops, 2, false);
        drop(stack);
        assert_eq!(*drops.borrow(), [2]);
        drop(value);
        assert_eq!(*drops.borrow(), [2, 1]);
    }

    #[test]
    /// Leaves no valid owner registered when raw initialization panics.
    fn initializer_panic_registers_no_owner() {
        let mut stack = RawStack::with_base_alignment(align_of::<Tracked>());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            stack.reserve_and_write(align_of::<Tracked>(), size_of::<Tracked>(), |_| {
                panic!("uninitialized")
            });
        }));
        assert!(result.is_err());
        assert_eq!(stack.depth(), 0);
        stack.unwind_to(0, 0);
        assert_eq!(stack.len(), 0);
    }

    #[test]
    /// Releases zero-sized ownership without destroying values or changing bytes.
    fn release_top_preserves_bytes_and_releases_zero_sized_owner() {
        ZST_DROPS.with(|count| count.set(0));
        let mut stack = RawStack::with_base_alignment(align_of::<usize>());
        stack.push(42usize);
        stack.push(ZeroSized);
        let origin = stack.len();
        stack.release_top(1);
        assert_eq!(stack.len(), origin);
        assert_eq!(stack.depth(), 1);
        let value: usize = unsafe { stack.pop(false) };
        assert_eq!(value, 42);
        drop(stack);
        ZST_DROPS.with(|count| assert_eq!(count.get(), 0));
    }

    #[test]
    /// Restores the caller's prefix after a suffix destructor panics.
    fn scoped_destructor_panic_restores_prefix() {
        let drops = Rc::new(RefCell::new(Vec::new()));
        let mut stack = RawStack::with_base_alignment(align_of::<Tracked>());
        track(&mut stack, &drops, 1, false);
        let origin = stack.len();
        let depth = stack.depth();
        track(&mut stack, &drops, 2, false);
        track(&mut stack, &drops, 3, true);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            stack.unwind_to(origin, depth);
        }));
        assert!(result.is_err());
        assert_eq!(*drops.borrow(), [3, 2]);
        assert_eq!(stack.len(), origin);
        assert_eq!(stack.depth(), depth);
        drop(stack);
        assert_eq!(*drops.borrow(), [3, 2, 1]);
    }

    #[test]
    fn push_pop_u32() {
        let mut stack = RawStack::with_base_alignment(align_of::<u32>());
        let padding = stack.push(10u32);
        let result: u32 = unsafe { stack.pop(padding) };
        assert_eq!(result, 10);
    }

    #[test]
    fn multiple_push_pop() {
        let mut stack = RawStack::with_base_alignment(align_of::<u32>());
        let padding1 = stack.push(1u32);
        let padding2 = stack.push(2u32);
        let padding3 = stack.push(3u32);
        let v3: u32 = unsafe { stack.pop(padding3) };
        let v2: u32 = unsafe { stack.pop(padding2) };
        let v1: u32 = unsafe { stack.pop(padding1) };
        assert_eq!(v1, 1);
        assert_eq!(v2, 2);
        assert_eq!(v3, 3);
    }

    #[test]
    fn push_pop_different_types() {
        let mut stack = RawStack::with_base_alignment(max(align_of::<u32>(), align_of::<f64>()));
        let padding1 = stack.push(42u32);
        let padding2 = stack.push(42.14f64);
        let value_f: f64 = unsafe { stack.pop(padding2) };
        let value_u: u32 = unsafe { stack.pop(padding1) };
        assert_eq!(value_f, 42.14);
        assert_eq!(value_u, 42);
    }

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
        let value = 42.14f64;
        let padding2 = unsafe {
            stack.push_raw(
                align_of::<f64>(),
                size_of::<f64>(),
                (&value as *const f64).cast::<MaybeUninit<u8>>(),
            )
        };
        let popped: f64 = unsafe { stack.pop(padding2) };
        assert_eq!(popped, 42.14);
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
            stack_b.push_raw(
                align_of::<f64>(),
                size_of::<f64>(),
                (&value as *const f64).cast::<MaybeUninit<u8>>(),
            )
        };
        assert_eq!(padding_typed, padding_raw);
    }

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

    #[test]
    fn copy_from_reads_bytes_at_offset() {
        let mut stack = RawStack::with_base_alignment(align_of::<u32>());
        let _ = stack.push(10u32);
        let _ = stack.push(20u32);
        let mut buf = [0u8; 4];
        unsafe { stack.copy_from(0, 4, buf.as_mut_ptr().cast::<MaybeUninit<u8>>()) };
        assert_eq!(u32::from_ne_bytes(buf), 10);
    }

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
        stack.release_top(1);
        unsafe {
            stack.drop_at(0, |ptr| std::ptr::drop_in_place(ptr.cast::<DropCounter>()));
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
        stack.release_top(1);
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
            stack.drop_sized(size_of::<DropCounter>(), padding, |ptr| {
                std::ptr::drop_in_place(ptr.cast::<DropCounter>())
            });
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(stack.len(), 1);
    }

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

        stack.release_top(3);
        let padding = unsafe {
            stack.repack(
                ambient_start,
                dest_base,
                total_size,
                &src_offsets,
                &dest_offsets,
                &sizes,
            )
        };
        assert!(
            !padding,
            "ambient_start was already aligned; no leading pad expected"
        );
        assert_eq!(stack.len(), dest_base + total_size);

        let mut a = [0u8; 1];
        let mut b = [0u8; 4];
        let mut c = [0u8; 1];
        unsafe {
            stack.copy_from(dest_base, 1, a.as_mut_ptr().cast::<MaybeUninit<u8>>());
            stack.copy_from(dest_base + 4, 4, b.as_mut_ptr().cast::<MaybeUninit<u8>>());
            stack.copy_from(dest_base + 8, 1, c.as_mut_ptr().cast::<MaybeUninit<u8>>());
        }
        assert_eq!(a[0], 0xAA);
        assert_eq!(u32::from_ne_bytes(b), 0xBBBB_BBBB);
        assert_eq!(c[0], 0xCC);
    }

    #[test]
    fn repack_shifts_right_without_corrupting_unread_source_bytes() {
        // A misaligned ambient_start forces dest_base > ambient_start, which
        // means the destination of the *first* tuple element can land inside
        // the *source* range of a *later*, not-yet-copied element. Processing
        // elements low-index-first would silently corrupt that later element's
        // bytes before they're read; this test fails under that ordering and
        // passes under the correct (reverse) ordering.
        let mut stack = RawStack::with_base_alignment(align_of::<u32>());
        let _ = stack.push(0xFFu8); // sentinel, ambient offset 0, before the tuple
        let ambient_start = stack.len(); // 1: misaligned relative to u32's 4-byte align
        let _ = stack.push(0xDDu8); // element 0: ambient offset 1
        let _ = stack.push(0x1122_3344u32); // element 1: ambient offset 4 (3 bytes padded)

        // Ideal (u8, u32) layout from zero: offset 0 (u8), offset 4 (u32) -> total 8.
        let src_offsets = [1usize, 4];
        let dest_offsets = [0usize, 4];
        let sizes = [1usize, 4];
        let total_size = 8usize;
        let dest_base = 4usize; // align_index(4, ambient_start=1) == 4, so this shifts right

        stack.release_top(2);
        let padding = unsafe {
            stack.repack(
                ambient_start,
                dest_base,
                total_size,
                &src_offsets,
                &dest_offsets,
                &sizes,
            )
        };
        assert!(
            padding,
            "ambient_start (1) is not 4-aligned; a leading pad is expected"
        );
        assert_eq!(stack.len(), dest_base + total_size);

        let mut sentinel = [0u8; 1];
        let mut a = [0u8; 1];
        let mut b = [0u8; 4];
        unsafe {
            stack.copy_from(0, 1, sentinel.as_mut_ptr().cast::<MaybeUninit<u8>>());
            stack.copy_from(dest_base, 1, a.as_mut_ptr().cast::<MaybeUninit<u8>>());
            stack.copy_from(dest_base + 4, 4, b.as_mut_ptr().cast::<MaybeUninit<u8>>());
        }
        assert_eq!(
            sentinel[0], 0xFF,
            "bytes before the tuple must be untouched"
        );
        assert_eq!(a[0], 0xDD);
        assert_eq!(
            u32::from_ne_bytes(b),
            0x1122_3344,
            "the u32's bytes must survive the repack uncorrupted"
        );
    }
}
