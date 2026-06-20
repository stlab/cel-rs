# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Setup

After cloning, activate the shared git hooks (one-time):

```bash
git config core.hooksPath .githooks
```

## Commands

```bash
# Format (required before every commit; enforced by pre-commit hook)
cargo fmt --all

# Build
cargo build --workspace

# Test (all, including doc tests)
cargo test --workspace
cargo test --doc --workspace

# Run a single test
cargo test --workspace <test_name>

# Lint (warnings are errors)
cargo clippy --workspace -- -D warnings
cargo clippy --fix --workspace

# Docs
cargo doc --lib --no-deps --open --workspace
```

Sanitizer runs require nightly and a target triple (e.g. `x86_64-apple-darwin` or `x86_64-unknown-linux-gnu`):
```bash
RUSTFLAGS=-Zsanitizer=address cargo +nightly test -Zbuild-std --target <triple> --lib --workspace
RUSTFLAGS=-Zsanitizer=thread  cargo +nightly test -Zbuild-std --target <triple> --workspace
RUSTFLAGS=-Zsanitizer=leak    cargo +nightly test -Zbuild-std --target <triple> --workspace
```

## Architecture

This is a workspace with three crates:

- **`cel-runtime`** — core runtime; all meaningful code lives here
- **`cel-rs`** — thin façade that re-exports from `cel-runtime`
- **`cel-rs-macros`** — proc-macro crate for compile-time CEL expression validation

### Four-layer stack abstraction (cel-runtime/src/)

The runtime is a **stack-based expression evaluator** built in four layers of increasing type safety:

| Layer | File | Role |
|-------|------|------|
| `RawStack` | `raw_stack.rs` | Byte-aligned unsafe stack; `push<T>` returns padding bool, `pop<T>` requires it |
| `RawSegment` | `raw_segment.rs` | Op list + closure storage + per-op dropper functions |
| `DynSegment` | `dyn_segment.rs` | Runtime type-checking wrapper; maintains `stack_ids: Vec<StackInfo>` |
| `Segment<Args, Stack>` | `segment.rs` | Zero-cost compile-time phantom wrapper; `Args: IntoList`, `Stack: List` |

The compile-time type system uses **cons-cell heterogeneous lists** (`CStackList<H,T>` / `CNil`) defined in `c_stack_list.rs` and `list_traits.rs`.

### Parser pipeline (cel-runtime/src/parser/)

```
&str → TokenStream (proc_macro2) → LexLexer (flatten + combine multi-char ops) → CELParser (recursive descent) → DynSegment
```

`parser/mod.rs` contains the full recursive-descent grammar. Function names mirror grammar productions directly (e.g. `is_additive_expression`).

`parser/op_table.rs` implements `OpLookup`: a stack of custom `ScopeFn` scopes (LIFO) backed by static `phf_map` built-in ops. Overloading is by arity + `TypeId`.

`parser/error.rs` defines `CELError` with `SourceSpan` and `format_rustc_style()` for caret diagnostics.

## Code Style

### Avoid heap allocations
- Pass `&str` / `&[T]` rather than cloning into `String` / `Vec<T>`
- Use generics or `fn` pointers instead of `Box<dyn Trait>` when the type set is statically known
- Return `&[T]` or `impl Iterator` over owned collections when the data already lives elsewhere
- Borrow inside a block to release the borrow before the next mutable use rather than collecting into a `Vec`

### Documentation comments

Every function must have a `///` doc comment written in **contract style**. The contract lives adjacent to the declaration so it stays synchronized with the code.

**Required sections** (include only those that apply):

1. **Summary** — A concise present-tense sentence fragment describing what the function does or returns, ending with a period.
2. **Preconditions** — Non-obvious preconditions not implied by the summary, as `/// - Precondition: <condition>` bullets. Preconditions implied by the summary need not be restated. Violation has unspecified behavior (which may include a panic); do NOT document what happens on violation — instead use `debug_assert!()` to check preconditions in debug builds.
   - `# Errors` — conditions that cause an `Err` return (runtime errors, not precondition violations).
   - `# Safety` — invariants the caller must uphold for `unsafe` functions. This is the one place where the consequence of violation (undefined behavior) must be documented.
3. **Postconditions** — `/// - Postcondition: <condition>` bullet in the body, only when not implicit in the summary.
4. **Complexity** — `/// - Complexity: <description>` bullet, **required whenever the operation is not O(1)**. Default assumption is O(1) time and space.

If you cannot write a simple contract for a function, treat that as a signal that the design needs improvement.

Additional rules:
- For parser functions, the grammar production is the summary: `/// \`additive_expression = multiplicative_expression { ("+" | "-") multiplicative_expression }.\``
- Use `# Examples` for all public APIs.
- Modules use `//!` with a usage tutorial.

**Example:**
```rust
/// Removes and returns the top element.
///
/// - Precondition: `padding` matches the value returned by the corresponding `push`.
///
/// - Complexity: O(1).
pub fn pop<T>(&mut self, padding: bool) -> T
```

### Unit tests

Derive tests from the **contract and public interface only** — do not read or consider the implementation. The test suite verifies observable behavior as specified by the contract:

- Each `# Errors` condition should assert the `Err` variant is returned.
- Each postcondition should be asserted.
- Edge cases implied by the summary (empty input, single element, boundary values) should be covered.

Precondition violations have unspecified behavior and should not be tested. Tests written against the implementation risk encoding bugs rather than verifying intent.

### Fallible ops
Operations that can fail use `.op1r` / `.op2r` variants (returning `Result`) rather than `.op1` / `.op2`. Arithmetic on signed integers must use `checked_*` operations, not wrapping arithmetic.
