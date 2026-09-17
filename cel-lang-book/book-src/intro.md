# Introduction

This book documents the CEL implementation in `cel-parser` and
`cel-runtime`. It describes the syntax and runtime behavior that this
repository actually supports, not every possible CEL dialect or extension.

The tutorial chapters are ordered as follows:

1. [Literals and types](literals-and-types.md)
2. [Expressions](expressions.md)
3. [Operators](operators.md)
4. [Control flow](control-flow.md)
5. [Collections](collections.md)
6. [Casts and closures](casts-and-closures.md)
7. [Lexical conventions](lexical-conventions.md)

Each chapter uses copyable snippets and links back to the checked examples
in [cel-lang-book/tests/examples.rs](https://github.com/stlab/cel-rs/blob/main/cel-lang-book/tests/examples.rs)
or to the parser/runtime source that defines the behavior.

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
