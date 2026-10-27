//! Materializes borrowed checked plans into immutable, repeatable native factories.
//!
//! ```
//! use adam_lang::{ModuleCompiler, ModuleSet, TypeRegistry};
//! let path = "example".parse()?;
//! let mut modules = ModuleSet::new();
//! modules.insert_source(path, "example.adm2", "sheet reusable() {}")?;
//! let compiled = ModuleCompiler::new(TypeRegistry::new(), cel_parser::OpLookup::new())
//!     .compile(modules).map_err(|mut errors| errors.remove(0))?;
//! assert!(compiled.template(&"example".parse()?, "reusable").is_some());
//! # Ok::<(), adam_lang::ModuleDiagnostic>(())
//! ```

use std::{collections::HashSet, rc::Rc};

use adam_rs::{CellKind, RelationshipSlot};
use cel_parser::{Expr, OpLookup};
use cel_runtime::{DynSegment, ValueType};

use super::{
    analysis::{self, Body, DefinitionPlan, Kind},
    *,
};
use crate::{
    CompositionBuilder, TypeRegistry,
    cell_value::{CellValue, RawFrame},
    composition::{AuthoredItem, AuthoredRole, CellSlot, MethodRecipe},
    type_registry::TypeShape,
};

/// Shares the immutable, noncloneable operation environment between owning factories.
pub(crate) struct Environment {
    pub(crate) types: Rc<TypeRegistry>,
    pub(crate) lookup: OpLookup,
}

/// Owns the type and operation environments for one collection-wide compilation.
///
/// Compilation does not execute user expressions or construct live graphs.
/// See [`Self::new`] for an example.
pub struct ModuleCompiler {
    environment: Rc<Environment>,
}

impl ModuleCompiler {
    /// Takes ownership of the immutable environments without cloning either.
    ///
    /// # Examples
    /// ```
    /// use adam_lang::{ModuleCompiler, ModuleSet, TypeRegistry};
    /// let mut modules = ModuleSet::new();
    /// modules.insert_source("example".parse()?, "example.adm2", "sheet reusable() {}")?;
    /// let compiled = ModuleCompiler::new(TypeRegistry::new(), cel_parser::OpLookup::new())
    ///     .compile(modules).map_err(|mut errors| errors.remove(0))?;
    /// assert!(compiled.template(&"example".parse()?, "reusable").is_some());
    /// # Ok::<(), adam_lang::ModuleDiagnostic>(())
    /// ```
    pub fn new(types: TypeRegistry, lookup: OpLookup) -> Self {
        Self {
            environment: Rc::new(Environment {
                types: Rc::new(types),
                lookup,
            }),
        }
    }

    /// Checks the collection once and materializes every definition in dependency order.
    ///
    /// All definitions, including unused definitions and inactive branches, must check.
    /// Initializers, defaults, policies, operations and matches remain unexecuted.
    /// Whole-graph solvability and live-root construction are not compilation checks.
    /// # Errors
    /// Returns source-owning diagnostics for invalid collections or recipe preparation.
    /// No partial compiled collection escapes on failure.
    /// - Complexity: Borrowed analysis plus expression emission, native recipe validation
    ///   and O(n) owning syntax/metadata storage in authored nodes, not expanded uses.
    ///
    /// # Examples
    /// See [`Self::new`].
    pub fn compile(self, modules: ModuleSet) -> Result<CompiledModules, Vec<ModuleDiagnostic>> {
        compile(self.environment, modules)
    }
}

/// Retains one root's own declaration, never an included private name.
///
/// Native slots belong to the corresponding prepared root's immutable recipe.
/// See [`CompiledModules::prepared_root`].
#[derive(Clone, Debug)]
pub struct RootCell {
    /// Retains the authored root-local name.
    pub name: String,
    /// Retains the native kind after private-output promotion.
    pub kind: CellKind,
    /// Retains the complete language shape.
    pub shape: TypeShape,
    /// Identifies the local cell in the root occurrence.
    pub slot: adam_rs::CellSlot,
    /// Retains the exact declaration source.
    pub label: SourceLabel,
}

/// Owns a lowered root and its public-name assembly inputs, without constructing it.
///
/// Task 6 consumes the internal recipe through the same native instantiator as components.
/// See [`CompiledModules::prepared_root`].
pub struct PreparedRoot {
    pub(crate) template: TypedTemplate,
    cells: Vec<RootCell>,
}

impl PreparedRoot {
    /// Borrows authored root-local declarations in source order.
    ///
    /// Included private cells never enter this slice.
    /// # Examples
    /// See [`CompiledModules::prepared_root`].
    pub fn cells(&self) -> &[RootCell] {
        &self.cells
    }

    /// Borrows the genuine root definition label.
    ///
    /// # Examples
    /// See [`CompiledModules::prepared_root`].
    pub fn definition_label(&self) -> &SourceLabel {
        self.template.definition_label()
    }
}

/// Owns resolved immutable exports and explicitly selectable prepared roots.
///
/// Imported aliases do not become exports. Factories and diagnostics retain their
/// actual source owners independently of this collection.
/// See [`ModuleCompiler::new`] for an example.
pub struct CompiledModules {
    templates: HashMap<ModulePath, HashMap<String, TypedTemplate>>,
    roots: HashMap<ModulePath, Vec<(String, PreparedRoot)>>,
    sources: Vec<SourceId>,
    /// Keeps the immutable entry available to Task 6 without cloning a registry/lookup.
    pub(crate) environment: Rc<Environment>,
}

impl CompiledModules {
    /// Borrows source identities from supplied files and imported immutable templates.
    ///
    /// Includes empty source modules. Independent sources keep their ownership identity
    /// even when file names, text or original collection indices coincide.
    /// # Examples
    /// ```
    /// let mut modules = adam_lang::ModuleSet::new();
    /// modules.insert_source("empty".parse()?, "empty.adm2", "")?;
    /// let compiled = adam_lang::ModuleCompiler::new(adam_lang::TypeRegistry::new(),
    ///     cel_parser::OpLookup::new()).compile(modules).map_err(|mut e| e.remove(0))?;
    /// assert_eq!(compiled.sources()[0].file_name(), "empty.adm2");
    /// # Ok::<(), adam_lang::ModuleDiagnostic>(())
    /// ```
    pub fn sources(&self) -> &[SourceId] {
        &self.sources
    }

    /// Creates a compiler sharing only this collection's immutable environments.
    ///
    /// New compilations retain independent module collections and construction frames.
    /// # Examples
    /// ```
    /// let compiled = adam_lang::ModuleCompiler::new(adam_lang::TypeRegistry::new(),
    ///     cel_parser::OpLookup::new()).compile(adam_lang::ModuleSet::new())
    ///     .map_err(|mut errors| errors.remove(0))?;
    /// let other = compiled.compiler().compile(adam_lang::ModuleSet::new());
    /// assert!(other.is_ok());
    /// # Ok::<(), adam_lang::ModuleDiagnostic>(())
    /// ```
    pub fn compiler(&self) -> ModuleCompiler {
        ModuleCompiler {
            environment: self.environment.clone(),
        }
    }

    /// Borrows a declared reusable export, excluding roots and imported aliases.
    ///
    /// - Complexity: Expected O(p + n) module-path and definition-name bytes.
    /// # Examples
    /// See [`ModuleCompiler::new`].
    pub fn template(&self, module: &ModulePath, name: &str) -> Option<&TypedTemplate> {
        self.templates.get(module)?.get(name)
    }

    /// Enumerates explicitly selectable root names in authored order.
    ///
    /// An unknown module has no roots; no arbitrary root is selected.
    /// - Complexity: O(r) traversal for r roots.
    /// # Examples
    /// See [`Self::prepared_root`].
    pub fn roots<'a>(&'a self, module: &ModulePath) -> impl Iterator<Item = &'a str> {
        self.roots
            .get(module)
            .into_iter()
            .flatten()
            .map(|(name, _)| name.as_str())
    }

    /// Borrows one explicit root's lowered recipe metadata without instantiating it.
    ///
    /// - Complexity: O(r + p + n) root/name comparisons and module-path bytes.
    /// # Examples
    /// ```
    /// let mut modules = adam_lang::ModuleSet::new();
    /// let path = "app".parse()?;
    /// modules.insert_source(path, "app.adm2", "sheet first { cell x: i32; } sheet second {}")?;
    /// let compiled = adam_lang::ModuleCompiler::new(adam_lang::TypeRegistry::new(),
    ///     cel_parser::OpLookup::new()).compile(modules).map_err(|mut e| e.remove(0))?;
    /// let path = "app".parse()?;
    /// assert_eq!(compiled.roots(&path).collect::<Vec<_>>(), ["first", "second"]);
    /// assert_eq!(compiled.prepared_root(&path, "first").unwrap().cells()[0].name, "x");
    /// # Ok::<(), adam_lang::ModuleDiagnostic>(())
    /// ```
    pub fn prepared_root(&self, module: &ModulePath, name: &str) -> Option<&PreparedRoot> {
        self.roots
            .get(module)?
            .iter()
            .find(|(root, _)| root == name)
            .map(|(_, root)| root)
    }
}

/// Owns only the syntax and planned descriptors read by one repeatable emitter.
struct Expression {
    syntax: Expr,
    inputs: Vec<(String, ValueType)>,
    candidate: Option<ValueType>,
    descriptor: ValueType,
    label: SourceLabel,
    environment: Rc<Environment>,
}

impl Expression {
    /// Copies immutable checked syntax/bindings without retaining the module collection.
    ///
    /// - Complexity: O(e + i + t) AST nodes, input names and descriptor nodes.
    fn own(
        expression: &analysis::Expression<'_>,
        plan: &DefinitionPlan<'_>,
        environment: &Rc<Environment>,
    ) -> Self {
        Self {
            syntax: expression.syntax.clone(),
            inputs: expression
                .inputs
                .iter()
                .map(|id| {
                    let cell = &plan.cells[*id];
                    (
                        cell.name.to_owned(),
                        environment.types.value_type(&cell.shape),
                    )
                })
                .collect(),
            candidate: expression
                .candidate
                .map(|id| environment.types.value_type(&plan.cells[id].shape)),
            descriptor: expression.descriptor.clone(),
            label: expression.label.clone(),
            environment: environment.clone(),
        }
    }

    /// Emits fresh executable resources using only the already planned positional reads.
    ///
    /// # Errors
    /// Propagates operation-emission errors, without executing the resulting segment.
    /// - Complexity: CEL emission plus O(nr) positional name comparisons.
    fn emit(&self) -> anyhow::Result<DynSegment> {
        analysis::Emission {
            syntax: &self.syntax,
            inputs: self
                .inputs
                .iter()
                .map(|(name, descriptor)| (name.as_str(), descriptor.clone())),
            candidate: self.candidate.clone(),
            descriptor: &self.descriptor,
            label: &self.label,
        }
        .emit(&self.environment.types, &self.environment.lookup)
        .map_err(|error| anyhow::anyhow!("{error}"))
    }

    /// Emits and evaluates a checked literal once during native construction.
    ///
    /// # Errors
    /// Propagates literal-operation/frame errors without losing their original cause.
    /// - Complexity: Literal emission, frame preparation and execution costs.
    fn key(&self) -> anyhow::Result<CellValue> {
        let mut frame =
            RawFrame::new(self.emit()?, &[], &self.descriptor, &self.environment.types)?;
        frame.call(&[]).map(CellValue)
    }
}

/// Compiles with a shared immutable entry, including future AdamParser root lowering.
///
/// # Errors
/// Returns collection-wide checked or owning-preparation diagnostics.
/// - Complexity: As [`ModuleCompiler::compile`].
pub(crate) fn compile(
    environment: Rc<Environment>,
    modules: ModuleSet,
) -> Result<CompiledModules, Vec<ModuleDiagnostic>> {
    let checked = analysis::analyze(&modules, &environment.types, &environment.lookup)?;
    debug_assert!(std::ptr::eq(checked.types, environment.types.as_ref()));
    debug_assert!(std::ptr::eq(checked.lookup, &environment.lookup));
    let mut completed: Vec<Option<(TypedTemplate, Vec<RootCell>)>> =
        (0..checked.definitions.len()).map(|_| None).collect();
    // The resolver already supplies dependency postorder; no second sort or expansion runs.
    for &id in &checked.resolved.order {
        let plan = &checked.definitions[id];
        let result = if let Some(native) = plan.native {
            Ok((native.clone(), Vec::new()))
        } else {
            materialize(plan, &checked.definitions, &environment, &completed)
        };
        completed[id] = Some(result.map_err(|mut error| {
            error.labels.push(plan.label.clone());
            vec![error]
        })?);
    }
    let mut result = CompiledModules {
        templates: HashMap::new(),
        roots: HashMap::new(),
        sources: modules.sources.sources().to_vec(),
        environment: environment.clone(),
    };
    let mut seen_sources = result.sources.iter().cloned().collect();
    let mut seen_definitions = HashSet::new();
    for (id, &(module, local)) in checked.resolved.definitions.iter().enumerate() {
        let entry = &modules.entries[module];
        let name = &entry.definitions[local].name;
        let (template, cells) = completed[id]
            .take()
            .expect("dependency order covers all definitions");
        template.retain_sources(
            &mut seen_definitions,
            &mut seen_sources,
            &mut result.sources,
        );
        if checked.definitions[id].component {
            result
                .templates
                .entry(entry.path.clone())
                .or_default()
                .insert(name.clone(), template);
        } else {
            result
                .roots
                .entry(entry.path.clone())
                .or_default()
                .push((name.clone(), PreparedRoot { template, cells }));
        }
    }
    Ok(result)
}

/// Maps checked definition-local ordinals to native slots without allocating graph cells.
///
/// - Precondition: Every ordinal names an already declared local slot.
/// - Complexity: O(i) handles.
fn slots(ordinals: &[usize], cells: &[CellSlot]) -> Vec<CellSlot> {
    debug_assert!(ordinals.iter().all(|id| *id < cells.len()));
    ordinals.iter().map(|id| cells[*id].clone()).collect()
}

/// Constructs a method using the checked explicit output-pattern flag.
///
/// # Errors
/// Propagates native recipe preparation errors.
/// - Complexity: Expression ownership/emission plus native method preparation.
fn method(
    builder: &CompositionBuilder,
    plan: &DefinitionPlan<'_>,
    environment: &Rc<Environment>,
    cells: &[CellSlot],
    outputs: &[usize],
    destructure: bool,
    expression: &analysis::Expression<'_>,
) -> Result<MethodRecipe, ModuleDiagnostic> {
    let inputs = slots(&expression.inputs, cells);
    let outputs = slots(outputs, cells);
    let owned = Expression::own(expression, plan, environment);
    builder.method_pattern(&inputs, &outputs, destructure, move || owned.emit())
}

/// Lowers every checked body form in authored order through the native recipe adapters.
///
/// Roots use this identical lowering and retain only their own public-name assembly inputs.
/// # Errors
/// Propagates recipe preparation errors with owning definition/item labels.
/// - Complexity: O(n) local handles/metadata plus retained emission and native validation.
fn materialize(
    plan: &DefinitionPlan<'_>,
    definitions: &[DefinitionPlan<'_>],
    environment: &Rc<Environment>,
    completed: &[Option<(TypedTemplate, Vec<RootCell>)>],
) -> Result<(TypedTemplate, Vec<RootCell>), ModuleDiagnostic> {
    let mut builder = CompositionBuilder::with_types(environment.types.clone());
    builder.set_label(plan.label.clone());
    let mut cells = Vec::with_capacity(plan.cells.len());
    let mut authored = Vec::with_capacity(plan.formals + plan.items.len());
    for cell in &plan.cells[..plan.formals] {
        let slot = builder.parameter(cell.name, cell.shape.clone())?;
        authored.push(AuthoredItem {
            role: AuthoredRole::Parameter,
            parent: None,
            label: cell.label.clone(),
            cell: Some(slot.native().clone()),
            relationship: None,
            method_index: None,
            requirement_name: None,
            expression: None,
        });
        cells.push(slot);
    }
    let mut groups: Vec<Option<RelationshipSlot>> = vec![None; plan.items.len()];
    let mut pending_conditional = None;
    // Native declarations and groups are created once, at their authored positions.
    for (ordinal, item) in plan.items.iter().enumerate() {
        let mut metadata = AuthoredItem {
            role: AuthoredRole::Cell,
            parent: item.parent.map(|p| p + plan.formals),
            label: item.label.clone(),
            cell: None,
            relationship: None,
            method_index: None,
            requirement_name: None,
            expression: None,
        };
        let lower = (|| -> Result<(), ModuleDiagnostic> {
            match &item.body {
                Body::Cell { cell, initializer } => {
                    debug_assert_eq!(*cell, cells.len());
                    let declaration = &plan.cells[*cell];
                    let source = declaration.kind == Kind::Source;
                    metadata.role = if source {
                        AuthoredRole::Source
                    } else {
                        AuthoredRole::Cell
                    };
                    metadata.expression = initializer
                        .as_ref()
                        .map(|expression| expression.label.clone());
                    let slot = if let Some(expression) = initializer {
                        let owned = Expression::own(expression, plan, environment);
                        if source {
                            builder.source(
                                declaration.name,
                                declaration.shape.clone(),
                                move || owned.emit(),
                            )?
                        } else {
                            builder.cell(
                                declaration.name,
                                declaration.shape.clone(),
                                move || owned.emit(),
                            )?
                        }
                    } else {
                        builder.default_private(
                            declaration.name,
                            declaration.shape.clone(),
                            source,
                        )?
                    };
                    metadata.cell = Some(slot.native().clone());
                    metadata.label = declaration.label.clone();
                    cells.push(slot);
                }
                Body::Out { cell, writer } => {
                    metadata.role = AuthoredRole::Out;
                    metadata.expression = Some(writer.label.clone());
                    debug_assert_eq!(*cell, cells.len());
                    let declaration = &plan.cells[*cell];
                    let slot = builder.default_private(
                        declaration.name,
                        declaration.shape.clone(),
                        false,
                    )?;
                    metadata.cell = Some(slot.native().clone());
                    metadata.label = declaration.label.clone();
                    cells.push(slot);
                    let recipe =
                        method(&builder, plan, environment, &cells, &[*cell], false, writer)?;
                    metadata.relationship = Some(builder.out(recipe)?);
                    metadata.method_index = Some(0);
                }
                Body::Filter {
                    cell,
                    expression,
                    range,
                } => {
                    metadata.role = if *range {
                        AuthoredRole::RangeFilter
                    } else {
                        AuthoredRole::Filter
                    };
                    metadata.expression = Some(expression.label.clone());
                    let inputs = slots(&expression.inputs, &cells);
                    let owned = Expression::own(expression, plan, environment);
                    builder.filter(&cells[*cell], &inputs, move || owned.emit())?;
                    metadata.cell = Some(cells[*cell].native().clone());
                }
                Body::Requirement {
                    cell,
                    name,
                    expression,
                } => {
                    metadata.role = AuthoredRole::Requirement;
                    metadata.expression = Some(expression.label.clone());
                    let inputs = slots(&expression.inputs, &cells);
                    let owned = Expression::own(expression, plan, environment);
                    builder.require(&cells[*cell], &inputs, move || owned.emit())?;
                    metadata.cell = Some(cells[*cell].native().clone());
                    metadata.requirement_name = name.map(str::to_owned);
                }
                Body::Relationship { methods } => {
                    metadata.role = AuthoredRole::Relationship;
                    let recipes = methods
                        .clone()
                        .map(|id| match &plan.items[id].body {
                            Body::Method {
                                outputs,
                                destructure,
                                expression,
                            } => method(
                                &builder,
                                plan,
                                environment,
                                &cells,
                                outputs,
                                *destructure,
                                expression,
                            ),
                            _ => unreachable!("checked relationship contains only methods"),
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let group = builder.relationship(recipes)?;
                    metadata.relationship = Some(group.clone());
                    groups[ordinal] = Some(group);
                }
                Body::Method { expression, .. } => {
                    metadata.role = AuthoredRole::Method;
                    metadata.expression = Some(expression.label.clone());
                    let parent = item.parent.expect("method has relationship parent");
                    let Body::Relationship { methods } = &plan.items[parent].body else {
                        unreachable!()
                    };
                    metadata.relationship = groups[parent].clone();
                    metadata.method_index = Some(ordinal - methods.start);
                }
                Body::Use {
                    target,
                    bindings,
                    syntax,
                } => {
                    metadata.role = AuthoredRole::Use;
                    if let Some(location) = &item.label.location {
                        metadata.label = label(&location.source, syntax.span, "component use");
                    }
                    let child = &completed[*target]
                        .as_ref()
                        .expect("dependency materialized")
                        .0;
                    let signature = &definitions[*target];
                    let actuals = signature.cells[..signature.formals]
                        .iter()
                        .zip(bindings)
                        .map(|(formal, id)| (formal.name, cells[*id].clone()))
                        .collect::<Vec<_>>();
                    let inclusion = builder.include(child, &actuals)?;
                    groups[ordinal] = Some(inclusion.relationships()[0].clone());
                    metadata.relationship = groups[ordinal].clone();
                }
                Body::Conditional { expression } => {
                    metadata.role = AuthoredRole::Conditional;
                    metadata.expression = Some(expression.label.clone());
                    pending_conditional = Some(ordinal);
                }
                Body::Branch { syntax } => {
                    metadata.role = AuthoredRole::Branch;
                    if let Some(syntax) = syntax
                        && let Some(location) = &item.label.location
                    {
                        metadata.expression =
                            Some(label(&location.source, syntax.literal_span, "branch key"));
                    }
                }
            }
            Ok(())
        })();
        lower.map_err(|mut error| {
            error.labels.push(item.label.clone());
            error
        })?;
        authored.push(metadata);
        // A conditional's group registration follows the last authored branch member.
        // Its members already occupy their correct native declaration positions.
        let next_conditional = plan
            .items
            .get(ordinal + 1)
            .is_none_or(|next| next.parent.is_none());
        if next_conditional && let Some(id) = pending_conditional.take() {
            lower_conditional(&mut builder, plan, id, environment, &cells, &groups)?;
        }
    }
    let locals = if plan.component {
        Vec::new()
    } else {
        plan.cells
            .iter()
            .zip(&cells)
            .map(|(cell, slot)| RootCell {
                name: cell.name.to_owned(),
                shape: cell.shape.clone(),
                slot: slot.native().clone(),
                label: cell.label.clone(),
                kind: match cell.kind {
                    Kind::Cell => CellKind::Cell,
                    Kind::Source => CellKind::Source,
                    Kind::Out => CellKind::Out,
                    Kind::Parameter => unreachable!("root has no formals"),
                },
            })
            .collect()
    };
    builder.set_authored(authored);
    Ok((builder.finish()?, locals))
}

/// Attaches a conditional's already lowered native groups and deferred literal keys.
///
/// # Errors
/// Propagates native conditional preparation or retained literal-emission errors.
/// - Complexity: O(b + n) branches and local body scan, plus expression ownership/emission.
fn lower_conditional(
    builder: &mut CompositionBuilder,
    plan: &DefinitionPlan<'_>,
    id: usize,
    environment: &Rc<Environment>,
    cells: &[CellSlot],
    groups: &[Option<RelationshipSlot>],
) -> Result<(), ModuleDiagnostic> {
    let Body::Conditional { expression } = &plan.items[id].body else {
        unreachable!()
    };
    let inputs = slots(&expression.inputs, cells);
    let owned = Expression::own(expression, plan, environment);
    let shape = expression.shape.clone();
    let mut keys = Vec::new();
    let mut branches = Vec::new();
    let mut default = Vec::new();
    let mut current_branch = None;
    for (item_id, item) in plan
        .items
        .iter()
        .enumerate()
        .skip(id + 1)
        .take_while(|(_, item)| item.parent.is_some())
    {
        if let Body::Branch { syntax } = &item.body {
            if let Some(syntax) = syntax {
                let literal = cel_parser::literal_expr(&syntax.literal)
                    .map_err(crate::composition::diagnostic)?;
                let syntax = if syntax.negated {
                    Expr::Op {
                        name: "-".into(),
                        operands: vec![literal],
                        span: syntax.literal_span,
                    }
                } else {
                    literal
                };
                keys.push(Expression {
                    syntax,
                    inputs: Vec::new(),
                    candidate: None,
                    descriptor: owned.descriptor.clone(),
                    label: item.label.clone(),
                    environment: environment.clone(),
                });
                branches.push(Vec::new());
                current_branch = Some(branches.len() - 1);
            } else {
                current_branch = None;
            }
        } else if let Some(group) = &groups[item_id] {
            if let Some(branch) = current_branch {
                branches[branch].push(group.clone());
            } else {
                default.push(group.clone());
            }
        }
    }
    builder.conditional_owned(
        &inputs,
        shape,
        move || owned.emit(),
        move || {
            keys.iter()
                .map(|key| key.key().map(|value| vec![value]))
                .collect()
        },
        branches,
        default,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the exact Task 6 recipe/slot handoff without implementing root assembly.
    #[test]
    fn prepared_roots_use_component_lowering_without_private_name_flattening() {
        let path: ModulePath = "roots".parse().unwrap();
        let mut modules = ModuleSet::new();
        modules
            .insert_source(
                path.clone(),
                "roots.adm2",
                "sheet hidden(a: i32, b: i32) { cell private: i32 = 4;
                relationship { b := a + private; } }
             sheet root { source input: i32 = 3; cell output: i32;
                relationship hidden(a: input, b: output);
                out answer := output; }",
            )
            .unwrap();
        let compiled = ModuleCompiler::new(TypeRegistry::new(), OpLookup::new())
            .compile(modules)
            .unwrap();
        let root = compiled.prepared_root(&path, "root").unwrap();
        let mut destination = crate::ParsedSheet::new();
        let instance = root.template.instantiate(&mut destination, &[]).unwrap();
        let root_id = instance.occurrences()[0].id();
        let output = instance.cell(root_id, &root.cells()[1].slot).unwrap();
        let answer = instance.cell(root_id, &root.cells()[2].slot).unwrap();
        assert_eq!(
            root.cells()
                .iter()
                .map(|cell| cell.name.as_str())
                .collect::<Vec<_>>(),
            ["input", "output", "answer"]
        );
        assert_eq!(root.cells()[2].kind, CellKind::Out);
        assert_eq!(destination.cell_kind(answer), Some(CellKind::Out));
        drop((instance, compiled));
        destination.propagate().unwrap();
        assert_eq!(*destination.read::<i32>(output).unwrap(), 7);
        assert_eq!(*destination.read::<i32>(answer).unwrap(), 7);
        assert!(destination.cell_names.is_empty());
    }
}
