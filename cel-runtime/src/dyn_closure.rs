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

use crate::ValueType;
use crate::dyn_segment::DynSegment;

/// An exact compile-time callable shape, including nested aggregate and callable results.
#[derive(Clone, Debug)]
pub(crate) struct ClosureSignature {
    pub(crate) parameters: Vec<ValueType>,
    pub(crate) result: ValueType,
}

/// Compares complete known signatures without relying on the erased closure marker.
///
/// - Complexity: O(number of recursive type nodes).
pub(crate) fn same_signature(
    left: Option<&ClosureSignature>,
    right: Option<&ClosureSignature>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.parameters.len() == b.parameters.len()
                && a.parameters
                    .iter()
                    .zip(&b.parameters)
                    .all(|(a, b)| a.same_shape(b))
                && a.result.same_shape(&b.result)
        }
        _ => false,
    }
}

/// A monomorphized dispatcher for [`DynClosure::call_boxed`]: runs a closure's body via
/// `DynSegment::call_dyn::<R>` for some concrete `R` the dispatcher itself already knows, boxing
/// the result so the caller (which only knows `R` dynamically, as a `TypeId`) doesn't need to.
pub type CallDynFn = fn(&mut DynSegment, &[&dyn Any]) -> anyhow::Result<Box<dyn Any>>;

struct ClosureData {
    param_types: Vec<TypeId>,
    return_type: TypeId,
    body: RefCell<DynSegment>,
    signature: Option<std::sync::Arc<ClosureSignature>>,
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
            signature: None,
        }))
    }

    /// Wraps a checked CEL body while retaining its full callable shape.
    ///
    /// # Errors
    /// Returns an error if the body does not produce exactly one value, requires preloaded
    /// arguments, reads an undeclared or differently shaped argument, or reads a tuple whose
    /// leaves lack clone adapters.
    ///
    /// - Complexity: O(number of parameters plus recursive nodes in body argument reads/result).
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::{DynClosure, DynSegment, ValueType};
    /// let mut body = DynSegment::new::<()>();
    /// body.just(42i32);
    /// let closure = DynClosure::typed(vec![], body).unwrap();
    /// assert_eq!(closure.call::<i32>(&[]).unwrap(), 42);
    /// ```
    pub fn typed(parameters: Vec<ValueType>, body: DynSegment) -> anyhow::Result<Self> {
        anyhow::ensure!(
            body.stack_ids.len() == 1,
            "closure body must produce exactly one value"
        );
        anyhow::ensure!(
            body.argument_ids.is_empty(),
            "closure body must not require preloaded arguments"
        );
        for (index, read_type) in &body.argument_reads {
            anyhow::ensure!(
                parameters
                    .get(*index)
                    .is_some_and(|expected| expected.same_shape(read_type)),
                "closure body argument {index} differs from its declared signature"
            );
            if read_type.tuple_elements().is_some() {
                read_type.validate_clone()?;
            }
        }
        let result = body.peek_stack_infos(1)[0].value_type.clone();
        let param_types = parameters
            .iter()
            .map(|p| {
                if p.tuple_elements().is_some() {
                    TypeId::of::<crate::DynamicSequence>()
                } else {
                    p.type_id()
                }
            })
            .collect();
        Ok(Self(Rc::new(ClosureData {
            param_types,
            return_type: result.type_id(),
            signature: Some(std::sync::Arc::new(ClosureSignature { parameters, result })),
            body: RefCell::new(body),
        })))
    }

    /// Returns the metadata retained for a typed CEL closure.
    pub(crate) fn signature(&self) -> Option<std::sync::Arc<ClosureSignature>> {
        self.0.signature.clone()
    }

    /// Evaluates a previously checked borrowed raw argument frame into an independent stack.
    ///
    /// # Safety
    /// Every pointer must reference a live value matching `expected.parameters`, in order,
    /// until execution returns. No pointer may be retained by an operation.
    ///
    /// # Errors
    /// Returns signature mismatch, reentrant body borrow, or body evaluation errors.
    /// - Complexity: body-dependent.
    pub(crate) unsafe fn call_raw(
        &self,
        expected: &ClosureSignature,
        arguments: &[*const u8],
    ) -> anyhow::Result<crate::raw_stack::RawStack> {
        debug_assert_eq!(expected.parameters.len(), arguments.len());
        anyhow::ensure!(
            same_signature(self.0.signature.as_deref(), Some(expected)),
            "closure signature mismatch"
        );
        let mut body = self
            .0
            .body
            .try_borrow_mut()
            .map_err(|_| anyhow::anyhow!("closure body is already executing"))?;
        unsafe { body.execute_raw(arguments) }
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
        call_dyn_fn: CallDynFn,
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

    /// A typed body cannot reinterpret an argument through a different Rust representation.
    #[test]
    fn typed_rejects_incompatible_body_argument_reads() {
        let mut body = DynSegment::new::<()>();
        body.push_arg::<String>(0);
        assert!(DynClosure::typed(vec![ValueType::leaf::<i32>()], body).is_err());
    }

    /// An undeclared positional argument cannot enter a typed raw call frame.
    #[test]
    fn typed_rejects_undeclared_body_argument_reads() {
        let mut body = DynSegment::new::<()>();
        body.push_arg::<i32>(1);
        assert!(DynClosure::typed(vec![ValueType::leaf::<i32>()], body).is_err());
    }

    /// A runtime closure value cannot masquerade as another known callable signature.
    #[test]
    fn invocation_checks_runtime_signature_before_body_execution() {
        let mut expected_body = DynSegment::new::<()>();
        expected_body.just(1i32);
        let expected = DynClosure::typed(vec![], expected_body).unwrap();
        let executed = Rc::new(std::cell::Cell::new(false));
        let observed = executed.clone();
        let mut actual_body = DynSegment::new::<()>();
        actual_body.op0(move || {
            observed.set(true);
            true
        });
        let actual = DynClosure::typed(vec![], actual_body).unwrap();
        // Both signatures have no parameters, so the borrowed argument frame is valid.
        let error = unsafe { actual.call_raw(&expected.signature().unwrap(), &[]) }.unwrap_err();
        assert!(error.to_string().contains("signature mismatch"));
        assert!(!executed.get());
    }

    /// Argument reads inside alternate fragments participate in signature validation.
    #[test]
    fn typed_checks_conditional_argument_reads() {
        let mut body = DynSegment::new::<()>();
        body.just(true);
        let mut then_branch = body.new_fragment();
        then_branch.push_arg::<String>(0);
        let mut else_branch = body.new_fragment();
        else_branch.just(String::new());
        body.join2(then_branch, else_branch).unwrap();
        assert!(DynClosure::typed(vec![ValueType::leaf::<i32>()], body).is_err());
    }

    /// Rejected invocation leaves the evaluated operand descriptors and operations unchanged.
    #[test]
    fn wrong_argument_type_is_rejected_before_emitting() {
        let mut body = DynSegment::new::<()>();
        body.push_arg::<i32>(0);
        let closure = DynClosure::typed(vec![ValueType::leaf::<i32>()], body).unwrap();
        let mut caller = DynSegment::new::<()>();
        caller.push_closure(closure);
        caller.just(false);
        assert!(caller.invoke_closure(1).is_err());
        assert_eq!(caller.peek_stack_infos(2).len(), 2);
        caller.op1(|_: bool| 8i32).unwrap();
        caller.invoke_closure(1).unwrap();
        assert_eq!(caller.call0::<i32>().unwrap(), 8);
    }

    /// Preloaded Rust argument layouts cannot enter the positional CEL invocation adapter.
    #[test]
    fn preloaded_arguments_fail_explicitly() {
        let mut body = DynSegment::new::<()>();
        body.just(1i32);
        let mut caller = DynSegment::new::<(i32,)>();
        caller.push_closure(DynClosure::typed(vec![], body).unwrap());
        assert!(caller.invoke_closure(0).is_err());
    }
}
