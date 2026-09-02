//! File I/O operations: open, save, and export via rfd. See
//! `docs/superpowers/specs/2026-08-26-ez-adam-ui-design.md` §6.

use std::path::{Path, PathBuf};

use crate::model::document::Document;
use crate::persistence::{from_json, to_json};

/// Reads and deserializes `path` as an `ez-adam` native-format document.
///
/// # Errors
///
/// Returns a human-readable message if `path` cannot be read or its
/// contents aren't valid `ez-adam` JSON.
pub fn read_document_file(path: &Path) -> Result<Document, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    from_json(&text).map_err(|e| format!("failed to parse {}: {e}", path.display()))
}

/// Serializes `doc` and writes it to `path`.
///
/// # Errors
///
/// Returns a human-readable message if `path` cannot be written.
pub fn write_document_file(path: &Path, doc: &Document) -> Result<(), String> {
    std::fs::write(path, to_json(doc))
        .map_err(|e| format!("failed to write {}: {e}", path.display()))
}

/// Writes already-generated `.adm2` text to `path`.
///
/// # Errors
///
/// Returns a human-readable message if `path` cannot be written.
pub fn write_adm2_file(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

/// Opens the native "Open" dialog restricted to `ez-adam`'s native JSON
/// format, returning the picked path or `None` if cancelled.
///
/// - Complexity: awaits user interaction; no upper bound on wall-clock time.
pub async fn pick_open_path() -> Option<PathBuf> {
    let handle = rfd::AsyncFileDialog::new()
        .add_filter("ez-adam document", &["json"])
        .pick_file()
        .await?;
    Some(handle.path().to_path_buf())
}

/// Opens the native "Save" dialog for `ez-adam`'s native JSON format,
/// returning the picked path or `None` if cancelled.
///
/// - Complexity: awaits user interaction; no upper bound on wall-clock time.
pub async fn pick_save_path() -> Option<PathBuf> {
    let handle = rfd::AsyncFileDialog::new()
        .add_filter("ez-adam document", &["json"])
        .save_file()
        .await?;
    Some(handle.path().to_path_buf())
}

/// Opens the native "Export" dialog for `.adm2` files, returning the
/// picked path or `None` if cancelled.
///
/// - Complexity: awaits user interaction; no upper bound on wall-clock time.
pub async fn pick_export_path() -> Option<PathBuf> {
    let handle = rfd::AsyncFileDialog::new()
        .add_filter("adm2", &["adm2"])
        .save_file()
        .await?;
    Some(handle.path().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::cell::CellType;
    use crate::ops::cells::add_cell;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn write_then_read_document_file_round_trips() {
        let mut doc = Document::new("demo");
        let _ = add_cell(&mut doc, "a", CellType::i64());
        let path = temp_path("ez_adam_file_io_test_round_trip.json");

        write_document_file(&path, &doc).unwrap();
        let read_back = read_document_file(&path).unwrap();

        std::fs::remove_file(&path).unwrap();
        assert_eq!(doc, read_back);
    }

    #[test]
    fn read_document_file_missing_file_returns_err() {
        let path = temp_path("ez_adam_file_io_test_does_not_exist.json");
        let _ = std::fs::remove_file(&path);
        assert!(read_document_file(&path).is_err());
    }

    #[test]
    fn write_adm2_file_writes_the_given_text() {
        let path = temp_path("ez_adam_file_io_test_export.adm2");
        write_adm2_file(&path, "sheet s {}\n").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(contents, "sheet s {}\n");
    }
}
