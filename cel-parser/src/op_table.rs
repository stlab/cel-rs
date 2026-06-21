//! Operation table for dynamically dispatching operations based on type signatures.
//!
//! This module provides a scope-based registry for operations that can be looked up
//! based on an operation name (string) and the types of the operands. Built-in operations
//! use compile-time hash tables (via `phf`) for efficient lookup, while custom operations
//! can be added dynamically through scope functions.
//!
//! # Design
//!
//! - **Operator symbols as names**: Operations are identified by their operator symbols
//!   (e.g., `"+"`, `"-"`, `"*"`) to avoid conflicts with valid identifiers.
//! - **Function pointers**: Built-in operations use stateless function pointers for
//!   zero-allocation dispatch.
//! - **Scope stack**: Custom operations are handled through a stack of scope functions
//!   that can be pushed and popped as needed.
//! - **Type optimization**: Most built-in operations are homogeneous (both operands share
//!   a type), so signatures store a primary `TypeId` plus arity. Heterogeneous binary ops
//!   (e.g. shifts, where the RHS is always `u32`) additionally store an RHS `TypeId` index.
//!
//! # Semantics
//!
//! Built-in operations follow Rust language semantics. Deviations are:
//!
//! - **Signed integer overflow**: CEL returns `Err` rather than panicking (debug) or wrapping
//!   (release). Use wrapping arithmetic explicitly if overflow is intended.
//! - **Bit-shift with out-of-range count**: CEL returns `Err` rather than panicking (debug)
//!   or masking the shift count (release).

use anyhow::{Result, anyhow};
use cel_runtime::DynSegment;
use once_cell::sync::Lazy;
use phf::phf_map;
use std::any::TypeId;

use crate::SourceSpan;

/// A function that pushes an operation onto a DynSegment.
///
/// Receives the segment and the source span of the expression that triggered
/// this operation. This is a simple function pointer since built-in operations
/// have no state.
pub type OpFn = fn(&mut DynSegment, SourceSpan) -> Result<()>;

/// A scope function that attempts to resolve and apply an operation.
///
/// Receives the operation name, the segment, the number of operands on top of the stack,
/// and the source span of the expression. The scope may call
/// `segment.peek_stack_infos(num_operands)` to inspect types. Returns `Ok(true)` if
/// handled, `Ok(false)` if not found, or `Err` on error.
///
/// Error messages returned by scope functions surface verbatim to the user. They should be
/// lowercase, end without a period, and wrap identifiers and type names in backticks.
pub type ScopeFn =
    Box<dyn Fn(&str, &mut DynSegment, usize, SourceSpan) -> Result<bool> + Send + Sync>;

/// A signature for a built-in operation.
///
/// For homogeneous ops (e.g. `u32 + u32`) `rhs_type_id_index` equals `type_id_index`.
/// For heterogeneous binary ops (e.g. `u64 << u32`) they differ.
#[derive(Clone, Copy)]
struct OpSignature {
    /// Index into TYPE_IDS for the LHS (or sole) operand type.
    type_id_index: usize,
    /// Index into TYPE_IDS for the RHS operand type; equals `type_id_index` for homogeneous ops.
    rhs_type_id_index: usize,
    /// Number of operands this operation accepts.
    arity: u8,
    /// Function pointer to the operation implementation.
    op_fn: OpFn,
}

impl OpSignature {
    /// Returns the `TypeId` of the LHS (or sole) operand.
    fn lhs_type_id(&self) -> TypeId {
        TYPE_IDS[self.type_id_index]
    }

    /// Returns the `TypeId` of the RHS operand.
    fn rhs_type_id(&self) -> TypeId {
        TYPE_IDS[self.rhs_type_id_index]
    }
}

/// Single lazy-initialized vector containing all unique TypeIds for built-in types.
///
/// This avoids duplicating TypeId storage across all operation signatures.
static TYPE_IDS: Lazy<Vec<TypeId>> = Lazy::new(|| {
    vec![
        TypeId::of::<u8>(),
        TypeId::of::<u16>(),
        TypeId::of::<u32>(),
        TypeId::of::<u64>(),
        TypeId::of::<u128>(),
        TypeId::of::<usize>(),
        TypeId::of::<i8>(),
        TypeId::of::<i16>(),
        TypeId::of::<i32>(),
        TypeId::of::<i64>(),
        TypeId::of::<i128>(),
        TypeId::of::<isize>(),
        TypeId::of::<f32>(),
        TypeId::of::<f64>(),
        TypeId::of::<bool>(),
        TypeId::of::<String>(),
    ]
});

// Type index constants for readability
const TYPE_U8: usize = 0;
const TYPE_U16: usize = 1;
const TYPE_U32: usize = 2;
const TYPE_U64: usize = 3;
const TYPE_U128: usize = 4;
const TYPE_USIZE: usize = 5;
const TYPE_I8: usize = 6;
const TYPE_I16: usize = 7;
const TYPE_I32: usize = 8;
const TYPE_I64: usize = 9;
const TYPE_I128: usize = 10;
const TYPE_ISIZE: usize = 11;
const TYPE_F32: usize = 12;
const TYPE_F64: usize = 13;
const TYPE_BOOL: usize = 14;
const TYPE_STR: usize = 15;

// Helper macros to reduce boilerplate in signature definitions.
// `sig!` builds a homogeneous signature; `sig_het!` a heterogeneous binary one.
macro_rules! sig {
    ($type_idx:expr, $arity:expr, $closure:expr) => {
        OpSignature {
            type_id_index: $type_idx,
            rhs_type_id_index: $type_idx,
            arity: $arity,
            op_fn: $closure,
        }
    };
}

macro_rules! sig_het {
    ($lhs_idx:expr, $rhs_idx:expr, $closure:expr) => {
        OpSignature {
            type_id_index: $lhs_idx,
            rhs_type_id_index: $rhs_idx,
            arity: 2,
            op_fn: $closure,
        }
    };
}

// Addition signatures
static ADD_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, _span| seg
        .op2(|a: u8, b: u8| a.wrapping_add(b))),
    sig!(TYPE_U16, 2, |seg, _span| seg
        .op2(|a: u16, b: u16| a.wrapping_add(b))),
    sig!(TYPE_U32, 2, |seg, _span| seg
        .op2(|a: u32, b: u32| a.wrapping_add(b))),
    sig!(TYPE_U64, 2, |seg, _span| seg
        .op2(|a: u64, b: u64| a.wrapping_add(b))),
    sig!(TYPE_U128, 2, |seg, _span| seg
        .op2(|a: u128, b: u128| a.wrapping_add(b))),
    sig!(TYPE_USIZE, 2, |seg, _span| seg
        .op2(|a: usize, b: usize| a.wrapping_add(b))),
    sig!(TYPE_I8, 2, |seg, _span| seg.op2r(|a: i8, b: i8| a
        .checked_add(b)
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_I16, 2, |seg, _span| seg.op2r(|a: i16, b: i16| a
        .checked_add(b)
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_I32, 2, |seg, _span| seg.op2r(|a: i32, b: i32| a
        .checked_add(b)
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_I64, 2, |seg, _span| seg.op2r(|a: i64, b: i64| a
        .checked_add(b)
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_I128, 2, |seg, _span| seg.op2r(|a: i128, b: i128| a
        .checked_add(b)
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_ISIZE, 2, |seg, _span| seg.op2r(
        |a: isize, b: isize| a
            .checked_add(b)
            .ok_or_else(|| anyhow!("arithmetic overflow"))
    )),
    sig!(TYPE_F32, 2, |seg, _span| seg.op2(|a: f32, b: f32| a + b)),
    sig!(TYPE_F64, 2, |seg, _span| seg.op2(|a: f64, b: f64| a + b)),
    sig!(TYPE_STR, 2, |seg, _span| seg
        .op2(|a: String, b: String| a + &b)),
];

// Subtraction signatures (both binary and unary)
static SUB_SIGNATURES: &[OpSignature] = &[
    // Binary subtraction
    sig!(TYPE_U8, 2, |seg, _span| seg
        .op2(|a: u8, b: u8| a.wrapping_sub(b))),
    sig!(TYPE_U16, 2, |seg, _span| seg
        .op2(|a: u16, b: u16| a.wrapping_sub(b))),
    sig!(TYPE_U32, 2, |seg, _span| seg
        .op2(|a: u32, b: u32| a.wrapping_sub(b))),
    sig!(TYPE_U64, 2, |seg, _span| seg
        .op2(|a: u64, b: u64| a.wrapping_sub(b))),
    sig!(TYPE_U128, 2, |seg, _span| seg
        .op2(|a: u128, b: u128| a.wrapping_sub(b))),
    sig!(TYPE_USIZE, 2, |seg, _span| seg
        .op2(|a: usize, b: usize| a.wrapping_sub(b))),
    sig!(TYPE_I8, 2, |seg, _span| seg.op2r(|a: i8, b: i8| a
        .checked_sub(b)
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_I16, 2, |seg, _span| seg.op2r(|a: i16, b: i16| a
        .checked_sub(b)
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_I32, 2, |seg, _span| seg.op2r(|a: i32, b: i32| a
        .checked_sub(b)
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_I64, 2, |seg, _span| seg.op2r(|a: i64, b: i64| a
        .checked_sub(b)
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_I128, 2, |seg, _span| seg.op2r(|a: i128, b: i128| a
        .checked_sub(b)
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_ISIZE, 2, |seg, _span| seg.op2r(
        |a: isize, b: isize| a
            .checked_sub(b)
            .ok_or_else(|| anyhow!("arithmetic overflow"))
    )),
    sig!(TYPE_F32, 2, |seg, _span| seg.op2(|a: f32, b: f32| a - b)),
    sig!(TYPE_F64, 2, |seg, _span| seg.op2(|a: f64, b: f64| a - b)),
    // Unary negation
    sig!(TYPE_I8, 1, |seg, _span| seg.op1r(|a: i8| a
        .checked_neg()
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_I16, 1, |seg, _span| seg.op1r(|a: i16| a
        .checked_neg()
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_I32, 1, |seg, _span| seg.op1r(|a: i32| a
        .checked_neg()
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_I64, 1, |seg, _span| seg.op1r(|a: i64| a
        .checked_neg()
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_I128, 1, |seg, _span| seg.op1r(|a: i128| a
        .checked_neg()
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_ISIZE, 1, |seg, _span| seg.op1r(|a: isize| a
        .checked_neg()
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_F32, 1, |seg, _span| seg.op1(|a: f32| -a)),
    sig!(TYPE_F64, 1, |seg, _span| seg.op1(|a: f64| -a)),
];

// Multiplication signatures
static MUL_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, _span| seg
        .op2(|a: u8, b: u8| a.wrapping_mul(b))),
    sig!(TYPE_U16, 2, |seg, _span| seg
        .op2(|a: u16, b: u16| a.wrapping_mul(b))),
    sig!(TYPE_U32, 2, |seg, _span| seg
        .op2(|a: u32, b: u32| a.wrapping_mul(b))),
    sig!(TYPE_U64, 2, |seg, _span| seg
        .op2(|a: u64, b: u64| a.wrapping_mul(b))),
    sig!(TYPE_U128, 2, |seg, _span| seg
        .op2(|a: u128, b: u128| a.wrapping_mul(b))),
    sig!(TYPE_USIZE, 2, |seg, _span| seg
        .op2(|a: usize, b: usize| a.wrapping_mul(b))),
    sig!(TYPE_I8, 2, |seg, _span| seg.op2r(|a: i8, b: i8| a
        .checked_mul(b)
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_I16, 2, |seg, _span| seg.op2r(|a: i16, b: i16| a
        .checked_mul(b)
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_I32, 2, |seg, _span| seg.op2r(|a: i32, b: i32| a
        .checked_mul(b)
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_I64, 2, |seg, _span| seg.op2r(|a: i64, b: i64| a
        .checked_mul(b)
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_I128, 2, |seg, _span| seg.op2r(|a: i128, b: i128| a
        .checked_mul(b)
        .ok_or_else(|| anyhow!("arithmetic overflow")))),
    sig!(TYPE_ISIZE, 2, |seg, _span| seg.op2r(
        |a: isize, b: isize| a
            .checked_mul(b)
            .ok_or_else(|| anyhow!("arithmetic overflow"))
    )),
    sig!(TYPE_F32, 2, |seg, _span| seg.op2(|a: f32, b: f32| a * b)),
    sig!(TYPE_F64, 2, |seg, _span| seg.op2(|a: f64, b: f64| a * b)),
];

// Division signatures
//
// Integer division uses `checked_div` via `op2r` so that division by zero returns an error
// instead of panicking. Float division keeps `op2` (IEEE 754 defines x/0.0 as inf/nan).
static DIV_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, _span| seg.op2r(|a: u8, b: u8| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_U16, 2, |seg, _span| seg.op2r(|a: u16, b: u16| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_U32, 2, |seg, _span| seg.op2r(|a: u32, b: u32| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_U64, 2, |seg, _span| seg.op2r(|a: u64, b: u64| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_U128, 2, |seg, _span| seg.op2r(|a: u128, b: u128| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_USIZE, 2, |seg, _span| seg.op2r(
        |a: usize, b: usize| a.checked_div(b).ok_or_else(|| anyhow!("division by zero"))
    )),
    sig!(TYPE_I8, 2, |seg, _span| seg.op2r(|a: i8, b: i8| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_I16, 2, |seg, _span| seg.op2r(|a: i16, b: i16| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_I32, 2, |seg, _span| seg.op2r(|a: i32, b: i32| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_I64, 2, |seg, _span| seg.op2r(|a: i64, b: i64| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_I128, 2, |seg, _span| seg.op2r(|a: i128, b: i128| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_ISIZE, 2, |seg, _span| seg.op2r(
        |a: isize, b: isize| a.checked_div(b).ok_or_else(|| anyhow!("division by zero"))
    )),
    sig!(TYPE_F32, 2, |seg, _span| seg.op2(|a: f32, b: f32| a / b)),
    sig!(TYPE_F64, 2, |seg, _span| seg.op2(|a: f64, b: f64| a / b)),
];

// Modulo signatures
//
// Integer modulo uses `checked_rem` via `op2r` so that division by zero returns an error
// instead of panicking. Float modulo keeps `op2` (x % 0.0 yields NaN without panicking).
static MOD_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, _span| seg.op2r(|a: u8, b: u8| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_U16, 2, |seg, _span| seg.op2r(|a: u16, b: u16| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_U32, 2, |seg, _span| seg.op2r(|a: u32, b: u32| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_U64, 2, |seg, _span| seg.op2r(|a: u64, b: u64| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_U128, 2, |seg, _span| seg.op2r(|a: u128, b: u128| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_USIZE, 2, |seg, _span| seg.op2r(
        |a: usize, b: usize| a.checked_rem(b).ok_or_else(|| anyhow!("division by zero"))
    )),
    sig!(TYPE_I8, 2, |seg, _span| seg.op2r(|a: i8, b: i8| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_I16, 2, |seg, _span| seg.op2r(|a: i16, b: i16| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_I32, 2, |seg, _span| seg.op2r(|a: i32, b: i32| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_I64, 2, |seg, _span| seg.op2r(|a: i64, b: i64| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_I128, 2, |seg, _span| seg.op2r(|a: i128, b: i128| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero")))),
    sig!(TYPE_ISIZE, 2, |seg, _span| seg.op2r(
        |a: isize, b: isize| a.checked_rem(b).ok_or_else(|| anyhow!("division by zero"))
    )),
    sig!(TYPE_F32, 2, |seg, _span| seg.op2(|a: f32, b: f32| a % b)),
    sig!(TYPE_F64, 2, |seg, _span| seg.op2(|a: f64, b: f64| a % b)),
];

// Bitwise AND signatures
static BITWISE_AND_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, _span| seg.op2(|a: u8, b: u8| a & b)),
    sig!(TYPE_U16, 2, |seg, _span| seg.op2(|a: u16, b: u16| a & b)),
    sig!(TYPE_U32, 2, |seg, _span| seg.op2(|a: u32, b: u32| a & b)),
    sig!(TYPE_U64, 2, |seg, _span| seg.op2(|a: u64, b: u64| a & b)),
    sig!(TYPE_U128, 2, |seg, _span| seg.op2(|a: u128, b: u128| a & b)),
    sig!(TYPE_USIZE, 2, |seg, _span| seg
        .op2(|a: usize, b: usize| a & b)),
    sig!(TYPE_I8, 2, |seg, _span| seg.op2(|a: i8, b: i8| a & b)),
    sig!(TYPE_I16, 2, |seg, _span| seg.op2(|a: i16, b: i16| a & b)),
    sig!(TYPE_I32, 2, |seg, _span| seg.op2(|a: i32, b: i32| a & b)),
    sig!(TYPE_I64, 2, |seg, _span| seg.op2(|a: i64, b: i64| a & b)),
    sig!(TYPE_I128, 2, |seg, _span| seg.op2(|a: i128, b: i128| a & b)),
    sig!(TYPE_ISIZE, 2, |seg, _span| seg
        .op2(|a: isize, b: isize| a & b)),
];

// Bitwise OR signatures
static BITWISE_OR_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, _span| seg.op2(|a: u8, b: u8| a | b)),
    sig!(TYPE_U16, 2, |seg, _span| seg.op2(|a: u16, b: u16| a | b)),
    sig!(TYPE_U32, 2, |seg, _span| seg.op2(|a: u32, b: u32| a | b)),
    sig!(TYPE_U64, 2, |seg, _span| seg.op2(|a: u64, b: u64| a | b)),
    sig!(TYPE_U128, 2, |seg, _span| seg.op2(|a: u128, b: u128| a | b)),
    sig!(TYPE_USIZE, 2, |seg, _span| seg
        .op2(|a: usize, b: usize| a | b)),
    sig!(TYPE_I8, 2, |seg, _span| seg.op2(|a: i8, b: i8| a | b)),
    sig!(TYPE_I16, 2, |seg, _span| seg.op2(|a: i16, b: i16| a | b)),
    sig!(TYPE_I32, 2, |seg, _span| seg.op2(|a: i32, b: i32| a | b)),
    sig!(TYPE_I64, 2, |seg, _span| seg.op2(|a: i64, b: i64| a | b)),
    sig!(TYPE_I128, 2, |seg, _span| seg.op2(|a: i128, b: i128| a | b)),
    sig!(TYPE_ISIZE, 2, |seg, _span| seg
        .op2(|a: isize, b: isize| a | b)),
];

// Bitwise XOR signatures
static BITWISE_XOR_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, _span| seg.op2(|a: u8, b: u8| a ^ b)),
    sig!(TYPE_U16, 2, |seg, _span| seg.op2(|a: u16, b: u16| a ^ b)),
    sig!(TYPE_U32, 2, |seg, _span| seg.op2(|a: u32, b: u32| a ^ b)),
    sig!(TYPE_U64, 2, |seg, _span| seg.op2(|a: u64, b: u64| a ^ b)),
    sig!(TYPE_U128, 2, |seg, _span| seg.op2(|a: u128, b: u128| a ^ b)),
    sig!(TYPE_USIZE, 2, |seg, _span| seg
        .op2(|a: usize, b: usize| a ^ b)),
    sig!(TYPE_I8, 2, |seg, _span| seg.op2(|a: i8, b: i8| a ^ b)),
    sig!(TYPE_I16, 2, |seg, _span| seg.op2(|a: i16, b: i16| a ^ b)),
    sig!(TYPE_I32, 2, |seg, _span| seg.op2(|a: i32, b: i32| a ^ b)),
    sig!(TYPE_I64, 2, |seg, _span| seg.op2(|a: i64, b: i64| a ^ b)),
    sig!(TYPE_I128, 2, |seg, _span| seg.op2(|a: i128, b: i128| a ^ b)),
    sig!(TYPE_ISIZE, 2, |seg, _span| seg
        .op2(|a: isize, b: isize| a ^ b)),
];

// Macros that push shift signatures onto a Vec as statements.
// Rust macros may not expand to multiple comma-separated expressions in a static
// array initialiser, so we use Lazy<Vec<_>> with push statements instead.
//
// RHS → u32 conversion (required by checked_shl / checked_shr):
//   u8, u16              : u32::from  (infallible widening)
//   u32                  : identity
//   u64 / u128 / usize   : u32::try_from; fails if value > u32::MAX
//   all signed types     : u32::try_from; fails if value < 0 or > u32::MAX
//   In all failure cases the error is "shift overflow", matching Rust's
//   debug-mode panic for shift-with-overflow.
macro_rules! shl_push {
    ($v:ident, $lhs_idx:expr, $lhs_ty:ty) => {
        $v.push(sig_het!($lhs_idx, TYPE_U8, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: u8| a
                .checked_shl(u32::from(b))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_U16, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: u16| a
                .checked_shl(u32::from(b))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_U32, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: u32| a.checked_shl(b).ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_U64, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: u64| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_U128, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: u128| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_USIZE, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: usize| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I8, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: i8| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I16, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: i16| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I32, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: i32| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I64, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: i64| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I128, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: i128| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_ISIZE, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: isize| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
    };
}

macro_rules! shr_push {
    ($v:ident, $lhs_idx:expr, $lhs_ty:ty) => {
        $v.push(sig_het!($lhs_idx, TYPE_U8, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: u8| a
                .checked_shr(u32::from(b))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_U16, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: u16| a
                .checked_shr(u32::from(b))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_U32, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: u32| a.checked_shr(b).ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_U64, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: u64| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_U128, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: u128| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_USIZE, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: usize| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I8, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: i8| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I16, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: i16| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I32, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: i32| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I64, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: i64| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I128, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: i128| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_ISIZE, |seg, _span| seg.op2r(
            |a: $lhs_ty, b: isize| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
        )));
    };
}

// Left shift: all 144 combinations T << U for integer T and U (mirrors Rust's Shl implementations).
// Stored as Lazy<Vec<_>> because the shl_push! macro expands to statements, not array items.
static LEFT_SHIFT_SIGNATURES: Lazy<Vec<OpSignature>> = Lazy::new(|| {
    let mut v = Vec::with_capacity(144);
    shl_push!(v, TYPE_U8, u8);
    shl_push!(v, TYPE_U16, u16);
    shl_push!(v, TYPE_U32, u32);
    shl_push!(v, TYPE_U64, u64);
    shl_push!(v, TYPE_U128, u128);
    shl_push!(v, TYPE_USIZE, usize);
    shl_push!(v, TYPE_I8, i8);
    shl_push!(v, TYPE_I16, i16);
    shl_push!(v, TYPE_I32, i32);
    shl_push!(v, TYPE_I64, i64);
    shl_push!(v, TYPE_I128, i128);
    shl_push!(v, TYPE_ISIZE, isize);
    v
});

// Right shift: all 144 combinations T >> U for integer T and U (mirrors Rust's Shr implementations).
static RIGHT_SHIFT_SIGNATURES: Lazy<Vec<OpSignature>> = Lazy::new(|| {
    let mut v = Vec::with_capacity(144);
    shr_push!(v, TYPE_U8, u8);
    shr_push!(v, TYPE_U16, u16);
    shr_push!(v, TYPE_U32, u32);
    shr_push!(v, TYPE_U64, u64);
    shr_push!(v, TYPE_U128, u128);
    shr_push!(v, TYPE_USIZE, usize);
    shr_push!(v, TYPE_I8, i8);
    shr_push!(v, TYPE_I16, i16);
    shr_push!(v, TYPE_I32, i32);
    shr_push!(v, TYPE_I64, i64);
    shr_push!(v, TYPE_I128, i128);
    shr_push!(v, TYPE_ISIZE, isize);
    v
});

// Logical NOT signatures
static LOGICAL_NOT_SIGNATURES: &[OpSignature] =
    &[sig!(TYPE_BOOL, 1, |seg, _span| seg.op1(|a: bool| !a))];

// Equality signatures
static EQUAL_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, _span| seg.op2(|a: u8, b: u8| a == b)),
    sig!(TYPE_U16, 2, |seg, _span| seg.op2(|a: u16, b: u16| a == b)),
    sig!(TYPE_U32, 2, |seg, _span| seg.op2(|a: u32, b: u32| a == b)),
    sig!(TYPE_U64, 2, |seg, _span| seg.op2(|a: u64, b: u64| a == b)),
    sig!(TYPE_U128, 2, |seg, _span| seg
        .op2(|a: u128, b: u128| a == b)),
    sig!(TYPE_USIZE, 2, |seg, _span| seg
        .op2(|a: usize, b: usize| a == b)),
    sig!(TYPE_I8, 2, |seg, _span| seg.op2(|a: i8, b: i8| a == b)),
    sig!(TYPE_I16, 2, |seg, _span| seg.op2(|a: i16, b: i16| a == b)),
    sig!(TYPE_I32, 2, |seg, _span| seg.op2(|a: i32, b: i32| a == b)),
    sig!(TYPE_I64, 2, |seg, _span| seg.op2(|a: i64, b: i64| a == b)),
    sig!(TYPE_I128, 2, |seg, _span| seg
        .op2(|a: i128, b: i128| a == b)),
    sig!(TYPE_ISIZE, 2, |seg, _span| seg
        .op2(|a: isize, b: isize| a == b)),
    sig!(TYPE_F32, 2, |seg, _span| seg.op2(|a: f32, b: f32| a == b)),
    sig!(TYPE_F64, 2, |seg, _span| seg.op2(|a: f64, b: f64| a == b)),
    sig!(TYPE_BOOL, 2, |seg, _span| seg
        .op2(|a: bool, b: bool| a == b)),
    sig!(TYPE_STR, 2, |seg, _span| seg
        .op2(|a: String, b: String| a == b)),
];

// Inequality signatures
static NOT_EQUAL_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, _span| seg.op2(|a: u8, b: u8| a != b)),
    sig!(TYPE_U16, 2, |seg, _span| seg.op2(|a: u16, b: u16| a != b)),
    sig!(TYPE_U32, 2, |seg, _span| seg.op2(|a: u32, b: u32| a != b)),
    sig!(TYPE_U64, 2, |seg, _span| seg.op2(|a: u64, b: u64| a != b)),
    sig!(TYPE_U128, 2, |seg, _span| seg
        .op2(|a: u128, b: u128| a != b)),
    sig!(TYPE_USIZE, 2, |seg, _span| seg
        .op2(|a: usize, b: usize| a != b)),
    sig!(TYPE_I8, 2, |seg, _span| seg.op2(|a: i8, b: i8| a != b)),
    sig!(TYPE_I16, 2, |seg, _span| seg.op2(|a: i16, b: i16| a != b)),
    sig!(TYPE_I32, 2, |seg, _span| seg.op2(|a: i32, b: i32| a != b)),
    sig!(TYPE_I64, 2, |seg, _span| seg.op2(|a: i64, b: i64| a != b)),
    sig!(TYPE_I128, 2, |seg, _span| seg
        .op2(|a: i128, b: i128| a != b)),
    sig!(TYPE_ISIZE, 2, |seg, _span| seg
        .op2(|a: isize, b: isize| a != b)),
    sig!(TYPE_F32, 2, |seg, _span| seg.op2(|a: f32, b: f32| a != b)),
    sig!(TYPE_F64, 2, |seg, _span| seg.op2(|a: f64, b: f64| a != b)),
    sig!(TYPE_BOOL, 2, |seg, _span| seg
        .op2(|a: bool, b: bool| a != b)),
    sig!(TYPE_STR, 2, |seg, _span| seg
        .op2(|a: String, b: String| a != b)),
];

// Less than signatures
static LESS_THAN_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, _span| seg.op2(|a: u8, b: u8| a < b)),
    sig!(TYPE_U16, 2, |seg, _span| seg.op2(|a: u16, b: u16| a < b)),
    sig!(TYPE_U32, 2, |seg, _span| seg.op2(|a: u32, b: u32| a < b)),
    sig!(TYPE_U64, 2, |seg, _span| seg.op2(|a: u64, b: u64| a < b)),
    sig!(TYPE_U128, 2, |seg, _span| seg.op2(|a: u128, b: u128| a < b)),
    sig!(TYPE_USIZE, 2, |seg, _span| seg
        .op2(|a: usize, b: usize| a < b)),
    sig!(TYPE_I8, 2, |seg, _span| seg.op2(|a: i8, b: i8| a < b)),
    sig!(TYPE_I16, 2, |seg, _span| seg.op2(|a: i16, b: i16| a < b)),
    sig!(TYPE_I32, 2, |seg, _span| seg.op2(|a: i32, b: i32| a < b)),
    sig!(TYPE_I64, 2, |seg, _span| seg.op2(|a: i64, b: i64| a < b)),
    sig!(TYPE_I128, 2, |seg, _span| seg.op2(|a: i128, b: i128| a < b)),
    sig!(TYPE_ISIZE, 2, |seg, _span| seg
        .op2(|a: isize, b: isize| a < b)),
    sig!(TYPE_F32, 2, |seg, _span| seg.op2(|a: f32, b: f32| a < b)),
    sig!(TYPE_F64, 2, |seg, _span| seg.op2(|a: f64, b: f64| a < b)),
    sig!(TYPE_STR, 2, |seg, _span| seg
        .op2(|a: String, b: String| a < b)),
];

// Less than or equal signatures
static LESS_THAN_OR_EQUAL_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, _span| seg.op2(|a: u8, b: u8| a <= b)),
    sig!(TYPE_U16, 2, |seg, _span| seg.op2(|a: u16, b: u16| a <= b)),
    sig!(TYPE_U32, 2, |seg, _span| seg.op2(|a: u32, b: u32| a <= b)),
    sig!(TYPE_U64, 2, |seg, _span| seg.op2(|a: u64, b: u64| a <= b)),
    sig!(TYPE_U128, 2, |seg, _span| seg
        .op2(|a: u128, b: u128| a <= b)),
    sig!(TYPE_USIZE, 2, |seg, _span| seg
        .op2(|a: usize, b: usize| a <= b)),
    sig!(TYPE_I8, 2, |seg, _span| seg.op2(|a: i8, b: i8| a <= b)),
    sig!(TYPE_I16, 2, |seg, _span| seg.op2(|a: i16, b: i16| a <= b)),
    sig!(TYPE_I32, 2, |seg, _span| seg.op2(|a: i32, b: i32| a <= b)),
    sig!(TYPE_I64, 2, |seg, _span| seg.op2(|a: i64, b: i64| a <= b)),
    sig!(TYPE_I128, 2, |seg, _span| seg
        .op2(|a: i128, b: i128| a <= b)),
    sig!(TYPE_ISIZE, 2, |seg, _span| seg
        .op2(|a: isize, b: isize| a <= b)),
    sig!(TYPE_F32, 2, |seg, _span| seg.op2(|a: f32, b: f32| a <= b)),
    sig!(TYPE_F64, 2, |seg, _span| seg.op2(|a: f64, b: f64| a <= b)),
    sig!(TYPE_STR, 2, |seg, _span| seg
        .op2(|a: String, b: String| a <= b)),
];

// Greater than signatures
static GREATER_THAN_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, _span| seg.op2(|a: u8, b: u8| a > b)),
    sig!(TYPE_U16, 2, |seg, _span| seg.op2(|a: u16, b: u16| a > b)),
    sig!(TYPE_U32, 2, |seg, _span| seg.op2(|a: u32, b: u32| a > b)),
    sig!(TYPE_U64, 2, |seg, _span| seg.op2(|a: u64, b: u64| a > b)),
    sig!(TYPE_U128, 2, |seg, _span| seg.op2(|a: u128, b: u128| a > b)),
    sig!(TYPE_USIZE, 2, |seg, _span| seg
        .op2(|a: usize, b: usize| a > b)),
    sig!(TYPE_I8, 2, |seg, _span| seg.op2(|a: i8, b: i8| a > b)),
    sig!(TYPE_I16, 2, |seg, _span| seg.op2(|a: i16, b: i16| a > b)),
    sig!(TYPE_I32, 2, |seg, _span| seg.op2(|a: i32, b: i32| a > b)),
    sig!(TYPE_I64, 2, |seg, _span| seg.op2(|a: i64, b: i64| a > b)),
    sig!(TYPE_I128, 2, |seg, _span| seg.op2(|a: i128, b: i128| a > b)),
    sig!(TYPE_ISIZE, 2, |seg, _span| seg
        .op2(|a: isize, b: isize| a > b)),
    sig!(TYPE_F32, 2, |seg, _span| seg.op2(|a: f32, b: f32| a > b)),
    sig!(TYPE_F64, 2, |seg, _span| seg.op2(|a: f64, b: f64| a > b)),
    sig!(TYPE_STR, 2, |seg, _span| seg
        .op2(|a: String, b: String| a > b)),
];

// Greater than or equal signatures
static GREATER_THAN_OR_EQUAL_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, _span| seg.op2(|a: u8, b: u8| a >= b)),
    sig!(TYPE_U16, 2, |seg, _span| seg.op2(|a: u16, b: u16| a >= b)),
    sig!(TYPE_U32, 2, |seg, _span| seg.op2(|a: u32, b: u32| a >= b)),
    sig!(TYPE_U64, 2, |seg, _span| seg.op2(|a: u64, b: u64| a >= b)),
    sig!(TYPE_U128, 2, |seg, _span| seg
        .op2(|a: u128, b: u128| a >= b)),
    sig!(TYPE_USIZE, 2, |seg, _span| seg
        .op2(|a: usize, b: usize| a >= b)),
    sig!(TYPE_I8, 2, |seg, _span| seg.op2(|a: i8, b: i8| a >= b)),
    sig!(TYPE_I16, 2, |seg, _span| seg.op2(|a: i16, b: i16| a >= b)),
    sig!(TYPE_I32, 2, |seg, _span| seg.op2(|a: i32, b: i32| a >= b)),
    sig!(TYPE_I64, 2, |seg, _span| seg.op2(|a: i64, b: i64| a >= b)),
    sig!(TYPE_I128, 2, |seg, _span| seg
        .op2(|a: i128, b: i128| a >= b)),
    sig!(TYPE_ISIZE, 2, |seg, _span| seg
        .op2(|a: isize, b: isize| a >= b)),
    sig!(TYPE_F32, 2, |seg, _span| seg.op2(|a: f32, b: f32| a >= b)),
    sig!(TYPE_F64, 2, |seg, _span| seg.op2(|a: f64, b: f64| a >= b)),
    sig!(TYPE_STR, 2, |seg, _span| seg
        .op2(|a: String, b: String| a >= b)),
];

/// Compile-time perfect hash map for built-in operations.
///
/// Maps operator symbols to their signature arrays for O(1) lookup.
static BUILTINS: phf::Map<&'static str, &'static [OpSignature]> = phf_map! {
    "+" => ADD_SIGNATURES,
    "-" => SUB_SIGNATURES,
    "*" => MUL_SIGNATURES,
    "/" => DIV_SIGNATURES,
    "%" => MOD_SIGNATURES,
    "&" => BITWISE_AND_SIGNATURES,
    "|" => BITWISE_OR_SIGNATURES,
    "^" => BITWISE_XOR_SIGNATURES,
    "!" => LOGICAL_NOT_SIGNATURES,
    "==" => EQUAL_SIGNATURES,
    "!=" => NOT_EQUAL_SIGNATURES,
    "<" => LESS_THAN_SIGNATURES,
    "<=" => LESS_THAN_OR_EQUAL_SIGNATURES,
    ">" => GREATER_THAN_SIGNATURES,
    ">=" => GREATER_THAN_OR_EQUAL_SIGNATURES,
};

/// Built-in operation scope.
///
/// Provides lookup for standard operations using a compile-time hash table.
struct BuiltinScope;

impl BuiltinScope {
    /// Attempts to find and apply a built-in operation.
    ///
    /// Returns `Ok(true)` if found and applied, `Ok(false)` if not found.
    ///
    /// - Complexity: O(s) where s is the number of signatures registered for `name`.
    fn lookup(
        &self,
        name: &str,
        segment: &mut DynSegment,
        num_operands: usize,
        span: SourceSpan,
    ) -> Result<bool> {
        let stack_infos = segment.peek_stack_infos(num_operands);
        let signatures: &[OpSignature] = match name {
            "<<" => &LEFT_SHIFT_SIGNATURES,
            ">>" => &RIGHT_SHIFT_SIGNATURES,
            _ => match BUILTINS.get(name) {
                Some(s) => s,
                None => return Ok(false),
            },
        };
        for sig in signatures {
            let arity = sig.arity as usize;
            let matches = arity == stack_infos.len()
                && stack_infos[0].type_id == sig.lhs_type_id()
                && (arity < 2 || stack_infos[1].type_id == sig.rhs_type_id());
            if matches {
                (sig.op_fn)(segment, span)?;
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Operation lookup with scope stack support.
///
/// Provides a stack of scopes for operation resolution, with built-in operations
/// as the fallback. Scopes are searched in LIFO order (most recently pushed first).
///
/// # Examples
///
/// ```rust
/// use cel_parser::op_table::OpLookup;
/// use cel_runtime::DynSegment;
/// use std::any::TypeId;
///
/// let mut lookup = OpLookup::new();
///
/// // Use built-in addition
/// let mut segment = DynSegment::new::<()>();
/// segment.just(10u32);
/// segment.just(20u32);
/// lookup.lookup("+", &mut segment, 2, proc_macro2::Span::call_site(), proc_macro2::Span::call_site()).unwrap();
/// assert_eq!(segment.call0::<u32>().unwrap(), 30);
/// ```
pub struct OpLookup {
    scopes: Vec<ScopeFn>,
    builtin_scope: BuiltinScope,
}

impl OpLookup {
    /// Creates a new operation lookup with only built-in operations.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::OpLookup;
    ///
    /// let lookup = OpLookup::new();
    /// ```
    pub fn new() -> Self {
        OpLookup {
            scopes: Vec::new(),
            builtin_scope: BuiltinScope,
        }
    }

    /// Pushes a new scope onto the stack.
    ///
    /// Accepts a closure directly; it is boxed internally. The scope should return
    /// `Ok(true)` if it handled the operation, `Ok(false)` to pass to the next scope,
    /// or `Err` on error. Error messages surface verbatim; they should be lowercase, end
    /// without a period, and wrap identifiers and type names in backticks.
    pub fn push_scope<F>(&mut self, scope: F)
    where
        F: Fn(&str, &mut DynSegment, usize, SourceSpan) -> Result<bool> + Send + Sync + 'static,
    {
        self.scopes.push(Box::new(scope));
    }

    /// Pops the most recent scope from the stack.
    ///
    /// Returns the popped scope, or `None` if the stack is empty.
    pub fn pop_scope(&mut self) -> Option<ScopeFn> {
        self.scopes.pop()
    }

    /// Looks up and applies an operation, attaching the expression span to any error.
    ///
    /// Searches scopes in LIFO order, then falls back to built-in operations.
    ///
    /// # Errors
    ///
    /// Returns a [`crate::ParseError`] spanning `start..=end` if no scope or built-in
    /// handles the request, or if a scope itself returns an error.
    ///
    /// - Complexity: O(k) in the number of registered scopes, plus O(s) for the built-in
    ///   signature scan where s is the number of signatures for the operator.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use proc_macro2::Span;
    /// use cel_parser::OpLookup;
    /// use cel_runtime::DynSegment;
    ///
    /// let lookup = OpLookup::new();
    /// let mut seg = DynSegment::new::<()>();
    /// // A lookup with zero operands for a known operator succeeds when types match.
    /// // This example shows the signature only; real usage requires pushed types.
    /// let result = lookup.lookup("+", &mut seg, 2, Span::call_site(), Span::call_site());
    /// // result is Err because no operands are on the segment
    /// assert!(result.is_err());
    /// ```
    pub fn lookup(
        &self,
        name: &str,
        segment: &mut DynSegment,
        num_operands: usize,
        start: proc_macro2::Span,
        end: proc_macro2::Span,
    ) -> std::result::Result<(), crate::ParseError> {
        let source_span = SourceSpan::from_proc_macro2_range(start, end);
        for scope in self.scopes.iter().rev() {
            match scope(name, segment, num_operands, source_span) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(e) => return Err(crate::ParseError::new_range(e.to_string(), start, end)),
            }
        }

        match self
            .builtin_scope
            .lookup(name, segment, num_operands, source_span)
        {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(e) => {
                return Err(crate::ParseError::new_range(
                    format!("operation error: {}", e),
                    start,
                    end,
                ));
            }
        }

        if num_operands == 0 {
            return Err(crate::ParseError::new(
                format!("undefined identifier: `{name}`"),
                start,
            ));
        }
        let infos = segment.peek_stack_infos(num_operands);
        let mut type_names = String::new();
        for (i, info) in infos.iter().enumerate() {
            if i > 0 {
                type_names.push_str(", ");
            }
            type_names.push('`');
            type_names.push_str(info.type_name.as_ref());
            type_names.push('`');
        }
        Err(crate::ParseError::new_range(
            format!("no operation `{name}` for types [{type_names}]"),
            start,
            end,
        ))
    }
}

impl Default for OpLookup {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::Span;

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

    #[test]
    fn test_addition_u32() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(10u32);
        segment.just(20u32);
        lookup.lookup("+", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<u32>()?, 30);
        Ok(())
    }

    #[test]
    fn test_subtraction_i32() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(50i32);
        segment.just(20i32);
        lookup.lookup("-", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<i32>()?, 30);
        Ok(())
    }

    #[test]
    fn test_arithmetic_overflow() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(i32::MAX);
        segment.just(1i32);
        lookup.lookup("+", &mut segment, 2, Span::call_site(), Span::call_site())?;
        let result = segment.call0::<i32>();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("arithmetic overflow"),
            "error message should mention arithmetic overflow"
        );
        Ok(())
    }

    #[test]
    fn test_division_by_zero() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(10i32);
        segment.just(0i32);
        lookup.lookup("/", &mut segment, 2, Span::call_site(), Span::call_site())?;
        let result = segment.call0::<i32>();
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("division by zero"),
            "error message should mention division by zero"
        );
        Ok(())
    }

    #[test]
    fn test_modulo_by_zero() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(10u32);
        segment.just(0u32);
        lookup.lookup("%", &mut segment, 2, Span::call_site(), Span::call_site())?;
        let result = segment.call0::<u32>();
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("division by zero"),
            "error message should mention division by zero"
        );
        Ok(())
    }

    #[test]
    fn test_multiplication_f64() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(3.5f64);
        segment.just(2.0f64);
        lookup.lookup("*", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<f64>()?, 7.0);
        Ok(())
    }

    #[test]
    fn test_comparison_less_than() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(10u32);
        segment.just(20u32);
        lookup.lookup("<", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<bool>()?, true);
        Ok(())
    }

    #[test]
    fn test_bitwise_and() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(0b1010u32);
        segment.just(0b1100u32);
        lookup.lookup("&", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<u32>()?, 0b1000);
        Ok(())
    }

    #[test]
    fn test_unary_negation() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(42i32);
        lookup.lookup("-", &mut segment, 1, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<i32>()?, -42);
        Ok(())
    }

    #[test]
    fn test_logical_not() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(true);
        lookup.lookup("!", &mut segment, 1, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<bool>()?, false);
        Ok(())
    }

    #[test]
    fn test_unregistered_operation() {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(10u32);
        segment.just(20u32);
        let result = lookup.lookup(
            "unknown_op",
            &mut segment,
            2,
            Span::call_site(),
            Span::call_site(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_custom_scope() -> Result<()> {
        let mut lookup = OpLookup::new();

        lookup.push_scope(|name, segment, num_operands, _span| {
            let matches = {
                let top = segment.peek_stack_infos(num_operands);
                name == "double" && top.len() == 1 && top[0].type_id == TypeId::of::<u32>()
            };
            if matches {
                segment.op1(|a: u32| a * 2)?;
                Ok(true)
            } else {
                Ok(false)
            }
        });

        let mut segment = DynSegment::new::<()>();
        segment.just(21u32);
        lookup.lookup(
            "double",
            &mut segment,
            1,
            Span::call_site(),
            Span::call_site(),
        )?;
        assert_eq!(segment.call0::<u32>()?, 42);

        Ok(())
    }

    #[test]
    fn test_scope_override() -> Result<()> {
        let mut lookup = OpLookup::new();

        lookup.push_scope(|name, segment, num_operands, _span| {
            let matches = {
                let top = segment.peek_stack_infos(num_operands);
                name == "+" && top.len() == 2 && top[0].type_id == TypeId::of::<u32>()
            };
            if matches {
                segment.op2(|_a: u32, _b: u32| 100u32)?;
                Ok(true)
            } else {
                Ok(false)
            }
        });

        let mut segment = DynSegment::new::<()>();
        segment.just(10u32);
        segment.just(20u32);
        lookup.lookup("+", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<u32>()?, 100);

        Ok(())
    }

    #[test]
    fn test_left_shift_u64() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(1u64);
        segment.just(3u32);
        lookup.lookup("<<", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<u64>()?, 8);
        Ok(())
    }

    #[test]
    fn test_right_shift_i32() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(16i32);
        segment.just(2u32);
        lookup.lookup(">>", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<i32>()?, 4);
        Ok(())
    }

    #[test]
    fn test_shift_overflow() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(1u32);
        segment.just(32u32);
        lookup.lookup("<<", &mut segment, 2, Span::call_site(), Span::call_site())?;
        let result = segment.call0::<u32>();
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("shift overflow"),
            "error message should mention shift overflow"
        );
        Ok(())
    }

    #[test]
    fn test_shift_i32_rhs() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(1u32);
        segment.just(3i32);
        lookup.lookup("<<", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<u32>()?, 8);
        Ok(())
    }

    #[test]
    fn test_shift_negative_rhs_errors() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(1u32);
        segment.just(-1i32);
        lookup.lookup("<<", &mut segment, 2, Span::call_site(), Span::call_site())?;
        let result = segment.call0::<u32>();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("shift overflow"));
        Ok(())
    }

    #[test]
    fn test_shift_wide_rhs_overflow_errors() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(1u32);
        segment.just(u32::MAX as u64 + 1);
        lookup.lookup("<<", &mut segment, 2, Span::call_site(), Span::call_site())?;
        let result = segment.call0::<u32>();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("shift overflow"));
        Ok(())
    }

    #[test]
    fn test_shift_rejects_float_rhs() {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(1u32);
        segment.just(3.0f64);
        assert!(
            lookup
                .lookup("<<", &mut segment, 2, Span::call_site(), Span::call_site())
                .is_err()
        );
    }

    #[test]
    fn test_scope_pop() -> Result<()> {
        let mut lookup = OpLookup::new();

        lookup.push_scope(|name, segment, num_operands, _span| {
            let matches = {
                let top = segment.peek_stack_infos(num_operands);
                name == "+" && top.len() == 2 && top[0].type_id == TypeId::of::<u32>()
            };
            if matches {
                segment.op2(|_a: u32, _b: u32| 100u32)?;
                Ok(true)
            } else {
                Ok(false)
            }
        });

        let mut segment = DynSegment::new::<()>();
        segment.just(10u32);
        segment.just(20u32);
        lookup.lookup("+", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<u32>()?, 100);

        lookup.pop_scope();
        let mut segment = DynSegment::new::<()>();
        segment.just(10u32);
        segment.just(20u32);
        lookup.lookup("+", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<u32>()?, 30);

        Ok(())
    }

    #[test]
    fn lookup_not_found_error_carries_span() {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(10u32);
        segment.just(20.0f64);
        let err = lookup
            .lookup("+", &mut segment, 2, Span::call_site(), Span::call_site())
            .unwrap_err();
        assert!(
            err.message().starts_with("no operation"),
            "expected 'no operation' prefix, got: {}",
            err.message()
        );
        assert!(err.message().contains("`+`"));
        assert!(err.message().contains("`u32`"));
        assert!(err.message().contains("`f64`"));
    }

    #[test]
    fn lookup_not_found_error_has_range() {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(10u32);
        segment.just(20.0f64);
        let err = lookup
            .lookup("+", &mut segment, 2, Span::call_site(), Span::call_site())
            .unwrap_err();
        assert!(
            err.end_span().is_some(),
            "op-lookup errors should carry an end span"
        );
    }
}
