//! Seedfill: reconstructs, for each self-referencing input a plan will read, the value
//! that cell *aspires* to before its own claiming relationship tightens it.
//!
//! A self-referencing method `X := f(X, ..)` reads `X`'s own value as one input. Its
//! `source` is frozen at the last explicit `write`, which goes stale the moment another
//! relationship pushes `X` around (e.g. `a <= b` forcing `b` up to match `a`): `b`'s
//! `source` still reads its original declaration even though the chain has settled it
//! elsewhere. Reading `source` directly then discards a consistent edit; reading the
//! previous round's `derived` value instead re-introduces a shrinking accumulator.
//!
//! Seedfill computes the right value structurally, without the planner ever comparing
//! candidate values: `X`'s seed is what `X` would settle to under every relationship
//! incident to it *except* its own claimant, evaluated from `source` values. In the
//! `a<=b<=c` chain, `b`'s claimant (the `b<=c` relationship) is set aside and `b` is
//! seeded by the `a<=b` relationship's `b`-producing method (`b := max(a, b)`), giving
//! `b`'s aspiration from `a`; the claimant then tightens it. Because the seed is rebuilt
//! from `source` every round, it is never stale and never accumulates -- see
//! `docs/superpowers/specs/2026-09-07-adam-rs-value-aware-self-ref-planning-design.md`.
//!
//! One sibling fold is gated by relative *strength* (a cheap field comparison, not a
//! candidate-value comparison): when `a<=b`'s claimant this round is itself `a` (not
//! `b`), folding `a<=b`'s `b`-producing method to seed some other cell `x` would use `a`'s
//! `source` as if it were authoritative -- but if `a` was written *before* `x`, `a`'s
//! `source` is exactly the stale declaration this module exists to avoid re-introducing.
//! `compute_seed` skips such a fold unless the self-referenced input genuinely outranks
//! `x` in strength; see its doc comment.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use slotmap::{Key, SlotMap};

use crate::cell::{CellData, CellId};
use crate::error::{Error, ErrorSite};
use crate::relationship::{RelationshipData, RelationshipId};

use super::{PlanStep, Seeds};

/// Stores immutable ordered method and relationship signatures for one graph structure.
pub(crate) struct SeedSignatures {
    methods: HashMap<(RelationshipId, usize), Vec<u64>>,
    relationships: HashMap<RelationshipId, Vec<Vec<u64>>>,
}

impl SeedSignatures {
    /// Encodes each method's declared cell order and each relationship's method order.
    ///
    /// - Complexity: O(M · K) time and space for M methods with K referenced cells.
    pub(crate) fn new(relationships: &SlotMap<RelationshipId, RelationshipData>) -> Self {
        let mut methods = HashMap::new();
        let mut signatures = HashMap::new();
        for (relationship, data) in relationships {
            let signature: Vec<_> = data.methods.iter().map(method_content_key).collect();
            for (index, key) in signature.iter().enumerate() {
                methods.insert((relationship, index), key.clone());
            }
            signatures.insert(relationship, signature);
        }
        Self {
            methods,
            relationships: signatures,
        }
    }
}

/// Stores structural seed choices reachable from the requested selected-plan roots.
///
/// Preparation records conflicts without reporting them; evaluation visits roots in plan order.
pub(crate) struct SeedRecipes {
    claimant: HashMap<CellId, RelationshipId>,
    roots: Vec<CellId>,
    cells: HashMap<CellId, SeedRecipe>,
}

/// Records either a cell's structural siblings or an ambiguous sibling relationship.
enum SeedRecipe {
    Ready(Vec<SeedSibling>),
    Conflict(RelationshipId),
}

/// Identifies a unique target-producing method of an active sibling relationship.
#[derive(Clone, Copy)]
struct SeedSibling {
    relationship: RelationshipId,
    method_index: usize,
}

impl SeedRecipes {
    /// Prepares reachable sibling choices without inspecting strengths or source values.
    ///
    /// - Precondition: Method steps name valid methods in `relationships`.
    /// - Precondition: `seed_steps` is a subset of `execution_order`.
    /// - Postcondition: Conflicts and cycles remain lazy until evaluation traverses a root.
    /// - Complexity: O(V · K² + C · A · M · K²) time and O(V · K + C · A · K) space for
    ///   V selected steps, C reachable cells, A incident relationships, M methods, and
    ///   K referenced cells per method.
    pub(crate) fn new(
        execution_order: &[PlanStep],
        seed_steps: &[PlanStep],
        cells: &SlotMap<CellId, CellData>,
        relationships: &SlotMap<RelationshipId, RelationshipData>,
    ) -> Self {
        #[cfg(debug_assertions)]
        {
            let steps: HashSet<_> = execution_order.iter().copied().collect();
            debug_assert!(seed_steps.iter().all(|step| steps.contains(step)));
            debug_assert!(execution_order.iter().all(|step| {
                match *step {
                    PlanStep::Method(relationship, index) => relationships
                        .get(relationship)
                        .is_some_and(|data| index < data.methods.len()),
                    PlanStep::FilterReclamp(_) => true,
                }
            }));
        }
        let mut claimant = HashMap::new();
        let mut active = HashSet::new();
        for &step in execution_order {
            if let PlanStep::Method(relationship, index) = step {
                active.insert(relationship);
                for &output in &relationships[relationship].methods[index].outputs {
                    claimant.insert(output, relationship);
                }
            }
        }
        let mut roots = Vec::new();
        for &step in seed_steps {
            if let PlanStep::Method(relationship, index) = step {
                let method = &relationships[relationship].methods[index];
                roots.extend(
                    method
                        .outputs
                        .iter()
                        .copied()
                        .filter(|id| method.inputs.contains(id)),
                );
            }
        }
        let mut recipes = HashMap::new();
        let mut worklist = roots.clone();
        let mut visited = HashSet::new();
        while let Some(target) = worklist.pop() {
            if !visited.insert(target) {
                continue;
            }
            let mut siblings = Vec::new();
            let mut conflict = None;
            for &relationship in &cells[target].adj {
                if !active.contains(&relationship) || claimant.get(&target) == Some(&relationship) {
                    continue;
                }
                match select_seed_method(relationship, target, relationships) {
                    Ok(Some(method_index)) => siblings.push(SeedSibling {
                        relationship,
                        method_index,
                    }),
                    Ok(None) => {}
                    Err(_) => {
                        conflict = Some(relationship);
                        break;
                    }
                }
            }
            let recipe = if let Some(relationship) = conflict {
                SeedRecipe::Conflict(relationship)
            } else {
                for sibling in &siblings {
                    worklist.extend(
                        relationships[sibling.relationship].methods[sibling.method_index]
                            .inputs
                            .iter()
                            .copied()
                            .filter(|&input| input != target),
                    );
                }
                SeedRecipe::Ready(siblings)
            };
            recipes.insert(target, recipe);
        }
        Self {
            claimant,
            roots,
            cells: recipes,
        }
    }
}

/// Borrows immutable recipe, signature, and current-value state for one evaluation.
struct SeedBuildContext<'context, 'source> {
    recipes: &'context SeedRecipes,
    signatures: &'context SeedSignatures,
    cells: &'context SlotMap<CellId, CellData>,
    relationships: &'context SlotMap<RelationshipId, RelationshipData>,
    source: &'context dyn Fn(CellId) -> SeedSource<'source>,
}

/// One cell's staged source value and the version identifying that exact value.
///
/// Within one propagation call, two reads of the same cell with equal `version`s observe
/// the same source value; a staged source write produces a different version.
#[derive(Clone, Copy)]
pub(crate) struct SeedSource<'source> {
    /// The cell's current staged source value.
    pub(crate) value: &'source dyn Any,
    /// Identifies `value` among every source value the cell holds during one propagation.
    pub(crate) version: u64,
}

/// Stores the mutable memoization and DFS path state for one seed-construction pass.
struct SeedTraversal {
    seeds: Seeds,
    path_cells: Vec<CellId>,
    path_relationships: Vec<RelationshipId>,
    path_indices: HashMap<CellId, usize>,
}

/// Memoizes prerequisite seed choices and callback results across phases of one propagation.
#[derive(Default)]
pub(crate) struct SeedEvaluationCache {
    shapes: HashMap<CellId, SeedShape>,
    callbacks: HashMap<SeedCallbackId, CachedSeedCallback>,
}

/// The chosen sibling-method set that defines one cell's seed evaluation.
#[derive(PartialEq, Eq)]
struct SeedShape {
    claimant: Option<RelationshipId>,
    siblings: Vec<(RelationshipId, usize)>,
}

/// One seed callback invocation, distinguished from selected method execution.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SeedCallbackId {
    target: CellId,
    relationship: RelationshipId,
    method_index: usize,
}

/// Identifies the exact value one seed callback input read without comparing dynamic values.
#[derive(Clone)]
enum SeedInputProvenance {
    /// Reads the cell's staged `source` value at the given source version.
    Source(CellId, u64),
    /// Reads the seed computed for the referenced cell; equal only for the same allocation.
    Seed(CellId, Rc<dyn Any>),
    /// Reads the current accumulation for the self-referencing target; equal only for the
    /// same allocation.
    Accumulated(CellId, Rc<dyn Any>),
}

impl PartialEq for SeedInputProvenance {
    /// Returns whether both inputs name the same cell and the identical value.
    ///
    /// Seed values are compared by allocation identity: every cached seed result is kept
    /// alive by the cache, so an equal address always denotes the same evaluation result.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Source(lhs, lhs_version), Self::Source(rhs, rhs_version)) => {
                lhs == rhs && lhs_version == rhs_version
            }
            (Self::Seed(lhs, lhs_value), Self::Seed(rhs, rhs_value))
            | (Self::Accumulated(lhs, lhs_value), Self::Accumulated(rhs, rhs_value)) => {
                lhs == rhs && Rc::ptr_eq(lhs_value, rhs_value)
            }
            _ => false,
        }
    }
}

/// A seed callback's recorded input origins and reusable target output.
struct CachedSeedCallback {
    inputs: Vec<SeedInputProvenance>,
    value: Option<Rc<dyn Any>>,
}

/// Lexicographic tie-break data for one sibling seed fold.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SeedFoldOrderKey<'signature> {
    strongest_input: u64,
    selected_method_signature: &'signature [u64],
    relationship_signature: &'signature [Vec<u64>],
}

/// Evaluates prepared roots against current strengths and staged source values.
///
/// A cell absent from the result reads its own `source` during execution (the common
/// case: no other relationship pushes it). Requested self-referencing roots and recursive
/// inputs receive entries only when an active sibling contributes a valid value.
/// Structural sibling choices are prepared independently of release order;
/// their evaluation order reflects current input strengths and immutable signatures.
///
/// - Precondition: `recipes` and `signatures` describe the current graph structure.
/// - Precondition: `source(id).value` is a value of `cells[id].type_id` for every live
///   `id` referenced by `recipes`.
/// - Precondition: `cache` is shared only across planning phases for one propagation call,
///   and `source(id).version` identifies the returned value across all of those phases.
///
/// # Errors
///
/// - `Error::Conflict` — a sibling relationship that can seed a self-referencing cell
///   has no unique structurally compatible method, or two sibling seed
///   folds are still structurally indistinguishable after comparing primary strength,
///   selected method signature, and full relationship signature.
/// - `Error::Conflict` — a cached seed claimant or sibling selection differs from the
///   earlier prerequisite evaluation, or a cached callback's input would now read a
///   different source version or a different recursive seed or accumulated value. The
///   current implementation reports the mismatch rather than recomputing the seed;
///   this is not a public callback invocation guarantee.
/// - `Error::SeedCycle` — sibling seed dependencies form a non-self cycle instead of
///   falling back to any revisited cell's `source` value.
///
/// Failed, wrong-arity, or mistyped seed callback outputs contribute no value.
///
/// - Complexity: O(T · A log(A + 1) · (K + L) + (A + K)²) excluding callback
///   costs, where T = traversal visits (including revisited cells without seed values),
///   A = siblings, K = inputs per method, and L = signature comparison length.
///   Traversal uses O(C · A · K) space for C distinct cells, including provenance.
pub(crate) fn evaluate_seeds<'source>(
    recipes: &SeedRecipes,
    signatures: &SeedSignatures,
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    source: &dyn Fn(CellId) -> SeedSource<'source>,
    cache: &mut SeedEvaluationCache,
) -> Result<Seeds, Error> {
    let context = SeedBuildContext {
        recipes,
        signatures,
        cells,
        relationships,
        source,
    };
    let mut traversal = SeedTraversal {
        seeds: HashMap::new(),
        path_cells: Vec::new(),
        path_relationships: Vec::new(),
        path_indices: HashMap::new(),
    };
    for &cell in &recipes.roots {
        compute_seed(cell, &context, &mut traversal, cache)?;
    }
    Ok(traversal.seeds)
}

/// Computes seeds against the sheet's live source values for unit tests.
///
/// # Errors
///
/// Returns the structural, cycle, and compatibility errors of [`evaluate_seeds`].
///
/// - Complexity: Recipe/signature preparation plus [`evaluate_seeds`]'s traversal cost.
#[cfg(test)]
fn evaluate_seeds_live(
    execution_order: &[PlanStep],
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
) -> Result<Seeds, Error> {
    let mut cache = SeedEvaluationCache::default();
    let recipes = SeedRecipes::new(execution_order, execution_order, cells, relationships);
    let signatures = SeedSignatures::new(relationships);
    evaluate_seeds(
        &recipes,
        &signatures,
        cells,
        relationships,
        &|id| SeedSource {
            value: cells[id].source.as_ref(),
            version: 0,
        },
        &mut cache,
    )
}

/// Populates `seeds[x]` with `x`'s aspiration, computed by folding every incident
/// relationship other than `x`'s claimant through its structurally compatible
/// `x`-producing method, ordered by the strongest non-`x` input's strength from weakest
/// to strongest, seeded from `x`'s `source` and each other input's own seed
/// (recursively). Leaves `x` absent when no such relationship exists (its seed is just
/// `source`) or when every candidate method errors or mistypes its output.
///
/// Only active relationships recorded in `context.recipes` count as
/// siblings: an inactive conditional branch that happens to name `x` must not seed it.
///
/// `traversal.path_cells`/`traversal.path_relationships` record the active seed
/// dependency path. A sibling input that reaches a cell already on that path reports
/// `Error::SeedCycle` naming the participating cells and sibling relationships in
/// traversal order.
///
/// A sibling method is skipped (contributes nothing) when `x` itself holds a live
/// explicit strength (a `write()`/`add_cell` not yet superseded by a later claim) *and*
/// some other input `y` to that method is claimed this round, self-referencingly, by
/// that same sibling relationship, with `y`'s strength not exceeding `x`'s. Folding that
/// method would use `y`'s stale `source` to override `x`'s own explicit edit even though
/// `y` is not the stronger party, and the relationship's own chosen method this round
/// already computes `y` from `x` (not the reverse). Gating on `x`'s *own* strength being
/// explicit (rather than comparing `y` against whatever value `x` currently holds,
/// explicit or not) matters because a purely derived `x` has no live explicit edit to
/// protect, and its post-round strength is only an execution-order tie-break, not a
/// genuine priority signal -- treating it as one here would incorrectly suppress the
/// aspiration fold for the ordinary case this module exists to handle. This is the same
/// relative-strength test `release::resolve` uses elsewhere, not a comparison of
/// candidate values. Among the sibling methods that survive that gate, weaker
/// relationships fold first and the strongest surviving influence folds last so the
/// final seed respects strength rather than relationship insertion order; equal-primary
/// folds break ties by the selected method's ordered signature and then the
/// relationship's full ordered method-signature sequence. If both relationships are
/// still structurally identical under that comparison, propagation rejects the
/// ambiguous shape instead of folding in adjacency order.
///
/// # Errors
///
/// - `Error::Conflict` — some sibling relationship that can seed `x` has no surviving
///   structurally compatible `x`-producing method, or two
///   equal-primary sibling folds remain structurally indistinguishable even after
///   comparing the full ordered method-signature sequence of their relationships.
/// - `Error::SeedCycle` — a sibling seed dependency reaches a currently visiting cell.
///
/// - Complexity: O(T · A log(A + 1) · (K + L) + (A + K)²) excluding callback
///   costs, for T traversal visits, A siblings, K inputs, and signature length L.
fn compute_seed(
    x: CellId,
    context: &SeedBuildContext<'_, '_>,
    traversal: &mut SeedTraversal,
    cache: &mut SeedEvaluationCache,
) -> Result<(), Error> {
    if traversal.seeds.contains_key(&x) {
        return Ok(());
    }
    debug_assert!(!traversal.path_indices.contains_key(&x));
    traversal.path_indices.insert(x, traversal.path_cells.len());
    traversal.path_cells.push(x);

    let own_claimant = context.recipes.claimant.get(&x).copied();
    let mut sibling_methods = match &context.recipes.cells[&x] {
        SeedRecipe::Ready(siblings) => siblings.clone(),
        SeedRecipe::Conflict(relationship) => {
            return Err(Error::Conflict {
                sites: vec![ErrorSite::Relationship(*relationship)],
            });
        }
    };
    let order_key = |sibling: &SeedSibling| {
        seed_fold_order_key(
            sibling.relationship,
            sibling.method_index,
            x,
            context.cells,
            context.relationships,
            context.signatures,
        )
    };
    sibling_methods.sort_by(|lhs, rhs| order_key(lhs).cmp(&order_key(rhs)));
    for window in sibling_methods.windows(2) {
        if order_key(&window[0]) == order_key(&window[1]) {
            return Err(Error::Conflict {
                sites: vec![
                    ErrorSite::Relationship(window[0].relationship),
                    ErrorSite::Relationship(window[1].relationship),
                ],
            });
        }
    }

    for &SeedSibling {
        relationship: rel_id,
        method_index: method_idx,
    } in &sibling_methods
    {
        for &input in &context.relationships[rel_id].methods[method_idx].inputs {
            if input != x {
                if let Some(&cycle_start) = traversal.path_indices.get(&input) {
                    let sites = seed_cycle_sites(
                        &traversal.path_cells,
                        &traversal.path_relationships,
                        cycle_start,
                        rel_id,
                        x,
                    );
                    traversal.path_indices.remove(&x);
                    traversal.path_cells.pop();
                    return Err(Error::SeedCycle { sites });
                }
                traversal.path_relationships.push(rel_id);
                let result = compute_seed(input, context, traversal, cache);
                traversal.path_relationships.pop();
                if let Err(err) = result {
                    traversal.path_indices.remove(&x);
                    traversal.path_cells.pop();
                    return Err(err);
                }
            }
        }
    }

    let shape = SeedShape {
        claimant: own_claimant,
        siblings: sibling_methods
            .iter()
            .map(|sibling| (sibling.relationship, sibling.method_index))
            .collect(),
    };
    if let Some(previous) = cache.shapes.get(&x) {
        if previous != &shape {
            let mut sites = vec![ErrorSite::Cell(x)];
            for relationship in [previous.claimant, own_claimant].into_iter().flatten() {
                let site = ErrorSite::Relationship(relationship);
                if !sites.contains(&site) {
                    sites.push(site);
                }
            }
            for &(relationship, _) in previous.siblings.iter().chain(shape.siblings.iter()) {
                let site = ErrorSite::Relationship(relationship);
                if !sites.contains(&site) {
                    sites.push(site);
                }
            }
            traversal.path_indices.remove(&x);
            traversal.path_cells.pop();
            return Err(Error::Conflict { sites });
        }
    } else {
        cache.shapes.insert(x, shape);
    }

    let mut accumulated: Option<Rc<dyn Any>> = None;
    for &SeedSibling {
        relationship: rel_id,
        method_index: method_idx,
    } in &sibling_methods
    {
        let method = &context.relationships[rel_id].methods[method_idx];

        let has_weaker_self_referenced_input = context.cells[x].has_explicit_strength()
            && method.inputs.iter().any(|&input| {
                input != x
                    && context.recipes.claimant.get(&input) == Some(&rel_id)
                    && context.cells[input].strength <= context.cells[x].strength
            });
        if has_weaker_self_referenced_input {
            continue;
        }

        let mut input_provenance = Vec::with_capacity(method.inputs.len());
        let inputs: Vec<&dyn Any> = method
            .inputs
            .iter()
            .map(|&input| {
                if input == x {
                    if let Some(value) = accumulated.as_ref() {
                        input_provenance.push(SeedInputProvenance::Accumulated(x, value.clone()));
                        value.as_ref()
                    } else {
                        let source = (context.source)(x);
                        input_provenance.push(SeedInputProvenance::Source(x, source.version));
                        source.value
                    }
                } else if let Some(value) = traversal.seeds.get(&input) {
                    input_provenance.push(SeedInputProvenance::Seed(input, value.clone()));
                    value.as_ref()
                } else {
                    let source = (context.source)(input);
                    input_provenance.push(SeedInputProvenance::Source(input, source.version));
                    source.value
                }
            })
            .collect();

        let callback_id = SeedCallbackId {
            target: x,
            relationship: rel_id,
            method_index: method_idx,
        };
        let produced = if let Some(cached) = cache.callbacks.get(&callback_id) {
            if cached.inputs != input_provenance {
                let mut sites = vec![ErrorSite::Relationship(rel_id), ErrorSite::Cell(x)];
                for &input in &method.inputs {
                    let site = ErrorSite::Cell(input);
                    if !sites.contains(&site) {
                        sites.push(site);
                    }
                }
                traversal.path_indices.remove(&x);
                traversal.path_cells.pop();
                return Err(Error::Conflict { sites });
            }
            cached.value.clone()
        } else {
            let value = (method.function)(&inputs)
                .ok()
                .filter(|outputs| outputs.len() == method.outputs.len())
                .and_then(|mut outputs| {
                    let pos = method.outputs.iter().position(|&o| o == x)?;
                    Some(outputs.swap_remove(pos))
                })
                .filter(|value| value.as_ref().type_id() == context.cells[x].type_id)
                .map(Rc::from);
            cache.callbacks.insert(
                callback_id,
                CachedSeedCallback {
                    inputs: input_provenance,
                    value: value.clone(),
                },
            );
            value
        };
        if produced.is_some() {
            accumulated = produced;
        }
    }

    traversal.path_indices.remove(&x);
    traversal.path_cells.pop();
    if let Some(value) = accumulated {
        traversal.seeds.insert(x, value);
    }
    Ok(())
}

/// Returns the deterministic ordering key for one sibling seed fold.
///
/// Equal-primary siblings compare the selected method's ordered signature first, then
/// the relationship's full ordered method-signature sequence. Relationships that still
/// compare equal under this key are structurally indistinguishable to seedfill and are
/// therefore rejected rather than folded in adjacency order.
///
/// - Complexity: O(K) for K method inputs; signature metadata is borrowed.
fn seed_fold_order_key<'signature>(
    rel_id: RelationshipId,
    method_idx: usize,
    target: CellId,
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    signatures: &'signature SeedSignatures,
) -> SeedFoldOrderKey<'signature> {
    let relationship = &relationships[rel_id];
    let method = &relationship.methods[method_idx];
    let strongest_input = method
        .inputs
        .iter()
        .filter(|&&input| input != target)
        .map(|&input| cells[input].strength)
        .max()
        .unwrap_or(0);
    SeedFoldOrderKey {
        strongest_input,
        selected_method_signature: &signatures.methods[&(rel_id, method_idx)],
        relationship_signature: &signatures.relationships[&rel_id],
    }
}

/// Returns the selected-method component of a deterministic sibling-fold ordering key.
///
/// The key encodes the input and output arities, then the referenced input and output
/// cell IDs in their declared order, so equal-primary sibling folds can compare the
/// selected method before falling back to the whole relationship signature.
///
/// - Complexity: O(k) where k = `method.inputs.len() + method.outputs.len()`.
fn method_content_key(method: &crate::relationship::Method) -> Vec<u64> {
    let mut key = Vec::with_capacity(2 + method.inputs.len() + method.outputs.len());
    key.push(method.inputs.len() as u64);
    key.extend(method.inputs.iter().map(|&input| cell_sort_key(input)));
    key.push(method.outputs.len() as u64);
    key.extend(method.outputs.iter().map(|&output| cell_sort_key(output)));
    key
}

/// Returns a stable numeric sort key for a cell handle within one sheet.
///
/// `slotmap` keys carry both slot and generation; `as_ffi()` exposes that stable payload
/// as an opaque integer suitable for deterministic ordering without relying on debug
/// formatting or relationship insertion order.
fn cell_sort_key(cell: CellId) -> u64 {
    cell.data().as_ffi()
}

/// Returns `Error::SeedCycle` sites for a revisited path cell.
///
/// - Precondition: `cycle_start < path_cells.len()`.
/// - Precondition: `path_cells.len() == path_relationships.len() + 1`.
/// - Complexity: O(n) where n is the number of reported cycle members.
fn seed_cycle_sites(
    path_cells: &[CellId],
    path_relationships: &[RelationshipId],
    cycle_start: usize,
    closing_relationship: RelationshipId,
    current: CellId,
) -> Vec<ErrorSite> {
    debug_assert!(cycle_start < path_cells.len());
    debug_assert_eq!(path_cells.len(), path_relationships.len() + 1);
    debug_assert_eq!(path_cells.last().copied(), Some(current));

    let mut sites = Vec::with_capacity((path_cells.len() - cycle_start) * 2);
    sites.push(ErrorSite::Cell(path_cells[cycle_start]));
    for index in cycle_start..path_relationships.len() {
        sites.push(ErrorSite::Relationship(path_relationships[index]));
        sites.push(ErrorSite::Cell(path_cells[index + 1]));
    }
    sites.push(ErrorSite::Relationship(closing_relationship));
    sites
}

/// Selects the unique method that produces `target` without purely claiming another cell.
///
/// Returns `Ok(None)` when no method produces the target.
///
/// # Errors
///
/// Returns `Err(())` when target-producing methods have zero or multiple structurally
/// compatible survivors; preparation records the relationship for lazy conflict reporting.
///
/// - Complexity: O(M · K²) for M methods with K referenced cells.
fn select_seed_method(
    rel_id: RelationshipId,
    target: CellId,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
) -> Result<Option<usize>, ()> {
    let methods = &relationships[rel_id].methods;
    if !methods
        .iter()
        .any(|method| method.outputs.contains(&target))
    {
        return Ok(None);
    }
    let mut candidates = methods.iter().enumerate().filter_map(|(index, method)| {
        (method.outputs.contains(&target)
            && method
                .outputs
                .iter()
                .all(|output| *output == target || method.inputs.contains(output)))
        .then_some(index)
    });
    match (candidates.next(), candidates.next()) {
        (Some(index), None) => Ok(Some(index)),
        _ => Err(()),
    }
}

/// Replays the old complete elimination algorithm as a test-only selection oracle.
///
/// Selects `rel_id`'s unique `target`-producing seed method compatible with the
/// planner's elimination order.
///
/// Starts from every method in `rel_id` whose declared outputs contain `target`, then
/// replays elimination across the relationship's other referenced cells in
/// `elimination_order`. Eliminating a cell removes any candidate method that would still
/// purely claim that cell under the matching layer's semantics
/// ([`super::matching::pure_outputs`]). The
/// surviving method is therefore the one consistent with the planner's actual release
/// decisions this round, never simply the first declaration that mentions `target`.
///
/// Returns `Ok(None)` when `rel_id` has no `target`-producing method.
///
/// # Errors
///
/// - `Error::Conflict` — `rel_id` could seed `target`, but the elimination replay leaves
///   either no candidate method or more than one surviving candidate.
///
/// - Complexity: O(E · M · K²) where E = `rel_id`'s non-`target` referenced cells, M =
///   methods in the relationship, K = cells per method.
#[cfg(test)]
fn replay_seed_method(
    rel_id: RelationshipId,
    target: CellId,
    elimination_order: &[CellId],
    relationships: &SlotMap<RelationshipId, RelationshipData>,
) -> Result<Option<usize>, Error> {
    let rel = &relationships[rel_id];
    let mut candidates: Vec<usize> = rel
        .methods
        .iter()
        .enumerate()
        .filter_map(|(idx, method)| method.outputs.contains(&target).then_some(idx))
        .collect();
    if candidates.is_empty() {
        return Ok(None);
    }

    for &eliminated in elimination_order
        .iter()
        .filter(|&&cell| cell != target && rel.adj.contains(&cell))
    {
        candidates
            .retain(|&idx| !super::matching::pure_outputs(&rel.methods[idx]).contains(&eliminated));
        if candidates.is_empty() {
            return Err(Error::Conflict {
                sites: vec![ErrorSite::Relationship(rel_id)],
            });
        }
    }

    match candidates.as_slice() {
        [survivor] => Ok(Some(*survivor)),
        _ => Err(Error::Conflict {
            sites: vec![ErrorSite::Relationship(rel_id)],
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::TypeId;

    use crate::{ErrorSite, Method, Sheet};

    /// Compares structural selection with complete elimination replay for every cell permutation.
    ///
    /// - Complexity: O(n! · n · m · k²) for n cells, m methods, and k cells per method.
    fn assert_complete_replay(
        sheet: &Sheet,
        sibling: RelationshipId,
        claimant: RelationshipId,
        target: CellId,
        expected: Result<Option<usize>, ()>,
    ) {
        /// Checks each permutation by swapping the next cell into the prefix.
        ///
        /// - Precondition: `start <= order.len()`.
        /// - Complexity: O(n! · n · m · k²) for the remaining n cells.
        fn check(
            sheet: &Sheet,
            sibling: RelationshipId,
            claimant: RelationshipId,
            target: CellId,
            expected: Result<Option<usize>, ()>,
            order: &mut [CellId],
            start: usize,
        ) {
            debug_assert!(start <= order.len());
            if start == order.len() {
                assert_eq!(
                    replay_seed_method(sibling, target, order, &sheet.relationships)
                        .map_err(|_| ()),
                    expected
                );
                let assignment = [PlanStep::Method(sibling, 0), PlanStep::Method(claimant, 0)];
                let recipes = SeedRecipes::new(
                    &assignment,
                    &assignment[1..],
                    &sheet.cells,
                    &sheet.relationships,
                );
                let selected = match &recipes.cells[&target] {
                    SeedRecipe::Ready(siblings) => Ok(siblings
                        .iter()
                        .find(|entry| entry.relationship == sibling)
                        .map(|entry| entry.method_index)),
                    SeedRecipe::Conflict(relationship) => {
                        assert_eq!(*relationship, sibling);
                        Err(())
                    }
                };
                assert_eq!(selected, expected);
                return;
            }
            for next in start..order.len() {
                order.swap(start, next);
                check(sheet, sibling, claimant, target, expected, order, start + 1);
                order.swap(start, next);
            }
        }
        let mut order: Vec<_> = sheet.cells.keys().collect();
        check(sheet, sibling, claimant, target, expected, &mut order, 0);
    }

    /// Checks that preparation does not freeze source values across evaluations.
    #[test]
    fn prepared_recipes_read_current_sources() {
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(0_i32);
        let a = sheet.add_cell(1_i32);
        let sibling = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, x, |value: &i32| Ok(*value)),
                Method::from_fn_1_1(x, a, |value: &i32| Ok(*value)),
            ])
            .unwrap();
        let claimant = sheet
            .add_relationship(vec![Method::from_fn_1_1(x, x, |value: &i32| Ok(*value))])
            .unwrap();
        assert_complete_replay(&sheet, sibling, claimant, x, Ok(Some(0)));
        let order = [PlanStep::Method(sibling, 1), PlanStep::Method(claimant, 0)];
        let recipes = SeedRecipes::new(&order, &order, &sheet.cells, &sheet.relationships);
        let signatures = SeedSignatures::new(&sheet.relationships);
        for (input, expected) in [(3_i32, 3_i32), (8_i32, 8_i32)] {
            sheet.write(a, input).unwrap();
            let mut cache = SeedEvaluationCache::default();
            let seeds = evaluate_seeds(
                &recipes,
                &signatures,
                &sheet.cells,
                &sheet.relationships,
                &|id| SeedSource {
                    value: sheet.cells[id].source.as_ref(),
                    version: 0,
                },
                &mut cache,
            )
            .unwrap();
            assert_eq!(*seeds[&x].downcast_ref::<i32>().unwrap(), expected);
        }
    }

    /// Checks that an incident relationship without a target output supplies no seed.
    #[test]
    fn prepared_recipes_distinguish_absent_target() {
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(0_i32);
        let a = sheet.add_cell(1_i32);
        let sibling = sheet
            .add_relationship(vec![Method::from_fn_1_1(x, a, |value: &i32| Ok(*value))])
            .unwrap();
        let claimant = sheet
            .add_relationship(vec![Method::from_fn_1_1(x, x, |value: &i32| Ok(*value))])
            .unwrap();
        assert_complete_replay(&sheet, sibling, claimant, x, Ok(None));
    }

    /// Checks that zero structurally compatible survivors conflict only when evaluated.
    #[test]
    fn evaluate_seeds_reports_conflict_when_no_sibling_method_survives() {
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(0_i32);
        let a = sheet.add_cell(5_i32);
        let b = sheet.add_cell(7_i32);
        let i32_type = TypeId::of::<i32>();

        let sibling = sheet
            .add_relationship(vec![
                Method::new(
                    vec![b],
                    vec![x, a],
                    vec![i32_type],
                    vec![i32_type, i32_type],
                    |_| Ok(vec![Box::new(11_i32), Box::new(13_i32)]),
                ),
                Method::new(
                    vec![a],
                    vec![x, b],
                    vec![i32_type],
                    vec![i32_type, i32_type],
                    |_| Ok(vec![Box::new(17_i32), Box::new(19_i32)]),
                ),
                Method::new(
                    vec![x],
                    vec![a, b],
                    vec![i32_type],
                    vec![i32_type, i32_type],
                    |_| Ok(vec![Box::new(23_i32), Box::new(29_i32)]),
                ),
            ])
            .unwrap();
        let claimant = sheet
            .add_relationship(vec![Method::from_fn_1_1(x, x, |value: &i32| {
                Ok(value.checked_add(1).unwrap())
            })])
            .unwrap();

        assert_complete_replay(&sheet, sibling, claimant, x, Err(()));
        let order = [PlanStep::Method(sibling, 2), PlanStep::Method(claimant, 0)];
        let unused = SeedRecipes::new(&order, &[], &sheet.cells, &sheet.relationships);
        let signatures = SeedSignatures::new(&sheet.relationships);
        let mut cache = SeedEvaluationCache::default();
        assert!(
            evaluate_seeds(
                &unused,
                &signatures,
                &sheet.cells,
                &sheet.relationships,
                &|id| SeedSource {
                    value: sheet.cells[id].source.as_ref(),
                    version: 0
                },
                &mut cache,
            )
            .unwrap()
            .is_empty()
        );
        let err = evaluate_seeds_live(
            &[PlanStep::Method(sibling, 2), PlanStep::Method(claimant, 0)],
            &sheet.cells,
            &sheet.relationships,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            crate::Error::Conflict {
                sites
            } if sites == vec![ErrorSite::Relationship(sibling)]
        ));
    }

    /// Checks that multiple structurally compatible survivors remain ambiguous.
    #[test]
    fn evaluate_seeds_reports_conflict_when_multiple_sibling_methods_survive() {
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(0_i32);
        let a = sheet.add_cell(5_i32);
        let b = sheet.add_cell(7_i32);
        let i32_type = TypeId::of::<i32>();

        let sibling = sheet
            .add_relationship(vec![
                Method::new(
                    vec![x, a, b],
                    vec![x, a],
                    vec![i32_type, i32_type, i32_type],
                    vec![i32_type, i32_type],
                    |_| Ok(vec![Box::new(11_i32), Box::new(13_i32)]),
                ),
                Method::new(
                    vec![x, a, b],
                    vec![x, b],
                    vec![i32_type, i32_type, i32_type],
                    vec![i32_type, i32_type],
                    |_| Ok(vec![Box::new(17_i32), Box::new(19_i32)]),
                ),
            ])
            .unwrap();
        let claimant = sheet
            .add_relationship(vec![Method::from_fn_1_1(x, x, |value: &i32| {
                Ok(value.checked_add(1).unwrap())
            })])
            .unwrap();

        assert_complete_replay(&sheet, sibling, claimant, x, Err(()));
        let err = evaluate_seeds_live(
            &[PlanStep::Method(sibling, 0), PlanStep::Method(claimant, 0)],
            &sheet.cells,
            &sheet.relationships,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            crate::Error::Conflict {
                sites
            } if sites == vec![ErrorSite::Relationship(sibling)]
        ));
    }

    /// Checks that one propagation rejects a changed cross-phase sibling selection.
    #[test]
    fn changed_cached_seed_selection_reports_conflict() {
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(0_i32);
        let a = sheet.add_cell(1_i32);
        let b = sheet.add_cell(2_i32);

        let first_sibling = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, x, |value: &i32| Ok(value.checked_add(1).unwrap())),
                Method::from_fn_1_1(x, a, |value: &i32| Ok(*value)),
            ])
            .unwrap();
        let second_sibling = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(b, x, |value: &i32| Ok(value.checked_add(1).unwrap())),
                Method::from_fn_1_1(x, b, |value: &i32| Ok(*value)),
            ])
            .unwrap();
        let claimant = sheet
            .add_relationship(vec![Method::from_fn_1_1(x, x, |value: &i32| Ok(*value))])
            .unwrap();
        let claimant_step = PlanStep::Method(claimant, 0);
        let mut cache = SeedEvaluationCache::default();

        let first_order = [PlanStep::Method(first_sibling, 1), claimant_step];
        let signatures = SeedSignatures::new(&sheet.relationships);
        let first_recipes = SeedRecipes::new(
            &first_order,
            &[claimant_step],
            &sheet.cells,
            &sheet.relationships,
        );
        let seeds = evaluate_seeds(
            &first_recipes,
            &signatures,
            &sheet.cells,
            &sheet.relationships,
            &|id| SeedSource {
                value: sheet.cells[id].source.as_ref(),
                version: 0,
            },
            &mut cache,
        )
        .unwrap();
        assert_eq!(*seeds[&x].downcast_ref::<i32>().unwrap(), 2);

        let second_recipes = SeedRecipes::new(
            &[
                PlanStep::Method(first_sibling, 1),
                PlanStep::Method(second_sibling, 1),
                claimant_step,
            ],
            &[claimant_step],
            &sheet.cells,
            &sheet.relationships,
        );
        let error = evaluate_seeds(
            &second_recipes,
            &signatures,
            &sheet.cells,
            &sheet.relationships,
            &|id| SeedSource {
                value: sheet.cells[id].source.as_ref(),
                version: 0,
            },
            &mut cache,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            Error::Conflict { sites }
                if sites.contains(&ErrorSite::Cell(x))
                    && sites.contains(&ErrorSite::Relationship(first_sibling))
                    && sites.contains(&ErrorSite::Relationship(second_sibling))
        ));
    }

    /// Checks that equal-strength fold results are independent of sibling insertion order.
    #[test]
    fn evaluate_seeds_breaks_equal_strength_ties_by_ordered_signature() {
        /// Constructs a fold that returns the maximum of its first two inputs.
        fn ordered_max(
            first: CellId,
            second: CellId,
            third: CellId,
            strongest: CellId,
            output: CellId,
        ) -> Method {
            Method::new(
                vec![first, second, third, strongest],
                vec![output],
                vec![
                    TypeId::of::<i32>(),
                    TypeId::of::<i32>(),
                    TypeId::of::<i32>(),
                    TypeId::of::<i32>(),
                ],
                vec![TypeId::of::<i32>()],
                |args| {
                    let lhs = args[0]
                        .downcast_ref::<i32>()
                        .expect("type checked at add_relationship");
                    let rhs = args[1]
                        .downcast_ref::<i32>()
                        .expect("type checked at add_relationship");
                    Ok(vec![Box::new((*lhs).max(*rhs))])
                },
            )
        }

        /// Constructs a fold that returns the minimum of its first two inputs.
        fn ordered_min(
            first: CellId,
            second: CellId,
            third: CellId,
            strongest: CellId,
            output: CellId,
        ) -> Method {
            Method::new(
                vec![first, second, third, strongest],
                vec![output],
                vec![
                    TypeId::of::<i32>(),
                    TypeId::of::<i32>(),
                    TypeId::of::<i32>(),
                    TypeId::of::<i32>(),
                ],
                vec![TypeId::of::<i32>()],
                |args| {
                    let lhs = args[0]
                        .downcast_ref::<i32>()
                        .expect("type checked at add_relationship");
                    let rhs = args[1]
                        .downcast_ref::<i32>()
                        .expect("type checked at add_relationship");
                    Ok(vec![Box::new((*lhs).min(*rhs))])
                },
            )
        }

        /// Constructs equal-strength siblings in either insertion order.
        fn build_sheet(
            reverse_siblings: bool,
        ) -> (
            Sheet,
            RelationshipId,
            RelationshipId,
            RelationshipId,
            CellId,
        ) {
            let mut sheet = Sheet::new();
            let x = sheet.add_cell(4_i32);
            let a = sheet.add_cell(5_i32);
            let b = sheet.add_cell(3_i32);
            let shared_strongest = sheet.add_cell(0_i32);

            let a_le_x = vec![
                ordered_min(a, x, b, shared_strongest, a),
                ordered_max(a, x, b, shared_strongest, x),
            ];
            let x_le_b = vec![
                ordered_min(b, x, a, shared_strongest, x),
                ordered_max(b, x, a, shared_strongest, b),
            ];
            let claimant = sheet
                .add_relationship(vec![Method::from_fn_1_1(x, x, |value: &i32| Ok(*value))])
                .unwrap();
            let (a_le_x_rel, x_le_b_rel) = if reverse_siblings {
                let x_le_b_rel = sheet.add_relationship(x_le_b).unwrap();
                let a_le_x_rel = sheet.add_relationship(a_le_x).unwrap();
                (a_le_x_rel, x_le_b_rel)
            } else {
                let a_le_x_rel = sheet.add_relationship(a_le_x).unwrap();
                let x_le_b_rel = sheet.add_relationship(x_le_b).unwrap();
                (a_le_x_rel, x_le_b_rel)
            };

            (sheet, a_le_x_rel, x_le_b_rel, claimant, x)
        }

        let (forward_sheet, forward_up, forward_down, forward_claimant, forward_x) =
            build_sheet(false);
        let forward_seeds = evaluate_seeds_live(
            &[
                PlanStep::Method(forward_up, 0),
                PlanStep::Method(forward_down, 1),
                PlanStep::Method(forward_claimant, 0),
            ],
            &forward_sheet.cells,
            &forward_sheet.relationships,
        )
        .expect("forward evaluation should succeed");

        let (reversed_sheet, reversed_up, reversed_down, reversed_claimant, reversed_x) =
            build_sheet(true);
        let reversed_seeds = evaluate_seeds_live(
            &[
                PlanStep::Method(reversed_up, 0),
                PlanStep::Method(reversed_down, 1),
                PlanStep::Method(reversed_claimant, 0),
            ],
            &reversed_sheet.cells,
            &reversed_sheet.relationships,
        )
        .expect("reversed evaluation should succeed");

        let forward_seed = forward_seeds[&forward_x]
            .downcast_ref::<i32>()
            .expect("seed should stay typed as i32");
        let reversed_seed = reversed_seeds[&reversed_x]
            .downcast_ref::<i32>()
            .expect("seed should stay typed as i32");
        assert_eq!(*forward_seed, 3);
        assert_eq!(*reversed_seed, 3);
    }

    /// Checks that full ordered relationship signatures break selected-signature ties.
    #[test]
    fn seed_fold_order_key_uses_full_relationship_structure_after_selected_signature() {
        /// Constructs the shared selected-method signature.
        fn selected_method(x: CellId, a: CellId, b: CellId) -> Method {
            Method::new(
                vec![x, a],
                vec![x, b],
                vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
                vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
                |_| Ok(vec![Box::new(0_i32), Box::new(0_i32)]),
            )
        }

        /// Constructs an inert method preserving its declared input and output order.
        ///
        /// - Complexity: O(K) for K referenced cells.
        fn inert_method(inputs: Vec<CellId>, outputs: Vec<CellId>) -> Method {
            let output_len = outputs.len();
            let input_len = inputs.len();
            Method::new(
                inputs,
                outputs,
                vec![TypeId::of::<i32>(); input_len],
                vec![TypeId::of::<i32>(); output_len],
                move |_| {
                    Ok((0..output_len)
                        .map(|_| Box::new(0_i32) as Box<dyn Any>)
                        .collect())
                },
            )
        }

        let mut sheet = Sheet::new();
        let x = sheet.add_cell(1_i32);
        let a = sheet.add_cell(9_i32);
        let b = sheet.add_cell(0_i32);

        let first = sheet
            .add_relationship(vec![
                selected_method(x, a, b),
                inert_method(vec![x, b], vec![a, b]),
                inert_method(vec![b], vec![a, x]),
            ])
            .unwrap();
        let second = sheet
            .add_relationship(vec![
                selected_method(x, a, b),
                inert_method(vec![b], vec![a, x]),
                inert_method(vec![x, b], vec![a, b]),
            ])
            .unwrap();

        let signatures = SeedSignatures::new(&sheet.relationships);
        let first_key =
            seed_fold_order_key(first, 0, x, &sheet.cells, &sheet.relationships, &signatures);
        let second_key = seed_fold_order_key(
            second,
            0,
            x,
            &sheet.cells,
            &sheet.relationships,
            &signatures,
        );

        assert_eq!(first_key.strongest_input, second_key.strongest_input);
        assert_eq!(
            first_key.selected_method_signature,
            second_key.selected_method_signature
        );
        assert_ne!(
            first_key.relationship_signature,
            second_key.relationship_signature
        );
        assert_ne!(first_key, second_key);
    }

    /// Checks that indistinguishable sibling fold keys conflict before callback evaluation.
    #[test]
    fn evaluate_seeds_rejects_structurally_identical_equal_primary_siblings() {
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(1_i32);
        let a = sheet.add_cell(9_i32);

        let first = sheet
            .add_relationship(vec![Method::from_fn_2_1(
                [x, a],
                x,
                |value: &i32, _: &i32| Ok(value.checked_add(1).unwrap()),
            )])
            .unwrap();
        let second = sheet
            .add_relationship(vec![Method::from_fn_2_1(
                [x, a],
                x,
                |value: &i32, _: &i32| Ok(value.checked_mul(2).unwrap()),
            )])
            .unwrap();
        let claimant = sheet
            .add_relationship(vec![Method::from_fn_1_1(x, x, |value: &i32| Ok(*value))])
            .unwrap();

        let err = evaluate_seeds_live(
            &[
                PlanStep::Method(first, 0),
                PlanStep::Method(second, 0),
                PlanStep::Method(claimant, 0),
            ],
            &sheet.cells,
            &sheet.relationships,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            crate::Error::Conflict { sites }
                if sites
                    == vec![
                        ErrorSite::Relationship(first),
                        ErrorSite::Relationship(second),
                    ]
        ));
    }
}
