//! Toolbar for selecting the active interaction tool. See
//! `docs/superpowers/specs/2026-08-26-ez-adam-ui-design.md` §2.

use crate::model::document::Document;
use crate::model::geometry::Point;
use crate::ui::canvas::{NodeId, duplicate_selection};
use adam_web_ui::spectrum::{SpActionButton, SpActionGroup};
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
        SpActionGroup {
            compact: false,
            for tool in tools {
                SpActionButton {
                    selected: tool == *active_tool.read(),
                    onclick: move |_| activate_tool(tool, active_tool, document, selection),
                    "{tool_label(tool)}"
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
}
