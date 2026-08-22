//! A first-class, callable CEL value: a compiled body plus its declared signature.
//!
//! See `docs/superpowers/specs/2026-08-21-cel-closures-design.md` for the full design rationale
//! (why `Rc`, why `RefCell`, why no captured environment).
//!
//! # Examples
//!
//! Build a closure that adds two `i32` values, then call it with different arguments:
//!
//! ```rust
//! use cel_runtime::{DynClosure, DynSegment};
//! use std::any::TypeId;
//!
//! // Compile a body that takes two i32 arguments and returns their sum.
//! let mut body = DynSegment::new::<()>();
//! body.push_arg::<i32>(0);
//! body.push_arg::<i32>(1);
//! body.op2(|a: i32, b: i32| a + b).unwrap();
//!
//! // Wrap it as a callable closure with its declared signature.
//! let closure = DynClosure::new(
//!     vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
//!     TypeId::of::<i32>(),
//!     body,
//! );
//!
//! // Call the closure multiple times with different arguments.
//! let (a, b) = (2i32, 3i32);
//! assert_eq!(closure.call::<i32>(&[&a, &b]).unwrap(), 5);
//!
//! let (c, d) = (10i32, 20i32);
//! assert_eq!(closure.call::<i32>(&[&c, &d]).unwrap(), 30);
//! ```

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::rc::Rc;

use crate::dyn_segment::DynSegment;

struct ClosureData {
    param_types: Vec<TypeId>,
    return_type: TypeId,
    body: RefCell<DynSegment>,
}

/// `Debug` implementation that formats the closure's signature but deliberately omits `body` from the output
/// (via `finish_non_exhaustive`), since `DynSegment` is not easily debuggable and the signature alone
/// is typically what callers need to see.
impl std::fmt::Debug for ClosureData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClosureData")
            .field("param_types", &self.param_types)
            .field("return_type", &self.return_type)
            .finish_non_exhaustive()
    }
}

/// A first-class, callable CEL value: a compiled body plus its declared parameter/return types.
///
/// Holds no captured environment — only its own parameters resolve inside `body`. Calling it
/// twice with different `args` is exactly as fresh each time as calling any other `DynSegment`.
/// `Rc`-wrapped so `Clone` never requires `DynSegment` itself to implement `Clone` (it doesn't —
/// see the design spec); `RefCell` because callers only ever reach a `DynClosure` through `&self`
/// (matching `adam_rs::Filter`/`Method`'s `Fn`, not `FnMut`, storage), while `DynSegment`'s own
/// call methods need `&mut self`.
#[derive(Clone, Debug)]
pub struct DynClosure(Rc<ClosureData>);

impl DynClosure {
    /// Wraps `body` as a closure value declaring `param_types` (in order) and `return_type`.
    ///
    /// - Precondition: `body` was compiled expecting exactly `param_types.len()` positional
    ///   arguments (via `push_arg`/`push_arg_as_dynamic_sequence_tuple`), in that order, and
    ///   produces exactly one result of `return_type`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{DynClosure, DynSegment};
    /// use std::any::TypeId;
    ///
    /// let mut body = DynSegment::new::<()>();
    /// body.push_arg::<i32>(0);
    /// body.push_arg::<i32>(1);
    /// body.op2(|a: i32, b: i32| a + b).unwrap();
    ///
    /// let closure = DynClosure::new(
    ///     vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
    ///     TypeId::of::<i32>(),
    ///     body,
    /// );
    /// assert_eq!(closure.param_types().len(), 2);
    /// ```
    #[must_use]
    pub fn new(param_types: Vec<TypeId>, return_type: TypeId, body: DynSegment) -> Self {
        DynClosure(Rc::new(ClosureData {
            param_types,
            return_type,
            body: RefCell::new(body),
        }))
    }

    /// Returns this closure's declared parameter types, in order.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{DynClosure, DynSegment};
    /// use std::any::TypeId;
    ///
    /// let mut body = DynSegment::new::<()>();
    /// body.push_arg::<i32>(0);
    /// body.push_arg::<i32>(1);
    /// body.op2(|a: i32, b: i32| a + b).unwrap();
    ///
    /// let closure = DynClosure::new(
    ///     vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
    ///     TypeId::of::<i32>(),
    ///     body,
    /// );
    /// assert_eq!(closure.param_types(), &[TypeId::of::<i32>(), TypeId::of::<i32>()]);
    /// ```
    #[must_use]
    pub fn param_types(&self) -> &[TypeId] {
        &self.0.param_types
    }

    /// Returns this closure's declared return type.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{DynClosure, DynSegment};
    /// use std::any::TypeId;
    ///
    /// let mut body = DynSegment::new::<()>();
    /// body.push_arg::<i32>(0);
    /// body.push_arg::<i32>(1);
    /// body.op2(|a: i32, b: i32| a + b).unwrap();
    ///
    /// let closure = DynClosure::new(
    ///     vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
    ///     TypeId::of::<i32>(),
    ///     body,
    /// );
    /// assert_eq!(closure.return_type(), TypeId::of::<i32>());
    /// ```
    #[must_use]
    pub fn return_type(&self) -> TypeId {
        self.0.return_type
    }

    /// Invokes the closure's body with `args`, positionally matched against `param_types`.
    ///
    /// - Precondition: `args.len() == self.param_types().len()` and each `args[i]`'s runtime type
    ///   matches `param_types()[i]` — the caller (adam-lang's generated `Filter` wrapper, or any
    ///   future caller) must guarantee this ahead of time; a violation is a caller bug, not user
    ///   error (matches `DynSegment::push_arg`'s own existing precondition, which this delegates
    ///   to unchanged).
    /// - Precondition: `TypeId::of::<R>() == self.return_type()`.
    /// - Complexity: whatever the body's own evaluation complexity is.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{DynClosure, DynSegment};
    /// use std::any::TypeId;
    ///
    /// let mut body = DynSegment::new::<()>();
    /// body.push_arg::<i32>(0);
    /// body.push_arg::<i32>(1);
    /// body.op2(|a: i32, b: i32| a + b).unwrap();
    ///
    /// let closure = DynClosure::new(
    ///     vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
    ///     TypeId::of::<i32>(),
    ///     body,
    /// );
    /// let (a, b) = (2i32, 3i32);
    /// let result: i32 = closure.call(&[&a, &b]).unwrap();
    /// assert_eq!(result, 5);
    /// ```
    pub fn call<R: 'static>(&self, args: &[&dyn Any]) -> anyhow::Result<R> {
        debug_assert_eq!(args.len(), self.0.param_types.len());
        debug_assert_eq!(TypeId::of::<R>(), self.0.return_type);
        self.0.body.borrow_mut().call_dyn::<R>(args)
    }

    /// Invokes the closure's body with `args`, like [`call`](Self::call), for a caller that only
    /// knows the return type dynamically (as a `TypeId`) rather than as a static Rust generic —
    /// `call_dyn_fn` is a monomorphized dispatcher the caller already has for that type (e.g.
    /// `adam-lang`'s per-type `TypeRegistry::TypeEntry::call_dyn_fn`).
    ///
    /// - Precondition: `args.len() == self.param_types().len()` and each `args[i]`'s runtime type
    ///   matches `param_types()[i]`, exactly as for [`call`](Self::call).
    /// - Precondition: `call_dyn_fn` is a dispatcher for `self.return_type()` (i.e. it calls
    ///   `DynSegment::call_dyn::<R>` for the same concrete `R` that `TypeId` names).
    /// - Complexity: whatever the body's own evaluation complexity is.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{DynClosure, DynSegment};
    /// use std::any::{Any, TypeId};
    ///
    /// let mut body = DynSegment::new::<()>();
    /// body.push_arg::<i32>(0);
    /// body.push_arg::<i32>(1);
    /// body.op2(|a: i32, b: i32| a + b).unwrap();
    ///
    /// let closure = DynClosure::new(
    ///     vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
    ///     TypeId::of::<i32>(),
    ///     body,
    /// );
    ///
    /// fn dispatch_i32(seg: &mut DynSegment, inputs: &[&dyn Any]) -> anyhow::Result<Box<dyn Any>> {
    ///     let result: i32 = seg.call_dyn(inputs)?;
    ///     Ok(Box::new(result))
    /// }
    ///
    /// let (a, b) = (4i32, 5i32);
    /// let result = closure.call_boxed(&[&a, &b], dispatch_i32).unwrap();
    /// assert_eq!(*result.downcast_ref::<i32>().unwrap(), 9);
    /// ```
    pub fn call_boxed(
        &self,
        args: &[&dyn Any],
        call_dyn_fn: fn(&mut DynSegment, &[&dyn Any]) -> anyhow::Result<Box<dyn Any>>,
    ) -> anyhow::Result<Box<dyn Any>> {
        debug_assert_eq!(args.len(), self.0.param_types.len());
        call_dyn_fn(&mut self.0.body.borrow_mut(), args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dyn_segment::DynSegment;

    fn adder_closure() -> DynClosure {
        let mut body = DynSegment::new::<()>();
        body.push_arg::<i32>(0);
        body.push_arg::<i32>(1);
        body.op2(|a: i32, b: i32| a + b).unwrap();
        DynClosure::new(
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
            TypeId::of::<i32>(),
            body,
        )
    }

    #[test]
    fn call_invokes_body_with_positional_args() {
        let closure = adder_closure();
        let (a, b) = (2i32, 3i32);
        let result: i32 = closure.call(&[&a, &b]).unwrap();
        assert_eq!(result, 5);
    }

    #[test]
    fn call_is_repeatable_with_different_args() {
        let closure = adder_closure();
        let (a1, b1) = (2i32, 3i32);
        assert_eq!(closure.call::<i32>(&[&a1, &b1]).unwrap(), 5);
        let (a2, b2) = (10i32, 20i32);
        assert_eq!(closure.call::<i32>(&[&a2, &b2]).unwrap(), 30);
    }

    #[test]
    fn clone_shares_the_same_body_and_both_remain_callable() {
        let closure = adder_closure();
        let cloned = closure.clone();
        let (a, b) = (1i32, 1i32);
        assert_eq!(closure.call::<i32>(&[&a, &b]).unwrap(), 2);
        assert_eq!(cloned.call::<i32>(&[&a, &b]).unwrap(), 2);
    }

    #[test]
    fn call_boxed_dispatches_through_a_supplied_call_dyn_fn() {
        let closure = adder_closure();
        fn call_dyn_fn(seg: &mut DynSegment, inputs: &[&dyn Any]) -> anyhow::Result<Box<dyn Any>> {
            let v: i32 = seg.call_dyn(inputs)?;
            Ok(Box::new(v))
        }
        let (a, b) = (4i32, 5i32);
        let result = closure.call_boxed(&[&a, &b], call_dyn_fn).unwrap();
        assert_eq!(*result.downcast_ref::<i32>().unwrap(), 9);
    }

    #[test]
    fn param_types_and_return_type_are_queryable() {
        let closure = adder_closure();
        assert_eq!(
            closure.param_types(),
            &[TypeId::of::<i32>(), TypeId::of::<i32>()]
        );
        assert_eq!(closure.return_type(), TypeId::of::<i32>());
    }
}
