# Euclid geometry in CEL and Adam

Status: approved by the user on 2026-10-15.

## Intent

Add an optional `cel-euclid` library that exposes useful Euclid 2D geometry to
CEL and Adam. Add reusable, Rust-like language support for named construction
and receiver-style operations. Exercise the library through real Adam
constraints, including migrating the existing graph view's live pan/zoom
constraints out of JavaScript.

The motivating applications are:

- Keep graph content visible while panning, compute minimum zoom, and fit/reset
  the view.
- Constrain an aspect-ratio-preserving rectangular crop inside a rectangular
  image during straightening. The crop center remains attached to the same
  image point. In a presentation, keep the crop overlay fixed and rotate/scale
  the image around that point.

This is library-first work, not a collection of graph-specific CEL builtins.

## Decisions made with the user

- Expose only `f64` component types initially.
- Do not expose Euclid unit parameters or introduce generic language syntax.
  Use Euclid's `UnknownUnit` aliases.
- Dot syntax resolves registered operations by receiver/argument types.
  Cells and locals named `x`, `width`, or similar do not shadow dot operations.
- Include migration of the live graph view, not just library demonstrations.
- Express crop conformance using general Euclid operations in CEL, not a
  library-provided `constrain` function. The crop relationship reads and
  writes `crop`, using Adam's existing self-referencing semantics.
  Do not add a separate requested-crop cell to simulate memory. Adam already
  reconstructs self-referencing inputs from their source/seed each round.
  Tightening a constraint must not overwrite the user's source crop.

## Proposed implementation calls

These are recommendations, not decisions already approved by the user.

### Deliverable boundary

Include the library, general language extensions, Adam integration, editor and
formatter support for the new expressions, executable geometry examples, and
the live graph migration. Do not build a crop/straighten UI in this project.
Its example and tests must nevertheless implement the real crop constraint.

Do not add generics, 3D geometry, mutation through fields, assignment to member
expressions, implicit borrowing, struct definitions in CEL, associated-item
paths, or a native compiled CEL backend.

### Types

Expose these unqualified CEL/Adam names, corresponding to Euclid types rather
than newly invented geometry wrappers:

| Name | Rust representation |
| --- | --- |
| `Point2D` | `euclid::default::Point2D<f64>` |
| `Vector2D` | `euclid::default::Vector2D<f64>` |
| `Size2D` | `euclid::default::Size2D<f64>` |
| `Rect` | `euclid::default::Rect<f64>` |
| `Box2D` | `euclid::default::Box2D<f64>` |
| `Angle` | `euclid::Angle<f64>` |
| `Transform2D` | `euclid::default::Transform2D<f64>` |

Use the already locked Euclid 0.22 series as a direct dependency; a transitive
Dioxus dependency is not a library dependency declaration. Re-export the
supported concrete aliases for host-side interoperability.

Use ordinary CEL numeric rules: `0.0` is `f64`; `0` is not implicitly converted
because it appears in a geometry field. Explicit casts remain available.

Euclid public-field construction is available for the supported records:
`x/y`, `width/height`, `origin/size`, `min/max`, `radians`, and
`m11/m12/m21/m22/m31/m32`, respectively. Register only the actual public
fields of the chosen Euclid release; do not expose phantom unit fields.
Convenience operations should make full matrix literals unnecessary.

### Core operations

Publish an explicit supported-signature table in the library documentation.
The initial table must cover:

- Equality and inequality for each supported type.
- Point/vector addition and subtraction where Euclid defines them, vector
  negation, and scalar multiplication/division for vectors and sizes.
- Vector dot product, cross product, squared length, length, normalization,
  and point distance.
- Record accessors and useful derived geometry: rectangle/box width, height,
  size, center, minimum/maximum coordinates, translation, inflation, conversion
  between rectangle and box, containment, intersection testing, and union.
- Angle construction from degrees/radians and conversion to degrees/radians.
- Identity, translation, scale, and rotation transform construction;
  composition, determinant, inversion, point/vector transformation, and
  transformed axis-aligned bounds.
- General geometry predicates and operations sufficient to express the crop
  constraint below entirely in CEL.

Use Euclid method names where meaningful. Distinguish fields such as `x` from
derived methods such as `center` in documentation, but both are unary
operations at the CEL boundary. Transform composition follows Euclid's
`then` convention: `a.then(b)` applies `a`, then `b`. Document this with
noncommuting translation/scale examples.

Avoid return-type-only overloads: a signature's name and operand types must
identify the operation. Use explicit, unambiguous names such as
`identity_transform`, `translation`, `scale`, and `rotation` for constructors
whose result type cannot otherwise be inferred. The implementation plan owns
the final complete signature table, subject to these rules.

Do not introduce `Option` as an incidental language feature. Operations such
as inverse and normalization that cannot produce a valid result return a
runtime error through `opNr`. For intersection, expose `intersects` initially;
an operation returning an optional rectangle is outside this version.

Ordinary Euclid arithmetic retains Euclid's semantics. Checked inverse,
transformed bounds, and checked normalization explicitly reject
invalid/non-finite inputs and non-finite results. Do not silently return an
identity transform, empty rectangle, or unchanged input on failure.

## Language design

### Named construction

Support:

```text
Point2D { x: 0.0, y: 42.0 }
Point2D { y: 42.0, x: 0.0, }
Rect {
    origin: Point2D { x: 0.0, y: 0.0 },
    size: Size2D { width: 640.0, height: 480.0 },
}
```

Struct names resolve in the registered type/constructor namespace, not the
value namespace. A same-named local does not change a struct literal's meaning.
This is a general host-registration facility, not parser knowledge of Euclid.

Each field is required exactly once. Diagnose unknown, missing, duplicate,
and incorrectly typed fields with useful source spans. Allow arbitrary field
order and a trailing comma. Evaluate field expressions exactly once in written
order, even if the constructor's canonical field order differs. Preserve
concrete Rust type identity on the runtime stack.

Initially require explicit `field: expression` entries. Field shorthand and
struct-update syntax are not required; reject them clearly rather than
misparsing them.

### Receiver operations

Support:

```text
p.x
p.x()
rect.center()
rect.origin.x
point + delta
transform.transform_point(point)
first.then(second).transform_point(point)
```

`receiver.name` invokes a unary registered operation. `receiver.name(args...)`
invokes the operation with the receiver followed by the explicit arguments.
In particular, `p.x()` invokes the same unary operation as `p.x`; it does not
call the scalar result of `p.x`. Parenthesized `(p.x)()` does attempt to call
that result under the existing callable-value rules.

Bare member access is not a bound-method value. Do not add callable-field
fallback or mutation. Continue supporting `.0` tuple indexing and existing
chained numeric indices.

Dot and ordinary call syntax use the same underlying registered operation
implementations and type signatures. Ordinary free-function calls retain
normal value-name shadowing. Dot lookup ignores value bindings but honors
the documented operation-scope precedence. A locally declared value named
`x` therefore does not break `point.x`, although it can shadow `x(point)`.

Evaluate the receiver once, then explicit arguments left-to-right. A failed
overload must not consume stack entries or emit part of an operation.

### Predictive grammar

Left-factor the identifier primary and extend the existing postfix loop:

```text
identifier_primary = identifier [ struct_body ].
struct_body = "{" [ field_init { "," field_init } [ "," ] ] "}".
field_init = identifier ":" expression.
postfix_expression = primary_expression {
    "(" [ parameter_list ] ")"
  | "." (tuple_index | identifier [ "(" [ parameter_list ] ")" ])
}.
```

After consuming the common identifier prefix, one next token selects the
struct body or the ordinary identifier interpretation. After a dot, an
identifier selects named access and an unsuffixed integer selects tuple
access. After a member name, `(` selects a member call. Do not add speculative
parsing or semantic type-name lookahead to distinguish productions.

As in Rust, unparenthesized struct literals are restricted where a following
brace opens a control-flow body. Preserve `if flag { ... }` and Adam's
condition/match headers that embed CEL expressions. Parentheses explicitly
re-enable a struct literal within an otherwise restricted expression.
Audit every embedded-expression entry point, not just CEL `if`.

Document the actual restricted productions and their lookahead choices in
the parser. Nested constructor bodies parse their field expressions normally.
Existing expression precedence remains unchanged; named postfix operations
bind at the same level as calls and tuple indexing.

## Registration and toolchain architecture

`cel-parser` owns general operation/record metadata and parser support.
`cel-euclid` supplies concrete registrations. No core parser or runtime module
may depend on Euclid or hardcode its type names.

The current standard-library call protocol pushes a marker for a bare
function name and resolves the subsequent `()` operation. Dot lowering must
not mistake this protocol for an existing direct named unary operation.
Extend registration cleanly so a registered operation can be used from
receiver syntax and ordinary calls without duplicating its implementation.
Do not require rewriting unrelated `cel-std` behavior.

Keep record field schemas and member signatures available to both runtime
compilation and static tooling. The exact typed registration API and stack
adaptation strategy are builder decisions. Prefer existing safe stack and
fragment primitives; do not use raw memory reflection to inspect Euclid.

Carry named construction and receiver syntax through the generic
`ParserContext` pipeline. Preserve their spelling, member/field identifiers,
source order, and spans in dedicated AST nodes rather than erasing them into
ordinary calls before formatting or dependency analysis.

Update all directly affected AST visitors: span access, free-variable and
Adam cell-dependency discovery, closure handling, formatters/comment recovery,
diagnostics, and code-generation consumers in `ez-adam`. Field labels and
member names are not variable references. Keep interpreted and AST-driven
paths consistent.

The current static checker is best-effort and treats custom types and call
results as `Ty::Any`. Do not claim complete inference as part of this project.
Use registration metadata to diagnose new expressions where operand types
are known; recursively check their children even when types are unresolved.
Runtime compilation remains authoritative for overload/type errors.
The implementation plan must explicitly describe any extension of named
type identity needed for these checks rather than silently equating distinct
geometry types.

Expose `cel_euclid::install(&mut OpLookup)` in the style of `cel-std`.
Provide optional Adam integration behind an `adam` Cargo feature, with a
type-registration helper accepting `&mut adam_lang::TypeRegistry`.
Dependency direction is `cel-euclid -> adam-lang` for that optional adapter;
`adam-lang` must not acquire a mandatory geometry dependency.

Configure the opted-in application hosts and `adam-lsp` consistently so the
same geometry sheet can execute and receive editor diagnostics. Update
extension syntax highlighting only where existing rules do not cover the
new forms. Keep base CEL and Adam usable without installing the library.

## Crop constraint expressed in CEL

Do not export a crop-specific `constrain` operation or equivalent Rust
helper. The executable Adam example must express this calculation using
registered general-purpose Euclid operations and ordinary CEL expressions.
This example is an expressiveness requirement for the library surface.

`image` is the rectangular source image in image coordinates. `transform`
maps image coordinates into the coordinate frame containing the axis-aligned
requested `crop`. For straightening, the caller constructs the transform
around the image point underlying the crop center.

The expression computes the largest rectangle obtained by uniformly shrinking
`crop` about its center, with scale in `(0, 1]`, whose four corners map through
the inverse transform into the image rectangle. Preserve the crop's center
and aspect ratio. If it already fits, leave it unchanged.

Require finite, positive image/crop extents, a finite invertible transform,
and a crop-center image anchor strictly inside the image. Document and check
these conditions at the example's input boundary, through checked general
operations and Adam requirements as appropriate. Surface invalid inputs,
infeasible anchors, and unrepresentable results explicitly rather than
silently producing an empty crop. Never translate the crop to disguise an
infeasible anchor or treat a transformed image bounding box as its interior.

The expression works for invertible affine transforms, not only
origin-centered rotation. Inverse-transform the crop into image coordinates,
then use the resulting bounds and anchor-to-image-edge distances to find the
uniform scale with scalar `min`/`max` and arithmetic. Bounding the
inverse-transformed crop against the axis-aligned source image is sufficient;
bounding the transformed image in crop coordinates is not.

Use existing typed, noncapturing closures for intermediate expression values
where helpful; pass their inputs explicitly. Do not add local-binding syntax
or a crop builtin just to make this example shorter. Keep any conservative
rounding and validation visible in the expression/example and document its
numerical tolerance. General-purpose missing primitives belong in the
appropriate library, not behind a crop-specific operation.

The Adam example uses a self-referencing assignment of the composed expression
to `crop`. Test tightening, loosening, returning to the original angle, and
explicit crop edits to prove the source aspiration is preserved by the
existing solver. All geometry operations remain pure and memoryless.

For presentation, derive the inverse crop-shrink factor and compose an
additional image scale about the crop center. This makes the effective crop
fill the unchanged requested overlay while the source image rotates/scales.
Document the coordinate frames and show that the same source-image anchor
remains under the overlay center. Do not build a crop UI.

## Live graph migration

The current renderer lives in `begin/assets/graph.js`; shared
`adam-web-ui::GraphView` drives it for Begin and live book graphs.
Its existing JavaScript helpers and tests provide the behavior baseline.

Move fit, zoom-limit, and pan-conformance arithmetic into an executable Adam
sheet using Euclid values and `cel-std` scalars. D3 retains force layout,
visible-node bounding-box measurement, gesture interpretation, animation,
and SVG rendering. The Adam model, not a parallel JavaScript implementation,
owns the final permitted view.

Use a per-graph model with source inputs for content bounds, viewport size,
and proposed pan/zoom. Self-referencing methods clamp the user's proposed
values without overwriting their source aspiration. Derive the render
transform from the accepted pan/zoom. The precise internal cell decomposition
is a builder decision; do not force a general affine transform into an
API that only supports uniform-scale graph views.

Preserve these observable rules:

- Measure visible node geometry with the current fit margin; empty content
  uses the viewport fallback.
- Initial fit and reset center content at
  `min(viewport.width / max(content.width, 1),
  viewport.height / max(content.height, 1))`.
- Minimum zoom is fit scale; maximum is `max(fit_scale, 8)`.
- If scaled content is smaller than the viewport along an axis, center that
  axis. Otherwise clamp translation to
  `[viewport_extent - scale * content_max, -scale * content_min]`.
- Resize, topology/layout changes, and inactive-node visibility changes
  refresh bounds and conform the view without unconditionally fitting it.
- Preserve drag, wheel/pinch, zoom buttons, reset animations, node dragging,
  and multiple independent graph instances.
- First render waits for a real nonzero viewport; temporary zero-size
  observations do not erase a valid viewport.

Use a lifecycle-owned bidirectional Dioxus evaluation bridge. Desktop
WebView2 cannot synchronously call Rust from D3's JavaScript constrain
callback. Treat gestures as proposed views, propagate in Rust/Adam, and
apply accepted responses under a no-feedback guard. Never render an
unconstrained proposed view as though the model had approved it.

Tag messages with instance generation and request sequence. Reject responses
from destroyed/replaced graphs and stale requests. Coalesce superseded
gesture requests without losing reset or bounds changes. Updating D3's
internal transform must not recursively resubmit the accepted view.
Dispose listeners/tasks/models when their graph instance ends.

The bridge must run on desktop and web, including book-hosted graphs.
Surface parse, propagation, and bridge errors through the host's existing
diagnostic/logging path; do not silently keep executing the old JavaScript
constraint policy on failure. Avoid per-gesture reparsing or rebuilding the
sheet. Verify responsiveness during sustained gestures, not just eventual
agreement after interaction stops.

## Verification and documentation

Derive tests from the public contracts, including:

- Constructors: arbitrary field order, trailing commas, nested records,
  source-order evaluation, duplicate/missing/unknown fields, wrong scalar
  types, unknown constructors, and value/type namespace separation.
- Postfix operations: named access/calls/chaining, tuple indexing coexistence,
  precedence, one-time receiver evaluation, cell/local shadowing, wrong
  arity/types, closure bodies, and no accidental invocation of member results.
- Grammar: restricted control-flow headers, parenthesized constructors,
  embedded Adam expressions, valid legacy `if` expressions, and diagnostics.
- Toolchain: AST round-tripping and comments, source spans, dependency
  discovery, LSP diagnostics, and affected `ez-adam` code generation.
- Library: documented signatures, noncommuting transform order, inverse and
  normalization failures, and standalone use without Adam.
- Crop: zero/nonzero/right-angle rotations, off-center anchors, aspect-ratio
  preservation, maximality, all four actual image-edge constraints, invalid
  geometry, idempotence, and Adam source/seed spring-back.
- Graph: current fit/pan contracts, tiny/empty bounds, aspect-ratio mismatch,
  resize, topology/visibility changes, reset/buttons/gestures, no message
  feedback loops, response ordering, disposal, and multi-instance isolation.

Use existing Cargo, formatter, extension, and `begin` Vitest tooling. Update
directly related CEL/Adam books and library documentation, including the
intentional distinction between dot lookup and shadowable ordinary calls.
Update `docs/VISION.md` where this work supersedes its exploratory geometry
direction.

Run rendered Begin verification using `verifying-begin-ui` on the web build;
inspect live transforms during interaction as well as screenshots/DOM.
Cargo checks alone do not verify this migration.

Before a PR, run the repository's complete build, test, doctest, documentation,
and five-clippy-command gate, with zero plain build/test compiler warnings,
plus affected JavaScript and extension checks.

## Planning and handoff

This is a project requiring a written implementation plan after design
approval. Plan cohesive stages for the general language/registration surface,
geometry library and Adam examples, and graph bridge/migration. Share one
operation schema rather than independently inventing parser/tooling APIs.

Before coding, perform the required algorithm-research pass. Follow the
repository's contract documentation, test-first, independent review, and
rendered UI requirements. If work is delivered over multiple sessions or
PRs, maintain a dated handoff under `docs/superpowers/` stating completed,
deferred, and remaining work. Do not merge or open a PR without a user request.
