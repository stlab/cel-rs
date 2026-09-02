# Chapter 5: Relationships and the Solver

## 5.1 Bindings are alternative methods

```text
relationship_decl = "relationship" "{" { binding } "}".
binding            = binding_target ":=" expression ";".
binding_target     = identifier | "(" identifier { "," identifier } [ "," ] ")".
```

Each `binding` inside a `relationship` block is a candidate **method**: an expression whose
dependencies are [deduced](expressions.md#44-deduced-dependencies) from whichever
already-declared cells it references, paired with the cell(s) named on its left of `:=`. A
relationship's bindings are alternatives, not a sequence: at any moment, exactly one of them
is *selected*, and only the selected one's output cell(s) are actually written when the sheet
resolves. The other bindings simply aren't evaluated that round. A [`source`](source.md) cell
can never be named on a binding's left-hand side — it's always a source, by construction, never
a method's output — and an [`out`](outputs.md) cell can be a binding's *input* but never its
output either, since an `out` already has its own fixed writer.

## 5.2 Strength: who gets to stay a source

Every cell carries a **strength**, a write-recency counter: resolving the sheet re-derives the
*stalest* cells it safely can and leaves the *freshest* cells alone. Two things bump a cell's
strength: an explicit write, and, once only, the cell's own declaration. Before any explicit
write has happened, declaration order alone orders every cell's freshness, earliest declared
being stalest. Chapter 1's [§1.2](tutorial.md#12-relationships-multi-way-constraints) walks
through the simplest case of this rule. A write never touches strength itself except to promote
the written cell to "freshest of all"; reading a cell never changes it.

## 5.3 A shared-cell example

Cells can be shared across more than one relationship, letting the solver's strength
preference cross relationship boundaries. Four cells, two relationships, with `b` and `c`
shared between them:

```text
sheet diamond {
    cell a = 0.0;
    cell b = 0.0;
    cell c = 2.0;
    cell d = 3.0;

    relationship {
        c := a * b;
        b := c / a;
        a := c / b;
    }

    relationship {
        d := b * c;
        c := d / b;
        b := d / c;
    }
}
```

Declaration order makes `d` freshest, then `c`, then `b`, then `a` stalest. The solver tries,
strongest first, to leave each cell a source:

- **`d`**: the second relationship has a binding that doesn't write `d` (`b := d/c` or
  `c := d/b`), so `d` can stay a source.
- **`c`**: with `d` already pinned as a source, the second relationship's only way to avoid
  writing `d` is `b := d/c`, which doesn't touch `c` either, so `c` can *also* stay a source,
  as long as the first relationship also avoids writing it (`a := c/b` does).
- **`b`**: now both relationships are already spoken for in a way that avoids writing `b` only
  by the first relationship (`a := c/b`), but the second relationship's only binding that
  doesn't write `c` or `d` is `b := d/c`, which *does* write `b`. There is no way left to leave
  `b` a source, so the attempt fails and `b` stays claimed by the second relationship.
- **`a`**: stalest, and the first relationship's remaining choice is `a := c/b`; `a` is
  derived.

```
{{#include examples/relationships/shared_cell_example.adm2}}
```

Whether a cell came out of the last resolution as a source, left alone rather than derived, is
useful for a host UI deciding whether a field should be editable.

## 5.4 When no assignment exists

Every relationship in a sheet must end up with exactly one selected binding once the sheet
resolves: if that's not possible, resolution fails instead of silently picking something
inconsistent. Two relationships that both, unconditionally, insist on writing the *same* cell
can never both be satisfied:

```
{{#include examples/relationships/conflict_error.adm2}}
```

A subtler failure is a **cycle**: an assignment exists, but every valid choice of bindings
forms a closed loop with no cell left as a source anywhere in the loop; nothing external ever
breaks the chain:

```
{{#include examples/relationships/cycle_error.adm2}}
```

Each relationship above has only one binding, so the solver has no alternative to try: `x`,
`y`, and `z` are forced into a cycle regardless of strength. Giving even one of the three
relationships a second, cycle-breaking binding (e.g. also allowing `y := x` to run in reverse
as `x := y`) would let the solver route around the loop instead.

## 5.5 Destructuring bindings

A binding's left-hand side can name more than one output cell by parenthesizing it, in which
case the right-hand side must be a tuple expression of matching arity, split element-wise:

```
{{#include examples/relationships/destructuring_binding.adm2}}
```

`(a, b) := ...` and the one-element `(a,) := ...` (trailing comma mandatory, matching Rust's
own 1-tuple pattern) both destructure; a bare `a := ...` or the equivalent single parenthesized
`(a) := ...` (mere grouping, no comma) instead binds the right-hand side's *whole* result
(including a tuple-typed one) directly to the one named cell. Destructuring and direct-bind are
otherwise governed by the same type-matching rules as any other binding: each output's declared
type must structurally match what the expression actually produces, checked at parse time.
