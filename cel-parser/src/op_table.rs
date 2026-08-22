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
use cel_runtime::{DynSegment, DynTuple};
use once_cell::sync::Lazy;
use phf::phf_map;
use std::any::TypeId;

use crate::SourceSpan;

/// Wraps a runtime error with span context when the `span-diagnostics` feature is enabled.
///
/// When the feature is off this is a no-op and compiles to nothing.
#[cfg(feature = "span-diagnostics")]
#[inline]
fn span_err(span: SourceSpan, e: anyhow::Error) -> anyhow::Error {
    e.context(crate::SpanContext::new(span))
}

#[cfg(not(feature = "span-diagnostics"))]
#[inline]
fn span_err(_span: SourceSpan, e: anyhow::Error) -> anyhow::Error {
    e
}

/// A function that pushes an operation onto a DynSegment.
///
/// Receives the segment and the source span of the expression that triggered
/// this operation. This is a simple function pointer since built-in operations
/// have no state.
pub type OpFn = fn(&mut DynSegment, SourceSpan) -> Result<()>;

/// A signature for an operator/function whose selected operand is a tuple.
///
/// Matches when the operand at `tuple_operand_index` (0-based, in the same
/// stack order [`DynSegment::peek_stack_infos`] returns) is a tuple whose
/// element `TypeId`s equal `shape`, in order, and every other peeked operand's
/// flat `TypeId` equals the corresponding entry in `operand_type_ids` (the
/// entry at `tuple_operand_index` in `operand_type_ids` is never read).
///
/// `operand_type_ids` must have an entry for every non-tuple operand
/// position: a missing entry (out of bounds, including an entirely empty
/// vector when there are non-tuple operands) simply never matches, rather
/// than panicking — it is only safe to omit the whole vector when
/// `tuple_operand_index` is the *only* operand position.
///
/// `shape` is flat: an element position that is itself a nested tuple can
/// only be recorded as `DynTuple`'s `TypeId`, which matches *any* nested
/// tuple at that position regardless of its inner arity or element types.
/// Two registrations that would only differ by that inner shape are not
/// distinguishable — do not rely on nested-tuple precision at this level.
pub struct TupleOpSignature {
    /// Operator/function name this signature is registered under.
    pub name: String,
    /// Expected element `TypeId`s, in order, for the tuple-shaped operand.
    /// See the struct-level note on nested tuples.
    pub shape: Vec<TypeId>,
    /// Which peeked operand position must be the tuple.
    pub tuple_operand_index: usize,
    /// Flat `TypeId`s expected for the non-tuple operands, in stack order
    /// (the `tuple_operand_index` entry is ignored).
    pub operand_type_ids: Vec<TypeId>,
    /// Function that pushes the operation onto the segment.
    pub op_fn: OpFn,
}

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
    sig!(TYPE_I8, 2, |seg, span| seg.op2r(move |a: i8, b: i8| a
        .checked_add(b)
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I16, 2, |seg, span| seg.op2r(move |a: i16, b: i16| a
        .checked_add(b)
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I32, 2, |seg, span| seg.op2r(move |a: i32, b: i32| a
        .checked_add(b)
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I64, 2, |seg, span| seg.op2r(move |a: i64, b: i64| a
        .checked_add(b)
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I128, 2, |seg, span| seg.op2r(
        move |a: i128, b: i128| a
            .checked_add(b)
            .ok_or_else(|| anyhow!("arithmetic overflow"))
            .map_err(|e| span_err(span, e))
    )),
    sig!(TYPE_ISIZE, 2, |seg, span| seg.op2r(
        move |a: isize, b: isize| a
            .checked_add(b)
            .ok_or_else(|| anyhow!("arithmetic overflow"))
            .map_err(|e| span_err(span, e))
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
    sig!(TYPE_I8, 2, |seg, span| seg.op2r(move |a: i8, b: i8| a
        .checked_sub(b)
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I16, 2, |seg, span| seg.op2r(move |a: i16, b: i16| a
        .checked_sub(b)
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I32, 2, |seg, span| seg.op2r(move |a: i32, b: i32| a
        .checked_sub(b)
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I64, 2, |seg, span| seg.op2r(move |a: i64, b: i64| a
        .checked_sub(b)
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I128, 2, |seg, span| seg.op2r(
        move |a: i128, b: i128| a
            .checked_sub(b)
            .ok_or_else(|| anyhow!("arithmetic overflow"))
            .map_err(|e| span_err(span, e))
    )),
    sig!(TYPE_ISIZE, 2, |seg, span| seg.op2r(
        move |a: isize, b: isize| a
            .checked_sub(b)
            .ok_or_else(|| anyhow!("arithmetic overflow"))
            .map_err(|e| span_err(span, e))
    )),
    sig!(TYPE_F32, 2, |seg, _span| seg.op2(|a: f32, b: f32| a - b)),
    sig!(TYPE_F64, 2, |seg, _span| seg.op2(|a: f64, b: f64| a - b)),
    // Unary negation
    sig!(TYPE_I8, 1, |seg, span| seg.op1r(move |a: i8| a
        .checked_neg()
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I16, 1, |seg, span| seg.op1r(move |a: i16| a
        .checked_neg()
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I32, 1, |seg, span| seg.op1r(move |a: i32| a
        .checked_neg()
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I64, 1, |seg, span| seg.op1r(move |a: i64| a
        .checked_neg()
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I128, 1, |seg, span| seg.op1r(move |a: i128| a
        .checked_neg()
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_ISIZE, 1, |seg, span| seg.op1r(move |a: isize| a
        .checked_neg()
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
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
    sig!(TYPE_I8, 2, |seg, span| seg.op2r(move |a: i8, b: i8| a
        .checked_mul(b)
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I16, 2, |seg, span| seg.op2r(move |a: i16, b: i16| a
        .checked_mul(b)
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I32, 2, |seg, span| seg.op2r(move |a: i32, b: i32| a
        .checked_mul(b)
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I64, 2, |seg, span| seg.op2r(move |a: i64, b: i64| a
        .checked_mul(b)
        .ok_or_else(|| anyhow!("arithmetic overflow"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I128, 2, |seg, span| seg.op2r(
        move |a: i128, b: i128| a
            .checked_mul(b)
            .ok_or_else(|| anyhow!("arithmetic overflow"))
            .map_err(|e| span_err(span, e))
    )),
    sig!(TYPE_ISIZE, 2, |seg, span| seg.op2r(
        move |a: isize, b: isize| a
            .checked_mul(b)
            .ok_or_else(|| anyhow!("arithmetic overflow"))
            .map_err(|e| span_err(span, e))
    )),
    sig!(TYPE_F32, 2, |seg, _span| seg.op2(|a: f32, b: f32| a * b)),
    sig!(TYPE_F64, 2, |seg, _span| seg.op2(|a: f64, b: f64| a * b)),
];

// Division signatures
//
// Integer division uses `checked_div` via `op2r` so that division by zero returns an error
// instead of panicking. Float division keeps `op2` (IEEE 754 defines x/0.0 as inf/nan).
static DIV_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, span| seg.op2r(move |a: u8, b: u8| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_U16, 2, |seg, span| seg.op2r(move |a: u16, b: u16| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_U32, 2, |seg, span| seg.op2r(move |a: u32, b: u32| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_U64, 2, |seg, span| seg.op2r(move |a: u64, b: u64| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_U128, 2, |seg, span| seg.op2r(
        move |a: u128, b: u128| a
            .checked_div(b)
            .ok_or_else(|| anyhow!("division by zero"))
            .map_err(|e| span_err(span, e))
    )),
    sig!(TYPE_USIZE, 2, |seg, span| seg.op2r(
        move |a: usize, b: usize| a
            .checked_div(b)
            .ok_or_else(|| anyhow!("division by zero"))
            .map_err(|e| span_err(span, e))
    )),
    sig!(TYPE_I8, 2, |seg, span| seg.op2r(move |a: i8, b: i8| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I16, 2, |seg, span| seg.op2r(move |a: i16, b: i16| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I32, 2, |seg, span| seg.op2r(move |a: i32, b: i32| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I64, 2, |seg, span| seg.op2r(move |a: i64, b: i64| a
        .checked_div(b)
        .ok_or_else(|| anyhow!("division by zero"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I128, 2, |seg, span| seg.op2r(
        move |a: i128, b: i128| a
            .checked_div(b)
            .ok_or_else(|| anyhow!("division by zero"))
            .map_err(|e| span_err(span, e))
    )),
    sig!(TYPE_ISIZE, 2, |seg, span| seg.op2r(
        move |a: isize, b: isize| a
            .checked_div(b)
            .ok_or_else(|| anyhow!("division by zero"))
            .map_err(|e| span_err(span, e))
    )),
    sig!(TYPE_F32, 2, |seg, _span| seg.op2(|a: f32, b: f32| a / b)),
    sig!(TYPE_F64, 2, |seg, _span| seg.op2(|a: f64, b: f64| a / b)),
];

// Modulo signatures
//
// Integer modulo uses `checked_rem` via `op2r` so that division by zero returns an error
// instead of panicking. Float modulo keeps `op2` (x % 0.0 yields NaN without panicking).
static MOD_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, span| seg.op2r(move |a: u8, b: u8| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_U16, 2, |seg, span| seg.op2r(move |a: u16, b: u16| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_U32, 2, |seg, span| seg.op2r(move |a: u32, b: u32| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_U64, 2, |seg, span| seg.op2r(move |a: u64, b: u64| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_U128, 2, |seg, span| seg.op2r(
        move |a: u128, b: u128| a
            .checked_rem(b)
            .ok_or_else(|| anyhow!("division by zero"))
            .map_err(|e| span_err(span, e))
    )),
    sig!(TYPE_USIZE, 2, |seg, span| seg.op2r(
        move |a: usize, b: usize| a
            .checked_rem(b)
            .ok_or_else(|| anyhow!("division by zero"))
            .map_err(|e| span_err(span, e))
    )),
    sig!(TYPE_I8, 2, |seg, span| seg.op2r(move |a: i8, b: i8| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I16, 2, |seg, span| seg.op2r(move |a: i16, b: i16| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I32, 2, |seg, span| seg.op2r(move |a: i32, b: i32| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I64, 2, |seg, span| seg.op2r(move |a: i64, b: i64| a
        .checked_rem(b)
        .ok_or_else(|| anyhow!("division by zero"))
        .map_err(|e| span_err(span, e)))),
    sig!(TYPE_I128, 2, |seg, span| seg.op2r(
        move |a: i128, b: i128| a
            .checked_rem(b)
            .ok_or_else(|| anyhow!("division by zero"))
            .map_err(|e| span_err(span, e))
    )),
    sig!(TYPE_ISIZE, 2, |seg, span| seg.op2r(
        move |a: isize, b: isize| a
            .checked_rem(b)
            .ok_or_else(|| anyhow!("division by zero"))
            .map_err(|e| span_err(span, e))
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
        $v.push(sig_het!($lhs_idx, TYPE_U8, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: u8| a
                .checked_shl(u32::from(b))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_U16, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: u16| a
                .checked_shl(u32::from(b))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_U32, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: u32| a
                .checked_shl(b)
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_U64, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: u64| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_U128, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: u128| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_USIZE, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: usize| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I8, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: i8| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I16, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: i16| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I32, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: i32| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I64, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: i64| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I128, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: i128| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_ISIZE, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: isize| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shl(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
    };
}

macro_rules! shr_push {
    ($v:ident, $lhs_idx:expr, $lhs_ty:ty) => {
        $v.push(sig_het!($lhs_idx, TYPE_U8, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: u8| a
                .checked_shr(u32::from(b))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_U16, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: u16| a
                .checked_shr(u32::from(b))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_U32, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: u32| a
                .checked_shr(b)
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_U64, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: u64| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_U128, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: u128| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_USIZE, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: usize| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I8, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: i8| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I16, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: i16| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I32, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: i32| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I64, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: i64| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_I128, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: i128| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
        )));
        $v.push(sig_het!($lhs_idx, TYPE_ISIZE, |seg, span| seg.op2r(
            move |a: $lhs_ty, b: isize| u32::try_from(b)
                .ok()
                .and_then(|r| a.checked_shr(r))
                .ok_or_else(|| anyhow!("shift overflow"))
                .map_err(|e| span_err(span, e))
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

/// A single built-in overload's declared operand types, exposed for the static type checker
/// (`cel_parser::ty::check_expr`) — never `op_fn`, which is execution-only and stays private.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperandTypes {
    /// Number of operands this overload accepts (1 or 2).
    pub arity: u8,
    /// The LHS (or sole, for arity 1) operand's `TypeId`.
    pub lhs: TypeId,
    /// The RHS operand's `TypeId`; equal to `lhs` for a homogeneous or arity-1 overload.
    pub rhs: TypeId,
}

/// Returns every built-in overload's declared operand types for `name`, in registration order.
///
/// Reads the exact same static signature tables `BuiltinScope::lookup` dispatches against, so
/// the static type checker and the runtime dispatcher share one source of truth and can't drift
/// apart on which operand-type combinations a built-in operator accepts.
///
/// - Postcondition: returns an empty `Vec` if `name` names no built-in operator. A custom scope
///   registered via [`OpLookup::push_scope`] or a tuple-shaped op registered via
///   [`OpLookup::register_tuple_op`] is runtime-only (attached to one `OpLookup` instance, not
///   this module's static tables) and is not visible here — the type checker treats such an
///   operator as unchecked, not an error.
///
/// - Complexity: O(s) where s is the number of overloads registered for `name`.
///
/// # Examples
///
/// ```rust
/// use cel_parser::op_table::builtin_operand_types;
///
/// assert!(builtin_operand_types("+").iter().any(|sig| sig.arity == 2));
/// assert!(builtin_operand_types("not_an_operator").is_empty());
/// ```
pub fn builtin_operand_types(name: &str) -> Vec<OperandTypes> {
    let Some(signatures) = signatures_for(name) else {
        return Vec::new();
    };
    signatures
        .iter()
        .map(|sig| OperandTypes {
            arity: sig.arity,
            lhs: sig.lhs_type_id(),
            rhs: sig.rhs_type_id(),
        })
        .collect()
}

/// Routes an operator name to its static signature table, or `None` if `name` names no built-in
/// operator. Shared by [`builtin_operand_types`] and [`BuiltinScope::lookup`] so a future
/// heterogeneous operator only needs its routing added in one place.
fn signatures_for(name: &str) -> Option<&'static [OpSignature]> {
    match name {
        "<<" => Some(&LEFT_SHIFT_SIGNATURES),
        ">>" => Some(&RIGHT_SHIFT_SIGNATURES),
        _ => BUILTINS.get(name).copied(),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Casts (`expr as Type`)
// ─────────────────────────────────────────────────────────────────────────
//
// Covers every conversion Rust's own `as` operator supports among the
// built-in types this crate's `Ty` model recognizes: the 12 integer widths,
// `f32`/`f64`, `bool`, and `String`. A source/target pair not registered
// here is one Rust's `as` itself rejects at compile time - e.g. `x as bool`
// for a numeric `x` (E0054: "cannot cast ... as `bool`") or `x as f64` for
// a `bool` `x` (E0606: "casting `bool` as `f64` is invalid") - so `bool` and
// `String` are legitimate, fully-recognized cast *targets*, just ones with a
// narrow set of legal sources, not omitted types.
//
// Fallibility follows one rule, not Rust's `as` (which never fails - it
// saturates or truncates silently): a conversion that can always represent
// its source value exactly is infallible (`op1`); one that might not is
// checked and returns `Err` rather than silently losing information,
// matching this crate's established convention for arithmetic (see the
// module doc comment's "Semantics" section) and `round`'s own design.
//   - int -> int: checked via `TryFrom`, uniformly (including same-width and
//     identity pairs, which just always succeed).
//   - int -> float: infallible (`as`) - Rust's own behavior; precision loss
//     for very large integers is accepted, matching Rust exactly.
//   - float -> int: checked - `Err` for non-finite or out-of-range values;
//     a value with a fractional part is *truncated* toward zero, same as
//     Rust's `as` (checking is only about range/finiteness, not fractional
//     truncation policy - `round(x) as i32` is the idiom for "round to
//     nearest first").
//   - f32 -> f64: infallible (always exact). f64 -> f32: checked (may not
//     fit in `f32`'s finite range).
//   - bool -> int: infallible (`as`), matching Rust exactly (`true` -> `1`,
//     `false` -> `0`) for all 12 integer widths. Rust has no `bool -> float`
//     `as` cast (E0606), so none is registered here either.
//   - bool -> bool, String -> String: the only legal `as bool` / `as String`
//     sources are their own identity conversions (Rust allows `e as T`
//     whenever `e`'s type is already `T`, for any `T`, not just numerics);
//     infallible, a plain pass-through.

/// A signature for a numeric cast (`expr as Type`): one per legal source
/// type for a given target type.
#[derive(Clone, Copy)]
struct CastSignature {
    /// Index into TYPE_IDS for the source (operand) type.
    source_type_id_index: usize,
    /// Function pointer to the conversion implementation.
    op_fn: OpFn,
}

impl CastSignature {
    /// Returns the `TypeId` of the source (operand) type.
    fn source_type_id(&self) -> TypeId {
        TYPE_IDS[self.source_type_id_index]
    }
}

/// Pushes one checked int->int `CastSignature` (via `TryFrom`) onto `$v`.
macro_rules! try_from_cast_push {
    ($v:ident, $src_idx:expr, $src_ty:ty, $tgt_ty:ty) => {
        $v.push(CastSignature {
            source_type_id_index: $src_idx,
            op_fn: |seg, span| {
                seg.op1r(move |x: $src_ty| -> Result<$tgt_ty> {
                    <$tgt_ty>::try_from(x).map_err(|_| {
                        span_err(
                            span,
                            anyhow!(concat!("value does not fit in `", stringify!($tgt_ty), "`")),
                        )
                    })
                })
            },
        });
    };
}

/// Pushes all 12 checked int->int `CastSignature`s targeting `$tgt_ty` onto `$v`.
macro_rules! all_int_to_int_casts {
    ($v:ident, $tgt_ty:ty) => {
        try_from_cast_push!($v, TYPE_U8, u8, $tgt_ty);
        try_from_cast_push!($v, TYPE_U16, u16, $tgt_ty);
        try_from_cast_push!($v, TYPE_U32, u32, $tgt_ty);
        try_from_cast_push!($v, TYPE_U64, u64, $tgt_ty);
        try_from_cast_push!($v, TYPE_U128, u128, $tgt_ty);
        try_from_cast_push!($v, TYPE_USIZE, usize, $tgt_ty);
        try_from_cast_push!($v, TYPE_I8, i8, $tgt_ty);
        try_from_cast_push!($v, TYPE_I16, i16, $tgt_ty);
        try_from_cast_push!($v, TYPE_I32, i32, $tgt_ty);
        try_from_cast_push!($v, TYPE_I64, i64, $tgt_ty);
        try_from_cast_push!($v, TYPE_I128, i128, $tgt_ty);
        try_from_cast_push!($v, TYPE_ISIZE, isize, $tgt_ty);
    };
}

/// Pushes one infallible int->float `CastSignature` (via `as`) onto `$v`.
macro_rules! as_cast_push {
    ($v:ident, $src_idx:expr, $src_ty:ty, $tgt_ty:ty) => {
        $v.push(CastSignature {
            source_type_id_index: $src_idx,
            op_fn: |seg, _span| seg.op1(|x: $src_ty| x as $tgt_ty),
        });
    };
}

/// Pushes all 12 infallible int->float `CastSignature`s targeting `$tgt_ty` onto `$v`.
macro_rules! all_int_to_float_casts {
    ($v:ident, $tgt_ty:ty) => {
        as_cast_push!($v, TYPE_U8, u8, $tgt_ty);
        as_cast_push!($v, TYPE_U16, u16, $tgt_ty);
        as_cast_push!($v, TYPE_U32, u32, $tgt_ty);
        as_cast_push!($v, TYPE_U64, u64, $tgt_ty);
        as_cast_push!($v, TYPE_U128, u128, $tgt_ty);
        as_cast_push!($v, TYPE_USIZE, usize, $tgt_ty);
        as_cast_push!($v, TYPE_I8, i8, $tgt_ty);
        as_cast_push!($v, TYPE_I16, i16, $tgt_ty);
        as_cast_push!($v, TYPE_I32, i32, $tgt_ty);
        as_cast_push!($v, TYPE_I64, i64, $tgt_ty);
        as_cast_push!($v, TYPE_I128, i128, $tgt_ty);
        as_cast_push!($v, TYPE_ISIZE, isize, $tgt_ty);
    };
}

/// Pushes one checked float->int `CastSignature` onto `$v`: `Err` for a
/// non-finite or out-of-range value, otherwise truncates toward zero (same
/// as Rust's `as`).
///
/// The upper bound is deliberately not `<$tgt_ty>::MAX as $src_ty`: `MAX` (`0b0111...1`) is one
/// less than the power of two `MAX + 1`, so whenever `$tgt_ty` has more value bits than
/// `$src_ty`'s mantissa (e.g. `i32::MAX as f32`), the conversion has nowhere to round to but
/// *up*, landing exactly on `MAX + 1` - a value that itself is genuinely out of range but would
/// then compare equal to (not greater than) the bound, silently passing the check and letting
/// Rust's `as` saturate it to `MAX` instead of returning `Err`. Deriving the exclusive upper
/// bound directly as a power of two (`2^bits` unsigned, `2^(bits-1)` signed) sidesteps the
/// rounding entirely: whenever that power of two is within `$src_ty`'s finite range, it's exact
/// (no mantissa bits are needed to represent a power of two, only the exponent). The one case
/// where it isn't - `u128` (`2^128`) as an `f32` source, since `2^128` exceeds `f32::MAX` - falls
/// back to `f32::INFINITY` as the bound, which is still correct: every finite `f32` value is
/// already below `u128::MAX`, so a bound of infinity never wrongly accepts one.
macro_rules! float_to_int_cast_push {
    ($v:ident, $src_idx:expr, $src_ty:ty, $tgt_ty:ty) => {
        $v.push(CastSignature {
            source_type_id_index: $src_idx,
            op_fn: |seg, span| {
                seg.op1r(move |x: $src_ty| -> Result<$tgt_ty> {
                    if !x.is_finite() {
                        return Err(span_err(span, anyhow!("value is not finite")));
                    }
                    let is_unsigned = <$tgt_ty>::MIN == 0;
                    let bits = <$tgt_ty>::BITS as i32;
                    let exponent = if is_unsigned { bits } else { bits - 1 };
                    let upper_exclusive: $src_ty = (2.0 as $src_ty).powi(exponent);
                    if x < <$tgt_ty>::MIN as $src_ty || x >= upper_exclusive {
                        return Err(span_err(
                            span,
                            anyhow!(concat!("value does not fit in `", stringify!($tgt_ty), "`")),
                        ));
                    }
                    Ok(x as $tgt_ty)
                })
            },
        });
    };
}

/// Pushes both checked float->int `CastSignature`s (`f32`, `f64`) targeting `$tgt_ty` onto `$v`.
macro_rules! all_float_to_int_casts {
    ($v:ident, $tgt_ty:ty) => {
        float_to_int_cast_push!($v, TYPE_F32, f32, $tgt_ty);
        float_to_int_cast_push!($v, TYPE_F64, f64, $tgt_ty);
    };
}

macro_rules! int_cast_sources {
    ($name:ident, $tgt_ty:ty) => {
        static $name: Lazy<Vec<CastSignature>> = Lazy::new(|| {
            let mut v = Vec::with_capacity(15);
            all_int_to_int_casts!(v, $tgt_ty);
            all_float_to_int_casts!(v, $tgt_ty);
            as_cast_push!(v, TYPE_BOOL, bool, $tgt_ty);
            v
        });
    };
}

int_cast_sources!(U8_CAST_SOURCES, u8);
int_cast_sources!(U16_CAST_SOURCES, u16);
int_cast_sources!(U32_CAST_SOURCES, u32);
int_cast_sources!(U64_CAST_SOURCES, u64);
int_cast_sources!(U128_CAST_SOURCES, u128);
int_cast_sources!(USIZE_CAST_SOURCES, usize);
int_cast_sources!(I8_CAST_SOURCES, i8);
int_cast_sources!(I16_CAST_SOURCES, i16);
int_cast_sources!(I32_CAST_SOURCES, i32);
int_cast_sources!(I64_CAST_SOURCES, i64);
int_cast_sources!(I128_CAST_SOURCES, i128);
int_cast_sources!(ISIZE_CAST_SOURCES, isize);

static F32_CAST_SOURCES: Lazy<Vec<CastSignature>> = Lazy::new(|| {
    let mut v = vec![
        // f64 -> f32: checked (may not fit in f32's finite range). Unlike the int-narrowing
        // checks below, comparing directly against `f32::MAX as f64` is safe here rather than
        // needing the power-of-two-derived bound: f64's 53-bit mantissa always represents
        // f32::MAX exactly (f32 needs at most 24 significant bits), so this conversion never
        // rounds and the bound is exact.
        CastSignature {
            source_type_id_index: TYPE_F64,
            op_fn: |seg, span| {
                seg.op1r(move |x: f64| -> Result<f32> {
                    if !x.is_finite() {
                        return Err(span_err(span, anyhow!("value is not finite")));
                    }
                    if x.abs() > f32::MAX as f64 {
                        return Err(span_err(span, anyhow!("value does not fit in `f32`")));
                    }
                    Ok(x as f32)
                })
            },
        },
        CastSignature {
            source_type_id_index: TYPE_F32,
            op_fn: |seg, _span| seg.op1(|x: f32| x),
        },
    ];
    all_int_to_float_casts!(v, f32);
    v
});

static F64_CAST_SOURCES: Lazy<Vec<CastSignature>> = Lazy::new(|| {
    let mut v = vec![
        // f32 -> f64: always exact.
        CastSignature {
            source_type_id_index: TYPE_F32,
            op_fn: |seg, _span| seg.op1(|x: f32| x as f64),
        },
        CastSignature {
            source_type_id_index: TYPE_F64,
            op_fn: |seg, _span| seg.op1(|x: f64| x),
        },
    ];
    all_int_to_float_casts!(v, f64);
    v
});

/// `bool`'s only legal `as` source is `bool` itself (Rust has no
/// int/float/String -> `bool` `as` cast - E0054): a no-op identity
/// conversion.
static BOOL_CAST_SOURCES: Lazy<Vec<CastSignature>> = Lazy::new(|| {
    vec![CastSignature {
        source_type_id_index: TYPE_BOOL,
        op_fn: |seg, _span| seg.op1(|x: bool| x),
    }]
});

/// `String`'s only legal `as` source is `String` itself (Rust's `as` has no
/// conversion at all to `String` from any other type): a no-op identity
/// conversion.
static STRING_CAST_SOURCES: Lazy<Vec<CastSignature>> = Lazy::new(|| {
    vec![CastSignature {
        source_type_id_index: TYPE_STR,
        op_fn: |seg, _span| seg.op1(|x: String| x),
    }]
});

/// Routes a cast target type name to its static signature table (one legal
/// source per entry), or `None` if `target_name` names no type this
/// language's `as` operator recognizes as a cast target at all. Shared by
/// [`cast_source_types`] (the type checker) and [`OpLookup::lookup_cast`]
/// (execution) so they can't drift.
fn signatures_for_cast(target_name: &str) -> Option<&'static [CastSignature]> {
    match target_name {
        "u8" => Some(&U8_CAST_SOURCES),
        "u16" => Some(&U16_CAST_SOURCES),
        "u32" => Some(&U32_CAST_SOURCES),
        "u64" => Some(&U64_CAST_SOURCES),
        "u128" => Some(&U128_CAST_SOURCES),
        "usize" => Some(&USIZE_CAST_SOURCES),
        "i8" => Some(&I8_CAST_SOURCES),
        "i16" => Some(&I16_CAST_SOURCES),
        "i32" => Some(&I32_CAST_SOURCES),
        "i64" => Some(&I64_CAST_SOURCES),
        "i128" => Some(&I128_CAST_SOURCES),
        "isize" => Some(&ISIZE_CAST_SOURCES),
        "f32" => Some(&F32_CAST_SOURCES),
        "f64" => Some(&F64_CAST_SOURCES),
        "bool" => Some(&BOOL_CAST_SOURCES),
        "String" => Some(&STRING_CAST_SOURCES),
        _ => None,
    }
}

/// Lists every source `TypeId` with a registered cast to `target_name`. Used by the type checker
/// (`ty.rs`) to validate a cast without needing to touch a [`DynSegment`] - reads the exact same
/// tables [`OpLookup::lookup_cast`] does, so the two can't drift out of sync.
///
/// - Postcondition: yields no items if `target_name` names no recognized cast-target type.
///
/// - Complexity: O(s) where s is the number of registered sources for `target_name`.
///
/// # Examples
///
/// ```rust
/// use cel_parser::op_table::cast_source_types;
///
/// assert!(cast_source_types("i32").count() > 0);
/// assert_eq!(cast_source_types("not_a_type").count(), 0);
/// ```
pub fn cast_source_types(target_name: &str) -> impl Iterator<Item = TypeId> {
    signatures_for_cast(target_name)
        .into_iter()
        .flat_map(|sigs| sigs.iter().map(CastSignature::source_type_id))
}

/// One built-in scalar type's identity plus everything needed to declare a `DynSegment`
/// argument or tuple leaf of that type without the caller knowing it as a static Rust generic.
///
/// Covers exactly the fixed set of scalar type names [`signatures_for_cast`] already recognizes
/// as `as`-cast targets — closures are the first feature needing to *declare* a value of a named
/// type (rather than convert an already-stack-resident one), so this is new, additive surface
/// area; it deliberately reuses that same closed name set rather than inventing a second one.
#[allow(dead_code)]
pub(crate) struct BuiltinScalarType {
    pub(crate) type_id: TypeId,
    pub(crate) type_name: &'static str,
    pub(crate) size: usize,
    pub(crate) align: usize,
    pub(crate) dropper: cel_runtime::RawDropper,
    pub(crate) push_arg: fn(&mut DynSegment, usize),
}

macro_rules! builtin_scalar {
    ($name:literal, $ty:ty) => {
        BuiltinScalarType {
            type_id: TypeId::of::<$ty>(),
            type_name: $name,
            size: std::mem::size_of::<$ty>(),
            align: std::mem::align_of::<$ty>(),
            dropper: cel_runtime::raw_dropper_for::<$ty>(),
            push_arg: |seg, idx| seg.push_arg::<$ty>(idx),
        }
    };
}

/// Resolves a closure parameter type annotation's bare identifier to its full built-in
/// descriptor, or `None` if `name` names no recognized scalar type.
///
/// - Complexity: O(1).
#[allow(dead_code)]
pub(crate) fn builtin_scalar_type(name: &str) -> Option<BuiltinScalarType> {
    Some(match name {
        "u8" => builtin_scalar!("u8", u8),
        "u16" => builtin_scalar!("u16", u16),
        "u32" => builtin_scalar!("u32", u32),
        "u64" => builtin_scalar!("u64", u64),
        "u128" => builtin_scalar!("u128", u128),
        "usize" => builtin_scalar!("usize", usize),
        "i8" => builtin_scalar!("i8", i8),
        "i16" => builtin_scalar!("i16", i16),
        "i32" => builtin_scalar!("i32", i32),
        "i64" => builtin_scalar!("i64", i64),
        "i128" => builtin_scalar!("i128", i128),
        "isize" => builtin_scalar!("isize", isize),
        "f32" => builtin_scalar!("f32", f32),
        "f64" => builtin_scalar!("f64", f64),
        "bool" => builtin_scalar!("bool", bool),
        "String" => builtin_scalar!("String", String),
        _ => return None,
    })
}

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
        let Some(signatures) = signatures_for(name) else {
            return Ok(false);
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

/// Marker pushed onto the stack for the `round` builtin's callee (see
/// [`round_scope`]) - carries no data, it only lets the paired `"()"` match
/// arm recognize "this call's callee is `round`" among any other
/// same-arity callable that might one day share the stack.
struct RoundFn;

/// Scope function implementing the `round(x: f64) -> f64` builtin: rounds
/// to the nearest integer, halfway values away from zero, matching
/// `f64::round` exactly - narrowing to an integer type is a separate,
/// explicit step (`round(x) as i32`) via the general cast operator (see
/// the "Casts" section above), not this function's job.
///
/// Registered by every [`OpLookup::new()`] (see there), so `round` reads
/// like any other builtin operator without a caller needing to set it up.
///
/// A function call parses as two independent lookups - an arity-0 lookup
/// for the callee name, then an arity-`N+1` lookup for `"()"` with the
/// callee and its arguments on the stack (see `cel-parser/src/lib.rs`'s
/// primary/postfix expression grammar) - so this one scope function
/// handles both halves: `("round", 0)` pushes the [`RoundFn`] marker,
/// and `("()", 2)` peeks the stack to confirm both that it actually has
/// two operands and that this specific call's callee is that marker
/// before consuming it, deferring to any other registered scope
/// (`Ok(false)`) otherwise.
fn round_scope(
    name: &str,
    segment: &mut DynSegment,
    num_operands: usize,
    _span: SourceSpan,
) -> Result<bool> {
    match (name, num_operands) {
        ("round", 0) => {
            segment.op0(|| RoundFn);
            Ok(true)
        }
        ("()", 2) => {
            let top = segment.peek_stack_infos(2);
            if top.len() != 2 || top[0].type_id != TypeId::of::<RoundFn>() {
                return Ok(false);
            }
            segment.op2(|_callee: RoundFn, x: f64| x.round())?;
            Ok(true)
        }
        _ => Ok(false),
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
    library_scope_count: usize,
    builtin_scope: BuiltinScope,
    tuple_signatures: Vec<TupleOpSignature>,
}

impl OpLookup {
    /// Creates a new operation lookup with only built-in operations - the
    /// infix/prefix operators, the cast operator (`as`), and the
    /// `round` function.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::OpLookup;
    ///
    /// let lookup = OpLookup::new();
    /// ```
    pub fn new() -> Self {
        let mut lookup = OpLookup {
            scopes: Vec::new(),
            library_scope_count: 0,
            builtin_scope: BuiltinScope,
            tuple_signatures: Vec::new(),
        };
        lookup.push_library_scope(round_scope);
        lookup
    }

    /// Registers a tuple-shaped operator signature, matched by element
    /// `TypeId` sequence the same way built-in operators are matched by flat
    /// `TypeId`.
    pub fn register_tuple_op(&mut self, signature: TupleOpSignature) {
        self.tuple_signatures.push(signature);
    }

    /// Attempts to find and apply a registered tuple-shaped signature.
    ///
    /// Returns `Ok(true)` if found and applied, `Ok(false)` if not found.
    ///
    /// - Complexity: O(s) where s is the number of registered tuple signatures.
    fn lookup_tuple_signature(
        &self,
        name: &str,
        segment: &mut DynSegment,
        num_operands: usize,
        span: SourceSpan,
    ) -> Result<bool> {
        let stack_infos = segment.peek_stack_infos(num_operands);
        for sig in &self.tuple_signatures {
            if sig.name != name || sig.tuple_operand_index >= stack_infos.len() {
                continue;
            }
            let tuple_info = &stack_infos[sig.tuple_operand_index];
            let shape_matches = tuple_info.type_id == TypeId::of::<DynTuple>()
                && tuple_info.associated.len() == sig.shape.len()
                && tuple_info
                    .associated
                    .iter()
                    .zip(&sig.shape)
                    .all(|(a, t)| a.type_id == *t);
            if !shape_matches {
                continue;
            }
            let others_match = stack_infos.iter().enumerate().all(|(i, info)| {
                i == sig.tuple_operand_index || sig.operand_type_ids.get(i) == Some(&info.type_id)
            });
            if others_match {
                (sig.op_fn)(segment, span)?;
                return Ok(true);
            }
        }
        Ok(false)
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
    ///
    /// - Precondition: must not remove a library scope registered via
    ///   [`push_library_scope`](Self::push_library_scope) — those are permanent setup-time
    ///   registrations that must survive across multiple parses.
    pub fn pop_scope(&mut self) -> Option<ScopeFn> {
        debug_assert!(
            self.scopes.len() > self.library_scope_count,
            "pop_scope must not remove a library scope — use isolate_scopes/restore_scopes semantics instead"
        );
        self.scopes.pop()
    }

    /// Registers a permanent, library-level scope that is reachable from every parse,
    /// including inside closure bodies.
    ///
    /// Used for built-in language features (like `round`) and statically-installed library
    /// functions (like `clamp` from a `cel-std`-style crate). These scopes are registered
    /// once at setup time and must always be available, even when [`isolate_scopes`](Self::isolate_scopes)
    /// is active — library scopes are *never* isolated.
    ///
    /// Do not use for scopes tied to a single parse's lifetime — use [`push_scope`](Self::push_scope)
    /// for those, which can be isolated during nested body compilation (closures).
    ///
    /// - Complexity: O(1).
    pub fn push_library_scope<F>(&mut self, scope: F)
    where
        F: Fn(&str, &mut DynSegment, usize, SourceSpan) -> Result<bool> + Send + Sync + 'static,
    {
        self.scopes.push(Box::new(scope));
        self.library_scope_count = self.scopes.len();
    }

    /// Temporarily removes every transient scope (those pushed via [`push_scope`](Self::push_scope)),
    /// returning them so a later [`restore_scopes`](Self::restore_scopes) call can put them back.
    /// Library scopes registered via [`push_library_scope`](Self::push_library_scope) are *never*
    /// isolated and remain reachable.
    ///
    /// Used when compiling an independent nested body (a closure literal) that must resolve names
    /// against only its own declared parameters and library functions (like `round`, `clamp`) —
    /// never whatever transient per-parse scopes happen to be active. This maintains the invariant
    /// that library functions are always available, including inside closures, while per-declaration
    /// scopes (which tie to a single outer parse's lifetime) are hidden from nested bodies.
    ///
    /// - Postcondition: library scopes remain reachable; transient scopes are inaccessible until
    ///   [`restore_scopes`](Self::restore_scopes) is called.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::OpLookup;
    /// use cel_runtime::DynSegment;
    ///
    /// let mut lookup = OpLookup::new();
    /// lookup.push_scope(|name, segment, arity, _span| {
    ///     if name == "custom" && arity == 0 {
    ///         segment.just(42i32);
    ///         Ok(true)
    ///     } else {
    ///         Ok(false)
    ///     }
    /// });
    ///
    /// let saved = lookup.isolate_scopes();
    /// // Now the custom scope is inaccessible
    /// let mut segment = DynSegment::new::<()>();
    /// let result = lookup.lookup("custom", &mut segment, 0, proc_macro2::Span::call_site(), proc_macro2::Span::call_site());
    /// assert!(result.is_err());
    ///
    /// lookup.restore_scopes(saved);
    /// // Now the custom scope is reachable again
    /// let mut segment = DynSegment::new::<()>();
    /// let result = lookup.lookup("custom", &mut segment, 0, proc_macro2::Span::call_site(), proc_macro2::Span::call_site());
    /// assert!(result.is_ok());
    /// ```
    pub fn isolate_scopes(&mut self) -> Vec<ScopeFn> {
        self.scopes.split_off(self.library_scope_count)
    }

    /// Restores a scope stack previously removed by [`isolate_scopes`](Self::isolate_scopes),
    /// discarding whatever scopes were pushed while isolated.
    ///
    /// Library scopes (those registered via [`push_library_scope`](Self::push_library_scope))
    /// are unaffected by isolation and restoration — they persist across the entire operation.
    ///
    /// - Precondition: `scopes` came from a matching `isolate_scopes()` call on this same
    ///   `OpLookup` — restoring an arbitrary `Vec<ScopeFn>` is well-typed but not a meaningful use
    ///   of this method.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::OpLookup;
    /// use cel_runtime::DynSegment;
    ///
    /// let mut lookup = OpLookup::new();
    /// lookup.push_scope(|name, segment, arity, _span| {
    ///     if name == "outer" && arity == 0 {
    ///         segment.just(100i32);
    ///         Ok(true)
    ///     } else {
    ///         Ok(false)
    ///     }
    /// });
    ///
    /// let saved = lookup.isolate_scopes();
    /// lookup.restore_scopes(saved);
    /// // The outer scope is now reachable again
    /// ```
    pub fn restore_scopes(&mut self, scopes: Vec<ScopeFn>) {
        self.scopes.truncate(self.library_scope_count);
        self.scopes.extend(scopes);
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

        match self.lookup_tuple_signature(name, segment, num_operands, source_span) {
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

    /// Looks up and applies a cast (`expr as Type`), attaching the expression span to any error.
    ///
    /// - Postcondition: on success, an operation is queued onto `segment` that replaces its
    ///   top-of-stack operand with the converted value once the segment is run (e.g. via
    ///   [`DynSegment::call0`]) - like every other op this crate queues rather than executes
    ///   immediately.
    ///
    /// # Errors
    ///
    /// Returns a [`crate::ParseError`] spanning `start..=end` if `segment`'s stack is empty, if
    /// `type_name` isn't a recognized cast-target type, or if no cast from the operand's type to
    /// it is registered. A registered conversion's own value-range failure (e.g. an out-of-range
    /// or non-finite value) is deferred to execution - it surfaces from running the segment, not
    /// from this function (see `cast_errors_when_the_value_does_not_fit_in_the_target_type`
    /// below).
    ///
    /// - Complexity: O(s) in the number of registered sources for `type_name`.
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
    /// seg.just(1024i32);
    /// lookup
    ///     .lookup_cast("f64", &mut seg, Span::call_site(), Span::call_site())
    ///     .unwrap();
    /// assert_eq!(seg.call0::<f64>().unwrap(), 1024.0);
    /// ```
    pub fn lookup_cast(
        &self,
        type_name: &str,
        segment: &mut DynSegment,
        start: proc_macro2::Span,
        end: proc_macro2::Span,
    ) -> std::result::Result<(), crate::ParseError> {
        let source_span = SourceSpan::from_proc_macro2_range(start, end);
        let Some(signatures) = signatures_for_cast(type_name) else {
            return Err(crate::ParseError::new_range(
                format!("unknown type `{type_name}`"),
                start,
                end,
            ));
        };
        let Some(operand) = segment.peek_stack_infos(1).first() else {
            return Err(crate::ParseError::new_range(
                "cast requires an operand on the stack".to_string(),
                start,
                end,
            ));
        };
        let source_type_id = operand.type_id;
        for sig in signatures {
            if sig.source_type_id() == source_type_id {
                (sig.op_fn)(segment, source_span).map_err(|e| {
                    crate::ParseError::new_range(format!("cast error: {}", e), start, end)
                })?;
                return Ok(());
            }
        }
        Err(crate::ParseError::new_range(
            format!("no cast from `{}` to `{type_name}`", operand.type_name),
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
        let err = result.unwrap_err();
        let message = format!("{:#}", err);
        assert!(
            message.contains("arithmetic overflow"),
            "error message should mention arithmetic overflow, got: {message}"
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
        let err = result.unwrap_err();
        let message = format!("{:#}", err);
        assert!(
            message.contains("division by zero"),
            "error message should mention division by zero, got: {message}"
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
        let err = result.unwrap_err();
        let message = format!("{:#}", err);
        assert!(
            message.contains("division by zero"),
            "error message should mention division by zero, got: {message}"
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
        assert!(segment.call0::<bool>()?);
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
        assert!(!segment.call0::<bool>()?);
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
    fn round_rounds_half_away_from_zero() -> Result<()> {
        // 3.5/-3.5 are the actual halfway cases (3.6 rounds to 4.0 regardless of which direction
        // "away from zero" means, so it can't distinguish this rule from ordinary
        // round-to-nearest); checking both signs also confirms "away from zero" rather than
        // "toward positive infinity".
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        lookup.lookup(
            "round",
            &mut segment,
            0,
            Span::call_site(),
            Span::call_site(),
        )?;
        segment.just(3.5f64);
        lookup.lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<f64>()?, 4.0);

        let mut segment = DynSegment::new::<()>();
        lookup.lookup(
            "round",
            &mut segment,
            0,
            Span::call_site(),
            Span::call_site(),
        )?;
        segment.just(-3.5f64);
        lookup.lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<f64>()?, -4.0);
        Ok(())
    }

    #[test]
    fn round_of_an_expression_result() -> Result<()> {
        // The motivating case: converting a physical size times a resolution
        // (both f64) into a whole pixel count, still as an `f64` - narrowing
        // to `i32` is a separate `as` cast, tested in the cast tests below.
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        lookup.lookup(
            "round",
            &mut segment,
            0,
            Span::call_site(),
            Span::call_site(),
        )?;
        segment.just(3.41333333f64);
        segment.just(300.0f64);
        lookup.lookup("*", &mut segment, 2, Span::call_site(), Span::call_site())?;
        lookup.lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<f64>()?, 1024.0);
        Ok(())
    }

    #[test]
    fn round_scope_declines_a_call_whose_callee_is_not_round() -> Result<()> {
        // Defensive case for round_scope's own ("()", 2) arm: a callee that
        // isn't the `RoundFn` marker must be declined (Ok(false)), not
        // mistaken for a round() call - see round_scope's doc comment.
        let mut segment = DynSegment::new::<()>();
        segment.just(7i32);
        segment.just(3.0f64);
        let handled = round_scope("()", &mut segment, 2, SourceSpan::new(1, 0, 1, 1))?;
        assert!(!handled);
        Ok(())
    }

    #[test]
    fn round_scope_declines_rather_than_panics_on_an_undersized_stack() -> Result<()> {
        // Regression test: `("()", 2)` used to index `peek_stack_infos(2)[0]` unconditionally,
        // but `peek_stack_infos` returns an *empty* slice (not a short one) when the stack has
        // fewer than the requested count - an empty stack here panicked instead of declining.
        let mut segment = DynSegment::new::<()>();
        let handled = round_scope("()", &mut segment, 2, SourceSpan::new(1, 0, 1, 1))?;
        assert!(!handled);

        let mut segment = DynSegment::new::<()>();
        segment.just(3.0f64); // only one of the two expected operands
        let handled = round_scope("()", &mut segment, 2, SourceSpan::new(1, 0, 1, 1))?;
        assert!(!handled);
        Ok(())
    }

    #[test]
    fn lookup_cast_errors_rather_than_panics_on_an_empty_stack() -> Result<()> {
        // Regression test: `lookup_cast` used to index `peek_stack_infos(1)[0]` unconditionally,
        // which panicked (rather than returning a `ParseError`) when the stack was empty -
        // reachable directly through this public API without going through the grammar, which
        // always pushes the operand first.
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        let result = lookup.lookup_cast("i32", &mut segment, Span::call_site(), Span::call_site());
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn cast_widens_i32_to_f64_exactly() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(1024i32);
        lookup.lookup_cast("f64", &mut segment, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<f64>()?, 1024.0);
        Ok(())
    }

    #[test]
    fn cast_narrows_f64_to_i32_when_the_value_fits() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(1024.0f64);
        lookup.lookup_cast("i32", &mut segment, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<i32>()?, 1024);
        Ok(())
    }

    #[test]
    fn cast_composes_with_round_for_the_image_resize_pattern() -> Result<()> {
        // (width_px as f64) / dpi, mirrored back with round(... * dpi) as i32 -
        // the actual pattern image_resize.adm2 needs for its width_px triangle. Exercises the
        // full round trip: widening cast, round(), and the narrowing cast back to i32 - not just
        // the widening half (see the PR review comment this regression-tests: a prior version of
        // this test only checked `(width_px as f64) / dpi` and would not have caught a regression
        // in `round`'s dispatch or the checked `f64 as i32` narrowing cast).
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(1024i32);
        lookup.lookup_cast("f64", &mut segment, Span::call_site(), Span::call_site())?;
        segment.just(300.0f64);
        lookup.lookup("/", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<f64>()?, 1024.0 / 300.0);

        let mut segment = DynSegment::new::<()>();
        lookup.lookup(
            "round",
            &mut segment,
            0,
            Span::call_site(),
            Span::call_site(),
        )?;
        segment.just(1024.0f64 / 300.0);
        segment.just(300.0f64);
        lookup.lookup("*", &mut segment, 2, Span::call_site(), Span::call_site())?;
        lookup.lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())?;
        lookup.lookup_cast("i32", &mut segment, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<i32>()?, 1024);
        Ok(())
    }

    #[test]
    fn cast_errors_when_the_value_does_not_fit_in_the_target_type() -> Result<()> {
        // lookup_cast's own Result only reports type-checking failures (unknown type, no
        // registered source) - a checked cast's own out-of-range failure is deferred to
        // execution, same as any other fallible op (see DynSegment::op1r's doc comment), so
        // it's asserted via call0.
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(1.0e20f64);
        lookup.lookup_cast("i32", &mut segment, Span::call_site(), Span::call_site())?;
        assert!(segment.call0::<i32>().is_err());
        Ok(())
    }

    #[test]
    fn cast_errors_just_past_the_target_types_max_even_when_max_is_not_exactly_representable()
    -> Result<()> {
        // Regression test: `i32::MAX as f32` isn't exactly representable (f32's 24-bit mantissa
        // can't hold all 31 value bits) and rounds *up* to exactly `i32::MAX as i64 + 1`
        // (2147483648.0, a power of two). A bound check comparing against that rounded value
        // with `>` would let this value silently pass, then Rust's saturating `as` would turn it
        // into `i32::MAX` instead of the `Err` a genuinely out-of-range value should produce.
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(2147483648.0f32); // i32::MAX + 1, exactly representable in f32
        lookup.lookup_cast("i32", &mut segment, Span::call_site(), Span::call_site())?;
        assert!(segment.call0::<i32>().is_err());
        Ok(())
    }

    #[test]
    fn cast_accepts_the_target_types_actual_max_value() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(2147483520.0f32); // largest f32 value below i32::MAX + 1
        lookup.lookup_cast("i32", &mut segment, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<i32>()?, 2147483520);
        Ok(())
    }

    #[test]
    fn cast_errors_just_past_i64_max_from_f64() -> Result<()> {
        // Same rounding pitfall as the f32 -> i32 case above, but for the f64 -> i64 pair (f64's
        // 53-bit mantissa can't hold i64's 63 value bits either).
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(9223372036854775808.0f64); // i64::MAX + 1, exactly representable in f64
        lookup.lookup_cast("i64", &mut segment, Span::call_site(), Span::call_site())?;
        assert!(segment.call0::<i64>().is_err());
        Ok(())
    }

    #[test]
    fn cast_errors_for_an_unknown_target_type() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(1i32);
        let result = lookup.lookup_cast(
            "nonsense",
            &mut segment,
            Span::call_site(),
            Span::call_site(),
        );
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn cast_errors_for_an_unregistered_source_type() -> Result<()> {
        // String -> i32 has no registered cast: Rust's own `as` has no conversion from `String`
        // to anything (see the "Casts" section's doc comment).
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just("1".to_string());
        let result = lookup.lookup_cast("i32", &mut segment, Span::call_site(), Span::call_site());
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn cast_from_bool_is_registered_for_every_integer_width() -> Result<()> {
        // Matches Rust's own `bool as <int>`: infallible for all 12 integer widths.
        let lookup = OpLookup::new();
        for target in [
            "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
        ] {
            let mut segment = DynSegment::new::<()>();
            segment.just(true);
            lookup.lookup_cast(target, &mut segment, Span::call_site(), Span::call_site())?;
        }
        Ok(())
    }

    #[test]
    fn cast_widens_bool_to_a_specific_integer_value() -> Result<()> {
        // Matches Rust's own `bool as <int>`: `true` -> 1, `false` -> 0.
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(true);
        lookup.lookup_cast("i32", &mut segment, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<i32>()?, 1);

        let mut segment = DynSegment::new::<()>();
        segment.just(false);
        lookup.lookup_cast("u64", &mut segment, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<u64>()?, 0);
        Ok(())
    }

    #[test]
    fn cast_bool_to_bool_is_a_no_op_identity_conversion() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(true);
        lookup.lookup_cast("bool", &mut segment, Span::call_site(), Span::call_site())?;
        assert!(segment.call0::<bool>()?);
        Ok(())
    }

    #[test]
    fn cast_string_to_string_is_a_no_op_identity_conversion() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just("hello".to_string());
        lookup.lookup_cast("String", &mut segment, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<String>()?, "hello");
        Ok(())
    }

    #[test]
    fn cast_errors_from_a_number_to_bool() -> Result<()> {
        // Rust's own `as` rejects `<int> as bool` (E0054) - no int/float -> bool conversion is
        // registered, matching Rust exactly, not just "unknown type".
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(1i32);
        let result = lookup.lookup_cast("bool", &mut segment, Span::call_site(), Span::call_site());
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn cast_errors_from_bool_to_a_float() -> Result<()> {
        // Rust's own `as` rejects `bool as f64` (E0606: "cast through an integer first").
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(true);
        let result = lookup.lookup_cast("f64", &mut segment, Span::call_site(), Span::call_site());
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn cast_errors_between_string_and_bool_in_both_directions() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(true);
        assert!(
            lookup
                .lookup_cast("String", &mut segment, Span::call_site(), Span::call_site())
                .is_err()
        );

        let mut segment = DynSegment::new::<()>();
        segment.just("true".to_string());
        assert!(
            lookup
                .lookup_cast("bool", &mut segment, Span::call_site(), Span::call_site())
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn cast_errors_between_string_and_numbers_in_both_directions() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(1i32);
        assert!(
            lookup
                .lookup_cast("String", &mut segment, Span::call_site(), Span::call_site())
                .is_err()
        );

        let mut segment = DynSegment::new::<()>();
        segment.just("1".to_string());
        assert!(
            lookup
                .lookup_cast("i32", &mut segment, Span::call_site(), Span::call_site())
                .is_err()
        );
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
        let err = result.unwrap_err();
        let message = format!("{:#}", err);
        assert!(
            message.contains("shift overflow"),
            "error message should mention shift overflow, got: {message}"
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
        let err = result.unwrap_err();
        let message = format!("{:#}", err);
        assert!(message.contains("shift overflow"), "got: {message}");
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
        let err = result.unwrap_err();
        let message = format!("{:#}", err);
        assert!(message.contains("shift overflow"), "got: {message}");
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

    /// Verifies that `ScopeFn` closures compile when written with an explicit `SourceSpan`
    /// parameter, confirming the type alias signature is correct.
    #[cfg(feature = "span-diagnostics")]
    #[test]
    fn scope_fn_accepts_source_span_parameter() {
        let mut lookup = OpLookup::new();
        lookup.push_scope(
            |_name: &str, _seg: &mut DynSegment, _n: usize, span: crate::SourceSpan| {
                // span is available for forwarding to op closures
                let _ = span;
                Ok(false)
            },
        );
        // If this compiles, the ScopeFn signature correctly includes SourceSpan.
    }

    /// Verifies that `FormatRustcStyle` on an `anyhow::Error` without a `SpanContext`
    /// falls back to the plain error message — the expected behavior for errors from
    /// client-added ops that do not attach span context.
    #[test]
    fn format_rustc_style_falls_back_for_client_added_op_error() {
        use crate::FormatRustcStyle;
        use annotate_snippets::Renderer;

        let err = anyhow::anyhow!("custom domain error");
        let output = err.format_rustc_style("unused source", "unused.cel", 1, &Renderer::plain());
        assert_eq!(output, "custom domain error");
    }

    #[cfg(feature = "span-diagnostics")]
    #[test]
    fn runtime_error_carries_span_context() {
        use crate::{CELParser, FormatRustcStyle, SpanContext};
        use annotate_snippets::Renderer;

        let mut parser = CELParser::new(OpLookup::new());
        let source = "1i32 + 2147483647i32"; // i32::MAX + 1 → overflow
        let mut segment = parser.parse_str(source).expect("should parse");
        let err = segment.call0::<i32>().expect_err("should overflow");
        let ctx = err
            .downcast_ref::<SpanContext>()
            .expect("expected SpanContext on runtime error");
        // The span is on line 1 (1-indexed). In test mode, proc_macro2 with
        // span-locations assigns spans relative to the parsed string, starting
        // at column 0 for the first token on each line.
        assert_eq!(ctx.span().start.line, 1);
        // End-to-end rendering must mention the error and mark the source location.
        let rendered = err.format_rustc_style(source, "test.cel", 1, &Renderer::plain());
        assert!(
            rendered.contains("arithmetic overflow"),
            "expected 'arithmetic overflow' in rendered output, got: {rendered}"
        );
        assert!(
            rendered.contains('^'),
            "expected caret marker in rendered output, got: {rendered}"
        );
    }

    #[test]
    fn tuple_shaped_signature_matches_and_dispatches() -> Result<()> {
        let mut lookup = OpLookup::new();
        lookup.register_tuple_op(TupleOpSignature {
            name: "greet".to_string(),
            shape: vec![TypeId::of::<String>(), TypeId::of::<i32>()],
            tuple_operand_index: 0,
            operand_type_ids: vec![],
            op_fn: |seg, _span| {
                seg.tuple_index(1);
                seg.op1(|_ignored: i32| true)
            },
        });

        let mut segment = DynSegment::new::<()>();
        let ambient_start = segment.current_stack_offset();
        segment.op0(|| "hi".to_string());
        segment.op0(|| 7i32);
        segment.make_tuple(2, ambient_start);

        lookup.lookup(
            "greet",
            &mut segment,
            1,
            Span::call_site(),
            Span::call_site(),
        )?;
        assert!(segment.call0::<bool>()?);
        Ok(())
    }

    #[test]
    fn tuple_shaped_signature_does_not_match_wrong_shape() {
        let mut lookup = OpLookup::new();
        lookup.register_tuple_op(TupleOpSignature {
            name: "greet".to_string(),
            shape: vec![TypeId::of::<String>(), TypeId::of::<i32>()],
            tuple_operand_index: 0,
            operand_type_ids: vec![],
            op_fn: |seg, _span| {
                seg.tuple_index(1);
                seg.op1(|_ignored: i32| true)
            },
        });

        let mut segment = DynSegment::new::<()>();
        let ambient_start = segment.current_stack_offset();
        segment.op0(|| 1i32);
        segment.op0(|| 2i32);
        segment.make_tuple(2, ambient_start);

        let result = lookup.lookup(
            "greet",
            &mut segment,
            1,
            Span::call_site(),
            Span::call_site(),
        );
        assert!(
            result.is_err(),
            "shape (i32, i32) should not match (String, i32)"
        );
    }

    #[test]
    fn tuple_shaped_signature_with_empty_shape_does_not_match_non_tuple() {
        // Regression test: a 0-element `shape` must only match an actual
        // 0-arity tuple, not any non-tuple operand (which also reports an
        // empty `associated` list).
        let mut lookup = OpLookup::new();
        lookup.register_tuple_op(TupleOpSignature {
            name: "unit_greet".to_string(),
            shape: vec![],
            tuple_operand_index: 0,
            operand_type_ids: vec![],
            op_fn: |seg, _span| seg.op1(|_ignored: i32| true),
        });

        let mut segment = DynSegment::new::<()>();
        segment.op0(|| 42i32);

        let result = lookup.lookup(
            "unit_greet",
            &mut segment,
            1,
            Span::call_site(),
            Span::call_site(),
        );
        assert!(
            result.is_err(),
            "empty-shape tuple signature must not match a non-tuple operand"
        );
    }

    #[test]
    fn builtin_operand_types_reports_a_binary_arithmetic_overload() {
        let sigs = builtin_operand_types("+");
        assert!(
            sigs.iter().any(|s| s.arity == 2
                && s.lhs == TypeId::of::<i32>()
                && s.rhs == TypeId::of::<i32>())
        );
    }

    #[test]
    fn builtin_operand_types_reports_a_unary_overload() {
        let sigs = builtin_operand_types("!");
        assert!(
            sigs.iter()
                .any(|s| s.arity == 1 && s.lhs == TypeId::of::<bool>())
        );
    }

    #[test]
    fn builtin_operand_types_includes_unary_negation_but_only_for_signed_and_float_types() {
        let sigs = builtin_operand_types("-");
        assert!(
            sigs.iter()
                .any(|s| s.arity == 1 && s.lhs == TypeId::of::<i32>())
        );
        assert!(
            sigs.iter()
                .any(|s| s.arity == 2 && s.lhs == TypeId::of::<i32>())
        );
        assert!(
            !sigs
                .iter()
                .any(|s| s.arity == 1 && s.lhs == TypeId::of::<u32>()),
            "unsigned types have no unary negation overload"
        );
    }

    #[test]
    fn builtin_operand_types_covers_heterogeneous_shift_signatures() {
        let sigs = builtin_operand_types("<<");
        assert!(
            sigs.iter().any(|s| s.arity == 2
                && s.lhs == TypeId::of::<u64>()
                && s.rhs == TypeId::of::<u32>())
        );
    }

    #[test]
    fn builtin_operand_types_is_empty_for_an_unregistered_name() {
        assert!(builtin_operand_types("not_an_operator").is_empty());
    }

    #[test]
    fn builtin_operand_types_is_empty_for_a_runtime_only_tuple_op() {
        // Tuple-shaped ops are registered on an OpLookup instance at runtime
        // (OpLookup::register_tuple_op), never in the static BUILTINS table this function reads —
        // confirming they're invisible here, not an oversight.
        assert!(builtin_operand_types("greet").is_empty());
    }

    #[test]
    fn builtin_scalar_type_resolves_every_documented_name() {
        for name in [
            "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
            "f32", "f64", "bool", "String",
        ] {
            let scalar =
                builtin_scalar_type(name).unwrap_or_else(|| panic!("expected `{name}` to resolve"));
            assert_eq!(scalar.type_name, name);
        }
        assert!(builtin_scalar_type("not_a_type").is_none());
    }

    #[test]
    fn builtin_scalar_type_i32_matches_std_any_type_id() {
        let scalar = builtin_scalar_type("i32").unwrap();
        assert_eq!(scalar.type_id, TypeId::of::<i32>());
        assert_eq!(scalar.size, std::mem::size_of::<i32>());
        assert_eq!(scalar.align, std::mem::align_of::<i32>());
    }

    #[test]
    fn builtin_scalar_type_push_arg_declares_a_readable_argument() {
        let scalar = builtin_scalar_type("i32").unwrap();
        let mut segment = DynSegment::new::<()>();
        (scalar.push_arg)(&mut segment, 0);
        let value = 42i32;
        let result: i32 = segment.call_dyn(&[&value]).unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn isolate_scopes_removes_pushed_scopes_until_restored() {
        let mut lookup = OpLookup::new();
        lookup.push_scope(|name, segment, arity, _span| {
            if name == "custom" && arity == 0 {
                segment.just(1i32);
                Ok(true)
            } else {
                Ok(false)
            }
        });

        let mut segment = DynSegment::new::<()>();
        let isolated = lookup.isolate_scopes();
        let err = lookup.lookup(
            "custom",
            &mut segment,
            0,
            proc_macro2::Span::call_site(),
            proc_macro2::Span::call_site(),
        );
        assert!(
            err.is_err(),
            "custom scope must not be reachable while isolated"
        );

        lookup.restore_scopes(isolated);
        let mut segment = DynSegment::new::<()>();
        lookup
            .lookup(
                "custom",
                &mut segment,
                0,
                proc_macro2::Span::call_site(),
                proc_macro2::Span::call_site(),
            )
            .unwrap();
        assert_eq!(segment.call0::<i32>().unwrap(), 1);
    }

    #[test]
    fn isolate_scopes_leaves_library_scopes_reachable() {
        // round_scope's own protocol is two lookups: ("round", 0) pushes a marker value, then
        // ("()", 2) (with the marker plus an f64 operand on the stack) computes the actual round.
        // This test only needs to prove the *first* half is still reachable while isolated — that's
        // enough to demonstrate round_scope (a library scope) survived isolate_scopes, without
        // needing to replicate the whole call protocol.
        let mut lookup = OpLookup::new(); // registers round_scope via push_library_scope
        let mut segment = DynSegment::new::<()>();
        let isolated = lookup.isolate_scopes();
        lookup
            .lookup(
                "round",
                &mut segment,
                0,
                proc_macro2::Span::call_site(),
                proc_macro2::Span::call_site(),
            )
            .expect("round is a library scope and must survive isolation");
        lookup.restore_scopes(isolated);
        assert_eq!(segment.peek_stack_infos(1).len(), 1); // the RoundFn marker was pushed
    }
}
