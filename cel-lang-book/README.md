# The CEL Language — book

This crate holds *The CEL Language*, a static mdBook for the CEL
implementation in [`cel-parser`](../cel-parser) and [`cel-runtime`](../cel-runtime).
It is a documentation-only release: examples are checked against the Rust
crates, but there is no live evaluator and no custom mdBook preprocessor. You only
need [mdBook](https://rust-lang.github.io/mdBook/) to build it.

## Building the book

From the repository root:

```text
mdbook build cel-lang-book
mdbook serve cel-lang-book
cargo test -p cel-lang-book
```

`mdbook build cel-lang-book` writes to `cel-lang-book/book-dist/`, and GitHub Pages
publishes that static site at `/cel-book/`.

## Scope

This book documents the CEL syntax and runtime behavior implemented in this
repository. It does not claim to cover every CEL dialect or extension.
