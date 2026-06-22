# property-model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `property-model` crate — a standalone Rust library for constructing and executing property model constraint graphs with multi-way relationships and explicit propagation.

**Architecture:** A bipartite graph of value cells and multi-way relationships, stored as two `SlotMap`s with distinct typed handles. A greedy planner selects one method per relationship by minimising the minimum strength (write-recency clock) of the output cells, then executes the selected methods in Kahn's-algorithm topological order. Propagation is explicit — the client calls `Sheet::propagate()`.

**Tech Stack:** Rust 2024 edition, `slotmap 1.0`, `anyhow 1.0`

## Global Constraints

- Rust edition: 2024 (matches workspace)
- `cargo fmt --all` required before every commit (pre-commit hook enforced)
- `cargo clippy --workspace -- -D warnings` must pass (warnings are errors)
- `cargo test --workspace` must pass before commit
- `missing_docs = "warn"` is a workspace lint — every `pub` item needs a `///` contract-style doc comment
- Every function needs a `///` doc comment: Summary sentence, `# Errors` for fallible fns, `/// - Complexity: O(?)` if non-O(1); see CLAUDE.md
- No `Box<dyn Error>` for method errors — use `anyhow::Error`
- No `unsafe`, no `unwrap` in production code paths (use `expect` only where the invariant is established by construction)

---

## File Map

| File | Responsibility |
|---|---|
| `property-model/Cargo.toml` | Crate manifest; `slotmap`, `anyhow` deps |
| `property-model/src/lib.rs` | Public re-exports; crate-level doc with example |
| `property-model/src/error.rs` | `Error` enum; `Display` + `std::error::Error` impls |
| `property-model/src/cell.rs` | `CellId` (slotmap key); `CellData` (pub(crate)) |
| `property-model/src/relationship.rs` | `RelationshipId`; `Method`; typed helpers; `RelationshipData` |
| `property-model/src/sheet.rs` | `Sheet`; `add_cell`, `add_relationship`, `write`, `read`, `propagate`, `changed`, `clear_changed` |
| `property-model/src/planner.rs` | `plan()`: method selection (Phase 1) + topological sort (Phase 2); `Plan` |
| `property-model/tests/integration.rs` | End-to-end propagation tests |

---

### Task 1: Crate Scaffold and Error Types

**Files:**
- Create: `property-model/Cargo.toml`
- Create: `property-model/src/lib.rs`
- Create: `property-model/src/error.rs`
- Modify: `Cargo.toml` (root — add `"property-model"` to `[workspace] members`)

**Interfaces:**
- Consumes: nothing
- Produces: `property_model::Error`; crate compiles and `cargo test -p property-model` runs

- [ ] **Step 1: Add `property-model` to the workspace**

In root `Cargo.toml`, change the `members` list:

```toml
[workspace]
members = [
    "cel-runtime",
    "cel-parser",
    "cel-rs-macros",
    "property-model",
]
```

- [ ] **Step 2: Create `property-model/Cargo.toml`**

```toml
[package]
name = "property-model"
version = "0.1.0"
edition = "2024"
description = "Runtime library for property model constraint graphs"

[dependencies]
slotmap = "1.0"
anyhow = "1.0"

[lints]
workspace = true
```

- [ ] **Step 3: Write failing tests for `error.rs`**

Create `property-model/src/error.rs` with only the test module first:

```rust
//! The `Error` type returned by all fallible operations in this crate.

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::TypeId;

    #[test]
    fn type_mismatch_display_contains_type_mismatch() {
        let err = Error::TypeMismatch {
            expected: TypeId::of::<i32>(),
            found: TypeId::of::<f64>(),
        };
        assert!(err.to_string().contains("type mismatch"));
    }

    #[test]
    fn invalid_id_display_contains_invalid() {
        assert!(Error::InvalidId.to_string().contains("invalid"));
    }

    #[test]
    fn conflict_display_contains_overconstrained() {
        assert!(Error::Conflict.to_string().contains("overconstrained"));
    }

    #[test]
    fn cycle_display_contains_cycle() {
        assert!(Error::Cycle.to_string().contains("cycle"));
    }

    #[test]
    fn method_failed_display_contains_source_message() {
        let err = Error::MethodFailed(anyhow::anyhow!("division by zero"));
        assert!(err.to_string().contains("division by zero"));
    }

    #[test]
    fn invalid_method_display_contains_invalid() {
        assert!(Error::InvalidMethod.to_string().contains("invalid"));
    }

    #[test]
    fn error_implements_std_error() {
        fn takes_error(_: &dyn std::error::Error) {}
        takes_error(&Error::InvalidId);
        takes_error(&Error::Conflict);
    }
}
```

- [ ] **Step 4: Create `property-model/src/lib.rs` and run tests to see them fail**

```rust
//! # property-model
//!
//! A library for constructing and executing property model constraint graphs.
//!
//! A property model is a bipartite graph of **value cells** and **relationships**.
//! Cells hold type-erased values. Relationships define multi-way constraints: each
//! relationship supplies multiple methods, and at propagation time the planner
//! selects one method per relationship based on cell write-recency (strength),
//! then executes the selected methods in dependency order.
//!
//! # Example
//!
//! ```rust
//! use property_model::{Sheet, Method};
//!
//! let mut sheet = Sheet::new();
//! let a = sheet.add_cell(2.0_f64);
//! let b = sheet.add_cell(3.0_f64);
//! let c = sheet.add_cell(0.0_f64);
//!
//! // Three methods encoding a × b = c in each direction.
//! let methods = vec![
//!     Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok((*x) * (*y))),
//!     Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok((*y) / (*x))),
//!     Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok((*y) / (*x))),
//! ];
//! sheet.add_relationship(methods).unwrap();
//!
//! sheet.write(a, 2.0_f64).unwrap();
//! sheet.write(b, 3.0_f64).unwrap();
//! sheet.propagate().unwrap();
//!
//! assert_eq!(*sheet.read::<f64>(c).unwrap(), 6.0);
//! ```

pub mod error;

pub use error::Error;
```

Run: `cargo test -p property-model`
Expected: FAIL — `Error` is not defined yet

- [ ] **Step 5: Implement `Error` in `error.rs`**

Add the enum and its impls above the `#[cfg(test)]` block:

```rust
use std::any::TypeId;

/// Errors returned by `Sheet` operations and propagation.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A value's `TypeId` did not match the cell's registered `TypeId`.
    TypeMismatch { expected: TypeId, found: TypeId },

    /// A `CellId` or `RelationshipId` was not found in the sheet.
    InvalidId,

    /// No valid method assignment exists (overconstrained).
    Conflict,

    /// The selected methods form a cycle.
    Cycle,

    /// A method's function returned an error during execution.
    MethodFailed(anyhow::Error),

    /// A method is structurally invalid (e.g. inputs ∩ outputs is non-empty,
    /// or the outputs list is empty).
    InvalidMethod,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TypeMismatch { expected, found } => {
                write!(f, "type mismatch: expected {expected:?}, found {found:?}")
            }
            Error::InvalidId => write!(f, "invalid cell or relationship id"),
            Error::Conflict => write!(f, "no valid method assignment (overconstrained)"),
            Error::Cycle => write!(f, "selected methods form a cycle"),
            Error::MethodFailed(e) => write!(f, "method execution failed: {e}"),
            Error::InvalidMethod => write!(f, "method is structurally invalid"),
        }
    }
}

impl std::error::Error for Error {
    /// Returns the underlying `anyhow::Error` source for `MethodFailed`.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let Error::MethodFailed(e) = self {
            Some(e.as_ref())
        } else {
            None
        }
    }
}
```

- [ ] **Step 6: Run tests and verify they pass**

Run: `cargo test -p property-model`
Expected: all tests in `error::tests` pass

- [ ] **Step 7: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
git add property-model/ Cargo.toml
git commit -m "feat(property-model): scaffold crate with Error type"
```

---

### Task 2: Cell and Relationship Types

**Files:**
- Create: `property-model/src/cell.rs`
- Create: `property-model/src/relationship.rs`
- Modify: `property-model/src/lib.rs` (add module declarations)

**Interfaces:**
- Consumes: `Error` from Task 1
- Produces: `CellId`, `CellData` (pub(crate)); `RelationshipId`, `RelationshipData` (pub(crate)); `Method` (pub, empty shell for now)

- [ ] **Step 1: Write failing tests for cell and relationship types**

Create `property-model/src/cell.rs`:

```rust
//! Value cells in the property model bipartite graph.
//!
//! Cells are accessed exclusively through [`crate::sheet::Sheet`].

use std::any::{Any, TypeId};

use slotmap::new_key_type;

use crate::relationship::RelationshipId;

new_key_type! {
    /// A stable handle to a cell in a [`crate::sheet::Sheet`].
    pub struct CellId;
}

/// Internal storage for a single value cell.
pub(crate) struct CellData {
    /// The type-erased current value.
    pub(crate) value: Box<dyn Any>,
    /// The `TypeId` of the value, fixed at cell creation.
    pub(crate) type_id: TypeId,
    /// Monotonically increasing write-recency clock; incremented by `Sheet::write`.
    pub(crate) strength: u64,
    /// Set during `Sheet::propagate`; cleared by `Sheet::clear_changed`.
    pub(crate) changed: bool,
    /// Relationships that include this cell.
    pub(crate) adj: Vec<RelationshipId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_data_initial_state() {
        let data = CellData {
            value: Box::new(42_i32),
            type_id: TypeId::of::<i32>(),
            strength: 0,
            changed: false,
            adj: vec![],
        };
        assert_eq!(data.type_id, TypeId::of::<i32>());
        assert_eq!(data.strength, 0);
        assert!(!data.changed);
        assert!(data.adj.is_empty());
        assert_eq!(*data.value.downcast_ref::<i32>().unwrap(), 42);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail (CellId undefined)**

Run: `cargo test -p property-model 2>&1 | head -20`
Expected: compile error — `RelationshipId` not found

- [ ] **Step 3: Create `property-model/src/relationship.rs`**

```rust
//! Relationships and methods in the property model bipartite graph.
//!
//! Each relationship holds a list of [`Method`]s; the planner selects one
//! method per relationship at propagation time based on cell strength.

use std::any::{Any, TypeId};

use slotmap::new_key_type;

use crate::cell::CellId;

new_key_type! {
    /// A stable handle to a relationship in a [`crate::sheet::Sheet`].
    pub struct RelationshipId;
}

/// A single method within a relationship.
///
/// A method declares a disjoint partition of some cells into inputs and
/// outputs, plus a type-erased function that computes the outputs from the
/// inputs. TypeIds for inputs and outputs are stored alongside the function
/// and validated at [`crate::sheet::Sheet::add_relationship`] time.
pub struct Method {
    pub(crate) inputs: Vec<CellId>,
    pub(crate) outputs: Vec<CellId>,
    pub(crate) input_types: Vec<TypeId>,
    pub(crate) output_types: Vec<TypeId>,
    pub(crate) function: Box<dyn Fn(&[&dyn Any]) -> Result<Vec<Box<dyn Any>>, anyhow::Error>>,
}

/// Internal storage for a relationship.
pub(crate) struct RelationshipData {
    pub(crate) methods: Vec<Method>,
    /// Union of all cell IDs referenced by any method in this relationship.
    pub(crate) adj: Vec<CellId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relationship_id_is_copy() {
        fn takes_copy<T: Copy>(_: T) {}
        // RelationshipId must be Copy so it can be stored in adjacency Vecs cheaply.
        // This test fails to compile if RelationshipId is not Copy.
        takes_copy(RelationshipId::default());
    }

    #[test]
    fn cell_id_is_copy() {
        fn takes_copy<T: Copy>(_: T) {}
        takes_copy(CellId::default());
    }
}
```

- [ ] **Step 4: Wire modules into `lib.rs` and run tests**

Add to `property-model/src/lib.rs`:

```rust
pub mod cell;
pub mod error;
pub mod relationship;

pub use cell::CellId;
pub use error::Error;
pub use relationship::{Method, RelationshipId};
```

Run: `cargo test -p property-model`
Expected: all tests pass

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
git add property-model/src/cell.rs property-model/src/relationship.rs property-model/src/lib.rs
git commit -m "feat(property-model): add CellId, RelationshipId, and Method shell"
```

---

### Task 3: Method Construction API

**Files:**
- Modify: `property-model/src/relationship.rs` (add `Method::new`, `from_fn_1_1`, `from_fn_2_1`)

**Interfaces:**
- Consumes: `CellId`, `RelationshipId`, `TypeId`
- Produces: `Method::new`, `Method::from_fn_1_1`, `Method::from_fn_2_1` — constructors verified by the TypeId validation in Task 4's `add_relationship`

Note: `Method` constructors cannot be tested in complete isolation because meaningful `CellId` values come from a `Sheet`. Method construction is tested implicitly through `Sheet::add_relationship` in Task 4. The tests here verify that TypeIds are stored correctly using `Method::new`.

- [ ] **Step 1: Write failing test for `Method::new`**

Add to `relationship.rs` test module:

```rust
    #[test]
    fn method_new_stores_types_and_cell_ids() {
        use slotmap::SlotMap;
        use crate::cell::CellId;

        // Create real CellIds from a SlotMap to avoid using the default (null) key.
        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());
        let b = map.insert(());
        let c = map.insert(());

        let method = Method::new(
            vec![a, b],
            vec![c],
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
            vec![TypeId::of::<i32>()],
            |args| {
                let x = args[0].downcast_ref::<i32>().unwrap();
                let y = args[1].downcast_ref::<i32>().unwrap();
                Ok(vec![Box::new(x + y)])
            },
        );

        assert_eq!(method.inputs, vec![a, b]);
        assert_eq!(method.outputs, vec![c]);
        assert_eq!(method.input_types, vec![TypeId::of::<i32>(), TypeId::of::<i32>()]);
        assert_eq!(method.output_types, vec![TypeId::of::<i32>()]);

        // Verify the stored function works.
        let x: i32 = 3;
        let y: i32 = 4;
        let result = (method.function)(&[&x, &y]).unwrap();
        assert_eq!(*result[0].downcast_ref::<i32>().unwrap(), 7);
    }

    #[test]
    fn from_fn_1_1_stores_correct_type_ids() {
        use slotmap::SlotMap;
        use crate::cell::CellId;

        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());
        let b = map.insert(());

        let method = Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2));

        assert_eq!(method.inputs, vec![a]);
        assert_eq!(method.outputs, vec![b]);
        assert_eq!(method.input_types, vec![TypeId::of::<i32>()]);
        assert_eq!(method.output_types, vec![TypeId::of::<i32>()]);

        let x: i32 = 5;
        let result = (method.function)(&[&x]).unwrap();
        assert_eq!(*result[0].downcast_ref::<i32>().unwrap(), 10);
    }

    #[test]
    fn from_fn_2_1_stores_correct_type_ids() {
        use slotmap::SlotMap;
        use crate::cell::CellId;

        let mut map: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = map.insert(());
        let b = map.insert(());
        let c = map.insert(());

        let method = Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok((*x) * (*y)));

        assert_eq!(method.inputs, vec![a, b]);
        assert_eq!(method.outputs, vec![c]);
        assert_eq!(method.input_types, vec![TypeId::of::<f64>(), TypeId::of::<f64>()]);
        assert_eq!(method.output_types, vec![TypeId::of::<f64>()]);

        let x: f64 = 2.0;
        let y: f64 = 3.0;
        let result = (method.function)(&[&x, &y]).unwrap();
        assert_eq!(*result[0].downcast_ref::<f64>().unwrap(), 6.0);
    }
```

Run: `cargo test -p property-model 2>&1 | head -20`
Expected: compile error — `Method::new`, `from_fn_1_1`, `from_fn_2_1` not defined

- [ ] **Step 2: Implement the constructors in `relationship.rs`**

Add these impl blocks above the `#[cfg(test)]` block:

```rust
impl Method {
    /// Creates a method from explicit TypeIds and a type-erased function.
    ///
    /// - Precondition: `inputs.len() == input_types.len()` and `outputs.len() == output_types.len()`.
    /// - Precondition: The function must return exactly `outputs.len()` values in the correct order.
    pub fn new(
        inputs: Vec<CellId>,
        outputs: Vec<CellId>,
        input_types: Vec<TypeId>,
        output_types: Vec<TypeId>,
        f: impl Fn(&[&dyn Any]) -> Result<Vec<Box<dyn Any>>, anyhow::Error> + 'static,
    ) -> Self {
        Method {
            inputs,
            outputs,
            input_types,
            output_types,
            function: Box::new(f),
        }
    }

    /// Creates a 1-input, 1-output method from a typed closure.
    ///
    /// TypeIds for `A` and `B` are captured automatically. The method is validated
    /// against its cell registrations when passed to [`crate::sheet::Sheet::add_relationship`].
    pub fn from_fn_1_1<A, B, F>(input: CellId, output: CellId, f: F) -> Self
    where
        A: Any + 'static,
        B: Any + 'static,
        F: Fn(&A) -> Result<B, anyhow::Error> + 'static,
    {
        Method {
            inputs: vec![input],
            outputs: vec![output],
            input_types: vec![TypeId::of::<A>()],
            output_types: vec![TypeId::of::<B>()],
            function: Box::new(move |args| {
                let a = args[0]
                    .downcast_ref::<A>()
                    .expect("type checked at add_relationship");
                Ok(vec![Box::new(f(a)?)])
            }),
        }
    }

    /// Creates a 2-input, 1-output method from a typed closure.
    ///
    /// `inputs[0]` maps to `A` and `inputs[1]` maps to `B`. TypeIds are captured
    /// automatically. The method is validated when passed to
    /// [`crate::sheet::Sheet::add_relationship`].
    pub fn from_fn_2_1<A, B, C, F>(inputs: [CellId; 2], output: CellId, f: F) -> Self
    where
        A: Any + 'static,
        B: Any + 'static,
        C: Any + 'static,
        F: Fn(&A, &B) -> Result<C, anyhow::Error> + 'static,
    {
        Method {
            inputs: inputs.to_vec(),
            outputs: vec![output],
            input_types: vec![TypeId::of::<A>(), TypeId::of::<B>()],
            output_types: vec![TypeId::of::<C>()],
            function: Box::new(move |args| {
                let a = args[0]
                    .downcast_ref::<A>()
                    .expect("type checked at add_relationship");
                let b = args[1]
                    .downcast_ref::<B>()
                    .expect("type checked at add_relationship");
                Ok(vec![Box::new(f(a, b)?)])
            }),
        }
    }
}
```

- [ ] **Step 3: Run tests and verify they pass**

Run: `cargo test -p property-model`
Expected: all tests pass

- [ ] **Step 4: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
git add property-model/src/relationship.rs
git commit -m "feat(property-model): implement Method construction helpers"
```

---

### Task 4: Sheet Construction API

**Files:**
- Create: `property-model/src/sheet.rs`
- Modify: `property-model/src/lib.rs` (add `sheet` module, re-export `Sheet`)

**Interfaces:**
- Consumes: `CellId`, `CellData`, `RelationshipId`, `RelationshipData`, `Method`, `Error`
- Produces: `Sheet::new`, `add_cell`, `add_relationship`, `write`, `read`

- [ ] **Step 1: Write failing tests**

Create `property-model/src/sheet.rs` with only the test module initially:

```rust
//! The [`Sheet`] owns and manages a property model constraint graph.
//!
//! All cells and relationships are created through the sheet and are
//! destroyed when the sheet is dropped.

#[cfg(test)]
mod tests {
    use std::any::TypeId;
    use crate::{Error, Method, Sheet};

    #[test]
    fn add_cell_returns_distinct_ids() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(1_i32);
        let b = sheet.add_cell(2_i32);
        assert_ne!(a, b);
    }

    #[test]
    fn write_read_roundtrip() {
        let mut sheet = Sheet::new();
        let id = sheet.add_cell(42_i32);
        sheet.write(id, 99_i32).unwrap();
        assert_eq!(*sheet.read::<i32>(id).unwrap(), 99);
    }

    #[test]
    fn write_wrong_type_returns_type_mismatch() {
        let mut sheet = Sheet::new();
        let id = sheet.add_cell(0_i32);
        assert!(matches!(
            sheet.write(id, 1.0_f64),
            Err(Error::TypeMismatch { .. })
        ));
    }

    #[test]
    fn read_wrong_type_returns_type_mismatch() {
        let mut sheet = Sheet::new();
        let id = sheet.add_cell(0_i32);
        assert!(matches!(
            sheet.read::<f64>(id),
            Err(Error::TypeMismatch { .. })
        ));
    }

    #[test]
    fn add_relationship_empty_methods_returns_invalid_method() {
        let mut sheet = Sheet::new();
        assert!(matches!(
            sheet.add_relationship(vec![]),
            Err(Error::InvalidMethod)
        ));
    }

    #[test]
    fn add_relationship_type_mismatch_returns_error() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        // Method declares f64 input but cell holds i32.
        let method = Method::from_fn_1_1(a, b, |x: &f64| Ok(*x * 2.0));
        assert!(matches!(
            sheet.add_relationship(vec![method]),
            Err(Error::TypeMismatch { .. })
        ));
    }

    #[test]
    fn add_relationship_overlapping_inputs_outputs_returns_invalid_method() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        // Cell `a` appears in both inputs and outputs.
        let method = Method::new(
            vec![a, b],
            vec![a],
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
            vec![TypeId::of::<i32>()],
            |args| Ok(vec![Box::new(*args[0].downcast_ref::<i32>().unwrap())]),
        );
        assert!(matches!(
            sheet.add_relationship(vec![method]),
            Err(Error::InvalidMethod)
        ));
    }

    #[test]
    fn add_relationship_empty_outputs_returns_invalid_method() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let method = Method::new(
            vec![a],
            vec![],          // no outputs
            vec![TypeId::of::<i32>()],
            vec![],
            |_| Ok(vec![]),
        );
        assert!(matches!(
            sheet.add_relationship(vec![method]),
            Err(Error::InvalidMethod)
        ));
    }

    #[test]
    fn add_relationship_returns_distinct_ids() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let r1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let c = sheet.add_cell(0_i32);
        let r2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(b, c, |x: &i32| Ok(*x))])
            .unwrap();
        assert_ne!(r1, r2);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Add `pub mod sheet;` and `pub use sheet::Sheet;` to `lib.rs` first (so the module exists but is empty):

```rust
// At top of lib.rs, add:
pub mod sheet;
pub use sheet::Sheet;
```

Run: `cargo test -p property-model 2>&1 | head -20`
Expected: compile errors — `Sheet` not defined

- [ ] **Step 3: Implement `Sheet` in `sheet.rs`**

Add above the `#[cfg(test)]` block in `sheet.rs`:

```rust
use std::any::{Any, TypeId};

use slotmap::SlotMap;

use crate::{
    cell::{CellData, CellId},
    error::Error,
    relationship::{Method, RelationshipData, RelationshipId},
};

/// Owns a complete property model constraint graph.
///
/// Create cells with [`Sheet::add_cell`], define multi-way constraints with
/// [`Sheet::add_relationship`], write input values with [`Sheet::write`],
/// then call [`Sheet::propagate`] to execute the planning pass and update
/// derived cells.
pub struct Sheet {
    pub(crate) cells: SlotMap<CellId, CellData>,
    pub(crate) relationships: SlotMap<RelationshipId, RelationshipData>,
    /// Explicit insertion-order list; `SlotMap` does not guarantee iteration order.
    pub(crate) relationship_order: Vec<RelationshipId>,
    pub(crate) changed_cells: Vec<CellId>,
    /// Global write-recency clock; incremented by each `write()` call.
    next_strength: u64,
}

impl Sheet {
    /// Creates an empty sheet with no cells or relationships.
    pub fn new() -> Self {
        Sheet {
            cells: SlotMap::with_key(),
            relationships: SlotMap::with_key(),
            relationship_order: Vec::new(),
            changed_cells: Vec::new(),
            next_strength: 0,
        }
    }

    /// Registers a cell with an initial value and returns a stable handle.
    ///
    /// The cell's `TypeId` is fixed at creation time; subsequent `write` and
    /// `read` calls that use a different type will return `Error::TypeMismatch`.
    pub fn add_cell<T: Any + 'static>(&mut self, value: T) -> CellId {
        self.cells.insert(CellData {
            value: Box::new(value),
            type_id: TypeId::of::<T>(),
            strength: 0,
            changed: false,
            adj: Vec::new(),
        })
    }

    /// Registers a relationship.
    ///
    /// Validates every method: TypeIds must match the registered cells; inputs
    /// and outputs must be disjoint; each method must have at least one output.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidMethod` — `methods` is empty, a method has no outputs, or
    ///   a method's inputs and outputs overlap.
    /// - `Error::InvalidId` — a `CellId` in any method is not found in this sheet.
    /// - `Error::TypeMismatch` — a method's declared `TypeId` does not match the
    ///   cell's registered `TypeId`.
    pub fn add_relationship(&mut self, methods: Vec<Method>) -> Result<RelationshipId, Error> {
        if methods.is_empty() {
            return Err(Error::InvalidMethod);
        }

        for method in &methods {
            if method.outputs.is_empty() {
                return Err(Error::InvalidMethod);
            }

            // inputs ∩ outputs must be empty
            for output in &method.outputs {
                if method.inputs.contains(output) {
                    return Err(Error::InvalidMethod);
                }
            }

            // input count must match declared input_types count
            if method.inputs.len() != method.input_types.len()
                || method.outputs.len() != method.output_types.len()
            {
                return Err(Error::InvalidMethod);
            }

            for (&cell_id, &declared) in
                method.inputs.iter().zip(method.input_types.iter())
            {
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                    });
                }
            }

            for (&cell_id, &declared) in
                method.outputs.iter().zip(method.output_types.iter())
            {
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                    });
                }
            }
        }

        // Collect all adjacent cells (union across all methods, deduplicated).
        let mut adj: Vec<CellId> = Vec::new();
        for method in &methods {
            for &cell_id in method.inputs.iter().chain(method.outputs.iter()) {
                if !adj.contains(&cell_id) {
                    adj.push(cell_id);
                }
            }
        }

        let rel_id = self.relationships.insert(RelationshipData {
            methods,
            adj: adj.clone(),
        });

        for cell_id in adj {
            if let Some(cell) = self.cells.get_mut(cell_id) {
                if !cell.adj.contains(&rel_id) {
                    cell.adj.push(rel_id);
                }
            }
        }

        self.relationship_order.push(rel_id);
        Ok(rel_id)
    }

    /// Writes a value to a cell, incrementing its strength (write-recency clock).
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `id` is not a cell in this sheet.
    /// - `Error::TypeMismatch` — `T` does not match the cell's registered `TypeId`.
    pub fn write<T: Any + 'static>(&mut self, id: CellId, value: T) -> Result<(), Error> {
        let cell = self.cells.get_mut(id).ok_or(Error::InvalidId)?;
        if cell.type_id != TypeId::of::<T>() {
            return Err(Error::TypeMismatch {
                expected: cell.type_id,
                found: TypeId::of::<T>(),
            });
        }
        self.next_strength += 1;
        cell.strength = self.next_strength;
        cell.value = Box::new(value);
        Ok(())
    }

    /// Returns a reference to the current value of a cell.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `id` is not a cell in this sheet.
    /// - `Error::TypeMismatch` — `T` does not match the cell's registered `TypeId`.
    pub fn read<T: Any + 'static>(&self, id: CellId) -> Result<&T, Error> {
        let cell = self.cells.get(id).ok_or(Error::InvalidId)?;
        if cell.type_id != TypeId::of::<T>() {
            return Err(Error::TypeMismatch {
                expected: cell.type_id,
                found: TypeId::of::<T>(),
            });
        }
        Ok(cell.value.downcast_ref::<T>().expect("type checked above"))
    }

    /// Iterates the cells whose values changed during the last `propagate()` call.
    ///
    /// - Complexity: O(n) where n is the number of changed cells.
    pub fn changed(&self) -> impl Iterator<Item = CellId> + '_ {
        self.changed_cells.iter().copied()
    }

    /// Clears the changed-cell set and resets each cell's `changed` flag.
    ///
    /// Call after processing the results of `propagate()`.
    ///
    /// - Complexity: O(n) where n is the number of changed cells.
    pub fn clear_changed(&mut self) {
        for id in std::mem::take(&mut self.changed_cells) {
            if let Some(cell) = self.cells.get_mut(id) {
                cell.changed = false;
            }
        }
    }
}

impl Default for Sheet {
    /// Returns `Sheet::new()`.
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p property-model`
Expected: all tests pass

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
git add property-model/src/sheet.rs property-model/src/lib.rs
git commit -m "feat(property-model): implement Sheet construction API"
```

---

### Task 5: Planner

**Files:**
- Create: `property-model/src/planner.rs`
- Modify: `property-model/src/lib.rs` (add `mod planner;` — private module)

**Interfaces:**
- Consumes: `CellData`, `CellId`, `RelationshipData`, `RelationshipId`, `Error`
- Produces: `pub(crate) fn plan(...)  -> Result<Plan, Error>`; `pub(crate) struct Plan { execution_order: Vec<(RelationshipId, usize)> }`

- [ ] **Step 1: Write failing tests**

Create `property-model/src/planner.rs`:

```rust
//! Planning pass: selects one method per relationship and topologically orders execution.
//!
//! Phase 1 greedily assigns a method to each relationship by minimising the minimum
//! strength (write-recency clock value) of the method's output cells — preferring to
//! derive cells that were written least recently. Phase 2 runs Kahn's algorithm to
//! produce a dependency-ordered execution sequence.

#[cfg(test)]
mod tests {
    use crate::{Error, Method, Sheet};

    // Propagation-behavior tests (single_method, strength_drives_selection, chained) live in
    // Task 6's integration tests, where the full propagate() implementation is wired.

    #[test]
    fn conflict_returns_error() {
        // Two relationships both want to overwrite the same cell; only one method
        // each, and both output the same cell.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let out = sheet.add_cell(0_i32);

        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, out, |x: &i32| Ok(*x))])
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(b, out, |x: &i32| Ok(*x))])
            .unwrap();

        assert!(matches!(sheet.propagate(), Err(Error::Conflict)));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Add `mod planner;` (private) to `lib.rs`:

```rust
mod planner;
```

Run: `cargo test -p property-model planner 2>&1 | head -30`
Expected: compile errors — `plan`, `Plan` not defined

- [ ] **Step 3: Implement the planner**

Add above the `#[cfg(test)]` block in `planner.rs`:

```rust
use std::collections::{HashMap, HashSet, VecDeque};

use slotmap::SlotMap;

use crate::{
    cell::{CellData, CellId},
    error::Error,
    relationship::{RelationshipData, RelationshipId},
};

/// The output of the planning pass.
pub(crate) struct Plan {
    /// Selected `(RelationshipId, method_index)` pairs in execution order.
    pub(crate) execution_order: Vec<(RelationshipId, usize)>,
}

/// Runs Phase 1 (greedy method selection) and Phase 2 (topological sort).
///
/// Phase 1 iterates `relationship_order` and, for each relationship, selects
/// the method whose output cells have the minimum write-strength — preferring
/// to derive cells that were written least recently. A method is invalid if
/// any of its output cells were already claimed by an earlier relationship.
///
/// Phase 2 runs Kahn's algorithm over the selected methods, where an edge
/// A→B means method A writes a cell that method B reads as input.
///
/// # Errors
///
/// - `Error::Conflict` — no valid method exists for some relationship.
/// - `Error::Cycle` — the selected methods form a dependency cycle.
///
/// - Complexity: O(R·M + N) where R = relationships, M = methods per
///   relationship, N = total cells across all selected methods.
pub(crate) fn plan(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    relationship_order: &[RelationshipId],
) -> Result<Plan, Error> {
    // ── Phase 1: greedy method selection ────────────────────────────────────
    let mut claimed: HashSet<CellId> = HashSet::new();
    let mut selected: Vec<(RelationshipId, usize)> = Vec::new();

    for &rel_id in relationship_order {
        let rel = &relationships[rel_id];

        let best = rel
            .methods
            .iter()
            .enumerate()
            .filter(|(_, m)| m.outputs.iter().all(|o| !claimed.contains(o)))
            .min_by_key(|(_, m)| {
                m.outputs
                    .iter()
                    .map(|&id| cells[id].strength)
                    .min()
                    .unwrap_or(0)
            });

        let (method_idx, method) = best.ok_or(Error::Conflict)?;

        for &output in &method.outputs {
            claimed.insert(output);
        }
        selected.push((rel_id, method_idx));
    }

    // ── Phase 2: Kahn's topological sort ────────────────────────────────────
    let n = selected.len();

    // Map each output cell to the index (in `selected`) of the method that produces it.
    let mut producer: HashMap<CellId, usize> = HashMap::new();
    for (i, (rel_id, method_idx)) in selected.iter().enumerate() {
        let method = &relationships[*rel_id].methods[*method_idx];
        for &output in &method.outputs {
            producer.insert(output, i);
        }
    }

    // Adjacency list and in-degree for the execution DAG.
    let mut adj: Vec<Vec<usize>> = vec![vec![]; n];
    let mut in_degree: Vec<usize> = vec![0; n];

    for (i, (rel_id, method_idx)) in selected.iter().enumerate() {
        let method = &relationships[*rel_id].methods[*method_idx];
        for &input in &method.inputs {
            if let Some(&p) = producer.get(&input) {
                if p != i {
                    adj[p].push(i);
                    in_degree[i] += 1;
                }
            }
        }
    }

    let mut queue: VecDeque<usize> = in_degree
        .iter()
        .enumerate()
        .filter(|(_, &d)| d == 0)
        .map(|(i, _)| i)
        .collect();

    let mut order: Vec<usize> = Vec::with_capacity(n);
    while let Some(node) = queue.pop_front() {
        order.push(node);
        for &next in &adj[node] {
            in_degree[next] -= 1;
            if in_degree[next] == 0 {
                queue.push_back(next);
            }
        }
    }

    if order.len() != n {
        return Err(Error::Cycle);
    }

    Ok(Plan {
        execution_order: order.iter().map(|&i| selected[i]).collect(),
    })
}
```

- [ ] **Step 4: Add `Sheet::propagate` stub (needed for tests to compile)**

Tests call `sheet.propagate()`. Add a minimal implementation to `sheet.rs` (the full implementation is in Task 6; this stub returns `Ok(())` to let planner tests compile):

```rust
    /// Runs the planning pass and executes the selected methods.
    ///
    /// After propagation, call [`Sheet::changed`] to inspect which cells were updated,
    /// and [`Sheet::clear_changed`] when done.
    ///
    /// # Errors
    ///
    /// - `Error::Conflict` — no valid method assignment exists.
    /// - `Error::Cycle` — the selected methods form a dependency cycle.
    /// - `Error::MethodFailed` — a method's function returned an error.
    pub fn propagate(&mut self) -> Result<(), Error> {
        let _ = crate::planner::plan(&self.cells, &self.relationships, &self.relationship_order)?;
        Ok(()) // execution wired in Task 6
    }
```

- [ ] **Step 5: Run all tests**

Run: `cargo test -p property-model`
Expected: all tests pass, including `conflict_returns_error`

- [ ] **Step 6: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
git add property-model/src/planner.rs property-model/src/lib.rs property-model/src/sheet.rs
git commit -m "feat(property-model): implement planner (method selection + topological sort)"
```

---

### Task 6: Propagation and Change Tracking

**Files:**
- Modify: `property-model/src/sheet.rs` (replace stub `propagate`; wire execution and change tracking)
- Create: `property-model/tests/integration.rs`

**Interfaces:**
- Consumes: `Plan` from `planner::plan`
- Produces: `Sheet::propagate`, `Sheet::changed`, `Sheet::clear_changed` fully implemented

- [ ] **Step 1: Write the integration test file**

Create `property-model/tests/integration.rs`:

```rust
//! End-to-end integration tests for the property-model crate.

use property_model::{Error, Method, Sheet};

#[test]
fn propagate_executes_single_method() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(5_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 3))])
        .unwrap();

    sheet.write(a, 7_i32).unwrap();
    sheet.propagate().unwrap();

    assert_eq!(*sheet.read::<i32>(b).unwrap(), 21);
}

#[test]
fn changed_returns_updated_cells_after_propagate() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x + 1))])
        .unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(b, c, |x: &i32| Ok(*x + 1))])
        .unwrap();

    sheet.write(a, 10_i32).unwrap();
    sheet.propagate().unwrap();

    let changed: Vec<_> = sheet.changed().collect();
    assert_eq!(changed.len(), 2);
    assert!(changed.contains(&b));
    assert!(changed.contains(&c));
}

#[test]
fn clear_changed_empties_the_changed_set() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();

    sheet.write(a, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(sheet.changed().count(), 1);

    sheet.clear_changed();
    assert_eq!(sheet.changed().count(), 0);
}

#[test]
fn propagate_clears_previous_changed_set() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();

    sheet.write(a, 1_i32).unwrap();
    sheet.propagate().unwrap();
    // b changed in first propagation

    sheet.write(a, 2_i32).unwrap();
    sheet.propagate().unwrap();
    // b changed again; changed set should have only cells from this propagation
    let changed: Vec<_> = sheet.changed().collect();
    assert_eq!(changed.len(), 1);
    assert!(changed.contains(&b));
}

#[test]
fn multiway_constraint_derives_weakest_cell() {
    // a * b = c — three methods, one per direction.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0.0_f64);
    let b = sheet.add_cell(0.0_f64);
    let c = sheet.add_cell(0.0_f64);

    let methods = vec![
        Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok((*x) * (*y))),
        Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok((*y) / (*x))),
        Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok((*y) / (*x))),
    ];
    sheet.add_relationship(methods).unwrap();

    // Write a=2 (strength=1), b=3 (strength=2). c.strength=0 is weakest → derive c.
    sheet.write(a, 2.0_f64).unwrap();
    sheet.write(b, 3.0_f64).unwrap();
    sheet.propagate().unwrap();
    assert!((sheet.read::<f64>(c).unwrap() - 6.0).abs() < 1e-10);

    // Write c=12 (strength=3). a.strength=1 is now weakest → derive a.
    sheet.write(c, 12.0_f64).unwrap();
    sheet.propagate().unwrap();
    assert!((sheet.read::<f64>(a).unwrap() - 4.0).abs() < 1e-10);
}

#[test]
fn method_returning_error_propagates_as_method_failed() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0.0_f64);
    let b = sheet.add_cell(0.0_f64);
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |_: &f64| {
            Err(anyhow::anyhow!("intentional error"))
        })])
        .unwrap();

    let result = sheet.propagate();
    assert!(matches!(result, Err(Error::MethodFailed(_))));
}

#[test]
fn cycle_in_selected_methods_returns_cycle_error() {
    // a→b and b→a: each relationship has only one method, forming a cycle.
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    // Relationship 1: inputs=[a], outputs=[b] — but a is input, so the only method writes b.
    // Relationship 2: inputs=[b], outputs=[a] — writes a.
    // Planner selects both (no conflict on outputs), but they form a cycle.
    sheet
        .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
        .unwrap();
    sheet
        .add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))])
        .unwrap();

    assert!(matches!(sheet.propagate(), Err(Error::Cycle)));
}
```

- [ ] **Step 2: Run integration tests to verify they fail**

Run: `cargo test -p property-model --test integration 2>&1 | head -30`
Expected: multiple FAILs — `propagate` is still a stub

- [ ] **Step 3: Implement full `propagate` in `sheet.rs`**

Replace the stub `propagate` with:

```rust
    pub fn propagate(&mut self) -> Result<(), Error> {
        // Clear the previous changed set.
        for id in std::mem::take(&mut self.changed_cells) {
            if let Some(cell) = self.cells.get_mut(id) {
                cell.changed = false;
            }
        }

        let plan =
            crate::planner::plan(&self.cells, &self.relationships, &self.relationship_order)?;

        for (rel_id, method_idx) in plan.execution_order {
            // Gather inputs in a scoped block so the shared borrow on `self.cells`
            // is released before the mutable borrow below.
            let outputs = {
                let method = &self.relationships[rel_id].methods[method_idx];
                let inputs: Vec<&dyn Any> = method
                    .inputs
                    .iter()
                    .map(|&id| self.cells[id].value.as_ref())
                    .collect();
                (method.function)(&inputs).map_err(Error::MethodFailed)?
            };

            // Clone output IDs so we can release the immutable borrow on
            // `self.relationships` before mutably borrowing `self.cells`.
            let output_ids: Vec<CellId> =
                self.relationships[rel_id].methods[method_idx].outputs.clone();

            for (cell_id, new_value) in output_ids.into_iter().zip(outputs) {
                let cell = &mut self.cells[cell_id];
                cell.value = new_value;
                if !cell.changed {
                    cell.changed = true;
                    self.changed_cells.push(cell_id);
                }
            }
        }

        Ok(())
    }
```

- [ ] **Step 4: Run all tests and verify they pass**

Run: `cargo test --workspace`
Expected: all tests pass, including planner unit tests and integration tests

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
git add property-model/src/sheet.rs property-model/tests/integration.rs
git commit -m "feat(property-model): implement propagation and change tracking"
```

---

## Self-Review

### Spec coverage

| Spec requirement | Task |
|---|---|
| New crate `property-model` with `slotmap`, `anyhow` | Task 1 |
| `CellId`, `RelationshipId` — distinct typed stable handles | Task 2 |
| `CellData` with `value`, `type_id`, `strength`, `changed`, `adj` | Task 2 |
| `Method` with typed helpers `from_fn_1_1`, `from_fn_2_1`, and escape-hatch `new` | Task 3 |
| TypeId checking at `add_relationship` time | Task 4 |
| TypeId checking at `write` time | Task 4 |
| `InvalidMethod` for empty outputs, inputs ∩ outputs overlap | Task 4 |
| `Sheet::write` increments cell strength | Task 4 |
| `Sheet::read` — typed read | Task 4 |
| Planner Phase 1: greedy method selection by minimum output strength | Task 5 |
| Planner Phase 2: Kahn's topological sort | Task 5 |
| `Error::Conflict` when no valid method assignment | Task 5 |
| `Error::Cycle` when selected methods form a cycle | Task 5 |
| `Sheet::propagate` — calls planner then executes in order | Task 6 |
| `Sheet::changed` — iterates changed cells | Task 6 |
| `Sheet::clear_changed` — resets changed state | Task 6 |
| `Error::MethodFailed(anyhow::Error)` on method function error | Task 6 |
| Insertion-order processing in Phase 1 (via `relationship_order: Vec<RelationshipId>`) | Task 4 |

### Placeholder scan

None found. All steps contain runnable code.

### Type consistency

- `CellId` and `RelationshipId` are defined in Tasks 2 and used consistently through Tasks 3–6.
- `Method` shell defined in Task 2; constructors added in Task 3; used in Tasks 4–6.
- `Plan.execution_order: Vec<(RelationshipId, usize)>` — matches consumption in `Sheet::propagate`.
- `Error` variants used identically across all tasks.
