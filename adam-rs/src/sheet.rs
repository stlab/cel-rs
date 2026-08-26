//! The [`Sheet`] owns and manages a property model constraint graph.
//!
//! All cells and relationships are created through the sheet and are
//! destroyed when the sheet is dropped.

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};

use slotmap::SlotMap;

use crate::{
    cell::{CellData, CellId},
    conditional::{Branch, ConditionalData, ConditionalId, MatchExpr, MatchSource},
    error::Error,
    filter::{Filter, FilterKind, FilterViolation},
    output::{OutputData, OutputId},
    planner::PlanStep,
    relationship::{Method, RelationshipData, RelationshipId},
    requirement::{Requirement, RequirementData, RequirementId},
};

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
    last_plan: Option<Vec<PlanStep>>,
    /// Cells reported forced (see [`Sheet::is_forced`]) by the last full `propagate()`
    /// call. Not recomputed by `propagate_without_replan`.
    last_forced: Option<HashSet<CellId>>,
    /// Relationships reported forced (see [`Sheet::is_relationship_forced`]) by the
    /// last full `propagate()` call. Not recomputed by `propagate_without_replan`.
    last_forced_relationships: Option<HashSet<RelationshipId>>,
    /// All conditionals registered on this sheet.
    pub(crate) conditionals: SlotMap<ConditionalId, ConditionalData>,
    /// Union of all RelationshipIds assigned to any conditional branch or default.
    /// Used to exclude them from the unconditional active set.
    pub(crate) conditional_relationships: HashSet<RelationshipId>,
    /// Cells belonging to a registered output (see [`Sheet::add_output`]). Such a cell
    /// can never be referenced as an input to a relationship, conditional, requirement, or
    /// another output, and can never be the target of `write`.
    terminal_cells: HashSet<CellId>,
    /// All outputs registered on this sheet.
    outputs: SlotMap<OutputId, OutputData>,
    /// All requirements registered on this sheet, across all outputs.
    requirements: SlotMap<RequirementId, RequirementData>,
    /// Requirements that evaluated `false` as of the last `propagate()` call, grouped by
    /// output. Sparse: an output with no entry had all its requirements hold. Not
    /// recomputed by `propagate_without_replan`.
    last_violated: HashMap<OutputId, Vec<RequirementId>>,
    /// Filter violations recorded against a derived value as of the last `propagate()`
    /// call. Not recomputed by `propagate_without_replan`, consistent with
    /// `last_violated`.
    last_filter_violations: HashMap<CellId, FilterViolation>,
    /// Reverse index of `filter_args`: for each cell, the live cells whose filter
    /// references it as one of its dynamic arguments. Built incrementally in
    /// `add_filter`; cells and filters are never removed once added, so this needs no
    /// invalidation, matching `terminal_cells` and every other per-cell set `Sheet`
    /// already maintains for its own lifetime.
    filter_dependents: HashMap<CellId, Vec<CellId>>,
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

impl Sheet {
    /// Creates an empty sheet with no cells or relationships.
    pub fn new() -> Self {
        Sheet {
            cells: SlotMap::with_key(),
            relationships: SlotMap::with_key(),
            changed_cells: Vec::new(),
            next_strength: 0,
            last_plan: None,
            last_forced: None,
            last_forced_relationships: None,
            conditionals: SlotMap::with_key(),
            conditional_relationships: HashSet::new(),
            terminal_cells: HashSet::new(),
            outputs: SlotMap::with_key(),
            requirements: SlotMap::with_key(),
            last_violated: HashMap::new(),
            last_filter_violations: HashMap::new(),
            filter_dependents: HashMap::new(),
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
        })
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
    /// - `Error::DuplicateMethodOutputs` — a method's own `outputs` list names a cell
    ///   more than once, or two methods in the same relationship have identical
    ///   `outputs` sets.
    /// - `Error::InvalidId` — a `CellId` in any method is not found in this sheet.
    /// - `Error::TypeMismatch` — a method's declared `TypeId` does not match the
    ///   cell's registered `TypeId`.
    /// - `Error::TerminalCell` — a method input or output cell already belongs to
    ///   an existing output.
    ///
    /// - Complexity: O(m² × c) where m is the total number of methods and c is the
    ///   maximum number of cells per method (due to duplicate output set comparison).
    pub fn add_relationship(&mut self, methods: Vec<Method>) -> Result<RelationshipId, Error> {
        if methods.is_empty() {
            return Err(Error::InvalidMethod);
        }

        for method in &methods {
            if method.outputs.is_empty() {
                return Err(Error::InvalidMethod);
            }

            // declared type counts must match cell-id counts
            if method.inputs.len() != method.input_types.len()
                || method.outputs.len() != method.output_types.len()
            {
                return Err(Error::InvalidMethod);
            }

            for (&cell_id, &declared) in method.inputs.iter().zip(method.input_types.iter()) {
                if self.terminal_cells.contains(&cell_id) {
                    return Err(Error::TerminalCell);
                }
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                    });
                }
            }

            for (&cell_id, &declared) in method.outputs.iter().zip(method.output_types.iter()) {
                if self.terminal_cells.contains(&cell_id) {
                    return Err(Error::TerminalCell);
                }
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
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
        if cell_sets[1..].iter().any(|set| set != &cell_sets[0]) {
            return Err(Error::MismatchedMethodCells);
        }

        // A method's own outputs must be duplicate-free, and no two methods in a
        // relationship may claim the same output set: the planner's matching stage
        // treats a method's pure-output set as an indivisible claim, so two methods
        // sharing an output set would make that claim ambiguous.
        let mut seen_output_sets: Vec<HashSet<CellId>> = Vec::with_capacity(methods.len());
        for method in &methods {
            let output_set: HashSet<CellId> = method.outputs.iter().copied().collect();
            if output_set.len() != method.outputs.len() || seen_output_sets.contains(&output_set) {
                return Err(Error::DuplicateMethodOutputs);
            }
            seen_output_sets.push(output_set);
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
    /// - `Error::TerminalCell` — the match subject references a cell that already belongs
    ///   to an existing output.
    /// - `Error::TypeMismatch` — (expression match subject only) an input cell's registered
    ///   type doesn't match the expression's declared type for that input.
    /// - `Error::InvalidConditional` — the match subject's output type does not match `T`;
    ///   a branch relationship shares a cell with the match subject or any of its
    ///   unconditional upstream contributors and has more than one method; a referenced
    ///   relationship does not exist; a relationship already appears in another
    ///   conditional branch; or a branch has no keys.
    ///
    /// - Complexity: O(B·(K + R)) where B = branches, K = keys per branch, R =
    ///   relationships per branch.
    pub fn add_conditional<T: Any + PartialEq + 'static>(
        &mut self,
        source: MatchExpr,
        branches: Vec<(Vec<T>, Vec<RelationshipId>)>,
        default: Vec<RelationshipId>,
    ) -> Result<ConditionalId, Error> {
        let match_cells: Vec<CellId> = match &source.0 {
            MatchSource::Cell(cell) => {
                let cell_data = self.cells.get(*cell).ok_or(Error::InvalidId)?;
                if self.terminal_cells.contains(cell) {
                    return Err(Error::TerminalCell);
                }
                if cell_data.type_id != TypeId::of::<T>() {
                    return Err(Error::InvalidConditional);
                }
                vec![*cell]
            }
            MatchSource::Expr(expr) => {
                if expr.output_type != TypeId::of::<T>() {
                    return Err(Error::InvalidConditional);
                }
                for (&cell_id, &declared) in expr.inputs.iter().zip(expr.input_types.iter()) {
                    if self.terminal_cells.contains(&cell_id) {
                        return Err(Error::TerminalCell);
                    }
                    let cell_data = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                    if cell_data.type_id != declared {
                        return Err(Error::TypeMismatch {
                            expected: cell_data.type_id,
                            found: declared,
                        });
                    }
                }
                expr.inputs.clone()
            }
        };

        // Collect and validate all relationship IDs (branches + default).
        let all_rels: Vec<RelationshipId> = branches
            .iter()
            .flat_map(|(_, rels)| rels.iter().copied())
            .chain(default.iter().copied())
            .collect();
        let all_rels_set: HashSet<RelationshipId> = all_rels.iter().copied().collect();

        // Compute the set of cells that contribute to the match subject: BFS upstream
        // through unconditional relationships (excluding already-committed conditional
        // relationships and the relationships currently being added), seeded from *every*
        // match cell. A branch relationship with multiple methods is invalid if any of its
        // adjacent cells is in this contributing set, because the branch could then flip
        // method selection in the match subject's upstream subgraph.
        let contributing_cells: HashSet<CellId> = {
            let mut cells: HashSet<CellId> = HashSet::new();
            let mut queue: std::collections::VecDeque<CellId> = std::collections::VecDeque::new();
            for &cell in &match_cells {
                if cells.insert(cell) {
                    queue.push_back(cell);
                }
            }
            while let Some(c) = queue.pop_front() {
                if let Some(cell_data) = self.cells.get(c) {
                    for &rel_id in &cell_data.adj {
                        if self.conditional_relationships.contains(&rel_id)
                            || all_rels_set.contains(&rel_id)
                        {
                            continue;
                        }
                        let rel = &self.relationships[rel_id];
                        if !rel.methods.iter().any(|m| m.outputs.contains(&c)) {
                            continue;
                        }
                        for &adj_cell in &rel.adj {
                            if cells.insert(adj_cell) {
                                queue.push_back(adj_cell);
                            }
                        }
                    }
                }
            }
            cells
        };

        for &rel_id in &all_rels {
            let rel = self
                .relationships
                .get(rel_id)
                .ok_or(Error::InvalidConditional)?;
            if rel.adj.iter().any(|c| contributing_cells.contains(c)) && rel.methods.len() != 1 {
                return Err(Error::InvalidConditional);
            }
            if self.conditional_relationships.contains(&rel_id) {
                return Err(Error::InvalidConditional);
            }
        }

        // Validate branch keys are non-empty.
        for (keys, _) in &branches {
            if keys.is_empty() {
                return Err(Error::InvalidConditional);
            }
        }

        // Check for duplicate relationship IDs within this call.
        let mut seen: HashSet<RelationshipId> = HashSet::new();
        for &rel_id in &all_rels {
            if !seen.insert(rel_id) {
                return Err(Error::InvalidConditional);
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
        for &rel_id in &all_rels {
            self.conditional_relationships.insert(rel_id);
        }

        Ok(self.conditionals.insert(ConditionalData {
            source: source.0,
            branches: typed_branches,
            default,
        }))
    }

    /// Returns `true` if `id` already has adjacency (a relationship referencing it, or
    /// use as some conditional's match cell) — i.e. it cannot legally become an output's
    /// terminal cell, since that would retroactively violate the terminal invariant for
    /// whatever already references it.
    fn cell_has_prior_use(&self, id: CellId) -> bool {
        self.cells.get(id).is_some_and(|cell| !cell.adj.is_empty())
            || self
                .conditionals
                .values()
                .any(|c| c.match_cells().contains(&id))
    }

    /// Registers an output: a cell written by exactly one method, together with zero or
    /// more named requirements checked after every `propagate()`.
    ///
    /// `writer` must have exactly one output cell — that cell becomes terminal: it can
    /// never afterward be referenced as an input to a relationship, conditional,
    /// requirement, or another output, nor be the target of `write`. A requirement's
    /// inputs may be any cells in the sheet, including the output's own cell, but not a
    /// cell that already belongs to a different output.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidOutput` — `writer` does not have exactly one output cell, a
    ///   requirement name is empty, or two requirements share a name.
    /// - `Error::TerminalCell` — a requirement input is already another output's cell, or
    ///   the writer's output cell already has prior use (see `cell_has_prior_use`)
    ///   and so cannot become terminal.
    /// - `Error::InvalidId` — a cell referenced by `writer` or a requirement is not in this
    ///   sheet.
    /// - `Error::TypeMismatch` — a requirement input's declared type does not match the
    ///   cell's registered type.
    /// - Any error `add_relationship` can return, for `writer`'s own validation.
    ///
    /// - Complexity: O(k + m²×c) where k is the number of requirements (each validated
    ///   in a single pass over its inputs), plus the cost of `add_relationship` for
    ///   `writer` alone (m = 1 method, c = cells in that method).
    pub fn add_output(
        &mut self,
        writer: Method,
        requirements: Vec<(&str, Requirement)>,
    ) -> Result<OutputId, Error> {
        if writer.outputs.len() != 1 {
            return Err(Error::InvalidOutput);
        }
        let output_cell = writer.outputs[0];

        let mut seen_names: HashSet<&str> = HashSet::new();
        for &(name, _) in &requirements {
            if name.is_empty() || !seen_names.insert(name) {
                return Err(Error::InvalidOutput);
            }
        }

        for (_, requirement) in &requirements {
            if requirement.inputs.len() != requirement.input_types.len() {
                return Err(Error::InvalidOutput);
            }
            for (&cell_id, &declared) in requirement
                .inputs
                .iter()
                .zip(requirement.input_types.iter())
            {
                if self.terminal_cells.contains(&cell_id) {
                    return Err(Error::TerminalCell);
                }
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                    });
                }
            }
        }

        if self.cell_has_prior_use(output_cell) {
            return Err(Error::TerminalCell);
        }

        self.add_relationship(vec![writer])?;
        self.terminal_cells.insert(output_cell);

        let output_id = self.outputs.insert(OutputData {
            cell: output_cell,
            requirements: Vec::new(),
        });

        let requirement_ids: Vec<RequirementId> = requirements
            .into_iter()
            .map(|(name, requirement)| {
                self.requirements.insert(RequirementData {
                    name: name.to_string(),
                    output: output_id,
                    inputs: requirement.inputs,
                    function: requirement.function,
                })
            })
            .collect();
        self.outputs[output_id].requirements = requirement_ids;

        Ok(output_id)
    }

    /// Attaches `filter` to `cell`.
    ///
    /// Immediately applies `filter` to `cell`'s current `source` value, exactly as
    /// [`Sheet::write`] would, so a filtered cell's value is guaranteed to conform from
    /// this call onward — not just from the next external write.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `cell`, or one of `filter`'s argument cells, is not a
    ///   live cell in this sheet.
    /// - `Error::TerminalCell` — `cell` already belongs to an existing output.
    /// - `Error::InvalidFilter` — `cell` already has a filter, `filter`'s own value
    ///   type does not match `cell`'s registered type, or `filter`'s argument list
    ///   names `cell` itself.
    /// - `Error::TypeMismatch` — an argument cell's registered type does not match the
    ///   type `filter` declared for it, or (defensively) `filter`'s function returned
    ///   a value of a different type than `cell`'s registered type.
    /// - `Error::MethodFailed` — `filter` rejected `cell`'s current value.
    ///
    /// - Complexity: O(a) where a is the number of `filter`'s argument cells.
    pub fn add_filter(&mut self, cell: CellId, filter: Filter) -> Result<(), Error> {
        let cell_type = self.cells.get(cell).ok_or(Error::InvalidId)?.type_id;
        if self.terminal_cells.contains(&cell) {
            return Err(Error::TerminalCell);
        }
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
                });
            }
        }

        let args: Vec<&dyn Any> = filter
            .0
            .args
            .iter()
            .map(|&a| self.cells[a].effective())
            .collect();
        let conformed = (filter.0.function)(self.cells[cell].source.as_ref(), &args)
            .map_err(Error::MethodFailed)?;
        if conformed.as_ref().type_id() != cell_type {
            return Err(Error::TypeMismatch {
                expected: cell_type,
                found: conformed.as_ref().type_id(),
            });
        }

        for &arg in &filter.0.args {
            self.filter_dependents.entry(arg).or_default().push(cell);
        }

        let cell_data = &mut self.cells[cell];
        cell_data.source = conformed;
        cell_data.derived = None;
        cell_data.filter = Some(filter.0);
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
    /// `requirement_contributing_cells`/`output_violation_cells` already provide for
    /// `Requirement`.
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

    /// Returns the terminal cell backing output `id`. Read its value with [`Sheet::read`].
    ///
    /// Returns `None` if `id` is not a live output in this sheet.
    pub fn output_cell(&self, id: OutputId) -> Option<CellId> {
        self.outputs.get(id).map(|o| o.cell)
    }

    /// Returns the requirements registered on output `id`, in declaration order.
    ///
    /// Returns `None` if `id` is not a live output in this sheet.
    pub fn output_requirements(&self, id: OutputId) -> Option<&[RequirementId]> {
        self.outputs.get(id).map(|o| o.requirements.as_slice())
    }

    /// Returns the name of requirement `id`.
    ///
    /// Returns `None` if `id` is not a live requirement in this sheet.
    pub fn requirement_name(&self, id: RequirementId) -> Option<&str> {
        self.requirements.get(id).map(|c| c.name.as_str())
    }

    /// Returns the output that requirement `id` belongs to.
    ///
    /// Returns `None` if `id` is not a live requirement in this sheet.
    pub fn requirement_output(&self, id: RequirementId) -> Option<OutputId> {
        self.requirements.get(id).map(|c| c.output)
    }

    /// Returns the cells requirement `id` reads.
    ///
    /// Returns `None` if `id` is not a live requirement in this sheet.
    pub fn requirement_inputs(&self, id: RequirementId) -> Option<&[CellId]> {
        self.requirements.get(id).map(|c| c.inputs.as_slice())
    }

    /// Returns `true` if every requirement on `id` held as of the last `propagate()` call.
    ///
    /// Returns `false` if no propagation has run yet. Also returns `true` for an `id`
    /// that is not a live output in this sheet, since no requirement can have failed
    /// for an output that doesn't exist.
    pub fn output_valid(&self, id: OutputId) -> bool {
        if self.last_plan.is_none() {
            return false;
        }
        !self.last_violated.contains_key(&id)
    }

    /// Iterates the requirements on `id` that evaluated to `false` as of the last
    /// `propagate()` call.
    ///
    /// - Postcondition: empty if `id`'s requirements all held, `id` is not a live
    ///   output in this sheet, or no propagation has run yet.
    pub fn violated_requirements(&self, id: OutputId) -> impl Iterator<Item = RequirementId> + '_ {
        self.last_violated.get(&id).into_iter().flatten().copied()
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
    /// `propagate()` call — an unconditional relationship is always active; a conditional
    /// branch/default relationship is active only while its conditional currently selects
    /// it.
    ///
    /// Returns `false` if no propagation has run yet.
    fn is_relationship_active(&self, rel_id: RelationshipId) -> bool {
        self.last_plan.as_ref().is_some_and(|plan| {
            plan.iter()
                .any(|step| matches!(step, PlanStep::Method(r, _) if *r == rel_id))
        })
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

    /// Returns the union of `contributing_cells` over every live output's cell — the set
    /// of cells currently determining at least one output's value, as of the last
    /// `propagate()` call.
    ///
    /// - Postcondition: empty if the sheet has no outputs.
    /// - Complexity: O(sum of `contributing_cells` cost over every output).
    pub fn output_relevant_cells(&self) -> HashSet<CellId> {
        self.outputs()
            .filter_map(|id| self.output_cell(id))
            .flat_map(|cell| self.contributing_cells(cell))
            .collect()
    }

    /// Returns the union of `requirement_contributing_cells` over every requirement that
    /// evaluated `false` as of the last `propagate()` call, across every output in the
    /// sheet.
    ///
    /// - Postcondition: empty if the sheet has no outputs, or if every requirement on
    ///   every output currently holds.
    /// - Complexity: O(sum of `requirement_contributing_cells` cost over every violated
    ///   requirement).
    pub fn output_violation_cells(&self) -> HashSet<CellId> {
        self.outputs()
            .flat_map(|id| self.violated_requirements(id))
            .flat_map(|cid| self.requirement_contributing_cells(cid))
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
    /// - `Error::TerminalCell` — `id` already belongs to an existing output.
    /// - `Error::MethodFailed` — the cell has a filter and it rejected `value`; the
    ///   cell is left completely unchanged (no strength bump, no `source` change).
    pub fn write<T: Any + 'static>(&mut self, id: CellId, value: T) -> Result<(), Error> {
        if self.terminal_cells.contains(&id) {
            return Err(Error::TerminalCell);
        }
        let cell_type = self.cells.get(id).ok_or(Error::InvalidId)?.type_id;
        if cell_type != TypeId::of::<T>() {
            return Err(Error::TypeMismatch {
                expected: cell_type,
                found: TypeId::of::<T>(),
            });
        }

        let boxed: Box<dyn Any> = if let Some(filter) = self.cells[id].filter.as_ref() {
            let args: Vec<&dyn Any> = filter
                .args
                .iter()
                .map(|&a| self.cells[a].effective())
                .collect();
            let conformed = (filter.function)(&value, &args).map_err(Error::MethodFailed)?;
            if conformed.as_ref().type_id() != cell_type {
                return Err(Error::TypeMismatch {
                    expected: cell_type,
                    found: conformed.as_ref().type_id(),
                });
            }
            conformed
        } else {
            Box::new(value)
        };

        self.next_strength += 1;
        let cell = &mut self.cells[id];
        cell.strength = self.next_strength | (1u64 << 63);
        cell.source = boxed;
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
    /// equality.
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

    /// Returns the set of unconditional relationships transitively needed to derive
    /// the given `match_cells`.
    ///
    /// Walks upstream (from each match cell, through relationships whose outputs include
    /// the cell) collecting only relationships not in `self.conditional_relationships`.
    /// Relationships that only take a match cell as *input* (not output) are skipped.
    ///
    /// - Complexity: O(C·R) in the worst case where C = cells and R = relationships.
    fn match_cell_subgraph(&self, match_cells: &[CellId]) -> HashSet<RelationshipId> {
        let mut result: HashSet<RelationshipId> = HashSet::new();
        let mut visited: HashSet<CellId> = HashSet::new();
        let mut queue: std::collections::VecDeque<CellId> = match_cells.iter().copied().collect();

        for &cell in match_cells {
            visited.insert(cell);
        }

        while let Some(cell) = queue.pop_front() {
            for &rel_id in &self.cells[cell].adj {
                if self.conditional_relationships.contains(&rel_id) {
                    continue;
                }
                if result.contains(&rel_id) {
                    continue;
                }
                let rel = &self.relationships[rel_id];
                // Only include relationships that output this cell.
                let outputs_cell = rel.methods.iter().any(|m| m.outputs.contains(&cell));
                if !outputs_cell {
                    continue;
                }
                result.insert(rel_id);
                // Enqueue all inputs of this relationship for upstream BFS.
                for method in &rel.methods {
                    for &input in &method.inputs {
                        if visited.insert(input) {
                            queue.push_back(input);
                        }
                    }
                }
            }
        }

        result
    }

    /// Evaluates conditional `cond`'s current match value: borrows the cell directly for a
    /// plain match subject (no allocation), or calls the expression's function once for a
    /// computed match subject.
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
                let value = (expr.function)(&args).map_err(Error::MethodFailed)?;
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

    /// Builds the active relationship set for the general planning pass.
    ///
    /// Starts with all unconditional relationships (those not in
    /// `self.conditional_relationships`), then evaluates each conditional: the first
    /// branch whose keys contain the match subject's current value is selected, and its
    /// relationships are added. If no branch matches, the default relationships are added.
    ///
    /// # Errors
    ///
    /// - `Error::MethodFailed` — an expression-sourced conditional's function returned an
    ///   error.
    ///
    /// - Complexity: O(R + C·B·K) where R = total relationships, C = conditionals,
    ///   B = branches per conditional, K = keys per branch.
    fn build_active_set(&self) -> Result<HashSet<RelationshipId>, Error> {
        let mut active: HashSet<RelationshipId> = self
            .relationships
            .keys()
            .filter(|id| !self.conditional_relationships.contains(id))
            .collect();

        for (_, cond) in &self.conditionals {
            let value = self.evaluate_match_source(cond)?;
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

    /// Assigns derived-cell strengths after a planning pass.
    ///
    /// Walks `execution_order` and assigns a decrementing counter (starting at
    /// `0x7FFF_FFFF_FFFF_FFFF`) to each output cell of each selected method, in
    /// execution order. Cells evaluated first receive the highest derived strength.
    /// Source cells (not the output of any selected method) are not modified.
    ///
    /// - Complexity: O(R·K) where R is the number of entries and K is the maximum
    ///   outputs per method.
    fn post_process_strengths(&mut self, execution_order: &[PlanStep]) {
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
                    if seen.insert(output)
                        && let Some(cell) = self.cells.get_mut(output)
                    {
                        cell.strength = derived_strength;
                        derived_strength = derived_strength.saturating_sub(1);
                    }
                }
            }
        }
    }

    /// Runs the planning pass and executes the selected methods.
    ///
    /// Clears the changed-cell set from the previous `propagate()` call before planning.
    /// After propagation, call [`Sheet::changed`] to inspect which cells were updated,
    /// and [`Sheet::clear_changed`] when done.
    ///
    /// **Phase 0 — Derived reset:** every cell's derived override is cleared before
    /// planning begins, so no pure-input read this round can observe a derived value
    /// left over from a previous round.
    ///
    /// **Phase 1 — Pre-plan:** if any conditional match cells are derived (have an
    /// in-edge in the unconditional relationship graph), the minimal unconditional
    /// subgraph needed to compute them is planned and executed so their values are
    /// current before branch evaluation.
    ///
    /// **Phase 2 — Conditional evaluation:** each conditional's match cell value is
    /// read and compared against branch keys; the active relationship set is built.
    ///
    /// **Phase 3 — General plan:** the Adam algorithm runs on the active set.
    ///
    /// **Phase 4 — Strength post-processing:** derived cells receive low-order strengths
    /// in evaluation order, enforcing the stability invariant.
    ///
    /// **Phase 5 — Reversion change-tracking:** a cell whose derived override existed
    /// before this round but wasn't reclaimed by any method this round has effectively
    /// reverted to its source value (e.g. its forcing conditional went inactive); it is
    /// marked changed even though no method wrote to it this round.
    ///
    /// **Phase 6 — Requirement evaluation:** every registered requirement is evaluated
    /// against current cell values, rebuilding `last_violated` from scratch, so
    /// [`Sheet::output_valid`] and [`Sheet::violated_requirements`] reflect this round.
    ///
    /// # Errors
    ///
    /// - `Error::Conflict` — no valid method assignment exists.
    /// - `Error::MethodFailed` — a method's function returned an error, a method
    ///   produced the wrong number of outputs, or a requirement's function returned
    ///   an error.
    /// - `Error::TypeMismatch` — a method output's runtime type does not match the
    ///   cell's registered type.
    pub fn propagate(&mut self) -> Result<(), Error> {
        self.clear_changed();

        // Phase 0: snapshot cells with a live derived override (for Phase 5 only),
        // then reset every cell's derived override before planning begins.
        let previously_derived: Vec<CellId> = self
            .cells
            .iter()
            .filter(|(_, cell)| cell.derived.is_some())
            .map(|(id, _)| id)
            .collect();
        for (_, cell) in self.cells.iter_mut() {
            cell.derived = None;
        }

        // Phase 1: pre-plan for derived match cells.
        if !self.conditionals.is_empty() {
            let match_cells: Vec<CellId> = self
                .conditionals
                .values()
                .flat_map(|c| c.match_cells().iter().copied())
                .collect();
            let pre_active = self.match_cell_subgraph(&match_cells);
            if !pre_active.is_empty() {
                let pre_plan = crate::planner::plan(&self.cells, &self.relationships, &pre_active)?;
                self.execute_plan(&pre_plan.execution_order, &mut Vec::new())?;
            }
        }

        // Phase 2: evaluate conditionals and build the active relationship set.
        let active = self.build_active_set()?;

        // Phase 3: general plan on the active set.
        let plan = crate::planner::plan(&self.cells, &self.relationships, &active)?;
        let mut source_filter_violations: Vec<(CellId, FilterViolation)> = Vec::new();
        self.execute_plan(&plan.execution_order, &mut source_filter_violations)?;

        // Phase 4: assign derived-cell strengths in evaluation order.
        self.post_process_strengths(&plan.execution_order);

        // Phase 5: cells that reverted (had a derived override, didn't get a fresh one
        // this round) need explicit change-tracking.
        for id in previously_derived {
            if let Some(cell) = self.cells.get_mut(id)
                && cell.derived.is_none()
                && !cell.changed
            {
                cell.changed = true;
                self.changed_cells.push(id);
            }
        }

        // Phase 6: evaluate every registered requirement against current cell values.
        let mut last_violated: HashMap<OutputId, Vec<RequirementId>> = HashMap::new();
        for (requirement_id, requirement) in self.requirements.iter() {
            let inputs: Vec<&dyn Any> = requirement
                .inputs
                .iter()
                .map(|&id| self.cells[id].effective())
                .collect();
            let holds = (requirement.function)(&inputs).map_err(Error::MethodFailed)?;
            if !holds {
                last_violated
                    .entry(requirement.output)
                    .or_default()
                    .push(requirement_id);
            }
        }
        self.last_violated = last_violated;

        // Phase 6b: evaluate every filter against a value derived by a method this
        // round — a non-gating diagnostic. A filter is never re-checked against a
        // value that came from a plain external write: `write`/`add_filter` already
        // conformed it, and nothing here ever mutates a cell.
        let mut derived_this_round: HashSet<CellId> = HashSet::new();
        for step in &plan.execution_order {
            if let PlanStep::Method(rel_id, method_idx) = step
                && let Some(method) = self
                    .relationships
                    .get(*rel_id)
                    .and_then(|r| r.methods.get(*method_idx))
            {
                derived_this_round.extend(method.outputs.iter().copied());
            }
        }
        // Seeded from execute_plan's source-cell reclamp failures above; disjoint keys
        // from the derived-cell loop below (a cell is a source or derived this round,
        // never both), so there's no merge conflict.
        let mut last_filter_violations: HashMap<CellId, FilterViolation> =
            source_filter_violations.into_iter().collect();
        for &cell_id in &derived_this_round {
            let Some(filter) = self.cells[cell_id].filter.as_ref() else {
                continue;
            };
            let args: Vec<&dyn Any> = filter
                .args
                .iter()
                .map(|&a| self.cells[a].effective())
                .collect();
            let current = self.cells[cell_id].effective();
            match (filter.function)(current, &args) {
                Ok(conformed) => {
                    let cell = &self.cells[cell_id];
                    if conformed.as_ref().type_id() != cell.type_id {
                        last_filter_violations.insert(
                            cell_id,
                            FilterViolation::Failed(anyhow::anyhow!(
                                "filter returned a value of a different type than the cell"
                            )),
                        );
                    } else if !(cell.eq_fn)(conformed.as_ref(), current) {
                        last_filter_violations.insert(cell_id, FilterViolation::NotConformed);
                    }
                }
                Err(e) => {
                    last_filter_violations.insert(cell_id, FilterViolation::Failed(e));
                }
            }
        }
        self.last_filter_violations = last_filter_violations;

        self.last_forced = Some(plan.forced_outputs);
        self.last_forced_relationships = Some(plan.forced_relationships);
        self.last_plan = Some(plan.execution_order);
        Ok(())
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
    /// unchanged. A reclamp whose filter returns `Err`, or a value of the wrong type, is
    /// pushed into `filter_violations` instead of aborting; the cell's stored value is
    /// left untouched in that case (its `derived` stays unset, so `read()` falls back to
    /// `source`).
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
    fn execute_plan(
        &mut self,
        execution_order: &[PlanStep],
        filter_violations: &mut Vec<(CellId, FilterViolation)>,
    ) -> Result<(), Error> {
        for step in execution_order {
            match *step {
                PlanStep::Method(rel_id, method_idx) => {
                    let is_conditional = self.conditional_relationships.contains(&rel_id);
                    let (outputs, output_ids, shadow_outputs) = {
                        let method = &self.relationships[rel_id].methods[method_idx];
                        let inputs: Vec<&dyn Any> = method
                            .inputs
                            .iter()
                            .map(|&id| {
                                if method.outputs.contains(&id) {
                                    // Self-referencing input: always the pre-execution
                                    // source, never a derived override from a previous
                                    // execution.
                                    self.cells[id].source.as_ref()
                                } else {
                                    self.cells[id].effective()
                                }
                            })
                            .collect();
                        let outputs = (method.function)(&inputs).map_err(Error::MethodFailed)?;
                        let output_ids = method.outputs.clone();
                        let shadow_outputs: Vec<bool> = method
                            .outputs
                            .iter()
                            .map(|o| method.inputs.contains(o) || is_conditional)
                            .collect();
                        (outputs, output_ids, shadow_outputs)
                    };

                    if outputs.len() != output_ids.len() {
                        return Err(Error::MethodFailed(anyhow::anyhow!(
                            "method produced {} outputs but relationship expects {}",
                            outputs.len(),
                            output_ids.len()
                        )));
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
                            });
                        }
                        if shadow {
                            cell.derived = Some(new_value);
                        } else {
                            cell.source = new_value;
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

    /// Returns the index of the method selected for `rel` in the last propagation.
    ///
    /// Returns `None` if no propagation has run yet, `rel` is not in the cached plan,
    /// or `rel` was added after the last `propagate()` call.
    pub fn selected_method(&self, rel: RelationshipId) -> Option<usize> {
        self.last_plan.as_ref()?.iter().find_map(|step| match step {
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

    /// Returns `true` if `id` was not written by any selected method in the last propagation.
    ///
    /// Returns `false` if no propagation has run yet (conservatively forces a full re-plan).
    ///
    /// - Complexity: O(R·K) where R is the number of relationships in the cached plan and K is the maximum number of outputs per method.
    pub fn is_source(&self, id: CellId) -> bool {
        let Some(plan) = &self.last_plan else {
            return false;
        };
        !plan.iter().any(|step| match step {
            PlanStep::Method(rel_id, method_idx) => self
                .relationships
                .get(*rel_id)
                .and_then(|r| r.methods.get(*method_idx))
                .map(|m| m.outputs.contains(&id))
                .unwrap_or(false),
            PlanStep::FilterReclamp(_) => false,
        })
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
        self.last_forced
            .as_ref()
            .is_some_and(|forced| forced.contains(&id))
    }

    /// Iterates cells that are forced (see [`Sheet::is_forced`]) as of the last
    /// `propagate()` call.
    ///
    /// - Complexity: O(n) where n is the number of forced cells.
    pub fn forced_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.last_forced.iter().flatten().copied()
    }

    /// Returns `true` if `id` had exactly one viable method as of the last successful
    /// `propagate()` call — the planner has no alternative method to choose for this
    /// relationship, regardless of cell strength.
    ///
    /// Returns `false` if no propagation has run yet.
    pub fn is_relationship_forced(&self, id: RelationshipId) -> bool {
        self.last_forced_relationships
            .as_ref()
            .is_some_and(|forced| forced.contains(&id))
    }

    /// Iterates relationships that are forced (see [`Sheet::is_relationship_forced`])
    /// as of the last `propagate()` call.
    ///
    /// - Complexity: O(n) where n is the number of forced relationships.
    pub fn forced_relationships(&self) -> impl Iterator<Item = RelationshipId> + '_ {
        self.last_forced_relationships.iter().flatten().copied()
    }

    /// Iterates all live conditional IDs in the sheet.
    ///
    /// - Complexity: O(n) where n is the number of conditionals.
    pub fn conditionals(&self) -> impl Iterator<Item = ConditionalId> + '_ {
        self.conditionals.keys()
    }

    /// Iterates all live output IDs in the sheet.
    ///
    /// - Complexity: O(n) where n is the number of outputs.
    pub fn outputs(&self) -> impl Iterator<Item = OutputId> + '_ {
        self.outputs.keys()
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

    /// Re-executes the cached plan without invoking the planner.
    ///
    /// - Precondition: Every cell written since the last successful `propagate()` or
    ///   `propagate_without_replan()` call satisfies `is_source(id)`. Violation produces
    ///   incorrect output values but no panic.
    /// - Precondition: If the sheet has conditionals, no match-cell value has changed
    ///   since the last `propagate()`. Violation produces incorrect branch activation.
    ///
    /// `is_forced` and `forced_cells` continue to reflect the last full `propagate()`
    /// call; this method does not recompute them. Likewise, `output_valid` and
    /// `violated_requirements` continue to reflect the last full `propagate()` call; this
    /// method does not re-evaluate output requirements.
    ///
    /// A cached [`PlanStep::FilterReclamp`] step is still re-executed on every call,
    /// using each argument's *current* effective value — only the `last_filter_violations`
    /// diagnostic map stays pinned, not the reclamp's mutation itself.
    ///
    /// # Errors
    ///
    /// - `Error::Conflict` — `propagate()` has not yet been called; no plan is cached.
    /// - `Error::MethodFailed` — a method's function returned an error.
    /// - `Error::TypeMismatch` — a method output's runtime type does not match the cell's
    ///   registered type.
    ///
    /// - Complexity: O(R·K) where R is the number of relationships in the cached plan and K is the maximum cells per method, plus per-method execution cost.
    pub fn propagate_without_replan(&mut self) -> Result<(), Error> {
        let Some(execution_order) = self.last_plan.take() else {
            return Err(Error::Conflict);
        };
        self.clear_changed();
        // Discarded: this replays any cached FilterReclamp step's mutation
        // unconditionally, but last_filter_violations stays pinned to the last full
        // propagate()'s result, per this method's documented contract.
        let result = self.execute_plan(&execution_order, &mut Vec::new());
        if result.is_ok() {
            self.post_process_strengths(&execution_order);
        }
        self.last_plan = Some(execution_order);
        result
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
        ConditionalId, Error, MatchExpr, Method, Sheet,
        cell::CellId,
        filter::{Filter, FilterKind, FilterViolation},
        relationship::RelationshipId,
    };
    use std::any::{Any, TypeId};

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
        assert!(matches!(result, Err(Error::InvalidConditional)));
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
        assert!(matches!(result, Err(Error::InvalidConditional)));
    }

    #[test]
    fn add_conditional_returns_invalid_conditional_for_multi_method_relationship_involving_match_cell()
     {
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
        let result =
            sheet.add_conditional(MatchExpr::cell(a), vec![(vec![0_i32], vec![rel])], vec![]);
        assert!(matches!(result, Err(Error::InvalidConditional)));
    }

    #[test]
    fn add_conditional_returns_error_when_branch_rel_involves_cell_upstream_of_match_cell() {
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
        let result =
            sheet.add_conditional(MatchExpr::cell(p), vec![(vec![0_i32], vec![rel])], vec![]);
        assert!(matches!(result, Err(Error::InvalidConditional)));
    }

    #[test]
    fn add_conditional_returns_error_when_branch_rel_involves_cell_upstream_of_either_expr_input() {
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
        let result = sheet.add_conditional(expr, vec![(vec![0_i32], vec![rel])], vec![]);
        assert!(matches!(result, Err(Error::InvalidConditional)));
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
        assert!(matches!(result, Err(Error::InvalidConditional)));
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
        assert!(matches!(result, Err(Error::MethodFailed(_))));
    }

    #[test]
    fn add_conditional_allows_multi_method_rel_not_involving_match_cell() {
        let mut sheet = Sheet::new();
        let mode = sheet.add_cell(0_i32);
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        // Relationship has two methods but does not involve `mode` (the match cell).
        // Branch relationships that don't contribute to the match cell may have any
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
        assert!(matches!(result, Err(Error::InvalidConditional)));
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
        assert!(matches!(result, Err(Error::InvalidConditional)));
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
    fn write_returns_terminal_cell_for_terminal_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        sheet.terminal_cells.insert(a);
        assert!(matches!(sheet.write(a, 1_i32), Err(Error::TerminalCell)));
    }

    #[test]
    fn add_relationship_returns_terminal_cell_for_terminal_input() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet.terminal_cells.insert(a);
        let result = sheet.add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))]);
        assert!(matches!(result, Err(Error::TerminalCell)));
    }

    #[test]
    fn add_relationship_returns_terminal_cell_for_terminal_output() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet.terminal_cells.insert(b);
        let result = sheet.add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))]);
        assert!(matches!(result, Err(Error::TerminalCell)));
    }

    #[test]
    fn add_conditional_returns_terminal_cell_for_terminal_match_cell() {
        let mut sheet = Sheet::new();
        let p = sheet.add_cell(0_i32);
        sheet.terminal_cells.insert(p);
        let result = sheet.add_conditional::<i32>(MatchExpr::cell(p), vec![], vec![]);
        assert!(matches!(result, Err(Error::TerminalCell)));
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
        assert!(matches!(
            sheet.add_relationship(vec![]),
            Err(Error::InvalidMethod)
        ));
    }

    #[test]
    fn add_relationship_type_mismatch_returns_error() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        // Method declares f64 input but cell holds i32.
        let method = Method::from_fn_1_1(a, b, |x: &f64| Ok(*x * 2.0));
        assert!(matches!(
            sheet.add_relationship(vec![method]),
            Err(Error::TypeMismatch { .. })
        ));
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
        assert!(matches!(
            sheet.add_relationship(vec![method]),
            Err(Error::InvalidMethod)
        ));
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
        assert!(matches!(result, Err(Error::MismatchedMethodCells)));
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
    fn add_relationship_duplicate_output_set_across_methods_returns_duplicate_method_outputs() {
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
        assert!(matches!(result, Err(Error::DuplicateMethodOutputs)));
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
        assert!(matches!(result, Err(Error::MismatchedMethodCells)));
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
        assert!(matches!(result, Err(Error::DuplicateMethodOutputs)));
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
        assert!(matches!(result, Err(Error::DuplicateMethodOutputs)));
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

    #[test]
    fn propagate_without_replan_returns_conflict_before_propagate() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        assert!(matches!(
            sheet.propagate_without_replan(),
            Err(Error::Conflict)
        ));
    }

    #[test]
    fn propagate_without_replan_executes_cached_plan() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        // Write to `a` so it has the highest strength and becomes the source.
        sheet.write(a, 0_i32).unwrap();
        sheet.propagate().unwrap();
        sheet.write(a, 5_i32).unwrap();
        sheet.propagate_without_replan().unwrap();
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 10);
    }

    #[test]
    fn selected_method_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert!(sheet.selected_method(RelationshipId::default()).is_none());
    }

    #[test]
    fn add_cell_and_write_set_high_order_bit_on_strength() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        assert!(
            sheet.cells[a].strength & (1u64 << 63) != 0,
            "add_cell must set high-order bit"
        );
        sheet.write(a, 1_i32).unwrap();
        assert!(
            sheet.cells[a].strength & (1u64 << 63) != 0,
            "write must set high-order bit"
        );
    }

    #[test]
    fn propagate_assigns_low_order_strength_to_derived_cells() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        sheet.write(a, 1_i32).unwrap();
        sheet.propagate().unwrap();
        assert!(
            sheet.cells[a].strength & (1u64 << 63) != 0,
            "source cell must keep high-order strength"
        );
        assert!(
            sheet.cells[b].strength & (1u64 << 63) == 0,
            "derived cell must have low-order strength"
        );
        assert!(sheet.cells[a].strength > sheet.cells[b].strength);
    }

    #[test]
    fn propagate_without_replan_keeps_derived_strengths_in_low_partition() {
        // Set up a sheet with a conditional: mode=1 → rel_on active (a→b).
        let mut sheet = Sheet::new();
        let mode = sheet.add_cell(0_i32);
        let a = sheet.add_cell(10_i32);
        let b = sheet.add_cell(0_i32);

        let rel_on = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        sheet
            .add_conditional(
                MatchExpr::cell(mode),
                vec![(vec![1_i32], vec![rel_on])],
                vec![],
            )
            .unwrap();

        // Full propagation with mode=1 (conditional active).
        sheet.write(mode, 1_i32).unwrap();
        sheet.write(a, 10_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 10);

        // b should have a low-order derived strength (high-bit clear).
        assert_eq!(
            sheet.cells[b].strength & (1u64 << 63),
            0,
            "derived cell b must have low-order strength after propagate"
        );

        // Re-execute the plan without replanning. b should still be derived correctly.
        sheet.propagate_without_replan().unwrap();
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 10);

        // b's strength should still be in the low partition after propagate_without_replan.
        assert_eq!(
            sheet.cells[b].strength & (1u64 << 63),
            0,
            "derived cell b must have low-order strength after propagate_without_replan"
        );
    }

    #[test]
    fn propagate_without_replan_correct_after_plan_switch() {
        // Setup: two cells, b added last (higher strength), so b→a method is selected.
        // Sheet has two methods: b→a and a→b.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![
                Method::from_fn_1_1(b, a, |x: &i32| Ok(*x * 2)),
                Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 3)),
            ])
            .unwrap();
        // First propagate: b is source (added last, higher strength). b→a selected.
        sheet.write(b, 5_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 10); // a = b * 2 = 10
        assert!(!sheet.is_source(a)); // a is output

        // Write to a: raises a's strength above b, plan switches to a→b.
        sheet.write(a, 4_i32).unwrap();
        sheet.propagate().unwrap(); // plan now: a→b selected (a*3)
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 12); // b = a * 3 = 12
        assert!(sheet.is_source(a)); // a is now a source

        // Second write to a: is_source(a) is true → propagate_without_replan is safe.
        sheet.write(a, 7_i32).unwrap();
        sheet.propagate_without_replan().unwrap();
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 21); // b = a * 3 = 21
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
    fn add_filter_conforms_the_cells_current_value_immediately() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(500_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 100);
    }

    #[test]
    fn add_filter_leaves_a_conforming_value_unchanged() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    }

    #[test]
    fn add_filter_returns_method_failed_when_current_value_cannot_conform() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let result = sheet.add_filter(
            a,
            Filter::from_fn_0(|_x: &i32| Err(anyhow::anyhow!("cannot conform"))),
        );
        assert!(matches!(result, Err(Error::MethodFailed(_))));
        // Rejected: the cell's original value must survive untouched.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    }

    #[test]
    fn add_filter_returns_invalid_id_for_missing_cell() {
        let mut sheet = Sheet::new();
        let result = sheet.add_filter(CellId::default(), Filter::from_fn_0(|x: &i32| Ok(*x)));
        assert!(matches!(result, Err(Error::InvalidId)));
    }

    #[test]
    fn add_filter_returns_terminal_cell_for_an_output_cell() {
        let mut sheet = Sheet::new();
        let writer_input = sheet.add_cell(1_i32);
        let out_cell = sheet.add_cell(0_i32);
        let out = sheet
            .add_output(
                Method::from_fn_1_1(writer_input, out_cell, |x: &i32| Ok(*x)),
                vec![],
            )
            .unwrap();
        let terminal = sheet.output_cell(out).unwrap();
        let result = sheet.add_filter(terminal, Filter::from_fn_0(|x: &i32| Ok(*x)));
        assert!(matches!(result, Err(Error::TerminalCell)));
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
    fn add_filter_resolves_a_dynamic_argument_cells_current_value() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(500_i32);
        let bound = sheet.add_cell(10_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |x: &i32, bound: &i32| Ok((*x).min(*bound))),
            )
            .unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 10);
    }

    #[test]
    fn from_fn_2_conforms_values_through_sheet_using_both_dynamic_arguments() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(50_i32);
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
        // Attach-time value (50) already conforms.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 50);
        // A later write is conformed against both dynamic argument cells.
        sheet.write(a, 500_i32).unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 100);
        sheet.write(a, -10_i32).unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 0);
    }

    #[test]
    fn add_filter_returns_type_mismatch_when_the_filters_function_returns_the_wrong_type() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        // `value_type` matches `a`'s registered type (so add_filter's own value-type
        // check passes), but the function itself always returns a `f64`, tripping
        // add_filter's defensive check on the conformed result.
        let filter = Filter::new(TypeId::of::<i32>(), vec![], vec![], |_value, _args| {
            Ok(Box::new(1.5_f64) as Box<dyn Any>)
        });
        let result = sheet.add_filter(a, filter);
        assert!(matches!(result, Err(Error::TypeMismatch { .. })));
    }

    #[test]
    fn write_returns_type_mismatch_when_the_filters_function_returns_the_wrong_type() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        // Conforms correctly for the attach-time value (5), so `add_filter` succeeds,
        // but returns a `f64` for any other input, tripping `write`'s defensive check.
        let filter = Filter::new(TypeId::of::<i32>(), vec![], vec![], |value, _args| {
            let v = *value.downcast_ref::<i32>().unwrap();
            if v == 5 {
                Ok(Box::new(v) as Box<dyn Any>)
            } else {
                Ok(Box::new(1.5_f64) as Box<dyn Any>)
            }
        });
        sheet.add_filter(a, filter).unwrap();
        let result = sheet.write(a, 99_i32);
        assert!(matches!(result, Err(Error::TypeMismatch { .. })));
        // Rejected write: cell fully untouched.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    }

    #[test]
    fn write_conforms_a_value_through_the_cells_filter() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet.write(a, 500_i32).unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 100);
    }

    #[test]
    fn write_rejects_a_value_the_filter_cannot_conform() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_0(|x: &i32| {
                    if *x > 100 {
                        Err(anyhow::anyhow!("value exceeds maximum"))
                    } else {
                        Ok(*x)
                    }
                }),
            )
            .unwrap();
        let result = sheet.write(a, 500_i32);
        assert!(matches!(result, Err(Error::MethodFailed(_))));
        // Rejected write: cell fully untouched.
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    }

    #[test]
    fn write_without_a_filter_behaves_exactly_as_before() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        sheet.write(a, 42_i32).unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 42);
    }

    #[test]
    fn write_through_a_filter_still_bumps_strength() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).clamp(0, 100))))
            .unwrap();
        sheet.write(b, 1_i32).unwrap();
        sheet.write(a, 500_i32).unwrap();
        // `a` was written after `b`, so its strength must be higher even though its
        // stored value was conformed away from what was passed in.
        assert!(sheet.cells[a].strength > sheet.cells[b].strength);
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
        // `add_filter` re-checks the cell's *current* value immediately (see §3.2 of
        // the design), so a filter that unconditionally errors would reject at
        // attach time (b's initial value is 0) before propagate() ever runs. Accept
        // exactly 0 so attach succeeds, and let the relationship's derived value (1,
        // copied from `a`) be the one that trips the filter.
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
        // Accept anything up to `bound` so add_filter's own immediate re-check (against
        // a's current value, 5, and bound's current value, 100) succeeds; the write to
        // `bound` below is what trips the filter.
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
    fn propagate_without_replan_reapplies_a_cached_filter_reclamp_but_does_not_touch_last_filter_violations()
     {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(50_i32);
        let bound = sheet.add_cell(100_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |v: &i32, b: &i32| Ok((*v).min(*b))),
            )
            .unwrap();
        sheet.propagate().unwrap();
        assert!(sheet.filter_violation(a).is_none());

        // bound is itself a plain source (is_source(bound) holds), so rewriting it and
        // re-running only the cached plan is exactly propagate_without_replan's
        // documented precondition.
        sheet.write(bound, 10_i32).unwrap();
        sheet.propagate_without_replan().unwrap();

        assert_eq!(*sheet.read::<i32>(a).unwrap(), 10);
        // last_filter_violations is not recomputed by propagate_without_replan.
        assert!(sheet.filter_violation(a).is_none());
    }

    #[test]
    fn propagate_without_replan_does_not_recompute_filter_violations() {
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
        assert!(sheet.last_filter_violations.contains_key(&b));

        // Rewrite `a` back into range and re-run only the cached plan.
        sheet.write(a, 10_i32).unwrap();
        sheet.propagate_without_replan().unwrap();
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 20);
        // Still reports the *old* violation: propagate_without_replan doesn't
        // recompute it, matching last_violated's existing behavior.
        assert!(sheet.last_filter_violations.contains_key(&b));
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
}
