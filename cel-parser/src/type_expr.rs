//! Recursive CEL type expressions plus the small resolver boundary they need.
//!
//! Parsing stays in `cel-parser`, while leaf-name lookup is delegated to a [`TypeResolver`].
//! This keeps CEL independent of `adam-lang` while still allowing a host to register custom
//! types for typed arrays and future typed CEL surfaces.

use std::any::TypeId;
use std::borrow::Cow;

use cel_runtime::ValueType;

use crate::{ExprSpan, ParseError, Result, op_table};

/// `type_expression = identifier [ "(" [ type_expression { "," type_expression } ] ")" ]
///                   | "[" type_expression "]"
///                   | "(" [ type_expression ["," [ type_expression { "," type_expression } ]] ] ")".`
///
/// `()` is the empty tuple type (0 elements); `(T)` is grouping (same as bare `T`); `(T,)` is a
/// 1-element tuple; `(T, U, ...)` is n-element, no trailing comma. `[T]` names a rank-one array
/// whose elements have type `T`; nested brackets compose recursively. `Name(A, B)` names a
/// parameterized (generic) type applying zero or more type arguments (`RangeInclusive(f64)`,
/// `RangeFull()`); an identifier immediately followed by `(` has no other meaning in type
/// position, so this is unambiguous with adjacent tuple-grouping syntax.
/// `Fn((A, B), R)` reserves structural callable syntax: the first argument must be a tuple
/// of parameter types and the second is the result. `Fn((), R)` has no parameters;
/// `Fn((A,), R)` has one. This syntax is resolved before host generic-leaf lookup.
#[derive(Clone, Debug)]
pub enum TypeExpr {
    /// A single type name, optionally applying type arguments, resolved later through a
    /// [`TypeResolver`].
    Named {
        /// The unresolved type name, exactly as written.
        name: String,
        /// Type arguments, e.g. `f64` in `RangeInclusive(f64)`. Empty for a plain name.
        args: Vec<TypeExpr>,
        /// The source span of the full name token, including any parenthesized argument list.
        span: ExprSpan,
    },
    /// A recursively nested array type expression (`[T]`).
    Array {
        /// The element type expression.
        element: Box<TypeExpr>,
        /// The span of the whole bracketed type.
        span: ExprSpan,
    },
    /// A recursively nested tuple type expression.
    Tuple {
        /// The tuple element type expressions, in source order.
        elements: Vec<TypeExpr>,
        /// The span of the whole parenthesized type.
        span: ExprSpan,
    },
}

impl TypeExpr {
    /// Returns this type expression's source span.
    #[must_use]
    pub fn span(&self) -> ExprSpan {
        match self {
            TypeExpr::Named { span, .. }
            | TypeExpr::Array { span, .. }
            | TypeExpr::Tuple { span, .. } => *span,
        }
    }

    /// Resolves every named leaf through `resolver`, preserving the recursive type shape.
    ///
    /// # Errors
    ///
    /// Returns `Err` if some named leaf is not recognized by `resolver`, or `Fn` lacks
    /// exactly two arguments with a tuple of parameter types first.
    ///
    /// - Complexity: O(n) in the number of nodes in this type tree.
    pub fn resolve(&self, resolver: &dyn TypeResolver) -> Result<ResolvedType> {
        match self {
            TypeExpr::Named { name, args, span } => {
                let resolved_args = args
                    .iter()
                    .map(|arg| arg.resolve(resolver))
                    .collect::<Result<Vec<_>>>()?;
                if name == "Fn" {
                    let mut resolved_args = resolved_args.into_iter();
                    return match (
                        resolved_args.next(),
                        resolved_args.next(),
                        resolved_args.next(),
                    ) {
                        (Some(ResolvedType::Tuple { elements }), Some(result), None) => {
                            Ok(ResolvedType::Callable {
                                parameters: elements,
                                result: Box::new(result),
                            })
                        }
                        _ => Err(ParseError::new_range(
                            "Fn requires a parameter tuple and a result type",
                            span.start,
                            span.end,
                        )),
                    };
                }
                resolver
                    .resolve_named_type(name, &resolved_args)
                    .map_or_else(
                        || {
                            Err(ParseError::new_range(
                                format!("unknown type `{name}`"),
                                span.start,
                                span.end,
                            ))
                        },
                        |leaf| Ok(ResolvedType::Scalar(leaf)),
                    )
            }
            TypeExpr::Array { element, .. } => Ok(ResolvedType::Array {
                element: Box::new(element.resolve(resolver)?),
            }),
            TypeExpr::Tuple { elements, .. } => Ok(ResolvedType::Tuple {
                elements: elements
                    .iter()
                    .map(|element| element.resolve(resolver))
                    .collect::<Result<Vec<_>>>()?,
            }),
        }
    }
}

/// Resolves one named leaf type for CEL type expressions.
///
/// Implementations return `None` when `name` is unknown. Recursive array and tuple composition is
/// performed by [`TypeExpr::resolve`], so a resolver only needs to recognize leaf names.
pub trait TypeResolver: Send + Sync {
    /// Returns the registered leaf type named `name` applying `args` (already resolved), or
    /// `None` if `name`/`args` is unrecognized. `args` is empty for a plain (non-generic) name.
    fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType>;
}

impl<F> TypeResolver for F
where
    F: Fn(&str, &[ResolvedType]) -> Option<ResolvedLeafType> + Send + Sync,
{
    fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType> {
        self(name, args)
    }
}

impl<const N: usize> TypeResolver for [(&'static str, ResolvedLeafType); N] {
    fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType> {
        if !args.is_empty() {
            return None;
        }
        self.iter()
            .find(|(registered_name, _)| *registered_name == name)
            .map(|(_, leaf)| leaf.clone())
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct BuiltinTypeResolver;

impl TypeResolver for BuiltinTypeResolver {
    fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType> {
        let builtin = match args {
            [] => op_table::builtin_scalar_type(name)
                .or_else(|| op_table::builtin_generic_type_0(name)),
            [ResolvedType::Scalar(arg_leaf)] => {
                op_table::builtin_generic_type(name, arg_leaf.type_name())
            }
            _ => None,
        }?;
        Some(ResolvedLeafType::new(
            builtin.type_name,
            (builtin.value_type)(),
        ))
    }
}

/// Returns `cel-parser`'s own built-in type resolver — every scalar name recognized by
/// `builtin_scalar_type`, plus every built-in generic type recognized by `builtin_generic_type`
/// (the `Range` family).
///
/// A host embedding `cel-parser` with its own [`TypeResolver`] (one that also knows
/// host-specific custom types) can delegate any name it doesn't itself recognize to this
/// resolver, so built-in types stay visible inside host-embedded CEL expressions.
///
/// The returned resolver is uniquely owned and zero-sized; its box does not allocate.
///
/// # Examples
/// ```
/// let resolver = cel_parser::builtin_type_resolver();
/// assert!(resolver.resolve_named_type("u32", &[]).is_some());
/// ```
#[must_use]
pub fn builtin_type_resolver() -> Box<dyn TypeResolver> {
    Box::new(BuiltinTypeResolver)
}

/// Resolves `name`/`args` against `cel-parser`'s own built-in type resolver, without allocating
/// a [`TypeResolver`] trait object.
///
/// Equivalent to `builtin_type_resolver().resolve_named_type(name, args)`, but for a host that
/// only needs a one-off built-in fallback lookup (rather than a `TypeResolver` trait object to
/// pass around), this avoids constructing a trait object.
#[must_use]
pub fn resolve_builtin_named_type(name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType> {
    BuiltinTypeResolver.resolve_named_type(name, args)
}

/// One resolved scalar leaf type.
///
/// This pairs a human-readable type name with complete canonical leaf metadata.
/// Hosts use [`ValueType::cloneable_leaf`] when this leaf may also appear in a closure
/// annotation; the same descriptor then supplies its checked argument-cloning adapter.
///
/// Concrete element factories retain complete metadata. Use
/// [`Self::with_complete_value_type`] to enrich its optional capabilities;
/// metadata cannot be recovered from a `TypeId`.
/// Equality compares registered identity, not optional capability availability.
#[derive(Clone, Debug)]
pub struct ResolvedLeafType {
    type_id: TypeId,
    type_name: Cow<'static, str>,
    complete: ValueType,
}

impl PartialEq for ResolvedLeafType {
    /// Compares registered leaf identities independently of optional capabilities.
    ///
    /// - Complexity: O(l) for name length l.
    fn eq(&self, other: &Self) -> bool {
        self.type_id == other.type_id
            && self.type_name == other.type_name
            && self.complete.same_shape(&other.complete)
    }
}

impl Eq for ResolvedLeafType {}

impl ResolvedLeafType {
    /// Registers an opaque concrete host leaf without optional capabilities.
    ///
    /// Foreign types require no trait implementation or wrapper.
    ///
    /// # Errors
    ///
    /// Rejects an erased whole-value owner, which is not a concrete CEL leaf.
    ///
    /// - Complexity: O(l) time and space for a source-facing name of length l.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_parser::ResolvedLeafType;
    /// let leaf = ResolvedLeafType::leaf::<std::path::PathBuf>("Path").unwrap();
    /// assert!(leaf.complete_value_type().is_ok());
    /// ```
    pub fn leaf<T: 'static>(type_name: impl Into<Cow<'static, str>>) -> anyhow::Result<Self> {
        anyhow::ensure!(
            TypeId::of::<T>() != TypeId::of::<cel_runtime::OwnedValue>(),
            "whole-value owners require an explicit representation witness"
        );
        Ok(Self::new(type_name, ValueType::leaf::<T>()))
    }

    /// Creates one resolved leaf from a user-facing type name and runtime element descriptor.
    ///
    /// The descriptor adopts `type_name`, so a runtime mismatch diagnostic against this leaf
    /// reports the name the source annotation (or the host's registry) uses rather than
    /// [`ValueType::leaf`]'s Rust type path.
    ///
    /// - Postcondition: the returned leaf's [`type_id`](Self::type_id) matches `element_type`,
    ///   and its [`element_type`](Self::element_type) reports `type_name`.
    /// - Precondition: `element_type` describes a concrete opaque leaf.
    /// - Complexity: O(l) for name length l.
    ///
    /// # Examples
    /// ```
    /// let leaf = cel_parser::ResolvedLeafType::new("Integer", cel_runtime::ValueType::leaf::<i32>());
    /// assert_eq!(leaf.type_name(), "Integer");
    /// ```
    #[must_use]
    pub fn new(type_name: impl Into<Cow<'static, str>>, element_type: ValueType) -> Self {
        let type_name = type_name.into();
        debug_assert!(matches!(
            element_type.kind(),
            cel_runtime::ValueKind::Leaf(_)
        ));
        Self {
            type_id: element_type.type_id(),
            complete: element_type.with_type_name(type_name.clone()),
            type_name,
        }
    }

    /// Attaches a complete leaf descriptor built while its concrete type was known.
    ///
    /// Optional capabilities are retained unchanged and independently; attaching
    /// metadata emits no interpreter instructions.
    ///
    /// # Errors
    ///
    /// Returns an error if either descriptor is structural or concrete identity
    /// or layout differs.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_parser::ResolvedLeafType;
    /// use cel_runtime::value_type::ValueType;
    /// let leaf = ResolvedLeafType::leaf::<String>("Text").unwrap()
    ///     .with_complete_value_type(ValueType::cloneable_leaf::<String>()).unwrap();
    /// assert!(leaf.complete_value_type().is_ok());
    /// ```
    pub fn with_complete_value_type(
        mut self,
        complete: cel_runtime::value_type::ValueType,
    ) -> anyhow::Result<Self> {
        complete.validate_opaque_leaf(self.type_id)?;
        anyhow::ensure!(
            complete.layout().size() == self.complete.size()
                && complete.layout().align() == self.complete.align(),
            "complete leaf layout mismatch"
        );
        self.complete = complete.with_type_name(self.type_name.clone());
        Ok(self)
    }

    /// Borrows complete registration-time metadata without inventing erased adapters.
    ///
    /// # Errors
    ///
    /// Rejects an erased whole-value owner, which cannot be a registered leaf.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_parser::ResolvedLeafType;
    /// assert!(ResolvedLeafType::leaf::<()>("HostUnit").unwrap()
    ///     .complete_value_type().is_ok());
    /// ```
    pub fn complete_value_type(&self) -> anyhow::Result<&cel_runtime::value_type::ValueType> {
        anyhow::ensure!(
            self.type_id != TypeId::of::<cel_runtime::OwnedValue>(),
            "whole-value owners require an explicit representation witness"
        );
        Ok(&self.complete)
    }

    /// Returns this leaf type's concrete Rust [`TypeId`].
    #[must_use]
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Returns this leaf type's human-readable name.
    #[must_use]
    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    /// Returns complete canonical metadata for this leaf.
    #[must_use]
    pub fn element_type(&self) -> &ValueType {
        &self.complete
    }
}

/// A resolved recursive CEL type expression.
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedType {
    /// A resolved scalar leaf.
    Scalar(ResolvedLeafType),
    /// A resolved array type.
    Array {
        /// The resolved element type.
        element: Box<ResolvedType>,
    },
    /// A resolved tuple type.
    Tuple {
        /// The resolved tuple element types, in source order.
        elements: Vec<ResolvedType>,
    },
    /// A callable's source-ordered parameters and single result.
    Callable {
        /// The complete parameter types.
        parameters: Vec<ResolvedType>,
        /// The complete result type.
        result: Box<ResolvedType>,
    },
}

impl From<ResolvedLeafType> for ResolvedType {
    /// Promotes a registered leaf without changing its identity.
    fn from(leaf: ResolvedLeafType) -> Self {
        Self::Scalar(leaf)
    }
}

impl ResolvedType {
    /// Consumes a complete source type and returns its canonical runtime descriptor.
    ///
    /// # Errors
    /// Returns an error for unavailable leaf factories or unrepresentable tuple layouts.
    ///
    /// - Complexity: O(n) in recursive type nodes.
    ///
    /// # Examples
    /// ```
    /// use cel_parser::ResolvedType;
    /// assert!(ResolvedType::Tuple { elements: vec![] }.into_value_type().is_ok());
    /// ```
    pub fn into_value_type(self) -> anyhow::Result<cel_runtime::ValueType> {
        self.complete_value_type()
    }

    /// Builds canonical recursive metadata for interpreter storage.
    ///
    /// Empty tuples describe source unit, not an opaque host `()`.
    ///
    /// # Errors
    ///
    /// Returns an error for missing leaf factories or an unrepresentable tuple layout.
    ///
    /// - Complexity: O(n) time and output space for n recursive type nodes.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_parser::ResolvedType;
    /// use cel_runtime::value_type::{Nil, ValueRepr};
    /// assert!(ResolvedType::Tuple { elements: vec![] }
    ///     .complete_value_type().unwrap().same_shape(&Nil::value_type()));
    /// ```
    pub fn complete_value_type(&self) -> anyhow::Result<cel_runtime::value_type::ValueType> {
        use cel_runtime::value_type::{AssociatedType, ValueType};
        match self {
            Self::Scalar(leaf) => Ok(leaf.complete_value_type()?.clone()),
            Self::Array { element } => Ok(ValueType::array(element.complete_value_type()?)),
            Self::Tuple { elements } => ValueType::tuple(
                elements
                    .iter()
                    .map(|element| {
                        Ok(AssociatedType {
                            offset: 0,
                            value_type: element.complete_value_type()?,
                        })
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?,
            ),
            Self::Callable { parameters, result } => Ok(ValueType::callable(
                parameters
                    .iter()
                    .map(Self::complete_value_type)
                    .collect::<anyhow::Result<_>>()?,
                result.complete_value_type()?,
            )),
        }
    }

    /// Returns a human-readable name for this resolved type.
    ///
    /// - Complexity: O(n) in the number of nodes in this type tree.
    #[must_use]
    pub fn display_name(&self) -> String {
        match self {
            ResolvedType::Scalar(leaf) => leaf.type_name().to_string(),
            ResolvedType::Array { element } => format!("[{}]", element.display_name()),
            ResolvedType::Tuple { elements } => {
                let parts: Vec<String> = elements.iter().map(Self::display_name).collect();
                format!(
                    "({}{})",
                    parts.join(", "),
                    if parts.len() == 1 { "," } else { "" }
                )
            }
            Self::Callable { parameters, result } => format!(
                "Fn(({}{}), {})",
                parameters
                    .iter()
                    .map(Self::display_name)
                    .collect::<Vec<_>>()
                    .join(", "),
                if parameters.len() == 1 { "," } else { "" },
                result.display_name()
            ),
        }
    }
}

/// The resolved descriptor for one complete array type annotation.
#[derive(Clone, Debug)]
pub struct ResolvedArrayType {
    element_type: cel_runtime::ValueType,
}

impl PartialEq for ResolvedArrayType {
    /// Compares recursive identities independently of optional capabilities.
    ///
    /// - Complexity: O(n) in descriptor nodes.
    fn eq(&self, other: &Self) -> bool {
        self.element_type.same_shape(&other.element_type)
    }
}

impl ResolvedArrayType {
    /// Creates a resolved array type from its already-resolved element descriptor.
    ///
    /// - Precondition: `element_type` retains a concrete canonical factory.
    ///
    /// - Postcondition: [`element_type`](Self::element_type) returns `element_type` unchanged, so
    ///   the result describes an array whose elements have exactly that type.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::ResolvedArrayType;
    /// use cel_runtime::ValueType;
    ///
    /// let array = ResolvedArrayType::from_element_type(ValueType::leaf::<i32>());
    /// assert_eq!(array.element_type().type_name(), "i32");
    /// ```
    #[must_use]
    pub fn from_element_type(element_type: ValueType) -> Self {
        Self { element_type }
    }

    /// Creates a resolved array type from a fully resolved recursive type expression.
    ///
    /// # Errors
    ///
    /// Returns `Err` if `resolved` is not an array type or its layout is unrepresentable.
    pub fn from_resolved_type(resolved: ResolvedType, span: ExprSpan) -> Result<Self> {
        match resolved {
            ResolvedType::Array { element } => Ok(Self {
                element_type: element.into_value_type().map_err(|error| {
                    ParseError::new_range(error.to_string(), span.start, span.end)
                })?,
            }),
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

    /// Returns the recursive element descriptor this array annotation resolves to.
    #[must_use]
    pub fn element_type(&self) -> &cel_runtime::ValueType {
        &self.element_type
    }

    /// Returns the recursive element descriptor, consuming `self`.
    #[must_use]
    pub fn into_element_type(self) -> cel_runtime::ValueType {
        self.element_type
    }
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;
    use std::str::FromStr;

    use cel_runtime::ValueType;
    use proc_macro2::TokenStream;

    use crate::{CELParser, OpLookup};

    use super::{ResolvedLeafType, ResolvedType, TypeExpr, TypeResolver};

    #[derive(Clone)]
    struct RegistryType;

    /// Registers foreign leaves without orphan impls or mandatory capabilities.
    #[test]
    fn complete_factory_foreign_leaf_preserves_optional_capabilities() {
        use cel_runtime::{
            owned_value::OwnedValue,
            value_type::{Leaf, ValueType},
        };
        let leaf = ResolvedLeafType::leaf::<std::path::PathBuf>("Path").unwrap();
        let descriptor = leaf.complete_value_type().unwrap().clone();
        let owner = OwnedValue::new::<Leaf<std::path::PathBuf>>("path".into())
            .with_value_type(descriptor)
            .unwrap();
        assert!(owner.try_clone().is_err());
        assert!(owner.try_eq(&owner).is_err());
        assert!(owner.try_debug().is_err());
        let descriptor = ValueType::leaf::<std::path::PathBuf>()
            .with_equality::<Leaf<std::path::PathBuf>>()
            .unwrap();
        let leaf = leaf.with_complete_value_type(descriptor).unwrap();
        let owner = owner
            .with_value_type(leaf.complete_value_type().unwrap().clone())
            .unwrap();
        assert!(owner.try_eq(&owner).unwrap());
        assert!(owner.try_clone().is_err());
        assert!(owner.try_debug().is_err());
        assert!(
            leaf.with_complete_value_type(ValueType::leaf::<String>())
                .is_err()
        );
        for (descriptor, cloneable, comparable, debuggable) in [
            (
                ValueType::cloneable_leaf::<std::path::PathBuf>(),
                true,
                false,
                false,
            ),
            (
                ValueType::leaf::<std::path::PathBuf>()
                    .with_debug::<Leaf<std::path::PathBuf>>()
                    .unwrap(),
                false,
                false,
                true,
            ),
        ] {
            let leaf = ResolvedLeafType::leaf::<std::path::PathBuf>("Path")
                .unwrap()
                .with_complete_value_type(descriptor)
                .unwrap();
            let owner = OwnedValue::new::<Leaf<std::path::PathBuf>>("path".into())
                .with_value_type(leaf.complete_value_type().unwrap().clone())
                .unwrap();
            assert_eq!(owner.try_clone().is_ok(), cloneable);
            assert_eq!(owner.try_eq(&owner).is_ok(), comparable);
            assert_eq!(owner.try_debug().is_ok(), debuggable);
        }
        assert!(ResolvedLeafType::leaf::<cel_runtime::OwnedValue>("Owner").is_err());
        assert!(
            ResolvedLeafType::leaf::<i32>("Integer")
                .unwrap()
                .with_complete_value_type(ValueType::tuple(vec![]).unwrap())
                .is_err()
        );
        assert!(
            ResolvedLeafType::leaf::<i32>("Integer")
                .unwrap()
                .with_complete_value_type(ValueType::leaf::<u32>())
                .is_err()
        );
    }

    /// Stores and moves leaves that implement none of the optional capability traits.
    #[test]
    fn complete_factory_does_not_require_capability_traits() {
        use cel_runtime::{owned_value::OwnedValue, value_type::Leaf};
        /// Owns one move-only payload without clone, equality, or formatting.
        struct MoveOnly(String);
        let leaf = ResolvedLeafType::leaf::<MoveOnly>("MoveOnly").unwrap();
        let owner = OwnedValue::new::<Leaf<MoveOnly>>(MoveOnly(String::from("owned")))
            .with_value_type(leaf.complete_value_type().unwrap().clone())
            .unwrap();
        assert!(owner.try_clone().is_err());
        assert!(owner.try_eq(&owner).is_err());
        assert!(owner.try_debug().is_err());
        assert_eq!(owner.try_into_typed::<Leaf<MoveOnly>>().unwrap().0, "owned");
    }

    /// Composes complete array-of-tuple metadata with nested arrays.
    #[test]
    fn complete_factory_resolved_recursive_shapes_retain_fields() {
        use cel_runtime::value_type::{ArrayOf, Cons, Leaf, Nil, ValueRepr};
        let resolved = ResolvedType::Array {
            element: Box::new(ResolvedType::Tuple {
                elements: vec![
                    ResolvedType::Scalar(ResolvedLeafType::leaf::<i32>("Integer").unwrap()),
                    ResolvedType::Array {
                        element: Box::new(ResolvedType::Scalar(
                            ResolvedLeafType::leaf::<String>("Text").unwrap(),
                        )),
                    },
                ],
            }),
        };
        assert!(
            resolved
                .complete_value_type()
                .unwrap()
                .same_shape(
                    &ArrayOf::<Cons<Leaf<i32>, Cons<ArrayOf<Leaf<String>>, Nil>>>::value_type()
                )
        );
        let unresolved = ResolvedType::Array {
            element: Box::new(ResolvedType::Scalar(ResolvedLeafType::new(
                "Erased",
                ValueType::leaf::<i32>(),
            ))),
        };
        assert!(unresolved.complete_value_type().is_ok());
    }

    /// Keeps source unit distinct from an explicitly registered opaque host unit.
    #[test]
    fn complete_factory_resolved_source_unit_is_not_host_unit() {
        use cel_runtime::value_type::{Nil, ValueRepr};
        let host = ResolvedLeafType::leaf::<()>("HostUnit").unwrap();
        let source = ResolvedType::Tuple { elements: vec![] };
        let complete = source.complete_value_type().unwrap();
        assert!(complete.same_shape(&Nil::value_type()));
        assert!(!complete.same_shape(host.complete_value_type().unwrap()));
        assert!(
            ResolvedLeafType::new("old", ValueType::leaf::<i32>())
                .complete_value_type()
                .is_ok()
        );
    }

    /// Built-ins retain cloning, equality, and formatting in complete metadata.
    #[test]
    fn complete_factory_builtin_capabilities_are_available() {
        use cel_runtime::{owned_value::OwnedValue, value_type::Leaf};
        let leaf = super::resolve_builtin_named_type("i32", &[]).unwrap();
        let owner = OwnedValue::new::<Leaf<i32>>(7)
            .with_value_type(leaf.complete_value_type().unwrap().clone())
            .unwrap();
        assert_eq!(
            owner
                .try_clone()
                .unwrap()
                .try_into_typed::<Leaf<i32>>()
                .unwrap(),
            7
        );
        assert!(owner.try_eq(&owner).unwrap());
        assert_eq!(owner.try_debug().unwrap(), "7");
        for (name, args) in [
            ("RangeFull", vec![]),
            ("RangeInclusive", vec![ResolvedType::Scalar(leaf)]),
        ] {
            assert!(
                super::resolve_builtin_named_type(name, &args)
                    .unwrap()
                    .complete_value_type()
                    .is_ok()
            );
        }
    }

    #[test]
    fn parse_named_type_expr_preserves_its_name_and_span() {
        let mut parser = CELParser::new(OpLookup::new());
        let expr = parser.parse_type_expr_str("i32").unwrap();

        match expr {
            TypeExpr::Named { name, args, span } => {
                assert_eq!(name, "i32");
                assert!(args.is_empty());
                assert_eq!(span.start.source_text().as_deref(), Some("i32"));
                assert_eq!(span.end.source_text().as_deref(), Some("i32"));
            }
            other => panic!("expected a named type expression, got {other:?}"),
        }
    }

    #[test]
    fn parse_type_expr_consumes_one_prefix_and_leaves_following_tokens() {
        let mut parser = CELParser::new(OpLookup::new());
        let input = TokenStream::from_str("[i32] [f64]").unwrap();
        parser.set_tokens(input.into_iter());

        let first = parser.parse_type_expr().unwrap();
        let second = parser.parse_type_expr().unwrap();

        match first {
            TypeExpr::Array { element, .. } => match *element {
                TypeExpr::Named { name, .. } => assert_eq!(name, "i32"),
                other => panic!("expected the first element type to be named, got {other:?}"),
            },
            other => panic!("expected the first prefix to be an array type, got {other:?}"),
        }

        match second {
            TypeExpr::Array { element, .. } => match *element {
                TypeExpr::Named { name, .. } => assert_eq!(name, "f64"),
                other => panic!("expected the second element type to be named, got {other:?}"),
            },
            other => panic!("expected the second prefix to be an array type, got {other:?}"),
        }
    }

    #[test]
    fn parse_type_expr_str_still_rejects_trailing_tokens() {
        let mut parser = CELParser::new(OpLookup::new());
        let err = parser
            .parse_type_expr_str("[i32] [f64]")
            .expect_err("whole-input helpers must reject trailing tokens");

        assert!(
            err.message().contains("unexpected token"),
            "got: {}",
            err.message()
        );
    }

    #[test]
    fn parse_recursive_array_type_exprs() {
        let mut parser = CELParser::new(OpLookup::new());
        let expr = parser.parse_type_expr_str("[[i32]]").unwrap();

        match expr {
            TypeExpr::Array { element, span } => {
                assert_eq!(span.start.source_text().as_deref(), Some("[[i32]]"));
                assert_eq!(span.end.source_text().as_deref(), Some("[[i32]]"));
                match *element {
                    TypeExpr::Array { element, .. } => match *element {
                        TypeExpr::Named { name, .. } => assert_eq!(name, "i32"),
                        other => panic!("expected the inner-most type to be named, got {other:?}"),
                    },
                    other => panic!("expected a nested array type, got {other:?}"),
                }
            }
            other => panic!("expected an array type expression, got {other:?}"),
        }
    }

    #[test]
    fn parse_tuple_type_exprs_and_reuse_them_inside_array_syntax() {
        let mut parser = CELParser::new(OpLookup::new());
        let grouped = parser.parse_type_expr_str("([i32])").unwrap();
        let tuple = parser.parse_type_expr_str("(i32, f64)").unwrap();
        let array = parser.parse_type_expr_str("[(i32, f64)]").unwrap();

        match grouped {
            TypeExpr::Array { span, .. } => {
                assert_eq!(span.start.source_text().as_deref(), Some("([i32])"));
                assert_eq!(span.end.source_text().as_deref(), Some("([i32])"));
            }
            other => panic!("expected grouped array syntax to stay an array type, got {other:?}"),
        }

        match tuple {
            TypeExpr::Tuple { elements, span } => {
                assert_eq!(span.start.source_text().as_deref(), Some("(i32, f64)"));
                assert_eq!(span.end.source_text().as_deref(), Some("(i32, f64)"));
                assert_eq!(elements.len(), 2);
            }
            other => panic!("expected a tuple type expression, got {other:?}"),
        }

        match array {
            TypeExpr::Array { element, .. } => match *element {
                TypeExpr::Tuple { elements, .. } => assert_eq!(elements.len(), 2),
                other => panic!("expected tuple syntax inside an array type, got {other:?}"),
            },
            other => panic!("expected an array type expression, got {other:?}"),
        }
    }

    #[test]
    fn parse_type_expr_reports_missing_closing_delimiters() {
        let mut parser = CELParser::new(OpLookup::new());
        let err = parser
            .parse_type_expr_str("[(i32, f64)")
            .expect_err("a missing closing bracket must fail");

        assert!(
            err.message().contains("closing") || err.message().contains("matching `]`"),
            "got: {}",
            err.message()
        );
    }

    #[test]
    fn resolve_unknown_named_type_expr_reports_the_leaf_name() {
        let mut parser = CELParser::new(OpLookup::new());
        let expr = parser.parse_type_expr_str("[does_not_exist]").unwrap();
        let err = parser
            .resolve_type_expr(&expr)
            .expect_err("unknown names must fail through the resolver");

        assert!(
            err.message().contains("unknown type `does_not_exist`"),
            "got: {}",
            err.message()
        );
    }

    #[test]
    fn resolve_type_expr_uses_a_custom_registry_resolver_recursively() {
        let resolver = [(
            "RegistryType",
            ResolvedLeafType::new("RegistryType", ValueType::leaf::<RegistryType>()),
        )];
        let mut parser = CELParser::with_type_resolver(OpLookup::new(), resolver);
        let expr = parser.parse_type_expr_str("[[RegistryType]]").unwrap();
        let resolved = parser.resolve_type_expr(&expr).unwrap();

        match resolved {
            ResolvedType::Array { element } => match *element {
                ResolvedType::Array { element } => match *element {
                    ResolvedType::Scalar(leaf) => {
                        assert_eq!(leaf.type_id(), TypeId::of::<RegistryType>());
                        assert_eq!(leaf.type_name(), "RegistryType");
                    }
                    other => panic!("expected the nested leaf to resolve, got {other:?}"),
                },
                other => panic!("expected a nested array resolution, got {other:?}"),
            },
            other => panic!("expected an array resolution, got {other:?}"),
        }
    }

    #[test]
    fn resolve_type_expr_passes_resolved_args_to_the_resolver() {
        struct GenericResolver;
        impl TypeResolver for GenericResolver {
            fn resolve_named_type(
                &self,
                name: &str,
                args: &[ResolvedType],
            ) -> Option<ResolvedLeafType> {
                match (name, args) {
                    ("Wrapper", [ResolvedType::Scalar(inner)]) => Some(ResolvedLeafType::new(
                        format!("Wrapper({})", inner.type_name()),
                        ValueType::leaf::<i32>(),
                    )),
                    ("i32", []) => Some(ResolvedLeafType::new("i32", ValueType::leaf::<i32>())),
                    _ => None,
                }
            }
        }

        let mut parser = CELParser::with_type_resolver(OpLookup::new(), GenericResolver);
        let expr = parser.parse_type_expr_str("Wrapper(i32)").unwrap();
        let resolved = parser.resolve_type_expr(&expr).unwrap();

        match resolved {
            ResolvedType::Scalar(leaf) => assert_eq!(leaf.type_name(), "Wrapper(i32)"),
            other => panic!("expected a resolved scalar, got {other:?}"),
        }
    }

    #[test]
    fn resolve_builtin_generic_type_expr_end_to_end() {
        let mut parser = CELParser::new(OpLookup::new());
        let expr = parser.parse_type_expr_str("RangeInclusive(f64)").unwrap();
        let resolved = parser.resolve_type_expr(&expr).unwrap();

        match resolved {
            ResolvedType::Scalar(leaf) => {
                assert_eq!(leaf.type_name(), "RangeInclusive(f64)");
                assert_eq!(
                    leaf.type_id(),
                    TypeId::of::<std::ops::RangeInclusive<f64>>()
                );
            }
            other => panic!("expected a resolved scalar, got {other:?}"),
        }
    }

    #[test]
    fn resolve_builtin_generic_type_as_array_element() {
        let mut parser = CELParser::new(OpLookup::new());
        let expr = parser.parse_type_expr_str("[RangeInclusive(f64)]").unwrap();
        let resolved = parser.resolve_type_expr(&expr).unwrap();
        let array = crate::ResolvedArrayType::from_resolved_type(resolved, expr.span()).unwrap();

        assert_eq!(
            array.element_type().type_id(),
            TypeId::of::<std::ops::RangeInclusive<f64>>()
        );
    }

    #[test]
    fn resolve_range_full_generic_type_expr() {
        let mut parser = CELParser::new(OpLookup::new());
        let expr = parser.parse_type_expr_str("RangeFull()").unwrap();
        let resolved = parser.resolve_type_expr(&expr).unwrap();

        match resolved {
            ResolvedType::Scalar(leaf) => assert_eq!(leaf.type_name(), "RangeFull"),
            other => panic!("expected a resolved scalar, got {other:?}"),
        }
    }

    #[test]
    fn resolve_type_expr_reports_unknown_type_when_args_are_unrecognized() {
        let mut parser = CELParser::new(OpLookup::new());
        let expr = parser.parse_type_expr_str("Wrapper(f64)").unwrap();
        let err = parser
            .resolve_type_expr(&expr)
            .expect_err("no resolver registers Wrapper as a built-in generic type");

        assert!(
            err.message().contains("unknown type `Wrapper`"),
            "got: {}",
            err.message()
        );
    }

    #[test]
    fn parse_named_type_expr_with_one_type_argument() {
        let mut parser = CELParser::new(OpLookup::new());
        let expr = parser.parse_type_expr_str("RangeInclusive(f64)").unwrap();

        match expr {
            TypeExpr::Named { name, args, span } => {
                assert_eq!(name, "RangeInclusive");
                assert_eq!(span.start.source_text().as_deref(), Some("RangeInclusive"));
                assert_eq!(span.end.source_text().as_deref(), Some("(f64)"));
                assert_eq!(args.len(), 1);
                match &args[0] {
                    TypeExpr::Named { name, args, .. } => {
                        assert_eq!(name, "f64");
                        assert!(args.is_empty());
                    }
                    other => panic!("expected a named type argument, got {other:?}"),
                }
            }
            other => panic!("expected a named type expression, got {other:?}"),
        }
    }

    #[test]
    fn parse_named_type_expr_with_zero_type_arguments() {
        let mut parser = CELParser::new(OpLookup::new());
        let expr = parser.parse_type_expr_str("RangeFull()").unwrap();

        match expr {
            TypeExpr::Named { name, args, span } => {
                assert_eq!(name, "RangeFull");
                assert_eq!(span.end.source_text().as_deref(), Some("()"));
                assert!(args.is_empty());
            }
            other => panic!("expected a named type expression, got {other:?}"),
        }
    }

    #[test]
    fn parse_named_type_expr_with_multiple_and_nested_type_arguments() {
        let mut parser = CELParser::new(OpLookup::new());
        let expr = parser
            .parse_type_expr_str("Pair(RangeInclusive(f64), i32)")
            .unwrap();

        match expr {
            TypeExpr::Named { name, args, .. } => {
                assert_eq!(name, "Pair");
                assert_eq!(args.len(), 2);
                match &args[0] {
                    TypeExpr::Named { name, args, .. } => {
                        assert_eq!(name, "RangeInclusive");
                        assert_eq!(args.len(), 1);
                    }
                    other => panic!("expected the first argument to be named, got {other:?}"),
                }
                match &args[1] {
                    TypeExpr::Named { name, args, .. } => {
                        assert_eq!(name, "i32");
                        assert!(args.is_empty());
                    }
                    other => panic!("expected the second argument to be named, got {other:?}"),
                }
            }
            other => panic!("expected a named type expression, got {other:?}"),
        }
    }

    #[test]
    fn parse_named_type_expr_with_no_parens_still_parses_a_bare_name() {
        let mut parser = CELParser::new(OpLookup::new());
        let expr = parser.parse_type_expr_str("i32").unwrap();

        match expr {
            TypeExpr::Named { name, args, .. } => {
                assert_eq!(name, "i32");
                assert!(args.is_empty());
            }
            other => panic!("expected a named type expression, got {other:?}"),
        }
    }

    #[test]
    fn parse_named_type_expr_rejects_a_trailing_comma_in_the_argument_list() {
        let mut parser = CELParser::new(OpLookup::new());
        let err = parser
            .parse_type_expr_str("RangeInclusive(f64,)")
            .expect_err("a trailing comma is not a valid type-argument list");

        assert!(
            err.message().contains("expected a type name"),
            "got: {}",
            err.message()
        );
    }

    #[test]
    fn parse_named_type_expr_rejects_a_missing_closing_paren() {
        // `proc_macro2::TokenStream::from_str` tokenizes delimiters into a matched tree up
        // front, so a truly unclosed `(` is rejected at tokenization (a `LexError`), before
        // `parse_type_expression`'s own "expected ',' or closing ')'" check ever runs — see
        // `parse_named_type_expr_rejects_arguments_missing_a_separating_comma` for that case.
        let mut parser = CELParser::new(OpLookup::new());
        let err = parser
            .parse_type_expr_str("RangeInclusive(f64")
            .expect_err("an unclosed type-argument list must be rejected");

        assert!(
            err.message().contains("unclosed delimiter"),
            "got: {}",
            err.message()
        );
    }

    #[test]
    fn parse_named_type_expr_rejects_arguments_missing_a_separating_comma() {
        let mut parser = CELParser::new(OpLookup::new());
        let err = parser
            .parse_type_expr_str("Pair(i32 f64)")
            .expect_err("adjacent type arguments without a comma must be rejected");

        assert!(
            err.message()
                .contains("expected ',' or closing ')' in type argument list"),
            "got: {}",
            err.message()
        );
    }

    #[test]
    fn parse_named_type_expr_supports_deeply_nested_generic_arguments() {
        let mut parser = CELParser::new(OpLookup::new());
        let expr = parser
            .parse_type_expr_str("Outer(Middle(Inner(f64)))")
            .unwrap();

        match expr {
            TypeExpr::Named { name, args, .. } => {
                assert_eq!(name, "Outer");
                match &args[..] {
                    [
                        TypeExpr::Named {
                            name: middle_name,
                            args: middle_args,
                            ..
                        },
                    ] => {
                        assert_eq!(middle_name, "Middle");
                        match &middle_args[..] {
                            [
                                TypeExpr::Named {
                                    name: inner_name,
                                    args: inner_args,
                                    ..
                                },
                            ] => {
                                assert_eq!(inner_name, "Inner");
                                match &inner_args[..] {
                                    [TypeExpr::Named { name, args, .. }] => {
                                        assert_eq!(name, "f64");
                                        assert!(args.is_empty());
                                    }
                                    other => panic!("expected one named argument, got {other:?}"),
                                }
                            }
                            other => panic!("expected one named argument, got {other:?}"),
                        }
                    }
                    other => panic!("expected one named argument, got {other:?}"),
                }
            }
            other => panic!("expected a named type expression, got {other:?}"),
        }
    }

    #[test]
    fn builtin_type_resolver_resolves_scalars_and_generics() {
        let resolver = crate::builtin_type_resolver();

        let scalar = resolver.resolve_named_type("i32", &[]).unwrap();
        assert_eq!(scalar.type_name(), "i32");

        let arg = ResolvedType::Scalar(resolver.resolve_named_type("f64", &[]).unwrap());
        let generic = resolver
            .resolve_named_type("RangeInclusive", std::slice::from_ref(&arg))
            .unwrap();
        assert_eq!(generic.type_name(), "RangeInclusive(f64)");

        assert!(resolver.resolve_named_type("not_a_type", &[]).is_none());
    }

    #[test]
    fn resolve_builtin_named_type_matches_the_boxed_resolver() {
        let scalar = crate::resolve_builtin_named_type("i32", &[]).unwrap();
        assert_eq!(scalar.type_name(), "i32");

        let arg = ResolvedType::Scalar(crate::resolve_builtin_named_type("f64", &[]).unwrap());
        let generic =
            crate::resolve_builtin_named_type("RangeInclusive", std::slice::from_ref(&arg))
                .unwrap();
        assert_eq!(generic.type_name(), "RangeInclusive(f64)");

        assert!(crate::resolve_builtin_named_type("not_a_type", &[]).is_none());
    }
}
