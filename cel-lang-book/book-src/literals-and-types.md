# Literals and types

CEL source includes literal values for numbers, booleans, text, characters, and
bytes. It also uses tuple, array, range, and closure syntax to build larger
values from expressions. For the full grammar of these forms, see the
[Reference Manual](reference.md).

## Built-in scalar type names

These sixteen names are the built-in scalar type names used by `as` casts and
typed closure parameters.

| Category | Types |
| --- | --- |
| Signed integers | `i8`, `i16`, `i32`, `i64`, `i128`, `isize` |
| Unsigned integers | `u8`, `u16`, `u32`, `u64`, `u128`, `usize` |
| Floating-point numbers | `f32`, `f64` |
| Other scalars | `bool`, `String` |

The language also accepts character, byte-string, C-string, and unit values in
expressions. The named scalar type set remains the sixteen type names listed
above.

## Syntax

```text
literal = integer_literal
        | floating_point_literal
        | boolean_literal
        | string_literal
        | character_literal
        | byte_literal
        | byte_string_literal
        | c_string_literal .
```

## Integer literals

An integer literal is a whole number optionally followed by one of the accepted
integer suffixes.

| Suffix | Resulting type |
| --- | --- |
| none | `i32` |
| `i8` | `i8` |
| `i16` | `i16` |
| `i32` | `i32` |
| `i64` | `i64` |
| `i128` | `i128` |
| `isize` | `isize` |
| `u8` | `u8` |
| `u16` | `u16` |
| `u32` | `u32` |
| `u64` | `u64` |
| `u128` | `u128` |
| `usize` | `usize` |

Unary `-` combines with a numeric literal to form a negative numeric
expression.

```text
0
42i8
42i64
42u16
42u128
42usize
-1i32
```

## Floating-point literals

A floating-point literal uses decimal or exponent notation, optionally followed
by a floating-point suffix.

| Suffix | Resulting type |
| --- | --- |
| none | `f64` |
| `f64` | `f64` |
| `f32` | `f32` |

```text
3.5
1e3
0.25f64
6.022e23f32
-3.5
```

## Boolean, string, character, byte, and unit values

Boolean literals are `true` and `false`. String literals produce `String`
values. Character literals produce `char` values. Byte literals produce `u8`
values. Byte-string and C-string literals produce byte-oriented string values.
Write the unit value as `()`.

```text
true
false
"hello"
'a'
b'A'
b"bytes"
c"header"
()
```

A byte literal such as `b'A'` and an explicitly suffixed integer such as `65u8`
both denote `u8` values.

## Compound values

CEL also builds compound values from ordinary expressions:

- `()` is the unit value, `(expr)` is grouping, `(expr,)` is a 1-tuple, and
  `(a, b, c)` is a tuple.
- Arrays are homogeneous bracketed lists. An unannotated array is non-empty;
  a type annotation such as `[]: [i32]` supplies the element type for an empty
  array.
- `|| expr` and `|x: T| expr` are closure values.
- These forms can nest inside calls, tuples, arrays, conditionals, and one
  another wherever the grammar permits.
- Array homogeneity compares complete recursive shapes, including every tuple
  field and callable parameter/result. `[]: [(i32, [f64])]` and
  `[]: [Fn((i32,), [i32])]` need no registration of those aggregate shapes.

```text
(1,)
(1, 2, 3)
[0, 1, 2]
[0, 1]: [i32]
[]: [i32]
|x: i32| x + 1
```

Source `()` and the empty tuple type are one canonical type, witnessed by `Nil`
and represented by `CNil<()>`. Bridge ordinary Rust `()` with the zero-element
`IntoCStackList` / `IntoRustTuple` conversion; it is not a separate source leaf
that merely shares a zero-sized layout. An explicitly registered opaque host
`Leaf<()>` remains a distinct host identity.

```rust
{{#include ../tests/composable_containers.rs:source-unit}}
```

## Host-registered records

A host can register a named record schema and operations without changing CEL.
Write `Sample { value: 3.0, }` to construct a record when the host supplies that
schema. Field labels select schema fields rather than variables; values evaluate
in written order. Supply every field exactly once. Empty schemas accept `{}`,
and non-empty field lists accept a trailing comma. CEL reports unknown
constructors, unknown/duplicate/missing fields, and mismatched field types.

Records use exact scalar types: `1` is `i32`, never implicitly `f64`; use `1.0`
or an explicit supported cast for an `f64` field. Constructor names occupy a
separate namespace from values, so a value named `Sample` does not hide the
constructor. The host must separately register any operation such as `.value`;
a record schema does not automatically create accessors.

This executable embedding example registers a real Rust value and accessor.
Both bare and parenthesized member spellings return `3.0`; calling that numeric
result with `(record.value)()` fails.

```rust
{{#include ../tests/examples.rs:registered-record}}
```

## Range values

A range expression produces a range value from zero, one, or two numeric
endpoints. CEL supports bounded, half-bounded, and unbounded forms:

```text
a..b
a..=b
a..
..b
..=b
..
```

Endpoint-bearing ranges require homogeneous numeric endpoints. Each endpoint
is a full expression, so operators inside either side are parsed before the
range is formed. The [Operators](operators.md) chapter defines precedence and
the non-chaining rule for range expressions.

Use this chapter for value categories and [Reference Manual](reference.md)
for the complete grammar of literal, tuple, array, range, and closure syntax.
