//! Controlled numeric controls without model ownership.
//!
//! Pass canonical strings and handle aspirations in `on_value`; the host's
//! validation remains authoritative. Wrap controls in [`crate::components::Theme`].
use crate::components::numeric_events::{SourceChannel, SourceFrame};
use crate::components::{
    button::{Button, ButtonVariant},
    input::Input,
    label::Label,
    number::*,
    slider::Slider,
};
use crate::input::{InputError, NumberKind, NumericRange};
use dioxus::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

include!(concat!(env!("OUT_DIR"), "/numeric_editor-styles.rs"));

thread_local! {
    /// Allocates distinct increasing identities across roots on the UI thread.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

/// Returns a fresh instance identity without wrapping.
fn next_generation() -> u64 {
    GENERATION.with(|counter| {
        let next = counter
            .get()
            .checked_add(1)
            .expect("numeric instance identities exhausted");
        counter.set(next);
        next
    })
}

/// Supplies controlled canonical values and model-independent numeric callbacks.
///
/// `range` guides the slider and limits nudges; the host validates typed aspirations.
/// Required blanks remain local; optional blanks submit `None`. Locale and
/// transport failures reach `on_error`. Programmatic updates never call `on_value`.
///
/// # Examples
/// ```
/// use adam_web_ui::{components::NumericEditor, input::NumberKind};
/// use dioxus::prelude::*;
/// fn app() -> Element {
///     let mut value = use_signal(|| Some("12".to_string()));
///     let range = use_signal(|| None);
///     let flags = use_signal(|| false);
///     rsx! { NumericEditor {
///         id: "amount", label: "Amount", value, range, kind: NumberKind::Signed(32),
///         step: "1", disabled: flags, readonly: flags, invalid: flags,
///         locale: None, optional: false,
///         on_value: move |v| value.set(v), on_error: |_| {},
///     } }
/// }
/// ```
#[derive(Props, Clone, PartialEq)]
pub struct NumericEditorProps {
    /// Names the native input and its help/error associations.
    pub id: String,
    /// Supplies the accessible visible name.
    pub label: String,
    /// Contains the effective canonical model value, not the draft.
    pub value: ReadSignal<Option<String>>,
    /// Preserves the concrete primitive width.
    pub kind: NumberKind,
    /// Supplies inclusive slider and nudge bounds without clamping typed aspirations.
    pub range: ReadSignal<Option<NumericRange>>,
    /// Supplies a positive canonical step.
    pub step: String,
    /// Disables every editing affordance.
    pub disabled: ReadSignal<bool>,
    /// Prevents edits while preserving readable content.
    pub readonly: ReadSignal<bool>,
    /// Exposes model validation independently of lexical input errors.
    pub invalid: ReadSignal<bool>,
    /// Selects an Intl locale; absence requests the host default.
    pub locale: Option<String>,
    /// Accepts a cleared property as `None` when true.
    pub optional: bool,
    /// Receives only user aspirations, never programmatic synchronization.
    pub on_value: EventHandler<Option<String>>,
    /// Receives explicit lexical, type, support, and locale errors.
    pub on_error: EventHandler<InputError>,
}

/// Renders an exact, localized controlled numeric editor.
///
/// Focus retains the originating locale and equivalent spelling. Blur restores
/// the latest effective value without submitting. Blur clears discarded lexical/range
/// errors, but retains unresolved locale, support, and event-source failures.
/// Replacing `id` or `kind` retires
/// the old instance; use a different key when replacing the underlying model.
///
/// Native capture supports selected/collapsed composition spans and expansions
/// exposed by synchronous `beforeinput` selection. A native mutation outside
/// that span without authoritative range metadata reports `InputError::EventSource`
/// without guessing a write (range generalization: issue #280). Native composition
/// terminal echoes use composition input metadata, not text equality alone.
///
/// - Complexity: O(value + draft + locale bytes) per transition.
///
/// # Examples
/// ```
/// use adam_web_ui::{components::NumericEditor, input::NumberKind};
/// use dioxus::prelude::*;
/// fn app() -> Element {
///     let mut value = use_signal(|| Some("12".to_string()));
///     let range = use_signal(|| None);
///     let flags = use_signal(|| false);
///     rsx! { NumericEditor {
///         id: "amount", label: "Amount", value, range, kind: NumberKind::Signed(32),
///         step: "1", disabled: flags, readonly: flags, invalid: flags,
///         locale: None, optional: false,
///         on_value: move |v| value.set(v), on_error: |_| {},
///     } }
/// }
/// ```
#[component]
pub fn NumericEditor(props: NumericEditorProps) -> Element {
    rsx! {
        EditorInstance { key: "{props.id}:{props.kind:?}", ..props }
    }
}

/// Owns the reducer, locale request epoch, and optional-clear composition state.
struct EditorState {
    draft: NumberDraft,
    generation: u64,
    epoch: u64,
    locale: Option<NumberLocale>,
    model: Option<(bool, bool)>,
    composing: bool,
    retired: bool,
}

impl EditorState {
    /// Creates a uniquely identified uninitialized editor.
    fn new() -> Self {
        Self {
            draft: NumberDraft::default(),
            generation: next_generation(),
            epoch: 0,
            locale: None,
            model: None,
            composing: false,
            retired: false,
        }
    }

    /// Accepts asynchronous work only for the current live model/edit and locale.
    fn accepts(&self, generation: u64, revision: u64, epoch: u64) -> bool {
        !self.retired
            && self.epoch == epoch
            && self.draft.generation() == generation
            && self.draft.revision() == revision
    }

    /// Permanently retires this mounted instance and its delayed work.
    ///
    /// - Complexity: O(latest display bytes) time and space.
    fn retire(&mut self) {
        self.retired = true;
        self.draft.retire();
    }

    /// Retires a failed locale session before allowing a future resolution to recover.
    ///
    /// - Complexity: O(latest display bytes) time and space.
    fn lose_locale(&mut self) {
        self.draft.retire();
        self.draft = NumberDraft::default();
        self.generation = next_generation();
        self.locale = None;
        self.model = None;
        self.composing = false;
    }

    /// Synchronizes changed model state and reports whether it discards a forced draft.
    ///
    /// - Complexity: O(canonical + locale bytes) time and space.
    fn sync(&mut self, canonical: &str, disabled: bool, readonly: bool, kind: NumberKind) -> bool {
        if self.retired
            || self.model.as_ref().is_some_and(|(d, r)| {
                self.draft.canonical() == canonical && *d == disabled && *r == readonly
            })
        {
            return false;
        }
        let Some(locale) = &self.locale else {
            return false;
        };
        self.draft.reduce(
            DraftEvent::ModelUpdate {
                canonical: canonical.to_string(),
                disabled,
                readonly,
                generation: self.generation,
            },
            locale,
            kind,
        );
        self.model = Some((disabled, readonly));
        if disabled || readonly {
            self.composing = false;
        }
        disabled || readonly
    }

    /// Applies a stamped native transition and derives the optional-clear callback.
    ///
    /// - Complexity: O(draft + locale bytes) time and space.
    fn event(&mut self, event: DraftEvent, kind: NumberKind, optional: bool) -> UiEffect {
        if self.retired {
            return UiEffect::None;
        }
        let Some(locale) = &self.locale else {
            return UiEffect::None;
        };
        let blocked = self.model.as_ref().is_none_or(|(d, r)| *d || *r);
        let clear = match &event {
            DraftEvent::Input(text) => optional && text.is_empty() && !self.composing,
            DraftEvent::CompositionStart => {
                self.composing = true;
                false
            }
            DraftEvent::CompositionEnd(text) => {
                let was_composing = std::mem::take(&mut self.composing);
                optional && text.is_empty() && was_composing
            }
            DraftEvent::Blur | DraftEvent::CompositionCancel => {
                self.composing = false;
                false
            }
            _ => false,
        };
        let effect =
            self.draft
                .reduce_stamped(self.generation, self.draft.revision(), event, locale, kind);
        if clear && !blocked {
            return UiEffect::Submit(None);
        }
        match effect {
            DraftEffect::Submit(value) => UiEffect::Submit(Some(value)),
            DraftEffect::Report(error) => UiEffect::Report(error),
            DraftEffect::None => UiEffect::None,
        }
    }
}

/// Separates reducer decisions from framework callback delivery.
#[derive(Debug, PartialEq, Eq)]
enum UiEffect {
    None,
    Submit(Option<String>),
    Report(InputError),
}

/// Updates the native value only when its text actually changes.
///
/// - Complexity: O(text bytes) time and space.
fn publish(state: &EditorState, mut visible: Signal<String>) {
    if visible.peek().as_str() != state.draft.text() {
        visible.set(state.draft.text().to_string());
    }
}

/// Delivers one transition after releasing reducer borrows.
fn deliver(
    effect: UiEffect,
    mut error: Signal<Option<InputError>>,
    on_value: EventHandler<Option<String>>,
    on_error: EventHandler<InputError>,
) {
    match effect {
        UiEffect::None => {}
        UiEffect::Submit(value) => {
            error.set(None);
            on_value.call(value);
        }
        UiEffect::Report(failure) => {
            error.set(Some(failure.clone()));
            on_error.call(failure);
        }
    }
}

/// Receives the explicit locale transport envelope, including eval execution failures.
///
/// # Errors
/// Returns `Locale` for any eval/transport failure.
async fn eval_reply(script: &str) -> Result<String, InputError> {
    // Web evaluators cannot serialize an implicit undefined completion.
    let mut eval = document::eval(&format!("{script}\nreturn null;"));
    let reply: serde_json::Value = eval
        .recv()
        .await
        .map_err(|error| InputError::Locale(error.to_string()))?;
    eval.join::<serde_json::Value>()
        .await
        .map_err(|error| InputError::Locale(error.to_string()))?;
    serde_json::to_string(&reply).map_err(|error| InputError::Locale(error.to_string()))
}

/// Formats the latest effective model with Rust-owned generation/revision/locale guards.
///
/// - Complexity: O(canonical + locale bytes) time and space, plus Intl formatting.
fn format_model(
    state: Rc<RefCell<EditorState>>,
    visible: Signal<String>,
    mut error: Signal<Option<InputError>>,
    kind: NumberKind,
    on_error: EventHandler<InputError>,
) {
    let (canonical, locale, generation, revision, epoch) = {
        let state = state.borrow();
        let Some(locale) = &state.locale else { return };
        let canonical = state.draft.canonical();
        if canonical.is_empty()
            || matches!(canonical, "NaN" | "inf" | "-inf" | "Infinity" | "-Infinity")
        {
            return;
        }
        (
            canonical.to_string(),
            state.draft.display_locale(locale).resolved.clone(),
            state.generation,
            state.draft.revision(),
            state.epoch,
        )
    };
    spawn(async move {
        let result = async {
            let script = format_number_script(&canonical, kind, &locale, &serde_json::json!({}))?;
            decode_format_reply(&eval_reply(&script).await?, generation, revision)
        }
        .await;
        let effect = {
            let mut state = state.borrow_mut();
            if !state.accepts(generation, revision, epoch) {
                return;
            }
            match result {
                Ok(event) => {
                    let EditorState { draft, locale, .. } = &mut *state;
                    if let Some(locale) = locale.as_ref() {
                        draft.reduce_stamped(generation, revision, event, locale, kind);
                        publish(&state, visible);
                    }
                    None
                }
                Err(failure) => Some(failure),
            }
        };
        if let Some(failure) = effect {
            error.set(Some(failure.clone()));
            on_error.call(failure);
        }
    });
}

/// Owns one mounted numeric draft and its scope-cancelled locale tasks.
///
/// - Complexity: O(value + draft + locale bytes) per transition.
#[component]
fn EditorInstance(props: NumericEditorProps) -> Element {
    crate::components::styles::use_control_styles();
    let (value, disabled, readonly, kind, optional, on_value, on_error) = (
        props.value,
        props.disabled,
        props.readonly,
        props.kind,
        props.optional,
        props.on_value,
        props.on_error,
    );
    let state = use_hook(|| Rc::new(RefCell::new(EditorState::new())));
    let mut visible = use_signal(|| props.value.peek().clone().unwrap_or_default());
    let mut error = use_signal(|| None::<InputError>);
    let mut ready = use_signal(|| false);
    let mut source_failed = use_signal(|| false);
    let source_token = use_hook(|| state.borrow().generation);
    let source = use_hook(|| Rc::new(RefCell::new(None::<SourceChannel>)));
    let closing = source.clone();
    let dropped = state.clone();
    use_drop(move || {
        dropped.borrow_mut().retire();
        closing.borrow_mut().take();
    });
    let failed_locale_state = state.clone();
    let failed_locale_source = source.clone();
    let locale_failure = use_callback(move |failure: InputError| {
        failed_locale_state.borrow_mut().lose_locale();
        visible.set(value.peek().clone().unwrap_or_default());
        ready.set(false);
        let reset_error = failed_locale_source
            .borrow_mut()
            .as_mut()
            .and_then(|source| {
                source
                    .reset(value.peek().as_deref().unwrap_or_default(), true, false)
                    .err()
            });
        deliver(UiEffect::Report(failure), error, on_value, on_error);
        if let Some(message) = reset_error {
            deliver(
                UiEffect::Report(InputError::EventSource(message)),
                error,
                on_value,
                on_error,
            );
        }
    });

    let locale = props.locale.clone();
    let locale_source = source.clone();
    let resolving = state.clone();
    use_effect(use_reactive!(|locale| {
        let state = resolving.clone();
        let locale_source = locale_source.clone();
        let epoch = {
            let mut state = state.borrow_mut();
            state.epoch = state.epoch.checked_add(1).expect("locale epochs exhausted");
            state.epoch
        };
        spawn(async move {
            let result = async {
                let script = number_locale_script(locale.as_deref())?;
                decode_locale_reply(&eval_reply(&script).await?)
            }
            .await;
            let failure = {
                let mut state = state.borrow_mut();
                if state.retired || state.epoch != epoch {
                    return;
                }
                match result {
                    Ok(locale) => {
                        state.locale = Some(locale);
                        state.sync(
                            value.peek().as_deref().unwrap_or_default(),
                            *disabled.peek(),
                            *readonly.peek(),
                            kind,
                        );
                        publish(&state, visible);
                        ready.set(true);
                        None
                    }
                    Err(failure) => {
                        state.lose_locale();
                        visible.set(value.peek().clone().unwrap_or_default());
                        if let Some(source) = locale_source.borrow_mut().as_mut()
                            && let Err(message) = source.reset(
                                value.peek().as_deref().unwrap_or_default(),
                                true,
                                false,
                            )
                        {
                            on_error.call(InputError::EventSource(message));
                        }
                        Some(failure)
                    }
                }
            };
            if let Some(failure) = failure {
                ready.set(false);
                error.set(Some(failure.clone()));
                on_error.call(failure);
            } else {
                if matches!(*error.peek(), Some(InputError::Locale(_))) {
                    error.set(None);
                }
                format_model(state, visible, error, kind, locale_failure);
            }
        });
    }));

    let updating = state.clone();
    let resetting = source.clone();
    let source_flags = use_hook(|| Rc::new(RefCell::new(None)));
    use_effect(move || {
        let canonical = value.read();
        let disabled = disabled();
        let readonly = readonly();
        {
            let mut state = updating.borrow_mut();
            let discarded = state.sync(
                canonical.as_deref().unwrap_or_default(),
                disabled,
                readonly,
                kind,
            );
            if discard_error(discarded, error.peek().as_ref()) {
                error.set(None);
            }
            if state.locale.is_some() {
                publish(&state, visible);
            } else {
                visible.set(canonical.clone().unwrap_or_default());
            }
        }
        let flags = (disabled, readonly, ready());
        if source_flags.borrow().as_ref() != Some(&flags) {
            *source_flags.borrow_mut() = Some(flags);
            if let Some(source) = resetting.borrow_mut().as_mut()
                && let Err(message) = source.reset(&visible.peek(), disabled || !flags.2, readonly)
            {
                let failure = InputError::EventSource(message);
                error.set(Some(failure.clone()));
                on_error.call(failure);
            }
        }
        format_model(updating.clone(), visible, error, kind, locale_failure);
    });

    let event_state = state.clone();
    let native = use_callback(move |event: DraftEvent| {
        if *disabled.peek() || *readonly.peek() || !*ready.peek() {
            return;
        }
        let blur = matches!(event, DraftEvent::Blur);
        let effect = {
            let mut state = event_state.borrow_mut();
            let effect = state.event(event, kind, optional);
            publish(&state, visible);
            effect
        };
        deliver(effect, error, on_value, on_error);
        if blur {
            if discard_error(true, error.peek().as_ref()) {
                error.set(None);
            }
            format_model(event_state.clone(), visible, error, kind, locale_failure);
        }
    });
    let step_state = state.clone();
    let step = props.step.clone();
    let range = props.range;
    let stepping = use_callback(move |increase: bool| {
        if !*ready.peek() || *source_failed.peek() {
            return;
        }
        let mut state = step_state.borrow_mut();
        state.sync(
            value.peek().as_deref().unwrap_or_default(),
            *disabled.peek(),
            *readonly.peek(),
            kind,
        );
        let effect = step_effect(
            &state,
            kind,
            &step,
            increase,
            *disabled.peek(),
            *readonly.peek(),
            range.peek().as_ref(),
        );
        drop(state);
        deliver(effect, error, on_value, on_error);
    });
    let receiving = source.clone();
    let source_state = state.clone();
    let receive = use_callback(move |result: Result<SourceFrame, String>| {
        let frame = match result {
            Ok(frame) if frame.action != "fatal" => frame,
            result => {
                let message = match result {
                    Ok(frame) => frame.message,
                    Err(message) => message,
                };
                source_failed.set(true);
                receiving.borrow_mut().take();
                deliver(
                    UiEffect::Report(InputError::EventSource(message)),
                    error,
                    on_value,
                    on_error,
                );
                return;
            }
        };
        let accepted = receiving
            .borrow_mut()
            .as_mut()
            .is_some_and(|source| source.accept(&frame, source_token));
        if !accepted {
            return;
        }
        match frame.action.as_str() {
            "error" => deliver(
                UiEffect::Report(InputError::EventSource(frame.message)),
                error,
                on_value,
                on_error,
            ),
            "ready" => {}
            "focus" => native.call(DraftEvent::Focus),
            "blur" => native.call(DraftEvent::Blur),
            "start" => native.call(DraftEvent::CompositionStart),
            "draft" | "input" => native.call(DraftEvent::Input(frame.value)),
            "end" => native.call(DraftEvent::CompositionEnd(frame.value)),
            "cancel" => native.call(DraftEvent::CompositionCancel),
            "up" | "down" if !source_state.borrow().composing => {
                stepping.call(frame.action == "up")
            }
            "up" | "down" => {}
            _ => deliver(
                UiEffect::Report(InputError::EventSource(
                    "Unknown numeric source action".into(),
                )),
                error,
                on_value,
                on_error,
            ),
        }
        // Retirement must restore native text even when the reactive display is unchanged.
        if matches!(frame.action.as_str(), "ready" | "blur" | "cancel" | "error")
            && let Some(source) = receiving.borrow().as_ref()
            && let Err(message) = source.display(&visible.peek())
        {
            deliver(
                UiEffect::Report(InputError::EventSource(message)),
                error,
                on_value,
                on_error,
            );
        }
    });
    let starting = source.clone();
    use_effect(move || {
        if starting.borrow().is_none() && !*source_failed.peek() {
            *starting.borrow_mut() = Some(SourceChannel::start(source_token, receive));
        }
    });
    let displaying = source.clone();
    use_effect(move || {
        let text = visible.read();
        if let Some(source) = displaying.borrow().as_ref()
            && let Err(message) = source.display(&text)
        {
            deliver(
                UiEffect::Report(InputError::EventSource(message)),
                error,
                on_value,
                on_error,
            );
        }
    });
    let error_text = error.read().as_ref().map(error_message);
    let invalid = (props.invalid)() || error_text.is_some();
    let blocked = (props.disabled)() || (props.readonly)() || !ready() || source_failed();
    let current = props.value.read();
    let range = props.range.read();
    let at_bound = |increase| {
        current.as_deref().is_some_and(|current| {
            matches!(
                bounded_step(current, kind, &props.step, increase, range.as_ref()),
                Ok(None)
            )
        })
    };
    let up_blocked = blocked || at_bound(true);
    let down_blocked = blocked || at_bound(false);
    rsx! {
        div { class: Styles::editor,
            Label { html_for: props.id.clone(), "{props.label}" }
            div { class: Styles::controls,
                Input {
                    id: props.id.clone(), r#type: "text", inputmode: "decimal",
                    // The native source owns text; volatile value attributes overwrite local drafts.
                    disabled: (props.disabled)() || !ready() || source_failed(), readonly: (props.readonly)(),
                    "data-adam-number-source": "{source_token}",
                    "aria-invalid": "{invalid}",
                    "aria-describedby": "{props.id}-help {props.id}-error",
                }
                div { class: Styles::stepper,
                    Button {
                        r#type: "button", variant: ButtonVariant::Ghost, disabled: up_blocked,
                        "aria-label": "Increase {props.label}", onclick: move |_| stepping.call(true),
                        "+"
                    }
                    Button {
                        r#type: "button", variant: ButtonVariant::Ghost, disabled: down_blocked,
                        "aria-label": "Decrease {props.label}", onclick: move |_| stepping.call(false),
                        "−"
                    }
                }
            }
            small { id: "{props.id}-help", class: Styles::visually_hidden,
                "Enter a number. Arrow Up and Arrow Down move to the next multiple of {props.step}."
            }
            small { id: "{props.id}-error", role: "status",
                if let Some(message) = error_text { "{message}" }
                else if (props.invalid)() { "The model rejected this value." }
            }
        }
    }
}

/// Computes checked stepping without submitting nonfinite or blocked values.
///
/// The inclusive range blocks nudges, while model filters validate submissions.
/// - Complexity: O(value + step + range bytes) time and space.
fn step_effect(
    state: &EditorState,
    kind: NumberKind,
    step: &str,
    increase: bool,
    disabled: bool,
    readonly: bool,
    range: Option<&NumericRange>,
) -> UiEffect {
    if state.retired || disabled || readonly || state.composing {
        return UiEffect::None;
    }
    if state.locale.is_none() {
        return UiEffect::None;
    }
    match bounded_step(state.draft.canonical(), kind, step, increase, range) {
        Ok(Some(value)) => UiEffect::Submit(Some(value)),
        Ok(None) => UiEffect::None,
        Err(failure) => UiEffect::Report(failure),
    }
}

/// Computes the next step if it lies within the inclusive range.
///
/// - Complexity: O(value + step + range bytes) time and space.
///
/// # Errors
/// Returns numeric input errors for invalid steps, transitions, or bounds.
fn bounded_step(
    current: &str,
    kind: NumberKind,
    step: &str,
    increase: bool,
    range: Option<&NumericRange>,
) -> Result<Option<String>, InputError> {
    let Some(range) = range else {
        return step_number(current, kind, step, increase).map(Some);
    };
    validate_canonical_number(&range.min, kind)?;
    validate_canonical_number(&range.max, kind)?;
    let stepped = step_number(current, kind, step, increase);
    if matches!(stepped, Err(InputError::TypeRange { .. })) {
        validate_canonical_number(current, kind)?;
        step_number("0", kind, step, true)?;
    }
    let text = stepped.as_deref().unwrap_or(current);
    macro_rules! compare {
        ($ty:ty) => {{
            let parse = |text: &str| -> Result<$ty, InputError> {
                text.parse::<$ty>().map_err(|_| InputError::TypeRange {
                    kind,
                    value: text.into(),
                })
            };
            let min = parse(&range.min)?;
            let max = parse(&range.max)?;
            if min > max {
                return Err(InputError::Lexical(
                    "numeric range minimum exceeds maximum".into(),
                ));
            }
            let value = parse(text)?;
            (
                value >= min && value <= max,
                if increase { value >= max } else { value <= min },
            )
        }};
    }
    let (inside, at_limit) = match kind {
        NumberKind::Signed(_) => compare!(i128),
        NumberKind::Unsigned(_) => compare!(u128),
        NumberKind::F32 => compare!(f32),
        NumberKind::F64 => compare!(f64),
    };
    match stepped {
        Ok(candidate) => Ok(inside.then_some(candidate)),
        Err(InputError::TypeRange { .. })
            if matches!(kind, NumberKind::Signed(_) | NumberKind::Unsigned(_)) || at_limit =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// Returns a readable input diagnostic without hiding its category.
///
/// - Complexity: O(diagnostic bytes) time and space.
fn error_message(error: &InputError) -> String {
    match error {
        InputError::Lexical(value) => format!("Invalid number: {value}"),
        InputError::TypeRange { kind, value } => format!("{value} is outside {kind:?}."),
        InputError::UnsupportedEditor(message) => message.clone(),
        InputError::Locale(message) => format!("Number locale error: {message}"),
        InputError::EventSource(message) => format!("Number event source error: {message}"),
    }
}

/// Clears only errors belonging to a discarded draft, retaining locale/support diagnostics.
fn discard_error(discarded: bool, error: Option<&InputError>) -> bool {
    discarded
        && matches!(
            error,
            Some(InputError::Lexical(_) | InputError::TypeRange { .. })
        )
}

/// Contains a finite, representable official slider domain and controlled handle.
#[derive(Clone, Copy, PartialEq)]
struct SliderDomain {
    value: f64,
    min: f64,
    max: f64,
    step: f64,
}

/// Selects only float or at-most-32-bit integer domains that the official slider represents.
///
/// Out-of-range and nonfinite model values keep the exact editor, never a clamped handle.
/// - Complexity: O(value + range + step bytes) time and O(1) space.
fn slider_domain(
    kind: NumberKind,
    value: Option<&str>,
    range: Option<&NumericRange>,
    step: &str,
) -> Option<SliderDomain> {
    if !matches!(
        kind,
        NumberKind::F32
            | NumberKind::F64
            | NumberKind::Signed(8 | 16 | 32)
            | NumberKind::Unsigned(8 | 16 | 32)
    ) {
        return None;
    }
    let range = range?;
    let domain = SliderDomain {
        value: value?.parse().ok()?,
        min: range.min.parse().ok()?,
        max: range.max.parse().ok()?,
        step: step.parse().ok()?,
    };
    (domain.value.is_finite()
        && domain.min.is_finite()
        && domain.max.is_finite()
        && (domain.max - domain.min).is_finite()
        && domain.step.is_finite()
        && domain.step > 0.0
        && domain.min < domain.max
        && domain.value >= domain.min
        && domain.value <= domain.max)
        .then_some(domain)
}

/// Validates an official slider aspiration without clamping or admitting non-numbers.
///
/// - Complexity: O(canonical bytes) time and space.
fn slider_effect(value: f64, kind: NumberKind, blocked: bool) -> UiEffect {
    if blocked {
        return UiEffect::None;
    }
    let canonical = value.to_string();
    if !value.is_finite() {
        return UiEffect::Report(InputError::Lexical(canonical));
    }
    let valid = match kind {
        NumberKind::F64 => true,
        NumberKind::F32 => canonical
            .parse::<f32>()
            .is_ok_and(|value| value.is_finite()),
        NumberKind::Signed(8 | 16 | 32) | NumberKind::Unsigned(8 | 16 | 32) => {
            kind.range().is_ok_and(|range| {
                value.fract() == 0.0
                    && value >= range.min.parse::<f64>().unwrap()
                    && value <= range.max.parse::<f64>().unwrap()
            })
        }
        _ => false,
    };
    if valid {
        UiEffect::Submit(Some(canonical))
    } else {
        UiEffect::Report(InputError::TypeRange {
            kind,
            value: canonical,
        })
    }
}

/// Composes the official slider with the exact editor and shared canonical callbacks.
///
/// Wide integers, absent bounds and values outside the handle domain retain only
/// the editor. Bounds never rewrite the visible model value.
///
/// - Complexity: O(value + draft + range + locale bytes) per transition.
///
/// # Examples
/// ```
/// use adam_web_ui::{components::NumericSlider, input::{NumberKind, NumericRange}};
/// use dioxus::prelude::*;
/// fn app() -> Element {
///     let mut value = use_signal(|| Some("12".to_string()));
///     let range = use_signal(|| Some(NumericRange { min: "0".into(), max: "100".into() }));
///     let flags = use_signal(|| false);
///     rsx! { NumericSlider {
///         id: "amount", label: "Amount", value, range, kind: NumberKind::Signed(32),
///         step: "1", disabled: flags, readonly: flags, invalid: flags,
///         locale: None, optional: false,
///         on_value: move |v| value.set(v), on_error: |_| {},
///     } }
/// }
/// ```
#[component]
pub fn NumericSlider(props: NumericEditorProps) -> Element {
    let domain = {
        let value = props.value.read();
        let range = props.range.read();
        slider_domain(props.kind, value.as_deref(), range.as_ref(), &props.step)
    };
    let (kind, disabled, readonly, on_value, on_error) = (
        props.kind,
        props.disabled,
        props.readonly,
        props.on_value,
        props.on_error,
    );
    rsx! {
        div { class: Styles::slider,
            if let Some(domain) = domain {
                Slider {
                    value: Some(domain.value), min: domain.min, max: domain.max, step: domain.step,
                    disabled: (props.disabled)() || (props.readonly)(),
                    label: Some(props.label.clone()),
                    on_value_change: move |value| {
                        match slider_effect(value, kind,
                            *disabled.peek() || *readonly.peek()) {
                            UiEffect::Submit(value) => on_value.call(value),
                            UiEffect::Report(error) => on_error.call(error),
                            UiEffect::None => {}
                        }
                    },
                }
            }
            NumericEditor { ..props }
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
