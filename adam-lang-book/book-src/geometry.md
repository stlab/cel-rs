# Optional geometry and self-referencing constraints

The book, `adam-web-ui`, and `adam-lsp` explicitly install `cel-std` operations,
`cel-euclid` operations, and the seven geometry cell types. A standalone
`adam-lang` host still chooses its own libraries; Adam has no mandatory geometry
dependency. Enable `cel-euclid`'s `adam` Cargo feature, call
`cel_euclid::register_adam_types(&mut types)` and
`cel_euclid::install(&mut lookup)`, and construct `AdamParser::new(types, lookup)`.
Pass the same registry and lookup to `adam_lang::check_sheet` for editor checking.
The checker uses exact record/member metadata where types are known, visits
closure bodies, and defers unresolved call-result and tuple shapes to runtime.
It does not claim complete inference.

Geometry uses concrete `f64` / `UnknownUnit` aliases, not generic Adam types.
Write `1.0` or `1 as f64` rather than passing `i32` fields implicitly.
Field order is arbitrary; every declared field must occur once. Consult the
[complete signature table](https://stlab.github.io/cel-rs/cel_euclid/#supported-geometry-signatures)
for fields, supported operand types, checked errors, and containment semantics.

## Crop and straighten

This is the **actual library example**, included rather than copied. Run
`cargo run -p cel-euclid --features adam --example adam_geometry` to execute it.
`cargo test -p adam-lang-book --test geometry` verifies these same included
sheets through the book's parser. These cross-library demonstrations are
executable Rust-driven examples, not live inspector mounts or a crop UI.

```rust
{{#include ../../cel-euclid/examples/crop_straighten.adm2}}
```

### Coordinate frames and source memory

`image` is an axis-aligned source-image rectangle. `transform` maps image points
to the frame containing the axis-aligned requested `crop`. The runner constructs
straightening rotations around the source-image point under the crop center.
The relationship also works with arbitrary invertible affine translations,
nonuniform scales, reflections, and shears; it does not assume a rotation or
an origin-centered anchor.

The relationship reads and writes `crop`. Adam preserves the user's source
aspiration while it recomputes the effective value. Tightening the angle shrinks
the crop uniformly; loosening it springs back. An explicit crop write replaces
the aspiration. There is no extra requested-crop cell or helper-owned memory.
Every typed, noncapturing IIFE forwards its needed values explicitly.

The expression maps the requested center and the four-corner outer bounds
through the inverse transform. It divides each nearest-image-edge distance by
the corresponding maximum inverse half-extent, then selects the minimum with
one. This gives the largest uniform shrink about that center. Bounding the
transformed image instead would include uncovered regions under rotation or
shear. A crop with an infeasible center is an error; translating it is not a
valid repair.

### Numerical and error contract

Inputs need finite positive image/crop sizes, advancing finite endpoints, a
finite center, a checked invertible transform, representable inverse bounds,
and an image anchor **strictly inside** all four image edges. The runner's
validation-only write boundary checks candidates before changing source cells.
It uses the general checked inverse and bounds operations, not a Rust fitting
calculation.

The CEL relationship leaves a fitting crop bit-for-bit unchanged. When it must
shrink, it multiplies the computed factor by `1 - 8*f64::EPSILON` to round
conservatively inward. The `precise` flag rejects inverse-bound cancellation
that differs from affine support half-extents by more than `32*EPSILON`; it also
rejects subnormal rounding that changes either uniform size ratio by more than
`64*EPSILON` relative to the shrink factor. This flag is validation metadata,
not another requested rectangle. The expression contains these policies openly.

Acceptance uses a `64*f64::EPSILON` scale-relative tolerance for closed-edge
coverage, center preservation, maximality, and repeated application. For image
coordinates the scale is the maximum image extent or absolute image endpoint;
for dimensionless shrink ratios the scale is one. Ordinary Euclid point
containment is half-open; crop coverage allows corners on maximum edges.
The conservative result is maximal within this tolerance, not an exact-real
arithmetic guarantee. Identity preserves a fitting crop exactly; applying the
expression to its effective crop again is stable within tolerance.

Finite extreme magnitudes are supported when these computations remain
representable. Nonfinite values, singular/lost-cofactor inverses, overflow,
complete underflow, collapsed endpoints, infeasible centers, zero effective
sizes, unrepresentable reciprocal scales, or insufficient precision produce a
checked evaluation error or a named failed Adam requirement. Propagation alone
does not mean requirements passed: the runner checks **every requirement**
and refuses invalid effective values. Inspect errors and requirements instead
of silently accepting an empty or numerically misleading crop.

### Presentation

The runner reads `parsed.sheet.source::<Rect>(crop_id)` for the unchanged
overlay, never the previously fitted value. `shrink` describes the current
source-to-effective size ratio. `presentation` applies the image transform and
then its reciprocal shrink around the same crop center. Thus the effective
crop fills the original overlay and the same image anchor stays under its
center (within the rounding tolerance). In `a.then(b)`, `a` runs first.

For a `[0,100] × [0,100]` image, a centered square at 45 degrees has side
`100 / sqrt(2)` within `1e-12`. For `[0,100] × [0,80]` with a requested
`80 × 40` crop centered at `(20,40)`, identity yields origin `(0,30)` and
size `(40,20)` within tolerance. The center never moves to hide infeasibility.

## Pan and zoom policy

```rust
{{#include ../../cel-euclid/examples/pan_zoom.adm2}}
```

The caller supplies measured visible-node bounds including the fit margin and
a positive viewport. Empty content falls back to viewport bounds. Fit uses
`min(viewport.width / max(width,1), viewport.height / max(height,1))`.
Zoom clamps to `[fit_scale, max(fit_scale,8)]`. Each pan axis centers undersized
content; otherwise it clamps to the actual scaled content edges.
Self-referencing writes retain pan/zoom aspirations. For initial fit or reset,
the caller writes `fit_scale` to `zoom` and `fit_pan` to `pan`.

This sheet demonstrates policy only. The live graph still uses its existing
JavaScript model; no Dioxus bridge or graph migration ships in this stage.
The future bridge must wait for real positive viewport observations and retain
the last valid viewport during transient zero-size observations.
