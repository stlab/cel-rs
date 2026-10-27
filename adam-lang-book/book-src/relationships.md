# Relationships and the Solver

## Bindings are alternative methods

```text
relationship_decl = "relationship" "{" { binding } "}".
binding            = binding_target ":=" expression ";".
binding_target     = identifier | "(" identifier { "," identifier } [ "," ] ")".
```

Each `binding` inside a `relationship` block is a candidate _method_: an expression whose
dependencies are [deduced](expressions.md#deduced-dependencies) from whichever
already-declared cells it references, paired with the cell(s) named on its left of `:=`. A
relationship's bindings are alternatives, not a sequence: at any moment, exactly one of them
is *selected*, and only the selected one's output cell(s) are actually written when the sheet
resolves. Other bindings may contribute to self-referencing seed construction. Selection
does not guarantee that an expression is evaluated; the runtime may reuse its result.
A [`source`](source.md) cell can never be named on a binding's left-hand side —
it's always a source, by construction, never
a method's output — and an [`out`](outputs.md) cell can be a binding's *input* but never its
output either, since an `out` already has its own fixed writer.

## Strength: who gets to stay a source

Every cell carries a _strength_, a write-recency counter: resolving the sheet re-derives the
*stalest* cells it safely can and leaves the *freshest* cells alone. Two things bump a cell's
strength: an explicit write, and, once only, the cell's own declaration. Before any explicit
write has happened, declaration order alone orders every cell's freshness, earliest declared
being stalest. The tutorial's [Cells and Relationships](tutorial.md#cells-and-relationships) section
walks through the simplest case of this rule. A write never touches strength itself except to
promote the written cell to "freshest of all"; reading a cell never changes it.

## Automatic plan reuse

Resolving the sheet does not always rerun method selection. The runtime retains one
prepared unconditional plan for conditional guards and one prepared main plan for the
current active relationships. A write to a released source can preserve a phase's plan;
a write to a claimed cell, a changed active set, or a structural mutation can require
planning again. A filtered source remains released, but a self-referencing method's
output is claimed. The host calls the same `propagate` API either way.

Reuse preserves assignments and structural seed choices, not old values. Resolution
still uses current inputs, strength-sensitive seed gates and sibling ordering, and
current diagnostics. Released-source membership bookkeeping takes expected constant
time; comparing active sets takes time proportional to their size, and checking a
pending source-priority certificate takes time proportional to the cell count.
Local sibling sorting and fresh value evaluation retain their costs. Discarding a
prepared plan can also take time proportional to its artifact size. This optimization
does not add cross-resolution value memoization or incremental assignment repair.

## The rules a relationship's methods must satisfy

Methods are purely functional: their results depend only on input values and immutable
constants. Equal inputs produce equivalent values or errors. They have no externally
observable side effects and do not read mutable external state. There is no guarantee
that a method executes, or when or how often it executes. The same contract applies to
filters, conditional expressions, and requirements, including callbacks supplied through
the Rust host API. Represent changing dependencies as cells, not hidden callback state.

Every method in the same `relationship` must reference exactly the same set of cells — the
union of that method's own `inputs` and `outputs` — as every other method in that
relationship; violating this fails to parse with `methods in a relationship must reference
the same set of cells`. A relationship models one fixed group of related cells; its
methods differ only in which subset of that group they treat as the output, using an
"ignore an input" pattern — not in which cells they touch at all. The multiplication triangle from
the tutorial's [Cells and Relationships](tutorial.md#cells-and-relationships) section
satisfies this: all three of `c := a * b`, `a := c / b`, and
`b := c / a` reference the same `{a, b, c}`.

A method's own `outputs` list must be duplicate-free, and no two methods in the same
relationship may claim an identical `outputs` set; violating either fails to parse with
`a method's outputs must be duplicate-free, and no two methods in a relationship may share
an outputs set`. The planner treats a method's output set as one indivisible claim, so two
methods claiming the same set would make that claim ambiguous. The triangle's three output
sets — `{c}`, `{a}`, `{b}` — are pairwise distinct, as required.

A cell may appear in both a method's `inputs` and its own `outputs` — a _self-referencing_
method — which is explicitly allowed and has its own rules; see the
[relationships-continued chapter](relationships-continued.md).

## A shared-cell example

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

## Reuse a component

A *parameterized sheet* names a reusable component. Its concrete formal parameters
alias caller-owned cells. A sheet without parentheses defines a root; `sheet empty() {}`
defines a zero-parameter component, not a root.

This inline definition uses the same three floating multiplication methods as the
triangle above. Binding `factor` to a fixed source removes the method that would
write it, leaving the two directions between `value` and `result`.

```adam
{{#include examples/relationships/inline_component.adm2}}
```

Each selected division needs a nonzero divisor; callers requiring finite arithmetic
also require finite operands and representable results. Components do not infer
inverse equations or choose directions from current numeric values.
Floating operations retain IEEE 754 behavior, including infinities and NaNs;
they do not report the checked errors that integer division reports.

The shipped `adam-lang/examples/relationships.adm2` also defines finite ordered
bounds. `composed.adm2` imports both definitions, aliases `multiply` as `mul`, and
uses the same three-way floating component twice with distinct named actuals.
Those files contain several roots and a component-only module, so hosts register
both modules and select a root explicitly:

```rust
{{#include ../tests/relationships.rs:imported_components}}
```

The test above executes the real files. Their arithmetic and comparisons need
only builtin operations. Hosts using the image examples register Euclid types and
install `cel-std` and `cel-euclid` on the lookup **before** creating the immutable
compiler or parser. An empty lookup does not supply those optional functions.

For source-free Rust construction, run `cargo run -p adam-lang --example templates`.
Its native path builds `TemplateBuilder`/`MethodRecipe` factories and instantiates
them into an ordinary `adam_rs::Sheet`; it never parses source. The checked integer
example uses Rust's truncating division, so exact inverse multiplication requires
a divisible product. The parsed examples use `f64`.

`cel-euclid/examples/image_constraints.adm2` extracts the active resampling axis,
an aspect relationship and the crop fit from the existing examples.
`resize_straighten.adm2` binds two axis instances and opts into
`aspect(width: width_inches, height: height_inches, ratio: width_per_height)`.
The caller supplies the positive finite `width_per_height` source, initially
`1600.0 / 1200.0`. Equating the original width/height percentages gives
`width = height * ratio`; the reverse direction divides width by that nonzero
ratio. Changing only width to 4 inches derives height as 3 inches; changing only
height back to 4 inches restores width to `1600.0 / 300.0` inches.
Dimensions and selected products/quotients must stay positive, finite and
representable. This relation constrains physical dimensions; the pixel axes still
round independently, so integer pixel counts need not have an exact ratio.
Ordinary derived dimensions update their raw source slots;
edited source dimensions, filtered pixel requests and the self-referencing crop
retain their caller aspirations under the ordinary sheet rules.

The root feeds an ordinary
relationship-produced image rectangle into the crop component. The caller owns
sources, filters, requirements and the crop aspiration. Private anchor/coverage
outputs remain diagnostics, never actual arguments.
This caller chooses filters that conform pixel requests to the original
300000-pixel limit while preserving their source aspirations; the original
`begin` example instead reports that limit through requirements alone. The
reusable axis installs neither policy, so other callers retain their own choice.

The axis retains ties-even pixel rounding and checked casts. The caller selects
the reusable aspect relation separately; neither component implements the original
example's resample toggle or other UI machinery. The fit retains its
8-epsilon inward guard, 32-epsilon cancellation check and 64-epsilon overlay and
coverage budgets. The caller rotates about the image point under the requested
crop center. Singular transforms fail checked inversion. Insufficient precision
invalidates an explicit requirement, rather than producing a promised valid fit.
Read requirement validity after propagation; a successful propagation alone does
not certify coverage. Resize, rotation and reversal preserve `source(crop)`.

See the [component reference](reference.md#components-modules-and-roots) for binding,
conditional eligibility, ownership and error rules.

## When no assignment exists

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
own output cell, are covered next, in the [relationships-continued chapter](relationships-continued.md).
