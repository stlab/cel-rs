//! Parser-to-native contracts for canonical recursive containers.
use cel_parser::{CELParser, OpLookup};
use cel_runtime::{ArrayOf, Cons, IntoCStackList, IntoRustTuple, Leaf, Nil};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Describes a concrete tuple's source-ordered integer and floating-point fields.
type Pair = Cons<Leaf<i32>, Cons<Leaf<f64>, Nil>>;

/// Parsed tuple arrays transfer concrete lists and retain independent owners.
#[test]
fn parsed_tuple_array_repeated_calls() {
    let body = CELParser::new(OpLookup::new())
        .parse_str("[(1i32, 2.0f64), (3i32, 4.0f64)]")
        .unwrap();
    let mut prepared = body.prepare::<Nil, ArrayOf<Pair>>().unwrap();
    let first = prepared.call(&().into_c_stack_list()).unwrap().into_vec();
    let second = prepared.call(&().into_c_stack_list()).unwrap().into_vec();
    assert_eq!(first[0].clone().into_rust_tuple(), (1, 2.0));
    assert_eq!(first[1].clone().into_rust_tuple(), (3, 4.0));
    assert_eq!(second[1].clone().into_rust_tuple(), (3, 4.0));
    assert_ne!(first.as_ptr(), second.as_ptr());
}

/// Source unit and arrays nest in tuple fields without external wrappers.
#[test]
fn parsed_unit_and_recursive_empty_arrays() {
    type Item = Cons<Nil, Cons<ArrayOf<Leaf<i32>>, Nil>>;
    let body = CELParser::new(OpLookup::new())
        .parse_str("[((), [1i32]), ((), [2i32])]")
        .unwrap();
    let mut prepared = body.prepare::<Nil, ArrayOf<Item>>().unwrap();
    let values = prepared.call(&().into_c_stack_list()).unwrap().into_vec();
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].clone().into_rust_tuple().1.into_vec(), [1]);
    assert_eq!(values[1].clone().into_rust_tuple().1.into_vec(), [2]);
    let body = CELParser::new(OpLookup::new())
        .parse_str("[]: [[(i32, f64)]]")
        .unwrap();
    let mut prepared = body.prepare::<Nil, ArrayOf<ArrayOf<Pair>>>().unwrap();
    assert!(prepared.call(&().into_c_stack_list()).unwrap().is_empty());
}

/// Tuple selection preserves source order above ambient scalar operands.
#[test]
fn parsed_tuple_fields_above_ambient() {
    for (source, expected) in [
        ("10i32 + (1i32, 2i32).0", 11),
        ("10i32 + (1i32, 2i32).1", 12),
    ] {
        let mut body = CELParser::new(OpLookup::new()).parse_str(source).unwrap();
        assert_eq!(body.call0::<i32>().unwrap(), expected);
    }
}

/// Owns one noncloneable payload and records its observable destruction.
struct Tracked(Arc<Mutex<Vec<&'static str>>>);

impl Drop for Tracked {
    /// Records the destruction of one initialized value.
    fn drop(&mut self) {
        self.0.lock().unwrap().push("drop");
    }
}

/// Emits live values and a later failing initializer without compile-time side effects.
fn tracked_parser(events: Arc<Mutex<Vec<&'static str>>>, panic: bool) -> CELParser {
    let mut lookup = OpLookup::new();
    lookup.push_scope(move |name, segment, arity, _| {
        if arity != 0 {
            return Ok(false);
        }
        let log = events.clone();
        match name {
            "first" => segment.op0(move || {
                log.lock().unwrap().push("create");
                Tracked(log.clone())
            }),
            "bad" => segment.op0r(move || -> anyhow::Result<Tracked> {
                log.lock().unwrap().push("fail");
                if panic {
                    panic!("initializer panic");
                }
                Err(anyhow::anyhow!("original error").context("later element"))
            }),
            _ => return Ok(false),
        }
        Ok(true)
    });
    CELParser::new(lookup)
}

/// Later errors and panics destroy completed nested values and preserve repeatability.
#[test]
fn parsed_later_initializer_failure_cleans_nested_prefix() {
    type Item = Cons<Nil, Cons<ArrayOf<Leaf<Tracked>>, Nil>>;
    for panic in [false, true] {
        let events = Arc::new(Mutex::new(Vec::new()));
        let body = tracked_parser(events.clone(), panic)
            .parse_str("[((), [first]), ((), [bad])]")
            .unwrap();
        let mut prepared = body.prepare::<Nil, ArrayOf<Item>>().unwrap();
        assert!(events.lock().unwrap().is_empty());
        for _ in 0..2 {
            if panic {
                assert!(
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        prepared.call(&().into_c_stack_list())
                    }))
                    .is_err()
                );
            } else {
                let error = prepared.call(&().into_c_stack_list()).err().unwrap();
                assert_eq!(error.to_string(), "later element");
                assert_eq!(error.root_cause().to_string(), "original error");
            }
        }
        assert_eq!(
            *events.lock().unwrap(),
            ["create", "fail", "drop", "create", "fail", "drop"]
        );
    }
}

/// Slice adapters borrow a parsed array's original allocation and drop it after returning.
#[test]
fn parsed_slice_adapter_preserves_payload_pointer_and_owner_lifetime() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let pointer = Rc::new(Cell::new(std::ptr::null::<Tracked>()));
    let mut body = tracked_parser(events.clone(), false)
        .parse_str("[first]")
        .unwrap();
    let allocation = pointer.clone();
    body.op1_vec::<Leaf<Tracked>, ArrayOf<Leaf<Tracked>>, _>(move |values| {
        allocation.set(values.as_ptr());
        cel_runtime::Array::from_vec(values)
    })
    .unwrap();
    let log = events.clone();
    body.op1_slice::<Leaf<Tracked>, Leaf<usize>, _>(move |slice| {
        assert_eq!(slice.as_ptr(), pointer.get());
        log.lock().unwrap().push("slice");
        slice.len()
    })
    .unwrap();
    let mut prepared = body.prepare::<Nil, Leaf<usize>>().unwrap();
    assert_eq!(prepared.call(&().into_c_stack_list()).unwrap(), 1);
    assert_eq!(*events.lock().unwrap(), ["create", "slice", "drop"]);
}

/// Array construction and indexing preserve a heterogeneous ambient tuple's source order.
#[test]
fn parsed_array_above_ambient_and_moving_index() {
    type Result = Cons<Leaf<u8>, Cons<ArrayOf<Pair>, Nil>>;
    let body = CELParser::new(OpLookup::new())
        .parse_str("(7u8, [(1i32, 2.0f64), (3i32, 4.0f64)])")
        .unwrap();
    let mut prepared = body.prepare::<Nil, Result>().unwrap();
    let (ambient, values) = prepared
        .call(&().into_c_stack_list())
        .unwrap()
        .into_rust_tuple();
    assert_eq!(ambient, 7);
    assert_eq!(values.as_slice()[0].clone().into_rust_tuple(), (1, 2.0));

    let mut body = CELParser::new(OpLookup::new())
        .parse_str("[(1i32, 2.0f64), (3i32, 4.0f64)]")
        .unwrap();
    body.just(1i32);
    body.array_index::<i32>().unwrap();
    let mut prepared = body.prepare::<Nil, Pair>().unwrap();
    assert_eq!(
        prepared
            .call(&().into_c_stack_list())
            .unwrap()
            .into_rust_tuple(),
        (3, 4.0)
    );
}

/// Preparation rejects wrong recursive result identities before executing any initializer.
#[test]
fn parsed_prepare_rejects_recursive_mismatches_without_execution() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let body = tracked_parser(events.clone(), false)
        .parse_str("[first]")
        .unwrap();
    assert!(
        body.prepare::<Nil, ArrayOf<ArrayOf<Leaf<Tracked>>>>()
            .is_err()
    );
    assert!(events.lock().unwrap().is_empty());
}

/// A registered owning leaf whose optional capabilities must survive literal composition.
#[derive(Clone, Debug, PartialEq)]
struct Payload(i32);

/// Registered result factories preserve independent optional capabilities through tuples/arrays.
#[test]
fn parsed_registered_results_retain_optional_capabilities() {
    use cel_parser::{RecordField, RecordSchema, ResolvedLeafType};
    use cel_runtime::ValueType;
    let mut lookup = OpLookup::new();
    lookup
        .register_record(RecordSchema {
            ty: ResolvedLeafType::leaf::<Payload>("Payload")
                .unwrap()
                .with_complete_value_type(
                    ValueType::cloneable_leaf::<Payload>()
                        .with_equality::<Leaf<Payload>>()
                        .unwrap()
                        .with_debug::<Leaf<Payload>>()
                        .unwrap(),
                )
                .unwrap(),
            fields: vec![RecordField {
                name: "value",
                ty: ResolvedLeafType::leaf::<i32>("i32").unwrap().into(),
            }],
            emit: Box::new(|segment, _, _, _| segment.op1(Payload)),
        })
        .unwrap();
    let mut body = CELParser::new(lookup)
        .parse_str("[(Payload { value: 1 }, 2i32)]")
        .unwrap();
    let first = body.call_dyn_value(&[]).unwrap();
    let second = first.try_clone().unwrap();
    assert!(first.try_eq(&second).unwrap());
    assert!(first.try_debug().unwrap().contains("Payload(1)"));
}
