use crate::c_stack_list::{CNil, CStackList, IntoCStackList};
use crate::list_traits::{List, ListTypeIteratorAdvance, TypeIdIterator};
use crate::memory::align_index;
use crate::raw_segment::RawSegment;
use crate::raw_stack::RawStack;
use crate::{CStackListHeadLimit, CStackListHeadPadded, ReverseList};
use anyhow::Result;
use anyhow::anyhow;
use anyhow::ensure;
use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::cell::Cell;
use std::cmp::max;
use std::mem::MaybeUninit;

thread_local! {
    // Safety: valid only during the execution of `call_dyn` on this thread.
    // Set before executing the segment; cleared by `DynCallGuard::drop` even on panic.
    static CALL_DYN_PTR: Cell<usize> = const { Cell::new(0) };
    static CALL_DYN_LEN: Cell<usize> = const { Cell::new(0) };
}

/// Clears the `call_dyn` thread-locals when dropped.
struct DynCallGuard;

impl Drop for DynCallGuard {
    // Zeroes rather than restores the prior thread-local state.
    // This is safe as long as call_dyn is not re-entered on the same thread
    // (no nested call_dyn). If nested calls become necessary in the future,
    // change to save/restore: capture (CALL_DYN_PTR, CALL_DYN_LEN) at guard
    // construction and restore them here instead of zeroing.
    fn drop(&mut self) {
        CALL_DYN_PTR.with(|c| c.set(0));
        CALL_DYN_LEN.with(|c| c.set(0));
    }
}

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
/// Never strips the tuple's own leading padding (if any): the extracted
/// element inherits that same leading-padding relationship unchanged, since
/// `tuple_base` is by construction already aligned for every element inside
/// the tuple (the tuple's own alignment is the max of all its elements'), so
/// re-pushing the target from `tuple_base` introduces no padding of its own.
///
/// - Complexity: O(n) in the tuple's arity.
///
/// # Safety
/// The top `tuple_size` bytes of `stack` must be a live tuple value whose
/// layout matches `associated`.
unsafe fn extract_tuple_element(
    stack: &mut RawStack,
    tuple_size: usize,
    associated: &[AssociatedType],
    index: usize,
) {
    let tuple_base = stack.len() - tuple_size;
    let target = &associated[index];
    debug_assert!(tuple_base.is_multiple_of(target.align));

    // MaybeUninit<u8>, not u8: `target`'s bytes may include its own interior
    // padding, which is itself uninitialized — reading it into a `Vec<u8>`
    // (whose elements must always be valid, initialized `u8`s) would be
    // undefined behavior even though these bytes are never inspected, only
    // moved.
    let mut scratch: Vec<MaybeUninit<u8>> = vec![MaybeUninit::uninit(); target.size];
    unsafe {
        stack.copy_from(
            tuple_base + target.offset,
            target.size,
            scratch.as_mut_ptr(),
        );
    }

    for (i, elem) in associated.iter().enumerate().rev() {
        if i == index {
            continue;
        }
        let elem_associated = &elem.associated;
        unsafe {
            stack.drop_at(tuple_base + elem.offset, |ptr| {
                (elem.dropper)(ptr, elem_associated)
            });
        }
    }

    unsafe {
        // padding=false: this truncates only down to tuple_base, never past
        // the tuple's own leading pad (see doc comment above).
        stack.truncate_to(tuple_base, false);
        let repushed_padding = stack.push_raw(target.align, target.size, scratch.as_ptr());
        debug_assert!(
            !repushed_padding,
            "tuple_base is already aligned for every element inside the tuple"
        );
    }
}

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

/// Trait for converting a type list into a list of stack information.
///
/// This trait allows compile-time type lists to be converted into runtime
/// stack information that can be used for type checking and cleanup.
pub trait ToTypeIdList: List {
    /// Converts the type list into a vector of stack information.
    ///
    /// This method creates `StackInfo` entries for each type in the list,
    /// including the necessary cleanup functions and padding information.
    fn to_stack_info_list() -> Vec<StackInfo>;
}

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

/// A dynamic segment that provides runtime type checking for stack operations.
///
/// This struct wraps a [`RawSegment`] and maintains type information about the stack
/// to ensure type safety during operation execution. It validates that operations
/// receive arguments of the correct type and manages stack cleanup.
///
/// # Type Safety
///
/// The segment tracks the types of values on the stack and verifies that operations
/// receive arguments of the expected type. This prevents runtime type mismatches
/// that could occur when using [`RawSegment`] directly.
///
/// # Examples
///
/// ```rust
/// use cel_runtime::DynSegment;
///
/// let mut segment = DynSegment::new::<()>();
/// segment.op0(|| 42u32);
/// segment.op1(|n: u32| n.to_string()).unwrap();
///
/// let result: String = segment.call0().unwrap();
/// assert_eq!(result, "42");
/// ```
pub struct DynSegment {
    pub(crate) segment: RawSegment,
    pub(crate) argument_ids: Vec<TypeId>,
    /// Type names for each argument slot, for error reporting (parallel to `argument_ids`).
    pub(crate) argument_names: Vec<Cow<'static, str>>,
    pub(crate) stack_ids: Vec<StackInfo>,
    /// Fixed byte offset `stack_ids[0]` is laid out relative to; established
    /// once at construction (post-argument space for a full segment, or the
    /// as-if-already-popped ambient offset for a fragment — see
    /// [`new_fragment`](Self::new_fragment)). The current top-of-stack offset
    /// is always recomputed from this plus `stack_ids`, never cached, so it
    /// can never drift out of sync after ops consume stack entries.
    base_stack_index: usize,
}

impl DynSegment {
    /// Creates a new empty segment with no operations.
    #[must_use]
    pub fn new<Args: IntoCStackList>() -> Self
    where
        ReverseList<Args::Output>: ToTypeIdList,
    {
        let stack_ids = ReverseList::<Args::Output>::to_stack_info_list();
        DynSegment {
            segment: RawSegment::new(),
            argument_ids: stack_ids.iter().map(|s| s.type_id).collect(),
            argument_names: stack_ids.iter().map(|s| s.type_name.clone()).collect(),
            stack_ids,
            base_stack_index: size_of::<ReverseList<Args::Output>>(),
        }
    }

    /// Create a DynSegment that is a fragment of a larger segment, it may
    /// be used to implement conditional execution.
    ///
    /// - Precondition: the top of the stack currently holds the condition
    ///   value that [`join2`](Self::join2) will pop before this fragment's
    ///   ops run — the fragment's own local offsets are computed as if that
    ///   pop had already happened, matching the layout the fragment will
    ///   actually see at execution time. (`join2` independently rejects a
    ///   non-`bool` condition with an `Err`, so a mismatched condition type
    ///   at this point is a pending parse error, not a violated invariant.)
    #[must_use]
    pub fn new_fragment(&self) -> Self {
        debug_assert!(
            !self.stack_ids.is_empty(),
            "new_fragment requires a condition value on top of the stack"
        );
        DynSegment {
            segment: RawSegment::new(),
            argument_ids: Vec::new(),
            argument_names: Vec::new(),
            stack_ids: Vec::new(),
            base_stack_index: self.stack_offset_after(self.stack_ids.len().saturating_sub(1)),
        }
    }

    /// Verifies that the argument types match the expected types on the type stack.
    ///
    /// Returns an error if the argument types don't match the expected types or if
    /// there are too many arguments.
    ///
    /// To avoid reversing the arguments and reversing the slice, this operation
    /// is done in argument order, not stack order.
    // REVISIT: pop_types should just return the last n padding values
    fn pop_types<L: ListTypeIteratorAdvance<TypeId> + 'static>(&mut self) -> Result<()> {
        ensure!(
            L::LENGTH <= self.stack_ids.len(),
            "wrong number of arguments: expected {}, got {}",
            L::LENGTH,
            self.stack_ids.len()
        );
        let start = self.stack_ids.len() - L::LENGTH;
        ensure!(
            TypeIdIterator::<L>::new().eq(self.stack_ids[start..].iter().map(|info| info.type_id)),
            "stack type ids do not match"
        );
        self.stack_ids.truncate(start);
        Ok(())
    }

    /// Computes the top-of-stack byte offset after the first `count` entries
    /// of `stack_ids`, replaying each entry's own alignment/size from
    /// `base_stack_index`.
    ///
    /// Recomputing on demand (rather than caching a running total) keeps this
    /// correct after any operation that removes entries from `stack_ids`
    /// (e.g. [`pop_types`](Self::pop_types)), since there is no cached value
    /// that could fall out of sync with the actual entries left on the stack.
    ///
    /// - Complexity: O(count).
    fn stack_offset_after(&self, count: usize) -> usize {
        let mut offset = self.base_stack_index;
        for info in &self.stack_ids[..count] {
            offset = align_index(info.align, offset);
            offset += info.size;
        }
        offset
    }

    /// Push type to stack and register dropper.
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
            raw_dropper: |ptr, _associated| unsafe { std::ptr::drop_in_place(ptr.cast::<T>()) },
            associated: Vec::new(),
        });
    }

    /// Returns the current parse-time stack byte offset.
    ///
    /// Snapshot this before parsing a tuple's first element and pass it to
    /// [`make_tuple`](Self::make_tuple).
    #[must_use]
    pub fn current_stack_offset(&self) -> usize {
        self.stack_offset_after(self.stack_ids.len())
    }

    /// Returns the arity of the tuple on top of the stack, or `None` if the
    /// top value isn't a tuple.
    #[must_use]
    pub fn peek_tuple_arity(&self) -> Option<usize> {
        let info = self.stack_ids.last()?;
        (info.type_id == TypeId::of::<DynTuple>()).then_some(info.associated.len())
    }

    /// Collapses the top `n` stack values (pushed starting at byte offset
    /// `ambient_start`, e.g. via [`current_stack_offset`](Self::current_stack_offset)
    /// captured before parsing the first element) into one tuple value.
    ///
    /// The tuple's internal layout (offsets between elements) depends only on
    /// the elements' own types — never on `ambient_start` — matching the
    /// layout that [`CStackList`]'s own nested `#[repr(C)]` cons cells
    /// produce when built via sequential `.push()` calls in the same
    /// declaration order: each element is placed at its own alignment, then
    /// the running offset is padded up to the maximum alignment of every
    /// element seen so far (not just the tuple's overall alignment) before
    /// the next element is placed, mirroring how each nested cons cell pads
    /// itself to its own alignment before the next field is appended.
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
        let mut offset = 0usize;
        let mut tuple_align = 1usize;
        let mut src_offsets = Vec::with_capacity(n);
        let mut associated = Vec::with_capacity(n);
        for elem in &elems {
            // ambient_offset tracks where this element already sits on the
            // ambient RawStack from ordinary sequential pushes — a plain
            // flat layout, unrelated to CStackList's nested convention.
            ambient_offset = align_index(elem.align, ambient_offset);
            src_offsets.push(ambient_offset);
            ambient_offset += elem.size;

            // offset tracks the element's position in the tuple's canonical
            // (CStackList-matching) layout: place at this element's own
            // alignment, then pad up to the running max alignment so far.
            offset = align_index(elem.align, offset);
            tuple_align = tuple_align.max(elem.align);

            associated.push(AssociatedType {
                type_id: elem.type_id,
                type_name: elem.type_name.clone(),
                offset,
                size: elem.size,
                align: elem.align,
                dropper: elem.raw_dropper,
                associated: elem.associated.clone(),
            });

            offset += elem.size;
            offset = align_index(tuple_align, offset);
        }
        // The last iteration's rounding already used the full tuple_align
        // (tuple_align has accumulated every element's alignment by then),
        // so `offset` is already the tuple's correct total size.
        let total_size = offset;
        let dest_base = align_index(tuple_align, ambient_start);

        let dest_offsets: Vec<usize> = associated.iter().map(|a| a.offset).collect();
        let sizes: Vec<usize> = elems.iter().map(|e| e.size).collect();

        self.segment.raw0_(move |stack| {
            unsafe {
                stack.repack(
                    ambient_start,
                    dest_base,
                    total_size,
                    &src_offsets,
                    &dest_offsets,
                    &sizes,
                );
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

        // The extracted element inherits the tuple's own leading-padding
        // relationship unchanged — see extract_tuple_element's doc comment
        // for why no realignment is needed or performed.
        let target = info.associated[index].clone();
        let associated = info.associated.clone();
        let tuple_padding = info.padding;
        let tuple_size = info.size;

        self.segment.raw0_(move |stack| {
            unsafe {
                extract_tuple_element(stack, tuple_size, &associated, index);
            }
            Ok(())
        });

        self.stack_ids.push(StackInfo {
            type_id: target.type_id,
            type_name: target.type_name,
            padding: tuple_padding,
            size: target.size,
            align: target.align,
            raw_dropper: target.dropper,
            associated: target.associated,
        });
    }

    /// Returns the padding flags for the top N entries of the type stack.
    ///
    /// - Complexity: O(N).
    fn get_last_n_padded<const N: usize>(&self) -> [bool; N] {
        let mut result = [false; N];
        let start = self.stack_ids.len().saturating_sub(N);
        for (i, info) in self.stack_ids[start..].iter().enumerate() {
            result[i] = info.padding;
        }
        result
    }

    /// Captures the current stack droppers for use when unwinding on error.
    ///
    /// - Complexity: O(n) in the current stack depth.
    fn capture_unwind(&self) -> Vec<(usize, bool, RawDropper, Vec<AssociatedType>)> {
        self.stack_ids
            .iter()
            .map(|info| {
                (
                    info.size,
                    info.padding,
                    info.raw_dropper,
                    info.associated.clone(),
                )
            })
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
                        stack.drop_sized(*size, *padding, |ptr| raw_dropper(ptr, associated));
                    }
                }
                Err(e)
            }
        }
    }

    /// Returns the `TypeId` of the value currently on top of the stack, or `None` if the stack is empty.
    ///
    /// Used to verify method output types at parse time without consuming the stack.
    #[must_use]
    pub fn peek_output_type_id(&self) -> Option<TypeId> {
        self.stack_ids.last().map(|info| info.type_id)
    }

    /// Returns a slice of the top N [`StackInfo`] entries (stack order: oldest first in the slice).
    ///
    /// Use this for operation lookup so errors can report type names. Returns an empty slice
    /// if `n` is 0 or greater than the current stack size.
    #[must_use]
    pub fn peek_stack_infos(&self, n: usize) -> &[StackInfo] {
        if n > self.stack_ids.len() {
            return &[];
        }
        let start = self.stack_ids.len() - n;
        &self.stack_ids[start..]
    }

    /// Pushes a nullary operation that takes no arguments and returns a value of type R.
    ///
    /// The return type is tracked in the type stack for subsequent operations.
    pub fn op0<R, F>(&mut self, op: F)
    where
        F: Fn() -> R + 'static,
        R: 'static,
    {
        self.segment.push_op0(op);
        self.push_type::<R>();
    }

    /// Pushes a nullary operation that takes no arguments and returns a `Result<R>`.
    ///
    /// If the operation succeeds, the result is pushed onto the stack. If it fails,
    /// the stack is unwound to its previous state and the error is propagated.
    pub fn op0r<R, F>(&mut self, op: F)
    where
        F: Fn() -> anyhow::Result<R> + 'static,
        R: 'static,
    {
        let unwind = self.capture_unwind();
        self.segment
            .raw0(move |stack| Self::unwind_on_err(&unwind, stack, op()));
        self.push_type::<R>();
    }

    /// Pushes a unary operation that takes one argument of type `T` and returns a `Result<R>`.
    ///
    /// If the operation succeeds, the result is pushed onto the stack. If it fails,
    /// the stack is unwound to its previous state and the error is propagated.
    ///
    /// # Errors
    ///
    /// Returns an error if the argument type does not match the expected type.
    pub fn op1r<T, R, F>(&mut self, op: F) -> Result<()>
    where
        F: Fn(T) -> anyhow::Result<R> + 'static,
        T: 'static,
        R: 'static,
    {
        let [p0] = self.get_last_n_padded::<1>();
        self.pop_types::<(T, ())>()?;
        let unwind = self.capture_unwind();
        self.segment.raw1(
            move |stack, t| Self::unwind_on_err(&unwind, stack, op(t)),
            p0,
        );
        self.push_type::<R>();
        Ok(())
    }

    /// Pushes a binary operation that takes two arguments of types `T` and `U` and returns a `Result<R>`.
    ///
    /// If the operation succeeds, the result is pushed onto the stack. If it fails,
    /// the stack is unwound to its previous state and the error is propagated.
    ///
    /// # Errors
    ///
    /// Returns an error if the argument types do not match the expected types.
    pub fn op2r<T, U, R, F>(&mut self, op: F) -> Result<()>
    where
        F: Fn(T, U) -> anyhow::Result<R> + 'static,
        T: 'static,
        U: 'static,
        R: 'static,
    {
        let [p0, p1] = self.get_last_n_padded::<2>();
        self.pop_types::<(T, (U, ()))>()?;
        let unwind = self.capture_unwind();
        self.segment.raw2(
            move |stack, t, u| Self::unwind_on_err(&unwind, stack, op(t, u)),
            p0,
            p1,
        );
        self.push_type::<R>();
        Ok(())
    }

    /// Pushes a value to the stack without any operations.
    pub fn just<T: 'static + Clone>(&mut self, value: T) {
        self.op0(move || value.clone());
    }

    /// Emits a zero-argument op that clones the call argument at `index` and pushes it.
    ///
    /// At execution time the op reads `inputs[index]` from the slice supplied to
    /// [`call_dyn`](Self::call_dyn) and clones the value onto the stack.
    ///
    /// - Precondition: Every call to [`call_dyn`] must supply an `inputs` slice where
    ///   `inputs[index]` is a value of type `T`.
    ///
    /// - Complexity: O(1).
    pub fn push_arg<T: 'static + Clone>(&mut self, index: usize) {
        self.segment.push_op0(move || {
            CALL_DYN_PTR.with(|ptr_cell| {
                CALL_DYN_LEN.with(|len_cell| {
                    let raw_ptr = ptr_cell.get() as *const &dyn Any;
                    let len = len_cell.get();
                    assert!(!raw_ptr.is_null(), "push_arg op invoked outside call_dyn");
                    debug_assert!(index < len, "push_arg index {index} out of range {len}");
                    // Safety: raw_ptr is non-null (checked above) and valid for the duration
                    // of the enclosing call_dyn call; DynCallGuard clears it on return.
                    let slice = unsafe { std::slice::from_raw_parts(raw_ptr, len) };
                    slice[index]
                        .downcast_ref::<T>()
                        .expect("push_arg type mismatch at runtime")
                        .clone()
                })
            })
        });
        self.push_type::<T>();
    }

    /// Executes the segment with `inputs` as call arguments and returns the final result.
    ///
    /// Ops registered via [`push_arg`](Self::push_arg) read their values from `inputs`
    /// by index at execution time. Unlike [`call0`](Self::call0), this method does not
    /// consume the type stack, so the same segment may be called repeatedly.
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - The segment requires pre-loaded arguments (created with a non-unit `Args` type).
    /// - The stack does not contain exactly one value after expression compilation.
    /// - The `TypeId` of `R` does not match the top-of-stack type.
    /// - Any op returns an error during execution.
    ///
    /// - Complexity: O(n) in the number of ops.
    pub fn call_dyn<R: 'static>(&mut self, inputs: &[&dyn Any]) -> anyhow::Result<R> {
        ensure!(
            self.argument_ids.is_empty(),
            "call_dyn: segment requires {} pre-loaded argument(s); \
             use call_dyn only with push_arg-based segments",
            self.argument_ids.len()
        );
        ensure!(
            self.stack_ids.len() == 1,
            "call_dyn: expected exactly 1 value on stack, got {}",
            self.stack_ids.len()
        );
        ensure!(
            self.stack_ids[0].type_id == TypeId::of::<R>(),
            "call_dyn: result type mismatch: expected {}, got {}",
            std::any::type_name::<R>(),
            self.stack_ids[0].type_name,
        );
        CALL_DYN_PTR.with(|c| c.set(inputs.as_ptr() as usize));
        CALL_DYN_LEN.with(|c| c.set(inputs.len()));
        let _guard = DynCallGuard;
        // Safety: type check above verified R matches stack top; bypasses pop_types
        // so stack_ids is not consumed, enabling repeated calls.
        unsafe { self.segment.call0() }
    }

    /// Pushes a unary operation that takes one argument of type T and returns a value of type R.
    ///
    /// Verifies that the top of the type stack matches the expected input type T
    /// before adding the operation.
    ///
    /// # Errors
    ///
    /// Returns an error if the argument type doesn't match the expected type.
    pub fn op1<T, R, F>(&mut self, op: F) -> Result<()>
    where
        F: Fn(T) -> R + 'static,
        T: 'static,
        R: 'static,
    {
        let [p0] = self.get_last_n_padded::<1>();
        self.pop_types::<(T, ())>()?;
        self.segment.push_op1(op, p0);
        self.push_type::<R>();
        Ok(())
    }

    /// Pushes a binary operation that takes two arguments of types T and U and returns a value of type R.
    ///
    /// Verifies that the top two types on the type stack match the expected input types U and T
    /// (in that order) before adding the operation.
    ///
    /// # Errors
    ///
    /// Returns an error if the argument types do not match the expected types.
    pub fn op2<T, U, R, F>(&mut self, op: F) -> Result<()>
    where
        F: Fn(T, U) -> R + 'static,
        T: 'static,
        U: 'static,
        R: 'static,
    {
        let [p0, p1] = self.get_last_n_padded::<2>();
        self.pop_types::<(T, (U, ()))>()?;
        self.segment.push_op2(op, p0, p1);
        self.push_type::<R>();
        Ok(())
    }

    /// Pushes a ternary operation that takes three arguments of types T, U, and V and returns a value of type R.
    ///
    /// Verifies that the top three types on the type stack match the expected input types V, U, and T
    /// (in that order) before adding the operation.
    ///
    /// # Errors
    ///
    /// Returns an error if the argument types do not match the expected types.
    pub fn op3<T, U, V, R, F>(&mut self, op: F) -> Result<()>
    where
        F: Fn(T, U, V) -> R + 'static,
        T: 'static,
        U: 'static,
        V: 'static,
        R: 'static,
    {
        let [p0, p1, p2] = self.get_last_n_padded::<3>();
        self.pop_types::<(T, (U, (V, ())))>()?;
        self.segment.push_op3(op, p0, p1, p2);
        self.push_type::<R>();
        Ok(())
    }

    /// Joins two conditional fragments into a conditional execution operation.
    ///
    /// This method creates a conditional operation that executes one of two fragments
    /// based on a boolean value on the stack. Both fragments must have no arguments
    /// and return the same type.
    ///
    /// # Arguments
    ///
    /// * `fragment_0` - The fragment to execute when the condition is true
    /// * `fragment_1` - The fragment to execute when the condition is false
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * Either fragment takes arguments
    /// * Either fragment doesn't return exactly one value
    /// * The fragments return different types
    /// * The top of the stack is not a boolean value
    pub fn join2(&mut self, mut fragment_0: DynSegment, fragment_1: DynSegment) -> Result<()> {
        let [p0] = self.get_last_n_padded::<1>();
        self.pop_types::<(bool, ())>()?;

        // fragment results must match and cannot take arguments.
        ensure!(
            fragment_0.argument_ids.is_empty(),
            "fragment 0 cannot take arguments, but has {} argument(s)",
            fragment_0.argument_ids.len()
        );
        ensure!(
            fragment_1.argument_ids.is_empty(),
            "fragment 1 cannot take arguments, but has {} argument(s)",
            fragment_1.argument_ids.len()
        );
        ensure!(
            fragment_0.stack_ids.len() == 1,
            "fragment 0 must have exactly 1 result, but has {}",
            fragment_0.stack_ids.len()
        );
        ensure!(
            fragment_1.stack_ids.len() == 1,
            "fragment 1 must have exactly 1 result, but has {}",
            fragment_1.stack_ids.len()
        );
        ensure!(
            fragment_0.stack_ids[0].type_id == fragment_1.stack_ids[0].type_id,
            "fragment result types must match"
        );

        self.stack_ids.push(fragment_0.stack_ids.pop().unwrap());
        self.segment.update_base_alignment(max(
            fragment_0.segment.base_alignment(),
            fragment_1.segment.base_alignment(),
        ));

        let raw_segment_0 = fragment_0.segment;
        let raw_segment_1 = fragment_1.segment;

        /*
           - pass the stack to call0
        */
        self.segment.raw0_(move |stack| {
            let conditional = unsafe { stack.pop(p0) };
            if conditional {
                unsafe {
                    raw_segment_0.call0_stack(stack)?;
                }
            } else {
                unsafe {
                    raw_segment_1.call0_stack(stack)?;
                }
            }
            Ok(())
        });
        Ok(())
    }

    /// Executes all operations in the segment and returns the final result.
    ///
    /// # Returns
    /// - `Ok(R)` if execution succeeds and the final value is of type R
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///   - There are unexpected arguments (expected none)
    ///   - The final type doesn't match R
    ///   - There are remaining values on the stack after getting the result
    ///
    pub fn call0<R>(&mut self) -> Result<R>
    where
        R: 'static,
    {
        if !self.argument_ids.is_empty() {
            return Err(anyhow::anyhow!(
                "expected no arguments, but segment requires {} argument(s)",
                self.argument_ids.len()
            ));
        }
        self.pop_types::<(R, ())>()?;
        if !self.stack_ids.is_empty() {
            return Err(anyhow::anyhow!(
                "{} value(s) left on execution stack",
                self.stack_ids.len()
            ));
        }
        unsafe { self.segment.call0() }
    }

    /// Executes all operations in the segment with one argument and returns the final result.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///   - The number of arguments doesn't match (expected one)
    ///   - The argument type doesn't match the expected type
    ///   - The final type doesn't match R
    ///   - There are remaining values on the stack after getting the result
    ///
    pub fn call1<A, R>(&mut self, arg: A) -> Result<R>
    where
        A: 'static,
        R: 'static,
    {
        if self.argument_ids.len() != 1 {
            return Err(anyhow::anyhow!(
                "expected 1 argument, but segment requires {} argument(s)",
                self.argument_ids.len()
            ));
        }
        if self.argument_ids[0] != TypeId::of::<A>() {
            let got = self.argument_names.first().map(Cow::as_ref).unwrap_or("?");
            return Err(anyhow::anyhow!(
                "argument type mismatch: expected {}, got {}",
                std::any::type_name::<A>(),
                got
            ));
        }
        self.pop_types::<(R, ())>()?;
        if !self.stack_ids.is_empty() {
            return Err(anyhow::anyhow!(
                "{} value(s) left on execution stack",
                self.stack_ids.len()
            ));
        }
        unsafe { self.segment.call1(arg) }
    }

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
        let mut align_so_far = 1usize;
        let associated = element_infos
            .iter()
            .map(|elem_info| {
                offset = align_index(elem_info.align, offset);
                align_so_far = align_so_far.max(elem_info.align);
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
                offset = align_index(align_so_far, offset);
                a
            })
            .collect();
        info.type_id = TypeId::of::<DynTuple>();
        info.type_name = Cow::Borrowed(std::any::type_name::<DynTuple>());
        info.raw_dropper = drop_tuple;
        info.associated = associated;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct DropCounter(Arc<AtomicUsize>);

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl Clone for DropCounter {
        fn clone(&self) -> Self {
            DropCounter(self.0.clone())
        }
    }

    #[test]
    fn drop_on_error() -> Result<(), anyhow::Error> {
        let mut segment = DynSegment::new::<()>();

        let drop_count = Arc::new(AtomicUsize::new(0));
        let tracker = DropCounter(drop_count.clone());

        segment.op0(move || tracker.clone());
        segment.op0r(|| -> Result<u32> { Err(anyhow::anyhow!("error")) });
        segment.op2(|_: DropCounter, _: u32| 42u32)?;

        assert_eq!(drop_count.load(Ordering::SeqCst), 0); // Nothing dropped yet
        let result = segment.call0::<u32>();
        assert!(matches!(result, Err(e) if e.to_string() == "error"));
        assert_eq!(drop_count.load(Ordering::SeqCst), 1); // The DropCounter from op0 was dropped

        Ok(())
    }

    #[test]
    fn op1r_success() -> Result<(), anyhow::Error> {
        let mut segment = DynSegment::new::<()>();
        segment.op0(|| 21u32);
        segment.op1r(|n: u32| Ok::<_, anyhow::Error>(n * 2))?;
        let result: u32 = segment.call0()?;
        assert_eq!(result, 42);
        Ok(())
    }

    #[test]
    fn op1r_error_unwinds() -> Result<(), anyhow::Error> {
        let mut segment = DynSegment::new::<()>();
        let drop_count = Arc::new(AtomicUsize::new(0));
        let tracker = DropCounter(drop_count.clone());
        segment.op0(move || tracker.clone());
        segment.op0(|| 7u32);
        segment.op1r(|_n: u32| -> Result<DropCounter> { Err(anyhow::anyhow!("op1r error")) })?;
        segment.op1(|_: DropCounter| 0u32)?;
        segment.op2(|_: DropCounter, x: u32| x)?; // consume to single u32 for call0
        let result = segment.call0::<u32>();
        assert!(result.is_err(), "expected Err, got {:?}", result);
        assert_eq!(result.unwrap_err().to_string(), "op1r error");
        // DropCounter (under the u32) was unwound when op1r failed.
        assert_eq!(drop_count.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[test]
    fn op2r_success() -> Result<(), anyhow::Error> {
        let mut segment = DynSegment::new::<()>();
        segment.op0(|| 10u32);
        segment.op0(|| 32u32);
        segment.op2r(|a: u32, b: u32| Ok::<_, anyhow::Error>(a + b))?;
        let result: u32 = segment.call0()?;
        assert_eq!(result, 42);
        Ok(())
    }

    #[test]
    fn op2r_error_unwinds() -> Result<(), anyhow::Error> {
        let mut segment = DynSegment::new::<()>();
        let drop_count = Arc::new(AtomicUsize::new(0));
        let tracker = DropCounter(drop_count.clone());
        segment.op0(move || tracker.clone());
        segment.op0(|| 7u32);
        segment.op0(|| 8u32);
        segment.op2r(|_a: u32, _b: u32| -> Result<DropCounter> {
            Err(anyhow::anyhow!("op2r error"))
        })?;
        segment.op1(|_: DropCounter| 0u32)?;
        segment.op2(|_: DropCounter, x: u32| x)?; // consume to single u32 for call0
        let result = segment.call0::<u32>();
        assert!(result.is_err(), "expected Err, got {:?}", result);
        assert_eq!(result.unwrap_err().to_string(), "op2r error");
        // DropCounter (under the two u32s) was unwound when op2r failed.
        assert_eq!(drop_count.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[test]
    fn segment_operations() -> Result<(), anyhow::Error> {
        let mut operations = DynSegment::new::<()>();

        operations.op0(|| -> u32 { 30 });
        operations.op0(|| -> u32 { 12 });
        operations.op2(|x: u32, y: u32| -> u32 { x + y })?;
        operations.op0(|| -> u32 { 100 });
        operations.op0(|| -> u32 { 10 });
        operations.op3(|x: u32, y: u32, z: u32| -> u32 { x + y - z })?;
        operations.op1(|x: u32| -> String { format!("result: {}", x) })?;

        let final_result: String = operations.call0()?;
        assert_eq!(final_result, "result: 132");

        Ok(())
    }

    #[test]
    fn segment_with_just() -> Result<(), anyhow::Error> {
        let mut operations = DynSegment::new::<()>();
        operations.just(42u32);
        let result: u32 = operations.call0()?;
        assert_eq!(result, 42);
        operations.just("hello".to_string());
        let result: String = operations.call0()?;
        assert_eq!(result, "hello");
        Ok(())
    }
    #[test]
    fn segment_with_argument() -> Result<(), anyhow::Error> {
        let mut operations = DynSegment::new::<(u32,)>();

        operations.op0(|| -> u32 { 12 });
        operations.op2(|x: u32, y: u32| -> u32 { x + y })?;
        operations.op0(|| -> u32 { 100 });
        operations.op0(|| -> u32 { 10 });
        operations.op3(|x: u32, y: u32, z: u32| -> u32 { x + y - z })?;
        operations.op1(|x: u32| -> String { format!("result: {}", x) })?;

        let final_result: String = operations.call1(30u32)?;
        assert_eq!(final_result, "result: 132");

        Ok(())
    }

    #[test]
    fn example_conditional_expression() -> Result<(), anyhow::Error> {
        let mut root_segment = DynSegment::new::<()>();
        root_segment.op0(|| true);
        root_segment.op0(|| false);
        root_segment.op2(|x: bool, y: bool| x && y)?;

        let mut segment_1 = root_segment.new_fragment();
        segment_1.op0(|| 42u32);

        let mut segment_2 = root_segment.new_fragment();
        segment_2.op0(|| 2u32);

        root_segment.join2(segment_1, segment_2)?;

        let result = root_segment.call0::<u32>()?;
        println!("Result: {}", result);

        Ok(())
    }

    #[test]
    fn push_arg_single_input() -> Result<(), anyhow::Error> {
        let mut seg = DynSegment::new::<()>();
        seg.push_arg::<i32>(0);
        let x: i32 = 42;
        let result: i32 = seg.call_dyn(&[&x as &dyn Any])?;
        assert_eq!(result, 42);
        Ok(())
    }

    #[test]
    fn push_arg_two_inputs_with_op() -> Result<(), anyhow::Error> {
        let mut seg = DynSegment::new::<()>();
        seg.push_arg::<i32>(0);
        seg.push_arg::<i32>(1);
        seg.op2(|a: i32, b: i32| a + b)?;
        let a: i32 = 3;
        let b: i32 = 4;
        let result: i32 = seg.call_dyn(&[&a as &dyn Any, &b as &dyn Any])?;
        assert_eq!(result, 7);
        Ok(())
    }

    #[test]
    fn call_dyn_is_repeatable() -> Result<(), anyhow::Error> {
        let mut seg = DynSegment::new::<()>();
        seg.push_arg::<i32>(0);
        let x: i32 = 5;
        let r1: i32 = seg.call_dyn(&[&x as &dyn Any])?;
        let y: i32 = 10;
        let r2: i32 = seg.call_dyn(&[&y as &dyn Any])?;
        assert_eq!(r1, 5);
        assert_eq!(r2, 10);
        Ok(())
    }

    #[test]
    fn call_dyn_type_mismatch_returns_error() {
        let mut seg = DynSegment::new::<()>();
        seg.push_arg::<i32>(0);
        let x: i32 = 5;
        let result = seg.call_dyn::<String>(&[&x as &dyn Any]);
        assert!(result.is_err(), "expected Err on type mismatch");
    }

    #[test]
    fn call_dyn_errors_if_segment_has_preloaded_arguments() {
        // DynSegment::new::<(T,)>() creates a segment that expects a pre-loaded T argument.
        let mut seg = DynSegment::new::<(i32,)>();
        let result = seg.call_dyn::<i32>(&[]);
        assert!(
            result.is_err(),
            "expected Err when segment has pre-loaded argument types"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("pre-loaded"),
            "error message should mention pre-loaded: {msg}"
        );
    }

    #[test]
    fn call_dyn_errors_if_stack_has_wrong_count() {
        let mut seg = DynSegment::new::<()>();
        seg.push_arg::<i32>(0);
        seg.push_arg::<i32>(1);
        // Two values on stack, no combining op — stack_ids.len() == 2
        let x: i32 = 1;
        let y: i32 = 2;
        let result = seg.call_dyn::<i32>(&[&x as &dyn Any, &y as &dyn Any]);
        assert!(result.is_err(), "expected Err when stack has 2 values");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("exactly 1"),
            "error message should mention count: {msg}"
        );
    }

    #[test]
    fn call_dyn_errors_if_op_returns_error() -> Result<(), anyhow::Error> {
        let mut seg = DynSegment::new::<()>();
        seg.push_arg::<i32>(0);
        seg.op1r(|_x: i32| -> anyhow::Result<i32> {
            Err(anyhow::anyhow!("op failed deliberately"))
        })?;
        let x: i32 = 5;
        let result = seg.call_dyn::<i32>(&[&x as &dyn Any]);
        assert!(result.is_err(), "expected Err when op fails");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("op failed"),
            "error message should propagate op error: {msg}"
        );
        Ok(())
    }

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
        // Return the deeper (pre-tuple) value, not the just-extracted one: the
        // extracted u32's own read doesn't depend on its padding flag being
        // correct (it's computed from the live buffer length), but correctly
        // recovering `extra` underneath it does.
        seg_b.op2(|extra: u8, _x: u32| extra).unwrap();
        assert_eq!(seg_b.call0::<u8>().unwrap(), 99);
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
    fn tuple_index_result_inherits_leading_padding_when_element_align_is_smaller() {
        // Regression test: index element 0 (u32, align 4) out of a leading-
        // padded (u32, u64) tuple (tuple_align 8) at a misaligned ambient
        // start. The extracted element's align (4) is smaller than the
        // tuple's own align (8), which previously caused the result's
        // padding flag and stack_index bookkeeping to disagree with the
        // actual runtime layout (see plan/task-5 review history) — popping a
        // deeper sentinel afterward would silently read the wrong bytes.
        let mut seg = DynSegment::new::<()>();
        seg.op0(|| 0xEEu8); // sentinel, ambient offset 0
        let ambient_start = seg.current_stack_offset(); // 1: misaligned for align-8 tuple
        seg.op0(|| 0xAABB_CCDDu32); // element 0
        seg.op0(|| 0x1122_3344_5566_7788u64); // element 1
        seg.make_tuple(2, ambient_start);
        seg.tuple_index(0);
        // Return the deeper sentinel, not the just-extracted u32: recovering
        // it correctly is exactly what depends on the fixed padding/offset
        // bookkeeping (the u32's own read would succeed even under the bug).
        seg.op2(|sentinel: u8, _x: u32| sentinel).unwrap();
        assert_eq!(seg.call0::<u8>().unwrap(), 0xEE);
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

    /// `(u32, u8, u8)` shape used to distinguish the correct CStackList-nested
    /// layout (offsets `[0, 4, 8]`, size 12) from the old flat `#[repr(C)]`
    /// struct formula (offsets `[0, 4, 5]`, size 8): a higher-alignment
    /// element (`u32`) is followed by two lower-alignment elements (`u8`,
    /// `u8`), which is exactly the case the two formulas disagree on.
    type DivergentAlignmentShape = CStackList<u8, CStackList<u8, CStackList<u32, CNil<()>>>>;

    fn build_divergent_alignment_shape() -> DivergentAlignmentShape {
        // Declaration order (u32, u8, u8): elem0 = u32 is pushed first (innermost
        // tail), elem2 = the second u8 is pushed last (outermost head).
        CStackList(
            CStackList(CStackList(().into_c_stack_list(), 0xAAu32), 0xBBu8),
            0xCCu8,
        )
    }

    #[test]
    fn make_tuple_then_index_last_element_of_divergent_alignment_shape() {
        // Confirms element 2 reads back correctly via make_tuple's own
        // internally consistent repack. This alone can't distinguish offset 8
        // from offset 5 (make_tuple's write and read paths both use the same
        // formula) — see `push_tuple_round_trips_divergent_alignment_shape`
        // below for the test that actually exercises the real CStackList
        // layout and would fail under the old (flat) formula.
        let mut seg = DynSegment::new::<()>();
        let ambient_start = seg.current_stack_offset();
        seg.op0(|| 0xAAu32);
        seg.op0(|| 0xBBu8);
        seg.op0(|| 0xCCu8);
        seg.make_tuple(3, ambient_start);
        assert_eq!(seg.peek_tuple_arity(), Some(3));
        seg.tuple_index(2);
        assert_eq!(seg.call0::<u8>().unwrap(), 0xCCu8);
    }

    #[test]
    fn push_tuple_round_trips_divergent_alignment_shape() {
        // Round-trips a real `CStackList<u8, CStackList<u8, CStackList<u32,
        // CNil<()>>>>` value (memory layout produced by rustc itself, not by
        // our offset formula) through `push_tuple`, then indexes each
        // element. This test fails under the old flat-struct offset formula
        // (which computes offset 5 for the last element, when the real
        // struct places it at offset 8) and passes under the fix.
        let sample = build_divergent_alignment_shape();
        // Confirm construction order really is (u32, u8, u8) in declaration
        // order (outermost push is last / head — see pop_tuple_as's doc
        // comment for this convention).
        assert_eq!(*sample.head(), 0xCCu8);
        assert_eq!(*sample.tail().head(), 0xBBu8);
        assert_eq!(*sample.tail().tail().head(), 0xAAu32);

        let mut seg = DynSegment::new::<()>();
        seg.op0(build_divergent_alignment_shape);
        seg.push_tuple::<DivergentAlignmentShape>();
        assert_eq!(seg.peek_tuple_arity(), Some(3));

        // tuple_index consumes the tuple, so index each element in its own
        // segment (mirrors make_tuple_then_index_each_element's pattern).
        let mut seg0 = DynSegment::new::<()>();
        seg0.op0(build_divergent_alignment_shape);
        seg0.push_tuple::<DivergentAlignmentShape>();
        seg0.tuple_index(0);
        assert_eq!(seg0.call0::<u32>().unwrap(), 0xAAu32);

        let mut seg1 = DynSegment::new::<()>();
        seg1.op0(build_divergent_alignment_shape);
        seg1.push_tuple::<DivergentAlignmentShape>();
        seg1.tuple_index(1);
        assert_eq!(seg1.call0::<u8>().unwrap(), 0xBBu8);

        seg.tuple_index(2);
        assert_eq!(seg.call0::<u8>().unwrap(), 0xCCu8);
    }

    #[test]
    fn make_tuple_then_pop_tuple_as_round_trips_divergent_alignment_shape()
    -> Result<(), anyhow::Error> {
        // Closes the loop on the other bridge direction: build via
        // make_tuple (which now computes the CStackList-matching layout),
        // relabel with pop_tuple_as, and confirm the resulting concrete
        // value's fields (read by rustc's own layout, via .head()/.tail())
        // agree with what was pushed for every element, including the
        // divergent-alignment last element.
        let mut seg = DynSegment::new::<()>();
        let ambient_start = seg.current_stack_offset();
        seg.op0(|| 0xAAu32);
        seg.op0(|| 0xBBu8);
        seg.op0(|| 0xCCu8);
        seg.make_tuple(3, ambient_start);
        assert_eq!(seg.peek_tuple_arity(), Some(3));

        seg.pop_tuple_as::<DivergentAlignmentShape>()?;
        let result = seg.call0::<DivergentAlignmentShape>()?;
        assert_eq!(*result.tail().tail().head(), 0xAAu32);
        assert_eq!(*result.tail().head(), 0xBBu8);
        assert_eq!(*result.head(), 0xCCu8);
        Ok(())
    }
}
