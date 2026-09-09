# adam-lang Error Location Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `adam_rs::Error` and `adam-lang`'s parser report the relationship/method that actually caused an error, instead of `adam-lang` always falling back to the sheet's opening span.

**Architecture:** `adam-rs` gains a small `ErrorLocation` enum (`MethodIndex(usize)` for a method not yet assigned a `RelationshipId`, `Method(RelationshipId, usize)` for one in an already-registered relationship) attached to the six `Error` variants that can pinpoint a single method. `adam-lang`'s parser resolves `MethodIndex` immediately (it still has the binding's span in scope) and, for `add_conditional`/`add_filter`/`add_out`/`add_requirement`, stops discarding the span it already holds. `ParsedSheet` gains a `method_spans` table so `Method(rel_id, idx)` — raised later, e.g. from `Sheet::propagate()` — can still be translated back to a span. `adam-web-ui`'s `format_adam_error` consults that table as a fallback when a `MethodFailed`/`TypeMismatch` has no finer CEL-internal `SpanContext`.

**Tech Stack:** Rust; `adam-rs`, `adam-lang`, `adam-web-ui` crates; `proc_macro2`/`cel_parser::SourceSpan` for spans; `slotmap`-backed `RelationshipId`.

**Spec:** [docs/superpowers/specs/2026-09-08-adam-lang-error-location-design.md](../specs/2026-09-08-adam-lang-error-location-design.md)

## Global Constraints

- `cargo fmt --all` before every commit.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` must stay clean (run at minimum after Task 5 and Task 8, and again before the final commit).
- `cargo build --workspace` / `cargo test --workspace` must produce zero compiler warnings (an unused `mut`, unused import, etc.) — not just zero clippy findings.
- `adam-rs`'s `Error` enum is `#[non_exhaustive]`; every new/changed variant stays `#[non_exhaustive]`-compatible (no exhaustive external match relies on this crate).
- Out of scope: `Error::Cycle`/`Conflict`/`FilterCycle` (tracked as [stlab/cel-rs#188](https://github.com/stlab/cel-rs/issues/188)); threading `method_spans` through `begin`'s live Dioxus UI (tracked as a follow-up issue opened in Task 8).

---

### Task 1: `adam_rs::ErrorLocation`

**Files:**
- Modify: `adam-rs/src/error.rs`

**Interfaces:**
- Produces: `pub enum ErrorLocation { MethodIndex(usize), Method(RelationshipId, usize) }` (`#[non_exhaustive]`, `Debug, Clone, Copy, PartialEq, Eq, Hash`); `impl Error { pub fn location(&self) -> Option<ErrorLocation> }` — returns `None` for every variant until Tasks 2–5 wire each one up.

- [ ] **Step 1: Write the failing test**

Add to `adam-rs/src/error.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn error_location_variants_are_distinct() {
    let a = ErrorLocation::MethodIndex(0);
    let b = ErrorLocation::MethodIndex(1);
    let c = ErrorLocation::Method(RelationshipId::default(), 0);
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_eq!(a, ErrorLocation::MethodIndex(0));
}

#[test]
fn location_is_none_for_a_locationless_variant() {
    assert_eq!(Error::InvalidId.location(), None);
}
```

Add `use crate::RelationshipId;` to the test module's existing `use super::*;` scope if `RelationshipId` isn't already reachable from `error.rs` — check `adam-rs/src/lib.rs`'s `pub use` list; `RelationshipId` is re-exported from `adam-rs/src/relationship.rs`, so `use crate::relationship::RelationshipId;` at the top of `error.rs` (outside `#[cfg(test)]`, since `ErrorLocation` itself needs it) makes it available to both.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --package adam-rs error_location_variants_are_distinct`
Expected: FAIL with "cannot find type `ErrorLocation`" (doesn't exist yet).

- [ ] **Step 3: Implement `ErrorLocation` and `Error::location()`**

At the top of `adam-rs/src/error.rs`, after the existing `use std::any::TypeId;`:

```rust
use crate::relationship::RelationshipId;

/// Identifies which sheet component an error originates from, for translating back to a
/// source location.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorLocation {
    /// Index into the `methods` `Vec` passed to `add_relationship`, before any
    /// `RelationshipId` exists for it.
    MethodIndex(usize),
    /// A method within an already-registered relationship.
    Method(RelationshipId, usize),
}
```

At the end of the `impl std::error::Error for Error { ... }` block, add a new `impl Error` block:

```rust
impl Error {
    /// Returns the sheet component (relationship method) this error originates from, if
    /// known. `None` for variants that never track a location, or when a location
    /// legitimately doesn't apply to this occurrence (e.g. `add_relationship(vec![])`'s
    /// `InvalidMethod`).
    pub fn location(&self) -> Option<ErrorLocation> {
        None
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --package adam-rs error_location_variants_are_distinct location_is_none_for_a_locationless_variant`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/error.rs
git commit -m "feat(adam-rs): add ErrorLocation and Error::location()"
```

---

### Task 2: `TypeMismatch` location

**Files:**
- Modify: `adam-rs/src/error.rs`
- Modify: `adam-rs/src/sheet.rs`

**Interfaces:**
- Consumes: `ErrorLocation` from Task 1.
- Produces: `Error::TypeMismatch { expected: TypeId, found: TypeId, location: Option<ErrorLocation> }`.

- [ ] **Step 1: Write the failing tests**

In `adam-rs/src/sheet.rs`'s test module, change `add_relationship_type_mismatch_returns_error` (currently at line 2192) to assert the location:

```rust
#[test]
fn add_relationship_type_mismatch_returns_error() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    // Method declares f64 input but cell holds i32.
    let method = Method::from_fn_1_1(a, b, |x: &f64| Ok(*x * 2.0));
    let result = sheet.add_relationship(vec![method]);
    assert!(matches!(result, Err(Error::TypeMismatch { .. })));
    assert_eq!(
        result.unwrap_err().location(),
        Some(ErrorLocation::MethodIndex(0))
    );
}
```

Add a new test for the runtime path, right after `add_relationship_type_mismatch_returns_error`:

```rust
#[test]
fn execute_plan_type_mismatch_reports_the_method_that_produced_it() {
    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(false); // bool cell
    // Declares an i32 output (matching nothing about `b`'s actual bool type at
    // add_relationship time -- add_relationship validates output_types against the cell,
    // so declare bool here to pass that check, then lie about it at runtime below).
    let method = Method::new(
        vec![a],
        vec![b],
        vec![TypeId::of::<i32>()],
        vec![TypeId::of::<bool>()],
        |args| {
            let x = *args[0].downcast_ref::<i32>().unwrap();
            // Lies: returns an i32 though output_types declared bool, reproducing the
            // execute_plan runtime type-mismatch path (add_relationship can't catch this --
            // it only checks the declared TypeId, not what the closure actually returns).
            Ok(vec![Box::new(x) as Box<dyn std::any::Any>])
        },
    );
    let rel_id = sheet.add_relationship(vec![method]).unwrap();
    let result = sheet.propagate();
    assert!(matches!(result, Err(Error::TypeMismatch { .. })));
    assert_eq!(
        result.unwrap_err().location(),
        Some(ErrorLocation::Method(rel_id, 0))
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --package adam-rs add_relationship_type_mismatch_returns_error execute_plan_type_mismatch_reports_the_method_that_produced_it`
Expected: FAIL — `location()` returns `None` (Task 1's stub), and the new test's `Method::new` call doesn't yet compile against the field list until Step 3 lands (compile first, then check assertions fail/pass appropriately). If it fails to compile, that's expected until Step 3.

- [ ] **Step 3: Add the `location` field and wire every construction site**

In `adam-rs/src/error.rs`, change the variant definition:

```rust
    TypeMismatch {
        /// The TypeId registered when the cell was created.
        expected: TypeId,
        /// The TypeId of the value or declaration supplied by the caller.
        found: TypeId,
        /// The method this mismatch was detected in, if known.
        location: Option<ErrorLocation>,
    },
```

Update the `Display` match arm (was `Error::TypeMismatch { expected, found } => {`):

```rust
            Error::TypeMismatch {
                expected, found, ..
            } => {
                write!(f, "type mismatch: expected {expected:?}, found {found:?}")
            }
```

Update `Error::location()`:

```rust
    pub fn location(&self) -> Option<ErrorLocation> {
        match self {
            Error::TypeMismatch { location, .. } => *location,
            _ => None,
        }
    }
```

Update the three test constructions in `error.rs`'s test module (`type_mismatch_fields_convention`, `type_mismatch_display_contains_type_mismatch`, `non_method_failed_variants_have_no_source`) to add `location: None`, e.g.:

```rust
        let e = Error::TypeMismatch {
            expected,
            found,
            location: None,
        };
```

(apply the same `location: None` addition to the other two construction sites at lines 155 and 208).

In `adam-rs/src/sheet.rs`, update all 9 construction sites. `add_relationship`'s per-method loop (the two `TypeMismatch` sites inside it, currently plain `for method in &methods`) becomes:

```rust
        for (idx, method) in methods.iter().enumerate() {
            if method.outputs.is_empty() {
                return Err(Error::InvalidMethod);
            }

            // declared type counts must match cell-id counts
            if method.inputs.len() != method.input_types.len()
                || method.outputs.len() != method.output_types.len()
            {
                return Err(Error::InvalidMethod);
            }

            for (&cell_id, &declared) in method.inputs.iter().zip(method.input_types.iter()) {
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                        location: Some(ErrorLocation::MethodIndex(idx)),
                    });
                }
            }

            for (&cell_id, &declared) in method.outputs.iter().zip(method.output_types.iter()) {
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.kind == CellKind::Source {
                    return Err(Error::InvalidCellKind);
                }
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                        location: Some(ErrorLocation::MethodIndex(idx)),
                    });
                }
            }
        }
```

(`InvalidMethod`/`InvalidCellKind` stay unchanged here — Tasks 3/4 give them `location` — but the loop must already be `.enumerate()`-based since Task 4 reuses this same `idx`.)

`add_conditional` (line 323), `add_requirement` (line 490), `add_filter` (line 607), `write` (line 953), `read` (line 977), `source` (line 1012) all add `location: None`, e.g. `add_conditional`'s:

```rust
                    if cell_data.type_id != declared {
                        return Err(Error::TypeMismatch {
                            expected: cell_data.type_id,
                            found: declared,
                            location: None,
                        });
                    }
```

`execute_plan`'s `TypeMismatch` (inside the `PlanStep::Method(rel_id, method_idx) =>` arm, line 1507):

```rust
                        if found != cell.type_id {
                            return Err(Error::TypeMismatch {
                                expected: cell.type_id,
                                found,
                                location: Some(ErrorLocation::Method(rel_id, method_idx)),
                            });
                        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --package adam-rs`
Expected: PASS — the whole `adam-rs` suite compiles and passes (existing `Error::TypeMismatch { .. }` matches elsewhere already use `..` so they're unaffected).

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/error.rs adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): attach ErrorLocation to TypeMismatch"
```

---

### Task 3: `MismatchedMethodCells` and `DuplicateMethodOutputs` location

**Files:**
- Modify: `adam-rs/src/error.rs`
- Modify: `adam-rs/src/sheet.rs`

**Interfaces:**
- Produces: `Error::MismatchedMethodCells { location: Option<ErrorLocation> }`, `Error::DuplicateMethodOutputs { location: Option<ErrorLocation> }`.

- [ ] **Step 1: Write the failing tests**

In `adam-rs/src/sheet.rs`, update `add_relationship_inconsistent_cell_sets_across_methods_returns_mismatched_method_cells` and `add_relationship_mismatched_cells_returns_error` to assert location `MethodIndex(1)` (method 0 is the baseline; method 1 is the first to diverge from it in both tests):

```rust
    #[test]
    fn add_relationship_inconsistent_cell_sets_across_methods_returns_mismatched_method_cells() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        // Method 0 references {a, b}; method 1 references {b, c} -- inconsistent.
        let result = sheet.add_relationship(vec![
            Method::from_fn_1_1(a, b, |v: &i32| Ok(*v)),
            Method::from_fn_1_1(b, c, |v: &i32| Ok(*v)),
        ]);
        assert!(matches!(result, Err(Error::MismatchedMethodCells { .. })));
        assert_eq!(
            result.unwrap_err().location(),
            Some(ErrorLocation::MethodIndex(1))
        );
    }
```

```rust
    #[test]
    fn add_relationship_mismatched_cells_returns_error() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let d = sheet.add_cell(0_i32);
        // Method 0 spans {a, b}; Method 1 spans {c, d} — mismatched cell sets.
        let result = sheet.add_relationship(vec![
            Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
            Method::from_fn_1_1(c, d, |x: &i32| Ok(*x)),
        ]);
        assert!(matches!(result, Err(Error::MismatchedMethodCells { .. })));
        assert_eq!(
            result.unwrap_err().location(),
            Some(ErrorLocation::MethodIndex(1))
        );
    }
```

Update `add_relationship_duplicate_output_set_across_methods_returns_duplicate_method_outputs` and `add_relationship_duplicate_output_sets_across_methods_returns_error` to assert `MethodIndex(1)` (both tests' second method is the one that collides), and `add_relationship_duplicate_cell_within_own_outputs_returns_error` to assert `MethodIndex(0)` (its single method self-collides):

```rust
    #[test]
    fn add_relationship_duplicate_output_set_across_methods_returns_duplicate_method_outputs() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let result = sheet.add_relationship(vec![
            Method::from_fn_2_1([a, b], b, |x: &i32, _y: &i32| Ok(*x)),
            Method::from_fn_2_1([a, b], b, |_x: &i32, y: &i32| Ok(*y)),
        ]);
        assert!(matches!(result, Err(Error::DuplicateMethodOutputs { .. })));
        assert_eq!(
            result.unwrap_err().location(),
            Some(ErrorLocation::MethodIndex(1))
        );
    }
```

```rust
    #[test]
    fn add_relationship_duplicate_output_sets_across_methods_returns_error() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let result = sheet.add_relationship(vec![
            Method::from_fn_2_1([a, b], c, |x: &i32, y: &i32| Ok(*x + *y)),
            Method::from_fn_2_1([a, b], c, |x: &i32, y: &i32| Ok(*x - *y)),
        ]);
        assert!(matches!(result, Err(Error::DuplicateMethodOutputs { .. })));
        assert_eq!(
            result.unwrap_err().location(),
            Some(ErrorLocation::MethodIndex(1))
        );
    }
```

```rust
    #[test]
    fn add_relationship_duplicate_cell_within_own_outputs_returns_error() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let method = Method::new(
            vec![a],
            vec![b, b],
            vec![TypeId::of::<i32>()],
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
            |args| {
                let x = args[0].downcast_ref::<i32>().unwrap();
                Ok(vec![Box::new(*x), Box::new(*x)])
            },
        );
        let result = sheet.add_relationship(vec![method]);
        assert!(matches!(result, Err(Error::DuplicateMethodOutputs { .. })));
        assert_eq!(
            result.unwrap_err().location(),
            Some(ErrorLocation::MethodIndex(0))
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --package adam-rs mismatched_method_cells duplicate_method_outputs duplicate_cell_within_own_outputs`
Expected: FAIL to compile (`Error::MismatchedMethodCells { .. }`/`Error::DuplicateMethodOutputs { .. }` don't exist as struct variants yet).

- [ ] **Step 3: Add the `location` field and wire construction/match sites**

In `adam-rs/src/error.rs`, change both variant definitions:

```rust
    MismatchedMethodCells {
        /// The first method (by index within the `Vec` passed to `add_relationship`) whose
        /// cell set diverges from method 0's, if known.
        location: Option<ErrorLocation>,
    },
```

```rust
    DuplicateMethodOutputs {
        /// The method (by index within the `Vec` passed to `add_relationship`) whose output
        /// set collided, if known.
        location: Option<ErrorLocation>,
    },
```

Update their `Display` arms to ignore the new field:

```rust
            Error::MismatchedMethodCells { .. } => write!(
                f,
                "methods in a relationship must reference the same set of cells"
            ),
```

```rust
            Error::DuplicateMethodOutputs { .. } => write!(
                f,
                "a method's outputs must be duplicate-free, and no two methods in a \
                 relationship may share an outputs set"
            ),
```

Extend `Error::location()`:

```rust
    pub fn location(&self) -> Option<ErrorLocation> {
        match self {
            Error::TypeMismatch { location, .. } => *location,
            Error::MismatchedMethodCells { location } => *location,
            Error::DuplicateMethodOutputs { location } => *location,
            _ => None,
        }
    }
```

Update `error.rs`'s test constructions/matches: `non_method_failed_variants_have_no_source` constructs both bare (`&Error::MismatchedMethodCells`, needs `&Error::MismatchedMethodCells { location: None }`; same for `DuplicateMethodOutputs`); `mismatched_method_cells_display_contains_cells` and `duplicate_method_outputs_display_contains_outputs` construct them bare too — add `{ location: None }` to each.

In `adam-rs/src/sheet.rs`, the `MismatchedMethodCells` check becomes:

```rust
        let cell_sets: Vec<HashSet<CellId>> = methods
            .iter()
            .map(|m| m.inputs.iter().chain(m.outputs.iter()).copied().collect())
            .collect();
        if let Some(rel_idx) = cell_sets[1..].iter().position(|set| set != &cell_sets[0]) {
            return Err(Error::MismatchedMethodCells {
                location: Some(ErrorLocation::MethodIndex(rel_idx + 1)),
            });
        }
```

The `DuplicateMethodOutputs` check becomes:

```rust
        let mut seen_output_sets: Vec<HashSet<CellId>> = Vec::with_capacity(methods.len());
        for (idx, method) in methods.iter().enumerate() {
            let output_set: HashSet<CellId> = method.outputs.iter().copied().collect();
            if output_set.len() != method.outputs.len() || seen_output_sets.contains(&output_set) {
                return Err(Error::DuplicateMethodOutputs {
                    location: Some(ErrorLocation::MethodIndex(idx)),
                });
            }
            seen_output_sets.push(output_set);
        }
```

Update the four remaining bare matches in `sheet.rs` to `{ .. }`: `assert!(matches!(result, Err(Error::MismatchedMethodCells)));` → `Err(Error::MismatchedMethodCells { .. })` (two occurrences, in `add_relationship_inconsistent_cell_sets_across_methods_returns_mismatched_method_cells` — already rewritten in Step 1 above — and nowhere else bare); same for the two remaining bare `Err(Error::DuplicateMethodOutputs)` matches (already rewritten in Step 1's `add_relationship_duplicate_output_set_across_methods_returns_duplicate_method_outputs`/`..._returns_error` — no other bare occurrences remain).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --package adam-rs`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/error.rs adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): attach ErrorLocation to MismatchedMethodCells and DuplicateMethodOutputs"
```

---

### Task 4: `InvalidMethod` and `InvalidCellKind` location

**Files:**
- Modify: `adam-rs/src/error.rs`
- Modify: `adam-rs/src/sheet.rs`

**Interfaces:**
- Produces: `Error::InvalidMethod { location: Option<ErrorLocation> }`, `Error::InvalidCellKind { location: Option<ErrorLocation> }`.

- [ ] **Step 1: Write the failing tests**

In `adam-rs/src/sheet.rs`:

```rust
    #[test]
    fn add_relationship_empty_methods_returns_invalid_method() {
        let mut sheet = Sheet::new();
        let result = sheet.add_relationship(vec![]);
        assert!(matches!(result, Err(Error::InvalidMethod { .. })));
        assert_eq!(result.unwrap_err().location(), None);
    }
```

```rust
    #[test]
    fn add_relationship_empty_outputs_returns_invalid_method() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let method = Method::new(
            vec![a],
            vec![], // no outputs
            vec![TypeId::of::<i32>()],
            vec![],
            |_| Ok(vec![]),
        );
        let result = sheet.add_relationship(vec![method]);
        assert!(matches!(result, Err(Error::InvalidMethod { .. })));
        assert_eq!(
            result.unwrap_err().location(),
            Some(ErrorLocation::MethodIndex(0))
        );
    }
```

```rust
    #[test]
    fn add_relationship_returns_invalid_cell_kind_when_a_source_cell_is_an_output() {
        let mut sheet = Sheet::new();
        let a = sheet.add_source(0_i32);
        let b = sheet.add_cell(0_i32);
        let result = sheet.add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))]);
        assert!(matches!(result, Err(Error::InvalidCellKind { .. })));
        assert_eq!(
            result.unwrap_err().location(),
            Some(ErrorLocation::MethodIndex(0))
        );
    }
```

For the two `add_out`/`write` call sites where `InvalidCellKind` has no method index, just widen the existing bare matches to `{ .. }` (no new assertion needed — `location` is `None` there, already covered structurally by Task 1's default):

- `write_returns_invalid_cell_kind_for_an_output_cell` (line ~2070): `Err(Error::InvalidCellKind)` → `Err(Error::InvalidCellKind { .. })`.
- `add_out_returns_invalid_cell_kind_for_a_write` (line ~3683): same change.
- `add_out_returns_invalid_cell_kind_for_a_second_writer` (line ~3697): same change.
- `error_variant_is_invalid_cell_kind_not_terminal_cell` (line ~2111): `let _err = Error::InvalidCellKind;` → `let _err = Error::InvalidCellKind { location: None };`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --package adam-rs invalid_method invalid_cell_kind`
Expected: FAIL to compile (struct-variant syntax doesn't match the current unit variants).

- [ ] **Step 3: Add the `location` field and wire construction/match sites**

In `adam-rs/src/error.rs`:

```rust
    InvalidMethod {
        /// The method (by index within the `Vec` passed to `add_relationship`) that's
        /// invalid, if known — `None` when `methods` itself is empty.
        location: Option<ErrorLocation>,
    },
```

```rust
    InvalidCellKind {
        /// The method (by index within the `Vec` passed to `add_relationship`) whose output
        /// cell has the wrong kind, if the error originated there.
        location: Option<ErrorLocation>,
    },
```

Update `Display`:

```rust
            Error::InvalidMethod { .. } => write!(f, "method is structurally invalid"),
```

```rust
            Error::InvalidCellKind { .. } => write!(f, "cell's kind does not permit this operation"),
```

Extend `Error::location()`:

```rust
    pub fn location(&self) -> Option<ErrorLocation> {
        match self {
            Error::TypeMismatch { location, .. } => *location,
            Error::MismatchedMethodCells { location } => *location,
            Error::DuplicateMethodOutputs { location } => *location,
            Error::InvalidMethod { location } => *location,
            Error::InvalidCellKind { location } => *location,
            _ => None,
        }
    }
```

Update `error.rs`'s remaining bare test sites: `invalid_method_display_contains_invalid` (`Error::InvalidMethod` → `Error::InvalidMethod { location: None }`), `non_method_failed_variants_have_no_source`'s `&Error::InvalidMethod` (same fix), `invalid_cell_kind_display_contains_kind`/`invalid_cell_kind_display_does_not_mention_terminal`/`invalid_cell_kind_has_no_source` (each `Error::InvalidCellKind` → `Error::InvalidCellKind { location: None }`).

In `adam-rs/src/sheet.rs`, `add_relationship`'s loop (already `.enumerate()`-based from Task 2) gains locations on its `InvalidMethod`/`InvalidCellKind` returns:

```rust
        for (idx, method) in methods.iter().enumerate() {
            if method.outputs.is_empty() {
                return Err(Error::InvalidMethod {
                    location: Some(ErrorLocation::MethodIndex(idx)),
                });
            }

            if method.inputs.len() != method.input_types.len()
                || method.outputs.len() != method.output_types.len()
            {
                return Err(Error::InvalidMethod {
                    location: Some(ErrorLocation::MethodIndex(idx)),
                });
            }

            for (&cell_id, &declared) in method.inputs.iter().zip(method.input_types.iter()) {
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                        location: Some(ErrorLocation::MethodIndex(idx)),
                    });
                }
            }

            for (&cell_id, &declared) in method.outputs.iter().zip(method.output_types.iter()) {
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.kind == CellKind::Source {
                    return Err(Error::InvalidCellKind {
                        location: Some(ErrorLocation::MethodIndex(idx)),
                    });
                }
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                        location: Some(ErrorLocation::MethodIndex(idx)),
                    });
                }
            }
        }
```

`add_relationship`'s empty-`methods` check at the top of the function stays `Error::InvalidMethod { location: None }`:

```rust
        if methods.is_empty() {
            return Err(Error::InvalidMethod { location: None });
        }
```

`add_out` (line 552) and `write` (line 949) both become `Error::InvalidCellKind { location: None }`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --package adam-rs`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/error.rs adam-rs/src/sheet.rs
git commit -m "feat(adam-rs): attach ErrorLocation to InvalidMethod and InvalidCellKind"
```

---

### Task 5: `MethodFailed` location

**Files:**
- Modify: `adam-rs/src/error.rs`
- Modify: `adam-rs/src/sheet.rs`
- Modify: `adam-web-ui/src/labels.rs`

**Interfaces:**
- Produces: `Error::MethodFailed { error: anyhow::Error, location: Option<ErrorLocation> }` (was a tuple variant `MethodFailed(anyhow::Error)`).

- [ ] **Step 1: Write the failing test**

In `adam-rs/src/sheet.rs`, add a new test near `execute_plan_type_mismatch_reports_the_method_that_produced_it` (Task 2):

```rust
    #[test]
    fn execute_plan_method_failed_reports_the_method_that_produced_it() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let method = Method::from_fn_1_1(a, b, |_: &i32| -> Result<i32, anyhow::Error> {
            Err(anyhow::anyhow!("boom"))
        });
        let rel_id = sheet.add_relationship(vec![method]).unwrap();
        let result = sheet.propagate();
        assert!(matches!(result, Err(Error::MethodFailed { .. })));
        assert_eq!(
            result.unwrap_err().location(),
            Some(ErrorLocation::Method(rel_id, 0))
        );
    }
```

Update the two existing bare-tuple matches to the struct form (no new assertions needed — both are `None`-location cases):

- `propagate_surfaces_method_failed_from_a_failing_match_expression` (line ~1988): `Err(Error::MethodFailed(_))` → `Err(Error::MethodFailed { .. })`.
- `add_requirement_propagates_method_failed_when_evaluation_errors` (line ~3609): same change.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --package adam-rs execute_plan_method_failed_reports_the_method_that_produced_it`
Expected: FAIL to compile (`MethodFailed` is still a tuple variant).

- [ ] **Step 3: Convert `MethodFailed` to a struct variant and wire every site**

In `adam-rs/src/error.rs`, change the variant:

```rust
    MethodFailed {
        /// The underlying error the method's function (or a requirement's/conditional's
        /// expression function) returned.
        error: anyhow::Error,
        /// The method this failure originated from, if known.
        location: Option<ErrorLocation>,
    },
```

Update `Display`:

```rust
            Error::MethodFailed { error, .. } => write!(f, "method execution failed: {error}"),
```

Update `std::error::Error::source`:

```rust
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let Error::MethodFailed { error, .. } = self {
            Some(error.as_ref())
        } else {
            None
        }
    }
```

Extend `Error::location()`:

```rust
    pub fn location(&self) -> Option<ErrorLocation> {
        match self {
            Error::TypeMismatch { location, .. } => *location,
            Error::MismatchedMethodCells { location } => *location,
            Error::DuplicateMethodOutputs { location } => *location,
            Error::InvalidMethod { location } => *location,
            Error::InvalidCellKind { location } => *location,
            Error::MethodFailed { location, .. } => *location,
            _ => None,
        }
    }
```

Update `error.rs`'s two test constructions (`method_failed_display_contains_source_message`, `method_failed_source_returns_some`):

```rust
        let err = Error::MethodFailed {
            error: anyhow::anyhow!("division by zero"),
            location: None,
        };
```

(same pattern for the `"inner"` one).

In `adam-rs/src/sheet.rs`, every `.map_err(Error::MethodFailed)` (a bare function-pointer conversion, which no longer type-checks once `MethodFailed` isn't a tuple constructor) becomes a closure:

`add_requirement` (line 503):

```rust
            let holds = (requirement.function)(&inputs)
                .map_err(|error| Error::MethodFailed {
                    error,
                    location: None,
                })?;
```

`evaluate_match_source` (line 1141):

```rust
                let value = (expr.function)(&args).map_err(|error| Error::MethodFailed {
                    error,
                    location: None,
                })?;
```

`propagate`'s requirement-evaluation loop (line 1356):

```rust
            let holds = (requirement.function)(&inputs)
                .map_err(|error| Error::MethodFailed {
                    error,
                    location: None,
                })?;
```

`execute_plan`'s method-call site (line 1483, inside the `PlanStep::Method(rel_id, method_idx) =>` arm):

```rust
                        let outputs = (method.function)(&inputs).map_err(|error| {
                            Error::MethodFailed {
                                error,
                                location: Some(ErrorLocation::Method(rel_id, method_idx)),
                            }
                        })?;
```

`execute_plan`'s output-count check (line 1494):

```rust
                    if outputs.len() != output_ids.len() {
                        return Err(Error::MethodFailed {
                            error: anyhow::anyhow!(
                                "method produced {} outputs but relationship expects {}",
                                outputs.len(),
                                output_ids.len()
                            ),
                            location: Some(ErrorLocation::Method(rel_id, method_idx)),
                        });
                    }
```

In `adam-web-ui/src/labels.rs`, update the two `Error::MethodFailed(anyhow::anyhow!(...))` constructions (`Labels::add_cell`'s `write_str`, line 88, and `Labels::add_tuple_cell`'s `write_str`, line 121) to the struct form with `location: None`:

```rust
                write_str: Box::new(move |sheet, s| {
                    let value = s.parse::<T>().map_err(|e| Error::MethodFailed {
                        error: anyhow::anyhow!("parse error: {}", e),
                        location: None,
                    })?;
                    sheet.write(id, value)
                }),
```

```rust
                write_str: Box::new(|_sheet, _s| {
                    Err(Error::MethodFailed {
                        error: anyhow::anyhow!("editing tuple-typed cells is not yet supported"),
                        location: None,
                    })
                }),
```

`format_adam_error`'s match arm (line 304) becomes (full `ErrorLocation`-aware behavior lands in Task 8 — for this task, just keep it compiling and behaviorally identical):

```rust
pub fn format_adam_error(e: &Error, source: &str, file_name: &str, renderer: &Renderer) -> String {
    match e {
        Error::MethodFailed { error, .. } => error.format_rustc_style(source, file_name, 1, renderer),
        other => other.to_string(),
    }
}
```

Update the two test constructions (`format_adam_error_method_failed_renders_caret_diagnostic`, `format_adam_error_plain_renderer_has_no_ansi_escape_codes`) from `Error::MethodFailed(inner)` to:

```rust
        let err = Error::MethodFailed {
            error: inner,
            location: None,
        };
```

- [ ] **Step 4: Run tests to verify they pass, then lint**

Run: `cargo test --package adam-rs --package adam-web-ui`
Expected: PASS

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Expected: clean (no warnings) — this task touches the most call sites, so check now rather than waiting until Task 8.

- [ ] **Step 5: Commit**

```bash
git add adam-rs/src/error.rs adam-rs/src/sheet.rs adam-web-ui/src/labels.rs
git commit -m "feat(adam-rs): attach ErrorLocation to MethodFailed"
```

---

### Task 6: `adam-lang` — resolve `MethodIndex` immediately and populate `method_spans`

**Files:**
- Modify: `adam-lang/src/parser.rs`

**Interfaces:**
- Consumes: `adam_rs::ErrorLocation` (Task 1), `cel_parser::SourceSpan::from_proc_macro2_range` (existing).
- Produces: `ParsedSheet.method_spans: HashMap<(RelationshipId, usize), cel_parser::SourceSpan>`; `parse_relationship_decl` and `parse_binding`'s new internal signatures (private, no external consumers yet).

- [ ] **Step 1: Write the failing tests**

Add to `adam-lang/src/parser.rs`'s test module (find the existing `#[cfg(test)] mod tests` block; place near the other `relationship`-focused tests, e.g. after `parse_relationship_with_multiple_bindings_lets_the_planner_pick_a_direction`):

```rust
#[test]
fn mismatched_method_cells_error_spans_the_relationship_block_not_the_sheet() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    let source = "sheet s {\n    cell a: i32;\n    cell b: i32;\n\n    relationship {\n        a := b;\n        b := 42;\n    }\n}";
    let err = parser.parse_str(source).unwrap_err();
    // The mismatched binding (`b := 42;`, method index 1) is on line 7; the sheet's
    // opening line (1) must not be reported instead. `ParseError::span()` returns a
    // `proc_macro2::Span`, so `.start()` is a method call here, not a field access (unlike
    // `cel_parser::SourceSpan`, whose `start`/`end` are public `LineColumn` fields).
    assert_eq!(err.span().start().line, 7);
}

#[test]
fn successful_relationship_parse_populates_method_spans() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    let source = "sheet s {\n    cell a: i32;\n    cell b: i32;\n\n    relationship {\n        a := b;\n    }\n}";
    let parsed = parser.parse_str(source).unwrap();
    assert_eq!(parsed.method_spans.len(), 1);
    let ((_, idx), span) = parsed.method_spans.iter().next().unwrap();
    assert_eq!(*idx, 0);
    assert_eq!(span.start.line, 6);
}
```

(`ParsedSheet::method_spans`'s values are `cel_parser::SourceSpan`, whose `start`/`end` are public `LineColumn` fields — `span.start.line` is a plain field access here, unlike `ParseError::span()` below, which returns a `proc_macro2::Span` and needs `.start()` as a method call.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --package adam-lang mismatched_method_cells_error_spans_the_relationship_block_not_the_sheet successful_relationship_parse_populates_method_spans`
Expected: FAIL — today's `parse_relationship_decl` reports `Span::call_site()` (line 1, not line 7), and `ParsedSheet` has no `method_spans` field yet (compile error).

- [ ] **Step 3: Add `method_spans` to `ParseContext`/`ParsedSheet` and rewrite `parse_relationship_decl`/`parse_binding`**

At the top of `adam-lang/src/parser.rs`, add the import:

```rust
use cel_parser::SourceSpan;
use std::collections::HashMap;
```

(`adam_rs::ErrorLocation` joins the existing `use adam_rs::{CellId, MatchExpr, Method, RelationshipId, Requirement, Sheet};` line.)

Add the field to `ParsedSheet` (after `output_names`):

```rust
pub struct ParsedSheet {
    /// The constructed sheet.
    pub sheet: Sheet,
    /// Cell name → `(CellId, TypeShape)`, in declaration order.
    pub cell_names: IndexMap<String, (CellId, TypeShape)>,
    /// Output name → `CellId`, in declaration order — parity with `cell_names`, for callers
    /// that need to look up `Sheet::cell_requirements_valid`/`Sheet::violated_requirements` by
    /// name.
    pub output_names: IndexMap<String, CellId>,
    /// `(RelationshipId, method index)` → the source span of that binding, populated for
    /// every successfully-added relationship. Lets a caller translate an `adam_rs::Error`'s
    /// `ErrorLocation::Method` (raised well after parsing, e.g. from `Sheet::propagate`) back
    /// to a source location.
    pub method_spans: HashMap<(RelationshipId, usize), SourceSpan>,
}
```

Add the matching field to `ParseContext` (after its own `output_names`):

```rust
struct ParseContext {
    cursor: crate::token_cursor::TokenCursor,
    sheet: Sheet,
    /// Maps cell name → (CellId, TypeShape), in declaration order, for method and
    /// conditional compilation and for exposing to callers via `ParsedSheet`.
    cell_names: IndexMap<String, (CellId, TypeShape)>,
    /// Maps output name → `CellId`, in declaration order, for exposing to callers via
    /// `ParsedSheet`.
    output_names: IndexMap<String, CellId>,
    /// Accumulates spans for every successfully-added relationship's methods, for exposing to
    /// callers via `ParsedSheet::method_spans`.
    method_spans: HashMap<(RelationshipId, usize), SourceSpan>,
}
```

Initialize it in `parse_str` (alongside the existing `cell_names: IndexMap::new(), output_names: IndexMap::new(),`):

```rust
            cell_names: IndexMap::new(),
            output_names: IndexMap::new(),
            method_spans: HashMap::new(),
        };
```

and carry it into the returned `ParsedSheet`:

```rust
        Ok(ParsedSheet {
            sheet: ctx.sheet,
            cell_names: ctx.cell_names,
            output_names: ctx.output_names,
            method_spans: ctx.method_spans,
        })
```

Change `parse_binding`'s signature and return the binding's start/end spans:

```rust
    /// `binding = binding_target ":=" expression ";".`
    fn parse_binding(&mut self, ctx: &mut ParseContext) -> Result<(Method, Span, Span)> {
        let start_span = ctx.peek_span();
        let (names, destructure) = parse_binding_target(ctx)?;
        let mut outputs: NamedCells = Vec::with_capacity(names.len());
        for (name, span) in names {
            let (cell_id, shape) = ctx
                .cell_names
                .get(&name)
                .cloned()
                .ok_or_else(|| ParseError::new(format!("undeclared cell `{name}`"), span))?;
            outputs.push((name, cell_id, shape));
        }
        ctx.expect_punct(":=")?;
        let (segment, inputs) = self.parse_deduced_expr(ctx)?;
        let end_span = ctx.expect_punct(";")?;
        let compiled = self.compile_outputs(ctx, &segment, &outputs, destructure)?;
        Ok((
            build_method(inputs, outputs, segment, compiled),
            start_span,
            end_span,
        ))
    }
```

Rewrite `parse_relationship_decl`:

```rust
    fn parse_relationship_decl(&mut self, ctx: &mut ParseContext) -> Result<RelationshipId> {
        let block_start = ctx.peek_span();
        ctx.is_keyword("relationship"); // consume
        ctx.expect_open_brace()?;
        let mut methods = Vec::new();
        let mut spans: Vec<(Span, Span)> = Vec::new();
        while !ctx.at_close_brace() {
            let (method, start, end) = self.parse_binding(ctx)?;
            methods.push(method);
            spans.push((start, end));
        }
        let close_span = ctx.expect_close_brace()?;
        match ctx.sheet.add_relationship(methods) {
            Ok(rel_id) => {
                for (idx, (start, end)) in spans.into_iter().enumerate() {
                    ctx.method_spans.insert(
                        (rel_id, idx),
                        SourceSpan::from_proc_macro2_range(start, end),
                    );
                }
                Ok(rel_id)
            }
            Err(e) => {
                let (start, end) = match e.location() {
                    Some(adam_rs::ErrorLocation::MethodIndex(i)) => {
                        spans.get(i).copied().unwrap_or((block_start, close_span))
                    }
                    _ => (block_start, close_span),
                };
                Err(ParseError::new_range(e.to_string(), start, end))
            }
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --package adam-lang`
Expected: PASS — including the two new tests and the full existing `parser.rs` suite (`parse_relationship_decl`/`parse_binding`'s signature changes are private, so no other file needs updating).

- [ ] **Step 5: Commit**

```bash
git add adam-lang/src/parser.rs
git commit -m "fix(adam-lang): resolve add_relationship errors to the failing binding's span"
```

---

### Task 7: `adam-lang` — stop discarding the span already in scope for `add_conditional`/`add_out`

**Files:**
- Modify: `adam-lang/src/parser.rs`

- [ ] **Step 1: Write the failing tests**

Add near the tests from Task 6:

```rust
#[test]
fn conditional_structural_error_spans_the_conditional_not_the_sheet() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    // The branch relationship shares `mode` (the match cell) and has 2 methods --
    // add_conditional's InvalidConditional ("a branch relationship that shares a cell with
    // the match cell ... has more than one method") fires here, on line 5 (`conditional
    // mode {`), not the sheet's opening line.
    let source = "sheet s {\n    cell mode: i32 = 0;\n    cell other: i32 = 0;\n\n    conditional mode {\n        0i32 => {\n            relationship {\n                mode := other;\n                other := mode;\n            }\n        }\n    }\n}";
    let err = parser.parse_str(source).unwrap_err();
    assert_eq!(err.span().start().line, 5);
}

#[test]
fn out_decl_structural_error_spans_the_out_name_not_the_sheet() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    // Two requirements named `pos` in the same `require` block -- add_out's internal
    // add_requirement call returns InvalidRequirement ("cell already has a same-named
    // requirement") on its second call; the error must point at the out declaration's
    // name (line 3, `out a: i32 := w require {`), not the sheet's opening line.
    let source = "sheet s {\n    cell w: i32 = 1;\n    out a: i32 := w require {\n        pos: a > 0;\n        pos: a > 0;\n    };\n}";
    let err = parser.parse_str(source).unwrap_err();
    assert_eq!(err.span().start().line, 3);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --package adam-lang conditional_structural_error_spans_the_conditional_not_the_sheet out_decl_structural_error_spans_the_out_name_not_the_sheet`
Expected: FAIL — both currently report `Span::call_site()` (line 1), not lines 5/3.

- [ ] **Step 3: Replace `Span::call_site()` with the span already in scope**

In `parse_conditional_decl` (`adam-lang/src/parser.rs`, the `match &match_shape { TypeShape::Named(type_id) => { ... add_cond_fn(...).map_err(...) } TypeShape::Tuple(_) => { ... add_conditional::<...>(...).map_err(...) } }` block, currently at lines 1153–1185):

```rust
                add_cond_fn(&mut ctx.sheet, match_expr, branches, default_rel_ids)
                    .map_err(|e| ParseError::new(e.to_string(), match_span))?;
```

```rust
                ctx.sheet
                    .add_conditional::<cel_runtime::DynamicSequence>(
                        match_expr,
                        typed_branches,
                        default_rel_ids,
                    )
                    .map_err(|e| ParseError::new(e.to_string(), match_span))?;
```

In `parse_out_decl` (`adam-lang/src/parser.rs`, currently line 1319–1322):

```rust
        let out_cell = ctx
            .sheet
            .add_out(writer, named_requirements)
            .map_err(|e| ParseError::new(e.to_string(), name_span))?;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --package adam-lang`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add adam-lang/src/parser.rs
git commit -m "fix(adam-lang): use the already-known span for add_conditional/add_out errors"
```

---

### Task 8: `adam-web-ui` — `format_adam_error` consults `method_spans`

**Files:**
- Modify: `adam-web-ui/src/labels.rs`
- Modify: `adam-web-ui/src/build.rs`
- Modify: `adam-web-ui/src/inspector.rs`

**Interfaces:**
- Consumes: `ParsedSheet::method_spans` (Task 6), `Error::location()` (Tasks 1–5).
- Produces: `format_adam_error(e: &Error, method_spans: &HashMap<(RelationshipId, usize), SourceSpan>, source: &str, file_name: &str, renderer: &Renderer) -> String` (signature change — was 4 params, now 5).

- [ ] **Step 1: Write the failing tests**

In `adam-web-ui/src/labels.rs`'s test module, update the three existing `format_adam_error` calls to pass an empty table as the new second argument:

```rust
    #[test]
    fn format_adam_error_invalid_id_falls_back_to_display() {
        let msg = format_adam_error(
            &Error::InvalidId,
            &HashMap::new(),
            "source text",
            "test.adm2",
            &Renderer::styled(),
        );
        assert_eq!(msg, "invalid cell or relationship id");
    }
```

```rust
    #[test]
    fn format_adam_error_method_failed_renders_caret_diagnostic() {
        use cel_parser::{SourceSpan, SpanContext};

        let source = "1i32 / 0i32";
        let span = SourceSpan::new(1, 0, 1, 11);
        let inner = anyhow::anyhow!("division by zero").context(SpanContext::new(span));
        let err = Error::MethodFailed {
            error: inner,
            location: None,
        };

        let msg = format_adam_error(&err, &HashMap::new(), source, "test.adm2", &Renderer::styled());

        assert!(msg.contains("division by zero"), "{msg}");
        assert!(msg.contains(source), "{msg}");
    }
```

```rust
    #[test]
    fn format_adam_error_plain_renderer_has_no_ansi_escape_codes() {
        use cel_parser::{SourceSpan, SpanContext};

        let source = "1i32 / 0i32";
        let span = SourceSpan::new(1, 0, 1, 11);
        let inner = anyhow::anyhow!("division by zero").context(SpanContext::new(span));
        let err = Error::MethodFailed {
            error: inner,
            location: None,
        };

        let msg = format_adam_error(&err, &HashMap::new(), source, "test.adm2", &Renderer::plain());

        assert!(msg.contains("division by zero"), "{msg}");
        assert!(
            !msg.contains('\u{1b}'),
            "expected no ANSI escapes, got: {msg}"
        );
    }
```

Add a new test for the `ErrorLocation` fallback (no `SpanContext`, but a `method_spans` entry resolves it):

```rust
    #[test]
    fn format_adam_error_method_failed_falls_back_to_method_span_without_a_span_context() {
        use adam_rs::{ErrorLocation, RelationshipId};
        use cel_parser::SourceSpan;

        let source = "sheet s {\n    cell a: i32;\n    relationship {\n        a := oops();\n    }\n}";
        let mut sheet = adam_rs::Sheet::new();
        let rel_id = sheet
            .add_relationship(vec![adam_rs::Method::new(
                vec![],
                vec![sheet.add_cell(0_i32)],
                vec![],
                vec![std::any::TypeId::of::<i32>()],
                |_| Err(anyhow::anyhow!("boom")),
            )])
            .unwrap();
        let mut method_spans = HashMap::new();
        method_spans.insert((rel_id, 0), SourceSpan::new(4, 8, 4, 20));

        let err = Error::MethodFailed {
            error: anyhow::anyhow!("boom"),
            location: Some(ErrorLocation::Method(rel_id, 0)),
        };

        let msg = format_adam_error(&err, &method_spans, source, "test.adm2", &Renderer::plain());

        assert!(msg.contains("boom"), "{msg}");
        assert!(msg.contains("a := oops();"), "{msg}");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --package adam-web-ui format_adam_error`
Expected: FAIL to compile — `format_adam_error` still takes 4 arguments.

- [ ] **Step 3: Update `format_adam_error`'s signature and its two callers**

In `adam-web-ui/src/labels.rs`, add imports (`RelationshipId` joins the existing `use adam_rs::{CellId, Error, Sheet};` line; add `use cel_parser::{SourceSpan, SpanContext};` — `SpanContext` is new, `FormatRustcStyle` stays for the has-a-span branch) and `use std::collections::HashMap;` (may already be present via `IndexMap`'s neighbor import — add it if not).

Replace `format_adam_error` and add the private helper:

```rust
/// Formats an [`Error`] as a rustc-style diagnostic when possible.
///
/// `Error::MethodFailed` wraps an `anyhow::Error` raised by a compiled method
/// body; when that error carries a `SpanContext` (attached automatically by
/// cel-parser's `span-diagnostics` feature for built-in arithmetic ops) this
/// renders a full caret diagnostic against `source` using `renderer` — pass
/// [`Renderer::styled`] for a real terminal (ANSI colors) or [`Renderer::plain`]
/// for a context that can't display them (a browser `<pre>` element, a log file) —
/// with `file_name` (e.g. `"begin/examples/toy_example.adm2"`) shown in the
/// diagnostic header. When it doesn't (a custom method error with no CEL-internal
/// span), `method_spans` — `adam_lang::ParsedSheet::method_spans` — is consulted via
/// the error's `ErrorLocation` as a fallback, underlining the whole failing binding
/// instead of a sub-expression. `Error::TypeMismatch` has no inner CEL error, so it goes
/// straight to the `method_spans` fallback. Every other variant has no source span and
/// falls back to its `Display` message, ignoring `method_spans`/`file_name`/`renderer`.
pub fn format_adam_error(
    e: &Error,
    method_spans: &HashMap<(RelationshipId, usize), SourceSpan>,
    source: &str,
    file_name: &str,
    renderer: &Renderer,
) -> String {
    match e {
        Error::MethodFailed { error, location } => {
            if error.downcast_ref::<SpanContext>().is_some() {
                return error.format_rustc_style(source, file_name, 1, renderer);
            }
            match location_span(*location, method_spans) {
                Some(span) => {
                    SpanContext::new(span).format_rustc_style(&error.to_string(), source, file_name, 1, renderer)
                }
                None => e.to_string(),
            }
        }
        Error::TypeMismatch { location, .. } => match location_span(*location, method_spans) {
            Some(span) => {
                SpanContext::new(span).format_rustc_style(&e.to_string(), source, file_name, 1, renderer)
            }
            None => e.to_string(),
        },
        other => other.to_string(),
    }
}

/// Resolves an `ErrorLocation` to a source span via `method_spans`. `MethodIndex` never
/// appears here in practice — it's only ever produced by `add_relationship` and always
/// resolved immediately by `adam-lang`'s parser (see `AdamParser::parse_relationship_decl`),
/// so it never survives into a post-parse `adam_rs::Error`.
fn location_span(
    location: Option<adam_rs::ErrorLocation>,
    method_spans: &HashMap<(RelationshipId, usize), SourceSpan>,
) -> Option<SourceSpan> {
    match location? {
        adam_rs::ErrorLocation::Method(rel_id, idx) => method_spans.get(&(rel_id, idx)).copied(),
        adam_rs::ErrorLocation::MethodIndex(_) => None,
    }
}
```

In `adam-web-ui/src/build.rs`, update the call site (line 64) — `parsed: ParsedSheet` is still in scope here:

```rust
        Err(e) => {
            let msg = format_adam_error(&e, &parsed.method_spans, source, file_name, renderer);
            BuildOutcome {
                sheet_labels: Some((parsed.sheet, labels)),
                error: Some(msg),
            }
        }
```

In `adam-web-ui/src/inspector.rs`, update the call site (line 279) to pass an empty table — `write_and_propagate` only has a bare `Signal<Sheet>` at this point, not the `ParsedSheet` that built it, so it can't offer a real one yet (see the comment below for why):

```rust
        Err(e) => {
            has_error.set(true);
            // Empty table: `write_and_propagate` only has the built `Sheet`/`Labels`, not the
            // `ParsedSheet` that produced them, so `ErrorLocation::Method` can't be resolved
            // here yet. `Error::MethodFailed`'s existing CEL-internal `SpanContext` path (e.g.
            // division-by-zero) is unaffected by this. Tracked as a follow-up: threading a
            // `method_spans` signal through `begin`'s component tree so a live cell edit gets
            // the same fallback `build_sheet`'s initial parse already does.
            crate::diagnostics::report_error(&format_adam_error(
                &e,
                &std::collections::HashMap::new(),
                &source_text.read(),
                &source_name.read(),
                &Renderer::styled(),
            ));
        }
```

- [ ] **Step 4: Run tests to verify they pass, then lint**

Run: `cargo test --package adam-web-ui`
Expected: PASS

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` and `cargo clippy -p begin --no-default-features --all-targets -- -D warnings` and `cargo clippy -p begin --all-targets -- -D warnings`
Expected: all three clean.

Run: `cargo build --workspace` and `cargo test --workspace`
Expected: zero warnings, all green.

- [ ] **Step 5: File the follow-up issue and commit**

```bash
gh issue create --repo stlab/cel-rs \
  --title "begin: thread method_spans through the live Dioxus UI for write_and_propagate" \
  --body "Follow-up from docs/superpowers/specs/2026-09-08-adam-lang-error-location-design.md. adam-web-ui/src/inspector.rs's write_and_propagate (a live cell edit in the running begin app) currently passes an empty method_spans table to format_adam_error, so a MethodFailed/TypeMismatch raised there without a CEL-internal SpanContext still has no source location -- unlike build.rs's initial parse, which has the real ParsedSheet in scope. Needs a new Signal<HashMap<(RelationshipId, usize), SourceSpan>> threaded through App -> OpenFileControls/load_example/load_opened -> SheetInspector -> CellRow -> write_and_propagate."
```

```bash
git add adam-web-ui/src/labels.rs adam-web-ui/src/build.rs adam-web-ui/src/inspector.rs
git commit -m "feat(adam-web-ui): format_adam_error falls back to ErrorLocation via method_spans"
```

---

### Task 9: Final verification

**Files:** none (verification only)

- [ ] **Step 1: Run the full check suite**

```bash
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace
```

Expected: all pass with zero warnings (per this repo's `CLAUDE.md`: plain `build`/`test` warnings — e.g. an unused `mut` left over from a mechanical edit — aren't caught by clippy alone).

- [ ] **Step 2: Re-read the spec's Section 6 (Testing) and confirm every listed case has a corresponding test**

Cross-check against `docs/superpowers/specs/2026-09-08-adam-lang-error-location-design.md`'s Section 6: `adam-rs`'s `add_relationship_*`/`execute_plan`-driven `location()` assertions (Tasks 2–5), `adam-lang`'s mismatched-cells-block-span and `method_spans`-population tests (Task 6), `adam-web-ui`'s no-`SpanContext`-falls-back-to-`method_spans` test (Task 8). If any is missing, add it now rather than closing out the branch.

- [ ] **Step 3: Commit if Step 1 required any fixes**

```bash
git add -A
git commit -m "chore: fix warnings from full check suite"
```
