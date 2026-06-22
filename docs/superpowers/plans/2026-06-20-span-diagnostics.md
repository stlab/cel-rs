# Span Diagnostics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add opt-in span information to CEL runtime errors so that `segment.call0()` failures can be formatted with rustc-style source location diagnostics.

**Architecture:** A `span-diagnostics` cargo feature in `cel-parser` gates closure wrapping at op-insertion time. `OpFn` and `ScopeFn` both gain a `SourceSpan` parameter so `lookup()` can pass the expression span into every op. Runtime errors carry `SpanContext` as anyhow context; a `FormatRustcStyle` extension trait on `anyhow::Error` provides the formatting entry point. `cel-runtime` is unchanged.

**Tech Stack:** Rust 2024 edition, anyhow 1.0, annotate-snippets 0.12, proc-macro2 1.0.

## Global Constraints

- Feature flag name: `span-diagnostics`, declared only in `cel-parser/Cargo.toml`
- `cel-runtime` (DynSegment, RawSegment, Segment, RawSegment) must not be modified
- `SpanContext`, `FormatRustcStyle` always compile (no cfg on the types)
- `ScopeFn`'s `SourceSpan` parameter is unconditional — no cfg on the signature
- `format_rustc_style` signature on the trait matches `CELError::format_rustc_style` exactly: `(&self, source_code: &str, filename: &str, start_line: u32, renderer: &Renderer) -> String`
- All existing tests must pass before and after each task
- Run `cargo fmt --all` before every commit
- Run `cargo clippy --workspace -- -D warnings` before every commit
- Run `cargo test --workspace` before every commit

---

## File Map

| File | Change |
| ---- | ------ |
| `cel-parser/Cargo.toml` | Add `span-diagnostics` feature |
| `cel-parser/src/error.rs` | Add `SpanContext`, `FormatRustcStyle` trait + impl |
| `cel-parser/src/lib.rs` | Re-export `SpanContext`, `FormatRustcStyle` |
| `cel-parser/src/op_table.rs` | Update `OpFn`, `ScopeFn`, `push_scope`, `BuiltinScope::lookup`, `OpLookup::lookup`, `shl_push!`, `shr_push!`, `sig!` closures, add `span_err` helper |

---

### Task 1: Add `span-diagnostics` feature flag

**Files:**
- Modify: `cel-parser/Cargo.toml`

**Interfaces:**
- Produces: `cfg(feature = "span-diagnostics")` usable in `cel-parser`

- [ ] **Step 1: Add the feature**

In `cel-parser/Cargo.toml`, add after the `[dependencies]` block:

```toml
[features]
span-diagnostics = []
```

- [ ] **Step 2: Verify build succeeds with and without the feature**

```
cargo build -p cel-parser
cargo build -p cel-parser --features span-diagnostics
```

Expected: both succeed with no errors.

- [ ] **Step 3: Commit**

```
git add cel-parser/Cargo.toml
git commit -m "feat(cel-parser): add span-diagnostics feature flag"
```

---

### Task 2: Add `SpanContext` type

**Files:**
- Modify: `cel-parser/src/error.rs`
- Modify: `cel-parser/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct SpanContext { span: SourceSpan }` — `Send + Sync`, implements `Display + Error`
  - `SpanContext::new(span: SourceSpan) -> SpanContext`
  - `SpanContext::span(&self) -> SourceSpan`
  - `SpanContext::format_rustc_style(&self, message: &str, source_code: &str, filename: &str, start_line: u32, renderer: &Renderer) -> String`
  - Re-exported as `cel_parser::SpanContext`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)]` block at the bottom of `cel-parser/src/error.rs`:

```rust
#[test]
fn span_context_display_shows_span_location() {
    let span = SourceSpan::new(1, 0, 1, 5);
    let ctx = SpanContext::new(span);
    let s = ctx.to_string();
    assert!(!s.is_empty());
}

#[test]
fn span_context_span_roundtrip() {
    let span = SourceSpan::new(2, 3, 2, 7);
    let ctx = SpanContext::new(span);
    assert_eq!(ctx.span(), span);
}

#[test]
fn span_context_format_rustc_style_contains_message_and_location() {
    use annotate_snippets::Renderer;
    let source = "1i32 + 2i32";
    let span = SourceSpan::new(1, 5, 1, 6);  // the "+" token
    let ctx = SpanContext::new(span);
    let output = ctx.format_rustc_style("arithmetic overflow", source, "test.cel", 1, &Renderer::plain());
    assert!(output.contains("arithmetic overflow"), "expected message in output:\n{output}");
    assert!(output.contains("test.cel"), "expected filename in output:\n{output}");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo test -p cel-parser span_context
```

Expected: compile error — `SpanContext` not found.

- [ ] **Step 3: Add `SpanContext` to `cel-parser/src/error.rs`**

Add after the `CELError` impl block (around line 242, before `ParseError`):

```rust
/// A runtime error context carrying the source span of the failing operation.
///
/// Add this as anyhow context with `.context(SpanContext::new(span))` when wrapping
/// an op closure. Retrieve it from an `anyhow::Error` with `e.downcast_ref::<SpanContext>()`.
pub struct SpanContext {
    span: SourceSpan,
}

impl SpanContext {
    /// Creates a new span context for the given source region.
    pub fn new(span: SourceSpan) -> Self {
        SpanContext { span }
    }

    /// Returns the source span.
    pub fn span(&self) -> SourceSpan {
        self.span
    }

    /// Formats a runtime error message with rustc-style source annotation.
    ///
    /// Delegates to `CELError::format_rustc_style` using `self.span` and `message`.
    pub fn format_rustc_style(
        &self,
        message: &str,
        source_code: &str,
        filename: &str,
        start_line: u32,
        renderer: &Renderer,
    ) -> String {
        CELError::new(message, self.span).format_rustc_style(source_code, filename, start_line, renderer)
    }
}

impl std::fmt::Display for SpanContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "at {}:{}-{}:{}",
            self.span.start.line,
            self.span.start.column,
            self.span.end.line,
            self.span.end.column
        )
    }
}

impl std::fmt::Debug for SpanContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpanContext").field("span", &self.span).finish()
    }
}

impl std::error::Error for SpanContext {}
```

- [ ] **Step 4: Re-export from `cel-parser/src/lib.rs`**

Change the existing re-export line from:
```rust
pub use error::{CELError, ParseError, SourceSpan};
```
to:
```rust
pub use error::{CELError, ParseError, SourceSpan, SpanContext};
```

- [ ] **Step 5: Run tests to verify they pass**

```
cargo test -p cel-parser span_context
```

Expected: 3 tests pass.

- [ ] **Step 6: Run full test suite**

```
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all
```

Expected: all pass, no warnings.

- [ ] **Step 7: Commit**

```
git add cel-parser/src/error.rs cel-parser/src/lib.rs
git commit -m "feat(cel-parser): add SpanContext type for runtime error span annotation"
```

---

### Task 3: Add `FormatRustcStyle` extension trait

**Files:**
- Modify: `cel-parser/src/error.rs`
- Modify: `cel-parser/src/lib.rs`

**Interfaces:**
- Consumes: `SpanContext::new`, `SpanContext::format_rustc_style` (from Task 2)
- Produces:
  - `pub trait FormatRustcStyle` with `fn format_rustc_style(&self, source_code: &str, filename: &str, start_line: u32, renderer: &Renderer) -> String`
  - `impl FormatRustcStyle for anyhow::Error`
  - Re-exported as `cel_parser::FormatRustcStyle`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)]` block in `cel-parser/src/error.rs`:

```rust
#[test]
fn format_rustc_style_with_span_context_uses_span() {
    use annotate_snippets::Renderer;
    let source = "1i32 + 2i32";
    let span = SourceSpan::new(1, 5, 1, 6);
    let inner = anyhow::anyhow!("arithmetic overflow");
    let wrapped = inner.context(SpanContext::new(span));
    let output = FormatRustcStyle::format_rustc_style(
        &wrapped, source, "test.cel", 1, &Renderer::plain(),
    );
    assert!(output.contains("arithmetic overflow"), "expected message:\n{output}");
    assert!(output.contains("test.cel"), "expected filename:\n{output}");
}

#[test]
fn format_rustc_style_without_span_context_falls_back_to_to_string() {
    use annotate_snippets::Renderer;
    let err = anyhow::anyhow!("something went wrong");
    let output = FormatRustcStyle::format_rustc_style(
        &err, "unused source", "unused.cel", 1, &Renderer::plain(),
    );
    assert_eq!(output, "something went wrong");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo test -p cel-parser format_rustc_style_with_span
cargo test -p cel-parser format_rustc_style_without_span
```

Expected: compile error — `FormatRustcStyle` not found.

- [ ] **Step 3: Add the trait and impl to `cel-parser/src/error.rs`**

Add after the `SpanContext` impl block:

```rust
/// Extension trait that adds rustc-style formatting to `anyhow::Error`.
///
/// Import this trait to call `.format_rustc_style(...)` directly on an `anyhow::Error`.
/// If the error carries a [`SpanContext`] (added during op execution with the
/// `span-diagnostics` feature), the output includes a source-location annotation.
/// Otherwise it falls back to `self.to_string()`.
///
/// # Examples
///
/// ```rust
/// use annotate_snippets::Renderer;
/// use cel_parser::FormatRustcStyle;
///
/// let err = anyhow::anyhow!("something went wrong");
/// let output = err.format_rustc_style("1 + 2", "example.cel", 1, &Renderer::plain());
/// assert_eq!(output, "something went wrong");
/// ```
pub trait FormatRustcStyle {
    /// Formats in rustc diagnostic style.
    ///
    /// If the error carries a [`SpanContext`], produces a multi-line caret diagnostic.
    /// Otherwise returns `self.to_string()`.
    fn format_rustc_style(
        &self,
        source_code: &str,
        filename: &str,
        start_line: u32,
        renderer: &Renderer,
    ) -> String;
}

impl FormatRustcStyle for anyhow::Error {
    fn format_rustc_style(
        &self,
        source_code: &str,
        filename: &str,
        start_line: u32,
        renderer: &Renderer,
    ) -> String {
        if let Some(ctx) = self.downcast_ref::<SpanContext>() {
            ctx.format_rustc_style(&self.to_string(), source_code, filename, start_line, renderer)
        } else {
            self.to_string()
        }
    }
}
```

- [ ] **Step 4: Re-export from `cel-parser/src/lib.rs`**

Change the re-export line to:
```rust
pub use error::{CELError, FormatRustcStyle, ParseError, SourceSpan, SpanContext};
```

- [ ] **Step 5: Run tests to verify they pass**

```
cargo test -p cel-parser format_rustc_style
```

Expected: both new tests pass.

- [ ] **Step 6: Run full test suite**

```
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all
```

Expected: all pass.

- [ ] **Step 7: Commit**

```
git add cel-parser/src/error.rs cel-parser/src/lib.rs
git commit -m "feat(cel-parser): add FormatRustcStyle extension trait for anyhow::Error"
```

---

### Task 4: Update `OpFn`, `ScopeFn`, and call sites

This task wires `SourceSpan` through the op-dispatch chain. No span wrapping yet — just updated signatures and passing the span around. All existing tests must still pass after this task.

**Files:**
- Modify: `cel-parser/src/op_table.rs`
- Modify: `cel-parser/src/error.rs` (add `from_proc_macro2_range` to `SourceSpan`)

**Interfaces:**
- Consumes: `SourceSpan` (already in `cel-parser/src/error.rs`)
- Produces:
  - `OpFn = fn(&mut DynSegment, SourceSpan) -> Result<()>`
  - `ScopeFn = Box<dyn Fn(&str, &mut DynSegment, usize, SourceSpan) -> Result<bool> + Send + Sync>`
  - `push_scope` generic bound updated to match new `ScopeFn`
  - `BuiltinScope::lookup` accepts `span: SourceSpan` and passes it to `op_fn`
  - `OpLookup::lookup` passes derived `SourceSpan` to scopes and `BuiltinScope::lookup`

- [ ] **Step 1: Update `OpFn` type alias**

In `cel-parser/src/op_table.rs`, add `use crate::SourceSpan;` at the top of the file (after the existing `use` statements), then change line 38:

```rust
// Before:
pub type OpFn = fn(&mut DynSegment) -> Result<()>;

// After:
pub type OpFn = fn(&mut DynSegment, SourceSpan) -> Result<()>;
```

- [ ] **Step 2: Update `ScopeFn` type alias**

Change line 48:

```rust
// Before:
pub type ScopeFn = Box<dyn Fn(&str, &mut DynSegment, usize) -> Result<bool> + Send + Sync>;

// After:
pub type ScopeFn = Box<dyn Fn(&str, &mut DynSegment, usize, SourceSpan) -> Result<bool> + Send + Sync>;
```

- [ ] **Step 3: Update `push_scope` generic bound**

Change the `push_scope` method:

```rust
// Before:
pub fn push_scope<F>(&mut self, scope: F)
where
    F: Fn(&str, &mut DynSegment, usize) -> Result<bool> + Send + Sync + 'static,

// After:
pub fn push_scope<F>(&mut self, scope: F)
where
    F: Fn(&str, &mut DynSegment, usize, SourceSpan) -> Result<bool> + Send + Sync + 'static,
```

- [ ] **Step 4: Update `BuiltinScope::lookup` to accept and pass span**

Change the `BuiltinScope::lookup` signature and the `op_fn` call:

```rust
// Before:
fn lookup(&self, name: &str, segment: &mut DynSegment, num_operands: usize) -> Result<bool> {
    // ...
    if matches {
        (sig.op_fn)(segment)?;
        return Ok(true);
    }

// After:
fn lookup(&self, name: &str, segment: &mut DynSegment, num_operands: usize, span: SourceSpan) -> Result<bool> {
    // ...
    if matches {
        (sig.op_fn)(segment, span)?;
        return Ok(true);
    }
```

- [ ] **Step 5: Update `OpLookup::lookup` to derive span and pass it everywhere**

In `OpLookup::lookup`, derive a `SourceSpan` from the existing `start`/`end` params and pass it to scope closures and `builtin_scope.lookup`:

```rust
pub fn lookup(
    &self,
    name: &str,
    segment: &mut DynSegment,
    num_operands: usize,
    start: proc_macro2::Span,
    end: proc_macro2::Span,
) -> std::result::Result<(), crate::ParseError> {
    let source_span = SourceSpan::from_proc_macro2_range(start, end);
    for scope in self.scopes.iter().rev() {
        match scope(name, segment, num_operands, source_span) {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(e) => return Err(crate::ParseError::new_range(e.to_string(), start, end)),
        }
    }

    match self.builtin_scope.lookup(name, segment, num_operands, source_span) {
        Ok(true) => return Ok(()),
        // ... rest unchanged
```

Note: `SourceSpan::from_proc_macro2_range` does not yet exist — add it in this step:

In `cel-parser/src/error.rs`, add to `impl SourceSpan`:

```rust
/// Builds a span from two `proc_macro2::Span` values (start of `start`, end of `end`).
///
/// Use this when an expression spans multiple tokens and you want a range span.
pub fn from_proc_macro2_range(start: proc_macro2::Span, end: proc_macro2::Span) -> Self {
    SourceSpan {
        start: start.start(),
        end: end.end(),
    }
}
```

- [ ] **Step 6: Update all `sig!` closures to add `_span` parameter**

Every closure in a `sig!` or `sig_het!` invocation currently has the form `|seg| ...`. Change ALL of them to `|seg, _span| ...`. There are approximately 130 such closures in the static signature arrays.

The pattern is mechanical: `|seg|` → `|seg, _span|`.

Examples showing before and after for a representative sample:

```rust
// Infallible binary op (before):
sig!(TYPE_U32, 2, |seg| seg.op2(|a: u32, b: u32| a.wrapping_add(b)))

// After:
sig!(TYPE_U32, 2, |seg, _span| seg.op2(|a: u32, b: u32| a.wrapping_add(b)))

// Fallible binary op (before):
sig!(TYPE_I32, 2, |seg| seg.op2r(|a: i32, b: i32| a
    .checked_add(b)
    .ok_or_else(|| anyhow!("arithmetic overflow"))))

// After (span captured but wrapping comes in Task 5):
sig!(TYPE_I32, 2, |seg, _span| seg.op2r(|a: i32, b: i32| a
    .checked_add(b)
    .ok_or_else(|| anyhow!("arithmetic overflow"))))
```

For the `shl_push!` and `shr_push!` macros, update every `sig_het!` closure body from `|seg|` to `|seg, _span|`. These are inside the macro bodies at the top of the file, not individual call sites — update the macro definitions themselves.

- [ ] **Step 7: Verify the project compiles**

```
cargo build --workspace
```

Expected: compiles with no errors. There will likely be compile errors from tests that use `push_scope` with 3-argument closures — fix those now by updating the test closures from `|name, seg, n|` to `|name, seg, n, _span|`. Search for all usages:

```
cargo test --workspace 2>&1 | head -50
```

Fix every failing test closure in `cel-parser/src/op_table.rs` tests.

- [ ] **Step 8: Run full test suite**

```
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all
```

Expected: all pass.

- [ ] **Step 9: Commit**

```
git add cel-parser/src/error.rs cel-parser/src/op_table.rs
git commit -m "feat(cel-parser): thread SourceSpan through OpFn, ScopeFn, and lookup"
```

---

### Task 5: Add `span_err` helper and wire span wrapping into fallible ops

This task adds the actual feature-gated `.context(SpanContext { span })` wrapping to all fallible builtin ops. Infallible ops (`op2`, `op1`) are untouched.

**Files:**
- Modify: `cel-parser/src/op_table.rs`

**Interfaces:**
- Consumes: `SpanContext::new` (Task 2), `SourceSpan` parameter now on `OpFn`/`ScopeFn` (Task 4)
- Produces: runtime errors from fallible builtin ops carry `SpanContext` when `span-diagnostics` is enabled

- [ ] **Step 1: Add `span_err` helper function**

Add near the top of `cel-parser/src/op_table.rs`, after the `use` declarations:

```rust
/// Wraps a runtime error with span context when the `span-diagnostics` feature is enabled.
///
/// When the feature is off this is a no-op and compiles to nothing.
#[cfg(feature = "span-diagnostics")]
#[inline]
fn span_err(span: SourceSpan, e: anyhow::Error) -> anyhow::Error {
    e.context(crate::SpanContext::new(span))
}

#[cfg(not(feature = "span-diagnostics"))]
#[inline]
fn span_err(_span: SourceSpan, e: anyhow::Error) -> anyhow::Error {
    e
}
```

- [ ] **Step 2: Write feature-gated integration test**

Add to the `#[cfg(test)]` block in `cel-parser/src/op_table.rs`:

```rust
#[cfg(feature = "span-diagnostics")]
#[test]
fn runtime_error_carries_span_context() {
    use crate::{CELParser, SpanContext};

    let mut parser = CELParser::new(OpLookup::new());
    let source = "1i32 + 2147483647i32";  // i32::MAX + 1 → overflow
    let segment = parser.parse_str(source).expect("should parse");
    let err = segment.call0::<i32>().expect_err("should overflow");
    let ctx = err.downcast_ref::<SpanContext>()
        .expect("expected SpanContext on runtime error");
    // span should cover the "+" operator region
    assert!(ctx.span().start.line >= 1);
}
```

- [ ] **Step 3: Run the test to verify it fails (span not attached yet)**

```
cargo test -p cel-parser --features span-diagnostics runtime_error_carries_span_context
```

Expected: FAIL — `SpanContext` not found in error chain (downcast returns `None`).

- [ ] **Step 4: Update fallible `sig!` closures to use `span_err`**

Change every fallible builtin op closure from `|seg, _span|` to `|seg, span|` and add `.map_err(|e| span_err(span, e))` at the end of the result expression.

The fallible ops are those using `op2r` or `op1r`. Here is the complete list of patterns to update — apply this transform to every matching `sig!` closure:

**Arithmetic overflow (signed add, sub, mul — for i8/i16/i32/i64/i128/isize):**

```rust
// Before:
sig!(TYPE_I32, 2, |seg, _span| seg.op2r(|a: i32, b: i32| a
    .checked_add(b)
    .ok_or_else(|| anyhow!("arithmetic overflow"))))

// After:
sig!(TYPE_I32, 2, |seg, span| seg.op2r(move |a: i32, b: i32| a
    .checked_add(b)
    .ok_or_else(|| anyhow!("arithmetic overflow"))
    .map_err(|e| span_err(span, e))))
```

Apply this same pattern to every i8/i16/i32/i64/i128/isize variant of `+`, `-`, `*`.

**Division and modulo (signed and unsigned):**

```rust
// Before:
sig!(TYPE_I32, 2, |seg, _span| seg.op2r(|a: i32, b: i32| {
    if b == 0 { Err(anyhow!("division by zero")) } else { Ok(a / b) }
}))

// After — .map_err chains on the if-expression inside the block:
sig!(TYPE_I32, 2, |seg, span| seg.op2r(move |a: i32, b: i32| {
    if b == 0 { Err(anyhow!("division by zero")) } else { Ok(a / b) }
        .map_err(|e| span_err(span, e))
}))
```

- [ ] **Step 5: Update `shl_push!` and `shr_push!` macros**

In the `shl_push!` macro body, change every `sig_het!` entry from:

```rust
$v.push(sig_het!($lhs_idx, TYPE_U8, |seg| seg.op2r(
    |a: $lhs_ty, b: u8| a
        .checked_shl(u32::from(b))
        .ok_or_else(|| anyhow!("shift overflow"))
)));
```

to:

```rust
$v.push(sig_het!($lhs_idx, TYPE_U8, |seg, span| seg.op2r(
    move |a: $lhs_ty, b: u8| a
        .checked_shl(u32::from(b))
        .ok_or_else(|| anyhow!("shift overflow"))
        .map_err(|e| span_err(span, e))
)));
```

Apply the same pattern to every entry in `shl_push!` and `shr_push!` (12 entries each × 2 macros = 24 changes).

- [ ] **Step 6: Run the integration test to verify it passes**

```
cargo test -p cel-parser --features span-diagnostics runtime_error_carries_span_context
```

Expected: PASS — `SpanContext` is found in the error chain.

- [ ] **Step 7: Verify the feature-off path still works**

```
cargo test -p cel-parser
```

Expected: all tests pass, including existing arithmetic tests that trigger runtime errors.

- [ ] **Step 8: Run full test suite both ways**

```
cargo test --workspace
cargo test --workspace --features cel-parser/span-diagnostics
cargo clippy --workspace -- -D warnings
cargo clippy --workspace --features cel-parser/span-diagnostics -- -D warnings
cargo fmt --all
```

Expected: all pass.

- [ ] **Step 9: Commit**

```
git add cel-parser/src/op_table.rs
git commit -m "feat(cel-parser): wrap fallible builtin op errors with SpanContext under span-diagnostics"
```

---

### Task 6: Tests for `ScopeFn` span forwarding, fallback, and client-added ops

**Files:**
- Modify: `cel-parser/src/op_table.rs` (tests section)
- Modify: `cel-parser/src/error.rs` (tests section)

**Interfaces:**
- Consumes: all prior tasks

- [ ] **Step 1: Add custom-scope span test (feature-gated)**

Add to the `#[cfg(test)]` block in `cel-parser/src/op_table.rs`:

```rust
#[cfg(feature = "span-diagnostics")]
#[test]
fn custom_scope_can_forward_span_to_ops() {
    use crate::{CELParser, SpanContext};

    let mut lookup = OpLookup::new();
    lookup.push_scope(|_name, seg, _n, span| {
        // Custom scope wraps its failing op with span context
        seg.op1r(move |a: i32| {
            if a < 0 {
                Err(anyhow::anyhow!("negative value").context(SpanContext::new(span)))
            } else {
                Ok(a * 2)
            }
        });
        Ok(true)
    });

    let mut parser = CELParser::new(lookup);
    // Build a segment that has a custom op on the stack — use just() + the custom op
    let mut seg = cel_runtime::DynSegment::new::<()>();
    seg.just(-1i32);
    // Directly call the registered scope by name through lookup
    let span = proc_macro2::Span::call_site();
    parser.op_lookup.lookup("custom", &mut seg, 1, span, span).ok();
    // (This tests that the span forwarding path compiles and the closure works)
}
```

Note: If `CELParser` doesn't expose `op_lookup` publicly, simplify the test by registering via `push_scope` and verifying the `ScopeFn` closure compiles and receives the span — a compile test is sufficient.

Adjust the test based on actual `CELParser` API. A minimal compile test:

```rust
#[cfg(feature = "span-diagnostics")]
#[test]
fn scope_fn_accepts_source_span_parameter() {
    // Verifies that ScopeFn closures compile with the SourceSpan parameter.
    let mut lookup = OpLookup::new();
    lookup.push_scope(|_name: &str, _seg: &mut cel_runtime::DynSegment, _n: usize, span: crate::SourceSpan| {
        // span is available for use
        let _ = span;
        Ok(false)
    });
    // If this compiles, the ScopeFn signature is correct.
}
```

- [ ] **Step 2: Add fallback test for client-added ops**

Add to `cel-parser/src/op_table.rs` tests (not feature-gated):

```rust
#[test]
fn format_rustc_style_falls_back_for_client_added_op_error() {
    use crate::FormatRustcStyle;
    use annotate_snippets::Renderer;

    // Simulate an error from a client op with no SpanContext
    let err = anyhow::anyhow!("custom domain error");
    let output = err.format_rustc_style("unused source", "unused.cel", 1, &Renderer::plain());
    assert_eq!(output, "custom domain error");
}
```

- [ ] **Step 3: Run new tests**

```
cargo test -p cel-parser scope_fn_accepts
cargo test -p cel-parser format_rustc_style_falls_back
cargo test -p cel-parser --features span-diagnostics scope_fn_accepts
```

Expected: all pass.

- [ ] **Step 4: Run full test suite**

```
cargo test --workspace
cargo test --workspace --features cel-parser/span-diagnostics
cargo clippy --workspace -- -D warnings
cargo fmt --all
```

Expected: all pass.

- [ ] **Step 5: Commit**

```
git add cel-parser/src/op_table.rs cel-parser/src/error.rs
git commit -m "test(cel-parser): add span-diagnostics coverage for custom scopes and fallback path"
```

---

## Self-Review Checklist (run before declaring done)

- [ ] `SpanContext` compiles without `span-diagnostics` feature
- [ ] `FormatRustcStyle` compiles without `span-diagnostics` feature
- [ ] Runtime overflow error carries `SpanContext` with `--features span-diagnostics`
- [ ] `call0()` still returns `anyhow::Result<R>` — signature unchanged
- [ ] All existing tests pass without feature flag
- [ ] All tests pass with `--features cel-parser/span-diagnostics`
- [ ] `cargo clippy` clean both ways
- [ ] `cargo doc --lib -p cel-parser --no-deps` shows `SpanContext` and `FormatRustcStyle`
