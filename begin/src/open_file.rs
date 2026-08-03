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
