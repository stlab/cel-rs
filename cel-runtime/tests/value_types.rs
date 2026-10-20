//! Verifies complete descriptor identities and whole-value ownership contracts.

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use cel_runtime::owned_value::OwnedValue;
use cel_runtime::value_type::{
    ArrayOf, AssociatedType, Cons, Leaf, Nil, ValueKind, ValueRepr, ValueType,
};
use cel_runtime::{Array, IntoCStackList, IntoRustTuple};

/// Represents an array of source-ordered pairs with nested array fields.
type Rows = ArrayOf<Cons<Leaf<i32>, Cons<ArrayOf<Leaf<f64>>, Nil>>>;

/// Checks recursive identities rather than aggregate carriers or total layout.
#[test]
fn complete_shapes_authorize_typed_moves() {
    let rows = Array::from_vec(vec![(7, Array::from_vec(vec![2.5])).into_c_stack_list()]);
    let pointer = rows.as_slice().as_ptr();
    let owner = OwnedValue::new::<Rows>(rows);
    let descriptor = owner.value_type();
    assert!(descriptor.same_shape(&Rows::value_type()));
    assert!(
        !descriptor
            .same_shape(&ArrayOf::<Cons<Leaf<u32>, Cons<ArrayOf<Leaf<f64>>, Nil>>>::value_type())
    );
    assert!(!descriptor.same_shape(&ArrayOf::<ArrayOf<Leaf<i32>>>::value_type()));
    let rows = owner.try_into_typed::<Rows>().unwrap();
    assert_eq!(rows.as_slice().as_ptr(), pointer);
    let (number, nested) = rows.into_vec().pop().unwrap().into_rust_tuple();
    assert_eq!(number, 7);
    assert_eq!(nested.as_slice(), &[2.5]);
}

/// Verifies canonical unit is structural and never conflated with an opaque host unit.
#[test]
fn canonical_nil_and_host_unit_are_distinct() {
    let canonical = Nil::value_type();
    assert!(matches!(canonical.kind(), ValueKind::Tuple(fields) if fields.is_empty()));
    assert!(!canonical.same_shape(&ValueType::leaf::<()>()));
    assert!(
        OwnedValue::new::<Nil>(().into_c_stack_list())
            .try_into_typed::<Leaf<()>>()
            .is_err()
    );
}

/// Verifies tuple constructors replace untrusted offsets with canonical source offsets.
#[test]
fn dynamic_tuple_matches_witness() {
    let tuple = ValueType::tuple(vec![
        AssociatedType {
            offset: usize::MAX,
            value_type: ValueType::leaf::<u8>(),
        },
        AssociatedType {
            offset: usize::MAX,
            value_type: ValueType::leaf::<u64>(),
        },
    ])
    .unwrap();
    assert!(tuple.same_shape(&Cons::<Leaf<u8>, Cons<Leaf<u64>, Nil>>::value_type()));
    assert!(
        matches!(tuple.kind(), ValueKind::Tuple(fields) if fields[0].offset == 8 && fields[1].offset == 0)
    );
}

/// Counts destruction without implementing optional capabilities.
struct DropOnly(Rc<Cell<usize>>);

impl Drop for DropOnly {
    /// Records exactly one destruction.
    fn drop(&mut self) {
        self.0.set(self.0.get().checked_add(1).unwrap());
    }
}

/// Verifies container ownership neither requests nor silently grants capabilities.
#[test]
fn drop_only_containers_move_and_drop() {
    type Pair = Cons<Leaf<DropOnly>, Cons<ArrayOf<Leaf<DropOnly>>, Nil>>;
    let drops = Rc::new(Cell::new(0));
    let pair = (
        DropOnly(drops.clone()),
        Array::from_vec(vec![DropOnly(drops.clone())]),
    )
        .into_c_stack_list();
    let owner = OwnedValue::new::<ArrayOf<Pair>>(Array::from_vec(vec![pair]));
    assert!(owner.try_clone().is_err());
    assert!(owner.try_eq(&owner).is_err());
    assert!(owner.try_debug().is_err());
    assert_eq!(drops.get(), 0);
    let moved = owner.try_into_typed::<ArrayOf<Pair>>().unwrap();
    assert_eq!(drops.get(), 0);
    drop(moved);
    assert_eq!(drops.get(), 2);
}

/// Verifies a failed extraction destroys the consumed owner without invoking host code.
#[test]
fn mismatch_drops_once_before_supplied_operation() {
    let drops = Rc::new(Cell::new(0));
    let invoked = Cell::new(false);
    let result =
        OwnedValue::new::<ArrayOf<Leaf<DropOnly>>>(Array::from_vec(vec![DropOnly(drops.clone())]))
            .try_into_typed::<ArrayOf<Leaf<i32>>>()
            .map(|_| invoked.set(true));
    assert!(result.is_err());
    assert!(!invoked.get());
    assert_eq!(drops.get(), 1);
}

/// Verifies a same-layout but unrelated witness cannot install any typed adapter.
#[test]
fn capability_installation_checks_complete_shape() {
    let descriptor = ArrayOf::<Leaf<i32>>::value_type();
    assert!(
        descriptor
            .clone()
            .with_clone::<ArrayOf<Leaf<u32>>>()
            .is_err()
    );
    assert!(
        descriptor
            .clone()
            .with_equality::<ArrayOf<Leaf<u32>>>()
            .is_err()
    );
    assert!(descriptor.with_debug::<ArrayOf<Leaf<u32>>>().is_err());
    let nested = ArrayOf::<Cons<Leaf<i32>, Nil>>::value_type();
    assert!(nested.with_clone::<ArrayOf<Leaf<i32>>>().is_err());
}

/// Verifies equality and formatting do not implicitly require or enable cloning.
#[test]
fn independent_capabilities_and_checked_descriptor_attachment() {
    let descriptor = Leaf::<i32>::value_type()
        .with_equality::<Leaf<i32>>()
        .unwrap()
        .with_debug::<Leaf<i32>>()
        .unwrap();
    let owner = OwnedValue::new::<Leaf<i32>>(42)
        .with_value_type(descriptor)
        .unwrap();
    assert!(owner.try_clone().is_err());
    assert!(owner.try_eq(&OwnedValue::new::<Leaf<i32>>(42)).unwrap());
    assert_eq!(owner.try_debug().unwrap(), "42");
    assert!(
        OwnedValue::new::<Leaf<u32>>(42)
            .with_value_type(ValueType::cloneable_leaf::<i32>())
            .is_err()
    );
}

/// Tracks clone cleanup, optionally panicking instead of creating the next value.
struct PanicClone {
    drops: Rc<Cell<usize>>,
    panic: bool,
}

impl Clone for PanicClone {
    /// Clones a tracked value unless this leaf requests an unwind.
    fn clone(&self) -> Self {
        assert!(!self.panic, "deliberate clone panic");
        Self {
            drops: self.drops.clone(),
            panic: false,
        }
    }
}

impl Drop for PanicClone {
    /// Records destruction of original and successfully cloned leaves.
    fn drop(&mut self) {
        self.drops.set(self.drops.get().checked_add(1).unwrap());
    }
}

/// Verifies recursive cloning cleans a completed array prefix and partial tuple.
#[test]
fn recursive_clone_unwind_cleans_initialized_prefix() {
    type Pair = Cons<ArrayOf<Leaf<PanicClone>>, Cons<Leaf<PanicClone>, Nil>>;
    let drops = Rc::new(Cell::new(0));
    let value = (
        Array::from_vec(vec![PanicClone {
            drops: drops.clone(),
            panic: false,
        }]),
        PanicClone {
            drops: drops.clone(),
            panic: true,
        },
    )
        .into_c_stack_list();
    let descriptor = ValueType::tuple(vec![
        AssociatedType {
            offset: 0,
            value_type: ValueType::array(ValueType::cloneable_leaf::<PanicClone>()),
        },
        AssociatedType {
            offset: 0,
            value_type: ValueType::cloneable_leaf::<PanicClone>(),
        },
    ])
    .unwrap();
    let owner = OwnedValue::new::<Pair>(value)
        .with_value_type(descriptor)
        .unwrap();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| owner.try_clone())).is_err());
    assert_eq!(drops.get(), 1);
    drop(owner);
    assert_eq!(drops.get(), 3);
}

/// Verifies a completed nested prefix is cleaned when a later array element panics.
#[test]
fn array_clone_unwind_cleans_initialized_elements() {
    let drops = Rc::new(Cell::new(0));
    let array = Array::from_vec(vec![
        PanicClone {
            drops: drops.clone(),
            panic: false,
        },
        PanicClone {
            drops: drops.clone(),
            panic: true,
        },
    ]);
    let owner = OwnedValue::new::<ArrayOf<Leaf<PanicClone>>>(array)
        .with_value_type(ValueType::array(ValueType::cloneable_leaf::<PanicClone>()))
        .unwrap();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| owner.try_clone())).is_err());
    assert_eq!(drops.get(), 1);
    drop(owner);
    assert_eq!(drops.get(), 3);
}

/// Verifies recursive cloning preserves contents while owning independent allocations.
#[test]
fn recursive_clone_preserves_values() {
    type Values = ArrayOf<Cons<Leaf<String>, Nil>>;
    let descriptor = ValueType::array(
        ValueType::tuple(vec![AssociatedType {
            offset: 0,
            value_type: ValueType::cloneable_leaf::<String>(),
        }])
        .unwrap(),
    );
    let owner = OwnedValue::new::<Values>(Array::from_vec(vec![
        (String::from("leaf"),).into_c_stack_list(),
    ]))
    .with_value_type(descriptor)
    .unwrap();
    let clone = owner
        .try_clone()
        .unwrap()
        .try_into_typed::<Values>()
        .unwrap();
    let original = owner.try_into_typed::<Values>().unwrap();
    assert_ne!(clone.as_slice().as_ptr(), original.as_slice().as_ptr());
    assert_eq!(clone.into_vec().pop().unwrap().into_rust_tuple().0, "leaf");
}

/// Verifies independent recursive equality and formatting use complete child plans.
#[test]
fn recursive_equality_and_formatting() {
    type Values = ArrayOf<Cons<Leaf<i32>, Cons<ArrayOf<Leaf<i32>>, Nil>>>;
    let leaf = ValueType::leaf::<i32>()
        .with_equality::<Leaf<i32>>()
        .unwrap()
        .with_debug::<Leaf<i32>>()
        .unwrap();
    let descriptor = ValueType::array(
        ValueType::tuple(vec![
            AssociatedType {
                offset: 0,
                value_type: leaf.clone(),
            },
            AssociatedType {
                offset: 0,
                value_type: ValueType::array(leaf),
            },
        ])
        .unwrap(),
    );
    let a = OwnedValue::new::<Values>(Array::from_vec(vec![
        (1, Array::from_vec(vec![2, 3])).into_c_stack_list(),
    ]))
    .with_value_type(descriptor)
    .unwrap();
    let equal = OwnedValue::new::<Values>(Array::from_vec(vec![
        (1, Array::from_vec(vec![2, 3])).into_c_stack_list(),
    ]));
    let different = OwnedValue::new::<Values>(Array::from_vec(vec![
        (1, Array::from_vec(vec![2, 4])).into_c_stack_list(),
    ]));
    assert!(a.try_eq(&equal).unwrap());
    assert!(!a.try_eq(&different).unwrap());
    assert!(a.try_clone().is_err());
    assert_eq!(a.try_debug().unwrap(), "[(1, [2, 3])]");
    assert!(a.try_eq(&OwnedValue::new::<Leaf<i32>>(1)).is_err());
}

/// Verifies empty arrays retain recursive shape and source unit is cloneable.
#[test]
fn empty_nested_arrays_and_source_unit() {
    type Empty = ArrayOf<ArrayOf<Leaf<String>>>;
    let owner = OwnedValue::new::<Empty>(Array::from_vec(vec![Array::from_vec(vec![])]))
        .with_value_type(ValueType::array(ValueType::array(
            ValueType::cloneable_leaf::<String>(),
        )))
        .unwrap();
    let cloned = owner
        .try_clone()
        .unwrap()
        .try_into_typed::<Empty>()
        .unwrap();
    assert!(cloned.as_slice()[0].is_empty());
    assert_eq!(cloned.len(), 1);
    let nil = OwnedValue::new::<Nil>(().into_c_stack_list());
    assert!(nil.try_clone().unwrap().try_into_typed::<Nil>().is_ok());
    assert!(nil.try_eq(&nil).unwrap());
    assert_eq!(nil.try_debug().unwrap(), "()");
}

/// Counts destruction of zero-sized leaves without owning any nonzero storage.
#[derive(Clone)]
struct ZeroSized;

thread_local! {
    /// Counts only the current test thread's zero-sized destructor calls.
    static ZST_DROPS: Cell<usize> = const { Cell::new(0) };
}

impl Drop for ZeroSized {
    /// Records one logical zero-sized value's destruction.
    fn drop(&mut self) {
        ZST_DROPS.with(|count| count.set(count.get().checked_add(1).unwrap()));
    }
}

/// Verifies zero-sized elements retain their logical counts across clone and move.
#[test]
fn zero_sized_destructors_are_not_lost_or_duplicated() {
    ZST_DROPS.with(|count| count.set(0));
    let owner = OwnedValue::new::<ArrayOf<Leaf<ZeroSized>>>(Array::from_vec(vec![
        ZeroSized, ZeroSized, ZeroSized,
    ]))
    .with_value_type(ValueType::array(ValueType::cloneable_leaf::<ZeroSized>()))
    .unwrap();
    let cloned = owner.try_clone().unwrap();
    drop(owner.try_into_typed::<ArrayOf<Leaf<ZeroSized>>>().unwrap());
    ZST_DROPS.with(|count| assert_eq!(count.get(), 3));
    drop(cloned);
    ZST_DROPS.with(|count| assert_eq!(count.get(), 6));
}

/// Exercises over-aligned fields with owned data and trailing padding.
#[repr(align(128))]
#[derive(Clone)]
struct Aligned(String);

/// Verifies erased clone, drop, and move honor concrete alignment and padding.
#[test]
fn over_aligned_owned_fields_round_trip() {
    type Pair = Cons<Leaf<u8>, Cons<Leaf<Aligned>, Nil>>;
    let descriptor = ValueType::tuple(vec![
        AssociatedType {
            offset: 0,
            value_type: ValueType::cloneable_leaf::<u8>(),
        },
        AssociatedType {
            offset: 0,
            value_type: ValueType::cloneable_leaf::<Aligned>(),
        },
    ])
    .unwrap();
    let owner = OwnedValue::new::<Pair>((7, Aligned(String::from("aligned"))).into_c_stack_list())
        .with_value_type(descriptor)
        .unwrap();
    let (number, aligned) = owner
        .try_clone()
        .unwrap()
        .try_into_typed::<Pair>()
        .unwrap()
        .into_rust_tuple();
    assert_eq!(number, 7);
    assert_eq!(aligned.0, "aligned");
    assert_eq!(std::ptr::from_ref(&aligned).addr() % 128, 0);
}

/// Records destruction order and optionally panics once.
struct OrderedDrop {
    log: Rc<RefCell<Vec<u8>>>,
    index: u8,
    panic: bool,
}

impl Drop for OrderedDrop {
    /// Records destruction before an optional deliberate unwind.
    fn drop(&mut self) {
        self.log.borrow_mut().push(self.index);
        assert!(!self.panic, "deliberate drop panic");
    }
}

/// Verifies nested erased destruction resumes at remaining children after unwinding.
#[test]
fn destruction_resumes_in_reverse_order_after_panic() {
    type Pair = Cons<Leaf<OrderedDrop>, Cons<ArrayOf<Leaf<OrderedDrop>>, Nil>>;
    let log = Rc::new(RefCell::new(Vec::new()));
    let owner = OwnedValue::new::<Pair>(
        (
            OrderedDrop {
                log: log.clone(),
                index: 0,
                panic: false,
            },
            Array::from_vec(vec![
                OrderedDrop {
                    log: log.clone(),
                    index: 1,
                    panic: false,
                },
                OrderedDrop {
                    log: log.clone(),
                    index: 2,
                    panic: true,
                },
                OrderedDrop {
                    log: log.clone(),
                    index: 3,
                    panic: false,
                },
            ]),
        )
            .into_c_stack_list(),
    );
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(owner))).is_err());
    assert_eq!(*log.borrow(), [3, 2, 1, 0]);
}

/// Verifies descriptor layout overflow is reported rather than wrapped.
#[test]
fn tuple_descriptor_rejects_layout_overflow() {
    let mut huge = ValueType::leaf::<u8>();
    for _ in 0..(isize::BITS - 2) {
        huge = ValueType::tuple(vec![
            AssociatedType {
                offset: 0,
                value_type: huge.clone(),
            },
            AssociatedType {
                offset: 0,
                value_type: huge,
            },
        ])
        .unwrap();
    }
    assert!(
        ValueType::tuple(vec![
            AssociatedType {
                offset: 0,
                value_type: huge.clone()
            },
            AssociatedType {
                offset: 0,
                value_type: huge
            },
        ])
        .is_err()
    );
}
