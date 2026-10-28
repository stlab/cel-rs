//! Verifies bulk provenance retains compact and authored method ordering.

use adam_rs::{Method, MethodRecipe, Sheet, TemplateBuilder};

#[test]
/// Checks bulk and checked method-origin agreement after template specialization.
fn bulk_method_origins_match_checked_lookup_after_specialization() {
    let mut builder = TemplateBuilder::new();
    let a = builder.parameter::<i32>("a").unwrap();
    let b = builder.parameter::<i32>("b").unwrap();
    builder
        .relationship(vec![
            MethodRecipe::new(
                std::slice::from_ref(&a),
                std::slice::from_ref(&b),
                |i, o| Ok(Method::from_fn_1_1(i[0], o[0], |x: &i32| Ok(*x))),
            )
            .unwrap(),
            MethodRecipe::new(&[b], &[a], |i, o| {
                Ok(Method::from_fn_1_1(i[0], o[0], |x: &i32| Ok(*x)))
            })
            .unwrap(),
        ])
        .unwrap();
    let template = builder.finish().unwrap();
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0i32);
    let b = sheet.add_source(1i32);
    let instance = template
        .instantiate(&mut sheet, &[("a", a), ("b", b)])
        .unwrap();
    let origins: Vec<_> = instance.method_origins().collect();
    assert_eq!(origins.len(), 1);
    let (relationship, live, origin) = &origins[0];
    assert_eq!(*live, 0);
    assert_eq!(origin.authored_index, 1);
    assert_eq!(
        instance.method_origin(*relationship, *live),
        Some(origin.clone())
    );
}
