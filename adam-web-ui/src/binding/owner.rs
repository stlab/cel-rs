//! Serializes transactions without exposing or reusing discarded candidates.
//!
//! [`SheetOwner`] keeps accepted inputs and views outside its mutable model.

use super::types::*;
use super::{CaptureError, PlaybackError, Recording, recording};
use crate::build::validation_issues;
use crate::{BuildError, BuiltSheet, Labels, RuntimeFailure, ValidationIssue};
use adam_lang::ParsedSheet;
use adam_rs::{CellId, Error};
use std::{collections::HashSet, rc::Rc};

/// Represents exclusive candidate ownership and distinguishes fatal retirement.
enum ModelState {
    /// Contains only an accepted live model.
    Live(Box<BuiltSheet>),
    /// Requires host-requested reconstruction.
    Unavailable,
    /// Forbids factory calls and further mutation.
    Retired,
    /// Reserves ownership while a transaction is in progress.
    Mutating,
}

/// Owns one event-loop-confined model, accepted journal, and immutable publication.
///
/// Every rejection preserves published views and journal, but discards the live
/// candidate. Only explicit restoration makes inspection available again.
/// A fatal propagation error permanently forbids reconstruction and retry.
/// The `Rc` factory and views intentionally make no `Send` or `Sync` promise.
///
/// # Examples
/// ```
/// use adam_web_ui::{binding::*, build_sheet};
/// let mut owner = SheetOwner::new(std::rc::Rc::new(||
///     build_sheet("sheet s { cell count: i32 = 1; }", "s.adm2")), "s/v1".into()).unwrap();
/// let edit = CellEdit { key: "count".into(), canonical: "2".into() };
/// assert!(matches!(owner.apply(&[edit], UpdateOrigin::User), HostOutcome::Accepted));
/// assert_eq!(owner.views()[0].text, "2");
/// ```
pub struct SheetOwner {
    factory: ModelFactory,
    model: ModelState,
    accepted: RecoveryState,
    views: Vec<Rc<CellView>>,
    recording_results: Option<Vec<String>>,
    recordings: Vec<Rc<Recording>>,
}

impl SheetOwner {
    /// Constructs and publishes one valid initial model without requiring outputs.
    ///
    /// # Errors
    /// Returns construction errors or typed publication/capability-read failures.
    ///
    /// - Complexity: Factory cost plus publication over all named cells and their
    ///   contributing graph traversals.
    ///
    /// # Examples
    /// See [`SheetOwner`] for construction and a transaction.
    pub fn new(factory: ModelFactory, schema_key: String) -> Result<Self, BuildError> {
        let built = factory()?;
        let views = publish(&built, &[]).map_err(|error| BuildError::Recovery(Box::new(error)))?;
        Ok(Self {
            factory,
            model: ModelState::Live(Box::new(built)),
            accepted: RecoveryState {
                schema_key,
                transactions: Vec::new(),
            },
            views,
            recording_results: None,
            recordings: Vec::new(),
        })
    }

    /// Parses every edit, writes in order, and propagates once before acceptance.
    ///
    /// All accepted origins enter recovery history. Only `User` actions append
    /// enabled recordings; playback and recovery suppress recording feedback.
    /// An empty group is an accepted no-op on a live model and is not journaled.
    ///
    /// - Postcondition: Rejection leaves accepted views and history unchanged and
    ///   makes inspection unavailable until explicit restoration.
    /// - Postcondition: A propagation error retires the owner permanently.
    /// - Postcondition: Recording failure preserves the newly accepted model,
    ///   views, and history, stops recording, and returns `RecordingFailed`.
    /// - Complexity: O(edit bytes + propagation + publication + enabled capture);
    ///   retained history grows only by the accepted group and new recording,
    ///   never by a copy of the prior journal.
    ///
    /// # Examples
    /// See [`SheetOwner`] for submitting a canonical edit.
    pub fn apply(&mut self, edits: &[CellEdit], origin: UpdateOrigin) -> HostOutcome {
        if let Some(error) = state_error(&self.model) {
            return HostOutcome::Rejected(HostRejection::Binding(error));
        }
        if edits.is_empty() {
            return HostOutcome::Accepted;
        }
        let ModelState::Live(built) = std::mem::replace(&mut self.model, ModelState::Mutating)
        else {
            unreachable!("live state checked before taking exclusive ownership");
        };
        match transact(built, edits) {
            Ok(built) => match publish(&built, &self.views) {
                Ok(views) => {
                    self.accepted.transactions.push(edits.to_vec());
                    self.views = views;
                    self.model = ModelState::Live(built);
                    self.record_accepted(origin)
                }
                Err(error) => {
                    self.model = ModelState::Unavailable;
                    HostOutcome::Rejected(error)
                }
            },
            Err(outcome) => {
                self.model = if matches!(outcome, HostOutcome::Fatal(_)) {
                    ModelState::Retired
                } else {
                    ModelState::Unavailable
                };
                outcome
            }
        }
    }

    /// Captures exact effective sources contributing to the selected stable keys.
    ///
    /// Empty selection captures all sources. Source order is unspecified; aliases
    /// use the first registered stable key. Models must satisfy the fixed-point
    /// replay contracts of [`adam_rs::Sheet::source_cells`].
    ///
    /// # Errors
    /// Returns explicit result, source identity, codec, encoding, or availability errors.
    ///
    /// - Complexity: Source-analysis cost plus expected O(results + keys + sources
    ///   + encoded bytes) time and space.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::{binding::*, build_sheet};
    /// let owner = SheetOwner::new(std::rc::Rc::new(||
    ///     build_sheet("sheet s { cell x: i32 = 1; }", "s.adm2")), "s/v1".into()).unwrap();
    /// assert_eq!(owner.capture(&["x"]).unwrap().values[0].canonical, "1");
    /// ```
    pub fn capture(&self, result_keys: &[&str]) -> Result<Recording, CaptureError> {
        match &self.model {
            ModelState::Live(built) => recording::capture(
                built,
                &self.accepted.schema_key,
                result_keys.iter().copied(),
            ),
            other => Err(CaptureError::Unavailable(
                state_error(other).expect("non-live state"),
            )),
        }
    }

    /// Enables subsequent accepted user-action capture after validating selection.
    ///
    /// Does not append an initial recording. A failed start preserves prior selection.
    ///
    /// # Errors
    /// Returns the same capability, encoding, and availability errors as [`Self::capture`].
    ///
    /// - Complexity: Capture cost plus O(result-key bytes) retained space.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::{binding::*, build_sheet};
    /// let mut owner = SheetOwner::new(std::rc::Rc::new(||
    ///     build_sheet("sheet s { cell x: i32 = 1; }", "s.adm2")), "s/v1".into()).unwrap();
    /// owner.start_recording(&["x"]).unwrap();
    /// let outcome = owner.apply(&[CellEdit { key: "x".into(), canonical: "2".into() }],
    ///     UpdateOrigin::User);
    /// assert!(matches!(outcome, HostOutcome::Accepted));
    /// assert_eq!(owner.recordings()[0].values[0].canonical, "2");
    /// owner.stop_recording();
    /// ```
    pub fn start_recording(&mut self, result_keys: &[&str]) -> Result<(), CaptureError> {
        self.capture(result_keys)?;
        self.recording_results = Some(result_keys.iter().map(|key| (*key).into()).collect());
        Ok(())
    }

    /// Stops future action recording without deleting retained recordings.
    ///
    /// - Complexity: O(selected result-key bytes) destruction.
    ///
    /// # Examples
    /// See [`Self::start_recording`]; call `owner.stop_recording()` to stop capture.
    pub fn stop_recording(&mut self) {
        self.recording_results = None;
    }

    /// Borrows shared immutable recordings without copying action history.
    ///
    /// # Examples
    /// See [`Self::start_recording`]; `owner.recordings().is_empty()` holds before actions.
    pub fn recordings(&self) -> &[Rc<Recording>] {
        &self.recordings
    }

    /// Replays one effective source group on a fresh compatible owner without recording.
    ///
    /// Validates schema before construction and decodes every edit before mutation.
    /// Writes all values before one propagation and requirement validation.
    ///
    /// # Errors
    /// Returns schema, build, rejected-input/requirement, or fatal runtime errors.
    /// Fatal failures never trigger a retry or reconstruction.
    ///
    /// - Complexity: Factory, transaction, and publication cost plus encoded bytes.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::{binding::*, build_sheet};
    /// let factory: ModelFactory = std::rc::Rc::new(||
    ///     build_sheet("sheet s { cell x: i32 = 1; }", "s.adm2"));
    /// let owner = SheetOwner::new(factory.clone(), "s/v1".into()).unwrap();
    /// let recording = owner.capture(&[]).unwrap();
    /// let replayed = SheetOwner::playback(factory, "s/v1".into(), &recording).unwrap();
    /// assert!(replayed.recordings().is_empty());
    /// ```
    pub fn playback(
        factory: ModelFactory,
        schema_key: String,
        recording: &Recording,
    ) -> Result<Self, PlaybackError> {
        if schema_key != recording.schema_key {
            return Err(PlaybackError::Capture(CaptureError::SchemaMismatch {
                expected: schema_key,
                actual: recording.schema_key.clone(),
            }));
        }
        let mut owner = Self::new(factory, schema_key).map_err(|error| match error {
            BuildError::Runtime(failure) => PlaybackError::Fatal(*failure),
            other => PlaybackError::Build(other),
        })?;
        match owner.apply(&recording.values, UpdateOrigin::Playback) {
            HostOutcome::Accepted => Ok(owner),
            HostOutcome::Rejected(error) => Err(PlaybackError::Rejected(error)),
            HostOutcome::Fatal(error) => Err(PlaybackError::Fatal(error)),
            HostOutcome::RecordingFailed(error) => Err(PlaybackError::Capture(error)),
        }
    }

    /// Appends one user recording or explicitly stops a failed recorder.
    ///
    /// - Complexity: Capture cost.
    fn record_accepted(&mut self, origin: UpdateOrigin) -> HostOutcome {
        if origin != UpdateOrigin::User {
            return HostOutcome::Accepted;
        }
        let Some(results) = &self.recording_results else {
            return HostOutcome::Accepted;
        };
        let ModelState::Live(built) = &self.model else {
            unreachable!("recording follows accepted publication");
        };
        match recording::capture(
            built,
            &self.accepted.schema_key,
            results.iter().map(String::as_str),
        ) {
            Ok(recording) => {
                self.recordings.push(Rc::new(recording));
                HostOutcome::Accepted
            }
            Err(error) => {
                self.stop_recording();
                HostOutcome::RecordingFailed(error)
            }
        }
    }

    /// Reconstructs accepted ordered inputs only at the host's explicit request.
    ///
    /// Publication changes only after the factory and every replay group succeed.
    /// Failed candidates are discarded, never substituted or inspected.
    ///
    /// # Errors
    /// Returns typed construction, replay, or publication failures. A retired
    /// owner rejects restoration without calling its factory. A fatal failure
    /// during reconstruction or replay permanently retires the owner.
    ///
    /// - Complexity: O(history bytes + propagation per group + publication),
    ///   plus factory and independently owned diagnostic metadata costs.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::{binding::*, build_sheet};
    /// let mut owner = SheetOwner::new(std::rc::Rc::new(||
    ///     build_sheet("sheet s { cell x: i32 = 1; }", "s.adm2")), "s/v1".into()).unwrap();
    /// let _ = owner.apply(&[CellEdit { key: "x".into(), canonical: "bad".into() }],
    ///     UpdateOrigin::User);
    /// owner.restore_accepted().unwrap();
    /// assert_eq!(owner.views()[0].text, "1");
    /// ```
    pub fn restore_accepted(&mut self) -> Result<(), BuildError> {
        match self.model {
            ModelState::Retired => return Err(binding_build_error(BindingError::RetiredInstance)),
            ModelState::Mutating => {
                return Err(binding_build_error(BindingError::ReentrantMutation));
            }
            _ => {}
        }
        self.model = ModelState::Mutating;
        let result = self.reconstruct();
        match result {
            Ok((built, views)) => {
                self.model = ModelState::Live(built);
                self.views = views;
                Ok(())
            }
            Err(error) => {
                self.model = if matches!(error, BuildError::Runtime(_)) {
                    ModelState::Retired
                } else {
                    ModelState::Unavailable
                };
                Err(error)
            }
        }
    }

    /// Borrows host-owned accepted history without cloning it.
    ///
    /// # Examples
    /// See [`SheetOwner`] for construction; a new owner's transactions are empty.
    pub fn accepted_state(&self) -> &RecoveryState {
        &self.accepted
    }

    /// Borrows immutable accepted cell views in stable schema-key order.
    ///
    /// Callers may clone individual `Rc`s without copying the journal or model.
    ///
    /// # Examples
    /// See [`SheetOwner`] for reading the first published cell.
    pub fn views(&self) -> &[Rc<CellView>] {
        &self.views
    }

    /// Invokes a read-only callback only on an accepted live model.
    ///
    /// # Errors
    /// Returns a typed unavailable, retired, or reentrant error without invoking
    /// the callback when no accepted live model exists.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::{binding::*, build_sheet};
    /// let owner = SheetOwner::new(std::rc::Rc::new(||
    ///     build_sheet("sheet s {}", "s.adm2")), "s/v1".into()).unwrap();
    /// assert_eq!(owner.inspect(|parsed, _| parsed.cell_names.len()).unwrap(), 0);
    /// ```
    pub fn inspect<R>(
        &self,
        f: impl FnOnce(&ParsedSheet, &Labels) -> R,
    ) -> Result<R, BindingError> {
        match &self.model {
            ModelState::Live(built) => Ok(f(&built.parsed, &built.labels)),
            other => Err(state_error(other).expect("non-live states have a diagnostic")),
        }
    }

    /// Builds and replays without changing publication or copying the journal.
    ///
    /// - Complexity: Factory cost plus O(history bytes + propagation per group +
    ///   diagnostic metadata + publication).
    fn reconstruct(&self) -> Result<(Box<BuiltSheet>, Vec<Rc<CellView>>), BuildError> {
        let mut built = Box::new((self.factory)()?);
        let diagnostics = built.diagnostics.clone();
        let file_name = built.file_name.clone();
        for edits in &self.accepted.transactions {
            built = match transact(built, edits) {
                Ok(built) => built,
                Err(HostOutcome::Fatal(error)) => return Err(BuildError::Runtime(Box::new(error))),
                Err(HostOutcome::Rejected(HostRejection::Validation(issues))) => {
                    return Err(BuildError::Validation {
                        issues,
                        diagnostics: Box::new(diagnostics),
                        file_name,
                    });
                }
                Err(HostOutcome::Rejected(error)) => {
                    return Err(BuildError::Recovery(Box::new(error)));
                }
                Err(HostOutcome::Accepted | HostOutcome::RecordingFailed(_)) => {
                    unreachable!("transaction errors are never accepted")
                }
            };
        }
        let views =
            publish(&built, &self.views).map_err(|error| BuildError::Recovery(Box::new(error)))?;
        Ok((built, views))
    }
}

/// Returns the lifetime diagnostic for a non-live model state.
fn state_error(state: &ModelState) -> Option<BindingError> {
    match state {
        ModelState::Live(_) => None,
        ModelState::Unavailable => Some(BindingError::Unavailable),
        ModelState::Retired => Some(BindingError::RetiredInstance),
        ModelState::Mutating => Some(BindingError::ReentrantMutation),
    }
}

/// Wraps an owner-state failure for the construction/recovery boundary.
fn binding_build_error(error: BindingError) -> BuildError {
    BuildError::Recovery(Box::new(HostRejection::Binding(error)))
}

/// Moves pre-retained independent diagnostics out while discarding a candidate.
fn runtime_failure(built: BuiltSheet, error: Error) -> RuntimeFailure {
    RuntimeFailure {
        error,
        diagnostics: Box::new(built.diagnostics),
        file_name: built.file_name,
    }
}

/// Applies a fully prepared ordered transaction to an exclusively owned candidate.
///
/// - Complexity: O(edits + outputs + edit bytes + propagation + requirement validation).
fn transact(
    mut built: Box<BuiltSheet>,
    edits: &[CellEdit],
) -> Result<Box<BuiltSheet>, HostOutcome> {
    let outputs: HashSet<_> = built.parsed.out_cells().collect();
    let prepared = edits
        .iter()
        .map(|edit| {
            let id = *built.keys.get(&edit.key).ok_or_else(|| {
                HostRejection::Binding(BindingError::UnknownKey(edit.key.clone()))
            })?;
            let meta = built.labels.cells.get(&id).ok_or_else(|| {
                HostRejection::Binding(BindingError::UnsupportedCodec(edit.key.clone()))
            })?;
            if !meta.codec_supported {
                return Err(HostRejection::Binding(BindingError::UnsupportedCodec(
                    edit.key.clone(),
                )));
            }
            if outputs.contains(&id) {
                return Err(HostRejection::Binding(BindingError::IncompatibleEditor(
                    edit.key.clone(),
                )));
            }
            (meta.prepare)(&edit.canonical)
                .map(|write| (id, write))
                .map_err(HostRejection::Input)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(HostOutcome::Rejected)?;
    // Apply only after every conversion succeeds; order defines write recency.
    for (id, write) in prepared {
        if let Err(error) = write.apply(&mut built.parsed, id) {
            return Err(HostOutcome::Rejected(HostRejection::Mutation(
                runtime_failure(*built, error),
            )));
        }
    }
    if let Err(error) = built.parsed.propagate() {
        return Err(HostOutcome::Fatal(runtime_failure(*built, error)));
    }
    let issues = validation_issues(&built.parsed);
    if !issues.is_empty() {
        return Err(HostOutcome::Rejected(HostRejection::Validation(issues)));
    }
    built.parsed.clear_changed();
    Ok(built)
}

/// Publishes every named cell or reports a read error instead of substituting a value.
///
/// - Complexity: O(named cells + contributing graph traversals + encoded bytes).
fn publish(
    built: &BuiltSheet,
    previous: &[Rc<CellView>],
) -> Result<Vec<Rc<CellView>>, HostRejection> {
    let sheet = &built.parsed;
    let outputs: HashSet<CellId> = sheet.out_cells().collect();
    let relevant: HashSet<CellId> = outputs
        .iter()
        .flat_map(|&id| sheet.contributing_cells(id))
        .chain(sheet.requirement_relevant_cells())
        .chain(
            sheet
                .conditionals()
                .filter_map(|id| sheet.conditional_match_cells(id))
                .flatten()
                .copied(),
        )
        .collect();
    let invalid: HashSet<CellId> = sheet
        .filter_violation_cells()
        .into_iter()
        .chain(sheet.filter_violated_cells())
        .collect();
    built
        .keys
        .iter()
        .enumerate()
        .map(|(index, (key, &id))| {
            let meta = built.labels.cells.get(&id).ok_or_else(|| {
                HostRejection::Binding(BindingError::UnsupportedCodec(key.clone()))
            })?;
            let mut issues = Vec::new();
            let text = if meta.codec_supported {
                (meta.capture)(sheet).map_err(HostRejection::Input)?
            } else {
                (meta.display)(sheet)
            };
            if let Some(violation) = sheet.filter_violation(id) {
                issues.push(ValidationIssue {
                    key: key.clone(),
                    requirement: None,
                    labels: Vec::new(),
                    message: format!("filter violation: {violation:?}"),
                });
            }
            let forced = sheet.is_forced(id);
            let relevant = outputs.is_empty() || relevant.contains(&id);
            let range = meta
                .exact_range
                .as_ref()
                .map(|range| range(sheet))
                .transpose()
                .map_err(HostRejection::Input)?;
            let flags = CellFlags {
                disabled: forced || !relevant,
                readonly: outputs.contains(&id),
                relevant,
                invalid: invalid.contains(&id) || !issues.is_empty(),
                forced,
            };
            let is_source = sheet.is_source(id);
            if let Some(view) = previous.get(index).filter(|view| {
                view.key == *key
                    && view.text == text
                    && view.is_source == is_source
                    && view.flags == flags
                    && view.number_kind == meta.number_kind
                    && view.is_bool == meta.is_bool
                    && view.codec_supported == meta.codec_supported
                    && view.range == range
                    && view.issues == issues
            }) {
                return Ok(Rc::clone(view));
            }
            Ok(Rc::new(CellView {
                key: key.clone(),
                text,
                is_source,
                flags,
                number_kind: meta.number_kind,
                is_bool: meta.is_bool,
                codec_supported: meta.codec_supported,
                range,
                issues,
            }))
        })
        .collect()
}
