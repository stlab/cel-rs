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
//! - **Type optimization**: Since all built-in operations have matching operand types,
//!   signatures store a single `TypeId` plus arity rather than arrays.

use anyhow::{anyhow, Result};
use cel_runtime::DynSegment;
use once_cell::sync::Lazy;
use phf::phf_map;
use std::any::TypeId;

/// A function that pushes an operation onto a DynSegment.
///
/// This is a simple function pointer since built-in operations have no state.
pub type OpFn = fn(&mut DynSegment) -> Result<()>;

/// A scope function that attempts to resolve and apply an operation.
///
/// Takes an operation name, type signature, and segment. Returns `Ok(true)` if the
/// operation was handled, `Ok(false)` if not found, or `Err` on error.
pub type ScopeFn = Box<dyn Fn(&str, &[TypeId], &mut DynSegment) -> Result<bool> + Send + Sync>;

/// A signature for an operation with matching operand types.
///
/// For example, `u32 + u32 -> u32` would have `type_id_index = TYPE_U32`
/// and `arity = 2`. This optimization reduces memory usage by ~50% compared to
/// storing full type arrays.
#[derive(Clone, Copy)]
struct OpSignature {
    /// Index into TYPE_IDS vector for the TypeId that all operands must match
    type_id_index: usize,
    /// Number of operands this operation accepts
    arity: u8,
    /// Function pointer to the operation implementation
    op_fn: OpFn,
}

impl OpSignature {
    /// Returns the TypeId for this signature.
    fn type_id(&self) -> TypeId {
        TYPE_IDS[self.type_id_index]
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

// Helper macro to reduce boilerplate in signature definitions
macro_rules! sig {
    ($type_idx:expr, $arity:expr, $closure:expr) => {
        OpSignature {
            type_id_index: $type_idx,
            arity: $arity,
            op_fn: $closure,
        }
    };
}

// Addition signatures
static ADD_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg| seg.op2(|a: u8, b: u8| a.wrapping_add(b))),
    sig!(TYPE_U16, 2, |seg| seg.op2(|a: u16, b: u16| a.wrapping_add(b))),
    sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a.wrapping_add(b))),
    sig!(TYPE_U64, 2, |seg| seg.op2(|a: u64, b: u64| a.wrapping_add(b))),
    sig!(TYPE_U128, 2, |seg| seg.op2(|a: u128, b: u128| a.wrapping_add(b))),
    sig!(TYPE_USIZE, 2, |seg| seg.op2(|a: usize, b: usize| a.wrapping_add(b))),
    sig!(TYPE_I8, 2, |seg| seg.op2(|a: i8, b: i8| a.wrapping_add(b))),
    sig!(TYPE_I16, 2, |seg| seg.op2(|a: i16, b: i16| a.wrapping_add(b))),
    sig!(TYPE_I32, 2, |seg| seg.op2(|a: i32, b: i32| a.wrapping_add(b))),
    sig!(TYPE_I64, 2, |seg| seg.op2(|a: i64, b: i64| a.wrapping_add(b))),
    sig!(TYPE_I128, 2, |seg| seg.op2(|a: i128, b: i128| a.wrapping_add(b))),
    sig!(TYPE_ISIZE, 2, |seg| seg.op2(|a: isize, b: isize| a.wrapping_add(b))),
    sig!(TYPE_F32, 2, |seg| seg.op2(|a: f32, b: f32| a + b)),
    sig!(TYPE_F64, 2, |seg| seg.op2(|a: f64, b: f64| a + b)),
];

// Subtraction signatures (both binary and unary)
static SUB_SIGNATURES: &[OpSignature] = &[
    // Binary subtraction
    sig!(TYPE_U8, 2, |seg| seg.op2(|a: u8, b: u8| a.wrapping_sub(b))),
    sig!(TYPE_U16, 2, |seg| seg.op2(|a: u16, b: u16| a.wrapping_sub(b))),
    sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a.wrapping_sub(b))),
    sig!(TYPE_U64, 2, |seg| seg.op2(|a: u64, b: u64| a.wrapping_sub(b))),
    sig!(TYPE_U128, 2, |seg| seg.op2(|a: u128, b: u128| a.wrapping_sub(b))),
    sig!(TYPE_USIZE, 2, |seg| seg.op2(|a: usize, b: usize| a.wrapping_sub(b))),
    sig!(TYPE_I8, 2, |seg| seg.op2(|a: i8, b: i8| a.wrapping_sub(b))),
    sig!(TYPE_I16, 2, |seg| seg.op2(|a: i16, b: i16| a.wrapping_sub(b))),
    sig!(TYPE_I32, 2, |seg| seg.op2(|a: i32, b: i32| a.wrapping_sub(b))),
    sig!(TYPE_I64, 2, |seg| seg.op2(|a: i64, b: i64| a.wrapping_sub(b))),
    sig!(TYPE_I128, 2, |seg| seg.op2(|a: i128, b: i128| a.wrapping_sub(b))),
    sig!(TYPE_ISIZE, 2, |seg| seg.op2(|a: isize, b: isize| a.wrapping_sub(b))),
    sig!(TYPE_F32, 2, |seg| seg.op2(|a: f32, b: f32| a - b)),
    sig!(TYPE_F64, 2, |seg| seg.op2(|a: f64, b: f64| a - b)),
    // Unary negation
    sig!(TYPE_I8, 1, |seg| seg.op1(|a: i8| a.wrapping_neg())),
    sig!(TYPE_I16, 1, |seg| seg.op1(|a: i16| a.wrapping_neg())),
    sig!(TYPE_I32, 1, |seg| seg.op1(|a: i32| a.wrapping_neg())),
    sig!(TYPE_I64, 1, |seg| seg.op1(|a: i64| a.wrapping_neg())),
    sig!(TYPE_I128, 1, |seg| seg.op1(|a: i128| a.wrapping_neg())),
    sig!(TYPE_ISIZE, 1, |seg| seg.op1(|a: isize| a.wrapping_neg())),
    sig!(TYPE_F32, 1, |seg| seg.op1(|a: f32| -a)),
    sig!(TYPE_F64, 1, |seg| seg.op1(|a: f64| -a)),
];

// Multiplication signatures
static MUL_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg| seg.op2(|a: u8, b: u8| a.wrapping_mul(b))),
    sig!(TYPE_U16, 2, |seg| seg.op2(|a: u16, b: u16| a.wrapping_mul(b))),
    sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a.wrapping_mul(b))),
    sig!(TYPE_U64, 2, |seg| seg.op2(|a: u64, b: u64| a.wrapping_mul(b))),
    sig!(TYPE_U128, 2, |seg| seg.op2(|a: u128, b: u128| a.wrapping_mul(b))),
    sig!(TYPE_USIZE, 2, |seg| seg.op2(|a: usize, b: usize| a.wrapping_mul(b))),
    sig!(TYPE_I8, 2, |seg| seg.op2(|a: i8, b: i8| a.wrapping_mul(b))),
    sig!(TYPE_I16, 2, |seg| seg.op2(|a: i16, b: i16| a.wrapping_mul(b))),
    sig!(TYPE_I32, 2, |seg| seg.op2(|a: i32, b: i32| a.wrapping_mul(b))),
    sig!(TYPE_I64, 2, |seg| seg.op2(|a: i64, b: i64| a.wrapping_mul(b))),
    sig!(TYPE_I128, 2, |seg| seg.op2(|a: i128, b: i128| a.wrapping_mul(b))),
    sig!(TYPE_ISIZE, 2, |seg| seg.op2(|a: isize, b: isize| a.wrapping_mul(b))),
    sig!(TYPE_F32, 2, |seg| seg.op2(|a: f32, b: f32| a * b)),
    sig!(TYPE_F64, 2, |seg| seg.op2(|a: f64, b: f64| a * b)),
];

// Division signatures
static DIV_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg| seg.op2(|a: u8, b: u8| a / b)),
    sig!(TYPE_U16, 2, |seg| seg.op2(|a: u16, b: u16| a / b)),
    sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a / b)),
    sig!(TYPE_U64, 2, |seg| seg.op2(|a: u64, b: u64| a / b)),
    sig!(TYPE_U128, 2, |seg| seg.op2(|a: u128, b: u128| a / b)),
    sig!(TYPE_USIZE, 2, |seg| seg.op2(|a: usize, b: usize| a / b)),
    sig!(TYPE_I8, 2, |seg| seg.op2(|a: i8, b: i8| a / b)),
    sig!(TYPE_I16, 2, |seg| seg.op2(|a: i16, b: i16| a / b)),
    sig!(TYPE_I32, 2, |seg| seg.op2(|a: i32, b: i32| a / b)),
    sig!(TYPE_I64, 2, |seg| seg.op2(|a: i64, b: i64| a / b)),
    sig!(TYPE_I128, 2, |seg| seg.op2(|a: i128, b: i128| a / b)),
    sig!(TYPE_ISIZE, 2, |seg| seg.op2(|a: isize, b: isize| a / b)),
    sig!(TYPE_F32, 2, |seg| seg.op2(|a: f32, b: f32| a / b)),
    sig!(TYPE_F64, 2, |seg| seg.op2(|a: f64, b: f64| a / b)),
];

// Modulo signatures
static MOD_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg| seg.op2(|a: u8, b: u8| a % b)),
    sig!(TYPE_U16, 2, |seg| seg.op2(|a: u16, b: u16| a % b)),
    sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a % b)),
    sig!(TYPE_U64, 2, |seg| seg.op2(|a: u64, b: u64| a % b)),
    sig!(TYPE_U128, 2, |seg| seg.op2(|a: u128, b: u128| a % b)),
    sig!(TYPE_USIZE, 2, |seg| seg.op2(|a: usize, b: usize| a % b)),
    sig!(TYPE_I8, 2, |seg| seg.op2(|a: i8, b: i8| a % b)),
    sig!(TYPE_I16, 2, |seg| seg.op2(|a: i16, b: i16| a % b)),
    sig!(TYPE_I32, 2, |seg| seg.op2(|a: i32, b: i32| a % b)),
    sig!(TYPE_I64, 2, |seg| seg.op2(|a: i64, b: i64| a % b)),
    sig!(TYPE_I128, 2, |seg| seg.op2(|a: i128, b: i128| a % b)),
    sig!(TYPE_ISIZE, 2, |seg| seg.op2(|a: isize, b: isize| a % b)),
    sig!(TYPE_F32, 2, |seg| seg.op2(|a: f32, b: f32| a % b)),
    sig!(TYPE_F64, 2, |seg| seg.op2(|a: f64, b: f64| a % b)),
];

// Bitwise AND signatures
static BITWISE_AND_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg| seg.op2(|a: u8, b: u8| a & b)),
    sig!(TYPE_U16, 2, |seg| seg.op2(|a: u16, b: u16| a & b)),
    sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a & b)),
    sig!(TYPE_U64, 2, |seg| seg.op2(|a: u64, b: u64| a & b)),
    sig!(TYPE_U128, 2, |seg| seg.op2(|a: u128, b: u128| a & b)),
    sig!(TYPE_USIZE, 2, |seg| seg.op2(|a: usize, b: usize| a & b)),
    sig!(TYPE_I8, 2, |seg| seg.op2(|a: i8, b: i8| a & b)),
    sig!(TYPE_I16, 2, |seg| seg.op2(|a: i16, b: i16| a & b)),
    sig!(TYPE_I32, 2, |seg| seg.op2(|a: i32, b: i32| a & b)),
    sig!(TYPE_I64, 2, |seg| seg.op2(|a: i64, b: i64| a & b)),
    sig!(TYPE_I128, 2, |seg| seg.op2(|a: i128, b: i128| a & b)),
    sig!(TYPE_ISIZE, 2, |seg| seg.op2(|a: isize, b: isize| a & b)),
];

// Bitwise OR signatures
static BITWISE_OR_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg| seg.op2(|a: u8, b: u8| a | b)),
    sig!(TYPE_U16, 2, |seg| seg.op2(|a: u16, b: u16| a | b)),
    sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a | b)),
    sig!(TYPE_U64, 2, |seg| seg.op2(|a: u64, b: u64| a | b)),
    sig!(TYPE_U128, 2, |seg| seg.op2(|a: u128, b: u128| a | b)),
    sig!(TYPE_USIZE, 2, |seg| seg.op2(|a: usize, b: usize| a | b)),
    sig!(TYPE_I8, 2, |seg| seg.op2(|a: i8, b: i8| a | b)),
    sig!(TYPE_I16, 2, |seg| seg.op2(|a: i16, b: i16| a | b)),
    sig!(TYPE_I32, 2, |seg| seg.op2(|a: i32, b: i32| a | b)),
    sig!(TYPE_I64, 2, |seg| seg.op2(|a: i64, b: i64| a | b)),
    sig!(TYPE_I128, 2, |seg| seg.op2(|a: i128, b: i128| a | b)),
    sig!(TYPE_ISIZE, 2, |seg| seg.op2(|a: isize, b: isize| a | b)),
];

// Bitwise XOR signatures
static BITWISE_XOR_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg| seg.op2(|a: u8, b: u8| a ^ b)),
    sig!(TYPE_U16, 2, |seg| seg.op2(|a: u16, b: u16| a ^ b)),
    sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a ^ b)),
    sig!(TYPE_U64, 2, |seg| seg.op2(|a: u64, b: u64| a ^ b)),
    sig!(TYPE_U128, 2, |seg| seg.op2(|a: u128, b: u128| a ^ b)),
    sig!(TYPE_USIZE, 2, |seg| seg.op2(|a: usize, b: usize| a ^ b)),
    sig!(TYPE_I8, 2, |seg| seg.op2(|a: i8, b: i8| a ^ b)),
    sig!(TYPE_I16, 2, |seg| seg.op2(|a: i16, b: i16| a ^ b)),
    sig!(TYPE_I32, 2, |seg| seg.op2(|a: i32, b: i32| a ^ b)),
    sig!(TYPE_I64, 2, |seg| seg.op2(|a: i64, b: i64| a ^ b)),
    sig!(TYPE_I128, 2, |seg| seg.op2(|a: i128, b: i128| a ^ b)),
    sig!(TYPE_ISIZE, 2, |seg| seg.op2(|a: isize, b: isize| a ^ b)),
];

// Left shift signatures
static LEFT_SHIFT_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg| seg.op2(|a: u8, b: u8| a << b)),
    sig!(TYPE_U16, 2, |seg| seg.op2(|a: u16, b: u16| a << b)),
    sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a << b)),
    sig!(TYPE_U64, 2, |seg| seg.op2(|a: u64, b: u64| a << b)),
    sig!(TYPE_U128, 2, |seg| seg.op2(|a: u128, b: u128| a << b)),
    sig!(TYPE_USIZE, 2, |seg| seg.op2(|a: usize, b: usize| a << b)),
    sig!(TYPE_I8, 2, |seg| seg.op2(|a: i8, b: i8| a << b)),
    sig!(TYPE_I16, 2, |seg| seg.op2(|a: i16, b: i16| a << b)),
    sig!(TYPE_I32, 2, |seg| seg.op2(|a: i32, b: i32| a << b)),
    sig!(TYPE_I64, 2, |seg| seg.op2(|a: i64, b: i64| a << b)),
    sig!(TYPE_I128, 2, |seg| seg.op2(|a: i128, b: i128| a << b)),
    sig!(TYPE_ISIZE, 2, |seg| seg.op2(|a: isize, b: isize| a << b)),
];

// Right shift signatures
static RIGHT_SHIFT_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg| seg.op2(|a: u8, b: u8| a >> b)),
    sig!(TYPE_U16, 2, |seg| seg.op2(|a: u16, b: u16| a >> b)),
    sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a >> b)),
    sig!(TYPE_U64, 2, |seg| seg.op2(|a: u64, b: u64| a >> b)),
    sig!(TYPE_U128, 2, |seg| seg.op2(|a: u128, b: u128| a >> b)),
    sig!(TYPE_USIZE, 2, |seg| seg.op2(|a: usize, b: usize| a >> b)),
    sig!(TYPE_I8, 2, |seg| seg.op2(|a: i8, b: i8| a >> b)),
    sig!(TYPE_I16, 2, |seg| seg.op2(|a: i16, b: i16| a >> b)),
    sig!(TYPE_I32, 2, |seg| seg.op2(|a: i32, b: i32| a >> b)),
    sig!(TYPE_I64, 2, |seg| seg.op2(|a: i64, b: i64| a >> b)),
    sig!(TYPE_I128, 2, |seg| seg.op2(|a: i128, b: i128| a >> b)),
    sig!(TYPE_ISIZE, 2, |seg| seg.op2(|a: isize, b: isize| a >> b)),
];

// Logical AND signatures
static LOGICAL_AND_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_BOOL, 2, |seg| seg.op2(|a: bool, b: bool| a && b)),
];

// Logical OR signatures
static LOGICAL_OR_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_BOOL, 2, |seg| seg.op2(|a: bool, b: bool| a || b)),
];

// Logical NOT signatures
static LOGICAL_NOT_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_BOOL, 1, |seg| seg.op1(|a: bool| !a)),
];

// Equality signatures
static EQUAL_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg| seg.op2(|a: u8, b: u8| a == b)),
    sig!(TYPE_U16, 2, |seg| seg.op2(|a: u16, b: u16| a == b)),
    sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a == b)),
    sig!(TYPE_U64, 2, |seg| seg.op2(|a: u64, b: u64| a == b)),
    sig!(TYPE_U128, 2, |seg| seg.op2(|a: u128, b: u128| a == b)),
    sig!(TYPE_USIZE, 2, |seg| seg.op2(|a: usize, b: usize| a == b)),
    sig!(TYPE_I8, 2, |seg| seg.op2(|a: i8, b: i8| a == b)),
    sig!(TYPE_I16, 2, |seg| seg.op2(|a: i16, b: i16| a == b)),
    sig!(TYPE_I32, 2, |seg| seg.op2(|a: i32, b: i32| a == b)),
    sig!(TYPE_I64, 2, |seg| seg.op2(|a: i64, b: i64| a == b)),
    sig!(TYPE_I128, 2, |seg| seg.op2(|a: i128, b: i128| a == b)),
    sig!(TYPE_ISIZE, 2, |seg| seg.op2(|a: isize, b: isize| a == b)),
    sig!(TYPE_F32, 2, |seg| seg.op2(|a: f32, b: f32| a == b)),
    sig!(TYPE_F64, 2, |seg| seg.op2(|a: f64, b: f64| a == b)),
    sig!(TYPE_BOOL, 2, |seg| seg.op2(|a: bool, b: bool| a == b)),
];

// Inequality signatures
static NOT_EQUAL_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg| seg.op2(|a: u8, b: u8| a != b)),
    sig!(TYPE_U16, 2, |seg| seg.op2(|a: u16, b: u16| a != b)),
    sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a != b)),
    sig!(TYPE_U64, 2, |seg| seg.op2(|a: u64, b: u64| a != b)),
    sig!(TYPE_U128, 2, |seg| seg.op2(|a: u128, b: u128| a != b)),
    sig!(TYPE_USIZE, 2, |seg| seg.op2(|a: usize, b: usize| a != b)),
    sig!(TYPE_I8, 2, |seg| seg.op2(|a: i8, b: i8| a != b)),
    sig!(TYPE_I16, 2, |seg| seg.op2(|a: i16, b: i16| a != b)),
    sig!(TYPE_I32, 2, |seg| seg.op2(|a: i32, b: i32| a != b)),
    sig!(TYPE_I64, 2, |seg| seg.op2(|a: i64, b: i64| a != b)),
    sig!(TYPE_I128, 2, |seg| seg.op2(|a: i128, b: i128| a != b)),
    sig!(TYPE_ISIZE, 2, |seg| seg.op2(|a: isize, b: isize| a != b)),
    sig!(TYPE_F32, 2, |seg| seg.op2(|a: f32, b: f32| a != b)),
    sig!(TYPE_F64, 2, |seg| seg.op2(|a: f64, b: f64| a != b)),
    sig!(TYPE_BOOL, 2, |seg| seg.op2(|a: bool, b: bool| a != b)),
];

// Less than signatures
static LESS_THAN_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg| seg.op2(|a: u8, b: u8| a < b)),
    sig!(TYPE_U16, 2, |seg| seg.op2(|a: u16, b: u16| a < b)),
    sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a < b)),
    sig!(TYPE_U64, 2, |seg| seg.op2(|a: u64, b: u64| a < b)),
    sig!(TYPE_U128, 2, |seg| seg.op2(|a: u128, b: u128| a < b)),
    sig!(TYPE_USIZE, 2, |seg| seg.op2(|a: usize, b: usize| a < b)),
    sig!(TYPE_I8, 2, |seg| seg.op2(|a: i8, b: i8| a < b)),
    sig!(TYPE_I16, 2, |seg| seg.op2(|a: i16, b: i16| a < b)),
    sig!(TYPE_I32, 2, |seg| seg.op2(|a: i32, b: i32| a < b)),
    sig!(TYPE_I64, 2, |seg| seg.op2(|a: i64, b: i64| a < b)),
    sig!(TYPE_I128, 2, |seg| seg.op2(|a: i128, b: i128| a < b)),
    sig!(TYPE_ISIZE, 2, |seg| seg.op2(|a: isize, b: isize| a < b)),
    sig!(TYPE_F32, 2, |seg| seg.op2(|a: f32, b: f32| a < b)),
    sig!(TYPE_F64, 2, |seg| seg.op2(|a: f64, b: f64| a < b)),
];

// Less than or equal signatures
static LESS_THAN_OR_EQUAL_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg| seg.op2(|a: u8, b: u8| a <= b)),
    sig!(TYPE_U16, 2, |seg| seg.op2(|a: u16, b: u16| a <= b)),
    sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a <= b)),
    sig!(TYPE_U64, 2, |seg| seg.op2(|a: u64, b: u64| a <= b)),
    sig!(TYPE_U128, 2, |seg| seg.op2(|a: u128, b: u128| a <= b)),
    sig!(TYPE_USIZE, 2, |seg| seg.op2(|a: usize, b: usize| a <= b)),
    sig!(TYPE_I8, 2, |seg| seg.op2(|a: i8, b: i8| a <= b)),
    sig!(TYPE_I16, 2, |seg| seg.op2(|a: i16, b: i16| a <= b)),
    sig!(TYPE_I32, 2, |seg| seg.op2(|a: i32, b: i32| a <= b)),
    sig!(TYPE_I64, 2, |seg| seg.op2(|a: i64, b: i64| a <= b)),
    sig!(TYPE_I128, 2, |seg| seg.op2(|a: i128, b: i128| a <= b)),
    sig!(TYPE_ISIZE, 2, |seg| seg.op2(|a: isize, b: isize| a <= b)),
    sig!(TYPE_F32, 2, |seg| seg.op2(|a: f32, b: f32| a <= b)),
    sig!(TYPE_F64, 2, |seg| seg.op2(|a: f64, b: f64| a <= b)),
];

// Greater than signatures
static GREATER_THAN_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg| seg.op2(|a: u8, b: u8| a > b)),
    sig!(TYPE_U16, 2, |seg| seg.op2(|a: u16, b: u16| a > b)),
    sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a > b)),
    sig!(TYPE_U64, 2, |seg| seg.op2(|a: u64, b: u64| a > b)),
    sig!(TYPE_U128, 2, |seg| seg.op2(|a: u128, b: u128| a > b)),
    sig!(TYPE_USIZE, 2, |seg| seg.op2(|a: usize, b: usize| a > b)),
    sig!(TYPE_I8, 2, |seg| seg.op2(|a: i8, b: i8| a > b)),
    sig!(TYPE_I16, 2, |seg| seg.op2(|a: i16, b: i16| a > b)),
    sig!(TYPE_I32, 2, |seg| seg.op2(|a: i32, b: i32| a > b)),
    sig!(TYPE_I64, 2, |seg| seg.op2(|a: i64, b: i64| a > b)),
    sig!(TYPE_I128, 2, |seg| seg.op2(|a: i128, b: i128| a > b)),
    sig!(TYPE_ISIZE, 2, |seg| seg.op2(|a: isize, b: isize| a > b)),
    sig!(TYPE_F32, 2, |seg| seg.op2(|a: f32, b: f32| a > b)),
    sig!(TYPE_F64, 2, |seg| seg.op2(|a: f64, b: f64| a > b)),
];

// Greater than or equal signatures
static GREATER_THAN_OR_EQUAL_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg| seg.op2(|a: u8, b: u8| a >= b)),
    sig!(TYPE_U16, 2, |seg| seg.op2(|a: u16, b: u16| a >= b)),
    sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a >= b)),
    sig!(TYPE_U64, 2, |seg| seg.op2(|a: u64, b: u64| a >= b)),
    sig!(TYPE_U128, 2, |seg| seg.op2(|a: u128, b: u128| a >= b)),
    sig!(TYPE_USIZE, 2, |seg| seg.op2(|a: usize, b: usize| a >= b)),
    sig!(TYPE_I8, 2, |seg| seg.op2(|a: i8, b: i8| a >= b)),
    sig!(TYPE_I16, 2, |seg| seg.op2(|a: i16, b: i16| a >= b)),
    sig!(TYPE_I32, 2, |seg| seg.op2(|a: i32, b: i32| a >= b)),
    sig!(TYPE_I64, 2, |seg| seg.op2(|a: i64, b: i64| a >= b)),
    sig!(TYPE_I128, 2, |seg| seg.op2(|a: i128, b: i128| a >= b)),
    sig!(TYPE_ISIZE, 2, |seg| seg.op2(|a: isize, b: isize| a >= b)),
    sig!(TYPE_F32, 2, |seg| seg.op2(|a: f32, b: f32| a >= b)),
    sig!(TYPE_F64, 2, |seg| seg.op2(|a: f64, b: f64| a >= b)),
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
    "<<" => LEFT_SHIFT_SIGNATURES,
    ">>" => RIGHT_SHIFT_SIGNATURES,
    "&&" => LOGICAL_AND_SIGNATURES,
    "||" => LOGICAL_OR_SIGNATURES,
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
    fn lookup(&self, name: &str, types: &[TypeId], segment: &mut DynSegment) -> Result<bool> {
        if let Some(signatures) = BUILTINS.get(name) {
            // Linear search through signatures (typically very few per operation)
            for sig in *signatures {
                // Check if signature matches the provided types
                let matches = sig.arity as usize == types.len()
                    && types.iter().all(|t| *t == sig.type_id());

                if matches {
                    (sig.op_fn)(segment)?;
                    return Ok(true);
                }
            }
        }
        Ok(false) // Operation name not found or no matching signature
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
/// let types = segment.peek_types_vec(2);
/// lookup.lookup("+", &types, &mut segment).unwrap();
/// assert_eq!(segment.call0::<u32>().unwrap(), 30);
/// ```
pub struct OpLookup {
    scopes: Vec<ScopeFn>,
    builtin_scope: BuiltinScope,
}

impl OpLookup {
    /// Creates a new operation lookup with only built-in operations.
    pub fn new() -> Self {
        OpLookup {
            scopes: Vec::new(),
            builtin_scope: BuiltinScope,
        }
    }

    /// Pushes a new scope onto the stack.
    ///
    /// The scope function should return `Ok(true)` if it handled the operation,
    /// `Ok(false)` to pass to the next scope, or `Err` on error.
    pub fn push_scope(&mut self, scope: ScopeFn) {
        self.scopes.push(scope);
    }

    /// Pops the most recent scope from the stack.
    ///
    /// Returns the popped scope, or `None` if the stack is empty.
    pub fn pop_scope(&mut self) -> Option<ScopeFn> {
        self.scopes.pop()
    }

    /// Looks up and applies an operation.
    ///
    /// Searches scopes in LIFO order, then falls back to built-in operations.
    ///
    /// # Arguments
    ///
    /// * `name` - The operation name (e.g., `"+"`, `"-"`, or a custom identifier)
    /// * `types` - The TypeIds of the operands in order
    /// * `segment` - The DynSegment to apply the operation to
    ///
    /// # Errors
    ///
    /// Returns an error if no scope or built-in operation can handle the request.
    pub fn lookup(&self, name: &str, types: &[TypeId], segment: &mut DynSegment) -> Result<()> {
        // Try scopes in reverse order (most recent first)
        for scope in self.scopes.iter().rev() {
            if scope(name, types, segment)? {
                return Ok(());
            }
        }

        // Fall back to built-in scope
        if self.builtin_scope.lookup(name, types, segment)? {
            return Ok(());
        }

        Err(anyhow!(
            "Operation '{}' not found for types {:?}",
            name,
            types
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

    #[test]
    fn test_addition_u32() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(10u32);
        segment.just(20u32);
        let types = segment.peek_types_vec(2);
        lookup.lookup("+", &types, &mut segment)?;
        assert_eq!(segment.call0::<u32>()?, 30);
        Ok(())
    }

    #[test]
    fn test_subtraction_i32() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(50i32);
        segment.just(20i32);
        let types = segment.peek_types_vec(2);
        lookup.lookup("-", &types, &mut segment)?;
        assert_eq!(segment.call0::<i32>()?, 30);
        Ok(())
    }

    #[test]
    fn test_multiplication_f64() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(3.5f64);
        segment.just(2.0f64);
        let types = segment.peek_types_vec(2);
        lookup.lookup("*", &types, &mut segment)?;
        assert_eq!(segment.call0::<f64>()?, 7.0);
        Ok(())
    }

    #[test]
    fn test_comparison_less_than() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(10u32);
        segment.just(20u32);
        let types = segment.peek_types_vec(2);
        lookup.lookup("<", &types, &mut segment)?;
        assert_eq!(segment.call0::<bool>()?, true);
        Ok(())
    }

    #[test]
    fn test_logical_and() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(true);
        segment.just(false);
        let types = segment.peek_types_vec(2);
        lookup.lookup("&&", &types, &mut segment)?;
        assert_eq!(segment.call0::<bool>()?, false);
        Ok(())
    }

    #[test]
    fn test_bitwise_and() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(0b1010u32);
        segment.just(0b1100u32);
        let types = segment.peek_types_vec(2);
        lookup.lookup("&", &types, &mut segment)?;
        assert_eq!(segment.call0::<u32>()?, 0b1000);
        Ok(())
    }

    #[test]
    fn test_unary_negation() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(42i32);
        let types = segment.peek_types_vec(1);
        lookup.lookup("-", &types, &mut segment)?;
        assert_eq!(segment.call0::<i32>()?, -42);
        Ok(())
    }

    #[test]
    fn test_logical_not() -> Result<()> {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(true);
        let types = segment.peek_types_vec(1);
        lookup.lookup("!", &types, &mut segment)?;
        assert_eq!(segment.call0::<bool>()?, false);
        Ok(())
    }

    #[test]
    fn test_unregistered_operation() {
        let lookup = OpLookup::new();
        let mut segment = DynSegment::new::<()>();
        segment.just(10u32);
        segment.just(20u32);
        let types = segment.peek_types_vec(2);

        let result = lookup.lookup("unknown_op", &types, &mut segment);
        assert!(result.is_err());
    }

    #[test]
    fn test_custom_scope() -> Result<()> {
        let mut lookup = OpLookup::new();

        // Add a custom scope that handles "double"
        lookup.push_scope(Box::new(|name, types, segment| {
            if name == "double" && types.len() == 1 && types[0] == TypeId::of::<u32>() {
                segment.op1(|a: u32| a * 2)?;
                Ok(true)
            } else {
                Ok(false)
            }
        }));

        let mut segment = DynSegment::new::<()>();
        segment.just(21u32);
        let types = segment.peek_types_vec(1);
        lookup.lookup("double", &types, &mut segment)?;
        assert_eq!(segment.call0::<u32>()?, 42);

        Ok(())
    }

    #[test]
    fn test_scope_override() -> Result<()> {
        let mut lookup = OpLookup::new();

        // Override addition to always return 100
        lookup.push_scope(Box::new(|name, types, segment| {
            if name == "+" && types.len() == 2 && types[0] == TypeId::of::<u32>() {
                segment.op2(|_a: u32, _b: u32| 100u32)?;
                Ok(true)
            } else {
                Ok(false)
            }
        }));

        let mut segment = DynSegment::new::<()>();
        segment.just(10u32);
        segment.just(20u32);
        let types = segment.peek_types_vec(2);
        lookup.lookup("+", &types, &mut segment)?;
        assert_eq!(segment.call0::<u32>()?, 100);

        Ok(())
    }

    #[test]
    fn test_scope_pop() -> Result<()> {
        let mut lookup = OpLookup::new();

        // Add override
        lookup.push_scope(Box::new(|name, types, segment| {
            if name == "+" && types.len() == 2 && types[0] == TypeId::of::<u32>() {
                segment.op2(|_a: u32, _b: u32| 100u32)?;
                Ok(true)
            } else {
                Ok(false)
            }
        }));

        // Test with override
        let mut segment = DynSegment::new::<()>();
        segment.just(10u32);
        segment.just(20u32);
        let types = segment.peek_types_vec(2);
        lookup.lookup("+", &types, &mut segment)?;
        assert_eq!(segment.call0::<u32>()?, 100);

        // Pop scope and test normal behavior
        lookup.pop_scope();
        let mut segment = DynSegment::new::<()>();
        segment.just(10u32);
        segment.just(20u32);
        let types = segment.peek_types_vec(2);
        lookup.lookup("+", &types, &mut segment)?;
        assert_eq!(segment.call0::<u32>()?, 30);

        Ok(())
    }
}
