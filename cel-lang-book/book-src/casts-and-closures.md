# Casts and closures

CEL uses `as` for explicit conversion and `|...|` for closure literals.
Both forms are expressions and can appear anywhere an expression is
expected. For the exact grammar, see the [Reference Manual](reference.md).

## Concept

A cast converts a value to one of the built-in scalar types supported by
the language. A closure packages a parameter list and a single expression
body into a callable value.

## Syntax

```text
expression as Type
expression as Type as OtherType
|| expression
|x: Type| expression
|x: Type, y: Type| expression
|pair: (Type, Type)| expression
```

## Worked examples

```text
1.5 as i32
true as i64
|x: i32| x + 1
|pair: (i32, i32)| pair.0 + pair.1
|| 1..=5
```

## Exact rules

Casts:

- `as` associates from left to right, so `x as T as U` applies the first
  cast before the second one.
- The built-in scalar type names are `i8`, `i16`, `i32`, `i64`, `i128`,
  `isize`, `u8`, `u16`, `u32`, `u64`, `u128`, `usize`, `f32`, `f64`,
  `bool`, and `String`.
- Any integer target accepts integer, floating-point, and `bool` source
  values.
- `f32` and `f64` targets accept integer and floating-point source
  values.
- `bool` casts only to `bool`, and `String` casts only to `String`.
- Integer-to-integer casts check that the source fits in the target.
- Floating-point to integer casts require a finite, in-range source and
  truncate toward zero.
- `f64 as f32` checks finite range before narrowing.
- `true` casts to integer `1`, and `false` casts to integer `0`.

Closures:

- `|| body` declares a zero-parameter closure.
- `|name: Type| body` declares one typed parameter.
- `|a: T, b: U| body` declares multiple typed parameters in source order.
- A closure parameter type is either a built-in scalar name or a
  parenthesized tuple type built recursively from closure parameter
  types.
- `|pair: (i32, i32)| pair.0 + pair.1` declares one tuple-typed
  parameter, not two separate parameters.
- The closure body is one expression.
- A closure body resolves its own parameters and does not capture free
  variables from an enclosing expression.

## Edge cases

- Cast targets are limited to the sixteen built-in scalar type names.
- `char`, byte-string, C-string, unit, array, tuple, range, and closure
  values are not additional cast target names.
- Number-to-`bool`, `bool`-to-float, and `String`-to-number or
  `String`-to-`bool` casts are not valid.
- Closure parameter lists do not accept a trailing comma.
- `(i32,)` is not accepted as a one-element closure tuple type.
- Closure bodies do not introduce statement blocks or local binding
  syntax.
