//! Transports element-local native snapshots without Dioxus's lossy form serializer.
//!
//! The JavaScript owner captures values synchronously and validates composition
//! ranges. Rust retains ordering/epoch guards and owns channel cancellation.

use dioxus::{core::Task, prelude::*};
use serde::Deserialize;
use std::{cell::Cell, future::poll_fn, rc::Rc, task::Poll};

/// Carries an immutable source snapshot; identities never pass through JS Number.
#[derive(Clone, Debug, Deserialize)]
pub(super) struct SourceFrame {
    pub token: String,
    pub epoch: String,
    pub sequence: String,
    pub action: String,
    pub value: String,
    #[serde(default)]
    pub message: String,
}

/// Owns one evaluator and its scope-bound receiver until explicit closure.
pub(super) struct SourceChannel {
    eval: document::Eval,
    task: Task,
    pub epoch: u64,
    pub sequence: u64,
    pub acknowledged: bool,
    acknowledged_epoch: Rc<Cell<Option<u64>>>,
    callback: Callback<Result<SourceFrame, String>>,
}

impl SourceChannel {
    /// Starts the common web/desktop source; reports completion as a source failure.
    ///
    /// - Complexity: O(adapter source bytes) startup time and space.
    pub fn start(token: u64, callback: Callback<Result<SourceFrame, String>>) -> Self {
        let script = format!(
            "{}\nreturn await runNumericEvents(window, \"{token}\", dioxus);",
            include_str!("numeric_events.js").replace("export ", "")
        );
        let eval = document::eval(&script);
        let task = spawn(async move {
            let mut receiver = eval;
            let mut ended = std::pin::pin!(eval.join::<serde_json::Value>());
            loop {
                let mut message = std::pin::pin!(receiver.recv::<SourceFrame>());
                let result = poll_fn(|cx| {
                    if let Poll::Ready(message) = message.as_mut().poll(cx) {
                        return Poll::Ready(message.map_err(|error| error.to_string()));
                    }
                    ended.as_mut().poll(cx).map(|result| {
                        Err(match result {
                            Ok(_) => "Numeric source closed unexpectedly".into(),
                            Err(error) => error.to_string(),
                        })
                    })
                })
                .await;
                let failed = result.is_err();
                callback.call(result);
                if failed {
                    let _ = eval.send(serde_json::json!({"kind":"close"}));
                    break;
                }
            }
        });
        Self {
            eval,
            task,
            epoch: 0,
            sequence: 0,
            acknowledged: false,
            acknowledged_epoch: Rc::new(Cell::new(None)),
            callback,
        }
    }

    /// Accepts only current epoch, increasing source sequence, and reset acknowledgment.
    pub fn accept(&mut self, frame: &SourceFrame, token: u64) -> bool {
        if frame.token.parse::<u64>() != Ok(token) || frame.epoch.parse::<u64>() != Ok(self.epoch) {
            return false;
        }
        let Ok(sequence) = frame.sequence.parse::<u64>() else {
            return false;
        };
        if sequence <= self.sequence {
            return false;
        }
        self.sequence = sequence;
        if frame.action == "ready" {
            self.acknowledged = true;
            self.acknowledged_epoch.set(Some(self.epoch));
        }
        self.acknowledged
    }

    /// Retires queued old input before atomically resetting the native source owner.
    ///
    /// - Complexity: O(value bytes) serialization time and space.
    /// # Errors
    /// Returns the transport diagnostic when reset cannot be delivered.
    pub fn reset(&mut self, value: &str, disabled: bool, readonly: bool) -> Result<(), String> {
        self.epoch = self.epoch.checked_add(1).expect("source epochs exhausted");
        self.acknowledged = false;
        self.eval
            .send(serde_json::json!({
                "kind":"reset", "epoch":self.epoch.to_string(), "value":value,
                "disabled":disabled,"readonly":readonly
            }))
            .map_err(|error| error.to_string())?;
        let epoch = self.epoch;
        let acknowledged = self.acknowledged_epoch.clone();
        let callback = self.callback;
        spawn(async move {
            // This deadline diagnoses transport health; it never completes composition.
            let result = document::eval(
                "return new Promise(resolve => setTimeout(() => resolve(null), 5000));",
            )
            .join::<serde_json::Value>()
            .await;
            if acknowledged.get().is_none_or(|received| received < epoch) {
                callback.call(Err(match result {
                    Ok(_) => "Numeric source reset acknowledgment timed out".into(),
                    Err(error) => format!("Numeric source reset deadline failed: {error}"),
                }));
            }
        });
        Ok(())
    }

    /// Applies display text only if no newer local edit or composition has occurred.
    ///
    /// - Complexity: O(value bytes) serialization time and space.
    /// # Errors
    /// Returns the transport diagnostic when display cannot be delivered.
    pub fn display(&self, value: &str) -> Result<(), String> {
        self.eval
            .send(serde_json::json!({
                "kind":"display", "epoch":self.epoch.to_string(),
                "sequence":self.sequence.to_string(),"value":value
            }))
            .map_err(|error| error.to_string())
    }
}

impl Drop for SourceChannel {
    /// Closes the JS owner before cancelling the scoped receiver, including during startup.
    fn drop(&mut self) {
        self.acknowledged_epoch.set(Some(u64::MAX));
        let _ = self.eval.send(serde_json::json!({"kind":"close"}));
        self.task.cancel();
    }
}
