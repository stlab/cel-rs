use crate::c_stack_list::{CNil, CStackList, IntoCStackList};
use crate::dynamic_array::{
    ArrayElementType, DynamicArray, DynamicArrayBuilder, check_array_capacity,
};
use crate::dynamic_sequence::{
    DynamicSequence, ElementCloner, ElementDebug, ElementDropper, ElementEq, SequenceElement,
    SequenceList, TupleSequence, element_cloner_for, element_debug_for, element_dropper_for,
    element_eq_for,
};
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

/// Reads and clones a value of some fixed type from `ptr`, boxing it as
/// `Box<dyn Any>`.
///
/// # Safety
/// `ptr` must point to a valid, live, properly aligned value of the type this
/// function was generated for. The implementation must clone the value rather than
/// take ownership of it via a move or `ptr::read`, because the caller retains the
/// original bytes and drops them itself afterward — an implementation that moves out
/// of `ptr` instead of cloning causes a double-drop.
pub type BoxExtractor = unsafe fn(*const u8) -> Box<dyn Any>;

/// The semantic shape of a runtime value: an opaque leaf, an ordered tuple, or a
/// homogeneous array.
///
/// This discriminates shapes that share one physical `TypeId`: every tuple is tagged
/// [`DynTuple`] and every array is tagged [`DynamicArray`], so semantic identity lives here
/// rather than in the flat `TypeId`.
#[derive(Clone, Debug)]
pub enum ValueKind {
    /// A value with no runtime-visible children (every ordinary Rust type).
    Leaf,
    /// A tuple value, described by its ordered elements.
    Tuple(Vec<AssociatedType>),
    /// An array value, described by the one shape every element repeats.
    Array(ArrayElementType),
}

/// The complete type of a runtime value: its physical identity and layout (`type_id`,
/// `type_name`, `size`, `align`, and an in-place dropper) plus its semantic [`ValueKind`].
///
/// Physical metadata describes the bytes; `kind` describes what those bytes mean. The two are
/// deliberately separate: an array and a tuple of arrays share neither shape nor layout rules,
/// but every array has the same physical layout regardless of its element type.
///
/// Every field is private and set only by a validating constructor ([`leaf`](Self::leaf),
/// [`leaf_from_parts`](Self::leaf_from_parts), [`tuple`](Self::tuple), [`array`](Self::array)),
/// so a `ValueType`'s layout, dropper, and kind always agree — the invariant
/// [`same_shape`](Self::same_shape) relies on when it reports that two equal shapes have
/// identical layout.
#[derive(Clone, Debug)]
pub struct ValueType {
    /// Runtime type id of the value's concrete Rust representation.
    type_id: TypeId,
    /// Human-readable name for error reporting (borrowed when from `type_name::<T>()`).
    type_name: Cow<'static, str>,
    /// Size in bytes of the value.
    size: usize,
    /// Required alignment in bytes of the value.
    align: usize,
    /// In-place dropper for the value, callable at its own start address.
    raw_dropper: RawDropper,
    /// Semantic shape of the value.
    kind: ValueKind,
}

impl ValueType {
    /// Returns the leaf type describing `T`.
    ///
    /// - Precondition: `T` is neither [`DynTuple`] nor [`DynamicArray`] — those mark aggregates,
    ///   whose values are built by [`tuple`](Self::tuple) and [`array`](Self::array) so that the
    ///   aggregate's own shape travels with its `TypeId`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::ValueType;
    /// use std::any::TypeId;
    ///
    /// let leaf = ValueType::leaf::<i32>();
    /// assert_eq!(leaf.type_id(), TypeId::of::<i32>());
    /// assert_eq!(leaf.size(), size_of::<i32>());
    /// ```
    #[must_use]
    pub fn leaf<T: 'static>() -> Self {
        Self::leaf_from_parts(
            TypeId::of::<T>(),
            Cow::Borrowed(std::any::type_name::<T>()),
            size_of::<T>(),
            align_of::<T>(),
            raw_dropper_for::<T>(),
        )
    }

    /// Returns the leaf type described by already-erased metadata, for a caller that resolves a
    /// type by name at runtime rather than naming it statically.
    ///
    /// - Precondition: `size`, `align`, and `raw_dropper` are those of the single Rust type
    ///   identified by `type_id` (e.g. taken from [`raw_dropper_for`] for that same type).
    ///
    /// - Precondition: `type_id` is neither [`DynTuple`]'s nor [`DynamicArray`]'s — those mark
    ///   aggregates, whose values are built by [`tuple`](Self::tuple) and [`array`](Self::array)
    ///   so that the aggregate's own shape travels with its `TypeId`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{ValueType, raw_dropper_for};
    /// use std::any::TypeId;
    ///
    /// let leaf = ValueType::leaf_from_parts(
    ///     TypeId::of::<i32>(),
    ///     "i32".into(),
    ///     size_of::<i32>(),
    ///     align_of::<i32>(),
    ///     raw_dropper_for::<i32>(),
    /// );
    /// assert_eq!(leaf.type_id(), TypeId::of::<i32>());
    /// ```
    #[must_use]
    pub fn leaf_from_parts(
        type_id: TypeId,
        type_name: Cow<'static, str>,
        size: usize,
        align: usize,
        raw_dropper: RawDropper,
    ) -> Self {
        debug_assert!(align.is_power_of_two(), "align must be a power of two");
        debug_assert!(
            size.is_multiple_of(align),
            "size must be a multiple of align"
        );
        debug_assert!(
            type_id != TypeId::of::<DynTuple>() && type_id != TypeId::of::<DynamicArray>(),
            "a leaf value type must not claim an aggregate marker TypeId"
        );
        ValueType {
            type_id,
            type_name,
            size,
            align,
            raw_dropper,
            kind: ValueKind::Leaf,
        }
    }

    /// Returns the tuple type holding `elements`, laid out by [`layout_associated_recursive`]
    /// (every element's `offset` — and every nested tuple element's `size`/`align` — is
    /// overwritten, so callers supply only each leaf's own layout and each tuple's children).
    ///
    /// - Complexity: O(n) in the total (nested) element count.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{AssociatedType, ValueType};
    ///
    /// let tuple = ValueType::tuple(vec![
    ///     AssociatedType { offset: 0, value_type: ValueType::leaf::<i32>() },
    ///     AssociatedType { offset: 0, value_type: ValueType::leaf::<f64>() },
    /// ]);
    /// assert_eq!(tuple.size(), 16);
    /// assert_eq!(tuple.tuple_elements().unwrap()[1].offset, 8);
    /// ```
    #[must_use]
    pub fn tuple(mut elements: Vec<AssociatedType>) -> Self {
        let (size, align) = layout_associated_recursive(&mut elements);
        ValueType {
            type_id: TypeId::of::<DynTuple>(),
            type_name: Cow::Borrowed(std::any::type_name::<DynTuple>()),
            size,
            align,
            raw_dropper: drop_tuple,
            kind: ValueKind::Tuple(elements),
        }
    }

    /// Returns the array type whose elements all have shape `element`, named for diagnostics by
    /// that element (e.g. `[i32]`) rather than by the erased `DynamicArray` representation.
    ///
    /// - Complexity: O(depth) in the element's array nesting depth.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{ArrayElementType, DynamicArray, ValueType};
    /// use std::any::TypeId;
    ///
    /// let array = ValueType::array(ArrayElementType::leaf::<i32>().unwrap());
    /// assert_eq!(array.type_id(), TypeId::of::<DynamicArray>());
    /// assert_eq!(array.type_name(), "[i32]");
    /// assert_eq!(array.array_element().unwrap().type_id(), TypeId::of::<i32>());
    /// ```
    #[must_use]
    pub fn array(element: ArrayElementType) -> Self {
        ValueType {
            type_id: TypeId::of::<DynamicArray>(),
            type_name: Cow::Owned(format!("[{}]", element.display_name())),
            size: size_of::<DynamicArray>(),
            align: align_of::<DynamicArray>(),
            raw_dropper: raw_dropper_for::<DynamicArray>(),
            kind: ValueKind::Array(element),
        }
    }

    /// Returns the `TypeId` of this value's concrete Rust representation — the aggregate marker
    /// ([`DynTuple`] or [`DynamicArray`]) for a tuple or array, not an element type.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{DynamicArray, ArrayElementType, ValueType};
    /// use std::any::TypeId;
    ///
    /// assert_eq!(ValueType::leaf::<i32>().type_id(), TypeId::of::<i32>());
    /// let array = ValueType::array(ArrayElementType::leaf::<i32>().unwrap());
    /// assert_eq!(array.type_id(), TypeId::of::<DynamicArray>());
    /// ```
    #[must_use]
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Returns this value's human-readable type name, for diagnostics.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::ValueType;
    ///
    /// assert_eq!(ValueType::leaf::<i32>().type_name(), "i32");
    /// ```
    #[must_use]
    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    /// Returns this value's size in bytes.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::ValueType;
    ///
    /// assert_eq!(ValueType::leaf::<i32>().size(), size_of::<i32>());
    /// ```
    #[must_use]
    pub fn size(&self) -> usize {
        self.size
    }

    /// Returns this value's required alignment in bytes.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::ValueType;
    ///
    /// assert_eq!(ValueType::leaf::<i32>().align(), align_of::<i32>());
    /// ```
    #[must_use]
    pub fn align(&self) -> usize {
        self.align
    }

    /// Returns this value's semantic shape.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{ValueKind, ValueType};
    ///
    /// assert!(matches!(ValueType::leaf::<i32>().kind(), ValueKind::Leaf));
    /// assert!(matches!(ValueType::tuple(Vec::new()).kind(), ValueKind::Tuple(_)));
    /// ```
    #[must_use]
    pub fn kind(&self) -> &ValueKind {
        &self.kind
    }

    /// Returns this tuple's ordered elements, or `None` for a non-tuple value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::ValueType;
    ///
    /// assert!(ValueType::leaf::<i32>().tuple_elements().is_none());
    /// assert_eq!(ValueType::tuple(Vec::new()).tuple_elements().unwrap().len(), 0);
    /// ```
    #[must_use]
    pub fn tuple_elements(&self) -> Option<&[AssociatedType]> {
        match &self.kind {
            ValueKind::Tuple(elements) => Some(elements),
            _ => None,
        }
    }

    /// Returns this array's element shape, or `None` for a non-array value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{ArrayElementType, ValueType};
    ///
    /// let array = ValueType::array(ArrayElementType::leaf::<i32>().unwrap());
    /// assert!(array.array_element().is_some());
    /// assert!(ValueType::leaf::<i32>().array_element().is_none());
    /// ```
    #[must_use]
    pub fn array_element(&self) -> Option<&ArrayElementType> {
        match &self.kind {
            ValueKind::Array(element) => Some(element),
            _ => None,
        }
    }

    /// Returns the children to hand this value's own [`RawDropper`]: its tuple elements, or an
    /// empty slice for a leaf or array (whose droppers ignore the argument).
    pub(crate) fn dropper_children(&self) -> &[AssociatedType] {
        self.tuple_elements().unwrap_or(&[])
    }

    /// Returns the descriptor an array whose elements all have this value's shape would carry,
    /// or `None` for a tuple value — a CEL tuple is a stack-layout pseudo-value with no concrete
    /// Rust element representation yet (see
    /// <https://github.com/stlab/cel-rs/issues/213>).
    ///
    /// - Complexity: O(depth) in this value's array nesting depth.
    pub(crate) fn as_array_element(&self) -> Option<ArrayElementType> {
        match &self.kind {
            ValueKind::Leaf => Some(ArrayElementType::leaf_from_parts(
                self.type_id,
                self.type_name.clone(),
                self.size,
                self.align,
                self.raw_dropper,
            )),
            ValueKind::Array(element) => Some(ArrayElementType::array_of(element.clone())),
            ValueKind::Tuple(_) => None,
        }
    }

    /// Returns a copy of this type that describes the same bytes but drops nothing, for a region
    /// whose value has already been moved out and so must not be dropped again.
    ///
    /// This is the one deliberate exception to the rule that a `ValueType`'s dropper matches its
    /// type: layout, `TypeId`, and [`ValueKind`] still describe the original value, so offsets
    /// and shape comparisons stay correct while the cleanup pass skips the moved-out bytes.
    pub(crate) fn with_dropper_suppressed(&self) -> ValueType {
        ValueType {
            raw_dropper: |_ptr, _associated| {},
            ..self.clone()
        }
    }

    /// Returns whether `self` and `other` denote the same semantic type: the same `TypeId` for
    /// leaves, the same ordered element shapes (recursively) for tuples, and the same recursive
    /// element descriptor for arrays.
    ///
    /// Physical layout is never compared: two distinct types with identical size and alignment
    /// have different shapes, and two equal shapes always have identical layout.
    ///
    /// - Complexity: O(n) in the total (nested) element count.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{ArrayElementType, ValueType};
    ///
    /// let i32s = ValueType::array(ArrayElementType::leaf::<i32>().unwrap());
    /// let f64s = ValueType::array(ArrayElementType::leaf::<f64>().unwrap());
    /// assert!(!i32s.same_shape(&f64s));
    /// assert!(i32s.same_shape(&ValueType::array(ArrayElementType::leaf::<i32>().unwrap())));
    /// ```
    #[must_use]
    pub fn same_shape(&self, other: &ValueType) -> bool {
        if self.type_id != other.type_id {
            return false;
        }
        match (&self.kind, &other.kind) {
            (ValueKind::Leaf, ValueKind::Leaf) => true,
            (ValueKind::Tuple(a), ValueKind::Tuple(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|(x, y)| x.value_type.same_shape(&y.value_type))
            }
            (ValueKind::Array(a), ValueKind::Array(b)) => a == b,
            _ => false,
        }
    }
}

/// One element of a tuple: its byte offset from the start of the enclosing tuple, and its own
/// recursive [`ValueType`].
#[derive(Clone, Debug)]
pub struct AssociatedType {
    /// Byte offset from the start of the enclosing tuple.
    pub offset: usize,
    /// This element's own type.
    pub value_type: ValueType,
}

/// Marker type used as the `TypeId` for tuple aggregate stack entries.
///
/// A tuple's real type identity is the ordered element list in its
/// [`ValueKind::Tuple`], not this marker's `TypeId` — comparisons that need to
/// distinguish tuple shapes must inspect that list (e.g. via
/// [`ValueType::same_shape`]), not `type_id`.
#[derive(Debug)]
pub struct DynTuple;

/// `RawDropper` for a tuple value: drops each element at `ptr + element.offset`
/// in reverse order, recursing into nested tuples via their own droppers.
///
/// # Safety
/// `ptr` must point to a live tuple value whose layout matches `associated`.
pub unsafe fn drop_tuple(ptr: *mut u8, associated: &[AssociatedType]) {
    for elem in associated.iter().rev() {
        unsafe {
            (elem.value_type.raw_dropper)(ptr.add(elem.offset), elem.value_type.dropper_children());
        }
    }
}

/// Returns a [`RawDropper`] that drops a value of type `T` in place, ignoring the `associated`
/// parameter (a non-tuple leaf value has no nested elements to recurse into).
pub fn raw_dropper_for<T: 'static>() -> RawDropper {
    |ptr, _associated| unsafe { std::ptr::drop_in_place(ptr.cast::<T>()) }
}

/// Computes each element's on-stack byte offset in place — the one authoritative tuple layout
/// routine, used by [`ValueType::tuple`] (and through it by [`DynSegment::make_tuple`]): place
/// at this element's own alignment, then pad up to the running max alignment seen so far
/// (matching `CStackList`'s nested layout). Each element's `size` and `align` must already be
/// set; `offset` is overwritten. Returns `(total_size, max_align)`.
///
/// - Complexity: O(n).
///
/// # Examples
///
/// ```rust
/// use cel_runtime::{AssociatedType, ValueType, layout_associated};
///
/// let mut elements = vec![
///     AssociatedType { offset: 0, value_type: ValueType::leaf::<u8>() },
///     AssociatedType { offset: 0, value_type: ValueType::leaf::<u64>() },
/// ];
/// assert_eq!(layout_associated(&mut elements), (16, 8));
/// assert_eq!(elements[0].offset, 0);
/// assert_eq!(elements[1].offset, 8);
/// ```
pub fn layout_associated(elements: &mut [AssociatedType]) -> (usize, usize) {
    let mut offset = 0usize;
    let mut max_align = 1usize;
    for elem in elements.iter_mut() {
        offset = align_index(elem.value_type.align, offset);
        max_align = max_align.max(elem.value_type.align);
        elem.offset = offset;
        offset += elem.value_type.size;
        offset = align_index(max_align, offset);
    }
    (offset, max_align)
}

/// Recursively lays out `elements`, first fixing up every nested tuple element's own
/// `size`/`align` fields (bottom-up, from its own recursively-laid-out element list),
/// then delegating to [`layout_associated`] for this level's own (now-correct) offsets.
///
/// [`layout_associated`] itself is a flat, single-level computation: it trusts each element's
/// `size`/`align` as already correct, which holds unconditionally for leaf elements but not for
/// a nested tuple element built from a caller-declared shape — that element's real footprint is
/// only known once its own elements have been laid out. Fixing it up first means an
/// enclosing tuple's `total_size` always accounts for a nested tuple's true size, never a
/// caller-supplied placeholder.
///
/// - Precondition: every leaf element's `type_id`/`size`/`align` is already set; a nested tuple
///   element ([`ValueKind::Tuple`]) only needs its own element list set correctly — its
///   `size`/`align` are overwritten here, the same way `offset` is (recursively, at every
///   nesting level).
///
/// - Complexity: O(n) in the total (nested) element count.
///
/// # Examples
///
/// ```rust
/// use cel_runtime::{AssociatedType, ValueType, layout_associated_recursive};
///
/// let mut elements = vec![
///     AssociatedType { offset: 0, value_type: ValueType::leaf::<u8>() },
///     AssociatedType {
///         offset: 0,
///         value_type: ValueType::tuple(vec![
///             AssociatedType { offset: 0, value_type: ValueType::leaf::<u32>() },
///             AssociatedType { offset: 0, value_type: ValueType::leaf::<u32>() },
///         ]),
///     },
/// ];
/// // The nested tuple occupies 8 bytes at alignment 4, so it starts at offset 4.
/// assert_eq!(layout_associated_recursive(&mut elements), (12, 4));
/// assert_eq!(elements[1].offset, 4);
/// ```
pub fn layout_associated_recursive(elements: &mut [AssociatedType]) -> (usize, usize) {
    for elem in elements.iter_mut() {
        if let ValueKind::Tuple(children) = &mut elem.value_type.kind {
            let (size, align) = layout_associated_recursive(children);
            elem.value_type.size = size;
            elem.value_type.align = align;
        }
    }
    layout_associated(elements)
}

/// Extracts element `index` from the tuple currently on top of `stack`,
/// dropping every other element, leaving just the extracted value on top.
///
/// Also strips the tuple's own leading padding (if any): once only one
/// element survives, that space is dead — no `StackInfo` entry accounts for
/// it — so it must not linger, or later offset computations (e.g. for a
/// tuple literal built from this result) would disagree with the real
/// stack. The extracted element is re-pushed at its own natural alignment
/// relative to the ambient offset the tuple originally started at, which
/// `expected_padding` (computed the same way at parse time) predicts.
///
/// - Complexity: O(n) in the tuple's arity.
///
/// # Safety
/// The top `tuple_size` bytes of `stack` must be a live tuple value whose
/// layout matches `associated`, and `tuple_padding` must be that tuple's own
/// recorded leading-padding flag.
unsafe fn extract_tuple_element(
    stack: &mut RawStack,
    tuple_size: usize,
    tuple_padding: bool,
    associated: &[AssociatedType],
    index: usize,
    expected_padding: bool,
) {
    let tuple_base = stack.len() - tuple_size;
    let target = &associated[index];
    debug_assert!(tuple_base.is_multiple_of(target.value_type.align));

    // MaybeUninit<u8>, not u8: `target`'s bytes may include its own interior
    // padding, which is itself uninitialized — reading it into a `Vec<u8>`
    // (whose elements must always be valid, initialized `u8`s) would be
    // undefined behavior even though these bytes are never inspected, only
    // moved.
    let mut scratch: Vec<MaybeUninit<u8>> = vec![MaybeUninit::uninit(); target.value_type.size];
    unsafe {
        stack.copy_from(
            tuple_base + target.offset,
            target.value_type.size,
            scratch.as_mut_ptr(),
        );
    }

    for (i, elem) in associated.iter().enumerate().rev() {
        if i == index {
            continue;
        }
        let elem_children = elem.value_type.dropper_children();
        let elem_dropper = elem.value_type.raw_dropper;
        unsafe {
            stack.drop_at(tuple_base + elem.offset, |ptr| {
                elem_dropper(ptr, elem_children)
            });
        }
    }

    unsafe {
        // tuple_padding: strip the tuple's own leading pad too, all the way
        // back to the true ambient offset it was built from — not just down
        // to tuple_base (see doc comment above).
        stack.truncate_to(tuple_base, tuple_padding);
        let repushed_padding = stack.push_raw(
            target.value_type.align,
            target.value_type.size,
            scratch.as_ptr(),
        );
        debug_assert_eq!(
            repushed_padding, expected_padding,
            "extracted element's padding must match the parse-time prediction"
        );
    }
}

/// Information about a value on the stack: its complete [`ValueType`] and whether padding was
/// inserted before it for alignment.
pub struct StackInfo {
    /// Whether padding was inserted before this value for alignment.
    pub(crate) padding: bool,
    /// This slot's complete type: physical layout plus semantic shape.
    pub value_type: ValueType,
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
            padding: Self::HEAD_PADDED,
            value_type: ValueType::leaf::<H>(),
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
            argument_ids: stack_ids.iter().map(|s| s.value_type.type_id).collect(),
            argument_names: stack_ids
                .iter()
                .map(|s| s.value_type.type_name.clone())
                .collect(),
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
            TypeIdIterator::<L>::new().eq(self.stack_ids[start..]
                .iter()
                .map(|info| info.value_type.type_id)),
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
            offset = align_index(info.value_type.align, offset);
            offset += info.value_type.size;
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
            padding: padded,
            value_type: ValueType::leaf::<T>(),
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
        Some(self.stack_ids.last()?.value_type.tuple_elements()?.len())
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

        // ambient_offset tracks where each element already sits on the ambient
        // RawStack from ordinary sequential pushes — a plain flat layout,
        // unrelated to the tuple's own (CStackList-matching) layout, which
        // `ValueType::tuple` computes below.
        let mut ambient_offset = ambient_start;
        let mut src_offsets = Vec::with_capacity(n);
        let mut associated = Vec::with_capacity(n);
        for elem in elems {
            ambient_offset = align_index(elem.value_type.align, ambient_offset);
            src_offsets.push(ambient_offset);
            ambient_offset += elem.value_type.size;

            associated.push(AssociatedType {
                offset: 0,
                value_type: elem.value_type,
            });
        }

        let value_type = ValueType::tuple(associated);
        let total_size = value_type.size;
        let dest_base = align_index(value_type.align, ambient_start);

        let elements = value_type
            .tuple_elements()
            .expect("ValueType::tuple produces a tuple");
        let dest_offsets: Vec<usize> = elements.iter().map(|a| a.offset).collect();
        let sizes: Vec<usize> = elements.iter().map(|a| a.value_type.size).collect();

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
            padding: dest_base != ambient_start,
            value_type,
        });
    }

    /// Collapses the top `n` stack values (pushed starting at byte offset `ambient_start`, e.g.
    /// via [`current_stack_offset`](Self::current_stack_offset) captured before parsing the first
    /// element) into one [`DynamicArray`] value owning all of them.
    ///
    /// Every element must have the same complete semantic shape, compared by
    /// [`ValueType::same_shape`], so the resulting array carries exactly one element descriptor;
    /// nested arrays are compared recursively, so `[[0], [1.0]]` is rejected. At execution time
    /// each element's bytes move — once, without running its destructor — from the stack into one
    /// exact-capacity allocation that a later `Vec<T>` conversion can adopt without reallocating.
    ///
    /// - Precondition: at least `n` values are on the stack, pushed contiguously starting at
    ///   `ambient_start` with no other values interleaved.
    ///
    /// # Errors
    /// Returns an error, leaving the segment exactly as it was, when `n` is zero (an empty array
    /// literal has no inferable element type; see
    /// <https://github.com/stlab/cel-rs/issues/212>), when the stack holds fewer than `n` values,
    /// when an element is a CEL tuple (see <https://github.com/stlab/cel-rs/issues/213>), when
    /// two elements have different shapes, or when the array's storage layout would overflow.
    ///
    /// - Complexity: O(n) in the total (nested) element count.
    pub fn make_array(&mut self, n: usize, ambient_start: usize) -> Result<()> {
        ensure!(
            n != 0,
            "an empty array literal has no inferable element type; \
             see https://github.com/stlab/cel-rs/issues/212"
        );
        ensure!(
            n <= self.stack_ids.len(),
            "make_array: expected {n} array element(s) on the stack, found {}",
            self.stack_ids.len()
        );

        // Validate before touching any state: a rejected literal must leave this segment (and
        // the ops that produced its elements) exactly as it found them.
        let start = self.stack_ids.len() - n;
        let expected = &self.stack_ids[start].value_type;
        let element = expected
            .as_array_element()
            .ok_or_else(|| Self::tuple_element_error(0))?;
        for (offset, info) in self.stack_ids[start + 1..].iter().enumerate() {
            let index = offset + 1;
            ensure!(
                !matches!(info.value_type.kind(), ValueKind::Tuple(_)),
                Self::tuple_element_error(index)
            );
            ensure!(
                expected.same_shape(&info.value_type),
                "array element {index} has type {}, expected {}",
                info.value_type.type_name(),
                expected.type_name()
            );
        }
        check_array_capacity(&element, n)?;

        // Every element has the same shape, so they sit on the ambient stack at a fixed stride:
        // the first is placed at the element alignment, and each element's size is a multiple of
        // that alignment, so no interior padding separates them.
        let element_size = element.size();
        let first_offset = align_index(element.align(), ambient_start);
        debug_assert!(element_size.is_multiple_of(element.align()));
        // Captured while the elements are still on the parse-time stack, so the (unreachable,
        // since `check_array_capacity` already passed) allocation-failure path below drops every
        // value the evaluation had produced instead of abandoning them.
        let unwind = self.capture_unwind();
        self.stack_ids.truncate(start);

        let value_type = ValueType::array(element.clone());
        let dest_base = align_index(value_type.align, ambient_start);
        let padding = dest_base != ambient_start;
        // raw0_ (unlike raw0/push_op0) does not fold its own result alignment into the segment's
        // `base_alignment`, so a `RawStack` allocated for a later call could be aligned only for
        // the (possibly less-aligned) element type, leaving the pushed array misaligned.
        self.segment.update_base_alignment(value_type.align);

        self.segment.raw0_(move |stack| {
            // A fresh allocation per execution: a segment may be called repeatedly, and each
            // call's array must own its own storage.
            let mut builder = match DynamicArrayBuilder::with_capacity(element.clone(), n) {
                Ok(builder) => builder,
                Err(error) => return Self::unwind_on_err(&unwind, stack, Err(error.into())),
            };
            let mut offset = first_offset;
            for _ in 0..n {
                // Safety: `offset` is element `i`'s live, properly aligned value on the ambient
                // stack (the precondition on `ambient_start`), and `push_bytes` moves out of it
                // exactly once; the bytes it vacates are released below without being dropped,
                // so each element is owned by the builder from here on. Byte moves cannot
                // panic, so the builder's initialized prefix always matches the elements it has
                // actually taken: if anything did unwind, it would drop exactly those and free
                // its allocation, leaving the still-untransferred stack bytes to the ordinary
                // (drop-free) teardown of the raw stack rather than double-dropping them.
                unsafe {
                    stack.read_at(offset, |src| {
                        builder.push_bytes(src.cast::<MaybeUninit<u8>>());
                    });
                }
                offset += element_size;
            }
            // Safety: every value at or above `ambient_start` was moved into the builder, so no
            // live value remains there. Truncating the whole region at once is exactly the
            // reverse-order removal of those values (nothing is left behind between them), and
            // it strips the first element's leading pad too.
            unsafe { stack.truncate_to(ambient_start, false) };

            let array = builder.finish();
            let pushed_padding = stack.push(array);
            debug_assert_eq!(
                pushed_padding, padding,
                "the collected array's padding must match the parse-time prediction"
            );
            Ok(())
        });

        self.stack_ids.push(StackInfo {
            padding,
            value_type,
        });
        Ok(())
    }

    /// Returns the error reporting that array element `index` is a CEL tuple.
    fn tuple_element_error(index: usize) -> anyhow::Error {
        anyhow!(
            "array element {index} is a tuple; CEL tuples cannot yet be array elements, \
             see https://github.com/stlab/cel-rs/issues/213"
        )
    }

    /// Extracts element `index` from the tuple on top of the stack, replacing
    /// the whole tuple with just that element's value.    ///
    /// - Precondition: the top-of-stack value is a tuple with at least
    ///   `index + 1` elements.
    ///
    /// - Complexity: O(n) in the tuple's arity.
    pub fn tuple_index(&mut self, index: usize) {
        let info = self
            .stack_ids
            .pop()
            .expect("tuple_index requires a value on the stack");
        let tuple_padding = info.padding;
        let tuple_size = info.value_type.size;
        let ValueKind::Tuple(associated) = info.value_type.kind else {
            unreachable!("tuple_index requires a tuple on top of the stack");
        };
        debug_assert!(index < associated.len(), "tuple_index out of range");

        let target = associated[index].clone();

        // The tuple's own leading pad becomes dead space once torn down to
        // one element, so the extracted element gets its own padding flag:
        // recomputed relative to the ambient offset the tuple was originally
        // built from (with the tuple's own entry already popped above, this
        // replay gives exactly that offset), not inherited from the tuple.
        let ambient_before_tuple = self.stack_offset_after(self.stack_ids.len());
        let new_offset = align_index(target.value_type.align, ambient_before_tuple);
        let new_padding = new_offset != ambient_before_tuple;

        self.segment.raw0_(move |stack| {
            unsafe {
                extract_tuple_element(
                    stack,
                    tuple_size,
                    tuple_padding,
                    &associated,
                    index,
                    new_padding,
                );
            }
            Ok(())
        });

        self.stack_ids.push(StackInfo {
            padding: new_padding,
            value_type: target.value_type,
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

    /// Captures the current stack values' padding and types for use when unwinding on error.
    ///
    /// - Complexity: O(n) in the current stack depth.
    fn capture_unwind(&self) -> Vec<(bool, ValueType)> {
        self.stack_ids
            .iter()
            .map(|info| (info.padding, info.value_type.clone()))
            .collect()
    }

    /// Runs the captured droppers in reverse order on error, then propagates the error.
    fn unwind_on_err<R>(
        unwind: &[(bool, ValueType)],
        stack: &mut RawStack,
        result: Result<R>,
    ) -> Result<R> {
        match result {
            Ok(r) => Ok(r),
            Err(e) => {
                for (padding, value_type) in unwind.iter().rev() {
                    unsafe {
                        stack.drop_sized(value_type.size, *padding, |ptr| {
                            (value_type.raw_dropper)(ptr, value_type.dropper_children());
                        });
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
        self.stack_ids.last().map(|info| info.value_type.type_id)
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
    /// - Precondition: Every call to [`call_dyn`](Self::call_dyn) must supply an `inputs` slice where
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

    /// Emits an op that clones `inputs[index]` (downcast to `&DynamicSequence`) onto the stack
    /// as a live, tagged `DynTuple`, so ordinary CEL tuple indexing/operators work on a
    /// tuple-typed input cell exactly as they would on an inline tuple literal. `associated`
    /// describes the expected (declared) element types, recursively — a nested tuple element's
    /// own elements describe its inner shape the same way, and are expanded back into a
    /// nested on-stack tuple region (the inverse of
    /// [`call_dyn_as_dynamic_sequence`](Self::call_dyn_as_dynamic_sequence)'s "nested tuple →
    /// nested `DynamicSequence`" conversion). Offsets in `associated` are ignored and overwritten
    /// internally, recursively, via [`layout_associated_recursive`] — callers only need to supply
    /// each leaf element's own [`ValueType`] (see [`ValueType::leaf`]) or, for a nested tuple
    /// element, a [`ValueType::tuple`] carrying its own recursively-built elements, whose
    /// `size`/`align` are likewise recomputed here from that inner shape.
    ///
    /// - Precondition: every call to a `call_dyn`-family execution supplies an `inputs` slice
    ///   where `inputs[index]` is a `DynamicSequence` whose own shape matches `associated`
    ///   exactly (same arity and element `TypeId`s, recursively).
    ///
    /// - Complexity: O(total element count, including nested) to lay out the declared shape at
    ///   parse time; the op itself is O(total element count, including nested) at execution time.
    pub fn push_arg_as_dynamic_sequence_tuple(
        &mut self,
        index: usize,
        associated: Vec<AssociatedType>,
    ) {
        // `ValueType::tuple` lays `associated` out through the same routine every other tuple
        // goes through, recursively: a nested tuple element's *real* footprint is only known
        // once its own elements have been laid out, so that happens first, bottom-up,
        // overwriting the nested element's `size`/`align` before this level's own offsets (and
        // total size) are computed from them. Skipping this step would let a nested tuple's
        // declared placeholder `size` (however small) survive into this level's `total_size`,
        // under-reserving the stack space the write below actually needs for the nested region —
        // a buffer overrun, not just a wrong offset.
        let value_type = ValueType::tuple(associated);
        let total_size = value_type.size;
        let tuple_align = value_type.align;
        // raw0_ (unlike raw0/push_op0) does not fold its own return-type alignment into the
        // segment's `base_alignment`, and this op's write target has no return-type alignment to
        // fold in anyway — the tuple's alignment is only known from `associated`. Without this,
        // `base_alignment` could stay smaller than `tuple_align`, so the fresh `RawStack` a later
        // `call_dyn` allocates would not actually be aligned as strictly as `tuple_align` requires
        // (mirrors `join2`'s own explicit `update_base_alignment` call for the same reason).
        self.segment.update_base_alignment(tuple_align);
        let ambient_start = self.current_stack_offset();
        let dest_base = align_index(tuple_align, ambient_start);

        let write_shape = value_type
            .tuple_elements()
            .expect("ValueType::tuple produces a tuple")
            .to_vec();
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
            padding: dest_base != ambient_start,
            value_type,
        });
    }
}

/// Writes `seq`'s elements into `dst`, at the offsets in `dest_shape` (already computed via
/// [`layout_associated_recursive`]), cloning each leaf and recursively expanding each
/// nested-tuple element into its own nested on-stack tuple region.
///
/// # Safety
/// `dst` must be valid for writes covering every offset + size in `dest_shape`; `dest_shape`'s
/// element count and per-element shape (leaf vs. nested tuple, recursively) must match `seq`'s
/// own shape exactly.
///
/// - Complexity: O(total element count, including nested).
unsafe fn write_dynamic_sequence_as_tuple(
    seq: &DynamicSequence,
    dest_shape: &[AssociatedType],
    dst: *mut u8,
) {
    debug_assert_eq!(
        dest_shape.len(),
        seq.shape().len(),
        "write_dynamic_sequence_as_tuple: dest_shape and seq.shape() must have the same \
         element count (precondition violated by caller)"
    );
    for (dest_elem, src_elem) in dest_shape.iter().zip(seq.shape()) {
        if let Some(children) = dest_elem.value_type.tuple_elements() {
            // Safety: `read_element_at`'s own contract requires `src_elem.offset` to be one of
            // `seq`'s recorded element offsets (true by the zip above) and the callback not to
            // retain the pointer past the call (true here — it's used only to build `nested`,
            // itself a borrow, not retained beyond this statement). The nested `DynamicSequence`
            // is *borrowed*, not moved, out of `seq`'s bytes — `write_dynamic_sequence_as_tuple`
            // only ever clones through it, matching this function's own "clone, don't move"
            // contract, so `seq` (and its nested value) remains fully live and droppable
            // afterward. `dst.add(dest_elem.offset)` is in-bounds per this function's own safety
            // precondition on `dst`/`dest_shape`, and `children`/the nested `seq`'s
            // own shape match per this function's precondition (recursively).
            unsafe {
                seq.read_element_at(src_elem.offset, |src| {
                    let nested = &*src.cast::<DynamicSequence>();
                    write_dynamic_sequence_as_tuple(nested, children, dst.add(dest_elem.offset));
                });
            }
        } else {
            // Safety: same `read_element_at` contract as above; `src_elem.clone` clones (per
            // `ElementCloner`'s own contract) rather than moving, so `seq`'s original bytes stay
            // live; `dst.add(dest_elem.offset)` is in-bounds per this function's precondition.
            unsafe {
                seq.read_element_at(src_elem.offset, |src| {
                    (src_elem.clone)(src, dst.add(dest_elem.offset));
                });
            }
        }
    }
}

impl DynSegment {
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
            self.stack_ids[0].value_type.type_id == TypeId::of::<R>(),
            "call_dyn: result type mismatch: expected {}, got {}",
            std::any::type_name::<R>(),
            self.stack_ids[0].value_type.type_name,
        );
        CALL_DYN_PTR.with(|c| c.set(inputs.as_ptr() as usize));
        CALL_DYN_LEN.with(|c| c.set(inputs.len()));
        let _guard = DynCallGuard;
        // Safety: type check above verified R matches stack top; bypasses pop_types
        // so stack_ids is not consumed, enabling repeated calls.
        unsafe { self.segment.call0() }
    }

    /// Executes the segment once and splits its tuple result into one boxed value
    /// per element, using `extractors[i].1` to read element `i`, after checking that
    /// `extractors[i].0` matches element `i`'s runtime type.
    ///
    /// Unlike [`tuple_index`](Self::tuple_index), which permanently specializes a
    /// segment to extract one fixed element at parse time, this runs the segment
    /// exactly once at call time and reads every element from that one evaluation.
    ///
    /// # Safety
    /// Every `extractors[i].1` must satisfy [`BoxExtractor`]'s contract: it must clone
    /// the value at the pointer it's given rather than take ownership of it (e.g. via
    /// a move or `ptr::read`). This method checks that `extractors[i].0` matches
    /// element `i`'s runtime `TypeId`, but it cannot check what the function pointer
    /// actually does with the pointer — an extractor that moves instead of clones
    /// causes a double-drop once this method drops the tuple's original bytes
    /// afterward.
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
    pub unsafe fn call_dyn_tuple(
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
        let elements = info.value_type.tuple_elements().ok_or_else(|| {
            anyhow!(
                "call_dyn_tuple: expected a tuple result, got {}",
                info.value_type.type_name
            )
        })?;
        ensure!(
            elements.len() == extractors.len(),
            "call_dyn_tuple: tuple has {} element(s) but {} extractor(s) were supplied",
            elements.len(),
            extractors.len(),
        );
        for (i, (elem, (expected_type_id, _))) in elements.iter().zip(extractors).enumerate() {
            ensure!(
                elem.value_type.type_id == *expected_type_id,
                "call_dyn_tuple: element {i} type mismatch: expected type {:?}, got `{}`",
                expected_type_id,
                elem.value_type.type_name,
            );
        }

        let tuple_size = info.value_type.size;
        let tuple_padding = info.padding;
        let associated = elements.to_vec();

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
    pub fn call_dyn_as_tuple<T: TupleSequence>(&mut self, inputs: &[&dyn Any]) -> anyhow::Result<T>
    where
        T::Output: SequenceList,
    {
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
        let elements = info.value_type.tuple_elements().ok_or_else(|| {
            anyhow!(
                "call_dyn_as_tuple: expected a tuple result, got {}",
                info.value_type.type_name
            )
        })?;

        let mut expected = Vec::new();
        let mut max_align = 1usize;
        T::Output::append_shape(&mut expected, 0, &mut max_align);
        ensure!(
            elements.len() == expected.len()
                && elements
                    .iter()
                    .zip(&expected)
                    .all(|(a, b)| a.value_type.type_id == b.type_id),
            "call_dyn_as_tuple: tuple shape does not match `{}`",
            std::any::type_name::<T>(),
        );

        let tuple_size = info.value_type.size;
        let tuple_padding = info.padding;
        let associated = elements.to_vec();

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

        // `read_from` already moved every element's bytes out into `result` (per its own safety
        // contract: the caller must not separately drop those bytes afterward), so the tuple's
        // vacated bytes are dead space now, not a live value. This `truncate_to` is defensive
        // bookkeeping that mirrors `call_dyn_tuple`'s cleanup shape above, not a double-drop
        // guard: `stack` is a local `RawStack`, which has no `Drop` impl of its own that runs
        // element destructors, so letting it go out of scope untruncated was never going to
        // double-drop anything either way.
        unsafe {
            stack.truncate_to(tuple_base, tuple_padding);
        }

        Ok(result)
    }

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
        leaf: &impl Fn(TypeId) -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)>,
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
        let elements = info.value_type.tuple_elements().ok_or_else(|| {
            anyhow!(
                "call_dyn_as_dynamic_sequence: expected a tuple result, got {}",
                info.value_type.type_name
            )
        })?;

        // Validate every leaf's registration BEFORE executing the segment -- a purely static
        // check over `associated`/`leaf`, independent of the segment's runtime values -- so a
        // failure here means nothing has been executed, built, or moved yet, and there is
        // nothing to clean up. This also rules out the double-free that a post-execution
        // Err-cleanup path can't safely avoid: a nested tuple element can finish building (and
        // moving its own interior bytes into a fresh DynamicSequence) before a later *sibling*
        // element fails, at which point the original bytes of that already-moved nested element
        // are no longer solely owned by the on-stack tuple -- dropping them again from here
        // would double-drop them. Validating first means `build_dynamic_sequence` below is
        // guaranteed to succeed, so that scenario can never arise.
        validate_associated_shape(elements, leaf)?;

        let tuple_size = info.value_type.size;
        let tuple_padding = info.padding;
        let associated = elements.to_vec();

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
            stack.read_at(tuple_base, |base| {
                build_dynamic_sequence(base, &associated, leaf)
            })
        }
        .expect("validate_associated_shape already confirmed every leaf resolves");

        // Every leaf at every nesting depth was moved (not cloned) into a fresh DynamicSequence
        // above; the vacated bytes are dead space now, matching call_dyn_as_tuple's own cleanup.
        unsafe {
            stack.truncate_to(tuple_base, tuple_padding);
        }

        Ok(result)
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

    /// Pushes a ternary operation that takes three arguments of types `T`, `U`, and `V` and
    /// returns a `Result<R>`.
    ///
    /// If the operation succeeds, the result is pushed onto the stack. If it fails,
    /// the stack is unwound to its previous state and the error is propagated.
    ///
    /// # Errors
    ///
    /// Returns an error if the argument types do not match the expected types.
    pub fn op3r<T, U, V, R, F>(&mut self, op: F) -> Result<()>
    where
        F: Fn(T, U, V) -> anyhow::Result<R> + 'static,
        T: 'static,
        U: 'static,
        V: 'static,
        R: 'static,
    {
        let [p0, p1, p2] = self.get_last_n_padded::<3>();
        self.pop_types::<(T, (U, (V, ())))>()?;
        let unwind = self.capture_unwind();
        self.segment.raw3(
            move |stack, t, u, v| Self::unwind_on_err(&unwind, stack, op(t, u, v)),
            p0,
            p1,
            p2,
        );
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
            fragment_0.stack_ids[0]
                .value_type
                .same_shape(&fragment_1.stack_ids[0].value_type),
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
        let elements = info
            .value_type
            .tuple_elements()
            .ok_or_else(|| anyhow!("pop_tuple_as: top of stack is not a tuple"))?;
        let expected: Vec<TypeId> = L::to_stack_info_list()
            .iter()
            .map(|s| s.value_type.type_id)
            .collect();
        let actual: Vec<TypeId> = elements.iter().map(|a| a.value_type.type_id).collect();
        ensure!(
            expected == actual,
            "pop_tuple_as: tuple element types do not match `{}`",
            std::any::type_name::<L>()
        );
        debug_assert_eq!(info.value_type.size, size_of::<L>());
        debug_assert_eq!(info.value_type.align, align_of::<L>());

        let info = self.stack_ids.last_mut().expect("checked above");
        info.value_type = ValueType::leaf::<L>();
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
            info.value_type.type_id,
            TypeId::of::<L>(),
            "push_tuple: top of stack is not the expected type"
        );
        let elements = L::to_stack_info_list()
            .into_iter()
            .map(|elem_info| AssociatedType {
                offset: 0,
                value_type: elem_info.value_type,
            })
            .collect();
        // `L`'s own size/align are the tuple's: `ValueType::tuple` lays the elements out with
        // the same natural-alignment, declaration-order convention `L` itself uses.
        let value_type = ValueType::tuple(elements);
        debug_assert_eq!(value_type.size, size_of::<L>());
        debug_assert_eq!(value_type.align, align_of::<L>());
        info.value_type = value_type;
    }
}

/// Recursively converts a described tuple region at `base` into an owned `DynamicSequence`,
/// moving each leaf's bytes and recursing into nested tuple elements as nested `DynamicSequence`
/// values.
///
/// # Safety
/// `base` must point to a live value laid out exactly as described by `associated`.
unsafe fn build_dynamic_sequence(
    base: *const u8,
    associated: &[AssociatedType],
    leaf: &(
         impl Fn(TypeId) -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> + ?Sized
     ),
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
        let children = elem.value_type.tuple_elements();
        let is_tuple = children.is_some();
        let (size, align, drop, clone, eq, debug, value) = if let Some(children) = children {
            let nested = unsafe { build_dynamic_sequence(base.add(elem.offset), children, leaf)? };
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
            let (drop, clone, eq, debug) = leaf(elem.value_type.type_id).ok_or_else(|| {
                anyhow!(
                    "call_dyn_as_dynamic_sequence: no Clone/PartialEq registered for element \
                     type `{}`",
                    elem.value_type.type_name
                )
            })?;
            (
                elem.value_type.size,
                elem.value_type.align,
                drop,
                clone,
                eq,
                debug,
                Built::Leaf,
            )
        };
        let aligned = align_index(align, offset);
        max_align = max_align.max(align);
        shape.push(SequenceElement {
            type_id: if is_tuple {
                TypeId::of::<DynamicSequence>()
            } else {
                elem.value_type.type_id
            },
            type_name: if is_tuple {
                Cow::Borrowed(std::any::type_name::<DynamicSequence>())
            } else {
                elem.value_type.type_name.clone()
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
                            src_elem.value_type.size,
                        );
                    }
                }
            }
        });
    }

    Ok(unsafe { DynamicSequence::from_raw_parts(buffer, shape, max_align) })
}

/// Recursively confirms every non-tuple element in `associated` has a leaf descriptor
/// available from `leaf`, without moving or reading any bytes — a pure validation pass so a
/// caller can call this *before* executing a segment, guaranteeing that
/// `build_dynamic_sequence` (called afterward, on the same `associated`/`leaf`) cannot fail.
///
/// # Errors
/// Returns `Err` naming the first element (at any nesting depth) whose `TypeId` `leaf` doesn't
/// recognize.
fn validate_associated_shape(
    associated: &[AssociatedType],
    leaf: &(
         impl Fn(TypeId) -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> + ?Sized
     ),
) -> anyhow::Result<()> {
    for elem in associated {
        if let Some(children) = elem.value_type.tuple_elements() {
            validate_associated_shape(children, leaf)?;
        } else {
            leaf(elem.value_type.type_id).ok_or_else(|| {
                anyhow!(
                    "no Clone/PartialEq registered for element type `{}`",
                    elem.value_type.type_name
                )
            })?;
        }
    }
    Ok(())
}

/// One N>1-output slot's extraction, for [`DynSegment::call_dyn_tuple_mixed`]: either a scalar
/// leaf (the existing [`BoxExtractor`] path, identical to [`call_dyn_tuple`](DynSegment::call_dyn_tuple)),
/// or a nested tuple, converted to a boxed `DynamicSequence` via the same recursive machinery
/// [`call_dyn_as_dynamic_sequence`](DynSegment::call_dyn_as_dynamic_sequence) uses.
#[allow(clippy::type_complexity)]
pub enum DynExtractor {
    /// A scalar element: `extractors[i].0` must match that element's runtime `TypeId`;
    /// `extractors[i].1` reads and clones it (see [`BoxExtractor`]'s own safety contract).
    Scalar(TypeId, BoxExtractor),
    /// A nested-tuple element: the closure supplies each of *its own* leaves'
    /// `Drop`/`Clone`/`PartialEq`/`Debug` function pointers by `TypeId`, exactly like
    /// `call_dyn_as_dynamic_sequence`'s `leaf` parameter.
    Tuple(Box<dyn Fn(TypeId) -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)>>),
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
    ///
    /// - Complexity: O(n) in the number of ops, plus O(total element count, including nested) to
    ///   build the result.
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
        let elements = info.value_type.tuple_elements().ok_or_else(|| {
            anyhow!(
                "call_dyn_tuple_mixed: expected a tuple result, got {}",
                info.value_type.type_name
            )
        })?;
        ensure!(
            elements.len() == extractors.len(),
            "call_dyn_tuple_mixed: tuple has {} element(s) but {} extractor(s) were supplied",
            elements.len(),
            extractors.len(),
        );
        // Validate every element BEFORE executing the segment: a Scalar slot's type_id must
        // match, and a Tuple slot must itself be a nested tuple whose own leaves (at every
        // nesting depth) all resolve via its own `leaf` closure. This is a purely static check
        // over the result's elements/`extractors`, independent of the segment's runtime values,
        // so a failure here means nothing has been executed, built, or moved yet, and there is
        // nothing to clean up. It also rules out a double-free that a post-execution
        // Err-cleanup path can't safely avoid: a Tuple slot's own nested build can partially
        // succeed (moving some of its interior bytes into a fresh DynamicSequence) before a
        // later *sibling* slot fails, at which point those already-moved bytes are no longer
        // solely owned by the on-stack tuple -- dropping them again from here would double-drop
        // them. Validating first means every `build_dynamic_sequence` call below is guaranteed
        // to succeed, so that scenario can never arise.
        for (i, (elem, extractor)) in elements.iter().zip(extractors).enumerate() {
            match extractor {
                DynExtractor::Scalar(expected_type_id, _) => {
                    ensure!(
                        elem.value_type.type_id == *expected_type_id,
                        "call_dyn_tuple_mixed: element {i} type mismatch: expected type {:?}, \
                         got `{}`",
                        expected_type_id,
                        elem.value_type.type_name,
                    );
                }
                DynExtractor::Tuple(leaf) => {
                    let children = elem.value_type.tuple_elements().ok_or_else(|| {
                        anyhow!(
                            "call_dyn_tuple_mixed: element {i}: expected a nested tuple, got `{}`",
                            elem.value_type.type_name
                        )
                    })?;
                    validate_associated_shape(children, leaf.as_ref())?;
                }
            }
        }

        let tuple_size = info.value_type.size;
        let tuple_padding = info.padding;
        let associated = elements.to_vec();

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
        let results: Vec<Box<dyn Any>> = associated
            .iter()
            .zip(extractors)
            .map(|(elem, extractor)| match extractor {
                DynExtractor::Scalar(_, boxextractor) => unsafe {
                    stack.read_at(tuple_base + elem.offset, |ptr| boxextractor(ptr))
                },
                DynExtractor::Tuple(leaf) => {
                    let children = elem
                        .value_type
                        .tuple_elements()
                        .expect("validated above: a Tuple slot's element is a nested tuple");
                    let nested = unsafe {
                        stack.read_at(tuple_base + elem.offset, |base| {
                            build_dynamic_sequence(base, children, leaf.as_ref())
                        })
                    }
                    .expect("validated above: every leaf in this region is registered");
                    Box::new(nested) as Box<dyn Any>
                }
            })
            .collect();

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
                    offset: elem.offset,
                    value_type: elem.value_type.with_dropper_suppressed(),
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

    /// Returns a tuple value type whose elements are the given value types, laid out in order.
    fn tuple_of(elements: Vec<ValueType>) -> ValueType {
        ValueType::tuple(
            elements
                .into_iter()
                .map(|value_type| AssociatedType {
                    offset: 0,
                    value_type,
                })
                .collect(),
        )
    }

    /// Returns a tuple element of type `T`, at a placeholder offset.
    fn leaf_element<T: 'static>() -> AssociatedType {
        AssociatedType {
            offset: 0,
            value_type: ValueType::leaf::<T>(),
        }
    }

    #[test]
    fn value_shapes_distinguish_nested_array_element_types() {
        let i32_array = ValueType::array(ArrayElementType::leaf::<i32>().unwrap());
        let f64_array = ValueType::array(ArrayElementType::leaf::<f64>().unwrap());
        assert!(!i32_array.same_shape(&f64_array));
    }

    #[test]
    fn value_shapes_distinguish_array_nesting_depth() {
        let flat = ValueType::array(ArrayElementType::leaf::<i32>().unwrap());
        let nested = ValueType::array(ArrayElementType::array_of(
            ArrayElementType::leaf::<i32>().unwrap(),
        ));
        assert!(!flat.same_shape(&nested));
        assert!(flat.same_shape(&ValueType::array(ArrayElementType::leaf::<i32>().unwrap())));
    }

    #[test]
    fn value_shapes_distinguish_leaves_tuples_and_arrays() {
        let leaf = ValueType::leaf::<i32>();
        let tuple = tuple_of(vec![ValueType::leaf::<i32>()]);
        let array = ValueType::array(ArrayElementType::leaf::<i32>().unwrap());
        assert!(!leaf.same_shape(&tuple));
        assert!(!tuple.same_shape(&array));
        assert!(!array.same_shape(&leaf));
        assert!(leaf.same_shape(&ValueType::leaf::<i32>()));
        assert!(!leaf.same_shape(&ValueType::leaf::<f64>()));
    }

    #[test]
    fn nested_tuple_shapes_compare_by_every_nested_leaf() {
        let shape = || {
            tuple_of(vec![
                ValueType::leaf::<i32>(),
                tuple_of(vec![ValueType::leaf::<u8>(), ValueType::leaf::<f64>()]),
            ])
        };
        assert!(shape().same_shape(&shape()));

        let changed_leaf = tuple_of(vec![
            ValueType::leaf::<i32>(),
            tuple_of(vec![ValueType::leaf::<u8>(), ValueType::leaf::<i64>()]),
        ]);
        assert!(!shape().same_shape(&changed_leaf));

        let changed_arity = tuple_of(vec![
            ValueType::leaf::<i32>(),
            tuple_of(vec![ValueType::leaf::<u8>()]),
        ]);
        assert!(!shape().same_shape(&changed_arity));
    }

    #[test]
    fn tuple_shapes_distinguish_nested_array_element_types() {
        let with_i32 = tuple_of(vec![
            ValueType::leaf::<i32>(),
            ValueType::array(ArrayElementType::leaf::<i32>().unwrap()),
        ]);
        let with_f64 = tuple_of(vec![
            ValueType::leaf::<i32>(),
            ValueType::array(ArrayElementType::leaf::<f64>().unwrap()),
        ]);
        assert!(!with_i32.same_shape(&with_f64));
    }

    #[test]
    fn array_value_types_are_named_for_their_element_type() {
        let flat = ValueType::array(ArrayElementType::leaf::<i32>().unwrap());
        assert_eq!(flat.type_name(), "[i32]");

        let nested = ValueType::array(ArrayElementType::array_of(
            ArrayElementType::leaf::<i32>().unwrap(),
        ));
        assert_eq!(nested.type_name(), "[[i32]]");
    }

    #[test]
    fn array_value_types_carry_the_dynamic_array_layout() {
        let array = ValueType::array(ArrayElementType::leaf::<i32>().unwrap());
        assert_eq!(array.type_id(), TypeId::of::<DynamicArray>());
        assert_eq!(array.size(), size_of::<DynamicArray>());
        assert_eq!(array.align(), align_of::<DynamicArray>());
        assert!(matches!(array.kind(), ValueKind::Array(_)));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "a leaf value type must not claim an aggregate marker TypeId")]
    fn leaf_rejects_the_array_marker_type() {
        let _ = ValueType::leaf::<DynamicArray>();
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "a leaf value type must not claim an aggregate marker TypeId")]
    fn leaf_from_parts_rejects_the_tuple_marker_type() {
        let _ = ValueType::leaf_from_parts(
            TypeId::of::<DynTuple>(),
            Cow::Borrowed("DynTuple"),
            0,
            1,
            raw_dropper_for::<DynTuple>(),
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "align must be a power of two")]
    fn leaf_from_parts_rejects_a_non_power_of_two_alignment() {
        let _ = ValueType::leaf_from_parts(
            TypeId::of::<i32>(),
            Cow::Borrowed("i32"),
            6,
            3,
            raw_dropper_for::<i32>(),
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "size must be a multiple of align")]
    fn leaf_from_parts_rejects_a_size_that_is_not_a_multiple_of_align() {
        let _ = ValueType::leaf_from_parts(
            TypeId::of::<i32>(),
            Cow::Borrowed("i32"),
            3,
            4,
            raw_dropper_for::<i32>(),
        );
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
    fn op3r_success() -> Result<(), anyhow::Error> {
        let mut segment = DynSegment::new::<()>();
        segment.op0(|| 10u32);
        segment.op0(|| 20u32);
        segment.op0(|| 12u32);
        segment.op3r(|a: u32, b: u32, c: u32| Ok::<_, anyhow::Error>(a + b + c))?;
        let result: u32 = segment.call0()?;
        assert_eq!(result, 42);
        Ok(())
    }

    #[test]
    fn op3r_error_unwinds() -> Result<(), anyhow::Error> {
        let mut segment = DynSegment::new::<()>();
        let drop_count = Arc::new(AtomicUsize::new(0));
        let tracker = DropCounter(drop_count.clone());
        segment.op0(move || tracker.clone());
        segment.op0(|| 7u32);
        segment.op0(|| 8u32);
        segment.op0(|| 9u32);
        segment.op3r(|_a: u32, _b: u32, _c: u32| -> Result<DropCounter> {
            Err(anyhow::anyhow!("op3r error"))
        })?;
        segment.op1(|_: DropCounter| 0u32)?;
        segment.op2(|_: DropCounter, x: u32| x)?; // consume to single u32 for call0
        let result = segment.call0::<u32>();
        assert!(result.is_err(), "expected Err, got {:?}", result);
        assert_eq!(result.unwrap_err().to_string(), "op3r error");
        // DropCounter (under the three u32s) was unwound when op3r failed.
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
        let results = unsafe { seg.call_dyn_tuple(&[], &extractors) }?;
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
        let r1 = unsafe { seg.call_dyn_tuple(&[], &extractors) }?;
        let r2 = unsafe { seg.call_dyn_tuple(&[], &extractors) }?;
        assert_eq!(*r1[0].downcast_ref::<u32>().unwrap(), 1);
        assert_eq!(*r2[1].downcast_ref::<u32>().unwrap(), 2);
        Ok(())
    }

    #[test]
    fn call_dyn_tuple_errors_if_result_is_not_a_tuple() {
        let mut seg = DynSegment::new::<()>();
        seg.op0(|| 5u32);
        let extractors = [(TypeId::of::<u32>(), extract_u32 as BoxExtractor)];
        let result = unsafe { seg.call_dyn_tuple(&[], &extractors) };
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
        let result = unsafe { seg.call_dyn_tuple(&[], &extractors) };
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
        let result = unsafe { seg.call_dyn_tuple(&[], &extractors) };
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
        let result = unsafe { seg.call_dyn_tuple(&[], &extractors) };
        assert!(result.is_err(), "expected Err when op fails");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("op failed"),
            "error message should propagate op error: {msg}"
        );
    }

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

    #[test]
    fn call_dyn_as_tuple_moves_fields_without_double_dropping_or_leaking() {
        // Regression test: read_from moves each element's bytes out of the tuple by
        // std::ptr::read (per its own safety contract, the caller must not separately drop
        // those bytes afterward). If call_dyn_as_tuple also ran the tuple's drop glue over the
        // same bytes, any element with a real Drop impl would be dropped twice.
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Clone, Debug)]
        struct DropCounter(Arc<AtomicUsize>);
        impl PartialEq for DropCounter {
            fn eq(&self, other: &Self) -> bool {
                Arc::ptr_eq(&self.0, &other.0)
            }
        }
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
        seg.op0(|| 7i32);
        seg.make_tuple(2, ambient_start);

        let (extracted, n): (DropCounter, i32) = seg.call_dyn_as_tuple(&[]).unwrap();
        assert_eq!(n, 7);
        assert_eq!(
            drop_count.load(Ordering::SeqCst),
            0,
            "moving out must not drop the element"
        );
        drop(extracted);
        assert_eq!(
            drop_count.load(Ordering::SeqCst),
            1,
            "the moved-out value must still drop exactly once, on its own schedule"
        );
    }

    #[test]
    fn call_dyn_as_dynamic_sequence_builds_a_flat_sequence() -> anyhow::Result<()> {
        let mut seg = DynSegment::new::<()>();
        let ambient_start = seg.current_stack_offset();
        seg.op0(|| 10i32);
        seg.op0(|| 2.5f64);
        seg.make_tuple(2, ambient_start);

        let leaf =
            |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> {
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
        assert_eq!(seq.arity(), 2);
        let (a, b): (i32, f64) = seq.try_to_tuple()?;
        assert_eq!((a, b), (10, 2.5));
        Ok(())
    }

    #[test]
    fn call_dyn_as_dynamic_sequence_result_debug_formats_correctly() -> anyhow::Result<()> {
        let mut seg = DynSegment::new::<()>();
        let ambient_start = seg.current_stack_offset();
        seg.op0(|| 10i32);
        seg.op0(|| 2.5f64);
        seg.make_tuple(2, ambient_start);

        let leaf =
            |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> {
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

        let leaf =
            |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> {
                (type_id == TypeId::of::<i32>()).then(|| {
                    (
                        element_dropper_for::<i32>(),
                        element_cloner_for::<i32>(),
                        element_eq_for::<i32>(),
                        element_debug_for::<i32>(),
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
        let leaf = |_: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> {
            None
        };
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
        let leaf = |_: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> {
            None
        };
        let result = seg.call_dyn_as_dynamic_sequence(&[], &leaf);
        assert!(result.is_err());
    }

    #[test]
    fn call_dyn_as_dynamic_sequence_drops_every_element_exactly_once() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Clone, Debug)]
        struct DropCounter(Arc<AtomicUsize>);
        impl PartialEq for DropCounter {
            fn eq(&self, other: &Self) -> bool {
                Arc::ptr_eq(&self.0, &other.0)
            }
        }
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

        let leaf =
            |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> {
                if type_id == TypeId::of::<DropCounter>() {
                    Some((
                        element_dropper_for::<DropCounter>(),
                        element_cloner_for::<DropCounter>(),
                        element_eq_for::<DropCounter>(),
                        element_debug_for::<DropCounter>(),
                    ))
                } else if type_id == TypeId::of::<i32>() {
                    Some((
                        element_dropper_for::<i32>(),
                        element_cloner_for::<i32>(),
                        element_eq_for::<i32>(),
                        element_debug_for::<i32>(),
                    ))
                } else {
                    None
                }
            };
        let seq = seg.call_dyn_as_dynamic_sequence(&[], &leaf).unwrap();
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "moving out must not drop the element"
        );
        drop(seq);
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "must still drop exactly once"
        );
    }

    #[test]
    fn call_dyn_as_dynamic_sequence_never_executes_when_a_leaf_is_unregistered() {
        // Regression test: if `leaf` doesn't recognize some element's type, that must be caught
        // by validation *before* the segment ever executes -- there is nothing on the stack to
        // build, move, or clean up. The op closures' own captured state (here, the DropCounter
        // `a`) is untouched by the failed call and drops normally, exactly once, whenever `seg`
        // itself is later dropped -- not leaked (dropped zero times), and not double-dropped.
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Clone, Debug)]
        struct DropCounter(Arc<AtomicUsize>);
        impl PartialEq for DropCounter {
            fn eq(&self, other: &Self) -> bool {
                Arc::ptr_eq(&self.0, &other.0)
            }
        }
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

        // Only DropCounter's type is registered; i32 (the element after it) is not, so
        // call_dyn_as_dynamic_sequence must fail during validation, before executing the
        // segment at all.
        let leaf =
            |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> {
                (type_id == TypeId::of::<DropCounter>()).then(|| {
                    (
                        element_dropper_for::<DropCounter>(),
                        element_cloner_for::<DropCounter>(),
                        element_eq_for::<DropCounter>(),
                        element_debug_for::<DropCounter>(),
                    )
                })
            };
        let result = seg.call_dyn_as_dynamic_sequence(&[], &leaf);
        assert!(result.is_err());
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "the segment must never execute -- nothing was ever built or moved, so there is \
             nothing to drop yet"
        );
        drop(seg);
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "the op closure's own captured DropCounter must still drop exactly once, normally, \
             when `seg` itself is dropped"
        );
    }

    #[test]
    fn call_dyn_as_dynamic_sequence_errors_on_unregistered_sibling_after_nested_tuple() {
        // Regression test for a double-free a prior (now-reverted) fix attempt introduced: a
        // nested tuple element (element 0) whose own interior leaf *is* registered, followed by
        // a sibling scalar element (element 1) whose leaf is *not* registered. A naive
        // post-execution error-path cleanup could double-drop element 0's interior bytes, since
        // its own nested build could already have completed (moving its bytes into a fresh
        // DynamicSequence) before element 1's failure was discovered. Validating the whole shape
        // recursively before ever executing the segment means this is caught up front, before
        // anything is built -- so there is nothing to double-drop.
        let mut seg = DynSegment::new::<()>();
        let ambient_start = seg.current_stack_offset();
        let inner_start = seg.current_stack_offset();
        seg.op0(|| 1i32);
        seg.make_tuple(1, inner_start);
        seg.op0(|| 2.5f64);
        seg.make_tuple(2, ambient_start);

        // Only i32 (the nested tuple's own interior element) is registered; f64 (the outer
        // sibling element) is not.
        let leaf =
            |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> {
                (type_id == TypeId::of::<i32>()).then(|| {
                    (
                        element_dropper_for::<i32>(),
                        element_cloner_for::<i32>(),
                        element_eq_for::<i32>(),
                        element_debug_for::<i32>(),
                    )
                })
            };
        let result = seg.call_dyn_as_dynamic_sequence(&[], &leaf);
        assert!(result.is_err());
    }

    #[test]
    fn call_dyn_tuple_mixed_splits_a_tuple_output_among_scalar_and_tuple_slots()
    -> anyhow::Result<()> {
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
        let leaf =
            |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> {
                (type_id == TypeId::of::<i32>()).then(|| {
                    (
                        element_dropper_for::<i32>(),
                        element_cloner_for::<i32>(),
                        element_eq_for::<i32>(),
                        element_debug_for::<i32>(),
                    )
                })
            };
        let extractors = [
            DynExtractor::Scalar(TypeId::of::<i32>(), extract_i32 as BoxExtractor),
            DynExtractor::Tuple(Box::new(leaf)),
        ];
        let results = unsafe { seg.call_dyn_tuple_mixed(&[], &extractors) }?;
        assert_eq!(results.len(), 2);
        assert_eq!(*results[0].downcast_ref::<i32>().unwrap(), 1);
        let nested = results[1].downcast_ref::<DynamicSequence>().unwrap();
        assert_eq!(nested.arity(), 2);
        let (b, c): (i32, i32) = nested.try_to_tuple()?;
        assert_eq!((b, c), (2, 3));
        Ok(())
    }

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

        let leaf =
            |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> {
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
        let results = unsafe { seg.call_dyn_tuple_mixed(&[], &extractors) }?;
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
        let extractors = [DynExtractor::Scalar(
            TypeId::of::<u32>(),
            extract_u32 as BoxExtractor,
        )];
        let result = unsafe { seg.call_dyn_tuple_mixed(&[], &extractors) };
        assert!(result.is_err());
    }

    #[test]
    fn call_dyn_tuple_mixed_drops_a_moved_out_tuple_slot_exactly_once() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Clone, Debug)]
        struct DropCounter(Arc<AtomicUsize>);
        impl PartialEq for DropCounter {
            fn eq(&self, other: &Self) -> bool {
                Arc::ptr_eq(&self.0, &other.0)
            }
        }
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

        let leaf =
            |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> {
                if type_id == TypeId::of::<DropCounter>() {
                    Some((
                        element_dropper_for::<DropCounter>(),
                        element_cloner_for::<DropCounter>(),
                        element_eq_for::<DropCounter>(),
                        element_debug_for::<DropCounter>(),
                    ))
                } else if type_id == TypeId::of::<i32>() {
                    Some((
                        element_dropper_for::<i32>(),
                        element_cloner_for::<i32>(),
                        element_eq_for::<i32>(),
                        element_debug_for::<i32>(),
                    ))
                } else {
                    None
                }
            };
        let extractors = [DynExtractor::Tuple(Box::new(leaf))];
        let results = unsafe { seg.call_dyn_tuple_mixed(&[], &extractors) }.unwrap();
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "moving out must not drop the element"
        );
        drop(results);
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "must still drop exactly once, not zero or twice"
        );
    }

    #[test]
    fn call_dyn_tuple_mixed_never_executes_when_a_tuple_slots_leaf_is_unregistered() {
        // Regression test: element 0 is a Scalar slot; element 1 is a Tuple slot whose own
        // nested leaf lookup is unregistered. That must be caught by validation *before* the
        // segment executes -- not discovered partway through building results, where element 0
        // might already have been cloned. The op closures' own captured state (here, the
        // DropCounter `a`) is untouched by the failed call and drops normally, exactly once,
        // whenever `seg` itself is later dropped -- not leaked, and not double-dropped.
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Clone)]
        struct DropCounter(Arc<AtomicUsize>);
        impl PartialEq for DropCounter {
            fn eq(&self, other: &Self) -> bool {
                Arc::ptr_eq(&self.0, &other.0)
            }
        }
        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        fn extract_drop_counter(ptr: *const u8) -> Box<dyn Any> {
            Box::new(unsafe { (*ptr.cast::<DropCounter>()).clone() })
        }

        let count = Arc::new(AtomicUsize::new(0));
        let mut seg = DynSegment::new::<()>();
        let ambient_start = seg.current_stack_offset();
        let a = DropCounter(count.clone());
        seg.op0(move || a.clone());
        let inner_start = seg.current_stack_offset();
        seg.op0(|| 1i32);
        seg.make_tuple(1, inner_start);
        seg.make_tuple(2, ambient_start);

        // The nested tuple's own element type (i32) is never registered, so validation must
        // fail before the segment ever executes.
        let leaf = |_: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> {
            None
        };
        let extractors = [
            DynExtractor::Scalar(
                TypeId::of::<DropCounter>(),
                extract_drop_counter as BoxExtractor,
            ),
            DynExtractor::Tuple(Box::new(leaf)),
        ];
        let result = unsafe { seg.call_dyn_tuple_mixed(&[], &extractors) };
        assert!(result.is_err());
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "the segment must never execute -- element 0 was never cloned, so there is \
             nothing to drop yet"
        );
        drop(seg);
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "the op closure's own captured DropCounter must still drop exactly once, normally, \
             when `seg` itself is dropped"
        );
    }

    #[test]
    fn stack_info_records_size_and_align() {
        let mut seg = DynSegment::new::<()>();
        seg.op0(|| 7u32);
        let infos = seg.peek_stack_infos(1);
        assert_eq!(infos[0].value_type.size(), size_of::<u32>());
        assert_eq!(infos[0].value_type.align(), align_of::<u32>());
        assert!(matches!(infos[0].value_type.kind(), ValueKind::Leaf));
    }

    #[test]
    fn associated_type_carries_offset_size_align_dropper() {
        let a = AssociatedType {
            offset: 4,
            value_type: ValueType::leaf::<u32>(),
        };
        assert_eq!(a.offset, 4);
        assert_eq!(a.value_type.size(), 4);
        assert_eq!(a.value_type.align(), 4);
        assert_eq!(a.value_type.type_id(), TypeId::of::<u32>());
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
    fn repack_relocated_element_drops_exactly_once() {
        // Regression test for the concern that repack's ptr::copy-based move
        // never runs destructors at an element's old (pre-relocation) offset:
        // that's the same memmove-without-drop pattern std itself uses (e.g.
        // Vec::remove) and is sound as long as nothing treats the vacated
        // slot as live afterward, which make_tuple's bookkeeping guarantees.
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Clone)]
        struct DropCounter(Arc<AtomicUsize>);
        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        #[repr(align(16))]
        #[allow(dead_code)]
        struct Over16(u8);

        let drop_count = Arc::new(AtomicUsize::new(0));
        let tracker = DropCounter(drop_count.clone());

        let mut seg = DynSegment::new::<()>();
        seg.op0(|| 0xEEu8); // sentinel, forces a misaligned ambient_start
        let ambient_start = seg.current_stack_offset(); // 1
        seg.op0(move || tracker.clone()); // element 0: DropCounter, align 8
        seg.op0(|| Over16(0)); // element 1: align 16, forces tuple_align=16
        // tuple_align (16) > DropCounter's own align (8), so dest_base
        // (align_index(16, 1) == 16) differs from DropCounter's natural
        // ambient position (align_index(8, 1) == 8): repack must physically
        // relocate it, leaving its old bytes behind.
        seg.make_tuple(2, ambient_start);
        seg.tuple_index(0); // keep the DropCounter, drop Over16 in place
        seg.op2(|_sentinel: u8, val: DropCounter| val).unwrap();

        let extracted = seg.call0::<DropCounter>().unwrap();
        assert_eq!(
            drop_count.load(Ordering::SeqCst),
            0,
            "extracting must not have already dropped the relocated element"
        );
        drop(extracted);
        assert_eq!(
            drop_count.load(Ordering::SeqCst),
            1,
            "relocated element must drop exactly once (no leak, no double-drop)"
        );
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

    #[test]
    fn layout_associated_matches_make_tuples_own_padding_convention() {
        // Mirrors make_tuple's documented layout for (f64, i8, i8): each element at its own
        // alignment, then padded up to the running max alignment seen so far — inserting extra
        // padding between the two i8 elements once f64 raises the running max to 8.
        let mut elements = vec![
            leaf_element::<f64>(),
            leaf_element::<i8>(),
            leaf_element::<i8>(),
        ];
        let (total_size, align) = layout_associated(&mut elements);
        assert_eq!(
            elements.iter().map(|e| e.offset).collect::<Vec<_>>(),
            vec![0, 8, 16]
        );
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

    #[test]
    fn push_arg_as_dynamic_sequence_tuple_supports_tuple_indexing() -> anyhow::Result<()> {
        let seq = DynamicSequence::from_tuple((10i32, 2.5f64));
        let mut seg = DynSegment::new::<()>();
        let shape = vec![leaf_element::<i32>(), leaf_element::<f64>()];
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
        let leaf =
            |type_id: TypeId| -> Option<(ElementDropper, ElementCloner, ElementEq, ElementDebug)> {
                (type_id == TypeId::of::<i32>()).then(|| {
                    (
                        element_dropper_for::<i32>(),
                        element_cloner_for::<i32>(),
                        element_eq_for::<i32>(),
                        element_debug_for::<i32>(),
                    )
                })
            };
        let seq = source.call_dyn_as_dynamic_sequence(&[], &leaf)?;

        let inner_shape = vec![leaf_element::<i32>(), leaf_element::<i32>()];
        let outer_shape = vec![
            leaf_element::<i32>(),
            AssociatedType {
                offset: 0,
                value_type: ValueType::tuple(inner_shape),
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
        let shape = || vec![leaf_element::<i32>(), leaf_element::<i32>()];

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

    /// An element type with no derived traits at all, to prove array collection needs none.
    #[derive(Debug, PartialEq)]
    struct NoTraits(u32);

    #[repr(align(64))]
    #[derive(Debug, PartialEq)]
    struct OverAligned(u64);

    static ZST_ELEMENT_DROPS: AtomicUsize = AtomicUsize::new(0);

    /// A zero-sized element whose destructor must still run once per collected element.
    struct ZstWithDrop;

    impl Drop for ZstWithDrop {
        fn drop(&mut self) {
            ZST_ELEMENT_DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn make_array_collects_homogeneous_values() -> anyhow::Result<()> {
        let mut segment = DynSegment::new::<()>();
        let start = segment.current_stack_offset();
        segment.just(0i32);
        segment.just(1i32);
        segment.just(2i32);
        segment.make_array(3, start)?;

        let array: DynamicArray = segment.call0()?;
        assert_eq!(array.len(), 3);
        assert_eq!(array.capacity(), 3);
        assert_eq!(array.try_into_vec::<i32>()?, vec![0, 1, 2]);
        Ok(())
    }

    #[test]
    fn make_array_rejects_mismatched_values_before_execution() {
        let mut segment = DynSegment::new::<()>();
        let start = segment.current_stack_offset();
        segment.just(0i32);
        segment.just(1f64);

        let error = segment.make_array(2, start).unwrap_err().to_string();

        assert!(error.contains("element 1"), "{error}");
        assert!(error.contains("i32") && error.contains("f64"), "{error}");
    }

    #[test]
    fn make_array_rejects_an_empty_element_list() {
        let mut segment = DynSegment::new::<()>();
        let start = segment.current_stack_offset();

        let error = segment.make_array(0, start).unwrap_err().to_string();

        assert!(error.contains("212"), "{error}");
    }

    #[test]
    fn make_array_rejects_more_elements_than_the_stack_holds() {
        let mut segment = DynSegment::new::<()>();
        let start = segment.current_stack_offset();
        segment.just(0i32);

        assert!(segment.make_array(2, start).is_err());
    }

    #[test]
    fn make_array_collects_values_of_a_type_without_clone() -> anyhow::Result<()> {
        let mut segment = DynSegment::new::<()>();
        let start = segment.current_stack_offset();
        segment.op0(|| NoTraits(1));
        segment.op0(|| NoTraits(2));
        segment.make_array(2, start)?;

        let array: DynamicArray = segment.call0()?;
        assert_eq!(
            array.try_into_vec::<NoTraits>()?,
            vec![NoTraits(1), NoTraits(2)]
        );
        Ok(())
    }

    #[test]
    fn make_array_preserves_over_aligned_elements() -> anyhow::Result<()> {
        let mut segment = DynSegment::new::<()>();
        let start = segment.current_stack_offset();
        segment.op0(|| OverAligned(1));
        segment.op0(|| OverAligned(2));
        segment.make_array(2, start)?;

        let array: DynamicArray = segment.call0()?;
        let values = array.try_as_slice::<OverAligned>()?;
        assert!(
            (values.as_ptr() as usize).is_multiple_of(align_of::<OverAligned>()),
            "collected storage must satisfy the element's alignment"
        );
        assert_eq!(values, &[OverAligned(1), OverAligned(2)]);
        Ok(())
    }

    #[test]
    fn make_array_drops_each_collected_element_exactly_once() -> anyhow::Result<()> {
        let drop_count = Arc::new(AtomicUsize::new(0));
        let mut segment = DynSegment::new::<()>();
        let start = segment.current_stack_offset();
        for _ in 0..3 {
            let tracker = DropCounter(drop_count.clone());
            segment.op0(move || tracker.clone());
        }
        segment.make_array(3, start)?;

        let array: DynamicArray = segment.call0()?;
        assert_eq!(array.len(), 3);
        assert_eq!(drop_count.load(Ordering::SeqCst), 0);

        drop(array);
        assert_eq!(drop_count.load(Ordering::SeqCst), 3);
        Ok(())
    }

    #[test]
    fn make_array_rejection_leaves_the_segment_usable() -> anyhow::Result<()> {
        let drop_count = Arc::new(AtomicUsize::new(0));
        let tracker = DropCounter(drop_count.clone());
        let mut segment = DynSegment::new::<()>();
        let start = segment.current_stack_offset();
        segment.op0(move || tracker.clone());
        segment.op0(|| 1f64);
        let offset_before = segment.current_stack_offset();

        assert!(segment.make_array(2, start).is_err());
        assert_eq!(
            segment.current_stack_offset(),
            offset_before,
            "a rejected make_array must not change the parse-time stack"
        );

        segment.op2(|counted: DropCounter, value: f64| {
            drop(counted);
            value
        })?;
        assert_eq!(segment.call0::<f64>()?, 1.0);
        assert_eq!(
            drop_count.load(Ordering::SeqCst),
            1,
            "the one produced element must be dropped exactly once"
        );
        Ok(())
    }

    #[test]
    fn make_array_unwinds_already_produced_elements_when_an_element_fails() -> anyhow::Result<()> {
        let drop_count = Arc::new(AtomicUsize::new(0));
        let mut segment = DynSegment::new::<()>();
        let start = segment.current_stack_offset();
        for _ in 0..2 {
            let tracker = DropCounter(drop_count.clone());
            segment.op0(move || tracker.clone());
        }
        segment.op0r(|| -> Result<DropCounter> { Err(anyhow!("element failed")) });
        segment.make_array(3, start)?;

        let result = segment.call0::<DynamicArray>();
        assert!(result.is_err());
        assert_eq!(
            drop_count.load(Ordering::SeqCst),
            2,
            "each already-produced element is dropped exactly once"
        );
        Ok(())
    }

    #[test]
    fn make_array_collects_zero_sized_elements_with_drop_glue() -> anyhow::Result<()> {
        ZST_ELEMENT_DROPS.store(0, Ordering::SeqCst);
        let mut segment = DynSegment::new::<()>();
        let start = segment.current_stack_offset();
        segment.op0(|| ZstWithDrop);
        segment.op0(|| ZstWithDrop);
        segment.op0(|| ZstWithDrop);
        segment.make_array(3, start)?;

        let array: DynamicArray = segment.call0()?;
        assert_eq!(array.len(), 3);
        assert_eq!(ZST_ELEMENT_DROPS.load(Ordering::SeqCst), 0);

        drop(array);
        assert_eq!(ZST_ELEMENT_DROPS.load(Ordering::SeqCst), 3);
        Ok(())
    }

    #[test]
    fn make_array_collects_elements_above_other_stack_values() -> anyhow::Result<()> {
        let mut segment = DynSegment::new::<()>();
        segment.just(9u8);
        let start = segment.current_stack_offset();
        segment.just(1i32);
        segment.just(2i32);
        segment.make_array(2, start)?;
        segment.op2(|head: u8, array: DynamicArray| {
            (
                head,
                array.try_into_vec::<i32>().expect("collected i32 elements"),
            )
        })?;

        let (head, values) = segment.call0::<(u8, Vec<i32>)>()?;
        assert_eq!(head, 9);
        assert_eq!(values, vec![1, 2]);
        Ok(())
    }

    #[test]
    fn make_array_collects_nested_arrays() -> anyhow::Result<()> {
        let mut segment = DynSegment::new::<()>();
        let outer_start = segment.current_stack_offset();
        let first_start = segment.current_stack_offset();
        segment.just(0i32);
        segment.just(1i32);
        segment.make_array(2, first_start)?;
        let second_start = segment.current_stack_offset();
        segment.just(2i32);
        segment.make_array(1, second_start)?;
        segment.make_array(2, outer_start)?;

        let array: DynamicArray = segment.call0()?;
        let inner = array.try_into_vec::<DynamicArray>()?;
        assert_eq!(inner.len(), 2);
        assert_eq!(inner[0].try_as_slice::<i32>()?, &[0, 1]);
        assert_eq!(inner[1].try_as_slice::<i32>()?, &[2]);
        Ok(())
    }

    #[test]
    fn make_array_rejects_mismatched_nested_element_types() -> anyhow::Result<()> {
        let mut segment = DynSegment::new::<()>();
        let outer_start = segment.current_stack_offset();
        let first_start = segment.current_stack_offset();
        segment.just(0i32);
        segment.make_array(1, first_start)?;
        let second_start = segment.current_stack_offset();
        segment.just(1f64);
        segment.make_array(1, second_start)?;

        let error = segment.make_array(2, outer_start).unwrap_err().to_string();

        assert!(error.contains("element 1"), "{error}");
        assert!(
            error.contains("[i32]") && error.contains("[f64]"),
            "{error}"
        );
        Ok(())
    }

    #[test]
    fn make_array_rejects_mismatched_nesting_depth() -> anyhow::Result<()> {
        let mut segment = DynSegment::new::<()>();
        let outer_start = segment.current_stack_offset();
        let flat_start = segment.current_stack_offset();
        segment.just(0i32);
        segment.make_array(1, flat_start)?;
        let nested_start = segment.current_stack_offset();
        let inner_start = segment.current_stack_offset();
        segment.just(1i32);
        segment.make_array(1, inner_start)?;
        segment.make_array(1, nested_start)?;

        let error = segment.make_array(2, outer_start).unwrap_err().to_string();

        assert!(error.contains("element 1"), "{error}");
        assert!(
            error.contains("[i32]") && error.contains("[[i32]]"),
            "{error}"
        );
        Ok(())
    }

    #[test]
    fn make_array_rejects_a_leading_tuple_element() {
        let mut segment = DynSegment::new::<()>();
        let start = segment.current_stack_offset();
        let tuple_start = segment.current_stack_offset();
        segment.just(0i32);
        segment.just(1i32);
        segment.make_tuple(2, tuple_start);

        let error = segment.make_array(1, start).unwrap_err().to_string();

        assert!(error.contains("element 0"), "{error}");
        assert!(error.contains("213"), "{error}");
    }

    #[test]
    fn make_array_rejects_a_trailing_tuple_element() {
        let mut segment = DynSegment::new::<()>();
        let start = segment.current_stack_offset();
        segment.just(0i32);
        let tuple_start = segment.current_stack_offset();
        segment.just(1i32);
        segment.just(2i32);
        segment.make_tuple(2, tuple_start);

        let error = segment.make_array(2, start).unwrap_err().to_string();

        assert!(error.contains("element 1"), "{error}");
        assert!(error.contains("213"), "{error}");
    }

    #[test]
    fn make_array_is_repeatable() -> anyhow::Result<()> {
        let mut segment = DynSegment::new::<()>();
        let start = segment.current_stack_offset();
        segment.just(1i32);
        segment.just(2i32);
        segment.make_array(2, start)?;

        let first: DynamicArray = segment.call_dyn(&[])?;
        let second: DynamicArray = segment.call_dyn(&[])?;

        assert_eq!(first.try_as_slice::<i32>()?, &[1, 2]);
        assert_eq!(second.try_as_slice::<i32>()?, &[1, 2]);
        assert_ne!(
            first.try_as_slice::<i32>()?.as_ptr(),
            second.try_as_slice::<i32>()?.as_ptr(),
            "each execution must own its own allocation"
        );
        Ok(())
    }
}
