//! The [`Sheet`] owns and manages a property model constraint graph.
//!
//! All cells and relationships are created through the sheet and are
//! destroyed when the sheet is dropped.

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use slotmap::SlotMap;

use crate::{
    cell::{CellData, CellId, CellKind},
    conditional::{Branch, ConditionalData, ConditionalId, MatchExpr, MatchSource},
    error::{Error, ErrorSite},
    filter::{Filter, FilterKind, FilterViolation},
    planner::{Plan, PlanStep, SeedSignatures, SeedSource, Seeds},
    relationship::{Method, RelationshipData, RelationshipId},
    requirement::{Requirement, RequirementData, RequirementId},
};

mod cached;
mod dependency;
mod prerequisites;

#[cfg(test)]
use cached::ReuseStats;
use cached::{CachedPlan, PreparedPlan};

/// Owns a complete property model constraint graph.
///
/// Create cells with [`Sheet::add_cell`], define multi-way constraints with
/// [`Sheet::add_relationship`], write input values with [`Sheet::write`],
/// then call [`Sheet::propagate`] to execute the planning pass and update
/// derived cells.
///
/// # Example
///
/// ```rust
/// use adam_rs::{Sheet, Method};
///
/// let mut sheet = Sheet::new();
/// let a = sheet.add_cell(0_i32);
/// let b = sheet.add_cell(0_i32);
/// sheet.add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))]).unwrap();
/// sheet.write(a, 3_i32).unwrap();
/// assert_eq!(*sheet.read::<i32>(a).unwrap(), 3);
/// ```
pub struct Sheet {
    pub(crate) cells: SlotMap<CellId, CellData>,
    pub(crate) relationships: SlotMap<RelationshipId, RelationshipData>,
    pub(crate) changed_cells: Vec<CellId>,
    /// Monotonic counter incremented by both `add_cell` and `write`; cells added
    /// later and cells written later have strictly higher strength, making the
    /// default method-selection direction deterministic.
    next_strength: u64,
    /// The prepared snapshot from the last successful `propagate()` call, retained so the
    /// display-only accessors ([`Sheet::is_source`], [`Sheet::selected_method`],
    /// [`Sheet::is_forced`]) can report which method the planner picked for each
    /// relationship without re-planning.
    last_plan: Option<Rc<PreparedPlan>>,
    /// Eligible prepared unconditional plan, independent of the display snapshot.
    pre_plan_cache: Option<CachedPlan>,
    /// Eligible prepared final active plan, independent of the unconditional plan.
    main_plan_cache: Option<CachedPlan>,
    /// Ordered method signatures retained until the graph structure changes.
    seed_signatures: Option<Rc<SeedSignatures>>,
    /// Counts actual preparation and reuse-validation work for this sheet only.
    #[cfg(test)]
    reuse_stats: ReuseStats,
    /// All conditionals registered on this sheet.
    pub(crate) conditionals: SlotMap<ConditionalId, ConditionalData>,
    /// Union of all RelationshipIds assigned to any conditional branch or default.
    /// Used to exclude them from the unconditional active set.
    pub(crate) conditional_relationships: HashSet<RelationshipId>,
    /// All requirements registered on this sheet, across all cells.
    requirements: SlotMap<RequirementId, RequirementData>,
    /// Requirements that evaluated `false` as of the last `propagate()` call, grouped
    /// by cell. Sparse: a cell with no entry had all its requirements hold.
    last_requirement_violations: HashMap<CellId, Vec<RequirementId>>,
    /// Filter violations recorded against a derived value as of the last `propagate()`
    /// call.
    last_filter_violations: HashMap<CellId, FilterViolation>,
    /// Reverse index of `filter_args`: for each cell, the live cells whose filter
    /// references it as one of its dynamic arguments. Built incrementally in
    /// `add_filter`; cells and filters are never removed once added, so this needs no
    /// invalidation, matching every other per-cell set/map `Sheet` already maintains
    /// for its own lifetime.
    filter_dependents: HashMap<CellId, Vec<CellId>>,
    /// Whether static guard independence has been validated for the current structure.
    guard_independent: bool,
}

/// A conditional's evaluated match value: borrowed (existing cell, no allocation) or owned
/// (freshly computed by a [`MatchExpr`] function).
enum MatchValue<'a> {
    Ref(&'a dyn Any),
    Owned(Box<dyn Any>),
}

impl MatchValue<'_> {
    /// Returns the contained value as a type-erased reference, regardless of variant.
    fn as_dyn(&self) -> &dyn Any {
        match self {
            MatchValue::Ref(r) => *r,
            MatchValue::Owned(b) => b.as_ref(),
        }
    }
}

/// Provenance needed to decide whether a staged guard-cone step can be reused.
///
/// Reuse is intentionally conservative: a step is reusable only when the final plan selects the
/// same method, producer path, source/derived classification, and seed inputs. A mismatch returns
/// `Error::Conflict` rather than guessing a compatible assignment or reusing stale values.
#[derive(Clone, PartialEq, Eq)]
struct StepProvenance {
    input_producers: Vec<Option<PlanStep>>,
    seed_inputs: Vec<CellId>,
    source_producer: Option<PlanStep>,
    output_classes: Vec<(CellId, OutputClassification)>,
}

/// Whether a selected step writes a cell's staged value as source or derived.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputClassification {
    /// The write replaces the staged source value.
    Source,
    /// The write shadows the staged source value.
    Derived,
}

/// Selected producer paths for every step and output in one execution plan.
#[derive(Default)]
struct PlanProvenance {
    steps: HashMap<PlanStep, StepProvenance>,
    output_producers: HashMap<CellId, PlanStep>,
}

/// Transactional propagation state whose writes remain private until commit.
#[derive(Default)]
struct PropagationStage {
    sources: HashMap<CellId, Box<dyn Any>>,
    /// Version of each staged source value; an absent cell reads its live source (version 0).
    source_versions: HashMap<CellId, u64>,
    last_source_version: u64,
    derived: HashMap<CellId, Box<dyn Any>>,
    executed_methods: HashSet<(RelationshipId, usize)>,
    executed_filters: HashSet<CellId>,
    step_provenance: HashMap<PlanStep, StepProvenance>,
    changed: Vec<CellId>,
    changed_set: HashSet<CellId>,
}

impl PropagationStage {
    /// Returns the staged effective value for `id`, ignoring any live sheet `derived`.
    fn effective<'a>(&'a self, cells: &'a SlotMap<CellId, CellData>, id: CellId) -> &'a dyn Any {
        self.derived
            .get(&id)
            .map(|value| value.as_ref())
            .or_else(|| self.sources.get(&id).map(|value| value.as_ref()))
            .unwrap_or_else(|| cells[id].source.as_ref())
    }

    /// Returns the staged source value for `id`, ignoring any live sheet `derived`.
    fn source<'a>(&'a self, cells: &'a SlotMap<CellId, CellData>, id: CellId) -> &'a dyn Any {
        self.sources
            .get(&id)
            .map(|value| value.as_ref())
            .unwrap_or_else(|| cells[id].source.as_ref())
    }

    /// Returns the staged source value for `id` with a version identifying that value.
    ///
    /// - Postcondition: two calls return equal versions only if no staged source write or
    ///   reclassification changed `id`'s source value between them.
    fn seed_source<'a>(
        &'a self,
        cells: &'a SlotMap<CellId, CellData>,
        id: CellId,
    ) -> SeedSource<'a> {
        SeedSource {
            value: self.source(cells, id),
            version: self.source_versions.get(&id).copied().unwrap_or(0),
        }
    }

    /// Records a new staged source value for `id` under a fresh version.
    fn insert_source(&mut self, id: CellId, value: Box<dyn Any>) {
        self.last_source_version += 1;
        self.source_versions.insert(id, self.last_source_version);
        self.sources.insert(id, value);
    }

    /// Applies a staged write using the same shadow/non-shadow rule as `execute_plan`.
    fn write(&mut self, id: CellId, value: Box<dyn Any>, shadow: bool) {
        if shadow {
            self.derived.insert(id, value);
        } else {
            self.insert_source(id, value);
            self.derived.remove(&id);
        }
        if self.changed_set.insert(id) {
            self.changed.push(id);
        }
    }

    /// Reclassifies an existing staged output for the general plan's shadowing rule.
    ///
    /// - Precondition: `id` has a staged source or derived value.
    fn reclassify(&mut self, id: CellId, shadow: bool) {
        if shadow {
            if let Some(value) = self.sources.remove(&id) {
                self.source_versions.remove(&id);
                self.derived.insert(id, value);
            }
        } else if let Some(value) = self.derived.remove(&id) {
            self.insert_source(id, value);
        }
    }
}

impl Sheet {
    /// Creates an empty sheet with no cells or relationships.
    pub fn new() -> Self {
        Sheet {
            cells: SlotMap::with_key(),
            relationships: SlotMap::with_key(),
            changed_cells: Vec::new(),
            next_strength: 0,
            last_plan: None,
            pre_plan_cache: None,
            main_plan_cache: None,
            seed_signatures: None,
            #[cfg(test)]
            reuse_stats: ReuseStats::default(),
            conditionals: SlotMap::with_key(),
            conditional_relationships: HashSet::new(),
            requirements: SlotMap::with_key(),
            last_requirement_violations: HashMap::new(),
            last_filter_violations: HashMap::new(),
            filter_dependents: HashMap::new(),
            guard_independent: true,
        }
    }

    /// Registers a cell with an initial value and returns a stable handle.
    ///
    /// The cell's `TypeId` is fixed at creation time; subsequent `write` and
    /// `read` calls that use a different type will return `Error::TypeMismatch`.
    ///
    /// Each call increments the sheet's internal strength counter and sets bit 63
    /// of the result. This partitions the strength space: written/added cells always
    /// have higher strength than derived cells, ensuring stability across conditional
    /// branch switches.
    pub fn add_cell<T: Any + PartialEq + 'static>(&mut self, value: T) -> CellId {
        self.invalidate_prepared_plans();
        self.next_strength += 1;
        let strength = self.next_strength | (1u64 << 63);
        self.cells.insert(CellData {
            source: Box::new(value),
            derived: None,
            type_id: TypeId::of::<T>(),
            strength,
            changed: false,
            adj: Vec::new(),
            eq_fn: |a, b| a.downcast_ref::<T>() == b.downcast_ref::<T>(),
            filter: None,
            kind: CellKind::Cell,
            requirements: Vec::new(),
        })
    }

    /// Registers a cell that can never be claimed as any method's output — always a
    /// planner source, forever.
    ///
    /// - Complexity: O(1).
    pub fn add_source<T: Any + PartialEq + 'static>(&mut self, value: T) -> CellId {
        let id = self.add_cell(value);
        self.cells[id].kind = CellKind::Source;
        id
    }

    /// Returns `id`'s fixed cell kind.
    ///
    /// Returns `None` if `id` is not a live cell in this sheet.
    pub fn cell_kind(&self, id: CellId) -> Option<CellKind> {
        self.cells.get(id).map(|c| c.kind)
    }

    /// Registers a relationship defined by a non-empty list of methods.
    ///
    /// All methods are validated: their declared `TypeId`s must match the
    /// registered cells, and each method must have at least one output. A method
    /// with no inputs is explicitly allowed: it defines a fixed point (a constant,
    /// independent of every other cell) rather than a derivation.
    /// On success the `RelationshipId` is added to each adjacent cell's adjacency list.
    ///
    /// A cell that appears in both a method's inputs and its outputs is a self-referencing
    /// cell and is explicitly allowed.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidMethod` — `methods` is empty, or a method has no outputs.
    /// - `Error::MismatchedMethodCells` — some method's `inputs ∪ outputs` differs
    ///   from another method's in the same relationship.
    /// - `Error::InvalidMethodOutputs` — a method's own `outputs` list names a cell
    ///   more than once, or two methods in the same relationship have identical or
    ///   nested `outputs` sets. Overlapping non-nested output sets are allowed.
    /// - `Error::InvalidId` — a `CellId` in any method is not found in this sheet.
    /// - `Error::TypeMismatch` — a method's declared `TypeId` does not match the
    ///   cell's registered `TypeId`.
    /// - `Error::InvalidCellKind` — a method's output cell is `Source`-kind.
    ///
    /// Guard independence is checked by [`Sheet::validate`] and by [`Sheet::propagate`],
    /// not by this mutator.
    ///
    /// - Complexity: O(m² × c) where m is the total number of methods and c is the
    ///   maximum number of cells per method, due to pairwise output-set comparison.
    pub fn add_relationship(&mut self, methods: Vec<Method>) -> Result<RelationshipId, Error> {
        if methods.is_empty() {
            return Err(Error::InvalidMethod { sites: vec![] });
        }

        for (idx, method) in methods.iter().enumerate() {
            if method.outputs.is_empty() {
                return Err(Error::InvalidMethod {
                    sites: vec![ErrorSite::MethodIndex(idx)],
                });
            }

            // declared type counts must match cell-id counts
            if method.inputs.len() != method.input_types.len()
                || method.outputs.len() != method.output_types.len()
            {
                return Err(Error::InvalidMethod {
                    sites: vec![ErrorSite::MethodIndex(idx)],
                });
            }

            for (&cell_id, &declared) in method.inputs.iter().zip(method.input_types.iter()) {
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                        sites: vec![ErrorSite::MethodIndex(idx)],
                    });
                }
            }

            for (&cell_id, &declared) in method.outputs.iter().zip(method.output_types.iter()) {
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.kind == CellKind::Source {
                    return Err(Error::InvalidCellKind {
                        sites: vec![ErrorSite::MethodIndex(idx), ErrorSite::Cell(cell_id)],
                    });
                }
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                        sites: vec![ErrorSite::MethodIndex(idx)],
                    });
                }
            }
        }

        // Every method in a relationship must reference the same set of cells: the
        // union of a method's inputs and outputs (as a set) must be identical across
        // all methods. A relationship models a fixed set of related cells; methods
        // differ only in which subset of that set they treat as outputs (using the
        // "ignore an input" pattern), not in which cells they reference at all.
        let cell_sets: Vec<HashSet<CellId>> = methods
            .iter()
            .map(|m| m.inputs.iter().chain(m.outputs.iter()).copied().collect())
            .collect();
        if let Some(rel_idx) = cell_sets[1..].iter().position(|set| set != &cell_sets[0]) {
            let diverging = rel_idx + 1;
            let mut sites = vec![ErrorSite::MethodIndex(diverging), ErrorSite::MethodIndex(0)];
            // Symmetric difference of the two cell sets, in a stable order.
            for &c in cell_sets[diverging].symmetric_difference(&cell_sets[0]) {
                sites.push(ErrorSite::Cell(c));
            }
            return Err(Error::MismatchedMethodCells { sites });
        }

        // A method's own outputs must be duplicate-free, and no two methods in a
        // relationship may claim identical or nested output sets: the planner's
        // matching stage treats a method's pure-output set as an indivisible claim,
        // so nested claims would make that claim ambiguous while still allowing
        // overlapping non-nested sets.
        let mut seen_output_sets: Vec<(usize, HashSet<CellId>, &[CellId])> =
            Vec::with_capacity(methods.len());
        for (idx, method) in methods.iter().enumerate() {
            let output_set: HashSet<CellId> = method.outputs.iter().copied().collect();
            if output_set.len() != method.outputs.len() {
                // A cell repeated within this method's own outputs. Report each
                // distinct repeated cell once, no matter how many times it repeats.
                let mut sites = vec![ErrorSite::MethodIndex(idx)];
                let mut seen = HashSet::new();
                let mut reported = HashSet::new();
                for &o in &method.outputs {
                    if !seen.insert(o) && reported.insert(o) {
                        sites.push(ErrorSite::Cell(o));
                    }
                }
                return Err(Error::InvalidMethodOutputs { sites });
            }
            for (earlier, earlier_set, earlier_outputs) in &seen_output_sets {
                if output_set == *earlier_set
                    || (output_set.len() < earlier_set.len() && output_set.is_subset(earlier_set))
                    || (earlier_set.len() < output_set.len() && earlier_set.is_subset(&output_set))
                {
                    let mut sites = vec![
                        ErrorSite::MethodIndex(idx),
                        ErrorSite::MethodIndex(*earlier),
                    ];
                    let offending_outputs: &[CellId] = if output_set.len() <= earlier_set.len() {
                        &method.outputs
                    } else {
                        earlier_outputs
                    };
                    for &o in offending_outputs {
                        sites.push(ErrorSite::Cell(o));
                    }
                    return Err(Error::InvalidMethodOutputs { sites });
                }
            }
            seen_output_sets.push((idx, output_set, &method.outputs));
        }

        // Collect the union of all adjacent cells in insertion order, deduplicated.
        let mut adj: Vec<CellId> = Vec::new();
        let mut seen: std::collections::HashSet<CellId> = std::collections::HashSet::new();
        for method in &methods {
            for &cell_id in method.inputs.iter().chain(method.outputs.iter()) {
                if seen.insert(cell_id) {
                    adj.push(cell_id);
                }
            }
        }

        self.invalidate_prepared_plans();
        let rel_id = self.relationships.insert(RelationshipData {
            methods,
            adj: adj.clone(),
        });

        for cell_id in adj {
            if let Some(cell) = self.cells.get_mut(cell_id)
                && !cell.adj.contains(&rel_id)
            {
                cell.adj.push(rel_id);
            }
        }

        self.guard_independent = false;
        Ok(rel_id)
    }

    /// Registers a conditional that activates relationships based on the value of a match
    /// subject: either a single existing cell, or a [`MatchExpr`] computed from multiple
    /// input cells.
    ///
    /// Each element of `branches` is `(keys, relationships)`: when the match subject's
    /// value equals any key in `keys`, the branch's `relationships` are added to the active
    /// set for `propagate`. Branches are evaluated in definition order; first match wins.
    /// `default` holds relationships activated when no branch matches; pass an empty `Vec`
    /// for no default.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — the match subject references a cell not in this sheet.
    /// - `Error::TypeMismatch` — (expression match subject only) an input cell's registered
    ///   type doesn't match the expression's declared type for that input.
    /// - `Error::InvalidConditional` — the match subject's output type does not match `T`;
    ///   a referenced relationship does not exist; a relationship already appears in
    ///   another conditional branch; or a branch has no keys.
    ///
    /// Guard independence is checked by [`Sheet::validate`] and by [`Sheet::propagate`],
    /// not by this mutator.
    ///
    /// - Complexity: O(B·(K + R)) where B = branches, K = keys per branch, R =
    ///   relationships per branch.
    pub fn add_conditional<T: Any + PartialEq + 'static>(
        &mut self,
        source: MatchExpr,
        branches: Vec<(Vec<T>, Vec<RelationshipId>)>,
        default: Vec<RelationshipId>,
    ) -> Result<ConditionalId, Error> {
        match &source.0 {
            MatchSource::Cell(cell) => {
                let cell_data = self.cells.get(*cell).ok_or(Error::InvalidId)?;
                if cell_data.type_id != TypeId::of::<T>() {
                    return Err(Error::InvalidConditional {
                        sites: vec![ErrorSite::Cell(*cell)],
                    });
                }
            }
            MatchSource::Expr(expr) => {
                if expr.output_type != TypeId::of::<T>() {
                    return Err(Error::InvalidConditional { sites: vec![] });
                }
                for (&cell_id, &declared) in expr.inputs.iter().zip(expr.input_types.iter()) {
                    let cell_data = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                    if cell_data.type_id != declared {
                        return Err(Error::TypeMismatch {
                            expected: cell_data.type_id,
                            found: declared,
                            sites: vec![],
                        });
                    }
                }
            }
        };

        // Collect and validate all relationship IDs (branches + default).
        let all_rels: Vec<RelationshipId> = branches
            .iter()
            .flat_map(|(_, rels)| rels.iter().copied())
            .chain(default.iter().copied())
            .collect();

        for &rel_id in &all_rels {
            if !self.relationships.contains_key(rel_id) {
                return Err(Error::InvalidConditional { sites: vec![] });
            }
            if self.conditional_relationships.contains(&rel_id) {
                return Err(Error::InvalidConditional {
                    sites: vec![ErrorSite::Relationship(rel_id)],
                });
            }
        }

        // Validate branch keys are non-empty.
        for (keys, _) in &branches {
            if keys.is_empty() {
                return Err(Error::InvalidConditional { sites: vec![] });
            }
        }

        // Check for duplicate relationship IDs within this call.
        let mut seen: HashSet<RelationshipId> = HashSet::new();
        for &rel_id in &all_rels {
            if !seen.insert(rel_id) {
                return Err(Error::InvalidConditional {
                    sites: vec![ErrorSite::Relationship(rel_id)],
                });
            }
        }

        // Type-erase branch keys.
        let typed_branches: Vec<Branch> = branches
            .into_iter()
            .map(|(keys, relationships)| Branch {
                keys: keys
                    .into_iter()
                    .map(|k| Box::new(k) as Box<dyn Any>)
                    .collect(),
                relationships,
            })
            .collect();

        // Record all relationships as conditional so they are excluded from the
        // unconditional active set in propagate().
        self.invalidate_prepared_plans();
        for &rel_id in &all_rels {
            self.conditional_relationships.insert(rel_id);
        }

        let id = self.conditionals.insert(ConditionalData {
            source: source.0,
            branches: typed_branches,
            default,
        });

        self.guard_independent = false;
        Ok(id)
    }

    /// Returns `true` if `id` is already claimed as some existing method's output —
    /// i.e. it cannot legally become an `out` cell's writer target, since that would
    /// leave two producers claiming the same cell.
    fn cell_has_prior_use(&self, id: CellId) -> bool {
        self.relationships
            .values()
            .any(|rel| rel.methods.iter().any(|m| m.outputs.contains(&id)))
    }

    /// Attaches a named requirement to `cell`. `requirement.inputs` may be any cells in
    /// the sheet, not only `cell` itself. For a `Cell`/`Source` kind `cell`, also
    /// evaluates `requirement` immediately against current effective values — its
    /// value is already authoritative. Skipped for an `Out`-kind `cell`: its value
    /// isn't authoritative until its writer next executes, so attachment always
    /// succeeds structurally there, deferring to the first post-`propagate()`
    /// diagnostic to report an initial violation if there is one.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `cell`, or one of `requirement`'s input cells, is not a
    ///   live cell in this sheet.
    /// - `Error::TypeMismatch` — an input's declared type does not match its cell's
    ///   registered type.
    /// - `Error::InvalidRequirement` — `name` is `Some` and `cell` already has a
    ///   requirement with that same name, or (`Cell`/`Source` kind only) evaluating
    ///   `requirement` against the referenced cells' current effective values
    ///   returns `Ok(false)`.
    /// - `Error::MethodFailed` — (`Cell`/`Source` kind only) evaluating `requirement`
    ///   against current values returns `Err`.
    ///
    /// - Complexity: O(k) where k is `requirement`'s input count.
    pub fn add_requirement(
        &mut self,
        cell: CellId,
        name: Option<&str>,
        requirement: Requirement,
    ) -> Result<RequirementId, Error> {
        let cell_data = self.cells.get(cell).ok_or(Error::InvalidId)?;
        if let Some(name) = name
            && cell_data
                .requirements
                .iter()
                .any(|&rid| self.requirements[rid].name.as_deref() == Some(name))
        {
            return Err(Error::InvalidRequirement);
        }
        if requirement.inputs.len() != requirement.input_types.len() {
            return Err(Error::InvalidRequirement);
        }
        for (&input_id, &declared) in requirement
            .inputs
            .iter()
            .zip(requirement.input_types.iter())
        {
            let input_cell = self.cells.get(input_id).ok_or(Error::InvalidId)?;
            if input_cell.type_id != declared {
                return Err(Error::TypeMismatch {
                    expected: input_cell.type_id,
                    found: declared,
                    sites: vec![],
                });
            }
        }

        if self.cells[cell].kind != CellKind::Out {
            let inputs: Vec<&dyn Any> = requirement
                .inputs
                .iter()
                .map(|&id| self.cells[id].effective())
                .collect();
            let holds = (requirement.function)(&inputs).map_err(|error| Error::MethodFailed {
                error,
                sites: vec![],
            })?;
            if !holds {
                return Err(Error::InvalidRequirement);
            }
        }

        self.invalidate_prepared_plans();
        let rid = self.requirements.insert(RequirementData {
            name: name.map(str::to_string),
            cell,
            inputs: requirement.inputs,
            function: requirement.function,
        });
        self.cells[cell].requirements.push(rid);
        Ok(rid)
    }

    /// Registers `writer` as the sole producer of its one output cell, which becomes
    /// an `out` cell: always derived by `writer`, never `write()`-able, but otherwise
    /// an ordinary, freely-referenceable cell. `requirements` are attached to that
    /// cell one at a time, in order, via [`Sheet::add_requirement`].
    ///
    /// # Errors
    ///
    /// - `Error::InvalidOutput` — `writer` does not have exactly one output cell.
    /// - `Error::InvalidCellKind` — the writer's output cell is already `Source` or
    ///   `Out` kind, or already claimed as some existing method's output.
    /// - Any error [`Sheet::add_relationship`] or [`Sheet::add_requirement`] can
    ///   return.
    ///
    /// - Postcondition: not atomic — if `add_requirement` fails partway through
    ///   `requirements`, the output cell is left `Out`-kind, `writer` is already
    ///   registered as its producer, and every requirement before the failing one is
    ///   already attached; none of this is rolled back on error.
    ///
    /// - Complexity: O(k + m²×c) where k is the number of requirements, plus the
    ///   cost of `add_relationship` for `writer` alone (m = 1 method, c = cells in
    ///   that method).
    pub fn add_out(
        &mut self,
        writer: Method,
        requirements: Vec<(Option<&str>, Requirement)>,
    ) -> Result<CellId, Error> {
        if writer.outputs.len() != 1 {
            return Err(Error::InvalidOutput);
        }
        let out_cell = writer.outputs[0];

        let kind = self.cells.get(out_cell).ok_or(Error::InvalidId)?.kind;
        if kind != CellKind::Cell || self.cell_has_prior_use(out_cell) {
            return Err(Error::InvalidCellKind { sites: vec![] });
        }

        self.add_relationship(vec![writer])?;
        self.cells[out_cell].kind = CellKind::Out;

        for (name, requirement) in requirements {
            self.add_requirement(out_cell, name, requirement)?;
        }

        Ok(out_cell)
    }

    /// Attaches `filter` to `cell`.
    ///
    /// Never evaluates `filter`'s function — attaching a filter is not a fresh
    /// external input, so it never changes `cell`'s current effective value. The next
    /// full [`Sheet::propagate`] call conforms `cell` via the planner's
    /// `PlanStep::FilterReclamp` step; until then, `read()` reflects whatever `cell`
    /// held before this call.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `cell`, or one of `filter`'s argument cells, is not a
    ///   live cell in this sheet.
    /// - `Error::InvalidFilter` — `cell` already has a filter, `filter`'s own value
    ///   type does not match `cell`'s registered type, or `filter`'s argument list
    ///   names `cell` itself.
    /// - `Error::TypeMismatch` — an argument cell's registered type does not match the
    ///   type `filter` declared for it.
    ///
    /// Guard independence is checked by [`Sheet::validate`] and by [`Sheet::propagate`],
    /// not by this mutator.
    ///
    /// - Complexity: O(a) where a is the number of filter argument cells.
    pub fn add_filter(&mut self, cell: CellId, filter: Filter) -> Result<(), Error> {
        let cell_type = self.cells.get(cell).ok_or(Error::InvalidId)?.type_id;
        if self.cells[cell].filter.is_some() {
            return Err(Error::InvalidFilter);
        }
        if filter.0.value_type != cell_type {
            return Err(Error::InvalidFilter);
        }
        if filter.0.args.contains(&cell) {
            return Err(Error::InvalidFilter);
        }
        for (&arg_id, &declared) in filter.0.args.iter().zip(filter.0.arg_types.iter()) {
            let arg_cell = self.cells.get(arg_id).ok_or(Error::InvalidId)?;
            if arg_cell.type_id != declared {
                return Err(Error::TypeMismatch {
                    expected: arg_cell.type_id,
                    found: declared,
                    sites: vec![],
                });
            }
        }

        self.invalidate_prepared_plans();
        for &arg in &filter.0.args {
            self.filter_dependents.entry(arg).or_default().push(cell);
        }
        self.cells[cell].filter = Some(filter.0);
        self.guard_independent = false;
        Ok(())
    }

    /// Returns the argument cells of `id`'s filter, in declaration order.
    ///
    /// Returns `None` if `id` is not a live cell in this sheet, or has no filter.
    pub fn filter_args(&self, id: CellId) -> Option<&[CellId]> {
        self.cells
            .get(id)?
            .filter
            .as_ref()
            .map(|f| f.args.as_slice())
    }

    /// Returns the live cells whose filter references `id` as one of its dynamic
    /// arguments — the reverse of a filter's own argument list ([`Sheet::filter_args`]).
    ///
    /// - Postcondition: empty if no live cell's filter references `id`.
    pub fn filter_dependents(&self, id: CellId) -> &[CellId] {
        self.filter_dependents
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns the kind of validation/derivation `id`'s filter performs, if it has one.
    ///
    /// Returns `None` if `id` is not a live cell in this sheet, or has no filter.
    pub fn filter_kind(&self, id: CellId) -> Option<&FilterKind> {
        self.cells.get(id)?.filter.as_ref().map(|f| &f.kind)
    }

    /// Returns `id`'s filter's current `(lo, hi)` bounds, if it has a [`FilterKind::Range`]
    /// filter.
    ///
    /// Resolves the filter's argument cells' current effective values via the same path
    /// [`Sheet::add_filter`] already uses, then calls the filter's `bounds` function.
    ///
    /// Returns `None` if `id` is not a live cell in this sheet, has no filter, its filter's
    /// kind isn't [`FilterKind::Range`], or the range expression fails to evaluate against
    /// the filter's current argument values (e.g. a fallible arithmetic op in a range
    /// endpoint) — the same degraded-to-`None` outcome as any other reason no live bounds
    /// are available right now.
    ///
    /// - Complexity: O(a) where a is the number of the filter's argument cells.
    pub fn filter_range<T: Any + Clone>(&self, id: CellId) -> Option<(T, T)> {
        let filter = self.cells.get(id)?.filter.as_ref()?;
        let FilterKind::Range { bounds } = &filter.kind else {
            return None;
        };
        let args: Vec<&dyn Any> = filter
            .args
            .iter()
            .map(|&a| self.cells[a].effective())
            .collect();
        let (lo, hi) = bounds(&args)?;
        Some((*lo.downcast::<T>().ok()?, *hi.downcast::<T>().ok()?))
    }

    /// Returns the filter violation recorded for `id` as of the last full
    /// `propagate()` call, if any.
    ///
    /// - Postcondition: `None` if `id` has no filter, `id`'s filter's last-checked
    ///   value held, or no full `propagate()` has run since `id` was last a plain
    ///   external write.
    pub fn filter_violation(&self, id: CellId) -> Option<&FilterViolation> {
        self.last_filter_violations.get(&id)
    }

    /// Iterates cells whose filter is currently violated, as of the last full
    /// `propagate()` call.
    ///
    /// - Complexity: O(n) where n is the number of currently-violated filters.
    pub fn filter_violated_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.last_filter_violations.keys().copied()
    }

    /// Returns the set of root cells currently determining a violated filter's own
    /// value or any of its argument values, as of the last full `propagate()` call —
    /// the same "which upstream cells caused this" query
    /// `requirement_contributing_cells`/`requirement_violation_cells` already provide
    /// for `Requirement`.
    ///
    /// - Postcondition: empty if no filter is currently violated.
    /// - Complexity: O(sum of `contributing_cells` cost over every violated filter and
    ///   its argument cells).
    pub fn filter_violation_cells(&self) -> HashSet<CellId> {
        let mut result = HashSet::new();
        for cell_id in self.filter_violated_cells() {
            result.extend(self.contributing_cells(cell_id));
            if let Some(args) = self.filter_args(cell_id) {
                for &arg in args {
                    result.extend(self.contributing_cells(arg));
                }
            }
        }
        result
    }

    /// Returns the requirements attached to `id`, in attachment order.
    ///
    /// Returns `None` if `id` is not a live cell in this sheet.
    pub fn cell_requirements(&self, id: CellId) -> Option<&[RequirementId]> {
        self.cells.get(id).map(|c| c.requirements.as_slice())
    }

    /// Returns the name of requirement `id`.
    ///
    /// Returns `None` if `id` is not a live requirement in this sheet.
    pub fn requirement_name(&self, id: RequirementId) -> Option<&str> {
        self.requirements.get(id)?.name.as_deref()
    }

    /// Returns the cell requirement `id` is attached to.
    ///
    /// Returns `None` if `id` is not a live requirement in this sheet.
    pub fn requirement_cell(&self, id: RequirementId) -> Option<CellId> {
        self.requirements.get(id).map(|c| c.cell)
    }

    /// Returns the cells requirement `id` reads.
    ///
    /// Returns `None` if `id` is not a live requirement in this sheet.
    pub fn requirement_inputs(&self, id: RequirementId) -> Option<&[CellId]> {
        self.requirements.get(id).map(|c| c.inputs.as_slice())
    }

    /// Returns `true` if every requirement on `id` held as of the last `propagate()`
    /// call.
    ///
    /// Returns `false` if no propagation has run yet. Also returns `true` for an
    /// `id` that is not a live cell in this sheet, since no requirement can have
    /// failed for a cell that doesn't exist.
    pub fn cell_requirements_valid(&self, id: CellId) -> bool {
        if self.last_plan.is_none() {
            return false;
        }
        !self.last_requirement_violations.contains_key(&id)
    }

    /// Iterates the requirements on `id` that evaluated to `false` as of the last
    /// `propagate()` call.
    ///
    /// - Postcondition: empty if `id`'s requirements all held, `id` is not a live
    ///   cell in this sheet, or no propagation has run yet.
    pub fn violated_requirements(&self, id: CellId) -> impl Iterator<Item = RequirementId> + '_ {
        self.last_requirement_violations
            .get(&id)
            .into_iter()
            .flatten()
            .copied()
    }

    /// Returns the set of root cells that could determine `id`'s value for *some* choice of
    /// cell strengths, as of the last `propagate()` call.
    ///
    /// A cell is a root candidate for `current` whenever some *active* relationship
    /// adjacent to `current` has a method producing it — not just the one method the
    /// current strengths happen to have selected: a different strength ordering could pick
    /// a different method of that same relationship, making any of its other cells the
    /// source instead. So every method (of every active relationship touching `current`)
    /// with `current` among its outputs contributes its inputs to the walk, not only the
    /// currently-selected one. [`Sheet::is_forced`] already answers "could `current` itself
    /// be left as a free source under some strength" precisely, so `current` is added
    /// directly whenever it is not forced. A self-referencing input (present in both a
    /// method's inputs and its outputs) is treated as one of its own roots directly, since
    /// it is read at its pre-execution value rather than derived further.
    ///
    /// Every visited cell is also checked against every conditional in the sheet: if any of
    /// that conditional's branches (or its default) has a method whose outputs include the
    /// visited cell, the conditional's match cell is added as a contributor too, and is
    /// itself traced recursively. This covers both an active producer that happens to
    /// belong to a conditional branch, and a cell with *no* active producer specifically
    /// because the branch that would define it isn't the one currently selected (that
    /// absence is itself a fact controlled by the match cell, not an indication that the
    /// match cell is irrelevant).
    ///
    /// - Postcondition: returns `{id}` if no propagation has run yet, or if `id` is not
    ///   forced and no active or conditional relationship could produce it.
    ///
    /// - Complexity: O(N · (R·M + B)) where N is the number of cells reachable from `id`
    ///   (including conditional match cells), R is the number of relationships adjacent to
    ///   a visited cell, M is the maximum number of methods per relationship, and B is the
    ///   total number of branch/default relationships across all conditionals.
    pub fn contributing_cells(&self, id: CellId) -> HashSet<CellId> {
        let mut result = HashSet::new();
        let mut visited: HashSet<CellId> = HashSet::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            if !visited.insert(current) {
                continue;
            }

            for match_cell in self.conditionals_potentially_producing(current) {
                stack.push(match_cell);
            }

            if !self.is_forced(current) {
                result.insert(current);
            }

            let Some(adj) = self.cells.get(current).map(|c| c.adj.clone()) else {
                continue;
            };
            for rel_id in adj {
                if !self.is_relationship_active(rel_id) {
                    continue;
                }
                for method in &self.relationships[rel_id].methods {
                    if !method.outputs.contains(&current) {
                        continue;
                    }
                    for &input in &method.inputs {
                        if method.outputs.contains(&input) {
                            result.insert(input);
                        } else {
                            stack.push(input);
                        }
                    }
                }
            }
        }
        result
    }

    /// Returns `true` if `rel_id` was part of the active relationship set for the last
    /// successful `propagate()` call — an unconditional relationship is always active; a conditional
    /// branch/default relationship is active only while its conditional currently selects
    /// it.
    ///
    /// Returns `false` if no propagation has run yet.
    fn is_relationship_active(&self, rel_id: RelationshipId) -> bool {
        self.last_plan
            .as_ref()
            .is_some_and(|plan| plan.active.contains(&rel_id))
    }

    /// Returns the match cells of every conditional with at least one branch (or default)
    /// relationship that touches `cell` (as an input or output of any of its methods) —
    /// every conditional whose branch choice currently determines, or could determine,
    /// `cell`'s value or whether it has an active producer at all.
    ///
    /// - Complexity: O(B) where B is the total number of branch/default relationships
    ///   across all conditionals.
    fn conditionals_potentially_producing(&self, cell: CellId) -> Vec<CellId> {
        self.conditionals
            .values()
            .filter(|cond| {
                cond.branches
                    .iter()
                    .flat_map(|branch| branch.relationships.iter())
                    .chain(cond.default.iter())
                    .any(|&rel_id| self.relationships[rel_id].adj.contains(&cell))
            })
            .flat_map(|cond| cond.match_cells().iter().copied())
            .collect()
    }

    /// Returns the union of [`Sheet::contributing_cells`] over requirement `id`'s own
    /// declared inputs.
    ///
    /// Returns an empty set if `id` is not a live requirement in this sheet.
    ///
    /// - Complexity: O(K·N) where K is the requirement's input count and N is the size of
    ///   each input's contributing set.
    pub fn requirement_contributing_cells(&self, id: RequirementId) -> HashSet<CellId> {
        let Some(requirement) = self.requirements.get(id) else {
            return HashSet::new();
        };
        requirement
            .inputs
            .iter()
            .flat_map(|&input| self.contributing_cells(input))
            .collect()
    }

    /// Returns the union of `contributing_cells` over every cell with at least one
    /// requirement — the set of cells currently determining at least one
    /// requirement-checked value, as of the last `propagate()` call.
    ///
    /// - Postcondition: empty if no cell in the sheet has any requirements.
    /// - Complexity: O(sum of `contributing_cells` cost over every cell with
    ///   requirements).
    pub fn requirement_relevant_cells(&self) -> HashSet<CellId> {
        self.cells
            .iter()
            .filter(|(_, c)| !c.requirements.is_empty())
            .flat_map(|(id, _)| self.contributing_cells(id))
            .collect()
    }

    /// Returns the union of `requirement_contributing_cells` over every requirement
    /// that evaluated `false` as of the last `propagate()` call, across every cell
    /// in the sheet.
    ///
    /// - Postcondition: empty if no requirement anywhere in the sheet currently
    ///   fails.
    /// - Complexity: O(sum of `requirement_contributing_cells` cost over every
    ///   violated requirement).
    pub fn requirement_violation_cells(&self) -> HashSet<CellId> {
        self.cells
            .keys()
            .flat_map(|id| self.violated_requirements(id))
            .flat_map(|rid| self.requirement_contributing_cells(rid))
            .collect()
    }

    /// Writes a value to a cell, incrementing the cell's write-recency strength.
    ///
    /// Each successful `write` increments a global monotonic counter and assigns
    /// the new value to `cell.strength`, so the most-recently-written cell always
    /// has the highest strength.
    ///
    /// - Postcondition: any pending derived override is cleared, so the written value is immediately visible via `read()`.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `id` is not a cell in this sheet.
    /// - `Error::TypeMismatch` — `T` does not match the cell's registered `TypeId`.
    /// - `Error::InvalidCellKind` — `id` is `Out`-kind.
    pub fn write<T: Any + 'static>(&mut self, id: CellId, value: T) -> Result<(), Error> {
        if self.cells.get(id).is_some_and(|c| c.kind == CellKind::Out) {
            return Err(Error::InvalidCellKind { sites: vec![] });
        }
        let cell_type = self.cells.get(id).ok_or(Error::InvalidId)?.type_id;
        if cell_type != TypeId::of::<T>() {
            return Err(Error::TypeMismatch {
                expected: cell_type,
                found: TypeId::of::<T>(),
                sites: vec![],
            });
        }

        self.invalidate_after_write(id);
        self.next_strength += 1;
        let cell = &mut self.cells[id];
        cell.strength = self.next_strength | (1u64 << 63);
        cell.source = Box::new(value);
        cell.derived = None;
        Ok(())
    }

    /// Returns a shared reference to the cell's effective current value: its derived
    /// override if one exists, otherwise its source (last written) value.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `id` is not a cell in this sheet.
    /// - `Error::TypeMismatch` — `T` does not match the cell's registered `TypeId`.
    pub fn read<T: Any + 'static>(&self, id: CellId) -> Result<&T, Error> {
        let cell = self.cells.get(id).ok_or(Error::InvalidId)?;
        if cell.type_id != TypeId::of::<T>() {
            return Err(Error::TypeMismatch {
                expected: cell.type_id,
                found: TypeId::of::<T>(),
                sites: vec![],
            });
        }
        Ok(cell
            .effective()
            .downcast_ref::<T>()
            .expect("type checked above"))
    }

    /// Returns the raw `source` slot: the last value written via `write()`/`add_cell`,
    /// ignoring any `derived` override produced by a self-referencing method or a
    /// conditionally forced relationship. For an ordinary (unshadowed) derived cell,
    /// `propagate()` writes straight into this same slot, so `source()` agrees with
    /// `read()`; the two diverge only for cells currently shadowed.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `id` is not a cell in this sheet.
    /// - `Error::TypeMismatch` — `T` does not match the cell's registered `TypeId`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use adam_rs::Sheet;
    ///
    /// let mut sheet = Sheet::new();
    /// let a = sheet.add_cell(3_i32);
    /// sheet.write(a, 8_i32).unwrap();
    /// assert_eq!(*sheet.source::<i32>(a).unwrap(), 8);
    /// ```
    pub fn source<T: Any + 'static>(&self, id: CellId) -> Result<&T, Error> {
        let cell = self.cells.get(id).ok_or(Error::InvalidId)?;
        if cell.type_id != TypeId::of::<T>() {
            return Err(Error::TypeMismatch {
                expected: cell.type_id,
                found: TypeId::of::<T>(),
                sites: vec![],
            });
        }
        Ok(cell.source.downcast_ref::<T>().expect("type checked above"))
    }

    /// Iterates over the cells that were updated during the last `propagate()` call.
    ///
    /// This includes cells written by selected methods and cells that reverted to their
    /// source values because the relationship that had been shadowing them (self-referencing
    /// or conditionally forced) is no longer producing them this round (Phase 5), even though
    /// no method wrote to them this round. It does not attempt to compare old/new values for
    /// equality. A `propagate()` call that returns `Error::SeedCycle` leaves this iterator
    /// empty: propagation clears stale changed-state before staged seed validation and
    /// aborts before committing any writes.
    ///
    /// - Complexity: O(n) where n is the number of changed cells.
    pub fn changed(&self) -> impl Iterator<Item = CellId> + '_ {
        self.changed_cells.iter().copied()
    }

    /// Clears the changed-cell set and resets each cell's `changed` flag.
    ///
    /// Call after processing the results of `propagate()`.
    ///
    /// - Complexity: O(n) where n is the number of changed cells.
    pub fn clear_changed(&mut self) {
        for id in std::mem::take(&mut self.changed_cells) {
            if let Some(cell) = self.cells.get_mut(id) {
                cell.changed = false;
            }
        }
    }

    /// Validates static guard independence for the current sheet structure.
    ///
    /// A filter's argument cells and a conditional's match cells must not depend on any
    /// cell governed by that filter or conditional in the static dependency graph. A
    /// successful call records the current structure as validated; a failed call leaves
    /// it unvalidated, so the same structure reports the dependency cycle again on the
    /// next [`Sheet::validate`] or [`Sheet::propagate`] call.
    ///
    /// # Errors
    ///
    /// - `Error::DependencyCycle` — a guard edge lies on a cycle in the static
    ///   dependency graph.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use adam_rs::{Filter, Method, Sheet};
    ///
    /// let mut sheet = Sheet::new();
    /// let limit = sheet.add_cell(10_i32);
    /// let value = sheet.add_cell(5_i32);
    /// let copy = sheet.add_cell(0_i32);
    /// sheet
    ///     .add_relationship(vec![Method::from_fn_1_1(value, copy, |x: &i32| Ok(*x))])
    ///     .unwrap();
    /// sheet
    ///     .add_filter(value, Filter::from_fn_1(limit, |v: &i32, hi: &i32| Ok((*v).min(*hi))))
    ///     .unwrap();
    /// sheet.validate().unwrap();
    /// ```
    ///
    /// - Complexity: O(1) when no structural mutation has happened since the last
    ///   successful validation; otherwise O(V + E), where V is the number of cells and
    ///   E is the number of static method and guard edges.
    pub fn validate(&mut self) -> Result<(), Error> {
        if self.guard_independent {
            return Ok(());
        }
        if let Some(path) = self.guard_violation() {
            return Err(Error::DependencyCycle {
                sites: path.into_sites(),
            });
        }
        self.guard_independent = true;
        Ok(())
    }

    /// Iterates all live cell IDs in the sheet.
    ///
    /// - Complexity: O(n) where n is the number of cells.
    pub fn cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.cells.keys()
    }

    /// Iterates all live relationship IDs in the sheet.
    ///
    /// - Complexity: O(n) where n is the number of relationships.
    pub fn relationships(&self) -> impl Iterator<Item = RelationshipId> + '_ {
        self.relationships.keys()
    }

    /// Returns the relationships adjacent to `id`.
    ///
    /// Returns `None` if `id` is not a live cell in this sheet.
    ///
    /// - Complexity: O(1).
    pub fn cell_adj(&self, id: CellId) -> Option<&[RelationshipId]> {
        self.cells.get(id).map(|c| c.adj.as_slice())
    }

    /// Returns the cells adjacent to `id` (union across all methods).
    ///
    /// Returns `None` if `id` is not a live relationship in this sheet.
    ///
    /// - Complexity: O(1).
    pub fn relationship_adj(&self, id: RelationshipId) -> Option<&[CellId]> {
        self.relationships.get(id).map(|r| r.adj.as_slice())
    }

    /// Evaluates conditional `cond`'s current live match value.
    ///
    /// # Errors
    ///
    /// - `Error::MethodFailed` — the match subject is a [`MatchExpr`] whose function
    ///   returned an error.
    fn evaluate_match_source(&self, cond: &ConditionalData) -> Result<MatchValue<'_>, Error> {
        match &cond.source {
            MatchSource::Cell(id) => Ok(MatchValue::Ref(self.cells[*id].effective())),
            MatchSource::Expr(expr) => {
                let args: Vec<&dyn Any> = expr
                    .inputs
                    .iter()
                    .map(|&id| self.cells[id].effective())
                    .collect();
                let value = (expr.function)(&args).map_err(|error| Error::MethodFailed {
                    error,
                    sites: vec![],
                })?;
                Ok(MatchValue::Owned(value))
            }
        }
    }

    /// Evaluates conditional `cond` against staged propagation state.
    ///
    /// Uses `stage`'s writes first, then falls back to each cell's `source` value,
    /// matching the state visible after Phase 0 excludes old derived overrides and any
    /// pre-plan steps already staged.
    ///
    /// # Errors
    ///
    /// - `Error::MethodFailed` — the match subject is a [`MatchExpr`] whose function
    ///   returned an error.
    fn evaluate_match_source_staged<'a>(
        &'a self,
        cond: &ConditionalData,
        stage: &'a PropagationStage,
    ) -> Result<MatchValue<'a>, Error> {
        match &cond.source {
            MatchSource::Cell(id) => Ok(MatchValue::Ref(stage.effective(&self.cells, *id))),
            MatchSource::Expr(expr) => {
                let args: Vec<&dyn Any> = expr
                    .inputs
                    .iter()
                    .map(|&id| stage.effective(&self.cells, id))
                    .collect();
                let value = (expr.function)(&args).map_err(|error| Error::MethodFailed {
                    error,
                    sites: vec![],
                })?;
                Ok(MatchValue::Owned(value))
            }
        }
    }

    /// Returns the equality function used to compare `cond`'s match value against branch
    /// keys: the match cell's own `eq_fn` for a plain match subject, or the expression's
    /// captured `eq_fn` for a computed one.
    fn match_eq_fn(&self, cond: &ConditionalData) -> fn(&dyn Any, &dyn Any) -> bool {
        match &cond.source {
            MatchSource::Cell(id) => self.cells[*id].eq_fn,
            MatchSource::Expr(expr) => expr.eq_fn,
        }
    }

    /// Builds the active relationship set against staged propagation state.
    ///
    /// Every conditional reads `stage`'s writes first and otherwise falls back to cell
    /// `source` values rather than any live
    /// `derived` override already present on the sheet.
    ///
    /// # Errors
    ///
    /// - `Error::MethodFailed` — an expression-sourced conditional's function returned an
    ///   error.
    fn build_active_set_staged(
        &self,
        stage: &PropagationStage,
    ) -> Result<HashSet<RelationshipId>, Error> {
        let mut active: HashSet<RelationshipId> = self
            .relationships
            .keys()
            .filter(|id| !self.conditional_relationships.contains(id))
            .collect();

        for (_, cond) in &self.conditionals {
            let value = self.evaluate_match_source_staged(cond, stage)?;
            let value_ref = value.as_dyn();
            let eq_fn = self.match_eq_fn(cond);

            let mut matched = false;
            for branch in &cond.branches {
                if branch.keys.iter().any(|key| eq_fn(value_ref, key.as_ref())) {
                    for &rel_id in &branch.relationships {
                        active.insert(rel_id);
                    }
                    matched = true;
                    break;
                }
            }
            if !matched {
                for &rel_id in &cond.default {
                    active.insert(rel_id);
                }
            }
        }

        Ok(active)
    }

    /// Records the actual dependency producers and write classifications for `plan`.
    ///
    /// - Complexity: expected O(V + E), where V is the number of selected steps and E is
    ///   the number of method inputs, outputs, and filter arguments.
    fn plan_provenance(&self, plan: &Plan) -> PlanProvenance {
        let mut provenance = PlanProvenance::default();
        let mut effective_producers = HashMap::new();
        let mut source_producers = HashMap::new();

        for &step in &plan.execution_order {
            let step_provenance = match step {
                PlanStep::Method(rel_id, method_index) => {
                    let method = &self.relationships[rel_id].methods[method_index];
                    let method_inputs: HashSet<CellId> = method.inputs.iter().copied().collect();
                    let method_outputs: HashSet<CellId> = method.outputs.iter().copied().collect();
                    let mut input_producers = Vec::with_capacity(method.inputs.len());
                    let mut seed_inputs = Vec::new();
                    for &input in &method.inputs {
                        if method_outputs.contains(&input) {
                            seed_inputs.push(input);
                            input_producers.push(None);
                        } else {
                            input_producers.push(effective_producers.get(&input).copied());
                        }
                    }
                    let output_classes: Vec<_> = method
                        .outputs
                        .iter()
                        .map(|&output| {
                            (
                                output,
                                if method_inputs.contains(&output)
                                    || plan.forced_outputs.contains(&output)
                                {
                                    OutputClassification::Derived
                                } else {
                                    OutputClassification::Source
                                },
                            )
                        })
                        .collect();
                    let step_provenance = StepProvenance {
                        input_producers,
                        seed_inputs,
                        source_producer: None,
                        output_classes: output_classes.clone(),
                    };
                    for &(output, _) in &output_classes {
                        effective_producers.insert(output, step);
                    }
                    for &(output, classification) in &output_classes {
                        if classification == OutputClassification::Source {
                            source_producers.insert(output, step);
                        }
                    }
                    step_provenance
                }
                PlanStep::FilterReclamp(cell) => {
                    let filter = self.cells[cell]
                        .filter
                        .as_ref()
                        .expect("plan() only emits FilterReclamp for a filtered cell");
                    let input_producers = filter
                        .args
                        .iter()
                        .map(|input| effective_producers.get(input).copied())
                        .collect();
                    let step_provenance = StepProvenance {
                        input_producers,
                        seed_inputs: Vec::new(),
                        source_producer: source_producers.get(&cell).copied(),
                        output_classes: vec![(cell, OutputClassification::Derived)],
                    };
                    effective_producers.insert(cell, step);
                    step_provenance
                }
            };
            provenance.steps.insert(step, step_provenance);
        }
        provenance.output_producers = effective_producers;
        provenance
    }

    /// Checks that a staged prerequisite step has identical selected producer paths in both plans.
    ///
    /// - Postcondition: `true` implies the selected method/filter, its input producers,
    ///   seed inputs, source producer, and output classifications all match.
    fn prerequisite_step_is_compatible(
        &self,
        pre_plan: &PlanProvenance,
        final_plan: &PlanProvenance,
        step: PlanStep,
        stage: &PropagationStage,
    ) -> bool {
        let Some(staged) = stage.step_provenance.get(&step) else {
            return false;
        };
        pre_plan.steps.get(&step) == Some(staged) && final_plan.steps.get(&step) == Some(staged)
    }

    /// Returns concrete cells and relationships implicated by a staged-step mismatch.
    ///
    /// - Complexity: O(K) sites for the step's cells and producer edges.
    fn conflict_sites_for_step(
        &self,
        step: PlanStep,
        pre_plan: &PlanProvenance,
        final_plan: &PlanProvenance,
    ) -> Vec<ErrorSite> {
        let mut sites = Vec::new();
        let mut seen_sites = HashSet::new();
        let mut add_site = |site| {
            if seen_sites.insert(site) {
                sites.push(site);
            }
        };
        let mut cells = Vec::new();

        match step {
            PlanStep::Method(rel_id, method_index) => {
                add_site(ErrorSite::Relationship(rel_id));
                add_site(ErrorSite::Method(rel_id, method_index));
                let method = &self.relationships[rel_id].methods[method_index];
                cells.extend(method.inputs.iter().copied());
                cells.extend(method.outputs.iter().copied());
            }
            PlanStep::FilterReclamp(cell) => {
                cells.push(cell);
                if let Some(filter) = &self.cells[cell].filter {
                    cells.extend(filter.args.iter().copied());
                }
            }
        }

        for provenance in [pre_plan.steps.get(&step), final_plan.steps.get(&step)]
            .into_iter()
            .flatten()
        {
            for producer in provenance
                .input_producers
                .iter()
                .copied()
                .flatten()
                .chain(provenance.source_producer)
            {
                Self::add_step_relationship_site(producer, &mut add_site);
            }
        }
        for cell in cells {
            add_site(ErrorSite::Cell(cell));
            for plan in [pre_plan, final_plan] {
                if let Some(&producer) = plan.output_producers.get(&cell) {
                    Self::add_step_relationship_site(producer, &mut add_site);
                }
            }
        }
        sites
    }

    /// Adds the relationship site associated with a selected method producer.
    fn add_step_relationship_site(step: PlanStep, add_site: &mut impl FnMut(ErrorSite)) {
        match step {
            PlanStep::Method(rel_id, _) => add_site(ErrorSite::Relationship(rel_id)),
            PlanStep::FilterReclamp(cell) => add_site(ErrorSite::Cell(cell)),
        }
    }

    /// Executes `execution_order` once into transactional staged state.
    ///
    /// A method or filter step already evaluated by the conditional pre-plan reuses
    /// its staged result when the general plan contains the same compatible step.
    /// This is an internal optimization, not a callback invocation guarantee.
    /// Filter failures remain non-fatal and leave the staged cell untouched,
    /// matching `execute_plan`.
    ///
    /// # Errors
    ///
    /// - `Error::MethodFailed` — a `PlanStep::Method` step's function returned an error,
    ///   or the method produced a different number of outputs than declared.
    /// - `Error::TypeMismatch` — a `PlanStep::Method` step's output runtime type does
    ///   not match the cell's registered type.
    fn execute_plan_staged(
        &self,
        execution_order: &[PlanStep],
        seeds: &Seeds,
        forced_outputs: &HashSet<CellId>,
        plan_provenance: &PlanProvenance,
        stage: &mut PropagationStage,
        filter_violations: &mut Vec<(CellId, FilterViolation)>,
    ) -> Result<(), Error> {
        for step in execution_order {
            match *step {
                PlanStep::Method(rel_id, method_idx) => {
                    if stage.executed_methods.contains(&(rel_id, method_idx)) {
                        let method = &self.relationships[rel_id].methods[method_idx];
                        for &output in &method.outputs {
                            stage.reclassify(
                                output,
                                method.inputs.contains(&output) || forced_outputs.contains(&output),
                            );
                        }
                        continue;
                    }
                    let (outputs, output_ids, shadow_outputs) = {
                        let method = &self.relationships[rel_id].methods[method_idx];
                        let inputs: Vec<&dyn Any> = method
                            .inputs
                            .iter()
                            .map(|&id| {
                                if method.outputs.contains(&id) {
                                    seeds
                                        .get(&id)
                                        .map(|value| value.as_ref())
                                        .unwrap_or_else(|| stage.source(&self.cells, id))
                                } else {
                                    stage.effective(&self.cells, id)
                                }
                            })
                            .collect();
                        let outputs =
                            (method.function)(&inputs).map_err(|error| Error::MethodFailed {
                                error,
                                sites: vec![ErrorSite::Method(rel_id, method_idx)],
                            })?;
                        let output_ids = method.outputs.clone();
                        let shadow_outputs: Vec<bool> = method
                            .outputs
                            .iter()
                            .map(|o| method.inputs.contains(o) || forced_outputs.contains(o))
                            .collect();
                        (outputs, output_ids, shadow_outputs)
                    };

                    if outputs.len() != output_ids.len() {
                        return Err(Error::MethodFailed {
                            error: anyhow::anyhow!(
                                "method produced {} outputs but relationship expects {}",
                                outputs.len(),
                                output_ids.len()
                            ),
                            sites: vec![ErrorSite::Method(rel_id, method_idx)],
                        });
                    }

                    for ((cell_id, new_value), shadow) in
                        output_ids.into_iter().zip(outputs).zip(shadow_outputs)
                    {
                        let found = new_value.as_ref().type_id();
                        let cell = &self.cells[cell_id];
                        if found != cell.type_id {
                            return Err(Error::TypeMismatch {
                                expected: cell.type_id,
                                found,
                                sites: vec![ErrorSite::Method(rel_id, method_idx)],
                            });
                        }
                        stage.write(cell_id, new_value, shadow);
                    }
                    stage.executed_methods.insert((rel_id, method_idx));
                    stage
                        .step_provenance
                        .insert(*step, plan_provenance.steps[step].clone());
                }
                PlanStep::FilterReclamp(id) => {
                    if stage.executed_filters.contains(&id) {
                        continue;
                    }
                    let filter = self.cells[id]
                        .filter
                        .as_ref()
                        .expect("plan() only emits FilterReclamp for a filtered cell");
                    let args: Vec<&dyn Any> = filter
                        .args
                        .iter()
                        .map(|&a| stage.effective(&self.cells, a))
                        .collect();
                    let current = stage.source(&self.cells, id);
                    match (filter.function)(current, &args) {
                        Ok(v) if v.as_ref().type_id() == self.cells[id].type_id => {
                            stage.write(id, v, true);
                        }
                        Ok(_) => filter_violations.push((
                            id,
                            FilterViolation::Failed(anyhow::anyhow!(
                                "filter returned a value of a different type than the cell"
                            )),
                        )),
                        Err(error) => {
                            filter_violations.push((id, FilterViolation::Failed(error)));
                        }
                    }
                    stage.executed_filters.insert(id);
                    stage
                        .step_provenance
                        .insert(*step, plan_provenance.steps[step].clone());
                }
            }
        }
        Ok(())
    }

    /// Assigns derived-cell strengths after a planning pass.
    ///
    /// Walks `execution_order` and assigns a decrementing counter (starting at
    /// `0x7FFF_FFFF_FFFF_FFFF`) to each output cell of each selected method, in
    /// execution order. Cells evaluated first receive the highest derived strength.
    /// Source cells (not the output of any selected method) are not modified.
    ///
    /// A cell claimed *self-referencingly* (its claiming method reads the cell as one of
    /// its own inputs) keeps a live explicit strength rather than being demoted: such a
    /// method computes the cell from its own `source` aspiration (see
    /// [`crate::planner::evaluate_seeds`]), so an explicit `write()` to that cell is still
    /// the authority behind the value and must keep outranking never-written cells in
    /// later rounds. Demoting it would discard the edit the next time the cell is
    /// re-seeded.
    ///
    /// - Complexity: O(R·K²) where R is the number of entries and K is the maximum
    ///   inputs or outputs per method.
    /// - Postcondition: returns whether any cell's strength actually changed.
    fn post_process_strengths(&mut self, execution_order: &[PlanStep]) -> bool {
        let mut changed = false;
        let mut derived_strength = u64::MAX >> 1; // 0x7FFF_FFFF_FFFF_FFFF
        let mut seen: std::collections::HashSet<CellId> = std::collections::HashSet::new();
        for step in execution_order {
            let PlanStep::Method(rel_id, method_idx) = step else {
                continue;
            };
            if let Some(rel) = self.relationships.get(*rel_id)
                && let Some(method) = rel.methods.get(*method_idx)
            {
                for &output in &method.outputs {
                    let self_referencing = method.inputs.contains(&output);
                    if seen.insert(output)
                        && let Some(cell) = self.cells.get_mut(output)
                    {
                        if self_referencing && cell.has_explicit_strength() {
                            continue;
                        }
                        changed |= cell.strength != derived_strength;
                        cell.strength = derived_strength;
                        derived_strength = derived_strength.saturating_sub(1);
                    }
                }
            }
        }
        changed
    }

    /// Publishes a fully validated propagation stage and returns previously derived cells.
    ///
    /// Clears every old derived override, applies staged source and derived writes, and
    /// records each staged output in [`Sheet::changed`]. No callback is evaluated here.
    ///
    /// - Complexity: O(cells).
    fn commit_stage(&mut self, stage: PropagationStage) -> Vec<CellId> {
        let previously_derived: Vec<CellId> = self
            .cells
            .iter()
            .filter(|(_, cell)| cell.derived.is_some())
            .map(|(id, _)| id)
            .collect();
        for (_, cell) in self.cells.iter_mut() {
            cell.derived = None;
        }
        for (id, value) in stage.sources {
            self.cells[id].source = value;
        }
        for (id, value) in stage.derived {
            self.cells[id].derived = Some(value);
        }
        for id in stage.changed {
            let cell = &mut self.cells[id];
            if !cell.changed {
                cell.changed = true;
                self.changed_cells.push(id);
            }
        }
        previously_derived
    }

    /// Runs the planning pass and executes the selected methods.
    ///
    /// Validates static guard independence before mutating state, then clears the
    /// changed-cell set from the previous `propagate()` call before staged planning.
    /// After propagation, call [`Sheet::changed`] to inspect which cells were updated,
    /// and [`Sheet::clear_changed`] when done.
    ///
    /// Method, filter, conditional-expression, and requirement callbacks must be
    /// purely functional. There is no guarantee that any callback executes, or when
    /// or how often it executes; results may be reused or recomputed while preserving
    /// cell values and diagnostics. The phases below describe the current algorithm,
    /// not a callback invocation contract.
    ///
    /// **Phase 0 — Staging:** propagation starts from each cell's source value, excluding
    /// every derived override from the previous round, but does not mutate live cells.
    ///
    /// **Phase 1 — Pre-plan:** if any conditional match cells are derived (have an
    /// in-edge in the unconditional relationship graph), the unconditional plan's selected
    /// guard-prerequisite cone is executed so their values are current before branch evaluation.
    /// Unrelated filters remain deferred until the final active plan, after the relationships
    /// that produce their arguments.
    ///
    /// **Phase 2 — Conditional evaluation:** each conditional's match cell value is
    /// read and compared against branch keys; the active relationship set is built.
    ///
    /// **Phase 3 — General plan:** the Adam algorithm runs on the active set. Seed-cycle
    /// validation and method execution consume staged Phase 1 values. A callback already
    /// evaluated in Phase 1 is reused only when the final plan preserves its selected
    /// method, input producers, and output classification. A mismatched plan returns a
    /// conservative conflict rather than committing stale values. This
    /// boundary may reject a sheet even when another assignment could have avoided the mismatch;
    /// propagation reports the implicated sites instead of attempting that alternate assignment.
    /// A Phase 1 seed callback is likewise reused only when every input reads the same staged
    /// source version, recursive seed result, or accumulated value; otherwise propagation
    /// returns a conflict instead of reusing a stale seed.
    ///
    /// **Phase 4 — Commit and strength post-processing:** after planning, seed
    /// validation, and staged method execution all succeed, staged writes are published
    /// atomically. Derived cells then receive low-order strengths in evaluation order,
    /// enforcing the stability invariant. A cell claimed self-referencingly keeps any
    /// live explicit strength instead, since its own written value is still the authority
    /// behind the result.
    ///
    /// **Phase 5 — Reversion change-tracking:** a cell whose derived override existed
    /// before this round but wasn't reclaimed by any method this round has effectively
    /// reverted to its source value (e.g. its forcing conditional went inactive); it is
    /// marked changed even though no method wrote to it this round.
    ///
    /// **Phase 6 — Requirement evaluation:** every registered requirement is evaluated
    /// against current cell values, rebuilding `last_requirement_violations` from
    /// scratch, so [`Sheet::cell_requirements_valid`] and [`Sheet::violated_requirements`]
    /// reflect this round.
    ///
    /// If staged seed validation detects a non-self sibling dependency cycle, or compatibility
    /// checking returns a prerequisite conflict, `propagate()` returns its error after validation
    /// and changed-state clearing but before commit. Live cell values therefore remain untouched
    /// and [`Sheet::changed`] stays empty for that failing call.
    ///
    /// - Complexity: on a cache miss, prerequisite-cone indexing, traversal, and producer
    ///   preparation take O(V + E) per selected plan, distinct from assignment matching.
    ///   Cache hits omit this preparation and assignment matching; a pending strength
    ///   certificate takes O(C) for C cells. Fresh staged execution, seed evaluation,
    ///   active-set comparison, and diagnostics still run for current values.
    ///
    /// # Errors
    ///
    /// - `Error::DependencyCycle` — a filter or conditional guard edge lies on a cycle in
    ///   the static dependency graph.
    /// - `Error::Conflict` — no valid method assignment exists, or a pre-executed guard
    ///   prerequisite is incompatible with the final selected plan.
    /// - `Error::SeedCycle` — sibling seed dependencies form a non-self cycle.
    /// - `Error::MethodFailed` — a method's function returned an error, a method
    ///   produced the wrong number of outputs, or a requirement's function returned
    ///   an error.
    /// - `Error::TypeMismatch` — a method output's runtime type does not match the
    ///   cell's registered type.
    pub fn propagate(&mut self) -> Result<(), Error> {
        let mut pre = self.pre_plan_cache.take();
        let mut main = self.main_plan_cache.take();
        let result = self.propagate_with_caches(&mut pre, &mut main);
        if result.is_ok() {
            self.pre_plan_cache = pre;
            self.main_plan_cache = main;
        }
        result
    }

    /// Executes `execution_order` without invoking the planner.
    ///
    /// A `PlanStep::FilterReclamp(id)` step re-evaluates `id`'s filter against `id`'s own
    /// current `source` value (never a possibly-shadowed `derived` — the same
    /// self-referencing-input rule a `PlanStep::Method` step's self-referencing inputs
    /// follow) and its filter arguments' current effective values, writing the result
    /// into `id`'s `derived` unconditionally — `source` is never touched by this step,
    /// exactly as it's never touched by any other self-referencing method's output. A
    /// `PlanStep::Method` step's outputs follow the existing shadow/non-shadow rule,
    /// unchanged; a non-shadow output also clears any leftover `derived` from an earlier
    /// step in this same call (e.g. Phase 1 shadowing a cell that a later Phase 3 step
    /// then claims as a plain output), so `effective()` reflects the fresh `source` write
    /// rather than a stale override. A reclamp whose filter returns `Err`, or a value of
    /// the wrong type, is pushed into `filter_violations` instead of aborting; the cell's
    /// stored value is left untouched in that case (its `derived` stays unset, so
    /// `read()` falls back to `source`).
    ///
    /// A `PlanStep::Method` step's self-referencing input reads its precomputed `seeds`
    /// value (see [`crate::planner::evaluate_seeds`]) if present, else its own `source` —
    /// never a `derived` override from this same execution.
    ///
    /// # Errors
    ///
    /// - `Error::MethodFailed` — a `PlanStep::Method` step's function returned an error,
    ///   or the method produced a different number of outputs than declared.
    /// - `Error::TypeMismatch` — a `PlanStep::Method` step's output runtime type does
    ///   not match the cell's registered type.
    ///
    /// - Complexity: O(R·K) where R is the number of entries and K is the max cells per method,
    ///   plus per-method execution cost.
    #[cfg(test)]
    fn execute_plan(
        &mut self,
        execution_order: &[PlanStep],
        seeds: &Seeds,
        forced_outputs: &HashSet<CellId>,
        filter_violations: &mut Vec<(CellId, FilterViolation)>,
    ) -> Result<(), Error> {
        for step in execution_order {
            match *step {
                PlanStep::Method(rel_id, method_idx) => {
                    let (outputs, output_ids, shadow_outputs) = {
                        let method = &self.relationships[rel_id].methods[method_idx];
                        let inputs: Vec<&dyn Any> = method
                            .inputs
                            .iter()
                            .map(|&id| {
                                if method.outputs.contains(&id) {
                                    // Self-referencing input: its precomputed seed, else
                                    // its own source -- never a derived override from
                                    // this same execution. See evaluate_seeds.
                                    seeds
                                        .get(&id)
                                        .map(|value| value.as_ref())
                                        .unwrap_or_else(|| self.cells[id].source.as_ref())
                                } else {
                                    self.cells[id].effective()
                                }
                            })
                            .collect();
                        let outputs =
                            (method.function)(&inputs).map_err(|error| Error::MethodFailed {
                                error,
                                sites: vec![ErrorSite::Method(rel_id, method_idx)],
                            })?;
                        let output_ids = method.outputs.clone();
                        let shadow_outputs: Vec<bool> = method
                            .outputs
                            .iter()
                            .map(|o| method.inputs.contains(o) || forced_outputs.contains(o))
                            .collect();
                        (outputs, output_ids, shadow_outputs)
                    };

                    if outputs.len() != output_ids.len() {
                        return Err(Error::MethodFailed {
                            error: anyhow::anyhow!(
                                "method produced {} outputs but relationship expects {}",
                                outputs.len(),
                                output_ids.len()
                            ),
                            sites: vec![ErrorSite::Method(rel_id, method_idx)],
                        });
                    }

                    for ((cell_id, new_value), shadow) in
                        output_ids.into_iter().zip(outputs).zip(shadow_outputs)
                    {
                        let cell = &mut self.cells[cell_id];
                        let found = new_value.as_ref().type_id();
                        if found != cell.type_id {
                            return Err(Error::TypeMismatch {
                                expected: cell.type_id,
                                found,
                                sites: vec![ErrorSite::Method(rel_id, method_idx)],
                            });
                        }
                        if shadow {
                            cell.derived = Some(new_value);
                        } else {
                            cell.source = new_value;
                            cell.derived = None;
                        }
                        if !cell.changed {
                            cell.changed = true;
                            self.changed_cells.push(cell_id);
                        }
                    }
                }
                PlanStep::FilterReclamp(id) => {
                    let filter = self.cells[id]
                        .filter
                        .as_ref()
                        .expect("plan() only emits FilterReclamp for a filtered cell");
                    let args: Vec<&dyn Any> = filter
                        .args
                        .iter()
                        .map(|&a| self.cells[a].effective())
                        .collect();
                    // Self-referencing input: always `source`, never a possibly-shadowed
                    // `derived` — same rule as any other self-referencing method (see
                    // the `PlanStep::Method` arm above, and the 2026-08-02 shadow-state
                    // design). This is what keeps `source` provably untouched by the
                    // filter across any number of rounds.
                    let current = self.cells[id].source.as_ref();
                    match (filter.function)(current, &args) {
                        Ok(v) => {
                            let cell_type = self.cells[id].type_id;
                            if v.as_ref().type_id() != cell_type {
                                filter_violations.push((
                                    id,
                                    FilterViolation::Failed(anyhow::anyhow!(
                                        "filter returned a value of a different type than \
                                         the cell"
                                    )),
                                ));
                            } else {
                                // Unconditional write, no equality check — matches every
                                // other shadowed output's "no equality check" convention
                                // (2026-08-02 design). The filter's Ok output is
                                // authoritative the same way any method's output is.
                                let cell = &mut self.cells[id];
                                cell.derived = Some(v);
                                if !cell.changed {
                                    cell.changed = true;
                                    self.changed_cells.push(id);
                                }
                            }
                        }
                        Err(e) => filter_violations.push((id, FilterViolation::Failed(e))),
                    }
                }
            }
        }
        Ok(())
    }

    /// Returns the index of the method selected for `rel` in the last successful propagation.
    ///
    /// Returns `None` if no propagation has run yet, `rel` is not in the cached plan,
    /// or `rel` was added after the last `propagate()` call.
    pub fn selected_method(&self, rel: RelationshipId) -> Option<usize> {
        self.last_plan
            .as_ref()?
            .plan
            .execution_order
            .iter()
            .find_map(|step| match step {
                PlanStep::Method(r, idx) if *r == rel => Some(*idx),
                _ => None,
            })
    }

    /// Returns the input cells of method `idx` in relationship `rel`.
    ///
    /// Returns `None` if `rel` is not a live relationship or `idx` is out of bounds.
    pub fn method_inputs(&self, rel: RelationshipId, idx: usize) -> Option<&[CellId]> {
        self.relationships
            .get(rel)?
            .methods
            .get(idx)
            .map(|m| m.inputs.as_slice())
    }

    /// Returns the output cells of method `idx` in relationship `rel`.
    ///
    /// Returns `None` if `rel` is not a live relationship or `idx` is out of bounds.
    pub fn method_outputs(&self, rel: RelationshipId, idx: usize) -> Option<&[CellId]> {
        self.relationships
            .get(rel)?
            .methods
            .get(idx)
            .map(|m| m.outputs.as_slice())
    }

    /// Returns `true` if `id` was not written by any selected method in the last successful propagation.
    ///
    /// Returns `false` if no propagation has run yet (conservatively forces a full re-plan).
    ///
    /// Cells absent from the completed plan, including newly added cells, return `true`.
    ///
    /// - Complexity: Expected O(1).
    pub fn is_source(&self, id: CellId) -> bool {
        let Some(plan) = &self.last_plan else {
            return false;
        };
        !plan.certificate.contains_cell(id) || plan.certificate.is_released(id)
    }

    /// Returns `true` if `id` can never be a source, as of the last successful
    /// `propagate()` call.
    ///
    /// Some active relationship's method structure guarantees the cell is always
    /// produced by a method, regardless of strength — writing to it has no lasting
    /// effect once `propagate()` runs again. Useful for disabling input fields in a UI.
    ///
    /// Returns `false` if no propagation has run yet.
    pub fn is_forced(&self, id: CellId) -> bool {
        self.last_plan
            .as_ref()
            .is_some_and(|prepared| prepared.plan.forced_outputs.contains(&id))
    }

    /// Iterates cells that are forced (see [`Sheet::is_forced`]) as of the last
    /// successful `propagate()` call.
    ///
    /// - Complexity: O(n) where n is the number of forced cells.
    pub fn forced_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.last_plan
            .iter()
            .flat_map(|prepared| prepared.plan.forced_outputs.iter().copied())
    }

    /// Returns `true` if `id` had exactly one viable method as of the last successful
    /// `propagate()` call — the planner has no alternative method to choose for this
    /// relationship, regardless of cell strength.
    ///
    /// Returns `false` if no propagation has run yet.
    pub fn is_relationship_forced(&self, id: RelationshipId) -> bool {
        self.last_plan
            .as_ref()
            .is_some_and(|prepared| prepared.plan.forced_relationships.contains(&id))
    }

    /// Iterates relationships that are forced (see [`Sheet::is_relationship_forced`])
    /// as of the last successful `propagate()` call.
    ///
    /// - Complexity: O(n) where n is the number of forced relationships.
    pub fn forced_relationships(&self) -> impl Iterator<Item = RelationshipId> + '_ {
        self.last_plan
            .iter()
            .flat_map(|prepared| prepared.plan.forced_relationships.iter().copied())
    }

    /// Iterates all live conditional IDs in the sheet.
    ///
    /// - Complexity: O(n) where n is the number of conditionals.
    pub fn conditionals(&self) -> impl Iterator<Item = ConditionalId> + '_ {
        self.conditionals.keys()
    }

    /// Iterates all live `Out`-kind cells in the sheet.
    ///
    /// - Complexity: O(n) where n is the number of cells.
    pub fn out_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.cells
            .iter()
            .filter(|(_, c)| c.kind == CellKind::Out)
            .map(|(id, _)| id)
    }

    /// Returns the match cells for conditional `id`: a single cell for a plain match
    /// subject, or every input of a [`MatchExpr`] match subject.
    ///
    /// Returns `None` if `id` is not a live conditional in this sheet.
    pub fn conditional_match_cells(&self, id: ConditionalId) -> Option<&[CellId]> {
        self.conditionals.get(id).map(|c| c.match_cells())
    }

    /// Returns the number of named branches in conditional `id`.
    ///
    /// Returns `None` if `id` is not a live conditional in this sheet.
    pub fn conditional_branch_count(&self, id: ConditionalId) -> Option<usize> {
        self.conditionals.get(id).map(|c| c.branches.len())
    }

    /// Returns the relationship IDs for branch `branch` of conditional `id`.
    ///
    /// Returns `None` if `id` is not a live conditional, or `branch` is out of bounds.
    pub fn conditional_branch_relationships(
        &self,
        id: ConditionalId,
        branch: usize,
    ) -> Option<&[RelationshipId]> {
        self.conditionals
            .get(id)?
            .branches
            .get(branch)
            .map(|b| b.relationships.as_slice())
    }

    /// Returns the default relationship IDs for conditional `id`.
    ///
    /// These relationships are active when no named branch key matches the match cell.
    /// Returns `None` if `id` is not a live conditional in this sheet.
    pub fn conditional_default_relationships(
        &self,
        id: ConditionalId,
    ) -> Option<&[RelationshipId]> {
        self.conditionals.get(id).map(|c| c.default.as_slice())
    }

    /// Returns the index of the currently matching branch for conditional `id`.
    ///
    /// Evaluates branch keys against the match subject's current value in definition
    /// order; returns the index of the first matching branch. Returns `Ok(None)` if no
    /// branch key matches (the default branch is active) or if `id` is not a live
    /// conditional.
    ///
    /// # Errors
    ///
    /// - `Error::MethodFailed` — `id` is a live, expression-sourced conditional whose
    ///   function returned an error.
    ///
    /// - Complexity: O(B·K) where B = branches, K = keys per branch.
    pub fn conditional_active_branch(&self, id: ConditionalId) -> Result<Option<usize>, Error> {
        let Some(cond) = self.conditionals.get(id) else {
            return Ok(None);
        };
        let value = self.evaluate_match_source(cond)?;
        let value_ref = value.as_dyn();
        let eq_fn = self.match_eq_fn(cond);
        Ok(cond
            .branches
            .iter()
            .enumerate()
            .find(|(_, branch)| branch.keys.iter().any(|key| eq_fn(value_ref, key.as_ref())))
            .map(|(i, _)| i))
    }
}

impl Default for Sheet {
    /// Returns `Sheet::new()`.
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        CellKind, ConditionalId, Error, MatchExpr, Method, Requirement, Sheet,
        cell::CellId,
        error::ErrorSite,
        filter::{Filter, FilterKind, FilterViolation},
        planner::{PlanStep, Seeds},
        relationship::RelationshipId,
    };
    use std::any::{Any, TypeId};
    use std::collections::{HashMap, HashSet};

    #[test]
    fn add_cell_has_cell_kind() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        assert_eq!(sheet.cell_kind(a), Some(CellKind::Cell));
    }

    #[test]
    fn add_source_has_source_kind() {
        let mut sheet = Sheet::new();
        let a = sheet.add_source(0_i32);
        assert_eq!(sheet.cell_kind(a), Some(CellKind::Source));
    }

    #[test]
    fn cell_kind_returns_none_for_invalid_id() {
        let mut sheet = Sheet::new();
        sheet.add_cell(0_i32); // occupies slotmap index 0 in `sheet`
        let mut other = Sheet::new();
        other.add_cell(0_i32); // index 0 in `other`
        let bogus = other.add_cell(0_i32); // index 1 in `other` -- out of range for `sheet`,
        // which only ever allocated index 0, so this is
        // guaranteed invalid regardless of generation
        // (a same-index key from a second fresh SlotMap
        // would otherwise collide with `sheet`'s own key)
        assert_eq!(sheet.cell_kind(bogus), None);
    }

    #[test]
    fn execute_plan_clears_a_stale_derived_override_on_a_later_plain_write() {
        // Simulates two execute_plan calls within one propagate() (e.g. Phase 1 then
        // Phase 3) without an intervening Phase 0 reset: the first shadows `x` via a
        // self-referencing method; the second claims `x` as a plain (non-self,
        // non-conditional) output of a different relationship. `x`'s stale `derived`
        // override from the first call must not survive to mask the second call's
        // fresh `source` write.
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(1_i32);
        let y = sheet.add_cell(2_i32);
        let z = sheet.add_cell(10_i32);
        let self_ref = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([x, y], x, |a: &i32, b: &i32| Ok((*a).min(*b))),
                Method::from_fn_2_1([x, y], y, |a: &i32, b: &i32| Ok((*a).max(*b))),
            ])
            .unwrap();
        let plain = sheet
            .add_relationship(vec![Method::from_fn_1_1(z, x, |v: &i32| Ok(*v + 1))])
            .unwrap();

        let no_seeds: Seeds = HashMap::new();
        let no_forced: HashSet<CellId> = HashSet::new();
        sheet
            .execute_plan(
                &[PlanStep::Method(self_ref, 0)],
                &no_seeds,
                &no_forced,
                &mut Vec::new(),
            )
            .unwrap();
        assert_eq!(*sheet.read::<i32>(x).unwrap(), 1);

        sheet.write(z, 41_i32).unwrap();
        sheet
            .execute_plan(
                &[PlanStep::Method(plain, 0)],
                &no_seeds,
                &no_forced,
                &mut Vec::new(),
            )
            .unwrap();

        assert_eq!(*sheet.read::<i32>(x).unwrap(), 42);
    }

    #[test]
    fn add_conditional_returns_error_for_invalid_cell() {
        let mut sheet = Sheet::new();
        let result = sheet.add_conditional(
            MatchExpr::cell(CellId::default()),
            vec![(vec![0_i32], vec![])],
            vec![],
        );
        assert!(matches!(result, Err(Error::InvalidId)));
    }

    #[test]
    fn add_conditional_returns_invalid_conditional_for_type_mismatch() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        // Branch keys are f64 but cell holds i32.
        let result =
            sheet.add_conditional(MatchExpr::cell(a), vec![(vec![0.0_f64], vec![])], vec![]);
        assert!(matches!(result, Err(Error::InvalidConditional { .. })));
    }

    #[test]
    fn add_conditional_returns_invalid_conditional_for_missing_relationship() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let result = sheet.add_conditional(
            MatchExpr::cell(a),
            vec![(vec![0_i32], vec![RelationshipId::default()])],
            vec![],
        );
        assert!(matches!(result, Err(Error::InvalidConditional { .. })));
    }

    #[test]
    fn validate_returns_dependency_cycle_for_multi_method_relationship_involving_match_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        // Relationship has two methods and involves `a` (the match cell).
        let rel = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
                Method::from_fn_1_1(b, a, |x: &i32| Ok(*x)),
            ])
            .unwrap();
        sheet
            .add_conditional(MatchExpr::cell(a), vec![(vec![0_i32], vec![rel])], vec![])
            .unwrap();
        assert!(matches!(
            sheet.validate(),
            Err(Error::DependencyCycle { .. })
        ));
    }

    #[test]
    fn dependency_cycle_from_a_conditional_names_the_match_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        // Relationship has two methods and involves `a` (the match cell).
        let rel = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
                Method::from_fn_1_1(b, a, |x: &i32| Ok(*x)),
            ])
            .unwrap();
        sheet
            .add_conditional(MatchExpr::cell(a), vec![(vec![0_i32], vec![rel])], vec![])
            .unwrap();
        let err = sheet.validate().unwrap_err();
        assert!(err.sites().contains(&ErrorSite::Cell(a)));
    }

    #[test]
    fn add_conditional_returns_error_when_branch_rel_writes_a_cell_upstream_of_match_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let p = sheet.add_cell(0_i32);
        // Unconditional: a → p  (a contributes to match cell p).
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, p, |x: &i32| Ok(*x))])
            .unwrap();
        // Branch relationship has two methods and involves `a`, which feeds p.
        let rel = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
                Method::from_fn_1_1(b, a, |x: &i32| Ok(*x)),
            ])
            .unwrap();
        sheet
            .add_conditional(MatchExpr::cell(p), vec![(vec![0_i32], vec![rel])], vec![])
            .unwrap();
        assert!(matches!(
            sheet.validate(),
            Err(Error::DependencyCycle { .. })
        ));
    }

    #[test]
    fn add_conditional_returns_error_when_branch_rel_writes_a_cell_upstream_of_either_expr_input() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let p = sheet.add_cell(0_i32);
        let q = sheet.add_cell(0_i32);
        // Unconditional: a → q  (a contributes to expr input q).
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, q, |x: &i32| Ok(*x))])
            .unwrap();
        // Branch relationship has two methods and involves `a`, which feeds q, one of the
        // match expression's two inputs (p, q).
        let rel = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
                Method::from_fn_1_1(b, a, |x: &i32| Ok(*x)),
            ])
            .unwrap();
        let expr = MatchExpr::from_fn_2([p, q], |x: &i32, y: &i32| Ok(*x + *y));
        sheet
            .add_conditional(expr, vec![(vec![0_i32], vec![rel])], vec![])
            .unwrap();
        assert!(matches!(
            sheet.validate(),
            Err(Error::DependencyCycle { .. })
        ));
    }

    #[test]
    fn add_conditional_activates_branch_from_two_cell_expression() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(false);
        let b = sheet.add_cell(false);
        let x = sheet.add_cell(0_i32);
        let y = sheet.add_cell(0_i32);
        let rel_true = sheet
            .add_relationship(vec![Method::from_fn_1_1(x, y, |v: &i32| Ok(*v))])
            .unwrap();
        let expr = MatchExpr::from_fn_2([a, b], |p: &bool, q: &bool| Ok(*p && *q));
        let cid = sheet
            .add_conditional(expr, vec![(vec![true], vec![rel_true])], vec![])
            .unwrap();

        sheet.write(a, true).unwrap();
        sheet.write(b, false).unwrap();
        assert_eq!(sheet.conditional_active_branch(cid).unwrap(), None);

        sheet.write(b, true).unwrap();
        assert_eq!(sheet.conditional_active_branch(cid).unwrap(), Some(0));
        assert_eq!(sheet.conditional_match_cells(cid).unwrap(), &[a, b]);
    }

    #[test]
    fn add_conditional_returns_invalid_conditional_for_expr_output_type_mismatch() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        // Expression computes an i32, but branch keys below are f64.
        let expr = MatchExpr::from_fn_2([a, b], |x: &i32, y: &i32| Ok(x + y));
        let result = sheet.add_conditional::<f64>(expr, vec![(vec![0.0], vec![])], vec![]);
        assert!(matches!(result, Err(Error::InvalidConditional { .. })));
    }

    #[test]
    fn add_conditional_returns_invalid_id_for_bad_expr_input_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let expr = MatchExpr::from_fn_2([a, CellId::default()], |x: &i32, y: &i32| Ok(x + y));
        let result = sheet.add_conditional::<i32>(expr, vec![], vec![]);
        assert!(matches!(result, Err(Error::InvalidId)));
    }

    #[test]
    fn propagate_surfaces_method_failed_from_a_failing_match_expression() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let expr = MatchExpr::from_fn_1(a, |_x: &i32| -> Result<i32, anyhow::Error> {
            Err(anyhow::anyhow!("boom"))
        });
        sheet
            .add_conditional::<i32>(expr, vec![(vec![0], vec![])], vec![])
            .unwrap();
        let result = sheet.propagate();
        assert!(matches!(result, Err(Error::MethodFailed { .. })));
    }

    #[test]
    fn add_conditional_allows_multi_method_rel_not_involving_match_cell() {
        let mut sheet = Sheet::new();
        let mode = sheet.add_cell(0_i32);
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        // Relationship has two methods but does not involve `mode` (the match cell).
        // Branch relationships whose outputs do not reach the match cell may have any
        // number of methods.
        let rel = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
                Method::from_fn_1_1(b, a, |x: &i32| Ok(*x)),
            ])
            .unwrap();
        let result = sheet.add_conditional(
            MatchExpr::cell(mode),
            vec![(vec![0_i32], vec![rel])],
            vec![],
        );
        assert!(result.is_ok());
    }

    #[test]
    fn add_conditional_returns_invalid_conditional_for_empty_branch_keys() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        // Empty key list is invalid.
        let result =
            sheet.add_conditional::<i32>(MatchExpr::cell(a), vec![(vec![], vec![rel])], vec![]);
        assert!(matches!(result, Err(Error::InvalidConditional { .. })));
    }

    #[test]
    fn add_conditional_returns_invalid_conditional_for_duplicate_relationship_across_branches() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        // Add rel to the first conditional.
        sheet
            .add_conditional(MatchExpr::cell(a), vec![(vec![0_i32], vec![rel])], vec![])
            .unwrap();
        // Try to add the same rel to a second conditional.
        let result =
            sheet.add_conditional(MatchExpr::cell(a), vec![(vec![1_i32], vec![rel])], vec![]);
        assert!(matches!(result, Err(Error::InvalidConditional { .. })));
    }

    #[test]
    fn invalid_conditional_duplicate_relationship_names_that_relationship() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let dup_rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        // Add dup_rel to the first conditional.
        sheet
            .add_conditional(
                MatchExpr::cell(a),
                vec![(vec![0_i32], vec![dup_rel])],
                vec![],
            )
            .unwrap();
        // Try to add the same relationship to a second conditional.
        let result = sheet.add_conditional(
            MatchExpr::cell(a),
            vec![(vec![1_i32], vec![dup_rel])],
            vec![],
        );
        let err = result.unwrap_err();
        assert!(matches!(err, Error::InvalidConditional { .. }));
        assert!(err.sites().contains(&ErrorSite::Relationship(dup_rel)));
    }

    #[test]
    fn add_conditional_returns_id_for_valid_input() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let cid = sheet
            .add_conditional(MatchExpr::cell(a), vec![(vec![0_i32], vec![rel])], vec![])
            .unwrap();
        // ConditionalId must be a live key.
        let _ = cid; // just check it compiles and succeeds
    }

    #[test]
    fn add_cell_returns_distinct_ids() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(1_i32);
        let b = sheet.add_cell(2_i32);
        assert_ne!(a, b);
    }

    #[test]
    fn write_returns_invalid_cell_kind_for_an_output_cell() {
        let mut sheet = Sheet::new();
        let writer_input = sheet.add_cell(1_i32);
        let out_cell = sheet.add_cell(0_i32);
        let out = sheet
            .add_out(
                Method::from_fn_1_1(writer_input, out_cell, |x: &i32| Ok(*x)),
                vec![],
            )
            .unwrap();
        assert!(matches!(
            sheet.write(out, 5_i32),
            Err(Error::InvalidCellKind { .. })
        ));
    }

    #[test]
    fn add_relationship_returns_invalid_cell_kind_when_a_source_cell_is_an_output() {
        let mut sheet = Sheet::new();
        let a = sheet.add_source(0_i32);
        let b = sheet.add_cell(0_i32);
        let result = sheet.add_relationship(vec![Method::from_fn_1_1(b, a, |x: &i32| Ok(*x))]);
        assert!(matches!(result, Err(Error::InvalidCellKind { .. })));
        assert_eq!(
            result.unwrap_err().sites().first().copied(),
            Some(ErrorSite::MethodIndex(0))
        );
    }

    #[test]
    fn invalid_cell_kind_names_the_method_and_the_source_output_cell() {
        let mut sheet = Sheet::new();
        let s = sheet.add_source(0_i32); // Source-kind cell
        // A Source-kind output is rejected.
        let err = sheet
            .add_relationship(vec![Method::from_fn_1_1(s, s, |x: &i32| Ok(*x))])
            .unwrap_err();
        let sites = err.sites();
        assert_eq!(sites[0], ErrorSite::MethodIndex(0));
        assert!(sites[1..].contains(&ErrorSite::Cell(s)));
    }

    #[test]
    fn add_relationship_allows_a_source_cell_as_an_input() {
        let mut sheet = Sheet::new();
        let a = sheet.add_source(5_i32);
        let b = sheet.add_cell(0_i32);
        let result = sheet.add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))]);
        assert!(result.is_ok());
    }

    #[test]
    fn write_succeeds_on_a_source_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_source(0_i32);
        assert!(sheet.write(a, 5_i32).is_ok());
    }

    #[test]
    fn error_variant_is_invalid_cell_kind_not_terminal_cell() {
        // Compile-time check that the rename landed; exercised for real once Task 3
        // wires up the CellKind-based checks that actually return this variant.
        let _err = Error::InvalidCellKind { sites: vec![] };
    }

    #[test]
    fn error_has_invalid_requirement_variant() {
        let _err = Error::InvalidRequirement;
    }

    #[test]
    fn write_read_roundtrip() {
        let mut sheet = Sheet::new();
        let id = sheet.add_cell(42_i32);
        sheet.write(id, 99_i32).unwrap();
        assert_eq!(*sheet.read::<i32>(id).unwrap(), 99);
    }

    #[test]
    fn write_wrong_type_returns_type_mismatch() {
        let mut sheet = Sheet::new();
        let id = sheet.add_cell(0_i32);
        assert!(matches!(
            sheet.write(id, 1.0_f64),
            Err(Error::TypeMismatch { .. })
        ));
    }

    #[test]
    fn read_wrong_type_returns_type_mismatch() {
        let mut sheet = Sheet::new();
        let id = sheet.add_cell(0_i32);
        assert!(matches!(
            sheet.read::<f64>(id),
            Err(Error::TypeMismatch { .. })
        ));
    }

    #[test]
    fn source_matches_read_for_a_plain_unshadowed_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(3_i32);
        assert_eq!(*sheet.source::<i32>(a).unwrap(), 3);
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 3);

        sheet.write(a, 8_i32).unwrap();
        assert_eq!(*sheet.source::<i32>(a).unwrap(), 8);
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 8);
    }

    #[test]
    fn source_returns_invalid_id_for_unknown_cell() {
        let sheet = Sheet::new();
        assert!(matches!(
            sheet.source::<i32>(CellId::default()),
            Err(Error::InvalidId)
        ));
    }

    #[test]
    fn source_wrong_type_returns_type_mismatch() {
        let mut sheet = Sheet::new();
        let id = sheet.add_cell(0_i32);
        assert!(matches!(
            sheet.source::<f64>(id),
            Err(Error::TypeMismatch { .. })
        ));
    }

    #[test]
    fn add_relationship_empty_methods_returns_invalid_method() {
        let mut sheet = Sheet::new();
        let result = sheet.add_relationship(vec![]);
        assert!(matches!(result, Err(Error::InvalidMethod { .. })));
        assert!(result.unwrap_err().sites().is_empty());
    }

    #[test]
    fn add_relationship_type_mismatch_returns_error() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        // Method declares f64 input but cell holds i32.
        let method = Method::from_fn_1_1(a, b, |x: &f64| Ok(*x * 2.0));
        let result = sheet.add_relationship(vec![method]);
        assert!(matches!(result, Err(Error::TypeMismatch { .. })));
        assert_eq!(
            result.unwrap_err().sites().first().copied(),
            Some(ErrorSite::MethodIndex(0))
        );
    }

    #[test]
    fn execute_plan_type_mismatch_reports_the_method_that_produced_it() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(false); // bool cell
        // Declares an i32 output (matching nothing about `b`'s actual bool type at
        // add_relationship time -- add_relationship validates output_types against the cell,
        // so declare bool here to pass that check, then lie about it at runtime below).
        let method = Method::new(
            vec![a],
            vec![b],
            vec![TypeId::of::<i32>()],
            vec![TypeId::of::<bool>()],
            |args| {
                let x = *args[0].downcast_ref::<i32>().unwrap();
                // Lies: returns an i32 though output_types declared bool, reproducing the
                // execute_plan runtime type-mismatch path (add_relationship can't catch this --
                // it only checks the declared TypeId, not what the closure actually returns).
                Ok(vec![Box::new(x) as Box<dyn std::any::Any>])
            },
        );
        let rel_id = sheet.add_relationship(vec![method]).unwrap();
        let result = sheet.propagate();
        assert!(matches!(result, Err(Error::TypeMismatch { .. })));
        assert_eq!(
            result.unwrap_err().sites().first().copied(),
            Some(ErrorSite::Method(rel_id, 0))
        );
    }

    #[test]
    fn execute_plan_method_failed_reports_the_method_that_produced_it() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let method = Method::from_fn_1_1(a, b, |_: &i32| -> Result<i32, anyhow::Error> {
            Err(anyhow::anyhow!("boom"))
        });
        let rel_id = sheet.add_relationship(vec![method]).unwrap();
        let result = sheet.propagate();
        assert!(matches!(result, Err(Error::MethodFailed { .. })));
        assert_eq!(
            result.unwrap_err().sites().first().copied(),
            Some(ErrorSite::Method(rel_id, 0))
        );
    }

    #[test]
    fn add_relationship_zero_input_method_defines_a_fixed_point() {
        // A method with no inputs is a valid degenerate case: it always produces the
        // same value, independent of every other cell in the sheet.
        let mut sheet = Sheet::new();
        let b = sheet.add_cell(0_i32);
        let method = Method::new(vec![], vec![b], vec![], vec![TypeId::of::<i32>()], |_| {
            Ok(vec![Box::new(42_i32)])
        });
        let rel = sheet.add_relationship(vec![method]).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 42);
        assert!(sheet.is_forced(b));
        assert!(sheet.is_relationship_forced(rel));
    }

    #[test]
    fn add_relationship_empty_outputs_returns_invalid_method() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let method = Method::new(
            vec![a],
            vec![], // no outputs
            vec![TypeId::of::<i32>()],
            vec![],
            |_| Ok(vec![]),
        );
        let result = sheet.add_relationship(vec![method]);
        assert!(matches!(result, Err(Error::InvalidMethod { .. })));
        assert_eq!(
            result.unwrap_err().sites().first().copied(),
            Some(ErrorSite::MethodIndex(0))
        );
    }

    #[test]
    fn add_relationship_consistent_cell_sets_across_methods_succeeds() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        // Triangle relationship: every method references {a, b, c}.
        let result = sheet.add_relationship(vec![
            Method::from_fn_2_1([a, b], c, |x: &i32, y: &i32| Ok(x + y)),
            Method::from_fn_2_1([a, c], b, |x: &i32, y: &i32| Ok(y - x)),
            Method::from_fn_2_1([b, c], a, |x: &i32, y: &i32| Ok(y - x)),
        ]);
        assert!(result.is_ok());
    }

    #[test]
    fn add_relationship_inconsistent_cell_sets_across_methods_returns_mismatched_method_cells() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        // Method 0 references {a, b}; method 1 references {b, c} -- inconsistent.
        let result = sheet.add_relationship(vec![
            Method::from_fn_1_1(a, b, |v: &i32| Ok(*v)),
            Method::from_fn_1_1(b, c, |v: &i32| Ok(*v)),
        ]);
        assert!(matches!(result, Err(Error::MismatchedMethodCells { .. })));
        assert_eq!(
            result.unwrap_err().sites().first().copied(),
            Some(ErrorSite::MethodIndex(1))
        );
    }

    #[test]
    fn add_relationship_distinct_output_sets_across_methods_succeeds() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        // Triangle relationship: every method has a distinct single-cell output set
        // ({c}, {b}, {a}).
        let result = sheet.add_relationship(vec![
            Method::from_fn_2_1([a, b], c, |x: &i32, y: &i32| Ok(x + y)),
            Method::from_fn_2_1([a, c], b, |x: &i32, y: &i32| Ok(y - x)),
            Method::from_fn_2_1([b, c], a, |x: &i32, y: &i32| Ok(y - x)),
        ]);
        assert!(result.is_ok());
    }

    #[test]
    fn add_relationship_duplicate_output_set_across_methods_returns_invalid_method_outputs() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        // Both methods reference {a, b} (so the cell-set-consistency check passes) but
        // both output {b} from different inputs -- their output sets are identical,
        // which must be rejected.
        let result = sheet.add_relationship(vec![
            Method::from_fn_2_1([a, b], b, |x: &i32, _y: &i32| Ok(*x)),
            Method::from_fn_2_1([a, b], b, |_x: &i32, y: &i32| Ok(*y)),
        ]);
        assert!(matches!(result, Err(Error::InvalidMethodOutputs { .. })));
        assert_eq!(
            result.unwrap_err().sites().first().copied(),
            Some(ErrorSite::MethodIndex(1))
        );
    }

    #[test]
    fn add_relationship_rejects_strictly_nested_method_outputs() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        // Both methods span {a, b, c}; one outputs {b}, the other outputs {b, c}.
        let result = sheet.add_relationship(vec![
            Method::from_fn_2_1([a, c], b, |x: &i32, y: &i32| Ok(x + y)),
            Method::new(
                vec![a],
                vec![b, c],
                vec![std::any::TypeId::of::<i32>()],
                vec![std::any::TypeId::of::<i32>(), std::any::TypeId::of::<i32>()],
                |args| {
                    let x = *args[0].downcast_ref::<i32>().unwrap();
                    Ok(vec![Box::new(x), Box::new(x)])
                },
            ),
        ]);
        let err = result.unwrap_err();
        assert!(matches!(err, Error::InvalidMethodOutputs { .. }));
        let sites = err.sites();
        assert_eq!(sites[0], ErrorSite::MethodIndex(1));
        assert_eq!(sites[1], ErrorSite::MethodIndex(0));
        assert_eq!(sites[2..], [ErrorSite::Cell(b)]);
    }

    #[test]
    fn add_relationship_accepts_overlapping_non_nested_method_outputs() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        // Both methods span {a, b, c}; outputs overlap at b but neither set nests the other.
        let result = sheet.add_relationship(vec![
            Method::new(
                vec![a, c],
                vec![a, b],
                vec![std::any::TypeId::of::<i32>(), std::any::TypeId::of::<i32>()],
                vec![std::any::TypeId::of::<i32>(), std::any::TypeId::of::<i32>()],
                |args| {
                    let x = *args[0].downcast_ref::<i32>().unwrap();
                    let y = *args[1].downcast_ref::<i32>().unwrap();
                    Ok(vec![Box::new(x), Box::new(y)])
                },
            ),
            Method::new(
                vec![a, b],
                vec![b, c],
                vec![std::any::TypeId::of::<i32>(), std::any::TypeId::of::<i32>()],
                vec![std::any::TypeId::of::<i32>(), std::any::TypeId::of::<i32>()],
                |args| {
                    let x = *args[0].downcast_ref::<i32>().unwrap();
                    let y = *args[1].downcast_ref::<i32>().unwrap();
                    Ok(vec![Box::new(y), Box::new(x)])
                },
            ),
        ]);
        assert!(result.is_ok());
    }

    #[test]
    fn add_relationship_returns_distinct_ids() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let r1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let c = sheet.add_cell(0_i32);
        let r2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(b, c, |x: &i32| Ok(*x))])
            .unwrap();
        assert_ne!(r1, r2);
    }

    #[test]
    fn add_relationship_mismatched_cells_returns_error() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let d = sheet.add_cell(0_i32);
        // Method 0 spans {a, b}; Method 1 spans {c, d} — mismatched cell sets.
        let result = sheet.add_relationship(vec![
            Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
            Method::from_fn_1_1(c, d, |x: &i32| Ok(*x)),
        ]);
        assert!(matches!(result, Err(Error::MismatchedMethodCells { .. })));
        assert_eq!(
            result.unwrap_err().sites().first().copied(),
            Some(ErrorSite::MethodIndex(1))
        );
    }

    #[test]
    fn add_relationship_duplicate_output_sets_across_methods_returns_error() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        // Both methods span {a, b, c} and both output {c} — identical output sets.
        let result = sheet.add_relationship(vec![
            Method::from_fn_2_1([a, b], c, |x: &i32, y: &i32| Ok(*x + *y)),
            Method::from_fn_2_1([a, b], c, |x: &i32, y: &i32| Ok(*x - *y)),
        ]);
        assert!(matches!(result, Err(Error::InvalidMethodOutputs { .. })));
        assert_eq!(
            result.unwrap_err().sites().first().copied(),
            Some(ErrorSite::MethodIndex(1))
        );
    }

    #[test]
    fn add_relationship_duplicate_cell_within_own_outputs_returns_error() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        // The method's own outputs list names `b` twice.
        let method = Method::new(
            vec![a],
            vec![b, b],
            vec![TypeId::of::<i32>()],
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
            |args| {
                let x = args[0].downcast_ref::<i32>().unwrap();
                Ok(vec![Box::new(*x), Box::new(*x)])
            },
        );
        let result = sheet.add_relationship(vec![method]);
        assert!(matches!(result, Err(Error::InvalidMethodOutputs { .. })));
        assert_eq!(
            result.unwrap_err().sites().first().copied(),
            Some(ErrorSite::MethodIndex(0))
        );
    }

    #[test]
    fn mismatched_method_cells_names_both_methods_and_the_differing_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        // method 0 references {a,b}; method 1 references {a,c}: c (and b) diverge.
        let err = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
                Method::from_fn_1_1(a, c, |x: &i32| Ok(*x)),
            ])
            .unwrap_err();
        let sites = err.sites();
        assert_eq!(sites[0], ErrorSite::MethodIndex(1));
        assert_eq!(sites[1], ErrorSite::MethodIndex(0));
        assert!(sites[2..].contains(&ErrorSite::Cell(c)));
    }

    #[test]
    fn duplicate_output_set_across_methods_names_both_methods_and_the_shared_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        // both methods reference {a,b} and both output b.
        let err = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
                Method::from_fn_1_1(a, b, |x: &i32| Ok(*x + 1)),
            ])
            .unwrap_err();
        let sites = err.sites();
        assert_eq!(sites[0], ErrorSite::MethodIndex(1));
        assert_eq!(sites[1], ErrorSite::MethodIndex(0));
        assert!(sites[2..].contains(&ErrorSite::Cell(b)));
    }

    #[test]
    fn duplicate_cell_within_own_outputs_names_the_method_and_the_repeated_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let i32_ty = std::any::TypeId::of::<i32>();
        // one method whose outputs name b twice.
        let err = sheet
            .add_relationship(vec![Method::new(
                vec![a],
                vec![b, b],
                vec![i32_ty],
                vec![i32_ty, i32_ty],
                |args| {
                    let v = *args[0].downcast_ref::<i32>().unwrap();
                    Ok(vec![Box::new(v), Box::new(v)])
                },
            )])
            .unwrap_err();
        let sites = err.sites();
        assert_eq!(sites[0], ErrorSite::MethodIndex(0));
        assert!(sites[1..].contains(&ErrorSite::Cell(b)));
    }

    #[test]
    fn duplicate_cell_repeated_three_times_reports_cell_once() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let i32_ty = std::any::TypeId::of::<i32>();
        // one method whose outputs name b three times.
        let err = sheet
            .add_relationship(vec![Method::new(
                vec![a],
                vec![b, b, b],
                vec![i32_ty],
                vec![i32_ty, i32_ty, i32_ty],
                |args| {
                    let v = *args[0].downcast_ref::<i32>().unwrap();
                    Ok(vec![Box::new(v), Box::new(v), Box::new(v)])
                },
            )])
            .unwrap_err();
        let sites = err.sites();
        assert_eq!(sites[0], ErrorSite::MethodIndex(0));
        let cell_sites: Vec<_> = sites[1..]
            .iter()
            .filter(|&&s| s == ErrorSite::Cell(b))
            .collect();
        assert_eq!(
            cell_sites.len(),
            1,
            "expected exactly one Cell(b) site for a triple repeat, got {sites:?}"
        );
    }

    #[test]
    fn changed_is_empty_before_propagate() {
        let sheet = Sheet::new();
        assert_eq!(sheet.changed().count(), 0);
    }

    #[test]
    fn changed_after_propagate_contains_method_outputs() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        sheet.write(a, 3_i32).unwrap();
        sheet.propagate().unwrap();
        let changed: Vec<_> = sheet.changed().collect();
        assert_eq!(changed, vec![b]);
    }

    #[test]
    fn clear_changed_empties_changed_set() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        sheet.write(a, 3_i32).unwrap();
        sheet.propagate().unwrap();
        sheet.clear_changed();
        assert_eq!(sheet.changed().count(), 0);
    }

    #[test]
    fn propagate_clears_previous_changed_set() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        sheet.write(a, 3_i32).unwrap();
        sheet.propagate().unwrap();
        sheet.write(a, 5_i32).unwrap();
        sheet.propagate().unwrap();
        let changed: Vec<_> = sheet.changed().collect();
        assert_eq!(changed, vec![b]);
    }

    #[test]
    fn cells_returns_all_cell_ids() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let ids: Vec<_> = sheet.cells().collect();
        assert!(ids.contains(&a));
        assert!(ids.contains(&b));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn cells_returns_empty_for_empty_sheet() {
        let sheet = Sheet::new();
        assert_eq!(sheet.cells().count(), 0);
    }

    #[test]
    fn relationships_returns_all_relationship_ids() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let r = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let ids: Vec<_> = sheet.relationships().collect();
        assert_eq!(ids, vec![r]);
    }

    #[test]
    fn relationships_returns_empty_for_empty_sheet() {
        let sheet = Sheet::new();
        assert_eq!(sheet.relationships().count(), 0);
    }

    #[test]
    fn cell_adj_returns_adjacent_relationships() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let r = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        assert!(sheet.cell_adj(a).unwrap().contains(&r));
        assert!(sheet.cell_adj(b).unwrap().contains(&r));
    }

    #[test]
    fn cell_adj_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert!(sheet.cell_adj(CellId::default()).is_none());
    }

    #[test]
    fn relationship_adj_returns_adjacent_cells() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let r = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let adj = sheet.relationship_adj(r).unwrap();
        assert!(adj.contains(&a));
        assert!(adj.contains(&b));
    }

    #[test]
    fn relationship_adj_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert!(sheet.relationship_adj(RelationshipId::default()).is_none());
    }

    #[test]
    fn selected_method_returns_none_before_propagate() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        assert!(sheet.selected_method(rel).is_none());
    }

    #[test]
    fn selected_method_returns_index_after_propagate() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        // Write to `a` so it has the highest strength and becomes the source,
        // making the a → b method eligible.
        sheet.write(a, 0_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(sheet.selected_method(rel), Some(0));
    }

    #[test]
    fn method_inputs_returns_inputs_for_valid_method() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        assert_eq!(sheet.method_inputs(rel, 0), Some([a].as_slice()));
    }

    #[test]
    fn method_outputs_returns_outputs_for_valid_method() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        assert_eq!(sheet.method_outputs(rel, 0), Some([b].as_slice()));
    }

    #[test]
    fn method_inputs_returns_none_for_out_of_bounds_idx() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        assert!(sheet.method_inputs(rel, 99).is_none());
    }

    #[test]
    fn method_outputs_returns_none_for_out_of_bounds_idx() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        assert!(sheet.method_outputs(rel, 99).is_none());
    }

    #[test]
    fn is_source_returns_false_before_propagate() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        assert!(!sheet.is_source(a));
    }

    #[test]
    fn is_source_returns_true_for_input_cell_after_propagate() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        // Write to `a` so it has the highest strength and becomes the source.
        sheet.write(a, 0_i32).unwrap();
        sheet.propagate().unwrap();
        assert!(sheet.is_source(a));
    }

    #[test]
    fn is_source_returns_false_for_output_cell_after_propagate() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        // Write to `a` so it has the highest strength and becomes the source.
        sheet.write(a, 0_i32).unwrap();
        sheet.propagate().unwrap();
        assert!(!sheet.is_source(b));
    }

    // ── Conditional accessor tests ─────────────────────────────────────────

    fn sheet_with_two_branch_conditional() -> (Sheet, ConditionalId) {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let p = sheet.add_cell(0_i32);

        let rel0 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |v: &i32| Ok(*v))])
            .unwrap();
        let rel1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(b, a, |v: &i32| Ok(*v))])
            .unwrap();

        let cid = sheet
            .add_conditional(
                MatchExpr::cell(p),
                vec![(vec![0_i32], vec![rel0]), (vec![1_i32], vec![rel1])],
                vec![],
            )
            .unwrap();
        (sheet, cid)
    }

    fn sheet_with_default_conditional() -> (Sheet, ConditionalId, RelationshipId) {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let p = sheet.add_cell(99_i32); // no branch matches → default

        let rel_default = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |v: &i32| Ok(*v))])
            .unwrap();

        let cid = sheet
            .add_conditional::<i32>(MatchExpr::cell(p), vec![], vec![rel_default])
            .unwrap();
        (sheet, cid, rel_default)
    }

    #[test]
    fn conditionals_returns_registered_id() {
        let (sheet, cid) = sheet_with_two_branch_conditional();
        assert!(sheet.conditionals().any(|id| id == cid));
    }

    #[test]
    fn conditionals_empty_on_new_sheet() {
        let sheet = Sheet::new();
        assert_eq!(sheet.conditionals().count(), 0);
    }

    #[test]
    fn conditional_match_cells_returns_correct_cell() {
        let mut sheet = Sheet::new();
        let p = sheet.add_cell(0_i32);
        let cid = sheet
            .add_conditional::<i32>(MatchExpr::cell(p), vec![], vec![])
            .unwrap();
        assert_eq!(sheet.conditional_match_cells(cid), Some([p].as_slice()));
    }

    #[test]
    fn conditional_match_cells_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert_eq!(
            sheet.conditional_match_cells(ConditionalId::default()),
            None
        );
    }

    #[test]
    fn conditional_branch_count_returns_correct_count() {
        let (sheet, cid) = sheet_with_two_branch_conditional();
        assert_eq!(sheet.conditional_branch_count(cid), Some(2));
    }

    #[test]
    fn conditional_branch_count_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert_eq!(
            sheet.conditional_branch_count(ConditionalId::default()),
            None
        );
    }

    #[test]
    fn conditional_branch_relationships_returns_correct_rels() {
        let (sheet, cid) = sheet_with_two_branch_conditional();
        let rels0 = sheet.conditional_branch_relationships(cid, 0).unwrap();
        let rels1 = sheet.conditional_branch_relationships(cid, 1).unwrap();
        assert_eq!(rels0.len(), 1);
        assert_eq!(rels1.len(), 1);
        assert_ne!(rels0[0], rels1[0]);
    }

    #[test]
    fn conditional_branch_relationships_returns_none_for_out_of_bounds() {
        let (sheet, cid) = sheet_with_two_branch_conditional();
        assert!(sheet.conditional_branch_relationships(cid, 2).is_none());
    }

    #[test]
    fn conditional_branch_relationships_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert!(
            sheet
                .conditional_branch_relationships(ConditionalId::default(), 0)
                .is_none()
        );
    }

    #[test]
    fn conditional_default_relationships_returns_correct_rels() {
        let (sheet, cid, rel_default) = sheet_with_default_conditional();
        let rels = sheet.conditional_default_relationships(cid).unwrap();
        assert_eq!(rels, [rel_default]);
    }

    #[test]
    fn conditional_default_relationships_empty_when_no_default() {
        let (sheet, cid) = sheet_with_two_branch_conditional();
        assert_eq!(sheet.conditional_default_relationships(cid).unwrap(), &[]);
    }

    #[test]
    fn conditional_default_relationships_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert!(
            sheet
                .conditional_default_relationships(ConditionalId::default())
                .is_none()
        );
    }

    #[test]
    fn conditional_active_branch_returns_matching_branch_index() {
        let (mut sheet, cid) = sheet_with_two_branch_conditional();
        let p = sheet.conditional_match_cells(cid).unwrap()[0];
        sheet.write(p, 0_i32).unwrap();
        assert_eq!(sheet.conditional_active_branch(cid).unwrap(), Some(0));
        sheet.write(p, 1_i32).unwrap();
        assert_eq!(sheet.conditional_active_branch(cid).unwrap(), Some(1));
    }

    #[test]
    fn conditional_active_branch_returns_none_when_no_branch_matches() {
        let (mut sheet, cid) = sheet_with_two_branch_conditional();
        let p = sheet.conditional_match_cells(cid).unwrap()[0];
        sheet.write(p, 99_i32).unwrap();
        assert_eq!(sheet.conditional_active_branch(cid).unwrap(), None);
    }

    #[test]
    fn conditional_active_branch_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert_eq!(
            sheet
                .conditional_active_branch(ConditionalId::default())
                .unwrap(),
            None
        );
    }

    #[test]
    fn add_filter_returns_invalid_id_for_missing_cell() {
        let mut sheet = Sheet::new();
        let result = sheet.add_filter(CellId::default(), Filter::from_fn_0(|x: &i32| Ok(*x)));
        assert!(matches!(result, Err(Error::InvalidId)));
    }

    #[test]
    fn add_filter_does_not_change_the_cells_current_value() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(500_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        // add_filter never evaluates the function against the current value: the raw
        // out-of-range value survives until the next propagate().
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 500);
    }

    #[test]
    fn propagate_after_add_filter_conforms_the_initial_value() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(500_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 500);
        sheet.propagate().unwrap();
        // `a` has no filter args and belongs to no relationship — this is the ordinary
        // first-round case of Task 1's fix, not a special "cold start" path.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 100);
    }

    #[test]
    fn add_filter_returns_invalid_filter_when_cell_already_has_a_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok(*x)))
            .unwrap();
        let result = sheet.add_filter(a, Filter::from_fn_0(|x: &i32| Ok(*x)));
        assert!(matches!(result, Err(Error::InvalidFilter)));
    }

    #[test]
    fn add_filter_returns_invalid_filter_for_mismatched_value_type() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let result = sheet.add_filter(a, Filter::from_fn_0(|x: &f64| Ok(*x)));
        assert!(matches!(result, Err(Error::InvalidFilter)));
    }

    #[test]
    fn add_filter_returns_invalid_filter_when_args_name_the_filtered_cell_itself() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let result = sheet.add_filter(
            a,
            Filter::from_fn_1(a, |x: &i32, bound: &i32| Ok((*x).min(*bound))),
        );
        assert!(matches!(result, Err(Error::InvalidFilter)));
    }

    #[test]
    fn add_filter_returns_invalid_id_for_missing_arg_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let result = sheet.add_filter(
            a,
            Filter::from_fn_1(CellId::default(), |x: &i32, bound: &i32| {
                Ok((*x).min(*bound))
            }),
        );
        assert!(matches!(result, Err(Error::InvalidId)));
    }

    #[test]
    fn add_filter_returns_type_mismatch_for_wrong_arg_cell_type() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let bound = sheet.add_cell(1.0_f64); // wrong type: filter declares i32
        let result = sheet.add_filter(
            a,
            Filter::from_fn_1(bound, |x: &i32, bound: &i32| Ok((*x).min(*bound))),
        );
        assert!(matches!(result, Err(Error::TypeMismatch { .. })));
    }

    #[test]
    fn add_filter_succeeds_on_a_source_kind_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_source(5_i32);
        assert!(
            sheet
                .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 10))))
                .is_ok()
        );
    }

    #[test]
    fn from_fn_2_conforms_values_through_sheet_using_both_dynamic_arguments() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(500_i32);
        let lo = sheet.add_cell(0_i32);
        let hi = sheet.add_cell(100_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_2([lo, hi], |x: &i32, lo: &i32, hi: &i32| {
                    Ok((*x).clamp(*lo, *hi))
                }),
            )
            .unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 100);

        sheet.write(a, -10_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 0);
    }

    #[test]
    fn write_without_a_filter_behaves_exactly_as_before() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        sheet.write(a, 42_i32).unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 42);
    }

    #[test]
    fn write_leaves_the_raw_value_in_source_until_propagate_conforms_it() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet.write(a, 500_i32).unwrap();
        // write() no longer runs the filter: the raw value stands until propagate().
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 500);
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 100);
    }

    #[test]
    fn propagate_reports_no_violation_when_a_derived_value_conforms() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(10_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_filter(b, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 10);
        assert!(sheet.last_filter_violations.is_empty());
    }

    #[test]
    fn propagate_reports_not_conformed_when_a_derived_value_violates_its_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(60_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_filter(b, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        sheet.propagate().unwrap();
        // 60 * 2 = 120, clamp(0, 100) => 100 != 120.
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 120);
        assert!(matches!(
            sheet.last_filter_violations.get(&b),
            Some(FilterViolation::NotConformed)
        ));
    }

    #[test]
    fn propagate_reports_failed_when_the_filter_errors_on_a_derived_value() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(1_i32);
        let b = sheet.add_cell(0_i32);
        // Accept exactly 0 (b's initial value) so this filter's shape is exercised
        // only by the relationship's derived value (1, copied from `a`), not by
        // anything add_filter itself does — add_filter no longer evaluates a filter's
        // function at all.
        sheet
            .add_filter(
                b,
                Filter::from_fn_0(|x: &i32| {
                    if *x == 0 {
                        Ok(*x)
                    } else {
                        Err(anyhow::anyhow!("cannot conform"))
                    }
                }),
            )
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        // propagate() must not abort even though the filter errors.
        sheet.propagate().unwrap();
        assert!(matches!(
            sheet.last_filter_violations.get(&b),
            Some(FilterViolation::Failed(_))
        ));
    }

    #[test]
    fn propagate_reports_failed_when_the_filter_returns_the_wrong_type_on_a_derived_value() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(1_i32);
        let b = sheet.add_cell(0_i32);
        // Conforms correctly for the attach-time value (b's initial 0), so
        // `add_filter` succeeds, but returns a `f64` for any other input — tripping
        // propagate()'s diagnostic-phase defensive check once `a`'s value (1) is
        // copied into `b` this round.
        let filter = Filter::new(TypeId::of::<i32>(), vec![], vec![], |value, _args| {
            let v = *value.downcast_ref::<i32>().unwrap();
            if v == 0 {
                Ok(Box::new(v) as Box<dyn Any>)
            } else {
                Ok(Box::new(1.5_f64) as Box<dyn Any>)
            }
        });
        sheet.add_filter(b, filter).unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        // propagate() must not abort even though the filter's function returns the
        // wrong type.
        sheet.propagate().unwrap();
        assert!(matches!(
            sheet.filter_violation(b),
            Some(FilterViolation::Failed(_))
        ));
    }

    #[test]
    fn propagate_never_flags_a_filtered_cell_that_stayed_a_plain_source() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(60_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet.propagate().unwrap();
        assert!(sheet.last_filter_violations.is_empty());
    }

    #[test]
    fn propagate_reclamps_a_filtered_source_cell_when_its_argument_changes() {
        // Issue #132's exact repro.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(50_i32);
        let bound = sheet.add_cell(100_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |v: &i32, b: &i32| Ok((*v).min(*b))),
            )
            .unwrap();
        sheet.write(bound, 10_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 10);
    }

    #[test]
    fn propagate_reclamps_before_a_relationship_consumes_the_reclamped_value() {
        // The inequality.adm2-shaped case: a and b are linked by a two-method mutual
        // relationship (b := min(a, b); a := max(a, b)); a is the currently-source cell
        // of the pair and is filtered against a bound that just shrank. b (derived) must
        // reflect the corrected a, not the pre-reclamp one, within a single propagate().
        //
        // b is created before a so that a (created later) outranks b in strength —
        // release::resolve keeps the higher-strength cell a source (see
        // release::tests::strength_prefers_the_higher_strength_cell_as_source) — making a
        // the source and b the derived cell of the pair, as this test needs.
        let mut sheet = Sheet::new();
        let b = sheet.add_cell(20_i32);
        let a = sheet.add_cell(50_i32);
        let bound = sheet.add_cell(100_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |v: &i32, bnd: &i32| Ok((*v).min(*bnd))),
            )
            .unwrap();
        sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], b, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        sheet.propagate().unwrap();

        sheet.write(bound, 5_i32).unwrap();
        sheet.propagate().unwrap();

        // a reclamps to min(50, 5) = 5; b's method (a.min(b)) then reads the reclamped
        // a, not the stale 50.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 5);
    }

    #[test]
    fn filtered_source_cell_springs_back_to_its_original_value_when_a_bound_loosens() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(50_i32);
        let bound = sheet.add_cell(100_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |v: &i32, b: &i32| Ok((*v).min(*b))),
            )
            .unwrap();

        sheet.write(bound, 10_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 10);

        sheet.write(bound, 100_i32).unwrap();
        sheet.propagate().unwrap();
        // a's original 50 must survive in `source` across the whole round-trip: it
        // springs back once the bound loosens again, rather than staying stuck at the
        // intermediate clamp.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 50);
    }

    #[test]
    fn filter_reclamp_records_failed_violation_when_the_filters_function_returns_the_wrong_type() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let trigger = sheet.add_cell(0_i32);
        // Conforms correctly for the attach-time values (a's initial 5 and trigger's 0), so
        // `add_filter` succeeds, but returns an f64 when trigger becomes non-zero —
        // tripping FilterReclamp's check once `trigger` is updated.
        let filter = Filter::new(
            TypeId::of::<i32>(),
            vec![trigger],
            vec![TypeId::of::<i32>()],
            |value, args| {
                let v = *value.downcast_ref::<i32>().unwrap();
                let t = *args[0].downcast_ref::<i32>().unwrap();
                if t == 0 {
                    Ok(Box::new(v) as Box<dyn Any>)
                } else {
                    Ok(Box::new(1.5_f64) as Box<dyn Any>)
                }
            },
        );
        sheet.add_filter(a, filter).unwrap();

        // trigger changes, causing reclamp where the filter returns wrong type
        sheet.write(trigger, 1_i32).unwrap();
        sheet.propagate().unwrap();

        assert!(matches!(
            sheet.filter_violation(a),
            Some(FilterViolation::Failed(_))
        ));
        // The wrong-type result is discarded: the cell's stored value is unchanged.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    }

    #[test]
    fn filter_reclamp_failure_is_recorded_without_aborting_propagate_or_changing_the_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let bound = sheet.add_cell(100_i32);
        // Accept anything up to `bound` (a's initial 5 is within bound's initial 100);
        // the write to `bound` below is what trips the filter's next live reclamp.
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |v: &i32, b: &i32| {
                    if *v <= *b {
                        Ok(*v)
                    } else {
                        Err(anyhow::anyhow!("cannot conform"))
                    }
                }),
            )
            .unwrap();
        sheet.write(bound, 0_i32).unwrap();

        sheet.propagate().unwrap();

        assert!(matches!(
            sheet.filter_violation(a),
            Some(FilterViolation::Failed(_))
        ));
        // Rejected reclamp: the cell's stored value is left completely unchanged.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    }

    #[test]
    fn filter_args_returns_the_filters_argument_cells() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let bound = sheet.add_cell(10_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |x: &i32, bound: &i32| Ok((*x).min(*bound))),
            )
            .unwrap();
        assert_eq!(sheet.filter_args(a), Some(&[bound][..]));
    }

    #[test]
    fn filter_args_returns_none_for_a_cell_with_no_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        assert_eq!(sheet.filter_args(a), None);
    }

    #[test]
    fn filter_args_returns_none_for_an_invalid_cell() {
        let sheet = Sheet::new();
        assert_eq!(sheet.filter_args(CellId::default()), None);
    }

    #[test]
    fn filter_violation_returns_none_before_any_propagate() {
        let sheet = Sheet::new();
        assert!(sheet.filter_violation(CellId::default()).is_none());
    }

    #[test]
    fn filter_violated_cells_reports_a_currently_violated_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(60_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_filter(b, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        sheet.propagate().unwrap();
        assert!(sheet.filter_violated_cells().any(|id| id == b));
        assert!(matches!(
            sheet.filter_violation(b),
            Some(FilterViolation::NotConformed)
        ));
    }

    #[test]
    fn filter_violation_cells_is_empty_when_nothing_is_violated() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(10_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet.propagate().unwrap();
        assert!(sheet.filter_violation_cells().is_empty());
    }

    #[test]
    fn filter_violation_cells_includes_root_causes_of_a_violation() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(60_i32);
        let bound = sheet.add_cell(100_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_filter(
                b,
                Filter::from_fn_1(bound, |x: &i32, bound: &i32| Ok((*x).min(*bound))),
            )
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        sheet.propagate().unwrap();
        let violation_cells = sheet.filter_violation_cells();
        // `b` is forced (its relationship has only one method), so — mirroring
        // `contributing_cells`'s existing semantics — it is `a` and `bound` that
        // appear as the upstream root causes, not `b` itself. `b`'s own membership
        // is already answered by `filter_violated_cells()`, tested separately above.
        assert!(violation_cells.contains(&a));
        assert!(violation_cells.contains(&bound));
    }

    #[test]
    fn filter_violation_cells_includes_root_causes_of_a_failed_violation() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(1_i32);
        let bound = sheet.add_cell(100_i32);
        let b = sheet.add_cell(0_i32);
        // Accepts b's attach-time value (0) so add_filter succeeds, but errors on any
        // other input so the relationship's derived value (copied from `a`) trips a
        // `Failed` violation instead of `NotConformed`.
        sheet
            .add_filter(
                b,
                Filter::from_fn_1(bound, |x: &i32, _bound: &i32| {
                    if *x == 0 {
                        Ok(*x)
                    } else {
                        Err(anyhow::anyhow!("cannot conform"))
                    }
                }),
            )
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        sheet.propagate().unwrap();
        assert!(matches!(
            sheet.filter_violation(b),
            Some(FilterViolation::Failed(_))
        ));
        let violation_cells = sheet.filter_violation_cells();
        // Mirroring the `NotConformed` case above: `b` is forced, so `a` and `bound`
        // are the upstream root causes, not `b` itself.
        assert!(violation_cells.contains(&a));
        assert!(violation_cells.contains(&bound));
    }

    #[test]
    fn filter_dependents_returns_the_cells_whose_filter_references_this_one() {
        let mut sheet = Sheet::new();
        let bound = sheet.add_cell(10_i32);
        let a = sheet.add_cell(5_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |v: &i32, b: &i32| Ok((*v).min(*b))),
            )
            .unwrap();
        assert_eq!(sheet.filter_dependents(bound), &[a]);
    }

    #[test]
    fn filter_dependents_is_empty_for_a_cell_no_filter_references() {
        let mut sheet = Sheet::new();
        let bound = sheet.add_cell(10_i32);
        let a = sheet.add_cell(5_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|v: &i32| Ok((*v).clamp(0, 100))))
            .unwrap();
        assert!(sheet.filter_dependents(bound).is_empty());
    }

    #[test]
    fn filter_dependents_is_empty_for_an_invalid_cell() {
        let sheet = Sheet::new();
        assert!(sheet.filter_dependents(CellId::default()).is_empty());
    }

    #[test]
    fn filter_dependents_aggregates_multiple_dependents_of_the_same_argument() {
        let mut sheet = Sheet::new();
        let bound = sheet.add_cell(10_i32);
        let a = sheet.add_cell(5_i32);
        let b = sheet.add_cell(5_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |v: &i32, bd: &i32| Ok((*v).min(*bd))),
            )
            .unwrap();
        sheet
            .add_filter(
                b,
                Filter::from_fn_1(bound, |v: &i32, bd: &i32| Ok((*v).min(*bd))),
            )
            .unwrap();
        let dependents = sheet.filter_dependents(bound);
        assert_eq!(dependents.len(), 2);
        assert!(dependents.contains(&a));
        assert!(dependents.contains(&b));
    }

    #[test]
    fn filter_kind_returns_none_for_a_cell_with_no_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        assert!(sheet.filter_kind(a).is_none());
    }

    #[test]
    fn filter_kind_returns_opaque_for_a_plain_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok(*x)))
            .unwrap();
        assert!(matches!(sheet.filter_kind(a), Some(FilterKind::Opaque)));
    }

    #[test]
    fn filter_kind_returns_range_for_a_range_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let filter = Filter::range(
            TypeId::of::<i32>(),
            vec![],
            vec![],
            |value, _args| Ok(Box::new(*value.downcast_ref::<i32>().unwrap()) as Box<dyn Any>),
            |_args| {
                Some((
                    Box::new(0i32) as Box<dyn Any>,
                    Box::new(100i32) as Box<dyn Any>,
                ))
            },
        );
        sheet.add_filter(a, filter).unwrap();
        assert!(matches!(
            sheet.filter_kind(a),
            Some(FilterKind::Range { .. })
        ));
    }

    #[test]
    fn filter_range_returns_live_bounds_from_argument_cells() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let lo = sheet.add_cell(0_i32);
        let hi = sheet.add_cell(100_i32);
        let filter = Filter::range(
            TypeId::of::<i32>(),
            vec![lo, hi],
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
            |value, args| {
                let v = *value.downcast_ref::<i32>().unwrap();
                let lo = *args[0].downcast_ref::<i32>().unwrap();
                let hi = *args[1].downcast_ref::<i32>().unwrap();
                Ok(Box::new(v.clamp(lo, hi)) as Box<dyn Any>)
            },
            |args| {
                Some((
                    Box::new(*args[0].downcast_ref::<i32>().unwrap()) as Box<dyn Any>,
                    Box::new(*args[1].downcast_ref::<i32>().unwrap()) as Box<dyn Any>,
                ))
            },
        );
        sheet.add_filter(a, filter).unwrap();
        assert_eq!(sheet.filter_range::<i32>(a), Some((0, 100)));
        sheet.write(hi, 10_i32).unwrap();
        assert_eq!(sheet.filter_range::<i32>(a), Some((0, 10)));
    }

    #[test]
    fn filter_range_reflects_a_bound_derived_by_a_relationship_not_just_a_direct_write() {
        // `hi` isn't itself written — its value is derived from `hi_source` via a relationship —
        // exercising `filter_range`'s use of `effective()` (which sees a relationship's derived
        // override), not just `source`.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let lo = sheet.add_cell(0_i32);
        let hi = sheet.add_cell(100_i32);
        let hi_source = sheet.add_cell(100_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(hi_source, hi, |v: &i32| Ok(*v))])
            .unwrap();
        let filter = Filter::range(
            TypeId::of::<i32>(),
            vec![lo, hi],
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
            |value, args| {
                let v = *value.downcast_ref::<i32>().unwrap();
                let lo = *args[0].downcast_ref::<i32>().unwrap();
                let hi = *args[1].downcast_ref::<i32>().unwrap();
                Ok(Box::new(v.clamp(lo, hi)) as Box<dyn Any>)
            },
            |args| {
                Some((
                    Box::new(*args[0].downcast_ref::<i32>().unwrap()) as Box<dyn Any>,
                    Box::new(*args[1].downcast_ref::<i32>().unwrap()) as Box<dyn Any>,
                ))
            },
        );
        sheet.add_filter(a, filter).unwrap();
        sheet.write(hi_source, 20_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(sheet.filter_range::<i32>(a), Some((0, 20)));
    }

    #[test]
    fn filter_range_returns_none_for_an_opaque_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok(*x)))
            .unwrap();
        assert!(sheet.filter_range::<i32>(a).is_none());
    }

    #[test]
    fn add_requirement_succeeds_on_a_plain_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let result = sheet.add_requirement(
            a,
            Some("positive"),
            Requirement::from_fn_1(a, |x: &i32| Ok(*x > 0)),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn add_requirement_returns_invalid_requirement_for_duplicate_name_on_same_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        sheet
            .add_requirement(
                a,
                Some("positive"),
                Requirement::from_fn_1(a, |x: &i32| Ok(*x > 0)),
            )
            .unwrap();
        let result = sheet.add_requirement(
            a,
            Some("positive"),
            Requirement::from_fn_1(a, |x: &i32| Ok(*x < 100)),
        );
        assert!(matches!(result, Err(Error::InvalidRequirement)));
    }

    #[test]
    fn add_requirement_allows_two_unnamed_requirements_on_the_same_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        sheet
            .add_requirement(a, None, Requirement::from_fn_1(a, |x: &i32| Ok(*x > 0)))
            .unwrap();
        let result =
            sheet.add_requirement(a, None, Requirement::from_fn_1(a, |x: &i32| Ok(*x < 100)));
        assert!(result.is_ok());
    }

    #[test]
    fn add_requirement_hard_fails_when_current_value_already_violates_it_on_a_plain_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(-5_i32);
        let result = sheet.add_requirement(
            a,
            Some("positive"),
            Requirement::from_fn_1(a, |x: &i32| Ok(*x > 0)),
        );
        assert!(matches!(result, Err(Error::InvalidRequirement)));
    }

    #[test]
    fn add_requirement_hard_fails_when_current_value_already_violates_it_on_a_source_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_source(-5_i32);
        let result = sheet.add_requirement(
            a,
            Some("positive"),
            Requirement::from_fn_1(a, |x: &i32| Ok(*x > 0)),
        );
        assert!(matches!(result, Err(Error::InvalidRequirement)));
    }

    #[test]
    fn add_requirement_propagates_method_failed_when_evaluation_errors() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let result = sheet.add_requirement(
            a,
            Some("always_errors"),
            Requirement::from_fn_1(a, |_: &i32| Err(anyhow::anyhow!("boom"))),
        );
        assert!(matches!(result, Err(Error::MethodFailed { .. })));
    }

    #[test]
    fn cell_has_the_requirement_it_was_given() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let rid = sheet
            .add_requirement(
                a,
                Some("positive"),
                Requirement::from_fn_1(a, |x: &i32| Ok(*x > 0)),
            )
            .unwrap();
        assert_eq!(sheet.cells[a].requirements, vec![rid]);
    }

    #[test]
    fn add_out_returns_the_cell_id_directly() {
        let mut sheet = Sheet::new();
        let width = sheet.add_cell(4_i32);
        let height = sheet.add_cell(5_i32);
        let area = sheet.add_cell(0_i32);
        let cell = sheet
            .add_out(
                Method::from_fn_2_1([width, height], area, |w: &i32, h: &i32| Ok(w * h)),
                vec![],
            )
            .unwrap();
        assert_eq!(cell, area);
        assert_eq!(sheet.cell_kind(area), Some(CellKind::Out));
    }

    #[test]
    fn out_cell_is_referenceable_as_another_relationships_input() {
        let mut sheet = Sheet::new();
        let width = sheet.add_cell(4_i32);
        let height = sheet.add_cell(5_i32);
        let area = sheet.add_cell(0_i32);
        let doubled = sheet.add_cell(0_i32);
        sheet
            .add_out(
                Method::from_fn_2_1([width, height], area, |w: &i32, h: &i32| Ok(w * h)),
                vec![],
            )
            .unwrap();
        let result =
            sheet.add_relationship(vec![Method::from_fn_1_1(
                area,
                doubled,
                |a: &i32| Ok(a * 2),
            )]);
        assert!(result.is_ok());
    }

    #[test]
    fn out_cell_is_referenceable_as_a_conditional_match_subject() {
        let mut sheet = Sheet::new();
        let flag = sheet.add_cell(0_i32);
        let derived_flag = sheet.add_cell(0_i32);
        sheet
            .add_out(
                Method::from_fn_1_1(flag, derived_flag, |f: &i32| Ok(*f)),
                vec![],
            )
            .unwrap();
        let result = sheet.add_conditional(
            MatchExpr::cell(derived_flag),
            Vec::<(Vec<i32>, Vec<RelationshipId>)>::new(),
            vec![],
        );
        assert!(result.is_ok());
    }

    #[test]
    fn add_out_returns_invalid_cell_kind_for_a_write() {
        let mut sheet = Sheet::new();
        let width = sheet.add_cell(4_i32);
        let area = sheet.add_cell(0_i32);
        sheet
            .add_out(Method::from_fn_1_1(width, area, |w: &i32| Ok(*w)), vec![])
            .unwrap();
        assert!(matches!(
            sheet.write(area, 99_i32),
            Err(Error::InvalidCellKind { .. })
        ));
    }

    #[test]
    fn add_out_returns_invalid_cell_kind_for_a_second_writer() {
        let mut sheet = Sheet::new();
        let width = sheet.add_cell(4_i32);
        let height = sheet.add_cell(5_i32);
        let area = sheet.add_cell(0_i32);
        sheet
            .add_out(Method::from_fn_1_1(width, area, |w: &i32| Ok(*w)), vec![])
            .unwrap();
        let result = sheet.add_out(Method::from_fn_1_1(height, area, |h: &i32| Ok(*h)), vec![]);
        assert!(matches!(result, Err(Error::InvalidCellKind { .. })));
    }

    #[test]
    fn add_out_succeeds_when_target_cell_was_previously_used_only_as_an_input() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(1_i32);
        let b = sheet.add_cell(2_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let c = sheet.add_cell(0_i32);
        let result = sheet.add_out(Method::from_fn_1_1(a, c, |x: &i32| Ok(*x * 2)), vec![]);
        assert!(result.is_ok());
    }

    #[test]
    fn add_out_with_a_requirement_that_would_fail_still_succeeds_and_propagate_reports_it() {
        let mut sheet = Sheet::new();
        let width = sheet.add_cell(4_i32);
        let area = sheet.add_cell(0_i32);
        let area_cell = sheet
            .add_out(
                Method::from_fn_1_1(width, area, |w: &i32| Ok(*w)),
                vec![(
                    Some("too_small"),
                    Requirement::from_fn_1(area, |a: &i32| Ok(*a > 100)),
                )],
            )
            .unwrap();
        sheet.propagate().unwrap();
        assert!(!sheet.cell_requirements_valid(area_cell));
        assert_eq!(sheet.violated_requirements(area_cell).count(), 1);
    }

    #[test]
    fn out_cells_iterates_only_out_kind_cells() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(1_i32);
        let b = sheet.add_cell(0_i32);
        let out_id = sheet
            .add_out(Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)), vec![])
            .unwrap();
        let ids: Vec<CellId> = sheet.out_cells().collect();
        assert_eq!(ids, vec![out_id]);
    }

    #[test]
    fn add_filter_succeeds_on_a_real_out_kind_cell() {
        let mut sheet = Sheet::new();
        let width = sheet.add_cell(4_i32);
        let area = sheet.add_cell(0_i32);
        let out_cell = sheet
            .add_out(Method::from_fn_1_1(width, area, |w: &i32| Ok(*w)), vec![])
            .unwrap();
        let result = sheet.add_filter(out_cell, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 10))));
        assert!(result.is_ok());
    }

    #[test]
    fn requirement_relevant_cells_covers_a_plain_cells_requirement_too() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        sheet
            .add_requirement(
                a,
                Some("positive"),
                Requirement::from_fn_1(a, |x: &i32| Ok(*x > 0)),
            )
            .unwrap();
        sheet.propagate().unwrap();
        assert!(sheet.requirement_relevant_cells().contains(&a));
    }

    #[test]
    fn requirement_violation_cells_covers_a_plain_cells_violation_too() {
        // `add_requirement` hard-fails immediately if a `Cell`/`Source` kind cell's
        // *current* value already violates it (see
        // `add_requirement_hard_fails_when_current_value_already_violates_it_on_a_plain_cell`),
        // so the requirement is attached while it still holds (200 > 100), then a
        // later `write` — not the initial value — is what drives it into violation.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(200_i32);
        sheet
            .add_requirement(
                a,
                Some("too_big"),
                Requirement::from_fn_1(a, |x: &i32| Ok(*x > 100)),
            )
            .unwrap();
        sheet.write(a, 5_i32).unwrap();
        sheet.propagate().unwrap();
        assert!(sheet.requirement_violation_cells().contains(&a));
    }
}
