//! Prepared formatting contracts over canonical whole-value ownership.

use cel_runtime::{Array, ArrayOf, Cons, IntoCStackList, Leaf, Nil, OwnedValue, ValueType};

/// Formatting preparation binds recursive capabilities, not a container wrapper.
#[test]
fn prepared_formatter_handles_nested_shapes_and_rejects_mismatches() {
    type Shape = Cons<ArrayOf<Cons<Leaf<i32>, Nil>>, Cons<ArrayOf<Leaf<i32>>, Nil>>;
    let integer = ValueType::leaf::<i32>().with_debug::<Leaf<i32>>().unwrap();
    let singleton = ValueType::tuple(vec![cel_runtime::AssociatedType {
        offset: 0,
        value_type: integer.clone(),
    }])
    .unwrap();
    let descriptor = ValueType::tuple(vec![
        cel_runtime::AssociatedType {
            offset: 0,
            value_type: ValueType::array(singleton),
        },
        cel_runtime::AssociatedType {
            offset: 0,
            value_type: ValueType::array(integer),
        },
    ])
    .unwrap();
    let value = OwnedValue::new::<Shape>(
        (
            Array::from_vec(vec![(7,).into_c_stack_list()]),
            Array::from_vec(vec![]),
        )
            .into_c_stack_list(),
    )
    .with_value_type(descriptor)
    .unwrap();
    let formatter = value.prepare_debug().unwrap();
    assert_eq!(formatter.format(&value).unwrap(), "([(7,)], [])");
    assert!(formatter.format(&OwnedValue::new::<Leaf<i32>>(7)).is_err());
    assert_eq!(formatter.format(&value).unwrap(), "([(7,)], [])");
}

/// Missing formatting is reported without requiring clone or equality.
#[test]
fn prepared_formatter_requires_only_debug_capability() {
    assert!(OwnedValue::new::<Leaf<i32>>(7).prepare_debug().is_err());
    let value = OwnedValue::new::<Leaf<i32>>(7)
        .with_value_type(ValueType::leaf::<i32>().with_debug::<Leaf<i32>>().unwrap())
        .unwrap();
    assert!(value.try_clone().is_err());
    assert!(value.try_eq(&value).is_err());
    assert_eq!(value.prepare_debug().unwrap().format(&value).unwrap(), "7");
}
