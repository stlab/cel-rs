//! Verifies numeric adapter decisions independently of a mounted model.
use super::*;

/// Resolves an independently specified locale fixture for transition tests.
fn locale() -> NumberLocale {
    serde_json::from_str(
        r#"{
      "resolved":"en-US","digits":["0","1","2","3","4","5","6","7","8","9"],
      "decimal":".","group":",","minus":"-","plus":"+","ignorable":[],
      "primary_group":3,"secondary_group":3}"#,
    )
    .unwrap()
}

/// Creates synchronized state without a model framework.
fn state() -> EditorState {
    let mut state = EditorState::new();
    state.locale = Some(locale());
    state.sync("10", false, false, NumberKind::Signed(32));
    state
}

/// Generation, edit, locale and retirement independently reject delayed work.
#[test]
fn delayed_work_is_bound_to_instance_edit_and_locale() {
    let mut first = state();
    let second = state();
    assert!(second.generation > first.generation);
    let generation = first.generation;
    let revision = first.draft.revision();
    let epoch = first.epoch;
    assert!(first.accepts(generation, revision, epoch));
    assert!(!first.accepts(second.generation, revision, epoch));
    first.event(DraftEvent::Input("-".into()), NumberKind::Signed(32), false);
    assert!(!first.accepts(generation, revision, epoch));
    let revision = first.draft.revision();
    first.epoch += 1;
    assert!(!first.accepts(generation, revision, epoch));
    first.retire();
    assert!(!first.accepts(generation, revision, first.epoch));
    assert_eq!(
        first.event(
            DraftEvent::Input("22".into()),
            NumberKind::Signed(32),
            false
        ),
        UiEffect::None
    );
}

/// Optional composition and forced state cannot leak a clear callback.
#[test]
fn optional_clear_waits_for_composition_and_respects_forced_state() {
    let mut state = state();
    state.event(DraftEvent::CompositionStart, NumberKind::Signed(32), true);
    assert_eq!(
        state.event(DraftEvent::Input("".into()), NumberKind::Signed(32), true),
        UiEffect::None
    );
    assert_eq!(
        state.event(
            DraftEvent::CompositionEnd("".into()),
            NumberKind::Signed(32),
            true
        ),
        UiEffect::Submit(None)
    );
    assert_eq!(
        state.event(DraftEvent::Input("".into()), NumberKind::Signed(32), true),
        UiEffect::Submit(None)
    );
    state.sync("12", true, false, NumberKind::Signed(32));
    assert_eq!(
        state.event(DraftEvent::Input("".into()), NumberKind::Signed(32), true),
        UiEffect::None
    );
}

/// Slider decisions reject imprecise, nonfinite and unrepresentable writes.
#[test]
fn slider_decisions_preserve_primitive_domains_and_aspirations() {
    let range = NumericRange {
        min: "0".into(),
        max: "10".into(),
    };
    assert!(slider_domain(NumberKind::Signed(32), Some("5"), Some(&range), "1").is_some());
    assert!(slider_domain(NumberKind::Signed(64), Some("5"), Some(&range), "1").is_none());
    assert!(slider_domain(NumberKind::F64, Some("NaN"), Some(&range), "1").is_none());
    assert!(slider_domain(NumberKind::F64, Some("11"), Some(&range), "1").is_none());
    assert!(slider_domain(NumberKind::F64, Some("5"), Some(&range), "0").is_none());
    assert_eq!(
        slider_effect(200.0, NumberKind::Signed(32), false),
        UiEffect::Submit(Some("200".into()))
    );
    assert_eq!(slider_effect(3.0, NumberKind::F64, true), UiEffect::None);
    for (value, kind) in [
        (f64::NAN, NumberKind::F64),
        (f64::INFINITY, NumberKind::F64),
        (256.0, NumberKind::Unsigned(8)),
        (1.5, NumberKind::Signed(32)),
        (5.0, NumberKind::Signed(64)),
    ] {
        assert!(matches!(
            slider_effect(value, kind, false),
            UiEffect::Report(_)
        ));
    }
    assert!(
        error_message(&InputError::Locale("unsupported locale".into()))
            .contains("unsupported locale")
    );
}

/// Stepping uses the effective value and remains blocked during composition or retirement.
#[test]
fn step_decisions_respect_model_state_and_session_lifetime() {
    let mut state = state();
    assert_eq!(
        step_effect(&state, NumberKind::Signed(32), "2", true, false, false),
        UiEffect::Submit(Some("12".into()))
    );
    assert_eq!(
        step_effect(&state, NumberKind::Signed(32), "2", false, false, false),
        UiEffect::Submit(Some("8".into()))
    );
    assert_eq!(
        step_effect(&state, NumberKind::Signed(32), "2", true, true, false),
        UiEffect::None
    );
    assert_eq!(
        step_effect(&state, NumberKind::Signed(32), "2", true, false, true),
        UiEffect::None
    );
    state.event(DraftEvent::CompositionStart, NumberKind::Signed(32), false);
    assert_eq!(
        step_effect(&state, NumberKind::Signed(32), "2", true, false, false),
        UiEffect::None
    );
    let generation = state.generation;
    state.lose_locale();
    assert!(state.generation > generation);
    assert_eq!(
        step_effect(&state, NumberKind::Signed(32), "2", true, false, false),
        UiEffect::None
    );
    state.locale = Some(locale());
    state.sync("20", false, false, NumberKind::Signed(32));
    assert_eq!(
        step_effect(&state, NumberKind::Signed(32), "2", true, false, false),
        UiEffect::Submit(Some("22".into()))
    );
    state.retire();
    assert_eq!(
        step_effect(&state, NumberKind::Signed(32), "2", true, false, false),
        UiEffect::None
    );
}

/// Finite endpoints whose span overflows cannot produce a representable official slider.
#[test]
fn overflowing_slider_span_keeps_only_the_exact_editor() {
    let range = NumberKind::F64.range().unwrap();
    assert!(slider_domain(NumberKind::F64, Some("0"), Some(&range), "1").is_none());
}

/// Only an explicit validated source completion can submit a composing value.
#[test]
fn composing_state_requires_explicit_source_completion() {
    let mut state = state();
    state.event(DraftEvent::CompositionStart, NumberKind::Signed(32), false);
    state.event(
        DraftEvent::Input("17".into()),
        NumberKind::Signed(32),
        false,
    );
    assert_eq!(
        state.event(
            DraftEvent::CompositionEnd("19".into()),
            NumberKind::Signed(32),
            false
        ),
        UiEffect::Submit(Some("19".into()))
    );
}

/// Forced drafts clear their own failures without concealing formatter/transport failures.
#[test]
fn discarded_draft_errors_remain_distinct_from_locale_diagnostics() {
    let lexical = InputError::Lexical("bad".into());
    let range = InputError::TypeRange {
        kind: NumberKind::Signed(8),
        value: "999".into(),
    };
    let locale = InputError::Locale("transport failed".into());
    assert!(discard_error(true, Some(&lexical)));
    assert!(discard_error(true, Some(&range)));
    assert!(!discard_error(false, Some(&lexical)));
    assert!(!discard_error(true, Some(&locale)));
    assert!(!discard_error(true, None));
}
