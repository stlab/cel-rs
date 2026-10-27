//! Public contracts for source-independent expansion provenance.

use std::rc::Rc;

use adam_rs::{CellSlot, Method, MethodRecipe, Sheet, TemplateBuilder, TemplateError};

/// Creates an identity writer without capturing live execution state.
fn copy(input: &CellSlot, output: &CellSlot) -> MethodRecipe {
    MethodRecipe::new(
        std::slice::from_ref(input),
        std::slice::from_ref(output),
        |i, o| Ok(Method::from_fn_1_1(i[0], o[0], |x: &i32| Ok(*x))),
    )
    .unwrap()
}

/// Detects collapsed repeated occurrences, foreign mappings, or lost inclusion order.
#[test]
fn shared_dag_occurrences_retain_paths_and_owner_checked_mappings() {
    let mut leaf = TemplateBuilder::new();
    let formal = leaf.parameter::<i32>("p").unwrap();
    let private = leaf.cell("private", || Ok(0i32)).unwrap();
    let relation = leaf.relationship(vec![copy(&formal, &private)]).unwrap();
    let leaf = Rc::new(leaf.finish().unwrap());
    let leaf_id = leaf.id();
    let mut middle = TemplateBuilder::new();
    let formal_middle = middle.parameter::<i32>("p").unwrap();
    let inner = middle
        .include(leaf.clone(), &[("p", formal_middle.clone())])
        .unwrap();
    let middle = Rc::new(middle.finish().unwrap());
    let mut root = TemplateBuilder::new();
    let actual = root.cell("actual", || Ok(7i32)).unwrap();
    let a = root
        .include(middle.clone(), &[("p", actual.clone())])
        .unwrap();
    let b = root.include(middle, &[("p", actual.clone())]).unwrap();
    let root = root.finish().unwrap();
    let mut sheet = Sheet::new();
    let instance = root.instantiate(&mut sheet, &[]).unwrap();
    let other_instance = root.instantiate(&mut sheet, &[]).unwrap();
    drop(root);
    drop(leaf);
    let occurrences = instance.occurrences();
    assert_eq!(occurrences.len(), 5);
    assert!(occurrences[0].parent().is_none());
    assert!(occurrences[0].inclusion_site().is_none());
    assert_eq!(occurrences[1].inclusion_site(), Some(&a.relationships()[0]));
    assert_eq!(occurrences[3].inclusion_site(), Some(&b.relationships()[0]));
    assert_eq!(
        occurrences[2].inclusion_site(),
        Some(&inner.relationships()[0])
    );
    assert_eq!(occurrences[2].parent(), Some(occurrences[1].id()));
    assert_eq!(occurrences[4].parent(), Some(occurrences[3].id()));
    assert_eq!(occurrences[2].template(), &leaf_id);
    assert_eq!(occurrences[4].template(), &leaf_id);
    assert_ne!(occurrences[2].id(), occurrences[4].id());
    assert_eq!(
        instance.occurrence(occurrences[2].id()).unwrap().id(),
        occurrences[2].id()
    );
    assert!(
        instance
            .occurrence(other_instance.occurrences()[0].id())
            .is_none()
    );
    assert_eq!(
        instance.cell(occurrences[2].id(), &formal),
        Some(instance.cells()[0])
    );
    assert_eq!(
        instance.cell(occurrences[4].id(), &formal),
        Some(instance.cells()[0])
    );
    assert_ne!(
        instance.cell(occurrences[2].id(), &private),
        instance.cell(occurrences[4].id(), &private)
    );
    assert!(instance.cell(occurrences[0].id(), &private).is_none());
    assert!(
        instance
            .cell(other_instance.occurrences()[2].id(), &private)
            .is_none()
    );
    assert!(
        instance
            .relationship_group(occurrences[0].id(), &relation)
            .is_none()
    );
    assert_eq!(
        instance.relationship_group(occurrences[2].id(), &relation),
        Some(&instance.relationships()[0..1])
    );
    assert_eq!(
        instance.relationship_group(occurrences[0].id(), &a.relationships()[0]),
        Some(&instance.relationships()[0..1])
    );
    assert_eq!(
        instance.relationship_group(occurrences[0].id(), &b.relationships()[0]),
        Some(&instance.relationships()[1..2])
    );
    let origin = instance
        .method_origin(instance.relationships()[1], 0)
        .unwrap();
    assert_eq!(&origin.occurrence, occurrences[4].id());
    assert_eq!(origin.relationship, relation);
    assert_eq!(origin.authored_index, 0);
    assert!(
        instance
            .method_origin(instance.relationships()[1], 1)
            .is_none()
    );
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(instance.cells()[2]).unwrap(), 7);
}

/// Detects authored indices replaced by source-specialized compact method indices.
#[test]
fn surviving_methods_and_private_output_writers_keep_authored_indices() {
    let mut builder = TemplateBuilder::new();
    let a = builder.parameter::<i32>("a").unwrap();
    let b = builder.parameter::<i32>("b").unwrap();
    let relation = builder
        .relationship(vec![copy(&b, &a), copy(&a, &b)])
        .unwrap();
    let output = builder.cell("out", || Ok(0i32)).unwrap();
    let writer = builder.out(copy(&b, &output)).unwrap();
    let template = builder.finish().unwrap();
    let mut sheet = Sheet::new();
    let a_live = sheet.add_source(3i32);
    let b_live = sheet.add_cell(0i32);
    let instance = template
        .instantiate(&mut sheet, &[("a", a_live), ("b", b_live)])
        .unwrap();
    let origin = instance
        .method_origin(instance.relationships()[0], 0)
        .unwrap();
    assert_eq!(origin.authored_index, 1);
    assert_eq!(origin.relationship, relation);
    assert_eq!(&origin.occurrence, instance.occurrences()[0].id());
    let origin = instance
        .method_origin(instance.relationships()[1], 0)
        .unwrap();
    assert_eq!(origin.authored_index, 0);
    assert_eq!(origin.relationship, writer);
    sheet.propagate().unwrap();
    assert_eq!(sheet.selected_method(instance.relationships()[0]), Some(0));
    assert_eq!(*sheet.read::<i32>(instance.cells()[2]).unwrap(), 3);
    assert!(
        instance
            .method_origin(instance.relationships()[0], 1)
            .is_none()
    );
}

/// Detects unbudgeted repeated empty-child occurrences before source preflight.
#[test]
fn empty_child_occurrence_overflow_precedes_factories() {
    let mut child = Rc::new(TemplateBuilder::new().finish().unwrap());
    for _ in 0..usize::BITS {
        let mut parent = TemplateBuilder::new();
        parent.include(child.clone(), &[]).unwrap();
        parent.include(child, &[]).unwrap();
        child = Rc::new(parent.finish().unwrap());
    }
    let mut root = TemplateBuilder::new();
    root.cell::<i32, _>("never", || {
        panic!("capacity preflight must precede initialization")
    })
    .unwrap();
    root.include(child, &[]).unwrap();
    let mut sheet = Sheet::new();
    assert!(matches!(
        root.finish().unwrap().instantiate(&mut sheet, &[]),
        Err(TemplateError::Capacity)
    ));
}

/// Detects pointer identity hashes inconsistent with local handle equality.
#[test]
fn cell_slot_hash_matches_owner_and_local_identity() {
    let mut a = TemplateBuilder::new();
    let first = a.parameter::<i32>("first").unwrap();
    let second = a.parameter::<i32>("second").unwrap();
    let mut b = TemplateBuilder::new();
    let foreign = b.parameter::<i32>("first").unwrap();
    let mut slots = std::collections::HashSet::new();
    slots.insert(first.clone());
    slots.insert(first);
    slots.insert(second);
    slots.insert(foreign);
    assert_eq!(slots.len(), 3);
}

/// Detects recycled template or occurrence identities after their owning graph is dropped.
#[test]
fn retained_identities_hash_consistently_without_retaining_recipe_edges() {
    let captured = Rc::new(1i32);
    let lifetime = Rc::downgrade(&captured);
    let mut builder = TemplateBuilder::new();
    builder.cell("private", move || Ok(*captured)).unwrap();
    let template = builder.finish().unwrap();
    let template_id = template.id();
    let instance = template.instantiate(&mut Sheet::new(), &[]).unwrap();
    let occurrence = instance.occurrences()[0].id().clone();
    let mut templates = std::collections::HashSet::new();
    templates.insert(template_id.clone());
    templates.insert(template.id());
    let mut occurrences = std::collections::HashSet::new();
    occurrences.insert(occurrence.clone());
    occurrences.insert(instance.occurrences()[0].id().clone());
    assert_eq!(templates.len(), 1);
    assert_eq!(occurrences.len(), 1);
    drop(template);
    assert!(lifetime.upgrade().is_none());
    drop(instance);
    let template = TemplateBuilder::new().finish().unwrap();
    let instance = template.instantiate(&mut Sheet::new(), &[]).unwrap();
    assert_ne!(template.id(), template_id);
    assert_ne!(instance.occurrences()[0].id(), &occurrence);
    assert!(instance.occurrence(&occurrence).is_none());
    templates.insert(template.id());
    occurrences.insert(instance.occurrences()[0].id().clone());
    assert_eq!(templates.len(), 2);
    assert_eq!(occurrences.len(), 2);
}

/// Detects omitted empty occurrences or empty groups confused with foreign handles.
#[test]
fn repeated_empty_children_still_have_distinct_ordered_occurrences() {
    let empty = Rc::new(TemplateBuilder::new().finish().unwrap());
    let mut builder = TemplateBuilder::new();
    let first = builder.include(empty.clone(), &[]).unwrap();
    let second = builder.include(empty, &[]).unwrap();
    let instance = builder
        .finish()
        .unwrap()
        .instantiate(&mut Sheet::new(), &[])
        .unwrap();
    let occurrences = instance.occurrences();
    assert_eq!(occurrences.len(), 3);
    assert_eq!(occurrences[1].parent(), Some(occurrences[0].id()));
    assert_eq!(occurrences[2].parent(), Some(occurrences[0].id()));
    assert_eq!(occurrences[1].template(), occurrences[2].template());
    assert_ne!(occurrences[1].id(), occurrences[2].id());
    assert_eq!(
        occurrences[1].inclusion_site(),
        Some(&first.relationships()[0])
    );
    assert_eq!(
        occurrences[2].inclusion_site(),
        Some(&second.relationships()[0])
    );
    for inclusion in [first, second] {
        assert_eq!(
            instance.relationship_group(occurrences[0].id(), &inclusion.relationships()[0]),
            Some(&[][..])
        );
    }
    assert!(instance.cells().is_empty());
    assert!(instance.relationships().is_empty());
}
