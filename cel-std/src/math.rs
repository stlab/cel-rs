//! Numeric CEL standard-library functions.
//!
//! Installations register complete operand signatures shared by ordinary calls and
//! direct named operations. Independent libraries can add disjoint overloads.

use anyhow::Result;
use cel_parser::{OpLookup, OperationSignature, ResolvedLeafType};
use cel_runtime::ArrayElementType;
use std::sync::Arc;

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

/// Returns a numeric leaf's complete concrete descriptor.
fn numeric_type<T: 'static>() -> cel_parser::ResolvedType {
    ResolvedLeafType::new(
        std::any::type_name::<T>(),
        ArrayElementType::leaf::<T>().expect("numeric types are non-array leaves"),
    )
    .into()
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

    macro_rules! extrema_and_clamp {
        ([$($t:ty),+ $(,)?]) => { $(
            lookup.register_operation(OperationSignature {
                name: "min", operands: vec![numeric_type::<$t>(); 2],
                result: numeric_type::<$t>(),
                emit: Arc::new(|segment, _| segment.op2(|a: $t, b: $t| a.min(b))),
            })?;
            lookup.register_operation(OperationSignature {
                name: "max", operands: vec![numeric_type::<$t>(); 2],
                result: numeric_type::<$t>(),
                emit: Arc::new(|segment, _| segment.op2(|a: $t, b: $t| a.max(b))),
            })?;
            lookup.register_operation(OperationSignature {
                name: "clamp", operands: vec![numeric_type::<$t>(); 3],
                result: numeric_type::<$t>(),
                emit: Arc::new(|segment, _| segment.op3r(|x: $t, lo: $t, hi: $t| {
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
                emit: Arc::new(|segment, _| segment.op1(|x: $t| x.$method())),
            })?;
        )+ };
    }
    macro_rules! checked_abs {
        ([$($t:ty),+ $(,)?]) => { $(
            lookup.register_operation(OperationSignature {
                name: "abs", operands: vec![numeric_type::<$t>()],
                result: numeric_type::<$t>(),
                emit: Arc::new(|segment, _| segment.op1r(|x: $t| {
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install;
    use cel_parser::OpLookup;
    use cel_runtime::DynSegment;
    use proc_macro2::Span;

    /// Confirms independent unary overloads compose with numeric calls in either install order.
    #[test]
    fn independent_min_overload_composes_with_standard_library() -> Result<()> {
        use cel_parser::{OperationSignature, ResolvedLeafType};
        use cel_runtime::ArrayElementType;
        use std::sync::Arc;

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
                operands: vec![
                    ResolvedLeafType::new("Corner", ArrayElementType::leaf::<Corner>().unwrap())
                        .into(),
                ],
                result: ResolvedLeafType::new("f64", ArrayElementType::leaf::<f64>().unwrap())
                    .into(),
                emit: Arc::new(|segment, _| segment.op1(|corner: Corner| corner.0)),
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

    #[test]
    fn round_is_not_a_standard_library_function() {
        let mut lookup = OpLookup::new();
        install(&mut lookup);

        assert!(
            cel_parser::CELParser::new(lookup)
                .parse_str("round(-3.5)")
                .is_err()
        );
    }

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
