# Introduction

This book is a tutorial and reference for the CEL expression language: its
tokens, literals, expressions, operators, numeric helper calls, control flow,
and collection forms.

The chapters are written in the style of K&R: each one introduces a piece of the
language, then shows the forms it accepts, the rules that govern it, and the
examples that make those rules memorable.

The tutorial chapters are ordered as follows:

1. [Lexical conventions](lexical-conventions.md)
2. [Literals and types](literals-and-types.md)
3. [Expressions](expressions.md)
4. [Operators](operators.md)
5. [Standard library](standard-library.md)
6. [Control flow](control-flow.md)
7. [Collections](collections.md)
8. [Casts and closures](casts-and-closures.md)

The [Standard library](standard-library.md) chapter explains numeric helper
calls layered over the core expression language. It distinguishes the core
`round` call from the optional library functions that are available only when
the evaluation environment installs that library.

Each chapter uses copyable CEL snippets and points back to the checked examples
that exercise the same forms in the book's test suite. When a chapter documents
environment-dependent library calls, the prose says so explicitly.

The first release is intentionally static:

- examples are checked as part of the book's test suite;
- the book builds with `mdbook build cel-lang-book`;
- live evaluation is intentionally deferred to a later release.

## Local commands

```text
mdbook build cel-lang-book
mdbook serve cel-lang-book
cargo test -p cel-lang-book
```

`mdbook build cel-lang-book` writes output to `cel-lang-book/book-dist`.
