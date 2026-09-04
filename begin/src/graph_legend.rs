//! [`GraphLegend`] — `begin`'s legend overlay for the constraint graph.
//!
//! Rendered by `App` as an absolutely-positioned overlay on top of the shared
//! [`adam_web_ui::GraphView`], not by `GraphView` itself: the legend is `begin`
//! chrome, and the book's live graphs deliberately render without it.

use dioxus::prelude::*;

/// Explains the graph's shapes, line styles, and outline colors. A static
/// key, not tied to any sheet — every symbol it documents is fixed by
/// `graph.css`/`graph.js`, not by which sheet happens to be loaded.
#[component]
pub fn GraphLegend() -> Element {
    rsx! {
        div {
            class: "graph-legend",
            // Every row's icon sits in a fixed-width `graph-legend-icon`
            // column, whatever shape is actually inside it (a 14px square,
            // a 10px diamond, a 22px-wide edge glyph) - so the text after
            // it always starts at the same x position instead of drifting
            // with each icon's own width.
            div {
                class: "graph-legend-row",
                div { class: "graph-legend-icon",
                    div { class: "graph-legend-shape cell" }
                }
                "Cell (value)"
            }
            div {
                class: "graph-legend-row",
                div { class: "graph-legend-icon",
                    div { class: "graph-legend-shape relationship" }
                }
                "Relationship (constraint)"
            }
            div {
                class: "graph-legend-row",
                div { class: "graph-legend-icon",
                    div { class: "graph-legend-shape conditional" }
                }
                "Conditional (branch)"
            }
            div {
                class: "graph-legend-row",
                // Mirrors graph.js's #arrowhead marker: a solid triangle,
                // tip in the direction of travel.
                div {
                    class: "graph-legend-icon",
                    svg {
                        view_box: "0 0 22 14",
                        width: "22",
                        height: "14",
                        line { x1: "2", y1: "7", x2: "16", y2: "7", stroke: "#444", stroke_width: "1.5" }
                        path { d: "M16,3.5 L21,7 L16,10.5 Z", fill: "#444" }
                    }
                }
                "Depends on"
            }
            div {
                class: "graph-legend-row",
                // Mirrors graph.js's #dot marker: a solid circle on a
                // dashed line, matching link-control edges.
                div {
                    class: "graph-legend-icon",
                    svg {
                        view_box: "0 0 22 14",
                        width: "22",
                        height: "14",
                        line {
                            x1: "2", y1: "7", x2: "17", y2: "7",
                            stroke: "#444", stroke_width: "1.5", stroke_dasharray: "4 3",
                        }
                        circle { cx: "19", cy: "7", r: "3", fill: "#444" }
                    }
                }
                "Activates when matched"
            }
            div {
                class: "graph-legend-row",
                div { class: "graph-legend-icon",
                    div { class: "graph-legend-shape forced" }
                }
                "Forced (not directly editable)"
            }
            div {
                class: "graph-legend-row",
                div { class: "graph-legend-icon",
                    div { class: "graph-legend-shape inactive" }
                }
                "Inactive (branch not selected)"
            }
        }
    }
}
