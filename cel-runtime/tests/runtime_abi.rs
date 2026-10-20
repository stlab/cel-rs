//! Regression contracts shared by opaque leaves and the canonical root ABI.

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cel_runtime::*;

/// Records destruction of actual runtime operands, not registration data.
struct Tracked(&'static str, Rc<RefCell<Vec<&'static str>>>);

impl Drop for Tracked {
    /// Appends this live owner's unique destruction event.
    fn drop(&mut self) {
        self.1.borrow_mut().push(self.0);
    }
}

/// Ordinary arities retain their source operand order and repeatable result metadata.
#[test]
fn opaque_arities_and_preloaded_arguments() -> anyhow::Result<()> {
    let mut body = DynSegment::new::<()>();
    body.op0(|| 30u32);
    body.op0(|| 12u32);
    body.op2(|a: u32, b: u32| a + b)?;
    body.op0(|| 100u32);
    body.op0(|| 10u32);
    body.op3(|a: u32, b: u32, c: u32| a + b - c)?;
    body.op1(|value: u32| value.to_string())?;
    assert_eq!(body.call0::<String>()?, "132");
    assert_eq!(body.call0::<String>()?, "132");

    let mut body = DynSegment::new::<(u32,)>();
    body.just(12u32);
    body.op2(|a: u32, b: u32| a + b)?;
    assert_eq!(body.call1::<u32, u32>(30)?, 42);
    assert_eq!(body.call1::<u32, u32>(1)?, 13);
    assert!(body.call1::<i32, u32>(1).is_err());

    let mut body = DynSegment::new::<(u8, f64)>();
    body.op2(|a: u8, b: f64| f64::from(a) + b)?;
    let typed = Segment::<(u8, f64), CStackList<f64, CNil<()>>>::try_from(body)?;
    assert_eq!(typed.call((7, 2.0))?, 9.0);
    Ok(())
}

/// Every ordinary fallible arity destroys the live ambient prefix on error.
#[test]
fn ordinary_fallible_arities_cleanup_and_reuse() -> anyhow::Result<()> {
    for arity in 0..=3 {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut body = DynSegment::new::<()>();
        let log = events.clone();
        body.op0(move || Tracked("ambient", log.clone()));
        for _ in 0..arity {
            body.just(1u32);
        }
        match arity {
            0 => body.op0r(|| -> anyhow::Result<u32> { anyhow::bail!("host error") }),
            1 => body.op1r(|_: u32| -> anyhow::Result<u32> { anyhow::bail!("host error") })?,
            2 => {
                body.op2r(|_: u32, _: u32| -> anyhow::Result<u32> { anyhow::bail!("host error") })?
            }
            3 => body.op3r(|_: u32, _: u32, _: u32| -> anyhow::Result<u32> {
                anyhow::bail!("host error")
            })?,
            _ => unreachable!(),
        }
        body.op2(|_: Tracked, value: u32| value)?;
        for _ in 0..2 {
            assert_eq!(body.call0::<u32>().unwrap_err().to_string(), "host error");
        }
        assert_eq!(*events.borrow(), ["ambient", "ambient"]);
    }
    Ok(())
}

/// Rejected setup checks do not consume operands or partially register operations.
#[test]
fn failed_registration_is_transactional() -> anyhow::Result<()> {
    let mut body = DynSegment::new::<()>();
    body.just(7i32);
    assert!(body.op1(|_: u32| 0u32).is_err());
    assert!(body.op2(|_: i32, _: i32| 0i32).is_err());
    assert!(body.make_tuple(1, 1).is_err());
    assert!(
        body.make_typed_array(1, 0, ValueType::leaf::<u32>())
            .is_err()
    );
    assert_eq!(body.call0::<i32>()?, 7);
    assert!(body.call_dyn::<String>(&[]).is_err());
    assert_eq!(body.call0::<i32>()?, 7);
    Ok(())
}

/// A fragment error destroys its local prefix and the parent ambient prefix exactly once.
#[test]
fn conditional_failure_cleans_each_ownership_region() -> anyhow::Result<()> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut body = DynSegment::new::<()>();
    let log = events.clone();
    body.op0(move || Tracked("parent", log.clone()));
    body.just(true);
    let mut yes = body.new_fragment();
    let log = events.clone();
    yes.op0(move || Tracked("branch", log.clone()));
    yes.op0r(|| -> anyhow::Result<u32> { anyhow::bail!("branch error") });
    yes.op2(|_: Tracked, value: u32| value)?;
    let mut no = body.new_fragment();
    no.just(0u32);
    body.join2(yes, no)?;
    body.op2(|_: Tracked, value: u32| value)?;
    assert_eq!(body.call0::<u32>().unwrap_err().to_string(), "branch error");
    assert_eq!(*events.borrow(), ["branch", "parent"]);
    Ok(())
}

/// Explicit witnesses authorize both tuple relabel directions and direct static conversion.
#[test]
fn witnessed_tuple_relabel_and_direct_static_segment() -> anyhow::Result<()> {
    type Pair = Cons<Leaf<i32>, Cons<Leaf<f64>, Nil>>;
    type Concrete = <Pair as ValueRepr>::Value;
    let mut body = DynSegment::new::<()>();
    body.just((7i32, 2f64).into_c_stack_list());
    body.push_tuple::<Pair>()?;
    assert_eq!(body.peek_tuple_arity(), Some(2));
    body.pop_tuple_as::<Pair>()?;
    type Stack = CStackList<Concrete, CNil<()>>;
    let typed = Segment::<(), Stack>::try_from(body)?;
    assert_eq!(typed.call(())?.into_rust_tuple(), (7, 2.0));
    Ok(())
}

/// External wrappers are imported once; subsequent trusted calls return canonical owners.
#[test]
fn checked_external_constants_and_borrowed_inputs() -> anyhow::Result<()> {
    let element = ValueType::cloneable_leaf::<i32>();
    let array = DynamicArray::try_from_vec(vec![1i32, 2]).unwrap();
    let mut body = DynSegment::new::<()>();
    body.just_external(ValueType::array(element.clone()), Box::new(array))?;
    let mut body = body.prepare::<Nil, ArrayOf<Leaf<i32>>>()?;
    assert_eq!(body.call(&().into_c_stack_list())?.into_vec(), [1, 2]);
    assert_eq!(body.call(&().into_c_stack_list())?.into_vec(), [1, 2]);

    let mut body = DynSegment::new::<()>();
    body.push_argument(0, ValueType::array(element))?;
    let array = DynamicArray::try_from_vec(vec![3i32, 4]).unwrap();
    let result = body
        .call_dyn_external(&[&array])?
        .downcast::<DynamicArray>()
        .unwrap();
    assert_eq!(result.try_into_vec::<i32>().unwrap(), [3, 4]);
    assert_eq!(array.len(), 2);
    assert!(body.call_dyn_value(&[&7i32]).is_err());
    Ok(())
}

/// Canonical callable owners preserve the freeze after the producer segment drops.
#[test]
fn escaped_callable_keeps_all_aliases_frozen() -> anyhow::Result<()> {
    let mut inner = DynSegment::new::<()>();
    inner.just(7i32);
    let closure = DynClosure::typed(vec![], inner)?;
    let alias = closure.clone();
    let mut body = DynSegment::new::<()>();
    body.push_closure(closure)?;
    let escaped = body.call_dyn_value(&[])?;
    drop(body);
    /// Deliberately replaces a body if mutation is allowed.
    fn mutate(body: &mut DynSegment, _: &[&dyn Any]) -> anyhow::Result<Box<dyn Any>> {
        *body = DynSegment::new::<()>();
        body.just(false);
        Ok(Box::new(false))
    }
    assert!(alias.call_boxed(&[], mutate).is_err());
    drop(escaped);
    assert!(alias.call_boxed(&[], mutate).is_ok());
    Ok(())
}

/// Splitting mixed external results converts nested lists rather than casting their bytes.
#[test]
fn mixed_tuple_external_boundary() -> anyhow::Result<()> {
    /// Clones a concrete scalar for the checked external extractor.
    unsafe fn extract_i32(pointer: *const u8) -> Box<dyn Any> {
        Box::new(unsafe { *pointer.cast::<i32>() })
    }
    let mut body = DynSegment::new::<()>();
    body.just(7i32);
    let origin = body.current_stack_offset();
    body.just(8i32);
    body.make_tuple(1, origin)?;
    body.make_tuple(2, 0)?;
    let extractors = [
        DynExtractor::Scalar(TypeId::of::<i32>(), extract_i32),
        DynExtractor::Tuple(Box::new(|id| {
            (id == TypeId::of::<i32>()).then(|| {
                (
                    element_dropper_for::<i32>(),
                    element_cloner_for::<i32>(),
                    element_eq_for::<i32>(),
                    element_debug_for::<i32>(),
                )
            })
        })),
    ];
    let result = unsafe { body.call_dyn_tuple_mixed(&[], &extractors) }?;
    assert_eq!(result[0].downcast_ref::<i32>(), Some(&7));
    assert_eq!(
        result[1]
            .downcast_ref::<DynamicSequence>()
            .unwrap()
            .try_to_tuple::<(i32,)>()?,
        (8,)
    );
    Ok(())
}

/// Zero-copy vector adapters transfer the same allocation through fallible host functions.
#[test]
fn vector_adapter_retains_allocation() -> anyhow::Result<()> {
    let pointer = Rc::new(Cell::new(std::ptr::null::<i32>()));
    let allocation = pointer.clone();
    let mut body = DynSegment::new::<()>();
    body.op0_repr::<ArrayOf<Leaf<i32>>, _>(move || {
        let values = vec![1i32, 2, 3];
        allocation.set(values.as_ptr());
        Array::from_vec(values)
    })?;
    body.op1r_vec::<Leaf<i32>, Leaf<usize>, _>(move |values| {
        assert_eq!(values.as_ptr(), pointer.get());
        Ok(values.len())
    })?;
    assert_eq!(body.call0::<usize>()?, 3);
    Ok(())
}

/// Array projection moves noncloneable values, destroys siblings, and preserves nested payloads.
#[test]
fn canonical_array_index_and_drop() -> anyhow::Result<()> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut body = DynSegment::new::<()>();
    for name in ["first", "selected", "last"] {
        let log = events.clone();
        body.op0(move || Tracked(name, log.clone()));
    }
    body.make_array(3, 0)?;
    body.just(1i32);
    body.array_index::<i32>()?;
    let selected = body.call0::<Tracked>()?;
    assert_eq!(*events.borrow(), ["last", "first"]);
    drop(selected);
    assert_eq!(*events.borrow(), ["last", "first", "selected"]);

    let pointer = Rc::new(Cell::new(std::ptr::null::<i32>()));
    let allocation = pointer.clone();
    let mut body = DynSegment::new::<()>();
    body.op0_repr::<ArrayOf<ArrayOf<Leaf<i32>>>, _>(move || {
        let inner = Array::from_vec(vec![7i32]);
        allocation.set(inner.as_slice().as_ptr());
        Array::from_vec(vec![inner])
    })?;
    body.just(0usize);
    body.array_index::<usize>()?;
    let mut body = body.prepare::<Nil, ArrayOf<Leaf<i32>>>()?;
    let result = body.call(&().into_c_stack_list())?;
    assert_eq!(result.as_slice().as_ptr(), pointer.get());
    Ok(())
}

/// Invalid array indices destroy every consumed and ambient owner on each call.
#[test]
fn invalid_array_indices_clean_the_whole_frame() -> anyhow::Result<()> {
    for index in [-1i32, 2] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut body = DynSegment::new::<()>();
        let log = events.clone();
        body.op0(move || Tracked("ambient", log.clone()));
        let origin = body.current_stack_offset();
        for name in ["first", "last"] {
            let log = events.clone();
            body.op0(move || Tracked(name, log.clone()));
        }
        body.make_array(2, origin)?;
        body.just(index);
        body.array_index::<i32>()?;
        body.op2(|_: Tracked, value: Tracked| value)?;
        for _ in 0..2 {
            assert!(body.call0::<Tracked>().is_err());
        }
        assert_eq!(
            *events.borrow(),
            ["last", "first", "ambient", "last", "first", "ambient"]
        );
    }
    Ok(())
}

/// Descriptor-aware dropping destroys nested owners without adding an opaque unit value.
#[test]
fn explicit_drop_preserves_ambient_values() -> anyhow::Result<()> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut body = DynSegment::new::<()>();
    body.just(7i32);
    let origin = body.current_stack_offset();
    let log = events.clone();
    body.op0(move || Tracked("nested", log.clone()));
    body.make_array(1, origin)?;
    body.drop_value()?;
    assert_eq!(body.call0::<i32>()?, 7);
    assert_eq!(*events.borrow(), ["nested"]);
    Ok(())
}

/// Registered capabilities attach only to the exact existing result shape.
#[test]
fn registered_capabilities_survive_checked_tuple_export() -> anyhow::Result<()> {
    let mut body = DynSegment::new::<()>();
    body.op0(|| 7i32);
    assert!(
        body.annotate_top(ValueType::cloneable_leaf::<u32>())
            .is_err()
    );
    body.annotate_top(
        ValueType::cloneable_leaf::<i32>()
            .with_equality::<Leaf<i32>>()?
            .with_debug::<Leaf<i32>>()?,
    )?;
    body.make_tuple(1, 0)?;
    assert_eq!(
        body.call_dyn_as_dynamic_sequence(&[])?
            .try_into_tuple::<(i32,)>()?,
        (7,)
    );
    Ok(())
}
/// Checked host calls export legacy wrappers without exposing them to interpreter operations.
#[test]
fn checked_host_call_exports_array() {
    let mut body = cel_runtime::DynSegment::new::<()>();
    body.just(1i32);
    body.make_array(1, 0).unwrap();
    let result: cel_runtime::DynamicArray = body.call0().unwrap();
    assert_eq!(result.try_into_vec::<i32>().unwrap(), [1]);
}

/// Checked host mismatches and missing tuple capabilities fail before evaluating the body.
#[test]
fn checked_host_export_preflight_never_executes_invalid_results() {
    let calls = Rc::new(Cell::new(0));
    let mut body = DynSegment::new::<()>();
    let counter = calls.clone();
    body.op0(move || {
        counter.set(counter.get() + 1);
        1i32
    });
    body.make_tuple(1, 0).unwrap();
    assert!(body.call0::<i32>().is_err());
    assert!(body.call_dyn_as_dynamic_sequence(&[]).is_err());
    assert_eq!(calls.get(), 0);

    body.annotate_top(
        ValueType::tuple(vec![cel_runtime::AssociatedType {
            offset: 0,
            value_type: ValueType::cloneable_leaf::<i32>()
                .with_equality::<Leaf<i32>>()
                .unwrap()
                .with_debug::<Leaf<i32>>()
                .unwrap(),
        }])
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        body.call_dyn_as_dynamic_sequence(&[])
            .unwrap()
            .try_into_tuple::<(i32,)>()
            .unwrap(),
        (1,)
    );
    assert_eq!(calls.get(), 1);
}

/// Provides an owning, over-aligned value for canonical packing regressions.
#[repr(align(64))]
struct Aligned(String);

/// Canonical array payloads retain over-alignment and heterogeneous tail-first tuple order.
#[test]
fn canonical_overaligned_array_and_divergent_tuple_layout() {
    type Fields = Cons<Leaf<u8>, Cons<Leaf<Aligned>, Cons<Leaf<u16>, Nil>>>;
    let mut body = DynSegment::new::<()>();
    body.just(7u8);
    let origin = body.current_stack_offset();
    body.just(1u8);
    body.op0(|| Aligned("owned".to_owned()));
    body.just(2u16);
    body.make_tuple(3, origin).unwrap();
    body.make_array(1, origin).unwrap();
    body.make_tuple(2, 0).unwrap();
    type Result = Cons<Leaf<u8>, Cons<ArrayOf<Fields>, Nil>>;
    let mut prepared = body.prepare::<Nil, Result>().unwrap();
    let (ambient, values) = prepared
        .call(&().into_c_stack_list())
        .unwrap()
        .into_rust_tuple();
    assert_eq!(ambient, 7);
    assert_eq!((values.as_slice().as_ptr() as usize) % 64, 0);
    let (first, aligned, last) = values.into_vec().pop().unwrap().into_rust_tuple();
    assert_eq!((first, aligned.0.as_str(), last), (1, "owned", 2));
}

thread_local! {
    /// Counts observable destructor calls for initialized zero-sized owners.
    static ZST_DROPS: Cell<usize> = const { Cell::new(0) };
}

/// Owns no bytes but still requires one destruction per live value.
struct ZeroSized;

impl Drop for ZeroSized {
    /// Counts one initialized owner's destruction.
    fn drop(&mut self) {
        ZST_DROPS.with(|count| count.set(count.get() + 1));
    }
}

/// Repeated canonical zero-sized arrays retain every element's destruction obligation.
#[test]
fn canonical_zero_sized_array_drops_each_initialized_element() {
    ZST_DROPS.with(|count| count.set(0));
    let mut body = DynSegment::new::<()>();
    for _ in 0..3 {
        body.op0(|| ZeroSized);
    }
    body.make_array(3, 0).unwrap();
    let mut prepared = body.prepare::<Nil, ArrayOf<Leaf<ZeroSized>>>().unwrap();
    for expected in [3, 6] {
        let values = prepared.call(&().into_c_stack_list()).unwrap();
        assert_eq!(values.len(), 3);
        drop(values);
        ZST_DROPS.with(|count| assert_eq!(count.get(), expected));
    }
}
