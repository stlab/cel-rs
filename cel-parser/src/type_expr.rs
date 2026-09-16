//! Recursive CEL type expressions plus the small resolver boundary they need.
//!
//! Parsing stays in `cel-parser`, while leaf-name lookup is delegated to a [`TypeResolver`].
//! This keeps CEL independent of `adam-lang` while still allowing a host to register custom
//! types for typed arrays and future typed CEL surfaces.

use std::any::TypeId;
use std::borrow::Cow;
use std::sync::Arc;

use cel_runtime::ArrayElementType;

use crate::{ExprSpan, ParseError, Result, op_table};

/// `type_expr = identifier | "[" type_expr "]" | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".`
///
/// `()` is the empty tuple type (0 elements); `(T)` is grouping (same as bare `T`); `(T,)` is a
/// 1-element tuple; `(T, U, ...)` is n-element, no trailing comma. `[T]` names a rank-one array
/// whose elements have type `T`; nested brackets compose recursively.
#[derive(Clone, Debug)]
pub enum TypeExpr {
    /// A single type name, resolved later through a [`TypeResolver`].
    Named {
        /// The unresolved type name, exactly as written.
        name: String,
        /// The source span of the full name token.
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
    /// Returns `Err` if some named leaf is not recognized by `resolver`.
    ///
    /// - Complexity: O(n) in the number of nodes in this type tree.
    pub fn resolve(&self, resolver: &dyn TypeResolver) -> Result<ResolvedType> {
        match self {
            TypeExpr::Named { name, span } => resolver.resolve_named_type(name).map_or_else(
                || {
                    Err(ParseError::new_range(
                        format!("unknown type `{name}`"),
                        span.start,
                        span.end,
                    ))
                },
                |leaf| Ok(ResolvedType::Scalar(leaf)),
            ),
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
    /// Returns the registered leaf type named `name`, or `None` if it is unknown.
    fn resolve_named_type(&self, name: &str) -> Option<ResolvedLeafType>;
}

impl<F> TypeResolver for F
where
    F: Fn(&str) -> Option<ResolvedLeafType> + Send + Sync,
{
    fn resolve_named_type(&self, name: &str) -> Option<ResolvedLeafType> {
        self(name)
    }
}

impl<const N: usize> TypeResolver for [(&'static str, ResolvedLeafType); N] {
    fn resolve_named_type(&self, name: &str) -> Option<ResolvedLeafType> {
        self.iter()
            .find(|(registered_name, _)| *registered_name == name)
            .map(|(_, leaf)| leaf.clone())
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct BuiltinTypeResolver;

impl TypeResolver for BuiltinTypeResolver {
    fn resolve_named_type(&self, name: &str) -> Option<ResolvedLeafType> {
        let element_type = builtin_array_element_type(name)?;
        let builtin = op_table::builtin_scalar_type(name)?;
        Some(ResolvedLeafType::new(builtin.type_name, element_type))
    }
}

pub(crate) fn default_type_resolver() -> Arc<dyn TypeResolver> {
    Arc::new(BuiltinTypeResolver)
}

fn builtin_array_element_type(name: &str) -> Option<ArrayElementType> {
    Some(match name {
        "u8" => ArrayElementType::leaf::<u8>().unwrap(),
        "u16" => ArrayElementType::leaf::<u16>().unwrap(),
        "u32" => ArrayElementType::leaf::<u32>().unwrap(),
        "u64" => ArrayElementType::leaf::<u64>().unwrap(),
        "u128" => ArrayElementType::leaf::<u128>().unwrap(),
        "usize" => ArrayElementType::leaf::<usize>().unwrap(),
        "i8" => ArrayElementType::leaf::<i8>().unwrap(),
        "i16" => ArrayElementType::leaf::<i16>().unwrap(),
        "i32" => ArrayElementType::leaf::<i32>().unwrap(),
        "i64" => ArrayElementType::leaf::<i64>().unwrap(),
        "i128" => ArrayElementType::leaf::<i128>().unwrap(),
        "isize" => ArrayElementType::leaf::<isize>().unwrap(),
        "f32" => ArrayElementType::leaf::<f32>().unwrap(),
        "f64" => ArrayElementType::leaf::<f64>().unwrap(),
        "bool" => ArrayElementType::leaf::<bool>().unwrap(),
        "String" => ArrayElementType::leaf::<String>().unwrap(),
        _ => return None,
    })
}

/// One resolved scalar leaf type.
///
/// This pairs a human-readable type name with the runtime element descriptor needed to build or
/// validate a `DynamicArray` whose elements have this leaf type.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedLeafType {
    type_id: TypeId,
    type_name: Cow<'static, str>,
    element_type: ArrayElementType,
}

impl ResolvedLeafType {
    /// Creates one resolved leaf from a user-facing type name and runtime element descriptor.
    ///
    /// - Postcondition: the returned leaf's [`type_id`](Self::type_id) matches `element_type`.
    #[must_use]
    pub fn new(type_name: impl Into<Cow<'static, str>>, element_type: ArrayElementType) -> Self {
        Self {
            type_id: element_type.type_id(),
            type_name: type_name.into(),
            element_type,
        }
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

    /// Returns the runtime array element descriptor for this leaf.
    #[must_use]
    pub fn element_type(&self) -> &ArrayElementType {
        &self.element_type
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
}

impl ResolvedType {
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
                format!("({})", parts.join(", "))
            }
        }
    }

    fn into_array_element_type(self, span: ExprSpan) -> Result<ArrayElementType> {
        match self {
            ResolvedType::Scalar(leaf) => Ok(leaf.element_type),
            ResolvedType::Array { element } => {
                Ok(ArrayElementType::array_of(element.into_array_element_type(span)?))
            }
            ResolvedType::Tuple { .. } => Err(ParseError::new_range(
                "tuple-valued array elements are not supported; see https://github.com/stlab/cel-rs/issues/213"
                    .to_string(),
                span.start,
                span.end,
            )),
        }
    }
}

/// The resolved descriptor for one complete array type annotation.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedArrayType {
    element_type: ArrayElementType,
}

impl ResolvedArrayType {
    /// Creates a resolved array type from its already-resolved element descriptor.
    #[must_use]
    pub fn from_element_type(element_type: ArrayElementType) -> Self {
        Self { element_type }
    }

    /// Creates a resolved array type from a fully resolved recursive type expression.
    ///
    /// # Errors
    ///
    /// Returns `Err` if `resolved` is not an array type or if its element type is a tuple, which
    /// remains unsupported for runtime CEL arrays (issue #213).
    pub fn from_resolved_type(resolved: ResolvedType, span: ExprSpan) -> Result<Self> {
        match resolved {
            ResolvedType::Array { element } => Ok(Self {
                element_type: element.into_array_element_type(span)?,
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
    pub fn element_type(&self) -> &ArrayElementType {
        &self.element_type
    }

    /// Returns the recursive element descriptor, consuming `self`.
    #[must_use]
    pub fn into_element_type(self) -> ArrayElementType {
        self.element_type
    }
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use cel_runtime::ArrayElementType;

    use crate::{CELParser, OpLookup};

    use super::{ResolvedLeafType, ResolvedType, TypeExpr};

    #[derive(Clone)]
    struct RegistryType;

    #[test]
    fn parse_named_type_expr_preserves_its_name_and_span() {
        let mut parser = CELParser::new(OpLookup::new());
        let expr = parser.parse_type_expr_str("i32").unwrap();

        match expr {
            TypeExpr::Named { name, span } => {
                assert_eq!(name, "i32");
                assert_eq!(span.start.source_text().as_deref(), Some("i32"));
                assert_eq!(span.end.source_text().as_deref(), Some("i32"));
            }
            other => panic!("expected a named type expression, got {other:?}"),
        }
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
        let tuple = parser.parse_type_expr_str("(i32, f64)").unwrap();
        let array = parser.parse_type_expr_str("[(i32, f64)]").unwrap();

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
            ResolvedLeafType::new(
                "RegistryType",
                ArrayElementType::leaf::<RegistryType>().unwrap(),
            ),
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
}
