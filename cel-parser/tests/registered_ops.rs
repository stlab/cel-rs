//! Contract tests for persistent named operation registration.

use std::sync::Arc;

use cel_parser::{
    CELParser, OpLookup, OperationSignature, RecordField, RecordSchema, ResolvedLeafType,
};
use cel_runtime::{ArrayElementType, DynSegment};
use proc_macro2::Span;

/// Resolves a complete source shape independently of operation installation.
fn shape(source: &str) -> cel_parser::ResolvedType {
    CELParser::new(OpLookup::new())
        .parse_type_expr_str(source)
        .unwrap()
        .resolve(cel_parser::builtin_type_resolver().as_ref())
        .unwrap()
}

/// Overloads and duplicate detection compare every recursive parameter, not erased carriers.
#[test]
fn complete_shapes_distinguish_overloads_and_reject_alias_duplicates() {
    use cel_runtime::{ArrayOf, Cons, FnOf, IntoCStackList, IntoRustTuple, Leaf, Nil};
    type IntTuple = Cons<Leaf<i32>, Nil>;
    type BoolTuple = Cons<Leaf<bool>, Nil>;
    type IntFn = FnOf<IntTuple, Leaf<i32>>;
    type BoolFn = FnOf<BoolTuple, Leaf<bool>>;
    #[derive(Clone)]
    struct Identity;
    let mut ops = OpLookup::new();
    ops.register_function("identity", Identity).unwrap();
    for (source, emitter) in [
        (
            "[(i32,)]",
            Arc::new(|s: &mut DynSegment, _| {
                s.op1_repr::<ArrayOf<IntTuple>, ArrayOf<IntTuple>, _>(|value| value)
            }) as cel_parser::OperationEmitter,
        ),
        (
            "[(bool,)]",
            Arc::new(|s: &mut DynSegment, _| {
                s.op1_repr::<ArrayOf<BoolTuple>, ArrayOf<BoolTuple>, _>(|value| value)
            }) as cel_parser::OperationEmitter,
        ),
        (
            "Fn((i32,), i32)",
            Arc::new(|s: &mut DynSegment, _| s.op1_repr::<IntFn, IntFn, _>(|value| value))
                as cel_parser::OperationEmitter,
        ),
        (
            "Fn((bool,), bool)",
            Arc::new(|s: &mut DynSegment, _| s.op1_repr::<BoolFn, BoolFn, _>(|value| value))
                as cel_parser::OperationEmitter,
        ),
    ] {
        ops.register_operation(OperationSignature {
            name: "identity",
            operands: vec![shape(source)],
            result: shape(source),
            emit: emitter,
        })
        .unwrap();
    }
    assert_eq!(ops.operation_signatures("identity").count(), 4);
    let alias = cel_parser::ResolvedType::Array {
        element: Box::new(cel_parser::ResolvedType::Tuple {
            elements: vec![ResolvedLeafType::leaf::<i32>("Alias").unwrap().into()],
        }),
    };
    assert!(
        ops.register_operation(OperationSignature {
            name: "identity",
            operands: vec![alias],
            result: shape("String"),
            emit: Arc::new(|_, _| panic!("alias duplicate emitter must never run")),
        })
        .is_err()
    );
    for source in [
        "[(i32,)]",
        "[(bool,)]",
        "Fn((i32,), i32)",
        "Fn((bool,), bool)",
    ] {
        assert!(
            ops.register_operation(OperationSignature {
                name: "identity",
                operands: vec![shape(source)],
                result: shape("String"),
                emit: Arc::new(|_, _| panic!("duplicate emitter must never run")),
            })
            .is_err()
        );
    }
    let mut parser = CELParser::new(ops);
    for source in [
        "identity([(1i32,)])",
        "[(1i32,)].identity",
        "[(1i32,)].identity()",
    ] {
        let body = parser.parse_str(source).unwrap();
        let result = body
            .prepare::<Nil, ArrayOf<IntTuple>>()
            .unwrap()
            .call(&().into_c_stack_list())
            .unwrap();
        assert_eq!(result.as_slice()[0].clone().into_rust_tuple(), (1,));
    }
    for source in [
        "identity(|x: i32| x)(42i32)",
        "(|x: i32| x).identity()(42i32)",
    ] {
        assert_eq!(
            parser.parse_str(source).unwrap().call0::<i32>().unwrap(),
            42
        );
    }
    assert!(parser.parse_str("identity([(1.0f64,)])").is_err());
    assert!(parser.parse_str("identity(|x: i32| true)").is_err());
}

/// A bare named function marker is not mistaken for its nullary operation result.
#[test]
fn nullary_function_values_do_not_infer_their_result_until_called() {
    #[derive(Clone)]
    struct Constant;
    let mut ops = OpLookup::new();
    ops.register_function("constant", Constant).unwrap();
    ops.register_operation(OperationSignature {
        name: "constant",
        operands: vec![],
        result: shape("i32"),
        emit: Arc::new(|segment, _| {
            segment.just(42i32);
            Ok(())
        }),
    })
    .unwrap();
    for (source, expected) in [
        ("constant", cel_parser::Ty::Any),
        ("constant()", cel_parser::Ty::I32),
    ] {
        let expr = cel_parser::Parser::<cel_parser::AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let (ty, diagnostics) = cel_parser::ty::check_expr_with_type_resolver(
            &expr,
            &|_| cel_parser::Ty::Any,
            cel_parser::builtin_type_resolver().as_ref(),
            &ops,
        );
        assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
        assert_eq!(ty, expected, "{source}");
    }
    assert_eq!(
        CELParser::new(ops)
            .parse_str("constant()")
            .unwrap()
            .call0::<i32>()
            .unwrap(),
        42
    );
}

/// Registers an integer function for lexical-shadowing contracts.
fn shadowing_lookup() -> OpLookup {
    /// Marks the registered global function independently of local value types.
    #[derive(Clone)]
    struct F;
    let mut ops = OpLookup::new();
    ops.register_function("f", F).unwrap();
    ops.register_operation(OperationSignature {
        name: "f",
        operands: vec![shape("i32")],
        result: shape("i32"),
        emit: Arc::new(|segment, _| segment.op1(|value: i32| value)),
    })
    .unwrap();
    ops
}

/// Known noncallable locals shadow same-named registered functions in ordinary calls.
#[test]
fn noncallable_locals_shadow_registered_functions() {
    let ops = shadowing_lookup();
    let mut failures = Vec::new();
    for source in [
        "(|f: (i32,)| f(1i32))((2i32,))",
        "(|f: [i32]| f(1i32))([2i32])",
        "(|f: i32| f(1i32))(2i32)",
        "(|f: bool| f(1i32))(true)",
        "(|f: String| f(1i32))(\"local\")",
        "(|f: ()| f(1i32))(())",
    ] {
        assert!(
            CELParser::new(shadowing_lookup())
                .parse_str(source)
                .is_err(),
            "executable parsing must reject {source}"
        );
        let expr = cel_parser::Parser::<cel_parser::AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let (ty, diagnostics) = cel_parser::ty::check_expr_with_type_resolver(
            &expr,
            &|_| cel_parser::Ty::Any,
            cel_parser::builtin_type_resolver().as_ref(),
            &ops,
        );
        if diagnostics.len() != 1 {
            failures.push(format!("{source}: inferred {ty:?}, {diagnostics:?}"));
            continue;
        }
        let error = &diagnostics[0];
        assert!(
            error.message().contains("not callable"),
            "{source}: {error}"
        );
        let start = source.find("f(1i32)").unwrap();
        assert_eq!(error.span().start().column, start, "{source}");
        assert_eq!(
            error.end_span().unwrap().end().column,
            start + "f(1i32)".len(),
            "{source}"
        );
        assert_eq!(ty, cel_parser::Ty::Any, "{source}");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Shadowing does not hide direct members or override a callable local's signature.
#[test]
fn registered_global_member_and_callable_local_resolution() {
    let ops = shadowing_lookup();
    for (source, expected_ty, expected_value) in [
        ("f(1i32)", cel_parser::Ty::I32, 1),
        ("1i32.f()", cel_parser::Ty::I32, 1),
        ("(|f: (i32,)| 1i32.f())((2i32,))", cel_parser::Ty::I32, 1),
        ("(|f: [i32]| 1i32.f)([2i32])", cel_parser::Ty::I32, 1),
    ] {
        let expr = cel_parser::Parser::<cel_parser::AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let (ty, diagnostics) = cel_parser::ty::check_expr_with_type_resolver(
            &expr,
            &|_| cel_parser::Ty::Any,
            cel_parser::builtin_type_resolver().as_ref(),
            &ops,
        );
        assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
        assert_eq!(ty, expected_ty, "{source}");
        assert_eq!(
            CELParser::new(shadowing_lookup())
                .parse_str(source)
                .unwrap()
                .call0::<i32>()
                .unwrap(),
            expected_value,
            "{source}"
        );
    }
    let source = "(|f: Fn((i32,), bool)| f(1i32))(|x: i32| x == 1i32)";
    let expr = cel_parser::Parser::<cel_parser::AstContext>::new(OpLookup::new())
        .parse_str_ast(source)
        .unwrap();
    let (ty, diagnostics) = cel_parser::ty::check_expr_with_type_resolver(
        &expr,
        &|_| cel_parser::Ty::Any,
        cel_parser::builtin_type_resolver().as_ref(),
        &ops,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(ty, cel_parser::Ty::Bool);
    assert!(
        CELParser::new(ops)
            .parse_str(source)
            .unwrap()
            .call0::<bool>()
            .unwrap()
    );
}

/// Identifies the first installer's callable.
#[derive(Clone)]
struct Min;
/// Identifies an independent installer's callable.
#[derive(Clone)]
struct OtherMin;
/// Supplies a custom unary overload.
#[derive(Clone)]
struct TestBox(f64);

/// Describes a concrete host leaf type.
fn leaf<T: 'static>() -> ResolvedLeafType {
    ResolvedLeafType::new(
        std::any::type_name::<T>(),
        ArrayElementType::leaf::<T>().unwrap(),
    )
}

/// Installs the ordinary binary numeric signature.
fn numeric(lookup: &mut OpLookup) {
    lookup.register_function("min", Min).unwrap();
    lookup
        .register_operation(OperationSignature {
            name: "min",
            operands: vec![leaf::<f64>().into(), leaf::<f64>().into()],
            result: leaf::<f64>().into(),
            emit: Arc::new(|s, _| s.op2(f64::min)),
        })
        .unwrap();
}

/// Installs an independently named marker and a unary custom signature.
fn custom(lookup: &mut OpLookup) {
    lookup.register_function("min", OtherMin).unwrap();
    lookup
        .register_operation(OperationSignature {
            name: "min",
            operands: vec![leaf::<TestBox>().into()],
            result: leaf::<f64>().into(),
            emit: Arc::new(|s, _| s.op1(|b: TestBox| b.0)),
        })
        .unwrap();
}

/// Applies one public lookup with a synthetic source span.
fn apply(
    lookup: &OpLookup,
    name: &str,
    s: &mut DynSegment,
    arity: usize,
) -> cel_parser::Result<()> {
    lookup.lookup(name, s, arity, Span::call_site(), Span::call_site())
}

#[test]
fn overloads_from_two_installers_compose() {
    for reverse in [false, true] {
        let mut lookup = OpLookup::new();
        if reverse {
            custom(&mut lookup);
            numeric(&mut lookup);
        } else {
            numeric(&mut lookup);
            custom(&mut lookup);
        }
        let mut s = DynSegment::new::<()>();
        apply(&lookup, "min", &mut s, 0).unwrap();
        let expected_marker = if reverse {
            std::any::TypeId::of::<OtherMin>()
        } else {
            std::any::TypeId::of::<Min>()
        };
        assert_eq!(s.peek_output_type_id(), Some(expected_marker));
        s.just(TestBox(9.0));
        apply(&lookup, "()", &mut s, 2).unwrap();
        assert_eq!(s.call0::<f64>().unwrap(), 9.0);
        let mut parser = CELParser::new(lookup);
        assert_eq!(
            parser
                .parse_str("min(3.0, 7.0)")
                .unwrap()
                .call0::<f64>()
                .unwrap(),
            3.0
        );
    }
}

#[test]
fn value_binding_does_not_hide_direct_operation() {
    let mut lookup = OpLookup::new();
    custom(&mut lookup);
    lookup.push_scope(|name, s, arity, _| {
        if name == "min" && arity == 0 {
            s.just(100u32);
            return Ok(true);
        }
        Ok(false)
    });
    let mut value = DynSegment::new::<()>();
    apply(&lookup, "min", &mut value, 0).unwrap();
    assert_eq!(value.call0::<u32>().unwrap(), 100);
    let mut s = DynSegment::new::<()>();
    s.just(TestBox(4.0));
    lookup
        .lookup_member("min", &mut s, 1, Span::call_site(), Span::call_site())
        .unwrap();
    assert_eq!(s.call0::<f64>().unwrap(), 4.0);
}

#[test]
fn duplicate_operand_signature_is_rejected() {
    let mut lookup = OpLookup::new();
    numeric(&mut lookup);
    assert!(
        lookup
            .register_operation(OperationSignature {
                name: "min",
                operands: vec![leaf::<f64>().into(), leaf::<f64>().into()],
                result: leaf::<u32>().into(),
                emit: Arc::new(|s, _| s.op2(|_: f64, _: f64| 0u32)),
            })
            .is_err()
    );
    assert_eq!(lookup.operation_signatures("min").count(), 1);
}

#[test]
fn marker_cannot_name_two_functions() {
    let mut lookup = OpLookup::new();
    lookup.register_function("first", Min).unwrap();
    assert!(lookup.register_function("second", Min).is_err());
    lookup.register_function("first", OtherMin).unwrap();
    assert!(lookup.register_function("third", OtherMin).is_err());
}

#[test]
fn failed_lookup_preserves_stack() {
    let mut lookup = OpLookup::new();
    numeric(&mut lookup);
    let mut s = DynSegment::new::<()>();
    apply(&lookup, "min", &mut s, 0).unwrap();
    s.just(1u32);
    s.just(2f64);
    let before: Vec<_> = s
        .peek_stack_infos(3)
        .iter()
        .map(|i| i.value_type.type_id())
        .collect();
    assert!(apply(&lookup, "()", &mut s, 3).is_err());
    let after: Vec<_> = s
        .peek_stack_infos(3)
        .iter()
        .map(|i| i.value_type.type_id())
        .collect();
    assert_eq!(before, after);
}

#[test]
fn zero_arg_function_invokes_operation_not_marker() {
    let mut lookup = OpLookup::new();
    lookup.register_function("answer", Min).unwrap();
    lookup
        .register_operation(OperationSignature {
            name: "answer",
            operands: vec![],
            result: leaf::<u32>().into(),
            emit: Arc::new(|s, _| {
                s.just(42u32);
                Ok(())
            }),
        })
        .unwrap();
    let mut parser = CELParser::new(lookup);
    assert_eq!(
        parser
            .parse_str("answer()")
            .unwrap()
            .call0::<u32>()
            .unwrap(),
        42
    );
}

#[test]
fn registered_function_survives_scope_isolation() {
    let mut lookup = OpLookup::new();
    numeric(&mut lookup);
    lookup.push_scope(|_, _, _, _| Ok(false));
    let saved = lookup.isolate_scopes();
    let mut s = DynSegment::new::<()>();
    apply(&lookup, "min", &mut s, 0).unwrap();
    s.just(3.0f64);
    s.just(7.0f64);
    apply(&lookup, "()", &mut s, 3).unwrap();
    assert_eq!(s.call0::<f64>().unwrap(), 3.0);
    lookup.restore_scopes(saved);
}

/// Builds metadata without invoking the record emitter.
fn schema(fields: Vec<RecordField>) -> RecordSchema {
    RecordSchema {
        ty: leaf::<TestBox>(),
        fields,
        emit: Arc::new(|_, _, _, _| panic!("metadata only")),
    }
}

#[test]
fn record_metadata_rejects_duplicates_and_survives_isolation() {
    let mut lookup = OpLookup::new();
    assert!(
        lookup
            .register_record(schema(vec![
                RecordField {
                    name: "corner",
                    ty: leaf::<f64>().into()
                },
                RecordField {
                    name: "corner",
                    ty: leaf::<u32>().into()
                },
            ]))
            .is_err()
    );
    lookup
        .register_record(schema(vec![RecordField {
            name: "corner",
            ty: leaf::<f64>().into(),
        }]))
        .unwrap();
    assert!(lookup.register_record(schema(vec![])).is_err());
    let saved = lookup.isolate_scopes();
    let record = lookup
        .record_schema(std::any::type_name::<TestBox>())
        .unwrap();
    assert_eq!(record.ty, leaf::<TestBox>());
    assert_eq!(record.fields[0].ty, leaf::<f64>().into());
    lookup.restore_scopes(saved);
}

#[test]
fn checked_stack_offset_rejects_underflow_and_preserves_ambient() {
    let mut s = DynSegment::new::<()>();
    assert_eq!(s.stack_offset_before_top(0).unwrap(), 0);
    assert!(s.stack_offset_before_top(1).is_err());
    s.just(1u8);
    let ambient = s.current_stack_offset();
    s.just(2u64);
    assert_eq!(s.stack_offset_before_top(1).unwrap(), ambient);
    assert_eq!(s.stack_offset_before_top(2).unwrap(), 0);
    assert!(s.stack_offset_before_top(3).is_err());
}

/// Distinguishes nested array descriptors that share the erased runtime array marker.
#[test]
fn full_operand_descriptors_control_dispatch() {
    use cel_runtime::{ArrayOf, Leaf};
    let mut lookup = OpLookup::new();
    lookup
        .register_operation(OperationSignature {
            name: "length",
            operands: vec![shape("[f64]")],
            result: leaf::<usize>().into(),
            emit: Arc::new(|s, _| {
                s.op1_repr::<ArrayOf<Leaf<f64>>, Leaf<usize>, _>(|values| values.len())
            }),
        })
        .unwrap();
    let mut wrong = DynSegment::new::<()>();
    wrong.just(10u8);
    let ambient = wrong.current_stack_offset();
    wrong.just(1u32);
    wrong.make_array(1, ambient).unwrap();
    let before: Vec<_> = wrong
        .peek_stack_infos(2)
        .iter()
        .map(|info| info.value_type.clone())
        .collect();
    assert!(
        lookup
            .lookup_member(
                "length",
                &mut wrong,
                1,
                Span::call_site(),
                Span::call_site()
            )
            .is_err()
    );
    assert!(
        before
            .iter()
            .zip(wrong.peek_stack_infos(2))
            .all(|(a, b)| a.same_shape(&b.value_type))
    );
    let mut right = DynSegment::new::<()>();
    right.just(1.0f64);
    right.just(2.0f64);
    right.make_array(2, 0).unwrap();
    lookup
        .lookup_member(
            "length",
            &mut right,
            1,
            Span::call_site(),
            Span::call_site(),
        )
        .unwrap();
    assert_eq!(right.call0::<usize>().unwrap(), 2);
}

/// Preserves concrete host identities even when source names and layouts agree.
#[test]
fn metadata_keeps_distinct_host_type_identities() {
    /// Supplies a distinct host type with the same layout.
    #[derive(Clone)]
    struct OtherBox {
        _corner: f64,
    }
    let mut lookup = OpLookup::new();
    for ty in [leaf::<TestBox>(), leaf::<OtherBox>()] {
        lookup
            .register_operation(OperationSignature {
                name: "corner",
                operands: vec![ResolvedLeafType::new("Box", ty.element_type().clone()).into()],
                result: ty.into(),
                emit: Arc::new(|_, _| Ok(())),
            })
            .unwrap();
    }
    let signatures: Vec<_> = lookup.operation_signatures("corner").collect();
    assert_ne!(
        signatures[0].operands[0]
            .complete_value_type()
            .unwrap()
            .type_id(),
        signatures[1].operands[0]
            .complete_value_type()
            .unwrap()
            .type_id()
    );
    assert_eq!(
        signatures[1]
            .result
            .complete_value_type()
            .unwrap()
            .type_id(),
        leaf::<OtherBox>().type_id()
    );
}

/// Gives transient direct operations precedence without querying a value marker.
#[test]
fn transient_operation_precedes_registration() {
    let mut lookup = OpLookup::new();
    custom(&mut lookup);
    lookup.push_scope(|name, s, arity, _| {
        if name == "min" && arity == 1 {
            s.op1(|_: TestBox| 25.0f64)?;
            return Ok(true);
        }
        Ok(false)
    });
    let mut s = DynSegment::new::<()>();
    s.just(TestBox(4.0));
    lookup
        .lookup_member("min", &mut s, 1, Span::call_site(), Span::call_site())
        .unwrap();
    assert_eq!(s.call0::<f64>().unwrap(), 25.0);
    let mut ordinary = DynSegment::new::<()>();
    apply(&lookup, "min", &mut ordinary, 0).unwrap();
    ordinary.just(TestBox(4.0));
    apply(&lookup, "()", &mut ordinary, 2).unwrap();
    assert_eq!(ordinary.call0::<f64>().unwrap(), 25.0);
}

/// Drops a nonzero-size callee without disturbing an unaligned ambient value or result.
#[test]
fn ordinary_call_preserves_ambient_and_drops_marker() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    /// Counts destruction of a nonzero-sized callable marker.
    #[derive(Clone)]
    struct Marker(Arc<AtomicUsize>);
    impl Drop for Marker {
        /// Counts destruction without changing operation values.
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let mut lookup = OpLookup::new();
    lookup
        .register_function("text", Marker(drops.clone()))
        .unwrap();
    lookup
        .register_operation(OperationSignature {
            name: "text",
            operands: vec![],
            result: leaf::<String>().into(),
            emit: Arc::new(|s, _| {
                s.just(String::from("result"));
                Ok(())
            }),
        })
        .unwrap();
    let mut s = DynSegment::new::<()>();
    s.just(7u8);
    apply(&lookup, "text", &mut s, 0).unwrap();
    apply(&lookup, "()", &mut s, 1).unwrap();
    s.op2(|ambient: u8, result: String| format!("{ambient}:{result}"))
        .unwrap();
    assert_eq!(s.call0::<String>().unwrap(), "7:result");
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

/// Compiles a real closure body with persistent registrations and rejects incompatible callees.
#[test]
fn registered_calls_in_closures_preserve_marker_type_checks() {
    let mut lookup = OpLookup::new();
    numeric(&mut lookup);
    lookup.register_function("other", OtherMin).unwrap();
    lookup
        .register_operation(OperationSignature {
            name: "other",
            operands: vec![leaf::<f64>().into(), leaf::<f64>().into()],
            result: leaf::<f64>().into(),
            emit: Arc::new(|s, _| s.op2(f64::max)),
        })
        .unwrap();
    let mut parser = CELParser::new(lookup);
    let closure: cel_runtime::DynClosure = parser
        .parse_str("|x: f64| min(x, 7.0)")
        .unwrap()
        .call0()
        .unwrap();
    assert_eq!(closure.call::<f64>(&[&3.0f64]).unwrap(), 3.0);
    let error = match parser.parse_str("(if true { min } else { other })(3.0, 7.0)") {
        Ok(_) => panic!("distinct marker types must not join"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("type"), "{error}");
    assert!(
        parser
            .parse_str("|| (if true { min } else { other })(3.0, 7.0)")
            .is_err()
    );
}
