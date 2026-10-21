//! Contracts for signature-typed native and interpreted callable values.
use cel_runtime::{
    Array, ArrayOf, CNil, Callable, Cons, DynSegment, EmptyList, FnOf, IntoCStackList, Leaf, Nil,
    ValueRepr, ValueType,
};
use std::cell::{Cell, RefCell};

type IntFn = FnOf<Nil, Leaf<i32>>;

/// Calls either body kind through exactly the same Rust interface.
fn apply(f: &Callable<CNil<()>, i32>) -> anyhow::Result<i32> {
    f.call(&CNil::<()>::root_empty())
}

/// Returns seven without consuming the borrowed empty frame.
fn seven(_: &CNil<()>) -> anyhow::Result<i32> {
    Ok(7)
}

/// Builds the interpreted counterpart of the native constant function.
fn interpreted() -> Callable<CNil<()>, i32> {
    let mut body = DynSegment::new::<()>();
    body.just(7i32);
    Callable::from_prepared(body.prepare::<Nil, Leaf<i32>>().unwrap())
}

/// Both bodies share one interface, including when nested in canonical containers.
#[test]
fn native_and_interpreted_share_container_representation() {
    let native = Callable::from_fn(seven);
    let interpreted = interpreted();
    assert_eq!(apply(&native).unwrap(), 7);
    assert_eq!(apply(&interpreted).unwrap(), 7);
    let functions = Array::from_vec(vec![native, interpreted]);
    let mut body = DynSegment::new::<()>();
    body.op0_repr::<ArrayOf<IntFn>, _>(move || functions.clone())
        .unwrap();
    body.make_tuple(1, 0).unwrap();
    let mut body = body.prepare::<Nil, Cons<ArrayOf<IntFn>, Nil>>().unwrap();
    let output = body.call(&CNil::<()>::root_empty()).unwrap();
    for function in output.1.as_slice() {
        assert_eq!(apply(function).unwrap(), 7);
    }
}

/// Returns an independently owned native array.
fn array(_: &CNil<()>) -> anyhow::Result<Array<i32>> {
    Ok(Array::from_vec(vec![3, 7]))
}

type ArrayArgs = <Cons<ArrayOf<Leaf<i32>>, Nil> as ValueRepr>::Value;

/// Reads a canonical tuple containing an array without consuming it.
fn sum(args: &ArrayArgs) -> anyhow::Result<i32> {
    Ok(args.1.as_slice().iter().sum())
}

/// Array arguments and results use the same borrowed/owned API for both bodies.
#[test]
fn native_and_interpreted_array_arguments_and_results() {
    let mut body = DynSegment::new::<()>();
    body.just(3i32);
    body.just(7i32);
    body.make_array(2, 0).unwrap();
    let interpreted = Callable::from_prepared(body.prepare::<Nil, ArrayOf<Leaf<i32>>>().unwrap());
    for f in [Callable::from_fn(array), interpreted] {
        assert_eq!(
            f.call(&CNil::<()>::root_empty()).unwrap().into_vec(),
            [3, 7]
        );
    }

    let mut body = DynSegment::new::<()>();
    body.push_argument(0, ValueType::array(ValueType::cloneable_leaf::<i32>()))
        .unwrap();
    body.op1_slice::<Leaf<i32>, Leaf<i32>, _>(|xs| xs.iter().sum())
        .unwrap();
    let interpreted = Callable::from_prepared(
        body.prepare::<Cons<ArrayOf<Leaf<i32>>, Nil>, Leaf<i32>>()
            .unwrap(),
    );
    let input = (Array::from_vec(vec![3, 7]),).into_c_stack_list();
    for f in [Callable::from_fn(sum), interpreted] {
        assert_eq!(f.call(&input).unwrap(), 10);
        assert_eq!(input.1.as_slice(), [3, 7]);
    }
}

/// CEL-emitted calls use the common frame dispatcher for either implementation.
#[test]
fn emitted_calls_accept_native_and_interpreted_arrays() {
    type Args = Cons<ArrayOf<Leaf<i32>>, Nil>;
    type F = FnOf<Args, ArrayOf<Leaf<i32>>>;
    /// Returns an independent native result while preserving the borrowed input.
    fn identity(args: &ArrayArgs) -> anyhow::Result<Array<i32>> {
        Ok(args.1.clone())
    }
    let mut interpreted = DynSegment::new::<()>();
    interpreted
        .push_argument(0, ValueType::array(ValueType::cloneable_leaf::<i32>()))
        .unwrap();
    let interpreted =
        Callable::from_prepared(interpreted.prepare::<Args, ArrayOf<Leaf<i32>>>().unwrap());
    for f in [Callable::from_fn(identity), interpreted] {
        let mut body = DynSegment::new::<()>();
        body.op0_repr::<F, _>(move || f.clone()).unwrap();
        body.push_argument(0, ValueType::array(ValueType::cloneable_leaf::<i32>()))
            .unwrap();
        body.invoke_closure(1).unwrap();
        let mut body = body.prepare::<Args, ArrayOf<Leaf<i32>>>().unwrap();
        let args = (Array::from_vec(vec![3, 7]),).into_c_stack_list();
        let output = body.call(&args).unwrap();
        assert_eq!(output.as_slice(), [3, 7]);
        assert_ne!(output.as_slice().as_ptr(), args.1.as_slice().as_ptr());
        assert_eq!(body.call(&args).unwrap().as_slice(), [3, 7]);
    }
}

/// Callable values can receive and return nested callable-containing containers.
#[test]
fn higher_order_nested_callables_share_typed_adapters() {
    type Nested = Cons<ArrayOf<IntFn>, Nil>;
    type Args = Cons<Nested, Nil>;
    type Higher = FnOf<Args, Nested>;
    /// Clones a tuple of shared callables without executing any of them.
    fn native(args: &<Args as ValueRepr>::Value) -> anyhow::Result<<Nested as ValueRepr>::Value> {
        Ok(args.1.clone())
    }
    let mut body = DynSegment::new::<()>();
    body.push_argument(0, Nested::value_type()).unwrap();
    let compiled = Callable::from_prepared(body.prepare::<Args, Nested>().unwrap());
    for f in [Callable::from_fn(native), compiled] {
        let mut body = DynSegment::new::<()>();
        body.op0_repr::<Higher, _>(move || f.clone()).unwrap();
        body.push_argument(0, Nested::value_type()).unwrap();
        body.invoke_closure(1).unwrap();
        let mut body = body.prepare::<Args, Nested>().unwrap();
        let args = ((Array::from_vec(vec![
            Callable::from_fn(seven),
            interpreted(),
        ]),)
            .into_c_stack_list(),)
            .into_c_stack_list();
        let result = body.call(&args).unwrap();
        assert_eq!(apply(&result.1.as_slice()[0]).unwrap(), 7);
        assert_eq!(apply(&result.1.as_slice()[1]).unwrap(), 7);
    }
}

thread_local! {
    /// Counts body effects so preparation failures cannot hide execution.
    static EFFECTS: Cell<usize> = const { Cell::new(0) };
    /// Retains a callable temporarily to exercise reentrant invocation.
    static REENTRANT: RefCell<Option<Callable<CNil<()>, i32>>> = const { RefCell::new(None) };
}

/// Enters the same native carrier again through an independent alias.
fn recurse(args: &CNil<()>) -> anyhow::Result<i32> {
    REENTRANT.with(|slot| slot.borrow().as_ref().unwrap().call(args))
}

/// Full callable identities reject nested argument/result mismatches before effects.
#[test]
fn preparation_rejects_nested_callable_signatures_before_effects() {
    type Nested = Cons<ArrayOf<IntFn>, Nil>;
    type WrongResult = Cons<ArrayOf<FnOf<Nil, Leaf<u32>>>, Nil>;
    type WrongArgs = Cons<ArrayOf<FnOf<Cons<Leaf<i32>, Nil>, Leaf<i32>>>, Nil>;
    for wrong_result in [true, false] {
        let mut body = DynSegment::new::<()>();
        body.op0_repr::<Nested, _>(|| {
            EFFECTS.with(|n| n.set(n.get() + 1));
            (Array::from_vec(vec![Callable::from_fn(seven)]),).into_c_stack_list()
        })
        .unwrap();
        if wrong_result {
            assert!(body.prepare::<Nil, WrongResult>().is_err());
        } else {
            assert!(body.prepare::<Nil, WrongArgs>().is_err());
        }
    }
    let mut body = DynSegment::new::<()>();
    body.push_argument(0, ArrayOf::<IntFn>::value_type())
        .unwrap();
    assert!(body.prepare::<WrongArgs, ArrayOf<IntFn>>().is_err());
    EFFECTS.with(|n| assert_eq!(n.get(), 0));
}

/// Reentrancy is an error and body failures release the shared execution guard.
#[test]
fn reentrancy_and_body_errors_allow_later_calls() {
    let native = Callable::from_fn(recurse);
    REENTRANT.with(|slot| *slot.borrow_mut() = Some(native.clone()));
    assert!(
        native
            .call(&CNil::<()>::root_empty())
            .unwrap_err()
            .to_string()
            .contains("already executing")
    );
    REENTRANT.with(|slot| *slot.borrow_mut() = None);

    let mut body = DynSegment::new::<()>();
    body.op0r(|| {
        REENTRANT.with(|slot| {
            if let Some(f) = slot.borrow().as_ref() {
                return f.call(&CNil::<()>::root_empty());
            }
            Ok(7i32)
        })
    });
    let f = Callable::from_prepared(body.prepare::<Nil, Leaf<i32>>().unwrap());
    REENTRANT.with(|slot| *slot.borrow_mut() = Some(f.clone()));
    assert!(
        f.call(&CNil::<()>::root_empty())
            .unwrap_err()
            .to_string()
            .contains("already executing")
    );
    REENTRANT.with(|slot| *slot.borrow_mut() = None);
    assert_eq!(apply(&f).unwrap(), 7);
}

/// Cloning the facade does not require either signature component to clone.
#[test]
fn clone_has_no_argument_or_result_clone_bound() {
    /// Deliberately lacks Clone.
    struct NonClone;
    /// Returns a fresh noncloneable owner.
    fn fresh(_: &NonClone) -> anyhow::Result<NonClone> {
        Ok(NonClone)
    }
    let f = Callable::from_fn(fresh).clone();
    let _ = f.call(&NonClone).unwrap();
}

/// Native body errors preserve context and permit a later successful invocation.
#[test]
fn native_body_error_recovery() {
    /// Fails once with a recognizable error, then succeeds.
    fn fail_once(_: &CNil<()>) -> anyhow::Result<i32> {
        EFFECTS.with(|n| {
            let previous = n.replace(n.get() + 1);
            anyhow::ensure!(previous > 0, "native body context");
            Ok(7)
        })
    }
    EFFECTS.with(|n| n.set(0));
    let f = Callable::from_fn(fail_once);
    assert_eq!(apply(&f).unwrap_err().to_string(), "native body context");
    assert_eq!(apply(&f).unwrap(), 7);
}

/// Escaped typed carriers and their clones retain the shared mutable-alias freeze.
#[test]
fn escaped_typed_array_retains_freeze_until_last_clone_drops() {
    use std::any::Any;
    /// Replaces the body only when no prepared or escaped owner protects it.
    fn replace(body: &mut DynSegment, _: &[&dyn Any]) -> anyhow::Result<Box<dyn Any>> {
        *body = DynSegment::new::<()>();
        body.just(9i32);
        Ok(Box::new(()))
    }
    let mut body = DynSegment::new::<()>();
    body.just(7i32);
    let closure = cel_runtime::DynClosure::typed(vec![], body).unwrap();
    let alias = closure.clone();
    let mut outer = DynSegment::new::<()>();
    outer.push_closure(closure).unwrap();
    outer.make_array(1, 0).unwrap();
    let mut outer = outer.prepare::<Nil, ArrayOf<IntFn>>().unwrap();
    let first = outer.call(&CNil::<()>::root_empty()).unwrap();
    let second = outer.call(&CNil::<()>::root_empty()).unwrap();
    let cloned = first.as_slice()[0].clone();
    drop(outer);
    assert!(alias.call_boxed(&[], replace).is_err());
    drop(first);
    drop(second);
    assert!(alias.call_boxed(&[], replace).is_err());
    assert_eq!(apply(&cloned).unwrap(), 7);
    drop(cloned);
    alias.call_boxed(&[], replace).unwrap();
    assert_eq!(alias.call::<i32>(&[]).unwrap(), 9);
}

/// Both dispatcher kinds release the execution guard while unwinding.
#[test]
fn panics_release_native_and_interpreted_execution_guards() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    /// Panics on first invocation and returns seven thereafter.
    fn panic_once(_: &CNil<()>) -> anyhow::Result<i32> {
        EFFECTS.with(|n| {
            let previous = n.replace(n.get() + 1);
            assert!(previous != 0, "body panic");
        });
        Ok(7)
    }
    let mut body = DynSegment::new::<()>();
    body.op0r(|| panic_once(&CNil::<()>::root_empty()));
    let interpreted = Callable::from_prepared(body.prepare::<Nil, Leaf<i32>>().unwrap());
    for f in [Callable::from_fn(panic_once), interpreted] {
        EFFECTS.with(|n| n.set(0));
        assert!(catch_unwind(AssertUnwindSafe(|| apply(&f))).is_err());
        assert_eq!(apply(&f).unwrap(), 7);
    }
}
