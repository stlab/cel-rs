# Sheets, Cells, and Types

## Sheets

Every Adam source file is one sheet:

```adam
sheet name {
    /* cell, relationship, conditional, and out declarations, in any order,
       except that each identifier must be declared before it is referenced —
       see "Names and declaration order" below. */
}
```

The sheet's own name (`name` above) is consumed by the parser and is not otherwise meaningful
to `adam-rs`; it exists for readability and for host tooling, such as a language server or an
example picker, to have something to display.

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

At least one of `: type_expr` / `= expression` must be present: `cell width;` alone is a
syntax error ("expected `:` or `=` in cell declaration"). A `cell` may also carry an optional
trailing `filter` clause and/or `require` block — a standing domain constraint and named
boolean diagnostics, respectively, both covered in full in the [filters chapter](filters.md) and
[requirements: diagnostics, not gates](outputs.md#requirements-diagnostics-not-gates) (those two chapters introduce the
mechanisms via `out`, but both apply to a plain `cell` — and, `require` only, to a
[`source`](source.md) cell — exactly the same way).

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

When a cell has an initializer but no `: type_expr` annotation, its type is inferred from the
initializer expression's own result type (an ordinary CEL literal-defaulting rule: an
unsuffixed integer literal like `0` is `i32`, an unsuffixed float literal like `0.0` is `f64`;
see `cel-parser`'s documentation for the full literal grammar). When both are present, they
must agree exactly, or the sheet fails to parse with a "type mismatch: expected `T`, got `U`" error (see [error messages](reference.md#error-messages)).

A host application can also register additional Rust types under their own Adam type
names; that's a Rust-level embedding concern, not something a sheet author does; see
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

A parenthesized type list is a tuple type. `()` is the empty tuple (an inert, zero-element
value; see the [relationships chapter](relationships.md) for where it's useful); `(T)` with no comma is plain
grouping, identical to `T` (types have no precedence to disambiguate, but the parentheses are
accepted for symmetry with expression grammar); `(T,)` (trailing comma mandatory) is a
genuine one-element tuple; `(T, U, ...)` is the general case:

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
rejects a callable cell when its descriptor lacks equality. Adam does not invent function
identity or suppress propagation. Registered host function markers retain the concrete
capabilities their type registration supplies.

## Executable host boundary

`CellValue` is Adam's private whole-value compound owner, not a wrapper around
each array element. A host supplies `ArrayOf`, `Cons`, `Nil`, and `Leaf` witnesses
to the checked `ParsedSheet` boundary. A wrong nested shape fails before mutation.
Cloning and equality are required for persistence/propagation; debug formatting
is an independent capability. Containers themselves do not impose those bounds.
Even an empty array of callable elements lacks equality when its element
descriptor lacks equality; emptiness does not manufacture that capability.

This example defaults recursive compounds, writes an array of concrete tuple
lists, propagates it to an output, and reads the source aspiration:

```rust
{{#include ../tests/composable_containers.rs:compound-cells}}
```

The corresponding capability error is also executable:

```rust
{{#include ../tests/composable_containers.rs:callable-cell-capability}}
```

The Inspector formats arrays and tuples recursively when debug capabilities
exist, including typed empty arrays and unit. Compound edits are explicitly
unsupported and leave the cell unchanged; primitive editors retain their
ordinary behavior. Formatting does not imply an editable source round trip.

## Names and declaration order

A cell's name must be unique across the whole sheet: `cell`s and [`out`](outputs.md)
declarations share one namespace, so declaring `cell result: i32 = 0;` and later
`out result := ...;` in the same sheet is a duplicate-name error, exactly as two `cell result`
declarations would be.

Referencing a name before its declaration — as a `relationship` binding's output, a dependency
inside any expression, a `conditional`'s match subject, or a `filter`'s dependency — is an
"undeclared cell `name`" error (see [error messages](reference.md#error-messages)).
