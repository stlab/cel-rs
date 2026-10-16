# Euclid library

The optional `cel-euclid` library provides concrete two-dimensional geometry
types and operations. A host installs it once on a fresh `OpLookup` with
`cel_euclid::install`. The core language does not install geometry implicitly.
Install [`cel-std`](standard-library.md) separately for scalar operations;
either installation order preserves both libraries' overloads.

## Conventions

Every coordinate, extent, angle component, and matrix coefficient has type
`f64`. The Rust types use Euclid's unknown units; CEL does not expose generic
unit parameters. Operand types must match the listed signatures exactly:
write `1.0`, `1f64`, or `1 as f64` where an operation requires `f64`.
There are no implicit numeric casts or return-type-only overloads.

Signatures use Rust-style parameter and return-type notation to describe CEL
calls, not to declare functions. `T` denotes only the concrete alternatives
listed with a signature; it is not CEL generic syntax.

Every named operation with a first argument also accepts receiver syntax:
`name(value, args...)` and `value.name(args...)` are equivalent. A unary
operation additionally accepts `value.name`. Only `identity_transform()`
has no receiver. Operators use their ordinary CEL spelling.

Except for [`normalize`](#normalize), [`inverse`](#inverse), and
[`outer_transformed_rect`](#outer_transformed_rect), operations preserve
Euclid's ordinary floating-point behavior. Overflow, division by zero,
NaNs, and infinities do not by themselves produce evaluation errors.
Record construction does not validate finiteness, order bounds, or require
nonnegative sizes.

## Types

Each record requires every listed field exactly once, in any order.
Fields must have the indicated types. Missing, repeated, unknown, or
incorrectly typed fields cause a parse error. Record construction takes
constant time and space.

### Point2D

A position with fields `x: f64` and `y: f64`.

```text
Point2D { x: 1.0, y: 2.0 }
// => Point2D { y: 2.0, x: 1.0 }
```

Adding a vector produces a point; subtracting two points produces a vector.
Access coordinates with [`x` and `y`](#x-y).

### Vector2D

A displacement with fields `x: f64` and `y: f64`.

```text
Vector2D { x: 3.0, y: 4.0 }
// => Vector2D { y: 4.0, x: 3.0 }
```

Vectors support arithmetic, [`dot`](#dot), [`cross`](#cross),
[`length`](#length), and [`normalize`](#normalize).

### Size2D

Two extents with fields `width: f64` and `height: f64`.
Construction permits zero and negative extents.

```text
Size2D { width: 4.0, height: 6.0 }
// => Size2D { height: 6.0, width: 4.0 }
```

Access extents with [`width` and `height`](#width-height).

### Rect

An axis-aligned rectangle with fields `origin: Point2D` and `size: Size2D`.
Its maximum coordinates are the origin coordinates plus the extents.
Construction does not reorder negative extents.

```text
Rect {
    origin: Point2D { x: 1.0, y: 2.0 },
    size: Size2D { width: 4.0, height: 6.0 }
}
// => Rect { size: Size2D { width: 4.0, height: 6.0 }, origin: Point2D { x: 1.0, y: 2.0 } }
```

See [`origin`](#origin), [`size`](#size), and [`contains`](#contains).

### Box2D

An axis-aligned box with fields `min: Point2D` and `max: Point2D`.
Construction stores the supplied endpoints without sorting them.

```text
Box2D {
    min: Point2D { x: 1.0, y: 2.0 },
    max: Point2D { x: 5.0, y: 8.0 }
}
// => Box2D { max: Point2D { x: 5.0, y: 8.0 }, min: Point2D { x: 1.0, y: 2.0 } }
```

See [`min` and `max`](#min-max), [`size`](#size), and [`contains`](#contains).

### Angle

An angle with one field, `radians: f64`. Construction does not wrap the
value to a particular interval.

```text
Angle { radians: 1.5 }
// => radians(1.5)
```

Use [`degrees`](#degrees) or [`radians`](#radians) to construct an angle
from a scalar or extract its magnitude.

### Transform2D

An affine transform with six `f64` fields:
`m11`, `m12`, `m21`, `m22`, `m31`, and `m32`.
It maps a point `(x, y)` to:

```text
x' = m11 * x + m21 * y + m31
y' = m12 * x + m22 * y + m32
```

```text
Transform2D {
    m11: 1.0, m12: 0.0,
    m21: 0.0, m22: 1.0,
    m31: 0.0, m32: 0.0
}
// => identity_transform()
```

Use [`then`](#then) to compose transforms. Vectors use only the linear
coefficients and ignore translation.

## Operators

### Equality: `==`, `!=`

```text
a: T == b: T -> bool
a: T != b: T -> bool
```

`T` is any one of the seven geometry types. Compares corresponding stored
components using `f64` equality; it does not compare geometric equivalence.

#### Parameters

| Parameter | Description |
| --- | --- |
| `a`, `b` | Geometry values of the same type. |

#### Return value

`==` returns `true` if all corresponding components compare equal.
`!=` returns its logical negation. A NaN component compares unequal even
to itself; positive and negative zero compare equal. Angles differing by
a full turn do not compare equal merely because they describe the same direction.

#### Complexity

Constant time and space.

#### Example

```text
Point2D { x: 1.0, y: 2.0 } == Point2D { y: 2.0, x: 1.0 }
// => true
```

#### See also

[`is_finite`](#is_finite).

### Addition: `+`

```text
point: Point2D + offset: Vector2D -> Point2D
a: Vector2D + b: Vector2D -> Vector2D
a: Size2D + b: Size2D -> Size2D
```

Adds corresponding components.

#### Parameters

| Parameter | Description |
| --- | --- |
| `point`, `offset` | A position and the displacement to add. |
| `a`, `b` | Two vectors or two sizes. |

#### Return value

A translated point, component-wise vector sum, or component-wise size sum.
Vector-plus-point and point-plus-point are not supported.

#### Complexity

Constant time and space.

#### Example

```text
Point2D { x: 1.0, y: 2.0 } + Vector2D { x: 3.0, y: 4.0 }
// => Point2D { x: 4.0, y: 6.0 }
```

#### See also

[`-`](#subtraction--), [`translate`](#translate).

### Subtraction: `-`

```text
a: Point2D - b: Point2D -> Vector2D
point: Point2D - offset: Vector2D -> Point2D
a: Vector2D - b: Vector2D -> Vector2D
a: Size2D - b: Size2D -> Size2D
```

Subtracts corresponding components.

#### Parameters

| Parameter | Description |
| --- | --- |
| `a`, `b` | Two points, two vectors, or two sizes. |
| `point`, `offset` | A position and the displacement to subtract. |

#### Return value

For two points, the displacement from `b` to `a`. Otherwise, a point,
vector, or size containing the component-wise differences.

#### Complexity

Constant time and space.

#### Example

```text
Point2D { x: 4.0, y: 6.0 } - Point2D { x: 1.0, y: 2.0 }
// => Vector2D { x: 3.0, y: 4.0 }
```

#### See also

[`+`](#addition-), [`distance_to`](#distance_to).

### Negation: unary `-`

```text
-v: Vector2D -> Vector2D
```

Negates both components of a vector.

#### Parameters

| Parameter | Description |
| --- | --- |
| `v` | The vector to negate. |

#### Return value

A vector with components `-v.x` and `-v.y`.

#### Complexity

Constant time and space.

#### Example

```text
-Vector2D { x: 3.0, y: -4.0 }
// => Vector2D { x: -3.0, y: 4.0 }
```

#### See also

[`-`](#subtraction--).

### Scaling: `*`, `/`

```text
value: T * factor: f64 -> T
value: T / factor: f64 -> T
```

`T` is `Vector2D` or `Size2D`. Multiplies or divides each component by a scalar.

#### Parameters

| Parameter | Description |
| --- | --- |
| `value` | A vector or size. |
| `factor` | The scalar multiplier or divisor. |

#### Return value

A vector or size containing the scaled components. Scalar-left products
and point scaling are not supported. Division by zero follows `f64`
arithmetic rather than producing an evaluation error.

#### Complexity

Constant time and space.

#### Example

```text
Vector2D { x: 3.0, y: 4.0 } * 2.0
// => Vector2D { x: 6.0, y: 8.0 }
```

```text
Size2D { width: 4.0, height: 6.0 } / 2.0
// => Size2D { width: 2.0, height: 3.0 }
```

#### See also

[`scale`](#scale), [`normalize`](#normalize).

## Component and bounds access

### `is_finite`

```text
is_finite(value: T) -> bool
```

`T` is any one of the seven geometry types. Tests whether all stored scalar
components are finite.

#### Parameters

| Parameter | Description |
| --- | --- |
| `value` | The geometry value to inspect. |

#### Return value

`true` if no stored component is NaN or infinite; otherwise `false`.
For `Rect`, this checks the origin and size, not the computed maximum
coordinates. It does not validate sizes, bounds ordering, or invertibility.

#### Complexity

Constant time and space.

#### Example

```text
(Vector2D { x: 1.0, y: 2.0 } / 0.0).is_finite
// => false
```

#### See also

[`normalize`](#normalize), [`inverse`](#inverse).

### `x`, `y`

```text
x(value: T) -> f64
y(value: T) -> f64
```

`T` is `Point2D` or `Vector2D`. Reads one coordinate or component.

#### Parameters

| Parameter | Description |
| --- | --- |
| `value` | A point or vector. |

#### Return value

The stored `x` or `y` component, respectively.

#### Complexity

Constant time and space.

#### Example

```text
(Point2D { x: 1.0, y: 2.0 }).y
// => 2.0
```

#### See also

[`origin`](#origin), [`min` and `max`](#min-max).

### `width`, `height`

```text
width(value: T) -> f64
height(value: T) -> f64
```

`T` is `Size2D`, `Rect`, or `Box2D`. Reads one extent.

#### Parameters

| Parameter | Description |
| --- | --- |
| `value` | A size, rectangle, or box. |

#### Return value

For sizes and rectangles, the corresponding stored size component.
For boxes, `max.x - min.x` or `max.y - min.y`. Negative extents are
returned unchanged rather than clamped to zero.

#### Complexity

Constant time and space.

#### Example

```text
(Size2D { width: 4.0, height: 6.0 }).width
// => 4.0
```

#### See also

[`size`](#size), [`min_x`, `min_y`, `max_x`, and `max_y`](#min_x-min_y-max_x-max_y).

### `origin`

```text
origin(rect: Rect) -> Point2D
```

Reads a rectangle's stored origin.

#### Parameters

| Parameter | Description |
| --- | --- |
| `rect` | The rectangle to inspect. |

#### Return value

The `origin` field, without normalizing the rectangle.

#### Complexity

Constant time and space.

#### Example

```text
(Rect {
    origin: Point2D { x: 1.0, y: 2.0 },
    size: Size2D { width: 4.0, height: 6.0 }
}).origin
// => Point2D { x: 1.0, y: 2.0 }
```

#### See also

[`size`](#size), [`min_x`, `min_y`, `max_x`, and `max_y`](#min_x-min_y-max_x-max_y).

### `size`

```text
size(value: Rect) -> Size2D
size(value: Box2D) -> Size2D
```

Returns both extents of a rectangle or box.

#### Parameters

| Parameter | Description |
| --- | --- |
| `value` | A rectangle or box. |

#### Return value

For a rectangle, its stored `size`. For a box, the width and height
computed by subtracting `min` from `max`.

#### Complexity

Constant time and space.

#### Example

```text
(Box2D {
    min: Point2D { x: 1.0, y: 2.0 },
    max: Point2D { x: 5.0, y: 8.0 }
}).size
// => Size2D { width: 4.0, height: 6.0 }
```

#### See also

[`width` and `height`](#width-height), [`origin`](#origin).

### `min`, `max`

```text
min(bounds: Box2D) -> Point2D
max(bounds: Box2D) -> Point2D
```

Reads a box's stored minimum or maximum point.

#### Parameters

| Parameter | Description |
| --- | --- |
| `bounds` | The box to inspect. |

#### Return value

The stored `min` or `max` field. These overloads do not compute a
component-wise minimum or maximum of two geometry values.

#### Complexity

Constant time and space.

#### Example

```text
(Box2D {
    min: Point2D { x: 1.0, y: 2.0 },
    max: Point2D { x: 5.0, y: 8.0 }
}).max
// => Point2D { x: 5.0, y: 8.0 }
```

#### See also

[`min_x`, `min_y`, `max_x`, and `max_y`](#min_x-min_y-max_x-max_y),
scalar [`min`](standard-library.md#min), scalar [`max`](standard-library.md#max).

### `min_x`, `min_y`, `max_x`, `max_y`

```text
min_x(value: T) -> f64
min_y(value: T) -> f64
max_x(value: T) -> f64
max_y(value: T) -> f64
```

`T` is `Rect` or `Box2D`. Reads one bound coordinate.

#### Parameters

| Parameter | Description |
| --- | --- |
| `value` | A rectangle or box. |

#### Return value

For rectangles, minimum coordinates are the origin components; maximum
coordinates add the corresponding size components. For boxes, these
operations read the components of the stored `min` or `max` point.
They do not reorder inverted bounds.

#### Complexity

Constant time and space.

#### Example

```text
(Rect {
    origin: Point2D { x: 1.0, y: 2.0 },
    size: Size2D { width: 4.0, height: 6.0 }
}).max_y
// => 8.0
```

#### See also

[`min` and `max`](#min-max), [`width` and `height`](#width-height).

### `center`

```text
center(value: T) -> Point2D
```

`T` is `Rect` or `Box2D`. Computes the center of the bounds.

#### Parameters

| Parameter | Description |
| --- | --- |
| `value` | A rectangle or box. |

#### Return value

The midpoint computed using Euclid's floating-point arithmetic, without
validating or normalizing the bounds.

#### Complexity

Constant time and space.

#### Example

```text
(Box2D {
    min: Point2D { x: 1.0, y: 2.0 },
    max: Point2D { x: 5.0, y: 8.0 }
}).center
// => Point2D { x: 3.0, y: 5.0 }
```

#### See also

[`origin`](#origin), [`min` and `max`](#min-max).

## Vector queries

### `dot`

```text
dot(a: Vector2D, b: Vector2D) -> f64
```

Computes the dot product of two vectors.

#### Parameters

| Parameter | Description |
| --- | --- |
| `a`, `b` | The vectors to multiply. |

#### Return value

`a.x * b.x + a.y * b.y`, subject to floating-point rounding.

#### Complexity

Constant time and space.

#### Example

```text
dot(Vector2D { x: 3.0, y: 4.0 }, Vector2D { x: 1.0, y: 2.0 })
// => 11.0
```

#### See also

[`cross`](#cross), [`square_length`](#square_length).

### `cross`

```text
cross(a: Vector2D, b: Vector2D) -> f64
```

Computes the scalar two-dimensional cross product.

#### Parameters

| Parameter | Description |
| --- | --- |
| `a`, `b` | The vectors, in operand order. |

#### Return value

`a.x * b.y - a.y * b.x`, subject to floating-point rounding.

#### Complexity

Constant time and space.

#### Example

```text
cross(Vector2D { x: 3.0, y: 4.0 }, Vector2D { x: 1.0, y: 2.0 })
// => 2.0
```

#### See also

[`dot`](#dot).

### `square_length`

```text
square_length(v: Vector2D) -> f64
```

Computes the squared length of a vector.

#### Parameters

| Parameter | Description |
| --- | --- |
| `v` | The vector to measure. |

#### Return value

`v.x * v.x + v.y * v.y`, subject to floating-point rounding.
Intermediate products can overflow or underflow.

#### Complexity

Constant time and space.

#### Example

```text
(Vector2D { x: 3.0, y: 4.0 }).square_length
// => 25.0
```

#### See also

[`length`](#length), [`dot`](#dot), [`sqrt`](standard-library.md#sqrt).

### `length`

```text
length(v: Vector2D) -> f64
```

Computes a vector's Euclidean length.

#### Parameters

| Parameter | Description |
| --- | --- |
| `v` | The vector to measure. |

#### Return value

The square root of the squared length, subject to floating-point rounding,
overflow, and underflow. This operation does not reject non-finite results.

#### Complexity

Constant time and space.

#### Example

```text
(Vector2D { x: 3.0, y: 4.0 }).length
// => 5.0
```

#### See also

[`square_length`](#square_length), [`normalize`](#normalize), [`distance_to`](#distance_to).

### `normalize`

```text
normalize(v: Vector2D) -> Vector2D
```

Returns a vector in the direction of `v` with unit length, subject to
floating-point rounding.

#### Parameters

| Parameter | Description |
| --- | --- |
| `v` | The vector to normalize. |

#### Return value

A new vector obtained by dividing each component of `v` by its computed length.

#### Errors

Evaluation fails if the computed length is zero or non-finite, or if either
result component is non-finite. Finite, nonzero components do not guarantee
success: computing the length can underflow to zero or overflow to infinity.

#### Complexity

Constant time and space.

#### Example

```text
normalize(Vector2D { x: 3.0, y: 4.0 })
// => Vector2D { x: 0.6, y: 0.8 }
```

Equivalent receiver-call syntax:

```text
(Vector2D { x: 3.0, y: 4.0 }).normalize()
// => Vector2D { x: 0.6, y: 0.8 }
```

A zero vector produces an evaluation error:

```text
normalize(Vector2D { x: 0.0, y: 0.0 })
// Evaluation error: normalize requires a positive finite length
```

#### See also

[`length`](#length), [`square_length`](#square_length), [`is_finite`](#is_finite).

### `distance_to`

```text
distance_to(a: Point2D, b: Point2D) -> f64
```

Computes the Euclidean distance between two points.

#### Parameters

| Parameter | Description |
| --- | --- |
| `a`, `b` | The points to measure between. |

#### Return value

The length of their displacement, subject to floating-point rounding,
overflow, and underflow.

#### Complexity

Constant time and space.

#### Example

```text
(Point2D { x: 1.0, y: 2.0 }).distance_to(Point2D { x: 4.0, y: 6.0 })
// => 5.0
```

#### See also

[`-`](#subtraction--), [`length`](#length).

## Bounds operations

### `translate`

```text
translate(value: T, offset: Vector2D) -> T
```

`T` is `Rect` or `Box2D`. Moves bounds by a displacement.

#### Parameters

| Parameter | Description |
| --- | --- |
| `value` | The rectangle or box to move. |
| `offset` | The displacement to add. |

#### Return value

For a rectangle, a translated origin with the original size.
For a box, translated minimum and maximum points.

#### Complexity

Constant time and space.

#### Example

```text
(Rect {
    origin: Point2D { x: 1.0, y: 2.0 },
    size: Size2D { width: 4.0, height: 6.0 }
}).translate(Vector2D { x: 3.0, y: 4.0 })
// => Rect { origin: Point2D { x: 4.0, y: 6.0 }, size: Size2D { width: 4.0, height: 6.0 } }
```

#### See also

[`+`](#addition-), [`translation`](#translation).

### `inflate`

```text
inflate(value: T, dx: f64, dy: f64) -> T
```

`T` is `Rect` or `Box2D`. Expands bounds on both sides of each axis.

#### Parameters

| Parameter | Description |
| --- | --- |
| `value` | The rectangle or box to expand. |
| `dx`, `dy` | Expansion on each horizontal and vertical side. Negative values shrink. |

#### Return value

Bounds with minimum coordinates decreased by `dx`, `dy` and maximum
coordinates increased by `dx`, `dy`. The width changes by `2 * dx` and
the height by `2 * dy`. Excessive shrinking can produce negative extents;
it is not an evaluation error.

#### Complexity

Constant time and space.

#### Example

```text
(Box2D {
    min: Point2D { x: 1.0, y: 2.0 },
    max: Point2D { x: 5.0, y: 8.0 }
}).inflate(1.0, 2.0)
// => Box2D { min: Point2D { x: 0.0, y: 0.0 }, max: Point2D { x: 6.0, y: 10.0 } }
```

#### See also

[`translate`](#translate), [`width` and `height`](#width-height).

### `to_box2d`

```text
to_box2d(rect: Rect) -> Box2D
```

Converts an origin-and-size rectangle to endpoint bounds.

#### Parameters

| Parameter | Description |
| --- | --- |
| `rect` | The rectangle to convert. |

#### Return value

A box whose `min` is the rectangle's origin and whose `max` is the origin
plus the size. The conversion does not normalize negative extents.

#### Complexity

Constant time and space.

#### Example

```text
(Rect {
    origin: Point2D { x: 1.0, y: 2.0 },
    size: Size2D { width: 4.0, height: 6.0 }
}).to_box2d
// => Box2D { min: Point2D { x: 1.0, y: 2.0 }, max: Point2D { x: 5.0, y: 8.0 } }
```

#### See also

[`to_rect`](#to_rect), [`min` and `max`](#min-max).

### `to_rect`

```text
to_rect(bounds: Box2D) -> Rect
```

Converts endpoint bounds to an origin-and-size rectangle.

#### Parameters

| Parameter | Description |
| --- | --- |
| `bounds` | The box to convert. |

#### Return value

A rectangle whose origin is `min` and whose size is `max - min`.
Floating-point rounding means conversions need not round-trip exactly.

#### Complexity

Constant time and space.

#### Example

```text
(Box2D {
    min: Point2D { x: 1.0, y: 2.0 },
    max: Point2D { x: 5.0, y: 8.0 }
}).to_rect
// => Rect { origin: Point2D { x: 1.0, y: 2.0 }, size: Size2D { width: 4.0, height: 6.0 } }
```

#### See also

[`to_box2d`](#to_box2d), [`size`](#size).

### `contains`

```text
contains(bounds: Rect, point: Point2D) -> bool
contains(bounds: Box2D, point: Point2D) -> bool
```

Tests point containment using half-open bounds.

#### Parameters

| Parameter | Description |
| --- | --- |
| `bounds` | The rectangle or box to test. |
| `point` | The point to locate. |

#### Return value

`true` when both minimum coordinates are included and both maximum
coordinates are excluded:
`min_x <= point.x < max_x` and `min_y <= point.y < max_y`.
Empty or inverted bounds contain no points.

#### Complexity

Constant time and space.

#### Example

```text
(Box2D {
    min: Point2D { x: 1.0, y: 2.0 },
    max: Point2D { x: 5.0, y: 8.0 }
}).contains(Point2D { x: 5.0, y: 4.0 })
// => false
```

#### See also

[`contains_rect`](#contains_rect), [`contains_box`](#contains_box), [`intersects`](#intersects).

### `contains_rect`

```text
contains_rect(outer: Rect, inner: Rect) -> bool
```

Tests whether one rectangle contains another using Euclid's bounds rules.

#### Parameters

| Parameter | Description |
| --- | --- |
| `outer` | The containing rectangle. |
| `inner` | The rectangle to test. |

#### Return value

`true` if `inner` is empty, or if `outer` is nonempty and contains all of
`inner`'s bounds. Coincident maximum edges are allowed for nonempty rectangles,
unlike point containment. NaN extents follow Euclid's empty-bounds rules.

#### Complexity

Constant time and space.

#### Example

```text
contains_rect(
    Rect { origin: Point2D { x: 0.0, y: 0.0 }, size: Size2D { width: 10.0, height: 10.0 } },
    Rect { origin: Point2D { x: 1.0, y: 2.0 }, size: Size2D { width: 4.0, height: 6.0 } }
)
// => true
```

#### See also

[`contains`](#contains), [`contains_box`](#contains_box).

### `contains_box`

```text
contains_box(outer: Box2D, inner: Box2D) -> bool
```

Tests whether one box contains another using Euclid's bounds rules.

#### Parameters

| Parameter | Description |
| --- | --- |
| `outer` | The containing box. |
| `inner` | The box to test. |

#### Return value

`true` if `inner` is empty, or if `outer` is nonempty and contains all of
`inner`'s bounds. Coincident maximum edges are allowed for nonempty boxes.
NaN endpoints follow Euclid's empty-bounds rules.

#### Complexity

Constant time and space.

#### Example

```text
contains_box(
    Box2D { min: Point2D { x: 0.0, y: 0.0 }, max: Point2D { x: 10.0, y: 10.0 } },
    Box2D { min: Point2D { x: 1.0, y: 2.0 }, max: Point2D { x: 5.0, y: 8.0 } }
)
// => true
```

#### See also

[`contains`](#contains), [`contains_rect`](#contains_rect).

### `intersects`

```text
intersects(a: T, b: T) -> bool
```

`T` is `Rect` or `Box2D`. Tests whether two bounds overlap.

#### Parameters

| Parameter | Description |
| --- | --- |
| `a`, `b` | Two rectangles or two boxes. |

#### Return value

`true` when both axes satisfy the strict endpoint comparisons
`a.min < b.max` and `a.max > b.min`. For nonempty finite bounds, this
means positive-area overlap; touching at an edge or corner returns `false`.
The operation does not separately reject empty or inverted bounds: an
empty box strictly inside another box can return `true`.

#### Complexity

Constant time and space.

#### Example

```text
intersects(
    Box2D { min: Point2D { x: 0.0, y: 0.0 }, max: Point2D { x: 2.0, y: 2.0 } },
    Box2D { min: Point2D { x: 2.0, y: 0.0 }, max: Point2D { x: 4.0, y: 2.0 } }
)
// => false
```

```text
intersects(
    Box2D { min: Point2D { x: 1.0, y: 1.0 }, max: Point2D { x: 1.0, y: 1.0 } },
    Box2D { min: Point2D { x: 0.0, y: 0.0 }, max: Point2D { x: 2.0, y: 2.0 } }
)
// => true
```

#### See also

[`contains`](#contains), [`union`](#union).

### `union`

```text
union(a: T, b: T) -> T
```

`T` is `Rect` or `Box2D`. Computes enclosing axis-aligned bounds.

#### Parameters

| Parameter | Description |
| --- | --- |
| `a`, `b` | Two rectangles or two boxes. |

#### Return value

The enclosing bounds under Euclid's union rules. For nonempty finite inputs,
the minimum coordinates are the component-wise minima and the maximum
coordinates are the component-wise maxima. If one operand is empty, its
bounds are ignored; if both are empty, the first operand supplies the bounds.
Rectangles convert through endpoint bounds, so their extents can incur
rounding. This is not a shape-set union.

#### Complexity

Constant time and space.

#### Example

```text
union(
    Box2D { min: Point2D { x: 0.0, y: 0.0 }, max: Point2D { x: 2.0, y: 2.0 } },
    Box2D { min: Point2D { x: 1.0, y: 1.0 }, max: Point2D { x: 4.0, y: 3.0 } }
)
// => Box2D { min: Point2D { x: 0.0, y: 0.0 }, max: Point2D { x: 4.0, y: 3.0 } }
```

#### See also

[`intersects`](#intersects), [`outer_transformed_rect`](#outer_transformed_rect).

## Angles

### `degrees`

```text
degrees(value: f64) -> Angle
degrees(angle: Angle) -> f64
```

Constructs an angle from degrees or expresses an angle in degrees.

#### Parameters

| Parameter | Description |
| --- | --- |
| `value` | An angle magnitude in degrees. |
| `angle` | An angle to convert to a scalar. |

#### Return value

For `f64`, an angle stored in radians. For `Angle`, its magnitude in degrees.
Neither overload wraps the angle to a particular interval.

#### Complexity

Constant time and space.

#### Example

```text
degrees(0.0).radians
// => 0.0
```

#### See also

[`radians`](#radians), [`rotation`](#rotation).

### `radians`

```text
radians(value: f64) -> Angle
radians(angle: Angle) -> f64
```

Constructs an angle from radians or reads its stored magnitude.

#### Parameters

| Parameter | Description |
| --- | --- |
| `value` | An angle magnitude in radians. |
| `angle` | An angle whose magnitude is requested. |

#### Return value

For `f64`, an angle with the given stored radians. For `Angle`, its stored
`radians` component. Neither overload wraps the angle.

#### Complexity

Constant time and space.

#### Example

```text
radians(1.5).radians
// => 1.5
```

#### See also

[`degrees`](#degrees), [`rotation`](#rotation).

## Affine transforms

### `m11`, `m12`, `m21`, `m22`, `m31`, `m32`

```text
m11(transform: Transform2D) -> f64
m12(transform: Transform2D) -> f64
m21(transform: Transform2D) -> f64
m22(transform: Transform2D) -> f64
m31(transform: Transform2D) -> f64
m32(transform: Transform2D) -> f64
```

Reads a stored affine matrix coefficient.

#### Parameters

| Parameter | Description |
| --- | --- |
| `transform` | The transform to inspect. |

#### Return value

The coefficient named by the operation. `m31` and `m32` are translation
components; the other four form the linear part.

#### Complexity

Constant time and space.

#### Example

```text
translation(Vector2D { x: 10.0, y: 20.0 }).m31
// => 10.0
```

#### See also

[`transform_point`](#transform_point), [`determinant`](#determinant).

### `identity_transform`

```text
identity_transform() -> Transform2D
```

Constructs the identity affine transform.

#### Return value

A transform with `m11 = m22 = 1.0` and all other coefficients zero.
It leaves finite points and vectors unchanged.

#### Complexity

Constant time and space.

#### Example

```text
identity_transform().transform_point(Point2D { x: 1.0, y: 2.0 })
// => Point2D { x: 1.0, y: 2.0 }
```

#### See also

[`translation`](#translation), [`scale`](#scale), [`rotation`](#rotation).

### `translation`

```text
translation(offset: Vector2D) -> Transform2D
```

Constructs a translation transform.

#### Parameters

| Parameter | Description |
| --- | --- |
| `offset` | The displacement to apply to points. |

#### Return value

A transform with identity linear coefficients and translation components
`offset.x`, `offset.y`. Applying it to a vector ignores the translation.

#### Complexity

Constant time and space.

#### Example

```text
translation(Vector2D { x: 10.0, y: 20.0 })
    .transform_point(Point2D { x: 1.0, y: 2.0 })
// => Point2D { x: 11.0, y: 22.0 }
```

#### See also

[`translate`](#translate), [`then`](#then), [`transform_vector`](#transform_vector).

### `scale`

```text
scale(sx: f64, sy: f64) -> Transform2D
```

Constructs an axis-aligned scaling transform about the origin.

#### Parameters

| Parameter | Description |
| --- | --- |
| `sx`, `sy` | Horizontal and vertical scale factors. |

#### Return value

A transform with `m11 = sx`, `m22 = sy`, and all other coefficients zero.
Negative factors reflect the corresponding axes; zero factors produce a
singular transform. Construction does not reject either case.

#### Complexity

Constant time and space.

#### Example

```text
scale(2.0, 3.0).transform_point(Point2D { x: 1.0, y: 2.0 })
// => Point2D { x: 2.0, y: 6.0 }
```

#### See also

[`*` and `/`](#scaling--), [`then`](#then), [`inverse`](#inverse).

### `rotation`

```text
rotation(angle: Angle) -> Transform2D
```

Constructs a rotation about the origin.

#### Parameters

| Parameter | Description |
| --- | --- |
| `angle` | The rotation angle. |

#### Return value

A transform with linear coefficients `cos(angle)`, `sin(angle)`,
`-sin(angle)`, `cos(angle)` and zero translation.
Positive angles rotate `(1, 0)` toward `(0, 1)`; the visual direction
depends on whether the consuming coordinate system's y-axis points up or down.

#### Complexity

Constant time and space.

#### Example

```text
rotation(degrees(0.0)).transform_point(Point2D { x: 1.0, y: 2.0 })
// => Point2D { x: 1.0, y: 2.0 }
```

#### See also

[`degrees`](#degrees), [`radians`](#radians), [`then`](#then).

### `then`

```text
then(first: Transform2D, second: Transform2D) -> Transform2D
```

Composes transforms in application order: `first`, then `second`.

#### Parameters

| Parameter | Description |
| --- | --- |
| `first` | The transform applied first. |
| `second` | The transform applied second. |

#### Return value

The composed affine transform. Composition generally does not commute.
Its result can differ from two separately evaluated mappings because
floating-point operations round at different points.

#### Complexity

Constant time and space.

#### Example

```text
translation(Vector2D { x: 10.0, y: 0.0 })
    .then(scale(2.0, 2.0))
    .transform_point(Point2D { x: 1.0, y: 0.0 })
// => Point2D { x: 22.0, y: 0.0 }
```

#### See also

[`translation`](#translation), [`scale`](#scale), [`transform_point`](#transform_point).

### `determinant`

```text
determinant(transform: Transform2D) -> f64
```

Computes the determinant of the linear part of an affine transform.

#### Parameters

| Parameter | Description |
| --- | --- |
| `transform` | The transform to inspect. |

#### Return value

`m11 * m22 - m12 * m21`, subject to floating-point rounding.
Translation does not affect the result. A zero determinant indicates a
singular linear part; a nonzero result alone does not guarantee that
checked inversion succeeds.

#### Complexity

Constant time and space.

#### Example

```text
scale(2.0, 3.0).determinant
// => 6.0
```

#### See also

[`inverse`](#inverse), [`is_finite`](#is_finite).

### `inverse`

```text
inverse(transform: Transform2D) -> Transform2D
```

Computes a checked affine inverse.

#### Parameters

| Parameter | Description |
| --- | --- |
| `transform` | The transform to invert. |

#### Return value

Euclid's inverse with finite coefficients and no nonzero exact cofactor
lost to a zero inverse coefficient. Exact-zero cofactors and nonzero
subnormal coefficients are allowed.

The check does not guarantee exact-real arithmetic, numerical stability,
or an exact round trip. Other rounding and conditioning errors retain
Euclid's floating-point behavior.

#### Errors

Evaluation fails for:

- Non-finite input coefficients.
- A zero or non-finite computed determinant.
- Non-finite inverse coefficients.
- A zero inverse coefficient whose exact cofactor is nonzero, including
  loss within translation-cofactor arithmetic before determinant scaling.

#### Complexity

Constant time and space.

#### Example

```text
scale(2.0, 4.0).inverse().transform_point(Point2D { x: 6.0, y: 8.0 })
// => Point2D { x: 3.0, y: 2.0 }
```

#### See also

[`determinant`](#determinant), [`then`](#then), [`is_finite`](#is_finite).

### `transform_point`

```text
transform_point(transform: Transform2D, point: Point2D) -> Point2D
```

Applies an affine transform to a point, including translation.

#### Parameters

| Parameter | Description |
| --- | --- |
| `transform` | The mapping to apply. |
| `point` | The input position. |

#### Return value

The point given by the coordinate equations in [Transform2D](#transform2d).
Mapping is unchecked and can return non-finite coordinates.

#### Complexity

Constant time and space.

#### Example

```text
translation(Vector2D { x: 10.0, y: 0.0 })
    .transform_point(Point2D { x: 1.0, y: 2.0 })
// => Point2D { x: 11.0, y: 2.0 }
```

#### See also

[`transform_vector`](#transform_vector), [`outer_transformed_rect`](#outer_transformed_rect).

### `transform_vector`

```text
transform_vector(transform: Transform2D, vector: Vector2D) -> Vector2D
```

Applies the linear part of an affine transform to a displacement.

#### Parameters

| Parameter | Description |
| --- | --- |
| `transform` | The mapping to apply. |
| `vector` | The input displacement. |

#### Return value

A vector with components `m11 * x + m21 * y` and `m12 * x + m22 * y`.
The translation coefficients `m31`, `m32` do not contribute.
Mapping is unchecked and can return non-finite components.

#### Complexity

Constant time and space.

#### Example

```text
translation(Vector2D { x: 10.0, y: 20.0 })
    .transform_vector(Vector2D { x: 3.0, y: 4.0 })
// => Vector2D { x: 3.0, y: 4.0 }
```

#### See also

[`transform_point`](#transform_point), [`then`](#then).

### `outer_transformed_rect`

```text
outer_transformed_rect(transform: Transform2D, rect: Rect) -> Rect
```

Computes checked axis-aligned bounds enclosing all four transformed corners.

#### Parameters

| Parameter | Description |
| --- | --- |
| `transform` | The affine mapping, including rotation, shear, or reflection. |
| `rect` | The source rectangle. |

#### Return value

A finite rectangle enclosing all four corners after Euclid maps them.
If reconstructing a maximum from an origin and extent rounds inward,
the extent advances by one representable `f64` step to enclose that maximum.
Zero source extents and singular transforms are allowed.

An outer bound is not an interior-containment test: its corners need not
lie inside the transformed shape.

#### Errors

Evaluation fails for:

- Non-finite matrix coefficients, source origins, extents, or endpoints.
- Negative source extents, or positive extents whose computed endpoints
  do not advance beyond the corresponding origin.
- Projected edges that overflow or lose a nonzero extent completely to
  underflow, or non-finite projected spans.
- Non-finite mapped corners, or a positive projected span that collapses
  to zero after mapping.
- Output extents or enclosing endpoints that cannot be represented finitely.

Failure produces an evaluation error, not an empty rectangle.

#### Complexity

Constant time and space; maps four corners.

#### Example

```text
scale(2.0, 3.0).outer_transformed_rect(Rect {
    origin: Point2D { x: 1.0, y: 2.0 },
    size: Size2D { width: 4.0, height: 6.0 }
})
// => Rect { origin: Point2D { x: 2.0, y: 6.0 }, size: Size2D { width: 8.0, height: 18.0 } }
```

#### See also

[`transform_point`](#transform_point), [`union`](#union), [`inverse`](#inverse).

## Host integration

This checked Rust example installs both libraries, constructs records with
reversed field order, and composes transforms in application order:

```rust
{{#include ../tests/geometry.rs:geometry}}
```

Run `cargo test -p cel-lang-book --test geometry` to execute the host example
and the successful CEL examples marked with `// =>` in this chapter.
Those comments describe results; they are not part of the signature notation.

Installing the library makes records and operations available. Geometry names
in closure annotations additionally require a configured `TypeResolver`.
The optional Adam adapter supplies type registration when the host enables
`cel-euclid`'s `adam` feature and calls `cel_euclid::register_adam_types`.
The host must still install operations on the lookup.

## Geometry policy belongs in the consuming sheet

The library supplies general geometry, not crop-specific constraints.
The [Adam geometry chapter](https://stlab.github.io/cel-rs/book/geometry.html)
composes inverse bounds and scalar operations into a self-referencing fitting
relationship. Its precision, coverage, and inward-margin requirements belong
to that sheet, not to these operations.
