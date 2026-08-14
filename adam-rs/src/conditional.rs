//! Conditionals in the property model: match-subject branching.
//!
//! Each conditional evaluates a match subject — either a single existing cell, read
//! directly, or a [`MatchExpr`] computed from multiple input cells — and holds a list of
//! branches. During propagation the branch whose keys contain the current match value is
//! activated; its relationships participate in the general planning pass.

use std::any::{Any, TypeId};

use slotmap::new_key_type;

use crate::{cell::CellId, relationship::RelationshipId};

new_key_type! {
    /// A stable handle to a conditional in a [`crate::sheet::Sheet`].
    pub struct ConditionalId;
}

/// Type-erased function stored inside a [`MatchExprData`].
///
/// Takes a slice of type-erased input references and returns a type-erased boxed
/// result value, or an error.
type MatchExprFn = Box<dyn Fn(&[&dyn Any]) -> Result<Box<dyn Any>, anyhow::Error>>;

/// A conditional's match subject: an existing cell, or a method-like expression computed
/// from a set of input cells.
///
/// Constructed via [`MatchExpr::cell`] for the common single-cell case, or
/// [`MatchExpr::new`]/[`MatchExpr::from_fn_1`]/[`MatchExpr::from_fn_2`] to compute the
/// match value from multiple cells (analogous to [`crate::relationship::Method`]).
pub struct MatchExpr(pub(crate) MatchSource);

pub(crate) enum MatchSource {
    /// The match value is `cell`'s current effective value, read directly with no
    /// allocation and no extra trait bounds.
    Cell(CellId),
    /// The match value is computed from `MatchExprData`.
    Expr(MatchExprData),
}

pub(crate) struct MatchExprData {
    pub(crate) inputs: Vec<CellId>,
    pub(crate) input_types: Vec<TypeId>,
    pub(crate) output_type: TypeId,
    pub(crate) eq_fn: fn(&dyn Any, &dyn Any) -> bool,
    pub(crate) function: MatchExprFn,
}

impl MatchExpr {
    /// Wraps a single existing cell as the match subject.
    ///
    /// - Postcondition: behaves exactly as a plain-cell conditional does today — the match
    ///   value is `cell`'s current effective value, with no extra allocation or trait
    ///   bounds beyond what [`crate::sheet::Sheet::add_conditional`] itself requires.
    #[must_use]
    pub fn cell(cell: CellId) -> Self {
        MatchExpr(MatchSource::Cell(cell))
    }

    /// Creates a match expression from explicit `TypeId`s and a type-erased function.
    ///
    /// - Precondition: `inputs.len() == input_types.len()`.
    /// - Precondition: `f` returns a value whose runtime type matches `output_type`.
    /// - Precondition: `eq_fn` correctly compares two values of the type identified by
    ///   `output_type`.
    #[must_use]
    pub fn new<F>(
        inputs: Vec<CellId>,
        input_types: Vec<TypeId>,
        output_type: TypeId,
        eq_fn: fn(&dyn Any, &dyn Any) -> bool,
        f: F,
    ) -> Self
    where
        F: Fn(&[&dyn Any]) -> Result<Box<dyn Any>, anyhow::Error> + 'static,
    {
        debug_assert_eq!(inputs.len(), input_types.len());
        MatchExpr(MatchSource::Expr(MatchExprData {
            inputs,
            input_types,
            output_type,
            eq_fn,
            function: Box::new(f),
        }))
    }

    /// Creates a 1-input match expression from a typed closure.
    ///
    /// `TypeId`s for `A` and `T` are captured automatically, along with `T`'s equality
    /// function. The expression is validated against its cell registration when passed to
    /// [`crate::sheet::Sheet::add_conditional`].
    #[must_use]
    pub fn from_fn_1<A, T, F>(input: CellId, f: F) -> Self
    where
        A: Any + 'static,
        T: Any + PartialEq + 'static,
        F: Fn(&A) -> Result<T, anyhow::Error> + 'static,
    {
        MatchExpr::new(
            vec![input],
            vec![TypeId::of::<A>()],
            TypeId::of::<T>(),
            |a, b| a.downcast_ref::<T>() == b.downcast_ref::<T>(),
            move |args| {
                let a = args[0]
                    .downcast_ref::<A>()
                    .expect("type checked at add_conditional");
                Ok(Box::new(f(a)?) as Box<dyn Any>)
            },
        )
    }

    /// Creates a 2-input match expression from a typed closure.
    ///
    /// `inputs[0]` maps to `A` and `inputs[1]` maps to `B`. `TypeId`s for `A`, `B`, and `T`
    /// are captured automatically, along with `T`'s equality function. The expression is
    /// validated when passed to [`crate::sheet::Sheet::add_conditional`].
    #[must_use]
    pub fn from_fn_2<A, B, T, F>(inputs: [CellId; 2], f: F) -> Self
    where
        A: Any + 'static,
        B: Any + 'static,
        T: Any + PartialEq + 'static,
        F: Fn(&A, &B) -> Result<T, anyhow::Error> + 'static,
    {
        MatchExpr::new(
            inputs.to_vec(),
            vec![TypeId::of::<A>(), TypeId::of::<B>()],
            TypeId::of::<T>(),
            |a, b| a.downcast_ref::<T>() == b.downcast_ref::<T>(),
            move |args| {
                let a = args[0]
                    .downcast_ref::<A>()
                    .expect("type checked at add_conditional");
                let b = args[1]
                    .downcast_ref::<B>()
                    .expect("type checked at add_conditional");
                Ok(Box::new(f(a, b)?) as Box<dyn Any>)
            },
        )
    }
}

/// One arm of a [`ConditionalData`]: a set of key values and the relationships
/// to activate when the match value equals any key.
pub(crate) struct Branch {
    /// Type-erased key values; each `TypeId` matches the match subject's output type.
    pub(crate) keys: Vec<Box<dyn Any>>,
    /// Relationships activated when any key matches.
    pub(crate) relationships: Vec<RelationshipId>,
}

/// Internal storage for a conditional.
pub(crate) struct ConditionalData {
    /// The match subject whose value is tested.
    pub(crate) source: MatchSource,
    /// Branches evaluated in definition order; first match wins.
    pub(crate) branches: Vec<Branch>,
    /// Relationships activated when no branch matches. Empty means no default.
    pub(crate) default: Vec<RelationshipId>,
}

impl ConditionalData {
    /// Returns the cells that determine this conditional's match value: a single cell for
    /// [`MatchSource::Cell`], or every input of the expression for [`MatchSource::Expr`].
    pub(crate) fn match_cells(&self) -> &[CellId] {
        match &self.source {
            MatchSource::Cell(id) => std::slice::from_ref(id),
            MatchSource::Expr(expr) => &expr.inputs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditional_id_is_copy() {
        fn takes_copy<T: Copy>(_: T) {}
        takes_copy(ConditionalId::default());
    }

    #[test]
    fn match_expr_cell_wraps_a_single_cell() {
        use slotmap::SlotMap;
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let cell = map.insert(());
        let expr = MatchExpr::cell(cell);
        match expr.0 {
            MatchSource::Cell(id) => assert_eq!(id, cell),
            MatchSource::Expr(_) => panic!("expected Cell variant"),
        }
    }

    #[test]
    fn match_expr_from_fn_1_stores_correct_type_ids_and_computes_value() {
        use slotmap::SlotMap;
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());

        let expr = MatchExpr::from_fn_1(a, |x: &i32| Ok(*x * 2));
        match expr.0 {
            MatchSource::Expr(data) => {
                assert_eq!(data.inputs, vec![a]);
                assert_eq!(data.input_types, vec![TypeId::of::<i32>()]);
                assert_eq!(data.output_type, TypeId::of::<i32>());
                let x: i32 = 5;
                let result = (data.function)(&[&x]).unwrap();
                assert_eq!(*result.downcast_ref::<i32>().unwrap(), 10);
                let y: i32 = 10;
                assert!((data.eq_fn)(&y, &10_i32));
                assert!(!(data.eq_fn)(&y, &11_i32));
            }
            MatchSource::Cell(_) => panic!("expected Expr variant"),
        }
    }

    #[test]
    fn match_expr_from_fn_2_stores_correct_type_ids_and_computes_value() {
        use slotmap::SlotMap;
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());
        let b = map.insert(());

        let expr = MatchExpr::from_fn_2([a, b], |x: &bool, y: &bool| Ok(*x && *y));
        match expr.0 {
            MatchSource::Expr(data) => {
                assert_eq!(data.inputs, vec![a, b]);
                assert_eq!(
                    data.input_types,
                    vec![TypeId::of::<bool>(), TypeId::of::<bool>()]
                );
                assert_eq!(data.output_type, TypeId::of::<bool>());
                let x = true;
                let y = false;
                let result = (data.function)(&[&x, &y]).unwrap();
                assert!(!*result.downcast_ref::<bool>().unwrap());
            }
            MatchSource::Cell(_) => panic!("expected Expr variant"),
        }
    }

    #[test]
    fn match_cells_returns_single_cell_for_cell_variant() {
        use slotmap::SlotMap;
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let cell = map.insert(());
        let data = ConditionalData {
            source: MatchSource::Cell(cell),
            branches: Vec::new(),
            default: Vec::new(),
        };
        assert_eq!(data.match_cells(), &[cell]);
    }

    #[test]
    fn match_cells_returns_all_inputs_for_expr_variant() {
        use slotmap::SlotMap;
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());
        let b = map.insert(());
        let expr = MatchExpr::from_fn_2([a, b], |x: &i32, y: &i32| Ok(x + y));
        let MatchSource::Expr(data) = expr.0 else {
            panic!("expected Expr variant")
        };
        let cond = ConditionalData {
            source: MatchSource::Expr(data),
            branches: Vec::new(),
            default: Vec::new(),
        };
        assert_eq!(cond.match_cells(), &[a, b]);
    }

    #[test]
    fn match_expr_new_reports_the_error_a_failing_function_returns() {
        use slotmap::SlotMap;
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());

        let expr = MatchExpr::new(
            vec![a],
            vec![TypeId::of::<i32>()],
            TypeId::of::<i32>(),
            |x, y| x.downcast_ref::<i32>() == y.downcast_ref::<i32>(),
            |_args| Err(anyhow::anyhow!("boom")),
        );
        let MatchSource::Expr(data) = expr.0 else {
            panic!("expected Expr variant")
        };
        let x: i32 = 1;
        let err = (data.function)(&[&x]).unwrap_err();
        assert_eq!(err.to_string(), "boom");
    }
}
