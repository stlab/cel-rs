use super::*;
use crate::input::{InputError, NumberKind};

/// Executes an eval script against a real JavaScript engine and captures its transport reply.
fn run_script(script: &str) -> String {
    let output = std::process::Command::new("node")
        .args(["-e", &format!("const dioxus = {{ send: value => process.stdout.write(JSON.stringify(value)) }};\n{script}")])
        .output().expect("numeric locale integration tests require Node");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Catches script interpolation, hidden locale errors, and lossy reply stamps.
#[test]
fn eval_scripts_encode_untrusted_strings_and_preserve_rust_event_stamps() {
    let reply = run_script(&number_locale_script(Some("de-DE")).unwrap());
    let de = decode_locale_reply(&reply).unwrap();
    assert_eq!(de.decimal, ",");
    let reply = run_script(
        &format_number_script(
            "340282366920938463463374607431768211455",
            NumberKind::Unsigned(128),
            "en-US",
            &serde_json::json!({"useGrouping":false}),
        )
        .unwrap(),
    );
    assert_eq!(
        decode_format_reply(&reply, u64::MAX, u64::MAX).unwrap(),
        DraftEvent::Formatted {
            text: "340282366920938463463374607431768211455".into(),
            generation: u64::MAX,
            revision: u64::MAX
        }
    );
    let reply = run_script(
        &number_locale_script(Some("en-US\"); throw new Error('injected') //")).unwrap(),
    );
    assert!(matches!(
        decode_locale_reply(&reply),
        Err(InputError::Locale(_))
    ));
    let reply = run_script(
        &format_number_script("1", NumberKind::F64, "zz-ZZ", &serde_json::json!({})).unwrap(),
    );
    assert!(matches!(
        decode_format_reply(&reply, 1, 1),
        Err(InputError::Locale(_))
    ));
    assert!(matches!(
        decode_locale_reply("broken transport"),
        Err(InputError::Locale(_))
    ));
    assert!(matches!(
        decode_format_reply("{}", 1, 1),
        Err(InputError::Locale(_))
    ));
    assert!(matches!(
        format_number_script("3.5e38", NumberKind::F32, "en-US", &serde_json::json!({})),
        Err(InputError::TypeRange { .. })
    ));
    assert!(
        format_number_script(
            "1\"); throw new Error('injected') //",
            NumberKind::F64,
            "en-US",
            &serde_json::json!({})
        )
        .is_err()
    );
}

/// Creates independent locale fixtures matching Intl symbols.
fn locale(name: &str) -> NumberLocale {
    let (decimal, group, digits) = match name {
        "de-DE" => (",", ".", "0123456789"),
        "sv-SE" => (",", "\u{a0}", "0123456789"),
        "fr-FR" => (",", "\u{202f}", "0123456789"),
        "ar-EG" => ("\u{66b}", "\u{66c}", "٠١٢٣٤٥٦٧٨٩"),
        _ => (".", ",", "0123456789"),
    };
    NumberLocale {
        resolved: name.into(),
        digits: std::array::from_fn(|i| digits.chars().nth(i).unwrap().to_string()),
        decimal: decimal.into(),
        group: group.into(),
        minus: if name == "sv-SE" { "−" } else { "-" }.into(),
        plus: "+".into(),
        ignorable: vec!["\u{61c}".into(), "\u{200e}".into(), "\u{200f}".into()],
        primary_group: 3,
        secondary_group: 3,
    }
}

/// Catches localized signs being rejected or losing their numeric direction.
#[test]
fn localized_signs_preserve_positive_and_negative_values() {
    let mut swedish = locale("sv-SE");
    for (raw, expected) in [("−12,5", "-12.5"), ("+12,5", "+12.5")] {
        assert_eq!(
            normalize_draft(raw, &swedish, NumberKind::F64),
            Ok(NormalizedDraft::Complete(expected.into()))
        );
    }
    // A caller-supplied non-ASCII plus token exercises the same locale contract.
    swedish.plus = "＋".into();
    assert_eq!(
        normalize_draft("＋42", &swedish, NumberKind::Signed(32)),
        Ok(NormalizedDraft::Complete("+42".into()))
    );
    assert_eq!(
        normalize_draft("−42", &swedish, NumberKind::Signed(32)),
        Ok(NormalizedDraft::Complete("-42".into()))
    );
}

/// Rejects normalization that drops precision, malformed grouping, or incomplete prefixes.
#[test]
fn localized_normalization_and_prefixes() {
    for text in [".", "-.", "1e", "1e-", "1e+"] {
        assert_eq!(
            normalize_draft(text, &locale("en-US"), NumberKind::Signed(32)),
            Ok(NormalizedDraft::Incomplete),
            "{text}"
        );
    }
    for text in ["", "-", "+", ".", "-.", "1e", "1e-", "1e+"] {
        assert_eq!(
            normalize_draft(text, &locale("en-US"), NumberKind::F64),
            Ok(NormalizedDraft::Incomplete)
        );
    }
    for (name, text, expected) in [
        ("de-DE", "1.234,5", "1234.5"),
        ("fr-FR", "1\u{a0}234,5", "1234.5"),
        ("ar-EG", "\u{61c}-١٬٢٣٤٫٥", "-1234.5"),
        (
            "en-US",
            "18,446,744,073,709,551,615",
            "18446744073709551615",
        ),
    ] {
        assert_eq!(
            normalize_draft(text, &locale(name), NumberKind::F64),
            Ok(NormalizedDraft::Complete(expected.into()))
        );
    }
    for text in [
        "1,23", "1,,234", ",123", "123,", "1234,567", "1.2,345", "1e1,234", "NaN", "Infinity",
        "12x", "1e.",
    ] {
        assert!(
            matches!(
                normalize_draft(text, &locale("en-US"), NumberKind::F64),
                Err(InputError::Lexical(_))
            ),
            "{text}"
        );
    }
    assert!(matches!(
        normalize_draft("1.2", &locale("en-US"), NumberKind::Signed(32)),
        Err(InputError::Lexical(_))
    ));
}

/// Nudges to the strictly next or previous zero-anchored step.
#[test]
fn steps_to_next_zero_anchored_multiple() {
    for (value, step, increment, expected) in [
        ("3.45", "1", true, 4.0),
        ("3.45", "1", false, 3.0),
        ("3", "1", true, 4.0),
        ("3", "1", false, 2.0),
        ("-3.45", "1", true, -3.0),
        ("-3.45", "1", false, -4.0),
        ("0.3", "0.1", true, 0.4),
        ("0.3", "0.1", false, 0.2),
        ("0.33", "0.25", true, 0.5),
        ("0.33", "0.25", false, 0.25),
        ("0", "0.1", false, -0.1),
    ] {
        let actual = step_number(value, NumberKind::F64, step, increment).unwrap();
        assert_eq!(
            actual.parse::<f64>().unwrap(),
            expected,
            "{value} {step} {increment}"
        );
    }

    for (value, increment, expected) in [
        ("-7", true, "-6"),
        ("-7", false, "-9"),
        ("6", true, "9"),
        ("6", false, "3"),
    ] {
        assert_eq!(
            step_number(value, NumberKind::Signed(128), "3", increment).unwrap(),
            expected
        );
    }
    assert_eq!(
        step_number(
            "18446744073709551615",
            NumberKind::Unsigned(128),
            "10",
            true
        )
        .unwrap(),
        "18446744073709551620"
    );
    assert_eq!(
        step_number("0.3", NumberKind::F32, "0.1", true)
            .unwrap()
            .parse::<f32>()
            .unwrap(),
        0.4
    );
    assert_eq!(
        step_number("5e-324", NumberKind::F64, "5e-324", true)
            .unwrap()
            .parse::<f64>()
            .unwrap(),
        1e-323
    );
    assert!(matches!(
        step_number("1e308", NumberKind::F64, "1", true),
        Err(InputError::TypeRange { .. })
    ));
    assert!(matches!(
        step_number("255", NumberKind::Unsigned(8), "10", true),
        Err(InputError::TypeRange { .. })
    ));
}

/// Catches typed overflow and imprecise wide-integer conversion or stepping.
#[test]
fn exact_ranges_and_checked_steps() {
    let f32_range = NumberKind::F32.range().unwrap();
    assert_eq!(f32_range.min.parse::<f32>().unwrap(), f32::MIN);
    assert_eq!(f32_range.max.parse::<f32>().unwrap(), f32::MAX);
    let f64_range = NumberKind::F64.range().unwrap();
    assert_eq!(f64_range.min.parse::<f64>().unwrap(), f64::MIN);
    assert_eq!(f64_range.max.parse::<f64>().unwrap(), f64::MAX);
    assert_eq!(
        normalize_draft(
            "18,446,744,073,709,551,615",
            &locale("en-US"),
            NumberKind::Unsigned(64)
        ),
        Ok(NormalizedDraft::Complete("18446744073709551615".into()))
    );
    for (kind, min, max) in [
        (
            NumberKind::Signed(128),
            "-170141183460469231731687303715884105728",
            "170141183460469231731687303715884105727",
        ),
        (
            NumberKind::Unsigned(128),
            "0",
            "340282366920938463463374607431768211455",
        ),
        (NumberKind::Signed(8), "-128", "127"),
    ] {
        assert_eq!(kind.range().unwrap().min, min);
        assert_eq!(kind.range().unwrap().max, max);
        for value in [min, max] {
            assert_eq!(
                normalize_draft(value, &locale("en-US"), kind),
                Ok(NormalizedDraft::Complete(value.into()))
            );
        }
        assert!(matches!(
            step_number(max, kind, "1", true),
            Err(InputError::TypeRange { .. })
        ));
        assert!(matches!(
            step_number(min, kind, "1", false),
            Err(InputError::TypeRange { .. })
        ));
    }
    assert_eq!(
        step_number("18446744073709551615", NumberKind::Unsigned(128), "1", true).unwrap(),
        "18446744073709551616"
    );
    assert_eq!(
        step_number("1.5", NumberKind::F32, "0.25", false).unwrap(),
        "1.25"
    );
    for (text, kind) in [
        ("128", NumberKind::Signed(8)),
        ("-1", NumberKind::Unsigned(64)),
        ("3.5e38", NumberKind::F32),
        ("1e999", NumberKind::F64),
        ("1e-999", NumberKind::F32),
        ("1e-9999", NumberKind::F64),
        (
            "340282366920938463463374607431768211456",
            NumberKind::Unsigned(128),
        ),
        (
            "-170141183460469231731687303715884105729",
            NumberKind::Signed(128),
        ),
    ] {
        assert!(matches!(
            normalize_draft(text, &locale("en-US"), kind),
            Err(InputError::TypeRange { .. })
        ));
    }
    assert!(matches!(
        NumberKind::Signed(7).range(),
        Err(InputError::UnsupportedEditor(_))
    ));
    for text in ["NaN", "inf", "-inf"] {
        assert!(step_number(text, NumberKind::F64, "1", true).is_err());
    }
    let mut broken = locale("en-US");
    broken.decimal.clear();
    assert!(matches!(
        normalize_draft("1", &broken, NumberKind::F64),
        Err(InputError::Locale(_))
    ));
}

/// Creates a model snapshot event.
fn update(value: &str, disabled: bool, generation: u64) -> DraftEvent {
    DraftEvent::ModelUpdate {
        canonical: value.into(),
        disabled,
        readonly: false,
        generation,
    }
}

/// Catches blur writes, lost local drafts, and reinterpretation after locale changes.
#[test]
fn draft_preserves_origin_and_restores_latest_without_writes() {
    let en = locale("en-US");
    let de = locale("de-DE");
    let kind = NumberKind::F64;
    let mut draft = NumberDraft::default();
    assert_eq!(
        draft.reduce(update("10", false, 1), &en, kind),
        DraftEffect::None
    );
    draft.reduce(DraftEvent::Focus, &en, kind);
    assert_eq!(
        draft.reduce(DraftEvent::Input("-".into()), &en, kind),
        DraftEffect::None
    );
    draft.reduce(update("20", false, 1), &de, kind);
    assert_eq!(draft.text(), "-");
    let revision = draft.revision();
    draft.reduce(
        DraftEvent::Formatted {
            text: "20,00".into(),
            generation: 1,
            revision,
        },
        &de,
        kind,
    );
    assert_eq!(draft.text(), "-");
    assert_eq!(
        draft.reduce(DraftEvent::Input("1.5".into()), &de, kind),
        DraftEffect::Submit("1.5".into())
    );
    assert_eq!(
        draft.reduce(DraftEvent::Input("-".into()), &de, kind),
        DraftEffect::None
    );
    assert_eq!(draft.reduce(DraftEvent::Blur, &de, kind), DraftEffect::None);
    assert_eq!(draft.text(), "20,00");
}

/// Cancelling composition preserves the originating locale and never submits.
#[test]
fn composition_cancellation_retains_focus_locale() {
    let en = locale("en-US");
    let de = locale("de-DE");
    let kind = NumberKind::F64;
    let mut draft = NumberDraft::default();
    draft.reduce(update("5", false, 1), &en, kind);
    draft.reduce(DraftEvent::Focus, &en, kind);
    draft.reduce(DraftEvent::CompositionStart, &en, kind);
    draft.reduce(DraftEvent::Input("17".into()), &en, kind);
    assert_eq!(
        draft.reduce(DraftEvent::CompositionCancel, &de, kind),
        DraftEffect::None
    );
    assert_eq!(draft.text(), "5");
    assert_eq!(
        draft.reduce(DraftEvent::Input("1.5".into()), &de, kind),
        DraftEffect::Submit("1.5".into())
    );
}

/// Catches partial composition writes and stale replies after edits or replacement.
#[test]
fn composition_disable_replacement_and_stale_work() {
    let en = locale("en-US");
    let kind = NumberKind::Signed(32);
    let mut draft = NumberDraft::default();
    draft.reduce(update("5", false, 1), &en, kind);
    draft.reduce(DraftEvent::Focus, &en, kind);
    let old = draft.revision();
    draft.reduce(DraftEvent::CompositionStart, &en, kind);
    assert_eq!(
        draft.reduce(DraftEvent::Input("12".into()), &en, kind),
        DraftEffect::None
    );
    assert_eq!(
        draft.reduce(DraftEvent::CompositionEnd("12".into()), &en, kind),
        DraftEffect::Submit("12".into())
    );
    assert_eq!(
        draft.reduce(DraftEvent::Input("12".into()), &en, kind),
        DraftEffect::Submit("12".into())
    );
    draft.reduce(
        DraftEvent::Formatted {
            text: "OLD".into(),
            generation: 1,
            revision: old,
        },
        &en,
        kind,
    );
    assert_eq!(draft.text(), "12");
    assert!(matches!(
        draft.reduce(DraftEvent::Input("999999999999".into()), &en, kind),
        DraftEffect::Report(InputError::TypeRange { .. })
    ));
    draft.reduce(update("9", true, 1), &en, kind);
    assert_eq!(draft.text(), "9");
    assert_eq!(
        draft.reduce(DraftEvent::CompositionEnd("22".into()), &en, kind),
        DraftEffect::None
    );
    draft.reduce(update("8", false, 2), &en, kind);
    draft.reduce(update("7", false, 1), &en, kind);
    assert_eq!(draft.text(), "8");
    assert_eq!(
        draft.reduce_stamped(
            1,
            draft.revision(),
            DraftEvent::Input("33".into()),
            &en,
            kind
        ),
        DraftEffect::None
    );
    draft.retire();
    assert_eq!(
        draft.reduce(DraftEvent::Input("44".into()), &en, kind),
        DraftEffect::None
    );
}

/// Catches nonfinite model values being hidden or converted into finite edits.
#[test]
fn nonfinite_model_display_is_explicit() {
    let en = locale("en-US");
    let mut draft = NumberDraft::default();
    for value in ["NaN", "inf", "-inf"] {
        draft.reduce(update(value, false, 1), &en, NumberKind::F64);
        assert_eq!(draft.text(), value);
        draft.reduce(
            DraftEvent::Formatted {
                text: "0".into(),
                generation: 1,
                revision: draft.revision(),
            },
            &en,
            NumberKind::F64,
        );
        assert_eq!(draft.text(), value);
        assert!(matches!(
            draft.reduce(DraftEvent::Input(value.into()), &en, NumberKind::F64),
            DraftEffect::Report(InputError::Lexical(_))
        ));
    }
}

/// Catches acknowledged complete edits losing equivalent user-entered formatting.
#[test]
fn complete_edits_preserve_spelling_until_blur_or_authoritative_change() {
    let en = locale("en-US");
    let mut draft = NumberDraft::default();
    draft.reduce(update("5", false, 1), &en, NumberKind::F64);
    draft.reduce(DraftEvent::Focus, &en, NumberKind::F64);
    assert_eq!(
        draft.reduce(DraftEvent::Input("1,234.00".into()), &en, NumberKind::F64),
        DraftEffect::Submit("1234.00".into())
    );
    draft.reduce(update("1234", false, 1), &en, NumberKind::F64);
    assert_eq!(draft.text(), "1,234.00");
    assert_eq!(draft.canonical(), "1234");
    draft.reduce(
        DraftEvent::Formatted {
            text: "1,234".into(),
            generation: 1,
            revision: draft.revision(),
        },
        &en,
        NumberKind::F64,
    );
    assert_eq!(draft.text(), "1,234.00");
    draft.reduce(update("1200", false, 1), &en, NumberKind::F64);
    assert_eq!(draft.text(), "1200");
    draft.reduce(update("1234", false, 1), &en, NumberKind::F64);
    assert_eq!(draft.text(), "1234");
    draft.reduce(DraftEvent::Blur, &en, NumberKind::F64);
    assert_eq!(draft.text(), "1234");
}

/// Catches stale native edits, readonly writes, and composition revival after blur.
#[test]
fn readonly_and_revision_guards_suppress_delayed_native_events() {
    let en = locale("en-US");
    let kind = NumberKind::Signed(32);
    let mut draft = NumberDraft::default();
    draft.reduce(update("5", false, 1), &en, kind);
    let revision = draft.revision();
    draft.reduce(DraftEvent::Focus, &en, kind);
    draft.reduce(DraftEvent::Input("-".into()), &en, kind);
    assert_eq!(
        draft.reduce_stamped(1, revision, DraftEvent::Input("7".into()), &en, kind),
        DraftEffect::None
    );
    assert_eq!(draft.text(), "-");
    draft.reduce(
        DraftEvent::ModelUpdate {
            canonical: "6".into(),
            disabled: false,
            readonly: true,
            generation: 1,
        },
        &en,
        kind,
    );
    assert_eq!(draft.text(), "6");
    assert_eq!(
        draft.reduce(DraftEvent::Input("7".into()), &en, kind),
        DraftEffect::None
    );
    draft.reduce(update("8", false, 2), &en, kind);
    draft.reduce(
        DraftEvent::Formatted {
            text: "STALE".into(),
            generation: 1,
            revision: draft.revision(),
        },
        &en,
        kind,
    );
    assert_eq!(draft.text(), "8");
    draft.reduce(DraftEvent::CompositionStart, &en, kind);
    draft.reduce(DraftEvent::Blur, &en, kind);
    assert_eq!(
        draft.reduce(DraftEvent::CompositionEnd("9".into()), &en, kind),
        DraftEffect::None
    );
}

/// Catches retained obsolete focus locales after a draft is forcibly discarded.
#[test]
fn disabled_updates_end_the_local_edit_session() {
    let en = locale("en-US");
    let de = locale("de-DE");
    let kind = NumberKind::F64;
    let mut draft = NumberDraft::default();
    draft.reduce(update("5", false, 1), &en, kind);
    draft.reduce(DraftEvent::Focus, &en, kind);
    draft.reduce(DraftEvent::Input("-".into()), &en, kind);
    draft.reduce(update("6", true, 1), &de, kind);
    draft.reduce(update("6", false, 1), &de, kind);
    draft.reduce(DraftEvent::Focus, &de, kind);
    assert_eq!(
        draft.reduce(DraftEvent::Input("1,5".into()), &de, kind),
        DraftEffect::Submit("1.5".into())
    );
    draft.reduce(DraftEvent::Input("-".into()), &de, kind);
    draft.retire();
    assert_eq!(draft.text(), "6");
}

/// Catches incorrect grouping rules and acceptance of ambiguous token vocabularies.
#[test]
fn grouping_and_locale_validation_boundaries() {
    let mut indian = locale("en-US");
    indian.secondary_group = 2;
    assert_eq!(
        normalize_draft("12,34,567", &indian, NumberKind::Unsigned(32)),
        Ok(NormalizedDraft::Complete("1234567".into()))
    );
    assert!(matches!(
        normalize_draft("1,234,567", &indian, NumberKind::Unsigned(32)),
        Err(InputError::Lexical(_))
    ));
    for (name, value) in [("de-DE", "1.23,4"), ("fr-FR", "1 23,4"), ("ar-EG", "١٬٢٣")] {
        assert!(matches!(
            normalize_draft(value, &locale(name), NumberKind::F64),
            Err(InputError::Lexical(_))
        ));
    }
    let mut broken = locale("en-US");
    broken.digits[0] = "1".into();
    assert!(matches!(
        normalize_draft("1", &broken, NumberKind::F64),
        Err(InputError::Locale(_))
    ));
    broken = locale("en-US");
    broken.ignorable = vec!["x".into()];
    assert!(matches!(
        normalize_draft("1x", &broken, NumberKind::F64),
        Err(InputError::Locale(_))
    ));
    assert!(step_number("1", NumberKind::F32, "0", true).is_err());
    assert!(step_number("1", NumberKind::Signed(32), "-1", true).is_err());
    assert!(step_number("1", NumberKind::Unsigned(32), "0", true).is_err());
    assert!(step_number("1", NumberKind::F64, "", true).is_err());
    assert!(
        step_number(
            "1.7976931348623157e308",
            NumberKind::F64,
            "1.7976931348623157e308",
            true
        )
        .is_err()
    );
    assert!(step_number("3.4028235e38", NumberKind::F32, "3.4028235e38", true).is_err());
}
