# CEL Standard Library Design

**Date:** 2026-08-21
**Status:** Approved

## Goal

Add a `cel-std` crate providing a small set of CEL-callable functions backed by Rust's
standard library — `min`, `max`, `clamp`, and a handful of related numeric functions —
registered entirely through `cel-parser`'s existing public extension mechanism
(`OpLookup::push_scope`), with no changes to `cel-parser`'s core grammar or built-in
operator table. `begin` wires it in so adam-lang sources (and `begin`'s own examples) can
call these functions.

## Background

`docs/VISION.md` already shows the target usage — plain scalar `min`/`max` in a
view-constraint sheet:

```text
cell fit_scale = min(viewport.width / content_bounds.width(), viewport.height / content_bounds.height());
cell max_scale = max(fit_scale, max_zoom);
```

`cel-parser`'s built-in `round` function demonstrates the pattern a library-defined
function must follow: a CEL function call parses as two independent operator lookups — an
arity-0 lookup for the bare callee name, then an arity-`N+1` lookup for `"()"` with the
callee and its arguments already on the stack (`cel-parser/src/op_table.rs:1345-1390`).
`round` registers a scope handling both halves: a marker struct (`RoundFn`) is pushed for
the arity-0 case, and consumed by the `"()"` case once it confirms (via
`peek_stack_infos`) that the callee on the stack really is that marker.

`OpLookup::push_scope` is already public API, exercised in `cel-parser`'s own doctests, so
a function library needs no changes to `cel-parser` itself — only to compose scopes the
same way `round` does, from outside the crate.

`docs/VISION.md` separately lists geometry/affine-transform types (`Point`/`Size`/`Rect`/
`TranslateScale`, likely via the `kurbo` crate) as a future direction, explicitly gated on
method-call syntax landing in `cel-parser` first. That is out of scope here: this crate is
limited to what's expressible today with free-function-call syntax over the existing
scalar types (the 14 numeric types, `bool`, `String`).

## Design

### New crate: `cel-std`

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

Added to the root `Cargo.toml` workspace `members` list.

Public surface — one function:

```rust
/// Registers every CEL standard-library function on `lookup` — `min`, `max`, `clamp`,
/// `abs`, `signum`, `sqrt`, `floor`, `ceil`, `trunc`.
pub fn install(lookup: &mut cel_parser::OpLookup);
```

`install` calls `lookup.push_scope(...)` one or more times (grouped by function category —
e.g. one scope for `min`/`max`/`clamp`, one for the unary math functions — mirroring how
`cel-parser` itself composes `round_scope`). Each scope follows the `round`/`RoundFn`
idiom exactly: a marker struct per function name, pushed on the arity-0 lookup of that
name and consumed by the paired `"()"` lookup.

### Function set

All functions dispatch by the operand(s)' `TypeId`, covering the same 14 numeric types as
the existing built-in operators (`u8`/`u16`/`u32`/`u64`/`u128`/`usize`/`i8`/`i16`/`i32`/
`i64`/`i128`/`isize`/`f32`/`f64`), restricted to the types where each operation is
meaningful:

| Function | Types | Semantics | Fallibility |
|---|---|---|---|
| `min(a, b)` | all 14 | `Ord::min` (integers), `f32::min`/`f64::min` (floats — NaN-avoiding: returns the non-NaN operand if exactly one is NaN) | infallible |
| `max(a, b)` | all 14 | `Ord::max` (integers), `f32::max`/`f64::max` (floats) | infallible |
| `clamp(x, lo, hi)` | all 14 | `x` bounded to `[lo, hi]` | `Err("invalid clamp bounds")` if `!(lo <= hi)` |
| `abs(x)` | signed integers + floats | absolute value | integers: `Err("arithmetic overflow")` via `checked_abs` (matches the existing negation/sub overflow convention); floats: infallible |
| `signum(x)` | signed integers + floats | sign (-1/0/1 for integers; ±1.0/NaN for floats, matching Rust's `f64::signum`) | infallible |
| `sqrt(x)` | `f32`, `f64` | square root (negative input yields `NaN`, matching Rust) | infallible |
| `floor(x)`, `ceil(x)`, `trunc(x)` | `f32`, `f64` | as Rust's methods of the same name | infallible |

`clamp`'s bounds check (`lo <= hi`) is written as a single comparison rather than two
(`lo > hi` plus a separate NaN check) because it's also correctly `false` whenever either
bound is `NaN` — no separate NaN branch is needed.

Unsigned types are excluded from `abs`/`signum` (the operation is either the identity or
meaningless); calling `abs`/`signum` on an unsigned operand simply falls through to
`OpLookup`'s existing "no operation ... for types [...]" error, the same failure mode as
calling any undefined function on a mismatched type today.

No new opaque value type (e.g. a `Duration`) is introduced — no concrete consumer for one
was identified, and the extension pattern here (`push_scope` + marker structs) already
demonstrates how a future library-defined type would plug in as free functions, without
needing method-call syntax.

### `cel-runtime`: new `op3r` on `DynSegment`

`clamp` is the first ternary *fallible* operation in the codebase — `op1r`/`op2r` exist,
`op3` exists, but `op3r` does not. Add it, mirroring `op2r` exactly:

```rust
/// Pushes a ternary operation that takes three arguments of types T, U, V and returns a
/// Result<R>.
///
/// If the operation succeeds, the result is pushed onto the stack. If it fails, the stack
/// is unwound to its previous state and the error is propagated.
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
```

Implemented via `get_last_n_padded::<3>()` / `pop_types::<(T, (U, (V, ())))>()` /
`capture_unwind` / `push_type::<R>()`, exactly like `op2r` (`cel-runtime/src/dyn_segment.rs:745-762`)
but for three operands.

This requires a new `raw3` on `RawSegment` (`cel-runtime/src/raw_segment.rs`), mirroring
the existing `raw2` (line 248) / `push_op2r_` (line 230) pair: a `push_op3r_` dispatch
closure parameterized over the three padding bools (8 combinations, matching `push_op3`'s
existing match at line 298), and a public `raw3` that calls it.

This is a generically useful, small addition to the core runtime (not std-library-specific)
— the same shape as the existing `op1r`/`op2r`, just extended to arity 3.

### `begin` integration

`begin/src/example_source.rs` currently constructs `OpLookup::new()` directly at three
call sites (`build_sheet`, and two tests:
`image_resize_constrain_is_relevant_despite_only_being_a_conditional_expression_input`,
`image_resize_relevance_does_not_depend_on_which_cell_currently_holds_strength`). Add one
small helper:

```rust
fn op_lookup() -> cel_parser::OpLookup {
    let mut lookup = cel_parser::OpLookup::new();
    cel_std::install(&mut lookup);
    lookup
}
```

and use it at all three call sites, so every adam-lang source `begin` parses — bundled
examples and ad-hoc test sources alike — has the same function set available. `begin`
depends on `cel-std` directly (it already depends on `cel-parser` directly in this file).

### `begin/examples/inequality.adm2`: rewritten to use `min`/`max`

The existing example hand-rolls a two-cell sort via `if`/`else`:

```text
sheet inequality {
    cell a = 0.0;
    cell b = 0.0;
    cell c = 2.0;

    relationship {
        method [a, b] -> [a] { if a < b { a } else { b } }
        method [a, b] -> [b] { if b < a { a } else { b } }
    }
    relationship {
        method [b, c] -> [b] { if b < c { b } else { c } }
        method [b, c] -> [c] { if c < b { b } else { c } }
    }
}
```

Each pair of methods is exactly a `min`/`max` sort (method `[a,b] -> [a]` computes
`min(a, b)`; method `[a,b] -> [b]` computes `max(a, b)`; likewise for `b`/`c`). Rewritten:

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

This is also the end-to-end proof that the wiring works: `every_bundled_example_parses_successfully`
(`begin/src/example_source.rs`) already iterates every bundled example through `build_sheet`,
so it exercises `min`/`max` through the full `AdamParser` → `cel-std` path once this example
is updated.

## Testing

- **`cel-std`**: unit tests per function, derived from the contract table above — for each
  function, at least one signed-integer, one unsigned-integer (where applicable), and one
  float case, plus: `clamp`'s `Err` path (`lo > hi`) and its `NaN`-bound case; float
  `min`/`max`'s one-NaN-operand behavior; `abs`'s integer overflow `Err` path
  (`i32::MIN`). Tests are derived from the function contracts only, per this project's
  testing convention — not from the macro-generated dispatch internals.
- **`cel-runtime`**: unit tests for `op3r`, mirroring the existing `op1r`/`op2r` tests —
  a success case and an error-unwinds-the-stack case.
- **`begin`**: no new tests beyond the existing `every_bundled_example_parses_successfully`
  and the general `build_sheet` tests, which now transitively cover the `min`/`max`
  rewrite of `inequality.adm2` and the `op_lookup()` helper.

## Non-goals

- No new CEL value types (`Duration` or otherwise).
- No changes to `cel-parser`'s grammar, built-in operator table, or `ScopeFn` signature.
- No geometry/affine-transform types — tracked separately in `docs/VISION.md`, blocked on
  method-call syntax.
- No re-export of `cel-std` from the `cel-rs` facade crate — nothing currently consumes the
  facade crate's re-exports for this purpose; `begin` depends on `cel-std` directly, the
  same way it already depends on `cel-parser` directly.
