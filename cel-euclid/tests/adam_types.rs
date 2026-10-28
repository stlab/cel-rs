//! Contracts for optional Adam type registration and Euclid defaults.

#![cfg(feature = "adam")]

use cel_euclid::{Point2D, Rect, Size2D, register_adam_types};

/// Adam's existing registry resolver supplies geometry annotations to ordinary CEL calls.
#[test]
fn registered_geometry_closure_uses_adam_type_environment() {
    let mut registry = adam_lang::TypeRegistry::new();
    register_adam_types(&mut registry);
    let mut lookup = cel_parser::OpLookup::new();
    cel_std::install(&mut lookup);
    cel_euclid::install(&mut lookup);
    let mut sheet = adam_lang::AdamParser::new(registry, lookup)
        .parse_str(
            "sheet s {
            out result := (|r: Rect| r)(Rect {
                origin: Point2D { x: 2.0, y: 3.0 },
                size: Size2D { width: 4.0, height: 5.0 }
            });
        }",
        )
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(
        *sheet.read::<Rect>(sheet.cell_names["result"].0).unwrap(),
        Rect::new(Point2D::new(2.0, 3.0), Size2D::new(4.0, 5.0)),
    );
}
