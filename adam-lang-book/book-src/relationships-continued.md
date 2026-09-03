# Chapter 8: Relationships Continued: Destructuring and Self-Referencing Methods

## 8.1 Destructuring bindings

A binding's left-hand side can name more than one output cell by parenthesizing it, in which
case the right-hand side must be a tuple expression of matching arity, split element-wise:

```adam
{{#include examples/relationships-continued/destructuring_binding.adm2}}
```

`(a, b) := ...` and the one-element `(a,) := ...` (trailing comma mandatory, matching Rust's
own 1-tuple pattern) both destructure; a bare `a := ...` or the equivalent single parenthesized
`(a) := ...` (mere grouping, no comma) instead binds the right-hand side's *whole* result
(including a tuple-typed one) directly to the one named cell. Destructuring and direct-bind are
otherwise governed by the same type-matching rules as any other binding: each output's declared
type must structurally match what the expression actually produces, checked at parse time.

## 8.2 Self-referencing methods

A method's expression may reference the very cell it writes — a _self-referencing_ method —
which [Chapter 7](relationships.md#73-the-rules-a-relationships-methods-must-satisfy) already
noted is explicitly allowed: a cell may appear in both a method's inputs and its own outputs.
Each time the sheet resolves, a self-referencing method reads its own cell's *source* value —
never a previous round's derived value — the same source/derived split
[Chapter 5](filters.md#53-the-raw-value-is-never-lost) already introduced for filters:

```adam
{{#include examples/relationships-continued/self_referencing_method.adm2}}
```

Writing `level` above never applies the clamp itself — exactly like a filter, the correction
happens live, the next time the sheet resolves, against `level`'s own raw value, and that raw
value survives underneath the clamp forever.

## 8.3 Self-referencing methods must be idempotent

A self-referencing method exists to correct its own cell into whatever set of values the
relationship enforces. That only makes sense if reapplying the method to its own already-corrected
output leaves it unchanged — `f(f(x)) == f(x)`. `min(level, 0)` above satisfies this: once
`level` is at most `0`, computing `min` of that value and `0` again produces the same value.

The solver never checks this — nothing about `add_relationship` inspects a method's function for
idempotence, and nothing about `propagate` would refuse to resolve a sheet whose self-referencing
method isn't idempotent. It is purely a correctness obligation on the sheet author. A
self-referencing binding built from a genuinely non-idempotent operation — a literal swap between
two cells' current values, for instance, rather than a one-sided correction like `min`/`max`/
`clamp` — has no single well-defined corrected value for the solver to settle on.
