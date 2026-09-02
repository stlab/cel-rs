# Chapter 2: Sheets, Cells, and Types

## 2.1 Sheets

Every Adam source file is one sheet:

```text
sheet name {
    /* cell, relationship, conditional, and out declarations, in any order,
       except that each identifier must be declared before it is referenced —
       see 2.6 below. */
}
```

The sheet's own name (`name` above) is consumed by the parser and is not otherwise meaningful
to `adam-rs`; it exists for readability and for host tooling, such as a language server or an
example picker, to have something to display.

## 2.2 Cell declarations

```text
cell_decl      = "cell" identifier cell_type_init [ cell_filter ] [ require_block ] ";".
cell_type_init = (":" type_expr ["=" expression]) | ("=" expression).
```

A cell needs a type, an initial value, or both:

```text
cell width: i32;             // type only — needs a registered default (2.4)
cell height: i32 = 1080;     // type and initializer
cell area = 0;               // initializer only — type is inferred (2.3)
```

At least one of `: type_expr` / `= expression` must be present: `cell width;` alone is a
syntax error ("expected `:` or `=` in cell declaration"). A `cell` may also carry an optional
trailing `filter` clause and/or `require` block — a standing domain constraint and named
boolean diagnostics, respectively, both covered in full in [Chapter 7](filters.md) and
[Chapter 8](outputs.md#83-requirements-diagnostics-not-gates) (those two chapters introduce the
mechanisms via `out`, but both apply to a plain `cell` — and, `require` only, to a
[`source`](source.md) cell — exactly the same way).

A cell's initializer is evaluated **once**, eagerly, at parse time; it may reference literals
and CEL operators, but not other cells (there's no "current sheet state" yet for it to read).
To compute one cell from others, use a [`relationship`](relationships.md) or an
[`out`](outputs.md) declaration instead.

## 2.3 Built-in types and inference

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
must agree exactly, or the sheet fails to parse:

```
{{#include examples/cells/type_mismatch_is_a_parse_error.adm2}}
```

A host application can also register additional Rust types under their own Adam type
names; that's a Rust-level embedding concern, not something a sheet author does; see
[Appendix A.5](reference.md#a5-the-type-registry).

## 2.4 Cells with no default

A type registered without a `Default` (via the embedding API's `register_no_default`) can
still be used for a cell, but only with an initializer; declaring one with a bare `: T` and no
`= ...` fails to parse ("type `T` has no default; provide `= ...`"). Every built-in type in the
table above has a default, so this only matters for a host-registered custom type.

## 2.5 Tuple types

```text
type_expr = identifier | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".
```

A parenthesized type list is a tuple type. `()` is the empty tuple (an inert, zero-element
value; see [Chapter 5](relationships.md) for where it's useful); `(T)` with no comma is plain
grouping, identical to `T` (types have no precedence to disambiguate, but the parentheses are
accepted for symmetry with expression grammar); `(T,)` (trailing comma mandatory) is a
genuine one-element tuple; `(T, U, ...)` is the general case:

```
{{#include examples/cells/tuple_typed_cell.adm2}}
```

Every tuple shape (regardless of arity or element types) shares the same underlying storage
type, `cel_runtime::DynamicSequence`; that's the type to `read`/`write` a tuple-typed cell with
from host code. Tuples nest: `(i32, (f64, String))` is a 2-tuple whose second element is itself
a 2-tuple.

## 2.6 Names and declaration order

A cell's name must be unique across the whole sheet: `cell`s and [`out`](outputs.md)
declarations share one namespace, so declaring `cell result: i32 = 0;` and later
`out result := ...;` in the same sheet is a duplicate-name error, exactly as two `cell result`
declarations would be.

Adam has **no forward references and no hoisting**: a cell must be declared before
anything else in the sheet mentions its name, whether as a `relationship` binding's output, a
dependency inside any expression, a `conditional`'s match subject, or a `filter`'s dependency.
Declaration order matters for more than readability: it determines name resolution and, as
[Chapter 5](relationships.md) covers, the solver's initial notion of which cells are "fresher"
than others.

```
{{#include examples/cells/no_forward_references.adm2}}
```
