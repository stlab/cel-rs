//! Type registry for adam-lang cell declarations.
//!
//! [`TypeRegistry`] maps DSL type-name strings to Rust types. Each registration
//! stores type-erased function pointers covering:
//! - `push_arg_fn` — registers a [`cel_runtime::DynSegment::push_arg`] op
//! - `add_cell_fn` — creates a sheet cell from a `Box<dyn Any>` value
//! - `add_source_fn` — creates a source-kind sheet cell (via [`Sheet::add_source`]) from a
//!   `Box<dyn Any>` value
//! - `call_dyn_fn` — an explicit checked host-call convenience for registered leaves
//! - `default_fn`  — constructs a default `Box<dyn Any>` (when `Default` is available)
//!
//! Persistent compound storage owns canonical values, not external tuple wrappers
//! wrappers. Sheet construction freezes complete descriptors and raw-frame adapters once;
//! the legacy external tuple helpers below are host-boundary utilities only.
//!
//! # Example
//!
//! ```rust
//! use adam_lang::TypeRegistry;
//! use std::any::TypeId;
//!
//! let reg = TypeRegistry::new();
//! assert_eq!(reg.get("f64").unwrap().type_id, TypeId::of::<f64>());
//! ```

use std::any::{Any, TypeId};
use std::collections::HashMap;

use adam_rs::{CellId, ConditionalId, MatchExpr, RelationshipId, Sheet};
use cel_parser::{ResolvedLeafType, ResolvedType, TypeResolver};
use cel_runtime::DynSegment;

/// Identifies a declared cell recursively, independently of its erased storage carrier.
///
/// Only concrete registered leaves carry Rust `TypeId`s; compound identities are structural.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum TypeShape {
    /// A registered leaf type, by its Rust `TypeId`.
    Named(TypeId),
    /// A tuple type, recursively — an empty `Vec` for `()`.
    Tuple(Vec<TypeShape>),
    /// A canonical array whose element identity is recursive.
    Array(Box<TypeShape>),
    /// A canonical callable with source-ordered parameters and one result.
    Callable(Vec<TypeShape>, Box<TypeShape>),
}

/// Registers a `push_arg<T>(index)` op on a segment.
pub type PushArgFn = fn(&mut DynSegment, usize);

/// Adds a typed cell from a boxed value and returns its handle.
pub type AddCellFn = fn(&mut Sheet, Box<dyn Any>) -> CellId;

/// Adds a typed source cell (via [`Sheet::add_source`]) from a boxed value and returns its
/// handle.
pub type AddSourceFn = fn(&mut Sheet, Box<dyn Any>) -> CellId;

/// Executes a compiled segment with the supplied inputs and boxes the result.
pub type CallDynFn = fn(&mut DynSegment, &[&dyn Any]) -> anyhow::Result<Box<dyn Any>>;

/// Calls `Sheet::add_conditional` with the appropriate concrete type.
///
/// Each branch carries a single boxed key value and the `RelationshipId`s active for that
/// branch. The default is a list of `RelationshipId`s active when no branch key matches.
pub type AddConditionalFn = fn(
    &mut Sheet,
    MatchExpr,
    Vec<(Box<dyn Any>, Vec<RelationshipId>)>,
    Vec<RelationshipId>,
) -> Result<ConditionalId, adam_rs::Error>;

/// Per-numeric-type support for a `RangeInclusive<T>`-typed filter expression, keyed by the
/// range's own `TypeId` — populated in [`TypeRegistry::new`] for exactly the primitives
/// `cel-parser`'s `..=` operator supports (`cel_parser::op_table`'s `RANGE_INCLUSIVE_SIGNATURES`).
#[allow(clippy::type_complexity)] // `clamp_fn`/`bounds_fn` are inherently multi-argument fn pointers.
#[derive(Clone)]
pub(crate) struct RangeEntry {
    /// `T`'s own `TypeId` — compared against a filtered cell's declared type by the caller.
    pub(crate) element_type_id: TypeId,
    /// Evaluates a compiled segment producing a `RangeInclusive<T>` against `value` (the
    /// candidate being conformed — read only if the segment's expression actually references
    /// `_`) and `args` (the filter's deduced cell dependencies, in declaration order), clamping
    /// `value` into the resulting bounds.
    pub(crate) clamp_fn: fn(cel_runtime::OwnedValue, &dyn Any) -> anyhow::Result<Box<dyn Any>>,
    /// Evaluates the same kind of segment against `placeholder` (substituted for `_`, which a
    /// recognized range expression's bounds never actually depend on) and `args`, returning the
    /// resulting `(lo, hi)` bounds.
    pub(crate) bounds_fn:
        fn(cel_runtime::OwnedValue) -> anyhow::Result<(Box<dyn Any>, Box<dyn Any>)>,
}

/// Metadata for a single type registered in a [`TypeRegistry`].
#[derive(Clone)]
pub struct TypeEntry {
    /// Moves an exactly bound canonical leaf into its concrete Adam storage.
    pub(crate) box_owned: unsafe fn(cel_runtime::OwnedValue) -> Box<dyn Any>,
    /// Runtime type identity.
    pub type_id: TypeId,
    /// Rust type name for error messages.
    pub type_name: &'static str,
    /// Registers a `push_arg<T>` op at the given argument index.
    pub push_arg_fn: PushArgFn,
    /// Creates a sheet cell from a `Box<dyn Any>` holding a `T`.
    pub add_cell_fn: AddCellFn,
    /// Creates a source-kind sheet cell (via [`Sheet::add_source`]) from a `Box<dyn Any>` holding
    /// a `T`.
    pub add_source_fn: AddSourceFn,
    /// Calls `DynSegment::call_dyn::<T>` and boxes the result.
    pub call_dyn_fn: CallDynFn,
    /// Compares two type-erased values of this type for equality.
    pub eq_dyn_fn: fn(&dyn Any, &dyn Any) -> bool,
    /// Constructs a default `T` if the type implements `Default`; otherwise `None`.
    pub default_fn: Option<fn() -> Box<dyn Any>>,
    /// Calls `Sheet::add_conditional::<T>` with type-erased branch keys.
    pub add_conditional_fn: AddConditionalFn,
    /// Builds complete concrete leaf metadata with the registered capabilities.
    pub value_type_fn: fn() -> cel_runtime::value_type::ValueType,
}

/// Maps DSL type names to Rust types for adam-lang cell declarations.
///
/// # Example
///
/// ```rust
/// use adam_lang::TypeRegistry;
///
/// let mut reg = TypeRegistry::new();
/// assert!(reg.get("i32").is_some());
/// assert!(reg.get("unknown").is_none());
/// ```
#[derive(Clone)]
pub struct TypeRegistry {
    by_name: HashMap<String, TypeEntry>,
    by_type_id: HashMap<TypeId, String>,
    range_inclusive: HashMap<TypeId, RangeEntry>,
}

fn push_arg_impl<T: 'static + Clone>(segment: &mut DynSegment, index: usize) {
    segment.push_arg::<T>(index);
}

/// Moves a leaf whose exact identity was validated when preparing the adapter.
///
/// # Safety
/// `value` must have the complete representation `Leaf<T>`.
unsafe fn box_owned_impl<T: 'static>(value: cel_runtime::OwnedValue) -> Box<dyn Any> {
    Box::new(unsafe { value.into_typed_bound::<cel_runtime::Leaf<T>>() })
}

/// Calls `Sheet::add_conditional::<T>` from type-erased branch data.
///
/// - Precondition: each `Box<dyn Any>` in `branches` holds a value of type `T`.
fn add_conditional_impl<T: Any + PartialEq + 'static>(
    sheet: &mut Sheet,
    source: MatchExpr,
    branches: Vec<(Box<dyn Any>, Vec<RelationshipId>)>,
    default: Vec<RelationshipId>,
) -> Result<ConditionalId, adam_rs::Error> {
    let typed_branches: Vec<(Vec<T>, Vec<RelationshipId>)> = branches
        .into_iter()
        .map(|(val, rel_ids)| {
            let v = *val
                .downcast::<T>()
                .expect("add_conditional_impl: type matches registration");
            (vec![v], rel_ids)
        })
        .collect();
    sheet.add_conditional::<T>(source, typed_branches, default)
}

fn add_cell_impl<T: Any + PartialEq + 'static>(sheet: &mut Sheet, value: Box<dyn Any>) -> CellId {
    let v = value
        .downcast::<T>()
        .expect("add_cell_impl: type matches registration");
    sheet.add_cell(*v)
}

/// Adds `value` to `sheet` as a source-kind cell (via [`Sheet::add_source`]), for
/// `TypeEntry::add_source_fn`.
///
/// - Precondition: `value` holds a `T` matching the registration that produced this fn pointer.
fn add_source_impl<T: Any + PartialEq + 'static>(sheet: &mut Sheet, value: Box<dyn Any>) -> CellId {
    let v = value
        .downcast::<T>()
        .expect("add_source_impl: type matches registration");
    sheet.add_source(*v)
}

/// Compares two type-erased values of `T`, for `TypeEntry::eq_dyn_fn`.
///
/// A generic function monomorphized per registered `T`, with no captured state — this is
/// what lets it coerce to a bare `fn` pointer despite `T` only being known via a runtime
/// `TypeId` at the call site (exactly like `call_dyn_impl` already does for calling a
/// compiled segment).
fn eq_dyn_impl<T: PartialEq + 'static>(a: &dyn Any, b: &dyn Any) -> bool {
    a.downcast_ref::<T>() == b.downcast_ref::<T>()
}

fn call_dyn_impl<T: 'static + Clone>(
    seg: &mut DynSegment,
    inputs: &[&dyn Any],
) -> anyhow::Result<Box<dyn Any>> {
    Ok(Box::new(seg.call_dyn::<T>(inputs)?))
}

/// Builds complete leaf ownership and the capabilities required by Adam registration.
fn value_type_impl<T: Clone + PartialEq + std::fmt::Debug + 'static>()
-> cel_runtime::value_type::ValueType {
    use cel_runtime::value_type::{Leaf, ValueType};
    ValueType::cloneable_leaf::<T>()
        .with_equality::<Leaf<T>>()
        .expect("same registered leaf")
        .with_debug::<Leaf<T>>()
        .expect("same registered leaf")
}

/// Evaluates `seg` (producing a `RangeInclusive<T>`) against `value` prepended to `args`, then
/// clamps `value` into the resulting bounds. For [`RangeEntry::clamp_fn`].
fn range_clamp_impl<T: Clone + PartialOrd + 'static>(
    result: cel_runtime::OwnedValue,
    value: &dyn Any,
) -> anyhow::Result<Box<dyn Any>> {
    // SAFETY: The registered range and cell descriptors are bound during preparation.
    let range =
        unsafe { result.into_typed_bound::<cel_runtime::Leaf<std::ops::RangeInclusive<T>>>() };
    let v = unsafe { &*std::ptr::from_ref(value).cast::<T>() };
    let clamped = if v < range.start() {
        range.start().clone()
    } else if v > range.end() {
        range.end().clone()
    } else {
        v.clone()
    };
    Ok(Box::new(clamped) as Box<dyn Any>)
}

/// Evaluates `seg` (producing a `RangeInclusive<T>`) against `placeholder` prepended to `args`,
/// returning the resulting `(lo, hi)` bounds. For [`RangeEntry::bounds_fn`].
fn range_bounds_impl<T: Clone + 'static>(
    result: cel_runtime::OwnedValue,
) -> anyhow::Result<(Box<dyn Any>, Box<dyn Any>)> {
    // SAFETY: This callback is selected only after binding the exact range descriptor.
    let range =
        unsafe { result.into_typed_bound::<cel_runtime::Leaf<std::ops::RangeInclusive<T>>>() };
    Ok((
        Box::new(range.start().clone()) as Box<dyn Any>,
        Box::new(range.end().clone()) as Box<dyn Any>,
    ))
}

impl TypeRegistry {
    /// Retains recursive identities while installing registered leaf capabilities.
    ///
    /// # Errors
    /// Returns an error if a tuple layout is unrepresentable.
    ///
    /// - Complexity: O(n) in descriptor nodes.
    pub(crate) fn enrich_value_type(
        &self,
        ty: &cel_runtime::ValueType,
    ) -> anyhow::Result<cel_runtime::ValueType> {
        use cel_runtime::{AssociatedType, ValueKind, ValueType};
        match ty.kind() {
            ValueKind::Leaf(id) => Ok(self
                .entry_by_type_id(*id)
                .map_or_else(|| ty.clone(), |entry| (entry.value_type_fn)())),
            ValueKind::Array(element) => Ok(ValueType::array(self.enrich_value_type(element)?)),
            ValueKind::Tuple(fields) => ValueType::tuple(
                fields
                    .iter()
                    .map(|field| {
                        Ok(AssociatedType {
                            offset: 0,
                            value_type: self.enrich_value_type(&field.value_type)?,
                        })
                    })
                    .collect::<anyhow::Result<_>>()?,
            ),
            ValueKind::Callable { .. } => Ok(ty.clone()),
        }
    }

    /// Returns a registered leaf's source-facing name and complete CEL element descriptor.
    ///
    /// - Complexity: O(l) to clone the registered name of length l.
    pub(crate) fn resolved_leaf_type(&self, type_id: TypeId) -> Option<ResolvedLeafType> {
        let name = self.by_type_id.get(&type_id)?;
        let entry = self.by_name.get(name)?;
        Some(ResolvedLeafType::new(name.clone(), (entry.value_type_fn)()))
    }

    /// Builds a CEL-facing resolver snapshot from this registry's registered leaf types.
    ///
    /// The returned resolver owns only the name-to-descriptor metadata CEL needs for typed array
    /// annotations, keeping sheet construction and other Adam-specific function pointers out of
    /// `cel-parser`.
    ///
    /// - Complexity: O(n) in the number of registered leaf types.
    #[must_use]
    pub fn cel_type_resolver(&self) -> RegistryTypeResolver {
        let by_name = self
            .by_name
            .iter()
            .map(|(name, entry)| {
                (
                    name.clone(),
                    ResolvedLeafType::new(name.clone(), (entry.value_type_fn)()),
                )
            })
            .collect();
        RegistryTypeResolver { by_name }
    }

    /// Creates a registry pre-populated with all built-in CEL/Rust primitive types.
    ///
    /// Registered types: `i8`, `i16`, `i32`, `i64`, `i128`, `isize`,
    /// `u8`, `u16`, `u32`, `u64`, `u128`, `usize`, `f32`, `f64`, `bool`, `String`.
    #[must_use]
    pub fn new() -> Self {
        let mut r = TypeRegistry {
            by_name: HashMap::new(),
            by_type_id: HashMap::new(),
            range_inclusive: HashMap::new(),
        };
        r.register::<i8>("i8");
        r.register::<i16>("i16");
        r.register::<i32>("i32");
        r.register::<i64>("i64");
        r.register::<i128>("i128");
        r.register::<isize>("isize");
        r.register::<u8>("u8");
        r.register::<u16>("u16");
        r.register::<u32>("u32");
        r.register::<u64>("u64");
        r.register::<u128>("u128");
        r.register::<usize>("usize");
        r.register::<f32>("f32");
        r.register::<f64>("f64");
        r.register::<bool>("bool");
        r.register::<String>("String");
        r.register_range_inclusive::<i8>();
        r.register_range_inclusive::<i16>();
        r.register_range_inclusive::<i32>();
        r.register_range_inclusive::<i64>();
        r.register_range_inclusive::<i128>();
        r.register_range_inclusive::<isize>();
        r.register_range_inclusive::<u8>();
        r.register_range_inclusive::<u16>();
        r.register_range_inclusive::<u32>();
        r.register_range_inclusive::<u64>();
        r.register_range_inclusive::<u128>();
        r.register_range_inclusive::<usize>();
        r.register_range_inclusive::<f32>();
        r.register_range_inclusive::<f64>();
        r
    }

    /// Registers `T` under `name` with default initialization support.
    ///
    /// - Postcondition: `self.get(name)` returns `Some(entry)` with `entry.default_fn.is_some()`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use adam_lang::TypeRegistry;
    /// let mut reg = TypeRegistry::new();
    /// reg.register::<u64>("my_u64");
    /// assert!(reg.get("my_u64").is_some());
    /// ```
    pub fn register<T: Any + PartialEq + Default + Clone + std::fmt::Debug + 'static>(
        &mut self,
        name: &str,
    ) {
        let type_id = TypeId::of::<T>();
        if let Some(old) = self.by_name.get(name) {
            self.by_type_id.remove(&old.type_id);
        }
        self.by_name.insert(
            name.to_owned(),
            TypeEntry {
                box_owned: box_owned_impl::<T>,
                type_id,
                type_name: std::any::type_name::<T>(),
                push_arg_fn: push_arg_impl::<T>,
                add_cell_fn: add_cell_impl::<T>,
                add_source_fn: add_source_impl::<T>,
                call_dyn_fn: call_dyn_impl::<T>,
                eq_dyn_fn: eq_dyn_impl::<T>,
                default_fn: Some(|| Box::new(T::default()) as Box<dyn Any>),
                add_conditional_fn: add_conditional_impl::<T>,
                value_type_fn: value_type_impl::<T>,
            },
        );
        self.by_type_id.insert(type_id, name.to_owned());
    }

    /// Registers `T` under `name` without default initialization support.
    ///
    /// A cell declared as `cell x: T;` (no initializer) is a parse error for this type.
    ///
    /// - Postcondition: `self.get(name)` returns `Some(entry)` with `entry.default_fn.is_none()`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use adam_lang::TypeRegistry;
    /// #[derive(PartialEq, Clone, Debug)]
    /// struct MyType(i32);
    /// let mut reg = TypeRegistry::new();
    /// reg.register_no_default::<MyType>("MyType");
    /// let entry = reg.get("MyType").unwrap();
    /// assert!(entry.default_fn.is_none());
    /// ```
    pub fn register_no_default<T: Any + PartialEq + Clone + std::fmt::Debug + 'static>(
        &mut self,
        name: &str,
    ) {
        let type_id = TypeId::of::<T>();
        if let Some(old) = self.by_name.get(name) {
            self.by_type_id.remove(&old.type_id);
        }
        self.by_name.insert(
            name.to_owned(),
            TypeEntry {
                box_owned: box_owned_impl::<T>,
                type_id,
                type_name: std::any::type_name::<T>(),
                push_arg_fn: push_arg_impl::<T>,
                add_cell_fn: add_cell_impl::<T>,
                add_source_fn: add_source_impl::<T>,
                call_dyn_fn: call_dyn_impl::<T>,
                eq_dyn_fn: eq_dyn_impl::<T>,
                default_fn: None,
                add_conditional_fn: add_conditional_impl::<T>,
                value_type_fn: value_type_impl::<T>,
            },
        );
        self.by_type_id.insert(type_id, name.to_owned());
    }

    /// Looks up a type entry by its DSL name.
    ///
    /// Returns `None` if `name` has not been registered.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use adam_lang::TypeRegistry;
    /// let reg = TypeRegistry::new();
    /// assert!(reg.get("f64").is_some());
    /// assert!(reg.get("nonexistent").is_none());
    /// ```
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&TypeEntry> {
        self.by_name.get(name)
    }

    /// Looks up a type entry by its `TypeId`.
    ///
    /// Returns `None` if no type with this `TypeId` has been registered.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use adam_lang::TypeRegistry;
    /// use std::any::TypeId;
    /// let reg = TypeRegistry::new();
    /// assert!(reg.entry_by_type_id(TypeId::of::<f64>()).is_some());
    /// assert!(reg.entry_by_type_id(TypeId::of::<Vec<u8>>()).is_none());
    /// ```
    #[must_use]
    pub fn entry_by_type_id(&self, type_id: TypeId) -> Option<&TypeEntry> {
        let name = self.by_type_id.get(&type_id)?;
        self.by_name.get(name)
    }

    /// Registers `RangeInclusive<T>` support for `T`, keyed by `RangeInclusive<T>`'s own
    /// `TypeId`. Private — only called from [`TypeRegistry::new`] for the fixed set of numeric
    /// primitives `cel-parser`'s `..=` operator supports; unlike [`TypeRegistry::register`], this
    /// is not part of the public API a host binary extends for its own custom types, since
    /// `RangeInclusive<T>` recognition is not extensible per-type in this codebase's current
    /// design.
    fn register_range_inclusive<T: Clone + PartialOrd + 'static>(&mut self) {
        self.range_inclusive.insert(
            TypeId::of::<std::ops::RangeInclusive<T>>(),
            RangeEntry {
                element_type_id: TypeId::of::<T>(),
                clamp_fn: range_clamp_impl::<T>,
                bounds_fn: range_bounds_impl::<T>,
            },
        );
    }

    /// Looks up `RangeInclusive<T>` support by the range's own `TypeId`.
    ///
    /// Returns `None` if `range_type_id` is not `RangeInclusive<T>` for any `T` this registry
    /// recognizes range support for (see [`TypeRegistry::new`]).
    #[allow(dead_code)] // Used by Task 4 (parser.rs)
    pub(crate) fn range_entry(&self, range_type_id: TypeId) -> Option<&RangeEntry> {
        self.range_inclusive.get(&range_type_id)
    }

    /// Resolves a parsed `type_expr` against this registry, recursively.
    ///
    /// # Errors
    /// Returns an unknown leaf name, malformed callable signature, or unrepresentable layout.
    /// Registered generic leaves use their complete source name, e.g. `RangeInclusive(i32)`.
    ///
    /// - Complexity: O(n) in type nodes and registered-name bytes.
    ///
    /// # Examples
    /// ```
    /// let registry = adam_lang::TypeRegistry::new();
    /// let ty = cel_parser::CELParser::new(cel_parser::OpLookup::new())
    ///     .parse_type_expr_str("[(i32, [f64])]").unwrap();
    /// assert_eq!(registry.display_name(&registry.resolve(&ty).unwrap()), "[(i32, [f64])]");
    /// ```
    pub fn resolve(
        &self,
        expr: &cel_parser::TypeExpr,
    ) -> std::result::Result<TypeShape, (String, proc_macro2::Span)> {
        let resolved = expr
            .resolve(self)
            .map_err(|error| (error.message().to_string(), error.span()))?;
        self.shape_of(
            &resolved
                .into_value_type()
                .map_err(|error| (error.to_string(), expr.span().start))?,
        )
        .map_err(|error| (error, expr.span().start))
    }

    /// Returns the DSL name `type_id` was registered under, or `None` if it is unregistered.
    ///
    /// This is the name every diagnostic should use for a live value's type: a `TypeEntry`'s own
    /// `type_name` is the Rust type path (`alloc::string::String`), not the registered name.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use adam_lang::TypeRegistry;
    /// let reg = TypeRegistry::new();
    /// assert_eq!(reg.registered_name(std::any::TypeId::of::<String>()), Some("String"));
    /// assert_eq!(reg.registered_name(std::any::TypeId::of::<Vec<u8>>()), None);
    /// ```
    #[must_use]
    pub fn registered_name(&self, type_id: TypeId) -> Option<&str> {
        self.by_type_id.get(&type_id).map(String::as_str)
    }

    /// Formats `shape` recursively, e.g. `"(i32, (f64, String))"`, for error messages.
    #[must_use]
    pub fn display_name(&self, shape: &TypeShape) -> String {
        match shape {
            TypeShape::Named(type_id) => self.registered_name(*type_id).unwrap_or("?").to_string(),
            TypeShape::Tuple(elements) => {
                let parts: Vec<String> = elements.iter().map(|e| self.display_name(e)).collect();
                format!(
                    "({}{})",
                    parts.join(", "),
                    if elements.len() == 1 { "," } else { "" }
                )
            }
            TypeShape::Array(element) => format!("[{}]", self.display_name(element)),
            TypeShape::Callable(parameters, result) => format!(
                "Fn({}, {})",
                self.display_name(&TypeShape::Tuple(parameters.clone())),
                self.display_name(result)
            ),
        }
    }

    /// Builds one `AssociatedType` entry describing `shape`: a leaf's own registered layout for
    /// `TypeShape::Named`, or a nested tuple's own recursively-laid-out elements (via
    /// `cel_runtime::ValueType::tuple`) for `TypeShape::Tuple`.
    ///
    /// - Precondition: every leaf `TypeId` reachable from `shape` is registered.
    ///
    /// - Postcondition: the returned entry's `offset` is always 0; the caller
    ///   (`ValueType::tuple`) computes canonical tail-first sibling offsets.
    fn one_associated(&self, shape: &TypeShape) -> cel_runtime::AssociatedType {
        let value_type = match shape {
            TypeShape::Named(type_id) => {
                let entry = self
                    .entry_by_type_id(*type_id)
                    .expect("one_associated: type registered");
                (entry.value_type_fn)()
            }
            TypeShape::Tuple(elements) => cel_runtime::ValueType::tuple(
                elements.iter().map(|e| self.one_associated(e)).collect(),
            )
            .expect("registered tuple layout"),
            TypeShape::Array(element) => cel_runtime::ValueType::array(self.value_type(element)),
            TypeShape::Callable(parameters, result) => cel_runtime::ValueType::callable(
                parameters.iter().map(|p| self.value_type(p)).collect(),
                self.value_type(result),
            ),
        };
        cel_runtime::AssociatedType {
            offset: 0,
            value_type,
        }
    }

    /// Returns the canonical descriptor of a resolved shape.
    ///
    /// - Precondition: Every named leaf is registered.
    /// - Complexity: O(n) in shape nodes.
    pub(crate) fn value_type(&self, shape: &TypeShape) -> cel_runtime::ValueType {
        self.one_associated(shape).value_type
    }

    /// Recovers structural identity without synthesizing aggregate Rust identities.
    ///
    /// # Errors
    /// Names an unregistered leaf, including leaves nested in a signature.
    /// - Complexity: O(n) in descriptor nodes.
    pub(crate) fn shape_of(&self, ty: &cel_runtime::ValueType) -> Result<TypeShape, String> {
        use cel_runtime::ValueKind;
        Ok(match ty.kind() {
            ValueKind::Leaf(id) => {
                if self.entry_by_type_id(*id).is_none() {
                    return Err(format!(
                        "cannot infer a type: unregistered type `{}`",
                        ty.type_name()
                    ));
                }
                TypeShape::Named(*id)
            }
            ValueKind::Array(element) => TypeShape::Array(Box::new(self.shape_of(element)?)),
            ValueKind::Tuple(fields) => TypeShape::Tuple(
                fields
                    .iter()
                    .map(|field| self.shape_of(&field.value_type))
                    .collect::<Result<_, _>>()?,
            ),
            ValueKind::Callable { parameters, result } => TypeShape::Callable(
                parameters
                    .iter()
                    .map(|p| self.shape_of(p))
                    .collect::<Result<_, _>>()?,
                Box::new(self.shape_of(result)?),
            ),
        })
    }

    /// Checks default availability without constructing any values.
    ///
    /// Arrays need no element defaults; tuple fields are checked in source order.
    /// # Errors
    /// Names the first missing leaf default or callable requiring an initializer.
    /// - Complexity: O(n) in constructed shape nodes.
    pub(crate) fn validate_default(&self, shape: &TypeShape) -> anyhow::Result<()> {
        match shape {
            TypeShape::Named(id) => {
                anyhow::ensure!(
                    self.entry_by_type_id(*id)
                        .expect("resolved leaf")
                        .default_fn
                        .is_some(),
                    "type `{}` has no default; provide `= ...`",
                    self.display_name(shape)
                );
            }
            TypeShape::Tuple(fields) => {
                for field in fields {
                    self.validate_default(field)?;
                }
            }
            TypeShape::Array(_) => {}
            TypeShape::Callable(_, _) => {
                anyhow::bail!("callable has no default; provide an initializer `= ...`")
            }
        }
        Ok(())
    }

    /// Emits recursive defaults, leaving arrays empty without constructing their elements.
    ///
    /// Cell builders preflight default availability and cell capabilities for the entire
    /// descriptor before calling this effectful emitter.
    ///
    /// # Errors
    /// Names the precise missing default leaf; callables require an initializer.
    /// - Complexity: O(n) in constructed shape nodes.
    pub(crate) fn emit_default(
        &self,
        shape: &TypeShape,
        body: &mut DynSegment,
    ) -> anyhow::Result<()> {
        let origin = body.current_stack_offset();
        match shape {
            TypeShape::Named(id) => {
                let entry = self.entry_by_type_id(*id).expect("resolved leaf");
                let default = entry.default_fn.ok_or_else(|| {
                    anyhow::anyhow!(
                        "type `{}` has no default; provide `= ...`",
                        self.display_name(shape)
                    )
                })?;
                body.just_external(self.value_type(shape), default())?;
            }
            TypeShape::Tuple(fields) => {
                for field in fields {
                    self.emit_default(field, body)?;
                }
                body.make_tuple(fields.len(), origin)?;
            }
            TypeShape::Array(element) => {
                body.make_typed_array(0, origin, self.value_type(element))?
            }
            TypeShape::Callable(_, _) => {
                anyhow::bail!("callable has no default; provide an initializer `= ...`")
            }
        }
        Ok(())
    }
}

/// A CEL-facing snapshot of one [`TypeRegistry`]'s registered leaf types.
#[derive(Clone, Default)]
pub struct RegistryTypeResolver {
    by_name: HashMap<String, ResolvedLeafType>,
}

impl TypeResolver for RegistryTypeResolver {
    /// Resolves registered plain or generic leaves, then CEL built-ins.
    ///
    /// - Complexity: O(n) in generic argument nodes and name bytes.
    fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType> {
        let key = named_key(name, args);
        if let Some(leaf) = self.by_name.get(key.as_ref()).cloned() {
            return Some(leaf);
        }
        cel_parser::resolve_builtin_named_type(name, args)
    }
}

impl TypeResolver for TypeRegistry {
    /// Resolves a leaf without allocating a complete registry snapshot.
    ///
    /// - Complexity: O(n) in generic argument nodes and name bytes.
    fn resolve_named_type(&self, name: &str, args: &[ResolvedType]) -> Option<ResolvedLeafType> {
        let key = named_key(name, args);
        self.get(key.as_ref())
            .map(|entry| ResolvedLeafType::new(key.into_owned(), (entry.value_type_fn)()))
            .or_else(|| cel_parser::resolve_builtin_named_type(name, args))
    }
}

/// Borrows simple names and formats only explicitly parameterized leaf lookups.
///
/// - Complexity: O(n) in generic argument nodes and name bytes.
fn named_key<'a>(name: &'a str, args: &[ResolvedType]) -> std::borrow::Cow<'a, str> {
    if args.is_empty() {
        std::borrow::Cow::Borrowed(name)
    } else {
        std::borrow::Cow::Owned(format!(
            "{name}({})",
            args.iter()
                .map(resolved_name)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

/// Formats a resolved generic argument using registered names.
///
/// - Complexity: O(n) in type nodes and name bytes.
fn resolved_name(ty: &ResolvedType) -> String {
    match ty {
        ResolvedType::Scalar(leaf) => leaf.type_name().to_string(),
        ResolvedType::Array { element } => format!("[{}]", resolved_name(element)),
        ResolvedType::Tuple { elements } => format!(
            "({}{})",
            elements
                .iter()
                .map(resolved_name)
                .collect::<Vec<_>>()
                .join(", "),
            if elements.len() == 1 { "," } else { "" }
        ),
        ResolvedType::Callable { parameters, result } => format!(
            "Fn({}, {})",
            resolved_parameters(parameters),
            resolved_name(result)
        ),
    }
}

/// Formats source-ordered callable parameters without cloning their type trees.
///
/// - Complexity: O(n) in parameter type nodes and name bytes.
fn resolved_parameters(parameters: &[ResolvedType]) -> String {
    format!(
        "({}{})",
        parameters
            .iter()
            .map(resolved_name)
            .collect::<Vec<_>>()
            .join(", "),
        if parameters.len() == 1 { "," } else { "" }
    )
}

impl Default for TypeRegistry {
    /// Returns `TypeRegistry::new()`.
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::Span;
    use std::any::TypeId;

    fn point(span: Span) -> crate::ast::ExprSpan {
        crate::ast::ExprSpan {
            start: span,
            end: span,
        }
    }

    #[test]
    fn new_registers_builtin_i32() {
        let reg = TypeRegistry::new();
        let entry = reg.get("i32").expect("i32 registered");
        assert_eq!(entry.type_id, TypeId::of::<i32>());
    }

    /// Installs complete factories while foreign concrete types are still known.
    #[test]
    fn complete_factory_registered_foreign_leaf_retains_capabilities() {
        use cel_runtime::{owned_value::OwnedValue, value_type::Leaf};
        let mut registry = TypeRegistry::new();
        registry.register::<std::path::PathBuf>("Path");
        registry.register_no_default::<std::ops::RangeInclusive<i32>>("Bounds");
        let entry = registry.get("Path").unwrap();
        let descriptor = (entry.value_type_fn)();
        let owner = OwnedValue::new::<Leaf<std::path::PathBuf>>("example".into())
            .with_value_type(descriptor)
            .unwrap();
        assert!(owner.try_eq(&owner.try_clone().unwrap()).unwrap());
        assert_eq!(owner.try_debug().unwrap(), "\"example\"");
        for name in ["Path", "Bounds"] {
            let resolved = registry
                .cel_type_resolver()
                .resolve_named_type(name, &[])
                .unwrap();
            assert_eq!(resolved.type_name(), name);
            assert!(
                resolved
                    .complete_value_type()
                    .unwrap()
                    .same_shape(&(registry.get(name).unwrap().value_type_fn)())
            );
        }
    }

    #[test]
    fn new_registers_builtin_f64_with_default() {
        let reg = TypeRegistry::new();
        let entry = reg.get("f64").expect("f64 registered");
        assert_eq!(entry.type_id, TypeId::of::<f64>());
        assert!(entry.default_fn.is_some(), "f64 must have a default");
    }

    #[test]
    fn new_registers_builtin_string() {
        let reg = TypeRegistry::new();
        let entry = reg.get("String").expect("String registered");
        assert_eq!(entry.type_id, TypeId::of::<String>());
    }

    #[test]
    fn register_custom_type_with_default() {
        let mut reg = TypeRegistry::new();
        reg.register::<u64>("my_u64");
        let entry = reg.get("my_u64").expect("custom type registered");
        assert_eq!(entry.type_id, TypeId::of::<u64>());
        assert!(entry.default_fn.is_some());
    }

    #[test]
    fn register_no_default_has_no_default_fn() {
        #[derive(PartialEq, Clone, Debug)]
        struct NoDefault(i32);

        let mut reg = TypeRegistry::new();
        reg.register_no_default::<NoDefault>("no_default");
        let entry = reg.get("no_default").expect("registered");
        assert!(entry.default_fn.is_none());
    }

    #[test]
    fn cel_type_resolver_uses_registered_leaf_names() {
        #[derive(PartialEq, Clone, Debug)]
        struct Custom(i32);

        let mut reg = TypeRegistry::new();
        reg.register_no_default::<Custom>("Custom");

        let resolver = reg.cel_type_resolver();
        let leaf = resolver
            .resolve_named_type("Custom", &[])
            .expect("custom leaf registered");

        assert_eq!(leaf.type_id(), TypeId::of::<Custom>());
        assert_eq!(leaf.type_name(), "Custom");
    }

    #[test]
    fn push_arg_fn_drives_call_dyn() {
        use cel_runtime::DynSegment;
        use std::any::Any;

        let reg = TypeRegistry::new();
        let entry = reg.get("i32").unwrap();
        let mut seg = DynSegment::new::<()>();
        (entry.push_arg_fn)(&mut seg, 0);
        let x: i32 = 7;
        let result: i32 = seg.call_dyn(&[&x as &dyn Any]).unwrap();
        assert_eq!(result, 7);
    }

    #[test]
    fn add_cell_fn_creates_cell() {
        use adam_rs::Sheet;
        use std::any::Any;

        let reg = TypeRegistry::new();
        let entry = reg.get("f64").unwrap();
        let mut sheet = Sheet::new();
        let val: Box<dyn Any> = Box::new(42.14_f64);
        let _cell_id = (entry.add_cell_fn)(&mut sheet, val);
        // Compiles and runs without panicking: add_cell_fn is callable.
    }

    #[test]
    fn call_dyn_fn_returns_boxed_result() {
        use cel_runtime::DynSegment;
        use std::any::Any;

        let reg = TypeRegistry::new();
        let entry = reg.get("i32").unwrap();
        let mut seg = DynSegment::new::<()>();
        (entry.push_arg_fn)(&mut seg, 0);
        let x: i32 = 99;
        let boxed = (entry.call_dyn_fn)(&mut seg, &[&x as &dyn Any]).unwrap();
        let result = boxed.downcast::<i32>().expect("i32");
        assert_eq!(*result, 99);
    }

    #[test]
    fn entry_by_type_id_roundtrip() {
        let reg = TypeRegistry::new();
        let entry = reg
            .entry_by_type_id(std::any::TypeId::of::<f64>())
            .expect("f64 registered");
        assert_eq!(entry.type_id, std::any::TypeId::of::<f64>());
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let reg = TypeRegistry::new();
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn entry_by_type_id_nonexistent_returns_none() {
        let reg = TypeRegistry::new();
        // Vec<u8> is not a registered built-in type.
        assert!(
            reg.entry_by_type_id(std::any::TypeId::of::<Vec<u8>>())
                .is_none()
        );
    }

    #[test]
    fn register_overwrite_removes_stale_type_id() {
        #[derive(PartialEq, Clone, Default, Debug)]
        struct TypeA;
        #[derive(PartialEq, Clone, Default, Debug)]
        struct TypeB;

        let mut reg = TypeRegistry::new();
        reg.register::<TypeA>("alias");
        reg.register::<TypeB>("alias");

        // After overwriting, TypeA's TypeId must no longer be found.
        assert!(
            reg.entry_by_type_id(TypeId::of::<TypeA>()).is_none(),
            "stale TypeA mapping should have been removed"
        );
        // TypeB must be reachable by both name and TypeId.
        assert!(reg.entry_by_type_id(TypeId::of::<TypeB>()).is_some());
        assert_eq!(reg.get("alias").unwrap().type_id, TypeId::of::<TypeB>());
    }

    #[test]
    fn register_no_default_overwrite_removes_stale_type_id() {
        #[derive(PartialEq, Clone, Debug)]
        struct TypeA;
        #[derive(PartialEq, Clone, Debug)]
        struct TypeB;

        let mut reg = TypeRegistry::new();
        reg.register_no_default::<TypeA>("alias");
        reg.register_no_default::<TypeB>("alias");

        assert!(
            reg.entry_by_type_id(TypeId::of::<TypeA>()).is_none(),
            "stale TypeA mapping should have been removed"
        );
        assert!(reg.entry_by_type_id(TypeId::of::<TypeB>()).is_some());
    }

    /// Registered leaves retain independent canonical clone, equality, and debug plans.
    #[test]
    fn registered_leaf_capabilities_operate_on_canonical_owners() {
        let reg = TypeRegistry::new();
        let entry = reg.get("i32").unwrap();
        let value = cel_runtime::OwnedValue::new::<cel_runtime::Leaf<i32>>(7)
            .with_value_type((entry.value_type_fn)())
            .unwrap();
        let cloned = value.try_clone().unwrap();
        assert!(value.try_eq(&cloned).unwrap());
        assert_eq!(cloned.try_debug().unwrap(), "7");
    }

    #[test]
    fn resolve_named_type_expr_returns_the_matching_type_shape() {
        let reg = TypeRegistry::new();
        let expr = cel_parser::TypeExpr::Named {
            name: "i32".to_string(),
            args: Vec::new(),
            span: point(Span::call_site()),
        };
        let shape = reg.resolve(&expr).unwrap();
        assert_eq!(shape, TypeShape::Named(TypeId::of::<i32>()));
    }

    #[test]
    fn resolve_unknown_named_type_expr_is_an_error() {
        let reg = TypeRegistry::new();
        let expr = cel_parser::TypeExpr::Named {
            name: "bogus".to_string(),
            args: Vec::new(),
            span: point(Span::call_site()),
        };
        assert!(reg.resolve(&expr).is_err());
    }

    #[test]
    fn resolve_tuple_type_expr_returns_a_nested_type_shape() {
        let reg = TypeRegistry::new();
        let span = point(Span::call_site());
        let expr = cel_parser::TypeExpr::Tuple {
            elements: vec![
                cel_parser::TypeExpr::Named {
                    name: "i32".to_string(),
                    args: Vec::new(),
                    span,
                },
                cel_parser::TypeExpr::Tuple {
                    elements: vec![
                        cel_parser::TypeExpr::Named {
                            name: "f64".to_string(),
                            args: Vec::new(),
                            span,
                        },
                        cel_parser::TypeExpr::Named {
                            name: "String".to_string(),
                            args: Vec::new(),
                            span,
                        },
                    ],
                    span,
                },
            ],
            span,
        };
        let shape = reg.resolve(&expr).unwrap();
        assert_eq!(
            shape,
            TypeShape::Tuple(vec![
                TypeShape::Named(TypeId::of::<i32>()),
                TypeShape::Tuple(vec![
                    TypeShape::Named(TypeId::of::<f64>()),
                    TypeShape::Named(TypeId::of::<String>()),
                ]),
            ])
        );
    }

    #[test]
    fn resolve_accepts_structural_array_cell_type() {
        let registry = TypeRegistry::new();
        let span = point(Span::call_site());
        let expr = cel_parser::TypeExpr::Array {
            element: Box::new(cel_parser::TypeExpr::Named {
                name: "i32".to_string(),
                args: Vec::new(),
                span,
            }),
            span,
        };
        assert_eq!(
            registry.resolve(&expr).unwrap(),
            TypeShape::Array(Box::new(TypeShape::Named(TypeId::of::<i32>())))
        );
    }

    #[test]
    fn resolve_accepts_registered_generic_leaf() {
        let mut registry = TypeRegistry::new();
        registry.register_no_default::<std::ops::RangeInclusive<f64>>("RangeInclusive(f64)");
        let span = point(Span::call_site());
        let expr = cel_parser::TypeExpr::Named {
            name: "RangeInclusive".to_string(),
            args: vec![cel_parser::TypeExpr::Named {
                name: "f64".to_string(),
                args: Vec::new(),
                span,
            }],
            span,
        };
        assert_eq!(
            registry.resolve(&expr).unwrap(),
            TypeShape::Named(TypeId::of::<std::ops::RangeInclusive<f64>>())
        );
    }

    /// Both resolvers retain requested names and concrete identities across alias orders.
    #[test]
    fn registered_aliases_preserve_requested_leaf_names() {
        for names in [["Integer", "i32"], ["i32", "Integer"]] {
            let mut registry = TypeRegistry::new();
            for name in names {
                registry.register::<i32>(name);
            }
            let snapshot = registry.cel_type_resolver();
            for name in names {
                let direct = registry.resolve_named_type(name, &[]).unwrap();
                let copied = snapshot.resolve_named_type(name, &[]).unwrap();
                assert_eq!(direct.type_name(), name);
                assert_eq!(copied.type_name(), name);
                assert!(
                    direct
                        .complete_value_type()
                        .unwrap()
                        .same_shape(&cel_runtime::ValueType::leaf::<i32>())
                );
                assert!(
                    direct
                        .complete_value_type()
                        .unwrap()
                        .same_shape(copied.complete_value_type().unwrap())
                );
            }
        }
    }

    /// Aliases never rewrite registered keys inside recursive generic arguments.
    #[test]
    fn registered_aliases_preserve_recursive_generic_keys() {
        use std::ops::RangeInclusive;

        for alias_first in [false, true] {
            let mut registry = TypeRegistry::new();
            if alias_first {
                registry.register::<i32>("Integer");
            }
            let names = [
                "Bounds(i32)",
                "Bounds(Integer)",
                "Bounds(Bounds(i32))",
                "Bounds([i32])",
                "Bounds((i32, [Integer]))",
                "Bounds((i32,))",
            ];
            for name in names {
                registry.register_no_default::<RangeInclusive<i32>>(name);
            }
            if !alias_first {
                registry.register::<i32>("Integer");
            }
            let snapshot = registry.cel_type_resolver();
            for name in names {
                for source in [name.to_owned(), format!("[({name}, [Integer])]")] {
                    let expr = cel_parser::CELParser::new(cel_parser::OpLookup::new())
                        .parse_type_expr_str(&source)
                        .unwrap();
                    let copied = expr.resolve(&snapshot).unwrap().into_value_type().unwrap();
                    let direct = expr.resolve(&registry).unwrap();
                    assert_eq!(resolved_name(&direct), source);
                    let direct = direct.into_value_type().unwrap();
                    assert!(direct.same_shape(&copied), "{source}");
                    assert_eq!(
                        registry.resolve(&expr).unwrap(),
                        registry.shape_of(&copied).unwrap(),
                        "{source}"
                    );
                }
            }
            let missing = cel_parser::CELParser::new(cel_parser::OpLookup::new())
                .parse_type_expr_str("Bounds([Integer])")
                .unwrap();
            let direct = missing.resolve(&registry).unwrap_err();
            let copied = missing.resolve(&snapshot).unwrap_err();
            assert_eq!(direct.message(), copied.message());
            assert!(direct.message().contains("unknown type `Bounds`"));
        }
    }

    #[test]
    fn cel_type_resolver_falls_back_to_builtin_generics() {
        let registry = TypeRegistry::new();
        let resolver = registry.cel_type_resolver();
        let f64_leaf = resolver.resolve_named_type("f64", &[]).expect("f64");
        let resolved = resolver
            .resolve_named_type("RangeInclusive", &[ResolvedType::Scalar(f64_leaf)])
            .expect("RangeInclusive(f64) should resolve via the builtin fallback");
        assert_eq!(resolved.type_name(), "RangeInclusive(f64)");
    }

    #[test]
    fn display_name_formats_a_nested_tuple_shape() {
        let reg = TypeRegistry::new();
        let shape = TypeShape::Tuple(vec![
            TypeShape::Named(TypeId::of::<i32>()),
            TypeShape::Tuple(vec![
                TypeShape::Named(TypeId::of::<f64>()),
                TypeShape::Named(TypeId::of::<String>()),
            ]),
        ]);
        assert_eq!(reg.display_name(&shape), "(i32, (f64, String))");
    }

    #[test]
    fn canonical_descriptor_describes_a_flat_tuple() {
        let reg = TypeRegistry::new();
        let shape = TypeShape::Tuple(vec![
            TypeShape::Named(TypeId::of::<i32>()),
            TypeShape::Named(TypeId::of::<f64>()),
        ]);
        let descriptor = reg.value_type(&shape);
        let prototype = descriptor.tuple_elements().unwrap();
        assert_eq!(prototype.len(), 2);
        assert_eq!(prototype[0].value_type.type_id(), TypeId::of::<i32>());
        assert_eq!(prototype[1].value_type.type_id(), TypeId::of::<f64>());
        assert_eq!(prototype[0].offset, 8);
        assert_eq!(prototype[1].offset, 0);
    }

    #[test]
    fn canonical_descriptor_describes_a_nested_tuple() {
        let reg = TypeRegistry::new();
        let shape = TypeShape::Tuple(vec![
            TypeShape::Named(TypeId::of::<i32>()),
            TypeShape::Tuple(vec![
                TypeShape::Named(TypeId::of::<i32>()),
                TypeShape::Named(TypeId::of::<i32>()),
            ]),
        ]);
        let descriptor = reg.value_type(&shape);
        let prototype = descriptor.tuple_elements().unwrap();
        assert_eq!(prototype.len(), 2);
        assert_eq!(
            prototype[1].value_type.type_id(),
            TypeId::of::<cel_runtime::CNil<()>>()
        );
        assert_eq!(
            prototype[1]
                .value_type
                .tuple_elements()
                .expect("nested tuple")
                .len(),
            2
        );
    }

    /// Compound defaults compose arrays and tuples, and reject leaves lacking defaults.
    #[test]
    fn canonical_defaults_compose_and_report_missing_leaf_defaults() {
        use cel_runtime::{ArrayOf, Cons, IntoRustTuple, Leaf, Nil};
        let parsed = crate::AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new())
            .parse_str("sheet s { cell x: (i32, ([f64],)); }")
            .unwrap();
        type Shape = Cons<Leaf<i32>, Cons<Cons<ArrayOf<Leaf<f64>>, Nil>, Nil>>;
        let (number, nested) = parsed
            .read_compound::<Shape>(parsed.cell_names["x"].0)
            .unwrap()
            .into_rust_tuple();
        assert_eq!(number, 0);
        assert!(nested.into_rust_tuple().0.is_empty());
        #[derive(Clone, PartialEq, Debug)]
        struct NoDefault;
        let mut registry = TypeRegistry::new();
        registry.register_no_default::<NoDefault>("NoDefault");
        let error = crate::AdamParser::new(registry, cel_parser::OpLookup::new())
            .parse_str("sheet s { cell x: (NoDefault,); }")
            .err()
            .unwrap();
        assert!(error.to_string().contains("NoDefault"));
    }

    #[test]
    fn eq_dyn_fn_compares_equal_i32_values_as_equal() {
        let reg = TypeRegistry::new();
        let entry = reg.get("i32").unwrap();
        let a: i32 = 7;
        let b: i32 = 7;
        assert!((entry.eq_dyn_fn)(&a, &b));
    }

    #[test]
    fn eq_dyn_fn_compares_unequal_i32_values_as_unequal() {
        let reg = TypeRegistry::new();
        let entry = reg.get("i32").unwrap();
        let a: i32 = 7;
        let b: i32 = 8;
        assert!(!(entry.eq_dyn_fn)(&a, &b));
    }

    #[test]
    fn register_no_default_also_populates_eq_dyn_fn() {
        #[derive(PartialEq, Clone, Debug)]
        struct NoDefault(i32);

        let mut reg = TypeRegistry::new();
        reg.register_no_default::<NoDefault>("NoDefault");
        let entry = reg.get("NoDefault").unwrap();
        let a = NoDefault(1);
        let b = NoDefault(1);
        let c = NoDefault(2);
        assert!((entry.eq_dyn_fn)(&a, &b));
        assert!(!(entry.eq_dyn_fn)(&a, &c));
    }

    #[test]
    fn range_entry_recognizes_a_registered_numeric_range_inclusive_type() {
        let reg = TypeRegistry::new();
        let entry = reg
            .range_entry(TypeId::of::<std::ops::RangeInclusive<i32>>())
            .expect("i32 range recognized");
        assert_eq!(entry.element_type_id, TypeId::of::<i32>());
    }

    #[test]
    fn range_entry_returns_none_for_a_non_range_type() {
        let reg = TypeRegistry::new();
        assert!(reg.range_entry(TypeId::of::<i32>()).is_none());
    }

    #[test]
    fn range_entry_clamp_fn_clamps_a_value_into_the_evaluated_bounds() {
        let reg = TypeRegistry::new();
        let entry = reg
            .range_entry(TypeId::of::<std::ops::RangeInclusive<i32>>())
            .unwrap();
        let mut segment = DynSegment::new::<()>();
        segment.push_arg::<i32>(1);
        segment.push_arg::<i32>(2);
        segment.op2(|a: i32, b: i32| a..=b).unwrap();
        let value = 500i32;
        let lo = 0i32;
        let hi = 100i32;
        let evaluated = segment.call_dyn_value(&[&value, &lo, &hi]).unwrap();
        let result = (entry.clamp_fn)(evaluated, &value).unwrap();
        assert_eq!(*result.downcast_ref::<i32>().unwrap(), 100);
    }

    #[test]
    fn range_entry_bounds_fn_returns_the_evaluated_bounds() {
        let reg = TypeRegistry::new();
        let entry = reg
            .range_entry(TypeId::of::<std::ops::RangeInclusive<i32>>())
            .unwrap();
        let mut segment = DynSegment::new::<()>();
        segment.push_arg::<i32>(1);
        segment.push_arg::<i32>(2);
        segment.op2(|a: i32, b: i32| a..=b).unwrap();
        let placeholder = 0i32;
        let lo = 0i32;
        let hi = 100i32;
        let evaluated = segment.call_dyn_value(&[&placeholder, &lo, &hi]).unwrap();
        let (lo_out, hi_out) = (entry.bounds_fn)(evaluated).unwrap();
        assert_eq!(*lo_out.downcast_ref::<i32>().unwrap(), 0);
        assert_eq!(*hi_out.downcast_ref::<i32>().unwrap(), 100);
    }
}
