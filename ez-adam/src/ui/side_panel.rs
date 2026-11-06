//! Context-sensitive side panel for property and formula editing. See
//! `docs/superpowers/specs/2026-08-26-ez-adam-ui-design.md` §5.

use crate::model::cell::{CellId, CellType};
use crate::model::conditional_group::{ConditionExpr, ConditionalBranch, ConditionalGroupId};
use crate::model::document::Document;
use crate::model::relationship_group::RelationshipGroupId;
use crate::ops::cells::{set_clamp_max, set_clamp_min, set_name, set_output, set_restrict};
use crate::ops::conditionals::{
    set_condition_formula, set_display_name as set_conditional_display_name, toggle_enabled_group,
};
use crate::ops::relationships::{
    set_display_name as set_relationship_display_name, set_member_formula,
};
use crate::ui::canvas::NodeId;
use crate::validation::validate_cel_expression;
use adam_web_ui::components::{Checkbox, CheckboxState, Input, Label, NumericEditor, Separator};
use adam_web_ui::input::{InputError, NumberKind};
use annotate_snippets::Renderer;
use dioxus::prelude::*;

/// Returns `(min_text, max_text)` for `ty`'s clamp bounds if it's numeric,
/// or `None` if `ty` is `Bool`/`Text` (no clamp fields to show). Each
/// bound renders as an empty string when unset, matching an editable
/// text-field's natural "nothing typed" state.
///
/// - Complexity: O(formatted bound bytes) time and space.
///
/// # Examples
///
/// `clamp_bounds_text(&CellType::Bool)` returns `None`; an unbounded numeric type
/// returns two empty bound drafts.
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

/// Identifies the numeric bound edited by a clamp control.
#[derive(Clone, Copy)]
enum ClampBound {
    /// Selects the inclusive lower bound.
    Min,
    /// Selects the inclusive upper bound.
    Max,
}

/// Describes a value event or blur from an optional numeric editor.
enum NumericBoundEvent<'a> {
    /// Carries the editor's canonical value, or `None` when the field is cleared.
    Value(Option<&'a str>),
    /// Synchronizes the draft after focus leaves without changing the model.
    Blur,
}

/// Applies a numeric editor event to the selected document bound.
///
/// - Postcondition: a value event updates or clears the selected bound; blur leaves the
///   document unchanged.
fn apply_numeric_bound_event(
    panel_epoch: u64,
    document_epoch: Signal<u64>,
    document: Signal<Document>,
    cell: crate::model::cell::CellId,
    bound: ClampBound,
    event: NumericBoundEvent<'_>,
) -> bool {
    match event {
        NumericBoundEvent::Value(value) => {
            apply_panel_edit(panel_epoch, document_epoch, document, |document| {
                let text = value.unwrap_or("");
                match bound {
                    ClampBound::Min => set_clamp_min(document, cell, text),
                    ClampBound::Max => set_clamp_max(document, cell, text),
                }
            })
        }
        NumericBoundEvent::Blur => false,
    }
}

/// Returns a user-facing message for a numeric editor error.
///
/// # Examples
///
/// `numeric_error_message(&InputError::Lexical("bad input".into()))`
/// returns `"Enter a complete number."`.
pub(super) fn numeric_error_message(error: &InputError) -> &str {
    match error {
        InputError::Lexical(_) => "Enter a complete number.",
        InputError::TypeRange { .. } => "The number is outside the supported range.",
        InputError::UnsupportedEditor(message)
        | InputError::Locale(message)
        | InputError::EventSource(message) => message,
    }
}

/// Returns whether a panel callback belongs to the active document generation.
fn is_current_panel_epoch(expected_epoch: u64, document_epoch: Signal<u64>) -> bool {
    *document_epoch.peek() == expected_epoch
}

/// Converts a document boolean to the shared checkbox's initial state.
///
/// # Examples
///
/// `checkbox_state(true)` returns `CheckboxState::Checked`.
fn checkbox_state(checked: bool) -> CheckboxState {
    if checked {
        CheckboxState::Checked
    } else {
        CheckboxState::Unchecked
    }
}

/// Updates the controlled checkbox state when its model value changes.
///
/// - Postcondition: returns `true` if and only if `state` now matches `checked`.
fn sync_checkbox_state(mut state: Signal<Option<CheckboxState>>, checked: bool) -> bool {
    let next = Some(checkbox_state(checked));
    if *state.peek() == next {
        return false;
    }
    state.set(next);
    true
}

/// Renders a controlled checkbox and reports its next state to the owner.
///
/// - Complexity: O(1) time and space per state transition.
#[component]
fn PanelCheckbox(
    checked: bool,
    aria_label: String,
    on_checked_change: EventHandler<CheckboxState>,
) -> Element {
    let mut state = use_signal(|| Some(checkbox_state(checked)));
    use_effect(use_reactive!(|checked| {
        sync_checkbox_state(state, checked);
    }));

    rsx! {
        Checkbox {
            checked: state,
            aria_label,
            on_checked_change: move |next| {
                state.set(Some(next));
                on_checked_change.call(next);
            },
        }
    }
}

/// Applies a panel edit only while its originating document epoch remains active.
///
/// - Postcondition: `edit` runs exactly once if `expected_epoch` matches the current epoch.
/// - Complexity: O(1) time and space in addition to `edit`.
fn apply_panel_edit(
    expected_epoch: u64,
    document_epoch: Signal<u64>,
    mut document: Signal<Document>,
    edit: impl FnOnce(&mut Document),
) -> bool {
    if !is_current_panel_epoch(expected_epoch, document_epoch) {
        return false;
    }
    edit(&mut document.write());
    true
}

/// Replaces a numeric editor value only when its canonical model value changes.
///
/// - Postcondition: returns `true` if and only if `value` now equals `model`.
/// - Complexity: O(n) time and O(1) auxiliary space for n canonical text bytes.
fn sync_numeric_value(mut value: Signal<Option<String>>, model: Option<String>) -> bool {
    if *value.peek() == model {
        return false;
    }
    value.set(model);
    true
}

/// Renders `cell`'s editable properties: name, type (read-only — changing
/// a cell's type is out of scope for this plan), output checkbox, clamp
/// min/max (numeric types only, via [`clamp_bounds_text`]), and restrict
/// expression text.
///
/// - Complexity: O(t) per render for copied name/restriction/bound text bytes t.
#[component]
///
/// # Examples
///
/// `SidePanel { document, selection, document_epoch }` mounts this panel when
/// `selection` contains exactly one live `NodeId::CellNode`.
pub fn CellPanel(
    mut document: Signal<Document>,
    cell: CellId,
    document_epoch: Signal<u64>,
    panel_epoch: u64,
) -> Element {
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
    let is_integer = matches!(ty, CellType::I64 { .. });
    let number_kind = if is_integer {
        NumberKind::Signed(64)
    } else {
        NumberKind::F64
    };
    let step = if is_integer { "1" } else { "0.01" };
    let has_clamp_bounds = bounds.is_some();
    let (min, max) = bounds.unwrap_or_default();
    let model_min = (!min.is_empty()).then_some(min);
    let model_max = (!max.is_empty()).then_some(max);
    let mut min_value = use_signal(|| model_min.clone());
    let mut max_value = use_signal(|| model_max.clone());
    use_effect(use_reactive!(|model_min, model_max| {
        sync_numeric_value(min_value, model_min);
        sync_numeric_value(max_value, model_max);
    }));
    let range = use_signal(|| None);
    let mut min_invalid = use_signal(|| false);
    let mut max_invalid = use_signal(|| false);
    let mut min_error = use_signal(|| None::<String>);
    let mut max_error = use_signal(|| None::<String>);
    let name_id = format!("name-{cell:?}");
    let restrict_id = format!("restrict-{cell:?}");
    let clamp_min_id = format!("clamp-min-{cell:?}");
    let clamp_max_id = format!("clamp-max-{cell:?}");

    rsx! {
        div {
            class: "cell-panel",
            Label { html_for: name_id.clone(), "Name" }
            Input {
                id: name_id,
                r#type: "text",
                style: "width: 100%; box-sizing: border-box;",
                aria_label: "Name",
                value: name,
                oninput: move |event: FormEvent| {
                    let text = event.value();
                    apply_panel_edit(panel_epoch, document_epoch, document, |document| {
                        set_name(document, cell, text);
                    });
                },
            }
            div {
                class: "cell-output",
                PanelCheckbox {
                    checked: output,
                    aria_label: "Output",
                    on_checked_change: move |state| {
                        apply_panel_edit(panel_epoch, document_epoch, document, |document| {
                            set_output(document, cell, state == CheckboxState::Checked);
                        });
                    },
                }
                span { "Output" }
            }
            if has_clamp_bounds {
                div {
                    class: "clamp-bound",
                    onfocusout: move |_| {
                        apply_numeric_bound_event(
                            panel_epoch,
                            document_epoch,
                            document,
                            cell,
                            ClampBound::Min,
                            NumericBoundEvent::Blur,
                        );
                        if is_current_panel_epoch(panel_epoch, document_epoch) {
                            min_invalid.set(false);
                            min_error.set(None);
                        }
                    },
                    NumericEditor {
                        id: clamp_min_id,
                        label: "Clamp min",
                        value: min_value,
                        range,
                        kind: number_kind,
                        step,
                        disabled: false,
                        readonly: false,
                        invalid: min_invalid,
                        locale: None,
                        optional: true,
                        on_value: move |value: Option<String>| {
                            if apply_numeric_bound_event(
                                panel_epoch,
                                document_epoch,
                                document,
                                cell,
                                ClampBound::Min,
                                NumericBoundEvent::Value(value.as_deref()),
                            ) {
                                min_value.set(value);
                                min_invalid.set(false);
                                min_error.set(None);
                            }
                        },
                        on_error: move |error: InputError| {
                            if is_current_panel_epoch(panel_epoch, document_epoch) {
                                min_invalid.set(true);
                                min_error.set(Some(numeric_error_message(&error).to_owned()));
                            }
                        },
                    }
                    if let Some(error) = min_error.read().as_deref() {
                        div { role: "alert", class: "diagnostic", "{error}" }
                    }
                }
                div {
                    class: "clamp-bound",
                    onfocusout: move |_| {
                        apply_numeric_bound_event(
                            panel_epoch,
                            document_epoch,
                            document,
                            cell,
                            ClampBound::Max,
                            NumericBoundEvent::Blur,
                        );
                        if is_current_panel_epoch(panel_epoch, document_epoch) {
                            max_invalid.set(false);
                            max_error.set(None);
                        }
                    },
                    NumericEditor {
                        id: clamp_max_id,
                        label: "Clamp max",
                        value: max_value,
                        range,
                        kind: number_kind,
                        step,
                        disabled: false,
                        readonly: false,
                        invalid: max_invalid,
                        locale: None,
                        optional: true,
                        on_value: move |value: Option<String>| {
                            if apply_numeric_bound_event(
                                panel_epoch,
                                document_epoch,
                                document,
                                cell,
                                ClampBound::Max,
                                NumericBoundEvent::Value(value.as_deref()),
                            ) {
                                max_value.set(value);
                                max_invalid.set(false);
                                max_error.set(None);
                            }
                        },
                        on_error: move |error: InputError| {
                            if is_current_panel_epoch(panel_epoch, document_epoch) {
                                max_invalid.set(true);
                                max_error.set(Some(numeric_error_message(&error).to_owned()));
                            }
                        },
                    }
                    if let Some(error) = max_error.read().as_deref() {
                        div { role: "alert", class: "diagnostic", "{error}" }
                    }
                }
            }
            Separator {}
            Label { html_for: restrict_id.clone(), "Restrict" }
            Input {
                id: restrict_id,
                r#type: "text",
                style: "width: 100%; box-sizing: border-box;",
                aria_label: "Restrict",
                value: restrict,
                oninput: move |event: FormEvent| {
                    let text = parse_restrict_input(event.value());
                    apply_panel_edit(panel_epoch, document_epoch, document, |document| {
                        set_restrict(document, cell, text);
                    });
                },
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
///
/// - Complexity: O(text length) parsing and diagnostic formatting work.
///
/// # Examples
///
/// `formula_diagnostic("width +")` returns a readable diagnostic;
/// `formula_diagnostic("")` leaves an unfinished draft quiet.
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
///
/// # Examples
///
/// `panel_target(&HashSet::new())` returns `None` rather than selecting an arbitrary node.
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
/// - Complexity: expected O(n) time and O(u) space for n references and u distinct IDs.
///
/// # Examples
///
/// For a live relationship `group`, `referenced_groups(&[group, group], &[])`
/// returns `vec![group]`.
pub fn referenced_groups(
    default: &[RelationshipGroupId],
    branches: &[ConditionalBranch],
) -> Vec<RelationshipGroupId> {
    let mut seen = std::collections::HashSet::new();
    default
        .iter()
        .chain(branches.iter().flat_map(|branch| &branch.enabled_groups))
        .copied()
        .filter(|group| seen.insert(*group))
        .collect()
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
/// - Precondition: `known` contains distinct group IDs.
/// - Complexity: expected O(k + r) time and O(k + r) space for known and referenced IDs.
///
/// # Examples
///
/// `union_preserving_known_order(&[group_a], &[group_b])` returns
/// `vec![group_a, group_b]` when the IDs differ.
fn union_preserving_known_order(
    known: &[RelationshipGroupId],
    referenced: &[RelationshipGroupId],
) -> Vec<RelationshipGroupId> {
    let mut seen: std::collections::HashSet<_> = known.iter().copied().collect();
    debug_assert_eq!(seen.len(), known.len());
    known
        .iter()
        .copied()
        .chain(
            referenced
                .iter()
                .copied()
                .filter(|group| seen.insert(*group)),
        )
        .collect()
}

/// Retains unchecked live columns in their existing order and discards deleted groups.
///
/// - Complexity: expected O(k + r) time and O(k + r) space for known and referenced columns.
fn live_columns(
    doc: &Document,
    known: &[RelationshipGroupId],
    referenced: &[RelationshipGroupId],
) -> Vec<RelationshipGroupId> {
    let mut columns = union_preserving_known_order(known, referenced);
    columns.retain(|group| doc.relationship_groups.contains_key(*group));
    columns
}

/// Renders `group`'s member formulas as an editable list, each validated
/// live via [`formula_diagnostic`].
///
/// - Complexity: O(m + t) member/text construction per render for m members and t
///   copied text bytes, plus one parse/diagnostic pass per formula.
#[component]
///
/// # Examples
///
/// `SidePanel` mounts this panel for a single live
/// `NodeId::RelationshipGroup`, exposing each member's editable formula.
pub fn RelationshipPanel(
    mut document: Signal<Document>,
    group: RelationshipGroupId,
    document_epoch: Signal<u64>,
    panel_epoch: u64,
) -> Element {
    let (display_name, members) = {
        let doc = document.read();
        let g = &doc.relationship_groups[group];
        let members: Vec<_> = g
            .members
            .iter()
            .map(|(node, formula)| {
                let cell = doc.cells[doc.cell_nodes[*node].cell].name.clone();
                (*node, cell, formula.clone())
            })
            .collect();
        (g.display_name.clone(), members)
    };
    let name_id = format!("name-{group:?}");

    rsx! {
        div {
            class: "relationship-panel",
            Label { html_for: name_id.clone(), "Name" }
            Input {
                id: name_id,
                r#type: "text",
                style: "width: 100%; box-sizing: border-box;",
                aria_label: "Name",
                value: display_name,
                oninput: move |event: FormEvent| {
                    let text = event.value();
                    apply_panel_edit(panel_epoch, document_epoch, document, |document| {
                        set_relationship_display_name(document, group, text);
                    });
                },
            }
            Separator {}
            for (node, cell_name, formula) in members {
                {
                    let field_id = format!("member-formula-{node:?}");
                    let error_id = format!("{field_id}-error");
                    let diagnostic = formula_diagnostic(&formula);
                    rsx! {
                        div {
                            key: "{node:?}",
                            Label { html_for: field_id.clone(), "{cell_name} :=" }
                            Input {
                                id: field_id,
                                r#type: "text",
                                style: "width: 100%; box-sizing: border-box;",
                                aria_label: format!("Formula for {cell_name}"),
                                aria_invalid: diagnostic.is_some(),
                                value: formula,
                                oninput: move |event: FormEvent| {
                                    let text = event.value();
                                    apply_panel_edit(panel_epoch, document_epoch, document, |document| {
                                        set_member_formula(document, group, node, text);
                                    });
                                },
                            }
                            if let Some(diagnostic) = diagnostic {
                                div {
                                    id: error_id,
                                    role: "alert",
                                    class: "diagnostic",
                                    "{diagnostic}"
                                }
                            }
                        }
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
/// - Complexity: O(r + t + b·g·e) per render for copied references r, copied text t,
///   b branches, g displayed groups, and at most e enabled IDs per branch; formula
///   parsing adds its own cost. Retained-column reconciliation is expected linear
///   in known/referenced IDs.
///
/// - Precondition: the caller gives this component a distinct `key` per
///   `conditional` (e.g. in [`SidePanel`]) so switching to a different
///   conditional starts a fresh column set instead of carrying over the
///   previous conditional's columns.
#[component]
///
/// # Examples
///
/// Select one `NodeId::ConditionalGroup` in `SidePanel` to edit its selector
/// draft and enabled-relationship table.
pub fn ConditionalPanel(
    mut document: Signal<Document>,
    conditional: ConditionalGroupId,
    document_epoch: Signal<u64>,
    panel_epoch: u64,
) -> Element {
    let mut known_groups = use_signal(move || {
        let doc = document.peek();
        let cond = &doc.conditional_groups[conditional];
        live_columns(&doc, &[], &referenced_groups(&cond.default, &cond.branches))
    });

    use_effect(move || {
        let referenced = {
            let doc = document.read();
            let cond = &doc.conditional_groups[conditional];
            referenced_groups(&cond.default, &cond.branches)
        };
        let merged = live_columns(&document.read(), &known_groups.peek(), &referenced);
        if merged != *known_groups.peek() {
            known_groups.set(merged);
        }
    });

    let (display_name, branches, formula_expr) = {
        let doc = document.read();
        let cond = &doc.conditional_groups[conditional];
        let formula_expr = formula_expr_for_display(&cond.condition).map(str::to_owned);
        (
            cond.display_name.clone(),
            cond.branches.clone(),
            formula_expr,
        )
    };
    let all_groups = {
        let doc = document.read();
        let cond = &doc.conditional_groups[conditional];
        let referenced = referenced_groups(&cond.default, &cond.branches);
        live_columns(&doc, &known_groups.read(), &referenced)
    };
    let diagnostic = formula_expr.as_deref().and_then(formula_diagnostic);

    let name_id = format!("name-{conditional:?}");
    let formula_id = format!("conditional-formula-{conditional:?}");

    rsx! {
        div {
            class: "conditional-panel",
            Label { html_for: name_id.clone(), "Name" }
            Input {
                id: name_id,
                r#type: "text",
                style: "width: 100%; box-sizing: border-box;",
                aria_label: "Name",
                value: display_name,
                oninput: move |event: FormEvent| {
                    let text = event.value();
                    apply_panel_edit(panel_epoch, document_epoch, document, |document| {
                        set_conditional_display_name(document, conditional, text);
                    });
                },
            }
            Separator {}
            if let Some(expr) = formula_expr {
                div {
                    class: "conditional-formula",
                    Label { html_for: formula_id.clone(), "Condition" }
                    Input {
                        id: formula_id,
                        r#type: "text",
                        style: "width: 100%; box-sizing: border-box;",
                        aria_label: "Condition",
                        aria_invalid: diagnostic.is_some(),
                        value: expr,
                        oninput: move |event: FormEvent| {
                            let text = event.value();
                            apply_panel_edit(panel_epoch, document_epoch, document, |document| {
                                set_condition_formula(document, conditional, text);
                            });
                        },
                    }
                    if let Some(diagnostic) = diagnostic {
                        div { role: "alert", class: "diagnostic", "{diagnostic}" }
                    }
                }
                Separator {}
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
                                    let group_name =
                                        document.read().relationship_groups[group].display_name.clone();
                                    rsx! {
                                        td {
                                            key: "{group:?}",
                                            PanelCheckbox {
                                                checked,
                                                aria_label: format!(
                                                    "Enable {group_name} for branch {}",
                                                    branch_index + 1,
                                                ),
                                                on_checked_change: move |_| {
                                                    apply_panel_edit(
                                                        panel_epoch,
                                                        document_epoch,
                                                        document,
                                                        |document| {
                                                            toggle_enabled_group(
                                                                document,
                                                                conditional,
                                                                branch_index,
                                                                group,
                                                            );
                                                        },
                                                    );
                                                },
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
}

/// Renders whichever of [`CellPanel`]/[`RelationshipPanel`]/[`ConditionalPanel`]
/// matches the current single-node `selection`, or nothing if the
/// selection is empty or multi-node (per [`panel_target`]).
///
/// - Complexity: dispatch is O(1), plus the selected panel's rendering cost.
#[component]
///
/// # Examples
///
/// Inside a Dioxus scope, `SidePanel { document, selection, document_epoch }`
/// follows the document, selection, and replacement generation.
pub fn SidePanel(
    document: Signal<Document>,
    selection: Signal<std::collections::HashSet<NodeId>>,
    document_epoch: Signal<u64>,
) -> Element {
    let panel_epoch = *document_epoch.read();
    let target = panel_target(&selection.read());
    rsx! {
        div {
            class: "side-panel",
            match target {
                Some(NodeId::CellNode(node)) => {
                    let cell = document.read().cell_nodes.get(node).map(|node| node.cell);
                    rsx! { if let Some(cell) = cell { CellPanel {
                        key: "{cell:?}",
                        document,
                        cell,
                        document_epoch,
                        panel_epoch,
                    } } }
                }
                Some(NodeId::RelationshipGroup(group)) => {
                    rsx! { if document.read().relationship_groups.contains_key(group) { RelationshipPanel {
                        key: "{group:?}",
                        document,
                        group,
                        document_epoch,
                        panel_epoch,
                    } } }
                }
                Some(NodeId::ConditionalGroup(conditional)) => rsx! {
                    if document.read().conditional_groups.contains_key(conditional) { ConditionalPanel {
                        key: "{conditional:?}",
                        document,
                        conditional,
                        document_epoch,
                        panel_epoch,
                    } }
                },
                None => rsx! { div { "No selection" } },
            }
        }
    }
}

#[cfg(test)]
mod component_migration_tests {
    use super::*;
    use crate::model::cell::CellType;
    use crate::ops::cells::add_cell;
    use crate::persistence::to_json;

    #[test]
    /// Stores canonical numeric text in the cell's clamp range.
    fn canonical_editor_value_updates_the_numeric_bound() {
        /// Applies and observes the editor's canonical value.
        fn host() -> Element {
            let mut initial = Document::new("demo");
            let cell = add_cell(&mut initial, "amount", CellType::f64());
            let document = use_signal(|| initial);
            let epoch = use_signal(|| 1);

            assert!(apply_numeric_bound_event(
                1,
                epoch,
                document,
                cell,
                ClampBound::Min,
                NumericBoundEvent::Value(Some("1234.5")),
            ));

            let CellType::F64 { clamp } = &document.read().cells[cell].ty else {
                panic!("expected F64");
            };
            assert_eq!(clamp.min, Some(1234.5));
            rsx! {}
        }

        VirtualDom::new(host).rebuild_in_place();
    }

    #[test]
    /// Preserves every digit when a wide integer bound reaches the document operation.
    fn wide_integer_numeric_bounds_preserve_exact_digits() {
        /// Applies and observes the exact canonical integer text.
        fn host() -> Element {
            let mut initial = Document::new("demo");
            let cell = add_cell(&mut initial, "amount", CellType::i64());
            let document = use_signal(|| initial);
            let epoch = use_signal(|| 1);

            assert!(apply_numeric_bound_event(
                1,
                epoch,
                document,
                cell,
                ClampBound::Min,
                NumericBoundEvent::Value(Some("9007199254740993")),
            ));

            let CellType::I64 { clamp } = &document.read().cells[cell].ty else {
                panic!("expected I64");
            };
            assert_eq!(clamp.min, Some(9_007_199_254_740_993));
            rsx! {}
        }

        VirtualDom::new(host).rebuild_in_place();
    }

    #[test]
    /// Stores canonical numeric text in the upper-bound operation.
    fn canonical_editor_value_updates_the_upper_bound() {
        /// Applies and observes a canonical upper-bound value.
        fn host() -> Element {
            let mut initial = Document::new("demo");
            let cell = add_cell(&mut initial, "amount", CellType::f64());
            let document = use_signal(|| initial);
            let epoch = use_signal(|| 1);

            assert!(apply_numeric_bound_event(
                1,
                epoch,
                document,
                cell,
                ClampBound::Max,
                NumericBoundEvent::Value(Some("9876.5")),
            ));

            let CellType::F64 { clamp } = &document.read().cells[cell].ty else {
                panic!("expected F64");
            };
            assert_eq!(clamp.max, Some(9876.5));
            rsx! {}
        }

        VirtualDom::new(host).rebuild_in_place();
    }

    #[test]
    /// Clearing an optional numeric bound removes it from the document.
    fn clearing_a_numeric_bound_sets_it_to_none() {
        /// Clears and observes the minimum bound inside its document owner.
        fn host() -> Element {
            let mut initial = Document::new("demo");
            let cell = add_cell(&mut initial, "amount", CellType::i64());
            let mut document = use_signal(|| initial);
            let epoch = use_signal(|| 1);
            set_clamp_min(&mut document.write(), cell, "10");

            assert!(apply_numeric_bound_event(
                1,
                epoch,
                document,
                cell,
                ClampBound::Min,
                NumericBoundEvent::Value(None),
            ));

            let CellType::I64 { clamp } = &document.read().cells[cell].ty else {
                panic!("expected I64");
            };
            assert_eq!(clamp.min, None);
            rsx! {}
        }

        VirtualDom::new(host).rebuild_in_place();
    }

    #[test]
    /// Blurring a numeric editor leaves the serialized document unchanged.
    fn numeric_editor_blur_does_not_change_the_document() {
        /// Blurs without submitting an editor value.
        fn host() -> Element {
            let mut initial = Document::new("demo");
            let cell = add_cell(&mut initial, "amount", CellType::i64());
            let mut document = use_signal(|| initial);
            let epoch = use_signal(|| 1);
            set_clamp_min(&mut document.write(), cell, "10");
            let serialized_document_before_blur = to_json(&document.read()).unwrap();

            assert!(!apply_numeric_bound_event(
                1,
                epoch,
                document,
                cell,
                ClampBound::Min,
                NumericBoundEvent::Blur,
            ));

            let serialized_document_after_blur = to_json(&document.read()).unwrap();
            assert_eq!(
                serialized_document_after_blur,
                serialized_document_before_blur
            );
            rsx! {}
        }

        VirtualDom::new(host).rebuild_in_place();
    }

    #[test]
    /// Synchronizes controlled checkbox state only when the model changes.
    fn checkbox_state_tracks_document_changes() {
        /// Observes checked and unchecked model updates inside a Dioxus owner.
        fn host() -> Element {
            let state = use_signal(|| None);
            assert!(sync_checkbox_state(state, true));
            assert_eq!(*state.read(), Some(CheckboxState::Checked));
            assert!(!sync_checkbox_state(state, true));
            assert!(sync_checkbox_state(state, false));
            assert_eq!(*state.read(), Some(CheckboxState::Unchecked));
            rsx! {}
        }

        VirtualDom::new(host).rebuild_in_place();
    }

    #[test]
    /// Changes the editor value only when the model value changes.
    fn numeric_value_sync_tracks_only_model_changes() {
        /// Observes changed, unchanged, and cleared values inside a Dioxus owner.
        fn host() -> Element {
            let value = use_signal(|| None::<String>);
            assert!(sync_numeric_value(
                value,
                Some("9007199254740993".to_string())
            ));
            assert_eq!(value.read().as_deref(), Some("9007199254740993"));
            assert!(!sync_numeric_value(
                value,
                Some("9007199254740993".to_string())
            ));
            assert!(sync_numeric_value(value, None));
            assert_eq!(*value.read(), None);
            rsx! {}
        }

        VirtualDom::new(host).rebuild_in_place();
    }

    #[test]
    /// Applies a property edit only when its originating document epoch is current.
    fn current_panel_property_edits_update_the_document() {
        /// Applies and observes a property edit inside its signal owner.
        fn host() -> Element {
            let mut initial = Document::new("demo");
            let cell = add_cell(&mut initial, "before", CellType::i64());
            let document = use_signal(|| initial);
            let epoch = use_signal(|| 7);

            assert!(apply_panel_edit(7, epoch, document, |document| {
                set_name(document, cell, "after");
            }));
            assert_eq!(document.read().cells[cell].name, "after");
            rsx! {}
        }

        VirtualDom::new(host).rebuild_in_place();
    }

    #[test]
    /// An event from an old panel cannot alter a replacement document.
    fn stale_panel_property_event_preserves_the_replacement_document() {
        /// Delivers an old panel event after installing a replacement document.
        fn host() -> Element {
            let mut old = Document::new("old");
            let cell = add_cell(&mut old, "value", CellType::i64());
            let mut document = use_signal(|| old);
            let mut epoch = use_signal(|| 3);

            let replacement = Document::new("replacement");
            document.set(replacement);
            epoch.set(4);
            let serialized_replacement_document = to_json(&document.read()).unwrap();

            assert!(!apply_panel_edit(3, epoch, document, |document| {
                set_name(document, cell, "stale");
            }));

            let serialized_document_after_old_event = to_json(&document.read()).unwrap();
            assert_eq!(
                serialized_document_after_old_event,
                serialized_replacement_document
            );
            rsx! {}
        }

        VirtualDom::new(host).rebuild_in_place();
    }
}

#[cfg(test)]
mod formula_tests {
    use super::*;

    /// Deleted columns disappear while unchecked live columns keep their positions.
    #[test]
    fn conditional_columns_keep_only_live_groups() {
        let mut doc = Document::new("s");
        let live = doc.relationship_groups.insert(
            crate::model::relationship_group::RelationshipGroup::new(
                "live",
                crate::model::geometry::Point::new(0.0, 0.0),
            ),
        );
        let dead = doc.relationship_groups.insert(
            crate::model::relationship_group::RelationshipGroup::new(
                "dead",
                crate::model::geometry::Point::new(0.0, 0.0),
            ),
        );
        doc.relationship_groups.remove(dead);
        assert_eq!(live_columns(&doc, &[live, dead], &[]), vec![live]);
        assert_eq!(referenced_groups(&[live, live], &[]), vec![live]);
    }

    #[test]
    /// Valid formula drafts have no syntax diagnostic.
    fn formula_diagnostic_is_none_for_valid_cel() {
        assert_eq!(formula_diagnostic("a + b"), None);
    }

    #[test]
    /// Empty editable formula drafts suppress syntax diagnostics.
    fn formula_diagnostic_is_none_for_empty_text() {
        assert_eq!(formula_diagnostic(""), None);
    }

    #[test]
    /// Invalid nonempty formula drafts produce syntax diagnostics.
    fn formula_diagnostic_is_some_for_invalid_cel() {
        let diagnostic = formula_diagnostic("a +").expect("invalid CEL should have a diagnostic");
        assert!(!diagnostic.is_empty());
    }

    #[test]
    /// Formula diagnostics use readable messages rather than internal debug output.
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
    /// Nonnumeric cells do not show numeric bound fields.
    fn clamp_bounds_text_is_none_for_bool_and_text() {
        assert_eq!(clamp_bounds_text(&CellType::Bool), None);
        assert_eq!(clamp_bounds_text(&CellType::Text), None);
    }

    #[test]
    /// Bound editors display the stored numeric endpoints.
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
    /// Absent numeric endpoints display blank drafts.
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
    /// Clearing a restriction removes the optional draft.
    fn parse_restrict_input_converts_empty_to_none() {
        assert_eq!(parse_restrict_input(String::new()), None);
    }

    #[test]
    /// Nonempty restrictions retain the authored draft.
    fn parse_restrict_input_converts_nonempty_to_some() {
        assert_eq!(
            parse_restrict_input("x > 0".to_string()),
            Some("x > 0".to_string())
        );
    }

    #[test]
    /// Empty conditional tables reference no relationship groups.
    fn referenced_groups_with_empty_default_and_branches_returns_empty() {
        let result = referenced_groups(&[], &[]);
        assert!(result.is_empty());
    }

    #[test]
    /// Default relationships appear in the referenced-column set.
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
    /// Branch relationships appear in the referenced-column set.
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
    /// Shared default/branch relationships produce one referenced column.
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
    /// Relationships shared across branches produce one referenced column.
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
    /// Newly observed columns preserve reference order.
    fn union_preserving_known_order_with_nothing_known_returns_referenced_in_order() {
        use slotmap::SlotMap;

        let mut groups: SlotMap<RelationshipGroupId, ()> = SlotMap::with_key();
        let group_a = groups.insert(());
        let group_b = groups.insert(());

        let result = union_preserving_known_order(&[], &[group_a, group_b]);
        assert_eq!(result, vec![group_a, group_b]);
    }

    #[test]
    /// Unchecked known columns remain available for later re-enabling.
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
    /// New references append after stable existing columns.
    fn union_preserving_known_order_appends_newly_referenced_groups_after_known_ones() {
        use slotmap::SlotMap;

        let mut groups: SlotMap<RelationshipGroupId, ()> = SlotMap::with_key();
        let group_a = groups.insert(());
        let group_b = groups.insert(());

        let result = union_preserving_known_order(&[group_a], &[group_a, group_b]);
        assert_eq!(result, vec![group_a, group_b]);
    }

    #[test]
    /// Rediscovering a known relationship does not duplicate its column.
    fn union_preserving_known_order_does_not_duplicate_an_already_known_group() {
        use slotmap::SlotMap;

        let mut groups: SlotMap<RelationshipGroupId, ()> = SlotMap::with_key();
        let group_a = groups.insert(());

        let result = union_preserving_known_order(&[group_a], &[group_a]);
        assert_eq!(result, vec![group_a]);
    }

    #[test]
    /// Formula condition panels display the authored scalar expression.
    fn formula_expr_for_display_returns_the_expr_for_formula_mode() {
        let condition = ConditionExpr::Formula {
            referenced_cells: vec![],
            expr: "x > 1.0".to_string(),
        };
        assert_eq!(formula_expr_for_display(&condition), Some("x > 1.0"));
    }

    #[test]
    /// Truth-table condition panels do not invent a formula draft.
    fn formula_expr_for_display_is_none_for_cells_mode() {
        let condition = ConditionExpr::Cells(vec![]);
        assert_eq!(formula_expr_for_display(&condition), None);
    }

    #[test]
    /// Empty selections have no editable side-panel target.
    fn panel_target_is_none_for_empty_selection() {
        let selection = std::collections::HashSet::new();
        assert_eq!(panel_target(&selection), None);
    }

    #[test]
    /// Multiple selections do not choose an arbitrary editable target.
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
    /// Single selections expose their node as the side-panel target.
    fn panel_target_returns_the_single_selected_node() {
        let mut cells: slotmap::SlotMap<crate::model::cell_node::CellNodeId, ()> =
            slotmap::SlotMap::with_key();
        let node = cells.insert(());
        let mut selection = std::collections::HashSet::new();
        selection.insert(NodeId::CellNode(node));
        assert_eq!(panel_target(&selection), Some(NodeId::CellNode(node)));
    }
}
