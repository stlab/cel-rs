//! Examples backing `book-src/relationships.md` (Chapter 7). See `src/lib.rs` for how these
//! `.adm2` files are wired into the book.

#[test]
fn shared_cell_example() {
    let mut parser = adam_lang_book::support::parser();
    let mut parsed = parser
        .parse_str(include_str!(
            "../book-src/examples/relationships/shared_cell_example.adm2"
        ))
        .unwrap();
    parsed.propagate().unwrap();

    let (a, b, c, d) = (
        parsed.cell_names["a"].0,
        parsed.cell_names["b"].0,
        parsed.cell_names["c"].0,
        parsed.cell_names["d"].0,
    );
    assert!(parsed.is_source(c) && parsed.is_source(d));
    assert!(!parsed.is_source(a) && !parsed.is_source(b));
    assert_eq!(*parsed.read::<f64>(c).unwrap(), 2.0); // untouched
    assert_eq!(*parsed.read::<f64>(d).unwrap(), 3.0); // untouched
    assert_eq!(*parsed.read::<f64>(b).unwrap(), 1.5); // d / c
    assert_eq!(*parsed.read::<f64>(a).unwrap(), 4.0 / 3.0); // c / b
}

/// Detects missing inline component expansion and source-bound reverse specialization.
#[test]
fn inline_component() {
    let mut parsed = adam_lang_book::support::parser()
        .parse_str(include_str!(
            "../book-src/examples/relationships/inline_component.adm2"
        ))
        .unwrap();
    let value = parsed.cell_names["value"].0;
    let result = parsed.cell_names["result"].0;
    assert_eq!(*parsed.read::<f64>(result).unwrap(), 0.0);
    parsed.sheet.write(value, 2.0f64).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(result).unwrap(), 6.0);
    parsed.sheet.write(result, 12.0f64).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(value).unwrap(), 4.0);
}

// ANCHOR: imported_components
/// Compiles the shipped modules, selects a root, and retains execution after owner drop.
#[test]
fn imported_components() {
    use adam_lang::{ModuleCompiler, ModuleSet, TypeRegistry};
    let mut modules = ModuleSet::new();
    modules
        .insert_source(
            "relationships".parse().unwrap(),
            "relationships.adm2",
            include_str!("../../adam-lang/examples/relationships.adm2"),
        )
        .unwrap();
    modules
        .insert_source(
            "composed".parse().unwrap(),
            "composed.adm2",
            include_str!("../../adam-lang/examples/composed.adm2"),
        )
        .unwrap();
    // These scalar examples need only builtin CEL operations.
    let compiled = ModuleCompiler::new(TypeRegistry::new(), cel_parser::OpLookup::new())
        .compile(modules)
        .unwrap();
    let path = "composed".parse().unwrap();
    assert_eq!(
        compiled.roots(&path).collect::<Vec<_>>(),
        ["ordered_demo", "source_specialized", "repeated_products"]
    );
    assert!(
        compiled
            .roots(&"relationships".parse().unwrap())
            .next()
            .is_none()
    );
    assert!(compiled.instantiate_root(&path, "multiply").is_err());
    let mut parsed = compiled
        .instantiate_root(&path, "source_specialized")
        .unwrap();
    drop(compiled);
    let product = parsed.cell_names["product"].0;
    let value = parsed.cell_names["value"].0;
    parsed.sheet.write(value, 7.0f64).unwrap();
    parsed.propagate().unwrap();
    assert_eq!(*parsed.read::<f64>(product).unwrap(), 21.0);
}
// ANCHOR_END: imported_components
