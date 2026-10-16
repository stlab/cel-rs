//! Verifies closure invocation with standard operations and registered Adam geometry.
//!
//! Install the standard and Euclid operations before parsing an immediately invoked
//! closure. Register Adam geometry types to resolve its parameter annotations.
#![cfg(feature = "adam")]

/// Invokes a scalar closure without a host call scope.
#[test]
fn immediately_invoked_scalar_closure_needs_no_custom_scope() {
    let mut lookup = cel_parser::OpLookup::new();
    cel_std::install(&mut lookup);
    cel_euclid::install(&mut lookup);
    let mut parser = cel_parser::CELParser::new(lookup);
    let mut segment = parser.parse_str("(|x: f64| x)(1.0)").unwrap();
    assert_eq!(segment.call0::<f64>().unwrap(), 1.0);
}

/// Resolves registered geometry annotations in Adam closures.
#[test]
fn geometry_closure_parameter_uses_registered_adam_type() {
    let mut registry = adam_lang::TypeRegistry::new();
    cel_euclid::register_adam_types(&mut registry);
    let mut lookup = cel_parser::OpLookup::new();
    cel_std::install(&mut lookup);
    cel_euclid::install(&mut lookup);
    let mut parser = adam_lang::AdamParser::new(registry, lookup);
    // An uninvoked closure has no registered cell type in this environment.
    // Invocation exposes its registered result and exercises annotation resolution.
    parser
        .parse_str("sheet s { out r: Rect := (|r: Rect| r)(Rect { origin: Point2D { x: 0.0, y: 0.0 }, size: Size2D { width: 1.0, height: 1.0 } }); }")
        .unwrap_or_else(|error| panic!("{}", error.message()));
}
