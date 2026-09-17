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
- Arrays are non-empty bracketed, homogeneous lists, and `[]` is not accepted.
- `a..b`, `a..=b`, `a..`, `..b`, `..=b`, and `..` are range values.
- `|| expr` and `|x: T| expr` are closure values.
- These forms can nest inside calls, tuples, arrays, conditionals, and one
  another wherever the grammar permits.

```text
(1,)
(1, 2, 3)
[0, 1, 2]
1..=5
..10
|x: i32| x + 1
```

Use this chapter for the value categories and [Reference Manual](reference.md)
for the detailed grammar of literal, tuple, array, range, and closure syntax.
