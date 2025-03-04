use std::usize;

/**
This module provides type-level list operations using nested tuples.

# Examples

Here's an example of creating a constrained list that requires all elements to be printable:

```rust
use cel_rs::{List, IntoList};

// Define a trait for lists with printable elements
pub trait PrintableList: List {
    type Head;
    type Tail: PrintableList;

    fn print(self, i: usize) -> usize;
}

// Base case: empty list
impl PrintableList for () {
    type Head = ();
    type Tail = ();

    fn print(self, i: usize) -> usize {
        i
    }
}

// Recursive case: head must implement Display
impl<H: std::fmt::Display, T: PrintableList> PrintableList for (H, T) {
    type Head = H;
    type Tail = T;

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

// IntoList is now sealed and cannot be implemented outside this module
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

impl<A> IntoList for (A,) {
    type Result = (A, ());

    fn into_list(self) -> Self::Result {
        (self.0, ())
    }
}

impl<A, B> IntoList for (A, B) {
    type Result = (A, (B, ()));

    fn into_list(self) -> Self::Result {
        (self.0, (self.1, ()))
    }
}

impl<A, B, C> IntoList for (A, B, C) {
    type Result = (A, (B, (C, ())));

    fn into_list(self) -> Self::Result {
        (self.0, (self.1, (self.2, ())))
    }
}

impl<A, B, C, D> IntoList for (A, B, C, D) {
    type Result = (A, (B, (C, (D, ()))));

    fn into_list(self) -> Self::Result {
        (self.0, (self.1, (self.2, (self.3, ()))))
    }
}

impl<A, B, C, D, E> IntoList for (A, B, C, D, E) {
    type Result = (A, (B, (C, (D, (E, ())))));

    fn into_list(self) -> Self::Result {
        (self.0, (self.1, (self.2, (self.3, (self.4, ())))))
    }
}

impl<A, B, C, D, E, F> IntoList for (A, B, C, D, E, F) {
    type Result = (A, (B, (C, (D, (E, (F, ()))))));

    fn into_list(self) -> Self::Result {
        (self.0, (self.1, (self.2, (self.3, (self.4, (self.5, ()))))))
    }
}

impl<A, B, C, D, E, F, G> IntoList for (A, B, C, D, E, F, G) {
    type Result = (A, (B, (C, (D, (E, (F, (G, ())))))));

    fn into_list(self) -> Self::Result {
        (
            self.0,
            (self.1, (self.2, (self.3, (self.4, (self.5, (self.6, ())))))),
        )
    }
}

impl<A, B, C, D, E, F, G, H> IntoList for (A, B, C, D, E, F, G, H) {
    type Result = (A, (B, (C, (D, (E, (F, (G, (H, ()))))))));

    fn into_list(self) -> Self::Result {
        (
            self.0,
            (
                self.1,
                (self.2, (self.3, (self.4, (self.5, (self.6, (self.7, ())))))),
            ),
        )
    }
}
impl<A, B, C, D, E, F, G, H, I> IntoList for (A, B, C, D, E, F, G, H, I) {
    type Result = (A, (B, (C, (D, (E, (F, (G, (H, (I, ())))))))));

    fn into_list(self) -> Self::Result {
        (
            self.0,
            (
                self.1,
                (
                    self.2,
                    (self.3, (self.4, (self.5, (self.6, (self.7, (self.8, ())))))),
                ),
            ),
        )
    }
}
impl<A, B, C, D, E, F, G, H, I, J> IntoList for (A, B, C, D, E, F, G, H, I, J) {
    type Result = (A, (B, (C, (D, (E, (F, (G, (H, (I, (J, ()))))))))));

    fn into_list(self) -> Self::Result {
        (
            self.0,
            (
                self.1,
                (
                    self.2,
                    (
                        self.3,
                        (self.4, (self.5, (self.6, (self.7, (self.8, (self.9, ())))))),
                    ),
                ),
            ),
        )
    }
}
impl<A, B, C, D, E, F, G, H, I, J, K> IntoList for (A, B, C, D, E, F, G, H, I, J, K) {
    type Result = (A, (B, (C, (D, (E, (F, (G, (H, (I, (J, (K, ())))))))))));

    fn into_list(self) -> Self::Result {
        (
            self.0,
            (
                self.1,
                (
                    self.2,
                    (
                        self.3,
                        (
                            self.4,
                            (
                                self.5,
                                (self.6, (self.7, (self.8, (self.9, (self.10, ()))))),
                            ),
                        ),
                    ),
                ),
            ),
        )
    }
}
impl<A, B, C, D, E, F, G, H, I, J, K, L> IntoList for (A, B, C, D, E, F, G, H, I, J, K, L) {
    type Result = (A, (B, (C, (D, (E, (F, (G, (H, (I, (J, (K, (L, ()))))))))))));

    fn into_list(self) -> Self::Result {
        (
            self.0,
            (
                self.1,
                (
                    self.2,
                    (
                        self.3,
                        (
                            self.4,
                            (
                                self.5,
                                (
                                    self.6,
                                    (self.7, (self.8, (self.9, (self.10, (self.11, ()))))),
                                ),
                            ),
                        ),
                    ),
                ),
            ),
        )
    }
}

pub trait List {
    type Head;
    type Tail: List;
    const LENGTH: usize;
}

impl List for () {
    type Head = ();
    type Tail = ();
    const LENGTH: usize = 0;
}

impl<T, U> List for (T, U)
where
    U: List,
{
    type Head = T;
    type Tail = U;
    const LENGTH: usize = U::LENGTH + 1;
}

pub trait Concat<U: List> {
    type Result: List;

    fn concat(self, other: U) -> Self::Result;
}

// Base case: concatenating with empty list
impl<U: List> Concat<U> for () {
    type Result = U;

    fn concat(self, other: U) -> Self::Result {
        other
    }
}

// Recursive case: (H, T) + U = (H, (T + U))
impl<H, T: List, U: List> Concat<U> for (H, T)
where
    T: Concat<U>,
{
    type Result = (H, <T as Concat<U>>::Result);

    fn concat(self, other: U) -> Self::Result {
        (self.0, <T as Concat<U>>::concat(self.1, other))
    }
}

pub trait Reverse {
    type Result: List;

    fn reverse(self) -> Self::Result;
}

// Base case: empty list reverses to itself
impl Reverse for () {
    type Result = ();

    fn reverse(self) -> Self::Result {
        self
    }
}

// Recursive case: reverse (H, T) = reverse(T) + (H, ())
impl<H, T: List> Reverse for (H, T)
where
    T: Reverse,
    <T as Reverse>::Result: Concat<(H, ())>,
{
    type Result = <<T as Reverse>::Result as Concat<(H, ())>>::Result;

    fn reverse(self) -> Self::Result {
        <T as Reverse>::reverse(self.1).concat((self.0, ()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::TypeId;

    trait Eq<U: List> {
        fn equal() -> bool;
    }

    impl<U: List + 'static> Eq<U> for () {
        fn equal() -> bool {
            TypeId::of::<U>() == TypeId::of::<()>()
        }
    }

    impl<H1: 'static, T1: List + 'static, U: List + 'static> Eq<U> for (H1, T1)
    where
        U: List,
    {
        fn equal() -> bool {
            TypeId::of::<(H1, T1)>() == TypeId::of::<U>()
        }
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
        type Concat = <L1 as crate::Concat<L2>>::Result;

        // Should be (i32, (f64, (bool, (char, ()))))
        type Expected = (i32, (f64, (bool, (char, ()))));
        assert!(<Concat as Eq<Expected>>::equal());
    }

    #[test]
    fn test_type_list_reverse() {
        type List = <(i32, f64, String) as crate::IntoList>::Result;
        type Expected = (i32, (f64, (String, ())));
        assert!(<List as Eq<Expected>>::equal());
    }

    #[test]
    fn test_type_list_reverse_types() {
        // Original: (i32, (f64, (bool, ())))
        type List = (i32, (f64, (bool, ())));
        // Should become: (bool, (f64, (i32, ())))
        type Reversed = <List as Reverse>::Result;
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
