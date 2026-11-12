//! Checked examples for the static CEL book.

use cel_parser::{CELParser, OpLookup};
use cel_runtime::DynClosure;
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

// ANCHOR: registered-record
/// Evaluates a host record and its direct accessor with exact field types.
#[test]
fn registered_record_and_member_example() {
    use cel_parser::{OperationSignature, RecordField, RecordSchema, ResolvedLeafType};
    use cel_runtime::ValueType;

    /// A host-defined value, not a built-in CEL type.
    #[derive(Clone, Debug, PartialEq)]
    struct Sample {
        value: f64,
    }
    let sample = ResolvedLeafType::new("Sample", ValueType::leaf::<Sample>());
    let scalar = ResolvedLeafType::new("f64", ValueType::leaf::<f64>());
    let mut lookup = OpLookup::new();
    lookup
        .register_record(RecordSchema {
            ty: sample.clone(),
            fields: vec![RecordField {
                name: "value",
                ty: scalar.clone().into(),
            }],
            emit: Box::new(|s, _, _, _| s.op1(|value: f64| Sample { value })),
        })
        .unwrap();
    lookup
        .register_operation(OperationSignature {
            name: "value",
            operands: vec![sample.into()],
            result: scalar.into(),
            emit: Box::new(|s, _| s.op1(|sample: Sample| sample.value)),
        })
        .unwrap();
    let mut parser = CELParser::new(lookup);
    for source in [
        "Sample { value: 3.0 }.value",
        "Sample { value: 3.0 }.value()",
    ] {
        assert_eq!(
            parser.parse_str(source).unwrap().call0::<f64>().unwrap(),
            3.0
        );
    }
    assert_eq!(
        parser
            .parse_str("if (Sample { value: 3.0 }.value) > 0.0 { 1 } else { 0 }")
            .unwrap()
            .call0::<i32>()
            .unwrap(),
        1
    );
    assert!(parser.parse_str("Sample { value: 1 }").is_err());
    assert!(parser.parse_str("(Sample { value: 3.0 }.value)()").is_err());
    assert!(
        parser
            .parse_str("Sample { value: 3.0 }.value = 4.0")
            .is_err()
    );
}
// ANCHOR_END: registered-record

/// Rounding is opt-in and distinguishes ties-away-from-zero from ties-to-even.
#[test]
fn round_requires_the_standard_library() {
    assert!(
        CELParser::new(OpLookup::new())
            .parse_str("round(-3.5)")
            .is_err()
    );

    assert_eq!(eval_with_std::<f64>("round(-3.5)"), -4.0);
    assert_eq!(eval_with_std::<f64>("(2.5).round()"), 3.0);
    assert_eq!(eval_with_std::<f64>("round_ties_even(2.5)"), 2.0);
}

#[test]
fn if_expression_selects_the_true_branch() {
    let mut segment = CELParser::new(OpLookup::new())
        .parse_str("if true { 1i32 } else { 2i32 }")
        .unwrap();

    assert_eq!(segment.call0::<i32>().unwrap(), 1);
}

#[test]
fn array_literal_round_trips_through_canonical_array() {
    let mut segment = CELParser::new(OpLookup::new())
        .parse_str("[0, 1, 2]")
        .unwrap();

    let array = segment
        .call0::<cel_runtime::OwnedValue>()
        .unwrap()
        .try_into_typed::<cel_runtime::ArrayOf<cel_runtime::Leaf<i32>>>()
        .unwrap();
    assert_eq!(array.into_vec(), vec![0, 1, 2]);
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
    let empty_array = empty
        .call0::<cel_runtime::OwnedValue>()
        .unwrap()
        .try_into_typed::<cel_runtime::ArrayOf<cel_runtime::Leaf<i32>>>()
        .unwrap();
    assert_eq!(empty_array.into_vec(), Vec::<i32>::new());

    let mut values = CELParser::new(OpLookup::new())
        .parse_str("[0i32, 1i32]: [i32]")
        .unwrap();
    let array = values
        .call0::<cel_runtime::OwnedValue>()
        .unwrap()
        .try_into_typed::<cel_runtime::ArrayOf<cel_runtime::Leaf<i32>>>()
        .unwrap();
    assert_eq!(array.into_vec(), vec![0, 1]);
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
    assert_eq!(eval_with_std::<f64>("round_ties_even(2.5)"), 2.0);
    assert_eq!(eval_with_std::<f32>("fract(-3.75f32)"), -0.75);
}

/// Computes graphics angles and polar coordinates using both call forms.
#[test]
fn standard_library_graphics_angle_examples() {
    for (source, expected) in [
        ("(30.0).to_radians().sin()", 0.5),
        ("asin(0.5).to_degrees()", 30.0),
        ("acos(0.5).to_degrees()", 60.0),
        ("atan(1.0).to_degrees()", 45.0),
        ("atan2(1.0, -1.0).to_degrees()", 135.0),
        ("to_radians(180.0)", std::f64::consts::PI),
        ("to_radians(180.0).to_degrees()", 180.0),
        ("sin_cos((30.0).to_radians()).0 * 10.0", 5.0),
    ] {
        assert!(
            (eval_with_std::<f64>(source) - expected).abs() < 1e-12,
            "{source}"
        );
    }
    assert_eq!(eval_with_std::<f32>("cos(0.0f32)"), 1.0);
    assert_eq!(eval_with_std::<f64>("tan(0.0)"), 0.0);
    assert_eq!(eval_with_std::<f32>("hypot(3.0f32, 4.0f32)"), 5.0);
}

/// Evaluates the documented power, exponential, and logarithmic curves.
#[test]
fn standard_library_graphics_curve_examples() {
    for (source, expected) in [
        ("cbrt(-8.0)", -2.0),
        ("powf(0.25, 0.5)", 0.5),
        ("exp(1.0)", std::f64::consts::E),
        ("exp2(3.0)", 8.0),
        ("ln(2.0)", std::f64::consts::LN_2),
        ("log(8.0, 2.0)", 3.0),
        ("log2(8.0)", 3.0),
        ("log10(100.0)", 2.0),
    ] {
        assert!(
            (eval_with_std::<f64>(source) - expected).abs() < 1e-12,
            "{source}"
        );
    }
    assert_eq!(eval_with_std::<f32>("powi(2.0f32, -3i32)"), 0.125);
}

/// Evaluates the documented reciprocal, interpolation, wrapping, and sign examples.
#[test]
fn standard_library_graphics_utility_examples() {
    assert_eq!(eval_with_std::<f64>("recip(4.0)"), 0.25);
    assert_eq!(eval_with_std::<f64>("mul_add(0.25, 8.0, 2.0)"), 4.0);
    assert_eq!(eval_with_std::<f64>("rem_euclid(-450.0, 360.0)"), 270.0);
    assert_eq!(eval_with_std::<f64>("copysign(2.0, -1.0)"), -2.0);
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
