//! Tests native tuple bridges through their public, source-ordered interface.

use cel_runtime::{IntoCStackList, IntoRustTuple, List};

/// Holds a value without clone, equality, or debug capabilities.
struct MoveOnly(String);

/// Stores an over-aligned value without optional trait capabilities.
#[repr(align(64))]
struct Aligned(u8);

/// Verifies that heterogeneous fields retain their source order.
#[test]
fn round_trip_preserves_source_order() {
    let list = (String::from("left"), 7_u8, 2.5_f64).into_c_stack_list();
    let value: (String, u8, f64) = list.into_rust_tuple();
    assert_eq!(value, (String::from("left"), 7, 2.5));
}

/// Verifies that conversion moves owned fields without requiring extra traits.
#[test]
fn round_trip_moves_fields() {
    let value = MoveOnly(String::from("owned"));
    let allocation = value.0.as_ptr();
    let (value,) = (value,).into_c_stack_list().into_rust_tuple();
    assert_eq!(value.0, "owned");
    assert_eq!(value.0.as_ptr(), allocation);
}

/// Verifies that nested lists remain fields rather than being flattened.
#[test]
fn round_trip_preserves_nested_lists() {
    let inner = (MoveOnly(String::from("inner")), 9_u16).into_c_stack_list();
    let (inner, last) = (inner, 3_u8).into_c_stack_list().into_rust_tuple();
    let (first, second) = inner.into_rust_tuple();
    assert_eq!(first.0, "inner");
    assert_eq!(second, 9);
    assert_eq!(last, 3);
}

/// Verifies the empty native tuple bridge.
#[test]
fn round_trip_empty() {
    let value: () = ().into_c_stack_list().into_rust_tuple();
    assert_eq!(value, ());
}

/// Verifies singleton tuples retain their tuple shape.
#[test]
fn round_trip_singleton() {
    let value: (u16,) = (42_u16,).into_c_stack_list().into_rust_tuple();
    assert_eq!(value, (42,));
}

/// Verifies every multi-field native tuple arity through twelve.
#[test]
fn round_trip_native_arities() {
    /// Checks that a native tuple retains all fields in source order.
    macro_rules! check {
        ($($value:expr),+) => {
            assert_eq!(
                ($($value,)+).into_c_stack_list().into_rust_tuple(),
                ($($value,)+),
            );
        };
    }
    check!(1, 2);
    check!(1, 2, 3);
    check!(1, 2, 3, 4);
    check!(1, 2, 3, 4, 5);
    check!(1, 2, 3, 4, 5, 6);
    check!(1, 2, 3, 4, 5, 6, 7);
    check!(1, 2, 3, 4, 5, 6, 7, 8);
    check!(1, 2, 3, 4, 5, 6, 7, 8, 9);
    check!(1, 2, 3, 4, 5, 6, 7, 8, 9, 10);
    check!(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11);
    check!(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12);
}

/// Verifies conversion supports over-aligned heads and tails.
#[test]
fn round_trip_over_aligned() {
    let (first, middle, last) = (Aligned(1), 2_u8, Aligned(3))
        .into_c_stack_list()
        .into_rust_tuple();
    assert_eq!((first.0, middle, last.0), (1, 2, 3));
}

/// Verifies zero-sized fields retain their logical positions.
#[test]
fn round_trip_zero_sized() {
    let value = ((), 1_u8, (), 2_u16, ())
        .into_c_stack_list()
        .into_rust_tuple();
    assert_eq!(value, ((), 1, (), 2, ()));
}

/// Verifies the native tuple arity limit does not limit list construction.
#[test]
fn lists_can_exceed_twelve_fields() {
    let list = (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12)
        .into_c_stack_list()
        .push(0);
    assert_eq!(list.len(), 13);
    assert_eq!(*list.head(), 0);
    assert_eq!(
        list.0.into_rust_tuple(),
        (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12),
    );
}
