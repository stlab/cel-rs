# Chapter 7: Relationships and the Solver

## 7.1 Bindings are alternative methods

```text
relationship_decl = "relationship" "{" { binding } "}".
binding            = binding_target ":=" expression ";".
binding_target     = identifier | "(" identifier { "," identifier } [ "," ] ")".
```

Each `binding` inside a `relationship` block is a candidate _method_: an expression whose
dependencies are [deduced](expressions.md#44-deduced-dependencies) from whichever
already-declared cells it references, paired with the cell(s) named on its left of `:=`. A
relationship's bindings are alternatives, not a sequence: at any moment, exactly one of them
is *selected*, and only the selected one's output cell(s) are actually written when the sheet
resolves. The other bindings simply aren't evaluated that round. A [`source`](source.md) cell
can never be named on a binding's left-hand side — it's always a source, by construction, never
a method's output — and an [`out`](outputs.md) cell can be a binding's *input* but never its
output either, since an `out` already has its own fixed writer.

## 7.2 Strength: who gets to stay a source

Every cell carries a _strength_, a write-recency counter: resolving the sheet re-derives the
*stalest* cells it safely can and leaves the *freshest* cells alone. Two things bump a cell's
strength: an explicit write, and, once only, the cell's own declaration. Before any explicit
write has happened, declaration order alone orders every cell's freshness, earliest declared
being stalest. Chapter 1's [§1.5](tutorial.md#15-relationships-a-cell-that-can-be-either-a-source-or-derived)
walks through the simplest case of this rule. A write never touches strength itself except to
promote the written cell to "freshest of all"; reading a cell never changes it.

## 7.3 The rules a relationship's methods must satisfy

Every method in the same `relationship` must reference exactly the same set of cells — the
union of that method's own `inputs` and `outputs` — as every other method in that
relationship; violating this fails to parse with `methods in a relationship must reference
the same set of cells`. A relationship models one fixed group of related cells; its
methods differ only in which subset of that group they treat as the output, using an
"ignore an input" pattern — not in which cells they touch at all. [Chapter 1 §1.5](tutorial.md#15-relationships-a-cell-that-can-be-either-a-source-or-derived)'s
multiplication triangle satisfies this: all three of `c := a * b`, `a := c / b`, and
`b := c / a` reference the same `{a, b, c}`.

A method's own `outputs` list must be duplicate-free, and no two methods in the same
relationship may claim an identical `outputs` set; violating either fails to parse with
`a method's outputs must be duplicate-free, and no two methods in a relationship may share
an outputs set`. The planner treats a method's output set as one indivisible claim, so two
methods claiming the same set would make that claim ambiguous. The triangle's three output
sets — `{c}`, `{a}`, `{b}` — are pairwise distinct, as required.

A cell may appear in both a method's `inputs` and its own `outputs` — a _self-referencing_
method — which is explicitly allowed and has its own rules; see
[Chapter 8](relationships-continued.md).

## 7.4 A shared-cell example

Cells can be shared across more than one relationship, letting the solver's strength
preference cross relationship boundaries. Four cells, two relationships, with `b` and `c`
shared between them:

```adam
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

```adam
{{#include examples/relationships/shared_cell_example.adm2}}
```

Whether a cell came out of the last resolution as a source, left alone rather than derived, is
useful for a host UI deciding whether a field should be editable.

## 7.5 When no assignment exists

Every relationship in a sheet must end up with exactly one selected binding once the sheet
resolves; if that's not possible, resolution fails instead of silently picking something
inconsistent. Two relationships that both, unconditionally, insist on writing the *same* cell
can never both be satisfied, and resolving fails with `no valid method assignment
(overconstrained)` (`Error::Conflict`).

A subtler failure is a _cycle_: an assignment exists, but every valid choice of bindings
forms a closed loop with no cell left as a source anywhere in the loop, and resolving fails
with `selected methods form a cycle` (`Error::Cycle`). This happens when every
relationship in the loop has only one binding, leaving the solver no alternative to try;
giving even one relationship in the loop a second, cycle-breaking binding lets the solver
route around it instead.

Destructuring a binding's output across more than one cell, and a binding that references its
own output cell, are covered next, in [Chapter 8](relationships-continued.md).
