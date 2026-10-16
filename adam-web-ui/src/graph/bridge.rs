//! Container-scoped lifecycle and approval protocol for the graph view.
//!
//! Start a fresh monotonically numbered generation before submitting requests.
//! Disposing a generation is terminal; unknown generations never create models.
//!
//! ```
//! use adam_web_ui::graph::BridgeState;
//! let mut bridge = BridgeState::default();
//! bridge.start(1)?;
//! bridge.dispose(1);
//! # Ok::<(), anyhow::Error>(())
//! ```

use anyhow::{Result, ensure};
use cel_euclid::{Point2D, Rect, Size2D};
use serde::{Deserialize, Serialize};

use super::{ViewAction, ViewConstraints, ViewInput, ViewOutput};

/// A geometry snapshot and source action with a generation-local ordering key.
///
/// Bounds use ordered finite minimum/maximum endpoints; viewport sizes are positive.
///
/// # Examples
/// ```
/// # use adam_web_ui::graph::{ViewRequest, ViewAction};
/// let request = ViewRequest { generation: 1, revision: 1, sequence: 1,
///     bounds: [0.0, 0.0, 200.0, 100.0], viewport: [400.0, 300.0],
///     action: ViewAction::Fit };
/// assert_eq!(request.sequence, 1);
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewRequest {
    /// The explicitly started lifecycle.
    pub generation: u32,
    /// The geometry revision measured by the renderer.
    pub revision: u32,
    /// The strictly increasing generation-local submission number.
    pub sequence: u32,
    /// Minimum x/y followed by maximum x/y.
    pub bounds: [f64; 4],
    /// Positive viewport width and height.
    pub viewport: [f64; 2],
    /// The requested source operation.
    pub action: ViewAction,
}

/// Lifecycle and update messages for one mounted container channel.
///
/// # Examples
/// ```
/// use adam_web_ui::graph::ViewMessage;
/// assert_eq!(serde_json::to_string(&ViewMessage::Start { generation: 1 })?,
///     r#"{"kind":"start","generation":1}"#);
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ViewMessage {
    /// Explicitly creates the generation's single cached model.
    Start {
        /// The new strictly greater generation.
        generation: u32,
    },
    /// Evaluates one complete snapshot.
    Update {
        /// The geometry, action and ordering identity.
        request: ViewRequest,
    },
    /// Ends the matching active generation without affecting another generation.
    Dispose {
        /// The generation to retire.
        generation: u32,
    },
}

/// An approval or explicit current-request failure carrying the original identity.
///
/// # Examples
/// ```
/// use adam_web_ui::graph::ViewReply;
/// let reply = ViewReply::Failed { generation: 1, revision: 1, sequence: 1,
///     message: "invalid viewport".into() };
/// assert!(serde_json::to_string(&reply)?.contains(r#""kind":"failed""#));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ViewReply {
    /// Contains only fully evaluated model output.
    Accepted {
        /// The request generation.
        generation: u32,
        /// The request geometry revision.
        revision: u32,
        /// The request submission sequence.
        sequence: u32,
        /// The approved view and model-owned gesture limits.
        output: ViewOutput,
    },
    /// Reports invalid current geometry or a model propagation failure.
    Failed {
        /// The request generation.
        generation: u32,
        /// The request geometry revision.
        revision: u32,
        /// The request submission sequence.
        sequence: u32,
        /// The diagnostic to report visibly and through the host error channel.
        message: String,
    },
}

/// Owns one cached model and lifecycle/order watermarks for one container.
///
/// # Examples
/// ```
/// let mut bridge = adam_web_ui::graph::BridgeState::default();
/// bridge.start(1)?;
/// assert!(bridge.start(1).is_err());
/// # Ok::<(), anyhow::Error>(())
/// ```
#[derive(Default)]
pub struct BridgeState {
    /// The greatest successfully claimed generation, including disposed generations.
    generation: Option<u32>,
    /// The active generation's model, absent after disposal or failed startup.
    model: Option<ViewConstraints>,
    /// The last consumed sequence and geometry revision.
    processed: Option<(u32, u32)>,
}

impl BridgeState {
    /// Decodes a wire message or returns a JavaScript diagnostic as an error.
    ///
    /// # Errors
    /// Returns errors for diagnostic strings, malformed messages, or startup failure.
    ///
    /// - Complexity: O(n + p) time and space in message size `n` and model work `p`.
    pub(crate) fn receive(&mut self, payload: serde_json::Value) -> Result<Option<ViewReply>> {
        if let Some(message) = payload.as_str() {
            anyhow::bail!("{message}");
        }
        self.process(serde_json::from_value(payload)?)
    }

    /// Dispatches a typed lifecycle message without framework-specific branching.
    ///
    /// # Errors
    /// Returns startup errors; current update failures are explicit failed replies.
    ///
    /// - Complexity: O(p) time and space in model construction/propagation work.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::graph::{BridgeState, ViewMessage};
    /// assert!(BridgeState::default().process(ViewMessage::Start { generation: 1 })?.is_none());
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn process(&mut self, message: ViewMessage) -> Result<Option<ViewReply>> {
        match message {
            ViewMessage::Start { generation } => {
                self.start(generation)?;
                Ok(None)
            }
            ViewMessage::Update { request } => self.handle(request),
            ViewMessage::Dispose { generation } => {
                self.dispose(generation);
                Ok(None)
            }
        }
    }
    /// Claims a strictly newer generation and parses its model exactly once.
    ///
    /// # Errors
    /// Returns an error for a repeated/older generation or model initialization failure.
    /// A failed initialization still retires the claimed generation.
    ///
    /// - Complexity: O(p) time and space for model initialization work `p`.
    ///
    /// # Examples
    /// ```
    /// let mut bridge = adam_web_ui::graph::BridgeState::default();
    /// bridge.start(1)?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn start(&mut self, generation: u32) -> Result<()> {
        ensure!(
            self.generation.is_none_or(|last| generation > last),
            "graph generation must increase monotonically"
        );
        self.generation = Some(generation);
        self.processed = None;
        self.model = None;
        self.model = Some(ViewConstraints::new()?);
        Ok(())
    }

    /// Evaluates current ordered work without implicitly starting a generation.
    ///
    /// Returns `None` only for inactive/unknown generations, consumed sequences,
    /// or older geometry. Invalid current inputs produce a `Failed` reply.
    /// Failed current requests consume their ordering identity too.
    ///
    /// # Errors
    /// Reserves transport-level errors for the channel owner; model/input errors
    /// are represented by `ViewReply::Failed`.
    ///
    /// - Complexity: O(p) time and space in cached model propagation work `p`.
    ///
    /// # Examples
    /// ```
    /// # use adam_web_ui::graph::{BridgeState, ViewAction, ViewRequest};
    /// let mut bridge = BridgeState::default();
    /// assert!(bridge.handle(ViewRequest { generation: 1, revision: 1, sequence: 1,
    ///     bounds: [0.0, 0.0, 1.0, 1.0], viewport: [100.0, 100.0],
    ///     action: ViewAction::Fit })?.is_none());
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn handle(&mut self, request: ViewRequest) -> Result<Option<ViewReply>> {
        if self.generation != Some(request.generation) {
            return Ok(None);
        }
        let Some(model) = self.model.as_mut() else {
            return Ok(None);
        };
        if self.processed.is_some_and(|(sequence, revision)| {
            request.sequence <= sequence || request.revision < revision
        }) {
            return Ok(None);
        }
        self.processed = Some((request.sequence, request.revision));
        let [min_x, min_y, max_x, max_y] = request.bounds;
        let result = (|| {
            ensure!(
                request.bounds.iter().all(|v| v.is_finite()) && min_x <= max_x && min_y <= max_y,
                "invalid graph bounds"
            );
            model.update(ViewInput {
                bounds: Rect::new(
                    Point2D::new(min_x, min_y),
                    Size2D::new(max_x - min_x, max_y - min_y),
                ),
                viewport: Size2D::new(request.viewport[0], request.viewport[1]),
                action: request.action,
            })
        })();
        Ok(Some(match result {
            Ok(output) => ViewReply::Accepted {
                generation: request.generation,
                revision: request.revision,
                sequence: request.sequence,
                output,
            },
            Err(error) => ViewReply::Failed {
                generation: request.generation,
                revision: request.revision,
                sequence: request.sequence,
                message: error.to_string(),
            },
        }))
    }

    /// Retires only the matching generation, retaining its terminal watermark.
    ///
    /// # Examples
    /// ```
    /// let mut bridge = adam_web_ui::graph::BridgeState::default();
    /// bridge.start(1)?;
    /// bridge.dispose(1);
    /// assert!(bridge.start(1).is_err());
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn dispose(&mut self, generation: u32) {
        if self.generation == Some(generation) {
            self.model = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{ViewAction, ViewTransform};

    /// Builds a valid absolute proposal with fixed geometry.
    fn request(generation: u32, sequence: u32) -> ViewRequest {
        ViewRequest {
            generation,
            revision: 1,
            sequence,
            bounds: [0.0, 0.0, 200.0, 100.0],
            viewport: [400.0, 300.0],
            action: ViewAction::Propose {
                view: ViewTransform {
                    x: 0.0,
                    y: 50.0,
                    k: 2.0,
                },
            },
        }
    }

    /// Unknown generations never implicitly start a model.
    #[test]
    fn stale_generation_is_ignored() {
        let mut state = BridgeState::default();
        assert!(state.handle(request(1, 1)).unwrap().is_none());
        state.start(2).unwrap();
        assert!(state.handle(request(1, 1)).unwrap().is_none());
        assert!(state.handle(request(3, 1)).unwrap().is_none());
    }

    /// Disposal is terminal even when a delayed Start arrives.
    #[test]
    fn disposed_generation_cannot_restart() {
        let mut state = BridgeState::default();
        state.start(1).unwrap();
        state.dispose(1);
        assert!(state.start(1).is_err());
        assert!(state.handle(request(1, 1)).unwrap().is_none());
        state.start(2).unwrap();
        state.dispose(1);
        assert!(state.handle(request(2, 1)).unwrap().is_some());
    }

    /// Old sequences and geometry never edit the retained source aspiration.
    #[test]
    fn stale_sequence_does_not_mutate_model() {
        let mut state = BridgeState::default();
        state.start(1).unwrap();
        let first = state.handle(request(1, 2)).unwrap().unwrap();
        let mut old = request(1, 1);
        old.action = ViewAction::Fit;
        assert!(state.handle(old).unwrap().is_none());
        let mut old_geometry = request(1, 3);
        old_geometry.revision = 0;
        assert!(state.handle(old_geometry).unwrap().is_none());
        let mut refresh = request(1, 4);
        refresh.action = ViewAction::Refresh;
        let Some(ViewReply::Accepted { output: actual, .. }) = state.handle(refresh).unwrap()
        else {
            panic!("refresh must be approved");
        };
        let ViewReply::Accepted {
            output: expected, ..
        } = first
        else {
            panic!("initial proposal must be approved");
        };
        assert_eq!(expected, actual);
    }

    /// Invalid current geometry returns a failure with the request identity.
    #[test]
    fn current_failure_is_a_failed_reply() {
        let mut state = BridgeState::default();
        state.start(1).unwrap();
        let mut invalid = request(1, 1);
        invalid.viewport[0] = 0.0;
        assert!(matches!(
            state.handle(invalid).unwrap(),
            Some(ViewReply::Failed {
                generation: 1,
                sequence: 1,
                ..
            })
        ));
    }

    /// Serde validates u32 fields rather than truncating hostile numbers.
    #[test]
    fn message_numbers_are_validated() {
        for number in ["-1", "1.5", "4294967296"] {
            assert!(
                serde_json::from_str::<ViewMessage>(&format!(
                    r#"{{"kind":"start","generation":{number}}}"#
                ))
                .is_err()
            );
        }
    }

    /// Malformed current messages and JS diagnostics are errors, never stale success.
    #[test]
    fn wire_errors_are_explicit_and_do_not_consume_valid_work() {
        let mut state = BridgeState::default();
        state.start(1).unwrap();
        assert!(
            state
                .receive(serde_json::json!({
                    "kind": "update", "request": { "generation": 1, "sequence": -1 }
                }))
                .is_err()
        );
        assert_eq!(
            state
                .receive(serde_json::json!("channel failed"))
                .unwrap_err()
                .to_string(),
            "channel failed"
        );
        assert!(state.handle(request(1, 1)).unwrap().is_some());
    }

    /// Container owners retain independent models and sequence watermarks.
    #[test]
    fn two_containers_are_independent() {
        let mut a = BridgeState::default();
        let mut b = BridgeState::default();
        a.start(1).unwrap();
        b.start(1).unwrap();
        a.dispose(1);
        assert!(b.handle(request(1, 1)).unwrap().is_some());
        assert!(a.handle(request(1, 1)).unwrap().is_none());
    }

    /// Shared fixtures encode the exact tags, field names, and action shapes.
    #[test]
    fn wire_fixtures_round_trip() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../begin/js/graph-protocol.json")).unwrap();
        let mut state = BridgeState::default();
        for message in fixture["messages"].as_array().unwrap() {
            let parsed: ViewMessage = serde_json::from_value(message.clone()).unwrap();
            assert_eq!(serde_json::to_value(&parsed).unwrap(), *message);
            assert!(state.process(parsed).is_ok());
        }
        for reply in fixture["replies"].as_array().unwrap() {
            let parsed: ViewReply = serde_json::from_value(reply.clone()).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), *reply);
        }
    }
}
