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

/// `min(a, b) = a.min(b)`, `max(a, b) = a.max(b)` over all 14 numeric types — `Ord::min`/
/// `max` for integers, the inherent (NaN-avoiding) `f32`/`f64` `min`/`max` for floats.
///
/// - Precondition: `a` and `b` have the same type.
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
}
