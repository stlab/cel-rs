//! Named boolean checks attached to outputs.
//!
//! Each [`Requirement`] is a pure predicate over some set of cells, evaluated after every
//! `Sheet::propagate` to determine whether an output's preconditions currently hold. A
//! requirement's inputs may be any cells in the sheet, not only the inputs of the output's
//! writer method. See [`crate::sheet::Sheet::add_output`].

use std::any::{Any, TypeId};

use slotmap::new_key_type;

use crate::cell::CellId;
use crate::output::OutputId;

new_key_type! {
    /// A stable handle to a requirement in a [`crate::sheet::Sheet`].
    pub struct RequirementId;
}

/// Type-erased predicate stored inside a [`Requirement`].
type RequirementFn = Box<dyn Fn(&[&dyn Any]) -> Result<bool, anyhow::Error>>;

/// A single named boolean check over some set of cells, attached to an output.
pub struct Requirement {
    pub(crate) inputs: Vec<CellId>,
    pub(crate) input_types: Vec<TypeId>,
    pub(crate) function: RequirementFn,
}

impl Requirement {
    /// Creates a requirement from explicit TypeIds and a type-erased predicate.
    ///
    /// - Precondition: `inputs.len() == input_types.len()`.
    pub fn new<F>(inputs: Vec<CellId>, input_types: Vec<TypeId>, f: F) -> Self
    where
        F: Fn(&[&dyn Any]) -> Result<bool, anyhow::Error> + 'static,
    {
        debug_assert_eq!(inputs.len(), input_types.len());
        Requirement {
            inputs,
            input_types,
            function: Box::new(f),
        }
    }

    /// Creates a 1-input requirement from a typed closure.
    ///
    /// The TypeId for `A` is captured automatically. The requirement is validated against
    /// its cell registration when passed to [`crate::sheet::Sheet::add_output`].
    pub fn from_fn_1<A, F>(input: CellId, f: F) -> Self
    where
        A: Any + 'static,
        F: Fn(&A) -> Result<bool, anyhow::Error> + 'static,
    {
        Requirement {
            inputs: vec![input],
            input_types: vec![TypeId::of::<A>()],
            function: Box::new(move |args| {
                let a = args[0]
                    .downcast_ref::<A>()
                    .expect("type checked at add_output");
                f(a)
            }),
        }
    }

    /// Creates a 2-input requirement from a typed closure.
    ///
    /// `inputs[0]` maps to `A` and `inputs[1]` maps to `B`. TypeIds are captured
    /// automatically. The requirement is validated when passed to
    /// [`crate::sheet::Sheet::add_output`].
    pub fn from_fn_2<A, B, F>(inputs: [CellId; 2], f: F) -> Self
    where
        A: Any + 'static,
        B: Any + 'static,
        F: Fn(&A, &B) -> Result<bool, anyhow::Error> + 'static,
    {
        Requirement {
            inputs: inputs.to_vec(),
            input_types: vec![TypeId::of::<A>(), TypeId::of::<B>()],
            function: Box::new(move |args| {
                let a = args[0]
                    .downcast_ref::<A>()
                    .expect("type checked at add_output");
                let b = args[1]
                    .downcast_ref::<B>()
                    .expect("type checked at add_output");
                f(a, b)
            }),
        }
    }
}

/// Internal storage for a single requirement.
pub(crate) struct RequirementData {
    pub(crate) name: String,
    pub(crate) output: OutputId,
    pub(crate) inputs: Vec<CellId>,
    pub(crate) function: RequirementFn,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirement_id_is_copy() {
        fn takes_copy<T: Copy>(_: T) {}
        takes_copy(RequirementId::default());
    }

    #[test]
    fn requirement_new_stores_types_and_cell_ids() {
        use slotmap::SlotMap;

        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());
        let b = map.insert(());

        let requirement = Requirement::new(
            vec![a, b],
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
            |args| {
                let x = args[0].downcast_ref::<i32>().unwrap();
                let y = args[1].downcast_ref::<i32>().unwrap();
                Ok(x + y <= 10)
            },
        );

        assert_eq!(requirement.inputs, vec![a, b]);
        assert_eq!(
            requirement.input_types,
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()]
        );

        let x: i32 = 3;
        let y: i32 = 4;
        assert!((requirement.function)(&[&x, &y]).unwrap());
        let x: i32 = 8;
        let y: i32 = 8;
        assert!(!(requirement.function)(&[&x, &y]).unwrap());
    }

    #[test]
    fn from_fn_1_stores_correct_type_ids() {
        use slotmap::SlotMap;

        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());

        let requirement = Requirement::from_fn_1(a, |x: &i32| Ok(*x <= 5));

        assert_eq!(requirement.inputs, vec![a]);
        assert_eq!(requirement.input_types, vec![TypeId::of::<i32>()]);

        let x: i32 = 3;
        assert!((requirement.function)(&[&x]).unwrap());
        let x: i32 = 9;
        assert!(!(requirement.function)(&[&x]).unwrap());
    }

    #[test]
    fn from_fn_2_stores_correct_type_ids() {
        use slotmap::SlotMap;

        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());
        let b = map.insert(());

        let requirement = Requirement::from_fn_2([a, b], |x: &i32, y: &i32| Ok(x * y <= 20));

        assert_eq!(requirement.inputs, vec![a, b]);
        assert_eq!(
            requirement.input_types,
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()]
        );

        let x: i32 = 4;
        let y: i32 = 5;
        assert!((requirement.function)(&[&x, &y]).unwrap());
        let x: i32 = 5;
        let y: i32 = 5;
        assert!(!(requirement.function)(&[&x, &y]).unwrap());
    }
}
