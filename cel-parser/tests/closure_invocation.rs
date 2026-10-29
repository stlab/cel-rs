//! Contracts for checked, noncapturing CEL closure invocation.

use cel_parser::{CELParser, OpLookup, ResolvedLeafType, resolve_builtin_named_type};
use cel_runtime::ValueType;

/// Operator operands require grouped closures, and formatting preserves those groups.
#[test]
fn closure_operands_require_grouping() {
    for (bare, grouped) in [
        ("1 + || 2", "1 + (|| 2)"),
        ("1 + |x: i32| x", "1 + (|x: i32| x)"),
        ("-|| 1", "-(|| 1)"),
        ("!|| true", "!(|| true)"),
        ("true && || true", "true && (|| true)"),
        ("false || || true", "false || (|| true)"),
        ("1 == || 1", "1 == (|| 1)"),
        ("1 | |x: i32| x", "1 | (|x: i32| x)"),
        ("1 ^ || 2", "1 ^ (|| 2)"),
        ("1 & || 2", "1 & (|| 2)"),
        ("1 << || 2", "1 << (|| 2)"),
        ("1 >> || 2", "1 >> (|| 2)"),
        ("1 * || 2", "1 * (|| 2)"),
        ("1 / || 2", "1 / (|| 2)"),
        ("1 % || 2", "1 % (|| 2)"),
        ("..|| 1", "..(|| 1)"),
        ("..=|| 1", "..=(|| 1)"),
        ("1..|| 2", "1..(|| 2)"),
        ("1..=|x: i32| x", "1..=(|x: i32| x)"),
    ] {
        assert!(
            cel_parser::Parser::<cel_parser::AstContext>::new(OpLookup::new())
                .parse_str_ast(bare)
                .is_err(),
            "ungrouped closure operand accepted: {bare}"
        );
        assert!(parser().parse_str(bare).is_err(), "{bare}");

        let expr = cel_parser::Parser::<cel_parser::AstContext>::new(OpLookup::new())
            .parse_str_ast(grouped)
            .unwrap();
        let formatted = cel_parser::format_expr(&expr, grouped, 0);
        assert_eq!(formatted, grouped);
        let reparsed = cel_parser::Parser::<cel_parser::AstContext>::new(OpLookup::new())
            .parse_str_ast(&formatted)
            .unwrap();
        assert_eq!(cel_parser::format_expr(&reparsed, &formatted, 0), formatted);
    }
}

/// Grouped calls and complete-expression arguments retain closure body precedence.
#[test]
fn closure_grouping_preserves_direct_and_retained_execution() {
    for source in [
        "(|| 1 + 2 * 3)()",
        "(|x: i32| x - 2 - 1)(10)",
        "(|f: Fn((i32,), i32)| f(6))(|x: i32| x + 1)",
        "(|| || 7)()()",
    ] {
        assert_eq!(
            parser().parse_str(source).unwrap().call0::<i32>().unwrap(),
            7
        );
        let expr = cel_parser::Parser::<cel_parser::AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let mut segment = cel_parser::compile_expr(
            &expr,
            &OpLookup::new(),
            cel_parser::builtin_type_resolver().as_ref(),
            &mut |_, _, _| Ok(false),
        )
        .unwrap();
        assert_eq!(segment.call0::<i32>().unwrap(), 7);
    }
}

/// Callable annotations retain complete signatures through higher-order and tuple intermediates.
#[test]
fn full_callable_annotations_execute_without_host_shape_registration() {
    use cel_runtime::{ArrayOf, IntoCStackList, Leaf, Nil};
    for source in [
        "(|f: Fn((i32,), [i32])| f(42i32))(|x: i32| [x])",
        "(|f: Fn((), [i32])| f())(|| [42i32])",
        "(|f: Fn(((i32, [i32]),), [i32])| f((42i32, [42i32])))(|x: (i32, [i32])| x.1)",
        "(if true { |x: i32| [x] } else { |x: i32| [x] })(42i32)",
    ] {
        let body = parser().parse_str(source).unwrap();
        let value = body
            .prepare::<Nil, ArrayOf<Leaf<i32>>>()
            .unwrap()
            .call(&().into_c_stack_list())
            .unwrap();
        assert_eq!(value.as_slice(), &[42]);
        let expr = cel_parser::Parser::<cel_parser::AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap();
        let (ty, diagnostics) = cel_parser::ty::check_expr(&expr, &|_| cel_parser::Ty::Any);
        assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
        assert_eq!(ty, cel_parser::Ty::Array(Box::new(cel_parser::Ty::I32)));
    }
}

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
            segment.annotate_top(
                cel_runtime::ValueType::cloneable_leaf::<Parcel>()
                    .with_equality::<cel_runtime::Leaf<Parcel>>()?
                    .with_debug::<cel_runtime::Leaf<Parcel>>()?,
            )?;
            return Ok(true);
        }
        Ok(false)
    });
    CELParser::with_type_resolver(lookup, |name: &str, args: &[cel_parser::ResolvedType]| {
        if name == "Parcel" && args.is_empty() {
            Some(
                ResolvedLeafType::new("Parcel", ValueType::cloneable_leaf::<Parcel>())
                    .with_complete_value_type(
                        cel_runtime::ValueType::cloneable_leaf::<Parcel>()
                            .with_equality::<cel_runtime::Leaf<Parcel>>()
                            .unwrap()
                            .with_debug::<cel_runtime::Leaf<Parcel>>()
                            .unwrap(),
                    )
                    .unwrap(),
            )
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
    use cel_runtime::{Cons, IntoRustTuple, Leaf, Nil};
    type Nested = Cons<Leaf<i32>, Cons<Leaf<Parcel>, Nil>>;
    let result = segment.call_dyn_value(&[]).unwrap();
    let (first, nested) = result
        .try_into_typed::<Cons<Leaf<Parcel>, Cons<Nested, Nil>>>()
        .unwrap()
        .into_rust_tuple();
    assert_eq!(first, Parcel("first".into()));
    assert_eq!(nested.into_rust_tuple(), (7, Parcel("last".into())));
}

/// Array annotations retain recursive element shapes rather than only the array marker.
#[test]
fn recursive_arrays_cross_invocation_boundaries() {
    let mut segment = parser()
        .parse_str("(|x: [[Parcel]]| (|y: [[Parcel]]| y)(x))([[first], [last]])")
        .unwrap();
    let rows = segment
        .call0::<cel_runtime::OwnedValue>()
        .unwrap()
        .try_into_typed::<cel_runtime::ArrayOf<cel_runtime::ArrayOf<cel_runtime::Leaf<Parcel>>>>()
        .unwrap()
        .into_vec();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].as_slice(), &[Parcel("first".into())]);
    assert_eq!(rows[1].as_slice(), &[Parcel("last".into())]);
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
                ValueType::cloneable_leaf::<Item>(),
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
            segment.drop_value()?;
            segment.drop_value()?;
            segment.just(99i32);
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
            segment.tuple_index(0)?;
            segment.drop_value()?;
            segment.drop_value()?;
            segment.just(77i32);
            Ok(())
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
    assert!(
        error.message().contains("callable signature required"),
        "{error}"
    );
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
    assert_eq!(diagnostics.len(), 2);
    assert!(
        diagnostics[0].message().contains("array"),
        "{}",
        diagnostics[0]
    );
    assert!(diagnostics[1].message().contains("Unregistered"));
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
            ResolvedLeafType::new("Ticket", ValueType::leaf::<Ticket>()),
        )],
    );
    let error = parser.parse_str("|x: Ticket| x").err().unwrap();
    assert!(error.message().contains("clone capability"), "{error}");
}
