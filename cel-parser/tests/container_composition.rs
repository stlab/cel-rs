//! Contracts for recursively composed source types.
use cel_parser::{AstContext, CELParser, OpLookup, Parser, ResolvedType, Ty, ty::check_expr};
use cel_runtime::{ArrayOf, Cons, FnOf, IntoCStackList, IntoRustTuple, Leaf, Nil, ValueRepr};

/// Resolves complete source syntax using the public builtin registry.
fn resolved(source: &str) -> ResolvedType {
    CELParser::new(OpLookup::new())
        .parse_type_expr_str(source)
        .unwrap()
        .resolve(cel_parser::builtin_type_resolver().as_ref())
        .unwrap()
}

/// Complete syntax retains its shape through checking and executable preparation.
#[test]
fn recursive_source_shapes() {
    for (source, expected) in [
        ("[]: [(i32, [f64])]", "[(i32, [f64])]"),
        ("[]: [Fn((i32,), [f64])]", "[Fn((i32,), [f64])]"),
        ("((), [()])", "((), [()])"),
        ("[(1i32,), (2i32,)]", "[(i32,)]"),
        ("[|x: i32| x + 1, |x: i32| x + 2]", "[Fn((i32,), i32)]"),
    ] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let (ty, diagnostics) = check_expr(&expr, &|_| Ty::Any);
        assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
        assert_eq!(ty.name(), expected, "{source}: {ty:?}");
        let formatted = cel_parser::format_expr(&expr, source, 0);
        let roundtrip = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(&formatted)
            .unwrap();
        assert_eq!(check_expr(&roundtrip, &|_| Ty::Any).0, ty);
        assert_eq!(
            cel_parser::format_expr(&roundtrip, &formatted, 0),
            formatted
        );
        CELParser::new(OpLookup::new())
            .parse_str(&formatted)
            .unwrap();
        CELParser::new(OpLookup::new()).parse_str(source).unwrap();
    }
    type F = FnOf<Cons<Leaf<i32>, Nil>, ArrayOf<Leaf<f64>>>;
    let body = CELParser::new(OpLookup::new())
        .parse_str("[]: [Fn((i32,), [f64])]")
        .unwrap();
    let mut prepared = body.prepare::<Nil, ArrayOf<F>>().unwrap();
    assert!(prepared.call(&().into_c_stack_list()).unwrap().is_empty());
    type Pair = Cons<Leaf<i32>, Cons<ArrayOf<Leaf<f64>>, Nil>>;
    let body = CELParser::new(OpLookup::new())
        .parse_str("[]: [(i32, [f64])]")
        .unwrap();
    assert!(
        body.prepare::<Nil, ArrayOf<Pair>>()
            .unwrap()
            .call(&().into_c_stack_list())
            .unwrap()
            .is_empty()
    );
    type UnitArray = Cons<Nil, Cons<ArrayOf<Nil>, Nil>>;
    let body = CELParser::new(OpLookup::new())
        .parse_str("((), [()])")
        .unwrap();
    let (_, units) = body
        .prepare::<Nil, UnitArray>()
        .unwrap()
        .call(&().into_c_stack_list())
        .unwrap()
        .into_rust_tuple();
    assert_eq!(units.len(), 1);
    type One = Cons<Leaf<i32>, Nil>;
    let body = CELParser::new(OpLookup::new())
        .parse_str("[(1i32,), (2i32,)]")
        .unwrap();
    let values = body
        .prepare::<Nil, ArrayOf<One>>()
        .unwrap()
        .call(&().into_c_stack_list())
        .unwrap();
    assert_eq!(values.as_slice()[1].clone().into_rust_tuple(), (2,));
    type Increment = FnOf<One, Leaf<i32>>;
    let body = CELParser::new(OpLookup::new())
        .parse_str("[|x: i32| x + 1, |x: i32| x + 2]")
        .unwrap();
    let functions = body
        .prepare::<Nil, ArrayOf<Increment>>()
        .unwrap()
        .call(&().into_c_stack_list())
        .unwrap();
    assert_eq!(
        functions.as_slice()[1]
            .call(&(40,).into_c_stack_list())
            .unwrap(),
        42
    );
}

/// Incompatible complete shapes are rejected by both public parser surfaces.
#[test]
fn incompatible_source_shapes() {
    for source in [
        "[(1i32,), (2i32, 3i32)]",
        "[(1i32,), (2.0f64,)]",
        "[|x: i32| x, |x: f64| x]",
        "[|x: i32| x, |x: i32| true]",
        "[]: i32",
        "if true { (1i32,) } else { (1.0f64,) }",
    ] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let diagnostics = check_expr(&expr, &|_| Ty::Any).1;
        assert!(!diagnostics.is_empty(), "{source}");
        assert!(diagnostics[0].end_span().is_some(), "{source}");
        let error = CELParser::new(OpLookup::new())
            .parse_str(source)
            .err()
            .unwrap();
        assert!(error.end_span().is_some(), "{source}");
    }
}

/// Callable syntax is structural and never delegated to a host generic fallback.
#[test]
fn callable_resolution_and_singleton_names() {
    for (source, expected) in [
        (
            "Fn((), [i32])",
            FnOf::<Nil, ArrayOf<Leaf<i32>>>::value_type(),
        ),
        (
            "Fn((i32,), [f64])",
            FnOf::<Cons<Leaf<i32>, Nil>, ArrayOf<Leaf<f64>>>::value_type(),
        ),
        (
            "Fn((i32, f64), ())",
            FnOf::<Cons<Leaf<i32>, Cons<Leaf<f64>, Nil>>, Nil>::value_type(),
        ),
    ] {
        let ty = resolved(source);
        assert_eq!(ty.display_name(), source);
        assert!(ty.into_value_type().unwrap().same_shape(&expected));
    }
    assert_eq!(resolved("(i32,)").display_name(), "(i32,)");
    for source in [
        "Fn",
        "Fn()",
        "Fn(i32, i32)",
        "Fn((i32,))",
        "Fn((), i32, i32)",
    ] {
        let syntax = CELParser::new(OpLookup::new())
            .parse_type_expr_str(source)
            .unwrap();
        let error = syntax
            .resolve(&|_: &str, _: &[ResolvedType]| {
                Some(cel_parser::ResolvedLeafType::leaf::<i32>("host fallback").unwrap())
            })
            .unwrap_err();
        assert!(error.message().contains("Fn"));
        assert!(error.end_span().is_some());
    }
}

/// Borrowed and owned adapters round-trip concrete tuple lists without erasure.
#[test]
fn host_vector_slice_and_list_registration() {
    use cel_parser::OperationSignature;
    use std::sync::Arc;
    #[derive(Clone)]
    struct Mirror;
    type Item = Cons<Leaf<i32>, Cons<ArrayOf<Leaf<f64>>, Nil>>;
    let mut ops = OpLookup::new();
    ops.register_function("mirror", Mirror).unwrap();
    for (name, emitter) in [
        (
            "mirror",
            Arc::new(|s: &mut cel_runtime::DynSegment, _| {
                s.op1_vec::<Item, ArrayOf<Item>, _>(cel_runtime::Array::from_vec)
            }) as cel_parser::OperationEmitter,
        ),
        (
            "borrowed",
            Arc::new(|s: &mut cel_runtime::DynSegment, _| {
                s.op1_slice::<Item, ArrayOf<Item>, _>(|values| {
                    cel_runtime::Array::from_vec(values.to_vec())
                })
            }) as cel_parser::OperationEmitter,
        ),
    ] {
        ops.register_operation(OperationSignature {
            name,
            operands: vec![resolved("[(i32, [f64])]")],
            result: resolved("[(i32, [f64])]"),
            emit: emitter,
        })
        .unwrap();
    }
    for source in [
        "mirror([(1i32, [2.0f64])]).borrowed",
        "[(1i32, [2.0f64])].mirror().borrowed()",
    ] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let (ty, diagnostics) = cel_parser::ty::check_expr_with_type_resolver(
            &expr,
            &|_| Ty::Any,
            cel_parser::builtin_type_resolver().as_ref(),
            &ops,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(ty.name(), "[(i32, [f64])]");
    }
    let mut parser = CELParser::new(ops);
    let body = parser
        .parse_str("mirror([(1i32, [2.0f64])]).borrowed")
        .unwrap();
    let values = body
        .prepare::<Nil, ArrayOf<Item>>()
        .unwrap()
        .call(&().into_c_stack_list())
        .unwrap()
        .into_vec();
    let (number, array) = values[0].clone().into_rust_tuple();
    assert_eq!(number, 1);
    assert_eq!(array.as_slice(), &[2.0]);
}

/// Invalid construction never executes operand producers, even across nested shapes.
#[test]
fn rejected_shapes_have_no_effects() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let effects = Arc::new(AtomicUsize::new(0));
    let mut ops = OpLookup::new();
    let observed = effects.clone();
    ops.push_scope(move |name, segment, arity, _| {
        if name != "effect" || arity != 0 {
            return Ok(false);
        }
        let observed = observed.clone();
        segment.op0(move || {
            observed.fetch_add(1, Ordering::SeqCst);
            1i32
        });
        Ok(true)
    });
    let mut parser = CELParser::new(ops);
    for source in [
        "[(effect,), (true,)]",
        "[effect]: [f64]",
        "if true { (effect,) } else { (true,) }",
        "(|f: Fn((i32,), i32)| f(effect))(|x: f64| x)",
    ] {
        assert!(parser.parse_str(source).is_err(), "{source}");
        assert_eq!(effects.load(Ordering::SeqCst), 0);
    }
    let expr = Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast("[unknown]")
        .unwrap();
    assert!(check_expr(&expr, &|_| Ty::Any).1.is_empty());
    assert!(parser.parse_str("[unknown]").is_err());
}

/// Known noncallable and nontuple values produce diagnostics rather than unresolved types.
#[test]
fn known_invalid_projections_and_calls_are_diagnostics() {
    for source in [
        "1i32.0",
        "(1i32,).1",
        "(1i32)(2i32)",
        "(1i32,) + 2i32",
        "(1i32,) as f64",
    ] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        assert!(!check_expr(&expr, &|_| Ty::Any).1.is_empty(), "{source}");
        assert!(
            CELParser::new(OpLookup::new()).parse_str(source).is_err(),
            "{source}"
        );
    }
}

/// Native and prepared host callables share a source signature with interpreted closures.
#[test]
fn registered_callables_mix_with_source_closures() {
    use cel_parser::OperationSignature;
    use cel_runtime::{CNil, CStackList, Callable};
    use std::sync::Arc;
    type Args = Cons<Leaf<i32>, Nil>;
    type Function = FnOf<Args, ArrayOf<Leaf<i32>>>;
    /// Returns one independently owned array for one native argument.
    fn native(args: &CStackList<i32, CNil<()>>) -> anyhow::Result<Array<i32>> {
        Ok(Array::from_vec(vec![args.clone().into_rust_tuple().0]))
    }
    use cel_runtime::Array;
    let mut ops = OpLookup::new();
    ops.register_operation(OperationSignature {
        name: "native",
        operands: vec![],
        result: resolved("Fn((i32,), [i32])"),
        emit: Arc::new(|s, _| s.op0_repr::<Function, _>(|| Callable::from_fn(native))),
    })
    .unwrap();
    ops.register_operation(OperationSignature {
        name: "prepared",
        operands: vec![],
        result: resolved("Fn((i32,), [i32])"),
        emit: Arc::new(|s, _| {
            let mut body = cel_runtime::DynSegment::new::<()>();
            body.push_argument(0, cel_runtime::ValueType::cloneable_leaf::<i32>())?;
            body.op1_repr::<Leaf<i32>, ArrayOf<Leaf<i32>>, _>(|value| {
                Array::from_vec(vec![value])
            })?;
            let value = Callable::from_prepared(body.prepare::<Args, ArrayOf<Leaf<i32>>>()?);
            s.op0_repr::<Function, _>(move || value.clone())
        }),
    })
    .unwrap();
    let source = "[native, prepared, |x: i32| [x]]";
    let expr = Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast(source)
        .unwrap();
    let (ty, diagnostics) = cel_parser::ty::check_expr_with_type_resolver(
        &expr,
        &|_| Ty::Any,
        cel_parser::builtin_type_resolver().as_ref(),
        &ops,
    );
    assert!(diagnostics.is_empty());
    assert_eq!(ty.name(), "[Fn((i32,), [i32])]");
    let expr = Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast("[native, prepared]")
        .unwrap();
    assert_eq!(
        cel_parser::ty::check_expr_with_type_resolver(
            &expr,
            &|_| Ty::Any,
            cel_parser::builtin_type_resolver().as_ref(),
            &ops,
        )
        .0,
        ty
    );
    let body = CELParser::new(ops).parse_str(source).unwrap();
    let functions = body
        .prepare::<Nil, ArrayOf<Function>>()
        .unwrap()
        .call(&().into_c_stack_list())
        .unwrap();
    for function in functions.as_slice() {
        assert_eq!(
            function
                .call(&(42,).into_c_stack_list())
                .unwrap()
                .as_slice(),
            &[42]
        );
    }
}
