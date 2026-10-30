//! Builds and borrows heterogeneous lists without erasing their field types.
//!
//! ```
//! use cel_runtime::{IntoTupleList, List, TypeIdIterator};
//! let list = (String::from("first"), 7u8).into_tuple_list();
//! assert_eq!(list.head(), "first");
//! assert_eq!(*list.tail().head(), 7);
//! assert_eq!(list.len(), 2);
//! assert!(!list.is_empty());
//! assert_eq!(list.reverse().reverse().append(()).len(), 2);
//! assert_eq!(TypeIdIterator::<(String, (u8, ()))>::default().count(), 2);
//! ```

use std::any::TypeId;

/// Represents a finite heterogeneous list with source-ordered heads and tails.
///
/// The `List` trait provides a type-safe way to work with lists where each node
/// contains a value (head) and the remainder of the list (tail). This trait
/// is implemented for both empty lists and non-empty lists.
///
/// Empty lists have length zero; each nonempty list has one more field than
/// its tail. Borrowed heads and tails remain part of the caller-owned list.
///
/// # Examples
/// See the [module tutorial](self).
pub trait List {
    /// The type returned by [`List::empty()`].
    type Empty: EmptyList;
    /// Returns a new empty list that can be used to create a list with the same characteristics.
    ///
    /// # Examples
    /// See [`List`].
    fn empty() -> Self::Empty;

    /// The type of head.
    type Head: 'static;
    /// Returns a reference to the head.
    ///
    /// - Precondition: The list is nonempty.
    ///
    /// # Examples
    /// See [`List`].
    fn head(&self) -> &Self::Head;

    /// The type of the rest of the list.
    type Tail: List;
    /// Returns a reference to the rest of the list.
    ///
    /// - Precondition: The list is nonempty.
    ///
    /// # Examples
    /// See [`List`].
    fn tail(&self) -> &Self::Tail;

    /// Returns true if the list is empty.
    ///
    /// # Examples
    /// See [`List`].
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The length of the list, computed at compile time.
    const LENGTH: usize = Self::Tail::LENGTH + 1;
    /// Returns the length of the list.
    ///
    /// # Examples
    /// See [`List`].
    fn len(&self) -> usize {
        Self::LENGTH
    }

    /// The type of the list after pushing a new value.
    type Push<U: 'static>: List;
    /// Pushes a new value onto the front of the list, returning a new list.
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::List;
    /// assert_eq!(().push("first").push(7u8), (7, ("first", ())));
    /// ```
    fn push<U: 'static>(self, item: U) -> Self::Push<U>;

    // ---

    /// The style of `List` appended to `Self`
    type Append<U: List>: List;

    /// Moves `other` after this list's source-ordered fields.
    ///
    /// - Complexity: O(n) field moves for n fields in this list.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::*;
    ///
    /// assert_eq!(
    ///     (1, (2, (3, ()))).append((4, (5, (6, ())))),
    ///     (1, (2, (3, (4, (5, (6, ()))))))
    /// );
    /// ```
    fn append<U: List>(self, other: U) -> Self::Append<U>;

    /// The resulting list when reversing `Self` onto `U`.
    type ReverseOnto<U: List>: List;
    /// Moves this list's reversed fields before `other`.
    ///
    /// - Complexity: O(n) field moves for n fields in this list.
    ///
    /// # Examples
    /// See [`List::reverse`].
    fn reverse_onto<U: List>(self, other: U) -> Self::ReverseOnto<U>;

    /// Moves this list's fields into opposite source order.
    ///
    /// - Complexity: O(n) field moves for n fields.
    ///
    /// # Examples
    /// See [`List`].
    fn reverse(self) -> Self::ReverseOnto<Self::Empty>
    where
        Self: Sized,
    {
        self.reverse_onto(Self::empty())
    }
}

/// Convenience alias for reversing a `List` onto its empty counterpart.
///
/// # Examples
/// ```
/// use cel_runtime::{List, ReverseList};
/// let reversed: ReverseList<(u8, (String, ()))> = (7u8, (String::from("first"), ())).reverse();
/// assert_eq!(reversed.head(), "first");
/// ```
pub type ReverseList<T> = <T as List>::ReverseOnto<<T as List>::Empty>;

// Iterate a list (not recurse) to implement equal against an iterator.
/// Property generator for iterating over type-level lists.
///
/// # Examples
/// See [`ListTypeIterator`], which uses `TypeId` as its property generator.
pub trait ListTypeProperty {
    /// The per-element property type produced during iteration.
    type Output;
    /// Computes the property for the current list head in `R`.
    ///
    /// # Examples
    /// See [`ListTypeIterator`].
    fn property<R: List>() -> Self::Output;
}

impl ListTypeProperty for TypeId {
    type Output = Self;
    /// Returns the current logical head's runtime type identity.
    fn property<R: List>() -> Self::Output {
        TypeId::of::<R::Head>()
    }
}

/// Iterator over type-level `List` values producing properties via `P`.
///
/// Iteration visits each logical head once and remains exhausted after its tail.
///
/// # Examples
/// ```
/// use cel_runtime::{ListTypeIterator, TypeIdIterator};
/// use std::any::TypeId;
/// let mut iter = ListTypeIterator::<(u8, (String, ())), TypeId>::new();
/// assert_eq!(iter.next(), Some(TypeId::of::<u8>()));
/// assert_eq!(iter.next(), Some(TypeId::of::<String>()));
/// assert_eq!(iter.next(), None);
/// assert_eq!(TypeIdIterator::<()>::default().next(), None);
/// ```
pub struct ListTypeIterator<T: List, P: ListTypeProperty> {
    /// Dispatches the next logical node, or the persistent empty-tail sentinel.
    pub(crate) advance: fn(&mut Self) -> Option<P::Output>,
}

/// Advance function provider used by [`ListTypeIterator`].
///
/// Implementations advance to the next tail before returning a head property,
/// and return `None` at an empty tail.
///
/// # Examples
/// See [`ListTypeIterator`].
pub trait ListTypeIteratorAdvance<P: ListTypeProperty>: List + Sized {
    /// Advances the iterator and returns the next property, or `None` at exhaustion.
    ///
    /// - Complexity: The property generator's complexity for a nonempty head.
    ///
    /// # Examples
    /// See [`ListTypeIterator`].
    fn advancer<R: List>(iter: &mut ListTypeIterator<R, P>) -> Option<P::Output>;
}

impl<T: ListTypeIteratorAdvance<P> + 'static, P: ListTypeProperty> ListTypeIterator<T, P> {
    /// Constructs an iterator positioned at the first logical head of `T`.
    ///
    /// # Examples
    /// See [`ListTypeIterator`].
    #[must_use]
    pub fn new() -> Self {
        ListTypeIterator {
            advance: T::advancer::<T>,
        }
    }
}

impl<T: ListTypeIteratorAdvance<P> + 'static, P: ListTypeProperty> Default
    for ListTypeIterator<T, P>
{
    /// Constructs an iterator positioned at the first logical head.
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ListTypeIteratorAdvance<P> + 'static, P: ListTypeProperty> Iterator
    for ListTypeIterator<T, P>
{
    type Item = P::Output;
    /// Returns the next source-ordered property, or `None` after exhaustion.
    ///
    /// - Complexity: The property generator's complexity.
    fn next(&mut self) -> Option<Self::Item> {
        (self.advance)(self)
    }
}

/// Iterator that yields the [`TypeId`] of each head in the `List`.
///
/// # Examples
/// See [`ListTypeIterator`].
pub type TypeIdIterator<T> = ListTypeIterator<T, TypeId>;

/// Marker type representing the head of an empty list.
///
/// No empty-list operation constructs or borrows a head value.
///
/// # Examples
/// ```
/// use cel_runtime::{List, Undefined};
/// fn empty_head_type(_: Option<&<() as List>::Head>) {}
/// empty_head_type(None::<&Undefined>);
/// ```
pub struct Undefined;

/// Trait implemented by empty lists to support list construction.
///
/// # Examples
/// ```
/// use cel_runtime::{EmptyList, List};
/// let list = <()>::root_empty().push_first(String::from("owned"));
/// assert_eq!(list.head(), "owned");
/// ```
pub trait EmptyList {
    /// The result of pushing the first element onto an empty list.
    type PushFirst<U: 'static>: List;
    /// Moves the first element into a singleton list.
    ///
    /// # Examples
    /// See [`EmptyList`].
    fn push_first<U: 'static>(self, item: U) -> Self::PushFirst<U>;

    /// The canonical empty list type for this family.
    type RootEmpty: EmptyList;
    /// Returns a new canonical empty list.
    ///
    /// # Examples
    /// See [`EmptyList`].
    fn root_empty() -> Self::RootEmpty;
}

/// A blanket implementation for EmptyList.
impl<T: EmptyList> List for T {
    type Empty = T::RootEmpty;
    /// Constructs this family's canonical empty root.
    fn empty() -> Self::Empty {
        T::root_empty()
    }

    type Head = Undefined;
    /// Borrows a logical head.
    ///
    /// - Precondition: The list is nonempty.
    fn head(&self) -> &Self::Head {
        debug_assert!(!self.is_empty());
        unreachable!("EmptyList has no head")
    }

    type Tail = T; // Satisfy the List trait
    /// Borrows the remaining logical fields.
    ///
    /// - Precondition: The list is nonempty.
    fn tail(&self) -> &Self::Tail {
        debug_assert!(!self.is_empty());
        unreachable!("EmptyList has no tail")
    }

    type Push<U: 'static> = T::PushFirst<U>;
    /// Moves the first logical field into this empty list.
    fn push<U>(self, item: U) -> Self::Push<U> {
        self.push_first(item)
    }

    const LENGTH: usize = 0;

    type Append<U: List> = U;
    /// Returns `other` because this list contributes no logical fields.
    fn append<U: List>(self, other: U) -> Self::Append<U> {
        other
    }

    type ReverseOnto<U: List> = U;
    /// Returns `other` because reversing the empty list adds no fields.
    fn reverse_onto<U: List>(self, other: U) -> Self::ReverseOnto<U> {
        other
    }
}

/// Indexing for lists using typenum-based indices and ranges.
///
/// The index type establishes that the selected field or range exists.
///
/// # Examples
/// ```
/// use cel_runtime::{IntoTupleList, ListIndex};
/// let list = ("first", 7u8).into_tuple_list();
/// assert_eq!(*list.index(typenum::U1::new()), 7);
/// ```
pub trait ListIndex<Idx: ?Sized> {
    /// The output reference type when indexing with `Idx`.
    type Output;
    /// Borrows the element or sublist selected by `index`.
    ///
    /// - Complexity: Representation-dependent; recursive suffix traversal takes
    ///   O(i) steps for source index i.
    ///
    /// # Examples
    /// See [`ListIndex`].
    fn index(&self, index: Idx) -> &Self::Output;
}

/// Names the field or sublist type selected by index `N` in list `L`.
///
/// # Examples
/// ```
/// use cel_runtime::Item;
/// let first: Item<(String, (u8, ())), typenum::U0> = String::from("owned");
/// assert_eq!(first, "owned");
/// ```
pub type Item<L, N> = <L as ListIndex<N>>::Output;

/// Converts to a type-level `List` without consuming the source.
///
/// # Examples
/// A converter retains its source ownership and states whether its fields are
/// borrowed or independently copied; see [`IntoList`] for the consuming bridge.
pub trait ToList {
    /// The resulting `List` type using `T` as the empty list type family.
    type ToList<T: EmptyList>: List;
    /// Converts to a `List` using `T` as the empty list type family.
    ///
    /// - Complexity: The converter's field borrowing or copying costs.
    ///
    /// # Examples
    /// See [`ToList`].
    fn to_list<T: EmptyList>(&self) -> Self::ToList<T>;
}

/// Moves source fields into a type-level `List`.
///
/// # Examples
/// ```
/// use cel_runtime::{IntoList, List};
/// let list = (String::from("first"), 7u8).into_list::<()>();
/// assert_eq!(list.head(), "first");
/// ```
pub trait IntoList {
    /// The resulting `List` type using `T` as the empty list type family.
    type Output<T: EmptyList>: List;
    /// Moves fields into a `List` using `T` as the empty list type family.
    ///
    /// - Complexity: O(n) field moves for the native tuple bridges, without allocation.
    ///
    /// # Examples
    /// See [`IntoList`].
    fn into_list<T: EmptyList>(self) -> Self::Output<T>;
}

impl IntoList for () {
    type Output<T: EmptyList> = <T as List>::Empty;
    /// Constructs the requested family's empty root.
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        T::empty()
    }
}

impl<A: 'static> IntoList for (A,) {
    type Output<T: EmptyList> = <<T as List>::Empty as List>::Push<A>;
    /// Moves the singleton source field into the requested list family.
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        ().into_list::<T>().push(self.0)
    }
}

impl<A: 'static, B: 'static> IntoList for (A, B) {
    type Output<T: EmptyList> = <<(B,) as IntoList>::Output<T> as List>::Push<A>;
    /// Moves both source fields into the requested list family without cloning.
    ///
    /// - Complexity: O(n) field moves for n tuple fields.
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        (self.1,).into_list::<T>().push(self.0)
    }
}

impl<A: 'static, B: 'static, C: 'static> IntoList for (A, B, C) {
    type Output<T: EmptyList> = <<(B, C) as IntoList>::Output<T> as List>::Push<A>;
    /// Moves source fields into the requested list family without cloning.
    ///
    /// - Complexity: O(n) field moves for n tuple fields.
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        (self.1, self.2).into_list::<T>().push(self.0)
    }
}
impl<A: 'static, B: 'static, C: 'static, D: 'static> IntoList for (A, B, C, D) {
    type Output<T: EmptyList> = <<(B, C, D) as IntoList>::Output<T> as List>::Push<A>;
    /// Moves source fields into the requested list family without cloning.
    ///
    /// - Complexity: O(n) field moves for n tuple fields.
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        (self.1, self.2, self.3).into_list::<T>().push(self.0)
    }
}

impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static> IntoList for (A, B, C, D, E) {
    type Output<T: EmptyList> = <<(B, C, D, E) as IntoList>::Output<T> as List>::Push<A>;
    /// Moves source fields into the requested list family without cloning.
    ///
    /// - Complexity: O(n) field moves for n tuple fields.
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        (self.1, self.2, self.3, self.4)
            .into_list::<T>()
            .push(self.0)
    }
}

impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static, F: 'static> IntoList
    for (A, B, C, D, E, F)
{
    type Output<T: EmptyList> = <<(B, C, D, E, F) as IntoList>::Output<T> as List>::Push<A>;
    /// Moves source fields into the requested list family without cloning.
    ///
    /// - Complexity: O(n) field moves for n tuple fields.
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        (self.1, self.2, self.3, self.4, self.5)
            .into_list::<T>()
            .push(self.0)
    }
}

impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static, F: 'static, G: 'static> IntoList
    for (A, B, C, D, E, F, G)
{
    type Output<T: EmptyList> = <<(B, C, D, E, F, G) as IntoList>::Output<T> as List>::Push<A>;
    /// Moves source fields into the requested list family without cloning.
    ///
    /// - Complexity: O(n) field moves for n tuple fields.
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        (self.1, self.2, self.3, self.4, self.5, self.6)
            .into_list::<T>()
            .push(self.0)
    }
}

impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static, F: 'static, G: 'static, H: 'static>
    IntoList for (A, B, C, D, E, F, G, H)
{
    type Output<T: EmptyList> = <<(B, C, D, E, F, G, H) as IntoList>::Output<T> as List>::Push<A>;
    /// Moves source fields into the requested list family without cloning.
    ///
    /// - Complexity: O(n) field moves for n tuple fields.
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        (self.1, self.2, self.3, self.4, self.5, self.6, self.7)
            .into_list::<T>()
            .push(self.0)
    }
}

impl<
    A: 'static,
    B: 'static,
    C: 'static,
    D: 'static,
    E: 'static,
    F: 'static,
    G: 'static,
    H: 'static,
    I: 'static,
> IntoList for (A, B, C, D, E, F, G, H, I)
{
    type Output<T: EmptyList> =
        <<(B, C, D, E, F, G, H, I) as IntoList>::Output<T> as List>::Push<A>;
    /// Moves source fields into the requested list family without cloning.
    ///
    /// - Complexity: O(n) field moves for n tuple fields.
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        (
            self.1, self.2, self.3, self.4, self.5, self.6, self.7, self.8,
        )
            .into_list::<T>()
            .push(self.0)
    }
}

impl<
    A: 'static,
    B: 'static,
    C: 'static,
    D: 'static,
    E: 'static,
    F: 'static,
    G: 'static,
    H: 'static,
    I: 'static,
    J: 'static,
> IntoList for (A, B, C, D, E, F, G, H, I, J)
{
    type Output<T: EmptyList> =
        <<(B, C, D, E, F, G, H, I, J) as IntoList>::Output<T> as List>::Push<A>;
    /// Moves source fields into the requested list family without cloning.
    ///
    /// - Complexity: O(n) field moves for n tuple fields.
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        (
            self.1, self.2, self.3, self.4, self.5, self.6, self.7, self.8, self.9,
        )
            .into_list::<T>()
            .push(self.0)
    }
}

impl<
    A: 'static,
    B: 'static,
    C: 'static,
    D: 'static,
    E: 'static,
    F: 'static,
    G: 'static,
    H: 'static,
    I: 'static,
    J: 'static,
    K: 'static,
> IntoList for (A, B, C, D, E, F, G, H, I, J, K)
{
    type Output<T: EmptyList> =
        <<(B, C, D, E, F, G, H, I, J, K) as IntoList>::Output<T> as List>::Push<A>;
    /// Moves source fields into the requested list family without cloning.
    ///
    /// - Complexity: O(n) field moves for n tuple fields.
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        (
            self.1, self.2, self.3, self.4, self.5, self.6, self.7, self.8, self.9, self.10,
        )
            .into_list::<T>()
            .push(self.0)
    }
}

impl<
    A: 'static,
    B: 'static,
    C: 'static,
    D: 'static,
    E: 'static,
    F: 'static,
    G: 'static,
    H: 'static,
    I: 'static,
    J: 'static,
    K: 'static,
    L: 'static,
> IntoList for (A, B, C, D, E, F, G, H, I, J, K, L)
{
    type Output<T: EmptyList> =
        <<(B, C, D, E, F, G, H, I, J, K, L) as IntoList>::Output<T> as List>::Push<A>;
    /// Moves source fields into the requested list family without cloning.
    ///
    /// - Complexity: O(n) field moves for n tuple fields.
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        (
            self.1, self.2, self.3, self.4, self.5, self.6, self.7, self.8, self.9, self.10,
            self.11,
        )
            .into_list::<T>()
            .push(self.0)
    }
}
