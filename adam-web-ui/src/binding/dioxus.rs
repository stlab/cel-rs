//! Binds scope-owned reactive handles to the single reviewed transaction owner.
//!
//! Initialize with [`use_sheet`], pass [`SheetHandle::cell`] to bound controls,
//! and match [`SheetEvent::outcome`] at the host boundary. Only the host restores
//! rejected candidates. All access remains on the owning Dioxus event loop.
use super::*;
use crate::{BuildError, Labels};
use ::dioxus::prelude::*;
use adam_lang::ParsedSheet;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

thread_local! {
    /// Allocates event-loop-wide identities, including independent Dioxus roots.
    static INSTANCE: Cell<u64> = const { Cell::new(0) };
}

/// Returns a distinct identity without wrapping into a retired identity.
fn next_instance() -> u64 {
    INSTANCE.with(|counter| {
        let next = counter
            .get()
            .checked_add(1)
            .expect("sheet identities exhausted");
        counter.set(next);
        next
    })
}

/// Owns the sole mutable model and its host callback outside reactive publication.
struct HandleState {
    owner: RefCell<SheetOwner>,
    instance: Cell<u64>,
    on_outcome: EventHandler<SheetEvent>,
}

/// Shares a scope-owned model; copying a handle does not copy its model or journal.
///
/// Signals track accepted publications, while checked borrows reject reentrancy.
/// Handles make no `Send` or `Sync` promise and expire with their Dioxus scope.
///
/// # Examples
/// See [`use_sheet`] for initialization and a bound editor.
#[derive(Clone, Copy, PartialEq)]
pub struct SheetHandle {
    state: Signal<Rc<HandleState>>,
    revision: Signal<u64>,
}

/// Delivers a structured outcome after releasing the model borrow.
///
/// # Examples
/// ```
/// use adam_web_ui::binding::{SheetEvent, HostOutcome, BindingError};
/// fn outcome(event: SheetEvent) {
///     if matches!(event.outcome, HostOutcome::Rejected(_))
///         && matches!(event.sheet.inspect(|_, _| ()), Err(BindingError::Unavailable))
///     {
///         let _ = event.sheet.restore_accepted(); // The host chooses recovery.
///     }
/// }
/// ```
pub struct SheetEvent {
    /// Addresses the owner without circular callback initialization.
    pub sheet: SheetHandle,
    /// Preserves acceptance, recorder failure, validation and fatal categories.
    pub outcome: HostOutcome,
}

/// Initializes one owner per mounted hook, including cached construction failure.
///
/// Later factory/key prop changes do not replace the owner; call [`SheetHandle::replace`].
///
/// # Errors
/// Returns independently owned construction diagnostics without a partial model.
///
/// - Complexity: Factory and publication cost once; O(1) on subsequent renders.
///
/// # Examples
/// ```
/// use adam_web_ui::{binding::*, build_sheet};
/// use dioxus::prelude::*;
/// use std::rc::Rc;
/// fn app() -> Element {
///     let factory: ModelFactory = Rc::new(||
///         build_sheet("sheet s { cell x: i32 = 1; }", "s.adm2"));
///     match use_sheet(factory, "s/v1".into(), EventHandler::new(|_| {})) {
///         Ok(sheet) => rsx! { EditNumber { bind: sheet.cell("x") } },
///         Err(error) => rsx! { pre { "{error:?}" } },
///     }
/// }
/// ```
pub fn use_sheet(
    factory: ModelFactory,
    schema_key: String,
    on_outcome: EventHandler<SheetEvent>,
) -> Result<SheetHandle, Rc<BuildError>> {
    use_hook(move || SheetHandle::new(factory, schema_key, on_outcome).map_err(Rc::new))
}

impl SheetHandle {
    /// Constructs a scope-owned handle without installing a failed model.
    ///
    /// Call once during scope initialization, or prefer [`use_sheet`].
    ///
    /// # Errors
    /// Returns the factory's construction or initial publication error.
    ///
    /// - Complexity: Factory and publication cost.
    ///
    /// # Examples
    /// See [`use_sheet`]; `SheetHandle::new` uses the same arguments inside a hook.
    pub fn new(
        factory: ModelFactory,
        schema_key: String,
        on_outcome: EventHandler<SheetEvent>,
    ) -> Result<Self, BuildError> {
        Ok(Self::from_owner(
            SheetOwner::new(factory, schema_key)?,
            on_outcome,
        ))
    }

    /// Adopts an owner without rebuilding or duplicating its model.
    ///
    /// Call in a mounted Dioxus scope; this transfers its journal, recordings and
    /// availability state. A discarded or retired owner stays unavailable.
    ///
    /// # Examples
    /// See [`SheetOwner`] for construction; inside a hook call
    /// `SheetHandle::from_owner(owner, EventHandler::new(|_| {}))`.
    pub fn from_owner(owner: SheetOwner, on_outcome: EventHandler<SheetEvent>) -> Self {
        Self {
            state: Signal::new(Rc::new(HandleState {
                owner: RefCell::new(owner),
                instance: Cell::new(next_instance()),
                on_outcome,
            })),
            revision: Signal::new(0),
        }
    }

    /// Returns the shared state without subscribing callers to internal ownership.
    fn state(&self) -> Rc<HandleState> {
        self.state.peek().clone()
    }

    /// Publishes one transition without retaining a model borrow across notification.
    fn publish(&self) {
        let mut revision = self.revision;
        revision += 1;
    }

    /// Forwards an outcome without choosing host recovery policy.
    fn emit(&self, outcome: HostOutcome) {
        self.state().on_outcome.call(SheetEvent {
            sheet: *self,
            outcome,
        });
    }

    /// Returns the reactive instance identity used to key replacement controls.
    ///
    /// # Examples
    /// See [`Self::replace`]; a successful replacement changes `sheet.instance()`.
    pub fn instance(&self) -> u64 {
        let _ = self.revision.read();
        self.state().instance.get()
    }

    /// Creates a symbolic binding owned by this exact model installation.
    ///
    /// Lookup errors are returned by [`CellBinding::view`], not hidden here.
    ///
    /// - Complexity: O(name bytes).
    ///
    /// # Examples
    /// See [`use_sheet`]; `sheet.cell("x")` binds the named cell.
    pub fn cell(&self, name: &str) -> CellBinding {
        CellBinding {
            sheet: *self,
            instance: self.instance(),
            key: name.into(),
        }
    }

    /// Submits an ordered group and forwards every structured outcome to the host.
    ///
    /// Reentrant access rejects without invoking the transaction. Rejections that
    /// discard a candidate retire its bindings; recovery installs a new instance.
    ///
    /// - Complexity: Transaction/publication cost plus O(1) notification.
    ///
    /// # Examples
    /// See [`SheetOwner`] for `CellEdit` groups; call `sheet.apply(&edits, UpdateOrigin::User)`.
    pub fn apply(&self, edits: &[CellEdit], origin: UpdateOrigin) {
        let state = self.state();
        let outcome = match state.owner.try_borrow_mut() {
            Ok(mut owner) => {
                let outcome = owner.apply(edits, origin);
                if owner.inspect(|_, _| ()).is_err() {
                    state.instance.set(next_instance());
                }
                outcome
            }
            Err(_) => {
                HostOutcome::Rejected(HostRejection::Binding(BindingError::ReentrantMutation))
            }
        };
        self.publish();
        self.emit(outcome);
    }

    /// Restores accepted aspirations only at the host's explicit request.
    ///
    /// # Errors
    /// Returns typed reconstruction, fatal retirement, or reentrancy errors.
    ///
    /// - Complexity: Factory, journal replay, and publication cost.
    ///
    /// # Examples
    /// See [`SheetEvent`] for explicit host recovery.
    pub fn restore_accepted(&self) -> Result<(), BuildError> {
        let state = self.state();
        let result = {
            let mut owner = state.owner.try_borrow_mut().map_err(|_| build_busy())?;
            let result = owner.restore_accepted();
            state.instance.set(next_instance());
            result
        };
        self.publish();
        result
    }

    /// Borrows the single accepted journal, including after rejection or retirement.
    ///
    /// # Errors
    /// Returns `ReentrantMutation` without invoking `f` while a mutation owns it.
    /// Writes attempted inside `f` likewise reject without changing the journal.
    ///
    /// - Complexity: O(1) plus callback cost; does not copy history.
    ///
    /// # Examples
    /// See [`use_sheet`]; `sheet.with_accepted_state(|s| s.transactions.len())`
    /// returns the number of accepted groups.
    pub fn with_accepted_state<R>(
        &self,
        f: impl FnOnce(&RecoveryState) -> R,
    ) -> Result<R, BindingError> {
        let state = self.state();
        let owner = state
            .owner
            .try_borrow()
            .map_err(|_| BindingError::ReentrantMutation)?;
        Ok(f(owner.accepted_state()))
    }

    /// Tracks a read of an accepted live model without exposing mutable ownership.
    ///
    /// # Errors
    /// Returns unavailable, retired, or reentrant diagnostics without calling `f`.
    ///
    /// - Complexity: O(1) plus callback cost.
    ///
    /// # Examples
    /// See [`use_sheet`]; `sheet.inspect(|parsed, _| parsed.cell_names.len())`
    /// counts named cells, and can project application-specific graph data.
    pub fn inspect<R>(
        &self,
        f: impl FnOnce(&ParsedSheet, &Labels) -> R,
    ) -> Result<R, BindingError> {
        let _ = self.revision.read();
        let state = self.state();
        let owner = state
            .owner
            .try_borrow()
            .map_err(|_| BindingError::ReentrantMutation)?;
        owner.inspect(f)
    }

    /// Captures exact effective sources from the accepted live model.
    ///
    /// # Errors
    /// Returns capture capability, encoding, availability, or reentrancy failures.
    ///
    /// - Complexity: Source analysis and encoded bytes.
    ///
    /// # Examples
    /// See [`use_sheet`]; `sheet.capture(&["x"])` selects the named result.
    pub fn capture(&self, result_keys: &[&str]) -> Result<Recording, CaptureError> {
        let state = self.state();
        let owner = state.owner.try_borrow().map_err(|_| capture_busy())?;
        owner.capture(result_keys)
    }

    /// Enables capture for subsequent accepted user actions without cloning history.
    ///
    /// # Errors
    /// Returns capture validation, availability, or reentrancy errors.
    ///
    /// - Complexity: Capture cost plus selected-key bytes.
    ///
    /// # Examples
    /// See [`use_sheet`]; `sheet.start_recording(&["x"])` starts action capture.
    pub fn start_recording(&self, result_keys: &[&str]) -> Result<(), CaptureError> {
        let state = self.state();
        let mut owner = state.owner.try_borrow_mut().map_err(|_| capture_busy())?;
        owner.start_recording(result_keys)
    }

    /// Stops recording, reporting contention through the host outcome boundary.
    ///
    /// - Complexity: O(selected-key bytes) destruction.
    ///
    /// # Examples
    /// See [`Self::start_recording`]; call `sheet.stop_recording()` to stop capture.
    pub fn stop_recording(&self) {
        let state = self.state();
        let result = match state.owner.try_borrow_mut() {
            Ok(mut owner) => {
                owner.stop_recording();
                Ok(())
            }
            Err(_) => Err(BindingError::ReentrantMutation),
        };
        if let Err(error) = result {
            self.emit(HostOutcome::Rejected(HostRejection::Binding(error)));
        }
    }

    /// Returns the latest immutable recording without copying its values.
    ///
    /// Contention reports a structured host outcome and returns no recording.
    ///
    /// # Examples
    /// See [`Self::start_recording`]; before an action `sheet.last_recording().is_none()`.
    pub fn last_recording(&self) -> Option<Rc<Recording>> {
        let _ = self.revision.read();
        let state = self.state();
        let result = state
            .owner
            .try_borrow()
            .map(|owner| owner.recordings().last().cloned());
        match result {
            Ok(recording) => recording,
            Err(_) => {
                self.emit(HostOutcome::Rejected(HostRejection::Binding(
                    BindingError::ReentrantMutation,
                )));
                None
            }
        }
    }

    /// Installs a fully validated replacement and retires all prior bindings on success.
    ///
    /// # Errors
    /// Returns construction or reentrancy errors, preserving the prior owner.
    /// A fatal owner rejects replacement without invoking the factory.
    ///
    /// - Complexity: Factory/publication cost and old-model destruction.
    ///
    /// # Examples
    /// See [`use_sheet`]; `sheet.replace(factory, "s/v2".into())` replaces explicitly.
    pub fn replace(&self, factory: ModelFactory, schema_key: String) -> Result<(), BuildError> {
        let state = self.state();
        {
            let mut owner = state.owner.try_borrow_mut().map_err(|_| build_busy())?;
            replacement_allowed(&owner)
                .map_err(|error| BuildError::Recovery(Box::new(HostRejection::Binding(error))))?;
            *owner = SheetOwner::new(factory, schema_key)?;
            state.instance.set(next_instance());
        }
        self.publish();
        Ok(())
    }

    /// Adopts a prevalidated replacement and retires the preceding installation.
    ///
    /// This avoids rebuilding a candidate already checked by a host's load path.
    ///
    /// # Errors
    /// Returns candidate availability, fatal retirement, or reentrancy errors
    /// without replacing the current owner.
    ///
    /// - Complexity: Old model/history destruction; O(1) transfer of the new owner.
    ///
    /// # Examples
    /// See [`Self::from_owner`]; `sheet.install(owner)` installs a validated load.
    pub fn install(&self, candidate: SheetOwner) -> Result<(), BindingError> {
        candidate.inspect(|_, _| ())?;
        let state = self.state();
        {
            let mut owner = state
                .owner
                .try_borrow_mut()
                .map_err(|_| BindingError::ReentrantMutation)?;
            replacement_allowed(&owner)?;
            *owner = candidate;
            state.instance.set(next_instance());
        }
        self.publish();
        Ok(())
    }

    /// Installs a fresh successfully replayed owner without recording playback as a user edit.
    ///
    /// # Errors
    /// Returns compatibility, construction, playback or reentrancy errors;
    /// failed replay leaves the prior owner installed. A retired owner never
    /// invokes the factory.
    ///
    /// - Complexity: Factory, replay, publication and old-model destruction cost.
    ///
    /// # Examples
    /// See [`Self::capture`]; pass its recording to
    /// `sheet.playback(factory, "s/v1".into(), &recording)`.
    pub fn playback(
        &self,
        factory: ModelFactory,
        schema_key: String,
        recording: &Recording,
    ) -> Result<(), PlaybackError> {
        let state = self.state();
        {
            let mut owner = state
                .owner
                .try_borrow_mut()
                .map_err(|_| PlaybackError::Capture(capture_busy()))?;
            replacement_allowed(&owner)
                .map_err(|error| PlaybackError::Capture(CaptureError::Unavailable(error)))?;
            *owner = SheetOwner::playback(factory, schema_key, recording)?;
            state.instance.set(next_instance());
        }
        self.publish();
        Ok(())
    }

    /// Copies shared view references, never the accepted history.
    ///
    /// # Errors
    /// Returns an owner-state or reentrancy error without inspecting a failed model.
    ///
    /// - Complexity: O(named cells) time and space.
    ///
    /// # Examples
    /// See [`use_sheet`]; `sheet.views()` enumerates accepted published cells.
    pub fn views(&self) -> Result<Vec<Rc<CellView>>, BindingError> {
        let _ = self.revision.read();
        let state = self.state();
        let owner = state
            .owner
            .try_borrow()
            .map_err(|_| BindingError::ReentrantMutation)?;
        owner.inspect(|_, _| ())?;
        Ok(owner.views().to_vec())
    }
}

/// Addresses one cell in one installation, never a later same-named replacement.
///
/// # Examples
/// See [`use_sheet`] for creating a binding and passing it to a widget.
#[derive(Clone, PartialEq)]
pub struct CellBinding {
    sheet: SheetHandle,
    instance: u64,
    key: Rc<str>,
}

impl CellBinding {
    /// Returns the accepted published view after checked instance and name lookup.
    ///
    /// # Errors
    /// Returns unknown key, unavailable, retired-instance, or reentrancy errors.
    ///
    /// - Complexity: O(named cells + key bytes).
    ///
    /// # Examples
    /// See [`use_sheet`]; `sheet.cell("x").view()` reads its exact effective value.
    pub fn view(&self) -> Result<Rc<CellView>, BindingError> {
        if self.instance != self.sheet.instance() {
            return Err(BindingError::RetiredInstance);
        }
        let state = self.sheet.state();
        let owner = state
            .owner
            .try_borrow()
            .map_err(|_| BindingError::ReentrantMutation)?;
        owner.inspect(|_, _| ())?;
        owner
            .views()
            .iter()
            .find(|view| view.key == self.key.as_ref())
            .cloned()
            .ok_or_else(|| BindingError::UnknownKey(self.key.to_string()))
    }

    /// Submits canonical user input only to the originating instance.
    ///
    /// All outcomes, including stale events and conversion errors, reach the host.
    ///
    /// - Complexity: Transaction cost plus key and input bytes.
    ///
    /// # Examples
    /// See [`use_sheet`]; `sheet.cell("x").submit("2".into())` submits an aspiration.
    pub fn submit(&self, canonical: String) {
        if self.instance != self.sheet.instance() {
            self.report(HostRejection::Binding(BindingError::RetiredInstance));
            return;
        }
        self.sheet.apply(
            &[CellEdit {
                key: self.key.to_string(),
                canonical,
            }],
            UpdateOrigin::User,
        );
    }

    /// Reports widget input/configuration failures without mutating the model.
    pub(super) fn report(&self, rejection: HostRejection) {
        let rejection = if self.instance == self.sheet.instance() {
            rejection
        } else {
            HostRejection::Binding(BindingError::RetiredInstance)
        };
        self.sheet.emit(HostOutcome::Rejected(rejection));
    }

    /// Returns a stable installation-qualified key for component remounts.
    ///
    /// - Complexity: O(key bytes) time and space.
    pub(super) fn identity(&self) -> String {
        format!("{}:{}", self.instance, self.key)
    }

    /// Supplies the stable name when a widget omits its accessible label.
    ///
    /// - Complexity: O(key bytes) time and space when the label is empty.
    pub(super) fn label(&self, label: String) -> String {
        if label.is_empty() {
            self.key.to_string()
        } else {
            label
        }
    }
}

/// Wraps checked owner contention in the construction/recovery error channel.
fn build_busy() -> BuildError {
    BuildError::Recovery(Box::new(HostRejection::Binding(
        BindingError::ReentrantMutation,
    )))
}

/// Wraps checked owner contention in the recording error channel.
fn capture_busy() -> CaptureError {
    CaptureError::Unavailable(BindingError::ReentrantMutation)
}

/// Forbids factory calls and installation on a permanently retired owner.
fn replacement_allowed(owner: &SheetOwner) -> Result<(), BindingError> {
    match owner.inspect(|_, _| ()) {
        Err(BindingError::RetiredInstance) => Err(BindingError::RetiredInstance),
        _ => Ok(()),
    }
}
