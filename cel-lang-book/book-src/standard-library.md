# Standard library

These numeric operations are opt-in: the evaluation environment must install
`cel-std` with `cel_std::install` on its operation lookup. Without that
installation, the operations in this chapter are unavailable; the core
language still works unchanged. Installed operations use ordinary CEL calls.

The descriptive signatures below use type-family shorthand, not CEL generics:

| Shorthand | Supported concrete types |
| --- | --- |
| `T` | `i8`, `i16`, `i32`, `i64`, `i128`, `isize`, `u8`, `u16`, `u32`, `u64`, `u128`, `usize`, `f32`, `f64` |
| `S` | `i8`, `i16`, `i32`, `i64`, `i128`, `isize`, `f32`, `f64` |
| `F` | `f32`, `f64` |

Each occurrence of a shorthand in one call denotes the **same concrete type**.
There is no implicit numeric conversion: for example, `min(1i32, 2i64)` has no
matching overload. Unsupported types and arities fail during expression
compilation, rather than producing the evaluation errors documented below.
Return types always match the operand type; even rounded results remain floats.
Unsuffixed floating-point literals in the examples have type `f64`.

`NaN` and infinity below describe floating-point values, not CEL literal
spellings. Operations that produce `NaN` do not report an evaluation error
unless an entry explicitly says otherwise. NaN payloads and signs are not
promised.

## `min` {#min}

```text
min(a: T, b: T) -> T
```

Selects the smaller of two same-type numeric values.

### Parameters

| Parameter | Description |
| --- | --- |
| `a`, `b` | Values to compare, both of the same type `T`. |

### Return value

The smaller operand. For floats, exactly one NaN operand causes the other
operand to be returned; two NaNs produce NaN. Infinities compare in the usual
order. Equal operands may yield either operand, so comparing `-0.0` and `+0.0`
does not guarantee a particular zero sign.

### Complexity

O(1) time and space.

### Example

```text
min(3i32, -5i32)
```

Returns `-5i32`.

### See also

[`max`](#max), [`clamp`](#clamp).

## `max` {#max}

```text
max(a: T, b: T) -> T
```

Selects the larger of two same-type numeric values.

### Parameters

| Parameter | Description |
| --- | --- |
| `a`, `b` | Values to compare, both of the same type `T`. |

### Return value

The larger operand. For floats, exactly one NaN operand causes the other
operand to be returned; two NaNs produce NaN. Infinities compare in the usual
order. Equal operands may yield either operand, so comparing `-0.0` and `+0.0`
does not guarantee a particular zero sign.

### Complexity

O(1) time and space.

### Example

```text
max(3.5f64, 2.5f64)
```

Returns `3.5f64`.

### See also

[`min`](#min), [`clamp`](#clamp).

## `clamp` {#clamp}

```text
clamp(x: T, lo: T, hi: T) -> T
```

Restricts a numeric value to an inclusive interval.

### Parameters

| Parameter | Description |
| --- | --- |
| `x` | Value to restrict. |
| `lo` | Inclusive lower bound, of the same type as `x`. |
| `hi` | Inclusive upper bound, of the same type as `x`. |

### Return value

`lo` if `x < lo`, `hi` if `x > hi`, otherwise `x`, all in the same type `T`.
Equal bounds are allowed. With valid floating-point bounds, a NaN `x` produces
NaN; infinite values and bounds follow the same comparisons. Because signed
zeros compare equal, an in-range zero retains the sign of `x`.

### Errors

Reports `invalid clamp bounds` if `lo <= hi` is false: either `lo > hi`, or
either floating-point bound is NaN. Bounds are checked even when `x` is NaN.

### Complexity

O(1) time and space.

### Example

```text
clamp(-2i32, 0i32, 10i32)
```

Returns `0i32`.

### See also

[`min`](#min), [`max`](#max).

## `abs` {#abs}

```text
abs(x: S) -> S
```

Computes an absolute value for a signed integer or float. Unsigned integers
are not supported.

### Parameters

| Parameter | Description |
| --- | --- |
| `x` | Signed integer or floating-point value of type `S`. |

### Return value

The magnitude of `x`, in the same type. Floating-point `-0.0` becomes `+0.0`,
either infinity becomes positive infinity, and NaN produces NaN.

### Errors

Reports `arithmetic overflow` for the minimum value of any supported signed
integer type, because its positive magnitude is not representable.

### Complexity

O(1) time and space.

### Example

```text
abs(-42i64)
```

Returns `42i64`.

### See also

[`signum`](#signum), [`clamp`](#clamp).

## `signum` {#signum}

```text
signum(x: S) -> S
```

Extracts a signed integer's or float's sign. Unsigned integers are not
supported.

### Parameters

| Parameter | Description |
| --- | --- |
| `x` | Signed integer or floating-point value of type `S`. |

### Return value

For integers, returns `-1`, `0`, or `1` according to whether `x` is negative,
zero, or positive, in the same integer type.

For floats, returns `-1.0` for negative values (including negative infinity
and `-0.0`), `1.0` for positive values (including positive infinity and
`+0.0`), or NaN for NaN. The result has the same floating-point type as `x`.
In particular, floating-point zero does **not** produce zero.

### Complexity

O(1) time and space.

### Examples

```text
signum(-7i32)
```

Returns `-1i32`.

```text
signum(-3.5)
```

Returns `-1.0` (`f64`).

### See also

[`abs`](#abs), [`min`](#min), [`max`](#max).

## `sqrt` {#sqrt}

```text
sqrt(x: F) -> F
```

Computes a floating-point square root. Integer operands are not supported.

### Parameters

| Parameter | Description |
| --- | --- |
| `x` | Floating-point value of type `F`. |

### Return value

The nonnegative square root of a nonnegative value, in the same type.
Negative values, including negative infinity, produce NaN rather than an
evaluation error. NaN produces NaN; positive infinity remains positive
infinity. Signed zeros retain their signs, so `sqrt(-0.0)` returns `-0.0`.

### Complexity

O(1) time and space.

### Example

```text
sqrt(9.0f32)
```

Returns `3.0f32`.

### See also

[`abs`](#abs), [`clamp`](#clamp).

## `floor` {#floor}

```text
floor(x: F) -> F
```

Rounds a float toward negative infinity. Integer operands are not supported.

### Parameters

| Parameter | Description |
| --- | --- |
| `x` | Floating-point value of type `F`. |

### Return value

The greatest integer less than or equal to `x`, represented in the same
floating-point type. Infinities are unchanged, NaN produces NaN, and signed
zeros retain their signs.

### Complexity

O(1) time and space.

### Example

```text
floor(-3.2)
```

Returns `-4.0` (`f64`).

### See also

[`ceil`](#ceil), [`trunc`](#trunc), [`round_ties_even`](#round-ties-even).

## `ceil` {#ceil}

```text
ceil(x: F) -> F
```

Rounds a float toward positive infinity. Integer operands are not supported.

### Parameters

| Parameter | Description |
| --- | --- |
| `x` | Floating-point value of type `F`. |

### Return value

The least integer greater than or equal to `x`, represented in the same
floating-point type. Infinities are unchanged, NaN produces NaN, and signed
zeros retain their signs. A value strictly between `-1.0` and `0.0` rounds
to `-0.0`.

### Complexity

O(1) time and space.

### Example

```text
ceil(-3.2)
```

Returns `-3.0` (`f64`).

### See also

[`floor`](#floor), [`trunc`](#trunc), [`round_ties_even`](#round-ties-even).

## `trunc` {#trunc}

```text
trunc(x: F) -> F
```

Rounds a float toward zero. Integer operands are not supported.

### Parameters

| Parameter | Description |
| --- | --- |
| `x` | Floating-point value of type `F`. |

### Return value

The integer part of `x`, represented in the same floating-point type.
Infinities are unchanged and NaN produces NaN. Signed zeros retain their
signs; a negative value strictly between `-1.0` and `0.0` becomes `-0.0`.

### Complexity

O(1) time and space.

### Example

```text
trunc(-3.2)
```

Returns `-3.0` (`f64`).

### See also

[`fract`](#fract), [`floor`](#floor), [`ceil`](#ceil).

## `fract` {#fract}

```text
fract(x: F) -> F
```

Extracts a float's fractional part. Integer operands are not supported.

### Parameters

| Parameter | Description |
| --- | --- |
| `x` | Floating-point value of type `F`. |

### Return value

`x - trunc(x)`, in the same floating-point type. For finite nonintegral
values the fraction has the sign of `x`, not necessarily a positive sign.
Finite integral values, including either signed zero, produce `+0.0`.
Either infinity or NaN produces NaN rather than an evaluation error.

### Complexity

O(1) time and space.

### Example

```text
fract(-3.75f32)
```

Returns `-0.75f32`.

### See also

[`trunc`](#trunc), [`floor`](#floor), [`ceil`](#ceil).

## `round_ties_even` {#round-ties-even}

```text
round_ties_even(x: F) -> F
```

Rounds a float to the nearest integer, resolving exact halfway cases toward
the even integer. Integer operands are not supported.

### Parameters

| Parameter | Description |
| --- | --- |
| `x` | Floating-point value of type `F`. |

### Return value

The nearest integer, represented in the same floating-point type.
Infinities are unchanged and NaN produces NaN. Signed zeros retain their
signs; a negative input that rounds to zero produces `-0.0`.
This operation is named `round_ties_even`; `round` is not provided.

### Complexity

O(1) time and space.

### Example

```text
round_ties_even(2.5)
```

Returns `2.0` (`f64`), because `2` is the even neighbor.

### See also

[`floor`](#floor), [`ceil`](#ceil), [`trunc`](#trunc).
