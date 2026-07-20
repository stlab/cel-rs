# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

See `docs/VISION.md` for the long-term goals and direction behind each crate in this
workspace — read it when a task touches roadmap, priorities, or "why does this exist"
questions.

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
# --all-targets is required so tests, doctests, and benches are linted too —
# without it, clippy silently skips everything behind #[cfg(test)] and tests/.
# The begin crate is excluded from the workspace commands and checked separately:
# once with --no-default-features (to avoid platform-specific renderer dependencies)
# and once with its default features (desktop) so #[cfg(feature = "desktop")] code —
# the code path the app actually ships — is linted too. Neither of the other two
# invocations covers that code, so skipping this one lets desktop-only warnings
# through unnoticed.
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
cargo clippy --fix --workspace --exclude begin --all-targets

# Docs
cargo doc --lib --no-deps --open --workspace
```

Sanitizer runs require nightly and a target triple (e.g. `x86_64-apple-darwin` or `x86_64-unknown-linux-gnu`):

```bash
RUSTFLAGS=-Zsanitizer=address cargo +nightly test -Zbuild-std --target <triple> --lib --workspace
RUSTFLAGS=-Zsanitizer=thread  cargo +nightly test -Zbuild-std --target <triple> --workspace
RUSTFLAGS=-Zsanitizer=leak    cargo +nightly test -Zbuild-std --target <triple> --workspace
```

## Git Workflow

If a request is made that requires any modification, additions, or deletions to files in the project, stop and suggest the user create a worktree first.

Never commit directly to `main`.

Before creating a PR, run the full check suite locally — every command in the Commands
section above, including all three clippy invocations (workspace, and begin with and
without its default features).

`cargo build --workspace` and `cargo test --workspace` must produce zero compiler
warnings — clippy's `-D warnings` does not catch everything a plain build/test compile
can warn about (e.g. an unused `mut`). Read the build/test output and fix any warnings
before opening the PR.

For any multi-phase or multi-step piece of work (a design doc phased into several sub-plans, a
plan executed across multiple sessions), create or update a dated handoff document under
`docs/superpowers/` (e.g. `docs/superpowers/YYYY-MM-DD-phase-N-handoff.md`) summarizing what's
done, what's deliberately deferred, and what's left, before opening a PR for that step — see
`docs/superpowers/2026-07-18-phase-3-handoff.md` for the established format. This is what lets a
new conversation/context pick up the remaining work without re-deriving status from git history.

## UI Verification (begin)

`cargo build`/`cargo clippy` passing proves `begin`'s UI code compiles — it proves
nothing about what renders. Before considering any UI change to `begin` complete,
actually render it and look: use the `verifying-begin-ui` skill
(`.claude/skills/verifying-begin-ui/SKILL.md`), which serves `begin` as a web app and
drives headless Edge to screenshot it, dump its DOM, and (when needed) query live
computed styles/shadow-DOM state — `begin`'s default desktop WebView2 window can't be
driven by standard headless-browser tooling. A change that "looks right" in the RSX is
not verified until you've seen it rendered.

## Project Status

This project has not been released yet and has no clients. The API is not stable and may change at
any time. The project is in **active development** and is not yet feature-complete. Prefer
redesigning any components rather than patching them or layering on top of them. The goal is to have a
**clean, correct, and efficient** implementation.

## Architecture

This workspace is centered on `cel-runtime` and is split into libraries, a façade crate, and supporting tools:

- **`cel-rs`** — root façade crate that re-exports `cel-runtime` and depends on `cel-parser` and `cel-rs-macros`
- **`cel-runtime`** — core stack-based runtime; all evaluation and stack machinery lives here
- **`cel-parser`** — recursive-descent CEL parser, lexer, and parser error types
- **`cel-rs-macros`** — proc-macro crate for compile-time CEL expression validation
- **`property-model`** and **`pm-lang`** — supporting crates for the property-model and PM language features
- **`begin`** — Dioxus-based UI application
- **`xtask`** — repository automation and maintenance tasks

### Four-layer stack abstraction (cel-runtime/src/)

The runtime is a **stack-based expression evaluator** built in four layers of increasing type safety:

| Layer                  | File             | Role                                                                            |
| ---------------------- | ---------------- | ------------------------------------------------------------------------------- |
| `RawStack`             | `raw_stack.rs`   | Byte-aligned unsafe stack; `push<T>` returns padding bool, `pop<T>` requires it |
| `RawSegment`           | `raw_segment.rs` | Op list + closure storage + per-op dropper functions                            |
| `DynSegment`           | `dyn_segment.rs` | Runtime type-checking wrapper; maintains `stack_ids: Vec<StackInfo>`            |
| `Segment<Args, Stack>` | `segment.rs`     | Zero-cost compile-time phantom wrapper; `Args: IntoList`, `Stack: List`         |

The compile-time type system uses **cons-cell heterogeneous lists** (`CStackList<H,T>` / `CNil`) defined in `c_stack_list.rs` and `list_traits.rs`.

### Parser pipeline (cel-parser/src/)

```text
&str → TokenStream (proc_macro2) → LexLexer (flatten + combine multi-char ops) → CELParser (recursive descent) → DynSegment
```

`cel-parser/src/lib.rs` contains the grammar entry points and parser pipeline. Function names mirror grammar productions directly (e.g. `is_additive_expression`).

`cel-parser/src/op_table.rs` implements `OpLookup`: a stack of custom `ScopeFn` scopes (LIFO) backed by static `phf_map` built-in ops. Overloading is by arity + `TypeId`.

`cel-parser/src/error.rs` defines `CELError` and `SourceSpan`, plus `format_rustc_style()` for caret diagnostics.

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
