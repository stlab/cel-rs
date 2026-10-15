//! Public contracts for the opt-in geometry library.

use cel_euclid::{Angle, Box2D, Point2D, Rect, Size2D, Transform2D, Vector2D, install};
use cel_parser::{CELParser, OpLookup};
use std::fmt::Debug;

const P: &str = "Point2D { x: 1.0, y: 2.0 }";
const Q: &str = "Point2D { x: 4.0, y: 6.0 }";
const V: &str = "Vector2D { x: 3.0, y: 4.0 }";
const S: &str = "Size2D { width: 4.0, height: 6.0 }";
const R: &str =
    "Rect { origin: Point2D { x: 1.0, y: 2.0 }, size: Size2D { width: 4.0, height: 6.0 } }";
const B: &str = "Box2D { min: Point2D { x: 1.0, y: 2.0 }, max: Point2D { x: 5.0, y: 8.0 } }";
const A: &str = "Angle { radians: 1.5 }";
const T: &str = "Transform2D { m11: 1.0, m12: 2.0, m21: 3.0, m22: 4.0, m31: 5.0, m32: 6.0 }";

/// Creates an independently installed geometry parser.
fn parser() -> CELParser {
    let mut lookup = OpLookup::new();
    install(&mut lookup);
    CELParser::new(lookup)
}

/// Checks one exact result through parsed CEL.
fn check<T: 'static + PartialEq + Debug>(source: &str, expected: T) {
    let actual = parser()
        .parse_str(source)
        .unwrap_or_else(|e| panic!("{source}: {e}"))
        .call0::<T>()
        .unwrap();
    assert_eq!(actual, expected, "{source}");
}

/// Checks ordinary and receiver dispatch and rejects wrong receiver/argument types.
fn named<T: 'static + PartialEq + Debug + Clone>(name: &str, args: &[&str], expected: T) {
    check(&format!("{name}({})", args.join(", ")), expected.clone());
    let tail = &args[1..];
    check(
        &format!("({}).{name}({})", args[0], tail.join(", ")),
        expected,
    );
    for index in 0..args.len() {
        let mut wrong = args.to_vec();
        wrong[index] = "true";
        assert!(
            parser()
                .parse_str(&format!("{name}({})", wrong.join(", ")))
                .is_err(),
            "{name}: operand {index}"
        );
        assert!(
            parser()
                .parse_str(&format!("({}).{name}({})", wrong[0], wrong[1..].join(", ")))
                .is_err(),
            "{name}: receiver operand {index}"
        );
    }
}

/// Checks all seven schemas, preserving named rather than positional field identity.
#[test]
fn named_records_match_euclid() {
    check(P, Point2D::new(1.0, 2.0));
    check(V, Vector2D::new(3.0, 4.0));
    check(S, Size2D::new(4.0, 6.0));
    check(R, Rect::new(Point2D::new(1.0, 2.0), Size2D::new(4.0, 6.0)));
    check(
        B,
        Box2D::new(Point2D::new(1.0, 2.0), Point2D::new(5.0, 8.0)),
    );
    check(A, Angle::radians(1.5));
    check(T, Transform2D::new(1.0, 2.0, 3.0, 4.0, 5.0, 6.0));
    check("Point2D { y: 42.0, x: 0.0 }.x", 0.0);
    check("Vector2D { y: 42.0, x: 0.0 }.y", 42.0);
    check("Size2D { height: 42.0, width: 0.0 }.width", 0.0);
    check(
        "Box2D { max: Point2D { x: 5.0, y: 8.0 }, min: Point2D { y: 2.0, x: 1.0 } }",
        Box2D::new(Point2D::new(1.0, 2.0), Point2D::new(5.0, 8.0)),
    );
}

/// Checks both heterogeneous Rect layouts above differently aligned ambient tuple values.
#[test]
fn reversed_rect_fields_keep_types() {
    let reverse = format!("Rect {{ size: {S}, origin: {P} }}");
    let expected = Rect::new(Point2D::new(1.0, 2.0), Size2D::new(4.0, 6.0));
    for rect in [R, reverse.as_str()] {
        check(rect, expected);
        check(&format!("(7u8, \"owned\", {rect}).2"), expected);
        check(&format!("({rect}, 7u8).0"), expected);
    }
    for invalid in [
        format!("Rect {{ origin: {S}, size: {P} }}"),
        "Point2D { x: 1.0 }".into(),
        "Point2D { x: 1.0, x: 2.0, y: 3.0 }".into(),
        "Point2D { x: 1.0, z: 2.0 }".into(),
    ] {
        assert!(parser().parse_str(&invalid).is_err(), "{invalid}");
    }
}

/// Checks the exact arithmetic table and rejects unsupported operand pairs.
#[test]
fn vector_and_point_arithmetic() {
    check(&format!("{P} + {V}"), Point2D::new(4.0, 6.0));
    check(&format!("{V} + {V}"), Vector2D::new(6.0, 8.0));
    check(&format!("{S} + {S}"), Size2D::new(8.0, 12.0));
    check(&format!("{Q} - {P}"), Vector2D::new(3.0, 4.0));
    check(&format!("{Q} - {V}"), Point2D::new(1.0, 2.0));
    check(&format!("{V} - {V}"), Vector2D::zero());
    check(&format!("{S} - {S}"), Size2D::zero());
    check(&format!("-({V})"), Vector2D::new(-3.0, -4.0));
    check(&format!("{V} * 2.0"), Vector2D::new(6.0, 8.0));
    check(&format!("{V} / 2.0"), Vector2D::new(1.5, 2.0));
    check(&format!("{S} * 2.0"), Size2D::new(8.0, 12.0));
    check(&format!("{S} / 2.0"), Size2D::new(2.0, 3.0));
    for source in [
        format!("{P} + {P}"),
        format!("{V} + {P}"),
        format!("{P} * 2.0"),
        format!("2.0 * {V}"),
        format!("{V} / 2"),
        format!("-({S})"),
        format!("{V} - {P}"),
        format!("{S} + {V}"),
    ] {
        assert!(parser().parse_str(&source).is_err(), "{source}");
    }
    check(&format!("({V} / 0.0).is_finite"), false);
}

/// Checks every non-transform accessor and vector query through both dispatch forms.
#[test]
fn accessors_and_vector_queries() {
    for value in [P, V] {
        named("x", &[value], if value == P { 1.0 } else { 3.0 });
        named("y", &[value], if value == P { 2.0 } else { 4.0 });
    }
    for value in [S, R, B] {
        named("width", &[value], 4.0);
        named("height", &[value], 6.0);
    }
    named("origin", &[R], Point2D::new(1.0, 2.0));
    for value in [R, B] {
        named("size", &[value], Size2D::new(4.0, 6.0));
        named("min_x", &[value], 1.0);
        named("min_y", &[value], 2.0);
        named("max_x", &[value], 5.0);
        named("max_y", &[value], 8.0);
        named("center", &[value], Point2D::new(3.0, 5.0));
    }
    named("min", &[B], Point2D::new(1.0, 2.0));
    named("max", &[B], Point2D::new(5.0, 8.0));
    named("dot", &[V, V], 25.0);
    named("cross", &[V, "Vector2D { x: 1.0, y: 2.0 }"], 2.0);
    named("square_length", &[V], 25.0);
    named("length", &[V], 5.0);
    named("distance_to", &[P, Q], 5.0);
    named("normalize", &[V], Vector2D::new(0.6, 0.8));
}

/// Checks translated, inflated and converted geometry against literal coordinates.
#[test]
fn bounds_operations() {
    named(
        "translate",
        &[R, V],
        Rect::new(Point2D::new(4.0, 6.0), Size2D::new(4.0, 6.0)),
    );
    named(
        "translate",
        &[B, V],
        Box2D::new(Point2D::new(4.0, 6.0), Point2D::new(8.0, 12.0)),
    );
    named(
        "inflate",
        &[R, "1.0", "2.0"],
        Rect::new(Point2D::new(0.0, 0.0), Size2D::new(6.0, 10.0)),
    );
    named(
        "inflate",
        &[B, "1.0", "2.0"],
        Box2D::new(Point2D::new(0.0, 0.0), Point2D::new(6.0, 10.0)),
    );
    named(
        "to_box2d",
        &[R],
        Box2D::new(Point2D::new(1.0, 2.0), Point2D::new(5.0, 8.0)),
    );
    named(
        "to_rect",
        &[B],
        Rect::new(Point2D::new(1.0, 2.0), Size2D::new(4.0, 6.0)),
    );
    named("contains_rect", &[R, R], true);
    named("contains_box", &[B, B], true);
    named("intersects", &[R, R], true);
    named("intersects", &[B, B], true);
    let shifted_r = format!("({R}).translate({V})");
    let shifted_b = format!("({B}).translate({V})");
    named(
        "union",
        &[R, &shifted_r],
        Rect::new(Point2D::new(1.0, 2.0), Size2D::new(7.0, 10.0)),
    );
    named(
        "union",
        &[B, &shifted_b],
        Box2D::new(Point2D::new(1.0, 2.0), Point2D::new(8.0, 12.0)),
    );
}

/// Preserves Euclid's half-open boundary and empty-bound containment semantics.
#[test]
fn ordinary_containment_matches_euclid() {
    for bound in [R, B] {
        named("contains", &[bound, P], true);
        named("contains", &[bound, Q], true);
        named("contains", &[bound, "Point2D { x: 5.0, y: 8.0 }"], false);
        named("contains", &[bound, "Point2D { x: 0.0, y: 2.0 }"], false);
    }
    for bound in [
        "Rect { origin: Point2D { x: 1.0, y: 2.0 }, size: Size2D { width: 0.0, height: 6.0 } }",
        "Box2D { min: Point2D { x: 1.0, y: 2.0 }, max: Point2D { x: 1.0, y: 8.0 } }",
    ] {
        named("contains", &[bound, P], false);
        named("intersects", &[bound, bound], false);
    }
}

/// Preserves all seven exact equality identities and six currently supported finite predicates.
#[test]
fn equality_and_finite_predicates() {
    for value in [P, V, S, R, B, A, T] {
        check(&format!("{value} == {value}"), true);
        check(&format!("{value} != {value}"), false);
        assert!(parser().parse_str(&format!("{value} == true")).is_err());
    }
    for (a, b) in [
        (P, Q),
        (V, "Vector2D { x: 1.0, y: 4.0 }"),
        (S, "Size2D { width: 4.0, height: 7.0 }"),
        (
            R,
            "Rect { origin: Point2D { x: 2.0, y: 2.0 }, size: Size2D { width: 4.0, height: 6.0 } }",
        ),
        (
            B,
            "Box2D { min: Point2D { x: 0.0, y: 2.0 }, max: Point2D { x: 5.0, y: 8.0 } }",
        ),
        (A, "Angle { radians: 2.5 }"),
        (
            T,
            "Transform2D { m11: 1.0, m12: 2.0, m21: 3.0, m22: 4.0, m31: 5.0, m32: 7.0 }",
        ),
    ] {
        check(&format!("{a} == {b}"), false);
        check(&format!("{a} != {b}"), true);
        assert!(parser().parse_str(&format!("{a} != true")).is_err());
    }
    for value in [P, V, S, R, B, A] {
        named("is_finite", &[value], true);
    }
    for value in [
        "Point2D { x: 1.0 / 0.0, y: 0.0 }",
        "Vector2D { x: 0.0, y: 0.0 / 0.0 }",
        "Size2D { width: 1.0 / 0.0, height: 0.0 }",
        "Angle { radians: 0.0 / 0.0 }",
        "Rect { origin: Point2D { x: 1.0 / 0.0, y: 0.0 }, size: Size2D { width: 1.0, height: 1.0 } }",
        "Box2D { min: Point2D { x: 0.0, y: 0.0 }, max: Point2D { x: 1.0 / 0.0, y: 1.0 } }",
    ] {
        named("is_finite", &[value], false);
    }
    // Each stored scalar must participate, not just the first scalar in each record.
    for value in [P, V, S, R, B, A] {
        for scalar in ["1.0", "2.0", "3.0", "4.0", "5.0", "6.0", "8.0", "1.5"] {
            if value.contains(scalar) {
                let nonfinite = value.replacen(scalar, "(0.0 / 0.0)", 1);
                named("is_finite", &[&nonfinite], false);
            }
        }
    }
}

/// Rejects zero, underflowed, overflowing and non-finite normalization lengths at execution.
#[test]
fn checked_normalization_rejects_invalid_lengths() {
    for vector in [
        "Vector2D { x: 0.0, y: 0.0 }",
        "Vector2D { x: 1e-300, y: 0.0 }",
        "Vector2D { x: 1e300, y: 0.0 }",
        "Vector2D { x: 1.0 / 0.0, y: 0.0 }",
        "Vector2D { x: 0.0 / 0.0, y: 0.0 }",
    ] {
        for source in [
            format!("normalize({vector})"),
            format!("({vector}).normalize()"),
        ] {
            let mut segment = parser().parse_str(&source).unwrap();
            for _ in 0..2 {
                let error = segment.call_dyn::<Vector2D>(&[]).unwrap_err();
                assert!(error.to_string().contains("normalize"), "{source}: {error}");
            }
        }
    }
}

/// Composes unary box extrema with standard scalar extrema independently of install order.
#[test]
fn box_min_extends_scalar_min() {
    for geometry_first in [false, true] {
        let mut lookup = OpLookup::new();
        if geometry_first {
            install(&mut lookup);
        }
        cel_std::install(&mut lookup);
        if !geometry_first {
            install(&mut lookup);
        }
        let mut parser = CELParser::new(lookup);
        for (source, want) in [
            ("min(3.0, 7.0)".to_owned(), 3.0),
            ("max(3.0, 7.0)".to_owned(), 7.0),
            (format!("min({B}).x"), 1.0),
            (format!("max({B}).y"), 8.0),
            (format!("({B}).min.x"), 1.0),
        ] {
            assert_eq!(
                parser.parse_str(&source).unwrap().call0::<f64>().unwrap(),
                want
            );
        }
    }
}

/// Requires exact f64 fields rather than inserting implicit integer conversions.
#[test]
fn integer_fields_require_explicit_casts() {
    assert!(parser().parse_str("Point2D { x: 1, y: 2.0 }").is_err());
    check(
        "Point2D { x: 1 as f64, y: 2 as f64 }",
        Point2D::new(1.0, 2.0),
    );
}

/// Visits every permutation without retaining a factorial-sized list.
///
/// - Complexity: O(n! * n) time and O(n) stack space, excluding the visitor.
fn permutations<T>(items: &mut [T], first: usize, visit: &mut impl FnMut(&[T])) {
    if first == items.len() {
        visit(items);
        return;
    }
    for next in first..items.len() {
        items.swap(first, next);
        permutations(items, first + 1, visit);
        items.swap(first, next);
    }
}

/// Preserves every arbitrary matrix field order, not only the reversed layout.
#[test]
fn all_matrix_field_permutations_preserve_values() {
    let mut fields = [
        "m11: 1.0", "m12: 2.0", "m21: 3.0", "m22: 4.0", "m31: 5.0", "m32: 6.0",
    ];
    permutations(&mut fields, 0, &mut |fields| {
        check(
            &format!("(1u8, Transform2D {{ {} }}).1", fields.join(", ")),
            Transform2D::new(1.0, 2.0, 3.0, 4.0, 5.0, 6.0),
        );
    });
}

/// Records source evaluation order independently of canonical field placement.
#[test]
fn field_expressions_execute_once_in_written_order() {
    use std::sync::{Arc, Mutex};
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut lookup = OpLookup::new();
    install(&mut lookup);
    let log = events.clone();
    lookup.push_scope(move |name, segment, arity, _| {
        if arity != 0 {
            return Ok(false);
        }
        let log = log.clone();
        match name {
            "first" => segment.op0(move || {
                log.lock().unwrap().push(1);
                1.0
            }),
            "second" => segment.op0(move || {
                log.lock().unwrap().push(2);
                2.0
            }),
            "origin_value" => segment.op0(move || {
                log.lock().unwrap().push(3);
                Point2D::new(1.0, 2.0)
            }),
            "size_value" => segment.op0(move || {
                log.lock().unwrap().push(4);
                Size2D::new(4.0, 6.0)
            }),
            _ => return Ok(false),
        }
        Ok(true)
    });
    let mut parser = CELParser::new(lookup);
    for (source, expected_events) in [
        ("Point2D { y: second, x: first }.x", vec![2, 1]),
        ("Vector2D { y: second, x: first }.x", vec![2, 1]),
        ("Size2D { height: second, width: first }.width", vec![2, 1]),
        (
            "Rect { size: size_value, origin: origin_value }.origin.x",
            vec![4, 3],
        ),
        (
            "Rect { origin: origin_value, size: size_value }.origin.x",
            vec![3, 4],
        ),
        (
            "Box2D { max: origin_value, min: origin_value }.min.x",
            vec![3, 3],
        ),
        (
            "Angle { radians: first } == Angle { radians: first }",
            vec![1, 1],
        ),
        (
            "Transform2D { m32: second, m31: first, m22: second, m21: first, m12: second, m11: first } == Transform2D { m11: 1.0, m12: 2.0, m21: 1.0, m22: 2.0, m31: 1.0, m32: 2.0 }",
            vec![2, 1, 2, 1, 2, 1],
        ),
    ] {
        events.lock().unwrap().clear();
        let mut segment = parser.parse_str(source).unwrap();
        if source.contains(" == ") {
            assert!(segment.call0::<bool>().unwrap());
        } else {
            assert_eq!(segment.call0::<f64>().unwrap(), 1.0);
        }
        assert_eq!(*events.lock().unwrap(), expected_events, "{source}");
    }
}

/// Tracks ownership of an ambient value around record construction and evaluation errors.
struct DropProbe(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl Drop for DropProbe {
    /// Records exactly one observable destruction of this ambient value.
    fn drop(&mut self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Drops owned ambient values once on success, on normalization errors, and on retry.
#[test]
fn record_adapters_preserve_ambient_drop_safety() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let drops = Arc::new(AtomicUsize::new(0));
    let mut lookup = OpLookup::new();
    install(&mut lookup);
    let count = drops.clone();
    lookup.push_scope(move |name, segment, arity, _| {
        if name != "tracked" || arity != 0 {
            return Ok(false);
        }
        let count = count.clone();
        segment.op0(move || DropProbe(count.clone()));
        Ok(true)
    });
    let mut parser = CELParser::new(lookup);
    let reverse = format!("Rect {{ size: {S}, origin: {P} }}");
    let mut success = parser
        .parse_str(&format!("(tracked, {reverse}).1"))
        .unwrap();
    assert_eq!(success.call0::<Rect>().unwrap().size, Size2D::new(4.0, 6.0));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let mut failure = parser
        .parse_str(&format!(
            "(tracked, {reverse}, Vector2D {{ x: 0.0, y: 0.0 }}.normalize()).1"
        ))
        .unwrap();
    for expected in [2, 3] {
        let error = failure.call_dyn::<Rect>(&[]).unwrap_err();
        assert!(error.to_string().contains("normalize"), "{error}");
        assert_eq!(drops.load(Ordering::SeqCst), expected);
    }
    drop(failure);
    drop(success);
    assert_eq!(drops.load(Ordering::SeqCst), 3);
}

/// Checks defaults even when the optional Adam adapter is not built.
#[test]
fn standalone_defaults_match_euclid() {
    assert_eq!(Point2D::default(), Point2D::new(0.0, 0.0));
    assert_eq!(Vector2D::default(), Vector2D::new(0.0, 0.0));
    assert_eq!(Size2D::default(), Size2D::new(0.0, 0.0));
    assert_eq!(Rect::default(), Rect::new(Point2D::zero(), Size2D::zero()));
    assert_eq!(
        Box2D::default(),
        Box2D::new(Point2D::zero(), Point2D::zero())
    );
    assert_eq!(Angle::default(), Angle::radians(0.0));
    assert_eq!(Transform2D::default(), Transform2D::identity());
}

/// Rejects every signature with one incorrectly typed operand before emission.
#[test]
fn every_signature_rejects_wrong_operands() {
    use cel_runtime::DynSegment;
    use std::any::TypeId;
    let mut lookup = OpLookup::new();
    install(&mut lookup);
    let names = [
        "==",
        "!=",
        "+",
        "-",
        "*",
        "/",
        "is_finite",
        "x",
        "y",
        "width",
        "height",
        "origin",
        "size",
        "min",
        "max",
        "min_x",
        "min_y",
        "max_x",
        "max_y",
        "center",
        "dot",
        "cross",
        "square_length",
        "length",
        "normalize",
        "distance_to",
        "translate",
        "inflate",
        "to_box2d",
        "to_rect",
        "contains",
        "contains_rect",
        "contains_box",
        "intersects",
        "union",
        "degrees",
        "radians",
        "m11",
        "m12",
        "m21",
        "m22",
        "m31",
        "m32",
        "identity_transform",
        "translation",
        "scale",
        "rotation",
        "then",
        "determinant",
        "inverse",
        "transform_point",
        "transform_vector",
        "outer_transformed_rect",
    ];
    assert_eq!(
        names
            .iter()
            .map(|name| lookup.operation_signatures(name).count())
            .sum::<usize>(),
        98
    );
    for name in names {
        for signature in lookup.operation_signatures(name) {
            for bad in 0..signature.operands.len() {
                let mut segment = DynSegment::new::<()>();
                for (index, operand) in signature.operands.iter().enumerate() {
                    if index == bad {
                        segment.just(true);
                        continue;
                    }
                    macro_rules! push {
                        ($($ty:ty),+) => {
                            $(if operand.type_id() == TypeId::of::<$ty>() {
                                segment.just(<$ty>::default());
                            } else)+ { panic!("uncovered geometry operand"); }
                        };
                    }
                    push!(
                        Point2D,
                        Vector2D,
                        Size2D,
                        Rect,
                        Box2D,
                        Angle,
                        Transform2D,
                        f64
                    );
                }
                let before = segment.current_stack_offset();
                let span = proc_macro2::Span::call_site();
                assert!(
                    lookup
                        .lookup_member(name, &mut segment, signature.operands.len(), span, span)
                        .is_err(),
                    "{name}: incorrect operand {bad}"
                );
                assert_eq!(segment.current_stack_offset(), before);
            }
        }
    }
    // These remain deliberately outside the fixed operation table.
    for name in ["intersection", "contains_box2d"] {
        assert_eq!(lookup.operation_signatures(name).count(), 0, "{name}");
    }
}
