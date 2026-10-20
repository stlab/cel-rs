//! Foreign Euclid leaves compose with canonical containers without orphan impls.
use cel_parser::{CELParser, OpLookup, OperationSignature, ResolvedLeafType, ResolvedType};
use cel_runtime::{Array, ArrayOf, Cons, IntoCStackList, IntoRustTuple, Leaf, Nil};

/// Distinguishes this external client's coordinate space from installed geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ClientSpace;
/// Uses Euclid directly, not a runtime-specific wrapper or trait implementation.
type Point = euclid::Point2D<f32, ClientSpace>;
/// Stores the actual point and owning point-array values in a concrete list.
type Item = Cons<Leaf<Point>, Cons<ArrayOf<Leaf<Point>>, Nil>>;

/// Registers only the foreign leaf and delegates other named leaves to CEL.
fn resolve(name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType> {
    if name == "ClientPoint" && args.is_empty() {
        Some(ResolvedLeafType::new(
            "ClientPoint",
            cel_runtime::ValueType::cloneable_leaf::<Point>(),
        ))
    } else {
        cel_parser::resolve_builtin_named_type(name, args)
    }
}

/// Resolves one full client shape without registering any aggregate instantiation.
fn shape(source: &str) -> ResolvedType {
    CELParser::new(OpLookup::new())
        .parse_type_expr_str(source)
        .unwrap()
        .resolve(&resolve)
        .unwrap()
}

/// Real vectors, slices, and tuple-list host callbacks share complete CEL identities.
#[test]
fn foreign_points_roundtrip_nested_lists_vectors_and_slices() {
    #[derive(Clone)]
    struct Roundtrip;
    let mut ops = OpLookup::new();
    ops.register_function("roundtrip", Roundtrip).unwrap();
    ops.push_scope(|name, segment, arity, _| {
        if name == "point" && arity == 0 {
            segment.just(Point::new(3.0, 4.0));
            return Ok(true);
        }
        Ok(false)
    });
    ops.register_operation(OperationSignature {
        name: "roundtrip",
        operands: vec![shape("[(ClientPoint, [ClientPoint])]")],
        result: shape("[(ClientPoint, [ClientPoint])]"),
        emit: Box::new(|segment, _| segment.op1_vec::<Item, ArrayOf<Item>, _>(Array::from_vec)),
    })
    .unwrap();
    ops.register_operation(OperationSignature {
        name: "borrowed",
        operands: vec![shape("[(ClientPoint, [ClientPoint])]")],
        result: shape("[(ClientPoint, [ClientPoint])]"),
        emit: Box::new(|segment, _| {
            segment.op1_slice::<Item, ArrayOf<Item>, _>(|values| Array::from_vec(values.to_vec()))
        }),
    })
    .unwrap();
    ops.register_operation(OperationSignature {
        name: "list",
        operands: vec![shape("(ClientPoint, [ClientPoint])")],
        result: shape("(ClientPoint, [ClientPoint])"),
        emit: Box::new(|segment, _| segment.op1_repr::<Item, Item, _>(|list| list)),
    })
    .unwrap();
    let mut parser = CELParser::with_type_resolver(ops, resolve);
    for source in [
        "roundtrip([(point, [point]).list]).borrowed",
        "[(point, [point]).list()].roundtrip().borrowed()",
        "(|x: [(ClientPoint, [ClientPoint])]| x.roundtrip().borrowed())([(point, [point])])",
    ] {
        let body = parser.parse_str(source).unwrap();
        let mut prepared = body.prepare::<Nil, ArrayOf<Item>>().unwrap();
        for _ in 0..2 {
            let values = prepared.call(&().into_c_stack_list()).unwrap().into_vec();
            let (point, points) = values[0].clone().into_rust_tuple();
            assert_eq!(point, Point::new(3.0, 4.0));
            assert_eq!(points.as_slice(), &[point]);
        }
    }
}
