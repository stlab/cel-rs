//! Persists exact effective sources independently of aspiration-preserving recovery.
//!
//! Use [`super::SheetOwner::capture`] after acceptance, serialize [`Recording`]
//! with serde, and pass it to [`super::SheetOwner::playback`] with the same schema.

use super::{BindingError, CellEdit, HostRejection};
use crate::{BuildError, BuiltSheet, RuntimeFailure, input::InputError};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Stores a compatible schema's exact effective source values by stable key.
///
/// Values own their encoded text; no runtime IDs or display-rounded values persist.
/// The schema key identifies types, codecs, graph, and uncaptured initial values.
///
/// # Examples
/// ```
/// use adam_web_ui::binding::{CellEdit, Recording};
/// let recording = Recording {
///     schema_key: "example/v1".into(),
///     values: vec![CellEdit { key: "count".into(), canonical: "10".into() }],
/// };
/// let json = serde_json::to_string(&recording).unwrap();
/// assert_eq!(serde_json::from_str::<Recording>(&json).unwrap(), recording);
/// ```
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recording {
    /// Host-defined schema and exact-codec identity.
    pub schema_key: String,
    /// Deduplicated effective sources, never input aspirations.
    pub values: Vec<CellEdit>,
}

/// Distinguishes capture capabilities, encoding, schema, and model availability.
///
/// # Examples
/// ```
/// use adam_web_ui::binding::CaptureError;
/// assert!(matches!(CaptureError::UnnamedSource, CaptureError::UnnamedSource));
/// ```
#[derive(Debug)]
pub enum CaptureError {
    /// A requested result lacks a live stable key.
    UnknownResult(String),
    /// A contributing source has no host-owned stable key.
    UnnamedSource,
    /// A contributing source lacks an exact scalar codec.
    MissingCodec(String),
    /// An exact typed read or encoding failed.
    Encoding {
        /// Stable source key.
        key: String,
        /// Structured codec diagnostic.
        error: InputError,
    },
    /// The recording belongs to another schema or codec contract.
    SchemaMismatch {
        /// Requested model schema.
        expected: String,
        /// Persisted recording schema.
        actual: String,
    },
    /// No accepted live model is available for capture.
    Unavailable(BindingError),
}

/// Preserves construction, input rejection, capture, and fatal playback categories.
///
/// # Examples
/// ```
/// use adam_web_ui::binding::{CaptureError, PlaybackError};
/// let error = PlaybackError::Capture(CaptureError::UnnamedSource);
/// assert!(matches!(error, PlaybackError::Capture(_)));
/// ```
#[derive(Debug)]
pub enum PlaybackError {
    /// Recording compatibility validation failed.
    Capture(CaptureError),
    /// Fresh model construction failed.
    Build(BuildError),
    /// The captured group was rejected and its candidate discarded.
    Rejected(HostRejection),
    /// Runtime propagation failed without reconstruction or retry.
    Fatal(RuntimeFailure),
}

/// Captures selected effective sources using only public source analysis and codecs.
///
/// # Errors
/// Returns unknown results, unnamed sources, missing codecs, or exact encoding errors.
///
/// - Complexity: Public source-analysis cost plus expected O(results + keys +
///   sources + encoded bytes) time and space.
pub(super) fn capture<'a>(
    built: &BuiltSheet,
    schema_key: &str,
    result_keys: impl Iterator<Item = &'a str>,
) -> Result<Recording, CaptureError> {
    let results = result_keys
        .map(|key| {
            built
                .keys
                .get(key)
                .copied()
                .ok_or_else(|| CaptureError::UnknownResult(key.into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut keys = HashMap::with_capacity(built.keys.len());
    // Iterate the authoritative IndexMap in registration order; the reverse
    // HashMap serves only lookups and never determines which alias wins.
    for (key, &id) in &built.keys {
        keys.entry(id).or_insert(key.as_str());
    }
    let mut seen = HashSet::new();
    let values = built
        .parsed
        .source_cells(&results)
        .filter(|id| seen.insert(*id))
        .map(|id| {
            let key = *keys.get(&id).ok_or(CaptureError::UnnamedSource)?;
            let meta = built
                .labels
                .cells
                .get(&id)
                .filter(|meta| meta.codec_supported)
                .ok_or_else(|| CaptureError::MissingCodec(key.into()))?;
            let canonical =
                (meta.capture)(&built.parsed).map_err(|error| CaptureError::Encoding {
                    key: key.into(),
                    error,
                })?;
            Ok(CellEdit {
                key: key.into(),
                canonical,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Recording {
        schema_key: schema_key.into(),
        values,
    })
}
