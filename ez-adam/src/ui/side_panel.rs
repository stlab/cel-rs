//! Context-sensitive side panel for property and formula editing. See
//! `docs/superpowers/specs/2026-08-26-ez-adam-ui-design.md` §5.

use crate::model::cell::{CellId, CellType};
use crate::model::conditional_group::{ConditionalBranch, ConditionalGroupId};
use crate::model::document::Document;
use crate::model::relationship_group::RelationshipGroupId;
use crate::ops::cells::{set_output, set_restrict};
use crate::ops::conditionals::toggle_enabled_group;
use crate::ops::relationships::set_member_formula;
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

/// Renders `conditional`'s enable-table: rows = branches, columns =
/// relationship groups referenced by the conditional's default or any branch,
/// checkboxes = whether each group is enabled on that branch.
#[component]
pub fn ConditionalPanel(
    mut document: Signal<Document>,
    conditional: ConditionalGroupId,
) -> Element {
    let (branches, all_groups) = {
        let doc = document.read();
        let cond = &doc.conditional_groups[conditional];
        let groups = referenced_groups(&cond.default, &cond.branches);
        (cond.branches.clone(), groups)
    };

    rsx! {
        table {
            class: "enable-table",
            thead {
                tr {
                    th { "" }
                    for group in &all_groups {
                        th { "{document.read().relationship_groups[*group].display_name}" }
                    }
                }
            }
            tbody {
                for (branch_index, branch) in branches.iter().enumerate() {
                    tr {
                        td { "{branch_index}" }
                        for group in &all_groups {
                            {
                                let group = *group;
                                let checked = branch.enabled_groups.contains(&group);
                                rsx! {
                                    td {
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
}
