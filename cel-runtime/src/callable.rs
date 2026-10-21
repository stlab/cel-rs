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
    carrier: DynClosure,
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

    /// Both typed body kinds and emitted invocation perform zero descriptor comparisons.
    #[test]
    fn trusted_callable_calls_never_compare_descriptors() {
        type Args = Cons<ArrayOf<Leaf<i32>>, Nil>;
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
