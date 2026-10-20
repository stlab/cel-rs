//! Checks source expressions using complete recursive array, tuple, and callable shapes.
//!
//! Use [`check_expr`] with a free-identifier resolver for builtin-only expressions, or
//! [`check_expr_with_type_resolver`] with a host registry and operation metadata.
//! [`Ty::Any`] retains incomplete editor expressions; it never authorizes executable storage.

use std::any::TypeId;
use std::borrow::Cow;

use crate::op_table::{builtin_operand_types, cast_source_types};
use crate::type_expr::{BuiltinTypeResolver, ResolvedType, TypeExpr, TypeResolver};
use crate::{Expr, ExprSpan, Literal, OpLookup, ParseError, ResolvedLeafType};

/// A complete recursive source type, or an unresolved editor value.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// A homogeneous array (`cel_runtime::DynamicArray`) of the boxed element type, which may
    /// itself be an array (`[[i32]]`) or [`Ty::Any`] (an array whose element type isn't
    /// statically known — e.g. `[f(), g()]`, whose elements are unresolved call results).
    Array(Box<Ty>),
    /// A source-ordered tuple, including unit and singleton tuples.
    Tuple(Vec<Ty>),
    /// A callable with complete parameter and result types.
    Callable(Vec<Ty>, Box<Ty>),
    /// A known host leaf, retaining its concrete identity and complete runtime descriptor.
    Registered(ResolvedLeafType),
    /// Anything not statically known: an unannotated cell, an
    /// unresolved identifier, or a call without available metadata.
    /// Unifies silently with every other `Ty`, in both directions.
    Any,
}

impl Ty {
    /// Preserves a resolved leaf descriptor, using primitive and recursive array variants.
    ///
    /// - Complexity: O(d + l) in descriptor depth d and leaf display-name length l.
    ///
    /// # Examples
    /// ```
    /// use cel_parser::{ResolvedLeafType, Ty};
    /// use cel_runtime::ArrayElementType;
    /// let leaf = ResolvedLeafType::new("custom", ArrayElementType::leaf::<()>().unwrap());
    /// assert_eq!(Ty::from_leaf(&leaf), Ty::Registered(leaf));
    /// ```
    pub fn from_leaf(leaf: &ResolvedLeafType) -> Self {
        if let Some(element) = leaf.element_type().nested() {
            return Ty::Array(Box::new(descriptor_ty(element)));
        }
        match Self::from_type_id(leaf.type_id()) {
            Ty::Any | Ty::Array(_) => Ty::Registered(leaf.clone()),
            primitive => primitive,
        }
    }

    /// Maps a resolved [`Literal`] to its exact primitive or registered leaf identity.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::{Literal, Ty};
    ///
    /// assert_eq!(Ty::from_literal(&Literal::I32(1)), Ty::I32);
    /// assert_eq!(Ty::from_literal(&Literal::Unit).type_id(), Some(std::any::TypeId::of::<()>()));
    /// ```
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
            Literal::Char(_) => Self::known_leaf::<char>("char"),
            Literal::ByteStr(_) => Self::known_leaf::<Vec<u8>>("byte string"),
            Literal::CStr(_) => Self::known_leaf::<std::ffi::CString>("C string"),
            Literal::Unit => Ty::Tuple(vec![]),
        }
    }

    /// Describes a statically known non-array leaf.
    ///
    /// - Precondition: T is not `DynamicArray`, whose descriptor requires element metadata.
    fn known_leaf<T: 'static>(name: &'static str) -> Ty {
        debug_assert_ne!(TypeId::of::<T>(), TypeId::of::<cel_runtime::DynamicArray>());
        Ty::Registered(ResolvedLeafType::new(
            name,
            cel_runtime::ArrayElementType::leaf::<T>().expect("non-array leaf"),
        ))
    }

    /// Returns whether this type contains an unresolved leaf.
    ///
    /// - Complexity: O(n) in recursive type nodes.
    fn contains_any(&self) -> bool {
        match self {
            Ty::Any => true,
            Ty::Array(element) => element.contains_any(),
            Ty::Tuple(elements) => elements.iter().any(Self::contains_any),
            Ty::Callable(parameters, result) => {
                parameters.iter().any(Self::contains_any) || result.contains_any()
            }
            _ => false,
        }
    }

    /// Maps a `TypeId` (e.g. from `adam_lang::TypeRegistry::TypeEntry::type_id`) to its [`Ty`].
    /// An unrecognized `TypeId` maps to [`Ty::Any`] — not an error, matching adam-lang/CEL's
    /// extensible type system (a host binary's custom registered types are invisible here).
    /// `cel_runtime::DynamicArray`'s own `TypeId` maps to `Array(Box::new(Ty::Any))`: a bare
    /// `TypeId` is flat, so it carries no element descriptor to recover the array's element type
    /// from, and [`Ty::Any`] is the element type that unifies with every actual element type
    /// rather than claiming one falsely.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::any::TypeId;
    /// use cel_parser::Ty;
    ///
    /// assert_eq!(Ty::from_type_id(TypeId::of::<i32>()), Ty::I32);
    /// assert_eq!(Ty::from_type_id(TypeId::of::<Vec<u8>>()), Ty::Any);
    /// assert_eq!(
    ///     Ty::from_type_id(TypeId::of::<cel_runtime::DynamicArray>()),
    ///     Ty::Array(Box::new(Ty::Any))
    /// );
    /// ```
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
        } else if id == TypeId::of::<cel_runtime::DynamicArray>() {
            Ty::Array(Box::new(Ty::Any))
        } else {
            Ty::Any
        }
    }

    /// Returns this type's `TypeId`, or `None` for [`Ty::Any`] and nonempty tuples (whose
    /// storage needs a complete shape). Every [`Ty::Array`], whatever its element type, reports
    /// `cel_runtime::DynamicArray`'s `TypeId` — the one runtime type every array value has.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::any::TypeId;
    /// use cel_parser::Ty;
    ///
    /// assert_eq!(Ty::I32.type_id(), Some(TypeId::of::<i32>()));
    /// assert_eq!(Ty::Any.type_id(), None);
    /// assert_eq!(
    ///     Ty::Array(Box::new(Ty::I32)).type_id(),
    ///     Some(TypeId::of::<cel_runtime::DynamicArray>())
    /// );
    /// ```
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
            Ty::Array(_) => TypeId::of::<cel_runtime::DynamicArray>(),
            Ty::Registered(leaf) => leaf.type_id(),
            Ty::Tuple(elements) if elements.is_empty() => TypeId::of::<()>(),
            Ty::Tuple(_) => return None,
            Ty::Callable(..) => TypeId::of::<cel_runtime::DynClosure>(),
            Ty::Any => return None,
        })
    }

    /// Returns a human-readable name for diagnostics, borrowing primitive and leaf names.
    ///
    /// - Complexity: O(n * l) time and O(l) space for n recursive nodes and l output bytes;
    ///   primitive and registered leaf names take O(1).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::Ty;
    ///
    /// assert_eq!(Ty::I32.name(), "i32");
    /// assert_eq!(Ty::Any.name(), "<any>");
    /// assert_eq!(Ty::Array(Box::new(Ty::Array(Box::new(Ty::I32)))).name(), "[[i32]]");
    /// ```
    pub fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(match self {
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
            Ty::Array(element) => return Cow::Owned(format!("[{}]", element.name())),
            Ty::Tuple(elements) => {
                return Cow::Owned(format!(
                    "({}{})",
                    elements
                        .iter()
                        .map(|ty| ty.name())
                        .collect::<Vec<_>>()
                        .join(", "),
                    if elements.len() == 1 { "," } else { "" }
                ));
            }
            Ty::Callable(parameters, result) => {
                return Cow::Owned(format!(
                    "Fn(({}{}), {})",
                    parameters
                        .iter()
                        .map(|ty| ty.name())
                        .collect::<Vec<_>>()
                        .join(", "),
                    if parameters.len() == 1 { "," } else { "" },
                    result.name()
                ));
            }
            Ty::Registered(leaf) => leaf.type_name(),
            Ty::Any => "<any>",
        })
    }

    /// Maps a type's bare name (as written in source, e.g. in a cast's target position) to its
    /// [`Ty`], or `None` if `name` isn't a recognized primitive - the inverse of [`Ty::name`],
    /// except it has no input that maps to [`Ty::Any`] (nothing is spelled `<any>` in source).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::Ty;
    ///
    /// assert_eq!(Ty::from_name("i32"), Some(Ty::I32));
    /// assert_eq!(Ty::from_name("nonsense"), None);
    /// ```
    pub fn from_name(name: &str) -> Option<Ty> {
        match name {
            "i8" => Some(Ty::I8),
            "i16" => Some(Ty::I16),
            "i32" => Some(Ty::I32),
            "i64" => Some(Ty::I64),
            "i128" => Some(Ty::I128),
            "isize" => Some(Ty::Isize),
            "u8" => Some(Ty::U8),
            "u16" => Some(Ty::U16),
            "u32" => Some(Ty::U32),
            "u64" => Some(Ty::U64),
            "u128" => Some(Ty::U128),
            "usize" => Some(Ty::Usize),
            "f32" => Some(Ty::F32),
            "f64" => Some(Ty::F64),
            "bool" => Some(Ty::Bool),
            "String" => Some(Ty::String),
            _ => None,
        }
    }

    /// Returns `true` if `self` and `other` have compatible recursive shapes.
    /// `Ty::Any` unifying silently with everything (in both directions and at every nesting level) is the
    /// load-bearing property that lets unresolved inputs produce zero false-positive diagnostics.
    /// Registered leaves unify only when their complete descriptors agree, regardless of aliases.
    ///
    /// - Complexity: O(n + l) time and space for n recursive nodes and l registered name bytes.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::Ty;
    ///
    /// assert!(Ty::Any.unifies_with(&Ty::I32));
    /// assert!(!Ty::I32.unifies_with(&Ty::F64));
    /// assert!(Ty::Array(Box::new(Ty::Any)).unifies_with(&Ty::Array(Box::new(Ty::I32))));
    /// assert!(!Ty::Array(Box::new(Ty::I32)).unifies_with(&Ty::Array(Box::new(Ty::F64))));
    /// ```
    pub fn unifies_with(&self, other: &Ty) -> bool {
        self.unify(other).is_some()
    }

    /// The most specific type both `self` and `other` describe, or `None` if they're
    /// incompatible: [`Ty::Any`] yields the other side (it constrains nothing); arrays,
    /// tuples, and callables unify recursively with equal arity; equal leaves yield themselves.
    ///
    /// - Postcondition: `self.unify(other).is_some() == self.unifies_with(other)`, and the result
    ///   (when `Some`) unifies with both `self` and `other`.
    /// - Complexity: O(n + l) time and space for n recursive nodes and l registered name bytes.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::Ty;
    ///
    /// assert_eq!(Ty::Any.unify(&Ty::I32), Some(Ty::I32));
    /// assert_eq!(
    ///     Ty::Array(Box::new(Ty::Any)).unify(&Ty::Array(Box::new(Ty::I32))),
    ///     Some(Ty::Array(Box::new(Ty::I32)))
    /// );
    /// assert_eq!(Ty::I32.unify(&Ty::F64), None);
    /// ```
    pub fn unify(&self, other: &Ty) -> Option<Ty> {
        match (self, other) {
            (Ty::Any, other) => Some(other.clone()),
            (this, Ty::Any) => Some(this.clone()),
            (Ty::Array(lhs), Ty::Array(rhs)) => Some(Ty::Array(Box::new(lhs.unify(rhs)?))),
            (Ty::Tuple(lhs), Ty::Tuple(rhs)) if lhs.len() == rhs.len() => Some(Ty::Tuple(
                lhs.iter()
                    .zip(rhs)
                    .map(|(a, b)| a.unify(b))
                    .collect::<Option<_>>()?,
            )),
            (Ty::Callable(lhs, lr), Ty::Callable(rhs, rr)) if lhs.len() == rhs.len() => {
                Some(Ty::Callable(
                    lhs.iter()
                        .zip(rhs)
                        .map(|(a, b)| a.unify(b))
                        .collect::<Option<_>>()?,
                    Box::new(lr.unify(rr)?),
                ))
            }
            (Ty::Registered(lhs), Ty::Registered(rhs))
                if lhs.element_type() == rhs.element_type() =>
            {
                Some(self.clone())
            }
            (this, other) if this == other => Some(this.clone()),
            _ => None,
        }
    }
}

/// Preserves an array element descriptor without repeatedly cloning nested descriptors.
///
/// - Complexity: O(d + l) in descriptor depth d and leaf display-name length l.
fn descriptor_ty(descriptor: &cel_runtime::ArrayElementType) -> Ty {
    if let Some(element) = descriptor.nested() {
        return Ty::Array(Box::new(descriptor_ty(element)));
    }
    match Ty::from_type_id(descriptor.type_id()) {
        Ty::Any | Ty::Array(_) => Ty::Registered(ResolvedLeafType::new(
            descriptor.type_name().to_owned(),
            descriptor.clone(),
        )),
        primitive => primitive,
    }
}

/// Infers `expr`'s type against `resolve_ident` (looks up a free identifier's declared type, or
/// `Ty::Any` if unknown — e.g. a bare CEL builtin name, or an adam-lang cell with no `: type`
/// annotation), returning the expression's inferred type plus every type diagnostic found.
///
/// Uses an empty [`OpLookup`]; metadata-aware hosts use [`check_expr_with_type_resolver`].
/// Checks every child, including closure bodies under parameter-local bindings.
/// Arrays require homogeneous complete shapes, tuples retain source order, and `if` joins
/// compatible branch types. Callable applications check parameter and result types.
/// Calls without metadata and free identifiers without declarations remain unresolved.
///
/// - Complexity: O(n) in the number of nodes in `expr`.
///
/// # Examples
///
/// ```rust
/// use cel_parser::{Expr, ExprSpan, Literal, Ty};
/// use cel_parser::ty::check_expr;
///
/// let span = ExprSpan {
///     start: proc_macro2::Span::call_site(),
///     end: proc_macro2::Span::call_site(),
/// };
/// let expr = Expr::Literal {
///     value: Literal::I32(1),
///     span,
/// };
/// let (ty, diagnostics) = check_expr(&expr, &|_name| Ty::Any);
/// assert_eq!(ty, Ty::I32);
/// assert!(diagnostics.is_empty());
/// ```
pub fn check_expr(expr: &Expr, resolve_ident: &dyn Fn(&str) -> Ty) -> (Ty, Vec<ParseError>) {
    check_expr_with_type_resolver(expr, resolve_ident, &BuiltinTypeResolver, &OpLookup::new())
}

/// Infers `expr`'s type using value/type resolvers and registered operation/record metadata.
///
/// Resolves constructors through [`OpLookup::record_schema`] and members through
/// [`OpLookup::operation_signatures`], never through same-named values. Checks every child even
/// when a constructor is unknown or an operand is unresolved. Known fields and operands match
/// exactly, without numeric coercion. Invalid constructors/operations infer as [`Ty::Any`] to
/// avoid cascading errors. Custom calls without signatures remain best-effort.
///
/// - Complexity: O(n + Σ(f² + s · a · d)), for n expression nodes, f schema fields per
///   constructor, s overloads and a operands per operation, and descriptor nesting depth d,
///   plus resolver and type-unification costs.
///
/// # Examples
///
/// ```rust
/// use cel_parser::{AstContext, OpLookup, Parser, Ty};
/// use cel_parser::ty::check_expr_with_type_resolver;
///
/// let mut parser = Parser::<AstContext>::new(OpLookup::new());
/// let expr = parser.parse_str_ast("[]: [i32]").unwrap();
/// let (ty, diagnostics) =
///     check_expr_with_type_resolver(&expr, &|_name| Ty::Any, &[(
///         "i32",
///         cel_parser::ResolvedLeafType::new(
///             "i32",
///             cel_runtime::ArrayElementType::leaf::<i32>().unwrap(),
///         ),
///     )], &OpLookup::new());
/// assert_eq!(ty, Ty::Array(Box::new(Ty::I32)));
/// assert!(diagnostics.is_empty());
/// ```
pub fn check_expr_with_type_resolver(
    expr: &Expr,
    resolve_ident: &dyn Fn(&str) -> Ty,
    resolve_type: &dyn TypeResolver,
    lookup: &OpLookup,
) -> (Ty, Vec<ParseError>) {
    match expr {
        Expr::Literal { value, .. } => (Ty::from_literal(value), Vec::new()),
        Expr::Ident { name, .. } => {
            let ty = resolve_ident(name);
            let ty = if ty == Ty::Any && !lookup.is_registered_function(name) {
                matching_registered_type(name, &[], lookup).unwrap_or(ty)
            } else {
                ty
            };
            (ty, Vec::new())
        }
        Expr::Struct {
            name,
            name_span,
            fields,
            ..
        } => {
            let mut diagnostics = Vec::new();
            let schema = lookup.record_schema(name);
            let mut seen = std::collections::HashSet::new();
            for field in fields {
                let (actual, child_diags) = check_expr_with_type_resolver(
                    &field.value,
                    resolve_ident,
                    resolve_type,
                    lookup,
                );
                let valid = child_diags.is_empty();
                diagnostics.extend(child_diags);
                let Some(schema) = schema else { continue };
                let error = if !seen.insert(field.name.as_str()) {
                    Some(format!("duplicate field `{}`", field.name))
                } else if let Some(expected) = schema.fields.iter().find(|f| f.name == field.name) {
                    let expected = resolved_type_to_element_ty(&expected.ty);
                    (valid && !expected.unifies_with(&actual)).then(|| {
                        format!(
                            "field `{}` expects `{}`, found `{}`",
                            field.name,
                            expected.name(),
                            actual.name(),
                        )
                    })
                } else {
                    Some(format!("unknown field `{}` for `{name}`", field.name))
                };
                if let Some(error) = error {
                    diagnostics.push(ParseError::new_range(
                        error,
                        field.name_span.start,
                        field.value.span().end,
                    ));
                }
            }
            if let Some(schema) = schema {
                for field in &schema.fields {
                    if !seen.contains(field.name) {
                        diagnostics.push(ParseError::new_range(
                            format!("missing field `{}` for `{name}`", field.name),
                            name_span.start,
                            name_span.end,
                        ));
                    }
                }
                let ty = if diagnostics.is_empty() {
                    Ty::from_leaf(&schema.ty)
                } else {
                    Ty::Any
                };
                (ty, diagnostics)
            } else {
                diagnostics.push(ParseError::new_range(
                    format!("unknown record type `{name}`"),
                    name_span.start,
                    name_span.end,
                ));
                (Ty::Any, diagnostics)
            }
        }
        Expr::Op {
            name,
            operands,
            span,
        } => check_op(name, operands, *span, resolve_ident, resolve_type, lookup),
        Expr::Logical { lhs, rhs, span, .. } => {
            check_logical(lhs, rhs, *span, resolve_ident, resolve_type, lookup)
        }
        Expr::Apply { callee, args, span } => {
            let (callee_ty, mut diagnostics) =
                check_expr_with_type_resolver(callee, resolve_ident, resolve_type, lookup);
            let mut types = Vec::with_capacity(args.len());
            for arg in args {
                let (ty, errors) =
                    check_expr_with_type_resolver(arg, resolve_ident, resolve_type, lookup);
                types.push(ty);
                diagnostics.extend(errors);
            }
            if let Ty::Callable(parameters, result) = callee_ty {
                if parameters.len() == types.len()
                    && parameters
                        .iter()
                        .zip(&types)
                        .all(|(a, b)| a.unifies_with(b))
                {
                    return (*result, diagnostics);
                }
                diagnostics.push(ParseError::new_range(
                    "callable argument type mismatch",
                    span.start,
                    span.end,
                ));
            } else {
                if matches!(callee_ty, Ty::Any | Ty::Registered(_))
                    && let Expr::Ident { name, .. } = callee.as_ref()
                    && let Some(ty) =
                        registered_operation_ty(name, &types, *span, lookup, &mut diagnostics)
                {
                    return (ty, diagnostics);
                }
                if !matches!(callee_ty, Ty::Any | Ty::Registered(_)) {
                    diagnostics.push(ParseError::new_range(
                        "value is not callable",
                        span.start,
                        span.end,
                    ));
                }
            }
            (Ty::Any, diagnostics)
        }
        Expr::Member {
            receiver,
            name,
            args,
            span,
            ..
        } => {
            let mut diagnostics = Vec::new();
            let operands = std::iter::once(receiver.as_ref()).chain(args.iter().flatten());
            let types: Vec<_> = operands
                .map(|expr| {
                    let (ty, child_diags) =
                        check_expr_with_type_resolver(expr, resolve_ident, resolve_type, lookup);
                    let valid = child_diags.is_empty();
                    diagnostics.extend(child_diags);
                    if valid { ty } else { Ty::Any }
                })
                .collect();
            let ty = registered_operation_ty(name, &types, *span, lookup, &mut diagnostics)
                .unwrap_or(Ty::Any);
            (ty, diagnostics)
        }
        Expr::Tuple { elements, .. } => {
            let mut diagnostics = Vec::new();
            let types = elements
                .iter()
                .map(|element| {
                    let (ty, errors) =
                        check_expr_with_type_resolver(element, resolve_ident, resolve_type, lookup);
                    diagnostics.extend(errors);
                    ty
                })
                .collect();
            (Ty::Tuple(types), diagnostics)
        }
        // An array literal's own type is `Array` of its elements' unified type; see `check_array`.
        Expr::Array {
            elements,
            type_annotation,
            annotation_span,
            ..
        } => check_array(
            elements,
            type_annotation.as_ref(),
            *annotation_span,
            resolve_ident,
            resolve_type,
            lookup,
        ),
        Expr::TupleIndex { base, index, span } => {
            let (ty, mut diagnostics) =
                check_expr_with_type_resolver(base, resolve_ident, resolve_type, lookup);
            if let Ty::Tuple(elements) = ty {
                if let Some(element) = elements.get(*index) {
                    return (element.clone(), diagnostics);
                }
                diagnostics.push(ParseError::new_range(
                    "tuple index out of bounds",
                    span.start,
                    span.end,
                ));
            } else if ty != Ty::Any {
                diagnostics.push(ParseError::new_range(
                    "tuple index requires a tuple",
                    span.start,
                    span.end,
                ));
            }
            (Ty::Any, diagnostics)
        }
        Expr::Cast {
            expr,
            type_name,
            span,
        } => check_cast(expr, type_name, *span, resolve_ident, resolve_type, lookup),
        Expr::If {
            cond,
            then_branch,
            else_branch,
            span,
        } => {
            let (condition, mut diagnostics) =
                check_expr_with_type_resolver(cond, resolve_ident, resolve_type, lookup);
            if !condition.unifies_with(&Ty::Bool) {
                diagnostics.push(ParseError::new_range(
                    "if condition must be bool",
                    cond.span().start,
                    cond.span().end,
                ));
            }
            let (then_ty, errors) =
                check_expr_with_type_resolver(then_branch, resolve_ident, resolve_type, lookup);
            diagnostics.extend(errors);
            let (else_ty, errors) = else_branch
                .as_ref()
                .map(|branch| {
                    check_expr_with_type_resolver(branch, resolve_ident, resolve_type, lookup)
                })
                .unwrap_or((Ty::Tuple(vec![]), vec![]));
            diagnostics.extend(errors);
            let ty = then_ty.unify(&else_ty).unwrap_or_else(|| {
                diagnostics.push(ParseError::new_range(
                    "if branch type mismatch",
                    span.start,
                    span.end,
                ));
                Ty::Any
            });
            (ty, diagnostics)
        }
        Expr::Closure { params, body, .. } => {
            let mut annotation_errors = Vec::new();
            let parameters: Vec<_> = params
                .iter()
                .map(|p| match p.type_expr.resolve(resolve_type) {
                    Ok(resolved) => resolved_type_to_element_ty(&resolved),
                    Err(error) => {
                        annotation_errors.push(error);
                        Ty::Any
                    }
                })
                .collect();
            let resolve_with_params = |name: &str| -> Ty {
                if let Some(index) = params.iter().position(|p| p.name == name) {
                    parameters[index].clone()
                } else {
                    resolve_ident(name)
                }
            };
            let (result, mut diagnostics) =
                check_expr_with_type_resolver(body, &resolve_with_params, resolve_type, lookup);
            diagnostics.extend(annotation_errors);
            (Ty::Callable(parameters, Box::new(result)), diagnostics)
        }
    }
}

/// Checks an [`Expr::Array`] node: infers every element (retaining all of their own nested
/// diagnostics, whatever the fold below finds), then folds the element types left to right with
/// [`Ty::unify`] — so [`Ty::Any`] elements (an unresolved identifier, a call result) adopt the
/// other elements' concrete type instead of defeating inference, and nested arrays unify
/// recursively. The first element that doesn't unify with the accumulated type gets one
/// diagnostic, reported at that element's own span, and folding stops there: a heterogeneous
/// literal yields exactly one conflict diagnostic, naming the first conflicting element, matching
/// `cel_runtime::DynamicArray`'s own runtime "first conflicting index" report. Later elements are
/// still fully checked, so their own internal diagnostics still surface.
///
/// A literal whose elements conflict infers as `Array(Any)` rather than `Any`: the node is
/// certainly an array, just one whose element type isn't known, and `Any` inside keeps the
/// mismatch from cascading into further diagnostics at enclosing nodes. An element-less
/// `Expr::Array` (which the grammar rejects, see <https://github.com/stlab/cel-rs/issues/212>,
/// but which can still be built directly) infers as `Array(Any)` with no diagnostic.
///
/// - Postcondition: the returned type is always a [`Ty::Array`].
/// - Complexity: O(n · d) where n is the number of elements and d is their array nesting depth,
///   plus the cost of checking each element (see [`check_expr`]'s own complexity note).
fn check_array(
    elements: &[Expr],
    type_annotation: Option<&TypeExpr>,
    annotation_span: Option<ExprSpan>,
    resolve_ident: &dyn Fn(&str) -> Ty,
    resolve_type: &dyn TypeResolver,
    lookup: &OpLookup,
) -> (Ty, Vec<ParseError>) {
    let mut diagnostics = Vec::new();
    let element_tys: Vec<Ty> = elements
        .iter()
        .map(|element| {
            let (ty, element_diags) =
                check_expr_with_type_resolver(element, resolve_ident, resolve_type, lookup);
            diagnostics.extend(element_diags);
            ty
        })
        .collect();
    let Some(type_annotation) = type_annotation else {
        let mut unified = Ty::Any;
        for (element, ty) in elements.iter().zip(&element_tys) {
            match unified.unify(ty) {
                Some(merged) => unified = merged,
                None => {
                    diagnostics.push(ParseError::new_range(
                        format!(
                            "array elements must share one type: expected `{}`, found `{}`",
                            unified.name(),
                            ty.name()
                        ),
                        element.span().start,
                        element.span().end,
                    ));
                    return (Ty::Array(Box::new(Ty::Any)), diagnostics);
                }
            }
        }
        return (Ty::Array(Box::new(unified)), diagnostics);
    };

    let (resolved_annotation, declared_ty, mut annotation_diags) =
        check_array_annotation(type_annotation, annotation_span, resolve_type);
    diagnostics.append(&mut annotation_diags);
    let (Some(ResolvedType::Array { element: _ }), Ty::Array(declared_element_ty)) =
        (&resolved_annotation, &declared_ty)
    else {
        return (Ty::Array(Box::new(Ty::Any)), diagnostics);
    };

    for (element, actual_ty) in elements.iter().zip(&element_tys) {
        if !declared_element_ty.unifies_with(actual_ty) {
            diagnostics.push(ParseError::new_range(
                format!(
                    "array elements must match the annotation exactly: expected `{}`, found `{}`",
                    declared_element_ty.name(),
                    actual_ty.name()
                ),
                element.span().start,
                element.span().end,
            ));
            break;
        }
    }
    (declared_ty, diagnostics)
}

/// Resolves one array annotation to both its recursive descriptor tree and its best-effort `Ty`.
///
/// # Errors
///
/// Returns one diagnostic when the annotation names an unknown type, does not name a complete
/// array type.
///
/// - Complexity: O(n) in the annotation tree plus resolver and descriptor-copying costs.
fn check_array_annotation(
    type_annotation: &TypeExpr,
    annotation_span: Option<ExprSpan>,
    resolve_type: &dyn TypeResolver,
) -> (Option<ResolvedType>, Ty, Vec<ParseError>) {
    let resolved = match type_annotation.resolve(resolve_type) {
        Ok(resolved) => resolved,
        Err(err) => return (None, Ty::Array(Box::new(Ty::Any)), vec![err]),
    };
    match resolved_type_to_array_ty(
        &resolved,
        annotation_span.unwrap_or_else(|| type_annotation.span()),
    ) {
        Ok(array_ty) => (Some(resolved), array_ty, Vec::new()),
        Err(err) => (Some(resolved), Ty::Array(Box::new(Ty::Any)), vec![err]),
    }
}

/// Converts a resolved array annotation, rejecting non-array types.
///
/// - Complexity: O(d) in descriptor nesting depth.
///
/// # Errors
/// Returns an error for non-array annotations.
fn resolved_type_to_array_ty(resolved: &ResolvedType, span: ExprSpan) -> crate::Result<Ty> {
    match resolved {
        ResolvedType::Array { element } => {
            Ok(Ty::Array(Box::new(resolved_type_to_element_ty(element))))
        }
        other => Err(ParseError::new_range(
            format!(
                "array annotations must name a complete array type, found `{}`",
                other.display_name()
            ),
            span.start,
            span.end,
        )),
    }
}

/// Preserves every complete resolved shape in the static model.
///
/// - Complexity: O(n + l) in recursive descriptor nodes n and total leaf-name length l.
///
fn resolved_type_to_element_ty(resolved: &ResolvedType) -> Ty {
    match resolved {
        ResolvedType::Scalar(leaf) => Ty::from_leaf(leaf),
        ResolvedType::Array { element } => {
            Ty::Array(Box::new(resolved_type_to_element_ty(element)))
        }
        ResolvedType::Tuple { elements } => {
            Ty::Tuple(elements.iter().map(resolved_type_to_element_ty).collect())
        }
        ResolvedType::Callable { parameters, result } => Ty::Callable(
            parameters.iter().map(resolved_type_to_element_ty).collect(),
            Box::new(resolved_type_to_element_ty(result)),
        ),
    }
}

/// Checks an [`Expr::Op`] node by matching known operands against registered signatures
/// before [`builtin_operand_types`]. Both sources accept unresolved operands only when
/// a signature accepts all known operands; such applications retain an unresolved result.
/// Known matches preserve their exact result and registered signatures take precedence.
/// Invalid operand expressions retain their diagnostics without a cascading operator mismatch.
/// An operator neither metadata source recognizes can't be checked and infers as `Ty::Any`.
///
/// - Complexity: O(s · a · d) for s registered and built-in signatures, a operands and
///   descriptor depth d, plus the cost of checking each operand (see [`check_expr`]).
fn check_op(
    name: &str,
    operands: &[Expr],
    span: ExprSpan,
    resolve_ident: &dyn Fn(&str) -> Ty,
    resolve_type: &dyn TypeResolver,
    lookup: &OpLookup,
) -> (Ty, Vec<ParseError>) {
    let mut diagnostics = Vec::new();
    let operand_tys: Vec<Ty> = operands
        .iter()
        .map(|operand| {
            let (ty, operand_diags) =
                check_expr_with_type_resolver(operand, resolve_ident, resolve_type, lookup);
            diagnostics.extend(operand_diags);
            ty
        })
        .collect();
    if !diagnostics.is_empty() {
        return (Ty::Any, diagnostics);
    }
    if let Some(ty) = matching_registered_type(name, &operand_tys, lookup) {
        return (ty, diagnostics);
    }
    let signatures = builtin_operand_types(name);
    let matched = signatures.iter().find(|sig| {
        sig.arity as usize == operand_tys.len()
            && (operand_tys[0] == Ty::Any || operand_tys[0].type_id() == Some(sig.lhs))
            && (operand_tys.len() < 2
                || operand_tys[1] == Ty::Any
                || operand_tys[1].type_id() == Some(sig.rhs))
    });
    if matched.is_some() {
        let ty = if operand_tys.iter().any(Ty::contains_any) {
            Ty::Any
        } else {
            result_ty_for_op(name, &operand_tys[0])
        };
        return (ty, diagnostics);
    }
    if let Some(ty) = registered_operation_ty(name, &operand_tys, span, lookup, &mut diagnostics) {
        return (ty, diagnostics);
    }
    if !signatures.is_empty() {
        let described = operand_tys
            .iter()
            .map(Ty::name)
            .collect::<Vec<_>>()
            .join(", ");
        diagnostics.push(ParseError::new_range(
            format!("`{name}` is not defined for operand type(s) `{described}`"),
            span.start,
            span.end,
        ));
    }
    (Ty::Any, diagnostics)
}

/// Resolves a named metadata overload without consulting same-named value bindings.
///
/// Returns `None` only when no metadata exists. Checks known operands exactly; unresolved
/// operands keep the result unknown. A mismatch produces one diagnostic and an unknown result.
///
/// - Complexity: O(s · a · d) for s signatures, a operands and descriptor depth d.
fn registered_operation_ty(
    name: &str,
    operands: &[Ty],
    span: ExprSpan,
    lookup: &OpLookup,
    diagnostics: &mut Vec<ParseError>,
) -> Option<Ty> {
    lookup.operation_signatures(name).next()?;
    if let Some(ty) = matching_registered_type(name, operands, lookup) {
        return Some(ty);
    }
    let described = operands.iter().map(Ty::name).collect::<Vec<_>>().join(", ");
    diagnostics.push(ParseError::new_range(
        format!("`{name}` is not defined for operand type(s) `{described}`"),
        span.start,
        span.end,
    ));
    Some(Ty::Any)
}

/// Returns the first compatible registered result, or `Any` when an operand is unresolved.
///
/// Returns `None` when no signature accepts the known operand constraints.
///
/// - Complexity: O(s · a · d) for s signatures, a operands and descriptor depth d.
fn matching_registered_type(name: &str, operands: &[Ty], lookup: &OpLookup) -> Option<Ty> {
    let signature = lookup.operation_signatures(name).find(|signature| {
        signature.operands.len() == operands.len()
            && signature
                .operands
                .iter()
                .zip(operands)
                .all(|(expected, actual)| {
                    resolved_type_to_element_ty(expected).unifies_with(actual)
                })
    })?;
    Some(if operands.iter().any(Ty::contains_any) {
        Ty::Any
    } else {
        resolved_type_to_element_ty(&signature.result)
    })
}

/// Returns the result type of a matched built-in operator application: `Ty::Bool` for the
/// comparison operators, otherwise the (matched, homogeneous) operand type — every built-in
/// signature is either a comparison (returning `bool`) or same-type-in-same-type-out (arithmetic,
/// bitwise, shifts, unary negation, logical not).
fn result_ty_for_op(name: &str, operand_ty: &Ty) -> Ty {
    match name {
        "==" | "!=" | "<" | "<=" | ">" | ">=" => Ty::Bool,
        _ => operand_ty.clone(),
    }
}

/// Checks an [`Expr::Cast`] node (`expr as type_name`): infers `expr`'s type, then (only if it
/// resolved to a concrete type) checks that a conversion to `type_name` is registered via
/// [`cast_source_types`]. Unlike [`check_op`], the node always infers as the target type once
/// `type_name` itself resolves - a cast declares its own result type - regardless of whether a
/// diagnostic was recorded for the source, matching [`check_logical`]'s pattern.
///
/// - Complexity: O(s) where s is the number of registered sources [`cast_source_types`] returns
///   for `type_name`, plus the cost of checking `expr` (see [`check_expr`]'s own complexity
///   note).
fn check_cast(
    expr: &Expr,
    type_name: &str,
    span: ExprSpan,
    resolve_ident: &dyn Fn(&str) -> Ty,
    resolve_type: &dyn TypeResolver,
    lookup: &OpLookup,
) -> (Ty, Vec<ParseError>) {
    let (expr_ty, mut diagnostics) =
        check_expr_with_type_resolver(expr, resolve_ident, resolve_type, lookup);
    let Some(target_ty) = Ty::from_name(type_name) else {
        diagnostics.push(ParseError::new_range(
            format!("unknown type `{type_name}`"),
            span.start,
            span.end,
        ));
        return (Ty::Any, diagnostics);
    };
    if expr_ty != Ty::Any
        && !expr_ty.type_id().is_some_and(|source_type_id| {
            cast_source_types(type_name).any(|id| id == source_type_id)
        })
    {
        diagnostics.push(ParseError::new_range(
            format!("no cast from `{}` to `{type_name}`", expr_ty.name()),
            span.start,
            span.end,
        ));
    }
    (target_ty, diagnostics)
}

/// Checks an [`Expr::Logical`] (`&&`/`||`) node: both operands should unify with `Ty::Bool` (CEL's
/// fixed short-circuit semantics, not table-driven like [`Expr::Op`]); the node's own type is
/// always `Ty::Bool` regardless of whether a diagnostic was recorded.
///
/// - Complexity: the sum of both operands' recursive type-checking costs.
fn check_logical(
    lhs: &Expr,
    rhs: &Expr,
    span: ExprSpan,
    resolve_ident: &dyn Fn(&str) -> Ty,
    resolve_type: &dyn TypeResolver,
    lookup: &OpLookup,
) -> (Ty, Vec<ParseError>) {
    let (lhs_ty, mut diagnostics) =
        check_expr_with_type_resolver(lhs, resolve_ident, resolve_type, lookup);
    let (rhs_ty, rhs_diags) =
        check_expr_with_type_resolver(rhs, resolve_ident, resolve_type, lookup);
    diagnostics.extend(rhs_diags);
    for (side, ty) in [("left", lhs_ty), ("right", rhs_ty)] {
        if !ty.unifies_with(&Ty::Bool) {
            diagnostics.push(ParseError::new_range(
                format!(
                    "`&&`/`||` requires `bool`, found `{}` on the {side}",
                    ty.name()
                ),
                span.start,
                span.end,
            ));
        }
    }
    (Ty::Bool, diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LogicalOp;

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
    fn from_literal_preserves_nonprimitive_leaf_identity() {
        for (literal, id) in [
            (Literal::Char('a'), TypeId::of::<char>()),
            (Literal::ByteStr(vec![1]), TypeId::of::<Vec<u8>>()),
            (
                Literal::CStr(std::ffi::CString::new("a").unwrap()),
                TypeId::of::<std::ffi::CString>(),
            ),
            (Literal::Unit, TypeId::of::<()>()),
        ] {
            assert_eq!(Ty::from_literal(&literal).type_id(), Some(id));
            assert!(!Ty::from_literal(&literal).unifies_with(&Ty::F64));
        }
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

    /// Every concrete (non-[`Ty::Array`], non-[`Ty::Any`]) variant, in declaration order.
    fn concrete_tys() -> [Ty; 16] {
        [
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
        ]
    }

    #[test]
    fn every_builtin_scalar_table_agrees_on_the_same_names_and_types() {
        use crate::op_table::{BUILTIN_SCALAR_NAMES, builtin_scalar_type};
        use crate::type_expr::TypeResolver;

        for ty in concrete_tys() {
            let name = ty.name();
            let expected_id = ty.type_id().expect("a concrete Ty has a TypeId");
            assert!(
                BUILTIN_SCALAR_NAMES.contains(&&*name),
                "`{name}` must be one of the built-in scalar names"
            );
            assert_eq!(
                Ty::from_name(&name),
                Some(ty.clone()),
                "`{name}` must round-trip through Ty::from_name"
            );
            let scalar = builtin_scalar_type(&name)
                .unwrap_or_else(|| panic!("`{name}` must resolve as a built-in scalar"));
            assert_eq!((scalar.element_type)().type_id(), expected_id);
            assert_eq!(scalar.type_name, name);
            let leaf = BuiltinTypeResolver
                .resolve_named_type(&name, &[])
                .unwrap_or_else(|| panic!("`{name}` must resolve as a CEL leaf type"));
            assert_eq!(leaf.type_id(), expected_id);
            assert_eq!(leaf.type_name(), name);
            assert_eq!(leaf.element_type().type_id(), expected_id);
            assert_eq!(
                leaf.element_type().type_name(),
                name,
                "the runtime element descriptor must carry the source-level name"
            );
            assert_eq!(leaf.element_type().size(), (scalar.element_type)().size());
            assert_eq!(leaf.element_type().align(), (scalar.element_type)().align());
        }
    }

    #[test]
    fn every_builtin_scalar_name_has_a_concrete_ty() {
        use crate::op_table::BUILTIN_SCALAR_NAMES;

        let types = concrete_tys();
        let ty_names: Vec<_> = types.iter().map(Ty::name).collect();
        for name in BUILTIN_SCALAR_NAMES {
            assert!(
                ty_names.iter().any(|ty_name| ty_name == name),
                "the built-in scalar `{name}` has no concrete Ty variant"
            );
        }
    }

    #[test]
    fn name_is_distinct_per_type() {
        let types = [
            Ty::I32,
            Ty::F64,
            Ty::Bool,
            Ty::String,
            Ty::Array(Box::new(Ty::I32)),
            Ty::Array(Box::new(Ty::Array(Box::new(Ty::I32)))),
            Ty::Any,
        ];
        let names: Vec<_> = types.iter().map(Ty::name).collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            names.len(),
            unique.len(),
            "every listed Ty has a distinct name"
        );
    }

    fn any_resolver(_name: &str) -> Ty {
        Ty::Any
    }

    fn point(span: proc_macro2::Span) -> ExprSpan {
        ExprSpan {
            start: span,
            end: span,
        }
    }

    fn lit_i32(v: i32) -> Expr {
        Expr::Literal {
            value: Literal::I32(v),
            span: point(proc_macro2::Span::call_site()),
        }
    }

    fn lit_f64(v: f64) -> Expr {
        Expr::Literal {
            value: Literal::F64(v),
            span: point(proc_macro2::Span::call_site()),
        }
    }

    fn lit_bool(v: bool) -> Expr {
        Expr::Literal {
            value: Literal::Bool(v),
            span: point(proc_macro2::Span::call_site()),
        }
    }

    fn lit_str(v: &str) -> Expr {
        Expr::Literal {
            value: Literal::Str(v.to_string()),
            span: point(proc_macro2::Span::call_site()),
        }
    }

    fn op(name: &str, operands: Vec<Expr>) -> Expr {
        Expr::Op {
            name: name.to_string(),
            operands,
            span: point(proc_macro2::Span::call_site()),
        }
    }

    fn array(elements: Vec<Expr>) -> Expr {
        Expr::Array {
            elements,
            type_annotation: None,
            annotation_span: None,
            span: point(proc_macro2::Span::call_site()),
        }
    }

    fn call(callee: &str) -> Expr {
        Expr::Apply {
            callee: Box::new(Expr::Ident {
                name: callee.to_string(),
                span: point(proc_macro2::Span::call_site()),
            }),
            args: Vec::new(),
            span: point(proc_macro2::Span::call_site()),
        }
    }

    #[test]
    fn literal_infers_its_own_type() {
        let (ty, diags) = check_expr(&lit_i32(1), &any_resolver);
        assert_eq!(ty, Ty::I32);
        assert!(diags.is_empty());
    }

    #[test]
    fn ident_resolves_via_the_supplied_resolver() {
        let expr = Expr::Ident {
            name: "width".to_string(),
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&expr, &|name| {
            if name == "width" { Ty::F64 } else { Ty::Any }
        });
        assert_eq!(ty, Ty::F64);
        assert!(diags.is_empty());
    }

    #[test]
    fn unknown_ident_is_any_and_not_a_diagnostic() {
        let expr = Expr::Ident {
            name: "mystery".to_string(),
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Any);
        assert!(diags.is_empty());
    }

    #[test]
    fn op_with_matching_signature_infers_the_operand_type() {
        let expr = op("+", vec![lit_i32(1), lit_i32(2)]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::I32);
        assert!(diags.is_empty());
    }

    #[test]
    fn comparison_op_always_infers_bool() {
        let expr = op("==", vec![lit_i32(1), lit_i32(2)]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Bool);
        assert!(diags.is_empty());
    }

    #[test]
    fn unary_negation_preserves_the_operand_type() {
        let expr = op("-", vec![lit_i32(1)]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::I32);
        assert!(diags.is_empty());
    }

    #[test]
    fn op_with_mismatched_operand_types_produces_one_diagnostic_and_infers_any() {
        let expr = op("+", vec![lit_i32(1), lit_str("s")]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Any);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn op_with_an_any_operand_produces_no_diagnostic() {
        let expr = op(
            "+",
            vec![
                Expr::Ident {
                    name: "mystery".to_string(),
                    span: point(proc_macro2::Span::call_site()),
                },
                lit_i32(1),
            ],
        );
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Any);
        assert!(diags.is_empty());
    }

    #[test]
    fn unregistered_operator_name_is_any_and_not_a_diagnostic() {
        // "greet" is only ever registered at runtime via OpLookup::register_tuple_op; the static
        // checker can't see it and must not guess.
        let expr = op("greet", vec![lit_i32(1)]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Any);
        assert!(diags.is_empty());
    }

    #[test]
    fn logical_and_with_bool_operands_infers_bool_with_no_diagnostic() {
        let expr = Expr::Logical {
            op: LogicalOp::And,
            lhs: Box::new(lit_bool(true)),
            rhs: Box::new(lit_bool(false)),
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Bool);
        assert!(diags.is_empty());
    }

    #[test]
    fn logical_or_with_a_non_bool_operand_produces_a_diagnostic_but_still_infers_bool() {
        let expr = Expr::Logical {
            op: LogicalOp::Or,
            lhs: Box::new(lit_i32(1)),
            rhs: Box::new(lit_bool(true)),
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Bool);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn a_broken_op_nested_inside_a_tuple_still_surfaces_a_diagnostic() {
        let expr = Expr::Tuple {
            elements: vec![op("+", vec![lit_i32(1), lit_str("s")]), lit_i32(2)],
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Tuple(vec![Ty::Any, Ty::I32]));
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn a_broken_op_nested_inside_an_if_condition_still_surfaces_a_diagnostic() {
        let expr = Expr::If {
            cond: Box::new(op("+", vec![lit_i32(1), lit_str("s")])),
            then_branch: Box::new(lit_i32(1)),
            else_branch: Some(Box::new(lit_i32(2))),
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::I32);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn a_broken_op_nested_inside_a_call_argument_still_surfaces_a_diagnostic() {
        let expr = Expr::Apply {
            callee: Box::new(Expr::Ident {
                name: "f".to_string(),
                span: point(proc_macro2::Span::call_site()),
            }),
            args: vec![op("+", vec![lit_i32(1), lit_str("s")])],
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Any, "Apply itself is not type-checked in v1");
        assert_eq!(diags.len(), 1);
    }

    fn cast(inner: Expr, type_name: &str) -> Expr {
        Expr::Cast {
            expr: Box::new(inner),
            type_name: type_name.to_string(),
            span: point(proc_macro2::Span::call_site()),
        }
    }

    #[test]
    fn cast_to_a_registered_numeric_target_infers_the_target_type_with_no_diagnostic() {
        let (ty, diags) = check_expr(&cast(lit_i32(1), "f64"), &any_resolver);
        assert_eq!(ty, Ty::F64);
        assert!(diags.is_empty());
    }

    #[test]
    fn cast_from_bool_to_bool_produces_no_diagnostic() {
        // Regression test for the `ty.rs`/`op_table.rs` "no cast from ... to bool" vs. "unknown
        // type" inconsistency: bool is the one type Rust's own `as` allows as a source for `as
        // bool` (a no-op identity conversion), so this must not be a diagnostic.
        let (ty, diags) = check_expr(&cast(lit_bool(true), "bool"), &any_resolver);
        assert_eq!(ty, Ty::Bool);
        assert!(diags.is_empty());
    }

    #[test]
    fn cast_from_a_number_to_bool_is_a_diagnostic_not_an_unknown_type_error() {
        // `bool` is a recognized cast target (matches [`op_table::signatures_for_cast`]), just one
        // with no registered `i32` source - same "no cast from `i32` to `bool`" diagnostic
        // `OpLookup::lookup_cast` would report at execution time, not "unknown type `bool`".
        let (ty, diags) = check_expr(&cast(lit_i32(1), "bool"), &any_resolver);
        assert_eq!(ty, Ty::Bool);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message().contains("no cast from"));
    }

    #[test]
    fn cast_to_an_unrecognized_type_name_is_a_diagnostic_and_infers_any() {
        let (ty, diags) = check_expr(&cast(lit_i32(1), "nonsense"), &any_resolver);
        assert_eq!(ty, Ty::Any);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn cast_of_an_any_typed_operand_produces_no_diagnostic() {
        let expr = Expr::Ident {
            name: "mystery".to_string(),
            span: point(proc_macro2::Span::call_site()),
        };
        let (ty, diags) = check_expr(&cast(expr, "i32"), &any_resolver);
        assert_eq!(ty, Ty::I32);
        assert!(diags.is_empty());
    }

    fn closure_param(name: &str, type_name: &str) -> crate::ClosureParam {
        crate::ClosureParam {
            name: name.to_string(),
            name_span: point(proc_macro2::Span::call_site()),
            type_expr: crate::TypeExpr::Named {
                name: type_name.to_string(),
                args: Vec::new(),
                span: point(proc_macro2::Span::call_site()),
            },
        }
    }

    fn closure(params: Vec<crate::ClosureParam>, body: Expr) -> Expr {
        Expr::Closure {
            params,
            body: Box::new(body),
            span: point(proc_macro2::Span::call_site()),
        }
    }

    #[test]
    fn closure_literal_preserves_its_signature() {
        let expr = closure(
            vec![closure_param("x", "i32")],
            Expr::Ident {
                name: "x".to_string(),
                span: point(proc_macro2::Span::call_site()),
            },
        );
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Callable(vec![Ty::I32], Box::new(Ty::I32)));
        assert!(diags.is_empty());
    }

    #[test]
    fn closure_body_type_error_surfaces_through_the_closure() {
        let body = op(
            "+",
            vec![
                Expr::Ident {
                    name: "x".to_string(),
                    span: point(proc_macro2::Span::call_site()),
                },
                lit_str("s"),
            ],
        );
        let expr = closure(vec![closure_param("x", "i32")], body);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Callable(vec![Ty::I32], Box::new(Ty::Any)));
        assert_eq!(diags.len(), 1, "the body's i32 + String mismatch surfaces");
    }

    #[test]
    fn closure_param_shadows_the_outer_resolver() {
        // Outer resolver claims "x" is a String; the closure's own "x: i32" parameter must win
        // inside the body, so `x + 1i32` type-checks cleanly with no diagnostic.
        let outer_resolver = |_: &str| Ty::String;
        let body = op(
            "+",
            vec![
                Expr::Ident {
                    name: "x".to_string(),
                    span: point(proc_macro2::Span::call_site()),
                },
                lit_i32(1),
            ],
        );
        let expr = closure(vec![closure_param("x", "i32")], body);
        let (_, diags) = check_expr(&expr, &outer_resolver);
        assert!(diags.is_empty());
    }

    #[test]
    fn array_type_id_is_the_runtime_array_wrapper_regardless_of_element_type() {
        let element_id = TypeId::of::<cel_runtime::DynamicArray>();
        assert_eq!(Ty::Array(Box::new(Ty::I32)).type_id(), Some(element_id));
        assert_eq!(Ty::Array(Box::new(Ty::Any)).type_id(), Some(element_id));
    }

    #[test]
    fn from_type_id_maps_the_array_wrapper_to_an_array_of_any() {
        assert_eq!(
            Ty::from_type_id(TypeId::of::<cel_runtime::DynamicArray>()),
            Ty::Array(Box::new(Ty::Any))
        );
    }

    #[test]
    fn array_name_renders_bracket_syntax_recursively() {
        assert_eq!(Ty::Array(Box::new(Ty::I32)).name(), "[i32]");
        assert_eq!(
            Ty::Array(Box::new(Ty::Array(Box::new(Ty::I32)))).name(),
            "[[i32]]"
        );
        assert_eq!(Ty::Array(Box::new(Ty::Any)).name(), "[<any>]");
    }

    #[test]
    fn arrays_unify_only_when_their_element_types_unify_recursively() {
        assert!(Ty::Array(Box::new(Ty::I32)).unifies_with(&Ty::Array(Box::new(Ty::I32))));
        assert!(Ty::Array(Box::new(Ty::Any)).unifies_with(&Ty::Array(Box::new(Ty::I32))));
        assert!(!Ty::Array(Box::new(Ty::I32)).unifies_with(&Ty::Array(Box::new(Ty::F64))));
        assert!(
            !Ty::Array(Box::new(Ty::Array(Box::new(Ty::I32))))
                .unifies_with(&Ty::Array(Box::new(Ty::I32))),
            "differing nesting depth does not unify"
        );
        assert!(!Ty::Array(Box::new(Ty::I32)).unifies_with(&Ty::I32));
        assert!(Ty::Any.unifies_with(&Ty::Array(Box::new(Ty::I32))));
        assert!(Ty::Array(Box::new(Ty::I32)).unifies_with(&Ty::Any));
    }

    #[test]
    fn unify_keeps_the_more_concrete_side_at_every_nesting_level() {
        assert_eq!(Ty::Any.unify(&Ty::I32), Some(Ty::I32));
        assert_eq!(Ty::I32.unify(&Ty::Any), Some(Ty::I32));
        assert_eq!(
            Ty::Array(Box::new(Ty::Any)).unify(&Ty::Array(Box::new(Ty::Array(Box::new(Ty::I32))))),
            Some(Ty::Array(Box::new(Ty::Array(Box::new(Ty::I32)))))
        );
    }

    #[test]
    fn unify_of_incompatible_types_is_none() {
        assert_eq!(Ty::I32.unify(&Ty::F64), None);
        assert_eq!(Ty::Array(Box::new(Ty::I32)).unify(&Ty::I32), None);
        assert_eq!(
            Ty::Array(Box::new(Ty::I32)).unify(&Ty::Array(Box::new(Ty::F64))),
            None
        );
    }

    #[test]
    fn a_unified_type_unifies_with_both_of_its_sides() {
        let lhs = Ty::Array(Box::new(Ty::Any));
        let rhs = Ty::Array(Box::new(Ty::I32));
        let merged = lhs.unify(&rhs).expect("compatible types unify");
        assert!(merged.unifies_with(&lhs));
        assert!(merged.unifies_with(&rhs));
    }

    #[test]
    fn homogeneous_array_infers_its_element_type() {
        let (ty, diags) = check_expr(&array(vec![lit_i32(1), lit_i32(2)]), &any_resolver);
        assert_eq!(ty, Ty::Array(Box::new(Ty::I32)));
        assert!(diags.is_empty());
    }

    #[test]
    fn single_element_array_infers_that_element_type() {
        let (ty, diags) = check_expr(&array(vec![lit_str("s")]), &any_resolver);
        assert_eq!(ty, Ty::Array(Box::new(Ty::String)));
        assert!(diags.is_empty());
    }

    #[test]
    fn nested_array_types_unify_recursively() {
        let expr = array(vec![array(vec![lit_i32(1)]), array(vec![lit_i32(2)])]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Array(Box::new(Ty::Array(Box::new(Ty::I32)))));
        assert!(diags.is_empty());
    }

    #[test]
    fn heterogeneous_array_reports_the_conflicting_element() {
        let (ty, diags) = check_expr(&array(vec![lit_i32(1), lit_f64(2.0)]), &any_resolver);
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0].message().contains("expected `i32`, found `f64`"),
            "got: {}",
            diags[0].message()
        );
        assert_eq!(
            ty,
            Ty::Array(Box::new(Ty::Any)),
            "a conflicting literal is still an array, of an unknown element type"
        );
    }

    #[test]
    fn heterogeneous_nested_array_reports_the_recursive_element_types() {
        let expr = array(vec![array(vec![lit_i32(1)]), array(vec![lit_f64(1.0)])]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0]
                .message()
                .contains("expected `[i32]`, found `[f64]`"),
            "got: {}",
            diags[0].message()
        );
        assert_eq!(ty, Ty::Array(Box::new(Ty::Any)));
    }

    #[test]
    fn mismatched_nesting_depth_is_a_diagnostic() {
        let expr = array(vec![array(vec![lit_i32(1)]), lit_i32(2)]);
        let (_, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0].message().contains("expected `[i32]`, found `i32`"),
            "got: {}",
            diags[0].message()
        );
    }

    #[test]
    fn array_of_unresolved_call_results_infers_an_array_of_any_with_no_diagnostic() {
        // `Apply` results are deliberately `Ty::Any` in v1, so `[f(), g()]` cannot be proven
        // homogeneous - and must not be reported as heterogeneous either.
        let expr = array(vec![call("f"), call("g")]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Array(Box::new(Ty::Any)));
        assert!(diags.is_empty());
    }

    #[test]
    fn an_any_element_keeps_the_concrete_element_type() {
        let expr = array(vec![call("f"), lit_i32(1)]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Array(Box::new(Ty::I32)));
        assert!(diags.is_empty());
    }

    #[test]
    fn an_any_nested_element_keeps_the_concrete_recursive_element_type() {
        let expr = array(vec![array(vec![call("f")]), array(vec![lit_i32(1)])]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Array(Box::new(Ty::Array(Box::new(Ty::I32)))));
        assert!(diags.is_empty());
    }

    #[test]
    fn every_elements_own_diagnostics_are_retained() {
        let expr = array(vec![
            op("+", vec![lit_i32(1), lit_str("s")]),
            op("+", vec![lit_i32(2), lit_str("t")]),
        ]);
        let (_, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(diags.len(), 2, "both broken elements are reported");
    }

    #[test]
    fn a_later_elements_own_diagnostic_survives_an_earlier_element_conflict() {
        let expr = array(vec![
            lit_i32(1),
            lit_f64(2.0),
            op("+", vec![lit_i32(3), lit_str("s")]),
        ]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(
            diags.len(),
            2,
            "one element-conflict diagnostic plus the third element's own operator diagnostic"
        );
        assert_eq!(ty, Ty::Array(Box::new(Ty::Any)));
    }

    #[test]
    fn only_the_first_conflicting_element_is_reported() {
        let expr = array(vec![lit_i32(1), lit_f64(2.0), lit_bool(true)]);
        let (_, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(diags.len(), 1, "one diagnostic per array literal");
        assert!(diags[0].message().contains("found `f64`"), "the first one");
    }

    #[test]
    fn a_conflicting_elements_diagnostic_is_anchored_at_that_element() {
        // Every other array test builds spans with `Span::call_site()`, where "this element's
        // span", "the whole literal's span", and "some other element's span" are all
        // indistinguishable. Parsing real source (proc-macro2's `span-locations` feature is on)
        // gives each token its own line/column, making the anchoring observable: the conflict is
        // reported at the element on line 4, not at the enclosing `[`...`]` starting on line 1,
        // and not at a fixed earlier position such as line 2 or 3.
        let source = "[\n    1i32,\n    2i32,\n    3.0f64,\n    4i32\n]";
        let mut parser = crate::Parser::<crate::AstContext>::new(crate::OpLookup::new());
        let expr = parser.parse_str_ast(source).expect("source parses");
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Array(Box::new(Ty::Any)));
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0].message().contains("expected `i32`, found `f64`"),
            "got: {}",
            diags[0].message()
        );
        let start = diags[0].span().start();
        let end = diags[0]
            .end_span()
            .expect("the conflict diagnostic spans a range")
            .end();
        assert_eq!(start.line, 4, "anchored at the conflicting element's line");
        assert_eq!(start.column, 4, "at the element, not the enclosing group");
        assert_eq!(end.line, 4);
        assert_eq!(end.column, 4 + "3.0f64".len());
    }

    #[test]
    fn a_conflict_inside_a_nested_array_does_not_cascade_to_the_outer_literal() {
        // `[[1i32, 2.0f64], [3i32, 4i32]]`: the inner `[1i32, 2.0f64]` conflicts and falls back
        // to `Array(Any)`, which still unifies with the valid inner array's `Array(i32)` - so the
        // outer fold stays silent and the outer literal keeps the concrete nested element type.
        let expr = array(vec![
            array(vec![lit_i32(1), lit_f64(2.0)]),
            array(vec![lit_i32(3), lit_i32(4)]),
        ]);
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(diags.len(), 1, "only the inner conflict is reported");
        assert!(
            diags[0].message().contains("expected `i32`, found `f64`"),
            "got: {}",
            diags[0].message()
        );
        assert_eq!(ty, Ty::Array(Box::new(Ty::Array(Box::new(Ty::I32)))));
    }

    #[test]
    fn typed_array_annotation_returns_the_declared_array_type() {
        let mut parser = crate::Parser::<crate::AstContext>::new(crate::OpLookup::new());
        let expr = parser
            .parse_str_ast("[0, 1]: [i32]")
            .expect("source parses");
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Array(Box::new(Ty::I32)));
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    #[test]
    fn typed_empty_array_annotation_returns_the_declared_nested_array_type() {
        let mut parser = crate::Parser::<crate::AstContext>::new(crate::OpLookup::new());
        let expr = parser.parse_str_ast("[]: [[i32]]").expect("source parses");
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Array(Box::new(Ty::Array(Box::new(Ty::I32)))));
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    #[test]
    fn typed_array_annotation_reports_exact_element_type_mismatches() {
        let mut parser = crate::Parser::<crate::AstContext>::new(crate::OpLookup::new());
        let expr = parser
            .parse_str_ast("[0, 1]: [f64]")
            .expect("source parses");
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Array(Box::new(Ty::F64)));
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0].message().contains("expected `f64`, found `i32`"),
            "got: {}",
            diags[0].message()
        );
    }

    #[test]
    fn typed_array_annotation_reports_exact_custom_element_type_mismatches() {
        #[derive(Clone)]
        struct Custom;

        let mut parser = crate::Parser::<crate::AstContext>::new(crate::OpLookup::new());
        let expr = parser
            .parse_str_ast("[0]: [Custom]")
            .expect("source parses");
        let (ty, diags) = check_expr_with_type_resolver(
            &expr,
            &any_resolver,
            &[(
                "Custom",
                crate::ResolvedLeafType::new(
                    "Custom",
                    cel_runtime::ArrayElementType::leaf::<Custom>().unwrap(),
                ),
            )],
            &OpLookup::new(),
        );
        assert_eq!(
            ty,
            Ty::Array(Box::new(Ty::Registered(crate::ResolvedLeafType::new(
                "Custom",
                cel_runtime::ArrayElementType::leaf::<Custom>().unwrap()
            ))))
        );
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0]
                .message()
                .contains("expected `Custom`, found `i32`"),
            "got: {}",
            diags[0].message()
        );
    }

    /// A nested literal's own annotation is part of what it means: `[[]: [CustomA]]` is an array
    /// of `[CustomA]`, which the runtime rejects against `[[CustomB]]`.
    #[test]
    fn nested_annotated_array_reports_a_custom_element_mismatch() {
        let mut parser = crate::Parser::<crate::AstContext>::new(crate::OpLookup::new());
        let expr = parser
            .parse_str_ast("[[]: [CustomA]]: [[CustomB]]")
            .expect("source parses");
        let (ty, diags) = check_expr_with_type_resolver(
            &expr,
            &any_resolver,
            &custom_ab_resolver(),
            &OpLookup::new(),
        );

        assert_eq!(
            ty,
            Ty::Array(Box::new(Ty::Array(Box::new(Ty::Registered(
                custom_ab_resolver()[1].1.clone()
            )))))
        );
        assert_eq!(diags.len(), 1, "unexpected diagnostics: {diags:?}");
        assert!(
            diags[0]
                .message()
                .contains("expected `[CustomB]`, found `[CustomA]`"),
            "got: {}",
            diags[0].message()
        );
    }

    /// The same nesting with agreeing annotations is accepted with no diagnostic.
    #[test]
    fn nested_annotated_array_accepts_a_matching_custom_element_annotation() {
        let mut parser = crate::Parser::<crate::AstContext>::new(crate::OpLookup::new());
        let expr = parser
            .parse_str_ast("[[]: [CustomA]]: [[CustomA]]")
            .expect("source parses");
        let (ty, diags) = check_expr_with_type_resolver(
            &expr,
            &any_resolver,
            &custom_ab_resolver(),
            &OpLookup::new(),
        );

        assert_eq!(
            ty,
            Ty::Array(Box::new(Ty::Array(Box::new(Ty::Registered(
                custom_ab_resolver()[0].1.clone()
            )))))
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// A resolver registering two distinct custom leaf types.
    fn custom_ab_resolver() -> [(&'static str, crate::ResolvedLeafType); 2] {
        #[derive(Clone)]
        struct CustomA;
        #[derive(Clone)]
        struct CustomB;

        [
            (
                "CustomA",
                crate::ResolvedLeafType::new(
                    "CustomA",
                    cel_runtime::ArrayElementType::leaf::<CustomA>().unwrap(),
                ),
            ),
            (
                "CustomB",
                crate::ResolvedLeafType::new(
                    "CustomB",
                    cel_runtime::ArrayElementType::leaf::<CustomB>().unwrap(),
                ),
            ),
        ]
    }

    #[test]
    fn typed_array_annotation_diagnostics_are_anchored_at_the_annotation_span() {
        let source = "[0]: i32";
        let mut parser = crate::Parser::<crate::AstContext>::new(crate::OpLookup::new());
        let expr = parser.parse_str_ast(source).expect("source parses");
        let (_, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0]
                .message()
                .contains("array annotations must name a complete array type"),
            "got: {}",
            diags[0].message()
        );
        let start = diags[0].span().start();
        let end = diags[0]
            .end_span()
            .expect("annotation diagnostics span a range")
            .end();
        assert_eq!(&source[start.column..end.column], ": i32");
    }

    #[test]
    fn typed_array_annotation_preserves_tuple_array_element_types() {
        let mut parser = crate::Parser::<crate::AstContext>::new(crate::OpLookup::new());
        let expr = parser
            .parse_str_ast("[]: [(i32, f64)]")
            .expect("source parses");
        let (ty, diags) = check_expr(&expr, &any_resolver);
        assert_eq!(ty, Ty::Array(Box::new(Ty::Tuple(vec![Ty::I32, Ty::F64]))));
        assert!(diags.is_empty());
    }

    #[test]
    fn an_empty_array_expression_infers_an_array_of_any() {
        // The grammar rejects `[]` outright (see #212), but an `Expr::Array` built directly still
        // has to infer conservatively rather than panic.
        let (ty, diags) = check_expr(&array(Vec::new()), &any_resolver);
        assert_eq!(ty, Ty::Array(Box::new(Ty::Any)));
        assert!(diags.is_empty());
    }
}
