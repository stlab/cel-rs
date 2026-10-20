//! Owns an erased whole value together with its frozen ownership descriptor.
//!
//! Checked host conversion validates the complete representation before moving:
//!
//! ```
//! use cel_runtime::owned_value::OwnedValue;
//! use cel_runtime::value_type::{ArrayOf, Leaf};
//! use cel_runtime::Array;
//! let array = Array::from_vec(vec![String::from("owned")]);
//! let pointer = array.as_slice().as_ptr();
//! let owner = OwnedValue::new::<ArrayOf<Leaf<String>>>(array);
//! let array = owner.try_into_typed::<ArrayOf<Leaf<String>>>().unwrap();
//! assert_eq!(array.as_slice().as_ptr(), pointer);
//! ```

use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use std::mem::MaybeUninit;
use std::ptr::{self, NonNull};

use crate::array::RawArray;
use crate::value_type::{AssociatedType, ValueKind, ValueRepr, ValueType};

/// Owns one aligned allocation, including an aligned dangling pointer for ZSTs.
///
/// This owner deallocates storage but does not destroy its contents.
pub(crate) struct Allocation {
    pub(crate) pointer: NonNull<u8>,
    pub(crate) layout: Layout,
}

impl Allocation {
    /// Allocates uninitialized storage or an aligned dangling pointer for a ZST.
    pub(crate) fn new(layout: Layout) -> Self {
        let pointer = if layout.size() == 0 {
            NonNull::new(ptr::without_provenance_mut(layout.align())).unwrap()
        } else {
            // SAFETY: Layout is nonzero and valid; allocation failure aborts.
            NonNull::new(unsafe { alloc(layout) }).unwrap_or_else(|| handle_alloc_error(layout))
        };
        Self { pointer, layout }
    }
}

impl Drop for Allocation {
    /// Releases nonzero storage after its live contents have been destroyed.
    fn drop(&mut self) {
        if self.layout.size() != 0 {
            // SAFETY: This owner holds the unique allocation with this exact layout.
            unsafe { dealloc(self.pointer.as_ptr(), self.layout) };
        }
    }
}

/// Owns exactly one live erased value and its complete immutable descriptor.
///
/// No clone, equality, formatting, or thread-safety bounds are imposed on the
/// stored value. Consuming errors destroy the original value exactly once;
/// borrowed capability errors leave the original value unchanged.
///
/// - Complexity: Destruction visits every contained value in reverse logical
///   field/element order, using O(d) call space for nesting depth d.
///
/// # Examples
///
/// ```
/// use cel_runtime::{owned_value::OwnedValue, value_type::Leaf};
/// let owner = OwnedValue::new::<Leaf<String>>(String::from("hello"));
/// assert_eq!(owner.try_into_typed::<Leaf<String>>().unwrap(), "hello");
/// ```
pub struct OwnedValue {
    allocation: Allocation,
    value_type: ValueType,
    live: bool,
}

/// Binds recursive debug capabilities independently of a value's ownership.
///
/// The formatter never clones or consumes a value. Each checked formatting
/// boundary validates the complete shape before invoking any leaf hook.
///
/// # Examples
/// ```
/// use cel_runtime::{Leaf, OwnedValue, ValueType};
/// let value = OwnedValue::new::<Leaf<i32>>(7)
///     .with_value_type(ValueType::leaf::<i32>().with_debug::<Leaf<i32>>().unwrap()).unwrap();
/// let formatter = value.prepare_debug().unwrap();
/// assert_eq!(formatter.format(&value).unwrap(), "7");
/// ```
#[derive(Clone)]
pub struct ValueFormatter {
    value_type: ValueType,
}

impl ValueFormatter {
    /// Formats a matching live value using this previously prepared debug capability.
    ///
    /// # Errors
    /// Rejects a complete shape mismatch before invoking any formatter.
    /// - Complexity: O(n + v + s) for n type nodes, v values and s output bytes,
    ///   excluding leaf formatting costs.
    ///
    /// # Examples
    /// See [`OwnedValue::prepare_debug`].
    pub fn format(&self, value: &OwnedValue) -> anyhow::Result<String> {
        anyhow::ensure!(
            self.value_type.same_shape(value.value_type()),
            "formatter value shape mismatch"
        );
        let mut output = String::new();
        // SAFETY: Preparation checks every debug hook; the boundary checks full shape.
        unsafe { debug_raw(&self.value_type, value.as_ptr(), &mut output) };
        Ok(output)
    }
}

impl OwnedValue {
    /// Prepares an independent reusable formatter without consuming or formatting this value.
    ///
    /// # Errors
    /// Rejects absent debug capability at any recursive node.
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::{Leaf, OwnedValue, ValueType};
    /// let value = OwnedValue::new::<Leaf<i32>>(7)
    ///     .with_value_type(ValueType::leaf::<i32>().with_debug::<Leaf<i32>>().unwrap()).unwrap();
    /// assert_eq!(value.prepare_debug().unwrap().format(&value).unwrap(), "7");
    /// ```
    pub fn prepare_debug(&self) -> anyhow::Result<ValueFormatter> {
        anyhow::ensure!(self.value_type.can_debug(), "debug capability unavailable");
        Ok(ValueFormatter {
            value_type: self.value_type.clone(),
        })
    }

    /// Moves one bound array element and destroys every unselected element.
    ///
    /// # Safety
    /// This owner must have the canonical array descriptor bound by the caller.
    /// # Errors
    /// Returns an error for an out-of-bounds index, destroying the consumed array.
    /// - Complexity: O(n) sibling destruction for n array elements plus one raw move.
    pub(crate) unsafe fn take_array_element(mut self, index: usize) -> anyhow::Result<Self> {
        let ValueKind::Array(element) = self.value_type.kind() else {
            unreachable!("validated canonical array projection")
        };
        let raw = unsafe { &*self.as_ptr().cast::<RawArray>() };
        anyhow::ensure!(
            index < raw.len,
            "array index {index} is out of bounds for length {}",
            raw.len
        );
        let allocation = Allocation::new(element.layout());
        unsafe {
            std::ptr::copy_nonoverlapping(
                raw.ptr.as_ptr().cast::<u8>().add(index * element.size()),
                allocation.pointer.as_ptr(),
                element.size(),
            );
        }
        let result = unsafe { Self::from_allocation(allocation, element.clone()) };
        let _payload = Allocation {
            pointer: raw.ptr.cast(),
            layout: array_layout(element, raw.capacity),
        };
        /// Destroys the still-owned array elements during normal and unwinding cleanup.
        struct Siblings<'a> {
            base: *mut u8,
            element: &'a ValueType,
            remaining: usize,
            skip: usize,
        }
        impl Siblings<'_> {
            /// Destroys unselected elements in reverse index order.
            fn destroy(&mut self) {
                while self.remaining != 0 {
                    self.remaining -= 1;
                    if self.remaining != self.skip {
                        unsafe {
                            drop_raw(
                                self.element,
                                self.base.add(self.remaining * self.element.size()),
                            )
                        };
                    }
                }
            }
        }
        impl Drop for Siblings<'_> {
            /// Continues remaining element destruction after a sibling panics.
            fn drop(&mut self) {
                self.destroy();
            }
        }
        let mut siblings = Siblings {
            base: raw.ptr.as_ptr().cast(),
            element,
            remaining: raw.len,
            skip: index,
        };
        self.live = false;
        siblings.destroy();
        Ok(result)
    }

    /// Splits a canonical tuple into independent source-ordered field owners.
    ///
    /// Nested arrays retain their payload allocations; no leaf is cloned.
    /// # Errors
    /// Rejects a nontuple owner without transferring any field ownership.
    /// - Complexity: O(n + s) field allocations and raw moves for n fields and s bytes.
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::{OwnedValue, Cons, Leaf, Nil, IntoCStackList};
    /// let tuple = OwnedValue::new::<Cons<Leaf<i32>, Nil>>((7,).into_c_stack_list());
    /// let fields = tuple.into_fields().unwrap();
    /// assert_eq!(fields.len(), 1);
    /// ```
    pub fn into_fields(mut self) -> anyhow::Result<Vec<Self>> {
        let crate::value_type::ValueKind::Tuple(fields) = self.value_type.kind() else {
            anyhow::bail!("canonical tuple owner required");
        };
        let destinations: Vec<_> = fields
            .iter()
            .map(|field| {
                (
                    Allocation::new(field.value_type.layout()),
                    field.offset,
                    field.value_type.clone(),
                )
            })
            .collect();
        let mut result = Vec::with_capacity(destinations.len());
        let source = self.as_ptr();
        // Every allocation precedes the first transfer; no callback, allocation,
        // or fallible operation follows disarming this enclosing owner.
        self.live = false;
        for (allocation, offset, value_type) in destinations {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    source.add(offset),
                    allocation.pointer.as_ptr(),
                    value_type.size(),
                );
                result.push(Self::from_allocation(allocation, value_type));
            }
        }
        Ok(result)
    }

    /// Moves a previously validated tuple field and destroys the remaining fields.
    ///
    /// # Safety
    /// This owner must be a canonical tuple and `index` must be in range.
    /// The caller must bind the returned field's full descriptor before execution.
    /// - Complexity: O(n) destruction calls for n fields plus one field move.
    pub(crate) unsafe fn take_field(mut self, index: usize) -> Self {
        let crate::value_type::ValueKind::Tuple(fields) = self.value_type.kind() else {
            unreachable!("validated tuple projection")
        };
        let selected = &fields[index];
        let allocation = Allocation::new(selected.value_type.layout());
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.as_ptr().add(selected.offset),
                allocation.pointer.as_ptr(),
                selected.value_type.size(),
            );
        }
        let result = unsafe { Self::from_allocation(allocation, selected.value_type.clone()) };
        self.live = false;
        /// Continues sibling destruction if one sibling destructor unwinds.
        struct Siblings<'a> {
            pointer: *mut u8,
            fields: &'a [crate::value_type::AssociatedType],
            remaining: usize,
            skip: usize,
        }
        impl Siblings<'_> {
            /// Destroys unselected fields in reverse source order.
            fn destroy(&mut self) {
                while self.remaining != 0 {
                    self.remaining -= 1;
                    if self.remaining != self.skip {
                        let field = &self.fields[self.remaining];
                        unsafe { drop_raw(&field.value_type, self.pointer.add(field.offset)) };
                    }
                }
            }
        }
        impl Drop for Siblings<'_> {
            /// Continues cleanup after the first destructor panic.
            fn drop(&mut self) {
                self.destroy();
            }
        }
        let mut siblings = Siblings {
            pointer: self.allocation.pointer.as_ptr(),
            fields,
            remaining: fields.len(),
            skip: index,
        };
        siblings.destroy();
        result
    }
    /// Adopts an allocation containing one fully initialized canonical value.
    ///
    /// # Safety
    ///
    /// The allocation must uniquely own one live value of exactly `value_type`,
    /// including all nested allocations, with no other destruction responsibility.
    pub(crate) unsafe fn from_allocation(allocation: Allocation, value_type: ValueType) -> Self {
        debug_assert_eq!(allocation.layout, value_type.layout());
        Self {
            allocation,
            value_type,
            live: true,
        }
    }

    /// Takes ownership using the sealed witness's canonical descriptor.
    ///
    /// Moving an array copies only its header and preserves its element allocation.
    ///
    /// - Complexity: O(n) descriptor construction plus one concrete value move,
    ///   for n witness nodes; no contained values are cloned.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::{owned_value::OwnedValue, value_type::Leaf};
    /// let _ = OwnedValue::new::<Leaf<String>>(String::from("owned"));
    /// ```
    pub fn new<R: ValueRepr>(value: R::Value) -> Self {
        let value_type = R::value_type();
        debug_assert_eq!(value_type.layout(), Layout::new::<R::Value>());
        let allocation = Allocation::new(value_type.layout());
        // SAFETY: The sealed witness supplies precisely R::Value's layout.
        unsafe { allocation.pointer.as_ptr().cast::<R::Value>().write(value) };
        Self {
            allocation,
            value_type,
            live: true,
        }
    }

    /// Borrows this whole value's frozen descriptor.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::{owned_value::OwnedValue, value_type::{Leaf, ValueType}};
    /// let owner = OwnedValue::new::<Leaf<i32>>(1);
    /// assert!(owner.value_type().same_shape(&ValueType::leaf::<i32>()));
    /// ```
    pub fn value_type(&self) -> &ValueType {
        &self.value_type
    }

    /// Attaches independently configured capabilities after complete shape validation.
    ///
    /// The live representation and allocation do not change.
    ///
    /// - Complexity: O(n) comparison time and O(d) call space for n type nodes
    ///   and nesting depth d.
    ///
    /// # Errors
    ///
    /// Returns an error if any identity or physical layout differs. On error the
    /// consumed original is destroyed exactly once using its original descriptor;
    /// none of the supplied descriptor's value hooks execute.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::{owned_value::OwnedValue, value_type::{Leaf, ValueType}};
    /// let owner = OwnedValue::new::<Leaf<String>>(String::from("clone me"))
    ///     .with_value_type(ValueType::cloneable_leaf::<String>()).unwrap();
    /// assert!(owner.try_clone().is_ok());
    /// ```
    pub fn with_value_type(mut self, value_type: ValueType) -> anyhow::Result<Self> {
        anyhow::ensure!(
            self.value_type.same_shape(&value_type),
            "value representation mismatch"
        );
        self.value_type = value_type;
        Ok(self)
    }

    /// Moves into the concrete witness type after checking the entire descriptor.
    ///
    /// - Postcondition: Success preserves contained allocations and transfers
    ///   ownership without cloning or running value destructors.
    /// - Complexity: O(n) witness construction and comparison for n type nodes,
    ///   plus a move of the concrete representation.
    ///
    /// # Errors
    ///
    /// Returns an error on any identity or physical layout mismatch. The consumed
    /// original is then destroyed exactly once; no unrelated typed access occurs.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::{owned_value::OwnedValue, value_type::Leaf};
    /// assert_eq!(OwnedValue::new::<Leaf<i32>>(7)
    ///     .try_into_typed::<Leaf<i32>>().unwrap(), 7);
    /// ```
    pub fn try_into_typed<R: ValueRepr>(mut self) -> anyhow::Result<R::Value> {
        self.value_type.validate::<R>()?;
        let mut destination = MaybeUninit::<R::Value>::uninit();
        // SAFETY: Complete validation established the exact destination type.
        unsafe {
            self.move_into(destination.as_mut_ptr().cast());
            Ok(destination.assume_init())
        }
    }

    /// Moves into an exact previously bound concrete representation.
    ///
    /// # Safety
    /// This owner's complete descriptor must match `R`, not merely its carrier.
    /// - Complexity: O(s) for s representation bytes.
    ///
    /// # Examples
    /// Prefer [`Self::try_into_typed`] at checked host boundaries.
    pub unsafe fn into_typed_bound<R: ValueRepr>(mut self) -> R::Value {
        let mut destination = MaybeUninit::<R::Value>::uninit();
        unsafe {
            self.move_into(destination.as_mut_ptr().cast());
            destination.assume_init()
        }
    }

    /// Clones through the frozen plan without consuming or mutating this value.
    ///
    /// If a leaf clone panics, every completed cloned prefix is destroyed and
    /// its storage released; this original remains live.
    ///
    /// - Complexity: O(v) traversal time, O(d) call space, and storage for the
    ///   cloned values, excluding leaf clone costs; v counts contained values.
    ///
    /// # Errors
    ///
    /// Returns an error before cloning anything if cloning is unavailable.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::{owned_value::OwnedValue, value_type::{Leaf, ValueType}};
    /// let original = OwnedValue::new::<Leaf<String>>(String::from("leaf"))
    ///     .with_value_type(ValueType::cloneable_leaf::<String>()).unwrap();
    /// assert_eq!(original.try_clone().unwrap()
    ///     .try_into_typed::<Leaf<String>>().unwrap(), "leaf");
    /// ```
    pub fn try_clone(&self) -> anyhow::Result<Self> {
        anyhow::ensure!(self.value_type.can_clone(), "clone capability unavailable");
        let mut cloned = Self {
            allocation: Allocation::new(self.value_type.layout()),
            value_type: self.value_type.clone(),
            live: false,
        };
        // SAFETY: The frozen plan is cloneable and both allocations match it.
        unsafe {
            clone_raw(
                &self.value_type,
                self.as_ptr(),
                cloned.allocation.pointer.as_ptr(),
            )
        };
        cloned.live = true;
        Ok(cloned)
    }

    /// Compares same-shaped values through this owner's independent equality plan.
    ///
    /// The other owner's capability configuration need not match this one's.
    ///
    /// - Complexity: O(n + v) worst-case traversal time and O(d) call space,
    ///   excluding leaf equality costs, for n type nodes and v contained values.
    ///
    /// # Errors
    ///
    /// Returns an error without invoking equality if shapes differ or this
    /// owner's equality capability is absent. Neither owner is consumed.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::{owned_value::OwnedValue, value_type::{Leaf, ValueType}};
    /// let a = OwnedValue::new::<Leaf<i32>>(1)
    ///     .with_value_type(ValueType::leaf::<i32>().with_equality::<Leaf<i32>>().unwrap())
    ///     .unwrap();
    /// assert!(a.try_eq(&OwnedValue::new::<Leaf<i32>>(1)).unwrap());
    /// ```
    pub fn try_eq(&self, other: &Self) -> anyhow::Result<bool> {
        anyhow::ensure!(
            self.value_type.same_shape(&other.value_type),
            "value representation mismatch"
        );
        anyhow::ensure!(
            self.value_type.can_compare(),
            "equality capability unavailable"
        );
        // SAFETY: Both live values match this prevalidated comparable plan.
        Ok(unsafe { equal_raw(&self.value_type, self.as_ptr(), other.as_ptr()) })
    }

    /// Formats through the independent debug plan without consuming the value.
    ///
    /// Structural arrays use brackets; tuples use parentheses in source order.
    ///
    /// - Complexity: O(v + s) time, O(d) call space, and O(s) output storage,
    ///   excluding leaf formatting costs, for v values and s output bytes.
    ///
    /// # Errors
    ///
    /// Returns an error before formatting anything if debug support is absent.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::{owned_value::OwnedValue, value_type::{Leaf, ValueType}};
    /// let owner = OwnedValue::new::<Leaf<i32>>(7)
    ///     .with_value_type(ValueType::leaf::<i32>().with_debug::<Leaf<i32>>().unwrap())
    ///     .unwrap();
    /// assert_eq!(owner.try_debug().unwrap(), "7");
    /// ```
    pub fn try_debug(&self) -> anyhow::Result<String> {
        anyhow::ensure!(self.value_type.can_debug(), "debug capability unavailable");
        let mut output = String::new();
        // SAFETY: This live value matches its prevalidated debug plan.
        unsafe { debug_raw(&self.value_type, self.as_ptr(), &mut output) };
        Ok(output)
    }

    /// Borrows the raw representation for trusted, already validated execution.
    ///
    /// The pointer remains live only while this owner remains borrowed.
    ///
    /// # Examples
    /// ```
    /// let owner = cel_runtime::OwnedValue::new::<cel_runtime::Leaf<i32>>(1);
    /// assert!(!owner.as_ptr().is_null());
    /// ```
    pub fn as_ptr(&self) -> *const u8 {
        debug_assert!(self.live);
        self.allocation.pointer.as_ptr()
    }

    /// Compares values whose complete identity has already been bound by a host.
    ///
    /// # Safety
    /// Both owners must have exactly the same complete shape, and this owner's
    /// descriptor must support equality.
    /// - Complexity: O(v) for v contained values, excluding leaf comparison costs.
    ///
    /// # Examples
    /// Use [`Self::try_eq`] at checked host boundaries.
    pub unsafe fn eq_bound(&self, other: &Self) -> bool {
        unsafe { equal_raw(&self.value_type, self.as_ptr(), other.as_ptr()) }
    }

    /// Transfers the representation without any runtime identity checks.
    ///
    /// - Postcondition: This owner no longer destroys the transferred value.
    /// - Complexity: O(s) bytes copied for representation size s.
    ///
    /// # Safety
    ///
    /// The caller must have prevalidated the full identity and layout. The
    /// destination must be aligned, disjoint uninitialized storage for exactly
    /// this representation, and must assume unique destruction responsibility.
    pub(crate) unsafe fn move_into(&mut self, destination: *mut u8) {
        debug_assert!(self.live);
        // SAFETY: The caller guarantees disjoint storage of the validated size.
        // Byte copying preserves potentially uninitialized padding without reading it.
        unsafe {
            ptr::copy_nonoverlapping(self.as_ptr(), destination, self.value_type.layout().size());
        }
        self.live = false;
    }
}

impl Drop for OwnedValue {
    /// Destroys the live value before its allocation is automatically released.
    fn drop(&mut self) {
        if self.live {
            self.live = false;
            // SAFETY: This owner uniquely owns one initialized value of this plan.
            unsafe { drop_raw(&self.value_type, self.allocation.pointer.as_ptr()) };
        }
    }
}

/// Describes initialized children without per-element descriptors or allocations.
pub(crate) enum Children<'a> {
    /// Source-ordered tuple fields at canonical offsets.
    Fields(&'a [AssociatedType]),
    /// Homogeneous elements at their concrete stride.
    Elements(&'a ValueType),
}

impl Children<'_> {
    /// Locates one initialized child relative to the aggregate base.
    fn child(&self, index: usize) -> (&ValueType, usize) {
        match self {
            Self::Fields(fields) => (&fields[index].value_type, fields[index].offset),
            Self::Elements(element) => (element, index * element.layout().size()),
        }
    }
}

/// Destroys a live child prefix backwards, including remaining children on unwind.
pub(crate) struct Prefix<'a> {
    pub(crate) base: *mut u8,
    pub(crate) children: Children<'a>,
    pub(crate) initialized: usize,
}

impl Prefix<'_> {
    /// Destroys remaining initialized children in reverse logical order.
    ///
    /// Decrements the live prefix before each destructor so this guard's own
    /// destructor resumes at the next child if the first destructor unwinds.
    ///
    /// - Complexity: O(n) child destructions and O(1) local space.
    ///
    /// # Safety
    ///
    /// The prefix must uniquely own exactly `initialized` live children matching
    /// `children`, at their validated offsets from `base`.
    pub(crate) unsafe fn destroy(&mut self) {
        while self.initialized != 0 {
            self.initialized -= 1;
            let (value_type, offset) = self.children.child(self.initialized);
            unsafe { drop_raw(value_type, self.base.add(offset)) };
        }
    }
}

impl Drop for Prefix<'_> {
    /// Destroys any initialized children not yet transferred or destroyed.
    fn drop(&mut self) {
        // SAFETY: Private construction and prefix updates maintain live ownership.
        unsafe { self.destroy() };
    }
}

/// Returns the exact global-allocation layout for a concrete array capacity.
///
/// - Precondition: Capacity comes from a valid array header of this element type.
fn array_layout(element: &ValueType, capacity: usize) -> Layout {
    Layout::from_size_align(
        element
            .layout()
            .size()
            .checked_mul(capacity)
            .expect("valid array capacity"),
        element.layout().align(),
    )
    .expect("valid array allocation layout")
}

/// Destroys a prevalidated live representation and all its owned allocations.
///
/// - Complexity: O(v) traversal time and O(d) call space for v values and depth d.
///
/// # Safety
///
/// `pointer` must uniquely own one aligned live value matching `value_type`.
pub(crate) unsafe fn drop_raw(value_type: &ValueType, pointer: *mut u8) {
    if let Some(dropper) = value_type.leaf_drop() {
        unsafe { dropper(pointer) };
        return;
    }
    match value_type.kind() {
        ValueKind::Tuple(fields) => {
            let mut prefix = Prefix {
                base: pointer,
                children: Children::Fields(fields),
                initialized: fields.len(),
            };
            unsafe { prefix.destroy() };
        }
        ValueKind::Array(element) => {
            let raw = unsafe { &*pointer.cast::<RawArray>() };
            let _allocation = Allocation {
                pointer: raw.ptr.cast(),
                layout: array_layout(element, raw.capacity),
            };
            let mut prefix = Prefix {
                base: raw.ptr.as_ptr().cast(),
                children: Children::Elements(element),
                initialized: raw.len,
            };
            unsafe { prefix.destroy() };
        }
        // No public constructor supplies callable storage before its carrier exists.
        ValueKind::Leaf(_) | ValueKind::Callable { .. } => unreachable!("missing ownership plan"),
    }
}

/// Clones through a frozen, prevalidated plan with panic-safe prefix ownership.
///
/// - Complexity: O(v) traversal time, O(d) call space, and cloned value storage.
///
/// # Safety
///
/// The plan must support cloning. Source must be live and aligned; destination
/// must be disjoint, aligned uninitialized storage of the same complete shape.
/// A panic leaves no initialized value in the destination.
pub(crate) unsafe fn clone_raw(value_type: &ValueType, source: *const u8, destination: *mut u8) {
    debug_assert!(value_type.can_clone());
    if let Some(cloner) = value_type.clone_hook() {
        unsafe { cloner(source, destination) };
        return;
    }
    match value_type.kind() {
        ValueKind::Tuple(fields) => {
            let mut prefix = Prefix {
                base: destination,
                children: Children::Fields(fields),
                initialized: 0,
            };
            for field in fields {
                unsafe {
                    clone_raw(
                        &field.value_type,
                        source.add(field.offset),
                        destination.add(field.offset),
                    );
                }
                prefix.initialized += 1;
            }
            prefix.initialized = 0;
        }
        ValueKind::Array(element) => {
            let source = unsafe { &*source.cast::<RawArray>() };
            let capacity = if element.layout().size() == 0 {
                usize::MAX
            } else {
                source.len
            };
            let allocation = Allocation::new(array_layout(element, capacity));
            let mut prefix = Prefix {
                base: allocation.pointer.as_ptr(),
                children: Children::Elements(element),
                initialized: 0,
            };
            for index in 0..source.len {
                let offset = index * element.layout().size();
                unsafe {
                    clone_raw(
                        element,
                        source.ptr.as_ptr().cast::<u8>().add(offset),
                        allocation.pointer.as_ptr().add(offset),
                    );
                }
                prefix.initialized += 1;
            }
            unsafe {
                destination.cast::<RawArray>().write(RawArray {
                    ptr: allocation.pointer.cast(),
                    len: source.len,
                    capacity,
                });
            }
            prefix.initialized = 0;
            std::mem::forget(allocation);
        }
        ValueKind::Leaf(_) | ValueKind::Callable { .. } => unreachable!("missing clone plan"),
    }
}

/// Compares live values using a prevalidated complete equality plan.
///
/// - Complexity: O(v) traversal time and O(d) call space, excluding leaf costs.
///
/// # Safety
///
/// Both live values must match the complete comparable descriptor.
unsafe fn equal_raw(value_type: &ValueType, left: *const u8, right: *const u8) -> bool {
    if let Some(equal) = value_type.equality_hook() {
        return unsafe { equal(left, right) };
    }
    match value_type.kind() {
        ValueKind::Tuple(fields) => fields.iter().all(|field| unsafe {
            equal_raw(
                &field.value_type,
                left.add(field.offset),
                right.add(field.offset),
            )
        }),
        ValueKind::Array(element) => {
            let left = unsafe { &*left.cast::<RawArray>() };
            let right = unsafe { &*right.cast::<RawArray>() };
            left.len == right.len
                && (0..left.len).all(|index| unsafe {
                    let offset = index * element.layout().size();
                    equal_raw(
                        element,
                        left.ptr.as_ptr().cast::<u8>().add(offset),
                        right.ptr.as_ptr().cast::<u8>().add(offset),
                    )
                })
        }
        ValueKind::Leaf(_) | ValueKind::Callable { .. } => unreachable!("missing equality plan"),
    }
}

/// Formats live values in source order using a prevalidated debug plan.
///
/// - Complexity: O(v + s) time and O(d + s) space, excluding leaf costs.
///
/// # Safety
///
/// The live value must match the complete debug-capable descriptor.
unsafe fn debug_raw(value_type: &ValueType, pointer: *const u8, output: &mut String) {
    if let Some(debug) = value_type.debug_hook() {
        unsafe { debug(pointer, output) };
        return;
    }
    match value_type.kind() {
        ValueKind::Tuple(fields) => {
            output.push('(');
            for (index, field) in fields.iter().enumerate() {
                if index != 0 {
                    output.push_str(", ");
                }
                unsafe { debug_raw(&field.value_type, pointer.add(field.offset), output) };
            }
            if fields.len() == 1 {
                output.push(',');
            }
            output.push(')');
        }
        ValueKind::Array(element) => {
            output.push('[');
            let raw = unsafe { &*pointer.cast::<RawArray>() };
            for index in 0..raw.len {
                if index != 0 {
                    output.push_str(", ");
                }
                unsafe {
                    debug_raw(
                        element,
                        raw.ptr
                            .as_ptr()
                            .cast::<u8>()
                            .add(index * element.layout().size()),
                        output,
                    )
                };
            }
            output.push(']');
        }
        ValueKind::Leaf(_) | ValueKind::Callable { .. } => unreachable!("missing debug plan"),
    }
}
