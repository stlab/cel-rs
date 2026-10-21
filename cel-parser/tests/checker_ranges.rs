//! Checker and executable agreement for concrete builtin range results.

use cel_parser::{AstContext, CELParser, OpLookup, Parser, Ty};

/// Checks one executable result descriptor against inference and an independent annotation.
fn assert_range_agreement(expression: &str, expected: &str) {
    let mut parser = CELParser::new(OpLookup::new());
    let executable = parser.parse_str(expression).unwrap();
    let resolved = parser
        .parse_type_expr_str(expected)
        .unwrap()
        .resolve(cel_parser::builtin_type_resolver().as_ref())
        .unwrap();
    let descriptor = resolved.complete_value_type().unwrap();
    assert!(
        executable.peek_stack_infos(1)[0]
            .value_type
            .same_shape(&descriptor),
        "{expression}"
    );
    let ast = Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast(expression)
        .unwrap();
    let (ty, diagnostics) = cel_parser::ty::check_expr(&ast, &|_| Ty::Any);
    assert!(
        diagnostics.is_empty(),
        "{expression}: {ty:?}, {diagnostics:?}"
    );
    let annotation = format!("|x: {expected}| x");
    let ast = Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast(&annotation)
        .unwrap();
    let (Ty::Callable(parameters, _), _) = cel_parser::ty::check_expr(&ast, &|_| Ty::Any) else {
        panic!("closure")
    };
    assert_eq!(ty, parameters[0], "{expression}");
}

/// Range arguments, results and nested annotations retain their concrete identities.
#[test]
fn concrete_range_results_agree_with_executable() {
    for (expression, expected) in [
        (
            "(|x: RangeInclusive(i32)| x)(1i32..=2i32)",
            "RangeInclusive(i32)",
        ),
        ("(|x: Range(i32)| x)(1i32..2i32)", "Range(i32)"),
        (
            "[|x: i32| 1i32..=x]: [Fn((i32,), RangeInclusive(i32))]",
            "[Fn((i32,), RangeInclusive(i32))]",
        ),
        (
            "[|x: i32| 1i32..x]: [Fn((i32,), Range(i32))]",
            "[Fn((i32,), Range(i32))]",
        ),
        (
            "[[|x: i32| 1i32..=x]]: [[Fn((i32,), RangeInclusive(i32))]]",
            "[[Fn((i32,), RangeInclusive(i32))]]",
        ),
        (
            "[[|x: i32| 1i32..x]]: [[Fn((i32,), Range(i32))]]",
            "[[Fn((i32,), Range(i32))]]",
        ),
        (
            "[[1i32..=2i32]]: [[RangeInclusive(i32)]]",
            "[[RangeInclusive(i32)]]",
        ),
        ("[[1i32..2i32]]: [[Range(i32)]]", "[[Range(i32)]]"),
        ("[1i32..]: [RangeFrom(i32)]", "[RangeFrom(i32)]"),
        ("[..2i32]: [RangeTo(i32)]", "[RangeTo(i32)]"),
        (
            "[..=2i32]: [RangeToInclusive(i32)]",
            "[RangeToInclusive(i32)]",
        ),
        ("[..]: [RangeFull()]", "[RangeFull()]"),
        (
            "(|x: RangeInclusive(f64)| x)(1.0f64..=2.0f64)",
            "RangeInclusive(f64)",
        ),
    ] {
        assert_range_agreement(expression, expected);
    }
    assert_eq!(
        CELParser::new(OpLookup::new())
            .parse_str("(|x: RangeInclusive(i32)| x)(1i32..=2i32)")
            .unwrap()
            .call0::<std::ops::RangeInclusive<i32>>()
            .unwrap(),
        1..=2,
    );
    assert_eq!(
        CELParser::new(OpLookup::new())
            .parse_str("(|x: Range(i32)| x)(1i32..2i32)")
            .unwrap()
            .call0::<std::ops::Range<i32>>()
            .unwrap(),
        1..2,
    );
}

/// Every supported endpoint and range constructor has the same concrete checker result.
#[test]
fn all_builtin_range_endpoints_agree() {
    for ty in [
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
        "f32", "f64",
    ] {
        let (lo, hi) = if ty.starts_with('f') {
            (format!("1.0{ty}"), format!("2.0{ty}"))
        } else {
            (format!("1{ty}"), format!("2{ty}"))
        };
        for (expression, constructor) in [
            (format!("{lo}..{hi}"), "Range"),
            (format!("{lo}..={hi}"), "RangeInclusive"),
            (format!("{lo}.."), "RangeFrom"),
            (format!("..{hi}"), "RangeTo"),
            (format!("..={hi}"), "RangeToInclusive"),
        ] {
            assert_range_agreement(&expression, &format!("{constructor}({ty})"));
        }
    }
}

/// Registered operator metadata takes precedence over builtin range result inference.
#[test]
fn registered_range_overload_retains_its_result() {
    use cel_parser::{OperationSignature, ResolvedLeafType};
    use cel_runtime::ValueType;
    let mut lookup = OpLookup::new();
    let int = ResolvedLeafType::new("i32", ValueType::leaf::<i32>());
    lookup
        .register_operation(OperationSignature {
            name: "range_inclusive",
            operands: vec![int.clone().into(), int.into()],
            result: ResolvedLeafType::new("bool", ValueType::leaf::<bool>()).into(),
            emit: Box::new(|s, _| s.op2(|a: i32, b: i32| a <= b)),
        })
        .unwrap();
    let source = "(|x: bool| x)(1i32..=2i32)";
    let ast = Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast(source)
        .unwrap();
    let (ty, diagnostics) = cel_parser::ty::check_expr_with_type_resolver(
        &ast,
        &|_| Ty::Any,
        cel_parser::builtin_type_resolver().as_ref(),
        &lookup,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(ty, Ty::Bool);
    assert!(
        CELParser::new(lookup)
            .parse_str(source)
            .unwrap()
            .call0::<bool>()
            .unwrap()
    );
}

/// Inclusive and exclusive ranges are not interchangeable with scalars or one another.
#[test]
fn range_mismatches_agree_with_executable() {
    for source in [
        "(|x: i32| x)(1i32..=2i32)",
        "(|x: Range(i32)| x)(1i32..=2i32)",
        "[|x: i32| 1i32..=x]: [Fn((i32,), i32)]",
        "[[1i32..2i32]]: [[RangeInclusive(i32)]]",
    ] {
        assert!(
            CELParser::new(OpLookup::new()).parse_str(source).is_err(),
            "{source}"
        );
        let ast = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let (_, diagnostics) = cel_parser::ty::check_expr(&ast, &|_| Ty::Any);
        assert!(!diagnostics.is_empty(), "{source}");
    }
}
