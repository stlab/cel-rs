# The Common Expression Language — book

This book presents CEL: its syntax, values, operators, and the
rules that make expressions meaningful.

It is a documentation-only release: the examples are written as CEL snippets, and
there is no live evaluator bundled into the book itself. You only need
[mdBook](https://rust-lang.github.io/mdBook/) to build it.

## Building the book

From the project root:

```text
mdbook build cel-lang-book
mdbook serve cel-lang-book
cargo test -p cel-lang-book
```

`mdbook build cel-lang-book` writes to `cel-lang-book/book-dist/`.

## Scope

This book documents CEL syntax and semantics as a reference. It focuses
on the language forms described throughout the book and does not try to describe
all possible dialect-specific extensions.
