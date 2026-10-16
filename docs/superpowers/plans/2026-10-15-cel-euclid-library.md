# CEL Euclid Library Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a standalone optional Euclid library, Adam geometry integration, and real pan/zoom and crop/straighten examples.

**Architecture:** Register concrete Euclid aliases, record schemas and typed operations through the language stage's shared metadata. Express crop policy in the Adam example using those operations, not a Rust constraint helper. An optional adapter registers cell types, and opted-in hosts install matching operations/types.

**Tech Stack:** Rust 2024, Euclid 0.22.14, existing CEL runtime, `cel-std`, Adam.

**Spec:** `docs\superpowers\specs\2026-10-15-cel-euclid-design.md`; prerequisite: `2026-10-15-cel-geometry-language.md`.

## Global Constraints

- Expose only `f64` component types initially.
- Do not expose Euclid unit parameters or introduce generic language syntax.
- Use Euclid's `UnknownUnit` aliases.
- Do not add a separate requested-crop cell to simulate memory.
- Each field is required exactly once.
- Never translate the crop to disguise an infeasible anchor.
- Do not introduce `Option` as an incidental language feature.
- Ordinary Euclid arithmetic retains Euclid's semantics.

## Review Focus

- `Box2D.min`/`.max` accessor overloads must not hide scalar `min`/`max`.
- Euclid's ordinary containment may be half-open; image coverage permits crop corners on the actual image edge.
- Large coordinates, tiny extents and non-finite inverses must fail explicitly rather than produce a plausible but invalid crop.
- Off-center anchors and affine shear must work; origin-centered rotation is not the general contract.
- Restoring an angle must restore the original crop through Adam, with no helper-owned memory.

---

## Public interfaces and supported signatures

Create `cel-euclid` with dependencies on `cel-parser`, `cel-runtime`,
`euclid = "0.22.14"` and `anyhow`. Feature
`adam = ["dep:adam-lang"]` enables the optional path dependency. No default
features pull in Adam. Use existing workspace lint settings.

```rust
pub type Point2D = euclid::default::Point2D<f64>;
pub type Vector2D = euclid::default::Vector2D<f64>;
pub type Size2D = euclid::default::Size2D<f64>;
pub type Rect = euclid::default::Rect<f64>;
pub type Box2D = euclid::default::Box2D<f64>;
pub type Angle = euclid::Angle<f64>;
pub type Transform2D = euclid::default::Transform2D<f64>;

pub fn install(lookup: &mut cel_parser::OpLookup);
#[cfg(feature = "adam")]
pub fn register_adam_types(registry: &mut adam_lang::TypeRegistry);
```

Installation binds ordinary function names and direct signatures once; static
schema inconsistencies are programmer errors with descriptive `expect`
messages. Runtime/user failures use `opNr`, never `expect`.

Record schemas use these exact field orders:
`Point2D(x,y)`, `Vector2D(x,y)`, `Size2D(width,height)`,
`Rect(origin,size)`, `Box2D(min,max)`, `Angle(radians)`,
`Transform2D(m11,m12,m21,m22,m31,m32)`.
Written order remains arbitrary.

The table fixes the initial CEL operation set. All identifier operations
support ordinary calls and receiver syntax when they have a receiver;
zero-argument construction uses ordinary calls. `T` below is table shorthand,
not CEL generic syntax.

| Operation | Operands -> result |
| --- | --- |
| `==`, `!=` | `(T,T) -> bool` for all seven aliases |
| `is_finite` | `(T) -> bool` for all seven aliases |
| `+` | `(Point2D,Vector2D) -> Point2D`; `(Vector2D,Vector2D) -> Vector2D`; `(Size2D,Size2D) -> Size2D` |
| `-` | `(Point2D,Point2D) -> Vector2D`; `(Point2D,Vector2D) -> Point2D`; vector/vector and size/size -> same type |
| unary `-` | `(Vector2D) -> Vector2D` |
| `*`, `/` | `(Vector2D,f64) -> Vector2D`; `(Size2D,f64) -> Size2D` |
| `x`, `y` | `(Point2D) -> f64`; `(Vector2D) -> f64` |
| `width`, `height` | `(Size2D) -> f64`; `(Rect) -> f64`; `(Box2D) -> f64` |
| `origin` | `(Rect) -> Point2D` |
| `size` | `(Rect) -> Size2D`; `(Box2D) -> Size2D` |
| `min`, `max` | `(Box2D) -> Point2D` |
| `min_x`, `min_y`, `max_x`, `max_y` | `(Rect) -> f64`; `(Box2D) -> f64` |
| `center` | `(Rect) -> Point2D`; `(Box2D) -> Point2D` |
| `dot`, `cross` | `(Vector2D,Vector2D) -> f64` |
| `square_length`, `length` | `(Vector2D) -> f64` |
| `normalize` | `(Vector2D) -> Vector2D`, checked |
| `distance_to` | `(Point2D,Point2D) -> f64` |
| `translate` | `(Rect,Vector2D) -> Rect`; `(Box2D,Vector2D) -> Box2D` |
| `inflate` | `(Rect,f64,f64) -> Rect`; `(Box2D,f64,f64) -> Box2D` |
| `to_box2d`, `to_rect` | `(Rect) -> Box2D`; `(Box2D) -> Rect`, respectively |
| `contains` | `(Rect,Point2D) -> bool`; `(Box2D,Point2D) -> bool` |
| `contains_rect`, `contains_box` | `(Rect,Rect) -> bool`; `(Box2D,Box2D) -> bool`, respectively |
| `intersects`, `union` | `(T,T) -> bool/T` for `Rect`, `Box2D` |
| `degrees`, `radians` | `(f64) -> Angle`; `(Angle) -> f64` |
| matrix field accessors | `(Transform2D) -> f64` for the six schema fields |
| `identity_transform` | `() -> Transform2D` |
| `translation` | `(Vector2D) -> Transform2D` |
| `scale` | `(f64,f64) -> Transform2D` |
| `rotation` | `(Angle) -> Transform2D` |
| `then` | `(Transform2D,Transform2D) -> Transform2D` |
| `determinant` | `(Transform2D) -> f64` |
| `inverse` | `(Transform2D) -> Transform2D`, checked |
| `transform_point`, `transform_vector` | `(Transform2D,Point2D/Vector2D) -> same geometry type` |
| `outer_transformed_rect` | `(Transform2D,Rect) -> Rect`, checked axis-aligned bound |

Do not add optional intersections, type-parameter syntax, scalar-left
multiplication, or unrelated convenience overloads in this pass. Ordinary
geometry arithmetic follows Euclid even for non-finite values; the explicitly
checked operations document and reject invalid inputs/results.

### Task 1: Geometry records, operations and optional Adam registration

**Prerequisites:** Completed language plan.

**Files:** Create `cel-euclid\Cargo.toml`,
`cel-euclid\src\{lib,records,geometry,adam}.rs`,
`cel-euclid\tests\{geometry,adam_types}.rs`; modify root `Cargo.toml`
and generated `Cargo.lock`.

**Interfaces:** Produces seven aliases, `install`, `register_adam_types`, the
record schemas and the table's non-transform operations. Complete transform
finite predicates in Task 2.

- [ ] **Step 1: Add public contract tests** `named_records_match_euclid`,
  `reversed_rect_fields_keep_types`, `vector_and_point_arithmetic`,
  `box_min_extends_scalar_min`, `ordinary_containment_matches_euclid`, and
  `integer_fields_require_explicit_casts`.
  Assert `Point2D { y: 42.0, x: 0.0 }.x == 0.0`,
  point `(1,2)` plus vector `(3,4)` equals `(4,6)`, and scalar
  `min(3.0, 7.0)` still returns `3.0` after either installation order.
  Check every table signature, including errors for wrong operands.
- [ ] **Step 2: Run** `cargo test -p cel-euclid --no-default-features`.
  Expected: missing package/API failure. Creating the manifest, workspace
  membership and failing test harness is part of this test cycle.
- [ ] **Step 3: Implement record and geometry registrations.** For homogeneous
  records, pack source-order values once with `make_tuple`, relabel to the
  checked cons-list with `pop_tuple_as`, and build from the schema index map.
  `Rect` has two typed source-order cases; do not allocate boxed fields.
  Implement operators with the same exact operand descriptors as named ops.
  Use Euclid implementations wherever their contracts match the table.
- [ ] **Step 4: Implement checked normalization and optional Adam registration.**
  Reject zero/non-finite length and non-finite normalization results.
  Register each alias under its exact table name with Euclid's `Default`
  when available, otherwise `register_no_default`; document that distinction
  and test default presence explicitly against the chosen release.
- [ ] **Step 5: Run** `cargo test -p cel-euclid --no-default-features`;
  `cargo test -p cel-euclid --features adam`;
  `cargo tree -p cel-euclid --no-default-features`.
  Expected: all tests pass; the last tree has no `adam-lang` or Dioxus.
- [ ] **Step 6: Format and commit**:
  `feat(cel-euclid): expose f64 two-dimensional geometry`.

### Task 2: Angles and affine transform primitives

**Prerequisites:** Task 1.

**Files:** Create `cel-euclid\src\transforms.rs`,
`cel-euclid\tests\transforms.rs`;
modify `cel-euclid\src\lib.rs` and registration wiring.

**Interfaces:** Produces all remaining table operations. Do not introduce a
crop-specific operation or Rust fitting helper.

- [ ] **Step 1: Add tests** `transform_then_order_is_observable`,
  `rotation_preserves_anchor`, `inverse_round_trips_points`,
  `singular_or_nonfinite_inverse_errors`, and
  `matrix_literal_field_order_is_named`.
  Assert translating `(1,0)` by `(10,0)` then scaling `(2,2)` gives `(22,0)`,
  while scaling then translating gives `(12,0)`. A six-field matrix literal
  in reversed order must match the same Euclid matrix.
- [ ] **Step 2: Run** `cargo test -p cel-euclid --test transforms`.
  Expected: missing transform registrations.
- [ ] **Step 3: Implement angle and transform operations using Euclid.**
  Validate finite coefficients, determinant/inverse and checked results.
  Derive transformed outer bounds from the four corners or Euclid's matching
  helper; document that this is not the crop-interior test.
- [ ] **Step 4: Add primitive tests** `inverse_crop_bounds_enclose_all_corners`,
  `transformed_image_bounds_are_not_an_interior_test`, and
  `finite_geometry_predicates_reject_nonfinite_components`.
  Assert inverse-transformed bounds match the extrema of all four
  inverse-transformed corners for rotation, reflection and shear.
  Pin invalid/non-finite and unrepresentable transformed-bound errors.
  These tests specify general geometry, not a hidden crop fitting API.
- [ ] **Step 5: Run** `cargo test -p cel-euclid --all-features`;
  `cargo clippy -p cel-euclid --all-targets --all-features -- -D warnings`.
  Expected: all pass; no custom iterative search or extra math dependency.
- [ ] **Step 6: Format and commit**:
  `feat(cel-euclid): add affine transform primitives`.

### Task 3: Adam examples, host configuration and user documentation

**Prerequisites:** Tasks 1 and 2.

**Files:** Create `cel-euclid\examples\{pan_zoom,crop_straighten}.adm2`,
`cel-euclid\tests\adam_examples.rs`,
`cel-euclid\examples\adam_geometry.rs`,
`cel-lang-book\book-src\geometry.md`,
`adam-lang-book\book-src\geometry.md`,
`adam-lang-book\tests\geometry.rs`;
modify `adam-web-ui\src\build.rs`, `adam-web-ui\Cargo.toml`,
`adam-lsp\{Cargo.toml,src\diagnostics.rs}`,
`adam-lang-book\{Cargo.toml,src\support.rs,book-src\SUMMARY.md}`,
`cel-lang-book\{Cargo.toml,book-src\SUMMARY.md}`,
`docs\VISION.md`, `README.md`.

**Interfaces:** Add `adam_web_ui::build::type_registry() -> TypeRegistry`,
install geometry in existing `op_lookup()`, and use both helpers in
`build_sheet`. LSP and book support explicitly construct equivalent
type/operation environments without depending on the Dioxus host crate.
Add no geometry dependency to `adam-lang` itself.

- [ ] **Step 1: Add tests** `self_referencing_crop_springs_back`,
  `explicit_crop_edit_replaces_aspiration`,
  `presentation_keeps_overlay_and_image_anchor_fixed`,
  `crop_fitting_uses_only_general_registered_operations`,
  `already_fitting_crop_is_unchanged`, `forty_five_degree_crop_scales_uniformly`,
  `off_center_anchor_uses_nearest_edge`, `sheared_image_uses_actual_edges`,
  `crop_expression_is_idempotent`, `invalid_crop_inputs_are_reported`,
  `geometry_sheet_builds_in_opted_in_host`, and
  `lsp_geometry_fields_and_members_have_matching_diagnostics`.
  Parse the actual crop example, write a rotation of 45 degrees and then
  zero, propagate each time, and assert the requested square returns to its
  original size. Write a new requested crop between angle edits and assert
  the new size becomes the source aspiration. For image `[0,100] x [0,100]`
  and the same square crop, a 45-degree rotation about `(50,50)` gives side
  `100 / sqrt(2)` within `1e-12`. For image `[0,100] x [0,80]`, identity
  transform and crop centered `(20,40)` of size `(80,40)`, assert result
  origin `(0,30)`, size `(40,20)`. Run the expression without installing
  any custom crop function scope.
- [ ] **Step 2: Run** `cargo test -p cel-euclid --features adam --test adam_examples`;
  `cargo test -p adam-web-ui -p adam-lsp`.
  Expected: new fixture/host installation failures.
- [ ] **Step 3: Write the crop expression in `crop_straighten.adm2`.**
  For input crop `r`, image `i`, and inverse transform `inv`, let
  `b = inv.outer_transformed_rect(r)` and
  `a = inv.transform_point(r.center())`. Compute source-space half-extents
  `ex = max(a.x - b.min_x(), b.max_x() - a.x)` and the analogous `ey`.
  The scale is
  `s = min(1.0, min(min(a.x-i.min_x(), i.max_x()-a.x)/ex,
  min(a.y-i.min_y(), i.max_y()-a.y)/ey))`.
  Construct `Rect` with size `r.size * s` and origin
  `r.center() - Vector2D { x: r.width*s*0.5, y: r.height*s*0.5 }`.
  Bind intermediate values with immediately invoked typed noncapturing
  closures, forwarding every needed argument explicitly; no new local-binding
  syntax, host-side crop calculation, or `constrain` registration.
  Assign the composed expression to `crop` in a self-referencing relationship.
- [ ] **Step 4: Add boundary validation and numerical checks to the example.**
  Check candidate source geometry at the runner's write boundary, with
  checked inverse/bounds operations and Adam requirements for valid image
  geometry, positive effective crop extents and feasible anchors. Report
  invalid/non-finite geometry, impossible anchors, underflow and
  unrepresentable results. Keep any conservative rounding in CEL scalar
  arithmetic, document it, and assert closed-edge containment/maximality and
  repeated-application stability within a scale-relative
  `64 * f64::EPSILON` tolerance. Exercise finite extreme coordinates,
  right-angle rotation, reflection, shear and unequal aspect ratios.
  Do not move numerical policy into a crop-specific Rust helper.
- [ ] **Step 5: Write the executable runner and pan/zoom sheet.**
  Rotate about the crop's source-image anchor. Derive a presentation transform
  that scales the image by the reciprocal shrink factor about that anchor,
  leaving the requested overlay fixed; the runner reads the source aspiration
  using `parsed.sheet.source::<Rect>(crop_id)`.
  The pan/zoom sheet demonstrates the graph policy formulas in the design;
  the next stage will make the graph's actual sheet authoritative.
- [ ] **Step 6: Install the optional library in the opted-in hosts.**
  `adam-web-ui`, `adam-lsp`, and runnable book support enable `cel-euclid/adam`;
  each installs `cel-std`, Euclid operations and Euclid type registrations.
  Feed the same lookup into the language stage's registration-aware checker.
  Preserve examples intentionally demonstrating the absence of libraries,
  and adjust live-book exclusions only if their contract requires it.
- [ ] **Step 7: Document the complete signature table**, concrete types and
  field order independence, numeric casts, `then` order, half-open ordinary
  containment versus closed crop coverage, checked-operation errors, optional
  installation, and the crop source/presentation coordinate frames.
  Add runnable book examples and update the exploratory Euclid/kurbo roadmap
  text to describe the implemented boundary.
- [ ] **Step 8: Run** `cargo test -p cel-euclid --all-features`;
  `cargo run -p cel-euclid --features adam --example adam_geometry`;
  `cargo test -p adam-web-ui -p adam-lsp -p cel-lang-book -p adam-lang-book`;
  `$env:RUSTDOCFLAGS = '-D warnings'; cargo doc -p cel-euclid --all-features --no-deps --lib`.
  Expected: tests pass, runner produces fitting geometry and spring-back
  assertions hold, rustdoc has no warnings.
- [ ] **Step 9: Format, review the library stage, fix findings, and commit**:
  `feat(adam): demonstrate and configure Euclid geometry`.

Do not migrate the live graph in this stage. Continue with
`2026-10-15-cel-euclid-graph.md` after its independent gate.
