//! Input filters: idempotent, per-cell domain constraints.
//!
//! A [`Filter`] conforms or rejects a value written externally to its cell, applied
//! live by [`crate::sheet::Sheet::propagate`] against a source cell's own current
//! value (see [`crate::sheet::Sheet::add_filter`]), and is separately re-evaluated as
//! a non-gating diagnostic against a value a relationship's method derives for that
//! cell (also in [`crate::sheet::Sheet::propagate`]).
//!
//! ```
//! use adam_rs::{Filter, Sheet};
//! use std::any::TypeId;
//! let mut sheet = Sheet::new();
//! let bound = sheet.add_source(10_i32);
//! let low = sheet.add_source(0_i32);
//! let raw = sheet.add_cell(5_i32);
//! let zero = sheet.add_cell(-5_i32);
//! let one = sheet.add_cell(20_i32);
//! let two = sheet.add_cell(30_i32);
//! sheet.add_filter(raw, Filter::new(TypeId::of::<i32>(), vec![], vec![],
//!     |v, _| Ok(Box::new(*v.downcast_ref::<i32>().unwrap()))))?;
//! sheet.add_filter(zero, Filter::from_fn_0(|v: &i32| Ok((*v).max(0))))?;
//! sheet.add_filter(one, Filter::from_fn_1(bound, |v: &i32, hi: &i32| Ok((*v).min(*hi))))?;
//! sheet.add_filter(two, Filter::from_fn_2([low, bound],
//!     |v: &i32, lo: &i32, hi: &i32| Ok((*v).clamp(*lo, *hi))))?;
//! sheet.propagate()?;
//! assert_eq!(*sheet.read::<i32>(raw)?, 5);
//! assert_eq!(*sheet.read::<i32>(zero)?, 0);
//! assert_eq!(*sheet.read::<i32>(one)?, 10);
//! assert_eq!(*sheet.read::<i32>(two)?, 10);
//! assert_eq!(*sheet.source::<i32>(two)?, 30);
//! # Ok::<(), adam_rs::Error>(())
//! ```

use std::any::{Any, TypeId};

use crate::cell::CellId;

/// Type-erased function stored inside a [`FilterData`].
///
/// Takes the candidate value and a slice of the filter's argument cells' current
/// effective values, and returns the conformed value or an error.
type FilterFn = Box<dyn Fn(&dyn Any, &[&dyn Any]) -> Result<Box<dyn Any>, anyhow::Error>>;

/// What shape of validation/derivation a [`Filter`] performs, beyond its opaque function — set by
/// `adam-lang`'s compile phase when a filter's expression matches a recognized structural form.
/// `Opaque` carries no extra information; consumers that don't care about structure treat every
/// kind identically at write/propagate time — `FilterKind` is purely informational, queried by
/// consumers like `begin`'s UI that want to render a specialized editor without inspecting the
/// filter's function.
///
/// # Examples
/// See the live range filter in the [graph example](crate::sheet).
pub enum FilterKind {
    /// The filter's expression wasn't a recognized structural form.
    Opaque,
    /// Compiled from a `RangeInclusive<T>`-typed expression (`lo..=hi`). `bounds` re-evaluates
    /// that expression against the filter's current argument values, returning the resulting
    /// `(lo, hi)` as type-erased values of the filtered cell's own type `T`, or `None` if the
    /// underlying expression fails to evaluate (e.g. a fallible arithmetic op in a range
    /// endpoint, such as a `checked_*` operation overflowing).
    /// The bound evaluator satisfies [`Filter`]'s purity and invocation contract.
    Range {
        /// Re-evaluates the range expression, returning `(lo, hi)` as type-erased values, or
        /// `None` if evaluation fails.
        ///
        /// Adam-compiled evaluators also return `None` for missing or extra arguments,
        /// incorrect scalar identities, or mismatched compound carriers or shapes.
        /// They validate every argument before binding raw addresses or evaluating,
        /// including on calls following a successful evaluation or rejection.
        #[allow(clippy::type_complexity)]
        bounds: Box<dyn Fn(&[&dyn Any]) -> Option<(Box<dyn Any>, Box<dyn Any>)>>,
    },
}

/// An idempotent, per-cell domain constraint with optional dynamic arguments.
///
/// Constructed via [`Filter::from_fn_0`]/[`Filter::from_fn_1`]/[`Filter::from_fn_2`] for
/// the common typed cases, or [`Filter::new`] for the fully type-erased form. Attached
/// to a cell with [`crate::sheet::Sheet::add_filter`].
///
/// Its callbacks must be pure, deterministic functions of their input values and
/// immutable captured constants, with no externally observable side effects.
/// Equal inputs produce equivalent outputs or errors. There is no guarantee that
/// a callback executes, or when or how often it executes. This includes range-bound
/// evaluators; the conforming callback must also be idempotent.
/// Holding dynamic arguments fixed, conforming the returned value again produces
/// an equivalent value.
///
/// # Examples
/// See the [filter example](crate::filter).
pub struct Filter(pub(crate) FilterData);

/// Internal storage for a single filter.
///
/// Argument identities and declared types have equal lengths. `value_type` declares
/// the expected callback result type. Attachment validates argument identities and
/// types; propagation validates successful results and records a soft failure for
/// a result with a different runtime type.
pub(crate) struct FilterData {
    /// The `TypeId` of the value this filter operates on, validated against its cell's
    /// registered type by `add_filter`.
    pub(crate) value_type: TypeId,
    /// Dynamic argument cells, resolved via `effective()` wherever the filter runs.
    pub(crate) args: Vec<CellId>,
    pub(crate) arg_types: Vec<TypeId>,
    pub(crate) function: FilterFn,
    /// What shape of validation/derivation this filter performs, beyond `function` — see
    /// [`FilterKind`]. Purely informational; never consulted by `write`/`propagate`/`add_filter`.
    #[allow(dead_code)]
    pub(crate) kind: FilterKind,
}

impl Filter {
    /// Creates a filter from an explicit value `TypeId`, argument `TypeId`s, and a
    /// type-erased function.
    ///
    /// - Precondition: `args.len() == arg_types.len()`.
    /// - Precondition: `f` satisfies [`Filter`]'s purity and idempotence contract.
    ///
    /// Propagation records a soft [`FilterViolation::Failed`] when a successful
    /// callback result has a different runtime type from `value_type`.
    ///
    /// # Examples
    /// See the [filter example](crate::filter).
    #[must_use]
    pub fn new<F>(value_type: TypeId, args: Vec<CellId>, arg_types: Vec<TypeId>, f: F) -> Self
    where
        F: Fn(&dyn Any, &[&dyn Any]) -> Result<Box<dyn Any>, anyhow::Error> + 'static,
    {
        debug_assert_eq!(args.len(), arg_types.len());
        Filter(FilterData {
            value_type,
            args,
            arg_types,
            function: Box::new(f),
            kind: FilterKind::Opaque,
        })
    }

    /// Creates a filter with no dynamic arguments from a typed closure.
    ///
    /// The `TypeId` for `T` is captured automatically. The filter is validated against
    /// its cell registration when passed to [`crate::sheet::Sheet::add_filter`].
    ///
    /// - Precondition: `f` satisfies [`Filter`]'s purity and idempotence contract.
    ///
    /// # Examples
    /// See the [filter example](crate::filter).
    #[must_use]
    pub fn from_fn_0<T, F>(f: F) -> Self
    where
        T: Any + 'static,
        F: Fn(&T) -> Result<T, anyhow::Error> + 'static,
    {
        Filter::new(TypeId::of::<T>(), vec![], vec![], move |value, _args| {
            let value = value
                .downcast_ref::<T>()
                .expect("type checked at add_filter");
            Ok(Box::new(f(value)?) as Box<dyn Any>)
        })
    }

    /// Creates a filter with one dynamic argument cell from a typed closure.
    ///
    /// `TypeId`s for `A` and `T` are captured automatically. The filter is validated
    /// against its cell registration when passed to [`crate::sheet::Sheet::add_filter`].
    ///
    /// - Precondition: `f` satisfies [`Filter`]'s purity and idempotence contract.
    ///
    /// # Examples
    /// See the [filter example](crate::filter).
    #[must_use]
    pub fn from_fn_1<A, T, F>(arg: CellId, f: F) -> Self
    where
        A: Any + 'static,
        T: Any + 'static,
        F: Fn(&T, &A) -> Result<T, anyhow::Error> + 'static,
    {
        Filter::new(
            TypeId::of::<T>(),
            vec![arg],
            vec![TypeId::of::<A>()],
            move |value, args| {
                let value = value
                    .downcast_ref::<T>()
                    .expect("type checked at add_filter");
                let a = args[0]
                    .downcast_ref::<A>()
                    .expect("type checked at add_filter");
                Ok(Box::new(f(value, a)?) as Box<dyn Any>)
            },
        )
    }

    /// Creates a filter with two dynamic argument cells from a typed closure.
    ///
    /// `args[0]` maps to `A` and `args[1]` maps to `B`. `TypeId`s for `A`, `B`, and `T`
    /// are captured automatically. The filter is validated when passed to
    /// [`crate::sheet::Sheet::add_filter`].
    ///
    /// - Precondition: `f` satisfies [`Filter`]'s purity and idempotence contract.
    ///
    /// # Examples
    /// See the [filter example](crate::filter).
    #[must_use]
    pub fn from_fn_2<A, B, T, F>(args: [CellId; 2], f: F) -> Self
    where
        A: Any + 'static,
        B: Any + 'static,
        T: Any + 'static,
        F: Fn(&T, &A, &B) -> Result<T, anyhow::Error> + 'static,
    {
        Filter::new(
            TypeId::of::<T>(),
            args.to_vec(),
            vec![TypeId::of::<A>(), TypeId::of::<B>()],
            move |value, cell_args| {
                let value = value
                    .downcast_ref::<T>()
                    .expect("type checked at add_filter");
                let a = cell_args[0]
                    .downcast_ref::<A>()
                    .expect("type checked at add_filter");
                let b = cell_args[1]
                    .downcast_ref::<B>()
                    .expect("type checked at add_filter");
                Ok(Box::new(f(value, a, b)?) as Box<dyn Any>)
            },
        )
    }

    /// Creates a range-clamp filter from an explicit value `TypeId`, argument `TypeId`s, a clamp
    /// function, and a `bounds` re-evaluator — the tagged counterpart of what [`Filter::new`]
    /// builds for [`FilterKind::Opaque`]. `clamp` is `Filter`'s actual per-write/per-propagate
    /// function (called exactly like an opaque filter's); `bounds` is called independently, with
    /// no candidate value, by [`crate::sheet::Sheet::filter_range`].
    ///
    /// - Precondition: `args.len() == arg_types.len()`.
    /// - Precondition: `clamp` returns a value whose runtime type matches `value_type`.
    /// - Precondition: when `bounds` returns `Some`, the pair's values have a runtime type
    ///   matching `value_type`.
    /// - Precondition: `clamp` and `bounds` satisfy [`Filter`]'s purity contract;
    ///   `clamp` is also idempotent.
    ///
    /// # Examples
    /// See the live range filter in the [graph example](crate::sheet).
    #[must_use]
    pub fn range<F, B>(
        value_type: TypeId,
        args: Vec<CellId>,
        arg_types: Vec<TypeId>,
        clamp: F,
        bounds: B,
    ) -> Self
    where
        F: Fn(&dyn Any, &[&dyn Any]) -> Result<Box<dyn Any>, anyhow::Error> + 'static,
        B: Fn(&[&dyn Any]) -> Option<(Box<dyn Any>, Box<dyn Any>)> + 'static,
    {
        debug_assert_eq!(args.len(), arg_types.len());
        Filter(FilterData {
            value_type,
            args,
            arg_types,
            function: Box::new(clamp),
            kind: FilterKind::Range {
                bounds: Box::new(bounds),
            },
        })
    }
}

/// The outcome of re-checking a filter against a value a relationship's method
/// derived, rather than a value written externally.
///
/// See [`crate::sheet::Sheet::filter_violation`].
///
/// # Examples
/// ```
/// use adam_rs::{Filter, FilterViolation, Method, Sheet};
/// let mut sheet = Sheet::new();
/// let input = sheet.add_source(20_i32);
/// let output = sheet.add_cell(0_i32);
/// sheet.add_filter(output, Filter::from_fn_0(|v: &i32| Ok((*v).min(10))))?;
/// sheet.add_relationship(vec![Method::from_fn_1_1(input, output, |v: &i32| Ok(*v))])?;
/// sheet.propagate()?;
/// assert!(matches!(sheet.filter_violation(output), Some(FilterViolation::NotConformed)));
/// assert_eq!(*sheet.read::<i32>(output)?, 20);
/// # Ok::<(), adam_rs::Error>(())
/// ```
#[derive(Debug)]
pub enum FilterViolation {
    /// The filter succeeded but its output differs from the cell's current value. Only
    /// ever recorded for a *derived* cell (`Sheet::propagate`'s post-execution
    /// diagnostic phase) — a filtered source cell's filter output is unconditionally
    /// authoritative once computed (see `PlanStep::FilterReclamp`), so this variant
    /// never applies there.
    NotConformed,
    /// The filter's function itself returned an error, or returned a value of a
    /// different type than the cell — both treated as an equally soft diagnostic (see
    /// the design spec §4 for why a filter's own `Err` is not a propagation-aborting
    /// failure the way a `Requirement`'s is). Can occur on either side: a derived
    /// cell's read-only diagnostic check, or a source cell's live reclamp, in which
    /// case the cell's stored value is left unchanged.
    Failed(anyhow::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;
    use std::any::TypeId;

    use crate::cell::CellId;

    #[test]
    /// Checks a static conformer's declared type, result and idempotence.
    fn from_fn_0_stores_correct_value_type_and_computes_value() {
        let filter = Filter::from_fn_0(|x: &i32| Ok((*x).max(10)));
        assert_eq!(filter.0.value_type, TypeId::of::<i32>());
        assert!(filter.0.args.is_empty());
        let x: i32 = 5;
        let result = (filter.0.function)(&x, &[]).unwrap();
        assert_eq!(*result.downcast_ref::<i32>().unwrap(), 10);
        let repeated = (filter.0.function)(result.as_ref(), &[]).unwrap();
        assert_eq!(repeated.downcast_ref::<i32>(), result.downcast_ref::<i32>());
    }

    #[test]
    /// Checks unary dynamic-filter types and its conformed value.
    fn from_fn_1_stores_correct_type_ids_and_computes_value() {
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let arg = map.insert(());

        let filter = Filter::from_fn_1(arg, |x: &i32, bound: &i32| Ok((*x).min(*bound)));
        assert_eq!(filter.0.value_type, TypeId::of::<i32>());
        assert_eq!(filter.0.args, vec![arg]);
        assert_eq!(filter.0.arg_types, vec![TypeId::of::<i32>()]);

        let x: i32 = 50;
        let bound: i32 = 10;
        let result = (filter.0.function)(&x, &[&bound]).unwrap();
        assert_eq!(*result.downcast_ref::<i32>().unwrap(), 10);
    }

    #[test]
    /// Checks binary dynamic-filter types and its conformed value.
    fn from_fn_2_stores_correct_type_ids_and_computes_value() {
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let lo = map.insert(());
        let hi = map.insert(());

        let filter = Filter::from_fn_2([lo, hi], |x: &i32, lo: &i32, hi: &i32| {
            Ok((*x).clamp(*lo, *hi))
        });
        assert_eq!(filter.0.value_type, TypeId::of::<i32>());
        assert_eq!(filter.0.args, vec![lo, hi]);
        assert_eq!(
            filter.0.arg_types,
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()]
        );

        let x: i32 = 500;
        let lo_v: i32 = 0;
        let hi_v: i32 = 100;
        let result = (filter.0.function)(&x, &[&lo_v, &hi_v]).unwrap();
        assert_eq!(*result.downcast_ref::<i32>().unwrap(), 100);
    }

    #[test]
    /// Checks preservation of a static conformer's callback failure.
    fn from_fn_0_reports_the_error_a_failing_function_returns() {
        let filter =
            Filter::from_fn_0(|_x: &i32| Err(std::io::Error::other("cannot conform").into()));
        let x: i32 = 1;
        let err = (filter.0.function)(&x, &[]).unwrap_err();
        assert_eq!(err.to_string(), "cannot conform");
        assert!(err.downcast_ref::<std::io::Error>().is_some());
    }

    #[test]
    /// Checks preservation of an erased filter's declared value and argument types.
    fn new_stores_explicit_value_type_and_arg_types() {
        let filter = Filter::new(TypeId::of::<i32>(), vec![], vec![], |value, _args| {
            let v = value.downcast_ref::<i32>().unwrap();
            Ok(Box::new(*v) as Box<dyn std::any::Any>)
        });
        assert_eq!(filter.0.value_type, TypeId::of::<i32>());
    }

    /// Preserves heterogeneous argument order and typed failures in dynamic adapters.
    #[test]
    fn typed_dynamic_adapters_preserve_order_and_errors() {
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());
        let b = map.insert(());
        let filter = Filter::from_fn_2([a, b], |v: &String, prefix: &String, suffix: &char| {
            if v.starts_with(prefix) && v.ends_with(*suffix) {
                Ok(v.clone())
            } else {
                Ok(format!("{prefix}{v}{suffix}"))
            }
        });
        let result =
            (filter.0.function)(&"value".to_string(), &[&"pre:".to_string(), &'!']).unwrap();
        assert_eq!(result.downcast_ref::<String>().unwrap(), "pre:value!");
        let repeated = (filter.0.function)(result.as_ref(), &[&"pre:".to_string(), &'!']).unwrap();
        assert_eq!(
            repeated.downcast_ref::<String>(),
            result.downcast_ref::<String>()
        );
        let one = Filter::from_fn_1(a, |_: &i32, _: &bool| {
            Err(std::io::Error::other("one").into())
        });
        let two = Filter::from_fn_2([a, b], |_: &i32, _: &bool, _: &char| {
            Err(std::io::Error::other("two").into())
        });
        assert!(
            (one.0.function)(&1_i32, &[&true])
                .unwrap_err()
                .downcast_ref::<std::io::Error>()
                .is_some()
        );
        assert!(
            (two.0.function)(&1_i32, &[&true, &'!'])
                .unwrap_err()
                .downcast_ref::<std::io::Error>()
                .is_some()
        );
    }

    #[test]
    /// Checks that an erased filter carries opaque presentation metadata.
    fn new_defaults_to_opaque_kind() {
        let filter = Filter::new(TypeId::of::<i32>(), vec![], vec![], |value, _args| {
            Ok(Box::new(*value.downcast_ref::<i32>().unwrap()) as Box<dyn Any>)
        });
        assert!(matches!(filter.0.kind, FilterKind::Opaque));
    }

    #[test]
    /// Checks range metadata and callback clamping.
    fn range_stores_range_kind_and_clamps_via_function() {
        let filter = Filter::range(
            TypeId::of::<i32>(),
            vec![],
            vec![],
            |value: &dyn Any, _args: &[&dyn Any]| {
                let v = *value.downcast_ref::<i32>().unwrap();
                Ok(Box::new(v.clamp(0, 100)) as Box<dyn Any>)
            },
            |_args: &[&dyn Any]| {
                Some((
                    Box::new(0i32) as Box<dyn Any>,
                    Box::new(100i32) as Box<dyn Any>,
                ))
            },
        );
        assert_eq!(filter.0.value_type, TypeId::of::<i32>());
        let x: i32 = 500;
        let result = (filter.0.function)(&x, &[]).unwrap();
        assert_eq!(*result.downcast_ref::<i32>().unwrap(), 100);
        let FilterKind::Range { bounds } = &filter.0.kind else {
            panic!("expected FilterKind::Range");
        };
        let (lo, hi) = bounds(&[]).unwrap();
        assert_eq!(*lo.downcast_ref::<i32>().unwrap(), 0);
        assert_eq!(*hi.downcast_ref::<i32>().unwrap(), 100);
    }
}
