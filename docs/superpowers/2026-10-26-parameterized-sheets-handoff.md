# v1.0 Group 5: Parameterized sheets implementation handoff

Status as of 2026-10-26: the approved #29 implementation is present in the
`toward-v1-1` linked worktree. Tasks 1–9 received independent task-scoped reviews
and approved corrections. Task 10 audited the public contracts, added missing
boundary coverage, corrected source-only Unicode diagnostic columns, and ran
the complete local gate. The independent whole-branch review then found one
Important and two actionable Minor findings. The consolidated local fix wave
addresses all three. The scoped re-review approved those fixes with no new
breakage or open findings on 2026-10-27, at
`2b817a95f94cb59430adc13558b759cd0ee8d0c0`.
No push, pull request, merge or release is claimed.

The approved semantic authority is
[`specs/2026-10-25-parameterized-sheets-design.md`](specs/2026-10-25-parameterized-sheets-design.md).
The original implementation plan remains unchanged by Task 10.
The branch merge base is `844ef48e32126a1db3198524a72ca2b04a60e800`;
Task 10 began at `1409d0d3b10e02f515fe04a671626ecd5f6a6fea`.
The final fix wave began at `f67a79a0579ab6947e2f02c9b204fdff5d6a5a14`.

## PR-review-fix addendum — 2026-10-27

The local-only status above records the prior milestone. PR #265 now exists,
and its published review snapshot is
`736b1a0a4ceb1d7fd00370b82d8c1ed0050bad00`. A consolidated local fix wave
addresses all four challenge-validated findings from Talos review 5675830937:

- The extension canonicalizes both the workspace root and each mapped target,
  rejects file/directory link escapes, and reads the approved canonical path.
  Lexical URIs still identify editor overlays. Tests cover internal links,
  symlinked workspace roots, explicit failure snapshots, and recovery.
- The formatter scans optional trailing commas separately from required list
  closers. Declaration and invocation tests assert block/line comments survive
  on both sides of `)`, including the LSP whole-document replacement.
- Nonexecuting module checking validates output backing-cell defaults even
  when a writer exists. Counted host-type tests verify recursive shape rules,
  unused definitions, compiler agreement, and no default/writer execution.
- Native source preflight iteratively memoizes successful recipe/complete-mask
  specializations. It preserves actual failing ancestry, distinct masks,
  inactive-branch checks, and occurrence-specific live expansion. A deterministic
  counter catches repeated work; a public check/compile test handles a compact
  doubled empty DAG with 60 inclusion levels on a 64-bit host. Distinct complete
  masks can still grow exponentially; the API documents that cost.

Each reproduction failed through a real behavioral assertion before its fix
and passed after recompilation. The sole final-product full gate ran from
`2026-10-27T23:36:49Z` through `23:38:37Z`; all 15 selectors exited **0**:
workspace formatting/build/test/doctest, all five all-target Clippy variants,
strict library docs, extension compile/tests, feature-enabled Euclid, and final
formatting/whitespace checks. Additional no-default-feature Begin/ez-adam tests
and Begin's npm tests all exited **0** by `23:38:48Z`.

- Workspace: **2,974 passed, 0 failed, 4 existing ignored**, across 106 result
  groups: 2,590 non-documentation tests and 384 doctests. The explicit doctest
  command repeats those 384 passing and four ignored cases.
- Extension: **21 passed**. Feature-enabled Euclid: **81 passed**, including
  doctests; that selector overlaps workspace coverage.
- Additional selectors: Begin without default features **20 passed**; ez-adam
  without default features **103 passed**; Begin npm **65 passed** in two files.
  Feature variants and focused regressions overlap the workspace total.
- No project compiler warnings appeared. The existing third-party
  `block v0.1.6` future-incompatibility notice remains separately disclosed.

Raw logs and `talos-265-fix-gate-status.tsv` live under
`.superpowers/sdd/2026-10-25-parameterized-sheets/`. The controller's session
artifact `files/pr-265-fix-report.md` records roots/fixes, exact RED/GREEN
commands and outputs, changed files, validation counts, and commit identity.
This wave changes no production renderer and makes no live VS Code claim.
Distribution checks still require CI's mdbook/wasm-pack tooling.

The implementation self-review is scoped to these four fixes, not an independent
re-review. Publication of this wave, re-review, and review-thread resolution
remain pending for the controller. No push or GitHub write occurred in this
fix wave. Pre-existing Scout memory and instruction-file changes remain
untouched and unstaged. The approved semantics and all deferred boundaries below
remain unchanged.

## Broad review and consolidated fixes

The broad review covered all 88 branch paths and confirmed a post-construction
conditional failure that retained the CEL span but lost its executing occurrence.
Both native live and staged match evaluation now attach the actual
`ErrorSite::Conditional` identity without changing the original error cause.
Native occurrence records retain conditional IDs in authored order. The language
adapter owns those mappings alongside definition sources and parent links, so
the locator selects the exact expression and only its actual inclusion chain.
Neither adapter guesses from input cells or coincident spans.

Public regressions first failed with empty conditional labels/sites. They now
exercise successful propagation, owner drops and a later zero-divisor edit:
repeated nested imported instances, identical definition text/spans in separate
modules, the second authored conditional, the public host adapter, source-free
native execution and the shared renderer. Existing construction/key and method
error contracts remain covered.

The namespace fix consistently rejects reserved `sheet`, `use` and `as` at
module-path/native-export registration, sheet declarations, aliases and
references. The existing ordinary-identifier restrictions also apply at source
boundaries; Unicode names remain valid and file-level recovery retains following
definitions. The milestone summary now distinguishes local implementation and
review fixes from pending scoped re-review/publication.

The complete fix-wave report and distinct raw logs live under
`.superpowers/sdd/2026-10-25-parameterized-sheets/final-fix-report.md`.
They record RED/GREEN evidence, exact final-gate statuses, changed paths and
commit identities. The controller must inspect that report before scoped
re-review. This implementation self-review is not an independent re-review.
All seven rulings and all deferred boundaries below remain unchanged.

## What's done

### Native construction and one graph

`adam_rs::{TemplateBuilder, SheetTemplate, MethodRecipe}` construct reusable
definitions without CEL, parsing or filesystem access. Owner-scoped slots reject
foreign handles. Named bindings check completeness, distinct actual cells, exact
types and supported cell kinds before destination mutation. Each occurrence
allocates fresh private cells and preserves written depth-first order.
Instantiation does not propagate. The destination owns the live cells and
relationships after template owners disappear.

Source bindings discard an entire authored method if any output is a source.
They never project a tuple. The rule applies through nested parameters and
private sources. All authored method signatures/bodies check before pruning.
An empty constituent relationship is an error, including in an inactive branch.
Relationships retain separate alternative-method sets in one caller graph.

Branch eligibility is structural and transitive: only relationships and nested
eligible uses qualify. Private cells, sources, outputs and internal conditionals
disqualify a component before allocation. Eligible groups contribute concrete
relationship IDs to the existing branch activation mechanism.

Compact definition DAGs retain inclusion groups rather than eagerly expanding
descendants. Iterative traversal checks expanded counts and fallible reservation;
overflow/allocation failures return diagnostics. Iterative last-owner release
handles deep native recipe and language metadata chains while preserving shared
child owners. A finite acyclic DAG can still have an unaffordable expansion;
the API reports that failure rather than imposing a fixed nesting cap.

### Language and ownership

`adam_lang::{ModuleSet, ModulePath, check_module, ModuleCompiler, CompiledModules,
CompositionBuilder, TypedTemplate}` expose the same functionality to library
clients. Clients register sources/native exports under explicit module paths.
Definitions resolve in their defining module, with aliases, forward references,
import-only cycles and explicit root selection. Actual use cycles, duplicate
exports/imports, missing names and unsupported bindings are diagnostics.
Neither core layer discovers or loads files.

Both parser paths, AST/CST and comment-preserving formatter retain the module
syntax and complete accepted concrete `TypeExpr`. Boundary checks compare full
`TypeShape`, including registered leaves, recursive arrays/tuples and callable
signatures, not erased storage carriers. `AdamParser::parse_str` retains the
single-root convenience without arbitrarily selecting among multiple roots.

`check_module` borrows the registered collection and selects a `ModulePath`.
It checks all definitions, bodies, branches and source masks without constructing
defaults, running host callbacks, evaluating expressions or demanding isolated
component solvability. `ModuleCompiler` shares one immutable operation/type
environment with owning recipes. Retained expression trees emit fresh frames
at construction; propagation never recompiles. Independent instances have
distinct execution resources and survive module/compiler/template drop.

Caller cells retain identity, kinds, values, priorities, filters and requirements;
binding does not write. Root lookup maps exclude private names. Source aspirations
retain the existing runtime semantics: edited/fixed sources, filtered aspirations
and shadowed self-references have their ordinary guarantees. An ordinary
unshadowed derived cell may update its raw `source()` slot. The feature does
**not** promise to preserve every ordinary cell's initializer forever.

Structural/binding failures precede mutation. Later factory/construction failures
leave the destination safely discardable, not necessarily unchanged or reusable.
Root construction returns no partial successful live sheet. Existing whole-sheet
guard/structural validation runs on the combined graph, without propagation.

### Provenance, clients and examples

`SourceMap`, owning `SourceId`, `SourceLocation` and optional-location
`SourceLabel` retain real file text after collection drop. Occurrence paths,
inclusion sites and authored method indices distinguish repeated instances and
source-pruned methods. Runtime, initializer, requirement, output-writer,
conditional and preflight errors identify the actual failing definition/use
chain, preserve native causes and do not invent spans for native definitions.
The shared web renderer selects text by source identity, not caller text.

The LSP accepts explicit `{path, uri, text}` snapshots with open-document overlays.
Invalid/incomplete refreshes retain registered ownership and surface failure;
they never silently fall back to source-only success. Only a valid explicit
replacement, including an explicit empty array, removes owners. Independent
dependency groups preserve unrelated diagnostics when parsing/resolution/signature
checking fails elsewhere; affected groups visibly report incomplete checking.
Configuration refresh, close, recovery and dependent republication have transport
tests. The extension serializes refreshes, reports loader/publication failures,
uses exact mapped paths, and suppresses disposed/unrelated work.

Task 10 found that source-only diagnostics still treated scalar columns as
UTF-16 offsets. They now use the same text-based conversion as module diagnostics.
Tests parse real non-BMP comments and accented identifiers, retain both source
owners after collection drop, slice exact original tokens, and verify primary
and related UTF-16 ranges through the actual LSP transport. Source-only type and
lexical failures also check exact offsets; lexical errors retain their existing
point spans.

Executable native/inline/imported examples demonstrate ordered bounds, repeated
three-way multiplication, source-specialized two-way multiplication, caller-owned
resolution/aspect constraints and resize/crop/straighten. They use ordinary
boundary cells, explicit module/root setup and caller-owned policy. Geometry
tests independently verify maximal support fit, inverse-mapped/presented corners,
coverage, precision, resize/rotation/reversal, checked casts/inversion and source
aspirations. Physical aspect does not promise exact ratios of independently
rounded pixel counts; finite/nonzero inverse preconditions remain explicit.

## Public verification map

These are executable contract tests, not an assertion that test success proves
every possible input. The complete requirement-by-requirement ledger and exact
raw-log references are in the local Task 10 report.

| Contract | Public evidence |
|---|---|
| Native owner/binding validation and private/caller policies | `adam-rs/tests/templates.rs`: `foreign_slots_are_rejected`, `rejected_bindings_leave_destination_and_factories_untouched`, `private_outputs_filters_requirements_work`, `caller_filters_and_requirements_remain_effective`, `bidirectional_use_preserves_caller_priorities`. |
| Whole-method source pruning, including invalid/inactive directions | Same file: `source_binding_removes_whole_multi_output_method`, `private_sources_specialize_included_methods`, `pruned_method_factories_still_validate_every_edge_and_type`, `inactive_branch_empty_relationship_is_preflight_error`; `adam-lang/tests/module_analysis.rs::module_analysis_checks_unused_modules_and_pruned_direction_types`. |
| Transitive conditional eligibility and existing activation | `templates.rs`: `conditional_wrapper_rejects_transitive_state`, `conditional_without_private_state_is_still_ineligible_for_branch_inclusion`, `conditional_groups_match_manual_inline_after_edits`; `adam-lang/tests/composition.rs::conditional_uses_only_relationships` includes inactive private cells/sources/outputs/conditionals. |
| Compact expansion, checked failure and iterative destruction | `templates.rs`: `deep_inclusions_expand_and_drop_without_host_recursion`, `shared_child_factories_live_until_the_last_template_owner_drops`, `repeated_child_edges_preserve_external_template_ownership`, `expansion_size_overflow_is_diagnostic`, `expansion_reservation_failure_is_diagnostic`, `inclusion_groups_keep_repeated_child_dags_compact`; `composition.rs::deep_last_owner_metadata_teardown_is_iterative`; `module_resolution.rs::deep_compact_dependency_graphs_are_checked_iteratively`. |
| Shapes and nonexecuting checking | `adam-lang/tests/module_analysis.rs`: `module_analysis_never_executes_host_code`, `module_analysis_checks_named_bindings_and_full_shapes`, `module_analysis_does_not_run_an_isolated_planner`; `composition.rs`: `full_shapes_reject_same_carrier_arrays_and_tuples`, `nested_bindings_compare_callable_signatures_and_reject_foreign_slots`, `registered_host_leaf_compounds_keep_canonical_capabilities`. |
| Definition isolation, immutable environment and fresh frames | `adam-lang/tests/module_compile.rs`: `compiled_wrapper_keeps_defining_module_and_source_owners`, `compiled_factories_do_not_execute_and_preserve_host_shapes`, `compiled_methods_keep_explicit_tuple_patterns_and_recursive_values`, `compiled_instances_own_distinct_execution_resources`. |
| Inlining, source aspirations and no propagation during construction | `composition.rs::composed_matches_manual_inline`, `roots_validate_the_combined_guard_graph`; `module_compile.rs::compiled_binding_does_not_write_or_replace_caller_policies`; `templates.rs::method_factories_run_once_per_instance_not_during_propagation`. |
| Actual failure paths and typed/native causes | `composition.rs`: `composition_diagnostic_selects_definition_and_repeated_instance`, `composition_diagnostic_construction_selects_actual_use`, `composition_diagnostic_preflight_selects_actual_use`, `composition_diagnostic_requirement_construction_keeps_definition_span`, `composition_diagnostic_output_writer_keeps_definition_span`, `composition_diagnostic_imported_native_selects_actual_use`; `module_compile.rs::compiled_deferred_key_failure_retains_child_conditional_and_actual_use`. |
| Syntax, recovery, formatting and roots | `adam-lang/tests/modules.rs`, `module_formatting.rs`; `composition.rs::root_selection_and_parser_share_common_lowering`; `adam-fmt/tests/{cli,layout}.rs`; `adam-lsp::dispatch::tests::format_edits_preserve_complete_modules_and_refuse_recovered_items`. |
| Editor snapshots, isolated errors and Unicode transport | `adam-lsp/tests/modules.rs`: `transport_malformed_definition_preserves_independent_errors_and_recovers`, `transport_incomplete_refresh_retains_owners_until_explicit_removal`, `transport_close_failures_recovery_and_removal_are_consistent`, `transport_unicode_binding_locations_match_original_tokens`, `transport_unregistered_unicode_source_uses_utf16`; `editors/vscode-adam-lang/test/moduleSources.test.ts`. |
| Numeric examples and executable guidance | `cel-euclid/tests/composition.rs` (11 tests), existing `adam_examples.rs` (18 tests), `adam-lang-book/tests/relationships.rs`, `adam-lang/examples/templates.rs` and public Adam/Euclid doctests. |

## Controller rulings retained

1. Move the mechanical `ez-adam` `ast::Sheet` constructor-field update into
   Task 3, where the AST producer changes. Dependent constructors must remain
   consistent immediately. If wrong, a small constructor-only change arrives
   early, with no intended behavior change.
2. Let an inclusion relationship slot denote an ordered included relationship
   group, resolving concrete IDs only during live expansion. Eagerly returning
   transitive IDs violates compact-DAG/checked-size requirements. If wrong,
   native handle documentation/API needs refinement; live relationships remain
   separate and unchanged.
3. Release library-owned inclusion edges iteratively while respecting shared
   `Rc` owners, rather than imposing a nesting cap. Explicit traversal alone
   does not prevent recursive destruction. If wrong, destructor complexity
   requires shared-child and last-owner regressions; those tests are present.
4. Preserve full accepted concrete `TypeExpr` and comments through formatting
   instead of rejecting an accepted structural form. Generic definitions/type
   parameters remain deferred. If wrong, structural syntax can represent types
   the semantic registry rejects, as it already does for unknown names.
5. Permit absent native source locations in `SourceLabel` and instance-origin
   definitions. Mandatory locations would invent spans. If wrong, consumers
   need optional-location checks; genuine parsed locations remain unchanged.
6. Execute Task 5 through serial 5A source/native metadata, 5B complete-shape
   source-free adapters and 5C resolution/body-lowering gates. The combined task
   exceeded one bounded worker context and mixed prerequisite API design with
   semantic integration. If wrong, two extra review gates cost time but do not
   change scope.
7. Select a registered `ModulePath` in `check_module`, not an unassociated AST.
   The collection already owns AST/source identity and must identify the
   selected file. If wrong, new callers register/select their source rather
   than pass an AST; no existing implemented client API changes.

## Verification

### Consolidated final fix wave

The final exact gate ran from `2026-10-27T05:29:02Z` through `05:30:24Z`
(October 26 locally). All commands listed below exited **0**, including both
desktop/default-feature checks for Begin and ez-adam. Distinct raw logs use
`final-fix-final-*.log`; `final-fix-final-gate-status.tsv` retains all 15 statuses
beside the fix report.

- Workspace: **2,968 passed, 0 failed, 4 existing ignored**, across 106 result
  groups: 2,585 non-documentation tests and 383 doctests. The separate explicit
  doctest command repeats those 383 passing and four ignored cases; do not add
  them again.
- Extension: **19 passed**, no failures, skips or cancellations.
- Feature-enabled Euclid: **81 passed**, including its 13 doctests; this
  feature selector overlaps workspace coverage and is reported separately.
- Focused language, native template, renderer and cached diagnostic selectors
  passed 77, 52, 25 and one test respectively; these overlap the final workspace
  total.
- Plain workspace build/test emitted no project compiler warnings. The existing
  third-party `block v0.1.6` future-incompatibility notice remains.

The first gate attempt exposed a missing exhaustive conditional arm in the
test-only cached-plan diagnostic comparator. The fix adds fixture-ordinal
normalization and its regression. Its failed logs/statuses remain under
`final-fix-*`; both subsequent complete gates passed and have distinct prefixes.
No prior Task 10 or review evidence was overwritten.

Only a renderer contract test changes `adam-web-ui`; renderer/UI production
code is unchanged. The new post-edit conditional rendering test checks the
actual shared renderer, imported source/caret and only the failing use.
Task 7's screenshots below remain reused evidence, not a new browser run.

### Historical Task 10 gate

The Task 10 report records commands, terminal exit markers, output summaries,
the complete coverage ledger and the task/branch manifests:
`.superpowers/sdd/2026-10-25-parameterized-sheets/task-10-report.md`.
Raw logs and `task-10-gate-status.tsv` live beside it. These ignored local
artifacts are not published repository files.

Every command below exited **0** on the Task 10 source state. The Task 10 gate
ran from `2026-10-27T04:53:16Z` through `04:53:54Z` (October 26 locally).

```sh
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
cargo clippy -p ez-adam --no-default-features --all-targets -- -D warnings
cargo clippy -p ez-adam --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace
(cd editors/vscode-adam-lang && npm run compile)
(cd editors/vscode-adam-lang && npm test)
cargo test -p cel-euclid --features adam
cargo fmt --all -- --check
git diff --check
```

Observed results:

- Workspace tests/doctests: **2,959 passed, 0 failed, 4 ignored**, across
  106 result groups. The separate explicit workspace doctest command repeats
  382 passed and the same four ignored tests, not additional distinct tests.
- Feature-enabled Euclid: **81 passed, 0 failed, 0 ignored**, including all
  11 new composition contracts, 18 existing Adam example contracts and
  13 feature-enabled doctests.
- Extension: TypeScript compilation succeeded; **19 tests passed**, with no
  failures, skips, cancellations or todos.
- Focused `cargo test -p adam-lsp`: 28 unit, 8 transport/integration and
  4 documentation tests passed. The source-only Unicode regression first
  failed at UTF-16 column 37 versus expected 38.
- Extra `cargo test -p adam-lang --example templates` passed both tests;
  `cargo run -p adam-lang --example templates` printed
  `native: 7 * 3 = 21` and the successful parsed example summary.
- All five all-target Clippy selectors, strict workspace library documentation,
  formatting and whitespace checks succeeded.

Build/test output contains **no project compiler warnings**. Cargo still emits
the previously reported third-party `block v0.1.6` future-incompatibility notice.
No dependency was upgraded or warning suppressed. Existing npm notices from
earlier work remain unassessed; these compile/test commands are not an npm
dependency audit. The four ignored doctests are existing wasm-bindgen exports
in `adam-lang-book-live` and two `ez-adam` side-panel helpers; Task 10 introduces
no ignore or disabled selector.

## What's deliberately excluded

- #261: all out-cell actuals, even read-only ones.
- #262: language generics; native Rust generics can still build concrete recipes.
- #263: inline equations and inferred directions.
- #264: general conditional component uses with private state/conditionals.
- Filesystem/package discovery, implicit captures, instance removal and nested
  solvers are outside #29. No compatibility layer or new solver was added.

Task 7's existing rendered Begin evidence remains applicable: its runtime and
construction screenshots, DOM/Spectrum checks and console diagnostic assertions
are recorded in `task-7-report.md`, `task-7-ui-runtime.png`,
`task-7-ui-construction.png` and `task-7-ui-verification.log` beside the Task 10
report. Task 10 changes only LSP conversion, contract tests and completion prose;
it does not alter that renderer or UI. This is reused evidence, not a new
browser run. No interactive VS Code smoke test, sanitizer run, packaged editor
installation or new mdBook distribution build is claimed.

## What's left

The independent whole-branch review and scoped fix re-review are complete.
All three findings are addressed; no findings remain open. The final product
commits are `b82d55c` (review fixes) and `2b817a9` (durable Scout knowledge).
The full gate recorded above covers that product state. The final status
updates change only prose.

#29 remains open in v1.0 pending publication. Publishing the branch/PR, merging
and release require separate authorization; no automatic closure or merged
status follows from this handoff. Existing #261–#264 exclusions are unchanged.
Retain the ignored local review reports and raw logs until publication so their
evidence links remain usable. The scoped report is
`.superpowers/sdd/2026-10-25-parameterized-sheets/final-fix-review.md`.
