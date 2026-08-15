//! Type registry for adam-lang cell declarations.
//!
//! [`TypeRegistry`] maps DSL type-name strings to Rust types. Each registration
//! stores type-erased function pointers covering:
//! - `push_arg_fn` — registers a [`cel_runtime::DynSegment::push_arg`] op
//! - `add_cell_fn` — creates a sheet cell from a `Box<dyn Any>` value
//! - `call_dyn_fn` — executes a compiled segment and boxes the result
//! - `default_fn`  — constructs a default `Box<dyn Any>` (when `Default` is available)
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
use cel_runtime::{BoxExtractor, DynSegment};

/// The identity of a declared adam-lang cell type. Every distinct tuple *shape* erases to the
/// same Rust type (`cel_runtime::DynamicSequence`), so a flat `TypeId` alone can no longer
/// identify a declared cell type once tuples exist — this recursive identity replaces it
/// wherever a declared cell type is tracked.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum TypeShape {
    /// A registered leaf type, by its Rust `TypeId`.
    Named(TypeId),
    /// A tuple type, recursively — an empty `Vec` for `()`.
    Tuple(Vec<TypeShape>),
}

/// Registers a `push_arg<T>(index)` op on a segment.
pub type PushArgFn = fn(&mut DynSegment, usize);

/// Adds a typed cell from a boxed value and returns its handle.
pub type AddCellFn = fn(&mut Sheet, Box<dyn Any>) -> CellId;

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

/// Metadata for a single type registered in a [`TypeRegistry`].
pub struct TypeEntry {
    /// Runtime type identity.
    pub type_id: TypeId,
    /// Rust type name for error messages.
    pub type_name: &'static str,
    /// Registers a `push_arg<T>` op at the given argument index.
    pub push_arg_fn: PushArgFn,
    /// Creates a sheet cell from a `Box<dyn Any>` holding a `T`.
    pub add_cell_fn: AddCellFn,
    /// Calls `DynSegment::call_dyn::<T>` and boxes the result.
    pub call_dyn_fn: CallDynFn,
    /// Compares two type-erased values of this type for equality.
    pub eq_dyn_fn: fn(&dyn Any, &dyn Any) -> bool,
    /// Reads and clones a `T` from a raw pointer into a type-erased box; used to
    /// split a multi-output method's tuple result into per-cell values.
    pub extract_box_fn: BoxExtractor,
    /// Constructs a default `T` if the type implements `Default`; otherwise `None`.
    pub default_fn: Option<fn() -> Box<dyn Any>>,
    /// Calls `Sheet::add_conditional::<T>` with type-erased branch keys.
    pub add_conditional_fn: AddConditionalFn,
    /// Size in bytes of a value of this type.
    pub size: usize,
    /// Required alignment in bytes of a value of this type.
    pub align: usize,
    /// In-place dropper taking an unused `associated` parameter, for building an `AssociatedType`
    /// "prototype" describing a tuple-typed cell's on-stack shape.
    pub raw_dropper: cel_runtime::RawDropper,
    /// In-place dropper for this type as a `DynamicSequence` tuple element.
    pub element_drop: cel_runtime::ElementDropper,
    /// In-place cloner for this type as a `DynamicSequence` tuple element.
    pub element_clone: cel_runtime::ElementCloner,
    /// Equality comparator for this type as a `DynamicSequence` tuple element.
    pub element_eq: cel_runtime::ElementEq,
    /// Debug-formatter for this type as a `DynamicSequence` tuple element.
    pub element_debug: cel_runtime::ElementDebug,
    /// Moves a boxed value of this type into a `DynamicSequence` being built from boxed defaults.
    pub element_write: unsafe fn(Box<dyn Any>, *mut u8),
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
pub struct TypeRegistry {
    by_name: HashMap<String, TypeEntry>,
    by_type_id: HashMap<TypeId, String>,
}

fn push_arg_impl<T: 'static + Clone>(segment: &mut DynSegment, index: usize) {
    segment.push_arg::<T>(index);
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

/// Reads and clones a `T` from `ptr`, boxing it as `Box<dyn Any>`.
///
/// # Safety
/// `ptr` must point to a valid, live, properly aligned `T`.
unsafe fn extract_box_impl<T: Clone + 'static>(ptr: *const u8) -> Box<dyn Any> {
    Box::new(unsafe { (*ptr.cast::<T>()).clone() })
}

impl TypeRegistry {
    /// Creates a registry pre-populated with all built-in CEL/Rust primitive types.
    ///
    /// Registered types: `i8`, `i16`, `i32`, `i64`, `i128`, `isize`,
    /// `u8`, `u16`, `u32`, `u64`, `u128`, `usize`, `f32`, `f64`, `bool`, `String`.
    #[must_use]
    pub fn new() -> Self {
        let mut r = TypeRegistry {
            by_name: HashMap::new(),
            by_type_id: HashMap::new(),
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
                type_id,
                type_name: std::any::type_name::<T>(),
                push_arg_fn: push_arg_impl::<T>,
                add_cell_fn: add_cell_impl::<T>,
                call_dyn_fn: call_dyn_impl::<T>,
                eq_dyn_fn: eq_dyn_impl::<T>,
                extract_box_fn: extract_box_impl::<T>,
                default_fn: Some(|| Box::new(T::default()) as Box<dyn Any>),
                add_conditional_fn: add_conditional_impl::<T>,
                size: std::mem::size_of::<T>(),
                align: std::mem::align_of::<T>(),
                raw_dropper: cel_runtime::raw_dropper_for::<T>(),
                element_drop: cel_runtime::element_dropper_for::<T>(),
                element_clone: cel_runtime::element_cloner_for::<T>(),
                element_eq: cel_runtime::element_eq_for::<T>(),
                element_debug: cel_runtime::element_debug_for::<T>(),
                element_write: cel_runtime::element_writer_for::<T>(),
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
                type_id,
                type_name: std::any::type_name::<T>(),
                push_arg_fn: push_arg_impl::<T>,
                add_cell_fn: add_cell_impl::<T>,
                call_dyn_fn: call_dyn_impl::<T>,
                eq_dyn_fn: eq_dyn_impl::<T>,
                extract_box_fn: extract_box_impl::<T>,
                default_fn: None,
                add_conditional_fn: add_conditional_impl::<T>,
                size: std::mem::size_of::<T>(),
                align: std::mem::align_of::<T>(),
                raw_dropper: cel_runtime::raw_dropper_for::<T>(),
                element_drop: cel_runtime::element_dropper_for::<T>(),
                element_clone: cel_runtime::element_cloner_for::<T>(),
                element_eq: cel_runtime::element_eq_for::<T>(),
                element_debug: cel_runtime::element_debug_for::<T>(),
                element_write: cel_runtime::element_writer_for::<T>(),
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

    /// Resolves a parsed `type_expr` against this registry, recursively.
    ///
    /// # Errors
    /// Returns the unknown type name and its span if some leaf name isn't registered.
    pub fn resolve(
        &self,
        expr: &crate::ast::TypeExpr,
    ) -> std::result::Result<TypeShape, (String, proc_macro2::Span)> {
        match expr {
            crate::ast::TypeExpr::Named(name, span) => {
                let entry = self
                    .get(name)
                    .ok_or_else(|| (format!("unknown type `{name}`"), span.start))?;
                Ok(TypeShape::Named(entry.type_id))
            }
            crate::ast::TypeExpr::Tuple(elements, _) => {
                let shapes = elements
                    .iter()
                    .map(|e| self.resolve(e))
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(TypeShape::Tuple(shapes))
            }
        }
    }

    /// Formats `shape` recursively, e.g. `"(i32, (f64, String))"`, for error messages.
    #[must_use]
    pub fn display_name(&self, shape: &TypeShape) -> String {
        match shape {
            TypeShape::Named(type_id) => self
                .entry_by_type_id(*type_id)
                .map(|e| e.type_name.to_string())
                .unwrap_or_else(|| "?".to_string()),
            TypeShape::Tuple(elements) => {
                let parts: Vec<String> = elements.iter().map(|e| self.display_name(e)).collect();
                format!("({})", parts.join(", "))
            }
        }
    }

    /// Returns the `(Drop, Clone, PartialEq, Debug)` quadruple registered for `type_id`, for use
    /// as the `leaf` callback `cel_runtime::DynSegment::call_dyn_as_dynamic_sequence` needs.
    #[must_use]
    pub fn element_descriptor(
        &self,
        type_id: TypeId,
    ) -> Option<(
        cel_runtime::ElementDropper,
        cel_runtime::ElementCloner,
        cel_runtime::ElementEq,
        cel_runtime::ElementDebug,
    )> {
        self.entry_by_type_id(type_id).map(|e| {
            (
                e.element_drop,
                e.element_clone,
                e.element_eq,
                e.element_debug,
            )
        })
    }

    /// Builds an owned table of every leaf `TypeId` in `shape` paired with its
    /// `Drop`/`Clone`/`PartialEq`/`Debug` descriptor, for a closure that must outlive this
    /// registry (e.g. a `Method`'s stored output-extraction closure).
    ///
    /// - Precondition: every leaf `TypeId` in `shape` is registered (already resolved via
    ///   `TypeRegistry::resolve`, which would have already errored otherwise).
    ///
    /// - Complexity: O(n) in the number of leaves in `shape`.
    #[must_use]
    pub fn element_descriptors_for(
        &self,
        shape: &TypeShape,
    ) -> Vec<(
        TypeId,
        cel_runtime::ElementDropper,
        cel_runtime::ElementCloner,
        cel_runtime::ElementEq,
        cel_runtime::ElementDebug,
    )> {
        match shape {
            TypeShape::Named(type_id) => {
                let (drop, clone, eq, debug) = self
                    .element_descriptor(*type_id)
                    .expect("element_descriptors_for: type registered");
                vec![(*type_id, drop, clone, eq, debug)]
            }
            TypeShape::Tuple(elements) => elements
                .iter()
                .flat_map(|e| self.element_descriptors_for(e))
                .collect(),
        }
    }

    /// Builds the recursive `AssociatedType` "prototype" describing `shape`'s on-stack tuple
    /// layout, for `cel_runtime::DynSegment::push_arg_as_dynamic_sequence_tuple`.
    ///
    /// - Precondition: `shape` is `TypeShape::Tuple(_)` — a scalar cell never needs this.
    #[must_use]
    pub fn associated_prototype(&self, shape: &TypeShape) -> Vec<cel_runtime::AssociatedType> {
        let TypeShape::Tuple(elements) = shape else {
            debug_assert!(
                false,
                "associated_prototype's precondition: shape is a Tuple"
            );
            return Vec::new();
        };
        let mut associated: Vec<_> = elements.iter().map(|e| self.one_associated(e)).collect();
        cel_runtime::layout_associated(&mut associated);
        associated
    }

    /// Builds one `AssociatedType` entry describing `shape`: a leaf's own registered layout for
    /// `TypeShape::Named`, or a nested tuple's layout (computed recursively via
    /// `cel_runtime::layout_associated` over its own children) for `TypeShape::Tuple`.
    ///
    /// - Precondition: every leaf `TypeId` reachable from `shape` is registered.
    ///
    /// - Postcondition: the returned entry's `offset` is always 0; the caller
    ///   (`associated_prototype`) lays out sibling entries via `cel_runtime::layout_associated`.
    fn one_associated(&self, shape: &TypeShape) -> cel_runtime::AssociatedType {
        match shape {
            TypeShape::Named(type_id) => {
                let entry = self
                    .entry_by_type_id(*type_id)
                    .expect("one_associated: type registered");
                cel_runtime::AssociatedType {
                    type_id: *type_id,
                    type_name: std::borrow::Cow::Owned(entry.type_name.to_string()),
                    offset: 0,
                    size: entry.size,
                    align: entry.align,
                    dropper: entry.raw_dropper,
                    associated: Vec::new(),
                }
            }
            TypeShape::Tuple(elements) => {
                let mut associated: Vec<_> =
                    elements.iter().map(|e| self.one_associated(e)).collect();
                let (size, align) = cel_runtime::layout_associated(&mut associated);
                cel_runtime::AssociatedType {
                    type_id: TypeId::of::<cel_runtime::DynTuple>(),
                    type_name: std::borrow::Cow::Borrowed("tuple"),
                    offset: 0,
                    size,
                    align,
                    dropper: cel_runtime::drop_tuple,
                    associated,
                }
            }
        }
    }

    /// Builds a default `DynamicSequence` for a tuple-typed cell, recursively, using each leaf's
    /// own registered default.
    ///
    /// - Precondition: `shape` is `TypeShape::Tuple(_)`.
    ///
    /// # Errors
    /// Returns an error naming the specific leaf type that has no registered default.
    pub fn default_dynamic_sequence(
        &self,
        shape: &TypeShape,
    ) -> std::result::Result<cel_runtime::DynamicSequence, String> {
        let TypeShape::Tuple(elements) = shape else {
            debug_assert!(
                false,
                "default_dynamic_sequence's precondition: shape is a Tuple"
            );
            return Ok(cel_runtime::DynamicSequence::from_dyn_elements(Vec::new()));
        };
        let built = elements
            .iter()
            .map(|e| self.default_dyn_element(e))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(cel_runtime::DynamicSequence::from_dyn_elements(built))
    }

    /// Builds one `(DynElementSpec, Box<dyn Any>)` pair for `shape`: a leaf's own registered
    /// default value for `TypeShape::Named`, or — recursively, via `default_dynamic_sequence` —
    /// a nested `DynamicSequence` built from its own children's defaults for `TypeShape::Tuple`.
    ///
    /// # Errors
    /// Returns an error naming the specific leaf type that has no registered default.
    fn default_dyn_element(
        &self,
        shape: &TypeShape,
    ) -> std::result::Result<(cel_runtime::DynElementSpec, Box<dyn Any>), String> {
        match shape {
            TypeShape::Named(type_id) => {
                let entry = self
                    .entry_by_type_id(*type_id)
                    .expect("default_dyn_element: type registered");
                let default_fn = entry.default_fn.ok_or_else(|| {
                    format!("type `{}` has no default; provide `= ...`", entry.type_name)
                })?;
                Ok((
                    cel_runtime::DynElementSpec {
                        type_id: *type_id,
                        type_name: std::borrow::Cow::Owned(entry.type_name.to_string()),
                        size: entry.size,
                        align: entry.align,
                        drop: entry.element_drop,
                        clone: entry.element_clone,
                        eq: entry.element_eq,
                        debug: entry.element_debug,
                        write: entry.element_write,
                    },
                    default_fn(),
                ))
            }
            TypeShape::Tuple(_) => {
                let nested = self.default_dynamic_sequence(shape)?;
                Ok((
                    cel_runtime::DynElementSpec {
                        type_id: TypeId::of::<cel_runtime::DynamicSequence>(),
                        type_name: std::borrow::Cow::Borrowed("DynamicSequence"),
                        size: std::mem::size_of::<cel_runtime::DynamicSequence>(),
                        align: std::mem::align_of::<cel_runtime::DynamicSequence>(),
                        drop: cel_runtime::element_dropper_for::<cel_runtime::DynamicSequence>(),
                        clone: cel_runtime::element_cloner_for::<cel_runtime::DynamicSequence>(),
                        eq: cel_runtime::element_eq_for::<cel_runtime::DynamicSequence>(),
                        debug: cel_runtime::element_debug_for::<cel_runtime::DynamicSequence>(),
                        write: cel_runtime::element_writer_for::<cel_runtime::DynamicSequence>(),
                    },
                    Box::new(nested) as Box<dyn Any>,
                ))
            }
        }
    }
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
    fn extract_box_fn_reads_and_clones_value() {
        let reg = TypeRegistry::new();
        let entry = reg.get("i32").unwrap();
        let value: i32 = 42;
        let boxed = unsafe { (entry.extract_box_fn)((&value as *const i32).cast::<u8>()) };
        let result: Box<i32> = boxed.downcast::<i32>().expect("i32");
        assert_eq!(*result, 42);
    }

    #[test]
    fn extract_box_fn_clones_string_independently_of_original() {
        let reg = TypeRegistry::new();
        let entry = reg.get("String").unwrap();
        let original = String::from("hello world, this is heap allocated");
        let original_ptr = original.as_ptr();
        let boxed = unsafe { (entry.extract_box_fn)((&original as *const String).cast::<u8>()) };
        let extracted: Box<String> = boxed.downcast::<String>().expect("String");
        assert_eq!(*extracted, original);
        assert_ne!(
            extracted.as_ptr(),
            original_ptr,
            "extract_box_fn must clone (new heap allocation), not move out the original's buffer"
        );
        // `original` is still independently valid and droppable here — proving it wasn't
        // moved out from under us. Both `original` and `extracted` drop safely at scope end.
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

    #[test]
    fn new_registers_i32_with_the_new_descriptor_fields() {
        let reg = TypeRegistry::new();
        let entry = reg.get("i32").unwrap();
        assert_eq!(entry.size, std::mem::size_of::<i32>());
        assert_eq!(entry.align, std::mem::align_of::<i32>());
        let mut a = 7i32;
        let mut b = 0i32;
        unsafe {
            (entry.element_clone)((&raw mut a).cast::<u8>(), (&raw mut b).cast::<u8>());
        }
        assert_eq!(b, 7);
        assert!(unsafe {
            (entry.element_eq)((&raw const a).cast::<u8>(), (&raw const b).cast::<u8>())
        });
    }

    #[test]
    fn resolve_named_type_expr_returns_the_matching_type_shape() {
        let reg = TypeRegistry::new();
        let expr = crate::ast::TypeExpr::Named("i32".to_string(), point(Span::call_site()));
        let shape = reg.resolve(&expr).unwrap();
        assert_eq!(shape, TypeShape::Named(TypeId::of::<i32>()));
    }

    #[test]
    fn resolve_unknown_named_type_expr_is_an_error() {
        let reg = TypeRegistry::new();
        let expr = crate::ast::TypeExpr::Named("bogus".to_string(), point(Span::call_site()));
        assert!(reg.resolve(&expr).is_err());
    }

    #[test]
    fn resolve_tuple_type_expr_returns_a_nested_type_shape() {
        let reg = TypeRegistry::new();
        let span = point(Span::call_site());
        let expr = crate::ast::TypeExpr::Tuple(
            vec![
                crate::ast::TypeExpr::Named("i32".to_string(), span),
                crate::ast::TypeExpr::Tuple(
                    vec![
                        crate::ast::TypeExpr::Named("f64".to_string(), span),
                        crate::ast::TypeExpr::Named("String".to_string(), span),
                    ],
                    span,
                ),
            ],
            span,
        );
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
    fn display_name_formats_a_nested_tuple_shape() {
        let reg = TypeRegistry::new();
        let shape = TypeShape::Tuple(vec![
            TypeShape::Named(TypeId::of::<i32>()),
            TypeShape::Tuple(vec![
                TypeShape::Named(TypeId::of::<f64>()),
                TypeShape::Named(TypeId::of::<String>()),
            ]),
        ]);
        assert_eq!(
            reg.display_name(&shape),
            "(i32, (f64, alloc::string::String))"
        );
    }

    #[test]
    fn element_descriptor_returns_the_registered_types_own_functions() {
        let reg = TypeRegistry::new();
        let (drop, clone, eq, _debug) = reg.element_descriptor(TypeId::of::<i32>()).unwrap();
        let (mut a, mut b) = (7i32, 0i32);
        unsafe {
            clone((&raw mut a).cast::<u8>(), (&raw mut b).cast::<u8>());
        }
        assert_eq!(b, 7);
        assert!(unsafe { eq((&raw const a).cast::<u8>(), (&raw const b).cast::<u8>()) });
        unsafe { drop((&raw mut b).cast::<u8>()) };
    }

    #[test]
    fn element_descriptor_is_none_for_an_unregistered_type() {
        let reg = TypeRegistry::new();
        assert!(reg.element_descriptor(TypeId::of::<Vec<u8>>()).is_none());
    }

    #[test]
    fn associated_prototype_describes_a_flat_tuple() {
        let reg = TypeRegistry::new();
        let shape = TypeShape::Tuple(vec![
            TypeShape::Named(TypeId::of::<i32>()),
            TypeShape::Named(TypeId::of::<f64>()),
        ]);
        let prototype = reg.associated_prototype(&shape);
        assert_eq!(prototype.len(), 2);
        assert_eq!(prototype[0].type_id, TypeId::of::<i32>());
        assert_eq!(prototype[1].type_id, TypeId::of::<f64>());
        assert_eq!(prototype[1].offset, 8); // i32 at [0,4); f64 aligned up to 8
    }

    #[test]
    fn associated_prototype_describes_a_nested_tuple() {
        let reg = TypeRegistry::new();
        let shape = TypeShape::Tuple(vec![
            TypeShape::Named(TypeId::of::<i32>()),
            TypeShape::Tuple(vec![
                TypeShape::Named(TypeId::of::<i32>()),
                TypeShape::Named(TypeId::of::<i32>()),
            ]),
        ]);
        let prototype = reg.associated_prototype(&shape);
        assert_eq!(prototype.len(), 2);
        assert_eq!(prototype[1].type_id, TypeId::of::<cel_runtime::DynTuple>());
        assert_eq!(prototype[1].associated.len(), 2);
    }

    #[test]
    fn default_dynamic_sequence_builds_a_matching_default_value() {
        let reg = TypeRegistry::new();
        let shape = TypeShape::Tuple(vec![
            TypeShape::Named(TypeId::of::<i32>()),
            TypeShape::Named(TypeId::of::<f64>()),
        ]);
        let seq = reg.default_dynamic_sequence(&shape).unwrap();
        assert_eq!(seq.arity(), 2);
        let (a, b): (i32, f64) = seq.try_to_tuple().unwrap();
        assert_eq!((a, b), (i32::default(), f64::default()));
    }

    #[test]
    fn default_dynamic_sequence_recurses_into_nested_tuples() {
        let reg = TypeRegistry::new();
        let shape = TypeShape::Tuple(vec![
            TypeShape::Named(TypeId::of::<i32>()),
            TypeShape::Tuple(vec![TypeShape::Named(TypeId::of::<f64>())]),
        ]);
        let seq = reg.default_dynamic_sequence(&shape).unwrap();
        let (_, nested): (i32, cel_runtime::DynamicSequence) = seq.try_to_tuple().unwrap();
        assert_eq!(nested.arity(), 1);
    }

    #[test]
    fn default_dynamic_sequence_errors_naming_a_leaf_with_no_default() {
        #[derive(PartialEq, Clone, Debug)]
        struct NoDefault(i32);
        let mut reg = TypeRegistry::new();
        reg.register_no_default::<NoDefault>("NoDefault");
        let shape = TypeShape::Tuple(vec![TypeShape::Named(TypeId::of::<NoDefault>())]);
        let result = reg.default_dynamic_sequence(&shape);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("NoDefault"));
    }

    #[test]
    fn new_registers_i32_with_a_debug_descriptor() {
        let reg = TypeRegistry::new();
        let entry = reg.get("i32").unwrap();
        let value = 7i32;
        struct Wrapper(*const u8, cel_runtime::ElementDebug);
        impl std::fmt::Debug for Wrapper {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                unsafe { (self.1)(self.0, f) }
            }
        }
        let wrapper = Wrapper((&raw const value).cast::<u8>(), entry.element_debug);
        assert_eq!(format!("{wrapper:?}"), "7");
    }

    #[test]
    fn element_descriptor_includes_a_working_debug_formatter() {
        let reg = TypeRegistry::new();
        let (_, _, _, debug) = reg.element_descriptor(TypeId::of::<i32>()).unwrap();
        let value = 7i32;
        struct Wrapper(*const u8, cel_runtime::ElementDebug);
        impl std::fmt::Debug for Wrapper {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                unsafe { (self.1)(self.0, f) }
            }
        }
        let wrapper = Wrapper((&raw const value).cast::<u8>(), debug);
        assert_eq!(format!("{wrapper:?}"), "7");
    }

    #[test]
    fn element_descriptors_for_a_tuple_shape_includes_debug_formatters() {
        let reg = TypeRegistry::new();
        let shape = TypeShape::Tuple(vec![
            TypeShape::Named(TypeId::of::<i32>()),
            TypeShape::Named(TypeId::of::<f64>()),
        ]);
        let table = reg.element_descriptors_for(&shape);
        assert_eq!(table.len(), 2);
        assert_eq!(table[0].0, TypeId::of::<i32>());
        assert_eq!(table[1].0, TypeId::of::<f64>());
    }

    #[test]
    fn default_dynamic_sequence_result_debug_formats_correctly() {
        let reg = TypeRegistry::new();
        let shape = TypeShape::Tuple(vec![
            TypeShape::Named(TypeId::of::<i32>()),
            TypeShape::Named(TypeId::of::<f64>()),
        ]);
        let seq = reg.default_dynamic_sequence(&shape).unwrap();
        // i32::default() Debug-formats as "0"; f64::default() (0.0) Debug-formats as "0.0", not "0".
        assert_eq!(format!("{seq:?}"), "(0, 0.0)");
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
}
