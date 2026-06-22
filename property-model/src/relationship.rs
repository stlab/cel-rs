//! Relationships and methods in the property model bipartite graph.
//!
//! Each relationship holds a list of [`Method`]s; the planner selects one
//! method per relationship at propagation time based on cell strength.

use std::any::{Any, TypeId};

use slotmap::new_key_type;

use crate::cell::CellId;

new_key_type! {
    /// A stable handle to a relationship in a [`crate::sheet::Sheet`].
    pub struct RelationshipId;
}

/// Type-erased function stored inside a [`Method`].
///
/// Takes a slice of type-erased input references and returns a `Vec` of
/// type-erased boxed outputs, or an error.
type MethodFn = Box<dyn Fn(&[&dyn Any]) -> Result<Vec<Box<dyn Any>>, anyhow::Error>>;

/// A single method within a relationship.
#[allow(dead_code)]
pub struct Method {
    pub(crate) inputs: Vec<CellId>,
    pub(crate) outputs: Vec<CellId>,
    pub(crate) input_types: Vec<TypeId>,
    pub(crate) output_types: Vec<TypeId>,
    pub(crate) function: MethodFn,
}

impl Method {
    /// Creates a method from explicit TypeIds and a type-erased function.
    ///
    /// - Precondition: `inputs.len() == input_types.len()` and `outputs.len() == output_types.len()`.
    /// - Precondition: The function must return exactly `outputs.len()` values in the correct order.
    pub fn new<F>(
        inputs: Vec<CellId>,
        outputs: Vec<CellId>,
        input_types: Vec<TypeId>,
        output_types: Vec<TypeId>,
        f: F,
    ) -> Self
    where
        F: Fn(&[&dyn Any]) -> Result<Vec<Box<dyn Any>>, anyhow::Error> + 'static,
    {
        debug_assert_eq!(inputs.len(), input_types.len());
        debug_assert_eq!(outputs.len(), output_types.len());
        Method {
            inputs,
            outputs,
            input_types,
            output_types,
            function: Box::new(f),
        }
    }

    /// Creates a 1-input, 1-output method from a typed closure.
    ///
    /// TypeIds for `A` and `B` are captured automatically. The method is validated
    /// against its cell registrations when passed to [`crate::sheet::Sheet::add_relationship`].
    pub fn from_fn_1_1<A, B, F>(input: CellId, output: CellId, f: F) -> Self
    where
        A: Any + 'static,
        B: Any + 'static,
        F: Fn(&A) -> Result<B, anyhow::Error> + 'static,
    {
        Method {
            inputs: vec![input],
            outputs: vec![output],
            input_types: vec![TypeId::of::<A>()],
            output_types: vec![TypeId::of::<B>()],
            function: Box::new(move |args| {
                let a = args[0]
                    .downcast_ref::<A>()
                    .expect("type checked at add_relationship");
                Ok(vec![Box::new(f(a)?)])
            }),
        }
    }

    /// Creates a 2-input, 1-output method from a typed closure.
    ///
    /// `inputs[0]` maps to `A` and `inputs[1]` maps to `B`. TypeIds are captured
    /// automatically. The method is validated when passed to
    /// [`crate::sheet::Sheet::add_relationship`].
    pub fn from_fn_2_1<A, B, C, F>(inputs: [CellId; 2], output: CellId, f: F) -> Self
    where
        A: Any + 'static,
        B: Any + 'static,
        C: Any + 'static,
        F: Fn(&A, &B) -> Result<C, anyhow::Error> + 'static,
    {
        Method {
            inputs: inputs.to_vec(),
            outputs: vec![output],
            input_types: vec![TypeId::of::<A>(), TypeId::of::<B>()],
            output_types: vec![TypeId::of::<C>()],
            function: Box::new(move |args| {
                let a = args[0]
                    .downcast_ref::<A>()
                    .expect("type checked at add_relationship");
                let b = args[1]
                    .downcast_ref::<B>()
                    .expect("type checked at add_relationship");
                Ok(vec![Box::new(f(a, b)?)])
            }),
        }
    }
}

/// Internal storage for a relationship; fields are used by `Sheet` (added in a later task).
#[allow(dead_code)]
pub(crate) struct RelationshipData {
    pub(crate) methods: Vec<Method>,
    /// Union of all cell IDs referenced by any method in this relationship.
    pub(crate) adj: Vec<CellId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relationship_id_is_copy() {
        fn takes_copy<T: Copy>(_: T) {}
        // RelationshipId must be Copy so it can be stored in adjacency Vecs cheaply.
        // This test fails to compile if RelationshipId is not Copy.
        takes_copy(RelationshipId::default());
    }

    #[test]
    fn method_new_stores_types_and_cell_ids() {
        use slotmap::SlotMap;

        // Create real CellIds from a SlotMap to avoid using the default (null) key.
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());
        let b = map.insert(());
        let c = map.insert(());

        let method = Method::new(
            vec![a, b],
            vec![c],
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
            vec![TypeId::of::<i32>()],
            |args| {
                let x = args[0].downcast_ref::<i32>().unwrap();
                let y = args[1].downcast_ref::<i32>().unwrap();
                Ok(vec![Box::new(x + y)])
            },
        );

        assert_eq!(method.inputs, vec![a, b]);
        assert_eq!(method.outputs, vec![c]);
        assert_eq!(
            method.input_types,
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()]
        );
        assert_eq!(method.output_types, vec![TypeId::of::<i32>()]);

        // Verify the stored function works.
        let x: i32 = 3;
        let y: i32 = 4;
        let result = (method.function)(&[&x, &y]).unwrap();
        assert_eq!(*result[0].downcast_ref::<i32>().unwrap(), 7);
    }

    #[test]
    fn from_fn_1_1_stores_correct_type_ids() {
        use slotmap::SlotMap;

        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());
        let b = map.insert(());

        let method = Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2));

        assert_eq!(method.inputs, vec![a]);
        assert_eq!(method.outputs, vec![b]);
        assert_eq!(method.input_types, vec![TypeId::of::<i32>()]);
        assert_eq!(method.output_types, vec![TypeId::of::<i32>()]);

        let x: i32 = 5;
        let result = (method.function)(&[&x]).unwrap();
        assert_eq!(*result[0].downcast_ref::<i32>().unwrap(), 10);
    }

    #[test]
    fn from_fn_2_1_stores_correct_type_ids() {
        use slotmap::SlotMap;

        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());
        let b = map.insert(());
        let c = map.insert(());

        let method = Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok((*x) * (*y)));

        assert_eq!(method.inputs, vec![a, b]);
        assert_eq!(method.outputs, vec![c]);
        assert_eq!(
            method.input_types,
            vec![TypeId::of::<f64>(), TypeId::of::<f64>()]
        );
        assert_eq!(method.output_types, vec![TypeId::of::<f64>()]);

        let x: f64 = 2.0;
        let y: f64 = 3.0;
        let result = (method.function)(&[&x, &y]).unwrap();
        assert_eq!(*result[0].downcast_ref::<f64>().unwrap(), 6.0);
    }
}
