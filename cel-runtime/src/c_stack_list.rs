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
//! it cannot be implemented for `List` types. Instead we use the [`std::ops::RangeFrom`] and
//! [`std::ops::RangeTo`]
//! traits. To Access a range of elements, you can use the syntax `list[..end][start..]`.
//!
//! # Example
//!
//!
//! ```rust
//! use cel_runtime::*;
//! use typenum::*;
//!
//! let list = (1, 2.5, 3, 4, "world", "Hello").into_c_stack_list();
//! assert_eq!(list[..U5::new()][U2::new()..], (3, 4, "world").into_c_stack_list());
//! ```
//!
//! Indexing out of bounds will result in a compile error.
//!
//! ```compile_fail,E0277
//! use cel_runtime::c_stack_list::*;
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

/// Restricts layout metadata to representations controlled by this module.
mod sealed {
    /// Marks canonical representations whose field offsets the crate defines.
    pub trait Layout {}
}

impl sealed::Layout for () {}
impl<T: CStackListHeadLimit> sealed::Layout for CNil<T> {}
impl<H: 'static, T: CStackListHeadLimit> sealed::Layout for CStackList<H, T> {}

/// A list using a guaranteed memory layout (`repr(C)`), with tail stored first so appending items
/// does not change the memory layout of prior items.
///
/// Cloning copies each field according to its `Clone` contract. Equality compares
/// lengths and corresponding logical fields, independently of physical layout.
///
/// # Examples
/// ```
/// use cel_runtime::IntoCStackList;
/// let list = (String::from("owned"), 7u8).into_c_stack_list();
/// assert_eq!(list, list.clone());
/// ```
#[repr(C)]
#[derive(Clone)]
pub struct CStackList<H, T: CStackListHeadLimit>(pub T, pub H);

/// Describes the trusted head boundary of a canonical stack-list representation.
///
/// Only this crate defines layout metadata used by safe indexing and execution.
///
/// # Examples
/// ```
/// use cel_runtime::{CStackList, CStackListHeadLimit, CNil};
/// assert_eq!(CStackList::<u32, CNil<()>>::HEAD_LIMIT, 4);
/// ```
///
/// Foreign types cannot supply unchecked representation metadata:
///
/// ```compile_fail
/// use cel_runtime::CStackListHeadLimit;
/// struct Foreign;
/// impl CStackListHeadLimit for Foreign {
///     const HEAD_LIMIT: usize = 0;
/// }
/// ```
pub trait CStackListHeadLimit: sealed::Layout {
    /// The offset to the _end_ of the head element.
    ///
    /// # Examples
    /// See the [`CStackListHeadLimit`] example.
    const HEAD_LIMIT: usize;
}

/// Indicates whether the head element is padded to satisfy alignment.
///
/// # Examples
/// ```
/// use cel_runtime::{CStackList, CStackListHeadPadded, CNil};
/// type ByteStack = CStackList<u8, CNil<()>>;
/// assert!(!ByteStack::HEAD_PADDED);
/// ```
pub trait CStackListHeadPadded: CStackListHeadLimit {
    /// Whether the head element is padded to the next alignment boundary.
    ///
    /// # Examples
    /// See the [`CStackListHeadPadded`] example.
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
    /// Creates the canonical empty list without borrowing this list's values.
    fn empty() -> Self::Empty {
        CNil(())
    }

    type Head = H;
    /// Borrows the first logical field.
    fn head(&self) -> &Self::Head {
        &self.1
    }

    type Tail = T;
    /// Borrows the remaining logical fields.
    fn tail(&self) -> &Self::Tail {
        &self.0
    }

    type Push<U: 'static> = CStackList<U, Self>;
    /// Moves a new first field and this list into their canonical node.
    fn push<U: 'static>(self, item: U) -> Self::Push<U> {
        CStackList(self, item)
    }

    type Append<U: List> = <T::Append<U> as List>::Push<H>;
    /// Moves these fields before `other` without cloning them.
    ///
    /// - Complexity: O(n) field moves for n fields in this list.
    fn append<U: List>(self, other: U) -> Self::Append<U> {
        self.0.append(other).push(self.1)
    }

    type ReverseOnto<U: List> = T::ReverseOnto<U::Push<H>>;
    /// Moves these fields in reverse order before `other`.
    ///
    /// - Complexity: O(n) field moves for n fields in this list.
    fn reverse_onto<U: List>(self, other: U) -> Self::ReverseOnto<U> {
        self.0.reverse_onto(other.push(self.1))
    }
}

/// Converts tuples and list-like values into a source-ordered [`CStackList`].
///
/// The first source field becomes the logical head, even though each `repr(C)`
/// node physically stores its tail before its head.
///
/// # Examples
///
/// ```
/// use cel_runtime::{IntoCStackList, IntoRustTuple};
///
/// let list = (String::from("left"), 7_u8).into_c_stack_list();
/// assert_eq!(list.into_rust_tuple(), (String::from("left"), 7));
/// ```
pub trait IntoCStackList {
    /// The resulting list type.
    type Output: List;
    /// Moves fields into a `CStackList`, preserving logical source order.
    ///
    /// - Complexity: Native tuple conversions take O(n) field moves for n fields,
    ///   without heap allocation.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::{IntoCStackList, List};
    ///
    /// assert_eq!((1_u8, 2_u16).into_c_stack_list().len(), 2);
    /// ```
    fn into_c_stack_list(self) -> Self::Output;
}

impl<T: IntoList> IntoCStackList for T {
    type Output = T::Output<CNil<()>>;
    /// Moves the source fields into canonical source order.
    ///
    /// - Complexity: O(n) field moves for n source fields.
    fn into_c_stack_list(self) -> Self::Output {
        self.into_list()
    }
}

/// Moves a canonical list into a native Rust tuple in logical source order.
///
/// Canonical lists terminate in [`CNil<()>`]. Native tuple bridges support zero
/// through twelve fields; lists themselves have no arity limit. Nested list
/// fields remain lists rather than being recursively converted or flattened.
/// Conversion requires no cloning, equality, or formatting capabilities.
///
/// # Examples
///
/// ```
/// use cel_runtime::{IntoCStackList, IntoRustTuple};
///
/// let list = (String::from("left"), 7_u8, 2.5_f64).into_c_stack_list();
/// let value: (String, u8, f64) = list.into_rust_tuple();
/// assert_eq!(value, (String::from("left"), 7, 2.5));
/// ```
pub trait IntoRustTuple {
    /// The native tuple type with fields in logical source order.
    type Tuple;

    /// Moves each logical field into the corresponding native tuple field.
    ///
    /// - Complexity: O(n) field moves for n fields, without heap allocation.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::{IntoCStackList, IntoRustTuple};
    ///
    /// let value: (String,) = (String::from("owned"),)
    ///     .into_c_stack_list()
    ///     .into_rust_tuple();
    /// assert_eq!(value.0, "owned");
    /// ```
    fn into_rust_tuple(self) -> Self::Tuple;
}

impl IntoRustTuple for CNil<()> {
    type Tuple = ();

    /// Consumes the canonical empty list and returns the empty tuple.
    fn into_rust_tuple(self) -> Self::Tuple {}
}

/// Implements native tuple bridges by destructuring tail-first canonical nodes.
macro_rules! impl_rust_tuple {
    (@type) => { CNil<()> };
    (@type $head:ident $(, $tail:ident)*) => {
        CStackList<$head, impl_rust_tuple!(@type $($tail),*)>
    };
    (@pattern) => { CNil(()) };
    (@pattern $head:ident $(, $tail:ident)*) => {
        CStackList(impl_rust_tuple!(@pattern $($tail),*), $head)
    };
    ($($head:ident : $value:ident),+) => {
        impl<$($head: 'static),+> IntoRustTuple for impl_rust_tuple!(@type $($head),+) {
            type Tuple = ($($head,)+);

            /// Moves the canonical fields into their corresponding native tuple positions.
            ///
            /// - Complexity: O(n) field moves for n fields, without allocation.
            fn into_rust_tuple(self) -> Self::Tuple {
                let impl_rust_tuple!(@pattern $($value),+) = self;
                ($($value,)+)
            }
        }
    };
}

impl_rust_tuple!(A: a);
impl_rust_tuple!(A: a, B: b);
impl_rust_tuple!(A: a, B: b, C: c);
impl_rust_tuple!(A: a, B: b, C: c, D: d);
impl_rust_tuple!(A: a, B: b, C: c, D: d, E: e);
impl_rust_tuple!(A: a, B: b, C: c, D: d, E: e, F: f);
impl_rust_tuple!(A: a, B: b, C: c, D: d, E: e, F: f, G: g);
impl_rust_tuple!(A: a, B: b, C: c, D: d, E: e, F: f, G: g, H: h);
impl_rust_tuple!(A: a, B: b, C: c, D: d, E: e, F: f, G: g, H: h, I: i);
impl_rust_tuple!(A: a, B: b, C: c, D: d, E: e, F: f, G: g, H: h, I: i, J: j);
impl_rust_tuple!(A: a, B: b, C: c, D: d, E: e, F: f, G: g, H: h, I: i, J: j, K: k);
impl_rust_tuple!(A: a, B: b, C: c, D: d, E: e, F: f, G: g, H: h, I: i, J: j, K: k, L: l);

impl<H: 'static, T: List + CStackListHeadLimit> ListIndex<RangeFrom<U0>> for CStackList<H, T> {
    type Output = CStackList<H, T>;
    /// Borrows the complete suffix starting at zero.
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
    /// Borrows the suffix at the type-checked source index.
    ///
    /// - Complexity: O(i) tail traversals for source index i.
    fn index(&self, index: RangeFrom<UInt<U, B>>) -> &Self::Output {
        self.tail().index((index.start - B1)..)
    }
}

impl<H: 'static, T: List + CStackListHeadLimit> ListIndex<RangeTo<U0>> for CStackList<H, T> {
    type Output = CNil<CStackList<H, T>>;
    /// Borrows an empty prefix without transferring or dropping the backing fields.
    fn index(&self, _index: RangeTo<U0>) -> &Self::Output {
        unsafe { &*ptr::from_ref(self).cast::<Self::Output>() }
    }
}

/// Selects the tail's exclusive prefix after removing the current logical head.
type TailRangeTo<T, U, B> = <T as ListIndex<RangeTo<Sub1<UInt<U, B>>>>>::Output;

impl<H: 'static, T: List + CStackListHeadLimit, U: Unsigned, B: Bit> ListIndex<RangeTo<UInt<U, B>>>
    for CStackList<H, T>
where
    T: ListIndex<RangeTo<Sub1<UInt<U, B>>>>,
    TailRangeTo<T, U, B>: List,
    UInt<U, B>: Sub<B1>,
{
    type Output = <TailRangeTo<T, U, B> as List>::Push<H>;
    /// Borrows the type-checked exclusive prefix in place.
    fn index(&self, _index: RangeTo<UInt<U, B>>) -> &Self::Output {
        unsafe { &*ptr::from_ref(self).cast::<Self::Output>() }
    }
}

impl<H: 'static, T: List + CStackListHeadLimit> ListIndex<RangeToInclusive<U0>>
    for CStackList<H, T>
{
    type Output = CStackList<H, CNil<T>>;
    /// Borrows the singleton prefix without moving the backing tail.
    fn index(&self, _index: RangeToInclusive<U0>) -> &Self::Output {
        unsafe { &*ptr::from_ref(self).cast::<Self::Output>() }
    }
}

/// Selects the tail's inclusive prefix after removing the current logical head.
type TailRangeToInclusive<T, U, B> = <T as ListIndex<RangeToInclusive<Sub1<UInt<U, B>>>>>::Output;

impl<H: 'static, T: List + CStackListHeadLimit, U: Unsigned, B: Bit>
    ListIndex<RangeToInclusive<UInt<U, B>>> for CStackList<H, T>
where
    T: ListIndex<RangeToInclusive<Sub1<UInt<U, B>>>>,
    TailRangeToInclusive<T, U, B>: List,
    UInt<U, B>: Sub<B1>,
{
    type Output = <TailRangeToInclusive<T, U, B> as List>::Push<H>;
    /// Borrows the type-checked inclusive prefix in place.
    fn index(&self, _index: RangeToInclusive<UInt<U, B>>) -> &Self::Output {
        unsafe { &*ptr::from_ref(self).cast::<Self::Output>() }
    }
}

impl<H: 'static, T: List + CStackListHeadLimit> ListIndex<U0> for CStackList<H, T> {
    type Output = H;
    /// Borrows the first logical field.
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
    /// Borrows the field at the type-checked source index.
    ///
    /// - Complexity: O(i) tail traversals for source index i.
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
    /// Borrows the field or sublist selected by the underlying list index.
    ///
    /// - Complexity: The selected [`ListIndex`] operation's complexity.
    fn index(&self, index: Idx) -> &Self::Output {
        ListIndex::index(self, index)
    }
}

/// Formats the remaining source-ordered fields after an opening tuple field.
trait DebugHelper {
    /// Writes field separators and closes the tuple.
    ///
    /// # Errors
    /// Returns an error when the destination or a field formatter fails.
    ///
    /// - Complexity: O(n) field formatting operations for n remaining fields.
    fn fmt_helper(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}

impl<T: CStackListHeadLimit> DebugHelper for CNil<T> {
    /// Closes a tuple after its last field.
    ///
    /// # Errors
    /// Returns a destination formatting error.
    fn fmt_helper(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ")")
    }
}

impl<H: 'static, T: List + CStackListHeadLimit> DebugHelper for CStackList<H, T>
where
    H: fmt::Debug,
    T: DebugHelper,
{
    /// Formats this head and the remaining logical fields in source order.
    ///
    /// # Errors
    /// Returns a destination or field formatting error.
    ///
    /// - Complexity: O(n) field formatting operations for n fields.
    fn fmt_helper(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ", {:?}", self.head())?;
        self.tail().fmt_helper(f)
    }
}

impl<T: CStackListHeadLimit> fmt::Debug for CNil<T> {
    /// Formats the empty list as an empty tuple.
    ///
    /// # Errors
    /// Returns a destination formatting error.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "()")
    }
}

impl<H: 'static, T: List + CStackListHeadLimit> fmt::Debug for CStackList<H, T>
where
    H: fmt::Debug,
    T: DebugHelper,
{
    /// Formats the logical fields as a source-ordered tuple.
    ///
    /// # Errors
    /// Returns a destination or field formatting error.
    ///
    /// - Complexity: O(n) field formatting operations for n fields.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({:?}", self.head())?;
        self.tail().fmt_helper(f)
    }
}

impl<T: List + CStackListHeadLimit, O: List> std::cmp::PartialEq<O> for CNil<T> {
    /// Returns whether the other logical list is also empty.
    fn eq(&self, other: &O) -> bool {
        other.is_empty()
    }
}

impl<H: 'static, T: List + CStackListHeadLimit, O: List> std::cmp::PartialEq<O> for CStackList<H, T>
where
    H: PartialEq<O::Head>,
    T: PartialEq<O::Tail>,
{
    /// Compares lengths and corresponding logical elements.
    ///
    /// - Complexity: O(n) field comparisons for n elements, excluding field costs.
    fn eq(&self, other: &O) -> bool {
        self.len() == other.len() && self.head() == other.head() && self.tail() == other.tail()
    }
}

/// Empty `repr(C)` list used as the tail sentinel for [`CStackList`].
///
/// The hidden backing tail preserves prefix-view layout but contributes no
/// logical fields. Construct an owning empty root through [`EmptyList::root_empty`].
///
/// # Examples
/// ```
/// use cel_runtime::{CNil, EmptyList, IntoCStackList, IntoRustTuple, List};
/// let empty = CNil::<()>::root_empty();
/// assert!(empty.is_empty());
/// assert_eq!(empty.push_first("first").into_rust_tuple(), ("first",));
/// assert_eq!(().into_c_stack_list().len(), 0);
/// ```
#[repr(C)]
#[derive(Clone)]
pub struct CNil<T: CStackListHeadLimit>(T);

impl<T: CStackListHeadLimit> EmptyList for CNil<T> {
    type PushFirst<U: 'static> = CStackList<U, CNil<T>>;
    /// Moves a first field into a node backed by this empty sentinel.
    fn push_first<U: 'static>(self, item: U) -> Self::PushFirst<U> {
        CStackList(self, item)
    }

    type RootEmpty = CNil<()>;
    /// Constructs the empty canonical root without backing fields.
    fn root_empty() -> Self::RootEmpty {
        CNil(())
    }
}

impl<T: CStackListHeadLimit, P: ListTypeProperty> ListTypeIteratorAdvance<P> for CNil<T> {
    /// Reports exhaustion without generating a property for the backing tail.
    fn advancer<R: List>(_iter: &mut ListTypeIterator<R, P>) -> Option<P::Output> {
        None
    }
}

impl<P: ListTypeProperty, H: 'static, T: ListTypeIteratorAdvance<P> + CStackListHeadLimit>
    ListTypeIteratorAdvance<P> for CStackList<H, T>
{
    /// Generates this head's property and advances the iterator to its tail.
    ///
    /// - Complexity: The property generator's complexity.
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

    /// Moves empty, singleton, and maximum-arity native tuples without cloning their fields.
    #[test]
    fn native_tuple_bridge_moves_boundary_arities() {
        /// Owns text without providing clone or equality capabilities.
        struct MoveOnly(Box<str>);

        let (): () = ().into_c_stack_list().into_rust_tuple();
        let value = MoveOnly(String::from("singleton").into_boxed_str());
        let pointer = value.0.as_ptr();
        let (value,) = (value,).into_c_stack_list().into_rust_tuple();
        assert_eq!(value.0.as_ptr(), pointer);
        assert_eq!(&*value.0, "singleton");

        let first = MoveOnly(String::from("first").into_boxed_str());
        let last = MoveOnly(String::from("last").into_boxed_str());
        let pointers = (first.0.as_ptr(), last.0.as_ptr());
        let (first, a, b, c, d, e, flag, number, letter, unit, text, last) = (
            first,
            1u8,
            2u16,
            3u32,
            4u64,
            5u128,
            true,
            7f64,
            'x',
            (),
            String::from("middle"),
            last,
        )
            .into_c_stack_list()
            .into_rust_tuple();
        assert_eq!((first.0.as_ptr(), last.0.as_ptr()), pointers);
        assert_eq!((&*first.0, &*last.0), ("first", "last"));
        assert_eq!(
            (a, b, c, d, e, flag, number, letter, unit, text.as_str()),
            (1, 2, 3, 4, 5, true, 7.0, 'x', (), "middle")
        );
    }

    /// Keeps cloned mutable fields disjoint and equality consistent across copies.
    #[test]
    fn clone_and_equality_laws() {
        let mut original = (vec![String::from("first")],).into_c_stack_list();
        let mut copied = original.clone();
        let third = copied.clone();
        assert!(original == original);
        assert!(original == copied && copied == original);
        assert!(copied == third && original == third);
        assert!(!(original != copied));
        copied.1.push(String::from("copy-only"));
        assert_eq!(original.head(), &["first"]);
        original.1[0].push_str("-original");
        assert_eq!(copied.head(), &["first", "copy-only"]);
        assert_eq!(third.head(), &["first"]);
        assert!(original != copied);
    }

    /// Compares list lengths before accessing a potentially absent head.
    #[test]
    fn equality_handles_different_lengths() {
        /// Compares equal to another token but not to an absent value.
        #[derive(Clone, PartialEq)]
        struct Token;

        impl PartialEq<crate::list_traits::Undefined> for Token {
            /// Distinguishes a present token from an empty-list marker.
            fn eq(&self, _: &crate::list_traits::Undefined) -> bool {
                false
            }
        }

        let empty = ().into_c_stack_list();
        let one = (Token,).into_c_stack_list();
        let two = (Token, Token).into_c_stack_list();
        assert!(one != empty);
        assert!(empty != one);
        assert!(two != one);
        assert!(one != two);
        assert!(one == one.clone());
    }

    /// Preserves heterogeneous native-tuple field order, including the empty tuple.
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

    /// Borrows each heterogeneous field at its type-checked source index.
    #[test]
    fn index() {
        let list = (1, 2.5, "Hello").into_c_stack_list();
        assert_eq!(list[U0::new()], 1);
        assert_eq!(list[U1::new()], 2.5);
        assert_eq!(list[U2::new()], "Hello");
    }

    /// Formats a composed borrowed prefix/suffix view without exposing its backing tail.
    #[test]
    fn slice() {
        let list = (1, 2.5, 3, 4, "world", "Hello").into_c_stack_list();
        let view = &list[..U5::new()][U2::new()..];
        assert_eq!(view.len(), 3);
        assert_eq!(format!("{view:?}"), "(3, 4, \"world\")");
        assert_eq!(format!("{:?}", ().into_c_stack_list()), "()");
    }

    /// Indexes a borrowed suffix relative to its new logical beginning.
    #[test]
    fn index_range_from() {
        let list = (1, 2.5, "Hello").into_c_stack_list();
        assert_eq!(list[U1::new()..][U1::new()], "Hello");
    }

    /// Borrows an exclusive prefix without changing the retained fields.
    #[test]
    fn index_range_to() {
        let list = (1, 2.5, "Hello").into_c_stack_list();
        assert_eq!(list[..U2::new()][U1::new()], 2.5);
    }

    /// Resolves each logical field's Rust type from its type-level index.
    #[test]
    fn index_type() {
        use std::any::type_name;

        /// Describes the heterogeneous canonical source list.
        type List = <(i32, f64, &'static str) as IntoList>::Output<CNil<()>>;
        /// Selects the first field's concrete type.
        type Zero = Item<List, U0>;
        /// Selects the second field's concrete type.
        type One = Item<List, U1>;
        /// Selects the third field's concrete type.
        type Two = Item<List, U2>;
        assert_eq!(type_name::<Zero>(), "i32");
        assert_eq!(type_name::<One>(), "f64");
        assert_eq!(type_name::<Two>(), "&str");
    }

    /// Preserves the documented tail-first C representation of owned fields.
    #[test]
    fn cstack_list() {
        let list = CStackList(CNil(()), 32i32).push("Hello").push(42.5);
        // Test that we can cast to a C struct and read values
        /// Mirrors the three fields' public tail-first C layout.
        #[repr(C)]
        struct TestStruct(i32, &'static str, f64);

        let test_struct = unsafe {
            std::mem::transmute::<
                CStackList<f64, CStackList<&str, CStackList<i32, CNil<()>>>>,
                TestStruct,
            >(list)
        };

        assert_eq!(test_struct.0, 32);
        assert_eq!(test_struct.1, "Hello");
        assert_eq!(test_struct.2, 42.5);
    }
}
