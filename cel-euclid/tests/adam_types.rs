//! Contracts for optional Adam type registration and Euclid defaults.

#![cfg(feature = "adam")]

use cel_euclid::{Angle, Box2D, Point2D, Rect, Size2D, Transform2D, Vector2D, register_adam_types};

/// Checks exact names, identities and the explicit upstream-defined default of every alias.
#[test]
fn every_alias_has_its_euclid_default() {
    let mut registry = adam_lang::TypeRegistry::new();
    register_adam_types(&mut registry);
    macro_rules! check {
        ($ty:ty, $expected:expr) => {{
            let entry = registry.get(stringify!($ty)).unwrap();
            assert_eq!(entry.type_id, std::any::TypeId::of::<$ty>());
            let value = entry.default_fn.expect("Euclid implements Default")();
            assert_eq!(*value.downcast::<$ty>().unwrap(), $expected);
        }};
    }
    check!(Point2D, Point2D::new(0.0, 0.0));
    check!(Vector2D, Vector2D::new(0.0, 0.0));
    check!(Size2D, Size2D::new(0.0, 0.0));
    check!(Rect, Rect::new(Point2D::zero(), Size2D::zero()));
    check!(Box2D, Box2D::new(Point2D::zero(), Point2D::zero()));
    check!(Angle, Angle::radians(0.0));
    check!(Transform2D, Transform2D::identity());
}

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
