# Parameterized sheets and reusable relationships

Date: 2026-10-25

Status: approved by the user on 2026-10-25. The linked implementation now supplies
the contracts below, with public regression coverage and a local full-gate
record in the [2026-10-26 implementation handoff](../2026-10-26-parameterized-sheets-handoff.md).
The independent whole-branch review found a conditional runtime provenance gap,
inconsistent reserved-name acceptance and stale milestone status. The consolidated
local fix wave addresses those findings without changing the approved semantics.
Controller-scoped re-review and publication remain pending; this is not a
merged/released completion claim.

Issue: #29, v1.0. This replaces its original equation-inference scope.

## Intent

A reusable sheet describes a complex relationship through authored directional
methods, private helper cells, and optional conditional relationship groups.
A containing sheet binds its existing cells to the reusable sheet's parameters.
Composition constructs one planning graph, not independent solvers exchanging
values.

The motivating examples are reusable inequality and multiplication relationships,
resolution/aspect-ratio constraints from image resizing, and composition of
resizing with crop/straighten. A sheet must be explicitly authored for reuse;
merging arbitrary running sheets is not a goal.

## Agreed boundaries

- Every parameter has an explicit concrete type. Language generics are deferred
  to #262, outside v1.0.
- Uses bind cells by parameter name. Each parameter is supplied exactly once,
  and one actual cell cannot fill two parameters in a single use.
- Actual cells may be shared between different component instances.
- Callers supply boundary-cell initial values, filters, and requirements.
  Component-specific value obligations are documented preconditions, not new
  parameter-level filter/require syntax.
- Out-cell arguments are unsupported initially, including read-only uses of
  caller outputs. Their design is deferred to #261, outside v1.0.
- Source arguments remove whole methods that write them. A relationship left
  without a method produces an instantiation diagnostic.
- Component bodies support ordinary private cells, sources, outputs, filters,
  requirements, relationships, conditionals, and nested component uses.
- A use within a conditional branch may expand only to relationships, with no
  private cells or conditionals. Relaxing this is deferred to #264, outside v1.0.
- Inline equations and inferred method directions are deferred to #263, outside
  v1.0. Components supply their directional methods explicitly.
- Definitions may appear inline in a file or be brought into scope by Rust-like
  `use`. Clients supply module collections; the core library does not load files.
- Clients can obtain templates from parsed files or build them programmatically.
  Native construction and instantiation do not require parsing source text.

## Language

### Definitions

```adam
/// Keeps a <= b. Bound cells must support the intended finite-value policy.
sheet ordered(a: f64, b: f64) {
    relationship {
        a := min(a, b);
        b := max(a, b);
    }
}
```

A parameter list marks an explicitly reusable definition. A declaration without
one remains an ordinary root sheet:

```adam
sheet editor {
    cell low: f64 = 0.0;
    cell high: f64 = 1.0;
    relationship ordered(a: low, b: high);
}
```

Parameters are cell bindings, not evaluated function arguments. They have no
default values, initializers, or `source`/`out` modifiers. A parameter can be read
or written by authored methods; the actual cell kind controls specialization.
Parameter names and private declaration names occupy the component's cell
namespace and cannot collide.

Types use the existing complete type-expression model, including registered
host types and compound shapes. Binding checks require exact resolved type
identity, not numeric promotion or carrier `TypeId` equality alone.

Builder choice: permit `sheet helper()` as an explicitly reusable zero-parameter
component. A root can instantiate it through `relationship helper();`.

### Uses

```adam
relationship multiply(a: width, b: height, c: area);
relationship multiply(a: inches, b: resolution, c: pixels);
```

Bindings are named and may appear in any order. No positional arguments or
name-punning shorthand are needed initially. Each actual is a cell identifier,
not an arbitrary expression, tuple field, or implicit temporary.

The containing scope must already have declared each actual cell. The component
cannot capture other cells from the caller. Private names do not enter the
caller namespace, and users do not access results through instance members.
Values that must participate in surrounding relationships are parameters.

Builder choice: uses do not require user-visible instance names. Construction
assigns opaque instance identities and retains invocation paths for inspection
and diagnostics. Two uses of the same template remain distinguishable.

### Files and imports

```adam
use constraints::ordered;
use image::dimensions as image_dimensions;

sheet editor {
    // Caller-owned cells.
    // relationship ordered(a: minimum, b: maximum);
}
```

A source file is a module containing file-level imports and sheet definitions.
All definitions in the same file are available for lookup; forward references
between definitions are allowed. Cell declarations within a body keep their
existing declaration-order rules.

The client registers modules under stable paths. Resolution uses that supplied
collection; it does not inspect the working directory, fetch dependencies, or
infer filesystem paths. Tools can implement conventional loaders separately.
Programmatic templates participate in the same definition lookup as parsed ones.

Builder choices for the initial import subset:

- Support explicit paths and optional `as` aliases; defer glob/group imports,
  re-exports, visibility modifiers, and filesystem module declarations.
- A path selects a module and a definition from its declared exports. Initially
  all file-level sheet definitions are exported.
- Duplicate local definitions/imported names are diagnostics, not shadowing.
- Resolve names in the definition's module, never the invocation's module.
- Cyclic imports alone need not imply recursive expansion: registration supplies
  the module graph before resolution. Diagnose actual component-use cycles.

Root selection is explicit by name when a module has multiple roots. Retain a
single-root convenience API for existing one-sheet files, but do not pick an
arbitrary definition from a multi-root module.

## Composition semantics

Instantiation behaves as if the component's private declarations and
relationships were inserted at the invocation site, in their original order,
with hygienically renamed private cells and parameters bound to existing cells.
Nested uses expand depth-first at their written positions.

This is a semantic model, not a textual substitution implementation.

- Bound cells retain their identities, initial/effective values, kinds,
  priorities, filters, and requirements. Binding does not perform a write.
- Each instance allocates its own private cells. Initialization and the relative
  initial strengths of those cells follow ordinary inline construction.
- Internal initializers retain existing language rules; this feature does not
  add cell-dependent initialization expressions.
- Every authored relationship remains a separate relationship. Its alternative
  methods remain alternatives of that relationship, not of the whole component.
- Existing self-reference, source aspiration, guard-independence, filter,
  requirement, and propagation rules continue to apply to the combined graph.
- Shared boundary cells allow the one planner to choose directions across
  components. There is no component-local propagation or value-copy bridge.
- Internal outputs retain ordinary sole-writer semantics and stay private.
- Instantiation is construction, not propagation. It does not run a solver or
  automatically refresh existing derived values.

### Source specialization

For each authored method, map its outputs to actual cell identities. If any
output is `Source` kind, discard that entire method. Do not remove individual
outputs or project a tuple result.

```adam
source upper: f64 = 10.0;
cell value: f64 = 4.0;
relationship ordered(a: value, b: upper);
```

Only `a := min(a, b)` survives. The method writing `b` is removed.

Perform specialization transitively using actual kinds, including private
sources and sources passed through nested parameters. Ordinary method bodies
must be well-typed before specialization; a discarded direction is not a way
to hide an invalid definition.

If any constituent relationship loses all its methods, reject the invocation.
This applies even when the caller intends to place it in an inactive branch.
Do not silently drop relationships, assert their current truth, or turn them
into requirements.

### Uses in conditional branches

Classify eligibility transitively before allocating anything. A component is
relationship-only if its entire expansion contains only ordinary relationships.
Nested uses of relationship-only components qualify. Private `cell`, `source`,
or `out` declarations and any internal conditional make it ineligible.
Source specialization does not change this structural classification.

An eligible use in a branch contributes all its resulting relationship IDs to
that branch's existing activation group, in expansion order. No new activation
mechanism, conditional cells, or nested conditional groups are introduced.
An ineligible use produces a diagnostic identifying the use and the declaration
or nested use that violates the restriction.

## Library construction and ownership

The reusable abstraction belongs at the library boundary, not solely in parser
AST nodes. Keep native graph composition independent of CEL and filesystem
loading. The language layer supplies adapters for complete language types,
expression execution, names, and source provenance.

The following are responsibilities, not final public type names:

- A template builder owns typed formal cell slots, private declaration recipes,
  ordered relationships/methods, conditional recipes, and nested uses.
- Template-local cell/relationship handles are scoped to that owner. Reject
  foreign handles and references outside the component's permitted namespace.
- Finalization produces an immutable, reusable definition with a fixed signature
  and validated local structure.
- Instantiation maps formal slots to checked destination cells and private slots
  to fresh allocations. The destination sheet owns all resulting live graph
  relationships and cells.
- An instantiation result records the opaque instance identity and created graph
  handles for metadata/inspection. This is not a runtime add/remove lifecycle API.
- Dropping the template/module after successful construction must not invalidate
  a live sheet. Instantiated executable bodies retain what they need.
- Private mutable execution state must not be shared between instances. Sharing
  immutable definitions or pure native callables is permitted.

Native builders must support ordinary methods, initial values, filters,
requirements, conditionals, and nested typed template uses without source text.
Rust generics may build concretely typed templates without language generics.
Compound language types must retain their complete shapes through the adapter;
the runtime's erased storage carrier alone is insufficient for binding checks.

### Executable recipes

Existing live `Method` values and CEL `DynSegment` values are not template
definitions to clone. The former embeds destination cell IDs; the latter owns
execution resources. Retain repeatable body-construction recipes and create
fresh execution frames when instantiating.

For native methods, a typed body factory can capture immutable configuration and
construct the callable with mapped cells. For parsed methods, the language
frontend retains the typed parameter/local bindings and expression construction
information needed to emit a fresh frame. Instantiation must not substitute
strings or require the caller to invoke a parser.

Builder discretion: choose the smallest reusable executable representation that
fits existing CEL ownership. Re-emitting/compiling retained expression structure
at construction time is acceptable; do not add `Clone` to the entire runtime or
share mutable `RefCell<Frame>` state between instances just to obtain reuse.
Repeated propagation must not recompile definitions.

### Checking and errors

Separate definition checks from destination binding checks:

1. Parse/build and resolve modules, signatures, and definition-local names.
2. Validate concrete types, all method bodies, local structure, and use cycles.
3. Check invocation completeness, exact types, distinct actual cells, supported
   cell kinds, and conditional eligibility; compute source specialization.
4. Instantiate in written order; then run existing whole-sheet validation on the
   combined graph, including guard independence and planner structural checks.

Do not require each template to be independently solvable before it has caller
bindings. Destination constraints and source specialization can change validity.
Existing runtime rules still decide whether the combined graph is valid.

Public fallible APIs report invalid handles, malformed definitions, unsupported
bindings, and construction failures. Untrusted source is not a precondition.
Documented interface obligations about value domains are preconditions, not
automatically added filters or requirements.

Perform structural/binding preflight before changing the destination. This does
not require expensive graph rollback: if later construction fails, the
destination must remain safely discardable; do not promise it is unchanged or
reusable. A convenience module-to-live-sheet operation returns no live sheet on
failure. Do not return a partial graph as successful composition.

## Provenance and tooling

Retain source identity as well as spans: definitions may be in another file.
An instantiation diagnostic labels both the use/binding and relevant formal or
definition item. Runtime failures identify the instance path and definition
location, with the invocation as context. Programmatic templates use supplied
labels/metadata without inventing source spans.

Preserve private scopes in metadata without flattening private names into public
caller lookup maps. Host root-cell access remains unambiguous. Inspection can
use instance metadata; no new UI controls are required by this feature.

Update both language parser paths, the AST/CST, comment-preserving formatter,
type checker, LSP and VS Code grammar, host loading/selection APIs, books, and
examples. Preserve ordinary single-root source behavior. Libraries must expose
modules/templates without depending on a Begin-only registry.

## Contract-derived verification

Test equivalent manually inlined and composed graphs through observable values,
errors, directions/source status where public, and source aspirations:

- Repeated instances with disjoint private state; different instances sharing
  boundary cells; deterministic initialization and strength order.
- Named argument reordering; missing, extra, repeated, and unknown names;
  repeated actual cells; exact scalar/registered/compound type mismatches.
- Source-bound single/multi-output method elimination, no-resulting-direction
  errors, and nested source bindings.
- Rejection of all out-cell arguments, including read-only ones.
- Internal outputs, filters, requirements, and conditionals outside branch uses.
- Valid relationship-only branch uses, nested eligible uses, branch switching,
  and rejection of transitive private state/conditionals.
- Inline/imported/native definitions, aliases, forward definitions, root
  selection, missing/ambiguous symbols, recursive-use diagnostics, and definition
  environment isolation.
- Independent instance execution and lifetime after dropping module owners.
- Definition/use-site diagnostics across files and formatter comment round trips.
- Native construction and use without parsing any language text.

Demonstrate inequality, two-way and three-way multiplication, extracted
resolution/aspect-ratio relationships, and an authored resize/crop-straighten
composition. Use private outputs for diagnostics if helpful, but do not smuggle
out-cell bindings into the examples. Caller boundary requirements remain explicit.

## Builder-made calls and deferred work

The proposed builder-made calls are zero-parameter components, opaque instance
identities, explicit imports with aliases, exported file-level definitions,
transitive relationship-only eligibility, explicit multi-root selection, and
construction failure guarantees without rollback. These need user approval with
the rest of this design.

Deferred issues: #261 (out arguments), #262 (language generics), #263 (equations),
and #264 (general conditional component uses). Filesystem/package conventions,
runtime instance removal, implicit capture, and symbolic direction inference
are not part of #29.
