//! Contracts for compiling retained syntax without retaining its owner or environments.

use cel_parser::{
    AstContext, CELParser, Expr, OpLookup, Parser, SourceSpan, builtin_type_resolver, compile_expr,
};
use cel_runtime::DynSegment;
use std::{cell::Cell, fmt::Debug, rc::Rc};

/// Parses syntax independently of the environment that later compiles it.
fn ast(source: &str) -> Expr {
    Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast(source)
        .unwrap()
}

/// Declines names that are not external cells.
fn no_external(_: &str, _: &mut DynSegment, _: SourceSpan) -> anyhow::Result<bool> {
    Ok(false)
}

/// Compares both public execution paths against the specified result.
fn compare<T: 'static + PartialEq + Debug>(cases: &[(&str, T)]) {
    let lookup = OpLookup::new();
    let types = builtin_type_resolver();
    for (source, expected) in cases {
        let mut direct = CELParser::new(OpLookup::new()).parse_str(source).unwrap();
        let mut retained =
            compile_expr(&ast(source), &lookup, types.as_ref(), &mut no_external).expect(source);
        assert_eq!(&direct.call0::<T>().unwrap(), expected, "{source}");
        assert_eq!(&retained.call0::<T>().unwrap(), expected, "{source}");
    }
}

/// Every scalar expression form preserves evaluation and lazy branch selection.
#[test]
fn scalar_variants_match_direct_execution() {
    compare(&[
        ("(3i16 as i32) * 4 + -2", 10),
        ("(1, (2, 3)).1.0", 2),
        ("(|x: i32| x + 1)(8)", 9),
        ("(|| |x: i32| x + 2)()(3)", 5),
        ("(|f: Fn((i32,), i32)| f(4))(|x: i32| x + 3)", 7),
        ("if false { 1 / 0 } else { 42 }", 42),
        ("if true { 42 } else { 1 / 0 }", 42),
        ("(|| 7)()", 7),
    ]);
    compare(&[
        ("false && (1i32 / 0i32 == 0i32)", false),
        ("true || (1i32 / 0i32 == 0i32)", true),
        ("true && (3 < 4)", true),
        ("false || (3 < 4)", true),
    ]);
    compare(&[("if false { () }", ()), ("()", ())]);
    compare(&[("1.5f32 as f64 + 0.5", 2.0f64)]);
    compare(&[("\"retained\"", "retained".to_owned())]);
    compare(&[("'x'", 'x')]);
    compare(&[("b\"bytes\"", b"bytes".to_vec())]);
    compare(&[("c\"owned\"", std::ffi::CString::new("owned").unwrap())]);
    compare(&[("1i8", 1i8)]);
    compare(&[("1i16", 1i16)]);
    compare(&[("1i64", 1i64)]);
    compare(&[("1i128", 1i128)]);
    compare(&[("1isize", 1isize)]);
    compare(&[("1u8", 1u8)]);
    compare(&[("1u16", 1u16)]);
    compare(&[("1u32", 1u32)]);
    compare(&[("1u64", 1u64)]);
    compare(&[("1u128", 1u128)]);
    compare(&[("1usize", 1usize)]);
    compare(&[("1.0f32", 1.0f32)]);
}

/// Annotated arrays and callable parameters preserve recursive descriptors.
#[test]
fn arrays_and_higher_order_results_match_direct_execution() {
    use cel_runtime::{ArrayOf, Leaf, OwnedValue};
    for (source, expected) in [
        ("[]: [i32]", vec![]),
        ("[1, 2, 3]: [i32]", vec![1, 2, 3]),
        ("[1, 2]", vec![1, 2]),
        ("(|f: Fn((i32,), [i32])| f(42))(|x: i32| [x])", vec![42]),
    ] {
        for mut segment in [
            CELParser::new(OpLookup::new()).parse_str(source).unwrap(),
            compile_expr(
                &ast(source),
                &OpLookup::new(),
                builtin_type_resolver().as_ref(),
                &mut no_external,
            )
            .unwrap(),
        ] {
            assert_eq!(
                segment
                    .call0::<OwnedValue>()
                    .unwrap()
                    .try_into_typed::<ArrayOf<Leaf<i32>>>()
                    .unwrap()
                    .into_vec(),
                expected,
                "{source}"
            );
        }
    }
}

/// Range operators use the same registered built-in operations.
#[test]
fn ranges_match_direct_execution() {
    compare(&[("1..4", 1i32..4i32)]);
    compare(&[("1..=4", 1i32..=4i32)]);
    compare(&[("1..", 1i32..)]);
    compare(&[("..4", ..4i32)]);
    compare(&[("..=4", ..=4i32)]);
    compare(&[("..", std::ops::RangeFull)]);
}

/// Semantic rejections retain source locations rather than becoming runtime failures.
#[test]
fn invalid_variants_match_direct_rejection() {
    for source in [
        "1 + true",
        "1 as bool",
        "(1,).2",
        "1i32.0",
        "[1, true]",
        "[1]: [bool]",
        "if true { 1 } else { false }",
        "if true { 1 }",
        "1 && true",
        "false || 1",
        "(|x: i32| x)(true)",
        "(|x: Unknown| x)(1)",
        "(|x: i32| || x)(1)",
        "|| missing",
    ] {
        let direct_error = CELParser::new(OpLookup::new())
            .parse_str(source)
            .err()
            .expect(source);
        let expr = ast(source);
        let error = compile_expr(
            &expr,
            &OpLookup::new(),
            builtin_type_resolver().as_ref(),
            &mut no_external,
        )
        .err()
        .expect(source);
        assert_eq!(error.message(), direct_error.message(), "{source}");
        assert_eq!(error.span().start().line, 1, "{source}: {error}");
    }
}

/// External emitters create fresh owners and returned segments outlive the retained syntax.
#[test]
fn repeated_compilation_owns_independent_execution_resources() {
    let expr = ast("counter + (|x: i32| x)(1)");
    let lookup = OpLookup::new();
    let types = builtin_type_resolver();
    let mut resolve = |name: &str, segment: &mut DynSegment, _: SourceSpan| {
        if name != "counter" {
            return Ok(false);
        }
        let counter = Cell::new(0i32);
        segment.op0(move || {
            let next = counter.get().checked_add(1).unwrap();
            counter.set(next);
            next
        });
        Ok(true)
    };
    let mut first = compile_expr(&expr, &lookup, types.as_ref(), &mut resolve).unwrap();
    let mut second = compile_expr(&expr, &lookup, types.as_ref(), &mut resolve).unwrap();
    drop((expr, lookup, types));
    assert_eq!(first.call0::<i32>().unwrap(), 2);
    assert_eq!(first.call0::<i32>().unwrap(), 3);
    assert_eq!(second.call0::<i32>().unwrap(), 2);
}

/// Noncapturing bodies hide external names and transient lookup scopes, but retain libraries.
#[test]
fn closures_isolate_external_and_transient_names() {
    let mut lookup = OpLookup::new();
    lookup.push_library_scope(|name, segment, arity, _| {
        if name == "library" && arity == 0 {
            segment.just(9i32);
            return Ok(true);
        }
        Ok(false)
    });
    lookup.push_scope(|name, segment, arity, _| {
        if name == "outer" && arity == 0 {
            segment.just(5i32);
            return Ok(true);
        }
        Ok(false)
    });
    let calls = Cell::new(0);
    let mut resolve = |_: &str, segment: &mut DynSegment, _: SourceSpan| {
        calls.set(calls.get() + 1);
        segment.just(7i32);
        Ok(true)
    };
    let types = builtin_type_resolver();
    for source in ["|| external", "|| outer", "|x: i32| || x"] {
        assert!(compile_expr(&ast(source), &lookup, types.as_ref(), &mut resolve).is_err());
    }
    let mut segment = compile_expr(
        &ast("(|x: i32| x + library)(1)"),
        &lookup,
        types.as_ref(),
        &mut resolve,
    )
    .unwrap();
    assert_eq!(segment.call0::<i32>().unwrap(), 10);
    assert_eq!(calls.get(), 0);
}

/// Failed compilation releases emitted owners and leaves borrowed environments reusable.
#[test]
fn partial_failure_releases_resources() {
    let owner = Rc::new(());
    let lookup = OpLookup::new();
    let types = builtin_type_resolver();
    let mut resolve = |name: &str, segment: &mut DynSegment, _: SourceSpan| {
        if name == "owned" {
            let owner = owner.clone();
            segment.op0(move || {
                let _ = &owner;
                1i32
            });
            Ok(true)
        } else {
            Ok(false)
        }
    };
    assert!(
        compile_expr(
            &ast("(owned, missing)"),
            &lookup,
            types.as_ref(),
            &mut resolve
        )
        .is_err()
    );
    assert_eq!(Rc::strong_count(&owner), 1);
    let mut segment = compile_expr(&ast("owned"), &lookup, types.as_ref(), &mut resolve).unwrap();
    assert_eq!(segment.call0::<i32>().unwrap(), 1);
    drop(segment);
    assert_eq!(Rc::strong_count(&owner), 1);
}

/// A registered host record retains its nominal identity.
#[derive(Clone)]
struct Record(i32);

/// Registers a constructor and a receiver operation without any cloneable lookup.
fn records() -> OpLookup {
    use cel_parser::{OperationSignature, RecordField, RecordSchema, ResolvedLeafType};
    use cel_runtime::ValueType;
    let record = ResolvedLeafType::new("Record", ValueType::cloneable_leaf::<Record>());
    let integer = ResolvedLeafType::new("i32", ValueType::cloneable_leaf::<i32>());
    let mut lookup = OpLookup::new();
    lookup
        .register_record(RecordSchema {
            ty: record.clone(),
            fields: vec![RecordField {
                name: "value",
                ty: integer.clone().into(),
            }],
            emit: Box::new(|segment, _, _, _| segment.op1(Record)),
        })
        .unwrap();
    lookup
        .register_operation(OperationSignature {
            name: "value",
            operands: vec![record.into()],
            result: integer.into(),
            emit: Box::new(|segment, _| segment.op1(|record: Record| record.0)),
        })
        .unwrap();
    lookup
}

/// Constructors and receiver operations use the borrowed registration environment.
#[test]
fn records_and_members_match_direct_execution() {
    let lookup = records();
    for source in ["Record { value: 42 }.value", "Record { value: 42 }.value()"] {
        let mut direct = CELParser::new(records()).parse_str(source).unwrap();
        let mut retained = compile_expr(
            &ast(source),
            &lookup,
            builtin_type_resolver().as_ref(),
            &mut no_external,
        )
        .unwrap();
        assert_eq!(direct.call0::<i32>().unwrap(), 42);
        assert_eq!(retained.call0::<i32>().unwrap(), 42);
    }
    for source in [
        "Record { value: true }",
        "Record { missing: 1 }",
        "Record { value: 1 }.missing",
        "Unknown { value: 1 }",
    ] {
        assert!(CELParser::new(records()).parse_str(source).is_err());
        assert!(
            compile_expr(
                &ast(source),
                &lookup,
                builtin_type_resolver().as_ref(),
                &mut no_external,
            )
            .is_err()
        );
    }
}

/// External callable values retain complete signatures and are invoked as ordinary locals.
#[test]
fn external_callable_preserves_descriptor_and_span() {
    let expr = ast("\n  function(42)");
    let mut resolve = |name: &str, segment: &mut DynSegment, span: SourceSpan| {
        assert_eq!(name, "function");
        assert_eq!(span, SourceSpan::new(2, 2, 2, 10));
        let mut factory = CELParser::new(OpLookup::new())
            .parse_str("|x: i32| x + 1")
            .unwrap();
        let closure = factory.call0::<cel_runtime::DynClosure>()?;
        segment.push_closure(closure)?;
        Ok(true)
    };
    let mut segment = compile_expr(
        &expr,
        &OpLookup::new(),
        builtin_type_resolver().as_ref(),
        &mut resolve,
    )
    .unwrap();
    assert_eq!(segment.call0::<i32>().unwrap(), 43);
}

/// Custom closure annotations borrow complete host metadata through nested arrays and tuples.
#[test]
fn custom_types_and_recursive_shapes_survive_retention() {
    use cel_parser::{ResolvedLeafType, ResolvedType, resolve_builtin_named_type};
    use cel_runtime::{ArrayOf, Cons, Leaf, Nil, ValueType};
    let types = |name: &str, args: &[ResolvedType]| {
        if name == "Record" && args.is_empty() {
            Some(ResolvedLeafType::new(
                "Record",
                ValueType::cloneable_leaf::<Record>(),
            ))
        } else {
            resolve_builtin_named_type(name, args)
        }
    };
    let lookup = records();
    let source = "(|x: ([Record], (i32,))| x)(([Record { value: 7 }], (42,)))";
    let mut retained = compile_expr(&ast(source), &lookup, &types, &mut no_external).unwrap();
    let mut direct = CELParser::with_type_resolver(records(), types)
        .parse_str(source)
        .unwrap();
    type ResultShape = Cons<ArrayOf<Leaf<Record>>, Cons<Cons<Leaf<i32>, Nil>, Nil>>;
    use cel_runtime::IntoRustTuple;
    for segment in [&mut retained, &mut direct] {
        let result = segment
            .call_dyn_value(&[])
            .unwrap()
            .try_into_typed::<ResultShape>()
            .unwrap()
            .into_rust_tuple();
        assert_eq!(result.0.as_slice()[0].0, 7);
        assert_eq!(result.1.into_rust_tuple(), (42,));
    }
}

/// Runtime arithmetic errors retain the exact span of the retained operation.
#[test]
fn runtime_errors_keep_source_spans() {
    let source = "\n  1i32 / 0i32";
    let expr = ast(source);
    let expected = SourceSpan::from_proc_macro2_range(expr.span().start, expr.span().end);
    let mut retained = compile_expr(
        &expr,
        &OpLookup::new(),
        builtin_type_resolver().as_ref(),
        &mut no_external,
    )
    .unwrap();
    let error = retained.call0::<i32>().unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<cel_parser::SpanContext>()
            .unwrap()
            .span(),
        expected
    );
}

/// Callback failures preserve their identifier location and release already emitted owners.
#[test]
fn external_emission_failure_is_spanned_and_discardable() {
    let owner = Rc::new(());
    let mut resolve = |_: &str, segment: &mut DynSegment, _: SourceSpan| {
        let captured = owner.clone();
        segment.op0(move || {
            let _ = &captured;
            1i32
        });
        anyhow::bail!("external setup failed");
    };
    let error = compile_expr(
        &ast("\n  external"),
        &OpLookup::new(),
        builtin_type_resolver().as_ref(),
        &mut resolve,
    )
    .err()
    .unwrap();
    assert_eq!(error.message(), "external setup failed");
    assert_eq!(
        error.span().start(),
        cel_parser::LineColumn { line: 2, column: 2 }
    );
    assert_eq!(
        error.end_span().unwrap().end(),
        cel_parser::LineColumn {
            line: 2,
            column: 10
        }
    );
    assert_eq!(Rc::strong_count(&owner), 1);
}
