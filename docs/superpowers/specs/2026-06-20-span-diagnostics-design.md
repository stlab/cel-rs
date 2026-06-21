# Span Diagnostics for Runtime Errors

**Date:** 2026-06-20
**Branch:** `sean-parent/span-diagnostics`

## Summary

Add optional span information to runtime errors so that a failed `segment.call0()` (or similar) can be formatted with rustc-style source location diagnostics. The feature is opt-in via a cargo feature flag in `cel-parser`.

## Section 1 — New types: `SpanContext` and `FormatRustcStyle`

### `SpanContext`

A new public type in `cel-parser`:

```rust
pub struct SpanContext {
    span: SourceSpan,
}
```

- `Display` impl: renders minimal span location info
- `std::error::Error` impl: no source (the wrapped op error is the layer below in the anyhow chain)
- An internal `format_rustc_style(&self, source_text: &str, message: &str) -> String` helper: formats with source location using `annotate-snippets`. The parameters and formatting options match `CELError::format_rustc_style` exactly — confirmed during implementation.

`SpanContext` is always compiled — no feature gate on the type itself.

### `FormatRustcStyle` trait

An extension trait on `anyhow::Error`, also in `cel-parser`:

```rust
pub trait FormatRustcStyle {
    fn format_rustc_style(&self, source_text: &str) -> String;
}

impl FormatRustcStyle for anyhow::Error {
    fn format_rustc_style(&self, source_text: &str) -> String {
        if let Some(ctx) = self.downcast_ref::<SpanContext>() {
            ctx.format_rustc_style(source_text, &self.to_string())
        } else {
            self.to_string()
        }
    }
}
```

The downcast is attempted unconditionally — no feature flag inside the method. If it succeeds, the error is formatted with span; if not, it falls back to the plain error message. This handles:

- Feature disabled (no `SpanContext` ever added)
- Ops added outside the parser by other clients
- Stack arity errors from `call0` itself

Call site:

```rust
use cel_parser::FormatRustcStyle;

if let Err(e) = segment.call0::<u32>() {
    println!("{}", e.format_rustc_style(source_text));
}
```

## Section 2 — Span flow through `lookup()`

### Crate constraint

`cel-parser` depends on `cel-runtime`; `cel-runtime` does not depend on `cel-parser`. Therefore all span-related logic stays in `cel-parser`. `DynSegment` and `RawSegment` are unchanged.

### `ScopeFn` signature change

`ScopeFn` (defined in `cel-parser`) gains an unconditional `SourceSpan` parameter:

```rust
type ScopeFn = Box<dyn Fn(&str, &mut DynSegment, usize, SourceSpan) -> Result<bool>>;
```

This is not feature-gated — `SourceSpan` is already in `cel-parser`. Existing custom scopes that don't need span info ignore the parameter.

### Wrapping in `lookup()`

`lookup()` already receives `start: proc_macro2::Span` and `end: proc_macro2::Span`. It:

1. Derives a `SourceSpan` from them
2. Passes it to each `ScopeFn` invocation
3. For the builtin scope path, wraps op closures inline (feature-gated):

```rust
#[cfg(feature = "span-diagnostics")]
let f = {
    let inner = f;
    move |...| inner(...).map_err(|e| e.context(SpanContext { span }))
};
segment.op_method(f);
```

Custom `ScopeFn` closures receive the span and can apply the same wrapping pattern to their own op additions.

### `join2` / `new_fragment`

Parser code building conditional fragments (e.g. `is_if_expression`) may need minor cleanup to pass spans through explicitly rather than relying on error construction at the `lookup()` level. Runtime errors from ops inside fragments already carry `SpanContext` (added when those ops were inserted via `lookup()`), so they format correctly when errors propagate out of `join2`'s runtime dispatch.

## Section 3 — Feature flag

- **Name:** `span-diagnostics`
- **Location:** `cel-parser/Cargo.toml` only
- **Gates:** the closure wrapping in `lookup()` and the builtin scope path
- `SpanContext`, `FormatRustcStyle`, and the `anyhow::Error` impl are always compiled
- `ScopeFn`'s `SourceSpan` parameter is unconditional
- `cel-runtime` gains no new dependencies or feature flags

## Section 4 — Error handling / fallback

`format_rustc_style` on `anyhow::Error`:

| Condition | Behavior |
| --------- | -------- |
| `SpanContext` in chain (feature on, op from parser) | Formats with source location annotation |
| No `SpanContext` (feature off) | Falls back to `self.to_string()` |
| No `SpanContext` (op added outside parser) | Falls back to `self.to_string()` |
| No `SpanContext` (arity error from `call0` itself) | Falls back to `self.to_string()` |

The downcast result alone determines the path — no feature-flag branching inside the method.

## Section 5 — Testing

- **`SpanContext` + `FormatRustcStyle`:** Unit tests in `cel-parser` — construct a `SpanContext` with a known span, wrap an `anyhow::Error` with `.context(SpanContext { span })`, call `format_rustc_style(source_text)`, assert the output contains the expected line/column annotation.
- **Fallback path:** `format_rustc_style` on a plain `anyhow::Error` (no `SpanContext`) returns `self.to_string()`.
- **`lookup()` wrapping (feature-gated):** Integration test that builds a segment with a failing op via the parser, calls `call0()`, and asserts the error downcasts to `SpanContext` with the correct span — compiled only under `#[cfg(feature = "span-diagnostics")]`.
- **Client-added ops:** A test appends an op outside the parser after parsing, triggers a failure in that op, and asserts `format_rustc_style` falls back gracefully to the plain message.
- **`ScopeFn` with span:** Tests for custom scopes that use the span parameter to wrap their own ops, verifying span info propagates correctly.
