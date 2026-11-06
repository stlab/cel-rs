//! Numeric draft transitions.
//!
//! Keep one [`NumberDraft`] per binding instance and retire it on unmount.
use super::{NormalizedDraft, NumberLocale, normalize_draft};
use crate::input::{InputError, NumberKind};

/// Carries synchronous native events or revision-tagged formatting replies.
///
/// `generation` identifies the binding instance, not a model value revision.
///
/// # Examples
/// ```
/// use adam_web_ui::components::number::DraftEvent;
/// let input = DraftEvent::Input("-".into());
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DraftEvent {
    /// Replaces the user draft.
    Input(String),
    /// Captures the originating locale.
    Focus,
    /// Restores latest model display without submitting.
    Blur,
    /// Suppresses submissions until composition ends.
    CompositionStart,
    /// Processes the final composed draft.
    CompositionEnd(String),
    /// Synchronizes model state; a newer generation retires the previous draft.
    ModelUpdate {
        /// Contains canonical model text, including explicit NaN/infinity text.
        canonical: String,
        /// Disables edits when forced or disconnected.
        disabled: bool,
        /// Prohibits edits without treating the cell as disconnected.
        readonly: bool,
        /// Identifies the binding instance.
        generation: u64,
    },
    /// Supplies a formatter reply for exactly one generation and revision.
    Formatted {
        /// Contains localized model display text.
        text: String,
        /// Identifies the binding instance.
        generation: u64,
        /// Identifies the edit/model revision being formatted.
        revision: u64,
    },
}

/// Describes the only externally observable side effect of a transition.
///
/// # Examples
/// ```
/// use adam_web_ui::components::number::DraftEffect;
/// assert_ne!(DraftEffect::None, DraftEffect::Submit("1".into()));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DraftEffect {
    /// Performs no model write or error report.
    None,
    /// Submits a complete, concretely representable value.
    Submit(String),
    /// Reports invalid input without a model write.
    Report(InputError),
}

/// Owns one binding instance's local draft and latest model display.
///
/// Hosts assign every replacement binding a unique, increasing generation,
/// including when constructing a fresh reducer. Revisions never wrap;
/// exhaustion retires the instance.
///
/// # Examples
/// ```
/// use adam_web_ui::components::number::NumberDraft;
/// let draft = NumberDraft::default();
/// assert_eq!(draft.text(), "");
/// ```
#[derive(Debug, Default)]
pub struct NumberDraft {
    /// Contains the visible user draft.
    text: String,
    /// Contains the latest canonical model value, independently of local edits.
    canonical: String,
    /// Contains the latest accepted model display.
    display: String,
    /// Retains the locale captured at focus until blur or replacement.
    origin: Option<NumberLocale>,
    /// Indicates an active focus session.
    focused: bool,
    /// Indicates an input-method composition in progress.
    composing: bool,
    /// Indicates incomplete or invalid local text.
    local: bool,
    /// Prohibits user input when disabled or readonly.
    blocked: bool,
    /// Makes retirement permanent.
    retired: bool,
    /// Identifies this binding instance.
    generation: u64,
    /// Distinguishes asynchronous work from subsequent edits or model changes.
    revision: u64,
    /// Suppresses the native input duplicate following compositionend.
    composed_input: bool,
    /// Retains the submitted value so acknowledgments preserve equivalent spelling.
    submitted: Option<String>,
    /// Allows localization of authoritative model text, never of an entered draft.
    format_visible: bool,
}

impl NumberDraft {
    /// Applies a synchronous event belonging to this binding instance.
    ///
    /// Queue asynchronous native events through [`Self::reduce_stamped`].
    /// Focus retains its locale; composition never submits intermediate text.
    /// Blur restores the latest display without submitting. Disabled/readonly
    /// updates discard local drafts. Formatter replies never replace focused
    /// user edits, and stale generations/revisions have no effect.
    ///
    /// - Complexity: O(event text + locale vocabulary bytes) time and space.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::{components::number::*, input::NumberKind};
    /// fn edit(locale: &NumberLocale) {
    ///     let mut draft = NumberDraft::default();
    ///     assert_eq!(draft.reduce(DraftEvent::Input("-".into()), locale,
    ///         NumberKind::F64), DraftEffect::None);
    /// }
    /// ```
    pub fn reduce(
        &mut self,
        event: DraftEvent,
        locale: &NumberLocale,
        kind: NumberKind,
    ) -> DraftEffect {
        if self.retired {
            return DraftEffect::None;
        }
        match event {
            DraftEvent::ModelUpdate {
                canonical,
                disabled,
                readonly,
                generation,
            } => {
                if generation < self.generation {
                    return DraftEffect::None;
                }
                if generation != self.generation {
                    self.focused = false;
                    self.origin = None;
                    self.local = false;
                    self.composing = false;
                    self.composed_input = false;
                    self.submitted = None;
                    self.generation = generation;
                }
                if !self.advance() {
                    return DraftEffect::None;
                }
                self.blocked = disabled || readonly;
                if self.blocked {
                    self.focused = false;
                    self.origin = None;
                    self.local = false;
                    self.composing = false;
                    self.composed_input = false;
                    self.submitted = None;
                }
                let keep = self.focused
                    && !self.blocked
                    && (self.local
                        || self.composing
                        || self.canonical == canonical
                        || self
                            .submitted
                            .as_ref()
                            .is_some_and(|value| equivalent(value, &canonical, kind)));
                self.display.clone_from(&canonical);
                self.canonical = canonical;
                if !keep {
                    self.text.clone_from(&self.display);
                    self.submitted = None;
                    self.format_visible = true;
                }
            }
            DraftEvent::Focus => {
                if !self.blocked && !self.focused {
                    self.focused = true;
                    self.origin = Some(locale.clone());
                }
            }
            DraftEvent::Blur => {
                if !self.advance() {
                    return DraftEffect::None;
                }
                self.focused = false;
                self.composing = false;
                self.local = false;
                self.origin = None;
                self.composed_input = false;
                self.submitted = None;
                self.text.clone_from(&self.display);
                self.format_visible = true;
            }
            DraftEvent::CompositionStart => {
                if !self.blocked {
                    if !self.advance() {
                        return DraftEffect::None;
                    }
                    self.composing = true;
                    self.composed_input = false;
                    self.format_visible = false;
                }
            }
            DraftEvent::CompositionEnd(text) => {
                if self.blocked || !self.composing {
                    return DraftEffect::None;
                }
                self.composing = false;
                self.composed_input = true;
                return self.input(text, locale, kind);
            }
            DraftEvent::Input(text) => {
                if self.blocked {
                    return DraftEffect::None;
                }
                if std::mem::take(&mut self.composed_input) && self.text == text {
                    return DraftEffect::None;
                }
                return self.input(text, locale, kind);
            }
            DraftEvent::Formatted {
                text,
                generation,
                revision,
            } => {
                if generation == self.generation
                    && revision == self.revision
                    && !matches!(
                        self.canonical.as_str(),
                        "NaN" | "inf" | "-inf" | "Infinity" | "-Infinity"
                    )
                {
                    self.display = text;
                    if !self.focused || self.format_visible {
                        self.text.clone_from(&self.display);
                    }
                    self.format_visible = false;
                }
            }
        }
        DraftEffect::None
    }

    /// Ignores delayed native events unless their instance and revision match.
    ///
    /// - Complexity: O(event text + locale vocabulary bytes) time and space.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::{components::number::*, input::NumberKind};
    /// fn delayed_edit(draft: &mut NumberDraft, locale: &NumberLocale) {
    ///     let generation = draft.generation();
    ///     let revision = draft.revision();
    ///     draft.reduce_stamped(generation, revision, DraftEvent::Input("1".into()),
    ///         locale, NumberKind::F64);
    /// }
    /// ```
    pub fn reduce_stamped(
        &mut self,
        generation: u64,
        revision: u64,
        event: DraftEvent,
        locale: &NumberLocale,
        kind: NumberKind,
    ) -> DraftEffect {
        if generation != self.generation || revision != self.revision {
            return DraftEffect::None;
        }
        self.reduce(event, locale, kind)
    }

    /// Returns the visible draft without allocation.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::components::number::NumberDraft;
    /// assert_eq!(NumberDraft::default().text(), "");
    /// ```
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the monotonically increasing edit/model revision.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::components::number::NumberDraft;
    /// assert_eq!(NumberDraft::default().revision(), 0);
    /// ```
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the current binding generation for delayed-event stamps.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::components::number::NumberDraft;
    /// assert_eq!(NumberDraft::default().generation(), 0);
    /// ```
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the latest model value independently of the visible draft.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::components::number::NumberDraft;
    /// assert_eq!(NumberDraft::default().canonical(), "");
    /// ```
    pub fn canonical(&self) -> &str {
        &self.canonical
    }

    /// Returns the focus session's parsing locale, or the current unfocused locale.
    ///
    /// Hosts use this same locale when formatting authoritative model updates so
    /// the displayed number remains valid input without reinterpreting a draft.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::components::number::{NumberDraft, NumberLocale};
    /// fn unfocused(current: &NumberLocale) {
    ///     let draft = NumberDraft::default();
    ///     assert_eq!(draft.display_locale(current), current);
    /// }
    /// ```
    pub fn display_locale<'a>(&'a self, current: &'a NumberLocale) -> &'a NumberLocale {
        self.origin.as_ref().unwrap_or(current)
    }

    /// Discards the local draft and permanently retires this instance.
    ///
    /// - Postcondition: No later event submits a value.
    /// - Complexity: O(latest display bytes) time and space.
    ///
    /// # Examples
    /// ```
    /// use adam_web_ui::components::number::NumberDraft;
    /// let mut draft = NumberDraft::default();
    /// draft.retire();
    /// ```
    pub fn retire(&mut self) {
        self.retired = true;
        self.focused = false;
        self.local = false;
        self.composing = false;
        self.origin = None;
        self.composed_input = false;
        self.submitted = None;
        self.format_visible = false;
        self.text.clone_from(&self.display);
    }

    /// Advances the event revision, retiring instead of wrapping on exhaustion.
    ///
    /// - Complexity: O(1), or O(latest display bytes) when retiring.
    fn advance(&mut self) -> bool {
        match self.revision.checked_add(1) {
            Some(revision) => {
                self.revision = revision;
                true
            }
            None => {
                self.retire();
                false
            }
        }
    }

    /// Retains raw input and emits at most one complete numeric submission.
    ///
    /// - Complexity: O(text + locale vocabulary bytes) time and space.
    fn input(&mut self, text: String, locale: &NumberLocale, kind: NumberKind) -> DraftEffect {
        if !self.advance() {
            return DraftEffect::None;
        }
        self.text = text;
        self.format_visible = false;
        self.local = true;
        self.submitted = None;
        if self.composing {
            return DraftEffect::None;
        }
        match normalize_draft(&self.text, self.origin.as_ref().unwrap_or(locale), kind) {
            Ok(NormalizedDraft::Incomplete) => DraftEffect::None,
            Ok(NormalizedDraft::Complete(canonical)) => {
                self.local = false;
                self.submitted = Some(canonical.clone());
                DraftEffect::Submit(canonical)
            }
            Err(error) => DraftEffect::Report(error),
        }
    }
}

/// Compares canonical primitive values without routing integers through floats.
///
/// - Complexity: O(left bytes + right bytes) time and O(1) additional space.
fn equivalent(left: &str, right: &str, kind: NumberKind) -> bool {
    match kind {
        NumberKind::Signed(_) => parsed_equal::<i128>(left, right),
        NumberKind::Unsigned(_) => parsed_equal::<u128>(left, right),
        NumberKind::F32 => parsed_equal::<f32>(left, right),
        NumberKind::F64 => parsed_equal::<f64>(left, right),
    }
}

/// Compares two successfully parsed values of the same primitive.
///
/// - Complexity: O(left bytes + right bytes) time and O(1) additional space.
fn parsed_equal<T: std::str::FromStr + PartialEq>(left: &str, right: &str) -> bool {
    matches!((left.parse::<T>(), right.parse::<T>()), (Ok(a), Ok(b)) if a == b)
}
