//! Contracts for checked, noncapturing CEL closure invocation.

use cel_parser::{CELParser, OpLookup, ResolvedLeafType, resolve_builtin_named_type};
use cel_runtime::ArrayElementType;

/// A registered, owning leaf with stricter alignment than ordinary scalars.
#[derive(Clone, Debug, PartialEq)]
#[repr(align(64))]
struct Parcel(String);

/// Builds an environment with a registered owning leaf and its constructor.
fn parser() -> CELParser {
    let mut lookup = OpLookup::new();
    lookup.push_library_scope(|name, segment, arity, _| {
        if matches!(name, "first" | "last") && arity == 0 {
            segment.just(Parcel(name.to_string()));
            return Ok(true);
        }
        Ok(false)
    });
    CELParser::with_type_resolver(lookup, |name: &str, args: &[cel_parser::ResolvedType]| {
        if name == "Parcel" && args.is_empty() {
            Some(ResolvedLeafType::new(
                "Parcel",
                ArrayElementType::cloneable::<Parcel>().unwrap(),
            ))
        } else {
            resolve_builtin_named_type(name, args)
        }
    })
}

/// A scalar closure needs no host call operation.
#[test]
fn scalar_immediate_invocation() {
    let mut segment = parser().parse_str("(|x: f64| x)(1.0)").unwrap();
    assert_eq!(segment.call0::<f64>().unwrap(), 1.0);
}

/// Registered annotations use the configured resolver for Rust-callable closure values.
#[test]
fn registered_leaf_annotation() {
    let mut segment = parser().parse_str("|x: Parcel| x").unwrap();
    let closure: cel_runtime::DynClosure = segment.call0().unwrap();
    let input = Parcel("owned".into());
    assert_eq!(closure.call::<Parcel>(&[&input]).unwrap(), input);
}

/// Recursive tuple intermediates preserve registered leaves and nested callable results.
#[test]
fn registered_recursive_tuple_and_nested_invocation() {
    let mut segment = parser()
        .parse_str(
            "(|x: (Parcel, (i32, Parcel))| \
             (|y: (Parcel, (i32, Parcel))| y)(x))\
             ((first, (7, last)))",
        )
        .unwrap();
    let result = segment
        .call_dyn_as_dynamic_sequence(&[], &|id| {
            if id == std::any::TypeId::of::<Parcel>() {
                Some((
                    cel_runtime::element_dropper_for::<Parcel>(),
                    cel_runtime::element_cloner_for::<Parcel>(),
                    cel_runtime::element_eq_for::<Parcel>(),
                    cel_runtime::element_debug_for::<Parcel>(),
                ))
            } else if id == std::any::TypeId::of::<i32>() {
                Some((
                    cel_runtime::element_dropper_for::<i32>(),
                    cel_runtime::element_cloner_for::<i32>(),
                    cel_runtime::element_eq_for::<i32>(),
                    cel_runtime::element_debug_for::<i32>(),
                ))
            } else {
                None
            }
        })
        .unwrap();
    let (first, nested): (Parcel, cel_runtime::DynamicSequence) = result.try_into_tuple().unwrap();
    assert_eq!(first, Parcel("first".into()));
    assert_eq!(
        nested.try_into_tuple::<(i32, Parcel)>().unwrap(),
        (7, Parcel("last".into()))
    );
}

/// Array annotations retain recursive element shapes rather than only the array marker.
#[test]
fn recursive_arrays_cross_invocation_boundaries() {
    let mut segment = parser()
        .parse_str("(|x: [[Parcel]]| (|y: [[Parcel]]| y)(x))([[first], [last]])")
        .unwrap();
    let rows = segment
        .call0::<cel_runtime::DynamicArray>()
        .unwrap()
        .try_into_vec::<cel_runtime::DynamicArray>()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].try_as_slice::<Parcel>().unwrap(),
        &[Parcel("first".into())]
    );
    assert_eq!(
        rows[1].try_as_slice::<Parcel>().unwrap(),
        &[Parcel("last".into())]
    );
}

/// Nullary and higher-order literal results retain their known signatures.
#[test]
fn nullary_and_returned_closure_calls() {
    parser()
        .parse_str("(|| ())()")
        .unwrap()
        .call0::<()>()
        .unwrap();
    for (source, expected) in [
        ("(|| 17)()", 17),
        ("(|| |x: i32| x + 2)()(3)", 5),
        ("((|| (|x: i32| x, 9))()).0(4)", 4),
        ("(if true { |x: i32| x + 1 } else { |y: i32| y + 2 })(3)", 4),
    ] {
        let mut segment = parser().parse_str(source).unwrap();
        assert_eq!(segment.call0::<i32>().unwrap(), expected, "{source}");
    }
}

/// Compile-time signature checks reject incompatible calls without evaluating arguments.
#[test]
fn rejects_incompatible_signatures() {
    for source in [
        "(|x: i32| x)()",
        "(|x: i32| x)(1, 2)",
        "(|x: i32| x)(true)",
        "(|x: (i32, bool)| x)((true, 1))",
        "(|x: [[i32]]| x)([[true]])",
        "(|x: [i32]| x)([[1]])",
        "(|x: [(i32, i32)]| x)([])",
        "(if true { |x: i32| x } else { |x: bool| x })(1)",
        "(if false { || [1] } else { || [true] })()",
        "(if true { || (1, true) } else { || (true, 1) })()",
    ] {
        assert!(parser().parse_str(source).is_err(), "{source}");
    }
}

/// Nested invocation restores the outer parameter frame before subsequent parameter reads.
#[test]
fn arguments_evaluate_once_in_source_order_and_restore_outer_frame() {
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut lookup = OpLookup::new();
    let captured = events.clone();
    lookup.push_library_scope(move |name, segment, arity, _| {
        if arity == 0 && matches!(name, "a" | "b" | "c") {
            let name = name.to_owned();
            let events = captured.clone();
            segment.op0(move || {
                events.lock().unwrap().push(name.clone());
                1i32
            });
            return Ok(true);
        }
        Ok(false)
    });
    let mut segment = CELParser::new(lookup)
        .parse_str("(|x: i32, y: i32, z: i32| (|n: i32| n)(x) + y + x + z)(a, b, c)")
        .unwrap();
    assert_eq!(segment.call0::<i32>().unwrap(), 4);
    assert_eq!(*events.lock().unwrap(), ["a", "b", "c"]);
}

/// Errors raised in nested closure bodies reach the caller.
#[test]
fn body_errors_propagate() {
    let mut segment = parser()
        .parse_str("(|x: i32| (|y: i32| y / 0)(x))(7)")
        .unwrap();
    let error = segment.call0::<i32>().unwrap_err();
    assert!(
        format!("{error:#}").contains("division by zero"),
        "{error:#}"
    );
}

/// A counted owning value exercises clones and drops across aligned invocation frames.
#[derive(Debug)]
#[repr(align(128))]
struct Item(std::sync::Arc<std::sync::atomic::AtomicIsize>);

impl Clone for Item {
    /// Creates one separately owned value.
    fn clone(&self) -> Self {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self(self.0.clone())
    }
}

impl Drop for Item {
    /// Releases one live ownership.
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Builds counted values only during execution, not while compiling the test program.
fn counted_parser(live: std::sync::Arc<std::sync::atomic::AtomicIsize>) -> CELParser {
    let mut lookup = OpLookup::new();
    lookup.push_library_scope(move |name, segment, arity, _| {
        if arity == 0 && name == "item" {
            let live = live.clone();
            segment.op0(move || {
                live.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Item(live.clone())
            });
            return Ok(true);
        }
        if arity == 0 && name == "bad" {
            segment.op0r(|| -> anyhow::Result<i32> { anyhow::bail!("body failed") });
            return Ok(true);
        }
        Ok(false)
    });
    CELParser::with_type_resolver(lookup, |name: &str, args: &[cel_parser::ResolvedType]| {
        if name == "Item" && args.is_empty() {
            Some(ResolvedLeafType::new(
                "Item",
                ArrayElementType::cloneable::<Item>().unwrap(),
            ))
        } else {
            resolve_builtin_named_type(name, args)
        }
    })
}

/// Successful calls move one result out and release every intermediate ownership.
#[test]
fn aligned_owning_values_drop_once_on_success() {
    for source in [
        "(|x: Item| x)(item)",
        "((|x: Item| (x, x))(item)).0",
        "(|x: Item| (|y: Item| y)(x))(item)",
        "((|x: ([Item], Item)| x)(([item], item))).1",
    ] {
        let live = std::sync::Arc::new(std::sync::atomic::AtomicIsize::new(0));
        let mut segment = counted_parser(live.clone()).parse_str(source).unwrap();
        let result = segment.call0::<Item>().unwrap();
        assert_eq!(
            live.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "{source}"
        );
        drop(result);
        drop(segment);
        assert_eq!(
            live.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "{source}"
        );
    }
}

/// Failed argument or body evaluation releases all previously evaluated values.
#[test]
fn aligned_owning_values_drop_once_on_error() {
    for source in [
        "(|x: Item| bad)(item)",
        "(|x: Item, y: i32| y)(item, bad)",
        "(|x: Item, y: i32| y)(item, if true { bad } else { 0 })",
        "(|x: Item| (x, bad).1)(item)",
        "(|x: Item| if true { (x, bad).1 } else { 0 })(item)",
    ] {
        let live = std::sync::Arc::new(std::sync::atomic::AtomicIsize::new(0));
        let mut segment = counted_parser(live.clone()).parse_str(source).unwrap();
        assert!(segment.call0::<i32>().is_err(), "{source}");
        drop(segment);
        assert_eq!(
            live.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "{source}"
        );
    }
}

/// Host-defined call operations retain priority even for a statically known CEL closure.
#[test]
fn host_call_scope_precedes_general_invocation() {
    let mut lookup = OpLookup::new();
    lookup.push_scope(|name, segment, arity, _| {
        if name == "()" && arity == 2 {
            segment.op2(|_: cel_runtime::DynClosure, _: i32| 99i32)?;
            return Ok(true);
        }
        Ok(false)
    });
    let mut segment = CELParser::new(lookup).parse_str("(|x: i32| x)(1)").unwrap();
    assert_eq!(segment.call0::<i32>().unwrap(), 99);
}

/// Legacy tuple-shaped host call registrations also precede general closure invocation.
#[test]
fn host_tuple_call_precedes_general_invocation() {
    use std::any::TypeId;
    let mut lookup = OpLookup::new();
    lookup.register_tuple_op(cel_parser::op_table::TupleOpSignature {
        name: "()".into(),
        shape: vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
        tuple_operand_index: 1,
        operand_type_ids: vec![TypeId::of::<cel_runtime::DynClosure>()],
        op_fn: |segment, _| {
            segment.tuple_index(0);
            segment.op2(|_: cel_runtime::DynClosure, _: i32| 77i32)
        },
    });
    let mut segment = CELParser::new(lookup)
        .parse_str("(|x: (i32, i32)| x.0)((1, 2))")
        .unwrap();
    assert_eq!(segment.call0::<i32>().unwrap(), 77);
}

/// An erased Rust closure must not infer its result signature from its arguments.
#[test]
fn unknown_callable_signature_fails_explicitly() {
    let mut lookup = OpLookup::new();
    lookup.push_scope(|name, segment, arity, _| {
        if name == "opaque" && arity == 0 {
            let mut body = cel_runtime::DynSegment::new::<()>();
            body.just(true);
            segment.just(cel_runtime::DynClosure::new(
                vec![],
                std::any::TypeId::of::<bool>(),
                body,
            ));
            return Ok(true);
        }
        Ok(false)
    });
    let error = CELParser::new(lookup).parse_str("opaque()").err().unwrap();
    assert!(error.message().contains("signature is unknown"), "{error}");
}

/// Syntax-only parsing preserves custom recursive annotations without any registry lookup.
#[test]
fn ast_preserves_unresolved_annotations_and_static_body_walks() {
    use cel_parser::{AstContext, Expr, Parser, TypeExpr};
    let source = "(|x: (Unregistered, [[Unregistered]])| [1, true])(missing)";
    let expression = Parser::<AstContext>::new(OpLookup::new())
        .parse_str_ast(source)
        .unwrap();
    let Expr::Apply { callee, .. } = &expression else {
        panic!("expected application")
    };
    let Expr::Closure { params, .. } = callee.as_ref() else {
        panic!("expected closure")
    };
    let TypeExpr::Tuple { elements, .. } = &params[0].type_expr else {
        panic!("expected tuple")
    };
    assert!(matches!(&elements[0], TypeExpr::Named { name, .. } if name == "Unregistered"));
    assert!(
        matches!(&elements[1], TypeExpr::Array { element, .. } if matches!(element.as_ref(), TypeExpr::Array { .. }))
    );
    let (_, diagnostics) = cel_parser::ty::check_expr(&expression, &|_| cel_parser::Ty::Any);
    assert_eq!(diagnostics.len(), 1);
    assert!(
        diagnostics[0].message().contains("array"),
        "{}",
        diagnostics[0]
    );
}

/// Invocation imposes no built-in finite limit on parameter count.
#[test]
fn arbitrary_parameter_count() {
    let parameters = (0..32)
        .map(|index| format!("x{index}: i32"))
        .collect::<Vec<_>>()
        .join(", ");
    let arguments = (0..32)
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let source = format!("(|{parameters}| x0 + x31)({arguments})");
    let mut segment = parser().parse_str(&source).unwrap();
    assert_eq!(segment.call0::<i32>().unwrap(), 31);
}

/// A Rust caller retains its Any argument context around nested CEL invocations.
#[test]
fn rust_closure_api_remains_repeatable_with_nested_calls() {
    let mut segment = parser()
        .parse_str("|x: i32| (|y: i32| y + 1)(x) + x")
        .unwrap();
    let closure: cel_runtime::DynClosure = segment.call0().unwrap();
    for (input, expected) in [(2i32, 5), (7, 15)] {
        assert_eq!(closure.call::<i32>(&[&input]).unwrap(), expected);
    }
}

/// An opaque result need not implement Clone to cross a known invocation boundary.
#[derive(Debug, PartialEq)]
struct Ticket(String);

/// A known result moves directly from the body without requiring per-type result dispatch.
#[test]
fn arbitrary_nonclone_result_type() {
    let mut lookup = OpLookup::new();
    lookup.push_library_scope(|name, segment, arity, _| {
        if name == "ticket" && arity == 0 {
            segment.op0(|| Ticket("unique".into()));
            return Ok(true);
        }
        Ok(false)
    });
    let mut parser = CELParser::new(lookup);
    for source in ["(|| ticket)()", "((|| (ticket, ticket))()).1"] {
        let mut segment = parser.parse_str(source).unwrap();
        assert_eq!(segment.call0::<Ticket>().unwrap(), Ticket("unique".into()));
    }
}

/// A descriptor without clone capability reports an explicit annotation-use error.
#[test]
fn parameter_without_clone_adapter_is_explicitly_unsupported() {
    let mut parser = CELParser::with_type_resolver(
        OpLookup::new(),
        [(
            "Ticket",
            ResolvedLeafType::new("Ticket", ArrayElementType::leaf::<Ticket>().unwrap()),
        )],
    );
    let error = parser.parse_str("|x: Ticket| x").err().unwrap();
    assert!(error.message().contains("clone support"), "{error}");
}
