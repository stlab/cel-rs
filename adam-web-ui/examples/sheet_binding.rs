//! Runs the standalone named-root binding demo described in `docs/dioxus-integration.md`.
//!
//! Run `cargo run -p adam-web-ui --example sheet-binding --features desktop`.
//! Edit Area to 24: Width becomes 8 with Height fixed at 3. Capture, then replay
//! into a fresh owner. See the integration guide for failure and recovery policy.

use adam_lang::{ModuleCompiler, ModulePath, ModuleSet, TypeRegistry};
use adam_web_ui::{
    BuildError, Renderer, ScalarCodec,
    binding::{
        CellEdit, CheckBox, EditNumber, EditSlider, EditText, HostOutcome, HostRejection,
        ModelFactory, PlaybackError, Recording, RecoveryState, SheetEvent, SheetHandle, SheetOwner,
        UpdateOrigin, use_sheet,
    },
    components::Theme,
    initialize_sheet,
};
use cel_parser::{OpLookup, OperationSignature, ResolvedLeafType};
use cel_runtime::ValueType;
use dioxus::prelude::*;
use serde::Serialize;
use std::{io::Write, path::Path, rc::Rc};

/// Identifies the selected graph, initial state, and exact codec contract.
const SCHEMA: &str = "sheet-binding/demo/v1";
/// Contains both roots, including the deliberately unselected root.
const SOURCE: &str = include_str!("sheet_binding.adm2");
/// Names the recovery document relative to the process launch directory.
const RECOVERY_FILE: &str = "sheet-binding-recovery.json";

/// Stores an application scalar with exact unsigned integer encoding.
#[derive(Clone, Debug, PartialEq)]
struct Badge(u32);

impl Default for Badge {
    /// Returns the fixture's initial application value.
    fn default() -> Self {
        Self(7)
    }
}

/// Encodes the application scalar exactly in decimal notation.
///
/// - Complexity: O(decimal digits) time and space.
fn encode_badge(value: &Badge) -> String {
    value.0.to_string()
}

/// Decodes the complete application scalar, rejecting malformed or overflowing text.
///
/// # Errors
/// Returns a lexical diagnostic when text is not an exact `u32`.
/// - Complexity: O(text bytes).
fn decode_badge(text: &str) -> Result<Badge, adam_web_ui::input::InputError> {
    text.parse()
        .map(Badge)
        .map_err(|error: std::num::ParseIntError| {
            adam_web_ui::input::InputError::Lexical(error.to_string())
        })
}

/// Compiles the application modules and returns the selected-root factory.
///
/// # Errors
/// Returns registration or compilation diagnostics. The returned factory reports
/// root-selection and initialization failures.
/// - Complexity: Module compilation cost plus retained compiled source size.
fn demo_factory() -> anyhow::Result<ModelFactory> {
    let mut types = TypeRegistry::new();
    types.register::<Badge>("Badge");
    let mut lookup = OpLookup::new();
    lookup.register_operation(OperationSignature {
        name: "badge_number",
        operands: vec![ResolvedLeafType::new("Badge", ValueType::leaf::<Badge>()).into()],
        result: ResolvedLeafType::new("u32", ValueType::leaf::<u32>()).into(),
        emit: Box::new(|segment, _| segment.op1(|badge: Badge| badge.0)),
    })?;
    let module: ModulePath = "consumer".parse().map_err(|error| {
        anyhow::anyhow!(
            "{}",
            adam_web_ui::format_module_diagnostic(&error, &Renderer::plain())
        )
    })?;
    let mut modules = ModuleSet::new();
    modules
        .insert_source(module.clone(), "sheet_binding.adm2", SOURCE)
        .map_err(|error| {
            anyhow::anyhow!(
                "{}",
                adam_web_ui::format_module_diagnostic(&error, &Renderer::plain())
            )
        })?;
    let compiled = ModuleCompiler::new(types, lookup)
        .compile(modules)
        .map_err(|errors| {
            let mut message = String::new();
            // Preserve every independently owned compilation diagnostic.
            for error in &errors {
                message.push_str(&adam_web_ui::format_module_diagnostic(
                    error,
                    &Renderer::plain(),
                ));
                message.push('\n');
            }
            anyhow::anyhow!(message)
        })?;
    Ok(Rc::new(move || {
        let parsed = compiled
            .instantiate_root(&module, "demo")
            .map_err(BuildError::Module)?;
        let mut built = initialize_sheet(parsed)?;
        built.labels.add_cell_with_codec(
            built.keys["badge"],
            "badge",
            ScalarCodec {
                encode: encode_badge,
                decode: decode_badge,
            },
        );
        Ok(built)
    }))
}

/// Chooses an application action without performing recovery or terminating.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Policy {
    /// Reports acceptance, configuration/input errors, and accepted recording failures.
    Report,
    /// Explicitly reconstructs the last accepted aspirations.
    Restore,
    /// Persists accepted recovery inputs and terminates without reconstruction.
    Terminate,
}

/// Selects recovery for discarded model rejections, never for input/configuration errors.
fn policy(outcome: &HostOutcome) -> Policy {
    match outcome {
        HostOutcome::Rejected(HostRejection::Validation(_) | HostRejection::Mutation(_)) => {
            Policy::Restore
        }
        HostOutcome::Fatal(_) => Policy::Terminate,
        HostOutcome::Accepted
        | HostOutcome::RecordingFailed(_)
        | HostOutcome::Rejected(HostRejection::Input(_) | HostRejection::Binding(_)) => {
            Policy::Report
        }
    }
}

/// Borrows accepted source/configuration and ordered aspirations for durable recovery.
///
/// Contains no executable closures, runtime cell IDs, or effective-value recording.
#[derive(Serialize)]
struct RecoveryDocument<'a> {
    source: &'a str,
    file_name: &'a str,
    module: &'a str,
    root: &'a str,
    schema_key: &'a str,
    transactions: &'a [Vec<CellEdit>],
}

/// Serializes accepted aspirations without copying the journal.
///
/// # Errors
/// Returns serialization or destination-write errors.
/// - Complexity: O(source and accepted input bytes).
fn write_recovery(writer: impl Write, accepted: &RecoveryState) -> serde_json::Result<()> {
    serde_json::to_writer_pretty(
        writer,
        &RecoveryDocument {
            source: SOURCE,
            file_name: "sheet_binding.adm2",
            module: "consumer",
            root: "demo",
            schema_key: &accepted.schema_key,
            transactions: &accepted.transactions,
        },
    )
}

/// Writes recovery JSON to the launch directory, reporting creation/write/flush failures.
///
/// # Errors
/// Returns persistence errors without substituting an empty document.
/// - Complexity: O(source and accepted input bytes).
fn save_recovery(accepted: &RecoveryState) -> anyhow::Result<()> {
    let mut file = std::fs::File::create(Path::new(RECOVERY_FILE))?;
    write_recovery(&mut file, accepted)?;
    file.sync_all()?;
    Ok(())
}

/// Reports persistence status and exits nonzero; never reconstructs a retired model.
///
/// - Complexity: O(persistence diagnostic bytes).
fn terminate(persistence: anyhow::Result<()>) -> ! {
    match persistence {
        Ok(()) => eprintln!("Accepted recovery saved to {RECOVERY_FILE}"),
        Err(error) => eprintln!("Recovery persistence failed for {RECOVERY_FILE}: {error:#}"),
    }
    std::process::exit(1)
}

/// Reports a structured build failure and saves accepted inputs on fatal propagation.
///
/// - Complexity: Diagnostic rendering and recovery persistence cost.
fn report_build(error: BuildError, sheet: SheetHandle, mut status: Signal<String>) {
    let fatal = matches!(error, BuildError::Runtime(_));
    let message = error.render(&Renderer::plain());
    eprintln!("{message}");
    status.set(message);
    if fatal {
        terminate(
            sheet
                .with_accepted_state(save_recovery)
                .map_err(anyhow::Error::from)
                .and_then(|saved| saved),
        );
    }
}

/// Handles host outcomes after the owner borrow has been released.
///
/// Recovery is explicit for validation/mutation rejection. Recording failure is
/// already accepted, so this handler does not undo its publication or history.
/// - Complexity: Diagnostic rendering and optional reconstruction/persistence cost.
fn handle_event(event: SheetEvent, mut status: Signal<String>) {
    let action = policy(&event.outcome);
    let message = match event.outcome {
        HostOutcome::Accepted => "Accepted".into(),
        HostOutcome::RecordingFailed(error) => format!("Accepted; recording stopped: {error:?}"),
        HostOutcome::Rejected(error) => {
            BuildError::Recovery(Box::new(error)).render(&Renderer::plain())
        }
        HostOutcome::Fatal(error) => {
            BuildError::Runtime(Box::new(error)).render(&Renderer::plain())
        }
    };
    eprintln!("{message}");
    status.set(message);
    match action {
        Policy::Report => {}
        Policy::Restore => {
            if let Err(error) = event.sheet.restore_accepted() {
                report_build(error, event.sheet, status);
            }
        }
        Policy::Terminate => terminate(
            event
                .sheet
                .with_accepted_state(save_recovery)
                .map_err(anyhow::Error::from)
                .and_then(|saved| saved),
        ),
    }
}

/// Reports playback errors and treats fresh-candidate propagation failure as fatal.
///
/// - Complexity: Diagnostic rendering and optional recovery persistence cost.
fn report_playback(error: PlaybackError, sheet: SheetHandle, mut status: Signal<String>) {
    match error {
        PlaybackError::Fatal(error) => {
            report_build(BuildError::Runtime(Box::new(error)), sheet, status)
        }
        PlaybackError::Build(error) => report_build(error, sheet, status),
        PlaybackError::Rejected(error) => {
            report_build(BuildError::Recovery(Box::new(error)), sheet, status)
        }
        error => {
            let message = format!("Playback rejected: {error:?}");
            eprintln!("{message}");
            status.set(message);
        }
    }
}

/// Starts accepted-user-action capture and reports capability/availability errors.
///
/// - Complexity: Effective-source capture cost.
fn start_recording(sheet: SheetHandle, mut status: Signal<String>) {
    match sheet.start_recording(&[]) {
        Ok(()) => status.set("Recording accepted user actions".into()),
        Err(error) => status.set(format!("Cannot record: {error:?}")),
    }
}

/// Retains one shared effective-source recording without copying accepted history.
///
/// - Complexity: Effective-source capture and encoding cost.
fn capture_recording(
    sheet: SheetHandle,
    mut captured: Signal<Option<Rc<Recording>>>,
    mut status: Signal<String>,
) {
    match sheet.capture(&[]) {
        Ok(recording) => {
            captured.set(Some(Rc::new(recording)));
            status.set("Captured effective sources".into());
        }
        Err(error) => status.set(format!("Capture failed: {error:?}")),
    }
}

/// Replays the captured sources into a fresh owner, or does nothing without a capture.
///
/// - Complexity: Fresh factory, transaction, and publication cost.
fn play_recording(
    sheet: SheetHandle,
    factory: ModelFactory,
    captured: Signal<Option<Rc<Recording>>>,
    mut status: Signal<String>,
) {
    let recording = captured.read().as_ref().cloned();
    if let Some(recording) = recording {
        match sheet.playback(factory, SCHEMA.into(), &recording) {
            Ok(()) => status.set("Fresh playback; zero user recordings".into()),
            Err(error) => report_playback(error, sheet, status),
        }
    }
}

/// Requests explicit accepted-aspiration restoration and reports reconstruction errors.
///
/// - Complexity: Factory and accepted-history replay cost.
fn restore_accepted(sheet: SheetHandle, status: Signal<String>) {
    if let Err(error) = sheet.restore_accepted() {
        report_build(error, sheet, status);
    }
}

/// Renders explicit controls over the one selected, event-loop-confined owner.
///
/// - Complexity: Named-cell publication plus child rendering cost per revision.
#[component]
fn App() -> Element {
    let factory = use_hook(|| demo_factory().map_err(Rc::new));
    let factory = match factory {
        Ok(factory) => factory,
        Err(error) => {
            eprintln!("{error:#}");
            return rsx! { pre { role: "alert", "{error:#}" } };
        }
    };
    let status = use_signal(|| "Ready".to_owned());
    let captured = use_signal::<Option<Rc<Recording>>>(|| None);
    let sheet = match use_sheet(
        factory.clone(),
        SCHEMA.into(),
        EventHandler::new(move |event| handle_event(event, status)),
    ) {
        Ok(sheet) => sheet,
        Err(error) => {
            let message = error.render(&Renderer::plain());
            eprintln!("{message}");
            if matches!(error.as_ref(), BuildError::Runtime(_)) {
                terminate(save_recovery(&RecoveryState {
                    schema_key: SCHEMA.into(),
                    transactions: vec![],
                }));
            }
            return rsx! { pre { role: "alert", "{message}" } };
        }
    };
    let history = sheet.with_accepted_state(|accepted| accepted.transactions.len());
    rsx! {
        Theme {
            main { style: "max-width:48rem;padding:2rem;margin:auto;display:grid;gap:1rem",
                h1 { "Sheet binding consumer" }
                p { "Height is a fixed source. Edit Area to 24: Width becomes 8." }
                div { id: "width", EditNumber { bind: sheet.cell("width"), label: "Width" } }
                div { id: "height", EditNumber { bind: sheet.cell("height"), label: "Height" } }
                div { id: "area", EditNumber { bind: sheet.cell("area"), label: "Area", locale: "de-DE".to_owned() } }
                div { id: "rectangle", CheckBox { bind: sheet.cell("rectangle"), label: "Rectangle (off: sum)" } }
                div { id: "limited", EditSlider { bind: sheet.cell("limited"), label: "Limited aspiration (try 20)" } }
                div { id: "ceiling", EditNumber { bind: sheet.cell("ceiling"), label: "Ceiling (try 30)" } }
                div { id: "badge", EditText { bind: sheet.cell("badge"), label: "Application badge (u32)" } }
                div { style: "display:flex;gap:1rem",
                    button { id: "record", onclick: move |_| start_recording(sheet, status), "Start recording" }
                    button { id: "capture", onclick: move |_| capture_recording(sheet, captured, status), "Capture" }
                    button { id: "playback", disabled: captured.read().is_none(),
                        onclick: move |_| play_recording(sheet, factory.clone(), captured, status), "Fresh playback" }
                    button { id: "restore", onclick: move |_| restore_accepted(sheet, status), "Restore accepted inputs" }
                }
                p { id: "history", "Accepted input groups: {history:?}" }
                p { id: "recording-count", "Last user recording: {sheet.last_recording().is_some()}" }
                pre { id: "status", role: "status", style: "white-space:pre-wrap", "{status}" }
                details {
                    summary { "Deliberate fatal runtime test" }
                    p { "Division by zero saves accepted recovery JSON and exits with status 1." }
                    button { id: "fatal", onclick: move |_| sheet.apply(
                        &[CellEdit { key: "divisor".into(), canonical: "0".into() }], UpdateOrigin::User),
                        "Inject runtime failure"
                    }
                }
            }
        }
    }
}

/// Runs the fatal boundary in a subprocess without launching a desktop renderer.
///
/// # Errors
/// Returns construction errors; the deliberate fatal path exits with status 1.
/// - Complexity: Compilation, three transactions, and persistence cost.
fn fatal_check(factory: ModelFactory) -> anyhow::Result<()> {
    let mut owner = SheetOwner::new(factory, SCHEMA.into())
        .map_err(|error| anyhow::anyhow!("{}", error.render(&Renderer::plain())))?;
    for (key, canonical) in [("area", "24"), ("limited", "20"), ("divisor", "0")] {
        let outcome = owner.apply(
            &[CellEdit {
                key: key.into(),
                canonical: canonical.into(),
            }],
            UpdateOrigin::User,
        );
        if policy(&outcome) == Policy::Terminate {
            if let HostOutcome::Fatal(error) = outcome {
                eprintln!(
                    "{}",
                    BuildError::Runtime(Box::new(error)).render(&Renderer::plain())
                );
            }
            terminate(save_recovery(owner.accepted_state()));
        }
        anyhow::ensure!(
            matches!(outcome, HostOutcome::Accepted),
            "fatal check prerequisite rejected: {outcome:?}"
        );
    }
    anyhow::bail!("fault injection did not produce fatal propagation")
}

/// Launches the standalone desktop consumer or the explicit fatal subprocess check.
///
/// # Errors
/// Returns setup errors in fatal-check mode; the desktop root reports its own setup errors.
/// - Complexity: Compilation plus application lifetime.
fn main() -> anyhow::Result<()> {
    if std::env::args().any(|arg| arg == "--fatal-check") {
        return fatal_check(demo_factory()?);
    }
    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            dioxus::desktop::Config::new().with_window(
                dioxus::desktop::WindowBuilder::new()
                    .with_title("Sheet binding consumer")
                    .with_inner_size(dioxus::desktop::LogicalSize::new(900.0, 1000.0)),
            ),
        )
        .launch(App);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use adam_web_ui::binding::{CellEdit, HostOutcome, SheetOwner, UpdateOrigin};

    /// Exposes scope-owned handles while their virtual DOM remains alive.
    #[derive(Clone, Copy)]
    struct Host {
        sheet: SheetHandle,
        status: Signal<String>,
        captured: Signal<Option<Rc<Recording>>>,
    }

    /// Mounts the example's real host policy without starting a renderer.
    ///
    /// - Complexity: Fixture compilation and initialization cost.
    fn host() -> Element {
        let slot: Rc<std::cell::RefCell<Option<Host>>> = consume_context();
        let status = use_signal(String::new);
        let captured = use_signal(|| None);
        let sheet = use_sheet(
            demo_factory().unwrap(),
            SCHEMA.into(),
            EventHandler::new(move |event| handle_event(event, status)),
        )
        .unwrap();
        *slot.borrow_mut() = Some(Host {
            sheet,
            status,
            captured,
        });
        rsx! {}
    }

    /// Exercises the application callback, action controls, and explicit restore boundary.
    #[test]
    fn demo_host_actions_and_error_reporting() {
        let slot = Rc::new(std::cell::RefCell::new(None::<Host>));
        let mut dom = VirtualDom::new(host);
        dom.provide_root_context(slot.clone());
        dom.rebuild_in_place();
        let Host {
            sheet,
            mut status,
            mut captured,
        } = slot.borrow().unwrap();
        start_recording(sheet, status);
        assert!(status.peek().contains("Recording"));
        sheet.apply(&[edit("area", "24")], UpdateOrigin::User);
        assert_eq!(sheet.cell("width").view().unwrap().text, "8");
        capture_recording(sheet, captured, status);
        assert!(captured.peek().is_some());
        assert!(sheet.last_recording().is_some());
        play_recording(sheet, demo_factory().unwrap(), captured, status);
        assert_eq!(sheet.cell("width").view().unwrap().text, "8");
        assert!(sheet.last_recording().is_none());
        assert!(status.peek().contains("zero"));
        sheet.apply(&[edit("area", "200")], UpdateOrigin::User);
        assert!(status.peek().contains("area_limit"));
        assert!(sheet.inspect(|_, _| ()).is_ok());
        assert_eq!(sheet.cell("area").view().unwrap().text, "24");
        sheet.apply(&[edit("area", "invalid")], UpdateOrigin::User);
        assert!(sheet.inspect(|_, _| ()).is_err());
        let rejected = status.peek().clone();
        start_recording(sheet, status);
        assert!(status.peek().contains("Cannot record"));
        capture_recording(sheet, captured, status);
        assert!(status.peek().contains("Capture failed"));
        restore_accepted(sheet, status);
        assert!(sheet.inspect(|_, _| ()).is_ok());
        assert!(!rejected.is_empty());
        captured.set(None);
        status.set("unchanged".into());
        play_recording(sheet, demo_factory().unwrap(), captured, status);
        assert_eq!(&*status.peek(), "unchanged");
        captured.set(Some(Rc::new(Recording {
            schema_key: "wrong".into(),
            values: vec![],
        })));
        play_recording(sheet, demo_factory().unwrap(), captured, status);
        assert!(status.peek().contains("SchemaMismatch"));
        let mut rejected_owner = SheetOwner::new(demo_factory().unwrap(), SCHEMA.into()).unwrap();
        let HostOutcome::Rejected(rejection) =
            rejected_owner.apply(&[edit("area", "200")], UpdateOrigin::User)
        else {
            panic!("fixture must reject its named area requirement");
        };
        report_playback(PlaybackError::Rejected(rejection), sheet, status);
        assert!(status.peek().contains(" --> sheet_binding.adm2"));
        assert!(status.peek().contains("area_limit"));
        report_build(
            BuildError::Recovery(Box::new(HostRejection::Binding(
                adam_web_ui::binding::BindingError::UnknownKey("missing".into()),
            ))),
            sheet,
            status,
        );
        assert!(status.peek().contains("missing"));
        handle_event(
            SheetEvent {
                sheet,
                outcome: HostOutcome::RecordingFailed(
                    adam_web_ui::binding::CaptureError::UnnamedSource,
                ),
            },
            status,
        );
        assert!(status.peek().contains("Accepted; recording stopped"));
        assert!(sheet.inspect(|_, _| ()).is_ok());
        let html = dioxus_ssr::render_element(rsx! { App {} });
        assert!(html.contains("Sheet binding consumer"));
        assert!(html.contains("Inject runtime failure"));
    }

    /// Replays effective sources of the selected root without recording feedback.
    #[test]
    fn demo_recording_replays_selected_root() {
        let factory = demo_factory().unwrap();
        let mut owner = SheetOwner::new(factory.clone(), SCHEMA.into()).unwrap();
        assert_eq!(text(&owner, "width"), "4");
        assert_eq!(text(&owner, "height"), "3");
        assert_eq!(text(&owner, "area"), "12");
        owner.start_recording(&[]).unwrap();
        assert!(matches!(
            owner.apply(&[edit("area", "24")], UpdateOrigin::User),
            HostOutcome::Accepted
        ));
        assert_eq!(text(&owner, "width"), "8");
        assert!(matches!(
            owner.apply(&[edit("limited", "20")], UpdateOrigin::User),
            HostOutcome::Accepted
        ));
        assert_eq!(text(&owner, "limited"), "10");
        assert_eq!(owner.accepted_state().transactions[1][0].canonical, "20");
        let recording = owner.capture(&[]).unwrap();
        assert!(
            recording
                .values
                .iter()
                .any(|e| e.key == "limited" && e.canonical == "10")
        );
        let replayed = SheetOwner::playback(factory, SCHEMA.into(), &recording).unwrap();
        assert_eq!(text(&replayed, "width"), "8");
        assert_eq!(text(&replayed, "area"), "24");
        assert_eq!(text(&replayed, "limited"), "10");
        assert!(replayed.recordings().is_empty());
    }

    /// Retains aspirations across filtering and rejects a named requirement before publication.
    #[test]
    fn demo_filter_requirement_and_conditional_contract() {
        let mut owner = SheetOwner::new(demo_factory().unwrap(), SCHEMA.into()).unwrap();
        assert!(matches!(
            owner.apply(&[edit("limited", "20")], UpdateOrigin::User),
            HostOutcome::Accepted
        ));
        assert_eq!(text(&owner, "limited"), "10");
        assert!(matches!(
            owner.apply(&[edit("ceiling", "30")], UpdateOrigin::User),
            HostOutcome::Accepted
        ));
        assert_eq!(text(&owner, "limited"), "20");
        assert!(
            matches!(owner.apply(&[edit("area", "200")], UpdateOrigin::User),
            HostOutcome::Rejected(adam_web_ui::binding::HostRejection::Validation(issues))
            if issues.iter().any(|issue| issue.requirement.as_deref() == Some("area_limit")))
        );
        assert_eq!(owner.accepted_state().transactions.len(), 2);
        assert_eq!(text(&owner, "area"), "12");
        assert!(owner.inspect(|_, _| ()).is_err());
        owner.restore_accepted().unwrap();
        assert_eq!(text(&owner, "limited"), "20");
        assert!(matches!(
            owner.apply(&[edit("width", "0")], UpdateOrigin::User),
            HostOutcome::Rejected(HostRejection::Validation(issues))
            if issues.iter().any(|issue| issue.requirement.as_deref() == Some("positive_width"))
        ));
        assert_eq!(owner.accepted_state().transactions.len(), 2);
        owner.restore_accepted().unwrap();
        assert!(matches!(
            owner.apply(
                &[edit("rectangle", "false"), edit("area", "24")],
                UpdateOrigin::User
            ),
            HostOutcome::Accepted
        ));
        assert_eq!(text(&owner, "width"), "21");
    }

    /// Keeps exact application scalar codecs separate from the language registry.
    #[test]
    fn demo_application_scalar_round_trips() {
        let mut owner = SheetOwner::new(demo_factory().unwrap(), SCHEMA.into()).unwrap();
        assert_eq!(text(&owner, "badge"), "7");
        assert!(matches!(
            owner.apply(&[edit("badge", "42")], UpdateOrigin::User),
            HostOutcome::Accepted
        ));
        assert_eq!(text(&owner, "badge_number"), "42");
        assert_eq!(decode_badge(&encode_badge(&Badge(42))).unwrap(), Badge(42));
        assert!(decode_badge("no").is_err());
        assert!(decode_badge("4294967296").is_err());
    }

    /// Chooses recovery only for model rejection and keeps recording failure accepted.
    #[test]
    fn demo_failure_policy_and_recovery_document() {
        use adam_web_ui::binding::{BindingError, HostRejection};
        assert_eq!(policy(&HostOutcome::Accepted), Policy::Report);
        assert_eq!(
            policy(&HostOutcome::Rejected(HostRejection::Binding(
                BindingError::UnknownKey("missing".into())
            ))),
            Policy::Report
        );
        assert_eq!(
            policy(&HostOutcome::Rejected(HostRejection::Input(
                adam_web_ui::input::InputError::Lexical("draft".into())
            ))),
            Policy::Report
        );
        assert_eq!(
            policy(&HostOutcome::RecordingFailed(
                adam_web_ui::binding::CaptureError::UnnamedSource
            )),
            Policy::Report
        );
        let mut owner = SheetOwner::new(demo_factory().unwrap(), SCHEMA.into()).unwrap();
        let rejected = owner.apply(&[edit("area", "200")], UpdateOrigin::User);
        assert_eq!(policy(&rejected), Policy::Restore);
        owner.restore_accepted().unwrap();
        owner.apply(&[edit("limited", "20")], UpdateOrigin::User);
        let fatal = owner.apply(&[edit("divisor", "0")], UpdateOrigin::User);
        assert_eq!(policy(&fatal), Policy::Terminate);
        let HostOutcome::Fatal(error) = fatal else {
            panic!("zero divisor must fail propagation");
        };
        assert_eq!(
            policy(&HostOutcome::Rejected(HostRejection::Mutation(error))),
            Policy::Restore
        );
        assert_eq!(text(&owner, "limited"), "10");
        let mut bytes = Vec::new();
        write_recovery(&mut bytes, owner.accepted_state()).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["transactions"][0][0]["canonical"], "20");
        assert_eq!(json["root"], "demo");
        assert_eq!(json["module"], "consumer");
        assert_eq!(json["schema_key"], "sheet-binding/demo/v1");
        assert_eq!(json["file_name"], "sheet_binding.adm2");
        assert_eq!(json["source"], SOURCE);
        assert_eq!(json["transactions"].as_array().unwrap().len(), 1);
        assert!(json.get("values").is_none());
        assert!(
            write_recovery(&mut &mut [][..], owner.accepted_state())
                .unwrap_err()
                .is_io()
        );
    }

    /// Creates one canonical user aspiration.
    ///
    /// - Complexity: O(key and value bytes) time and space.
    fn edit(key: &str, value: &str) -> CellEdit {
        CellEdit {
            key: key.into(),
            canonical: value.into(),
        }
    }

    /// Reads an accepted published scalar without accessing the mutable model.
    ///
    /// - Complexity: O(named cells).
    fn text<'a>(owner: &'a SheetOwner, key: &str) -> &'a str {
        &owner
            .views()
            .iter()
            .find(|view| view.key == key)
            .unwrap()
            .text
    }
}
