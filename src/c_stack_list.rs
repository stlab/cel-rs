//! A [`CStackList`] is a [`List`] with guaranteed memory layout (`repr(C)`). The tail is stored
//! first, so appending items does not change the memory layout of prior items (though the required
//! alignment may increase). [`CNil<T>`] represents the empty list where `T` is used to hide the
//! tail for [`std::ops::RangeTo`] and [`std::ops::RangeToInclusive`] indexing.
//!
//! See <https://doc.rust-lang.org/stable/reference/type-layout.html#r-layout.repr.c.struct>
//!
//! # Indexing
//!
//! Indexing is done using the [`typenum::uint::UInt`] type integral constants.
//!
//! Because the [`std::ops::Range`] trait requires a `start` and `end` that are the same type,
//! it cannot be implemented for `List` types. Instead we use the [`RangeFrom`] and [`RangeTo`]
//! traits. To Access a range of elements, you can use the syntax `list[..end][start..]`.
//!
//! # Example
//!
//!
//! ```rust
//! use cel_rs::*;
//! use typenum::*;
//!
//! let list = (1, 2.5, 3, 4, "world", "Hello").into_c_stack_list();
//! assert_eq!(list[..U5::new()][U2::new()..], (3, 4, "world").into_c_stack_list());
//! ```
//!
//! Indexing out of bounds will result in a compile error.
//!
//! ```compile_fail,E0277
//! use cel_rs::c_stack_list::*;
//! use typenum::*;
//!
//! let list = (1, 2.5, 3, 4, "world", "Hello").into_c_stack_list()[U6::new()];
//! ```
use std::mem::offset_of;
use std::ops::{Index, RangeFrom, RangeTo, RangeToInclusive, Sub};
use std::{fmt, ptr};

use typenum::{B1, Bit, Sub1, U0, UInt, Unsigned};

use crate::list_traits::{
    EmptyList, IntoList, List, ListIndex, ListTypeIterator, ListTypeIteratorAdvance,
    ListTypeProperty,
};

/// A list using a guaranteed memory layout (`repr(C)`), with tail stored first so appending items
/// does not change the memory layout of prior items.
#[repr(C)]
#[derive(Clone)]
pub struct CStackList<H, T: CStackListHeadLimit>(pub T, pub H);

/// A trait describing the memory layout of a [`CStackList`].
/// Describes the memory layout of a [`CStackList`]'s head and tail boundary.
pub trait CStackListHeadLimit {
    /// The offset to the _end_ of the head element.
    const HEAD_LIMIT: usize;
}

/// Indicates whether the head element is padded to satisfy alignment.
pub trait CStackListHeadPadded {
    /// Whether the head element is padded to the next alignment boundary.
    const HEAD_PADDED: bool;
}

impl<H: 'static, T: CStackListHeadLimit> CStackListHeadPadded for CStackList<H, T> {
    const HEAD_PADDED: bool = offset_of!(Self, 1) != T::HEAD_LIMIT;
}

impl<H: 'static, T: CStackListHeadLimit> CStackListHeadLimit for CStackList<H, T> {
    const HEAD_LIMIT: usize = offset_of!(Self, 1) + size_of::<H>();
}

impl<T: CStackListHeadLimit> CStackListHeadLimit for CNil<T> {
    const HEAD_LIMIT: usize = T::HEAD_LIMIT;
}

impl CStackListHeadLimit for () {
    const HEAD_LIMIT: usize = 0;
}

impl<H: 'static, T: List + CStackListHeadLimit> List for CStackList<H, T> {
    type Empty = CNil<()>;
    fn empty() -> Self::Empty {
        CNil(())
    }

    type Head = H;
    fn head(&self) -> &Self::Head {
        &self.1
    }

    type Tail = T;
    fn tail(&self) -> &Self::Tail {
        &self.0
    }

    type Push<U: 'static> = CStackList<U, Self>;
    fn push<U: 'static>(self, item: U) -> Self::Push<U> {
        CStackList(self, item)
    }

    type Append<U: List> = <T::Append<U> as List>::Push<H>;
    fn append<U: List>(self, other: U) -> Self::Append<U> {
        self.0.append(other).push(self.1)
    }

    type ReverseOnto<U: List> = T::ReverseOnto<U::Push<H>>;
    fn reverse_onto<U: List>(self, other: U) -> Self::ReverseOnto<U> {
        self.0.reverse_onto(other.push(self.1))
    }
}

/// Converts tuples and list-like values into a [`CStackList`].
pub trait IntoCStackList {
    /// The resulting list type.
    type Output: List;
    /// Convert into a `CStackList` preserving element order.
    fn into_c_stack_list(self) -> Self::Output;
}

impl<T: IntoList> IntoCStackList for T {
    type Output = T::Output<CNil<()>>;
    fn into_c_stack_list(self) -> Self::Output {
        self.into_list()
    }
}

impl<H: 'static, T: List + CStackListHeadLimit> ListIndex<RangeFrom<U0>> for CStackList<H, T> {
    type Output = CStackList<H, T>;
    fn index(&self, _index: RangeFrom<U0>) -> &Self::Output {
        self
    }
}

impl<H: 'static, T: List + CStackListHeadLimit, U: Unsigned, B: Bit>
    ListIndex<RangeFrom<UInt<U, B>>> for CStackList<H, T>
where
    T: ListIndex<RangeFrom<Sub1<UInt<U, B>>>>,
    UInt<U, B>: Sub<B1>,
{
    type Output = <T as ListIndex<RangeFrom<Sub1<UInt<U, B>>>>>::Output;
    fn index(&self, index: RangeFrom<UInt<U, B>>) -> &Self::Output {
        self.tail().index((index.start - B1)..)
    }
}

impl<H: 'static, T: List + CStackListHeadLimit> ListIndex<RangeTo<U0>> for CStackList<H, T> {
    type Output = CNil<CStackList<H, T>>;
    fn index(&self, _index: RangeTo<U0>) -> &Self::Output {
        unsafe { &*ptr::from_ref(self).cast::<Self::Output>() }
    }
}

type TailRangeTo<T, U, B> = <T as ListIndex<RangeTo<Sub1<UInt<U, B>>>>>::Output;

impl<H: 'static, T: List + CStackListHeadLimit, U: Unsigned, B: Bit> ListIndex<RangeTo<UInt<U, B>>>
    for CStackList<H, T>
where
    T: ListIndex<RangeTo<Sub1<UInt<U, B>>>>,
    TailRangeTo<T, U, B>: List,
    UInt<U, B>: Sub<B1>,
{
    type Output = <TailRangeTo<T, U, B> as List>::Push<H>;
    fn index(&self, _index: RangeTo<UInt<U, B>>) -> &Self::Output {
        unsafe { &*ptr::from_ref(self).cast::<Self::Output>() }
    }
}

impl<H: 'static, T: List + CStackListHeadLimit> ListIndex<RangeToInclusive<U0>>
    for CStackList<H, T>
{
    type Output = CStackList<H, CNil<T>>;
    fn index(&self, _index: RangeToInclusive<U0>) -> &Self::Output {
        unsafe { &*ptr::from_ref(self).cast::<Self::Output>() }
    }
}

type TailRangeToInclusive<T, U, B> = <T as ListIndex<RangeToInclusive<Sub1<UInt<U, B>>>>>::Output;

impl<H: 'static, T: List + CStackListHeadLimit, U: Unsigned, B: Bit>
    ListIndex<RangeToInclusive<UInt<U, B>>> for CStackList<H, T>
where
    T: ListIndex<RangeToInclusive<Sub1<UInt<U, B>>>>,
    TailRangeToInclusive<T, U, B>: List,
    UInt<U, B>: Sub<B1>,
{
    type Output = <TailRangeToInclusive<T, U, B> as List>::Push<H>;
    fn index(&self, _index: RangeToInclusive<UInt<U, B>>) -> &Self::Output {
        unsafe { &*ptr::from_ref(self).cast::<Self::Output>() }
    }
}

impl<H: 'static, T: List + CStackListHeadLimit> ListIndex<U0> for CStackList<H, T> {
    type Output = H;
    fn index(&self, _index: U0) -> &Self::Output {
        self.head()
    }
}

impl<H: 'static, T: List + CStackListHeadLimit, U: Unsigned, B: Bit> ListIndex<UInt<U, B>>
    for CStackList<H, T>
where
    T: ListIndex<Sub1<UInt<U, B>>>,
    UInt<U, B>: Sub<B1>,
{
    type Output = <T as ListIndex<Sub1<UInt<U, B>>>>::Output;
    fn index(&self, index: UInt<U, B>) -> &Self::Output {
        self.tail().index(index - B1)
    }
}

// Implement Index in terms of ListIndex for CStackList
impl<H: 'static, T: List + CStackListHeadLimit, Idx> Index<Idx> for CStackList<H, T>
where
    Self: ListIndex<Idx>,
    <Self as ListIndex<Idx>>::Output: Sized,
{
    type Output = <Self as ListIndex<Idx>>::Output;
    fn index(&self, index: Idx) -> &Self::Output {
        ListIndex::index(self, index)
    }
}

// Move this to an #[derive(DebugList)] macro
trait DebugHelper {
    fn fmt_helper(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}

impl<T: CStackListHeadLimit> DebugHelper for CNil<T> {
    fn fmt_helper(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ")")
    }
}

impl<H: 'static, T: List + CStackListHeadLimit> DebugHelper for CStackList<H, T>
where
    H: fmt::Debug,
    T: DebugHelper,
{
    fn fmt_helper(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ", {:?}", self.head())?;
        self.tail().fmt_helper(f)
    }
}

impl<T: CStackListHeadLimit> fmt::Debug for CNil<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "()")
    }
}

impl<H: 'static, T: List + CStackListHeadLimit> fmt::Debug for CStackList<H, T>
where
    H: fmt::Debug,
    T: DebugHelper,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({:?}", self.head())?;
        self.tail().fmt_helper(f)
    }
}

impl<T: List + CStackListHeadLimit, O: List> std::cmp::PartialEq<O> for CNil<T> {
    fn eq(&self, other: &O) -> bool {
        other.is_empty()
    }
}

impl<H: 'static, T: List + CStackListHeadLimit, O: List> std::cmp::PartialEq<O> for CStackList<H, T>
where
    H: PartialEq<O::Head>,
    T: PartialEq<O::Tail>,
{
    fn eq(&self, other: &O) -> bool {
        self.head() == other.head() && self.tail() == other.tail()
    }
}

/// Empty `repr(C)` list used as the tail sentinel for [`CStackList`].
#[repr(C)]
pub struct CNil<T: CStackListHeadLimit>(T);

impl<T: CStackListHeadLimit> EmptyList for CNil<T> {
    type PushFirst<U: 'static> = CStackList<U, CNil<T>>;
    fn push_first<U: 'static>(self, item: U) -> Self::PushFirst<U> {
        CStackList(self, item)
    }

    type RootEmpty = CNil<()>;
    fn root_empty() -> Self::RootEmpty {
        CNil(())
    }
}

impl<T: CStackListHeadLimit, P: ListTypeProperty> ListTypeIteratorAdvance<P> for CNil<T> {
    fn advancer<R: List>(_iter: &mut ListTypeIterator<R, P>) -> Option<P::Output> {
        None
    }
}

impl<P: ListTypeProperty, H: 'static, T: ListTypeIteratorAdvance<P> + CStackListHeadLimit>
    ListTypeIteratorAdvance<P> for CStackList<H, T>
{
    fn advancer<R: List>(iter: &mut ListTypeIterator<R, P>) -> Option<P::Output> {
        iter.advance = T::advancer::<R>;
        Some(P::property::<CStackList<H, T>>())
    }
}

#[cfg(test)]
mod tests {
    use typenum::{U1, U2, U5};

    use crate::list_traits::Item;

    use super::*;
    #[test]
    fn into_c_stack_list() {
        let list = (1, 2.5, 3, 4, "world", "Hello").into_c_stack_list();
        assert_eq!(
            list,
            CStackList(
                CStackList(
                    CStackList(
                        CStackList(CStackList(CStackList(CNil(()), "Hello"), "world"), 4),
                        3
                    ),
                    2.5
                ),
                1
            )
        );
        assert_eq!(().into_c_stack_list(), CNil(()));
    }

    #[test]
    fn index() {
        let list = (1, 2.5, "Hello").into_c_stack_list();
        assert_eq!(list[U0::new()], 1);
        assert_eq!(list[U1::new()], 2.5);
        assert_eq!(list[U2::new()], "Hello");
    }

    #[test]
    fn slice() {
        let list = (1, 2.5, 3, 4, "world", "Hello").into_c_stack_list();
        println!("{:?}", list[..U5::new()][U2::new()..]);
    }

    #[test]
    fn index_range_from() {
        let list = (1, 2.5, "Hello").into_c_stack_list();
        assert_eq!(list[U1::new()..][U1::new()], "Hello");
    }

    #[test]
    fn index_range_to() {
        let list = (1, 2.5, "Hello").into_c_stack_list();
        assert_eq!(list[..U2::new()][U1::new()], 2.5);
    }

    #[test]
    fn index_type() {
        use std::any::type_name;

        type List = <(i32, f64, &'static str) as IntoList>::Output<CNil<()>>;
        type Zero = Item<List, U0>;
        type One = Item<List, U1>;
        type Two = Item<List, U2>;
        assert_eq!(type_name::<Zero>(), "i32");
        assert_eq!(type_name::<One>(), "f64");
        assert_eq!(type_name::<Two>(), "&str");
    }

    #[test]
    fn cstack_list() {
        let list = CStackList(CNil(()), 32i32).push("Hello").push(42.5);
        // Test that we can cast to a C struct and read values
        #[repr(C)]
        struct TestStruct(i32, &'static str, f64);

        let test_struct = unsafe { std::mem::transmute::<_, TestStruct>(list) };

        assert_eq!(test_struct.0, 32);
        assert_eq!(test_struct.1, "Hello");
        assert_eq!(test_struct.2, 42.5);
    }
}
