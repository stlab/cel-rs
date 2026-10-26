//! Public contracts for source-free native composition.

use std::any::TypeId;
use std::rc::Rc;

use adam_rs::{CellId, Method, MethodRecipe, Sheet, SheetTemplate, TemplateBuilder, TemplateError};

/// Builds an identity relationship without parsing source text.
fn identity() -> SheetTemplate {
    let mut builder = TemplateBuilder::new();
    let input = builder.parameter::<i32>("input").unwrap();
    let output = builder.parameter::<i32>("output").unwrap();
    let method = MethodRecipe::new(&[input], &[output], |ins, outs| {
        Ok(Method::from_fn_1_1(ins[0], outs[0], |x: &i32| Ok(*x)))
    })
    .unwrap();
    builder.relationship(vec![method]).unwrap();
    builder.finish().unwrap()
}

/// Detects positional binding and accidental writes to caller-owned cells.
#[test]
fn named_bindings_preserve_caller_cells() {
    let template = identity();
    let mut sheet = Sheet::new();
    let input = sheet.add_source(7i32);
    let output = sheet.add_cell(0i32);
    let instance = template
        .instantiate(&mut sheet, &[("output", output), ("input", input)])
        .unwrap();
    assert_eq!(instance.cells(), &[input, output]);
    assert_eq!(instance.relationships().len(), 1);
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(output).unwrap(), 7);
    assert_eq!(*sheet.source::<i32>(input).unwrap(), 7);
}

/// Detects reused private cells or initial values between instantiations.
#[test]
fn private_cells_are_disjoint() {
    let mut builder = TemplateBuilder::new();
    builder.cell("private", || Ok(3i32)).unwrap();
    builder
        .source("source", || Ok(String::from("fresh")))
        .unwrap();
    let template = builder.finish().unwrap();
    assert!(!template.is_relationship_only());
    let mut sheet = Sheet::new();
    let a = template.instantiate(&mut sheet, &[]).unwrap();
    let b = template.instantiate(&mut sheet, &[]).unwrap();
    assert_ne!(a.cells()[0], b.cells()[0]);
    assert_ne!(a.cells()[1], b.cells()[1]);
    sheet.write(a.cells()[0], 9i32).unwrap();
    assert_eq!(*sheet.read::<i32>(b.cells()[0]).unwrap(), 3);
}

/// Detects accepting local handles from another builder.
#[test]
fn foreign_slots_are_rejected() {
    let mut a = TemplateBuilder::new();
    let mut b = TemplateBuilder::new();
    let input = a.parameter::<i32>("input").unwrap();
    let output = b.parameter::<i32>("output").unwrap();
    assert!(matches!(
        MethodRecipe::new(
            std::slice::from_ref(&input),
            std::slice::from_ref(&output),
            |_, _| unreachable!()
        ),
        Err(TemplateError::ForeignHandle { .. })
    ));
    let method = MethodRecipe::new(
        std::slice::from_ref(&input),
        std::slice::from_ref(&input),
        |ins, outs| Ok(Method::from_fn_1_1(ins[0], outs[0], |x: &i32| Ok(*x))),
    )
    .unwrap();
    assert!(matches!(
        b.relationship(vec![method]),
        Err(TemplateError::ForeignHandle { .. })
    ));
    assert!(matches!(
        a.include(Rc::new(identity()), &[("input", output)]),
        Err(TemplateError::ForeignHandle { .. })
    ));
}

/// Detects a parser dependency in the public native path.
#[test]
fn native_use_requires_no_parser() {
    assert!(identity().is_relationship_only());
    let sheet = Sheet::new();
    assert_eq!(sheet.cell_type_id(CellId::default()), None);
}

/// Detects missing binding validation before private initialization or mutation.
#[test]
fn binding_errors_precede_factories() {
    let template = identity();
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(1i32);
    let b = sheet.add_cell(2i32);
    let wrong = sheet.add_cell(false);
    assert_eq!(sheet.cell_type_id(a), Some(TypeId::of::<i32>()));
    assert!(matches!(
        template.instantiate(&mut sheet, &[("input", a)]),
        Err(TemplateError::MissingParameter { .. })
    ));
    assert!(matches!(
        template.instantiate(&mut sheet, &[("input", a), ("extra", b)]),
        Err(TemplateError::UnknownParameter { .. })
    ));
    assert!(matches!(
        template.instantiate(&mut sheet, &[("input", a), ("input", b)]),
        Err(TemplateError::DuplicateParameter { .. })
    ));
    assert!(matches!(
        template.instantiate(&mut sheet, &[("input", a), ("output", a)]),
        Err(TemplateError::AliasedArgument { .. })
    ));
    assert!(matches!(
        template.instantiate(&mut sheet, &[("input", a), ("output", CellId::default())]),
        Err(TemplateError::InvalidActual { .. })
    ));
    assert!(matches!(
        template.instantiate(&mut sheet, &[("input", a), ("output", wrong)]),
        Err(TemplateError::TypeMismatch { .. })
    ));
}

/// Detects incorrect inclusion remapping and loss of inline declaration order.
#[test]
fn inclusion_remaps_private_cells_and_relationships() {
    let child = Rc::new(identity());
    let mut builder = TemplateBuilder::new();
    let a = builder.parameter::<i32>("a").unwrap();
    let b = builder.cell("b", || Ok(0i32)).unwrap();
    let c = builder.parameter::<i32>("c").unwrap();
    let first = builder
        .include(child.clone(), &[("input", a), ("output", b.clone())])
        .unwrap();
    let second = builder
        .include(child, &[("input", b), ("output", c)])
        .unwrap();
    assert_ne!(first.relationships()[0], second.relationships()[0]);
    let template = builder.finish().unwrap();
    let mut sheet = Sheet::new();
    let a = sheet.add_source(7i32);
    let c = sheet.add_cell(0i32);
    let instance = template
        .instantiate(&mut sheet, &[("c", c), ("a", a)])
        .unwrap();
    assert_eq!(instance.relationships().len(), 2);
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 7);
}

/// Detects accepting factories whose positional edges or concrete types differ.
#[test]
fn factories_must_match_declared_edges_and_types() {
    for wrong_type in [false, true] {
        let mut builder = TemplateBuilder::new();
        let a = builder.parameter::<i32>("a").unwrap();
        let b = builder.parameter::<i32>("b").unwrap();
        let recipe = MethodRecipe::new(&[a], &[b], move |ins, outs| {
            Ok(if wrong_type {
                Method::from_fn_1_1(ins[0], outs[0], |v: &bool| Ok(*v))
            } else {
                Method::from_fn_1_1(outs[0], ins[0], |v: &i32| Ok(*v))
            })
        })
        .unwrap();
        builder.relationship(vec![recipe]).unwrap();
        let template = builder.finish().unwrap();
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(1i32);
        let b = sheet.add_cell(0i32);
        assert!(matches!(
            template.instantiate(&mut sheet, &[("a", a), ("b", b)]),
            Err(TemplateError::InvalidDefinition { .. })
        ));
    }
}

/// Detects lost factory errors and malformed declarations.
#[test]
fn definition_and_factory_errors_are_reported() {
    let mut builder = TemplateBuilder::new();
    builder.parameter::<i32>("a").unwrap();
    assert!(matches!(
        builder.parameter::<i32>("a"),
        Err(TemplateError::DuplicateParameter { .. })
    ));
    assert!(matches!(
        builder.relationship(vec![]),
        Err(TemplateError::EmptyRelationship)
    ));
    assert!(matches!(
        MethodRecipe::new(&[], &[], |_, _| unreachable!()),
        Err(TemplateError::InvalidDefinition { .. })
    ));
    builder
        .cell::<i32, _>("fails", || anyhow::bail!("initializer failed"))
        .unwrap();
    let template = builder.finish().unwrap();
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(1i32);
    assert!(matches!(
        template.instantiate(&mut sheet, &[("a", a)]),
        Err(TemplateError::Factory { .. })
    ));
}

/// Detects initializing private cells before binding errors have been ruled out.
#[test]
fn rejected_bindings_leave_destination_and_factories_untouched() {
    let calls = Rc::new(std::cell::Cell::new(0usize));
    let mut builder = TemplateBuilder::new();
    builder.parameter::<i32>("input").unwrap();
    let observed = calls.clone();
    builder
        .cell("private", move || {
            observed.set(observed.get().checked_add(1).unwrap());
            Ok(11i32)
        })
        .unwrap();
    let template = builder.finish().unwrap();
    let mut sheet = Sheet::new();
    let caller = sheet.add_cell(7i32);
    assert!(matches!(
        template.instantiate(&mut sheet, &[]),
        Err(TemplateError::MissingParameter { .. })
    ));
    assert_eq!(calls.get(), 0);
    assert_eq!(*sheet.source::<i32>(caller).unwrap(), 7);
    template
        .instantiate(&mut sheet, &[("input", caller)])
        .unwrap();
    template
        .instantiate(&mut sheet, &[("input", caller)])
        .unwrap();
    assert_eq!(calls.get(), 2);
}

/// Detects reusing a constructed method instead of calling its recipe factory.
#[test]
fn method_factories_run_once_per_instance_not_during_propagation() {
    let calls = Rc::new(std::cell::Cell::new(0usize));
    let observed = calls.clone();
    let mut builder = TemplateBuilder::new();
    let a = builder.parameter::<i32>("a").unwrap();
    let b = builder.parameter::<i32>("b").unwrap();
    builder
        .relationship(vec![
            MethodRecipe::new(&[a], &[b], move |ins, outs| {
                observed.set(observed.get().checked_add(1).unwrap());
                Ok(Method::from_fn_1_1(ins[0], outs[0], |x: &i32| Ok(*x)))
            })
            .unwrap(),
        ])
        .unwrap();
    let template = builder.finish().unwrap();
    assert_eq!(calls.get(), 0);
    let mut sheet = Sheet::new();
    let a = sheet.add_source(7i32);
    let b = sheet.add_cell(0i32);
    let c = sheet.add_cell(0i32);
    template
        .instantiate(&mut sheet, &[("a", a), ("b", b)])
        .unwrap();
    template
        .instantiate(&mut sheet, &[("a", a), ("b", c)])
        .unwrap();
    sheet.propagate().unwrap();
    sheet.write(a, 9i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(calls.get(), 2);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 9);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 9);
}

/// Detects loss of nested inclusion mapping, ownership, or initializer lifetime.
#[test]
fn nested_inclusions_own_fresh_private_declarations() {
    let child = {
        let value = String::from("owned");
        let mut builder = TemplateBuilder::new();
        builder.cell("private", move || Ok(value.clone())).unwrap();
        Rc::new(builder.finish().unwrap())
    };
    let mut parent = TemplateBuilder::default();
    parent.include(child.clone(), &[]).unwrap();
    parent.include(child, &[]).unwrap();
    let mut root = TemplateBuilder::new();
    root.include(Rc::new(parent.finish().unwrap()), &[])
        .unwrap();
    let template = root.finish().unwrap();
    let mut sheet = Sheet::new();
    let instance = template.instantiate(&mut sheet, &[]).unwrap();
    assert_eq!(instance.cells().len(), 2);
    sheet
        .write(instance.cells()[0], String::from("changed"))
        .unwrap();
    assert_eq!(sheet.read::<String>(instance.cells()[1]).unwrap(), "owned");
}

/// Detects missing structural validation before a recipe enters the builder.
#[test]
fn malformed_relationships_are_rejected_without_factories() {
    let mut builder = TemplateBuilder::new();
    let a = builder.parameter::<i32>("a").unwrap();
    let b = builder.parameter::<i32>("b").unwrap();
    let source = builder.source("source", || Ok(1i32)).unwrap();
    let fail = |_: &[CellId], _: &[CellId]| -> anyhow::Result<Method> {
        panic!("structurally rejected recipes must not execute factories")
    };
    assert!(matches!(
        MethodRecipe::new(std::slice::from_ref(&a), &[b.clone(), b.clone()], fail),
        Err(TemplateError::InvalidDefinition { .. })
    ));
    let methods = vec![
        MethodRecipe::new(std::slice::from_ref(&a), std::slice::from_ref(&b), fail).unwrap(),
        MethodRecipe::new(std::slice::from_ref(&a), std::slice::from_ref(&a), fail).unwrap(),
    ];
    assert!(matches!(
        builder.relationship(methods),
        Err(TemplateError::InvalidDefinition { .. })
    ));
    let methods = vec![
        MethodRecipe::new(std::slice::from_ref(&a), std::slice::from_ref(&b), fail).unwrap(),
        MethodRecipe::new(std::slice::from_ref(&a), std::slice::from_ref(&b), fail).unwrap(),
    ];
    assert!(matches!(
        builder.relationship(methods),
        Err(TemplateError::InvalidDefinition { .. })
    ));
    assert!(matches!(
        builder.relationship(vec![MethodRecipe::new(&[a], &[source], fail).unwrap()]),
        Err(TemplateError::InvalidDefinition { .. })
    ));
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0i32);
    let b = sheet.add_cell(0i32);
    let instance = builder
        .finish()
        .unwrap()
        .instantiate(&mut sheet, &[("a", a), ("b", b)])
        .unwrap();
    assert!(instance.relationships().is_empty());
}

/// Detects method factory errors being swallowed instead of returned to the caller.
#[test]
fn failed_method_factory_reports_its_local_site() {
    let mut builder = TemplateBuilder::new();
    let a = builder.parameter::<i32>("a").unwrap();
    let b = builder.parameter::<i32>("b").unwrap();
    let output = b.clone();
    builder
        .relationship(vec![
            MethodRecipe::new(&[a], &[b], |_, _| anyhow::bail!("method factory failed")).unwrap(),
        ])
        .unwrap();
    let template = builder.finish().unwrap();
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(7i32);
    let b = sheet.add_cell(0i32);
    let error = template
        .instantiate(&mut sheet, &[("a", a), ("b", b)])
        .unwrap_err();
    assert!(error.to_string().contains("method factory failed"));
    assert_eq!(
        std::error::Error::source(&error).unwrap().to_string(),
        "method factory failed"
    );
    match error {
        TemplateError::Factory { source, slot } => {
            assert_eq!(slot, output);
            assert_eq!(source.to_string(), "method factory failed");
        }
        _ => panic!("expected a located factory failure"),
    }
}

/// Detects initializing private cells before rejecting out cells in either formal position.
#[test]
fn out_arguments_are_rejected_before_private_initialization() {
    let initializations = Rc::new(std::cell::Cell::new(0usize));
    let observed = initializations.clone();
    let mut builder = TemplateBuilder::new();
    let input = builder.parameter::<i32>("input").unwrap();
    let output = builder.parameter::<i32>("output").unwrap();
    builder
        .cell("private", move || {
            observed.set(observed.get().checked_add(1).unwrap());
            Ok(0i32)
        })
        .unwrap();
    builder
        .include(Rc::new(identity()), &[("input", input), ("output", output)])
        .unwrap();
    let template = builder.finish().unwrap();
    let mut sheet = Sheet::new();
    let source = sheet.add_source(7i32);
    let out = sheet.add_cell(0i32);
    sheet
        .add_out(Method::from_fn_1_1(source, out, |x: &i32| Ok(*x)), vec![])
        .unwrap();
    let ordinary = sheet.add_cell(0i32);
    for bindings in [
        [("input", out), ("output", ordinary)],
        [("input", ordinary), ("output", out)],
    ] {
        let result = template.instantiate(&mut sheet, &bindings);
        assert_eq!(initializations.get(), 0);
        assert!(matches!(result, Err(TemplateError::OutArgument { actual, .. }) if actual == out));
    }
    template
        .instantiate(&mut sheet, &[("input", source), ("output", ordinary)])
        .unwrap();
    assert_eq!(initializations.get(), 1);
}

/// Detects inclusion bypassing the same name, alias, and type rules as live use.
#[test]
fn inclusion_binding_errors_do_not_append_child_structure() {
    let child = Rc::new(identity());
    let mut builder = TemplateBuilder::new();
    let a = builder.parameter::<i32>("a").unwrap();
    let b = builder.parameter::<i32>("b").unwrap();
    let wrong = builder.cell("wrong", || Ok(false)).unwrap();
    assert!(matches!(
        builder.include(child.clone(), &[]),
        Err(TemplateError::MissingParameter { .. })
    ));
    assert!(matches!(
        builder.include(child.clone(), &[("extra", a.clone())]),
        Err(TemplateError::UnknownParameter { .. })
    ));
    assert!(matches!(
        builder.include(child.clone(), &[("input", a.clone()), ("input", b.clone())]),
        Err(TemplateError::DuplicateParameter { .. })
    ));
    assert!(matches!(
        builder.include(
            child.clone(),
            &[("input", a.clone()), ("output", a.clone())]
        ),
        Err(TemplateError::AliasedArgument { .. })
    ));
    assert!(matches!(
        builder.include(child.clone(), &[("input", a.clone()), ("output", wrong)]),
        Err(TemplateError::TypeMismatch { .. })
    ));
    let inclusion = builder
        .include(child, &[("input", a), ("output", b)])
        .unwrap();
    assert_eq!(inclusion.relationships().len(), 1);
    let mut sheet = Sheet::new();
    let a = sheet.add_source(7i32);
    let b = sheet.add_cell(0i32);
    let instance = builder
        .finish()
        .unwrap()
        .instantiate(&mut sheet, &[("a", a), ("b", b)])
        .unwrap();
    assert_eq!(instance.cells().len(), 3);
    assert_eq!(instance.relationships().len(), 1);
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 7);
}

/// Detects writes or reallocations that change caller priorities during binding.
#[test]
fn bidirectional_use_preserves_caller_priorities() {
    let mut builder = TemplateBuilder::new();
    let a = builder.parameter::<i32>("a").unwrap();
    let b = builder.parameter::<i32>("b").unwrap();
    let forward = MethodRecipe::new(
        std::slice::from_ref(&a),
        std::slice::from_ref(&b),
        |ins, outs| Ok(Method::from_fn_1_1(ins[0], outs[0], |x: &i32| Ok(*x))),
    )
    .unwrap();
    let backward = MethodRecipe::new(&[b], &[a], |ins, outs| {
        Ok(Method::from_fn_1_1(ins[0], outs[0], |x: &i32| Ok(*x)))
    })
    .unwrap();
    builder.relationship(vec![forward, backward]).unwrap();
    let template = builder.finish().unwrap();
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(7i32);
    let b = sheet.add_cell(2i32);
    sheet.write(a, 9i32).unwrap();
    template
        .instantiate(&mut sheet, &[("a", a), ("b", b)])
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 9);
    assert_eq!(*sheet.source::<i32>(a).unwrap(), 9);
}

/// Detects omitting zero-input methods or empty templates from the native substrate.
#[test]
fn constants_and_empty_templates_instantiate() {
    let mut sheet = Sheet::new();
    let empty = TemplateBuilder::new()
        .finish()
        .unwrap()
        .instantiate(&mut sheet, &[])
        .unwrap();
    assert!(empty.cells().is_empty());
    assert!(empty.relationships().is_empty());
    let mut builder = TemplateBuilder::new();
    let output = builder.cell("constant", || Ok(0i32)).unwrap();
    builder
        .relationship(vec![
            MethodRecipe::new(&[], &[output], |_, outs| {
                Ok(Method::new(
                    vec![],
                    vec![outs[0]],
                    vec![],
                    vec![TypeId::of::<i32>()],
                    |_| Ok(vec![Box::new(42i32)]),
                ))
            })
            .unwrap(),
        ])
        .unwrap();
    let instance = builder
        .finish()
        .unwrap()
        .instantiate(&mut sheet, &[])
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(instance.cells()[0]).unwrap(), 42);
}

/// Detects eagerly flattening transitive relationships during recipe inclusion.
#[test]
fn inclusion_groups_keep_repeated_child_dags_compact() {
    let mut child = Rc::new(identity());
    for _ in 0..64 {
        let mut builder = TemplateBuilder::new();
        let a = builder.parameter::<i32>("input").unwrap();
        let b = builder.parameter::<i32>("output").unwrap();
        for _ in 0..2 {
            let group = builder
                .include(
                    child.clone(),
                    &[("input", a.clone()), ("output", b.clone())],
                )
                .unwrap();
            assert_eq!(group.relationships().len(), 1);
        }
        child = Rc::new(builder.finish().unwrap());
        assert!(child.is_relationship_only());
    }
}

/// Detects collapsing included group members into one runtime relationship.
#[test]
fn relationship_groups_expand_to_distinct_live_relationships() {
    let mut leaf = TemplateBuilder::new();
    let a = leaf.cell("a", || Ok(1i32)).unwrap();
    let b = leaf.cell("b", || Ok(0i32)).unwrap();
    leaf.relationship(vec![
        MethodRecipe::new(&[a], &[b], |ins, outs| {
            Ok(Method::from_fn_1_1(ins[0], outs[0], |x: &i32| Ok(*x)))
        })
        .unwrap(),
    ])
    .unwrap();
    let leaf = Rc::new(leaf.finish().unwrap());
    let mut middle = TemplateBuilder::new();
    middle.include(leaf.clone(), &[]).unwrap();
    middle.include(leaf, &[]).unwrap();
    let middle = Rc::new(middle.finish().unwrap());
    let mut root = TemplateBuilder::new();
    let group = root.include(middle.clone(), &[]).unwrap();
    assert_eq!(group.relationships().len(), 1);
    root.include(middle, &[]).unwrap();
    let mut sheet = Sheet::new();
    let live = root.finish().unwrap().instantiate(&mut sheet, &[]).unwrap();
    assert_eq!(live.cells().len(), 8);
    assert_eq!(live.relationships().len(), 4);
    for (i, id) in live.relationships().iter().enumerate() {
        assert!(!live.relationships()[..i].contains(id));
    }
}

/// Detects missing transitive output summaries before source specialization exists.
#[test]
fn nested_source_outputs_are_rejected_before_factories() {
    let mut parent = TemplateBuilder::new();
    let a = parent.parameter::<i32>("a").unwrap();
    let b = parent.parameter::<i32>("b").unwrap();
    parent
        .cell::<i32, _>("private", || {
            panic!("preflight must precede initialization")
        })
        .unwrap();
    parent
        .include(Rc::new(identity()), &[("input", a), ("output", b)])
        .unwrap();
    let parent = Rc::new(parent.finish().unwrap());
    let mut root = TemplateBuilder::new();
    let a = root.parameter::<i32>("a").unwrap();
    let b = root.parameter::<i32>("b").unwrap();
    root.include(parent, &[("a", a), ("b", b)]).unwrap();
    let template = root.finish().unwrap();
    assert!(!template.is_relationship_only());
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(7i32);
    let b = sheet.add_source(0i32);
    assert!(matches!(
        template.instantiate(&mut sheet, &[("a", a), ("b", b)]),
        Err(TemplateError::Runtime(
            adam_rs::Error::InvalidCellKind { .. }
        ))
    ));
}
