//! Input filters: idempotent, per-cell domain constraints.
//!
//! A [`Filter`] conforms or rejects a value written externally to its cell (see
//! [`crate::sheet::Sheet::write`]), and is re-evaluated as a non-gating diagnostic
//! against a value a relationship's method derives for that cell (see
//! [`crate::sheet::Sheet::propagate`]). See [`crate::sheet::Sheet::add_filter`].

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
pub enum FilterKind {
    /// The filter's expression wasn't a recognized structural form.
    Opaque,
    /// Compiled from a `RangeInclusive<T>`-typed expression (`lo..=hi`). `bounds` re-evaluates
    /// that expression against the filter's current argument values, returning the resulting
    /// `(lo, hi)` as type-erased values of the filtered cell's own type `T`.
    Range {
        /// Re-evaluates the range expression, returning `(lo, hi)` as type-erased values.
        #[allow(clippy::type_complexity)]
        bounds: Box<dyn Fn(&[&dyn Any]) -> (Box<dyn Any>, Box<dyn Any>)>,
    },
}

/// An idempotent, per-cell domain constraint with optional dynamic arguments.
///
/// Constructed via [`Filter::from_fn_0`]/[`Filter::from_fn_1`]/[`Filter::from_fn_2`] for
/// the common typed cases, or [`Filter::new`] for the fully type-erased form. Attached
/// to a cell with [`crate::sheet::Sheet::add_filter`].
pub struct Filter(pub(crate) FilterData);

/// Internal storage for a single filter.
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
    /// - Precondition: `f` returns a value whose runtime type matches `value_type`.
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
    /// - Precondition: `bounds` returns a pair of values whose runtime type matches `value_type`.
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
        B: Fn(&[&dyn Any]) -> (Box<dyn Any>, Box<dyn Any>) + 'static,
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
#[derive(Debug)]
pub enum FilterViolation {
    /// The filter succeeded but its output differs from the cell's current value.
    NotConformed,
    /// The filter's function itself returned an error, or returned a value of a
    /// different type than the cell — both treated as an equally soft diagnostic (see
    /// the design spec §4 for why a filter's own `Err` is not a propagation-aborting
    /// failure the way a `Requirement`'s is).
    Failed(anyhow::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;
    use std::any::TypeId;

    use crate::cell::CellId;

    #[test]
    fn from_fn_0_stores_correct_value_type_and_computes_value() {
        let filter = Filter::from_fn_0(|x: &i32| Ok(*x * 2));
        assert_eq!(filter.0.value_type, TypeId::of::<i32>());
        assert!(filter.0.args.is_empty());
        let x: i32 = 5;
        let result = (filter.0.function)(&x, &[]).unwrap();
        assert_eq!(*result.downcast_ref::<i32>().unwrap(), 10);
    }

    #[test]
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
    fn from_fn_0_reports_the_error_a_failing_function_returns() {
        let filter = Filter::from_fn_0(|_x: &i32| Err(anyhow::anyhow!("cannot conform")));
        let x: i32 = 1;
        let err = (filter.0.function)(&x, &[]).unwrap_err();
        assert_eq!(err.to_string(), "cannot conform");
    }

    #[test]
    fn new_stores_explicit_value_type_and_arg_types() {
        let filter = Filter::new(TypeId::of::<i32>(), vec![], vec![], |value, _args| {
            let v = value.downcast_ref::<i32>().unwrap();
            Ok(Box::new(*v) as Box<dyn std::any::Any>)
        });
        assert_eq!(filter.0.value_type, TypeId::of::<i32>());
    }

    #[test]
    fn new_defaults_to_opaque_kind() {
        let filter = Filter::new(TypeId::of::<i32>(), vec![], vec![], |value, _args| {
            Ok(Box::new(*value.downcast_ref::<i32>().unwrap()) as Box<dyn Any>)
        });
        assert!(matches!(filter.0.kind, FilterKind::Opaque));
    }

    #[test]
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
                (
                    Box::new(0i32) as Box<dyn Any>,
                    Box::new(100i32) as Box<dyn Any>,
                )
            },
        );
        assert_eq!(filter.0.value_type, TypeId::of::<i32>());
        let x: i32 = 500;
        let result = (filter.0.function)(&x, &[]).unwrap();
        assert_eq!(*result.downcast_ref::<i32>().unwrap(), 100);
        let FilterKind::Range { bounds } = &filter.0.kind else {
            panic!("expected FilterKind::Range");
        };
        let (lo, hi) = bounds(&[]);
        assert_eq!(*lo.downcast_ref::<i32>().unwrap(), 0);
        assert_eq!(*hi.downcast_ref::<i32>().unwrap(), 100);
    }
}
