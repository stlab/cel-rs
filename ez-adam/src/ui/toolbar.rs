//! Toolbar for selecting the active interaction tool. See
//! `docs/superpowers/specs/2026-08-26-ez-adam-ui-design.md` §2.

use dioxus::prelude::*;

/// The active canvas interaction mode, selected via the toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    AddRelationship,
    AddConditional,
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

/// Returns the CSS class name for a tool button based on whether it is the active tool.
///
/// Returns `"tool-active"` if `active == tool`, `"tool"` otherwise.
fn tool_button_class(active: Tool, tool: Tool) -> &'static str {
    if active == tool {
        "tool-active"
    } else {
        "tool"
    }
}

/// Renders one button per [`Tool`], highlighting whichever is currently
/// active in `active_tool`, and updates it on click. A direct passthrough
/// of a click to `active_tool.set(...)` with no branching of its own —
/// per this workspace's testing convention, delegating the button class
/// decision to `tool_button_class`.
#[component]
pub fn Toolbar(active_tool: Signal<Tool>) -> Element {
    let tools = [
        Tool::Select,
        Tool::AddRelationship,
        Tool::AddConditional,
        Tool::Duplicate,
    ];
    rsx! {
        div {
            class: "toolbar",
            for tool in tools {
                button {
                    class: tool_button_class(*active_tool.read(), tool),
                    onclick: move |_| active_tool.set(tool),
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
    fn default_tool_is_select() {
        assert_eq!(Tool::default(), Tool::Select);
    }

    #[test]
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
    fn tool_button_class_active_tool_returns_tool_active() {
        assert_eq!(tool_button_class(Tool::Select, Tool::Select), "tool-active");
        assert_eq!(
            tool_button_class(Tool::AddRelationship, Tool::AddRelationship),
            "tool-active"
        );
    }

    #[test]
    fn tool_button_class_inactive_tool_returns_tool() {
        assert_eq!(
            tool_button_class(Tool::Select, Tool::AddRelationship),
            "tool"
        );
        assert_eq!(
            tool_button_class(Tool::AddConditional, Tool::Duplicate),
            "tool"
        );
    }
}
