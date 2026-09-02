# Chapter 1: A Tutorial Introduction

An Adam sheet declares the relationships among a set of properties (invariants in a
document's structure, constraints between a command's arguments and its result, or values
useful for constructing a new argument or document state), instead of the event logic that
would otherwise maintain them by hand.

Let's get started. The best way to learn a new language is to write programs in it, and
Adam programs are called **sheets**. This chapter is a fast, informal tour of every
construct Adam has; later chapters go back over the same ground in more detail, and the
[reference manual](reference.md) collects the precise rules for looking things up.

You don't need to install anything to follow along: every source fragment below stands on its
own as a `.adm2` file, included directly from a test that also exercises it, so a chapter's
prose can never drift from code that actually parses and runs.

## 1.1 A first sheet

An Adam program is a single `sheet`, named, with a body of declarations between braces.
The simplest useful sheet declares a few cells and nothing else:

```
{{#include examples/tutorial/first_sheet.adm2}}
```

A **cell** is a named, typed storage location: the basic unit of state in a property model.
`width` and `height` are `i32`-typed cells, each given an initial value. Semicolons end
declarations, exactly as in Rust or C; a sheet's body is a sequence of declarations, not a
sequence of statements: there is no control flow at this level, no loops, and no imperative
execution order. A sheet describes a *graph* of cells and the constraints between them, not a
sequence of steps to run.

Parsing this text and reading or writing its cells is a Rust-level embedding concern, not
something a sheet author does; see [Appendix A.11](reference.md#a11-the-host-embedding-api)
for how a host application actually drives a parsed sheet.

## 1.2 Relationships: multi-way constraints

A sheet with only cells and no relationships is just a struct. What makes Adam interesting
is the **relationship**: a set of alternative ways to keep a group of cells consistent, any one
of which the solver may pick at any given moment. (A `relationship` binding can never derive a
[`source`](source.md) cell — that's the one kind of cell always left alone as a source; more on
that in [Chapter 3](source.md).)

The classic example is three numbers related by multiplication (`a * b = c`), where any one of
the three can be computed from the other two, the same shape as `pixels == inches * resolution`
from [the introduction](intro.md#why-adam). As a sheet:

```
{{#include examples/tutorial/multiplication_triangle.adm2}}
```

The `relationship` block offers three **bindings**: `c := a * b`, `a := c / b`, and
`b := c / a`, each an alternative *method* for deriving one cell from the others. Only one
binding is active at a time; which one is chosen depends on which cells were written most
recently (see [Chapter 5](relationships.md) for the full rule). A cell's *declaration* counts as
a write for this purpose, so before anything is ever explicitly written, cells declared earlier
are treated as "staler" than cells declared later. The solver prefers to leave the freshest
cells alone and derive the stalest one: here, `c`, declared first.

Nothing here names *which* cell is the "output"; that's the whole point. Whichever cell was
written (or, failing that, declared) least recently is the one the solver derives; `write`ing a
cell is what tells the solver "trust this one; recompute something else instead."

## 1.3 Conditionals

A `conditional` groups relationships that are only active under a matching condition. It
evaluates a **match subject**, then activates whichever branch's literal equals the current
match value:

```
{{#include examples/tutorial/mode_demo.adm2}}
```

Only the active branch's relationships participate in that round's solve; every other branch's
relationships are as if they weren't declared at all. The `_` branch, if present, catches any
value none of the named branches list, and must be written last.

See [Chapter 6](conditionals.md) for branch types, tuple match subjects, and what happens when
no branch matches and there's no default.

## 1.4 Filters: self-correcting cells

A `filter` clause attaches a standing domain constraint to a cell, most commonly a range:

```
{{#include examples/tutorial/clamp_demo.adm2}}
```

Write an out-of-range value and the cell keeps it, raw: a filter never inspects or blocks the
value at the moment it's written. The clamp only takes effect the next time the sheet resolves,
and it's that corrected value every read of the cell sees from then on.

A filter's bounds don't have to be constants: `0..=max` references another cell, and the clamp
tracks it live. [Chapter 7](filters.md) covers filters in full, including the precise
source/derived model behind "the cell keeps its own raw value forever, and the filter only ever
corrects what you *read*," and how the same `filter` clause also attaches to an `out`
declaration.

## 1.5 Destructuring

A binding's left-hand side can name more than one output cell by parenthesizing it, splitting a
tuple-valued expression on the right into its parts, one cell per element, using the same
`(a, b)` syntax Rust uses for tuple patterns:

```
{{#include examples/tutorial/destructuring_demo.adm2}}
```

Tuple *types* (`cell point: (f64, f64) = (0.0, 0.0);`) are a CEL feature, documented in
[Chapter 2](cells.md); destructuring is the relationship-binding syntax built on top of them,
and could one day extend to struct patterns too. See
[Chapter 5](relationships.md#55-destructuring-bindings) for the full
destructuring-vs-direct-bind distinction.

## 1.6 Outputs and requirements

An `out` declaration computes one final, read-only value from the rest of the sheet, and can
carry named `require`ments: boolean checks re-evaluated and reported each time the sheet
resolves, never enforced by rejecting a write:

```
{{#include examples/tutorial/area_with_requirement.adm2}}
```

See [Chapter 8](outputs.md) for the full rules: an output's cell can be read anywhere a plain
cell can, but nothing may ever write it directly, and a failed requirement never stops the
sheet from resolving; it's a diagnostic, not a gate. `require` isn't limited to `out`, either —
see [Chapter 2](cells.md#22-cell-declarations) and [Chapter 3](source.md).

## 1.7 Comments

`//` starts a line comment; `/* ... */` a block comment, exactly as in C, Rust, or CEL. `///`
immediately before a declaration and `//!` immediately before the `sheet` keyword are doc
comments, carried through by the language server and formatter but otherwise inert:

```text
//! A sheet describing a simple resize dialog.
sheet image_resize {
    /// The image's width in pixels, before any resampling.
    cell width_pixels: i32 = 1920;
}
```

See [Chapter 9](style.md) for the formatter's canonical layout.

## 1.8 Where to go next

That's the whole language. [Chapter 2](cells.md) onward covers each construct in the depth this
chapter skipped past, and the [reference manual](reference.md) gives you the full grammar and
every built-in type in one place.
