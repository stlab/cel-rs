//! Persists accepted host inputs and selects irreversible shutdown actions.
//!
//! Borrow the owner's journal only while serializing a `RecoveryDocument`;
//! a failed runtime is never consulted to construct recovery data.

use crate::example_source::ActiveSource;
use adam_web_ui::binding::{RecoveryState, SheetHandle};
use dioxus::prelude::*;

/// Borrows the accepted source, fixed codec configuration, and ordered aspirations.
///
/// The journal remains owned by `SheetOwner`; serialization never clones it.
#[derive(serde::Serialize)]
pub(crate) struct RecoveryDocument<'a> {
    /// Persistent document and scalar-codec contract version.
    version: u32,
    /// Accepted source text and stable example or opened-file identity.
    source: &'a ActiveSource,
    /// Stable root schema identity and ordered accepted aspiration groups.
    recovery: &'a RecoveryState,
}

impl<'a> RecoveryDocument<'a> {
    /// Describes the accepted source and journal without consulting a live sheet.
    fn new(source: &'a ActiveSource, recovery: &'a RecoveryState) -> Self {
        Self {
            version: 1,
            source,
            recovery,
        }
    }
}

/// Selects ordered irreversible fatal handling, with no reconstruction action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShutdownAction {
    /// Reports the independently owned fatal diagnostic.
    ReportFatal,
    /// Attempts to persist only the accepted application state.
    PersistAccepted,
    /// Terminates the desktop process or unmounts the web runtime.
    Terminate,
}

/// Tracks accepted revisions and durability independently of the mutable model.
#[derive(Default)]
pub(crate) struct RecoveryStatus {
    /// The latest accepted application revision.
    revision: u64,
    /// The revision whose persistence completed successfully.
    durable: Option<u64>,
    /// Permanently blocks subsequent recovery, install, or shutdown attempts.
    fatal: bool,
}

impl RecoveryStatus {
    /// Advances acceptance without copying the journal.
    pub(crate) fn accept(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("accepted revision exhausted");
    }

    /// Marks only this exact accepted revision as durable.
    pub(crate) fn mark_durable(&mut self) {
        self.durable = Some(self.revision);
    }

    /// Retires the boundary once and returns the shutdown sequence.
    pub(crate) fn fatal_actions(&mut self) -> &'static [ShutdownAction] {
        if self.fatal {
            return &[];
        }
        self.fatal = true;
        if self.durable == Some(self.revision) {
            &[ShutdownAction::ReportFatal, ShutdownAction::Terminate]
        } else {
            &[
                ShutdownAction::ReportFatal,
                ShutdownAction::PersistAccepted,
                ShutdownAction::Terminate,
            ]
        }
    }
}

/// Owns application provenance and shutdown policy, not another model or journal.
#[derive(Clone, Copy)]
pub(crate) struct HostBoundary {
    /// Accepted source and its sole owner, available after runtime retirement.
    accepted: Signal<Option<(ActiveSource, SheetHandle)>>,
    /// Durability and one-way fatal state.
    status: Signal<RecoveryStatus>,
    /// The diagnostic that replaces the web runtime after fatal propagation.
    message: Signal<Option<String>>,
}

impl HostBoundary {
    /// Allocates scope-owned host state before the first source candidate is built.
    pub(crate) fn new() -> Self {
        Self {
            accepted: Signal::new(None),
            status: Signal::new(RecoveryStatus::default()),
            message: Signal::new(None),
        }
    }

    /// Returns whether fatal propagation permanently closed this host.
    pub(crate) fn is_fatal(self) -> bool {
        self.status.read().fatal
    }

    /// Returns the visible fatal diagnostic without accessing a sheet.
    pub(crate) fn message(self) -> Option<String> {
        self.message.read().clone()
    }

    /// Commits source provenance only after a successfully initialized installation.
    ///
    /// - Complexity: O(source bytes) owned provenance transfer.
    pub(crate) fn installed(mut self, source: ActiveSource, sheet: SheetHandle) {
        self.accepted.set(Some((source, sheet)));
        self.accept();
    }

    /// Marks an accepted transaction dirty without copying its history.
    pub(crate) fn accept(mut self) {
        self.status.write().accept();
    }

    /// Persists the borrowed accepted journal independently of live model availability.
    ///
    /// # Errors
    /// Returns unavailable journal, serialization, platform directory, or IO failures.
    ///
    /// - Complexity: O(source and accepted journal bytes) time and space.
    #[cfg(feature = "desktop")]
    fn persist(mut self) -> std::io::Result<()> {
        let accepted = self.accepted.peek();
        if let Some((source, sheet)) = accepted.as_ref() {
            let path = recovery_path()?;
            sheet
                .with_accepted_state(|state| {
                    persist_recovery(&path, &RecoveryDocument::new(source, state))
                })
                .map_err(std::io::Error::other)??;
            self.status.write().mark_durable();
        }
        Ok(())
    }

    /// Persists accepted JSON in browser storage and reports denied/quota errors.
    ///
    /// The revision guard prevents a late acknowledgment from marking newer edits durable.
    ///
    /// - Complexity: O(source and journal bytes) serialization and transport.
    #[cfg(not(feature = "desktop"))]
    fn persist(mut self) {
        let serialized = {
            let accepted = self.accepted.peek();
            accepted.as_ref().map(|(source, sheet)| {
                sheet.with_accepted_state(|state| {
                    serde_json::to_string(&RecoveryDocument::new(source, state))
                })
            })
        };
        let Some(serialized) = serialized else { return };
        let serialized = match serialized {
            Ok(Ok(serialized)) => serialized,
            error => {
                adam_web_ui::diagnostics::report_error(&format!(
                    "Recovery persistence failed: {error:?}"
                ));
                return;
            }
        };
        let revision = self.status.peek().revision;
        dioxus::core::Runtime::current().spawn(ScopeId::APP, async move {
            let script = format!(
                "try {{ localStorage.setItem('stlab.begin.recovery.v1', {}); dioxus.send(null); }} catch (error) {{ dioxus.send(String(error)); }}",
                serde_json::to_string(&serialized).expect("JSON string serializes"),
            );
            let result = document::eval(&script).recv::<Option<String>>().await;
            match result {
                Ok(None) if self.status.peek().revision == revision => self.status.write().mark_durable(),
                Ok(None) => {}
                result => adam_web_ui::diagnostics::report_error(&format!("Recovery persistence failed: {result:?}")),
            }
        });
    }

    /// Reports fatal propagation, attempts persistence once, and terminates without retry.
    ///
    /// - Complexity: Diagnostic and accepted serialization bytes.
    pub(crate) fn fatal(mut self, message: String) {
        let actions = self.status.write().fatal_actions();
        for action in actions {
            match action {
                ShutdownAction::ReportFatal => {
                    adam_web_ui::diagnostics::report_error(&message);
                    self.message.set(Some(message.clone()));
                }
                ShutdownAction::PersistAccepted => {
                    #[cfg(feature = "desktop")]
                    if let Err(error) = self.persist() {
                        adam_web_ui::diagnostics::report_error(&format!(
                            "Recovery persistence failed: {error}"
                        ));
                    }
                    #[cfg(not(feature = "desktop"))]
                    self.persist();
                }
                ShutdownAction::Terminate => {
                    #[cfg(feature = "desktop")]
                    terminate();
                }
            }
        }
    }
}

/// Returns the platform-local recovery destination without a working-directory fallback.
///
/// # Errors
/// Returns an error when the platform cannot identify an application data directory.
#[cfg(feature = "desktop")]
fn recovery_path() -> std::io::Result<std::path::PathBuf> {
    #[cfg(test)]
    if let Some(path) = std::env::var_os("BEGIN_TEST_RECOVERY_PATH") {
        return Ok(path.into());
    }
    let directories = directories::ProjectDirs::from("org", "stlab", "begin");
    recovery_path_from(directories.as_ref().map(|dirs| dirs.data_local_dir()))
}

/// Resolves only a platform-provided data directory, never the launch directory.
///
/// # Errors
/// Returns an error when the platform supplied no application directory.
///
/// - Complexity: O(directory path bytes).
#[cfg(feature = "desktop")]
fn recovery_path_from(directory: Option<&std::path::Path>) -> std::io::Result<std::path::PathBuf> {
    directory
        .map(|directory| directory.join("recovery.json"))
        .ok_or_else(|| std::io::Error::other("platform application data directory unavailable"))
}

/// Atomically replaces a recovery document with a synchronized same-directory file.
///
/// # Errors
/// Returns serialization, directory, file synchronization, or atomic replacement failures.
///
/// - Complexity: O(serialized source and journal bytes).
#[cfg(feature = "desktop")]
pub(crate) fn persist_recovery(
    path: &std::path::Path,
    document: &RecoveryDocument<'_>,
) -> std::io::Result<()> {
    #[cfg(test)]
    if let Some(log) = std::env::var_os("BEGIN_TEST_PERSIST_LOG") {
        use std::io::Write;
        writeln!(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log)?,
            "persist"
        )?;
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| std::io::Error::other("recovery destination requires a parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(file.as_file_mut(), document)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

/// Exits the owning desktop process with failure after its persistence attempt.
#[cfg(feature = "desktop")]
fn terminate() -> ! {
    std::process::exit(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Missing platform directories are an explicit error, with no relative-path fallback.
    #[cfg(feature = "desktop")]
    #[test]
    fn missing_platform_directory_is_an_error() {
        assert!(recovery_path_from(None).is_err());
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(
            recovery_path_from(Some(directory)).unwrap(),
            directory.join("recovery.json")
        );
    }

    /// Round-trips accepted aspirations and atomically overwrites an existing document.
    #[cfg(feature = "desktop")]
    #[test]
    fn atomic_recovery_contains_only_accepted_stable_inputs() {
        let directory = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let path = directory.path().join("recovery.json");
        std::fs::write(&path, "old revision").unwrap();
        let source = ActiveSource {
            name: "test".into(),
            text: "sheet s { cell x: i32 = 1; }".into(),
            origin: crate::example_source::SourceOrigin::Opened("test.adm2".into()),
        };
        let recovery = RecoveryState {
            schema_key: "test/v1".into(),
            transactions: vec![vec![adam_web_ui::binding::CellEdit {
                key: "x".into(),
                canonical: "20".into(),
            }]],
        };
        persist_recovery(&path, &RecoveryDocument::new(&source, &recovery)).unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["source"]["text"], source.text);
        assert_eq!(
            serde_json::from_value::<std::ffi::OsString>(
                json["source"]["origin"]["Opened"].clone()
            )
            .unwrap(),
            "test.adm2"
        );
        assert_eq!(json["recovery"]["transactions"][0][0]["canonical"], "20");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        let blocked = directory.path().join("not-a-directory");
        std::fs::write(&blocked, "blocked").unwrap();
        assert!(
            persist_recovery(
                &blocked.join("recovery.json"),
                &RecoveryDocument::new(&source, &recovery)
            )
            .is_err()
        );
    }

    /// Executes the actual desktop boundary in an owned test subprocess only.
    #[cfg(feature = "desktop")]
    #[test]
    #[ignore = "launched by desktop_fatal_subprocesses; never exits the controlling harness"]
    fn desktop_fatal_child() {
        /// Publishes an accepted edit, then triggers the real fatal callback.
        fn host() -> Element {
            let boundary = use_context_provider(HostBoundary::new);
            use_hook(move || {
                let counter_path = std::env::var_os("BEGIN_TEST_FACTORY_LOG").unwrap();
                let owner = adam_web_ui::binding::SheetOwner::new(
                    std::rc::Rc::new(move || {
                        use std::io::Write;
                        let mut file = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&counter_path)
                            .unwrap();
                        writeln!(file, "factory").unwrap();
                        adam_web_ui::build_sheet(
                            "sheet s { cell divisor: i32 = 1; out result: i32 := 10 / divisor; }",
                            "fatal.adm2",
                        )
                    }),
                    "fatal/v1".into(),
                )
                .unwrap();
                let sheet =
                    SheetHandle::from_owner(owner, EventHandler::new(crate::app::sheet_outcome));
                boundary.installed(
                    ActiveSource {
                        name: "fatal".into(),
                        text: "sheet s { cell divisor: i32 = 1; out result: i32 := 10 / divisor; }"
                            .into(),
                        origin: crate::example_source::SourceOrigin::Example,
                    },
                    sheet,
                );
                sheet.cell("divisor").submit("2".into());
                if std::env::var_os("BEGIN_TEST_DURABLE").is_some() {
                    boundary.persist().unwrap();
                }
                if let Ok(mode) = std::env::var("BEGIN_TEST_FATAL_MODE")
                    && (mode == "candidate" || mode == "graph")
                {
                    let error = adam_web_ui::build_sheet(
                        "sheet s { cell divisor: i32 = 0; out result: i32 := 10 / divisor; }",
                        "candidate.adm2",
                    )
                    .err()
                    .unwrap();
                    if mode == "graph" {
                        let adam_web_ui::BuildError::Runtime(failure) = error else {
                            panic!("runtime fixture required");
                        };
                        crate::app::graph_outcome(adam_web_ui::graph::GraphFailure::Runtime(
                            failure,
                        ));
                    } else {
                        crate::app::report_build_error(&error);
                    }
                    panic!("fatal candidate handling must terminate");
                }
                sheet.cell("divisor").submit("0".into());
                panic!("fatal handling must terminate before another model operation");
            });
            rsx! {}
        }
        VirtualDom::new(host).rebuild_in_place();
    }

    /// Verifies nonzero desktop exit after successful, failed, and already durable persistence.
    #[cfg(feature = "desktop")]
    #[test]
    fn desktop_fatal_subprocesses() {
        let directory = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        for mode in ["success", "failure", "durable", "candidate", "graph"] {
            let base = directory.path().join(mode);
            std::fs::create_dir(&base).unwrap();
            let destination = if mode == "failure" {
                let blocked = base.join("blocked");
                std::fs::write(&blocked, "not a directory").unwrap();
                blocked.join("recovery.json")
            } else {
                base.join("recovery.json")
            };
            let log = base.join("factory.log");
            let persist_log = base.join("persist.log");
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "recovery::tests::desktop_fatal_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("BEGIN_TEST_RECOVERY_PATH", &destination)
                .env("BEGIN_TEST_FACTORY_LOG", &log);
            command.env("BEGIN_TEST_PERSIST_LOG", &persist_log);
            command.env("BEGIN_TEST_FATAL_MODE", mode);
            if mode == "durable" {
                command.env("BEGIN_TEST_DURABLE", "1");
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(1), "{mode}: {output:?}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains("division"), "{mode}: {stderr}");
            assert_eq!(std::fs::read_to_string(&log).unwrap(), "factory\n");
            assert_eq!(
                std::fs::read_to_string(&persist_log).unwrap(),
                "persist\n",
                "{mode}"
            );
            if mode == "failure" {
                assert!(stderr.contains("Recovery persistence failed"), "{stderr}");
            } else {
                let json: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(&destination).unwrap()).unwrap();
                assert_eq!(
                    json["recovery"]["transactions"].as_array().unwrap().len(),
                    1
                );
                assert_eq!(json["recovery"]["transactions"][0][0]["canonical"], "2");
            }
            if let Some(evidence) = std::env::var_os("BEGIN_TASK8_EVIDENCE") {
                let evidence = std::path::PathBuf::from(evidence);
                std::fs::copy(&log, evidence.join(format!("desktop-{mode}.factory.log"))).unwrap();
                std::fs::copy(
                    &persist_log,
                    evidence.join(format!("desktop-{mode}.persist.log")),
                )
                .unwrap();
                std::fs::write(
                    evidence.join(format!("desktop-{mode}.result.json")),
                    serde_json::to_vec_pretty(&serde_json::json!({
                        "mode": mode,
                        "exit_code": output.status.code(),
                        "accepted_owner_factory_calls": 1,
                        "total_persistence_attempts": 1,
                        "fatal_persistence_attempts": if mode == "durable" { 0 } else { 1 },
                        "recovery_written": destination.is_file(),
                    }))
                    .unwrap(),
                )
                .unwrap();
                std::fs::write(
                    evidence.join(format!("desktop-{mode}.stderr")),
                    &output.stderr,
                )
                .unwrap();
                std::fs::write(
                    evidence.join(format!("desktop-{mode}.stdout")),
                    &output.stdout,
                )
                .unwrap();
                if destination.is_file() {
                    std::fs::copy(&destination, evidence.join(format!("desktop-{mode}.json")))
                        .unwrap();
                }
            }
        }
    }

    /// Fatal handling reports, persists once, and terminates without a rebuild action.
    #[test]
    fn fatal_is_irreversible_and_persists_only_once() {
        let mut status = RecoveryStatus::default();
        assert_eq!(
            status.fatal_actions(),
            &[
                ShutdownAction::ReportFatal,
                ShutdownAction::PersistAccepted,
                ShutdownAction::Terminate
            ]
        );
        assert!(status.fatal_actions().is_empty());
    }

    /// A durable accepted revision needs no second persistence attempt.
    #[test]
    fn durable_revision_skips_persistence_until_next_acceptance() {
        let mut status = RecoveryStatus::default();
        status.mark_durable();
        assert_eq!(
            status.fatal_actions(),
            &[ShutdownAction::ReportFatal, ShutdownAction::Terminate]
        );
        let mut newer = RecoveryStatus::default();
        newer.mark_durable();
        newer.accept();
        assert!(
            newer
                .fatal_actions()
                .contains(&ShutdownAction::PersistAccepted)
        );
    }
}
