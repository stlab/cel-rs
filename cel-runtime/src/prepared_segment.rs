//! Executes one construction-validated canonical borrowed frame.
//!
//! ```
//! use cel_runtime::{DynSegment, Nil, Leaf, IntoCStackList};
//! let mut body = DynSegment::new::<()>();
//! body.just(7i32);
//! let mut prepared = body.prepare::<Nil, Leaf<i32>>().unwrap();
//! assert_eq!(prepared.call(&().into_c_stack_list()).unwrap(), 7);
//! ```

use std::marker::PhantomData;

use crate::container_execution::{ArgumentPlan, ResultPlan};
use crate::{DynSegment, ValueRepr};

/// Owns the same frozen raw engine as [`PreparedSegment`] with runtime-bound witnesses.
///
/// Hosts validate their storage adapters once, then lend exact canonical pointers.
pub struct PreparedRawSegment {
    /// Owns the validated independent instruction body.
    body: DynSegment,
    /// Binds exact result ownership and extraction.
    result: ResultPlan,
    /// Counts the borrowed positional pointers required on each call.
    arity: usize,
}

impl PreparedRawSegment {
    /// Binds every argument read and the complete result before any operation executes.
    ///
    /// # Errors
    /// Rejects dependent bodies, undeclared or mismatched reads, missing cloning,
    /// and a mismatched result.
    /// - Complexity: O(n) in descriptor nodes and emitted reads.
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::{DynSegment, ValueType, prepared_segment::PreparedRawSegment};
    /// let mut body = DynSegment::new::<()>();
    /// body.just(7i32);
    /// let mut body = PreparedRawSegment::new(body, &[], &ValueType::leaf::<i32>())?;
    /// let result = unsafe { body.call(&[]) }?;
    /// assert_eq!(result.try_into_typed::<cel_runtime::Leaf<i32>>()?, 7);
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn new(
        body: DynSegment,
        arguments: &[crate::ValueType],
        result: &crate::ValueType,
    ) -> anyhow::Result<Self> {
        body.validate_independent_body()?;
        for (index, read) in &body.argument_reads {
            let expected = arguments
                .get(*index)
                .ok_or_else(|| anyhow::anyhow!("undeclared argument {index}"))?;
            anyhow::ensure!(
                expected.same_shape(read),
                "argument {index} representation mismatch"
            );
            anyhow::ensure!(
                read.can_clone(),
                "argument {index} clone capability unavailable"
            );
        }
        anyhow::ensure!(
            body.result_type()?.same_shape(result),
            "result representation mismatch"
        );
        let result = ResultPlan::new(0, body.result_type()?.clone())?;
        Ok(Self {
            body,
            result,
            arity: arguments.len(),
        })
    }

    /// Executes the bound raw frame without conversions or descriptor comparisons.
    ///
    /// # Safety
    /// `arguments` must have the prepared arity. Each pointer must borrow a live,
    /// aligned value of its exact preparation descriptor throughout this call.
    /// No pointer may alias mutable storage accessed by an operation.
    /// # Errors
    /// Propagates operation errors after frame cleanup.
    /// - Complexity: Body execution and emitted clone costs.
    ///
    /// # Examples
    /// See [`Self::new`].
    pub unsafe fn call(&mut self, arguments: &[*const u8]) -> anyhow::Result<crate::OwnedValue> {
        debug_assert_eq!(arguments.len(), self.arity);
        let mut output = unsafe { self.body.execute_raw(arguments)? };
        Ok(unsafe { self.result.take(&mut output) })
    }
}

/// Owns a frozen raw body with exact canonical argument/result witnesses.
///
/// Calls borrow arguments, clone only emitted reads, and return independent owners.
pub struct PreparedSegment<A: ValueRepr, R: ValueRepr> {
    /// Owns the frozen instruction body.
    body: DynSegment,
    /// Binds the canonical borrowed argument tuple layout.
    arguments: ArgumentPlan,
    /// Reuses fixed-arity pointer slots, rebound before each call.
    pointers: Vec<*const u8>,
    /// Binds exact result ownership and extraction.
    result: ResultPlan,
    /// Associates the validated body with its concrete boundary witnesses.
    witness: PhantomData<fn(A, R)>,
}

impl<A: ValueRepr, R: ValueRepr> PreparedSegment<A, R> {
    /// Validates exact read/result identities and borrowed clone capabilities once.
    ///
    /// # Errors
    /// Rejects dependent bodies, non-list arguments, missing/mismatched positional
    /// reads, missing clone capability, or a result differing at any recursive node.
    /// - Complexity: O(n) complete descriptor comparisons and frame planning.
    pub(crate) fn new(body: DynSegment) -> anyhow::Result<Self> {
        body.validate_independent_body()?;
        let args = A::value_type();
        let fields = args
            .tuple_elements()
            .ok_or_else(|| anyhow::anyhow!("prepared arguments must be a canonical list"))?;
        for (index, read) in &body.argument_reads {
            let expected = fields
                .get(*index)
                .ok_or_else(|| anyhow::anyhow!("undeclared argument {index}"))?;
            anyhow::ensure!(
                expected.value_type.same_shape(read),
                "argument {index} representation mismatch"
            );
            anyhow::ensure!(
                read.can_clone(),
                "argument {index} clone capability unavailable"
            );
        }
        body.result_type()?.validate::<R>()?;
        let result = ResultPlan::new(0, body.result_type()?.clone())?;
        let pointers = vec![std::ptr::null(); fields.len()];
        Ok(Self {
            body,
            arguments: ArgumentPlan::borrowed(args)?,
            pointers,
            result,
            witness: PhantomData,
        })
    }

    /// Executes without shape comparisons, checked conversions, or `Any` downcasts.
    ///
    /// - Postcondition: The input remains borrowed and every returned owner is independent.
    /// # Errors
    /// Propagates the original operation error and its context after frame cleanup.
    /// - Complexity: O(a) pointer setup for a arguments plus body/clone costs.
    ///
    /// # Examples
    /// See the module example.
    pub fn call(&mut self, args: &A::Value) -> anyhow::Result<R::Value> {
        let base = std::ptr::from_ref(args).cast::<u8>();
        // SAFETY: The sealed witnesses, immutable body, and preparation bind all
        // reads/results to this exact representation for every subsequent call.
        unsafe {
            self.arguments.bind_pointers(base, &mut self.pointers);
            let mut output = self.body.execute_raw(&self.pointers)?;
            Ok(self.result.take_typed::<R::Value>(&mut output))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Array, ArrayOf, Cons, IntoCStackList, Leaf, Nil, ValueType};

    /// Prepared nested borrowed frames perform no recursive comparisons on repeated calls.
    #[test]
    fn trusted_calls_never_compare_descriptors() {
        type Pair = Cons<Leaf<i32>, Cons<Leaf<f64>, Nil>>;
        type Argument = ArrayOf<Pair>;
        type Args = Cons<Argument, Nil>;
        let mut body = DynSegment::new::<()>();
        body.push_argument(0, Argument::value_type().with_clone::<Argument>().unwrap())
            .unwrap();
        let mut body = body.prepare::<Args, Argument>().unwrap();
        let input = (Array::from_vec(vec![(1i32, 2f64).into_c_stack_list()]),).into_c_stack_list();
        crate::value_type::SHAPE_COMPARISONS.with(|count| count.set(0));
        let first = body.call(&input).unwrap();
        let second = body.call(&input).unwrap();
        crate::value_type::SHAPE_COMPARISONS.with(|count| assert_eq!(count.get(), 0));
        assert_ne!(first.as_slice().as_ptr(), second.as_slice().as_ptr());
        assert_ne!(input.1.as_slice().as_ptr(), first.as_slice().as_ptr());
    }

    /// Runtime-bound frames use the identical frozen execution and equality plans.
    #[test]
    fn erased_prepared_calls_never_compare_descriptors() {
        type Pair = Cons<Leaf<i32>, Cons<ArrayOf<Leaf<f64>>, Nil>>;
        type Argument = ArrayOf<Pair>;
        let descriptor = Argument::value_type()
            .with_clone::<Argument>()
            .unwrap()
            .with_equality::<Argument>()
            .unwrap();
        let mut body = DynSegment::new::<()>();
        body.push_argument(0, descriptor.clone()).unwrap();
        let mut body =
            PreparedRawSegment::new(body, std::slice::from_ref(&descriptor), &descriptor).unwrap();
        let input = Array::from_vec(vec![(1, Array::from_vec(vec![2.0])).into_c_stack_list()]);
        let pointer = std::ptr::from_ref(&input).cast::<u8>();
        crate::value_type::SHAPE_COMPARISONS.with(|count| count.set(0));
        // SAFETY: Input uses exactly the witness bound at construction.
        let first = unsafe { body.call(&[pointer]) }.unwrap();
        let second = unsafe { body.call(&[pointer]) }.unwrap();
        // SAFETY: Both values come from the same comparable bound result plan.
        assert!(unsafe { first.eq_bound(&second) });
        crate::value_type::SHAPE_COMPARISONS.with(|count| assert_eq!(count.get(), 0));
        let first = first.try_into_typed::<Argument>().unwrap();
        let second = second.try_into_typed::<Argument>().unwrap();
        assert_ne!(input.as_slice().as_ptr(), first.as_slice().as_ptr());
        assert_ne!(first.as_slice().as_ptr(), second.as_slice().as_ptr());
        assert_ne!(
            first.as_slice()[0].0.1.as_slice().as_ptr(),
            second.as_slice()[0].0.1.as_slice().as_ptr()
        );
    }

    /// Erased preparation rejects mismatched reads and results before executing effects.
    #[test]
    fn erased_preparation_checks_complete_signature_before_effects() {
        let mut body = DynSegment::new::<()>();
        body.push_arg::<i32>(0);
        assert!(
            PreparedRawSegment::new(body, &[ValueType::leaf::<u32>()], &ValueType::leaf::<i32>())
                .is_err()
        );
        let mut body = DynSegment::new::<()>();
        body.push_arg::<i32>(0);
        assert!(PreparedRawSegment::new(body, &[], &ValueType::leaf::<i32>()).is_err());
        let mut body = DynSegment::new::<()>();
        body.just(1i32);
        assert!(PreparedRawSegment::new(body, &[], &ValueType::leaf::<u32>()).is_err());
        let body = DynSegment::new::<(i32,)>();
        assert!(PreparedRawSegment::new(body, &[], &ValueType::leaf::<i32>()).is_err());
    }

    /// Exact argument/result identities, independent origins, and clone plans bind once.
    #[test]
    fn preparation_rejects_every_boundary_mismatch() {
        let mut wrong_result = DynSegment::new::<()>();
        wrong_result.just(1i32);
        assert!(wrong_result.prepare::<Nil, Leaf<u32>>().is_err());

        let mut wrong_arg = DynSegment::new::<()>();
        wrong_arg.push_arg::<i32>(0);
        assert!(
            wrong_arg
                .prepare::<Cons<Leaf<u32>, Nil>, Leaf<i32>>()
                .is_err()
        );

        let mut undeclared = DynSegment::new::<()>();
        undeclared.push_arg::<i32>(1);
        assert!(
            undeclared
                .prepare::<Cons<Leaf<i32>, Nil>, Leaf<i32>>()
                .is_err()
        );

        let mut nonlist = DynSegment::new::<()>();
        nonlist.just(1i32);
        assert!(nonlist.prepare::<Leaf<()>, Leaf<i32>>().is_err());

        let preloaded = DynSegment::new::<(i32,)>();
        assert!(
            preloaded
                .prepare::<Cons<Leaf<i32>, Nil>, Leaf<i32>>()
                .is_err()
        );

        let mut nonclone = DynSegment::new::<()>();
        assert!(
            nonclone
                .push_argument(0, ValueType::leaf::<String>())
                .is_err()
        );
        assert!(nonclone.peek_stack_infos(0).is_empty());
    }
}
