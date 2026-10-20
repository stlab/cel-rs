//! Describes complete recursive identities independently of interpreter descriptors.
//!
//! Witnesses name concrete Rust representations without requiring foreign types
//! to implement a trait. Capability adapters are optional and checked separately:
//!
//! ```
//! use cel_runtime::value_type::{ArrayOf, Leaf, ValueRepr, ValueType};
//! let array = ValueType::array(ValueType::cloneable_leaf::<String>());
//! assert!(array.same_shape(&ArrayOf::<Leaf<String>>::value_type()));
//! ```

use std::alloc::Layout;
use std::any::TypeId;
use std::fmt::{Debug, Write};
use std::marker::PhantomData;
use std::sync::Arc;

use crate::array::RawArray;
use crate::c_stack_list::CStackListHeadLimit;
use crate::tuple_value::TupleLayout;
use crate::{Array, CNil, CStackList};

#[cfg(test)]
thread_local! {
    /// Counts descriptor comparisons on this test thread, including recursive calls.
    pub(crate) static SHAPE_COMPARISONS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

/// Seals representation witnesses and limits list tails to canonical lists.
mod sealed {
    use super::{AssociatedType, ValueRepr};

    /// Prevents downstream implementations from inventing unsafe representations.
    pub trait Sealed {}

    /// Supplies source-ordered fields only for canonical list witnesses.
    pub trait List: ValueRepr {
        /// Appends this list's source-ordered fields.
        ///
        /// - Complexity: O(n) time and space for n fields.
        fn fields(fields: &mut Vec<AssociatedType>);
    }
}

/// Names a trusted concrete representation without wrapping its runtime value.
///
/// Only this crate implements the trait; use [`Leaf`] for any foreign host type.
///
/// # Examples
///
/// ```
/// use cel_runtime::value_type::{ArrayOf, Leaf, ValueRepr};
/// use cel_runtime::Array;
/// let _: <ArrayOf<Leaf<i32>> as ValueRepr>::Value = Array::from_vec(vec![1]);
/// ```
pub trait ValueRepr: sealed::Sealed {
    /// The concrete, owned Rust representation.
    type Value: 'static;

    /// Returns the complete identity, layout, and ownership plan.
    ///
    /// Leaves have no optional capabilities by default.
    ///
    /// - Complexity: O(n) time and space for n nodes in the descriptor tree.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::value_type::{Leaf, ValueRepr, ValueType};
    /// assert!(Leaf::<i32>::value_type().same_shape(&ValueType::leaf::<i32>()));
    /// ```
    fn value_type() -> ValueType;
}

/// Names an opaque host leaf of type `T`, including explicitly registered `()`.
///
/// Aggregate-looking host types remain opaque when explicitly named as leaves.
///
/// # Examples
///
/// ```
/// use cel_runtime::value_type::{Leaf, ValueRepr};
/// let _: <Leaf<String> as ValueRepr>::Value = String::new();
/// ```
pub struct Leaf<T>(PhantomData<fn() -> T>);

/// Names a homogeneous array whose elements use witness `E`.
///
/// # Examples
///
/// ```
/// use cel_runtime::value_type::{ArrayOf, Leaf, ValueRepr};
/// use cel_runtime::Array;
/// let _: <ArrayOf<Leaf<u8>> as ValueRepr>::Value = Array::from_vec(vec![1]);
/// ```
pub struct ArrayOf<E>(PhantomData<fn() -> E>);

/// Names canonical source unit, represented by `CNil<()>`, not a host `()`.
///
/// # Examples
///
/// ```
/// use cel_runtime::{IntoCStackList, value_type::{Nil, ValueRepr}};
/// let _: <Nil as ValueRepr>::Value = ().into_c_stack_list();
/// ```
pub struct Nil;

/// Names a source-first tuple head `H` followed by canonical list tail `T`.
///
/// # Examples
///
/// ```
/// use cel_runtime::{IntoCStackList, value_type::{Cons, Leaf, Nil, ValueRepr}};
/// let _: <Cons<Leaf<i32>, Nil> as ValueRepr>::Value = (1,).into_c_stack_list();
/// ```
///
/// Noncanonical tails cannot authorize a representation:
///
/// ```compile_fail
/// use cel_runtime::value_type::{Cons, Leaf, ValueRepr};
/// let _ = Cons::<Leaf<i32>, Leaf<()>>::value_type();
/// ```
pub struct Cons<H, T>(PhantomData<fn() -> (H, T)>);

/// Names a canonical tuple argument list, sealed to [`Nil`] and [`Cons`].
///
/// # Examples
/// ```
/// use cel_runtime::{ArgumentList, Cons, Leaf, Nil};
/// fn list<A: ArgumentList>() {}
/// list::<Cons<Leaf<i32>, Nil>>();
/// ```
pub trait ArgumentList: ValueRepr + sealed::List {}
impl<T: sealed::List> ArgumentList for T {}

/// Names a callable with a complete canonical parameter and result signature.
///
/// The shared carrier always clones, regardless of argument/result capabilities.
/// Equality and default construction are not implied.
///
/// # Examples
/// ```
/// use cel_runtime::{FnOf, Nil, Leaf, ValueRepr, Callable, CNil};
/// let _: <FnOf<Nil, Leaf<i32>> as ValueRepr>::Value =
///     Callable::from_fn(|_: &CNil<()>| Ok(7));
/// ```
///
/// Non-list arguments cannot authorize a callable representation:
/// ```compile_fail
/// use cel_runtime::{FnOf, Leaf, ValueRepr};
/// let _ = FnOf::<Leaf<()>, Leaf<i32>>::value_type();
/// ```
pub struct FnOf<A, R>(PhantomData<fn() -> (A, R)>);

impl<A: ArgumentList, R: ValueRepr> sealed::Sealed for FnOf<A, R> {}
impl<A: ArgumentList, R: ValueRepr> ValueRepr for FnOf<A, R> {
    type Value = crate::Callable<A::Value, R::Value>;

    /// Describes every parameter and result node with canonical carrier ownership.
    ///
    /// - Complexity: O(n) time and storage for n signature nodes.
    fn value_type() -> ValueType {
        let args = A::value_type();
        ValueType::callable(
            args.tuple_elements()
                .expect("sealed canonical argument list")
                .iter()
                .map(|field| field.value_type.clone())
                .collect(),
            R::value_type(),
        )
    }
}

impl<T: 'static> sealed::Sealed for Leaf<T> {}
impl<T: 'static> ValueRepr for Leaf<T> {
    /// Describes the explicitly registered opaque host representation.
    fn value_type() -> ValueType {
        ValueType::leaf::<T>()
    }
    type Value = T;
}

impl<E: ValueRepr> sealed::Sealed for ArrayOf<E> {}
impl<E: ValueRepr> ValueRepr for ArrayOf<E> {
    type Value = Array<E::Value>;
    /// Describes this array and its complete concrete element representation.
    ///
    /// - Complexity: O(n) time and space for n element descriptor nodes.
    fn value_type() -> ValueType {
        ValueType::array(E::value_type())
    }
}

impl sealed::Sealed for Nil {}
impl ValueRepr for Nil {
    type Value = CNil<()>;
    /// Describes canonical source unit with its empty-list ownership plan.
    fn value_type() -> ValueType {
        ValueType::tuple(Vec::new()).expect("empty canonical tuple fits Layout")
    }
}
impl sealed::List for Nil {
    /// Appends no fields for canonical source unit.
    fn fields(_: &mut Vec<AssociatedType>) {}
}

impl<H: ValueRepr, T: sealed::List> sealed::Sealed for Cons<H, T> where T::Value: CStackListHeadLimit
{}
impl<H: ValueRepr, T: sealed::List> ValueRepr for Cons<H, T>
where
    T::Value: CStackListHeadLimit,
{
    type Value = CStackList<H::Value, T::Value>;
    /// Describes source-ordered fields using their canonical recursive layout.
    ///
    /// - Complexity: O(n) time and space for n descriptor nodes.
    fn value_type() -> ValueType {
        let mut fields = Vec::new();
        <Self as sealed::List>::fields(&mut fields);
        let descriptor = ValueType::tuple(fields).expect("concrete Rust list fits Layout");
        debug_assert_eq!(descriptor.layout(), Layout::new::<Self::Value>());
        descriptor
    }
}
impl<H: ValueRepr, T: sealed::List> sealed::List for Cons<H, T>
where
    T::Value: CStackListHeadLimit,
{
    /// Appends the head descriptor followed by the canonical tail's descriptors.
    ///
    /// - Complexity: O(n) time and space for n descriptor nodes.
    fn fields(fields: &mut Vec<AssociatedType>) {
        fields.push(AssociatedType {
            offset: 0,
            value_type: H::value_type(),
        });
        T::fields(fields);
    }
}

/// Identifies a value structurally, independently of aggregate storage markers.
///
/// Callable metadata includes every parameter and the result. Its private
/// ownership plan uses the shared `DynClosure` carrier, exactly matching the
/// transparent [`crate::Callable`] facade authorized by [`FnOf`].
///
/// # Examples
///
/// ```
/// use cel_runtime::value_type::{ValueKind, ValueType};
/// assert!(matches!(ValueType::leaf::<i32>().kind(), ValueKind::Leaf(_)));
/// ```
#[derive(Clone, Debug)]
pub enum ValueKind {
    /// A single explicitly registered opaque Rust type.
    Leaf(TypeId),
    /// Source-ordered fields with canonical physical offsets.
    Tuple(Vec<AssociatedType>),
    /// One complete homogeneous element descriptor.
    Array(ValueType),
    /// Complete semantic callable signature for the shared closure carrier.
    Callable {
        /// Parameter types in source order.
        parameters: Vec<ValueType>,
        /// The complete result type, including source unit or aggregates.
        result: Box<ValueType>,
    },
}

/// Associates a source-ordered tuple field with its canonical physical offset.
///
/// [`ValueType::tuple`] replaces supplied offsets; callers cannot forge layout.
///
/// # Examples
///
/// ```
/// use cel_runtime::value_type::{AssociatedType, ValueType};
/// let field = AssociatedType { offset: 0, value_type: ValueType::leaf::<i32>() };
/// assert!(ValueType::tuple(vec![field]).is_ok());
/// ```
#[derive(Clone, Debug)]
pub struct AssociatedType {
    /// Byte offset relative to the enclosing canonical list.
    pub offset: usize,
    /// Complete field identity and ownership metadata.
    pub value_type: ValueType,
}

/// Clones a live concrete value into aligned, disjoint uninitialized storage.
type CloneHook = unsafe fn(*const u8, *mut u8);
/// Compares two live values of one validated concrete representation.
type EqualityHook = unsafe fn(*const u8, *const u8) -> bool;
/// Formats a live value of one validated concrete representation.
type DebugHook = unsafe fn(*const u8, &mut String);
/// Destroys a live leaf of the registered concrete type exactly once.
type DropHook = unsafe fn(*mut u8);

/// Freezes one descriptor's ownership and independent capability plans.
#[derive(Clone, Debug)]
struct Plan {
    kind: ValueKind,
    layout: Layout,
    leaf_drop: Option<DropHook>,
    clone_hook: Option<CloneHook>,
    equality_hook: Option<EqualityHook>,
    debug_hook: Option<DebugHook>,
    cloneable: bool,
    comparable: bool,
    debuggable: bool,
    host_leaf: Option<crate::host_value_bridge::LeafBridge>,
}

/// Owns immutable recursive identity, canonical layout, and trusted operations.
///
/// Capabilities never change type identity. Cloning a descriptor shares its
/// frozen plan; capability installation returns an independently updated plan.
///
/// # Examples
///
/// ```
/// use cel_runtime::value_type::ValueType;
/// let a = ValueType::array(ValueType::leaf::<i32>());
/// let b = ValueType::array(ValueType::cloneable_leaf::<i32>());
/// assert!(a.same_shape(&b));
/// ```
#[derive(Clone, Debug)]
pub struct ValueType(Arc<Plan>);

impl ValueType {
    /// Sets an opaque leaf's diagnostic name without changing identity or capabilities.
    ///
    /// Structural descriptors retain their recursively defined names.
    ///
    /// # Examples
    /// ```
    /// let ty = cel_runtime::ValueType::leaf::<i32>().with_type_name("Integer");
    /// assert_eq!(ty.type_name(), "Integer");
    /// ```
    pub fn with_type_name(mut self, name: impl Into<std::borrow::Cow<'static, str>>) -> Self {
        if let Some(host) = Arc::make_mut(&mut self.0).host_leaf.as_mut() {
            host.name = name.into();
        }
        self
    }

    /// Rejects erased whole-value owners as opaque canonical elements.
    ///
    /// # Errors
    /// Rejects an `OwnedValue` leaf at any recursive node; owners require a checked whole-value boundary.
    /// - Complexity: O(n) traversal for n descriptor nodes.
    pub(crate) fn validate_canonical(&self) -> anyhow::Result<()> {
        match self.kind() {
            ValueKind::Leaf(id) => anyhow::ensure!(
                *id != TypeId::of::<crate::OwnedValue>(),
                "whole-value owners cannot be opaque canonical elements"
            ),
            ValueKind::Array(element) => element.validate_canonical()?,
            ValueKind::Tuple(fields) => {
                for field in fields {
                    field.value_type.validate_canonical()?;
                }
            }
            ValueKind::Callable { parameters, result } => {
                for parameter in parameters {
                    parameter.validate_canonical()?;
                }
                result.validate_canonical()?;
            }
        }
        Ok(())
    }

    /// Returns whether the complete frozen plan supports borrowed argument cloning.
    ///
    /// # Examples
    /// ```
    /// assert!(cel_runtime::ValueType::cloneable_leaf::<i32>().can_clone_for_argument());
    /// ```
    pub fn can_clone_for_argument(&self) -> bool {
        self.can_clone()
    }
    /// Returns a diagnostic carrier identity; structural casts require witnesses.
    ///
    /// # Examples
    /// ```
    /// assert_eq!(cel_runtime::ValueType::leaf::<i32>().type_id(), std::any::TypeId::of::<i32>());
    /// ```
    pub fn type_id(&self) -> TypeId {
        match self.kind() {
            ValueKind::Leaf(id) => *id,
            ValueKind::Tuple(_) => TypeId::of::<crate::CNil<()>>(),
            ValueKind::Array(_) => TypeId::of::<RawArray>(),
            ValueKind::Callable { .. } => TypeId::of::<crate::DynClosure>(),
        }
    }

    /// Returns a human-readable diagnostic representation name.
    ///
    /// # Examples
    /// ```
    /// assert_eq!(cel_runtime::ValueType::leaf::<i32>().type_name(), "i32");
    /// ```
    pub fn type_name(&self) -> &str {
        match self.kind() {
            ValueKind::Leaf(_) => self.host_leaf().map_or("opaque", |host| host.name()),
            ValueKind::Tuple(_) => "canonical tuple",
            ValueKind::Array(_) => "canonical array",
            ValueKind::Callable { .. } => "callable",
        }
    }

    /// Returns the canonical representation size.
    ///
    /// # Examples
    /// ```
    /// assert_eq!(cel_runtime::ValueType::leaf::<i32>().size(), 4);
    /// ```
    pub fn size(&self) -> usize {
        self.layout().size()
    }

    /// Returns the canonical representation alignment.
    ///
    /// # Examples
    /// ```
    /// assert_eq!(cel_runtime::ValueType::leaf::<u8>().align(), 1);
    /// ```
    pub fn align(&self) -> usize {
        self.layout().align()
    }

    /// Borrows source-ordered canonical tuple fields, including empty source unit.
    ///
    /// # Examples
    /// ```
    /// assert!(cel_runtime::ValueType::leaf::<i32>().tuple_elements().is_none());
    /// ```
    pub fn tuple_elements(&self) -> Option<&[AssociatedType]> {
        match self.kind() {
            ValueKind::Tuple(fields) => Some(fields),
            _ => None,
        }
    }

    /// Borrows the complete canonical array element descriptor.
    ///
    /// # Examples
    /// ```
    /// assert!(cel_runtime::ValueType::array(cel_runtime::ValueType::leaf::<i32>()).array_element().is_some());
    /// ```
    pub fn array_element(&self) -> Option<&ValueType> {
        match self.kind() {
            ValueKind::Array(element) => Some(element),
            _ => None,
        }
    }

    /// Registers a borrowed canonical argument read using this complete clone plan.
    ///
    /// # Errors
    /// Rejects unavailable cloning or an unrepresentable destination frame.
    /// - Complexity: O(n) registration for n existing stack slots.
    ///
    /// # Examples
    /// ```
    /// let mut body = cel_runtime::DynSegment::new::<()>();
    /// cel_runtime::ValueType::cloneable_leaf::<i32>().push_argument(&mut body, 0).unwrap();
    /// ```
    pub fn push_argument(
        &self,
        segment: &mut crate::DynSegment,
        index: usize,
    ) -> anyhow::Result<()> {
        segment.push_argument(index, self.clone())
    }

    /// Checks the borrowed whole-value clone capability.
    ///
    /// # Errors
    /// Rejects a descriptor whose frozen plan cannot clone its representation.
    pub(crate) fn validate_clone(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.can_clone(), "argument clone capability unavailable");
        Ok(())
    }

    /// Describes an opaque host leaf without requiring optional capabilities.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::value_type::ValueType;
    /// assert!(!ValueType::leaf::<i32>().same_shape(&ValueType::leaf::<u32>()));
    /// ```
    pub fn leaf<T: 'static>() -> Self {
        Self(Arc::new(Plan {
            kind: ValueKind::Leaf(TypeId::of::<T>()),
            layout: Layout::new::<T>(),
            leaf_drop: Some(drop_leaf::<T>),
            clone_hook: None,
            equality_hook: None,
            debug_hook: None,
            cloneable: false,
            comparable: false,
            debuggable: false,
            host_leaf: Some(crate::host_value_bridge::LeafBridge::new::<T>()),
        }))
    }

    /// Describes an opaque host leaf with cloning but no other capabilities.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::value_type::ValueType;
    /// let _ = ValueType::cloneable_leaf::<String>();
    /// ```
    pub fn cloneable_leaf<T: Clone + 'static>() -> Self {
        Self::leaf::<T>()
            .with_clone::<Leaf<T>>()
            .expect("identical leaf representation")
    }

    /// Describes a known signature using the existing shared closure carrier.
    ///
    /// Cloning shares the compiled body, independent of parameter/result clone
    /// capabilities. Equality is unavailable; formatting describes the carrier.
    /// This constructor installs ownership directly from the statically known
    /// carrier, not from an opaque leaf witness.
    ///
    /// - Complexity: O(1), transferring ownership of the signature.
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::{ValueType, FnOf, Nil, Leaf, ValueRepr};
    /// assert!(ValueType::callable(vec![], ValueType::leaf::<i32>())
    ///     .same_shape(&FnOf::<Nil, Leaf<i32>>::value_type()));
    /// ```
    pub fn callable(parameters: Vec<ValueType>, result: ValueType) -> Self {
        Self(Arc::new(Plan {
            kind: ValueKind::Callable {
                parameters,
                result: Box::new(result),
            },
            layout: Layout::new::<crate::DynClosure>(),
            leaf_drop: Some(drop_leaf::<crate::DynClosure>),
            clone_hook: Some(clone_leaf::<crate::DynClosure>),
            equality_hook: None,
            debug_hook: Some(debug_leaf::<crate::DynClosure>),
            cloneable: true,
            comparable: false,
            debuggable: true,
            host_leaf: None,
        }))
    }

    /// Describes a homogeneous array using the fixed descriptor-free header.
    ///
    /// Recursive capabilities are present exactly when the element supports them.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::value_type::{ArrayOf, Leaf, ValueRepr, ValueType};
    /// assert!(ValueType::array(ValueType::leaf::<i32>())
    ///     .same_shape(&ArrayOf::<Leaf<i32>>::value_type()));
    /// ```
    pub fn array(element: ValueType) -> Self {
        let cloneable = element.0.cloneable;
        let comparable = element.0.comparable;
        let debuggable = element.0.debuggable;
        Self(Arc::new(Plan {
            kind: ValueKind::Array(element),
            layout: Layout::new::<RawArray>(),
            leaf_drop: None,
            clone_hook: None,
            equality_hook: None,
            debug_hook: None,
            cloneable,
            comparable,
            debuggable,
            host_leaf: None,
        }))
    }

    /// Computes canonical tuple layout from source-ordered fields.
    ///
    /// Supplied offsets are ignored. Empty input describes canonical source unit.
    ///
    /// - Complexity: O(n) time and space for n immediate fields.
    ///
    /// # Errors
    ///
    /// Returns an error if the recursive padded layout exceeds [`Layout`]'s bounds.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::value_type::{Nil, ValueRepr, ValueType};
    /// assert!(ValueType::tuple(vec![]).unwrap().same_shape(&Nil::value_type()));
    /// ```
    pub fn tuple(mut elements: Vec<AssociatedType>) -> anyhow::Result<Self> {
        let fields: Vec<_> = elements
            .iter()
            .map(|field| field.value_type.layout())
            .collect();
        let layout = TupleLayout::from_fields(&fields)?;
        for (field, offset) in elements.iter_mut().zip(layout.offsets) {
            field.offset = offset;
        }
        let cloneable = elements.iter().all(|field| field.value_type.0.cloneable);
        let comparable = elements.iter().all(|field| field.value_type.0.comparable);
        let debuggable = elements.iter().all(|field| field.value_type.0.debuggable);
        Ok(Self(Arc::new(Plan {
            kind: ValueKind::Tuple(elements),
            layout: layout.layout,
            leaf_drop: None,
            clone_hook: None,
            equality_hook: None,
            debug_hook: None,
            cloneable,
            comparable,
            debuggable,
            host_leaf: None,
        })))
    }

    /// Borrows the complete semantic shape without permitting mutation.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::value_type::{Nil, ValueKind, ValueRepr};
    /// assert!(matches!(Nil::value_type().kind(), ValueKind::Tuple(_)));
    /// ```
    pub fn kind(&self) -> &ValueKind {
        &self.0.kind
    }

    /// Returns the validated complete physical layout.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::value_type::ValueType;
    /// assert_eq!(ValueType::leaf::<u64>().layout(), std::alloc::Layout::new::<u64>());
    /// ```
    pub fn layout(&self) -> Layout {
        self.0.layout
    }

    /// Compares complete semantic identities and every physical field layout.
    ///
    /// Capability differences do not affect identity. Equal size and alignment
    /// alone never authorize a cast.
    ///
    /// - Complexity: O(n) time and O(d) call space for n nodes and nesting depth d.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::value_type::ValueType;
    /// assert!(!ValueType::array(ValueType::leaf::<i32>())
    ///     .same_shape(&ValueType::array(ValueType::leaf::<u32>())));
    /// ```
    pub fn same_shape(&self, other: &Self) -> bool {
        #[cfg(test)]
        SHAPE_COMPARISONS.with(|count| count.set(count.get().saturating_add(1)));
        self.layout() == other.layout() && same_kind(self.kind(), other.kind())
    }

    /// Checks concrete identity for a legacy conversion that names only a host leaf.
    ///
    /// Structural values require explicit representation witnesses, regardless
    /// of a shared carrier ID or equal layout. This checks metadata only and
    /// does not change the existing typed segment conversion.
    ///
    /// # Errors
    ///
    /// Returns an error for structural descriptors or a different leaf identity.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::any::TypeId;
    /// use cel_runtime::value_type::ValueType;
    /// assert!(ValueType::leaf::<String>()
    ///     .validate_opaque_leaf(TypeId::of::<String>()).is_ok());
    /// assert!(ValueType::tuple(vec![]).unwrap()
    ///     .validate_opaque_leaf(TypeId::of::<()>()).is_err());
    /// ```
    pub fn validate_opaque_leaf(&self, type_id: TypeId) -> anyhow::Result<()> {
        anyhow::ensure!(
            matches!(self.kind(), ValueKind::Leaf(id) if *id == type_id),
            "value requires a matching opaque leaf or an explicit structural witness"
        );
        Ok(())
    }

    /// Installs a concrete clone adapter after validating the entire witness.
    ///
    /// - Complexity: O(n) time and space for n descriptor nodes.
    ///
    /// # Errors
    ///
    /// Returns an error on any semantic or physical representation mismatch.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::value_type::{Leaf, ValueType};
    /// assert!(ValueType::leaf::<i32>().with_clone::<Leaf<i32>>().is_ok());
    /// ```
    pub fn with_clone<R: ValueRepr>(mut self) -> anyhow::Result<Self>
    where
        R::Value: Clone,
    {
        self.validate::<R>()?;
        let plan = Arc::make_mut(&mut self.0);
        plan.clone_hook = Some(clone_leaf::<R::Value>);
        plan.cloneable = true;
        Ok(self)
    }

    /// Installs concrete equality independently of cloning and formatting.
    ///
    /// - Complexity: O(n) time and space for n descriptor nodes.
    ///
    /// # Errors
    ///
    /// Returns an error on any semantic or physical representation mismatch.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::value_type::{Leaf, ValueType};
    /// assert!(ValueType::leaf::<i32>().with_equality::<Leaf<i32>>().is_ok());
    /// ```
    pub fn with_equality<R: ValueRepr>(mut self) -> anyhow::Result<Self>
    where
        R::Value: PartialEq,
    {
        self.validate::<R>()?;
        let plan = Arc::make_mut(&mut self.0);
        plan.equality_hook = Some(equal_leaf::<R::Value>);
        plan.comparable = true;
        Ok(self)
    }

    /// Installs concrete debug formatting independently of other capabilities.
    ///
    /// - Complexity: O(n) time and space for n descriptor nodes.
    ///
    /// # Errors
    ///
    /// Returns an error on any semantic or physical representation mismatch.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::value_type::{Leaf, ValueType};
    /// assert!(ValueType::leaf::<i32>().with_debug::<Leaf<i32>>().is_ok());
    /// ```
    pub fn with_debug<R: ValueRepr>(mut self) -> anyhow::Result<Self>
    where
        R::Value: Debug,
    {
        self.validate::<R>()?;
        let plan = Arc::make_mut(&mut self.0);
        plan.debug_hook = Some(debug_leaf::<R::Value>);
        plan.debuggable = true;
        Ok(self)
    }

    /// Validates a concrete adapter before any raw operations are installed.
    ///
    /// - Complexity: O(n) time and space for n witness nodes.
    ///
    /// # Errors
    ///
    /// Returns an error if the full descriptor differs from the sealed witness.
    pub(crate) fn validate<R: ValueRepr>(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.same_shape(&R::value_type()),
            "value representation mismatch"
        );
        Ok(())
    }

    /// Returns whether the frozen plan can clone every required child.
    pub(crate) fn can_clone(&self) -> bool {
        self.0.cloneable
    }

    /// Returns whether this complete descriptor supplies value equality.
    ///
    /// # Examples
    /// ```
    /// assert!(!cel_runtime::ValueType::leaf::<i32>().can_compare());
    /// ```
    pub fn can_compare(&self) -> bool {
        self.0.comparable
    }

    /// Returns whether the frozen plan can format every required child.
    pub(crate) fn can_debug(&self) -> bool {
        self.0.debuggable
    }

    /// Returns the optional prevalidated whole-representation clone adapter.
    pub(crate) fn clone_hook(&self) -> Option<CloneHook> {
        self.0.clone_hook
    }

    /// Returns the optional prevalidated whole-representation equality adapter.
    pub(crate) fn equality_hook(&self) -> Option<EqualityHook> {
        self.0.equality_hook
    }

    /// Returns the optional prevalidated whole-representation formatting adapter.
    pub(crate) fn debug_hook(&self) -> Option<DebugHook> {
        self.0.debug_hook
    }

    /// Returns the concrete leaf or callable destructor, absent for tuples and arrays.
    pub(crate) fn leaf_drop(&self) -> Option<DropHook> {
        self.0.leaf_drop
    }

    /// Borrows concrete host adapters installed while the leaf type is known.
    pub(crate) fn host_leaf(&self) -> Option<&crate::host_value_bridge::LeafBridge> {
        self.0.host_leaf.as_ref()
    }
}

/// Compares recursive shapes, including callable argument and result identities.
///
/// - Complexity: O(n) time and O(d) call space for n nodes and depth d.
fn same_kind(left: &ValueKind, right: &ValueKind) -> bool {
    match (left, right) {
        (ValueKind::Leaf(a), ValueKind::Leaf(b)) => a == b,
        (ValueKind::Array(a), ValueKind::Array(b)) => a.same_shape(b),
        (ValueKind::Tuple(a), ValueKind::Tuple(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(a, b)| a.offset == b.offset && a.value_type.same_shape(&b.value_type))
        }
        (
            ValueKind::Callable {
                parameters: a,
                result: ar,
            },
            ValueKind::Callable {
                parameters: b,
                result: br,
            },
        ) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.same_shape(b)) && ar.same_shape(br)
        }
        _ => false,
    }
}

/// Destroys one live leaf.
///
/// - Complexity: The concrete type's destructor costs apply.
///
/// # Safety
///
/// `pointer` must address an exclusively owned, aligned, live `T`.
unsafe fn drop_leaf<T>(pointer: *mut u8) {
    unsafe { pointer.cast::<T>().drop_in_place() };
}

/// Clones a live value into a disjoint, aligned uninitialized destination.
///
/// - Complexity: The concrete type's clone time and allocation costs apply.
///
/// # Safety
///
/// `source` must address a live `T`; `destination` must hold uninitialized `T`
/// storage. On panic no destination value is initialized.
unsafe fn clone_leaf<T: Clone>(source: *const u8, destination: *mut u8) {
    unsafe {
        destination
            .cast::<T>()
            .write((&*source.cast::<T>()).clone())
    };
}

/// Compares two live values without taking ownership.
///
/// - Complexity: The concrete type's equality costs apply.
///
/// # Safety
///
/// Both pointers must address aligned live values of `T` for this call.
unsafe fn equal_leaf<T: PartialEq>(left: *const u8, right: *const u8) -> bool {
    unsafe { *left.cast::<T>() == *right.cast::<T>() }
}

/// Appends one live value's debug representation without taking ownership.
///
/// - Complexity: The concrete formatter's time and output storage costs apply.
///
/// # Safety
///
/// `pointer` must address an aligned live value of `T` for this call.
unsafe fn debug_leaf<T: Debug>(pointer: *const u8, output: &mut String) {
    unsafe { write!(output, "{:?}", *pointer.cast::<T>()) }
        .expect("debug formatter writes to a string");
}

#[cfg(test)]
mod tests {
    use super::{ValueKind, ValueType, same_kind};

    /// Rejects structural descriptors when a legacy host conversion names only a leaf.
    #[test]
    fn complete_factory_legacy_leaf_validation_requires_concrete_identity() {
        use std::any::TypeId;
        assert!(
            ValueType::leaf::<()>()
                .validate_opaque_leaf(TypeId::of::<()>())
                .is_ok()
        );
        assert!(
            ValueType::leaf::<i32>()
                .validate_opaque_leaf(TypeId::of::<u32>())
                .is_err()
        );
        for (structural, marker) in [
            (
                ValueType::tuple(vec![]).unwrap(),
                TypeId::of::<crate::CNil<()>>(),
            ),
            (
                ValueType::array(ValueType::leaf::<i32>()),
                TypeId::of::<crate::Array<i32>>(),
            ),
            (
                ValueType::callable(vec![], ValueType::leaf::<()>()),
                TypeId::of::<crate::DynClosure>(),
            ),
        ] {
            assert!(structural.validate_opaque_leaf(marker).is_err());
        }
    }

    /// Builds semantic callable metadata without constructing an execution carrier.
    fn signature(parameter: ValueType, result: ValueType) -> ValueKind {
        ValueKind::Callable {
            parameters: vec![parameter],
            result: Box::new(result),
        }
    }

    /// Verifies complete callable parameter and result shapes participate in identity.
    #[test]
    fn callable_signatures_compare_every_shape() {
        let signature_a = signature(
            ValueType::array(ValueType::leaf::<i32>()),
            ValueType::tuple(vec![]).unwrap(),
        );
        assert!(same_kind(
            &signature_a,
            &signature(
                ValueType::array(ValueType::leaf::<i32>()),
                ValueType::tuple(vec![]).unwrap(),
            )
        ));
        assert!(!same_kind(
            &signature_a,
            &signature(
                ValueType::array(ValueType::leaf::<u32>()),
                ValueType::tuple(vec![]).unwrap(),
            )
        ));
        assert!(!same_kind(
            &signature_a,
            &signature(
                ValueType::array(ValueType::leaf::<i32>()),
                ValueType::leaf::<()>(),
            )
        ));
        assert!(!same_kind(
            &signature_a,
            &ValueKind::Callable {
                parameters: vec![],
                result: Box::new(ValueType::tuple(vec![]).unwrap()),
            }
        ));
    }
}
