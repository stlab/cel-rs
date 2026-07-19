//! A minimal static type model for the built-in primitives `pm_lang::TypeRegistry::new()`
//! registers by default, plus [`Ty::Any`] for everything else (custom host-registered types,
//! unannotated cells, unresolved identifiers). Used by [`check_expr`] to type-check
//! [`crate::Expr`] trees built by [`crate::AstContext`]. Not a complete type system — see the
//! design doc's "Type checking (v1)" section for what's deliberately out of scope.

use std::any::TypeId;

use crate::Literal;

/// A static type: one of the built-in primitives, or [`Ty::Any`] for anything pm-lang/CEL's
/// extensible type system doesn't statically know about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    /// `i8`.
    I8,
    /// `i16`.
    I16,
    /// `i32`.
    I32,
    /// `i64`.
    I64,
    /// `i128`.
    I128,
    /// `isize`.
    Isize,
    /// `u8`.
    U8,
    /// `u16`.
    U16,
    /// `u32`.
    U32,
    /// `u64`.
    U64,
    /// `u128`.
    U128,
    /// `usize`.
    Usize,
    /// `f32`.
    F32,
    /// `f64`.
    F64,
    /// `bool`.
    Bool,
    /// `String`.
    String,
    /// Anything not statically known: a custom host-registered type, an unannotated cell, an
    /// unresolved identifier, or a node kind [`check_expr`] doesn't check directly (e.g. a tuple
    /// or call result). Unifies silently with every other `Ty`, in both directions.
    Any,
}

impl Ty {
    /// Maps a resolved [`Literal`] to its [`Ty`]. `Char`/`ByteStr`/`CStr`/`Unit` have no `Ty`
    /// variant and map to [`Ty::Any`] — not an error, matching this model's "unresolved falls
    /// back to `Any`" convention.
    pub fn from_literal(lit: &Literal) -> Ty {
        match lit {
            Literal::I8(_) => Ty::I8,
            Literal::I16(_) => Ty::I16,
            Literal::I32(_) => Ty::I32,
            Literal::I64(_) => Ty::I64,
            Literal::I128(_) => Ty::I128,
            Literal::Isize(_) => Ty::Isize,
            Literal::U8(_) => Ty::U8,
            Literal::U16(_) => Ty::U16,
            Literal::U32(_) => Ty::U32,
            Literal::U64(_) => Ty::U64,
            Literal::U128(_) => Ty::U128,
            Literal::Usize(_) => Ty::Usize,
            Literal::F32(_) => Ty::F32,
            Literal::F64(_) => Ty::F64,
            Literal::Bool(_) => Ty::Bool,
            Literal::Str(_) => Ty::String,
            Literal::Char(_) | Literal::ByteStr(_) | Literal::CStr(_) | Literal::Unit => Ty::Any,
        }
    }

    /// Maps a `TypeId` (e.g. from `pm_lang::TypeRegistry::TypeEntry::type_id`) to its [`Ty`].
    /// An unrecognized `TypeId` maps to [`Ty::Any`] — not an error, matching pm-lang/CEL's
    /// extensible type system (a host binary's custom registered types are invisible here).
    pub fn from_type_id(id: TypeId) -> Ty {
        if id == TypeId::of::<i8>() {
            Ty::I8
        } else if id == TypeId::of::<i16>() {
            Ty::I16
        } else if id == TypeId::of::<i32>() {
            Ty::I32
        } else if id == TypeId::of::<i64>() {
            Ty::I64
        } else if id == TypeId::of::<i128>() {
            Ty::I128
        } else if id == TypeId::of::<isize>() {
            Ty::Isize
        } else if id == TypeId::of::<u8>() {
            Ty::U8
        } else if id == TypeId::of::<u16>() {
            Ty::U16
        } else if id == TypeId::of::<u32>() {
            Ty::U32
        } else if id == TypeId::of::<u64>() {
            Ty::U64
        } else if id == TypeId::of::<u128>() {
            Ty::U128
        } else if id == TypeId::of::<usize>() {
            Ty::Usize
        } else if id == TypeId::of::<f32>() {
            Ty::F32
        } else if id == TypeId::of::<f64>() {
            Ty::F64
        } else if id == TypeId::of::<bool>() {
            Ty::Bool
        } else if id == TypeId::of::<String>() {
            Ty::String
        } else {
            Ty::Any
        }
    }

    /// Returns this type's `TypeId`, or `None` for [`Ty::Any`] (which has no single concrete
    /// Rust type).
    pub fn type_id(&self) -> Option<TypeId> {
        Some(match self {
            Ty::I8 => TypeId::of::<i8>(),
            Ty::I16 => TypeId::of::<i16>(),
            Ty::I32 => TypeId::of::<i32>(),
            Ty::I64 => TypeId::of::<i64>(),
            Ty::I128 => TypeId::of::<i128>(),
            Ty::Isize => TypeId::of::<isize>(),
            Ty::U8 => TypeId::of::<u8>(),
            Ty::U16 => TypeId::of::<u16>(),
            Ty::U32 => TypeId::of::<u32>(),
            Ty::U64 => TypeId::of::<u64>(),
            Ty::U128 => TypeId::of::<u128>(),
            Ty::Usize => TypeId::of::<usize>(),
            Ty::F32 => TypeId::of::<f32>(),
            Ty::F64 => TypeId::of::<f64>(),
            Ty::Bool => TypeId::of::<bool>(),
            Ty::String => TypeId::of::<String>(),
            Ty::Any => return None,
        })
    }

    /// A human-readable name for diagnostics (e.g. `"i32"`, `"<any>"`).
    pub fn name(&self) -> &'static str {
        match self {
            Ty::I8 => "i8",
            Ty::I16 => "i16",
            Ty::I32 => "i32",
            Ty::I64 => "i64",
            Ty::I128 => "i128",
            Ty::Isize => "isize",
            Ty::U8 => "u8",
            Ty::U16 => "u16",
            Ty::U32 => "u32",
            Ty::U64 => "u64",
            Ty::U128 => "u128",
            Ty::Usize => "usize",
            Ty::F32 => "f32",
            Ty::F64 => "f64",
            Ty::Bool => "bool",
            Ty::String => "String",
            Ty::Any => "<any>",
        }
    }

    /// Returns `true` if `self` and `other` are compatible: either is [`Ty::Any`], or they're
    /// equal. `Ty::Any` unifying silently with everything (in both directions) is the load-bearing
    /// property that lets unannotated cells and custom host types produce zero false-positive
    /// diagnostics.
    pub fn unifies_with(&self, other: &Ty) -> bool {
        matches!(self, Ty::Any) || matches!(other, Ty::Any) || self == other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_literal_maps_every_concrete_variant() {
        assert_eq!(Ty::from_literal(&Literal::I8(1)), Ty::I8);
        assert_eq!(Ty::from_literal(&Literal::I16(1)), Ty::I16);
        assert_eq!(Ty::from_literal(&Literal::I32(1)), Ty::I32);
        assert_eq!(Ty::from_literal(&Literal::I64(1)), Ty::I64);
        assert_eq!(Ty::from_literal(&Literal::I128(1)), Ty::I128);
        assert_eq!(Ty::from_literal(&Literal::Isize(1)), Ty::Isize);
        assert_eq!(Ty::from_literal(&Literal::U8(1)), Ty::U8);
        assert_eq!(Ty::from_literal(&Literal::U16(1)), Ty::U16);
        assert_eq!(Ty::from_literal(&Literal::U32(1)), Ty::U32);
        assert_eq!(Ty::from_literal(&Literal::U64(1)), Ty::U64);
        assert_eq!(Ty::from_literal(&Literal::U128(1)), Ty::U128);
        assert_eq!(Ty::from_literal(&Literal::Usize(1)), Ty::Usize);
        assert_eq!(Ty::from_literal(&Literal::F32(1.0)), Ty::F32);
        assert_eq!(Ty::from_literal(&Literal::F64(1.0)), Ty::F64);
        assert_eq!(Ty::from_literal(&Literal::Bool(true)), Ty::Bool);
        assert_eq!(Ty::from_literal(&Literal::Str("s".to_string())), Ty::String);
    }

    #[test]
    fn from_literal_maps_unsupported_kinds_to_any() {
        assert_eq!(Ty::from_literal(&Literal::Char('a')), Ty::Any);
        assert_eq!(Ty::from_literal(&Literal::ByteStr(vec![1])), Ty::Any);
        assert_eq!(Ty::from_literal(&Literal::Unit), Ty::Any);
    }

    #[test]
    fn type_id_round_trips_through_from_type_id_for_every_concrete_variant() {
        for ty in [
            Ty::I8,
            Ty::I16,
            Ty::I32,
            Ty::I64,
            Ty::I128,
            Ty::Isize,
            Ty::U8,
            Ty::U16,
            Ty::U32,
            Ty::U64,
            Ty::U128,
            Ty::Usize,
            Ty::F32,
            Ty::F64,
            Ty::Bool,
            Ty::String,
        ] {
            let id = ty.type_id().expect("concrete Ty has a TypeId");
            assert_eq!(Ty::from_type_id(id), ty);
        }
    }

    #[test]
    fn any_has_no_type_id() {
        assert_eq!(Ty::Any.type_id(), None);
    }

    #[test]
    fn from_type_id_maps_an_unregistered_type_id_to_any() {
        assert_eq!(Ty::from_type_id(TypeId::of::<Vec<u8>>()), Ty::Any);
    }

    #[test]
    fn any_unifies_with_every_concrete_type_in_both_directions() {
        assert!(Ty::Any.unifies_with(&Ty::I32));
        assert!(Ty::I32.unifies_with(&Ty::Any));
        assert!(Ty::Any.unifies_with(&Ty::Any));
    }

    #[test]
    fn identical_concrete_types_unify() {
        assert!(Ty::F64.unifies_with(&Ty::F64));
    }

    #[test]
    fn distinct_concrete_types_do_not_unify() {
        assert!(!Ty::I32.unifies_with(&Ty::F64));
        assert!(!Ty::I32.unifies_with(&Ty::Bool));
    }

    #[test]
    fn name_is_distinct_per_type() {
        let names: Vec<&str> = [Ty::I32, Ty::F64, Ty::Bool, Ty::String, Ty::Any]
            .iter()
            .map(Ty::name)
            .collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            names.len(),
            unique.len(),
            "every listed Ty has a distinct name"
        );
    }
}
