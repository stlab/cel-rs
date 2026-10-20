//! Named record adapters preserve source evaluation order before mapping fields.
//!
//! Each adapter packs once in source order and relabels to a checked typed
//! cons-list. Only the final typed constructor rearranges values by field name.

use crate::{Angle, Box2D, Point2D, Rect, Size2D, Transform2D, Vector2D, leaf};
use anyhow::Result;
use cel_parser::{OpLookup, RecordField, RecordSchema};
use cel_runtime::{Cons, IntoRustTuple, Leaf, Nil};

/// Holds two scalars in their evaluated source order.
type Pair = Cons<Leaf<f64>, Cons<Leaf<f64>, Nil>>;
/// Holds six scalars in their evaluated source order.
type Six = Cons<Leaf<f64>, Cons<Leaf<f64>, Cons<Leaf<f64>, Cons<Leaf<f64>, Pair>>>>;
/// Holds source-order origin then size without erasing their distinct types.
type OriginSize = Cons<Leaf<Point2D>, Cons<Leaf<Size2D>, Nil>>;
/// Holds source-order size then origin without erasing their distinct types.
type SizeOrigin = Cons<Leaf<Size2D>, Cons<Leaf<Point2D>, Nil>>;
/// Holds two source-order points.
type Points = Cons<Leaf<Point2D>, Cons<Leaf<Point2D>, Nil>>;

/// Maps evaluated source positions to canonical schema positions.
///
/// - Precondition: `mapping` is a permutation of `0..N`.
/// - Complexity: O(N) time and space.
fn ordered<const N: usize>(source: [f64; N], mapping: [usize; N]) -> [f64; N] {
    debug_assert!(mapping.iter().all(|&index| index < N));
    debug_assert!((0..N).all(|index| mapping.contains(&index)));
    let mut canonical = [0.0; N];
    for (value, index) in source.into_iter().zip(mapping) {
        canonical[index] = value;
    }
    canonical
}

/// Registers a homogeneous two-scalar schema with a source-order typed adapter.
///
/// - Complexity: O(r) in existing schemas.
///
/// # Errors
/// Returns registration errors for a duplicate schema.
fn scalar_pair<T: 'static>(
    lookup: &mut OpLookup,
    name: &'static str,
    fields: [&'static str; 2],
    build: fn(f64, f64) -> T,
) -> Result<()> {
    lookup.register_record(RecordSchema {
        ty: leaf::<T>(name),
        fields: fields
            .into_iter()
            .map(|name| RecordField {
                name,
                ty: leaf::<f64>("f64").into(),
            })
            .collect(),
        emit: Box::new(move |s, mapping, ambient, _| {
            let mapping: [usize; 2] = mapping
                .try_into()
                .expect("validated two-field scalar schema");
            s.make_tuple(2, ambient)?;
            s.op1_repr::<Pair, Leaf<T>, _>(move |value| {
                let (a, b) = value.into_rust_tuple();
                let [a, b] = ordered([a, b], mapping);
                build(a, b)
            })
        }),
    })
}

/// Registers all seven exact field schemas independently of value bindings.
///
/// - Complexity: O(r) in existing schemas; field counts are bounded by six.
///
/// # Errors
/// Returns duplicate-schema registration errors.
pub(crate) fn install(lookup: &mut OpLookup) -> Result<()> {
    scalar_pair(lookup, "Point2D", ["x", "y"], Point2D::new)?;
    scalar_pair(lookup, "Vector2D", ["x", "y"], Vector2D::new)?;
    scalar_pair(lookup, "Size2D", ["width", "height"], Size2D::new)?;
    lookup.register_record(RecordSchema {
        ty: leaf::<Rect>("Rect"),
        fields: vec![
            RecordField {
                name: "origin",
                ty: leaf::<Point2D>("Point2D").into(),
            },
            RecordField {
                name: "size",
                ty: leaf::<Size2D>("Size2D").into(),
            },
        ],
        emit: Box::new(|s, mapping, ambient, _| {
            debug_assert!(matches!(mapping, [0, 1] | [1, 0]));
            s.make_tuple(2, ambient)?;
            if mapping == [0, 1] {
                s.op1_repr::<OriginSize, Leaf<Rect>, _>(|value| {
                    let (origin, size) = value.into_rust_tuple();
                    Rect::new(origin, size)
                })
            } else {
                s.op1_repr::<SizeOrigin, Leaf<Rect>, _>(|value| {
                    let (size, origin) = value.into_rust_tuple();
                    Rect::new(origin, size)
                })
            }
        }),
    })?;
    lookup.register_record(RecordSchema {
        ty: leaf::<Box2D>("Box2D"),
        fields: vec![
            RecordField {
                name: "min",
                ty: leaf::<Point2D>("Point2D").into(),
            },
            RecordField {
                name: "max",
                ty: leaf::<Point2D>("Point2D").into(),
            },
        ],
        emit: Box::new(|s, mapping, ambient, _| {
            debug_assert!(matches!(mapping, [0, 1] | [1, 0]));
            let reversed = mapping == [1, 0];
            s.make_tuple(2, ambient)?;
            s.op1_repr::<Points, Leaf<Box2D>, _>(move |value| {
                let (a, b) = value.into_rust_tuple();
                if reversed {
                    Box2D::new(b, a)
                } else {
                    Box2D::new(a, b)
                }
            })
        }),
    })?;
    lookup.register_record(RecordSchema {
        ty: leaf::<Angle>("Angle"),
        fields: vec![RecordField {
            name: "radians",
            ty: leaf::<f64>("f64").into(),
        }],
        emit: Box::new(|s, _, ambient, _| {
            s.make_tuple(1, ambient)?;
            s.op1_repr::<Cons<Leaf<f64>, Nil>, Leaf<Angle>, _>(|value| {
                let (radians,) = value.into_rust_tuple();
                Angle::radians(radians)
            })
        }),
    })?;
    lookup.register_record(RecordSchema {
        ty: leaf::<Transform2D>("Transform2D"),
        fields: ["m11", "m12", "m21", "m22", "m31", "m32"]
            .into_iter()
            .map(|name| RecordField {
                name,
                ty: leaf::<f64>("f64").into(),
            })
            .collect(),
        emit: Box::new(|s, mapping, ambient, _| {
            let mapping: [usize; 6] = mapping
                .try_into()
                .expect("validated six-field matrix schema");
            s.make_tuple(6, ambient)?;
            s.op1_repr::<Six, Leaf<Transform2D>, _>(move |value| {
                let (a, b, c, d, e, f) = value.into_rust_tuple();
                let [a, b, c, d, e, f] = ordered([a, b, c, d, e, f], mapping);
                Transform2D::new(a, b, c, d, e, f)
            })
        }),
    })?;
    Ok(())
}
