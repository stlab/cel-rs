//! Desktop file-open primitives: OS-native dialog, direct filesystem read,
//! and a filesystem watcher for the currently opened file. See
//! `begin/assets/open_file.js` and this module's web-only items for the web
//! build's equivalent (File System Access API instead of a real filesystem).

#[cfg(feature = "desktop")]
use std::path::Path;

/// Reads `path`'s full text.
///
/// # Errors
///
/// Returns `Err` with a human-readable message if `path` cannot be read
/// (missing, permission denied, not valid UTF-8, etc).
#[cfg(feature = "desktop")]
pub fn read_opened_file(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("failed to read {}: {e}", path.display()))
}

/// Opens the native "Open File" dialog restricted to `.adm2` files and
/// returns the picked path, or `None` if the user cancelled.
///
/// - Complexity: awaits user interaction; no upper bound on wall-clock time.
#[cfg(feature = "desktop")]
pub async fn pick_file() -> Option<std::path::PathBuf> {
    let handle = rfd::AsyncFileDialog::new()
        .add_filter("adm2", &["adm2"])
        .pick_file()
        .await?;
    Some(handle.path().to_path_buf())
}

/// Watches `path` for changes, calling `on_change` on every filesystem event.
/// The returned watcher must be kept alive for as long as the watch should
/// remain active — dropping it stops watching.
///
/// # Errors
///
/// Returns `Err` if the underlying OS watch could not be established (e.g.
/// `path`'s parent directory doesn't exist).
#[cfg(feature = "desktop")]
pub fn spawn_watch(
    path: std::path::PathBuf,
    mut on_change: impl FnMut() + Send + 'static,
) -> notify::Result<notify::RecommendedWatcher> {
    use notify::Watcher;

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            on_change();
        }
    })?;
    watcher.watch(&path, notify::RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

/// The result of a web-side `open()`/`refresh()` call, sent from JS via
/// `dioxus.send()` and read back with `eval.recv::<Option<OpenedFilePayload>>()`.
///
/// `id` is `Some(handle_id)` when a re-readable `FileSystemFileHandle` backs
/// this result (the File System Access path) — pass it to [`refresh_script`]
/// to reload later. `None` means the `<input type="file">` fallback was used:
/// the load is one-shot, with nothing to refresh from.
#[cfg(not(feature = "desktop"))]
#[derive(serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct OpenedFilePayload {
    pub id: Option<u32>,
    pub name: String,
    pub text: String,
}

/// `document::eval` script that opens the file picker (or its `<input
/// type="file">` fallback) and sends the result back via `dioxus.send()`.
/// Resolves to `null` on JS's side (received as `None`) if the user
/// cancelled.
#[cfg(not(feature = "desktop"))]
pub const OPEN_SCRIPT: &str =
    "(async () => { dioxus.send(await window.beginOpenFile.open()); })();";

/// `document::eval` script that re-reads the file behind handle `id` and
/// sends the refreshed `{id, name, text}` back via `dioxus.send()`.
#[cfg(not(feature = "desktop"))]
pub fn refresh_script(id: u32) -> String {
    format!("(async () => {{ dioxus.send(await window.beginOpenFile.refresh({id})); }})();")
}

#[cfg(all(test, feature = "desktop"))]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn read_opened_file_returns_file_contents() {
        let path = temp_path("begin_open_file_test_contents.adm2");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"sheet s { cell a: i32 = 1; }")
            .unwrap();

        let result = read_opened_file(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.unwrap(), "sheet s { cell a: i32 = 1; }");
    }

    #[test]
    fn read_opened_file_missing_file_returns_err() {
        let path = temp_path("begin_open_file_test_does_not_exist.adm2");
        let _ = std::fs::remove_file(&path);

        let result = read_opened_file(&path);

        assert!(result.is_err());
    }
}

#[cfg(all(test, not(feature = "desktop")))]
mod web_tests {
    use super::*;

    #[test]
    fn opened_file_payload_deserializes_with_handle_id() {
        let json = r#"{"id": 3, "name": "my_model.adm2", "text": "sheet s {}"}"#;
        let payload: OpenedFilePayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.id, Some(3));
        assert_eq!(payload.name, "my_model.adm2");
        assert_eq!(payload.text, "sheet s {}");
    }

    #[test]
    fn opened_file_payload_deserializes_without_handle_id() {
        let json = r#"{"id": null, "name": "my_model.adm2", "text": "sheet s {}"}"#;
        let payload: OpenedFilePayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.id, None);
    }

    #[test]
    fn refresh_script_embeds_the_given_id() {
        let script = refresh_script(3);
        assert!(script.contains("beginOpenFile.refresh(3)"), "{script}");
    }
}
