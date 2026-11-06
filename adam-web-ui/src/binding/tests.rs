//! Tests public transaction, recovery, scalar-codec, and publication contracts.

use super::*;
use crate::{BuildError, BuiltSheet, PreparedWrite, build_sheet};
use adam_rs::Method;
use std::{cell::Cell, rc::Rc};

/// Captures filtered effective values without replacing recovery aspirations.
#[test]
fn recording_captures_effective_not_aspired_values() {
    let (mut owner, _) = owner();
    owner.start_recording(&["limited", "limited"]).unwrap();
    assert!(matches!(
        owner.apply(&[edit("limited", "20")], UpdateOrigin::User),
        HostOutcome::Accepted
    ));
    let recording = &owner.recordings()[0];
    assert_eq!(recording.values, [edit("limited", "10")]);
    assert_eq!(
        owner.accepted_state().transactions[0],
        [edit("limited", "20")]
    );
    let json = serde_json::to_string(recording.as_ref()).unwrap();
    let persisted: Recording = serde_json::from_str(&json).unwrap();
    let replayed = SheetOwner::playback(Rc::new(model), "test/v1".into(), &persisted).unwrap();
    assert_eq!(
        replayed
            .inspect(|p, _| *p.read::<i32>(p.cell_names["limited"].0).unwrap())
            .unwrap(),
        10
    );
    owner.stop_recording();
    owner.apply(&[edit("limited", "9")], UpdateOrigin::User);
    assert_eq!(owner.recordings().len(), 1);
}

/// Requires the complete captured group to satisfy a requirement at propagation.
#[test]
fn playback_writes_all_sources_before_propagating() {
    let evaluations = Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = evaluations.clone();
    let factory: ModelFactory = Rc::new(move || {
        let mut built = build_sheet(
            "sheet s { source a: i32 = 1; source b: i32 = 1; source valid: bool = true require { @equal a == b; }; }",
            "batch.adm2",
        )?;
        let result = built.parsed.add_cell(0_i32);
        let observed = observed.clone();
        built
            .parsed
            .add_relationship(vec![Method::from_fn_2_1(
                [built.keys["a"], built.keys["b"]],
                result,
                move |a: &i32, b: &i32| {
                    observed.borrow_mut().push((*a, *b));
                    Ok(*a)
                },
            )])
            .unwrap();
        built.parsed.propagate().unwrap();
        Ok(built)
    });
    let recording = Recording {
        schema_key: "batch/v1".into(),
        values: vec![edit("a", "9"), edit("b", "9")],
    };
    let owner = SheetOwner::playback(factory, "batch/v1".into(), &recording).unwrap();
    assert_eq!(owner.accepted_state().transactions.len(), 1);
    assert_eq!(*evaluations.borrow(), [(1, 1), (9, 9)]);
    assert_eq!(
        owner.views().iter().find(|v| v.key == "a").unwrap().text,
        "9"
    );
}

/// Playback and recovery origins never append user action recordings.
#[test]
fn playback_does_not_record_itself() {
    let (mut owner, _) = owner();
    owner.start_recording(&[]).unwrap();
    owner.apply(&[edit("limited", "20")], UpdateOrigin::Playback);
    owner.apply(&[edit("limited", "8")], UpdateOrigin::Recovery);
    assert!(owner.recordings().is_empty());
    let captured = owner.capture(&[]).unwrap();
    let replayed = SheetOwner::playback(Rc::new(model), "test/v1".into(), &captured).unwrap();
    assert!(replayed.recordings().is_empty());
}

/// Captures self-referencing producers even when they are not unclaimed sources.
#[test]
fn recording_self_reference_and_exact_float() {
    let factory: ModelFactory = Rc::new(|| {
        let mut built = build_sheet("sheet s { cell x: f64 = 1.2345678901234567; }", "self.adm2")?;
        let x = built.keys["x"];
        built
            .parsed
            .add_relationship(vec![Method::from_fn_1_1(x, x, |x: &f64| Ok(*x))])
            .unwrap();
        built.parsed.propagate().unwrap();
        Ok(built)
    });
    let owner = SheetOwner::new(factory.clone(), "float/v1".into()).unwrap();
    assert!(
        !owner
            .inspect(|p, _| p.is_source(p.cell_names["x"].0))
            .unwrap()
    );
    assert_ne!(
        owner
            .inspect(|p, labels| (labels.cells[&p.cell_names["x"].0].display)(p))
            .unwrap(),
        "1.2345678901234567"
    );
    let captured = owner.capture(&["x", "x"]).unwrap();
    assert_eq!(captured.values, [edit("x", "1.2345678901234567")]);
    let replayed = SheetOwner::playback(factory, "float/v1".into(), &captured).unwrap();
    assert_eq!(
        replayed
            .inspect(|p, _| p.read::<f64>(p.cell_names["x"].0).unwrap().to_bits())
            .unwrap(),
        1.2345678901234567_f64.to_bits()
    );
}

/// Selects the first registered alias independently of lexical order.
#[test]
fn recording_alias_uses_first_registered_key() {
    for aliases in [["z-first", "a-second"], ["a-second", "z-first"]] {
        let factory: ModelFactory = Rc::new(move || {
            let mut built = build_sheet("sheet s {}", "alias.adm2")?;
            let id = built.parsed.add_source(7_i32);
            built.labels.add_cell::<i32>(id, "value");
            for alias in aliases {
                built.keys.insert(alias.into(), id);
            }
            built.parsed.propagate().unwrap();
            Ok(built)
        });
        let owner = SheetOwner::new(factory, "alias/v1".into()).unwrap();
        assert_eq!(owner.capture(&[]).unwrap().values, [edit(aliases[0], "7")]);
        assert_eq!(
            owner.capture(&[aliases[1]]).unwrap().values,
            [edit(aliases[0], "7")]
        );
    }
}

/// Preserves wide integers, custom scalar codecs, private host keys, and aliases.
#[test]
fn recording_private_custom_and_wide_values_round_trip() {
    /// Holds a host-defined scalar that has no display or parsing implementation.
    #[derive(Debug, PartialEq)]
    struct Token(u8);
    let factory: ModelFactory = Rc::new(|| {
        let mut built = build_sheet(
            "sheet s { cell wide: u128 = 340282366920938463463374607431768211455u128; }",
            "wide.adm2",
        )?;
        let id = built.parsed.add_source(Token(9));
        built.keys.insert("private-token".into(), id);
        built.keys.insert("alias".into(), id);
        built.labels.add_cell_with_codec(
            id,
            "Token",
            crate::ScalarCodec {
                encode: |v: &Token| format!("token:{}", v.0),
                decode: |text| {
                    text.strip_prefix("token:")
                        .and_then(|v| v.parse().ok())
                        .map(Token)
                        .ok_or_else(|| crate::input::InputError::Lexical(text.into()))
                },
            },
        );
        built.parsed.propagate().unwrap();
        Ok(built)
    });
    let owner = SheetOwner::new(factory.clone(), "custom/v1".into()).unwrap();
    let captured = owner.capture(&[]).unwrap();
    assert_eq!(captured.values.len(), 2);
    assert!(captured.values.contains(&edit("private-token", "token:9")));
    assert!(
        captured
            .values
            .contains(&edit("wide", "340282366920938463463374607431768211455"))
    );
    let replayed = SheetOwner::playback(factory, "custom/v1".into(), &captured).unwrap();
    assert_eq!(
        replayed
            .inspect(|p, _| *p.read::<u128>(p.cell_names["wide"].0).unwrap())
            .unwrap(),
        u128::MAX
    );
    assert_eq!(
        replayed
            .views()
            .iter()
            .find(|v| v.key == "private-token")
            .unwrap()
            .text,
        "token:9"
    );
}

/// Empty models capture and replay without manufacturing a user action.
#[test]
fn recording_empty_model_has_empty_sources() {
    let factory: ModelFactory = Rc::new(|| build_sheet("sheet s {}", "empty.adm2"));
    let mut owner = SheetOwner::new(factory.clone(), "empty/v1".into()).unwrap();
    owner.start_recording(&[]).unwrap();
    assert!(matches!(
        owner.apply(&[], UpdateOrigin::User),
        HostOutcome::Accepted
    ));
    let recording = owner.capture(&[]).unwrap();
    assert!(recording.values.is_empty());
    let replayed = SheetOwner::playback(factory, "empty/v1".into(), &recording).unwrap();
    assert!(replayed.accepted_state().transactions.is_empty());
    assert!(replayed.recordings().is_empty());
}

/// Retains shared recordings across rejection and explicit aspiration-based recovery.
#[test]
fn recording_rejected_actions_do_not_append() {
    let (mut owner, _) = owner();
    owner.start_recording(&["limited"]).unwrap();
    assert!(matches!(
        owner.apply(&[edit("limited", "20")], UpdateOrigin::User),
        HostOutcome::Accepted
    ));
    let first = owner.recordings()[0].clone();
    assert!(matches!(
        owner.apply(&[edit("valid", "false")], UpdateOrigin::User),
        HostOutcome::Rejected(_)
    ));
    assert_eq!(owner.recordings().len(), 1);
    owner.restore_accepted().unwrap();
    assert!(Rc::ptr_eq(&first, &owner.recordings()[0]));
    assert!(matches!(
        owner.apply(&[edit("limited", "7")], UpdateOrigin::User),
        HostOutcome::Accepted
    ));
    assert_eq!(owner.recordings().len(), 2);
    assert_eq!(owner.recordings()[1].values, [edit("limited", "7")]);
}

/// Recomputes source membership after each conditional selection.
#[test]
fn recording_conditional_source_changes() {
    let factory: ModelFactory = Rc::new(|| {
        let mut built = build_sheet(
            "sheet s { source enabled: bool = true; source input: i32 = 7; cell result: i32 = 0; }",
            "conditional.adm2",
        )?;
        let relationship = built
            .parsed
            .add_relationship(vec![Method::from_fn_1_1(
                built.keys["input"],
                built.keys["result"],
                |v: &i32| Ok(*v),
            )])
            .unwrap();
        built
            .parsed
            .add_conditional(
                adam_rs::MatchExpr::cell(built.keys["enabled"]),
                vec![(vec![true], vec![relationship])],
                vec![],
            )
            .unwrap();
        built.parsed.propagate().unwrap();
        Ok(built)
    });
    let mut owner = SheetOwner::new(factory.clone(), "conditional/v1".into()).unwrap();
    let before = owner.capture(&["result"]).unwrap();
    assert!(before.values.iter().any(|e| e.key == "input"));
    owner.start_recording(&["result"]).unwrap();
    owner.apply(&[edit("enabled", "false")], UpdateOrigin::User);
    let after = &owner.recordings()[0];
    assert!(!after.values.iter().any(|e| e.key == "input"));
    assert!(after.values.iter().any(|e| e.key == "result"));
    let replayed = SheetOwner::playback(factory, "conditional/v1".into(), after).unwrap();
    assert_eq!(
        replayed
            .views()
            .iter()
            .find(|v| v.key == "result")
            .unwrap()
            .text,
        "0"
    );
}

/// Reports unknown results, missing stable identities/codecs, and unavailable models.
#[test]
fn recording_capture_errors_are_explicit() {
    let (mut owner, _) = owner();
    assert!(matches!(
        owner.capture(&["absent"]),
        Err(CaptureError::UnknownResult(_))
    ));
    assert!(owner.start_recording(&["absent"]).is_err());
    owner.apply(&[edit("left", "bad")], UpdateOrigin::User);
    assert!(matches!(
        owner.capture(&[]),
        Err(CaptureError::Unavailable(_))
    ));
    let factory = Rc::new(|| {
        let mut built = model()?;
        built.parsed.add_source(3_i32);
        built.parsed.propagate().unwrap();
        Ok(built)
    });
    assert!(matches!(
        SheetOwner::new(factory, "private/v1".into())
            .unwrap()
            .capture(&[]),
        Err(CaptureError::UnnamedSource)
    ));
    let factory = Rc::new(|| {
        let mut built = model()?;
        built
            .labels
            .cells
            .get_mut(&built.keys["limited"])
            .unwrap()
            .codec_supported = false;
        Ok(built)
    });
    assert!(matches!(
        SheetOwner::new(factory, "unsupported/v1".into())
            .unwrap()
            .capture(&["limited"]),
        Err(CaptureError::MissingCodec(_))
    ));
}

/// Preserves typed playback errors without retrying a failed runtime.
#[test]
fn recording_playback_errors_are_classified() {
    let recording = Recording {
        schema_key: "test/v1".into(),
        values: vec![edit("limited", "true")],
    };
    assert!(matches!(
        SheetOwner::playback(Rc::new(model), "other/v1".into(), &recording),
        Err(PlaybackError::Capture(CaptureError::SchemaMismatch { .. }))
    ));
    assert!(matches!(
        SheetOwner::playback(Rc::new(model), "test/v1".into(), &recording),
        Err(PlaybackError::Rejected(HostRejection::Input(_)))
    ));
    let unknown = Recording {
        schema_key: "test/v1".into(),
        values: vec![edit("absent", "1")],
    };
    assert!(matches!(
        SheetOwner::playback(Rc::new(model), "test/v1".into(), &unknown),
        Err(PlaybackError::Rejected(HostRejection::Binding(
            BindingError::UnknownKey(_)
        )))
    ));
    let invalid = Recording {
        schema_key: "test/v1".into(),
        values: vec![edit("valid", "false")],
    };
    assert!(matches!(
        SheetOwner::playback(Rc::new(model), "test/v1".into(), &invalid),
        Err(PlaybackError::Rejected(HostRejection::Validation(_)))
    ));
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let factory = Rc::new(move || {
        count.set(count.get() + 1);
        build_sheet(
            "sheet s { cell divisor: i32 = 1; out result: i32 := 10 / divisor; }",
            "fatal.adm2",
        )
    });
    let fatal = Recording {
        schema_key: "fatal/v1".into(),
        values: vec![edit("divisor", "0")],
    };
    assert!(matches!(
        SheetOwner::playback(factory, "fatal/v1".into(), &fatal),
        Err(PlaybackError::Fatal(_))
    ));
    assert_eq!(calls.get(), 1);
    assert!(matches!(
        SheetOwner::playback(
            Rc::new(|| build_sheet("bad", "bad.adm2")),
            "test/v1".into(),
            &recording
        ),
        Err(PlaybackError::Build(_))
    ));
}

/// Stops a failed recorder while retaining the newly accepted state and journal.
#[test]
fn recording_failure_preserves_accepted_transaction() {
    let captures = Rc::new(Cell::new(0));
    let counter = captures.clone();
    let factory = Rc::new(move || {
        let mut built = model()?;
        let counter = counter.clone();
        let id = built.keys["limited"];
        built.labels.cells.get_mut(&id).unwrap().capture = Box::new(move |sheet| {
            counter.set(counter.get() + 1);
            if counter.get() == 4 {
                Err(crate::input::InputError::UnsupportedEditor(
                    "capture unavailable".into(),
                ))
            } else {
                Ok(sheet.read::<i32>(id).unwrap().to_string())
            }
        });
        Ok(built)
    });
    let mut owner = SheetOwner::new(factory, "record/v1".into()).unwrap();
    owner.start_recording(&["limited"]).unwrap();
    assert!(matches!(
        owner.apply(&[edit("limited", "20")], UpdateOrigin::User),
        HostOutcome::RecordingFailed(CaptureError::Encoding { .. })
    ));
    assert_eq!(
        owner
            .views()
            .iter()
            .find(|v| v.key == "limited")
            .unwrap()
            .text,
        "10"
    );
    assert_eq!(
        owner.accepted_state().transactions[0],
        [edit("limited", "20")]
    );
    assert!(owner.inspect(|_, _| ()).is_ok());
    assert!(owner.recordings().is_empty());
    assert!(matches!(
        owner.apply(&[edit("limited", "8")], UpdateOrigin::User),
        HostOutcome::Accepted
    ));
    assert!(owner.recordings().is_empty());
    owner.restore_accepted().unwrap();
    assert_eq!(
        owner
            .views()
            .iter()
            .find(|v| v.key == "limited")
            .unwrap()
            .text,
        "8"
    );
}

/// Creates one canonical edit.
fn edit(key: &str, canonical: &str) -> CellEdit {
    CellEdit {
        key: key.into(),
        canonical: canonical.into(),
    }
}

/// Builds a filtered model with a reversible relationship and a requirement.
fn model() -> Result<BuiltSheet, BuildError> {
    let mut built = build_sheet(
        "sheet s {
            cell limited: i32 = 5 filter 0..=10;
            cell left: i32 = 1;
            cell right: i32 = 1;
            source valid: bool = true require { @accepted valid; };
        }",
        "owner.adm2",
    )?;
    let left = built.keys["left"];
    let right = built.keys["right"];
    built
        .parsed
        .add_relationship(vec![
            Method::from_fn_1_1(left, right, |x: &i32| Ok(*x)),
            Method::from_fn_1_1(right, left, |x: &i32| Ok(*x)),
        ])
        .unwrap();
    let file_name = built.file_name;
    let mut initialized = crate::initialize_sheet(built.parsed)?;
    initialized.file_name = file_name;
    Ok(initialized)
}

/// Counts construction without making recovery policy implicit.
fn owner() -> (SheetOwner, Rc<Cell<usize>>) {
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let factory = Rc::new(move || {
        count.set(count.get() + 1);
        model()
    });
    (SheetOwner::new(factory, "test/v1".into()).unwrap(), calls)
}

/// Conversion of the entire group precedes every write and retains accepted views.
#[test]
fn conversion_rejection_does_not_mutate() {
    let (mut owner, calls) = owner();
    let before = owner.views()[0].clone();
    assert!(matches!(
        owner.apply(
            &[edit("limited", "8"), edit("left", "bad")],
            UpdateOrigin::User
        ),
        HostOutcome::Rejected(HostRejection::Input(_))
    ));
    assert!(Rc::ptr_eq(&before, &owner.views()[0]));
    assert!(owner.accepted_state().transactions.is_empty());
    assert_eq!(calls.get(), 1);
    assert!(owner.inspect(|_, _| panic!("unavailable")).is_err());
    owner.restore_accepted().unwrap();
    assert_eq!(
        owner
            .inspect(|p, _| *p.read::<i32>(p.cell_names["limited"].0).unwrap())
            .unwrap(),
        5
    );
}

/// Ordered accepted input replay restores raw aspirations and relationship direction.
#[test]
fn requirement_rejection_preserves_accepted_aspiration_and_direction() {
    let (mut owner, calls) = owner();
    assert!(matches!(
        owner.apply(
            &[edit("limited", "20"), edit("right", "7")],
            UpdateOrigin::User
        ),
        HostOutcome::Accepted
    ));
    let direction = owner
        .inspect(|p, _| p.is_source(p.cell_names["right"].0))
        .unwrap();
    assert!(direction);
    assert_eq!(owner.accepted_state().transactions[0][0].canonical, "20");
    let before = owner.views()[0].clone();
    assert!(matches!(
        owner.apply(
            &[edit("left", "9"), edit("valid", "false")],
            UpdateOrigin::Playback
        ),
        HostOutcome::Rejected(HostRejection::Validation(_))
    ));
    assert!(Rc::ptr_eq(&before, &owner.views()[0]));
    assert!(owner.inspect(|_, _| ()).is_err());
    assert_eq!(calls.get(), 1);
    owner.restore_accepted().unwrap();
    let (aspiration, effective, restored_direction) = owner
        .inspect(|p, _| {
            let limited = p.cell_names["limited"].0;
            (
                *p.source::<i32>(limited).unwrap(),
                *p.read::<i32>(limited).unwrap(),
                p.is_source(p.cell_names["right"].0),
            )
        })
        .unwrap();
    assert_eq!(aspiration, 20);
    assert_eq!(effective, 10);
    assert_eq!(restored_direction, direction);
    assert_eq!(owner.accepted_state().transactions.len(), 1);
}

/// Fatal propagation retires the instance, including explicit restoration attempts.
#[test]
fn propagation_failure_retires_model_without_rebuilding() {
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let factory = Rc::new(move || {
        count.set(count.get() + 1);
        build_sheet(
            "sheet s { cell divisor: i32 = 1; out result: i32 := 10 / divisor; }",
            "fatal.adm2",
        )
    });
    let mut owner = SheetOwner::new(factory, "fatal/v1".into()).unwrap();
    let before = calls.get();
    match owner.apply(&[edit("divisor", "0")], UpdateOrigin::User) {
        HostOutcome::Fatal(failure) => assert_eq!(failure.file_name, "fatal.adm2"),
        other => panic!("expected fatal: {other:?}"),
    }
    assert!(owner.inspect(|_, _| panic!("failed candidate")).is_err());
    assert!(owner.restore_accepted().is_err());
    assert!(matches!(
        owner.apply(&[edit("divisor", "2")], UpdateOrigin::Recovery),
        HostOutcome::Rejected(HostRejection::Binding(BindingError::RetiredInstance))
    ));
    assert_eq!(calls.get(), before);
}

/// Write errors remain distinct from conversion and propagation errors.
#[test]
fn write_error_discards_candidate() {
    let factory = Rc::new(|| {
        let mut built = model()?;
        built
            .labels
            .cells
            .get_mut(&built.keys["left"])
            .unwrap()
            .prepare = Box::new(|_| Ok(PreparedWrite::new(1_u8)));
        Ok(built)
    });
    let mut owner = SheetOwner::new(factory, "write/v1".into()).unwrap();
    assert!(matches!(
        owner.apply(&[edit("left", "1")], UpdateOrigin::User),
        HostOutcome::Rejected(HostRejection::Mutation(_))
    ));
    assert!(owner.inspect(|_, _| ()).is_err());
    assert!(owner.accepted_state().transactions.is_empty());
    owner.restore_accepted().unwrap();
}

/// Two edits publish coherently without requiring any explicit outputs.
#[test]
fn two_edit_transaction_without_outputs() {
    let (mut owner, _) = owner();
    assert!(owner.views().iter().all(|v| !v.flags.disabled));
    assert!(matches!(
        owner.apply(
            &[edit("left", "9"), edit("right", "11")],
            UpdateOrigin::Playback
        ),
        HostOutcome::Accepted
    ));
    assert_eq!(owner.accepted_state().transactions.len(), 1);
    assert_eq!(owner.accepted_state().transactions[0].len(), 2);
    assert_eq!(
        owner
            .inspect(|p, _| *p.read::<i32>(p.cell_names["left"].0).unwrap())
            .unwrap(),
        11
    );
}

/// A failed explicit reconstruction reports its error and keeps accepted publication.
#[test]
fn restoration_failure_does_not_substitute_a_model() {
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let mut owner = SheetOwner::new(
        Rc::new(move || {
            count.set(count.get() + 1);
            if count.get() > 1 {
                build_sheet("sheet broken { cell x }", "broken.adm2")
            } else {
                model()
            }
        }),
        "restore/v1".into(),
    )
    .unwrap();
    let before = owner.views()[0].clone();
    let _ = owner.apply(&[edit("valid", "false")], UpdateOrigin::User);
    assert!(owner.restore_accepted().is_err());
    assert!(Rc::ptr_eq(&before, &owner.views()[0]));
    assert!(owner.inspect(|_, _| ()).is_err());
}

/// Unknown keys and unsupported compounds report typed binding errors.
#[test]
fn unknown_and_unsupported_keys_are_explicit() {
    let mut owner = SheetOwner::new(
        Rc::new(|| {
            build_sheet(
                "sheet s { cell pair: (i32, bool) = (1, true); }",
                "compound.adm2",
            )
        }),
        "compound/v1".into(),
    )
    .unwrap();
    assert_eq!(owner.views().len(), 1);
    assert_eq!(owner.views()[0].text, "(1, true)");
    assert!(!owner.views()[0].issues.is_empty());
    assert!(matches!(
        owner.apply(&[edit("absent", "1")], UpdateOrigin::User),
        HostOutcome::Rejected(HostRejection::Binding(BindingError::UnknownKey(_)))
    ));
    owner.restore_accepted().unwrap();
    assert!(matches!(
        owner.apply(&[edit("pair", "(2, false)")], UpdateOrigin::User),
        HostOutcome::Rejected(HostRejection::Binding(BindingError::UnsupportedCodec(_)))
    ));
}

/// Exact capture preserves primitive endpoints and unrounded floating-point values.
#[test]
fn all_primitive_codecs_round_trip_exactly() {
    /// Checks capture/prepare against an independently supplied canonical value.
    fn check<T>(value: T, canonical: &str)
    where
        T: std::any::Any + std::fmt::Display + std::str::FromStr + PartialEq + std::fmt::Debug,
        T::Err: std::fmt::Display + 'static,
    {
        let mut sheet = adam_rs::Sheet::new();
        let id = sheet.add_cell(value);
        let mut labels = crate::Labels::new();
        labels.add_cell::<T>(id, "value");
        let meta = &labels.cells[&id];
        assert_eq!((meta.capture)(&sheet).unwrap(), canonical);
        (meta.prepare)(canonical)
            .unwrap()
            .apply(&mut sheet, id)
            .unwrap();
        assert_eq!((meta.capture)(&sheet).unwrap(), canonical);
    }
    check(i8::MIN, "-128");
    check(i16::MIN, "-32768");
    check(i32::MIN, "-2147483648");
    check(i64::MAX, "9223372036854775807");
    check(i128::MIN, "-170141183460469231731687303715884105728");
    check(-123_isize, "-123");
    check(u8::MAX, "255");
    check(u16::MAX, "65535");
    check(u32::MAX, "4294967295");
    check(u64::MAX, "18446744073709551615");
    check(u128::MAX, "340282366920938463463374607431768211455");
    check(123_usize, "123");
    check(1.2345679_f32, "1.2345679");
    check(1.2345678901234567_f64, "1.2345678901234567");
    check(-0.0_f64, "-0");
    check(true, "true");
    check("raw\n\"text\"".to_string(), "raw\n\"text\"");
}

/// A custom codec supports scalar types without Display or FromStr.
#[test]
fn custom_scalar_codec_preserves_values() {
    /// Holds a custom application scalar.
    #[derive(Debug, PartialEq)]
    struct Token(u8);
    let mut sheet = adam_rs::Sheet::new();
    let id = sheet.add_cell(Token(7));
    let mut labels = crate::Labels::new();
    labels.add_cell_with_codec(
        id,
        "token",
        crate::ScalarCodec {
            encode: |v: &Token| format!("token:{}", v.0),
            decode: |s| {
                s.strip_prefix("token:")
                    .and_then(|v| v.parse().ok())
                    .map(Token)
                    .ok_or_else(|| crate::input::InputError::Lexical(s.into()))
            },
        },
    );
    let meta = &labels.cells[&id];
    assert_eq!((meta.capture)(&sheet).unwrap(), "token:7");
    (meta.prepare)("token:9")
        .unwrap()
        .apply(&mut sheet, id)
        .unwrap();
    assert_eq!(sheet.read::<Token>(id).unwrap(), &Token(9));
    assert!((meta.prepare)("wrong").is_err());
}

/// Range metadata never transports wide integers through binary64.
#[test]
fn exact_range_and_rounded_display_are_separate_from_capture() {
    let built = build_sheet("sheet s {
        cell wide: u64 = 18446744073709551615u64 filter 9007199254740993u64..=18446744073709551615u64;
        cell float: f64 = 1.2345678901234567;
    }", "exact.adm2").unwrap();
    let wide = &built.labels.cells[&built.keys["wide"]];
    let range = wide.exact_range.as_ref().unwrap()(&built.parsed).unwrap();
    assert_eq!(range.min, "9007199254740993");
    assert_eq!(range.max, "18446744073709551615");
    let float = &built.labels.cells[&built.keys["float"]];
    assert_eq!((float.display)(&built.parsed), "1.23");
    assert_eq!(
        (float.capture)(&built.parsed).unwrap(),
        "1.2345678901234567"
    );
}

/// Malformed syntax differs from concrete primitive overflow.
#[test]
fn scalar_conversion_errors_keep_their_classification() {
    let mut sheet = adam_rs::Sheet::new();
    let id = sheet.add_cell(1_i8);
    let mut labels = crate::Labels::new();
    labels.add_cell::<i8>(id, "small");
    let meta = &labels.cells[&id];
    assert!(matches!(
        (meta.prepare)("bad"),
        Err(crate::input::InputError::Lexical(_))
    ));
    assert!(matches!(
        (meta.prepare)("128"),
        Err(crate::input::InputError::TypeRange {
            kind: crate::input::NumberKind::Signed(8),
            ..
        })
    ));
    assert_eq!(*sheet.read::<i8>(id).unwrap(), 1);
}

/// Conversion of later edits prevents even the first prepared write from running.
#[test]
fn conversion_of_all_edits_precedes_the_first_write() {
    let factory = Rc::new(|| {
        let mut built = model()?;
        built
            .labels
            .cells
            .get_mut(&built.keys["limited"])
            .unwrap()
            .prepare = Box::new(|_| Ok(PreparedWrite::new(1_u8)));
        Ok(built)
    });
    let mut owner = SheetOwner::new(factory, "prepare/v1".into()).unwrap();
    assert!(matches!(
        owner.apply(
            &[edit("limited", "7"), edit("left", "bad")],
            UpdateOrigin::User
        ),
        HostOutcome::Rejected(HostRejection::Input(_))
    ));
}

/// Public views distinguish derived editability, forced values, and disconnected cells.
#[test]
fn published_flags_follow_model_capabilities() {
    let factory = Rc::new(|| {
        build_sheet(
            "sheet s {
        cell left: i32 = 1; cell right: i32 = 1; cell unused: bool = false;
        cell forced: i32 = 0;
        relationship { left := right; right := left; }
        relationship { forced := 3; }
        out result: i32 := left + forced;
    }",
            "flags.adm2",
        )
    });
    let mut owner = SheetOwner::new(factory, "flags/v1".into()).unwrap();
    let derived = owner
        .views()
        .iter()
        .find(|view| (view.key == "left" || view.key == "right") && !view.is_source)
        .unwrap();
    assert!(!derived.flags.disabled);
    assert!(!derived.flags.readonly);
    assert!(derived.flags.relevant);
    let forced = owner.views().iter().find(|v| v.key == "forced").unwrap();
    assert!(forced.flags.disabled && forced.flags.forced);
    let unused = owner.views().iter().find(|v| v.key == "unused").unwrap();
    assert!(unused.flags.disabled && !unused.flags.relevant);
    let output = owner.views().iter().find(|v| v.key == "result").unwrap();
    assert!(output.flags.readonly);
    assert!(matches!(
        owner.apply(&[edit("result", "2")], UpdateOrigin::User),
        HostOutcome::Rejected(HostRejection::Binding(BindingError::IncompatibleEditor(_)))
    ));
}

/// Unknown scalar types stay in schema order and expose explicit capability diagnostics.
#[test]
fn unsupported_named_scalar_is_not_omitted() {
    /// Represents a host-defined scalar without a registered codec.
    #[derive(PartialEq)]
    struct Opaque;
    let factory = Rc::new(|| {
        let mut built = model()?;
        let id = built.parsed.add_cell(Opaque);
        built.parsed.cell_names.insert(
            "opaque".into(),
            (
                id,
                adam_lang::type_registry::TypeShape::Named(std::any::TypeId::of::<Opaque>()),
            ),
        );
        built.keys.insert("opaque".into(), id);
        built.labels = crate::labels_from_cell_names(&built.parsed, &built.parsed.cell_names);
        built.parsed.propagate().unwrap();
        Ok(built)
    });
    let owner = SheetOwner::new(factory, "opaque/v1".into()).unwrap();
    let view = owner.views().last().unwrap();
    assert_eq!(view.key, "opaque");
    assert!(view.flags.invalid);
    assert!(view.issues[0].message.contains("unsupported"));
    assert!(
        owner
            .inspect(|p, labels| (labels.cells[&p.cell_names["opaque"].0].capture)(p).is_err())
            .unwrap()
    );
}

/// Publication errors reject rather than substituting display text.
#[test]
fn capture_errors_never_fall_back_to_display() {
    let captures = Rc::new(Cell::new(0));
    let counter = captures.clone();
    let factory = Rc::new(move || {
        let mut built = model()?;
        let counter = counter.clone();
        built
            .labels
            .cells
            .get_mut(&built.keys["left"])
            .unwrap()
            .capture = Box::new(move |_| {
            counter.set(counter.get() + 1);
            if counter.get() == 1 {
                Ok("1".into())
            } else {
                Err(crate::input::InputError::UnsupportedEditor(
                    "encoder unavailable".into(),
                ))
            }
        });
        Ok(built)
    });
    let mut owner = SheetOwner::new(factory, "capture/v1".into()).unwrap();
    assert!(matches!(
        owner.apply(&[edit("left", "2")], UpdateOrigin::User),
        HostOutcome::Rejected(HostRejection::Input(_))
    ));
    assert_eq!(
        owner.views().iter().find(|v| v.key == "left").unwrap().text,
        "1"
    );
    assert!(owner.accepted_state().transactions.is_empty());
    assert!(owner.inspect(|_, _| ()).is_err());
}

/// Replay propagates groups separately and keeps accepted publication on replay failure.
#[test]
fn replay_validation_failure_preserves_history_and_provenance() {
    let count = Rc::new(Cell::new(0));
    let calls = count.clone();
    let factory = Rc::new(move || {
        calls.set(calls.get() + 1);
        let limit = if calls.get() == 1 { 100 } else { 5 };
        build_sheet(
            &format!("sheet s {{ cell x: i32 = 1 require {{ @limit x < {limit}; }}; }}"),
            "replay.adm2",
        )
    });
    let mut owner = SheetOwner::new(factory, "replay/v1".into()).unwrap();
    assert!(matches!(
        owner.apply(&[edit("x", "7")], UpdateOrigin::Recovery),
        HostOutcome::Accepted
    ));
    let before = owner.views()[0].clone();
    let _ = owner.apply(&[edit("x", "101")], UpdateOrigin::User);
    let error = owner.restore_accepted().unwrap_err();
    assert!(matches!(error, BuildError::Validation { .. }));
    let rendered = error.render(&crate::Renderer::plain());
    assert!(rendered.contains("replay.adm2"), "{rendered}");
    assert!(rendered.contains("@limit"), "{rendered}");
    assert!(Rc::ptr_eq(&before, &owner.views()[0]));
    assert_eq!(owner.accepted_state().transactions.len(), 1);
    assert!(owner.inspect(|_, _| ()).is_err());
}

/// A fatal replay permanently forbids subsequent factory calls.
#[test]
fn fatal_during_replay_retires_restoration() {
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let factory = Rc::new(move || {
        count.set(count.get() + 1);
        let denominator = if count.get() == 1 { "x" } else { "2 - x" };
        build_sheet(
            &format!("sheet s {{ cell x: i32 = 1; out y: i32 := 10 / ({denominator}); }}"),
            "replay-fatal.adm2",
        )
    });
    let mut owner = SheetOwner::new(factory, "fatal-replay/v1".into()).unwrap();
    assert!(matches!(
        owner.apply(&[edit("x", "2")], UpdateOrigin::User),
        HostOutcome::Accepted
    ));
    assert!(matches!(
        owner.restore_accepted(),
        Err(BuildError::Runtime(_))
    ));
    assert!(owner.restore_accepted().is_err());
    assert_eq!(calls.get(), 2);
    assert!(owner.inspect(|_, _| ()).is_err());
}

/// Empty models and empty groups need no output or synthetic history entry.
#[test]
fn empty_model_and_transaction_are_valid() {
    let mut owner = SheetOwner::new(
        Rc::new(|| build_sheet("sheet s {}", "empty.adm2")),
        "empty/v1".into(),
    )
    .unwrap();
    assert!(owner.views().is_empty());
    assert!(matches!(
        owner.apply(&[], UpdateOrigin::User),
        HostOutcome::Accepted
    ));
    assert!(owner.accepted_state().transactions.is_empty());
    assert!(owner.inspect(|_, _| ()).is_ok());
}

/// Publications share unchanged cells while preserving old accepted snapshots.
#[test]
fn accepted_publication_reuses_unchanged_views() {
    let (mut owner, _) = owner();
    let valid = owner
        .views()
        .iter()
        .find(|view| view.key == "valid")
        .unwrap()
        .clone();
    let limited = owner
        .views()
        .iter()
        .find(|view| view.key == "limited")
        .unwrap()
        .clone();
    assert!(matches!(
        owner.apply(&[edit("limited", "8")], UpdateOrigin::User),
        HostOutcome::Accepted
    ));
    assert!(Rc::ptr_eq(
        &valid,
        owner
            .views()
            .iter()
            .find(|view| view.key == "valid")
            .unwrap()
    ));
    assert_eq!(limited.text, "5");
    assert_eq!(
        owner
            .views()
            .iter()
            .find(|view| view.key == "limited")
            .unwrap()
            .text,
        "8"
    );
}

/// Accepted groups restore last-writer direction without replaying rejected writes.
#[test]
fn ordered_replay_preserves_write_strength_across_groups() {
    let (mut owner, _) = owner();
    for (key, value) in [("right", "7"), ("left", "9")] {
        assert!(matches!(
            owner.apply(&[edit(key, value)], UpdateOrigin::User),
            HostOutcome::Accepted
        ));
    }

    let _ = owner.apply(
        &[edit("right", "11"), edit("valid", "false")],
        UpdateOrigin::Playback,
    );
    owner.restore_accepted().unwrap();
    owner
        .inspect(|p, _| {
            assert!(p.is_source(p.cell_names["left"].0));
            assert!(!p.is_source(p.cell_names["right"].0));
            assert_eq!(*p.read::<i32>(p.cell_names["right"].0).unwrap(), 9);
        })
        .unwrap();
    assert_eq!(owner.accepted_state().transactions.len(), 2);
}

/// Recovery diagnostics retain authored requirement spans after discarding the model.
#[test]
fn recovery_rendering_keeps_validation_source_labels() {
    let (mut owner, _) = owner();
    let HostOutcome::Rejected(rejection) =
        owner.apply(&[edit("valid", "false")], UpdateOrigin::User)
    else {
        panic!("expected validation rejection")
    };
    let error = BuildError::Recovery(Box::new(rejection));
    let rendered = error.render(&crate::Renderer::plain());
    assert!(rendered.contains("@accepted valid;"), "{rendered}");
}

/// Replay write failure remains a mutation rejection, not a fatal propagation error.
#[test]
fn replay_write_failure_reports_typed_mutation() {
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let factory = Rc::new(move || {
        count.set(count.get() + 1);
        let mut built = model()?;
        if count.get() > 1 {
            built
                .labels
                .cells
                .get_mut(&built.keys["left"])
                .unwrap()
                .prepare = Box::new(|_| Ok(PreparedWrite::new(1_u8)));
        }
        Ok(built)
    });
    let mut owner = SheetOwner::new(factory, "replay-write/v1".into()).unwrap();
    assert!(matches!(
        owner.apply(&[edit("left", "7")], UpdateOrigin::User),
        HostOutcome::Accepted
    ));
    let _ = owner.apply(&[edit("valid", "false")], UpdateOrigin::User);
    let error = owner.restore_accepted().unwrap_err();
    let BuildError::Recovery(rejection) = error else {
        panic!("expected typed replay rejection")
    };
    assert!(
        matches!(rejection.as_ref(), HostRejection::Mutation(failure)
            if matches!(failure.error, adam_rs::Error::TypeMismatch { .. }))
    );
    assert!(owner.inspect(|_, _| ()).is_err());
    assert_eq!(owner.accepted_state().transactions.len(), 1);
    assert_eq!(
        owner
            .views()
            .iter()
            .find(|view| view.key == "left")
            .unwrap()
            .text,
        "7"
    );
}
