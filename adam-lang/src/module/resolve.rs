//! Resolves exports before actual-use dependencies and checks those edges iteratively.

use super::*;

/// Stores a compact resolved graph indexed by global definition ordinal.
pub(super) struct Resolved<'a> {
    modules: &'a ModuleSet,
    pub definitions: Vec<(usize, usize)>,
    pub scopes: Vec<HashMap<String, usize>>,
    pub edges: Vec<Vec<Edge>>,
    pub order: Vec<usize>,
}

/// Retains one actual-use dependency and its authored invocation location.
pub(super) struct Edge {
    pub target: usize,
    pub label: SourceLabel,
}

/// Resolves every local/import namespace before traversing actual component uses.
///
/// # Errors
/// Reports duplicates, unknown exports, root uses and actual-use cycles.
/// - Complexity: O(n + p) time for file/body/branch declarations n and qualified-name
///   bytes p; O(d + i + u + p) space for definitions, imports, uses and names.
pub(super) fn resolve(modules: &ModuleSet) -> Result<Resolved<'_>, Vec<ModuleDiagnostic>> {
    let mut result = Resolved {
        modules,
        definitions: Vec::new(),
        scopes: vec![HashMap::new(); modules.entries.len()],
        edges: Vec::new(),
        order: Vec::new(),
    };
    let mut errors = Vec::new();
    let mut sites = vec![HashMap::<String, SourceLabel>::new(); modules.entries.len()];
    // Register all exports before looking up any import, including cyclic imports.
    for (module, entry) in modules.entries.iter().enumerate() {
        for (local, definition) in entry.definitions.iter().enumerate() {
            let id = result.definitions.len();
            result.definitions.push((module, local));
            if let Some(previous) = sites[module].get(&definition.name) {
                errors.push(ModuleDiagnostic {
                    message: format!("duplicate definition `{}`", definition.name),
                    labels: vec![definition.label().clone(), previous.clone()],
                });
            } else {
                sites[module].insert(definition.name.clone(), definition.label().clone());
                result.scopes[module].insert(definition.name.clone(), id);
            }
        }
    }
    let exports = result.scopes.clone();
    for (module, entry) in modules.entries.iter().enumerate() {
        for import in &entry.imports {
            let owner = entry.source.as_ref().expect("parsed import has a source");
            let site = label(owner, import.span, "import");
            let target = exported_path(modules, &exports, &import.path).map_err(|message| {
                ModuleDiagnostic {
                    message,
                    labels: vec![site.clone()],
                }
            });
            let name = import
                .alias
                .as_ref()
                .map(|(name, _)| name)
                .or_else(|| import.path.last().map(|(name, _)| name))
                .expect("parsed import is nonempty");
            let duplicate = if let Some(previous) = sites[module].get(name) {
                errors.push(ModuleDiagnostic {
                    message: format!("duplicate local or imported name `{name}`"),
                    labels: vec![site.clone(), previous.clone()],
                });
                true
            } else {
                sites[module].insert(name.clone(), site);
                false
            };
            match target {
                Ok(target) if !duplicate => {
                    result.scopes[module].insert(name.clone(), target);
                }
                Err(error) => errors.push(error),
                _ => {}
            }
        }
    }
    // Resolve every authored dependency, including bodies never selected by a root.
    for &(module, local) in &result.definitions {
        let definition = &modules.entries[module].definitions[local];
        let mut edges = Vec::new();
        if let DefinitionBody::Parsed { sheet, .. } = &definition.body {
            let owner = modules.entries[module]
                .source
                .as_ref()
                .expect("parsed body has source");
            for invocation in uses(sheet) {
                let site = label(owner, invocation.span, "component use");
                let target = if invocation.path.len() == 1 {
                    result.scopes[module]
                        .get(&invocation.path[0].0)
                        .copied()
                        .ok_or_else(|| {
                            format!(
                                "unknown definition `{}` in module `{}`",
                                invocation.path[0].0, modules.entries[module].path
                            )
                        })
                } else {
                    exported_path(modules, &exports, &invocation.path)
                };
                match target {
                    Ok(target) => {
                        if matches!(&definition_at(&result, target).body,
                            DefinitionBody::Parsed { sheet, .. } if sheet.parameters.is_none())
                        {
                            errors.push(ModuleDiagnostic {
                                message: "a root sheet is not a reusable component".into(),
                                labels: vec![site, definition_at(&result, target).label().clone()],
                            });
                        } else {
                            edges.push(Edge {
                                target,
                                label: site,
                            });
                        }
                    }
                    Err(message) => errors.push(ModuleDiagnostic {
                        message,
                        labels: vec![site],
                    }),
                }
            }
        }
        result.edges.push(edges);
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    result.order = postorder(&result.edges).map_err(|error| vec![error])?;
    Ok(result)
}

/// Borrows the authored definition for a resolved global ordinal.
fn definition_at<'a>(resolved: &Resolved<'a>, id: usize) -> &'a Definition {
    let (module, local) = resolved.definitions[id];
    &resolved.modules.entries[module].definitions[local]
}

/// Selects only a module's declared exports, never its imported aliases.
///
/// # Errors
/// Reports a missing module, missing export or incomplete qualified path.
/// - Complexity: O(p) qualified path bytes.
fn exported_path(
    modules: &ModuleSet,
    exports: &[HashMap<String, usize>],
    path: &[(String, ast::ExprSpan)],
) -> Result<usize, String> {
    if path.len() < 2 {
        return Err("an import requires a module and definition".into());
    }
    let module = ModulePath(
        path[..path.len() - 1]
            .iter()
            .map(|(name, _)| name.clone())
            .collect(),
    );
    let index = modules
        .indices
        .get(&module)
        .ok_or_else(|| format!("unknown module `{module}`"))?;
    let name = &path.last().expect("nonempty path").0;
    exports[*index]
        .get(name)
        .copied()
        .ok_or_else(|| format!("module `{module}` does not export `{name}`"))
}

/// Enumerates authored uses in exactly the order consumed by body lowering.
///
/// - Complexity: O(n) body and branch declarations, without expanding definitions.
fn uses(sheet: &ast::Sheet) -> Vec<&ast::RelationshipUse> {
    let mut result = Vec::new();
    for item in &sheet.items {
        match item {
            ast::SheetItem::RelationshipUse(invocation) => result.push(invocation),
            ast::SheetItem::Conditional(conditional) => {
                for branch in conditional
                    .branches
                    .iter()
                    .map(|branch| &branch.relationships)
                    .chain(
                        conditional
                            .default
                            .iter()
                            .map(|branch| &branch.relationships),
                    )
                {
                    for item in branch {
                        if let ast::BranchItem::RelationshipUse(invocation) = item {
                            result.push(invocation);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    result
}

/// Produces memoized dependency-first order with explicit-stack tri-color DFS.
///
/// # Errors
/// Returns the complete actual-use cycle's invocation chain.
/// - Complexity: O(d + u) time and space in definitions and use edges.
fn postorder(edges: &[Vec<Edge>]) -> Result<Vec<usize>, ModuleDiagnostic> {
    let mut colors = vec![0u8; edges.len()];
    let mut order = Vec::with_capacity(edges.len());
    let mut stack = Vec::new();
    for root in 0..edges.len() {
        if colors[root] != 0 {
            continue;
        }
        colors[root] = 1;
        stack.push((root, 0usize));
        while let Some(&(node, next)) = stack.last() {
            if next == edges[node].len() {
                colors[node] = 2;
                order.push(node);
                stack.pop();
                continue;
            }
            stack.last_mut().expect("nonempty stack").1 += 1;
            let edge = &edges[node][next];
            match colors[edge.target] {
                0 => {
                    colors[edge.target] = 1;
                    stack.push((edge.target, 0));
                }
                1 => {
                    let start = stack
                        .iter()
                        .position(|(id, _)| *id == edge.target)
                        .expect("gray node is active");
                    let labels = stack[start..stack.len() - 1]
                        .iter()
                        .map(|(id, next)| edges[*id][next - 1].label.clone())
                        .chain(std::iter::once(edge.label.clone()))
                        .collect();
                    return Err(ModuleDiagnostic {
                        message: "component-use cycle".into(),
                        labels,
                    });
                }
                _ => {}
            }
        }
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Constructs a source-free dependency with a distinguishable invocation label.
    fn edge(target: usize, name: &str) -> Edge {
        Edge {
            target,
            label: SourceLabel {
                location: None,
                label: name.into(),
            },
        }
    }

    /// Orders each unique dependency before all callers, including disconnected nodes.
    #[test]
    fn postorder_memoizes_shared_dependencies() {
        let graph = vec![
            vec![edge(1, "left"), edge(2, "right")],
            vec![edge(3, "left leaf")],
            vec![edge(3, "right leaf")],
            vec![],
            vec![],
        ];
        let order = postorder(&graph).unwrap();
        assert_eq!(order.len(), graph.len());
        let mut positions = vec![None; graph.len()];
        for (index, node) in order.into_iter().enumerate() {
            assert!(positions[node].replace(index).is_none());
        }
        for (caller, edges) in graph.iter().enumerate() {
            for edge in edges {
                assert!(positions[edge.target] < positions[caller]);
            }
        }
    }

    /// Excludes acyclic prefixes from a reported cycle chain.
    #[test]
    fn cycle_chain_contains_only_cycle_use_sites() {
        let graph = vec![
            vec![edge(1, "prefix")],
            vec![edge(2, "first")],
            vec![edge(1, "second")],
        ];
        let error = postorder(&graph).unwrap_err();
        assert_eq!(
            error
                .labels
                .iter()
                .map(|label| label.label.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert!(error.labels.iter().all(|label| label.location.is_none()));
    }

    /// Reports a self-use exactly once, including a one-node graph.
    #[test]
    fn self_use_has_one_invocation_label() {
        let error = postorder(&[vec![edge(0, "self")]]).unwrap_err();
        assert_eq!(error.labels.len(), 1);
        assert_eq!(error.labels[0].label, "self");
        assert!(postorder(&[]).unwrap().is_empty());
    }
}
