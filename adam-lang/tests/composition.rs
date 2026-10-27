//! Exercises source-free composition through its public contracts.

use std::any::TypeId;
use std::cell::Cell;
use std::rc::Rc;

use adam_lang::type_registry::TypeShape;
use adam_lang::{CompositionBuilder, ParsedSheet, TypeRegistry};
use cel_runtime::{Array, ArrayOf, Cons, DynSegment, IntoCStackList, Leaf, Nil};

/// Returns the registered integer shape.
fn int() -> TypeShape {
    TypeShape::Named(TypeId::of::<i32>())
}

/// Emits a constant integer without evaluating it during definition checking.
fn zero() -> anyhow::Result<DynSegment> {
    let mut body = DynSegment::new::<()>();
    body.op0(|| 0i32);
    Ok(body)
}

#[test]
fn source_free_instances_keep_private_shapes_and_fresh_initializers() {
    let types = TypeRegistry::new();
    let calls = Rc::new(Cell::new(0usize));
    let mut builder = CompositionBuilder::new(types.clone());
    let count = calls.clone();
    let private = builder
        .cell("hidden", int(), move || {
            let count = count.clone();
            let mut body = DynSegment::new::<()>();
            body.op0(move || {
                count.set(count.get() + 1);
                i32::try_from(count.get()).unwrap()
            });
            Ok(body)
        })
        .unwrap();
    let template = builder.finish().unwrap();
    assert_eq!(calls.get(), 0);
    let mut destination = ParsedSheet::new();
    let first = template.instantiate(&mut destination, &[]).unwrap();
    let second = template.instantiate(&mut destination, &[]).unwrap();
    let a = first
        .cell(first.occurrences()[0].id(), private.native())
        .unwrap();
    let b = second
        .cell(second.occurrences()[0].id(), private.native())
        .unwrap();
    assert_ne!(a, b);
    assert_eq!(*destination.read::<i32>(a).unwrap(), 1);
    assert_eq!(*destination.read::<i32>(b).unwrap(), 2);
    assert_eq!(destination.cell_shape(a), Some(&int()));
    assert_eq!(
        destination.cell_origins[&a][0].occurrence,
        *first.occurrences()[0].id()
    );
    assert_eq!(&destination.cell_origins[&a][0].slot, private.native());
    assert!(destination.cell_names.is_empty());
    assert_eq!(destination.instance_origins.len(), 2);
    let mut consumer = CompositionBuilder::new(types);
    consumer.parameter("private_actual", int()).unwrap();
    consumer
        .finish()
        .unwrap()
        .instantiate(&mut destination, &[("private_actual", a)])
        .unwrap();
    assert_eq!(destination.cells().count(), 2);
    assert_eq!(destination.cell_origins[&a].len(), 2);
    drop(template);
    drop(first);
    drop(second);
    assert_eq!(destination.cell_shape(b), Some(&int()));
}

#[test]
fn binding_rejections_leave_destination_unchanged() {
    let types = TypeRegistry::new();
    let mut builder = CompositionBuilder::new(types.clone());
    builder.parameter("a", int()).unwrap();
    builder.parameter("b", int()).unwrap();
    builder.cell("never", int(), zero).unwrap();
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let x = destination.add_cell(3i32);
    let y = destination.add_cell(false);
    destination.register_cell("x", x, int(), &types).unwrap();
    destination
        .register_cell("y", y, TypeShape::Named(TypeId::of::<bool>()), &types)
        .unwrap();
    for bindings in [
        vec![("a", x)],
        vec![("a", x), ("b", x)],
        vec![("a", x), ("b", y)],
        vec![("a", x), ("a", x)],
        vec![("unknown", x), ("b", y)],
    ] {
        let error = template
            .instantiate(&mut destination, &bindings)
            .unwrap_err();
        assert!(error.labels.iter().all(|label| label.location.is_none()));
        assert_eq!(destination.cells().count(), 2);
        assert!(destination.instance_origins.is_empty());
    }
}

#[test]
fn full_shapes_reject_same_carrier_arrays_and_tuples() {
    let types = TypeRegistry::new();
    let mut destination = ParsedSheet::new();
    let array = destination
        .add_compound_cell::<ArrayOf<Leaf<bool>>>("array", Array::from_vec(vec![true]), &types)
        .unwrap();
    let tuple = destination
        .add_compound_cell::<Cons<Leaf<bool>, Nil>>("tuple", (true,).into_c_stack_list(), &types)
        .unwrap();
    for (shape, actual) in [
        (TypeShape::Array(Box::new(int())), array),
        (TypeShape::Tuple(vec![int()]), tuple),
    ] {
        let mut builder = CompositionBuilder::new(types.clone());
        builder.parameter("x", shape).unwrap();
        let template = builder.finish().unwrap();
        assert!(
            template
                .instantiate(&mut destination, &[("x", actual)])
                .is_err()
        );
        assert_eq!(destination.cells().count(), 2);
    }
}

#[test]
fn nested_bindings_compare_callable_signatures_and_reject_foreign_slots() {
    let mut child = CompositionBuilder::new(TypeRegistry::new());
    child
        .parameter("f", TypeShape::Callable(vec![int()], Box::new(int())))
        .unwrap();
    let child = child.finish().unwrap();
    let mut parent = CompositionBuilder::new(TypeRegistry::new());
    let wrong = parent
        .parameter("f", TypeShape::Callable(vec![], Box::new(int())))
        .unwrap();
    assert!(parent.include(&child, &[("f", wrong)]).is_err());
    let mut other = CompositionBuilder::new(TypeRegistry::new());
    let foreign = other.parameter("foreign", int()).unwrap();
    assert!(parent.method(&[], &[foreign], zero).is_err());
}

#[test]
fn unregistered_shapes_are_errors_at_public_boundaries() {
    let unknown = TypeShape::Array(Box::new(TypeShape::Named(TypeId::of::<Cell<usize>>())));
    let mut builder = CompositionBuilder::new(TypeRegistry::new());
    assert!(builder.parameter("x", unknown.clone()).is_err());
    assert!(builder.cell("x", unknown.clone(), zero).is_err());
    let mut destination = ParsedSheet::new();
    let x = destination.add_cell(0i32);
    assert!(
        destination
            .register_cell("x", x, unknown, &TypeRegistry::new())
            .is_err()
    );
    assert!(destination.cell_names.is_empty());
}

/// Represents an arbitrary registered host leaf.
#[derive(Clone, Debug, Default, PartialEq)]
struct Host(i32);

#[test]
fn registered_host_leaf_methods_survive_template_owner_drop() {
    let mut types = TypeRegistry::new();
    types.register::<Host>("Host");
    let shape = TypeShape::Named(TypeId::of::<Host>());
    let mut builder = CompositionBuilder::new(types.clone());
    let input = builder.parameter("input", shape.clone()).unwrap();
    let output = builder
        .cell("private", shape, || {
            let mut body = DynSegment::new::<()>();
            body.op0(|| Host(0));
            Ok(body)
        })
        .unwrap();
    let method = builder
        .method(&[input], std::slice::from_ref(&output), || {
            let mut body = DynSegment::new::<()>();
            body.push_arg::<Host>(0);
            body.op1r(|x: Host| {
                x.0.checked_add(1)
                    .map(Host)
                    .ok_or_else(|| anyhow::anyhow!("overflow"))
            })?;
            Ok(body)
        })
        .unwrap();
    builder.relationship(vec![method]).unwrap();
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let x = destination.add_source(Host(4));
    destination
        .register_cell("x", x, TypeShape::Named(TypeId::of::<Host>()), &types)
        .unwrap();
    let instance = template
        .instantiate(&mut destination, &[("input", x)])
        .unwrap();
    drop(template);
    drop(types);
    destination.propagate().unwrap();
    let output = instance
        .cell(instance.occurrences()[0].id(), output.native())
        .unwrap();
    assert_eq!(destination.read::<Host>(output).unwrap(), &Host(5));
}

#[test]
fn all_private_body_operations_delegate_and_retain_nested_occurrences() {
    let mut child = CompositionBuilder::new(TypeRegistry::new());
    let input = child.parameter("input", int()).unwrap();
    let output = child.parameter("output", int()).unwrap();
    let method = child
        .method(&[input], &[output], || {
            let mut body = DynSegment::new::<()>();
            body.push_arg::<i32>(0);
            Ok(body)
        })
        .unwrap();
    child.relationship(vec![method]).unwrap();
    let child = child.finish().unwrap();

    let mut parent = CompositionBuilder::new(TypeRegistry::new());
    let source = parent
        .source("source", int(), || {
            let mut body = DynSegment::new::<()>();
            body.op0(|| 7i32);
            Ok(body)
        })
        .unwrap();
    let value = parent.cell("value", int(), zero).unwrap();
    parent
        .filter(&value, &[], || {
            let mut body = DynSegment::new::<()>();
            body.push_arg::<i32>(0);
            body.op1(|x: i32| x.max(0))?;
            Ok(body)
        })
        .unwrap();
    parent
        .require(&value, std::slice::from_ref(&value), || {
            let mut body = DynSegment::new::<()>();
            body.push_arg::<i32>(0);
            body.op1(|x: i32| x >= 0)?;
            Ok(body)
        })
        .unwrap();
    let included = parent
        .include(
            &child,
            &[("input", source.clone()), ("output", value.clone())],
        )
        .unwrap();
    parent
        .conditional(
            &[],
            || {
                let mut body = DynSegment::new::<()>();
                body.op0(|| true);
                Ok(body)
            },
            vec![(vec![true], included.relationships().to_vec())],
            vec![],
        )
        .unwrap();
    let out = parent.cell("out", int(), zero).unwrap();
    let writer = parent
        .method(&[value], std::slice::from_ref(&out), || {
            let mut body = DynSegment::new::<()>();
            body.push_arg::<i32>(0);
            Ok(body)
        })
        .unwrap();
    parent.out(writer).unwrap();
    let template = parent.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let instance = template.instantiate(&mut destination, &[]).unwrap();
    assert_eq!(instance.occurrences().len(), 2);
    let out_id = instance
        .cell(instance.occurrences()[0].id(), out.native())
        .unwrap();
    drop(child);
    drop(template);
    destination.propagate().unwrap();
    assert_eq!(*destination.read::<i32>(out_id).unwrap(), 7);
    assert!(destination.cell_names.is_empty());
    assert!(
        destination
            .method_origins
            .values()
            .all(|origin| instance.occurrence(&origin.occurrence).is_some())
    );
}

#[test]
fn canonical_defaults_range_filters_and_compound_conditionals() {
    let mut builder = CompositionBuilder::new(TypeRegistry::new());
    let value = builder.default_cell("default", int()).unwrap();
    builder
        .filter(&value, &[], || {
            let mut body = DynSegment::new::<()>();
            body.op0(|| 2i32..=4i32);
            Ok(body)
        })
        .unwrap();
    builder
        .conditional_compound::<ArrayOf<Leaf<bool>>, _>(
            &[],
            || {
                let mut body = DynSegment::new::<()>();
                body.op0_repr::<ArrayOf<Leaf<bool>>, _>(|| Array::from_vec(vec![true]))?;
                Ok(body)
            },
            vec![(vec![Array::from_vec(vec![true])], vec![])],
            vec![],
        )
        .unwrap();
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let instance = template.instantiate(&mut destination, &[]).unwrap();
    let value = instance
        .cell(instance.occurrences()[0].id(), value.native())
        .unwrap();
    destination.write(value, -1i32).unwrap();
    destination.propagate().unwrap();
    assert_eq!(*destination.read::<i32>(value).unwrap(), 2);
}

#[test]
fn requirement_rejects_an_unregistered_bool_result_without_panicking() {
    let mut types = TypeRegistry::new();
    types.register::<Host>("bool");
    let mut builder = CompositionBuilder::new(types);
    let cell = builder.cell("x", int(), zero).unwrap();
    assert!(
        builder
            .require(&cell, &[], || {
                let mut body = DynSegment::new::<()>();
                body.op0(|| true);
                Ok(body)
            })
            .is_err()
    );
}

#[test]
fn compound_methods_preserve_full_values_across_independent_instances() {
    type Tuple = Cons<Leaf<i32>, Cons<Leaf<bool>, Nil>>;
    let types = TypeRegistry::new();
    let shape = TypeShape::Tuple(vec![int(), TypeShape::Named(TypeId::of::<bool>())]);
    let mut builder = CompositionBuilder::new(types.clone());
    let input = builder.parameter("input", shape.clone()).unwrap();
    let output = builder.default_cell("private", shape.clone()).unwrap();
    let owners = Rc::new(std::cell::RefCell::new(Vec::new()));
    let emitted = owners.clone();
    let descriptor = cel_runtime::ValueType::tuple(vec![
        cel_runtime::AssociatedType {
            offset: 0,
            value_type: (types.get("i32").unwrap().value_type_fn)(),
        },
        cel_runtime::AssociatedType {
            offset: 0,
            value_type: (types.get("bool").unwrap().value_type_fn)(),
        },
    ])
    .unwrap();
    let method = builder
        .method(&[input], std::slice::from_ref(&output), move || {
            let owner = Rc::new(());
            emitted.borrow_mut().push(Rc::downgrade(&owner));
            let mut body = DynSegment::new::<()>();
            body.push_argument(0, descriptor.clone())?;
            body.op1_repr::<Tuple, Tuple, _>(move |tuple| {
                let _retained = &owner;
                tuple
            })?;
            Ok(body)
        })
        .unwrap();
    builder.relationship(vec![method]).unwrap();
    assert!(
        owners
            .borrow()
            .iter()
            .all(|owner| owner.upgrade().is_none())
    );
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let a = destination
        .add_compound_cell::<Tuple>("a", (1, true).into_c_stack_list(), &types)
        .unwrap();
    let b = destination
        .add_compound_cell::<Tuple>("b", (2, false).into_c_stack_list(), &types)
        .unwrap();
    let first = template
        .instantiate(&mut destination, &[("input", a)])
        .unwrap();
    let second = template
        .instantiate(&mut destination, &[("input", b)])
        .unwrap();
    let a_out = first
        .cell(first.occurrences()[0].id(), output.native())
        .unwrap();
    let b_out = second
        .cell(second.occurrences()[0].id(), output.native())
        .unwrap();
    let live: Vec<_> = owners
        .borrow()
        .iter()
        .filter_map(std::rc::Weak::upgrade)
        .collect();
    assert_eq!(live.len(), 2);
    assert!(!Rc::ptr_eq(&live[0], &live[1]));
    drop(live);
    drop(template);
    destination.propagate().unwrap();
    assert_eq!(
        destination.read_compound::<Tuple>(a_out).unwrap(),
        (1, true).into_c_stack_list()
    );
    assert_eq!(
        destination.read_compound::<Tuple>(b_out).unwrap(),
        (2, false).into_c_stack_list()
    );
    destination
        .write_compound::<Tuple>(a, (9, true).into_c_stack_list())
        .unwrap();
    destination.propagate().unwrap();
    assert_eq!(
        destination.read_compound::<Tuple>(a_out).unwrap(),
        (9, true).into_c_stack_list()
    );
    assert_eq!(
        destination.read_compound::<Tuple>(b_out).unwrap(),
        (2, false).into_c_stack_list()
    );
    drop(destination);
    assert!(
        owners
            .borrow()
            .iter()
            .all(|owner| owner.upgrade().is_none())
    );
}

#[test]
fn source_owners_and_parent_links_survive_all_definition_owners() {
    let mut sources = adam_lang::SourceMap::new();
    let first = sources.insert("first.adm2", "first source");
    let mut other_sources = adam_lang::SourceMap::new();
    let second = other_sources.insert("second.adm2", "second source");
    let mut child = CompositionBuilder::new(TypeRegistry::new());
    child.set_label(adam_lang::SourceLabel {
        location: Some(adam_lang::SourceLocation {
            source: first.clone(),
            span: cel_parser::SourceSpan::from_proc_macro2(proc_macro2::Span::call_site()),
        }),
        label: "child".into(),
    });
    let hidden = child.default_cell("hidden", int()).unwrap();
    let child = child.finish().unwrap();
    let mut parent = CompositionBuilder::new(TypeRegistry::new());
    parent.set_label(adam_lang::SourceLabel {
        location: Some(adam_lang::SourceLocation {
            source: second.clone(),
            span: cel_parser::SourceSpan::from_proc_macro2(proc_macro2::Span::call_site()),
        }),
        label: "parent".into(),
    });
    let inclusion = parent.include(&child, &[]).unwrap();
    let parent = parent.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let instance = parent.instantiate(&mut destination, &[]).unwrap();
    let child_occurrence = &instance.occurrences()[1];
    let private = instance
        .cell(child_occurrence.id(), hidden.native())
        .unwrap();
    let records = &destination.instance_origins[0].occurrences;
    assert_eq!(records[1].parent.as_ref(), Some(&records[0].id));
    assert_eq!(
        records[1].inclusion_site.as_ref(),
        Some(&inclusion.relationships()[0])
    );
    drop(parent);
    drop(child);
    drop(sources);
    drop(other_sources);
    drop(instance);
    assert_eq!(destination.cell_shape(private), Some(&int()));
    assert_eq!(destination.sources.len(), 2);
    assert!(destination.sources.contains(&first));
    assert!(destination.sources.contains(&second));
    assert_eq!(first.text(), "first source");
    assert!(destination.cell_names.is_empty());
}

#[test]
fn factory_failures_leave_destination_safely_discardable() {
    let calls = Rc::new(Cell::new(0usize));
    let attempts = calls.clone();
    let mut builder = CompositionBuilder::new(TypeRegistry::new());
    let value = builder.default_cell("value", int()).unwrap();
    let writer = builder
        .method(&[], &[value], move || {
            attempts.set(attempts.get() + 1);
            if attempts.get() > 1 {
                anyhow::bail!("body factory failed");
            }
            zero()
        })
        .unwrap();
    builder.relationship(vec![writer]).unwrap();
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let error = template.instantiate(&mut destination, &[]).unwrap_err();
    assert!(error.to_string().contains("body factory failed"));
    assert_eq!(destination.cells().count(), 1);
    assert!(destination.instance_origins.is_empty());
    drop(destination);

    let mut builder = CompositionBuilder::new(TypeRegistry::new());
    builder
        .cell("fails", int(), || {
            let mut body = DynSegment::new::<()>();
            body.op0r(|| -> anyhow::Result<i32> { anyhow::bail!("initializer failed") });
            Ok(body)
        })
        .unwrap();
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    assert!(
        template
            .instantiate(&mut destination, &[])
            .unwrap_err()
            .to_string()
            .contains("initializer failed")
    );
    drop(destination);
}

#[test]
fn native_kind_and_source_specialization_rejection_precedes_initialization() {
    let types = TypeRegistry::new();
    let mut builder = CompositionBuilder::new(types.clone());
    let output = builder.parameter("output", int()).unwrap();
    let calls = Rc::new(Cell::new(0usize));
    let initializers = calls.clone();
    builder
        .cell("private", int(), move || {
            let initializers = initializers.clone();
            let mut body = DynSegment::new::<()>();
            body.op0(move || {
                initializers.set(initializers.get() + 1);
                0i32
            });
            Ok(body)
        })
        .unwrap();
    let writer = builder.method(&[], &[output], zero).unwrap();
    builder.relationship(vec![writer]).unwrap();
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let source = destination.add_source(3i32);
    destination
        .register_cell("source", source, int(), &types)
        .unwrap();
    assert!(
        template
            .instantiate(&mut destination, &[("output", source)])
            .is_err()
    );
    assert_eq!(destination.cells().count(), 1);
    assert_eq!(calls.get(), 0);
    let out = destination.add_cell(0i32);
    destination
        .add_out(
            adam_rs::Method::from_fn_1_1(source, out, |x: &i32| Ok(*x)),
            vec![],
        )
        .unwrap();
    destination
        .register_cell("out", out, int(), &types)
        .unwrap();
    assert_eq!(destination.output_names["out"], out);
    assert!(
        template
            .instantiate(&mut destination, &[("output", out)])
            .is_err()
    );
    assert_eq!(destination.cells().count(), 2);
    assert_eq!(calls.get(), 0);
}

#[test]
fn deep_last_owner_metadata_teardown_is_iterative() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut template = CompositionBuilder::new(TypeRegistry::new())
                .finish()
                .unwrap();
            for _ in 0..4000 {
                let mut parent = CompositionBuilder::new(TypeRegistry::new());
                parent.include(&template, &[]).unwrap();
                template = parent.finish().unwrap();
            }
            drop(template);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn multi_output_source_specialization_retains_authored_method_indices() {
    let types = TypeRegistry::new();
    let mut builder = CompositionBuilder::new(types.clone());
    let a = builder.parameter("a", int()).unwrap();
    let b = builder.parameter("b", int()).unwrap();
    let c = builder.parameter("c", int()).unwrap();
    let pair = builder
        .method(std::slice::from_ref(&a), &[b.clone(), c.clone()], || {
            let mut body = DynSegment::new::<()>();
            body.push_arg::<i32>(0);
            body.push_arg::<i32>(0);
            body.make_tuple(2, 0)?;
            Ok(body)
        })
        .unwrap();
    let inverse = builder
        .method(&[b, c], &[a], || {
            let mut body = DynSegment::new::<()>();
            body.push_arg::<i32>(0);
            Ok(body)
        })
        .unwrap();
    let relationship = builder.relationship(vec![pair, inverse]).unwrap();
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let a = destination.add_cell(0i32);
    let b = destination.add_source(3i32);
    let c = destination.add_cell(8i32);
    for (name, id) in [("a", a), ("b", b), ("c", c)] {
        destination.register_cell(name, id, int(), &types).unwrap();
    }
    let instance = template
        .instantiate(&mut destination, &[("a", a), ("b", b), ("c", c)])
        .unwrap();
    let group = instance
        .relationship_group(instance.occurrences()[0].id(), &relationship)
        .unwrap();
    assert_eq!(group.len(), 1);
    assert_eq!(destination.method_origins[&(group[0], 0)].authored_index, 1);
    assert!(instance.method_origin(group[0], 1).is_none());
    destination.propagate().unwrap();
    assert_eq!(*destination.read::<i32>(a).unwrap(), 3);
    assert_eq!(*destination.read::<i32>(c).unwrap(), 8);
    let a2 = destination.add_source(5i32);
    let b2 = destination.add_cell(0i32);
    let c2 = destination.add_cell(0i32);
    for (name, id) in [("a2", a2), ("b2", b2), ("c2", c2)] {
        destination.register_cell(name, id, int(), &types).unwrap();
    }
    template
        .instantiate(&mut destination, &[("c", c2), ("a", a2), ("b", b2)])
        .unwrap();
    destination.propagate().unwrap();
    assert_eq!(*destination.read::<i32>(b2).unwrap(), 5);
    assert_eq!(*destination.read::<i32>(c2).unwrap(), 5);
}

#[test]
fn registered_host_leaf_compounds_keep_canonical_capabilities() {
    let mut types = TypeRegistry::new();
    types.register::<Host>("Host");
    let mut builder = CompositionBuilder::new(types);
    let shape = TypeShape::Array(Box::new(TypeShape::Named(TypeId::of::<Host>())));
    let private = builder
        .source("hosts", shape.clone(), || {
            let mut body = DynSegment::new::<()>();
            body.op0_repr::<ArrayOf<Leaf<Host>>, _>(|| Array::from_vec(vec![Host(7)]))?;
            Ok(body)
        })
        .unwrap();
    let template = builder.finish().unwrap();
    let mut destination = ParsedSheet::new();
    let instance = template.instantiate(&mut destination, &[]).unwrap();
    let cell = instance
        .cell(instance.occurrences()[0].id(), private.native())
        .unwrap();
    drop(template);
    assert_eq!(destination.cell_shape(cell), Some(&shape));
    assert_eq!(
        destination
            .read_compound::<ArrayOf<Leaf<Host>>>(cell)
            .unwrap()
            .as_slice(),
        &[Host(7)]
    );
}
