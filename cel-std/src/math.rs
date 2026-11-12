//! Numeric CEL standard-library functions.
//!
//! Installations register complete operand signatures shared by ordinary calls and
//! direct named operations. Independent libraries can add disjoint overloads.

use anyhow::Result;
use cel_parser::{OpLookup, OperationSignature, ResolvedLeafType, ResolvedType};
use cel_runtime::{Cons, IntoCStackList, Leaf, Nil, ValueType};

/// Marker pushed for a bare `min` lookup; consumed by the paired `"()"` call.
#[derive(Clone)]
struct MinFn;
/// Marker pushed for a bare `max` lookup; consumed by the paired `"()"` call.
#[derive(Clone)]
struct MaxFn;
/// Marker pushed for a bare `clamp` lookup; consumed by the paired `"()"` call.
#[derive(Clone)]
struct ClampFn;

/// Marker pushed for a bare `abs` lookup; consumed by the paired `"()"` call.
#[derive(Clone)]
struct AbsFn;

/// Marker pushed for a bare `signum` lookup; consumed by the paired `"()"` call.
#[derive(Clone)]
struct SignumFn;
/// Marker pushed for a bare `sqrt` lookup; consumed by the paired `"()"` call.
#[derive(Clone)]
struct SqrtFn;
/// Marker pushed for a bare `floor` lookup; consumed by the paired `"()"` call.
#[derive(Clone)]
struct FloorFn;
/// Marker pushed for a bare `ceil` lookup; consumed by the paired `"()"` call.
#[derive(Clone)]
struct CeilFn;
/// Marker pushed for a bare `trunc` lookup; consumed by the paired `"()"` call.
#[derive(Clone)]
struct TruncFn;
/// Marker pushed for a bare `fract` lookup; consumed by the paired `"()"` call.
#[derive(Clone)]
struct FractFn;
/// Marker pushed for a bare `round_ties_even` lookup; consumed by the paired `"()"` call.
#[derive(Clone)]
struct RoundTiesEvenFn;

/// Identifies the sine function in ordinary calls.
#[derive(Clone)]
struct SinFn;
/// Identifies the cosine function in ordinary calls.
#[derive(Clone)]
struct CosFn;
/// Identifies the tangent function in ordinary calls.
#[derive(Clone)]
struct TanFn;
/// Identifies the inverse sine function in ordinary calls.
#[derive(Clone)]
struct AsinFn;
/// Identifies the inverse cosine function in ordinary calls.
#[derive(Clone)]
struct AcosFn;
/// Identifies the inverse tangent function in ordinary calls.
#[derive(Clone)]
struct AtanFn;
/// Identifies the quadrant-aware inverse tangent function in ordinary calls.
#[derive(Clone)]
struct Atan2Fn;
/// Identifies the paired sine and cosine function in ordinary calls.
#[derive(Clone)]
struct SinCosFn;
/// Identifies the degrees-to-radians conversion in ordinary calls.
#[derive(Clone)]
struct ToRadiansFn;
/// Identifies the radians-to-degrees conversion in ordinary calls.
#[derive(Clone)]
struct ToDegreesFn;
/// Identifies the hypotenuse function in ordinary calls.
#[derive(Clone)]
struct HypotFn;
/// Identifies the cube root function in ordinary calls.
#[derive(Clone)]
struct CbrtFn;
/// Identifies integer exponentiation in ordinary calls.
#[derive(Clone)]
struct PowiFn;
/// Identifies floating-point exponentiation in ordinary calls.
#[derive(Clone)]
struct PowfFn;
/// Identifies the natural exponential function in ordinary calls.
#[derive(Clone)]
struct ExpFn;
/// Identifies the base-two exponential function in ordinary calls.
#[derive(Clone)]
struct Exp2Fn;
/// Identifies the natural logarithm function in ordinary calls.
#[derive(Clone)]
struct LnFn;
/// Identifies the arbitrary-base logarithm function in ordinary calls.
#[derive(Clone)]
struct LogFn;
/// Identifies the base-two logarithm function in ordinary calls.
#[derive(Clone)]
struct Log2Fn;
/// Identifies the base-ten logarithm function in ordinary calls.
#[derive(Clone)]
struct Log10Fn;
/// Identifies the reciprocal function in ordinary calls.
#[derive(Clone)]
struct RecipFn;
/// Identifies fused multiply-add in ordinary calls.
#[derive(Clone)]
struct MulAddFn;
/// Identifies the Euclidean remainder function in ordinary calls.
#[derive(Clone)]
struct RemEuclidFn;
/// Identifies sign copying in ordinary calls.
#[derive(Clone)]
struct CopysignFn;
/// Identifies ties-away-from-zero rounding in ordinary calls.
#[derive(Clone)]
struct RoundFn;

/// Returns a numeric leaf's complete concrete descriptor.
fn numeric_type<T: 'static>() -> cel_parser::ResolvedType {
    ResolvedLeafType::new(std::any::type_name::<T>(), ValueType::leaf::<T>()).into()
}

/// Registers existing numeric contracts as composable direct operation signatures.
///
/// Integer extrema use `Ord`; floating extrema avoid NaNs. Absolute value checks
/// signed overflow. Clamp rejects bounds unless `lo <= hi`, including NaN bounds.
/// Other unary functions retain the corresponding Rust method semantics.
///
/// # Errors
/// Returns an error if any numeric signature was previously installed.
///
/// - Complexity: O(s²) in the number of installed signatures.
pub(crate) fn install(lookup: &mut OpLookup) -> Result<()> {
    lookup.register_function("min", MinFn)?;
    lookup.register_function("max", MaxFn)?;
    lookup.register_function("clamp", ClampFn)?;
    lookup.register_function("abs", AbsFn)?;
    lookup.register_function("signum", SignumFn)?;
    lookup.register_function("sqrt", SqrtFn)?;
    lookup.register_function("floor", FloorFn)?;
    lookup.register_function("ceil", CeilFn)?;
    lookup.register_function("trunc", TruncFn)?;
    lookup.register_function("fract", FractFn)?;
    lookup.register_function("round_ties_even", RoundTiesEvenFn)?;
    lookup.register_function("sin", SinFn)?;
    lookup.register_function("cos", CosFn)?;
    lookup.register_function("tan", TanFn)?;
    lookup.register_function("asin", AsinFn)?;
    lookup.register_function("acos", AcosFn)?;
    lookup.register_function("atan", AtanFn)?;
    lookup.register_function("atan2", Atan2Fn)?;
    lookup.register_function("sin_cos", SinCosFn)?;
    lookup.register_function("to_radians", ToRadiansFn)?;
    lookup.register_function("to_degrees", ToDegreesFn)?;
    lookup.register_function("hypot", HypotFn)?;
    lookup.register_function("cbrt", CbrtFn)?;
    lookup.register_function("powi", PowiFn)?;
    lookup.register_function("powf", PowfFn)?;
    lookup.register_function("exp", ExpFn)?;
    lookup.register_function("exp2", Exp2Fn)?;
    lookup.register_function("ln", LnFn)?;
    lookup.register_function("log", LogFn)?;
    lookup.register_function("log2", Log2Fn)?;
    lookup.register_function("log10", Log10Fn)?;
    lookup.register_function("recip", RecipFn)?;
    lookup.register_function("mul_add", MulAddFn)?;
    lookup.register_function("rem_euclid", RemEuclidFn)?;
    lookup.register_function("copysign", CopysignFn)?;
    lookup.register_function("round", RoundFn)?;

    macro_rules! extrema_and_clamp {
        ([$($t:ty),+ $(,)?]) => { $(
            lookup.register_operation(OperationSignature {
                name: "min", operands: vec![numeric_type::<$t>(); 2],
                result: numeric_type::<$t>(),
                emit: Box::new(|segment, _| segment.op2(|a: $t, b: $t| a.min(b))),
            })?;
            lookup.register_operation(OperationSignature {
                name: "max", operands: vec![numeric_type::<$t>(); 2],
                result: numeric_type::<$t>(),
                emit: Box::new(|segment, _| segment.op2(|a: $t, b: $t| a.max(b))),
            })?;
            lookup.register_operation(OperationSignature {
                name: "clamp", operands: vec![numeric_type::<$t>(); 3],
                result: numeric_type::<$t>(),
                emit: Box::new(|segment, _| segment.op3r(|x: $t, lo: $t, hi: $t| {
                    if lo <= hi { Ok(x.clamp(lo, hi)) }
                    else { Err(anyhow::anyhow!("invalid clamp bounds")) }
                })),
            })?;
        )+ };
    }
    extrema_and_clamp!([
        u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64
    ]);

    macro_rules! unary {
        ($method:ident, [$($t:ty),+ $(,)?]) => { $(
            lookup.register_operation(OperationSignature {
                name: stringify!($method), operands: vec![numeric_type::<$t>()],
                result: numeric_type::<$t>(),
                emit: Box::new(|segment, _| segment.op1(|x: $t| x.$method())),
            })?;
        )+ };
    }
    macro_rules! checked_abs {
        ([$($t:ty),+ $(,)?]) => { $(
            lookup.register_operation(OperationSignature {
                name: "abs", operands: vec![numeric_type::<$t>()],
                result: numeric_type::<$t>(),
                emit: Box::new(|segment, _| segment.op1r(|x: $t| {
                    x.checked_abs().ok_or_else(|| anyhow::anyhow!("arithmetic overflow"))
                })),
            })?;
        )+ };
    }
    checked_abs!([i8, i16, i32, i64, i128, isize]);
    unary!(abs, [f32, f64]);
    unary!(signum, [i8, i16, i32, i64, i128, isize, f32, f64]);
    unary!(sqrt, [f32, f64]);
    unary!(floor, [f32, f64]);
    unary!(ceil, [f32, f64]);
    unary!(trunc, [f32, f64]);
    unary!(fract, [f32, f64]);
    unary!(round_ties_even, [f32, f64]);
    unary!(sin, [f32, f64]);
    unary!(cos, [f32, f64]);
    unary!(tan, [f32, f64]);
    unary!(asin, [f32, f64]);
    unary!(acos, [f32, f64]);
    unary!(atan, [f32, f64]);
    unary!(to_radians, [f32, f64]);
    unary!(to_degrees, [f32, f64]);
    unary!(cbrt, [f32, f64]);
    unary!(exp, [f32, f64]);
    unary!(exp2, [f32, f64]);
    unary!(ln, [f32, f64]);
    unary!(log2, [f32, f64]);
    unary!(log10, [f32, f64]);
    unary!(recip, [f32, f64]);
    unary!(round, [f32, f64]);

    /// Registers same-precision binary Rust float methods.
    macro_rules! binary {
        ($method:ident, [$($t:ty),+ $(,)?]) => { $(
            lookup.register_operation(OperationSignature {
                name: stringify!($method), operands: vec![numeric_type::<$t>(); 2],
                result: numeric_type::<$t>(),
                emit: Box::new(|segment, _| segment.op2(<$t>::$method)),
            })?;
        )+ };
    }
    binary!(atan2, [f32, f64]);
    binary!(hypot, [f32, f64]);
    binary!(powf, [f32, f64]);
    binary!(log, [f32, f64]);
    binary!(rem_euclid, [f32, f64]);
    binary!(copysign, [f32, f64]);

    /// Registers heterogeneous, ternary, and tuple-valued Rust float methods.
    macro_rules! remaining_float_operations {
        ([$($t:ty),+ $(,)?]) => { $(
            lookup.register_operation(OperationSignature {
                name: "powi",
                operands: vec![numeric_type::<$t>(), numeric_type::<i32>()],
                result: numeric_type::<$t>(),
                emit: Box::new(|segment, _| segment.op2(<$t>::powi)),
            })?;
            lookup.register_operation(OperationSignature {
                name: "mul_add",
                operands: vec![numeric_type::<$t>(); 3],
                result: numeric_type::<$t>(),
                emit: Box::new(|segment, _| segment.op3(<$t>::mul_add)),
            })?;
            lookup.register_operation(OperationSignature {
                name: "sin_cos",
                operands: vec![numeric_type::<$t>()],
                result: ResolvedType::Tuple { elements: vec![numeric_type::<$t>(); 2] },
                emit: Box::new(|segment, _| {
                    segment.op1_repr::<Leaf<$t>, Cons<Leaf<$t>, Cons<Leaf<$t>, Nil>>, _>(
                        |x| x.sin_cos().into_c_stack_list(),
                    )
                }),
            })?;
        )+ };
    }
    remaining_float_operations!([f32, f64]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install;
    use cel_parser::OpLookup;
    use cel_runtime::DynSegment;
    use proc_macro2::Span;

    /// Evaluates a scalar expression with the standard library installed.
    ///
    /// - Precondition: `source` compiles and returns `T`.
    /// - Complexity: O(n + s²) for source length n and installed signatures s.
    fn evaluate<T: 'static>(source: &str) -> T {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        cel_parser::CELParser::new(lookup)
            .parse_str(source)
            .unwrap()
            .call0::<T>()
            .unwrap()
    }

    /// Evaluates a float expression after replacing `$` with its concrete precision.
    ///
    /// - Precondition: `precision` is `f32` or `f64`; substituted source returns that type.
    /// - Complexity: O(n + s²) for source length n and installed signatures s.
    fn evaluate_float(source: &str, precision: &str) -> f64 {
        debug_assert!(matches!(precision, "f32" | "f64"));
        let source = source.replace('$', precision);
        if precision == "f32" {
            f64::from(evaluate::<f32>(&source))
        } else {
            evaluate::<f64>(&source)
        }
    }

    /// Preserves float precision, argument order, and both call forms for graphics math.
    #[test]
    fn graphics_math_supports_ordinary_and_receiver_calls() {
        for (name, receiver, arguments, expected) in [
            ("sin", "0.5$", "", 0.479_425_538_604_203),
            ("cos", "0.5$", "", 0.877_582_561_890_372_8),
            ("tan", "0.5$", "", 0.546_302_489_843_790_5),
            ("asin", "0.5$", "", std::f64::consts::FRAC_PI_6),
            ("acos", "0.5$", "", std::f64::consts::FRAC_PI_3),
            ("atan", "1.0$", "", std::f64::consts::FRAC_PI_4),
            ("atan2", "1.0$", "-1.0$", 3.0 * std::f64::consts::FRAC_PI_4),
            ("to_radians", "180.0$", "", std::f64::consts::PI),
            ("to_degrees", "0.5$", "", 28.647_889_756_541_16),
            ("hypot", "3.0$", "4.0$", 5.0),
            ("cbrt", "-8.0$", "", -2.0),
            ("powi", "2.0$", "-3i32", 0.125),
            ("powf", "9.0$", "0.5$", 3.0),
            ("exp", "1.0$", "", std::f64::consts::E),
            ("exp2", "3.0$", "", 8.0),
            ("ln", "2.0$", "", std::f64::consts::LN_2),
            ("log", "8.0$", "2.0$", 3.0),
            ("log2", "8.0$", "", 3.0),
            ("log10", "100.0$", "", 2.0),
            ("recip", "4.0$", "", 0.25),
            ("mul_add", "2.0$", "3.0$, 4.0$", 10.0),
            ("rem_euclid", "-7.0$", "4.0$", 1.0),
            ("copysign", "2.0$", "-1.0$", -2.0),
            ("round", "-2.5$", "", -3.0),
        ] {
            let separator = if arguments.is_empty() { "" } else { ", " };
            for source in [
                format!("{name}({receiver}{separator}{arguments})"),
                format!("({receiver}).{name}({arguments})"),
            ] {
                for precision in ["f32", "f64"] {
                    let actual = evaluate_float(&source, precision);
                    let tolerance = if precision == "f32" { 1e-5 } else { 1e-12 };
                    assert!(
                        (actual - expected).abs() <= tolerance,
                        "{source} ({precision}): {actual} != {expected}"
                    );
                }
            }
        }
    }

    /// Exposes sine and cosine as a composable CEL tuple in both precisions.
    #[test]
    fn sin_cos_returns_a_structural_tuple() {
        for precision in ["f32", "f64"] {
            for source in ["sin_cos(0.5$)", "(0.5$).sin_cos()"] {
                for (index, expected) in [(0, 0.479_425_538_604_203), (1, 0.877_582_561_890_372_8)]
                {
                    let source = format!("({source}).{index}");
                    let actual = evaluate_float(&source, precision);
                    let tolerance = if precision == "f32" { 1e-6 } else { 1e-12 };
                    assert!((actual - expected).abs() <= tolerance, "{source}");
                }
            }
            assert_eq!(
                evaluate_float("(|pair: ($, $)| pair.0 + pair.1)(sin_cos(0.0$))", precision),
                1.0
            );
        }
    }

    /// Rejects unsupported numeric types, mixed precisions, exponents, and arities.
    #[test]
    fn graphics_math_rejects_unsupported_signatures() {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut parser = cel_parser::CELParser::new(lookup);
        for source in [
            "sin(1i32)",
            "sin(true)",
            "sin()",
            "sin(0.0, 1.0)",
            "sin_cos(1i32)",
            "atan2(1.0f32, 1.0f64)",
            "hypot(3.0, 4i32)",
            "powi(2.0, 3i64)",
            "powi(2.0, 3.0)",
            "powf(2.0, 3i32)",
            "log(8.0f32, 2.0f64)",
            "mul_add(1.0, 2.0)",
            "mul_add(1.0, 2.0, 3.0f32)",
            "(1i32).sin()",
            "(2.0).powi(3u32)",
        ] {
            assert!(parser.parse_str(source).is_err(), "{source}");
        }
    }

    /// Preserves Rust's NaN, infinity, domain, and signed-zero behavior.
    #[test]
    fn graphics_math_preserves_special_values() {
        for precision in ["f32", "f64"] {
            for source in [
                "sin(1.0$ / 0.0$)",
                "cos(0.0$ / 0.0$)",
                "tan(1.0$ / 0.0$)",
                "asin(2.0$)",
                "acos(-2.0$)",
                "ln(-1.0$)",
                "log2(-1.0$)",
                "log10(-1.0$)",
                "log(8.0$, -2.0$)",
                "powf(-2.0$, 0.5$)",
                "rem_euclid(1.0$, 0.0$)",
                "mul_add(0.0$, 1.0$ / 0.0$, 1.0$)",
            ] {
                assert!(evaluate_float(source, precision).is_nan(), "{source}");
            }
            for source in ["recip(0.0$)", "exp(1.0$ / 0.0$)", "exp2(1.0$ / 0.0$)"] {
                assert_eq!(evaluate_float(source, precision), f64::INFINITY, "{source}");
            }
            for source in ["ln(0.0$)", "log2(0.0$)", "log10(0.0$)", "recip(-0.0$)"] {
                assert_eq!(
                    evaluate_float(source, precision),
                    f64::NEG_INFINITY,
                    "{source}"
                );
            }
            for source in [
                "sin(-0.0$)",
                "tan(-0.0$)",
                "asin(-0.0$)",
                "atan(-0.0$)",
                "cbrt(-0.0$)",
                "to_radians(-0.0$)",
                "to_degrees(-0.0$)",
                "round(-0.25$)",
                "copysign(0.0$, -1.0$)",
            ] {
                let actual = evaluate_float(source, precision);
                assert_eq!(actual, 0.0, "{source}");
                assert!(actual.is_sign_negative(), "{source}");
            }
            assert_eq!(evaluate_float("powi(-2.0$, 3i32)", precision), -8.0);
            assert_eq!(evaluate_float("powi(0.0$ / 0.0$, 0i32)", precision), 1.0);
            assert_eq!(
                evaluate_float("hypot(1.0$ / 0.0$, 0.0$ / 0.0$)", precision),
                f64::INFINITY
            );
            let angle = evaluate_float("atan2(-0.0$, -1.0$)", precision);
            let tolerance = if precision == "f32" { 1e-6 } else { 1e-12 };
            assert!((angle + std::f64::consts::PI).abs() <= tolerance);
        }
    }

    /// Uses fused multiplication and addition instead of separately rounded operations.
    #[test]
    fn mul_add_rounds_only_once() {
        assert_eq!(
            evaluate::<f64>("mul_add(1.0000000000000002, 0.9999999999999998, -1.0)"),
            -4.930_380_657_631_324e-32
        );
        assert_eq!(
            evaluate::<f32>("mul_add(1.0000001192092896f32, 0.9999998807907104f32, -1.0f32)"),
            -1.421_085_5e-14
        );
    }

    /// Confirms independent unary overloads compose with numeric calls in either install order.
    #[test]
    fn independent_min_overload_composes_with_standard_library() -> Result<()> {
        use cel_parser::{OperationSignature, ResolvedLeafType};
        use cel_runtime::ValueType;

        /// Names the host's independent overload installer.
        #[derive(Clone)]
        struct HostMin;
        /// Supplies one custom operand.
        #[derive(Clone)]
        struct Corner(f64);

        for reverse in [false, true] {
            let mut lookup = OpLookup::new();
            if reverse {
                install(&mut lookup);
            }
            lookup.register_function("min", HostMin)?;
            lookup.register_operation(OperationSignature {
                name: "min",
                operands: vec![ResolvedLeafType::new("Corner", ValueType::leaf::<Corner>()).into()],
                result: ResolvedLeafType::new("f64", ValueType::leaf::<f64>()).into(),
                emit: Box::new(|segment, _| segment.op1(|corner: Corner| corner.0)),
            })?;
            if !reverse {
                install(&mut lookup);
            }
            let span = Span::call_site();
            let mut segment = DynSegment::new::<()>();
            lookup.lookup("min", &mut segment, 0, span, span).unwrap();
            segment.just(Corner(5.0));
            lookup.lookup("()", &mut segment, 2, span, span).unwrap();
            assert_eq!(segment.call0::<f64>()?, 5.0);
            let mut parser = cel_parser::CELParser::new(lookup);
            assert_eq!(
                parser.parse_str("min(3.0, 7.0)").unwrap().call0::<f64>()?,
                3.0
            );
        }
        Ok(())
    }

    /// Preserves every numeric type's extrema and clamp contracts through both dispatch paths.
    #[test]
    fn numeric_registrations_cover_every_existing_type() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let span = Span::call_site();
        /// Checks extrema and clamp dispatch for each listed numeric representation.
        macro_rules! check {
            ([$($t:ty),+ $(,)?]) => { $(
                let mut min = DynSegment::new::<()>();
                min.just(3 as $t);
                min.just(7 as $t);
                lookup.lookup_member("min", &mut min, 2, span, span).unwrap();
                assert_eq!(min.call0::<$t>()?, 3 as $t);

                let mut max = DynSegment::new::<()>();
                lookup.lookup("max", &mut max, 0, span, span).unwrap();
                max.just(3 as $t);
                max.just(7 as $t);
                lookup.lookup("()", &mut max, 3, span, span).unwrap();
                assert_eq!(max.call0::<$t>()?, 7 as $t);

                let mut clamp = DynSegment::new::<()>();
                lookup.lookup("clamp", &mut clamp, 0, span, span).unwrap();
                clamp.just(9 as $t);
                clamp.just(3 as $t);
                clamp.just(7 as $t);
                lookup.lookup("()", &mut clamp, 4, span, span).unwrap();
                assert_eq!(clamp.call0::<$t>()?, 7 as $t);
            )+ };
        }
        check!([
            u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64
        ]);
        Ok(())
    }

    /// Selects the smaller signed operand through installed callable dispatch.
    #[test]
    fn min_returns_the_smaller_of_two_signed_operands() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup("min", &mut segment, 0, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(3i32);
        segment.just(-5i32);
        lookup
            .lookup("()", &mut segment, 3, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<i32>()?, -5);
        Ok(())
    }

    /// Selects the smaller unsigned operand through installed callable dispatch.
    #[test]
    fn min_returns_the_smaller_of_two_unsigned_operands() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup("min", &mut segment, 0, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(3u32);
        segment.just(5u32);
        lookup
            .lookup("()", &mut segment, 3, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<u32>()?, 3);
        Ok(())
    }

    /// Selects the numeric floating-point minimum when the other operand is NaN.
    #[test]
    fn min_avoids_nan_when_exactly_one_float_operand_is_nan() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup("min", &mut segment, 0, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(f64::NAN);
        segment.just(2.0f64);
        lookup
            .lookup("()", &mut segment, 3, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<f64>()?, 2.0);
        Ok(())
    }

    /// Selects the larger signed operand through installed callable dispatch.
    #[test]
    fn max_returns_the_larger_of_two_signed_operands() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup("max", &mut segment, 0, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(3i32);
        segment.just(-5i32);
        lookup
            .lookup("()", &mut segment, 3, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<i32>()?, 3);
        Ok(())
    }

    /// Selects the larger unsigned operand through installed callable dispatch.
    #[test]
    fn max_returns_the_larger_of_two_unsigned_operands() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup("max", &mut segment, 0, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(3u32);
        segment.just(5u32);
        lookup
            .lookup("()", &mut segment, 3, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<u32>()?, 5);
        Ok(())
    }

    /// Selects the larger floating-point operand through installed callable dispatch.
    #[test]
    fn max_returns_the_larger_of_two_float_operands() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup("max", &mut segment, 0, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(3.5f64);
        segment.just(2.5f64);
        lookup
            .lookup("()", &mut segment, 3, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<f64>()?, 3.5);
        Ok(())
    }

    /// Retains a signed value already inside its inclusive clamp bounds.
    #[test]
    fn clamp_bounds_a_value_inside_its_range_unchanged() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup(
                "clamp",
                &mut segment,
                0,
                Span::call_site(),
                Span::call_site(),
            )
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(5i32);
        segment.just(0i32);
        segment.just(10i32);
        lookup
            .lookup("()", &mut segment, 4, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<i32>()?, 5);
        Ok(())
    }

    /// Raises a signed value below its clamp range to the lower bound.
    #[test]
    fn clamp_bounds_a_value_below_its_range_up_to_lo() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup(
                "clamp",
                &mut segment,
                0,
                Span::call_site(),
                Span::call_site(),
            )
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(-5i32);
        segment.just(0i32);
        segment.just(10i32);
        lookup
            .lookup("()", &mut segment, 4, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<i32>()?, 0);
        Ok(())
    }

    /// Lowers a signed value above its clamp range to the upper bound.
    #[test]
    fn clamp_bounds_a_value_above_its_range_down_to_hi() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup(
                "clamp",
                &mut segment,
                0,
                Span::call_site(),
                Span::call_site(),
            )
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(15i32);
        segment.just(0i32);
        segment.just(10i32);
        lookup
            .lookup("()", &mut segment, 4, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<i32>()?, 10);
        Ok(())
    }

    /// Rejects reversed clamp bounds through the runtime error channel.
    #[test]
    fn clamp_errs_when_lo_is_greater_than_hi() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup(
                "clamp",
                &mut segment,
                0,
                Span::call_site(),
                Span::call_site(),
            )
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(5i32);
        segment.just(10i32);
        segment.just(0i32);
        lookup
            .lookup("()", &mut segment, 4, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        let result = segment.call0::<i32>();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "invalid clamp bounds");
        Ok(())
    }

    /// Rejects a NaN clamp bound through the runtime error channel.
    #[test]
    fn clamp_errs_when_a_bound_is_nan() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup(
                "clamp",
                &mut segment,
                0,
                Span::call_site(),
                Span::call_site(),
            )
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(5.0f64);
        segment.just(f64::NAN);
        segment.just(10.0f64);
        lookup
            .lookup("()", &mut segment, 4, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        let result = segment.call0::<f64>();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "invalid clamp bounds");
        Ok(())
    }

    /// Applies inclusive clamp bounds to an unsigned operand.
    #[test]
    fn clamp_bounds_an_unsigned_integer_correctly() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup(
                "clamp",
                &mut segment,
                0,
                Span::call_site(),
                Span::call_site(),
            )
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(100u32);
        segment.just(0u32);
        segment.just(10u32);
        lookup
            .lookup("()", &mut segment, 4, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<u32>()?, 10);
        Ok(())
    }

    /// Retains a floating-point value already inside its clamp bounds.
    #[test]
    fn clamp_bounds_a_float_value_inside_its_range_unchanged() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup(
                "clamp",
                &mut segment,
                0,
                Span::call_site(),
                Span::call_site(),
            )
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(5.0f64);
        segment.just(0.0f64);
        segment.just(10.0f64);
        lookup
            .lookup("()", &mut segment, 4, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<f64>()?, 5.0);
        Ok(())
    }

    /// Returns the positive magnitude of a representable negative signed operand.
    #[test]
    fn abs_returns_the_absolute_value_of_a_negative_signed_integer() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup("abs", &mut segment, 0, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(-7i32);
        lookup
            .lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<i32>()?, 7);
        Ok(())
    }

    /// Reports an unrepresentable signed magnitude as a runtime error.
    #[test]
    fn abs_errs_on_signed_integer_overflow() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup("abs", &mut segment, 0, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(i32::MIN);
        lookup
            .lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        let result = segment.call0::<i32>();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "arithmetic overflow");
        Ok(())
    }

    /// Returns the positive magnitude of a negative floating-point operand.
    #[test]
    fn abs_returns_the_absolute_value_of_a_negative_float() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup("abs", &mut segment, 0, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(-2.5f64);
        lookup
            .lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<f64>()?, 2.5);
        Ok(())
    }

    /// Returns the negative unit sign for a negative signed operand.
    #[test]
    fn signum_of_a_negative_signed_integer_is_minus_one() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup(
                "signum",
                &mut segment,
                0,
                Span::call_site(),
                Span::call_site(),
            )
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(-7i32);
        lookup
            .lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<i32>()?, -1);
        Ok(())
    }

    /// Preserves NaN through floating-point sign calculation.
    #[test]
    fn signum_of_a_nan_float_is_nan() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup(
                "signum",
                &mut segment,
                0,
                Span::call_site(),
                Span::call_site(),
            )
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(f64::NAN);
        lookup
            .lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert!(segment.call0::<f64>()?.is_nan());
        Ok(())
    }

    /// Returns NaN for the square root of a negative floating-point operand.
    #[test]
    fn sqrt_of_a_negative_float_is_nan() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup(
                "sqrt",
                &mut segment,
                0,
                Span::call_site(),
                Span::call_site(),
            )
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(-4.0f64);
        lookup
            .lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert!(segment.call0::<f64>()?.is_nan());
        Ok(())
    }

    /// Returns the positive square root of a positive floating-point operand.
    #[test]
    fn sqrt_of_a_positive_float_returns_the_positive_root() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup(
                "sqrt",
                &mut segment,
                0,
                Span::call_site(),
                Span::call_site(),
            )
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(9.0f64);
        lookup
            .lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<f64>()?, 3.0);
        Ok(())
    }

    /// Rounds a floating-point operand toward negative infinity.
    #[test]
    fn floor_rounds_a_float_toward_negative_infinity() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup(
                "floor",
                &mut segment,
                0,
                Span::call_site(),
                Span::call_site(),
            )
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(3.7f64);
        lookup
            .lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<f64>()?, 3.0);
        Ok(())
    }

    /// Rounds a floating-point operand toward positive infinity.
    #[test]
    fn ceil_rounds_a_float_toward_positive_infinity() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup(
                "ceil",
                &mut segment,
                0,
                Span::call_site(),
                Span::call_site(),
            )
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(3.2f64);
        lookup
            .lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<f64>()?, 4.0);
        Ok(())
    }

    /// Removes the fractional part of a floating-point operand toward zero.
    #[test]
    fn trunc_rounds_a_float_toward_zero() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup(
                "trunc",
                &mut segment,
                0,
                Span::call_site(),
                Span::call_site(),
            )
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        segment.just(-3.7f64);
        lookup
            .lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())
            .map_err(|_| anyhow::anyhow!("lookup failed"))?;
        assert_eq!(segment.call0::<f64>()?, -3.0);
        Ok(())
    }

    /// Rounds halfway values away from zero without changing ties-to-even rounding.
    #[test]
    fn round_resolves_ties_away_from_zero() {
        assert_eq!(evaluate::<f64>("round(2.5)"), 3.0);
        assert_eq!(evaluate::<f64>("round(-2.5)"), -3.0);
        assert_eq!(evaluate::<f64>("round_ties_even(2.5)"), 2.0);
    }

    /// Rounds a halfway value toward the nearest even integer.
    #[test]
    fn round_ties_even_rounds_halfway_values_to_the_nearest_even_integer() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = cel_parser::CELParser::new(lookup)
            .parse_str("round_ties_even(2.5)")
            .unwrap();

        assert_eq!(segment.call0::<f64>()?, 2.0);
        Ok(())
    }

    /// Returns the signed fractional part through parsed callable dispatch.
    #[test]
    fn fract_returns_the_fractional_part_of_a_float() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = cel_parser::CELParser::new(lookup)
            .parse_str("fract(-3.75f32)")
            .unwrap();

        assert_eq!(segment.call0::<f32>()?, -0.75);
        Ok(())
    }
}
