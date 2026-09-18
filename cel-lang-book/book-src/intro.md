# Introduction

This book is a tutorial and reference for CEL: its
tokens, literals, basic types, operators, and collections.

Every CEL source form is an expression: it produces a value or helps compose
one. Identifiers, calls, operators, collections, control flow, casts, and
closures all participate in the same expression language.

The tutorial chapters follow this order:

1. [Lexical conventions](lexical-conventions.md) explains tokens, whitespace,
   comments, identifiers, and punctuation.
2. [Literals and types](literals-and-types.md) explains scalar, compound, and
   closure values.
3. [Operators](operators.md) explains precedence, associativity, arithmetic,
   comparison, logical, bitwise, cast, and range operators.
4. [Conditional expressions](conditional-expressions.md) explains
   value-producing `if` expressions.
5. [Collections](collections.md) explains tuples and homogeneous arrays.
6. [Closures](closures.md) explains closure literals and parameter typing.
7. [Standard library](standard-library.md) explains additional operations
   available when the evaluation environment installs the library.

The [Reference Manual](reference.md) gathers the complete grammar, exact
operator rules, type names, library operations, and edge cases in one place.
