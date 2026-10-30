//! Implements [`List`] for tuples where `()` is an empty list and `(H, T)` is a list with a head
//! and tail.
//!
//! ```
//! use cel_runtime::{IntoTupleList, List};
//! let list = (String::from("first"), 7u8).into_tuple_list();
//! assert_eq!(list.head(), "first");
//! assert_eq!(list.reverse().reverse().len(), 2);
//! ```

use typenum::{B1, Bit, Sub1, U0, UInt, Unsigned};

use crate::list_traits::{
    EmptyList, IntoList, List, ListIndex, ListTypeIterator, ListTypeIteratorAdvance,
    ListTypeProperty,
};
use std::ops::{RangeFrom, Sub};

/// Converts tuples into a `List` representation where `()` is empty and `(H, T)` is a cons cell.
///
/// # Examples
/// See the [module tutorial](self).
pub trait IntoTupleList {
    /// The resulting list type.
    type Output: List;
    /// Moves source fields into a `List` where `(H, T)` models a cons cell.
    ///
    /// - Complexity: O(n) field moves for n source fields, without allocation.
    ///
    /// # Examples
    /// See [`IntoTupleList`].
    fn into_tuple_list(self) -> Self::Output;
}

impl<T: IntoList> IntoTupleList for T {
    type Output = T::Output<()>;
    /// Moves fields into their source-ordered tuple-list representation.
    ///
    /// - Complexity: O(n) field moves for n source fields.
    fn into_tuple_list(self) -> Self::Output {
        self.into_list()
    }
}

//--------------------------------------------------------------------------------------------------
// ListTypeIteratorAdvance

impl<P: ListTypeProperty> ListTypeIteratorAdvance<P> for () {
    /// Reports exhaustion without generating a head property.
    fn advancer<R: List>(_iter: &mut ListTypeIterator<R, P>) -> Option<P::Output> {
        None
    }
}

impl<P: ListTypeProperty, H: 'static, T: ListTypeIteratorAdvance<P>> ListTypeIteratorAdvance<P>
    for (H, T)
{
    /// Generates the current head property and advances to the tail.
    ///
    /// - Complexity: The property generator's complexity.
    fn advancer<R: List>(iter: &mut ListTypeIterator<R, P>) -> Option<P::Output> {
        iter.advance = T::advancer::<R>;
        Some(P::property::<(H, T)>())
    }
}

//--------------------------------------------------------------------------------------------------
// EmptyList for ()

impl EmptyList for () {
    type PushFirst<U: 'static> = (U, ());
    /// Moves the first field into a singleton tuple list.
    fn push_first<U: 'static>(self, item: U) -> Self::PushFirst<U> {
        (item, ())
    }

    type RootEmpty = Self;
    /// Constructs the empty tuple-list root.
    fn root_empty() -> Self::RootEmpty {}
}

//--------------------------------------------------------------------------------------------------
// List for (H, T)

impl<H: 'static, T: List> List for (H, T) {
    type Empty = ();
    /// Constructs an empty tuple-list root without accessing these fields.
    fn empty() -> Self::Empty {}

    type Head = H;
    /// Borrows the first logical field.
    fn head(&self) -> &Self::Head {
        &self.0
    }

    type Tail = T;
    /// Borrows the remaining logical fields.
    fn tail(&self) -> &Self::Tail {
        &self.1
    }

    type Push<U: 'static> = (U, Self);
    /// Moves a new first field and this list into their tuple node.
    fn push<U: 'static>(self, item: U) -> Self::Push<U> {
        (item, self)
    }

    type Append<U: List> = <T::Append<U> as List>::Push<H>;
    /// Moves these source-ordered fields before `other`.
    ///
    /// - Complexity: O(n) field moves for n fields in this list.
    fn append<U: List>(self, other: U) -> Self::Append<U> {
        self.1.append(other).push(self.0)
    }

    type ReverseOnto<U: List> = T::ReverseOnto<U::Push<H>>;
    /// Moves these fields in reverse order before `other`.
    ///
    /// - Complexity: O(n) field moves for n fields in this list.
    fn reverse_onto<U: List>(self, other: U) -> Self::ReverseOnto<U> {
        self.1.reverse_onto(other.push(self.0))
    }
}

impl ListIndex<RangeFrom<U0>> for () {
    type Output = ();
    /// Borrows the empty suffix at the only valid empty-list boundary.
    fn index(&self, _index: RangeFrom<U0>) -> &Self::Output {
        self
    }
}

impl<H: 'static, T: List> ListIndex<RangeFrom<U0>> for (H, T) {
    type Output = (H, T);
    /// Borrows the complete suffix starting at zero.
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
    /// Borrows the suffix at the type-checked source index.
    ///
    /// - Complexity: O(i) tail traversals for source index i.
    fn index(&self, index: RangeFrom<UInt<U, B>>) -> &Self::Output {
        self.tail().index((index.start - B1)..)
    }
}

impl<H: 'static, T: List> ListIndex<U0> for (H, T) {
    type Output = H;
    /// Borrows the first source field.
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
    /// Borrows the field at the type-checked source index.
    ///
    /// - Complexity: O(i) tail traversals for source index i.
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
    use typenum::{U0, U1, U2, U3};

    /// Iterates both list families in source order and remains exhausted.
    #[test]
    fn type_id_iterator() {
        let ids: [TypeId; 3] = [
            TypeId::of::<u32>(),
            TypeId::of::<f64>(),
            TypeId::of::<&str>(),
        ];
        for mut iter in [
            TypeIdIterator::<(u32, (f64, (&str, ())))>::new(),
            TypeIdIterator::<(u32, (f64, (&str, ())))>::default(),
        ] {
            assert!(iter.by_ref().eq(ids.iter().copied()));
            assert_eq!(iter.next(), None);
        }
        /// Describes the same source fields in the other canonical list family.
        type Canonical = <(u32, f64, &'static str) as crate::IntoCStackList>::Output;
        let mut canonical = TypeIdIterator::<Canonical>::default();
        assert!(canonical.by_ref().eq(ids));
        assert_eq!(canonical.next(), None);
        assert_eq!(TypeIdIterator::<()>::default().next(), None);
        assert_eq!(TypeIdIterator::<crate::CNil<()>>::new().next(), None);
    }

    /// Constructs the empty tuple-list identity.
    #[test]
    fn empty_list() {
        let () = <()>::empty();
    }

    /// Moves empty and heterogeneous native tuples into logical source order.
    #[test]
    fn into_list() {
        let () = ().into_tuple_list();
        assert_eq!((1, 2, 3).into_tuple_list(), (1, (2, (3, ()))));
        assert_eq!(
            (1, 2.5, "Hello").into_tuple_list(),
            (1, (2.5, ("Hello", ())))
        );
    }

    /// Prepends a value while preserving every existing field.
    #[test]
    fn push() {
        assert_eq!(().push_first(1), (1, ()));
        assert_eq!((1, 2, 3).into_tuple_list().push(4), (4, (1, (2, (3, ())))));
    }

    /// Concatenates two lists without reversing either sequence.
    #[test]
    fn append() {
        assert_eq!(
            (1, 2, 3)
                .into_tuple_list()
                .append((4, 5, 6).into_tuple_list()),
            (1, (2, (3, (4, (5, (6, ()))))))
        );
    }

    /// Reverses logical source order while retaining every value.
    #[test]
    fn reverse() {
        assert_eq!((1, 2, 3).into_tuple_list().reverse(), (3, (2, (1, ()))));
    }

    /// Accounts for empty, singleton, and multiple-field logical lists.
    #[test]
    fn list_length() {
        assert_eq!(<()>::LENGTH, 0);
        assert_eq!(<(i32, ())>::LENGTH, 1);
        assert_eq!(<(i32, (f64, ()))>::LENGTH, 2);
    }

    /// Borrows each heterogeneous field at its type-checked source index.
    #[test]
    fn list_index() {
        let list = (1, 2.5, "Hello").into_tuple_list();
        assert_eq!(*list.index(U0::new()), 1);
        assert_eq!(*list.index(U1::new()), 2.5);
        assert_eq!(*list.index(U2::new()), "Hello");
    }

    /// Borrows each suffix, including the empty suffix at the terminal index.
    #[test]
    fn list_range_from_index() {
        let list = (1, 2.5, "Hello").into_tuple_list();
        assert_eq!(*list.index(U0::new()..), (1, (2.5, ("Hello", ()))));
        assert_eq!(*list.index(U1::new()..), (2.5, ("Hello", ())));
        assert_eq!(*list.index(U2::new()..), ("Hello", ()));
        let () = *list.index(U3::new()..);
        // assert_eq!(*list.index(U4::new()..), ()); // Compiler error: index out of bounds
    }
}
