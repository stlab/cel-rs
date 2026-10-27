//! Checks retained bodies against a borrowed environment without executing their segments.
//!
//! The flat plan retains authored syntax and local ordinals for owning factory materialization.

#![allow(
    dead_code,
    reason = "checked plans are the handoff to the next materialization subgate"
)]

use std::{any::TypeId, collections::HashSet};

use cel_parser::{Expr, ExprSpan, OpLookup, compile_expr};
use cel_runtime::DynSegment;

use super::*;
use crate::{
    TypeRegistry,
    cell_value::{CellValue, RawFrame},
    type_registry::TypeShape,
};

/// Checks every registered definition using its declaring module and real source owner.
///
/// `module` must identify a registered module; checking includes unused definitions in
/// other registered modules, so the same collection has the same semantic validity for
/// every valid selection. Checks all authored directions before source specialization,
/// including inactive branches. No initializer, default, method, filter, requirement,
/// or match expression executes. Whole-graph planning and guard independence remain
/// construction-time checks; this operation does not require isolated solvability.
///
/// Returns diagnostics for unknown selection, names, complete types, bodies, bindings,
/// branch eligibility and source-specialized empty relationships. Labels own genuine
/// sources independently of `modules`.
///
/// - Complexity: O(n² + s) bookkeeping plus CEL compilation, registry resolution
///   and native preflight, where n is authored syntax and s is body edges in
///   distinct (definition, source-parameter-mask) specializations.
///
/// # Examples
/// ```
/// use adam_lang::{ModulePath, ModuleSet, TypeRegistry, check_module};
/// let path: ModulePath = "example".parse()?;
/// let mut modules = ModuleSet::new();
/// modules.insert_source(path.clone(), "example.adm2",
///     "sheet copy(a: i32, b: i32) { relationship { a := b; b := a; } }")?;
/// assert!(check_module(&path, &modules, &TypeRegistry::new(),
///     &cel_parser::OpLookup::new()).is_empty());
/// # Ok::<(), adam_lang::ModuleDiagnostic>(())
/// ```
pub fn check_module(
    module: &ModulePath,
    modules: &ModuleSet,
    types: &TypeRegistry,
    lookup: &OpLookup,
) -> Vec<ModuleDiagnostic> {
    if !modules.indices.contains_key(module) {
        return vec![crate::composition::diagnostic(format!(
            "unknown requested module `{module}`"
        ))];
    }
    match analyze(modules, types, lookup) {
        Ok(_) => Vec::new(),
        Err(errors) => errors,
    }
}

/// Distinguishes bound formals from allocating local declarations.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Parameter,
    Cell,
    Source,
    Out,
}

/// Retains one declaration's complete shape and genuine source label.
pub(super) struct Cell<'a> {
    pub name: &'a str,
    pub shape: TypeShape,
    pub kind: Kind,
    pub label: SourceLabel,
}

/// Retains checked syntax, positional reads, and its complete result descriptor.
pub(super) struct Expression<'a> {
    pub syntax: &'a Expr,
    pub inputs: Vec<usize>,
    pub candidate: Option<usize>,
    pub shape: TypeShape,
    pub descriptor: cel_runtime::ValueType,
    pub label: SourceLabel,
}

impl Expression<'_> {
    /// Emits fresh execution resources using checked positional bindings, without deducing names.
    ///
    /// - Precondition: `cells`, `types` and `lookup` belong to this expression's checked plan.
    /// # Errors
    /// Propagates checked operation emission failures without executing the result.
    /// - Complexity: CEL compilation plus O(nr) identifier comparisons for n names and r reads.
    pub(super) fn emit(
        &self,
        cells: &[Cell<'_>],
        types: &TypeRegistry,
        lookup: &OpLookup,
    ) -> Result<DynSegment, ModuleDiagnostic> {
        debug_assert!(
            self.inputs
                .iter()
                .chain(&self.candidate)
                .all(|cell| *cell < cells.len())
        );
        let mut segment = compile_expr(
            self.syntax,
            lookup,
            &types.cel_type_resolver(),
            &mut |name, segment, _| {
                if name == "_"
                    && let Some(candidate) = self.candidate
                {
                    segment.push_argument(0, types.checked_value_type(&cells[candidate].shape)?)?;
                    return Ok(true);
                }
                if let Some((index, cell)) = self
                    .inputs
                    .iter()
                    .enumerate()
                    .find(|(_, cell)| cells[**cell].name == name)
                {
                    segment.push_argument(
                        index + usize::from(self.candidate.is_some()),
                        types.checked_value_type(&cells[*cell].shape)?,
                    )?;
                    return Ok(true);
                }
                Ok(false)
            },
        )
        .map_err(|error| failure(error, self.label.clone()))?;
        segment
            .annotate_top(self.descriptor.clone())
            .map_err(|error| failure(error, self.label.clone()))?;
        Ok(segment)
    }
}

/// Retains a flat authored body node with its enclosing conditional/branch ordinal.
pub(super) struct Item<'a> {
    pub parent: Option<usize>,
    pub label: SourceLabel,
    pub body: Body<'a>,
}

/// Preserves item ordering, expression roles, and definition-local output ordinals.
pub(super) enum Body<'a> {
    Cell {
        cell: usize,
        initializer: Option<Expression<'a>>,
    },
    Out {
        cell: usize,
        writer: Expression<'a>,
    },
    Filter {
        cell: usize,
        expression: Expression<'a>,
        range: bool,
    },
    Requirement {
        cell: usize,
        name: Option<&'a str>,
        expression: Expression<'a>,
    },
    Relationship {
        methods: std::ops::Range<usize>,
    },
    Method {
        outputs: Vec<usize>,
        destructure: bool,
        expression: Expression<'a>,
    },
    Use {
        target: usize,
        bindings: Vec<usize>,
        syntax: &'a ast::RelationshipUse,
    },
    Conditional {
        expression: Expression<'a>,
    },
    Branch {
        syntax: Option<&'a ast::ConditionalBranch>,
    },
}

/// Owns one checked local body, or borrows an already validated native template.
pub(super) struct DefinitionPlan<'a> {
    pub cells: Vec<Cell<'a>>,
    pub names: HashMap<&'a str, usize>,
    pub formals: usize,
    pub items: Vec<Item<'a>>,
    pub native: Option<&'a TypedTemplate>,
    pub component: bool,
    pub label: SourceLabel,
    pub ineligible: Option<SourceLabel>,
}

/// Owns flat definition plans while borrowing syntax and the immutable compiler environment.
pub(super) struct Checked<'a> {
    pub resolved: resolve::Resolved<'a>,
    pub definitions: Vec<DefinitionPlan<'a>>,
    pub types: &'a TypeRegistry,
    pub lookup: &'a OpLookup,
}

/// Analyzes all signatures and bodies once, then checks statically known source masks.
///
/// # Errors
/// Returns authored diagnostics without executing retained expressions.
/// - Complexity: As [`check_module`], with O(n + s) plan and memoization storage.
pub(super) fn analyze<'a>(
    modules: &'a ModuleSet,
    types: &'a TypeRegistry,
    lookup: &'a OpLookup,
) -> Result<Checked<'a>, Vec<ModuleDiagnostic>> {
    let resolved = resolve::resolve(modules)?;
    let mut errors = Vec::new();
    let mut definitions = Vec::new();
    // Collect every complete signature before checking any body.
    for &(module, local) in &resolved.definitions {
        let definition = &modules.entries[module].definitions[local];
        let mut plan = DefinitionPlan {
            cells: Vec::new(),
            names: HashMap::new(),
            formals: 0,
            items: Vec::new(),
            native: None,
            component: true,
            label: definition.label().clone(),
            ineligible: None,
        };
        match &definition.body {
            DefinitionBody::Native(template) => {
                plan.native = Some(template);
                for (name, shape) in template.parameters() {
                    if let Err(error) = types.validate_shape(shape) {
                        errors.push(failure(error, plan.label.clone()));
                    }
                    plan.cells.push(Cell {
                        name,
                        shape: shape.clone(),
                        kind: Kind::Parameter,
                        label: plan.label.clone(),
                    });
                }
                if !template.is_relationship_only() {
                    plan.ineligible = Some(plan.label.clone());
                }
            }
            DefinitionBody::Parsed { sheet, .. } => {
                plan.component = sheet.parameters.is_some();
                let owner = modules.entries[module]
                    .source
                    .as_ref()
                    .expect("parsed source");
                for formal in sheet.parameters.iter().flatten() {
                    let site = label(owner, formal.name_span, "formal parameter");
                    if plan.cells.iter().any(|cell| cell.name == formal.name) {
                        errors.push(failure("duplicate parameter name", site));
                        continue;
                    }
                    match types.resolve(&formal.type_name) {
                        Ok(shape) => {
                            if let Err(error) = types
                                .checked_value_type(&shape)
                                .and_then(|ty| CellValue::validate(&ty))
                            {
                                errors.push(failure(error, site.clone()));
                            }
                            plan.cells.push(Cell {
                                name: &formal.name,
                                shape,
                                kind: Kind::Parameter,
                                label: site,
                            });
                        }
                        Err((error, _)) => errors.push(failure(error, site)),
                    }
                }
            }
        }
        plan.formals = plan.cells.len();
        plan.names.extend(
            plan.cells
                .iter()
                .enumerate()
                .map(|(id, cell)| (cell.name, id)),
        );
        definitions.push(plan);
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    // Dependency postorder makes transitive branch classification available at each use.
    for &id in &resolved.order {
        let (module, local) = resolved.definitions[id];
        if let DefinitionBody::Parsed { sheet, .. } =
            &modules.entries[module].definitions[local].body
        {
            let owner = modules.entries[module]
                .source
                .as_ref()
                .expect("parsed source");
            let mut checker = Checker {
                types,
                lookup,
                owner,
                definitions: &mut definitions,
                id,
                edge: 0,
                resolved: &resolved,
            };
            for item in &sheet.items {
                if let Err(error) = checker.item(item) {
                    errors.push(error);
                    break;
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let checked = Checked {
        resolved,
        definitions,
        types,
        lookup,
    };
    checked.sources()?;
    Ok(checked)
}

/// Borrows the evolving local namespace and completed dependency plans.
struct Checker<'a, 'b> {
    types: &'a TypeRegistry,
    lookup: &'a OpLookup,
    owner: &'a SourceId,
    definitions: &'b mut [DefinitionPlan<'a>],
    id: usize,
    edge: usize,
    resolved: &'b resolve::Resolved<'a>,
}

/// Attaches a semantic failure to an owned genuine source label.
///
/// - Complexity: O(n) formatted message bytes.
fn failure(message: impl fmt::Display, site: SourceLabel) -> ModuleDiagnostic {
    ModuleDiagnostic {
        message: message.to_string(),
        labels: vec![site],
    }
}

impl<'a> Checker<'a, '_> {
    /// Appends a local item and returns its stable ordinal.
    ///
    /// - Complexity: Amortized O(1), plus label bytes.
    fn push(&mut self, parent: Option<usize>, span: ExprSpan, body: Body<'a>) -> usize {
        let items = &mut self.definitions[self.id].items;
        let ordinal = items.len();
        items.push(Item {
            parent,
            label: label(self.owner, span, "body item"),
            body,
        });
        ordinal
    }

    /// Resolves an already declared local cell, without caller capture.
    ///
    /// # Errors
    /// Rejects names outside the declaration-order namespace.
    /// - Complexity: Expected O(n) name bytes.
    fn cell(&self, name: &str, span: ExprSpan) -> Result<usize, ModuleDiagnostic> {
        self.definitions[self.id]
            .names
            .get(name)
            .copied()
            .ok_or_else(|| {
                failure(
                    format!("undeclared cell `{name}`"),
                    label(self.owner, span, "cell reference"),
                )
            })
    }

    /// Compiles a retained expression with complete borrowed descriptors, without calling it.
    ///
    /// # Errors
    /// Rejects unknown names, invalid operations, and missing canonical capabilities.
    /// - Complexity: CEL compilation plus O(c × r) name/read bookkeeping.
    fn expression(
        &self,
        syntax: &'a Expr,
        capture: bool,
        candidate: Option<usize>,
    ) -> Result<(Expression<'a>, DynSegment, bool), ModuleDiagnostic> {
        let definition = &self.definitions[self.id];
        let cells = &definition.cells;
        let mut inputs = Vec::new();
        let mut used = false;
        let site = label(self.owner, syntax.span(), "expression");
        let segment = compile_expr(
            syntax,
            self.lookup,
            &self.types.cel_type_resolver(),
            &mut |name, segment, _| {
                if name == "_"
                    && let Some(cell) = candidate
                {
                    used = true;
                    segment.push_argument(0, self.types.checked_value_type(&cells[cell].shape)?)?;
                    return Ok(true);
                }
                if !capture {
                    anyhow::ensure!(
                        !definition.names.contains_key(name),
                        "cell-dependent initializers are unsupported"
                    );
                    return Ok(false);
                }
                let Some(&cell) = definition.names.get(name) else {
                    return Ok(false);
                };
                let index = if let Some(index) = inputs.iter().position(|input| *input == cell) {
                    index
                } else {
                    inputs.push(cell);
                    inputs.len() - 1
                };
                segment.push_argument(
                    index + usize::from(candidate.is_some()),
                    self.types.checked_value_type(&cells[cell].shape)?,
                )?;
                Ok(true)
            },
        )
        .map_err(|error| failure(error, site.clone()))?;
        let ty = segment
            .peek_stack_infos(1)
            .first()
            .ok_or_else(|| failure("expression produces no value", site.clone()))?
            .value_type
            .clone();
        let shape = if candidate.is_some()
            && let Some(id) = segment.peek_output_type_id()
            && self.types.range_entry(id).is_some()
        {
            TypeShape::Named(id)
        } else {
            self.types
                .shape_of(&ty)
                .map_err(|error| failure(error, site.clone()))?
        };
        Ok((
            Expression {
                syntax,
                inputs,
                candidate,
                shape,
                descriptor: ty,
                label: site,
            },
            segment,
            used,
        ))
    }

    /// Validates positional reads and complete results using the ordinary canonical frame.
    ///
    /// # Errors
    /// Rejects shape/capability mismatches without executing a frame.
    /// - Complexity: O(n + r) shape nodes and reads.
    fn frame(
        &self,
        expression: &Expression<'_>,
        segment: DynSegment,
        result: &TypeShape,
    ) -> Result<(), ModuleDiagnostic> {
        let cells = &self.definitions[self.id].cells;
        let inputs: Vec<_> = expression
            .candidate
            .iter()
            .chain(&expression.inputs)
            .map(|id| cells[*id].shape.clone())
            .collect();
        let result = self
            .types
            .checked_value_type(result)
            .map_err(|error| failure(error, expression.label.clone()))?;
        RawFrame::new(segment, &inputs, &result, self.types)
            .map(|_| ())
            .map_err(|error| failure(error, expression.label.clone()))
    }

    /// Adds a checked declaration after compiling its initializer in the permitted namespace.
    ///
    /// # Errors
    /// Rejects duplicate names, invalid shapes/defaults or initializer mismatches.
    /// - Complexity: O(c + n) namespace and descriptor nodes, plus CEL compilation.
    fn declaration(
        &mut self,
        name: &'a str,
        span: ExprSpan,
        annotation: Option<&ast::TypeExpr>,
        initializer: Option<&'a Expr>,
        kind: Kind,
    ) -> Result<usize, ModuleDiagnostic> {
        let site = label(self.owner, span, "cell declaration");
        if self.definitions[self.id].names.contains_key(name) {
            return Err(failure(format!("duplicate cell `{name}`"), site));
        }
        let compiled = initializer
            .map(|syntax| self.expression(syntax, kind == Kind::Out, None))
            .transpose()?;
        let shape = if let Some(annotation) = annotation {
            self.types
                .resolve(annotation)
                .map_err(|(error, _)| failure(error, site.clone()))?
        } else {
            compiled
                .as_ref()
                .expect("parsed declaration has type or initializer")
                .0
                .shape
                .clone()
        };
        let ty = self
            .types
            .checked_value_type(&shape)
            .map_err(|error| failure(error, site.clone()))?;
        CellValue::validate(&ty).map_err(|error| failure(error, site.clone()))?;
        if let Some((expression, _, _)) = &compiled {
            if expression.shape != shape {
                return Err(failure(
                    "initializer type mismatch",
                    expression.label.clone(),
                ));
            }
        } else {
            self.types
                .validate_default(&shape)
                .map_err(|error| failure(error, site.clone()))?;
        }
        let id = self.definitions[self.id].cells.len();
        self.definitions[self.id].cells.push(Cell {
            name,
            shape: shape.clone(),
            kind,
            label: site.clone(),
        });
        self.definitions[self.id].names.insert(name, id);
        self.definitions[self.id].ineligible.get_or_insert(site);
        let body = if let Some((expression, segment, _)) = compiled {
            self.frame(&expression, segment, &shape)?;
            if kind == Kind::Out {
                Body::Out {
                    cell: id,
                    writer: expression,
                }
            } else {
                Body::Cell {
                    cell: id,
                    initializer: Some(expression),
                }
            }
        } else {
            Body::Cell {
                cell: id,
                initializer: None,
            }
        };
        self.push(None, span, body);
        Ok(id)
    }

    /// Checks the ordinary filter and requirement contracts in declaration order.
    ///
    /// # Errors
    /// Rejects missing filter candidates, mismatched ranges/results or non-bool requirements.
    /// - Complexity: Expression compilation plus O(n) policy descriptors.
    fn policies(
        &mut self,
        cell: usize,
        filter: Option<&'a ast::CellFilter>,
        requirements: Option<&'a ast::RequireBlock>,
    ) -> Result<(), ModuleDiagnostic> {
        let shape = self.definitions[self.id].cells[cell].shape.clone();
        if let Some(filter) = filter {
            let (expression, segment, used) = self.expression(&filter.body, true, Some(cell))?;
            if expression.inputs.contains(&cell) {
                return Err(failure(
                    "filter cannot depend on its own cell",
                    expression.label,
                ));
            }
            let range = segment
                .peek_output_type_id()
                .and_then(|id| self.types.range_entry(id));
            let is_range = range.is_some();
            if let Some(range) = range {
                if range.element_type_id != crate::parser::cell_type_id(&shape) {
                    return Err(failure("filter range element mismatch", expression.label));
                }
                let result = segment.peek_stack_infos(1)[0].value_type.clone();
                let cells = &self.definitions[self.id].cells;
                let inputs: Vec<_> = expression
                    .candidate
                    .iter()
                    .chain(&expression.inputs)
                    .map(|id| cells[*id].shape.clone())
                    .collect();
                RawFrame::new(segment, &inputs, &result, self.types)
                    .map_err(|error| failure(error, expression.label.clone()))?;
            } else {
                if !used {
                    return Err(failure("value filter must reference `_`", expression.label));
                }
                self.frame(&expression, segment, &shape)?;
            }
            self.push(
                None,
                filter.span,
                Body::Filter {
                    cell,
                    expression,
                    range: is_range,
                },
            );
        }
        for requirement in requirements
            .into_iter()
            .flat_map(|block| &block.requirements)
        {
            let (expression, segment, _) = self.expression(&requirement.body, true, None)?;
            self.frame(
                &expression,
                segment,
                &TypeShape::Named(TypeId::of::<bool>()),
            )?;
            self.push(
                None,
                requirement.span,
                Body::Requirement {
                    cell,
                    name: requirement.name.as_deref(),
                    expression,
                },
            );
        }
        Ok(())
    }

    /// Checks a complete ordinary relationship before any source-based pruning.
    ///
    /// # Errors
    /// Rejects invalid targets, shapes and ordinary relationship edge structure.
    /// - Complexity: O(m²e) structural checks plus expression compilation.
    fn relationship(
        &mut self,
        relationship: &'a ast::RelationshipDecl,
        parent: Option<usize>,
    ) -> Result<(), ModuleDiagnostic> {
        let ordinal = self.push(
            parent,
            relationship.span,
            Body::Relationship { methods: 0..0 },
        );
        let mut edges = Vec::new();
        for binding in &relationship.bindings {
            let outputs = binding
                .outputs
                .iter()
                .map(|(name, span)| self.cell(name, *span))
                .collect::<Result<Vec<_>, _>>()?;
            if outputs
                .iter()
                .any(|id| self.definitions[self.id].cells[*id].kind == Kind::Out)
            {
                return Err(failure(
                    "out cell already has its sole writer",
                    label(self.owner, binding.span, "method"),
                ));
            }
            let (expression, segment, _) = self.expression(&binding.body, true, None)?;
            let shapes: Vec<_> = outputs
                .iter()
                .map(|id| self.definitions[self.id].cells[*id].shape.clone())
                .collect();
            let result = if binding.destructure {
                TypeShape::Tuple(shapes)
            } else {
                shapes[0].clone()
            };
            self.frame(&expression, segment, &result)?;
            edges.push((expression.inputs.clone(), outputs.clone()));
            self.push(
                Some(ordinal),
                binding.span,
                Body::Method {
                    outputs,
                    destructure: binding.destructure,
                    expression,
                },
            );
        }
        let views: Vec<_> = edges
            .iter()
            .map(|(inputs, outputs)| (inputs.as_slice(), outputs.as_slice()))
            .collect();
        adam_rs::template::check_relationship_edges(&views).map_err(|error| {
            failure(error, label(self.owner, relationship.span, "relationship"))
        })?;
        let end = self.definitions[self.id].items.len();
        self.definitions[self.id].items[ordinal].body = Body::Relationship {
            methods: ordinal + 1..end,
        };
        Ok(())
    }

    /// Checks complete named bindings against a dependency's checked signature.
    ///
    /// # Errors
    /// Rejects missing/unknown/repeated/aliased/out arguments, shapes and ineligible branches.
    /// - Complexity: O(a × (f + c)) actuals, formals and caller cells.
    fn invocation(
        &mut self,
        invocation: &'a ast::RelationshipUse,
        parent: Option<usize>,
    ) -> Result<(), ModuleDiagnostic> {
        let target = self.resolved.edges[self.id][self.edge].target;
        self.edge += 1;
        let child = &self.definitions[target];
        let site = label(self.owner, invocation.span, "component use");
        if parent.is_some()
            && let Some(ineligible) = &child.ineligible
        {
            return Err(ModuleDiagnostic {
                message: "conditional use requires a relationship-only component".into(),
                labels: vec![site, ineligible.clone()],
            });
        }
        let mut bindings = vec![None; child.formals];
        let mut actuals = HashSet::new();
        for argument in &invocation.arguments {
            let formal = child.cells[..child.formals]
                .iter()
                .position(|cell| cell.name == argument.name)
                .ok_or_else(|| {
                    failure(
                        format!("unknown parameter `{}`", argument.name),
                        label(self.owner, argument.name_span, "named argument"),
                    )
                })?;
            let actual = self.cell(&argument.cell, argument.cell_span)?;
            if bindings[formal].replace(actual).is_some() || !actuals.insert(actual) {
                return Err(failure("repeated parameter or aliased actual cell", site));
            }
            let cell = &self.definitions[self.id].cells[actual];
            if cell.kind == Kind::Out {
                return Err(failure("out-cell arguments are unsupported", site));
            }
            if cell.shape != child.cells[formal].shape {
                return Err(ModuleDiagnostic {
                    message: "component argument type mismatch".into(),
                    labels: vec![
                        label(self.owner, argument.cell_span, "actual cell"),
                        child.cells[formal].label.clone(),
                    ],
                });
            }
        }
        let bindings = bindings
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| failure("missing component parameter", site.clone()))?;
        let ineligible = child.ineligible.clone();
        self.push(
            parent,
            invocation.span,
            Body::Use {
                target,
                bindings,
                syntax: invocation,
            },
        );
        if let Some(ineligible) = ineligible {
            self.definitions[self.id]
                .ineligible
                .get_or_insert(ineligible);
        }
        Ok(())
    }

    /// Checks each branch key and every inactive or active branch body.
    ///
    /// # Errors
    /// Rejects match/key mismatches and invalid branch relationships or uses.
    /// - Complexity: Expression compilation and authored branch traversal.
    fn conditional(
        &mut self,
        conditional: &'a ast::ConditionalDecl,
    ) -> Result<(), ModuleDiagnostic> {
        let (expression, segment, _) = self.expression(&conditional.match_expr, true, None)?;
        let shape = expression.shape.clone();
        CellValue::validate(&self.types.value_type(&shape))
            .map_err(|error| failure(error, expression.label.clone()))?;
        self.frame(&expression, segment, &shape)?;
        let ordinal = self.push(None, conditional.span, Body::Conditional { expression });
        self.definitions[self.id]
            .ineligible
            .get_or_insert_with(|| label(self.owner, conditional.span, "conditional declaration"));
        for branch in &conditional.branches {
            let literal = cel_parser::literal_expr(&branch.literal).map_err(|error| {
                failure(error, label(self.owner, branch.literal_span, "branch key"))
            })?;
            let literal = if branch.negated {
                Expr::Op {
                    name: "-".into(),
                    operands: vec![literal],
                    span: branch.literal_span,
                }
            } else {
                literal
            };
            // Literal descriptors are checked without evaluating keys or user match code.
            let segment = compile_expr(
                &literal,
                self.lookup,
                &self.types.cel_type_resolver(),
                &mut |_, _, _| Ok(false),
            )
            .map_err(|error| {
                failure(error, label(self.owner, branch.literal_span, "branch key"))
            })?;
            let actual = self
                .types
                .shape_of(&segment.peek_stack_infos(1)[0].value_type)
                .map_err(|error| {
                    failure(error, label(self.owner, branch.literal_span, "branch key"))
                })?;
            if actual != shape {
                return Err(failure(
                    "conditional branch type mismatch",
                    label(self.owner, branch.literal_span, "branch key"),
                ));
            }
            let branch_id = self.push(
                Some(ordinal),
                branch.span,
                Body::Branch {
                    syntax: Some(branch),
                },
            );
            self.branch(&branch.relationships, branch_id)?;
        }
        if let Some(default) = &conditional.default {
            let branch_id = self.push(Some(ordinal), default.span, Body::Branch { syntax: None });
            self.branch(&default.relationships, branch_id)?;
        }
        Ok(())
    }

    /// Checks a branch's authored relationship order.
    ///
    /// # Errors
    /// Returns the first invalid branch item.
    /// - Complexity: Sum of branch-item checks.
    fn branch(
        &mut self,
        items: &'a [ast::BranchItem],
        parent: usize,
    ) -> Result<(), ModuleDiagnostic> {
        for item in items {
            match item {
                ast::BranchItem::Relationship(item) => self.relationship(item, Some(parent))?,
                ast::BranchItem::RelationshipUse(item) => self.invocation(item, Some(parent))?,
            }
        }
        Ok(())
    }

    /// Checks one top-level item in its declaration-order namespace.
    ///
    /// # Errors
    /// Returns the item's semantic diagnostic.
    /// - Complexity: The selected body check's complexity.
    fn item(&mut self, item: &'a ast::SheetItem) -> Result<(), ModuleDiagnostic> {
        match item {
            ast::SheetItem::Cell(item) => {
                let cell = self.declaration(
                    &item.name,
                    item.name_span,
                    item.type_name.as_ref(),
                    item.initializer.as_ref(),
                    Kind::Cell,
                )?;
                self.policies(cell, item.filter.as_ref(), item.require.as_ref())
            }
            ast::SheetItem::Source(item) => {
                let cell = self.declaration(
                    &item.name,
                    item.name_span,
                    item.type_name.as_ref(),
                    item.initializer.as_ref(),
                    Kind::Source,
                )?;
                self.policies(cell, item.filter.as_ref(), item.require.as_ref())
            }
            ast::SheetItem::Out(item) => {
                let cell = self.declaration(
                    &item.name,
                    item.name_span,
                    item.type_name.as_ref(),
                    Some(&item.initializer),
                    Kind::Out,
                )?;
                self.policies(cell, item.filter.as_ref(), item.require.as_ref())
            }
            ast::SheetItem::Relationship(item) => self.relationship(item, None),
            ast::SheetItem::RelationshipUse(item) => self.invocation(item, None),
            ast::SheetItem::Conditional(item) => self.conditional(item),
            ast::SheetItem::Error { span, .. } => Err(failure(
                "invalid body syntax",
                label(self.owner, *span, "body"),
            )),
        }
    }
}

/// Retains one pending source specialization and an ordinal-only invocation ancestry.
struct SourceVisit {
    definition: usize,
    parameters: Vec<bool>,
    parent: Option<usize>,
    site: SourceLabel,
}

/// Labels an outer-to-inner source-specialization path without recursively owned metadata.
///
/// - Precondition: Each parent ordinal precedes its child and `visit` is in bounds.
/// - Complexity: O(h) labels for invocation depth h.
fn source_labels(
    visits: &[SourceVisit],
    mut visit: usize,
    terminal: SourceLabel,
) -> Vec<SourceLabel> {
    debug_assert!(visit < visits.len());
    let mut labels = Vec::new();
    loop {
        labels.push(visits[visit].site.clone());
        let Some(parent) = visits[visit].parent else {
            break;
        };
        debug_assert!(parent < visit);
        visit = parent;
    }
    labels.reverse();
    labels.push(terminal);
    labels
}

impl Checked<'_> {
    /// Checks every distinct source specialization with an iterative memoized worklist.
    ///
    /// # Errors
    /// Rejects whole-method pruning that empties any constituent, even in inactive branches.
    /// - Complexity: O(s) specialized body edges, plus native source preflight.
    fn sources(&self) -> Result<(), Vec<ModuleDiagnostic>> {
        let mut visits: Vec<_> = self
            .definitions
            .iter()
            .enumerate()
            .map(|(id, definition)| SourceVisit {
                definition: id,
                parameters: vec![false; definition.formals],
                parent: None,
                site: definition.label.clone(),
            })
            .collect();
        let mut seen = HashSet::new();
        let mut errors = Vec::new();
        let mut next = 0;
        while next < visits.len() {
            let visit = next;
            next += 1;
            let id = visits[visit].definition;
            let parameters = &visits[visit].parameters;
            if !seen.insert((id, parameters.clone())) {
                continue;
            }
            let definition = &self.definitions[id];
            if let Some(native) = definition.native {
                let bindings: Vec<_> = definition.cells[..definition.formals]
                    .iter()
                    .zip(parameters)
                    .map(|(cell, source)| (cell.name, *source))
                    .collect();
                if let Err(error) = native.check_source_bindings(&bindings) {
                    errors.push(ModuleDiagnostic {
                        message: error.to_string(),
                        labels: source_labels(&visits, visit, definition.label.clone()),
                    });
                }

                continue;
            }
            let sources: Vec<_> = definition
                .cells
                .iter()
                .enumerate()
                .map(|(cell, declaration)| {
                    declaration.kind == Kind::Source
                        || parameters.get(cell).copied().unwrap_or(false)
                })
                .collect();
            for item in &definition.items {
                match &item.body {
                    Body::Relationship { methods } => {
                        let survives = definition.items[methods.clone()].iter().any(|method| {
                            matches!(&method.body, Body::Method { outputs, .. } if outputs.iter().all(|cell| !sources[*cell]))
                        });
                        if !survives {
                            errors.push(ModuleDiagnostic {
                                message: "source specialization leaves an empty relationship"
                                    .into(),
                                labels: source_labels(&visits, visit, item.label.clone()),
                            });
                        }
                    }
                    Body::Use {
                        target, bindings, ..
                    } => {
                        visits.push(SourceVisit {
                            definition: *target,
                            parameters: bindings.iter().map(|cell| sources[*cell]).collect(),
                            parent: Some(visit),
                            site: item.label.clone(),
                        });
                    }
                    _ => {}
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell_value::Frame;

    /// Emits candidate-first filters with distinct, first-use-ordered, repeated dependencies.
    #[test]
    fn checked_emission_preserves_filter_candidate_and_dependency_order() {
        let mut modules = ModuleSet::new();
        modules
            .insert_source(
                "filters".parse().unwrap(),
                "filters.adm2",
                "
            sheet filters {
                cell left: i32;
                cell right: i32;
                cell first: i32 filter right * 100 + _ * 10 + left + right;
                cell second: i32 filter left * 100 + right * 10 + _ + left;
            }
        ",
            )
            .unwrap();
        let types = TypeRegistry::new();
        let lookup = OpLookup::new();
        let checked =
            analyze(&modules, &types, &lookup).unwrap_or_else(|errors| panic!("{errors:?}"));
        let definition = &checked.definitions[0];
        let scalar = TypeShape::Named(TypeId::of::<i32>());
        let inputs = vec![scalar.clone(); 3];
        let mut filters = 0;
        for item in &definition.items {
            let Body::Filter {
                cell,
                expression,
                range,
            } = &item.body
            else {
                continue;
            };
            assert!(!range);
            assert_eq!(expression.candidate, Some(*cell));
            let (names, cases) = match definition.cells[*cell].name {
                "first" => (["right", "left"], [(3, 5, 7, 542), (11, 13, 17, 1440)]),
                "second" => (["left", "right"], [(3, 7, 5, 760), (11, 17, 13, 1858)]),
                name => panic!("unexpected filter {name}"),
            };
            assert_eq!(
                expression
                    .inputs
                    .iter()
                    .map(|input| definition.cells[*input].name)
                    .collect::<Vec<_>>(),
                names
            );
            let mut frame = Frame::new(
                expression.emit(&definition.cells, &types, &lookup).unwrap(),
                &inputs,
                std::slice::from_ref(&scalar),
                false,
                &types,
            )
            .unwrap();
            for (candidate, first, second, expected) in cases {
                let result = frame
                    .call(&[&candidate, &first, &second])
                    .unwrap()
                    .pop()
                    .unwrap();
                assert_eq!(*result.downcast::<i32>().unwrap(), expected);
            }
            assert!(
                Frame::new(
                    expression.emit(&definition.cells, &types, &lookup).unwrap(),
                    &inputs,
                    &[TypeShape::Named(TypeId::of::<f64>())],
                    false,
                    &types
                )
                .is_err()
            );
            filters += 1;
        }
        assert_eq!(filters, 2);
    }

    /// Re-emits an exact range descriptor with candidate and repeated reordered input reads.
    #[test]
    fn checked_emission_retains_exact_range_result() {
        let mut modules = ModuleSet::new();
        modules
            .insert_source(
                "ranges".parse().unwrap(),
                "ranges.adm2",
                "
            sheet ranges {
                cell left: i32;
                cell right: i32;
                cell bounded: i32 filter (right - _ + left - left)..=(left + right - left);
            }
        ",
            )
            .unwrap();
        let types = TypeRegistry::new();
        let lookup = OpLookup::new();
        let checked =
            analyze(&modules, &types, &lookup).unwrap_or_else(|errors| panic!("{errors:?}"));
        let definition = &checked.definitions[0];
        let (cell, expression, range) = definition
            .items
            .iter()
            .find_map(|item| match &item.body {
                Body::Filter {
                    cell,
                    expression,
                    range,
                } => Some((*cell, expression, *range)),
                _ => None,
            })
            .unwrap();
        assert!(range);
        assert_eq!(expression.candidate, Some(cell));
        assert_eq!(
            expression
                .inputs
                .iter()
                .map(|input| definition.cells[*input].name)
                .collect::<Vec<_>>(),
            ["right", "left"]
        );
        let expected = cel_runtime::ValueType::cloneable_leaf::<std::ops::RangeInclusive<i32>>();
        let wrong = cel_runtime::ValueType::cloneable_leaf::<std::ops::RangeInclusive<f64>>();
        assert!(expression.descriptor.same_shape(&expected));
        assert!(!expression.descriptor.same_shape(&wrong));
        let inputs = vec![TypeShape::Named(TypeId::of::<i32>()); 3];
        let emitted = expression.emit(&definition.cells, &types, &lookup).unwrap();
        assert!(
            emitted.peek_stack_infos(1)[0]
                .value_type
                .same_shape(&expected)
        );
        let mut frame = RawFrame::new(emitted, &inputs, &expected, &types).unwrap();
        for (candidate, right, left, expected) in
            [(3i32, 20i32, 5i32, 17..=20), (7, 40, 11, 33..=40)]
        {
            let result = frame
                .call(&[&candidate, &right, &left])
                .unwrap()
                .try_into_typed::<cel_runtime::Leaf<std::ops::RangeInclusive<i32>>>()
                .unwrap();
            assert_eq!(result, expected);
        }
        assert!(
            RawFrame::new(
                expression.emit(&definition.cells, &types, &lookup).unwrap(),
                &inputs,
                &wrong,
                &types
            )
            .is_err()
        );
    }

    /// Executes one-field destructuring separately from assigning a whole one-element tuple.
    #[test]
    fn checked_emission_preserves_one_element_output_patterns() {
        let mut modules = ModuleSet::new();
        modules
            .insert_source(
                "patterns".parse().unwrap(),
                "patterns.adm2",
                "
            sheet patterns(a: i32, b: i32, whole: (i32,)) {
                relationship { (a,) := (b,); }
                relationship { whole := (b,); }
            }
        ",
            )
            .unwrap();
        let types = TypeRegistry::new();
        let lookup = OpLookup::new();
        let checked =
            analyze(&modules, &types, &lookup).unwrap_or_else(|errors| panic!("{errors:?}"));
        let definition = &checked.definitions[0];
        let scalar = TypeShape::Named(TypeId::of::<i32>());
        let mut methods = 0;
        for item in &definition.items {
            let Body::Method {
                outputs,
                destructure,
                expression,
            } = &item.body
            else {
                continue;
            };
            assert_eq!(outputs.len(), 1);
            let output = &definition.cells[outputs[0]];
            assert_eq!(*destructure, output.name == "a");
            let mut frame = Frame::new(
                expression.emit(&definition.cells, &types, &lookup).unwrap(),
                std::slice::from_ref(&scalar),
                std::slice::from_ref(&output.shape),
                *destructure,
                &types,
            )
            .unwrap();
            for input in [19i32, 31] {
                let result = frame.call(&[&input]).unwrap().pop().unwrap();
                let actual = if *destructure {
                    *result.downcast::<i32>().unwrap()
                } else {
                    let compound = *result.downcast::<CellValue>().unwrap();
                    let mut fields = compound.0.into_fields().unwrap();
                    assert_eq!(fields.len(), 1);
                    fields
                        .pop()
                        .unwrap()
                        .try_into_typed::<cel_runtime::Leaf<i32>>()
                        .unwrap()
                };
                assert_eq!(actual, input);
            }
            assert!(
                Frame::new(
                    expression.emit(&definition.cells, &types, &lookup).unwrap(),
                    std::slice::from_ref(&scalar),
                    std::slice::from_ref(&output.shape),
                    !*destructure,
                    &types
                )
                .is_err()
            );
            assert!(
                Frame::new(
                    expression.emit(&definition.cells, &types, &lookup).unwrap(),
                    std::slice::from_ref(&scalar),
                    &[TypeShape::Named(TypeId::of::<f64>())],
                    *destructure,
                    &types
                )
                .is_err()
            );
            methods += 1;
        }
        assert_eq!(methods, 2);
    }

    /// Retains borrowed environments, local ordinals, syntax, direction order and real labels.
    #[test]
    fn checked_plan_retains_factory_inputs_without_expansion() {
        let path: ModulePath = "plans".parse().unwrap();
        let mut modules = ModuleSet::new();
        modules.insert_source(path, "plans.adm2", "sheet copy(a: i32, b: i32) { relationship { a := b; b := a; } } sheet wrapper(x: i32, y: i32) { cell hidden: [i32]; relationship copy(b: y, a: x); conditional true { true => { relationship copy(a: x, b: y); } } }").unwrap();
        let types = TypeRegistry::new();
        let lookup = OpLookup::new();
        let checked =
            analyze(&modules, &types, &lookup).unwrap_or_else(|errors| panic!("{errors:?}"));
        assert!(std::ptr::eq(checked.types, &types));
        assert!(std::ptr::eq(checked.lookup, &lookup));
        assert_eq!(checked.definitions.len(), 2);
        assert_eq!(checked.resolved.order, [0, 1]);
        let copy = &checked.definitions[0];
        assert_eq!(
            copy.cells.iter().map(|cell| cell.name).collect::<Vec<_>>(),
            ["a", "b"]
        );
        let Body::Method {
            outputs,
            expression,
            ..
        } = &copy.items[1].body
        else {
            panic!("method plan");
        };
        assert_eq!(outputs, &[0]);
        assert_eq!(expression.inputs, [1]);
        let emitted = expression.emit(&copy.cells, &types, &lookup).unwrap();
        assert!(
            emitted.peek_stack_infos(1)[0]
                .value_type
                .same_shape(&expression.descriptor)
        );
        let (module, local) = checked.resolved.definitions[0];
        let DefinitionBody::Parsed { sheet, .. } = &modules.entries[module].definitions[local].body
        else {
            panic!("parsed");
        };
        let ast::SheetItem::Relationship(relationship) = &sheet.items[0] else {
            panic!("relationship");
        };
        assert!(std::ptr::eq(
            expression.syntax,
            &relationship.bindings[0].body
        ));
        let wrapper = &checked.definitions[1];
        assert_eq!(wrapper.formals, 2);
        assert_eq!(wrapper.cells.len(), 3);
        let uses: Vec<_> = wrapper
            .items
            .iter()
            .filter_map(|item| match &item.body {
                Body::Use {
                    target, bindings, ..
                } => Some((*target, bindings.as_slice())),
                _ => None,
            })
            .collect();
        assert_eq!(uses, [(0, &[0, 1][..]), (0, &[0, 1][..])]);
        assert!(
            wrapper
                .items
                .iter()
                .any(|item| item.parent.is_some() && matches!(item.body, Body::Use { .. }))
        );
        for definition in &checked.definitions {
            for site in definition
                .cells
                .iter()
                .map(|cell| &cell.label)
                .chain(definition.items.iter().map(|item| &item.label))
            {
                assert_eq!(
                    site.location.as_ref().unwrap().source.file_name(),
                    "plans.adm2"
                );
                assert_eq!(
                    &site.location.as_ref().unwrap().source,
                    modules.entries[module].source.as_ref().unwrap()
                );
            }
        }
    }
}
