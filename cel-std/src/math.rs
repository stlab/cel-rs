//! Numeric CEL standard-library functions.
//!
//! Each function follows the pattern `cel-parser`'s built-in `round` uses: a marker
//! struct is pushed for the bare-identifier (arity-0) lookup of the function's name, and
//! consumed by the paired `"()"` call lookup once the marker confirms this call's callee
//! is that function (see `cel-parser/src/op_table.rs`'s `round_scope`).

use anyhow::Result;
use cel_parser::SourceSpan;
use cel_runtime::DynSegment;
use std::any::TypeId;

/// Marker pushed for a bare `min` lookup; consumed by the paired `"()"` call.
struct MinFn;
/// Marker pushed for a bare `max` lookup; consumed by the paired `"()"` call.
struct MaxFn;
/// Marker pushed for a bare `clamp` lookup; consumed by the paired `"()"` call.
struct ClampFn;

/// `min(a, b) = a.min(b)`, `max(a, b) = a.max(b)` over all 14 numeric types — `Ord::min`/
/// `max` for integers, the inherent (NaN-avoiding) `f32`/`f64` `min`/`max` for floats.
///
/// Declines the call (returns `Ok(false)`) when the two operands have different types.
pub(crate) fn min_max_scope(
    name: &str,
    segment: &mut DynSegment,
    num_operands: usize,
    _span: SourceSpan,
) -> Result<bool> {
    match (name, num_operands) {
        ("min", 0) => {
            segment.op0(|| MinFn);
            Ok(true)
        }
        ("max", 0) => {
            segment.op0(|| MaxFn);
            Ok(true)
        }
        ("()", 3) => {
            let top = segment.peek_stack_infos(3);
            if top.len() != 3 || top[1].type_id != top[2].type_id {
                return Ok(false);
            }
            let callee_type = top[0].type_id;
            let operand_type = top[1].type_id;

            macro_rules! dispatch {
                ($marker:ty, $method:ident, [$($t:ty),+ $(,)?]) => {
                    if callee_type == TypeId::of::<$marker>() {
                        $(
                            if operand_type == TypeId::of::<$t>() {
                                segment.op3(|_callee: $marker, a: $t, b: $t| a.$method(b))?;
                                return Ok(true);
                            }
                        )+
                        return Ok(false);
                    }
                };
            }

            dispatch!(
                MinFn,
                min,
                [
                    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64
                ]
            );
            dispatch!(
                MaxFn,
                max,
                [
                    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64
                ]
            );
            Ok(false)
        }
        _ => Ok(false),
    }
}

/// `clamp(x, lo, hi)` bounds `x` to `[lo, hi]`, over all 14 numeric types.
///
/// Dispatches as two chained ops: the first computes the clamped value from `x`/`lo`/`hi`
/// alone (and can fail if `lo > hi`); the second discards the still-buried `ClampFn`
/// marker and passes the result through unchanged.
///
/// - Precondition: `x`, `lo`, and `hi` all have the same type.
///
/// # Errors
///
/// Returns `Err("invalid clamp bounds")` if `!(lo <= hi)` — this single comparison is
/// `false` whenever either bound is `NaN`, so no separate `NaN` check is needed.
pub(crate) fn clamp_scope(
    name: &str,
    segment: &mut DynSegment,
    num_operands: usize,
    _span: SourceSpan,
) -> Result<bool> {
    match (name, num_operands) {
        ("clamp", 0) => {
            segment.op0(|| ClampFn);
            Ok(true)
        }
        ("()", 4) => {
            let top = segment.peek_stack_infos(4);
            if top.len() != 4
                || top[0].type_id != TypeId::of::<ClampFn>()
                || top[1].type_id != top[2].type_id
                || top[1].type_id != top[3].type_id
            {
                return Ok(false);
            }
            let operand_type = top[1].type_id;

            macro_rules! dispatch {
                ([$($t:ty),+ $(,)?]) => {
                    $(
                        if operand_type == TypeId::of::<$t>() {
                            segment.op3r(move |x: $t, lo: $t, hi: $t| {
                                if lo <= hi {
                                    Ok(x.clamp(lo, hi))
                                } else {
                                    Err(anyhow::anyhow!("invalid clamp bounds"))
                                }
                            })?;
                            segment.op2(|_callee: ClampFn, result: $t| result)?;
                            return Ok(true);
                        }
                    )+
                };
            }

            dispatch!([
                u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64
            ]);
            Ok(false)
        }
        _ => Ok(false),
    }
}

/// Marker pushed for a bare `abs` lookup; consumed by the paired `"()"` call.
struct AbsFn;

/// `abs(x)` over signed integers (checked; `Err` on overflow) and floats (infallible).
///
/// # Errors
///
/// Returns `Err("arithmetic overflow")` if `x` is a signed integer at its type's minimum
/// value (the one value whose absolute value doesn't fit in that type).
pub(crate) fn abs_scope(
    name: &str,
    segment: &mut DynSegment,
    num_operands: usize,
    _span: SourceSpan,
) -> Result<bool> {
    match (name, num_operands) {
        ("abs", 0) => {
            segment.op0(|| AbsFn);
            Ok(true)
        }
        ("()", 2) => {
            let top = segment.peek_stack_infos(2);
            if top.len() != 2 || top[0].type_id != TypeId::of::<AbsFn>() {
                return Ok(false);
            }
            let operand_type = top[1].type_id;

            macro_rules! dispatch_checked {
                ([$($t:ty),+ $(,)?]) => {
                    $(
                        if operand_type == TypeId::of::<$t>() {
                            segment.op2r(|_callee: AbsFn, x: $t| {
                                x.checked_abs()
                                    .ok_or_else(|| anyhow::anyhow!("arithmetic overflow"))
                            })?;
                            return Ok(true);
                        }
                    )+
                };
            }
            macro_rules! dispatch_float {
                ([$($t:ty),+ $(,)?]) => {
                    $(
                        if operand_type == TypeId::of::<$t>() {
                            segment.op2(|_callee: AbsFn, x: $t| x.abs())?;
                            return Ok(true);
                        }
                    )+
                };
            }

            dispatch_checked!([i8, i16, i32, i64, i128, isize]);
            dispatch_float!([f32, f64]);
            Ok(false)
        }
        _ => Ok(false),
    }
}

/// Marker pushed for a bare `signum` lookup; consumed by the paired `"()"` call.
struct SignumFn;
/// Marker pushed for a bare `sqrt` lookup; consumed by the paired `"()"` call.
struct SqrtFn;
/// Marker pushed for a bare `floor` lookup; consumed by the paired `"()"` call.
struct FloorFn;
/// Marker pushed for a bare `ceil` lookup; consumed by the paired `"()"` call.
struct CeilFn;
/// Marker pushed for a bare `trunc` lookup; consumed by the paired `"()"` call.
struct TruncFn;

/// `signum(x)` (signed integers and floats), `sqrt(x)`/`floor(x)`/`ceil(x)`/`trunc(x)`
/// (floats only) — all infallible, matching the semantics of Rust's method of the same
/// name (e.g. `sqrt` of a negative float yields `NaN`, not an error).
pub(crate) fn unary_math_scope(
    name: &str,
    segment: &mut DynSegment,
    num_operands: usize,
    _span: SourceSpan,
) -> Result<bool> {
    match (name, num_operands) {
        ("signum", 0) => {
            segment.op0(|| SignumFn);
            Ok(true)
        }
        ("sqrt", 0) => {
            segment.op0(|| SqrtFn);
            Ok(true)
        }
        ("floor", 0) => {
            segment.op0(|| FloorFn);
            Ok(true)
        }
        ("ceil", 0) => {
            segment.op0(|| CeilFn);
            Ok(true)
        }
        ("trunc", 0) => {
            segment.op0(|| TruncFn);
            Ok(true)
        }
        ("()", 2) => {
            let top = segment.peek_stack_infos(2);
            if top.len() != 2 {
                return Ok(false);
            }
            let callee_type = top[0].type_id;
            let operand_type = top[1].type_id;

            macro_rules! dispatch {
                ($marker:ty, $method:ident, [$($t:ty),+ $(,)?]) => {
                    if callee_type == TypeId::of::<$marker>() {
                        $(
                            if operand_type == TypeId::of::<$t>() {
                                segment.op2(|_callee: $marker, x: $t| x.$method())?;
                                return Ok(true);
                            }
                        )+
                        return Ok(false);
                    }
                };
            }

            dispatch!(SignumFn, signum, [i8, i16, i32, i64, i128, isize, f32, f64]);
            dispatch!(SqrtFn, sqrt, [f32, f64]);
            dispatch!(FloorFn, floor, [f32, f64]);
            dispatch!(CeilFn, ceil, [f32, f64]);
            dispatch!(TruncFn, trunc, [f32, f64]);
            Ok(false)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install;
    use cel_parser::OpLookup;
    use proc_macro2::Span;

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
}
