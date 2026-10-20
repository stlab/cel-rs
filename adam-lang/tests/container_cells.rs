//! Persistent compound cells preserve recursive ownership across propagation.
use adam_lang::{AdamParser, TypeRegistry};
use cel_parser::OpLookup;
use cel_runtime::{Array, ArrayOf, Cons, IntoCStackList, Leaf, Nil};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// A source-first integer paired with an owning array.
type Pair = Cons<Leaf<i32>, Cons<ArrayOf<Leaf<f64>>, Nil>>;
/// An owning canonical array of recursively compound pairs.
type Pairs = ArrayOf<Pair>;

/// Resolves complete signatures through exactly the Adam registry used by execution.
fn shape(types: &TypeRegistry, source: &str) -> cel_parser::ResolvedType {
    cel_parser::CELParser::new(OpLookup::new())
        .parse_type_expr_str(source)
        .unwrap()
        .resolve(&types.cel_type_resolver())
        .unwrap()
}

/// Installs actual borrowed-slice requirements and a canonical owning-array filter.
fn operations(types: &TypeRegistry, calls: Arc<AtomicUsize>) -> OpLookup {
    let mut ops = OpLookup::new();
    ops.register_operation(cel_parser::OperationSignature {
        name: "nonempty",
        operands: vec![shape(types, "[(i32, [f64])]")],
        result: shape(types, "bool"),
        emit: Arc::new(move |body, _| {
            let calls = calls.clone();
            body.op1_slice::<Pair, Leaf<bool>, _>(move |values| {
                calls.fetch_add(1, Ordering::SeqCst);
                values.iter().all(|value| !value.0.1.is_empty())
            })
        }),
    })
    .unwrap();
    ops.register_operation(cel_parser::OperationSignature {
        name: "head",
        operands: vec![shape(types, "[(i32, [f64])]")],
        result: shape(types, "[(i32, [f64])]"),
        emit: Arc::new(|body, _| {
            body.op1_vec::<Pair, Pairs, _>(|mut values| {
                values.truncate(1);
                Array::from_vec(values)
            })
        }),
    })
    .unwrap();
    ops
}

/// Source writes retain independent aspirations and repeatedly update compound outputs.
#[test]
fn recursive_sources_update_twice() {
    let types = TypeRegistry::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut parser = AdamParser::new(types.clone(), operations(&types, calls.clone()));
    let mut sheet = parser
        .parse_str(
            "sheet s {
            source pairs: [(i32, [f64])] filter _.head;
            cell copy: [(i32, [f64])];
            relationship { copy := pairs; }
            out wrapped: ([(i32, [f64])],) := (copy,) require { @valid copy.nonempty; };
        }",
        )
        .unwrap();
    let pairs = sheet.cell_names["pairs"].0;
    let copy = sheet.cell_names["copy"].0;
    for n in [1i32, 2] {
        sheet
            .write_compound::<Pairs>(
                pairs,
                Array::from_vec(vec![
                    (n, Array::from_vec(vec![f64::from(n)])).into_c_stack_list(),
                    (99, Array::from_vec(vec![99.0])).into_c_stack_list(),
                ]),
            )
            .unwrap();
        sheet.propagate().unwrap();
        let original = sheet.read_compound::<Pairs>(pairs).unwrap();
        let result = sheet.read_compound::<Pairs>(copy).unwrap();
        assert_eq!(original.as_slice()[0].1, n);
        assert_eq!(result.as_slice()[0].1, n);
        assert_eq!(result.len(), 1);
        let aspiration = sheet.source_compound::<Pairs>(pairs).unwrap();
        assert_eq!(aspiration.len(), 2);
        assert_eq!(aspiration.as_slice()[1].1, 99);
        assert_ne!(
            aspiration.as_slice()[0].0.1.as_slice().as_ptr(),
            result.as_slice()[0].0.1.as_slice().as_ptr()
        );
        assert_ne!(original.as_slice().as_ptr(), result.as_slice().as_ptr());
        type Wrapped = Cons<Pairs, Nil>;
        let wrapped = sheet
            .read_compound::<Wrapped>(sheet.cell_names["wrapped"].0)
            .unwrap();
        assert_eq!(wrapped.1.as_slice()[0].1, n);
    }
    assert!(calls.load(Ordering::SeqCst) >= 2);
}

/// Empty arrays and recursive tuples default without needing element defaults.
#[test]
fn recursive_defaults_and_unit() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    let sheet = parser.parse_str(
        "sheet s { cell xs: [(i32, [f64])]; cell t: (i32, [f64]); cell unit: (); cell one: ([i32],); }"
    ).unwrap();
    assert!(
        sheet
            .read_compound::<Pairs>(sheet.cell_names["xs"].0)
            .unwrap()
            .is_empty()
    );
    assert!(
        sheet
            .read_compound::<Pair>(sheet.cell_names["t"].0)
            .unwrap()
            .0
            .1
            .is_empty()
    );
    sheet
        .read_compound::<Nil>(sheet.cell_names["unit"].0)
        .unwrap();
}

/// Callable cells reject absent equality rather than inventing function equality.
#[test]
fn callable_capabilities_are_explicit() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    let missing = parser
        .parse_str("sheet s { cell f: Fn((i32,), i32); }")
        .unwrap_err();
    assert!(missing.message().contains("initializer") || missing.message().contains("provide"));
    let equality = parser
        .parse_str("sheet s { cell f: Fn((i32,), i32) = |x: i32| x; }")
        .unwrap_err();
    assert!(equality.message().contains("equality"), "{equality}");
    assert!(equality.span().start().column > 0);
    let array = parser
        .parse_str("sheet s { cell fs = [|x: i32| x]: [Fn((i32,), i32)]; }")
        .unwrap_err();
    assert!(array.message().contains("equality"), "{array}");
    let default_array = parser
        .parse_str("sheet s { cell fs: [Fn((i32,), i32)]; }")
        .unwrap_err();
    assert!(
        default_array.message().contains("equality"),
        "{default_array}"
    );
    assert!(default_array.span().start().column > 0);
    let mut cel = cel_parser::CELParser::new(OpLookup::new());
    let body = cel.parse_str("[|x: i32| x]: [Fn((i32,), i32)]").unwrap();
    type Functions = ArrayOf<cel_runtime::FnOf<Cons<Leaf<i32>, Nil>, Leaf<i32>>>;
    let mut prepared = body.prepare::<Nil, Functions>().unwrap();
    assert_eq!(prepared.call(&().into_c_stack_list()).unwrap().len(), 1);
}

/// Multi-output moves include tuple arrays, singleton tuples, and source unit.
#[test]
fn destructuring_moves_recursive_outputs() {
    let mut sheet = AdamParser::new(TypeRegistry::new(), OpLookup::new())
        .parse_str(
            "sheet s {
            source pairs: [(i32, [f64])] = [(1, [2.0])];
            cell left: [(i32, [f64])]; cell count: i32; cell unit: (); cell one: ([i32],);
            relationship { (left, count, unit, one) := (pairs, 7, (), ([9],)); }
        }",
        )
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(sheet.cell_names["count"].0).unwrap(), 7);
    assert_eq!(
        sheet
            .read_compound::<Pairs>(sheet.cell_names["left"].0)
            .unwrap()
            .as_slice()[0]
            .1,
        1
    );
    sheet
        .read_compound::<Nil>(sheet.cell_names["unit"].0)
        .unwrap();
    type One = Cons<ArrayOf<Leaf<i32>>, Nil>;
    assert_eq!(
        sheet
            .read_compound::<One>(sheet.cell_names["one"].0)
            .unwrap()
            .1
            .as_slice(),
        &[9]
    );
}

/// Generic foreign leaves need only one registration, not each containing shape.
#[test]
fn registered_generic_leaves_and_missing_defaults() {
    let mut types = TypeRegistry::new();
    types.register_no_default::<std::ops::RangeInclusive<i32>>("Bounds(i32)");
    let mut parser = AdamParser::new(types, OpLookup::new());
    let mut sheet = parser
        .parse_str(
            "sheet s {
            source xs: [(Bounds(i32), [i32])];
            out ys: [(Bounds(i32), [i32])] := xs;
        }",
        )
        .unwrap();
    type Item = Cons<Leaf<std::ops::RangeInclusive<i32>>, Cons<ArrayOf<Leaf<i32>>, Nil>>;
    type Items = ArrayOf<Item>;
    let xs = sheet.cell_names["xs"].0;
    sheet
        .write_compound::<Items>(
            xs,
            Array::from_vec(vec![(1..=3, Array::from_vec(vec![4])).into_c_stack_list()]),
        )
        .unwrap();
    sheet.propagate().unwrap();
    let ys = sheet
        .read_compound::<Items>(sheet.cell_names["ys"].0)
        .unwrap();
    assert_eq!(ys.as_slice()[0].1, 1..=3);
    let missing = parser
        .parse_str("sheet s { cell x: ([i32], Bounds(i32)); }")
        .unwrap_err();
    assert!(missing.message().contains("Bounds(i32)"), "{missing}");
    assert!(missing.message().contains("no default"), "{missing}");
    assert!(missing.span().start().column > 0);
}

/// Complete mismatches fail before operand effects and before host writes.
#[test]
fn mismatches_reject_before_effects_or_writes() {
    let calls = Arc::new(AtomicUsize::new(0));
    let effects = calls.clone();
    let mut ops = OpLookup::new();
    ops.push_scope(move |name, body, arity, _| {
        if name != "effect" || arity != 0 {
            return Ok(false);
        }
        let effects = effects.clone();
        body.op0(move || {
            effects.fetch_add(1, Ordering::SeqCst);
            1i32
        });
        Ok(true)
    });
    let mut parser = AdamParser::new(TypeRegistry::new(), ops);
    let error = parser
        .parse_str("sheet s { cell xs: [(i32, [f64])] = [(effect, [1])]; }")
        .unwrap_err();
    assert!(error.message().contains("type mismatch"), "{error}");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let error = parser
        .parse_str("sheet s { cell xs = [(effect, |x: i32| x)]; }")
        .unwrap_err();
    assert!(error.message().contains("equality"), "{error}");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let mut sheet = parser
        .parse_str("sheet s { source xs: [i32] = [4]; }")
        .unwrap();
    let xs = sheet.cell_names["xs"].0;
    assert!(
        sheet
            .write_compound::<ArrayOf<Leaf<u32>>>(xs, Array::from_vec(vec![5]))
            .is_err()
    );
    assert!(sheet.read_compound::<ArrayOf<Leaf<u32>>>(xs).is_err());
    assert_eq!(
        sheet
            .read_compound::<ArrayOf<Leaf<i32>>>(xs)
            .unwrap()
            .as_slice(),
        &[4]
    );
}

/// Zero-sized source unit remains distinct from a host unit leaf inside arrays and tuples.
#[test]
fn source_unit_composes_with_empty_and_singleton_shapes() {
    let mut sheet = AdamParser::new(TypeRegistry::new(), OpLookup::new()).parse_str(
        "sheet s { source units: [()] = [(), ()]; out nested: ([()], (), ((),)) := (units, (), ((),)); }"
    ).unwrap();
    sheet.propagate().unwrap();
    type Nested = Cons<ArrayOf<Nil>, Cons<Nil, Cons<Cons<Nil, Nil>, Nil>>>;
    let value = sheet
        .read_compound::<Nested>(sheet.cell_names["nested"].0)
        .unwrap();
    assert_eq!(value.1.len(), 2);
    assert!(
        sheet
            .read_compound::<ArrayOf<Leaf<()>>>(sheet.cell_names["units"].0)
            .is_err()
    );
}

/// Registered function markers preserve their registered equality and concrete access.
#[test]
fn registered_callable_markers_keep_concrete_capabilities() {
    /// A comparable host function marker, not invented closure identity.
    #[derive(Clone, Debug, PartialEq)]
    struct Marker;
    let mut types = TypeRegistry::new();
    types.register_no_default::<Marker>("Marker");
    let mut ops = OpLookup::new();
    ops.register_function("successor", Marker).unwrap();
    ops.register_operation(cel_parser::OperationSignature {
        name: "successor",
        operands: vec![shape(&types, "i32")],
        result: shape(&types, "i32"),
        emit: Arc::new(|body, _| {
            body.op1r(|n: i32| n.checked_add(1).ok_or_else(|| anyhow::anyhow!("overflow")))
        }),
    })
    .unwrap();
    let mut sheet = AdamParser::new(types, ops)
        .parse_str(
            "sheet s {
                source n: i32 = 1; cell f: Marker = successor; cell fs: [Marker] = [successor];
                out value: i32 := f(n); out from_array: [Marker] := fs;
            }",
        )
        .unwrap();
    let n = sheet.cell_names["n"].0;
    for value in [2i32, 3] {
        sheet.write(n, value).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(
            *sheet.read::<i32>(sheet.cell_names["value"].0).unwrap(),
            value.checked_add(1).unwrap()
        );
        assert_eq!(
            sheet
                .read_compound::<ArrayOf<Leaf<Marker>>>(sheet.cell_names["from_array"].0)
                .unwrap()
                .as_slice(),
            &[Marker]
        );
    }
    assert_eq!(
        *sheet.read::<Marker>(sheet.cell_names["f"].0).unwrap(),
        Marker
    );
}

/// Conditional subjects and requirement callbacks borrow the same recursive cell frames.
#[test]
fn compound_dependencies_drive_conditionals_and_requirements() {
    let types = TypeRegistry::new();
    let ops = operations(&types, Arc::new(AtomicUsize::new(0)));
    let mut sheet = AdamParser::new(types, ops)
        .parse_str(
            "sheet s {
            source pairs: [(i32, [f64])] require { @valid pairs.nonempty; };
            source a: i32 = 11; source b: i32 = 22; cell result: i32;
            conditional pairs.nonempty {
                true => { relationship { result := a; } }
                _ => { relationship { result := b; } }
            }
        }",
        )
        .unwrap();
    let pairs = sheet.cell_names["pairs"].0;
    let result = sheet.cell_names["result"].0;
    for (values, expected, valid) in [(vec![2.0], 11, true), (vec![], 22, false)] {
        sheet
            .write_compound::<Pairs>(
                pairs,
                Array::from_vec(vec![(1, Array::from_vec(values)).into_c_stack_list()]),
            )
            .unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(result).unwrap(), expected);
        assert_eq!(sheet.cell_requirements_valid(pairs), valid);
    }
}
