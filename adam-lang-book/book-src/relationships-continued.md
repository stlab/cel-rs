# Relationships Continued: Destructuring and Self-Referencing Methods

## Destructuring bindings

A parenthesized binding target *destructures* a tuple result into output cells.
The result must have matching arity and element types:

```adam
{{#include examples/relationships-continued/destructuring_binding.adm2}}
```

`(a, b) := ...` and the one-element `(a,) := ...` both destructure. Parenthesized
binding targets require the first comma, so `(a) := ...` is invalid. A bare
`a := ...` binds the right-hand side's *whole* result (including a tuple-typed one)
directly to the named cell. Each output's declared type must structurally match its result.

## Self-referencing methods

A *self-referencing method* reads a cell it also writes.
During resolution, it reads that cell's source value, not its previous derived value.
The source remains separate from the computed correction, as it does for
[filters](filters.md#the-raw-value-is-never-lost):

```adam
{{#include examples/relationships-continued/self_referencing_method.adm2}}
```

Writing `level` stores its raw value. The next resolution applies the correction
without overwriting that source.

## Self-referencing methods must be idempotent

A self-referencing method must be *idempotent*: reapplying it to its corrected output
leaves the output unchanged, `f(f(x)) == f(x)`. `min(level, 0)` satisfies this:
once `level` is at most `0`, applying `min` again returns the same value.

The sheet author must ensure idempotence; neither construction nor propagation checks it.
A non-idempotent operation, such as swapping two cells' values, does not define a
stable correction.
