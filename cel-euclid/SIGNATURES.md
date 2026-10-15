# Supported geometry signatures

Every geometry value uses `f64` components and `euclid::UnknownUnit`. CEL exposes
concrete names, not Euclid's generic parameters. These records require each
field exactly once; callers may write fields in any order.

| CEL / Rust alias | Required fields (schema order) |
| --- | --- |
| `Point2D` | `x: f64, y: f64` |
| `Vector2D` | `x: f64, y: f64` |
| `Size2D` | `width: f64, height: f64` |
| `Rect` | `origin: Point2D, size: Size2D` |
| `Box2D` | `min: Point2D, max: Point2D` |
| `Angle` | `radians: f64` |
| `Transform2D` | `m11: f64, m12: f64, m21: f64, m22: f64, m31: f64, m32: f64` |

Use `0.0`, `0f64`, or `0 as f64`, not an unsuffixed integer where a field or
operand needs `f64`. There are no implicit numeric casts, scalar-left geometry
products, generic geometry constructors, or return-type-only overloads.

The following table is the complete set of **98 exact signatures**. `T` is
documentation shorthand, not language syntax. A slash in an operand or result
column lists corresponding alternatives, not additional cross-product overloads.
Every named operation with a receiver accepts both `name(receiver, args...)`
and `receiver.name(args...)`. Unary named operations also accept `receiver.name`.
`identity_transform()` has no receiver. Operators use their normal CEL spelling.

| Operation | Operands → result |
| --- | --- |
| `==`, `!=` | `(T, T) → bool` for all seven aliases |
| `is_finite` | `(T) → bool` for all seven aliases |
| `+` | `(Point2D, Vector2D) → Point2D`; `(Vector2D, Vector2D) → Vector2D`; `(Size2D, Size2D) → Size2D` |
| `-` | `(Point2D, Point2D) → Vector2D`; `(Point2D, Vector2D) → Point2D`; `(Vector2D, Vector2D) → Vector2D`; `(Size2D, Size2D) → Size2D` |
| unary `-` | `(Vector2D) → Vector2D` |
| `*`, `/` | `(Vector2D, f64) → Vector2D`; `(Size2D, f64) → Size2D` |
| `x`, `y` | `(Point2D) → f64`; `(Vector2D) → f64` |
| `width`, `height` | `(Size2D) → f64`; `(Rect) → f64`; `(Box2D) → f64` |
| `origin` | `(Rect) → Point2D` |
| `size` | `(Rect) → Size2D`; `(Box2D) → Size2D` |
| `min`, `max` | `(Box2D) → Point2D` (coexist with `cel-std` scalar overloads) |
| `min_x`, `min_y`, `max_x`, `max_y` | `(Rect) → f64`; `(Box2D) → f64` |
| `center` | `(Rect) → Point2D`; `(Box2D) → Point2D` |
| `dot`, `cross` | `(Vector2D, Vector2D) → f64` |
| `square_length`, `length` | `(Vector2D) → f64` |
| `normalize` | `(Vector2D) → Vector2D`, checked |
| `distance_to` | `(Point2D, Point2D) → f64` |
| `translate` | `(Rect, Vector2D) → Rect`; `(Box2D, Vector2D) → Box2D` |
| `inflate` | `(Rect, f64, f64) → Rect`; `(Box2D, f64, f64) → Box2D` |
| `to_box2d` | `(Rect) → Box2D` |
| `to_rect` | `(Box2D) → Rect` |
| `contains` | `(Rect, Point2D) → bool`; `(Box2D, Point2D) → bool` |
| `contains_rect` | `(Rect, Rect) → bool` |
| `contains_box` | `(Box2D, Box2D) → bool` |
| `intersects` | `(Rect, Rect) → bool`; `(Box2D, Box2D) → bool` |
| `union` | `(Rect, Rect) → Rect`; `(Box2D, Box2D) → Box2D` |
| `degrees`, `radians` | `(f64) → Angle`; `(Angle) → f64` |
| `m11`, `m12`, `m21`, `m22`, `m31`, `m32` | `(Transform2D) → f64` |
| `identity_transform` | `() → Transform2D` |
| `translation` | `(Vector2D) → Transform2D` |
| `scale` | `(f64, f64) → Transform2D` |
| `rotation` | `(Angle) → Transform2D` |
| `then` | `(Transform2D, Transform2D) → Transform2D` |
| `determinant` | `(Transform2D) → f64` |
| `inverse` | `(Transform2D) → Transform2D`, checked |
| `transform_point` | `(Transform2D, Point2D) → Point2D` |
| `transform_vector` | `(Transform2D, Vector2D) → Vector2D` |
| `outer_transformed_rect` | `(Transform2D, Rect) → Rect`, checked |

`a.then(b)` applies **a first, b second**. A translation by `(10, 0)` followed
by scale `(2, 2)` maps `(1, 0)` to `(22, 0)`, not `(12, 0)`. Vectors ignore
translation. The matrix maps `x' = m11*x + m21*y + m31` and
`y' = m12*x + m22*y + m32`.

Ordinary arithmetic retains Euclid's floating-point semantics, including
nonfinite results. Record `is_finite` checks stored components, not derived
endpoints. Rectangles and boxes use Euclid containment; ordinary point
containment is half-open at maximum edges. Crop **coverage** instead permits
corners on either image edge and uses explicit closed inequalities.

## Checked-operation errors

- `normalize` rejects zero/nonfinite lengths and nonfinite results.
- `inverse` rejects nonfinite coefficients, zero/nonfinite determinants,
  nonfinite inverse coefficients, and zero inverse coefficients with exact
  nonzero cofactors (including translation-cofactor underflow). Exact-zero
  cofactors and nonzero subnormals are allowed. Other rounding and conditioning
  errors retain Euclid semantics; this is not exact-real inversion.
- `outer_transformed_rect` rejects nonfinite coefficients, origins, sizes,
  endpoints or mapped corners; negative sizes; positive sizes whose endpoints
  do not advance; overflowing/completely underflowing projected edges;
  collapsed positive projected spans; and unrepresentable bounds. Zero sizes
  and singular transforms are otherwise allowed. The operation encloses all
  four mapped corners, advancing an extent one ULP when reconstructing its
  maximum would round inward. It does **not** prove that an axis-aligned
  rectangle fits inside the transformed shape.

All three return evaluation errors with source context. They never substitute
an empty shape for an error. A domain-specific sheet can impose stricter
requirements, as the crop example does, without changing these general operations.
