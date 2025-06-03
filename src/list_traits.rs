//! A collection of traits for homegenous lists (cons cells), similar to tuples.

use std::any::TypeId;

/// A trait representing a homogeneous list (cons cell) with a head and tail.
///
/// The `List` trait provides a type-safe way to work with lists where each node
/// contains a value (head) and the remainder of the list (tail). This trait
/// is implemented for both empty lists and non-empty lists.
pub trait List {
    /// The type returned by [`List::empty()`].
    type Empty: EmptyList;
    /// Returns a new empty list that can be used to create a list with the same characteristics.
    fn empty() -> Self::Empty;

    /// The type of head.
    type Head: 'static;
    /// Returns a reference to the head.
    fn head(&self) -> &Self::Head;

    /// The type of the rest of the list.
    type Tail: List;
    /// Returns a reference to the rest of the list.
    fn tail(&self) -> &Self::Tail;

    /// Returns true if the list is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The length of the list, computed at compile time.
    const LENGTH: usize = Self::Tail::LENGTH + 1;
    /// Returns the length of the list.
    fn len(&self) -> usize {
        Self::LENGTH
    }

    /// The type of the list after pushing a new value.
    type Push<U: 'static>: List;
    /// Pushes a new value onto the front of the list, returning a new list.
    fn push<U: 'static>(self, item: U) -> Self::Push<U>;

    // ---

    /// For a `CStackList`, the number of padding bytes between Tail and Head.
    /// This property will be moved to `CStackList` once I figure out a clean way to do it.
    const HEAD_PADDING: usize;

    /// The style of `List` appended to `Self`
    type Append<U: List>: List;

    /// Append `U` as the tail of `Self`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use cel_rs::*;
    ///
    /// assert_eq!(
    ///     (1, (2, (3, ()))).append((4, (5, (6, ())))),
    ///     (1, (2, (3, (4, (5, (6, ()))))))
    /// );
    /// ```
    fn append<U: List>(self, other: U) -> Self::Append<U>;

    type ReverseOnto<U: List>: List;
    fn reverse_onto<U: List>(self, other: U) -> Self::ReverseOnto<U>;

    fn reverse(self) -> Self::ReverseOnto<Self::Empty>
    where
        Self: Sized,
    {
        self.reverse_onto(Self::empty())
    }
}

pub type ReverseList<T> = <T as List>::ReverseOnto<<T as List>::Empty>;

// Iterate a list (not recurse) to implement equal against an iterator.
pub trait ListTypeProperty {
    type Output;
    fn property<R: List>() -> Self::Output;
}

impl ListTypeProperty for TypeId {
    type Output = Self;
    fn property<R: List>() -> Self::Output {
        TypeId::of::<R::Head>()
    }
}

pub struct ListTypeIterator<T: List, P: ListTypeProperty> {
    pub(crate) advance: fn(&mut Self) -> Option<P::Output>,
}

pub trait ListTypeIteratorAdvance<P: ListTypeProperty>: List + Sized {
    fn advancer<R: List>(iter: &mut ListTypeIterator<R, P>) -> Option<P::Output>;
}

impl<T: ListTypeIteratorAdvance<P> + 'static, P: ListTypeProperty> ListTypeIterator<T, P> {
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
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ListTypeIteratorAdvance<P> + 'static, P: ListTypeProperty> Iterator
    for ListTypeIterator<T, P>
{
    type Item = P::Output;
    fn next(&mut self) -> Option<Self::Item> {
        (self.advance)(self)
    }
}

pub type TypeIdIterator<T> = ListTypeIterator<T, TypeId>;

pub struct Undefined;
pub trait EmptyList {
    type PushFirst<U: 'static>: List;
    fn push_first<U: 'static>(self, item: U) -> Self::PushFirst<U>;

    type RootEmpty: EmptyList;
    fn root_empty() -> Self::RootEmpty;
}

/// A blanket implementation for EmptyList.
impl<T: EmptyList> List for T {
    type Empty = T::RootEmpty;
    fn empty() -> Self::Empty {
        T::root_empty()
    }

    type Head = Undefined;
    fn head(&self) -> &Self::Head {
        unreachable!("EmptyList has no head")
    }

    type Tail = T; // Satisfy the List trait
    fn tail(&self) -> &Self::Tail {
        unreachable!("EmptyList has no tail")
    }

    type Push<U: 'static> = T::PushFirst<U>;
    fn push<U>(self, item: U) -> Self::Push<U> {
        self.push_first(item)
    }

    const LENGTH: usize = 0;
    const HEAD_PADDING: usize = 0;

    type Append<U: List> = U;
    fn append<U: List>(self, other: U) -> Self::Append<U> {
        other
    }

    type ReverseOnto<U: List> = U;
    fn reverse_onto<U: List>(self, other: U) -> Self::ReverseOnto<U> {
        other
    }
}

pub trait ListIndex<Idx: ?Sized> {
    type Output;
    fn index(&self, index: Idx) -> &Self::Output;
}

/// Type alias for getting element type at index `N`, following [`std::ops::Index`] convention
pub type Item<L, N> = <L as ListIndex<N>>::Output;

pub trait ToList {
    type ToList<T: EmptyList>: List;
    fn to_list<T: EmptyList>(&self) -> Self::ToList<T>;
}

pub trait IntoList {
    type Output<T: EmptyList>: List;
    fn into_list<T: EmptyList>(self) -> Self::Output<T>;
}

impl IntoList for () {
    type Output<T: EmptyList> = <T as List>::Empty;
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        T::empty()
    }
}

impl<A: 'static> IntoList for (A,) {
    type Output<T: EmptyList> = <<T as List>::Empty as List>::Push<A>;
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        ().into_list::<T>().push(self.0)
    }
}

impl<A: 'static, B: 'static> IntoList for (A, B) {
    type Output<T: EmptyList> = <<(B,) as IntoList>::Output<T> as List>::Push<A>;
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        (self.1,).into_list::<T>().push(self.0)
    }
}

impl<A: 'static, B: 'static, C: 'static> IntoList for (A, B, C) {
    type Output<T: EmptyList> = <<(B, C) as IntoList>::Output<T> as List>::Push<A>;
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        (self.1, self.2).into_list::<T>().push(self.0)
    }
}
impl<A: 'static, B: 'static, C: 'static, D: 'static> IntoList for (A, B, C, D) {
    type Output<T: EmptyList> = <<(B, C, D) as IntoList>::Output<T> as List>::Push<A>;
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        (self.1, self.2, self.3).into_list::<T>().push(self.0)
    }
}

impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static> IntoList for (A, B, C, D, E) {
    type Output<T: EmptyList> = <<(B, C, D, E) as IntoList>::Output<T> as List>::Push<A>;
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
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        (
            self.1, self.2, self.3, self.4, self.5, self.6, self.7, self.8, self.9, self.10,
            self.11,
        )
            .into_list::<T>()
            .push(self.0)
    }
}
