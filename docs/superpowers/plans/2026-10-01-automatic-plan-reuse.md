# Automatic Plan Reuse Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `Sheet::propagate()` automatically reuse prepared assignments and structural seed work without changing values, diagnostics, change tracking, or failure boundaries.

**Architecture:** Keep separate main and unconditional prepared-plan caches. Released-source promotions preserve eligibility; other internal priority changes use a linear source-prefix certificate. Prepare selected graph metadata and structural seed recipes once, but evaluate seeds from current staged values and strengths.

**Tech Stack:** Rust 2024, existing `slotmap` and `anyhow`, standard collections and `Rc`; no new dependencies.

**Spec:** `docs\superpowers\specs\2026-09-27-adam-rs-planner-generalization-design.md`, especially rules 4 and 6, §1.2, and §3.1-3.7.

## Global Constraints

- Plan reuse is an internal optimization with no API change.
- All callbacks are purely functional. There is no guarantee that they execute, or when or how often they execute.
- A filtered released source remains released; a self-referencing claimant is not released.
- Each phase has its own released-source set and active subgraph.
- Cache hits perform no global strength sort or complete seed-elimination replay.
- Retain one prepared plan per phase, not a cache of branch combinations.
- Cross-propagation value memoization, dirty-cone execution, and incremental assignment repair are outside this implementation.
- Preserve existing seed-cycle, prerequisite-conflict, filter-diagnostic, and requirement-error behavior.
- Preserve the existing boundary where requirement errors occur after commit.
- Invalidate after actual structural mutation, including partial mutation before an error; rejected writes do not invalidate.
- Preserve pure-callback contract edits already present in this worktree.
- Every new type/function has an adjacent contract. Document nonconstant complexity and errors; assert checkable preconditions.
- Use checked signed arithmetic in fixtures and implementation.
- Use the existing worktree, Windows paths, and repository tools. Never commit directly to `main`.
- Format before commits and include `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`.
- Before a PR, run the complete workspace validation suite, including all five clippy commands.

---

## Starting State and Checkpoint

The current worktree is `worktree-planner-generalize-phase-c`; the initial specification
commit is `4e4bc25`. Subsequent approved purity-contract and specification refinements
are currently uncommitted. Inspect `git status` before execution; do not discard edits.
If the user has already committed this checkpoint with the plan, rerun the checks
below and verify the commit includes the listed paths, but skip the duplicate
checkpoint commit.

The current runtime still replans both phases. Existing pure-contract changes remove
invocation counters from public integration coverage and retain value/rollback assertions.
The adjusted tests and warning-denied library documentation already passed, but the
executor must obtain fresh evidence before committing.

- [ ] Run the existing contract checkpoint:

```powershell
cargo fmt --all
cargo test -p adam-rs -p adam-lang-book
cargo clippy -p adam-rs --all-targets -- -D warnings
$env:RUSTDOCFLAGS = '-D warnings'
cargo doc --workspace --no-deps --lib
git --no-pager diff --check
```

- [ ] Commit the existing approved changes and this plan separately from implementation.
  Stage the exact modified paths reported by `git status` from this session: the four
  callback API modules, `lib.rs`, `sheet.rs`, `planner\seed.rs`, `tests\integration.rs`,
  the three edited book chapters, the generalization spec/handoff, the selected-guard
  spec's supersession note, and this plan. If unexpected edits appear, inspect them and
  exclude unrelated work.

```powershell
git add -- adam-rs\src\conditional.rs adam-rs\src\filter.rs adam-rs\src\lib.rs adam-rs\src\planner\seed.rs adam-rs\src\relationship.rs adam-rs\src\requirement.rs adam-rs\src\sheet.rs adam-rs\tests\integration.rs adam-lang-book\book-src\filters.md adam-lang-book\book-src\outputs.md adam-lang-book\book-src\relationships.md docs\superpowers\2026-09-27-planner-generalization-phase-a-handoff.md docs\superpowers\specs\2026-09-27-adam-rs-planner-generalization-design.md docs\superpowers\specs\2026-09-28-selected-guard-prerequisites-design.md docs\superpowers\plans\2026-10-01-automatic-plan-reuse.md
git commit -m "docs: establish pure callbacks and refined automatic reuse design" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

## File Structure and Task Dependencies

| File | Responsibility |
|---|---|
| `adam-rs\src\planner\reuse.rs` (new) | Source-prefix certificate, including stable priority ties. |
| `adam-rs\src\planner\seed.rs` | Structural signatures/recipes and current-value seed evaluation. |
| `adam-rs\src\planner.rs` | Re-exports; clarify release-order ownership and seed interfaces. |
| `adam-rs\src\planner\release.rs` | Preserve release semantics; explain certificate use of release order. |
| `adam-rs\src\sheet\cached.rs` (new) | Prepared artifacts, cache acquisition/invalidation, and private reuse tests. |
| `adam-rs\src\sheet.rs` | Mutator hooks, propagation orchestration, post-processing, display queries. |
| `adam-rs\src\sheet\prerequisites.rs` | Existing selected guard-cone computation, invoked only during preparation. |
| `adam-rs\tests\integration.rs` | Public value/diagnostic/error regression coverage. |
| Existing generalization spec/handoff and book chapters | Final documentation of implemented behavior and remaining scope. |

Tasks 1 and 2 are independent research/implementation units. Task 3 consumes both.
Task 4 consumes Task 3. Task 5 consumes Task 4.

Do not concurrently edit `planner.rs` for Tasks 1 and 2: execute their export edits
sequentially even if the focused modules are delegated separately. Keep the minimum
number of agents; the propagation wiring is one tightly coupled task, not a worker pool.

## Shared Interface Ledger

These are crate-private interfaces, not new public API. Define every listed type
in the task named below; later tasks consume these exact names.

**Task 1, `planner\reuse.rs`, re-exported by `planner.rs`:**

```rust
pub(crate) struct SourceCertificate {
    original_order: Vec<CellId>,
    released: HashSet<CellId>,
    tie_ordinals: HashMap<CellId, usize>,
}

impl SourceCertificate {
    pub(crate) fn new(
        cells: &SlotMap<CellId, CellData>,
        original_order: &[CellId],
        released: HashSet<CellId>,
    ) -> Self;

    pub(crate) fn is_released(&self, cell: CellId) -> bool;
    pub(crate) fn contains_cell(&self, cell: CellId) -> bool;
    pub(crate) fn is_valid(&self, cells: &SlotMap<CellId, CellData>) -> bool;
}
```

**Task 2, `planner\seed.rs`, re-exported by `planner.rs`:**

```rust
pub(crate) struct SeedSignatures {
    methods: HashMap<(RelationshipId, usize), Vec<u64>>,
    relationships: HashMap<RelationshipId, Vec<Vec<u64>>>,
}

pub(crate) struct SeedRecipes {
    claimant: HashMap<CellId, RelationshipId>,
    roots: Vec<CellId>,
    cells: HashMap<CellId, SeedRecipe>,
}

enum SeedRecipe {
    Ready(Vec<SeedSibling>),
    Conflict(RelationshipId),
}

#[derive(Clone, Copy)]
struct SeedSibling {
    relationship: RelationshipId,
    method_index: usize,
}

impl SeedSignatures {
    pub(crate) fn new(
        relationships: &SlotMap<RelationshipId, RelationshipData>,
    ) -> Self;
}

impl SeedRecipes {
    pub(crate) fn new(
        execution_order: &[PlanStep],
        seed_steps: &[PlanStep],
        cells: &SlotMap<CellId, CellData>,
        relationships: &SlotMap<RelationshipId, RelationshipData>,
    ) -> Self;
}

pub(crate) fn evaluate_seeds<'source>(
    recipes: &SeedRecipes,
    signatures: &SeedSignatures,
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    source: &dyn Fn(CellId) -> SeedSource<'source>,
    cache: &mut SeedEvaluationCache,
) -> Result<Seeds, Error>;
```

`SeedRecipe::Conflict` records structural ambiguity for one relationship; it is
reported when the corresponding recipe is actually traversed, not during preparation.
Keep existing `SeedSource`, `Seeds`, and `SeedEvaluationCache` definitions.

**Task 3, `sheet\cached.rs`:**

```rust
#[derive(Clone, Copy)]
enum PlanPhase {
    Unconditional,
    Main,
}

struct PreparedPlan {
    plan: Plan,
    active: HashSet<RelationshipId>,
    certificate: SourceCertificate,
    provenance: PlanProvenance,
    prerequisite_steps: Vec<PlanStep>,
    seeds: SeedRecipes,
    signatures: Rc<SeedSignatures>,
    diagnostic_outputs: HashSet<CellId>,
}

struct CachedPlan {
    prepared: Rc<PreparedPlan>,
    needs_certificate_check: bool,
}

#[cfg(test)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct ReuseStats {
    pre_plans: usize,
    main_plans: usize,
    preparations: usize,
    signature_builds: usize,
    certificate_checks: usize,
}

impl Sheet {
    fn invalidate_prepared_plans(&mut self);
    fn invalidate_after_write(&mut self, cell: CellId);
    fn acquire_prepared_plan<'cache>(
        &mut self,
        cache: &'cache mut Option<CachedPlan>,
        active: &HashSet<RelationshipId>,
        phase: PlanPhase,
    ) -> Result<&'cache PreparedPlan, Error>;
    fn propagate_with_caches(
        &mut self,
        pre: &mut Option<CachedPlan>,
        main: &mut Option<CachedPlan>,
    ) -> Result<(), Error>;
}
```

Add `pre_plan_cache: Option<CachedPlan>`, `main_plan_cache: Option<CachedPlan>`,
`seed_signatures: Option<Rc<SeedSignatures>>`, and test-only `reuse_stats` to `Sheet`.
Change `last_plan` to `Option<Rc<PreparedPlan>>`; remove duplicate `last_forced`
and `last_forced_relationships` storage. `last_plan` is the completed display
snapshot, not an eligibility flag.

`Plan.elimination_order` remains the complete original release order for certificates;
its contract no longer states that seeds consume it. Do not sort or overwrite it on
cache hits.

### Task 1: Source-Prefix Certificate

**Files:** Create `adam-rs\src\planner\reuse.rs`; modify `adam-rs\src\planner.rs`
module declarations/exports. Tests live in `reuse.rs`.

**Interfaces:** Produces `SourceCertificate` from the ledger. Consumes existing
`CellId`, `CellData`, `Plan`, and crate-private `Sheet.cells`.

- [ ] **Step 1: Add failing contract tests.** Include this explicit passing/failing
  certificate fixture; set strengths directly only in module tests.

```rust
#[test]
fn certificate_accepts_demotions_and_rejects_crossed_source_prefixes() {
    let mut sheet = Sheet::new();
    let source = sheet.add_cell(0_i32);
    let rejected = sheet.add_cell(0_i32);
    sheet.cells[source].strength = 20;
    sheet.cells[rejected].strength = 10;
    let certificate =
        SourceCertificate::new(&sheet.cells, &[source, rejected], HashSet::from([source]));

    sheet.cells[rejected].strength = 5;
    assert!(certificate.is_valid(&sheet.cells));
    sheet.cells[source].strength = 4;
    assert!(!certificate.is_valid(&sheet.cells));
}
```

Also test empty/all-released/no-released inputs, multiple released prefixes,
source promotion, and equal strengths where the original `cells.keys()` ordinal
breaks the tie. Do not manufacture missing cells that violate the constructor contract.

- [ ] **Step 2: Establish red.**

```powershell
cargo test -p adam-rs --lib planner::reuse::
```

Expected: compilation fails because the certificate interface is absent.

- [ ] **Step 3: Implement the ledger interface.** Capture `cells.keys()` enumeration
  ordinals during construction, not enumeration of the strength-sorted original order.
  Descending strength with ascending ordinal matches the release pass's stable sort.
  This is the entire certificate scan:

```rust
let mut weakest = None;
for &cell in &self.original_order {
    let priority = (cells[cell].strength, std::cmp::Reverse(self.tie_ordinals[&cell]));
    if self.released.contains(&cell) {
        weakest = Some(weakest.map_or(priority, |previous| std::cmp::min(previous, priority)));
    } else if weakest.is_some_and(|source_priority| priority >= source_priority) {
        return false;
    }
}
true
```

Constructor contracts require a complete original order and the released set from
that plan. `is_valid` requires unchanged cells/structure. Debug-assert lengths and
membership; document O(C) construction/scan, O(C) storage, and expected O(1) membership.
Implement `contains_cell` using `tie_ordinals.contains_key`, so completed-plan
queries can recognize cells created after that plan without scanning its order.

- [ ] **Step 4: Verify against actual greedy planning.** Build a three-cell
  relationship with methods `a -> b` and `b -> a` plus an independent `c`. For each
  of the six strict priority permutations, compute a baseline plan and certificate;
  for each new permutation accepted by the certificate, assert that a fresh
  `planner::plan` selects the same relationship method and released-source set.
  Derive the source set from all selected method outputs, including self-reference.
  Do not use the certificate itself to decide expected releases.

- [ ] **Step 5: Run focused checks and commit.**

```powershell
cargo test -p adam-rs --lib planner::reuse::
cargo fmt --all
git add -- adam-rs\src\planner\reuse.rs adam-rs\src\planner.rs
git commit -m "feat(adam-rs): certify prepared source sets after priority changes" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

### Task 2: Structural Seed Recipes and Borrowed Signatures

**Files:** Modify `adam-rs\src\planner\seed.rs`, `adam-rs\src\planner.rs`,
`adam-rs\src\planner\release.rs`, and the two seed-construction call sites in
`adam-rs\src\sheet.rs`. Keep runtime planning unconditional until Task 3.

**Interfaces:** Produces `SeedSignatures`, `SeedRecipes`, `evaluate_seeds` from
the ledger. Keeps `SeedEvaluationCache` strictly per propagation.

- [ ] **Step 1: Add failing structural-selection tests.** For a target `x`,
  select by this predicate, retaining the current absent/conflict distinctions:

```rust
method.outputs.contains(&x)
    && method.outputs.iter().all(|output| *output == x || method.inputs.contains(output))
```

Use existing zero-survivor and ambiguous-survivor fixtures in `seed.rs`; add a
single-survivor fixture and an absent-target fixture. In test-only reference code,
retain the old complete elimination replay and compare its result with prepared
selection for every permutation of each fixture's complete cell list.

Add this current-value contract test using the new evaluator:

```rust
let mut sheet = Sheet::new();
let x = sheet.add_cell(0_i32);
let a = sheet.add_cell(1_i32);
let sibling = sheet.add_relationship(vec![
    Method::from_fn_1_1(a, x, |value: &i32| Ok(*value)),
    Method::from_fn_1_1(x, a, |value: &i32| Ok(*value)),
]).unwrap();
let claimant = sheet.add_relationship(vec![
    Method::from_fn_1_1(x, x, |value: &i32| Ok(*value)),
]).unwrap();
let order = [PlanStep::Method(sibling, 1), PlanStep::Method(claimant, 0)];
let recipes = SeedRecipes::new(&order, &order, &sheet.cells, &sheet.relationships);
let signatures = SeedSignatures::new(&sheet.relationships);
for (input, expected) in [(3_i32, 3_i32), (8_i32, 8_i32)] {
    sheet.write(a, input).unwrap();
    let mut cache = SeedEvaluationCache::default();
    let seeds = evaluate_seeds(
        &recipes, &signatures, &sheet.cells, &sheet.relationships,
        &|id| SeedSource { value: sheet.cells[id].source.as_ref(), version: 0 },
        &mut cache,
    ).unwrap();
    assert_eq!(*seeds[&x].downcast_ref::<i32>().unwrap(), expected);
}
```

- [ ] **Step 2: Establish red.**

```powershell
cargo test -p adam-rs --lib planner::seed::
```

Expected: new recipe/evaluator symbols are missing.

- [ ] **Step 3: Implement signatures and recipes.** Compute method-content keys
  exactly as current `method_content_key` does; store full ordered relationship
  signatures once. Build claimant/active indices from the entire selected assignment.
  Extract requested self-reference roots from `seed_steps`. Walk reachable structural
  siblings with a worklist and a visited set; record `Conflict` recipes without
  returning an error. Do not report cycles during preparation: evaluation's current
  dynamically ordered traversal determines errors and sites.

- [ ] **Step 4: Adapt evaluation without changing its boundaries.** Replace the
  current `SeedBuildContext` elimination/active fields with borrowed recipes/signatures.
  For each target, copy only its small `SeedSibling` list into a local ordering buffer.
  Sort by the existing tuple:

```rust
(strongest_non_target_input_strength, selected_method_signature, full_relationship_signature)
```

Borrow the last two components from `SeedSignatures`. Check equal complete keys
for `Error::Conflict`; do not use relationship insertion order. Preserve the
sequence: sort/ambiguity check, recursive dependency traversal and cycle detection,
cross-phase `SeedShape` check, then strength gate and callback/value provenance.
Preserve the existing behavior for failing/mistyped seed callbacks, not a new error policy.

- [ ] **Step 5: Replace internal seed entry points coherently.** `evaluate_seeds`
  replaces `build_seeds` and `build_seeds_for_steps`; update re-exports, rustdoc
  links in `lib.rs`, `sheet.rs`, planner modules, and test helpers. Temporarily
  prepare signatures once at the start of each `propagate` and recipes for each
  freshly planned phase; Task 3 moves them into structural/prepared caches.
  Remove obsolete seed `elimination_order` parameters and selection replay from
  production. Keep full replay only as a test oracle.

- [ ] **Step 6: Verify and commit.**

```powershell
cargo test -p adam-rs --lib planner::
cargo test -p adam-rs --test integration
cargo fmt --all
git add -- adam-rs\src\planner.rs adam-rs\src\planner\seed.rs adam-rs\src\planner\release.rs adam-rs\src\sheet.rs adam-rs\src\lib.rs
git commit -m "refactor(adam-rs): prepare structural seed recipes without elimination replay" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

### Task 3: Prepared Artifacts and Automatic Propagation

**Files:** Create `adam-rs\src\sheet\cached.rs`; modify `adam-rs\src\sheet.rs`.
Reuse `sheet\prerequisites.rs`; do not duplicate its guard walk.

**Interfaces:** Produces the prepared/cache/phase/stat types and `Sheet` methods
from the ledger. Consumes Tasks 1 and 2.

- [ ] **Step 1: Add failing reuse tests in `cached.rs`.** Define a fixture helper
  with two ordinary cells `a`, `b`, and identity alternatives `a -> b`, `b -> a`.
  `b` is initially stronger. Assert these exact observable results and private
  optimization counters:

```rust
sheet.propagate().unwrap();
let initial = sheet.reuse_stats;
assert_eq!(initial.main_plans, 1);
assert_eq!(initial.pre_plans, 0);
sheet.propagate().unwrap();
assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans);
assert_eq!(sheet.reuse_stats.preparations, initial.preparations);
assert_eq!(sheet.reuse_stats.signature_builds, initial.signature_builds);
sheet.write(b, 9_i32).unwrap();
sheet.propagate().unwrap();
assert_eq!(*sheet.read::<i32>(a).unwrap(), 9);
assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans);
sheet.write(a, 4_i32).unwrap();
sheet.propagate().unwrap();
assert_eq!(*sheet.read::<i32>(b).unwrap(), 4);
assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans + 1);
```

Add an empty-sheet repeat test and a filtered released-source test; neither should
create a pre-plan. Counters belong to the sheet, not global atomics or callbacks.
After a completed plan, add a cell and assert `is_source(new_cell)` before the next
propagation, while existing `selected_method` still reports the completed plan.

- [ ] **Step 2: Establish red.**

```powershell
cargo test -p adam-rs --lib sheet::cached::
```

Expected: cache/stat fields are absent or repeated propagation increments planning.

- [ ] **Step 3: Build immutable prepared artifacts.** `acquire_prepared_plan`
  reuses only when active sets match and an outstanding certificate check passes.
  On a miss, call `planner::plan`, build source membership from all method outputs,
  build `SourceCertificate`, call existing `plan_provenance`, and for unconditional
  plans call existing `guard_prerequisite_steps`. Prepare phase-specific seed roots
  and diagnostic output membership. Retain shared structural signatures with
  `Rc<SeedSignatures>`; increment stats at the actual operations.

- [ ] **Step 4: Move propagation into `propagate_with_caches`.** Take eligibility
  entries out of the sheet to avoid borrowing a mutable sheet and its immutable
  prepared fields simultaneously. The public wrapper's ownership pattern is:

```rust
let mut pre = self.pre_plan_cache.take();
let mut main = self.main_plan_cache.take();
let result = self.propagate_with_caches(&mut pre, &mut main);
if result.is_ok() {
    self.pre_plan_cache = pre;
    self.main_plan_cache = main;
}
result
```

Keep `validate()` and `clear_changed()` in their existing relative order inside
the worker; even validation failure discards taken eligibility entries. On success,
publish the main `Rc<PreparedPlan>` as `last_plan` only after diagnostics complete.
Do not erase the previous display snapshot on failure.

Borrow prepared step slices, provenance, recipes, signatures, and forced sets.
Use one fresh stage and seed evaluation cache for both phases. Keep every existing
compatibility check. Replace the final selected-output scan with prepared
`diagnostic_outputs`, but still evaluate current filter diagnostics.

- [ ] **Step 5: Wire every actual mutation.**

| Operation in `sheet.rs` | Hook location |
|---|---|
| `add_cell` | Before inserting the new cell. |
| `add_source` | Covered by `add_cell`; preserve kind change before next preparation. |
| `add_relationship` | After all validation, before relationship/adjacency insertion. |
| `add_conditional` | After validation, before conditional relationship membership changes. |
| `add_filter` | After validation, before reverse-index/filter attachment. |
| `add_requirement` | After validation/current predicate check, before requirement insertion. |
| `add_out` | Relationship/requirement hooks cover mutations even on partial failure. |
| `write` | After ID/kind/type validation, before value/strength mutation. |

`invalidate_prepared_plans` drops both eligibility caches and structural signatures,
but leaves `guard_independent` and display state governed by their existing contracts.
`invalidate_after_write` drops each cache independently when its certificate does not
release the written cell; it does not clear structural signatures.

- [ ] **Step 6: Account for post-processing and queries.** Make
  `post_process_strengths(&mut self, order: &[PlanStep]) -> bool` return whether
  any actual strength changed; preserve all assignment rules. If true, mark both
  local cache entries as needing certificate checks before future reuse. Do not
  call the solver merely because this flag is true.

Update every consumer of `last_plan`: `is_relationship_active`, `selected_method`,
`is_source`, `is_forced`, `forced_cells`, `is_relationship_forced`, and
`forced_relationships`; also check `cell_requirements_valid`'s initial-state branch.
Read completed data through `last_plan.plan`/`last_plan.certificate`, not eligibility
entries. Preserve missing/new-ID behavior: a cell added after propagation remains
reported as a source by the old API, even though it is absent from the old certificate.
Use `SourceCertificate::contains_cell` to distinguish new cells
from old claimed cells when implementing `is_source`, without scanning the plan.

- [ ] **Step 7: Verify and commit.**

```powershell
cargo test -p adam-rs --lib sheet::
cargo test -p adam-rs --test integration --test dependency_guards
cargo clippy -p adam-rs --all-targets -- -D warnings
cargo fmt --all
git add -- adam-rs\src\sheet.rs adam-rs\src\sheet\cached.rs
git commit -m "feat(adam-rs): automatically reuse prepared propagation plans" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

### Task 4: Differential, Invalidation, and Failure Coverage

**Files:** Tests in `adam-rs\src\sheet\cached.rs` and
`adam-rs\tests\integration.rs`; correct only related wiring exposed by these tests.

**Interfaces:** Consumes `ReuseStats`, completed-plan queries, and the two private
eligibility slots. Define a test-only full-replan helper, not a public execution mode:

```rust
fn propagate_forced_replan(sheet: &mut Sheet) -> Result<(), Error> {
    sheet.pre_plan_cache = None;
    sheet.main_plan_cache = None;
    sheet.propagate()
}
```

- [ ] **Step 1: Add paired-sheet snapshots and write sequences.** Build two identical
  fixtures and retain their own IDs; never pass one sheet's IDs to the other.
  Compare `read::<i32>`, `source::<i32>`, `selected_method`, `is_source`, `is_forced`,
  forced relationship classification, and changed membership by fixture ordinal.
  Normalize error sites by fixture cell/relationship ordinals, and filter errors by
  variant/message rather than comparing `anyhow::Error` object identity.
  Run automatic vs forced propagation with:

```rust
for (target, value) in [(1_usize, 5_i32), (1, 8), (0, 3), (1, -2), (0, 0)] {
    automatic.write(automatic_ids[target], value).unwrap();
    forced.write(forced_ids[target], value).unwrap();
    automatic.propagate().unwrap();
    propagate_forced_replan(&mut forced).unwrap();
    assert_same_snapshot(&automatic, &automatic_ids, &forced, &forced_ids);
}
```

Define this snapshot comparison in the private test module. These fixtures contain
`i32` cells; use identity relationships so no unchecked arithmetic is needed.

```rust
/// Returns a fixture cell's comparable current filter diagnostic.
///
/// - Complexity: O(error message length).
fn filter_status(sheet: &Sheet, cell: CellId) -> Option<String> {
    sheet.filter_violation(cell).map(|violation| match violation {
        FilterViolation::NotConformed => "not-conformed".to_owned(),
        FilterViolation::Failed(error) => format!("failed:{error}"),
    })
}

/// Returns violated-requirement membership in attachment order for a fixture cell.
///
/// - Complexity: O(Q²), where Q is the cell's requirement count.
fn requirement_status(sheet: &Sheet, cell: CellId) -> Vec<bool> {
    sheet.cell_requirements(cell).expect("registered fixture cell").iter()
        .map(|requirement| sheet.violated_requirements(cell).any(|id| id == *requirement))
        .collect()
}

/// Asserts equivalent completed state for corresponding integer fixture cells.
///
/// - Complexity: O(C² + R²K + sum(Q²) + diagnostic message lengths).
///   C counts fixture cells, R relationships, K method outputs, and Q requirements per cell.
fn assert_same_snapshot(
    automatic: &Sheet,
    automatic_ids: &[CellId],
    forced: &Sheet,
    forced_ids: &[CellId],
) {
    assert_eq!(automatic_ids.len(), forced_ids.len());
    for (&left, &right) in automatic_ids.iter().zip(forced_ids) {
        assert_eq!(automatic.read::<i32>(left).unwrap(), forced.read::<i32>(right).unwrap());
        assert_eq!(automatic.source::<i32>(left).unwrap(), forced.source::<i32>(right).unwrap());
        assert_eq!(automatic.is_source(left), forced.is_source(right));
        assert_eq!(automatic.is_forced(left), forced.is_forced(right));
        assert_eq!(
            automatic.changed().any(|id| id == left),
            forced.changed().any(|id| id == right),
        );
        assert_eq!(automatic.cell_requirements_valid(left), forced.cell_requirements_valid(right));
        assert_eq!(requirement_status(automatic, left), requirement_status(forced, right));
        assert_eq!(filter_status(automatic, left), filter_status(forced, right));
    }
    let choices = |sheet: &Sheet| {
        sheet.relationships().map(|relationship| {
            (sheet.selected_method(relationship), sheet.is_relationship_forced(relationship))
        }).collect::<Vec<_>>()
    };
    assert_eq!(choices(automatic), choices(forced));
}
```

Keep fixture relationship creation order identical. For boolean guards, compare
their `read::<bool>` and `source::<bool>` values separately instead of passing
their IDs to this integer-only helper.

- [ ] **Step 2: Add phase-specific and strength tests.** Fixture: unconditional
  `a -> b`/`b -> a`, a true branch `source -> a`, and an independent source guard.
  Warm until the pre-plan certificate is valid after main post-processing, recording
  why an initial pre-plan check fails if main demotion changes its source prefix.
  Write the current pre-plan released cell (claimed in the main plan); assert
  `pre_plans` does not increase, `main_plans` increases, and snapshots match forced
  planning. Also test branch flips and branch-preserving guard writes. Test equal
  priority ties directly in the certificate module, not through invalid public writes.

- [ ] **Step 3: Cover all structural surfaces.** After a valid cached propagation,
  add a cell/source, relationship, conditional, filter, requirement, or out writer,
  call `validate()` where applicable, then propagate. Assert preparation/plan counters
  increase and current values match the forced path. For rejected writes (wrong type,
  missing cell, Out-kind), assert the error and unchanged planning counters on the
  next eligible propagation.

Partial-mutation fixture: create an unused output candidate before warming; call
`add_out` with its writer and two requirements having the same `Some("check")`
name. Assert `InvalidRequirement`, confirm the writer/cell-kind/first requirement
persist, then confirm propagation rebuilds instead of using the old artifact.

- [ ] **Step 4: Preserve dynamic seed/diagnostic/error semantics.** Reuse existing
  inequality-chain, sibling fold-order, spring-back, filtered-guard, restaged-seed,
  prerequisite-conflict, and seed-cycle fixtures; execute paired source-write
  sequences after warm cache hits. Compare current filter violations and requirement
  IDs by fixture ordinal. Include:

```rust
Requirement::from_fn_1(input, |value: &i32| {
    if *value == 7 {
        Err(anyhow::anyhow!("requirement probe"))
    } else {
        Ok(*value >= 0)
    }
})
```

Attach at a valid initial value; at input `7`, verify the existing post-commit
requirement error boundary and unchanged previous display plan, then recover with
a valid write and require fresh preparation. For method failure, use a pure closure
that errors for input `7`; verify no staged outputs commit and `changed()` is empty.
Do not use stateful toggles or callback counters to induce failure/recovery.

- [ ] **Step 5: Establish red, fix diagnosed gaps, and verify.** Run the selectors
  below after adding tests and before any wiring correction. A failure is either a
  missing coverage helper or a specific semantic/counter mismatch; investigate its
  first failing assertion. If they all pass, retain them as regression coverage
  without inventing production changes merely to obtain red.

```powershell
cargo test -p adam-rs --lib sheet::cached::
cargo test -p adam-rs --test integration --test dependency_guards
cargo test -p adam-lang
cargo test -p adam-lang-book
```

- [ ] **Step 6: Commit the tested coverage/corrections.**

```powershell
cargo fmt --all
git add -- adam-rs\src\sheet\cached.rs adam-rs\tests\integration.rs
```

Stage any tightly related production correction explicitly if one was needed.

```powershell
git commit -m "test(adam-rs): verify prepared reuse against full replanning" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

### Task 5: Documentation, Whole-Branch Review, and Full Validation

**Files:** `adam-rs\src\sheet.rs`/planner docs, the existing generalization
spec/handoff, and directly related book descriptions of propagation.

**Interfaces:** No new runtime interfaces. Consumers receive the same public API.

- [ ] **Step 1: Update implemented status and contracts.** Describe automatic reuse,
  O(1)-expected write bookkeeping, conditional O(C) certificate checks, prepared
  structural seed selection, remaining local fold ordering, and current diagnostic/
  failure semantics. Remove obsolete seed-elimination-replay links and the assumption
  that `propagate` always invokes the planner. Retain the no-invocation-guarantee
  purity contract. Record exactly which differential and operation-count tests pass
  in the handoff; do not claim cross-propagation value caching or incremental repair.

- [ ] **Step 2: Review the branch against every §3 acceptance criterion.** Inspect
  invalidation before actual mutations, tie ordinals, phase-specific sources, newly
  added cell display queries, provenance checks, fresh seed caches, diagnostics,
  partial `add_out` failure, and after-commit requirement failures. Resolve in-scope
  findings immediately. If a genuine larger/out-of-scope defect is found, open a
  GitHub issue and reference it in the handoff rather than silently parking it.

- [ ] **Step 3: Run the complete repository check suite.**

```powershell
cargo fmt --all
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
cargo clippy -p ez-adam --no-default-features --all-targets -- -D warnings
cargo clippy -p ez-adam --all-targets -- -D warnings
$env:RUSTDOCFLAGS = '-D warnings'
cargo doc --workspace --no-deps --lib
git --no-pager diff --check
```

Read each exit code and compiler output; stop on failure. Build/test must emit zero
compiler warnings. No UI was changed, so rendered verification is not required.
Do not install dependencies unless a validation command reports a missing dependency.

- [ ] **Step 4: Commit documentation and handoff.** Stage only the documented paths
  changed in this task, then:

```powershell
git commit -m "docs(adam-rs): record automatic prepared-plan reuse completion" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

Opening a PR is a separate user decision; this plan does not authorize pushing,
merging, or closing #152.

## Plan Self-Review and Acceptance Map

| Spec requirement | Task |
|---|---|
| §3.1 Prepared artifacts, no complete artifact cloning on hits | 3 |
| §3.2 Source-prefix proof, stable ties, phase-specific invalidation | 1, 3, 4 |
| §1.2 / §3.3 Structural seed equivalence and dynamic folds | 2, 4 |
| §3.4 Staging, queries, changed-state, failure boundaries | 3, 4 |
| §3.5 Purity and evaluation-reuse scope | Checkpoint, 2, 3, 5 |
| §3.6 Reduced hot-path work without measured-speedup claims | 2, 3, 4, 5 |
| §3.7 Differential, operation counts, mutators, errors | 1, 2, 3, 4 |

No new dependencies, public execution modes, callback-count contracts, or unrelated
UI changes are part of this plan. Tasks 1/2 can be reviewed independently; propagation
integration is deliberately one coherent task.
