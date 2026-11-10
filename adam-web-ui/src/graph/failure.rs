//! Preserves graph error classification at the application boundary.
//!
//! Match `GraphFailure::Runtime` before choosing a shutdown policy.
use crate::{Renderer, RuntimeFailure};

/// Distinguishes malformed requests, channel errors, and fatal model propagation.
///
/// # Examples
/// ```
/// use adam_web_ui::graph::GraphFailure;
/// let error = GraphFailure::InvalidInput("invalid viewport".into());
/// assert!(error.to_string().contains("viewport"));
/// ```
#[derive(Debug)]
pub enum GraphFailure {
    /// An input rejected before mutation or an invalid computed view.
    InvalidInput(String),
    /// A failed or malformed browser channel.
    Transport(String),
    /// A failed mutating operation with independently owned diagnostics.
    Runtime(Box<RuntimeFailure>),
    /// A previous mutating error permanently discarded the candidate.
    Unavailable,
}

impl GraphFailure {
    /// Returns whether the channel must stop rather than accept another request.
    pub(crate) fn stops_channel(&self) -> bool {
        matches!(self, Self::Runtime(_) | Self::Unavailable)
    }
}

impl std::fmt::Display for GraphFailure {
    /// Renders independently owned diagnostics without consulting a failed sheet.
    ///
    /// - Complexity: O(rendered diagnostic bytes).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message) | Self::Transport(message) => f.write_str(message),
            Self::Runtime(failure) => {
                let source = failure.diagnostics.anonymous_source();
                f.write_str(&crate::labels::format_adam_error(
                    &failure.error,
                    &failure.diagnostics,
                    source.map_or("", adam_lang::SourceId::text),
                    &failure.file_name,
                    &Renderer::plain(),
                ))
            }
            Self::Unavailable => f.write_str("graph model unavailable after a mutating failure"),
        }
    }
}

impl std::error::Error for GraphFailure {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only discarded candidates stop the channel; malformed external data is recoverable.
    #[test]
    fn terminal_classification_preserves_runtime_cause() {
        assert!(!GraphFailure::InvalidInput("bad bounds".into()).stops_channel());
        assert!(!GraphFailure::Transport("bad wire".into()).stops_channel());
        assert!(GraphFailure::Unavailable.stops_channel());
        let error = crate::build_sheet(
            "sheet s { cell divisor: i32 = 0; out result: i32 := 10 / divisor; }",
            "graph-failure.adm2",
        )
        .err()
        .unwrap();
        let crate::BuildError::Runtime(failure) = error else {
            panic!("runtime fixture");
        };
        assert!(GraphFailure::Runtime(failure).stops_channel());
    }
}
