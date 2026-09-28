//! Static dependency-graph checks backing `Sheet`'s guard-independence invariant.
//!
//! The static dependency graph has one node per cell and three kinds of edge:
//! every method's `input -> output` edges (a self-referencing input adds none), each
//! filter's `argument -> filtered` *guard* edges, and each conditional's
//! `match cell -> output` *guard* edges for every method output of its branch and
//! default relationships. The invariant is that no guard edge `g -> t` lies on a cycle,
//! including the single-node case `g == t`. Cycles made only of method edges are
//! ordinary multi-way relationships and are allowed.
//!
//! `Sheet::validate` builds this graph once, computes strongly connected components
//! once, and scans guard edges in deterministic order. It reconstructs a shortest
//! `t -> ... -> g` path only for the first violated guard edge.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::cell::CellId;
use crate::error::ErrorSite;
use crate::relationship::RelationshipId;

use super::Sheet;

/// One edge of a [`DependencyPath`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hop {
    /// A method edge through `relationship`.
    Method {
        /// The relationship traversed by this dependency edge.
        relationship: RelationshipId,
    },
    /// A filter or conditional guard edge.
    Guard,
}

/// A path in the static dependency graph.
///
/// - Invariant: `cells.len() == hops.len() + 1`; `hops[i]` leads from `cells[i]` to
///   `cells[i + 1]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DependencyPath {
    pub(super) cells: Vec<CellId>,
    pub(super) hops: Vec<Hop>,
}

impl DependencyPath {
    /// Returns the path as `Error::DependencyCycle` sites.
    ///
    /// Each cell appears as `Cell`, each method hop appears as `Relationship`, and each
    /// guard hop appears as no extra site, leaving two consecutive `Cell` entries.
    ///
    /// - Complexity: O(n) in the path length.
    pub(super) fn into_sites(self) -> Vec<ErrorSite> {
        let mut sites = Vec::with_capacity(self.cells.len() + self.hops.len());
        sites.push(ErrorSite::Cell(self.cells[0]));
        for (hop, &cell) in self.hops.iter().zip(&self.cells[1..]) {
            if let Hop::Method { relationship } = *hop {
                sites.push(ErrorSite::Relationship(relationship));
            }
            sites.push(ErrorSite::Cell(cell));
        }
        sites
    }
}

/// One static dependency edge.
#[derive(Debug, Clone, Copy)]
struct DependencyEdge {
    to: CellId,
    hop: Hop,
}

/// The static dependency graph and guard edges to check.
struct StaticDependencyGraph {
    nodes: Vec<CellId>,
    adjacency: HashMap<CellId, Vec<DependencyEdge>>,
    guard_edges: Vec<(CellId, CellId)>,
}

impl StaticDependencyGraph {
    /// Returns the outgoing dependency edges for `cell`.
    ///
    /// - Complexity: O(1).
    fn successors(&self, cell: CellId) -> &[DependencyEdge] {
        self.adjacency.get(&cell).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// One suspended depth-first-search frame for iterative Tarjan SCC.
struct TarjanFrame {
    node: CellId,
    next_successor: usize,
    entered: bool,
}

impl Sheet {
    /// Returns a violated guard's cycle, as a path from the governed cell `t` to the guard
    /// cell `g` whose guard edge `g -> t` closes it, or `None` if every filter and
    /// conditional is independent of the cells it governs.
    ///
    /// Filters are checked in cell order, then conditionals in insertion order; the first
    /// violation found is returned.
    ///
    /// - Complexity: O(V + E) when no guard violates the invariant, or O(V + E) plus one
    ///   shortest-path search within the violated component on the error path.
    pub(super) fn guard_violation(&self) -> Option<DependencyPath> {
        let graph = self.static_dependency_graph();
        let components = graph.strongly_connected_components();
        for &(guard, target) in &graph.guard_edges {
            if guard == target {
                return Some(DependencyPath {
                    cells: vec![target],
                    hops: Vec::new(),
                });
            }
            if components.get(&guard) == components.get(&target) {
                return graph.shortest_path_within_component(target, guard, &components);
            }
        }
        None
    }

    /// Returns the static dependency graph and guard-edge list for this sheet.
    ///
    /// - Complexity: O(V + E), where E is the number of static method and guard edges.
    fn static_dependency_graph(&self) -> StaticDependencyGraph {
        let mut adjacency: HashMap<CellId, Vec<DependencyEdge>> = HashMap::new();
        let mut guard_edges = Vec::new();

        for (relationship, data) in &self.relationships {
            for method in &data.methods {
                let outputs: HashSet<CellId> = method.outputs.iter().copied().collect();
                for &input in &method.inputs {
                    if outputs.contains(&input) {
                        continue;
                    }
                    for &output in &method.outputs {
                        adjacency.entry(input).or_default().push(DependencyEdge {
                            to: output,
                            hop: Hop::Method { relationship },
                        });
                    }
                }
            }
        }

        for (cell, data) in &self.cells {
            if let Some(filter) = &data.filter {
                for &arg in &filter.args {
                    adjacency.entry(arg).or_default().push(DependencyEdge {
                        to: cell,
                        hop: Hop::Guard,
                    });
                    guard_edges.push((arg, cell));
                }
            }
        }

        for conditional in self.conditionals.values() {
            let governed: Vec<CellId> = self.governed_outputs(conditional).collect();
            for &match_cell in conditional.match_cells() {
                for &output in &governed {
                    adjacency
                        .entry(match_cell)
                        .or_default()
                        .push(DependencyEdge {
                            to: output,
                            hop: Hop::Guard,
                        });
                    guard_edges.push((match_cell, output));
                }
            }
        }

        StaticDependencyGraph {
            nodes: self.cells.keys().collect(),
            adjacency,
            guard_edges,
        }
    }

    /// Returns every method output of `conditional`'s branch and default relationships,
    /// possibly with repeats.
    ///
    /// - Complexity: O(R * M * K) over the governed relationships' methods and outputs.
    fn governed_outputs<'a>(
        &'a self,
        conditional: &'a crate::conditional::ConditionalData,
    ) -> impl Iterator<Item = CellId> + 'a {
        conditional
            .branches
            .iter()
            .flat_map(|branch| branch.relationships.iter())
            .chain(conditional.default.iter())
            .flat_map(move |&rel| self.relationships[rel].methods.iter())
            .flat_map(|method| method.outputs.iter().copied())
    }
}

impl StaticDependencyGraph {
    /// Returns the strongly connected component id for each graph node.
    ///
    /// Uses iterative Tarjan's algorithm so large sheets do not consume call stack.
    ///
    /// - Complexity: O(V + E).
    fn strongly_connected_components(&self) -> HashMap<CellId, usize> {
        let mut next_index = 0usize;
        let mut next_component = 0usize;
        let mut indices: HashMap<CellId, usize> = HashMap::new();
        let mut lowlinks: HashMap<CellId, usize> = HashMap::new();
        let mut stack = Vec::new();
        let mut on_stack = HashSet::new();
        let mut components = HashMap::new();

        for &start in &self.nodes {
            if indices.contains_key(&start) {
                continue;
            }
            let mut frames = vec![TarjanFrame {
                node: start,
                next_successor: 0,
                entered: false,
            }];

            while !frames.is_empty() {
                let frame_index = frames.len() - 1;
                if !frames[frame_index].entered {
                    let node = frames[frame_index].node;
                    indices.insert(node, next_index);
                    lowlinks.insert(node, next_index);
                    next_index += 1;
                    stack.push(node);
                    on_stack.insert(node);
                    frames[frame_index].entered = true;
                }

                let node = frames[frame_index].node;
                let successors = self.successors(node);
                if frames[frame_index].next_successor < successors.len() {
                    let successor = successors[frames[frame_index].next_successor].to;
                    frames[frame_index].next_successor += 1;
                    if !indices.contains_key(&successor) {
                        frames.push(TarjanFrame {
                            node: successor,
                            next_successor: 0,
                            entered: false,
                        });
                    } else if on_stack.contains(&successor) {
                        let successor_index = indices[&successor];
                        let lowlink = lowlinks
                            .get_mut(&node)
                            .expect("entered nodes have a lowlink");
                        *lowlink = (*lowlink).min(successor_index);
                    }
                    continue;
                }

                let completed = frames.pop().expect("frame exists").node;
                if lowlinks[&completed] == indices[&completed] {
                    while let Some(member) = stack.pop() {
                        on_stack.remove(&member);
                        components.insert(member, next_component);
                        if member == completed {
                            break;
                        }
                    }
                    next_component += 1;
                }
                if let Some(parent) = frames.last() {
                    let child_lowlink = lowlinks[&completed];
                    let parent_lowlink = lowlinks
                        .get_mut(&parent.node)
                        .expect("entered parent nodes have a lowlink");
                    *parent_lowlink = (*parent_lowlink).min(child_lowlink);
                }
            }
        }

        components
    }

    /// Returns a shortest path from `start` to `target` inside their shared SCC.
    ///
    /// Returns `None` if no path exists, which indicates inconsistent component data.
    ///
    /// - Precondition: `components[start] == components[target]`.
    /// - Complexity: O(V + E) within the component.
    fn shortest_path_within_component(
        &self,
        start: CellId,
        target: CellId,
        components: &HashMap<CellId, usize>,
    ) -> Option<DependencyPath> {
        debug_assert_eq!(components.get(&start), components.get(&target));
        let component = components[&start];
        let mut parent: HashMap<CellId, Option<(CellId, Hop)>> = HashMap::new();
        let mut queue = VecDeque::from([start]);
        parent.insert(start, None);

        while let Some(cell) = queue.pop_front() {
            if cell == target {
                return Some(Self::rebuild_path(cell, &parent));
            }
            for edge in self.successors(cell) {
                if components.get(&edge.to) != Some(&component) {
                    continue;
                }
                if let std::collections::hash_map::Entry::Vacant(e) = parent.entry(edge.to) {
                    e.insert(Some((cell, edge.hop)));
                    queue.push_back(edge.to);
                }
            }
        }
        None
    }

    /// Returns the path ending at `end`, following `parent` links back to a root.
    ///
    /// - Precondition: `parent` links from `end` terminate at a `None` root.
    /// - Complexity: O(n) in the path length.
    fn rebuild_path(
        end: CellId,
        parent: &HashMap<CellId, Option<(CellId, Hop)>>,
    ) -> DependencyPath {
        let mut cells = vec![end];
        let mut hops = Vec::new();
        let mut cursor = end;
        while let Some(&Some((prev, hop))) = parent.get(&cursor) {
            cells.push(prev);
            hops.push(hop);
            cursor = prev;
        }
        cells.reverse();
        hops.reverse();
        DependencyPath { cells, hops }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Filter, MatchExpr, Method};

    #[test]
    fn guard_violation_is_none_for_a_sheet_without_guards() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
                Method::from_fn_1_1(b, a, |x: &i32| Ok(*x)),
            ])
            .unwrap();
        assert_eq!(sheet.guard_violation(), None);
    }

    #[test]
    fn guard_violation_is_none_for_an_upstream_filter_argument() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        sheet
            .add_filter(
                b,
                Filter::from_fn_1(a, |v: &i32, lo: &i32| Ok((*v).max(*lo))),
            )
            .unwrap();
        assert_eq!(sheet.guard_violation(), None);
    }

    #[test]
    fn into_sites_renders_method_hops_as_relationships() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let r = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let path = DependencyPath {
            cells: vec![a, b, c],
            hops: vec![Hop::Method { relationship: r }, Hop::Guard],
        };
        assert_eq!(
            path.into_sites(),
            vec![
                ErrorSite::Cell(a),
                ErrorSite::Relationship(r),
                ErrorSite::Cell(b),
                ErrorSite::Cell(c),
            ]
        );
    }

    #[test]
    fn guard_violation_is_none_for_a_conditional_whose_branch_only_reads_its_match_cell() {
        let mut sheet = Sheet::new();
        let mode = sheet.add_cell(0_i32);
        let out = sheet.add_cell(0_i32);
        let r = sheet
            .add_relationship(vec![Method::from_fn_1_1(mode, out, |x: &i32| Ok(*x))])
            .unwrap();
        sheet
            .add_conditional(MatchExpr::cell(mode), vec![(vec![0_i32], vec![r])], vec![])
            .unwrap();
        assert_eq!(sheet.guard_violation(), None);
    }
}
