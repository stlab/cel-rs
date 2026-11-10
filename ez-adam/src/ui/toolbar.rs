//! Toolbar for selecting the active interaction tool. See
//! `docs/superpowers/specs/2026-08-26-ez-adam-ui-design.md` §2.

use crate::model::document::Document;
use crate::model::geometry::Point;
use crate::ui::canvas::{NodeId, duplicate_selection};
use adam_web_ui::components::{Button, ButtonVariant};
use dioxus::prelude::*;
use std::collections::HashSet;

/// The active canvas interaction mode, selected via the toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
///
/// # Examples
///
/// `Tool::Select` enables dragging; choosing `Tool::Duplicate` dispatches a one-shot
/// relationship copy without replacing the active persistent mode.
pub enum Tool {
    /// Selects and moves existing nodes.
    Select,
    /// Collects cells into a relationship group.
    AddRelationship,
    /// Creates a conditional from selected cells and relationships.
    AddConditional,
    /// Duplicates selected relationships once without changing modes.
    Duplicate,
}

impl Default for Tool {
    /// Returns `Tool::Select`.
    fn default() -> Self {
        Tool::Select
    }
}

/// Returns `tool`'s toolbar button label.
fn tool_label(tool: Tool) -> &'static str {
    match tool {
        Tool::Select => "Select",
        Tool::AddRelationship => "Add Relationship",
        Tool::AddConditional => "Add Conditional",
        Tool::Duplicate => "Duplicate",
    }
}

/// Returns whether `tool` is the currently active toolbar mode.
///
/// # Examples
///
/// `tool_button_pressed(Tool::Select, Tool::Select)` returns `true`.
fn tool_button_pressed(active_tool: Tool, tool: Tool) -> bool {
    active_tool == tool
}

/// Applies a tool selection or duplicates selected relationships without changing tools.
///
/// Re-selecting the active tool leaves its signal untouched, preserving pending gestures.
///
/// - Complexity: duplication copies the selected groups and their member/formula contents;
///   selecting a mode takes O(1) time and space.
fn activate_tool(
    tool: Tool,
    mut active_tool: Signal<Tool>,
    mut document: Signal<Document>,
    mut selection: Signal<HashSet<NodeId>>,
) {
    if tool == Tool::Duplicate {
        let duplicated = duplicate_selection(
            &mut document.write(),
            &selection.read(),
            Point::new(0.0, 100.0),
        );
        selection.set(
            duplicated
                .into_iter()
                .map(NodeId::RelationshipGroup)
                .collect(),
        );
    } else if tool != *active_tool.peek() {
        active_tool.set(tool);
    }
}

/// Renders the active mode and dispatches mode changes and one-shot duplication.
///
/// # Examples
///
/// Inside a Dioxus scope, `rsx! { Toolbar { active_tool, document, selection } }`
/// binds tool selection and one-shot duplication to the editor's signals.
#[component]
pub fn Toolbar(
    active_tool: Signal<Tool>,
    document: Signal<Document>,
    selection: Signal<HashSet<NodeId>>,
) -> Element {
    let tools = [
        Tool::Select,
        Tool::AddRelationship,
        Tool::AddConditional,
        Tool::Duplicate,
    ];
    rsx! {
        div {
            class: "toolbar",
            role: "group",
            aria_label: "Canvas tools",
            style: "display: flex; gap: 4px;",
            for tool in tools {
                {
                    let selected = tool_button_pressed(*active_tool.read(), tool);
                    rsx! {
                        Button {
                            variant: if selected { ButtonVariant::Primary } else { ButtonVariant::Outline },
                            aria_pressed: selected,
                            onclick: move |_| activate_tool(tool, active_tool, document, selection),
                            "{tool_label(tool)}"
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Editors start in selection mode.
    fn default_tool_is_select() {
        assert_eq!(Tool::default(), Tool::Select);
    }

    #[test]
    /// Every toolbar mode has a distinct user-visible label.
    fn tool_label_covers_every_variant_with_a_distinct_label() {
        let labels = [
            tool_label(Tool::Select),
            tool_label(Tool::AddRelationship),
            tool_label(Tool::AddConditional),
            tool_label(Tool::Duplicate),
        ];
        let unique: std::collections::HashSet<_> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len());
    }

    #[test]
    /// Only the active toolbar mode exposes a pressed state.
    fn tool_button_pressed_state_tracks_the_active_mode() {
        assert!(tool_button_pressed(
            Tool::AddConditional,
            Tool::AddConditional
        ));
        assert!(!tool_button_pressed(Tool::AddConditional, Tool::Select));
    }

    #[test]
    /// Selecting a toolbar mode updates the mode and preserves the current selection.
    fn selecting_a_toolbar_mode_preserves_selection() {
        /// Applies one toolbar selection inside its owning Dioxus scope.
        fn host() -> Element {
            let document = use_signal(|| Document::new("demo"));
            let selection = use_signal(HashSet::new);
            let active_tool = use_signal(|| Tool::Select);
            activate_tool(Tool::AddConditional, active_tool, document, selection);
            assert_eq!(*active_tool.read(), Tool::AddConditional);
            assert!(selection.read().is_empty());
            rsx! {}
        }

        VirtualDom::new(host).rebuild_in_place();
    }
}
