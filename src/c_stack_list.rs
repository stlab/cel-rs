use std::mem::offset_of;
use std::ops::{Index, RangeFrom, RangeTo, RangeToInclusive, Sub};
use std::{fmt, ptr};
use typenum::{B1, Bit, Sub1, U0, UInt, Unsigned};

use crate::list_traits::{
    EmptyList, IntoList, List, ListTypeIterator, ListTypeIteratorAdvance, ListTypeProperty,
};

/// A list using a guaranteed memory layout (`repr(C)`), with tail stored first so appending items
/// does not change the memory layout of prior items. The tail may itself contain a list to ensure
/// alignment during [`std::ops::RangeTo`] and [`std::ops::RangeToInclusive`] indexing.
///
/// See <https://doc.rust-lang.org/stable/reference/type-layout.html#r-layout.repr.c.struct>
///
/// # Indexing
///
/// Indexing is done using the [`typenum::uint::UInt`] type integral constants.
///
/// Because the [`std::ops::Range`] trait requires a `start` and `end` that are the same type,
/// it cannot be implemented for `List` types. Instead we use the [`RangeFrom`] and [`RangeTo`]
/// traits. To Access a range of elements, you can use the syntax `list[..end][start..]`.
///
/// # Example
///
///
/// ```rust
/// use cel_rs::c_stack_list::*;
/// use typenum::*;
///
/// let list = (1, 2.5, 3, 4, "world", "Hello").into_cstack_list();
/// assert_eq!(list[..U5::new()][U2::new()..], (3, 4, "world").into_cstack_list());
/// ```
///
/// Indexing out of bounds will result in a compile error.
///
/// ```compile_fail,E0277
/// use cel_rs::c_stack_list::*;
/// use typenum::*;
///
/// let list = (1, 2.5, 3, 4, "world", "Hello").into_cstack_list()[U6::new()];
/// ```
#[repr(C)]
#[derive(Clone)]
pub struct CStackList<H, T>(pub T, pub H);

/*
pub trait HeadPadding: List {
    const _HEAD_PADDING: usize;
    const _HEAD_OFFSET: usize;
}

impl<T: List> HeadPadding for CNil<T> {
    const _HEAD_PADDING: usize = 0;
    const _HEAD_OFFSET: usize = 0;
}

impl<H: 'static, T: HeadPadding> HeadPadding for CStackList<H, T> {
    const _HEAD_PADDING: usize =
        Self::_HEAD_OFFSET - (Self::Tail::_HEAD_OFFSET + size_of::<<Self::Tail as List>::Head>());
    const _HEAD_OFFSET: usize = offset_of!(Self, 1);
}
 */
impl<H: 'static, T: List> List for CStackList<H, T> {
    type Head = H;
    fn head(&self) -> &Self::Head {
        &self.1
    }

    type Tail = T;
    fn tail(&self) -> &Self::Tail {
        &self.0
    }

    const HEAD_PADDING: usize =
        Self::HEAD_OFFSET - (Self::Tail::HEAD_OFFSET + size_of::<<Self::Tail as List>::Head>());
    const HEAD_OFFSET: usize = offset_of!(Self, 1);
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

    type Reverse = Self::ReverseOnto<CNil<()>>;
    fn reverse(self) -> Self::Reverse {
        self.reverse_onto(CNil(()))
    }
}

pub trait IntoCStackList {
    type Output: List;
    fn into_cstack_list(self) -> Self::Output;
}

impl<T: IntoList> IntoCStackList for T {
    type Output = T::Output<CNil<()>>;
    fn into_cstack_list(self) -> Self::Output {
        self.into_list()
    }
}

impl<H: 'static, T: List> Index<RangeFrom<U0>> for CStackList<H, T> {
    type Output = CStackList<H, T>;
    fn index(&self, _index: RangeFrom<U0>) -> &Self::Output {
        self
    }
}

impl<H: 'static, T: List, U: Unsigned, B: Bit> Index<RangeFrom<UInt<U, B>>> for CStackList<H, T>
where
    T: Index<RangeFrom<Sub1<UInt<U, B>>>>,
    UInt<U, B>: Sub<B1>,
{
    type Output = <T as Index<RangeFrom<Sub1<UInt<U, B>>>>>::Output;
    fn index(&self, index: RangeFrom<UInt<U, B>>) -> &Self::Output {
        self.tail().index((index.start - B1)..)
    }
}

impl<H: 'static, T: List> Index<RangeTo<U0>> for CStackList<H, T> {
    type Output = CNil<CStackList<H, T>>;
    fn index(&self, _index: RangeTo<U0>) -> &Self::Output {
        unsafe { &*ptr::from_ref(self).cast::<Self::Output>() }
    }
}

type TailRangeTo<T, U, B> = <T as Index<RangeTo<Sub1<UInt<U, B>>>>>::Output;

impl<H: 'static, T: List, U: Unsigned, B: Bit> Index<RangeTo<UInt<U, B>>> for CStackList<H, T>
where
    T: Index<RangeTo<Sub1<UInt<U, B>>>>,
    TailRangeTo<T, U, B>: List,
    UInt<U, B>: Sub<B1>,
{
    type Output = <TailRangeTo<T, U, B> as List>::Push<H>;
    fn index(&self, _index: RangeTo<UInt<U, B>>) -> &Self::Output {
        unsafe { &*ptr::from_ref(self).cast::<Self::Output>() }
    }
}

impl<H: 'static, T: List> Index<RangeToInclusive<U0>> for CStackList<H, T> {
    type Output = CStackList<H, CNil<T>>;
    fn index(&self, _index: RangeToInclusive<U0>) -> &Self::Output {
        unsafe { &*ptr::from_ref(self).cast::<Self::Output>() }
    }
}

type TailRangeToInclusive<T, U, B> = <T as Index<RangeToInclusive<Sub1<UInt<U, B>>>>>::Output;
impl<H: 'static, T: List, U: Unsigned, B: Bit> Index<RangeToInclusive<UInt<U, B>>>
    for CStackList<H, T>
where
    T: Index<RangeToInclusive<Sub1<UInt<U, B>>>>,
    TailRangeToInclusive<T, U, B>: List,
    UInt<U, B>: Sub<B1>,
{
    type Output = <TailRangeToInclusive<T, U, B> as List>::Push<H>;
    fn index(&self, _index: RangeToInclusive<UInt<U, B>>) -> &Self::Output {
        unsafe { &*ptr::from_ref(self).cast::<Self::Output>() }
    }
}

impl<H: 'static, T: List> Index<U0> for CStackList<H, T> {
    type Output = H;
    fn index(&self, _index: U0) -> &Self::Output {
        self.head()
    }
}

// Move this to an #[derive(DebugList)] macro
trait DebugHelper {
    fn fmt_helper(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}

impl<T> DebugHelper for CNil<T> {
    fn fmt_helper(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ")")
    }
}

impl<H: 'static, T: List> DebugHelper for CStackList<H, T>
where
    H: fmt::Debug,
    T: DebugHelper,
{
    fn fmt_helper(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ", {:?}", self.head())?;
        self.tail().fmt_helper(f)
    }
}

impl<T> fmt::Debug for CNil<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "()")
    }
}

impl<H: 'static, T: List> fmt::Debug for CStackList<H, T>
where
    H: fmt::Debug,
    T: DebugHelper,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({:?}", self.head())?;
        self.tail().fmt_helper(f)
    }
}

impl<T: List, O: List> std::cmp::PartialEq<O> for CNil<T> {
    fn eq(&self, other: &O) -> bool {
        other.is_empty()
    }
}

impl<H: 'static, T: List, O: List> std::cmp::PartialEq<O> for CStackList<H, T>
where
    H: PartialEq<O::Head>,
    T: PartialEq<O::Tail>,
{
    fn eq(&self, other: &O) -> bool {
        self.head() == other.head() && self.tail() == other.tail()
    }
}

/// Type alias for getting element type at index `N`, following [`std::ops::Index`] convention
pub type Item<L, N> = <L as Index<N>>::Output;

impl<H: 'static, T: List, U: Unsigned, B: Bit> Index<UInt<U, B>> for CStackList<H, T>
where
    T: Index<Sub1<UInt<U, B>>>,
    UInt<U, B>: Sub<B1>,
{
    type Output = <T as Index<Sub1<UInt<U, B>>>>::Output;
    fn index(&self, index: UInt<U, B>) -> &Self::Output {
        self.tail().index(index - B1)
    }
}

#[repr(C)]
pub struct CNil<T>(T);

impl<T> EmptyList for CNil<T> {
    type PushFirst<U: 'static> = CStackList<U, CNil<T>>;
    fn push_first<U: 'static>(self, item: U) -> Self::PushFirst<U> {
        CStackList(self, item)
    }

    type FromTuple<L: IntoList> = L::Output<CNil<T>>;
    fn from_tuple<L: IntoList>(tuple: L) -> Self::FromTuple<L> {
        tuple.into_list()
    }

    type Empty = CNil<()>;
    fn empty() -> Self::Empty {
        CNil(())
    }
}

impl<T, P: ListTypeProperty> ListTypeIteratorAdvance<P> for CNil<T> {
    fn advancer<R: List>(_iter: &mut ListTypeIterator<R, P>) -> Option<P::Output> {
        None
    }
}

impl<P: ListTypeProperty, H: 'static, T: ListTypeIteratorAdvance<P>> ListTypeIteratorAdvance<P>
    for CStackList<H, T>
{
    fn advancer<R: List>(iter: &mut ListTypeIterator<R, P>) -> Option<P::Output> {
        iter.advance = T::advancer::<R>;
        Some(P::property::<CStackList<H, T>>())
    }
}

#[cfg(test)]
mod tests {
    use typenum::{U1, U2, U5};

    use super::*;
    #[test]
    fn test_into_cstack_list() {
        let list = (1, 2.5, 3, 4, "world", "Hello").into_cstack_list();
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
        assert_eq!(().into_cstack_list(), CNil(()));
    }

    #[test]
    fn test_index() {
        let list = (1, 2.5, "Hello").into_cstack_list();
        assert_eq!(list[U0::new()], 1);
        assert_eq!(list[U1::new()], 2.5);
        assert_eq!(list[U2::new()], "Hello");
    }

    #[test]
    fn test_slice() {
        let list = (1, 2.5, 3, 4, "world", "Hello").into_list::<CNil<()>>();
        println!("{:?}", list[..U5::new()][U2::new()..]);
    }

    #[test]
    fn test_index_range_from() {
        let list = (1, 2.5, "Hello").into_list::<CNil<()>>();
        assert_eq!(list[U1::new()..][U1::new()], "Hello");
    }

    #[test]
    fn test_index_range_to() {
        let list = (1, 2.5, "Hello").into_list::<CNil<()>>();
        assert_eq!(list[..U2::new()][U1::new()], 2.5);
    }

    // Update the test to use Item instead of At
    #[test]
    fn test_index_type() {
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
    fn test_cstack_list() {
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
