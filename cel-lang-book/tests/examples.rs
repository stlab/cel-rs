//! Checked examples for the static CEL language book.

use cel_parser::{CELParser, OpLookup};
use cel_runtime::{DynClosure, DynamicArray};

#[test]
fn arithmetic_respects_precedence() {
    let mut segment = CELParser::new(OpLookup::new())
        .parse_str("10u32 + 20u32 * 5u32")
        .unwrap();

    assert_eq!(segment.call0::<u32>().unwrap(), 110);
}

#[test]
fn if_expression_selects_the_true_branch() {
    let mut segment = CELParser::new(OpLookup::new())
        .parse_str("if true { 1i32 } else { 2i32 }")
        .unwrap();

    assert_eq!(segment.call0::<i32>().unwrap(), 1);
}

#[test]
fn array_literal_round_trips_through_dynamic_array() {
    let mut segment = CELParser::new(OpLookup::new())
        .parse_str("[0, 1, 2]")
        .unwrap();

    let array: DynamicArray = segment.call0().unwrap();
    assert_eq!(array.try_into_vec::<i32>().unwrap(), vec![0, 1, 2]);
}

#[test]
fn heterogeneous_array_is_rejected() {
    assert!(
        CELParser::new(OpLookup::new())
            .parse_str("[1i32, 2.0f64]")
            .is_err()
    );
}

#[test]
fn closure_literal_compiles_and_calls() {
    let mut segment = CELParser::new(OpLookup::new())
        .parse_str("|x: i32| x + 1")
        .unwrap();

    let closure: DynClosure = segment.call0().unwrap();
    let value = 5i32;

    assert_eq!(closure.call::<i32>(&[&value]).unwrap(), 6);
}

#[test]
fn op_lookup_resolves_identifiers_from_a_custom_scope() {
    let mut lookup = OpLookup::new();
    lookup.push_scope(
        |name, segment, num_operands, _span| match (name, num_operands) {
            ("x", 0) => {
                segment.op0(|| 10i32);
                Ok(true)
            }
            ("y", 0) => {
                segment.op0(|| 20i32);
                Ok(true)
            }
            _ => Ok(false),
        },
    );

    let mut segment = CELParser::new(lookup).parse_str("x + y").unwrap();

    assert_eq!(segment.call0::<i32>().unwrap(), 30);
}
