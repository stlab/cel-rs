//! Contract tests for library-neutral receiver operation syntax.

use std::sync::{Arc, Mutex};

use cel_parser::{
    AstContext, CELParser, Expr, OpLookup, OperationSignature, Parser, ResolvedLeafType,
    SourceSpan, Ty, format_expr,
};
use cel_runtime::ArrayElementType;

/// Supplies a host-defined receiver.
#[derive(Clone)]
struct TestPoint(f64);

/// Supplies a distinct host-defined argument.
#[derive(Clone)]
struct TestVector(f64);

/// Describes one concrete host leaf.
fn leaf<T: 'static>() -> ResolvedLeafType {
    ResolvedLeafType::new(
        std::any::type_name::<T>(),
        ArrayElementType::leaf::<T>().unwrap(),
    )
}

/// Installs host operations and values without geometry-library dependencies.
fn lookup() -> OpLookup {
    let mut lookup = OpLookup::new();
    for name in ["x", "width"] {
        lookup
            .register_operation(OperationSignature {
                name,
                operands: vec![leaf::<f64>()],
                result: leaf::<f64>(),
                emit: Arc::new(|s, _| s.op1(|value: f64| value)),
            })
            .unwrap();
    }
    lookup
        .register_operation(OperationSignature {
            name: "x",
            operands: vec![leaf::<TestPoint>()],
            result: leaf::<f64>(),
            emit: Arc::new(|s, _| s.op1(|p: TestPoint| p.0)),
        })
        .unwrap();
    lookup
        .register_operation(OperationSignature {
            name: "shift",
            operands: vec![leaf::<TestPoint>(), leaf::<TestVector>()],
            result: leaf::<TestPoint>(),
            emit: Arc::new(|s, _| s.op2(|p: TestPoint, v: TestVector| TestPoint(p.0 + v.0))),
        })
        .unwrap();
    lookup
        .register_operation(OperationSignature {
            name: "combine",
            operands: vec![leaf::<TestPoint>(), leaf::<f64>(), leaf::<f64>()],
            result: leaf::<f64>(),
            emit: Arc::new(|s, _| s.op3(|p: TestPoint, a: f64, b: f64| p.0 + 10.0 * a + b)),
        })
        .unwrap();
    lookup.push_scope(|name, s, arity, _| {
        if arity != 0 {
            return Ok(false);
        }
        match name {
            "p" => s.just(TestPoint(3.0)),
            "v" => s.just(TestVector(4.0)),
            "x" | "width" => s.just(100.0f64),
            _ => return Ok(false),
        };
        Ok(true)
    });
    lookup
}

/// Evaluates a host expression to its scalar result.
fn evaluate(source: &str) -> f64 {
    CELParser::new(lookup())
        .parse_str(source)
        .unwrap()
        .call0::<f64>()
        .unwrap()
}

/// Verifies property spelling and explicit empty arguments share unary dispatch.
#[test]
fn member_property_and_zero_arg_call_agree() {
    assert_eq!(evaluate("p.x"), 3.0);
    assert_eq!(evaluate("p.x()"), 3.0);
}

/// Verifies heterogeneous receiver and explicit argument positions.
#[test]
fn member_call_passes_receiver_first() {
    assert_eq!(evaluate("p.shift(v).x"), 7.0);
    assert_eq!(evaluate("p.combine(2.0, 5.0)"), 28.0);
}

/// Verifies tuple-index chains and ordinary operator precedence survive named postfixes.
#[test]
fn member_chains_with_tuple_index() {
    assert_eq!(evaluate("((p, v), 1i32).0.0.shift(v).x()"), 7.0);
    assert_eq!(evaluate("-p.x * 2.0 + 1.0"), -5.0);
}

/// Verifies value bindings and closure locals cannot hide direct dot operations.
#[test]
fn shadowed_free_function_does_not_hide_member() {
    assert_eq!(evaluate("x"), 100.0);
    assert_eq!(evaluate("p.x()"), 3.0);
    assert!(CELParser::new(lookup()).parse_str("x()").is_err());
    assert_eq!(evaluate("width.width"), 100.0);
    let closure = CELParser::new(lookup())
        .parse_str("|x: f64, width: f64| x.x() + width.width")
        .unwrap()
        .call0::<cel_runtime::DynClosure>()
        .unwrap();
    assert_eq!(closure.call::<f64>(&[&3.0f64, &4.0f64]).unwrap(), 7.0);
}

/// Verifies grouping distinguishes calls of member results from direct member dispatch.
#[test]
fn parenthesized_member_result_is_an_ordinary_callee() {
    assert_eq!(evaluate("p.x()"), 3.0);
    assert!(CELParser::new(lookup()).parse_str("(p.x)()").is_err());
    assert_eq!(evaluate("(p.x) + 2.0"), 5.0);
}

/// Verifies compilation defers evaluation and execution evaluates each operand once in order.
#[test]
fn receiver_and_arguments_evaluate_once_in_order() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mut lookup = lookup();
    let recorded = trace.clone();
    lookup.push_scope(move |name, s, arity, _| {
        if arity != 0 {
            return Ok(false);
        }
        let trace = recorded.clone();
        match name {
            "receiver" => s.op0(move || {
                trace.lock().unwrap().push("receiver");
                TestPoint(3.0)
            }),
            "arg1" | "arg2" => {
                let (label, value) = if name == "arg1" {
                    ("arg1", 2.0)
                } else {
                    ("arg2", 5.0)
                };
                s.op0(move || {
                    trace.lock().unwrap().push(label);
                    value
                });
            }
            _ => return Ok(false),
        };
        Ok(true)
    });
    let mut segment = CELParser::new(lookup)
        .parse_str("receiver.combine(arg1, arg2)")
        .unwrap();
    assert!(trace.lock().unwrap().is_empty());
    assert_eq!(segment.call0::<f64>().unwrap(), 28.0);
    assert_eq!(*trace.lock().unwrap(), ["receiver", "arg1", "arg2"]);
}

/// Verifies formatting preserves property spelling and ordinary callee grouping.
#[test]
fn member_formatting_preserves_spelling_and_call_precedence() {
    for source in [
        "a.center().x",
        "a.x()",
        "(a.x)()",
        "(a + b).x",
        "a.x.0",
        "a.x(1i32, b.y())",
        "a.x()()",
        "(a.x)(1i32)",
    ] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        assert_eq!(format_expr(&expr, source, 0), source);
    }
}

/// Verifies comments around dots and argument delimiters survive an idempotent round trip.
#[test]
fn member_formatting_preserves_comments() {
    for source in [
        "a /* before */ . /* after */ center /* call */ ( /* empty */ ).x",
        "a // before\n . // after\n center // call\n ( // empty\n ).x",
    ] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let formatted = format_expr(&expr, source, 0);
        for comment in ["before", "after", "call", "empty"] {
            assert_eq!(formatted.matches(comment).count(), 1, "{formatted}");
        }
        let reparsed = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(&formatted)
            .unwrap();
        assert_eq!(format_expr(&reparsed, &formatted, 0), formatted);
    }
}

/// Verifies grouped receivers retain dot comments without changing postfix precedence.
#[test]
fn grouped_member_receivers_preserve_dot_comments() {
    for (source, expected) in [
        ("((a)).x", "a.x"),
        ("(a.center()).x()", "a.center().x()"),
        (
            "(a) /* before */ . /* after */ x",
            "a /* before */ . /* after */ x",
        ),
        (
            "(a.center()) /* before */ . /* after */ x()",
            "a.center() /* before */ . /* after */ x()",
        ),
        (
            "((a.x)) /* before */ . /* after */ y.z",
            "a.x /* before */ . /* after */ y.z",
        ),
        (
            "(a + b) /* before */ . /* after */ x",
            "(a + b) /* before */ . /* after */ x",
        ),
        (
            "((a) /* inner ) */ ) /* before */ . /* after */ x",
            "a /* inner ) */ /* before */ . /* after */ x",
        ),
        (
            "(a) // before\n . // after\n x",
            "a // before\n    . // after\n    x",
        ),
    ] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let formatted = format_expr(&expr, source, 0);
        assert_eq!(formatted, expected, "{source}");
        let reparsed = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(&formatted)
            .unwrap();
        assert_eq!(format_expr(&reparsed, &formatted, 0), formatted);
    }
}

/// Verifies postfix operations stay outside a closure body after formatting.
#[test]
fn closure_postfix_formatting_preserves_evaluation() {
    let mut lookup = lookup();
    lookup.push_scope(|name, s, arity, _| {
        if name == "()" && arity == 1 {
            s.invoke_closure(0)?;
            return Ok(true);
        }
        if name != "arity" || arity != 1 {
            return Ok(false);
        }
        let cel_runtime::ValueKind::Callable { parameters, .. } =
            s.peek_stack_infos(1)[0].value_type.kind()
        else {
            anyhow::bail!("callable required");
        };
        let count = parameters.len();
        s.drop_value()?;
        s.just(count);
        Ok(true)
    });
    let mut parser = CELParser::new(lookup);
    for (source, expected_arity) in [("(|| 1i32).arity", 0), ("(|x: i32| x).arity()", 1)] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let formatted = format_expr(&expr, source, 0);
        assert_eq!(formatted, source);
        let before = parser.parse_str(source).unwrap().call0::<usize>().unwrap();
        assert_eq!(before, expected_arity);
        let after = parser
            .parse_str(&formatted)
            .unwrap()
            .call0::<usize>()
            .unwrap();
        assert_eq!(before, after);
        let reparsed = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(&formatted)
            .unwrap();
        assert_eq!(format_expr(&reparsed, &formatted, 0), formatted);
    }
    for source in ["(|| 1i32)()", "(|| (1i32, 2i32)).0"] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let formatted = format_expr(&expr, source, 0);
        assert_eq!(formatted, source);
        match (&expr, source.ends_with(".0")) {
            (Expr::TupleIndex { base, .. }, true) => {
                assert!(matches!(base.as_ref(), Expr::Closure { .. }));
            }
            (Expr::Apply { callee, .. }, false) => {
                assert!(matches!(callee.as_ref(), Expr::Closure { .. }));
            }
            _ => panic!("postfix must remain outside the closure"),
        }
        let reparsed = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(&formatted)
            .unwrap();
        assert_eq!(format_expr(&reparsed, &formatted, 0), formatted);
    }
    let source = "(|| 1i32)()";
    let expr = Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast(source)
        .unwrap();
    let formatted = format_expr(&expr, source, 0);
    for text in [source, &formatted] {
        assert_eq!(parser.parse_str(text).unwrap().call0::<i32>().unwrap(), 1);
    }
}

/// Verifies omitted receiver groups retain each leading and trailing comment in source order.
#[test]
fn grouped_member_receivers_preserve_leading_comments() {
    for source in [
        "(/* first */ a).x",
        "((/* first */ a)).x",
        "(/* first */ (/* second */ a) /* third */) /* fourth */ .x",
        "(/* first /* nested ) */ end */ a).x",
        "(// first\n (/* second */ a) // third\n).x.y",
        "(/* first */ a.x).y",
        "(/* first */ a + b).x",
    ] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let formatted = format_expr(&expr, source, 0);
        if source == "(/* first */ a).x" || source == "((/* first */ a)).x" {
            assert_eq!(formatted, "(/* first */ a).x");
        }
        let mut previous = 0;
        for label in ["first", "second", "third", "fourth"] {
            if source.contains(label) {
                assert_eq!(
                    formatted.matches(label).count(),
                    1,
                    "{source} -> {formatted}"
                );
                let position = formatted.find(label).unwrap();
                assert!(position >= previous, "{formatted}");
                previous = position;
            }
        }
        let reparsed = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(&formatted)
            .unwrap();
        assert_eq!(format_expr(&reparsed, &formatted, 0), formatted);
    }
}

/// Verifies adjacent postfix forms retain grouped callee and index-base leading trivia.
#[test]
fn grouped_postfix_bases_preserve_comments() {
    for source in [
        "(/* first */ f) /* second */ (/* third */ 1i32)",
        "((/* first */ f))()",
        "(/* first */ (1i32, 2i32)).0",
        "(/* first */ || 1i32) /* second */ ()",
    ] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let formatted = format_expr(&expr, source, 0);
        for label in ["first", "second", "third"] {
            if source.contains(label) {
                assert_eq!(
                    formatted.matches(label).count(),
                    1,
                    "{source} -> {formatted}"
                );
            }
        }
        let reparsed = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(&formatted)
            .unwrap();
        assert_eq!(format_expr(&reparsed, &formatted, 0), formatted);
    }
}

/// Verifies nested dot comments retain their text and remain parseable and idempotent.
#[test]
fn member_receivers_preserve_nested_block_comments() {
    for (source, expected) in [
        (
            "(a) /* outer /* inner */ outer */ .x",
            "a /* outer /* inner */ outer */ .x",
        ),
        (
            "a /* outer /* inner */ outer */ .x",
            "a /* outer /* inner */ outer */ .x",
        ),
        (
            "(a). /* outer /* inner */ outer */ x",
            "a. /* outer /* inner */ outer */ x",
        ),
        (
            "a. /* outer /* inner */ outer */ x",
            "a. /* outer /* inner */ outer */ x",
        ),
        (
            "((a) /* outer /* ) . */ outer */ ) /* before */ . /* after /* nested */ end */ x()",
            "a /* outer /* ) . */ outer */ /* before */ . /* after /* nested */ end */ x()",
        ),
        (
            "(a + b) /* α /* β /* γ */ β */ α */ .x",
            "(a + b) /* α /* β /* γ */ β */ α */ .x",
        ),
        (
            "a /* first /* nested */ end */ /* second */ . /* third /* nested */ end */ x",
            "a /* first /* nested */ end */ /* second */ . /* third /* nested */ end */ x",
        ),
        (
            "(a) /* outer // line\n /* inner */ outer */ .x",
            "a /* outer // line\n/* inner */ outer */ .x",
        ),
    ] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let formatted = format_expr(&expr, source, 0);
        assert_eq!(formatted, expected, "{source}");
        let reparsed = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(&formatted)
            .unwrap();
        assert_eq!(format_expr(&reparsed, &formatted, 0), formatted);
    }
}

/// Verifies an unresolved member reports its label rather than the full expression.
#[test]
fn unknown_member_diagnostic_points_to_name() {
    let error = CELParser::new(lookup())
        .parse_str("p.missing(1.0)")
        .err()
        .unwrap();
    assert!(error.message().contains("no operation"), "{error}");
    assert_eq!(
        SourceSpan::from_proc_macro2_range(error.span(), error.end_span().unwrap_or(error.span())),
        SourceSpan::new(1, 2, 1, 9)
    );
}

/// Verifies deferred checking traverses all operands without resolving the member label.
#[test]
fn member_checker_visits_operands_not_labels() {
    let expr = Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast("(true + 1i32).label(false + 2i32)")
        .unwrap();
    let (ty, diagnostics) =
        cel_parser::ty::check_expr(&expr, &|name| panic!("label resolved: {name}"));
    assert_eq!(ty, Ty::Any);
    assert_eq!(diagnostics.len(), 2);
}

/// Verifies the AST preserves empty-call spelling, name spans, and enclosing member spans.
#[test]
fn member_ast_preserves_spelling_and_spans() {
    for (source, has_args, end) in [("a.x", false, 3), ("a.x()", true, 5)] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        assert_eq!(
            SourceSpan::from_proc_macro2_range(expr.span().start, expr.span().end),
            SourceSpan::new(1, 0, 1, end)
        );
        let Expr::Member {
            receiver,
            name,
            name_span,
            args,
            ..
        } = expr
        else {
            panic!("expected member");
        };
        assert!(matches!(*receiver, Expr::Ident { ref name, .. } if name == "a"));
        assert_eq!(name, "x");
        assert_eq!(
            SourceSpan::from_proc_macro2_range(name_span.start, name_span.end),
            SourceSpan::new(1, 2, 1, 3)
        );
        assert_eq!(args.is_some(), has_args);
        assert!(args.iter().all(Vec::is_empty));
    }
}

/// Verifies a member's callable result uses the ordinary call path after grouping.
#[test]
fn member_result_can_be_called_normally() {
    /// Identifies a test callable value, not a member operation.
    #[derive(Clone)]
    struct Callable;
    let mut lookup = lookup();
    lookup.push_scope(|name, s, arity, _| match (name, arity) {
        ("callable", 1) => {
            s.op1(|_: TestPoint| Callable)?;
            Ok(true)
        }
        ("()", 2)
            if s.peek_stack_infos(2)[0].value_type.type_id()
                == std::any::TypeId::of::<Callable>() =>
        {
            s.op2(|_: Callable, value: f64| value * 2.0)?;
            Ok(true)
        }
        _ => Ok(false),
    });
    assert_eq!(
        CELParser::new(lookup)
            .parse_str("(p.callable)(4.0)")
            .unwrap()
            .call0::<f64>()
            .unwrap(),
        8.0
    );
}
