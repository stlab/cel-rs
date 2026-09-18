# Standard library

CEL keeps the core expression language small. The standard library adds
operations such as numeric functions to the core environment. When an
evaluation environment installs the library, its operations behave like
ordinary CEL calls and produce ordinary CEL values.

If an evaluation environment does not install the library, the operations in
this chapter are unavailable there. The core language described elsewhere in
the book still works unchanged.

## Syntax

```text
standard_library_call = identifier "(" [ argument_list ] ")" .
argument_list = expression { "," expression } .
```

## Availability at a glance

| Library | Functions | Availability |
| --- | --- | --- |
| Standard library | `round`, `min`, `max`, `clamp`, `abs`, `signum`, `sqrt`, `floor`, `ceil`, `trunc` | Available when the evaluation environment installs the library. |

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

`round` is a standard-library operation.

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

`min` and `max` are standard-library operations.

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

`clamp` is a standard-library operation.

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

These are standard-library operations.

`abs(x)`:

- accepts signed integers and floating-point values
- returns the same type as `x`
- reports `arithmetic overflow` for the minimum signed integer value
- succeeds for every other supported operand

`signum(x)`:

- accepts signed integers and floating-point values
- returns the same type as `x`
- succeeds for every supported operand

```text
abs(-42i64)
signum(-7i32)
signum(-3.5)
```

## `sqrt(x)`, `floor(x)`, `ceil(x)`, and `trunc(x)`

These are standard-library operations.

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

- Standard-library functions use ordinary CEL call syntax.
- `min`, `max`, `clamp`, `abs`, `signum`, `sqrt`, `floor`, `ceil`, and
  `trunc` belong to the standard library.
- Standard-library operations are available only when the evaluation
  environment installs the library.
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
