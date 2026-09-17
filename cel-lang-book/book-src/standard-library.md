# Standard library

CEL keeps the core expression language small. Numeric helper calls come in two
layers: `round` is part of the core environment, and some evaluation
environments also install an optional standard library that adds more numeric
functions. When that library is present, its calls behave like ordinary CEL
calls and produce ordinary CEL values.

If an evaluation environment does not install the optional library, the
optional function names in this chapter are simply unavailable there. The core
language described elsewhere in the book still works unchanged.

## Availability at a glance

| Layer | Functions | Availability |
| --- | --- | --- |
| Core environment | `round` | Available in the default core environment. |
| Optional standard library | `min`, `max`, `clamp`, `abs`, `signum`, `sqrt`, `floor`, `ceil`, `trunc` | Available when the evaluation environment installs the optional library. |

## Worked examples

```text
round(3.5)
min(3i32, -5i32)
max(3.5f64, 2.5f64)
clamp(-2i32, 0i32, 10i32)
abs(-42i64)
signum(-3.5)
sqrt(9.0f32)
floor(-3.2)
ceil(-3.2)
trunc(-3.2)
```

## Numeric domains

None of the documented functions are integer-only.

- `round`, `sqrt`, `floor`, `ceil`, and `trunc` are float-only.
- `min`, `max`, and `clamp` support both integers and floats.
- `abs` and `signum` support signed integers and floats, but not unsigned
  integers.

`min`, `max`, and `clamp` accept any one numeric type from this set when all
operands match: `i8`, `i16`, `i32`, `i64`, `i128`, `isize`, `u8`, `u16`,
`u32`, `u64`, `u128`, `usize`, `f32`, `f64`. The result type is the same as
the operand type.

`abs` and `signum` accept `i8`, `i16`, `i32`, `i64`, `i128`, `isize`, `f32`,
and `f64`. `sqrt`, `floor`, `ceil`, and `trunc` accept `f32` and `f64`. Each
of those functions returns the same type it receives.

## `round(x)`

`round` is a core numeric call, not an optional-library addition.

- Accepted operand type: `f64`
- Result type: `f64`
- Semantics: rounds to the nearest integral value represented as `f64`
- Halfway values round away from zero
- Successful calls are infallible

```text
round(3.5)
round(-3.5)
```

## `min(a, b)` and `max(a, b)`

`min` and `max` are optional-library calls.

- Both operands must have the same numeric type.
- Supported domains: all signed integers, all unsigned integers, `f32`, and
  `f64`.
- Result type: the same type as both operands.
- Successful same-type calls are infallible.

```text
min(3i32, -5i32)
max(3u32, 5u32)
max(3.5f64, 2.5f64)
```

## `clamp(x, lo, hi)`

`clamp` is an optional-library call.

- `x`, `lo`, and `hi` must all have the same numeric type.
- Supported domains: all signed integers, all unsigned integers, `f32`, and
  `f64`.
- Result type: the same type as all three operands.
- Bounds must be ordered, so `lo <= hi`.
- Successful same-type calls with ordered bounds are infallible.

```text
clamp(-2i32, 0i32, 10i32)
clamp(12.5f64, 0.0f64, 10.0f64)
```

## `abs(x)` and `signum(x)`

These are optional-library calls.

`abs(x)`:

- accepts signed integers and floating-point values
- returns the same type as `x`
- reports `arithmetic overflow` for the minimum signed integer value
- is otherwise infallible after successful lookup and type matching

`signum(x)`:

- accepts signed integers and floating-point values
- returns the same type as `x`
- is infallible after successful lookup and type matching

```text
abs(-42i64)
signum(-7i32)
signum(-3.5)
```

## `sqrt(x)`, `floor(x)`, `ceil(x)`, and `trunc(x)`

These are optional-library calls.

- Supported domains: `f32` and `f64` only.
- Result type: the same floating-point type as the operand.
- `sqrt` follows normal floating-point behavior, so negative inputs yield `NaN`
  rather than an error.
- `floor` rounds toward negative infinity.
- `ceil` rounds toward positive infinity.
- `trunc` rounds toward zero.
- Successful calls are infallible.

```text
sqrt(9.0f32)
floor(-3.2)
ceil(-3.2)
trunc(-3.2)
```

## Exact rules

- Function-call syntax is the same ordinary CEL call syntax documented in
  [Expressions](expressions.md).
- `round` belongs to the core environment.
- `min`, `max`, `clamp`, `abs`, `signum`, `sqrt`, `floor`, `ceil`, and
  `trunc` belong to an optional library layered over the core expression
  language.
- Optional-library functions are available only when the evaluation environment
  installs that library.
- `min` and `max` require same-type numeric operands.
- `clamp` requires same-type operands and ordered bounds.
- `abs` does not accept unsigned integers.
- `signum` does not accept unsigned integers.
- `sqrt`, `floor`, `ceil`, and `trunc` do not accept integers.
- The documented successful calls are otherwise infallible except for `clamp`
  with unordered bounds and `abs` on the minimum signed integer.

## Edge cases

- `round(-3.5)` returns `-4.0`.
- `min` and `max` do not coerce mixed numeric types.
- `clamp` reports `invalid clamp bounds` when `lo <= hi` is false.
- `abs(-128i8)` reports `arithmetic overflow`.
- `sqrt(-1.0)` yields `NaN`.
