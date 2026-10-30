# Relationships and the Solver

## Bindings are alternative methods

```text
relationship_decl = "relationship" "{" { binding } "}".
binding            = binding_target ":=" expression ";".
binding_target     = identifier | "(" identifier "," [ identifier { "," identifier } ] ")".
```

Each `binding` defines a candidate *method*: an expression with
[deduced inputs](expressions.md#deduced-dependencies) and the outputs named to the left
of `:=`. Resolution selects exactly one method per active relationship.
Only that method writes its outputs; other methods may contribute to self-referencing
seed construction. Selection does not guarantee evaluation; the runtime may reuse a result.
Neither a [`source`](source.md) nor an [`out`](outputs.md) cell may be a binding's output.
Both may be inputs.

## Strength: who gets to stay a source

Every cell carries a *strength*, a write-recency counter. The solver prefers to derive
the stalest cells and leave the freshest as sources. A declaration gives a cell its initial
strength, so later declarations start stronger. An explicit write promotes its cell to
freshest; reading never changes strength. See the
[tutorial example](tutorial.md#cells-and-relationships).

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

Every method in a relationship must reference the same `inputs ∪ outputs` cell set.
A violation fails at parse time with `methods in a relationship must reference
the same set of cells`. Methods differ in which cells they derive, not which group
they constrain. The tutorial's multiplication triangle satisfies this rule:
`c := a * b`, `a := c / b`, and `b := c / a` all reference `{a, b, c}`.

A method's `outputs` list must be duplicate-free, and no two methods in a relationship
may claim the same output set. Either violation fails at parse time with
`a method's outputs must be duplicate-free, and no two methods in a relationship may share
an outputs set`. The triangle's output sets, `{c}`, `{a}`, and `{b}`, are distinct.

A *self-referencing method* includes a cell in both its inputs and outputs; see the
[relationships-continued chapter](relationships-continued.md).

## A shared-cell example

Cells may participate in multiple relationships. Here, two relationships share `b` and `c`:

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

- `d`: the second relationship has a binding that doesn't write `d` (`b := d/c` or
  `c := d/b`), so `d` can stay a source.
- `c`: keeping both `d` and `c` as sources selects `b := d/c` in the second relationship.
  The first can select `a := c/b`, so both sources can be preserved.
- `b`: preserving `c` and `d` requires the second relationship to write `b`.
  The solver cannot also leave `b` as a source.
- `a`: the first relationship's remaining choice is `a := c/b`, deriving `a`.

```adam
{{#include examples/relationships/shared_cell_example.adm2}}
```

Hosts can query which cells remained sources to choose which fields to make editable.

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

The imported examples need only built-in arithmetic and comparisons.
Hosts using the image examples register Euclid types and
install `cel-std` and `cel-euclid` on the lookup before creating the immutable
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

Resolution requires exactly one selected binding per active relationship.
Two unconditional relationships that can only write the same cell cannot both be
satisfied; resolution fails with `no valid method assignment
(overconstrained)` (`Error::Conflict`).

A *cycle* occurs when assignments exist, but every choice of bindings
forms a closed dependency loop without a source. Resolution fails
with `selected methods form a cycle` (`Error::Cycle`). This happens when every
relationship in the loop has only one binding, leaving the solver no alternative to try;
giving even one relationship in the loop a second, cycle-breaking binding lets the solver
route around it instead.
