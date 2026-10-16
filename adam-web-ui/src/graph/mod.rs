//! The property-model constraint graph: D3-ready serialization ([`GraphData`] and friends) and
//! the Dioxus component that renders it, and a pure Adam-backed view model.

mod constraints;
mod data;
mod view;

pub use constraints::{ViewAction, ViewConstraints, ViewInput, ViewOutput, ViewTransform};
pub use data::{GraphData, LinkData, LinkKind, NodeData, NodeKind, to_graph_data};
pub use view::{GraphDrive, GraphView, graph_drive_script};
