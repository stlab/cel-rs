# Standard library

These numeric operations are opt-in: the evaluation environment must install
`cel-std` with `cel_std::install` on its operation lookup. Without that
installation, the operations in this chapter are unavailable; the core
language still works unchanged. Installed operations support both ordinary
and receiver calls: `sqrt(x)` and `x.sqrt()` select the same operation.
The receiver supplies the first argument, so `atan2(y, x)` is also `y.atan2(x)`.

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
Scalar return types match the floating-point or numeric operand type; even
rounded results remain floats. `powi` takes an `i32` exponent, and `sin_cos`
returns a tuple whose two elements have the input float type.
Unsuffixed floating-point literals in the examples have type `f64`.

`NaN` and infinity below describe floating-point values, not CEL literal
spellings. Operations that produce `NaN` do not report an evaluation error
unless an entry explicitly says otherwise. NaN payloads and signs are not
promised. Float operations delegate to the corresponding Rust `f32` or `f64`
method. Transcendental results have Rust's unspecified precision, which can
vary across platforms and Rust versions; compare approximate results with a
tolerance. Each operation takes O(1) time and space.

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
Unlike [`round`](#round), this operation resolves halfway cases toward the
even neighbor rather than away from zero.

### Complexity

O(1) time and space.

### Example

```text
round_ties_even(2.5)
```

Returns `2.0` (`f64`), because `2` is the even neighbor.

### See also

[`floor`](#floor), [`ceil`](#ceil), [`trunc`](#trunc).

## `round` {#round}

```text
round(x: F) -> F
```

Rounds `x` to the nearest integer, resolving halfway cases away from zero.

### Return value

The rounded value in the input float type. NaN produces NaN, infinities
remain unchanged, and signed zeros retain their signs. Negative inputs
that round to zero produce `-0.0`.

### Example

```text
round(2.5)
```

Returns `3.0` (`f64`). In contrast, `round_ties_even(2.5)` returns `2.0`.

### See also

[`round_ties_even`](#round-ties-even), [`floor`](#floor), [`ceil`](#ceil).

## `sin` {#sin}

```text
sin(x: F) -> F
```

Computes the sine of the angle `x` in radians.

### Return value

The sine in the input float type. Infinite or NaN input produces NaN;
signed zeros retain their signs.

### Example

```text
(30.0).to_radians().sin()
```

Returns approximately `0.5` (`f64`).

### See also

[`cos`](#cos), [`sin_cos`](#sin-cos), [`to_radians`](#to-radians).

## `cos` {#cos}

```text
cos(x: F) -> F
```

Computes the cosine of the angle `x` in radians.

### Return value

The cosine in the input float type. Infinite or NaN input produces NaN.
Either signed zero produces `1.0`.

### Example

```text
cos(0.0f32)
```

Returns `1.0f32`.

### See also

[`sin`](#sin), [`sin_cos`](#sin-cos).

## `tan` {#tan}

```text
tan(x: F) -> F
```

Computes the tangent of the angle `x` in radians.

### Return value

The tangent in the input float type. Infinite or NaN input produces NaN;
signed zeros retain their signs. Results near odd multiples of `pi / 2`
are sensitive to the represented input angle.

### Example

```text
tan(0.0)
```

Returns `0.0` (`f64`).

### See also

[`atan`](#atan), [`to_radians`](#to-radians).

## `asin` {#asin}

```text
asin(x: F) -> F
```

Computes the inverse sine of `x`.

### Return value

An angle in radians between `-pi / 2` and `pi / 2`, in the input float type.
Inputs outside `[-1, 1]`, including infinities, produce NaN. NaN produces
NaN; signed zeros retain their signs.

### Example

```text
asin(0.5).to_degrees()
```

Returns approximately `30.0` (`f64`).

### See also

[`sin`](#sin), [`acos`](#acos), [`atan`](#atan).

## `acos` {#acos}

```text
acos(x: F) -> F
```

Computes the inverse cosine of `x`.

### Return value

An angle in radians between `0` and `pi`, in the input float type.
Inputs outside `[-1, 1]`, including infinities, produce NaN. NaN produces NaN.

### Example

```text
acos(0.5).to_degrees()
```

Returns approximately `60.0` (`f64`).

### See also

[`cos`](#cos), [`asin`](#asin).

## `atan` {#atan}

```text
atan(x: F) -> F
```

Computes the inverse tangent of `x`.

### Return value

An angle in radians between `-pi / 2` and `pi / 2`, in the input float type.
Positive and negative infinity approach the corresponding signed endpoint.
NaN produces NaN; signed zeros retain their signs.

### Example

```text
atan(1.0).to_degrees()
```

Returns approximately `45.0` (`f64`).

### See also

[`tan`](#tan), [`atan2`](#atan2).

## `atan2` {#atan2}

```text
atan2(y: F, x: F) -> F
```

Computes the angle of the vector `(x, y)`, retaining its quadrant.

### Parameters

| Parameter | Description |
| --- | --- |
| `y` | The vertical coordinate, also the receiver in `y.atan2(x)`. |
| `x` | The horizontal coordinate of the same float type. |

### Return value

An angle in radians between `-pi` and `pi`, in the operand float type.
Signed zeros and infinities follow Rust's `atan2` semantics; a NaN
coordinate produces NaN. For negative `x`, `+0.0` and `-0.0` values of
`y` select the positive and negative sides of the branch cut respectively.

### Example

```text
atan2(1.0, -1.0).to_degrees()
```

Returns approximately `135.0` (`f64`).

### See also

[`atan`](#atan), [`hypot`](#hypot), [`to_degrees`](#to-degrees).

## `sin_cos` {#sin-cos}

```text
sin_cos(x: F) -> (F, F)
```

Computes the sine and cosine of the angle `x` in radians.

### Return value

A CEL tuple ordered as `(sin(x), cos(x))`, with both elements in the input
float type. The tuple supports `.0` and `.1`, destructuring, and tuple-typed
arguments. Infinite or NaN input produces two NaNs. At signed zero,
the sine retains the zero's sign and the cosine is `1.0`.

### Example

```text
sin_cos((30.0).to_radians()).0 * 10.0
```

Returns approximately `5.0` (`f64`), the vertical coordinate of a point
10 units from the origin at 30 degrees.

### See also

[`sin`](#sin), [`cos`](#cos), [`to_radians`](#to-radians).

## `to_radians` {#to-radians}

```text
to_radians(degrees: F) -> F
```

Converts an angle in degrees to radians without wrapping it.

### Return value

The angle in radians in the input float type. Signed zeros retain their
signs; infinities retain their signs, and NaN produces NaN.

### Example

```text
to_radians(180.0)
```

Returns approximately `3.141592653589793` (`f64`).

### See also

[`to_degrees`](#to-degrees), [`sin`](#sin).

## `to_degrees` {#to-degrees}

```text
to_degrees(radians: F) -> F
```

Converts an angle in radians to degrees without wrapping it.

### Return value

The angle in degrees in the input float type. Signed zeros retain their
signs; infinities retain their signs, and NaN produces NaN. A sufficiently
large finite input can overflow to infinity.

### Example

```text
to_radians(180.0).to_degrees()
```

Returns approximately `180.0` (`f64`).

### See also

[`to_radians`](#to-radians), [`atan2`](#atan2), [`rem_euclid`](#rem-euclid).

## `hypot` {#hypot}

```text
hypot(x: F, y: F) -> F
```

Computes the length of a vector with same-type coordinates `x` and `y`.

### Return value

The nonnegative hypotenuse, mathematically `sqrt(x * x + y * y)`, in the
operand float type. An infinite operand produces positive infinity even
if the other operand is NaN. Otherwise, a NaN operand produces NaN.

### Example

```text
hypot(3.0f32, 4.0f32)
```

Returns `5.0f32`.

### See also

[`sqrt`](#sqrt), [`atan2`](#atan2).

## `cbrt` {#cbrt}

```text
cbrt(x: F) -> F
```

Computes the real cube root of `x`, including negative inputs.

### Return value

The cube root in the input float type. Signed zeros and infinities retain
their signs; NaN produces NaN.

### Example

```text
cbrt(-8.0)
```

Returns approximately `-2.0` (`f64`).

### See also

[`sqrt`](#sqrt), [`powf`](#powf).

## `powi` {#powi}

```text
powi(x: F, exponent: i32) -> F
```

Raises `x` to an integer power. The exponent is always `i32`, including
when `x` is `f64`; other exponent types have no matching overload.

### Return value

The power in the base's float type. Negative exponents compute reciprocal
powers. Negative bases are supported. A zero exponent returns `1.0`,
including for a NaN base. Overflow and division by zero follow Rust's
floating-point behavior rather than reporting evaluation errors.

### Example

```text
powi(2.0f32, -3i32)
```

Returns `0.125f32`.

### See also

[`powf`](#powf), [`recip`](#recip).

## `powf` {#powf}

```text
powf(x: F, exponent: F) -> F
```

Raises `x` to a same-type floating-point power.

### Return value

The power in the operand float type. A finite negative base with a finite
nonintegral exponent produces NaN. Other special values, overflow, and
underflow follow Rust's `powf` semantics without evaluation errors.

### Example

```text
powf(0.25, 0.5)
```

Returns approximately `0.5` (`f64`), a power curve suitable for adjusting
normalized values.

### See also

[`powi`](#powi), [`exp`](#exp), [`sqrt`](#sqrt).

## `exp` {#exp}

```text
exp(x: F) -> F
```

Computes `e` raised to `x`.

### Return value

The natural exponential in the input float type. Negative infinity
produces `0.0`, positive infinity produces positive infinity, and NaN
produces NaN. Finite inputs can underflow to zero or overflow to infinity.

### Example

```text
exp(1.0)
```

Returns approximately `2.718281828459045` (`f64`).

### See also

[`ln`](#ln), [`exp2`](#exp2), [`powf`](#powf).

## `exp2` {#exp2}

```text
exp2(x: F) -> F
```

Computes `2` raised to `x`.

### Return value

The base-two exponential in the input float type. Negative infinity
produces `0.0`, positive infinity produces positive infinity, and NaN
produces NaN. Finite inputs can underflow to zero or overflow to infinity.

### Example

```text
exp2(3.0)
```

Returns `8.0` (`f64`).

### See also

[`log2`](#log2), [`exp`](#exp).

## `ln` {#ln}

```text
ln(x: F) -> F
```

Computes the natural logarithm of `x`.

### Return value

The logarithm in the input float type. Either signed zero produces
negative infinity; negative values produce NaN. Positive infinity remains
positive infinity, and NaN produces NaN.

### Example

```text
ln(2.0)
```

Returns approximately `0.6931471805599453` (`f64`).

### See also

[`exp`](#exp), [`log`](#log), [`log2`](#log2), [`log10`](#log10).

## `log` {#log}

```text
log(x: F, base: F) -> F
```

Computes the logarithm of `x` in a same-type base.

### Parameters

| Parameter | Description |
| --- | --- |
| `x` | The value whose logarithm is computed. |
| `base` | The logarithm base, normally positive and unequal to `1.0`. |

### Return value

The logarithm in the operand float type, following Rust's `log` semantics.
Nonpositive values and invalid bases can produce NaN or infinity rather
than evaluation errors; the operation does not validate the mathematical
domain. Use the specialized logarithms for bases `e`, `2`, or `10`.

### Example

```text
log(8.0, 2.0)
```

Returns approximately `3.0` (`f64`).

### See also

[`ln`](#ln), [`log2`](#log2), [`log10`](#log10).

## `log2` {#log2}

```text
log2(x: F) -> F
```

Computes the base-two logarithm of `x`.

### Return value

The logarithm in the input float type. Either signed zero produces
negative infinity; negative values produce NaN. Positive infinity remains
positive infinity, and NaN produces NaN.

### Example

```text
log2(8.0)
```

Returns `3.0` (`f64`).

### See also

[`exp2`](#exp2), [`ln`](#ln), [`log`](#log).

## `log10` {#log10}

```text
log10(x: F) -> F
```

Computes the base-ten logarithm of `x`.

### Return value

The logarithm in the input float type. Either signed zero produces
negative infinity; negative values produce NaN. Positive infinity remains
positive infinity, and NaN produces NaN.

### Example

```text
log10(100.0)
```

Returns `2.0` (`f64`).

### See also

[`powf`](#powf), [`ln`](#ln), [`log`](#log).

## `recip` {#recip}

```text
recip(x: F) -> F
```

Computes the reciprocal `1 / x`.

### Return value

The reciprocal in the input float type. Signed zeros produce infinities
of the same sign, infinities produce zeros of the same sign, and NaN
produces NaN. These results are not evaluation errors.

### Example

```text
recip(4.0)
```

Returns `0.25` (`f64`).

### See also

[`powi`](#powi).

## `mul_add` {#mul-add}

```text
mul_add(x: F, multiplier: F, addend: F) -> F
```

Computes `x * multiplier + addend` with one rounding step.

### Return value

The fused multiply-add result in the operand float type. Unlike separate
`*` and `+` operations, the intermediate product is not rounded.
Special values follow Rust's `mul_add` semantics without evaluation errors.

### Example

```text
mul_add(0.25, 8.0, 2.0)
```

Returns `4.0` (`f64`), interpolating one quarter of an 8-unit span starting
at `2.0`.

### See also

[`clamp`](#clamp).

## `rem_euclid` {#rem-euclid}

```text
rem_euclid(x: F, divisor: F) -> F
```

Computes the least nonnegative remainder, useful for wrapping angles or
periodic coordinates. The divisor has the same float type as `x`.

### Return value

The Euclidean remainder in the operand float type. The mathematical range
is `[0, abs(divisor))`, but floating-point rounding can produce
`abs(divisor)`. A zero or NaN divisor, a NaN dividend, or an infinite
dividend produces NaN rather than an evaluation error.

### Example

```text
rem_euclid(-450.0, 360.0)
```

Returns `270.0` (`f64`).

### See also

[`to_degrees`](#to-degrees), [`fract`](#fract).

## `copysign` {#copysign}

```text
copysign(magnitude: F, sign: F) -> F
```

Combines the magnitude of the first operand with the sign bit of the second.

### Return value

The signed magnitude in the operand float type. The sign bit is copied
even from negative zero or NaN; a NaN magnitude remains NaN.

### Example

```text
copysign(2.0, -1.0)
```

Returns `-2.0` (`f64`).

### See also

[`abs`](#abs), [`signum`](#signum).
