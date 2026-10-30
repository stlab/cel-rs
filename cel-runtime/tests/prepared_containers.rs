//! Acceptance contracts for the canonical dynamic executor.
use cel_runtime::{
    Array, ArrayOf, Cons, DynSegment, IntoCStackList, IntoRustTuple, Leaf, Nil, ValueType,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

type Pair = Cons<Leaf<i32>, Cons<Leaf<f64>, Nil>>;

/// Arrays contain concrete source-ordered lists and independent returned owners.
#[test]
fn canonical_tuple_array_repeated_calls() {
    let mut body = DynSegment::new::<()>();
    for (i, f) in [(1, 2.0), (3, 4.0)] {
        let origin = body.current_stack_offset();
        body.just(i);
        body.just(f);
        body.make_tuple(2, origin).unwrap();
    }
    body.make_array(2, 0).unwrap();
    let mut prepared = body.prepare::<Nil, ArrayOf<Pair>>().unwrap();
    let first = prepared.call(&().into_c_stack_list()).unwrap().into_vec();
    let second = prepared.call(&().into_c_stack_list()).unwrap().into_vec();
    assert_eq!(first[0].clone().into_rust_tuple(), (1, 2.0));
    assert_eq!(second[1].clone().into_rust_tuple(), (3, 4.0));
    assert_ne!(first.as_ptr(), second.as_ptr());
}

/// Canonical argument reads clone borrowed nested arrays, never steal their owners.
#[test]
fn borrowed_nested_arguments_and_zero_comparisons() {
    type A = Cons<ArrayOf<Leaf<i32>>, Nil>;
    let mut body = DynSegment::new::<()>();
    body.push_argument(0, ValueType::array(ValueType::cloneable_leaf::<i32>()))
        .unwrap();
    body.op1_vec::<Leaf<i32>, ArrayOf<Leaf<i32>>, _>(Array::from_vec)
        .unwrap();
    let mut prepared = body.prepare::<A, ArrayOf<Leaf<i32>>>().unwrap();
    let input = (Array::from_vec(vec![1, 2]),).into_c_stack_list();
    assert_eq!(prepared.call(&input).unwrap().into_vec(), vec![1, 2]);
    assert_eq!(prepared.call(&input).unwrap().into_vec(), vec![1, 2]);
}

/// Indexing follows source order even when packing above ambient operands.
#[test]
fn tuple_index_above_ambient() {
    let mut body = DynSegment::new::<()>();
    body.just(10u8);
    let origin = body.current_stack_offset();
    body.just(1i32);
    body.just(2f64);
    body.make_tuple(2, origin).unwrap();
    body.tuple_index(1).unwrap();
    body.op2(|a: u8, b: f64| f64::from(a) + b).unwrap();
    assert_eq!(body.call0::<f64>().unwrap(), 12.0);
}

/// Unit and typed empty nested arrays use canonical, not external wrapper, storage.
#[test]
fn canonical_unit_nested_arrays() {
    type Item = Cons<Nil, Cons<ArrayOf<Leaf<i32>>, Nil>>;
    let mut body = DynSegment::new::<()>();
    for i in [1i32, 2] {
        let origin = body.current_stack_offset();
        body.make_tuple(0, origin).unwrap();
        let start = body.current_stack_offset();
        body.just(i);
        body.make_array(1, start).unwrap();
        body.make_tuple(2, origin).unwrap();
    }

    body.make_array(2, 0).unwrap();
    let mut prepared = body.prepare::<Nil, ArrayOf<Item>>().unwrap();
    let result = prepared.call(&().into_c_stack_list()).unwrap().into_vec();
    assert_eq!(result.len(), 2);
    for (record, expected) in result.into_iter().zip([[1], [2]]) {
        let (unit, values) = record.into_rust_tuple();
        let (): () = unit.into_rust_tuple();
        assert_eq!(values.into_vec(), expected);
    }

    let mut empty = DynSegment::new::<()>();
    empty
        .make_typed_array(0, 0, ValueType::array(ValueType::leaf::<i32>()))
        .unwrap();
    let mut empty = empty.prepare::<Nil, ArrayOf<ArrayOf<Leaf<i32>>>>().unwrap();
    assert!(empty.call(&().into_c_stack_list()).unwrap().is_empty());
}

/// Records only live runtime owners, independently of stored operation closures.
struct Tracked {
    name: &'static str,
    events: Rc<RefCell<Vec<&'static str>>>,
}

impl Drop for Tracked {
    /// Records exactly one destruction per initialized owner.
    fn drop(&mut self) {
        self.events.borrow_mut().push(self.name);
    }
}

/// Later initializer errors destroy completed nested values and retain context.
#[test]
fn later_nested_element_error_destroys_completed_prefix() {
    type Item = Cons<Nil, Cons<ArrayOf<Leaf<Tracked>>, Nil>>;
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut body = DynSegment::new::<()>();
    for failing in [false, true] {
        let origin = body.current_stack_offset();
        body.make_tuple(0, origin).unwrap();
        let array_origin = body.current_stack_offset();
        let log = events.clone();
        body.op0r(move || -> anyhow::Result<Tracked> {
            if failing {
                log.borrow_mut().push("later failed");
                Err(anyhow::anyhow!("original error").context("later element"))
            } else {
                log.borrow_mut().push("first initialized");
                Ok(Tracked {
                    name: "first dropped",
                    events: log.clone(),
                })
            }
        });
        body.make_array(1, array_origin).unwrap();
        body.make_tuple(2, origin).unwrap();
    }
    body.make_array(2, 0).unwrap();
    let mut body = body.prepare::<Nil, ArrayOf<Item>>().unwrap();
    for _ in 0..2 {
        let error = body.call(&().into_c_stack_list()).err().unwrap();
        assert_eq!(error.to_string(), "later element");
        assert_eq!(error.root_cause().to_string(), "original error");
    }
    assert_eq!(
        *events.borrow(),
        [
            "first initialized",
            "later failed",
            "first dropped",
            "first initialized",
            "later failed",
            "first dropped",
        ]
    );
}

/// A slice adapter observes the original payload and destroys its consumed owner afterward.
#[test]
fn slice_adapter_borrows_original_allocation_and_drops_after_callback() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let pointer = Rc::new(Cell::new(std::ptr::null::<Tracked>()));
    let mut body = DynSegment::new::<()>();
    let log = events.clone();
    let allocation = pointer.clone();
    body.op0_repr::<ArrayOf<Leaf<Tracked>>, _>(move || {
        let array = Array::from_vec(vec![Tracked {
            name: "dropped",
            events: log.clone(),
        }]);
        allocation.set(array.as_slice().as_ptr());
        array
    })
    .unwrap();
    let log = events.clone();
    body.op1_slice::<Leaf<Tracked>, Leaf<usize>, _>(move |slice| {
        assert_eq!(slice.as_ptr(), pointer.get());
        log.borrow_mut().push("callback");
        slice.len()
    })
    .unwrap();
    assert_eq!(body.call0::<usize>().unwrap(), 1);
    assert_eq!(*events.borrow(), ["callback", "dropped"]);
}

/// A host panic destroys operands and ambient canonical owners without corrupting reuse.
#[test]
fn host_panic_drops_nested_ambient_and_consumed_array() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut body = DynSegment::new::<()>();
    for name in ["ambient", "operand"] {
        let origin = body.current_stack_offset();
        let log = events.clone();
        body.op0(move || Tracked {
            name,
            events: log.clone(),
        });
        body.make_array(1, origin).unwrap();
    }
    body.op1_slice::<Leaf<Tracked>, Leaf<()>, _>(|_| panic!("host panic"))
        .unwrap();
    body.op2_repr::<ArrayOf<Leaf<Tracked>>, Leaf<()>, Leaf<()>, _>(|_, _| ())
        .unwrap();
    for _ in 0..2 {
        assert!(catch_unwind(AssertUnwindSafe(|| body.call0::<()>())).is_err());
    }
    assert_eq!(
        *events.borrow(),
        ["operand", "ambient", "operand", "ambient"]
    );
}

/// A tuple projection moves one field and destroys all siblings exactly once.
#[test]
fn projection_moves_selected_owner_and_drops_siblings() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut body = DynSegment::new::<()>();
    for name in ["first", "second", "third"] {
        let log = events.clone();
        body.op0(move || Tracked {
            name,
            events: log.clone(),
        });
    }
    body.make_tuple(3, 0).unwrap();
    body.tuple_index(1).unwrap();
    let result = body.call0::<Tracked>().unwrap();
    assert_eq!(*events.borrow(), ["third", "first"]);
    drop(result);
    assert_eq!(*events.borrow(), ["third", "first", "second"]);
}

/// Ordinary operations cannot implicitly cast structural results to opaque carrier leaves.
#[test]
fn ordinary_operations_and_static_conversion_reject_structural_values() {
    let mut body = DynSegment::new::<()>();
    body.make_tuple(0, 0).unwrap();
    assert!(body.op1(|_: cel_runtime::CNil<()>| 7i32).is_err());
    type Stack = cel_runtime::CStackList<cel_runtime::CNil<()>, cel_runtime::CNil<()>>;
    assert!(cel_runtime::Segment::<(), Stack>::try_from(body).is_err());

    let mut body = DynSegment::new::<()>();
    body.just(vec![1i32, 2]);
    body.op1(|values: Vec<i32>| values.len()).unwrap();
    assert_eq!(body.call0::<usize>().unwrap(), 2);
}

/// Callable arguments and results share the canonical container ABI.
#[test]
fn closure_passes_and_returns_canonical_nested_arrays() {
    let descriptor = ValueType::array(ValueType::array(ValueType::cloneable_leaf::<i32>()));
    let mut inner = DynSegment::new::<()>();
    inner.push_argument(0, descriptor.clone()).unwrap();
    let closure = cel_runtime::DynClosure::typed(vec![descriptor], inner).unwrap();
    let mut body = DynSegment::new::<()>();
    body.push_closure(closure).unwrap();
    let outer = body.current_stack_offset();
    let inner = body.current_stack_offset();
    body.just(7i32);
    body.make_array(1, inner).unwrap();
    body.make_array(1, outer).unwrap();
    body.invoke_closure(1).unwrap();
    let mut body = body.prepare::<Nil, ArrayOf<ArrayOf<Leaf<i32>>>>().unwrap();
    assert_eq!(
        body.call(&().into_c_stack_list())
            .unwrap()
            .into_vec()
            .remove(0)
            .into_vec(),
        [7]
    );
}
