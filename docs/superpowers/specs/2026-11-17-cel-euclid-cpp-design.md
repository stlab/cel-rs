# C++ `cel-euclid` port

## Intent

Deliver a C++ library that exposes the same two-dimensional geometry types,
expressions, operations, and observable behavior as Rust's `cel-euclid`.
Applications should be able to move supported CEL geometry expressions
between the Rust and C++ language implementations without changing their
meaning. Use GLM for C++ geometry calculations and `cpp-library` for package
structure, build integration, tests, examples, and documentation.

The C++ language supports `bool` and `f64` values, not integer values. A
literal such as `1f64` is an `f64` literal; an integer literal or an integer
cast such as `1 as f64` remains unsupported.

The work spans the C++ `cel-runtime`, `cel-parser`, and a standalone
`cel-euclid` repository. The current Rust library and its geometry reference
define the operation set and behavior.

## Goals

- Preserve the seven nominal CEL record schemas, field names, field order
  semantics, ordinary function calls, receiver calls, and operators defined by
  Rust `cel-euclid`.
- Add a reusable C++ extension point that adapts a nominal CEL record to and
  from a typed C++ value for operation execution. Keep canonical CEL record
  storage and never assume that a native C++ or GLM object has the record's
  byte layout.
- Use GLM-backed C++ geometry values and register all operations in the Rust
  library's supported-signature table.
- Match successful results, overload selection, and checked-operation failure
  conditions. C++ reports runtime operation failures through the exception
  conventions of the C++ runtime.
- Package the geometry library as a standalone `cpp-library` project using
  CPM for dependencies.
- Provide a tutorial and API reference. Link each operation entry to the
  corresponding Rust CEL geometry documentation.
- Verify cross-language behavior with the same representative expressions and
  contract-derived expected results.

## Non-goals

- Change Rust `cel-euclid` or the Rust geometry contract, or add integer values
  to the C++ language.
- Add three-dimensional geometry, generic unit parameters, implicit numeric
  conversions, or new geometry operations.
- Store GLM objects by reinterpret-casting or aliasing CEL record storage.
- Make the C++ implementation depend on the Rust implementation at runtime.
- Port geometry-specific Adam integrations or UI examples.

## Constraints

- Keep the adapter generic to nominal CEL records and independent of GLM.
- Keep parser selection based on complete nominal input descriptors, not
  display names or field layout alone.
- Preserve arbitrary field order in record literals; the declared schema order
  remains canonical for conversions.
- Use binary64 (`double`) components and GLM double-precision types.
- Keep operation semantics aligned with `cel-euclid/SIGNATURES.md`, including
  half-open ordinary rectangle/box containment and the explicitly checked
  behavior of `normalize`, `inverse`, and `outer_transformed_rect`.
- Follow the C++ repositories' existing `cpp-library`, CPM, test, and
  documentation conventions. Keep the geometry package's CMake configuration
  as small as the `cel-std` package permits.
- Coordinate dependency versions so the geometry package consumes the C++
  parser/runtime revisions that provide the adapter.

## Approaches

### Preserve canonical records and add a typed adapter

Keep records in the runtime's existing canonical field representation. Add a
generic adapter that associates a nominal record descriptor with a C++ carrier
and explicitly converts between the record fields and that carrier at the
operation boundary. Let the parser register exact operations over the record
descriptors while invoking ordinary typed C++ callbacks.

This preserves CEL syntax and semantics, keeps the core representation
independent of GLM, and gives later libraries a reusable extension. It adds
work to the C++ runtime and parser and requires explicit conversions.

### Lower every record operation into field-level instructions

Have the parser or each extension expand a record operation into individual
field reads, scalar operations, and record construction. This avoids a native
record adapter, but pushes geometry-specific conversion and operation logic
into emitters. It also makes multi-field operations harder to express and
duplicates low-level code across libraries.

### Register GLM values as opaque CEL leaves

Register GLM vectors and matrices directly as leaf types and expose constructor
functions. This needs less record adaptation but changes expressions such as
`Point2D { x: 1.0, y: 2.0 }` and member access, so it does not meet the
cross-language syntax requirement.

**Recommended approach:** preserve canonical records and add the generic typed
adapter. This is the only approach that preserves the existing CEL record
contract without making extension libraries depend on record-layout internals.

## Design

### Runtime adapter

Extend `cel-runtime` with an explicit, reusable adapter contract for a
trivially copyable or otherwise safely owned C++ carrier backed by a nominal
record descriptor. The adapter supplies the complete record type and
conversion between canonical CEL record values and the C++ carrier. Conversion
must validate the complete record shape and nominal identity. It must not
reinterpret the record's bytes as a native object.

Add a typed operation path that consumes and produces these adapted values.
The existing exact-type checks, callback ownership, execution cleanup, and
error propagation remain in force. A callback exception propagates, and the
temporary execution suffix remains safely discardable. Conversions must preserve
the record's field names, types, and schema order, including nested records.

### Parser registration

Extend `cel-parser` registration so an extension can register an adapted
nominal type and exact operations using the runtime adapter. The parser
continues to resolve record literals and member projections through its
existing schema metadata. Operation selection remains based on full nominal
descriptors; return-type-only overloading remains unsupported. Existing
`register_record`, typed `register_op`, and ordinary scalar overloads keep
their behavior.

The implementation plan will settle the public C++ adapter API after checking
the runtime value-construction and operation-emission boundaries. It must keep
the runtime independent of the parser and avoid adding GLM-specific behavior to
either core library.

### Geometry package

Create a standalone `cel-euclid` C++ package using the existing C++ package
pattern: CPM loads `cpp-library`, `cel-parser`, `cel-runtime` through the
parser dependency, and GLM. The package registers the seven CEL records and
the complete 98 exact signatures in `cel-euclid/SIGNATURES.md`.

Use distinct C++ carrier types where needed to preserve the semantic
distinctions among points, vectors, sizes, angles, and transforms. Use GLM
double-precision values for component and matrix calculations. Convert
explicitly at the adapter boundary. Preserve the Rust matrix field mapping and
`a.then(b)` order; do not assume GLM's matrix storage or multiplication order
matches Euclid's representation.

The package exposes an installer analogous to `cel::stdlib::install`. It
registers schemas, constructors through record literals, operators, and named
operations so ordinary calls and receiver syntax select the same signatures.
The scalar `cel-std` `min` and `max` overloads remain usable alongside the
geometry overloads.

### Documentation and verification

The tutorial teaches a C++ consumer to install the library, compile a record
literal and receiver expression, prepare and execute the result, and handle
checked-operation errors. The API reference lists every supported signature,
field schema, numeric-cast rule, and failure contract. Each operation links to
its Rust geometry-book entry.

Tests cover record construction with reordered fields, nested records, member
access, overload coexistence, each operation signature, matrix order, and
checked failures. Reuse identical CEL expression cases in Rust and C++ tests
where practical; compare exact values for exact contracts and documented
tolerances for floating-point operations. Validate installation generation
through the existing `cpp-library` package check; installed-consumer
compatibility is deferred until the dependency releases.

## Repository sequence

1. Implement and test the generic runtime adapter in `stlab/cel-runtime`.
2. Integrate exact record-aware registration in `stlab/cel-parser`, including
   tests that exercise the public CEL record syntax.
3. Implement and document the standalone `stlab/cel-euclid` package against
   those revisions.

Keep each repository's changes independently buildable and testable. Pin the
geometry package to exact parser/runtime commit revisions while the adapter is
unreleased. Replace those pins with release tags when the prerequisite changes
are published.
