//! Context-sensitive side panel for property and formula editing. See
//! `docs/superpowers/specs/2026-08-26-ez-adam-ui-design.md` §5.

use crate::model::cell::{CellId, CellType};
use crate::model::conditional_group::{ConditionExpr, ConditionalBranch, ConditionalGroupId};
use crate::model::document::Document;
use crate::model::relationship_group::RelationshipGroupId;
use crate::ops::cells::{set_output, set_restrict};
use crate::ops::conditionals::{set_condition_formula, toggle_enabled_group};
use crate::ops::relationships::set_member_formula;
use crate::ui::canvas::NodeId;
use crate::validation::validate_cel_expression;
use annotate_snippets::Renderer;
use dioxus::prelude::*;

/// Returns `(min_text, max_text)` for `ty`'s clamp bounds if it's numeric,
/// or `None` if `ty` is `Bool`/`Text` (no clamp fields to show). Each
/// bound renders as an empty string when unset, matching an editable
/// text-field's natural "nothing typed" state.
pub fn clamp_bounds_text(ty: &CellType) -> Option<(String, String)> {
    match ty {
        CellType::F64 { clamp } => Some((
            clamp.min.map(|v| v.to_string()).unwrap_or_default(),
            clamp.max.map(|v| v.to_string()).unwrap_or_default(),
        )),
        CellType::I64 { clamp } => Some((
            clamp.min.map(|v| v.to_string()).unwrap_or_default(),
            clamp.max.map(|v| v.to_string()).unwrap_or_default(),
        )),
        CellType::Bool | CellType::Text => None,
    }
}

/// Parses a restrict expression input field: empty string becomes `None`,
/// non-empty becomes `Some(text)`.
fn parse_restrict_input(text: String) -> Option<String> {
    if text.is_empty() { None } else { Some(text) }
}

/// Renders `cell`'s editable properties: name, type (read-only — changing
/// a cell's type is out of scope for this plan), output checkbox, clamp
/// min/max (numeric types only, via [`clamp_bounds_text`]), and restrict
/// expression text.
#[component]
pub fn CellPanel(mut document: Signal<Document>, cell: CellId) -> Element {
    let (name, ty, output, restrict) = {
        let doc = document.read();
        let c = &doc.cells[cell];
        (
            c.name.clone(),
            c.ty.clone(),
            c.output,
            c.restrict.clone().unwrap_or_default(),
        )
    };
    let bounds = clamp_bounds_text(&ty);

    rsx! {
        div {
            class: "cell-panel",
            div { "Name: {name}" }
            label {
                input {
                    r#type: "checkbox",
                    checked: output,
                    onchange: move |evt| set_output(&mut document.write(), cell, evt.checked()),
                }
                "Output"
            }
            if let Some((min, max)) = bounds {
                div {
                    "Clamp min: "
                    input { value: "{min}" }
                    " max: "
                    input { value: "{max}" }
                }
            }
            div {
                "Restrict: "
                input {
                    value: "{restrict}",
                    onchange: move |evt| {
                        let text = evt.value();
                        set_restrict(&mut document.write(), cell, parse_restrict_input(text));
                    },
                }
            }
        }
    }
}

/// Returns a rendered diagnostic for `text` if it's not valid CEL, or
/// `None` if it parses cleanly. The empty string is treated as "not yet
/// filled in" rather than an error worth surfacing, so it also returns
/// `None`.
///
/// The diagnostic is rendered as a caret-annotated, potentially multi-line
/// string via [`cel_parser::ParseError::format_rustc_style`] (the same
/// `annotate-snippets`-backed technique `adam-web-ui::labels::format_adam_error`
/// uses), with [`Renderer::plain`] so no ANSI escapes leak into the DOM.
pub fn formula_diagnostic(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    validate_cel_expression(text)
        .err()
        .map(|e| e.format_rustc_style(text, "formula", 1, &Renderer::plain()))
}

/// Returns the single node the side panel should show properties for, or
/// `None` if nothing (or more than one thing) is selected — the panel
/// only ever targets exactly one node at a time.
#[must_use]
pub fn panel_target(selection: &std::collections::HashSet<NodeId>) -> Option<NodeId> {
    let mut iter = selection.iter();
    let first = *iter.next()?;
    if iter.next().is_some() {
        return None;
    }
    Some(first)
}

/// Collects all relationship groups referenced by a conditional group's
/// default and branches, deduplicating so each group appears exactly once.
///
/// - Postcondition: result contains each group from `default` and from any
///   branch's `enabled_groups` exactly once, with no duplicates.
/// - Complexity: O(n*m) where n is the total number of groups across all
///   sources and m is the size of the combined default (due to `Vec::contains`
///   checks).
///
/// # Examples
///
/// A group appearing in both default and a branch deduplicates to one entry:
/// ```ignore
/// let default = vec![group_a];
/// let branch = ConditionalBranch { enabled_groups: vec![group_a, group_b], .. };
/// let result = referenced_groups(&default, &[branch]);
/// assert_eq!(result.len(), 2);
/// assert_eq!(result.iter().filter(|g| *g == &group_a).count(), 1);
/// ```
pub fn referenced_groups(
    default: &[RelationshipGroupId],
    branches: &[ConditionalBranch],
) -> Vec<RelationshipGroupId> {
    let mut groups: Vec<_> = default.to_vec();
    for branch in branches {
        for g in &branch.enabled_groups {
            if !groups.contains(g) {
                groups.push(*g);
            }
        }
    }
    groups
}

/// Returns `known` extended with any group from `referenced` not already
/// present in it, appended in `referenced`'s order.
///
/// Used to grow — but never shrink — [`ConditionalPanel`]'s displayed set
/// of enable-table columns: unchecking a group's last enabled checkbox
/// removes it from [`referenced_groups`]'s result (see
/// [`crate::ops::conditionals::toggle_enabled_group`]), and recomputing the
/// table's columns from only the *current* reference set on every render
/// would make that column — and every column after it — disappear and
/// shift left out from under the very click that just happened, so a
/// follow-up click can land on a different group's checkbox than the one
/// the user is looking at.
///
/// - Postcondition: every element of `known` appears in the result, in the
///   same relative order it had in `known`, followed by every element of
///   `referenced` not already present, in `referenced`'s order, with no
///   duplicates.
///
/// # Examples
///
/// ```ignore
/// let known = vec![group_a];
/// let referenced = vec![group_b];
/// assert_eq!(union_preserving_known_order(&known, &referenced), vec![group_a, group_b]);
/// ```
fn union_preserving_known_order(
    known: &[RelationshipGroupId],
    referenced: &[RelationshipGroupId],
) -> Vec<RelationshipGroupId> {
    let mut result = known.to_vec();
    for &group in referenced {
        if !result.contains(&group) {
            result.push(group);
        }
    }
    result
}

/// Renders `group`'s member formulas as an editable list, each validated
/// live via [`formula_diagnostic`].
#[component]
pub fn RelationshipPanel(mut document: Signal<Document>, group: RelationshipGroupId) -> Element {
    let members: Vec<_> = {
        let doc = document.read();
        doc.relationship_groups[group]
            .members
            .iter()
            .map(|(node, formula)| {
                let cell = doc.cells[doc.cell_nodes[*node].cell].name.clone();
                (*node, cell, formula.clone())
            })
            .collect()
    };

    rsx! {
        div {
            class: "relationship-panel",
            for (node, cell_name, formula) in members {
                div {
                    "{cell_name} := "
                    input {
                        value: "{formula}",
                        onchange: move |evt| set_member_formula(&mut document.write(), group, node, evt.value()),
                    }
                    if let Some(diagnostic) = formula_diagnostic(&formula) {
                        div { class: "diagnostic", "{diagnostic}" }
                    }
                }
            }
        }
    }
}

/// Returns `condition`'s formula expression text if it is `Formula`-mode, or
/// `None` for `Cells`-mode conditions, which have no formula to edit.
fn formula_expr_for_display(condition: &ConditionExpr) -> Option<&str> {
    match condition {
        ConditionExpr::Formula { expr, .. } => Some(expr.as_str()),
        ConditionExpr::Cells(_) => None,
    }
}

/// Renders `conditional`'s enable-table: rows = branches, columns =
/// relationship groups referenced by the conditional's default or any
/// branch (plus any group this panel has already shown as a column during
/// its current lifetime — see [`union_preserving_known_order`]),
/// checkboxes = whether each group is enabled on that branch.
///
/// - Precondition: the caller gives this component a distinct `key` per
///   `conditional` (e.g. in [`SidePanel`]) so switching to a different
///   conditional starts a fresh column set instead of carrying over the
///   previous conditional's columns.
#[component]
pub fn ConditionalPanel(
    mut document: Signal<Document>,
    conditional: ConditionalGroupId,
) -> Element {
    let mut known_groups = use_signal(Vec::<RelationshipGroupId>::new);

    use_effect(move || {
        let referenced = {
            let doc = document.read();
            let cond = &doc.conditional_groups[conditional];
            referenced_groups(&cond.default, &cond.branches)
        };
        let merged = union_preserving_known_order(&known_groups.peek(), &referenced);
        if merged != *known_groups.peek() {
            known_groups.set(merged);
        }
    });

    let (branches, formula_expr) = {
        let doc = document.read();
        let cond = &doc.conditional_groups[conditional];
        let formula_expr = formula_expr_for_display(&cond.condition).map(str::to_owned);
        (cond.branches.clone(), formula_expr)
    };
    let all_groups = known_groups.read().clone();

    rsx! {
        if let Some(expr) = formula_expr {
            div {
                class: "conditional-formula",
                "Condition: "
                input {
                    value: "{expr}",
                    onchange: move |evt| {
                        set_condition_formula(&mut document.write(), conditional, evt.value());
                    },
                }
                if let Some(diagnostic) = formula_diagnostic(&expr) {
                    div { class: "diagnostic", "{diagnostic}" }
                }
            }
        }
        table {
            class: "enable-table",
            thead {
                tr {
                    th { "" }
                    for group in &all_groups {
                        th {
                            key: "{group:?}",
                            "{document.read().relationship_groups[*group].display_name}"
                        }
                    }
                }
            }
            tbody {
                for (branch_index, branch) in branches.iter().enumerate() {
                    tr {
                        key: "{branch_index}",
                        td { "{branch_index}" }
                        for group in &all_groups {
                            {
                                let group = *group;
                                let checked = branch.enabled_groups.contains(&group);
                                rsx! {
                                    td {
                                        key: "{group:?}",
                                        input {
                                            r#type: "checkbox",
                                            checked: checked,
                                            onchange: move |_| toggle_enabled_group(&mut document.write(), conditional, branch_index, group),
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Renders whichever of [`CellPanel`]/[`RelationshipPanel`]/[`ConditionalPanel`]
/// matches the current single-node `selection`, or nothing if the
/// selection is empty or multi-node (per [`panel_target`]).
#[component]
pub fn SidePanel(
    document: Signal<Document>,
    selection: Signal<std::collections::HashSet<NodeId>>,
) -> Element {
    let target = panel_target(&selection.read());
    rsx! {
        div {
            class: "side-panel",
            match target {
                Some(NodeId::CellNode(node)) => {
                    let cell = document.read().cell_nodes[node].cell;
                    rsx! { CellPanel { document, cell } }
                }
                Some(NodeId::RelationshipGroup(group)) => rsx! { RelationshipPanel { document, group } },
                Some(NodeId::ConditionalGroup(conditional)) => rsx! {
                    ConditionalPanel {
                        key: "{conditional:?}",
                        document,
                        conditional,
                    }
                },
                None => rsx! { div { "No selection" } },
            }
        }
    }
}

#[cfg(test)]
mod formula_tests {
    use super::*;

    #[test]
    fn formula_diagnostic_is_none_for_valid_cel() {
        assert_eq!(formula_diagnostic("a + b"), None);
    }

    #[test]
    fn formula_diagnostic_is_none_for_empty_text() {
        assert_eq!(formula_diagnostic(""), None);
    }

    #[test]
    fn formula_diagnostic_is_some_for_invalid_cel() {
        let diagnostic = formula_diagnostic("a +").expect("invalid CEL should have a diagnostic");
        assert!(!diagnostic.is_empty());
    }

    #[test]
    fn formula_diagnostic_does_not_use_debug_formatting() {
        // The rendered diagnostic must be a real annotate-snippets caret
        // rendering, not `format!("{e:?}")` Debug output — which for
        // `ParseError` would read like `ParseError { message: ..., span: ... }`.
        let diagnostic = formula_diagnostic("a +").expect("invalid CEL should have a diagnostic");
        assert!(!diagnostic.contains("ParseError {"));
        assert!(!diagnostic.contains("span:"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::cell::ClampRange;

    #[test]
    fn clamp_bounds_text_is_none_for_bool_and_text() {
        assert_eq!(clamp_bounds_text(&CellType::Bool), None);
        assert_eq!(clamp_bounds_text(&CellType::Text), None);
    }

    #[test]
    fn clamp_bounds_text_shows_set_bounds() {
        let ty = CellType::I64 {
            clamp: ClampRange {
                min: Some(0),
                max: Some(100),
            },
        };
        assert_eq!(
            clamp_bounds_text(&ty),
            Some(("0".to_string(), "100".to_string()))
        );
    }

    #[test]
    fn clamp_bounds_text_shows_empty_string_for_unset_bounds() {
        let ty = CellType::F64 {
            clamp: ClampRange {
                min: None,
                max: None,
            },
        };
        assert_eq!(clamp_bounds_text(&ty), Some((String::new(), String::new())));
    }

    #[test]
    fn parse_restrict_input_converts_empty_to_none() {
        assert_eq!(parse_restrict_input(String::new()), None);
    }

    #[test]
    fn parse_restrict_input_converts_nonempty_to_some() {
        assert_eq!(
            parse_restrict_input("x > 0".to_string()),
            Some("x > 0".to_string())
        );
    }

    #[test]
    fn referenced_groups_with_empty_default_and_branches_returns_empty() {
        let result = referenced_groups(&[], &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn referenced_groups_includes_groups_from_default() {
        use slotmap::SlotMap;

        let mut groups: SlotMap<RelationshipGroupId, ()> = SlotMap::with_key();
        let group_a = groups.insert(());
        let group_b = groups.insert(());

        let default = vec![group_a, group_b];
        let result = referenced_groups(&default, &[]);

        assert_eq!(result.len(), 2);
        assert!(result.contains(&group_a));
        assert!(result.contains(&group_b));
    }

    #[test]
    fn referenced_groups_includes_groups_from_branches() {
        use slotmap::SlotMap;

        let mut groups: SlotMap<RelationshipGroupId, ()> = SlotMap::with_key();
        let group_a = groups.insert(());
        let group_b = groups.insert(());

        let branch = ConditionalBranch {
            values: vec![],
            enabled_groups: vec![group_a, group_b],
        };
        let result = referenced_groups(&[], &[branch]);

        assert_eq!(result.len(), 2);
        assert!(result.contains(&group_a));
        assert!(result.contains(&group_b));
    }

    #[test]
    fn referenced_groups_deduplicates_groups_in_default_and_branches() {
        use slotmap::SlotMap;

        let mut groups: SlotMap<RelationshipGroupId, ()> = SlotMap::with_key();
        let group_a = groups.insert(());
        let group_b = groups.insert(());
        let group_c = groups.insert(());

        let default = vec![group_a];
        let branch = ConditionalBranch {
            values: vec![],
            enabled_groups: vec![group_a, group_b, group_c],
        };
        let result = referenced_groups(&default, &[branch]);

        // Should have group_a once (not twice), plus group_b and group_c
        assert_eq!(result.len(), 3);
        assert_eq!(result.iter().filter(|g| *g == &group_a).count(), 1);
        assert!(result.contains(&group_b));
        assert!(result.contains(&group_c));
    }

    #[test]
    fn referenced_groups_deduplicates_groups_across_multiple_branches() {
        use slotmap::SlotMap;

        let mut groups: SlotMap<RelationshipGroupId, ()> = SlotMap::with_key();
        let group_a = groups.insert(());
        let group_b = groups.insert(());

        let branch1 = ConditionalBranch {
            values: vec![],
            enabled_groups: vec![group_a],
        };
        let branch2 = ConditionalBranch {
            values: vec![],
            enabled_groups: vec![group_b, group_a],
        };
        let result = referenced_groups(&[], &[branch1, branch2]);

        // Should have group_a once and group_b once
        assert_eq!(result.len(), 2);
        assert_eq!(result.iter().filter(|g| *g == &group_a).count(), 1);
        assert_eq!(result.iter().filter(|g| *g == &group_b).count(), 1);
    }

    #[test]
    fn union_preserving_known_order_with_nothing_known_returns_referenced_in_order() {
        use slotmap::SlotMap;

        let mut groups: SlotMap<RelationshipGroupId, ()> = SlotMap::with_key();
        let group_a = groups.insert(());
        let group_b = groups.insert(());

        let result = union_preserving_known_order(&[], &[group_a, group_b]);
        assert_eq!(result, vec![group_a, group_b]);
    }

    #[test]
    fn union_preserving_known_order_keeps_a_known_group_that_is_no_longer_referenced() {
        use slotmap::SlotMap;

        let mut groups: SlotMap<RelationshipGroupId, ()> = SlotMap::with_key();
        let group_a = groups.insert(());

        // group_a was shown before but is no longer referenced at all
        // (e.g. its last enabled checkbox was just unchecked) — it must
        // stay in the result so the column doesn't vanish underneath the
        // user.
        let result = union_preserving_known_order(&[group_a], &[]);
        assert_eq!(result, vec![group_a]);
    }

    #[test]
    fn union_preserving_known_order_appends_newly_referenced_groups_after_known_ones() {
        use slotmap::SlotMap;

        let mut groups: SlotMap<RelationshipGroupId, ()> = SlotMap::with_key();
        let group_a = groups.insert(());
        let group_b = groups.insert(());

        let result = union_preserving_known_order(&[group_a], &[group_a, group_b]);
        assert_eq!(result, vec![group_a, group_b]);
    }

    #[test]
    fn union_preserving_known_order_does_not_duplicate_an_already_known_group() {
        use slotmap::SlotMap;

        let mut groups: SlotMap<RelationshipGroupId, ()> = SlotMap::with_key();
        let group_a = groups.insert(());

        let result = union_preserving_known_order(&[group_a], &[group_a]);
        assert_eq!(result, vec![group_a]);
    }

    #[test]
    fn formula_expr_for_display_returns_the_expr_for_formula_mode() {
        let condition = ConditionExpr::Formula {
            referenced_cells: vec![],
            expr: "x > 1.0".to_string(),
        };
        assert_eq!(formula_expr_for_display(&condition), Some("x > 1.0"));
    }

    #[test]
    fn formula_expr_for_display_is_none_for_cells_mode() {
        let condition = ConditionExpr::Cells(vec![]);
        assert_eq!(formula_expr_for_display(&condition), None);
    }

    #[test]
    fn panel_target_is_none_for_empty_selection() {
        let selection = std::collections::HashSet::new();
        assert_eq!(panel_target(&selection), None);
    }

    #[test]
    fn panel_target_is_none_for_multiple_selections() {
        let mut selection = std::collections::HashSet::new();
        // Two arbitrary distinct NodeIds — construct via slotmap fixtures
        // matching this file's existing test conventions.
        let mut cells: slotmap::SlotMap<crate::model::cell_node::CellNodeId, ()> =
            slotmap::SlotMap::with_key();
        selection.insert(NodeId::CellNode(cells.insert(())));
        selection.insert(NodeId::CellNode(cells.insert(())));
        assert_eq!(panel_target(&selection), None);
    }

    #[test]
    fn panel_target_returns_the_single_selected_node() {
        let mut cells: slotmap::SlotMap<crate::model::cell_node::CellNodeId, ()> =
            slotmap::SlotMap::with_key();
        let node = cells.insert(());
        let mut selection = std::collections::HashSet::new();
        selection.insert(NodeId::CellNode(node));
        assert_eq!(panel_target(&selection), Some(NodeId::CellNode(node)));
    }
}
