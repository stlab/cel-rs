//! Public contracts for source-free native composition.

use std::any::TypeId;
use std::rc::Rc;

use adam_rs::{
    CellId, CellKind, Filter, MatchExpr, Method, MethodRecipe, Requirement, Sheet, SheetTemplate,
    TemplateBuilder, TemplateError,
};

/// Keeps complete masks distinct and reports the active failing inclusion path.
#[test]
fn source_preflight_distinguishes_masks_and_preserves_failure_ancestry() {
    let mut leaf = TemplateBuilder::new();
    let input = leaf.parameter::<i32>("input").unwrap();
    let output = leaf.parameter::<i32>("output").unwrap();
    let relationship = leaf
        .relationship(vec![
            MethodRecipe::new(&[input], &[output], |_, _| {
                panic!("source checking must not run method factories")
            })
            .unwrap(),
        ])
        .unwrap();
    let leaf = Rc::new(leaf.finish().unwrap());
    let leaf_id = leaf.id();
    let mut wrapper = TemplateBuilder::new();
    let a = wrapper.parameter::<i32>("a").unwrap();
    let b = wrapper.parameter::<i32>("b").unwrap();
    let inner = wrapper
        .include(leaf, &[("input", a), ("output", b)])
        .unwrap();
    let inner_site = inner.relationships()[0].clone();
    let wrapper = Rc::new(wrapper.finish().unwrap());
    let mut root = TemplateBuilder::new();
    let x = root.parameter::<i32>("x").unwrap();
    let y = root.parameter::<i32>("y").unwrap();
    root.include(wrapper.clone(), &[("a", y.clone()), ("b", x.clone())])
        .unwrap();
    let failing = root.include(wrapper, &[("a", x), ("b", y)]).unwrap();
    let failing_site = failing.relationships()[0].clone();
    let root = root.finish().unwrap();
    for _ in 0..2 {
        let error = root
            .check_source_bindings(&[("x", false), ("y", true)])
            .unwrap_err();
        let TemplateError::Construction {
            template,
            site,
            inclusions,
            source,
        } = error
        else {
            panic!("expected located empty relationship");
        };
        assert_eq!(template, leaf_id);
        let adam_rs::template::ConstructionSite::Relationship(site) = site else {
            panic!("expected failed leaf relationship");
        };
        assert_eq!(site, relationship);
        assert_eq!(inclusions, [failing_site.clone(), inner_site.clone()]);
        assert!(matches!(*source, TemplateError::EmptyRelationship));
    }
    root.check_source_bindings(&[("x", false), ("y", false)])
        .unwrap();
}

/// Creates an ordinary identity writer for two local declarations.
fn copy_recipe(input: &adam_rs::CellSlot, output: &adam_rs::CellSlot) -> MethodRecipe {
    MethodRecipe::new(
        std::slice::from_ref(input),
        std::slice::from_ref(output),
        |i, o| Ok(Method::from_fn_1_1(i[0], o[0], |x: &i32| Ok(*x))),
    )
    .unwrap()
}

/// Detects missing private policies, output promotion, or changed inline runtime semantics.
#[test]
fn private_outputs_filters_requirements_work() {
    let mut builder = TemplateBuilder::new();
    let input = builder.cell("input", || Ok(3i32)).unwrap();
    let output = builder.cell("output", || Ok(0i32)).unwrap();
    builder
        .filter(&input, &[], |_, _| {
            Ok(Filter::from_fn_0(|x: &i32| Ok((*x).max(0))))
        })
        .unwrap();
    builder.out(copy_recipe(&input, &output)).unwrap();
    builder
        .require(&output, std::slice::from_ref(&output), |i| {
            Ok(Requirement::from_fn_1(i[0], |x: &i32| Ok(*x < 10)))
        })
        .unwrap();
    let template = builder.finish().unwrap();
    let mut sheet = Sheet::new();
    let instance = template.instantiate(&mut sheet, &[]).unwrap();
    drop(template);
    let mut inline = Sheet::new();
    let a = inline.add_cell(3i32);
    let b = inline.add_cell(0i32);
    inline
        .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).max(0))))
        .unwrap();
    inline
        .add_out(Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)), vec![])
        .unwrap();
    inline
        .add_requirement(b, Requirement::from_fn_1(b, |x: &i32| Ok(*x < 10)))
        .unwrap();
    for edit in [3, -2, 12, 4] {
        sheet.write(instance.cells()[0], edit).unwrap();
        inline.write(a, edit).unwrap();
        sheet.propagate().unwrap();
        inline.propagate().unwrap();
        for (&actual, expected) in instance.cells().iter().zip([a, b]) {
            assert_eq!(
                sheet.read::<i32>(actual).unwrap(),
                inline.read::<i32>(expected).unwrap()
            );
            assert_eq!(
                sheet.source::<i32>(actual).unwrap(),
                inline.source::<i32>(expected).unwrap()
            );
            assert_eq!(sheet.cell_kind(actual), inline.cell_kind(expected));
            assert_eq!(sheet.is_source(actual), inline.is_source(expected));
            assert_eq!(
                sheet.cell_requirements_valid(actual),
                inline.cell_requirements_valid(expected)
            );
        }
        assert_eq!(sheet.selected_method(instance.relationships()[0]), Some(0));
    }
    assert_eq!(sheet.cell_kind(instance.cells()[1]), Some(CellKind::Out));
    assert!(sheet.write(instance.cells()[1], 1i32).is_err());
}

/// Detects component-owned policies or output promotion attached to caller-owned formals.
#[test]
fn parameters_cannot_acquire_private_policies() {
    let mut builder = TemplateBuilder::new();
    let p = builder.parameter::<i32>("p").unwrap();
    let q = builder.parameter::<i32>("q").unwrap();
    assert!(matches!(
        builder.out(copy_recipe(&p, &q)),
        Err(TemplateError::InvalidDefinition { .. })
    ));
    assert!(matches!(
        builder.filter(&p, &[], |_, _| Ok(Filter::from_fn_0(|x: &i32| Ok(*x)))),
        Err(TemplateError::InvalidDefinition { .. })
    ));
    assert!(matches!(
        builder.require(&p, std::slice::from_ref(&p), |i| Ok(
            Requirement::from_fn_1(i[0], |_: &i32| Ok(true))
        )),
        Err(TemplateError::InvalidDefinition { .. })
    ));
}

/// Detects replaced caller policies after composition, including observable filter errors.
#[test]
fn caller_filters_and_requirements_remain_effective() {
    let template = identity();
    let mut sheet = Sheet::new();
    let input = sheet.add_cell(2i32);
    let output = sheet.add_cell(0i32);
    sheet
        .add_filter(
            input,
            Filter::from_fn_0(|x: &i32| {
                anyhow::ensure!(*x >= 0, "negative caller edit");
                Ok(*x)
            }),
        )
        .unwrap();
    let requirement = sheet
        .add_requirement(input, Requirement::from_fn_1(input, |x: &i32| Ok(*x < 10)))
        .unwrap();
    template
        .instantiate(&mut sheet, &[("input", input), ("output", output)])
        .unwrap();
    assert_eq!(
        sheet.cell_requirements(input),
        Some([requirement].as_slice())
    );
    sheet.write(input, -1i32).unwrap();
    sheet.propagate().unwrap();
    assert!(
        matches!(sheet.filter_violation(input), Some(adam_rs::FilterViolation::Failed(error)) if error.to_string() == "negative caller edit")
    );
    assert_eq!(*sheet.source::<i32>(input).unwrap(), -1);
    sheet.write(input, 12i32).unwrap();
    sheet.propagate().unwrap();
    assert!(!sheet.cell_requirements_valid(input));
    assert_eq!(*sheet.read::<i32>(output).unwrap(), 12);
}

/// Detects accepting state hidden behind transitive inclusion groups in a branch.
#[test]
fn conditional_wrapper_rejects_transitive_state() {
    let mut leaf = TemplateBuilder::new();
    leaf.cell("state", || Ok(0i32)).unwrap();
    let mut wrapper = TemplateBuilder::new();
    wrapper
        .include(Rc::new(leaf.finish().unwrap()), &[])
        .unwrap();
    let mut parent = TemplateBuilder::new();
    let guard = parent.parameter::<bool>("guard").unwrap();
    let group = parent
        .include(Rc::new(wrapper.finish().unwrap()), &[])
        .unwrap();
    assert!(matches!(
        parent.conditional(
            &[guard],
            |i| Ok(MatchExpr::cell(i[0])),
            vec![(vec![true], group.relationships().to_vec())],
            vec![]
        ),
        Err(TemplateError::ConditionalComponent)
    ));
}

/// Detects flattened-away constituent identities or a second conditional activation layer.
#[test]
fn nested_relationship_groups_switch_branches() {
    let mut wrapper = TemplateBuilder::new();
    let a = wrapper.parameter::<i32>("input").unwrap();
    let b = wrapper.parameter::<i32>("output").unwrap();
    wrapper
        .include(Rc::new(identity()), &[("input", a), ("output", b)])
        .unwrap();
    let mut parent = TemplateBuilder::new();
    let guard = parent.source("guard", || Ok(false)).unwrap();
    let input = parent.source("input", || Ok(7i32)).unwrap();
    let output = parent.cell("output", || Ok(0i32)).unwrap();
    let included = parent
        .include(
            Rc::new(wrapper.finish().unwrap()),
            &[("input", input), ("output", output)],
        )
        .unwrap();
    parent
        .conditional(
            &[guard],
            |i| Ok(MatchExpr::cell(i[0])),
            vec![(vec![true], included.relationships().to_vec())],
            vec![],
        )
        .unwrap();
    let template = parent.finish().unwrap();
    assert!(!template.is_relationship_only());
    let mut sheet = Sheet::new();
    let instance = template.instantiate(&mut sheet, &[]).unwrap();
    drop(template);
    assert_eq!(instance.relationships().len(), 1);
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(instance.cells()[2]).unwrap(), 0);
    sheet.write(instance.cells()[0], true).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(instance.cells()[2]).unwrap(), 7);
    assert_eq!(sheet.selected_method(instance.relationships()[0]), Some(0));
    sheet.write(instance.cells()[0], false).unwrap();
    sheet.write(instance.cells()[1], 9i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(instance.cells()[2]).unwrap(), 0);
    assert_eq!(sheet.selected_method(instance.relationships()[0]), None);
}

/// Detects missing constituent group members and differing native branch/default behavior.
#[test]
fn conditional_groups_match_manual_inline_after_edits() {
    let mut child = TemplateBuilder::new();
    let input = child.parameter::<i32>("input").unwrap();
    let a = child.parameter::<i32>("a").unwrap();
    let b = child.parameter::<i32>("b").unwrap();
    child.relationship(vec![copy_recipe(&input, &a)]).unwrap();
    child.relationship(vec![copy_recipe(&input, &b)]).unwrap();
    let child = Rc::new(child.finish().unwrap());
    let mut builder = TemplateBuilder::new();
    let guard = builder.parameter::<bool>("guard").unwrap();
    let input = builder.parameter::<i32>("input").unwrap();
    let fallback = builder.parameter::<i32>("fallback").unwrap();
    let a = builder.parameter::<i32>("a").unwrap();
    let b = builder.parameter::<i32>("b").unwrap();
    let yes = builder
        .include(
            child.clone(),
            &[("input", input), ("a", a.clone()), ("b", b.clone())],
        )
        .unwrap();
    let no = builder
        .include(child, &[("input", fallback), ("a", a), ("b", b)])
        .unwrap();
    builder
        .conditional(
            &[guard],
            |i| Ok(MatchExpr::from_fn_1(i[0], |x: &bool| Ok(*x))),
            vec![(vec![true], yes.relationships().to_vec())],
            no.relationships().to_vec(),
        )
        .unwrap();
    let template = builder.finish().unwrap();
    let mut sheet = Sheet::new();
    let g = sheet.add_source(false);
    let input = sheet.add_source(7i32);
    let fallback = sheet.add_source(9i32);
    let a = sheet.add_cell(0i32);
    let b = sheet.add_cell(0i32);
    let instance = template
        .instantiate(
            &mut sheet,
            &[
                ("guard", g),
                ("input", input),
                ("fallback", fallback),
                ("a", a),
                ("b", b),
            ],
        )
        .unwrap();
    let mut inline = Sheet::new();
    let ig = inline.add_source(false);
    let ii = inline.add_source(7i32);
    let fi = inline.add_source(9i32);
    let ia = inline.add_cell(0i32);
    let ib = inline.add_cell(0i32);
    let rels = [(ii, ia), (ii, ib), (fi, ia), (fi, ib)].map(|(i, o)| {
        inline
            .add_relationship(vec![Method::from_fn_1_1(i, o, |x: &i32| Ok(*x))])
            .unwrap()
    });
    inline
        .add_conditional(
            MatchExpr::from_fn_1(ig, |x: &bool| Ok(*x)),
            vec![(vec![true], rels[..2].to_vec())],
            rels[2..].to_vec(),
        )
        .unwrap();
    assert_eq!(instance.relationships().len(), 4);
    drop(template);
    for edit in [false, true, false, true] {
        sheet.write(g, edit).unwrap();
        inline.write(ig, edit).unwrap();
        sheet.propagate().unwrap();
        inline.propagate().unwrap();
        for (actual, expected) in [(a, ia), (b, ib)] {
            assert_eq!(
                sheet.read::<i32>(actual).unwrap(),
                inline.read::<i32>(expected).unwrap()
            );
            assert_eq!(
                sheet.source::<i32>(actual).unwrap(),
                inline.source::<i32>(expected).unwrap()
            );
            assert_eq!(sheet.is_source(actual), inline.is_source(expected));
        }
        for (actual, expected) in instance.relationships().iter().zip(rels) {
            assert_eq!(
                sheet.selected_method(*actual),
                inline.selected_method(expected)
            );
        }
    }
}

/// Detects treating conditionals with no private cells as structurally relationship-only.
#[test]
fn conditional_without_private_state_is_still_ineligible_for_branch_inclusion() {
    let mut child = TemplateBuilder::new();
    let guard = child.parameter::<bool>("guard").unwrap();
    child
        .conditional(
            &[guard],
            |i| Ok(MatchExpr::cell(i[0])),
            vec![(vec![true], vec![])],
            vec![],
        )
        .unwrap();
    let child = Rc::new(child.finish().unwrap());
    assert!(!child.is_relationship_only());
    let mut parent = TemplateBuilder::new();
    let guard = parent.parameter::<bool>("guard").unwrap();
    let included = parent.include(child, &[("guard", guard.clone())]).unwrap();
    assert!(matches!(
        parent.conditional(
            &[guard],
            |i| Ok(MatchExpr::cell(i[0])),
            vec![(vec![true], included.relationships().to_vec())],
            vec![]
        ),
        Err(TemplateError::ConditionalComponent)
    ));
}

/// Detects rejected empty zero-parameter groups or invalid ownership in an empty conditional.
#[test]
fn zero_parameter_empty_groups_can_be_conditional() {
    let mut builder = TemplateBuilder::new();
    let included = builder
        .include(Rc::new(TemplateBuilder::new().finish().unwrap()), &[])
        .unwrap();
    builder
        .conditional::<bool, _>(
            &[],
            |_| {
                Ok(MatchExpr::new(
                    vec![],
                    vec![],
                    TypeId::of::<bool>(),
                    |a, b| a.downcast_ref::<bool>() == b.downcast_ref::<bool>(),
                    |_| Ok(Box::new(true)),
                ))
            },
            vec![(vec![true], included.relationships().to_vec())],
            vec![],
        )
        .unwrap();
    let mut sheet = Sheet::new();
    let instance = builder
        .finish()
        .unwrap()
        .instantiate(&mut sheet, &[])
        .unwrap();
    sheet.propagate().unwrap();
    assert!(instance.cells().is_empty());
    assert!(instance.relationships().is_empty());
}

/// Detects recursive traversal or last-owner destruction of an accepted deep include chain.
#[test]
fn deep_inclusions_expand_and_drop_without_host_recursion() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let mut root = Rc::new(TemplateBuilder::new().finish().unwrap());
            for _ in 0..32768 {
                let mut builder = TemplateBuilder::new();
                builder.include(root, &[]).unwrap();
                root = Rc::new(builder.finish().unwrap());
            }
            let instance = root.instantiate(&mut Sheet::new(), &[]).unwrap();
            assert!(instance.cells().is_empty());
            assert!(instance.relationships().is_empty());
            eprintln!("instantiated 32768 includes; dropping sole root");
            drop(root);
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Detects releasing a child's factories while another including template still owns it.
#[test]
fn shared_child_factories_live_until_the_last_template_owner_drops() {
    let captured = Rc::new(7i32);
    let lifetime = Rc::downgrade(&captured);
    let mut child = TemplateBuilder::new();
    child.cell("private", move || Ok(*captured)).unwrap();
    let child = Rc::new(child.finish().unwrap());
    let mut first = TemplateBuilder::new();
    first.include(child.clone(), &[]).unwrap();
    let first = first.finish().unwrap();
    let mut second = TemplateBuilder::new();
    second.include(child.clone(), &[]).unwrap();
    let second = second.finish().unwrap();
    drop(child);
    drop(first);
    assert!(lifetime.upgrade().is_some());
    let mut sheet = Sheet::new();
    let instance = second.instantiate(&mut sheet, &[]).unwrap();
    assert_eq!(*sheet.read::<i32>(instance.cells()[0]).unwrap(), 7);
    drop(second);
    assert!(lifetime.upgrade().is_none());
    sheet.write(instance.cells()[0], 9i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(instance.cells()[0]).unwrap(), 9);
}

/// Detects premature release through repeated DAG edges or an external child owner.
#[test]
fn repeated_child_edges_preserve_external_template_ownership() {
    let captured = Rc::new(3i32);
    let lifetime = Rc::downgrade(&captured);
    let mut leaf = TemplateBuilder::new();
    leaf.cell("private", move || Ok(*captured)).unwrap();
    let leaf = Rc::new(leaf.finish().unwrap());
    let mut parent = TemplateBuilder::new();
    parent.include(leaf.clone(), &[]).unwrap();
    parent.include(leaf.clone(), &[]).unwrap();
    let parent = parent.finish().unwrap();
    let mut sheet = Sheet::new();
    let instance = parent.instantiate(&mut sheet, &[]).unwrap();
    assert_ne!(instance.cells()[0], instance.cells()[1]);
    drop(parent);
    assert!(lifetime.upgrade().is_some());
    let instance = leaf.instantiate(&mut sheet, &[]).unwrap();
    assert_eq!(*sheet.read::<i32>(instance.cells()[0]).unwrap(), 3);
    drop(leaf);
    assert!(lifetime.upgrade().is_none());
}

/// Builds three equality directions, each preserving its entire output tuple.
fn equality(builder: &mut TemplateBuilder, slots: &[adam_rs::CellSlot; 3]) {
    let methods = (0..3)
        .map(|input| {
            let outputs = (0..3)
                .filter(|&i| i != input)
                .map(|i| slots[i].clone())
                .collect::<Vec<_>>();
            MethodRecipe::new(&[slots[input].clone()], &outputs, |ins, outs| {
                Ok(Method::new(
                    ins.to_vec(),
                    outs.to_vec(),
                    vec![TypeId::of::<i32>()],
                    vec![TypeId::of::<i32>(); 2],
                    |values| {
                        let x = *values[0].downcast_ref::<i32>().unwrap();
                        Ok(vec![Box::new(x), Box::new(x)])
                    },
                ))
            })
            .unwrap()
        })
        .collect();
    builder.relationship(methods).unwrap();
}

/// Detects tuple projection or rejection instead of whole-method source specialization.
#[test]
fn source_binding_removes_whole_multi_output_method() {
    let mut builder = TemplateBuilder::new();
    let slots = [
        builder.parameter::<i32>("a").unwrap(),
        builder.parameter::<i32>("b").unwrap(),
        builder.parameter::<i32>("c").unwrap(),
    ];
    equality(&mut builder, &slots);
    let template = builder.finish().unwrap();
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(4i32);
    let b = sheet.add_source(10i32);
    let c = sheet.add_cell(4i32);
    let instance = template
        .instantiate(&mut sheet, &[("a", a), ("b", b), ("c", c)])
        .unwrap();
    assert_eq!(instance.relationships().len(), 1);
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 10);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 10);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 10);
    assert_eq!(*sheet.source::<i32>(b).unwrap(), 10);
    let mut inline = Sheet::new();
    let ia = inline.add_cell(4i32);
    let ib = inline.add_source(10i32);
    let ic = inline.add_cell(4i32);
    let relation = inline
        .add_relationship(vec![Method::new(
            vec![ib],
            vec![ia, ic],
            vec![TypeId::of::<i32>()],
            vec![TypeId::of::<i32>(); 2],
            |values| {
                let x = *values[0].downcast_ref::<i32>().unwrap();
                Ok(vec![Box::new(x), Box::new(x)])
            },
        )])
        .unwrap();
    for edit in [10, 15, -3] {
        sheet.write(b, edit).unwrap();
        inline.write(ib, edit).unwrap();
        sheet.propagate().unwrap();
        inline.propagate().unwrap();
        for (actual, expected) in [(a, ia), (b, ib), (c, ic)] {
            assert_eq!(
                sheet.read::<i32>(actual).unwrap(),
                inline.read::<i32>(expected).unwrap()
            );
            assert_eq!(
                sheet.source::<i32>(actual).unwrap(),
                inline.source::<i32>(expected).unwrap()
            );
            assert_eq!(sheet.cell_kind(actual), inline.cell_kind(expected));
            assert_eq!(sheet.is_source(actual), inline.is_source(expected));
        }
        assert_eq!(
            sheet.selected_method(instance.relationships()[0]),
            inline.selected_method(relation)
        );
    }
}

/// Detects rejection of private sources, including formals bound through an inclusion.
#[test]
fn private_sources_specialize_included_methods() {
    let mut child = TemplateBuilder::new();
    let a = child.parameter::<i32>("a").unwrap();
    let b = child.parameter::<i32>("b").unwrap();
    child
        .relationship(vec![copy_recipe(&a, &b), copy_recipe(&b, &a)])
        .unwrap();
    let mut parent = TemplateBuilder::new();
    let a = parent.cell("a", || Ok(0i32)).unwrap();
    let b = parent.source("b", || Ok(8i32)).unwrap();
    parent
        .include(Rc::new(child.finish().unwrap()), &[("a", a), ("b", b)])
        .unwrap();
    let mut sheet = Sheet::new();
    let instance = parent
        .finish()
        .unwrap()
        .instantiate(&mut sheet, &[])
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(instance.cells()[0]).unwrap(), 8);
    assert_eq!(sheet.cell_kind(instance.cells()[1]), Some(CellKind::Source));

    let mut direct = TemplateBuilder::new();
    let slots = [
        direct.cell("a", || Ok(4i32)).unwrap(),
        direct.source("b", || Ok(10i32)).unwrap(),
        direct.cell("c", || Ok(4i32)).unwrap(),
    ];
    equality(&mut direct, &slots);
    let instance = direct
        .finish()
        .unwrap()
        .instantiate(&mut sheet, &[])
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(instance.cells()[0]).unwrap(), 10);
    assert_eq!(*sheet.read::<i32>(instance.cells()[2]).unwrap(), 10);
}

/// Detects factory validation skipped when its entire direction will be removed.
#[test]
fn pruned_method_factories_still_validate_every_edge_and_type() {
    for wrong_edges in [false, true] {
        let mut builder = TemplateBuilder::new();
        let a = builder.parameter::<i32>("a").unwrap();
        let b = builder.parameter::<i32>("b").unwrap();
        let malformed = MethodRecipe::new(
            std::slice::from_ref(&a),
            std::slice::from_ref(&b),
            move |i, o| {
                if wrong_edges {
                    Ok(Method::from_fn_1_1(o[0], i[0], |x: &i32| Ok(*x)))
                } else {
                    Ok(Method::from_fn_1_1(i[0], o[0], |_: &i32| Ok(false)))
                }
            },
        )
        .unwrap();
        builder
            .relationship(vec![malformed, copy_recipe(&b, &a)])
            .unwrap();
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0i32);
        let b = sheet.add_source(10i32);
        assert!(matches!(
            builder
                .finish()
                .unwrap()
                .instantiate(&mut sheet, &[("a", a), ("b", b)]),
            Err(TemplateError::Construction { source, .. })
                if matches!(*source, TemplateError::InvalidDefinition { .. })
        ));
        assert_eq!(sheet.relationships().count(), 0);
    }
}

/// Detects ignored inactive constituent failures before any private factory executes.
#[test]
fn inactive_branch_empty_relationship_is_preflight_error() {
    let calls = Rc::new(std::cell::Cell::new(0));
    let mut builder = TemplateBuilder::new();
    let count = calls.clone();
    builder
        .cell("uninitialized", move || {
            count.set(count.get() + 1);
            Ok(0i32)
        })
        .unwrap();
    let guard = builder.source("guard", || Ok(false)).unwrap();
    let a = builder.parameter::<i32>("a").unwrap();
    let b = builder.parameter::<i32>("b").unwrap();
    let included = builder
        .include(Rc::new(identity()), &[("input", a), ("output", b)])
        .unwrap();
    builder
        .conditional(
            &[guard],
            |i| Ok(MatchExpr::cell(i[0])),
            vec![(vec![true], included.relationships().to_vec())],
            vec![],
        )
        .unwrap();
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0i32);
    let b = sheet.add_source(1i32);
    assert!(matches!(
        builder
            .finish()
            .unwrap()
            .instantiate(&mut sheet, &[("a", a), ("b", b)]),
        Err(TemplateError::Construction { source, .. })
            if matches!(*source, TemplateError::EmptyRelationship)
    ));
    assert_eq!(calls.get(), 0);
    assert_eq!(sheet.relationships().count(), 0);
}

/// Detects mutation or initializer execution before an emptied relationship is diagnosed.
#[test]
fn all_directions_removed_is_error() {
    let calls = Rc::new(std::cell::Cell::new(0));
    let mut builder = TemplateBuilder::new();
    let count = calls.clone();
    builder
        .cell("private", move || {
            count.set(count.get() + 1);
            Ok(0i32)
        })
        .unwrap();
    let a = builder.parameter::<i32>("a").unwrap();
    let b = builder.parameter::<i32>("b").unwrap();
    builder
        .relationship(vec![
            MethodRecipe::new(&[a], &[b], |i, o| {
                Ok(Method::from_fn_1_1(i[0], o[0], |x: &i32| Ok(*x)))
            })
            .unwrap(),
        ])
        .unwrap();
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(1i32);
    let b = sheet.add_source(2i32);
    assert!(matches!(
        builder
            .finish()
            .unwrap()
            .instantiate(&mut sheet, &[("a", a), ("b", b)]),
        Err(TemplateError::Construction { source, .. })
            if matches!(*source, TemplateError::EmptyRelationship)
    ));
    assert_eq!(calls.get(), 0);
}

/// Detects exponential expansion before compact-DAG overflow is reported.
#[test]
fn expansion_size_overflow_is_diagnostic() {
    let mut leaf = TemplateBuilder::new();
    leaf.cell::<i32, _>("never", || anyhow::bail!("overflow must precede factories"))
        .unwrap();
    let mut child = Rc::new(leaf.finish().unwrap());
    for _ in 0..usize::BITS {
        let mut builder = TemplateBuilder::new();
        builder.include(child.clone(), &[]).unwrap();
        builder.include(child, &[]).unwrap();
        child = Rc::new(builder.finish().unwrap());
    }
    assert!(matches!(
        child.instantiate(&mut Sheet::new(), &[]),
        Err(TemplateError::Capacity)
    ));
}

/// Detects infallible reservation for representable but unreservable live expansions.
#[test]
fn expansion_reservation_failure_is_diagnostic() {
    let mut leaf = TemplateBuilder::new();
    leaf.cell::<i32, _>("never", || {
        anyhow::bail!("reservation must precede factories")
    })
    .unwrap();
    let mut child = Rc::new(leaf.finish().unwrap());
    for _ in 0..usize::BITS - 4 {
        let mut builder = TemplateBuilder::new();
        builder.include(child.clone(), &[]).unwrap();
        builder.include(child, &[]).unwrap();
        child = Rc::new(builder.finish().unwrap());
    }
    assert!(matches!(
        child.instantiate(&mut Sheet::new(), &[]),
        Err(TemplateError::Capacity)
    ));
}

/// Detects missing output-ownership validation for every private output declaration.
#[test]
fn private_out_rejects_sources_multiple_outputs_and_prior_writers() {
    let mut builder = TemplateBuilder::new();
    let a = builder.cell("a", || Ok(1i32)).unwrap();
    let b = builder.cell("b", || Ok(0i32)).unwrap();
    let source = builder.source("source", || Ok(1i32)).unwrap();
    assert!(matches!(
        builder.out(copy_recipe(&a, &source)),
        Err(TemplateError::InvalidDefinition { .. })
    ));
    let tuple = MethodRecipe::new(
        std::slice::from_ref(&a),
        &[a.clone(), b.clone()],
        |_, _| unreachable!(),
    )
    .unwrap();
    assert!(matches!(
        builder.out(tuple),
        Err(TemplateError::InvalidDefinition { .. })
    ));
    builder.relationship(vec![copy_recipe(&a, &b)]).unwrap();
    assert!(matches!(
        builder.out(copy_recipe(&a, &b)),
        Err(TemplateError::InvalidDefinition { .. })
    ));
}

/// Detects an inclusion's read-only formal acquiring a private out actual after inclusion.
#[test]
fn private_out_argument_is_rejected_even_read_only() {
    let mut builder = TemplateBuilder::new();
    let a = builder.cell("a", || Ok(1i32)).unwrap();
    let b = builder.cell("b", || Ok(0i32)).unwrap();
    let c = builder.cell("c", || Ok(0i32)).unwrap();
    builder
        .include(Rc::new(identity()), &[("input", b.clone()), ("output", c)])
        .unwrap();
    builder.out(copy_recipe(&a, &b)).unwrap();
    assert!(matches!(
        builder.finish(),
        Err(TemplateError::InvalidDefinition { .. })
    ));
}

/// Detects accepting repeated, self-dependent, or foreign-input filters.
#[test]
fn private_policy_declarations_validate_ownership_and_uniqueness() {
    let mut builder = TemplateBuilder::new();
    let a = builder.cell("a", || Ok(0i32)).unwrap();
    let mut foreign = TemplateBuilder::new();
    let other = foreign.parameter::<i32>("other").unwrap();
    assert!(matches!(
        builder.filter(&a, std::slice::from_ref(&a), |_, _| unreachable!()),
        Err(TemplateError::InvalidDefinition { .. })
    ));
    assert!(matches!(
        builder.filter(&a, std::slice::from_ref(&other), |_, _| unreachable!()),
        Err(TemplateError::ForeignHandle { .. })
    ));
    assert!(matches!(
        builder.require(&a, &[other], |_| unreachable!()),
        Err(TemplateError::ForeignHandle { .. })
    ));
    builder
        .filter(&a, &[], |_, _| Ok(Filter::from_fn_0(|x: &i32| Ok(*x))))
        .unwrap();
    assert!(matches!(
        builder.filter(&a, &[], |_, _| unreachable!()),
        Err(TemplateError::InvalidDefinition { .. })
    ));
}

/// Detects private policy factories substituting undeclared edge lists or types.
#[test]
fn private_policy_factories_validate_signatures() {
    for case in 0..4 {
        let mut builder = TemplateBuilder::new();
        let a = builder.cell("a", || Ok(1i32)).unwrap();
        let b = builder.cell("b", || Ok(2i32)).unwrap();
        match case {
            0 => builder
                .filter(&a, &[], |_, _| Ok(Filter::from_fn_0(|x: &bool| Ok(*x))))
                .unwrap(),
            1 => builder
                .filter(&a, std::slice::from_ref(&b), |_, _| {
                    Ok(Filter::from_fn_0(|x: &i32| Ok(*x)))
                })
                .unwrap(),
            2 => builder
                .require(&a, std::slice::from_ref(&b), |i| {
                    Ok(Requirement::from_fn_1(i[0], |_: &bool| Ok(true)))
                })
                .unwrap(),
            _ => builder
                .require(&a, &[], |_| {
                    Ok(Requirement::from_fn_1(CellId::default(), |_: &i32| {
                        Ok(true)
                    }))
                })
                .unwrap(),
        }
        assert!(matches!(
            builder
                .finish()
                .unwrap()
                .instantiate(&mut Sheet::new(), &[]),
            Err(TemplateError::Construction { source, .. })
                if matches!(*source, TemplateError::InvalidDefinition { .. })
        ));
    }
}

/// Detects swallowed policy factory failures or changed native attachment timing.
#[test]
fn private_policy_failures_leave_destination_discardable() {
    for case in 0..3 {
        let mut builder = TemplateBuilder::new();
        let a = builder.cell("a", || Ok(1i32)).unwrap();
        match case {
            0 => builder
                .filter(&a, &[], |_, _| anyhow::bail!("filter factory"))
                .unwrap(),
            1 => builder
                .require(&a, &[], |_| anyhow::bail!("require factory"))
                .unwrap(),
            _ => builder
                .require(&a, std::slice::from_ref(&a), |i| {
                    Ok(Requirement::from_fn_1(i[0], |_: &i32| Ok(false)))
                })
                .unwrap(),
        }
        let mut sheet = Sheet::new();
        let error = builder
            .finish()
            .unwrap()
            .instantiate(&mut sheet, &[])
            .unwrap_err();
        let TemplateError::Construction {
            source,
            site,
            inclusions,
            ..
        } = error
        else {
            panic!("expected construction context");
        };
        assert!(inclusions.is_empty());
        assert!(
            matches!(site, adam_rs::template::ConstructionSite::Policy { cell, index: 0 } if cell == a)
        );
        if case < 2 {
            assert!(matches!(*source, TemplateError::Factory { .. }));
        } else {
            assert!(matches!(
                *source,
                TemplateError::Runtime(adam_rs::Error::InvalidRequirement)
            ));
        }
        drop(sheet);
    }
}

/// Detects malformed conditional owners, duplicate claims, and empty branch keys.
#[test]
fn conditional_declarations_validate_groups_before_registration() {
    let mut builder = TemplateBuilder::new();
    let guard = builder.parameter::<bool>("guard").unwrap();
    let empty = Rc::new(TemplateBuilder::new().finish().unwrap());
    let group = builder
        .include(empty.clone(), &[])
        .unwrap()
        .relationships()
        .to_vec();
    assert!(matches!(
        builder.conditional(
            std::slice::from_ref(&guard),
            |i| Ok(MatchExpr::cell(i[0])),
            vec![(Vec::<bool>::new(), group.clone())],
            vec![]
        ),
        Err(TemplateError::InvalidDefinition { .. })
    ));
    assert!(matches!(
        builder.conditional(
            std::slice::from_ref(&guard),
            |i| Ok(MatchExpr::cell(i[0])),
            vec![(vec![true], group.clone())],
            group.clone()
        ),
        Err(TemplateError::InvalidDefinition { .. })
    ));
    let foreign = TemplateBuilder::new()
        .include(empty, &[])
        .unwrap()
        .relationships()
        .to_vec();
    assert!(matches!(
        builder.conditional(
            std::slice::from_ref(&guard),
            |i| Ok(MatchExpr::cell(i[0])),
            vec![(vec![true], foreign)],
            vec![]
        ),
        Err(TemplateError::InvalidDefinition { .. })
    ));
    builder
        .conditional(
            std::slice::from_ref(&guard),
            |i| Ok(MatchExpr::cell(i[0])),
            vec![(vec![true], group.clone())],
            vec![],
        )
        .unwrap();
    assert!(matches!(
        builder.conditional(
            &[guard],
            |i| Ok(MatchExpr::cell(i[0])),
            vec![(vec![true], group)],
            vec![]
        ),
        Err(TemplateError::InvalidDefinition { .. })
    ));
}

/// Detects missing match-factory edge/type checks and lost conditional factory errors.
#[test]
fn conditional_factories_validate_signatures_and_errors() {
    for case in 0..3 {
        let mut builder = TemplateBuilder::new();
        let guard = builder.source("guard", || Ok(true)).unwrap();
        builder
            .conditional(
                &[guard],
                move |ids| match case {
                    0 => Ok(MatchExpr::cell(CellId::default())),
                    1 => Ok(MatchExpr::from_fn_1(ids[0], |_: &i32| Ok(true))),
                    _ => anyhow::bail!("conditional factory"),
                },
                vec![(vec![true], vec![])],
                vec![],
            )
            .unwrap();
        let error = builder
            .finish()
            .unwrap()
            .instantiate(&mut Sheet::new(), &[])
            .unwrap_err();
        let TemplateError::ConditionalConstruction {
            conditional,
            inclusions,
            source,
            ..
        } = error
        else {
            panic!("missing conditional construction context");
        };
        assert_eq!(conditional, 0);
        assert!(inclusions.is_empty());
        if case < 2 {
            assert!(matches!(*source, TemplateError::InvalidDefinition { .. }));
        } else {
            assert!(matches!(*source, TemplateError::ConditionalFactory { .. }));
        }
    }
}

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
    let calls = Rc::new(std::cell::Cell::new(0));
    let mut builder = TemplateBuilder::new();
    let factory_calls = calls.clone();
    builder
        .cell("private", move || {
            factory_calls.set(factory_calls.get() + 1);
            Ok(0_i32)
        })
        .unwrap();
    builder.parameter::<i32>("input").unwrap();
    builder.parameter::<i32>("output").unwrap();
    let template = builder.finish().unwrap();
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
    assert_eq!(calls.get(), 0);
    assert_eq!(sheet.cells().count(), 3);
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
            Err(TemplateError::Construction { source, .. })
                if matches!(*source, TemplateError::InvalidDefinition { .. })
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
        Err(TemplateError::Construction { source, .. })
            if matches!(*source, TemplateError::Factory { .. })
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
    let relationship = builder
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
    let TemplateError::Construction {
        source: error,
        site,
        inclusions,
        ..
    } = error
    else {
        panic!("expected method construction context");
    };
    assert!(matches!(
        site,
        adam_rs::template::ConstructionSite::Method { relationship: failed, index: 0 } if failed == relationship
    ));
    assert!(inclusions.is_empty());
    assert_eq!(
        std::error::Error::source(error.as_ref())
            .unwrap()
            .to_string(),
        "method factory failed"
    );
    match *error {
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

/// Detects missed empty constituents behind nested includes before private initialization.
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
        Err(TemplateError::Construction { source, .. })
            if matches!(*source, TemplateError::EmptyRelationship)
    ));
}
