# Composable CEL Containers — Implementation Handoff

Status snapshot: 2026-10-19. All nine tasks are implemented and reviewed.
The whole-branch review's two findings are fixed and its scoped re-review
approves both corrections. The complete required local gate passes again after
those fixes. No push or PR was requested.
The final two-finding fixwave below passes affected checks; scoped re-review
remains pending until the controller supplies its verdict and fresh full gate.

Approved contract:
[design](specs/2026-10-19-cel-composable-containers-design.md) and
[plan](plans/2026-10-19-cel-composable-containers.md).
Task 9 starts at `0e11c409010af69eb22cdc00881e4a7f9ba2a335`.
Earlier specs retain their original proposals with dated status/link notes.
Use the current books and library tutorials for final API spellings.

## What's done

| Task | Delivered |
| --- | --- |
| 1 | `Array<T>`: concrete contiguous storage, fixed pointer/length/capacity header, allocation-preserving vector transfers, borrowed slices, ownership/alignment/ZST coverage. |
| 2 | Source-ordered canonical lists, checked tail-first `repr(C)` layout, moving native tuple bridges with no implicit clone/equality/debug bounds. |
| 3 | Complete recursive `ValueType`, sealed `ValueRepr` witnesses, and whole-value `OwnedValue`. |
| 4 (4A–4D) | Descriptor factories, frozen layout/move/drop plans, safe shared-body freeze leases, canonical host operations, one atomic runtime/caller ABI switch, and prepared typed/raw execution. Borrowed scalar reads clone only when executed. |
| 5 | `Callable<Args, R>` / `FnOf`, native/prepared/common carrier dispatch, escaped-callable freeze retention, and explicit reentrancy errors. |
| 6 | Structural `Fn` syntax, singleton tuples, complete `ResolvedType` / checker `Ty`, structural overloads/record fields, and foreign Euclid vector/slice/list interop. Noncallable lexical bindings shadow global calls correctly. |
| 7 | Private persistent `CellValue`, typed compound read/write/source access, recursive defaults, explicit capability diagnostics, and prepared Adam relationships/filters/requirements/outputs. Preflight precedes default effects; failing multioutput/filter paths preserve ownership and aspirations. |
| 8 | Recursive Inspector formatting, preserved scalar editing, explicit nonmutating unsupported compound edits, and retirement of legacy storage/names from production. Registered leaf aliases unify by canonical identity rather than names/capability sets. |
| 9 | Executable CEL/Adam book contracts, current guidance/tutorials, historical status notes, this handoff, complete workspace gates, and configured-nightly AddressSanitizer verification. |

### Final APIs and boundaries

- CEL `[T]` is `Array<R(T)>`, with actual concrete elements. `from_vec`,
  `into_vec`, and `as_slice` do not perform type checks or per-element conversion.
  `[[T]]` is `Array<Array<T>>`: the O(1) vector view is `Vec<Array<T>>`,
  and the slice view is `&[Array<T>]`. Explicit deep `Vec<Vec<T>>` conversion
  creates a new outer vector and transfers the inner allocations separately.
- Tuple values are `CStackList<Head, Tail>` ending in `CNil<()>`.
  Witnesses are `Cons<HeadWitness, TailWitness>` and `Nil`. Logical order is
  source order; physical offsets account for tail-first fields and padding.
  `IntoCStackList` / `IntoRustTuple` move native tuples without extra traits.
  Source unit normalizes to `Nil`; explicitly registered opaque host `Leaf<()>`
  is not the same CEL identity.
- Complete descriptors distinguish nested identity, including empty arrays and
  callable parameter/results. No shared carrier ID or equal layout authorizes
  a structural cast. `Leaf`, `ArrayOf`, `Cons`, `Nil`, and `FnOf` witness concrete
  representations without downstream orphan implementations.
- `Fn((A,), R)` has one parameter; `Fn((), R)` has none.
  `Callable::from_fn` and `Callable::from_prepared` expose the same typed
  `call(&args)` interface. Storage does not require function equality.
- Arbitrary erased external imports and `OwnedValue::try_into_typed` validate
  complete shapes. `DynSegment::prepare` produces `PreparedSegment` after
  validating its typed frame. `prepared_segment::PreparedRawSegment::new`
  binds runtime descriptors; its unsafe `call` requires exact live/aligned
  canonical pointers. Both execute the existing raw engine without repeated
  shape comparisons, `Any` downcasts, or signature checks. Value-dependent
  bounds/allocation checks, errors, reentrancy protection, and cleanup remain.
- `ResolvedLeafType` owns a complete `ValueType`. `ResolvedArrayType` accepts a
  complete element descriptor. `ResolvedType` and `Ty` retain recursive shapes.
  There is no narrower production array-element type model.
- Clone, equality, and debug are independent capabilities. Adam requires clone
  and equality for persistence; it diagnoses missing capabilities even for a
  typed empty callable array. Arrays default to empty without element defaults;
  tuples default recursively. `read_compound` returns an independent owner,
  `write_compound` checks before mutation, and `source_compound` preserves the
  original aspiration independently of a filtered effective value.
- `OwnedValue::prepare_debug` and `prepare_compound_cell_formatter` retain the
  canonical formatter without cloning or consuming values. Formatting at the
  external cell boundary still checks the current owner's complete shape.
- The sole checked host bridge moves whole owners and registered concrete scalar
  boxes; it is not a second container ABI, per-element boxing path, or opaque
  `OwnedValue` element representation. Task 8 removed all legacy production
  names/storage. Its retained audit records zero Rust hits.

## Task 9 executable contracts and RED → GREEN

`cel-lang-book/tests/composable_containers.rs` supplies five anchored examples:
arrays of tuple lists, typed empty/populated callable arrays with native and
interpreted invocation, nested allocation-preserving vector views with explicit
deep conversion, one shared typed Rust operation, and source-unit normalization.
`adam-lang-book/tests/composable_containers.rs` supplies two: recursive defaults
and typed read/write/source/output propagation with failed-write nonmutation,
plus explicit callable equality rejection. Assertions check semantic values,
ownership/allocation promises, and errors, not exact source spelling.

The first command ran before documentation or registration-glue fixes:

```sh
cargo test -p cel-lang-book -p adam-lang-book --test composable_containers
```

**RED:** six examples passed; `shared_native_and_interpreted_operation` failed
with `undefined identifier: reverse_rows`. `register_operation` supplied a
signature but did not bind the ordinary-call name. Adding the example's explicit
`register_function` marker fixed that intended glue gap; the identical command
then passed **7/7**. No production behavior change was necessary. Other examples
already exercise completed Tasks 1–8 and were not artificially made to fail.
Final standalone-import examples also pass **7/7**.

## Required gate: exact commands and results

All commands ran from this worktree root on the existing stable toolchain.
All eleven exit **0**, with zero project compiler, Clippy, or rustdoc warnings.
Cargo prints the pre-existing third-party **`block v0.1.6` future-incompatibility
advisory** in desktop/workspace commands. That is not a project warning and was
not introduced or suppressed here.

Evidence directory (archived outside the repository):
`/Users/sean-parent/.copilot/session-state/3f9dbc0b-1ade-468d-80e2-b5a8009e0d85/files/cel-composable-containers-evidence/`.
Each filename below is relative to that directory.

| Exact command | Result | Log |
| --- | --- | --- |
| `cargo fmt --all --check` | Exit 0 | `task-9-fmt.log` |
| `cargo build --workspace` | Exit 0, no project warnings | `task-9-build.log` |
| `cargo test --workspace` | 2562 passed, 0 failed, 4 ignored | `task-9-tests.log` |
| `cargo test --doc --workspace` | 293 passed, 0 failed, 4 ignored | `task-9-doctests.log` |
| `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` | Exit 0 | `task-9-clippy-workspace.log` |
| `cargo clippy -p begin --no-default-features --all-targets -- -D warnings` | Exit 0 | `task-9-clippy-begin-no-default.log` |
| `cargo clippy -p begin --all-targets -- -D warnings` | Exit 0 | `task-9-clippy-begin.log` |
| `cargo clippy -p ez-adam --no-default-features --all-targets -- -D warnings` | Exit 0 | `task-9-clippy-ez-adam-no-default.log` |
| `cargo clippy -p ez-adam --all-targets -- -D warnings` | Exit 0 | `task-9-clippy-ez-adam.log` |
| `RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace` | Exit 0, no rustdoc warnings | `task-9-docs.log` |
| `git diff --check` | Exit 0 | `task-9-diff.log` |

Workspace test totals include doctests; the explicit doc command repeats them,
so do not add the two totals. The four existing ignored doctests are two WASM
mount examples in `adam-lang-book-live` and two framework-coupled UI examples in
`ez-adam`. The existing untracked crop test participates in Cargo's discovery
but remains untouched and uncommitted.

### Existing sanitizer support

`rustup toolchain list`, `rustup component list --installed --toolchain nightly`,
and `rustup target list --installed --toolchain nightly` confirmed the installed
`nightly-aarch64-apple-darwin`, native target/std, and no rust-src.
`rustc +nightly --version` reports
`1.101.0-nightly (7df81381d 2026-10-14)`;
`rustc +nightly --print cfg -Zsanitizer=address --target aarch64-apple-darwin`
confirms AddressSanitizer. No tooling, components, targets, or dependencies were
installed. Prebuilt std suffices here; no `-Zbuild-std` was used.

```sh
RUSTFLAGS=-Zsanitizer=address CARGO_TARGET_DIR=target/task-9-asan \
  cargo +nightly test -p cel-runtime --target aarch64-apple-darwin \
  --lib --test array_value --test tuple_value --test prepared_containers \
  --test value_types --test callable_values --test runtime_abi
```

**206 passed, 0 failed**, including raw/list layout, allocation/drop/unwind,
overalignment, ZST, complete identity, callable, and prepared-execution contracts.
`task-9-asan.log` also records nightly Cargo's existing
`cargo::missing_lints_inheritance` manifest warning for `xtask/Cargo.toml`.
It is not a sanitizer finding or a changed manifest. The exact same installed
nightly compiler/AddressSanitizer tests were repeated through existing stable
Cargo, without changing that unrelated manifest:

```sh
RUSTC="$(rustup which --toolchain nightly rustc)" \
  RUSTFLAGS=-Zsanitizer=address CARGO_TARGET_DIR=target/task-9-asan-stable-cargo \
  cargo test -p cel-runtime --target aarch64-apple-darwin \
  --lib --test array_value --test tuple_value --test prepared_containers \
  --test value_types --test callable_values --test runtime_abi
```

**206 passed, 0 failed, no warnings or sanitizer findings**:
`task-9-asan-stable-cargo.log`. These runs instrument the runtime crate/tests,
not a rebuilt standard library. Thread/leak sanitizers and other platforms were
not run. No new runtime or unsafe code was added by Task 9.

## Retained Task 8 rendered verification

Task 9 changes no UI, dependency, or bundle, so it does not repeat rendering.
Task 8 served the real `begin` web build with `dx`, verified HTTP 200, and drove
system Edge on macOS through a dependency-free CDP client. The fixture displayed
`([(7, true), (8, false)], [])`, typed `[]`, and scalar `5` in the real Inspector
and graph. DOM evidence confirmed upgraded Spectrum controls and theme styles.
A compound edit showed invalid feedback without mutating the graph, blur
restored the formatted value, and editing the actual scalar shadow input to `9`
updated both Inspector and graph without changing compounds.

Retained artifacts: `task-8-report.md`, `task-8-inspector.png`,
`task-8-unsupported-edit.png`, `task-8-scalar-edit.png`,
`task-8-render-dom.json`, `task-8-render-edit.json`, and
`task-8-render-scalar.json` in the evidence directory. Task 8 stopped all
Edge/dx helpers. No new long-lived process is required by Task 9.

## Rulings carried forward (exhaustive ledger decisions)

1. **Split Task 4 into 4A factories/metadata, 4B plans, 4C checked boundaries,
   then atomic 4D.** This preserves one executable ABI while bounding coupled
   safety work. **Cost if wrong:** extra prerequisite commits and rework, not a
   relaxed contract.
2. **Legacy generic `Segment` conversion accepts opaque concrete leaves but
   rejects structural values without explicit witnesses; preparation is the
   aggregate bridge.** Marker IDs cannot establish recursive identity.
   **Cost if wrong:** a later ergonomic typed bridge, not an unsound cast.
3. **Freeze shared closure bodies with private trusted-binding leases.**
   Checked mutable dispatch rejects while leased and resumes after release;
   safe aliases cannot invalidate prepared ownership plans. **Cost if wrong:**
   later ergonomic redesign of mutable dispatcher access, not unchecked ABI use.
4. **Implement the remaining atomic cutover as runtime work plus immediate
   caller migration in one uncommitted integration change.** Repeated all-at-once
   workers exceeded bounded scope; the split still publishes one verified ABI.
   **Cost if wrong:** extra integration rework before that single verified commit.
5. **Do not fabricate production RED for already-implemented Task 1–8 paths.**
   Retain the real shared-operation name-registration RED and fix only example
   glue; the brief cannot imply that completed production paths must fail.
   **Cost if wrong:** weaker regression provenance, not runtime changes.
6. **Retain ignored evidence and leave mutation review to the controller.**
   The explicit Task 9 instructions prohibit agents/reviewers and require
   retained logs, overriding automatic workflow cleanup/review dispatch.
   **Cost if wrong:** a later review round before integration.

## Deliberate non-goals and what remains

- No native macro-compiled CEL backend, implicit deep vector ABI, function
  equality policy, capturing-closure feature, bracket indexing, or compound
  Inspector editor was added. These are boundaries, not silent partial support.
- Task 9 changes docs and contract examples only. No runtime/parser/Adam/UI
  behavior, dependency manifest/lockfile, bundle, or crop source was changed.
- No newly discovered implementation finding remains deferred; no new GitHub
  issue was necessary. The existing nightly Cargo manifest warning is reported
  above as a toolchain caveat, not concealed as a clean nightly-Cargo build.
- The whole-branch review assessed nested identity checks, canonical offsets,
  accidental capability bounds, and repeated prepared-invocation checks against
  the contract tests and source. It found the two checker issues below; the
  scoped re-review approves their corrections. No unresolved findings remain.
  No source mutations were executed. Comparison probes instrument `same_shape`;
  source inspection supplements them for direct identity checks or downcasts.
  No push, PR, or merge occurred.

## Final review fixwave — concrete callable and range identities

Base: `ce762425728618ffa0bebf2b57e2821dfed63791`.
Both Important findings are addressed together, with real RED–GREEN tests:

1. **Registered leaves are not automatically callable.** The earlier
   builtin-only shadowing guard left generic and foreign leaves eligible for a
   same-spelled global function signature. Persistent function registration now
   retains its concrete marker descriptor. The checker resolves known callee
   identities through the same selected-marker table as executable dispatch,
   independently of identifier spelling. Generic/foreign local shadows fail;
   genuine marker aliases, including aliases shadowing a different global
   function and marker values transported through tuples/branches, remain
   callable. Unknown metadata-free values retain best-effort handling.
2. **Range operators return ranges, not endpoints.** Matched builtin range
   operators obtain concrete descriptors from the existing builtin generic
   factories, including half-open, inclusive, one-sided and full ranges.
   Registered operator metadata keeps its precedence; arithmetic, comparisons
   and heterogeneous shifts retain their existing behavior. Tests cover all
   fourteen numeric endpoint types, typed callable arguments/results, nested
   arrays and annotations, negative mismatches, and executable agreement.

Accurate range inference exposed a tightly coupled Adam filter dependency on
the old shortcut. Filter checking now uses `TypeRegistry::range_entry` on the
inferred result, like executable filter dispatch: inclusive-range endpoint
identity must match the cell, and a range-producing expression need not use `_`.
Cell/source/out tests include callable-produced bounds and invalid endpoints.
This does not change executable evaluation, runtime ABI, or persistent storage.
The only registration storage change retains marker metadata in place of its
flat ID; executable marker selection is unchanged.

Affected validation:

```sh
cargo fmt --all --check
cargo test -p cel-parser -p cel-std -p cel-euclid -p adam-rs -p adam-lang \
  -p adam-lsp -p cel-lang-book -p adam-lang-book \
  -p adam-lang-book-live-config -p adam-lang-book-preprocessor
cargo clippy -p cel-parser -p cel-std -p cel-euclid -p adam-rs -p adam-lang \
  -p adam-lsp -p cel-lang-book -p adam-lang-book \
  -p adam-lang-book-live-config -p adam-lang-book-preprocessor \
  --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc -p cel-parser -p cel-std -p cel-euclid \
  -p adam-rs -p adam-lang -p adam-lsp -p cel-lang-book -p adam-lang-book \
  -p adam-lang-book-live-config -p adam-lang-book-preprocessor --no-deps --lib
git diff --check
```

All pass: **1842 tests including doctests, 0 failed, 0 ignored**, no compiler,
Clippy or rustdoc warnings. Detailed RED–GREEN commands/results and limitations
are retained in `final-fix-report.md` in the archived evidence directory above.
The crop test remains untouched with its previously recorded SHA-256.
No agents/reviewers, dependencies, UI edits, push, PR, or merge were used.
The scoped re-review approves both fixes and finds no new breakage. After that
review, the controller repeated all eleven required gate commands at
`346e6f671bdfabe8316e1e071a2db03b37fe6a35`: **2569 workspace tests passed,
0 failed, 4 ignored**, with **293 doctests passed, 0 failed, 4 ignored** in the
separate doc run. Plain build, all five Clippy configurations, formatting,
strict rustdoc, and diff checks pass. No project warnings remain; the existing
third-party `block v0.1.6` Cargo advisory remains unchanged.
Logs use the `final-gate-*.log` prefix in the archived evidence directory.
The earlier AddressSanitizer run remains applicable to the unchanged runtime;
the final fixes affect checker metadata and Adam filter checking only.
