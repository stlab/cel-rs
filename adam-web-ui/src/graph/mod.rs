//! The property-model constraint graph: D3-ready serialization ([`data`]) and the Dioxus
//! component that renders it.

mod data;

pub use data::{GraphData, LinkData, LinkKind, NodeData, NodeKind, to_graph_data};
