//! Static dependency-graph checks backing `Sheet`'s guard-independence invariant.
//!
//! The static dependency graph has one node per cell and three kinds of edge:
//! every method's `input → output` edges (a self-referencing input adds none), each
//! filter's `argument → filtered` *guard* edges, and each conditional's
//! `match cell → output` *guard* edges for every method output of its branch and
//! default relationships. The invariant is that no guard edge `g → t` lies on a cycle,
//! i.e. `t` never reaches `g`. Cycles made only of method edges are ordinary multi-way
//! relationships and are allowed.
//!
//! `Sheet::add_filter`, `Sheet::add_conditional` and `Sheet::add_relationship` apply
//! their mutation, call [`Sheet::guard_violation`], and roll back on `Some`.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::cell::CellId;
use crate::error::ErrorSite;
use crate::relationship::RelationshipId;

use super::Sheet;

/// One edge of a [`DependencyPath`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hop {
    /// A method edge through `relationship`'s method at index `method`.
    Method {
        /// The relationship traversed by this dependency edge.
        relationship: RelationshipId,
        /// The method index traversed within `relationship`.
        method: usize,
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
    /// Returns the path as `Error::DependencyCycle` sites: each cell as `Cell`, each
    /// method hop as `Relationship`, or as `MethodIndex` when its relationship is
    /// `pending` (still being added), and each guard hop as nothing.
    ///
    /// - Complexity: O(n) in the path length.
    pub(super) fn into_sites(self, pending: Option<RelationshipId>) -> Vec<ErrorSite> {
        let mut sites = Vec::with_capacity(self.cells.len() + self.hops.len());
        sites.push(ErrorSite::Cell(self.cells[0]));
        for (hop, &cell) in self.hops.iter().zip(&self.cells[1..]) {
            if let Hop::Method {
                relationship,
                method,
            } = *hop
            {
                sites.push(if Some(relationship) == pending {
                    ErrorSite::MethodIndex(method)
                } else {
                    ErrorSite::Relationship(relationship)
                });
            }
            sites.push(ErrorSite::Cell(cell));
        }
        sites
    }
}

impl Sheet {
    /// Returns a violated guard's cycle, as a path from the governed cell `t` to the guard
    /// cell `g` whose guard edge `g → t` closes it, or `None` if every filter and
    /// conditional is independent of the cells it governs.
    ///
    /// Filters are checked in cell order, then conditionals in insertion order; the first
    /// violation found is returned.
    ///
    /// - Complexity: O(G · (V + E)) where G = filters + conditionals, V = cells, E =
    ///   method, filter and conditional edges.
    pub(super) fn guard_violation(&self) -> Option<DependencyPath> {
        let gates = self.conditional_gates();
        for (cell, data) in &self.cells {
            if let Some(filter) = &data.filter {
                let args: HashSet<CellId> = filter.args.iter().copied().collect();
                if let Some(path) = self.dependency_path(&[cell], &args, &gates) {
                    return Some(path);
                }
            }
        }
        for conditional in self.conditionals.values() {
            let governed: Vec<CellId> = self.governed_outputs(conditional).collect();
            let matches: HashSet<CellId> = conditional.match_cells().iter().copied().collect();
            if let Some(path) = self.dependency_path(&governed, &matches, &gates) {
                return Some(path);
            }
        }
        None
    }

    /// Returns every method output of `conditional`'s branch and default relationships,
    /// possibly with repeats.
    ///
    /// - Complexity: O(R · M · K) over the governed relationships' methods and outputs.
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

    /// Returns each match cell's conditional guard targets: `gates[m]` lists every method
    /// output governed by a conditional that has `m` as a match cell.
    ///
    /// - Complexity: O(total governed outputs · match cells per conditional).
    fn conditional_gates(&self) -> HashMap<CellId, Vec<CellId>> {
        let mut gates: HashMap<CellId, Vec<CellId>> = HashMap::new();
        for conditional in self.conditionals.values() {
            for &m in conditional.match_cells() {
                gates
                    .entry(m)
                    .or_default()
                    .extend(self.governed_outputs(conditional));
            }
        }
        gates
    }

    /// Returns a shortest path in the static dependency graph from any cell in `from` to
    /// any cell in `to`, or `None` if no cell of `to` is reachable. A cell in both sets
    /// yields a single-cell path.
    ///
    /// `gates` is [`Sheet::conditional_gates`]'s result for the current sheet.
    ///
    /// - Complexity: O(V + E).
    fn dependency_path(
        &self,
        from: &[CellId],
        to: &HashSet<CellId>,
        gates: &HashMap<CellId, Vec<CellId>>,
    ) -> Option<DependencyPath> {
        let mut parent: HashMap<CellId, Option<(CellId, Hop)>> = HashMap::new();
        let mut queue: VecDeque<CellId> = VecDeque::new();
        for &cell in from {
            if parent.insert(cell, None).is_none() {
                queue.push_back(cell);
            }
        }
        while let Some(cell) = queue.pop_front() {
            if to.contains(&cell) {
                return Some(Self::rebuild_path(cell, &parent));
            }
            let mut visit = |next: CellId, hop: Hop| {
                if let std::collections::hash_map::Entry::Vacant(e) = parent.entry(next) {
                    e.insert(Some((cell, hop)));
                    queue.push_back(next);
                }
            };
            for &relationship in &self.cells[cell].adj {
                for (method, data) in self.relationships[relationship].methods.iter().enumerate() {
                    if data.inputs.contains(&cell) {
                        for &output in data.outputs.iter().filter(|&&o| o != cell) {
                            visit(
                                output,
                                Hop::Method {
                                    relationship,
                                    method,
                                },
                            );
                        }
                    }
                }
            }
            for &filtered in self.filter_dependents.get(&cell).into_iter().flatten() {
                visit(filtered, Hop::Guard);
            }
            for &governed in gates.get(&cell).into_iter().flatten() {
                visit(governed, Hop::Guard);
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
    fn into_sites_renders_method_hops_as_relationships_or_pending_method_indices() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let r = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let path = DependencyPath {
            cells: vec![a, b, c],
            hops: vec![
                Hop::Method {
                    relationship: r,
                    method: 0,
                },
                Hop::Guard,
            ],
        };
        assert_eq!(
            path.clone().into_sites(None),
            vec![
                ErrorSite::Cell(a),
                ErrorSite::Relationship(r),
                ErrorSite::Cell(b),
                ErrorSite::Cell(c),
            ]
        );
        assert_eq!(
            path.into_sites(Some(r)),
            vec![
                ErrorSite::Cell(a),
                ErrorSite::MethodIndex(0),
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
