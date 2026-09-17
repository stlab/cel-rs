# CEL language book

## Status

Approved design for issue #211. This first implementation is intentionally static; live CEL
evaluation widgets are deferred to a follow-up design.

## Goal

Provide a standalone, reusable tutorial and reference manual for the Common Expression Language
(CEL) implemented by `cel-parser` and `cel-runtime`. The book must be useful to readers who do
not already know the Adam language and must make the supported language surface discoverable
without requiring them to read parser source code.

## Scope

The first release includes:

- A new `cel-lang-book/` mdBook with a `book-src/` source tree.
- Tutorial chapters covering:
  - literals and runtime types;
  - identifiers, calls, and member-style calls;
  - operators and precedence;
  - conditionals and ranges;
  - arrays, tuples, and grouping;
  - casts and closures;
  - lexical conventions and diagnostics.
- A reference chapter containing the supported grammar, type model, operation lookup and
  overloading rules, and known limitations.
- Static, copyable CEL examples. Examples that exercise the parser/runtime are checked by Rust
  tests where the existing public API makes that practical.
- Build instructions and CI coverage for the book.
- Publication in the existing GitHub Pages artifact at a stable `/cel-book/` path.
- A link from the Adam book's CEL expression material to the standalone reference.

The first release does not include a wasm evaluator, an editable example widget, or a change to
the Adam book's live-example infrastructure. Those require a separate design for evaluator
state, error rendering, and browser bundle ownership.

## Architecture

`cel-lang-book` is a documentation-only workspace member, matching the existing
`adam-lang-book` layout where useful:

```text
cel-lang-book/
  book.toml
  README.md
  src/lib.rs
  book-src/
    SUMMARY.md
    intro.md
    literals-and-types.md
    expressions.md
    operators.md
    control-flow.md
    collections.md
    casts-and-closures.md
    lexical-conventions.md
    reference.md
```

`src/lib.rs` owns the Rust-side example tests and documents how the checked examples correspond
to the prose. The book itself remains static and does not depend on a custom mdBook preprocessor.
The grammar in `reference.md` is copied from the parser's public grammar documentation and is
treated as synchronized documentation: parser grammar changes must update the reference chapter
in the same change.

The book's examples describe the parser/runtime API rather than Adam-specific sheet syntax. The
operation table section explains that built-ins and custom scopes are selected by operation name,
arity, and operand `TypeId`, while avoiding promises about an application-specific standard
library that the current crates do not provide.

## Publication and workflow changes

Both `.github/workflows/ci.yml` and `.github/workflows/docs.yml` install mdBook and run
`mdbook build cel-lang-book` after the existing Adam book build. The docs workflow copies the
result to `target/doc/cel-book`, alongside the generated Rust documentation and Adam book.
The root redirect remains pointed at the facade crate.

The Pages artifact therefore contains:

```text
target/doc/
  cel_rs/
  book/
  cel-book/
```

The book's `book.toml` uses the repository URL and an edit URL rooted at
`cel-lang-book/{path}`. Its README gives the minimal local build and serve commands.

## Documentation contracts

Every public Rust function added for example checking follows the repository's contract-style
documentation rules. Prose uses the terminology and behavior currently exposed by
`cel-parser/src/lib.rs`, `cel-parser/src/op_table.rs`, and the runtime public APIs. Unsupported
syntax and known parser limitations are called out explicitly rather than presented as general
CEL guarantees.

## Verification

The implementation is complete when:

1. `mdbook build cel-lang-book` succeeds from a clean checkout with only mdBook installed.
2. The book is selected by the workspace and its Rust example tests pass.
3. The Adam book links to the standalone CEL book without a broken relative URL in the published
   layout.
4. Both CI and Pages workflows build and publish both books.
5. `cargo fmt --all -- --check`, the focused book tests, and the relevant workspace checks pass.
