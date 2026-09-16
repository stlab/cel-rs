# Introduction

This book documents the CEL implementation in `cel-parser` and
`cel-runtime`. It describes the syntax and runtime behavior that this
repository actually supports, not every possible CEL dialect or extension.

The first release is intentionally static:

- examples are checked at compile time against the Rust crates;
- the book builds with `mdbook build cel-lang-book`;
- live evaluation is intentionally deferred to a later release.

## Local commands

```text
mdbook build cel-lang-book
mdbook serve cel-lang-book
cargo test -p cel-lang-book
```

`mdbook build cel-lang-book` writes output to `cel-lang-book/book-dist`.
