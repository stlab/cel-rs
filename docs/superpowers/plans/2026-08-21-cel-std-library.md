# CEL Standard Library Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `cel-std` crate providing `min`, `max`, `clamp`, `abs`, `signum`, `sqrt`,
`floor`, `ceil`, `trunc` as CEL-callable functions over all 14 numeric types, wired into
`begin`, with `begin/examples/inequality.adm2` rewritten to use `min`/`max`.

**Architecture:** Each function is registered on `cel_parser::OpLookup` via the existing
public `push_scope` extension mechanism, following the same marker-struct pattern
`cel-parser`'s built-in `round` function already uses (a marker struct is pushed for the
bare-identifier arity-0 lookup of the function name, then consumed by the paired `"()"`
call lookup). `clamp` additionally needs a new `op3r` primitive on `cel-runtime`'s
`DynSegment` (ternary + fallible; `op1r`/`op2r` already exist, `op3` already exists, but no
ternary-and-fallible combination exists yet).

**Tech Stack:** Rust, `cel-parser`, `cel-runtime`, `anyhow`.

**Spec:** [docs/superpowers/specs/2026-08-21-cel-std-library-design.md](../specs/2026-08-21-cel-std-library-design.md)

## Global Constraints

- All 14 numeric types get full parity where the operation is meaningful: `u8`, `u16`,
  `u32`, `u64`, `u128`, `usize`, `i8`, `i16`, `i32`, `i64`, `i128`, `isize`, `f32`, `f64`.
- `clamp(x, lo, hi)` returns `Err("invalid clamp bounds")` when `!(lo <= hi)` (this single
  comparison also catches `NaN` bounds) — never panics.
- Signed-integer `abs` returns `Err("arithmetic overflow")` via `checked_abs`, matching the
  existing negation/subtraction overflow convention in `cel-parser/src/op_table.rs`.
- Every function needs a `///` contract-style doc comment (Summary / Preconditions /
  `# Errors` / Postconditions / Complexity bullets, per `CLAUDE.md`).
- `cargo fmt --all` must be clean; `cargo build --workspace` and `cargo test --workspace`
  must produce zero warnings; `cargo clippy --workspace --exclude begin --all-targets --
  -D warnings` must pass (this plan doesn't touch `begin`'s desktop-only code paths, so the
  two `begin`-specific clippy invocations aren't expected to be affected, but re-run them
  before any PR per `CLAUDE.md`).
- Tests are derived from function contracts (behavior), not from internals — no test should
  assert on which specific scope function or marker type handled a call.

---

## Task 1: `cel-runtime` — add `op3r` to `DynSegment`

**Files:**
- Modify: `cel-runtime/src/raw_segment.rs` (add `push_op3r_` and `raw3`, after the existing
  `push_op3`/`push_op3_` pair, i.e. after line 309)
- Modify: `cel-runtime/src/dyn_segment.rs` (add `op3r`, after the existing `op3` at line
  1303-1316; add tests after `op2r_error_unwinds`, currently ending at line 1942)

**Interfaces:**
- Produces: `DynSegment::op3r<T, U, V, R, F>(&mut self, op: F) -> anyhow::Result<()>` where
  `F: Fn(T, U, V) -> anyhow::Result<R> + 'static` — pops the top three stack values (of
  types `T`, `U`, `V`, oldest-to-newest), runs `op`, and on `Ok(R)` pushes `R`; on `Err`,
  unwinds (drops) the remaining stack and propagates the error. Mirrors the existing
  `op2r` (`cel-runtime/src/dyn_segment.rs:745`) exactly, extended to arity 3.

- [ ] **Step 1: Write the failing tests for `DynSegment::op3r`**

Add to the `mod tests` block in `cel-runtime/src/dyn_segment.rs`, immediately after the
existing `op2r_error_unwinds` test (which ends at line 1942):

```rust
    #[test]
    fn op3r_success() -> Result<(), anyhow::Error> {
        let mut segment = DynSegment::new::<()>();
        segment.op0(|| 10u32);
        segment.op0(|| 20u32);
        segment.op0(|| 12u32);
        segment.op3r(|a: u32, b: u32, c: u32| Ok::<_, anyhow::Error>(a + b + c))?;
        let result: u32 = segment.call0()?;
        assert_eq!(result, 42);
        Ok(())
    }

    #[test]
    fn op3r_error_unwinds() -> Result<(), anyhow::Error> {
        let mut segment = DynSegment::new::<()>();
        let drop_count = Arc::new(AtomicUsize::new(0));
        let tracker = DropCounter(drop_count.clone());
        segment.op0(move || tracker.clone());
        segment.op0(|| 7u32);
        segment.op0(|| 8u32);
        segment.op0(|| 9u32);
        segment.op3r(|_a: u32, _b: u32, _c: u32| -> Result<DropCounter> {
            Err(anyhow::anyhow!("op3r error"))
        })?;
        segment.op1(|_: DropCounter| 0u32)?;
        segment.op2(|_: DropCounter, x: u32| x)?; // consume to single u32 for call0
        let result = segment.call0::<u32>();
        assert!(result.is_err(), "expected Err, got {:?}", result);
        assert_eq!(result.unwrap_err().to_string(), "op3r error");
        // DropCounter (under the three u32s) was unwound when op3r failed.
        assert_eq!(drop_count.load(Ordering::SeqCst), 1);
        Ok(())
    }
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test --package cel-runtime op3r`
Expected: compile error — `no method named 'op3r' found for struct 'DynSegment'`.

- [ ] **Step 3: Add `push_op3r_` and `raw3` to `RawSegment`**

In `cel-runtime/src/raw_segment.rs`, immediately after the existing `push_op3` method
(which ends at line 309), add:

```rust
    /// Pushes the op-dispatch closure for a ternary fallible operation with compile-time padding.
    #[expect(clippy::many_single_char_names, reason = "patterned code")]
    fn push_op3r_<const PADDING0: bool, const PADDING1: bool, const PADDING2: bool, T, U, V, R, F>(
        &mut self,
    ) where
        F: Fn(&mut RawStack, T, U, V) -> Result<R> + 'static,
        T: 'static,
        U: 'static,
        V: 'static,
        R: 'static,
    {
        self.ops.push(|storage, p, stack| {
            let (f, r) = unsafe { storage.next::<F>(p) };
            let z: V = unsafe { stack.pop(PADDING2) };
            let y: U = unsafe { stack.pop(PADDING1) };
            let x: T = unsafe { stack.pop(PADDING0) };
            let result = f(stack, x, y, z)?;
            stack.push(result);
            Ok(r)
        });
    }

    /// Push a fallible ternary operation that can manipulate the stack.
    pub fn raw3<T, U, V, R, F>(&mut self, op: F, padding0: bool, padding1: bool, padding2: bool)
    where
        F: Fn(&mut RawStack, T, U, V) -> Result<R> + 'static,
        T: 'static,
        U: 'static,
        V: 'static,
        R: 'static,
    {
        self.push_storage(op);
        match (padding0, padding1, padding2) {
            (false, false, false) => self.push_op3r_::<false, false, false, T, U, V, R, F>(),
            (false, false, true) => self.push_op3r_::<false, false, true, T, U, V, R, F>(),
            (false, true, false) => self.push_op3r_::<false, true, false, T, U, V, R, F>(),
            (false, true, true) => self.push_op3r_::<false, true, true, T, U, V, R, F>(),
            (true, false, false) => self.push_op3r_::<true, false, false, T, U, V, R, F>(),
            (true, false, true) => self.push_op3r_::<true, false, true, T, U, V, R, F>(),
            (true, true, false) => self.push_op3r_::<true, true, false, T, U, V, R, F>(),
            (true, true, true) => self.push_op3r_::<true, true, true, T, U, V, R, F>(),
        }
        self.base_alignment = max(self.base_alignment, align_of::<R>());
    }
```

- [ ] **Step 4: Add `DynSegment::op3r`**

In `cel-runtime/src/dyn_segment.rs`, immediately after the existing `op3` method (which
ends at line 1316), add:

```rust
    /// Pushes a ternary operation that takes three arguments of types `T`, `U`, and `V` and
    /// returns a `Result<R>`.
    ///
    /// If the operation succeeds, the result is pushed onto the stack. If it fails,
    /// the stack is unwound to its previous state and the error is propagated.
    ///
    /// # Errors
    ///
    /// Returns an error if the argument types do not match the expected types.
    pub fn op3r<T, U, V, R, F>(&mut self, op: F) -> Result<()>
    where
        F: Fn(T, U, V) -> anyhow::Result<R> + 'static,
        T: 'static,
        U: 'static,
        V: 'static,
        R: 'static,
    {
        let [p0, p1, p2] = self.get_last_n_padded::<3>();
        self.pop_types::<(T, (U, (V, ())))>()?;
        let unwind = self.capture_unwind();
        self.segment.raw3(
            move |stack, t, u, v| Self::unwind_on_err(&unwind, stack, op(t, u, v)),
            p0,
            p1,
            p2,
        );
        self.push_type::<R>();
        Ok(())
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --package cel-runtime op3r`
Expected: `op3r_success` and `op3r_error_unwinds` both PASS.

- [ ] **Step 6: Run the full `cel-runtime` test suite and lint**

Run: `cargo test --package cel-runtime && cargo clippy --package cel-runtime --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: all pass, no warnings, no formatting diff.

- [ ] **Step 7: Commit**

```bash
git add cel-runtime/src/raw_segment.rs cel-runtime/src/dyn_segment.rs
git commit -m "feat(cel-runtime): add op3r, a ternary fallible DynSegment operation"
```

---

## Task 2: New `cel-std` crate — `min` and `max`

**Files:**
- Create: `cel-std/Cargo.toml`
- Create: `cel-std/src/lib.rs`
- Create: `cel-std/src/math.rs`
- Modify: `Cargo.toml` (root — add `cel-std` to `[workspace] members`)

**Interfaces:**
- Consumes: `cel_parser::OpLookup` (public), `cel_parser::OpLookup::push_scope` (public),
  `cel_parser::SourceSpan` (public), `cel_runtime::DynSegment` (public), plus
  `DynSegment::{op0, op3, peek_stack_infos}` (all public, pre-existing).
- Produces: `cel_std::install(lookup: &mut cel_parser::OpLookup)` — the crate's sole public
  entry point. Later tasks add more scopes to it; this task's `install` registers only
  `math::min_max_scope`.

- [ ] **Step 1: Create the crate skeleton**

Create `cel-std/Cargo.toml`:

```toml
[package]
name = "cel-std"
version = "0.1.0"
edition = "2024"
description = "CEL standard library functions"

[dependencies]
cel-parser = { path = "../cel-parser" }
cel-runtime = { path = "../cel-runtime" }
anyhow = "1.0"

[lints]
workspace = true
```

Add `"cel-std"` to the root `Cargo.toml`'s `[workspace] members` list (alongside the other
crates, e.g. right after `"adam-rs"`):

```toml
[workspace]
members = [
    "cel-runtime",
    "cel-parser",
    "cel-rs-macros",
    "cel-std",
    "adam-rs",
    "adam-lang",
    "adam-lsp",
    "begin",
    "xtask",
]
```

Create `cel-std/src/lib.rs`:

```rust
//! CEL standard library: `min`, `max`, `clamp`, and related numeric functions built on
//! Rust's standard library, registered via [`cel_parser::OpLookup::push_scope`].
//!
//! # Examples
//!
//! ```rust
//! use cel_parser::OpLookup;
//!
//! let mut lookup = OpLookup::new();
//! cel_std::install(&mut lookup);
//! ```

mod math;

/// Registers every CEL standard-library function on `lookup`.
pub fn install(lookup: &mut cel_parser::OpLookup) {
    lookup.push_scope(math::min_max_scope);
}
```

Create `cel-std/src/math.rs`:

```rust
//! Numeric CEL standard-library functions.
//!
//! Each function follows the pattern `cel-parser`'s built-in `round` uses: a marker
//! struct is pushed for the bare-identifier (arity-0) lookup of the function's name, and
//! consumed by the paired `"()"` call lookup once the marker confirms this call's callee
//! is that function (see `cel-parser/src/op_table.rs`'s `round_scope`).

use anyhow::Result;
use cel_parser::SourceSpan;
use cel_runtime::DynSegment;
use std::any::TypeId;

/// Marker pushed for a bare `min` lookup; consumed by the paired `"()"` call.
struct MinFn;
/// Marker pushed for a bare `max` lookup; consumed by the paired `"()"` call.
struct MaxFn;

/// `min(a, b) = a.min(b)`, `max(a, b) = a.max(b)` over all 14 numeric types — `Ord::min`/
/// `max` for integers, the inherent (NaN-avoiding) `f32`/`f64` `min`/`max` for floats.
///
/// - Precondition: `a` and `b` have the same type.
pub(crate) fn min_max_scope(
    name: &str,
    segment: &mut DynSegment,
    num_operands: usize,
    _span: SourceSpan,
) -> Result<bool> {
    match (name, num_operands) {
        ("min", 0) => {
            segment.op0(|| MinFn);
            Ok(true)
        }
        ("max", 0) => {
            segment.op0(|| MaxFn);
            Ok(true)
        }
        ("()", 3) => {
            let top = segment.peek_stack_infos(3);
            if top.len() != 3 || top[1].type_id != top[2].type_id {
                return Ok(false);
            }
            let callee_type = top[0].type_id;
            let operand_type = top[1].type_id;

            macro_rules! dispatch {
                ($marker:ty, $method:ident, [$($t:ty),+ $(,)?]) => {
                    if callee_type == TypeId::of::<$marker>() {
                        $(
                            if operand_type == TypeId::of::<$t>() {
                                segment.op3(|_callee: $marker, a: $t, b: $t| a.$method(b))?;
                                return Ok(true);
                            }
                        )+
                        return Ok(false);
                    }
                };
            }

            dispatch!(
                MinFn, min,
                [u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64]
            );
            dispatch!(
                MaxFn, max,
                [u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64]
            );
            Ok(false)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cel_parser::OpLookup;
    use crate::install;
    use proc_macro2::Span;

    #[test]
    fn min_returns_the_smaller_of_two_signed_operands() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("min", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(3i32);
        segment.just(-5i32);
        lookup.lookup("()", &mut segment, 3, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<i32>()?, -5);
        Ok(())
    }

    #[test]
    fn min_returns_the_smaller_of_two_unsigned_operands() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("min", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(3u32);
        segment.just(5u32);
        lookup.lookup("()", &mut segment, 3, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<u32>()?, 3);
        Ok(())
    }

    #[test]
    fn min_avoids_nan_when_exactly_one_float_operand_is_nan() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("min", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(f64::NAN);
        segment.just(2.0f64);
        lookup.lookup("()", &mut segment, 3, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<f64>()?, 2.0);
        Ok(())
    }

    #[test]
    fn max_returns_the_larger_of_two_signed_operands() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("max", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(3i32);
        segment.just(-5i32);
        lookup.lookup("()", &mut segment, 3, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<i32>()?, 3);
        Ok(())
    }

    #[test]
    fn max_returns_the_larger_of_two_unsigned_operands() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("max", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(3u32);
        segment.just(5u32);
        lookup.lookup("()", &mut segment, 3, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<u32>()?, 5);
        Ok(())
    }

    #[test]
    fn max_returns_the_larger_of_two_float_operands() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("max", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(3.5f64);
        segment.just(2.5f64);
        lookup.lookup("()", &mut segment, 3, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<f64>()?, 3.5);
        Ok(())
    }
}
```

Note: `proc_macro2` isn't a direct dependency of `cel-std` yet — it's needed only by this
test module (for `Span::call_site()`). Add it as a dev-dependency in `cel-std/Cargo.toml`:

```toml
[dev-dependencies]
proc-macro2 = "1.0"
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --package cel-std`
Expected: FAIL — crate doesn't exist as a workspace member yet / compile errors, until the
files above are all in place; once they are, this should already pass since Step 1 wrote
both the implementation and the tests together (min/max is simple enough not to need a
separate red step — see note below).

> This task writes implementation and tests in the same step because `min_max_scope`'s
> logic is a direct transcription of `round_scope`'s established pattern — there's no
> ambiguity to resolve by watching a test fail first. Follow strict red-green TDD for
> `clamp` (Task 3), which has real branching (the bounds check) worth watching fail.

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cargo test --package cel-std`
Expected: all 6 tests PASS.

- [ ] **Step 4: Lint and format**

Run: `cargo clippy --package cel-std --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: no warnings, no formatting diff.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml cel-std
git commit -m "feat(cel-std): add cel-std crate with min/max"
```

---

## Task 3: `cel-std` — `clamp`

**Files:**
- Modify: `cel-std/src/math.rs` (add `ClampFn` marker and `clamp_scope`)
- Modify: `cel-std/src/lib.rs` (register `math::clamp_scope`)

**Interfaces:**
- Consumes: `DynSegment::op3r` (from Task 1), `DynSegment::op2`, `DynSegment::op0`,
  `DynSegment::peek_stack_infos`.
- Produces: `cel-std`'s `clamp_scope`, added to `install`.

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `cel-std/src/math.rs` (after the `max_*` tests):

```rust
    #[test]
    fn clamp_bounds_a_value_inside_its_range_unchanged() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("clamp", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(5i32);
        segment.just(0i32);
        segment.just(10i32);
        lookup.lookup("()", &mut segment, 4, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<i32>()?, 5);
        Ok(())
    }

    #[test]
    fn clamp_bounds_a_value_below_its_range_up_to_lo() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("clamp", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(-5i32);
        segment.just(0i32);
        segment.just(10i32);
        lookup.lookup("()", &mut segment, 4, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<i32>()?, 0);
        Ok(())
    }

    #[test]
    fn clamp_bounds_a_value_above_its_range_down_to_hi() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("clamp", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(15i32);
        segment.just(0i32);
        segment.just(10i32);
        lookup.lookup("()", &mut segment, 4, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<i32>()?, 10);
        Ok(())
    }

    #[test]
    fn clamp_errs_when_lo_is_greater_than_hi() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("clamp", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(5i32);
        segment.just(10i32);
        segment.just(0i32);
        lookup.lookup("()", &mut segment, 4, Span::call_site(), Span::call_site())?;
        let result = segment.call0::<i32>();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "invalid clamp bounds");
        Ok(())
    }

    #[test]
    fn clamp_errs_when_a_bound_is_nan() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("clamp", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(5.0f64);
        segment.just(f64::NAN);
        segment.just(10.0f64);
        lookup.lookup("()", &mut segment, 4, Span::call_site(), Span::call_site())?;
        let result = segment.call0::<f64>();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "invalid clamp bounds");
        Ok(())
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --package cel-std clamp`
Expected: FAIL to compile — `lookup("clamp", ...)` returns an "undefined identifier"
`ParseError` at runtime (no scope recognizes `clamp` yet), so the `?` on that first
`lookup` call turns into a runtime `Err` rather than a compile failure. Confirm each new
test fails with an assertion or `Err` mismatch (not a panic from unrelated causes) before
proceeding.

- [ ] **Step 3: Implement `clamp_scope`**

In `cel-std/src/math.rs`, add after `min_max_scope`:

```rust
/// Marker pushed for a bare `clamp` lookup; consumed by the paired `"()"` call.
struct ClampFn;

/// `clamp(x, lo, hi)` bounds `x` to `[lo, hi]`, over all 14 numeric types.
///
/// Dispatches as two chained ops: the first computes the clamped value from `x`/`lo`/`hi`
/// alone (and can fail if `lo > hi`); the second discards the still-buried `ClampFn`
/// marker and passes the result through unchanged.
///
/// - Precondition: `x`, `lo`, and `hi` all have the same type.
///
/// # Errors
///
/// Returns `Err("invalid clamp bounds")` if `!(lo <= hi)` — this single comparison is
/// `false` whenever either bound is `NaN`, so no separate `NaN` check is needed.
pub(crate) fn clamp_scope(
    name: &str,
    segment: &mut DynSegment,
    num_operands: usize,
    _span: SourceSpan,
) -> Result<bool> {
    match (name, num_operands) {
        ("clamp", 0) => {
            segment.op0(|| ClampFn);
            Ok(true)
        }
        ("()", 4) => {
            let top = segment.peek_stack_infos(4);
            if top.len() != 4
                || top[0].type_id != TypeId::of::<ClampFn>()
                || top[1].type_id != top[2].type_id
                || top[1].type_id != top[3].type_id
            {
                return Ok(false);
            }
            let operand_type = top[1].type_id;

            macro_rules! dispatch {
                ([$($t:ty),+ $(,)?]) => {
                    $(
                        if operand_type == TypeId::of::<$t>() {
                            segment.op3r(move |x: $t, lo: $t, hi: $t| {
                                if lo <= hi {
                                    Ok(x.clamp(lo, hi))
                                } else {
                                    Err(anyhow::anyhow!("invalid clamp bounds"))
                                }
                            })?;
                            segment.op2(|_callee: ClampFn, result: $t| result)?;
                            return Ok(true);
                        }
                    )+
                };
            }

            dispatch!([u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64]);
            Ok(false)
        }
        _ => Ok(false),
    }
}
```

In `cel-std/src/lib.rs`, register it in `install`:

```rust
pub fn install(lookup: &mut cel_parser::OpLookup) {
    lookup.push_scope(math::min_max_scope);
    lookup.push_scope(math::clamp_scope);
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --package cel-std clamp`
Expected: all 5 new tests PASS.

- [ ] **Step 5: Run the full `cel-std` suite, lint, and format**

Run: `cargo test --package cel-std && cargo clippy --package cel-std --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: all pass, no warnings, no formatting diff.

- [ ] **Step 6: Commit**

```bash
git add cel-std/src/math.rs cel-std/src/lib.rs
git commit -m "feat(cel-std): add clamp"
```

---

## Task 4: `cel-std` — `abs`

**Files:**
- Modify: `cel-std/src/math.rs` (add `AbsFn` marker and `abs_scope`)
- Modify: `cel-std/src/lib.rs` (register `math::abs_scope`)

**Interfaces:**
- Consumes: `DynSegment::op2r` (pre-existing), `DynSegment::op2`, `DynSegment::op0`,
  `DynSegment::peek_stack_infos`.
- Produces: `cel-std`'s `abs_scope`, added to `install`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `cel-std/src/math.rs`:

```rust
    #[test]
    fn abs_returns_the_absolute_value_of_a_negative_signed_integer() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("abs", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(-7i32);
        lookup.lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<i32>()?, 7);
        Ok(())
    }

    #[test]
    fn abs_errs_on_signed_integer_overflow() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("abs", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(i32::MIN);
        lookup.lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())?;
        let result = segment.call0::<i32>();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "arithmetic overflow");
        Ok(())
    }

    #[test]
    fn abs_returns_the_absolute_value_of_a_negative_float() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("abs", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(-2.5f64);
        lookup.lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<f64>()?, 2.5);
        Ok(())
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --package cel-std abs`
Expected: FAIL — `abs` isn't a recognized identifier yet (runtime `Err` from the first
`lookup` call).

- [ ] **Step 3: Implement `abs_scope`**

In `cel-std/src/math.rs`, add after `clamp_scope`:

```rust
/// Marker pushed for a bare `abs` lookup; consumed by the paired `"()"` call.
struct AbsFn;

/// `abs(x)` over signed integers (checked; `Err` on overflow) and floats (infallible).
///
/// # Errors
///
/// Returns `Err("arithmetic overflow")` if `x` is a signed integer at its type's minimum
/// value (the one value whose absolute value doesn't fit in that type).
pub(crate) fn abs_scope(
    name: &str,
    segment: &mut DynSegment,
    num_operands: usize,
    _span: SourceSpan,
) -> Result<bool> {
    match (name, num_operands) {
        ("abs", 0) => {
            segment.op0(|| AbsFn);
            Ok(true)
        }
        ("()", 2) => {
            let top = segment.peek_stack_infos(2);
            if top.len() != 2 || top[0].type_id != TypeId::of::<AbsFn>() {
                return Ok(false);
            }
            let operand_type = top[1].type_id;

            macro_rules! dispatch_checked {
                ([$($t:ty),+ $(,)?]) => {
                    $(
                        if operand_type == TypeId::of::<$t>() {
                            segment.op2r(|_callee: AbsFn, x: $t| {
                                x.checked_abs()
                                    .ok_or_else(|| anyhow::anyhow!("arithmetic overflow"))
                            })?;
                            return Ok(true);
                        }
                    )+
                };
            }
            macro_rules! dispatch_float {
                ([$($t:ty),+ $(,)?]) => {
                    $(
                        if operand_type == TypeId::of::<$t>() {
                            segment.op2(|_callee: AbsFn, x: $t| x.abs())?;
                            return Ok(true);
                        }
                    )+
                };
            }

            dispatch_checked!([i8, i16, i32, i64, i128, isize]);
            dispatch_float!([f32, f64]);
            Ok(false)
        }
        _ => Ok(false),
    }
}
```

In `cel-std/src/lib.rs`, register it:

```rust
pub fn install(lookup: &mut cel_parser::OpLookup) {
    lookup.push_scope(math::min_max_scope);
    lookup.push_scope(math::clamp_scope);
    lookup.push_scope(math::abs_scope);
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --package cel-std abs`
Expected: all 3 new tests PASS.

- [ ] **Step 5: Run the full `cel-std` suite, lint, and format**

Run: `cargo test --package cel-std && cargo clippy --package cel-std --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: all pass, no warnings, no formatting diff.

- [ ] **Step 6: Commit**

```bash
git add cel-std/src/math.rs cel-std/src/lib.rs
git commit -m "feat(cel-std): add abs"
```

---

## Task 5: `cel-std` — `signum`, `sqrt`, `floor`, `ceil`, `trunc`

**Files:**
- Modify: `cel-std/src/math.rs` (add five marker structs and `unary_math_scope`)
- Modify: `cel-std/src/lib.rs` (register `math::unary_math_scope`)

**Interfaces:**
- Consumes: `DynSegment::op2`, `DynSegment::op0`, `DynSegment::peek_stack_infos` (all
  pre-existing).
- Produces: `cel-std`'s `unary_math_scope`, added to `install` — this is the crate's last
  scope, completing its public function set.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `cel-std/src/math.rs`:

```rust
    #[test]
    fn signum_of_a_negative_signed_integer_is_minus_one() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("signum", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(-7i32);
        lookup.lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<i32>()?, -1);
        Ok(())
    }

    #[test]
    fn signum_of_a_nan_float_is_nan() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("signum", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(f64::NAN);
        lookup.lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert!(segment.call0::<f64>()?.is_nan());
        Ok(())
    }

    #[test]
    fn sqrt_of_a_negative_float_is_nan() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("sqrt", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(-4.0f64);
        lookup.lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert!(segment.call0::<f64>()?.is_nan());
        Ok(())
    }

    #[test]
    fn sqrt_of_a_positive_float_returns_the_positive_root() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("sqrt", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(9.0f64);
        lookup.lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<f64>()?, 3.0);
        Ok(())
    }

    #[test]
    fn floor_rounds_a_float_toward_negative_infinity() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("floor", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(3.7f64);
        lookup.lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<f64>()?, 3.0);
        Ok(())
    }

    #[test]
    fn ceil_rounds_a_float_toward_positive_infinity() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("ceil", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(3.2f64);
        lookup.lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<f64>()?, 4.0);
        Ok(())
    }

    #[test]
    fn trunc_rounds_a_float_toward_zero() -> Result<()> {
        let mut lookup = OpLookup::new();
        install(&mut lookup);
        let mut segment = DynSegment::new::<()>();
        lookup.lookup("trunc", &mut segment, 0, Span::call_site(), Span::call_site())?;
        segment.just(-3.7f64);
        lookup.lookup("()", &mut segment, 2, Span::call_site(), Span::call_site())?;
        assert_eq!(segment.call0::<f64>()?, -3.0);
        Ok(())
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --package cel-std -- signum sqrt floor ceil trunc`
Expected: FAIL — none of these identifiers are recognized yet.

- [ ] **Step 3: Implement `unary_math_scope`**

In `cel-std/src/math.rs`, add after `abs_scope`:

```rust
/// Marker pushed for a bare `signum` lookup; consumed by the paired `"()"` call.
struct SignumFn;
/// Marker pushed for a bare `sqrt` lookup; consumed by the paired `"()"` call.
struct SqrtFn;
/// Marker pushed for a bare `floor` lookup; consumed by the paired `"()"` call.
struct FloorFn;
/// Marker pushed for a bare `ceil` lookup; consumed by the paired `"()"` call.
struct CeilFn;
/// Marker pushed for a bare `trunc` lookup; consumed by the paired `"()"` call.
struct TruncFn;

/// `signum(x)` (signed integers and floats), `sqrt(x)`/`floor(x)`/`ceil(x)`/`trunc(x)`
/// (floats only) — all infallible, matching the semantics of Rust's method of the same
/// name (e.g. `sqrt` of a negative float yields `NaN`, not an error).
pub(crate) fn unary_math_scope(
    name: &str,
    segment: &mut DynSegment,
    num_operands: usize,
    _span: SourceSpan,
) -> Result<bool> {
    match (name, num_operands) {
        ("signum", 0) => {
            segment.op0(|| SignumFn);
            Ok(true)
        }
        ("sqrt", 0) => {
            segment.op0(|| SqrtFn);
            Ok(true)
        }
        ("floor", 0) => {
            segment.op0(|| FloorFn);
            Ok(true)
        }
        ("ceil", 0) => {
            segment.op0(|| CeilFn);
            Ok(true)
        }
        ("trunc", 0) => {
            segment.op0(|| TruncFn);
            Ok(true)
        }
        ("()", 2) => {
            let top = segment.peek_stack_infos(2);
            if top.len() != 2 {
                return Ok(false);
            }
            let callee_type = top[0].type_id;
            let operand_type = top[1].type_id;

            macro_rules! dispatch {
                ($marker:ty, $method:ident, [$($t:ty),+ $(,)?]) => {
                    if callee_type == TypeId::of::<$marker>() {
                        $(
                            if operand_type == TypeId::of::<$t>() {
                                segment.op2(|_callee: $marker, x: $t| x.$method())?;
                                return Ok(true);
                            }
                        )+
                        return Ok(false);
                    }
                };
            }

            dispatch!(SignumFn, signum, [i8, i16, i32, i64, i128, isize, f32, f64]);
            dispatch!(SqrtFn, sqrt, [f32, f64]);
            dispatch!(FloorFn, floor, [f32, f64]);
            dispatch!(CeilFn, ceil, [f32, f64]);
            dispatch!(TruncFn, trunc, [f32, f64]);
            Ok(false)
        }
        _ => Ok(false),
    }
}
```

In `cel-std/src/lib.rs`, register it — this completes `install`:

```rust
/// Registers every CEL standard-library function on `lookup`: `min`, `max`, `clamp`,
/// `abs`, `signum`, `sqrt`, `floor`, `ceil`, `trunc`.
pub fn install(lookup: &mut cel_parser::OpLookup) {
    lookup.push_scope(math::min_max_scope);
    lookup.push_scope(math::clamp_scope);
    lookup.push_scope(math::abs_scope);
    lookup.push_scope(math::unary_math_scope);
}
```

(This also replaces the earlier one-line doc comment on `install` from Task 2 with the
full list now that every function is registered.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --package cel-std -- signum sqrt floor ceil trunc`
Expected: all 7 new tests PASS.

- [ ] **Step 5: Run the full `cel-std` suite, lint, and format**

Run: `cargo test --package cel-std && cargo clippy --package cel-std --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: all pass (21 tests total across Tasks 2-5), no warnings, no formatting diff.

- [ ] **Step 6: Commit**

```bash
git add cel-std/src/math.rs cel-std/src/lib.rs
git commit -m "feat(cel-std): add signum, sqrt, floor, ceil, trunc"
```

---

## Task 6: `begin` integration

**Files:**
- Modify: `begin/Cargo.toml` (add `cel-std` dependency)
- Modify: `begin/src/example_source.rs` (add `op_lookup()` helper; use it at 3 call sites)
- Modify: `begin/examples/inequality.adm2` (rewrite to use `min`/`max`)

**Interfaces:**
- Consumes: `cel_std::install` (from Tasks 2-5).
- Produces: nothing new for other tasks to consume — this is the plan's final task.

- [ ] **Step 1: Add the `cel-std` dependency**

In `begin/Cargo.toml`, add a line after the existing `cel-parser = { path = "../cel-parser" }`
(currently line 24):

```toml
cel-std = { path = "../cel-std" }
```

- [ ] **Step 2: Add the `op_lookup()` helper and use it at all three call sites**

In `begin/src/example_source.rs`, add this function immediately before `pub fn
build_sheet` (currently at line 162):

```rust
/// Builds an [`cel_parser::OpLookup`] with the CEL standard library installed, so every
/// adam-lang source `begin` parses — bundled examples and test sources alike — has the
/// same function set (`min`, `max`, `clamp`, etc.) available.
fn op_lookup() -> cel_parser::OpLookup {
    let mut lookup = cel_parser::OpLookup::new();
    cel_std::install(&mut lookup);
    lookup
}
```

Then replace `cel_parser::OpLookup::new()` with `op_lookup()` at all three existing call
sites: inside `build_sheet` (currently line 163), and inside the two tests
`image_resize_constrain_is_relevant_despite_only_being_a_conditional_expression_input`
(currently line 335) and
`image_resize_relevance_does_not_depend_on_which_cell_currently_holds_strength` (currently
line 356). All three currently read:

```rust
let mut parser = AdamParser::new(TypeRegistry::new(), cel_parser::OpLookup::new());
```

Change each to:

```rust
let mut parser = AdamParser::new(TypeRegistry::new(), op_lookup());
```

(The two test call sites are inside `mod tests`, which already has `use super::*;` at the
top of the module, so `op_lookup` is in scope without an additional import.)

- [ ] **Step 3: Rewrite `begin/examples/inequality.adm2` to use `min`/`max`**

Replace the full contents of `begin/examples/inequality.adm2` with:

```text
sheet inequality {
    cell a = 0.0;
    cell b = 0.0;
    cell c = 2.0;

    relationship {
        method [a, b] -> [a] { min(a, b) }
        method [a, b] -> [b] { max(a, b) }
    }
    relationship {
        method [b, c] -> [b] { min(b, c) }
        method [b, c] -> [c] { max(b, c) }
    }
}
```

- [ ] **Step 4: Run `begin`'s tests**

Run: `cargo test --package begin --no-default-features`
Expected: all pass, including `every_bundled_example_parses_successfully` (which now
exercises `min`/`max` via the rewritten `inequality.adm2`) and the other `build_sheet`
tests. `--no-default-features` avoids pulling in desktop-only dependencies not needed to
verify this change; run the default-feature build too in Step 5.

- [ ] **Step 5: Run the full verification suite**

Run, in order:

```bash
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```

Expected: every command succeeds with zero warnings and zero formatting diffs (per
`CLAUDE.md`'s pre-PR checklist).

- [ ] **Step 6: Commit**

```bash
git add begin/Cargo.toml begin/src/example_source.rs begin/examples/inequality.adm2
git commit -m "feat(begin): wire in cel-std; rewrite inequality example to use min/max"
```

---

## Self-Review Notes

- **Spec coverage:** every row of the spec's function table (Task 2-5), the `op3r` addition
  (Task 1), the `begin` integration helper (Task 6 Step 2), and the `inequality.adm2`
  rewrite (Task 6 Step 3) each map to a task. The spec's "Non-goals" (no new value type, no
  `cel-parser` grammar changes, no `cel-rs` facade re-export) are respected — no task
  touches `cel-rs` or `cel-parser`'s grammar.
- **Type consistency:** `install(lookup: &mut cel_parser::OpLookup)` keeps the same
  signature from Task 2 through Task 5 (only its body and doc comment grow); `op_lookup()`
  in `begin` (Task 6) matches that signature exactly (`&mut` receiver, `cel_std::install`
  called once). All four scope functions share the exact `(name: &str, segment: &mut
  DynSegment, num_operands: usize, _span: SourceSpan) -> Result<bool>` signature required
  by `OpLookup::push_scope`.
- **No placeholders:** every step above contains complete, concrete code — no `TBD` or
  "add appropriate handling" language.
