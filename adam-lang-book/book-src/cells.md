# Sheets, Cells, and Types

## Sheets

The examples begin with a single root sheet. An Adam module may also contain imports
and multiple root or parameterized sheet definitions:

```adam
sheet name {
    /* cell, relationship, conditional, and out declarations, in any order,
       except that each identifier must be declared before it is referenced —
       see "Names and declaration order" below. */
}
```

The sheet's name (`name` above) identifies the definition for module resolution and
root selection. Host tooling also uses it for display.

## Cell declarations

```text
cell_decl      = "cell" identifier cell_type_init [ cell_filter ] [ require_block ] ";".
cell_type_init = (":" type_expr ["=" expression]) | ("=" expression).
```

A cell needs a type, an initial value, or both:

```adam
cell width: i32;             // type only — needs a registered default
cell height: i32 = 1080;     // type and initializer
cell area = 0;               // initializer only — type is inferred
```

`cell width;` is a syntax error ("expected `:` or `=` in cell declaration").
A cell may also carry a trailing [`filter`](filters.md) clause for a domain constraint,
a [`require`](outputs.md#requirements-diagnostics-not-gates) block for boolean diagnostics,
or both. These clauses also apply to [`source`](source.md) and [`out`](outputs.md) cells.

A cell's initializer may reference literals and CEL operators, but not other cells: it's
evaluated before the sheet has any state to read.
To compute one cell from others, use a [`relationship`](relationships.md) or an
[`out`](outputs.md) declaration instead.

## Built-in types and inference

Adam ships with the following types pre-registered, each with a `Default` value used when
a cell declares a type but no initializer:

| Type name | Rust type | Default |
|---|---|---|
| `i8`, `i16`, `i32`, `i64`, `i128`, `isize` | the matching signed integer | `0` |
| `u8`, `u16`, `u32`, `u64`, `u128`, `usize` | the matching unsigned integer | `0` |
| `f32`, `f64` | the matching float | `0.0` |
| `bool` | `bool` | `false` |
| `String` | `String` | `""` |

Without a `: type_expr` annotation, the initializer's result determines the cell's type.
CEL defaults an unsuffixed integer literal such as `0` to `i32` and an unsuffixed float
such as `0.0` to `f64`; see the [CEL literal grammar](../cel-book/literals-and-types.html).
When both a type annotation and an initializer are present, their types must agree exactly,
or parsing fails with "type mismatch: expected `T`, got `U`"
(see [error messages](reference.md#error-messages)).

A host application can register additional Rust types under Adam type names through
the [type registry](reference.md#the-type-registry).

## Cells with no default

A type registered without a `Default` (via the embedding API's `register_no_default`) can
still be used for a cell, but only with an initializer; declaring one with a bare `: T` and no
`= ...` fails to parse ("type `T` has no default; provide `= ...`"). Every built-in type in the
table above has a default, so this only matters for a host-registered custom type.

## Tuple types

```text
type_expr = identifier [ "(" type_expr { "," type_expr } ")" ]
          | "[" type_expr "]"
          | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".
```

A parenthesized type list is a *tuple type*. `()` is the empty tuple;
`(T)` groups the type `T`; `(T,)` is a one-element tuple with a required comma;
`(T, U, ...)` is a tuple with multiple elements:

```adam
{{#include examples/cells/tuple_typed_cell.adm2}}
```

Tuples nest: `(i32, (f64, String))` is a 2-tuple whose second element is itself a 2-tuple.
Compound cells own canonical CEL values. Host code uses `ParsedSheet::read_compound` and
`write_compound` with complete representation witnesses. For example,
`Cons<Leaf<i32>, Cons<Leaf<f64>, Nil>>` witnesses `(i32, f64)`. Reads return independent
owned clones; writes validate the complete recursive shape before changing the cell.
`source_compound` reads the original aspiration independently of any filter result.
Concrete registered leaves retain the ordinary Rust `read::<T>` / `write` APIs.

## Arrays, generic leaves, and callables

Arrays and tuples compose recursively: `[(i32, [f64])]` is an array of pairs whose second
field is another array. An array defaults to empty without requiring an element default;
a tuple defaults each child recursively. A missing leaf default names that leaf.
Register a generic foreign leaf under its full source name, such as `RangeInclusive(i32)`;
containing array and tuple shapes need no additional registration.

`Fn((A, B), R)` describes a callable with two parameters; `(A,)` and `()` describe one
and zero parameters. A callable has no default and requires an initializer. Every Adam
cell also requires cloning and value equality: CEL can store callable values, but Adam
rejects a callable cell when its descriptor lacks equality.
Registered host function markers retain the concrete
capabilities their type registration supplies.

## Executable host boundary

A host supplies `ArrayOf`, `Cons`, `Nil`, and `Leaf` witnesses
to the checked `ParsedSheet` boundary. A wrong nested shape fails before mutation.
Cloning and equality are required for persistence/propagation; debug formatting
is an independent capability. Containers themselves do not impose those bounds.
An empty array of callable elements also lacks equality when its element descriptor does.

<!-- [knowledge]
Keep CellValue as one canonical whole-value owner, rather than erasing each array
element. Preserve descriptor capabilities for empty containers; do not substitute
function identity for value equality.
-->

This example defaults recursive compounds, writes an array of concrete tuple
lists, propagates it to an output, and reads the source aspiration:

```rust
{{#include ../tests/composable_containers.rs:compound-cells}}
```

This example checks the missing-equality error:

```rust
{{#include ../tests/composable_containers.rs:callable-cell-capability}}
```

The Inspector formats arrays and tuples recursively when debug capabilities
exist, including typed empty arrays and unit. Compound edits are explicitly
unsupported and leave the cell unchanged; primitive editors retain their
ordinary behavior. Formatting does not imply an editable source round trip.

## Names and declaration order

A cell's name must be unique across the sheet. `cell`, [`source`](source.md),
and [`out`](outputs.md) declarations share one namespace; declaring both
`cell result: i32 = 0;` and `out result := ...;` is a duplicate-name error.

Referencing a name before its declaration, whether as a binding output or an expression
dependency, is an
"undeclared cell `name`" error (see [error messages](reference.md#error-messages)).
