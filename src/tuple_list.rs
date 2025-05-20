//! Implements [`List`] for tuples where `()` is an empty list and `(H, T)` is a list with a head
//! and tail.

use typenum::{B1, Bit, Sub1, U0, UInt, Unsigned};

use crate::list_traits::{
    EmptyList, IntoList, List, ListIndex, ListTypeIterator, ListTypeIteratorAdvance,
    ListTypeProperty,
};
use std::ops::{RangeFrom, Sub};

//--------------------------------------------------------------------------------------------------
// ListTypeIteratorAdvance

impl<P: ListTypeProperty> ListTypeIteratorAdvance<P> for () {
    fn advancer<R: List>(_iter: &mut ListTypeIterator<R, P>) -> Option<P::Output> {
        None
    }
}

impl<P: ListTypeProperty, H: 'static, T: ListTypeIteratorAdvance<P>> ListTypeIteratorAdvance<P>
    for (H, T)
{
    fn advancer<R: List>(iter: &mut ListTypeIterator<R, P>) -> Option<P::Output> {
        iter.advance = T::advancer::<R>;
        Some(P::property::<(H, T)>())
    }
}

//--------------------------------------------------------------------------------------------------
// EmptyList for ()

impl EmptyList for () {
    type PushFirst<U: 'static> = (U, ());
    fn push_first<U: 'static>(self, item: U) -> Self::PushFirst<U> {
        (item, ())
    }

    type FromTuple<T: IntoList> = T::Output<()>;
    fn from_tuple<T: IntoList>(tuple: T) -> Self::FromTuple<T> {
        tuple.into_list()
    }

    type RootEmpty = Self;
    fn root_empty() -> Self::RootEmpty {}
}

//--------------------------------------------------------------------------------------------------
// List for (H, T)

impl<H: 'static, T: List> List for (H, T) {
    type Empty = ();
    fn empty() -> Self::Empty {}

    type Head = H;
    fn head(&self) -> &Self::Head {
        &self.0
    }

    type Tail = T;
    fn tail(&self) -> &Self::Tail {
        &self.1
    }

    const HEAD_PADDING: usize = usize::MAX; // undefined

    type Push<U: 'static> = (U, Self);
    fn push<U: 'static>(self, item: U) -> Self::Push<U> {
        (item, self)
    }

    type Append<U: List> = <T::Append<U> as List>::Push<H>;
    fn append<U: List>(self, other: U) -> Self::Append<U> {
        self.1.append(other).push(self.0)
    }

    type ReverseOnto<U: List> = T::ReverseOnto<U::Push<H>>;
    fn reverse_onto<U: List>(self, other: U) -> Self::ReverseOnto<U> {
        self.1.reverse_onto(other.push(self.0))
    }
}

impl<H: 'static, T: List> ListIndex<RangeFrom<U0>> for (H, T) {
    type Output = (H, T);
    fn index(&self, _index: RangeFrom<U0>) -> &Self::Output {
        self
    }
}

impl<H: 'static, T: List, U: Unsigned, B: Bit> ListIndex<RangeFrom<UInt<U, B>>> for (H, T)
where
    T: ListIndex<RangeFrom<Sub1<UInt<U, B>>>>,
    UInt<U, B>: Sub<B1>,
{
    type Output = <T as ListIndex<RangeFrom<Sub1<UInt<U, B>>>>>::Output;
    fn index(&self, index: RangeFrom<UInt<U, B>>) -> &Self::Output {
        self.tail().index((index.start - B1)..)
    }
}

impl<H: 'static, T: List> ListIndex<U0> for (H, T) {
    type Output = H;
    fn index(&self, _index: U0) -> &Self::Output {
        self.head()
    }
}

impl<H: 'static, T: List, U: Unsigned, B: Bit> ListIndex<UInt<U, B>> for (H, T)
where
    T: ListIndex<Sub1<UInt<U, B>>>,
    UInt<U, B>: Sub<B1>,
{
    type Output = <T as ListIndex<Sub1<UInt<U, B>>>>::Output;
    fn index(&self, index: UInt<U, B>) -> &Self::Output {
        self.tail().index(index - B1)
    }
}

//--------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list_traits::TypeIdIterator;
    use std::any::TypeId;

    #[test]
    fn type_id_iterator() {
        let ids: [TypeId; 3] = [
            TypeId::of::<u32>(),
            TypeId::of::<f64>(),
            TypeId::of::<&str>(),
        ];
        assert!(TypeIdIterator::<(u32, (f64, (&str, ())))>::new().eq(ids.iter().map(|&id| id)));
    }

    #[test]
    fn empty_list() {
        assert_eq!(<()>::empty(), ());
    }

    #[test]
    fn into_list() {
        assert_eq!(().into_list::<()>(), ());
        assert_eq!((1, 2, 3).into_list::<()>(), (1, (2, (3, ()))));
        assert_eq!(
            (1, 2.5, "Hello").into_list::<()>(),
            (1, (2.5, ("Hello", ())))
        );
    }

    #[test]
    fn push_front() {
        assert_eq!(().push_first(1), (1, ()));
        assert_eq!((1, 2, 3).into_list::<()>().push(4), (4, (1, (2, (3, ())))));
    }

    #[test]
    fn concat() {
        assert_eq!(
            (1, 2, 3)
                .into_list::<()>()
                .append((4, 5, 6).into_list::<()>()),
            (1, (2, (3, (4, (5, (6, ()))))))
        );
    }

    #[test]
    fn reverse() {
        assert_eq!((1, 2, 3).into_list::<()>().reverse(), (3, (2, (1, ()))));
    }

    #[test]
    fn list_length() {
        assert_eq!(<()>::LENGTH, 0);
        assert_eq!(<(i32, ())>::LENGTH, 1);
        assert_eq!(<(i32, (f64, ()))>::LENGTH, 2);
    }

    #[test]
    fn tuple_list() {
        let list = (1, 2.5, "Hello").into_list::<()>();
        println!("{:?}", list);
    }
}
