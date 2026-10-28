//! Exercises the nonexecuting native substrate shared with language checking.

use adam_rs::{Method, MethodRecipe, TemplateBuilder, template::check_relationship_edges};

/// Enforces ordinary whole-relationship edges, without requiring any factories.
#[test]
fn pure_relationship_structure_has_valid_controls() {
    assert!(check_relationship_edges::<usize>(&[]).is_err());
    assert!(check_relationship_edges(&[(&[0][..], &[1][..]), (&[1][..], &[0][..])]).is_ok());
    assert!(check_relationship_edges(&[(&[0][..], &[1][..]), (&[2][..], &[0][..])]).is_err());
    assert!(check_relationship_edges(&[(&[0][..], &[1, 1][..])]).is_err());
    assert!(check_relationship_edges(&[(&[0][..], &[1][..]), (&[0][..], &[1][..])]).is_err());
}

/// Checks whole-method source specialization without a live sheet or factory execution.
#[test]
fn native_source_signature_check_does_not_execute_factories() {
    let mut builder = TemplateBuilder::new();
    let a = builder.parameter::<i32>("a").unwrap();
    let b = builder.parameter::<i32>("b").unwrap();
    builder
        .cell("private", || -> anyhow::Result<i32> {
            panic!("must not initialize")
        })
        .unwrap();
    builder
        .relationship(vec![
            MethodRecipe::new(
                std::slice::from_ref(&b),
                std::slice::from_ref(&a),
                |i, o| Ok(Method::from_fn_1_1(i[0], o[0], |x: &i32| Ok(*x))),
            )
            .unwrap(),
            MethodRecipe::new(&[a], &[b], |_, _| panic!("must not construct methods")).unwrap(),
        ])
        .unwrap();
    let template = builder.finish().unwrap();
    assert_eq!(template.parameter_names().collect::<Vec<_>>(), ["a", "b"]);
    assert!(
        template
            .check_source_bindings(&[("a", true), ("b", false)])
            .is_ok()
    );
    assert!(
        template
            .check_source_bindings(&[("a", false), ("unknown", false)])
            .is_err()
    );
    assert!(
        template
            .check_source_bindings(&[("a", true), ("b", true)])
            .is_err()
    );
    assert!(template.check_source_bindings(&[("a", false)]).is_err());
    assert!(
        template
            .check_source_bindings(&[("a", false), ("a", false)])
            .is_err()
    );
}
