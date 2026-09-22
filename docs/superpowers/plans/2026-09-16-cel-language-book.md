# CEL Language Book Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a standalone static CEL tutorial/reference book, checked Rust examples, Adam-book cross-links, and CI/Pages publication for issue #211.

**Architecture:** Create `cel-lang-book` as a documentation-only workspace member modeled on `adam-lang-book`, but without a preprocessor or wasm bundle. Keep prose and grammar in `book-src/`, validate representative parser/runtime behavior in `src/lib.rs` and tests, then build the book independently in CI and copy it to `target/doc/cel-book` beside the existing Adam book.

**Tech Stack:** Rust 2024 workspace, `cel-parser`, `cel-runtime`, mdBook, GitHub Actions, Markdown.

**Spec:** `docs/superpowers/specs/2026-09-16-cel-language-book-design.md`

## Global Constraints

- Keep the first release static; do not add a wasm evaluator or mdBook preprocessor.
- Preserve the existing Adam live-book build and publication path.
- Treat the grammar in `cel-parser/src/lib.rs` and the reference chapter as synchronized documentation.
- Follow contract-style Rust documentation for every public Rust function added.
- Avoid promising an application-specific CEL standard library; describe current parser/runtime behavior.
- Use the `/cel-book/` published path and copy output to `target/doc/cel-book`.
- Run focused tests and formatting before broader workspace checks.

---

## File Map

- Create `cel-lang-book/Cargo.toml`: documentation crate manifest and test target.
- Create `cel-lang-book/src/lib.rs`: module contract and checked parser/runtime examples.
- Create `cel-lang-book/book.toml`: standalone mdBook metadata and output settings.
- Create `cel-lang-book/README.md`: local build/serve instructions and scope notes.
- Create `cel-lang-book/book-src/SUMMARY.md`: book navigation.
- Create `cel-lang-book/book-src/intro.md`: purpose, implementation status, and notation.
- Create `cel-lang-book/book-src/literals-and-types.md`: literals and runtime type model.
- Create `cel-lang-book/book-src/expressions.md`: identifiers, calls, grouping, and expression evaluation.
- Create `cel-lang-book/book-src/operators.md`: precedence and operator behavior.
- Create `cel-lang-book/book-src/control-flow.md`: conditionals and ranges.
- Create `cel-lang-book/book-src/collections.md`: arrays, tuples, and current limitations.
- Create `cel-lang-book/book-src/casts-and-closures.md`: casts and closure syntax/typing.
- Create `cel-lang-book/book-src/lexical-conventions.md`: token and source conventions.
- Create `cel-lang-book/book-src/reference.md`: annotated grammar, type model, op lookup, and limitations.
- Create `cel-lang-book/tests/examples.rs`: contract-derived integration tests for representative examples.
- Modify `Cargo.toml`: add `cel-lang-book` to workspace members.
- Modify `adam-lang-book/book-src/expressions.md`: link CEL readers to `/cel-book/`.
- Modify `.github/workflows/ci.yml`: build the standalone book in PR CI.
- Modify `.github/workflows/docs.yml`: build and publish the standalone book.

## Interfaces Between Tasks

- The workspace member exposes package `cel-lang-book` and a library target whose tests use `cel_parser::{CELParser, OpLookup}` and `cel_runtime::DynamicArray`.
- The book builds with `mdbook build cel-lang-book` and writes to `cel-lang-book/book-dist`.
- The Pages workflow publishes the standalone book at `target/doc/cel-book/index.html`.
- Adam-book links use `../cel-book/index.html` from the copied `target/doc/book` site, producing the published `/cel-book/` path.

### Task 1: Scaffold the `cel-lang-book` crate and minimal book

**Files:**
- Create: `cel-lang-book/Cargo.toml`
- Create: `cel-lang-book/src/lib.rs`
- Create: `cel-lang-book/book.toml`
- Create: `cel-lang-book/README.md`
- Create: `cel-lang-book/book-src/SUMMARY.md`
- Create: `cel-lang-book/book-src/intro.md`
- Modify: `Cargo.toml`

**Interfaces:**
- Produces workspace package `cel-lang-book`.
- Produces a book that `mdbook build cel-lang-book` can compile without custom preprocessors.

- [ ] **Step 1: Write the package and book metadata**

Use a library package with the existing workspace lint inheritance and only path dependencies needed by checked examples:

```toml
[package]
name = "cel-lang-book"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
cel-parser = { path = "../cel-parser" }
cel-runtime = { path = "../cel-runtime" }

[lints]
workspace = true
```

Set `book.toml` to title the book “The CEL Language”, use `book-src` as `src`, write output to `book-dist`, and configure the repository/edit URLs under `cel-lang-book/{path}`. Do not register a preprocessor.

- [ ] **Step 2: Add the crate to the workspace and write the module contract**

Add `"cel-lang-book"` to the workspace members. In `src/lib.rs`, add a module-level tutorial explaining that the crate exists to compile-check examples used by the static book, then add a small public marker function only if the crate needs one for rustdoc; otherwise keep the library documentation-only and put checks in `tests/examples.rs`.

- [ ] **Step 3: Add the initial navigation and README**

Create `SUMMARY.md` with the complete chapter order from the file map and an intro page that states the book documents the implementation in `cel-parser`/`cel-runtime`, not all possible CEL dialects. Document the exact commands:

```text
mdbook build cel-lang-book
mdbook serve cel-lang-book
cargo test -p cel-lang-book
```

State that examples are static in this first release and live evaluation is intentionally deferred.

- [ ] **Step 4: Build the scaffold**

Run:

```text
cargo test -p cel-lang-book
mdbook build cel-lang-book
```

Expected: the package test target compiles and mdBook writes `cel-lang-book/book-dist/index.html`.

- [ ] **Step 5: Commit**

```text
git add Cargo.toml cel-lang-book
git commit -m "docs(cel-lang-book): scaffold standalone CEL book"
```

### Task 2: Add checked parser/runtime examples

**Files:**
- Create: `cel-lang-book/tests/examples.rs`
- Modify: `cel-lang-book/src/lib.rs`

**Interfaces:**
- Tests use only public APIs and validate behavior described in book snippets.
- Representative checks cover arithmetic precedence, conditionals, arrays, and closures/op lookup where the current API supports them.

- [ ] **Step 1: Write failing contract tests**

Add independent tests with concrete expected values:

```rust
use cel_parser::{CELParser, OpLookup};
use cel_runtime::DynamicArray;

#[test]
fn arithmetic_respects_precedence() {
    let mut segment = CELParser::new(OpLookup::new())
        .parse_str("10u32 + 20u32 * 5u32")
        .unwrap();
    assert_eq!(segment.call0::<u32>().unwrap(), 110);
}

#[test]
fn array_literal_has_one_recursive_element_type() {
    let mut segment = CELParser::new(OpLookup::new())
        .parse_str("[0, 1, 2]")
        .unwrap();
    let array: DynamicArray = segment.call0().unwrap();
    assert_eq!(array.try_into_vec::<i32>().unwrap(), vec![0, 1, 2]);
}

#[test]
fn heterogeneous_array_is_rejected() {
    assert!(CELParser::new(OpLookup::new())
        .parse_str("[1i32, 2.0f64]")
        .is_err());
}
```

Add further tests only for syntax and behavior documented in the prose, such as an `if` expression and a closure invocation, using the actual operation registrations available in `cel-parser` rather than inventing a standard library.

- [ ] **Step 2: Run the focused tests and confirm the intended baseline**

Run:

```text
cargo test -p cel-lang-book --test examples
```

Expected: any test exposing unsupported syntax or an incorrect assumed API fails with the compiler/parser diagnostic; adjust the example to the actual public contract before proceeding, without weakening the assertion.

- [ ] **Step 3: Document the test/example relationship**

Add a `//!` explanation in `src/lib.rs` that the tests are executable companions to the static book and that unsupported dialect features must not be presented as implemented behavior. Add contract docs to any public item introduced.

- [ ] **Step 4: Run the focused tests**

Run:

```text
cargo test -p cel-lang-book --test examples
```

Expected: all example tests pass.

- [ ] **Step 5: Commit**

```text
git add cel-lang-book/src/lib.rs cel-lang-book/tests/examples.rs
git commit -m "test(cel-lang-book): check representative CEL examples"
```

### Task 3: Write the tutorial chapters

**Files:**
- Create: `cel-lang-book/book-src/literals-and-types.md`
- Create: `cel-lang-book/book-src/expressions.md`
- Create: `cel-lang-book/book-src/operators.md`
- Create: `cel-lang-book/book-src/control-flow.md`
- Create: `cel-lang-book/book-src/collections.md`
- Create: `cel-lang-book/book-src/casts-and-closures.md`
- Create: `cel-lang-book/book-src/lexical-conventions.md`
- Modify: `cel-lang-book/book-src/intro.md`

**Interfaces:**
- Chapters use copyable fenced CEL/Rust snippets and link to the reference chapter.
- Prose must agree with the checked tests and current parser grammar.

- [ ] **Step 1: Write literals/types and expressions chapters**

Explain integer/float/bool/string/char literals as accepted by the lexer, identifiers as operation inputs or registered values, tuples/grouping, parameter lists, and the distinction between a parser expression and a runtime stack result. Include the smallest parser API example:

```rust
let mut segment = CELParser::new(OpLookup::new()).parse_str("10u32").unwrap();
assert_eq!(segment.call0::<u32>().unwrap(), 10);
```

- [ ] **Step 2: Write operators and control-flow chapters**

Present precedence from lowest to highest using the actual grammar: logical, comparison, bitwise, shift, additive, multiplicative, casts, unary, and postfix. Explain the supported `if`/`else if`/`else` expression form and inclusive/exclusive range forms, including that evaluation depends on registered operations.

- [ ] **Step 3: Write collections, casts/closures, and lexical chapters**

Document non-empty homogeneous arrays, recursive array element typing, tuple/grouping differences, `as` casts, closure parameter type syntax, closure body expressions, token spacing/comments as actually handled by the lexer, and source-span diagnostics. Explicitly list current limitations instead of implying that empty arrays or every CEL dialect feature is supported.

- [ ] **Step 4: Link each chapter to executable examples or reference rules**

For every behavior claimed as implemented, link either to a test in `tests/examples.rs` or to the relevant reference section. Keep prose static; do not add a fake live-evaluation placeholder.

- [ ] **Step 5: Build the book**

Run:

```text
mdbook build cel-lang-book
```

Expected: all chapters render without broken internal links.

- [ ] **Step 6: Commit**

```text
git add cel-lang-book/book-src
git commit -m "docs(cel-lang-book): add CEL tutorial chapters"
```

### Task 4: Add the reference manual and Adam cross-link

**Files:**
- Create: `cel-lang-book/book-src/reference.md`
- Modify: `cel-parser/src/lib.rs`
- Modify: `adam-lang-book/book-src/expressions.md`

**Interfaces:**
- `reference.md` is the reader-facing annotated grammar and implementation reference.
- `cel-parser/src/lib.rs` remains the rustdoc source of the grammar snippet; both must stay synchronized.
- Adam readers can navigate from the existing CEL explanation to `../cel-book/index.html`.

- [ ] **Step 1: Copy and annotate the current grammar**

Transfer the grammar currently documented in `cel-parser/src/lib.rs` into `reference.md`, preserving productions for expression/range/logical/bitwise/arithmetic/cast/unary/postfix/primary/tuple/array/if/closure/parameter expressions and literal patterns. Add a short prose paragraph before each grammar group explaining what it recognizes.

- [ ] **Step 2: Add the type and operation model**

Describe runtime values, stack-layout tuples, recursively homogeneous `DynamicArray` values, parser-time type checking, `OpLookup` LIFO custom scopes, built-in fallback, and overload selection by operation name, arity, and operand `TypeId`. State which behavior is implementation-specific and which is syntax.

- [ ] **Step 3: Add known limitations and synchronization guidance**

Document empty arrays, heterogeneous arrays, tuple array elements, and diagnostic-span distinctions using the current parser docs and issue references. Add a note that grammar changes require updating both the parser module docs and this chapter in the same change.

- [ ] **Step 4: Improve parser rustdoc cross-reference**

Update the parser module’s grammar heading to link readers to the standalone book’s reference chapter in the published layout, while retaining the inline grammar needed for rustdoc users. Do not remove the existing grammar or examples.

- [ ] **Step 5: Link Adam’s CEL section**

In `adam-lang-book/book-src/expressions.md`, replace the standalone “CEL grammar is defined by `cel-parser`” punt with a concise link to `../cel-book/index.html` and the reference chapter. Keep Adam-specific syntax explanation in place.

- [ ] **Step 6: Verify links and build**

Run:

```text
mdbook build cel-lang-book
mdbook build adam-lang-book
```

Expected: both books build; the Adam book contains a relative link that resolves when copied under `target/doc/book` beside `target/doc/cel-book`.

- [ ] **Step 7: Commit**

```text
git add cel-lang-book/book-src/reference.md cel-parser/src/lib.rs adam-lang-book/book-src/expressions.md
git commit -m "docs: connect CEL reference to parser and Adam book"
```

### Task 5: Wire CI and GitHub Pages publication

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/docs.yml`
- Modify: `cel-lang-book/README.md`

**Interfaces:**
- PR CI fails if the standalone book does not build.
- Pages publishes `target/doc/cel-book` beside the existing Rust docs and Adam book.

- [ ] **Step 1: Add CI book build**

After the existing Adam book build in `.github/workflows/ci.yml`, add:

```yaml
- name: Build the CEL language book
  run: mdbook build cel-lang-book
```

Do not install the Adam live-example preprocessor for the CEL book beyond the existing workflow steps, and do not alter the Adam build order.

- [ ] **Step 2: Add Pages book build and copy**

After building `adam-lang-book` in `.github/workflows/docs.yml`, add `mdbook build cel-lang-book`. After copying the Adam book, add a separate stale-directory-safe copy:

```yaml
- name: Copy the CEL language book into the docs site
  run: |
    rm -rf target/doc/cel-book
    cp -r cel-lang-book/book-dist target/doc/cel-book
```

- [ ] **Step 3: Update local build instructions**

Ensure `cel-lang-book/README.md` states that the book needs only mdBook, that its output is `book-dist`, and that the published location is `/cel-book/`. Keep Adam’s special wasm/preprocessor instructions out of this README.

- [ ] **Step 4: Validate workflow command parity locally**

Run:

```text
mdbook build cel-lang-book
mdbook build adam-lang-book
cargo test -p cel-lang-book
```

Expected: all commands succeed and `cel-lang-book/book-dist/index.html` exists.

- [ ] **Step 5: Commit**

```text
git add .github/workflows/ci.yml .github/workflows/docs.yml cel-lang-book/README.md
git commit -m "ci(docs): build and publish CEL language book"
```

### Task 6: Final verification and review

**Files:**
- No new files; inspect all changed files.

- [ ] **Step 1: Run formatting**

Run:

```text
cargo fmt --all -- --check
```

Expected: no formatting changes required.

- [ ] **Step 2: Run focused tests and builds**

Run:

```text
cargo test -p cel-lang-book
mdbook build cel-lang-book
mdbook build adam-lang-book
```

Expected: all tests and both books build successfully.

- [ ] **Step 3: Run relevant workspace checks**

Run:

```text
cargo check -p cel-lang-book
cargo test --workspace --exclude begin
```

Expected: no failures or new warnings attributable to the book.

- [ ] **Step 4: Inspect the publication layout**

Create the same directory arrangement used by the Pages workflow and verify:

```text
target/doc/book/index.html
target/doc/cel-book/index.html
```

Confirm the Adam book’s CEL link resolves to `../cel-book/index.html`.

- [ ] **Step 5: Review the diff against the spec**

Check that the diff contains no wasm evaluator, live widget, unrelated parser behavior change, or undocumented claim of full CEL dialect compatibility. Confirm the reference grammar matches `cel-parser/src/lib.rs`.

- [ ] **Step 6: Commit any final corrections**

```text
git status --short
git diff --check
git add Cargo.toml cel-lang-book adam-lang-book/book-src/expressions.md cel-parser/src/lib.rs .github/workflows/ci.yml .github/workflows/docs.yml
git commit -m "docs(cel-lang-book): finalize issue 211 implementation"
```
