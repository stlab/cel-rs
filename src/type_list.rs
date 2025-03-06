/*!
This module provides type list operations using nested tuples.

# Examples

Here's an example of creating a constrained list that requires all elements to be printable:

```rust
use cel_rs::type_list::{List, IntoList};

// Define a trait for lists with printable elements
pub trait PrintableList: List {
    fn print(self, i: usize) -> usize;
}

// Base case: empty list
impl PrintableList for () {
    fn print(self, i: usize) -> usize {
        i
    }
}

// Recursive case: head must implement Display
impl<H: std::fmt::Display, T: PrintableList> PrintableList for (H, T) {
    fn print(self, i: usize) -> usize {
        println!("{}", self.0);
        self.1.print(i + 1)
    }
}

// Example usage
let list = ("hello", (42.5, (true, ()))); // Create a list directly
list.print(0); // Prints each element

// Or use IntoList to convert from a tuple
let list = ("world", 123, false).into_list();
list.print(0);
```

This example shows how to create a constrained list where each element must implement
a specific trait (in this case `Display`). The trait provides a recursive operation
that processes each element in the list.
*/

/**
Converts a tuple into a type list.

IntoList is implemented for tuples up to 12 elements.

# Examples

```rust
use cel_rs::type_list::{List, IntoList};

let list = (1, "hello", 3.14).into_list();
println!("{:?}", list);
```

You can also use the `IntoList` trait to convert a tuple type into a type list.

```rust
# use cel_rs::type_list::{List, IntoList};
type ListType = <(i32, f64, bool) as IntoList>::Result;
let list: ListType = (1, (2.0, (true, ())));
```
*/
pub trait IntoList {
    type Result: List;

    fn into_list(self) -> Self::Result;
}

impl IntoList for () {
    type Result = ();

    fn into_list(self) -> Self::Result {
        self
    }
}

impl<A: 'static> IntoList for (A,) {
    type Result = (A, ());

    fn into_list(self) -> Self::Result {
        (self.0, ())
    }
}

impl<A: 'static, B: 'static> IntoList for (A, B) {
    type Result = (A, (B, ()));

    fn into_list(self) -> Self::Result {
        (self.0, (self.1, ()))
    }
}

impl<A: 'static, B: 'static, C: 'static> IntoList for (A, B, C) {
    type Result = (A, (B, (C, ())));

    fn into_list(self) -> Self::Result {
        (self.0, (self.1, (self.2, ())))
    }
}

impl<A: 'static, B: 'static, C: 'static, D: 'static> IntoList for (A, B, C, D) {
    type Result = (A, (B, (C, (D, ()))));

    fn into_list(self) -> Self::Result {
        (self.0, (self.1, (self.2, (self.3, ()))))
    }
}

impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static> IntoList for (A, B, C, D, E) {
    type Result = (A, (B, (C, (D, (E, ())))));

    fn into_list(self) -> Self::Result {
        (self.0, (self.1, (self.2, (self.3, (self.4, ())))))
    }
}

impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static, F: 'static> IntoList
    for (A, B, C, D, E, F)
{
    type Result = (A, (B, (C, (D, (E, (F, ()))))));

    fn into_list(self) -> Self::Result {
        (self.0, (self.1, (self.2, (self.3, (self.4, (self.5, ()))))))
    }
}

#[rustfmt::skip]
impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static, F: 'static, G: 'static> IntoList for (A, B, C, D, E, F, G) {
    type Result = (A, (B, (C, (D, (E, (F, (G, ())))))));

    fn into_list(self) -> Self::Result {
        (self.0, (self.1, (self.2, (self.3, (self.4, (self.5, (self.6, ())))))))
    }
}

#[rustfmt::skip]
impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static, F: 'static, G: 'static, H: 'static> IntoList for (A, B, C, D, E, F, G, H) {
    type Result = (A, (B, (C, (D, (E, (F, (G, (H, ()))))))));

    fn into_list(self) -> Self::Result {
        ( self.0, ( self.1, ( self.2, ( self.3, ( self.4, ( self.5, ( self.6,
            ( self.7, ()))))))))
    }
}

#[rustfmt::skip]
impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static, F: 'static, G: 'static, H: 'static, I: 'static> IntoList for (A, B, C, D, E, F, G, H, I) {
    type Result = (A, (B, (C, (D, (E, (F, (G, (H, (I, ())))))))));

    fn into_list(self) -> Self::Result {
        ( self.0, ( self.1, ( self.2, ( self.3, ( self.4, ( self.5, ( self.6,
            ( self.7, ( self.8, ())))))))))
    }
}

#[rustfmt::skip]
impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static, F: 'static, G: 'static, H: 'static, I: 'static, J: 'static> IntoList for (A, B, C, D, E, F, G, H, I, J) {
    type Result = (A, (B, (C, (D, (E, (F, (G, (H, (I, (J, ()))))))))));

    fn into_list(self) -> Self::Result {
        ( self.0, ( self.1, ( self.2, ( self.3, ( self.4, ( self.5, ( self.6,
            ( self.7, ( self.8, ( self.9, ()))))))))))
    }
}

#[rustfmt::skip]
impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static, F: 'static, G: 'static, H: 'static, I: 'static, J: 'static, K: 'static> IntoList for (A, B, C, D, E, F, G, H, I, J, K) {
    type Result = (A, (B, (C, (D, (E, (F, (G, (H, (I, (J, (K, ())))))))))));

    fn into_list(self) -> Self::Result {
        ( self.0, ( self.1, ( self.2, ( self.3, ( self.4, ( self.5, ( self.6,
            ( self.7, ( self.8, ( self.9, ( self.10, ())))))))))))
    }
}

#[rustfmt::skip]
impl<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static, F: 'static, G: 'static,
        H: 'static, I: 'static, J: 'static, K: 'static, L: 'static>
    IntoList for (A, B, C, D, E, F, G, H, I, J, K, L) {
    type Result = (A, (B, (C, (D, (E, (F, (G, (H, (I, (J, (K, (L, ()))))))))))));

    fn into_list(self) -> Self::Result {
        (self.0, (self.1, (self.2, (self.3, (self.4, (self.5, (self.6, (self.7, (self.8,
            (self.9, (self.10, (self.11, ()))))))))))))
    }
}

pub trait TypeHandler {
    fn handle<T: 'static>(self: &mut Self);
}

/**
Represents a type list with a head element and a tail.
*/
pub trait List {
    type Head;
    type Tail: List;
    const LENGTH: usize;

    // Add associated type for concatenation result
    type Concat<U: List>: List;
    type Reverse: List;

    fn for_each_type<H: TypeHandler>(handler: &mut H);

    fn concat<U: List>(self, other: U) -> Self::Concat<U>;

    fn reverse(self) -> Self::Reverse;
}

// Specialize for empty list
impl List for () {
    type Head = ();
    type Tail = ();
    const LENGTH: usize = 0;
    type Concat<U: List> = U;
    type Reverse = ();

    fn for_each_type<H: TypeHandler>(_handler: &mut H) {}

    fn concat<U: List>(self, other: U) -> U {
        other
    }

    fn reverse(self) -> Self::Reverse {
        self
    }
}

// Implement for non-empty lists
impl<T: 'static, U: List> List for (T, U)
where
    U: List,
{
    type Head = T;
    type Tail = U;
    const LENGTH: usize = U::LENGTH + 1;
    type Concat<V: List> = (T, U::Concat<V>);
    type Reverse = <U::Reverse as List>::Concat<(T, ())>;

    fn for_each_type<H: TypeHandler>(handler: &mut H) {
        handler.handle::<Self::Head>();
        Self::Tail::for_each_type(handler);
    }

    fn concat<V: List>(self, other: V) -> Self::Concat<V> {
        (self.0, self.1.concat(other))
    }

    fn reverse(self) -> Self::Reverse {
        self.1.reverse().concat((self.0, ()))
    }
}

#[cfg(test)]
mod tests {
    use crate::type_list::*;
    use std::any::TypeId;

    trait Eq<U: List> {
        fn equal() -> bool;
    }

    impl<U: List + 'static> Eq<U> for () {
        fn equal() -> bool {
            TypeId::of::<U>() == TypeId::of::<()>()
        }
    }

    impl<H1: 'static, T1: List + 'static, U: List + 'static> Eq<U> for (H1, T1) {
        fn equal() -> bool {
            TypeId::of::<(H1, T1)>() == TypeId::of::<U>()
        }
    }

    #[test]
    fn test_list_length() {
        type L0 = <() as IntoList>::Result;
        type L1 = <(i32,) as IntoList>::Result;
        type L2 = <(i32, f64) as IntoList>::Result;
        type L3 = <(f64, i16, bool) as IntoList>::Result;

        assert_eq!(<() as List>::LENGTH, 0);
        assert_eq!(L0::LENGTH, 0);
        assert_eq!(L1::LENGTH, 1);
        assert_eq!(L2::LENGTH, 2);
        assert_eq!(L3::LENGTH, 3);
    }

    #[test]
    fn test_type_list_eq() {
        type L1 = (i32, (f64, ()));
        type L2 = (i32, (f64, ()));
        type L3 = (f64, (i32, ()));

        assert!(<L1 as Eq<L2>>::equal());
        assert!(!<L1 as Eq<L3>>::equal());
        assert!(<() as Eq<()>>::equal());
    }

    #[test]
    fn test_type_list_concat() {
        type L1 = (i32, (f64, ()));
        type L2 = (bool, (char, ()));
        type Combined = <L1 as List>::Concat<L2>;

        // Should be (i32, (f64, (bool, (char, ()))))
        type Expected = (i32, (f64, (bool, (char, ()))));
        assert!(<Combined as Eq<Expected>>::equal());
    }

    #[test]
    fn test_type_list_reverse() {
        type L = <(i32, f64, String) as IntoList>::Result;
        type Expected = (i32, (f64, (String, ())));
        assert!(<L as Eq<Expected>>::equal());
    }

    #[test]
    fn test_type_list_reverse_types() {
        // Original: (i32, (f64, (bool, ())))
        type L = (i32, (f64, (bool, ())));
        // Should become: (bool, (f64, (i32, ())))
        type Reversed = <L as List>::Reverse;
        type Expected = (bool, (f64, (i32, ())));
        assert!(<Reversed as Eq<Expected>>::equal());
    }

    #[test]
    fn test_tuple_list_layout_equivalency() {
        // Test layout equivalence for tuples of different sizes
        assert_eq!((1, "hello", 3.14).into_list(), (1, ("hello", (3.14, ()))));
    }

    #[test]
    fn test_into_list() {
        let tuple = (1, "hello", 3.14);
        let list = tuple.into_list();
        assert_eq!(list.0, 1);
        assert_eq!((list.1).0, "hello");
        assert_eq!(((list.1).1).0, 3.14);
    }

    #[test]
    fn test_into_list_ownership() {
        let s = String::from("hello");
        let tuple = (1, s, 3.14); // s moved into tuple
        let list = tuple.into_list(); // tuple moved into list
        assert_eq!((list.1).0, "hello"); // String still valid in list
    }
}
