use std::any::TypeId;

pub trait List {
    type Head: 'static;
    fn head(&self) -> &Self::Head;

    type Tail: List;
    fn tail(&self) -> &Self::Tail;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn len(&self) -> usize {
        Self::LENGTH
    }

    const LENGTH: usize = Self::Tail::LENGTH + 1;
    const HEAD_PADDING: usize;

    type Push<U: 'static>: List;
    fn push<U: 'static>(self, item: U) -> Self::Push<U>;

    type Append<U: List>: List;
    fn append<U: List>(self, other: U) -> Self::Append<U>;

    type ReverseOnto<U: List>: List;
    fn reverse_onto<U: List>(self, other: U) -> Self::ReverseOnto<U>;

    type Reverse: List;
    fn reverse(self) -> Self::Reverse;

    fn for_each_type<H: TypeHandler>(handler: &mut H)
    where
        Self: Sized + 'static,
    {
        handler.invoke::<Self>();
        Self::Tail::for_each_type(handler);
    }
}

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

pub trait TypeHandler {
    fn invoke<T: List>(&mut self);
}

pub struct Bottom;
pub trait EmptyList {
    type PushFirst<U: 'static>: List;
    fn push_first<U: 'static>(self, item: U) -> Self::PushFirst<U>;

    type FromTuple<T: IntoList>: List;
    fn from_tuple<T: IntoList>(tuple: T) -> Self::FromTuple<T>;

    type Empty: EmptyList;
    fn empty() -> Self::Empty;
}

impl<T: EmptyList> List for T {
    type Head = Bottom;
    type Tail = T; // Satisfy the List trait
    type Push<U: 'static> = T::PushFirst<U>;
    type Append<U: List> = U;
    type Reverse = T;
    type ReverseOnto<U: List> = U;
    const LENGTH: usize = 0;
    const HEAD_PADDING: usize = 0;

    fn head(&self) -> &Self::Head {
        unreachable!("EmptyList has no head")
    }

    fn tail(&self) -> &Self::Tail {
        unreachable!("EmptyList has no tail")
    }

    fn push<U>(self, item: U) -> Self::Push<U> {
        self.push_first(item)
    }

    fn append<U: List>(self, other: U) -> Self::Append<U> {
        other
    }

    fn reverse_onto<U: List>(self, other: U) -> Self::ReverseOnto<U> {
        other
    }

    fn reverse(self) -> Self::Reverse {
        self
    }

    fn for_each_type<H: TypeHandler>(_handler: &mut H) {}
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
    type Output<T: EmptyList> = T::Empty;
    fn into_list<T: EmptyList>(self) -> Self::Output<T> {
        T::empty()
    }
}

impl<A: 'static> IntoList for (A,) {
    type Output<T: EmptyList> = <T::Empty as List>::Push<A>;
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
