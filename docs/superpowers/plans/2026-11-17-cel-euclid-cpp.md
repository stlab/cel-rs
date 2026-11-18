# C++ `cel-euclid` Implementation Plan

> **Status, 2026-11-17:** This historical plan does not track completion.
> Implementations are open in [cel-runtime #8](https://github.com/stlab/cel-runtime/pull/8),
> [cel-parser #2](https://github.com/stlab/cel-parser/pull/2),
> [cel-rs #292](https://github.com/stlab/cel-rs/pull/292), and
> [cel-euclid #1](https://github.com/stlab/cel-euclid/pull/1).
> Parser registration uses an explicit CEL name. Dependency releases and
> installed-consumer validation remain in
> [cel-euclid #2](https://github.com/stlab/cel-euclid/issues/2).

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a standalone C++ `cel-euclid` library whose supported CEL expressions and geometry behavior match the Rust library.

**Architecture:** Add a generic record adapter to the C++ runtime, expose it through exact nominal registration in the C++ parser, then implement the standalone GLM-backed geometry package. Preserve the runtime's canonical record storage and marshal explicitly at typed callback boundaries.

**Tech Stack:** C++20, `cel-runtime`, `cel-parser`, GLM 1.0.3, CPM, `cpp-library` 6.0.1, CMake, doctest.

**Spec:** `docs/superpowers/specs/2026-11-17-cel-euclid-cpp-design.md`; Rust contract: `cel-euclid/SIGNATURES.md` and `cel-lang-book/book-src/geometry.md`.

## Global Constraints

- Keep the adapter generic to nominal CEL records and independent of GLM.
- Keep parser selection based on complete nominal input descriptors, not display names or field layout alone.
- Preserve arbitrary field order in record literals; the declared schema order remains canonical for conversions.
- Use binary64 (`double`) components and GLM double-precision types.
- Keep the C++ scalar value domain limited to `bool` and `f64`: accept `1f64`
  as an `f64` literal, but reject integer values and integer casts such as
  `1 as f64`. Preserve existing identity casts on supported types.
- Keep operation semantics aligned with `cel-euclid/SIGNATURES.md`, including half-open ordinary rectangle/box containment and the explicitly checked behavior of `normalize`, `inverse`, and `outer_transformed_rect`.
- Preserve Euclid's separate floating-point rounding in optimized builds;
  implicit fused multiply-add contraction must not change geometry results.
- Follow the C++ repositories' existing `cpp-library`, CPM, test, and documentation conventions. Keep the geometry package's CMake configuration as small as the `cel-std` package permits.
- Give each public C++ declaration a concise contract and usage example; derive tests from its observable behavior.
- Coordinate dependency versions so the geometry package consumes the C++ parser/runtime revisions that provide the adapter.
- Pin unreleased parser/runtime dependencies to exact commits until their
  releases; then replace the pins in dependency order with released versions.
- Validate that installation artifacts are generated; installed-consumer
  compatibility and unreleased package versioning are out of scope.
- Make changes in dedicated worktrees for `stlab/cel-runtime`, `stlab/cel-parser`, and `stlab/cel-euclid`; never change their `main` branches directly.
- Run each repository's required checks, commit its changes on the worktree branch, and open a PR for review. Do not merge without an explicit request.
- Pin exact runtime/parser commit revisions during development; replace those pins with release tags when the prerequisite changes are published.
- Create the new `stlab/cel-euclid` repository as a repository-setup step; keep all library implementation changes on its feature worktree branch and deliver them through a PR.

## Review Focus

- Reordered and nested record fields must map to the declared schema, not written order; test record round trips and `Rect`/`Box2D` literals with reordered fields.
- Point, vector, size, angle, and transform identities must remain distinct even when their GLM-backed components share layouts; test exact overload selection and wrong-type compilation failures.
- Geometry `min`/`max` overloads must coexist with scalar `cel-std` overloads in either installation order; test both.
- Empty, negative-size, reversed, and nonfinite rectangles and boxes must follow
  Rust Euclid's existing operation semantics rather than being normalized or
  rejected by new checks.
- Euclid composition order and coordinate mapping must survive GLM's matrix conventions; test `translation(...).then(scale(...))` maps `(1,0)` to `(22,0)` and verify matrix fields.
- Nonfinite translation must not contaminate `transform_vector`: Euclid ignores
  translation for vectors, including when a homogeneous matrix calculation
  would introduce `0 * infinity`.
- Non-finite, singular, underflowing, and unrepresentable checked-operation inputs must fail rather than yield plausible geometry; test the Rust contract's checked cases.

---

### Task 1: Add nominal-record adapters to `cel-runtime`

**Repository:** `stlab/cel-runtime`, in a dedicated worktree based on its default branch.

**Files:**
- Create: `include/cel/runtime/record_adapter.hpp`
- Modify: `CMakeLists.txt`, `include/cel/runtime/runtime.hpp`, `include/cel/runtime/value.hpp`, `include/cel/runtime/dyn_segment.hpp`
- Modify as needed for the implementation: `src/value_ops.hpp`, `src/type_ops.hpp`, `src/dyn_segment.cpp`, `src/value.cpp`
- Test: `tests/value_test.cpp`, `tests/segment_test.cpp`

**Interfaces:**
- Produces the `cel::record_adapter<T>` customization point with `static value_type type()`, `static value to_value(const T&)`, and `static T from_value(const value&)`.
- `value::from<T>`, `value::as<T>`, `native_type<T>`, and typed `dyn_segment::op<R, A...>` accept `T` when `record_adapter<T>` is specialized.
- The adapter's descriptor is a nominal record. `value::as<T>` checks the complete identity and schema before decoding; `value::from<T>` verifies that encoding returns the declared schema. Mismatches throw `std::invalid_argument`; conversion never reinterprets canonical record bytes as `T`.

- [ ] **Step 1: Add a test adapter and contract tests** in `tests/value_test.cpp`. Specialize `cel::record_adapter` for a test record containing a scalar and a nested record. Assert `value::from(record).as<T>()` round-trips the values, the resulting descriptor retains the nominal identities and schema, and applying `from_value` to a different record schema throws `std::invalid_argument`.
- [ ] **Step 2: Run the focused value test** with the repository's configured CMake build. Expected: the test does not compile because `record_adapter<T>` is not supported.
- [ ] **Step 3: Add the runtime adapter customization point** in `include/cel/runtime/record_adapter.hpp`, include it from `runtime.hpp`, and route `native_type<T>`, `value::from<T>`, and `value::as<T>` through it for specialized types. Document nominal schema matching, conversion ownership, and errors in the public contract and `docs/libraries.md`.
- [ ] **Step 4: Add a typed execution test** in `tests/segment_test.cpp`. Emit a `dyn_segment` operation from an adapted record to another adapted record; assert correct execution and that a throwing callback propagates while the stack suffix remains safely discardable.
- [ ] **Step 5: Implement adapted typed operands and results** in the runtime operation path. Preserve exact descriptor checks and runtime ownership; invoke the registered field-wise adapter rather than accessing record bytes as `T`.
- [ ] **Step 6: Run runtime checks:** configure with `cmake -S . -B build -DBUILD_TESTING=ON`, then run `cmake --build build --parallel` and `ctest --test-dir build --output-on-failure`. Expected: all runtime tests pass, including the new record adapter cases.
- [ ] **Step 7: Commit and open the runtime PR.** Include only runtime adapter code and tests; describe parser/library use as a downstream consumer.

### Task 2: Register adapted records and operations in `cel-parser`

**Repository:** `stlab/cel-parser`, in a dedicated worktree. Build against the runtime adapter commit from Task 1.

**Files:**
- Modify: `include/cel/parser/lookup.hpp`, `src/lookup.cpp`
- Modify: `tests/lookup_test.cpp`, `tests/expression_test.cpp`, `docs/libraries.md`
- Modify the CPM runtime dependency pin in `CMakeLists.txt` to the exact runtime
  revision that provides the adapter.

**Interfaces:**
- Adds `template<class T> void op_lookup::register_record_type()`, which registers the schema returned by `cel::record_adapter<T>::type()`.
- Existing `register_op<R, A...>` uses `native_type<T>()` for adapted records and continues to resolve by full nominal operand descriptors.
- Record literals and member projections use the existing grammar and schema path; no parser grammar changes are required.

- [ ] **Step 1: Add `record_adapter_registration_preserves_nominal_types_and_member_calls`** to `tests/lookup_test.cpp`. Register a test adapter, register an operation over it, compile and execute a reordered record literal and receiver call, and assert the result. Add a same-layout second nominal record and assert that it does not select the first record's overload.
- [ ] **Step 2: Run the focused parser test.** Expected: the test fails because `register_record_type<T>()` is not available.
- [ ] **Step 3: Implement `op_lookup::register_record_type<T>()`** in `include/cel/parser/lookup.hpp`. Require `record_adapter<T>::type()` to describe a valid nominal record, register its identity and schema, and preserve existing duplicate/reserved-name errors.
- [ ] **Step 4: Add parser failure tests.** Assert that duplicate names and unknown/mismatched record fields fail during compilation and that an unsupported operand type does not invoke a registered callback.
- [ ] **Step 5: Document adapted record registration** in `docs/libraries.md`, including adapter specialization, exact nominal matching, source-order field evaluation, and callback lifetime.
- [ ] **Step 6: Run parser checks:** configure and build with the repository CMake options, then run `ctest --test-dir build --output-on-failure`. Expected: parser, runtime-dependent, and installed-consumer tests pass.
- [ ] **Step 7: Commit and open the parser PR** against the runtime adapter revision. Do not merge either PR without explicit approval.
- [ ] **Step 8: Add and run failing literal tests.** Assert `1f64` and other
  supported integer-form `f64` literals evaluate as `f64`; bare integers and
  `1 as f64` remain rejected. Preserve existing floating forms and invalid
  suffix handling.
- [ ] **Step 9: Fix `f64` literal recognition** at the parser/lexer boundary,
  without adding integer values or implicit numeric conversions. Match the
  Rust spelling and value for the tested shared-domain literal forms.
- [ ] **Step 10: Run parser checks and validate installation generation.**
  Commit the literal fix and update the parser PR. Preserve the exact runtime
  commit pin; do not change package versioning before release.

### Task 2a: Align Rust floating-point literal lexing

**Repository:** the existing `cel-rs` worktree, followed by the C++ parser
worktree for its shared parity corpus.

**Interfaces:**
- Rust CEL recognizes decimal literals with `f32` or `f64` suffixes as the
  corresponding floating-point values, including integer-form spellings such
  as `1f64`, while preserving the original span and spelling.
- Integer values and casts remain available in Rust; the C++ language remains
  limited to `bool` and `f64`, without integer values or integer casts.
  Existing identity casts on supported types remain available.
- Lexing, retained ASTs, validation, execution, literal patterns, and tuple
  selectors continue to agree on the literal's type.
- Both CEL parsers follow Rust's exponent spelling: an optional sign comes
  immediately after `e` or `E`, before any underscores.

- [ ] **Step 1: Add failing Rust literal tests** for `1f64`, `1_f64`, `0f64`,
  `1f32`, and their values, spans, and retained AST types. Cover malformed
  suffixes, radix-prefixed integers, suffixed tuple selectors, and adjacent
  range punctuation without changing their existing behavior.
- [ ] **Step 2: Run the focused Rust tests** and verify they fail for the
  current `f64`/`f32` suffix misclassification.
- [ ] **Step 3: Correct the shared lexical/literal path** without adding
  parser lookahead, implicit conversions, or rewritten source spellings.
  Keep the normal Rust and C++ numeric boundaries distinct.
- [ ] **Step 4: Add C++/Rust parity fixtures** for the supported shared `f64`
  spellings, and rerun the Rust tests and C++ differential tests against this
  Rust worktree. Reject misplaced signs such as `1e_+2f64`; accept valid forms
  such as `1e+_2f64` and `1e__2f64`.
- [ ] **Step 5: Commit the Rust fix and C++ parity update** separately. Leave
  the C++ PRs open and preserve the exact dependency pins.

### Task 3: Build the C++ geometry records and non-transform operations

**Repository:** continue in the existing `stlab/cel-euclid` feature worktree. During development, pin parser/runtime CPM dependencies to the exact prerequisite commits and the test-only `cel-std` dependency to v1.0.3.

**Files:**
- Create: `CMakeLists.txt`, `cmake/CPM.cmake`, `include/cel/euclid.hpp`, `src/euclid.cpp`
- Create: `src/records.hpp`, `src/geometry.cpp`
- Create: `tests/euclid_test.cpp`
- Create: `licenses/GLM-LICENSE.txt`, `licenses/CPM-MIT.txt`
- Modify: `README.md`

**Interfaces:**
- Produces `void cel::euclid::install(cel::op_lookup&)`.
- Registers record adapters for `Point2D`, `Vector2D`, `Size2D`, `Rect`, `Box2D`, `Angle`, and `Transform2D`, with the exact field schemas and order in `cel-euclid/SIGNATURES.md`.
- Uses distinct public carrier types for semantically distinct values; each stores or composes GLM double-precision values and converts to/from canonical CEL records by named fields.
- Registers every non-transform, non-checked signature in the Rust table, including equality, finite checks, arithmetic, accessors, bounds operations, containment, conversion, angles, and scalar overload coexistence.
- Owns all seven equality and `is_finite` overloads and both directions of
  `degrees` and `radians`; Task 4 must not register them again.

- [x] **Step 1: Create the public GitHub repository `stlab/cel-euclid` and establish a feature branch worktree.** The metadata-only default branch and separate feature worktree already exist.
- [ ] **Step 2: Add record and operation contract tests** named `record_schemas_match_rust`, `reordered_fields_preserve_named_values`, `point_vector_and_size_arithmetic_match_rust`, `rect_and_box_boundaries_match_rust`, and `geometry_min_max_coexist_with_scalar_min_max`. Exercise `Point2D { y: 2.0, x: 1.0 }`, nested `Rect` and `Box2D` values, receiver syntax, scalar `min`/`max`, and both installation orders. Verify reordered field expressions execute once in written order. Compare empty, negative-size, reversed, NaN, infinite, and signed-zero bounds behavior with Rust rather than normalizing or rejecting inputs.
- [ ] **Step 3: Add the minimal CPM/cpp-library CMake project.** Pin the
  released GLM 1.0.3, `cel-std` v1.0.3 (tests only), and `cpp-library` 6.0.1
  to their verified commits `8d1fd52e5ab5590e2c81768ace50c72bae28f2ed`,
  `493acb275cc59cdcf802175d07c806aeefed8686`, and
  `13be83b3d92aeb1969fb6f8d13e81b372bb57d43`; pin the parser/runtime
  revisions from Tasks 1–2. Link the exported parser target, which supplies
  the runtime dependency, and export GLM when its types appear in public
  headers. Resolve the parser and runtime before test-only `cel-std`, so
  `cel-std` cannot select older revisions. Use `cpp_library_setup` for the
  public header, source, and tests; Task 5 adds its example and docs once
  those files exist. Include and install the unmodified upstream GLM and CPM
  license notices.
- [ ] **Step 4: Run the record and operation tests.** Expected: compilation succeeds, then the tests fail because the geometry installer and registrations are missing.
- [ ] **Step 5: Implement the seven record adapters** in `src/records.hpp` and register their schemas from `install`. Map every field by name; do not rely on layout or field write order. Document each public carrier type and adapter contract in `include/cel/euclid.hpp`.
- [ ] **Step 6: Implement non-transform operations** in `src/geometry.cpp` with GLM double-precision vectors where applicable. Preserve Euclid arithmetic and half-open containment semantics; implement exact-type comparisons for each nominal schema.
- [ ] **Step 7: Add a signature coverage test.** Compile and evaluate every non-transform, non-checked signature against representative Rust results; assert wrong operand types and arities fail during parsing. Verify all seven `is_finite` overloads inspect every stored component, `1f64` is an `f64` literal, integer values and integer casts remain unsupported, and supported identity casts remain available.
- [ ] **Step 7a: Preserve rounding in optimized builds.** Test the dot product
  of `(1 + 2^-27, 1)` and `(1 - 2^-27, -1)`: Euclid returns zero, while a
  fused multiply-add returns `-2^-54`. Ensure the geometry target, including
  later transform sources, retains Euclid's separate rounding in Release.
- [ ] **Step 8: Run focused C++ tests.** Expected: all record, arithmetic, member access, boundary, and `cel-std` overload tests pass.

### Task 4: Add GLM affine transforms and checked geometry operations

**Repository:** continue in the `stlab/cel-euclid` worktree.

**Files:**
- Create: `src/transforms.cpp`
- Modify: `include/cel/euclid.hpp`, `src/euclid.cpp`, `CMakeLists.txt`
- Create: `tests/transforms_test.cpp`

**Interfaces:**
- Completes all 98 Rust-supported signatures, including matrix field accessors, constructors, composition, determinant, inverse, point/vector mapping, and transformed bounds.
- Checked operations throw evaluation exceptions for the same invalid domains/results rejected by Rust `cel-euclid`; ordinary arithmetic retains floating-point results.
- Uses the record, equality, finite-check, and angle registrations already
  provided by Task 3.

- [ ] **Step 1: Add transform and normalization tests** named `transform_then_order_is_observable`, `rotation_preserves_anchor`, `inverse_round_trips_points`, `singular_or_nonfinite_inverse_errors`, `inverse_rejects_complete_coefficient_underflow`, `inverse_preserves_exact_zeros_and_representable_subnormals`, `checked_normalization_rejects_invalid_lengths`, `matrix_fields_match_euclid_mapping`, and `ordinary_transform_operations_do_not_gain_finite_checks`. Mirror the Rust fixtures in `cel-euclid/tests/transforms.rs` and `cel-euclid/tests/geometry.rs`. Assert translation by `(10,0)` followed by scale `(2,2)` maps `(1,0)` to `(22,0)`, and reverse order maps it to `(12,0)`. A vector transformed by an otherwise finite matrix with infinite translation must retain its finite linear result. Preserve Euclid's positive-zero off-diagonal coefficients for the identity inverse and zero-angle rotation, and verify `scale(0.1, 0.1).inverse().m11` is exactly `10.0` as in Euclid's reciprocal-then-multiply evaluation.
- [ ] **Step 2: Add outer-bound tests** named `inverse_crop_bounds_enclose_all_corners`, `transformed_image_bounds_are_not_an_interior_test`, `checked_outer_bounds_errors`, `outer_bounds_accept_tiny_and_degenerate_geometry`, `outer_bounds_round_outward_when_extent_subtraction_rounds_inward`, and `outer_bounds_reject_overflowing_extents_between_finite_corners`. Mirror the Rust fixtures for rotation, reflection, shear, zero-size and tiny rectangles, negative extents, non-finite coefficients, endpoint overflow, projected-span underflow/collapse, and inward-rounding correction. For a transform `(-1, 0, 0, 1, -0, 0)` and rectangle origin `(-0, -0)` with size `(0, -0)`, assert the output width is positive zero, matching Euclid's first-tie extrema.
- [ ] **Step 3: Run the transform and outer-bound tests.** Expected: the tests compile and fail because the transform registrations and checked operations are missing.
- [ ] **Step 4: Implement transform operations** in `src/transforms.cpp`, using the `Transform2D` adapter and schema from Task 3. Convert explicitly between the Rust matrix field mapping and GLM's matrix convention; verify the `then` order with the noncommuting test rather than assuming GLM's multiplication order. Match Euclid's operation order and floating-point edge behavior, using explicit formulas where GLM rearranges arithmetic. Avoid homogeneous-coordinate products that add terms absent from Euclid's affine operations.
- [ ] **Step 5: Implement checked `normalize`, `inverse`, and `outer_transformed_rect`.** Match the exact conditions in `cel-euclid/SIGNATURES.md`, including non-finite/zero cases and the inverse cofactor checks. Compare exact binary64 products portably by factoring powers of two and using `std::gcd` for significand cancellation; do not rely on nonportable `__int128`. Preserve the first equal minimum and maximum, as Euclid does; `std::minmax_element` instead returns the last equal maximum.
- [ ] **Step 6: Assert full signature coverage.** Ensure all 98 exact signatures compile, including `min(Box2D)`/`max(Box2D)` alongside scalar `cel-std` overloads.
- [ ] **Step 7: Run all C++ library tests** with the configured CMake build. Expected: geometry and transform tests pass without fast-math flags.

### Task 5: Document, package, and publish the C++ library PR

**Repository:** continue in the `stlab/cel-euclid` worktree.

**Files:**
- Create: `docs/tutorial.md`, `docs/reference.md`, `examples/example.cpp`
- Modify: `CMakeLists.txt`, `README.md`, `include/cel/euclid.hpp`
- Track the `cpp-library`-generated formatting, ignore, presets, CI, and
  documentation support files required by the package, following `cel-std`.

**Interfaces:**
- Public package target: `cel::euclid`; public header: `<cel/euclid.hpp>`.
- Tutorial: install the library, compile a reordered record literal and receiver expression, prepare/execute a result, and handle checked-operation errors.
- Reference: include every supported signature and link each operation entry to its corresponding Rust geometry-book entry.
- The published Rust reference is
  `https://stlab.github.io/cel-rs/cel-book/geometry.html`.
  Live operator anchors are `#equality--`, `#addition-`, `#subtraction--`,
  `#negation-unary--`, and `#scaling--`; grouped accessor anchors are `#x-y`,
  `#width-height`, `#min-max`, `#min_x-min_y-max_x-max_y`, and
  `#m11-m12-m21-m22-m31-m32`. Named operation headings use their exact names.

- [ ] **Step 1: Write the runnable example** in `examples/example.cpp` using `cel::euclid::install`, `<cel/parser.hpp>`, and `<cel/euclid.hpp>`. Compile and execute a nested record expression and an affine transform expression.
- [ ] **Step 2: Write the tutorial and API reference.** Document record schemas, field-order behavior, the `bool`/`f64` value boundary and floating literal forms, receiver syntax, matrix mapping and composition order, checked failures, installation order, and all Rust-book links.
- [ ] **Step 3: Validate installation generation** with the existing
  `cpp-library` package-artifact check. Do not add installed-consumer
  compatibility tests before the dependencies are released.
- [ ] **Step 3a: Include generated project support files.** Preserve the
  `cpp-library` templates needed for formatting, CMake presets, CI, ignored
  build artifacts, and the documentation entry point; compare `cel-std`.
- [ ] **Step 4: Run the package gate:** configure with `cmake -S . -B build -DBUILD_TESTING=ON -DBUILD_DOCS=ON -DCEL_INSTALL=ON`; run `cmake --build build --parallel` and `ctest --test-dir build --output-on-failure`. Expected: unit tests, examples, documentation generation, and the installation-artifact check pass.
- [ ] **Step 5: Review dependency pins and exported targets.** Confirm the package uses exact parser/runtime commits containing the adapter and GLM 1.0.3, and that consumers do not need a separate runtime link.
- [ ] **Step 6: Commit and open the `cel-euclid` PR** after the runtime and parser PRs are available. Keep all PRs open for review; do not merge without an explicit request.
