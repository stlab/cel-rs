//! Defines canonical transactions, immutable publication, and typed host outcomes.
//!
//! Match [`HostOutcome`] at the application boundary; recovery remains explicit.

use crate::input::{InputError, NumberKind, NumericRange};
use crate::{BuildError, BuiltSheet, RuntimeFailure, ValidationIssue};
use std::rc::Rc;

/// Reconstructs the same initialized schema on its owning event loop.
///
/// The factory must reproduce initial values, relationships, keys, and codecs.
///
/// # Examples
/// ```
/// let factory: adam_web_ui::binding::ModelFactory =
///     std::rc::Rc::new(|| adam_web_ui::build_sheet("sheet s {}", "s.adm2"));
/// assert!(factory().is_ok());
/// ```
pub type ModelFactory = Rc<dyn Fn() -> Result<BuiltSheet, BuildError>>;

/// Addresses an input by stable schema key and exact canonical scalar text.
///
/// # Examples
/// ```
/// let edit = adam_web_ui::binding::CellEdit { key: "count".into(), canonical: "20".into() };
/// assert_eq!(edit.canonical, "20");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CellEdit {
    /// Stable host-schema name, never a persisted runtime ID.
    pub key: String,
    /// Exact scalar text: an aspiration in a transaction, effective in a recording.
    pub canonical: String,
}

/// Identifies feedback-loop policy without changing transaction acceptance.
///
/// # Examples
/// ```
/// use adam_web_ui::binding::UpdateOrigin;
/// assert_ne!(UpdateOrigin::User, UpdateOrigin::Playback);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateOrigin {
    /// An application user's edit.
    User,
    /// A replayed recording, not a new user recording event.
    Playback,
    /// An explicit host recovery operation.
    Recovery,
}

/// Retains accepted ordered input groups independently of any mutable model.
///
/// Groups preserve write recency and unclamped aspirations, not effective values.
///
/// # Examples
/// ```
/// let state = adam_web_ui::binding::RecoveryState {
///     schema_key: "example/v1".into(), transactions: vec![]
/// };
/// assert!(state.transactions.is_empty());
/// ```
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RecoveryState {
    /// Identifies the factory's schema and codec contract.
    pub schema_key: String,
    /// Accepted transaction groups in propagation order.
    pub transactions: Vec<Vec<CellEdit>>,
}

/// Publishes interaction state without treating derived cells as disabled.
///
/// # Examples
/// ```
/// let flags = adam_web_ui::binding::CellFlags::default();
/// assert!(!flags.disabled);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellFlags {
    /// Forced or disconnected from relevant results.
    pub disabled: bool,
    /// Explicit output that cannot accept edits.
    pub readonly: bool,
    /// Contributes to an output, requirement, or selector, or no outputs restrict relevance.
    pub relevant: bool,
    /// Has a capability or filter diagnostic, or contributes to a filter failure.
    pub invalid: bool,
    /// Has no alternative relationship direction.
    pub forced: bool,
}

/// Publishes one accepted effective value and its editor capabilities.
///
/// Supported scalars use exact codec text, independently of Inspector rounding.
/// Unsupported named cells remain visible with a capability issue.
///
/// # Examples
/// See [`super::SheetOwner::views`] for reading shared immutable views.
#[derive(Debug, PartialEq)]
pub struct CellView {
    /// Stable host-schema key.
    pub key: String,
    /// Exact effective scalar encoding, or compound display text.
    pub text: String,
    /// Source classification of the last accepted propagation.
    pub is_source: bool,
    /// Interaction and diagnostic state.
    pub flags: CellFlags,
    /// Concrete primitive type for numeric editors.
    pub number_kind: Option<NumberKind>,
    /// Selects the Boolean editor without inferring type from displayed text.
    pub is_bool: bool,
    /// Distinguishes editable scalar text from unsupported compound display text.
    pub codec_supported: bool,
    /// Exact current filter endpoints, when a range filter exists.
    pub range: Option<NumericRange>,
    /// Nonfatal filter or unsupported-capability diagnostics.
    pub issues: Vec<ValidationIssue>,
}

/// Distinguishes key, editor, capability, and owner-lifetime failures.
///
/// # Examples
/// ```
/// use adam_web_ui::binding::BindingError;
/// assert!(matches!(BindingError::UnknownKey("x".into()), BindingError::UnknownKey(_)));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindingError {
    /// No cell has this schema key.
    UnknownKey(String),
    /// The editor cannot write this cell, including explicit outputs.
    IncompatibleEditor(String),
    /// The cell has no exact scalar codec.
    UnsupportedCodec(String),
    /// A fatal failure permanently retired the instance.
    RetiredInstance,
    /// A recoverable rejection discarded the model; explicit restoration is required.
    Unavailable,
    /// Another transaction currently owns the candidate.
    ReentrantMutation,
}

impl std::fmt::Display for BindingError {
    /// Formats the structured failure for a human-readable diagnostic.
    ///
    /// - Complexity: O(message bytes).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownKey(key) => write!(f, "unknown cell key `{key}`"),
            Self::IncompatibleEditor(key) => write!(f, "incompatible editor for `{key}`"),
            Self::UnsupportedCodec(key) => write!(f, "unsupported scalar codec for `{key}`"),
            Self::RetiredInstance => f.write_str("model instance is permanently retired"),
            Self::Unavailable => f.write_str("model requires explicit accepted-state restoration"),
            Self::ReentrantMutation => f.write_str("model transaction is already in progress"),
        }
    }
}

impl std::error::Error for BindingError {}

/// Reports recoverable rejection without retaining the rejected candidate.
///
/// # Examples
/// ```
/// use adam_web_ui::binding::{HostRejection, BindingError};
/// let rejected = HostRejection::Binding(BindingError::Unavailable);
/// assert!(matches!(rejected, HostRejection::Binding(_)));
/// ```
#[derive(Debug)]
pub enum HostRejection {
    /// Conversion rejected the entire group before any mutation.
    Input(InputError),
    /// A key, editor, codec, or owner-state check rejected the group.
    Binding(BindingError),
    /// Successful propagation produced false requirements.
    Validation(Vec<ValidationIssue>),
    /// A mutating write failed; its candidate was discarded without propagation.
    Mutation(RuntimeFailure),
}

/// Returns every transaction outcome to the host without choosing recovery.
///
/// # Examples
/// ```
/// let outcome = adam_web_ui::binding::HostOutcome::Accepted;
/// assert!(matches!(outcome, adam_web_ui::binding::HostOutcome::Accepted));
/// ```
#[derive(Debug)]
pub enum HostOutcome {
    /// Commits the input group and coherently publishes its effective views.
    Accepted,
    /// Accepts the transaction and publication but stops a failed recorder.
    RecordingFailed(super::CaptureError),
    /// Preserves accepted publication/history but requires explicit restoration.
    Rejected(HostRejection),
    /// Permanently retires the model; reconstruction and retry are forbidden.
    Fatal(RuntimeFailure),
}
