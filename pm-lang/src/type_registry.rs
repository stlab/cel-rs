//! Type registry for pm-lang cell declarations.
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
//! use pm_lang::TypeRegistry;
//! use std::any::TypeId;
//!
//! let reg = TypeRegistry::new();
//! assert_eq!(reg.get("f64").unwrap().type_id, TypeId::of::<f64>());
//! ```

use std::any::{Any, TypeId};
use std::collections::HashMap;

use cel_runtime::DynSegment;
use property_model::{CellId, ConditionalId, RelationshipId, Sheet};

/// Registers a `push_arg<T>(index)` op on a segment.
pub type PushArgFn = fn(&mut DynSegment, usize);

/// Adds a typed cell from a boxed value and returns its handle.
pub type AddCellFn = fn(&mut Sheet, Box<dyn Any>) -> CellId;

/// Executes a compiled segment with the supplied inputs and boxes the result.
pub type CallDynFn = fn(&mut DynSegment, &[&dyn Any]) -> anyhow::Result<Box<dyn Any>>;

/// Calls `Sheet::add_conditional` with the appropriate concrete type.
///
/// Each branch carries a single boxed key value and the `RelationshipId` for that branch.
/// The default is a list of `RelationshipId`s active when no branch key matches.
pub type AddConditionalFn = fn(
    &mut Sheet,
    CellId,
    Vec<(Box<dyn Any>, RelationshipId)>,
    Vec<RelationshipId>,
) -> Result<ConditionalId, property_model::Error>;

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
    /// Constructs a default `T` if the type implements `Default`; otherwise `None`.
    pub default_fn: Option<fn() -> Box<dyn Any>>,
    /// Calls `Sheet::add_conditional::<T>` with type-erased branch keys.
    pub add_conditional_fn: AddConditionalFn,
}

/// Maps DSL type names to Rust types for pm-lang cell declarations.
///
/// # Example
///
/// ```rust
/// use pm_lang::TypeRegistry;
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
    cell: CellId,
    branches: Vec<(Box<dyn Any>, RelationshipId)>,
    default: Vec<RelationshipId>,
) -> Result<ConditionalId, property_model::Error> {
    let typed_branches: Vec<(Vec<T>, Vec<RelationshipId>)> = branches
        .into_iter()
        .map(|(val, rel_id)| {
            let v = *val
                .downcast::<T>()
                .expect("add_conditional_impl: type matches registration");
            (vec![v], vec![rel_id])
        })
        .collect();
    sheet.add_conditional::<T>(cell, typed_branches, default)
}

fn add_cell_impl<T: Any + PartialEq + 'static>(sheet: &mut Sheet, value: Box<dyn Any>) -> CellId {
    let v = value
        .downcast::<T>()
        .expect("add_cell_impl: type matches registration");
    sheet.add_cell(*v)
}

fn call_dyn_impl<T: 'static + Clone>(
    seg: &mut DynSegment,
    inputs: &[&dyn Any],
) -> anyhow::Result<Box<dyn Any>> {
    Ok(Box::new(seg.call_dyn::<T>(inputs)?))
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
    /// use pm_lang::TypeRegistry;
    /// let mut reg = TypeRegistry::new();
    /// reg.register::<u64>("my_u64");
    /// assert!(reg.get("my_u64").is_some());
    /// ```
    pub fn register<T: Any + PartialEq + Default + Clone + 'static>(&mut self, name: &str) {
        let type_id = TypeId::of::<T>();
        self.by_name.insert(
            name.to_owned(),
            TypeEntry {
                type_id,
                type_name: std::any::type_name::<T>(),
                push_arg_fn: push_arg_impl::<T>,
                add_cell_fn: add_cell_impl::<T>,
                call_dyn_fn: call_dyn_impl::<T>,
                default_fn: Some(|| Box::new(T::default()) as Box<dyn Any>),
                add_conditional_fn: add_conditional_impl::<T>,
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
    /// use pm_lang::TypeRegistry;
    /// #[derive(PartialEq, Clone)]
    /// struct MyType(i32);
    /// let mut reg = TypeRegistry::new();
    /// reg.register_no_default::<MyType>("MyType");
    /// let entry = reg.get("MyType").unwrap();
    /// assert!(entry.default_fn.is_none());
    /// ```
    pub fn register_no_default<T: Any + PartialEq + Clone + 'static>(&mut self, name: &str) {
        let type_id = TypeId::of::<T>();
        self.by_name.insert(
            name.to_owned(),
            TypeEntry {
                type_id,
                type_name: std::any::type_name::<T>(),
                push_arg_fn: push_arg_impl::<T>,
                add_cell_fn: add_cell_impl::<T>,
                call_dyn_fn: call_dyn_impl::<T>,
                default_fn: None,
                add_conditional_fn: add_conditional_impl::<T>,
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
    /// use pm_lang::TypeRegistry;
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
    /// use pm_lang::TypeRegistry;
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
    use std::any::TypeId;

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
        #[derive(PartialEq, Clone)]
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
        use property_model::Sheet;
        use std::any::Any;

        let reg = TypeRegistry::new();
        let entry = reg.get("f64").unwrap();
        let mut sheet = Sheet::new();
        let val: Box<dyn Any> = Box::new(3.14_f64);
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
}
