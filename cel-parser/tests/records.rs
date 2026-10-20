//! Contract tests for host-registered records and predictive expression boundaries.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use cel_parser::{
    AstContext, CELParser, Expr, ExpressionRestriction, OpLookup, OperationSignature, Parser,
    RecordField, RecordSchema, ResolvedLeafType, Ty, format_expr,
};
use cel_runtime::ValueType;

/// Stores a recursively typed field as a concrete host value.
#[derive(Clone)]
struct CompoundRecord(<CompoundField as cel_runtime::ValueRepr>::Value);
/// Combines an array of singleton tuples with a callable returning an array.
type CompoundField = cel_runtime::Cons<
    cel_runtime::ArrayOf<cel_runtime::Cons<cel_runtime::Leaf<i32>, cel_runtime::Nil>>,
    cel_runtime::Cons<
        cel_runtime::FnOf<
            cel_runtime::Cons<cel_runtime::Leaf<i32>, cel_runtime::Nil>,
            cel_runtime::ArrayOf<cel_runtime::Leaf<i32>>,
        >,
        cel_runtime::Nil,
    >,
>;

/// Compound fields retain exact shapes through checking, construction, and host use.
#[test]
fn compound_record_fields_preserve_arrays_tuples_and_callables() {
    use cel_runtime::{IntoCStackList, IntoRustTuple, Leaf, Nil};
    let field = CELParser::new(OpLookup::new())
        .parse_type_expr_str("([(i32,)], Fn((i32,), [i32]))")
        .unwrap()
        .resolve(cel_parser::builtin_type_resolver().as_ref())
        .unwrap();
    let effects = Arc::new(AtomicUsize::new(0));
    let observed = effects.clone();
    let mut ops = OpLookup::new();
    ops.register_record(RecordSchema {
        ty: leaf::<CompoundRecord>("Compound"),
        fields: vec![RecordField {
            name: "value",
            ty: field,
        }],
        emit: Arc::new(move |segment, _, _, _| {
            let observed = observed.clone();
            segment.op1_repr::<CompoundField, Leaf<CompoundRecord>, _>(move |value| {
                observed.fetch_add(1, Ordering::SeqCst);
                CompoundRecord(value)
            })
        }),
    })
    .unwrap();
    let source = "Compound { value: ([(1i32,)], |x: i32| [x]) }";
    for source in [
        source,
        "Compound { value: ([(1i32, 2i32)], |x: i32| [x]) }",
        "Compound { value: ([(1i32,)], |x: f64| [x]) }",
        "Compound { value: ([(1i32,)], |x: i32| x) }",
    ] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let (_, diagnostics) = cel_parser::ty::check_expr_with_type_resolver(
            &expr,
            &|_| Ty::Any,
            cel_parser::builtin_type_resolver().as_ref(),
            &ops,
        );
        assert_eq!(
            diagnostics.is_empty(),
            source.contains("[(1i32,)], |x: i32| [x]")
        );
    }
    let mut parser = CELParser::new(ops);
    for source in [
        "Compound { value: ([(1i32, 2i32)], |x: i32| [x]) }",
        "Compound { value: ([(1i32,)], |x: f64| [x]) }",
        "Compound { value: ([(1i32,)], |x: i32| x) }",
    ] {
        assert!(parser.parse_str(source).is_err());
        assert_eq!(effects.load(Ordering::SeqCst), 0);
    }
    let body = parser.parse_str(source).unwrap();
    let record = body
        .prepare::<Nil, Leaf<CompoundRecord>>()
        .unwrap()
        .call(&().into_c_stack_list())
        .unwrap();
    let (tuples, function) = record.0.into_rust_tuple();
    assert_eq!(tuples.as_slice()[0].clone().into_rust_tuple(), (1,));
    assert_eq!(
        function
            .call(&(42,).into_c_stack_list())
            .unwrap()
            .as_slice(),
        &[42]
    );
    assert_eq!(effects.load(Ordering::SeqCst), 1);
}

/// A host record with two independently named scalar fields.
#[derive(Clone, Debug, PartialEq)]
struct TestPoint {
    x: f64,
    y: f64,
}

/// Describes a concrete host type without registering any value binding.
fn leaf<T: 'static>(name: &'static str) -> ResolvedLeafType {
    ResolvedLeafType::new(name, ValueType::leaf::<T>())
}

/// Installs test constructors and an accessor in their own namespaces.
fn lookup() -> OpLookup {
    let mut lookup = OpLookup::new();
    lookup
        .register_record(RecordSchema {
            ty: leaf::<TestPoint>("TestPoint"),
            fields: vec![
                RecordField {
                    name: "x",
                    ty: leaf::<f64>("f64").into(),
                },
                RecordField {
                    name: "y",
                    ty: leaf::<f64>("f64").into(),
                },
            ],
            emit: Arc::new(|s, mapping, ambient, _| {
                assert_eq!(s.stack_offset_before_top(2)?, ambient);
                match mapping {
                    [0, 1] => s.op2(|x: f64, y: f64| TestPoint { x, y }),
                    [1, 0] => s.op2(|y: f64, x: f64| TestPoint { x, y }),
                    _ => panic!("invalid mapping"),
                }
            }),
        })
        .unwrap();
    lookup
        .register_record(RecordSchema {
            ty: leaf::<()>("Empty"),
            fields: vec![],
            emit: Arc::new(|s, mapping, ambient, _| {
                assert!(mapping.is_empty());
                assert_eq!(s.current_stack_offset(), ambient);
                s.just(());
                Ok(())
            }),
        })
        .unwrap();
    lookup
        .register_record(RecordSchema {
            ty: leaf::<(TestPoint,)>("Wrapper"),
            fields: vec![RecordField {
                name: "point",
                ty: leaf::<TestPoint>("TestPoint").into(),
            }],
            emit: Arc::new(|s, _, _, _| s.op1(|point: TestPoint| (point,))),
        })
        .unwrap();
    lookup
        .register_operation(OperationSignature {
            name: "x",
            operands: vec![leaf::<TestPoint>("TestPoint").into()],
            result: leaf::<f64>("f64").into(),
            emit: Arc::new(|s, _| s.op1(|point: TestPoint| point.x)),
        })
        .unwrap();
    lookup.push_scope(|name, s, arity, _| {
        if arity != 0 {
            return Ok(false);
        }
        match name {
            "TestPoint" => s.just(42i32),
            "flag" => s.just(true),
            _ => return Ok(false),
        }
        Ok(true)
    });
    lookup
}

/// Checks metadata without compiling or executing the expression.
fn checked(source: &str) -> (Ty, Vec<cel_parser::ParseError>) {
    let expr = Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast(source)
        .unwrap();
    cel_parser::ty::check_expr_with_type_resolver(
        &expr,
        &|_| Ty::Any,
        cel_parser::builtin_type_resolver().as_ref(),
        &lookup(),
    )
}

/// Rejects known constructor fields and accessor results with incompatible types.
#[test]
fn known_record_member_types_are_checked() {
    assert_eq!(checked("TestPoint { x: 1.0, y: 2.0 }.x").0, Ty::F64);
    for source in [
        "TestPoint { x: 1, y: 2.0 }",
        "TestPoint { x: 1.0 }",
        "TestPoint { x: 1.0, x: 2.0, y: 3.0 }",
        "TestPoint { x: 1.0, z: 2.0 }",
        "TestPoint { x: 1.0, y: 2.0 }.x + true",
        "TestPoint { x: 1.0, y: 2.0 }.x(2.0)",
        "TestPoint { x: 'a', y: 2.0 }",
        "TestPoint { x: (), y: 2.0 }",
        "TestPoint { x: (1.0,), y: 2.0 }",
        "'a'.x",
        "(1.0,).x",
    ] {
        assert!(!checked(source).1.is_empty(), "{source}");
    }
    assert_eq!(
        checked("TestPoint { x: [1]: [f64], y: 2.0 }.x + true")
            .1
            .len(),
        1,
        "an invalid field reports its own error without cascading outer mismatches"
    );
}

/// Reports unknown constructors while retaining nested child diagnostics.
#[test]
fn unknown_record_type_is_diagnostic() {
    let (_, diagnostics) = checked("Missing { x: true + 1 }");
    assert_eq!(diagnostics.len(), 2);
    assert!(diagnostics.iter().any(|d| d.message().contains("Missing")));
}

/// Preserves unrelated registered identities through annotated arrays and constructors.
#[test]
fn different_registered_types_do_not_unify() {
    assert_eq!(
        checked("[TestPoint { x: 1.0, y: 2.0 }, Empty {}]").1.len(),
        1
    );
}

/// Still checks arguments of unresolved members and custom calls.
#[test]
fn unresolved_member_checks_its_argument_expressions() {
    assert_eq!(checked("unknown.custom(true + 1)").1.len(), 1);
    assert_eq!(checked("custom(true + 1)").1.len(), 1);
    assert!(checked("unknown.custom(1)").1.is_empty());
}

/// Verifies field names, not their written positions, select constructor arguments.
#[test]
fn record_field_order_is_named() {
    for source in [
        "TestPoint { x: 1.0, y: 2.0 }",
        "TestPoint { y: 2.0, x: 1.0 }",
    ] {
        assert_eq!(
            CELParser::new(lookup())
                .parse_str(source)
                .unwrap()
                .call0::<TestPoint>()
                .unwrap(),
            TestPoint { x: 1.0, y: 2.0 }
        );
    }

    assert_eq!(
        CELParser::new(lookup())
            .parse_str("(7u8, TestPoint { y: 2.0, x: 1.0 }.x).1 + 3.0")
            .unwrap()
            .call0::<f64>()
            .unwrap(),
        4.0
    );
}

/// Gives a registered signature the same precedence over built-ins as runtime dispatch.
#[test]
fn registered_operator_results_precede_builtin_inference() {
    let mut ops = lookup();
    ops.register_operation(OperationSignature {
        name: "+",
        operands: vec![leaf::<i32>("i32").into(), leaf::<i32>("i32").into()],
        result: leaf::<f64>("f64").into(),
        emit: Arc::new(|s, _| s.op2(|a: i32, b: i32| f64::from(a) + f64::from(b))),
    })
    .unwrap();
    for source in ["1 + 2", "1.0 + 2.0"] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let (ty, diagnostics) = cel_parser::ty::check_expr_with_type_resolver(
            &expr,
            &|_| Ty::Any,
            cel_parser::builtin_type_resolver().as_ref(),
            &ops,
        );
        assert_eq!(ty, Ty::F64, "{source}");
        assert!(diagnostics.is_empty());
    }
    assert_eq!(
        CELParser::new(ops)
            .parse_str("1 + 2")
            .unwrap()
            .call0::<f64>()
            .unwrap(),
        3.0
    );
}

/// Leaves a member result unresolved when an array operand still has unknown elements.
#[test]
fn unresolved_array_operands_do_not_select_known_results() {
    let mut ops = lookup();
    ops.register_operation(OperationSignature {
        name: "length",
        operands: vec![cel_parser::ResolvedType::Array {
            element: Box::new(leaf::<i32>("i32").into()),
        }],
        result: leaf::<usize>("usize").into(),
        emit: Arc::new(|s, _| {
            s.op1_repr::<cel_runtime::ArrayOf<cel_runtime::Leaf<i32>>, cel_runtime::Leaf<usize>, _>(
                |array| array.len(),
            )
        }),
    })
    .unwrap();
    for (source, expected) in [("[custom()].length", Ty::Any), ("[1].length", Ty::Usize)] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let (ty, diagnostics) = cel_parser::ty::check_expr_with_type_resolver(
            &expr,
            &|_| Ty::Any,
            cel_parser::builtin_type_resolver().as_ref(),
            &ops,
        );
        assert_eq!(ty, expected, "{source}");
        assert!(diagnostics.is_empty());
    }
    assert_eq!(
        CELParser::new(ops)
            .parse_str("[1, 2].length")
            .unwrap()
            .call0::<usize>()
            .unwrap(),
        2
    );
}

/// Composes built-in and registered overloads without diagnosing viable unresolved operands.
#[test]
fn mixed_operator_overloads_preserve_unresolved_operands() {
    let mut ops = lookup();
    ops.register_operation(OperationSignature {
        name: "+",
        operands: vec![
            leaf::<TestPoint>("TestPoint").into(),
            leaf::<TestPoint>("TestPoint").into(),
        ],
        result: leaf::<TestPoint>("TestPoint").into(),
        emit: Arc::new(|s, _| {
            s.op2(|a: TestPoint, b: TestPoint| TestPoint {
                x: a.x + b.x,
                y: a.y + b.y,
            })
        }),
    })
    .unwrap();
    for (source, expected) in [
        ("unknown + 1", Ty::Any),
        ("1 + unknown", Ty::Any),
        ("unknown + unknown", Ty::Any),
        ("unknown + TestPoint { x: 1.0, y: 2.0 }", Ty::Any),
        ("TestPoint { x: 1.0, y: 2.0 } + unknown", Ty::Any),
        ("1 + 2", Ty::I32),
        (
            "TestPoint { x: 1.0, y: 2.0 } + TestPoint { x: 3.0, y: 4.0 }",
            Ty::Registered(leaf::<TestPoint>("TestPoint")),
        ),
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
        assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
        assert_eq!(ty, expected, "{source}");
    }
}

/// Rejects known incompatible operands even when another operand remains unresolved.
#[test]
fn mixed_operator_overloads_reject_known_incompatible_operands() {
    let mut registered = lookup();
    registered
        .register_operation(OperationSignature {
            name: "+",
            operands: vec![
                leaf::<TestPoint>("TestPoint").into(),
                leaf::<TestPoint>("TestPoint").into(),
            ],
            result: leaf::<TestPoint>("TestPoint").into(),
            emit: Arc::new(|s, _| s.op2(|a: TestPoint, _: TestPoint| a)),
        })
        .unwrap();
    for ops in [lookup(), registered] {
        for source in [
            "unknown + true",
            "true + unknown",
            "1 + true",
            "TestPoint { x: 1.0, y: 2.0 } + 1",
            "1 + TestPoint { x: 1.0, y: 2.0 }",
            "TestPoint { x: 1.0, y: 2.0 } + Empty {}",
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
            assert_eq!(ty, Ty::Any, "{source}");
            assert_eq!(diagnostics.len(), 1, "{source}: {diagnostics:?}");
        }
    }
}

/// Verifies malformed field sets and unknown constructors have useful diagnostics.
#[test]
fn record_missing_duplicate_unknown_fields_are_errors() {
    for (source, message) in [
        ("TestPoint { x: 1.0 }", "missing"),
        ("TestPoint { x: 1.0, x: 2.0, y: 3.0 }", "duplicate"),
        ("TestPoint { x: 1.0, z: 2.0 }", "unknown"),
        ("Unknown { x: 1.0 }", "record"),
    ] {
        let error = CELParser::new(lookup())
            .parse_str(source)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains(message), "{source}: {error}");
    }
}

/// Verifies field descriptors do not implicitly widen integers or floats.
#[test]
fn record_scalar_types_are_exact() {
    for value in ["1", "1.0f32", "true", "(1.0,)"] {
        let source = format!("TestPoint {{ x: {value}, y: 2.0 }}");
        let error = CELParser::new(lookup())
            .parse_str(&source)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("type"), "{error}");
    }
}

/// Verifies nested constructors, trailing commas, and empty schemas.
#[test]
fn nested_record_and_trailing_comma() {
    let value = CELParser::new(lookup())
        .parse_str("Wrapper { point: TestPoint { y: 2.0, x: 1.0, }, }")
        .unwrap()
        .call0::<(TestPoint,)>()
        .unwrap();
    assert_eq!(value.0, TestPoint { x: 1.0, y: 2.0 });
    CELParser::new(lookup())
        .parse_str("(7u8, Empty {}).1")
        .unwrap()
        .call0::<()>()
        .unwrap();
    for source in [
        "TestPoint { x 1.0, y: 2.0 }",
        "TestPoint { x: 1.0 y: 2.0 }",
        "TestPoint { x: , y: 2.0 }",
    ] {
        assert!(CELParser::new(lookup()).parse_str(source).is_err());
    }
}

/// Verifies constructor lookup never resolves a same-named value or closure local.
#[test]
fn record_type_namespace_ignores_value_shadowing() {
    let mut parser = CELParser::new(lookup());
    assert_eq!(
        parser
            .parse_str("TestPoint")
            .unwrap()
            .call0::<i32>()
            .unwrap(),
        42
    );
    let closure = parser
        .parse_str("|TestPoint: i32| TestPoint { x: 1.0, y: 2.0 }.x")
        .unwrap()
        .call0::<cel_runtime::DynClosure>()
        .unwrap();
    assert_eq!(closure.call::<f64>(&[&99i32]).unwrap(), 1.0);
}

/// Tracks lifetime of a nontrivially aligned constructor field.
#[repr(align(64))]
struct Aligned(Arc<AtomicUsize>);

impl Drop for Aligned {
    /// Records each destroyed runtime value.
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// Retains both heterogeneous fields without cloning them during construction.
struct Owned {
    text: String,
    aligned: Aligned,
}

/// Verifies original-order heterogeneous consumption, evaluation, and failure cleanup.
#[test]
fn heterogeneous_source_order_drops_once() {
    for fail in [false, true] {
        let drops = Arc::new(AtomicUsize::new(0));
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut ops = lookup();
        ops.register_record(RecordSchema {
            ty: leaf::<Owned>("Owned"),
            fields: vec![
                RecordField {
                    name: "text",
                    ty: leaf::<String>("String").into(),
                },
                RecordField {
                    name: "aligned",
                    ty: leaf::<Aligned>("Aligned").into(),
                },
            ],
            emit: Arc::new(|s, mapping, ambient, _| {
                assert_eq!(mapping, [1, 0]);
                assert_eq!(s.stack_offset_before_top(2)?, ambient);
                s.op2(|aligned: Aligned, text: String| Owned { text, aligned })
            }),
        })
        .unwrap();
        let recorded = trace.clone();
        let count = drops.clone();
        ops.push_scope(move |name, s, arity, _| {
            if arity != 0 {
                return Ok(false);
            }
            let recorded = recorded.clone();
            match name {
                "aligned" => {
                    let count = count.clone();
                    s.op0(move || {
                        recorded.lock().unwrap().push("aligned");
                        Aligned(count.clone())
                    });
                }
                "text" => s.op0r(move || -> anyhow::Result<String> {
                    recorded.lock().unwrap().push("text");
                    anyhow::ensure!(!fail, "second field failed");
                    Ok("owned".to_string())
                }),
                _ => return Ok(false),
            }
            Ok(true)
        });
        let mut segment = CELParser::new(ops)
            .parse_str("(9u8, Owned { aligned: aligned, text: text }).1")
            .unwrap();
        assert!(trace.lock().unwrap().is_empty());
        let result = segment.call0::<Owned>();
        if fail {
            assert!(result.is_err());
        } else {
            let value = result.unwrap();
            assert_eq!(value.text, "owned");
            assert_eq!(value.aligned.0.load(Ordering::SeqCst), 0);
            drop(value);
        }
        assert_eq!(*trace.lock().unwrap(), ["aligned", "text"]);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        drop(segment);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

/// Verifies identifiers directly before body braces remain ordinary conditions.
#[test]
fn if_identifier_condition_keeps_its_body() {
    for source in [
        "if flag { 1 } else { 2 }",
        "if true { 1 } else { 2 }",
        "if !false && flag { 1 } else { 2 }",
    ] {
        assert_eq!(
            CELParser::new(lookup())
                .parse_str(source)
                .unwrap()
                .call0::<i32>()
                .unwrap(),
            1
        );
    }
}

/// Verifies explicit grouping permits records in restricted headers and survives formatting.
#[test]
fn parenthesized_record_in_header_is_allowed() {
    let source = "if (TestPoint { x: 0.0, y: 1.0 }).x == 0.0 { 1 } else { 2 }";
    assert_eq!(
        CELParser::new(lookup())
            .parse_str(source)
            .unwrap()
            .call0::<i32>()
            .unwrap(),
        1
    );
    let ast = Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast(source)
        .unwrap();
    let formatted = format_expr(&ast, source, 0);
    assert_eq!(
        CELParser::new(lookup())
            .parse_str(&formatted)
            .unwrap()
            .call0::<i32>()
            .unwrap(),
        1,
        "{formatted}"
    );
    assert!(
        CELParser::new(lookup())
            .parse_str("if TestPoint { x: 0.0, y: 1.0 }.x == 0.0 { 1 } else { 2 }")
            .is_err()
    );
}

/// Verifies omitted header groups preserve comments, evaluation, and formatting idempotence.
#[test]
fn grouped_header_comments_round_trip() {
    for (source, comments) in [
        (
            "if (/* keep */ TestPoint { x: 0.0, y: 1.0 }).x == 0.0 { 1 } else { 2 }",
            &["/* keep */"][..],
        ),
        (
            "if (/* ordinary */ true /* tail */) { 1 } else { 2 }",
            &["/* ordinary */", "/* tail */"][..],
        ),
        (
            "if true && (true /* operand tail */) { 1 } else { 2 }",
            &["/* operand tail */"][..],
        ),
        (
            "if (/* outer /* nested */ end */ (// inner\n true /* inner tail */) // outer tail\n) { 1 } else { 2 }",
            &[
                "/* outer /* nested */ end */",
                "// inner",
                "/* inner tail */",
                "// outer tail",
            ][..],
        ),
        (
            "if (// record\n (/* nested record */ TestPoint { x: 0.0, y: 1.0 }).x == 0.0 /* record tail */) { 1 } else { 2 }",
            &["// record", "/* nested record */", "/* record tail */"][..],
        ),
        (
            "if (/* precedence */ 1 + 2) * 3 == 9 { 1 } else { 2 }",
            &["/* precedence */"][..],
        ),
    ] {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let ast = parser.parse_str_ast(source).unwrap();
        let text = format_expr(&ast, source, 0);
        let mut previous = 0;
        for comment in comments {
            assert_eq!(text.matches(comment).count(), 1, "{source}\n{text}");
            let position = text.find(comment).unwrap();
            assert!(position >= previous, "{text}");
            previous = position + comment.len();
        }
        let reparsed = parser.parse_str_ast(&text).unwrap();
        assert_eq!(format_expr(&reparsed, &text, 0), text);
        for expression in [source, text.as_str()] {
            assert_eq!(
                CELParser::new(lookup())
                    .parse_str(expression)
                    .unwrap()
                    .call0::<i32>()
                    .unwrap(),
                1,
                "{expression}"
            );
        }
    }
}

/// Verifies branch bodies lift the enclosing header restriction.
#[test]
fn record_in_branch_body_is_allowed() {
    let source = "if if flag { TestPoint { x: 0.0, y: 1.0 }.x == 0.0 } else { false } { TestPoint { x: 3.0, y: 4.0 } } else { TestPoint { x: 5.0, y: 6.0 } }";
    assert_eq!(
        CELParser::new(lookup())
            .parse_str(source)
            .unwrap()
            .call0::<TestPoint>()
            .unwrap()
            .x,
        3.0
    );
}

/// Verifies a failed restricted condition cannot poison the next parse.
#[test]
fn restriction_does_not_leak_after_parse_error() {
    let mut parser = CELParser::new(lookup());
    assert!(parser.parse_str("if ! { 1 } else { 2 }").is_err());
    assert_eq!(
        parser
            .parse_str("TestPoint { x: 1.0, y: 2.0 }")
            .unwrap()
            .call0::<TestPoint>()
            .unwrap()
            .x,
        1.0
    );
    parser.set_tokens(
        "!1.0 TestPoint { x: 1.0, y: 2.0 }"
            .parse::<proc_macro2::TokenStream>()
            .unwrap()
            .into_iter(),
    );
    assert!(
        parser
            .parse_expression_ctx_restricted(ExpressionRestriction::NoStructLiteral)
            .is_err()
    );
    assert!(parser.is_expression().unwrap());
    assert!(parser.take_lex_tokens().unwrap().next().is_none());
    parser.set_tokens(
        "true TestPoint { x: 1.0, y: 2.0 }"
            .parse::<proc_macro2::TokenStream>()
            .unwrap()
            .into_iter(),
    );
    parser
        .parse_expression_ctx_restricted(ExpressionRestriction::NoStructLiteral)
        .unwrap();
    assert!(parser.is_expression().unwrap());
    assert!(parser.take_lex_tokens().unwrap().next().is_none());
}

/// Verifies invalid records never invoke a constructor, while field compilation stays streaming.
#[test]
fn invalid_field_sets_do_not_invoke_constructor() {
    let emitted = Arc::new(AtomicUsize::new(0));
    let compiled = Arc::new(Mutex::new(Vec::new()));
    let mut ops = lookup();
    let count = emitted.clone();
    ops.register_record(RecordSchema {
        ty: leaf::<()>("Checked"),
        fields: vec![RecordField {
            name: "x",
            ty: leaf::<f64>("f64").into(),
        }],
        emit: Arc::new(move |s, _, _, _| {
            count.fetch_add(1, Ordering::SeqCst);
            s.op1(|_: f64| ())
        }),
    })
    .unwrap();
    let recorded = compiled.clone();
    ops.push_scope(move |name, s, arity, _| {
        if arity == 0 && ["first", "second"].contains(&name) {
            recorded.lock().unwrap().push(name.to_string());
            s.just(1.0f64);
            Ok(true)
        } else {
            Ok(false)
        }
    });
    let mut parser = CELParser::new(ops);
    for source in [
        "Checked {}",
        "Checked { x: first, x: second }",
        "Checked { unknown: first }",
        "Checked { x: 1i32 }",
    ] {
        assert!(parser.parse_str(source).is_err());
        assert_eq!(emitted.load(Ordering::SeqCst), 0);
    }
    assert_eq!(*compiled.lock().unwrap(), ["first", "second", "first"]);
    parser
        .parse_str("Checked { x: 1.0 }")
        .unwrap()
        .call0::<()>()
        .unwrap();
    assert_eq!(emitted.load(Ordering::SeqCst), 1);
}

/// Verifies constructor setup errors retain the complete constructor's source range.
#[test]
fn record_emitter_errors_keep_constructor_span() {
    let mut ops = lookup();
    ops.register_record(RecordSchema {
        ty: leaf::<()>("Failure"),
        fields: vec![],
        emit: Arc::new(|_, _, _, _| anyhow::bail!("constructor setup failed")),
    })
    .unwrap();
    let source = "Failure {}";
    let error = CELParser::new(ops).parse_str(source).err().unwrap();
    assert!(error.message().contains("constructor setup failed"));
    let error: cel_parser::CELError = error.into();
    assert_eq!(error.span().start.column, 0);
    assert_eq!(error.span().end.column, source.len());
}

/// Verifies source names, field order, and individual label spans survive AST construction.
#[test]
fn record_ast_preserves_names_order_and_spans() {
    let source = "TestPoint { y: other, x: input }";
    let expr = Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast(source)
        .unwrap();
    let Expr::Struct {
        name,
        name_span,
        fields,
        span,
    } = &expr
    else {
        panic!("expected record");
    };
    assert_eq!(name, "TestPoint");
    assert_eq!(name_span.start.source_text().as_deref(), Some("TestPoint"));
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["y", "x"]
    );
    for field in fields {
        assert_eq!(
            field.name_span.start.source_text().as_deref(),
            Some(field.name.as_str())
        );
    }
    assert_eq!(span.start.start().column, 0);
    assert_eq!(span.end.end().column, source.len());
    let referenced = Mutex::new(Vec::new());
    let (ty, diagnostics) = cel_parser::ty::check_expr(&expr, &|name| {
        referenced.lock().unwrap().push(name.to_string());
        Ty::F64
    });
    assert_eq!(ty, Ty::Any);
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].message().contains("unknown record type"));
    assert_eq!(*referenced.lock().unwrap(), ["other", "input"]);
    let bad = Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast("Record { x: 1 + true }")
        .unwrap();
    assert!(!cel_parser::ty::check_expr(&bad, &|_| Ty::Any).1.is_empty());
}

/// Verifies erased array markers cannot bypass a record field's recursive descriptor.
#[test]
fn record_array_descriptor_identity_is_exact() {
    let mut ops = lookup();
    ops.register_record(RecordSchema {
        ty: leaf::<()>("ArrayRecord"),
        fields: vec![RecordField {
            name: "values",
            ty: cel_parser::ResolvedType::Array {
                element: Box::new(leaf::<i32>("i32").into()),
            },
        }],
        emit: Arc::new(|s, _, _, _| {
            s.op1_repr::<cel_runtime::ArrayOf<cel_runtime::Leaf<i32>>, cel_runtime::Leaf<()>, _>(
                |_| (),
            )
        }),
    })
    .unwrap();
    for (source, errors) in [
        ("ArrayRecord { values: []: [i32] }", 0),
        ("ArrayRecord { values: []: [f64] }", 1),
        ("ArrayRecord { values: []: [[i32]] }", 1),
    ] {
        let expr = Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let (_, diagnostics) = cel_parser::ty::check_expr_with_type_resolver(
            &expr,
            &|_| Ty::Any,
            cel_parser::builtin_type_resolver().as_ref(),
            &ops,
        );
        assert_eq!(diagnostics.len(), errors, "{source}: {diagnostics:?}");
    }
    let mut parser = CELParser::new(ops);
    parser
        .parse_str("ArrayRecord { values: []: [i32] }")
        .unwrap()
        .call0::<()>()
        .unwrap();
    for source in [
        "ArrayRecord { values: []: [f64] }",
        "ArrayRecord { values: []: [[i32]] }",
    ] {
        assert!(
            parser
                .parse_str(source)
                .err()
                .unwrap()
                .to_string()
                .contains("type")
        );
    }
}

/// Verifies record syntax and comments format idempotently across every field boundary.
#[test]
fn record_formatting_preserves_comments_and_order() {
    for source in [
        "TestPoint { y: 2.0, x: 1.0, }",
        "Empty { /* empty */ }",
        "TestPoint /* name */ { /* open */ y /* label */ : /* value */ 2.0, /* next */ x: 1.0, /* close */ }",
        "TestPoint { y: 2.0, // next\n x: 1.0, // close\n }",
        "TestPoint { x: (1.0 + 2.0), /* tail */ y: 4.0 }",
        "TestPoint { x: ( /* open */ 1.0), y: ((2.0)) /* close */ }",
    ] {
        let mut parser = Parser::<AstContext>::new(OpLookup::new());
        let expr = parser.parse_str_ast(source).unwrap();
        let text = format_expr(&expr, source, 0);
        let reparsed = parser.parse_str_ast(&text).unwrap();
        assert_eq!(format_expr(&reparsed, &text, 0), text);
        for label in [
            "name", "open", "label", "value", "next", "close", "empty", "tail",
        ] {
            if source.contains(&format!("/* {label} */")) || source.contains(&format!("// {label}"))
            {
                assert_eq!(text.matches(label).count(), 1, "{text}");
            }
        }
    }
}

/// Verifies a lifted group restores restrictions before a later header operand.
#[test]
fn header_restriction_inherits_and_restores_at_boundaries() {
    let mut parser = Parser::<AstContext>::new(OpLookup::new());
    for source in [
        "if (TestPoint { x: 0.0, y: 1.0 }).x == flag { 1 } else { 2 }",
        "if flag && other { 1 } else { 2 }",
        "if !flag { 1 } else { 2 }",
        "if f(TestPoint { x: 0.0, y: 1.0 }) { 1 } else { 2 }",
        "if [TestPoint { x: 0.0, y: 1.0 }] { 1 } else { 2 }",
        "if |x: bool| x { 1 } else { 2 }",
    ] {
        let expr = parser.parse_str_ast(source).unwrap();
        let text = format_expr(&expr, source, 0);
        parser.parse_str_ast(&text).unwrap();
    }
    // The enclosing grammar must receive the body's opening brace untouched.
    parser.set_tokens(
        "flag {}"
            .parse::<proc_macro2::TokenStream>()
            .unwrap()
            .into_iter(),
    );
    parser
        .parse_expression_ctx_restricted(ExpressionRestriction::NoStructLiteral)
        .unwrap();
    let mut remaining = parser.take_lex_tokens().unwrap();
    assert!(matches!(
        remaining.next(),
        Some(cel_parser::lex_lexer::Token::OpenDelim {
            delimiter: proc_macro2::Delimiter::Brace,
            ..
        })
    ));
}
