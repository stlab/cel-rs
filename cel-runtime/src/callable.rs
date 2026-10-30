//! Calls native and prepared bodies through one statically fixed signature.
//!
//! ```
//! use cel_runtime::{Callable, CNil, EmptyList};
//! let f = Callable::from_fn(|_: &CNil<()>| Ok(7i32));
//! assert_eq!(f.call(&CNil::<()>::root_empty()).unwrap(), 7);
//! ```

use std::marker::PhantomData;
use std::mem::MaybeUninit;

use crate::{ArgumentList, DynClosure, PreparedSegment, ValueRepr};

/// Shares a callable whose borrowed arguments and owned result have fixed Rust types.
///
/// Every instantiation has exactly the canonical [`DynClosure`] representation.
/// Clones share execution state, not arguments or results. Recursive entry into
/// the same carrier returns an error; different carriers can call one another.
///
/// # Examples
///
/// ```
/// use cel_runtime::Callable;
/// let f = Callable::from_fn(|x: &i32| Ok(*x));
/// assert_eq!(f.clone().call(&7).unwrap(), 7);
/// ```
///
/// Wrong Rust arguments cannot enter the body:
///
/// ```compile_fail
/// use cel_runtime::Callable;
/// let f = Callable::from_fn(|x: &i32| Ok(*x));
/// f.call(&"not an integer");
/// ```
#[repr(transparent)]
pub struct Callable<Args, R> {
    /// Shares the exact native or interpreted body under its trusted binding.
    carrier: DynClosure,
    /// Associates the carrier with its validated canonical argument/result witnesses.
    signature: PhantomData<fn(&Args) -> R>,
}

impl<Args, R> Clone for Callable<Args, R> {
    /// Shares the carrier without imposing clone bounds on the signature.
    fn clone(&self) -> Self {
        Self {
            carrier: self.carrier.clone(),
            signature: PhantomData,
        }
    }
}

impl<Args: 'static, R: 'static> Callable<Args, R> {
    /// Shares a native function pointer with this exact borrowed/owned signature.
    ///
    /// # Examples
    /// See the module example.
    pub fn from_fn(f: fn(&Args) -> anyhow::Result<R>) -> Self {
        /// Invokes the construction-bound native function with a matching raw frame.
        ///
        /// # Safety
        /// `body` stores this exact function pointer, `args` borrows `Args`, and
        /// `result` is aligned uninitialized storage for `R`.
        unsafe fn dispatch<Args: 'static, R: 'static>(
            body: *mut (),
            args: *const u8,
            result: *mut u8,
        ) -> anyhow::Result<()> {
            let f = unsafe { &*body.cast::<fn(&Args) -> anyhow::Result<R>>() };
            let value = f(unsafe { &*args.cast::<Args>() })?;
            unsafe { result.cast::<R>().write(value) };
            Ok(())
        }

        Self {
            // SAFETY: This payload and dispatcher share the exact fn signature.
            carrier: unsafe { DynClosure::from_frame_body(Box::new(f), dispatch::<Args, R>, None) },
            signature: PhantomData,
        }
    }

    /// Shares an independently prepared body with matching canonical arguments.
    ///
    /// - Complexity: O(n) descriptor construction for n argument/result type nodes.
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::{Callable, DynSegment, Nil, Leaf, CNil, EmptyList};
    /// let mut body = DynSegment::new::<()>(); body.just(7i32);
    /// let f = Callable::from_prepared(body.prepare::<Nil, Leaf<i32>>().unwrap());
    /// assert_eq!(f.call(&CNil::<()>::root_empty()).unwrap(), 7);
    /// ```
    pub fn from_prepared<A, Result>(body: PreparedSegment<A, Result>) -> Self
    where
        A: ArgumentList<Value = Args> + 'static,
        Result: ValueRepr<Value = R> + 'static,
    {
        /// Invokes an already prepared body and initializes the result only on success.
        ///
        /// # Safety
        /// `body` stores exactly `PreparedSegment<A, R>` and both raw pointers
        /// match its preparation-validated concrete representations.
        unsafe fn dispatch<A: ValueRepr + 'static, R: ValueRepr + 'static>(
            body: *mut (),
            args: *const u8,
            result: *mut u8,
        ) -> anyhow::Result<()> {
            let body = unsafe { &mut *body.cast::<PreparedSegment<A, R>>() };
            let value = body.call(unsafe { &*args.cast::<A::Value>() })?;
            unsafe { result.cast::<R::Value>().write(value) };
            Ok(())
        }
        let args = A::value_type();
        let parameters = args
            .tuple_elements()
            .expect("sealed canonical argument list")
            .iter()
            .map(|field| field.value_type.clone())
            .collect();
        let signature = crate::dyn_closure::ClosureSignature {
            parameters,
            result: Result::value_type(),
        };
        Self {
            // SAFETY: Preparation validates the signature and concrete frames;
            // this exact payload remains paired with its monomorphized dispatcher.
            carrier: unsafe {
                DynClosure::from_frame_body(Box::new(body), dispatch::<A, Result>, Some(signature))
            },
            signature: PhantomData,
        }
    }

    /// Borrows arguments and returns an independent result without checking signatures.
    ///
    /// # Errors
    /// Returns an error on reentrant entry into this shared carrier, or propagates
    /// the original body error. Errors and unwinding release the execution guard
    /// so later calls remain valid.
    /// - Complexity: Body-dependent work; interpreted calls set up O(a) pointers
    ///   for a arguments.
    ///
    /// # Examples
    /// See the module example.
    pub fn call(&self, args: &Args) -> anyhow::Result<R> {
        let mut result = MaybeUninit::<R>::uninit();
        // SAFETY: Only the exact native signature or matching prepared witnesses
        // construct this facade. The carrier cannot escape through a mutable API.
        unsafe {
            self.carrier
                .call_frame_trusted(std::ptr::from_ref(args).cast(), result.as_mut_ptr().cast())?;
            Ok(result.assume_init())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Array, ArrayOf, Cons, DynSegment, FnOf, IntoCStackList, Leaf, Nil, ValueType};
    use std::cell::OnceCell;
    use std::rc::{Rc, Weak};

    /// Describes source-ordered arguments with distinct sizes, alignments, and ownership.
    type MixedArguments = Cons<Leaf<u8>, Cons<Leaf<u64>, Cons<Leaf<String>, Nil>>>;
    /// Stores the canonical borrowed frame for the mixed-layout argument contract.
    type MixedFrame = <MixedArguments as ValueRepr>::Value;
    /// Describes a mixed-layout callable that returns an independently owned string.
    type MixedFunction = FnOf<MixedArguments, Leaf<String>>;

    /// Builds an ordered rendering body with a zero-tag error and reserved panic tag.
    fn mixed_closure() -> DynClosure {
        let mut body = DynSegment::new::<()>();
        body.push_argument(0, ValueType::cloneable_leaf::<u8>())
            .unwrap();
        body.push_argument(1, ValueType::cloneable_leaf::<u64>())
            .unwrap();
        body.push_argument(2, ValueType::cloneable_leaf::<String>())
            .unwrap();
        body.op3r(|tag: u8, word: u64, text: String| {
            anyhow::ensure!(tag != 0, "zero tag");
            assert_ne!(tag, u8::MAX, "typed interpreted body panic");
            Ok(format!("{tag}/{word}/{text}"))
        })
        .unwrap();
        DynClosure::typed(
            vec![
                ValueType::cloneable_leaf::<u8>(),
                ValueType::cloneable_leaf::<u64>(),
                ValueType::cloneable_leaf::<String>(),
            ],
            body,
        )
        .unwrap()
    }

    /// Emits an invocation that borrows all three mixed-layout arguments.
    fn mixed_invocation(closure: DynClosure) -> PreparedSegment<MixedArguments, Leaf<String>> {
        let mut body = DynSegment::new::<()>();
        body.push_closure(closure).unwrap();
        body.push_argument(0, ValueType::cloneable_leaf::<u8>())
            .unwrap();
        body.push_argument(1, ValueType::cloneable_leaf::<u64>())
            .unwrap();
        body.push_argument(2, ValueType::cloneable_leaf::<String>())
            .unwrap();
        body.invoke_closure(3).unwrap();
        body.prepare::<MixedArguments, Leaf<String>>().unwrap()
    }

    /// Escapes a typed interpreted carrier through its public canonical callable witness.
    fn mixed_callable(closure: DynClosure) -> Callable<MixedFrame, String> {
        let mut factory = DynSegment::new::<()>();
        factory.push_closure(closure).unwrap();
        factory
            .prepare::<Nil, MixedFunction>()
            .unwrap()
            .call(&().into_c_stack_list())
            .unwrap()
    }

    /// Rebinds borrowed frames through direct/emitted calls and recovers after errors and panic.
    #[test]
    fn typed_interpreted_frames_rebind_and_recover() {
        let closure = mixed_closure();
        let direct = mixed_callable(closure.clone());
        let mut emitted = mixed_invocation(closure);
        let first = (7u8, 1001u64, String::from("alpha")).into_c_stack_list();
        let second = (9u8, 7002u64, String::from("second-longer")).into_c_stack_list();
        let rejected = (0u8, 3003u64, String::from("reject")).into_c_stack_list();
        let panicking = (u8::MAX, 4004u64, String::from("panic")).into_c_stack_list();
        let recovered = (11u8, 9004u64, String::from("recovered")).into_c_stack_list();

        assert_eq!(direct.call(&first).unwrap(), "7/1001/alpha");
        assert_eq!(emitted.call(&second).unwrap(), "9/7002/second-longer");
        assert!(direct.call(&rejected).is_err());
        assert!(emitted.call(&rejected).is_err());
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                emitted.call(&panicking)
            }))
            .is_err()
        );
        assert_eq!(emitted.call(&recovered).unwrap(), "11/9004/recovered");
        assert_eq!(direct.call(&second).unwrap(), "9/7002/second-longer");
        for (frame, expected) in [
            (&first, (7, 1001, "alpha")),
            (&second, (9, 7002, "second-longer")),
            (&rejected, (0, 3003, "reject")),
            (&panicking, (u8::MAX, 4004, "panic")),
            (&recovered, (11, 9004, "recovered")),
        ] {
            assert_eq!((frame.1, frame.0.1, frame.0.0.1.as_str()), expected);
        }
    }

    /// Restores outer arguments after typed nested calls and rejects same-carrier reentry.
    #[test]
    fn typed_interpreted_nested_frames_and_reentry() {
        let target = Rc::new(OnceCell::<Weak<Callable<MixedFrame, String>>>::new());
        let reentrant = target.clone();
        let mut body = DynSegment::new::<()>();
        body.push_closure(mixed_closure()).unwrap();
        body.just(5u8);
        body.just(600u64);
        body.just(String::from("nested"));
        body.invoke_closure(3).unwrap();
        body.push_argument(2, ValueType::cloneable_leaf::<String>())
            .unwrap();
        body.op2(|nested: String, outer: String| format!("{nested}|parent:{outer}"))
            .unwrap();
        body.push_argument(0, ValueType::cloneable_leaf::<u8>())
            .unwrap();
        body.push_argument(1, ValueType::cloneable_leaf::<u64>())
            .unwrap();
        body.op3r(move |rendered: String, tag: u8, word: u64| {
            if tag == 13 {
                let function = reentrant.get().unwrap().upgrade().unwrap();
                return function
                    .call(&(99u8, 999u64, String::from("reentrant")).into_c_stack_list());
            }
            Ok(format!("{tag}/{word}:{rendered}"))
        })
        .unwrap();
        let closure = DynClosure::typed(
            vec![
                ValueType::cloneable_leaf::<u8>(),
                ValueType::cloneable_leaf::<u64>(),
                ValueType::cloneable_leaf::<String>(),
            ],
            body,
        )
        .unwrap();
        let direct = Rc::new(mixed_callable(closure.clone()));
        target.set(Rc::downgrade(&direct)).unwrap();
        let mut emitted = mixed_invocation(closure);
        let first = (7u8, 1001u64, String::from("alpha")).into_c_stack_list();
        let recursive = (13u8, 2002u64, String::from("recursive")).into_c_stack_list();
        let after = (9u8, 7002u64, String::from("after")).into_c_stack_list();
        assert_eq!(
            emitted.call(&first).unwrap(),
            "7/1001:5/600/nested|parent:alpha"
        );
        let error = emitted.call(&recursive).unwrap_err();
        assert!(error.to_string().contains("already executing"));
        assert_eq!(
            direct.call(&after).unwrap(),
            "9/7002:5/600/nested|parent:after"
        );
        assert_eq!(first.0.0.1, "alpha");
        assert_eq!(recursive.0.0.1, "recursive");
        assert_eq!(after.0.0.1, "after");
    }

    /// Both typed body kinds and emitted invocation perform zero descriptor comparisons.
    #[test]
    fn trusted_callable_calls_never_compare_descriptors() {
        /// Describes the borrowed array argument shared by the compared call routes.
        type Args = Cons<ArrayOf<Leaf<i32>>, Nil>;
        /// Describes the callable's argument and independently owned array result.
        type Function = FnOf<Args, ArrayOf<Leaf<i32>>>;
        /// Returns an independently owned copy of the borrowed array.
        fn native(args: &<Args as ValueRepr>::Value) -> anyhow::Result<Array<i32>> {
            Ok(args.1.clone())
        }
        let mut body = DynSegment::new::<()>();
        body.push_argument(0, ValueType::array(ValueType::cloneable_leaf::<i32>()))
            .unwrap();
        let interpreted =
            Callable::from_prepared(body.prepare::<Args, ArrayOf<Leaf<i32>>>().unwrap());
        let args = (Array::from_vec(vec![7]),).into_c_stack_list();
        for f in [Callable::from_fn(native), interpreted] {
            let direct = f.clone();
            let mut body = DynSegment::new::<()>();
            body.op0_repr::<Function, _>(move || f.clone()).unwrap();
            body.push_argument(0, ValueType::array(ValueType::cloneable_leaf::<i32>()))
                .unwrap();
            body.invoke_closure(1).unwrap();
            let mut body = body.prepare::<Args, ArrayOf<Leaf<i32>>>().unwrap();
            crate::value_type::SHAPE_COMPARISONS.with(|n| n.set(0));
            for _ in 0..3 {
                assert_eq!(direct.call(&args).unwrap().as_slice(), [7]);
                assert_eq!(body.call(&args).unwrap().as_slice(), [7]);
            }
            crate::value_type::SHAPE_COMPARISONS.with(|n| assert_eq!(n.get(), 0));
        }

        let mut body = DynSegment::new::<()>();
        body.just(7i32);
        let closure = DynClosure::typed(vec![], body).unwrap();
        let mut outer = DynSegment::new::<()>();
        outer.push_closure(closure).unwrap();
        let mut outer = outer.prepare::<Nil, FnOf<Nil, Leaf<i32>>>().unwrap();
        let f = outer.call(&().into_c_stack_list()).unwrap();
        drop(outer);
        crate::value_type::SHAPE_COMPARISONS.with(|n| n.set(0));
        assert_eq!(f.call(&().into_c_stack_list()).unwrap(), 7);
        assert_eq!(f.call(&().into_c_stack_list()).unwrap(), 7);
        crate::value_type::SHAPE_COMPARISONS.with(|n| assert_eq!(n.get(), 0));
    }
}
