//! Optional Adam registration uses the same concrete aliases as CEL.
//!
//! Every alias in Euclid 0.22.14 implements `Default`: coordinates, sizes,
//! bounds and angles default to zero; transforms default to identity.

use crate::{Angle, Box2D, Point2D, Rect, Size2D, Transform2D, Vector2D};

/// Registers the seven exact geometry names with their Euclid defaults.
///
/// Registers zero points, vectors, sizes, rectangles, boxes and angles, and an
/// identity transform. Existing entries under these names follow
/// [`adam_lang::TypeRegistry::register`]'s replacement semantics.
///
/// - Complexity: O(1) amortized time and space for seven registrations.
///
/// # Examples
/// ```
/// let mut registry = adam_lang::TypeRegistry::new();
/// cel_euclid::register_adam_types(&mut registry);
/// assert!(registry.get("Point2D").unwrap().default_fn.is_some());
/// ```
pub fn register_adam_types(registry: &mut adam_lang::TypeRegistry) {
    registry.register::<Point2D>("Point2D");
    registry.register::<Vector2D>("Vector2D");
    registry.register::<Size2D>("Size2D");
    registry.register::<Rect>("Rect");
    registry.register::<Box2D>("Box2D");
    registry.register::<Angle>("Angle");
    registry.register::<Transform2D>("Transform2D");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registers every alias with its exact identity and upstream-defined default.
    #[test]
    fn every_alias_has_its_euclid_default() {
        let mut registry = adam_lang::TypeRegistry::new();
        register_adam_types(&mut registry);
        /// Checks one registration's concrete identity and independently specified default.
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
}
