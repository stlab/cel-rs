# CEL Closures for adam-lang Filters Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give CEL a first-class closure value (`DynClosure`), a `|params: Type| expr` literal
syntax to build one, and an adam-lang `filter` clause on `cell` declarations that uses one to build
an `adam_rs::Filter` — closing the gap so `cell a: i32 filter |x: i32| clamp(x, 1, 100);` works.

**Architecture:** Three crates, strictly layered, each task buildable and testable on its own:
`cel-runtime` gets a new, self-contained `DynClosure` value type with zero dependency on the other
two tasks; `cel-parser` gets the `|params| expr` grammar (built on `DynClosure` from Task 1) plus a
new built-in scalar/tuple type-name table and an `OpLookup` scope-isolation primitive; `adam-lang`
gets the `filter` clause (built on the `DynClosure` literal from Task 2), reusing the existing,
unmodified `Filter`/`Sheet::add_filter` API. No task changes any public API used by earlier tasks.

**Tech Stack:** Rust, `cel-runtime` (`DynSegment`/`RawStack`), `cel-parser` (`Parser<C>`/`OpLookup`),
`adam-rs` (`Filter`/`Sheet`), `adam-lang` (`AdamParser`/`TypeRegistry`).

**Spec:** `docs/superpowers/specs/2026-08-21-cel-closures-design.md`

## Global Constraints

- Every new/changed function — `pub`, `pub(crate)`, or private — needs a contract-style `///` doc
  comment per the workspace `CLAUDE.md` (Summary; `- Precondition:`/`# Errors`/`# Safety` bullets;
  `- Postcondition:` only when non-obvious; `- Complexity:` whenever not O(1)). CLAUDE.md's rule is
  not limited to `pub` items.
- Preconditions are `debug_assert!`-checked, never `Result`-checked — only genuine runtime error
  conditions (data-dependent, not caller-bug) return `Err`.
- Tests are derived from each function's contract/public interface only — never from reading the
  implementation.
- `cargo fmt --all` before every commit (pre-commit hook enforces this).
- `cargo build --workspace` / `cargo test --workspace` must produce zero compiler warnings; all
  three `cargo clippy` invocations (workspace, `begin` with/without default features) must pass
  with `-D warnings` before this branch is ready for a PR (run once at the end, not per-task).

---

## Task 1: `cel-runtime` — `DynClosure`

**Files:**
- Create: `cel-runtime/src/dyn_closure.rs`
- Modify: `cel-runtime/src/lib.rs` (add `pub mod dyn_closure;` and re-export)
- Test: inline `#[cfg(test)] mod tests` in `dyn_closure.rs`

**Interfaces:**
- Consumes: `cel_runtime::dyn_segment::{DynSegment, StackInfo}` (existing).
- Produces (for Task 2/3):
  - `pub struct DynClosure` — `Clone`, `Debug`.
  - `pub fn DynClosure::new(param_types: Vec<TypeId>, return_type: TypeId, body: DynSegment) -> DynClosure`
  - `pub fn DynClosure::param_types(&self) -> &[TypeId]`
  - `pub fn DynClosure::return_type(&self) -> TypeId`
  - `pub fn DynClosure::call<R: 'static>(&self, args: &[&dyn std::any::Any]) -> anyhow::Result<R>`
  - `pub fn DynClosure::call_boxed(&self, args: &[&dyn std::any::Any], call_dyn_fn: fn(&mut DynSegment, &[&dyn std::any::Any]) -> anyhow::Result<Box<dyn std::any::Any>>) -> anyhow::Result<Box<dyn std::any::Any>>` — for callers (Task 5) that only know the return type dynamically as a `TypeId`, via a monomorphized dispatcher they already have (mirrors `adam-lang`'s existing `TypeRegistry::TypeEntry::call_dyn_fn`, `adam-lang/src/type_registry.rs:69`) rather than a static Rust generic.

- [ ] **Step 1: Write the failing tests**

```rust
// cel-runtime/src/dyn_closure.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dyn_segment::DynSegment;

    fn adder_closure() -> DynClosure {
        let mut body = DynSegment::new::<()>();
        body.push_arg::<i32>(0);
        body.push_arg::<i32>(1);
        body.op2(|a: i32, b: i32| a + b).unwrap();
        DynClosure::new(vec![TypeId::of::<i32>(), TypeId::of::<i32>()], TypeId::of::<i32>(), body)
    }

    #[test]
    fn call_invokes_body_with_positional_args() {
        let closure = adder_closure();
        let (a, b) = (2i32, 3i32);
        let result: i32 = closure.call(&[&a, &b]).unwrap();
        assert_eq!(result, 5);
    }

    #[test]
    fn call_is_repeatable_with_different_args() {
        let closure = adder_closure();
        let (a1, b1) = (2i32, 3i32);
        assert_eq!(closure.call::<i32>(&[&a1, &b1]).unwrap(), 5);
        let (a2, b2) = (10i32, 20i32);
        assert_eq!(closure.call::<i32>(&[&a2, &b2]).unwrap(), 30);
    }

    #[test]
    fn clone_shares_the_same_body_and_both_remain_callable() {
        let closure = adder_closure();
        let cloned = closure.clone();
        let (a, b) = (1i32, 1i32);
        assert_eq!(closure.call::<i32>(&[&a, &b]).unwrap(), 2);
        assert_eq!(cloned.call::<i32>(&[&a, &b]).unwrap(), 2);
    }

    #[test]
    fn call_boxed_dispatches_through_a_supplied_call_dyn_fn() {
        let closure = adder_closure();
        fn call_dyn_fn(seg: &mut DynSegment, inputs: &[&dyn Any]) -> anyhow::Result<Box<dyn Any>> {
            let v: i32 = seg.call_dyn(inputs)?;
            Ok(Box::new(v))
        }
        let (a, b) = (4i32, 5i32);
        let result = closure.call_boxed(&[&a, &b], call_dyn_fn).unwrap();
        assert_eq!(*result.downcast_ref::<i32>().unwrap(), 9);
    }

    #[test]
    fn param_types_and_return_type_are_queryable() {
        let closure = adder_closure();
        assert_eq!(closure.param_types(), &[TypeId::of::<i32>(), TypeId::of::<i32>()]);
        assert_eq!(closure.return_type(), TypeId::of::<i32>());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail (compile error — `DynClosure` doesn't exist yet)**

Run: `cargo test -p cel-runtime dyn_closure`
Expected: FAIL to compile — `cannot find type DynClosure` / `cannot find function new`.

- [ ] **Step 3: Write the implementation**

```rust
//! A first-class, callable CEL value: a compiled body plus its declared signature.
//!
//! See `docs/superpowers/specs/2026-08-21-cel-closures-design.md` for the full design rationale
//! (why `Rc`, why `RefCell`, why no captured environment).

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::rc::Rc;

use crate::dyn_segment::DynSegment;

struct ClosureData {
    param_types: Vec<TypeId>,
    return_type: TypeId,
    body: RefCell<DynSegment>,
}

impl std::fmt::Debug for ClosureData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClosureData")
            .field("param_types", &self.param_types)
            .field("return_type", &self.return_type)
            .finish_non_exhaustive()
    }
}

/// A first-class, callable CEL value: a compiled body plus its declared parameter/return types.
///
/// Holds no captured environment — only its own parameters resolve inside `body`. Calling it
/// twice with different `args` is exactly as fresh each time as calling any other `DynSegment`.
/// `Rc`-wrapped so `Clone` never requires `DynSegment` itself to implement `Clone` (it doesn't —
/// see the design spec); `RefCell` because callers only ever reach a `DynClosure` through `&self`
/// (matching `adam_rs::Filter`/`Method`'s `Fn`, not `FnMut`, storage), while `DynSegment`'s own
/// call methods need `&mut self`.
#[derive(Clone, Debug)]
pub struct DynClosure(Rc<ClosureData>);

impl DynClosure {
    /// Wraps `body` as a closure value declaring `param_types` (in order) and `return_type`.
    ///
    /// - Precondition: `body` was compiled expecting exactly `param_types.len()` positional
    ///   arguments (via `push_arg`/`push_arg_as_dynamic_sequence_tuple`), in that order, and
    ///   produces exactly one result of `return_type`.
    #[must_use]
    pub fn new(param_types: Vec<TypeId>, return_type: TypeId, body: DynSegment) -> Self {
        DynClosure(Rc::new(ClosureData {
            param_types,
            return_type,
            body: RefCell::new(body),
        }))
    }

    /// Returns this closure's declared parameter types, in order.
    #[must_use]
    pub fn param_types(&self) -> &[TypeId] {
        &self.0.param_types
    }

    /// Returns this closure's declared return type.
    #[must_use]
    pub fn return_type(&self) -> TypeId {
        self.0.return_type
    }

    /// Invokes the closure's body with `args`, positionally matched against `param_types`.
    ///
    /// - Precondition: `args.len() == self.param_types().len()` and each `args[i]`'s runtime type
    ///   matches `param_types()[i]` — the caller (adam-lang's generated `Filter` wrapper, or any
    ///   future caller) must guarantee this ahead of time; a violation is a caller bug, not user
    ///   error (matches `DynSegment::push_arg`'s own existing precondition, which this delegates
    ///   to unchanged).
    /// - Precondition: `TypeId::of::<R>() == self.return_type()`.
    /// - Complexity: whatever the body's own evaluation complexity is.
    pub fn call<R: 'static>(&self, args: &[&dyn Any]) -> anyhow::Result<R> {
        debug_assert_eq!(args.len(), self.0.param_types.len());
        debug_assert_eq!(TypeId::of::<R>(), self.0.return_type);
        self.0.body.borrow_mut().call_dyn::<R>(args)
    }

    /// Invokes the closure's body with `args`, like [`call`](Self::call), for a caller that only
    /// knows the return type dynamically (as a `TypeId`) rather than as a static Rust generic —
    /// `call_dyn_fn` is a monomorphized dispatcher the caller already has for that type (e.g.
    /// `adam-lang`'s per-type `TypeRegistry::TypeEntry::call_dyn_fn`).
    ///
    /// - Precondition: `args.len() == self.param_types().len()` and each `args[i]`'s runtime type
    ///   matches `param_types()[i]`, exactly as for [`call`](Self::call).
    /// - Precondition: `call_dyn_fn` is a dispatcher for `self.return_type()` (i.e. it calls
    ///   `DynSegment::call_dyn::<R>` for the same concrete `R` that `TypeId` names).
    /// - Complexity: whatever the body's own evaluation complexity is.
    pub fn call_boxed(
        &self,
        args: &[&dyn Any],
        call_dyn_fn: fn(&mut DynSegment, &[&dyn Any]) -> anyhow::Result<Box<dyn Any>>,
    ) -> anyhow::Result<Box<dyn Any>> {
        debug_assert_eq!(args.len(), self.0.param_types.len());
        call_dyn_fn(&mut self.0.body.borrow_mut(), args)
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-runtime dyn_closure`
Expected: PASS (5 tests).

- [ ] **Step 5: Wire the module into the crate root**

In `cel-runtime/src/lib.rs`, alongside the existing `pub mod dyn_segment;`-style declarations, add:

```rust
pub mod dyn_closure;
pub use dyn_closure::DynClosure;
```

- [ ] **Step 6: Run the full `cel-runtime` test suite**

Run: `cargo test -p cel-runtime`
Expected: PASS, no new warnings.

- [ ] **Step 7: Commit**

```bash
git add cel-runtime/src/dyn_closure.rs cel-runtime/src/lib.rs
git commit -m "feat(cel-runtime): add DynClosure, a first-class callable CEL value"
```

---

## Task 2: `cel-parser` — built-in scalar/tuple type-name table + `OpLookup` scope isolation

**Files:**
- Modify: `cel-parser/src/op_table.rs`
- Test: inline `#[cfg(test)] mod tests` additions in `op_table.rs`

**Interfaces:**
- Consumes: `cel_runtime::{AssociatedType, DynSegment, DynTuple, RawDropper, raw_dropper_for, drop_tuple}` (all existing, all `pub`).
- Produces (for Task 3):
  - `pub(crate) struct BuiltinScalarType { pub type_id: TypeId, pub type_name: &'static str, pub size: usize, pub align: usize, pub dropper: RawDropper, pub push_arg: fn(&mut DynSegment, usize) }`
  - `pub(crate) fn builtin_scalar_type(name: &str) -> Option<BuiltinScalarType>`
  - `impl OpLookup { pub fn push_library_scope<F>(&mut self, scope: F) where F: Fn(&str, &mut DynSegment, usize, SourceSpan) -> Result<bool> + Send + Sync + 'static; pub fn isolate_scopes(&mut self) -> Vec<ScopeFn>; pub fn restore_scopes(&mut self, scopes: Vec<ScopeFn>); }` — `isolate_scopes` only removes scopes pushed via the ordinary, transient `push_scope`; anything pushed via `push_library_scope` (including `OpLookup::new()`'s own `round_scope`, changed to use it) survives isolation.

- [ ] **Step 1: Write the failing tests**

```rust
// in cel-parser/src/op_table.rs, inside the existing #[cfg(test)] mod tests block
#[test]
fn builtin_scalar_type_resolves_every_documented_name() {
    for name in ["u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128",
                 "isize", "f32", "f64", "bool", "String"] {
        let scalar = builtin_scalar_type(name)
            .unwrap_or_else(|| panic!("expected `{name}` to resolve"));
        assert_eq!(scalar.type_name, name);
    }
    assert!(builtin_scalar_type("not_a_type").is_none());
}

#[test]
fn builtin_scalar_type_i32_matches_std_any_type_id() {
    let scalar = builtin_scalar_type("i32").unwrap();
    assert_eq!(scalar.type_id, TypeId::of::<i32>());
    assert_eq!(scalar.size, std::mem::size_of::<i32>());
    assert_eq!(scalar.align, std::mem::align_of::<i32>());
}

#[test]
fn builtin_scalar_type_push_arg_declares_a_readable_argument() {
    let scalar = builtin_scalar_type("i32").unwrap();
    let mut segment = DynSegment::new::<()>();
    (scalar.push_arg)(&mut segment, 0);
    let value = 42i32;
    let result: i32 = segment.call_dyn(&[&value]).unwrap();
    assert_eq!(result, 42);
}

#[test]
fn isolate_scopes_removes_pushed_scopes_until_restored() {
    let mut lookup = OpLookup::new();
    lookup.push_scope(|name, segment, arity, _span| {
        if name == "custom" && arity == 0 {
            segment.just(1i32);
            Ok(true)
        } else {
            Ok(false)
        }
    });

    let mut segment = DynSegment::new::<()>();
    let isolated = lookup.isolate_scopes();
    let err = lookup.lookup(
        "custom",
        &mut segment,
        0,
        proc_macro2::Span::call_site(),
        proc_macro2::Span::call_site(),
    );
    assert!(err.is_err(), "custom scope must not be reachable while isolated");

    lookup.restore_scopes(isolated);
    let mut segment = DynSegment::new::<()>();
    lookup
        .lookup(
            "custom",
            &mut segment,
            0,
            proc_macro2::Span::call_site(),
            proc_macro2::Span::call_site(),
        )
        .unwrap();
    assert_eq!(segment.call0::<i32>().unwrap(), 1);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cel-parser op_table::tests`
Expected: FAIL to compile — `builtin_scalar_type`/`isolate_scopes`/`restore_scopes` don't exist.

- [ ] **Step 3: Write the implementation**

Add near `signatures_for_cast` in `cel-parser/src/op_table.rs` (same file, same style — a
mechanical per-type match, mirroring that function's exact shape):

```rust
/// One built-in scalar type's identity plus everything needed to declare a `DynSegment`
/// argument or tuple leaf of that type without the caller knowing it as a static Rust generic.
///
/// Covers exactly the fixed set of scalar type names [`signatures_for_cast`] already recognizes
/// as `as`-cast targets — closures are the first feature needing to *declare* a value of a named
/// type (rather than convert an already-stack-resident one), so this is new, additive surface
/// area; it deliberately reuses that same closed name set rather than inventing a second one.
pub(crate) struct BuiltinScalarType {
    pub(crate) type_id: TypeId,
    pub(crate) type_name: &'static str,
    pub(crate) size: usize,
    pub(crate) align: usize,
    pub(crate) dropper: cel_runtime::RawDropper,
    pub(crate) push_arg: fn(&mut DynSegment, usize),
}

macro_rules! builtin_scalar {
    ($name:literal, $ty:ty) => {
        BuiltinScalarType {
            type_id: TypeId::of::<$ty>(),
            type_name: $name,
            size: std::mem::size_of::<$ty>(),
            align: std::mem::align_of::<$ty>(),
            dropper: cel_runtime::raw_dropper_for::<$ty>(),
            push_arg: |seg, idx| seg.push_arg::<$ty>(idx),
        }
    };
}

/// Resolves a closure parameter type annotation's bare identifier to its full built-in
/// descriptor, or `None` if `name` names no recognized scalar type.
///
/// - Complexity: O(1).
pub(crate) fn builtin_scalar_type(name: &str) -> Option<BuiltinScalarType> {
    Some(match name {
        "u8" => builtin_scalar!("u8", u8),
        "u16" => builtin_scalar!("u16", u16),
        "u32" => builtin_scalar!("u32", u32),
        "u64" => builtin_scalar!("u64", u64),
        "u128" => builtin_scalar!("u128", u128),
        "usize" => builtin_scalar!("usize", usize),
        "i8" => builtin_scalar!("i8", i8),
        "i16" => builtin_scalar!("i16", i16),
        "i32" => builtin_scalar!("i32", i32),
        "i64" => builtin_scalar!("i64", i64),
        "i128" => builtin_scalar!("i128", i128),
        "isize" => builtin_scalar!("isize", isize),
        "f32" => builtin_scalar!("f32", f32),
        "f64" => builtin_scalar!("f64", f64),
        "bool" => builtin_scalar!("bool", bool),
        "String" => builtin_scalar!("String", String),
        _ => return None,
    })
}
```

**Design correction (found during this task's own review — see the plan-level ledger/spec for
the full ruling):** a blanket "remove everything" `isolate_scopes` is wrong. `round_scope`
(registered in `OpLookup::new()`, below) and, once a caller wires `cel-std` in, `clamp`/`min`/
`max`/etc. are *also* registered via plain `push_scope` — not via the separate `builtin_scope`
field — so a blanket clear would make them unreachable from inside every closure body too, which
is backwards (library functions should always be reachable, including inside a closure; only
*transient*, per-declaration/per-closure scopes must be isolated). `OpLookup` gains a
`library_scope_count: usize` field and a `push_library_scope` method that pushes a scope *and*
advances that floor; `isolate_scopes`/`restore_scopes` operate only on scopes *above* the floor.

Add the new field to `OpLookup`'s struct definition and initialize it in `new()`:

```rust
pub struct OpLookup {
    scopes: Vec<ScopeFn>,
    /// How many of `scopes`'s bottom entries are permanent "library" scopes (registered once at
    /// setup time via `push_library_scope`, e.g. this module's own `round_scope`) rather than
    /// transient ones pushed/popped around a single parse — see `isolate_scopes`.
    library_scope_count: usize,
    builtin_scope: BuiltinScope,
    tuple_signatures: Vec<TupleOpSignature>,
}
```

In `OpLookup::new()`, change the existing `lookup.push_scope(round_scope);` to
`lookup.push_library_scope(round_scope);` (and initialize `library_scope_count: 0` in the struct
literal alongside the other fields — `push_library_scope` sets it correctly on that first call).

Add to `impl OpLookup` (near `push_scope`/`pop_scope`):

```rust
/// Pushes a permanent "library" scope — always reachable, including from inside an isolated
/// (closure-body) scope stack — as opposed to [`push_scope`](Self::push_scope)'s transient kind.
///
/// Intended for setup-time registration only (this module's own `round_scope`; a future
/// `cel-std`-style crate's own functions). Never call this for a scope tied to one parse's
/// lifetime — use [`push_scope`](Self::push_scope) for that.
///
/// - Postcondition: the pushed scope is included in every future [`lookup`](Self::lookup) call,
///   even while isolated via [`isolate_scopes`](Self::isolate_scopes).
pub fn push_library_scope<F>(&mut self, scope: F)
where
    F: Fn(&str, &mut DynSegment, usize, SourceSpan) -> Result<bool> + Send + Sync + 'static,
{
    self.scopes.push(Box::new(scope));
    self.library_scope_count = self.scopes.len();
}

/// Temporarily removes every *transient* scope pushed via [`push_scope`](Self::push_scope),
/// returning them so a later [`restore_scopes`](Self::restore_scopes) call can put them back.
/// Scopes pushed via [`push_library_scope`](Self::push_library_scope) are never removed.
///
/// Used when compiling an independent nested body (a closure literal) that must resolve names
/// against only its own declared parameters, library functions, and built-ins — never whatever
/// enclosing *transient* scope happens to be active (e.g. adam-lang's own per-declaration
/// cell-name scope), which the LIFO `scopes` stack would otherwise still make reachable to a
/// scope pushed on top of it.
///
/// - Postcondition: only scopes at or below `library_scope_count` remain reachable until
///   [`restore_scopes`](Self::restore_scopes) is called.
pub fn isolate_scopes(&mut self) -> Vec<ScopeFn> {
    self.scopes.split_off(self.library_scope_count)
}

/// Restores a scope stack previously removed by [`isolate_scopes`](Self::isolate_scopes),
/// discarding whatever transient scopes were pushed while isolated.
///
/// - Precondition: `scopes` came from a matching `isolate_scopes()` call on this same
///   `OpLookup` — restoring an arbitrary `Vec<ScopeFn>` is well-typed but not a meaningful use
///   of this method.
pub fn restore_scopes(&mut self, scopes: Vec<ScopeFn>) {
    self.scopes.truncate(self.library_scope_count);
    self.scopes.extend(scopes);
}
```

Also add a defensive precondition check to the existing `pop_scope` (this task touches it because
it now shares the file's new invariant — a library scope must never be popped via the transient
path):

```rust
pub fn pop_scope(&mut self) -> Option<ScopeFn> {
    debug_assert!(
        self.scopes.len() > self.library_scope_count,
        "pop_scope must not remove a library scope — use isolate_scopes/restore_scopes semantics instead"
    );
    self.scopes.pop()
}
```

(This changes an existing method's body, not just adds new ones — `pop_scope`'s own doc comment
doesn't need to change, just this one line added inside it.)

Add one more test (alongside `isolate_scopes_removes_pushed_scopes_until_restored`) proving the
actual bug this design correction fixes — that a library scope survives isolation:

```rust
#[test]
fn isolate_scopes_leaves_library_scopes_reachable() {
    // round_scope's own protocol is two lookups: ("round", 0) pushes a marker value, then
    // ("()", 2) (with the marker plus an f64 operand on the stack) computes the actual round.
    // This test only needs to prove the *first* half is still reachable while isolated -- that's
    // enough to demonstrate round_scope (a library scope) survived isolate_scopes, without
    // needing to replicate the whole call protocol.
    let mut lookup = OpLookup::new(); // registers round_scope via push_library_scope
    let mut segment = DynSegment::new::<()>();
    let isolated = lookup.isolate_scopes();
    lookup
        .lookup(
            "round",
            &mut segment,
            0,
            proc_macro2::Span::call_site(),
            proc_macro2::Span::call_site(),
        )
        .expect("round is a library scope and must survive isolation");
    lookup.restore_scopes(isolated);
    assert_eq!(segment.peek_stack_infos(1).len(), 1); // the RoundFn marker was pushed
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser op_table::tests`
Expected: PASS (5 new tests, plus all existing `op_table.rs` tests still pass — including
`OpLookup::new()`'s change from `push_scope(round_scope)` to `push_library_scope(round_scope)`,
which must not change `round`'s ordinary (non-isolated) behavior at all).

- [ ] **Step 5: Update `cel-std` to register its functions as library scopes too**

`cel-std/src/lib.rs`'s `install` currently registers every one of its functions via plain
`push_scope`, which means they'd suffer the exact same bug `round_scope` had — unreachable from
inside an isolated (closure-body) scope stack once a caller wires `cel-std` in (tracked
separately, in [stlab/cel-rs#137](https://github.com/stlab/cel-rs/issues/137), for *whether*
`adam-lang` ever calls `install` at all — this step fixes the mechanism regardless). Change all
four calls from `push_scope` to `push_library_scope`:

```rust
// cel-std/src/lib.rs
pub fn install(lookup: &mut cel_parser::OpLookup) {
    lookup.push_library_scope(math::min_max_scope);
    lookup.push_library_scope(math::clamp_scope);
    lookup.push_library_scope(math::abs_scope);
    lookup.push_library_scope(math::unary_math_scope);
}
```

Run `cargo test -p cel-std` to confirm nothing else in that crate broke (it shouldn't — this is a
pure rename of which `OpLookup` method each call uses, with identical scope-resolution behavior
outside of isolation).

- [ ] **Step 6: Commit**

```bash
git add cel-parser/src/op_table.rs cel-std/src/lib.rs
git commit -m "feat(cel-parser): built-in scalar type table + OpLookup scope isolation

Distinguishes library scopes (round, and now cel-std's functions) from
transient per-parse scopes, so isolate_scopes only removes the latter."
```

---

## Task 3: `cel-parser` — nested independent context compilation + `ParserContext::push_closure`

**Files:**
- Modify: `cel-parser/src/parser_context.rs`
- Modify: `cel-parser/src/lib.rs`
- Test: inline `#[cfg(test)]` additions in both files

**Interfaces:**
- Consumes: `ParserContext` trait (existing), `DynClosure` (Task 1), `BuiltinScalarType`/`isolate_scopes`/`restore_scopes` (Task 2).
- Produces (for Task 4):
  - `ParserContext::push_closure(&mut self, param_types: Vec<TypeId>, return_type: TypeId, body: Self, span: Span) -> crate::Result<()>` (new trait method, default body returns `Err`).
  - `impl<C: ParserContext> Parser<C> { pub(crate) fn parse_nested_context<F>(&mut self, f: F) -> Result<C> where F: FnOnce(&mut Self) -> Result<bool> }` — swaps `self.context` for a fresh one, runs `f`, swaps back, returns the finished nested context.

- [ ] **Step 1: Write the failing test for `push_closure`'s default/override behavior**

```rust
// cel-parser/src/parser_context.rs, in the existing #[cfg(test)] mod tests
#[test]
fn dyn_segment_context_push_closure_builds_a_callable_closure() {
    let mut outer = DynSegmentContext::new_context();
    let mut body = DynSegmentContext::new_context();
    body.0.push_arg::<i32>(0);
    body.0.op1(|x: i32| x + 1).unwrap();

    outer
        .push_closure(
            vec![TypeId::of::<i32>()],
            TypeId::of::<i32>(),
            body,
            Span::call_site(),
        )
        .unwrap();

    let closure: cel_runtime::DynClosure = outer.into_inner().call0().unwrap();
    let x = 5i32;
    assert_eq!(closure.call::<i32>(&[&x]).unwrap(), 6);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p cel-parser parser_context::tests::dyn_segment_context_push_closure`
Expected: FAIL to compile — `push_closure` doesn't exist.

- [ ] **Step 3: Add `push_closure` to `ParserContext` and `DynSegmentContext`**

In the `ParserContext` trait (`cel-parser/src/parser_context.rs`), add (after `apply_cast`):

```rust
/// Packages a fully-parsed, independent nested context — the body of a closure literal — as a
/// value pushed onto `self`, given the closure's declared parameter/return types.
///
/// The default implementation reports closures as unsupported, so a `ParserContext`
/// implementation that has no use for them (e.g. an AST-building context for the formatter or
/// language server) needs no changes to keep compiling.
///
/// - Precondition: `body` was built via `Self::new_context()` and its own argument-binding
///   mechanism, in the same style [`Self::new_context`]'s other consumers already use.
///
/// # Errors
///
/// Returns `Err` if this `ParserContext` implementation doesn't support closures.
fn push_closure(
    &mut self,
    param_types: Vec<std::any::TypeId>,
    return_type: std::any::TypeId,
    body: Self,
    span: Span,
) -> crate::Result<()> {
    let _ = (param_types, return_type, body);
    Err(crate::ParseError::new_range(
        "closures are not supported in this context".to_string(),
        span,
        span,
    ))
}
```

In `impl ParserContext for DynSegmentContext` (after `apply_cast`):

```rust
fn push_closure(
    &mut self,
    param_types: Vec<std::any::TypeId>,
    return_type: std::any::TypeId,
    body: Self,
    span: Span,
) -> crate::Result<()> {
    self.0
        .just(cel_runtime::DynClosure::new(param_types, return_type, body.into_inner()));
    let _ = span;
    Ok(())
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p cel-parser parser_context::tests::dyn_segment_context_push_closure`
Expected: PASS.

- [ ] **Step 5: Write the failing test for `parse_nested_context`**

```rust
// cel-parser/src/lib.rs, in the existing #[cfg(test)] mod tests
#[test]
fn parse_nested_context_compiles_an_independent_segment_without_disturbing_the_outer_one() -> anyhow::Result<()> {
    let mut parser = CELParser::new(OpLookup::new());
    parser.set_tokens(quote::quote! { 1 + 2 }.into_iter());
    // Start the outer expression: push a literal directly onto the (not-yet-swapped) context.
    parser.context.push_literal(100i32, proc_macro2::Span::call_site());

    let nested = parser.parse_nested_context(|p| p.is_or_expression())?;

    // The outer context still only has the one literal pushed before the nested parse.
    assert_eq!(parser.context.into_inner().call0::<i32>()?, 100);
    // The nested context has its own, independently-evaluated result.
    assert_eq!(nested.into_inner().call0::<i32>()?, 3);
    Ok(())
}
```

- [ ] **Step 6: Run the test to verify it fails**

Run: `cargo test -p cel-parser lib::tests::parse_nested_context`
Expected: FAIL to compile — `parse_nested_context` doesn't exist.

- [ ] **Step 7: Implement `parse_nested_context` on `Parser<C>`**

In `cel-parser/src/lib.rs`, in `impl<C: ParserContext> Parser<C>` (near `parse_or_expression_ctx`):

```rust
/// Compiles a fully independent nested context — used for a closure literal's body — by
/// swapping `self.context` out for a fresh one, running `f` against it, then swapping the
/// original context back in and returning the finished nested one.
///
/// Unlike [`parse_or_expression_ctx`](Self::parse_or_expression_ctx), this does not reset
/// `self.tokens`/`self.op_lookup`/`self.last_span` — it's for compiling a sub-expression in the
/// middle of an already-in-progress outer parse, not starting a fresh top-level parse.
///
/// # Errors
///
/// Returns `Err` if `f` does, or if `f` returns `Ok(false)` (no expression found) — in both
/// cases the outer context is still restored before returning.
///
/// - Complexity: whatever `f`'s own parse cost is.
pub(crate) fn parse_nested_context<F>(&mut self, f: F) -> Result<C>
where
    F: FnOnce(&mut Self) -> Result<bool>,
{
    let outer = std::mem::replace(&mut self.context, C::new_context());
    let outcome = f(self);
    let nested = std::mem::replace(&mut self.context, outer);
    match outcome {
        Ok(true) => Ok(nested),
        Ok(false) => Err(self.error_at("expected expression")),
        Err(e) => Err(e),
    }
}
```

- [ ] **Step 8: Run the test to verify it passes**

Run: `cargo test -p cel-parser lib::tests::parse_nested_context`
Expected: PASS.

- [ ] **Step 9: Run the full `cel-parser` test suite**

Run: `cargo test -p cel-parser`
Expected: PASS, no new warnings.

- [ ] **Step 10: Commit**

```bash
git add cel-parser/src/parser_context.rs cel-parser/src/lib.rs
git commit -m "feat(cel-parser): nested context compilation + ParserContext::push_closure"
```

---

## Task 4: `cel-parser` — `|params: Type| expr` grammar

**Files:**
- Modify: `cel-parser/src/lib.rs`
- Test: inline `#[cfg(test)]` additions in `lib.rs`

**Interfaces:**
- Consumes: Task 1 (`DynClosure`), Task 2 (`builtin_scalar_type`, `isolate_scopes`/`restore_scopes`), Task 3 (`parse_nested_context`, `push_closure`).
- Produces (for Task 5 / adam-lang): a working `|x: i32| expr` primary expression, usable through
  `CELParser::parse_or_expression`/`parse_or_expression_ctx` exactly like any other expression —
  no new public API beyond the grammar itself (adam-lang drives it the same way it already drives
  every other CEL expression, via `parse_cel_or_expression`).

- [ ] **Step 1: Write the failing tests**

```rust
// cel-parser/src/lib.rs, in the existing #[cfg(test)] mod tests
#[test]
fn closure_literal_with_one_param_compiles_and_calls() -> anyhow::Result<()> {
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser.parse_str("|x: i32| x + 1")?;
    let closure: cel_runtime::DynClosure = segment.call0()?;
    let x = 5i32;
    assert_eq!(closure.call::<i32>(&[&x])?, 6);
    Ok(())
}

#[test]
fn closure_literal_with_zero_params_compiles_and_calls() -> anyhow::Result<()> {
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser.parse_str("|| 42")?;
    let closure: cel_runtime::DynClosure = segment.call0()?;
    assert_eq!(closure.call::<i32>(&[])?, 42);
    Ok(())
}

#[test]
fn closure_literal_with_two_params_compiles_and_calls_in_order() -> anyhow::Result<()> {
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser.parse_str("|a: i32, b: i32| a - b")?;
    let closure: cel_runtime::DynClosure = segment.call0()?;
    let (a, b) = (10i32, 3i32);
    assert_eq!(closure.call::<i32>(&[&a, &b])?, 7);
    Ok(())
}

#[test]
fn closure_literal_with_tuple_typed_param_compiles_and_calls() -> anyhow::Result<()> {
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser.parse_str("|r: (i32, i32)| r.0 + r.1")?;
    let closure: cel_runtime::DynClosure = segment.call0()?;
    let mut pair = DynSegment::new::<()>();
    pair.just(10i32);
    pair.just(20i32);
    pair.make_tuple(2, 0);
    let pair: cel_runtime::DynamicSequence = pair.extract_tuple_as_dynamic_sequence()?;
    assert_eq!(closure.call::<i32>(&[&pair])?, 30);
    Ok(())
}

#[test]
fn closure_body_referencing_an_undeclared_name_is_a_parse_error() {
    let mut parser = CELParser::new(OpLookup::new());
    let err = parser.parse_str("|x: i32| x + y");
    assert!(err.is_err());
}

#[test]
fn nested_closure_referencing_only_its_own_param_compiles_and_calls() -> anyhow::Result<()> {
    let mut parser = CELParser::new(OpLookup::new());
    // The outer closure's own parameter `x` must NOT be visible inside the inner closure body.
    let mut segment = parser.parse_str("|x: i32| { |y: i32| y + 1 }")?;
    let outer: cel_runtime::DynClosure = segment.call0()?;
    let x = 0i32;
    let inner: cel_runtime::DynClosure = outer.call(&[&x])?;
    let y = 41i32;
    assert_eq!(inner.call::<i32>(&[&y])?, 42);
    Ok(())
}

#[test]
fn closure_body_cannot_see_an_enclosing_scopes_names() {
    // A scope pushed before parsing (standing in for e.g. adam-lang's own cell-name scope) must
    // not leak into a closure body's name resolution.
    let mut lookup = OpLookup::new();
    lookup.push_scope(|name, segment, arity, _span| {
        if name == "outer_only" && arity == 0 {
            segment.just(1i32);
            Ok(true)
        } else {
            Ok(false)
        }
    });
    let mut parser = CELParser::new(lookup);
    let err = parser.parse_str("|x: i32| x + outer_only");
    assert!(err.is_err());
}
```

Note: `nested_closure_referencing_only_its_own_param_compiles_and_calls` requires block-expression
`{ expr }` grouping around the inner closure literal only if the grammar doesn't already accept a
bare closure literal as the body of an outer closure directly — check the actual grammar landed in
Step 3 below and simplify this test to `parser.parse_str("|x: i32| |y: i32| y + 1")?` if a bare
nested closure literal parses fine as `expression` with no extra grouping needed (expected, since
`closure_expression` is itself just another primary expression).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cel-parser lib::tests::closure`
Expected: FAIL — `|` is not currently accepted as a primary expression (parse errors on all of
these, including the ones expected to succeed).

- [ ] **Step 3: Implement the grammar**

In `cel-parser/src/lib.rs`, extend `is_primary_expression`'s match to recognize `|` (a new arm,
checked before the `Literal`/`Identifier`/open-paren arms since it's a distinct token shape):

```rust
fn is_primary_expression(&mut self) -> Result<bool> {
    if self.is_punctuation("|") {
        return self.is_closure_expression();
    }
    match self.peek_token() {
        // ...existing arms unchanged...
    }
}
```

Add the new grammar productions (near `is_tuple_or_group`):

```rust
/// `closure_expression = "|" [ closure_param { "," closure_param } ] "|" expression .`
fn is_closure_expression(&mut self) -> Result<bool> {
    let start_span = self.last_span; // set by is_punctuation("|") above
    let mut params: Vec<(String, ClosureParamType)> = Vec::new();
    if !self.is_punctuation("|") {
        loop {
            let name = self.expect_identifier()?;
            if !self.is_punctuation(":") {
                return Err(self.error_at("expected ':' in closure parameter"));
            }
            let ty = self.parse_closure_type_expression()?;
            params.push((name, ty));
            if self.is_punctuation(",") {
                continue;
            }
            break;
        }
        if !self.is_punctuation("|") {
            return Err(self.error_at("expected ',' or closing '|'"));
        }
    }

    let param_types: Vec<TypeId> = params.iter().map(|(_, ty)| ty.type_id()).collect();
    let isolated = self.op_lookup.isolate_scopes();
    let param_table: std::collections::HashMap<String, (usize, ClosureParamType)> = params
        .into_iter()
        .enumerate()
        .map(|(idx, (name, ty))| (name, (idx, ty)))
        .collect();
    self.op_lookup
        .push_scope(move |name, segment, arity, _span| {
            if arity != 0 {
                return Ok(false);
            }
            let Some((idx, ty)) = param_table.get(name) else {
                return Ok(false);
            };
            match ty {
                ClosureParamType::Scalar(scalar) => (scalar.push_arg)(segment, *idx),
                ClosureParamType::Tuple(elements) => {
                    segment.push_arg_as_dynamic_sequence_tuple(*idx, elements_to_associated(elements))
                }
            }
            Ok(true)
        });

    let body_result = self.parse_nested_context(|p| p.is_or_expression());
    self.op_lookup.pop_scope();
    self.op_lookup.restore_scopes(isolated);
    let body = body_result?;

    let return_type = body
        .peek_stack_infos(1)
        .first()
        .map(|info| info.type_id)
        .ok_or_else(|| self.error_at("closure body must produce exactly one value"))?;

    self.context
        .push_closure(param_types, return_type, body, start_span)?;
    Ok(true)
}

/// One resolved closure parameter type: a built-in scalar, or a tuple of them (recursively).
enum ClosureParamType {
    Scalar(crate::op_table::BuiltinScalarType),
    Tuple(Vec<ClosureParamType>),
}

impl ClosureParamType {
    fn type_id(&self) -> TypeId {
        match self {
            ClosureParamType::Scalar(s) => s.type_id,
            ClosureParamType::Tuple(_) => TypeId::of::<cel_runtime::DynTuple>(),
        }
    }
}

/// `closure_type_expression = identifier | "(" [ closure_type_expression { "," closure_type_expression } ] ")" .`
fn parse_closure_type_expression(&mut self) -> Result<ClosureParamType> {
    if let Some(Token::Identifier(ident)) = self.peek_token() {
        let name = ident.to_string();
        self.advance();
        return crate::op_table::builtin_scalar_type(&name)
            .map(ClosureParamType::Scalar)
            .ok_or_else(|| self.error_at(&format!("unknown type `{name}`")));
    }
    if !self.is_open_paren() {
        return Err(self.error_at("expected a type name or '('"));
    }
    let mut elements = Vec::new();
    if !self.is_close_paren() {
        loop {
            elements.push(self.parse_closure_type_expression()?);
            if self.is_punctuation(",") {
                continue;
            }
            break;
        }
        if !self.is_close_paren() {
            return Err(self.error_at("expected ',' or closing ')'"));
        }
    }
    Ok(ClosureParamType::Tuple(elements))
}

/// Builds a fresh `AssociatedType` prototype list from resolved closure parameter element
/// types, for `DynSegment::push_arg_as_dynamic_sequence_tuple` — leaf `size`/`align` are the
/// scalar's real values (that method's own precondition), a nested tuple's are placeholders
/// (`push_arg_as_dynamic_sequence_tuple` recomputes them recursively).
fn elements_to_associated(elements: &[ClosureParamType]) -> Vec<cel_runtime::AssociatedType> {
    elements
        .iter()
        .map(|ty| match ty {
            ClosureParamType::Scalar(s) => cel_runtime::AssociatedType {
                type_id: s.type_id,
                type_name: std::borrow::Cow::Borrowed(s.type_name),
                offset: 0,
                size: s.size,
                align: s.align,
                dropper: s.dropper,
                associated: Vec::new(),
            },
            ClosureParamType::Tuple(nested) => cel_runtime::AssociatedType {
                type_id: TypeId::of::<cel_runtime::DynTuple>(),
                type_name: std::borrow::Cow::Borrowed("tuple"),
                offset: 0,
                size: 0,
                align: 1,
                dropper: cel_runtime::drop_tuple,
                associated: elements_to_associated(nested),
            },
        })
        .collect()
}
```

Adjust exact helper method names (`is_open_paren`/`is_close_paren`/`expect_identifier`/
`error_at`/`advance`/`is_punctuation`) to match whatever `Parser<C>`'s existing private helpers are
actually called — `is_tuple_or_group` (already in this file) and `is_bitwise_or_expression` use
`self.advance()`, `self.is_punctuation(...)`, `self.peek_token()`, `self.error_at(...)` already;
confirm the exact identifier-consuming helper's name (used by the `Token::Identifier` arm of
`is_primary_expression`) before writing this step for real, and use that one rather than a
guessed `expect_identifier`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser lib::tests::closure`
Expected: PASS (7 tests).

- [ ] **Step 5: Run the full `cel-parser` test suite**

Run: `cargo test -p cel-parser`
Expected: PASS, no new warnings.

- [ ] **Step 6: Commit**

```bash
git add cel-parser/src/lib.rs
git commit -m "feat(cel-parser): |params: Type| expr closure literal grammar"
```

---

## Task 5: `adam-lang` — `filter` clause on `cell` declarations

**Files:**
- Modify: `adam-lang/src/parser.rs`
- Test: inline `#[cfg(test)]` additions in `parser.rs`

**Interfaces:**
- Consumes: Task 4's closure grammar (via `self.parse_cel_or_expression`, unchanged call site),
  `adam_rs::{Filter, Sheet}` (existing, unmodified), `cel_runtime::DynClosure` (Task 1).
- Produces: `cell_decl` now optionally ends in a `filter` clause; no new public `AdamParser` API.

- [ ] **Step 1: Write the failing tests**

```rust
// adam-lang/src/parser.rs, in the existing #[cfg(test)] mod tests
#[test]
fn cell_filter_with_no_extra_args_clamps_on_write() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    let mut parsed = parser
        .parse_str("sheet s { cell a: i32 filter |x: i32| if x < 1 { 1 } else if x > 100 { 100 } else { x }; }")
        .unwrap();
    let (cell_id, _) = parsed.cell_names["a"];
    parsed.sheet.write(cell_id, 500i32).unwrap();
    assert_eq!(*parsed.sheet.effective::<i32>(cell_id).unwrap(), 100);
}

#[test]
fn cell_filter_with_named_arg_cell_tracks_its_current_value() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    let mut parsed = parser
        .parse_str(
            "sheet s { \
                 cell hi: i32 = 100; \
                 cell a: i32 filter(hi) |x: i32, h: i32| if x < 1 { 1 } else if x > h { h } else { x }; \
             }",
        )
        .unwrap();
    let (a_id, _) = parsed.cell_names["a"];
    let (hi_id, _) = parsed.cell_names["hi"];

    parsed.sheet.write(a_id, 500i32).unwrap();
    assert_eq!(*parsed.sheet.effective::<i32>(a_id).unwrap(), 100);

    parsed.sheet.write(hi_id, 10i32).unwrap();
    parsed.sheet.write(a_id, 500i32).unwrap();
    assert_eq!(*parsed.sheet.effective::<i32>(a_id).unwrap(), 10);
}

#[test]
fn cell_filter_first_param_type_mismatch_is_a_parse_error() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    let err = parser.parse_str("sheet s { cell a: i32 filter |x: f64| x; }");
    assert!(err.is_err());
}

#[test]
fn cell_filter_named_arg_type_mismatch_is_a_parse_error() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    let err = parser.parse_str(
        "sheet s { \
             cell hi: f64 = 100.0; \
             cell a: i32 filter(hi) |x: i32, h: i32| if x < 1 { 1 } else if x > h { h } else { x }; \
         }",
    );
    assert!(err.is_err());
}

#[test]
fn cell_filter_undeclared_arg_cell_is_a_parse_error() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    let err = parser.parse_str("sheet s { cell a: i32 filter(nope) |x: i32, h: i32| x; }");
    assert!(err.is_err());
}
```

Note: confirmed (during Task 2's review) that `adam-lang` has no dependency on `cel-std` and never
calls `install` anywhere, so `clamp` itself is not reachable from any adam-lang source text today —
a pre-existing gap unrelated to closures, tracked in
[stlab/cel-rs#137](https://github.com/stlab/cel-rs/issues/137). The tests above already use the
`if`/comparison-based equivalent instead, per `cel-parser/src/lib.rs`'s existing
`if_expression = "if" or_expression "{" or_expression "}" [ "else" ( "{" or_expression "}" |
if_expression ) ]` grammar (confirmed to support chained `else if` directly).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p adam-lang parser::tests::cell_filter`
Expected: FAIL — `filter` is not a recognized token in `cell_decl` yet (parse error on every case,
including the ones expected to succeed).

- [ ] **Step 3: Implement the grammar**

In `parse_cell_decl` (`adam-lang/src/parser.rs:203`), after the existing initializer/default-value
handling and before `ctx.expect_punct(";")?`, add:

```rust
let filter = if ctx.is_keyword("filter") {
    Some(self.parse_cell_filter(ctx, &name, name_span, cell_id, &shape)?)
} else {
    None
};

ctx.expect_punct(";")?;
ctx.cell_names.insert(name, (cell_id, shape));
if let Some(filter) = filter {
    ctx.sheet
        .add_filter(cell_id, filter)
        .map_err(|e| ParseError::new(e.to_string(), name_span))?;
}
Ok(())
```

(replacing the existing bare `ctx.expect_punct(";")?; ctx.cell_names.insert(name, (cell_id,
shape)); Ok(())` tail — check the exact current tail text at `parser.rs:249-251` before editing,
since this plan was written against a snapshot of it.)

Add the new production:

```rust
/// `cell_filter = "filter" [ "(" identifier { "," identifier } ")" ] closure_expression .`
///
/// `cell_id`/`declared_shape` are the filtered cell's own identity/type, already resolved by the
/// caller — used to validate the closure's first parameter and to build the returned `Filter`.
fn parse_cell_filter(
    &mut self,
    ctx: &mut ParseContext,
    cell_name: &str,
    cell_span: Span,
    cell_id: CellId,
    declared_shape: &TypeShape,
) -> Result<adam_rs::Filter> {
    ctx.is_keyword("filter"); // consume

    let mut arg_cells: Vec<(CellId, TypeShape)> = Vec::new();
    if ctx.consume_punct("(") {
        loop {
            let (arg_name, arg_span) = ctx.consume_ident()?;
            let (arg_id, arg_shape) = ctx
                .cell_names
                .get(&arg_name)
                .cloned()
                .ok_or_else(|| ParseError::new(format!("undeclared cell `{arg_name}`"), arg_span))?;
            arg_cells.push((arg_id, arg_shape));
            if ctx.consume_punct(",") {
                continue;
            }
            break;
        }
        ctx.expect_punct(")")?;
    }

    let segment = self.parse_cel_or_expression(ctx)?;
    let closure: cel_runtime::DynClosure = segment
        .call0()
        .map_err(|e| ParseError::new(format!("filter: {e}"), cell_span))?;

    let value_type_id = cell_type_id(declared_shape);
    let expected_param_types: Vec<TypeId> = std::iter::once(value_type_id)
        .chain(arg_cells.iter().map(|(_, shape)| cell_type_id(shape)))
        .collect();
    if closure.param_types() != expected_param_types.as_slice() {
        return Err(ParseError::new(
            format!(
                "cell `{cell_name}`: filter closure parameter types don't match `{cell_name}`'s \
                 type followed by the filter's declared argument cells' types"
            ),
            cell_span,
        ));
    }
    if closure.return_type() != value_type_id {
        return Err(ParseError::new(
            format!(
                "cell `{cell_name}`: filter closure must return `{}`",
                self.types.display_name(declared_shape)
            ),
            cell_span,
        ));
    }

    // `call_dyn_fn` is the same monomorphized-per-registered-type dispatcher
    // `build_method` already uses (`adam-lang/src/parser.rs:1133`,
    // `TypeRegistry::TypeEntry::call_dyn_fn`) — reused here via `DynClosure::call_boxed`
    // (Task 1) instead of `DynClosure::call::<T>`, since `T` is only known dynamically here
    // (as `value_type_id`), not as a static Rust generic.
    let call_dyn_fn = self
        .types
        .entry_by_type_id(value_type_id)
        .expect("declared cell type registered")
        .call_dyn_fn;

    let arg_ids: Vec<CellId> = arg_cells.iter().map(|(id, _)| *id).collect();
    let arg_type_ids: Vec<TypeId> = arg_cells.iter().map(|(_, shape)| cell_type_id(shape)).collect();
    Ok(adam_rs::Filter::new(value_type_id, arg_ids, arg_type_ids, move |value, args| {
        let mut call_args: Vec<&dyn Any> = Vec::with_capacity(1 + args.len());
        call_args.push(value);
        call_args.extend_from_slice(args);
        // `Filter::new`'s own `value`/`args` are already downcast-checked by
        // `Sheet::add_filter`/`write`/`propagate` against `value_type_id`/`arg_type_ids` before
        // this closure runs, matching `closure.param_types()` exactly (checked above) — so
        // `DynClosure::call_boxed`'s own type-matching precondition already holds here.
        closure
            .call_boxed(&call_args, call_dyn_fn)
            .map_err(|e| anyhow::anyhow!("filter: {e}"))
    }))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p adam-lang parser::tests::cell_filter`
Expected: PASS (5 tests).

- [ ] **Step 5: Run the full `adam-lang` test suite**

Run: `cargo test -p adam-lang`
Expected: PASS, no new warnings.

- [ ] **Step 6: Commit**

```bash
git add adam-lang/src/parser.rs
git commit -m "feat(adam-lang): filter clause on cell declarations"
```

---

## Task 6: End-to-end integration test (the spec's motivating example)

**Files:**
- Test: `adam-lang/src/parser.rs` (or `adam-lang/tests/` if an existing integration-test
  convention lives there — check for an existing `adam-lang/tests/*.rs` file first and follow
  whichever convention this crate already uses for whole-sheet acceptance tests).

**Interfaces:**
- Consumes: everything from Tasks 1–5. Produces nothing new — this is a pure verification task.

- [ ] **Step 1: Write the test**

```rust
#[test]
fn filter_tracks_a_tuple_typed_range_cell_dynamically() {
    let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
    let mut parsed = parser
        .parse_str(
            "sheet s { \
                 cell a_range: (i32, i32) = (1, 100); \
                 cell max: i32 = 100; \
                 relate { a_range := (1, max); } \
                 cell a: i32 filter(a_range) |x: i32, r: (i32, i32)| \
                     if x < r.0 { r.0 } else if x > r.1 { r.1 } else { x }; \
             }",
        )
        .unwrap();
    let (a_id, _) = parsed.cell_names["a"];
    let (max_id, _) = parsed.cell_names["max"];

    parsed.sheet.write(a_id, 500i32).unwrap();
    assert_eq!(*parsed.sheet.effective::<i32>(a_id).unwrap(), 100);

    parsed.sheet.write(max_id, 10i32).unwrap();
    parsed.sheet.propagate().unwrap();
    parsed.sheet.write(a_id, 500i32).unwrap();
    assert_eq!(*parsed.sheet.effective::<i32>(a_id).unwrap(), 10);
}
```

Adjust the `if`/`else if` conditional syntax to whatever adam-lang's/cel-parser's actual `if`
expression grammar is (confirmed to exist per `is_if_expression` in `cel-parser/src/lib.rs`,
Task 4's `is_primary_expression` excerpt above) before finalizing — this plan assumes but does not
re-verify its exact surface syntax.

- [ ] **Step 2: Run the test to verify it fails for the right reason**

Run: `cargo test -p adam-lang filter_tracks_a_tuple_typed_range_cell_dynamically`
Expected (before this task, i.e. if run against Task 5's own commit): PASS already, since Tasks
1–5 are individually sufficient — this task exists to prove the *composed* motivating example
works, not to add new behavior. If it fails, that's a real integration gap between Tasks 1–5 to
fix before moving on, not something to patch around here.

- [ ] **Step 3: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS, zero warnings (per Global Constraints).

- [ ] **Step 4: Run all three clippy invocations**

```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

Expected: all three clean.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test(adam-lang): end-to-end filter tracking a tuple-typed range cell"
```

---

## Self-Review Notes

**Spec coverage:** `DynClosure` (Task 1) ↔ spec §1; scalar/tuple type table + scope isolation
(Task 2) ↔ spec §2's new-surface-area paragraph; nested context + `push_closure` (Task 3) ↔ spec
§2 steps 2–3/5; closure grammar (Task 4) ↔ spec §2 steps 1/4 + the "no new cel-runtime primitive"
`call` design; `filter` clause (Task 5) ↔ spec §3 + "Error handling"; the `a_range` motivating
example (Task 6) ↔ spec's worked example and its "Testing strategy" end-to-end bullet. Every
`Non-goal` in the spec (general `f(x)` application, closures as a cell type, the macro/static
path, recursion, type inference, auto free-variable capture, `AstContext` support) has no
corresponding task — confirmed intentionally absent, not a gap.

**Fixed during self-review:** Task 5's `Filter`-wrapper closure originally called
`DynClosure::call::<Box<dyn Any>>`, which can't work — `call`'s own `debug_assert_eq!(TypeId::of
::<R>(), self.0.return_type)` can never hold for `R = Box<dyn Any>` against a real closure's
concrete `return_type`. Fixed by adding `DynClosure::call_boxed` to Task 1 (takes a monomorphized
`fn(&mut DynSegment, &[&dyn Any]) -> anyhow::Result<Box<dyn Any>>` dispatcher instead of a static
`R`) and having Task 5 supply `TypeRegistry::TypeEntry::call_dyn_fn` for it — the exact same
per-registered-type dispatcher `build_method` already uses (`adam-lang/src/parser.rs:1133`,
`adam-lang/src/type_registry.rs:69`), so this introduces no new type-erasure mechanism, only a new
consumer of an existing one.

**Also fixed during self-review:** every Task 5/6 test string had `filter ... = 0;` — the
initializer written *after* the filter clause, but `parse_cell_decl` parses `filter` after the
initializer (Task 5, Step 3), so this was backwards relative to the very grammar the task
implements. Since none of these tests actually need an explicit initializer (each immediately
overwrites the cell via `write`, and `i32`'s `Default` already gives `0`), fixed by dropping `= 0`
entirely rather than reordering it, matching the spec's own examples exactly.

**Also found and fixed during SDD execution (Task 2's review, ledger has the full ruling):**
`isolate_scopes`'s original blanket-clear design would have made `round` — and, once `cel-std` is
ever wired into `adam-lang` (tracked separately, `stlab/cel-rs#137`), `clamp`/`min`/`max`/etc. —
unreachable from inside every closure body, backwards from the "library functions should always
be reachable" intent. Fixed via a `push_library_scope`/`library_scope_count` floor (Task 2) that
`isolate_scopes` respects. Confirmed `adam-lang` has no `cel-std` dependency and never calls
`install` today, so Tasks 5/6's tests were switched from `clamp(...)` to an equivalent built from
`cel-parser`'s existing `if`/`else if` expression grammar, verified against
`cel-parser/src/lib.rs`'s `is_if_expression` (confirmed to support chained `else if` directly, no
extra braces needed).

**Placeholder scan:** No `TBD`/`unimplemented!()`/hand-waved steps remain in the tasks above. The
only remaining "confirm before writing this step for real" notes are the exact-helper-name
confirmations in Tasks 4/6 (`is_open_paren`/`expect_identifier`) — each names precisely what to
check and where, not "figure it out later" in the abstract; they exist because this plan was
written from reading the source rather than running it, and a fresh executor should
verify a symbol's exact spelling before typing it, not because the design has a gap.
