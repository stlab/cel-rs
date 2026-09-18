//! Checked examples for the static CEL book.

use cel_parser::{CELParser, OpLookup};
use cel_runtime::{DynClosure, DynamicArray};
use cel_std::install as install_std;

/// Evaluates `source` with the optional CEL standard library installed.
fn eval_with_std<T: 'static>(source: &str) -> T {
    let mut lookup = OpLookup::new();
    install_std(&mut lookup);
    let mut segment = CELParser::new(lookup).parse_str(source).unwrap();
    segment.call0::<T>().unwrap()
}

#[test]
fn arithmetic_respects_precedence() {
    let mut segment = CELParser::new(OpLookup::new())
        .parse_str("10u32 + 20u32 * 5u32")
        .unwrap();

    assert_eq!(segment.call0::<u32>().unwrap(), 110);
}

#[test]
fn round_is_available_with_the_standard_library() {
    assert!(
        CELParser::new(OpLookup::new())
            .parse_str("round(-3.5)")
            .is_err()
    );
    assert_eq!(eval_with_std::<f64>("round(-3.5)"), -4.0);
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
fn typed_array_annotations_support_empty_arrays() {
    let mut empty = CELParser::new(OpLookup::new())
        .parse_str("[]: [i32]")
        .unwrap();
    let empty_array: DynamicArray = empty.call0().unwrap();
    assert_eq!(
        empty_array.try_into_vec::<i32>().unwrap(),
        Vec::<i32>::new()
    );

    let mut values = CELParser::new(OpLookup::new())
        .parse_str("[0i32, 1i32]: [i32]")
        .unwrap();
    let array: DynamicArray = values.call0().unwrap();
    assert_eq!(array.try_into_vec::<i32>().unwrap(), vec![0, 1]);
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
fn optional_standard_library_supports_min_max_and_clamp_examples() {
    assert_eq!(eval_with_std::<i32>("min(3i32, -5i32)"), -5);
    assert_eq!(eval_with_std::<f64>("max(3.5f64, 2.5f64)"), 3.5);
    assert_eq!(eval_with_std::<i32>("clamp(-2i32, 0i32, 10i32)"), 0);
}

#[test]
fn optional_standard_library_supports_abs_and_signum_examples() {
    assert_eq!(eval_with_std::<i64>("abs(-42i64)"), 42);
    assert_eq!(eval_with_std::<i32>("signum(-7i32)"), -1);
    assert_eq!(eval_with_std::<f64>("signum(-3.5)"), -1.0);
}

#[test]
fn optional_standard_library_supports_float_projection_examples() {
    assert_eq!(eval_with_std::<f32>("sqrt(9.0f32)"), 3.0);
    assert_eq!(eval_with_std::<f64>("floor(-3.2)"), -4.0);
    assert_eq!(eval_with_std::<f64>("ceil(-3.2)"), -3.0);
    assert_eq!(eval_with_std::<f64>("trunc(-3.2)"), -3.0);
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
